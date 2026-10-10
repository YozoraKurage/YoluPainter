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
fn headless_the_default_dock_pairs_the_material_with_the_properties_and_gives_each_tool_panel_its_own_group(
) {
    let dock = default_dock();
    assert_eq!(
        mates(&dock, Tab::Material),
        [Tab::Properties, Tab::Material, Tab::History],
        "右下のプロパティ・ヒストリーと並ぶ"
    );
    assert_eq!(mates(&dock, Tab::ToolProperties), [Tab::ToolProperties]);
    assert_eq!(mates(&dock, Tab::BrushSize), [Tab::BrushSize]);
    assert_eq!(
        mates(&dock, Tab::SubTools),
        [Tab::SubTools, Tab::Assets, Tab::Channels],
        "サブツールの組のタブは今のまま"
    );
    layout::validate(&dock).expect("どのタブも 1 つずつ");
    // ファイルで往復する
    let loaded = layout::parse(&layout::render(&dock, None));
    assert_eq!(loaded.problems, Vec::<String>::new());
    assert_eq!(shape(&loaded.dock.unwrap()), shape(&dock));
}

#[test]
fn the_default_window_stacks_the_tool_panels_in_the_left_column_and_the_material_sits_beside_the_properties(
) {
    for (width, height) in [(1600.0, 900.0), (1280.0, 800.0), (960.0, 640.0)] {
        let mut h = app(width, height, 128);
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
            assert!(
                pair[0].top() < pair[1].top(),
                "{width}: 上から順 {column:?}"
            );
            assert!(
                (pair[0].left() - pair[1].left()).abs() < 2.0,
                "{width}: 同じ列 {column:?}"
            );
        }
        // 右下: プロパティ・マテリアル・ヒストリーが同じ帯に並ぶ（最小のウィンドウでも、3 つとも見える）
        let right: Vec<Rect> = [Tab::Properties, Tab::Material, Tab::History]
            .iter()
            .map(|t| tab_rect(&h, *t))
            .collect();
        assert!(
            (right[0].top() - right[1].top()).abs() < 1.0
                && (right[1].top() - right[2].top()).abs() < 1.0
        );
        assert!(
            right[0].right() <= right[1].left() + 1.0 && right[1].right() <= right[2].left() + 1.0
        );
        assert!(right[2].right() <= width, "{width}: ヒストリーが切れない");
        // それぞれの中身が、自分の組に出る
        let (props, size) = (column[1], column[2]);
        assert!(
            h.query_all_by_label("直径")
                .any(|n| n.rect().top() > props.bottom() && n.rect().bottom() < size.top() + 2.0),
            "{width}: 直径はツールプロパティの中"
        );
        pick(&mut h, Tool::Fill);
        language(&mut h, Lang::En);
        assert_eq!(
            count(&h, "Brush Size"),
            0,
            "{width}: 名前はタブだけ（帯の見出しは無い）"
        );
    }
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
    let old = without_new_tabs(default_dock());
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
    // 足した場所は、既定の並びと同じ（サブツールの組の下に縦に、プロパティのすぐ後ろ）。ほかは前の並びのまま
    assert_eq!(shape(&dock), shape(&default_dock()));
    layout::validate(&dock).expect("どのタブも 1 つずつ");
    let sub = dock.find_main_surface_tab(&Tab::SubTools).unwrap().0;
    let props = dock.find_main_surface_tab(&Tab::ToolProperties).unwrap().0;
    let size = dock.find_main_surface_tab(&Tab::BrushSize).unwrap().0;
    // サブツールの組の下に、ツールプロパティとブラシサイズを含む組が兄弟として付き、その組が縦に 2 つに分かれる
    assert_eq!(
        (sub.0 % 2, props.0, size.0),
        (1, 2 * (sub.0 + 1) + 1, 2 * (sub.0 + 1) + 2),
        "縦に分けた兄弟"
    );
    assert!(matches!(
        dock.main_surface()[egui_dock::NodeIndex((sub.0 - 1) / 2)],
        Node::Vertical(_)
    ));
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
        layout::render(&without_new_tabs(default_dock()), None),
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
            let mut h = app(1280.0, 900.0, 128);
            language(&mut h, lang);
            give_room(&mut h, Tab::ToolProperties);
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
        Tool::Path,
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
    Rect::from_min_max(
        pos2(sub.left() - 2.0, sub.top()),
        pos2(sub.left() + 300.0, color.top()),
    )
}

/// 右の列の、プロパティのタブから下の端まで。
fn right_bottom(h: &H, height: f32) -> Rect {
    let props = tab_rect(h, Tab::Properties);
    Rect::from_min_max(
        pos2(props.left() - 2.0, props.top()),
        pos2(props.left() + 330.0, height),
    )
}

#[test]
fn snapshots_of_the_whole_default_dock_in_both_languages() {
    let mut results = egui_kittest::SnapshotResults::new();
    for (lang, name) in [(Lang::Ja, "ja"), (Lang::En, "en")] {
        let mut h = app(1280.0, 800.0, 128);
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
        let rect = right_bottom(&h, 790.0);
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
        let tab = tab_rect(&h, Tab::Material);
        let rect = Rect::from_min_max(
            pos2(tab.left() - 60.0, tab.top()),
            pos2(tab.left() + 300.0, 1290.0),
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
