//! 外へ出したウィンドウ（ドックの欄を、アプリのウィンドウの外の OS のウィンドウへ）: タブの右クリックの「別ウィンドウで開く」・「ドックに戻す」、ウィンドウの外で離す・ドックへ
//! 落として戻す、egui_dock の浮いたウィンドウが別ウィンドウになる、閉じると戻る、メニューの「ウィンドウ」、`layout.json` に覚えて起動で戻す、前の版の
//! 浮いた欄、画面が外れたらメインウィンドウの上、別ウィンドウでキー・メニュー・ドロップ・キャンバスが効く。
//!
//! 別ウィンドウは、試験の既定（kittest は子の viewport を根の中の egui のウィンドウに埋める）ではメインウィンドウの中の egui のウィンドウに描かれる。別ウィンドウの入力は
//! `common::viewports` の口で子の viewport を本物の別のパスとして回して確かめる（子ウィンドウの絵は撮らない）。
use crate::common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::viewports::Driver;
use common::*;
use egui::{
    pos2, vec2, Event, Key, Modifiers, PointerButton, Pos2, Rect, ViewportCommand, ViewportId,
};
use egui_dock::{DockState, Node, SurfaceIndex};
use egui_kittest::Harness;
use yolu_app::app::default_dock;
use yolu_app::detach::{self, place, DockOp, Place};
use yolu_app::layout::{self, DetachedRecord, FloatRecord};
use yolu_app::pen::{PenInput, PenSample};
use yolu_app::shell;
use yolu_app::state::{Action, AppState};
use yolu_app::windowpos::{Monitor, PxRect};
use yolu_app::{Tab, YoluApp};

fn settings_dir(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/detach-tests")
        .join(std::process::id().to_string())
        .join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
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

/// タブがいる組のタブ（主の面）。
fn mates(dock: &DockState<Tab>, tab: Tab) -> Vec<Tab> {
    let (node, _) = dock.find_main_surface_tab(&tab).expect("タブ");
    match &dock.main_surface()[node] {
        Node::Leaf(leaf) => leaf.tabs.clone(),
        _ => unreachable!(),
    }
}

/// 主のドックと別ウィンドウを合わせて、どのタブも 1 つずつ（ポーズは無い）。
fn assert_every_tab_once(main: &DockState<Tab>, outside: &[&DockState<Tab>]) {
    layout::validate_all(main, outside).expect("どのタブも 1 つずつ");
}

fn record(x: f32, y: f32, w: f32, h: f32) -> FloatRecord {
    FloatRecord {
        position: [x, y],
        size: [w, h],
        pixels_per_point: 1.0,
    }
}

// ───────── 形・並びの操作（画面なし） ─────────

#[test]
fn headless_outside_windows_survive_the_file_with_their_tabs_front_tab_place_and_home() {
    let mut main = default_dock();
    let mut outside = detach::Detached::new();
    assert!(outside.detach(
        &mut main,
        Tab::History,
        Place::Record(record(1700.0, 200.0, 320.0, 400.0))
    ));
    assert!(outside.detach(
        &mut main,
        Tab::Channels,
        Place::Record(record(-1500.0, 80.0, 300.0, 260.0))
    ));
    // チャンネルのウィンドウへログも入れる（前はログ）
    let serial = outside.windows[1].serial;
    assert!(outside.move_into(&mut main, Tab::Log, serial, None));
    assert_every_tab_once(&main, &[&outside.windows[0].dock, &outside.windows[1].dock]);
    let records: Vec<DetachedRecord> = outside
        .windows
        .iter()
        .map(|w| DetachedRecord {
            dock: w.dock.clone(),
            window: w.record,
            home: w.home.clone(),
        })
        .collect();
    let text = layout::render_all(&main, None, &[], &records);
    let loaded = layout::parse(&text);
    assert_eq!(loaded.problems, Vec::<String>::new());
    assert_eq!(shape(&loaded.dock.expect("主のドック")), shape(&main));
    assert_eq!(loaded.detached.len(), 2);
    for (read, written) in loaded.detached.iter().zip(&records) {
        assert_eq!(shape(&read.dock), shape(&written.dock));
        assert_eq!(read.window, written.window);
        assert_eq!(read.home, written.home);
    }
    assert_eq!(
        shape(&loaded.detached[1].dock),
        vec!["0/0 leaf [\"channels\", \"log\"] active=1"]
    );
    // 戻る先は、出したときに同じ組にいたタブ（履歴はプロパティの組、チャンネルはテクスチャセットの組）
    assert_eq!(
        loaded.detached[0].home,
        vec![Tab::Properties, Tab::Material]
    );
    assert_eq!(loaded.detached[1].home, vec![Tab::TextureSets, Tab::Assets]);
    // 別ウィンドウの無いファイルは、前の版と同じ中身（`detached` を書かない）
    let plain = default_dock();
    assert_eq!(
        layout::render_all(&plain, None, &[], &[]),
        layout::render(&plain, None)
    );
    assert!(!layout::render(&plain, None).contains("detached"));
}

#[test]
fn headless_tabs_are_counted_across_the_main_dock_and_the_outside_windows() {
    let main = default_dock();
    // 主のドックにあるタブを、別ウィンドウにも持つと重なる
    let twice = DockState::new(vec![Tab::History]);
    let reason = layout::validate_all(&main, &[&twice]).unwrap_err();
    assert!(
        reason.contains("history") && reason.contains("2"),
        "{reason}"
    );
    // 主のドックから出したタブは、別ウィンドウに無ければ足りない
    let mut without = default_dock();
    let path = without.find_tab(&Tab::History).unwrap();
    without.remove_tab(path);
    let reason = layout::validate_all(&without, &[]).unwrap_err();
    assert!(reason.contains("history"), "{reason}");
    assert!(layout::validate_all(&without, &[&twice]).is_ok());
    // 別ウィンドウの中に、さらに浮いたウィンドウの面があるのは使わない
    let mut nested = DockState::new(vec![Tab::History]);
    nested.add_window(vec![Tab::Pose]);
    let reason = layout::validate_all(&without, &[&nested]).unwrap_err();
    assert!(reason.contains("浮いたウィンドウ"), "{reason}");
    // 前の版（別ウィンドウを知らない読み手）が読むと、主のドックのタブが足りないので並び全体を既定へ戻す（落ちない）
    let text = layout::render_all(
        &without,
        None,
        &[],
        &[DetachedRecord {
            dock: twice.clone(),
            window: None,
            home: Vec::new(),
        }],
    );
    let main_only: serde_json::Value = serde_json::from_str(&text).unwrap();
    let old_reader = serde_json::from_value::<DockState<Tab>>(main_only["dock"].clone()).unwrap();
    assert!(layout::validate(&old_reader).is_err());
}

#[test]
fn headless_an_outside_window_with_a_broken_place_keeps_its_tabs_and_an_unreadable_one_drops_the_layout(
) {
    let mut main = default_dock();
    let path = main.find_tab(&Tab::History).unwrap();
    main.remove_tab(path);
    let outside = DockState::new(vec![Tab::History]);
    let good = layout::render_all(
        &main,
        None,
        &[],
        &[DetachedRecord {
            dock: outside,
            window: Some(record(100.0, 100.0, 320.0, 240.0)),
            home: vec![Tab::Properties],
        }],
    );
    // 位置の値が壊れている: ウィンドウは位置なし（メインウィンドウの上に開く）、タブはそのまま
    let mut value: serde_json::Value = serde_json::from_str(&good).unwrap();
    value["detached"][0]["window"]["x"] = serde_json::json!(1e9);
    let loaded = layout::parse(&value.to_string());
    assert_eq!(loaded.detached.len(), 1);
    assert_eq!(loaded.detached[0].window, None);
    assert_eq!(loaded.problems.len(), 1, "{:?}", loaded.problems);
    assert!(loaded.dock.is_some());
    // 小さすぎるウィンドウは、別ウィンドウの最小の大きさまで広げる
    let mut value: serde_json::Value = serde_json::from_str(&good).unwrap();
    value["detached"][0]["window"]["width"] = serde_json::json!(20.0);
    let loaded = layout::parse(&value.to_string());
    assert_eq!(
        loaded.detached[0].window.map(|w| w.size),
        Some([detach::MIN_SIZE[0], 240.0])
    );
    // 知らない戻る先のタブは飛ばす（並びは使う）
    let mut value: serde_json::Value = serde_json::from_str(&good).unwrap();
    value["detached"][0]["home"] = serde_json::json!(["nothing", "layers"]);
    let loaded = layout::parse(&value.to_string());
    assert_eq!(loaded.detached[0].home, vec![Tab::Layers]);
    // 中のドックが読めない・別ウィンドウが配列でない: 主のドックも一緒に捨てて既定の並び（理由は診断のログだけ）
    for broken in [
        serde_json::json!([{"dock": 3}]),
        serde_json::json!([{"window": null}]),
        serde_json::json!({"dock": {}}),
    ] {
        let mut value: serde_json::Value = serde_json::from_str(&good).unwrap();
        value["detached"] = broken.clone();
        let loaded = layout::parse(&value.to_string());
        assert!(
            loaded.dock.is_none() && loaded.detached.is_empty(),
            "{broken}"
        );
        assert!(!loaded.problems.is_empty());
    }
}

#[test]
fn headless_an_outside_window_stays_on_a_screen_and_moves_over_the_main_window_when_its_screen_is_gone(
) {
    let left = Monitor {
        bounds: PxRect::from_origin_size(0, 0, 1920, 1080),
        work: PxRect::from_origin_size(0, 0, 1920, 1040),
        scale: 1.0,
        primary: true,
    };
    let right = Monitor {
        bounds: PxRect::from_origin_size(1920, 0, 2560, 1440),
        work: PxRect::from_origin_size(1920, 0, 2560, 1400),
        scale: 1.5,
        primary: false,
    };
    let main = Some(PxRect::from_origin_size(200, 100, 1400, 900));
    // 画面が分からない OS（Linux）は、記録のまま
    assert_eq!(
        place::plan(&record(5000.0, 0.0, 300.0, 200.0), &[], main),
        None
    );
    // 右の画面（拡大率 1.5）にいたウィンドウ: 位置は画素のまま、大きさはその画面の拡大率で
    let on_right = FloatRecord {
        position: [2400.0 / 1.5, 300.0 / 1.5],
        size: [320.0, 400.0],
        pixels_per_point: 1.5,
    };
    let p = place::plan(&on_right, &[left, right], main).unwrap();
    assert_eq!((p.rect.left, p.rect.top), (2400, 300));
    assert_eq!((p.rect.width(), p.rect.height()), (480, 600));
    assert_eq!(p.scale, 1.5);
    // 右の画面を外した: メインウィンドウの上の中央へ（メインウィンドウの画面の拡大率で）
    let p = place::plan(&on_right, &[left], main).unwrap();
    assert_eq!((p.rect.width(), p.rect.height()), (320, 400));
    assert_eq!(
        (
            (p.rect.left + p.rect.right) / 2,
            (p.rect.top + p.rect.bottom) / 2
        ),
        (900, 550)
    );
    assert_eq!(p.scale, 1.0);
    // 上の帯は見えているが、ウィンドウが画面からはみ出す: 作業領域の中へ寄せる
    let p = place::plan(&record(1600.0, 900.0, 400.0, 300.0), &[left], main).unwrap();
    assert_eq!((p.rect.right, p.rect.bottom), (1920, 1040));
    // メインウィンドウも分からなければ、主の画面の中央
    let p = place::plan(
        &record(-9000.0, -9000.0, 400.0, 300.0),
        &[left, right],
        None,
    )
    .unwrap();
    assert_eq!((p.rect.left, p.rect.top), (760, 370));
}

#[test]
fn headless_taking_out_and_returning_tabs_keeps_every_tab_once_and_goes_back_to_where_it_was() {
    let mut main = default_dock();
    let mut outside = detach::Detached::new();
    let at = Place::Record(record(0.0, 0.0, 300.0, 300.0));
    // 出すと主のドックから消え、別ウィンドウに入る（戻る先は同じ組にいたタブ）
    assert!(outside.detach(&mut main, Tab::Layers, at));
    assert!(main.find_tab(&Tab::Layers).is_none());
    assert_eq!(outside.windows.len(), 1);
    assert_eq!(outside.windows[0].home, vec![Tab::Log]);
    assert_every_tab_once(&main, &[&outside.windows[0].dock]);
    // 主のドックのタブは「戻す」が効かない。無いタブは出さない
    assert!(!outside.return_tab(&mut main, Tab::Canvas, None));
    let mut gone = default_dock();
    let path = gone.find_tab(&Tab::Log).unwrap();
    gone.remove_tab(path);
    assert!(!detach::Detached::new().detach(&mut gone, Tab::Log, at));
    // 戻すと、戻る先の組（ログの組）へ入って前になる
    assert!(outside.return_tab(&mut main, Tab::Layers, None));
    assert!(outside.windows.is_empty(), "空になったウィンドウは消える");
    assert_eq!(mates(&main, Tab::Layers), vec![Tab::Log, Tab::Layers]);
    let (node, index) = main.find_main_surface_tab(&Tab::Layers).unwrap();
    assert_eq!(
        main.leaf(egui_dock::NodePath {
            surface: SurfaceIndex::main(),
            node
        })
        .unwrap()
        .active,
        index
    );
    // 閉じると、中のタブを全部戻し、前だったタブを戻した先でも前にする
    outside.detach(&mut main, Tab::History, at);
    let serial = outside.windows[0].serial;
    outside.move_into(&mut main, Tab::Color, serial, None);
    outside.move_into(&mut main, Tab::ColorSets, serial, None);
    detach::activate(&mut outside.windows[0].dock, Tab::Color);
    outside.close(&mut main, serial);
    assert!(outside.windows.is_empty());
    assert_eq!(
        mates(&main, Tab::History),
        vec![
            Tab::Properties,
            Tab::Material,
            Tab::History,
            Tab::Color,
            Tab::ColorSets
        ]
    );
    let (node, index) = main.find_main_surface_tab(&Tab::Color).unwrap();
    let leaf = main
        .leaf(egui_dock::NodePath {
            surface: SurfaceIndex::main(),
            node,
        })
        .unwrap();
    assert_eq!(leaf.active, index, "前だったカラーが前");
    assert_every_tab_once(&main, &[]);
    // 戻る先は、出したときに同じ組にいたタブ（プロパティを先に出したので、ヒストリーの戻る先はカラーの組）
    outside.detach(&mut main, Tab::Properties, at);
    outside.detach(&mut main, Tab::History, at);
    let history_window = outside.windows[1].serial;
    outside.close(&mut main, history_window);
    assert!(
        mates(&main, Tab::History).contains(&Tab::Color),
        "{:?}",
        mates(&main, Tab::History)
    );
    // 前に出す: 別ウィンドウのタブはそのウィンドウの viewport を返し、無いタブ（ポーズ）は既定の組（レイヤーの組）へ開く
    let properties_window = outside.windows[0].viewport_id();
    assert_eq!(
        outside.show(&mut main, Tab::Properties),
        Some(properties_window)
    );
    assert_eq!(outside.show(&mut main, Tab::Canvas), None);
    assert_eq!(outside.show(&mut main, Tab::Pose), None);
    assert!(mates(&main, Tab::Pose).contains(&Tab::Layers));
    // 主のドックが空でも、戻すと組ができる
    let mut empty = DockState::new(vec![Tab::Canvas]);
    let mut out = detach::Detached::new();
    out.detach(&mut empty, Tab::Canvas, at);
    assert_eq!(empty.main_surface().num_tabs(), 0);
    out.return_tab(&mut empty, Tab::Canvas, None);
    assert_eq!(empty.main_surface().num_tabs(), 1);
}

#[test]
fn headless_the_window_menu_lists_the_panels_and_holds_the_layout_reset() {
    let mut app = AppState::new(64, 64);
    // 見出しは「表示」と「ヘルプ」の間に「ウィンドウ」
    assert_eq!(
        shell::menu_titles(yolu_app::lang::Lang::Ja)[6],
        "ウィンドウ"
    );
    assert_eq!(shell::menu_titles(yolu_app::lang::Lang::En)[6], "Window");
    assert_eq!(
        shell::menu_titles(yolu_app::lang::Lang::Ja)[shell::HELP_MENU],
        "ヘルプ"
    );
    assert_eq!(shell::MENU_TITLES.len(), 8);
    app.ui.panels.open = Tab::ALL
        .iter()
        .copied()
        .filter(|t| *t != Tab::Pose)
        .collect();
    let entries = shell::menu_entries(&app, shell::WINDOW_MENU);
    let labels: Vec<&str> = entries.iter().filter_map(|e| e.label()).collect();
    assert_eq!(
        labels,
        vec![
            "サブツール",
            "ツールプロパティ",
            "ブラシサイズ",
            "アセット",
            "チャンネル",
            "カラー",
            "カラーセット",
            "テクスチャセット",
            "ナビゲーター",
            "レイヤー",
            "ログ",
            "アクション",
            "プロパティ",
            "マテリアル",
            "ヒストリー",
            "キャンバス",
            "3D ビュー",
            "パネルの並びを戻す",
        ]
    );
    // パネルの名前はタブの名前と同じ・開いているパネルにはチェック
    for entry in &entries {
        if let yolu_app::ui::menu::Entry::Item {
            label,
            check,
            action: Action::Dock(DockOp::Show(tab)),
            ..
        } = entry
        {
            assert_eq!(label, tab.title_in(app.lang));
            assert_eq!(*check, yolu_app::ui::menu::Check::Checked, "{label}");
        }
    }
    app.ui.panels.open.retain(|t| *t != Tab::Log);
    let entries = shell::menu_entries(&app, shell::WINDOW_MENU);
    let log = entries.iter().find(|e| e.label() == Some("ログ")).unwrap();
    assert!(matches!(
        log,
        yolu_app::ui::menu::Entry::Item {
            check: yolu_app::ui::menu::Check::None,
            ..
        }
    ));
    // 「表示」には、もうパネルの並びを戻すが無い
    let view: Vec<String> = shell::menu_entries(&app, 5)
        .iter()
        .filter_map(|e| e.label().map(str::to_owned))
        .collect();
    assert!(!view.iter().any(|l| l.contains("パネルの並び")), "{view:?}");
    // 筆圧の調整は設定のウィンドウの「ペン」へ移した
    assert!(!view.iter().any(|l| l.contains("筆圧の調整")), "{view:?}");
    // タブの右クリック: メインウィンドウのタブは「別ウィンドウで開く」、別ウィンドウのただ 1 つのタブは「ドックに戻す」、ほかのタブもあるウィンドウなら両方
    let names = |app: &AppState, tab: Tab| -> Vec<String> {
        shell::popup_entries(app, yolu_app::state::PopupKind::DockTab(tab))
            .iter()
            .filter_map(|e| e.label().map(str::to_owned))
            .collect()
    };
    assert_eq!(names(&app, Tab::Layers), vec!["別ウィンドウで開く"]);
    app.ui.panels.outside = vec![Tab::Layers];
    app.ui.panels.alone = vec![Tab::Layers];
    assert_eq!(names(&app, Tab::Layers), vec!["ドックに戻す"]);
    app.ui.panels.outside = vec![Tab::Layers, Tab::Log];
    app.ui.panels.alone.clear();
    assert_eq!(
        names(&app, Tab::Layers),
        vec!["別ウィンドウで開く", "ドックに戻す"]
    );
    app.lang = yolu_app::lang::Lang::En;
    assert_eq!(
        names(&app, Tab::Layers),
        vec!["Open in New Window", "Return to Dock"]
    );
}

#[test]
fn headless_the_window_menu_lists_every_tab_so_a_new_tab_must_be_placed_in_it() {
    use std::collections::HashSet;
    let listed: HashSet<Tab> = detach::menu::PANELS.iter().copied().collect();
    assert_eq!(
        listed.len(),
        detach::menu::PANELS.len(),
        "同じパネルを 2 度並べない"
    );
    // ポーズはスキンのあるモデルを読んだときだけ足されるタブなので、並べない。それ以外のタブは、全部並べる（新しいタブを足したら、
    // 「ウィンドウ」の中の位置を決める）
    assert!(!listed.contains(&Tab::Pose));
    let missing: Vec<&str> = Tab::ALL
        .iter()
        .filter(|t| **t != Tab::Pose && !listed.contains(t))
        .map(|t| t.key())
        .collect();
    assert!(
        missing.is_empty(),
        "「ウィンドウ」に並んでいないタブ: {missing:?}（detach::menu::PANELS に位置を決めて足す）"
    );
}

#[test]
fn headless_one_of_several_windows_on_the_same_rect_is_picked_by_its_title_or_not_at_all() {
    let found = |items: &[(isize, &str)]| -> Vec<(isize, String)> {
        items.iter().map(|(h, t)| (*h, t.to_string())).collect()
    };
    // 1 つだけ合ったなら、それ（題名が違っても。ほかに重なったウィンドウが無い）
    assert_eq!(
        detach::pick_window(&found(&[(7, "x")]), "ヒストリー"),
        Some(7)
    );
    assert_eq!(detach::pick_window(&[], "ヒストリー"), None);
    // 同じ矩形に重なったウィンドウが複数あるなら、題名が同じ 1 つ（列挙の順に依らない）
    let two = found(&[(7, "ログ"), (9, "ヒストリー · プロパティ")]);
    assert_eq!(
        detach::pick_window(&two, "ヒストリー · プロパティ"),
        Some(9)
    );
    assert_eq!(detach::pick_window(&two, "ログ"), Some(7));
    // 題名で 1 つに決まらなければ繋がない（ほかのウィンドウのペンの点を受けない）
    assert_eq!(detach::pick_window(&two, "ナビゲーター"), None);
    let same = found(&[(7, "ログ"), (9, "ログ")]);
    assert_eq!(detach::pick_window(&same, "ログ"), None);
}

// ───────── 画面（別ウィンドウはメインウィンドウの中の egui のウィンドウ） ─────────

/// 別ウィンドウの中のタブの見出しの矩形（別ウィンドウはメインウィンドウの中の egui のウィンドウなので、メインウィンドウの点）。
fn outside_tab(h: &Harness<'static, YoluApp>, tab: Tab) -> Rect {
    h.state()
        .detached
        .windows
        .iter()
        .find_map(|w| w.tab_rects.get(&tab).copied())
        .expect("別ウィンドウのタブ")
}

fn right_click(h: &mut Harness<'static, YoluApp>, at: Pos2) {
    press(h, at, PointerButton::Secondary);
    h.step();
    release(h, at, PointerButton::Secondary);
    h.run();
}

/// 見出しを押して、`points` をなぞって離す（egui_dock がつかんだと決めるまで、少しずつ動かす）。
fn drag_tab(h: &mut Harness<'static, YoluApp>, from: Pos2, to: Pos2) {
    let mut points = vec![from];
    for i in 1..=12 {
        let t = i as f32 / 12.0;
        points.push(from + (to - from) * t);
    }
    drag(h, &points);
}

#[test]
fn the_tab_menu_opens_a_panel_in_a_new_window_and_returns_it_to_the_dock() {
    let mut h = app(1280.0, 800.0, 64);
    let tab = h.state().tab_rects[&Tab::History];
    right_click(&mut h, tab.center());
    assert!(matches!(
        h.state().state.popup.as_ref().map(|p| p.kind),
        Some(yolu_app::state::PopupKind::DockTab(Tab::History))
    ));
    // egui_dock の英語の「Eject」は出さない
    use egui_kittest::kittest::Queryable;
    assert!(h.query_by_label("Eject").is_none());
    let item = popup_item(&h, "別ウィンドウで開く");
    click(&mut h, item.center());
    h.run();
    let app = h.state();
    assert!(
        app.dock.find_tab(&Tab::History).is_none(),
        "主のドックから消える"
    );
    assert_eq!(app.detached.windows.len(), 1);
    assert_eq!(app.detached.windows[0].tabs(), vec![Tab::History]);
    // 置き場所は、元の組の大きさ（最小と上限の間）
    let size = app.detached.windows[0].record.expect("置き場所").size;
    assert!(
        size[0] >= detach::MIN_SIZE[0] && size[1] >= detach::MIN_SIZE[1],
        "{size:?}"
    );
    assert_every_tab_once(&app.dock, &[&app.detached.windows[0].dock]);
    // 別ウィンドウ（試験ではメインウィンドウの中の egui のウィンドウ）に、ヒストリーの欄が描かれる
    let in_window = outside_tab(&h, Tab::History);
    right_click(&mut h, in_window.center());
    let item = popup_item(&h, "ドックに戻す");
    click(&mut h, item.center());
    h.run();
    let app = h.state();
    assert!(app.detached.windows.is_empty());
    assert_eq!(
        mates(&app.dock, Tab::History),
        vec![Tab::Properties, Tab::Material, Tab::History]
    );
}

#[test]
fn dropping_a_tab_outside_the_main_window_opens_it_in_a_new_window_and_dropping_it_on_the_dock_returns_it(
) {
    let mut h = app(1280.0, 800.0, 64);
    let tab = h.state().tab_rects[&Tab::Channels];
    // メインウィンドウの外（右）で離す
    drag_tab(&mut h, tab.center(), pos2(1500.0, 300.0));
    let app = h.state();
    assert_eq!(app.detached.windows.len(), 1, "別ウィンドウができる");
    assert_eq!(app.detached.windows[0].tabs(), vec![Tab::Channels]);
    // 離した点がタブの帯の下に来る置き場所（外枠の左上は、離した点から少し左上）
    let at = app.detached.windows[0].record.unwrap().position;
    assert_eq!(
        at,
        [
            1500.0 - detach::GRAB_OFFSET[0],
            300.0 - detach::GRAB_OFFSET[1]
        ]
    );
    assert!(app.dock.find_tab(&Tab::Channels).is_none());
    // 別ウィンドウのタブを、メインウィンドウのキャンバスの組の上で離すと、その組へ戻る
    let canvas_leaf = detach::leaf_rect(&h.state().dock, Tab::Canvas).unwrap();
    let from = outside_tab(&h, Tab::Channels).center();
    let to = canvas_leaf.center() + vec2(0.0, 80.0);
    drag_tab(&mut h, from, to);
    let app = h.state();
    assert!(
        app.detached.windows.is_empty(),
        "{:?}",
        app.detached.windows.len()
    );
    assert_eq!(
        mates(&app.dock, Tab::Channels),
        vec![Tab::Canvas, Tab::View3d, Tab::Channels]
    );
    assert_every_tab_once(&app.dock, &[]);
}

#[test]
fn a_floating_window_made_by_the_dock_becomes_an_outside_window_with_its_group() {
    let mut h = app(1280.0, 800.0, 64);
    // egui_dock がタブを浮いたウィンドウへ動かした（組の中ほどで離したときと同じ形）
    {
        let dock = &mut h.state_mut().dock;
        let path = dock.find_tab(&Tab::Color).unwrap();
        dock.detach_tab(
            path,
            Rect::from_min_size(pos2(500.0, 260.0), vec2(360.0, 280.0)),
        );
    }
    h.run();
    let app = h.state();
    assert_eq!(
        app.dock.surfaces_count(),
        1,
        "主のドックに浮いたウィンドウの面が残らない"
    );
    assert_eq!(app.detached.windows.len(), 1);
    assert_eq!(app.detached.windows[0].tabs(), vec![Tab::Color]);
    let r = app.detached.windows[0].record.unwrap();
    assert_eq!(r.position, [500.0, 260.0]);
    assert_every_tab_once(&app.dock, &[&app.detached.windows[0].dock]);
}

#[test]
fn the_window_menu_brings_a_panel_forward_and_reset_closes_the_outside_windows() {
    let mut h = app(1280.0, 800.0, 64);
    h.state_mut().apply(Action::Dock(DockOp::Detach(Tab::Log)));
    h.run();
    h.state_mut()
        .apply(Action::Dock(DockOp::Show(Tab::History)));
    h.run();
    // 「ウィンドウ」を開いて、ログ（別ウィンドウ）を選ぶと、そのウィンドウのタブが前に（ウィンドウを前へ出す頼みも送る）
    let at = menu_title(&h, "ウィンドウ").center();
    click(&mut h, at);
    let item = popup_item(&h, "ログ");
    click(&mut h, item.center());
    h.run();
    let app = h.state();
    assert!(app.detached.windows[0].dock.find_tab(&Tab::Log).is_some());
    // 主のドックの後ろのタブは前になる
    let at = menu_title(&h, "ウィンドウ").center();
    click(&mut h, at);
    let item = popup_item(&h, "テクスチャセット");
    click(&mut h, item.center());
    h.run();
    h.state_mut()
        .apply(Action::Dock(DockOp::Show(Tab::Channels)));
    h.run();
    let dock = &h.state().dock;
    let (node, index) = dock.find_main_surface_tab(&Tab::Channels).unwrap();
    assert_eq!(
        dock.leaf(egui_dock::NodePath {
            surface: SurfaceIndex::main(),
            node
        })
        .unwrap()
        .active,
        index
    );
    // パネルの並びを戻すと、別ウィンドウを閉じて既定の並び
    let at = menu_title(&h, "ウィンドウ").center();
    click(&mut h, at);
    let item = popup_item(&h, "パネルの並びを戻す");
    click(&mut h, item.center());
    h.run();
    let app = h.state();
    assert!(app.detached.windows.is_empty());
    assert_eq!(
        shape(&app.dock),
        shape(&yolu_app::app::default_dock_for(
            h.ctx.content_rect().width()
        ))
    );
}

/// 「ウィンドウ」を開いた所と、タブの右クリック（撮る）。
fn snapshot_menus(lang: yolu_app::lang::Lang, suffix: &str) {
    let mut h = app(1280.0, 800.0, 64);
    h.state_mut().state.lang = lang;
    h.run();
    let title = shell::menu_titles(lang)[shell::WINDOW_MENU];
    let at = menu_title(&h, title).center();
    click(&mut h, at);
    h.run();
    h.snapshot(format!("window_menu{suffix}"));
    h.state_mut().state.popup = None;
    h.run();
    let tab = h.state().tab_rects[&Tab::Layers];
    right_click(&mut h, tab.center());
    h.snapshot(format!("dock_tab_menu{suffix}"));
}

#[test]
fn snapshot_the_window_menu_and_the_tab_menu() {
    snapshot_menus(yolu_app::lang::Lang::Ja, "");
}

#[test]
fn snapshot_the_window_menu_and_the_tab_menu_in_english() {
    snapshot_menus(yolu_app::lang::Lang::En, "_english");
}

/// 設定のフォルダ（`layout.json` の置き場）を持つウィンドウ（別ウィンドウはメインウィンドウの中の egui のウィンドウ）。
fn app_with_settings(settings: &Path) -> Harness<'static, YoluApp> {
    let settings = settings.to_path_buf();
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(1280.0, 800.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .renderer(common::shared_gpu::renderer())
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

#[test]
fn outside_windows_are_remembered_and_come_back_at_the_next_start() {
    use eframe::App;
    let dir = settings_dir("remember");
    let settings = dir.join("settings.conf");
    let open = || app_with_settings(&settings);
    let mut h = open();
    h.state_mut()
        .apply(Action::Dock(DockOp::Detach(Tab::History)));
    h.run();
    // ウィンドウの置き場所（OS のウィンドウなら利用者が動かした所）
    h.state_mut().detached.windows[0].record = Some(record(1650.0, 140.0, 420.0, 360.0));
    h.state_mut().on_exit();
    drop(h);
    let h = open();
    let app = h.state();
    assert_eq!(app.detached.windows.len(), 1);
    assert_eq!(app.detached.windows[0].tabs(), vec![Tab::History]);
    assert_eq!(
        app.detached.windows[0].home,
        vec![Tab::Properties, Tab::Material]
    );
    assert_eq!(
        app.detached.windows[0].record,
        Some(record(1650.0, 140.0, 420.0, 360.0))
    );
    assert!(app.dock.find_tab(&Tab::History).is_none());
}

#[test]
fn dragging_the_only_tab_of_an_outside_window_inside_the_window_keeps_that_window_and_the_saved_layout(
) {
    use eframe::App;
    let dir = settings_dir("lone-drag");
    let settings = dir.join("settings.conf");
    let mut h = app_with_settings(&settings);
    h.state_mut()
        .apply(Action::Dock(DockOp::Detach(Tab::History)));
    h.run();
    // （外のウィンドウは、置いたあと 1 回描き直して矩形が決まる）
    h.run();
    let serial = h.state().detached.windows[0].serial;
    let leaf = detach::leaf_rect(&h.state().detached.windows[0].dock, Tab::History).expect("組");
    let from = outside_tab(&h, Tab::History).center();
    // 組の本体の、中ほど（組の真ん中の小さな範囲）の外で離す: egui_dock なら浮いたウィンドウの落とし先
    let to = leaf.min + vec2(leaf.width() * 0.2, leaf.height() * 0.35);
    drag_tab(&mut h, from, to);
    let app = h.state();
    assert_eq!(
        app.detached.windows.len(),
        1,
        "空のウィンドウも、新しいウィンドウも残らない"
    );
    assert_eq!(
        app.detached.windows[0].serial, serial,
        "同じウィンドウのまま"
    );
    assert_eq!(app.detached.windows[0].tabs(), vec![Tab::History]);
    assert_every_tab_once(&app.dock, &[&app.detached.windows[0].dock]);
    // 保存して読み直しても、並びが落ちずに戻る
    h.state_mut().on_exit();
    drop(h);
    let h = app_with_settings(&settings);
    let app = h.state();
    assert_eq!(app.detached.windows.len(), 1);
    assert_eq!(app.detached.windows[0].tabs(), vec![Tab::History]);
    assert!(app.dock.find_tab(&Tab::History).is_none());
}

#[test]
fn a_window_left_without_tabs_is_dropped_and_never_written_to_the_layout_file() {
    use eframe::App;
    let dir = settings_dir("empty-window");
    let settings = dir.join("settings.conf");
    let mut h = app_with_settings(&settings);
    h.state_mut().detached.open(
        DockState::new(Vec::new()),
        Vec::new(),
        Place::Record(record(200.0, 150.0, 360.0, 480.0)),
    );
    // フレームを回す前に書いても、タブの無いウィンドウは並びのファイルに入らない（読み直しの検証で並び全部が既定へ戻らない）
    h.state_mut().on_exit();
    let path = layout::path_for(&settings).expect("並びのファイル");
    let loaded = layout::load(&path);
    assert!(loaded.problems.is_empty(), "{:?}", loaded.problems);
    assert!(loaded.detached.is_empty());
    assert!(loaded.dock.is_some(), "主のドックは保たれる");
    // 描いたフレームの終わりに、タブの無いウィンドウは除かれる
    h.run();
    assert!(h.state().detached.windows.is_empty());
}

// ───────── 別ウィンドウを本物の別のパスとして回す ─────────

const ROOT_SIZE: [f32; 2] = [1280.0, 800.0];

/// 子の viewport を別のパスとして回すウィンドウ。メインウィンドウの内側は (0, 0) から。
fn app_with_viewports() -> (Harness<'static, YoluApp>, Driver) {
    let driver = Driver::default();
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(ROOT_SIZE[0], ROOT_SIZE[1]))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .renderer(driver.renderer(common::shared_gpu::renderer()))
        .build_eframe(move |cc| {
            with_render_state_cpu_canvas(
                YoluApp::for_context(&cc.egui_ctx, AppState::new(128, 128), PenInput::detached()),
                cc.wgpu_render_state.as_ref(),
            )
        });
    driver.install(&h.ctx);
    h.input_mut()
        .viewports
        .entry(ViewportId::ROOT)
        .or_default()
        .inner_rect = Some(Rect::from_min_size(Pos2::ZERO, ROOT_SIZE.into()));
    h.state_mut().state.bake.backend = yolu_app::bake::BakeBackend::Cpu;
    h.run();
    (h, driver)
}

/// タブを別ウィンドウへ出し、子ウィンドウを `inner`（仮想スクリーンの点）に置く。子の viewport を返す。
fn detach_to(
    h: &mut Harness<'static, YoluApp>,
    driver: &Driver,
    tab: Tab,
    inner: Rect,
) -> ViewportId {
    h.state_mut().apply(Action::Dock(DockOp::Detach(tab)));
    h.run();
    let id = h
        .state()
        .detached
        .windows
        .iter()
        .find(|w| w.dock.find_tab(&tab).is_some())
        .expect("別ウィンドウ")
        .viewport_id();
    driver.child(id, |c| {
        c.inner = Some(inner);
        c.focused = true;
    });
    sync(h, driver);
    h.run();
    id
}

/// 根の入力に、子ウィンドウの情報を写す（根のパスがウィンドウの位置とフォーカスを読む）。
fn sync(h: &mut Harness<'static, YoluApp>, driver: &Driver) {
    for (id, info) in driver.infos() {
        h.input_mut().viewports.insert(id, info);
    }
}

fn key_events(key: Key) -> Vec<Event> {
    [true, false]
        .into_iter()
        .map(|pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        })
        .collect()
}

#[test]
fn an_outside_window_runs_its_own_pass_and_takes_the_same_shortcuts() {
    let (mut h, driver) = app_with_viewports();
    let id = detach_to(
        &mut h,
        &driver,
        Tab::Layers,
        Rect::from_min_size(pos2(1400.0, 100.0), vec2(360.0, 480.0)),
    );
    driver.take_ran();
    h.run();
    assert!(driver.take_ran().contains(&id), "子ウィンドウのパスが回る");
    assert_eq!(h.state().state.tool, yolu_app::state::Tool::Brush);
    // 別ウィンドウに打ったキーは、メインウィンドウと同じ表で効く（E は消しゴム）
    driver.child(id, |c| c.events.extend(key_events(Key::E)));
    h.run();
    assert_eq!(h.state().state.tool, yolu_app::state::Tool::Eraser);
    // 別ウィンドウの中のタブの見出し（子ウィンドウの点）も控える
    assert!(h.state().detached.windows[0]
        .tab_rects
        .contains_key(&Tab::Layers));
}

/// 別ウィンドウの egui の事象は、主のフレームの初めには見えない（フレームの間隔の下限で、入力のあるフレームの 1/240 秒にしたい）。別ウィンドウのパスが
/// 入力を見たら、次の主のフレームの初めが入力のあるフレームとして数える印を残す。入力の無いパスは印を付けない。印は一度取り出したら消える。
#[test]
fn an_outside_window_that_saw_input_marks_the_next_main_frame_as_an_input_frame() {
    let (mut h, driver) = app_with_viewports();
    let id = detach_to(
        &mut h,
        &driver,
        Tab::Layers,
        Rect::from_min_size(pos2(1400.0, 100.0), vec2(360.0, 480.0)),
    );
    let empty = egui::RawInput::default();
    // 取り出して印を消す。入力の無い子のパスだけでは印は付かない
    h.state_mut().frame_has_input(&empty);
    driver.take_ran();
    h.run();
    assert!(driver.take_ran().contains(&id), "子ウィンドウのパスが回る");
    assert!(
        !h.state_mut().frame_has_input(&empty),
        "入力の無いパスは印を付けない"
    );
    // 別ウィンドウの上でポインタが動く
    driver.child(id, |c| {
        c.events.push(Event::PointerMoved(pos2(40.0, 40.0)));
    });
    h.run();
    assert!(
        h.state_mut().frame_has_input(&empty),
        "別ウィンドウが入力を見たら、次の主のフレームの初めは入力のあるフレーム"
    );
    assert!(
        !h.state_mut().frame_has_input(&empty),
        "一度取り出したら消える"
    );
}

#[test]
fn closing_an_outside_window_returns_its_tabs_to_the_dock() {
    let (mut h, driver) = app_with_viewports();
    let id = detach_to(
        &mut h,
        &driver,
        Tab::History,
        Rect::from_min_size(pos2(1400.0, 100.0), vec2(360.0, 480.0)),
    );
    driver.child(id, |c| c.close = true);
    h.run();
    let app = h.state();
    assert!(app.detached.windows.is_empty());
    assert_eq!(
        mates(&app.dock, Tab::History),
        vec![Tab::Properties, Tab::Material, Tab::History]
    );
    assert_every_tab_once(&app.dock, &[]);
}

#[test]
fn a_menu_opened_in_an_outside_window_is_drawn_and_chosen_there() {
    let (mut h, driver) = app_with_viewports();
    let id = detach_to(
        &mut h,
        &driver,
        Tab::History,
        Rect::from_min_size(pos2(1400.0, 100.0), vec2(360.0, 480.0)),
    );
    let tab = h.state().detached.windows[0].tab_rects[&Tab::History].center();
    driver.child(id, |c| {
        c.events.push(Event::PointerMoved(tab));
        for pressed in [true, false] {
            c.events.push(Event::PointerButton {
                pos: tab,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
    });
    h.run();
    let popup = h.state().state.popup.as_ref().expect("メニューが開く");
    assert_eq!(popup.state.viewport, id, "別ウィンドウに開く");
    assert!(popup.state.rect.is_positive(), "別ウィンドウのパスが描いた");
    // 別ウィンドウのキーで選ぶ（ただ 1 つのタブなので「ドックに戻す」だけ）
    driver.child(id, |c| {
        c.events.extend(key_events(Key::ArrowDown));
        c.events.extend(key_events(Key::Enter));
    });
    h.run();
    let app = h.state();
    assert!(app.state.popup.is_none());
    assert!(app.detached.windows.is_empty());
    assert!(app.dock.find_tab(&Tab::History).is_some());
}

/// コピー（Ctrl+C）として来るだけの事象を、Shift を押したあとのフレームで 1 つ渡す。結合したコピーになったか。
/// egui-winit は Ctrl+C を `Event::Copy` に置き換え、そのフレームに `ModifiersChanged` も `Key` も来ない。Shift は前のフレームの終わりの
/// 修飾で決まるので、別のウィンドウのパスがそれを上書きしてはいけない。
fn copy_after_shift_is_merged(
    h: &mut Harness<'static, YoluApp>,
    child: Option<(&Driver, ViewportId)>,
) -> bool {
    let os = yolu_app::clipboard::os::MemoryClipboard::new();
    h.state_mut().state.clip.set_os(Box::new(os));
    let layer = h.state().state.selected_layer.unwrap();
    for y in 10..30 {
        for x in 10..30 {
            h.state_mut()
                .state
                .doc
                .set_pixel(layer, x, y, yolu_app::engine::Rgba8::new(200, 10, 20, 255))
                .unwrap();
        }
    }
    h.state_mut().state.doc.clear_history().unwrap();
    let shift = Modifiers::COMMAND | Modifiers::SHIFT;
    match child {
        // フォーカスのあるウィンドウは外のウィンドウ（キーはそのウィンドウにだけ来る）
        Some((driver, id)) => {
            driver.child(id, |c| c.events.push(Event::ModifiersChanged(shift)));
            h.step();
            driver.child(id, |c| c.events.push(Event::Copy));
            h.step();
        }
        None => {
            h.event(Event::ModifiersChanged(shift));
            h.step();
            h.event(Event::Copy);
            h.step();
        }
    }
    h.run();
    h.state().state.clip.pixels.as_ref().map(|p| p.source())
        == Some(yolu_app::engine::ClipboardSource::Composite)
}

#[test]
fn an_unfocused_outside_window_does_not_lose_the_shift_of_the_focused_main_window() {
    let (mut h, driver) = app_with_viewports();
    let id = detach_to(
        &mut h,
        &driver,
        Tab::History,
        Rect::from_min_size(pos2(1400.0, 100.0), vec2(360.0, 480.0)),
    );
    // フォーカスはメインウィンドウにある
    driver.child(id, |c| c.focused = false);
    sync(&mut h, &driver);
    h.run();
    assert!(
        copy_after_shift_is_merged(&mut h, None),
        "外のウィンドウが開いていても、主のウィンドウの Ctrl+Shift+C は結合したコピー"
    );
}

#[test]
fn the_focused_outside_window_keeps_its_own_shift_for_the_merged_copy() {
    let (mut h, driver) = app_with_viewports();
    let id = detach_to(
        &mut h,
        &driver,
        Tab::History,
        Rect::from_min_size(pos2(1400.0, 100.0), vec2(360.0, 480.0)),
    );
    // フォーカスは外のウィンドウにある（主のウィンドウのパスは先に回る）
    assert!(
        copy_after_shift_is_merged(&mut h, Some((&driver, id))),
        "外のウィンドウの Ctrl+Shift+C は結合したコピー"
    );
}

#[test]
fn a_tab_dropped_from_an_outside_window_onto_the_main_window_joins_the_group_under_the_pointer() {
    let (mut h, driver) = app_with_viewports();
    let inner = Rect::from_min_size(pos2(1400.0, 100.0), vec2(360.0, 480.0));
    let id = detach_to(&mut h, &driver, Tab::Channels, inner);
    let from = h.state().detached.windows[0].tab_rects[&Tab::Channels].center();
    // メインウィンドウのキャンバスの組（メインウィンドウの点）を、子ウィンドウの点で言う
    let target = detach::leaf_rect(&h.state().dock, Tab::Canvas)
        .unwrap()
        .center();
    let to = target - inner.min.to_vec2();
    driver.child(id, |c| {
        c.events.push(Event::PointerMoved(from));
        c.events.push(Event::PointerButton {
            pos: from,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
    });
    h.step();
    for i in 1..=10 {
        let p = from + (to - from) * (i as f32 / 10.0);
        driver.child(id, |c| c.events.push(Event::PointerMoved(p)));
        h.step();
    }
    driver.child(id, |c| {
        c.events.push(Event::PointerButton {
            pos: to,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
    });
    h.run();
    let app = h.state();
    assert!(app.detached.windows.is_empty());
    assert_eq!(
        mates(&app.dock, Tab::Channels),
        vec![Tab::Canvas, Tab::Channels]
    );
}

#[test]
fn the_canvas_in_an_outside_window_still_draws() {
    let (mut h, driver) = app_with_viewports();
    let id = detach_to(
        &mut h,
        &driver,
        Tab::Canvas,
        Rect::from_min_size(pos2(1300.0, 0.0), vec2(700.0, 600.0)),
    );
    h.run();
    let rect = h
        .state()
        .canvas_view_rect()
        .expect("別ウィンドウでキャンバスを描いた");
    assert!(
        rect.max.x <= 700.0 && rect.max.y <= 600.0,
        "子ウィンドウの点: {rect:?}"
    );
    let c = rect.center();
    let before = h.state().state.can_undo();
    driver.child(id, |c2| {
        c2.events.push(Event::PointerMoved(c));
        c2.events.push(Event::PointerButton {
            pos: c,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
    });
    h.step();
    for dx in [10.0, 20.0, 30.0] {
        driver.child(id, |c2| {
            c2.events.push(Event::PointerMoved(c + vec2(dx, 0.0)))
        });
        h.step();
    }
    driver.child(id, |c2| {
        c2.events.push(Event::PointerButton {
            pos: c + vec2(30.0, 0.0),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
    });
    h.run();
    assert!(
        !before && h.state().state.can_undo(),
        "別ウィンドウのキャンバスに描けた"
    );
}

// ───────── 自前の枠（Windows。Linux でも `set_custom_frame(true)` で描いて確かめる） ─────────

/// 自前の枠のウィンドウに、別ウィンドウを 1 つ（子ウィンドウの内側は `OUTSIDE`）。
const OUTSIDE: Rect = Rect {
    min: pos2(1400.0, 100.0),
    max: pos2(1760.0, 580.0),
};

fn framed_with_viewports(
    custom: bool,
    tab: Tab,
) -> (Harness<'static, YoluApp>, Driver, ViewportId) {
    let (mut h, driver) = app_with_viewports();
    h.state_mut().set_custom_frame(custom);
    h.run();
    let id = detach_to(&mut h, &driver, tab, OUTSIDE);
    h.run();
    (h, driver, id)
}

/// 子ウィンドウに入力を 1 つずつ 1 フレームで渡し、そのフレームごとに子ウィンドウとメインウィンドウへ送った頼みを集める。
fn play_child(
    h: &mut Harness<'static, YoluApp>,
    driver: &Driver,
    id: ViewportId,
    events: Vec<Event>,
) -> (Vec<ViewportCommand>, Vec<ViewportCommand>) {
    play_child_groups(h, driver, id, events.into_iter().map(|e| vec![e]).collect())
}

/// 子ウィンドウに入力を、グループごとに 1 フレームで渡す（winit は、ペン・指の接触の始まりの Touch と、その押しの代わりの入力を、同じ入力のまとまりに入れる）。
fn play_child_groups(
    h: &mut Harness<'static, YoluApp>,
    driver: &Driver,
    id: ViewportId,
    groups: Vec<Vec<Event>>,
) -> (Vec<ViewportCommand>, Vec<ViewportCommand>) {
    let mut child = Vec::new();
    let mut root = Vec::new();
    for group in groups {
        driver.child(id, |c| c.events.extend(group));
        h.step();
        let out = h.output();
        let take = |v: ViewportId| {
            out.viewport_output
                .get(&v)
                .map(|o| o.commands.clone())
                .unwrap_or_default()
        };
        child.extend(take(id));
        root.extend(take(ViewportId::ROOT));
    }
    (child, root)
}

fn child_button(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

fn child_drag(from: Pos2, by: egui::Vec2) -> Vec<Event> {
    let mut events = vec![Event::PointerMoved(from), child_button(from, true)];
    for i in 1..=6 {
        events.push(Event::PointerMoved(from + by * (i as f32 / 6.0)));
    }
    events.push(child_button(from + by, false));
    events
}

fn child_double_click(at: Pos2) -> Vec<Event> {
    vec![
        Event::PointerMoved(at),
        child_button(at, true),
        child_button(at, false),
        child_button(at, true),
        child_button(at, false),
    ]
}

/// 別ウィンドウのタブの行の、何も無い所（タブの右、閉じるの左。子ウィンドウの点）。
fn empty_row_point(h: &Harness<'static, YoluApp>, tab: Tab) -> Pos2 {
    let tab = h.state().detached.windows[0].tab_rects[&tab];
    let x = tab.right() + 40.0;
    assert!(
        x < OUTSIDE.width() - yolu_app::titlebar::BUTTON_WIDTH - 10.0,
        "行に何も無い所が要る: {tab:?}"
    );
    pos2(x, tab.center().y)
}

/// 別ウィンドウの行の右端の閉じる（子ウィンドウの点）。
fn close_point() -> Pos2 {
    yolu_app::titlebar::close_rect(Rect::from_min_size(
        Pos2::ZERO,
        vec2(OUTSIDE.width(), yolu_app::ui::theme::PANEL_HEADER_HEIGHT),
    ))
    .center()
}

fn starts_drag(c: &ViewportCommand) -> bool {
    matches!(c, ViewportCommand::StartDrag)
}

#[test]
fn with_the_custom_frame_the_empty_tab_row_moves_and_maximizes_the_outside_window() {
    let (mut h, driver, id) = framed_with_viewports(true, Tab::History);
    let at = empty_row_point(&h, Tab::History);
    let (child, root) = play_child(&mut h, &driver, id, child_drag(at, vec2(60.0, 30.0)));
    assert_eq!(
        child.iter().filter(|c| starts_drag(c)).count(),
        1,
        "{child:?}"
    );
    assert!(
        !root.iter().any(starts_drag),
        "メインウィンドウは動かさない: {root:?}"
    );
    let (child, _) = play_child(&mut h, &driver, id, child_double_click(at));
    assert!(
        child
            .iter()
            .any(|c| matches!(c, ViewportCommand::Maximized(true))),
        "{child:?}"
    );
    assert!(!child.iter().any(starts_drag), "ダブルクリックは動かさない");
    assert_eq!(h.state().detached.windows.len(), 1, "ウィンドウはそのまま");
}

#[test]
fn with_the_custom_frame_dragging_a_tab_still_moves_the_tab_not_the_window() {
    let (mut h, driver, id) = framed_with_viewports(true, Tab::Channels);
    let from = h.state().detached.windows[0].tab_rects[&Tab::Channels].center();
    // メインウィンドウのキャンバスの組（メインウィンドウの点）を、子ウィンドウの点で言う
    let target = detach::leaf_rect(&h.state().dock, Tab::Canvas)
        .unwrap()
        .center();
    let to = target - OUTSIDE.min.to_vec2();
    let (child, _) = play_child(&mut h, &driver, id, child_drag(from, to - from));
    h.run();
    assert!(!child.iter().any(starts_drag), "{child:?}");
    let app = h.state();
    assert!(
        app.detached.windows.is_empty(),
        "タブはメインウィンドウへ戻る"
    );
    assert_eq!(
        mates(&app.dock, Tab::Channels),
        vec![Tab::Canvas, Tab::Channels]
    );
}

#[test]
fn with_the_custom_frame_the_close_button_returns_the_tabs_to_their_group() {
    let (mut h, driver, id) = framed_with_viewports(true, Tab::History);
    let at = close_point();
    let (child, _) = play_child(
        &mut h,
        &driver,
        id,
        vec![
            Event::PointerMoved(at),
            child_button(at, true),
            child_button(at, false),
        ],
    );
    h.run();
    assert!(!child.iter().any(starts_drag));
    let app = h.state();
    assert!(app.detached.windows.is_empty());
    assert_eq!(
        mates(&app.dock, Tab::History),
        vec![Tab::Properties, Tab::Material, Tab::History]
    );
    assert_every_tab_once(&app.dock, &[]);
}

#[test]
fn with_the_custom_frame_the_edges_of_the_outside_window_begin_a_resize() {
    use egui::ResizeDirection::*;
    let (mut h, driver, id) = framed_with_viewports(true, Tab::History);
    let (w, hh) = (OUTSIDE.width(), OUTSIDE.height());
    let row = empty_row_point(&h, Tab::History);
    for (at, want) in [
        (pos2(1.0, hh / 2.0), Some(West)),
        (pos2(w - 1.0, hh / 2.0), Some(East)),
        (pos2(w / 2.0, hh - 1.0), Some(South)),
        (pos2(w - 1.0, hh - 1.0), Some(SouthEast)),
        // 上の縁: 行の何も無い所（帯の代わり）の上は変える
        (pos2(row.x, 1.0), Some(North)),
        // 内側は変えない
        (pos2(w / 2.0, hh / 2.0), None),
    ] {
        let (child, root) = play_child(
            &mut h,
            &driver,
            id,
            vec![
                Event::PointerMoved(at),
                child_button(at, true),
                child_button(at, false),
            ],
        );
        let got: Vec<_> = child
            .iter()
            .filter_map(|c| match c {
                ViewportCommand::BeginResize(d) => Some(*d),
                _ => None,
            })
            .collect();
        assert_eq!(got, want.into_iter().collect::<Vec<_>>(), "{at:?}");
        assert!(
            !root
                .iter()
                .any(|c| matches!(c, ViewportCommand::BeginResize(_))),
            "メインウィンドウの大きさは変えない"
        );
    }
    // 最大化中は縁が無く、行のダブルクリックは元に戻す
    driver.child(id, |c| c.maximized = true);
    let at = pos2(1.0, hh / 2.0);
    let (child, _) = play_child(
        &mut h,
        &driver,
        id,
        vec![
            Event::PointerMoved(at),
            child_button(at, true),
            child_button(at, false),
        ],
    );
    assert!(
        !child
            .iter()
            .any(|c| matches!(c, ViewportCommand::BeginResize(_))),
        "{child:?}"
    );
    // 前の押しと続けて数えない（3 度目の押しはダブルクリックにならない）だけ間を置く
    for _ in 0..45 {
        h.step();
    }
    let (child, _) = play_child(&mut h, &driver, id, child_double_click(row));
    assert!(
        child
            .iter()
            .any(|c| matches!(c, ViewportCommand::Maximized(false))),
        "{child:?}"
    );
}

#[test]
fn without_the_custom_frame_the_tab_row_and_the_right_end_do_nothing_to_the_window() {
    let (mut h, driver, id) = framed_with_viewports(false, Tab::History);
    let at = empty_row_point(&h, Tab::History);
    let (child, _) = play_child(&mut h, &driver, id, child_drag(at, vec2(60.0, 30.0)));
    assert!(!child.iter().any(starts_drag), "{child:?}");
    let at = close_point();
    let (child, _) = play_child(
        &mut h,
        &driver,
        id,
        vec![
            Event::PointerMoved(pos2(1.0, OUTSIDE.height() / 2.0)),
            child_button(pos2(1.0, OUTSIDE.height() / 2.0), true),
            child_button(pos2(1.0, OUTSIDE.height() / 2.0), false),
            Event::PointerMoved(at),
            child_button(at, true),
            child_button(at, false),
        ],
    );
    h.run();
    assert!(
        !child
            .iter()
            .any(|c| matches!(c, ViewportCommand::BeginResize(_))),
        "{child:?}"
    );
    assert_eq!(h.state().detached.windows.len(), 1, "閉じるは無い");
}

// ───────── 帯をペン・指で引く（Windows。Linux でも手の差し替えで確かめる） ─────────

/// 別ウィンドウ 1 つ目のペンの受け口に、試験の手を付ける。
fn give_mover(h: &mut Harness<'static, YoluApp>, mover: &Arc<TestMover>) {
    let mover: Arc<dyn yolu_app::pen::WindowMover> = mover.clone();
    h.state_mut().detached.windows[0].set_pen_mover(Some(mover));
}

/// ペンや指の引き: 触れた入力（Touch の始まり）と押しを同じフレームで渡し、離さずに引く。
fn child_pen_hold(from: Pos2, by: egui::Vec2) -> Vec<Vec<Event>> {
    let mut groups = vec![vec![
        touch_start(from),
        Event::PointerMoved(from),
        child_button(from, true),
    ]];
    for i in 1..=6 {
        groups.push(vec![Event::PointerMoved(from + by * (i as f32 / 6.0))]);
    }
    groups
}

/// ペンや指の引き（離すまで）。
fn child_pen_drag(from: Pos2, by: egui::Vec2) -> Vec<Vec<Event>> {
    let mut groups = child_pen_hold(from, by);
    groups.push(vec![child_button(from + by, false)]);
    groups
}

fn starts(commands: &[ViewportCommand]) -> usize {
    commands.iter().filter(|c| starts_drag(c)).count()
}

#[test]
fn with_the_custom_frame_a_pen_drag_on_the_empty_tab_row_is_moved_by_the_app_not_by_the_os_loop() {
    let (mut h, driver, id) = framed_with_viewports(true, Tab::History);
    let mover = TestMover::new(true);
    give_mover(&mut h, &mover);
    let at = empty_row_point(&h, Tab::History);
    let (child, root) =
        play_child_groups(&mut h, &driver, id, child_pen_drag(at, vec2(60.0, 30.0)));
    assert_eq!(mover.calls(), 1, "引き始めでアプリの手を 1 度呼ぶ");
    assert_eq!(
        starts(&child),
        0,
        "ペンの引きは OS の移動の輪に渡さない: {child:?}"
    );
    assert_eq!(starts(&root), 0, "{root:?}");
    assert_eq!(mover.handed(), 0, "輪に渡していない");
    assert_eq!(h.state().detached.windows.len(), 1, "ウィンドウはそのまま");
}

/// 触れているポインタが分からなくて動かせなくても（点が途絶えた・指など）、ペン・指の押しは OS の移動の輪に渡さない: 元の不具合の道に戻らない。
#[test]
fn a_pen_or_finger_press_never_goes_to_the_os_loop_even_when_the_app_cannot_move_the_window() {
    let (mut h, driver, id) = framed_with_viewports(true, Tab::History);
    let mover = TestMover::new(false);
    give_mover(&mut h, &mover);
    let at = empty_row_point(&h, Tab::History);
    let (child, _) = play_child_groups(&mut h, &driver, id, child_pen_drag(at, vec2(60.0, 30.0)));
    assert_eq!(mover.calls(), 1);
    assert_eq!(starts(&child), 0, "{child:?}");
    assert_eq!(mover.handed(), 0);
    // 止めたペン: 押してしばらく動かさず（点が途絶える）から動かしても、アプリの側で動かす（OS の移動の輪に戻らない）
    let before = mover.calls();
    let (mut seen, _) = play_child_groups(
        &mut h,
        &driver,
        id,
        vec![vec![
            touch_start(at),
            Event::PointerMoved(at),
            child_button(at, true),
        ]],
    );
    for _ in 0..30 {
        h.step();
        seen.extend(
            h.output()
                .viewport_output
                .get(&id)
                .map(|o| o.commands.clone())
                .unwrap_or_default(),
        );
    }
    let (moved, _) = play_child(
        &mut h,
        &driver,
        id,
        vec![
            Event::PointerMoved(at + vec2(20.0, 0.0)),
            Event::PointerMoved(at + vec2(40.0, 10.0)),
            child_button(at + vec2(40.0, 10.0), false),
        ],
    );
    seen.extend(moved);
    assert_eq!(starts(&seen), 0, "{seen:?}");
    assert_eq!(
        mover.calls(),
        before + 1,
        "止めたあとに動かしたとき、手を呼ぶ"
    );
}

#[test]
fn a_mouse_drag_still_starts_the_os_move_and_tells_the_watch() {
    let (mut h, driver, id) = framed_with_viewports(true, Tab::History);
    let mover = TestMover::new(true);
    give_mover(&mut h, &mover);
    let at = empty_row_point(&h, Tab::History);
    let (child, _) = play_child(&mut h, &driver, id, child_drag(at, vec2(60.0, 30.0)));
    assert_eq!(starts(&child), 1, "{child:?}");
    assert_eq!(mover.calls(), 0, "マウスの引きでは、アプリの手を呼ばない");
    assert_eq!(mover.handed(), 1, "輪に渡したと見張りに伝える");
    assert!(
        mover.polls() >= 6,
        "見張りは毎フレーム呼ばれる: {}",
        mover.polls()
    );
    // 手の無い受け口（Windows 以外・繋ぐ前）の引きは、ペンの接触でも今までどおり StartDrag
    h.state_mut().detached.windows[0].set_pen_mover(None);
    let (child, _) = play_child_groups(&mut h, &driver, id, child_pen_drag(at, vec2(60.0, 30.0)));
    assert_eq!(starts(&child), 1, "{child:?}");
    let (child, _) = play_child(&mut h, &driver, id, child_drag(at, vec2(60.0, 30.0)));
    assert_eq!(starts(&child), 1, "{child:?}");
}

/// 補った離し（`WM_LBUTTONUP` → winit のマウスの離し → egui のポインタの離し）が届いたあとは、次のマウスの引きでまた StartDrag が出る。
#[test]
fn after_a_pen_press_was_taken_and_released_for_it_the_next_mouse_drag_starts_the_os_move_again() {
    let (mut h, driver, id) = framed_with_viewports(true, Tab::History);
    let mover = TestMover::new(true);
    give_mover(&mut h, &mover);
    let at = empty_row_point(&h, Tab::History);
    // ペンで引いている途中（離しは来ない）
    let (child, _) = play_child_groups(&mut h, &driver, id, child_pen_hold(at, vec2(60.0, 30.0)));
    assert_eq!(starts(&child), 0, "{child:?}");
    // 補われた離し
    let (child, _) = play_child(
        &mut h,
        &driver,
        id,
        vec![child_button(at + vec2(60.0, 30.0), false)],
    );
    assert_eq!(starts(&child), 0, "{child:?}");
    // 次はマウス
    let (child, _) = play_child(&mut h, &driver, id, child_drag(at, vec2(60.0, 30.0)));
    assert_eq!(starts(&child), 1, "{child:?}");
}

/// ペンの押しの離しが egui に一度も届かなくても、次のマウスの押しは、押し出しの出どころ（Touch の無い押し）を見て、マウスの引きとして StartDrag を出す。
/// ただし egui は、離しが来るまで前の引きを続けているので、マウスの 1 度目の引きは StartDrag にならず、押しと離しを 1 度挟んだ 2 度目で出る
/// （これが、ペンの受け口の側で離しを補う理由）。
#[test]
fn without_the_release_the_first_mouse_drag_after_a_pen_press_does_not_start_the_os_move() {
    let (mut h, driver, id) = framed_with_viewports(true, Tab::History);
    let mover = TestMover::new(true);
    give_mover(&mut h, &mover);
    let at = empty_row_point(&h, Tab::History);
    let _ = play_child_groups(&mut h, &driver, id, child_pen_hold(at, vec2(60.0, 30.0)));
    let (first, _) = play_child(&mut h, &driver, id, child_drag(at, vec2(60.0, 30.0)));
    let (second, _) = play_child(&mut h, &driver, id, child_drag(at, vec2(60.0, 30.0)));
    assert_eq!(starts(&first), 0, "{first:?}");
    assert_eq!(starts(&second), 1, "{second:?}");
    assert_eq!(
        mover.calls(),
        1,
        "手を呼んだのはペンの引きの 1 度だけ（マウスの押しは、ペンの押しとして数えない）"
    );
}

/// ペンでタブを引いて別ウィンドウにし、その帯をペンで引いて動かしたあとも、マウスでもペンでもまた引ける（ペンの引きは OS の移動の輪に渡さないので、輪の印が残らない）。
#[test]
fn after_a_tab_is_torn_off_by_the_pen_its_bar_can_be_dragged_by_the_pen_and_then_by_the_mouse() {
    let (mut h, driver) = app_with_viewports();
    h.state_mut().set_custom_frame(true);
    h.run();
    // ペンでタブの見出しを押して外へ引いて離す
    let tab = h.state().tab_rects[&Tab::Channels];
    let sample = |at: Pos2, contact: bool| PenSample {
        pos: [at.x, at.y],
        pressure: if contact { 0.5 } else { 0.0 },
        tilt: yolu_app::engine::Tilt::default(),
        rotation: None,
        contact,
        eraser: false,
        barrel: false,
        pointer_id: 5,
        time_ms: 0,
    };
    let (from, to) = (tab.center(), pos2(1500.0, 300.0));
    h.state().pen().push(sample(from, true));
    drag_tab(&mut h, from, to);
    h.state().pen().push(sample(to, false));
    h.run();
    assert_eq!(h.state().detached.windows.len(), 1, "別ウィンドウができる");
    let id = h.state().detached.windows[0].viewport_id();
    driver.child(id, |c| {
        c.inner = Some(OUTSIDE);
        c.focused = true;
    });
    sync(&mut h, &driver);
    h.run();
    // その帯をペンで引く → OS の移動の輪ではなく、アプリの側で動かす
    let mover = TestMover::new(true);
    give_mover(&mut h, &mover);
    let at = empty_row_point(&h, Tab::Channels);
    let (child, _) = play_child_groups(&mut h, &driver, id, child_pen_drag(at, vec2(60.0, 30.0)));
    assert_eq!(mover.calls(), 1);
    assert_eq!(starts(&child), 0, "{child:?}");
    // もう一度ペンで引ける
    let (child, _) = play_child_groups(&mut h, &driver, id, child_pen_drag(at, vec2(-40.0, 20.0)));
    assert_eq!(mover.calls(), 2);
    assert_eq!(starts(&child), 0, "{child:?}");
    // マウスで引くと OS の移動になる
    let (child, _) = play_child(&mut h, &driver, id, child_drag(at, vec2(60.0, 30.0)));
    assert_eq!(starts(&child), 1, "{child:?}");
    assert_eq!(mover.calls(), 2, "マウスの引きでは手を呼ばない");
    assert_eq!(h.state().detached.windows.len(), 1, "ウィンドウはそのまま");
}

/// 絵: 自前の枠の別ウィンドウ（試験のウィンドウではメインウィンドウの中に描く）。タブの行の右端に閉じる（名前は「ドックに戻す」）。
fn snapshot_frame(lang: yolu_app::lang::Lang, suffix: &str) {
    let mut h = app(1280.0, 800.0, 64);
    h.state_mut().state.lang = lang;
    h.state_mut().set_custom_frame(true);
    h.run();
    h.state_mut()
        .apply(Action::Dock(DockOp::Detach(Tab::Layers)));
    h.run();
    let app = h.state_mut();
    let serial = app.detached.windows[0].serial;
    assert!(app
        .detached
        .move_into(&mut app.dock, Tab::Log, serial, None));
    detach::activate(&mut app.detached.windows[0].dock, Tab::Layers);
    // 置き場所（まだ描いていなければ最初の置き場所から、描いた後なら記録から）
    let at = record(700.0, 200.0, 380.0, 440.0);
    app.detached.windows[0].place = Place::Record(at);
    app.detached.windows[0].record = Some(at);
    h.run();
    assert_eq!(h.state().detached.windows[0].record, Some(at));
    use egui_kittest::kittest::Queryable;
    let name = lang.pick("ドックに戻す", "Return to Dock");
    assert!(h.query_by_label(name).is_some(), "閉じるの名前");
    h.snapshot(format!("detached_frame{suffix}"));
}

#[test]
fn snapshot_the_outside_window_with_the_custom_frame() {
    snapshot_frame(yolu_app::lang::Lang::Ja, "");
}

#[test]
fn snapshot_the_outside_window_with_the_custom_frame_in_english() {
    snapshot_frame(yolu_app::lang::Lang::En, "_english");
}
