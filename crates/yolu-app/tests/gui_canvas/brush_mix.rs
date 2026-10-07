//! 色の混ぜ（厚塗りのブラシ）の画面と組み込み: 詳細のウィンドウの「色の混ぜ」の節（混ぜ方の排他の切り替え・スライダー・全レイヤーから・
//! 筆圧・既定に戻す・効かない理由）、組み込みの厚塗りの筆 3 つ（筆のグループ・見本の線・版 3 で保存して読み戻す）、
//! 「全レイヤーから」で下のレイヤーの色を拾う（アプリの入口から）、日英。
use crate::common;

use common::*;
use egui::{pos2, Event, Key, Modifiers, Pos2, Rect};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::brushes::sample::{self, SampleSpec};
use yolu_app::brushes::{store, BrushAction, BrushKey, Category, Group};
use yolu_app::engine::{Channel, ColorMix, DVec2, MixGround, MixMode, Rgba8};
use yolu_app::lang::Lang;
use yolu_app::m2::UiOp;
use yolu_app::state::{Action, AppState, Tool};
use yolu_app::YoluApp;

type H = Harness<'static, YoluApp>;

fn st(h: &H) -> &AppState {
    &h.state().state
}

fn b(id: &'static str) -> BrushKey {
    BrushKey::Builtin(id)
}

fn language(h: &mut H, lang: Lang) {
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(lang)));
    h.run();
}

fn open_detail(h: &mut H, category: Category) {
    let ui = &mut h.state_mut().state.brushes.ui;
    ui.detail.open = true;
    ui.detail.category = category;
    ui.detail.scroll = 0.0;
    h.run();
}

fn detail_rect(h: &H) -> Rect {
    yolu_app::ui::window::last_rect(&h.ctx, yolu_app::panels::brush_detail::id())
        .expect("ブラシの詳細のウィンドウを描いている")
}

/// ウィンドウの右側の欄の部品（左のカテゴリの同じ名前と区別する）。上から順に。
fn pane_nodes(h: &H, label: &str) -> Vec<Rect> {
    let window = detail_rect(h);
    let mut nodes: Vec<Rect> = h
        .query_all_by_label(label)
        .map(|n| n.rect())
        .filter(|r| window.contains(r.center()) && r.left() > window.left() + 168.0)
        .collect();
    nodes.sort_by(|a, b| a.top().total_cmp(&b.top()));
    nodes
}

fn in_pane(h: &H, label: &str) -> Rect {
    *pane_nodes(h, label)
        .first()
        .unwrap_or_else(|| panic!("ウィンドウの欄に {label} が無い"))
}

fn disabled(h: &H, label: &str) -> bool {
    let window = detail_rect(h);
    h.query_all_by_label(label)
        .find(|n| window.contains(n.rect().center()) && n.rect().left() > window.left() + 168.0)
        .unwrap_or_else(|| panic!("{label}"))
        .accesskit_node()
        .is_disabled()
}

fn shot(h: &mut H, rect: Rect, name: &str) {
    h.event(Event::PointerGone);
    h.step();
    let image = h.render().expect("描画");
    let cropped = image::imageops::crop_imm(
        &image,
        rect.left().floor() as u32,
        rect.top().floor() as u32,
        rect.width().ceil() as u32,
        rect.height().ceil() as u32,
    )
    .to_image();
    egui_kittest::image_snapshot(&cropped, name);
}

/// 試験ごとの設定のフォルダの中の、ブラシのフォルダ（ツールの並びの `tools.json` は、その隣に置かれる）。
fn temp_dir(name: &str) -> std::path::PathBuf {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/brush-mix-tests")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join("brushes");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ───────── 組み込みの厚塗りの筆 ─────────

#[test]
fn headless_the_thick_paint_brushes_are_built_in_brush_group_brushes_that_mix() {
    let mut s = AppState::new(64, 64);
    for (id, mode) in [
        ("oil", MixMode::Mix),
        ("gouache", MixMode::Mix),
        ("mixer", MixMode::Smear),
    ] {
        s.apply(Action::Brush(BrushAction::Select(b(id))));
        assert_eq!(s.brushes.lib.current(), b(id), "{id}");
        assert_eq!(s.shown_brush_group(), Some(Group::Brush), "{id}");
        assert_eq!(s.tool, Tool::Brush);
        assert_eq!(s.m2.brush.mix.mode, mode, "{id}");
        assert!(s.m2.brush.mix.validate().is_ok());
    }
    // 筆のグループの中、今までの筆の後ろ。名前は日英
    let names: Vec<String> = s
        .brushes
        .lib
        .entries()
        .iter()
        .filter(|e| e.group == Group::Brush)
        .map(|e| e.name_in(Lang::Ja))
        .collect();
    assert!(
        names.ends_with(&["油彩".into(), "ガッシュ".into(), "混色".into()]),
        "{names:?}"
    );
    let english: Vec<String> = s
        .brushes
        .lib
        .entries()
        .iter()
        .filter(|e| e.group == Group::Brush)
        .map(|e| e.name_in(Lang::En))
        .collect();
    assert!(
        english.ends_with(&["Oil paint".into(), "Gouache".into(), "Mixer".into()]),
        "{english:?}"
    );
    // 選び直すと元の設定へ戻る（変えたら変更あり、元へ戻すで戻る）
    s.apply(Action::Brush(BrushAction::Select(b("oil"))));
    s.m2.brush.mix.paint = 0.9;
    s.brush_sync();
    assert!(s.brush_is_modified(b("oil")));
    s.apply(Action::Brush(BrushAction::Revert(b("oil"))));
    assert_eq!(s.m2.brush.mix.paint, 0.65f32 as f64);
}

#[test]
fn headless_a_thick_paint_brush_is_saved_as_version_three_and_reads_back_equal() {
    let dir = temp_dir("save");
    let mut s = AppState::new(64, 64);
    s.attach_brush_store(dir.clone());
    s.apply(Action::Brush(BrushAction::Select(b("oil"))));
    s.m2.brush.mix.stretch = 0.8;
    s.apply(Action::Brush(BrushAction::Add));
    let saved = s.brushes.lib.current();
    assert!(saved.is_user());
    let live = s.brush_live();
    let file = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().ends_with(".ylbrush"))
        .expect("保存したブラシのファイル");
    let text = std::fs::read_to_string(file.path()).unwrap();
    assert!(text.starts_with(store::HEADER_V3), "{text}");
    assert!(text.lines().any(|l| l == "mix.mode=mix"), "{text}");
    assert!(text.lines().any(|l| l == "mix.stretch=0.8"), "{text}");
    // 起動し直したアプリが、同じブラシを読む
    let mut again = AppState::new(64, 64);
    again.attach_brush_store(dir.clone());
    assert!(
        again.brushes.problems.is_empty(),
        "{:?}",
        again.brushes.problems
    );
    again.apply(Action::Brush(BrushAction::Select(saved)));
    assert_eq!(again.brush_live(), live);
    assert_eq!(again.m2.brush.mix.stretch, 0.8f32 as f64);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_thick_paint_samples_draw_over_a_picture_so_the_mixing_shows() {
    use yolu_app::brushes::builtin;
    for id in ["oil", "gouache", "mixer"] {
        let brush = &builtin::find(id).unwrap().brush;
        let spec = SampleSpec::row(false);
        let image = sample::render(brush, spec).unwrap();
        assert_eq!(image, sample::render(brush, spec).unwrap(), "{id}: 同じ絵");
        // 縦の帯の絵の上に描くので、端の列も不透明（普通の筆は端が透明）
        assert_eq!(image.rgba[3], 255, "{id}");
        // 混ぜを切ると、同じブラシでも絵が変わる（混ぜが見本に出ている）
        let mut plain = brush.clone();
        plain.mix = ColorMix::default();
        let without = sample::render(&plain, spec).unwrap();
        assert_ne!(image.rgba, without.rgba, "{id}: 混ぜが見本に出る");
        // 見本は黒で描く: 下の色を拾った混ぜた色が、帯の赤や青の側へ寄る（黒だけではない）
        let coloured = image
            .rgba
            .chunks(4)
            .filter(|p| p[3] > 0 && (p[0] > 90 || p[2] > 90) && p[0].abs_diff(p[2]) > 30)
            .count();
        assert!(coloured > 200, "{id}: {coloured}");
    }
}

// ───────── 詳細のウィンドウの節 ─────────

#[test]
fn the_mix_category_switches_the_mode_exclusively_and_enables_the_sliders() {
    let mut h = app(1600.0, 1000.0, 128);
    open_detail(&mut h, Category::Mix);
    let window = detail_rect(&h);
    // 左のカテゴリに名前がある
    assert!(
        h.query_all_by_label("色の混ぜ")
            .any(|n| window.contains(n.rect().center())),
        "色の混ぜ"
    );
    assert_eq!(st(&h).m2.brush.mix, ColorMix::default());
    // なし: スライダーは触れない（理由はツールチップ。画面に注記は出さない）
    for label in ["絵の具の量", "絵の具の濃さ", "色延び", "全レイヤーから"] {
        assert!(disabled(&h, label), "{label}");
    }
    // 混ぜる
    let mix = in_pane(&h, "混ぜる");
    click(&mut h, mix.center());
    assert_eq!(st(&h).m2.brush.mix.mode, MixMode::Mix);
    assert!(st(&h).brush_is_modified(b("standard")));
    for label in ["絵の具の量", "絵の具の濃さ", "色延び", "全レイヤーから"] {
        assert!(!disabled(&h, label), "{label}");
    }
    // 伸ばすへ（排他）
    let smear = in_pane(&h, "伸ばす");
    click(&mut h, smear.center());
    assert_eq!(st(&h).m2.brush.mix.mode, MixMode::Smear);
    // もう一度押すと、なしへ
    click(&mut h, smear.center());
    assert_eq!(st(&h).m2.brush.mix.mode, MixMode::Off);
    // スライダー（左の端 0%・右の端 100%。真ん中あたりで約 50%）
    click(&mut h, mix.center());
    let amount = in_pane(&h, "絵の具の量");
    click(
        &mut h,
        pos2(amount.left() + amount.width() * 0.2, amount.center().y),
    );
    assert!(
        st(&h).m2.brush.mix.paint < 0.4,
        "{}",
        st(&h).m2.brush.mix.paint
    );
    let density = in_pane(&h, "絵の具の濃さ");
    click(
        &mut h,
        pos2(density.left() + density.width() * 0.3, density.center().y),
    );
    assert!(
        st(&h).m2.brush.mix.density < 0.6,
        "{}",
        st(&h).m2.brush.mix.density
    );
    // 全レイヤーから
    let all = in_pane(&h, "全レイヤーから");
    click(&mut h, all.center());
    assert_eq!(st(&h).m2.brush.mix.ground, MixGround::Composite);
    click(&mut h, all.center());
    assert_eq!(st(&h).m2.brush.mix.ground, MixGround::Layer);
    // 既定に戻す
    let reset = h.get_by_label("色の混ぜを既定に戻す").rect().center();
    click(&mut h, reset);
    assert_eq!(st(&h).m2.brush.mix, ColorMix::default());
}

#[test]
fn the_mix_pressure_items_follow_their_switch_and_edit_the_response() {
    let mut h = app(1600.0, 1000.0, 128);
    h.state_mut()
        .state
        .apply(Action::Brush(BrushAction::Select(b("gouache"))));
    h.run();
    open_detail(&mut h, Category::Mix);
    // 筆圧を使わない間は、最小と曲線は触れない
    assert!(disabled(&h, "最小"));
    assert!(!st(&h).m2.brush.mix.pressure_paint);
    let toggles = pane_nodes(&h, "筆圧を使う");
    assert!(
        !toggles.is_empty(),
        "量の筆圧（濃さは下にスクロールして出る）"
    );
    click(&mut h, toggles[0].center());
    assert!(st(&h).m2.brush.mix.pressure_paint);
    assert!(!st(&h).m2.brush.mix.pressure_density);
    assert!(!pane_nodes(&h, "最小").is_empty());
    let minimum = pane_nodes(&h, "最小")[0];
    click(
        &mut h,
        pos2(minimum.left() + minimum.width() * 0.4, minimum.center().y),
    );
    let response = st(&h).m2.brush.mix.response_paint.clone();
    assert!((0.3..0.5).contains(&response.min()), "{}", response.min());
    assert!(
        st(&h).m2.brush.mix.response_density.is_identity(),
        "濃さの応えは変わらない"
    );
    // 切ると、切り替えだけが戻る（応えは残る）
    click(&mut h, toggles[0].center());
    assert!(!st(&h).m2.brush.mix.pressure_paint);
    assert_eq!(st(&h).m2.brush.mix.response_paint, response);
}

#[test]
fn the_mix_section_says_why_it_does_not_apply_by_disabling_instead_of_a_note() {
    let mut h = app(1600.0, 1000.0, 128);
    open_detail(&mut h, Category::Mix);
    let mix = in_pane(&h, "混ぜる");
    click(&mut h, mix.center());
    assert!(!disabled(&h, "絵の具の量"));
    // 効果のブラシ
    h.state_mut()
        .state
        .apply(Action::Brush(BrushAction::Select(b("smudge"))));
    h.run();
    open_detail(&mut h, Category::Mix);
    assert!(disabled(&h, "混ぜる") && disabled(&h, "伸ばす"));
    assert!(disabled(&h, "絵の具の量"));
    // 消しゴム
    h.state_mut()
        .state
        .apply(Action::Brush(BrushAction::Select(b("soft-eraser"))));
    h.run();
    open_detail(&mut h, Category::Mix);
    assert!(disabled(&h, "混ぜる"));
    // 画面に注記の文を置かない（使い方の文・説明の段落）。名前と状態だけ
    let window = detail_rect(&h);
    for n in h.query_all_by_label_contains("します") {
        assert!(
            !window.contains(n.rect().center()),
            "{:?}",
            n.accesskit_node().label()
        );
    }
}

#[test]
fn the_mix_section_is_in_english_without_japanese() {
    let mut h = app(1600.0, 1000.0, 128);
    language(&mut h, Lang::En);
    h.state_mut()
        .state
        .apply(Action::Brush(BrushAction::Select(b("oil"))));
    h.run();
    open_detail(&mut h, Category::Mix);
    let window = detail_rect(&h);
    for label in [
        "Mix",
        "Smear",
        "Paint amount",
        "Paint density",
        "Color stretch",
        "All layers",
    ] {
        assert!(
            h.query_all_by_label(label)
                .any(|n| window.contains(n.rect().center())),
            "{label}"
        );
    }
    assert!(h.query_all_by_label("Color Mixing").next().is_some());
    let has_japanese = |t: &str| {
        t.chars().any(|c| {
            ('\u{3040}'..='\u{30ff}').contains(&c) || ('\u{4e00}'..='\u{9fff}').contains(&c)
        })
    };
    for n in h.query_all_by_label_contains("") {
        if window.contains(n.rect().center()) {
            if let Some(label) = n.accesskit_node().label() {
                assert!(!has_japanese(&label), "英語の画面に日本語: {label}");
            }
        }
    }
}

#[test]
fn snapshot_brush_detail_mix() {
    let mut h = app(1600.0, 1000.0, 128);
    h.state_mut()
        .state
        .apply(Action::Brush(BrushAction::Select(b("oil"))));
    open_detail(&mut h, Category::Mix);
    let rect = detail_rect(&h);
    shot(&mut h, rect, "brushes_detail_mix");
    language(&mut h, Lang::En);
    h.state_mut()
        .state
        .apply(Action::Brush(BrushAction::Select(b("mixer"))));
    open_detail(&mut h, Category::Mix);
    let rect = detail_rect(&h);
    shot(&mut h, rect, "brushes_detail_mix_english");
}

#[test]
fn snapshot_brush_panel_thick_paint_group() {
    let mut h = app(1600.0, 1000.0, 128);
    h.state_mut()
        .state
        .apply(Action::Brush(BrushAction::Select(b("oil"))));
    h.run();
    h.run();
    let tab = h.state().tab_rects[&yolu_app::Tab::SubTools];
    let color = h.state().tab_rects[&yolu_app::Tab::Color];
    let rect = Rect::from_min_max(
        pos2(tab.left() - 2.0, tab.top()),
        pos2(tab.left() + 300.0, color.top()),
    );
    shot(&mut h, rect, "brushes_panel_thick_paint");
}

// ───────── アプリの入口から描く ─────────

/// 下のレイヤーを赤で塗り、その上に空のレイヤーを足して選ぶ。
fn two_layers() -> (AppState, yolu_app::engine::LayerId) {
    let mut s = AppState::new(64, 64);
    let below = s.selected_layer.unwrap();
    s.doc
        .fill(
            below,
            Channel::Color,
            Rgba8::new(255, 0, 0, 255),
            1.0,
            None,
            false,
        )
        .unwrap();
    let top = s.doc.add_layer("top").unwrap();
    s.selected_layer = Some(top);
    s.color.main = [0.0, 0.0, 0.0, 1.0];
    (s, top)
}

fn dot(s: &mut AppState, layer: yolu_app::engine::LayerId) -> Rgba8 {
    let mut stroke = s.begin_paint_stroke(layer, false).unwrap();
    stroke
        .add_point(&mut s.doc, 32.0, 32.0, 1.0, DVec2::ZERO)
        .unwrap();
    s.doc.end_stroke(stroke).unwrap();
    s.doc
        .layer(layer)
        .unwrap()
        .pixel(Channel::Color, 32, 32)
        .unwrap()
}

#[test]
fn headless_all_layers_picks_up_the_layers_below_and_the_layer_alone_does_not() {
    let mix = |ground| ColorMix {
        mode: MixMode::Mix,
        paint: 0.5,
        stretch: 0.0,
        ground,
        ..ColorMix::default()
    };
    // 今のレイヤーだけ: 上のレイヤーは空なので、描く色（黒）のまま
    let (mut s, top) = two_layers();
    s.m2.brush.mix = mix(MixGround::Layer);
    s.brush.opacity = 1.0;
    s.brush.flow = 1.0;
    assert_eq!(dot(&mut s, top), Rgba8::new(0, 0, 0, 255));
    // 見えているレイヤーの重なり: 下の赤を拾って混ぜる
    let (mut s, top) = two_layers();
    s.m2.brush.mix = mix(MixGround::Composite);
    let picked = dot(&mut s, top);
    assert!(
        picked.r > 100 && picked.g == 0 && picked.b == 0,
        "{picked:?}"
    );
    // 1 回の Undo で戻る
    s.apply(Action::Undo);
    assert_eq!(
        s.doc
            .layer(top)
            .unwrap()
            .pixel(Channel::Color, 32, 32)
            .unwrap()
            .a,
        0
    );
    // 消しゴムは混ぜない（参照元を凍結しない）
    let (mut s, top) = two_layers();
    s.m2.brush.mix = mix(MixGround::Composite);
    let stroke = s.begin_paint_stroke(top, true).unwrap();
    assert_eq!(s.doc.clone_source_bytes(), 0, "消しゴムは下地を読まない");
    s.doc.cancel_stroke(stroke);
    // 混ぜる筆は下地を凍結する
    let stroke = s.begin_paint_stroke(top, false).unwrap();
    assert!(
        s.doc.clone_source_bytes() > 0,
        "混ぜる筆は見えているレイヤーの重なりを凍結する"
    );
    s.doc.cancel_stroke(stroke);
}

// ───────── 3D のビュー ─────────

const SIZE: u32 = 256;

/// 3D のタブを出し、試しの立方体を読み、右（+X）と手前（−Z）の面が見えるカメラにする。
fn cube_view() -> (H, Rect) {
    let mut h = app(1100.0, 760.0, SIZE);
    h.state_mut().state.view3d.load_demo();
    h.state_mut().state.view3d.camera.yaw = -40.0;
    h.state_mut().state.view3d.camera.pitch = 15.0;
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブを描いた");
    (h, rect)
}

fn drag_world(h: &mut H, rect: Rect, from: [f32; 3], to: [f32; 3], points: usize) {
    let view = h
        .state()
        .state
        .view3d
        .camera
        .view(rect.width(), rect.height());
    let at = |p: [f32; 3]| {
        let s = view
            .to_screen(yolu_core::glam::Vec3::new(p[0], p[1], p[2]))
            .expect("カメラの前");
        pos2(rect.left() + s.x, rect.top() + s.y)
    };
    let (a, b) = (at(from), at(to));
    let path: Vec<Pos2> = (0..=points)
        .map(|i| a + (b - a) * (i as f32 / points as f32))
        .collect();
    drag(h, &path);
}

fn fill_island(h: &mut H, col: u32, row: u32, color: Rgba8) {
    let layer = h.state().state.selected_layer.expect("レイヤー");
    let (w, hh) = (SIZE / 3, SIZE / 2);
    let doc = &mut h.state_mut().state.doc;
    for y in (row * hh + 10)..(row * hh + hh - 10) {
        for x in (col * w + 10)..(col * w + w - 10) {
            doc.set_pixel(layer, x, y, color).unwrap();
        }
    }
    doc.clear_history().unwrap();
}

fn snapshot(h: &H) -> Vec<u8> {
    let doc = &h.state().state.doc;
    doc.composite(doc.bounds()).unwrap()
}

fn pixel(h: &H, x: u32, y: u32) -> [u8; 4] {
    yolu_app::engine::composite_pixel(&h.state().state.doc, x, y)
}

#[test]
fn mixing_and_smearing_work_in_the_3d_view_and_undo_in_one_step() {
    let ink = |h: &mut H, mix: ColorMix| {
        h.state_mut().state.color.main = [0.0, 0.0, 1.0, 1.0];
        h.state_mut().state.m2.brush.mix = mix;
    };
    // 混ぜる: 赤い面へ青で描くと、赤と青が混ざった色（紫）が置かれる（混ぜなければ青だけ）
    let (mut h, rect) = cube_view();
    fill_island(&mut h, 0, 0, Rgba8::new(255, 0, 0, 255));
    ink(&mut h, ColorMix::default());
    let before = snapshot(&h);
    drag_world(&mut h, rect, [-0.2, 0.0, -0.5], [0.2, 0.0, -0.5], 8);
    let plain = pixels_where(&h, |p| p[2] > 200 && p[0] < 60);
    assert!(plain > 30, "混ぜないと青がそのまま置かれる: {plain}");
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert_eq!(snapshot(&h), before);
    ink(
        &mut h,
        ColorMix {
            mode: MixMode::Mix,
            paint: 0.5,
            stretch: 0.0,
            ..ColorMix::default()
        },
    );
    h.state_mut().state.message.clear();
    drag_world(&mut h, rect, [-0.2, 0.0, -0.5], [0.2, 0.0, -0.5], 8);
    assert!(
        h.state().state.message.is_empty(),
        "{}",
        h.state().state.message
    );
    let purple = pixels_where(&h, |p| p[0] > 80 && p[2] > 80 && p[1] < 40);
    assert!(purple > 30, "赤と青が混ざる: {purple}");
    assert_eq!(
        pixels_where(&h, |p| p[2] > 200 && p[0] < 60),
        0,
        "青だけは置かれない"
    );
    assert_eq!(h.state().state.doc.undo_count(), 1);
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert_eq!(snapshot(&h), before, "1 回の Undo で戻る");

    // 伸ばす: 辺の向こうの面（黒）へ、手前の面（赤）の色が引きずられる（指先と同じ読み方で、量 0 は下の色だけ）
    let (mut h, rect) = cube_view();
    fill_island(&mut h, 0, 0, Rgba8::new(220, 30, 30, 255));
    fill_island(&mut h, 0, 1, Rgba8::new(0, 0, 0, 255));
    ink(
        &mut h,
        ColorMix {
            mode: MixMode::Smear,
            paint: 0.0,
            stretch: 1.0,
            ..ColorMix::default()
        },
    );
    let before = snapshot(&h);
    drag_world(&mut h, rect, [0.35, 0.0, -0.5], [0.5, 0.0, -0.1], 12);
    assert!(
        h.state().state.message.is_empty(),
        "{}",
        h.state().state.message
    );
    let mut dragged = 0;
    for y in SIZE / 2..SIZE {
        for x in 10..SIZE / 3 - 10 {
            if pixel(&h, x, y)[0] > 20 {
                dragged += 1;
            }
        }
    }
    assert!(dragged > 20, "辺の向こうの面へ色が引きずられる: {dragged}");
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert_eq!(snapshot(&h), before);
}

fn pixels_where(h: &H, f: impl Fn([u8; 4]) -> bool) -> usize {
    let mut n = 0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            if f(pixel(h, x, y)) {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn a_canvas_stroke_with_a_mixing_brush_blends_with_the_picture_and_undoes() {
    let mut h = app(1280.0, 800.0, 256);
    let layer = h.state().state.selected_layer.unwrap();
    h.state_mut()
        .state
        .doc
        .fill(
            layer,
            Channel::Color,
            Rgba8::new(0, 0, 255, 255),
            1.0,
            None,
            false,
        )
        .unwrap();
    h.state_mut().state.doc.clear_history().unwrap();
    h.state_mut().state.brush.radius = 12.0;
    h.state_mut().state.brush.pressure_size = false;
    h.state_mut().state.brush.pressure_opacity = false;
    h.run();
    let c = canvas_rect(&h).center();
    let path = [
        offset(c, -30.0, 0.0),
        offset(c, 0.0, 0.0),
        offset(c, 30.0, 0.0),
    ];
    // 混ぜない: 描く色（黒）がそのまま
    drag(&mut h, &path);
    assert_eq!(canvas_pixel(&h, c), [0, 0, 0, 255]);
    h.state_mut().state.apply(Action::Undo);
    h.run();
    assert_eq!(canvas_pixel(&h, c), [0, 0, 255, 255]);
    // 混ぜる: 黒と青が半々（絵の具の量 50%）
    h.state_mut().state.m2.brush.mix = ColorMix {
        mode: MixMode::Mix,
        paint: 0.5,
        stretch: 0.0,
        ..ColorMix::default()
    };
    drag(&mut h, &path);
    let mixed = canvas_pixel(&h, c);
    assert!(
        mixed[0] == 0 && (120..=135).contains(&mixed[2]) && mixed[3] == 255,
        "{mixed:?}"
    );
    assert_eq!(h.state().state.doc.undo_count(), 1);
    h.state_mut().state.apply(Action::Undo);
    h.run();
    assert_eq!(canvas_pixel(&h, c), [0, 0, 255, 255]);
}
