//! 3D ビューを増やす: タブの並びの「+」で足したビューは、カメラ・正投影・表示の種類をビューごとに持ち、モデル・描くセット・文書は共通。
//! ポインタのあるビューが入力を受け、ストロークは始めたビューのカメラで最後まで描く。足したビューは × で閉じられ、並びは保存して戻る。
//! 描きは egui_kittest（wgpu のソフトの描画）。
use crate::common;
use crate::common::drive::{Drive, DriveApp};

use common::*;
use egui::{pos2, vec2, Event, Modifiers, PointerButton, Pos2, Rect};
use egui_dock::{DockState, Split, TabDestination, TabInsert};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::state::Action;
use yolu_app::view3d::display::{Op, Shading};
use yolu_app::view3d::ViewId;
use yolu_app::{Tab, YoluApp};
use yolu_core::{Channel, ChannelInfo, ChannelKind, ColorSpace, Rgba8};

const SECOND: ViewId = ViewId(1);

/// 既定の並び（3D ビューが左・キャンバスが右）で試しの立方体を読み、「+」で 3D ビュー 2 を足して、3D ビュー 1 の右に並べたウィンドウ。
fn two_views() -> Harness<'static, YoluApp> {
    let mut h = app_default(1400.0, 800.0, 128);
    h.view3d_mut().load_demo();
    h.run();
    h.get_by_label("3D ビューを追加").click();
    h.run();
    assert_eq!(h.view3d().view_count(), 2, "「+」で 1 つ足す");
    assert!(
        h.dock().find_tab(&Tab::View3dMore(SECOND)).is_some(),
        "足したビューのタブ"
    );
    place_beside(h.dock_mut(), Tab::View3dMore(SECOND), Tab::View3d);
    // ポインタは 3D ビューの外に置く（どのビューも今のビューに替えない）
    move_to(&h, pos2(1.0, 1.0));
    h.run();
    h
}

/// `tab` を `beside` の組の右へ分けて並べる。
fn place_beside(dock: &mut DockState<Tab>, tab: Tab, beside: Tab) {
    let from = dock.find_tab(&tab).expect("動かすタブ");
    let to = dock.find_tab(&beside).expect("隣のタブ").node_path();
    dock.move_tab(
        from,
        TabDestination::Node(to, TabInsert::Split(Split::Right)),
    );
}

/// ビューのタブの見出し（ドックのタブの文字は読み上げの木に出ないので、見出しを作る関数で読む）。
fn title(h: &Harness<'_, YoluApp>, id: ViewId) -> String {
    yolu_app::view3d::views::tab_title(h.app(), id)
}

fn rect_of_view(h: &Harness<'_, YoluApp>, id: ViewId) -> Rect {
    h.view3d_rect_of(id).expect("ビューを描いた")
}

/// ポインタをこのビューの真ん中へ（乗ったビューが今のビューになる）。
fn hover(h: &mut Harness<'_, YoluApp>, id: ViewId) -> Pos2 {
    let at = rect_of_view(h, id).center();
    move_to(h, at);
    h.run();
    at
}

/// このビューの左上の、表示の種類のドロップダウンから選ぶ。
fn choose_shading(h: &mut Harness<'_, YoluApp>, id: ViewId, current: &str, item: &str) {
    let view = rect_of_view(h, id);
    let dropdown = rect_of(h, current, |r| {
        view.contains(r.center()) && r.top() < view.top() + 40.0
    });
    click(h, dropdown.center());
    let at = popup_item(h, item).center();
    click(h, at);
}

/// 絵のこの範囲の中の、赤い画素の数。
fn red_pixels(image: &image::RgbaImage, area: Rect) -> usize {
    let mut n = 0;
    for y in area.top().ceil() as u32..area.bottom().floor() as u32 {
        for x in area.left().ceil() as u32..area.right().floor() as u32 {
            let c = image.get_pixel(x, y).0;
            if c[0] > 150 && c[1] < 90 && c[2] < 90 {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn each_view_keeps_its_own_camera_and_shading_and_the_tab_says_which() {
    let mut h = two_views();
    let first = rect_of_view(&h, ViewId::FIRST);
    let second = rect_of_view(&h, SECOND);
    assert!(
        second.left() >= first.right() - 1.0,
        "横に並ぶ: {first:?} {second:?}"
    );
    // ポインタの乗ったビューが今のビュー
    hover(&mut h, SECOND);
    assert_eq!(h.view3d().active_view(), SECOND);
    // 3D ビュー 2 の左上でラフネスを選ぶと、3D ビュー 2 だけがラフネスの表示になり、見出しに名前と表示の種類
    choose_shading(&mut h, SECOND, "マテリアル (PBR)", "ラフネス");
    let v3 = h.view3d();
    assert_eq!(
        v3.view_of(SECOND).unwrap().display.shading,
        Shading::Channel(Channel::Roughness)
    );
    assert_eq!(
        v3.view_of(ViewId::FIRST).unwrap().display.shading,
        Shading::Material
    );
    assert_eq!(title(&h, SECOND), "3D ビュー 2・ラフネス");
    assert_eq!(title(&h, ViewId::FIRST), "3D ビュー・マテリアル");
    // 3D ビュー 2 を回しても、3D ビュー 1 のカメラは動かない
    let before_first = h.view3d().view_of(ViewId::FIRST).unwrap().camera;
    let before_second = h.view3d().view_of(SECOND).unwrap().camera;
    let at = hover(&mut h, SECOND);
    press(&h, at, PointerButton::Secondary);
    h.step();
    for dx in [20.0, 40.0, 60.0] {
        move_to(&h, at + vec2(dx, 0.0));
        h.step();
    }
    release(&h, at + vec2(60.0, 0.0), PointerButton::Secondary);
    h.run();
    let after_second = h.view3d().view_of(SECOND).unwrap().camera;
    assert_ne!(after_second.yaw, before_second.yaw, "3D ビュー 2 が回る");
    assert_eq!(
        h.view3d().view_of(ViewId::FIRST).unwrap().camera,
        before_first,
        "3D ビュー 1 は動かない"
    );
    // 光の設定は全部のビューで同じ値
    h.act(Action::View3d(Op::LightIntensity(2.0)));
    h.run();
    for v in h.view3d().views() {
        assert_eq!(v.display.light_intensity, 2.0, "{:?}", v.id);
    }
    // 英語の画面では英語の名前
    h.set_lang(yolu_app::lang::Lang::En);
    h.run();
    assert_eq!(title(&h, SECOND), "3D View 2 · Roughness");
    assert_eq!(title(&h, ViewId::FIRST), "3D View · Material");
}

#[test]
fn a_stroke_from_the_second_view_paints_the_shared_document_and_shows_in_the_first() {
    let mut h = two_views();
    h.color_mut().set_main([0.85, 0.1, 0.1, 1.0]);
    let first = rect_of_view(&h, ViewId::FIRST);
    let before = red_pixels(&h.render().unwrap(), first);
    let at = hover(&mut h, SECOND);
    let undo = h.doc().undo_count();
    drag(
        &mut h,
        &[
            at,
            at + vec2(15.0, 5.0),
            at + vec2(30.0, 10.0),
            at + vec2(45.0, 10.0),
        ],
    );
    assert!(
        h.doc().undo_count() > undo,
        "3D ビュー 2 で描いた線が文書に入る"
    );
    // 同じテクスチャなので、3D ビュー 1 のマテリアルの表示にも出る
    let after = red_pixels(&h.render().unwrap(), first);
    assert!(
        after > before + 50,
        "3D ビュー 1 にも線が出る: {before} → {after}"
    );
}

#[test]
fn a_stroke_keeps_the_view_it_started_in_until_it_ends() {
    let mut h = two_views();
    let at = hover(&mut h, SECOND);
    press(&h, at, PointerButton::Primary);
    h.step();
    // 描いたまま 3D ビュー 1 の上へ出ても、今のビューは替えない（始めたビューのカメラで描き続ける）
    let first = rect_of_view(&h, ViewId::FIRST).center();
    for p in [at + vec2(10.0, 0.0), first, first + vec2(10.0, 0.0)] {
        move_to(&h, p);
        h.step();
    }
    assert!(h.app().is_stroking());
    assert_eq!(h.view3d().active_view(), SECOND);
    assert!(
        h.view3d().view.input.stroke.is_some(),
        "ストロークは 3D ビュー 2 のもの"
    );
    release(&h, first + vec2(10.0, 0.0), PointerButton::Primary);
    h.run();
    assert!(!h.app().is_stroking());
    // 離した後は、ポインタのあるビューが今のビューになる
    move_to(&h, first + vec2(5.0, 5.0));
    h.run();
    assert_eq!(h.view3d().active_view(), ViewId::FIRST);
}

#[test]
fn view_keys_go_to_the_view_under_the_pointer() {
    let mut h = two_views();
    let first_camera = h.view3d().view_of(ViewId::FIRST).unwrap().camera;
    hover(&mut h, SECOND);
    // 「選んだセットを収める」（.）は、ポインタのある 3D ビュー 2 だけを動かす
    h.view3d_mut().view_of_mut(SECOND).unwrap().camera.distance = 30.0;
    key(&h, egui::Key::Period, Modifiers::NONE);
    h.run();
    let second = h.view3d().view_of(SECOND).unwrap().camera;
    assert!(
        second.distance < 30.0,
        "3D ビュー 2 が収める位置へ: {second:?}"
    );
    assert_eq!(
        h.view3d().view_of(ViewId::FIRST).unwrap().camera,
        first_camera
    );
    // ポインタが 3D ビューの外へ出ても、最後に触った 3D ビュー 2 が受ける
    move_to(&h, pos2(1.0, 1.0));
    h.run();
    assert_eq!(h.view3d().active_view(), SECOND);
}

#[test]
fn closing_the_second_view_drops_its_state_and_its_picture() {
    let mut h = two_views();
    hover(&mut h, ViewId::FIRST);
    hover(&mut h, SECOND);
    assert_eq!(
        h.view3d_target_views(),
        vec![ViewId::FIRST, SECOND],
        "2 つのビューの描き先"
    );
    // 中ボタンでタブを閉じる（× と同じ）。最初のビューは閉じない
    let tab = h.tab_center(&Tab::View3dMore(SECOND));
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: tab,
            button: PointerButton::Middle,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.run();
    assert!(h.dock().find_tab(&Tab::View3dMore(SECOND)).is_none());
    assert_eq!(h.view3d().view_count(), 1);
    assert_eq!(h.view3d().active_view(), ViewId::FIRST);
    assert_eq!(h.view3d_target_views(), vec![ViewId::FIRST]);
    let tab = h.tab_center(&Tab::View3d);
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: tab,
            button: PointerButton::Middle,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.run();
    assert!(
        h.dock().find_tab(&Tab::View3d).is_some(),
        "最初のビューは閉じない"
    );
    // 見出しは名前だけに戻る
    assert_eq!(title(&h, ViewId::FIRST), "3D ビュー");
}

#[test]
fn the_views_are_saved_in_the_layout_and_come_back() {
    // 保存する名前: 足したビューは「view3d-番号」。番号の書き方は 1 通り・2 から上限まで
    assert_eq!(Tab::View3dMore(SECOND).save_key(), "view3d-2");
    assert_eq!(Tab::from_key("view3d-2"), Some(Tab::View3dMore(SECOND)));
    assert_eq!(Tab::from_key("view3d"), Some(Tab::View3d));
    for bad in [
        "view3d-1",
        "view3d-0",
        "view3d-02",
        "view3d-9",
        "view3d-x",
        "view3d-",
    ] {
        assert_eq!(Tab::from_key(bad), None, "{bad}");
    }
    let h = two_views();
    let json = serde_json::to_string(h.dock()).unwrap();
    assert!(json.contains("\"view3d-2\""), "{json}");
    let back: DockState<Tab> = serde_json::from_str(&json).unwrap();
    yolu_app::layout::validate(&back).expect("戻した並びは正しい");
    // 同じビューが 2 つある並びは読まない
    let doubled = json.replacen("\"canvas\"", "\"view3d-2\"", 1);
    let doubled: DockState<Tab> = serde_json::from_str(&doubled).unwrap();
    assert!(yolu_app::layout::validate(&doubled).is_err());
    // 戻した並びでウィンドウを開き直すと、ビューも戻る
    let mut again = app_default(1400.0, 800.0, 128);
    *again.dock_mut() = back;
    again.run();
    assert_eq!(again.view3d().view_ids(), vec![ViewId::FIRST, SECOND]);
    assert!(again.view3d_rect_of(SECOND).is_some());
}

#[test]
fn a_user_channel_shows_alone_in_a_view() {
    let mut h = two_views();
    let mask = h
        .doc_mut()
        .add_channel(ChannelInfo {
            name: "マスク".into(),
            kind: ChannelKind::Scalar,
            color_space: ColorSpace::Linear,
            default: Rgba8::new(0, 0, 0, 255),
        })
        .unwrap();
    h.doc_mut()
        .add_fill_layer("値", &[(mask, Rgba8::new(200, 200, 200, 255))], None)
        .unwrap();
    h.run();
    hover(&mut h, SECOND);
    choose_shading(&mut h, SECOND, "マテリアル (PBR)", "マスク");
    assert_eq!(
        h.view3d().view_of(SECOND).unwrap().display.shading,
        Shading::Channel(mask)
    );
    // 標準の見た目のセットでも、選んだユーザーチャンネルだけを GPU の配列に持つ
    let stats = h.view3d_stats().unwrap();
    assert!(stats.user_bytes > 0, "{stats:?}");
    let image = h.render().unwrap();
    let c = image.get_pixel(
        rect_of_view(&h, SECOND).center().x as u32,
        rect_of_view(&h, SECOND).center().y as u32,
    );
    for k in 0..3 {
        assert!(c.0[k].abs_diff(200) <= 3, "マスクの値がそのまま: {:?}", c.0);
    }
    assert_eq!(title(&h, SECOND), "3D ビュー 2・マスク");
    // 見なくなったら持たない
    h.act(Action::View3d(Op::ShadingOf(SECOND, Shading::Material)));
    h.run();
    hover(&mut h, SECOND);
    assert_eq!(h.view3d_stats().unwrap().user_bytes, 0);
}

#[test]
fn snapshot_two_views_side_by_side() {
    let mut h = two_views();
    hover(&mut h, SECOND);
    h.act(Action::View3d(Op::ShadingOf(
        SECOND,
        Shading::Channel(Channel::Roughness),
    )));
    h.view3d_mut().view_of_mut(SECOND).unwrap().camera.yaw = 30.0;
    move_to(&h, pos2(1.0, 1.0));
    h.run();
    let area = rect_of_view(&h, ViewId::FIRST).union(rect_of_view(&h, SECOND));
    common::shot::region(
        &mut h,
        "view3d_two_views",
        Rect::from_min_max(pos2(area.left(), area.top() - 26.0), area.max),
    );
}

// ───────── 計測（cargo test -p yolu-app --test gui_view3d -- --ignored --nocapture multiview::measure） ─────────

/// 1 つのビューと 2 つのビューで、カメラを回し続けるときの 1 フレームの時間（同じウィンドウの大きさ・同じ仕事）。
#[test]
#[ignore = "計測"]
fn measure_one_and_two_views_while_orbiting() {
    use std::time::Instant;
    let frames = 120;
    let run = |two: bool| {
        let mut h = app_default(1600.0, 900.0, 2048);
        h.view3d_mut().load_demo();
        h.run();
        if two {
            h.get_by_label("3D ビューを追加").click();
            h.run();
            place_beside(h.dock_mut(), Tab::View3dMore(SECOND), Tab::View3d);
            h.run();
        }
        let adapter = h.view3d_adapter().unwrap_or_default();
        let renders = h.view3d_stats().unwrap().renders;
        let mut times = Vec::new();
        for i in 0..frames {
            // どのビューも描き直す（カメラを替える）
            for id in h.view3d().view_ids() {
                h.view3d_mut().view_of_mut(id).unwrap().camera.yaw = i as f32;
            }
            let started = Instant::now();
            h.step();
            h.view3d_wait_gpu();
            times.push(started.elapsed().as_micros() as u64);
        }
        let drawn = h.view3d_stats().unwrap().renders - renders;
        times.sort_unstable();
        let mean = times.iter().sum::<u64>() as f64 / times.len() as f64 / 1000.0;
        let p95 = times[times.len() * 95 / 100] as f64 / 1000.0;
        (adapter, mean, p95, drawn)
    };
    for two in [false, true] {
        let (adapter, mean, p95, drawn) = run(two);
        println!(
            "GPU: {adapter}、ビュー {}: 1 フレーム 平均 {mean:.2} ms・95% {p95:.2} ms（{frames} フレーム、3D を描いた回数 {drawn}）",
            if two { 2 } else { 1 }
        );
    }
}
