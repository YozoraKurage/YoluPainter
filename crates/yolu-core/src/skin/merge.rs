//! 読んだ `Rig` を並べて 1 つにする（Live Link で、1 つの相手が使う複数の FBX を 1 つのモデルにする）。
//!
//! - 骨・メッシュ・マテリアル・BlendShape を部分の順に並べ、番号（親・関節の骨・メッシュの骨・サブメッシュのマテリアル）をずらす。ポーズ
//!   （`Pose.locals`・`blend_weights`）も同じ並び: 合わせた Rig の休みのポーズは、部分の休みのポーズを並べたもの。
//! - 部分ごとに入れるメッシュを選べる（相手が使わないメッシュは入れない）。同じ `Rig` を 2 つの部分に使ってよい（同じ FBX を 2 つの
//!   レンダラーの組が使う: 1 回読んで、骨とメッシュを 2 つ置く）。
//! - 部分の根の骨の名前を替えられる（FBX の番号で分ける。名前の道が部分をまたいで重ならないように）。
//! - 部分ごとの一様な倍率（Unity の取り込みの `globalScale` など）: 骨の休みの移動・頂点・BlendShape の位置の差分・関節の bind の移動に
//!   掛ける。回転・骨の大きさ・法線は変わらない（一様な拡大は回転と入れ替えられるので、どのポーズでも形は倍率を掛けたものと同じ）。
//! - マテリアルの並びを呼び手が決められる（Unity のマテリアルの番号へ付け替える）。決めないときは部分のマテリアルの名前を並べる。
//! - 部分が 1 つで、メッシュを全部・倍率 1・BlendShape を残す・名前もマテリアルも替えないなら、元と同じ Rig になる。

use std::ops::Range;

use super::{BlendFrame, BlendShape, Bone, Joint, Rig, RigBudget, RigError, RigMesh, Skin};
use crate::geometry::{ModelMesh, Submesh};

/// 入れるメッシュ 1 つ。
#[derive(Clone, Debug, PartialEq)]
pub struct MeshPick {
    /// 部分の `Rig::meshes()` の番号。
    pub mesh: usize,
    /// サブメッシュの順の、合わせた Rig のマテリアルの番号（[`merge`] の `materials` を決めたときだけ使う。サブメッシュと同じ数）。
    pub materials: Vec<i32>,
}

impl MeshPick {
    /// マテリアルを付け替えないもの。
    pub fn keep(mesh: usize) -> MeshPick {
        MeshPick {
            mesh,
            materials: Vec::new(),
        }
    }
}

/// 並べる部分 1 つ。
#[derive(Clone, Debug)]
pub struct MergePart<'a> {
    pub rig: &'a Rig,
    /// 入れるメッシュ（この順に並ぶ）。
    pub meshes: Vec<MeshPick>,
    /// BlendShape を残すか（Unity の取り込みが BlendShape を読まない設定なら false）。
    pub blend_shapes: bool,
    /// 一様な倍率（正の有限の数）。
    pub scale: f32,
    /// 根の骨（親の無い骨）の新しい名前（None は元のまま）。
    pub root_name: Option<String>,
}

impl<'a> MergePart<'a> {
    /// 全部のメッシュを、そのまま入れる部分。
    pub fn whole(rig: &'a Rig) -> MergePart<'a> {
        MergePart {
            rig,
            meshes: (0..rig.meshes().len()).map(MeshPick::keep).collect(),
            blend_shapes: true,
            scale: 1.0,
            root_name: None,
        }
    }
}

/// 部分が合わせた Rig のどこに入ったか。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartRange {
    pub bones: Range<usize>,
    pub meshes: Range<usize>,
    /// 部分のマテリアル（`materials` を決めなかったときだけ。決めたときは空）。
    pub materials: Range<usize>,
}

/// 合わせた Rig と、部分の置き場。
#[derive(Debug)]
pub struct Merged {
    pub rig: Rig,
    pub parts: Vec<PartRange>,
}

/// 部分を並べて 1 つの Rig にする。`materials` が Some なら、合わせた Rig のマテリアルはその並びで、各メッシュのサブメッシュは
/// `MeshPick::materials` の番号になる（数が違えば断る）。名前 `name` は部分が 2 つ以上のときに使い、1 つなら部分の名前のまま。
pub fn merge(
    name: &str,
    parts: &[MergePart<'_>],
    materials: Option<Vec<String>>,
    budget: &RigBudget,
) -> Result<Merged, RigError> {
    // 部分が 1 つで何も替えないなら、元の Rig をそのまま（確かめ直しでウェイトを割り直さない: 同じ値のまま）
    if let ([one], None) = (parts, &materials) {
        let all = one.meshes.len() == one.rig.meshes().len()
            && one.meshes.iter().enumerate().all(|(i, p)| p.mesh == i);
        if all && one.scale == 1.0 && one.blend_shapes && one.root_name.is_none() {
            let rig = one.rig.clone();
            let parts = vec![PartRange {
                bones: 0..rig.bones().len(),
                meshes: 0..rig.meshes().len(),
                materials: 0..rig.materials().len(),
            }];
            return Ok(Merged { rig, parts });
        }
    }
    let mut bones: Vec<Bone> = Vec::new();
    let mut meshes: Vec<RigMesh> = Vec::new();
    let mut weights: Vec<Vec<f32>> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut ranges = Vec::with_capacity(parts.len());
    let remap = materials.is_some();
    for part in parts {
        let k = part.scale;
        if !k.is_finite() || k <= 0.0 {
            return Err(RigError::NonFinite { what: "倍率" });
        }
        let rig = part.rig;
        let bone_base = bones.len() as u32;
        let material_base = names.len() as i32;
        for b in rig.bones() {
            let mut b = b.clone();
            b.parent = b.parent.map(|p| p + bone_base);
            b.rest.translation *= k;
            if b.parent.is_none() {
                if let Some(n) = &part.root_name {
                    b.name.clone_from(n);
                }
            }
            bones.push(b);
        }
        if !remap {
            names.extend(rig.materials().iter().cloned());
        }
        let mesh_start = meshes.len();
        let rest = rig.rest_pose();
        for pick in &part.meshes {
            let source = rig
                .meshes()
                .get(pick.mesh)
                .ok_or(RigError::BadMesh { mesh: pick.mesh })?;
            if remap && pick.materials.len() != source.mesh.submeshes.len() {
                return Err(RigError::BadMesh { mesh: pick.mesh });
            }
            let submeshes = source
                .mesh
                .submeshes
                .iter()
                .enumerate()
                .map(|(i, s)| Submesh {
                    material: if remap {
                        pick.materials[i]
                    } else if s.material < 0 {
                        s.material
                    } else {
                        s.material + material_base
                    },
                    indices: s.indices.clone(),
                })
                .collect();
            let mesh = ModelMesh {
                name: source.mesh.name.clone(),
                positions: source.mesh.positions.iter().map(|p| *p * k).collect(),
                normals: source.mesh.normals.clone(),
                uvs: source.mesh.uvs.clone(),
                submeshes,
            };
            let skin = Skin {
                joints: source
                    .skin
                    .joints
                    .iter()
                    .map(|j| {
                        let mut bind_inverse = j.bind_inverse;
                        bind_inverse.w_axis.x *= k;
                        bind_inverse.w_axis.y *= k;
                        bind_inverse.w_axis.z *= k;
                        Joint {
                            bone: j.bone + bone_base,
                            bind_inverse,
                        }
                    })
                    .collect(),
                offsets: source.skin.offsets.clone(),
                influences: source.skin.influences.clone(),
                fallback: source.skin.fallback,
            };
            let blend_shapes: Vec<BlendShape> = if part.blend_shapes {
                source
                    .blend_shapes
                    .iter()
                    .map(|s| BlendShape {
                        name: s.name.clone(),
                        frames: s
                            .frames
                            .iter()
                            .map(|f| BlendFrame {
                                weight: f.weight,
                                vertices: f.vertices.clone(),
                                positions: f.positions.iter().map(|p| *p * k).collect(),
                                normals: f.normals.clone(),
                            })
                            .collect(),
                    })
                    .collect()
            } else {
                Vec::new()
            };
            weights.push(if part.blend_shapes {
                rest.blend_weights[pick.mesh].clone()
            } else {
                Vec::new()
            });
            meshes.push(RigMesh {
                mesh,
                skin,
                blend_shapes,
                node: source.node.map(|n| n + bone_base),
            });
        }
        ranges.push(PartRange {
            bones: bone_base as usize..bones.len(),
            meshes: mesh_start..meshes.len(),
            materials: if remap {
                0..0
            } else {
                material_base as usize..names.len()
            },
        });
    }
    let names = materials.unwrap_or(names);
    let name = match parts {
        [one] => one.rig.name(),
        _ => name,
    };
    let rig = Rig::new(name, bones, meshes, names, weights, budget)?;
    Ok(Merged { rig, parts: ranges })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skin::{demo_figure, FigureDetail, Pose};
    use glam::{Quat, Vec3};

    fn figure() -> Rig {
        demo_figure(FigureDetail::SMALL)
    }

    fn deformed(rig: &Rig, pose: &Pose) -> Vec<Vec<Vec3>> {
        rig.deform(pose)
            .unwrap()
            .into_iter()
            .map(|m| m.positions)
            .collect()
    }

    fn close(a: &[Vec<Vec3>], b: &[Vec<Vec3>], k: f32) {
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b) {
            assert_eq!(x.len(), y.len());
            for (p, q) in x.iter().zip(y) {
                assert!((*p * k - *q).length() < 1e-4, "{p} × {k} と {q}");
            }
        }
    }

    #[test]
    fn one_whole_part_is_the_same_rig() {
        let rig = figure();
        let merged = merge(
            "合わせ",
            &[MergePart::whole(&rig)],
            None,
            &RigBudget::default(),
        )
        .unwrap();
        let m = &merged.rig;
        assert_eq!(m.name(), rig.name());
        assert_eq!(m.bones(), rig.bones());
        assert_eq!(m.meshes(), rig.meshes());
        assert_eq!(m.materials(), rig.materials());
        assert_eq!(m.rest_pose(), rig.rest_pose());
        assert_eq!(merged.parts[0].bones, 0..rig.bones().len());
        // 何かを替えた 1 つの部分は確かめ直して作るが、形は同じ
        let renamed = MergePart {
            root_name: Some("0:人形".into()),
            ..MergePart::whole(&rig)
        };
        let again = merge("合わせ", &[renamed], None, &RigBudget::default()).unwrap();
        assert_eq!(again.rig.bones()[0].name, "0:人形");
        close(
            &deformed(&rig, &rig.rest_pose()),
            &deformed(&again.rig, &again.rig.rest_pose()),
            1.0,
        );
    }

    #[test]
    fn two_parts_line_up_bones_meshes_materials_and_poses() {
        let a = figure();
        let b = figure();
        let parts = [
            MergePart {
                root_name: Some("0:人形".into()),
                ..MergePart::whole(&a)
            },
            MergePart {
                root_name: Some("1:人形".into()),
                ..MergePart::whole(&b)
            },
        ];
        let merged = merge("二体", &parts, None, &RigBudget::default()).unwrap();
        let m = &merged.rig;
        let (nb, nm) = (a.bones().len(), a.meshes().len());
        assert_eq!(m.name(), "二体");
        assert_eq!(m.bones().len(), nb * 2);
        assert_eq!(m.meshes().len(), nm * 2);
        assert_eq!(m.materials().len(), a.materials().len() * 2);
        assert_eq!(merged.parts[1].bones, nb..nb * 2);
        assert_eq!(merged.parts[1].meshes, nm..nm * 2);
        assert_eq!(m.roots().collect::<Vec<_>>(), [0, nb]);
        assert_eq!(m.bones()[nb].name, "1:人形");
        assert_eq!(
            m.bones()[nb + 1].parent,
            a.bones()[1].parent.map(|p| p + nb as u32)
        );
        // 2 つ目のメッシュのマテリアルは、2 つ目の部分のマテリアル
        let second = &m.meshes()[nm];
        assert_eq!(
            second.mesh.submeshes[0].material,
            a.meshes()[0].mesh.submeshes[0].material + a.materials().len() as i32
        );
        assert_eq!(second.node, a.meshes()[0].node.map(|n| n + nb as u32));
        assert!(second.skin.joints.iter().all(|j| j.bone as usize >= nb));
        // 名前の道は部分ごとに別
        assert_eq!(m.resolve_bone_path(&m.bone_path(nb + 3)), Ok(nb + 3));
        // 休みのポーズは並べたもの。2 つ目の部分の骨だけ動かすと、2 つ目のメッシュだけ動く
        let rest = m.rest_pose();
        assert_eq!(rest.locals.len(), nb * 2);
        assert_eq!(rest.blend_weights.len(), nm * 2);
        let before = deformed(m, &rest);
        let mut posed = rest.clone();
        let bone = (1..nb).find(|&i| a.is_deforming(i)).unwrap();
        posed.locals[nb + bone].rotation = Quat::from_rotation_z(0.8);
        let after = deformed(m, &posed);
        assert_eq!(after[..nm], before[..nm], "1 つ目はそのまま");
        assert_ne!(after[nm..], before[nm..], "2 つ目が動く");
        // 1 つの部分の形と同じ
        close(&deformed(&a, &a.rest_pose()), &before[..nm], 1.0);
    }

    #[test]
    fn a_scale_multiplies_the_shape_in_any_pose() {
        let rig = figure();
        let k = 2.5;
        let part = MergePart {
            scale: k,
            ..MergePart::whole(&rig)
        };
        let scaled = merge("倍", &[part], None, &RigBudget::default())
            .unwrap()
            .rig;
        close(
            &deformed(&rig, &rig.rest_pose()),
            &deformed(&scaled, &scaled.rest_pose()),
            k,
        );
        // 骨を回し、移動も変えたポーズ（移動は同じ倍率）
        let bone = (1..rig.bones().len())
            .find(|&i| rig.is_deforming(i))
            .unwrap();
        let mut pose = rig.rest_pose();
        pose.locals[bone].rotation = Quat::from_rotation_x(0.6);
        pose.locals[bone].translation += Vec3::new(0.01, 0.02, 0.0);
        for w in pose.blend_weights.iter_mut().flatten() {
            *w = 70.0;
        }
        let mut big = pose.clone();
        for l in &mut big.locals {
            l.translation *= k;
        }
        close(&deformed(&rig, &pose), &deformed(&scaled, &big), k);
        let bad = MergePart {
            scale: f32::NAN,
            ..MergePart::whole(&rig)
        };
        assert!(merge("x", &[bad], None, &RigBudget::default()).is_err());
    }

    #[test]
    fn picked_meshes_get_the_given_materials_and_shapes_can_be_left_out() {
        let rig = figure();
        let last = rig.meshes().len() - 1;
        let subs = rig.meshes()[last].mesh.submeshes.len();
        let part = MergePart {
            meshes: vec![MeshPick {
                mesh: last,
                materials: vec![1; subs],
            }],
            blend_shapes: false,
            ..MergePart::whole(&rig)
        };
        let merged = merge(
            "選び",
            std::slice::from_ref(&part),
            Some(vec!["A".into(), "B".into()]),
            &RigBudget::default(),
        )
        .unwrap();
        let m = &merged.rig;
        assert_eq!(m.meshes().len(), 1);
        assert_eq!(m.materials(), ["A", "B"]);
        assert!(m.meshes()[0].mesh.submeshes.iter().all(|s| s.material == 1));
        assert!(m.meshes()[0].blend_shapes.is_empty());
        assert_eq!(m.rest_pose().blend_weights, vec![Vec::<f32>::new()]);
        assert_eq!(m.bones().len(), rig.bones().len(), "骨は全部");
        // 数の合わない付け替え・無いメッシュは断る
        let wrong = MergePart {
            meshes: vec![MeshPick {
                mesh: last,
                materials: vec![0; subs + 1],
            }],
            ..part.clone()
        };
        assert!(merge("x", &[wrong], Some(vec!["A".into()]), &RigBudget::default()).is_err());
        let missing = MergePart {
            meshes: vec![MeshPick::keep(99)],
            ..part
        };
        assert!(merge("x", &[missing], None, &RigBudget::default()).is_err());
    }
}
