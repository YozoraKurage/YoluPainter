//! 編集のモードの物（点の印の選び・G/R/S・戻す・隠す・消す）と、ポーズのモードのボーンの G/R/S の試験（画面を作らない）。

use egui::{pos2, vec2, Key, Modifiers, Pos2, Rect};
use yolu_core::fill_image::ProjectionMode;
use yolu_core::glam::{Quat, Vec3};
use yolu_core::{Channel, FilterTarget, LayerId};

use super::transform::{self, Constraint, Input, Step};
use super::*;
use crate::fillfx::FillOp;
use crate::state::{StrokeSource, Tool};
use crate::view3d::pose::PoseAction;

const RECT: Rect = Rect {
    min: pos2(100.0, 100.0),
    max: pos2(700.0, 500.0),
};

/// 試しの立方体（中心が原点・半辺 0.5）を、手前（−Z の側）から正面に見た 3D ビュー（表示域 `RECT`。画面の右が +X）。
fn cube() -> AppState {
    let mut app = AppState::new(64, 64);
    app.apply(Action::LoadDemoModel);
    app.view3d.material = 0;
    app.view3d.camera.yaw = 0.0;
    app.view3d.camera.pitch = 0.0;
    app.view3d.view_rect = Some(RECT);
    app.view3d.visible = true;
    app
}

fn fill_layer(app: &mut AppState) -> LayerId {
    app.apply(Action::M2(crate::m2::Edit::NewFill));
    app.selected_layer.expect("塗りつぶしレイヤー")
}

/// 投影の箱（平面の投影。モデルの外形に合わせる）と、グラデーションデカール（色）・点のグラデーション（ラフネス）・フィルターの形を持つ
/// 塗りつぶしレイヤー。
fn every_kind(app: &mut AppState) -> LayerId {
    let layer = fill_layer(app);
    app.apply(Action::Fill(FillOp::ProjectionMode {
        layer,
        mode: ProjectionMode::Planar,
    }));
    app.apply(Action::Fill(FillOp::AddGradient {
        layer,
        channel: Channel::Color,
    }));
    app.apply(Action::Fill(FillOp::AddPoints {
        layer,
        channel: Channel::Roughness,
    }));
    app.apply(Action::Fx(crate::fx::FxOp::AddGenerator {
        target: FilterTarget::Content,
        kind: yolu_core::generator::Kind::ShapeGradient,
    }));
    layer
}

fn screen_of(app: &AppState, world: Vec3) -> Pos2 {
    let v = app.view3d.camera.view(RECT.width(), RECT.height());
    let s = v.to_screen(world).expect("カメラの前");
    pos2(RECT.left() + s.x, RECT.top() + s.y)
}

fn key(k: Key) -> (Key, Modifiers) {
    (k, Modifiers::NONE)
}

fn step_keys(app: &mut AppState, keys: &[(Key, Modifiers)]) -> Step {
    transform::step(
        app,
        &Input {
            keys: keys.to_vec(),
            ..Default::default()
        },
    )
}

fn step_pointer(app: &mut AppState, at: Pos2, ctrl: bool) -> Step {
    transform::step(
        app,
        &Input {
            pointer: Some(at),
            ctrl,
            ..Default::default()
        },
    )
}

/// 値を打つ（`-`・`.`・数字）。
fn type_value(app: &mut AppState, text: &str) {
    let keys: Vec<(Key, Modifiers)> = text
        .chars()
        .map(|c| {
            key(match c {
                '-' => Key::Minus,
                '.' => Key::Period,
                d => Key::from_name(&d.to_string()).expect("数字"),
            })
        })
        .collect();
    assert_eq!(step_keys(app, &keys), Step::Open, "{}", app.message);
}

/// 選んだ物の印の所から始める（ポーズのモードは表示域の真ん中から）。
fn begin(app: &mut AppState, kind: Kind) -> Pos2 {
    let at = app
        .objects
        .selected
        .and_then(|o| marker_of(app, o))
        .map_or(RECT.center(), |w| screen_of(app, w));
    transform::begin(app, kind, at, 0).unwrap_or_else(|e| panic!("{kind:?}: {e}"));
    at
}

fn shape(app: &AppState, o: Object) -> sg::Shape {
    let Object::Shape(t) = o else {
        panic!("形");
    };
    shape_of(app, t).expect("形")
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-4
}

fn transform_state(app: &AppState) -> &transform::Transform {
    app.objects.transform.as_ref().expect("G/R/S の途中")
}

#[test]
fn every_kind_of_object_on_visible_layers_gets_a_marker_and_hidden_layers_do_not() {
    let mut app = cube();
    let layer = every_kind(&mut app);
    let kinds = |app: &AppState| -> Vec<&'static str> {
        markers(app)
            .iter()
            .map(|m| match m.object {
                Object::Shape(Target::Projection(_)) => "投影",
                Object::Shape(Target::Gradient(..)) => "デカール",
                Object::Shape(Target::Filter(..)) => "フィルター",
                Object::Point { .. } => "点",
                Object::Path { .. } => "パス",
                Object::Ruler { .. } | Object::Shape(Target::Ruler(..)) => "定規",
            })
            .collect()
    };
    assert_eq!(kinds(&app), ["投影", "デカール", "点", "点", "フィルター"]);
    // 点の印はその点、形の印は形の中心
    let g = app
        .doc
        .layer(layer)
        .unwrap()
        .fill_points(Channel::Roughness)
        .unwrap()
        .clone();
    let points: Vec<Marker> = markers(&app)
        .into_iter()
        .filter(|m| matches!(m.object, Object::Point { .. }))
        .collect();
    for (m, p) in points.iter().zip(&g.points) {
        let at = Vec3::new(
            p.position[0] as f32,
            p.position[1] as f32,
            p.position[2] as f32,
        );
        assert!((m.world - at).length() < 1e-5);
    }
    let boxed = markers(&app)[0];
    let s = shape(&app, boxed.object);
    assert!((boxed.world - sg::world_center(&s, &gizmo::root())).length() < 1e-6);
    // 隠したレイヤーの物は出さない
    app.apply(Action::ToggleVisible(layer));
    assert!(markers(&app).is_empty());
    // UV の投影は箱が無い
    app.apply(Action::ToggleVisible(layer));
    app.apply(Action::Fill(FillOp::ProjectionMode {
        layer,
        mode: ProjectionMode::Uv,
    }));
    assert_eq!(kinds(&app), ["デカール", "点", "点", "フィルター"]);
    // モデルが無ければ何も出さない
    app.view3d.model = None;
    assert!(markers(&app).is_empty());
}

#[test]
fn pressing_markers_selects_their_layer_cycles_overlaps_in_place_and_empty_space_deselects() {
    let mut app = cube();
    let first = every_kind(&mut app);
    // ほかのレイヤーの点（最初のレイヤーの点と重ならない所へずらす）
    let other = fill_layer(&mut app);
    app.apply(Action::Fill(FillOp::AddPoints {
        layer: other,
        channel: Channel::Color,
    }));
    let mut g = app
        .doc
        .layer(other)
        .unwrap()
        .fill_points(Channel::Color)
        .unwrap()
        .clone();
    for p in &mut g.points {
        p.position[1] += 0.2;
    }
    app.apply(Action::Fill(FillOp::Points {
        layer: other,
        channel: Channel::Color,
        points: Some(Box::new(g)),
        coalesce: false,
    }));
    assert!(app.set_mode(EditorMode::Edit));
    // 投影の箱・グラデーションデカール・フィルターの形は、どれもモデルの外形の中心にある（重なった印）
    let center = marker_of(&app, Object::Shape(Target::Projection(first))).unwrap();
    let at = screen_of(&app, center);
    let overlapped: Vec<Object> = markers(&app)
        .into_iter()
        .filter(|m| screen_of(&app, m.world).distance(at) <= GRAB_POINTS)
        .map(|m| m.object)
        .collect();
    assert!(overlapped.len() >= 2, "{overlapped:?}");
    assert!(press(&mut app, RECT, at, gizmo::Source::Mouse));
    let mut seen = vec![app.objects.selected.unwrap()];
    assert_eq!(app.selected_layer, Some(first), "押した物のレイヤーを選ぶ");
    // 同じ所でもう一度押すたびに、重なった次の物（一巡すると最初へ）
    for _ in 1..overlapped.len() {
        press(&mut app, RECT, at, gizmo::Source::Mouse);
        let s = app.objects.selected.unwrap();
        assert!(!seen.contains(&s), "{s:?} {seen:?}");
        seen.push(s);
    }
    press(&mut app, RECT, at, gizmo::Source::Mouse);
    assert_eq!(app.objects.selected, Some(seen[0]));
    // ほかのレイヤーの点を押すと、そのレイヤーを選ぶ
    let other_point = markers(&app)
        .into_iter()
        .find(|m| m.object.layer() == other)
        .unwrap();
    let there = screen_of(&app, other_point.world);
    press(&mut app, RECT, there, gizmo::Source::Mouse);
    assert_eq!(app.objects.selected, Some(other_point.object));
    assert_eq!(app.selected_layer, Some(other));
    // 何も無い所: 選びを外す（レイヤーは替えない）
    press(
        &mut app,
        RECT,
        pos2(RECT.left() + 5.0, RECT.top() + 5.0),
        gizmo::Source::Mouse,
    );
    assert_eq!(app.objects.selected, None);
    assert_eq!(app.selected_layer, Some(other));
    // ペイントのモードでは受けない
    assert!(app.set_mode(EditorMode::Paint));
    assert!(!press(&mut app, RECT, at, gizmo::Source::Mouse));
}

#[test]
fn grab_moves_a_box_by_the_pointer_on_an_axis_and_by_a_typed_value_and_cancel_restores_without_history(
) {
    let mut app = cube();
    let layer = every_kind(&mut app);
    assert!(app.set_mode(EditorMode::Edit));
    let o = Object::Shape(Target::Projection(layer));
    select(&mut app, Some(o));
    let start = shape(&app, o);
    let undo = app.doc.undo_count();
    // ポインタで右へ（世界の +X。カメラに向いた面の中で動く）
    let from = begin(&mut app, Kind::Grab);
    assert!(crate::fillfx::dragging(&app), "途中は欄がまとめを終えない");
    assert_eq!(
        step_pointer(&mut app, from + vec2(60.0, 0.0), false),
        Step::Open
    );
    let moved = shape(&app, o);
    assert!(moved.center[0] - start.center[0] > 0.05, "{moved:?}");
    assert!(near(moved.center[1], start.center[1]), "{moved:?}");
    assert!(near(moved.center[2], start.center[2]), "{moved:?}");
    assert!(app.doc.is_coalescing(), "途中は続けて変える操作");
    // Y の軸: 右への動きは Y の量にならない
    step_keys(&mut app, &[key(Key::Y)]);
    let s = shape(&app, o);
    assert!(near(s.center[0], start.center[0]) && near(s.center[2], start.center[2]));
    // 打った値（Y に 0.25）はポインタより先
    type_value(&mut app, "0.25");
    assert!(near(shape(&app, o).center[1], start.center[1] + 0.25));
    assert_eq!(transform_state(&app).status(Lang::Ja), "移動 Y 0.25");
    // Esc: 始める前へ戻し、履歴に残さない
    assert_eq!(step_keys(&mut app, &[key(Key::Escape)]), Step::Done);
    assert_eq!(shape(&app, o), start);
    assert_eq!(app.doc.undo_count(), undo);
    assert!(!app.doc.is_coalescing());
    assert!(!transforming(&app));
    assert!(!crate::fillfx::dragging(&app));
}

#[test]
fn snapping_follows_ctrl_and_the_always_snap_toggle_and_confirm_is_one_undo_step() {
    let mut app = cube();
    let layer = every_kind(&mut app);
    assert!(app.set_mode(EditorMode::Edit));
    let o = Object::Shape(Target::Projection(layer));
    select(&mut app, Some(o));
    let start = shape(&app, o);
    let undo = app.doc.undo_count();
    let on_step = |d: f64| d.abs() > 0.05 && near((d / 0.1).round() * 0.1, d);
    // Ctrl を押している間は 0.1 の刻み
    let from = begin(&mut app, Kind::Grab);
    step_keys(&mut app, &[key(Key::X)]);
    step_pointer(&mut app, from + vec2(37.0, 0.0), true);
    let d = shape(&app, o).center[0] - start.center[0];
    assert!(on_step(d), "{d}");
    step_pointer(&mut app, from + vec2(37.0, 0.0), false);
    let free = shape(&app, o).center[0] - start.center[0];
    assert!(!near(free, d), "Ctrl を離すと刻まない: {free} {d}");
    // 左クリックで決める: 1 回の取り消し
    assert_eq!(
        transform::step(
            &mut app,
            &Input {
                primary: true,
                ..Default::default()
            }
        ),
        Step::Done
    );
    assert_eq!(app.doc.undo_count(), undo + 1);
    app.apply(Action::Undo);
    assert_eq!(shape(&app, o), start, "1 回で戻る");
    // Shift+Tab でスナップを常にする（そのときの Ctrl は逆）
    let from = begin(&mut app, Kind::Grab);
    step_keys(&mut app, &[(Key::Tab, Modifiers::SHIFT), key(Key::X)]);
    assert!(app.objects.snap);
    step_pointer(&mut app, from + vec2(37.0, 0.0), false);
    let d = shape(&app, o).center[0] - start.center[0];
    assert!(on_step(d), "{d}");
    step_pointer(&mut app, from + vec2(37.0, 0.0), true);
    assert!(!near(shape(&app, o).center[0] - start.center[0], d));
    // 右クリックでやめる
    assert_eq!(
        transform::step(
            &mut app,
            &Input {
                secondary: true,
                ..Default::default()
            }
        ),
        Step::Done
    );
    assert_eq!(shape(&app, o), start);
    // 刻みは欄で変えられる（回転は 15°）
    app.objects.steps.rotation = 15.0;
    let from = begin(&mut app, Kind::Rotate);
    step_keys(&mut app, &[key(Key::Z)]);
    // 中心のまわりに、右から上へ回す
    let r = 80.0;
    step_pointer(&mut app, from + vec2(r, 0.0), false);
    for i in 1..=8 {
        let a = (i as f32 * 5.0).to_radians();
        step_pointer(&mut app, from + vec2(r * a.cos(), -r * a.sin()), false);
    }
    let crate::objects::transform::Value::Turn { degrees, .. } = transform_state(&app).value else {
        panic!("回転");
    };
    assert!(
        (degrees.abs() / 15.0 - (degrees.abs() / 15.0).round()).abs() < 1e-4,
        "{degrees}"
    );
    step_keys(&mut app, &[key(Key::Escape)]);
    assert_eq!(shape(&app, o), start);
}

#[test]
fn rotate_and_scale_use_world_axes_object_axes_planes_and_typed_values() {
    let mut app = cube();
    let layer = every_kind(&mut app);
    assert!(app.set_mode(EditorMode::Edit));
    let o = Object::Shape(Target::Projection(layer));
    select(&mut app, Some(o));
    // Y のまわりに 90°
    begin(&mut app, Kind::Rotate);
    step_keys(&mut app, &[key(Key::Y)]);
    type_value(&mut app, "90");
    let q = sg::rotation_of(&shape(&app, o));
    assert!(
        q.angle_between(Quat::from_rotation_y(90f32.to_radians())) < 1e-3,
        "{q:?}"
    );
    assert_eq!(step_keys(&mut app, &[key(Key::Enter)]), Step::Done);
    // 物の向きの X（X を 2 回）に 1: 回した箱の X は、世界の −Z
    let turned = shape(&app, o);
    begin(&mut app, Kind::Grab);
    step_keys(&mut app, &[key(Key::X), key(Key::X)]);
    assert_eq!(
        transform_state(&app).constraint,
        Constraint::Axis {
            axis: 0,
            local: true
        }
    );
    type_value(&mut app, "1");
    let s = shape(&app, o);
    assert!(
        near(s.center[2], turned.center[2] - 1.0) && near(s.center[0], turned.center[0]),
        "{:?}",
        s.center
    );
    assert_eq!(transform_state(&app).status(Lang::En), "Move Local X 1");
    // 3 回目で軸なし
    step_keys(&mut app, &[key(Key::X)]);
    assert_eq!(transform_state(&app).constraint, Constraint::Free);
    step_keys(&mut app, &[key(Key::Escape)]);
    assert_eq!(shape(&app, o), turned);
    // 拡縮: 一様に 2 倍・Z だけ・Z を除いた面。大きさの軸は物の向きなので、1 回目の Z から物の向きの軸（線と札も「ローカル」）
    for (keys, wanted, constraint, status) in [
        (vec![], [2.0, 2.0, 2.0], Constraint::Free, "拡縮 2"),
        (
            vec![key(Key::Z)],
            [1.0, 1.0, 2.0],
            Constraint::Axis {
                axis: 2,
                local: true,
            },
            "拡縮 ローカル Z 2",
        ),
        (
            vec![(Key::Z, Modifiers::SHIFT)],
            [2.0, 2.0, 1.0],
            Constraint::Plane {
                axis: 2,
                local: true,
            },
            "拡縮 ローカル XY 2",
        ),
    ] {
        begin(&mut app, Kind::Scale);
        step_keys(&mut app, &keys);
        type_value(&mut app, "2");
        let s = shape(&app, o);
        for (i, k) in wanted.iter().enumerate() {
            assert!(
                near(s.size[i], turned.size[i] * k),
                "{keys:?} {:?} {:?}",
                s.size,
                turned.size
            );
        }
        assert_eq!(transform_state(&app).constraint, constraint);
        assert_eq!(transform_state(&app).status(Lang::Ja), status);
        // 軸の線は物の向き（回した箱の Z は世界の X）
        if let Constraint::Axis { axis, local } = constraint {
            let d = transform_state(&app).axis_direction(axis, local);
            assert!((d - Vec3::X).length() < 1e-4, "{d}");
        }
        // 同じ軸をもう一度押すと軸なし（拡縮は世界の軸を持たない）
        step_keys(&mut app, &keys);
        assert_eq!(transform_state(&app).constraint, Constraint::Free);
        step_keys(&mut app, &[key(Key::Escape)]);
    }
    // 打つ値: - は符号を替え、Backspace で 1 字ずつ消す
    begin(&mut app, Kind::Rotate);
    step_keys(&mut app, &[key(Key::Z)]);
    type_value(&mut app, "45-");
    assert_eq!(transform_state(&app).typed, "-45");
    type_value(&mut app, ".5");
    assert_eq!(transform_state(&app).typed, "-45.5");
    step_keys(&mut app, &[key(Key::Backspace), key(Key::Backspace)]);
    assert_eq!(transform_state(&app).typed, "-45");
    step_keys(&mut app, &[key(Key::Escape)]);
    assert_eq!(shape(&app, o), turned);
}

#[test]
fn a_gradient_point_only_moves_and_can_be_deleted_but_not_the_last_one() {
    let mut app = cube();
    let layer = every_kind(&mut app);
    assert!(app.set_mode(EditorMode::Edit));
    let point = |index| Object::Point {
        layer,
        channel: Channel::Roughness,
        index,
    };
    select(&mut app, Some(point(0)));
    let before = marker_of(&app, point(0)).unwrap();
    let undo = app.doc.undo_count();
    begin(&mut app, Kind::Grab);
    step_keys(&mut app, &[key(Key::Y)]);
    type_value(&mut app, "0.5");
    let after = marker_of(&app, point(0)).unwrap();
    assert!(
        (after - (before + Vec3::Y * 0.5)).length() < 1e-5,
        "{after}"
    );
    step_keys(&mut app, &[key(Key::Enter)]);
    assert_eq!(app.doc.undo_count(), undo + 1);
    // 回転・拡縮は断る
    for kind in [Kind::Rotate, Kind::Scale] {
        app.apply(Action::Object(ObjectAction::Transform(kind)));
        assert!(app.objects.request.is_none());
        assert!(app.message.contains("点は移動だけです"), "{}", app.message);
    }
    // 2D のキャンバスの点の編集は受けない（編集のモードの 2D は見るだけ）
    assert!(crate::fillfx::points::target(&app).is_none());
    // 消す: 2 つのうち 1 つは消せる、最後の 1 つは断る
    app.apply(Action::Object(ObjectAction::Delete));
    let left = |app: &AppState| {
        app.doc
            .layer(layer)
            .unwrap()
            .fill_points(Channel::Roughness)
            .unwrap()
            .points
            .len()
    };
    assert_eq!(left(&app), 1);
    assert_eq!(app.doc.undo_count(), undo + 2);
    select(&mut app, Some(point(0)));
    app.apply(Action::Object(ObjectAction::Delete));
    assert_eq!(left(&app), 1);
    assert!(app.message.contains("最後の点"), "{}", app.message);
}

#[test]
fn reset_hide_and_delete_act_on_the_selected_shape() {
    let mut app = cube();
    let layer = every_kind(&mut app);
    assert!(app.set_mode(EditorMode::Edit));
    let boxed = Object::Shape(Target::Projection(layer));
    select(&mut app, Some(boxed));
    let fitted = shape(&app, boxed);
    // 動かして回して大きくしてから、位置・回転・大きさを 1 つずつ戻す（1 つが 1 回の取り消し）
    for (kind, text) in [(Kind::Grab, "1"), (Kind::Rotate, "30"), (Kind::Scale, "2")] {
        begin(&mut app, kind);
        step_keys(&mut app, &[key(Key::X)]);
        type_value(&mut app, text);
        step_keys(&mut app, &[key(Key::Enter)]);
    }
    let changed = shape(&app, boxed);
    assert_ne!(changed.center, fitted.center);
    let undo = app.doc.undo_count();
    app.apply(Action::Object(ObjectAction::Reset(Kind::Grab)));
    let s = shape(&app, boxed);
    assert_eq!(s.center, fitted.center);
    assert_eq!(s.rotation, changed.rotation, "回転はそのまま");
    assert_eq!(app.doc.undo_count(), undo + 1);
    app.apply(Action::Object(ObjectAction::Reset(Kind::Rotate)));
    app.apply(Action::Object(ObjectAction::Reset(Kind::Scale)));
    assert_eq!(shape(&app, boxed), fitted);
    assert_eq!(app.doc.undo_count(), undo + 3);
    // 投影の置き場は消せない
    app.apply(Action::Object(ObjectAction::Delete));
    assert!(app.message.contains("投影"), "{}", app.message);
    assert!(marker_of(&app, boxed).is_some());
    // H で印を隠し、Alt+H で出す（文書は変えない）
    app.apply(Action::Object(ObjectAction::Hide));
    assert!(app.objects.selected.is_none());
    assert!(!markers(&app).iter().any(|m| m.object == boxed));
    app.apply(Action::Object(ObjectAction::Reveal));
    assert!(markers(&app).iter().any(|m| m.object == boxed));
    assert_eq!(app.doc.undo_count(), undo + 3);
    // グラデーションデカールとフィルターの形は消せる（1 回の取り消し）
    let filter = markers(&app)
        .into_iter()
        .find(|m| matches!(m.object, Object::Shape(Target::Filter(..))))
        .unwrap()
        .object;
    for o in [
        Object::Shape(Target::Gradient(layer, Channel::Color)),
        filter,
    ] {
        select(&mut app, Some(o));
        let undo = app.doc.undo_count();
        app.apply(Action::Object(ObjectAction::Delete));
        assert!(marker_of(&app, o).is_none(), "{o:?}: {}", app.message);
        assert_eq!(app.doc.undo_count(), undo + 1);
        assert_eq!(app.objects.selected, None);
    }
    // 選んでいなければ断る
    app.apply(Action::Object(ObjectAction::Delete));
    assert!(
        app.message.contains("選んだ物がありません"),
        "{}",
        app.message
    );
}

/// 立方体の手前の面（z = −0.5）に、パスの点を 2 つ打つ（ペイントのモードのパスのツール）。
fn path_on_front(app: &mut AppState) -> (LayerId, u128) {
    app.apply(Action::SelectTool(Tool::Path));
    for x in [-0.25f32, 0.15] {
        let at = screen_of(app, Vec3::new(x, 0.1, -0.5));
        crate::pathtool::surface::press(app, RECT, at, StrokeSource::Mouse);
        crate::pathtool::surface::release(app, RECT, at, StrokeSource::Mouse);
    }
    let layer = app.selected_layer.unwrap();
    let id = app.doc.layer(layer).unwrap().paths()[0].path.id();
    (layer, id)
}

#[test]
fn a_3d_path_moves_as_an_overlay_lands_on_the_surface_when_confirmed_and_refuses_points_off_it() {
    let mut app = cube();
    let (layer, id) = path_on_front(&mut app);
    let before = path_points(&app, layer, id).expect("パスの点");
    assert_eq!(before.len(), 2, "{}", app.message);
    assert!(app.set_mode(EditorMode::Edit));
    let o = Object::Path { layer, id };
    assert!(markers(&app).iter().any(|m| m.object == o));
    select(&mut app, Some(o));
    let undo = app.doc.undo_count();
    // X に 0.1: 途中は重ねて見せるだけ（文書は変えない）
    begin(&mut app, Kind::Grab);
    step_keys(&mut app, &[key(Key::X)]);
    type_value(&mut app, "0.1");
    assert_eq!(app.doc.undo_count(), undo);
    assert_eq!(path_points(&app, layer, id).unwrap(), before);
    let preview = transform_state(&app).path_preview().unwrap();
    assert!((preview[0] - (before[0] + Vec3::X * 0.1)).length() < 1e-5);
    // 決める: 面の点へ当て直して 1 回の取り消し
    assert_eq!(
        step_keys(&mut app, &[key(Key::Enter)]),
        Step::Done,
        "{}",
        app.message
    );
    let after = path_points(&app, layer, id).unwrap();
    for (a, b) in after.iter().zip(&before) {
        assert!((*a - (*b + Vec3::X * 0.1)).length() < 1e-3, "{a} {b}");
    }
    assert_eq!(app.doc.undo_count(), undo + 1);
    app.apply(Action::Undo);
    assert_eq!(path_points(&app, layer, id).unwrap(), before);
    // 面から遠く（Z に −5）: 決めるのを断り、続けられる。Esc でやめると元のまま
    begin(&mut app, Kind::Grab);
    step_keys(&mut app, &[key(Key::Z)]);
    type_value(&mut app, "-5");
    assert_eq!(step_keys(&mut app, &[key(Key::Enter)]), Step::Open);
    assert!(app.message.contains("面に乗りません"), "{}", app.message);
    assert!(transforming(&app));
    assert_eq!(step_keys(&mut app, &[key(Key::Escape)]), Step::Done);
    assert_eq!(path_points(&app, layer, id).unwrap(), before);
    assert_eq!(app.doc.undo_count(), undo);
    // 消す（1 回の取り消し）
    app.apply(Action::Object(ObjectAction::Delete));
    assert!(path_points(&app, layer, id).is_none(), "{}", app.message);
    assert_eq!(app.doc.undo_count(), undo + 1);
}

fn figure() -> AppState {
    let mut app = AppState::new(64, 64);
    app.apply(Action::Pose(PoseAction::LoadFigure));
    assert!(app.view3d.pose.session.is_some(), "{}", app.message);
    app.view3d.view_rect = Some(RECT);
    app.view3d.visible = true;
    app
}

fn bone(app: &AppState, name: &str) -> usize {
    let s = app.view3d.pose.session.as_ref().unwrap();
    s.rig.bones().iter().position(|b| b.name == name).unwrap()
}

fn local_of(app: &AppState, b: usize) -> yolu_core::skin::BoneTransform {
    app.view3d.pose.session.as_ref().unwrap().pose().locals[b]
}

fn pose_undo(app: &AppState) -> usize {
    app.view3d.pose.session.as_ref().unwrap().undo_len()
}

#[test]
fn bones_move_rotate_and_scale_with_one_pose_undo_step_and_cancel_restores() {
    let mut app = figure();
    assert!(app.set_mode(EditorMode::Pose), "{}", app.message);
    let b = bone(&app, "右上腕");
    app.view3d.pose.session.as_mut().unwrap().selected = Some(b);
    let rest = local_of(&app, b);
    // 移動: 世界の Y に 0.1（親の空間へ写して足す）
    begin(&mut app, Kind::Grab);
    step_keys(&mut app, &[key(Key::Y)]);
    type_value(&mut app, "0.1");
    assert_ne!(local_of(&app, b).translation, rest.translation);
    let s = app.view3d.pose.session.as_ref().unwrap();
    let mut start = s.pose().clone();
    start.locals[b] = rest;
    let (now, was) = (
        s.rig.world_matrices(s.pose()).unwrap(),
        s.rig.world_matrices(&start).unwrap(),
    );
    let shift =
        yolu_core::skin::Rig::bone_origin(&now, b) - yolu_core::skin::Rig::bone_origin(&was, b);
    assert!((shift - Vec3::Y * 0.1).length() < 1e-4, "{shift}");
    assert_eq!(pose_undo(&app), 0, "途中は積まない");
    // 途中で、ポーズの欄の続けて変える操作の確定に巻き込まれない
    crate::view3d::pose::edit::finish_live_edit(&mut app, false);
    assert!(app.view3d.pose.session.as_ref().unwrap().is_editing());
    step_keys(&mut app, &[key(Key::Enter)]);
    assert_eq!(pose_undo(&app), 1);
    // 回転: Z のまわりに 90°（Rig::rotate_bone_world と同じ）
    let before = app.view3d.pose.session.as_ref().unwrap().pose().clone();
    begin(&mut app, Kind::Rotate);
    step_keys(&mut app, &[key(Key::Z)]);
    type_value(&mut app, "90");
    let s = app.view3d.pose.session.as_ref().unwrap();
    let mut wanted = before.clone();
    let w = s.rig.world_matrices(&before).unwrap();
    s.rig
        .rotate_bone_world(&mut wanted, &w, b, Vec3::Z, 90f32.to_radians());
    assert!(
        s.pose().locals[b]
            .rotation
            .angle_between(wanted.locals[b].rotation)
            < 1e-4
    );
    step_keys(&mut app, &[key(Key::Enter)]);
    assert_eq!(pose_undo(&app), 2);
    // 拡縮: X だけ 2 倍（ボーンの向きの軸）。Esc で戻して積まない
    begin(&mut app, Kind::Scale);
    step_keys(&mut app, &[key(Key::X)]);
    type_value(&mut app, "2");
    let s = local_of(&app, b).scale;
    assert!(
        (s - rest.scale * Vec3::new(2.0, 1.0, 1.0)).length() < 1e-5,
        "{s}"
    );
    step_keys(&mut app, &[key(Key::Escape)]);
    assert_eq!(local_of(&app, b).scale, rest.scale);
    assert_eq!(pose_undo(&app), 2);
    assert!(!app.view3d.pose.session.as_ref().unwrap().is_editing());
    // Alt+R: 回転を戻す（ポーズの取り消し 1 段）
    app.apply(Action::Object(ObjectAction::Reset(Kind::Rotate)));
    assert!(local_of(&app, b).rotation.angle_between(rest.rotation) < 1e-5);
    assert_eq!(pose_undo(&app), 3);
    // ボーンを選んでいなければ断る
    app.view3d.pose.session.as_mut().unwrap().selected = None;
    app.apply(Action::Object(ObjectAction::Transform(Kind::Grab)));
    assert!(app.objects.request.is_none());
    assert!(
        app.message.contains("ボーンを選んでいません"),
        "{}",
        app.message
    );
}

#[test]
fn transforms_are_refused_while_dragging_and_finish_when_the_mode_changes() {
    let mut app = cube();
    let layer = every_kind(&mut app);
    assert!(app.set_mode(EditorMode::Edit));
    let o = Object::Shape(Target::Projection(layer));
    // 選んでいなければ断る
    app.apply(Action::Object(ObjectAction::Transform(Kind::Grab)));
    assert!(app.objects.request.is_none());
    assert!(
        app.message.contains("選んだ物がありません"),
        "{}",
        app.message
    );
    select(&mut app, Some(o));
    // 形のギズモのドラッグの途中は断る
    app.fillfx.drag = Some(gizmo::ShapeDrag {
        handle: sg::Handle::MoveX,
        start: shape(&app, o),
        from: yolu_core::glam::Vec2::ZERO,
        grab: yolu_core::glam::Vec2::ZERO,
        target: Target::Projection(layer),
        source: gizmo::Source::Mouse,
        doc_id: app.doc.id(),
    });
    app.apply(Action::Object(ObjectAction::Transform(Kind::Grab)));
    assert!(app.objects.request.is_none());
    assert!(
        app.message.contains("ほかの操作の途中です"),
        "{}",
        app.message
    );
    app.fillfx.drag = None;
    // 頼みは置ける（始めるのはキーの処理）
    app.apply(Action::Object(ObjectAction::Transform(Kind::Grab)));
    assert_eq!(app.objects.request, Some(Kind::Grab));
    app.objects.request = None;
    // 途中は G/R/S をもう一度頼めない（キーの繰り返しで始め直さない）
    let start = shape(&app, o);
    let undo = app.doc.undo_count();
    begin(&mut app, Kind::Grab);
    app.apply(Action::Object(ObjectAction::Transform(Kind::Rotate)));
    assert!(app.objects.request.is_none());
    // 途中は Delete・Alt+G を断る
    app.apply(Action::Object(ObjectAction::Delete));
    app.apply(Action::Object(ObjectAction::Reset(Kind::Grab)));
    assert!(marker_of(&app, o).is_some());
    // 途中でモードを替えると、そこまでで決めてから替える（1 回の取り消し）
    step_keys(&mut app, &[key(Key::X)]);
    type_value(&mut app, "1");
    assert!(app.set_mode(EditorMode::Paint));
    assert!(!transforming(&app));
    assert!(!app.doc.is_coalescing());
    assert!(near(shape(&app, o).center[0], start.center[0] + 1.0));
    assert_eq!(app.doc.undo_count(), undo + 1);
    // ペイントのモードでは断る
    app.apply(Action::Object(ObjectAction::Transform(Kind::Grab)));
    assert!(app.objects.request.is_none());
    // 編集のモードでも、3D ビューが見えていなければ断る
    assert!(app.set_mode(EditorMode::Edit));
    app.view3d.visible = false;
    app.apply(Action::Object(ObjectAction::Transform(Kind::Grab)));
    assert!(app.objects.request.is_none());
    assert!(app.message.contains("3D ビュー"), "{}", app.message);
}

#[test]
fn a_transform_left_open_by_a_closed_popup_or_a_new_document_does_not_linger() {
    let mut app = cube();
    let layer = every_kind(&mut app);
    assert!(app.set_mode(EditorMode::Edit));
    let o = Object::Shape(Target::Projection(layer));
    select(&mut app, Some(o));
    let start = shape(&app, o);
    let undo = app.doc.undo_count();
    // ポップアップ無しで残った G/R/S は、フレームの初めにそこまでで決める
    begin(&mut app, Kind::Grab);
    step_keys(&mut app, &[key(Key::X)]);
    type_value(&mut app, "0.5");
    transform::settle(&mut app);
    assert!(!transforming(&app));
    assert!(!app.doc.is_coalescing());
    assert!(near(shape(&app, o).center[0], start.center[0] + 0.5));
    assert_eq!(app.doc.undo_count(), undo + 1);
    // 物が無くなったら、何も当てずに終える
    begin(&mut app, Kind::Grab);
    app.objects.selected = None;
    app.apply(Action::Fill(FillOp::ProjectionMode {
        layer,
        mode: ProjectionMode::Uv,
    }));
    assert_eq!(step_keys(&mut app, &[key(Key::X)]), Step::Done);
    assert!(!transforming(&app));
}

#[test]
fn deleting_a_point_clears_the_selection_renumbers_hidden_points_and_undo_brings_it_back() {
    let mut app = cube();
    let layer = every_kind(&mut app);
    // 3 つ目の点を加える
    let mut g = app
        .doc
        .layer(layer)
        .unwrap()
        .fill_points(Channel::Roughness)
        .unwrap()
        .clone();
    let mut third = g.points[0];
    third.position[1] += 0.3;
    g.points.push(third);
    app.apply(Action::Fill(FillOp::Points {
        layer,
        channel: Channel::Roughness,
        points: Some(Box::new(g)),
        coalesce: false,
    }));
    assert!(app.set_mode(EditorMode::Edit));
    let point = |index| Object::Point {
        layer,
        channel: Channel::Roughness,
        index,
    };
    let third_at = marker_of(&app, point(2)).unwrap();
    // 3 つ目を隠し、1 つ目を消す
    select(&mut app, Some(point(2)));
    app.apply(Action::Object(ObjectAction::Hide));
    select(&mut app, Some(point(0)));
    let undo = app.doc.undo_count();
    app.apply(Action::Object(ObjectAction::Delete));
    assert_eq!(app.doc.undo_count(), undo + 1);
    assert_eq!(app.objects.selected, None, "隣の点へ選びが移らない");
    // 隠した点は番号が詰まっても同じ点（印は出ない）
    assert_eq!(app.objects.hidden, vec![point(1)]);
    assert_eq!(marker_of(&app, point(1)), Some(third_at));
    assert!(!markers(&app).iter().any(|m| m.object == point(1)));
    // もう一度 Delete: 何も消さない
    app.apply(Action::Object(ObjectAction::Delete));
    assert_eq!(app.doc.undo_count(), undo + 1);
    assert!(
        app.message.contains("選んだ物がありません"),
        "{}",
        app.message
    );
    // 取り消しで戻る
    app.apply(Action::Undo);
    let back = app
        .doc
        .layer(layer)
        .unwrap()
        .fill_points(Channel::Roughness)
        .unwrap()
        .points
        .len();
    assert_eq!(back, 3);
}

#[test]
fn objects_on_a_hidden_layer_or_in_a_hidden_group_cannot_be_selected_moved_or_deleted() {
    let mut app = cube();
    let layer = every_kind(&mut app);
    assert!(app.set_mode(EditorMode::Edit));
    let decal = Object::Shape(Target::Gradient(layer, Channel::Color));
    let check_hidden = |app: &mut AppState| {
        sync(app);
        assert!(markers(app).is_empty());
        assert_eq!(app.objects.selected, None, "隠したら選びを外す");
        assert!(!selectable(app, decal));
        assert_eq!(gizmo_target(app), None, "取っ手も枠も出さない");
        let undo = app.doc.undo_count();
        // 選んだままにしても（外から選んでも）、G・Alt+G・Delete は何もしない
        app.objects.selected = Some(decal);
        app.apply(Action::Object(ObjectAction::Transform(Kind::Grab)));
        assert!(app.objects.request.is_none());
        assert!(transform::begin(app, Kind::Grab, RECT.center(), 0).is_err());
        app.apply(Action::Object(ObjectAction::Reset(Kind::Grab)));
        app.apply(Action::Object(ObjectAction::Delete));
        assert_eq!(app.doc.undo_count(), undo);
        assert!(app
            .doc
            .layer(layer)
            .unwrap()
            .fill_gradient(Channel::Color)
            .is_some());
        app.objects.selected = None;
    };
    // レイヤーの目を消す
    select(&mut app, Some(decal));
    app.apply(Action::ToggleVisible(layer));
    check_hidden(&mut app);
    app.apply(Action::ToggleVisible(layer));
    assert!(selectable(&app, decal));
    // グループに入れて、グループの目を消す
    app.select_single_layer(layer);
    app.apply(Action::M2(crate::m2::Edit::GroupSelected));
    let group = app
        .doc
        .layer(layer)
        .unwrap()
        .parent()
        .expect("グループの中");
    select(&mut app, Some(decal));
    app.apply(Action::ToggleVisible(group));
    check_hidden(&mut app);
    // G/R/S の途中に物が選べなくなったら、続けない
    app.apply(Action::ToggleVisible(group));
    select(&mut app, Some(decal));
    begin(&mut app, Kind::Grab);
    app.apply(Action::ToggleVisible(group));
    assert_eq!(step_keys(&mut app, &[key(Key::X)]), Step::Done);
    assert!(!transforming(&app));
}

#[test]
fn a_typed_scale_of_minus_or_zero_keeps_the_last_value_and_a_negative_factor_is_refused() {
    let mut app = cube();
    let layer = every_kind(&mut app);
    assert!(app.set_mode(EditorMode::Edit));
    let o = Object::Shape(Target::Projection(layer));
    select(&mut app, Some(o));
    let start = shape(&app, o);
    begin(&mut app, Kind::Scale);
    step_keys(&mut app, &[key(Key::X)]);
    type_value(&mut app, "3");
    assert!(near(shape(&app, o).size[0], start.size[0] * 3.0));
    // 「-」で負になる: 潰さず 3 倍のまま、理由を出す
    type_value(&mut app, "-");
    assert_eq!(transform_state(&app).typed, "-3");
    assert!(near(shape(&app, o).size[0], start.size[0] * 3.0));
    assert!(app.message.contains("負の倍率"), "{}", app.message);
    // 「-」だけ・0 の間も前のまま
    step_keys(&mut app, &[key(Key::Backspace)]);
    assert_eq!(transform_state(&app).typed, "-");
    assert!(near(shape(&app, o).size[0], start.size[0] * 3.0));
    type_value(&mut app, "-0");
    assert_eq!(transform_state(&app).typed, "0");
    assert!(near(shape(&app, o).size[0], start.size[0] * 3.0));
    step_keys(&mut app, &[key(Key::Escape)]);
    assert_eq!(shape(&app, o), start);
    // ボーンも負の倍率は断る
    let mut app = figure();
    assert!(app.set_mode(EditorMode::Pose));
    let b = bone(&app, "右上腕");
    app.view3d.pose.session.as_mut().unwrap().selected = Some(b);
    let rest = local_of(&app, b).scale;
    begin(&mut app, Kind::Scale);
    step_keys(&mut app, &[key(Key::X)]);
    type_value(&mut app, "2");
    type_value(&mut app, "-");
    let s = local_of(&app, b).scale;
    assert!(s.x > 0.0 && (s.x - rest.x * 2.0).abs() < 1e-5, "{s}");
    step_keys(&mut app, &[key(Key::Escape)]);
    assert_eq!(local_of(&app, b).scale, rest);
}

/// 表（−Z を向く）と裏（+Z を向く）が 0.02 離れた薄い板（どちらもマテリアル 0）。
fn thin_plate() -> crate::view3d::model::ViewModel {
    use yolu_core::geometry::{ModelMesh, Submesh};
    use yolu_core::glam::Vec2;
    let quad = |name: &str, z: f32, indices: Vec<u32>| ModelMesh {
        name: name.into(),
        positions: vec![
            Vec3::new(-1.0, -1.0, z),
            Vec3::new(1.0, -1.0, z),
            Vec3::new(-1.0, 1.0, z),
            Vec3::new(1.0, 1.0, z),
        ],
        normals: Vec::new(),
        uvs: vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 1.0),
        ],
        submeshes: vec![Submesh {
            material: 0,
            indices,
        }],
    };
    crate::view3d::model::ViewModel::new(
        "板",
        vec![
            quad("表", -0.01, vec![0, 2, 1, 2, 3, 1]),
            quad("裏", 0.01, vec![0, 1, 2, 2, 1, 3]),
        ],
        vec![Some("板".into())],
        1,
    )
    .expect("モデル")
}

#[test]
fn a_path_turned_on_a_thin_plate_stays_on_the_face_it_was_drawn_on() {
    let mut app = AppState::new(64, 64);
    app.view3d.set_model(thin_plate());
    app.view3d.material = 0;
    app.view3d.camera.yaw = 0.0;
    app.view3d.camera.pitch = 0.0;
    app.view3d.view_rect = Some(RECT);
    app.view3d.visible = true;
    app.apply(Action::SelectTool(Tool::Path));
    for x in [-0.1f32, 0.1] {
        let at = screen_of(&app, Vec3::new(x, 0.0, -0.01));
        crate::pathtool::surface::press(&mut app, RECT, at, StrokeSource::Mouse);
        crate::pathtool::surface::release(&mut app, RECT, at, StrokeSource::Mouse);
    }
    let layer = app.selected_layer.unwrap();
    let id = app.doc.layer(layer).unwrap().paths()[0].path.id();
    let normals = |app: &AppState| -> Vec<Vec3> {
        path_points_with_normals(app, layer, id)
            .unwrap()
            .into_iter()
            .map(|(_, n)| n)
            .collect()
    };
    assert!(normals(&app).iter().all(|n| n.z < -0.9), "表に描いた");
    assert!(app.set_mode(EditorMode::Edit));
    select(&mut app, Some(Object::Path { layer, id }));
    // Y のまわりに 30°: 片方の点は板の裏の側（裏の面のほうが近い所）へ出るが、表の面へ載せ直す
    begin(&mut app, Kind::Rotate);
    step_keys(&mut app, &[key(Key::Y)]);
    type_value(&mut app, "30");
    let preview = transform_state(&app).path_preview().unwrap();
    assert!(preview.iter().any(|p| p.z > 0.02), "{preview:?}");
    assert_eq!(
        step_keys(&mut app, &[key(Key::Enter)]),
        Step::Done,
        "{}",
        app.message
    );
    assert!(
        normals(&app).iter().all(|n| n.z < -0.9),
        "裏の面に乗らない: {:?}",
        normals(&app)
    );
    for p in path_points(&app, layer, id).unwrap() {
        assert!((p.z + 0.01).abs() < 1e-4, "{p}");
    }
}

#[test]
fn a_3d_path_turns_and_scales_on_its_face_and_deleting_it_leaves_no_path_being_edited() {
    let mut app = cube();
    let (layer, id) = path_on_front(&mut app);
    assert!(app.path.active.is_some(), "描いたパスは編集中");
    let before = path_points(&app, layer, id).unwrap();
    let middle = polyline_middle(&before).unwrap();
    assert!(app.set_mode(EditorMode::Edit));
    let o = Object::Path { layer, id };
    select(&mut app, Some(o));
    // 面の法線（Z）のまわりに 90°: 面の中で回る
    begin(&mut app, Kind::Rotate);
    step_keys(&mut app, &[key(Key::Z)]);
    type_value(&mut app, "90");
    assert_eq!(
        step_keys(&mut app, &[key(Key::Enter)]),
        Step::Done,
        "{}",
        app.message
    );
    let turned = path_points(&app, layer, id).unwrap();
    for (a, b) in turned.iter().zip(&before) {
        let wanted = middle + Quat::from_rotation_z(90f32.to_radians()) * (*b - middle);
        assert!((*a - wanted).length() < 1e-3, "{a} {wanted}");
    }
    // 拡縮: 向きを持たないので、軸は世界の軸（札に「ローカル」が付かない）。Z を除いた面で 2 倍
    begin(&mut app, Kind::Scale);
    step_keys(&mut app, &[(Key::Z, Modifiers::SHIFT)]);
    assert_eq!(
        transform_state(&app).constraint,
        Constraint::Plane {
            axis: 2,
            local: false
        }
    );
    type_value(&mut app, "2");
    assert_eq!(transform_state(&app).status(Lang::Ja), "拡縮 XY 2");
    assert_eq!(
        step_keys(&mut app, &[key(Key::Enter)]),
        Step::Done,
        "{}",
        app.message
    );
    let scaled = path_points(&app, layer, id).unwrap();
    let mid = polyline_middle(&turned).unwrap();
    for (a, b) in scaled.iter().zip(&turned) {
        let wanted = mid + (*b - mid) * 2.0;
        assert!((*a - wanted).length() < 1e-3, "{a} {wanted}");
    }
    // 消す: 編集中のパス・選んだ点を外す。取り消しで戻る
    let undo = app.doc.undo_count();
    app.apply(Action::Object(ObjectAction::Delete));
    assert!(path_points(&app, layer, id).is_none());
    assert!(app.path.active.is_none() && app.path.selected.is_none());
    assert_eq!(app.objects.selected, None);
    app.apply(Action::Undo);
    assert_eq!(app.doc.undo_count(), undo);
    assert!(path_points(&app, layer, id).is_some());
}

#[test]
fn gradient_decals_and_filter_shapes_move_turn_and_scale_with_one_undo_step_each() {
    let mut app = cube();
    let layer = every_kind(&mut app);
    assert!(app.set_mode(EditorMode::Edit));
    let filter = markers(&app)
        .into_iter()
        .find(|m| matches!(m.object, Object::Shape(Target::Filter(..))))
        .unwrap()
        .object;
    for o in [
        Object::Shape(Target::Gradient(layer, Channel::Color)),
        filter,
    ] {
        select(&mut app, Some(o));
        let start = shape(&app, o);
        let undo = app.doc.undo_count();
        begin(&mut app, Kind::Grab);
        step_keys(&mut app, &[key(Key::X)]);
        type_value(&mut app, "0.2");
        step_keys(&mut app, &[key(Key::Enter)]);
        assert!(
            near(shape(&app, o).center[0], start.center[0] + 0.2),
            "{o:?}"
        );
        begin(&mut app, Kind::Rotate);
        step_keys(&mut app, &[key(Key::Y)]);
        type_value(&mut app, "90");
        step_keys(&mut app, &[key(Key::Enter)]);
        let q = sg::rotation_of(&shape(&app, o));
        assert!(
            q.angle_between(Quat::from_rotation_y(90f32.to_radians()) * sg::rotation_of(&start))
                < 1e-3
        );
        begin(&mut app, Kind::Scale);
        type_value(&mut app, "2");
        step_keys(&mut app, &[key(Key::Enter)]);
        let s = shape(&app, o);
        assert!(
            near(s.size[0], sg::clamp_size(start.size[0] * 2.0)),
            "{o:?} {:?}",
            s.size
        );
        assert_eq!(app.doc.undo_count(), undo + 3, "{o:?}");
        // 消して、取り消しで戻る
        app.apply(Action::Object(ObjectAction::Delete));
        assert!(marker_of(&app, o).is_none());
        app.apply(Action::Undo);
        assert_eq!(shape(&app, o), s, "{o:?}");
    }
}

#[test]
fn a_plane_move_keeps_the_excluded_axis_and_a_typed_value_goes_along_the_next_axis() {
    let mut app = cube();
    let layer = every_kind(&mut app);
    assert!(app.set_mode(EditorMode::Edit));
    let o = Object::Shape(Target::Projection(layer));
    select(&mut app, Some(o));
    let start = shape(&app, o);
    // 上から見下ろす（XZ の面がポインタの動きを受ける）
    app.view3d.camera.pitch = 40.0;
    // Shift+Y: Y を除いた面（XZ）。ポインタを右上へ動かしても Y は変わらない
    let from = begin(&mut app, Kind::Grab);
    step_keys(&mut app, &[(Key::Y, Modifiers::SHIFT)]);
    assert_eq!(transform_state(&app).status(Lang::Ja), "移動 XZ 0.000");
    step_pointer(&mut app, from + vec2(40.0, -40.0), false);
    let s = shape(&app, o);
    assert!(near(s.center[1], start.center[1]), "{:?}", s.center);
    assert!(s.center[0] - start.center[0] > 0.05, "{:?}", s.center);
    // 打った値は、除いた軸の次の軸（Y の次は Z）
    type_value(&mut app, "0.4");
    let s = shape(&app, o);
    assert!(near(s.center[2], start.center[2] + 0.4) && near(s.center[0], start.center[0]));
    step_keys(&mut app, &[key(Key::Escape)]);
}

#[test]
fn bones_reset_position_and_scale_with_alt_g_and_alt_s() {
    let mut app = figure();
    assert!(app.set_mode(EditorMode::Pose));
    let b = bone(&app, "右上腕");
    app.view3d.pose.session.as_mut().unwrap().selected = Some(b);
    let rest = local_of(&app, b);
    for (kind, text) in [(Kind::Grab, "0.2"), (Kind::Scale, "2")] {
        begin(&mut app, kind);
        step_keys(&mut app, &[key(Key::Y)]);
        type_value(&mut app, text);
        step_keys(&mut app, &[key(Key::Enter)]);
    }
    let moved = local_of(&app, b);
    assert_ne!(moved.translation, rest.translation);
    assert_ne!(moved.scale, rest.scale);
    let undo = pose_undo(&app);
    app.apply(Action::Object(ObjectAction::Reset(Kind::Grab)));
    assert!((local_of(&app, b).translation - rest.translation).length() < 1e-5);
    assert_eq!(local_of(&app, b).scale, moved.scale, "大きさはそのまま");
    app.apply(Action::Object(ObjectAction::Reset(Kind::Scale)));
    assert!((local_of(&app, b).scale - rest.scale).length() < 1e-5);
    assert_eq!(pose_undo(&app), undo + 2);
}

#[test]
fn the_move_tool_starts_a_drag_from_a_marker_that_ends_on_release_and_lights_its_tool() {
    use crate::mode::EditTool;
    let mut app = cube();
    let layer = every_kind(&mut app);
    assert!(app.set_mode(EditorMode::Edit));
    let o = Object::Shape(Target::Projection(layer));
    // G をキーで始めると、帯は移動のツールを光らせる（選んでいるツールは選択のまま）
    select(&mut app, Some(o));
    assert_eq!(EditTool::shown(&app), EditTool::Select);
    begin(&mut app, Kind::Rotate);
    assert_eq!(EditTool::shown(&app), EditTool::Rotate);
    step_keys(&mut app, &[key(Key::Escape)]);
    // 選択のツール: 印を押すと選ぶだけ
    app.objects.selected = None;
    let at = screen_of(&app, marker_of(&app, o).unwrap());
    press(&mut app, RECT, at, gizmo::Source::Mouse);
    assert!(app.objects.drag_request.is_none());
    // 移動のツール: 選んだ物の印から押すと、ドラッグの頼み
    app.apply(Action::Mode(crate::mode::ModeAction::Tool(EditTool::Move)));
    press(&mut app, RECT, at, gizmo::Source::Mouse);
    let (kind, from, source) = app.objects.drag_request.take().expect("ドラッグを始める");
    assert_eq!((kind, source), (Kind::Grab, gizmo::Source::Mouse));
    let start = shape(&app, o);
    let undo = app.doc.undo_count();
    transform::begin_with(&mut app, kind, from, 0, Some(source)).unwrap();
    // 動かす。押しの印（primary）では決めず、離して決める
    transform::step(
        &mut app,
        &Input {
            pointer: Some(from + vec2(50.0, 0.0)),
            primary: true,
            ..Default::default()
        },
    );
    assert!(transforming(&app));
    step_keys(&mut app, &[key(Key::X)]);
    let moved = shape(&app, o);
    assert!(moved.center[0] - start.center[0] > 0.05 && near(moved.center[1], start.center[1]));
    assert_eq!(
        transform::step(
            &mut app,
            &Input {
                released: true,
                ..Default::default()
            }
        ),
        Step::Done
    );
    assert_eq!(app.doc.undo_count(), undo + 1);
    // 点は回転のツールでは回らない（選ぶだけで、理由を出す）
    app.apply(Action::Mode(crate::mode::ModeAction::Tool(
        EditTool::Rotate,
    )));
    let point = Object::Point {
        layer,
        channel: Channel::Roughness,
        index: 0,
    };
    let p = screen_of(&app, marker_of(&app, point).unwrap());
    press(&mut app, RECT, p, gizmo::Source::Mouse);
    assert_eq!(app.objects.selected, Some(point));
    let (kind, from, source) = app.objects.drag_request.take().unwrap();
    assert!(transform::begin_with(&mut app, kind, from, 0, Some(source)).is_err());
}

#[test]
fn edit_in_3d_view_buttons_select_the_object_in_edit_mode() {
    let mut app = cube();
    let layer = every_kind(&mut app);
    assert!(app.set_mode(EditorMode::Edit));
    app.apply(Action::Fill(FillOp::EditGradient(Some((
        layer,
        Channel::Color,
    )))));
    assert_eq!(
        app.objects.selected,
        Some(Object::Shape(Target::Gradient(layer, Channel::Color)))
    );
    app.apply(Action::Fill(FillOp::EditPoints(Some((
        layer,
        Channel::Roughness,
    )))));
    assert_eq!(
        app.objects.selected,
        Some(Object::Point {
            layer,
            channel: Channel::Roughness,
            index: 0
        })
    );
    // ペイントのモードでは選ばない（今までどおり、その欄の編集に入るだけ）
    assert!(app.set_mode(EditorMode::Paint));
    app.objects.selected = None;
    app.apply(Action::Fill(FillOp::EditGradient(Some((
        layer,
        Channel::Color,
    )))));
    assert_eq!(app.objects.selected, None);
}
