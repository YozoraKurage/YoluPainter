//! 面のパスを別のメッシュへ付け直す（C# の SurfacePathRebind）。モデルの差し替えで、パスをメッシュの上に保つ（Substance Painter の
//! 「ストロークの位置をメッシュの上に保つ」に当たる）。
//!
//! 制御点を前のスナップショットの三角形と重心座標から 3D の位置に戻し、新しいスナップショットのいちばん近い面の点（同じ向きの面、
//! 描くテクスチャセットのマテリアルの三角形だけ、許す距離の内側）へ置き直す。1 点でも置けなければ付け直さない（呼び手が画素にして
//! 知らせる）。位置は両方のスナップショットの空間で比べるので、ルートの置き方が同じモデルどうしを前提にする。

use glam::Vec3;

use super::{fingerprint, PathPoint, SurfacePath};
use crate::geometry::unity::{fmax, magnitude};
use crate::geometry::{SurfaceGeometry, SurfaceHit};

/// 許す距離: 前のモデルの境界箱の対角線のこの割合か、パスの筆の半径の大きい方。
pub const REBIND_TOLERANCE_FRACTION: f32 = 0.01;
/// 1 点の近い点の探索で調べる BVH の節の数の上限。
pub const REBIND_MAX_NODE_VISITS: i32 = 200_000;

/// 付け直せなかった理由（点の番号は 0 から）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RebindError {
    /// パスが、前のモデルとは別のモデルで描かれている（三角形・UV・スロットの指紋が違う）。
    OtherModel,
    /// 描くテクスチャセットのマテリアルが、新しいモデルに無い。
    NoMaterial,
    /// 制御点が、前のモデルに無い三角形を指している。
    MissingTriangle { point: usize },
    /// 新しいメッシュで点を探すのに時間がかかりすぎた。
    SearchBudget { point: usize },
    /// 点の近く（許す距離の内側）に、同じマテリアルで向きの合う面が新しいメッシュに無い。
    NoSurface { point: usize, tolerance: f32 },
}

impl std::fmt::Display for RebindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RebindError::OtherModel => f.write_str("パスは別のモデルで描かれています"),
            RebindError::NoMaterial => f.write_str("マテリアルが新しいモデルにありません"),
            RebindError::MissingTriangle { point } => {
                write!(f, "点 {} はモデルに無い三角形を指しています", point + 1)
            }
            RebindError::SearchBudget { point } => {
                write!(f, "点 {} を新しいメッシュで探すのに時間がかかりすぎました", point + 1)
            }
            RebindError::NoSurface { point, tolerance } => write!(
                f,
                "点 {} の近く（{tolerance:.4} 以内）に、同じマテリアルの面が新しいメッシュにありません",
                point + 1
            ),
        }
    }
}

impl std::error::Error for RebindError {}

/// 許す距離（前のモデルの大きさと筆の半径から）。
pub fn rebind_tolerance(from: &SurfaceGeometry, path: &SurfacePath) -> f32 {
    fmax(
        path.brush.0.radius as f32,
        magnitude(from.bounds().size()) * REBIND_TOLERANCE_FRACTION,
    )
}

/// 制御点の今の 3D の位置と面の法線（C# の SurfacePathRenderer.Position）。三角形が無ければ None。
pub fn point_position(geometry: &SurfaceGeometry, p: &PathPoint) -> Option<(Vec3, Vec3)> {
    let t = geometry.triangles().get(p.triangle as usize)?;
    Some((
        t.a * (1.0 - p.u - p.v) as f32 + t.b * p.u as f32 + t.c * p.v as f32,
        t.normal(),
    ))
}

/// レイや最近点の当たりを制御点にする（C# の SurfacePathRenderer.PointOf）。重心座標の丸めで u + v が 1 をわずかに超えるときは、
/// 縮めて 1 に収める。
pub fn point_of(hit: &SurfaceHit, pressure: f64) -> Result<PathPoint, super::Error> {
    let (mut u, mut v) = (hit.barycentric.y as f64, hit.barycentric.z as f64);
    let sum = u + v;
    if sum > 1.0 {
        u /= sum;
        v /= sum;
    }
    PathPoint::new(hit.triangle, u, v, pressure)
}

/// path（from に結び付いたもの）を to の material の組の三角形に置き直す。置けたら新しい指紋のパス（ID・筆・チャンネル・組・筆圧は
/// そのまま）。置けなければ、どの点がなぜ置けなかったか。
pub fn rebind_surface_path(
    path: &SurfacePath,
    from: &SurfaceGeometry,
    to: &SurfaceGeometry,
    material: i32,
) -> Result<SurfacePath, RebindError> {
    if path.model_fingerprint != fingerprint(from) {
        return Err(RebindError::OtherModel);
    }
    if material < 0 {
        return Err(RebindError::NoMaterial);
    }
    let tolerance = rebind_tolerance(from, path);
    let mut points = Vec::with_capacity(path.points.len());
    for (i, p) in path.points.iter().enumerate() {
        let (position, normal) =
            point_position(from, p).ok_or(RebindError::MissingTriangle { point: i })?;
        let hit = to
            .find_closest_point(
                position,
                tolerance,
                normal,
                REBIND_MAX_NODE_VISITS,
                material,
            )
            .map_err(|_| RebindError::SearchBudget { point: i })?
            .ok_or(RebindError::NoSurface {
                point: i,
                tolerance,
            })?;
        points.push(
            point_of(&hit, p.pressure).map_err(|_| RebindError::MissingTriangle { point: i })?,
        );
    }
    Ok(SurfacePath {
        id: path.id,
        channel: path.channel,
        brush: path.brush,
        points,
        model_fingerprint: fingerprint(to),
        material: path.material.clone(),
    })
}
