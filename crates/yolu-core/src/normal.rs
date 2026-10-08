//! Normal チャンネルの計算: レイヤーを単位ベクトルとして重ねる式と、Normal の出力（塗った法線を平らな法線へ載せ、Height から作った
//! 法線を下に敷く）。
//!
//! - バイトは c / 255 × 2 − 1 と読み、使う前に必ず正規化する。長さ² が 1e-12 より短いベクトルは平ら (0, 0, 1)。
//! - レイヤーの重ねは色の合成と同じ W3C の source-over の形で、色の代わりにベクトル: t = 上のアルファ × 不透明度 × マスク、
//!   da = 下のアルファで normalize((1 − t)·da·下 + (1 − da)·t·上 + da·t·B(下, 上))、アルファは t + da(1 − t)。
//! - B は Overlay で RNM（Reoriented Normal Mapping、上を細部として下の向きへ回す）、ほかのモードは上そのもの（置き換え）。
//! - 調整レイヤーはこのチャンネルでも色の式のまま（出力で正規化する）。
//! - 出力は合成をアルファで平らな法線へ載せ（塗っていない所は (128, 128, 255)）、Height から作った法線を土台に塗った法線を
//!   細部として RNM で重ね、不透明。Height → Normal は高さ = R × A（0 の上）を Sobel 3×3 ÷ 8 で微分し（傾き s の坂はちょうど s）、
//!   n = normalize(−強さ·∂h/∂x, −強さ·∂h/∂y, 1)。UV の歪み・アイランドの境・余白は見ない。
//! - レイヤーの重ね（[`blend`]・[`clip_onto`]・[`fade`] と行の核）は f32 の式で、道（スカラー・SSE4.1・AVX2・NEON）とスレッド数によらず同じバイト
//!   （`rows`）。出力の式と、ほかの計算が使う [`decode`]・[`encode`]・[`rnm`] は f64。

use rayon::prelude::*;

mod output;
mod rows;
pub use rows::{blend_row, clip_row, fade_row};

use crate::error::CoreError;
use crate::math::{require_finite, to_byte, UNIT};
use crate::types::{BlendMode, Channel, Rect, Rgba8, RowOrder};
use crate::Document;

/// 法線のファイルの Y の向き。OpenGL（Y+、緑 = +V）が Unity の決まり。DirectX は緑が逆。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[repr(u8)]
pub enum NormalYDirection {
    #[default]
    OpenGL = 0,
    DirectX = 1,
}

/// 高さの微分がキャンバスの外で読むもの: 端のテクセル（Clamp）か、反対側の端（Wrap、繰り返すテクスチャ向け）。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[repr(u8)]
pub enum HeightEdgeMode {
    #[default]
    Clamp = 0,
    Wrap = 1,
}

/// 文書の Normal の出力の設定（変えない値。C# の NormalSettings）。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct NormalSettings {
    derive_from_height: bool,
    strength: f64,
    edges: HeightEdgeMode,
    file_direction: NormalYDirection,
}

impl Default for NormalSettings {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl NormalSettings {
    pub const ALGORITHM_VERSION: i32 = 1;
    pub const MAX_STRENGTH: f64 = 256.0;
    /// 作らない・強さ 4・端は Clamp・ファイルは OpenGL。
    pub const DEFAULT: NormalSettings = NormalSettings {
        derive_from_height: false,
        strength: 4.0,
        edges: HeightEdgeMode::Clamp,
        file_direction: NormalYDirection::OpenGL,
    };

    /// 強さは ±256（高さの全範囲 0 → 1 で上がるテクセルの数。負なら凸が凹に）。
    pub fn new(
        derive_from_height: bool,
        strength: f64,
        edges: HeightEdgeMode,
        file_direction: NormalYDirection,
    ) -> Result<Self, CoreError> {
        require_finite(strength, "strength")?;
        if !(-Self::MAX_STRENGTH..=Self::MAX_STRENGTH).contains(&strength) {
            return Err(CoreError::InvalidArgument("Height → Normal の強さ（±256）"));
        }
        Ok(NormalSettings {
            derive_from_height,
            strength,
            edges,
            file_direction,
        })
    }
    pub fn derive_from_height(&self) -> bool {
        self.derive_from_height
    }
    pub fn strength(&self) -> f64 {
        self.strength
    }
    pub fn edges(&self) -> HeightEdgeMode {
        self.edges
    }
    pub fn file_direction(&self) -> NormalYDirection {
        self.file_direction
    }
    pub fn with_derive(self, value: bool) -> Self {
        NormalSettings {
            derive_from_height: value,
            ..self
        }
    }
    pub fn with_strength(self, value: f64) -> Result<Self, CoreError> {
        Self::new(
            self.derive_from_height,
            value,
            self.edges,
            self.file_direction,
        )
    }
    pub fn with_edges(self, value: HeightEdgeMode) -> Self {
        NormalSettings {
            edges: value,
            ..self
        }
    }
    pub fn with_file_direction(self, value: NormalYDirection) -> Self {
        NormalSettings {
            file_direction: value,
            ..self
        }
    }
}

/// [`Document::normal_output`] の作業メモリの既定の上限（出力のキャンバスと帯）。
pub const DEFAULT_WORKING_BUDGET_BYTES: u64 = 256 * 1024 * 1024;
/// 向きを持たない長さ²（これより短いと平ら）。
const DEGENERATE_LENGTH_SQUARED: f64 = 1e-12;

/// Normal のチャンネルでベクトルの意味を持つモード。ほかは Normal（置き換え）として重ねる。
pub fn is_vector_mode(mode: BlendMode) -> bool {
    matches!(
        mode,
        BlendMode::Normal | BlendMode::PassThrough | BlendMode::Overlay
    )
}
/// Overlay は下へ細部として重ねる（RNM）。
pub fn is_detail(mode: BlendMode) -> bool {
    mode == BlendMode::Overlay
}

#[inline]
fn normalize(x: &mut f64, y: &mut f64, z: &mut f64) {
    let l2 = *x * *x + *y * *y + *z * *z;
    if l2 < DEGENERATE_LENGTH_SQUARED {
        *x = 0.0;
        *y = 0.0;
        *z = 1.0;
        return;
    }
    let l = l2.sqrt();
    *x /= l;
    *y /= l;
    *z /= l;
}

/// 符号化した画素の単位ベクトル（アルファは見ない）。
#[inline]
pub fn decode(c: Rgba8) -> (f64, f64, f64) {
    let mut x = UNIT[c.r as usize] * 2.0 - 1.0;
    let mut y = UNIT[c.g as usize] * 2.0 - 1.0;
    let mut z = UNIT[c.b as usize] * 2.0 - 1.0;
    normalize(&mut x, &mut y, &mut z);
    (x, y, z)
}

/// ベクトルを正規化して符号化する（四捨五入は [`crate::blend`] の丸めと同じ）。
#[inline]
pub fn encode(mut x: f64, mut y: f64, mut z: f64, alpha: u8) -> Rgba8 {
    normalize(&mut x, &mut y, &mut z);
    Rgba8::new(
        to_byte(x * 0.5 + 0.5),
        to_byte(y * 0.5 + 0.5),
        to_byte(z * 0.5 + 0.5),
        alpha,
    )
}

/// Reoriented Normal Mapping: 細部 d を土台 b の向きへ回す（どちらも単位、接空間）。平らな細部・平らな土台では恒等。
/// 真内向き（z = −1）の土台は座標系を持たないので土台のまま。
#[inline]
pub fn rnm(b: (f64, f64, f64), d: (f64, f64, f64)) -> (f64, f64, f64) {
    let (tx, ty, tz) = (b.0, b.1, b.2 + 1.0);
    let (ux, uy, uz) = (-d.0, -d.1, d.2);
    if tz <= 1e-6 {
        return b;
    }
    let k = (tx * ux + ty * uy + tz * uz) / tz;
    (tx * k - ux, ty * k - uy, tz * k - uz)
}

/// 1 つのレイヤーの画素を下へベクトルとして重ねる（不透明度 0〜1。f32 へ丸めて使う）。
pub fn blend(below: Rgba8, over: Rgba8, opacity: f64, mode: BlendMode) -> Rgba8 {
    blend_unchecked(below, over, opacity, mode)
}

/// [`blend`] の中身（行の核と同じ f32 の式）。上が透明・量が 0 以下なら下（その RGB も）をそのまま返す。
#[inline]
pub(crate) fn blend_unchecked(below: Rgba8, over: Rgba8, opacity: f64, mode: BlendMode) -> Rgba8 {
    rows::blend_pixel(below, over, opacity, mode)
}

/// クリッピングされたレイヤーを下地へ: normalize((1 − t)·下地 + t·B(下地, 上))、t = 上のアルファ × 量。下地のアルファのまま。
#[inline]
pub fn clip_onto(group: Rgba8, clipped: Rgba8, amount: f64, mode: BlendMode) -> Rgba8 {
    rows::clip_pixel(group, clipped, amount, mode)
}

/// 通過のグループのフェード: 下と中身のアルファで重みを付けた平均を正規化する。
#[inline]
pub fn fade(backdrop: Rgba8, inner: Rgba8, amount: f64) -> Rgba8 {
    rows::fade_pixel(backdrop, inner, amount)
}

/// 合成の画素を平らな面に載せた法線: normalize(a·n + (1 − a)·(0, 0, 1))。
#[inline]
fn flatten(r: u8, g: u8, b: u8, alpha: u8) -> (f64, f64, f64) {
    if alpha == 0 {
        return (0.0, 0.0, 1.0);
    }
    let (mut x, mut y, mut z) = decode(Rgba8::new(r, g, b, 255));
    let a = UNIT[alpha as usize];
    x *= a;
    y *= a;
    z = z * a + (1.0 - a);
    normalize(&mut x, &mut y, &mut z);
    (x, y, z)
}

/// 高さ = R × A（0 の上。塗っていない所は 0）。
#[inline]
fn height_of(r: u8, a: u8) -> f64 {
    (r as u32 * a as u32) as f64 / 65025.0
}

fn edge_row(row: i64, height: i64, edges: HeightEdgeMode) -> i64 {
    if row < 0 {
        if edges == HeightEdgeMode::Wrap {
            height - 1
        } else {
            0
        }
    } else if row >= height {
        if edges == HeightEdgeMode::Wrap {
            0
        } else {
            height - 1
        }
    } else {
        row
    }
}

/// 緑のバイトを反転する（OpenGL ⇔ DirectX、ちょうど 255 − G）。
pub fn flip_green(rgba: &mut [u8]) {
    for g in rgba.iter_mut().skip(1).step_by(4) {
        *g = 255 - *g;
    }
}

/// 出力の 1 画素（画素 x）。heights は下・同じ・上の行を 0、+w、+2w に持つ。
#[allow(clippy::too_many_arguments)]
fn output_pixel(
    x: usize,
    normal: Option<&[u8]>,
    heights: Option<&[f64]>,
    w: usize,
    wrap: bool,
    s: f64,
    output: &mut [u8],
) {
    let p = match normal {
        Some(n) => {
            let i = x * 4;
            flatten(n[i], n[i + 1], n[i + 2], n[i + 3])
        }
        None => (0.0, 0.0, 1.0),
    };
    let v = match heights {
        Some(h) => {
            let xl = if x > 0 {
                x - 1
            } else if wrap {
                w - 1
            } else {
                0
            };
            let xr = if x < w - 1 {
                x + 1
            } else if wrap {
                0
            } else {
                w - 1
            };
            let (below, center, above) = (0, w, 2 * w);
            let gx = (h[above + xr] + 2.0 * h[center + xr] + h[below + xr]
                - h[above + xl]
                - 2.0 * h[center + xl]
                - h[below + xl])
                / 8.0;
            let gy = (h[above + xl] + 2.0 * h[above + x] + h[above + xr]
                - h[below + xl]
                - 2.0 * h[below + x]
                - h[below + xr])
                / 8.0;
            let (mut hx, mut hy, mut hz) = (-s * gx, -s * gy, 1.0);
            normalize(&mut hx, &mut hy, &mut hz);
            rnm((hx, hy, hz), p)
        }
        None => p,
    };
    let c = encode(v.0, v.1, v.2, 255);
    output[x * 4..x * 4 + 4].copy_from_slice(&[c.r, c.g, c.b, 255]);
}

/// 出力の 1 行。heights は下・同じ・上の行を height_start、+w、+2w に持つ。内側の画素は SIMD で、端（高さを隣から読む画素）と
/// 余りは画素ごとの式で。
#[allow(clippy::too_many_arguments)]
fn row(
    normal: Option<&[u8]>,
    heights: Option<&[f64]>,
    w: usize,
    settings: &NormalSettings,
    output: &mut [u8],
) {
    let wrap = settings.edges == HeightEdgeMode::Wrap;
    let s = settings.strength;
    let (start, end) =
        output::output_row_at(crate::math::simd::level(), normal, heights, w, s, output);
    for x in (0..start).chain(end..w) {
        output_pixel(x, normal, heights, w, wrap, s, output);
    }
}

/// 全面の合成（下の行から、straight RGBA）から出力を作る（GPU の出力の段が 2 つの合成のテクスチャで行うこと）。
/// 作る設定なら高さの合成が要る。
pub fn output_from_composites(
    normal_composite: &[u8],
    height_composite: Option<&[u8]>,
    width: u32,
    height: u32,
    settings: &NormalSettings,
) -> Result<Vec<u8>, CoreError> {
    if width == 0 || height == 0 {
        return Err(CoreError::InvalidArgument("大きさ"));
    }
    let (w, h) = (width as usize, height as usize);
    let length = w * h * 4;
    if normal_composite.len() != length {
        return Err(CoreError::InvalidArgument(
            "Normal の合成は幅 × 高さ × 4 バイト",
        ));
    }
    let derive = settings.derive_from_height;
    let heights_src = if derive {
        match height_composite {
            Some(hc) if hc.len() == length => Some(hc),
            _ => {
                return Err(CoreError::InvalidArgument(
                    "Height の合成は幅 × 高さ × 4 バイト",
                ))
            }
        }
    } else {
        None
    };
    let mut output = vec![0u8; length];
    let mut heights = vec![0.0f64; if derive { 3 * w } else { 0 }];
    for y in 0..h {
        if let Some(hc) = heights_src {
            for r in 0..3 {
                let rr = edge_row(y as i64 + r as i64 - 1, h as i64, settings.edges) as usize;
                output::heights_from_rgba(
                    crate::math::simd::level(),
                    &hc[rr * w * 4..(rr + 1) * w * 4],
                    &mut heights[r * w..(r + 1) * w],
                );
            }
        }
        row(
            Some(&normal_composite[y * w * 4..(y + 1) * w * 4]),
            derive.then_some(&heights[..]),
            w,
            settings,
            &mut output[y * w * 4..(y + 1) * w * 4],
        );
    }
    Ok(output)
}

impl Document {
    /// 文書の Normal の出力の設定。
    pub fn normal_settings(&self) -> NormalSettings {
        self.normal_settings
    }

    /// Normal のレイヤーが無くても Normal の出力があるか: Height → Normal が有効で、Height を使うレイヤーがある。
    pub fn derives_normal(&self) -> bool {
        self.normal_settings.derive_from_height
            && self
                .layers()
                .iter()
                .any(|l| l.is_channel_enabled(Channel::Height))
    }

    /// [`Document::normal_output`] が確保するバイト数: 出力のキャンバス、Normal の合成の帯 1 つ、作る設定なら上下に 1 行ずつの
    /// 余白を持つ Height の帯（バイトと高さ）。
    pub fn normal_working_bytes(&self) -> u64 {
        let (w, h) = (self.width() as u64, self.height() as u64);
        let band = (self.tile_size() as u64).min(h);
        let mut bytes = 4 * w * h + 4 * w * band;
        if self.normal_settings.derive_from_height {
            bytes += (4 + 8) * w * (band + 2);
        }
        bytes
    }

    /// Unity へ渡す Normal の出力（OpenGL Y+、下の行から、不透明）: 塗った Normal の合成を平らな法線へ載せ、有効なら Height から
    /// 作った法線を土台にする。作業のバイト数が上限を超えるなら、確保の前に断る。
    pub fn normal_output(&self, max_working_bytes: u64) -> Result<Vec<u8>, CoreError> {
        if self.normal_working_bytes() > max_working_bytes {
            return Err(CoreError::WorkingBudgetExceeded);
        }
        self.evaluate_normal(
            &self.normal_settings,
            true,
            self.normal_settings.derive_from_height,
        )
    }

    /// 他のツールへのファイル向け: 文書のファイルの Y の向き（DirectX なら緑を反転）。
    pub fn normal_file_output(&self, max_working_bytes: u64) -> Result<Vec<u8>, CoreError> {
        let mut bytes = self.normal_output(max_working_bytes)?;
        if self.normal_settings.file_direction == NormalYDirection::DirectX {
            flip_green(&mut bytes);
        }
        Ok(bytes)
    }

    /// 高さのチャンネルだけから作った法線（OpenGL Y+、不透明）。文書の設定で作るかどうかによらず、与えた設定の強さと端で。
    /// 高さとして読めるのは Height のチャンネルだけ（Roughness・Metallic もスカラーだが高さではない）。
    pub fn derive_normal_from_height(
        &self,
        source: Channel,
        settings: &NormalSettings,
        max_working_bytes: u64,
    ) -> Result<Vec<u8>, CoreError> {
        self.require_channel(source)?;
        if source != Channel::Height {
            return Err(CoreError::Unsupported(
                "Height → Normal が読むのは Height のチャンネルだけ",
            ));
        }
        let (w, h) = (self.width() as u64, self.height() as u64);
        let needed = 4 * w * h + (4 + 8) * w * ((self.tile_size() as u64).min(h) + 2);
        if needed > max_working_bytes {
            return Err(CoreError::WorkingBudgetExceeded);
        }
        self.evaluate_normal(settings, false, true)
    }

    /// 帯ごと: Normal と Height の合成を 1 帯ずつ（Height は上下に 1 行の余白）。出力のほかにキャンバス全体を持たない。
    fn evaluate_normal(
        &self,
        settings: &NormalSettings,
        painted: bool,
        derive: bool,
    ) -> Result<Vec<u8>, CoreError> {
        let (w, h) = (self.width() as usize, self.height() as usize);
        let band = (self.tile_size() as usize).min(h);
        let mut output = vec![0u8; w * h * 4];
        let mut heights = vec![0.0f64; if derive { w * (band + 2) } else { 0 }];
        let mut normal = vec![0u8; if painted { w * band * 4 } else { 0 }];
        let mut y0 = 0;
        while y0 < h {
            let rows = band.min(h - y0);
            if painted {
                self.composite_into(
                    Channel::Normal,
                    Rect::new(0, y0 as u32, w as u32, rows as u32),
                    &mut normal[..w * rows * 4],
                    RowOrder::BottomUp,
                )?;
            }
            if derive {
                self.load_heights(settings.edges, y0, rows, &mut heights)?;
            }
            let out = &mut output[y0 * w * 4..(y0 + rows) * w * 4];
            let normal = &normal;
            let heights = &heights;
            let job = |(r, o): (usize, &mut [u8])| {
                row(
                    painted.then(|| &normal[r * w * 4..(r + 1) * w * 4]),
                    derive.then(|| &heights[r * w..(r + 3) * w]),
                    w,
                    settings,
                    o,
                )
            };
            // 行は独立（読むのは帯の合成と高さ、書くのはその行だけ）
            if rows * w < 1 << 14 {
                out.chunks_mut(w * 4).enumerate().for_each(job);
            } else {
                out.par_chunks_mut(w * 4).enumerate().for_each(job);
            }
            y0 += rows;
        }
        Ok(output)
    }

    /// heights[(r + 1) × w + x] = 行 y0 + r の高さ（r = −1 … rows、端の行は端の規則で）。
    fn load_heights(
        &self,
        edges: HeightEdgeMode,
        y0: usize,
        rows: usize,
        heights: &mut [f64],
    ) -> Result<(), CoreError> {
        let (w, h) = (self.width() as usize, self.height() as usize);
        let a = y0.saturating_sub(1);
        let b = h.min(y0 + rows + 1);
        let mut inner = vec![0u8; w * (b - a) * 4];
        self.composite_into(
            Channel::Height,
            Rect::new(0, a as u32, w as u32, (b - a) as u32),
            &mut inner,
            RowOrder::BottomUp,
        )?;
        let mut single = Vec::new();
        for r in -1..=rows as i64 {
            let rr = edge_row(y0 as i64 + r, h as i64, edges) as usize;
            let source: &[u8] = if rr < a || rr >= b {
                single.resize(w * 4, 0);
                self.composite_into(
                    Channel::Height,
                    Rect::new(0, rr as u32, w as u32, 1),
                    &mut single,
                    RowOrder::BottomUp,
                )?;
                &single
            } else {
                &inner[(rr - a) * w * 4..(rr - a + 1) * w * 4]
            };
            let base = ((r + 1) as usize) * w;
            output::heights_from_rgba(
                crate::math::simd::level(),
                &source[..w * 4],
                &mut heights[base..base + w],
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLAT: Rgba8 = Rgba8::new(128, 128, 255, 255);
    const TILT_X: Rgba8 = Rgba8::new(255, 128, 128, 255);
    const TILT45: Rgba8 = Rgba8::new(218, 128, 218, 255);

    #[test]
    fn partial_coverage_is_a_renormalized_average() {
        // C# NormalChannelTests: 平らと真横の半々は 45°。色の式なら (192, 128, 192)
        assert_eq!(
            blend(FLAT, TILT_X, 0.5, BlendMode::Normal),
            Rgba8::new(218, 128, 218, 255)
        );
        assert_eq!(
            crate::blend::blend(FLAT, TILT_X, 0.5, BlendMode::Normal),
            Rgba8::new(192, 128, 192, 255)
        );
    }

    #[test]
    fn overlay_reorients_and_other_modes_replace() {
        assert_eq!(
            blend(TILT45, TILT45, 1.0, BlendMode::Overlay),
            Rgba8::new(255, 128, 127, 255)
        );
        assert_eq!(
            blend(TILT45, TILT45, 0.5, BlendMode::Overlay),
            Rgba8::new(245, 128, 176, 255)
        );
        assert_eq!(rnm((0.6, -0.0, 0.8), (0.0, 0.0, 1.0)), (0.6, 0.0, 0.8));
        assert_eq!(rnm((0.0, 0.0, 1.0), (0.6, -0.0, 0.8)), (0.6, 0.0, 0.8));
        let over = Rgba8::new(60, 200, 210, 200);
        for mode in BlendMode::LAYER_MODES {
            assert_eq!(
                is_vector_mode(mode),
                matches!(mode, BlendMode::Normal | BlendMode::Overlay)
            );
            if mode != BlendMode::Overlay {
                assert_eq!(
                    blend(TILT45, over, 0.6, mode),
                    blend(TILT45, over, 0.6, BlendMode::Normal)
                );
            }
        }
    }

    #[test]
    fn transparent_pixels_keep_the_backdrop() {
        let hidden = Rgba8::new(10, 20, 30, 0);
        assert_eq!(
            blend(hidden, Rgba8::new(255, 0, 0, 0), 1.0, BlendMode::Normal),
            hidden
        );
        assert_eq!(blend(hidden, TILT_X, 0.0, BlendMode::Normal), hidden);
        assert_eq!(clip_onto(hidden, TILT_X, 1.0, BlendMode::Normal), hidden);
        assert_eq!(
            blend(
                Rgba8::TRANSPARENT,
                Rgba8::new(200, 90, 230, 128),
                1.0,
                BlendMode::Normal
            ),
            Rgba8::new(198, 91, 227, 128)
        );
    }

    #[test]
    fn settings_are_checked() {
        assert!(NormalSettings::new(
            true,
            f64::NAN,
            HeightEdgeMode::Clamp,
            NormalYDirection::OpenGL
        )
        .is_err());
        assert!(
            NormalSettings::new(true, 300.0, HeightEdgeMode::Clamp, NormalYDirection::OpenGL)
                .is_err()
        );
        assert!(NormalSettings::new(
            true,
            -300.0,
            HeightEdgeMode::Clamp,
            NormalYDirection::OpenGL
        )
        .is_err());
        assert!(NormalSettings::new(
            true,
            -256.0,
            HeightEdgeMode::Wrap,
            NormalYDirection::DirectX
        )
        .is_ok());
        let mut px = vec![1u8, 2, 3, 4, 5, 250, 7, 8];
        flip_green(&mut px);
        assert_eq!(px, vec![1, 253, 3, 4, 5, 5, 7, 8]);
    }
}
