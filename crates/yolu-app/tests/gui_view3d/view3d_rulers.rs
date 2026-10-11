//! 3D の定規（モデルの空間の定規。レイヤーが持つ文書の値）を 3D ビューで作る・寄せ先にする・見せる試験（egui_kittest。試しの立方体の上で、3D ビューの入力の道で）:
//! 面を押して引くと選んでいるレイヤーに作る（種類ごとの点・取り消し 1 回・特殊定規の印）、モデルの外で押したら作らない、対称は引いた線と視線で決めた面か
//! 境界の中心の X の鏡、ストロークの寄せ先が画面に写した線に乗る（直線・平行線・同心円・パース）、保存して開き直して同じ、「スナップする特殊定規の切り替え」が
//! ポインタのあるビューの空間を回す。絵は 3D の対称定規・直線定規・選んだ定規とギズモ（日英）。編集のモードで動かす試験は `view3d_rulers_edit.rs`。
use crate::common::*;
use crate::view3d_brush::{cube_view, press_with, release_with};
use egui::{pos2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::Harness;
use yolu_app::drafting::Constraint;
use yolu_app::lang::Lang;
use yolu_app::state::{Action, AppState, Tool};
use yolu_app::YoluApp;
use yolu_core::geometry::{pick, SurfaceHit};
use yolu_core::glam::{DVec2, DVec3, Vec2, Vec3};
use yolu_core::{Ruler, RulerKind, RulerPlace};

type H = Harness<'static, YoluApp>;

fn st(h: &H) -> &AppState {
    &h.state().state
}

fn screen(h: &H, rect: Rect, p: Vec3) -> Pos2 {
    let view = st(h).view3d.camera.view(rect.width(), rect.height());
    let s = view.to_screen(p).expect("カメラの前");
    pos2(rect.left() + s.x, rect.top() + s.y)
}

fn local(rect: Rect, p: Pos2) -> Vec2 {
    Vec2::new(p.x - rect.left(), p.y - rect.top())
}

fn hit_at(h: &H, rect: Rect, p: Pos2) -> Option<SurfaceHit> {
    let model = st(h).view3d.model.clone()?;
    let view = st(h).view3d.camera.view(rect.width(), rect.height());
    pick(&model.geometry, &view, local(rect, p))
}

fn tool(h: &mut H, t: Tool) {
    h.state_mut().state.apply(Action::SelectTool(t));
    h.run();
}

fn pull(h: &mut H, a: Pos2, b: Pos2) {
    drag(h, &[a, a + (b - a) * 0.5, b]);
}

fn kind(h: &mut H, k: RulerKind) {
    h.state_mut().state.rulers.kind = k;
}

/// 手前の面（−Z。カメラから見える）の 2 点。
fn front(h: &H, rect: Rect) -> (Pos2, Pos2) {
    (
        screen(h, rect, Vec3::new(-0.3, -0.3, -0.5)),
        screen(h, rect, Vec3::new(0.3, 0.3, -0.5)),
    )
}

/// 選んでいるレイヤーの定規。
fn rulers(h: &H) -> Vec<Ruler> {
    let layer = st(h).selected_layer.expect("選んでいるレイヤー");
    crate::common::rulers::of(st(h), layer)
}

fn points(r: &Ruler) -> (DVec3, DVec3, DVec3) {
    match r.place {
        RulerPlace::Model { a, b, up } => (a, b, up),
        RulerPlace::Canvas { .. } => panic!("2D の定規"),
    }
}

fn near3(a: DVec3, b: Vec3) -> bool {
    a.distance(b.as_dvec3()) < 1e-4
}

#[test]
fn pulling_on_the_model_makes_a_3d_ruler_of_the_chosen_kind_in_one_undo_step() {
    for k in [
        RulerKind::Line,
        RulerKind::Parallel,
        RulerKind::Concentric,
        RulerKind::Perspective,
    ] {
        let (mut h, rect) = cube_view();
        tool(&mut h, Tool::Ruler);
        kind(&mut h, k);
        let (a, b) = front(&h, rect);
        let (pa, pb) = (hit_at(&h, rect, a).unwrap(), hit_at(&h, rect, b).unwrap());
        let steps = st(&h).doc.undo_count();
        pull(&mut h, a, b);
        let made = rulers(&h);
        assert_eq!(made.len(), 1, "{k:?} {}", st(&h).message);
        let r = &made[0];
        assert_eq!(r.kind, k);
        let (ra, rb, up) = points(r);
        assert!(near3(ra, pa.position), "{k:?}: a は押した面の点");
        assert!(near3(rb, pb.position), "{k:?}: b は離した面の点");
        assert!(near3(up, pa.normal), "{k:?}: 向きは押した面の法線 {up}");
        assert_eq!(st(&h).doc.undo_count(), steps + 1, "{k:?}: 取り消し 1 回");
        let layer = st(&h).selected_layer.unwrap();
        assert_eq!(st(&h).rulers.selected, Some((layer, r.id)), "{k:?}");
        assert_eq!(r.snap, k != RulerKind::Line, "{k:?}: 特殊定規には印");
        // 取り消しで消える
        h.state_mut().state.apply(Action::Undo);
        assert!(rulers(&h).is_empty(), "{k:?}");
    }
}

#[test]
fn a_read_only_set_refuses_to_make_a_3d_ruler_and_changes_nothing() {
    let (mut h, rect) = cube_view();
    tool(&mut h, Tool::Ruler);
    kind(&mut h, RulerKind::Line);
    h.state_mut().state.sets.get_mut(0).unwrap().read_only = Some("テスト".into());
    let steps = st(&h).doc.undo_count();
    let (a, b) = front(&h, rect);
    pull(&mut h, a, b);
    assert!(rulers(&h).is_empty());
    assert_eq!(st(&h).doc.undo_count(), steps);
    assert!(st(&h).message.contains("読むだけ"), "{}", st(&h).message);
    assert!(!yolu_app::view3d::draft::dragging(st(&h)));
}

#[test]
fn a_perspective_ruler_follows_the_one_or_two_points_of_the_tool() {
    for two in [false, true] {
        let (mut h, rect) = cube_view();
        tool(&mut h, Tool::Ruler);
        kind(&mut h, RulerKind::Perspective);
        h.state_mut().state.rulers.two_points = two;
        let (a, b) = front(&h, rect);
        pull(&mut h, a, b);
        assert_eq!(rulers(&h)[0].two_points, two);
    }
}

#[test]
fn pressing_outside_the_model_or_pulling_nothing_makes_no_ruler_except_the_click_mirror() {
    for k in [
        RulerKind::Line,
        RulerKind::Parallel,
        RulerKind::Concentric,
        RulerKind::Perspective,
        RulerKind::Symmetry,
    ] {
        let (mut h, rect) = cube_view();
        tool(&mut h, Tool::Ruler);
        kind(&mut h, k);
        let (a, b) = front(&h, rect);
        // モデルの外で押す（表示域の隅）
        let outside = rect.min + egui::vec2(12.0, 12.0);
        assert!(hit_at(&h, rect, outside).is_none());
        let steps = st(&h).doc.undo_count();
        pull(&mut h, outside, b);
        assert!(rulers(&h).is_empty(), "{k:?}: 外で押したら作らない");
        assert_eq!(st(&h).doc.undo_count(), steps, "{k:?}");
        assert!(!yolu_app::view3d::draft::dragging(st(&h)), "{k:?}");
        // 面の上で押して、動かさずに離す: 対称だけは境界の中心の X の鏡
        click(&mut h, a);
        let made = rulers(&h);
        if k == RulerKind::Symmetry {
            assert_eq!(made.len(), 1, "{}", st(&h).message);
        } else {
            assert!(made.is_empty(), "{k:?}: 引かなければ作らない");
        }
    }
}

#[test]
fn a_symmetry_ruler_is_the_plane_of_the_pulled_line_and_the_sight_or_the_x_mirror_at_the_center() {
    let (mut h, rect) = cube_view();
    tool(&mut h, Tool::Ruler);
    kind(&mut h, RulerKind::Symmetry);
    let (a, b) = front(&h, rect);
    let pa = hit_at(&h, rect, a).unwrap();
    pull(&mut h, a, b);
    let r = rulers(&h).remove(0);
    let setup = r.surface_symmetry().expect("3D の写し");
    let mirror = setup.mirror.expect("線対称 2 本は鏡 1 枚");
    assert!(near3(points(&r).0, pa.position), "鏡は押した面の点を通る");
    assert!(mirror.point.distance(pa.position) < 1e-4);
    // 鏡の面は、画面で引いた線の向きと視線の両方を含む（法線は両方に直交）
    let view = st(&h).view3d.camera.view(rect.width(), rect.height());
    let sight = (pa.position - view.position).normalize();
    let drawn = (b - a).normalized();
    let line = view.rotation * Vec3::X * drawn.x - view.rotation * Vec3::Y * drawn.y;
    assert!(
        mirror.normal.dot(sight).abs() < 2e-2,
        "{}",
        mirror.normal.dot(sight)
    );
    assert!(
        mirror.normal.dot(line.normalize()).abs() < 2e-2,
        "{}",
        mirror.normal.dot(line.normalize())
    );
    // 押して離しただけ: モデルの境界の中心（立方体の中心）を通る X の鏡
    let (mut h, rect) = cube_view();
    tool(&mut h, Tool::Ruler);
    kind(&mut h, RulerKind::Symmetry);
    let (a, _) = front(&h, rect);
    click(&mut h, a);
    let r = rulers(&h).remove(0);
    let mirror = r.surface_symmetry().unwrap().mirror.unwrap();
    assert!(points(&r).0.length() < 1e-4, "中心は境界の中心（原点）");
    assert!(
        mirror.normal.dot(Vec3::X).abs() > 0.9999,
        "{}",
        mirror.normal
    );
    assert!(mirror.point.length() < 1e-4);
}

#[test]
fn shift_turns_the_mirror_normal_to_the_nearest_model_axis() {
    let (mut h, rect) = cube_view();
    tool(&mut h, Tool::Ruler);
    kind(&mut h, RulerKind::Symmetry);
    // 手前の面の上で、右上へ斜めに引く
    let (a, b) = front(&h, rect);
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    press_with(&h, a, Modifiers::SHIFT);
    h.step();
    move_to(&h, a + (b - a) * 0.5);
    h.step();
    move_to(&h, b);
    h.step();
    release_with(&h, b, Modifiers::SHIFT);
    h.step();
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run();
    let r = rulers(&h).remove(0);
    let mirror = r.surface_symmetry().unwrap().mirror.unwrap();
    let n = mirror.normal;
    let on_axis = [Vec3::X, Vec3::Y, Vec3::Z]
        .iter()
        .any(|axis| n.dot(*axis).abs() > 0.9999);
    assert!(on_axis, "鏡の法線はモデルの軸: {n}");
}

#[test]
fn with_on_layer_off_the_ruler_goes_to_a_new_ruler_layer_and_the_selection_stays() {
    let (mut h, rect) = cube_view();
    tool(&mut h, Tool::Ruler);
    kind(&mut h, RulerKind::Line);
    h.state_mut().state.rulers.on_layer = false;
    let layer = st(&h).selected_layer.unwrap();
    let steps = st(&h).doc.undo_count();
    let (a, b) = front(&h, rect);
    pull(&mut h, a, b);
    assert_eq!(st(&h).selected_layer, Some(layer));
    assert!(rulers(&h).is_empty(), "選んでいるレイヤーには付けない");
    assert_eq!(crate::common::rulers::total(st(&h)), 1);
    assert_eq!(st(&h).doc.undo_count(), steps + 1, "レイヤーと定規で 1 回");
}

/// 画面の点のうち、最後のストロークの点（凍結した寄せ先を通ったあと）。
fn last_point(h: &H, rect: Rect) -> DVec2 {
    let p = st(h).view3d.input.last_point.expect("点");
    DVec2::new((p.x - rect.left()) as f64, (p.y - rect.top()) as f64)
}

fn distance_to_line(a: DVec2, b: DVec2, p: DVec2) -> f64 {
    (b - a).perp_dot(p - a).abs() / (b - a).length()
}

/// 3D の定規を置き、ブラシで `from` から `moves` の順に動かしたストロークの点を返す（押した点が最初）。
fn stroke_points(h: &mut H, rect: Rect, ruler: Ruler, from: Pos2, moves: &[Pos2]) -> Vec<DVec2> {
    crate::common::rulers::add(&mut h.state_mut().state, ruler);
    tool(h, Tool::Brush);
    h.state_mut().state.rulers.snap_ruler = true;
    h.state_mut().state.rulers.snap_special = true;
    press(h, from, PointerButton::Primary);
    h.step();
    let mut out = vec![last_point(h, rect)];
    for m in moves {
        move_to(h, *m);
        h.step();
        out.push(last_point(h, rect));
    }
    release(h, *moves.last().unwrap(), PointerButton::Primary);
    h.run();
    out
}

fn screen_local(h: &H, rect: Rect, p: Vec3) -> DVec2 {
    let s = screen(h, rect, p);
    DVec2::new((s.x - rect.left()) as f64, (s.y - rect.top()) as f64)
}

#[test]
fn a_straight_3d_ruler_snaps_strokes_that_start_near_its_projected_line_only() {
    let (mut h, rect) = cube_view();
    let (a3, b3) = (Vec3::new(-0.4, -0.2, -0.5), Vec3::new(0.4, 0.2, -0.5));
    let line = Ruler::model(
        yolu_core::RulerId(1),
        RulerKind::Line,
        a3.as_dvec3(),
        b3.as_dvec3(),
        DVec3::NEG_Z,
    );
    let (sa, sb) = (screen_local(&h, rect, a3), screen_local(&h, rect, b3));
    // 線から画面で 8 点ほど離れた所から始める（近い）。動かした点は、線の上に乗る
    let normal = (sb - sa).perp().normalize();
    let mid = (sa + sb) * 0.5;
    let start = mid + normal * 8.0;
    let to = |p: DVec2| pos2(rect.left() + p.x as f32, rect.top() + p.y as f32);
    let pts = stroke_points(
        &mut h,
        rect,
        line.clone(),
        to(start),
        &[
            to(start + normal * 6.0 + (sb - sa).normalize() * 20.0),
            to(start - normal * 5.0 + (sb - sa).normalize() * 50.0),
        ],
    );
    for p in &pts {
        assert!(distance_to_line(sa, sb, *p) < 1e-2, "線の上: {p}");
    }
    // 遠い所（40 点）から始めると寄せない
    let (mut h, rect) = cube_view();
    let start = mid + normal * 40.0;
    let to = |p: DVec2| pos2(rect.left() + p.x as f32, rect.top() + p.y as f32);
    let moved = start + normal * 7.0 + (sb - sa).normalize() * 30.0;
    let pts = stroke_points(&mut h, rect, line, to(start), &[to(moved)]);
    assert!(
        pts[1].distance(moved) < 1e-2,
        "寄せない: {} {moved}",
        pts[1]
    );
}

/// 同心円・平行線・パースを寄せ先にして、押した面の点から動かしたストロークの点を返す。
fn special_stroke(h: &mut H, rect: Rect, ruler: Ruler) -> (Vec3, Pos2, Vec<DVec2>) {
    let start = screen(h, rect, Vec3::new(0.2, -0.1, -0.5));
    let hit = hit_at(h, rect, start).expect("面の上");
    let to = |p: DVec2| pos2(rect.left() + p.x as f32, rect.top() + p.y as f32);
    let origin = DVec2::new(
        (start.x - rect.left()) as f64,
        (start.y - rect.top()) as f64,
    );
    let moves: Vec<Pos2> = [(30.0, -9.0), (60.0, 14.0), (95.0, 3.0)]
        .iter()
        .map(|(x, y)| to(origin + DVec2::new(*x, *y)))
        .collect();
    let pts = stroke_points(h, rect, ruler, start, &moves);
    (hit.position, start, pts)
}

#[test]
fn a_parallel_3d_ruler_keeps_the_stroke_on_the_projected_direction_through_the_pressed_point() {
    let (mut h, rect) = cube_view();
    let d = DVec3::new(1.0, 0.3, 0.0);
    let r = Ruler::model(
        yolu_core::RulerId(1),
        RulerKind::Parallel,
        DVec3::new(0.0, 0.2, -0.5),
        DVec3::new(0.0, 0.2, -0.5) + d,
        DVec3::NEG_Z,
    );
    let (p, _, pts) = special_stroke(&mut h, rect, r);
    // 押した面の点から d の向きへ少し進んだ点を写した向きが、画面の線の向き
    let sp = screen_local(&h, rect, p);
    let sq = screen_local(&h, rect, p + d.as_vec3() * 0.1);
    for q in &pts {
        assert!(distance_to_line(sp, sq, *q) < 0.05, "平行線の向きの上: {q}");
    }
    assert!(pts.last().unwrap().distance(pts[0]) > 20.0, "動いた");
}

#[test]
fn a_perspective_3d_ruler_keeps_the_stroke_on_the_line_through_the_vanishing_point() {
    let (mut h, rect) = cube_view();
    let vanishing = DVec3::new(-0.2, 0.9, -0.5);
    let r = Ruler::model(
        yolu_core::RulerId(1),
        RulerKind::Perspective,
        vanishing,
        vanishing + DVec3::X,
        DVec3::NEG_Z,
    );
    let (p, _, pts) = special_stroke(&mut h, rect, r);
    let sp = screen_local(&h, rect, p);
    let sv = screen_local(&h, rect, vanishing.as_vec3());
    for q in &pts {
        assert!(distance_to_line(sp, sv, *q) < 0.05, "消失点への線の上: {q}");
    }
    assert!(pts.last().unwrap().distance(pts[0]) > 20.0, "動いた");
}

#[test]
fn a_concentric_3d_ruler_keeps_the_stroke_on_the_projected_circle_through_the_pressed_point() {
    let (mut h, rect) = cube_view();
    let center = DVec3::new(0.0, 0.0, -0.5);
    let r = Ruler::model(
        yolu_core::RulerId(1),
        RulerKind::Concentric,
        center,
        center + DVec3::X * 0.3,
        DVec3::NEG_Z,
    );
    let (p, _, pts) = special_stroke(&mut h, rect, r);
    // 押した面の点を通る、中心 center・軸 −Z の円（この面の上）を写した点の列に、ストロークの点が乗る
    let radius = (p - center.as_vec3()).length();
    let ring: Vec<DVec2> = (0..=256)
        .map(|k| {
            let t = k as f32 / 256.0 * std::f32::consts::TAU;
            screen_local(
                &h,
                rect,
                center.as_vec3() + Vec3::new(t.cos(), t.sin(), 0.0) * radius,
            )
        })
        .collect();
    for q in &pts {
        let nearest = ring
            .windows(2)
            .map(|w| {
                let ab = w[1] - w[0];
                let t = ((*q - w[0]).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
                (w[0] + ab * t).distance(*q)
            })
            .fold(f64::INFINITY, f64::min);
        assert!(nearest < 0.1, "円の上: {q} {nearest}");
    }
    assert!(
        pts.last().unwrap().distance(pts[0]) > 10.0,
        "円に沿って動いた"
    );
}

#[test]
fn special_3d_rulers_do_not_snap_when_the_press_is_off_the_model_but_a_straight_one_still_does() {
    let (mut h, rect) = cube_view();
    let outside = rect.min + egui::vec2(40.0, 40.0);
    assert!(hit_at(&h, rect, outside).is_none());
    crate::common::rulers::add(
        &mut h.state_mut().state,
        Ruler::model(
            yolu_core::RulerId(1),
            RulerKind::Parallel,
            DVec3::ZERO,
            DVec3::X,
            DVec3::Y,
        ),
    );
    tool(&mut h, Tool::Brush);
    press(&h, outside, PointerButton::Primary);
    h.step();
    assert!(st(&h).view3d.input.surface.is_some(), "{}", st(&h).message);
    assert!(
        st(&h).view3d.input.ruler_constraint.is_none(),
        "モデルの外で押すと、面の点が無いので平行線は寄せない"
    );
    release(&h, outside, PointerButton::Primary);
    h.run();
    // 同じ所の近く（画面で 10 点）を通る直線定規なら、面の点が無くても寄せる
    let to_world = |h: &H, p: Pos2| {
        let view = st(h).view3d.camera.view(rect.width(), rect.height());
        let r = view.ray(local(rect, p));
        r.origin() + r.direction() * 2.0
    };
    let a = to_world(&h, outside + egui::vec2(-50.0, 10.0));
    let b = to_world(&h, outside + egui::vec2(50.0, 10.0));
    crate::common::rulers::add(
        &mut h.state_mut().state,
        Ruler::model(
            yolu_core::RulerId(1),
            RulerKind::Line,
            a.as_dvec3(),
            b.as_dvec3(),
            DVec3::Y,
        ),
    );
    tool(&mut h, Tool::Brush);
    press(&h, outside, PointerButton::Primary);
    h.step();
    assert!(
        matches!(
            st(&h).view3d.input.ruler_constraint,
            Some(Constraint::Line { .. })
        ),
        "直線定規は面の点が無くても、近ければ寄せる"
    );
    release(&h, outside, PointerButton::Primary);
    h.run();
}

#[test]
fn a_saved_3d_ruler_comes_back_the_same_after_reopening() {
    let dir = crate::common::tmp::test_dir("view3d-rulers");
    let path = dir.join("rulers.ylp");
    let (mut h, rect) = cube_view();
    tool(&mut h, Tool::Ruler);
    for k in [RulerKind::Line, RulerKind::Symmetry] {
        kind(&mut h, k);
        let (a, b) = front(&h, rect);
        pull(&mut h, a, b);
    }
    let before = rulers(&h);
    assert_eq!(before.len(), 2);
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(path.clone()));
    h.state_mut().state.apply(Action::OpenProject(path));
    h.run();
    assert_eq!(rulers(&h), before, "{}", st(&h).message);
    let _ = std::fs::remove_dir_all(dir);
}

/// 印のある特殊定規の種類（空間ごと。2D か 3D）。
fn marked(h: &H, model: bool) -> Option<RulerKind> {
    let layer = st(h).selected_layer?;
    crate::common::rulers::of(st(h), layer)
        .into_iter()
        .find(|r| r.snap && matches!(r.place, RulerPlace::Model { .. }) == model)
        .map(|r| r.kind)
}

#[test]
fn ctrl_4_switches_the_special_ruler_of_the_view_under_the_pointer_else_the_last_drawn_view() {
    let mut h = app_default(1470.0, 760.0, 256);
    h.state_mut().apply(Action::LoadDemoModel);
    h.run();
    {
        let app = &mut h.state_mut().state;
        for kind in [RulerKind::Parallel, RulerKind::Concentric] {
            crate::common::rulers::add_canvas(
                app,
                kind,
                DVec2::new(10.0, 10.0),
                DVec2::new(40.0, 20.0),
            );
            crate::common::rulers::add(
                app,
                Ruler::model(yolu_core::RulerId(1), kind, DVec3::ZERO, DVec3::X, DVec3::Y),
            );
        }
    }
    h.run();
    assert!(
        st(&h).view3d.visible && st(&h).ui.canvas_visible,
        "2D も 3D も出ている"
    );
    // 最後に作った（同心円）に印。2D と 3D は別
    assert_eq!(marked(&h, false), Some(RulerKind::Concentric));
    assert_eq!(marked(&h, true), Some(RulerKind::Concentric));
    let view3d = h.state().view3d_rect().expect("3D");
    let canvas = canvas_rect(&h);
    // ポインタが 3D ビューの上: 3D の印だけが回る
    move_to(&h, view3d.center());
    h.run();
    key(&h, Key::Num4, Modifiers::COMMAND);
    h.run();
    assert_eq!(
        marked(&h, true),
        Some(RulerKind::Parallel),
        "3D の候補を回した"
    );
    assert_eq!(
        marked(&h, false),
        Some(RulerKind::Concentric),
        "2D は変わらない"
    );
    // ポインタがキャンバスの上: 2D の印だけが回る
    move_to(&h, canvas.center());
    h.run();
    key(&h, Key::Num4, Modifiers::COMMAND);
    h.run();
    assert_eq!(marked(&h, false), Some(RulerKind::Parallel));
    assert_eq!(
        marked(&h, true),
        Some(RulerKind::Parallel),
        "3D は変わらない"
    );
    // どちらにも乗っていない（ツールの帯の上）: 最後に描いたビューの空間
    let outside = pos2(20.0, canvas.center().y);
    assert!(!view3d.contains(outside) && !canvas.contains(outside));
    for (drew, model) in [
        (yolu_app::rulers::Place::View3d, true),
        (yolu_app::rulers::Place::Canvas, false),
    ] {
        h.state_mut().state.rulers.last_drew = Some(drew);
        move_to(&h, outside);
        h.run();
        let before = (marked(&h, true), marked(&h, false));
        key(&h, Key::Num4, Modifiers::COMMAND);
        h.run();
        let after = (marked(&h, true), marked(&h, false));
        assert_eq!(after.0 != before.0, model, "{drew:?}: 3D");
        assert_eq!(after.1 != before.1, !model, "{drew:?}: 2D");
    }
}

#[test]
fn ctrl_4_follows_the_view_where_a_gradient_or_a_shape_was_drawn_when_the_pointer_is_over_neither()
{
    let mut h = app_default(1470.0, 760.0, 256);
    h.state_mut().apply(Action::LoadDemoModel);
    h.run();
    {
        let app = &mut h.state_mut().state;
        for kind in [RulerKind::Parallel, RulerKind::Concentric] {
            crate::common::rulers::add_canvas(
                app,
                kind,
                DVec2::new(10.0, 10.0),
                DVec2::new(40.0, 20.0),
            );
            crate::common::rulers::add(
                app,
                Ruler::model(yolu_core::RulerId(1), kind, DVec3::ZERO, DVec3::X, DVec3::Y),
            );
        }
    }
    h.run();
    let view3d = h.state().view3d_rect().expect("3D");
    let canvas = canvas_rect(&h);
    let outside = pos2(20.0, canvas.center().y);
    assert!(!view3d.contains(outside) && !canvas.contains(outside));
    assert_eq!(st(&h).rulers.last_drew, None, "まだ何も描いていない");
    let ctrl_4 = |h: &mut H| {
        move_to(h, outside);
        h.run();
        key(h, Key::Num4, Modifiers::COMMAND);
        h.run();
    };
    // 3D ビューでグラデーションを引く（取り消し 1 回）→ 3D の印だけが回る
    tool(&mut h, Tool::Gradient);
    let steps = st(&h).doc.undo_count();
    let c = view3d.center();
    drag(
        &mut h,
        &[c + egui::vec2(-40.0, -20.0), c, c + egui::vec2(40.0, 30.0)],
    );
    assert!(
        st(&h).doc.undo_count() > steps,
        "3D のグラデーションを引いた {}",
        st(&h).message
    );
    assert_eq!(
        st(&h).rulers.last_drew,
        Some(yolu_app::rulers::Place::View3d)
    );
    let before = (marked(&h, true), marked(&h, false));
    ctrl_4(&mut h);
    assert_ne!(marked(&h, true), before.0, "3D の印が回った");
    assert_eq!(marked(&h, false), before.1, "2D は変わらない");
    // 2D のキャンバスで図形を引く → 2D の印だけが回る
    tool(&mut h, Tool::Shape);
    let steps = st(&h).doc.undo_count();
    let c = canvas.center();
    drag(
        &mut h,
        &[c + egui::vec2(-40.0, -20.0), c, c + egui::vec2(40.0, 30.0)],
    );
    assert!(
        st(&h).doc.undo_count() > steps,
        "2D の図形を引いた {}",
        st(&h).message
    );
    assert_eq!(
        st(&h).rulers.last_drew,
        Some(yolu_app::rulers::Place::Canvas)
    );
    let before = (marked(&h, true), marked(&h, false));
    ctrl_4(&mut h);
    assert_ne!(marked(&h, false), before.1, "2D の印が回った");
    assert_eq!(marked(&h, true), before.0, "3D は変わらない");
    // 3D ビューで定規を作る → また 3D
    tool(&mut h, Tool::Ruler);
    kind(&mut h, RulerKind::Line);
    let c = view3d.center();
    drag(
        &mut h,
        &[c + egui::vec2(-40.0, -20.0), c, c + egui::vec2(40.0, 30.0)],
    );
    assert_eq!(
        st(&h).rulers.last_drew,
        Some(yolu_app::rulers::Place::View3d)
    );
}

#[test]
fn shown_3d_rulers_are_drawn_and_a_hidden_one_is_not() {
    // 絵ではなく、見せる定規の集まりが 3D のものだけで、目を閉じると消えること
    let (mut h, rect) = cube_view();
    tool(&mut h, Tool::Ruler);
    kind(&mut h, RulerKind::Line);
    let (a, b) = front(&h, rect);
    pull(&mut h, a, b);
    kind(&mut h, RulerKind::Symmetry);
    pull(&mut h, a, b);
    crate::common::rulers::line(&mut h.state_mut().state, (0.0, 3.0), (60.0, 3.0));
    let shown = st(&h).rulers_shown(yolu_app::rulers::Place::View3d);
    assert_eq!(shown.len(), 2, "3D の定規だけ（2D の直線定規は出さない）");
    assert_eq!(
        shown.iter().filter(|s| s.effective).count(),
        2,
        "直線は「定規にスナップ」、対称は印のある 1 つ"
    );
    let layer = st(&h).selected_layer.unwrap();
    h.state_mut()
        .state
        .doc
        .set_layer_visible(layer, false)
        .unwrap();
    assert!(st(&h)
        .rulers_shown(yolu_app::rulers::Place::View3d)
        .is_empty());
}

fn two_materials() -> (H, Rect) {
    use yolu_app::view3d::model::ViewModel;
    use yolu_core::geometry::{ModelMesh, OrbitCamera, Submesh};
    let mut h = app(1100.0, 760.0, 64);
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    let plate = |cx: f32, material: i32| {
        let p = |x: f32, y: f32| Vec3::new(cx + x, y, 0.0);
        ModelMesh {
            name: format!("板{material}"),
            positions: vec![p(-0.9, -0.9), p(0.9, -0.9), p(-0.9, 0.9), p(0.9, 0.9)],
            normals: Vec::new(),
            uvs: vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(1.0, 0.0),
                Vec2::new(0.0, 1.0),
                Vec2::new(1.0, 1.0),
            ],
            submeshes: vec![Submesh {
                material,
                indices: vec![0, 2, 1, 2, 3, 1],
            }],
        }
    };
    {
        let state = &mut h.state_mut().state;
        let revision = state.view3d.next_revision();
        let model = ViewModel::new(
            "二枚",
            vec![plate(-1.1, 0), plate(1.1, 1)],
            vec![Some("a".to_string()), Some("b".to_string())],
            revision,
        )
        .unwrap();
        state.view3d.set_model(model);
        state.view3d.material = 0;
        state.view3d.camera = OrbitCamera {
            target: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            distance: 7.0,
            model_radius: 1.0,
            ..OrbitCamera::default()
        };
        state.sync_view3d();
    }
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブを描いた");
    (h, rect)
}

#[test]
fn a_ruler_is_not_made_from_a_face_of_another_texture_set_but_is_from_the_current_one() {
    let (mut h, rect) = two_materials();
    tool(&mut h, Tool::Ruler);
    kind(&mut h, RulerKind::Line);
    let on = |h: &H, x: f32, y: f32| screen(h, rect, Vec3::new(x, y, 0.0));
    // 右の板（マテリアル 1。セットに付いていない）: 作らず、ブラシと同じ知らせ
    let steps = st(&h).doc.undo_count();
    let (a, b) = (on(&h, 0.6, -0.3), on(&h, 1.6, 0.4));
    pull(&mut h, a, b);
    assert!(rulers(&h).is_empty());
    assert_eq!(st(&h).doc.undo_count(), steps);
    assert!(
        st(&h)
            .message
            .contains("ほかのテクスチャセット（b）の面です。"),
        "{}",
        st(&h).message
    );
    assert!(!yolu_app::view3d::draft::dragging(st(&h)));
    // 左の板（今のセット）は作る
    let (a, b) = (on(&h, -1.6, -0.3), on(&h, -0.6, 0.4));
    pull(&mut h, a, b);
    assert_eq!(rulers(&h).len(), 1, "{}", st(&h).message);
}

#[test]
fn a_symmetry_ruler_can_be_a_rotation_symmetry_around_the_pressed_face_normal_axis() {
    let (mut h, rect) = cube_view();
    tool(&mut h, Tool::Ruler);
    kind(&mut h, RulerKind::Symmetry);
    {
        let s = &mut h.state_mut().state.rulers;
        s.set_line_symmetry(false);
        s.set_lines(6);
    }
    let (a, b) = front(&h, rect);
    pull(&mut h, a, b);
    let r = rulers(&h).remove(0);
    assert!(!r.line_symmetry);
    assert_eq!(r.lines, 6);
    assert!(r.validate().is_ok());
    let setup = r.surface_symmetry().expect("3D の写し");
    assert!(setup.mirror.is_none(), "回転対称は鏡を使わない");
    assert_eq!(setup.radial.expect("回転").count, 6);
    // 引かずに離しても、回転対称（境界の中心を通る軸のまわり）
    click(&mut h, a);
    let made = rulers(&h);
    assert_eq!(made.len(), 2, "{}", st(&h).message);
    assert!(!made[1].line_symmetry && made[1].lines == 6);
}

#[test]
fn escape_and_losing_the_focus_while_pulling_a_ruler_drop_it_without_making_one() {
    for way in 0..2 {
        let (mut h, rect) = cube_view();
        tool(&mut h, Tool::Ruler);
        kind(&mut h, RulerKind::Line);
        let (a, b) = front(&h, rect);
        let steps = st(&h).doc.undo_count();
        press(&h, a, PointerButton::Primary);
        h.step();
        move_to(&h, b);
        h.step();
        assert!(
            yolu_app::view3d::draft::dragging(st(&h)),
            "{way}: 引いている"
        );
        if way == 0 {
            h.event(Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            });
        } else {
            h.event(Event::WindowFocused(false));
        }
        h.step();
        assert!(!yolu_app::view3d::draft::dragging(st(&h)), "{way}: 捨てた");
        release(&h, b, PointerButton::Primary);
        h.run();
        assert!(rulers(&h).is_empty(), "{way}: 離しても作らない");
        assert_eq!(st(&h).doc.undo_count(), steps, "{way}");
    }
}

/// 「定規にスナップ」「特殊定規にスナップ」を切ると、3D のストロークは寄らない（入れておけば寄る）。
#[test]
fn turning_off_a_snap_stops_the_3d_pulls_of_its_rulers() {
    let (mut h, rect) = cube_view();
    let line = Ruler::model(
        yolu_core::RulerId(1),
        RulerKind::Line,
        DVec3::new(-0.4, -0.2, -0.5),
        DVec3::new(0.4, 0.2, -0.5),
        DVec3::NEG_Z,
    );
    let parallel = Ruler::model(
        yolu_core::RulerId(1),
        RulerKind::Parallel,
        DVec3::new(0.0, 0.2, -0.5),
        DVec3::new(1.0, 0.5, -0.5),
        DVec3::NEG_Z,
    );
    crate::common::rulers::add(&mut h.state_mut().state, line);
    crate::common::rulers::add(&mut h.state_mut().state, parallel);
    tool(&mut h, Tool::Brush);
    // 直線の近く（画面で 8 点）から押す
    let (sa, sb) = (
        screen_local(&h, rect, Vec3::new(-0.4, -0.2, -0.5)),
        screen_local(&h, rect, Vec3::new(0.4, 0.2, -0.5)),
    );
    let normal = (sb - sa).perp().normalize();
    let near = (sa + sb) * 0.5 + normal * 8.0;
    let to = |p: DVec2| pos2(rect.left() + p.x as f32, rect.top() + p.y as f32);
    // 直線から遠いが、面の上（平行線は押した面の点から寄せる）
    let far = to(screen_local(&h, rect, Vec3::new(0.2, -0.4, -0.5)));
    let pressed = |h: &mut H, at: Pos2| {
        press(h, at, PointerButton::Primary);
        h.step();
        let c = st(h).view3d.input.ruler_constraint.clone();
        release(h, at, PointerButton::Primary);
        h.run();
        c
    };
    for (snap_ruler, snap_special) in [(true, true), (false, true), (true, false), (false, false)] {
        {
            let rulers = &mut h.state_mut().state.rulers;
            rulers.snap_ruler = snap_ruler;
            rulers.snap_special = snap_special;
        }
        let near_line = pressed(&mut h, to(near));
        assert_eq!(
            near_line.is_some(),
            snap_ruler || snap_special,
            "直線の近く ({snap_ruler}, {snap_special}): {near_line:?}"
        );
        if snap_ruler {
            assert!(
                matches!(near_line, Some(Constraint::Line { origin, .. }) if origin.distance(sa) < 1e-3 || distance_to_line(sa, sb, origin) < 1e-3)
            );
        }
        let far_press = pressed(&mut h, far);
        assert_eq!(
            far_press.is_some(),
            snap_special,
            "遠く ({snap_ruler}, {snap_special})"
        );
    }
}

/// 片端がカメラの後ろにある直線定規でも、画面に見えている所の近くで描き始めれば寄る。
#[test]
fn a_straight_3d_ruler_with_an_end_behind_the_camera_still_snaps_near_its_visible_part() {
    let (mut h, rect) = cube_view();
    let view = st(&h).view3d.camera.view(rect.width(), rect.height());
    let right = view.rotation * Vec3::X;
    let behind = view.position - view.forward * 2.0 + right * 0.4;
    let front = st(&h).view3d.camera.target + right * 0.1;
    assert!(view.depth(behind) < 0.0 && view.depth(front) > 0.0);
    let line = Ruler::model(
        yolu_core::RulerId(1),
        RulerKind::Line,
        behind.as_dvec3(),
        front.as_dvec3(),
        DVec3::Y,
    );
    // 手前に写る部分（深さが十分ある所）の 2 点から、画面の線を決める
    let at = |t: f32| screen_local(&h, rect, behind.lerp(front, t));
    let (s1, s2) = (at(0.7), at(1.0));
    assert!(s1.distance(s2) > 20.0, "画面で線が見える {s1} {s2}");
    let normal = (s2 - s1).perp().normalize();
    let start = (s1 + s2) * 0.5 + normal * 8.0;
    let to = |p: DVec2| pos2(rect.left() + p.x as f32, rect.top() + p.y as f32);
    let along = (s2 - s1).normalize();
    let pts = stroke_points(
        &mut h,
        rect,
        line,
        to(start),
        &[
            to(start + normal * 5.0 + along * 12.0),
            to(start - normal * 6.0 + along * 25.0),
        ],
    );
    for p in &pts {
        assert!(distance_to_line(s1, s2, *p) < 0.05, "写した線の上: {p}");
    }
    assert!(pts[2].distance(pts[0]) > 15.0, "動いた");
}

fn orthographic() -> (H, Rect) {
    let (mut h, rect) = cube_view();
    h.state_mut().state.view3d.camera.set_orthographic(true);
    h.run();
    assert!(st(&h).view3d.camera.is_orthographic());
    (h, rect)
}

#[test]
fn in_orthographic_a_3d_ruler_is_made_on_the_faces_pulled_over() {
    let (mut h, rect) = orthographic();
    tool(&mut h, Tool::Ruler);
    for k in [RulerKind::Line, RulerKind::Symmetry] {
        kind(&mut h, k);
        let (a, b) = front(&h, rect);
        let (pa, pb) = (hit_at(&h, rect, a).unwrap(), hit_at(&h, rect, b).unwrap());
        let before = rulers(&h).len();
        pull(&mut h, a, b);
        let made = rulers(&h);
        assert_eq!(made.len(), before + 1, "{k:?} {}", st(&h).message);
        let r = made.last().unwrap();
        let (ra, rb, _) = points(r);
        assert!(near3(ra, pa.position), "{k:?}: a は押した面の点");
        if k == RulerKind::Line {
            assert!(near3(rb, pb.position), "b は離した面の点");
        } else {
            // 鏡の面は引いた線と視線（正投影は前の向き）を含む
            let mirror = r.surface_symmetry().unwrap().mirror.unwrap();
            let forward = st(&h)
                .view3d
                .camera
                .view(rect.width(), rect.height())
                .forward;
            assert!(
                mirror.normal.dot(forward).abs() < 2e-2,
                "{}",
                mirror.normal.dot(forward)
            );
        }
    }
}

#[test]
fn in_orthographic_strokes_are_pulled_onto_the_projected_lines() {
    let (mut h, rect) = orthographic();
    let (a3, b3) = (Vec3::new(-0.4, -0.2, -0.5), Vec3::new(0.4, 0.2, -0.5));
    let line = Ruler::model(
        yolu_core::RulerId(1),
        RulerKind::Line,
        a3.as_dvec3(),
        b3.as_dvec3(),
        DVec3::NEG_Z,
    );
    let (sa, sb) = (screen_local(&h, rect, a3), screen_local(&h, rect, b3));
    let normal = (sb - sa).perp().normalize();
    let start = (sa + sb) * 0.5 + normal * 8.0;
    let to = |p: DVec2| pos2(rect.left() + p.x as f32, rect.top() + p.y as f32);
    let along = (sb - sa).normalize();
    let pts = stroke_points(
        &mut h,
        rect,
        line,
        to(start),
        &[
            to(start + normal * 6.0 + along * 20.0),
            to(start - normal * 4.0 + along * 45.0),
        ],
    );
    for p in &pts {
        assert!(distance_to_line(sa, sb, *p) < 1e-2, "線の上: {p}");
    }
    // 平行線: 押した面の点を通る、a→b の向きを写した向きの線の上
    let (mut h, rect) = orthographic();
    let d = DVec3::new(1.0, 0.3, 0.0);
    let parallel = Ruler::model(
        yolu_core::RulerId(1),
        RulerKind::Parallel,
        DVec3::new(0.0, 0.2, -0.5),
        DVec3::new(0.0, 0.2, -0.5) + d,
        DVec3::NEG_Z,
    );
    let (p, _, pts) = special_stroke(&mut h, rect, parallel);
    let sp = screen_local(&h, rect, p);
    let sq = screen_local(&h, rect, p + d.as_vec3() * 0.1);
    for q in &pts {
        assert!(distance_to_line(sp, sq, *q) < 0.05, "平行線の向きの上: {q}");
    }
    assert!(pts.last().unwrap().distance(pts[0]) > 20.0, "動いた");
}

#[test]
fn snapshots_of_the_3d_mirror_and_the_straight_ruler_in_both_languages() {
    let mut snapshots = egui_kittest::SnapshotResults::new();
    for lang in Lang::ALL {
        for what in ["symmetry", "line", "dim"] {
            let (mut h, rect) = cube_view();
            h.state_mut().state.lang = lang;
            tool(&mut h, Tool::Ruler);
            let (a, b) = front(&h, rect);
            if what == "symmetry" {
                kind(&mut h, RulerKind::Symmetry);
                click(&mut h, a);
            } else if what == "dim" {
                // 同心円（印を外された特殊定規＝薄い）・直線定規・対称（印のある特殊定規＝濃い）
                kind(&mut h, RulerKind::Concentric);
                let (c0, c1) = (
                    screen(&h, rect, Vec3::new(0.0, 0.05, -0.5)),
                    screen(&h, rect, Vec3::new(0.22, 0.05, -0.5)),
                );
                pull(&mut h, c0, c1);
                kind(&mut h, RulerKind::Line);
                let (l0, l1) = (
                    screen(&h, rect, Vec3::new(-0.4, -0.38, -0.5)),
                    screen(&h, rect, Vec3::new(0.4, -0.38, -0.5)),
                );
                pull(&mut h, l0, l1);
                kind(&mut h, RulerKind::Symmetry);
                click(&mut h, a);
            } else {
                kind(&mut h, RulerKind::Line);
                pull(&mut h, a, b);
            }
            h.state_mut().state.rulers.selected = None;
            h.state_mut().state.message.clear();
            h.event(Event::PointerGone);
            h.run();
            h.snapshot(format!("view3d_ruler_{what}_{}", lang.pick("ja", "en")));
            snapshots.extend_harness(&mut h);
        }
    }
}
