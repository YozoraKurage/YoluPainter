//! 3D ビューの質感（PBR・環境・トーンマッピング・表示の切り替え）の見た目と上げ方（egui_kittest。描画は wgpu のソフトの描画）。
//!
//! 数値は CPU で同じ式を書いた参照（`yolu_app::view3d::brdf`）と照らす。見た目の全体はスナップショット（コンテナの llvmpipe で撮ったもの。
//! Windows の D3D12 の実機では縁と補間が少し違うので、`kittest.toml` でその OS では緩める）。
use crate::common;

use common::*;
use egui::Pos2;
use egui_kittest::Harness;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use yolu_app::state::Action;
use yolu_app::view3d::brdf::{self, Curve, Indirect, Light, Surface};
use yolu_app::view3d::display::{Display, EnvKind, Op, Shading};
use yolu_app::view3d::model::ViewModel;
use yolu_app::view3d::paint::Slot;
use yolu_app::view3d::render::TangentHook;
use yolu_app::YoluApp;
use yolu_core::geometry::{cube_sphere, ModelMesh, OrbitCamera, Submesh};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::{Channel, HeightEdgeMode, LayerId, NormalSettings, NormalYDirection, Rgba8};

/// 3D のタブを出したウィンドウ（文書は doc × doc）。モデルは呼び手が入れる。
fn view(width: f32, height: f32, doc: u32) -> Harness<'static, YoluApp> {
    let mut h = app(width, height, doc);
    // この試験の場面は標準（PBR）の見た目を見る: 新しい文書の既定（lilToon）でなく標準を明示する
    h.state_mut()
        .state
        .doc
        .restore_look(yolu_core::look::MaterialLook::default())
        .unwrap();
    click_tab(&mut h, yolu_app::Tab::View3d);
    // マウスの矢印が 3D の表示域の上端に残ると、絵の測定（いちばん明るい画素など）に入る。見出しの帯が無いので、ポインタを外へ
    move_to(&h, egui::pos2(1.0, 1.0));
    h.run();
    h
}

/// カメラに向いた板（外向きの法線は −Z、U は +X、V は +Y。Unity の Quad と同じ向き）。一辺 size、UV は (0,0)〜(1,1)。
fn quad(size: f32) -> ModelMesh {
    let h = size * 0.5;
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

fn set_model(h: &mut Harness<'_, YoluApp>, meshes: Vec<ModelMesh>) {
    set_model_with(h, meshes, true);
}

/// `warm` なら接線を先に作っておく（描きが出来るのを待たない）。そうでなければ最初に要る所で別のスレッドが作る。
fn set_model_with(h: &mut Harness<'_, YoluApp>, meshes: Vec<ModelMesh>, warm: bool) {
    let state = &mut h.state_mut().state;
    let revision = state.view3d.next_revision();
    state.view3d.material = 0;
    let model = ViewModel::new("試験", meshes, vec![Some("試験".to_string())], revision).unwrap();
    if warm {
        model.tangents();
    }
    state.view3d.set_model(model);
    h.run();
}

fn look_at(h: &mut Harness<'_, YoluApp>, target: Vec3, distance: f32, yaw: f32, pitch: f32) {
    h.state_mut().state.view3d.camera = OrbitCamera {
        target,
        yaw,
        pitch,
        distance,
        model_radius: 1.0,
        ..Default::default()
    };
    h.run();
}

fn op(h: &mut Harness<'_, YoluApp>, op: Op) {
    h.state_mut().apply(Action::View3d(op));
    h.run();
}

fn first_layer(h: &Harness<'_, YoluApp>) -> LayerId {
    h.state().state.doc.layers()[0].id()
}

/// 文書の全面を覆う塗りつぶしレイヤーを足す（チャンネルごとの値）。
fn fill(h: &mut Harness<'_, YoluApp>, name: &str, values: &[(Channel, [u8; 4])]) -> LayerId {
    let values: Vec<(Channel, Rgba8)> = values
        .iter()
        .map(|(c, v)| (*c, Rgba8::new(v[0], v[1], v[2], v[3])))
        .collect();
    let id = h
        .state_mut()
        .state
        .doc
        .add_fill_layer(name, &values, None)
        .unwrap();
    h.run();
    id
}

fn gray(v: u8) -> [u8; 4] {
    [v, v, v, 255]
}

fn center(h: &Harness<'_, YoluApp>) -> Pos2 {
    h.state().view3d_rect().expect("3D のタブを描いた").center()
}

fn px(image: &image::RgbaImage, p: Pos2) -> [u8; 3] {
    let c = image.get_pixel(p.x.round() as u32, p.y.round() as u32).0;
    [c[0], c[1], c[2]]
}

fn luma(c: [u8; 3]) -> f32 {
    0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32
}

fn assert_close(actual: [u8; 3], expected: [u8; 3], tol: u8, what: &str) {
    for k in 0..3 {
        assert!(
            actual[k].abs_diff(expected[k]) <= tol,
            "{what}: {actual:?} と期待 {expected:?}（許す差 {tol}）"
        );
    }
}

/// 板に真正面から光を当て（光も目も法線の向き）、環境なし・一様な環境光の PBR を CPU の式で出す。
fn expected_head_on(color: [u8; 3], metallic: u8, roughness: u8, emission: [u8; 3]) -> [u8; 3] {
    let albedo = brdf::srgb_to_linear3(Vec3::new(
        color[0] as f32 / 255.0,
        color[1] as f32 / 255.0,
        color[2] as f32 / 255.0,
    ));
    let surface = Surface {
        albedo,
        metallic: metallic as f32 / 255.0,
        smoothness: 1.0 - roughness as f32 / 255.0,
        normal: Vec3::NEG_Z,
        emission: Vec3::ZERO,
    };
    let light = Light {
        to_light: Vec3::NEG_Z,
        radiance: Vec3::from(Display::default().direct_light()),
    };
    let flat = Vec3::splat(brdf::srgb_to_linear(0.2));
    let lit = brdf::standard_brdf(
        &surface,
        Vec3::NEG_Z,
        &light,
        &Indirect {
            diffuse: flat,
            specular: flat,
        },
    );
    let e = brdf::srgb_to_linear3(Vec3::new(
        emission[0] as f32 / 255.0,
        emission[1] as f32 / 255.0,
        emission[2] as f32 / 255.0,
    ));
    let out = brdf::linear_to_srgb3(lit + e);
    [0, 1, 2].map(|k| (out[k].clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// 真正面から光を当てる（光の来る向き = カメラの側 −Z）。
fn head_on_light(h: &mut Harness<'_, YoluApp>) {
    op(h, Op::LightYaw(180.0));
    op(h, Op::LightPitch(0.0));
    op(h, Op::Env(EnvKind::None));
}

#[test]
fn the_material_view_matches_the_cpu_brdf_head_on() {
    let mut h = view(900.0, 640.0, 64);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, Vec3::ZERO, 2.5, 0.0, 0.0);
    head_on_light(&mut h);
    let color = [200u8, 120, 60];
    let layer = fill(
        &mut h,
        "値",
        &[
            (Channel::Color, [color[0], color[1], color[2], 255]),
            (Channel::Metallic, gray(0)),
            (Channel::Roughness, gray(128)),
        ],
    );
    let at = center(&h);
    // 誘電体・金属・半金属、粗い・つるつる・中くらい
    for (metallic, roughness) in [
        (0u8, 255u8),
        (0, 128),
        (0, 0),
        (255, 128),
        (255, 20),
        (128, 200),
    ] {
        let doc = &mut h.state_mut().state.doc;
        doc.set_fill_value(
            layer,
            Channel::Metallic,
            Some(Rgba8::new(metallic, metallic, metallic, 255)),
            false,
        )
        .unwrap();
        doc.set_fill_value(
            layer,
            Channel::Roughness,
            Some(Rgba8::new(roughness, roughness, roughness, 255)),
            false,
        )
        .unwrap();
        h.run();
        let image = h.render().expect("描ける");
        assert_close(
            px(&image, at),
            expected_head_on(color, metallic, roughness, [0, 0, 0]),
            3,
            &format!("金属 {metallic}・粗さ {roughness}"),
        );
    }
}

#[test]
fn emission_adds_to_the_lit_color_and_shows_in_the_dark() {
    let mut h = view(900.0, 640.0, 64);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, Vec3::ZERO, 2.5, 0.0, 0.0);
    head_on_light(&mut h);
    op(&mut h, Op::LightIntensity(0.0)); // 直接光なし。環境光の分だけ
    fill(
        &mut h,
        "値",
        &[
            (Channel::Color, [90, 90, 90, 255]),
            (Channel::Metallic, gray(0)),
            (Channel::Roughness, gray(128)),
        ],
    );
    let at = center(&h);
    let before = px(&h.render().unwrap(), at);
    fill(&mut h, "光る", &[(Channel::Emission, [255, 140, 20, 255])]);
    let after = px(&h.render().unwrap(), at);
    // 発光はリニアで足す（CPU の式と同じ）。直接光 0 の環境光だけの暗い色に、橙が足される
    let lit = brdf::standard_brdf(
        &Surface {
            albedo: brdf::srgb_to_linear3(Vec3::splat(90.0 / 255.0)),
            metallic: 0.0,
            smoothness: 1.0 - 128.0 / 255.0,
            normal: Vec3::NEG_Z,
            emission: Vec3::ZERO,
        },
        Vec3::NEG_Z,
        &Light {
            to_light: Vec3::NEG_Z,
            radiance: Vec3::ZERO,
        },
        &Indirect {
            diffuse: Vec3::splat(brdf::srgb_to_linear(0.2)),
            specular: Vec3::splat(brdf::srgb_to_linear(0.2)),
        },
    );
    let e = brdf::srgb_to_linear3(Vec3::new(1.0, 140.0 / 255.0, 20.0 / 255.0));
    let want = brdf::linear_to_srgb3(lit + e);
    assert_close(
        after,
        [0, 1, 2].map(|k| (want[k].clamp(0.0, 1.0) * 255.0).round() as u8),
        3,
        "発光",
    );
    assert!(
        after[0] > before[0] + 100 && after[2] < after[0],
        "{before:?} → {after:?}"
    );
}

// ───────── 法線マップの向き ─────────

/// 板を真正面から見て、光を方位・高さで当てたときの中心の明るさ。
fn lit_luma(h: &mut Harness<'_, YoluApp>, yaw: f32, pitch: f32) -> f32 {
    op(h, Op::LightYaw(yaw));
    op(h, Op::LightPitch(pitch));
    let at = center(h);
    luma(px(&h.render().expect("描ける"), at))
}

/// 法線マップの試験の場面: 板に灰色・粗い・金属でない面と、環境なし。
fn normal_scene() -> Harness<'static, YoluApp> {
    let mut h = view(900.0, 640.0, 64);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, Vec3::ZERO, 2.5, 0.0, 0.0);
    op(&mut h, Op::Env(EnvKind::None));
    fill(
        &mut h,
        "面",
        &[
            (Channel::Color, [200, 200, 200, 255]),
            (Channel::Metallic, gray(0)),
            (Channel::Roughness, gray(255)),
        ],
    );
    h
}

#[test]
fn a_painted_normal_map_tilts_toward_u_and_v_like_unity() {
    let mut h = normal_scene();
    // U は画面の右（+X）、V は画面の上（+Y）。接空間の法線 (x, y, z) の x が右、y が上へ傾ける（OpenGL の Y+）
    let layer = fill(&mut h, "法線", &[(Channel::Normal, [217, 128, 217, 255])]);
    let set = |h: &mut Harness<'_, YoluApp>, rgb: [u8; 3]| {
        h.state_mut()
            .state
            .doc
            .set_fill_value(
                layer,
                Channel::Normal,
                Some(Rgba8::new(rgb[0], rgb[1], rgb[2], 255)),
                false,
            )
            .unwrap();
        h.run();
    };
    // 右へ傾けた法線: 右から光を当てると明るく、左から当てると暗い
    let from_right = lit_luma(&mut h, 90.0, 0.0);
    let from_left = lit_luma(&mut h, -90.0, 0.0);
    assert!(
        from_right > from_left + 40.0,
        "+U へ傾けた: 右の光 {from_right}、左の光 {from_left}"
    );
    set(&mut h, [38, 128, 217]);
    let from_right = lit_luma(&mut h, 90.0, 0.0);
    let from_left = lit_luma(&mut h, -90.0, 0.0);
    assert!(
        from_left > from_right + 40.0,
        "−U へ傾けた: 右の光 {from_right}、左の光 {from_left}"
    );
    // 上へ傾けた法線（緑が大きい）: 上から光を当てると明るい
    set(&mut h, [128, 217, 217]);
    let from_top = lit_luma(&mut h, 0.0, 85.0);
    let from_bottom = lit_luma(&mut h, 0.0, -85.0);
    assert!(
        from_top > from_bottom + 40.0,
        "+V へ傾けた: 上の光 {from_top}、下の光 {from_bottom}"
    );
    set(&mut h, [128, 38, 217]);
    let from_top = lit_luma(&mut h, 0.0, 85.0);
    let from_bottom = lit_luma(&mut h, 0.0, -85.0);
    assert!(
        from_bottom > from_top + 40.0,
        "−V へ傾けた: 上の光 {from_top}、下の光 {from_bottom}"
    );
}

/// 文書の全面に高さの傾斜を書く（x か y の向きに 0 → 252）。
fn height_ramp(h: &mut Harness<'_, YoluApp>, along_x: bool) {
    let layer = first_layer(h);
    let size = h.state().state.doc.width();
    let doc = &mut h.state_mut().state.doc;
    for y in 0..size {
        for x in 0..size {
            let t = if along_x { x } else { y };
            let v = (t * 252 / (size - 1)) as u8;
            doc.set_channel_pixel(layer, Channel::Height, x, y, Rgba8::new(v, v, v, 255))
                .unwrap();
        }
    }
    h.run();
}

#[test]
fn height_to_normal_tilts_away_from_the_rising_side() {
    let mut h = normal_scene();
    h.state_mut()
        .state
        .doc
        .set_normal_settings(
            NormalSettings::new(true, 40.0, HeightEdgeMode::Clamp, NormalYDirection::OpenGL)
                .unwrap(),
            false,
        )
        .unwrap();
    // x（U）に沿って高くなる坂: 法線は低い側（−U）へ傾く。左から光を当てると明るい
    height_ramp(&mut h, true);
    let from_left = lit_luma(&mut h, -90.0, 0.0);
    let from_right = lit_luma(&mut h, 90.0, 0.0);
    assert!(
        from_left > from_right + 40.0,
        "U の坂: 左の光 {from_left}、右の光 {from_right}"
    );
    // y（V）に沿って高くなる坂: 法線は −V（下）へ傾く。下から光を当てると明るい
    height_ramp(&mut h, false);
    let from_bottom = lit_luma(&mut h, 0.0, -85.0);
    let from_top = lit_luma(&mut h, 0.0, 85.0);
    assert!(
        from_bottom > from_top + 40.0,
        "V の坂: 下の光 {from_bottom}、上の光 {from_top}"
    );
    // 設定の強さを 0 に戻すと平ら（出力だけが変わる設定でも見た目が追いつく）
    h.state_mut()
        .state
        .doc
        .set_normal_settings(
            NormalSettings::new(true, 0.0, HeightEdgeMode::Clamp, NormalYDirection::OpenGL)
                .unwrap(),
            false,
        )
        .unwrap();
    h.run();
    let a = lit_luma(&mut h, 0.0, 85.0);
    let b = lit_luma(&mut h, 0.0, -85.0);
    assert!((a - b).abs() < 6.0, "強さ 0 は平ら: {a} {b}");
}

// ───────── チャンネルの上げ方 ─────────

#[test]
fn only_the_changed_channel_and_tiles_are_uploaded() {
    let mut h = view(1000.0, 700.0, 512); // 4 × 4 タイル（1 タイル 128²）
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, Vec3::ZERO, 2.5, 0.0, 0.0);
    let layer = first_layer(&h);
    let stats = |h: &Harness<'_, YoluApp>| h.state().view3d_stats().expect("wgpu の 3D");
    let base = stats(&h);
    assert_eq!(
        base.total_slot_tiles, [0; 6],
        "空の文書は上げるタイルが無い"
    );
    let color_bytes = base.paint_bytes;
    assert!(color_bytes > 0, "Color は使っているので作ってある");
    // Metallic を 1 タイルだけ塗る: Metallic のテクスチャができ、そのタイルだけが上がる（Color は触らない）
    let doc = &mut h.state_mut().state.doc;
    doc.set_channel_pixel(
        layer,
        Channel::Metallic,
        10,
        10,
        Rgba8::new(255, 255, 255, 255),
    )
    .unwrap();
    h.run();
    let s = stats(&h);
    assert_eq!(s.last_slot_tiles, [0, 1, 0, 0, 0, 0], "{s:?}");
    assert!(!s.last_rebuilt);
    assert!(
        s.paint_bytes > color_bytes,
        "使い始めたチャンネルの分が増える"
    );
    // 別のタイルへ: そのタイルだけ
    let doc = &mut h.state_mut().state.doc;
    doc.set_channel_pixel(
        layer,
        Channel::Metallic,
        300,
        300,
        Rgba8::new(255, 255, 255, 255),
    )
    .unwrap();
    h.run();
    let s = stats(&h);
    assert_eq!(s.last_slot_tiles, [0, 1, 0, 0, 0, 0], "{s:?}");
    assert_eq!(s.total_slot_tiles[1], 2);
    // Color を 1 タイル: Color だけ
    let doc = &mut h.state_mut().state.doc;
    doc.set_channel_pixel(layer, Channel::Color, 400, 20, Rgba8::new(255, 0, 0, 255))
        .unwrap();
    h.run();
    assert_eq!(stats(&h).last_slot_tiles, [1, 0, 0, 0, 0, 0]);
    // 何も変わらなければ上げない・描かない
    let idle = stats(&h);
    h.run();
    assert_eq!(stats(&h).renders, idle.renders);
    assert_eq!(stats(&h).last_slot_tiles, [0; 6]);
    // 使っていないチャンネル（Roughness・Normal・Emission・Height）はテクスチャを持たない
    assert_eq!(stats(&h).total_slot_tiles[2..], [0, 0, 0, 0]);
}

#[test]
fn height_to_normal_uploads_the_changed_height_tile_with_its_border() {
    let mut h = view(1000.0, 700.0, 512);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, Vec3::ZERO, 2.5, 0.0, 0.0);
    let layer = first_layer(&h);
    h.state_mut()
        .state
        .doc
        .set_normal_settings(
            NormalSettings::new(true, 4.0, HeightEdgeMode::Clamp, NormalYDirection::OpenGL)
                .unwrap(),
            false,
        )
        .unwrap();
    h.run();
    let doc = &mut h.state_mut().state.doc;
    doc.set_channel_pixel(
        layer,
        Channel::Height,
        10,
        10,
        Rgba8::new(255, 255, 255, 255),
    )
    .unwrap();
    h.run();
    let s = h.state().view3d_stats().unwrap();
    // Height を使い始めると Normal も使う（Height から作る）。Normal は全タイルで作る
    assert_eq!(s.last_slot_tiles[5], 1, "{s:?}");
    assert_eq!(s.last_slot_tiles[3], 16, "{s:?}");
    // もう 1 回: Height の 1 タイルと、その外 1 画素までの Normal の 1 つの矩形
    let doc = &mut h.state_mut().state.doc;
    doc.set_channel_pixel(
        layer,
        Channel::Height,
        200,
        200,
        Rgba8::new(255, 255, 255, 255),
    )
    .unwrap();
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!(
        (s.last_slot_tiles[5], s.last_slot_tiles[3]),
        (1, 1),
        "{s:?}"
    );
}

// ───────── 表示の切り替え ─────────

// ───────── メモリの予算とミップ・書き出しとの一致 ─────────

#[test]
fn a_document_over_the_byte_budget_is_shown_smaller_and_never_allocated_in_full() {
    use egui_kittest::kittest::Queryable;
    const BUDGET: u64 = 2 << 20;
    let mut h = view(1000.0, 700.0, 512);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, Vec3::ZERO, 2.5, 0.0, 0.0);
    h.state_mut().view3d_set_paint_budget(BUDGET);
    h.run();
    let stats = |h: &Harness<'_, YoluApp>| h.state().view3d_stats().expect("wgpu の 3D");
    // 使っているのは Color だけ（512² のミップ込みで約 1.4 MiB）: 予算に収まるので縮めない
    let s = stats(&h);
    assert_eq!((s.paint_level, s.paint_by_budget), (0, false), "{s:?}");
    assert!(s.paint_bytes > 0 && s.paint_bytes <= BUDGET, "{s:?}");
    assert!(h.query_by_label_contains("縮小表示 1/2").is_none());
    // Emission を使い始めると 8 バイト × 512² で予算（2 MiB）を超える: 確保する前に縮めを上げて作り直す
    let layer = first_layer(&h);
    h.state_mut()
        .state
        .doc
        .set_channel_pixel(
            layer,
            Channel::Emission,
            10,
            10,
            Rgba8::new(255, 255, 255, 255),
        )
        .unwrap();
    h.run();
    let s = stats(&h);
    assert_eq!((s.paint_level, s.paint_by_budget), (1, true), "{s:?}");
    assert!(s.last_rebuilt, "縮めが変わったので作り直した: {s:?}");
    assert!(
        s.paint_bytes > 0 && s.paint_bytes <= BUDGET,
        "予算を超えて確保していない: {} バイト",
        s.paint_bytes
    );
    // 縮めたことを 3D の右上の隅の警告のアイコンで出す（名前と理由はツールチップ）
    h.get_by_label_contains("縮小表示 1/2");
    // 縮めた後も、変わったタイルだけを上げる（Color の 1 タイル）。何も変わらなければ作り直さない・描かない
    h.state_mut()
        .state
        .doc
        .set_channel_pixel(layer, Channel::Color, 400, 400, Rgba8::new(255, 0, 0, 255))
        .unwrap();
    h.run();
    let s = stats(&h);
    assert_eq!(s.last_slot_tiles, [1, 0, 0, 0, 0, 0], "{s:?}");
    assert!(!s.last_rebuilt);
    assert_eq!(s.paint_level, 1);
    let idle = stats(&h);
    h.run();
    assert_eq!(stats(&h).renders, idle.renders);
    assert!(!stats(&h).last_rebuilt);
    // 予算の中なら 8 バイトでも縮めない文書: 同じ予算で 256² は 8 バイト × 約 87 K テクセル = 0.7 MiB
    let mut small = view(1000.0, 700.0, 256);
    set_model(&mut small, vec![quad(1.0)]);
    small.state_mut().view3d_set_paint_budget(BUDGET);
    let layer = first_layer(&small);
    small
        .state_mut()
        .state
        .doc
        .set_channel_pixel(
            layer,
            Channel::Emission,
            10,
            10,
            Rgba8::new(255, 255, 255, 255),
        )
        .unwrap();
    small.run();
    let s = stats(&small);
    assert_eq!(s.paint_level, 0, "{s:?}");
    assert!(s.paint_bytes <= BUDGET);
}

#[test]
fn a_mesh_map_over_the_budget_is_shown_smaller_and_the_budget_is_not_exceeded() {
    use yolu_core::mesh_maps::MeshMapKind;
    let mut h = view(900.0, 640.0, 64);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, Vec3::ZERO, 2.5, 0.0, 0.0);
    // 予算 64 KiB（メッシュマップ 1 枚はその半分）。128² の RGBA はミップ込みで約 85 KiB なので、1 段縮めて 64² にする（約 21 KiB）
    h.state_mut().view3d_set_paint_budget(64 << 10);
    h.run();
    {
        let state = &mut h.state_mut().state;
        let index = state.sets.current_index();
        let maps = &mut state.sets.get_mut(index).unwrap().mesh_maps;
        maps.put(vec![synthetic_map_sized(
            MeshMapKind::AmbientOcclusion,
            128,
            |x, _| x as f32 / 127.0,
        )]);
    }
    op(
        &mut h,
        Op::Shading(Shading::MeshMap(MeshMapKind::AmbientOcclusion)),
    );
    assert_eq!(h.state().view3d_stats().unwrap().map_level, 1);
    // 縮めても値は同じ（AO は U に沿って 0 → 1。中心は 0.5）
    let g = (0.5f32 * 255.0).round() as u8;
    let at = center(&h);
    let image = h.render().unwrap();
    assert_close(px(&image, at), [g, g, g], 4, "縮めたメッシュマップの中心");
    // 同じマップは作り直さない（毎フレーム作らない）
    let renders = h.state().view3d_stats().unwrap().renders;
    h.step();
    h.step();
    assert_eq!(h.state().view3d_stats().unwrap().renders, renders);
}

/// 全部の段のバイト列（チャンネルのテクスチャの中身）。
fn read_all_levels(h: &Harness<'_, YoluApp>, slot: Slot) -> Vec<(Vec<u8>, [u32; 2])> {
    (0..)
        .map_while(|level| h.state().view3d_read_paint_level(slot, level))
        .collect()
}

/// 矩形（文書の画素。端で切る）を 1 画素ずつ塗る。
fn paint_block(
    h: &mut Harness<'_, YoluApp>,
    layer: LayerId,
    channel: Channel,
    (x, y, w, hgt): (u32, u32, u32, u32),
    value: Rgba8,
) {
    let size = h.state().state.doc.width();
    let doc = &mut h.state_mut().state.doc;
    for py in y..(y + hgt).min(size) {
        for px in x..(x + w).min(size) {
            doc.set_channel_pixel(layer, channel, px, py, value)
                .unwrap();
        }
    }
}

/// 塗ったあとの部分更新（変わった範囲だけミップを作り直す）が、同じ文書を捨てて全部を作り直した結果と、全部の段で 1 バイトも違わないこと。
fn partial_mips_equal_a_full_rebuild(doc_size: u32, budget: Option<u64>) {
    let mut h = view(900.0, 640.0, doc_size);
    set_model(&mut h, vec![quad(1.0)]);
    if let Some(budget) = budget {
        h.state_mut().view3d_set_paint_budget(budget);
    }
    let layer = first_layer(&h);
    let last = doc_size - 1;
    let channels: [(Channel, Slot, [Rgba8; 2]); 3] = [
        (
            Channel::Color,
            Slot::Color,
            [Rgba8::new(255, 20, 10, 255), Rgba8::new(10, 40, 250, 200)],
        ),
        (
            Channel::Roughness,
            Slot::Roughness,
            [Rgba8::new(250, 0, 0, 255), Rgba8::new(10, 0, 0, 255)],
        ),
        (
            Channel::Normal,
            Slot::Normal,
            [
                Rgba8::new(217, 128, 217, 255),
                Rgba8::new(60, 200, 200, 255),
            ],
        ),
    ];
    // 初めの絵（タイルの境をまたぐ大きな塊と、端の細い線）を作って、全部を作った状態から始める
    for (channel, _, values) in &channels {
        paint_block(&mut h, layer, *channel, (100, 90, 70, 70), values[0]);
        paint_block(
            &mut h,
            layer,
            *channel,
            (last - 1, 5, 2, last - 10),
            values[1],
        );
    }
    h.run();
    // 変えるところ: 隅・端・タイルの境（127 | 128）・真ん中・奇数の大きさの最後の画素
    let edits: [(u32, u32, u32, u32); 8] = [
        (0, 0, 64, 64),
        (last - 63, last - 63, 64, 64),
        (96, 96, 64, 64),
        (last, 0, 1, 1),
        (0, last, 1, 1),
        (doc_size / 2 - 30, doc_size / 2 - 30, 61, 61),
        (127, 127, 2, 2),
        (last - 1, doc_size / 3, 2, 90),
    ];
    for (i, edit) in edits.iter().enumerate() {
        for (channel, _, values) in &channels {
            paint_block(&mut h, layer, *channel, *edit, values[i % 2]);
        }
        h.run();
        let s = h.state().view3d_stats().unwrap();
        assert!(!s.last_rebuilt, "{i}: 部分の更新のはず {s:?}");
        let partial: Vec<_> = channels
            .iter()
            .map(|(_, slot, _)| read_all_levels(&h, *slot))
            .collect();
        // 捨てて、同じ文書から全部を作り直す
        h.state_mut().view3d_invalidate_paint();
        h.run();
        assert!(h.state().view3d_stats().unwrap().last_rebuilt);
        for ((_, slot, _), partial) in channels.iter().zip(&partial) {
            let full = read_all_levels(&h, *slot);
            assert_eq!(partial.len(), full.len(), "{slot:?} の段の数");
            assert!(partial.len() > 4, "ミップの段がある");
            for (level, (a, b)) in partial.iter().zip(&full).enumerate() {
                assert_eq!(a.1, b.1, "{slot:?} 段 {level} の大きさ");
                if a.0 != b.0 {
                    let bytes = (a.0.len() / (a.1[0] * a.1[1]) as usize).max(1);
                    let at = a.0.iter().zip(&b.0).position(|(x, y)| x != y).unwrap();
                    let texel = at / bytes;
                    panic!(
                        "編集 {i} {edit:?}: {slot:?} の段 {level}（{}×{}）のテクセル ({}, {}) が、部分の更新 {:?} と全面の作り直し {:?} で違う",
                        a.1[0],
                        a.1[1],
                        texel as u32 % a.1[0],
                        texel as u32 / a.1[0],
                        &a.0[texel * bytes..(texel + 1) * bytes],
                        &b.0[texel * bytes..(texel + 1) * bytes],
                    );
                }
            }
        }
    }
}

#[test]
fn partial_mip_updates_equal_a_full_rebuild_on_an_odd_sized_document() {
    // 1001²（奇数。段ごとの大きさが 500・250・125・62・31・15・7・3・1）
    partial_mips_equal_a_full_rebuild(1001, None);
}

#[test]
fn partial_mip_updates_equal_a_full_rebuild_when_the_document_is_shown_shrunk() {
    // 予算で 1 段縮めた 501²（上げる矩形は縮めの境に合わせて広げる）
    partial_mips_equal_a_full_rebuild(1001, Some(8 << 20));
}

#[test]
fn the_channel_textures_hold_the_same_texels_as_the_export() {
    use yolu_core::export::{build, ExportTemplate};
    let mut h = view(900.0, 640.0, 200);
    set_model(&mut h, vec![quad(1.0)]);
    let layer = first_layer(&h);
    // Height → Normal を有効に（Normal の出力に Height から作った法線が入る）
    h.state_mut()
        .state
        .doc
        .set_normal_settings(
            NormalSettings::new(true, 6.0, HeightEdgeMode::Clamp, NormalYDirection::OpenGL)
                .unwrap(),
            false,
        )
        .unwrap();
    // 全チャンネルに、不透明・半透明・塗っていない画素ができるように塗る
    let patch = |h: &mut Harness<'_, YoluApp>, channel, at: (u32, u32), opaque: [u8; 3], alpha| {
        paint_block(
            h,
            layer,
            channel,
            (at.0, at.1, 20, 20),
            Rgba8::new(opaque[0], opaque[1], opaque[2], 255),
        );
        paint_block(
            h,
            layer,
            channel,
            (at.0 + 20, at.1, 20, 20),
            Rgba8::new(opaque[0], opaque[1], opaque[2], alpha),
        );
    };
    patch(&mut h, Channel::Color, (10, 10), [200, 120, 60], 100);
    patch(&mut h, Channel::Emission, (10, 50), [255, 128, 9], 128);
    patch(&mut h, Channel::Roughness, (10, 90), [200, 0, 0], 77);
    patch(&mut h, Channel::Metallic, (60, 90), [133, 0, 0], 201);
    patch(&mut h, Channel::Height, (10, 130), [220, 220, 220], 90);
    patch(&mut h, Channel::Normal, (100, 10), [217, 128, 217], 160);
    h.run();
    let doc = &h.state().state.doc;
    let template = ExportTemplate::unity_standard();
    let export = |name: &str| {
        build(
            doc,
            template.image(name).expect("テンプレートの画像"),
            None,
            u64::MAX,
        )
        .unwrap()
    };
    let texels = (200 * 200) as usize;
    let level0 = |slot: Slot| {
        let (bytes, size) = h
            .state()
            .view3d_read_paint_level(slot, 0)
            .expect("使っているチャンネルは作ってある");
        assert_eq!(size, [200, 200], "{slot:?} は縮めていない");
        bytes
    };
    // Normal: 書き出しの法線（塗った法線に Height から作った法線を重ねたもの）と全テクセルが同じ
    let (gpu, out) = (level0(Slot::Normal), export("Normal"));
    assert_eq!(gpu, out, "Normal");
    assert!(
        out.as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[..3] != [128, 128, 255]),
        "平らでない法線がある（Height と塗った法線）"
    );
    // Emission: RGB は書き出し（値 × アルファ）と全テクセルが同じ（アルファは 3D では読まない）
    let (gpu, out) = (level0(Slot::Emission), export("Emission"));
    for i in 0..texels {
        assert_eq!(gpu[i * 4..i * 4 + 3], out[i * 4..i * 4 + 3], "Emission {i}");
    }
    assert!(out[(60 * 200 + 35) * 4] > 0, "半透明の Emission がある");
    // Metallic・Height（値 × アルファ）と、Roughness（書き出しは Smoothness = 255 − 値 × アルファ）
    let (metallic, height) = (level0(Slot::Metallic), level0(Slot::Height));
    let roughness = level0(Slot::Roughness);
    let (packed, height_out) = (export("MetallicSmoothness"), export("Height"));
    for i in 0..texels {
        assert_eq!(metallic[i], packed[i * 4], "Metallic {i}");
        assert_eq!(255 - roughness[i], packed[i * 4 + 3], "Roughness {i}");
        assert_eq!(height[i], height_out[i * 4], "Height {i}");
    }
    assert!(
        (0..texels).any(|i| metallic[i] != 0 && metallic[i] != 133),
        "半透明の Metallic は値 × アルファ"
    );
    // Color: 不透明な画素は書き出しと同じ。半透明・透明は、書き出しが straight（透明の画素の RGB を保つ）のところを、3D は見せるために
    // 乗算済みで上げる。sRGB の形式（GPU が読むときにリニアにして補間する）なので、乗算はリニアで（sRGB(リニア(RGB) × アルファ)、
    // アルファは同じ。16 bit のリニアを経る丸めの 1 まで）
    let (gpu, out) = (level0(Slot::Color), export("Albedo"));
    let mut translucent = 0;
    for i in 0..texels {
        let (g, o) = (&gpu[i * 4..i * 4 + 4], &out[i * 4..i * 4 + 4]);
        if o[3] == 255 {
            assert_eq!(g, o, "不透明な Color {i}");
        } else {
            let a = o[3] as f32 / 255.0;
            for k in 0..3 {
                let linear = brdf::srgb_to_linear(o[k] as f32 / 255.0) * a;
                let expected = (brdf::linear_to_srgb(linear) * 255.0).round() as i32;
                assert!(
                    (g[k] as i32 - expected).abs() <= 1,
                    "Color {i}: {g:?} と {expected}"
                );
            }
            assert_eq!(g[3], o[3]);
            translucent += usize::from(o[3] > 0 && g[..3] != o[..3]);
        }
    }
    assert!(
        translucent > 0,
        "半透明の Color は書き出しと値が違う（乗算済み）"
    );
}

#[test]
fn the_srgb_channels_average_their_mips_in_linear() {
    // Color・Emission は sRGB の形式で持ち、GPU がミップを作るときもリニアで平均する（リニアの色空間の Unity が sRGB のテクスチャの
    // ミップを作るのと同じ）。1 画素の白黒の市松を 1 段縮めると、リニアの平均 0.5 → sRGB の 188（ガンマの値のまま平均すると 128）。
    // 値のチャンネル（Roughness。リニアの形式）は値のまま平均する
    let mut h = view(900.0, 640.0, 64);
    set_model(&mut h, vec![quad(1.0)]);
    let layer = first_layer(&h);
    {
        let doc = &mut h.state_mut().state.doc;
        for y in 0..64 {
            for x in 0..64 {
                let v = if (x + y) % 2 == 0 { 255 } else { 0 };
                for channel in [Channel::Color, Channel::Emission, Channel::Roughness] {
                    doc.set_channel_pixel(layer, channel, x, y, Rgba8::new(v, v, v, 255))
                        .unwrap();
                }
            }
        }
    }
    h.run();
    for (slot, expected) in [
        (Slot::Color, 188u8),
        (Slot::Emission, 188),
        (Slot::Roughness, 128),
    ] {
        let (bytes, size) = h
            .state()
            .view3d_read_paint_level(slot, 1)
            .expect("ミップの 1 段");
        assert_eq!(size, [32, 32], "{slot:?}");
        let per_texel = bytes.len() / (32 * 32);
        for (i, texel) in bytes.chunks(per_texel).enumerate() {
            let rgb = &texel[..per_texel.min(3)];
            assert!(
                rgb.iter().all(|v| v.abs_diff(expected) <= 2),
                "{slot:?} の 1 段の {i}: {texel:?}（期待 {expected}）"
            );
            if per_texel == 4 {
                assert_eq!(texel[3], 255, "{slot:?} の 1 段の {i}");
            }
        }
    }
}

#[test]
fn neutral_and_channel_only_views_follow_their_formulas() {
    let mut h = view(900.0, 640.0, 64);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, Vec3::ZERO, 2.5, 0.0, 0.0);
    head_on_light(&mut h);
    fill(
        &mut h,
        "値",
        &[
            (Channel::Color, [200, 120, 60, 255]),
            (Channel::Metallic, gray(255)),
            (Channel::Roughness, gray(77)),
            (Channel::Emission, [10, 20, 30, 255]),
        ],
    );
    let at = center(&h);
    let shot = |h: &mut Harness<'_, YoluApp>| px(&h.render().expect("描ける"), at);
    // 中立: 0.35 + 0.65 × saturate(n·L)（ガンマの空間）。真正面は 1、真横は 0.35。金属・粗さは見ない
    op(
        &mut h,
        Op::Shading(yolu_app::view3d::display::Shading::Neutral),
    );
    assert_close(shot(&mut h), [200, 120, 60], 1, "中立・真正面");
    op(&mut h, Op::LightYaw(90.0));
    assert_close(shot(&mut h), [70, 42, 21], 2, "中立・真横の光");
    // チャンネルだけ: 光・環境・トーンマッピングなしで値そのまま
    op(&mut h, Op::Exposure(2.0));
    op(&mut h, Op::Tone(Curve::Aces));
    op(&mut h, Op::LightIntensity(0.0));
    let tone_mapped = h.state().view3d_stats().unwrap().tone_mapped_renders;
    for (channel, want, what) in [
        (Channel::Color, [200, 120, 60], "Color"),
        (Channel::Roughness, [77, 77, 77], "Roughness"),
        (Channel::Metallic, [255, 255, 255], "Metallic"),
        (Channel::Emission, [10, 20, 30], "Emission"),
        // 使っていないチャンネルは書き出しの既定: Height 0、Normal は平ら
        (Channel::Height, [0, 0, 0], "Height（未使用）"),
        (Channel::Normal, [128, 128, 255], "Normal（未使用）"),
    ] {
        op(
            &mut h,
            Op::Shading(yolu_app::view3d::display::Shading::Channel(channel)),
        );
        assert_close(shot(&mut h), want, 1, what);
    }
    // 光なしの表示は環境の背景も出さない（背景は背景の色のまま）
    op(&mut h, Op::EnvBackground(true));
    op(&mut h, Op::Env(EnvKind::Sky));
    let image = h.render().unwrap();
    let corner = h.state().view3d_rect().unwrap().min + egui::vec2(4.0, 4.0);
    assert_close(px(&image, corner), [31, 33, 38], 1, "背景");
    assert_eq!(
        h.state().view3d_stats().unwrap().tone_mapped_renders,
        tone_mapped,
        "チャンネルだけの表示ではトーンマッピングを当てない"
    );
}

// ───────── トーンマッピングと露出 ─────────

/// 真正面の PBR の画面にそのまま出す値（ガンマ、1 を超えうる）。
fn display_value_head_on(color: [u8; 3], metallic: u8, roughness: u8) -> Vec3 {
    let albedo = brdf::srgb_to_linear3(Vec3::new(
        color[0] as f32 / 255.0,
        color[1] as f32 / 255.0,
        color[2] as f32 / 255.0,
    ));
    let flat = Vec3::splat(brdf::srgb_to_linear(0.2));
    let lit = brdf::standard_brdf(
        &Surface {
            albedo,
            metallic: metallic as f32 / 255.0,
            smoothness: 1.0 - roughness as f32 / 255.0,
            normal: Vec3::NEG_Z,
            emission: Vec3::ZERO,
        },
        Vec3::NEG_Z,
        &Light {
            to_light: Vec3::NEG_Z,
            radiance: Vec3::from(Display::default().direct_light()),
        },
        &Indirect {
            diffuse: flat,
            specular: flat,
        },
    );
    brdf::linear_to_srgb3(lit)
}

#[test]
fn tone_mapping_and_exposure_match_the_cpu_curves() {
    let mut h = view(900.0, 640.0, 64);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, Vec3::ZERO, 2.5, 0.0, 0.0);
    head_on_light(&mut h);
    let color = [235u8, 190, 150];
    fill(
        &mut h,
        "値",
        &[
            (Channel::Color, [color[0], color[1], color[2], 255]),
            (Channel::Metallic, gray(0)),
            (Channel::Roughness, gray(160)),
        ],
    );
    let at = center(&h);
    let base = display_value_head_on(color, 0, 160);
    assert_eq!(
        h.state().view3d_stats().unwrap().tone_mapped_renders,
        0,
        "既定は 8 bit へ直に"
    );
    for (curve, ev) in [
        (Curve::None, 0.0),
        (Curve::None, 1.0),
        (Curve::None, -1.0),
        (Curve::Neutral, 0.0),
        (Curve::Aces, 0.0),
        (Curve::Aces, 1.5),
        (Curve::Neutral, 2.0),
    ] {
        op(&mut h, Op::Tone(curve));
        op(&mut h, Op::Exposure(ev));
        let want = brdf::tone_map(base, curve, ev);
        assert_close(
            px(&h.render().expect("描ける"), at),
            [0, 1, 2].map(|k| (want[k] * 255.0).round() as u8),
            3,
            &format!("{curve:?}・露出 {ev}"),
        );
    }
    assert!(h.state().view3d_stats().unwrap().tone_mapped_renders >= 5);
    // 曲線は明るい所を抑える: ACES の +1.5 EV は、なしの +1.5 EV（切り詰め）より白飛びしない
    let aces = brdf::tone_map(base, Curve::Aces, 1.5);
    let none = brdf::tone_map(base, Curve::None, 1.5);
    assert!(
        none.x >= 0.99 && aces.x < none.x - 0.02,
        "{none:?} {aces:?}"
    );
}

/// 接線を作るスレッドの始まりを止めておく門（閉じているあいだ、接線は着かない。試験が着く前のフレームを決定的に作る）。
#[derive(Clone)]
struct Gate(Arc<(Mutex<bool>, Condvar)>);

impl Gate {
    fn closed() -> Gate {
        Gate(Arc::new((Mutex::new(false), Condvar::new())))
    }

    fn open(&self) {
        *self.0 .0.lock().unwrap() = true;
        self.0 .1.notify_all();
    }

    fn hook(&self) -> TangentHook {
        let gate = self.clone();
        Arc::new(move || {
            let mut open = gate.0 .0.lock().unwrap();
            while !*open {
                open = gate.0 .1.wait(open).unwrap();
            }
        })
    }
}

/// モデルを入れ替える（フレームは回さない。接線の門が閉じているとき、`run` は終わらない）。
fn swap_model(h: &mut Harness<'_, YoluApp>, meshes: Vec<ModelMesh>) {
    let state = &mut h.state_mut().state;
    let revision = state.view3d.next_revision();
    state.view3d.material = 0;
    let model = ViewModel::new("試験", meshes, vec![Some("試験".to_string())], revision).unwrap();
    state.view3d.set_model(model);
}

/// 接線が着くまで 1 フレームずつ回す（別のスレッドが作る）。
fn wait_for_tangents(h: &mut Harness<'_, YoluApp>) {
    for _ in 0..400 {
        h.step();
        if h.state().view3d_stats().unwrap().tangents_exact {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("接線が出来ない");
}

#[test]
fn tangents_are_made_in_the_background_and_a_replaced_model_keeps_the_normal_map() {
    let mut h = view(900.0, 640.0, 64);
    // 接線を先に作らずにモデルを入れる（法線マップを使う場面で、描きは待たずに始まる）
    set_model_with(&mut h, vec![quad(1.0)], false);
    look_at(&mut h, Vec3::ZERO, 2.5, 0.0, 0.0);
    op(&mut h, Op::Env(EnvKind::None));
    fill(
        &mut h,
        "面",
        &[
            (Channel::Color, [200, 200, 200, 255]),
            (Channel::Metallic, gray(0)),
            (Channel::Roughness, gray(255)),
            (Channel::Normal, [217, 128, 217, 255]),
        ],
    );
    wait_for_tangents(&mut h);
    let uploads = h.state().view3d_stats().unwrap().mesh_uploads;
    // 右へ傾けた法線マップは、接線が着いてから効く（右から光を当てると、左から当てるより明るい）
    let from_right = lit_luma(&mut h, 90.0, 0.0);
    let from_left = lit_luma(&mut h, -90.0, 0.0);
    assert!(from_right > from_left + 40.0, "{from_right} {from_left}");
    // ポーズのように、同じ形の新しいモデルへ入れ替える: 新しい接線が出来るまで前の接線で描き続け、法線マップは消えない。
    // 新しい接線の作業は門で止め、着く前のフレームを決定的に見る
    op(&mut h, Op::LightYaw(90.0));
    let gate = Gate::closed();
    h.state_mut().view3d_set_tangent_hook(Some(gate.hook()));
    swap_model(&mut h, vec![quad(1.0)]);
    for _ in 0..3 {
        h.step();
        let stats = h.state().view3d_stats().unwrap();
        assert!(
            !stats.tangents_exact,
            "入れ替えたモデルの接線はまだ着いていない（前の接線で描いている）"
        );
        assert!(h.state().view3d_wants_repaint(), "着いたら描き直したい");
        let held = luma(px(&h.render().expect("描ける"), center(&h)));
        assert!(
            (held - from_right).abs() < 3.0,
            "着く前のフレームも前の接線で法線マップが効く: {held} と {from_right}"
        );
    }
    // 門を開けると新しい接線が着いて、頂点が上がり直り、絵は変わらない
    gate.open();
    wait_for_tangents(&mut h);
    assert!(h.state().view3d_stats().unwrap().mesh_uploads > uploads);
    h.run();
    assert!(!h.state().view3d_wants_repaint());
    let settled = luma(px(&h.render().expect("描ける"), center(&h)));
    assert!((settled - from_right).abs() < 3.0, "{settled} {from_right}");
}

/// 法線マップを読む場面（接線を作っている最中）のウィンドウ: 接線の門を閉じたままモデルを入れ、Normal を塗ったレイヤーを置いた絵。
fn tangents_in_flight() -> (Harness<'static, YoluApp>, Gate, LayerId) {
    let mut h = view(900.0, 640.0, 64);
    let gate = Gate::closed();
    h.state_mut().view3d_set_tangent_hook(Some(gate.hook()));
    swap_model(&mut h, vec![quad(1.0)]);
    let layer = h
        .state_mut()
        .state
        .doc
        .add_fill_layer(
            "面",
            &[(Channel::Normal, Rgba8::new(217, 128, 217, 255))],
            None,
        )
        .unwrap();
    for _ in 0..3 {
        h.step();
    }
    (h, gate, layer)
}

#[test]
fn the_window_stops_asking_for_frames_when_the_tangents_are_no_longer_read() {
    let (mut h, gate, layer) = tangents_in_flight();
    assert!(
        h.state().view3d_wants_repaint(),
        "法線マップを読むあいだは、接線を作っている最中のウィンドウは次のフレームを求める"
    );
    // 光なしの表示（チャンネルだけ）へ替えると、接線は読まれない: 作っている最中でもウィンドウは描き直しを求めない
    h.state_mut()
        .apply(Action::View3d(Op::Shading(Shading::Channel(
            Channel::Color,
        ))));
    h.step();
    assert!(
        !h.state().view3d_wants_repaint(),
        "光なしの表示では求めない"
    );
    // 求めないので、ウィンドウは回り続けずに静まる（`run` が返る）
    h.run();
    // マテリアルへ戻すと、読むので求める（作業は続いている）
    h.state_mut()
        .apply(Action::View3d(Op::Shading(Shading::Material)));
    h.step();
    assert!(h.state().view3d_wants_repaint());
    // 法線マップを使うレイヤーが無くなっても同じ
    h.state_mut().state.doc.remove_layer(layer).unwrap();
    h.step();
    assert!(
        !h.state().view3d_wants_repaint(),
        "Normal を使うレイヤーが無ければ読まないので求めない"
    );
    gate.open();
    h.run();
}

#[test]
fn a_failed_tangent_worker_neither_keeps_the_window_repainting_nor_runs_again() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let mut h = view(900.0, 640.0, 64);
    h.state_mut()
        .view3d_set_tangent_hook(Some(Arc::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            panic!("試験: 接線の作業の失敗");
        })));
    swap_model(&mut h, vec![quad(1.0)]);
    h.state_mut()
        .state
        .doc
        .add_fill_layer(
            "面",
            &[(Channel::Normal, Rgba8::new(217, 128, 217, 255))],
            None,
        )
        .unwrap();
    // スレッドが終わるのを待つ（接線は残らない）
    for _ in 0..400 {
        h.step();
        if !h.state().view3d_wants_repaint() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        !h.state().view3d_wants_repaint(),
        "接線を残さずに終わったスレッドを待ち続けてウィンドウを回さない"
    );
    assert!(!h.state().view3d_stats().unwrap().tangents_exact);
    // ウィンドウは静まり、同じモデルでは作り直さない
    h.run();
    for _ in 0..5 {
        h.step();
    }
    assert!(!h.state().view3d_wants_repaint());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // 別のモデルは作る
    swap_model(&mut h, vec![quad(1.0)]);
    for _ in 0..400 {
        h.step();
        if calls.load(Ordering::SeqCst) == 2 && !h.state().view3d_wants_repaint() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

// ───────── 環境 ─────────

/// 球（中心 center・半径 radius）。UV は全部 uv の 1 点（その 1 テクセルの値だけで塗る）。法線は球の外向き。
fn ball(center: Vec3, radius: f32, uv: Vec2) -> ModelMesh {
    let mut m = cube_sphere(12, radius);
    for p in &mut m.positions {
        *p += center;
    }
    m.uvs.fill(uv);
    m
}

/// 3D の表示域の中の、明るい（最大の 95% 以上の）画素の重心と最大の明るさ。
fn highlight(image: &image::RgbaImage, area: egui::Rect) -> (Pos2, f32) {
    let (x0, y0) = (area.left().ceil() as u32, area.top().ceil() as u32);
    let (x1, y1) = (area.right().floor() as u32, area.bottom().floor() as u32);
    let mut max = 0.0f32;
    for y in y0..y1 {
        for x in x0..x1 {
            let c = image.get_pixel(x, y).0;
            max = max.max(luma([c[0], c[1], c[2]]));
        }
    }
    let (mut sx, mut sy, mut n) = (0.0f64, 0.0f64, 0.0f64);
    for y in y0..y1 {
        for x in x0..x1 {
            let c = image.get_pixel(x, y).0;
            if luma([c[0], c[1], c[2]]) >= max * 0.95 {
                sx += x as f64;
                sy += y as f64;
                n += 1.0;
            }
        }
    }
    (pos2_of(sx / n, sy / n), max)
}

fn pos2_of(x: f64, y: f64) -> Pos2 {
    Pos2::new(x as f32, y as f32)
}

fn shiny_metal_ball() -> Harness<'static, YoluApp> {
    let mut h = view(900.0, 640.0, 32);
    set_model(&mut h, vec![ball(Vec3::ZERO, 0.5, Vec2::new(0.5, 0.5))]);
    look_at(&mut h, Vec3::ZERO, 3.2, 0.0, 0.0);
    fill(
        &mut h,
        "金属",
        &[
            (Channel::Color, [255, 255, 255, 255]),
            (Channel::Metallic, gray(255)),
            (Channel::Roughness, gray(25)),
        ],
    );
    h
}

#[test]
fn the_studio_environment_reflects_off_metal_and_rotates() {
    let mut h = shiny_metal_ball();
    op(&mut h, Op::Env(EnvKind::Studio));
    // 面光源がどれも白く飽和しないよう、環境を暗くして、いちばん明るい 1 つ（正面の右上の面光源）だけを探す
    op(&mut h, Op::EnvIntensity(0.12));
    op(&mut h, Op::LightIntensity(0.0));
    let area = h.state().view3d_rect().unwrap();
    let c = area.center();
    // 右上の隅のアイコンの列（白いアイコン）と、その下の軸の印は、映り込みの測定に入れない
    let axes = view3d_axes_rect(&h).expect("軸の印");
    let scene = egui::Rect::from_min_max(
        egui::pos2(area.left(), area.top() + 44.0),
        egui::pos2(axes.left(), area.bottom()),
    );
    let (before, peak) = highlight(&h.render().unwrap(), scene);
    assert!(peak > 120.0, "つるつるの金属は面光源を映す: {peak}");
    // 正面の右上の面光源（方位 150°・高さ 40°）は、球の右上に映る
    assert!(
        before.x > c.x + 5.0 && before.y < c.y - 5.0,
        "右上: {before:?}（中心 {c:?}）"
    );
    // 環境を 60° 回すと、面光源は左上へ移る（150° → 210°）
    op(&mut h, Op::EnvRotation(60.0));
    let (after, _) = highlight(&h.render().unwrap(), scene);
    assert!(
        after.x < c.x - 5.0 && after.y < c.y - 5.0,
        "左上: {after:?}（中心 {c:?}）"
    );
    // 環境の明るさ 0 は、映り込みも拡散も無い（金属は黒）
    op(&mut h, Op::EnvIntensity(0.0));
    let (_, dark) = highlight(&h.render().unwrap(), scene);
    assert!(dark < 40.0, "{dark}");
    assert!(h.state().view3d_stats().unwrap().env_bakes >= 1);
}

#[test]
fn the_built_in_sky_is_symmetric_about_the_up_axis_and_lights_from_above() {
    let mut h = shiny_metal_ball();
    op(&mut h, Op::Env(EnvKind::Sky));
    op(&mut h, Op::LightIntensity(0.0));
    let area = h.state().view3d_rect().unwrap();
    let c = area.center();
    let probe = |h: &mut Harness<'_, YoluApp>, dy: f32| {
        let image = h.render().unwrap();
        px(&image, Pos2::new(c.x, c.y + dy))
    };
    let top = probe(&mut h, -60.0);
    let bottom = probe(&mut h, 60.0);
    // 球の上は空（青い・明るい）を、下は地面（暗い・褐色）を映す
    assert!(
        top[2] > top[0] && luma(top) > luma(bottom) + 20.0,
        "{top:?} {bottom:?}"
    );
    let before = probe(&mut h, -60.0);
    op(&mut h, Op::EnvRotation(90.0));
    let after = probe(&mut h, -60.0);
    assert_close(after, before, 1, "空は上の軸のまわりに回しても同じ");
    // 環境を背景に映す: 上は空、下は地面、背景の色は出ない
    let corner = area.min + egui::vec2(4.0, 4.0);
    assert_close(
        px(&h.render().unwrap(), corner),
        [31, 33, 38],
        1,
        "背景の色",
    );
    op(&mut h, Op::EnvBackground(true));
    let image = h.render().unwrap();
    let (sky, ground) = (
        px(&image, Pos2::new(area.center().x, area.top() + 6.0)),
        px(&image, Pos2::new(area.center().x, area.bottom() - 6.0)),
    );
    assert!(
        sky[2] > sky[0] && luma(sky) > luma(ground) + 20.0,
        "{sky:?} {ground:?}"
    );
    assert!(
        px(&image, corner) != [31, 33, 38],
        "背景に環境を映している: {:?}",
        px(&image, corner)
    );
}

// ───────── 影 ─────────

/// 水平な床（y = 0、外向きの法線は +Y、一辺 size）。UV は (0,0)〜(1,1)。
fn floor(size: f32) -> ModelMesh {
    let h = size * 0.5;
    ModelMesh {
        name: "床".into(),
        positions: vec![
            Vec3::new(-h, 0.0, -h),
            Vec3::new(h, 0.0, -h),
            Vec3::new(-h, 0.0, h),
            Vec3::new(h, 0.0, h),
        ],
        normals: vec![Vec3::Y; 4],
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

/// 床の上に浮かぶ球。光は +X の側の斜め上（高さ 55°）、カメラは +Z の側の斜め上。床の影は球の −X の側に落ち、カメラから見える。
/// 面は一様な粗い白（拡散だけ）で、環境なし・一様な環境光。
fn ball_over_floor() -> Harness<'static, YoluApp> {
    let mut h = view(900.0, 640.0, 32);
    set_model(
        &mut h,
        vec![
            floor(4.0),
            ball(Vec3::new(0.0, 0.8, 0.0), 0.4, Vec2::new(0.5, 0.5)),
        ],
    );
    look_at(&mut h, Vec3::new(0.0, 0.3, 0.0), 5.0, 180.0, 55.0);
    fill(
        &mut h,
        "白",
        &[
            (Channel::Color, [230, 230, 230, 255]),
            (Channel::Metallic, gray(0)),
            (Channel::Roughness, gray(255)),
        ],
    );
    op(&mut h, Op::Env(EnvKind::None));
    op(&mut h, Op::LightYaw(90.0));
    op(&mut h, Op::LightPitch(55.0));
    h
}

/// 世界の点が見えている画素（3D の表示域の左上から測った点を、画像の座標へ）。
fn screen_of(h: &Harness<'_, YoluApp>, world: Vec3) -> Pos2 {
    let area = h.state().view3d_rect().unwrap();
    let view = h
        .state()
        .state
        .view3d
        .camera
        .view(area.width(), area.height());
    let p = view.to_screen(world).expect("カメラの前");
    area.min + egui::vec2(p.x, p.y)
}

#[test]
fn shadows_darken_only_what_the_ball_covers_and_leave_lit_surfaces_alone() {
    let mut h = ball_over_floor();
    let in_shadow = screen_of(&h, Vec3::new(-0.8, 0.0, 0.0));
    let far_lit = screen_of(&h, Vec3::new(1.2, 0.0, 0.9));
    // 球の光に向いた面（光とカメラの間を向く法線）。影の使い方の自己の影（にきび）が出ていないかを見る
    let light = Vec3::new(55f32.to_radians().cos(), 55f32.to_radians().sin(), 0.0);
    let camera = Vec3::new(0.0, 55f32.to_radians().sin(), 55f32.to_radians().cos());
    let lit_points: Vec<Vec3> = [(1.0, 1.0), (2.0, 1.0), (1.0, 2.0), (1.0, 0.3)]
        .iter()
        .map(|(a, b)| {
            let n = (light * *a + camera * *b).normalize();
            Vec3::new(0.0, 0.8, 0.0) + n * 0.4
        })
        .collect();

    let off = h.render().unwrap();
    assert!(
        !h.state().view3d_stats().unwrap().shadow_active,
        "既定は影なし"
    );
    op(&mut h, Op::Shadows(true));
    let on = h.render().unwrap();
    let stats = h.state().view3d_stats().unwrap();
    assert!(stats.shadow_active && stats.shadow_renders == 1);

    // 球の影の中の床は暗くなる（環境光だけ。直接光の分が消える）
    let (a, b) = (luma(px(&off, in_shadow)), luma(px(&on, in_shadow)));
    assert!(b < a * 0.7, "床の影: {a} → {b}");
    // 球から離れた床・球の光に向いた面は変わらない
    assert_close(px(&on, far_lit), px(&off, far_lit), 2, "離れた床");
    for p in lit_points {
        let at = screen_of(&h, p);
        assert_close(px(&on, at), px(&off, at), 2, &format!("球の明るい面 {p:?}"));
    }
    // 中立の表示も同じ影を読む（影の中は環境光だけ）
    op(
        &mut h,
        Op::Shading(yolu_app::view3d::display::Shading::Neutral),
    );
    let neutral_on = h.render().unwrap();
    op(&mut h, Op::Shadows(false));
    let neutral_off = h.render().unwrap();
    op(&mut h, Op::Shadows(true));
    let (a, b) = (
        luma(px(&neutral_off, in_shadow)),
        luma(px(&neutral_on, in_shadow)),
    );
    assert!(b < a * 0.7, "中立の表示の床の影: {a} → {b}");
    assert_close(
        px(&neutral_on, far_lit),
        px(&neutral_off, far_lit),
        2,
        "中立の離れた床",
    );
    // 光なしの表示（チャンネルだけ）は影を使わない
    op(
        &mut h,
        Op::Shading(yolu_app::view3d::display::Shading::Channel(Channel::Color)),
    );
    assert!(!h.state().view3d_stats().unwrap().shadow_active);
    let unlit = h.render().unwrap();
    assert_close(
        px(&unlit, in_shadow),
        [230, 230, 230],
        1,
        "チャンネルだけの表示は影を使わない",
    );
}

#[test]
fn the_shadow_map_is_redrawn_only_when_the_light_or_the_model_changes() {
    let mut h = ball_over_floor();
    let renders = |h: &Harness<'_, YoluApp>| h.state().view3d_stats().unwrap().shadow_renders;
    assert_eq!(renders(&h), 0, "影なしでは描かない");
    op(&mut h, Op::Shadows(true));
    assert_eq!(renders(&h), 1);
    // カメラ・柔らかさ・表示の切り替え・露出では描き直さない
    look_at(&mut h, Vec3::new(0.0, 0.3, 0.0), 4.0, 150.0, 40.0);
    op(&mut h, Op::ShadowSoftness(0.8));
    op(&mut h, Op::Exposure(0.5));
    assert_eq!(renders(&h), 1);
    // 光の向きが変われば描き直す
    op(&mut h, Op::LightYaw(60.0));
    assert_eq!(renders(&h), 2);
    op(&mut h, Op::LightIntensity(2.0));
    assert_eq!(renders(&h), 2, "光の強さは影の形に効かない");
    // 切って入れ直しても、同じ光・同じモデルなら前の深さをそのまま使う
    op(&mut h, Op::Shadows(false));
    op(&mut h, Op::Shadows(true));
    assert_eq!(renders(&h), 2);
    // モデルが替われば描き直す
    set_model(
        &mut h,
        vec![
            floor(4.0),
            ball(Vec3::new(0.0, 0.9, 0.0), 0.4, Vec2::new(0.5, 0.5)),
        ],
    );
    assert_eq!(renders(&h), 3);
}

#[test]
fn a_softer_shadow_has_a_wider_penumbra() {
    let mut h = ball_over_floor();
    op(&mut h, Op::Shadows(true));
    // 床の影の縁を横切る線（床の上で x = −1.25 から −0.6 まで。カメラから見て右へ向かう。始まりは表示域の端に掛からない所）。完全な影と
    // 明るい床のあいだの途中の値の画素を数える
    let (from, to) = (
        screen_of(&h, Vec3::new(-1.25, 0.0, 0.0)),
        screen_of(&h, Vec3::new(-0.6, 0.0, 0.0)),
    );
    let penumbra = |h: &mut Harness<'_, YoluApp>, softness: f32| -> usize {
        op(h, Op::ShadowSoftness(softness));
        let image = h.render().unwrap();
        let n = 400;
        let values: Vec<f32> = (0..=n)
            .map(|i| from.lerp(to, i as f32 / n as f32))
            .map(|p| luma(px(&image, p)))
            .collect();
        let (lit, dark) = (
            values.iter().cloned().fold(f32::MIN, f32::max),
            values.iter().cloned().fold(f32::MAX, f32::min),
        );
        assert!(lit - dark > 30.0, "線は影の縁を横切る: {lit} {dark}");
        let (lo, hi) = (dark + (lit - dark) * 0.15, dark + (lit - dark) * 0.85);
        let mut pixels: Vec<(u32, u32)> = (0..=n)
            .map(|i| from.lerp(to, i as f32 / n as f32))
            .map(|p| (p.x.round() as u32, p.y.round() as u32))
            .collect();
        pixels.dedup();
        pixels
            .iter()
            .filter(|(x, y)| {
                let c = image.get_pixel(*x, *y).0;
                let l = luma([c[0], c[1], c[2]]);
                l > lo && l < hi
            })
            .count()
    };
    let sharp = penumbra(&mut h, 0.0);
    let soft = penumbra(&mut h, 1.0);
    assert!(soft > sharp * 3 && soft >= sharp + 8, "{sharp} → {soft}");
}

// ───────── 右上の隅の表示の切り替えと設定のパネル ─────────

#[test]
fn the_corner_shading_icon_switches_what_is_shown() {
    use egui_kittest::kittest::Queryable;
    let mut h = view(1100.0, 760.0, 64);
    h.state_mut().state.view3d.load_demo();
    h.run();
    assert_eq!(h.state().state.view3d.display.shading, Shading::Material);
    // 右上の隅のアイコン（3D の表示: マテリアル (PBR)）を押すと一覧が開き、並びは Unity 版と同じ（マテリアル・中立・チャンネルだけ・メッシュマップ）
    h.get_by_label("3D の表示: マテリアル (PBR)").click();
    h.run();
    let body = h
        .state()
        .state
        .popup
        .as_ref()
        .expect("一覧が開く")
        .state
        .rect;
    for label in [
        "中立",
        "カラー",
        "ラフネス",
        "メタリック",
        "ノーマル",
        "エミッション",
        "ハイト",
        "アンビエントオクルージョン",
    ] {
        assert!(
            h.query_all_by_label(label)
                .any(|n| body.contains(n.rect().center())),
            "{label} が一覧に無い"
        );
    }
    // メッシュマップは、今のセットに焼いてあるものだけ選べる（焼いていなければ押しても変わらない）
    let before = h.state().state.view3d.display.shading;
    let at = popup_item(&h, "アンビエントオクルージョン").center();
    click(&mut h, at);
    h.run();
    assert_eq!(h.state().state.view3d.display.shading, before);
    if h.state().state.popup.is_none() {
        h.get_by_label("3D の表示: マテリアル (PBR)").click();
        h.run();
    }
    let at = popup_item(&h, "中立").center();
    click(&mut h, at);
    h.run();
    assert_eq!(h.state().state.view3d.display.shading, Shading::Neutral);
    assert!(h.state().state.popup.is_none());
    h.get_by_label("3D の表示: 中立").click();
    h.run();
    let at = popup_item(&h, "ラフネス").center();
    click(&mut h, at);
    h.run();
    assert_eq!(
        h.state().state.view3d.display.shading,
        Shading::Channel(Channel::Roughness)
    );
    // 英語の画面では英語の名前
    h.state_mut().state.lang = yolu_app::lang::Lang::En;
    h.state_mut()
        .state
        .apply(Action::View3d(Op::Shading(Shading::Material)));
    h.run();
    assert!(h.query_by_label("3D shading: Material (PBR)").is_some());
}

#[test]
fn the_settings_panel_edits_lighting_and_closes_outside_or_with_escape() {
    use egui_kittest::kittest::Queryable;
    let mut h = view(1100.0, 760.0, 64);
    h.state_mut().state.view3d.load_demo();
    h.run();
    assert!(!h.state().state.view3d.display.settings_open);
    h.get_by_label("光・環境・トーンマッピング").click();
    h.run();
    assert!(h.state().state.view3d.display.settings_open);
    // 環境の元・トーンマッピングの曲線は、ボタンを押して選ぶ
    h.get_by_label("スタジオ").click();
    h.run();
    assert_eq!(h.state().state.view3d.display.env, EnvKind::Studio);
    assert!(h.state().view3d_stats().unwrap().env_bakes >= 1);
    h.get_by_label("ACES").click();
    h.run();
    assert_eq!(h.state().state.view3d.display.tone_map, Curve::Aces);
    // 影のオン・オフ
    assert!(!h.state().state.view3d.display.shadows);
    h.get_by_label("影").click();
    h.run();
    assert!(h.state().state.view3d.display.shadows);
    // 既定に戻す: 光・環境・影・トーンマッピングだけ（何を見せるかは変えない）
    h.state_mut().apply(Action::View3d(Op::Shading(
        yolu_app::view3d::display::Shading::Neutral,
    )));
    h.get_by_label("既定に戻す").click();
    h.run();
    let d = h.state().state.view3d.display;
    assert_eq!((d.env, d.tone_map), (EnvKind::Sky, Curve::None));
    assert!(!d.shadows, "影も既定に戻る");
    assert_eq!(d.shading, yolu_app::view3d::display::Shading::Neutral);
    // Esc で閉じる
    key(&h, egui::Key::Escape, egui::Modifiers::NONE);
    h.run();
    assert!(!h.state().state.view3d.display.settings_open);
    // 開き直して、パネルの外（3D の絵の上）を押すと閉じる（その押しが 3D で描き始め・回し始めにならないことは、
    // `pressing_outside_the_settings_panel_closes_it_without_painting_or_orbiting` が確かめる）
    h.get_by_label("光・環境・トーンマッピング").click();
    h.run();
    assert!(h.state().state.view3d.display.settings_open);
    let rect = h.state().view3d_rect().unwrap();
    click(&mut h, rect.left_top() + egui::vec2(60.0, 300.0));
    h.run();
    assert!(!h.state().state.view3d.display.settings_open);
    assert!(!h.state().state.doc.can_undo(), "閉じる押しで描かない");
    // 設定のパネルを開いている間、パネルの上のドラッグで描かない
    h.get_by_label("光・環境・トーンマッピング").click();
    h.run();
    let panel_point = egui::pos2(rect.right() - 60.0, rect.top() + 60.0);
    drag(&mut h, &[panel_point, panel_point + egui::vec2(20.0, 10.0)]);
    assert!(!h.state().state.doc.can_undo(), "パネルの上では描かない");
}

#[test]
fn pressing_outside_the_settings_panel_closes_it_without_painting_or_orbiting() {
    use egui_kittest::kittest::Queryable;
    let mut h = view(1100.0, 760.0, 64);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, Vec3::ZERO, 2.5, 0.0, 0.0);
    // モデルの上（パネルは右上に開くので、中心より左）
    let on_model = center(&h) - egui::vec2(60.0, 0.0);
    let open_panel = |h: &mut Harness<'_, YoluApp>| {
        h.get_by_label("光・環境・トーンマッピング").click();
        h.run();
        assert!(h.state().state.view3d.display.settings_open);
    };
    // 左ボタン: モデルの上を押してパネルを閉じる。その押しは 1 ダブも描かない（Undo の履歴にも入らない）
    open_panel(&mut h);
    press(&h, on_model, egui::PointerButton::Primary);
    h.step();
    assert!(
        !h.state().state.view3d.display.settings_open,
        "外の押しで閉じる"
    );
    assert!(
        !h.state().state.is_stroking(),
        "閉じる押しでストロークが始まらない"
    );
    release(&h, on_model, egui::PointerButton::Primary);
    h.run();
    assert!(!h.state().state.doc.can_undo(), "閉じる押しで描かない");
    // 右ボタン: 閉じる押しで回し始めない
    open_panel(&mut h);
    let yaw = h.state().state.view3d.camera.yaw;
    press(&h, on_model, egui::PointerButton::Secondary);
    h.step();
    move_to(&h, on_model + egui::vec2(50.0, 0.0));
    h.step();
    release(
        &h,
        on_model + egui::vec2(50.0, 0.0),
        egui::PointerButton::Secondary,
    );
    h.run();
    assert!(!h.state().state.view3d.display.settings_open);
    assert_eq!(
        h.state().state.view3d.camera.yaw,
        yaw,
        "閉じる押しで回さない"
    );
    // ホイールは、パネルを開いたままでも使える（寄れる）
    open_panel(&mut h);
    let distance = h.state().state.view3d.camera.distance;
    move_to(&h, on_model);
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, 1.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    h.run();
    assert!(
        h.state().state.view3d.camera.distance != distance,
        "パネルを開いたままホイールで寄れる"
    );
    assert!(
        h.state().state.view3d.display.settings_open,
        "ホイールでは閉じない"
    );
    // パネルを閉じたあとの同じ点の押しは描ける（点がモデルの上にあり、閉じていれば描く対照）
    key(&h, egui::Key::Escape, egui::Modifiers::NONE);
    h.run();
    assert!(!h.state().state.view3d.display.settings_open);
    click(&mut h, on_model);
    assert!(
        h.state().state.doc.can_undo(),
        "パネルが閉じていれば、同じ押しは描く"
    );
}

// ───────── メッシュマップだけの表示 ─────────

/// 16 × 16 の 1 チャンネルのメッシュマップ（全テクセルを覆う）。値は f(x, y)（0〜1。行は文書の y と同じ下から）。
fn synthetic_map(
    kind: yolu_core::mesh_maps::MeshMapKind,
    f: impl Fn(u32, u32) -> f32,
) -> yolu_core::mesh_maps::BakedMeshMap {
    synthetic_map_sized(kind, 16, f)
}

/// 一辺 size の 1 チャンネルのメッシュマップ（全テクセルを覆う）。f の引数は 0..size の画素。
fn synthetic_map_sized(
    kind: yolu_core::mesh_maps::MeshMapKind,
    size: u32,
    f: impl Fn(u32, u32) -> f32,
) -> yolu_core::mesh_maps::BakedMeshMap {
    use yolu_core::mesh_maps::{BakedMeshMap, MeshMapProvenance};
    let provenance = MeshMapProvenance {
        kind,
        engine_version: 1,
        mesh_hash: String::new(),
        topology_hash: String::new(),
        uv_channel: 0,
        width: size as i32,
        height: size as i32,
        target_slot: 0,
        target_slots: vec![0],
        padding: 0,
        antialiasing: 1,
        settings_key: String::new(),
        space: String::new(),
        pose: String::new(),
        source: String::new(),
        bounds_min: [0.0; 3],
        bounds_max: [1.0; 3],
    };
    let data = (0..size)
        .flat_map(|y| (0..size).map(move |x| (x, y)))
        .map(|(x, y)| (f(x, y) * 65535.0).round() as u16)
        .collect();
    BakedMeshMap::new(provenance, data, vec![1; (size * size) as usize]).unwrap()
}

#[test]
fn a_baked_mesh_map_is_shown_alone_on_the_model_and_falls_back_when_it_is_gone() {
    use yolu_core::mesh_maps::MeshMapKind;
    let mut h = view(900.0, 640.0, 64);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, Vec3::ZERO, 2.5, 0.0, 0.0);
    // 焼いたマップを今のセットへ入れる: AO は U に沿って 0 → 1、曲率は V（文書の y）に沿って 0 → 1
    {
        let state = &mut h.state_mut().state;
        let index = state.sets.current_index();
        let maps = &mut state.sets.get_mut(index).unwrap().mesh_maps;
        maps.put(vec![
            synthetic_map(MeshMapKind::AmbientOcclusion, |x, _| x as f32 / 15.0),
            synthetic_map(MeshMapKind::Curvature, |_, y| y as f32 / 15.0),
        ]);
    }
    h.run();
    let rect = h.state().view3d_rect().unwrap();
    let view = h
        .state()
        .state
        .view3d
        .camera
        .view(rect.width(), rect.height());
    let at = |x: f32, y: f32| {
        let s = view.to_screen(Vec3::new(x, y, 0.0)).unwrap();
        rect.min + egui::vec2(s.x, s.y)
    };
    op(
        &mut h,
        Op::Shading(yolu_app::view3d::display::Shading::MeshMap(
            MeshMapKind::AmbientOcclusion,
        )),
    );
    let image = h.render().unwrap();
    // 光なしで値そのまま（テクセルの中心の間は双線形）: u = 0.25 → (3.5 / 15)、0.75 → (11.5 / 15)
    for (x, want) in [
        (-0.25f32, 3.5f32 / 15.0),
        (0.0, 7.5 / 15.0),
        (0.25, 11.5 / 15.0),
    ] {
        let g = (want * 255.0).round() as u8;
        assert_close(px(&image, at(x, 0.0)), [g, g, g], 3, &format!("AO x = {x}"));
    }
    // 曲率は上へ明るくなる（V が増える向き = 画面の上）
    op(
        &mut h,
        Op::Shading(yolu_app::view3d::display::Shading::MeshMap(
            MeshMapKind::Curvature,
        )),
    );
    let image = h.render().unwrap();
    let (low, high) = (
        luma(px(&image, at(0.0, -0.25))),
        luma(px(&image, at(0.0, 0.25))),
    );
    assert!(high > low + 100.0, "曲率は上が明るい: 下 {low}、上 {high}");
    // 光・トーンマッピングは当たらない
    op(&mut h, Op::Exposure(3.0));
    assert_eq!(luma(px(&h.render().unwrap(), at(0.0, 0.25))), high);
    // 今のセットからマップが無くなると、マテリアルの表示へ戻る
    {
        let state = &mut h.state_mut().state;
        let index = state.sets.current_index();
        state.sets.get_mut(index).unwrap().mesh_maps = Default::default();
    }
    h.run();
    assert_eq!(
        h.state().state.view3d.display.shading,
        yolu_app::view3d::display::Shading::Material
    );
}

// ───────── 見た目（スナップショット） ─────────

/// 金属 5 段（左から 0・¼・½・¾・1）× 粗さ 3 段（上から 0.15・0.5・0.85）の球。UV は 1 つの文書のテクセル 1 つずつ。
fn material_balls(h: &mut Harness<'_, YoluApp>) {
    let metallic = [0u8, 64, 128, 191, 255];
    let roughness = [38u8, 128, 217];
    let mut meshes = Vec::new();
    let layer = first_layer(h);
    for (j, r) in roughness.iter().enumerate() {
        for (i, m) in metallic.iter().enumerate() {
            let center = Vec3::new((i as f32 - 2.0) * 1.1, (1.0 - j as f32) * 1.1, 0.0);
            meshes.push(ball(
                center,
                0.5,
                Vec2::new((i as f32 + 0.5) / 32.0, (j as f32 + 0.5) / 32.0),
            ));
            let doc = &mut h.state_mut().state.doc;
            let (x, y) = (i as u32, j as u32);
            doc.set_channel_pixel(layer, Channel::Color, x, y, Rgba8::new(230, 150, 70, 255))
                .unwrap();
            doc.set_channel_pixel(layer, Channel::Metallic, x, y, Rgba8::new(*m, *m, *m, 255))
                .unwrap();
            doc.set_channel_pixel(layer, Channel::Roughness, x, y, Rgba8::new(*r, *r, *r, 255))
                .unwrap();
        }
    }
    set_model(h, meshes);
    // 光の既定の向きはモデルの前（+Z）の斜め上なので、カメラも +Z の側から見る（光とカメラが同じ側。球は −Z の側へ向く面を見せない）
    look_at(h, Vec3::ZERO, 9.8, 180.0, 0.0);
}

#[test]
fn snapshot_material_balls_metal_and_roughness_steps() {
    let mut h = view(1300.0, 720.0, 32);
    material_balls(&mut h);
    op(&mut h, Op::Env(EnvKind::Studio));
    op(&mut h, Op::Tone(Curve::Neutral));
    // 映り込みの並びを見るので、主な光は弱め（Unity のライトの強さ 0.769）にして、滑らかな金属の光の点がトーンマッピングの上で
    // 飽和しないようにする（強さ 1 では、滑らかと中くらいのピークがどちらも飽和の近くで並びが入れ替わる）
    op(&mut h, Op::LightIntensity(0.769));
    let image = h.render().unwrap();
    // 並びの検算: 金属が増えるほど拡散の色（橙）は沈み、粗さが増えるほど映り込みのピークは落ちる
    let area = h.state().view3d_rect().unwrap();
    let view = h
        .state()
        .state
        .view3d
        .camera
        .view(area.width(), area.height());
    let at = |i: i32, j: i32| {
        let c = Vec3::new((i as f32 - 2.0) * 1.1, (1.0 - j as f32) * 1.1, 0.0);
        let s = view.to_screen(c).unwrap();
        area.min + egui::vec2(s.x, s.y)
    };
    let region_peak = |i: i32, j: i32| {
        let c = at(i, j);
        let half = 18.0;
        let region = egui::Rect::from_center_size(c, egui::vec2(half * 2.0, half * 2.0));
        highlight(&image, region).1
    };
    let (smooth, mid, rough) = (region_peak(4, 0), region_peak(4, 1), region_peak(4, 2));
    assert!(
        smooth > mid && mid > rough,
        "金属 1 の映り込みのピーク: {smooth} {mid} {rough}"
    );
    h.snapshot("view3d_material_balls");
}

#[test]
fn snapshot_normal_map_from_a_height_bump() {
    let mut h = view(1100.0, 760.0, 128);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, Vec3::ZERO, 2.2, 0.0, 0.0);
    op(&mut h, Op::Env(EnvKind::None));
    op(&mut h, Op::LightYaw(-130.0));
    op(&mut h, Op::LightPitch(40.0));
    fill(
        &mut h,
        "面",
        &[
            (Channel::Color, [190, 200, 215, 255]),
            (Channel::Metallic, gray(0)),
            (Channel::Roughness, gray(110)),
        ],
    );
    h.state_mut()
        .state
        .doc
        .set_normal_settings(
            NormalSettings::new(true, 30.0, HeightEdgeMode::Clamp, NormalYDirection::OpenGL)
                .unwrap(),
            false,
        )
        .unwrap();
    // 丸い盛り上がりと、斜めの溝（文書は 1 タイル 128²）
    let layer = first_layer(&h);
    let mut tile = vec![0u8; 128 * 128 * 4];
    for y in 0..128usize {
        for x in 0..128usize {
            let (dx, dy) = (x as f32 - 64.0, y as f32 - 64.0);
            let bump = (-(dx * dx + dy * dy) / (2.0 * 22.0 * 22.0)).exp();
            let groove = ((x as f32 + y as f32) * 0.35).sin() * 0.5 + 0.5;
            let v = (bump * 200.0 + groove * 30.0).min(255.0) as u8;
            let i = (y * 128 + x) * 4;
            tile[i..i + 4].copy_from_slice(&[v, v, v, 255]);
        }
    }
    h.state_mut()
        .state
        .doc
        .import_tile(
            layer,
            Channel::Height,
            yolu_core::TileCoord::new(0, 0),
            &tile,
        )
        .unwrap();
    h.run();
    h.snapshot("view3d_normal_map");
    // 光の来る向きを変えると、盛り上がりの明るい側が反対になる（法線マップが効いている）
    let rect = h.state().view3d_rect().unwrap();
    let view = h
        .state()
        .state
        .view3d
        .camera
        .view(rect.width(), rect.height());
    let spot = |x: f32| {
        let s = view.to_screen(Vec3::new(x, 0.0, 0.0)).unwrap();
        rect.min + egui::vec2(s.x, s.y)
    };
    let image = h.render().unwrap();
    let (left, right) = (luma(px(&image, spot(-0.12))), luma(px(&image, spot(0.12))));
    assert!(
        left > right + 15.0,
        "左上から光: 盛り上がりの左が明るい {left} {right}"
    );
}

#[test]
fn snapshot_ball_shadow_on_a_floor() {
    let mut h = ball_over_floor();
    op(&mut h, Op::Shadows(true));
    // ぼかしの粒（画素ごとに回す標本）が GPU ごとにずれて Windows の許しを超えないよう、縁を細めにして撮る
    op(&mut h, Op::ShadowSoftness(0.1));
    h.snapshot("view3d_ball_shadow");
}

#[test]
fn snapshot_settings_panel_and_shading_menu() {
    let mut h = view(1100.0, 760.0, 64);
    h.state_mut().state.view3d.load_demo();
    h.run();
    use egui_kittest::kittest::Queryable;
    h.get_by_label("光・環境・トーンマッピング").click();
    h.run();
    h.snapshot("view3d_settings_panel");
    h.state_mut().apply(Action::View3d(Op::CloseSettings));
    h.run();
    h.get_by_label("3D の表示: マテリアル (PBR)").click();
    h.run();
    h.snapshot("view3d_shading_menu");
}

// ───────── 計測（cargo test -p yolu-app --test gui_view3d -- --ignored --nocapture view3d_look::measure） ─────────

fn micros(v: &[u64]) -> (f64, u64) {
    let sum: u64 = v.iter().sum();
    (
        sum as f64 / v.len().max(1) as f64 / 1000.0,
        v.iter().copied().max().unwrap_or(0) / 1000,
    )
}

/// 7 万三角形・4096²・6 チャンネルで、描いている最中・カメラを回すときの 1 フレームの時間を、影なし・影ありで同じ仕事・同じ量を交互に測る。
#[test]
#[ignore = "計測"]
fn measure_painting_frames_with_seventy_thousand_triangles_and_six_channels() {
    use std::time::Instant;
    let mut h = view(1400.0, 900.0, 4096);
    let adapter = h.state().view3d_adapter().unwrap_or_default();
    // 7 万三角形の球（面ごとの UV アイランド）。接線は先に作る
    let started = Instant::now();
    set_model(&mut h, vec![cube_sphere(77, 0.5)]);
    println!("GPU: {adapter}");
    println!(
        "モデル: 71148 三角形、接線を含めた読み込み {:.0} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
    look_at(&mut h, Vec3::ZERO, 1.6, 20.0, 10.0);
    let layer = first_layer(&h);
    // 全チャンネルを使う: 塗りつぶし（Metallic・Roughness・Emission）、Height → Normal、Color の面
    fill(
        &mut h,
        "値",
        &[
            (Channel::Metallic, gray(60)),
            (Channel::Roughness, gray(120)),
            (Channel::Emission, [20, 10, 5, 255]),
        ],
    );
    h.state_mut()
        .state
        .doc
        .set_normal_settings(
            NormalSettings::new(true, 8.0, HeightEdgeMode::Clamp, NormalYDirection::OpenGL)
                .unwrap(),
            false,
        )
        .unwrap();
    let doc = &mut h.state_mut().state.doc;
    doc.set_channel_pixel(layer, Channel::Height, 5, 5, Rgba8::new(200, 200, 200, 255))
        .unwrap();
    doc.set_channel_pixel(layer, Channel::Color, 5, 5, Rgba8::new(200, 60, 40, 255))
        .unwrap();
    let started = Instant::now();
    h.step();
    h.state().view3d_wait_gpu();
    let s = h.state().view3d_stats().unwrap();
    println!(
        "初めての構築（6 チャンネル 4096²）: {:.0} ms（同期 {:.0} ms）、GPU のテクスチャ {:.0} MiB、上げたタイル {:?}",
        started.elapsed().as_secs_f64() * 1000.0,
        s.last_sync_us as f64 / 1000.0,
        s.paint_bytes as f64 / 1048576.0,
        s.last_slot_tiles
    );
    h.run();
    // 影なし・影ありを、同じ仕事・同じ量で、交互に何回か測る（回ごとの平均とそのばらつきを出す）。
    // 仕事は 3 つ: 全チャンネルの 2 タイルずつ（描いている最中の重い場合）、Color・Roughness の 1 タイルずつ、カメラを回すだけ。
    // どれも 1 フレームごとに step して GPU の完了まで待つ時間（全体）と、prepare の CPU の時間を取る。
    const ROUNDS: usize = 3;
    const FRAMES: u32 = 20;
    let mut counter = 0u32;
    let mut paint_frames = |h: &mut Harness<'_, YoluApp>, all_channels: bool| {
        let (mut wall, mut cpu, mut tiles) = (Vec::new(), Vec::new(), Vec::new());
        for _ in 0..FRAMES {
            counter += 1;
            let (x, y) = (200 + counter * 37 % 3500, 300 + counter * 53 % 3400);
            let doc = &mut h.state_mut().state.doc;
            let tiles_per_frame = if all_channels { 2 } else { 1 };
            for k in 0..tiles_per_frame {
                let (px, py) = (x + k * 130, y);
                let mut values = vec![
                    (Channel::Color, Rgba8::new(255, 128, 0, 255)),
                    (Channel::Roughness, Rgba8::new(10, 10, 10, 255)),
                ];
                if all_channels {
                    values.extend([
                        (Channel::Metallic, Rgba8::new(255, 255, 255, 255)),
                        (Channel::Emission, Rgba8::new(255, 255, 255, 255)),
                        (Channel::Height, Rgba8::new(255, 255, 255, 255)),
                    ]);
                }
                for (channel, v) in values {
                    doc.set_channel_pixel(layer, channel, px, py, v).unwrap();
                }
            }
            let started = Instant::now();
            h.step();
            h.state().view3d_wait_gpu();
            wall.push(started.elapsed().as_micros() as u64);
            let s = h.state().view3d_stats().unwrap();
            cpu.push(s.last_prepare_us);
            tiles.push(s.last_tiles as u64);
        }
        (
            micros(&wall).0,
            micros(&cpu).0,
            tiles.iter().sum::<u64>() as f64 / tiles.len() as f64,
        )
    };
    let orbit_frames = |h: &mut Harness<'_, YoluApp>| {
        let (mut wall, mut cpu) = (Vec::new(), Vec::new());
        for frame in 0..FRAMES {
            let yaw = &mut h.state_mut().state.view3d.camera.yaw;
            *yaw += 3.0 + frame as f32 * 0.01;
            let started = Instant::now();
            h.step();
            h.state().view3d_wait_gpu();
            wall.push(started.elapsed().as_micros() as u64);
            cpu.push(h.state().view3d_stats().unwrap().last_prepare_us);
        }
        (micros(&wall).0, micros(&cpu).0)
    };
    let spread = |v: &[f64]| {
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        let min = v.iter().copied().fold(f64::MAX, f64::min);
        let max = v.iter().copied().fold(f64::MIN, f64::max);
        format!("{mean:.1}（{min:.1}〜{max:.1}）")
    };
    // 影のマップの最初の 1 回（作る + 描く + そのフレーム）
    h.state_mut().apply(Action::View3d(Op::Shadows(true)));
    let started = Instant::now();
    h.step();
    h.state().view3d_wait_gpu();
    println!(
        "影のマップの最初の 1 回（作る + 描く + そのフレーム）: {:.1} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
    let shadow_renders = h.state().view3d_stats().unwrap().shadow_renders;
    // [影なし, 影あり] × [全チャンネル 2 タイル, Color・Roughness 1 タイル, カメラを回す] の、回ごとの（全体, CPU）
    let mut rows: [[Vec<(f64, f64)>; 3]; 2] = Default::default();
    let mut tiles_per_frame = [[0.0f64; 2]; 2];
    for _ in 0..ROUNDS {
        for (i, shadows) in [false, true].into_iter().enumerate() {
            h.state_mut().apply(Action::View3d(Op::Shadows(shadows)));
            h.step();
            h.state().view3d_wait_gpu();
            let (w, c, t) = paint_frames(&mut h, true);
            rows[i][0].push((w, c));
            tiles_per_frame[i][0] = t;
            let (w, c, t) = paint_frames(&mut h, false);
            rows[i][1].push((w, c));
            tiles_per_frame[i][1] = t;
            rows[i][2].push(orbit_frames(&mut h));
        }
    }
    for (name, k) in [
        ("描いている最中（全チャンネルの 2 タイルずつ）", 0),
        ("描いている最中（Color・Roughness の 1 タイルずつ）", 1),
        ("カメラを回す（絵は同じ）", 2),
    ] {
        for (i, label) in ["影なし", "影あり"].into_iter().enumerate() {
            let wall: Vec<f64> = rows[i][k].iter().map(|r| r.0).collect();
            let cpu: Vec<f64> = rows[i][k].iter().map(|r| r.1).collect();
            println!(
                "計測 {name} {label}: 全体（GPU の完了まで）平均 {} ms、prepare の CPU 平均 {} ms{}",
                spread(&wall),
                spread(&cpu),
                if k < 2 {
                    format!("、上げたタイル 平均 {:.1}", tiles_per_frame[i][k])
                } else {
                    String::new()
                }
            );
        }
    }
    // 変わらないフレーム: 描かない
    let renders = h.state().view3d_stats().unwrap().renders;
    h.step();
    h.step();
    assert_eq!(h.state().view3d_stats().unwrap().renders, renders);
    assert_eq!(
        h.state().view3d_stats().unwrap().shadow_renders,
        shadow_renders,
        "カメラを回しても塗っても影のマップは描き直さない"
    );
}
