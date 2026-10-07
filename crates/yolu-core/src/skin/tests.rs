//! スキンの試験: 既知の答えのスキニング（鎖の伝わり・混ぜ・法線）、ウェイトの正規化と fallback、4 本を超える影響、BlendShape
//! （中間のフレーム・スキニングより先）、骨の回し方、予算と壊れた入力の拒否、並列と 1 頂点ずつの式の一致、試しの人形。

use glam::{Mat4, Quat, Vec2, Vec3};

use super::*;
use crate::geometry::{ModelMesh, Submesh};

fn close(a: Vec3, b: Vec3) -> bool {
    (a - b).length() < 1e-5
}

fn assert_close(a: Vec3, b: Vec3, what: &str) {
    assert!(close(a, b), "{what}: {a} と {b}");
}

/// 根（原点）→ 上腕（原点）→ 前腕（x = 1）の鎖と、x 軸の上の頂点のメッシュ。
fn arm(vertices: &[(Vec3, &[(u32, f32)])]) -> Rig {
    let bones = vec![
        Bone {
            name: "根".into(),
            parent: None,
            rest: BoneTransform::IDENTITY,
        },
        Bone {
            name: "上腕".into(),
            parent: Some(0),
            rest: BoneTransform::IDENTITY,
        },
        Bone {
            name: "前腕".into(),
            parent: Some(1),
            rest: BoneTransform {
                translation: Vec3::X,
                ..BoneTransform::IDENTITY
            },
        },
    ];
    let mut offsets = vec![0u32];
    let mut influences = Vec::new();
    for (_, w) in vertices {
        for &(joint, weight) in *w {
            influences.push(Influence { joint, weight });
        }
        offsets.push(influences.len() as u32);
    }
    let n = vertices.len() as u32;
    let mesh = ModelMesh {
        name: "腕".into(),
        positions: vertices.iter().map(|(p, _)| *p).collect(),
        normals: vec![Vec3::Y; vertices.len()],
        uvs: Vec::new(),
        submeshes: vec![Submesh {
            material: 0,
            indices: (0..n.saturating_sub(2))
                .flat_map(|i| [i, i + 1, i + 2])
                .collect(),
        }],
    };
    let skin = Skin {
        joints: vec![
            Joint {
                bone: 1,
                bind_inverse: Mat4::IDENTITY,
            },
            Joint {
                bone: 2,
                bind_inverse: Mat4::from_translation(-Vec3::X),
            },
        ],
        offsets,
        influences,
        fallback: None,
    };
    Rig::new(
        "腕",
        bones,
        vec![RigMesh {
            mesh,
            skin,
            blend_shapes: Vec::new(),
            node: None,
        }],
        vec!["肌".into()],
        Vec::new(),
        &RigBudget::default(),
    )
    .unwrap()
}

#[test]
fn rest_pose_keeps_the_bind_shape() {
    let rig = arm(&[
        (Vec3::new(0.5, 0.0, 0.0), &[(0, 1.0)]),
        (Vec3::new(1.5, 0.0, 0.0), &[(1, 1.0)]),
        (Vec3::new(1.2, 0.1, 0.0), &[(0, 0.5), (1, 0.5)]),
    ]);
    let posed = rig.deform(&rig.rest_pose()).unwrap();
    assert_eq!(posed[0].positions, rig.meshes()[0].mesh.positions);
    assert_eq!(posed[0].uvs, rig.meshes()[0].mesh.uvs);
    assert_eq!(posed[0].submeshes, rig.meshes()[0].mesh.submeshes);
}

#[test]
fn rotating_the_forearm_moves_only_its_vertices_and_blends_at_the_elbow() {
    let rig = arm(&[
        (Vec3::new(0.5, 0.0, 0.0), &[(0, 1.0)]),
        (Vec3::new(1.5, 0.0, 0.0), &[(1, 1.0)]),
        (Vec3::new(1.2, 0.0, 0.0), &[(0, 0.5), (1, 0.5)]),
    ]);
    let mut pose = rig.rest_pose();
    pose.locals[2].rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    let p = &rig.deform(&pose).unwrap()[0];
    assert_close(
        p.positions[0],
        Vec3::new(0.5, 0.0, 0.0),
        "上腕だけの頂点は動かない",
    );
    // 前腕の原点 (1, 0, 0) のまわりに 90° 回る: 前腕の空間の (0.5, 0, 0) → (0, 0.5, 0)
    assert_close(p.positions[1], Vec3::new(1.0, 0.5, 0.0), "前腕の頂点");
    // 半々: 上腕の (1.2, 0, 0) と前腕の (1, 0.2, 0) の平均
    assert_close(p.positions[2], Vec3::new(1.1, 0.1, 0.0), "肘の頂点");
    // 法線: 前腕だけの頂点は (0, 1, 0) → (−1, 0, 0)、半々は (−1, 1, 0) を正規化
    assert_close(p.normals[0], Vec3::Y, "上腕の法線");
    assert_close(p.normals[1], -Vec3::X, "前腕の法線");
    assert_close(
        p.normals[2],
        Vec3::new(-1.0, 1.0, 0.0).normalize(),
        "肘の法線",
    );
}

#[test]
fn rotating_the_upper_arm_carries_the_child_bone() {
    let rig = arm(&[
        (Vec3::new(0.5, 0.0, 0.0), &[(0, 1.0)]),
        (Vec3::new(1.5, 0.0, 0.0), &[(1, 1.0)]),
    ]);
    let mut pose = rig.rest_pose();
    // y のまわりに 90°: x 軸は −z へ
    pose.locals[1].rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    let world = rig.world_matrices(&pose).unwrap();
    assert_close(
        Rig::bone_origin(&world, 2),
        Vec3::new(0.0, 0.0, -1.0),
        "前腕の原点",
    );
    let p = &rig.deform(&pose).unwrap()[0];
    assert_close(p.positions[0], Vec3::new(0.0, 0.0, -0.5), "上腕の頂点");
    assert_close(
        p.positions[1],
        Vec3::new(0.0, 0.0, -1.5),
        "前腕の頂点（親の回転が伝わる）",
    );
}

#[test]
fn weights_are_normalised_and_unweighted_vertices_use_the_fallback() {
    let mut rig = arm(&[
        (Vec3::new(1.2, 0.0, 0.0), &[(0, 2.0), (1, 2.0)]),
        (Vec3::new(1.5, 0.0, 0.0), &[]),
        (Vec3::new(1.5, 0.0, 0.0), &[(0, 0.0), (1, 0.0)]),
    ]);
    let skin = &rig.meshes()[0].skin;
    assert_eq!(skin.influences_of(0)[0].weight, 0.5, "和で割る");
    assert!(skin.influences_of(2).is_empty(), "0 の影響は捨てる");
    let mut pose = rig.rest_pose();
    pose.locals[2].rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    let p = &rig.deform(&pose).unwrap()[0];
    assert_close(p.positions[0], Vec3::new(1.1, 0.1, 0.0), "2 と 2 は半々");
    assert_close(
        p.positions[1],
        Vec3::new(1.5, 0.0, 0.0),
        "fallback が無ければバインドのまま",
    );
    assert_close(p.positions[2], Vec3::new(1.5, 0.0, 0.0), "和が 0 も同じ");
    // fallback を前腕にすると、ウェイトの無い頂点は前腕に付いて動く
    let mut meshes = rig.meshes().to_vec();
    meshes[0].skin.fallback = Some(1);
    rig = Rig::new(
        "腕",
        rig.bones().to_vec(),
        meshes,
        vec![],
        vec![],
        &RigBudget::default(),
    )
    .unwrap();
    let p = &rig.deform(&pose).unwrap()[0];
    assert_close(p.positions[1], Vec3::new(1.0, 0.5, 0.0), "fallback の関節");
}

#[test]
fn more_than_four_influences_average_all_of_them() {
    // 8 本の骨（根の子、x = 0..7 に置いた）から 1/8 ずつ。各骨を +y へ k だけ動かすと、頂点は平均の 3.5 だけ上がる
    let mut bones = vec![Bone {
        name: "根".into(),
        parent: None,
        rest: BoneTransform::IDENTITY,
    }];
    for k in 0..8 {
        bones.push(Bone {
            name: format!("骨 {k}"),
            parent: Some(0),
            rest: BoneTransform {
                translation: Vec3::new(k as f32, 0.0, 0.0),
                ..BoneTransform::IDENTITY
            },
        });
    }
    let joints = (0..8)
        .map(|k| Joint {
            bone: k + 1,
            bind_inverse: Mat4::from_translation(-Vec3::new(k as f32, 0.0, 0.0)),
        })
        .collect();
    let influences: Vec<Influence> = (0..8)
        .map(|k| Influence {
            joint: k,
            weight: 1.0,
        })
        .collect();
    let rig = Rig::new(
        "八本",
        bones,
        vec![RigMesh {
            mesh: ModelMesh {
                name: "点".into(),
                positions: vec![Vec3::new(2.0, 0.0, 0.0)],
                ..ModelMesh::default()
            },
            skin: Skin {
                joints,
                offsets: vec![0, 8],
                influences,
                fallback: None,
            },
            blend_shapes: Vec::new(),
            node: None,
        }],
        vec![],
        vec![],
        &RigBudget::default(),
    )
    .unwrap();
    let mut pose = rig.rest_pose();
    for k in 0..8 {
        pose.locals[k + 1].translation.y = k as f32;
    }
    let p = &rig.deform(&pose).unwrap()[0];
    assert_close(p.positions[0], Vec3::new(2.0, 3.5, 0.0), "8 本の平均");
    // 予算: 1 頂点 4 本までにすると断る
    let budget = RigBudget {
        max_influences_per_vertex: 4,
        ..RigBudget::default()
    };
    let err = Rig::new(
        "八本",
        rig.bones().to_vec(),
        rig.meshes().to_vec(),
        vec![],
        vec![],
        &budget,
    )
    .unwrap_err();
    assert!(matches!(err, RigError::TooLarge { .. }), "{err}");
}

fn shape_rig(frames: Vec<BlendFrame>) -> Rig {
    let mut rig = arm(&[
        (Vec3::new(1.5, 0.0, 0.0), &[(1, 1.0)]),
        (Vec3::new(0.5, 0.0, 0.0), &[(0, 1.0)]),
    ]);
    let mut meshes = rig.meshes().to_vec();
    meshes[0].blend_shapes.push(BlendShape {
        name: "伸ばす".into(),
        frames,
    });
    rig = Rig::new(
        "腕",
        rig.bones().to_vec(),
        meshes,
        vec!["肌".into()],
        vec![],
        &RigBudget::default(),
    )
    .unwrap();
    rig
}

fn frame(weight: f32, delta: Vec3) -> BlendFrame {
    BlendFrame {
        weight,
        vertices: vec![0],
        positions: vec![delta],
        normals: Vec::new(),
    }
}

#[test]
fn blend_shapes_interpolate_in_between_frames() {
    let rig = shape_rig(vec![frame(50.0, Vec3::X), frame(100.0, Vec3::Y * 2.0)]);
    let base = Vec3::new(1.5, 0.0, 0.0);
    let at = |w: f32| {
        let mut pose = rig.rest_pose();
        pose.blend_weights[0][0] = w;
        rig.deform(&pose).unwrap()[0].positions[0]
    };
    assert_close(at(0.0), base, "0");
    assert_close(
        at(25.0),
        base + Vec3::X * 0.5,
        "最初のフレームまでは 0 からの線形",
    );
    assert_close(at(50.0), base + Vec3::X, "1 つ目のフレーム");
    assert_close(at(75.0), base + Vec3::new(0.5, 1.0, 0.0), "フレームの間");
    assert_close(at(100.0), base + Vec3::Y * 2.0, "2 つ目のフレーム");
    assert_close(
        at(150.0),
        base + Vec3::new(-1.0, 4.0, 0.0),
        "外は端の区間を延ばす",
    );
    assert_eq!(rig.rest_pose().blend_weights, vec![vec![0.0]]);
    let single = shape_rig(vec![frame(100.0, Vec3::Y)]);
    let mut pose = single.rest_pose();
    pose.blend_weights[0][0] = 30.0;
    assert_close(
        single.deform(&pose).unwrap()[0].positions[0],
        base + Vec3::Y * 0.3,
        "1 フレームは比例",
    );
}

#[test]
fn blend_shapes_apply_before_skinning() {
    let rig = shape_rig(vec![frame(100.0, Vec3::Y * 0.2)]);
    let mut pose = rig.rest_pose();
    pose.blend_weights[0][0] = 100.0;
    pose.locals[2].rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    // (1.5, 0.2, 0) を前腕の原点のまわりに 90° → (1 − 0.2, 0.5, 0)
    assert_close(
        rig.deform(&pose).unwrap()[0].positions[0],
        Vec3::new(0.8, 0.5, 0.0),
        "差分も骨と一緒に回る",
    );
}

#[test]
fn rotating_a_bone_around_a_world_axis_follows_the_parent_rotation() {
    let rig = arm(&[(Vec3::new(1.5, 0.0, 0.0), &[(1, 1.0)])]);
    let mut pose = rig.rest_pose();
    pose.locals[1].rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    let world = rig.world_matrices(&pose).unwrap();
    let before = Rig::world_rotation(&world, 2);
    let q = Quat::from_axis_angle(Vec3::Z, 0.7);
    rig.rotate_bone_world(&mut pose, &world, 2, Vec3::Z, 0.7);
    let after = Rig::world_rotation(&rig.world_matrices(&pose).unwrap(), 2);
    assert!(after.abs_diff_eq(q * before, 1e-5) || after.abs_diff_eq(-(q * before), 1e-5));
    assert!((pose.locals[2].rotation.length() - 1.0).abs() < 1e-6);
    // 範囲外の骨・0 の軸・有限でない角度は何もしない
    let keep = pose.clone();
    rig.rotate_bone_world(&mut pose, &world, 99, Vec3::Z, 1.0);
    rig.rotate_bone_world(&mut pose, &world, 1, Vec3::ZERO, 1.0);
    rig.rotate_bone_world(&mut pose, &world, 1, Vec3::Z, f32::NAN);
    assert_eq!(pose, keep);
}

#[test]
fn rigid_skin_moves_the_whole_mesh_with_its_bone() {
    let bones = vec![
        Bone {
            name: "根".into(),
            parent: None,
            rest: BoneTransform::IDENTITY,
        },
        Bone {
            name: "頭".into(),
            parent: Some(0),
            rest: BoneTransform {
                translation: Vec3::Y,
                ..BoneTransform::IDENTITY
            },
        },
    ];
    let mesh = ModelMesh {
        name: "箱".into(),
        positions: vec![Vec3::new(0.0, 1.5, 0.0), Vec3::new(0.5, 1.0, 0.0)],
        ..ModelMesh::default()
    };
    let rig = Rig::new(
        "頭",
        bones,
        vec![RigMesh {
            skin: Skin::rigid(1, Mat4::from_translation(-Vec3::Y), 2),
            mesh,
            blend_shapes: vec![],
            node: None,
        }],
        vec![],
        vec![],
        &RigBudget::default(),
    )
    .unwrap();
    assert!(rig.is_deforming(1) && !rig.is_deforming(0));
    let mut pose = rig.rest_pose();
    pose.locals[1].rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    pose.locals[0].translation = Vec3::new(0.0, 0.0, 2.0);
    let p = &rig.deform(&pose).unwrap()[0];
    assert_close(p.positions[0], Vec3::new(-0.5, 1.0, 2.0), "頭の上");
    assert_close(p.positions[1], Vec3::new(0.0, 1.5, 2.0), "頭の右");
}

#[test]
fn broken_rigs_and_poses_are_refused() {
    let good = arm(&[
        (Vec3::new(0.5, 0.0, 0.0), &[(0, 1.0)]),
        (Vec3::new(1.5, 0.0, 0.0), &[(1, 1.0)]),
        (Vec3::new(1.2, 0.0, 0.0), &[(0, 0.5), (1, 0.5)]),
    ]);
    let make = |bones: Vec<Bone>, meshes: Vec<RigMesh>, budget: RigBudget| {
        Rig::new("x", bones, meshes, vec![], vec![], &budget)
    };
    let (bones, meshes) = (good.bones().to_vec(), good.meshes().to_vec());
    let b = RigBudget::default();
    // 親が後ろ
    let mut bad = bones.clone();
    bad[1].parent = Some(2);
    assert_eq!(
        make(bad, meshes.clone(), b).unwrap_err(),
        RigError::BadParent { bone: 1 }
    );
    // 有限でない骨
    let mut bad = bones.clone();
    bad[2].rest.translation.x = f32::NAN;
    assert!(matches!(
        make(bad, meshes.clone(), b).unwrap_err(),
        RigError::NonFinite { .. }
    ));
    // 関節の骨が範囲外・影響の関節が範囲外・並びの数が違う・負のウェイト
    let mut bad = meshes.clone();
    bad[0].skin.joints[0].bone = 7;
    assert_eq!(
        make(bones.clone(), bad, b).unwrap_err(),
        RigError::BadSkin { mesh: 0 }
    );
    let mut bad = meshes.clone();
    bad[0].skin.influences[0].joint = 5;
    assert_eq!(
        make(bones.clone(), bad, b).unwrap_err(),
        RigError::BadSkin { mesh: 0 }
    );
    let mut bad = meshes.clone();
    bad[0].skin.offsets.pop();
    assert_eq!(
        make(bones.clone(), bad, b).unwrap_err(),
        RigError::BadSkin { mesh: 0 }
    );
    let mut bad = meshes.clone();
    bad[0].skin.influences[0].weight = -1.0;
    assert!(matches!(
        make(bones.clone(), bad, b).unwrap_err(),
        RigError::NonFinite { .. }
    ));
    // 添字が範囲外のメッシュ・有限でない位置
    let mut bad = meshes.clone();
    bad[0].mesh.submeshes[0].indices[0] = 99;
    assert_eq!(
        make(bones.clone(), bad, b).unwrap_err(),
        RigError::BadMesh { mesh: 0 }
    );
    let mut bad = meshes.clone();
    bad[0].mesh.positions[0].y = f32::INFINITY;
    assert!(matches!(
        make(bones.clone(), bad, b).unwrap_err(),
        RigError::NonFinite { .. }
    ));
    // BlendShape: フレームが無い・重みが増えない・頂点が範囲外
    for frames in [
        vec![],
        vec![frame(50.0, Vec3::X), frame(50.0, Vec3::X)],
        vec![frame(0.0, Vec3::X)],
        vec![BlendFrame {
            weight: 100.0,
            vertices: vec![9],
            positions: vec![Vec3::X],
            normals: vec![],
        }],
    ] {
        let mut bad = meshes.clone();
        bad[0].blend_shapes.push(BlendShape {
            name: "壊れた".into(),
            frames,
        });
        assert_eq!(
            make(bones.clone(), bad, b).unwrap_err(),
            RigError::BadBlendShape { mesh: 0, shape: 0 }
        );
    }
    // 予算
    for budget in [
        RigBudget { max_bones: 2, ..b },
        RigBudget {
            max_vertices: 2,
            ..b
        },
        RigBudget {
            max_triangles: 0,
            ..b
        },
        RigBudget {
            max_influences: 3,
            ..b
        },
        RigBudget { max_meshes: 0, ..b },
    ] {
        assert!(matches!(
            make(bones.clone(), meshes.clone(), budget).unwrap_err(),
            RigError::TooLarge { .. }
        ));
    }
    // BlendShape の数・差分の数の予算（BlendShape が 1 つ・差分が 1 つの形で、ちょうどなら通り、1 つ少ないと断る）
    let mut with_shape = meshes.clone();
    with_shape[0].blend_shapes.push(BlendShape {
        name: "形".into(),
        frames: vec![frame(100.0, Vec3::X)],
    });
    assert!(make(bones.clone(), with_shape.clone(), b).is_ok());
    for (budget, what) in [
        (
            RigBudget {
                max_blend_shapes: 1,
                ..b
            },
            "BlendShape",
        ),
        (
            RigBudget {
                max_blend_offsets: 1,
                ..b
            },
            "BlendShape の差分",
        ),
    ] {
        assert!(
            make(bones.clone(), with_shape.clone(), budget).is_ok(),
            "{what}: ちょうど上限なら通る"
        );
    }
    assert_eq!(
        make(
            bones.clone(),
            with_shape.clone(),
            RigBudget {
                max_blend_shapes: 0,
                ..b
            }
        )
        .unwrap_err(),
        RigError::TooLarge {
            what: "BlendShape",
            value: 1,
            limit: 0
        }
    );
    assert_eq!(
        make(
            bones.clone(),
            with_shape,
            RigBudget {
                max_blend_offsets: 0,
                ..b
            }
        )
        .unwrap_err(),
        RigError::TooLarge {
            what: "BlendShape の差分",
            value: 1,
            limit: 0
        }
    );
    // ポーズの形が違う・有限でない
    let mut pose = good.rest_pose();
    pose.locals.pop();
    assert_eq!(good.deform(&pose).unwrap_err(), RigError::PoseMismatch);
    let mut pose = good.rest_pose();
    pose.blend_weights[0].push(1.0);
    assert_eq!(good.deform(&pose).unwrap_err(), RigError::PoseMismatch);
    let mut pose = good.rest_pose();
    pose.locals[1].rotation.x = f32::NAN;
    assert!(matches!(
        good.deform(&pose).unwrap_err(),
        RigError::NonFinite { .. }
    ));
}

/// 1 頂点ずつの素朴な式（並列の塊の境目を確かめる）。
fn reference(rig: &Rig, pose: &Pose) -> Vec<Vec<Vec3>> {
    let world = rig.world_matrices(pose).unwrap();
    rig.meshes()
        .iter()
        .zip(&pose.blend_weights)
        .map(|(m, weights)| {
            (0..m.mesh.positions.len())
                .map(|v| {
                    let mut p = m.mesh.positions[v];
                    for (s, &w) in m.blend_shapes.iter().zip(weights) {
                        for (f, k) in s.frame_factors(w) {
                            if k == 0.0 {
                                continue;
                            }
                            let fr = &s.frames[f];
                            if let Some(i) = fr.vertices.iter().position(|&x| x as usize == v) {
                                p += fr.positions[i] * k;
                            }
                        }
                    }
                    let list = m.skin.influences_of(v);
                    if list.is_empty() {
                        return match m.skin.fallback {
                            Some(f) => {
                                let j = m.skin.joints[f as usize];
                                (world[j.bone as usize] * j.bind_inverse).transform_point3(p)
                            }
                            None => p,
                        };
                    }
                    list.iter()
                        .map(|i| {
                            let j = m.skin.joints[i.joint as usize];
                            (world[j.bone as usize] * j.bind_inverse).transform_point3(p) * i.weight
                        })
                        .sum()
                })
                .collect()
        })
        .collect()
}

fn bent(rig: &Rig) -> Pose {
    let mut pose = rig.rest_pose();
    let find = |name: &str| rig.bones().iter().position(|b| b.name == name).unwrap();
    pose.locals[find("右上腕")].rotation = Quat::from_rotation_z(-1.2);
    pose.locals[find("右前腕")].rotation = Quat::from_rotation_y(0.9);
    pose.locals[find("左太もも")].rotation = Quat::from_rotation_x(-0.8);
    pose.locals[find("背骨")].rotation = Quat::from_rotation_x(0.3);
    pose.blend_weights[0][0] = 80.0;
    let last = pose.blend_weights.len() - 1;
    pose.blend_weights[last][0] = 75.0;
    pose
}

#[test]
fn parallel_deform_matches_the_per_vertex_formula_on_the_figure() {
    let rig = demo_figure(FigureDetail {
        sides: 32,
        ring_spacing: 0.005,
        head: 12,
    });
    assert!(
        rig.meshes().iter().any(|m| m.mesh.positions.len() > 4096),
        "並列の塊をまたぐ大きさ"
    );
    let pose = bent(&rig);
    let posed = rig.deform(&pose).unwrap();
    for (m, (got, want)) in posed.iter().zip(reference(&rig, &pose)).enumerate() {
        for (v, (a, b)) in got.positions.iter().zip(&want).enumerate() {
            assert!(
                (*a - *b).length() < 1e-5,
                "メッシュ {m} 頂点 {v}: {a} と {b}"
            );
        }
    }
}

#[test]
fn demo_figure_is_well_formed() {
    let rig = demo_figure(FigureDetail::SMALL);
    assert_eq!(rig.materials(), ["肌", "顔"]);
    assert_eq!(rig.meshes().len(), 6);
    assert_eq!(rig.blend_shape_count(), 2);
    assert!(rig.roots().eq([0]));
    let posed = rig.deform(&rig.rest_pose()).unwrap();
    for (m, p) in rig.meshes().iter().zip(&posed) {
        for (a, b) in m.mesh.positions.iter().zip(&p.positions) {
            assert!(
                (*a - *b).length() < 1e-5,
                "{}: 休みのポーズはバインドの形",
                m.mesh.name
            );
        }
        // 面の法線は外（頂点の法線の向き）を向く
        for t in m.mesh.submeshes[0].indices.chunks_exact(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| m.mesh.positions[i as usize]);
            let face = (b - a).cross(c - a);
            let n = m.mesh.normals[t[0] as usize]
                + m.mesh.normals[t[1] as usize]
                + m.mesh.normals[t[2] as usize];
            assert!(face.dot(n) > 0.0, "{} の面が裏返っている", m.mesh.name);
        }
        assert!(m
            .mesh
            .uvs
            .iter()
            .all(|uv| uv.cmpge(Vec2::ZERO).all() && uv.cmple(Vec2::ONE).all()));
    }
    // 7 万三角形の人形
    let big = demo_figure(FigureDetail::AVATAR);
    let n = big.triangle_count();
    assert!((65_000..75_000).contains(&n), "{n} 三角形");
}

#[test]
fn the_bone_under_a_triangle_is_the_strongest_weight_of_the_nearest_corner() {
    let rig = demo_figure(FigureDetail::SMALL);
    let triangles =
        crate::geometry::model_triangles(&rig.deform(&rig.rest_pose()).unwrap()).unwrap();
    let find = |name: &str| rig.bones().iter().position(|b| b.name == name).unwrap();
    // 右腕の筒の手の先の三角形は「右手」、頭の球は「頭」（固く付いた fallback）
    let torso = rig.meshes()[0].mesh.triangle_count();
    let arm = rig.meshes()[1].mesh.triangle_count();
    let tip = (torso + arm - 1) as u32;
    assert_eq!(
        rig.bone_at_triangle(tip, Vec3::new(1.0, 0.0, 0.0)),
        Some(find("右手"))
    );
    let shoulder = torso as u32;
    assert_eq!(
        rig.bone_at_triangle(shoulder, Vec3::new(0.0, 1.0, 0.0)),
        Some(find("右上腕"))
    );
    let head = (triangles.len() - 1) as u32;
    assert_eq!(
        rig.bone_at_triangle(head, Vec3::new(0.2, 0.3, 0.5)),
        Some(find("頭"))
    );
    assert_eq!(
        rig.bone_at_triangle(triangles.len() as u32, Vec3::X),
        None,
        "範囲外"
    );
}
