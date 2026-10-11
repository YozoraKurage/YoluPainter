//! 既定の並び（3D ビューが左・キャンバスが右）での操作: それぞれの側に描ける、ポインタの下のビューにキー（Space のパン）が届く、スポイトがそれぞれの側で色を取る。
//! 試験の標準のウィンドウ（`app`）は中央を 1 つの組にしているので、左右に並んだ既定そのものでの入力の道は、ここで見る。
use crate::common;

use common::*;
use egui::{Event, Key, Modifiers, Pos2, Rect};
use egui_kittest::Harness;
use yolu_app::state::{Action, Tool};
use yolu_app::YoluApp;
use yolu_core::glam::Vec3;

type H = Harness<'static, YoluApp>;

/// 手前の面（−Z）の上の線（左 → 右）の両端と真ん中（3D ビューは細長いので、立方体は小さく写る。端がビューの外に出ない短さ）。
const A: Vec3 = Vec3::new(-0.12, 0.0, -0.5);
const M: Vec3 = Vec3::new(0.0, 0.0, -0.5);
const B: Vec3 = Vec3::new(0.12, 0.0, -0.5);

/// 既定の並びのウィンドウに、試しの立方体を読む。3D ビューの矩形を返す。
fn default_with_cube() -> (H, Rect) {
    let mut h = app_default(1280.0, 800.0, 256);
    {
        let s = &mut h.state_mut().state;
        s.view3d.load_demo();
        s.view3d.camera.yaw = -40.0;
        s.view3d.camera.pitch = 15.0;
        s.brush.radius = 14.0;
    }
    h.run();
    let rect = h.state().view3d_rect().expect("3D ビューを描いた");
    assert!(
        rect.right() <= canvas_rect(&h).left() + 2.0,
        "3D ビューが左・キャンバスが右 {rect:?} {:?}",
        canvas_rect(&h)
    );
    (h, rect)
}

fn screen_of(h: &H, rect: Rect, p: Vec3) -> Pos2 {
    let view = h
        .state()
        .state
        .view3d
        .camera
        .view(rect.width(), rect.height());
    let s = view.to_screen(p).expect("カメラの前");
    egui::pos2(rect.left() + s.x, rect.top() + s.y)
}

/// 文書の合成（描いたかどうかの比べ）。
fn composite(h: &H) -> Vec<u8> {
    let doc = &h.state().state.doc;
    doc.composite(doc.bounds()).unwrap()
}

/// 2 点の間を n 等分した点の並び（3D の線は、細かく動かして引く）。
fn line(a: Pos2, b: Pos2, n: usize) -> Vec<Pos2> {
    (0..=n)
        .map(|i| a + (b - a) * (i as f32 / n as f32))
        .collect()
}

fn key_event(h: &H, key: Key, pressed: bool) {
    h.event(Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
}

fn main_rgb(h: &H) -> [u8; 3] {
    let main = h.state().state.color.main;
    [0, 1, 2].map(|i| (main[i] * 255.0).round() as u8)
}

/// 左の 3D ビューにもキャンバスにも、ブラシで描ける（描いた側の文書の合成だけが変わる。3D は立方体の面、2D はキャンバスの真ん中）。
#[test]
fn each_side_of_the_default_arrangement_paints() {
    // キャンバス（右）
    let (mut h, _) = default_with_cube();
    let before = composite(&h);
    let c = canvas_rect(&h).center();
    drag(&mut h, &[offset(c, -60.0, 0.0), c, offset(c, 60.0, 10.0)]);
    assert!(composite(&h) != before, "キャンバスに描けた");
    assert!(h.state().state.doc.can_undo());
    // 3D ビュー（左）
    let (mut h, rect) = default_with_cube();
    let before = composite(&h);
    let (a, b) = (screen_of(&h, rect, A), screen_of(&h, rect, B));
    drag(&mut h, &line(a, b, 8));
    assert!(
        composite(&h) != before,
        "3D ビューに描けた: {}",
        h.state().state.message
    );
    assert!(h.state().state.doc.can_undo());
}

/// Space を押しながらのドラッグ（パン）は、ポインタの下のビューだけを動かす。キャンバスの上ではキャンバスの表示、3D ビューの上では視点。
#[test]
fn the_space_key_reaches_the_view_under_the_pointer_on_each_side() {
    let (mut h, rect) = default_with_cube();
    let canvas = canvas_rect(&h);
    let (pan0, camera0) = (h.state().state.view.pan, h.state().state.view3d.camera);
    // キャンバスの上
    key_event(&h, Key::Space, true);
    let c = canvas.center();
    drag(&mut h, &[c, offset(c, 30.0, 0.0), offset(c, 60.0, 20.0)]);
    key_event(&h, Key::Space, false);
    h.run();
    let pan = h.state().state.view.pan;
    assert_ne!(pan, pan0, "キャンバスの表示が動いた");
    assert_eq!(h.state().state.view3d.camera, camera0, "視点は動かない");
    // 3D ビューの上
    key_event(&h, Key::Space, true);
    let v = rect.center();
    drag(&mut h, &[v, offset(v, 30.0, 0.0), offset(v, 60.0, 20.0)]);
    key_event(&h, Key::Space, false);
    h.run();
    assert_ne!(h.state().state.view3d.camera, camera0, "視点が動いた");
    assert_eq!(h.state().state.view.pan, pan, "キャンバスの表示は動かない");
}

/// スポイトは、キャンバス（右）でも 3D ビュー（左）でも、描いた色を取る。
#[test]
fn the_eyedropper_picks_on_each_side_of_the_default_arrangement() {
    for side in ["キャンバス", "3D ビュー"] {
        let (mut h, rect) = default_with_cube();
        h.state_mut().state.color.set_main([0.0, 0.0, 1.0, 1.0]);
        let at = if side == "キャンバス" {
            let c = canvas_rect(&h).center();
            drag(&mut h, &[offset(c, -40.0, 0.0), c, offset(c, 40.0, 0.0)]);
            c
        } else {
            let (a, b) = (screen_of(&h, rect, A), screen_of(&h, rect, B));
            drag(&mut h, &line(a, b, 8));
            screen_of(&h, rect, M)
        };
        h.state_mut().state.color.set_main([0.0, 0.0, 0.0, 1.0]);
        h.state_mut()
            .state
            .apply(Action::SelectTool(Tool::Eyedropper));
        h.run();
        click(&mut h, at);
        assert_eq!(
            main_rgb(&h),
            [0, 0, 255],
            "{side}: {}",
            h.state().state.message
        );
    }
}
