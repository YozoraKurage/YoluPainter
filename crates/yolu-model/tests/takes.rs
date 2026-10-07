//! FBX のテイク（アニメのスタック）の試験。フィクスチャはコードで組んだ ASCII の FBX（tests/common/fbx_ascii.rs）だけ。
//! 正解は (1) 手で求めた値（線形のキーの間・定数のキー・一覧の始まりと終わり）と (2) ufbx 自身の評価（`evaluate_scene` で
//! スキニングまで。3 次のキーの間・範囲の外・BlendShape の重み）を、Unity の座標に反転したもの。
mod common;

use std::sync::atomic::AtomicBool;
use std::time::Instant;

use common::fbx_ascii::{arm_scene, arm_takes_scene, avatar_scene, AvatarSpec, Lcl, Scene, Take};
use yolu_core::glam::{DVec3, Quat, Vec3};
use yolu_core::skin::{Pose, Rig};
use yolu_model::{
    evaluate_take, evaluate_take_bytes, evaluate_take_with, load_fbx_bytes, LoadControl,
    ModelError, ModelLimits, TakePose,
};

fn load(text: &str) -> yolu_model::LoadedModel {
    load_fbx_bytes(text.as_bytes(), "腕", &ModelLimits::default()).expect("読める")
}

fn bone(rig: &Rig, name: &str) -> usize {
    rig.bones().iter().position(|b| b.name == name).unwrap()
}

/// テイクのポーズを休みのポーズに重ねる（骨は全部、BlendShape は動かしたものだけ）。
fn pose_of(rig: &Rig, take: &TakePose) -> Pose {
    let mut pose = rig.rest_pose();
    pose.locals.clone_from(&take.locals);
    for &(m, k, w) in &take.weights {
        pose.blend_weights[m][k] = w;
    }
    pose
}

fn mirror(v: DVec3) -> Vec3 {
    Vec3::new(-v.x as f32, v.y as f32, v.z as f32)
}

/// ufbx の評価（スタック `stack` の `time` 秒。メッシュの名前ごとに、コントロールポイントのワールドの位置）。読みの設定は
/// yolu-model と同じ（メートル・Y が上へ変換する）なので、センチメートルや Z が上のファイルでも同じ空間で並ぶ。
fn ufbx_positions(text: &str, stack: usize, time: f64) -> Vec<(String, Vec<DVec3>)> {
    let scene = ufbx::load_memory(text.as_bytes(), options(true, true)).expect("ufbx");
    let anim = &scene.anim_stacks[stack].anim;
    let evaluated = ufbx::evaluate_scene(
        &scene,
        anim,
        time,
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

/// 自分の形（ポーズを付けた）が ufbx の評価と同じか。頂点の対応は休みの形の位置で取る。
fn same_as_ufbx(what: &str, rig: &Rig, pose: &Pose, text: &str, stack: usize, time: f64) {
    let theirs = ufbx_positions(text, stack, time);
    let rest_meshes = rig.deform(&rig.rest_pose()).unwrap();
    let theirs_rest = ufbx_rest_positions(text);
    let meshes = rig.deform(pose).unwrap();
    for ((m, rest_m), mine) in rig.meshes().iter().zip(&rest_meshes).zip(&meshes) {
        let r = &theirs_rest
            .iter()
            .find(|(n, _)| *n == m.mesh.name)
            .unwrap()
            .1;
        let t = &theirs.iter().find(|(n, _)| *n == m.mesh.name).unwrap().1;
        for (v, p) in rest_m.positions.iter().enumerate() {
            let cp = r
                .iter()
                .enumerate()
                .map(|(i, q)| (i, (mirror(*q) - *p).length()))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap()
                .0;
            let want = mirror(t[cp]);
            let got = mine.positions[v];
            assert!(
                (got - want).length() < 1e-4,
                "{what} {} 頂点 {v} 時刻 {time}: {got} と ufbx の {want}",
                m.mesh.name
            );
        }
    }
}

/// ufbx の休みの形（アニメを当てない読み）。
fn ufbx_rest_positions(text: &str) -> Vec<(String, Vec<DVec3>)> {
    let scene = ufbx::load_memory(text.as_bytes(), options(true, true)).expect("ufbx");
    let mut out = Vec::new();
    for n in scene.nodes.iter() {
        let Some(mesh) = n.mesh.as_ref() else {
            continue;
        };
        let m = &n.geometry_to_world;
        let positions = (0..mesh.num_vertices)
            .map(|cp| {
                let p = mesh.vertices[cp];
                DVec3::new(
                    m.m00 * p.x + m.m01 * p.y + m.m02 * p.z + m.m03,
                    m.m10 * p.x + m.m11 * p.y + m.m12 * p.z + m.m13,
                    m.m20 * p.x + m.m21 * p.y + m.m22 * p.z + m.m23,
                )
            })
            .collect();
        out.push((n.element.name.to_string(), positions));
    }
    out
}

/// 同じ形のテイクつきの腕を別の単位・軸で書き直す。`Scene::transformed` は休みの形だけを変えるので、曲線の値も同じ変換にする
/// （移動は座標の変換、大きさは軸の入れ替え、回転はオイラー角が軸 1 本だけの物で、軸と向きを入れ替える）。
fn converted(base: &Scene, f: impl Fn([f64; 3]) -> [f64; 3], k: f64) -> Scene {
    let mut s = base.transformed(&f, k);
    // 軸 i が、新しい軸 j に向き sign で移る（f は軸の入れ替えと反転）
    let axes: Vec<(usize, f64)> = (0..3)
        .map(|i| {
            let mut e = [0.0; 3];
            e[i] = 1.0;
            let v = f(e);
            let j = (0..3).find(|&j| v[j].abs() > 0.5).unwrap();
            (j, v[j].signum())
        })
        .collect();
    for take in &mut s.takes {
        for curve in &mut take.nodes {
            let lcl = curve.lcl;
            for (_, v) in &mut curve.keys {
                let old = *v;
                match lcl {
                    Lcl::Translation => *v = f([old[0] * k, old[1] * k, old[2] * k]),
                    Lcl::Scaling => {
                        for (i, &(j, _)) in axes.iter().enumerate() {
                            v[j] = old[i];
                        }
                    }
                    Lcl::Rotation => {
                        assert!(
                            old.iter().filter(|a| **a != 0.0).count() <= 1,
                            "軸 1 本の回転だけ変換できる"
                        );
                        for (i, &(j, sign)) in axes.iter().enumerate() {
                            v[j] = sign * old[i];
                        }
                    }
                }
            }
        }
    }
    s
}

/// 同じテイクつきの腕を、メートル・Y が上と、センチメートル・Z が上（Blender の書き出しの既定）で書いたもの。
fn variants() -> Vec<(&'static str, Scene)> {
    let base = arm_takes_scene();
    let z_up = |p: [f64; 3]| [p[0], -p[2], p[1]];
    let mut cm = converted(&base, |p| p, 100.0);
    cm.centimeters = true;
    let mut z = converted(&base, z_up, 1.0);
    z.z_up = true;
    let mut cm_z = converted(&base, z_up, 100.0);
    cm_z.centimeters = true;
    cm_z.z_up = true;
    vec![
        ("メートル・Y が上", base),
        ("センチメートル", cm),
        ("Z が上", z),
        ("センチメートル・Z が上", cm_z),
    ]
}

#[test]
fn takes_are_listed_with_their_frames_and_the_frame_rate() {
    let m = load(&arm_takes_scene().to_ascii());
    let t = &m.takes;
    assert_eq!(t.frame_rate, 30.0);
    let list: Vec<(&str, i64, i64)> = t
        .list
        .iter()
        .map(|t| (t.name.as_str(), t.first_frame, t.last_frame))
        .collect();
    // 動かす値の無い Empty は入らない
    assert_eq!(list, [("Wave", 0, 30), ("Raise", 15, 60)]);
    assert_eq!(t.find("Raise"), Some(1));
    assert_eq!(t.find("Empty"), None);
    // テイクの無い FBX は空
    let plain = load(&arm_scene().to_ascii());
    assert!(plain.takes.is_empty());
    // フレームの速さを書かない FBX は ufbx の既定（24）
    let mut s = arm_takes_scene();
    s.frame_rate = None;
    let m = load(&s.to_ascii());
    assert_eq!(m.takes.frame_rate, 24.0);
    assert_eq!(m.takes.list[1].last_frame, 48);
    // 速さが 0 や負の FBX も同じ（ufbx がそのまま返す不正な値は使わない）
    for bad in [0.0, -5.0] {
        let mut s = arm_takes_scene();
        s.frame_rate = Some(bad);
        let m = load(&s.to_ascii());
        assert_eq!(m.takes.frame_rate, 24.0, "{bad}");
    }
}

#[test]
fn reading_the_takes_does_not_change_the_model() {
    let with = load(&arm_takes_scene().to_ascii());
    let without = load(&arm_scene().to_ascii());
    assert_eq!(with.rig.bones(), without.rig.bones());
    assert_eq!(with.rig.meshes(), without.rig.meshes());
    assert_eq!(with.rig.rest_pose(), without.rig.rest_pose());
}

#[test]
fn a_frame_between_linear_keys_is_the_interpolated_rotation() {
    let text = arm_takes_scene().to_ascii();
    let m = load(&text);
    let rig = &m.rig;
    // Wave のフレーム 15（0.5 秒）: Lower の z は 45 度。Unity の座標の回転は X の反転（z のまわりは逆向き）
    let p = evaluate_take_bytes(text.as_bytes(), &m.takes, 0, 15, &ModelLimits::default()).unwrap();
    let lower = bone(rig, "Lower");
    let want = Quat::from_rotation_z(-45f32.to_radians());
    assert!(
        p.locals[lower].rotation.dot(want).abs() > 0.99999,
        "{:?}",
        p.locals[lower].rotation
    );
    // Bend は 50（線形）。Thick（動かさない）は返さない
    assert_eq!(p.weights.len(), 1);
    let (mesh, shape, w) = p.weights[0];
    assert_eq!(rig.meshes()[mesh].blend_shapes[shape].name, "Bend");
    assert!((w - 50.0).abs() < 1e-3, "{w}");
    // Raise のフレーム 30（1 秒）: Upper の移動は定数のキーで前の値のまま、Lower の大きさ x は 1 + 0.5 × (0.5 / 1.5)
    let p = evaluate_take_bytes(text.as_bytes(), &m.takes, 1, 30, &ModelLimits::default()).unwrap();
    let upper = bone(rig, "Upper");
    assert!((p.locals[upper].translation - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-6);
    assert!((p.locals[lower].scale.x - (1.0 + 0.5 / 3.0)).abs() < 1e-5);
    let p = evaluate_take_bytes(text.as_bytes(), &m.takes, 1, 50, &ModelLimits::default()).unwrap();
    assert!((p.locals[upper].translation - Vec3::new(0.0, 1.2, 0.0)).length() < 1e-6);
    assert!(p.weights.is_empty(), "Raise は BlendShape を動かさない");
}

#[test]
fn the_shape_at_a_frame_matches_ufbx_between_keys_and_outside_the_take() {
    // ufbx の評価は読みの設定（メートル・Y が上へ変換）が同じなので、単位や軸の違うファイルでも同じ空間で並ぶ
    for (what, scene) in variants() {
        let text = scene.to_ascii();
        let m = load(&text);
        let rig = &m.rig;
        // Wave: キーの上・線形と 3 次のキーの間・範囲の外（ufbx の延長）。Raise: 定数のキーの前後
        for (take, stack, frames) in [
            (0usize, 0usize, &[0i64, 7, 15, 22, 30, 45][..]),
            (1, 1, &[10, 15, 29, 31, 60]),
        ] {
            for &frame in frames {
                let p = evaluate_take_bytes(
                    text.as_bytes(),
                    &m.takes,
                    take,
                    frame,
                    &ModelLimits::default(),
                )
                .unwrap();
                same_as_ufbx(
                    what,
                    rig,
                    &pose_of(rig, &p),
                    &text,
                    stack,
                    frame as f64 / 30.0,
                );
            }
        }
    }
}

#[test]
fn take_values_are_in_the_same_space_as_the_rest_values() {
    // 評価した値は、休みの値（`Bone::rest`）と同じ空間（メートル・Y が上）。センチメートルや Z が上のファイルでも、
    // 休みの値に対する量（回転の角・移動の倍率・大きさ）は同じ
    for (what, scene) in variants() {
        let text = scene.to_ascii();
        let m = load(&text);
        let rig = &m.rig;
        let (upper, lower) = (bone(rig, "Upper"), bone(rig, "Lower"));
        let rest = |b: usize| rig.bones()[b].rest;
        // 変換が効いている（単位は 0.01 m、Z が上のファイルの骨のローカルは Z が上の向き）
        let unit = if scene.centimeters { 0.01 } else { 1.0 };
        assert!((m.report.file_unit_meters - unit).abs() < 1e-12, "{what}");
        let up = if scene.z_up { Vec3::Z } else { Vec3::Y };
        assert!((rest(upper).translation - up).length() < 1e-6, "{what}");
        let eval = |take: usize, frame: i64| {
            evaluate_take_bytes(
                text.as_bytes(),
                &m.takes,
                take,
                frame,
                &ModelLimits::default(),
            )
            .unwrap()
        };
        // Wave のフレーム 15: Lower の回転は休みから 45 度
        let p = eval(0, 15);
        let angle = (rest(lower).rotation.inverse() * p.locals[lower].rotation)
            .angle_between(Quat::IDENTITY);
        assert!((angle - 45f32.to_radians()).abs() < 1e-4, "{what}: {angle}");
        assert!((p.weights[0].2 - 50.0).abs() < 1e-3, "{what}");
        // Raise: Upper の移動は休みの値の 1（キーの前）→ 1.2 倍（キーの後）。メートルのまま（100 倍にならない）
        for (frame, times) in [(10, 1.0), (50, 1.2)] {
            let p = eval(1, frame);
            let want = rest(upper).translation * times;
            assert!(
                (p.locals[upper].translation - want).length() < 1e-5,
                "{what} フレーム {frame}: {} と {want}",
                p.locals[upper].translation
            );
            assert!(p.locals[upper].translation.length() < 2.0, "{what}");
        }
        // Raise のフレーム 30: Lower の大きさ x は 1 + 0.5 × (0.5 / 1.5)（軸が入れ替わっても x は x）
        let p = eval(1, 30);
        assert!(
            (p.locals[lower].scale.x - (1.0 + 0.5 / 3.0)).abs() < 1e-5,
            "{what}: {}",
            p.locals[lower].scale
        );
    }
}

#[test]
fn bones_the_take_does_not_move_keep_the_rest_values() {
    // 単位や軸の変換を通しても、動かさない骨は休みの値とビットまで同じ（評価と `local_transform` が同じ空間）
    for (what, scene) in variants() {
        let text = scene.to_ascii();
        let m = load(&text);
        let rig = &m.rig;
        for take in 0..2 {
            let p =
                evaluate_take_bytes(text.as_bytes(), &m.takes, take, 12, &ModelLimits::default())
                    .unwrap();
            assert_eq!(p.locals.len(), rig.bones().len());
            for name in ["腕", "Armature", "Hat", "ArmMesh"] {
                let b = bone(rig, name);
                assert_eq!(p.locals[b], rig.bones()[b].rest, "{what} {name}");
            }
        }
        let p =
            evaluate_take_bytes(text.as_bytes(), &m.takes, 0, 12, &ModelLimits::default()).unwrap();
        let upper = bone(rig, "Upper");
        assert_eq!(
            p.locals[upper].translation,
            rig.bones()[upper].rest.translation,
            "{what}"
        );
        assert_ne!(
            p.locals[upper].rotation,
            rig.bones()[upper].rest.rotation,
            "{what}"
        );
    }
}

#[test]
fn a_changed_file_or_a_missing_take_is_refused() {
    let scene = arm_takes_scene();
    let text = scene.to_ascii();
    let m = load(&text);
    let limits = ModelLimits::default();
    // 骨の名前が変わった
    let mut renamed = scene.clone();
    renamed.nodes[2].name = "Forearm".into();
    assert_eq!(
        evaluate_take_bytes(renamed.to_ascii().as_bytes(), &m.takes, 0, 0, &limits),
        Err(ModelError::Changed)
    );
    // BlendShape のチャンネルの名前が変わった
    let mut renamed = scene.clone();
    renamed.meshes[0].channels[1].name = "Curl".into();
    assert_eq!(
        evaluate_take_bytes(renamed.to_ascii().as_bytes(), &m.takes, 0, 0, &limits),
        Err(ModelError::Changed)
    );
    // テイクが消えた・番号が範囲の外
    let mut gone = scene.clone();
    gone.takes.remove(0);
    assert_eq!(
        evaluate_take_bytes(gone.to_ascii().as_bytes(), &m.takes, 0, 0, &limits),
        Err(ModelError::NoTake)
    );
    assert_eq!(
        evaluate_take_bytes(text.as_bytes(), &m.takes, 5, 0, &limits),
        Err(ModelError::NoTake)
    );
    // テイクの並びが変わった（書き出し直しで前にテイクが増えた・並べ替えた）: 名前で今のテイクを引く
    let eval = |bytes: &[u8], take: usize, frame: i64| {
        evaluate_take_bytes(bytes, &m.takes, take, frame, &limits)
    };
    let (wave, raise) = (eval(text.as_bytes(), 0, 12), eval(text.as_bytes(), 1, 40));
    assert!(wave.is_ok() && raise.is_ok());
    let mut added = scene.clone();
    let mut front = scene.takes[1].clone();
    front.name = "Added".into();
    added.takes.insert(0, front);
    let mut swapped = scene.clone();
    swapped.takes.swap(0, 1);
    // 同じ名前で何も動かさないテイクが先に増えても、動かす方を引く
    let mut shadowed = scene.clone();
    shadowed.takes.insert(
        0,
        Take {
            name: "Wave".into(),
            stop: 1.0,
            ..Take::default()
        },
    );
    for (what, changed) in [
        ("前に増えた", added),
        ("入れ替わった", swapped),
        ("同じ名前の空", shadowed),
    ] {
        let bytes = changed.to_ascii();
        assert_eq!(eval(bytes.as_bytes(), 0, 12), wave, "{what}: Wave");
        assert_eq!(eval(bytes.as_bytes(), 1, 40), raise, "{what}: Raise");
    }
    // 同じ形でキーだけ変わった: 今のファイルのテイクを評価する
    let mut edited = scene.clone();
    edited.takes[0].nodes[0].keys[1].1 = [0.0, 0.0, 60.0];
    let p = evaluate_take_bytes(edited.to_ascii().as_bytes(), &m.takes, 0, 30, &limits).unwrap();
    let lower = bone(&m.rig, "Lower");
    let want = Quat::from_rotation_z(-60f32.to_radians());
    assert!(p.locals[lower].rotation.dot(want).abs() > 0.99999);
}

#[test]
fn reading_from_a_file_can_be_cancelled_and_is_limited() {
    let dir = std::env::temp_dir().join(format!("yolu-model-takes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("腕.fbx");
    let text = arm_takes_scene().to_ascii();
    std::fs::write(&path, &text).unwrap();
    let m = load(&text);
    let limits = ModelLimits::default();
    let p = evaluate_take(&path, &m.takes, 1, 40, &limits).unwrap();
    assert_eq!(
        p,
        evaluate_take_bytes(text.as_bytes(), &m.takes, 1, 40, &limits).unwrap()
    );
    let cancel = AtomicBool::new(true);
    let r = evaluate_take_with(
        &path,
        &m.takes,
        1,
        40,
        &limits,
        LoadControl {
            cancel: Some(&cancel),
            progress: None,
        },
    );
    assert_eq!(r, Err(ModelError::Cancelled));
    let small = ModelLimits {
        max_file_bytes: 16,
        ..ModelLimits::default()
    };
    assert!(matches!(
        evaluate_take(&path, &m.takes, 1, 40, &small),
        Err(ModelError::FileTooLarge { .. })
    ));
    assert!(matches!(
        evaluate_take(&dir.join("無い.fbx"), &m.takes, 1, 40, &limits),
        Err(ModelError::Io(_))
    ));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// ufbx の読み（`yolu_model::fbx::load_options` と同じ設定。アニメを読むか・形を読むかだけを替える）。
fn options<'a>(animation: bool, geometry: bool) -> ufbx::LoadOpts<'a> {
    ufbx::LoadOpts {
        ignore_animation: !animation,
        ignore_geometry: !geometry,
        ignore_embedded: true,
        load_external_files: false,
        generate_missing_normals: true,
        clean_skin_weights: true,
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

fn mib(bytes: usize) -> f64 {
    bytes as f64 / 1048576.0
}

/// 読みの時間とメモリを測る（5 回の最小。メモリは ufbx の結果と一時の使った量）。
fn parse(data: &[u8], animation: bool, geometry: bool) -> (f64, usize, usize, ufbx::SceneRoot) {
    let mut best = f64::MAX;
    let mut last = None;
    for _ in 0..5 {
        let clock = Instant::now();
        let scene = ufbx::load_memory(data, options(animation, geometry)).expect("読める");
        best = best.min(clock.elapsed().as_secs_f64() * 1000.0);
        last = Some(scene);
    }
    let scene = last.unwrap();
    let (r, t) = (
        scene.metadata.result_memory_used,
        scene.metadata.temp_memory_used,
    );
    (best, r, t, scene)
}

/// 5 回の最小（ミリ秒）。
fn best_ms(mut f: impl FnMut()) -> f64 {
    (0..5)
        .map(|_| {
            let clock = Instant::now();
            f();
            clock.elapsed().as_secs_f64() * 1000.0
        })
        .fold(f64::MAX, f64::min)
}

#[test]
#[ignore = "読みの時間とメモリの測定（表を出力する）。`cargo test -p yolu-model --test takes -- --ignored --nocapture`"]
fn measure_reading_takes() {
    let avatar = |takes, frames| AvatarSpec {
        bones: 250,
        chains: 10,
        mesh_rings: vec![3000, 1500, 2500, 3000],
        blend_channels: 120,
        blend_offsets: 1500,
        takes,
        frames,
    };
    let cases = [
        ("テイク 1 つ × 2 フレーム", avatar(1, 2)),
        ("テイク 1 つ × 300 フレーム", avatar(1, 300)),
        ("テイク 5 つ × 300 フレーム", avatar(5, 300)),
    ];
    println!("アバターの形（骨 250・頂点 4 万・三角形 8 万・BlendShape 120 × 差分 1500）。どの骨も毎フレーム T・R・S にキー。ASCII。5 回の最小");
    println!("| テイク | ASCII | ufbx の読み（曲線を読まない。今も後も同じ設定） | yolu-model の読み全体（後） | 曲線も読む（参考） | 形を読まずアニメだけ | テイクのフレームのポーズ（読み直し＋評価） |");
    for (what, spec) in cases {
        let text = avatar_scene(&spec).to_ascii();
        let data = text.as_bytes();
        let (ms_a, r_a, t_a, scene_a) = parse(data, false, true);
        let mut loaded = None;
        let ms_b = best_ms(|| {
            loaded = Some(load_fbx_bytes(data, "測り", &ModelLimits::default()).expect("読める"));
        });
        let m = loaded.unwrap();
        let (ms_c, r_c, t_c, _) = parse(data, true, true);
        let (ms_d, r_d, t_d, _) = parse(data, true, false);
        let last = m.takes.list.len() - 1;
        let frame = m.takes.list[last].last_frame / 2;
        let ms_e = best_ms(|| {
            evaluate_take_bytes(data, &m.takes, last, frame, &ModelLimits::default()).unwrap();
        });
        println!(
            "| {what} | {:.1} MiB | {ms_a:.0} ms・結果 {:.1} MiB・一時 {:.1} MiB（スタック {}） | {ms_b:.0} ms（テイク {}） | {ms_c:.0} ms・結果 {:.1} MiB・一時 {:.1} MiB | {ms_d:.0} ms・結果 {:.1} MiB・一時 {:.1} MiB | {ms_e:.0} ms |",
            mib(text.len()),
            mib(r_a),
            mib(t_a),
            scene_a.anim_stacks.count,
            m.takes.list.len(),
            mib(r_c),
            mib(t_c),
            mib(r_d),
            mib(t_d),
        );
    }
}
