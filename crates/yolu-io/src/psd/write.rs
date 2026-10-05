use super::{binary::Emit, *};
use crate::{check, check_budget, Error, Result};
use std::borrow::Cow;
use std::collections::HashSet;
use std::io::{Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, Ordering};
const DIVIDER: &str = "</Layer group>";
/// PSD の辺の上限（画素）。
pub(super) const MAX_SIDE: u32 = 30000;
pub fn write_edited(origin: &ReadResult, edited: &Document, limits: &Limits) -> Result<Vec<u8>> {
    check(
        origin.mode == CompatibilityMode::EditableRaster,
        "PreserveOnly・Rejected 原本への編集書き戻しは禁止です",
    )?;
    write(edited, limits)
}
#[derive(Clone, Copy)]
pub(super) struct Record<'a> {
    pub(super) layer: &'a Layer,
    pub(super) divider: bool,
}
impl Record<'_> {
    pub(super) fn name(&self) -> &str {
        if self.divider {
            DIVIDER
        } else {
            &self.layer.name
        }
    }
    pub(super) fn id(&self) -> i32 {
        if self.divider {
            match self.layer.kind {
                LayerKind::Group { divider_id, .. } => divider_id,
                _ => 0,
            }
        } else {
            self.layer.id
        }
    }
    pub(super) fn mask(&self) -> Option<&Mask> {
        if self.divider {
            None
        } else {
            self.layer.mask.as_ref()
        }
    }
    pub(super) fn raster(&self) -> bool {
        !self.divider && self.layer.kind == LayerKind::Raster
    }
}
/// 層の並びを PSD の記録の順（下から上。グループは区切り・中身・グループ自身）に並べる。`pixels` が false のときは、画素に関わる確かめ（矩形・
/// 画素数・マスクの値）をしない（流して書くときの層の骨組みは画素を持たない。画素を渡すときに確かめる）。
fn flatten<'a>(
    layers: &'a [Layer],
    depth: usize,
    out: &mut Vec<Record<'a>>,
    ids: &mut HashSet<i32>,
    limits: &Limits,
    pixels: bool,
) -> Result<()> {
    for l in layers.iter().rev() {
        check(
            l.id > 0 && ids.insert(l.id),
            "PSD IDは正の一意な整数が必要です",
        )?;
        check(
            !l.name.contains('\0') && l.name.encode_utf16().count() <= limits.max_name_code_units,
            "PSD の名前が不正、または長すぎます",
        )?;
        check(l.locks & !0x80000007 == 0, "未対応のロックビット")?;
        if let Some(m) = l.mask.as_ref().filter(|_| pixels) {
            rectangle(m.left, m.top, m.width, m.height, limits)?;
            check(
                matches!(m.default_color, 0 | 255)
                    && m.pixels.len() as u64 == u64::from(m.width) * u64::from(m.height),
                "マスク値・画素数が不正です",
            )?
        }
        match &l.kind {
            LayerKind::Group {
                children,
                divider_id,
            } => {
                check_budget(depth < limits.max_group_depth, "グループ深さの予算超過")?;
                check(
                    *divider_id >= 0 && (*divider_id == 0 || ids.insert(*divider_id)),
                    "区切りIDが不正・重複しています",
                )?;
                out.push(Record {
                    layer: l,
                    divider: true,
                });
                check_budget(out.len() <= limits.max_layers, "レイヤー記録数の予算超過")?;
                flatten(children, depth + 1, out, ids, limits, pixels)?
            }
            LayerKind::Raster => {
                check(
                    l.blend_mode != BlendMode::PassThrough,
                    "通過モードはグループ専用です",
                )?;
                if pixels {
                    rectangle(l.left, l.top, l.width, l.height, limits)?;
                    check(
                        l.width > 0
                            && l.height > 0
                            && l.pixels_rgba.len() as u64
                                == u64::from(l.width) * u64::from(l.height) * 4,
                        "ラスター画素数が矩形と不一致です",
                    )?
                }
            }
            LayerKind::Adjustment(a) => {
                validate_adjustment(a)?;
                check(
                    l.blend_mode != BlendMode::PassThrough,
                    "調整に通過モードは使えません",
                )?
            }
            LayerKind::SolidColor(_) => check(
                l.blend_mode != BlendMode::PassThrough,
                "塗りつぶしに通過モードは使えません",
            )?,
        }
        out.push(Record {
            layer: l,
            divider: false,
        });
        check_budget(out.len() <= limits.max_layers, "レイヤー記録数の予算超過")?;
    }
    Ok(())
}
fn rectangle(x: i32, y: i32, w: u32, h: u32, l: &Limits) -> Result<()> {
    check(
        w <= l.max_dimension
            && h <= l.max_dimension
            && u64::from(w) * u64::from(h) <= l.max_canvas_pixels
            && i64::from(x) + i64::from(w) <= i64::from(i32::MAX)
            && i64::from(y) + i64::from(h) <= i64::from(i32::MAX),
        "PSD の矩形が不正、または予算超過です",
    )
}
/// 曲線の点が書ける形か（2〜19 点・入力は昇順）。
fn curve_ok(points: &[[u8; 2]]) -> bool {
    (2..=19).contains(&points.len()) && points.windows(2).all(|w| w[0][0] < w[1][0])
}
/// 分岐点の位置が昇順で、位置・中点が範囲に収まるか。
fn stops_ok(count: usize, positions: impl Iterator<Item = (u16, u8)>) -> bool {
    let mut last: Option<u16> = None;
    let mut n = 0;
    for (location, midpoint) in positions {
        if location > 4096 || midpoint > 100 || last.is_some_and(|l| location <= l) {
            return false;
        }
        last = Some(location);
        n += 1;
    }
    n == count && (2..=32).contains(&n)
}
fn validate_adjustment(a: &Adjustment) -> Result<()> {
    check(
        match a {
            Adjustment::Invert => true,
            &Adjustment::Levels {
                input_black: ib,
                input_white: iw,
                output_black: ob,
                output_white: ow,
                gamma: g,
            } => {
                ib <= 253
                    && (2..=255).contains(&iw)
                    && iw > ib
                    && ob <= 255
                    && ow <= 255
                    && (10..=999).contains(&g)
            }
            &Adjustment::HueSaturation {
                hue: h,
                saturation: s,
                lightness: l,
            } => {
                (-180..=180).contains(&h) && (-100..=100).contains(&s) && (-100..=100).contains(&l)
            }
            Adjustment::GradientMap {
                colors, opacities, ..
            } => {
                stops_ok(
                    colors.len(),
                    colors.iter().map(|c| (c.location, c.midpoint)),
                ) && stops_ok(
                    opacities.len(),
                    opacities.iter().map(|c| (c.location, c.midpoint)),
                )
            }
            Adjustment::ToneCurve {
                composite,
                red,
                green,
                blue,
            } => [composite, red, green, blue]
                .into_iter()
                .all(|c| curve_ok(c)),
            Adjustment::ColorBalance {
                shadows,
                midtones,
                highlights,
                ..
            } => [shadows, midtones, highlights]
                .into_iter()
                .flatten()
                .all(|v| (-100..=100).contains(v)),
            &Adjustment::BrightnessContrast {
                brightness,
                contrast,
            } => (-150..=150).contains(&brightness) && (-100..=100).contains(&contrast),
            &Adjustment::Threshold { level } => (1..=255).contains(&level),
            &Adjustment::Posterize { levels } => (2..=255).contains(&levels),
        },
        "調整がPSDの刻み・範囲に収まりません",
    )
}
/// 書く前の確かめ（無圧縮で書いたときの長さが出力の上限に入るかも見る。Unity 版の書き手と対の厳密な書き出し・`write` の確かめ）。
pub(super) fn validate(d: &Document, limits: &Limits) -> Result<()> {
    validate_for(d, limits, Compression::Raw)
}
/// 書く前の確かめ。RLE は書きながら圧縮したあとの長さを上限と比べる（`stream` の `FileSize`）ので、無圧縮で書いたときの長さは見ない
/// （無圧縮では 2 GiB を超えても、圧縮すれば収まる文書を断らない）。
pub(super) fn validate_for(d: &Document, limits: &Limits, compression: Compression) -> Result<()> {
    preflight(d, limits, compression).map(|_| ())
}
fn preflight<'a>(
    d: &'a Document,
    limits: &Limits,
    compression: Compression,
) -> Result<(Vec<Record<'a>>, usize)> {
    limits.validate()?;
    rectangle(0, 0, d.width, d.height, limits)?;
    check(
        d.width > 0 && d.height > 0 && !d.layers.is_empty(),
        "PSD は空のキャンバス・レイヤー一覧を持てません",
    )?;
    let canvas = u64::from(d.width) * u64::from(d.height) * 4;
    check(
        d.composite_rgba
            .as_ref()
            .is_none_or(|p| p.len() as u64 == canvas),
        "統合RGBAの大きさが不一致です",
    )?;
    let mut records = Vec::new();
    flatten(&d.layers, 0, &mut records, &mut HashSet::new(), limits, true)?;
    let mut pixels = canvas;
    let mut metadata = 0u64;
    let mut info = 2u64;
    for r in &records {
        let units = r.name().encode_utf16().count() as u64;
        let pascal = (units.min(255) + 1).div_ceil(4) * 4;
        let section = if r.divider {
            16
        } else {
            match &r.layer.kind {
                LayerKind::Raster => 0,
                LayerKind::Group { .. } => 24,
                LayerKind::Adjustment(a) => adjustment_blocks(a)
                    .iter()
                    .map(|(_, body)| 12 + (body.len() + body.len() % 2) as u64)
                    .sum(),
                LayerKind::SolidColor(c) => 12 + solid_block(*c).len() as u64,
            }
        };
        let extra = 8
            + pascal
            + 16
            + units * 2
            + if r.id() != 0 { 16 } else { 0 }
            + if !r.divider && r.layer.locks != 0 {
                16
            } else {
                0
            }
            + section
            + if r.mask().is_some() { 20 } else { 0 };
        let layer = if r.raster() {
            u64::from(r.layer.width) * u64::from(r.layer.height) * 4
        } else {
            0
        };
        let mask = r.mask().map_or(0, |m| m.pixels.len() as u64);
        pixels += layer + mask;
        metadata += extra;
        info += 58 + extra + 8 + layer + if r.mask().is_some() { 8 + mask } else { 0 };
    }
    check_budget(
        pixels <= limits.max_decoded_bytes && metadata <= limits.max_metadata_bytes as u64,
        "PSD の画素・メタデータ予算超過",
    )?;
    info += info & 1;
    let total = 26 + 4 + 4 + 4 + 4 + info + 4 + 2 + canvas;
    if compression == Compression::Raw {
        check_budget(
            total <= limits.max_output_bytes as u64 && total <= i32::MAX as u64,
            "PSD 出力予算超過",
        )?;
    }
    Ok((records, total.min(usize::MAX as u64) as usize))
}
/// 層・マスク・統合画像のチャンネルを書く圧縮。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Compression {
    /// 無圧縮。Unity 版の書き手と同じバイト列になる（厳密な書き出しの照合はこれ）。
    #[default]
    Raw,
    /// RLE（PackBits。Photoshop の既定）。層・マスクのチャンネルは 1 つずつ、圧縮して小さくならなければ無圧縮のまま書く。統合画像は全チャンネルで
    /// 1 つの圧縮なので、小さくならなければ無圧縮で書く。
    Rle,
}

/// 書き出しが予算・形式の上限で断る理由（層の名前つき。画面は種類から画面の言語の文を作る。`Error::Budget` の文は [`Overrun::message`]）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Overrun {
    /// キャンバスの画素の数が予算に入らない。
    Canvas { width: u32, height: u32 },
    /// キャンバスの辺が上限（`limit`）を超える。PSD の形式の上限は 30000 で、それより小さい上限（予算を決めない既定の上限）なら予算を決めれば書ける。
    Side { width: u32, height: u32, limit: u32 },
    /// 層の記録（グループは区切りの記録も要る）が上限を超える。
    Layers { count: usize, limit: usize },
    /// 全層の画素をメモリに組む書き出し（Normal の焼き込み・平らの 1 枚）で、層を足すと画素の予算を超える。`layer` が空なら 1 枚の作業の領域。
    Memory { layer: String },
    /// 層の付加情報（名前・調整の設定）の合計が予算を超える。
    Extra,
    /// PSD は 1 ファイル 2 GiB まで。この層を書くと超える（圧縮したあとの大きさ。`layer` が空なら統合画像）。
    FileSize { layer: String },
}
impl Overrun {
    /// 設定の「レイヤーのメモリ」の予算を上げると書けるようになる理由か（PSD の 2 GiB・辺 30000 は形式の上限なので、上げても書けない）。
    pub fn raised_by_budget(&self) -> bool {
        match self {
            Self::FileSize { .. } => false,
            Self::Side { limit, .. } => *limit < MAX_SIDE,
            _ => true,
        }
    }
    /// 日本語の診断（層の名前つき）。
    pub fn message(&self) -> String {
        match self {
            Self::Canvas { width, height } => {
                format!("キャンバス（{width}×{height}）が書き出しの画素の予算を超えます")
            }
            Self::Side {
                width,
                height,
                limit,
            } => format!("キャンバス（{width}×{height}）の辺が上限（{limit}）を超えます"),
            Self::Layers { count, limit } => format!(
                "レイヤーの記録が予算を超えます（{count} 件・上限 {limit} 件。グループは区切りの記録も要ります）"
            ),
            Self::Memory { layer } if layer.is_empty() => {
                "画素の予算を超えます（1 枚ぶんの作業の領域）".into()
            }
            Self::Memory { layer } => format!("層「{layer}」を足すと画素の予算を超えます"),
            Self::Extra => "層の付加情報（名前・調整の設定）が予算を超えます".into(),
            Self::FileSize { layer } if layer.is_empty() => {
                "統合画像を書くと PSD の大きさが上限（2 GiB）を超えます。PSD は 1 ファイル 2 GiB までです".into()
            }
            Self::FileSize { layer } => format!(
                "層「{layer}」を書くと PSD の大きさが上限（2 GiB）を超えます。PSD は 1 ファイル 2 GiB までです"
            ),
        }
    }
}
impl From<Overrun> for Error {
    fn from(o: Overrun) -> Self {
        Error::Budget(o.message())
    }
}

/// 書き出しの失敗。予算・形式の上限は種類を保つ（画面が層の名前つきの文とツールチップを作る）。ほかは [`Error`]（取消は `Error::Core(Cancelled)`）。
#[derive(Debug)]
pub enum ExportError {
    Overrun(Overrun),
    Other(Error),
}
impl From<Error> for ExportError {
    fn from(e: Error) -> Self {
        Self::Other(e)
    }
}
impl From<Overrun> for ExportError {
    fn from(o: Overrun) -> Self {
        Self::Overrun(o)
    }
}
impl From<std::io::Error> for ExportError {
    fn from(e: std::io::Error) -> Self {
        Self::Other(Error::Io(e))
    }
}
impl From<ExportError> for Error {
    fn from(e: ExportError) -> Self {
        match e {
            ExportError::Overrun(o) => o.into(),
            ExportError::Other(e) => e,
        }
    }
}
impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Overrun(o) => f.write_str(&o.message()),
            Self::Other(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for ExportError {}
pub(super) type XResult<T> = std::result::Result<T, ExportError>;

/// 書き終えた PSD。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Written {
    /// ファイルの大きさ（バイト）。
    pub bytes: u64,
    /// PSD の層の数（グループの区切りの記録を除く。取り込んだ文書の層の数と同じになる）。
    pub layers: usize,
    /// 書いたバイト列の照合用の値（読み戻したファイルが書いたとおりか、[`Checksum::matches`] で確かめる）。
    pub checksum: Checksum,
}

/// 書いたファイルの照合用の値（CRC-32 を 2 つと長さ）。流して書くと、先頭（ヘッダーと層の記録の表）は画素から決まる欄を最後に確定した値で書き直すので、
/// 先頭と、その後ろ（層の画素・統合画像。書いた順のまま）を別々に数える。構造を壊さない画素のビット化け・書き込みの取り違えを読み戻しで見つける
/// （バイト単位の完全な比較ではなく CRC-32 なので、化けを 2^-32 の確率で見逃す）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Checksum {
    head_len: u64,
    head: u32,
    tail: u32,
    len: u64,
}
impl Checksum {
    /// `reader`（ファイルの先頭から）を最後まで読んで、書いたバイト列と一致するか。長さ・先頭・後ろの 3 つが合うときだけ true。取消は `Error::Core(Cancelled)`。
    pub fn matches<R: std::io::Read>(
        &self,
        reader: &mut R,
        cancel: Option<&AtomicBool>,
    ) -> Result<bool> {
        let mut buf = vec![0u8; 256 * 1024];
        let (mut head, mut tail) = (crc32fast::Hasher::new(), crc32fast::Hasher::new());
        let mut pos = 0u64;
        loop {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Err(Error::Core(yolu_core::CoreError::Cancelled));
            }
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            };
            let in_head = self.head_len.saturating_sub(pos).min(n as u64) as usize;
            head.update(&buf[..in_head]);
            tail.update(&buf[in_head..n]);
            pos += n as u64;
            if pos > self.len {
                return Ok(false);
            }
        }
        Ok(pos == self.len && head.finalize() == self.head && tail.finalize() == self.tail)
    }
}

/// 書いたバイトを CRC-32 に数えながら中へ渡す（先頭の書き直しより後ろの、順に書く区間）。
struct Tail<'a, W: Write> {
    inner: &'a mut W,
    crc: crc32fast::Hasher,
}
impl<W: Write> Write for Tail<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.crc.update(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// 流して書くときに、1 つの記録（層・区切り）の画素を渡す形。画素は渡したあとすぐ捨てられる。
pub(super) struct Region<'a> {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub rgba: Cow<'a, [u8]>,
}
pub(super) struct MaskRegion<'a> {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub default_color: u8,
    pub values: Cow<'a, [u8]>,
}
#[derive(Default)]
pub(super) struct Supplied<'a> {
    /// ラスターの記録（`Record::raster`）の画素。
    pub raster: Option<Region<'a>>,
    /// マスクのある記録のマスクの値。
    pub mask: Option<MaskRegion<'a>>,
}

pub(super) struct StreamOptions<'a> {
    pub compression: Compression,
    pub limits: &'a Limits,
    /// 書くファイルの大きさの上限（PSD は 2 GiB = `i32::MAX`）。
    pub max_file_bytes: u64,
    pub cancel: Option<&'a AtomicBool>,
}

/// 記録の骨組みだけの層の並び（画素を持たない層）を、書く順（PSD の記録の順）に並べる。構造・名前・ID・調整の確かめだけで、画素は渡すときに確かめる。
pub(super) fn skeleton_records<'a>(
    d: &'a Document,
    limits: &Limits,
) -> Result<Vec<Record<'a>>> {
    limits.validate()?;
    rectangle(0, 0, d.width, d.height, limits)?;
    check(
        d.width > 0 && d.height > 0 && !d.layers.is_empty(),
        "PSD は空のキャンバス・レイヤー一覧を持てません",
    )?;
    let mut records = Vec::new();
    flatten(&d.layers, 0, &mut records, &mut HashSet::new(), limits, false)?;
    Ok(records)
}

/// 記録の中の、あとで画素から決まる欄の場所（`head` の中の位置）。
struct Patch {
    rect_at: usize,
    lens_at: usize,
    mask_at: Option<usize>,
}

/// 層の記録 1 つを、画素から決まる欄（矩形・チャンネルの長さ・マスクの矩形と既定値）を仮の値にして書く。付加情報の大きさを返す。
fn emit_record(r: &Record, info: &mut Vec<u8>) -> (Patch, u64) {
    let l = r.layer;
    let rect_at = info.len();
    info.extend([0; 16]);
    info.u16(if r.mask().is_some() { 5 } else { 4 });
    let lens_at = info.len();
    for id in [0, 1, 2, -1i16] {
        info.u16(id as u16);
        info.u32(2)
    }
    if r.mask().is_some() {
        info.u16((-2i16) as u16);
        info.u32(2)
    }
    info.extend(b"8BIM");
    if r.divider {
        info.extend(b"norm");
        info.extend([255, 0, 0, 0])
    } else {
        info.extend(if l.blend_mode == BlendMode::PassThrough {
            *b"norm"
        } else {
            l.blend_mode.key()
        });
        let flag = u8::from(!l.visible) * 2
            + u8::from(l.locks & 1 != 0 && l.locks & 0x80000000 == 0)
            + if matches!(l.kind, LayerKind::Adjustment(_) | LayerKind::SolidColor(_)) {
                24
            } else {
                0
            };
        info.extend([l.opacity, u8::from(l.clipping), flag, 0])
    }
    let mut e = Vec::new();
    let mut mask_in_extra = None;
    if let Some(m) = r.mask() {
        e.u32(20);
        mask_in_extra = Some(e.len());
        e.extend([0; 16]);
        e.push(0);
        e.push(u8::from(!m.enabled) * 2 + if m.density != 255 { 16 } else { 0 });
        if m.density != 255 {
            e.extend([1, m.density])
        } else {
            e.extend([0; 2])
        }
    } else {
        e.u32(0)
    }
    e.u32(0);
    let units: Vec<_> = r.name().encode_utf16().collect();
    let n = units.len().min(255);
    e.push(n as u8);
    e.extend(
        units[..n]
            .iter()
            .map(|v| if *v <= 127 { *v as u8 } else { b'?' }),
    );
    e.extend(vec![0; (4 - (n + 1) % 4) % 4]);
    let mut name = Vec::new();
    name.u32(units.len() as u32);
    for v in units {
        name.u16(v)
    }
    e.tag(b"luni", &name);
    if r.id() != 0 {
        e.tag(b"lyid", &r.id().to_be_bytes())
    }
    if !r.divider && l.locks != 0 {
        let bits = if l.locks & 0x80000000 != 0 {
            0x80000000u32
        } else {
            l.locks
        };
        e.tag(b"lspf", &bits.to_be_bytes())
    }
    if r.divider {
        e.tag(b"lsct", &3u32.to_be_bytes())
    } else {
        match &l.kind {
            LayerKind::Raster => {}
            LayerKind::Group { .. } => {
                let mut body = Vec::new();
                body.u32(1);
                body.extend(b"8BIM");
                body.extend(l.blend_mode.key());
                e.tag(b"lsct", &body)
            }
            LayerKind::Adjustment(a) => {
                for (key, body) in adjustment_blocks(a) {
                    e.tag(&key, &body)
                }
            }
            LayerKind::SolidColor(c) => e.tag(b"SoCo", &solid_block(*c)),
        }
    }
    let section_at = info.len();
    info.section(&e);
    let patch = Patch {
        rect_at,
        lens_at,
        mask_at: mask_in_extra.map(|at| section_at + 4 + at),
    };
    (patch, e.len() as u64)
}

fn put_bounds(at: &mut [u8], x: i32, y: i32, width: u32, height: u32) {
    let mut v = Vec::with_capacity(16);
    bounds(&mut v, x, y, width, height);
    at[..16].copy_from_slice(&v)
}

/// PackBits の 1 行（行をまたぐ繰り返しは作らない）。3 つ以上の同じ値は繰り返し、それ以外は 128 までの並びにする。
pub(super) fn packbits_row(row: &[u8], out: &mut Vec<u8>) {
    let n = row.len();
    let mut i = 0;
    while i < n {
        let b = row[i];
        let mut run = 1;
        while i + run < n && run < 128 && row[i + run] == b {
            run += 1
        }
        if run >= 3 {
            out.push((257 - run) as u8);
            out.push(b);
            i += run
        } else {
            let start = i;
            let mut len = 0;
            while i < n && len < 128 {
                if i + 2 < n && row[i] == row[i + 1] && row[i] == row[i + 2] {
                    break;
                }
                i += 1;
                len += 1
            }
            out.push((len - 1) as u8);
            out.extend_from_slice(&row[start..i])
        }
    }
}

/// チャンネルを書く間の作業の置き場（層ごとに作り直さない）。
#[derive(Default)]
struct Scratch {
    row: Vec<u8>,
    data: Vec<u8>,
    table: Vec<u16>,
}

fn cancelled(cancel: Option<&AtomicBool>) -> XResult<()> {
    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
        Err(Error::Core(yolu_core::CoreError::Cancelled).into())
    } else {
        Ok(())
    }
}

/// チャンネル 1 つ（幅 `w`・高さ `h`。`fill(y, row)` が y 行目を作る）を、圧縮の印・行の長さの表・データの順に書き、書いた長さ（印を含む）を返す。
/// RLE は、圧縮して小さくならない（表を含めた長さが無圧縮以上）か、行が 65535 バイトに入らなければ、無圧縮で書く。空のチャンネルは印だけ。
fn write_channel<W: Write>(
    out: &mut W,
    w: usize,
    h: usize,
    compression: Compression,
    sc: &mut Scratch,
    cancel: Option<&AtomicBool>,
    mut fill: impl FnMut(usize, &mut [u8]),
) -> XResult<u32> {
    if w == 0 || h == 0 {
        out.write_all(&[0, 0])?;
        return Ok(2);
    }
    let raw = 2 + w * h;
    sc.row.resize(w, 0);
    if compression == Compression::Rle {
        let overhead = 2 + 2 * h;
        sc.data.clear();
        sc.table.clear();
        let mut fits = true;
        for y in 0..h {
            if y % 128 == 0 {
                cancelled(cancel)?;
            }
            fill(y, &mut sc.row);
            let before = sc.data.len();
            packbits_row(&sc.row, &mut sc.data);
            let n = sc.data.len() - before;
            if n > usize::from(u16::MAX) || overhead + sc.data.len() >= raw {
                fits = false;
                break;
            }
            sc.table.push(n as u16)
        }
        if fits {
            out.write_all(&1u16.to_be_bytes())?;
            let mut table = Vec::with_capacity(sc.table.len() * 2);
            for n in &sc.table {
                table.extend(n.to_be_bytes())
            }
            out.write_all(&table)?;
            out.write_all(&sc.data)?;
            return Ok((overhead + sc.data.len()) as u32);
        }
    }
    out.write_all(&[0, 0])?;
    for y in 0..h {
        if y % 128 == 0 {
            cancelled(cancel)?;
        }
        fill(y, &mut sc.row);
        out.write_all(&sc.row)?
    }
    Ok(raw as u32)
}

/// 統合画像（RGBA。上の行から。透明は白へ寄せたもの）を書く。圧縮は全チャンネルで 1 つ。RLE は、4 チャンネルを圧縮してみて、小さくならなければ無圧縮。
fn write_merged<W: Write>(
    out: &mut W,
    merged: &[u8],
    w: usize,
    h: usize,
    compression: Compression,
    sc: &mut Scratch,
    cancel: Option<&AtomicBool>,
) -> XResult<u64> {
    let plane = |c: usize, y: usize, row: &mut [u8]| {
        for (d, px) in row.iter_mut().zip(merged[y * w * 4..(y + 1) * w * 4].as_chunks::<4>().0) {
            *d = px[c]
        }
    };
    let raw = 2 + 4 * w * h;
    sc.row.resize(w, 0);
    if compression == Compression::Rle {
        let overhead = 2 + 4 * h * 2;
        sc.data.clear();
        sc.table.clear();
        let mut fits = true;
        'planes: for c in 0..4 {
            cancelled(cancel)?;
            for y in 0..h {
                plane(c, y, &mut sc.row);
                let before = sc.data.len();
                packbits_row(&sc.row, &mut sc.data);
                let n = sc.data.len() - before;
                if n > usize::from(u16::MAX) || overhead + sc.data.len() >= raw {
                    fits = false;
                    break 'planes;
                }
                sc.table.push(n as u16)
            }
        }
        if fits {
            out.write_all(&1u16.to_be_bytes())?;
            let mut table = Vec::with_capacity(sc.table.len() * 2);
            for n in &sc.table {
                table.extend(n.to_be_bytes())
            }
            out.write_all(&table)?;
            out.write_all(&sc.data)?;
            return Ok((overhead + sc.data.len()) as u64);
        }
    }
    out.write_all(&[0, 0])?;
    for c in 0..4 {
        cancelled(cancel)?;
        for y in 0..h {
            plane(c, y, &mut sc.row);
            out.write_all(&sc.row)?
        }
    }
    Ok(raw as u64)
}

fn check_region(r: &Region, limits: &Limits) -> Result<()> {
    rectangle(r.left, r.top, r.width, r.height, limits)?;
    check(
        r.width > 0
            && r.height > 0
            && r.rgba.len() as u64 == u64::from(r.width) * u64::from(r.height) * 4,
        "ラスター画素数が矩形と不一致です",
    )
}
fn check_mask_region(m: &MaskRegion, limits: &Limits) -> Result<()> {
    rectangle(m.left, m.top, m.width, m.height, limits)?;
    check(
        matches!(m.default_color, 0 | 255)
            && m.values.len() as u64 == u64::from(m.width) * u64::from(m.height),
        "マスク値・画素数が不正です",
    )
}

/// PSD を流して書く。先に、ヘッダーと全層の記録（画素から決まる欄は仮の値）を書き、層の画素を記録の順に 1 枚ずつ `supply` から受けて圧縮して書き
/// （受けた画素はすぐ捨てる。メモリには層 1 枚ぶんと記録の表しか持たない）、最後に統合画像（`composite`。層を書いたあとに作る）を書いて、先頭へ戻って
/// 画素から決まる欄（矩形・チャンネルの長さ・マスクの矩形・各区間の長さ）を確定した記録で書き直す。書いたファイルの大きさと、書いたバイト列の照合用の値を返す。
/// `out` は書き始めの位置が先頭で、書き直しのために Seek が要る。途中で失敗したら、書きかけを残さないのは呼び手の仕事（一時ファイルを消す）。
pub(super) fn stream<'a, W: Write + Seek>(
    out: &mut W,
    width: u32,
    height: u32,
    records: &[Record<'_>],
    opts: &StreamOptions<'_>,
    mut supply: impl FnMut(usize) -> XResult<Supplied<'a>>,
    composite: impl FnOnce() -> XResult<Cow<'a, [u8]>>,
) -> XResult<(u64, Checksum)> {
    let limits = opts.limits;
    limits.validate()?;
    rectangle(0, 0, width, height, limits)?;
    check(
        width > 0 && height > 0 && !records.is_empty(),
        "PSD は空のキャンバス・レイヤー一覧を持てません",
    )?;
    check_budget(
        records.len() <= limits.max_layers && records.len() <= 32767,
        "レイヤー記録数の予算超過",
    )?;
    let (w, h) = (width as usize, height as usize);
    // ヘッダー・色モードデータ（空）・画像リソース（空）・レイヤーとマスクの情報の長さ・レイヤー情報の長さ・層の数・層の記録
    let mut head = Vec::new();
    head.extend(b"8BPS");
    head.u16(1);
    head.extend([0; 6]);
    head.u16(4);
    head.u32(height);
    head.u32(width);
    head.u16(8);
    head.u16(3);
    head.u32(0);
    head.u32(0);
    let lm_len_at = head.len();
    head.u32(0);
    let info_len_at = head.len();
    head.u32(0);
    head.u16((-(records.len() as i16)) as u16);
    let records_at = head.len();
    let mut patches = Vec::with_capacity(records.len());
    let mut extra = 0u64;
    for r in records {
        let (patch, n) = emit_record(r, &mut head);
        patches.push(patch);
        extra += n
    }
    if extra > limits.max_metadata_bytes as u64 {
        return Err(Overrun::Extra.into());
    }
    let records_len = (head.len() - records_at) as u64;
    out.write_all(&head)?;
    let mut pos = head.len() as u64;
    let head_len = pos;
    // ここから後ろは順に書く（先頭は最後に書き直す）ので、書きながら数える
    let mut out = Tail {
        inner: &mut *out,
        crc: crc32fast::Hasher::new(),
    };
    let too_big = |pos: u64, layer: &str| -> XResult<()> {
        if pos > opts.max_file_bytes {
            Err(Overrun::FileSize {
                layer: layer.to_owned(),
            }
            .into())
        } else {
            Ok(())
        }
    };
    let mut sc = Scratch::default();
    let mut data_len = 0u64;
    for (i, r) in records.iter().enumerate() {
        cancelled(opts.cancel)?;
        let wanted = r.raster() || r.mask().is_some();
        let supplied = if wanted { supply(i)? } else { Supplied::default() };
        check(
            supplied.raster.is_some() == r.raster() && supplied.mask.is_some() == r.mask().is_some(),
            "PSD に書く画素が記録と一致しません",
        )?;
        let patch = &patches[i];
        let mut lens = [2u32; 5];
        let mut written = 0u64;
        for c in 0..4 {
            let len = match &supplied.raster {
                Some(region) => {
                    check_region(region, limits)?;
                    if c == 0 {
                        put_bounds(
                            &mut head[patch.rect_at..],
                            region.left,
                            region.top,
                            region.width,
                            region.height,
                        )
                    }
                    let rgba: &[u8] = &region.rgba;
                    let rw = region.width as usize;
                    write_channel(
                        &mut out,
                        rw,
                        region.height as usize,
                        opts.compression,
                        &mut sc,
                        opts.cancel,
                        |y, row| {
                            for (d, px) in row.iter_mut().zip(rgba[y * rw * 4..(y + 1) * rw * 4].as_chunks::<4>().0) {
                                *d = px[c]
                            }
                        },
                    )?
                }
                None => {
                    out.write_all(&[0, 0])?;
                    2
                }
            };
            lens[c] = len;
            written += u64::from(len)
        }
        if let Some(m) = &supplied.mask {
            check_mask_region(m, limits)?;
            let mw = m.width as usize;
            let values: &[u8] = &m.values;
            let len = write_channel(
                &mut out,
                mw,
                m.height as usize,
                opts.compression,
                &mut sc,
                opts.cancel,
                |y, row| row.copy_from_slice(&values[y * mw..(y + 1) * mw]),
            )?;
            lens[4] = len;
            written += u64::from(len);
            let at = patch.mask_at.expect("マスクのある記録");
            put_bounds(&mut head[at..], m.left, m.top, m.width, m.height);
            head[at + 16] = m.default_color
        }
        for (k, len) in lens.iter().take(4 + usize::from(supplied.mask.is_some())).enumerate() {
            let at = patch.lens_at + k * 6 + 2;
            head[at..at + 4].copy_from_slice(&len.to_be_bytes())
        }
        data_len += written;
        pos += written;
        too_big(pos, r.name())?
    }
    // レイヤー情報は偶数の長さ。続けて全体のレイヤーマスク情報（空）
    let mut info_len = 2 + records_len + data_len;
    if !info_len.is_multiple_of(2) {
        out.write_all(&[0])?;
        info_len += 1;
        pos += 1
    }
    out.write_all(&[0; 4])?;
    pos += 4;
    head[lm_len_at..lm_len_at + 4].copy_from_slice(&((info_len + 8) as u32).to_be_bytes());
    head[info_len_at..info_len_at + 4].copy_from_slice(&(info_len as u32).to_be_bytes());
    // 統合画像
    cancelled(opts.cancel)?;
    let mut merged = composite()?.into_owned();
    check(
        merged.len() as u64 == u64::from(width) * u64::from(height) * 4,
        "統合RGBAの大きさが不一致です",
    )?;
    super::composite::matte(&mut merged);
    pos += write_merged(&mut out, &merged, w, h, opts.compression, &mut sc, opts.cancel)?;
    drop(merged);
    too_big(pos, "")?;
    let tail = out.crc.finalize();
    // 確定した記録で先頭を書き直す
    let out = out.inner;
    out.seek(SeekFrom::Start(0))?;
    out.write_all(&head)?;
    out.seek(SeekFrom::Start(pos))?;
    out.flush()?;
    let checksum = Checksum {
        head_len,
        head: crc32fast::hash(&head),
        tail,
        len: pos,
    };
    Ok((pos, checksum))
}

/// メモリに組んだ文書を流して書く（`write` と同じ文書の確かめをして、画素は文書から借りる）。ファイルの大きさの上限は `max_file_bytes`（`limits` の
/// 出力の上限を超える値は、その上限に切る）。
pub(super) fn stream_document<W: Write + Seek>(
    out: &mut W,
    d: &Document,
    limits: &Limits,
    compression: Compression,
    max_file_bytes: u64,
    cancel: Option<&AtomicBool>,
) -> XResult<Written> {
    let (records, _) = preflight(d, limits, compression)?;
    let opts = StreamOptions {
        compression,
        limits,
        max_file_bytes: max_file_bytes.min(limits.max_output_bytes as u64),
        cancel,
    };
    let (bytes, checksum) = stream(
        out,
        d.width,
        d.height,
        &records,
        &opts,
        |i| Ok(document_pixels(&records[i])),
        || {
            Ok(match &d.composite_rgba {
                Some(p) => Cow::Borrowed(p.as_slice()),
                None => Cow::Owned(super::composite::composite_cancellable(d, cancel)?),
            })
        },
    )?;
    Ok(Written {
        bytes,
        layers: records.iter().filter(|r| !r.divider).count(),
        checksum,
    })
}

fn document_pixels<'a>(r: &Record<'a>) -> Supplied<'a> {
    let l: &'a Layer = r.layer;
    let mask = if r.divider { None } else { l.mask.as_ref() };
    Supplied {
        raster: r.raster().then_some(Region {
            left: l.left,
            top: l.top,
            width: l.width,
            height: l.height,
            rgba: Cow::Borrowed(l.pixels_rgba.as_slice()),
        }),
        mask: mask.map(|m| MaskRegion {
            left: m.left,
            top: m.top,
            width: m.width,
            height: m.height,
            default_color: m.default_color,
            values: Cow::Borrowed(m.pixels.as_slice()),
        }),
    }
}

/// 文書を PSD のバイト列にする（無圧縮。Unity 版の書き手と同じバイト列）。
pub fn write(d: &Document, limits: &Limits) -> Result<Vec<u8>> {
    write_with(d, limits, Compression::Raw)
}

/// 文書を PSD のバイト列にする。`Compression::Raw` は Unity 版と同じバイト列、`Compression::Rle` は層・マスク・統合画像を RLE で書く。
pub fn write_with(d: &Document, limits: &Limits, compression: Compression) -> Result<Vec<u8>> {
    let (_, total) = preflight(d, limits, compression)?;
    // 無圧縮は長さが決まっている。RLE は圧縮したあとの長さが分からないので、無圧縮の長さを上限にせず、小さく始めて伸ばす
    let capacity = match compression {
        Compression::Raw => total,
        Compression::Rle => total.min(16 * 1024 * 1024),
    };
    let mut out = std::io::Cursor::new(Vec::with_capacity(capacity));
    stream_document(&mut out, d, limits, compression, limits.max_output_bytes as u64, None)?;
    let out = out.into_inner();
    // 無圧縮の長さは、書く前の確かめが数えた長さと一致する（書き手と確かめのずれを見つける）
    if compression == Compression::Raw {
        check(out.len() == total, "PSD 書き出し長と事前検証が不一致です")?;
    } else {
        check_budget(
            out.len() <= limits.max_output_bytes && out.len() <= i32::MAX as usize,
            "PSD 出力予算超過",
        )?;
    }
    Ok(out)
}
fn bounds(w: &mut Vec<u8>, x: i32, y: i32, width: u32, height: u32) {
    w.u32(y as u32);
    w.u32(x as u32);
    w.u32((i64::from(y) + i64::from(height)) as u32);
    w.u32((i64::from(x) + i64::from(width)) as u32)
}
/// 調整の記録（タグのキーと本体）。明るさ・コントラストだけは、旧式の `brit` と新しい式の `CgEd` の 2 つを書く。
fn adjustment_blocks(a: &Adjustment) -> Vec<([u8; 4], Vec<u8>)> {
    let mut blocks = vec![adjustment_block(a)];
    if let &Adjustment::BrightnessContrast {
        brightness,
        contrast,
    } = a
    {
        blocks.push((*b"CgEd", cged_block(brightness, contrast)))
    }
    blocks
}
/// 明るさ・コントラストの `CgEd`（版 16 の記述子: `Vrsn` 1・`Brgh`・`Cntr`・`means`・`Lab `・`useLegacy`・`auto`）。新しい式（旧式でない・自動でない）で書く。
fn cged_block(brightness: i16, contrast: i16) -> Vec<u8> {
    fn name(w: &mut Vec<u8>) {
        w.u32(1);
        w.u16(0)
    }
    fn key(w: &mut Vec<u8>, k: &[u8]) {
        // 4 文字の ID は長さ 0 と 4 文字、それ以外は長さと文字
        if k.len() == 4 {
            w.u32(0)
        } else {
            w.u32(k.len() as u32)
        }
        w.extend(k)
    }
    let mut w = Vec::new();
    w.u32(16);
    name(&mut w);
    key(&mut w, b"null");
    w.u32(7);
    for (k, v) in [
        (b"Vrsn".as_slice(), 1i32),
        (b"Brgh", i32::from(brightness)),
        (b"Cntr", i32::from(contrast)),
        (b"means", 127),
    ] {
        key(&mut w, k);
        w.extend(b"long");
        w.extend(v.to_be_bytes())
    }
    for k in [b"Lab ".as_slice(), b"useLegacy", b"auto"] {
        key(&mut w, k);
        w.extend(b"bool");
        w.push(0)
    }
    w
}
fn adjustment_block(a: &Adjustment) -> ([u8; 4], Vec<u8>) {
    let mut b = Vec::new();
    match a {
        Adjustment::Invert => (*b"nvrt", b),
        &Adjustment::Levels {
            input_black: ib,
            input_white: iw,
            output_black: ob,
            output_white: ow,
            gamma: g,
        } => {
            b.u16(2);
            for v in [ib, iw, ob, ow, g] {
                b.u16(v)
            }
            for _ in 1..29 {
                for v in [0, 255, 0, 255, 100] {
                    b.u16(v)
                }
            }
            (*b"levl", b)
        }
        &Adjustment::HueSaturation {
            hue,
            saturation,
            lightness,
        } => {
            b.u16(2);
            b.extend([0; 2]);
            for v in [0, 25, 0, hue, saturation, lightness] {
                b.u16(v as u16)
            }
            for range in super::read::HUE_RANGES {
                for v in range {
                    b.u16(v as u16)
                }
                b.extend([0; 6])
            }
            for i in 0..6 {
                for v in [i * 60, 100, 50] {
                    b.u16(v)
                }
            }
            (*b"hue2", b)
        }
        &Adjustment::Threshold { level } => {
            b.u16(level);
            b.extend([0; 2]);
            (*b"thrs", b)
        }
        &Adjustment::Posterize { levels } => {
            b.u16(levels);
            b.extend([0; 2]);
            (*b"post", b)
        }
        &Adjustment::BrightnessContrast {
            brightness,
            contrast,
        } => {
            // 平均値は 127（旧式の式の入力。この道具の式は使わない）、Lab だけの印は 0、余白 1 バイト
            for v in [brightness, contrast, 127] {
                b.u16(v as u16)
            }
            b.extend([0; 2]);
            (*b"brit", b)
        }
        Adjustment::ColorBalance {
            shadows,
            midtones,
            highlights,
            preserve_luminosity,
        } => {
            for v in shadows.iter().chain(midtones).chain(highlights) {
                b.u16(*v as u16)
            }
            b.extend([u8::from(*preserve_luminosity), 0]);
            (*b"blnc", b)
        }
        Adjustment::ToneCurve {
            composite,
            red,
            green,
            blue,
        } => {
            // 画素の表ではない・版 1・4 本すべて（RGB 全体・R・G・B のビット）。点は 出力・入力 の順。余白を 4 の倍数まで
            b.push(0);
            b.u16(1);
            b.u32(0xf);
            for curve in [composite, red, green, blue] {
                b.u16(curve.len() as u16);
                for [input, output] in curve {
                    b.u16(u16::from(*output));
                    b.u16(u16::from(*input))
                }
            }
            while b.len() % 4 != 0 {
                b.push(0)
            }
            (*b"curv", b)
        }
        Adjustment::GradientMap {
            reverse,
            colors,
            opacities,
        } => {
            b.u16(1);
            b.extend([u8::from(*reverse), 0]);
            b.u32(0); // 名前（空）
            b.u16(colors.len() as u16);
            for c in colors {
                b.u32(u32::from(c.location));
                b.u32(u32::from(c.midpoint));
                b.u16(0); // RGB
                for v in c.rgb {
                    b.u16(u16::from(v) * 257)
                }
                b.extend([0; 4]); // 4 つ目の成分と余白
            }
            b.u16(opacities.len() as u16);
            for o in opacities {
                b.u32(u32::from(o.location));
                b.u32(u32::from(o.midpoint));
                b.u16(u16::from(o.opacity))
            }
            // 拡張 2・なめらかさ 4096（100%）・長さ 32・モード 0（色の分岐点）、乱数・透明の表示（1）・ベクトルの色（0）・粗さ・色モデル（3 = RGB）・
            // 最小と最大の色・余白（ノイズ型の欄）
            for v in [2, 4096, 32, 0] {
                b.u16(v)
            }
            b.u32(0);
            b.u16(1);
            b.u16(0);
            b.u32(0);
            b.u16(3);
            b.extend([0; 16]);
            b.u16(0);
            while b.len() % 4 != 0 {
                b.push(0)
            }
            (*b"grdm", b)
        }
    }
}
fn solid_block(c: [u8; 3]) -> Vec<u8> {
    fn name(w: &mut Vec<u8>) {
        w.u32(1);
        w.u16(0)
    }
    fn key(w: &mut Vec<u8>, k: &[u8; 4]) {
        w.u32(0);
        w.extend(k)
    }
    let mut w = Vec::new();
    w.u32(16);
    name(&mut w);
    key(&mut w, b"null");
    w.u32(1);
    key(&mut w, b"Clr ");
    w.extend(b"Objc");
    name(&mut w);
    key(&mut w, b"RGBC");
    w.u32(3);
    for (k, v) in [b"Rd  ", b"Grn ", b"Bl  "].iter().zip(c) {
        key(&mut w, k);
        w.extend(b"doub");
        w.extend(f64::from(v).to_be_bytes())
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PackBits の復号（PSD の仕様どおり。-128 は何もしない）。
    fn unpack(data: &[u8], width: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < data.len() {
            let control = data[i] as i8;
            i += 1;
            match control {
                -128 => {}
                0..=127 => {
                    let n = control as usize + 1;
                    out.extend_from_slice(&data[i..i + n]);
                    i += n
                }
                _ => {
                    let n = 1 - i16::from(control);
                    out.extend(std::iter::repeat_n(data[i], n as usize));
                    i += 1
                }
            }
        }
        assert_eq!(out.len(), width, "行は幅ちょうどに戻る");
        out
    }

    fn round_trip(row: &[u8]) -> usize {
        let mut packed = Vec::new();
        packbits_row(row, &mut packed);
        assert_eq!(unpack(&packed, row.len()), row);
        packed.len()
    }

    #[test]
    fn packbits_rows_decode_back_for_runs_and_literals_at_every_boundary() {
        // 繰り返しの長さ（1・2・3 の境目と 128 の境目）× 前後の並び
        for run in [1usize, 2, 3, 4, 126, 127, 128, 129, 130, 255, 256, 257, 300] {
            for (before, after) in [(0, 0), (1, 0), (0, 1), (2, 3), (127, 128), (130, 5)] {
                let mut row: Vec<u8> = (0..before).map(|i| (i * 7 + 1) as u8 | 1).collect();
                row.extend(std::iter::repeat_n(0xaa, run));
                row.extend((0..after).map(|i| (i * 11 + 3) as u8 & 0x7e));
                round_trip(&row);
            }
        }
        // 長い並び（128 を超える）・交互・全部同じ・1 画素・空
        let mut seed = 9u32;
        let random: Vec<u8> = (0..1000)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 24) as u8
            })
            .collect();
        round_trip(&random);
        round_trip(&(0..1000).map(|i| (i % 2) as u8).collect::<Vec<_>>());
        round_trip(&[5; 1000]);
        round_trip(&[5]);
        round_trip(&[]);
        // 乱数は 128 画素ごとに 1 バイト増えるだけ
        assert!(round_trip(&random) <= random.len() + random.len() / 128 + 1);
        // 単色の行は 128 画素ごとに 2 バイト
        assert_eq!(round_trip(&[5; 1000]), 2 * 1000usize.div_ceil(128));
    }
}
