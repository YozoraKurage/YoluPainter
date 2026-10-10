//! 左のドックの「サブツール」（今のツールのサブツールの一覧）と、その下に縦に並ぶ「ツールプロパティ」「ブラシサイズ」（どれも別のパネル）、それに合わせた
//! オプションバー・右のプロパティ（egui_kittest）。サブツールのプリセットの状態の操作（画面を描かない）は `src/subtool` の単体試験、ブラシの一覧の操作は
//! `brush_list.rs`・`brushes.rs`、新しいパネルの並びと「塗るチャンネル」は `tool_panels.rs`。見た目の試験は、パネルの中だけを撮る
//! （ほかのパネルの変更で壊れない）。
use crate::common;

use common::*;
use egui::epaint::Shape;
use egui::{pos2, Event, Key, Modifiers, Rect};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::engine::LayerKind;
use yolu_app::lang::Lang;
use yolu_app::state::{Action, AppState, Tool};
use yolu_app::subtool::{fields, Key as PresetKey};
use yolu_app::tools::{SubTools, SELECTION_TOOLS};
use yolu_app::{Tab, YoluApp};

type H = Harness<'static, YoluApp>;

fn st(h: &H) -> &AppState {
    &h.state().state
}

fn pick(h: &mut H, tool: Tool) {
    h.state_mut().state.apply(Action::SelectTool(tool));
    h.run();
}

fn language(h: &mut H, lang: Lang) {
    h.state_mut()
        .state
        .apply(Action::M2Ui(yolu_app::m2::UiOp::Language(lang)));
    h.run();
}

/// 左のドックのパネルの矩形（タブの帯から、下のカラーのパネルの上まで）。
fn panel(h: &H) -> Rect {
    let tab = h.state().tab_rects[&Tab::SubTools];
    let color = h.state().tab_rects[&Tab::Color];
    Rect::from_min_max(
        pos2(tab.left() - 2.0, tab.bottom()),
        pos2(tab.left() + 340.0, color.top()),
    )
}

/// ウィンドウのどこかに出ている、この名前の部品の数。
fn count(h: &H, label: &str) -> usize {
    h.query_all_by_label(label).count()
}

/// パネルの中の、この名前の部品（上から順）。
fn in_panel(h: &H, label: &str) -> Vec<Rect> {
    let panel = panel(h);
    let mut found: Vec<Rect> = h
        .query_all_by_label(label)
        .map(|n| n.rect())
        .filter(|r| panel.contains(r.center()))
        .collect();
    found.sort_by(|a, b| a.top().total_cmp(&b.top()));
    found
}

/// 一覧の行（パネルの中の、いちばん上のその名前）。
fn row(h: &H, label: &str) -> Rect {
    *in_panel(h, label)
        .first()
        .unwrap_or_else(|| panic!("{label} の行が無い"))
}

fn click_row(h: &mut H, label: &str) {
    let at = row(h, label).center();
    click(h, at);
}

/// 一覧の行をダブルクリックする。
fn double_click(h: &mut H, label: &str) {
    let at = pos2(row(h, label).left() + 30.0, row(h, label).center().y);
    for _ in 0..2 {
        press(h, at, egui::PointerButton::Primary);
        release(h, at, egui::PointerButton::Primary);
        h.step();
    }
    h.run();
}

/// パネルの中の名前の部品が「選んでいる」印か（一覧の行）。
fn row_selected(h: &H, label: &str) -> bool {
    let panel = panel(h);
    h.query_all_by_label(label)
        .filter(|n| panel.contains(n.rect().center()))
        .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .unwrap_or_else(|| panic!("{label}"))
        .accesskit_node()
        .toggled()
        == Some(egui::accesskit::Toggled::True)
}

// ───────── パネルの場所と名前 ─────────

#[test]
fn the_sub_tool_panel_is_the_first_tab_of_the_left_dock_in_both_languages() {
    let h = app(1600.0, 900.0, 128);
    let tab = h.state().tab_rects[&Tab::SubTools];
    assert!(tab.left() < 340.0 && tab.top() < 80.0);
    assert!(
        tab.left() < h.state().tab_rects[&Tab::Assets].left(),
        "アセットより前"
    );
    assert_eq!(Tab::SubTools.title_in(Lang::Ja), "サブツール");
    assert_eq!(Tab::SubTools.title_in(Lang::En), "Tools");
    // ツールプロパティとブラシサイズは別のタブで、サブツールの下に縦に並ぶ（その下がカラー）
    assert_eq!(Tab::ToolProperties.title_in(Lang::Ja), "ツールプロパティ");
    assert_eq!(Tab::ToolProperties.title_in(Lang::En), "Tool Properties");
    assert_eq!(Tab::BrushSize.title_in(Lang::Ja), "ブラシサイズ");
    assert_eq!(Tab::BrushSize.title_in(Lang::En), "Brush Size");
    let column: Vec<Rect> = [
        Tab::SubTools,
        Tab::ToolProperties,
        Tab::BrushSize,
        Tab::Color,
    ]
    .iter()
    .map(|t| h.state().tab_rects[t])
    .collect();
    for pair in column.windows(2) {
        assert!(pair[0].top() < pair[1].top(), "上から順: {column:?}");
        assert!(
            (pair[0].left() - pair[1].left()).abs() < 2.0,
            "同じ列: {column:?}"
        );
    }
}

// ───────── 一覧 ─────────

#[test]
fn every_tool_lists_its_sub_tools_with_the_current_one_marked_in_both_languages() {
    for lang in Lang::ALL {
        for tool in Tool::ALL {
            // （一覧の行が全部入る高さで）
            let mut h = app(1280.0, 1400.0, 128);
            language(&mut h, lang);
            pick(&mut h, tool);
            match tool.def().subtools {
                SubTools::Brushes => {
                    assert!(
                        !in_panel(&h, lang.pick("標準", "Standard")).is_empty(),
                        "{lang:?} {tool:?}"
                    );
                    assert!(
                        in_panel(&h, lang.pick("ソフト消しゴム", "Soft Eraser")).is_empty(),
                        "ブラシの一覧に消しゴムは無い"
                    );
                    assert!(
                        row_selected(&h, lang.pick("標準", "Standard")),
                        "{lang:?} {tool:?}"
                    );
                }
                SubTools::Erasers => {
                    for name in lang.pick(
                        ["ソフト消しゴム", "ハード消しゴム"],
                        ["Soft Eraser", "Hard Eraser"],
                    ) {
                        assert!(!in_panel(&h, name).is_empty(), "{lang:?} {name}");
                    }
                    assert!(
                        in_panel(&h, lang.pick("ハード円", "Hard Round")).is_empty(),
                        "消しゴムの一覧にブラシは無い"
                    );
                    // グループのタブは出ない
                    assert!(
                        in_panel(&h, lang.pick("筆", "Brush")).is_empty(),
                        "{lang:?}"
                    );
                }
                SubTools::Presets => {
                    let list = st(&h).subtool_list(tool).expect("プリセットのツール");
                    let current = list.current();
                    for b in fields::builtins(tool) {
                        let name = lang.pick(b.ja, b.en);
                        assert!(!in_panel(&h, name).is_empty(), "{lang:?} {tool:?}: {name}");
                        assert_eq!(
                            row_selected(&h, name),
                            PresetKey::Builtin(b.id) == current,
                            "{lang:?} {tool:?}: {name} の印"
                        );
                    }
                }
                SubTools::Tools(tools) => {
                    for t in tools {
                        let name = t.name_in(lang);
                        assert!(!in_panel(&h, name).is_empty(), "{lang:?} {name}");
                        assert_eq!(row_selected(&h, name), *t == tool, "{lang:?} {name} の印");
                    }
                }
                SubTools::Single => {
                    assert!(row_selected(&h, tool.name_in(lang)), "{lang:?} {tool:?}");
                }
            }
        }
    }
}

#[test]
fn the_selection_tool_rows_switch_the_tool_and_keep_the_same_list() {
    // （一覧の行が全部入る高さで）
    let mut h = app(1280.0, 1400.0, 128);
    pick(&mut h, Tool::SelectRect);
    for tool in SELECTION_TOOLS {
        click_row(&mut h, tool.name_in(Lang::Ja));
        assert_eq!(st(&h).tool, tool);
        for t in SELECTION_TOOLS {
            assert_eq!(
                row_selected(&h, t.name_in(Lang::Ja)),
                t == tool,
                "{tool:?} を選んだとき {t:?}"
            );
        }
    }
}

#[test]
fn the_preset_rows_set_each_tools_settings_and_the_bar_follows_the_row() {
    let mut h = app(1280.0, 1300.0, 128);
    let steps = st(&h).doc.undo_count();
    // バケツ: 近い色（許容がバーにも出る）→ 三角形
    pick(&mut h, Tool::Fill);
    assert_eq!(count(&h, "許容"), 0);
    click_row(&mut h, "近い色");
    assert!(st(&h).region.by_color);
    assert_eq!(
        count(&h, "許容"),
        2,
        "オプションバーとツールプロパティの両方"
    );
    click_row(&mut h, "三角形");
    assert!(!st(&h).region.by_color);
    assert_eq!(
        st(&h).region.kind,
        yolu_core::geometry::SurfaceRegionKind::Triangle
    );
    assert_eq!(count(&h, "許容"), 0);
    // グラデーション: 形と終点の組
    pick(&mut h, Tool::Gradient);
    click_row(&mut h, "放射・サブの色へ");
    assert_eq!(
        st(&h).gradient.shape,
        yolu_core::material::GradientShape::Radial
    );
    assert_eq!(st(&h).gradient.end, yolu_app::gradient::End::Sub);
    // 図形・定規
    pick(&mut h, Tool::Shape);
    click_row(&mut h, "楕円");
    assert_eq!(st(&h).drafting.figure, yolu_app::drafting::Figure::Ellipse);
    pick(&mut h, Tool::Ruler);
    click_row(&mut h, "パース（2 点）");
    assert_eq!(st(&h).rulers.kind, yolu_core::RulerKind::Perspective);
    assert!(st(&h).rulers.two_points);
    // 対称定規（線対称・2 本）と回転対称（6 本）
    click_row(&mut h, "回転対称");
    assert_eq!(st(&h).rulers.kind, yolu_core::RulerKind::Symmetry);
    assert!(!st(&h).rulers.line_symmetry);
    assert_eq!(st(&h).rulers.lines, 6);
    click_row(&mut h, "対称定規");
    assert!(st(&h).rulers.line_symmetry);
    assert_eq!(st(&h).rulers.lines, 2);
    // スポイト
    pick(&mut h, Tool::Eyedropper);
    click_row(&mut h, "全レイヤー");
    assert!(st(&h).eyedrop.all_layers);
    // 移動・変形のメッシュは、分割の欄がツールプロパティに出る。ゆがみのモード
    pick(&mut h, Tool::Move);
    assert_eq!(count(&h, "列"), 0);
    click_row(&mut h, "メッシュ");
    assert_eq!(
        st(&h).transform.advanced.kind,
        yolu_app::transform::advanced::Kind::Mesh
    );
    assert_eq!(count(&h, "列"), 1);
    pick(&mut h, Tool::Liquify);
    click_row(&mut h, "膨張");
    assert_eq!(
        st(&h).transform.advanced.mode,
        yolu_core::LiquifyMode::Expand
    );
    // ポリゴン塗りつぶし: 自分の一覧（近い色は無い）
    pick(&mut h, Tool::PolygonFill);
    assert!(in_panel(&h, "近い色").is_empty());
    click_row(&mut h, "マテリアル");
    assert_eq!(
        st(&h).region.kind,
        yolu_core::geometry::SurfaceRegionKind::Material
    );
    // サブツールの設定は文書ではない（取り消しの段・変更の印に入らない）
    assert_eq!(st(&h).doc.undo_count(), steps);
    assert!(!st(&h).modified);
}

#[test]
fn changing_a_setting_in_the_bar_moves_the_mark_to_the_matching_sub_tool() {
    let mut h = app(1280.0, 800.0, 128);
    pick(&mut h, Tool::Gradient);
    assert!(row_selected(&h, "線形・透明へ"));
    // バーの「放射」を押すと、設定が「放射・透明へ」と同じになり、印がそちらへ移る
    let radial = bar_rect(&h, "放射");
    click(&mut h, radial.center());
    assert_eq!(
        st(&h).gradient.shape,
        yolu_core::material::GradientShape::Radial
    );
    assert!(row_selected(&h, "放射・透明へ"));
    assert!(!row_selected(&h, "線形・透明へ"));
    assert!(!st(&h).subtool_is_modified(Tool::Gradient, PresetKey::Builtin("radial")));
}

#[test]
fn the_brush_and_eraser_bars_show_the_two_ruler_snaps_and_the_buttons_toggle_them() {
    for lang in Lang::ALL {
        for tool in [Tool::Brush, Tool::Eraser] {
            let mut h = app(1280.0, 800.0, 128);
            language(&mut h, lang);
            pick(&mut h, tool);
            for (label, special) in [
                (
                    lang.pick("定規にスナップ（Ctrl+1）", "Snap to Ruler (Ctrl+1)"),
                    false,
                ),
                (
                    lang.pick(
                        "特殊定規にスナップ（Ctrl+2）",
                        "Snap to Special Ruler (Ctrl+2)",
                    ),
                    true,
                ),
            ] {
                let on = |h: &H| {
                    let r = &st(h).rulers;
                    if special {
                        r.snap_special
                    } else {
                        r.snap_ruler
                    }
                };
                assert!(on(&h), "{lang:?} {tool:?} {label}: 既定は入");
                // 状態がバーに見える（押した状態の印）
                let node = h.query_all_by_label(label).next().unwrap();
                assert_eq!(
                    node.accesskit_node().toggled(),
                    Some(egui::accesskit::Toggled::True),
                    "{lang:?} {tool:?} {label}"
                );
                let at = bar_rect(&h, label).center();
                click(&mut h, at);
                assert!(!on(&h), "{lang:?} {tool:?} {label}");
                let at = bar_rect(&h, label).center();
                click(&mut h, at);
                assert!(on(&h), "{lang:?} {tool:?} {label}");
            }
        }
    }
}

#[test]
fn a_changed_setting_marks_the_row_as_modified_and_the_footer_reverts_it() {
    let mut h = app(1280.0, 900.0, 128);
    pick(&mut h, Tool::Fill);
    click_row(&mut h, "近い色");
    let key = PresetKey::Builtin("similar-colors");
    assert!(!st(&h).subtool_is_modified(Tool::Fill, key));
    h.state_mut().state.region.tolerance = 90;
    h.run();
    assert!(st(&h).subtool_is_modified(Tool::Fill, key));
    // 変更ありの行は、ツールチップにも
    assert!(
        h.query_by_label_contains("近い色（変更あり）").is_none(),
        "名前の部品は名前だけ"
    );
    let revert = dock_rect(&h, "元の設定に戻す（変更あり）");
    click(&mut h, revert.center());
    assert_eq!(st(&h).region.tolerance, 32);
    assert!(!st(&h).subtool_is_modified(Tool::Fill, key));
}

#[test]
fn pressing_the_current_row_again_or_double_clicking_it_keeps_the_settings_changed_since() {
    // （足した行まで入る高さで）
    let mut h = app(1280.0, 1400.0, 128);
    pick(&mut h, Tool::Fill);
    click_row(&mut h, "近い色");
    let similar = PresetKey::Builtin("similar-colors");
    h.state_mut().state.region.tolerance = 90;
    h.run();
    assert!(st(&h).subtool_is_modified(Tool::Fill, similar));
    // 選んでいる行をもう一度押しても、変えた設定は基準に戻らず、「変更あり」の印も消えない
    click_row(&mut h, "近い色");
    assert_eq!(st(&h).region.tolerance, 90);
    assert!(st(&h).subtool_is_modified(Tool::Fill, similar));
    // 利用者のサブツールも、ダブルクリック（名前の変更の入り口）で設定が戻らない
    let add = dock_rect(&h, "今の設定を新しいサブツールに");
    click(&mut h, add.center());
    let key = st(&h).subtool_list(Tool::Fill).unwrap().current();
    assert!(key.is_user());
    h.state_mut().state.region.tolerance = 55;
    h.run();
    assert!(st(&h).subtool_is_modified(Tool::Fill, key));
    double_click(&mut h, "プリセット");
    assert_eq!(st(&h).subtools.ui.renaming, Some((Tool::Fill, key)));
    assert_eq!(st(&h).region.tolerance, 55);
    assert!(st(&h).subtool_is_modified(Tool::Fill, key));
    // 別の行へ移って戻ると、変えたままの設定
    self::key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    click_row(&mut h, "近い色");
    assert_eq!(st(&h).region.tolerance, 90);
    click_row(&mut h, "プリセット");
    assert_eq!(st(&h).region.tolerance, 55);
}

#[test]
fn a_long_list_scrolls_to_the_added_row_the_next_row_after_a_delete_and_the_current_row_after_switching_tools(
) {
    let mut h = app(1280.0, 900.0, 128);
    pick(&mut h, Tool::Fill);
    // 組み込み 5 つに自分のを 4 つ足すと 9 行で、一覧のウィンドウ（8 行）に入りきらない。足した行は見えるところまで送る
    for name in ["プリセット", "プリセット 2", "プリセット 3", "プリセット 4"] {
        // 一覧が伸びると下の帯が動くので、押すたびに探す
        let add = dock_rect(&h, "今の設定を新しいサブツールに");
        click(&mut h, add.center());
        h.run();
        assert!(
            row(&h, name).height() > 20.0,
            "{name} の行がウィンドウに見えている: {:?}",
            in_panel(&h, name)
        );
        assert!(row_selected(&h, name));
    }
    // ツールを替えて戻ると、今の行（足した最後の行）が見える
    pick(&mut h, Tool::Gradient);
    pick(&mut h, Tool::Fill);
    assert!(row(&h, "プリセット 4").height() > 20.0);
    // 最後の行を消すと、前の行が選ばれ、その行が見える
    let delete = dock_rect(&h, "サブツールを削除");
    click(&mut h, delete.center());
    h.run();
    assert!(row_selected(&h, "プリセット 3"));
    assert!(row(&h, "プリセット 3").height() > 20.0);
}

#[test]
fn the_footer_adds_duplicates_and_deletes_user_sub_tools_and_each_step_is_saved() {
    let dir = std::env::temp_dir().join(format!("yolu-subtools-ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    let mut h = app(1280.0, 1300.0, 128);
    h.state_mut().state.attach_subtool_store(dir.clone());
    pick(&mut h, Tool::Fill);
    h.state_mut().state.region.tolerance = 77;
    h.run();
    let file = dir.join("fill.ylsubtool");
    // 追加: 今の設定が新しいサブツールになり、ファイルに書く
    let add = dock_rect(&h, "今の設定を新しいサブツールに");
    click(&mut h, add.center());
    assert!(file.exists());
    assert_eq!(st(&h).subtool_list(Tool::Fill).unwrap().user_count(), 1);
    assert!(!in_panel(&h, "プリセット").is_empty(), "一覧に出る");
    assert!(row_selected(&h, "プリセット"));
    assert!(std::fs::read_to_string(&file)
        .unwrap()
        .contains("preset.1.tolerance=77"));
    // 複製・削除
    let duplicate = dock_rect(&h, "サブツールを複製");
    click(&mut h, duplicate.center());
    assert_eq!(st(&h).subtool_list(Tool::Fill).unwrap().user_count(), 2);
    assert!(!in_panel(&h, "プリセット のコピー").is_empty());
    let delete = dock_rect(&h, "サブツールを削除");
    click(&mut h, delete.center());
    assert_eq!(st(&h).subtool_list(Tool::Fill).unwrap().user_count(), 1);
    // 組み込みは消せない（ボタンが押せない）
    click_row(&mut h, "三角形");
    let delete = dock_rect(&h, "組み込みのサブツールは消せません");
    assert!(h
        .query_all_by_label("組み込みのサブツールは消せません")
        .next()
        .unwrap()
        .accesskit_node()
        .is_disabled());
    click(&mut h, delete.center());
    assert!(st(&h)
        .subtool_list(Tool::Fill)
        .unwrap()
        .entry(PresetKey::Builtin("triangle"))
        .is_some());
    // 行の右クリックのメニューで消す
    let at = row(&h, "プリセット").center();
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Secondary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.event(Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Secondary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run();
    let item = popup_item(&h, "削除").center();
    click(&mut h, item);
    assert_eq!(st(&h).subtool_list(Tool::Fill).unwrap().user_count(), 0);
    assert!(!file.exists(), "なくなればファイルも消える");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_user_eraser_is_added_to_the_eraser_list_only_and_a_user_brush_to_the_brush_list_only() {
    let mut h = app(1280.0, 900.0, 128);
    // 消しゴムの一覧で足す（今の設定が、消しゴムのグループの自分のブラシになる）
    pick(&mut h, Tool::Eraser);
    h.state_mut().state.brush.radius = 9.0;
    h.run();
    let add = dock_rect(&h, "今の設定を新しいブラシに");
    click(&mut h, add.center());
    let key = st(&h).brushes.lib.current();
    assert!(key.is_user());
    let entry = st(&h).brushes.lib.entry(key).unwrap();
    assert!(
        entry.group.is_eraser(),
        "消しゴムの一覧から足したブラシは消しゴムのグループ"
    );
    assert_eq!(st(&h).tool, Tool::Eraser);
    assert!(!in_panel(&h, "ブラシ").is_empty(), "消しゴムの一覧に出る");
    // ブラシの一覧には出ない
    pick(&mut h, Tool::Brush);
    assert!(in_panel(&h, "ブラシ").is_empty());
    // ブラシの一覧で足すと、ブラシのグループ
    let add = dock_rect(&h, "今の設定を新しいブラシに");
    click(&mut h, add.center());
    let brush_key = st(&h).brushes.lib.current();
    assert!(!st(&h)
        .brushes
        .lib
        .entry(brush_key)
        .unwrap()
        .group
        .is_eraser());
    assert_ne!(brush_key, key);
    // 消しゴムへ戻ると、消しゴムの一覧で選んでいたブラシ（足した消しゴム）
    pick(&mut h, Tool::Eraser);
    assert_eq!(st(&h).brushes.lib.current(), key);
    assert_eq!(st(&h).brush.radius, 9.0);
}

#[test]
fn double_click_renames_a_user_sub_tool_and_built_in_names_do_not_change() {
    let mut h = app(1280.0, 900.0, 128);
    pick(&mut h, Tool::Gradient);
    let add = dock_rect(&h, "今の設定を新しいサブツールに");
    click(&mut h, add.center());
    let key = st(&h).subtool_list(Tool::Gradient).unwrap().current();
    double_click(&mut h, "プリセット");
    assert_eq!(st(&h).subtools.ui.renaming, Some((Tool::Gradient, key)));
    self::key(&h, Key::A, Modifiers::COMMAND);
    h.event(Event::Text("太い放射".into()));
    self::key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert_eq!(
        st(&h)
            .subtool_list(Tool::Gradient)
            .unwrap()
            .entry(key)
            .unwrap()
            .name,
        "太い放射"
    );
    assert_eq!(st(&h).subtools.ui.renaming, None);
    assert!(!in_panel(&h, "太い放射").is_empty());
    // Esc でやめる（名前は変わらない）
    double_click(&mut h, "太い放射");
    h.event(Event::Text("あ".into()));
    self::key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert_eq!(
        st(&h)
            .subtool_list(Tool::Gradient)
            .unwrap()
            .entry(key)
            .unwrap()
            .name,
        "太い放射"
    );
    assert_eq!(st(&h).subtools.ui.renaming, None);
    // 組み込みはダブルクリックしても名前を変えない
    double_click(&mut h, "放射・透明へ");
    assert_eq!(st(&h).subtools.ui.renaming, None);
}

#[test]
fn headless_the_app_reads_the_sub_tool_folder_next_to_its_settings_and_reports_a_broken_file() {
    use yolu_app::pen::PenInput;
    use yolu_app::subtool::SubToolAction;
    let dir = std::env::temp_dir().join(format!("yolu-subtools-app-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let settings = dir.join("settings.conf");
    let ctx = egui::Context::default();
    let mut first =
        YoluApp::for_context_with_settings(&ctx, Some(settings.clone()), PenInput::detached());
    first.state.apply(Action::SelectTool(Tool::Ruler));
    first.state.apply(Action::SubTool(SubToolAction::Select(
        Tool::Ruler,
        PresetKey::Builtin("concentric"),
    )));
    first
        .state
        .apply(Action::SubTool(SubToolAction::Add(Tool::Ruler)));
    let key = first.state.subtool_list(Tool::Ruler).unwrap().current();
    assert!(dir.join("subtools").join("ruler.ylsubtool").is_file());
    // 別の起動で読み戻す（読んだあと、ツールを選ぶと同じ設定）
    let mut again =
        YoluApp::for_context_with_settings(&ctx, Some(settings.clone()), PenInput::detached());
    assert!(again
        .state
        .subtool_list(Tool::Ruler)
        .unwrap()
        .entry(key)
        .is_some());
    again.state.apply(Action::SelectTool(Tool::Ruler));
    again
        .state
        .apply(Action::SubTool(SubToolAction::Select(Tool::Ruler, key)));
    assert_eq!(again.state.rulers.kind, yolu_core::RulerKind::Concentric);
    // 壊れたファイルは、起動の知らせに出る（ほかのツールの保存は読む）
    std::fs::write(dir.join("subtools").join("gradient.ylsubtool"), "x").unwrap();
    let broken = YoluApp::for_context_with_settings(&ctx, Some(settings), PenInput::detached());
    assert!(
        broken.state.message.contains("gradient.ylsubtool"),
        "{}",
        broken.state.message
    );
    assert!(broken
        .state
        .subtool_list(Tool::Ruler)
        .unwrap()
        .entry(key)
        .is_some());
    // 設定が無ければ保存しない
    let mut none = YoluApp::for_context_with_settings(&ctx, None, PenInput::detached());
    none.state
        .apply(Action::SubTool(SubToolAction::Add(Tool::Fill)));
    assert!(none.state.subtools.store.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

// ───────── ツールプロパティとオプションバー ─────────

#[test]
fn the_option_bar_and_the_tool_properties_show_the_same_values() {
    let mut h = app(1600.0, 900.0, 128);
    // ブラシ: 直径と不透明度は両方に、硬さ・流量・間隔・筆圧はツールプロパティだけ
    h.state_mut().state.brush.radius = 20.0;
    h.state_mut().state.brush.opacity = 0.5;
    h.run();
    for (label, value) in [("直径", 40.0), ("不透明度", 50.0)] {
        let nodes: Vec<_> = h
            .query_all_by_label(label)
            .filter(|n| n.rect().left() < 340.0 || n.rect().top() < 60.0)
            .collect();
        assert_eq!(nodes.len(), 2, "{label}: バーとツールプロパティ");
        for n in nodes {
            assert_eq!(n.accesskit_node().numeric_value(), Some(value), "{label}");
        }
    }
    for label in ["硬さ", "流量", "間隔", "筆圧で直径を変える"] {
        assert!(
            in_panel(&h, label).len() == 1,
            "{label} はツールプロパティに"
        );
        assert!(
            h.query_all_by_label(label).all(|n| n.rect().top() > 62.0),
            "{label} はオプションバーに無い"
        );
    }
    // 消しゴムも同じ
    pick(&mut h, Tool::Eraser);
    for label in ["直径", "不透明度"] {
        assert_eq!(
            h.query_all_by_label(label)
                .filter(|n| n.rect().left() < 340.0 || n.rect().top() < 60.0)
                .count(),
            2,
            "{label}"
        );
    }
    // 選択ペン: 直径はバーとツールプロパティ、同じ値（ブラシと共通）
    pick(&mut h, Tool::SelectPen);
    let diameter = st(&h).brush.radius * 2.0;
    let sizes: Vec<_> = h
        .query_all_by_label("直径")
        .filter(|n| n.rect().left() < 340.0 || n.rect().top() < 60.0)
        .map(|n| n.accesskit_node().numeric_value())
        .collect();
    assert_eq!(sizes, [Some(diameter as f64), Some(diameter as f64)]);
    // 自動選択: 許容値はバーとツールプロパティ、隣接と全レイヤーはツールプロパティだけ
    pick(&mut h, Tool::Wand);
    assert_eq!(count(&h, "許容値"), 2);
    assert_eq!(count(&h, "隣接"), 1);
    assert!(h
        .query_all_by_label("隣接")
        .all(|n| n.rect().left() < 340.0));
}

#[test]
fn the_brush_size_panel_is_empty_for_tools_without_a_size() {
    for tool in Tool::ALL {
        // （丸が 2 段とも入る高さで）
        let mut h = app(1280.0, 1100.0, 128);
        pick(&mut h, tool);
        // どのツールでもタブはあるが、丸（「16 px」など）は大きさを持つツールだけ
        assert!(
            h.state().tab_rects.contains_key(&Tab::BrushSize),
            "{tool:?}"
        );
        assert_eq!(count(&h, "16 px"), usize::from(tool.is_sized()), "{tool:?}");
        // ツールプロパティの中身（タブの帯の下から、ブラシサイズの上まで）に描いた文字: 「ツールプロパティ」の見出しの帯は無く（タブの名前だけ）、
        // 設定のあるブラシでは中身の文字が出ている
        let (bar, size) = (
            h.state().tab_rects[&Tab::ToolProperties],
            h.state().tab_rects[&Tab::BrushSize],
        );
        let mut texts = Vec::new();
        fn collect(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
                egui::Shape::Text(t) => out.push((t.galley.job.text.clone(), t.pos)),
                _ => {}
            }
        }
        for shape in &h.output().shapes {
            collect(&shape.shape, &mut texts);
        }
        let body: Vec<&str> = texts
            .iter()
            .filter(|(_, p)| p.x < bar.right() && p.y > bar.bottom() && p.y < size.top())
            .map(|(t, _)| t.as_str())
            .collect();
        assert!(
            !body.contains(&"ツールプロパティ"),
            "{tool:?}: ツールプロパティはタブの名前だけ（見出しの帯は無い） {body:?}"
        );
        if tool == Tool::Brush {
            assert!(!body.is_empty(), "{tool:?}: 中身が出ている");
        }
    }
    assert!(Tool::Brush.is_sized() && Tool::Eraser.is_sized() && Tool::SelectPen.is_sized());
    assert!(!Tool::Fill.is_sized() && !Tool::Wand.is_sized() && !Tool::Liquify.is_sized());
}

#[test]
fn every_tool_has_its_settings_in_the_tool_properties_and_none_in_the_right_properties() {
    let mut h = app(1600.0, 1000.0, 128);
    // 右のプロパティ（ステンシル・レイヤーのタブ）の中身は、ツールによらない
    let props_tab = h.state().tab_rects[&Tab::Properties];
    let right = move |r: Rect| r.left() > props_tab.left() - 2.0 && r.top() > props_tab.top();
    let fingerprint = |h: &H| -> Vec<String> {
        let mut found: Vec<(i32, i32, String)> = h
            .root()
            .children_recursive()
            .filter_map(|n| {
                let label = n.accesskit_node().label()?;
                let r = n.rect();
                right(r).then(|| (r.top() as i32, r.left() as i32, label.to_string()))
            })
            .collect();
        found.sort();
        found.into_iter().map(|f| f.2).collect()
    };
    let base = fingerprint(&h);
    assert!(
        base.iter().any(|l| l == "ステンシル") && base.iter().any(|l| l == "レイヤー"),
        "{base:?}"
    );
    for tool in Tool::ALL {
        pick(&mut h, tool);
        assert_eq!(
            fingerprint(&h),
            base,
            "{tool:?}: 右のプロパティはレイヤーで決まる（ツールでは替わらない）"
        );
    }
    // 塗りつぶしのレイヤーを選ぶと、右はレイヤーの欄になる（ツールは関係なく同じ）。ツールプロパティはツールのまま
    pick(&mut h, Tool::Fill);
    h.state_mut()
        .state
        .apply(Action::M2(yolu_app::m2::Edit::NewFill));
    h.run();
    let kind = st(&h)
        .selected_layer
        .and_then(|id| st(&h).doc.layer(id))
        .map(|l| l.kind());
    assert_eq!(kind, Some(LayerKind::Fill));
    let on_fill_layer = fingerprint(&h);
    pick(&mut h, Tool::Gradient);
    assert_eq!(fingerprint(&h), on_fill_layer);
    pick(&mut h, Tool::SelectRect);
    assert_eq!(fingerprint(&h), on_fill_layer);
}

// ───────── 狭いウィンドウ・日英・説明文 ─────────

/// 描いた文字を全部集める（クリップの中に収まっているかも見る）。
fn drawn(h: &H) -> (Vec<String>, Vec<String>) {
    fn walk(shape: &Shape, clip: Rect, texts: &mut Vec<String>, clipped: &mut Vec<String>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, clip, texts, clipped)),
            Shape::Text(text) => {
                let bounds = Rect::from_min_size(text.pos, text.galley.size());
                texts.push(text.galley.job.text.clone());
                let visible = clip.y_range().contains(bounds.center().y);
                if visible
                    && (bounds.left() < clip.left() - 1.0 || bounds.right() > clip.right() + 1.0)
                {
                    clipped.push(format!(
                        "{}: {bounds:?} clip={clip:?}",
                        text.galley.job.text
                    ));
                }
            }
            _ => {}
        }
    }
    let (mut texts, mut clipped) = (Vec::new(), Vec::new());
    for shape in &h.output().shapes {
        walk(&shape.shape, shape.clip_rect, &mut texts, &mut clipped);
    }
    (texts, clipped)
}

/// 使い方の説明文が無い（名前・状態・短い理由だけ）。
fn assert_no_how_to(what: &str, text: &str) {
    const HOW_TO: &[&str] = &[
        "してください",
        "ください",
        "クリック",
        "ドラッグして",
        "押して",
        "タップ",
        "選んで",
        "入力して",
        "click",
        "drag ",
        "press ",
        "please",
        "choose ",
        "select a",
        "to add",
        "you can",
        "tap ",
    ];
    let lower = text.to_lowercase();
    for word in HOW_TO {
        assert!(
            !lower.contains(word),
            "{what}: 使い方の語「{word}」: {text}"
        );
    }
    assert!(
        text.chars().count() <= 50,
        "{what}: 長い（{} 字）: {text}",
        text.chars().count()
    );
}

#[test]
fn the_panel_and_the_bar_of_every_tool_fit_the_smallest_window_in_both_languages_without_instruction_text(
) {
    for lang in Lang::ALL {
        for tool in Tool::ALL {
            let mut h = app(960.0, 640.0, 128);
            language(&mut h, lang);
            pick(&mut h, tool);
            let what = format!("{lang:?} {tool:?}");
            yolu_app::ui::widgets::record_truncations(true);
            yolu_app::ui::widgets::take_truncations();
            h.step();
            let truncated = yolu_app::ui::widgets::take_truncations();
            yolu_app::ui::widgets::record_truncations(false);
            let (texts, clipped) = drawn(&h);
            assert!(clipped.is_empty(), "{what}: 切れた文字 {clipped:#?}");
            // ツールの名前・値は詰めない（一覧の行・ツールプロパティ・バー）。詰められたのは、ブラシの名前（水彩の縁）など長い固有の名前と、
            // 最小のウィンドウのツールプロパティの組（テキストの設定が入りきらず、スクロールの帯が幅を取る）のフォントの名前だけ
            for t in &truncated {
                assert!(
                    t == "Watercolor Edge"
                        || t.starts_with("Texture Set")
                        || t == "テクスチャセット 1"
                        || (tool == Tool::Text && t.starts_with("BIZ UDP")),
                    "{what}: 「…」に詰められた {t}"
                );
            }
            for text in &texts {
                assert_no_how_to(&what, text);
            }
            if lang == Lang::En {
                assert!(
                    texts.iter().all(|t| !has_japanese(t) || t.contains('→')),
                    "{what}: 英語の画面に日本語: {:?}",
                    texts.iter().filter(|t| has_japanese(t)).collect::<Vec<_>>()
                );
            }
        }
    }
}

#[test]
fn nothing_in_the_selection_panel_is_clipped_at_the_smallest_window() {
    for lang in Lang::ALL {
        let mut h = app(960.0, 640.0, 128);
        language(&mut h, lang);
        pick(&mut h, Tool::SelectRect);
        h.state_mut()
            .state
            .apply(Action::Sel(yolu_app::selection::SelAction::Edit(
                yolu_app::selection::SelEdit::All,
            )));
        h.run();
        let (_, clipped) = drawn(&h);
        assert!(clipped.is_empty(), "{lang:?}: {clipped:#?}");
        // スクロールしても（下のほうの行）切れない
        for scroll in [80.0, 160.0, 320.0] {
            h.state_mut().state.subtools.ui.props_scroll[Tool::SelectRect as usize] = scroll;
            h.run();
            let (_, clipped) = drawn(&h);
            assert!(clipped.is_empty(), "{lang:?} {scroll}: {clipped:#?}");
        }
    }
}

// ───────── キー・ツールを替える ─────────

#[test]
fn switching_tools_with_keys_changes_the_list_and_the_selection_of_each_tool_stays() {
    let mut h = app(1280.0, 900.0, 128);
    key(&h, Key::G, Modifiers::NONE);
    h.run();
    click_row(&mut h, "メッシュの塊");
    key(&h, Key::B, Modifiers::NONE);
    h.run();
    assert!(!in_panel(&h, "標準").is_empty());
    assert!(in_panel(&h, "メッシュの塊").is_empty());
    key(&h, Key::G, Modifiers::NONE);
    h.run();
    assert!(
        row_selected(&h, "メッシュの塊"),
        "バケツへ戻ると、バケツの選び"
    );
    // 消しゴム: E、ブラシへ戻ると描くツールの最後のブラシ
    key(&h, Key::E, Modifiers::NONE);
    h.run();
    assert!(!in_panel(&h, "ソフト消しゴム").is_empty());
    click_row(&mut h, "ソフト消しゴム");
    key(&h, Key::B, Modifiers::NONE);
    h.run();
    assert!(in_panel(&h, "ソフト消しゴム").is_empty());
    assert_eq!(st(&h).tool, Tool::Brush);
}

// ───────── 見た目（パネルの中だけを撮る） ─────────

/// パネルの全体の画像（タブの帯から、下のカラーのパネルの上まで）を撮って、正解の絵と比べる。
fn shot(h: &mut H, name: &str) {
    shot_width(h, name, 300.0);
}

/// `shot` の、横幅（点）を決める版（狭い列の絵）。
fn shot_width(h: &mut H, name: &str, width: f32) {
    // 直前に押した所のポインタが絵に残らないように
    h.event(Event::PointerGone);
    h.step();
    let tab = h.state().tab_rects[&Tab::SubTools];
    let color = h.state().tab_rects[&Tab::Color];
    let rect = Rect::from_min_max(
        pos2(tab.left() - 2.0, tab.top()),
        pos2(tab.left() + width, color.top()),
    );
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

#[test]
fn snapshots_of_the_sub_tool_panel_for_the_bucket_the_selection_and_the_liquify_tools() {
    for (lang, name) in [(Lang::Ja, "ja"), (Lang::En, "en")] {
        for (tool, label) in [
            (Tool::Fill, "fill"),
            (Tool::SelectRect, "select"),
            (Tool::Gradient, "gradient"),
            (Tool::Liquify, "liquify"),
            (Tool::Eraser, "eraser"),
        ] {
            let mut h = app(1280.0, 800.0, 128);
            language(&mut h, lang);
            h.state_mut().state.sel.animate = false;
            pick(&mut h, tool);
            if tool == Tool::Fill {
                click_row(&mut h, lang.pick("近い色", "Similar colors"));
            }
            shot(&mut h, &format!("subtools_{label}_{name}"));
        }
    }
}

#[test]
fn snapshots_of_the_selection_tool_properties_the_four_modes_the_narrow_column_and_the_selection_pen(
) {
    for (lang, name) in [(Lang::Ja, "ja"), (Lang::En, "en")] {
        let mut h = app(1280.0, 800.0, 128);
        language(&mut h, lang);
        h.state_mut().state.sel.animate = false;
        pick(&mut h, Tool::SelectRect);
        shot(&mut h, &format!("selection_props_{name}"));
        // 選択ペン（選択ペンと選択消しの 2 つ）
        pick(&mut h, Tool::SelectPen);
        shot(&mut h, &format!("selection_pen_props_{name}"));
    }
    // 「⋯」で 4 つ開いた所
    let mut h = app(1280.0, 800.0, 128);
    h.state_mut().state.sel.animate = false;
    pick(&mut h, Tool::SelectRect);
    h.state_mut()
        .state
        .apply(Action::Sel(yolu_app::selection::SelAction::Ui(
            yolu_app::selection::SelUiOp::AllModes(true),
        )));
    h.run();
    shot(&mut h, "selection_props_all_modes");
    // 共通を選んでいる所（畳む設定でも 4 つ。「⋯」は点いたまま押せない）
    let mut h = app(1280.0, 800.0, 128);
    h.state_mut().state.sel.animate = false;
    pick(&mut h, Tool::SelectRect);
    h.state_mut()
        .state
        .apply(Action::Sel(yolu_app::selection::SelAction::Ui(
            yolu_app::selection::SelUiOp::Combine(yolu_app::engine::SelectionCombine::Intersect),
        )));
    h.run();
    shot(&mut h, "selection_props_intersect");
    // 狭い列: 名前を外してアイコンだけにする（英語は名前が長い）
    for (lang, name) in [(Lang::Ja, "ja"), (Lang::En, "en")] {
        let mut h = app(1000.0, 640.0, 128);
        language(&mut h, lang);
        h.state_mut().state.sel.animate = false;
        let mut dock = egui_dock::DockState::new(vec![Tab::Canvas]);
        let surface = dock.main_surface_mut();
        let [_, left] = surface.split_left(egui_dock::NodeIndex::root(), 0.17, vec![Tab::SubTools]);
        let [_, props] = surface.split_below(left, 0.3, vec![Tab::ToolProperties]);
        surface.split_below(props, 0.8, vec![Tab::Color]);
        h.state_mut().dock = dock;
        pick(&mut h, Tool::SelectRect);
        shot_width(&mut h, &format!("selection_props_narrow_{name}"), 172.0);
        pick(&mut h, Tool::SelectPen);
        shot_width(&mut h, &format!("selection_pen_props_narrow_{name}"), 172.0);
    }
}
