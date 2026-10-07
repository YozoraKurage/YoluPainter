//! 文字をレイヤーの面に塗る: 並べた字形の輪郭を基準の点のまわりに回して文書の座標に置き、文書の原点に揃えた 256 画素の升目ごとに
//! `vello_cpu` で塗る。升目は互いに独立で、並べて塗っても 1 つずつ塗っても同じバイトになる。
//!
//! 塗るのは升目の少しずつ（行の順。1 回に塗る升目の数は [`default_batch`]）で、塗り終えて、あとの升目が重ならなくなったタイルから
//! 面へ入れる。予算は入れるたびに見るので、予算を超えるなら、全部の升目・タイルを塗って持つ前に断る（持つのは少しの升目と、
//! まだ書き足されるタイルだけ）。

use std::collections::{BTreeMap, HashMap};

use rayon::prelude::*;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{GlyphId, MetadataProvider};
use vello_cpu::kurbo::{Affine, BezPath, Rect, Shape};
use vello_cpu::{
    color, Level, Pixmap, RasterizerSettings, RenderContext, RenderSettings, Resources,
};

use super::layout::{layout_face, Face};
use super::TextSettings;
use crate::error::CoreError;
use crate::surface::{Growth, Surface};
use crate::types::TileCoord;

/// 塗る升目の一辺（画素）。文書のタイルの大きさによらず同じにして、結果がタイルの大きさで変わらないようにする。
const CELL: u32 = 256;

/// 文字を塗った面（文書と同じ大きさ・タイル）。`budget` は面に許すバイト数（超えれば `SourceBudgetExceeded`）。
pub fn render(
    settings: &TextSettings,
    bytes: &[u8],
    index: u32,
    width: u32,
    height: u32,
    tile_size: u32,
    budget: u64,
) -> Result<Surface, CoreError> {
    let mut surface = Surface::new(width, height, tile_size);
    render_into(settings, bytes, index, &mut surface, budget)?;
    Ok(surface)
}

/// 空の面 `surface` に文字を塗る（面の大きさ・タイルは呼び手が決める）。
pub fn render_into(
    settings: &TextSettings,
    bytes: &[u8],
    index: u32,
    surface: &mut Surface,
    budget: u64,
) -> Result<(), CoreError> {
    let mut painted = 0;
    paint_batches(
        settings,
        bytes,
        index,
        surface,
        budget,
        default_batch(),
        &mut painted,
    )
}

/// 1 回に塗る升目の数（スレッドの数の数倍。多すぎると、予算を超えるときに無駄に塗り、持つ画素も増える）。
pub fn default_batch() -> usize {
    (rayon::current_num_threads() * 4).max(8)
}

/// `render` の、1 回に塗る升目の数 `batch` を決められる形と、塗った升目の数（試験用: 数を変えても同じ面になること、
/// 予算を超えるとき全部を塗る前に止まることを見る）。
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn render_batched(
    settings: &TextSettings,
    bytes: &[u8],
    index: u32,
    width: u32,
    height: u32,
    tile_size: u32,
    budget: u64,
    batch: usize,
) -> (Result<Surface, CoreError>, usize) {
    let mut surface = Surface::new(width, height, tile_size);
    let mut painted = 0;
    let result = paint_batches(
        settings,
        bytes,
        index,
        &mut surface,
        budget,
        batch.max(1),
        &mut painted,
    );
    (result.map(|()| surface), painted)
}

fn paint_batches(
    settings: &TextSettings,
    bytes: &[u8],
    index: u32,
    surface: &mut Surface,
    budget: u64,
    batch: usize,
    painted_cells: &mut usize,
) -> Result<(), CoreError> {
    settings.validate()?;
    if surface.tile_count() != 0 {
        return Err(CoreError::InvalidArgument("テキストを塗る面は空であること"));
    }
    let face = Face::new(bytes, index)?;
    let layout = layout_face(settings, &face);
    let (width, height) = (surface.width(), surface.height());

    // 字形の輪郭（字形ごとに 1 度だけ。大きさの画素、y は上向き）
    let outlines = face.font.outline_glyphs();
    let size = Size::new(settings.size as f32);
    let mut shapes: HashMap<u32, Option<BezPath>> = HashMap::new();
    let radians = settings.rotation.to_radians();
    let place = Affine::translate((settings.x, settings.y)) * Affine::rotate(radians);
    // 置いた字形（文書の座標の輪郭）と、その外接の箱
    let mut placed: Vec<(BezPath, Rect)> = Vec::new();
    for g in &layout.glyphs {
        let shape = shapes.entry(g.id).or_insert_with(|| {
            let glyph = outlines.get(GlyphId::new(g.id))?;
            let mut pen = Pen(BezPath::new());
            glyph
                .draw(
                    DrawSettings::unhinted(size, LocationRef::default()),
                    &mut pen,
                )
                .ok()?;
            (!pen.0.elements().is_empty()).then_some(pen.0)
        });
        let Some(shape) = shape else { continue };
        let path = (place * Affine::translate((g.x, g.y))) * shape.clone();
        let bounds = path.bounding_box();
        if !(bounds.x0.is_finite()
            && bounds.y0.is_finite()
            && bounds.x1.is_finite()
            && bounds.y1.is_finite())
        {
            continue;
        }
        placed.push((path, bounds));
    }

    // 升目ごとに、かかる字形を集める（升目は行の順、行の中は左から）
    let columns = width.div_ceil(CELL);
    let rows = height.div_ceil(CELL);
    let mut cells: BTreeMap<(u32, u32), Vec<usize>> = BTreeMap::new();
    for (i, (_, b)) in placed.iter().enumerate() {
        // アンチエイリアスの端の 1 画素を含める
        let x0 = ((b.x0 - 1.0).floor().max(0.0) / CELL as f64) as u32;
        let y0 = ((b.y0 - 1.0).floor().max(0.0) / CELL as f64) as u32;
        let x1 = (b.x1 + 1.0).ceil();
        let y1 = (b.y1 + 1.0).ceil();
        if x1 <= 0.0 || y1 <= 0.0 || b.x0 - 1.0 >= width as f64 || b.y0 - 1.0 >= height as f64 {
            continue;
        }
        let x1 = ((x1 / CELL as f64).ceil() as u32).min(columns);
        let y1 = ((y1 / CELL as f64).ceil() as u32).min(rows);
        for cy in y0..y1 {
            for cx in x0..x1 {
                cells.entry((cy, cx)).or_default().push(i);
            }
        }
    }
    let cells: Vec<((u32, u32), Vec<usize>)> = cells.into_iter().collect();

    // タイルごとに、重なる升目のうち最後のもの（升目の並びの番号）。その升目を塗り終えたタイルは、もう書き足されない
    let ts = surface.tile_size();
    let tile_bytes = surface.tile_bytes();
    let mut last_cell: HashMap<(u32, u32), usize> = HashMap::new();
    for (i, ((cy, cx), _)) in cells.iter().enumerate() {
        let (x0, y0) = (cx * CELL, cy * CELL);
        let w = CELL.min(width - x0);
        let h = CELL.min(height - y0);
        for ty in y0 / ts..=(y0 + h - 1) / ts {
            for tx in x0 / ts..=(x0 + w - 1) / ts {
                last_cell.insert((tx, ty), i);
            }
        }
    }

    // 少しずつ塗り、覆う量を文字の色にしてタイルへ。塗り終えたタイルから面へ入れる（予算は入れるたびに見る）
    let c = settings.color;
    let growth = Growth { budget, others: 0 };
    let mut tiles: BTreeMap<(u32, u32), Vec<u8>> = BTreeMap::new();
    let mut start = 0;
    while start < cells.len() {
        let end = (start + batch).min(cells.len());
        let coverages: Vec<Vec<u8>> = cells[start..end]
            .par_iter()
            .map(|((cy, cx), glyphs)| {
                paint_cell(
                    *cx,
                    *cy,
                    width,
                    height,
                    glyphs.iter().map(|&i| &placed[i].0),
                )
            })
            .collect();
        *painted_cells += end - start;
        for (((cy, cx), _), coverage) in cells[start..end].iter().zip(&coverages) {
            let (x0, y0) = (cx * CELL, cy * CELL);
            let w = CELL.min(width - x0);
            let h = CELL.min(height - y0);
            for j in 0..h {
                // 升目の画像は上の行から。文書の y は下から
                let y = y0 + h - 1 - j;
                for i in 0..w {
                    let cov = coverage[(j * w + i) as usize];
                    if cov == 0 {
                        continue;
                    }
                    let a = ((cov as u32 * c.a as u32 + 127) / 255) as u8;
                    if a == 0 {
                        continue;
                    }
                    let x = x0 + i;
                    let tile = tiles
                        .entry((x / ts, y / ts))
                        .or_insert_with(|| vec![0u8; tile_bytes]);
                    let p = (((y % ts) * ts + x % ts) * 4) as usize;
                    tile[p..p + 4].copy_from_slice(&[c.r, c.g, c.b, a]);
                }
            }
        }
        drop(coverages);
        let done: Vec<(u32, u32)> = tiles
            .keys()
            .filter(|key| last_cell[*key] < end)
            .copied()
            .collect();
        for key in done {
            let bytes = tiles.remove(&key).expect("あるタイル");
            surface.import_tile(TileCoord::new(key.0, key.1), &bytes, growth)?;
        }
        start = end;
    }
    Ok(())
}

/// 升目 1 つを塗り、覆う量（0〜255。上の行から、行は左から）を返す。
fn paint_cell<'a>(
    cx: u32,
    cy: u32,
    width: u32,
    height: u32,
    paths: impl Iterator<Item = &'a BezPath>,
) -> Vec<u8> {
    let (x0, y0) = (cx * CELL, cy * CELL);
    let w = CELL.min(width - x0) as u16;
    let h = CELL.min(height - y0) as u16;
    // CPU を調べず、既定の道に固定する（CPU で結果が変わらないように）。スレッドは使わない
    let settings = RenderSettings {
        level: Level::baseline(),
        num_threads: 0,
    };
    let mut ctx = RenderContext::new_with(w, h, settings);
    // 文書の座標（y は上向き）→ 升目の画像（左上が原点、y は下向き）
    ctx.set_transform(Affine::new([
        1.0,
        0.0,
        0.0,
        -1.0,
        -(x0 as f64),
        (y0 + h as u32) as f64,
    ]));
    ctx.set_paint(color::OpaqueColor::<color::Srgb>::WHITE);
    let mut all = BezPath::new();
    for p in paths {
        all.extend(p.iter());
    }
    ctx.fill_path(&all);
    ctx.flush();
    let mut pixmap = Pixmap::new(w, h);
    let mut resources = Resources::new();
    ctx.render_with(&mut pixmap, &mut resources, RasterizerSettings::default());
    pixmap
        .data_as_u8_slice()
        .chunks_exact(4)
        .map(|p| p[3])
        .collect()
}

/// 字形の輪郭を kurbo の曲線へ。
struct Pen(BezPath);

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((x as f64, y as f64));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((x as f64, y as f64));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0
            .quad_to((cx0 as f64, cy0 as f64), (x as f64, y as f64));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to(
            (cx0 as f64, cy0 as f64),
            (cx1 as f64, cy1 as f64),
            (x as f64, y as f64),
        );
    }
    fn close(&mut self) {
        self.0.close_path();
    }
}
