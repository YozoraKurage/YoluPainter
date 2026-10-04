//! FBX（バイナリ・ASCII）を ufbx で読み、スキンにする。
//!
//! 骨: ufbx のノードを全部（根を含む）、親が先の深さ優先の並びで骨にする。ローカルの変換は ufbx の local_transform（前・後の回転と
//! ピボットを畳んだ T・R・S）。一様でない継承（Maya の segment scale compensate など）は ufbx の補助のノードに直させるので、
//! 骨のワールドは 親 × ローカル で ufbx の node_to_world と同じになる（読んだ後に確かめ、ずれたら知らせに書く）。
//!
//! メッシュ: メッシュを持つノード 1 つが 1 つのレンダラー。位置はノードの geometry_to_world（読んだときの形）で置いたモデルの空間。
//! スキンがあれば、関節 = クラスターの骨、bind_inverse = geometry_to_bone × geometry_to_world⁻¹（ufbx のスキニングと同じ式になる）。
//! ウェイトの無い頂点はメッシュのノードに付いて動く（ufbx の fallback と同じ）。スキンが無ければメッシュのノードに固く付く。
//! マテリアルはファイル全体で通し番号（最初に使われた順）にし、メッシュの中ではマテリアルごとにサブメッシュを分ける。

use std::collections::HashMap;
use std::time::Instant;

use yolu_core::geometry::{ModelMesh, Submesh};
use yolu_core::glam::{DMat3, DMat4, DQuat, DVec3, Mat4, Quat, Vec2, Vec3};
use yolu_core::skin::{
    BlendFrame, BlendShape, Bone, BoneTransform, Influence, Joint, Rig, RigBudget, RigError,
    RigMesh, Skin,
};

/// 読み込みの上限。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelLimits {
    /// ファイルの大きさ（バイト）。
    pub max_file_bytes: u64,
    /// ufbx が使うメモリ（一時と結果のそれぞれ。バイト）。
    pub parser_memory_bytes: usize,
    /// ノードの木の深さ。
    pub max_node_depth: u32,
    pub rig: RigBudget,
}

impl Default for ModelLimits {
    fn default() -> Self {
        ModelLimits {
            max_file_bytes: 1 << 30,
            parser_memory_bytes: 3 << 30,
            max_node_depth: 512,
            rig: RigBudget::default(),
        }
    }
}

/// 読めなかった理由。
#[derive(Clone, Debug, PartialEq)]
pub enum ModelError {
    /// ファイルを開けない・読めない。
    Io(String),
    /// ファイルが大きすぎる。
    FileTooLarge { bytes: u64, limit: u64 },
    /// ufbx が断った（壊れている・メモリの上限・深すぎる）。
    Parse(String),
    /// 描ける三角形のメッシュが無い。
    NoMesh,
    /// スキンの確かめ（予算を含む）で断った。
    Rig(RigError),
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelError::Io(e) => write!(f, "ファイルを読めません: {e}"),
            ModelError::FileTooLarge { bytes, limit } => write!(
                f,
                "ファイルが大きすぎます（{:.1} MiB、上限 {:.0} MiB）",
                *bytes as f64 / 1048576.0,
                *limit as f64 / 1048576.0
            ),
            ModelError::Parse(e) => write!(f, "FBX として読めません: {e}"),
            ModelError::NoMesh => f.write_str("三角形のメッシュがありません"),
            ModelError::Rig(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ModelError {}

impl From<RigError> for ModelError {
    fn from(e: RigError) -> Self {
        ModelError::Rig(e)
    }
}

/// 読んだ中身の数と、知らせ（読めたが近似したこと・飛ばしたもの）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadReport {
    pub meshes: usize,
    pub skinned_meshes: usize,
    pub triangles: usize,
    pub vertices: usize,
    pub bones: usize,
    pub blend_shapes: usize,
    pub materials: usize,
    pub warnings: Vec<String>,
    /// ufbx の読み込み（ミリ秒）。
    pub parse_ms: f64,
    /// スキンへの変換と確かめ（ミリ秒）。
    pub convert_ms: f64,
    /// 骨のワールドを 親 × ローカル で求め直したときの、ufbx の node_to_world との差の最大（メートル。行列の成分の差）。
    pub max_transform_error: f64,
}

/// 読んだモデル。
#[derive(Debug)]
pub struct LoadedModel {
    pub rig: Rig,
    pub report: LoadReport,
}

/// ファイルから読む（名前はファイル名の拡張子の前）。
pub fn load_fbx(path: &std::path::Path, limits: &ModelLimits) -> Result<LoadedModel, ModelError> {
    let bytes = std::fs::metadata(path)
        .map_err(|e| ModelError::Io(e.to_string()))?
        .len();
    if bytes > limits.max_file_bytes {
        return Err(ModelError::FileTooLarge {
            bytes,
            limit: limits.max_file_bytes,
        });
    }
    let data = std::fs::read(path).map_err(|e| ModelError::Io(e.to_string()))?;
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "モデル".into());
    load_fbx_bytes(&data, &name, limits)
}

/// バイト列から読む。
pub fn load_fbx_bytes(
    data: &[u8],
    name: &str,
    limits: &ModelLimits,
) -> Result<LoadedModel, ModelError> {
    if data.len() as u64 > limits.max_file_bytes {
        return Err(ModelError::FileTooLarge {
            bytes: data.len() as u64,
            limit: limits.max_file_bytes,
        });
    }
    let clock = Instant::now();
    let scene = ufbx::load_memory(data, load_options(limits))
        .map_err(|e| ModelError::Parse(format!("{:?}: {}", e.type_, &*e.description)))?;
    let parse_ms = clock.elapsed().as_secs_f64() * 1000.0;
    let clock = Instant::now();
    let mut model = convert(&scene, name, limits)?;
    model.report.parse_ms = parse_ms;
    model.report.convert_ms = clock.elapsed().as_secs_f64() * 1000.0;
    Ok(model)
}

/// ufbx の読み込みの設定（Unity の FBX の読み込みに合わせる。アニメーション・埋め込みの画像・外のファイルは読まない）。
pub(crate) fn load_options(limits: &ModelLimits) -> ufbx::LoadOpts<'static> {
    let memory = |limit: usize| ufbx::AllocatorOpts {
        memory_limit: limit,
        ..Default::default()
    };
    ufbx::LoadOpts {
        temp_allocator: memory(limits.parser_memory_bytes),
        result_allocator: memory(limits.parser_memory_bytes),
        ignore_animation: true,
        ignore_embedded: true,
        load_external_files: false,
        generate_missing_normals: true,
        clean_skin_weights: true,
        node_depth_limit: limits.max_node_depth,
        target_axes: ufbx::CoordinateAxes {
            right: ufbx::CoordinateAxis::PositiveX,
            up: ufbx::CoordinateAxis::PositiveY,
            front: ufbx::CoordinateAxis::PositiveZ,
        },
        target_unit_meters: 1.0,
        space_conversion: ufbx::SpaceConversion::ModifyGeometry,
        inherit_mode_handling: ufbx::InheritModeHandling::HelperNodes,
        geometry_transform_handling: ufbx::GeometryTransformHandling::Preserve,
        ..Default::default()
    }
}

// ---- 右手系（ufbx のメートル・Y が上）→ Unity の左手系（X を反転）----

pub(crate) fn dmat(m: &ufbx::Matrix) -> DMat4 {
    DMat4::from_cols_array(&[
        m.m00, m.m10, m.m20, 0.0, m.m01, m.m11, m.m21, 0.0, m.m02, m.m12, m.m22, 0.0, m.m03, m.m13,
        m.m23, 1.0,
    ])
}

pub(crate) fn dvec(v: ufbx::Vec3) -> DVec3 {
    DVec3::new(v.x, v.y, v.z)
}

/// 点・向きの X の反転。
pub(crate) fn mirror(v: DVec3) -> Vec3 {
    Vec3::new(-v.x as f32, v.y as f32, v.z as f32)
}

/// S × M × S（S = X の反転）。
pub(crate) fn mirror_matrix(m: DMat4) -> Mat4 {
    let mut m = m;
    m.x_axis.y = -m.x_axis.y;
    m.x_axis.z = -m.x_axis.z;
    m.x_axis.w = -m.x_axis.w;
    m.y_axis.x = -m.y_axis.x;
    m.z_axis.x = -m.z_axis.x;
    m.w_axis.x = -m.w_axis.x;
    m.as_mat4()
}

/// 回転の X の反転（S × R × S）。
pub(crate) fn mirror_quat(q: ufbx::Quat) -> Quat {
    let q = DQuat::from_xyzw(q.x, -q.y, -q.z, q.w).normalize();
    q.as_quat()
}

pub(crate) fn mirror_transform(t: &ufbx::Transform) -> BoneTransform {
    BoneTransform {
        translation: mirror(dvec(t.translation)),
        rotation: mirror_quat(t.rotation),
        scale: dvec(t.scale).as_vec3(),
    }
}

/// 分けた頂点の鍵（コントロールポイント・法線のビット・UV のビット）。
type VertexKey = (u32, [u64; 3], [u64; 2]);

/// ノードを根から深さ優先（親が先・子はファイルの順）に並べる。
fn node_order(scene: &ufbx::Scene) -> Vec<&ufbx::Node> {
    let mut out = Vec::with_capacity(scene.nodes.count);
    let mut stack: Vec<&ufbx::Node> = vec![&scene.root_node];
    while let Some(n) = stack.pop() {
        out.push(n);
        for c in n.children.iter().rev() {
            stack.push(c);
        }
    }
    out
}

fn convert(
    scene: &ufbx::Scene,
    name: &str,
    limits: &ModelLimits,
) -> Result<LoadedModel, ModelError> {
    let budget = &limits.rig;
    let mut report = LoadReport::default();
    let too_large = |what, value: usize, limit: usize| {
        ModelError::Rig(RigError::TooLarge { what, value, limit })
    };
    if scene.nodes.count > budget.max_bones {
        return Err(too_large("骨", scene.nodes.count, budget.max_bones));
    }
    // 大きな配列を作る前に、三角形の数で断る（インスタンスごとに数える）
    let triangles: usize = scene
        .nodes
        .iter()
        .filter_map(|n| n.mesh.as_ref())
        .map(|m| m.num_triangles)
        .sum();
    if triangles > budget.max_triangles {
        return Err(too_large("三角形", triangles, budget.max_triangles));
    }

    let order = node_order(scene);
    let mut bone_of: HashMap<u32, u32> = HashMap::with_capacity(order.len());
    let mut bones = Vec::with_capacity(order.len());
    let mut rest_world: Vec<DMat4> = Vec::with_capacity(order.len());
    for n in &order {
        let index = bones.len() as u32;
        bone_of.insert(n.element.typed_id, index);
        let parent = n
            .parent
            .as_ref()
            .and_then(|p| bone_of.get(&p.element.typed_id).copied());
        let bone_name = if n.is_root {
            name.to_string()
        } else {
            n.element.name.to_string()
        };
        bones.push(Bone {
            name: bone_name,
            parent,
            rest: mirror_transform(&n.local_transform),
        });
        rest_world.push(dmat(&n.node_to_world));
    }

    // マテリアル（ファイル全体の通し番号。最初に使われた順）
    let mut materials: Vec<String> = Vec::new();
    let mut material_of: HashMap<u32, i32> = HashMap::new();
    let mut unassigned: Option<i32> = None;

    let mut meshes = Vec::new();
    let mut rest_weights = Vec::new();
    // ウェイトと BlendShape の差分は、インスタンスを含む全体の和で数える（メッシュごとに数え直すと、同じメッシュを多数のノードで
    // 参照するファイルが、メッシュごとの上限をインスタンスの数だけ使い切ってから `Rig::new` で断られる）
    let mut influences_total: usize = 0;
    let mut offsets_total: usize = 0;
    for n in &order {
        let Some(mesh) = n.mesh.as_ref() else {
            continue;
        };
        let label = if n.element.name.is_empty() {
            mesh.element.name.to_string()
        } else {
            n.element.name.to_string()
        };
        let node_bone = bone_of[&n.element.typed_id];
        let g = dmat(&n.geometry_to_world);
        let det = DMat3::from_mat4(g).determinant();
        if !det.is_finite() || det.abs() < 1e-18 {
            report.warnings.push(format!(
                "{label}: 置き方の行列がつぶれているので飛ばしました"
            ));
            continue;
        }
        // 面のマテリアル → 通し番号
        let slots: Vec<&ufbx::Material> = if n.materials.count > 0 {
            n.materials.iter().map(|m| &**m).collect()
        } else {
            mesh.materials.iter().map(|m| &**m).collect()
        };
        let mut slot_material: Vec<i32> = Vec::with_capacity(slots.len());
        for m in &slots {
            let id = m.element.element_id;
            let next = materials.len() as i32;
            let index = *material_of.entry(id).or_insert_with(|| {
                materials.push(m.element.name.to_string());
                next
            });
            slot_material.push(index);
        }
        let mut unassigned_index = || {
            *unassigned.get_or_insert_with(|| {
                materials.push("マテリアルなし".into());
                materials.len() as i32 - 1
            })
        };
        let flip = det > 0.0; // X の反転で巡りが逆になるので戻す（置き方が鏡なら二重の反転で戻さない）
        if !flip {
            report
                .warnings
                .push(format!("{label}: 置き方が鏡になっています（負のスケール）"));
        }
        let normal_matrix = DMat3::from_mat4(g).inverse().transpose();
        let has_normals = mesh.vertex_normal.exists;
        let has_uvs = mesh.vertex_uv.exists;
        if !has_uvs {
            report
                .warnings
                .push(format!("{label}: UV が無いので描けません"));
        }

        // 頂点を（コントロールポイント・法線の値・UV の値）の組で分ける（添字ではなく値で比べる: Blender などは法線を
        // ポリゴンの頂点ごとに直に書くので、添字ではどの頂点も分かれてしまう）
        let mut key_to_vertex: HashMap<VertexKey, u32> = HashMap::with_capacity(mesh.num_indices);
        let mut vertex_cp: Vec<u32> = Vec::new();
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uvs = Vec::new();
        let mut by_slot: Vec<Vec<u32>> = vec![Vec::new(); slots.len().max(1)];
        let mut no_slot: Vec<u32> = Vec::new();
        let mut tri = vec![0u32; mesh.max_face_triangles * 3];
        let vertex_for = |ix: u32,
                          key_to_vertex: &mut HashMap<VertexKey, u32>,
                          vertex_cp: &mut Vec<u32>,
                          positions: &mut Vec<Vec3>,
                          normals: &mut Vec<Vec3>,
                          uvs: &mut Vec<Vec2>|
         -> u32 {
            let i = ix as usize;
            let cp = mesh.vertex_indices[i];
            let ni = if has_normals {
                mesh.vertex_normal.indices[i]
            } else {
                0
            };
            let ui = if has_uvs {
                mesh.vertex_uv.indices[i]
            } else {
                0
            };
            let nv = if has_normals {
                let v = mesh.vertex_normal.values[ni as usize];
                [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]
            } else {
                [0; 3]
            };
            let uv = if has_uvs {
                let v = mesh.vertex_uv.values[ui as usize];
                [v.x.to_bits(), v.y.to_bits()]
            } else {
                [0; 2]
            };
            *key_to_vertex.entry((cp, nv, uv)).or_insert_with(|| {
                let p = g.transform_point3(dvec(mesh.vertices[cp as usize]));
                positions.push(mirror(p));
                if has_normals {
                    let nn = normal_matrix * dvec(mesh.vertex_normal.values[ni as usize]);
                    normals.push(mirror(nn.normalize_or_zero()));
                }
                uvs.push(if has_uvs {
                    let uv = mesh.vertex_uv.values[ui as usize];
                    Vec2::new(uv.x as f32, uv.y as f32)
                } else {
                    Vec2::ZERO
                });
                vertex_cp.push(cp);
                vertex_cp.len() as u32 - 1
            })
        };
        for (fi, face) in mesh.faces.iter().enumerate() {
            if face.num_indices < 3 {
                continue;
            }
            let count = ufbx::triangulate_face(&mut tri, mesh, *face) as usize;
            let slot = mesh.face_material.get(fi).copied().unwrap_or(0) as usize;
            let target = if slot < slots.len() {
                &mut by_slot[slot]
            } else {
                &mut no_slot
            };
            for t in tri[..count * 3].as_chunks::<3>().0 {
                let [a, b, c] = [t[0], t[1], t[2]].map(|ix| {
                    vertex_for(
                        ix,
                        &mut key_to_vertex,
                        &mut vertex_cp,
                        &mut positions,
                        &mut normals,
                        &mut uvs,
                    )
                });
                if flip {
                    target.extend_from_slice(&[a, c, b]);
                } else {
                    target.extend_from_slice(&[a, b, c]);
                }
            }
        }
        let mut submeshes = Vec::new();
        for (slot, indices) in by_slot.into_iter().enumerate() {
            if indices.is_empty() {
                continue;
            }
            let material = match slot_material.get(slot) {
                Some(&m) => m,
                None => unassigned_index(),
            };
            submeshes.push(Submesh { material, indices });
        }
        if !no_slot.is_empty() {
            submeshes.push(Submesh {
                material: unassigned_index(),
                indices: no_slot,
            });
        }
        if submeshes.is_empty() {
            continue;
        }
        let vertex_count = positions.len();
        report.vertices += vertex_count;
        if report.vertices > budget.max_vertices {
            return Err(too_large("頂点", report.vertices, budget.max_vertices));
        }

        // コントロールポイント → 分けた頂点（CSR）
        let cp_count = mesh.num_vertices;
        let mut cp_offsets = vec![0u32; cp_count + 1];
        for &cp in &vertex_cp {
            cp_offsets[cp as usize + 1] += 1;
        }
        for i in 0..cp_count {
            cp_offsets[i + 1] += cp_offsets[i];
        }
        let mut fill = cp_offsets.clone();
        let mut cp_vertices = vec![0u32; vertex_count];
        for (v, &cp) in vertex_cp.iter().enumerate() {
            cp_vertices[fill[cp as usize] as usize] = v as u32;
            fill[cp as usize] += 1;
        }
        let split = |cp: u32| {
            &cp_vertices[cp_offsets[cp as usize] as usize..cp_offsets[cp as usize + 1] as usize]
        };

        let g_inverse = g.inverse();
        let node_bind_inverse = mirror_matrix(dmat(&n.node_to_world).inverse());
        let skin = match mesh.skin_deformers.first() {
            Some(sd) => {
                if mesh.skin_deformers.count > 1 {
                    report.warnings.push(format!(
                        "{label}: スキンが 2 つ以上あるので、最初のものだけ使います"
                    ));
                }
                if sd.skinning_method != ufbx::SkinningMethod::Linear {
                    report.warnings.push(format!(
                        "{label}: 線形でないスキニング（{:?}）を線形で近似します",
                        sd.skinning_method
                    ));
                }
                let mut joints = Vec::with_capacity(sd.clusters.count + 1);
                let mut joint_of_cluster: Vec<Option<u32>> = Vec::with_capacity(sd.clusters.count);
                for c in sd.clusters.iter() {
                    match c
                        .bone_node
                        .as_ref()
                        .and_then(|b| bone_of.get(&b.element.typed_id))
                    {
                        Some(&bone) => {
                            joint_of_cluster.push(Some(joints.len() as u32));
                            joints.push(Joint {
                                bone,
                                bind_inverse: mirror_matrix(dmat(&c.geometry_to_bone) * g_inverse),
                            });
                        }
                        None => joint_of_cluster.push(None),
                    }
                }
                if joint_of_cluster.iter().any(|j| j.is_none()) {
                    report
                        .warnings
                        .push(format!("{label}: 骨の無いクラスターのウェイトを捨てました"));
                }
                let fallback = joints.len() as u32;
                joints.push(Joint {
                    bone: node_bone,
                    bind_inverse: node_bind_inverse,
                });
                let mut offsets = Vec::with_capacity(vertex_count + 1);
                let mut influences = Vec::new();
                offsets.push(0u32);
                for &cp in &vertex_cp {
                    if let Some(sv) = sd.vertices.get(cp as usize) {
                        let start = sv.weight_begin as usize;
                        let end = (start + sv.num_weights as usize).min(sd.weights.count);
                        for w in &sd.weights.as_ref()[start.min(end)..end] {
                            if let Some(Some(joint)) =
                                joint_of_cluster.get(w.cluster_index as usize)
                            {
                                influences.push(Influence {
                                    joint: *joint,
                                    weight: w.weight as f32,
                                });
                            }
                        }
                    }
                    if influences_total + influences.len() > budget.max_influences {
                        return Err(too_large(
                            "ウェイト",
                            influences_total + influences.len(),
                            budget.max_influences,
                        ));
                    }
                    offsets.push(influences.len() as u32);
                }
                influences_total += influences.len();
                report.skinned_meshes += 1;
                Skin {
                    joints,
                    offsets,
                    influences,
                    fallback: Some(fallback),
                }
            }
            None => Skin::rigid(node_bone, node_bind_inverse, vertex_count),
        };

        // BlendShape（チャンネル 1 つが 1 つ。キーフレームが中間のフレーム）
        let linear = DMat3::from_mat4(g);
        let mut shapes = Vec::new();
        let mut weights = Vec::new();
        for bd in mesh.blend_deformers.iter() {
            for ch in bd.channels.iter() {
                let mut frames: Vec<BlendFrame> = Vec::new();
                let mut keys: Vec<&ufbx::BlendKeyframe> = ch.keyframes.iter().collect();
                keys.sort_by(|a, b| a.target_weight.total_cmp(&b.target_weight));
                for k in keys {
                    let weight = (k.target_weight * 100.0) as f32;
                    if weight.is_nan() || weight <= frames.last().map_or(0.0, |f| f.weight) {
                        report.warnings.push(format!(
                            "{label}: BlendShape「{}」の重み {weight} のフレームを飛ばしました",
                            &*ch.element.name
                        ));
                        continue;
                    }
                    let shape = &k.shape;
                    let mut f = BlendFrame {
                        weight,
                        ..BlendFrame::default()
                    };
                    let with_normals = shape.normal_offsets.count == shape.num_offsets;
                    let mut out_of_range = false;
                    for i in 0..shape.num_offsets {
                        let cp = shape.offset_vertices[i];
                        if cp as usize >= cp_count {
                            out_of_range = true;
                            continue;
                        }
                        let scale = shape.offset_weights.get(i).copied().unwrap_or(1.0);
                        let dp = mirror(linear * dvec(shape.position_offsets[i]) * scale);
                        let dn = if with_normals {
                            mirror(normal_matrix * dvec(shape.normal_offsets[i]) * scale)
                        } else {
                            Vec3::ZERO
                        };
                        for &v in split(cp) {
                            f.vertices.push(v);
                            f.positions.push(dp);
                            if with_normals {
                                f.normals.push(dn);
                            }
                        }
                        // 差分 1 つ（の分けた頂点）ごとに数える（同じ点を指す差分を並べて、1 フレームで膨らませない）
                        if offsets_total + f.vertices.len() > budget.max_blend_offsets {
                            return Err(too_large(
                                "BlendShape の差分",
                                offsets_total + f.vertices.len(),
                                budget.max_blend_offsets,
                            ));
                        }
                    }
                    if out_of_range {
                        report.warnings.push(format!(
                            "{label}: BlendShape「{}」の範囲外の頂点を飛ばしました",
                            &*ch.element.name
                        ));
                    }
                    offsets_total += f.vertices.len();
                    frames.push(f);
                }
                if frames.is_empty() {
                    continue;
                }
                shapes.push(BlendShape {
                    name: ch.element.name.to_string(),
                    frames,
                });
                weights.push((ch.weight * 100.0) as f32);
            }
        }
        report.blend_shapes += shapes.len();
        meshes.push(RigMesh {
            mesh: ModelMesh {
                name: label,
                positions,
                normals,
                uvs,
                submeshes,
            },
            skin,
            blend_shapes: shapes,
        });
        rest_weights.push(weights);
        if meshes.len() > budget.max_meshes {
            return Err(too_large("メッシュ", meshes.len(), budget.max_meshes));
        }
    }
    if meshes.is_empty() {
        return Err(ModelError::NoMesh);
    }
    let rig = Rig::new(name, bones, meshes, materials, rest_weights, budget)?;
    // 骨のワールドを 親 × ローカル で求め直し、ufbx と比べる（一様でない継承などが残っていないか）
    let world = rig.world_matrices(&rig.rest_pose())?;
    let mut worst = 0.0f64;
    for (mine, theirs) in world.iter().zip(&rest_world) {
        let theirs = mirror_matrix(*theirs);
        for (a, b) in mine.to_cols_array().iter().zip(theirs.to_cols_array()) {
            worst = worst.max((*a as f64 - b as f64).abs());
        }
    }
    report.max_transform_error = worst;
    if worst > 1e-3 {
        report.warnings.push(format!(
            "骨の変換を完全には再現できません（最大の差 {worst:.4}）"
        ));
    }
    report.meshes = rig.meshes().len();
    report.triangles = rig.triangle_count();
    report.bones = rig.bones().len();
    report.materials = rig.materials().len();
    Ok(LoadedModel { rig, report })
}
