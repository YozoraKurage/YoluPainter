//! 画面の並びの保存と復元（設定のフォルダの `layout.json`）: ドックの並び（タブの組・分け方・大きさ・どのタブが前か）と、ウィンドウの大きさ・位置・
//! 最大化を、終わるときに書き、次の起動で戻す。読めない・古い版・知らないタブ・足りない／重なるタブは捨てて既定の並び（理由は画面に出さない）。
//! 「ウィンドウ → パネルの並びを戻す」は既定へ。ウィンドウの大きさと位置は、ドックとは別に確かめる。
use crate::common;

use std::path::{Path, PathBuf};

use common::*;
use egui::{pos2, vec2, Rect};
use egui_dock::{DockState, Node, Split, SurfaceIndex, TabDestination, TabInsert};
use egui_kittest::Harness;
use yolu_app::app::{default_dock, default_dock_for};
use yolu_app::layout::{self, WindowRecord};
use yolu_app::pen::PenInput;
use yolu_app::state::Action;
use yolu_app::{Tab, YoluApp};

fn settings_dir(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/layout-tests")
        .join(std::process::id().to_string())
        .join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn layout_file(dir: &Path) -> PathBuf {
    dir.join(layout::FILE_NAME)
}

/// 読む大きさの上限（`layout::MAX_FILE_BYTES`。これを超えるファイルは壊れているとして読まない）。
const MAX_FILE_BYTES: usize = 1024 * 1024;

/// JSON のあとに空白を足して、ちょうど `bytes` バイトにする（JSON としては正しいまま）。
fn padded(json: &str, bytes: usize) -> String {
    assert!(json.len() <= bytes);
    format!("{json}{}", " ".repeat(bytes - json.len()))
}

fn app_in(dir: &Path) -> Harness<'static, YoluApp> {
    app_in_with(dir, false)
}

/// `fit` は、起動のあとウィンドウが画面より大きければ収める（実際のウィンドウの動き）。
fn app_in_with(dir: &Path, fit: bool) -> Harness<'static, YoluApp> {
    let settings = dir.join("settings.conf");
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
                )
                .fit_to_screen(fit),
                cc.wgpu_render_state.as_ref(),
            )
        });
    h.run();
    h
}

/// ドックの並びを、描いた大きさ（矩形）に依らない形にする: どのタブがどの組か・組の中の順・前のタブ・分け方と割合。
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
            Node::Horizontal(split) => out.push(format!("{at} horizontal {:.4}", split.fraction)),
            Node::Vertical(split) => out.push(format!("{at} vertical {:.4}", split.fraction)),
        }
    }
    out
}

/// 既定の並びから、少し動かした並び（カラーをレイヤーの組へ・その組の前をカラーに・右の列の区切りを動かす）。
fn changed_dock() -> DockState<Tab> {
    let mut dock = default_dock();
    let from = dock.find_tab(&Tab::Color).expect("カラー");
    let to = dock.find_tab(&Tab::Layers).expect("レイヤー").node_path();
    dock.move_tab(from, TabDestination::Node(to, TabInsert::Append));
    let color = dock.find_tab(&Tab::Color).expect("カラー（動かしたあと）");
    dock.set_active_tab(color).expect("前へ");
    // 中央の列と右の列の区切り
    for (_, node) in dock.iter_all_nodes_mut() {
        if let Node::Horizontal(split) = node {
            split.fraction = (split.fraction - 0.07).max(0.1);
            break;
        }
    }
    dock
}

/// 既定の並びから、履歴を浮かせたウィンドウにして（面 1）、チャンネルもそのウィンドウへ入れ、前のタブをチャンネルにした並び。
fn floating_dock() -> DockState<Tab> {
    let mut dock = default_dock();
    let history = dock.find_tab(&Tab::History).expect("履歴");
    let surface = dock.detach_tab(
        history,
        Rect::from_min_size(pos2(300.0, 200.0), vec2(320.0, 240.0)),
    );
    assert_eq!(surface, SurfaceIndex(1));
    let navigator = dock.find_tab(&Tab::Channels).expect("チャンネル");
    let window_leaf = dock
        .find_tab(&Tab::History)
        .expect("履歴（浮かせたあと）")
        .node_path();
    dock.move_tab(
        navigator,
        TabDestination::Node(window_leaf, TabInsert::Append),
    );
    let navigator = dock
        .find_tab(&Tab::Channels)
        .expect("チャンネル（動かしたあと）");
    dock.set_active_tab(navigator).expect("前へ");
    dock
}

/// 浮かせたウィンドウの状態（面の番号）を、書き出した形の JSON で。
fn window_state(dock: &mut DockState<Tab>, surface: usize) -> serde_json::Value {
    serde_json::to_value(
        dock.get_window_state(SurfaceIndex(surface))
            .expect("浮かせたウィンドウ"),
    )
    .unwrap()
}

fn window(maximized: bool) -> WindowRecord {
    WindowRecord {
        position: [120.0, 80.0],
        size: [1400.0, 900.0],
        pixels_per_point: 1.0,
        maximized,
    }
}

// ───────── 形（読み書き） ─────────

#[test]
fn headless_the_default_dock_and_a_changed_dock_survive_the_file_with_every_tab_placed() {
    for dock in [default_dock(), changed_dock()] {
        let text = layout::render(&dock, Some(&window(false)));
        let loaded = layout::parse(&text);
        assert_eq!(loaded.problems, Vec::<String>::new());
        let read = loaded.dock.expect("読めた");
        assert_eq!(
            shape(&read),
            shape(&dock),
            "組・順・前のタブ・分け方と割合が同じ"
        );
        assert!(layout::validate(&read).is_ok());
        assert_eq!(loaded.window, Some(window(false)));
    }
    // 変えた並びは、既定と違う（試験が何も確かめていないことにならない）
    assert_ne!(shape(&changed_dock()), shape(&default_dock()));
}

#[test]
fn headless_the_written_file_does_not_depend_on_the_drawn_sizes() {
    // 描いた矩形（まだ描いていない部品は無限大）は、書く値にしない: ウィンドウの大きさを変えても、同じ並びなら同じ中身
    let mut a = default_dock();
    let mut b = default_dock();
    for (_, node) in b.iter_all_nodes_mut() {
        match node {
            Node::Leaf(leaf) => {
                leaf.rect = Rect::from_min_size(pos2(3.0, 4.0), vec2(500.0, 600.0));
                leaf.viewport = leaf.rect;
                leaf.scroll = 12.0;
            }
            Node::Horizontal(s) | Node::Vertical(s) => {
                s.rect = Rect::from_min_size(pos2(1.0, 2.0), vec2(900.0, 700.0))
            }
            Node::Empty => {}
        }
    }
    assert_eq!(layout::render(&a, None), layout::render(&b, None));
    // 無限大の矩形があっても書ける（JSON に入らない値を書いて、次に読めなくならない）
    for (_, node) in a.iter_all_nodes_mut() {
        if let Node::Leaf(leaf) = node {
            leaf.rect = Rect::NOTHING;
        }
    }
    let loaded = layout::parse(&layout::render(&a, None));
    assert!(loaded.dock.is_some(), "{:?}", loaded.problems);
}

#[test]
fn headless_a_floating_window_survives_the_file_with_its_tabs_the_front_tab_and_its_place() {
    let mut dock = floating_dock();
    // 浮かせたウィンドウの中の組と前のタブ（履歴・チャンネルの 2 つ、前はチャンネル）
    let text = layout::render(&dock, None);
    let loaded = layout::parse(&text);
    assert_eq!(loaded.problems, Vec::<String>::new());
    let mut read = loaded.dock.expect("読めた");
    let floating = shape(&read)
        .into_iter()
        .find(|line| line.starts_with("1/"))
        .expect("浮かせたウィンドウの組");
    assert_eq!(floating, "1/0 leaf [\"history\", \"channels\"] active=1");
    assert_eq!(shape(&read), shape(&dock));
    assert_eq!(read.surfaces_count(), 2);
    // ウィンドウの位置と大きさ（最初に描くときの値）も戻る
    assert_eq!(window_state(&mut read, 1), window_state(&mut dock, 1));
    assert_eq!(
        window_state(&mut read, 1)["next_position"],
        serde_json::json!({"x": 300.0, "y": 200.0})
    );
    // 保存のとき、egui が覚えているウィンドウの今の位置と大きさを入れる（egui_dock はウィンドウの矩形を自分では更新しない）
    let moved = Rect::from_min_size(pos2(512.0, 384.0), vec2(300.0, 200.0));
    let text = layout::render_with(&dock, None, &[(SurfaceIndex(1), moved)]);
    let mut read = layout::parse(&text).dock.expect("読めた");
    assert_eq!(
        window_state(&mut read, 1)["next_position"],
        serde_json::json!({"x": 512.0, "y": 384.0})
    );
    assert_eq!(
        window_state(&mut read, 1)["next_size"],
        serde_json::json!({"x": 300.0, "y": 200.0})
    );
    assert_eq!(shape(&read), shape(&dock));
    // 値でない矩形・存在しない面は、入れずに黙って飛ばす
    let nonsense = [(SurfaceIndex(1), Rect::NOTHING), (SurfaceIndex(7), moved)];
    assert_eq!(
        layout::render_with(&dock, None, &nonsense),
        layout::render(&dock, None)
    );
}

/// egui_dock が実際に作る並び（分ける・浮かせる・戻す・組が空になって消える）は、どれも検査を通り、書いて読み戻して同じになる。
/// 検査が厳しすぎて、使える並びを捨てることが無いようにするための試験。
#[test]
fn headless_every_arrangement_egui_dock_makes_passes_the_check_and_comes_back_the_same() {
    let rect = |x: f32| Rect::from_min_size(pos2(x, 100.0), vec2(300.0, 220.0));
    let mut dock = default_dock();
    let check = |dock: &DockState<Tab>, what: &str| {
        assert_eq!(layout::validate(dock), Ok(()), "{what}");
        let loaded = layout::parse(&layout::render(dock, None));
        assert_eq!(loaded.problems, Vec::<String>::new(), "{what}");
        assert_eq!(shape(&loaded.dock.expect(what)), shape(dock), "{what}");
    };
    check(&dock, "既定");
    let history = dock.find_tab(&Tab::History).unwrap();
    let w1 = dock.detach_tab(history, rect(40.0));
    check(&dock, "履歴を浮かせた");
    let navigator = dock.find_tab(&Tab::Channels).unwrap();
    let w2 = dock.detach_tab(navigator, rect(400.0));
    check(&dock, "チャンネルも浮かせた");
    assert_ne!(w1, w2);
    // 2 つ目のウィンドウのタブを、1 つ目のウィンドウの組へ移す: 2 つ目のウィンドウは空になって消える
    let navigator = dock.find_tab(&Tab::Channels).unwrap();
    let target = dock.find_tab(&Tab::History).unwrap().node_path();
    dock.move_tab(navigator, TabDestination::Node(target, TabInsert::Append));
    check(
        &dock,
        "ウィンドウからウィンドウへ移して、空になったウィンドウが消えた",
    );
    // 浮かせたウィンドウの組を分ける
    let color = dock.find_tab(&Tab::Color).unwrap();
    let target = dock.find_tab(&Tab::History).unwrap().node_path();
    dock.move_tab(
        color,
        TabDestination::Node(target, TabInsert::Split(Split::Right)),
    );
    check(&dock, "浮かせたウィンドウの組を分けた");
    // メインの組を分けて、組を空にする（隣の組が上がる）
    let layers = dock.find_tab(&Tab::Layers).unwrap();
    let target = dock.find_tab(&Tab::Canvas).unwrap().node_path();
    dock.move_tab(
        layers,
        TabDestination::Node(target, TabInsert::Split(Split::Below)),
    );
    check(&dock, "メインの組を分けた");
    let properties = dock.find_tab(&Tab::Properties).unwrap();
    let target = dock.find_tab(&Tab::Canvas).unwrap().node_path();
    dock.move_tab(properties, TabDestination::Node(target, TabInsert::Append));
    check(&dock, "メインの組が空になって消えた");
    // 浮かせたウィンドウのタブを、メインの組へ戻す
    for tab in [Tab::History, Tab::Channels, Tab::Color] {
        let from = dock.find_tab(&tab).unwrap();
        let to = dock.find_tab(&Tab::Canvas).unwrap().node_path();
        dock.move_tab(from, TabDestination::Node(to, TabInsert::Append));
        check(&dock, &format!("{} をメインへ戻した", tab.key()));
    }
    // メインのタブを、キャンバスのほかは全部浮かせる（メインの木はほとんど空になる）
    for tab in Tab::ALL {
        // ポーズ・アクション・ナビゲーターは既定の並びに無い
        if tab == Tab::Canvas || tab == Tab::Pose || tab == Tab::Actions || tab == Tab::Navigator {
            continue;
        }
        let path = dock.find_tab(&tab).unwrap();
        if path.surface.is_main() && dock.main_surface().num_tabs() > 1 {
            dock.detach_tab(
                path,
                rect(60.0 + 20.0 * Tab::ALL.iter().position(|t| *t == tab).unwrap() as f32),
            );
            check(&dock, &format!("{} を浮かせた", tab.key()));
        }
    }
    assert!(
        dock.surfaces_count() > 3,
        "浮かせたウィンドウがいくつもある"
    );
    // キャンバスも浮かせて、メインを空にする
    let canvas = dock.find_tab(&Tab::Canvas).unwrap();
    dock.detach_tab(canvas, rect(200.0));
    assert!(
        dock.main_surface().is_empty() || dock.main_surface().num_tabs() == 0,
        "メインが空"
    );
    check(&dock, "メインが空（全部がウィンドウ）");
}

#[test]
fn headless_a_file_that_cannot_be_used_is_dropped_with_a_reason_for_the_log_only() {
    let good = layout::render(&changed_dock(), Some(&window(true)));
    let value: serde_json::Value = serde_json::from_str(&good).unwrap();
    let with = |edit: &dyn Fn(&mut serde_json::Value)| {
        let mut v = value.clone();
        edit(&mut v);
        serde_json::to_string(&v).unwrap()
    };
    let cases: Vec<(&str, String)> = vec![
        ("JSON でない", "not json at all {".into()),
        ("空", String::new()),
        (
            "版が無い",
            with(&|v| {
                v.as_object_mut().unwrap().remove("format");
            }),
        ),
        ("古い版", with(&|v| v["format"] = 0.into())),
        ("新しい版", with(&|v| v["format"] = 99.into())),
        (
            "ドックが無い",
            with(&|v| {
                v.as_object_mut().unwrap().remove("dock");
            }),
        ),
        (
            "ドックの形が違う",
            with(&|v| v["dock"] = serde_json::json!({"surfaces": 3})),
        ),
        (
            "知らないタブ",
            good.replace("\"layers\"", "\"layers_of_the_future\""),
        ),
        (
            "タブが足りない",
            good.replacen("\"history\"", "\"navigator\"", 1),
        ),
        (
            "タブが重なる",
            good.replacen("\"channels\"", "\"canvas\"", 1),
        ),
    ];
    for (what, text) in cases {
        let loaded = layout::parse(&text);
        assert!(loaded.dock.is_none(), "{what}: ドックを捨てる");
        assert!(!loaded.problems.is_empty(), "{what}: 理由がある");
        for reason in &loaded.problems {
            assert!(
                !reason.is_empty() && reason.chars().count() < 200,
                "{what}: {reason}"
            );
        }
    }
    // ポーズは、無くてよい（モデルを読むと足される）
    let mut no_pose = default_dock();
    assert!(layout::validate(&no_pose).is_ok());
    no_pose.push_to_first_leaf(Tab::Pose);
    assert!(layout::validate(&no_pose).is_ok());
    no_pose.push_to_first_leaf(Tab::Pose);
    assert!(layout::validate(&no_pose).is_err(), "ポーズが 2 つ");
}

/// egui_dock は、読んだ値をそのまま添字で引く（前のタブ・木の子・ウィンドウの組）ので、外れた値を渡すと起動のたびに落ちる。
/// 描く前の検査が、どれも理由つきで捨てることを確かめる（既定の並びで始め、ファイルは次の保存で置き換わる）。
#[test]
fn headless_a_dock_whose_values_egui_dock_would_index_blindly_is_dropped_with_a_reason() {
    use serde_json::json;
    let good = layout::render(&floating_dock(), None);
    let value: serde_json::Value = serde_json::from_str(&good).unwrap();
    let with = |edit: &dyn Fn(&mut serde_json::Value)| {
        let mut v = value.clone();
        edit(&mut v["dock"]["surfaces"]);
        serde_json::to_string(&v).unwrap()
    };
    // メインの木の、最初の組・最初の分け方の添字（既定の並びの形）
    let first = |surfaces: &serde_json::Value, kind: &str| {
        surfaces[0]["Main"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .position(|n| n.get(kind).is_some())
            .unwrap_or_else(|| panic!("{kind} が無い"))
    };
    let cases: Vec<(&str, String)> = vec![
        (
            "浮かせたウィンドウの前のタブが範囲の外",
            with(&|s| s[1]["Window"][0]["nodes"][0]["Leaf"]["active"] = 2.into()),
        ),
        (
            "メインの前のタブが範囲の外",
            with(&|s| {
                let i = first(s, "Leaf");
                s[0]["Main"]["nodes"][i]["Leaf"]["active"] = 99.into();
            }),
        ),
        (
            "組が空",
            with(&|s| {
                let i = first(s, "Leaf");
                s[0]["Main"]["nodes"][i]["Leaf"]["tabs"] = json!([]);
            }),
        ),
        (
            "浮かせたウィンドウの組が空",
            with(&|s| s[1]["Window"][0]["nodes"][0]["Leaf"]["tabs"] = json!([])),
        ),
        (
            "分け方が負",
            with(&|s| {
                let i = first(s, "Horizontal");
                s[0]["Main"]["nodes"][i]["Horizontal"]["fraction"] = (-0.5).into();
            }),
        ),
        (
            "分け方が 0",
            with(&|s| {
                let i = first(s, "Horizontal");
                s[0]["Main"]["nodes"][i]["Horizontal"]["fraction"] = 0.0.into();
            }),
        ),
        (
            "分け方が 1 以上",
            with(&|s| {
                let i = first(s, "Vertical");
                s[0]["Main"]["nodes"][i]["Vertical"]["fraction"] = 1.5.into();
            }),
        ),
        (
            "分け方が数でない",
            with(&|s| {
                let i = first(s, "Vertical");
                s[0]["Main"]["nodes"][i]["Vertical"]["fraction"] = json!(null);
            }),
        ),
        (
            "分けた所の子が木の外",
            with(&|s| {
                s[0]["Main"]["nodes"] = json!([{"Horizontal": {"fraction": 0.5}}]);
            }),
        ),
        (
            "浮かせたウィンドウの組が 1 つも無い",
            with(&|s| s[1]["Window"][0]["nodes"] = json!([])),
        ),
        (
            "分けた所の根が空でつながらない節がある",
            with(&|s| {
                s[0]["Main"]["nodes"][0] = json!("Empty");
            }),
        ),
        (
            "先頭の面が主の面でない",
            with(&|s| {
                s.as_array_mut().unwrap().swap(0, 1);
            }),
        ),
        (
            "主の面が 2 つ",
            with(&|s| {
                let main = s[0].clone();
                s.as_array_mut().unwrap().push(main);
            }),
        ),
        (
            "ウィンドウの位置が範囲の外",
            with(&|s| s[1]["Window"][1]["next_position"] = json!({"x": 1e9, "y": 0.0})),
        ),
        (
            "ウィンドウの位置が数でない",
            with(&|s| s[1]["Window"][1]["next_position"] = json!({"x": null, "y": 0.0})),
        ),
        (
            "ウィンドウの大きさが負",
            with(&|s| s[1]["Window"][1]["next_size"] = json!({"x": -10.0, "y": 100.0})),
        ),
        (
            "ウィンドウの大きさが大きすぎる",
            with(&|s| s[1]["Window"][1]["next_size"] = json!({"x": 1e9, "y": 100.0})),
        ),
    ];
    for (what, text) in cases {
        let loaded = layout::parse(&text);
        assert!(loaded.dock.is_none(), "{what}: ドックを捨てる");
        assert_eq!(
            loaded.problems.len(),
            1,
            "{what}: 理由が 1 つ: {:?}",
            loaded.problems
        );
        // 数でない値は、検査の前に読めずに捨てる。それ以外は、読めたうえで検査が断る
        let expected = if what.contains("数でない") {
            "ドックの並びを読めません"
        } else {
            "ドックの並びを使えません"
        };
        assert!(
            loaded.problems[0].contains(expected),
            "{what}: {}",
            loaded.problems[0]
        );
    }
    // 前の形（検査を通る値）は、そのまま使える
    let loaded = layout::parse(&good);
    assert_eq!(loaded.problems, Vec::<String>::new());
    assert!(loaded.dock.is_some());
    // フォーカスが無い組を指していても、読むときに外すので使える。浮かせたウィンドウの木の中の、組でない所を指していれば（外し忘れ）検査が断る
    let bad_focus = with(&|s| s[1]["Window"][0]["focused_node"] = 5.into());
    let loaded = layout::parse(&bad_focus);
    assert!(
        loaded.dock.is_some(),
        "フォーカスは外して使う: {:?}",
        loaded.problems
    );
    let mut raw = serde_json::from_str::<serde_json::Value>(&bad_focus).unwrap();
    let dock: DockState<Tab> = serde_json::from_value(raw["dock"].take()).unwrap();
    assert!(
        layout::validate(&dock).is_err(),
        "外していない値は、浮かせたウィンドウのフォーカスが組を指していないと断る"
    );
}

#[test]
fn headless_the_window_is_checked_separately_and_nonsense_values_are_dropped() {
    let text = |window: serde_json::Value| {
        let mut v: serde_json::Value =
            serde_json::from_str(&layout::render(&default_dock(), None)).unwrap();
        v["window"] = window;
        serde_json::to_string(&v).unwrap()
    };
    let good = serde_json::json!({"x": 10.0, "y": 20.0, "width": 1500.0, "height": 900.0, "pixels_per_point": 1.5, "maximized": true});
    let loaded = layout::parse(&text(good));
    assert!(loaded.problems.is_empty());
    assert_eq!(
        loaded.window,
        Some(WindowRecord {
            position: [10.0, 20.0],
            size: [1500.0, 900.0],
            pixels_per_point: 1.5,
            maximized: true
        })
    );
    // 最小の大きさより小さければ引き上げる
    let small = serde_json::json!({"x": 0.0, "y": 0.0, "width": 300.0, "height": 200.0, "pixels_per_point": 1.0});
    assert_eq!(
        layout::parse(&text(small)).window.unwrap().size,
        layout::MIN_SIZE
    );
    // 壊れた値のウィンドウは捨てるが、ドックは生かす
    for bad in [
        serde_json::json!({"x": 1.0}),
        serde_json::json!({"x": 1e9, "y": 0.0, "width": 1000.0, "height": 800.0, "pixels_per_point": 1.0}),
        serde_json::json!({"x": 0.0, "y": 0.0, "width": 1e9, "height": 800.0, "pixels_per_point": 1.0}),
        serde_json::json!({"x": 0.0, "y": 0.0, "width": 1000.0, "height": 800.0, "pixels_per_point": 0.0}),
        serde_json::json!({"x": "left", "y": 0.0, "width": 1000.0, "height": 800.0, "pixels_per_point": 1.0}),
        serde_json::json!(7),
    ] {
        let loaded = layout::parse(&text(bad.clone()));
        assert!(loaded.window.is_none(), "{bad}");
        assert!(
            loaded.dock.is_some(),
            "ウィンドウが壊れてもドックは生かす: {bad}"
        );
        assert_eq!(loaded.problems.len(), 1, "{bad}");
    }
    // ドックが壊れても、ウィンドウは戻す
    let mut v: serde_json::Value =
        serde_json::from_str(&layout::render(&default_dock(), Some(&window(false)))).unwrap();
    v["dock"] = serde_json::json!("junk");
    let loaded = layout::parse(&serde_json::to_string(&v).unwrap());
    assert!(loaded.dock.is_none());
    assert_eq!(loaded.window, Some(window(false)));
}

#[test]
fn headless_saving_replaces_the_file_in_one_step_and_leaves_no_pending_file() {
    let dir = settings_dir("replace");
    let path = layout_file(&dir.join("deep").join("er"));
    layout::save(&path, "first").unwrap();
    layout::save(&path, "second").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
    let names: Vec<String> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        vec![layout::FILE_NAME.to_owned()],
        "一時ファイルが残らない"
    );
}

#[test]
fn headless_the_tab_names_are_stable_and_every_tab_has_one() {
    let keys: Vec<&str> = Tab::ALL.iter().map(|t| t.key()).collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), Tab::ALL.len(), "名前が重ならない: {keys:?}");
    for tab in Tab::ALL {
        assert_eq!(Tab::from_key(tab.key()), Some(tab));
    }
    assert_eq!(Tab::from_key("nope"), None);
    // 保存済みのファイルの名前を変えない（変えると、保存した並びが「知らないタブ」で捨てられる）
    assert_eq!(
        keys,
        [
            "subtools",
            "tool_properties",
            "brush_size",
            "assets",
            "color",
            "pose",
            "canvas",
            "view3d",
            "texture_sets",
            "layers",
            "properties",
            "material",
            "channels",
            "history",
            "color_sets",
            "navigator",
            "log",
            "actions",
        ]
    );
    // ドックに出るタブは、全部 ALL にある
    for (_, tab) in default_dock().iter_all_tabs() {
        assert!(Tab::ALL.contains(tab));
    }
}

/// 前の版が保存した並び（ログのタブが無い）を読んでも、並び全部は捨てない: ログのタブだけを、既定の並びと同じくレイヤーと同じ組の後ろへ
/// 足す（前へは出さない）。レイヤーを浮かせた並びなら、そのウィンドウの組へ。
#[test]
fn headless_a_saved_layout_without_the_log_tab_keeps_its_arrangement_and_gains_the_log_tab() {
    let without_log = |mut dock: DockState<Tab>| {
        let log = dock.find_tab(&Tab::Log).expect("既定の並びにはある");
        dock.remove_tab(log);
        assert!(dock.find_tab(&Tab::Log).is_none());
        dock
    };
    // レイヤーを浮かせた、前の版の並び
    let mut old = without_log(default_dock());
    let layers = old.find_tab(&Tab::Layers).unwrap();
    old.detach_tab(
        layers,
        Rect::from_min_size(pos2(40.0, 100.0), vec2(300.0, 220.0)),
    );
    assert_eq!(layout::validate(&old), Ok(()), "ログが無くても使える並び");
    let loaded = layout::parse(&layout::render(&old, None));
    assert_eq!(loaded.problems, Vec::<String>::new(), "並びを捨てない");
    let dock = loaded.dock.expect("読めた");
    let layers = dock.find_tab(&Tab::Layers).unwrap();
    let log = dock.find_tab(&Tab::Log).expect("ログのタブが足された");
    assert_eq!(
        (layers.surface, layers.node_path()),
        (log.surface, log.node_path()),
        "レイヤーと同じ組"
    );
    assert_eq!(log.tab.0, layers.tab.0 + 1, "レイヤーの後ろ");
    // ほかは前の並びのまま。レイヤーの組は後ろにログが付くだけで、前のタブ（active）は変わらない
    let actual: Vec<String> = shape(&dock)
        .into_iter()
        .map(|line| line.replace(", \"log\"", ""))
        .collect();
    assert_eq!(actual, shape(&old));
}

/// アクションのパネルは既定の並びに無く、開くと（「ウィンドウ」のメニューの `DockOp::Show`）レイヤーと同じ組の後ろへ入って前に出る。開いた並びはファイルで往復し、
/// 前の版の並び（アクションが無い）を読んでも足さない。
#[test]
fn headless_the_actions_tab_is_not_in_the_default_dock_and_opens_beside_the_layers() {
    let mut dock = default_dock();
    assert!(dock.find_tab(&Tab::Actions).is_none(), "既定の並びに無い");
    let loaded = layout::parse(&layout::render(&dock, None));
    assert_eq!(loaded.problems, Vec::<String>::new());
    assert!(
        loaded.dock.unwrap().find_tab(&Tab::Actions).is_none(),
        "読んでも足さない"
    );
    yolu_app::detach::Detached::default().show(&mut dock, Tab::Actions);
    let layers = dock.find_tab(&Tab::Layers).unwrap();
    let actions = dock.find_tab(&Tab::Actions).expect("開いた");
    assert_eq!(
        (actions.surface, actions.node_path()),
        (layers.surface, layers.node_path()),
        "レイヤーと同じ組"
    );
    let leaf = dock.leaf(actions.node_path()).unwrap();
    assert_eq!(leaf.tabs.last(), Some(&Tab::Actions), "組の後ろ");
    assert_eq!(leaf.tabs[leaf.active.0], Tab::Actions, "前に出る");
    // もう一度開いても 1 つのまま。後ろにあれば前に出すだけ
    let layers = dock.find_tab(&Tab::Layers).unwrap();
    dock.set_active_tab(layers).unwrap();
    yolu_app::detach::Detached::default().show(&mut dock, Tab::Actions);
    assert_eq!(layout::validate(&dock), Ok(()));
    let leaf = dock.leaf(actions.node_path()).unwrap();
    assert_eq!(leaf.tabs.iter().filter(|t| **t == Tab::Actions).count(), 1);
    assert_eq!(leaf.tabs[leaf.active.0], Tab::Actions);
    let loaded = layout::parse(&layout::render(&dock, None));
    assert_eq!(loaded.problems, Vec::<String>::new());
    assert_eq!(
        shape(&loaded.dock.unwrap()),
        shape(&dock),
        "開いた並びが往復する"
    );
    // レイヤーのタブが無い並びなら、最初の組へ
    let mut bare = DockState::new(vec![Tab::Canvas]);
    yolu_app::detach::Detached::default().show(&mut bare, Tab::Actions);
    assert!(bare.find_tab(&Tab::Actions).is_some());
}

// ───────── アプリ ─────────

#[test]
fn the_dock_is_written_when_the_app_ends_and_comes_back_at_the_next_start() {
    use eframe::App;
    let dir = settings_dir("exit");
    let mut h = app_in(&dir);
    assert_eq!(
        shape(&h.state().dock),
        shape(&default_dock_for(1280.0)),
        "ファイルが無ければ既定の並び"
    );
    h.state_mut().dock = changed_dock();
    h.run();
    // 終わるとき、並びが変わっていれば書く（1 秒おきの書き込みを待たない）
    h.state_mut().on_exit();
    assert!(layout_file(&dir).exists(), "終わるときに書く");
    drop(h);
    let h = app_in(&dir);
    assert_eq!(
        shape(&h.state().dock),
        shape(&changed_dock()),
        "次の起動で戻る"
    );
    // 戻した並びでも、どのタブも 1 つずつ・前のタブが描かれる
    assert!(
        h.state().tab_rects.contains_key(&Tab::Color),
        "前にしたタブが描かれている"
    );
    assert!(h.state().dock.find_tab(&Tab::Canvas).is_some());
}

#[test]
fn a_change_is_written_about_a_second_later_without_waiting_for_the_end() {
    let dir = settings_dir("periodic");
    let mut h = app_in(&dir);
    let file = layout_file(&dir);
    for _ in 0..90 {
        h.step();
    }
    let first = std::fs::read_to_string(&file).expect("最初の 1 秒のあとに、今の並びを書く");
    assert!(layout::parse(&first).dock.is_some());
    // 変わっていなければ書き直さない（同じ中身を毎秒書かない）
    // 更新時刻を前にしておく（書き直されたら今の時刻になって食い違う。時刻の粒度に頼って待たない）
    let modified = common::tmp::backdate(&file);
    for _ in 0..180 {
        h.step();
    }
    assert_eq!(
        std::fs::metadata(&file).unwrap().modified().unwrap(),
        modified,
        "変わらないうちは書き直さない"
    );
    // タブを動かすと、次の 1 秒の見回りで書く
    h.state_mut().dock = changed_dock();
    for _ in 0..90 {
        h.step();
    }
    let second = layout::parse(&std::fs::read_to_string(&file).unwrap());
    assert_eq!(shape(&second.dock.expect("読める")), shape(&changed_dock()));
    // 区切りを動かすと、動かしている間も 1 秒おきに 1 回までで、離したあとの並びが書かれる
    h.state_mut().dock = default_dock();
    for _ in 0..90 {
        h.step();
    }
    assert_eq!(
        shape(
            &layout::parse(&std::fs::read_to_string(&file).unwrap())
                .dock
                .unwrap()
        ),
        shape(&default_dock())
    );
    let (left, center) = {
        let dock = &h.state().dock;
        let rect = |tab: Tab| match &dock[dock.find_tab(&tab).unwrap().node_path().surface]
            [dock.find_tab(&tab).unwrap().node]
        {
            Node::Leaf(leaf) => leaf.rect,
            _ => panic!("組でない"),
        };
        // 中央のいちばん左の組は 3D ビュー（左の列との区切りを動かす）
        (rect(Tab::SubTools), rect(Tab::View3d))
    };
    let at = pos2((left.right() + center.left()) / 2.0, 400.0);
    let before = h
        .state()
        .dock
        .main_surface()
        .iter()
        .find_map(|n| matches!(n, Node::Horizontal(_)).then(|| shape(&h.state().dock)));
    press(&h, at, egui::PointerButton::Primary);
    h.step();
    for dx in [10.0, 20.0, 40.0, 60.0] {
        h.event(egui::Event::PointerMoved(at + vec2(dx, 0.0)));
        for _ in 0..30 {
            h.step();
        }
    }
    release(&h, at + vec2(60.0, 0.0), egui::PointerButton::Primary);
    h.step();
    for _ in 0..90 {
        h.step();
    }
    let moved = shape(&h.state().dock);
    assert_ne!(Some(moved.clone()), before, "区切りが動いた");
    assert_eq!(
        shape(
            &layout::parse(&std::fs::read_to_string(&file).unwrap())
                .dock
                .unwrap()
        ),
        moved,
        "離したあとの並びが書かれている"
    );
}

#[test]
fn an_unreadable_file_starts_with_the_default_and_says_nothing_on_screen() {
    for (tag, content) in [
        ("junk", "this is not json".to_owned()),
        (
            "old",
            layout::render(&changed_dock(), None).replacen("\"format\": 1", "\"format\": 0", 1),
        ),
        (
            "future-tab",
            layout::render(&changed_dock(), None).replace("\"layers\"", "\"layers_of_the_future\""),
        ),
        (
            "missing-tab",
            layout::render(&changed_dock(), None).replacen("\"history\"", "\"navigator\"", 1),
        ),
        // 浮かせたウィンドウの前のタブが範囲の外（egui_dock が添字で引いて、起動のたびに落ちる値）
        ("window-active-out-of-range", {
            let mut v: serde_json::Value =
                serde_json::from_str(&layout::render(&floating_dock(), None)).unwrap();
            v["dock"]["surfaces"][1]["Window"][0]["nodes"][0]["Leaf"]["active"] = 9.into();
            serde_json::to_string(&v).unwrap()
        }),
        // 読む大きさの上限（1 MiB）を超える（中身は使える並びでも読まない）
        (
            "oversized",
            padded(&layout::render(&changed_dock(), None), MAX_FILE_BYTES + 1),
        ),
    ] {
        let dir = settings_dir(tag);
        std::fs::write(layout_file(&dir), &content).unwrap();
        let h = app_in(&dir);
        assert_eq!(
            shape(&h.state().dock),
            shape(&default_dock_for(1280.0)),
            "{tag}: 既定の並び"
        );
        assert_eq!(h.state().state.message, "", "{tag}: 理由は画面に出さない");
        // 使えなかったファイルは、今の（既定の）並びで置き換わる（次の起動で、また同じ理由で捨てることにならない）
        let after = layout::parse(&std::fs::read_to_string(layout_file(&dir)).unwrap());
        assert!(after.problems.is_empty(), "{tag}: {:?}", after.problems);
        assert_eq!(
            shape(&after.dock.expect("読める")),
            shape(&default_dock_for(1280.0)),
            "{tag}"
        );
    }
    // 使える並びのファイルは、読んだだけでは書き直さない
    let dir = settings_dir("valid-untouched");
    let text = layout::render(&changed_dock(), Some(&window(false)));
    std::fs::write(layout_file(&dir), &text).unwrap();
    let h = app_in(&dir);
    assert_eq!(shape(&h.state().dock), shape(&changed_dock()));
    assert_eq!(std::fs::read_to_string(layout_file(&dir)).unwrap(), text);
}

#[test]
fn a_file_with_a_good_dock_but_a_broken_window_keeps_the_dock() {
    let dir = settings_dir("window-broken");
    let mut v: serde_json::Value =
        serde_json::from_str(&layout::render(&changed_dock(), None)).unwrap();
    v["window"] = serde_json::json!({"x": "oops"});
    std::fs::write(layout_file(&dir), serde_json::to_string(&v).unwrap()).unwrap();
    let h = app_in(&dir);
    assert_eq!(shape(&h.state().dock), shape(&changed_dock()));
}

#[test]
fn reset_panel_layout_goes_back_to_the_default_and_the_default_is_what_gets_written() {
    use eframe::App;
    let dir = settings_dir("reset");
    let mut h = app_in(&dir);
    h.state_mut().dock = changed_dock();
    h.run();
    h.state_mut().on_exit();
    assert_eq!(
        shape(
            &layout::parse(&std::fs::read_to_string(layout_file(&dir)).unwrap())
                .dock
                .unwrap()
        ),
        shape(&changed_dock())
    );
    // ウィンドウ → パネルの並びを戻す
    h.state_mut().state.apply(Action::ResetLayout);
    h.run();
    assert_eq!(
        shape(&h.state().dock),
        shape(&default_dock_for(1280.0)),
        "今まで通り既定へ"
    );
    h.state_mut().on_exit();
    let written = layout::parse(&std::fs::read_to_string(layout_file(&dir)).unwrap());
    assert_eq!(
        shape(&written.dock.unwrap()),
        shape(&default_dock_for(1280.0)),
        "戻した並びが書かれる"
    );
}

#[test]
fn opening_the_actions_panel_puts_it_in_the_dock_and_it_is_written_and_reset_away() {
    use eframe::App;
    let dir = settings_dir("show-panel");
    let mut h = app_in(&dir);
    h.state_mut()
        .state
        .apply(Action::Dock(yolu_app::detach::DockOp::Show(Tab::Actions)));
    h.run();
    let dock = &h.state().dock;
    let actions = dock.find_tab(&Tab::Actions).expect("開いた");
    let leaf = dock.leaf(actions.node_path()).unwrap();
    assert!(leaf.tabs.contains(&Tab::Layers));
    assert_eq!(leaf.tabs[leaf.active.0], Tab::Actions, "前に出る");
    assert!(
        h.state().state.ui.dock_ops.is_empty(),
        "頼みは 1 回で使い切る"
    );
    h.state_mut().on_exit();
    drop(h);
    let mut h = app_in(&dir);
    assert!(
        h.state().dock.find_tab(&Tab::Actions).is_some(),
        "次の起動でも開いている"
    );
    // パネルの並びを戻すと、既定の並び（アクションは無い）
    h.state_mut().state.apply(Action::ResetLayout);
    h.run();
    assert!(h.state().dock.find_tab(&Tab::Actions).is_none());
}

fn set_viewport(
    h: &mut Harness<'static, YoluApp>,
    outer: Rect,
    inner: Rect,
    maximized: bool,
    monitor: egui::Vec2,
) {
    let info = h
        .input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default();
    info.outer_rect = Some(outer);
    info.inner_rect = Some(inner);
    info.maximized = Some(maximized);
    info.native_pixels_per_point = Some(1.5);
    info.monitor_size = Some(monitor);
}

#[test]
fn the_window_size_and_position_are_remembered_and_a_maximized_window_keeps_its_restored_size() {
    use eframe::App;
    let dir = settings_dir("window");
    let mut h = app_in(&dir);
    let outer = Rect::from_min_size(pos2(120.0, 80.0), vec2(1412.0, 940.0));
    let inner = Rect::from_min_size(pos2(126.0, 118.0), vec2(1400.0, 900.0));
    set_viewport(&mut h, outer, inner, false, vec2(3840.0, 2160.0));
    h.step();
    // 最大化したら、最大化した大きさは覚えず、戻したときの大きさと位置を覚える（終わるときは最大化の印）
    let full = Rect::from_min_size(pos2(0.0, 0.0), vec2(3840.0, 2100.0));
    set_viewport(&mut h, full, full, true, vec2(3840.0, 2160.0));
    h.step();
    h.state_mut().on_exit();
    let loaded = layout::load(&layout_file(&dir));
    assert!(loaded.problems.is_empty(), "{:?}", loaded.problems);
    assert_eq!(
        loaded.window,
        Some(WindowRecord {
            position: [120.0, 80.0],
            size: [1400.0, 900.0],
            pixels_per_point: 1.5,
            maximized: true
        })
    );
    // 次の起動で読める（main がこの値でウィンドウを作る）
    assert_eq!(layout::saved_window_at(&layout_file(&dir)), loaded.window);
    // 戻したら、最大化の印は外れ、新しい大きさと位置を書く
    let outer = Rect::from_min_size(pos2(200.0, 100.0), vec2(1112.0, 740.0));
    let inner = Rect::from_min_size(pos2(206.0, 138.0), vec2(1100.0, 700.0));
    set_viewport(&mut h, outer, inner, false, vec2(3840.0, 2160.0));
    h.step();
    h.state_mut().on_exit();
    assert_eq!(
        layout::load(&layout_file(&dir)).window,
        Some(WindowRecord {
            position: [200.0, 100.0],
            size: [1100.0, 700.0],
            pixels_per_point: 1.5,
            maximized: false
        })
    );
    // 最小化している間の値（位置がありえない値）は覚えない
    let hidden = Rect::from_min_size(pos2(-32000.0, -32000.0), vec2(160.0, 28.0));
    h.input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .minimized = Some(true);
    set_viewport(&mut h, hidden, hidden, false, vec2(3840.0, 2160.0));
    h.input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .minimized = Some(true);
    h.step();
    h.state_mut().on_exit();
    assert_eq!(
        layout::load(&layout_file(&dir)).window.unwrap().position,
        [200.0, 100.0],
        "最小化の値は書かない"
    );
}

/// 試験のウィンドウ（実際のウィンドウではない）は、ウィンドウの大きさを命じない。
#[test]
fn a_test_window_never_commands_the_window_size() {
    let dir = settings_dir("fit-test");
    let mut h = app_in(&dir);
    let big = Rect::from_min_size(pos2(0.0, 0.0), vec2(5000.0, 3000.0));
    set_viewport(&mut h, big, big, false, vec2(1920.0, 1080.0));
    h.step();
    h.step();
    let commands: Vec<_> = h
        .output()
        .viewport_output
        .values()
        .flat_map(|v| v.commands.iter())
        .collect();
    assert!(
        !commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::InnerSize(_))),
        "実際のウィンドウだけが収める: {commands:?}"
    );
}

/// 読む大きさの上限: ちょうど上限までは読み、1 バイトでも超えれば（中身が使える並びでも）理由つきで捨てる。
#[test]
fn headless_a_file_over_the_size_budget_is_refused_and_one_at_the_budget_is_read() {
    let good = layout::render(&changed_dock(), Some(&window(false)));
    let dir = settings_dir("budget");
    let file = layout_file(&dir);
    std::fs::write(&file, padded(&good, MAX_FILE_BYTES)).unwrap();
    let loaded = layout::load(&file);
    assert_eq!(loaded.problems, Vec::<String>::new(), "上限ちょうどは読む");
    assert_eq!(shape(&loaded.dock.expect("読めた")), shape(&changed_dock()));
    assert_eq!(loaded.window, Some(window(false)));
    std::fs::write(&file, padded(&good, MAX_FILE_BYTES + 1)).unwrap();
    let loaded = layout::load(&file);
    assert!(
        loaded.dock.is_none() && loaded.window.is_none(),
        "上限を 1 バイト超えたら読まない"
    );
    assert_eq!(loaded.problems.len(), 1);
    assert!(
        loaded.problems[0].contains("大きすぎ") && loaded.problems[0].contains("既定の並び"),
        "{:?}",
        loaded.problems
    );
    // ウィンドウの大きさだけを取る口も、同じ上限で読まない
    assert_eq!(layout::saved_window_at(&file), None);
}

/// 前の版の、アプリの中の浮いたウィンドウ（egui_dock のウィンドウの面）は、読んだあと別ウィンドウになり、組・前のタブ・位置・大きさを保つ。次に書くときは
/// 別ウィンドウ（`detached`）として書き、その次の起動でも同じに戻る。
#[test]
fn a_floating_window_of_an_earlier_version_opens_as_an_outside_window_and_comes_back_the_same() {
    use eframe::App;
    let dir = settings_dir("floating");
    let mut dock = floating_dock();
    dock.get_window_state_mut(SurfaceIndex(1))
        .unwrap()
        .set_position(pos2(520.0, 330.0))
        .set_size(vec2(300.0, 210.0));
    layout::save(&layout_file(&dir), &layout::render(&dock, None)).unwrap();
    let mut h = app_in(&dir);
    let windows = &h.state().detached.windows;
    assert_eq!(
        h.state().dock.surfaces_count(),
        1,
        "主のドックに浮いたウィンドウの面が残らない"
    );
    assert_eq!(windows.len(), 1);
    assert_eq!(
        shape(&windows[0].dock),
        vec!["0/0 leaf [\"history\", \"channels\"] active=1"]
    );
    // メインウィンドウの内側の左上からの位置のまま（試験のウィンドウは内側の左上が原点）
    let record = windows[0].record.expect("置き場所");
    assert_eq!(record.position, [520.0, 330.0]);
    assert_eq!(record.size, [300.0, 210.0]);
    layout::validate_all(&h.state().dock, &[&windows[0].dock]).expect("どのタブも 1 つずつ");
    h.state_mut().on_exit();
    let saved = layout::parse(&std::fs::read_to_string(layout_file(&dir)).unwrap());
    assert_eq!(saved.problems, Vec::<String>::new());
    assert_eq!(saved.detached.len(), 1);
    assert_eq!(saved.detached[0].window, Some(record));
    drop(h);
    // 次の起動で、組・前のタブ・位置・大きさが戻る（何度繰り返しても変わらない）
    for _ in 0..2 {
        let mut h = app_in(&dir);
        let windows = &h.state().detached.windows;
        assert_eq!(windows.len(), 1);
        assert_eq!(
            shape(&windows[0].dock),
            vec!["0/0 leaf [\"history\", \"channels\"] active=1"]
        );
        assert_eq!(windows[0].record, Some(record));
        h.state_mut().on_exit();
    }
}

/// ウィンドウが画面より大きければ、起動のあと 1 度だけ収める（実際のウィンドウの動き。保存したあとで画面が小さくなったとき）。
#[test]
fn a_window_larger_than_the_screen_is_fitted_once_after_start_in_the_real_window() {
    let inner_size_commands = |h: &Harness<'static, YoluApp>| -> Vec<egui::Vec2> {
        h.output()
            .viewport_output
            .values()
            .flat_map(|v| v.commands.iter())
            .filter_map(|c| match c {
                egui::ViewportCommand::InnerSize(size) => Some(*size),
                _ => None,
            })
            .collect()
    };
    let monitor = vec2(1920.0, 1080.0);
    // 画面より大きいウィンドウ: 画面の大きさへ 1 度だけ収める
    let mut h = app_in_with(&settings_dir("fit-real"), true);
    let big = Rect::from_min_size(pos2(0.0, 0.0), vec2(5000.0, 1000.0));
    set_viewport(&mut h, big, big, false, monitor);
    h.step();
    assert_eq!(
        inner_size_commands(&h),
        vec![vec2(1920.0, 1000.0)],
        "大きすぎる方だけ画面の大きさへ"
    );
    h.step();
    h.step();
    assert!(inner_size_commands(&h).is_empty(), "2 度目は命じない");
    // 画面に収まっているウィンドウ・最大化しているウィンドウには命じない
    for (what, rect, maximized) in [
        (
            "収まっている",
            Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 900.0)),
            false,
        ),
        (
            "最大化",
            Rect::from_min_size(pos2(0.0, 0.0), vec2(5000.0, 3000.0)),
            true,
        ),
    ] {
        let mut h = app_in_with(&settings_dir("fit-none"), true);
        set_viewport(&mut h, rect, rect, maximized, monitor);
        h.step();
        h.step();
        assert!(inner_size_commands(&h).is_empty(), "{what}: 命じない");
    }
}
