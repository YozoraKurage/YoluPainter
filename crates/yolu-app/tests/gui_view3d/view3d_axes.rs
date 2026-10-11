//! 3D ビューの軸の印（右上のアイコンの下）と正投影: 丸を押すと軸の視点（軸の向きで正投影の設定なら正投影）、真ん中の丸で透視と
//! 正投影の切り替え、印の上のドラッグで回る、設定のパネルが開いている間は出ない。正投影の 3D の絵（立方体）と、印の絵（日英）。
use crate::common::{self, click_tab};
use egui::{pos2, vec2, Event, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::{kittest::Queryable, Harness};
use yolu_app::lang::Lang;
use yolu_app::view3d::axis_gizmo::{self, Part};
use yolu_app::YoluApp;
use yolu_core::geometry::{AxisView, OrbitCamera, DEFAULT_YAW};

type H = Harness<'static, YoluApp>;

fn view(lang: Lang) -> H {
    let mut h = common::app(1100.0, 760.0, 64);
    h.state_mut().state.lang = lang;
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.state_mut().state.view3d.load_demo();
    h.event(Event::PointerMoved(pos2(1.0, 1.0)));
    h.run();
    h
}

fn camera(h: &H) -> OrbitCamera {
    h.state().state.view3d.camera
}

/// 軸の印の矩形（読み上げの名前で探す）。
fn gizmo(h: &H, lang: Lang) -> Rect {
    h.get_by_label(lang.pick("視点の軸", "View axes")).rect()
}

/// 今の向きの、その丸の画面の位置。
fn ball(h: &H, rect: Rect, view: AxisView) -> Pos2 {
    axis_gizmo::balls(&camera(h), rect.center())
        .into_iter()
        .find(|b| b.view == view)
        .unwrap()
        .at
}

fn click(h: &mut H, at: Pos2) {
    common::click(h, at);
    h.run();
}

#[test]
fn the_axis_widget_sits_under_the_corner_icons_on_the_right() {
    let h = view(Lang::Ja);
    let r = gizmo(&h, Lang::Ja);
    let content = h.state().view3d_rect().unwrap();
    let frame = h.get_by_label("モデル全体が見える位置へ戻す").rect();
    assert!(r.top() > frame.bottom(), "{r:?} {frame:?}");
    assert!(
        (content.right() - r.right() - 8.0).abs() < 1.0,
        "{r:?} {content:?}"
    );
    assert!(content.contains_rect(r));
}

#[test]
fn pressing_a_ball_looks_from_that_axis_and_the_middle_switches_the_projection() {
    let mut h = view(Lang::Ja);
    let r = gizmo(&h, Lang::Ja);
    // 上の丸（+Y）: 上の視点、軸の向きなので正投影（自動）
    let top = ball(&h, r, AxisView::Top);
    assert_eq!(
        axis_gizmo::hit(&camera(&h), r, top),
        Some(Part::Axis(AxisView::Top))
    );
    click(&mut h, top);
    let c = camera(&h);
    assert_eq!(c.pitch, 90.0);
    assert!(c.is_orthographic());
    assert!(h.state().state.view3d.auto_orthographic);
    // 真ん中: 手で透視へ（自動の印は外れる）、もう一度で正投影
    click(&mut h, r.center());
    assert!(!camera(&h).is_orthographic());
    click(&mut h, r.center());
    assert!(camera(&h).is_orthographic());
    assert!(!h.state().state.view3d.auto_orthographic);
    // 反対の側の小さい丸: 右の視点からは −Z の丸が横に見える。押すと背面の視点
    let at = ball(&h, r, AxisView::Right);
    click(&mut h, at);
    let (yaw, pitch) = AxisView::Right.orientation();
    let c = camera(&h);
    assert_eq!(c.pitch, pitch);
    assert!(((c.yaw - yaw) / 360.0 - ((c.yaw - yaw) / 360.0).round()).abs() < 1e-6);
    let back = ball(&h, r, AxisView::Back);
    click(&mut h, back);
    assert_eq!(
        yolu_core::geometry::aligned_axis_view(camera(&h).yaw, camera(&h).pitch),
        Some(AxisView::Back)
    );
    // 押しは 3D ビューの描く入力に渡らない（塗っていない・取り消しに積まない）
    assert!(!h.state().state.doc.can_undo());
    assert!(h.state().state.view3d.input.stroke.is_none());
}

#[test]
fn dragging_the_widget_orbits_and_leaves_the_automatic_orthographic() {
    let mut h = view(Lang::Ja);
    let r = gizmo(&h, Lang::Ja);
    let at = ball(&h, r, AxisView::Front);
    click(&mut h, at);
    assert!(camera(&h).is_orthographic());
    let before = camera(&h);
    // 丸の無い所から右へドラッグ（右ドラッグの回転と同じ量）
    let from = r.center() + vec2(-REACH_OFF, REACH_OFF);
    h.event(Event::PointerMoved(from));
    h.event(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    for dx in [10.0, 20.0, 30.0] {
        h.event(Event::PointerMoved(from + vec2(dx, 0.0)));
        h.step();
    }
    h.event(Event::PointerButton {
        pos: from + vec2(30.0, 0.0),
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run();
    let c = camera(&h);
    assert!(c.yaw > before.yaw + 5.0, "{} → {}", before.yaw, c.yaw);
    assert_eq!(c.pitch, before.pitch);
    assert!(!c.is_orthographic(), "軸から外れたら透視");
    assert!(!h.state().state.doc.can_undo());
}

/// 丸の無い所（中心からの斜めのずれ。丸は中心から 30 の所）。
const REACH_OFF: f32 = 12.0;

#[test]
fn the_widget_hides_while_the_settings_panel_is_open() {
    let mut h = view(Lang::Ja);
    assert!(h.query_by_label("視点の軸").is_some());
    h.get_by_label("光・環境・トーンマッピング").click();
    h.run();
    assert!(h.state().state.view3d.display.settings_open);
    assert!(h.query_by_label("視点の軸").is_none());
    h.get_by_label("光・環境・トーンマッピング").click();
    h.run();
    assert!(h.query_by_label("視点の軸").is_some());
}

#[test]
fn the_axis_setting_turns_the_automatic_orthographic_off() {
    let mut h = view(Lang::Ja);
    h.state_mut().state.prefs.settings.navigation.axis_ortho = false;
    let r = gizmo(&h, Lang::Ja);
    let at = ball(&h, r, AxisView::Top);
    click(&mut h, at);
    assert_eq!(camera(&h).pitch, 90.0);
    assert!(!camera(&h).is_orthographic());
    // 真ん中の丸は設定によらず切り替える
    click(&mut h, r.center());
    assert!(camera(&h).is_orthographic());
}

/// 3D ビューの右上の隅（アイコンと軸の印）の絵。
fn shot(h: &mut H, name: &str) {
    let content = h.state().view3d_rect().unwrap();
    h.event(Event::PointerGone);
    h.run();
    let image = h.render().expect("描画");
    let r = Rect::from_min_max(
        pos2(content.right() - 140.0, content.top()),
        pos2(content.right(), content.top() + 140.0),
    );
    let cropped = image::imageops::crop_imm(
        &image,
        r.left().floor() as u32,
        r.top().floor() as u32,
        r.width().ceil() as u32,
        r.height().ceil() as u32,
    )
    .to_image();
    egui_kittest::image_snapshot(&cropped, name);
}

#[test]
fn snapshot_the_axis_widget_in_both_languages_and_both_projections() {
    for lang in Lang::ALL {
        let mut h = view(lang);
        assert_eq!(camera(&h).yaw, DEFAULT_YAW);
        shot(&mut h, lang.pick("view3d_axes_ja", "view3d_axes_en"));
        // 正面の視点（正投影。+Z の丸が真ん中に重なり、真ん中の丸は明るく塗られる）
        let r = gizmo(&h, lang);
        let at = ball(&h, r, AxisView::Front);
        click(&mut h, at);
        assert!(camera(&h).is_orthographic());
        shot(
            &mut h,
            lang.pick("view3d_axes_front_ortho_ja", "view3d_axes_front_ortho_en"),
        );
        // ツールチップの名前は視点のパイと同じ
        assert_eq!(
            axis_gizmo::name(Part::Axis(AxisView::Top), lang),
            lang.pick("上の視点", "Top View")
        );
        assert_eq!(
            axis_gizmo::name(Part::Center, lang),
            lang.pick("正投影の切り替え", "Toggle Orthographic")
        );
    }
}

#[test]
fn snapshot_the_cube_in_orthographic_from_above() {
    let mut h = view(Lang::Ja);
    let r = gizmo(&h, Lang::Ja);
    // 斜め上から: 正投影では、奥の辺も手前の辺も同じ長さで、平行な辺は平行に見える
    h.state_mut().state.view3d.camera.yaw = DEFAULT_YAW;
    h.state_mut().state.view3d.camera.pitch = 30.0;
    click(&mut h, r.center());
    assert!(camera(&h).is_orthographic());
    let content = h.state().view3d_rect().unwrap();
    let c = camera(&h);
    let v = c.view(content.width(), content.height());
    // 立方体の上の面の 4 辺: 向かい合う辺は画面でも同じ長さ
    let corner = |x: f32, z: f32| v.to_screen(yolu_core::glam::Vec3::new(x, 0.5, z)).unwrap();
    let (a, b, cc, d) = (
        corner(-0.5, -0.5),
        corner(0.5, -0.5),
        corner(0.5, 0.5),
        corner(-0.5, 0.5),
    );
    assert!(((b - a).length() - (cc - d).length()).abs() < 1e-3);
    assert!(((d - a).length() - (cc - b).length()).abs() < 1e-3);
    h.event(Event::PointerGone);
    h.run();
    h.snapshot("view3d_cube_orthographic");
}
