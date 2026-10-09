//! 3D ビュー: 斜めに見た面の絵がぼやけない（塗った絵のサンプラーの異方性フィルタリング）。
//!
//! 1 画素おきの横縞（縞は縦方向 = 板の v）を貼った板を、横へ 78° 回した位置から見る。板の横（u）は強く縮み、縦（v）はほぼそのまま。
//! 等方のミップは強く縮む横に合わせて段を選ぶので、縮まない縦の縞まで潰れて灰色になる。異方性フィルタリングは縦の細かさを残す。
use crate::common;

use common::*;
use egui::Pos2;
use egui_kittest::Harness;
use yolu_app::state::Action;
use yolu_app::view3d::display::{EnvKind, Op, Shading};
use yolu_app::view3d::model::ViewModel;
use yolu_app::YoluApp;
use yolu_core::geometry::{ModelMesh, OrbitCamera, Submesh};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::look::MaterialLook;
use yolu_core::Rgba8;

const DOC: u32 = 256;

/// カメラに向いた板（外向きの法線は −Z）。
fn plane() -> ModelMesh {
    let h = 1.0;
    ModelMesh {
        name: "板".into(),
        positions: vec![
            Vec3::new(-h, -h, 0.0),
            Vec3::new(h, -h, 0.0),
            Vec3::new(-h, h, 0.0),
            Vec3::new(h, h, 0.0),
        ],
        normals: vec![Vec3::NEG_Z; 4],
        uvs: vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 1.0),
        ],
        submeshes: vec![Submesh {
            material: 0,
            indices: vec![0, 2, 1, 2, 3, 1],
        }],
    }
}

/// 1 画素おきの横縞（白と黒）の板を、横へ 78° 回して見る。`standard` なら標準の見た目と中立の光（`scene.wgsl`）、そうでなければ既定の lilToon。
fn striped_plane(standard: bool) -> Harness<'static, YoluApp> {
    let mut h = app(900.0, 700.0, DOC);
    click_tab(&mut h, yolu_app::Tab::View3d);
    move_to(&h, egui::pos2(1.0, 1.0));
    h.run();
    if standard {
        h.state_mut()
            .state
            .set_doc_mut(0)
            .restore_look(MaterialLook::default())
            .expect("作ったばかりの文書");
        h.state_mut()
            .apply(Action::View3d(Op::Shading(Shading::Neutral)));
    } else {
        h.state_mut()
            .apply(Action::View3d(Op::Shading(Shading::Material)));
    }
    h.state_mut().apply(Action::View3d(Op::Env(EnvKind::None)));
    {
        let state = &mut h.state_mut().state;
        let layer = state.selected_layer.expect("レイヤー");
        let doc = &mut state.doc;
        for y in 0..DOC {
            let v = if y % 2 == 0 { 0 } else { 255 };
            for x in 0..DOC {
                doc.set_pixel(layer, x, y, Rgba8::new(v, v, v, 255))
                    .unwrap();
            }
        }
        doc.clear_history().unwrap();
        let revision = state.view3d.next_revision();
        state.view3d.material = 0;
        let model =
            ViewModel::new("板", vec![plane()], vec![Some("板".to_string())], revision).unwrap();
        model.tangents();
        state.view3d.set_model(model);
        state.view3d.camera = OrbitCamera {
            target: Vec3::ZERO,
            yaw: 78.0,
            pitch: 0.0,
            distance: 3.2,
            model_radius: 1.5,
            ..Default::default()
        };
    }
    h.run();
    h
}

fn luminance(image: &image::RgbaImage, p: Pos2) -> f32 {
    let c = image.get_pixel(p.x.round() as u32, p.y.round() as u32).0;
    0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32
}

/// 板の中ほどを縦に通る線の明るさの標準偏差（縞が見えるほど大きい）。
fn stripe_contrast(h: &mut Harness<'_, YoluApp>) -> (f32, image::RgbaImage) {
    let image = h.render().expect("描ける");
    let rect = h.state().view3d_rect().expect("3D のタブを描いた");
    let x = rect.center().x;
    let values: Vec<f32> = (-110..110)
        .map(|dy| luminance(&image, Pos2::new(x, rect.center().y + dy as f32)))
        .collect();
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32;
    (variance.sqrt(), image)
}

/// 前後の絵を見るために書き出す（`YOLU_ANISO_SHOTS` に置き場を渡したとき）。
fn save_shot(image: &image::RgbaImage, name: &str) {
    if let Some(dir) = std::env::var_os("YOLU_ANISO_SHOTS") {
        let _ = std::fs::create_dir_all(&dir);
        image.save(std::path::Path::new(&dir).join(name)).unwrap();
    }
}

#[test]
fn an_oblique_face_keeps_its_fine_stripes_with_anisotropic_filtering() {
    for standard in [false, true] {
        let what = if standard { "標準" } else { "lilToon" };
        let mut h = striped_plane(standard);
        // CPU で描くアダプター（この試験の lavapipe）の既定は 1。持つ実 GPU の既定は 16（`default_anisotropy`）
        let default = h.state().view3d_paint_anisotropy().expect("wgpu の 3D");
        assert!(default == 1 || default == 16, "{what}: {default}");
        h.state_mut().view3d_set_paint_anisotropy(1);
        h.run();
        let (flat, image) = stripe_contrast(&mut h);
        save_shot(
            &image,
            &format!(
                "oblique_{}_aniso_off.png",
                if standard { "standard" } else { "liltoon" }
            ),
        );
        h.state_mut().view3d_set_paint_anisotropy(16);
        h.run();
        // 試験が 16 を求めたとき、GPU が異方性を持てば 16、持たなければ 1 のまま（落ちない）
        let now = h.state().view3d_paint_anisotropy().unwrap();
        assert!(now == 1 || now == 16, "{what}: {now}");
        let (sharp, image) = stripe_contrast(&mut h);
        save_shot(
            &image,
            &format!(
                "oblique_{}_aniso_on.png",
                if standard { "standard" } else { "liltoon" }
            ),
        );
        println!("{what}: 異方性 1 → 縞の標準偏差 {flat:.1}、{now} → {sharp:.1}");
        if now > 1 {
            // 等方のミップは縞を潰す。異方性は倍以上残す
            assert!(
                sharp > flat * 2.0 && sharp > 10.0,
                "{what}: 等方 {flat:.1}、異方性 {now} → {sharp:.1}"
            );
        } else {
            // GPU が持たない: 値を変えようとしても 1 のまま、見た目は変わらない
            assert!((sharp - flat).abs() < 1.0, "{what}: {flat:.1} {sharp:.1}");
        }
    }
}

/// 計測: 塗った絵のサンプラーの異方性（1 と 16）で、1 フレームの時間（GPU の完了まで待つ）を測る。2048² の文書（Color の塗りつぶし）を
/// 球（格子 77 = 約 71,000 三角形）と、斜めに見た大きな板に貼り、カメラを回すだけ（絵は同じ）のフレームを、設定を交互に 3 回ずつ（1 回 24 フレーム、
/// 最初の 4 つは捨てる）測った平均。
/// `cargo test -p yolu-app --test gui_view3d view3d_aniso::measure -- --ignored --nocapture`
#[test]
#[ignore = "計測"]
fn measure_frames_with_and_without_anisotropy() {
    use std::time::Instant;
    use yolu_core::geometry::cube_sphere;
    // 絵は 2048² の塗りつぶし（板の縞は、描ける文書の大きさのままの 256² なので、ここでは使わない）
    let mut big = app(1280.0, 800.0, 2048);
    click_tab(&mut big, yolu_app::Tab::View3d);
    big.run();
    println!("GPU: {}", big.state().view3d_adapter().unwrap_or_default());
    {
        let state = &mut big.state_mut().state;
        state
            .doc
            .add_fill_layer(
                "値",
                &[
                    (yolu_core::Channel::Color, Rgba8::new(200, 120, 60, 255)),
                    (
                        yolu_core::Channel::Roughness,
                        Rgba8::new(120, 120, 120, 255),
                    ),
                    (yolu_core::Channel::Normal, Rgba8::new(140, 120, 240, 255)),
                ],
                None,
            )
            .unwrap();
    }
    big.run();
    big.state_mut()
        .apply(Action::View3d(Op::Shading(Shading::Material)));
    let scenes: Vec<(&str, Vec<ModelMesh>, f32, f32)> = vec![
        (
            "球 71,148 三角形・正面寄り",
            vec![cube_sphere(77, 0.5)],
            20.0,
            10.0,
        ),
        ("板・横へ 78°", vec![plane()], 78.0, 0.0),
    ];
    for (name, meshes, yaw, pitch) in scenes {
        {
            let state = &mut big.state_mut().state;
            let revision = state.view3d.next_revision();
            state.view3d.material = 0;
            let model =
                ViewModel::new("試験", meshes, vec![Some("試験".to_string())], revision).unwrap();
            model.tangents();
            state.view3d.set_model(model);
            state.view3d.camera = OrbitCamera {
                target: Vec3::ZERO,
                yaw,
                pitch,
                distance: 1.6,
                model_radius: 1.0,
                ..Default::default()
            };
        }
        big.run();
        let mut results: Vec<(u16, Vec<f64>)> = vec![(1, Vec::new()), (16, Vec::new())];
        for _round in 0..3 {
            for (clamp, sums) in &mut results {
                big.state_mut().view3d_set_paint_anisotropy(*clamp);
                big.run();
                let mut wall = Vec::new();
                for frame in 0..24 {
                    big.state_mut().state.view3d.camera.yaw += 0.4 + frame as f32 * 0.001;
                    let started = Instant::now();
                    big.step();
                    big.state().view3d_wait_gpu();
                    wall.push(started.elapsed().as_micros() as f64 / 1000.0);
                }
                sums.push(wall[4..].iter().sum::<f64>() / (wall.len() - 4) as f64);
            }
        }
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        println!(
            "{name}: 異方性 1 = {:.2} ms（{:.2?}）、16 = {:.2} ms（{:.2?}）、実際の上限 {}",
            mean(&results[0].1),
            results[0].1,
            mean(&results[1].1),
            results[1].1,
            big.state().view3d_paint_anisotropy().unwrap_or(0),
        );
    }
}
