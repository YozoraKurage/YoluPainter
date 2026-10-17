//! 合成済みの参照元と、写像されたダブ（C# の BrushStroke.Sources）。3D の面のクローン・指先の土台。
//!
//! - **合成の参照元**: クローンが読むのは、描くレイヤーだけでなく見えているレイヤーの重なり（チャンネルごとの合成）。最初のダブの前に、
//!   レイヤーが画素を持ち得るタイルだけ合成して凍結する（書き込みの途中で合成を読み返さない）。タイルの写しと索引はストロークの予算に
//!   数え、超えたらストローク全体を取り消す。確定・取消で手放す。
//! - **写像されたダブ**: 面のダブの画素ごとに、読む場所（画素中心の最大 4 点と重み）を呼び手が決めて渡す。3D の面は UV アイランドの
//!   継ぎ目をまたいで隣の面の画素を読むので、画素の隣り合わせでは決まらない（[`crate::geometry::SamplingChart`]）。全ての読みを
//!   どの画素を書くより前に済ませ、プリマルチプライドで混ぜた色を画素ごとに持つ。同じ画素の重複・範囲外・参照の無い画素は、
//!   ダブごと取り消す。

use crate::Reason;
use std::collections::HashMap;

use super::effects::byte255;
use super::*;

/// 画素中心の参照 1 点（キャンバスの画素の座標）。重みが 0 の点は読まない。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BrushSourceTap {
    pub x: i64,
    pub y: i64,
    pub weight: f64,
}

impl BrushSourceTap {
    /// 読まない点。
    pub const NONE: BrushSourceTap = BrushSourceTap {
        x: 0,
        y: 0,
        weight: 0.0,
    };

    pub fn new(x: i64, y: i64, weight: f64) -> BrushSourceTap {
        BrushSourceTap { x, y, weight }
    }
}

/// 面のダブの画素と、継ぎ目の向こうも含む最大 4 点の参照（正の重みの和で正規化する）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrushMappedPixel {
    pub pixel: BrushPixel,
    pub a: BrushSourceTap,
    pub b: BrushSourceTap,
    pub c: BrushSourceTap,
    pub d: BrushSourceTap,
}

impl BrushMappedPixel {
    /// 参照 1 点だけ（重み 1）。
    pub fn single(pixel: BrushPixel, source_x: i64, source_y: i64) -> BrushMappedPixel {
        BrushMappedPixel {
            pixel,
            a: BrushSourceTap::new(source_x, source_y, 1.0),
            b: BrushSourceTap::NONE,
            c: BrushSourceTap::NONE,
            d: BrushSourceTap::NONE,
        }
    }

    pub fn new(
        pixel: BrushPixel,
        a: BrushSourceTap,
        b: BrushSourceTap,
        c: BrushSourceTap,
        d: BrushSourceTap,
    ) -> BrushMappedPixel {
        BrushMappedPixel { pixel, a, b, c, d }
    }
}

/// 凍結した合成のタイル 1 枚（straight RGBA8、行は下から上）。
pub(crate) struct CompositeTile {
    pub x: i64,
    pub width: i64,
    pub pixels: Vec<u8>,
}

/// 1 つのチャンネルの、見えているレイヤーの重なりを凍結した参照元（疎: レイヤーが画素を持ち得るタイルだけ。無い所は透明）。
pub(crate) struct CloneSource {
    tile_size: i64,
    tiles: HashMap<TileCoord, CompositeTile>,
    bytes: u64,
}

impl CloneSource {
    pub(crate) fn new(
        tile_size: u32,
        tiles: HashMap<TileCoord, CompositeTile>,
        bytes: u64,
    ) -> CloneSource {
        CloneSource {
            tile_size: tile_size as i64,
            tiles,
            bytes,
        }
    }

    /// 予算に数えたバイト（タイルの写しと名目の索引。C# の cloneSourceBytes）。
    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }

    /// 画素 (x, y) の合成（タイルが無ければ透明）。
    pub(crate) fn read(&self, x: i64, y: i64) -> Rgba8 {
        let ts = self.tile_size;
        match self
            .tiles
            .get(&TileCoord::new((x / ts) as u32, (y / ts) as u32))
        {
            None => Rgba8::TRANSPARENT,
            Some(t) => {
                let i = (((y % ts) * t.width + x - t.x) * 4) as usize;
                Rgba8::from_slice(&t.pixels[i..i + 4])
            }
        }
    }

    /// 行 y の、x から始まる out.len() 画素（同じタイルの中。タイルの無い所は透明のまま）。
    pub(crate) fn read_row(&self, y: i64, x: i64, out: &mut [Rgba8]) {
        let ts = self.tile_size;
        let Some(t) = self
            .tiles
            .get(&TileCoord::new((x / ts) as u32, (y / ts) as u32))
        else {
            return;
        };
        let start = (((y % ts) * t.width + x - t.x) * 4) as usize;
        let end = start + out.len() * 4;
        for (o, p) in out.iter_mut().zip(t.pixels[start..end].chunks_exact(4)) {
            *o = Rgba8::from_slice(p);
        }
    }
}

/// 写像されたダブの、画素ごとの参照の色（混ぜ終えたもの）。
pub(crate) struct MappedColors {
    index: HashMap<i64, usize>,
    colors: Vec<Rgba8>,
}

impl MappedColors {
    /// 画素 (x, y) の参照の色（ダブの画素に必ずある）。
    pub(crate) fn get(&self, width: i64, x: i64, y: i64) -> Rgba8 {
        self.colors[self.index[&(y * width + x)]]
    }
}

impl StrokeState {
    /// 合成の参照元を受けられるか: クローンで、最初のダブの前（入力も効果のダブも、手を付けたタイルも無く、まだ参照元も無い）。
    pub(crate) fn can_take_source(&self) -> bool {
        (matches!(self.brush.effect, BrushEffect::Clone { .. }) || self.mixes_from_composite())
            && !self.has_sample
            && !self.effect.started
            && self.tiles.is_empty()
            && self.source.is_none()
    }

    /// 色の混ぜが見えているレイヤーの重なりを下地にするブラシか（色を塗るブラシだけ。複数チャンネルのストロークでは、混ぜが効かない
    /// データのチャンネルの面も、同じ参照元を受けるものとして数える）。
    fn mixes_from_composite(&self) -> bool {
        self.brush.mix.wants_composite() && self.brush.effect.is_paint() && !self.brush.base.erase
    }

    pub(crate) fn set_source(&mut self, source: CloneSource) {
        self.source = Some(source);
    }

    /// 凍結した合成の参照元のバイト（試験・知らせ用）。
    pub(crate) fn clone_source_bytes(&self) -> u64 {
        self.source_bytes()
    }

    /// 効果が読む色（C# の ReadEffectSource）: 合成の参照元があればその合成、無ければ今の面。クローンは、ストロークがもう触った
    /// タイルでは巻き戻しの写し（ストロークの前）を読む。
    fn read_effect_source(&self, surface: &Surface, x: i64, y: i64) -> Result<Rgba8, CoreError> {
        if let Some(source) = &self.source {
            return Ok(source.read(x, y));
        }
        let ts = self.tile_size;
        let coord = TileCoord::new((x / ts) as u32, (y / ts) as u32);
        let at = (((y % ts) * ts + x % ts) * 4) as usize;
        match (&self.brush.effect, self.tiles.get(&coord)) {
            (BrushEffect::Clone { .. }, Some(held)) => Ok(held
                .before_px
                .as_ref()
                .map_or(Rgba8::TRANSPARENT, |t| t.get(at))),
            _ => {
                let mut c = [surface
                    .tile(coord)
                    .map_or(Ok(Rgba8::TRANSPARENT), |t| t.get(at))?];
                // 伸ばすの面のダブ: 描いている間に付けた水彩境界の縁は、下地に入れない（縁の無い値で読む）
                self.ground_without_edge(coord, at / 4, &mut c);
                Ok(c[0])
            }
        }
    }

    /// 参照の最大 4 点を、アルファで重みを付けて混ぜる（C# の SampleMapped）。参照が 1 点だけなら、その色そのもの。
    fn sample_mapped(&self, surface: &Surface, p: &BrushMappedPixel) -> Result<Rgba8, CoreError> {
        let (mut r, mut g, mut b, mut alpha, mut weight) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
        let mut count = 0;
        let mut single = Rgba8::TRANSPARENT;
        for tap in [p.a, p.b, p.c, p.d] {
            if tap.weight == 0.0 {
                continue;
            }
            let c = self.read_effect_source(surface, tap.x, tap.y)?;
            count += 1;
            single = c;
            let a = c.a as f64 * tap.weight;
            r += c.r as f64 * a;
            g += c.g as f64 * a;
            b += c.b as f64 * a;
            alpha += a;
            weight += tap.weight;
        }
        if count == 1 {
            return Ok(single);
        }
        if alpha <= 0.0 {
            return Ok(Rgba8::TRANSPARENT);
        }
        Ok(Rgba8::new(
            byte255(r / alpha),
            byte255(g / alpha),
            byte255(b / alpha),
            byte255(alpha / weight),
        ))
    }

    /// クローン・指先の面のダブを、全画素の参照を書く前にまとめて凍結してから 1 ダブ適用する（C# の ApplyMappedDab）。
    /// 指先の最初の拾いと面の方向は呼び手が決める。選択範囲の量は描き込みだけに掛かる。
    ///
    /// `targets` は同じ入力を受けるチャンネルの数（予算の見積もりに使う: 画素ごとの色をチャンネルの数だけ持つ）、`sampling_bytes` は
    /// 呼び手が作った参照の計画（展開の図など）の名目のバイト。見積もりが予算を超えたら、参照を読む前にストロークごと取り消す。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_mapped_dab(
        &mut self,
        surface: &mut Surface,
        pixels: &[BrushMappedPixel],
        pressure: f64,
        sampling_bytes: u64,
        points: Option<&[StencilPoint]>,
        targets: usize,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        // 色の混ぜを頼まれたが、このチャンネルでは効かない（複数チャンネルのストロークのデータのチャンネル）: 同じダブを混ぜずに塗る
        if !self.mix_on
            && self.brush.mix.is_active()
            && self.brush.effect.is_paint()
            && !self.brush.base.erase
        {
            let plain: Vec<BrushPixel> = pixels.iter().map(|p| p.pixel).collect();
            let any = self.apply_surface_dab(
                surface,
                &plain,
                DVec2::ZERO,
                None,
                pressure,
                points,
                changed,
            )?;
            return Ok(self.flush_water_edge(surface, changed)? | any);
        }
        let smears = self.mix_on && self.brush.mix.mode == MixMode::Smear;
        if !smears
            && !matches!(
                self.brush.effect,
                BrushEffect::Clone { .. } | BrushEffect::Smudge { .. } | BrushEffect::Blur { .. }
            )
        {
            return Err(CoreError::Unsupported(Reason::MappedDabsNeedSamplingBrush));
        }
        if points.is_some_and(|p| p.len() != pixels.len()) {
            return Err(CoreError::InvalidArgument(
                Reason::StencilPointCountMismatch,
            ));
        }
        require_finite(pressure, Reason::Pressure)?;
        if !(0.0..=1.0).contains(&pressure) {
            return Err(CoreError::InvalidArgument(Reason::Pressure));
        }
        self.effect.scratch = 0;
        self.effect.has_position = true;
        self.effect.started = true;
        // 呼び手の参照の計画も含む名目の大きさ（実際のヒープの上限ではない）
        let per_pixel = 160 + targets as u64 * 4 + if points.is_some() { 24 } else { 0 };
        let bytes = (pixels.len() as u64)
            .checked_mul(per_pixel)
            .and_then(|b| b.checked_add(sampling_bytes))
            .ok_or(CoreError::StrokeBudgetExceeded)?;
        if self
            .rollback_bytes
            .saturating_add(self.source_bytes())
            .saturating_add(bytes)
            > self.budgets.stroke
        {
            return Err(CoreError::StrokeBudgetExceeded);
        }
        self.effect.scratch = bytes;
        if smears {
            self.begin_mapped_mix_dab(self.surface_pressure(self.brush.pressure_scale(pressure)));
        }
        let result = self.apply_mapped(surface, pixels, pressure, points, changed);
        self.effect.scratch = 0;
        let any = result?;
        // 描いている間に付ける水彩境界（伸ばすの面のダブ）: このダブの画素の範囲を計算し直す
        Ok(self.flush_water_edge(surface, changed)? | any)
    }

    fn apply_mapped(
        &mut self,
        surface: &mut Surface,
        pixels: &[BrushMappedPixel],
        pressure: f64,
        points: Option<&[StencilPoint]>,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        let mut mapped = MappedColors {
            index: HashMap::with_capacity(pixels.len()),
            colors: Vec::with_capacity(pixels.len()),
        };
        let valid_tap = |t: &BrushSourceTap, w: i64, h: i64| {
            t.weight.is_finite()
                && (0.0..=1.0).contains(&t.weight)
                && (t.weight == 0.0 || (t.x >= 0 && t.x < w && t.y >= 0 && t.y < h))
        };
        for (i, p) in pixels.iter().enumerate() {
            let dest = p.pixel;
            require_finite(dest.coverage, Reason::Coverage)?;
            if dest.x < 0
                || dest.x >= self.width
                || dest.y < 0
                || dest.y >= self.height
                || !(0.0..=1.0).contains(&dest.coverage)
            {
                return Err(CoreError::InvalidArgument(Reason::MappedDabPixel));
            }
            for t in [&p.a, &p.b, &p.c, &p.d] {
                if !valid_tap(t, self.width, self.height) {
                    return Err(CoreError::InvalidArgument(Reason::MappedDabSource));
                }
            }
            if p.a.weight + p.b.weight + p.c.weight + p.d.weight <= 0.0 {
                return Err(CoreError::InvalidArgument(Reason::MappedPixelNeedsSource));
            }
            if mapped
                .index
                .insert(dest.y * self.width + dest.x, i)
                .is_some()
            {
                return Err(CoreError::InvalidArgument(Reason::DuplicateMappedDabPixel));
            }
            mapped.colors.push(self.sample_mapped(surface, p)?);
        }
        // はみ出し防止: 範囲の外の画素は書かない（読みは全部の画素で済ませた後）
        let keep = self.refer_keep(
            pixels
                .iter()
                .map(|p| (p.pixel.x, p.pixel.y, p.pixel.coverage)),
            None,
        )?;
        if self.edge_live() {
            for (i, p) in pixels.iter().enumerate() {
                if keep.as_ref().is_none_or(|k| k[i]) {
                    self.edge_mark_pixel(p.pixel.x, p.pixel.y);
                }
            }
        }
        let brush = self.brush.clone();
        let pressure = self.surface_pressure(brush.pressure_scale(pressure));
        let mut paint = self.paint(&brush, None);
        paint.mapped = Some(&mapped);
        let ts = surface.tile_size() as i64;
        let mut cursor = TileCursor::default();
        let mut any = false;
        let mut result = Ok(());
        for (i, p) in pixels.iter().enumerate() {
            if keep.as_ref().is_some_and(|k| !k[i]) {
                continue;
            }
            let d = p.pixel;
            let coord = TileCoord::new((d.x / ts) as u32, (d.y / ts) as u32);
            let local = ((d.y % ts) * ts + d.x % ts) as usize;
            let point = points.map(|v| v[i]);
            let Some((scales, paper)) = self.surface_scales(d.x, d.y) else {
                continue;
            };
            if let Err(e) = cursor.move_to(self, surface, coord) {
                result = Err(e);
                break;
            }
            let r = cursor.with(self, surface, &paint, |cx, held, live| {
                apply_at::<false>(
                    cx,
                    held,
                    live,
                    coord,
                    local,
                    d.coverage as f32,
                    pressure,
                    scales,
                    paper,
                    point,
                )
            });
            match r {
                Ok(true) => {
                    if changed.last() != Some(&coord) {
                        changed.push(coord);
                    }
                    any = true;
                }
                Ok(false) => {}
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        cursor.release(self, surface);
        result.map(|_| any)
    }
}
