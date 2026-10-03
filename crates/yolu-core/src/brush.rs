//! 丸いブラシのストローク（C# の BrushStroke の、M1 で使う部分: 丸い筆先・硬さ・間隔・流量・不透明度・筆圧・消しゴム）。
//!
//! - 入力の点を結んだ線の上に、直径 × 間隔ごとの弧長でダブを置く（入力の区切り方によらない）。筆圧は各ダブの位置で線形に補間する。
//!   同じ位置の点はダブを増やさない。確定のときに終点へ余分なダブを置かない。
//! - 塗りはストロークの中で画素ごとに溜まる（Photoshop・CLIP STUDIO と同じ）: 各ダブは画素の「ストロークの覆い」を、
//!   ダブの天井（不透明度 × 筆圧）へ流量（流量 × 覆い × 筆圧）の割合だけ寄せ、画素はストロークの前の色から毎回計算し直す。
//!   重なったダブがストロークの不透明度を超えることはない。
//! - 覆いは C# と同じく float（単精度）で持つ。計算は倍精度で、タイルごとの探索は 1 回だけ（画素の結果は C# とバイト一致）。

use std::collections::HashMap;

use glam::DVec2;
use rayon::prelude::*;

use crate::blend::blend;
use crate::error::CoreError;
use crate::math::{clamp01, require_finite, to_byte};
use crate::surface::{Growth, LiveTile, Surface, Tile};
use crate::types::{BlendMode, Channel, Rgba8, TileCoord};
use crate::LayerId;

/// ブラシの設定。ストロークを始めたときに写して固定する（途中で変えても、そのストロークには効かない）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrushSettings {
    /// 半径（画素）。0 < r ≤ 65536。
    pub radius: f64,
    /// 硬さ 0〜1。覆いが 1 のまま続く半径の割合（その外は smoothstep で 0 へ）。
    pub hardness: f64,
    /// ダブの間隔（公称の直径に対する割合、0.01〜4）。筆圧や入力の間隔によらない。
    pub spacing: f64,
    /// ストロークの不透明度（重なっても超えない天井）0〜1。
    pub opacity: f64,
    /// 流量（1 つのダブが天井へ寄せる割合）0〜1。
    pub flow: f64,
    /// 塗る色（straight）。消しゴムではアルファだけを消す強さに使う。
    pub color: Rgba8,
    /// 筆圧で大きさ・不透明度・流量を変えるか（それぞれ線形）。
    pub pressure_size: bool,
    pub pressure_opacity: bool,
    pub pressure_flow: bool,
    /// 消しゴム（アルファを消す。色のアルファの割合だけ）。
    pub erase: bool,
}

impl Default for BrushSettings {
    /// C# の BrushSettings の既定値と同じ。
    fn default() -> Self {
        BrushSettings {
            radius: 16.0,
            hardness: 0.8,
            spacing: 0.15,
            opacity: 1.0,
            flow: 1.0,
            color: Rgba8::new(255, 255, 255, 255),
            pressure_size: true,
            pressure_opacity: true,
            pressure_flow: false,
            erase: false,
        }
    }
}

impl BrushSettings {
    /// C# の BrushSettings.Validate の M1 の部分。
    pub fn validate(&self) -> Result<(), CoreError> {
        require_finite(self.radius, "radius")?;
        require_finite(self.hardness, "hardness")?;
        require_finite(self.spacing, "spacing")?;
        require_finite(self.opacity, "opacity")?;
        require_finite(self.flow, "flow")?;
        if self.radius <= 0.0 || self.radius > 65536.0 {
            return Err(CoreError::InvalidArgument("radius"));
        }
        if !(0.0..=1.0).contains(&self.hardness) {
            return Err(CoreError::InvalidArgument("hardness"));
        }
        if !(0.01..=4.0).contains(&self.spacing) {
            return Err(CoreError::InvalidArgument("spacing"));
        }
        if !(0.0..=1.0).contains(&self.opacity) {
            return Err(CoreError::InvalidArgument("opacity"));
        }
        if !(0.0..=1.0).contains(&self.flow) {
            return Err(CoreError::InvalidArgument("flow"));
        }
        Ok(())
    }
}

/// ペンの入力 1 つ。画素の座標（左下原点、画素の中心は n + 0.5）。時刻は減ってはならない。
/// 傾きはペンの直立からの角度（ラジアン、画布の X・Y 軸に沿って。0 は直立か傾きの情報なし）。M1 では記録するだけで使わない。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrushSample {
    pub x: f64,
    pub y: f64,
    pub pressure: f64,
    pub time: f64,
    pub tilt: DVec2,
}

impl BrushSample {
    /// 有限でない値は断る。筆圧は 0〜1、傾きは ±π/2 に収める（C# の BrushSample と同じ）。
    pub fn new(x: f64, y: f64, pressure: f64, time: f64, tilt: DVec2) -> Result<Self, CoreError> {
        require_finite(x, "x")?;
        require_finite(y, "y")?;
        require_finite(pressure, "pressure")?;
        require_finite(time, "time")?;
        require_finite(tilt.x, "tilt")?;
        require_finite(tilt.y, "tilt")?;
        let max = std::f64::consts::FRAC_PI_2;
        Ok(BrushSample {
            x,
            y,
            pressure: clamp01(pressure),
            time,
            tilt: DVec2::new(tilt.x.clamp(-max, max), tilt.y.clamp(-max, max)),
        })
    }
}

/// ストロークが手を付けたタイル 1 枚: 画素ごとのストロークの覆い（0〜1）と、ストロークの前のタイル（巻き戻し用の写し）。
pub(crate) struct StrokeTile {
    wash: Vec<f32>,
    pub before: Option<Tile>,
}

/// 予算（文書の設定をストロークの始めに写す。ストロークの間は、ほかの面は変わらない）。
#[derive(Clone, Copy)]
pub(crate) struct Budgets {
    pub growth: Growth,
    pub stroke: u64,
}

/// 進行中のストロークの中身（文書が持つ）。
pub(crate) struct StrokeState {
    pub id: u64,
    pub layer: LayerId,
    pub layer_index: usize,
    pub channel: Channel,
    settings: BrushSettings,
    budgets: Budgets,
    has_sample: bool,
    previous: BrushSample,
    distance_since_stamp: f64,
    stroke_length: f64,
    has_pen: bool,
    last_input: BrushSample,
    pub stamp_count: u64,
    pub sample_count: u64,
    /// 安全なタイルをワーカーで描いたダブの数。
    pub parallel_dabs: u64,
    pub tiles: HashMap<TileCoord, StrokeTile>,
    /// 巻き戻しの写しと覆いのバイト数（C# の RollbackBytes の M1 の部分）。
    pub rollback_bytes: u64,
}

/// これを超える長さの区間（ダブの数）は断る（C# と同じ百万）。
const MAX_STAMPS_PER_SEGMENT: f64 = 1_000_000.0;
/// 入力の座標の範囲（C# と同じ）。
const MAX_COORDINATE: f64 = 10_000_000.0;

impl StrokeState {
    pub(crate) fn new(
        id: u64,
        layer: LayerId,
        layer_index: usize,
        channel: Channel,
        settings: BrushSettings,
        budgets: Budgets,
    ) -> Self {
        let zero = BrushSample {
            x: 0.0,
            y: 0.0,
            pressure: 0.0,
            time: 0.0,
            tilt: DVec2::ZERO,
        };
        StrokeState {
            id,
            layer,
            layer_index,
            channel,
            settings,
            budgets,
            has_sample: false,
            previous: zero,
            distance_since_stamp: 0.0,
            stroke_length: 0.0,
            has_pen: false,
            last_input: zero,
            stamp_count: 0,
            sample_count: 0,
            parallel_dabs: 0,
            tiles: HashMap::new(),
            rollback_bytes: 0,
        }
    }

    pub(crate) fn last_time(&self) -> f64 {
        if self.has_pen {
            self.last_input.time
        } else {
            0.0
        }
    }

    /// 入力の点を 1 つ足す（C# の Add。手ぶれ補正なし）。changed に、画素の変わったタイルを足す。画素が変わったら true。
    pub(crate) fn add(
        &mut self,
        surface: &mut Surface,
        sample: BrushSample,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        if self.has_pen && sample.time < self.last_input.time {
            return Err(CoreError::InvalidArgument("入力の時刻が戻った"));
        }
        if sample.x.abs() > MAX_COORDINATE || sample.y.abs() > MAX_COORDINATE {
            return Err(CoreError::InvalidArgument("入力の座標が範囲外"));
        }
        self.last_input = sample;
        self.has_pen = true;
        self.add_path_point(surface, sample, changed)
    }

    /// 筆が実際に通る道の点: 間隔ごとにダブを置く（C# の AddPathPoint）。
    fn add_path_point(
        &mut self,
        surface: &mut Surface,
        sample: BrushSample,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        let mut any = false;
        if !self.has_sample {
            any |= self.stamp(surface, sample.x, sample.y, sample.pressure, changed)?;
            self.has_sample = true;
        } else {
            let p = self.previous;
            let dx = sample.x - p.x;
            let dy = sample.y - p.y;
            let length = (dx * dx + dy * dy).sqrt();
            let spacing = f64::max(0.01, self.settings.radius * 2.0 * self.settings.spacing);
            if length / spacing > MAX_STAMPS_PER_SEGMENT {
                return Err(CoreError::InvalidArgument("1 区間のダブが百万を超える"));
            }
            if length > 0.0 {
                let mut position = spacing - self.distance_since_stamp;
                // 許しは入力の境での丸めを吸うだけ。溜めた距離は下で詰める
                while position <= length + 1e-9 {
                    let t = f64::min(1.0, position / length);
                    any |= self.stamp(
                        surface,
                        p.x + dx * t,
                        p.y + dy * t,
                        p.pressure + (sample.pressure - p.pressure) * t,
                        changed,
                    )?;
                    position += spacing;
                }
                self.stroke_length += length;
                self.distance_since_stamp = length - (position - spacing);
                if self.distance_since_stamp < 1e-9 {
                    self.distance_since_stamp = 0.0;
                }
                if self.distance_since_stamp >= spacing {
                    self.distance_since_stamp %= spacing;
                }
            }
        }
        self.previous = sample;
        self.sample_count += 1;
        Ok(any)
    }

    /// ダブ 1 つ（C# の Stamp。筆先は 1 つ、ゆらぎ・入り抜き・傾きなし）。
    fn stamp(
        &mut self,
        surface: &mut Surface,
        x: f64,
        y: f64,
        pressure: f64,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        self.stamp_count += 1;
        let size_factor = 1.0; // 入り抜きなし（C# の Taper(arc, ∞) = 1）
        let radius = self.settings.radius
            * (if self.settings.pressure_size {
                pressure
            } else {
                1.0
            })
            * size_factor;
        if radius <= 0.0 {
            return Ok(false);
        }
        self.dab(surface, x, y, radius, pressure, changed)
    }

    /// 丸い筆先のダブ（C# の Dab と DabPixels の、回転も潰しも無い丸の式）。タイルごとに処理する（各画素は自分の入力だけで決まるので、
    /// 画素を回る順は結果を変えない。予算の確かめは増える一方なので、超えるかどうかも順によらない）。
    ///
    /// 外接の箱が [`PARALLEL_DAB_PIXELS`] 以上で複数のタイルにかかるダブは、このダブで起こり得る写しと確保を全部足しても予算に
    /// 収まるとき（普段はいつも）、タイルごとにワーカーで描き、増えたバイトを後で足す。どの確かめも失敗し得ないので、写すタイル・
    /// 確保・画素は呼んだスレッドだけで描いたときと同じになる。収まらないかもしれないときは、C# と同じく呼んだスレッドで順に描く。
    fn dab(
        &mut self,
        surface: &mut Surface,
        x: f64,
        y: f64,
        radius: f64,
        pressure: f64,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        let extent = radius;
        let (w, h) = (surface.width() as i64, surface.height() as i64);
        let min_x = ((x - extent - 0.5).ceil() as i64).max(0);
        let max_x = ((x + extent - 0.5).floor() as i64).min(w - 1);
        let min_y = ((y - extent - 0.5).ceil() as i64).max(0);
        let max_y = ((y + extent - 0.5).floor() as i64).min(h - 1);
        if min_x > max_x || min_y > max_y {
            return Ok(false);
        }
        let ts = surface.tile_size() as i64;
        let shape = DabShape {
            x,
            y,
            radius,
            hardness: self.settings.hardness,
            pressure,
        };
        let mut spans: Vec<TileSpan> = Vec::new();
        for ty in min_y / ts..=max_y / ts {
            for tx in min_x / ts..=max_x / ts {
                let xs = (min_x.max(tx * ts), max_x.min(tx * ts + ts - 1));
                let ys = (min_y.max(ty * ts), max_y.min(ty * ts + ts - 1));
                spans.push((TileCoord::new(tx as u32, ty as u32), xs, ys));
            }
        }
        let parallel = spans.len() > 1
            && rayon::current_num_threads() > 1
            && (max_x - min_x + 1) * (max_y - min_y + 1) >= PARALLEL_DAB_PIXELS
            && self.fits_any_order(surface, &spans);
        if parallel {
            self.parallel_dabs += 1;
            return Ok(self.dab_parallel(surface, &shape, &spans, changed));
        }
        let mut any = false;
        for &(coord, xs, ys) in &spans {
            if self.with_tile(surface, coord, |cx, held, live| {
                dab_tile(cx, held, live, &shape, coord, xs, ys)
            })? {
                changed.push(coord);
                any = true;
            }
        }
        Ok(any)
    }

    /// このダブのタイルで起こり得る写し（覆い・巻き戻しの写し）と面の確保を全部足しても、どちらの予算にも収まるか（控えめな見積もり）。
    fn fits_any_order(&self, surface: &Surface, spans: &[TileSpan]) -> bool {
        let full = surface.tile_bytes() as u64;
        let (mut capture, mut growth) = (0u64, 0u64);
        for &(coord, _, _) in spans {
            let live = surface.tile(coord);
            if !self.tiles.contains_key(&coord) {
                capture += 64 + full + live.map_or(0, |t| t.byte_size());
            }
            growth += match live {
                None => full,
                Some(Tile::Uniform(_)) => full - 4,
                Some(Tile::Data(_)) => 0,
            };
        }
        self.rollback_bytes + capture <= self.budgets.stroke
            && self
                .budgets
                .growth
                .ensure(surface.allocated, growth as i64)
                .is_ok()
    }

    /// ダブのタイルをワーカーで 1 枚ずつ描く（予算は fits_any_order で確かめ済み）。各ワーカーは自分のタイルの持ち分と面のタイルだけを
    /// 書き、増えたバイトを返す。
    fn dab_parallel(
        &mut self,
        surface: &mut Surface,
        shape: &DabShape,
        spans: &[TileSpan],
        changed: &mut Vec<TileCoord>,
    ) -> bool {
        let ts = surface.tile_size() as usize;
        let mut work: Vec<(TileSpan, Option<StrokeTile>, LiveTile)> = spans
            .iter()
            .map(|&span| {
                let coord = span.0;
                (
                    span,
                    self.tiles.remove(&coord),
                    LiveTile::from(surface.tiles.remove(&coord)),
                )
            })
            .collect();
        let settings = &self.settings;
        let unlimited = Budgets {
            growth: Growth {
                budget: u64::MAX,
                others: 0,
            },
            stroke: u64::MAX,
        };
        let results: Vec<(bool, u64, u64)> = work
            .par_iter_mut()
            .map(|((coord, xs, ys), held, live)| {
                let (mut rollback, mut allocated) = (0u64, 0u64);
                let mut cx = PixelContext {
                    settings,
                    budgets: unlimited,
                    rollback_bytes: &mut rollback,
                    allocated: &mut allocated,
                    tile_size: ts,
                };
                let painted = dab_tile(&mut cx, held, live, shape, *coord, *xs, *ys)
                    .expect("予算は確かめ済みなので失敗しない");
                (painted, rollback, allocated)
            })
            .collect();
        let mut any = false;
        for (((coord, _, _), held, live), (painted, rollback, allocated)) in
            work.into_iter().zip(results)
        {
            if let Some(h) = held {
                self.tiles.insert(coord, h);
            }
            if let Some(l) = live.into_tile() {
                surface.tiles.insert(coord, l);
            }
            self.rollback_bytes += rollback;
            surface.allocated += allocated;
            if painted {
                changed.push(coord);
                any = true;
            }
        }
        any
    }

    /// 与えた覆いを 1 画素に塗る（C# の ApplyPixel。メッシュのダブ向け。筆圧で大きさは変えない）。画布の外は何もしない。
    pub(crate) fn apply_pixel(
        &mut self,
        surface: &mut Surface,
        x: i64,
        y: i64,
        coverage: f64,
        pressure: f64,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        require_finite(coverage, "coverage")?;
        require_finite(pressure, "pressure")?;
        if !(0.0..=1.0).contains(&coverage) || !(0.0..=1.0).contains(&pressure) {
            return Err(CoreError::InvalidArgument("coverage/pressure"));
        }
        if x < 0 || y < 0 || x >= surface.width() as i64 || y >= surface.height() as i64 {
            return Ok(false);
        }
        let ts = surface.tile_size() as i64;
        let coord = TileCoord::new((x / ts) as u32, (y / ts) as u32);
        let local = ((y % ts) * ts + x % ts) as usize;
        let done = self.with_tile(surface, coord, |cx, held, live| {
            apply_at(cx, held, live, local, coverage, pressure)
        })?;
        if done {
            changed.push(coord);
        }
        Ok(done)
    }

    /// 1 枚のタイルの、ストロークの持ち分と面のタイルを取り出して f に渡し、終わったら（失敗しても）戻す。
    /// 面のタイルは、そのタイルで初めて書くときに 1 回だけ自分のものにし（写しと共有なら複製）、後の画素はそのまま書く。
    fn with_tile<F>(
        &mut self,
        surface: &mut Surface,
        coord: TileCoord,
        f: F,
    ) -> Result<bool, CoreError>
    where
        F: FnOnce(
            &mut PixelContext<'_>,
            &mut Option<StrokeTile>,
            &mut LiveTile,
        ) -> Result<bool, CoreError>,
    {
        let mut held = self.tiles.remove(&coord);
        let mut live = LiveTile::from(surface.tiles.remove(&coord));
        let ts = surface.tile_size() as usize;
        let mut cx = PixelContext {
            settings: &self.settings,
            budgets: self.budgets,
            rollback_bytes: &mut self.rollback_bytes,
            allocated: &mut surface.allocated,
            tile_size: ts,
        };
        let result = f(&mut cx, &mut held, &mut live);
        if let Some(h) = held {
            self.tiles.insert(coord, h);
        }
        if let Some(l) = live.into_tile() {
            surface.tiles.insert(coord, l);
        }
        result
    }
}

/// ダブの形（丸）。
struct DabShape {
    x: f64,
    y: f64,
    radius: f64,
    hardness: f64,
    pressure: f64,
}

/// 画素の処理が使う、ストロークと面の状態。
pub(crate) struct PixelContext<'a> {
    settings: &'a BrushSettings,
    budgets: Budgets,
    rollback_bytes: &'a mut u64,
    allocated: &'a mut u64,
    tile_size: usize,
}

/// 外接の箱がこれ以上（画素）のダブは、タイルごとにワーカーで描く（C# と同じ 128²）。小さいダブはワーカーを起こす費用が勝つ
/// （半径 40・460 ダブのストロークで、64² にすると 1 本のスレッドの約 2 倍かかった。2026-10-04 の計測）。
pub(crate) const PARALLEL_DAB_PIXELS: i64 = 128 * 128;

/// 丸い筆先の覆い（C# の DabPixels の plain の経路）。円の外は None。
#[inline(always)]
fn round_coverage(dx: f64, dy: f64, radius: f64, hardness: f64) -> Option<f64> {
    let d = (dx * dx + dy * dy).sqrt() / radius;
    if d > 1.0 {
        return None;
    }
    let mut coverage = 1.0;
    if d > hardness {
        let t = (1.0 - d) / (1.0 - hardness);
        coverage = t * t * (3.0 - 2.0 * t);
    }
    Some(coverage)
}

/// ダブの天井（不透明度 × 筆圧）と流量（覆い × 流量 × 筆圧）。どちらかが 0 以下なら塗らない。
#[inline(always)]
fn ceiling_and_flow(s: &BrushSettings, coverage: f64, pressure: f64) -> (f64, f64) {
    let (opacity_scale, flow_scale) = (1.0, 1.0);
    let ceiling = s.opacity * opacity_scale * (if s.pressure_opacity { pressure } else { 1.0 });
    let flow = coverage * s.flow * flow_scale * (if s.pressure_flow { pressure } else { 1.0 });
    (ceiling, flow)
}

/// ストロークの覆いを天井へ流量の割合だけ寄せる（天井に届いていれば、そのまま）。
#[inline(always)]
fn accumulate(previous: f64, ceiling: f64, flow: f64) -> f64 {
    if previous >= ceiling {
        previous
    } else {
        previous + (ceiling - previous) * f64_min(1.0, flow)
    }
}

/// 描く前の画素とストロークの覆いから、画素の新しい値（塗る: Normal で重ねる、消す: アルファを覆い × 色のアルファだけ減らす）。
#[inline(always)]
fn painted(s: &BrushSettings, start: Rgba8, accumulated: f64) -> Rgba8 {
    if s.erase {
        let alpha =
            to_byte(start.a as f64 / 255.0 * (1.0 - accumulated * s.color.a as f64 / 255.0));
        if alpha == 0 {
            Rgba8::TRANSPARENT
        } else {
            Rgba8::new(start.r, start.g, start.b, alpha)
        }
    } else {
        blend(start, s.color, f64_min(1.0, accumulated), BlendMode::Normal)
    }
}

/// 1 枚のタイルの中のダブの画素（xs・ys は画布の画素の範囲、両端を含む）。
fn dab_tile(
    cx: &mut PixelContext<'_>,
    held: &mut Option<StrokeTile>,
    live: &mut LiveTile,
    s: &DabShape,
    coord: TileCoord,
    xs: (i64, i64),
    ys: (i64, i64),
) -> Result<bool, CoreError> {
    let ts = cx.tile_size as i64;
    let (ox, oy) = (coord.x as i64 * ts, coord.y as i64 * ts);
    let mut changed = false;
    for py in ys.0..=ys.1 {
        let dy = (py as f64 + 0.5) - s.y;
        let row = (py - oy) * ts;
        for px in xs.0..=xs.1 {
            let dx = (px as f64 + 0.5) - s.x;
            let Some(coverage) = round_coverage(dx, dy, s.radius, s.hardness) else {
                continue;
            };
            changed |= apply_at(
                cx,
                held,
                live,
                (row + px - ox) as usize,
                coverage,
                s.pressure,
            )?;
        }
    }
    Ok(changed)
}

/// ダブが触るタイルと、その中の画布の画素の範囲（x の両端、y の両端）。
type TileSpan = (TileCoord, (i64, i64), (i64, i64));

/// 1 画素（C# の ApplyPixelAt の、選択・ステンシル・透明部分のロック・筆先ごとの色・効果の無い 1 チャンネルの経路）。
#[inline]
fn apply_at(
    cx: &mut PixelContext<'_>,
    held: &mut Option<StrokeTile>,
    live: &mut LiveTile,
    local: usize,
    coverage: f64,
    pressure: f64,
) -> Result<bool, CoreError> {
    let s = cx.settings;
    let (ceiling, flow) = ceiling_and_flow(s, coverage, pressure);
    if flow <= 0.0 || ceiling <= 0.0 {
        return Ok(false);
    }
    if let Some(st) = held.as_ref() {
        if st.wash[local] as f64 >= ceiling {
            return Ok(false);
        }
    }
    let ts = cx.tile_size;
    if held.is_none() {
        // タイルを初めて触る: 覆い（float × TileSize²）と写しを合わせて予算と比べる（写しは共有でも全体を数える）
        let capture = live.byte_size();
        let next = *cx.rollback_bytes + 64 + (ts * ts * 4) as u64 + capture;
        if next > cx.budgets.stroke {
            return Err(CoreError::StrokeBudgetExceeded);
        }
        *held = Some(StrokeTile {
            wash: vec![0.0; ts * ts],
            before: live.snapshot(),
        });
        *cx.rollback_bytes = next;
    }
    let st = held.as_mut().expect("直前に作った");
    let accumulated = accumulate(st.wash[local] as f64, ceiling, flow);
    st.wash[local] = accumulated as f32;
    let start = st
        .before
        .as_ref()
        .map_or(Rgba8::TRANSPARENT, |t| t.get(local * 4));
    let next = painted(s, start, accumulated);
    live.write(
        local * 4,
        next,
        cx.allocated,
        ts * ts * 4,
        cx.budgets.growth,
    )
}

/// C# の Math.Min(1, x)。
#[inline(always)]
fn f64_min(a: f64, b: f64) -> f64 {
    if a < b {
        a
    } else {
        b
    }
}
