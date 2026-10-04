//! スキン（骨の階層・バインドポーズ・頂点ごとのウェイト・BlendShape）と、ポーズを付けた形の計算（線形ブレンドのスキニング。CPU、rayon）。
//!
//! - 座標は `geometry` と同じ Unity の左手系（Y が上）。骨のローカルの変換は Unity の Transform と同じ T・R・S（行列は T × R × S、
//!   ワールドは 親のワールド × ローカル）。
//! - メッシュの位置はバインドのときのモデルの空間（`RigMesh::mesh`）。関節 j の行列は 骨のワールド × `bind_inverse`。頂点の位置は
//!   BlendShape の差分を足してから、ウェイトで混ぜた行列を掛ける（ufbx・Unity と同じ順）。法線は混ぜた行列の 3 × 3 を掛けて長さを 1 に。
//! - ウェイトは頂点ごとの CSR（本数に上限を置かない形。予算の `max_influences_per_vertex` まで）。作るときに頂点ごとの和で割る。
//!   和が 0 の頂点（ウェイトの無い頂点）は `Skin::fallback` の関節に固く付いて動く（無ければバインドの位置のまま）。
//! - BlendShape の重みは Unity と同じ 0〜100 の目盛り。中間のフレームは重みの間で線形に混ぜ、範囲の外は端の区間を延ばす。
//! - ポーズを付けても、描く先は UV のテクスチャ（画素の正本）で、ここは形だけを変える。

mod deform;
mod demo;
mod influence;
#[cfg(test)]
mod tests;

pub use demo::{demo_figure, FigureDetail};
pub use influence::BonePathError;

use glam::{Mat3, Mat4, Quat, Vec3};

use crate::geometry::ModelMesh;

/// 骨のローカルの変換（Unity の localPosition・localRotation・localScale）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneTransform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl BoneTransform {
    pub const IDENTITY: BoneTransform = BoneTransform {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// T × R × S。
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    fn is_finite(&self) -> bool {
        self.translation.is_finite() && self.rotation.is_finite() && self.scale.is_finite()
    }
}

impl Default for BoneTransform {
    fn default() -> Self {
        BoneTransform::IDENTITY
    }
}

/// 骨（FBX のノード 1 つ。メッシュの付いたノードや空のノードも骨として並べる）。
#[derive(Clone, Debug, PartialEq)]
pub struct Bone {
    pub name: String,
    /// 親の番号（自分より前）。None なら根。
    pub parent: Option<u32>,
    /// ファイルにあったときのローカルの変換（「ポーズを戻す」の戻り先）。
    pub rest: BoneTransform,
}

/// 関節（スキンが使う骨と、バインドのときのモデルの空間 → 骨の空間の行列）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Joint {
    pub bone: u32,
    pub bind_inverse: Mat4,
}

/// 頂点の影響 1 つ。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Influence {
    /// `Skin::joints` の番号。
    pub joint: u32,
    pub weight: f32,
}

/// メッシュのスキン（頂点ごとのウェイトの CSR）。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Skin {
    pub joints: Vec<Joint>,
    /// 頂点 i の影響は `influences[offsets[i]..offsets[i + 1]]`（長さは頂点の数 + 1）。
    pub offsets: Vec<u32>,
    pub influences: Vec<Influence>,
    /// ウェイトの無い頂点が付いて動く関節（None ならバインドの位置のまま）。
    pub fallback: Option<u32>,
}

impl Skin {
    /// 全部の頂点が 1 つの骨に固く付いて動く（スキンの無いメッシュ。影響の並びは持たない）。
    pub fn rigid(bone: u32, bind_inverse: Mat4, vertex_count: usize) -> Skin {
        Skin {
            joints: vec![Joint { bone, bind_inverse }],
            offsets: vec![0; vertex_count + 1],
            influences: Vec::new(),
            fallback: Some(0),
        }
    }

    /// 頂点 i の影響。
    pub fn influences_of(&self, vertex: usize) -> &[Influence] {
        &self.influences[self.offsets[vertex] as usize..self.offsets[vertex + 1] as usize]
    }
}

/// BlendShape のフレーム 1 つ（疎な差分。位置・法線はバインドのときのモデルの空間）。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct BlendFrame {
    /// このフレームが全部効く重み（0 より大きい。Unity の目盛りで 100 が普通）。
    pub weight: f32,
    pub vertices: Vec<u32>,
    pub positions: Vec<Vec3>,
    /// 空か、vertices と同じ数。
    pub normals: Vec<Vec3>,
}

/// BlendShape 1 つ（フレームは重みの小さい順）。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct BlendShape {
    pub name: String,
    pub frames: Vec<BlendFrame>,
}

impl BlendShape {
    /// 重み w でのフレームごとの掛け目（Unity と同じ: 最初のフレームまでは 0 からの線形、フレームの間は線形、外は端の区間を延ばす）。
    pub fn frame_factors(&self, w: f32) -> [(usize, f32); 2] {
        let none = [(0, 0.0), (0, 0.0)];
        let n = self.frames.len();
        if n == 0 || w == 0.0 {
            return none;
        }
        let w0 = self.frames[0].weight;
        if n == 1 || w <= w0 {
            return [(0, w / w0), (0, 0.0)];
        }
        let mut i = 0;
        while i + 2 < n && w > self.frames[i + 1].weight {
            i += 1;
        }
        let (a, b) = (self.frames[i].weight, self.frames[i + 1].weight);
        let t = (w - a) / (b - a);
        [(i, 1.0 - t), (i + 1, t)]
    }
}

/// ポーズを付けられるメッシュ（レンダラー 1 つ）。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RigMesh {
    /// バインドのときの形（位置・法線はモデルの空間。UV・サブメッシュはポーズで変わらない）。
    pub mesh: ModelMesh,
    pub skin: Skin,
    pub blend_shapes: Vec<BlendShape>,
}

/// 読み込める大きさの上限（壊れた・大きすぎるファイルで止まらないように。どれかを超えたら作らない）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RigBudget {
    pub max_bones: usize,
    pub max_meshes: usize,
    /// 全部のメッシュの頂点の和。
    pub max_vertices: usize,
    /// 全部のメッシュの三角形の和。
    pub max_triangles: usize,
    /// 全部の影響の和。
    pub max_influences: usize,
    pub max_influences_per_vertex: usize,
    /// 全部の BlendShape の数。
    pub max_blend_shapes: usize,
    /// 全部の BlendShape のフレームの差分の数の和。
    pub max_blend_offsets: usize,
}

impl Default for RigBudget {
    fn default() -> Self {
        RigBudget {
            max_bones: 16_384,
            max_meshes: 1_024,
            max_vertices: 4_000_000,
            max_triangles: 4_000_000,
            max_influences: 32_000_000,
            max_influences_per_vertex: 256,
            max_blend_shapes: 4_096,
            max_blend_offsets: 16_000_000,
        }
    }
}

/// スキンを作れない・ポーズを当てられない理由。
#[derive(Clone, Debug, PartialEq)]
pub enum RigError {
    /// 予算を超えた（何の数・値・上限）。
    TooLarge {
        what: &'static str,
        value: usize,
        limit: usize,
    },
    /// 骨の親が自分より後ろか範囲外。
    BadParent { bone: usize },
    /// メッシュの添字・頂点の数が合わない。
    BadMesh { mesh: usize },
    /// スキンの関節・影響・並びが合わない。
    BadSkin { mesh: usize },
    /// BlendShape のフレームが合わない（重みが正で増えていく・頂点が範囲内・数が揃う）。
    BadBlendShape { mesh: usize, shape: usize },
    /// 有限でない値（変換・ウェイト・位置・差分）。
    NonFinite { what: &'static str },
    /// ポーズの骨・BlendShape の数がスキンと違う。
    PoseMismatch,
}

impl std::fmt::Display for RigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RigError::TooLarge { what, value, limit } => {
                write!(f, "{what}が多すぎます（{value}、上限 {limit}）")
            }
            RigError::BadParent { bone } => write!(f, "ボーン {bone} の親が正しくありません"),
            RigError::BadMesh { mesh } => {
                write!(f, "メッシュ {mesh} の添字か頂点の数が正しくありません")
            }
            RigError::BadSkin { mesh } => write!(f, "メッシュ {mesh} のスキンが正しくありません"),
            RigError::BadBlendShape { mesh, shape } => {
                write!(
                    f,
                    "メッシュ {mesh} の BlendShape {shape} が正しくありません"
                )
            }
            RigError::NonFinite { what } => write!(f, "{what}に有限でない値があります"),
            RigError::PoseMismatch => f.write_str("ポーズがモデルと合いません"),
        }
    }
}

impl std::error::Error for RigError {}

/// ポーズ（骨ごとのローカルの変換と、メッシュごと・BlendShape ごとの重み）。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Pose {
    pub locals: Vec<BoneTransform>,
    /// `blend_weights[メッシュ][BlendShape]`（0〜100 の目盛り）。
    pub blend_weights: Vec<Vec<f32>>,
}

/// 骨・メッシュ・マテリアルの組（読んだ後は変えない。ポーズは `Pose` で別に持つ）。
#[derive(Clone, Debug)]
pub struct Rig {
    name: String,
    bones: Vec<Bone>,
    children: Vec<Vec<u32>>,
    meshes: Vec<RigMesh>,
    materials: Vec<String>,
    rest_weights: Vec<Vec<f32>>,
    /// どれかの頂点を動かす骨か（スキンの関節・固く付いたメッシュの骨）。
    deforming: Vec<bool>,
}

impl Rig {
    /// 確かめてから作る。ウェイトは頂点ごとの和で割り、0 以下の影響は捨てる（和が 0 の頂点は fallback へ）。
    /// rest_weights は BlendShape の初めの重み（空ならすべて 0）。
    pub fn new(
        name: &str,
        bones: Vec<Bone>,
        mut meshes: Vec<RigMesh>,
        materials: Vec<String>,
        rest_weights: Vec<Vec<f32>>,
        budget: &RigBudget,
    ) -> Result<Rig, RigError> {
        let limit = |what, value: usize, limit: usize| {
            if value > limit {
                Err(RigError::TooLarge { what, value, limit })
            } else {
                Ok(())
            }
        };
        limit("ボーン", bones.len(), budget.max_bones)?;
        limit("メッシュ", meshes.len(), budget.max_meshes)?;
        let vertices: usize = meshes.iter().map(|m| m.mesh.positions.len()).sum();
        limit("頂点", vertices, budget.max_vertices)?;
        let triangles: usize = meshes.iter().map(|m| m.mesh.triangle_count()).sum();
        limit("三角形", triangles, budget.max_triangles)?;
        let influences: usize = meshes.iter().map(|m| m.skin.influences.len()).sum();
        limit("ウェイト", influences, budget.max_influences)?;
        let shapes: usize = meshes.iter().map(|m| m.blend_shapes.len()).sum();
        limit("BlendShape", shapes, budget.max_blend_shapes)?;
        let offsets: usize = meshes
            .iter()
            .flat_map(|m| &m.blend_shapes)
            .flat_map(|s| &s.frames)
            .map(|f| f.vertices.len())
            .sum();
        limit("BlendShape の差分", offsets, budget.max_blend_offsets)?;

        let mut children = vec![Vec::new(); bones.len()];
        for (i, b) in bones.iter().enumerate() {
            if let Some(p) = b.parent {
                if p as usize >= i {
                    return Err(RigError::BadParent { bone: i });
                }
                children[p as usize].push(i as u32);
            }
            if !b.rest.is_finite() {
                return Err(RigError::NonFinite {
                    what: "ボーンの変換"
                });
            }
        }
        let mut deforming = vec![false; bones.len()];
        for (mi, m) in meshes.iter_mut().enumerate() {
            let n = m.mesh.positions.len();
            if !m.mesh.is_valid() || n > u32::MAX as usize - 1 {
                return Err(RigError::BadMesh { mesh: mi });
            }
            if m.mesh.positions.iter().any(|p| !p.is_finite())
                || m.mesh.normals.iter().any(|p| !p.is_finite())
                || m.mesh.uvs.iter().any(|p| !p.is_finite())
            {
                return Err(RigError::NonFinite {
                    what: "メッシュの位置・法線・UV",
                });
            }
            validate_skin(mi, &mut m.skin, n, bones.len(), budget)?;
            for j in &m.skin.joints {
                deforming[j.bone as usize] = true;
            }
            for (si, s) in m.blend_shapes.iter().enumerate() {
                validate_shape(mi, si, s, n)?;
            }
        }
        let rest_weights = if rest_weights.is_empty() {
            meshes
                .iter()
                .map(|m| vec![0.0; m.blend_shapes.len()])
                .collect()
        } else {
            rest_weights
        };
        if !weights_match(&meshes, &rest_weights) {
            return Err(RigError::PoseMismatch);
        }
        Ok(Rig {
            name: name.to_string(),
            bones,
            children,
            meshes,
            materials,
            rest_weights,
            deforming,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn bones(&self) -> &[Bone] {
        &self.bones
    }
    /// 骨 i の子（並びはファイルの順）。
    pub fn children(&self, bone: usize) -> &[u32] {
        &self.children[bone]
    }
    /// 根の骨（親の無い骨）。
    pub fn roots(&self) -> impl Iterator<Item = usize> + '_ {
        self.bones
            .iter()
            .enumerate()
            .filter(|(_, b)| b.parent.is_none())
            .map(|(i, _)| i)
    }
    /// 骨 i がどれかの頂点を動かすか（スキンの関節か、メッシュが固く付いた骨）。
    pub fn is_deforming(&self, bone: usize) -> bool {
        self.deforming[bone]
    }
    pub fn meshes(&self) -> &[RigMesh] {
        &self.meshes
    }
    pub fn materials(&self) -> &[String] {
        &self.materials
    }
    pub fn triangle_count(&self) -> usize {
        self.meshes.iter().map(|m| m.mesh.triangle_count()).sum()
    }
    pub fn vertex_count(&self) -> usize {
        self.meshes.iter().map(|m| m.mesh.positions.len()).sum()
    }
    pub fn blend_shape_count(&self) -> usize {
        self.meshes.iter().map(|m| m.blend_shapes.len()).sum()
    }

    /// ファイルにあったときのポーズ。
    pub fn rest_pose(&self) -> Pose {
        Pose {
            locals: self.bones.iter().map(|b| b.rest).collect(),
            blend_weights: self.rest_weights.clone(),
        }
    }

    /// ポーズの形がこのスキンに合うか（骨の数・BlendShape の数・有限）。
    pub fn check_pose(&self, pose: &Pose) -> Result<(), RigError> {
        if pose.locals.len() != self.bones.len()
            || !weights_match(&self.meshes, &pose.blend_weights)
        {
            return Err(RigError::PoseMismatch);
        }
        if pose.locals.iter().any(|t| !t.is_finite())
            || pose.blend_weights.iter().flatten().any(|w| !w.is_finite())
        {
            return Err(RigError::NonFinite { what: "ポーズ" });
        }
        Ok(())
    }

    /// 骨ごとのワールドの行列（親が先に並ぶので前から 1 回で求まる）。
    pub fn world_matrices(&self, pose: &Pose) -> Result<Vec<Mat4>, RigError> {
        self.check_pose(pose)?;
        let mut world: Vec<Mat4> = Vec::with_capacity(self.bones.len());
        for (i, b) in self.bones.iter().enumerate() {
            let local = pose.locals[i].matrix();
            world.push(match b.parent {
                Some(p) => world[p as usize] * local,
                None => local,
            });
        }
        Ok(world)
    }

    /// ポーズを付けたメッシュ（位置・法線だけが変わる。UV・サブメッシュ・名前は同じ）。メッシュの間も頂点の間も並列。
    pub fn deform(&self, pose: &Pose) -> Result<Vec<ModelMesh>, RigError> {
        let world = self.world_matrices(pose)?;
        Ok(deform::deform_meshes(self, &world, &pose.blend_weights))
    }

    /// 当たった三角形（`model_triangles` の通し番号。メッシュの順 × サブメッシュの順）と重心座標から、そこをいちばん動かす骨
    /// （3 頂点のうち重心座標のいちばん大きい頂点の、ウェイトのいちばん大きい関節の骨。ウェイトの無い頂点は fallback の骨）。
    pub fn bone_at_triangle(&self, triangle: u32, barycentric: Vec3) -> Option<usize> {
        let mut t = triangle as usize;
        for m in &self.meshes {
            let count = m.mesh.triangle_count();
            if t >= count {
                t -= count;
                continue;
            }
            for s in &m.mesh.submeshes {
                let n = s.indices.len() / 3;
                if t >= n {
                    t -= n;
                    continue;
                }
                let corner = if barycentric.x >= barycentric.y && barycentric.x >= barycentric.z {
                    0
                } else if barycentric.y >= barycentric.z {
                    1
                } else {
                    2
                };
                let v = s.indices[t * 3 + corner] as usize;
                let joint = match m
                    .skin
                    .influences_of(v)
                    .iter()
                    .max_by(|a, b| a.weight.total_cmp(&b.weight))
                {
                    Some(i) => i.joint,
                    None => m.skin.fallback?,
                };
                return Some(m.skin.joints[joint as usize].bone as usize);
            }
            return None;
        }
        None
    }

    /// 骨のワールドの原点（ギズモの中心・骨の線）。
    pub fn bone_origin(world: &[Mat4], bone: usize) -> Vec3 {
        world[bone].w_axis.truncate()
    }

    /// 骨のワールドの回転（スケールを除いた向き。親のスケールが一様でなければ近い回転）。
    pub fn world_rotation(world: &[Mat4], bone: usize) -> Quat {
        rotation_of(world[bone])
    }

    /// 骨 i をワールドの軸 axis のまわりに angle（ラジアン）回す（Unity の Transform.Rotate(axis, angle, Space.World) と同じ考え方:
    /// 親のワールドの回転で軸をローカルへ写す）。回転は長さ 1 に直す。
    pub fn rotate_bone_world(
        &self,
        pose: &mut Pose,
        world: &[Mat4],
        bone: usize,
        axis: Vec3,
        angle: f32,
    ) {
        let axis = axis.normalize_or_zero();
        if axis == Vec3::ZERO || !angle.is_finite() || bone >= pose.locals.len() {
            return;
        }
        let parent = match self.bones[bone].parent {
            Some(p) => rotation_of(world[p as usize]),
            None => Quat::IDENTITY,
        };
        let q = Quat::from_axis_angle(parent.inverse() * axis, angle);
        let local = &mut pose.locals[bone];
        local.rotation = (q * local.rotation).normalize();
    }
}

/// 行列の回転の部分（列の長さで割ってから四元数へ。負の行列式なら X を反転した回転）。
fn rotation_of(m: Mat4) -> Quat {
    let mut x = m.x_axis.truncate().normalize_or_zero();
    let y = m.y_axis.truncate().normalize_or_zero();
    let z = m.z_axis.truncate().normalize_or_zero();
    if x.cross(y).dot(z) < 0.0 {
        x = -x;
    }
    let q = Quat::from_mat3(&Mat3::from_cols(x, y, z));
    if q.is_finite() && q.length_squared() > 0.0 {
        q.normalize()
    } else {
        Quat::IDENTITY
    }
}

fn weights_match(meshes: &[RigMesh], weights: &[Vec<f32>]) -> bool {
    weights.len() == meshes.len()
        && meshes
            .iter()
            .zip(weights)
            .all(|(m, w)| w.len() == m.blend_shapes.len())
}

fn validate_skin(
    mesh: usize,
    skin: &mut Skin,
    vertices: usize,
    bones: usize,
    budget: &RigBudget,
) -> Result<(), RigError> {
    let bad = RigError::BadSkin { mesh };
    if skin.offsets.len() != vertices + 1
        || skin.offsets[0] != 0
        || *skin.offsets.last().unwrap_or(&0) as usize != skin.influences.len()
        || skin.offsets.windows(2).any(|w| w[0] > w[1])
        || skin.joints.iter().any(|j| j.bone as usize >= bones)
        || skin
            .fallback
            .is_some_and(|f| f as usize >= skin.joints.len())
        || skin
            .influences
            .iter()
            .any(|i| i.joint as usize >= skin.joints.len())
    {
        return Err(bad);
    }
    if skin.joints.iter().any(|j| !j.bind_inverse.is_finite())
        || skin
            .influences
            .iter()
            .any(|i| !i.weight.is_finite() || i.weight < 0.0)
    {
        return Err(RigError::NonFinite {
            what: "スキンの行列・ウェイト（負のウェイトを含む）",
        });
    }
    // 頂点ごとの和で割り、0 の影響を捨てる（並びを詰め直す）
    let mut offsets = Vec::with_capacity(skin.offsets.len());
    let mut kept = Vec::with_capacity(skin.influences.len());
    offsets.push(0u32);
    for v in 0..vertices {
        let list = skin.influences_of(v);
        if list.len() > budget.max_influences_per_vertex {
            return Err(RigError::TooLarge {
                what: "1 つの頂点のウェイト",
                value: list.len(),
                limit: budget.max_influences_per_vertex,
            });
        }
        let total: f64 = list.iter().map(|i| i.weight as f64).sum();
        if total > 0.0 {
            for i in list.iter().filter(|i| i.weight > 0.0) {
                kept.push(Influence {
                    joint: i.joint,
                    weight: (i.weight as f64 / total) as f32,
                });
            }
        }
        offsets.push(kept.len() as u32);
    }
    skin.offsets = offsets;
    skin.influences = kept;
    Ok(())
}

fn validate_shape(
    mesh: usize,
    shape_index: usize,
    shape: &BlendShape,
    vertices: usize,
) -> Result<(), RigError> {
    let bad = RigError::BadBlendShape {
        mesh,
        shape: shape_index,
    };
    if shape.frames.is_empty() {
        return Err(bad);
    }
    let mut previous = 0.0f32;
    for f in &shape.frames {
        if !f.weight.is_finite() || f.weight <= previous {
            return Err(bad.clone());
        }
        previous = f.weight;
        if f.positions.len() != f.vertices.len()
            || (!f.normals.is_empty() && f.normals.len() != f.vertices.len())
            || f.vertices.iter().any(|&v| v as usize >= vertices)
        {
            return Err(bad.clone());
        }
        if f.positions.iter().chain(&f.normals).any(|p| !p.is_finite()) {
            return Err(RigError::NonFinite {
                what: "BlendShape の差分",
            });
        }
    }
    Ok(())
}
