//! core の文書への明示変換（C# の `PsdBridge.Export`・`Import` と対）。名前によるレイヤー対応付けはしない。
//!
//! - 書き出し（`from_core`）は Color の写し。ラスター・グループ（入れ子・通過/分離）・単色の塗りつぶし・調整（反転・レベル補正・
//!   色相/彩度。PSD の刻みに収まるものだけ）・クリッピング・ラスターマスク（有効/無効・濃度）・層のロックを、PSD の形で書く。
//!   PSD に形が無い・この写しが書けないもの（反転したマスク・クリッピングされたグループ・半透明の塗りつぶし・刻みの間の調整・
//!   Color 以外のチャンネルの中身・Color と違うチャンネルごとの合成）は、平らにも黙って落とすこともせず、層の名前と理由で断る。
//! - 取り込み（`to_core`）は逆で、並びも ID も C# の取り込みと同じ（PSD の層 ID と区切りの ID を core の層 ID の中に持ち、
//!   書き出し直すと同じ ID になる）。
use super::*;
use crate::{check, check_budget, Error, Result};
use std::collections::{HashMap, HashSet};
use yolu_core::{
    AdjustmentSettings, AdjustmentType, Channel, Document as CoreDocument, Layer as CoreLayer,
    LayerId, LayerKind as CoreKind, LayerLocks, RasterMask, Rgba8, TileCoord,
};

/// PSD の画素の予算（C# と同じ。マスクは画布 1 枚ぶんを数える）。
const PIXEL_BUDGET: u64 = 128 * 1024 * 1024;

/// core の層のロック（1:透明部分・2:画素・4:位置・8:すべて）を PSD の lspf のビット（0:透明部分・1:画素・2:位置・31:すべて）へ。
/// すべては 0x80000000 だけにする（その下の個別のビットは足さない。効くロックは同じで、読み直した PSD も同じ形になる）。
fn psd_locks(locks: LayerLocks) -> u32 {
    if locks.contains(LayerLocks::ALL) {
        return 0x8000_0000;
    }
    let mut bits = 0;
    for (lock, bit) in [
        (LayerLocks::TRANSPARENCY, 1),
        (LayerLocks::PIXELS, 2),
        (LayerLocks::POSITION, 4),
    ] {
        if locks.contains(lock) {
            bits |= bit;
        }
    }
    bits
}
/// `psd_locks` の逆。
fn core_locks(bits: u32) -> LayerLocks {
    let mut locks = LayerLocks::NONE;
    for (bit, lock) in [
        (1, LayerLocks::TRANSPARENCY),
        (2, LayerLocks::PIXELS),
        (4, LayerLocks::POSITION),
        (0x8000_0000, LayerLocks::ALL),
    ] {
        if bits & bit != 0 {
            locks = locks | lock;
        }
    }
    locks
}
fn channel_label(d: &CoreDocument, c: Channel) -> String {
    d.channel_info(c)
        .map(|i| i.name.clone())
        .unwrap_or_else(|| format!("番号 {}", c.index()))
}
fn whole(v: f64) -> Option<i32> {
    let r = v.round_ties_even();
    ((v - r).abs() < 1e-9).then_some(r as i32)
}
/// core の調整を PSD の刻みの整数で。刻みの間は丸めず断る（C# の `AdjustmentRefusal` と同じ条件）。
fn psd_adjustment(s: &AdjustmentSettings) -> std::result::Result<Adjustment, Refusal> {
    match s.kind() {
        AdjustmentType::Invert => Ok(Adjustment::Invert),
        AdjustmentType::Levels => {
            let steps = [
                whole(s.input_black() * 255.0),
                whole(s.input_white() * 255.0),
                whole(s.output_black() * 255.0),
                whole(s.output_white() * 255.0),
                whole(s.gamma() * 100.0),
            ];
            let [Some(ib), Some(iw), Some(ob), Some(ow), Some(g)] = steps else {
                return Err(Refusal::LevelsBetweenSteps);
            };
            if !(ib <= 253 && iw >= 2 && iw > ib) {
                return Err(Refusal::LevelsRange);
            }
            Ok(Adjustment::Levels {
                input_black: ib as u16,
                input_white: iw as u16,
                output_black: ob as u16,
                output_white: ow as u16,
                gamma: g as u16,
            })
        }
        AdjustmentType::HueSaturation => {
            let steps = [
                whole(s.hue()),
                whole(s.saturation() * 100.0),
                whole(s.lightness() * 100.0),
            ];
            let [Some(hue), Some(saturation), Some(lightness)] = steps else {
                return Err(Refusal::HueSaturationBetweenSteps);
            };
            Ok(Adjustment::HueSaturation {
                hue: hue as i16,
                saturation: saturation as i16,
                lightness: lightness as i16,
            })
        }
    }
}
/// PSD の調整を core の設定へ（C# の取り込みと同じ割り算。刻みの整数をそのまま写す）。
fn core_adjustment(a: &Adjustment) -> Result<AdjustmentSettings> {
    Ok(match *a {
        Adjustment::Invert => AdjustmentSettings::invert(),
        Adjustment::Levels {
            input_black,
            input_white,
            output_black,
            output_white,
            gamma,
        } => AdjustmentSettings::levels(
            f64::from(input_black) / 255.0,
            f64::from(input_white) / 255.0,
            f64::from(gamma) / 100.0,
            f64::from(output_black) / 255.0,
            f64::from(output_white) / 255.0,
        )?,
        Adjustment::HueSaturation {
            hue,
            saturation,
            lightness,
        } => AdjustmentSettings::hue_saturation(
            f64::from(hue),
            f64::from(saturation) / 100.0,
            f64::from(lightness) / 100.0,
        )?,
    })
}
/// 層の Color での合成モードと不透明度（チャンネルごとの設定があればそれ）。PSD の層は 1 組しか持てないので、これを書く。
fn color_blend(l: &CoreLayer) -> (BlendMode, u8) {
    (
        BlendMode::ALL[l.blend_mode_in(Channel::Color) as usize],
        (l.opacity_in(Channel::Color) * 255.0).round_ties_even() as u8,
    )
}

/// PSD に書けない理由（層ごと）。画面は種類から画面の言語の文を作る。`message` は日本語の診断（`from_core` の断りの文）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Color 以外のチャンネルの中身（ラスターの面・塗りつぶしの値・有効の印）。チャンネルの名前。
    OtherChannel(String),
    /// クリッピングされたグループ（Photoshop が正しく扱うかを確かめられない）。
    ClippedGroup,
    /// 反転したマスク（PSD に非破壊の反転が無い）。
    InvertedMask,
    /// Color と違うチャンネルごとの合成（PSD の層は合成モードと不透明度を 1 組しか持てない）。チャンネルの名前。
    ChannelBlend(String),
    /// ラスターの Color が無効。
    ColorDisabled,
    /// 塗りつぶしの Color の値が無い、または Color が無効。
    FillWithoutColor,
    /// 半透明の塗りつぶし（PSD の単色の塗りつぶしは不透明だけ）。
    FillTranslucent,
    /// 調整が Color で無効。
    AdjustmentColorDisabled,
    /// レベル補正が PSD の刻み（0〜255 の整数・ガンマは 1/100）の間にある。
    LevelsBetweenSteps,
    /// レベル補正の入力が PSD の範囲（黒 0〜253、白はその上の 2〜255）に収まらない。
    LevelsRange,
    /// 色相・彩度が PSD の刻み（1 度・1%）の間にある。
    HueSaturationBetweenSteps,
    /// 層かマスクのフィルター・Generator（効いていない段・無効の段も。設定が PSD に残らず、層の画素と統合画像が食い違う）。
    Effects,
    /// Anchor（PSD に形が無い）。
    Anchor,
    /// パス（PSD に形が無い）。
    Path,
}
/// 層 1 枚の断りの理由。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blocker {
    pub layer: String,
    pub refusal: Refusal,
}
impl Blocker {
    /// 日本語の診断（層の名前つき）。
    pub fn message(&self) -> String {
        let name = &self.layer;
        match &self.refusal {
            Refusal::OtherChannel(label) => format!("層「{name}」は Color 以外のチャンネル（{label}）を使っています。PSD に書けるのは Color だけです"),
            Refusal::ClippedGroup => format!("グループ「{name}」はクリッピングされています。クリッピングされたフォルダーを Photoshop が正しく扱うかは確かめられていないので、PSD に書きません"),
            Refusal::InvertedMask => format!("層「{name}」のマスクは反転しています。PSD には非破壊のマスクの反転がないので、画素に焼かず断ります"),
            Refusal::ChannelBlend(label) => format!("層「{name}」のチャンネル（{label}）の合成が Color と違います。PSD の層は合成モードと不透明度を 1 組しか持てません"),
            Refusal::ColorDisabled => format!("層「{name}」の Color が無効です。無効にした Color の画素は PSD に書けません"),
            Refusal::FillWithoutColor => format!("塗りつぶし「{name}」は Color の値が無い、または Color が無効です。PSD に書けるのは Color の単色だけです"),
            Refusal::FillTranslucent => format!("塗りつぶし「{name}」の Color は半透明です。PSD の単色の塗りつぶしは不透明だけです"),
            Refusal::AdjustmentColorDisabled => format!("調整「{name}」は Color で無効です。PSD に書けるのは Color に効く調整だけです"),
            Refusal::LevelsBetweenSteps => format!("調整「{name}」のレベル補正は PSD の刻み（0〜255 の整数・ガンマは 1/100）の間にあります。丸めて書かず断ります"),
            Refusal::LevelsRange => format!("調整「{name}」のレベル補正は PSD に書けません。入力の黒は 0〜253、入力の白はそれより上の 2〜255 です"),
            Refusal::HueSaturationBetweenSteps => format!("調整「{name}」の色相・彩度は PSD の刻み（1 度・1%）の間にあります。丸めて書かず断ります"),
            Refusal::Effects => format!("層「{name}」にフィルターか Generator があります。効果は PSD に書けません"),
            Refusal::Anchor => format!("層「{name}」に Anchor があります。Anchor は PSD に書けません"),
            Refusal::Path => format!("層「{name}」にパスがあります。パスは PSD に書けません"),
        }
    }
}
/// PSD へ写せない中身（平らにも黙って落とすこともせず、機能ごとの理由で断る。C# の `PsdBridge.Export` の断りと対）。
/// 書き出すのは Color の写し: 層の中身が Color だけで、Color で有効なもの。PSD に形が無いもの（マスクの反転・クリッピングされた
/// グループ・半透明の塗りつぶし・刻みの間の調整）と、層が効くほかのチャンネルの実効の合成が Color と違うものは断る。1 枚の層の理由を全部（`from_core` は初めの 1 つ）。
fn refusals(d: &CoreDocument, l: &CoreLayer) -> Vec<Refusal> {
    let kind = l.kind();
    let mut out = Vec::new();
    // 効果（効いていない段・無効の段も、設定が PSD に残らない）・Anchor・パス。書き出す層の画素は元の画素で、効果入りの合成は
    // 統合画像にだけ入るので、書けば層と統合画像が食い違う（マスクのフィルターも同じ）
    if !l.filters().is_empty() || l.mask().is_some_and(|m| !m.filters().is_empty()) {
        out.push(Refusal::Effects)
    }
    if l.anchor().is_some() {
        out.push(Refusal::Anchor)
    }
    if l.path().is_some() {
        out.push(Refusal::Path)
    }
    if matches!(kind, CoreKind::Raster | CoreKind::Fill) {
        // 標準の 6 つだけでなくユーザーチャンネル（番号 6 以降）も、面・有効の印・塗りつぶしの値のどれかがあれば断る
        let mut others = l.surface_channels();
        others.extend(l.enabled_channels());
        others.extend(l.fill_values().map(|(c, _)| c));
        others.retain(|c| *c != Channel::Color);
        others.sort();
        others.dedup();
        if let Some(&ch) = others.first() {
            out.push(Refusal::OtherChannel(channel_label(d, ch)))
        }
    }
    if kind == CoreKind::Group && l.clipping() {
        out.push(Refusal::ClippedGroup)
    }
    if l.mask().is_some_and(|m| m.inverted()) {
        out.push(Refusal::InvertedMask)
    }
    // チャンネルごとの合成: PSD の層は 1 組しか持てない。Color の実効の値で書くので、層が効くほかのチャンネルの実効の値が違えば落とす
    // ことになる。効くチャンネルは、明示の設定があるチャンネルと、調整なら有効なチャンネル、グループなら標準のチャンネル（中の層が
    // どこへ効くかはグループ自身には分からない）。ラスターと塗りつぶしは、Color 以外に中身があれば上で断っている
    let color = (
        l.blend_mode_in(Channel::Color),
        l.opacity_in(Channel::Color),
    );
    let mut acting: Vec<Channel> = l.channel_blends().map(|(c, _)| c).collect();
    match kind {
        CoreKind::Adjustment => acting.extend(l.enabled_channels()),
        CoreKind::Group => acting.extend(Channel::ALL),
        CoreKind::Raster | CoreKind::Fill => {}
    }
    acting.retain(|c| *c != Channel::Color);
    acting.sort();
    acting.dedup();
    // 違うチャンネルが幾つあっても理由は 1 つ（初めのチャンネル。画面の断りの一覧を、1 つの層で埋めない）
    if let Some(c) = acting
        .into_iter()
        .find(|&c| (l.blend_mode_in(c), l.opacity_in(c)) != color)
    {
        out.push(Refusal::ChannelBlend(channel_label(d, c)))
    }
    let color_on = l.is_channel_enabled(Channel::Color);
    match kind {
        CoreKind::Raster => {
            if !color_on {
                out.push(Refusal::ColorDisabled)
            }
        }
        CoreKind::Fill => {
            let value = l.fill_value(Channel::Color);
            if value.is_none() || !color_on {
                out.push(Refusal::FillWithoutColor)
            } else if value.is_some_and(|v| v.a != 255) {
                out.push(Refusal::FillTranslucent)
            }
        }
        CoreKind::Adjustment => {
            // 調整の効くチャンネルは PSD に持てない（読み込み直すと、効く標準のチャンネル全部へ効く調整になる。C# の取り込みと同じ）。
            // Color で効かない調整は、ラスターの無効な Color と同じく書かない。
            if !color_on {
                out.push(Refusal::AdjustmentColorDisabled)
            }
            if let Some(Err(why)) = l.adjustment().map(psd_adjustment) {
                out.push(why)
            }
        }
        CoreKind::Group => {}
    }
    out
}
/// PSD に書けない層とその理由（下の層から。書き出せる文書なら空）。`from_core` が断るのと同じ判断で、画面が全部を言い分けるために使う。
pub fn export_blockers(d: &CoreDocument) -> Vec<Blocker> {
    let mut out = Vec::new();
    for l in d.layers() {
        for refusal in refusals(d, l) {
            out.push(Blocker {
                layer: l.name().into(),
                refusal,
            })
        }
    }
    out
}

/// PSD 側の ID（正の整数）。core の層 ID の上位 32 bit（C# の Guid の先頭 4 バイト）から取り、重なれば次の空きへ。
fn unique_id(raw: i32, used: &mut HashSet<i32>) -> i32 {
    let mut id = raw & i32::MAX;
    if id == 0 {
        id = 1
    }
    while !used.insert(id) {
        id = if id == i32::MAX { 1 } else { id + 1 }
    }
    id
}
fn layer_part(id: LayerId) -> i32 {
    (id.0 >> 96) as i32
}
/// グループの区切りの ID（C# の Guid の 4〜7 バイト目。core の層 ID では 64〜95 bit 目の 2 つの 16 bit）。
fn divider_part(id: LayerId) -> i32 {
    ((((id.0 >> 80) & 0xffff) as u32) | ((((id.0 >> 64) & 0xffff) as u32) << 16)) as i32
}
/// `divider_part` の逆（取り込みで区切りの ID を core の層 ID に持つ）。
fn divider_bits(divider: i32) -> u128 {
    let d = divider as u32;
    (u128::from(d & 0xffff) << 80) | (u128::from(d >> 16) << 64)
}

struct Export<'a> {
    d: &'a CoreDocument,
    /// 親ごとの子（下から上）。
    children: HashMap<Option<LayerId>, Vec<&'a CoreLayer>>,
    used: HashSet<i32>,
    budget: u64,
}
impl<'a> Export<'a> {
    fn new(d: &'a CoreDocument) -> Self {
        let mut children: HashMap<Option<LayerId>, Vec<&CoreLayer>> = HashMap::new();
        for l in d.layers() {
            children.entry(l.parent()).or_default().push(l)
        }
        Self {
            d,
            children,
            used: HashSet::new(),
            budget: 0,
        }
    }
    /// 1 つの段（None は一番上）。上から下の並びで返す。
    fn level(&mut self, parent: Option<LayerId>) -> Result<Vec<Layer>> {
        let kids = self.children.get(&parent).cloned().unwrap_or_default();
        kids.into_iter().rev().map(|l| self.layer(l)).collect()
    }
    fn layer(&mut self, l: &'a CoreLayer) -> Result<Layer> {
        if let Some(refusal) = refusals(self.d, l).into_iter().next() {
            return Err(Error::InvalidData(
                Blocker {
                    layer: l.name().into(),
                    refusal,
                }
                .message(),
            ));
        }
        let mask = l.mask().map(|m| self.mask(m)).transpose()?;
        let (blend_mode, opacity) = color_blend(l);
        let id = unique_id(layer_part(l.id()), &mut self.used);
        let mut layer = Layer {
            id,
            name: l.name().into(),
            opacity,
            visible: l.visible(),
            blend_mode,
            clipping: l.clipping(),
            mask,
            locks: psd_locks(l.locks()),
            ..Layer::default()
        };
        match l.kind() {
            CoreKind::Raster => self.raster(l, &mut layer)?,
            CoreKind::Fill => {
                let v = l.fill_value(Channel::Color).expect("確かめた");
                layer.kind = LayerKind::SolidColor([v.r, v.g, v.b])
            }
            CoreKind::Adjustment => {
                let settings = l.adjustment().expect("確かめた");
                layer.kind = LayerKind::Adjustment(psd_adjustment(settings).expect("確かめた"))
            }
            CoreKind::Group => {
                let divider_id = unique_id(divider_part(l.id()), &mut self.used);
                layer.kind = LayerKind::Group {
                    children: self.level(Some(l.id()))?,
                    divider_id,
                }
            }
        }
        Ok(layer)
    }
    fn raster(&mut self, l: &CoreLayer, layer: &mut Layer) -> Result<()> {
        let d = self.d;
        let surface = l
            .surface(Channel::Color)
            .ok_or_else(|| Error::InvalidData("Color面がありません".into()))?;
        let (mut left, mut bottom, mut right, mut top) = (d.width(), d.height(), 0, 0);
        for c in surface.tile_coords() {
            left = left.min(c.x * d.tile_size());
            bottom = bottom.min(c.y * d.tile_size());
            right = d.width().min(right.max((c.x + 1) * d.tile_size()));
            top = d.height().min(top.max((c.y + 1) * d.tile_size()))
        }
        if right <= left || top <= bottom {
            left = 0;
            bottom = 0;
            right = 1;
            top = 1
        }
        let width = right - left;
        let height = top - bottom;
        self.budget += u64::from(width) * u64::from(height) * 4;
        check_budget(self.budget <= PIXEL_BUDGET, "PSD 投影の128 MiB画素予算超過")?;
        let mut pixels = vec![0; width as usize * height as usize * 4];
        for y in 0..height {
            for x in 0..width {
                let p = (y as usize * width as usize + x as usize) * 4;
                pixels[p..p + 4].copy_from_slice(&surface.pixel(left + x, top - 1 - y)?.to_array())
            }
        }
        layer.left = left as i32;
        layer.top = (d.height() - top) as i32;
        layer.width = width;
        layer.height = height;
        layer.pixels_rgba = pixels;
        Ok(())
    }
    /// core のマスク（隠す量をアルファに持ち、左下原点）→ PSD のマスク（255 が見える、上から下）。矩形は既定の値と違う画素の外接矩形で、
    /// 既定の値は 255 と 0 のうち矩形が小さくなるほう（同じなら 255）。画布のどこでも同じ値になる。
    fn mask(&mut self, m: &RasterMask) -> Result<Mask> {
        let d = self.d;
        let (w, h, ts) = (
            d.width() as usize,
            d.height() as usize,
            d.tile_size() as usize,
        );
        self.budget += (w * h) as u64;
        check_budget(self.budget <= PIXEL_BUDGET, "PSD 投影の128 MiB画素予算超過")?;
        // 無いタイルは何も隠さない（見える = 255）
        let mut values = vec![255u8; w * h];
        let surface = m.surface();
        let mut tile = vec![0u8; surface.tile_bytes()];
        for c in surface.tile_coords() {
            surface.copy_tile(c, &mut tile)?;
            let (x0, y0) = (c.x as usize * ts, c.y as usize * ts);
            for row in 0..ts.min(h - y0) {
                let psd_row = (h - 1 - (y0 + row)) * w + x0;
                for col in 0..ts.min(w - x0) {
                    values[psd_row + col] = 255 - tile[(row * ts + col) * 4 + 3]
                }
            }
        }
        let white = bounds(&values, w, 255);
        let black = bounds(&values, w, 0);
        let area = |b: (usize, usize, usize, usize)| b.2 * b.3;
        let background = if area(black) < area(white) { 0 } else { 255 };
        let mut rect = if background == 0 { black } else { white };
        // 一様なマスクも 1×1 の矩形を書く（空の矩形を誤って読む書き手がある）
        if rect.2 == 0 || rect.3 == 0 {
            rect = (0, 0, 1, 1)
        }
        let mut pixels = Vec::with_capacity(rect.2 * rect.3);
        for y in 0..rect.3 {
            pixels.extend_from_slice(&values[(rect.1 + y) * w + rect.0..][..rect.2])
        }
        Ok(Mask {
            left: rect.0 as i32,
            top: rect.1 as i32,
            width: rect.2 as u32,
            height: rect.3 as u32,
            default_color: background,
            enabled: m.enabled(),
            density: (m.density() * 255.0).round_ties_even() as u8,
            pixels,
        })
    }
}
/// 背景と違う値の外接矩形（左・上・幅・高さ。無ければ幅 0）。
fn bounds(values: &[u8], w: usize, background: u8) -> (usize, usize, usize, usize) {
    let h = values.len() / w;
    let (mut left, mut top, mut right, mut bottom) = (w, h, 0, 0);
    for (y, row) in values.chunks_exact(w).enumerate() {
        let Some(first) = row.iter().position(|v| *v != background) else {
            continue;
        };
        let last = row.iter().rposition(|v| *v != background).expect("あった");
        left = left.min(first);
        right = right.max(last + 1);
        top = top.min(y);
        bottom = bottom.max(y + 1);
    }
    if right <= left {
        (0, 0, 0, 0)
    } else {
        (left, top, right - left, bottom - top)
    }
}

impl ReadResult {
    pub fn to_core(&self) -> Result<CoreDocument> {
        check(
            self.mode == CompatibilityMode::EditableRaster,
            "PSD 原本は編集できません",
        )?;
        self.document
            .as_ref()
            .ok_or_else(|| Error::InvalidData("編集用文書がありません".into()))?
            .to_core()
    }
}
/// 層の並び（下から上）。グループの中身はグループのすぐ下に続き、親はグループの並びの位置。
struct Item<'a> {
    layer: &'a Layer,
    parent: Option<usize>,
}
fn count(l: &Layer) -> usize {
    match &l.kind {
        LayerKind::Group { children, .. } => 1 + children.iter().map(count).sum::<usize>(),
        _ => 1,
    }
}
fn order<'a>(top_down: &'a [Layer], parent: Option<usize>, out: &mut Vec<Item<'a>>) {
    for l in top_down.iter().rev() {
        if let LayerKind::Group { children, .. } = &l.kind {
            let at = out.len() + children.iter().map(count).sum::<usize>();
            order(children, Some(at), out);
        }
        out.push(Item { layer: l, parent })
    }
}
fn mask_off_canvas(m: &Mask, width: u32, height: u32) -> bool {
    let (w, h) = (i64::from(width), i64::from(height));
    (0..m.height).any(|y| {
        (0..m.width).any(|x| {
            let cx = i64::from(m.left) + i64::from(x);
            let cy = i64::from(m.top) + i64::from(y);
            (cx < 0 || cy < 0 || cx >= w || cy >= h)
                && m.pixels[y as usize * m.width as usize + x as usize] != m.default_color
        })
    })
}
impl Document {
    /// core に入れられない内容を、層ごとに 1 つ（`layers[2].children[0]: …` の形）返す。キャンバス外の画素・マスクを切り捨てて黙って
    /// 捨てることはしない。グループ・調整・塗りつぶし・マスク・ロックは core が持てる。
    pub fn core_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();
        self.collect_core_issues(&self.layers, "layers", &mut issues);
        issues
    }
    fn collect_core_issues(&self, layers: &[Layer], path: &str, issues: &mut Vec<String>) {
        for (i, l) in layers.iter().enumerate() {
            let p = format!("{path}[{i}]");
            match &l.kind {
                LayerKind::Group { children, .. } => {
                    self.collect_core_issues(children, &format!("{p}.children"), issues)
                }
                LayerKind::Raster
                    if l.left < 0
                        || l.top < 0
                        || i64::from(l.left) + i64::from(l.width) > i64::from(self.width)
                        || i64::from(l.top) + i64::from(l.height) > i64::from(self.height) =>
                {
                    issues.push(format!("{p}: キャンバス外の画素を切り捨てられません"))
                }
                _ => {}
            }
            if l.mask
                .as_ref()
                .is_some_and(|m| mask_off_canvas(m, self.width, self.height))
            {
                issues.push(format!(
                    "{p}.mask: キャンバス外にあるマスクの値を切り捨てられません"
                ))
            }
        }
    }
    /// ラスターの層の画素を core の Color の面へ（PSD は上から下、core は下から上）。
    fn import_pixels(&self, d: &mut CoreDocument, id: LayerId, l: &Layer) -> Result<()> {
        let ts = d.tile_size();
        let x0 = l.left as u32;
        let y0 = self.height - l.top as u32 - l.height;
        let x1 = x0 + l.width;
        let y1 = y0 + l.height;
        for ty in y0 / ts..y1.div_ceil(ts) {
            for tx in x0 / ts..x1.div_ceil(ts) {
                let mut tile = vec![0; (ts * ts * 4) as usize];
                for y in (ty * ts).max(y0)..((ty + 1) * ts).min(y1) {
                    for x in (tx * ts).max(x0)..((tx + 1) * ts).min(x1) {
                        let src =
                            ((self.height - 1 - y - l.top as u32) * l.width + x - x0) as usize * 4;
                        let dest = ((y % ts) * ts + x % ts) as usize * 4;
                        tile[dest..dest + 4].copy_from_slice(&l.pixels_rgba[src..src + 4])
                    }
                }
                d.import_tile(id, Channel::Color, TileCoord::new(tx, ty), &tile)?;
            }
        }
        Ok(())
    }
    /// マスクを core へ。core は隠す量（255 − PSD の値）を持ち、何も隠さないタイルは持たないので、隠す所のあるタイルだけを入れる。
    fn import_mask(&self, d: &mut CoreDocument, id: LayerId, m: &Mask) -> Result<()> {
        d.add_layer_mask(id)?;
        let (w, h, ts) = (self.width, self.height, d.tile_size());
        let mut tile = vec![0u8; (ts * ts * 4) as usize];
        for ty in 0..h.div_ceil(ts) {
            for tx in 0..w.div_ceil(ts) {
                let (x0, y0) = (tx * ts, ty * ts);
                // 既定の値が 255（隠さない）なら、矩形に触れないタイルは何も隠さない
                if m.default_color == 255 {
                    let (top, bottom) = (h - (y0 + ts).min(h), h - y0);
                    let outside = i64::from(m.left) >= i64::from((x0 + ts).min(w))
                        || i64::from(m.left) + i64::from(m.width) <= i64::from(x0)
                        || i64::from(m.top) >= i64::from(bottom)
                        || i64::from(m.top) + i64::from(m.height) <= i64::from(top);
                    if outside {
                        continue;
                    }
                }
                tile.fill(0);
                let mut hides = false;
                for row in 0..ts.min(h - y0) {
                    let psd_y = i64::from(h - 1 - (y0 + row));
                    for col in 0..ts.min(w - x0) {
                        let hide = 255 - m.at(i64::from(x0 + col), psd_y);
                        if hide != 0 {
                            tile[((row * ts + col) * 4 + 3) as usize] = hide;
                            hides = true
                        }
                    }
                }
                if hides {
                    d.import_mask_tile(id, TileCoord::new(tx, ty), &tile)?;
                }
            }
        }
        d.set_layer_mask_enabled(id, m.enabled)?;
        d.set_layer_mask_density(id, f64::from(m.density) / 255.0, false)?;
        Ok(())
    }
    pub fn to_core(&self) -> Result<CoreDocument> {
        super::write::validate(self, &Limits::default())?;
        let issues = self.core_issues();
        check(
            issues.is_empty(),
            format!("core 変換を拒否しました: {}", issues.join("、")),
        )?;
        let mut d = CoreDocument::new(self.width, self.height)?;
        let salt = d.id() & ((1u128 << 64) - 1);
        let mut items = Vec::new();
        order(&self.layers, None, &mut items);
        let mut made: Vec<LayerId> = Vec::with_capacity(items.len());
        for item in &items {
            let l = item.layer;
            let id = match &l.kind {
                LayerKind::Raster => {
                    let id = d.add_layer(&l.name)?;
                    self.import_pixels(&mut d, id, l)?;
                    id
                }
                LayerKind::Group { .. } => d.add_group(&l.name, None)?,
                LayerKind::SolidColor([r, g, b]) => d.add_fill_layer(
                    &l.name,
                    &[(Channel::Color, Rgba8::new(*r, *g, *b, 255))],
                    None,
                )?,
                LayerKind::Adjustment(a) => {
                    d.add_adjustment_layer(&l.name, core_adjustment(a)?, None, None)?
                }
            };
            d.set_layer_visible(id, l.visible)?;
            d.set_layer_opacity(id, f64::from(l.opacity) / 255.0, false)?;
            d.set_layer_blend_mode(
                id,
                yolu_core::BlendMode::from_index(l.blend_mode as u8).unwrap(),
            )?;
            d.set_layer_clipping(id, l.clipping)?;
            if let Some(m) = &l.mask {
                self.import_mask(&mut d, id, m)?;
            }
            made.push(id);
        }
        let parents: Vec<Option<LayerId>> =
            items.iter().map(|i| i.parent.map(|p| made[p])).collect();
        if parents.iter().any(Option::is_some) {
            d.set_structure_for_load(&parents)?;
        }
        // ロックは最後に付ける（取り込みの設定をロックが断らないように。履歴には残らない）
        for (item, id) in items.iter().zip(&made) {
            if item.layer.locks != 0 {
                d.set_locks_for_load(*id, core_locks(item.layer.locks))?;
            }
        }
        // 層 ID に PSD の層 ID（上位 32 bit）とグループの区切りの ID を持たせ、書き出し直すと同じ ID になる。残りは文書ごとに違う
        let ids: Vec<LayerId> = items
            .iter()
            .map(|i| {
                let divider = match i.layer.kind {
                    LayerKind::Group { divider_id, .. } => divider_bits(divider_id),
                    _ => 0,
                };
                LayerId(((i.layer.id as u128) << 96) | divider | salt)
            })
            .collect();
        let doc_id = d.id();
        Ok(d.with_persistent_ids(doc_id, &ids)?)
    }

    /// PSDへの新規投影。インポート原本の編集保存には、この結果と `write_edited` を使う。
    pub fn from_core(d: &CoreDocument) -> Result<Self> {
        check(
            !d.has_active_stroke(),
            "ストロークを確定・取消してからPSDを書き出してください",
        )?;
        let limits = Limits::default();
        check_budget(
            d.width() <= limits.max_dimension
                && d.height() <= limits.max_dimension
                && u64::from(d.width()) * u64::from(d.height()) <= limits.max_canvas_pixels,
            "PSD キャンバス予算超過",
        )?;
        check_budget(
            !d.layers().is_empty() && d.layers().len() <= limits.max_layers,
            "PSD レイヤー数の予算超過",
        )?;
        let layers = Export::new(d).level(None)?;
        let rgba = d.composite(d.bounds())?;
        let mut top_down = Vec::with_capacity(rgba.len());
        for row in rgba.chunks_exact(d.width() as usize * 4).rev() {
            top_down.extend(row)
        }
        let out = Self {
            width: d.width(),
            height: d.height(),
            layers,
            composite_rgba: Some(top_down),
        };
        super::write::validate(&out, &limits)?;
        Ok(out)
    }
}
