//! 3D の面の対称（C# の SurfaceSymmetry・SurfaceRadialSymmetry）: ミラーと放射状。
//!
//! ダブの中心（面の上の点）を、モデルの軸に直交する面で映す／軸のまわりに回して、その点にいちばん近い向きの合う面の上の点へ
//! 投げ直し（[`SurfaceGeometry::find_closest_point`]）、同じ半径・硬さでもう 1 つダブを作る。
//!
//! - 面が届く距離に無い写し・別のテクスチャセット（マテリアルの組）の面に落ちた写しは、その写しだけ飛ばす（ほかの写しは塗る）。
//!   同じマテリアルなら別のメッシュでも塗る。写しの側がカメラから見えないときは、`ignore_visibility` でなければ塗らない
//!   （元の側はいつも見え方を確かめる）。
//! - 写しの中心が元の中心（または先に置いた写し）と半径の 1/10000 以内なら、同じダブとして 1 回だけ。軸の上の点・ミラーと
//!   回転が重なる点がこれに当たる。
//! - 全ての写しの画素は、画素ごとに大きい方の覆いで 1 つにする（足さない。1 つのダブの中で三角形が重なったときと同じ）。
//! - 予算は元と写しの合計で数え（放射状・組み合わせ）、写しの側が断られたらダブごと断る（一部の画素だけ塗らない）。
//! - 鏡映を先に、回転を後に当てる（独立の軸を選べるので、最大で 回転の数 × 2）。
//!
//! 回転の係数は glam の単精度の四元数で、Unity の `Quaternion.AngleAxis` とビットまでは一致しない（写しの点は最近点の探索で
//! 面へ投げ直すので、数 ULP のずれは結果に出ない）。

use glam::{Quat, Vec3};

use super::dab::{SurfaceBrushBudget, SurfaceDabResult, SurfaceVisibilityCache};
use super::unity::{dot, fmax, magnitude, sqr_magnitude, Bounds};
use super::{DabRefusal, SurfaceGeometry, SurfaceHit};

/// 対称の面に直交する、モデルのローカルの軸（X なら面は X = 中心）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum SymmetryAxis {
    #[default]
    X,
    Y,
    Z,
}

impl SymmetryAxis {
    pub const ALL: [SymmetryAxis; 3] = [SymmetryAxis::X, SymmetryAxis::Y, SymmetryAxis::Z];

    /// モデルのローカルの軸の向き。
    pub fn direction(self) -> Vec3 {
        match self {
            SymmetryAxis::X => Vec3::X,
            SymmetryAxis::Y => Vec3::Y,
            SymmetryAxis::Z => Vec3::Z,
        }
    }
}

/// 対称の設定を作れなかった理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymmetryError {
    /// 面の法線・回転の軸が 0 か有限でない。
    Axis,
    /// 写しの数が 2〜16 の外。
    Count,
    /// 中心の位置が有限でない。
    Origin,
}

impl std::fmt::Display for SymmetryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SymmetryError::Axis => "対称の面の法線・回転の軸が 0 か有限でありません",
            SymmetryError::Count => "放射状の写しの数は 2〜16 です",
            SymmetryError::Origin => "対称の中心が有限でありません",
        })
    }
}

impl std::error::Error for SymmetryError {}

/// 対称の面: 面の上の点と単位法線、面に沿う 2 本の軸（表示の四角に使う）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MirrorPlane {
    pub point: Vec3,
    pub normal: Vec3,
    pub axis_u: Vec3,
    pub axis_v: Vec3,
}

impl MirrorPlane {
    /// 法線は単位にして持つ（0 か有限でないものは断る）。
    pub fn new(
        point: Vec3,
        normal: Vec3,
        axis_u: Vec3,
        axis_v: Vec3,
    ) -> Result<MirrorPlane, SymmetryError> {
        if !point.is_finite() {
            return Err(SymmetryError::Origin);
        }
        if !normal.is_finite() || sqr_magnitude(normal) < 1e-20 {
            return Err(SymmetryError::Axis);
        }
        Ok(MirrorPlane {
            point,
            normal: super::unity::normalized(normal),
            axis_u,
            axis_v,
        })
    }

    /// モデルのルートのローカルの軸に直交する面を、ルートの位置から法線の向きに offset（モデルの単位）ずらして置く。ルートの大きさ
    /// （スケール）は面の向きを変えないので使わない。
    pub fn from_model(
        root_position: Vec3,
        root_rotation: Quat,
        axis: SymmetryAxis,
        offset: f32,
    ) -> MirrorPlane {
        let (n, u, v) = match axis {
            SymmetryAxis::Y => (Vec3::Y, Vec3::Z, Vec3::X),
            SymmetryAxis::Z => (Vec3::Z, Vec3::X, Vec3::Y),
            SymmetryAxis::X => (Vec3::X, Vec3::Y, Vec3::Z),
        };
        let (n, u, v) = (root_rotation * n, root_rotation * u, root_rotation * v);
        MirrorPlane {
            point: root_position + n * offset,
            normal: n,
            axis_u: u,
            axis_v: v,
        }
    }

    /// 面からの符号つきの距離（法線の側が正）。
    pub fn signed_distance(&self, p: Vec3) -> f32 {
        dot(p - self.point, self.normal)
    }

    pub fn reflect(&self, p: Vec3) -> Vec3 {
        p - self.normal * (2.0 * self.signed_distance(p))
    }

    pub fn reflect_direction(&self, d: Vec3) -> Vec3 {
        d - self.normal * (2.0 * dot(d, self.normal))
    }
}

/// 軸のまわりの放射状の対称（count 個。2〜16）。回転はモデルの単位のまま（スケールによらない）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadialSymmetry {
    pub origin: Vec3,
    pub axis: Vec3,
    pub count: u32,
}

impl RadialSymmetry {
    pub fn new(origin: Vec3, axis: Vec3, count: u32) -> Result<RadialSymmetry, SymmetryError> {
        if !(2..=16).contains(&count) {
            return Err(SymmetryError::Count);
        }
        if !origin.is_finite() {
            return Err(SymmetryError::Origin);
        }
        if !axis.is_finite() || sqr_magnitude(axis) < 1e-20 {
            return Err(SymmetryError::Axis);
        }
        Ok(RadialSymmetry {
            origin,
            axis: super::unity::normalized(axis),
            count,
        })
    }

    /// モデルのルート（位置と回り）のローカルの軸のまわり。
    pub fn from_model(
        origin: Vec3,
        rotation: Quat,
        axis: SymmetryAxis,
        count: u32,
    ) -> Result<RadialSymmetry, SymmetryError> {
        RadialSymmetry::new(origin, rotation * axis.direction(), count)
    }

    pub(crate) fn rotation(&self, copy: u32) -> Quat {
        // Unity の Quaternion.AngleAxis(360f * copy / Count, Axis)（度）。
        let degrees = 360.0f32 * copy as f32 / self.count as f32;
        Quat::from_axis_angle(self.axis, degrees * (std::f32::consts::PI / 180.0))
    }

    pub fn rotate_point(&self, point: Vec3, copy: u32) -> Vec3 {
        self.origin + self.rotation(copy) * (point - self.origin)
    }

    pub fn rotate_direction(&self, direction: Vec3, copy: u32) -> Vec3 {
        self.rotation(copy) * direction
    }
}

/// 写した側のダブがどうなったか（複数あるときは、あとの写しのもので上書きする。塗った写しだけなら Painted）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MirrorOutcome {
    /// 写しも塗った（覆いは元の側と画素ごとの大きいほうで合わせた）。
    Painted,
    /// 写しが元と同じ位置（軸の上など）なので元の側だけ。
    OnPlane,
    /// 写しの点から届く距離に、向きの合う面が無い（モデルがそこで対称でない）。
    NoSurface,
    /// 写しのいちばん近い面が、今のテクスチャセットとは別のマテリアル。
    OtherSlot,
    /// 見え方を無視しない設定で、写しの側の画素がカメラから見えない。
    Hidden,
    /// 2D のキャンバスのストロークの 3D の写しで、元か写し先の三角形の UV が潰れている・テクセルの細かさが 64 倍より違う
    /// （UV の写像を作れない。`uv_symmetry`）。
    UvMismatch,
}

/// ダブを断ったのが、どの側か（予算の断りの知らせに使う）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DabSide {
    Original,
    Mirror,
    Copy,
    /// 写しの点を面へ投げ直す探索が、BVH の仕事量の上限を超えた（断る理由は `DabRefusal::BvhBudget`）。
    Search,
}

/// 写しの点から面を探す距離の上限: ブラシの半径（それより遠い面には写したブラシの球が届かない）。ただし小さい筆でも、書き出しの
/// 丸めほどの左右のずれ（モデルの対角線の 1/1000）は許す。
pub fn search_distance(radius_world: f32, bounds: &Bounds) -> f32 {
    fmax(radius_world, magnitude(bounds.size()) * 1e-3)
}

/// 中心がこれより面に近ければ（半径に対する割合）、写しても同じダブとみなす。
pub const ON_PLANE_FRACTION: f32 = 1e-4;
/// 写しの点を探す問い合わせで BVH の節点を見る数の上限（超えたらダブを断り、ストロークを取り消す）。
pub const MAX_CLOSEST_POINT_NODE_VISITS: i32 = 1 << 20;

/// 2 つのダブを画素ごとに大きいほうの覆いで合わせる（どちらも左下からの行の順。結果も同じ順で、同じ画素は 1 つ）。
pub fn union_dabs(a: SurfaceDabResult, b: SurfaceDabResult, width: i32) -> SurfaceDabResult {
    let mut result = SurfaceDabResult {
        candidate_pixels: a.candidate_pixels + b.candidate_pixels,
        visibility_rays: a.visibility_rays + b.visibility_rays,
        ray_triangle_tests: a.ray_triangle_tests + b.ray_triangle_tests,
        visited_triangles: a.visited_triangles + b.visited_triangles,
        ray_node_visits: a.ray_node_visits + b.ray_node_visits,
        refusal: a.refusal.or(b.refusal),
        pixels: Vec::with_capacity(a.pixels.len() + b.pixels.len()),
    };
    let key = |p: &super::SurfacePixel| p.y as i64 * width as i64 + p.x as i64;
    let (mut i, mut j) = (0, 0);
    while i < a.pixels.len() || j < b.pixels.len() {
        let ka = a.pixels.get(i).map_or(i64::MAX, key);
        let kb = b.pixels.get(j).map_or(i64::MAX, key);
        if ka < kb {
            result.pixels.push(a.pixels[i]);
            i += 1;
        } else if kb < ka {
            result.pixels.push(b.pixels[j]);
            j += 1;
        } else {
            let (p, q) = (a.pixels[i], b.pixels[j]);
            result
                .pixels
                .push(if p.coverage >= q.coverage { p } else { q });
            i += 1;
            j += 1;
        }
    }
    result
}

/// ミラーだけのダブ（C# の SymmetricSurfaceDab）。
#[derive(Clone, Debug)]
pub struct SymmetricSurfaceDab {
    /// 塗る画素（元の側と写した側を画素ごとに大きいほうの覆いで合わせ、左下からの行の順）。予算を超えたら空で、`refusal` が付く。
    pub result: SurfaceDabResult,
    pub original: SurfaceDabResult,
    /// 写した側のダブ（作らなかったら None）。
    pub mirror: Option<SurfaceDabResult>,
    /// 写した点を面に落とした当たり（落とせたときだけ）。
    pub mirror_hit: Option<SurfaceHit>,
    pub outcome: MirrorOutcome,
    /// `result` を断ったのがどの側か。
    pub refused_side: Option<DabSide>,
}

/// ミラーと放射状のダブ（C# の ExpandedSurfaceDab）。
#[derive(Clone, Debug)]
pub struct ExpandedSurfaceDab {
    pub result: SurfaceDabResult,
    /// 写しの当たり（塗った・見えずに飛ばしたもの。面が無い・別のマテリアル・重複は入らない）。
    pub copies: Vec<SurfaceHit>,
    pub outcome: MirrorOutcome,
    pub refused_side: Option<DabSide>,
}

/// 写しの点の探索の結果（1 つの写し）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CopyHit {
    /// 元や先に置いた写しと同じ位置（飛ばす）。
    Duplicate,
    /// 面へ投げ直す探索が上限を超えた（ダブごと断る）。
    BudgetExceeded,
    /// 届く距離に向きの合う面が無い（その写しだけ飛ばす）。
    NoSurface,
    /// 別のマテリアルの面に落ちた（その写しだけ飛ばす）。
    OtherMaterial,
    Found(SurfaceHit),
}

/// 写しの数（元を含む）。ミラーがあれば 2 倍。
pub fn copy_count(mirror: Option<&MirrorPlane>, radial: Option<&RadialSymmetry>) -> usize {
    radial.map_or(1, |r| r.count as usize) * if mirror.is_some() { 2 } else { 1 }
}

/// 写し（copy 番目の回転・reflected ならミラーの後）の点を面へ投げ直す。positions は元と先に置いた写しの位置（重複を省く。この
/// 写しの位置を足す）。順番は C# と同じ: 鏡映を先に、回転を後に。copy 0 で鏡映なしは元そのもので、呼ばない。
#[allow(clippy::too_many_arguments)]
pub fn find_copy(
    geometry: &SurfaceGeometry,
    hit: &SurfaceHit,
    mirror: Option<&MirrorPlane>,
    radial: Option<&RadialSymmetry>,
    copy: u32,
    reflected: bool,
    radius: f32,
    positions: &mut Vec<Vec3>,
) -> CopyHit {
    let (mut p, mut n) = match (reflected, mirror) {
        (true, Some(m)) => (m.reflect(hit.position), m.reflect_direction(hit.normal)),
        _ => (hit.position, hit.normal),
    };
    if let Some(r) = radial {
        p = r.rotate_point(p, copy);
        n = r.rotate_direction(n, copy);
    }
    if positions
        .iter()
        .any(|old| magnitude(*old - p) <= radius * ON_PLANE_FRACTION)
    {
        return CopyHit::Duplicate;
    }
    positions.push(p);
    match geometry.find_closest_point(
        p,
        search_distance(radius, &geometry.bounds()),
        n,
        MAX_CLOSEST_POINT_NODE_VISITS,
        -1,
    ) {
        Err(_) => CopyHit::BudgetExceeded,
        Ok(None) => CopyHit::NoSurface,
        // 同じマテリアル（テクスチャセット）なら別のメッシュでも塗る
        Ok(Some(found)) if found.material != hit.material => CopyHit::OtherMaterial,
        Ok(Some(found)) => CopyHit::Found(found),
    }
}

/// 元の点から見た写しの当たり全部（カーソルの表示用。面に落ちなかった写し・重複・別のマテリアルは入らない）。
pub fn copy_hits(
    geometry: &SurfaceGeometry,
    hit: &SurfaceHit,
    mirror: Option<&MirrorPlane>,
    radial: Option<&RadialSymmetry>,
    radius: f32,
) -> Vec<SurfaceHit> {
    let mut positions = vec![hit.position];
    let mut found = Vec::new();
    for copy in 0..radial.map_or(1, |r| r.count) {
        for reflected in [false, true] {
            if (copy == 0 && !reflected) || (reflected && mirror.is_none()) {
                continue;
            }
            if let CopyHit::Found(h) = find_copy(
                geometry,
                hit,
                mirror,
                radial,
                copy,
                reflected,
                radius,
                &mut positions,
            ) {
                found.push(h);
            }
        }
    }
    found
}

/// 元のダブと、ミラーした側のダブを作って合わせる（C# の SurfaceSymmetry.Build）。元の側が断られた・作れなかったら、写した側は
/// 作らない。元と写しは同じ予算を別々に使う（放射状・組み合わせは [`build_expanded`] で合計）。
#[allow(clippy::too_many_arguments)]
pub fn build_mirrored(
    geometry: &SurfaceGeometry,
    hit: &SurfaceHit,
    plane: &MirrorPlane,
    radius_world: f32,
    width: i32,
    height: i32,
    camera: Vec3,
    hardness: f32,
    budget: &SurfaceBrushBudget,
    mut cache: Option<&mut SurfaceVisibilityCache>,
    ignore_visibility: bool,
) -> SymmetricSurfaceDab {
    let original = geometry.build_surface_dabs(
        hit,
        radius_world,
        width,
        height,
        camera,
        hardness,
        budget,
        cache.as_deref_mut(),
        false,
    );
    let mut dab = SymmetricSurfaceDab {
        result: original.clone(),
        original,
        mirror: None,
        mirror_hit: None,
        outcome: MirrorOutcome::OnPlane,
        refused_side: None,
    };
    if dab.original.refusal.is_some() {
        dab.refused_side = Some(DabSide::Original);
        return dab;
    }
    let mirrored = plane.reflect(hit.position);
    if magnitude(mirrored - hit.position) <= radius_world * ON_PLANE_FRACTION {
        return dab;
    }
    let found = geometry.find_closest_point(
        mirrored,
        search_distance(radius_world, &geometry.bounds()),
        plane.reflect_direction(hit.normal),
        MAX_CLOSEST_POINT_NODE_VISITS,
        -1,
    );
    let mirror_hit = match found {
        Err(_) => {
            dab.result = SurfaceDabResult::default().reject(DabRefusal::BvhBudget);
            dab.refused_side = Some(DabSide::Search);
            return dab;
        }
        Ok(None) => {
            dab.outcome = MirrorOutcome::NoSurface;
            return dab;
        }
        Ok(Some(h)) => h,
    };
    dab.mirror_hit = Some(mirror_hit);
    // 同じマテリアル（テクスチャセット）なら別のメッシュでも描く
    if mirror_hit.material != hit.material {
        dab.outcome = MirrorOutcome::OtherSlot;
        return dab;
    }
    let mirror = geometry.build_surface_dabs(
        &mirror_hit,
        radius_world,
        width,
        height,
        camera,
        hardness,
        budget,
        cache,
        ignore_visibility,
    );
    if let Some(why) = mirror.refusal {
        dab.result = SurfaceDabResult::default().reject(why);
        dab.mirror = Some(mirror);
        dab.refused_side = Some(DabSide::Mirror);
        return dab;
    }
    dab.outcome = if mirror.pixels.is_empty() && !dab.original.pixels.is_empty() {
        MirrorOutcome::Hidden
    } else {
        MirrorOutcome::Painted
    };
    dab.result = union_dabs(dab.original.clone(), mirror.clone(), width);
    dab.mirror = Some(mirror);
    dab
}

/// 回転 N 個と、各回転の鏡映を 1 つのダブへ合併する（C# の SurfaceRadialSymmetry.Build）。予算は元と写しの合計で、写しが
/// 断られたら全部断る。mirror も radial も None なら、元のダブそのもの。
#[allow(clippy::too_many_arguments)]
pub fn build_expanded(
    geometry: &SurfaceGeometry,
    hit: &SurfaceHit,
    mirror: Option<&MirrorPlane>,
    radial: Option<&RadialSymmetry>,
    ignore_visibility: bool,
    radius: f32,
    width: i32,
    height: i32,
    camera: Vec3,
    hardness: f32,
    budget: &SurfaceBrushBudget,
    mut cache: Option<&mut SurfaceVisibilityCache>,
) -> ExpandedSurfaceDab {
    let mut result = ExpandedSurfaceDab {
        result: geometry.build_surface_dabs(
            hit,
            radius,
            width,
            height,
            camera,
            hardness,
            budget,
            cache.as_deref_mut(),
            false,
        ),
        copies: Vec::new(),
        outcome: MirrorOutcome::OnPlane,
        refused_side: None,
    };
    if result.result.refusal.is_some() {
        result.refused_side = Some(DabSide::Original);
        return result;
    }
    if result.result.pixels.is_empty() {
        return result;
    }
    let mut positions = vec![hit.position];
    for copy in 0..radial.map_or(1, |r| r.count) {
        for reflected in [false, true] {
            if (copy == 0 && !reflected) || (reflected && mirror.is_none()) {
                continue;
            }
            let found = match find_copy(
                geometry,
                hit,
                mirror,
                radial,
                copy,
                reflected,
                radius,
                &mut positions,
            ) {
                CopyHit::Duplicate => continue,
                CopyHit::BudgetExceeded => {
                    result.result =
                        std::mem::take(&mut result.result).reject(DabRefusal::BvhBudget);
                    result.refused_side = Some(DabSide::Copy);
                    return result;
                }
                CopyHit::NoSurface => {
                    result.outcome = MirrorOutcome::NoSurface;
                    continue;
                }
                CopyHit::OtherMaterial => {
                    result.outcome = MirrorOutcome::OtherSlot;
                    continue;
                }
                CopyHit::Found(h) => h,
            };
            result.copies.push(found);
            let used = std::mem::take(&mut result.result);
            let remaining = SurfaceBrushBudget {
                max_triangles: budget.max_triangles - used.visited_triangles,
                max_candidate_pixels: budget.max_candidate_pixels - used.candidate_pixels,
                max_visibility_rays: budget.max_visibility_rays - used.visibility_rays,
                max_ray_triangle_tests: budget.max_ray_triangle_tests - used.ray_triangle_tests,
                max_ray_node_visits: budget.max_ray_node_visits
                    - used.ray_node_visits.min(i32::MAX as i64) as i32,
            };
            let dab = geometry.build_surface_dabs(
                &found,
                radius,
                width,
                height,
                camera,
                hardness,
                &remaining,
                cache.as_deref_mut(),
                ignore_visibility,
            );
            if let Some(why) = dab.refusal {
                result.result = used.reject(why);
                result.refused_side = Some(DabSide::Copy);
                return result;
            }
            if dab.pixels.is_empty() {
                result.outcome = MirrorOutcome::Hidden;
            } else if result.outcome == MirrorOutcome::OnPlane {
                result.outcome = MirrorOutcome::Painted;
            }
            result.result = union_dabs(used, dab, width);
        }
    }
    result
}
