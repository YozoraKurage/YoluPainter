//! ツールプロパティ・ブラシサイズ・マテリアルの独立したパネル: 既定の並び（左の列に上から サブツール・ツールプロパティ・ブラシサイズ・カラー、右下は
//! プロパティ・マテリアル・ヒストリー）、ほかのパネルと同じに開く・外へ出す・戻す・「ウィンドウ」のメニュー、前の `layout.json`（この 3 つが無い）を読んで足す、
//! ツールプロパティの「塗るチャンネル」（描くツールだけ。マスクに描くあいだはレイヤーマスクの欄）、プロパティのタブ（ステンシル・レイヤー）。
//! 見た目の試験（日英）はパネルや列の中だけを撮る（ほかのパネルの変更で壊れない）。
use crate::common;

use std::path::{Path, PathBuf};

use common::*;
use eframe::App as _;
use egui::{pos2, vec2, Event, Rect};
use egui_dock::{DockState, Node};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::app::default_dock;
use yolu_app::detach::{DockOp, Place};
use yolu_app::engine::Channel;
use yolu_app::lang::Lang;
use yolu_app::layout::{self, DetachedRecord, FloatRecord};
use yolu_app::matpaint::MatAction;
use yolu_app::pen::PenInput;
use yolu_app::state::{Action, AppState, Tool};
use yolu_app::{Tab, YoluApp};

type H = Harness<'static, YoluApp>;

const NEW_TABS: [Tab; 3] = [Tab::ToolProperties, Tab::BrushSize, Tab::Material];

fn st(h: &H) -> &AppState {
    &h.state().state
}

fn language(h: &mut H, lang: Lang) {
    h.state_mut()
        .state
        .apply(Action::M2Ui(yolu_app::m2::UiOp::Language(lang)));
    h.run();
}

fn pick(h: &mut H, tool: Tool) {
    h.state_mut().state.apply(Action::SelectTool(tool));
    h.run();
}

fn tab_rect(h: &H, tab: Tab) -> Rect {
    h.state().tab_rects[&tab]
}

fn count(h: &H, label: &str) -> usize {
    h.query_all_by_label(label).count()
}

/// 組の形（タブ・前のタブ・分け方）。描いた矩形に依らない。
fn shape(dock: &DockState<Tab>) -> Vec<String> {
    let mut out = Vec::new();
    for (path, node) in dock.iter_all_nodes() {
        let at = format!("{}/{}", path.surface.0, path.node.0);
        match node {
            Node::Empty => {}
            Node::Leaf(leaf) => {
                let tabs: Vec<&str> = leaf.tabs.iter().map(|t| t.key()).collect();
                out.push(format!("{at} leaf {tabs:?} active={}", leaf.active.0));
            }
            Node::Horizontal(s) => out.push(format!("{at} horizontal {:.4}", s.fraction)),
            Node::Vertical(s) => out.push(format!("{at} vertical {:.4}", s.fraction)),
        }
    }
    out
}

/// 主の面で、タブがいる組のタブ。
fn mates(dock: &DockState<Tab>, tab: Tab) -> Vec<Tab> {
    let (node, _) = dock.find_main_surface_tab(&tab).expect("タブ");
    match &dock.main_surface()[node] {
        Node::Leaf(leaf) => leaf.tabs.clone(),
        _ => unreachable!(),
    }
}

/// 3 つのタブを外した、前の版の並び。
fn without_new_tabs(mut dock: DockState<Tab>) -> DockState<Tab> {
    for tab in NEW_TABS {
        let path = dock.find_tab(&tab).expect("既定の並びにある");
        dock.remove_tab(path);
        assert!(dock.find_tab(&tab).is_none());
    }
    dock
}

/// 前の版（0.5.0）の既定の並び（3 つのタブが無い。左の列の上は サブツール・アセット・チャンネル、右上はテクスチャセットとナビゲーターだけ）。
fn previous_default() -> DockState<Tab> {
    use egui_dock::NodeIndex;
    let mut dock = DockState::new(vec![Tab::Canvas, Tab::View3d]);
    let surface = dock.main_surface_mut();
    let [center, left] = surface.split_left(
        NodeIndex::root(),
        0.21,
        vec![Tab::SubTools, Tab::Assets, Tab::Channels],
    );
    let [_, right] = surface.split_right(center, 0.764, vec![Tab::TextureSets, Tab::Navigator]);
    surface.split_below(left, 0.66, vec![Tab::Color, Tab::ColorSets]);
    let [_, layers] = surface.split_below(right, 0.24, vec![Tab::Layers, Tab::Log]);
    surface.split_below(layers, 0.45, vec![Tab::Properties, Tab::History]);
    dock
}

// ───────── 名前・既定の並び ─────────

#[test]
fn headless_the_new_panels_have_names_in_both_languages_and_stable_keys() {
    for (tab, key, ja, en) in [
        (
            Tab::ToolProperties,
            "tool_properties",
            "ツールプロパティ",
            "Tool Properties",
        ),
        (Tab::BrushSize, "brush_size", "ブラシサイズ", "Brush Size"),
        (Tab::Material, "material", "マテリアル", "Material"),
    ] {
        assert_eq!(tab.key(), key);
        assert_eq!(Tab::from_key(key), Some(tab));
        assert_eq!(tab.title_in(Lang::Ja), ja);
        assert_eq!(tab.title_in(Lang::En), en);
        assert!(Tab::ALL.contains(&tab));
    }
}

#[test]
fn headless_the_default_dock_keeps_the_sub_tools_alone_on_the_left_and_the_assets_with_the_texture_sets(
) {
    let dock = default_dock();
    // 左の列: サブツールの組はサブツールだけ（アセット・チャンネルは右上へ）。ツールプロパティとブラシサイズは、それぞれ自分の組
    assert_eq!(mates(&dock, Tab::SubTools), [Tab::SubTools]);
    assert_eq!(mates(&dock, Tab::ToolProperties), [Tab::ToolProperties]);
    assert_eq!(mates(&dock, Tab::BrushSize), [Tab::BrushSize]);
    assert_eq!(mates(&dock, Tab::Color), [Tab::Color, Tab::ColorSets]);
    // 右: 上はテクスチャセット・チャンネル・アセット（ナビゲーターは閉じている）、次はレイヤーとログ、下はプロパティ・マテリアル・ヒストリー
    assert_eq!(
        mates(&dock, Tab::Assets),
        [Tab::TextureSets, Tab::Channels, Tab::Assets]
    );
    assert!(
        dock.find_tab(&Tab::Navigator).is_none(),
        "ナビゲーターは閉じている"
    );
    assert_eq!(mates(&dock, Tab::Layers), [Tab::Layers, Tab::Log]);
    assert_eq!(
        mates(&dock, Tab::Material),
        [Tab::Properties, Tab::Material, Tab::History],
        "右下のプロパティ・ヒストリーと並ぶ"
    );
    // 中央: 3D ビューとキャンバスは別の組で、3D ビューが左（同じ分け方の左の子）
    assert_eq!(mates(&dock, Tab::View3d), [Tab::View3d]);
    assert_eq!(mates(&dock, Tab::Canvas), [Tab::Canvas]);
    let view3d = dock.find_tab(&Tab::View3d).unwrap().node_path();
    let canvas = dock.find_tab(&Tab::Canvas).unwrap().node_path();
    assert_eq!(view3d.node.parent(), canvas.node.parent(), "同じ分け方の子");
    assert!(
        view3d.node.0 % 2 == 1 && canvas.node.0 == view3d.node.0 + 1,
        "3D ビューが左（先の子）"
    );
    layout::validate(&dock).expect("どのタブも 1 つずつ（ナビゲーターは無くてよい）");
    // ファイルで往復する
    let loaded = layout::parse(&layout::render(&dock, None));
    assert_eq!(loaded.problems, Vec::<String>::new());
    assert_eq!(shape(&loaded.dock.unwrap()), shape(&dock));
    // 前の版の並びのナビゲーターは、そのまま残る（閉じない）。ナビゲーターの無い並びには足さない
    let mut old = previous_default();
    layout::add_missing_tabs(&mut old, &mut []);
    assert!(
        old.find_tab(&Tab::Navigator).is_some(),
        "前の並びのナビゲーターは残る"
    );
    let mut without = default_dock();
    layout::add_missing_tabs(&mut without, &mut []);
    assert!(
        without.find_tab(&Tab::Navigator).is_none(),
        "閉じている並びには足さない"
    );
    // 開くと、テクスチャセットの組へ（前の既定の並びの組）
    assert_eq!(
        yolu_app::detach::default_mates(Tab::Navigator),
        [Tab::TextureSets]
    );
}

/// 3 つの大きさのウィンドウで、既定の並びの各パネルが使える高さ・幅になっている。
/// - 左の列は上から サブツール・ツールプロパティ・ブラシサイズ・カラー。
/// - 右上の 3 つのタブ（テクスチャセット・チャンネル・アセット）は、最小のウィンドウでも日英とも切れない。
/// - チャンネルは 6 つとも見え、アセットは品が見える（最小のウィンドウは上の一部だけ）。レイヤーの組が狭くなりすぎない。
#[test]
fn the_default_window_gives_every_panel_the_room_it_needs_at_the_three_sizes_in_both_languages() {
    for lang in Lang::ALL {
        for (width, height) in [(960.0, 640.0), (1280.0, 800.0), (1600.0, 900.0)] {
            let what = format!("{lang:?} {width}x{height}");
            let mut h = app_default(width, height, 128);
            language(&mut h, lang);
            // 左の列
            let column: Vec<Rect> = [
                Tab::SubTools,
                Tab::ToolProperties,
                Tab::BrushSize,
                Tab::Color,
            ]
            .iter()
            .map(|t| tab_rect(&h, *t))
            .collect();
            for pair in column.windows(2) {
                assert!(pair[0].top() < pair[1].top(), "{what}: 上から順 {column:?}");
                assert!(
                    (pair[0].left() - pair[1].left()).abs() < 2.0,
                    "{what}: 同じ列 {column:?}"
                );
            }
            // 右上の 3 つのタブは、同じ帯に並び、右の端（ウィンドウの右）で切れない
            let top: Vec<Rect> = [Tab::TextureSets, Tab::Channels, Tab::Assets]
                .iter()
                .map(|t| tab_rect(&h, *t))
                .collect();
            for pair in top.windows(2) {
                assert!((pair[0].top() - pair[1].top()).abs() < 1.0, "{what}");
                assert!(pair[0].right() <= pair[1].left() + 1.0, "{what}");
            }
            assert!(
                top[2].right() <= width - 2.0,
                "{what}: 右上のタブが切れない {top:?}"
            );
            // 中央: 3D ビューが左・キャンバスが右で、どちらも使える幅（最小のウィンドウで 240 点以上）
            let view3d = h.state().view3d_rect().expect("3D ビューを描いた");
            let canvas = canvas_rect(&h);
            assert!(
                view3d.right() <= canvas.left(),
                "{what}: 3D ビューが左 {view3d:?} {canvas:?}"
            );
            assert!(
                view3d.width() >= 240.0 && canvas.width() >= 240.0,
                "{what}: 中央の幅 {view3d:?} {canvas:?}"
            );
            // 右の列は前の幅（1280 で約 230）に近い: 1280 で 250 点以下
            if width == 1280.0 {
                assert!(
                    width - tab_rect(&h, Tab::TextureSets).left() <= 250.0,
                    "{what}: 右の列の幅"
                );
            }
            // 右下: プロパティ・マテリアル・ヒストリー
            let right: Vec<Rect> = [Tab::Properties, Tab::Material, Tab::History]
                .iter()
                .map(|t| tab_rect(&h, *t))
                .collect();
            assert!((right[0].top() - right[1].top()).abs() < 1.0, "{what}");
            assert!(
                right[2].right() <= width - 2.0,
                "{what}: ヒストリーが切れない"
            );
            // レイヤーの組: 狭くなりすぎない
            assert!(
                layers_body(&h).height() >= 100.0,
                "{what}: レイヤーの組 {:?}",
                layers_body(&h)
            );
            // チャンネル: 6 つの行が全部見える
            click_tab(&mut h, Tab::Channels);
            let body = top_right_body(&h);
            for channel in yolu_app::matpaint::CHANNELS {
                let name = yolu_app::m2::channel_name(lang, &st(&h).doc, channel);
                let row = rect_of(&h, &name, |r| body.contains(r.center()));
                assert!(
                    row.bottom() <= body.bottom(),
                    "{what}: チャンネル {name} が見える {row:?} {body:?}"
                );
            }
            // アセット: 品が見える（高さの足りない最小のウィンドウは、品の上の部分だけでもよい）
            click_tab(&mut h, Tab::Assets);
            h.state_mut().state.shelf.wait_inspections();
            h.run();
            let first = yolu_core::smart_library::entries()
                .first()
                .map(|e| e.name(lang == Lang::Ja).to_owned())
                .expect("同梱の素材");
            let body = top_right_body(&h);
            let card = rect_of(&h, &first, |r| {
                body.contains(r.center()) && r.top() > body.top() + 67.0
            });
            if height >= 800.0 {
                assert!(
                    card.bottom() <= body.bottom(),
                    "{what}: アセットの 1 段目が見える {card:?} {body:?}"
                );
            } else {
                assert!(
                    card.top() + 20.0 <= body.bottom(),
                    "{what}: アセットの品の上が見える {card:?} {body:?}"
                );
            }
        }
    }
}

/// 左の列: 1280×800 でブラシサイズの 1 段目の数字が見え、「塗るチャンネル」の見出しが切れない。1280×1000 ではブラシサイズの丸が 2 段とも見える。
#[test]
fn the_default_left_column_shows_the_size_numbers_and_the_paint_channels_header() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 800.0, 128);
        language(&mut h, lang);
        let (props, size, color) = (
            tab_rect(&h, Tab::ToolProperties),
            tab_rect(&h, Tab::BrushSize),
            tab_rect(&h, Tab::Color),
        );
        let props_body =
            Rect::from_min_max(props.left_bottom(), pos2(props.left() + 300.0, size.top()));
        let header = rect_of(
            &h,
            lang.pick("塗るチャンネル", "Paint Channels"),
            |r| props_body.contains(r.center()),
        );
        assert!(
            header.bottom() <= props_body.bottom(),
            "{lang:?}: 見出しが切れない {header:?} {props_body:?}"
        );
        let size_body =
            Rect::from_min_max(size.left_bottom(), pos2(size.left() + 300.0, color.top()));
        for px in [1u32, 2, 3, 5, 8, 12, 16] {
            let cell = rect_of(&h, &format!("{px} px"), |r| size_body.contains(r.center()));
            assert!(
                size_body.contains_rect(cell),
                "{lang:?}: {px} の丸と数字が見える {cell:?} {size_body:?}"
            );
        }
        let mut tall = app(1280.0, 1000.0, 128);
        language(&mut tall, lang);
        let (size, color) = (tab_rect(&tall, Tab::BrushSize), tab_rect(&tall, Tab::Color));
        let size_body =
            Rect::from_min_max(size.left_bottom(), pos2(size.left() + 300.0, color.top()));
        for px in yolu_app::panels::brushes::SIZES {
            let cell = rect_of(&tall, &format!("{px} px"), |r| {
                size_body.contains(r.center())
            });
            assert!(
                size_body.contains_rect(cell),
                "{lang:?}: 大きいウィンドウでは丸が 2 段とも見える: {px} {cell:?} {size_body:?}"
            );
        }
    }
}

/// カラーのパネル: 円は欄の幅と高さの小さいほうをいっぱいに使い（選ぶ印の輪が円の外へ出る分は余白に取り、欄の上下左右からはみ出さない）、
/// 切り替えのボタンは円と印の外側の角、16 進とアルファは 1 段で小さく、その下に空きが無い（使った色が無いとき。あるときはその行の下も空かない）。
/// 円の直径は、前の版（82bae7e6）の 960×640 で 107、1280×800 で 158、1600×900 で 190 より小さくならない。
#[test]
fn the_color_wheel_fills_the_panel_without_the_marker_leaving_it_and_nothing_is_left_empty_below() {
    use yolu_app::panels::color::marker_overhang;
    for lang in Lang::ALL {
        for (width, height, least) in [
            (960.0, 640.0, 107.0),
            (1280.0, 800.0, 158.0),
            (1600.0, 900.0, 190.0),
        ] {
            let what = format!("{lang:?} {width}x{height}");
            let mut h = app_default(width, height, 128);
            language(&mut h, lang);
            let tab = tab_rect(&h, Tab::Color);
            // パネルの中身（タブの帯の下から、状態の帯の上まで。右は中央との区切り）
            let panel = Rect::from_min_max(
                tab.left_bottom(),
                pos2(
                    tab_rect(&h, Tab::View3d).left() - 2.0,
                    h.ctx.content_rect().bottom() - 22.0,
                ),
            );
            let wheel = h.get_by_label(lang.pick("色相の円", "Hue wheel")).rect();
            assert!(
                (wheel.width() - wheel.height()).abs() < 0.5,
                "{what}: 円は正方形 {wheel:?}"
            );
            assert!(
                wheel.width() >= least,
                "{what}: 円の直径 {} が小さい（前の版以上）",
                wheel.width()
            );
            // 選ぶ印の輪（円の真上・真下・真左・真右の外へ出る分）も、欄の中に収まる
            let reach = marker_overhang(wheel.width());
            let marker_box = wheel.expand(reach);
            assert!(
                panel.expand(0.5).contains_rect(marker_box),
                "{what}: 選ぶ印の輪が欄の中 {marker_box:?} {panel:?}"
            );
            let fields = h
                .query_all_by_value("#000000")
                .map(|n| n.rect())
                .find(|r| panel.contains(r.center()))
                .expect("16 進の欄");
            let alpha = rect_of(&h, "A", |r| panel.contains(r.center()));
            assert!(
                (fields.center().y - alpha.center().y).abs() < 1.5,
                "{what}: 16 進とアルファは 1 段 {fields:?} {alpha:?}"
            );
            assert!(
                alpha.height() <= 20.0,
                "{what}: 欄の高さ {}",
                alpha.height()
            );
            // 円は欄の幅と高さの小さいほうをいっぱいに使う（高さは、16 進とアルファの上まで）
            let free_height = alpha.top() - 3.0 - panel.top();
            let free_width = panel.width() - 16.0;
            let side = free_height.min(free_width);
            assert!(
                (marker_box.width() - side).abs() <= 4.0,
                "{what}: 円が欄いっぱい {} / 高さ {free_height} 幅 {free_width}",
                marker_box.width()
            );
            // 16 進とアルファの段の下に、行の間（3 点）より大きな空きが無く、段が欄からあふれてもいない
            let blank = panel.bottom() - alpha.bottom();
            assert!(
                (-0.5..=3.5).contains(&blank),
                "{what}: 段の下の空き {blank}"
            );
            // 切り替えのボタンは円と印の輪に掛からない
            let toggle = h
                .get_by_label(lang.pick("四角と色相の帯", "Square and hue bar"))
                .rect();
            let c = wheel.center();
            let nearest = pos2(
                c.x.clamp(toggle.left(), toggle.right()),
                c.y.clamp(toggle.top(), toggle.bottom()),
            );
            assert!(
                nearest.distance(c) >= wheel.width() * 0.5 + reach,
                "{what}: 切り替えのボタンが円の外 {toggle:?} {wheel:?}"
            );
            // 使った色が入ると、その行の分だけ円が小さくなり、その行の下も空かない
            h.state_mut().state.color.remember();
            h.run();
            let smaller = h.get_by_label(lang.pick("色相の円", "Hue wheel")).rect();
            assert!(
                smaller.width() < wheel.width() && smaller.width() >= wheel.width() - 20.0,
                "{what}: 使った色の行の分だけ小さい {} → {}",
                wheel.width(),
                smaller.width()
            );
            let swatch = h
                .query_all_by_label_contains("#000000FF")
                .map(|n| n.rect())
                .find(|r| panel.contains(r.center()))
                .expect("使った色の見本");
            assert!(
                panel.bottom() - swatch.bottom() <= 3.5,
                "{what}: 使った色の行の下の空き {}",
                panel.bottom() - swatch.bottom()
            );
        }
    }
    // 高さに余りがあるウィンドウでは、円は幅いっぱい（選ぶ印の輪の分を除く。切り替えのボタンの分は空けない）
    let h = app_default(1280.0, 1400.0, 128);
    let tab = tab_rect(&h, Tab::Color);
    let panel_width = tab_rect(&h, Tab::View3d).left() - 2.0 - tab.left();
    let wheel = h.get_by_label("色相の円").rect();
    let box_width = wheel.width() + 2.0 * yolu_app::panels::color::marker_overhang(wheel.width());
    assert!(
        (box_width - (panel_width - 16.0)).abs() < 1.5,
        "幅いっぱい {box_width} / {}",
        panel_width - 16.0
    );
}

// ───────── 開く・外へ出す・戻す・「ウィンドウ」のメニュー ─────────

#[test]
fn the_window_menu_lists_the_new_panels_and_opens_one_that_is_nowhere() {
    let mut h = app(1600.0, 900.0, 128);
    for lang in Lang::ALL {
        language(&mut h, lang);
        let entries = yolu_app::shell::menu_entries(st(&h), yolu_app::shell::WINDOW_MENU);
        let labels: Vec<&str> = entries.iter().filter_map(|e| e.label()).collect();
        for tab in NEW_TABS {
            assert!(
                labels.contains(&tab.title_in(lang)),
                "{lang:?}: {tab:?} は「ウィンドウ」にある"
            );
        }
    }
    // どこにも無いパネルは、「ウィンドウ」から開くと既定の並びの場所へ入る（前に出る）
    for (tab, with) in [
        (Tab::ToolProperties, Tab::SubTools),
        (Tab::BrushSize, Tab::SubTools),
        (Tab::Material, Tab::Properties),
    ] {
        let dock = &mut h.state_mut().dock;
        let path = dock.find_tab(&tab).unwrap();
        dock.remove_tab(path);
        h.run();
        assert!(!h.state().state.ui.panels.open.contains(&tab));
        h.state_mut().state.apply(Action::Dock(DockOp::Show(tab)));
        h.run();
        assert!(h.state().state.ui.panels.open.contains(&tab), "{tab:?}");
        let dock = &h.state().dock;
        assert!(
            mates(dock, tab).contains(&with),
            "{tab:?} は {with:?} の組へ"
        );
        let leaf_index = dock.find_main_surface_tab(&tab).unwrap();
        match &dock.main_surface()[leaf_index.0] {
            Node::Leaf(leaf) => assert_eq!(leaf.tabs[leaf.active.0], tab, "前に出る"),
            _ => unreachable!(),
        }
        layout::validate(dock).expect("どのタブも 1 つずつ");
    }
}

/// 起動して並びの保存が無いと、最初のフレームで、右の列の幅がウィンドウの幅に合う（幅の 19%、ただし 232 点より狭くしない）。
/// 保存した並びを読んだときと、試験が並びを替えたときは、そのまま。
#[test]
fn a_fresh_start_fits_the_right_column_to_the_window_width_and_a_saved_arrangement_is_left_alone() {
    for (width, height) in [(960.0, 640.0), (1280.0, 800.0), (1600.0, 900.0)] {
        let dir = settings_dir("fit");
        let settings = dir.join("settings.conf");
        let mut h = gpu_thread::builder()
            .with_size(vec2(width, height))
            .with_pixels_per_point(1.0)
            .with_step_dt(1.0 / 60.0)
            .with_max_steps(120)
            .renderer(shared_gpu::renderer())
            .build_eframe(move |cc| {
                with_render_state_cpu_canvas(
                    YoluApp::for_context_with_settings(
                        &cc.egui_ctx,
                        Some(settings),
                        PenInput::detached(),
                    ),
                    cc.wgpu_render_state.as_ref(),
                )
            });
        h.run();
        assert_eq!(
            shape(&h.state().dock),
            shape(&yolu_app::app::default_dock_for(width)),
            "{width}: 幅に合わせた既定の並び"
        );
        let right = width - tab_rect(&h, Tab::TextureSets).left();
        let want = (0.19 * width).max(232.0);
        assert!(
            (right - want).abs() <= 3.0,
            "{width}: 右の列の幅 {right}（{want} のはず）"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
    // 保存した並び（前の版の既定）は、幅に合わせ直さない
    let dir = settings_dir("fit-saved");
    std::fs::write(
        dir.join(layout::FILE_NAME),
        layout::render(&previous_default(), None),
    )
    .unwrap();
    let settings = dir.join("settings.conf");
    let mut h = gpu_thread::builder()
        .with_size(vec2(1280.0, 800.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .renderer(shared_gpu::renderer())
        .build_eframe(move |cc| {
            with_render_state_cpu_canvas(
                YoluApp::for_context_with_settings(
                    &cc.egui_ctx,
                    Some(settings),
                    PenInput::detached(),
                ),
                cc.wgpu_render_state.as_ref(),
            )
        });
    h.run();
    let mut want = previous_default();
    layout::add_missing_tabs(&mut want, &mut []);
    assert_eq!(
        shape(&h.state().dock),
        shape(&want),
        "保存した並びは今までの形のまま"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// 設定のフォルダ `dir` で、`width` × `height` のウィンドウに立てたアプリ（並びのファイルがあれば読む）。
fn app_in_settings(dir: &Path, width: f32, height: f32) -> H {
    let settings = dir.join("settings.conf");
    let mut h = gpu_thread::builder()
        .with_size(vec2(width, height))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .renderer(shared_gpu::renderer())
        .build_eframe(move |cc| {
            with_render_state_cpu_canvas(
                YoluApp::for_context_with_settings(
                    &cc.egui_ctx,
                    Some(settings),
                    PenInput::detached(),
                ),
                cc.wgpu_render_state.as_ref(),
            )
        });
    h.run();
    h
}

/// 右の列（テクスチャセットの組の左端から右端まで）の幅。
fn right_width(h: &H, width: f32) -> f32 {
    width - tab_rect(h, Tab::TextureSets).left()
}

/// 並びを動かしていない間は、ウィンドウの幅が変わると右の列の割合が追う（下限 232 点を保つ）。新しくその幅で起動した並びと同じ（1 点以内）。
#[test]
fn an_untouched_arrangement_follows_the_window_width() {
    for (start, end) in [(1600.0, 960.0), (960.0, 1920.0), (1280.0, 1281.0)] {
        let dir = settings_dir("follow");
        let mut h = app_in_settings(&dir, start, 900.0);
        h.set_size(vec2(end, 900.0));
        h.run();
        h.run();
        let fresh_dir = settings_dir("follow-fresh");
        let fresh = app_in_settings(&fresh_dir, end, 900.0);
        assert_eq!(
            shape(&h.state().dock),
            shape(&yolu_app::app::default_dock_for(end)),
            "{start}→{end}: 今の幅の既定の割合"
        );
        let (now, want) = (right_width(&h, end), right_width(&fresh, end));
        assert!(
            (now - want).abs() <= 1.0,
            "{start}→{end}: 右の列 {now}（新しく起動すると {want}）"
        );
        assert!(now >= 232.0 - 1.0, "{start}→{end}: 下限 {now}");
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(fresh_dir);
    }
}

/// 利用者が仕切りを動かした・タブを動かした・別ウィンドウに出した後は、幅を変えても割合を直さない。ナビゲーターを開いても自動のまま。
#[test]
fn the_follow_stops_once_the_user_changes_the_arrangement() {
    // 仕切りを動かす
    let dir = settings_dir("follow-moved");
    let mut h = app_in_settings(&dir, 1600.0, 900.0);
    if let Node::Horizontal(split) | Node::Vertical(split) =
        &mut h.state_mut().dock.main_surface_mut()[egui_dock::NodeIndex(0)]
    {
        split.fraction += 0.03;
    } else {
        panic!("根は分け目");
    }
    h.run();
    let moved = shape(&h.state().dock);
    h.set_size(vec2(1100.0, 900.0));
    h.run();
    h.run();
    assert_eq!(
        shape(&h.state().dock),
        moved,
        "仕切りを動かした後は割合を変えない"
    );
    // タブを動かす
    let dir2 = settings_dir("follow-tab");
    let mut h = app_in_settings(&dir2, 1600.0, 900.0);
    let from = h.state().dock.find_tab(&Tab::Assets).unwrap();
    let to = h.state().dock.find_tab(&Tab::Layers).unwrap().node_path();
    h.state_mut().dock.move_tab(
        from,
        egui_dock::TabDestination::Node(to, egui_dock::TabInsert::Append),
    );
    h.run();
    let moved = shape(&h.state().dock);
    h.set_size(vec2(1100.0, 900.0));
    h.run();
    h.run();
    assert_eq!(
        shape(&h.state().dock),
        moved,
        "タブを動かした後は割合を変えない"
    );
    // 別ウィンドウ
    let dir3 = settings_dir("follow-detached");
    let mut h = app_in_settings(&dir3, 1600.0, 900.0);
    h.state_mut()
        .state
        .apply(Action::Dock(DockOp::Detach(Tab::Log)));
    h.run();
    let moved = shape(&h.state().dock);
    h.set_size(vec2(1100.0, 900.0));
    h.run();
    h.run();
    assert_eq!(
        shape(&h.state().dock),
        moved,
        "別ウィンドウがあれば割合を変えない"
    );
    // ナビゲーターを開いても自動のまま（選んでいるタブも保つ）
    let dir4 = settings_dir("follow-navigator");
    let mut h = app_in_settings(&dir4, 1600.0, 900.0);
    h.state_mut()
        .state
        .apply(Action::Dock(DockOp::Show(Tab::Navigator)));
    h.run();
    h.set_size(vec2(1100.0, 900.0));
    h.run();
    h.run();
    let dock = &h.state().dock;
    let (node, index) = dock.find_main_surface_tab(&Tab::Navigator).unwrap();
    match &dock.main_surface()[node] {
        Node::Leaf(leaf) => assert_eq!(leaf.active.0, index.0, "選んでいるタブを保つ"),
        _ => unreachable!(),
    }
    assert!((right_width(&h, 1100.0) - (0.19f32 * 1100.0).max(232.0)).abs() <= 3.0);
    for d in [dir, dir2, dir3, dir4] {
        let _ = std::fs::remove_dir_all(d);
    }
}

/// 並びのファイルの `auto_fit`: 自動のまま閉じると true を書き、読むと今の幅へ合わせ直す。false・無い（前の版のファイル）は合わせない。
/// 仕切りを動かして閉じると false。
#[test]
fn auto_fit_is_written_when_closed_untouched_and_read_back() {
    use eframe::App;
    let narrow = yolu_app::app::default_dock_for(1600.0);
    // true: 今の幅に合わせ直す
    let dir = settings_dir("auto-true");
    std::fs::write(
        dir.join(layout::FILE_NAME),
        layout::render_full(&narrow, None, &[], &[], true),
    )
    .unwrap();
    let h = app_in_settings(&dir, 1000.0, 800.0);
    assert_eq!(
        shape(&h.state().dock),
        shape(&yolu_app::app::default_dock_for(1000.0)),
        "auto_fit: true は今の幅へ"
    );
    // false と、キーの無い前の版のファイルは、合わせない
    for (tag, text) in [
        (
            "auto-false",
            layout::render_full(&narrow, None, &[], &[], false),
        ),
        ("auto-absent", layout::render(&narrow, None)),
    ] {
        let dir = settings_dir(tag);
        assert!(!text.contains("auto_fit"), "{tag}: false は書かない");
        std::fs::write(dir.join(layout::FILE_NAME), text).unwrap();
        let mut h = app_in_settings(&dir, 1000.0, 800.0);
        assert_eq!(shape(&h.state().dock), shape(&narrow), "{tag}: 合わせない");
        h.set_size(vec2(1300.0, 800.0));
        h.run();
        h.run();
        assert_eq!(
            shape(&h.state().dock),
            shape(&narrow),
            "{tag}: 幅が変わっても合わせない"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
    // 自動のまま閉じると true を書く。仕切りを動かして閉じると書かない
    let dir = settings_dir("auto-write");
    let mut h = app_in_settings(&dir, 1400.0, 800.0);
    h.state_mut().on_exit();
    let text = std::fs::read_to_string(dir.join(layout::FILE_NAME)).unwrap();
    assert!(layout::parse(&text).auto_fit, "自動のまま閉じる: true");
    if let Node::Horizontal(split) | Node::Vertical(split) =
        &mut h.state_mut().dock.main_surface_mut()[egui_dock::NodeIndex(0)]
    {
        split.fraction += 0.03;
    }
    h.run();
    h.state_mut().on_exit();
    let text = std::fs::read_to_string(dir.join(layout::FILE_NAME)).unwrap();
    assert!(!layout::parse(&text).auto_fit, "動かして閉じる: false");
    let _ = std::fs::remove_dir_all(dir);
}

/// ナビゲーターは既定の並びでは閉じていて、「ウィンドウ」に名前があり、開くとテクスチャセットの組へ入って前に出る。
#[test]
fn the_navigator_is_closed_in_the_default_dock_and_opens_into_the_texture_sets_group_from_the_window_menu(
) {
    let mut h = app_default(1280.0, 800.0, 128);
    for lang in Lang::ALL {
        language(&mut h, lang);
        let entries = yolu_app::shell::menu_entries(st(&h), yolu_app::shell::WINDOW_MENU);
        assert!(
            entries
                .iter()
                .any(|e| e.label() == Some(Tab::Navigator.title_in(lang))),
            "{lang:?}: 「ウィンドウ」にナビゲーターがある"
        );
    }
    assert!(h.state().dock.find_tab(&Tab::Navigator).is_none());
    assert!(!h.state().state.ui.panels.open.contains(&Tab::Navigator));
    h.state_mut()
        .state
        .apply(Action::Dock(DockOp::Show(Tab::Navigator)));
    h.run();
    assert!(h.state().state.ui.panels.open.contains(&Tab::Navigator));
    let dock = &h.state().dock;
    assert!(mates(dock, Tab::Navigator).contains(&Tab::TextureSets));
    let leaf_index = dock.find_main_surface_tab(&Tab::Navigator).unwrap();
    match &dock.main_surface()[leaf_index.0] {
        Node::Leaf(leaf) => assert_eq!(leaf.tabs[leaf.active.0], Tab::Navigator, "前に出る"),
        _ => unreachable!(),
    }
    layout::validate(dock).expect("どのタブも 1 つずつ");
    // 開いた並びは、ファイルへ往復する
    let loaded = layout::parse(&layout::render(dock, None));
    assert_eq!(loaded.problems, Vec::<String>::new());
    assert_eq!(shape(&loaded.dock.unwrap()), shape(dock));
}

#[test]
fn each_new_panel_can_go_to_its_own_window_and_come_back_to_where_it_was() {
    for tab in NEW_TABS {
        let mut h = app(1600.0, 900.0, 128);
        let mates_before = mates(&h.state().dock, tab);
        h.state_mut().state.apply(Action::Dock(DockOp::Detach(tab)));
        h.run();
        h.step();
        let app = h.state();
        assert!(
            app.dock.find_tab(&tab).is_none(),
            "{tab:?}: ドックから外れた"
        );
        assert_eq!(app.detached.windows.len(), 1, "{tab:?}");
        assert_eq!(app.detached.windows[0].tabs(), vec![tab]);
        assert!(app.state.ui.panels.outside.contains(&tab));
        layout::validate_all(&app.dock, &[&app.detached.windows[0].dock])
            .expect("どのタブも 1 つずつ");
        // 別ウィンドウにも、そのパネルの中身が出る（試験では、メインウィンドウの中の egui のウィンドウ）
        let window = h.state().detached.windows[0].tab_rects[&tab];
        let label = match tab {
            Tab::ToolProperties => "硬さ",
            Tab::BrushSize => "16 px",
            _ => "種類: lilToon",
        };
        assert!(
            h.query_all_by_label(label).any(
                |n| n.rect().top() > window.bottom() && n.rect().left() >= window.left() - 40.0
            ),
            "{tab:?}: 中身が別ウィンドウに出る"
        );
        // 「ドックに戻す」で、出す前に同じ組にいたタブの組へ。1 つだけの組にいた物は、既定の並びで上にあるサブツールの組へ
        h.state_mut().state.apply(Action::Dock(DockOp::Return(tab)));
        h.run();
        let app = h.state();
        assert!(app.detached.windows.is_empty());
        let mut after = mates(&app.dock, tab);
        match tab {
            Tab::Material => {
                let mut was = mates_before.clone();
                after.sort_by_key(|t| t.key());
                was.sort_by_key(|t| t.key());
                assert_eq!(after, was, "{tab:?}: 元の組へ");
            }
            _ => assert!(
                after.contains(&Tab::SubTools),
                "{tab:?}: サブツールの組へ {after:?}"
            ),
        }
        layout::validate(&app.dock).expect("どのタブも 1 つずつ");
    }
}

/// 「ウィンドウ」のメニューの、タブの項目: チェックが付いているか・押したときの操作。
fn window_item(h: &H, tab: Tab) -> (bool, Action) {
    let lang = st(h).lang;
    let entries = yolu_app::shell::menu_entries(st(h), yolu_app::shell::WINDOW_MENU);
    for entry in entries {
        if let yolu_app::ui::menu::Entry::Item {
            label,
            check,
            action,
            ..
        } = entry
        {
            if label == tab.title_in(lang) {
                return (matches!(check, yolu_app::ui::menu::Check::Checked), action);
            }
        }
    }
    panic!("{tab:?} が「ウィンドウ」に無い");
}

/// ナビゲーターとアクションは、「ウィンドウ」のメニューで開いている間はチェックが付き、押すと閉じる（どこにも無くなる）。閉じていれば押すと開く
/// （ナビゲーターはテクスチャセットの組、アクションはレイヤーの組）。ほかのパネルは押しても閉じない。
#[test]
fn the_window_menu_toggles_the_navigator_and_the_actions_panel() {
    // 閉じられるのは、後の版で足してよいタブ（`OPTIONAL_TABS`）のうち、既定の並びに無いものだけ
    for tab in layout::HIDEABLE {
        assert!(layout::OPTIONAL_TABS.contains(&tab), "{tab:?}");
        assert!(default_dock().find_tab(&tab).is_none(), "{tab:?}");
    }
    let mut h = app_default(1280.0, 800.0, 128);
    for (tab, with) in [
        (Tab::Navigator, Tab::TextureSets),
        (Tab::Actions, Tab::Layers),
    ] {
        let (checked, action) = window_item(&h, tab);
        assert!(!checked, "{tab:?}: 閉じている");
        assert_eq!(action, Action::Dock(DockOp::Show(tab)));
        h.state_mut().state.apply(action);
        h.run();
        assert!(mates(&h.state().dock, tab).contains(&with), "{tab:?}");
        let (checked, action) = window_item(&h, tab);
        assert!(checked, "{tab:?}: 開くとチェックが付く");
        assert_eq!(action, Action::Dock(DockOp::Hide(tab)));
        h.state_mut().state.apply(action);
        h.run();
        assert!(h.state().dock.find_tab(&tab).is_none(), "{tab:?}: 閉じた");
        assert!(!h.state().state.ui.panels.open.contains(&tab));
        assert!(!window_item(&h, tab).0, "{tab:?}: チェックが外れる");
        layout::validate(&h.state().dock).expect("閉じても検証を通る");
        // また開くと、同じ組に入る
        h.state_mut().state.apply(Action::Dock(DockOp::Show(tab)));
        h.run();
        assert!(
            mates(&h.state().dock, tab).contains(&with),
            "{tab:?}: また開く"
        );
        // 別ウィンドウに出していても、メニューで閉じるとそのウィンドウごと消える
        h.state_mut().state.apply(Action::Dock(DockOp::Detach(tab)));
        h.run();
        h.step();
        assert_eq!(h.state().detached.windows.len(), 1, "{tab:?}");
        let (checked, action) = window_item(&h, tab);
        assert!(checked, "{tab:?}: 別ウィンドウでもチェックが付く");
        h.state_mut().state.apply(action);
        h.run();
        assert!(
            h.state().detached.windows.is_empty(),
            "{tab:?}: ウィンドウが消える"
        );
        assert!(!h.state().state.ui.panels.open.contains(&tab));
    }
    // 閉じられないパネルは、開いていても押すと前に出す（Hide にならない）
    let (_, action) = window_item(&h, Tab::Layers);
    assert_eq!(action, Action::Dock(DockOp::Show(Tab::Layers)));
    let mut headless = default_dock();
    let mut outside = yolu_app::detach::Detached::new();
    assert!(!outside.hide(&mut headless, Tab::Layers));
    assert!(headless.find_tab(&Tab::Layers).is_some());
    assert!(!outside.hide(&mut headless, Tab::Navigator), "どこにも無い");
}

/// 閉じたナビゲーターは、並びを保存して読み直しても閉じたまま（開いたまま閉じれば開いたまま）。
#[test]
fn a_closed_navigator_stays_closed_after_the_arrangement_is_saved_and_read_back() {
    use eframe::App;
    let dir = settings_dir("navigator-closed");
    let mut h = app_in_settings(&dir, 1280.0, 800.0);
    h.state_mut()
        .state
        .apply(Action::Dock(DockOp::Show(Tab::Navigator)));
    h.run();
    h.state_mut().on_exit();
    drop(h);
    let mut h = app_in_settings(&dir, 1280.0, 800.0);
    assert!(
        h.state().dock.find_tab(&Tab::Navigator).is_some(),
        "開いたまま閉じれば開いたまま"
    );
    h.state_mut()
        .state
        .apply(Action::Dock(DockOp::Hide(Tab::Navigator)));
    h.run();
    h.state_mut().on_exit();
    drop(h);
    let h = app_in_settings(&dir, 1280.0, 800.0);
    assert!(
        h.state().dock.find_tab(&Tab::Navigator).is_none(),
        "閉じたまま"
    );
    layout::validate(&h.state().dock).expect("検証を通る");
    let _ = std::fs::remove_dir_all(dir);
}

/// 既定の並びで 1 枚だけの組だったタブ（3D ビュー・キャンバス・サブツール）と、戻したあとに隣になるはずのタブ・向き。
const SINGLE_GROUPS: [(Tab, Tab, char); 3] = [
    (Tab::View3d, Tab::Canvas, 'l'),
    (Tab::Canvas, Tab::View3d, 'r'),
    (Tab::SubTools, Tab::ToolProperties, 'a'),
];

/// タブが自分だけの組にいて、隣のタブの組の左（l）・右（r）・上（a）にある（描いた矩形で比べる）。
fn assert_own_group_beside(dock: &DockState<Tab>, tab: Tab, neighbor: Tab, side: char, why: &str) {
    assert_eq!(mates(dock, tab), vec![tab], "{tab:?} {why}: 自分だけの組");
    let mine = yolu_app::detach::leaf_rect(dock, tab).expect("組の矩形");
    let other = yolu_app::detach::leaf_rect(dock, neighbor).expect("隣の組の矩形");
    let eps = 1.5;
    match side {
        'l' => assert!(
            mine.right() <= other.left() + eps,
            "{tab:?} {why}: 左 {mine:?} {other:?}"
        ),
        'r' => assert!(
            mine.left() >= other.right() - eps,
            "{tab:?} {why}: 右 {mine:?} {other:?}"
        ),
        _ => assert!(
            mine.bottom() <= other.top() + eps,
            "{tab:?} {why}: 上 {mine:?} {other:?}"
        ),
    }
    assert!(
        mine.width() > 20.0 && mine.height() > 20.0,
        "{tab:?} {why}: 描かれた {mine:?}"
    );
    layout::validate(dock).expect("どのタブも 1 つずつ");
}

#[test]
fn single_tab_groups_come_back_as_their_own_group_next_to_their_default_neighbor() {
    for (tab, neighbor, side) in SINGLE_GROUPS {
        // 「ドックに戻す」
        let mut h = app_default(1600.0, 900.0, 128);
        assert_own_group_beside(&h.state().dock, tab, neighbor, side, "出す前");
        h.state_mut().state.apply(Action::Dock(DockOp::Detach(tab)));
        h.run();
        h.step();
        assert_eq!(h.state().detached.windows.len(), 1, "{tab:?}");
        h.state_mut().state.apply(Action::Dock(DockOp::Return(tab)));
        h.run();
        h.run();
        assert!(h.state().detached.windows.is_empty());
        assert_own_group_beside(&h.state().dock, tab, neighbor, side, "戻す");
        // OS のウィンドウを閉じる道（`close`）
        h.state_mut().state.apply(Action::Dock(DockOp::Detach(tab)));
        h.run();
        h.step();
        let serial = h.state().detached.windows[0].serial;
        let app = h.state_mut();
        app.detached.close(&mut app.dock, serial);
        h.run();
        h.run();
        assert!(h.state().detached.windows.is_empty());
        assert_own_group_beside(&h.state().dock, tab, neighbor, side, "閉じる");
    }
}

/// 前の版の並びで「プロパティ」「レイヤー」が別ウィンドウにあるとき、足すマテリアル・ログは、そのウィンドウの同じ組の後ろへ入る（メインの最初の組へ入れない）。
#[test]
fn the_added_material_and_log_follow_properties_and_layers_into_their_windows() {
    let mut main = previous_default();
    let mut older = main.clone();
    for tab in [Tab::Properties, Tab::Layers] {
        assert!(older.find_tab(&tab).is_some(), "前の版の並びにある {tab:?}");
    }
    // 前の版には無かったマテリアル・ログを外した並びを作る（無ければそのまま）
    for tab in [Tab::Material, Tab::Log] {
        if let Some(path) = main.find_tab(&tab) {
            main.remove_tab(path);
        }
    }
    older = main.clone();
    let mut records = Vec::new();
    for tab in [Tab::Properties, Tab::Layers] {
        let mates: Vec<Tab> = {
            let (node, _) = older.find_main_surface_tab(&tab).unwrap();
            match &older.main_surface()[node] {
                Node::Leaf(leaf) => leaf.tabs.iter().copied().filter(|t| *t != tab).collect(),
                _ => unreachable!(),
            }
        };
        let path = main.find_tab(&tab).unwrap();
        main.remove_tab(path);
        records.push(DetachedRecord {
            dock: DockState::new(vec![tab]),
            window: None,
            home: mates,
        });
    }
    layout::add_missing_tabs(&mut main, &mut records);
    for (record, extra) in records.iter().zip([Tab::Material, Tab::Log]) {
        assert!(
            record.dock.find_tab(&extra).is_some(),
            "{extra:?} は {:?} のウィンドウへ",
            record
                .dock
                .iter_all_tabs()
                .map(|(_, t)| *t)
                .collect::<Vec<_>>()
        );
        assert!(
            main.find_tab(&extra).is_none(),
            "{extra:?} はメインへ入れない"
        );
    }
}

#[test]
fn detached_new_panels_are_written_to_the_layout_file_and_read_back() {
    let mut main = default_dock();
    let mut outside = yolu_app::detach::Detached::new();
    for (tab, x) in [(Tab::ToolProperties, 100.0), (Tab::Material, 500.0)] {
        assert!(outside.detach(
            &mut main,
            tab,
            Place::Record(FloatRecord {
                position: [x, 120.0],
                size: [360.0, 420.0],
                pixels_per_point: 1.0,
            })
        ));
    }
    let records: Vec<DetachedRecord> = outside
        .windows
        .iter()
        .map(|w| DetachedRecord {
            dock: w.dock.clone(),
            window: w.record,
            home: w.home.clone(),
        })
        .collect();
    let loaded = layout::parse(&layout::render_all(&main, None, &[], &records));
    assert_eq!(loaded.problems, Vec::<String>::new(), "並びを捨てない");
    assert_eq!(shape(&loaded.dock.expect("主のドック")), shape(&main));
    assert_eq!(loaded.detached.len(), 2);
    assert!(loaded.detached[0]
        .dock
        .find_tab(&Tab::ToolProperties)
        .is_some());
    assert_eq!(loaded.detached[1].home, vec![Tab::Properties, Tab::History]);
    // 別ウィンドウにあるタブは、足し直さない（主のドックへ重ねて足さない）
    let docks: Vec<&DockState<Tab>> = loaded.detached.iter().map(|d| &d.dock).collect();
    layout::validate_all(&main, &docks).expect("どのタブも 1 つずつ");
}

#[test]
fn headless_closing_the_windows_of_the_new_panels_returns_each_to_its_own_group() {
    let mut main = default_dock();
    let mut outside = yolu_app::detach::Detached::new();
    let at = |x: f32| {
        Place::Record(FloatRecord {
            position: [x, 100.0],
            size: [320.0, 400.0],
            pixels_per_point: 1.0,
        })
    };
    for (i, tab) in NEW_TABS.into_iter().enumerate() {
        assert!(outside.detach(&mut main, tab, at(100.0 + 340.0 * i as f32)));
    }
    assert_eq!(outside.windows.len(), 3);
    layout::validate_all(
        &main,
        &outside.windows.iter().map(|w| &w.dock).collect::<Vec<_>>(),
    )
    .expect("どのタブも 1 つずつ");
    // 全部を閉じる（OS のウィンドウを閉じるのと同じ）: 1 つだけの組にいたツールプロパティとブラシサイズは、既定の並びで上にあったサブツールの組へ、
    // マテリアルは、出したときに同じ組にいたプロパティの組へ
    while let Some(serial) = outside.windows.first().map(|w| w.serial) {
        outside.close(&mut main, serial);
    }
    layout::validate(&main).expect("どのタブも 1 つずつ");
    assert!(mates(&main, Tab::ToolProperties).contains(&Tab::SubTools));
    assert!(mates(&main, Tab::BrushSize).contains(&Tab::SubTools));
    assert!(mates(&main, Tab::Material).contains(&Tab::Properties));
    // 戻したあとも、ファイルで往復できる
    let loaded = layout::parse(&layout::render(&main, None));
    assert_eq!(loaded.problems, Vec::<String>::new());
    assert_eq!(shape(&loaded.dock.unwrap()), shape(&main));
}

// ───────── 前の layout.json（この 3 つが無い） ─────────

#[test]
fn headless_a_layout_from_before_the_new_panels_keeps_its_arrangement_and_gains_them_where_they_belong(
) {
    let old = previous_default();
    layout::validate(&old).expect("無くても使える並び");
    let text = layout::render(&old, None);
    assert!(
        !text.contains("tool_properties")
            && !text.contains("brush_size")
            && !text.contains("\"material\"")
    );
    let loaded = layout::parse(&text);
    assert_eq!(loaded.problems, Vec::<String>::new(), "並び全部は捨てない");
    let dock = loaded.dock.expect("読めた");
    layout::validate(&dock).expect("どのタブも 1 つずつ");
    // 足すのは無いタブだけ: 前からのタブは、組もその中の順もそのまま（アセット・チャンネルもサブツールの組のまま）
    assert_eq!(
        mates(&dock, Tab::SubTools),
        [Tab::SubTools, Tab::Assets, Tab::Channels]
    );
    assert_eq!(
        mates(&dock, Tab::TextureSets),
        [Tab::TextureSets, Tab::Navigator]
    );
    assert_eq!(
        mates(&dock, Tab::Properties),
        [Tab::Properties, Tab::Material, Tab::History],
        "マテリアルはプロパティのすぐ後ろ"
    );
    // 足した場所は、サブツールの組を縦に分けた下（ツールプロパティ、その下にブラシサイズ）。ほかの分け方・割合は前のまま
    let mut expected = previous_default();
    let sub_leaf = expected.find_tab(&Tab::SubTools).unwrap().node_path();
    expected.split(
        sub_leaf,
        egui_dock::Split::Below,
        0.38,
        Node::leaf_with(vec![Tab::ToolProperties]),
    );
    let props_leaf = expected.find_tab(&Tab::ToolProperties).unwrap().node_path();
    expected.split(
        props_leaf,
        egui_dock::Split::Below,
        0.8,
        Node::leaf_with(vec![Tab::BrushSize]),
    );
    let properties = expected.find_tab(&Tab::Properties).unwrap();
    expected
        .leaf_mut(properties.node_path())
        .unwrap()
        .tabs
        .insert(properties.tab.0 + 1, Tab::Material);
    assert_eq!(shape(&dock), shape(&expected));
    // 書き直して読み直しても同じ（足し直さない）
    let again = layout::parse(&layout::render(&dock, None));
    assert_eq!(shape(&again.dock.unwrap()), shape(&dock));
}

#[test]
fn headless_the_new_panels_follow_the_panels_they_belong_with_in_a_rearranged_old_layout() {
    // サブツールとプロパティを別の組へ動かした、前の並び: 足すタブは、そのとき居る組を基にする
    let mut old = without_new_tabs(default_dock());
    let sub = old.find_tab(&Tab::SubTools).unwrap();
    let canvas = old.find_tab(&Tab::Canvas).unwrap().node_path();
    old.move_tab(
        sub,
        egui_dock::TabDestination::Node(canvas, egui_dock::TabInsert::Append),
    );
    let props = old.find_tab(&Tab::Properties).unwrap();
    let layers = old.find_tab(&Tab::Layers).unwrap().node_path();
    old.move_tab(
        props,
        egui_dock::TabDestination::Node(layers, egui_dock::TabInsert::Append),
    );
    let loaded = layout::parse(&layout::render(&old, None));
    assert_eq!(loaded.problems, Vec::<String>::new());
    let dock = loaded.dock.unwrap();
    layout::validate(&dock).expect("どのタブも 1 つずつ");
    // マテリアルは、プロパティのすぐ後ろ
    let leaf = mates(&dock, Tab::Properties);
    let at = leaf.iter().position(|t| *t == Tab::Properties).unwrap();
    assert_eq!(leaf[at + 1], Tab::Material);
    // ツールプロパティは、サブツールの組の下の組（キャンバスの組が分かれる）
    let sub_leaf = mates(&dock, Tab::SubTools);
    assert!(
        sub_leaf.contains(&Tab::Canvas),
        "サブツールはキャンバスの組"
    );
    assert_eq!(mates(&dock, Tab::ToolProperties), [Tab::ToolProperties]);
    assert_eq!(mates(&dock, Tab::BrushSize), [Tab::BrushSize]);
}

#[test]
fn headless_a_layout_with_the_sub_tools_in_an_outside_window_gains_the_panels_in_that_window() {
    let mut main = default_dock();
    let mut outside = yolu_app::detach::Detached::new();
    assert!(outside.detach(
        &mut main,
        Tab::SubTools,
        Place::Record(FloatRecord {
            position: [60.0, 90.0],
            size: [320.0, 520.0],
            pixels_per_point: 1.0
        })
    ));
    let old_main = without_new_tabs_in(main);
    let records = vec![DetachedRecord {
        dock: outside.windows[0].dock.clone(),
        window: outside.windows[0].record,
        home: outside.windows[0].home.clone(),
    }];
    let loaded = layout::parse(&layout::render_all(&old_main, None, &[], &records));
    assert_eq!(loaded.problems, Vec::<String>::new());
    let window = &loaded.detached[0].dock;
    assert_eq!(
        window.main_surface().iter().find_map(|n| match n {
            Node::Leaf(leaf) => Some(leaf.tabs.clone()),
            _ => None,
        }),
        Some(vec![Tab::SubTools, Tab::ToolProperties, Tab::BrushSize]),
        "別ウィンドウのサブツールの組の、サブツールの後ろ"
    );
    let main = loaded.dock.unwrap();
    assert!(
        main.find_tab(&Tab::ToolProperties).is_none() && main.find_tab(&Tab::BrushSize).is_none()
    );
    assert!(
        main.find_tab(&Tab::Material).is_some(),
        "マテリアルは主のドックのプロパティの組"
    );
    layout::validate_all(&main, &[window]).expect("どのタブも 1 つずつ");
}

/// 主のドックから新しい 3 つのタブを外す（別ウィンドウに出した並びでは、主のドックにあるものだけ）。
fn without_new_tabs_in(mut dock: DockState<Tab>) -> DockState<Tab> {
    for tab in NEW_TABS {
        if let Some(path) = dock.find_tab(&tab) {
            dock.remove_tab(path);
        }
    }
    dock
}

#[test]
fn headless_a_layout_with_one_of_the_new_panels_twice_is_dropped_whole() {
    // 重なると、並び全部を捨てる（足りないタブだけを足すが、重なるタブは直さない）
    let mut twice = default_dock();
    twice.push_to_first_leaf(Tab::ToolProperties);
    let loaded = layout::parse(&layout::render(&twice, None));
    assert!(loaded.dock.is_none());
    assert!(
        loaded
            .problems
            .iter()
            .any(|p| p.contains("tool_properties")),
        "{:?}",
        loaded.problems
    );
}

fn settings_dir(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tool-panels-tests")
        .join(std::process::id().to_string())
        .join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn an_app_started_with_an_old_layout_file_shows_the_new_panels_and_writes_them_back() {
    let dir = settings_dir("old-layout");
    std::fs::write(
        dir.join(layout::FILE_NAME),
        layout::render(&previous_default(), None),
    )
    .unwrap();
    let settings = dir.join("settings.conf");
    let mut h = gpu_thread::builder()
        .with_size(vec2(1600.0, 900.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .renderer(shared_gpu::renderer())
        .build_eframe(move |cc| {
            with_render_state_cpu_canvas(
                YoluApp::for_context_with_settings(
                    &cc.egui_ctx,
                    Some(settings),
                    PenInput::detached(),
                ),
                cc.wgpu_render_state.as_ref(),
            )
        });
    h.run();
    for tab in NEW_TABS {
        assert!(h.state().tab_rects.contains_key(&tab), "{tab:?} が出ている");
    }
    let column: Vec<Rect> = [
        Tab::SubTools,
        Tab::ToolProperties,
        Tab::BrushSize,
        Tab::Color,
    ]
    .iter()
    .map(|t| tab_rect(&h, *t))
    .collect();
    for pair in column.windows(2) {
        assert!(pair[0].top() < pair[1].top(), "上から順 {column:?}");
    }
    // 並びを変えると（ヒストリーを別ウィンドウへ）、足した 3 つも含めて書かれる
    h.state_mut()
        .state
        .apply(Action::Dock(DockOp::Detach(Tab::History)));
    h.run();
    h.state_mut().on_exit();
    let written = std::fs::read_to_string(dir.join(layout::FILE_NAME)).unwrap();
    for key in ["tool_properties", "brush_size", "\"material\""] {
        assert!(written.contains(key), "{key} を書く");
    }
    let _ = std::fs::remove_dir_all(dir);
}

// ───────── プロパティのタブ（ステンシル・レイヤー） ─────────

#[test]
fn the_properties_have_two_tabs_and_no_material_tab_in_both_languages() {
    for lang in Lang::ALL {
        let mut h = app(1600.0, 900.0, 128);
        language(&mut h, lang);
        let props = tab_rect(&h, Tab::Properties);
        let in_props = |r: Rect| r.left() >= props.left() - 2.0 && r.top() > props.top();
        for name in lang.pick(["ステンシル", "レイヤー"], ["Stencil", "Layer"]) {
            assert!(
                h.query_all_by_label(name).any(|n| in_props(n.rect())),
                "{lang:?}: {name} のタブ"
            );
        }
        // タブはステンシルとレイヤーだけ（マテリアルのタブは無い。マテリアルはドックのタブ）
        assert!(
            h.query_all_by_label(lang.pick("マテリアル", "Material"))
                .all(|n| !in_props(n.rect())),
            "{lang:?}: プロパティの中にマテリアルのタブは無い"
        );
        // マスクに描いても、タブは 2 つのまま（マスクのタブは出ない）
        let id = st(&h).selected_layer.unwrap();
        h.state_mut()
            .state
            .apply(Action::M2(yolu_app::m2::Edit::AddMask(id)));
        h.run();
        assert!(st(&h).m2.edit_mask);
        assert_eq!(count(&h, lang.pick("マスク", "Mask")), 0, "{lang:?}");
    }
}

// ───────── ツールプロパティの「塗るチャンネル」 ─────────

/// 「塗るチャンネル」が効くツール（ツールの表の印）と、効かないツール。
#[test]
fn the_paint_channels_section_is_only_for_the_tools_that_paint_with_channels() {
    for lang in Lang::ALL {
        let name = lang.pick("塗るチャンネル", "Paint Channels");
        for tool in Tool::ALL {
            // （ツールプロパティの終わりまで入る高さで）
            let mut h = app(1280.0, 1800.0, 128);
            language(&mut h, lang);
            pick(&mut h, tool);
            assert_eq!(
                count(&h, name),
                usize::from(tool.def().paint_channels),
                "{lang:?} {tool:?}"
            );
        }
    }
    let painting = [
        Tool::Brush,
        Tool::Eraser,
        Tool::Fill,
        Tool::Gradient,
        Tool::Shape,
        Tool::PolygonFill,
        Tool::Eyedropper,
        Tool::Path,
        // 選択のツールは、選択範囲の塗りつぶし・消去が入れてある組を使うので、区分を見せる（表示だけ）
        Tool::SelectRect,
        Tool::SelectEllipse,
        Tool::Lasso,
        Tool::Polygon,
        Tool::Wand,
        Tool::SelectPen,
    ];
    for tool in Tool::ALL {
        assert_eq!(
            tool.def().paint_channels,
            painting.contains(&tool),
            "{tool:?}"
        );
    }
}

fn channel_pixel(s: &AppState, channel: Channel, x: u32, y: u32) -> yolu_app::engine::Rgba8 {
    let layer = s.doc.layer(s.selected_layer.unwrap()).unwrap();
    layer
        .surface(channel)
        .and_then(|surface| surface.pixel(x, y).ok())
        .unwrap_or(yolu_app::engine::Rgba8::TRANSPARENT)
}

#[test]
fn the_tool_properties_paint_channels_change_the_state_and_the_stroke_paints_those_channels() {
    let mut h = app(1280.0, 1500.0, 128);
    h.state_mut().state.color.set_main([1.0, 0.0, 0.0, 1.0]);
    {
        let b = &mut h.state_mut().state.brush;
        b.radius = 3.0;
        b.hardness = 1.0;
    }
    let props = tab_rect(&h, Tab::ToolProperties);
    let size = tab_rect(&h, Tab::BrushSize);
    let in_props = move |r: Rect| {
        r.top() > props.bottom() && r.bottom() < size.top() + 2.0 && r.left() < 340.0
    };
    // 初めは閉じている。名前を押して開く
    assert!(!st(&h).mat.enabled);
    assert!(h.query_by_label("複数のチャンネルを一度に塗る").is_none());
    let header = rect_of(&h, "塗るチャンネル", in_props);
    click(&mut h, header.center());
    let toggle = rect_of(&h, "複数のチャンネルを一度に塗る", in_props);
    click(&mut h, toggle.center());
    assert!(st(&h).mat.enabled);
    assert_eq!(st(&h).mat.included(), vec![Channel::Color]);
    // チップでラフネスを足す。最後の 1 つは外せない
    let chip = rect_of(&h, "ラフネス", |r| in_props(r) && r.height() < 30.0);
    click(&mut h, chip.center());
    assert_eq!(
        st(&h).mat.included(),
        vec![Channel::Color, Channel::Roughness]
    );
    let chip = rect_of(&h, "カラー", |r| in_props(r) && r.height() < 30.0);
    click(&mut h, chip.center());
    let chip = rect_of(&h, "ラフネス", |r| in_props(r) && r.height() < 30.0);
    click(&mut h, chip.center());
    assert_eq!(
        st(&h).mat.included(),
        vec![Channel::Roughness],
        "最後の 1 つは外せない"
    );
    assert!(st(&h).message.contains("1 つ以上"));
    let chip = rect_of(&h, "カラー", |r| in_props(r) && r.height() < 30.0);
    click(&mut h, chip.center());
    assert_eq!(
        st(&h).mat.included(),
        vec![Channel::Color, Channel::Roughness]
    );
    // 値を変えて描くと、組のチャンネルが 1 回の Undo で塗られる（組に無いチャンネルは塗らない）
    h.state_mut().state.mat.set_scalar(Channel::Roughness, 0.25);
    h.run();
    let steps = st(&h).doc.undo_count();
    let c = canvas_rect(&h).center();
    drag(&mut h, &[offset(c, -20.0, 0.0), offset(c, 20.0, 0.0)]);
    assert_eq!(
        st(&h).doc.undo_count(),
        steps + 1,
        "全チャンネルで 1 回の Undo"
    );
    let at = (64, 64);
    assert_eq!(
        channel_pixel(st(&h), Channel::Color, at.0, at.1),
        yolu_app::engine::Rgba8::new(255, 0, 0, 255)
    );
    assert_eq!(
        channel_pixel(st(&h), Channel::Roughness, at.0, at.1),
        yolu_app::engine::Rgba8::new(64, 64, 64, 255)
    );
    assert_eq!(
        channel_pixel(st(&h), Channel::Metallic, at.0, at.1),
        yolu_app::engine::Rgba8::TRANSPARENT
    );
    h.state_mut().state.apply(Action::Undo);
    for ch in [Channel::Color, Channel::Roughness] {
        assert_eq!(
            channel_pixel(st(&h), ch, at.0, at.1),
            yolu_app::engine::Rgba8::TRANSPARENT,
            "{ch:?}"
        );
    }
    // 取り消しに積まない値（組・値は画面の状態）
    let steps = st(&h).doc.undo_count();
    h.state_mut()
        .state
        .apply(Action::Mat(MatAction::Channel(Channel::Metallic, true)));
    assert_eq!(st(&h).doc.undo_count(), steps);
}

/// 選択のツールのツールプロパティにも「塗るチャンネル」が出る。入れた組で選択範囲を塗りつぶすと組の全部が塗られ（今の動きのまま）、
/// マスクに描いているあいだは区分の代わりにマスクの欄が出る。
#[test]
fn the_selection_tools_show_the_paint_channels_and_the_fill_paints_the_whole_set() {
    use yolu_app::selection::{SelAction, SelEdit};
    let mut h = app(1280.0, 1800.0, 128);
    pick(&mut h, Tool::SelectRect);
    assert_eq!(count(&h, "塗るチャンネル"), 1);
    // 組を入れて、全体を選んで塗りつぶす
    h.state_mut().state.color.set_main([1.0, 0.0, 0.0, 1.0]);
    h.state_mut()
        .state
        .apply(Action::Mat(MatAction::Enabled(true)));
    h.state_mut()
        .state
        .apply(Action::Mat(MatAction::Channel(Channel::Roughness, true)));
    h.state_mut().state.mat.set_scalar(Channel::Roughness, 0.25);
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Edit(SelEdit::All)));
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Edit(SelEdit::Fill)));
    h.run();
    assert_eq!(
        channel_pixel(st(&h), Channel::Color, 64, 64),
        yolu_app::engine::Rgba8::new(255, 0, 0, 255)
    );
    assert_eq!(
        channel_pixel(st(&h), Channel::Roughness, 64, 64),
        yolu_app::engine::Rgba8::new(64, 64, 64, 255)
    );
    assert_eq!(
        channel_pixel(st(&h), Channel::Metallic, 64, 64),
        yolu_app::engine::Rgba8::TRANSPARENT,
        "組に無いチャンネルは塗らない"
    );
    // マスクに描いているあいだは、区分でなくマスクの欄
    let id = st(&h).selected_layer.unwrap();
    h.state_mut()
        .state
        .apply(Action::M2(yolu_app::m2::Edit::AddMask(id)));
    h.run();
    assert!(st(&h).m2.edit_mask);
    assert_eq!(count(&h, "塗るチャンネル"), 0);
}

/// 今のブラシが塗り以外の効果（指先）でも、値を使うツール（バケツなど）では、塗るチャンネルの値の行が出る。ブラシ・消しゴムでは効果のあいだ出ない。
#[test]
fn the_paint_channel_values_follow_the_tool_not_the_brush_effect_of_other_tools() {
    let mut h = app(1280.0, 1800.0, 128);
    open_paint_channels(&mut h);
    h.state_mut()
        .state
        .apply(Action::Mat(MatAction::Enabled(true)));
    h.state_mut()
        .state
        .apply(Action::Mat(MatAction::Channel(Channel::Roughness, true)));
    h.state_mut().state.m2.brush.effect = yolu_core::brush::BrushEffect::Smudge { strength: 0.5 };
    // 値の行（ラフネスの値の欄）の数: チップの「ラフネス」のほかに、値の行の名前が出る
    let rows = |h: &H| count(h, "ラフネス");
    pick(&mut h, Tool::Brush);
    let brush = rows(&h);
    for tool in [
        Tool::Fill,
        Tool::Gradient,
        Tool::Shape,
        Tool::PolygonFill,
        Tool::Path,
        Tool::Eyedropper,
        Tool::SelectRect,
    ] {
        pick(&mut h, tool);
        assert!(
            rows(&h) > brush,
            "{tool:?}: 効果のブラシでも値の行が出る（{} / ブラシ {brush}）",
            rows(&h)
        );
    }
    h.state_mut().state.m2.brush.effect = yolu_core::brush::BrushEffect::Paint;
    pick(&mut h, Tool::Brush);
    assert!(rows(&h) > brush, "塗りのブラシでは値の行が出る");
}

/// 「塗るチャンネル」を入れているあいだは、閉じていても見出しの右端に点の印が出る（切ると消える）。
#[test]
fn the_paint_channels_header_carries_a_dot_while_the_setting_is_on() {
    let mut h = app(1280.0, 1500.0, 128);
    let props = tab_rect(&h, Tab::ToolProperties);
    let size = tab_rect(&h, Tab::BrushSize);
    let in_props = move |r: Rect| {
        r.top() > props.bottom() && r.bottom() < size.top() + 2.0 && r.left() < 340.0
    };
    let header = rect_of(&h, "塗るチャンネル", in_props);
    // （見出しの帯の全幅。名前の部品の矩形は帯の全体）
    let at = pos2(header.right() - 14.0, header.center().y);
    let dot_color = |h: &mut H| {
        h.event(Event::PointerGone);
        h.step();
        let image = h.render().expect("描画");
        image.get_pixel(at.x as u32, at.y as u32).0
    };
    let accent = yolu_app::ui::theme::ACCENT.to_array();
    assert!(!st(&h).section_open("paint-channels", false), "閉じている");
    let off = dot_color(&mut h);
    assert_ne!(off[..3], accent[..3], "切っているあいだは点が無い");
    h.state_mut()
        .state
        .apply(Action::Mat(MatAction::Enabled(true)));
    h.run();
    assert!(!st(&h).section_open("paint-channels", false), "閉じたまま");
    let on = dot_color(&mut h);
    assert_eq!(
        on[..3],
        accent[..3],
        "入れているあいだは、閉じていても点が出る"
    );
    h.state_mut()
        .state
        .apply(Action::Mat(MatAction::Enabled(false)));
    h.run();
    assert_ne!(dot_color(&mut h)[..3], accent[..3], "切ると消える");
}

#[test]
fn while_painting_a_mask_the_tool_properties_show_the_layer_mask_instead_of_the_paint_channels() {
    let mut h = app(1280.0, 1500.0, 128);
    let props = tab_rect(&h, Tab::ToolProperties);
    let size = tab_rect(&h, Tab::BrushSize);
    let in_props = move |r: Rect| {
        r.top() > props.bottom() && r.bottom() < size.top() + 2.0 && r.left() < 340.0
    };
    assert!(h
        .query_all_by_label("塗るチャンネル")
        .any(|n| in_props(n.rect())));
    assert!(!h
        .query_all_by_label("レイヤーマスク")
        .any(|n| in_props(n.rect())));
    let id = st(&h).selected_layer.unwrap();
    h.state_mut()
        .state
        .apply(Action::M2(yolu_app::m2::Edit::AddMask(id)));
    h.run();
    assert!(st(&h).m2.edit_mask);
    assert!(
        h.query_all_by_label("レイヤーマスク")
            .any(|n| in_props(n.rect())),
        "マスクの値（濃度・有効・反転）は同じ所"
    );
    assert!(!h
        .query_all_by_label("塗るチャンネル")
        .any(|n| in_props(n.rect())));
    let density = rect_of(&h, "濃度", in_props);
    // 値を変えると 1 回の Undo（レイヤーの設定）。ドラッグを離したところで区切る
    let steps = st(&h).doc.undo_count();
    drag(
        &mut h,
        &[
            pos2(density.left() + 20.0, density.center().y),
            pos2(density.left() + 40.0, density.center().y),
            pos2(density.left() + 60.0, density.center().y),
        ],
    );
    assert_eq!(st(&h).doc.undo_count(), steps + 1);
    h.state_mut().state.apply(Action::Undo);
    h.run();
    // マスクをやめると、塗るチャンネルへ戻る
    h.state_mut()
        .state
        .apply(Action::M2Ui(yolu_app::m2::UiOp::EditMask(false)));
    h.run();
    assert!(h
        .query_all_by_label("塗るチャンネル")
        .any(|n| in_props(n.rect())));
}

// ───────── 見た目（日英） ─────────

fn shot_rect(h: &mut H, rect: Rect, name: &str) {
    h.event(Event::PointerGone);
    h.step();
    let image = h.render().expect("描画");
    let cropped = image::imageops::crop_imm(
        &image,
        rect.left().floor().max(0.0) as u32,
        rect.top().floor().max(0.0) as u32,
        rect.width().ceil() as u32,
        rect.height().ceil() as u32,
    )
    .to_image();
    egui_kittest::image_snapshot(&cropped, name);
}

/// 左の列の、サブツールのタブからカラーのタブまで（3 つのパネルが縦に並ぶ所）。
fn left_stack(h: &H) -> Rect {
    let (sub, color) = (tab_rect(h, Tab::SubTools), tab_rect(h, Tab::Color));
    // 左の列の右の端は、キャンバスのタブの左
    Rect::from_min_max(
        pos2(sub.left() - 2.0, sub.top()),
        pos2(tab_rect(h, Tab::Canvas).left() - 2.0, color.top()),
    )
}

/// 右の列の、プロパティの組（プロパティのタブから、ウィンドウの右・状態の帯の上まで）。
fn right_bottom(h: &H) -> Rect {
    let props = tab_rect(h, Tab::Properties);
    let window = h.ctx.content_rect();
    Rect::from_min_max(
        pos2(props.left() - 2.0, props.top()),
        pos2(window.right(), window.bottom() - 24.0),
    )
}

#[test]
fn snapshots_of_the_whole_default_dock_in_both_languages() {
    let mut results = egui_kittest::SnapshotResults::new();
    for (lang, name) in [(Lang::Ja, "ja"), (Lang::En, "en")] {
        let mut h = app_default(1280.0, 800.0, 128);
        language(&mut h, lang);
        // 直前に押した所のポインタが絵に残らないように
        h.event(Event::PointerGone);
        h.step();
        h.snapshot(format!("tool_panels_dock_{name}"));
        results.extend_harness(&mut h);
    }
}

#[test]
fn snapshots_of_the_default_left_column_and_the_properties_group_in_both_languages() {
    for (lang, name) in [(Lang::Ja, "ja"), (Lang::En, "en")] {
        let mut h = app(1280.0, 800.0, 128);
        language(&mut h, lang);
        let rect = left_stack(&h);
        shot_rect(&mut h, rect, &format!("tool_panels_left_{name}"));
        let rect = right_bottom(&h);
        shot_rect(&mut h, rect, &format!("tool_panels_properties_{name}"));
    }
}

#[test]
fn snapshots_of_the_tool_properties_with_the_paint_channels_open_and_for_the_mask_in_both_languages(
) {
    for (lang, name) in [(Lang::Ja, "ja"), (Lang::En, "en")] {
        let mut h = app(1280.0, 1500.0, 128);
        language(&mut h, lang);
        open_paint_channels(&mut h);
        h.state_mut()
            .state
            .apply(Action::Mat(MatAction::Enabled(true)));
        h.state_mut()
            .state
            .apply(Action::Mat(MatAction::Channel(Channel::Roughness, true)));
        h.state_mut()
            .state
            .apply(Action::Mat(MatAction::Channel(Channel::Emission, true)));
        h.state_mut().state.mat.emission = [0.2, 0.8, 0.4];
        h.run();
        let (props, size) = (
            tab_rect(&h, Tab::ToolProperties),
            tab_rect(&h, Tab::BrushSize),
        );
        let rect = Rect::from_min_max(
            pos2(props.left() - 2.0, props.top()),
            pos2(props.left() + 300.0, size.top()),
        );
        shot_rect(&mut h, rect, &format!("tool_panels_paint_channels_{name}"));
        // マスクに描くあいだ
        let id = st(&h).selected_layer.unwrap();
        h.state_mut()
            .state
            .apply(Action::M2(yolu_app::m2::Edit::AddMask(id)));
        h.run();
        shot_rect(&mut h, rect, &format!("tool_panels_mask_{name}"));
    }
}

#[test]
fn snapshots_of_the_material_panel_with_the_standard_and_liltoon_looks_in_both_languages() {
    for (lang, name) in [(Lang::Ja, "ja"), (Lang::En, "en")] {
        let mut h = app(1500.0, 1300.0, 64);
        language(&mut h, lang);
        open_material(&mut h);
        // マテリアルの組の全体（同じ組のプロパティのタブの左から、ウィンドウの右・状態の帯の上まで）
        let group = tab_rect(&h, Tab::Properties);
        let window = h.ctx.content_rect();
        let rect = Rect::from_min_max(
            pos2(group.left() - 2.0, group.top()),
            pos2(window.right(), window.bottom() - 24.0),
        );
        // 標準
        h.state_mut()
            .state
            .apply(Action::Look(yolu_app::look::LookOp::Kind(
                yolu_core::look::LookKind::Standard,
            )));
        h.run();
        shot_rect(
            &mut h,
            rect,
            &format!("tool_panels_material_standard_{name}"),
        );
        // lilToon（ひな形つき）
        h.state_mut()
            .state
            .apply(Action::Look(yolu_app::look::LookOp::Kind(
                yolu_core::look::LookKind::LilToon,
            )));
        h.state_mut()
            .state
            .apply(Action::Look(yolu_app::look::LookOp::Template));
        h.run();
        shot_rect(
            &mut h,
            rect,
            &format!("tool_panels_material_liltoon_{name}"),
        );
    }
}

#[test]
fn snapshots_of_the_tool_properties_in_its_own_window() {
    for (lang, name) in [(Lang::Ja, "ja"), (Lang::En, "en")] {
        let mut h = app(1280.0, 900.0, 128);
        language(&mut h, lang);
        h.state_mut()
            .state
            .apply(Action::Dock(DockOp::Detach(Tab::ToolProperties)));
        h.run();
        h.step();
        // 試験では、別ウィンドウはメインウィンドウの中の egui のウィンドウ（その矩形だけを撮る）
        let window = h.state().detached.windows[0].tab_rects[&Tab::ToolProperties];
        let rect = Rect::from_min_max(
            pos2(window.left() - 8.0, window.top() - 30.0),
            pos2(window.left() + 360.0, (window.top() + 380.0).min(890.0)),
        );
        shot_rect(&mut h, rect, &format!("tool_panels_detached_{name}"));
    }
}
