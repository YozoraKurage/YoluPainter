//! FBX の読み込みの試験。フィクスチャはコードで組んだ小さな ASCII の FBX（tests/common/fbx_ascii.rs）だけで、実のデータは使わない。
//! 正解は (1) 手で求めた値（座標の反転・巡り・UV・数）と (2) ufbx 自身の評価（evaluate_skinning。休みの形と、骨の変換・
//! BlendShape の重みを上書きしたポーズ）を、Unity の座標に反転したもの。
mod common;

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use common::fbx_ascii::{arm_scene, box_tube, Node, Scene};
use yolu_core::glam::{DVec3, Quat, Vec3};
use yolu_core::skin::{Pose, RigBudget, RigError};
use yolu_model::{
    load_fbx, load_fbx_bytes, load_fbx_bytes_with, load_fbx_with, LoadControl, ModelError,
    ModelLimits,
};

fn mirror(v: DVec3) -> Vec3 {
    Vec3::new(-v.x as f32, v.y as f32, v.z as f32)
}

fn load(scene: &Scene) -> yolu_model::LoadedModel {
    load_fbx_bytes(scene.to_ascii().as_bytes(), "腕", &ModelLimits::default()).expect("読める")
}

/// ufbx の評価（メッシュの名前ごとに、コントロールポイントのワールドの位置）。overrides は（ノードの名前・回転）、
/// blend は（チャンネルの名前・DeformPercent）。
fn ufbx_positions(
    text: &str,
    overrides: &[(&str, ufbx::Quat)],
    blend: &[(&str, f64)],
) -> Vec<(String, Vec<DVec3>)> {
    let scene = ufbx::load_memory(text.as_bytes(), ufbx::LoadOpts::default()).expect("ufbx");
    let transform_overrides: Vec<ufbx::TransformOverride> = overrides
        .iter()
        .map(|(name, q)| {
            let n = scene
                .nodes
                .iter()
                .find(|n| &*n.element.name == *name)
                .expect("ノード");
            ufbx::TransformOverride {
                node_id: n.element.typed_id,
                transform: ufbx::Transform {
                    rotation: *q,
                    ..n.local_transform
                },
            }
        })
        .collect();
    let prop_overrides: Vec<ufbx::PropOverrideDesc> = blend
        .iter()
        .map(|(name, percent)| {
            let ch = scene
                .blend_channels
                .iter()
                .find(|c| &*c.element.name == *name)
                .expect("チャンネル");
            ufbx::PropOverrideDesc {
                element_id: ch.element.element_id,
                prop_name: "DeformPercent".into(),
                value: ufbx::Vec4 {
                    x: *percent,
                    y: 0.0,
                    z: 0.0,
                    w: 0.0,
                },
                value_str: Default::default(),
                value_int: *percent as i64,
            }
        })
        .collect();
    let anim = ufbx::create_anim(
        &scene,
        ufbx::AnimOpts {
            transform_overrides: transform_overrides.into(),
            prop_overrides: prop_overrides.into(),
            ..Default::default()
        },
    )
    .expect("anim");
    let evaluated = ufbx::evaluate_scene(
        &scene,
        &anim,
        0.0,
        ufbx::EvaluateOpts {
            evaluate_skinning: true,
            ..Default::default()
        },
    )
    .expect("evaluate");
    let mut out = Vec::new();
    for n in evaluated.nodes.iter() {
        let Some(mesh) = n.mesh.as_ref() else {
            continue;
        };
        let to_world = |p: ufbx::Vec3| {
            let p = DVec3::new(p.x, p.y, p.z);
            if mesh.skinned_is_local {
                let m = &n.geometry_to_world;
                DVec3::new(
                    m.m00 * p.x + m.m01 * p.y + m.m02 * p.z + m.m03,
                    m.m10 * p.x + m.m11 * p.y + m.m12 * p.z + m.m13,
                    m.m20 * p.x + m.m21 * p.y + m.m22 * p.z + m.m23,
                )
            } else {
                p
            }
        };
        let positions = (0..mesh.num_vertices)
            .map(|cp| to_world(mesh.skinned_position.values[cp]))
            .collect();
        out.push((n.element.name.to_string(), positions));
    }
    out
}

/// 自分の頂点 → ufbx のコントロールポイント（休みの形の位置で対応づける。試しの腕は位置が重ならない）。
fn match_vertices(mine: &[Vec3], theirs: &[DVec3]) -> Vec<usize> {
    mine.iter()
        .map(|p| {
            let (cp, d) = theirs
                .iter()
                .enumerate()
                .map(|(i, q)| (i, (mirror(*q) - *p).length()))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap();
            assert!(d < 1e-5, "対応するコントロールポイントが無い: {p}");
            cp
        })
        .collect()
}

#[test]
fn the_arm_reads_with_unity_axes_winding_uvs_and_materials() {
    let scene = arm_scene();
    let m = load(&scene);
    let rig = &m.rig;
    assert_eq!(m.report.meshes, 2);
    assert_eq!(m.report.skinned_meshes, 1);
    assert_eq!(m.report.triangles, 4 * 4 * 2 + 1);
    assert_eq!(
        m.report.bones, 6,
        "根・Armature・Upper・Lower・ArmMesh・Hat"
    );
    assert_eq!(m.report.blend_shapes, 2);
    assert!(m.report.warnings.is_empty(), "{:?}", m.report.warnings);
    assert!(m.report.max_transform_error < 1e-6);
    assert_eq!(rig.materials(), ["Skin", "Cloth"]);
    assert_eq!(rig.bones()[0].name, "腕", "根はモデルの名前");
    let names: Vec<&str> = rig.bones().iter().map(|b| b.name.as_str()).collect();
    assert_eq!(
        names,
        ["腕", "Armature", "Upper", "Lower", "Hat", "ArmMesh"]
    );
    let upper = rig.bones().iter().position(|b| b.name == "Upper").unwrap();
    assert_eq!(
        rig.bones()[upper].rest.translation,
        Vec3::new(0.0, 1.0, 0.0)
    );

    let arm = &rig.meshes()[1].mesh;
    assert_eq!(arm.name, "ArmMesh");
    // 面ごとに平らな法線と別の UV なので、20 のコントロールポイントが分かれる（4 面 × 輪 5 × 2 = 40）
    assert_eq!(arm.positions.len(), 40);
    // X の反転: ファイルの (2, 1.1, 0.1) は (−2, 1.1, 0.1)
    assert!(arm.positions.contains(&Vec3::new(-2.0, 1.1, 0.1)));
    assert!(arm.positions.iter().all(|p| p.x <= 0.0));
    // マテリアルごとのサブメッシュ（Skin = 0 は上と手前の面、Cloth = 1）
    assert_eq!(
        arm.submeshes
            .iter()
            .map(|s| (s.material, s.indices.len() / 3))
            .collect::<Vec<_>>(),
        vec![(0, 16), (1, 16)]
    );
    // 巡り: 面の法線 (B − A) × (C − A) が外（頂点の法線の向き）を向く = Unity の表
    for s in &arm.submeshes {
        for t in s.indices.as_chunks::<3>().0 {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| arm.positions[i as usize]);
            let face = (b - a).cross(c - a).normalize();
            assert!(face.dot(arm.normals[t[0] as usize]) > 0.99, "外向き");
            // 法線も反転されている: 手前（+Z）の面はそのまま、上下は Y
            assert!(arm.normals[t[0] as usize].x.abs() < 1e-6);
        }
    }
    // UV はそのまま（u = x / 2）
    for (p, uv) in arm.positions.iter().zip(&arm.uvs) {
        assert!((uv.x - (-p.x) / 2.0).abs() < 1e-6, "{p} {uv}");
    }
    // Hat はスキンの無いメッシュ: 自分のノード（Lower の子）に固く付く。法線の無いメッシュは ufbx が作る
    let hat = &rig.meshes()[0];
    assert_eq!(hat.mesh.name, "Hat");
    assert_eq!(hat.mesh.normals.len(), 3);
    let lower = rig.bones().iter().position(|b| b.name == "Lower").unwrap();
    let hat_bone = hat.skin.joints[0].bone as usize;
    assert_eq!(rig.bones()[hat_bone].name, "Hat");
    assert_eq!(rig.bones()[hat_bone].parent, Some(lower as u32));
    assert!(hat.skin.influences.is_empty());
    // BlendShape: Thick は DeformPercent 25 で休み、Bend は中間のフレーム 50・100
    let shapes = &rig.meshes()[1].blend_shapes;
    assert_eq!(shapes[0].name, "Thick");
    assert_eq!(rig.rest_pose().blend_weights[1], vec![25.0, 0.0]);
    assert_eq!(
        shapes[1]
            .frames
            .iter()
            .map(|f| f.weight)
            .collect::<Vec<_>>(),
        vec![50.0, 100.0]
    );
    // ウェイト: x = 1 の輪は半々
    let skin = &rig.meshes()[1].skin;
    for (v, p) in arm.positions.iter().enumerate() {
        let w = skin.influences_of(v);
        if (p.x + 1.0).abs() < 1e-6 {
            assert_eq!(w.len(), 2);
            assert!(w.iter().all(|i| (i.weight - 0.5).abs() < 1e-6));
        } else {
            assert_eq!(w.len(), 1);
        }
    }
}

fn compare(
    rig: &yolu_core::skin::Rig,
    pose: &Pose,
    rest: &[(String, Vec<DVec3>)],
    posed: &[(String, Vec<DVec3>)],
) {
    let rest_meshes = rig.deform(&rig.rest_pose()).unwrap();
    let meshes = rig.deform(pose).unwrap();
    for ((m, rest_m), mine) in rig.meshes().iter().zip(&rest_meshes).zip(&meshes) {
        let theirs_rest = &rest.iter().find(|(n, _)| *n == m.mesh.name).unwrap().1;
        let theirs = &posed.iter().find(|(n, _)| *n == m.mesh.name).unwrap().1;
        let cps = match_vertices(&rest_m.positions, theirs_rest);
        for (v, cp) in cps.iter().enumerate() {
            let want = mirror(theirs[*cp]);
            assert!(
                (mine.positions[v] - want).length() < 1e-4,
                "{} 頂点 {v}: {} と ufbx の {}",
                m.mesh.name,
                mine.positions[v],
                want
            );
        }
    }
}

#[test]
fn rest_and_posed_shapes_match_ufbx_evaluation() {
    let scene = arm_scene();
    let text = scene.to_ascii();
    let m = load(&scene);
    let rig = &m.rig;
    let rest = ufbx_positions(&text, &[], &[]);
    // 休みの形（Thick が 25 で効いている）
    compare(rig, &rig.rest_pose(), &rest, &rest);
    // Lower を z のまわりに 90°、Upper を y のまわりに 30°、Thick 60・Bend 75（中間のフレームの間）
    let (s45, c45) = (
        std::f64::consts::FRAC_PI_4.sin(),
        std::f64::consts::FRAC_PI_4.cos(),
    );
    let (s15, c15) = (15f64.to_radians().sin(), 15f64.to_radians().cos());
    let lower_q = ufbx::Quat {
        x: 0.0,
        y: 0.0,
        z: s45,
        w: c45,
    };
    let upper_q = ufbx::Quat {
        x: 0.0,
        y: s15,
        z: 0.0,
        w: c15,
    };
    let posed = ufbx_positions(
        &text,
        &[("Lower", lower_q), ("Upper", upper_q)],
        &[("Thick", 60.0), ("Bend", 75.0)],
    );
    let mut pose = rig.rest_pose();
    let find = |name: &str| rig.bones().iter().position(|b| b.name == name).unwrap();
    // Unity の座標の回転は X の反転: (x, −y, −z, w)
    let mirror_q =
        |q: ufbx::Quat| Quat::from_xyzw(q.x as f32, -q.y as f32, -q.z as f32, q.w as f32);
    pose.locals[find("Lower")].rotation = mirror_q(lower_q);
    pose.locals[find("Upper")].rotation = mirror_q(upper_q);
    pose.blend_weights[1] = vec![60.0, 75.0];
    compare(rig, &pose, &rest, &posed);
}

#[test]
fn broken_files_are_refused() {
    let text = arm_scene().to_ascii();
    let limits = ModelLimits::default();
    let cut = &text.as_bytes()[..text.len() / 2];
    for (what, data) in [
        ("空", &b""[..]),
        ("途中で切れた", cut),
        (
            "でたらめ",
            &b"Kaydara FBX Binary  \x00\x1a\x00\xff\xff\xff\xff\x00garbage"[..],
        ),
        ("FBX でない", &b"hello, this is not a model at all"[..]),
    ] {
        match load_fbx_bytes(data, "x", &limits) {
            Err(ModelError::Parse(_)) | Err(ModelError::NoMesh) => {}
            other => panic!("{what}: {other:?}"),
        }
    }
    // メッシュの無い FBX
    let empty = Scene {
        nodes: vec![Node::new("Only", None, [0.0; 3], true)],
        ..Scene::default()
    };
    assert_eq!(
        load_fbx_bytes(empty.to_ascii().as_bytes(), "x", &limits).unwrap_err(),
        ModelError::NoMesh
    );
}

#[test]
fn too_large_files_are_refused_by_the_limits() {
    let text = arm_scene().to_ascii();
    let data = text.as_bytes();
    let small_file = ModelLimits {
        max_file_bytes: 1024,
        ..ModelLimits::default()
    };
    assert!(matches!(
        load_fbx_bytes(data, "x", &small_file).unwrap_err(),
        ModelError::FileTooLarge { .. }
    ));
    let small_memory = ModelLimits {
        parser_memory_bytes: 16 * 1024,
        ..ModelLimits::default()
    };
    assert!(matches!(
        load_fbx_bytes(data, "x", &small_memory).unwrap_err(),
        ModelError::Parse(_)
    ));
    for rig in [
        RigBudget {
            max_triangles: 10,
            ..RigBudget::default()
        },
        RigBudget {
            max_bones: 3,
            ..RigBudget::default()
        },
        RigBudget {
            max_vertices: 10,
            ..RigBudget::default()
        },
        RigBudget {
            max_blend_offsets: 4,
            ..RigBudget::default()
        },
        RigBudget {
            max_influences: 10,
            ..RigBudget::default()
        },
    ] {
        let limits = ModelLimits {
            rig,
            ..ModelLimits::default()
        };
        match load_fbx_bytes(data, "x", &limits).unwrap_err() {
            ModelError::Rig(RigError::TooLarge { .. }) => {}
            other => panic!("{other:?}"),
        }
    }
    // 深すぎるノードの木
    let mut deep = Scene::default();
    for i in 0..40usize {
        deep.nodes.push(Node::new(
            &format!("n{i}"),
            i.checked_sub(1),
            [0.0, 0.1, 0.0],
            true,
        ));
    }
    deep.meshes.push(box_tube("m", 39, 1.0, 0.0, 0.1, 2));
    let shallow = ModelLimits {
        max_node_depth: 8,
        ..ModelLimits::default()
    };
    assert!(load_fbx_bytes(deep.to_ascii().as_bytes(), "x", &ModelLimits::default()).is_ok());
    assert!(matches!(
        load_fbx_bytes(deep.to_ascii().as_bytes(), "x", &shallow).unwrap_err(),
        ModelError::Parse(_)
    ));
}

/// 試しの腕のスキンと BlendShape のあるメッシュ（ArmMesh）を、count 個のノードからも参照する（FBX のインスタンス）。
fn instanced_arm(count: usize) -> String {
    let mut scene = arm_scene();
    let first = scene.nodes.len();
    for i in 0..count {
        scene
            .nodes
            .push(Node::new(&format!("Inst{i}"), None, [0.0; 3], false));
    }
    // ArmMesh の形（Geometry）の番号は 400000。同じ形をつなぐ
    let extra: String = (0..count)
        .map(|i| format!("\tC: \"OO\",400000,{}\n", 100_000 + first + i))
        .collect();
    scene
        .to_ascii()
        .replace("Connections:  {\n", &format!("Connections:  {{\n{extra}"))
}

#[test]
fn instances_count_toward_the_whole_weight_and_blend_budgets() {
    let default = ModelLimits::default();
    let single = load_fbx_bytes(instanced_arm(0).as_bytes(), "x", &default).unwrap();
    let arm = single
        .rig
        .meshes()
        .iter()
        .find(|m| m.mesh.name == "ArmMesh")
        .unwrap();
    let influences = arm.skin.influences.len();
    let offsets: usize = arm
        .blend_shapes
        .iter()
        .flat_map(|s| &s.frames)
        .map(|f| f.vertices.len())
        .sum();
    assert!(influences > 8 && offsets > 8, "{influences} {offsets}");
    // 上限の内なら、インスタンスを重ねても読める
    let many = load_fbx_bytes(instanced_arm(4).as_bytes(), "x", &default).unwrap();
    assert_eq!(many.report.meshes, 5 + 1, "ArmMesh が 5 つと Hat");
    let total = |what: &str| match what {
        "ウェイト" => influences * 5,
        _ => offsets * 5,
    };
    // 1 つ分は上限の内でも、全体では超える: メッシュごとの数え直しでは通り、全部を組み終えてから断られる。全体の累計なら、
    // 超えた所で止まるので、断る値は全部を組んだときの和より小さい
    for (what, rig) in [
        (
            "ウェイト",
            RigBudget {
                max_influences: influences * 2 + 1,
                ..RigBudget::default()
            },
        ),
        (
            "BlendShape の差分",
            RigBudget {
                max_blend_offsets: offsets * 2 + 1,
                ..RigBudget::default()
            },
        ),
    ] {
        let limit = if what == "ウェイト" {
            rig.max_influences
        } else {
            rig.max_blend_offsets
        };
        let limits = ModelLimits {
            rig,
            ..ModelLimits::default()
        };
        match load_fbx_bytes(instanced_arm(4).as_bytes(), "x", &limits).unwrap_err() {
            ModelError::Rig(RigError::TooLarge {
                what: w,
                value,
                limit: l,
            }) => {
                assert_eq!(w, what);
                assert_eq!(l, limit);
                assert!(value > limit, "{what}: {value}");
                assert!(
                    value < total(what),
                    "{what}: 組み終える前に止まる（{value} < {}）",
                    total(what)
                );
            }
            other => panic!("{what}: {other:?}"),
        }
        // 1 つ分だけなら同じ上限でも読める
        assert!(load_fbx_bytes(instanced_arm(1).as_bytes(), "x", &limits).is_ok());
    }
}

#[test]
fn reading_a_file_leaves_it_unchanged() {
    let dir = std::env::temp_dir().join(format!("yolu-model-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("腕.fbx");
    let text = arm_scene().to_ascii();
    std::fs::write(&path, &text).unwrap();
    // 更新時刻を少し前にしておく（書き直されたら今の時刻になって食い違う。時刻の粒度で同じ値に見えて見逃さない）
    let before = std::time::SystemTime::now() - std::time::Duration::from_secs(600);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(before)
        .unwrap();
    let m = load_fbx(&path, &ModelLimits::default()).unwrap();
    assert_eq!(m.rig.name(), "腕", "名前はファイル名");
    assert_eq!(std::fs::read(&path).unwrap(), text.as_bytes());
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        before
    );
    let small = ModelLimits {
        max_file_bytes: 100,
        ..ModelLimits::default()
    };
    assert!(matches!(
        load_fbx(&path, &small).unwrap_err(),
        ModelError::FileTooLarge { .. }
    ));
    assert!(matches!(
        load_fbx(&dir.join("無い.fbx"), &ModelLimits::default()).unwrap_err(),
        ModelError::Io(_)
    ));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn centimetres_and_z_up_files_read_to_the_same_metres_and_y_up() {
    let base = arm_scene();
    let m = load(&base);
    let rest = m.rig.deform(&m.rig.rest_pose()).unwrap();
    let origins = |rig: &yolu_core::skin::Rig| {
        let w = rig.world_matrices(&rig.rest_pose()).unwrap();
        (0..rig.bones().len())
            .map(|b| yolu_core::skin::Rig::bone_origin(&w, b))
            .collect::<Vec<_>>()
    };
    // センチメートル: 同じ形を 100 倍の数で書く
    let mut cm = base.transformed(|p| p, 100.0);
    cm.centimeters = true;
    // Z が上・−Y が前: (x, y, z) → (x, −z, y)
    let mut z_up = base.transformed(|p| [p[0], -p[2], p[1]], 1.0);
    z_up.z_up = true;
    for (what, scene) in [("cm", cm), ("Z が上", z_up)] {
        let other = load(&scene);
        assert!(other.report.max_transform_error < 1e-5, "{what}");
        let posed = other.rig.deform(&other.rig.rest_pose()).unwrap();
        for (a, b) in rest.iter().zip(&posed) {
            assert_eq!(a.positions.len(), b.positions.len());
            for (p, q) in a.positions.iter().zip(&b.positions) {
                assert!((*p - *q).length() < 1e-5, "{what}: {p} と {q}");
            }
            for (p, q) in a.normals.iter().zip(&b.normals) {
                assert!((*p - *q).length() < 1e-5, "{what} の法線: {p} と {q}");
            }
        }
        for (p, q) in origins(&m.rig).iter().zip(origins(&other.rig)) {
            assert!((*p - q).length() < 1e-5, "{what} の骨: {p} と {q}");
        }
    }
}

// ---- 読み込みの取り消しと進み具合 ----

/// 数 MB の ASCII の FBX（四角い筒 1 つ。ufbx の解析の呼び返しが何百回も来る大きさ。スキンは無い）。
fn big_fbx() -> String {
    let mut scene = Scene {
        nodes: vec![Node::new("Big", None, [0.0; 3], false)],
        materials: vec!["A".into(), "B".into()],
        ..Scene::default()
    };
    let mut tube = box_tube("Big", 0, 100.0, 0.0, 0.5, 3000);
    tube.materials = vec![0, 1];
    scene.meshes.push(tube);
    scene.to_ascii()
}

/// 進み具合を全部覚え、`stop_at` に届いたら旗を立てる（その時点の呼び返しの中で立てるので、止まる場所が決まる）。
struct Probe {
    cancel: AtomicBool,
    seen: RefCell<Vec<f32>>,
    stop_at: Cell<Option<f32>>,
}

impl Probe {
    fn new(stop_at: Option<f32>) -> Probe {
        Probe {
            cancel: AtomicBool::new(false),
            seen: RefCell::new(Vec::new()),
            stop_at: Cell::new(stop_at),
        }
    }
    fn note(&self, f: f32) {
        self.seen.borrow_mut().push(f);
        if self.stop_at.get().is_some_and(|at| f >= at) {
            self.cancel.store(true, Ordering::Relaxed);
        }
    }
    fn last(&self) -> f32 {
        self.seen.borrow().last().copied().unwrap_or(-1.0)
    }
}

fn run_bytes(data: &[u8], probe: &Probe) -> Result<yolu_model::LoadedModel, ModelError> {
    let note = |f: f32| probe.note(f);
    load_fbx_bytes_with(
        data,
        "大",
        &ModelLimits::default(),
        LoadControl {
            cancel: Some(&probe.cancel),
            progress: Some(&note),
        },
    )
}

#[test]
fn progress_reports_from_zero_to_one_without_going_back() {
    let text = big_fbx();
    let probe = Probe::new(None);
    let model = run_bytes(text.as_bytes(), &probe).expect("読める");
    assert!(model.rig.triangle_count() > 20000);
    let seen = probe.seen.borrow();
    assert_eq!(seen.first(), Some(&0.0));
    assert_eq!(seen.last(), Some(&1.0));
    assert!(
        seen.windows(2).all(|w| w[0] < w[1]),
        "戻らず、同じ値を繰り返さない"
    );
    // 解析の間にも細かく来る（解析の終わりの 1 回だけではない）
    assert!(
        seen.iter().filter(|f| **f > 0.05 && **f < 0.6).count() > 20,
        "{}",
        seen.len()
    );
    // 間引かれる（面ごと・16 KiB ごとに全部は来ない）
    assert!(seen.len() < 700, "{}", seen.len());
}

#[test]
fn cancelling_during_the_parse_stops_there_and_returns_nothing() {
    let text = big_fbx();
    let probe = Probe::new(Some(0.2));
    let result = run_bytes(text.as_bytes(), &probe);
    assert!(
        matches!(result, Err(ModelError::Cancelled)),
        "{:?}",
        result.err()
    );
    // 旗を立てた呼び返しの次の区切りで止まり、解析の残りも変換も走らない
    assert!(probe.last() < 0.3, "{}", probe.last());
    assert_eq!(ModelError::Cancelled.to_string(), "取り消しました");
}

#[test]
fn cancelling_during_the_conversion_stops_before_the_rig_is_built() {
    let text = big_fbx();
    // 解析が終わったあとの変換の途中（面の区切り）で止める
    let probe = Probe::new(Some(0.7));
    let result = run_bytes(text.as_bytes(), &probe);
    assert!(
        matches!(result, Err(ModelError::Cancelled)),
        "{:?}",
        result.err()
    );
    assert!(
        probe.last() >= 0.7 && probe.last() < 0.98,
        "{}",
        probe.last()
    );
    assert!(
        !probe.seen.borrow().contains(&1.0),
        "終わりの知らせは来ない"
    );
}

#[test]
fn a_flag_that_is_already_up_stops_before_anything_is_read() {
    let probe = Probe::new(None);
    probe.cancel.store(true, Ordering::Relaxed);
    let text = arm_scene().to_ascii();
    let result = run_bytes(text.as_bytes(), &probe);
    assert!(matches!(result, Err(ModelError::Cancelled)));
    assert!(probe.seen.borrow().is_empty(), "何も始めない");

    let dir = std::env::temp_dir().join(format!("yolu-model-cancel-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("腕.fbx");
    std::fs::write(&path, &text).unwrap();
    let result = load_fbx_with(
        &path,
        &ModelLimits::default(),
        LoadControl {
            cancel: Some(&probe.cancel),
            progress: None,
        },
    );
    assert!(matches!(result, Err(ModelError::Cancelled)));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn cancelling_while_the_file_is_read_stops_before_the_parse() {
    let dir = std::env::temp_dir().join(format!("yolu-model-cancel-read-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("大.fbx");
    std::fs::write(&path, big_fbx()).unwrap();
    // 読み始めの知らせ（0.0）の呼び返しで旗を立てる: 最初の塊を読んだあとの区切りで止まる
    let probe = Probe::new(Some(0.0));
    let note = |f: f32| probe.note(f);
    let result = load_fbx_with(
        &path,
        &ModelLimits::default(),
        LoadControl {
            cancel: Some(&probe.cancel),
            progress: Some(&note),
        },
    );
    assert!(
        matches!(result, Err(ModelError::Cancelled)),
        "{:?}",
        result.err()
    );
    assert!(probe.last() <= 0.05, "解析に進まない: {}", probe.last());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn loading_from_a_file_reports_progress_and_matches_loading_the_bytes() {
    let dir = std::env::temp_dir().join(format!("yolu-model-progress-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("腕.fbx");
    let text = arm_scene().to_ascii();
    std::fs::write(&path, &text).unwrap();
    let probe = Probe::new(None);
    let note = |f: f32| probe.note(f);
    let from_file = load_fbx_with(
        &path,
        &ModelLimits::default(),
        LoadControl {
            cancel: Some(&probe.cancel),
            progress: Some(&note),
        },
    )
    .unwrap();
    let from_bytes = load_fbx(&path, &ModelLimits::default()).unwrap();
    assert_eq!(from_file.report.triangles, from_bytes.report.triangles);
    assert_eq!(from_file.rig.name(), "腕");
    assert_eq!(probe.seen.borrow().first(), Some(&0.0));
    assert_eq!(probe.seen.borrow().last(), Some(&1.0));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 別のスレッドから取り消す: 読み込みのスレッドは、旗が立つまで解析の途中で待ち、立った次の区切りで止まる。
#[test]
fn another_thread_can_cancel_a_load_that_is_in_the_middle_of_the_parse() {
    let text = big_fbx();
    let cancel = AtomicBool::new(false);
    let reached = AtomicBool::new(false);
    let last = AtomicU32::new(0);
    let result = std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            let note = |f: f32| {
                last.store((f * 1000.0) as u32, Ordering::Relaxed);
                if f >= 0.2 {
                    reached.store(true, Ordering::Release);
                    // 取り消しが来るまで進まない（遅い読み込みの代わり。最大 20 秒で諦める）
                    let wait = std::time::Instant::now();
                    while !cancel.load(Ordering::Relaxed) && wait.elapsed().as_secs() < 20 {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                }
            };
            load_fbx_bytes_with(
                text.as_bytes(),
                "大",
                &ModelLimits::default(),
                LoadControl {
                    cancel: Some(&cancel),
                    progress: Some(&note),
                },
            )
            .map(|m| m.rig.triangle_count())
        });
        while !reached.load(Ordering::Acquire) {
            assert!(!worker.is_finished(), "取り消す前に終わった");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        cancel.store(true, Ordering::Relaxed);
        worker.join().unwrap()
    });
    assert!(matches!(result, Err(ModelError::Cancelled)), "{result:?}");
    assert!(
        last.load(Ordering::Relaxed) < 300,
        "{}",
        last.load(Ordering::Relaxed)
    );
}

/// 呼び返しの中の panic は、ufbx の C の関数を抜けずに、解析が戻ってから呼び手へそのまま出る（異常終了にしない）。
#[test]
fn a_panic_in_the_progress_callback_reaches_the_caller_after_the_parse_unwinds() {
    let text = big_fbx();
    let note = |f: f32| {
        if f >= 0.2 {
            panic!("試験の panic");
        }
    };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        load_fbx_bytes_with(
            text.as_bytes(),
            "大",
            &ModelLimits::default(),
            LoadControl {
                cancel: None,
                progress: Some(&note),
            },
        )
    }));
    let payload = outcome.expect_err("panic が出る");
    assert_eq!(payload.downcast_ref::<&str>(), Some(&"試験の panic"));
}
