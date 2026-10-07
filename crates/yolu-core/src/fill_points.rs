//! 点のグラデーション（塗りつぶしのチャンネルの値の作り方の 1 つ）: 置いた点ごとに色（と不透明度）を決め、点からの距離の重みで
//! 滑らかにつなぐ。
//!
//! - 空間: モデルの空間（焼いた位置のマップから、テクセルの 3D の位置。UV の継ぎ目で切れない）か UV の空間（テクセルの中心の UV）。
//!   モデルの空間の点の位置は、形のグラデーションの形と同じモデルのルートの空間（シーンの単位）。UV の空間の点は (u, v, 0)。
//! - つなぎ方: 点 i の重み `1 / (d² + s²)`（d は点までの距離、s は広がり × 長さの目安。モデルの空間は位置のマップの箱の対角線、
//!   UV の空間は 1）。s が 0 なら逆距離の 2 乗の重みで、点の上はその点の色そのもの。s を上げると点の周りの平らな所が丸くなり、
//!   遠くの色が混ざる。色は不透明度で重みを付けて混ぜる（透明な点の RGB が、ほかの点の色を暗くしない）。
//! - 点の数は 1〜[`MAX_POINTS`]。1 つなら全面がその色。

use crate::generator::{Map, MapKind, MapState, ModelFrame};
use crate::math::to_byte;
use crate::types::Rgba8;

/// 点の数の上限。
pub const MAX_POINTS: usize = 64;
/// 点の位置の大きさの上限（モデルの空間はシーンの単位、UV の空間は UV）。
pub const MAX_COORDINATE: f64 = 1e6;

/// 点を置く空間。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PointSpace {
    /// モデルの空間（焼いた位置のマップを読む）。
    #[default]
    Model = 0,
    /// UV の空間（0〜1 の正方形。外へ置いてもよい）。
    Uv = 1,
}

impl TryFrom<u8> for PointSpace {
    type Error = &'static str;
    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Self::Model),
            1 => Ok(Self::Uv),
            _ => Err("未知の点の空間"),
        }
    }
}

/// 置いた点 1 つ。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientPoint {
    /// 位置（モデルの空間はモデルのルートの空間のシーンの単位、UV の空間は (u, v, 0)）。
    pub position: [f64; 3],
    /// 色と不透明度（straight RGBA8。スカラーのチャンネルは灰色の値）。
    pub color: Rgba8,
}

/// 点のグラデーションの設定。
#[derive(Clone, Debug, PartialEq)]
pub struct PointGradient {
    pub space: PointSpace,
    /// 広がり（0〜1）。点の周りの平らさと、遠くの点の色の混ざり方。
    pub spread: f64,
    pub points: Vec<GradientPoint>,
}

impl Default for PointGradient {
    fn default() -> Self {
        PointGradient {
            space: PointSpace::Model,
            spread: DEFAULT_SPREAD,
            points: Vec::new(),
        }
    }
}

/// 広がりの既定。
pub const DEFAULT_SPREAD: f64 = 0.1;

impl PointGradient {
    /// 点の数（1〜64）・位置（有限で ±1e6）・広がり（0〜1）を確かめる。UV の空間の点の z は 0。
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.points.is_empty() || self.points.len() > MAX_POINTS {
            return Err("点の数は 1〜64");
        }
        if !self.spread.is_finite() || !(0.0..=1.0).contains(&self.spread) {
            return Err("広がりは 0〜1");
        }
        for p in &self.points {
            if p.position
                .iter()
                .any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE)
            {
                return Err("点の位置が範囲外");
            }
            if self.space == PointSpace::Uv && p.position[2] != 0.0 {
                return Err("UV の空間の点の z は 0");
            }
        }
        Ok(())
    }

    /// 位置 `at`（その空間の座標）の色。`scale` は広がりに掛ける長さ（モデルの空間は位置のマップの箱の対角線、UV の空間は 1）。
    pub fn color_at(&self, at: [f64; 3], scale: f64) -> Rgba8 {
        let s = self.spread * scale;
        let s2 = s * s;
        let (mut w_sum, mut a_sum) = (0.0f64, 0.0f64);
        let (mut rgb, mut rgb_plain) = ([0.0f64; 3], [0.0f64; 3]);
        for p in &self.points {
            let d2 = (0..3)
                .map(|k| (p.position[k] - at[k]) * (p.position[k] - at[k]))
                .sum::<f64>();
            let denominator = d2 + s2;
            if denominator <= 0.0 {
                // 広がり 0 で点の真上: その点の色
                return p.color;
            }
            let w = 1.0 / denominator;
            let a = p.color.a as f64;
            w_sum += w;
            a_sum += w * a;
            for (k, c) in [p.color.r, p.color.g, p.color.b].into_iter().enumerate() {
                rgb[k] += w * a * c as f64;
                rgb_plain[k] += w * c as f64;
            }
        }
        if w_sum <= 0.0 || !w_sum.is_finite() {
            return self.points.first().map_or(Rgba8::TRANSPARENT, |p| p.color);
        }
        let alpha = a_sum / w_sum;
        let channel = |k: usize| {
            if a_sum > 0.0 {
                rgb[k] / a_sum
            } else {
                rgb_plain[k] / w_sum
            }
        };
        Rgba8::new(
            to_byte(channel(0) / 255.0),
            to_byte(channel(1) / 255.0),
            to_byte(channel(2) / 255.0),
            to_byte(alpha / 255.0),
        )
    }
}

/// 点のグラデーションが使えない理由（モデルの空間で、位置のマップかモデルのルートが使えない）。Generator の理由と同じ形。
pub fn inactive_reason(
    g: &PointGradient,
    position: Option<&Map<'_>>,
    frame: Option<ModelFrame>,
    dimensions: (u32, u32),
) -> Option<crate::generator::Inactive> {
    use crate::generator::Inactive;
    if g.space == PointSpace::Uv {
        return None;
    }
    let Some(m) = position else {
        return Some(Inactive::MissingMap(MapKind::Position));
    };
    if m.state == MapState::Stale {
        return Some(Inactive::StaleMap(MapKind::Position));
    }
    if m.state == MapState::Unverified {
        return Some(Inactive::UnverifiedMap(MapKind::Position));
    }
    if (m.width, m.height) != dimensions {
        return Some(Inactive::MapSize(MapKind::Position));
    }
    if frame.is_none() {
        return Some(Inactive::MissingFrame);
    }
    if diagonal(m) <= 0.0 {
        return Some(Inactive::EmptyBounds);
    }
    None
}

/// 位置のマップの 16 bit の値からモデルのルートの空間への変換（行優先 3 × 3 と足す値）。形のグラデーションと同じ向き:
/// X_k = Σ_j R[j][k]·(P_j − t_j)、P_j = 最小_j + q_j·(最大_j − 最小_j) / 65535。
fn root_transform(m: &Map<'_>, frame: ModelFrame) -> Option<([f64; 9], [f64; 3])> {
    let r = frame.rotation_matrix();
    let t = frame.position();
    let mut matrix = [0.0; 9];
    let mut offset = [0.0; 3];
    for k in 0..3 {
        for j in 0..3 {
            let rjk = r[j * 3 + k];
            matrix[k * 3 + j] = rjk * (m.bounds_max[j] - m.bounds_min[j]) / 65535.0;
            offset[k] += rjk * (m.bounds_min[j] - t[j]);
        }
    }
    (!matrix.iter().chain(&offset).any(|v| !v.is_finite())).then_some((matrix, offset))
}

/// 位置のマップの画素 (x, y) の、モデルのルートの空間の位置（覆っていない画素・範囲の外は None）。点を 2D のビューで置くときに使う。
pub fn root_position(m: &Map<'_>, frame: ModelFrame, x: u32, y: u32) -> Option<[f64; 3]> {
    if m.kind != MapKind::Position || x >= m.width || y >= m.height {
        return None;
    }
    let i = y as usize * m.width as usize + x as usize;
    if *m.coverage.get(i)? == 0 || m.data.len() < (i + 1) * 3 {
        return None;
    }
    let (matrix, offset) = root_transform(m, frame)?;
    let q = [
        m.data[i * 3] as f64,
        m.data[i * 3 + 1] as f64,
        m.data[i * 3 + 2] as f64,
    ];
    Some(std::array::from_fn(|k| {
        matrix[k * 3] * q[0] + matrix[k * 3 + 1] * q[1] + matrix[k * 3 + 2] * q[2] + offset[k]
    }))
}

fn diagonal(m: &Map<'_>) -> f64 {
    (0..3)
        .map(|k| (m.bounds_max[k] - m.bounds_min[k]).powi(2))
        .sum::<f64>()
        .sqrt()
}

/// 文書の大きさと入力に束ねた点のグラデーション（画素ごとの色を返す）。
pub struct BoundPoints<'a> {
    gradient: &'a PointGradient,
    width: u32,
    height: u32,
    /// モデルの空間: 位置のマップと、16 bit の値からルートの空間への変換（行優先 3 × 3 と足す値）。
    model: Option<(Map<'a>, [f64; 9], [f64; 3])>,
    scale: f64,
}

impl<'a> BoundPoints<'a> {
    /// 束ねる。モデルの空間で位置のマップかルートが使えなければ None（呼び手は塗りつぶしの値を見せ、`inactive_reason` で理由を出す）。
    pub fn bind(
        gradient: &'a PointGradient,
        position: Option<Map<'a>>,
        frame: Option<ModelFrame>,
        dimensions: (u32, u32),
    ) -> Option<Self> {
        if gradient.validate().is_err() {
            return None;
        }
        let (width, height) = dimensions;
        if gradient.space == PointSpace::Uv {
            return Some(BoundPoints {
                gradient,
                width,
                height,
                model: None,
                scale: 1.0,
            });
        }
        if inactive_reason(gradient, position.as_ref(), frame, dimensions).is_some() {
            return None;
        }
        let m = position?;
        if m.kind != MapKind::Position
            || m.data.len() != m.width as usize * m.height as usize * 3
            || m.coverage.len() != m.width as usize * m.height as usize
        {
            return None;
        }
        let (matrix, offset) = root_transform(&m, frame?)?;
        let scale = diagonal(&m);
        Some(BoundPoints {
            gradient,
            width,
            height,
            model: Some((m, matrix, offset)),
            scale,
        })
    }

    /// 画素 (x, y)（左下原点）の色。モデルの空間で位置のマップが覆っていない画素は None（呼び手は値を見せる）。
    pub fn pixel(&self, x: u32, y: u32) -> Option<Rgba8> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let at = match &self.model {
            None => [
                (x as f64 + 0.5) / self.width as f64,
                (y as f64 + 0.5) / self.height as f64,
                0.0,
            ],
            Some((m, matrix, offset)) => {
                let i = y as usize * self.width as usize + x as usize;
                if m.coverage[i] == 0 {
                    return None;
                }
                let q = [
                    m.data[i * 3] as f64,
                    m.data[i * 3 + 1] as f64,
                    m.data[i * 3 + 2] as f64,
                ];
                std::array::from_fn(|k| {
                    matrix[k * 3] * q[0]
                        + matrix[k * 3 + 1] * q[1]
                        + matrix[k * 3 + 2] * q[2]
                        + offset[k]
                })
            }
        };
        Some(self.gradient.color_at(at, self.scale))
    }
}
