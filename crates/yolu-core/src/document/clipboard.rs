//! レイヤーの画素のコピー・カット・ペーストと、画像での置き換え（C# の `PaintDocument.LayerOps.cs` の `PixelClipboard`・`CopyPixels`・
//! `CopyMerged`・`CutPixels`・`PasteAsLayer` と、`PaintDocument.Regions.cs` の `ReplacePixels`）。
//!
//! - コピーは文書を変えない。選択範囲（無ければキャンバス全体）の中の、レイヤー自身の画素（フィルター・マスク・不透明度の前）か、マスク
//!   （隠す量を白で見える灰色に反転した不透明の画素）か、見えている合成を、値のある画素の外接矩形に切り詰めて写す。全部選ばれた
//!   画素は透明画素の RGB も含めてそのまま、部分的に選ばれた画素は透明へ向かってプリマルチプライドで混ぜる。
//! - カットは写してから、選択範囲の量だけ消す（1 回の Undo）。塗りつぶし・調整・グループは断り、画像・透明部分のロックで断る
//!   （マスクはすべてのロックだけ）。断る前にクリップボードは変えない。
//! - ペーストは新しいラスターレイヤー（指定チャンネルだけを持つ）として足し、選択を外す（1 回の Undo）。同じ大きさの文書へは元の位置、
//!   違う大きさへは中央で、はみ出す分は切って数を返す。
//! - 予算（バイト）を超えるなら何も変えずに断る。コピーは `max_bytes`（矩形の大きさ）、ペーストは一操作の予算（`stroke_budget`）。

use std::sync::Arc;

use rayon::prelude::*;

use super::{Command, Document, Entry, Target};
use crate::error::CoreError;
use crate::layer::{Layer, LayerId};
use crate::math::{to_byte, UNIT};
use crate::surface::{Surface, Tile};
use crate::types::{Channel, LayerKind, Rect, Rgba8, RowOrder, TileCoord};

/// 画素のコピー・カット・ペーストを断った理由（C# の `LayerOpRefusal` のうち、この操作のもの。パスレイヤーの切り取りは
/// 画素の書き込みの断り（`refuse_path_layer`）がそのまま返る）。数は断った理由の説明用で、画面には出さない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardRefusal {
    /// 画素を持たないレイヤー（グループ・調整）。
    NoPixels,
    /// 選択範囲の中（選択が無ければレイヤー全体）に、値のある画素が無い。`selection` は選択範囲があったか。
    NothingToCopy { selection: bool },
    /// 写す矩形が上限（`max_bytes`）を超える。`bytes` は超えた時点の矩形の大きさ。
    TooLarge { bytes: u64, limit: u64 },
    /// 貼るレイヤーの画素が一操作の予算を超える。
    OperationBudget { bytes: u64, limit: u64 },
    /// 塗りつぶしレイヤーは値から作るので切り取れない。
    NotPaintLayer,
}

impl std::fmt::Display for ClipboardRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClipboardRefusal::NoPixels => write!(f, "このレイヤーには画素が無い"),
            ClipboardRefusal::NothingToCopy { selection: true } => {
                write!(f, "選択範囲の中に画素が無い")
            }
            ClipboardRefusal::NothingToCopy { selection: false } => write!(f, "レイヤーが空"),
            ClipboardRefusal::TooLarge { .. } => write!(f, "コピーする範囲が大きすぎる"),
            ClipboardRefusal::OperationBudget { .. } => {
                write!(f, "貼るレイヤーが一回の操作の予算を超える")
            }
            ClipboardRefusal::NotPaintLayer => write!(f, "塗りつぶしレイヤーは切り取れない"),
        }
    }
}

/// 写した画素の出どころ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardSource {
    /// レイヤーのチャンネル。
    Layer,
    /// レイヤーのマスク（灰色で、白が見える）。
    Mask,
    /// 見えている合成。
    Composite,
    /// 外の画像（OS のクリップボードなど）。その画像の大きさの文書から写したものとして扱う。
    External,
}

/// レイヤー・マスク・合成から写した画素: straight RGBA8 の長方形（左下原点・下の行が先）と、写したときの文書の大きさ・位置。
/// 作ったあとは変わらない（中身は共有でき、写しは安い）。マスクは灰色（白 = 見える。保存している隠す量の反転）で不透明。
#[derive(Clone, PartialEq, Eq)]
pub struct PixelClipboard {
    pixels: Arc<Vec<u8>>,
    document_width: u32,
    document_height: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    source: ClipboardSource,
    channel: Channel,
}

/// 画素そのものは出さない（256 MiB 級の写しを、断言の失敗やログへ全部書かない）。
impl std::fmt::Debug for PixelClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PixelClipboard")
            .field("document", &(self.document_width, self.document_height))
            .field("rect", &self.rect())
            .field("source", &self.source)
            .field("channel", &self.channel)
            .field("bytes", &self.pixels.len())
            .finish()
    }
}

impl PixelClipboard {
    /// 写した長方形（文書 `document_width × document_height` の中の (x, y) から width × height）の画素 rgba（幅 × 高さ × 4）。
    /// 空の長方形・文書の外にはみ出す長方形・バイト数が合わない画素は断る。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        document_width: u32,
        document_height: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        source: ClipboardSource,
        channel: Channel,
    ) -> Result<PixelClipboard, CoreError> {
        if document_width == 0 || document_height == 0 {
            return Err(CoreError::InvalidArgument("写したキャンバスの大きさ"));
        }
        if width == 0 || height == 0 {
            return Err(CoreError::InvalidArgument("写した矩形が空"));
        }
        if x as u64 + width as u64 > document_width as u64
            || y as u64 + height as u64 > document_height as u64
        {
            return Err(CoreError::InvalidArgument("写した矩形がキャンバスの外"));
        }
        if rgba.len() as u64 != width as u64 * height as u64 * 4 {
            return Err(CoreError::InvalidArgument(
                "写した画素のバイト数が幅 × 高さ × 4 でない",
            ));
        }
        Ok(PixelClipboard {
            pixels: Arc::new(rgba),
            document_width,
            document_height,
            x,
            y,
            width,
            height,
            source,
            channel,
        })
    }

    /// 外の画像（width × height の straight RGBA8、下の行が先）。その画像の大きさの文書の全体から写したものとして扱うので、
    /// 貼ると、同じ大きさの文書へは (0, 0)、違う大きさの文書へは中央になる。
    pub fn from_image(
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        channel: Channel,
    ) -> Result<PixelClipboard, CoreError> {
        PixelClipboard::new(
            width,
            height,
            0,
            0,
            width,
            height,
            rgba,
            ClipboardSource::External,
            channel,
        )
    }

    /// 写したときの文書の大きさ。
    pub fn document_size(&self) -> (u32, u32) {
        (self.document_width, self.document_height)
    }
    /// 写した長方形（元の文書の中の位置と大きさ。左下原点）。
    pub fn rect(&self) -> Rect {
        Rect::new(self.x, self.y, self.width, self.height)
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn source(&self) -> ClipboardSource {
        self.source
    }
    /// 写したチャンネル（マスクは Color）。
    pub fn channel(&self) -> Channel {
        self.channel
    }
    /// 画素（幅 × 高さ × 4 バイト、下の行から）。
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
    /// 長方形の中の (x, y) の画素。
    pub fn pixel(&self, x: u32, y: u32) -> Result<Rgba8, CoreError> {
        if x >= self.width || y >= self.height {
            return Err(CoreError::InvalidArgument("画素が矩形の外"));
        }
        let o = (y as usize * self.width as usize + x as usize) * 4;
        Ok(Rgba8::from_slice(&self.pixels[o..o + 4]))
    }
    /// 画素のバイト数。
    pub fn byte_size(&self) -> u64 {
        self.pixels.len() as u64
    }
}

/// `Document::paste_as_layer` の結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PasteResult {
    /// 足したレイヤー。
    pub layer: LayerId,
    /// 写しの左下の角が行った位置（はみ出して切ったときはキャンバスの外になり得る）。
    pub x: i32,
    pub y: i32,
    /// 文書の大きさが写したときと違うので、元の位置ではなく中央へ置いた。
    pub centered: bool,
    /// キャンバスの外へはみ出して切り捨てた、値のある画素の数。
    pub clipped_pixels: u64,
}

/// 一度に読むタイルの上限（バイト）。一操作の予算の 1/4 を超えない（最低 1 枚）。
const CHUNK_BYTES: u64 = 16 * 1024 * 1024;

/// プリマルチプライドの補間: a から b へ t（0〜1）。ストレートで返す（透明になる結果は透明の黒）。C# の `Interpolate`。
pub(crate) fn interpolate(a: Rgba8, b: Rgba8, t: f64) -> Rgba8 {
    let aa = UNIT[a.a as usize];
    let ba = UNIT[b.a as usize];
    let alpha = aa + (ba - aa) * t;
    if alpha <= 0.0 {
        return Rgba8::TRANSPARENT;
    }
    let channel = |ac: u8, bc: u8| {
        (UNIT[ac as usize] * aa + (UNIT[bc as usize] * ba - UNIT[ac as usize] * aa) * t) / alpha
    };
    Rgba8::new(
        to_byte(channel(a.r, b.r)),
        to_byte(channel(a.g, b.g)),
        to_byte(channel(a.b, b.b)),
        to_byte(alpha),
    )
}

/// 透明部分のロックで、画像で置き換えるときの 1 画素（C# の `ReplaceKeepingAlpha`）: 置き換えの色を量だけ、画素自身のアルファで。
/// 透明な画素・透明な置き換え（色が無い）はそのまま。
fn replace_keeping_alpha(start: Rgba8, image: Rgba8, amount: f64) -> Rgba8 {
    if image.a == 0 {
        start
    } else {
        super::locks::paint_keeping_alpha(start, Rgba8::new(image.r, image.g, image.b, 255), amount)
    }
}

impl Document {
    // ───────── コピー ─────────

    /// レイヤーのチャンネル（from_mask ならマスク）の画素を、選択範囲の中（無ければ全体）だけ [`PixelClipboard`] へ写す。文書は変えない。
    /// 全部選ばれた所は透明画素の RGB も含めてそのまま、部分的に選ばれた所は透明へ向かってプリマルチプライドで混ぜる
    /// （[`Document::replace_pixels`] の式）。レイヤー自身の画素を写すので、フィルター・マスク・不透明度は掛けない（見えているものは
    /// [`Document::copy_merged`]）。塗りつぶしレイヤーは値を写す。矩形は値のある画素まで切り詰め、無ければ、`max_bytes` を超えれば断る。
    /// チャンネルが無効でも、持っている画素は写せる。
    pub fn copy_pixels(
        &self,
        layer: LayerId,
        channel: Channel,
        from_mask: bool,
        max_bytes: u64,
    ) -> Result<PixelClipboard, CoreError> {
        self.ensure_no_stroke()?;
        self.require_channel(channel)?;
        let index = self.index_of(layer)?;
        let l = &self.layers[index];
        let tile_bytes = self.tile_size as usize * self.tile_size as usize * 4;
        if from_mask {
            let mask = l
                .mask
                .as_ref()
                .ok_or(CoreError::Unsupported("レイヤーにマスクが無い"))?;
            return self.copy_region(ClipboardSource::Mask, Channel::Color, max_bytes, |chunk| {
                chunk
                    .iter()
                    .map(|&coord| {
                        let mut tile = vec![0u8; tile_bytes];
                        mask.surface.copy_tile(coord, &mut tile)?;
                        for p in tile.chunks_exact_mut(4) {
                            // 保存しているのは隠す量。コピーは白 = 見えるの灰色（Photoshop のマスクのコピーと同じ）
                            let grey = 255 - p[3];
                            p.copy_from_slice(&[grey, grey, grey, 255]);
                        }
                        Ok(tile)
                    })
                    .collect()
            });
        }
        if matches!(l.kind, LayerKind::Group | LayerKind::Adjustment) {
            return Err(CoreError::Clipboard(ClipboardRefusal::NoPixels));
        }
        self.copy_region(ClipboardSource::Layer, channel, max_bytes, |chunk| {
            chunk
                .iter()
                .map(|&coord| {
                    let mut tile = vec![0u8; tile_bytes];
                    self.layer_tile(l, channel, coord, &mut tile)?;
                    Ok(tile)
                })
                .collect()
        })
    }

    /// チャンネルの合成（見えている全部のレイヤー。Normal は描いたベクトルの合成で、Unity へ出す出力ではない）を、選択範囲の中だけ
    /// [`Document::copy_pixels`] と同じ規則で写す。
    pub fn copy_merged(
        &self,
        channel: Channel,
        max_bytes: u64,
    ) -> Result<PixelClipboard, CoreError> {
        self.ensure_no_stroke()?;
        self.require_channel(channel)?;
        let tile_size = self.tile_size as usize;
        self.copy_region(ClipboardSource::Composite, channel, max_bytes, |chunk| {
            chunk
                .par_iter()
                .map(|&coord| {
                    let rect = self
                        .tile_rect(coord)
                        .ok_or(CoreError::InvalidArgument("タイルがキャンバスの外"))?;
                    let mut rows = vec![0u8; rect.width as usize * rect.height as usize * 4];
                    self.composite_into(channel, rect, &mut rows, RowOrder::BottomUp)?;
                    // タイルの大きさの領域へ（キャンバスの外の余白は 0）
                    let mut tile = vec![0u8; tile_size * tile_size * 4];
                    let row_bytes = rect.width as usize * 4;
                    for (y, row) in rows.chunks_exact(row_bytes).enumerate() {
                        tile[y * tile_size * 4..][..row_bytes].copy_from_slice(row);
                    }
                    Ok(tile)
                })
                .collect()
        })
    }

    /// レイヤーの画素の 1 タイル（余白は 0）。塗りつぶしレイヤーは値をキャンバスの中へ敷く。持っていないチャンネルは 0。
    fn layer_tile(
        &self,
        layer: &Layer,
        channel: Channel,
        coord: TileCoord,
        out: &mut [u8],
    ) -> Result<(), CoreError> {
        match layer.kind {
            LayerKind::Raster => {
                match layer.surface(channel) {
                    Some(surface) => {
                        surface.copy_tile(coord, out)?;
                    }
                    None => out.fill(0),
                }
                Ok(())
            }
            LayerKind::Fill => {
                out.fill(0);
                let Some(value) = layer.fill_value(channel) else {
                    return Ok(());
                };
                let rect = self
                    .tile_rect(coord)
                    .ok_or(CoreError::InvalidArgument("タイルがキャンバスの外"))?;
                let ts = self.tile_size as usize;
                for y in 0..rect.height as usize {
                    for x in 0..rect.width as usize {
                        out[(y * ts + x) * 4..][..4].copy_from_slice(&value.to_array());
                    }
                }
                Ok(())
            }
            _ => {
                out.fill(0);
                Ok(())
            }
        }
    }

    /// 選択範囲（無ければキャンバス）のタイルを、1 回に一操作の予算の 1/4（最大 16 MiB）ぶんずつ読み、選択の量を当て、値のある画素の外接矩形へ
    /// 切り詰める。タイルは 1 回だけ読み、値のあるタイルだけを矩形に写し終わるまで持つ。矩形が `max_bytes` を超えた時点で断る。
    fn copy_region(
        &self,
        source: ClipboardSource,
        channel: Channel,
        max_bytes: u64,
        read: impl Fn(&[TileCoord]) -> Result<Vec<Vec<u8>>, CoreError>,
    ) -> Result<PixelClipboard, CoreError> {
        let ts = self.tile_size as usize;
        let n = ts * ts;
        let selection = self.selection.as_ref();
        let coords: Vec<TileCoord> = match selection {
            Some(m) => m.tile_coords(),
            None => self.canvas_tiles().collect(),
        };
        let chunk_len =
            ((CHUNK_BYTES.min(self.stroke_budget / 4)) / (n as u64 * 4)).max(1) as usize;
        let mut amounts = vec![0u8; n];
        let mut kept: Vec<(TileCoord, Vec<u8>)> = Vec::new();
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        let mut any_value = false;
        for chunk in coords.chunks(chunk_len) {
            let tiles = read(chunk)?;
            for (&coord, mut bytes) in chunk.iter().zip(tiles) {
                match selection {
                    Some(m) => {
                        if !m.copy_tile(coord, &mut amounts)? {
                            continue;
                        }
                    }
                    None => amounts.fill(255),
                }
                let w = (self.width as usize - coord.x as usize * ts).min(ts);
                let h = (self.height as usize - coord.y as usize * ts).min(ts);
                let mut any = false;
                for y in 0..ts {
                    for x in 0..ts {
                        let i = y * ts + x;
                        let o = i * 4;
                        let amount = if x < w && y < h { amounts[i] } else { 0 };
                        if amount == 0 {
                            bytes[o..o + 4].fill(0);
                            continue;
                        }
                        if amount < 255 {
                            let p = interpolate(
                                Rgba8::TRANSPARENT,
                                Rgba8::from_slice(&bytes[o..o + 4]),
                                amount as f64 / 255.0,
                            );
                            bytes[o..o + 4].copy_from_slice(&p.to_array());
                        }
                        if bytes[o..o + 4] == [0, 0, 0, 0] {
                            continue;
                        }
                        any = true;
                        let (px, py) = (coord.x as usize * ts + x, coord.y as usize * ts + y);
                        if !any_value {
                            (x0, y0, x1, y1) = (px, py, px, py);
                            any_value = true;
                        } else {
                            x0 = x0.min(px);
                            y0 = y0.min(py);
                            x1 = x1.max(px);
                            y1 = y1.max(py);
                        }
                    }
                }
                if !any {
                    continue;
                }
                kept.push((coord, bytes));
                let so_far = (x1 - x0 + 1) as u64 * (y1 - y0 + 1) as u64 * 4;
                if so_far > max_bytes {
                    return Err(CoreError::Clipboard(ClipboardRefusal::TooLarge {
                        bytes: so_far,
                        limit: max_bytes,
                    }));
                }
            }
        }
        if !any_value {
            return Err(CoreError::Clipboard(ClipboardRefusal::NothingToCopy {
                selection: selection.is_some(),
            }));
        }
        let (width, height) = (x1 - x0 + 1, y1 - y0 + 1);
        let mut rgba = vec![0u8; width * height * 4];
        for (coord, tile) in &kept {
            let (bx, by) = (coord.x as usize * ts, coord.y as usize * ts);
            let (cx0, cx1) = (bx.max(x0), (bx + ts - 1).min(x1));
            let (cy0, cy1) = (by.max(y0), (by + ts - 1).min(y1));
            if cx0 > cx1 || cy0 > cy1 {
                continue;
            }
            for y in cy0..=cy1 {
                let src = ((y - by) * ts + (cx0 - bx)) * 4;
                let dst = ((y - y0) * width + (cx0 - x0)) * 4;
                let len = (cx1 - cx0 + 1) * 4;
                rgba[dst..dst + len].copy_from_slice(&tile[src..src + len]);
            }
        }
        PixelClipboard::new(
            self.width,
            self.height,
            x0 as u32,
            y0 as u32,
            width as u32,
            height as u32,
            rgba,
            source,
            channel,
        )
    }

    // ───────── カット ─────────

    /// [`Document::copy_pixels`] で写してから、写した所を消す（1 回の Undo）。ラスターレイヤーのチャンネルは選択範囲の量だけアルファが
    /// 減り（全部選ばれた画素は透明の黒）、マスクは見える方へ戻る。塗りつぶし・調整・グループのレイヤーは断り（何も写さない）、
    /// 画像のロックと透明部分のロック（消すので）で断る。マスクはすべてのロックだけで断る。写すときの上限も、消すときの
    /// 予算も、断ったら何も変えない（返すクリップボードも無い）。
    pub fn cut_pixels(
        &mut self,
        layer: LayerId,
        channel: Channel,
        from_mask: bool,
        max_bytes: u64,
    ) -> Result<PixelClipboard, CoreError> {
        self.ensure_no_stroke()?;
        self.require_channel(channel)?;
        let index = self.index_of(layer)?;
        if !from_mask {
            match self.layers[index].kind {
                LayerKind::Raster => {}
                LayerKind::Fill => {
                    return Err(CoreError::Clipboard(ClipboardRefusal::NotPaintLayer))
                }
                _ => return Err(CoreError::Clipboard(ClipboardRefusal::NoPixels)),
            }
            if !self.layers[index].is_channel_enabled(channel) {
                return Err(CoreError::Unsupported("無効のチャンネルは切り取れない"));
            }
        }
        // 写す前に断る（クリップボードも変えない）
        if from_mask {
            self.refuse_lock(layer, super::LayerLocks::ALL)?;
        } else {
            self.pixel_write_guard(layer, true)?;
            // パスレイヤーの画素はパスが決めるので、手では切り取らない（C# の CutPixels も断る。断りの順はロックのあと）
            let index = self.index_of(layer)?;
            self.refuse_path_layer(index)?;
        }
        let copied = self.copy_pixels(layer, channel, from_mask, max_bytes)?;
        if from_mask {
            self.fill_mask(layer, 1.0, None, true)?;
        } else {
            self.fill(layer, channel, Rgba8::new(0, 0, 0, 255), 1.0, None, true)?;
        }
        Ok(copied)
    }

    // ───────── ペースト ─────────

    /// クリップボードの画素を、新しいラスターレイヤー（channel だけを持つ）として above のすぐ上（同じグループの中。無ければ一番上）へ
    /// 足し、選択を外す（1 回の Undo。Photoshop の貼り付けと同じで、そのあと移動で動かせる）。文書が写したときと同じ大きさなら
    /// 元の位置へ、違えば中央へ置き、キャンバスの外へはみ出す分は切って [`PasteResult::clipped_pixels`] に数える。透明画素の RGB も
    /// そのまま書く。新しいレイヤーの画素が一操作の予算（`stroke_budget`）か画素の予算を超えるなら、何も変えずに断る。
    pub fn paste_as_layer(
        &mut self,
        clip: &PixelClipboard,
        channel: Channel,
        name: Option<&str>,
        above: Option<LayerId>,
    ) -> Result<PasteResult, CoreError> {
        self.ensure_no_stroke()?;
        self.require_channel(channel)?;
        let (index, parent) = match above {
            Some(a) => {
                let i = self.index_of(a)?;
                (i + 1, self.layers[i].parent)
            }
            None => (self.layers.len(), None),
        };
        let (w, h) = (self.width as i64, self.height as i64);
        let (cw, ch) = (clip.width as i64, clip.height as i64);
        let centered = (clip.document_width, clip.document_height) != (self.width, self.height);
        let (ox, oy) = if centered {
            ((w - cw) / 2, (h - ch) / 2)
        } else {
            (clip.x as i64, clip.y as i64)
        };
        let ts = self.tile_size as i64;
        let tile_len = (ts * ts * 4) as usize;
        let src = clip.pixels();
        let (x0, y0) = (ox.max(0), oy.max(0));
        let (x1, y1) = ((ox + cw).min(w), (oy + ch).min(h));
        let mut clipped = 0u64;
        if x0 != ox || y0 != oy || x1 != ox + cw || y1 != oy + ch {
            for y in 0..ch {
                for x in 0..cw {
                    let (px, py) = (ox + x, oy + y);
                    if px >= x0 && px < x1 && py >= y0 && py < y1 {
                        continue;
                    }
                    let o = ((y * cw + x) * 4) as usize;
                    if src[o..o + 4] != [0, 0, 0, 0] {
                        clipped += 1;
                    }
                }
            }
        }
        // 行ごとにタイルの幅の区間をまとめて写す（全部 0 のタイルは後で捨てる）
        let mut tiles: std::collections::BTreeMap<TileCoord, Vec<u8>> =
            std::collections::BTreeMap::new();
        // 矩形がキャンバスと重ならないなら、写す画素は無い（x0 < x1・y0 < y1 は、写しが文書の中の矩形か中央に置く限り常に成り立つ）
        for py in y0..if x0 < x1 { y1 } else { y0 } {
            let mut tx = x0 / ts;
            while tx * ts < x1 {
                let (sx0, sx1) = (x0.max(tx * ts), x1.min((tx + 1) * ts));
                let coord = TileCoord::new(tx as u32, (py / ts) as u32);
                let tile = tiles.entry(coord).or_insert_with(|| vec![0u8; tile_len]);
                let from = (((py - oy) * cw + sx0 - ox) * 4) as usize;
                let to = (((py % ts) * ts + sx0 - tx * ts) * 4) as usize;
                let len = ((sx1 - sx0) * 4) as usize;
                tile[to..to + len].copy_from_slice(&src[from..from + len]);
                tx += 1;
            }
        }
        let mut surface = Surface::new(self.width, self.height, self.tile_size);
        let mut bytes = 0u64;
        for (coord, data) in &tiles {
            let Some(tile) = Tile::from_bytes(data) else {
                continue;
            };
            bytes += 64 + tile.byte_size();
            if bytes > self.stroke_budget {
                return Err(CoreError::Clipboard(ClipboardRefusal::OperationBudget {
                    bytes,
                    limit: self.stroke_budget,
                }));
            }
            surface.restore(*coord, Some(&tile));
        }
        let id = self.new_layer_id();
        let mut layer = Layer::new(id, name.unwrap_or("Layer"), LayerKind::Raster);
        let layer_bytes = surface.allocated_bytes();
        layer.put_surface(channel, Some(surface));
        layer.set_enabled(channel, true);
        layer.parent = parent;
        let mut steps = vec![Entry {
            kind: super::HistoryKind::AddLayer,
            command: Command::Insert {
                index,
                len: 1,
                block: Some(vec![layer]),
            },
            cost: 128 + layer_bytes,
        }];
        if self.selection.is_some() {
            steps.push(Entry {
                kind: super::HistoryKind::Selection,
                command: Command::Selection {
                    old: self.selection.clone(),
                    new: None,
                },
                cost: 64,
            });
        }
        self.execute_group(steps)?;
        Ok(PasteResult {
            layer: id,
            x: ox as i32,
            y: oy as i32,
            centered,
            clipped_pixels: clipped,
        })
    }

    // ───────── 置き換え ─────────

    /// レイヤーのチャンネルを画像（文書と同じ大きさの straight RGBA8、左下原点・下の行が先）で置き換える（1 回の Undo）。選択範囲の中
    /// （`within_selection` が false なら選択に関わらず全体）で、全部選ばれた画素は画像の画素をそのまま（透明画素の RGB も）、
    /// 部分的に選ばれた画素は元と画像をプリマルチプライドで補間する。画素が 1 つも変わらなければ false（段は積まない）。
    /// 画像のロックで断り、透明部分のロックでは色だけを置き換える（各画素のアルファは保ち、透明な画素はそのまま）。
    /// 画像の大きさが違う・ラスターでない・チャンネルが無効なら断る。
    pub fn replace_pixels(
        &mut self,
        layer: LayerId,
        channel: Channel,
        rgba: &[u8],
        within_selection: bool,
    ) -> Result<bool, CoreError> {
        if rgba.len() as u64 != self.width as u64 * self.height as u64 * 4 {
            return Err(CoreError::InvalidArgument("画像の大きさがキャンバスと違う"));
        }
        self.ensure_no_stroke()?;
        self.require_channel(channel)?;
        let index = self.index_of(layer)?;
        if self.layers[index].kind != LayerKind::Raster {
            return Err(CoreError::Unsupported(
                "画素を置き換えられるのはラスターレイヤーだけ",
            ));
        }
        if !self.layers[index].is_channel_enabled(channel) {
            return Err(CoreError::Unsupported("無効のチャンネルは置き換えられない"));
        }
        let keep_alpha = self.pixel_write_guard(layer, false)?;
        let created = self.ensure_surface(index, channel);
        let effective = if within_selection {
            self.selection.clone()
        } else {
            None
        };
        let width = self.width as usize;
        let result = self.edit_effective(
            index,
            Target::Channel(channel),
            effective,
            |x, y, start, amount| {
                let o = (y as usize * width + x as usize) * 4;
                let image = Rgba8::from_slice(&rgba[o..o + 4]);
                if keep_alpha {
                    replace_keeping_alpha(start, image, amount)
                } else if amount >= 1.0 {
                    image
                } else {
                    interpolate(start, image, amount)
                }
            },
        );
        if !matches!(result, Ok(true)) {
            self.drop_created_surface(index, channel, created);
        }
        result
    }
}
