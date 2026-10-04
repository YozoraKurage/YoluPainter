//! M1のラスターColor文書への明示変換。名前によるレイヤー対応付けはしない。
use super::*;
use crate::{check, check_budget, Error, Result};
use std::collections::HashSet;
use yolu_core::{
    Channel, Document as CoreDocument, Layer as CoreLayer, LayerId, LayerKind as CoreKind,
    LayerLocks, TileCoord,
};
/// core の層のロック（1:透明部分・2:画素・4:位置・8:すべて）を PSD の lspf のビット（0:透明部分・1:画素・2:位置・31:すべて）へ。
/// すべては 0x80000000 だけで書く（書き手がその下の個別のビットを足さない。効くロックは同じ。`write` が決める）。
fn psd_locks(locks: LayerLocks) -> u32 {
    let mut bits = 0;
    for (lock, bit) in [
        (LayerLocks::TRANSPARENCY, 1),
        (LayerLocks::PIXELS, 2),
        (LayerLocks::POSITION, 4),
        (LayerLocks::ALL, 0x8000_0000),
    ] {
        if locks.contains(lock) {
            bits |= bit;
        }
    }
    bits
}
fn kind_label(kind: CoreKind) -> &'static str {
    match kind {
        CoreKind::Raster => "ラスター",
        CoreKind::Fill => "塗りつぶし",
        CoreKind::Adjustment => "調整",
        CoreKind::Group => "グループ",
    }
}
/// PSD へ写せない中身を、平らにも黙って落とすこともせず、機能ごとの理由で断る（C# の `PsdBridge.Export` の断りと対）。
/// 書き出すのは Color のラスター層の画素・表示・不透明度・合成モード・クリッピング・ロックだけ。ほかの中身は PSD の形
/// （マスク・グループ・塗りつぶし・調整・1 チャンネルの合成）が一部あっても、この写しがまだ書かないので、書けるようになるまで断る。
fn refuse_what_psd_cannot_hold(d: &CoreDocument, l: &CoreLayer) -> Result<()> {
    let name = l.name();
    check(
        l.kind() == CoreKind::Raster,
        format!(
            "層「{name}」は{}の層です。PSD に書けるのはラスターの層だけです",
            kind_label(l.kind())
        ),
    )?;
    check(
        l.mask().is_none(),
        format!("層「{name}」にマスクがあります。マスクはまだ PSD に書けません"),
    )?;
    check(
        l.channel_blends().next().is_none(),
        format!("層「{name}」にチャンネルごとの合成があります。PSD の層は合成モードと不透明度を 1 組しか持てません"),
    )?;
    // 効果（効いていない段・無効の段も）・Anchor・パス。`from_core` は層ごとに保存している元の画素を書き、効果入りの合成は統合画像
    // だけに入るので、書けば層の画素と統合画像が食い違い、設定は PSD に残らない（マスクの段はマスクで断っている）
    check(
        l.filters().is_empty(),
        format!("層「{name}」にフィルターか Generator があります。効果はまだ PSD に書けません"),
    )?;
    check(
        l.anchor().is_none(),
        format!("層「{name}」に Anchor があります。Anchor は PSD に書けません"),
    )?;
    check(
        l.path().is_none(),
        format!("層「{name}」にパスがあります。パスは PSD に書けません"),
    )?;
    // 標準の 6 つだけでなくユーザーチャンネル（番号 6 以降）も、面・有効の印・塗りつぶしの値のどれかがあれば断る
    let mut others = l.surface_channels();
    others.extend(l.enabled_channels());
    others.extend(l.fill_values().map(|(c, _)| c));
    others.retain(|c| *c != Channel::Color);
    others.sort();
    others.dedup();
    if let Some(&ch) = others.first() {
        let label = d
            .channel_info(ch)
            .map(|i| i.name.clone())
            .unwrap_or_else(|| format!("番号 {}", ch.index()));
        check(
            false,
            format!("層「{name}」は Color 以外のチャンネル（{label}）を使っています。PSD に書けるのは Color だけです"),
        )?
    }
    check(
        l.is_channel_enabled(Channel::Color),
        format!("層「{name}」の Color が無効です。無効にした Color の画素は PSD に書けません"),
    )
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
impl Document {
    pub fn core_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();
        for (i, l) in self.layers.iter().enumerate() {
            let p = format!("layers[{i}]");
            if l.kind != LayerKind::Raster {
                issues.push(format!("{p}: M1はラスター以外を保持できません"))
            }
            if l.mask.is_some() {
                issues.push(format!("{p}.mask: M1はマスクを保持できません"))
            }
            if l.locks != 0 {
                issues.push(format!("{p}.locks: M1はロックを保持できません"))
            }
            if l.left < 0
                || l.top < 0
                || i64::from(l.left) + i64::from(l.width) > i64::from(self.width)
                || i64::from(l.top) + i64::from(l.height) > i64::from(self.height)
            {
                issues.push(format!("{p}: キャンバス外の画素を切り捨てられません"))
            }
        }
        issues
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
        let mut ids = Vec::new();
        let ts = d.tile_size();
        for l in self.layers.iter().rev() {
            let id = d.add_layer(&l.name)?;
            ids.push(LayerId(((l.id as u128) << 96) | salt));
            d.set_layer_visible(id, l.visible)?;
            d.set_layer_opacity(id, f64::from(l.opacity) / 255.0, false)?;
            d.set_layer_blend_mode(
                id,
                yolu_core::BlendMode::from_index(l.blend_mode as u8).unwrap(),
            )?;
            d.set_layer_clipping(id, l.clipping)?;
            let x0 = l.left as u32;
            let y0 = self.height - l.top as u32 - l.height;
            let x1 = x0 + l.width;
            let y1 = y0 + l.height;
            for ty in y0 / ts..y1.div_ceil(ts) {
                for tx in x0 / ts..x1.div_ceil(ts) {
                    let mut tile = vec![0; (ts * ts * 4) as usize];
                    for y in (ty * ts).max(y0)..((ty + 1) * ts).min(y1) {
                        for x in (tx * ts).max(x0)..((tx + 1) * ts).min(x1) {
                            let src = ((self.height - 1 - y - l.top as u32) * l.width + x - x0)
                                as usize
                                * 4;
                            let dest = ((y % ts) * ts + x % ts) as usize * 4;
                            tile[dest..dest + 4].copy_from_slice(&l.pixels_rgba[src..src + 4])
                        }
                    }
                    d.import_tile(id, Channel::Color, TileCoord::new(tx, ty), &tile)?;
                }
            }
        }
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
        let mut layers = Vec::new();
        let mut used = HashSet::new();
        let mut budget = 0u64;
        for l in d.layers().iter().rev() {
            refuse_what_psd_cannot_hold(d, l)?;
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
            budget += u64::from(width) * u64::from(height) * 4;
            check_budget(budget <= 128 * 1024 * 1024, "PSD 投影の128 MiB画素予算超過")?;
            let mut pixels = vec![0; width as usize * height as usize * 4];
            for y in 0..height {
                for x in 0..width {
                    let p = (y as usize * width as usize + x as usize) * 4;
                    pixels[p..p + 4]
                        .copy_from_slice(&surface.pixel(left + x, top - 1 - y)?.to_array())
                }
            }
            let mut id = ((l.id().0 >> 96) as i32) & i32::MAX;
            if id == 0 {
                id = 1
            }
            while !used.insert(id) {
                id = if id == i32::MAX { 1 } else { id + 1 }
            }
            layers.push(Layer {
                id,
                name: l.name().into(),
                left: left as i32,
                top: (d.height() - top) as i32,
                width,
                height,
                opacity: (l.opacity() * 255.0).round_ties_even() as u8,
                visible: l.visible(),
                blend_mode: BlendMode::ALL[l.blend_mode() as usize],
                clipping: l.clipping(),
                pixels_rgba: pixels,
                locks: psd_locks(l.locks()),
                ..Layer::default()
            });
        }
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
