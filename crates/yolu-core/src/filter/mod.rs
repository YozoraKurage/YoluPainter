//! 非破壊フィルター。入力は左下原点の straight RGBA8、マスクは隠す量。
//! 文書・履歴・キャッシュに依存せず、成功時だけ完成した領域を返す。
//! C# FilterEngine のアルゴリズム版 1。カーブ・HSL は調整レイヤーの責務。
//! Normalize の全域統計は `statistics` で単独に求められ、`Options::statistics` で評価へ渡せる（タイルごとの再走査を避けられる）。

mod generated;
mod pixels;
mod rows;
mod seams;
pub use seams::seam_working_bytes;
mod spatial;
#[cfg(test)]
mod tests;
use crate::{
    math::{clamp01, to_byte, UNIT},
    ranges, BrightnessContrast, ColorBalance, GradientMap, Posterize, Rect, Rgba8, Threshold,
    ToneCurves,
};
use rayon::prelude::*;
use std::{
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};

pub const MAX_STACK: usize = 32;
pub const MAX_HALO: u32 = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueType {
    Color,
    Scalar,
    TangentNormal,
    Mask,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locality {
    Point,
    Neighborhood,
    Global,
}

/// Generator のマップ解決は呼び出し側。値は 0..1、欠損は None。
/// 同じ座標は評価中いつも同じ値を返すこと。ランプ適用後は Mapped を返す。
/// 型は [`crate::generator::Generated`] そのもの（束縛済みの Generator の結果を写し替えずに渡せる）。
pub use crate::generator::Generated;
pub trait GeneratorInput: Sync {
    fn sample(&self, slot: u32, x: u32, y: u32) -> Option<Generated>;
    /// 行 `y` の `x0` から `out.len()` 画素の `sample` と同じ結果（範囲外は None）。評価器は段ごと・行ごとにこちらを呼ぶ。
    /// 既定は `sample` を 1 画素ずつ呼ぶ。行をまとめて作れる入力（束縛済みの Generator）は置き換える。
    fn sample_row(&self, slot: u32, x0: u32, y: u32, out: &mut [Option<Generated>]) {
        for (i, o) in out.iter_mut().enumerate() {
            *o = self.sample(slot, x0 + i as u32, y);
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeneratorBlend {
    Multiply,
    Replace,
    Screen,
    Max,
    Min,
    Add,
    Subtract,
}

/// スロープぼかしの合わせ方（取った値の平均・最小・最大）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlopeMode {
    Blur,
    Min,
    Max,
}
/// モルフォロジーの向き（丸いウィンドウの最大で太らせる・最小で細らせる）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MorphologyMode {
    Dilate,
    Erode,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Settings {
    GaussianBlur {
        radius: u32,
    },
    Sharpen {
        radius: u32,
        amount: f64,
        threshold: u32,
    },
    Noise {
        amount: f64,
        seed: i32,
        monochrome: bool,
    },
    Levels {
        input_black: f64,
        input_white: f64,
        gamma: f64,
        output_black: f64,
        output_white: f64,
    },
    Invert,
    Normalize,
    /// マップの種類・ランプの計算ではなく、解決済み Generator 出力の合成段。
    Generator {
        slot: u32,
        blend: GeneratorBlend,
    },
    // 色調補正の 6 種（調整レイヤーと同じ値と式。Rust 版だけの種類で、段の種類の番号は 64 から。画素ごとの点の処理で、強さは元との混ぜ）。
    /// グラデーションマップ。色のチャンネルだけ。
    GradientMap(GradientMap),
    /// トーンカーブ。スカラーとマスクでは RGB 全体の曲線だけが効く。
    ToneCurve(ToneCurves),
    /// カラーバランス。色のチャンネルだけ。
    ColorBalance(ColorBalance),
    BrightnessContrast(BrightnessContrast),
    Threshold(Threshold),
    Posterize(Posterize),
    // 0.5.0 の 10 種（Rust 版だけ。段の種類の番号は 70〜79）。式は `spatial.rs`（値の表の 2 種は `point`）。
    /// ヒストグラムスキャン（70）: 幅 w = max(1 − contrast, 1/255)、out = clamp((in − (position − w/2)) / w)。スカラーとマスクだけ。
    HistogramScan {
        position: f64,
        contrast: f64,
    },
    /// ヒストグラムレンジ（71）: out = clamp(position + (in − 0.5) × range)。スカラーとマスクだけ。
    HistogramRange {
        range: f64,
        position: f64,
    },
    /// スロープぼかし（72）: 内蔵の値ノイズの勾配の向きへ intensity 画素を samples 回取り、平均・最小・最大。
    SlopeBlur {
        intensity: f64,
        samples: u32,
        mode: SlopeMode,
        scale: f64,
        seed: i32,
    },
    /// 方向のぼかし（73）: angle の向きの両側へ distance 画素の線のぼかし。
    DirectionalBlur {
        angle: f64,
        distance: f64,
    },
    /// ゆがみ（74）: 内蔵の値ノイズの勾配で読む位置をずらす（双線形）。
    Warp {
        intensity: f64,
        scale: f64,
        seed: i32,
    },
    /// モルフォロジー（75）: 丸いウィンドウの最大（dilate）・最小（erode）。スカラーとマスクだけ。
    Morphology {
        mode: MorphologyMode,
        radius: u32,
    },
    /// エッジ検出（76）: width でぼかしてから Sobel の強さ、threshold 以下を 0。スカラーとマスクだけ。
    EdgeDetect {
        width: u32,
        threshold: f64,
    },
    /// ハイパス（77）: out = 0.5 + (in − ぼかし(in, radius))。アルファは元のまま。
    HighPass {
        radius: u32,
    },
    /// メディアン（78）: 正方形のウィンドウの中央値（チャンネルごと）。
    Median {
        radius: u32,
    },
    /// グロー（79）: out = in + ぼかし(max(in − threshold, 0), radius) × intensity。色のチャンネルだけ。アルファは元のまま。
    Glow {
        threshold: f64,
        radius: u32,
        intensity: f64,
    },
}
impl Settings {
    pub const ALGORITHM_VERSION: u32 = 1;
    pub fn halo(&self) -> u32 {
        match *self {
            Self::GaussianBlur { radius } | Self::Sharpen { radius, .. } => radius,
            Self::SlopeBlur { intensity, .. } | Self::Warp { intensity, .. } => halo_of(intensity),
            Self::DirectionalBlur { distance, .. } => halo_of(distance),
            Self::Morphology { radius, .. }
            | Self::HighPass { radius }
            | Self::Median { radius }
            | Self::Glow { radius, .. } => radius,
            Self::EdgeDetect { width, .. } => width + 1,
            _ => 0,
        }
    }
    /// 0.5.0 の近傍の段（`spatial.rs` が評価する。長さ 0 のスロープぼかし・方向のぼかし・ゆがみは何もしない）。
    pub(crate) fn is_spatial(&self) -> bool {
        matches!(
            self,
            Self::SlopeBlur { .. }
                | Self::DirectionalBlur { .. }
                | Self::Warp { .. }
                | Self::Morphology { .. }
                | Self::EdgeDetect { .. }
                | Self::HighPass { .. }
                | Self::Median { .. }
                | Self::Glow { .. }
        )
    }
    /// 粗い評価（歩幅 `stride` 画素ごとに 1 画素）の段: 画素で数える長さ・半径・ノイズの大きさを歩幅で割る（丸めて 0 になる半径の段は
    /// None。長さは実数のまま割る）。0.5.0 の近傍の段だけ（ぼかし・シャープは文書の評価の側が割る）。
    pub(crate) fn coarse(&self, stride: u32) -> Option<Self> {
        let k = f64::from(stride.max(1));
        let reduce = |r: u32| (r + stride / 2) / stride.max(1);
        Some(match *self {
            Self::SlopeBlur {
                intensity,
                samples,
                mode,
                scale,
                seed,
            } => Self::SlopeBlur {
                intensity: intensity / k,
                samples,
                mode,
                // ノイズの塊は 1 画素より細かくしない（範囲の下限。粗い絵では細かい模様は見えない）
                scale: (scale / k).max(*ranges::FILTER_NOISE_SCALE.start()),
                seed,
            },
            Self::DirectionalBlur { angle, distance } => Self::DirectionalBlur {
                angle,
                distance: distance / k,
            },
            Self::Warp {
                intensity,
                scale,
                seed,
            } => Self::Warp {
                intensity: intensity / k,
                scale: (scale / k).max(*ranges::FILTER_NOISE_SCALE.start()),
                seed,
            },
            Self::Morphology { mode, radius } => Self::Morphology {
                mode,
                radius: Some(reduce(radius)).filter(|r| *r > 0)?,
            },
            Self::EdgeDetect { width, threshold } => Self::EdgeDetect {
                width: reduce(width).max(1),
                threshold,
            },
            Self::HighPass { radius } => Self::HighPass {
                radius: Some(reduce(radius)).filter(|r| *r > 0)?,
            },
            Self::Median { radius } => Self::Median {
                radius: Some(reduce(radius)).filter(|r| *r > 0)?,
            },
            Self::Glow {
                threshold,
                radius,
                intensity,
            } => Self::Glow {
                threshold,
                radius: reduce(radius).max(1),
                intensity,
            },
            _ => self.clone(),
        })
    }
    pub fn locality(&self) -> Locality {
        if matches!(self, Self::Normalize) {
            Locality::Global
        } else if self.halo() > 0 {
            Locality::Neighborhood
        } else {
            Locality::Point
        }
    }
    pub fn validate(&self, value_type: ValueType) -> Result<(), Error> {
        self.validate_values()?;
        self.validate_type(value_type)
    }
    /// 値の範囲だけの検査（値の種類に依らない。効果の目録が欄の値から組むときに使う）。
    pub fn validate_values(&self) -> Result<(), Error> {
        let valid = match *self {
            Self::GaussianBlur { radius } => ranges::BLUR_RADIUS.contains(&radius),
            Self::Sharpen {
                radius,
                amount,
                threshold,
            } => {
                ranges::SHARPEN_RADIUS.contains(&radius)
                    && amount.is_finite()
                    && ranges::SHARPEN_AMOUNT.contains(&amount)
                    && ranges::SHARPEN_THRESHOLD.contains(&threshold)
            }
            Self::Noise { amount, .. } => {
                amount.is_finite() && ranges::NOISE_AMOUNT.contains(&amount)
            }
            Self::Levels {
                input_black: b,
                input_white: w,
                gamma: g,
                output_black: ob,
                output_white: ow,
            } => {
                [b, w, g, ob, ow].iter().all(|v| v.is_finite())
                    && b >= *ranges::LEVELS_UNIT.start()
                    && w <= *ranges::LEVELS_UNIT.end()
                    && w - b >= 1.0 / 255.0
                    && ranges::GAMMA.contains(&g)
                    && ranges::LEVELS_UNIT.contains(&ob)
                    && ranges::LEVELS_UNIT.contains(&ow)
            }
            Self::HistogramScan { position, contrast } => {
                within(&ranges::UNIT, position) && within(&ranges::UNIT, contrast)
            }
            Self::HistogramRange { range, position } => {
                within(&ranges::UNIT, range) && within(&ranges::UNIT, position)
            }
            Self::SlopeBlur {
                intensity,
                samples,
                scale,
                ..
            } => {
                within(&ranges::SLOPE_INTENSITY, intensity)
                    && ranges::SLOPE_SAMPLES.contains(&samples)
                    && within(&ranges::FILTER_NOISE_SCALE, scale)
            }
            Self::DirectionalBlur { angle, distance } => {
                within(&ranges::DIRECTIONAL_ANGLE, angle)
                    && within(&ranges::DIRECTIONAL_DISTANCE, distance)
            }
            Self::Warp {
                intensity, scale, ..
            } => {
                within(&ranges::WARP_INTENSITY, intensity)
                    && within(&ranges::FILTER_NOISE_SCALE, scale)
            }
            Self::Morphology { radius, .. } => ranges::MORPHOLOGY_RADIUS.contains(&radius),
            Self::EdgeDetect { width, threshold } => {
                ranges::EDGE_WIDTH.contains(&width) && within(&ranges::UNIT, threshold)
            }
            Self::HighPass { radius } => ranges::HIGH_PASS_RADIUS.contains(&radius),
            Self::Median { radius } => ranges::MEDIAN_RADIUS.contains(&radius),
            Self::Glow {
                threshold,
                radius,
                intensity,
            } => {
                within(&ranges::UNIT, threshold)
                    && ranges::GLOW_RADIUS.contains(&radius)
                    && within(&ranges::GLOW_INTENSITY, intensity)
            }
            _ => true,
        };
        if !valid {
            return Err(Error::Invalid("フィルターの設定が範囲外です"));
        }
        Ok(())
    }
    /// 値の種類（色・スカラー・接空間法線・マスク）に使えるか。
    fn validate_type(&self, value_type: ValueType) -> Result<(), Error> {
        if value_type == ValueType::TangentNormal && !matches!(self, Self::GaussianBlur { .. }) {
            return Err(Error::Invalid(
                "接空間法線には再正規化するぼかしだけを適用できます",
            ));
        }
        if matches!(value_type, ValueType::Scalar | ValueType::Mask)
            && matches!(
                self,
                Self::Noise {
                    monochrome: false,
                    ..
                }
            )
        {
            return Err(Error::Invalid("スカラーとマスクは色ノイズを受け付けません"));
        }
        if matches!(value_type, ValueType::Scalar | ValueType::Mask)
            && matches!(self, Self::GradientMap(_) | Self::ColorBalance(_))
        {
            return Err(Error::Invalid(
                "グラデーションマップとカラーバランスは色のチャンネルだけに適用できます",
            ));
        }
        if value_type == ValueType::Color
            && matches!(
                self,
                Self::HistogramScan { .. }
                    | Self::HistogramRange { .. }
                    | Self::Morphology { .. }
                    | Self::EdgeDetect { .. }
            )
        {
            return Err(Error::Invalid(
                "値の切り出し・値の幅・太らせる・細らせる・輪郭の検出はスカラーとマスクだけに適用できます",
            ));
        }
        if value_type != ValueType::Color && matches!(self, Self::Glow { .. }) {
            return Err(Error::Invalid("グローは色のチャンネルだけに適用できます"));
        }
        Ok(())
    }
}
/// 有限で範囲の中か。
fn within(range: &std::ops::RangeInclusive<f64>, v: f64) -> bool {
    v.is_finite() && range.contains(&v)
}
/// 実数の長さ（画素）の到達半径（切り上げ）。
fn halo_of(length: f64) -> u32 {
    length.ceil() as u32
}
#[derive(Clone, Debug, PartialEq)]
pub struct Stage {
    pub settings: Settings,
    pub enabled: bool,
    pub strength: f64,
}
impl Stage {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings,
            enabled: true,
            strength: 1.0,
        }
    }
    fn active(&self) -> bool {
        self.enabled && self.strength > 0.0
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid(&'static str),
    Budget { needed: u64, budget: u64 },
    Cancelled,
    Allocation,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(s) => f.write_str(s),
            Self::Budget { needed, budget } => write!(
                f,
                "フィルターの作業メモリが予算を超えます: {needed} > {budget} バイト"
            ),
            Self::Cancelled => f.write_str("フィルターの評価を取り消しました"),
            Self::Allocation => f.write_str("フィルターの作業メモリを確保できません"),
        }
    }
}
impl std::error::Error for Error {}

/// 読み取り専用。タイルの境界でもキャンバス座標を受け取る。
/// Mask の場合は返す画素の A が隠す量で、RGB は無視する。
pub trait Source: Sync {
    fn dimensions(&self) -> (u32, u32);
    fn pixel(&self, x: u32, y: u32) -> [u8; 4];
    /// 行の一部（x から `out.len() / 4` 画素）をまとめて読む。既定は 1 画素ずつ。タイルの面など、まとめて写せる読み元は置き換える。
    fn read_row(&self, x: u32, y: u32, out: &mut [u8]) {
        for (i, p) in out.chunks_exact_mut(4).enumerate() {
            p.copy_from_slice(&self.pixel(x + i as u32, y));
        }
    }
}
pub struct Image<'a> {
    data: &'a [u8],
    width: u32,
    height: u32,
}
impl<'a> Image<'a> {
    pub fn new(data: &'a [u8], width: u32, height: u32) -> Result<Self, Error> {
        if width == 0
            || height == 0
            || u64::from(width)
                .checked_mul(u64::from(height))
                .and_then(|n| n.checked_mul(4))
                != Some(data.len() as u64)
        {
            return Err(Error::Invalid("画像は幅×高さ×4バイト必要です"));
        }
        Ok(Self {
            data,
            width,
            height,
        })
    }
}
impl Source for Image<'_> {
    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        self.data[i..i + 4].try_into().unwrap()
    }
    fn read_row(&self, x: u32, y: u32, out: &mut [u8]) {
        let start = (y as usize * self.width as usize + x as usize) * 4;
        out.copy_from_slice(&self.data[start..start + out.len()]);
    }
}

/// 予算は返却画像と同時に生きている作業画素を含む。入力画像は借用なので除外。
/// メタデータ・rayon のスレッドスタックは含まない。C# のブロック上限式を使う。
pub struct Options<'a> {
    pub working_budget: u64,
    pub block_size: u32,
    pub cancel: Option<&'a AtomicBool>,
    pub generators: Option<&'a dyn GeneratorInput>,
    /// `statistics` が返した値（段と同じ並び）。Some の段は走査せずその値を使う。None の段は評価の中で求める。
    /// 入力・スタック・Generator の値が変わったら取り直すのは呼び出し側。長さと段の種類だけ検証する。
    pub statistics: Option<&'a [Option<Statistics>]>,
    /// UV の継ぎ目をまたいで読む帯の写し（大きさは読み元と同じこと）。Some なら、近傍の段（`halo` > 0）は、段の入力のアイランドの外の帯を
    /// 継ぎ目の相手のアイランドの画素で埋めてからかけ、アイランドの外は段の入力のまま戻す（`seams.rs`）。None は今までと同じバイト。
    pub seams: Option<&'a crate::geometry::SeamBand>,
}
impl Default for Options<'_> {
    fn default() -> Self {
        Self {
            working_budget: 256 * 1024 * 1024,
            block_size: 256,
            cancel: None,
            generators: None,
            statistics: None,
            seams: None,
        }
    }
}
impl Options<'_> {
    fn check(&self) -> Result<(), Error> {
        if self.cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}
fn zeros<T: Default + Clone>(n: usize) -> Result<Vec<T>, Error> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| Error::Allocation)?;
    v.resize(n, T::default());
    Ok(v)
}
fn area(r: Rect) -> usize {
    r.width as usize * r.height as usize
}
fn grow(r: Rect, m: u32, w: u32, h: u32) -> Rect {
    let x = r.x.saturating_sub(m);
    let y = r.y.saturating_sub(m);
    Rect::new(
        x,
        y,
        (r.x + r.width).saturating_add(m).min(w) - x,
        (r.y + r.height).saturating_add(m).min(h) - y,
    )
}
/// C# WorkingBytes の画素バッファ上限に縦パスの行累積を加える。
/// 返却画像の予算は別途加算する。
pub fn block_working_bytes(
    stack: &[Stage],
    block: u32,
    width: u32,
    height: u32,
) -> Result<u64, Error> {
    if block == 0
        || block > 4096
        || width == 0
        || height == 0
        || width > i32::MAX as u32
        || height > i32::MAX as u32
    {
        return Err(Error::Invalid("画像・ブロックの寸法が範囲外です"));
    }
    let mut halo = 0u32;
    for s in stack.iter().filter(|s| s.active()) {
        halo = halo
            .checked_add(s.settings.halo())
            .ok_or(Error::Invalid("halo が大きすぎます"))?;
    }
    if halo > MAX_HALO {
        return Err(Error::Invalid("スタックの半径の合計は512画素までです"));
    }
    let a = |m: u32| {
        u64::from(width.min(block.saturating_add(2 * m)))
            * u64::from(height.min(block.saturating_add(2 * m)))
    };
    let mut peak = a(halo) * 4;
    for s in stack.iter().filter(|s| s.active()) {
        let input = a(halo);
        let row_sums = u64::from(width.min(block.saturating_add(2 * halo))) * 16;
        halo -= s.settings.halo();
        let output = a(halo);
        let input_width =
            u64::from(width.min(block.saturating_add(2 * (halo + s.settings.halo()))));
        peak = peak.max(if s.settings.is_spatial() {
            spatial::working_bytes(&s.settings, input, output, input_width, row_sums)
        } else if s.settings.halo() > 0 {
            input * 28 + output * 4 + row_sums
        } else {
            input * 4
        });
    }
    Ok(peak)
}

/// Normalize の段が読む全域統計。その段への入力全体で、アルファが正の画素の RGB の最小と最大（マスクは全画素）。
/// 該当する画素が無いときは 0・0（何も変えない）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Statistics {
    pub min: u8,
    pub max: u8,
}

/// 評価の前に決まる計画。設定の検証・寸法・予算の見積りまでを 1 か所に集める。
struct Plan {
    /// 有効な段だけ（強さ 0・無効の段は除く）。
    chain: Vec<Stage>,
    /// `chain` と同じ並びで、呼び出し側が渡した統計。
    supplied: Vec<Option<Statistics>>,
    width: u32,
    height: u32,
    /// 1 ブロックの作業バイト（`block_working_bytes`）。
    working: u64,
}

fn prepare(
    source: &dyn Source,
    stack: &[Stage],
    value_type: ValueType,
    region: Option<Rect>,
    options: &Options<'_>,
) -> Result<Plan, Error> {
    options.check()?;
    let (w, h) = source.dimensions();
    if w == 0
        || h == 0
        || w > i32::MAX as u32
        || h > i32::MAX as u32
        || region.is_some_and(|r| {
            r.width == 0
                || r.height == 0
                || u64::from(r.x) + u64::from(r.width) > u64::from(w)
                || u64::from(r.y) + u64::from(r.height) > u64::from(h)
        })
        || options.block_size == 0
        || options.block_size > 4096
    {
        return Err(Error::Invalid("画像・領域・ブロックの寸法が範囲外です"));
    }
    if stack.len() > MAX_STACK {
        return Err(Error::Invalid("スタックは32段までです"));
    }
    if options.statistics.is_some_and(|s| s.len() != stack.len()) {
        return Err(Error::Invalid("統計の数が段の数と合いません"));
    }
    let mut chain = Vec::new();
    let mut supplied = Vec::new();
    for (i, s) in stack.iter().enumerate() {
        s.settings.validate(value_type)?;
        if !s.strength.is_finite() || !(0.0..=1.0).contains(&s.strength) {
            return Err(Error::Invalid("強さは有限の0..1です"));
        }
        if s.active()
            && matches!(s.settings, Settings::Generator { .. })
            && options.generators.is_none()
        {
            return Err(Error::Invalid("ジェネレーターの入力解決器がありません"));
        }
        let given = options.statistics.and_then(|all| all[i]);
        if let Some(st) = given {
            if !s.active() || !matches!(s.settings, Settings::Normalize) || st.min > st.max {
                return Err(Error::Invalid(
                    "統計は有効な Normalize の段にだけ、最小 <= 最大で付けられます",
                ));
            }
        }
        if s.active() {
            chain.push(s.clone());
            supplied.push(given);
        }
    }
    let mut working = block_working_bytes(&chain, options.block_size, w, h)?;
    if let Some(band) = options.seams {
        if (band.width(), band.height()) != (w, h) {
            return Err(Error::Invalid("継ぎ目の帯の写しの大きさが画像と合いません"));
        }
        // 帯の写しがあるので、帯のテクセルの数は分かる（文書が評価の前に見積もる最悪の数以下）
        working = working.saturating_add(seam_working_bytes(
            &chain,
            options.block_size,
            w,
            h,
            band.texel_count() as u64,
        ));
    }
    Ok(Plan {
        chain,
        supplied,
        width: w,
        height: h,
        working,
    })
}

/// 同時に動かすブロック数。空き予算に収まる数を、rayon の現在のプールのスレッド数で頭打ちにする（最小 1）。
fn worker_count(free_budget: u64, working: u64) -> usize {
    (free_budget / working.max(1))
        .min(rayon::current_num_threads() as u64)
        .max(1) as usize
}

/// 各段の Normalize の全域統計だけを求める。返す並びは `stack` と同じで、有効な Normalize の段だけが Some。
/// 統計はその段より前の段の出力（画像全体）から取る。走査はブロック並列で、結果は並列度・ブロックの大きさによらない。
/// `Options::statistics` に渡した段はそのまま返す。文書側が世代ごとに持ち、領域ごとの `evaluate` へ渡せる。
pub fn statistics(
    source: &dyn Source,
    value_type: ValueType,
    stack: &[Stage],
    options: &Options<'_>,
) -> Result<Vec<Option<Statistics>>, Error> {
    let plan = prepare(source, stack, value_type, None, options)?;
    if plan.working > options.working_budget {
        return Err(Error::Budget {
            needed: plan.working,
            budget: options.working_budget,
        });
    }
    let mut engine = Engine::new(source, value_type, &plan, options);
    engine.resolve_statistics(worker_count(options.working_budget, plan.working))?;
    let mut out = vec![None; stack.len()];
    let mut k = 0;
    for (i, s) in stack.iter().enumerate() {
        if s.active() {
            out[i] = engine.stats[k];
            k += 1;
        }
    }
    Ok(out)
}

/// 領域の完成画像。Mask も RGBA8 で返し、RGB=0・A=隠す量。
/// Normalize は指定領域の外を含む各段の入力全体で統計を取る（`Options::statistics` があればそれを使う）。
/// 取消・予算拒否・設定不正では入力を一切変更しない。
pub fn evaluate(
    source: &dyn Source,
    value_type: ValueType,
    stack: &[Stage],
    region: Rect,
    options: &Options<'_>,
) -> Result<Vec<u8>, Error> {
    let plan = prepare(source, stack, value_type, Some(region), options)?;
    let output_bytes = u64::from(region.width) * u64::from(region.height) * 4;
    let needed = plan
        .working
        .checked_add(output_bytes)
        .ok_or(Error::Allocation)?;
    if needed > options.working_budget {
        return Err(Error::Budget {
            needed,
            budget: options.working_budget,
        });
    }
    let mut engine = Engine::new(source, value_type, &plan, options);
    // 統計は返却画像を確保する前に済ませる（走査中は返却画像が生きていないので、予算いっぱいまで並列にできる）。
    engine.resolve_statistics(worker_count(options.working_budget, plan.working))?;
    let chain = &plan.chain;
    let mut output = zeros::<u8>(usize::try_from(output_bytes).map_err(|_| Error::Allocation)?)?;
    let workers = worker_count(options.working_budget - output_bytes, plan.working);
    let band_rows = options.block_size as usize;
    let stride = region.width as usize * 4;
    // 同時に確保するブロック数を予算内に制限する。rayon の既存プールを使い、新たなスレッドは作らない。
    for (batch, bands) in output.chunks_mut(stride * band_rows * workers).enumerate() {
        bands
            .par_chunks_mut(stride * band_rows)
            .enumerate()
            .try_for_each(|(band, dst)| -> Result<(), Error> {
                let y = region.y + ((batch * workers + band) * band_rows) as u32;
                let bh = (dst.len() / stride) as u32;
                for dx in (0..region.width).step_by(options.block_size as usize) {
                    let r = Rect::new(
                        region.x + dx,
                        y,
                        options.block_size.min(region.width - dx),
                        bh,
                    );
                    let data = engine.rect(chain.len(), r)?;
                    for row in 0..bh as usize {
                        let start = row * stride + dx as usize * 4;
                        dst[start..start + r.width as usize * 4].copy_from_slice(
                            &data[row * r.width as usize * 4..(row + 1) * r.width as usize * 4],
                        );
                    }
                }
                Ok(())
            })?;
    }
    options.check()?;
    if value_type == ValueType::Mask {
        for p in output.chunks_exact_mut(4) {
            let hide = p[0];
            p.copy_from_slice(&[0, 0, 0, hide]);
        }
    }
    Ok(output)
}
/// 色調補正の段（調整レイヤーと同じ式）の 1 画素の結果。トーンカーブはスカラーとマスクでは RGB 全体の曲線だけ。
fn adjust_pixel(settings: &Settings, value_type: ValueType, c: Rgba8) -> Rgba8 {
    match settings {
        Settings::GradientMap(v) => v.apply(c),
        Settings::ToneCurve(v) if matches!(value_type, ValueType::Scalar | ValueType::Mask) => {
            v.apply_scalar(c)
        }
        Settings::ToneCurve(v) => v.apply(c),
        Settings::ColorBalance(v) => v.apply(c),
        Settings::BrightnessContrast(v) => v.apply(c),
        Settings::Threshold(v) => v.apply(c),
        Settings::Posterize(v) => v.apply(c),
        _ => c,
    }
}
struct Engine<'a> {
    source: &'a dyn Source,
    value_type: ValueType,
    chain: &'a [Stage],
    options: &'a Options<'a>,
    /// `chain` と同じ並び。Normalize の段だけ、渡された値か走査の結果が入る。
    stats: Vec<Option<Statistics>>,
    width: u32,
    height: u32,
}
/// アルファが正の画素の RGB の最小・最大。該当が無ければ (255, 0)。
fn min_max(data: &[u8]) -> (u8, u8) {
    let (mut min, mut max) = (255u8, 0u8);
    for p in data.chunks_exact(4).filter(|p| p[3] > 0) {
        for &v in &p[..3] {
            min = min.min(v);
            max = max.max(v);
        }
    }
    (min, max)
}
impl<'a> Engine<'a> {
    fn new(
        source: &'a dyn Source,
        value_type: ValueType,
        plan: &'a Plan,
        options: &'a Options<'a>,
    ) -> Self {
        Self {
            source,
            value_type,
            chain: &plan.chain,
            options,
            stats: plan.supplied.clone(),
            width: plan.width,
            height: plan.height,
        }
    }
    /// 渡されていない Normalize の統計を、前の段から順に求める（後の段の統計は前の段の統計に依る）。
    fn resolve_statistics(&mut self, workers: usize) -> Result<(), Error> {
        for k in 0..self.chain.len() {
            if self.stats[k].is_none() && matches!(self.chain[k].settings, Settings::Normalize) {
                self.stats[k] = Some(self.scan(k, workers)?);
            }
        }
        Ok(())
    }
    /// 段 k の入力（段 0..k の出力）を画像全体で、ブロックを捨てながら集約する。`workers` ブロックずつ並列に処理し、
    /// 最小・最大は順序によらないので並列度で結果は変わらない。ブロックの一覧は持たず、番号から矩形を作る。
    fn scan(&self, k: usize, workers: usize) -> Result<Statistics, Error> {
        let block = self.options.block_size;
        let across = u64::from(self.width.div_ceil(block));
        let total = across * u64::from(self.height.div_ceil(block));
        let (mut min, mut max) = (255u8, 0u8);
        let mut start = 0u64;
        while start < total {
            let end = (start + workers as u64).min(total);
            let parts = (start..end)
                .into_par_iter()
                .map(|i| {
                    let x = (i % across) as u32 * block;
                    let y = (i / across) as u32 * block;
                    let r = Rect::new(x, y, block.min(self.width - x), block.min(self.height - y));
                    Ok(min_max(&self.rect(k, r)?))
                })
                .collect::<Result<Vec<_>, Error>>()?;
            for (lo, hi) in parts {
                min = min.min(lo);
                max = max.max(hi);
            }
            start = end;
        }
        Ok(if min > max {
            Statistics { min: 0, max: 0 }
        } else {
            Statistics { min, max }
        })
    }
    fn rect(&self, count: usize, target: Rect) -> Result<Vec<u8>, Error> {
        self.options.check()?;
        let mut after = self.chain[..count].iter().map(|s| s.settings.halo()).sum();
        let mut cur = grow(target, after, self.width, self.height);
        let mut buf = zeros::<u8>(area(cur) * 4)?;
        let row_bytes = cur.width as usize * 4;
        for y in 0..cur.height {
            self.options.check()?;
            let row = &mut buf[y as usize * row_bytes..(y as usize + 1) * row_bytes];
            self.source.read_row(cur.x, cur.y + y, row);
            if self.value_type == ValueType::Mask {
                for p in row.chunks_exact_mut(4) {
                    p.copy_from_slice(&[p[3], p[3], p[3], 255]);
                }
            }
        }
        for (k, s) in self.chain[..count].iter().enumerate() {
            self.options.check()?;
            after -= s.settings.halo();
            let next = grow(target, after, self.width, self.height);
            if s.settings.halo() > 0 {
                buf = self.neighborhood_stage(k, s, buf, cur, next)?;
            } else {
                self.point(&mut buf, cur, k, s)?;
            }
            cur = next;
        }
        Ok(buf)
    }
    /// 色調補正の段の結果の行（src と同じ並びの RGBA）を out へ（`adjust_pixel` の画素ごとの結果と同じ。カラーバランスだけレーンで）。
    fn adjust_row(
        &self,
        level: crate::math::simd::Level,
        settings: &Settings,
        src: &[u8],
        out: &mut [u8],
    ) {
        if let Settings::ColorBalance(c) = settings {
            crate::adjust::color_balance_rgba(level, c, src, out);
            return;
        }
        for (p, o) in src.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
            let v = adjust_pixel(settings, self.value_type, Rgba8::from_slice(p));
            o.copy_from_slice(&v.to_array());
        }
    }
    fn point(&self, buf: &mut [u8], r: Rect, k: usize, s: &Stage) -> Result<(), Error> {
        let mut lut = [0u8; 256];
        for (v, b) in lut.iter_mut().enumerate() {
            *b = match s.settings {
                Settings::Invert => 255 - v as u8,
                Settings::Levels {
                    input_black,
                    input_white,
                    gamma,
                    output_black,
                    output_white,
                } => to_byte(
                    output_black
                        + clamp01((v as f64 / 255.0 - input_black) / (input_white - input_black))
                            .powf(1.0 / gamma)
                            * (output_white - output_black),
                ),
                Settings::HistogramScan { position, contrast } => {
                    let w = (1.0 - contrast).max(1.0 / 255.0);
                    to_byte(clamp01((UNIT[v] - (position - w / 2.0)) / w))
                }
                Settings::HistogramRange { range, position } => {
                    to_byte(clamp01(position + (UNIT[v] - 0.5) * range))
                }
                Settings::Normalize => {
                    let st = self.stats[k].expect("統計は評価の前に段の順で確定している");
                    if st.max > st.min {
                        to_byte(clamp01(
                            (v as f64 - f64::from(st.min)) / f64::from(st.max - st.min),
                        ))
                    } else {
                        v as u8
                    }
                }
                _ => v as u8,
            };
        }
        let level = crate::math::simd::level();
        let width4 = r.width as usize * 4;
        // 調整の段が行ごとの結果を置く作業の領域
        let mut adjusted: Vec<u8> = Vec::new();
        // Generator の段の行の値（`generated::spans` の範囲の外は前の行の値が残るが、そこは透明な画素で読まない）
        let mut values: Vec<Option<Generated>> = Vec::new();
        for (y, row) in buf.chunks_exact_mut(width4).enumerate() {
            self.options.check()?;
            let y_canvas = r.y + y as u32;
            match s.settings {
                Settings::Noise {
                    amount,
                    seed,
                    monochrome,
                } => rows::noise_row_at(
                    level, row, r.x, y_canvas, seed, amount, monochrome, s.strength,
                ),
                Settings::GradientMap(_)
                | Settings::ToneCurve(_)
                | Settings::ColorBalance(_)
                | Settings::BrightnessContrast(_)
                | Settings::Threshold(_)
                | Settings::Posterize(_) => {
                    adjusted.resize(width4, 0);
                    self.adjust_row(level, &s.settings, row, &mut adjusted);
                    rows::lerp_rows_at(level, row, &adjusted, s.strength);
                }
                Settings::Generator { slot, blend } => {
                    let Some(input) = self.options.generators else {
                        continue;
                    };
                    let mask = self.value_type == ValueType::Mask;
                    values.resize(r.width as usize, None);
                    generated::spans(row, mask, |start, end| {
                        input.sample_row(
                            slot,
                            r.x + start as u32,
                            y_canvas,
                            &mut values[start..end],
                        );
                    });
                    for (p, g) in row.chunks_exact_mut(4).zip(&values) {
                        if p[3] == 0 && !mask {
                            continue;
                        }
                        let Some(g) = *g else {
                            continue;
                        };
                        if matches!(g,Generated::Scalar(v) if !v.is_finite() || !(0.0..=1.0).contains(&v))
                        {
                            return Err(Error::Invalid("ジェネレーターの値は有限の0..1です"));
                        }
                        pixels::generate(
                            p,
                            g,
                            blend,
                            s.strength,
                            self.value_type == ValueType::Mask,
                        );
                    }
                }
                // 長さ 0 のスロープぼかし・方向のぼかし・ゆがみ（近傍の段で半径が 0）は何もしない
                _ if s.settings.is_spatial() => {}
                _ => rows::lut_row_at(level, row, &lut, s.strength),
            }
        }
        Ok(())
    }
}
