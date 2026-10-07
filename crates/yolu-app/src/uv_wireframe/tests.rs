use super::*;
use crate::{
    lang::Lang,
    view3d::pose::{set_pose, PoseAction},
};
use egui::{pos2, vec2, Rect};
use yolu_core::{
    geometry::{BvhUpdate, DEFAULT_WELD_TOLERANCE},
    glam::{Quat, Vec3},
};

fn triangle(a: Vec2, b: Vec2, c: Vec2, material: i32) -> SurfaceTriangle {
    let mut t = SurfaceTriangle::new(Vec3::ZERO, Vec3::X, Vec3::Y, a, b, c);
    t.material = material;
    t
}

fn geometry(triangles: Vec<SurfaceTriangle>) -> Arc<SurfaceGeometry> {
    Arc::new(SurfaceGeometry::new(triangles, 1, DEFAULT_WELD_TOLERANCE).unwrap())
}

/// 位置だけを動かした次の世代（ポーズを当てたときと同じ `reposition`）。
fn moved(g: &SurfaceGeometry, revision: u32) -> Arc<SurfaceGeometry> {
    let triangles = g
        .triangles()
        .iter()
        .map(|t| SurfaceTriangle {
            a: t.a + Vec3::Z * revision as f32,
            b: t.b + Vec3::Z * revision as f32,
            c: t.c + Vec3::Z * revision as f32,
            ..*t
        })
        .collect();
    Arc::new(g.reposition(triangles, revision, BvhUpdate::Refit).unwrap())
}

fn demo_app() -> AppState {
    let mut app = AppState::new(64, 32);
    app.apply(Action::LoadDemoModel);
    app.sync_view3d();
    assert!(app.region_model().is_some());
    app
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    crate::YoluApp::setup(&ctx);
    ctx
}

/// 1 フレーム分の `show` を回し、重ねた線の本数（設定の色の線）を返す。
fn frame(ctx: &egui::Context, app: &mut AppState) -> usize {
    let rect = Rect::from_min_size(pos2(20.0, 30.0), vec2(400.0, 300.0));
    let c = app.prefs.settings.uv_wireframe_color;
    let color = Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        let view = app.view.view(rect, app.doc.width(), app.doc.height());
        show(&mut child, app, &view);
    });
    output.textures_delta.clear();
    output
        .shapes
        .iter()
        .filter(
            |s| matches!(s.shape, egui::Shape::LineSegment { stroke, .. } if stroke.color == color),
        )
        .count()
}

#[test]
fn unique_uv_edges_keep_seams_and_filter_materials() {
    let a = triangle(Vec2::ZERO, Vec2::X, Vec2::Y, 0);
    let b = triangle(Vec2::Y, Vec2::X, Vec2::ONE, 0);
    let other = triangle(Vec2::ZERO, Vec2::X * 2.0, Vec2::Y, 1);
    assert_eq!(edges(&[a, b, other], 0, 5).unwrap().len(), 5);
    assert_eq!(edges(&[a, b, other], 1, 5).unwrap().len(), 3);
    let seam = triangle(Vec2::ZERO, Vec2::X * 0.5, Vec2::Y, 0);
    assert_eq!(edges(&[a, seam], 0, 5).unwrap().len(), 5);
    assert!(edges(&[a, b], 0, 4).is_none());
    assert_eq!(edges(&[a], -1, 0), Some(vec![]));
}

#[test]
fn degenerate_nonfinite_and_signed_zero_edges_are_safe() {
    let a = triangle(Vec2::ZERO, Vec2::X, Vec2::X, 0);
    let b = triangle(Vec2::new(-0.0, 0.0), Vec2::X, Vec2::NAN, 0);
    assert_eq!(edges(&[a, b], 0, 1).unwrap(), vec![[Vec2::ZERO, Vec2::X]]);
}

#[test]
fn cache_reuses_edges_and_screen_until_geometry_material_or_view_changes() {
    let t = triangle(Vec2::ZERO, Vec2::X, Vec2::Y, 0);
    let g = geometry(vec![t]);
    let mut cache = Wireframe::default();
    cache.sync(&g, 0);
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
    let mut app = AppState::new(64, 32);
    for (angle, flip, zoom) in [(0.0, false, 1.0), (90.0, true, 2.0), (15.0, false, 0.5)] {
        app.view.angle = angle;
        app.view.flip = flip;
        app.view.zoom = zoom;
        app.view.pan = egui::vec2(13.0, -7.0);
        let view = app.view.view(rect, 64, 32);
        cache.project(&view, 64, 32);
        let saved = cache.screen.as_ptr();
        cache.sync(&g, 0);
        cache.project(&view, 64, 32);
        assert_eq!(saved, cache.screen.as_ptr());
        for (edge, screen) in cache.edges.iter().zip(&cache.screen) {
            for i in 0..2 {
                assert_eq!(
                    screen[i],
                    view.to_screen(edge[i].x as f64 * 64.0, edge[i].y as f64 * 32.0)
                );
            }
        }
    }
    assert_eq!(cache.builds, 1);
    cache.sync(&g, 2);
    assert!(cache.edges.is_empty());
    cache.sync(&g, 0);
    assert_eq!(cache.edges.len(), 3);
    assert_eq!(cache.builds, 3);
    cache.idle(None);
    assert!(cache.edges.is_empty() && cache.screen.is_empty() && cache.geometry.is_none());
}

#[test]
fn a_repositioned_geometry_keeps_the_edges_but_changed_uv_or_material_rebuilds_them() {
    let a = triangle(Vec2::ZERO, Vec2::X, Vec2::Y, 0);
    let b = triangle(Vec2::Y, Vec2::X, Vec2::ONE, 1);
    let first = geometry(vec![a, b]);
    let mut cache = Wireframe::default();
    cache.sync(&first, 0);
    assert_eq!((cache.builds, cache.edges.len()), (1, 3));
    // 位置だけが替わった世代が続いても、辺は作り直さず、持つ幾何だけ新しい世代へ移る。
    let mut latest = first.clone();
    for revision in 2..8 {
        latest = moved(&latest, revision);
        assert!(!Arc::ptr_eq(&latest, cache.geometry.as_ref().unwrap()));
        cache.sync(&latest, 0);
        assert!(Arc::ptr_eq(&latest, cache.geometry.as_ref().unwrap()));
    }
    assert_eq!(cache.builds, 1);
    // 別のマテリアルの UV が替わっても、いまのマテリアルの辺は変わらない。
    let other_uv = triangle(Vec2::Y, Vec2::X * 3.0, Vec2::ONE, 1);
    cache.sync(&geometry(vec![a, other_uv]), 0);
    assert_eq!(cache.builds, 1);
    // いまのマテリアルの UV が替われば作り直す。
    let shifted = triangle(Vec2::ZERO, Vec2::X * 2.0, Vec2::Y, 0);
    cache.sync(&geometry(vec![shifted, b]), 0);
    assert_eq!(cache.builds, 2);
    assert!(cache.edges.iter().any(|e| e.contains(&(Vec2::X * 2.0))));
    // 三角形の数・マテリアルの割り当てが替われば作り直す。
    cache.sync(&geometry(vec![shifted]), 0);
    assert_eq!(cache.builds, 3);
    let reassigned = triangle(Vec2::ZERO, Vec2::X * 2.0, Vec2::Y, 1);
    cache.sync(&geometry(vec![reassigned]), 0);
    assert_eq!(cache.builds, 4);
}

#[test]
fn repeated_poses_do_not_rebuild_the_edges_and_a_hidden_overlay_builds_nothing() {
    let ctx = context();
    let mut app = AppState::new(64, 64);
    app.apply(Action::Pose(PoseAction::LoadFigure));
    app.sync_view3d();
    assert!(app.region_model().is_some(), "{}", app.message);
    app.prefs.settings.uv_wireframe = false;
    for _ in 0..3 {
        assert_eq!(frame(&ctx, &mut app), 0);
    }
    assert_eq!(
        app.uv_wireframe.builds, 0,
        "表示が切れている間は辺を作らない"
    );
    app.apply(Action::ToggleUvWireframe);
    let lines = frame(&ctx, &mut app);
    assert!(lines > 0);
    assert_eq!(app.uv_wireframe.builds, 1);
    // ポーズのたびに別の幾何の Arc が入る（UV は同じ）。毎回同じ線を描き、辺は作り直さない。
    let bone = {
        let s = app.view3d.pose.session.as_ref().unwrap();
        s.rig
            .bones()
            .iter()
            .position(|b| b.name == "右上腕")
            .unwrap()
    };
    for step in 1..=6 {
        let before = app.view3d.model.as_ref().unwrap().geometry.clone();
        let mut pose = app.view3d.pose.session.as_ref().unwrap().pose().clone();
        pose.locals[bone].rotation = Quat::from_rotation_z(0.1 * step as f32);
        set_pose(&mut app.view3d, pose).unwrap();
        app.sync_view3d();
        assert!(!Arc::ptr_eq(
            &before,
            &app.view3d.model.as_ref().unwrap().geometry
        ));
        assert_eq!(frame(&ctx, &mut app), lines, "ポーズ {step}");
    }
    assert_eq!(app.uv_wireframe.builds, 1, "ポーズでは辺を作り直さない");
    // 切っている間にポーズが替わったら、古い幾何は手放す（入れ直すとき 1 回だけ作る）。
    app.apply(Action::ToggleUvWireframe);
    let mut pose = app.view3d.pose.session.as_ref().unwrap().pose().clone();
    pose.locals[bone].rotation = Quat::from_rotation_z(-0.5);
    set_pose(&mut app.view3d, pose).unwrap();
    app.sync_view3d();
    assert_eq!(frame(&ctx, &mut app), 0);
    assert!(app.uv_wireframe.geometry.is_none() && app.uv_wireframe.edges.is_empty());
    app.apply(Action::ToggleUvWireframe);
    assert_eq!(frame(&ctx, &mut app), lines);
    assert_eq!(app.uv_wireframe.builds, 2);
    assert!(!app.can_undo());
}

#[test]
fn edges_over_the_limit_draw_nothing_and_the_tooltip_says_why() {
    let ctx = context();
    for lang in Lang::ALL {
        let mut app = demo_app();
        app.lang = lang;
        let lines = frame(&ctx, &mut app);
        assert!(lines > 2);
        assert_eq!(tip(&app), lang.pick("UV ワイヤーフレーム", "UV Wireframe"));
        // 上限を 1 本下げる: 部分的な図も出さない
        app.uv_wireframe = Wireframe {
            limit: lines - 1,
            ..Wireframe::default()
        };
        assert_eq!(frame(&ctx, &mut app), 0);
        assert!(app.uv_wireframe.truncated && app.uv_wireframe.edges.is_empty());
        assert_eq!(
            tip(&app),
            lang.pick(
                "UV の辺が表示の上限を超えています",
                "UV edges exceed the display limit"
            )
        );
        // 上限ちょうどなら描く
        app.uv_wireframe = Wireframe {
            limit: lines,
            ..Wireframe::default()
        };
        assert_eq!(frame(&ctx, &mut app), lines);
        assert!(!app.uv_wireframe.truncated);
        // 切っているときは理由を出さない（描いていないので）
        app.uv_wireframe = Wireframe {
            limit: lines - 1,
            ..Wireframe::default()
        };
        frame(&ctx, &mut app);
        app.apply(Action::ToggleUvWireframe);
        assert_eq!(tip(&app), lang.pick("UV ワイヤーフレーム", "UV Wireframe"));
        assert!(!app.can_undo());
    }
}

#[test]
fn the_icon_and_the_menu_item_are_disabled_without_a_model() {
    use crate::ui::menu::Entry;
    let enabled = |app: &AppState| match menu_entry(app) {
        Entry::Item { enabled, .. } => enabled,
        _ => unreachable!(),
    };
    let ctx = context();
    for lang in Lang::ALL {
        let mut app = AppState::new(64, 32);
        app.lang = lang;
        assert!(!enabled(&app));
        assert_eq!(frame(&ctx, &mut app), 0);
        assert_eq!(tip(&app), lang.pick("モデルがありません", "No model"));
        assert_eq!(app.uv_wireframe.builds, 0);
        app.apply(Action::LoadDemoModel);
        app.sync_view3d();
        assert!(enabled(&app));
        assert!(frame(&ctx, &mut app) > 0);
        assert_eq!(tip(&app), lang.pick("UV ワイヤーフレーム", "UV Wireframe"));
    }
}

/// 2 つのマテリアル（Skin・Hair）の 1 枚板の Live Link のモデル。
fn link_model() -> yolu_protocol::Model {
    use yolu_protocol::{MaterialInfo, MaterialKey, MeshData, Model, Submesh};
    Model {
        generation: 1,
        name: "Sample".into(),
        materials: ["Skin", "Hair"]
            .iter()
            .map(|n| MaterialInfo {
                key: MaterialKey::Material {
                    name: (*n).into(),
                    asset: None,
                },
                shader: "Standard".into(),
                textures: vec![],
                routes: vec![],
            })
            .collect(),
        meshes: vec![MeshData {
            key: "0".into(),
            name: "Body".into(),
            skinned: false,
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            submeshes: (0..2)
                .map(|m| Submesh {
                    material: m,
                    indices: vec![0, 1, 2],
                })
                .collect(),
        }],
    }
}

#[test]
fn the_overlay_uses_the_surface_three_d_paints_even_when_the_link_is_stale() {
    let ctx = context();
    for lang in Lang::ALL {
        let mut app = AppState::new(64, 64);
        app.lang = lang;
        app.receive_link_model(&link_model()).1.unwrap();
        app.sync_view3d();
        let bound = app.sets.current().bound;
        assert!(bound.is_some());
        assert_eq!(app.view3d.material, bound.unwrap() as i32);
        assert!(frame(&ctx, &mut app) > 0, "つながっているモデルの面");
        // 記録の世代が 3D の世代と合わなくなる（古い世代）。3D は描かない（-1）ので、UV も重ねない。
        app.model.as_mut().unwrap().generation += 1;
        app.sync_view3d();
        assert_eq!(app.view3d.material, -1);
        assert!(app.region_model().is_none());
        assert_eq!(
            app.sets.current().bound,
            bound,
            "セットの結び付けは残っている（以前はこれを読んでいた）"
        );
        assert_eq!(frame(&ctx, &mut app), 0);
        assert_eq!(tip(&app), app.region_missing_reason());
        assert!(app.uv_wireframe.edges.is_empty());
        // 世代が戻れば、また同じ面を重ねる。
        app.model.as_mut().unwrap().generation -= 1;
        app.sync_view3d();
        assert!(frame(&ctx, &mut app) > 0);
        assert!(!app.can_undo());
    }
}

#[test]
fn settings_restore_color_alpha_and_visibility_without_document_changes() {
    let mut app = AppState::new(64, 64);
    let epoch = app.doc_epoch;
    let undo = app.can_undo();
    app.apply(Action::ToggleUvWireframe);
    app.prefs.settings.uv_wireframe_color = [12, 34, 56, 78];
    let mut encoded = String::new();
    save_settings(&mut encoded, &app.prefs.settings);
    assert_eq!(
        encoded,
        "uv_wireframe=off\nuv_wireframe_color=12,34,56,78\n"
    );
    assert_eq!(parse_color("12,34,56,78"), Some([12, 34, 56, 78]));
    assert_eq!(parse_color("256,0,0,255"), None);
    assert_eq!(parse_color("0,0,0"), None);
    assert_eq!(app.doc_epoch, epoch);
    assert_eq!(app.can_undo(), undo);
    assert!(!app.prefs.settings.uv_wireframe);
}

#[test]
fn personal_settings_roundtrip_and_reject_invalid_color_with_a_named_message() {
    let path = std::env::temp_dir().join(format!("yolu-uv-prefs-{}.conf", std::process::id()));
    let settings = crate::settings::Settings {
        uv_wireframe: false,
        uv_wireframe_color: [20, 40, 60, 80],
        ..Default::default()
    };
    crate::settings::save(&path, &settings).unwrap();
    let (restored, problems) = crate::settings::load(&path);
    assert!(problems.is_empty());
    assert_eq!(restored, settings);
    std::fs::write(&path, "uv_wireframe_color=300,0,0,255\nlanguage=en\n").unwrap();
    let (restored, problems) = crate::settings::load(&path);
    assert_eq!(restored.uv_wireframe_color, DEFAULT_COLOR);
    assert_eq!(restored.lang, Lang::En);
    assert_eq!(problems.len(), 1);
    assert_eq!(
        problems[0].text(Lang::En),
        "Invalid UV wireframe color setting (300,0,0,255); using the default."
    );
    assert_eq!(
        problems[0].text(Lang::Ja),
        "UV ワイヤーフレームの色の設定が正しくありません（300,0,0,255）。既定に戻します。"
    );
    std::fs::remove_file(path).unwrap();
}
