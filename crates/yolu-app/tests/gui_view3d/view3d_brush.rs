//! 3D ビューの効果ブラシ（ぼかし・指先・クローン）と 3D の対称定規（鏡・回転）の操作（egui_kittest。試しの立方体の上で）。
//! 面のダブの計算は core の `tests/surface_stroke.rs` が見る。ここは、画面の入力・状態・知らせ・Undo がつながっていること。
use crate::common;

use common::*;
use egui::{pos2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::Harness;
use yolu_app::engine::{composite_pixel, BrushEffect, Rgba8, Tilt};
use yolu_app::pen::adjust::PressureAdjust;
use yolu_app::pen::PenSample;
use yolu_app::rulers::RulerAction;
use yolu_app::state::Action;
use yolu_app::YoluApp;
use yolu_core::glam::{DVec3, Vec3};

const SIZE: u32 = 256;

/// 3D のタブを出し、試しの立方体を読み、右（+X）と手前（−Z）の面が見えるカメラにする。
pub(crate) fn cube_view() -> (Harness<'static, YoluApp>, Rect) {
    let mut h = app(1470.0, 760.0, SIZE);
    h.state_mut().state.view3d.load_demo();
    h.state_mut().state.view3d.camera.yaw = -40.0;
    h.state_mut().state.view3d.camera.pitch = 15.0;
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブを描いた");
    (h, rect)
}

pub(crate) fn screen_of(h: &Harness<'_, YoluApp>, rect: Rect, p: Vec3) -> Pos2 {
    let view = h
        .state()
        .state
        .view3d
        .camera
        .view(rect.width(), rect.height());
    let s = view.to_screen(p).expect("カメラの前");
    pos2(rect.left() + s.x, rect.top() + s.y)
}

/// 立方体の UV アイランド（3 × 2）。文書の画素 → (列, 行)。
fn island(x: u32, y: u32) -> (u32, u32) {
    (x * 3 / SIZE, y * 2 / SIZE)
}

fn alpha_at(h: &Harness<'_, YoluApp>, x: u32, y: u32) -> [u8; 4] {
    composite_pixel(&h.state().state.doc, x, y)
}

fn painted_islands(h: &Harness<'_, YoluApp>) -> std::collections::BTreeSet<(u32, u32)> {
    let mut out = std::collections::BTreeSet::new();
    for y in 0..SIZE {
        for x in 0..SIZE {
            if alpha_at(h, x, y)[3] > 0 {
                out.insert(island(x, y));
            }
        }
    }
    out
}

/// 文書の全画素の合成（Undo で戻ったかの比較）。
fn snapshot(h: &Harness<'_, YoluApp>) -> Vec<u8> {
    let doc = &h.state().state.doc;
    doc.composite(doc.bounds()).unwrap()
}

/// 選んでいるレイヤーの、アイランドの中（縁を除く）を f で塗る。履歴は消える（準備なので）。
fn fill_island(h: &mut Harness<'_, YoluApp>, col: u32, row: u32, f: impl Fn(u32, u32) -> Rgba8) {
    let layer = h.state().state.selected_layer.expect("レイヤー");
    let (w, hh) = (SIZE / 3, SIZE / 2);
    let doc = &mut h.state_mut().state.doc;
    for y in (row * hh + 10)..(row * hh + hh - 10) {
        for x in (col * w + 10)..(col * w + w - 10) {
            doc.set_pixel(layer, x, y, f(x, y)).unwrap();
        }
    }
    doc.clear_history().unwrap();
}

fn set_effect(h: &mut Harness<'_, YoluApp>, effect: BrushEffect) {
    h.state_mut().state.m2.brush.effect = effect;
}

pub(crate) fn press_with(h: &Harness<'_, YoluApp>, at: Pos2, modifiers: Modifiers) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: true,
        modifiers,
    });
}
pub(crate) fn release_with(h: &Harness<'_, YoluApp>, at: Pos2, modifiers: Modifiers) {
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: false,
        modifiers,
    });
}

/// 世界の 2 点の間を points 区間で引く。
fn drag_world(h: &mut Harness<'_, YoluApp>, rect: Rect, from: Vec3, to: Vec3, points: usize) {
    let (a, b) = (screen_of(h, rect, from), screen_of(h, rect, to));
    let path: Vec<Pos2> = (0..=points)
        .map(|i| a + (b - a) * (i as f32 / points as f32))
        .collect();
    drag(h, &path);
}

fn message(h: &Harness<'_, YoluApp>) -> String {
    h.state().state.message.clone()
}

#[test]
fn blur_smooths_the_texture_in_3d_and_undoes_in_one_step() {
    let (mut h, rect) = cube_view();
    set_effect(&mut h, BrushEffect::Blur { radius: 2 });
    fill_island(&mut h, 0, 0, |x, y| {
        if (x + y) % 2 == 0 {
            Rgba8::new(255, 255, 255, 255)
        } else {
            Rgba8::new(0, 0, 0, 255)
        }
    });
    let before = snapshot(&h);
    drag_world(
        &mut h,
        rect,
        Vec3::new(-0.2, 0.0, -0.5),
        Vec3::new(0.2, 0.0, -0.5),
        8,
    );
    assert!(message(&h).is_empty(), "{}", message(&h));
    assert!(h.state().state.doc.can_undo());
    assert_eq!(h.state().state.doc.undo_count(), 1);
    let mut smoothed = 0;
    for y in 10..SIZE / 2 - 10 {
        for x in 10..SIZE / 3 - 10 {
            let v = alpha_at(&h, x, y)[0];
            if v != 0 && v != 255 {
                smoothed += 1;
            }
        }
    }
    assert!(smoothed > 30, "市松模様が平均へ寄る: {smoothed}");
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert_eq!(snapshot(&h), before, "1 回の Undo で戻る");
}

#[test]
fn smudge_drags_the_color_across_the_cube_edge_in_3d() {
    let (mut h, rect) = cube_view();
    set_effect(&mut h, BrushEffect::Smudge { strength: 1.0 });
    fill_island(&mut h, 0, 0, |_, _| Rgba8::new(220, 30, 30, 255)); // 手前の面
    fill_island(&mut h, 0, 1, |_, _| Rgba8::new(0, 0, 0, 255)); // 右の面
    let before = snapshot(&h);
    drag_world(
        &mut h,
        rect,
        Vec3::new(0.35, 0.0, -0.5),
        Vec3::new(0.5, 0.0, -0.1),
        12,
    );
    assert!(message(&h).is_empty(), "{}", message(&h));
    let mut dragged = 0;
    for y in SIZE / 2..SIZE {
        for x in 10..SIZE / 3 - 10 {
            if alpha_at(&h, x, y)[0] > 20 {
                dragged += 1;
            }
        }
    }
    assert!(dragged > 20, "辺の向こうの面へ色が引きずられる: {dragged}");
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert_eq!(snapshot(&h), before);
}

#[test]
fn clone_needs_a_source_set_with_an_alt_click_and_copies_the_pattern_across_faces() {
    let (mut h, rect) = cube_view();
    set_effect(
        &mut h,
        BrushEffect::Clone {
            offset: Default::default(),
        },
    );
    fill_island(&mut h, 0, 0, |x, y| {
        Rgba8::new((x * 3 % 256) as u8, (y * 2 % 256) as u8, 90, 255)
    });
    let before = snapshot(&h);
    // 元が無ければ始めない
    let dest = Vec3::new(0.5, 0.0, -0.1);
    drag_world(&mut h, rect, dest, Vec3::new(0.5, 0.0, 0.15), 6);
    assert_eq!(message(&h), "クローンの元がありません");
    assert!(!h.state().state.doc.can_undo() && !h.state().state.is_stroking());
    // Alt を押して動かさずに離すと、そこが元になる（カメラは動かない）
    let yaw = h.state().state.view3d.camera.yaw;
    let source = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
    press_with(&h, source, Modifiers::ALT);
    h.step();
    release_with(&h, source, Modifiers::ALT);
    h.run();
    assert_eq!(message(&h), "クローンの元を決めました。");
    assert!(h.state().state.clone.source.is_some());
    assert_eq!(h.state().state.view3d.camera.yaw, yaw);
    // 右の面へ描くと、手前の面の模様が写る
    h.state_mut().state.message.clear();
    drag_world(&mut h, rect, dest, Vec3::new(0.5, 0.0, 0.15), 6);
    assert!(message(&h).is_empty(), "{}", message(&h));
    let mut painted = 0;
    for y in SIZE / 2..SIZE {
        for x in 0..SIZE / 3 {
            let p = alpha_at(&h, x, y);
            if p[3] > 0 {
                assert_eq!(p[2], 90, "元の模様の色: {p:?} at {x},{y}");
                painted += 1;
            }
        }
    }
    assert!(painted > 20, "{painted}");
    assert!(
        h.state().state.clone.destination.is_some(),
        "揃えるクローンは、先の基準を次のストロークへ渡す"
    );
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert_eq!(snapshot(&h), before);
}

#[test]
fn alt_drag_still_orbits_with_the_clone_brush() {
    let (mut h, rect) = cube_view();
    set_effect(
        &mut h,
        BrushEffect::Clone {
            offset: Default::default(),
        },
    );
    let yaw = h.state().state.view3d.camera.yaw;
    let a = rect.center();
    press_with(&h, a, Modifiers::ALT);
    h.step();
    h.event(Event::PointerMoved(a + egui::vec2(40.0, 0.0)));
    h.step();
    release_with(&h, a + egui::vec2(40.0, 0.0), Modifiers::ALT);
    h.run();
    assert_ne!(h.state().state.view3d.camera.yaw, yaw, "回る");
    assert!(
        h.state().state.clone.source.is_none(),
        "動かしたクリックは元にしない"
    );
}

#[test]
fn the_3d_mirror_paints_the_other_half_of_the_face() {
    let (mut h, rect) = cube_view();
    h.state_mut().state.color.set_main([0.9, 0.1, 0.1, 1.0]);
    // X に直交する面、モデルの原点
    common::rulers::mirror_3d(&mut h.state_mut().state, DVec3::ZERO, DVec3::X);
    let base = h.state().state.doc.undo_count();
    let at = screen_of(&h, rect, Vec3::new(0.25, 0.0, -0.5));
    click(&mut h, at);
    assert!(message(&h).is_empty(), "{}", message(&h));
    // 手前の面のアイランドの、左右の半分の画素
    let (mid, w) = ((10 + SIZE / 3 - 10) / 2, SIZE / 3);
    let count = |h: &Harness<'_, YoluApp>, range: std::ops::Range<u32>| -> usize {
        (0..SIZE / 2)
            .map(|y| range.clone().filter(|x| alpha_at(h, *x, y)[3] > 0).count())
            .sum()
    };
    let (left, right) = (count(&h, 0..mid), count(&h, mid..w));
    assert!(left > 20 && right > 20, "{left} {right}");
    assert!(
        (left as f32 / right as f32 - 1.0).abs() < 0.2,
        "左右でほぼ同じ面積: {left} {right}"
    );
    assert_eq!(h.state().state.doc.undo_count(), base + 1);
    // 鏡を隠すと、片側だけ
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    let layer = h.state().state.selected_layer.unwrap();
    h.state_mut()
        .state
        .apply(Action::Ruler(RulerAction::SetAllVisible {
            owner: layer,
            visible: false,
        }));
    click(&mut h, at);
    let (left, right) = (count(&h, 0..mid), count(&h, mid..w));
    assert!(left == 0 || right == 0, "{left} {right}");
}

#[test]
fn the_3d_radial_rotates_around_the_axis_and_skips_hidden_copies_unless_asked() {
    let (mut h, rect) = cube_view();
    // Y 軸のまわりの回転対称 4 本
    let id = common::rulers::symmetry_3d(
        &mut h.state_mut().state,
        DVec3::ZERO,
        DVec3::X,
        DVec3::Y,
        4,
        false,
    );
    let at = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
    click(&mut h, at);
    // 見える 2 面（手前と右）だけ。後ろと左は見えないので、知らせて飛ばす
    assert_eq!(painted_islands(&h), [(0, 0), (0, 1)].into_iter().collect());
    assert_eq!(message(&h), "見えない対称の写しは飛ばしました");
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    {
        let s = &mut h.state_mut().state;
        let layer = s.selected_layer.unwrap();
        let mut ruler = common::rulers::of(s, layer)
            .into_iter()
            .find(|r| r.id == id)
            .unwrap();
        ruler.see_through = true;
        s.apply(Action::Ruler(RulerAction::Replace {
            owner: layer,
            ruler,
            coalesce: false,
        }));
        s.message.clear();
    }
    let base = h.state().state.doc.undo_count();
    click(&mut h, at);
    assert_eq!(
        painted_islands(&h),
        [(0, 0), (1, 0), (2, 0), (0, 1)].into_iter().collect(),
        "見えない面の写しも塗る"
    );
    assert!(message(&h).is_empty(), "{}", message(&h));
    assert_eq!(h.state().state.doc.undo_count(), base + 1);
}

#[test]
fn smudge_and_clone_do_not_start_with_the_3d_symmetry() {
    let (mut h, rect) = cube_view();
    common::rulers::mirror_3d(&mut h.state_mut().state, DVec3::ZERO, DVec3::X);
    let base = h.state().state.doc.undo_count();
    for effect in [
        BrushEffect::Smudge { strength: 1.0 },
        BrushEffect::Clone {
            offset: Default::default(),
        },
    ] {
        set_effect(&mut h, effect);
        h.state_mut().state.message.clear();
        // クローンは元があっても始めない
        if matches!(effect, BrushEffect::Clone { .. }) {
            let layer_hit = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
            press_with(&h, layer_hit, Modifiers::ALT);
            h.step();
            release_with(&h, layer_hit, Modifiers::ALT);
            h.run();
        }
        drag_world(
            &mut h,
            rect,
            Vec3::new(0.1, 0.0, -0.5),
            Vec3::new(0.3, 0.0, -0.5),
            6,
        );
        assert_eq!(
            message(&h),
            "指先・クローンでは対称を使えません",
            "{effect:?}"
        );
        assert!(h.state().state.doc.undo_count() == base && !h.state().state.is_stroking());
    }
}

#[test]
fn the_ruler_panel_offers_the_axis_center_and_hidden_surface_items_for_a_3d_symmetry_ruler() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    // プロパティの欄が縦に収まる高さの画面に、試しの立方体を出す
    let mut h = app(1470.0, 2400.0, SIZE);
    h.state_mut().state.view3d.load_demo();
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    // 立方体の境界の中心は原点。中心をずらした 3D の対称定規（線対称 6 本、回転の軸は X）を置く
    let id = common::rulers::symmetry_3d(
        &mut h.state_mut().state,
        DVec3::new(0.2, 0.0, 0.0),
        DVec3::Y,
        DVec3::X,
        6,
        true,
    );
    h.state_mut().state.ui.property_tab = 1;
    h.run();
    let layer = h.state().state.selected_layer.unwrap();
    let ruler = |h: &Harness<'_, YoluApp>| {
        common::rulers::of(&h.state().state, layer)
            .into_iter()
            .find(|r| r.id == id)
            .unwrap()
    };
    // 軸 [X][Y][Z]、中心 [原点][境界の中心]、見えない面にも写す
    // 3D ビューの軸の部品にも同じ名前があるので、右の列（プロパティ）のものを取る
    let in_props = |h: &Harness<'_, YoluApp>, name: &str| -> Option<Rect> {
        h.query_all_by_role_and_label(Role::Button, name)
            .map(|n| n.rect())
            .find(|r| r.left() > rx())
    };
    assert!(in_props(&h, "X").is_some());
    let z = in_props(&h, "Z").expect("軸の Z");
    click(&mut h, z.center());
    match ruler(&h).place {
        yolu_core::RulerPlace::Model { up, b, a } => {
            assert!(
                (up - DVec3::Z).length() < 1e-9,
                "回転の軸が Z になる: {up:?}"
            );
            assert!(
                ((b - a).normalize() - DVec3::X).length() < 1e-9,
                "最初の線は次の軸"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(h.query_by_label("境界の中心").is_some());
    h.get_by_label("境界の中心").click();
    h.run();
    let yolu_core::RulerPlace::Model { a, .. } = ruler(&h).place else {
        panic!()
    };
    assert!(a.length() < 1e-5, "立方体の中心は原点: {a:?}");
    assert!(!ruler(&h).see_through);
    h.get_by_role_and_label(Role::CheckBox, "見えない面にも写す")
        .click();
    h.run();
    assert!(ruler(&h).see_through);
    // 説明文は画面に出さない（ツールチップだけ）
    assert!(h.query_by_label_contains("をクリック").is_none());
}

#[test]
fn all_layers_makes_the_2d_clone_read_the_visible_composite_too() {
    use yolu_app::engine::DVec2;
    use yolu_app::state::AppState;
    let mut app = AppState::new(32, 32);
    let below = app.selected_layer.expect("最初のレイヤー");
    for x in 0..8 {
        app.doc
            .set_pixel(below, x, 4, Rgba8::new(200, 30, 30, 255))
            .unwrap();
    }
    let target = app.doc.add_layer("target").unwrap();
    app.selected_layer = Some(target);
    app.doc.clear_history().unwrap();
    app.m2.random_seed = false;
    app.m2.brush.effect = BrushEffect::Clone {
        offset: DVec2::new(-10.0, 0.0),
    };
    for all_layers in [false, true] {
        app.clone.all_layers = all_layers;
        let mut stroke = app.begin_canvas_stroke(target, false, None).unwrap();
        stroke
            .add_point(&mut app.doc, 14.5, 4.5, 1.0, DVec2::ZERO)
            .unwrap();
        app.doc.end_stroke(stroke).unwrap();
        let copied = app
            .doc
            .layer(target)
            .unwrap()
            .pixel(yolu_app::engine::Channel::Color, 14, 4)
            .unwrap();
        // 描くレイヤーだけを読むと空を写すだけ。全レイヤーなら、下のレイヤーの赤を写す
        assert_eq!(
            copied.a > 0,
            all_layers,
            "all_layers={all_layers}: {copied:?}"
        );
        if all_layers {
            assert_eq!((copied.r, copied.g, copied.b), (200, 30, 30));
            assert_eq!(app.doc.undo_count(), 1);
            app.doc.undo().unwrap();
        }
    }
}

/// 左半分が白、右半分が黒の灰色の PNG（量のモードで、左半分だけが通る）を一時のフォルダに書く。
fn half_stencil_png(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("yolu-view3d-brush-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let img = image::RgbaImage::from_fn(64, 64, |x, _| {
        let v = if x < 32 { 255 } else { 0 };
        image::Rgba([v, v, v, 255])
    });
    let path = dir.join("half.png");
    img.save(&path).unwrap();
    (dir, path)
}

fn smoothed_pixels(h: &Harness<'_, YoluApp>) -> usize {
    let mut n = 0;
    for y in 10..SIZE / 2 - 10 {
        for x in 10..SIZE / 3 - 10 {
            let v = alpha_at(h, x, y)[0];
            if v != 0 && v != 255 {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn a_3d_blur_goes_through_the_stencil_laid_over_the_view_and_undoes_in_one_step() {
    let (dir, path) = half_stencil_png("blur");
    let checker = |x: u32, y: u32| {
        if (x + y).is_multiple_of(2) {
            Rgba8::new(255, 255, 255, 255)
        } else {
            Rgba8::new(0, 0, 0, 255)
        }
    };
    let line = (Vec3::new(-0.3, 0.0, -0.5), Vec3::new(0.3, 0.0, -0.5));
    // ステンシル無し
    let plain = {
        let (mut h, rect) = cube_view();
        set_effect(&mut h, BrushEffect::Blur { radius: 2 });
        fill_island(&mut h, 0, 0, checker);
        drag_world(&mut h, rect, line.0, line.1, 12);
        smoothed_pixels(&h)
    };
    assert!(plain > 60, "{plain}");
    // 画像の白い左半分と黒い右半分の境が、線の真ん中を通るように貼る
    let (mut h, rect) = cube_view();
    set_effect(&mut h, BrushEffect::Blur { radius: 2 });
    fill_island(&mut h, 0, 0, checker);
    h.state_mut().state.apply(yolu_app::state::Action::Stencil(
        yolu_app::stencil::StencilOp::Load(path),
    ));
    let mid = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
    h.state_mut().state.stencil.size = 0.9;
    h.state_mut().state.stencil.set_center(
        (mid.x - rect.left()) / rect.width(),
        (mid.y - rect.top()) / rect.height(),
    );
    h.state_mut().state.message.clear();
    h.run();
    let before = snapshot(&h);
    drag_world(&mut h, rect, line.0, line.1, 12);
    assert!(message(&h).is_empty(), "{}", message(&h));
    let through = smoothed_pixels(&h);
    assert!(
        through > plain / 4 && through < plain * 3 / 4,
        "片側だけがぼける: {through} / {plain}"
    );
    assert_eq!(h.state().state.doc.undo_count(), 1);
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert_eq!(snapshot(&h), before, "1 回の Undo で戻る");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_clone_toggles_show_their_state_by_being_dim_and_say_it_in_no_sentence() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT, Queryable};
    let (mut h, rect) = cube_view();
    set_effect(
        &mut h,
        BrushEffect::Clone {
            offset: Default::default(),
        },
    );
    // クローンの欄は、ブラシの詳細のウィンドウの「効果」のカテゴリ
    open_detail(&mut h, yolu_app::brushes::Category::Effect);
    let disabled = |h: &Harness<'_, YoluApp>, label: &str| {
        h.get_by_role_and_label(Role::CheckBox, label)
            .accesskit_node()
            .is_disabled()
    };
    // 元が無いあいだは、揃えるは薄い（状態の文は出さない）
    assert!(disabled(&h, "揃える"));
    assert!(!disabled(&h, "全レイヤーから"));
    assert!(h.query_by_label("元がありません").is_none());
    assert!(h.query_by_label("元を決めました").is_none());
    // Alt クリックで元を決めると、揃えるが使える（詳細のウィンドウがモデルに重ならないよう、押す間は閉じる）
    h.state_mut().state.brushes.ui.detail.open = false;
    h.run();
    let source = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
    press_with(&h, source, Modifiers::ALT);
    h.step();
    release_with(&h, source, Modifiers::ALT);
    h.run();
    assert!(h.state().state.clone.source.is_some());
    open_detail(&mut h, yolu_app::brushes::Category::Effect);
    assert!(!disabled(&h, "揃える"));
    assert!(h.query_by_label("元を決めました").is_none());
    // マスクを描くあいだは、全レイヤーから読めない（描いているマスクだけを読む）ので、切った表示で薄い
    h.state_mut().state.clone.all_layers = true;
    h.run();
    assert!(!disabled(&h, "全レイヤーから"));
    let layer = h.state().state.selected_layer.expect("レイヤー");
    h.state_mut().state.doc.add_layer_mask(layer).unwrap();
    h.state_mut().state.set_edit_mask(true);
    h.run();
    assert!(disabled(&h, "全レイヤーから"));
    assert_eq!(
        h.get_by_role_and_label(Role::CheckBox, "全レイヤーから")
            .accesskit_node()
            .toggled(),
        Some(egui::accesskit::Toggled::False)
    );
    assert!(
        h.state().state.clone.all_layers,
        "設定は変えない（マスクをやめれば戻る）"
    );
}

/// ブラシの詳細のウィンドウを、そのカテゴリで開く。
fn open_detail(h: &mut Harness<'_, YoluApp>, category: yolu_app::brushes::Category) {
    let ui = &mut h.state_mut().state.brushes.ui;
    ui.detail.open = true;
    ui.detail.category = category;
    ui.detail.scroll = 0.0;
    h.run();
}

/// 世界の 2 点の間を、ペンの筆圧 pressure（線の間ずっと同じ）で引く（触れる → 動く → 離す。1 点ごとに 1 フレーム）。
fn pen_world(h: &mut Harness<'_, YoluApp>, rect: Rect, from: Vec3, to: Vec3, pressure: f32) {
    let (a, b) = (screen_of(h, rect, from), screen_of(h, rect, to));
    let sample = |at: Pos2, pressure: f32, contact: bool, time_ms: u32| PenSample {
        pos: [at.x, at.y],
        pressure,
        tilt: Tilt::default(),
        rotation: None,
        contact,
        eraser: false,
        barrel: false,
        pointer_id: 5,
        time_ms,
    };
    for i in 0..=8u32 {
        let at = a + (b - a) * (i as f32 / 8.0);
        h.state().pen().push(sample(at, pressure, true, i * 10));
        h.step();
    }
    h.state().pen().push(sample(b, 0.0, false, 100));
    h.step();
    h.run();
}

/// 全体の筆圧の調整は、3D ビューのペンの線にも効く: 下限 0.25・上限 0.75 の調整を入れたペンの 0.375 は、調整しないペンの 0.25 と同じ線
/// （2 の冪の値なので丸めが無い）。調整しないペンの 0.375 は別の線。
#[test]
fn the_global_pressure_adjustment_reaches_pen_strokes_in_the_3d_view() {
    let (from, to) = (Vec3::new(-0.2, 0.0, -0.5), Vec3::new(0.2, 0.0, -0.5));
    let painted = |adjust: PressureAdjust, pressure: f32| {
        let (mut h, rect) = cube_view();
        h.state_mut().state.prefs.settings.pressure = adjust;
        pen_world(&mut h, rect, from, to, pressure);
        assert!(message(&h).is_empty(), "{}", message(&h));
        assert_eq!(h.state().state.doc.undo_count(), 1, "3D の線が 1 本入った");
        snapshot(&h)
    };
    let adjusted = painted(PressureAdjust::new(0.25, 0.75, vec![]).unwrap(), 0.375);
    let plain = painted(PressureAdjust::default(), 0.25);
    assert!(
        plain.iter().skip(3).step_by(4).any(|a| *a != 0),
        "3D に描けている"
    );
    assert_eq!(adjusted, plain);
    let unadjusted = painted(PressureAdjust::default(), 0.375);
    assert_ne!(
        unadjusted, plain,
        "調整が無ければ、ペンの 0.375 は 0.25 と別の線"
    );
}

/// 3D ビューで速く動かして、1 回の入力の区間が長くなっても（ドックの境をまたぐ時など）、描いていたストロークは消えない。入力は区間のダブを
/// 並べるだけで、塗るのはフレームごとに時間の枠まで。同じ点の列は、1 フレームに 1 点ずつ（持ち越しを塗り終えてから次の点）与えても、1 フレームに
/// まとめて与えても、離すのと同じフレームに与えても、持ち越しの残る間にウィンドウのフォーカスを失っても、同じ画素の 1 本の線になる（離したあとも
/// 時間の枠で塗り続け、塗り終えたら確定する）。
#[test]
fn a_fast_move_in_the_3d_view_keeps_the_stroke_whichever_frames_the_points_come_in() {
    let corners = [
        Vec3::new(-0.45, 0.3, -0.5),
        Vec3::new(0.45, -0.3, -0.5),
        Vec3::new(0.45, -0.25, -0.5),
        Vec3::new(0.4, -0.2, -0.5),
    ];
    let mut shots = Vec::new();
    for how in [
        "one per frame",
        "one frame",
        "with the release",
        "focus lost",
    ] {
        let (mut h, rect) = cube_view();
        h.state_mut().state.brush.radius = 2.0;
        h.state_mut().state.brush.spacing = 0.05;
        // 持ち越しが残る前提の場合は、1 フレーム 1 ダブの枠（0）にする
        if matches!(how, "one frame" | "focus lost") {
            h.state_mut().state.view3d.input.paint_budget = Some(std::time::Duration::ZERO);
        }
        let points: Vec<Pos2> = corners.iter().map(|c| screen_of(&h, rect, *c)).collect();
        let last = *points.last().unwrap();
        press_with(&h, points[0], Modifiers::NONE);
        h.step();
        match how {
            "one per frame" => {
                for p in &points[1..] {
                    h.event(Event::PointerMoved(*p));
                    h.run();
                    let left = h
                        .state()
                        .state
                        .view3d
                        .input
                        .surface
                        .as_ref()
                        .unwrap()
                        .queued();
                    assert_eq!(left, 0, "{how}: 持ち越した分は後のフレームで塗り終える");
                }
                release_with(&h, last, Modifiers::NONE);
            }
            "one frame" | "focus lost" => {
                for p in &points[1..] {
                    h.event(Event::PointerMoved(*p));
                }
                h.step();
                let left = h
                    .state()
                    .state
                    .view3d
                    .input
                    .surface
                    .as_ref()
                    .unwrap()
                    .queued();
                assert!(left > 0, "{how}: 試験の前提: 時間の枠を超えて持ち越した");
                if how == "focus lost" {
                    h.event(Event::WindowFocused(false));
                } else {
                    release_with(&h, last, Modifiers::NONE);
                }
            }
            _ => {
                for p in &points[1..] {
                    h.event(Event::PointerMoved(*p));
                }
                release_with(&h, last, Modifiers::NONE);
            }
        }
        h.step();
        // 離したあとも時間の枠で塗り続け、塗り終えたら確定する
        let mut frames = 0;
        while h.state().state.is_stroking() {
            h.step();
            frames += 1;
            assert!(frames < 5000, "{how}: 確定しない");
        }
        assert!(
            h.state().state.view3d.input.surface.is_none(),
            "{how}: 塗り終えたら確定"
        );
        h.run();
        assert!(message(&h).is_empty(), "{how}: {}", message(&h));
        assert_eq!(h.state().state.doc.undo_count(), 1, "{how}: 1 本の線");
        shots.push(snapshot(&h));
    }
    let painted = shots[0].chunks(4).filter(|p| p[3] > 0).count();
    assert!(painted > 100, "線が描けている（{painted}）");
    assert!(shots[1] == shots[0], "1 フレームにまとめても同じ画素");
    assert!(shots[2] == shots[0], "離すのと同じフレームでも同じ画素");
    assert!(
        shots[3] == shots[0],
        "フォーカスを失っても、持ち越しを塗ってから確定"
    );
}

/// 3D ビューのマウスの速さの制御も、2D のキャンバスと同じく、1 フレームに来るイベントの数（マウスの報告の頻度）によらない（イベントごとの
/// 時刻はフレームの間を等分する）。同じ速さで動かせば、1 フレーム 1・2・4 イベントで同じ絵になり、倍の速さなら絵が変わる。
#[test]
fn mouse_speed_in_3d_does_not_depend_on_how_many_events_arrive_per_frame() {
    let (mut h, rect) = cube_view();
    {
        let s = &mut h.state_mut().state;
        s.m2.random_seed = false;
        s.brush.hardness = 1.0;
        s.brush.radius = 8.0;
        s.m2.brush.controls.speed_size = true;
        // 3D の速さは画面の点 / 秒（1 フレーム 8 点・60 フレーム毎秒で 480）
        s.m2.brush.controls.speed_max = 2000.0;
    }
    let ink = |h: &Harness<'_, YoluApp>| {
        let bounds = h.state().state.doc.bounds();
        h.state()
            .state
            .doc
            .composite(bounds)
            .unwrap()
            .chunks(4)
            .map(|p| p[3] as u64)
            .sum::<u64>()
    };
    // 手前の面の中ほどを右へ、1 フレームに 8 点（画面の点）ずつ
    let start = offset(screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5)), -56.0, 0.0);
    let frames = 14;
    let per_frame = 8.0;
    let run = |h: &mut Harness<'_, YoluApp>, events: usize, speed: f32| {
        let at = |i: usize| offset(start, speed * per_frame * i as f32 / events as f32, 0.0);
        let frames = (frames as f32 / speed) as usize;
        press(h, at(0), PointerButton::Primary);
        h.step();
        for frame in 0..frames {
            // 1 フレームに複数のイベント（ハーネスの step は、待たせたイベントを 1 つずつ別のフレームにするので、直に入れる）
            for e in 1..=events {
                h.input_mut()
                    .events
                    .push(Event::PointerMoved(at(frame * events + e)));
            }
            h.step();
        }
        release(h, at(frames * events), PointerButton::Primary);
        h.step();
        h.run();
        let v = ink(h);
        h.state_mut().state.apply(yolu_app::state::Action::Undo);
        h.run();
        v
    };
    let one = run(&mut h, 1, 1.0);
    let two = run(&mut h, 2, 1.0);
    let four = run(&mut h, 4, 1.0);
    assert!(one > 0);
    for (name, v) in [("2", two), ("4", four)] {
        let diff = (v as f64 - one as f64).abs() / one as f64;
        assert!(
            diff < 0.08,
            "1 フレーム {name} イベントでも同じ速さ: {v} と {one}（{diff:.3}）"
        );
    }
    // 速さは実際に効いている: 倍の速さで動かすと絵が変わる
    let fast = run(&mut h, 1, 2.0);
    assert!(fast != one, "{fast} != {one}");
}
