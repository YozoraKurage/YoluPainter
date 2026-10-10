//! 選択範囲の作り方（CLIP STUDIO の「作成方法」に当たるもの）: 作成方法のアイコンの組と修飾キーの一時表示・選択のツールの設定
//! （アンチエイリアス・縦横比・中心から・角の丸め）・選択ペンと選択消し・境界をぼかす・保存と読み込み・クイックマスク。
//! `headless_` で始まる試験は画面を描かず、Wine でも回る。
use crate::common;

use common::*;
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::engine::{
    Channel, Document, SelectionCombine, SelectionMask, TileCoord, DEFAULT_WORKING_BUDGET_BYTES,
};
use yolu_app::lang::Lang;
use yolu_app::m2::UiOp;
use yolu_app::pen::PenSample;
use yolu_app::selection::canvas::active_symmetry;
use yolu_app::selection::pen::{dab_coverage, PenError, PenParams, PenStroke, PEN_BUDGET_BYTES};
use yolu_app::selection::saved::{SavedOp, MAX_SAVED, SAVED_BUDGET_BYTES};
use yolu_app::selection::{ModifyKind, SelAction, SelEdit, SelUiOp};
use yolu_app::sets::TextureSets;
use yolu_app::state::{Action, AppState, StrokeSource, Tool};
use yolu_app::YoluApp;

type H = Harness<'static, YoluApp>;

// ───────── ツール ─────────

fn st(h: &H) -> &AppState {
    &h.state().state
}

fn pick_tool(h: &mut H, tool: Tool) {
    h.state_mut().state.apply(Action::SelectTool(tool));
    h.run();
}

fn undo(h: &mut H) {
    key(h, Key::Z, Modifiers::COMMAND);
    h.run();
}

fn redo(h: &mut H) {
    key(h, Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    h.run();
}

fn steps(h: &H) -> usize {
    st(h).doc.undo_count()
}

fn at(h: &H, dx: f32, dy: f32) -> Pos2 {
    offset(canvas_rect(h).center(), dx, dy)
}

/// 画面の点をキャンバスの座標へ。
fn to_canvas(h: &H, p: Pos2) -> (f64, f64) {
    let s = st(h);
    s.view
        .view(canvas_rect(h), s.doc.width(), s.doc.height())
        .to_canvas(p)
}

fn amount_at(h: &H, p: Pos2) -> u8 {
    let (x, y) = to_canvas(h, p);
    st(h)
        .doc
        .selection()
        .map_or(0, |m| m.amount(x.floor() as u32, y.floor() as u32))
}

/// 選択範囲の量が 0 でない画素を囲む矩形（x0, y0, x1, y1。半開区間）。
fn bounds(m: &SelectionMask) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = (m.width(), m.height());
    let bytes = m.to_canvas_bytes();
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if bytes[(y * w + x) as usize] > 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    (x1 > 0).then_some((x0, y0, x1, y1))
}

fn drag_with(h: &mut H, points: &[Pos2], press: Modifiers, release: Modifiers) {
    h.event(Event::PointerMoved(points[0]));
    h.event(Event::PointerButton {
        pos: points[0],
        button: PointerButton::Primary,
        pressed: true,
        modifiers: press,
    });
    h.step();
    for p in &points[1..] {
        move_to(h, *p);
        h.step();
    }
    h.event(Event::PointerButton {
        pos: *points.last().unwrap(),
        button: PointerButton::Primary,
        pressed: false,
        modifiers: release,
    });
    h.step();
    h.run();
}

fn click_by(h: &mut H, dx: f32, dy: f32) {
    let p = at(h, dx, dy);
    click(h, p);
}

fn drag_by(h: &mut H, points: &[(f32, f32)]) {
    let pts: Vec<Pos2> = points.iter().map(|&(x, y)| at(h, x, y)).collect();
    drag(h, &pts);
}

/// ツールチップが名前になっているボタンの「選んでいる・点いている」印。オプションバーとツールプロパティの両方にあるボタンは、2 つが同じ印であること。
fn lit(h: &H, label: &str) -> bool {
    let marks: Vec<bool> = h
        .get_all_by_label(label)
        .map(|n| n.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True))
        .collect();
    assert!(!marks.is_empty(), "{label}");
    assert!(marks.iter().all(|m| *m == marks[0]), "{label}: {marks:?}");
    marks[0]
}

/// オプションバーのボタンを押す。
fn click_bar(h: &mut H, label: &str) {
    let at = bar_rect(h, label).center();
    click(h, at);
}

/// オプションバーのアイコンの名前（ツールチップ）。ツールプロパティの帯の名前は `DOCK_*`（新規は両方とも「新規」）。
const NEW: &str = "新規";
const ADD: &str = "追加（Shift）";
const SUB: &str = "削除（Ctrl）";
const ISECT: &str = "共通（Shift+Ctrl）";
const DOCK_MODES: [&str; 4] = ["新規", "追加", "削除", "共通"];
const MORE: &str = "すべての作成方法";

/// 左のドックのツールプロパティ（ドック側）の部品の「点いている」印。
fn dock_lit(h: &H, label: &str) -> bool {
    let target = dock_rect(h, label);
    h.get_all_by_label(label)
        .find(|n| n.rect() == target)
        .map(|n| n.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True))
        .unwrap_or_else(|| panic!("{label}"))
}

/// ドック側の部品が押せない（無効）か。
fn dock_disabled(h: &H, label: &str) -> bool {
    let target = dock_rect(h, label);
    h.get_all_by_label(label)
        .find(|n| n.rect() == target)
        .unwrap_or_else(|| panic!("{label}"))
        .accesskit_node()
        .is_disabled()
}

/// ドックの帯に出ている作成方法の名前（左から）。
fn dock_modes(h: &H) -> Vec<&'static str> {
    DOCK_MODES
        .into_iter()
        .filter(|label| {
            h.query_all_by_label(label).any(|n| {
                let r = n.rect();
                r.left() < 390.0 && r.top() > 62.0 && r.bottom() < 700.0
            })
        })
        .collect()
}

fn run(s: &mut AppState, e: SelEdit) {
    s.apply(Action::Sel(SelAction::Edit(e)));
}

fn rect(mode: SelectionCombine, x0: i64, y0: i64, x1: i64, y1: i64) -> SelEdit {
    SelEdit::Rect {
        x0,
        y0,
        x1,
        y1,
        mode,
    }
}

fn pen_at(pos: Pos2, contact: bool, pressure: f32, eraser: bool) -> PenSample {
    PenSample {
        pos: [pos.x, pos.y],
        pressure,
        tilt: yolu_app::engine::Tilt::default(),
        rotation: None,
        contact,
        eraser,
        barrel: false,
        pointer_id: 7,
        time_ms: 0,
    }
}

/// ペンで点をなぞる（最初と途中は触れて、最後は離す）。
fn pen_stroke(h: &mut H, points: &[Pos2], pressure: f32, eraser: bool) {
    for p in points {
        h.state().pen().push(pen_at(*p, true, pressure, eraser));
        h.step();
    }
    let last = *points.last().unwrap();
    h.state().pen().push(pen_at(last, false, pressure, eraser));
    h.step();
    h.run();
}

/// 太さを決めたブラシ（硬い・不透明・筆圧は使わない）。
fn set_brush(h: &mut H, diameter: f32, hardness: f32, opacity: f32) {
    let b = &mut h.state_mut().state.brush;
    b.radius = diameter / 2.0;
    b.hardness = hardness;
    b.opacity = opacity;
    b.pressure_size = false;
    b.pressure_opacity = false;
}

// ───────── 作成方法のアイコンの組 ─────────

#[test]
fn the_creation_mode_buttons_pick_the_mode_and_light_up_with_the_modifier_keys() {
    let mut h = app(1000.0, 640.0, 256);
    pick_tool(&mut h, Tool::SelectRect);
    let bar = [NEW, ADD, SUB, ISECT];
    // 初めは「新規」だけが点く（バーは 4 つ、ツールプロパティは 3 つ）
    let check = |h: &H, shown: usize, why: &str| {
        for (i, label) in bar.iter().enumerate() {
            assert_eq!(lit(h, label), i == shown, "{why}: バーの {label}");
        }
        for (i, label) in dock_modes(h).iter().enumerate() {
            assert_eq!(dock_lit(h, label), i == shown, "{why}: 帯の {label}");
        }
    };
    check(&h, 0, "初め");
    assert_eq!(dock_modes(&h), vec!["新規", "追加", "削除"]);
    // ボタンで選ぶ（オプションバーの値が変わる。帯も同じ値）
    for (mode, label, shown) in [
        (SelectionCombine::Add, ADD, 1),
        (SelectionCombine::Subtract, SUB, 2),
        (SelectionCombine::Intersect, ISECT, 3),
        (SelectionCombine::Replace, NEW, 0),
    ] {
        click_bar(&mut h, label);
        assert_eq!(st(&h).sel.combine, mode);
        check(&h, shown, label);
    }
    // 帯のボタンでも選べる
    for (mode, name) in [
        (SelectionCombine::Add, "追加"),
        (SelectionCombine::Subtract, "削除"),
        (SelectionCombine::Replace, "新規"),
    ] {
        let at = dock_rect(&h, name).center();
        click(&mut h, at);
        assert_eq!(st(&h).sel.combine, mode, "帯の {name}");
    }
    // 修飾キーを押しているあいだ、効く作成方法が一時的に点く（選んでいる値は変わらない）
    for (mods, shown) in [
        (Modifiers::SHIFT, 1),
        (Modifiers::COMMAND, 2),
        (Modifiers::COMMAND | Modifiers::SHIFT, 3),
    ] {
        h.event(Event::ModifiersChanged(mods));
        h.step();
        h.run();
        for (i, label) in bar.iter().enumerate() {
            assert_eq!(
                lit(&h, label),
                i == shown,
                "{mods:?} を押しているとき {label}"
            );
        }
        // 帯は畳んでいるので、共通は「⋯」が代わりに点く
        for (i, label) in dock_modes(&h).iter().enumerate() {
            assert_eq!(dock_lit(&h, label), i == shown, "{mods:?} の帯の {label}");
        }
        assert_eq!(
            dock_lit(&h, MORE),
            shown == 3,
            "{mods:?}: 畳んでいる帯は共通のあいだ「⋯」が点く"
        );
        assert_eq!(st(&h).sel.combine, SelectionCombine::Replace);
    }
    // 離すと選んでいる作成方法に戻る
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.step();
    h.run();
    check(&h, 0, "離した後");
    assert!(!dock_lit(&h, MORE));
}

#[test]
fn the_creation_mode_buttons_follow_the_language_and_all_selection_tools_show_them() {
    let mut h = app(1000.0, 640.0, 256);
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(Lang::En)));
    for tool in [
        Tool::SelectRect,
        Tool::SelectEllipse,
        Tool::Lasso,
        Tool::Polygon,
        Tool::Wand,
    ] {
        pick_tool(&mut h, tool);
        // オプションバー: 名前とキーだけのツールチップ
        for label in [
            "New",
            "Add (Shift)",
            "Subtract (Ctrl)",
            "Intersect (Shift+Ctrl)",
        ] {
            bar_rect(&h, label);
        }
        // ツールプロパティ: 新規・追加・削除に「⋯」
        for label in ["New", "Add", "Subtract", "All modes"] {
            dock_rect(&h, label);
        }
        assert!(
            h.query_all_by_label("Intersect").next().is_none(),
            "畳んでいるあいだは共通の帯のボタンは無い"
        );
    }
    assert_eq!(
        yolu_app::selection::combine_name(Lang::Ja, SelectionCombine::Subtract),
        "削除"
    );
    assert_eq!(
        yolu_app::selection::combine_name(Lang::En, SelectionCombine::Replace),
        "New"
    );
    for (mode, ja, en) in [
        (SelectionCombine::Replace, "新規", "New"),
        (SelectionCombine::Add, "追加（Shift）", "Add (Shift)"),
        (
            SelectionCombine::Subtract,
            "削除（Ctrl）",
            "Subtract (Ctrl)",
        ),
        (
            SelectionCombine::Intersect,
            "共通（Shift+Ctrl）",
            "Intersect (Shift+Ctrl)",
        ),
    ] {
        assert_eq!(yolu_app::selection::combine_tooltip(Lang::Ja, mode), ja);
        assert_eq!(yolu_app::selection::combine_tooltip(Lang::En, mode), en);
    }
}

#[test]
fn the_more_button_shows_all_four_modes_and_the_choice_survives_a_restart() {
    let mut h = app(1000.0, 640.0, 256);
    pick_tool(&mut h, Tool::SelectRect);
    assert!(!st(&h).settings().selection_all_modes, "既定は畳む");
    assert_eq!(dock_modes(&h), vec!["新規", "追加", "削除"]);
    let more = dock_rect(&h, MORE);
    click(&mut h, more.center());
    assert_eq!(dock_modes(&h), DOCK_MODES.to_vec(), "「⋯」で 4 つになる");
    assert!(dock_lit(&h, MORE), "開いているあいだ「⋯」が点く");
    assert!(st(&h).settings().selection_all_modes);
    // 設定のファイルに書いて読み直すと、開いたまま始まる
    let dir = std::env::temp_dir().join(format!("yolu-selmodes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("settings.conf");
    yolu_app::settings::save(&path, &st(&h).settings()).unwrap();
    let (loaded, problems) = yolu_app::settings::load(&path);
    assert!(problems.is_empty() && loaded.selection_all_modes);
    let mut h2 = app(1000.0, 640.0, 256);
    h2.state_mut().state.load_settings(loaded);
    pick_tool(&mut h2, Tool::SelectRect);
    assert_eq!(dock_modes(&h2), DOCK_MODES.to_vec());
    // 4 つの帯の「共通」で選べる。もう一度押すと畳む
    let isect = dock_rect(&h2, "共通");
    click(&mut h2, isect.center());
    assert_eq!(st(&h2).sel.combine, SelectionCombine::Intersect);
    assert!(dock_lit(&h2, "共通"));
    // 共通を選んでいるあいだは「⋯」は押せない（畳めない）。別の作成方法に替えてから畳む
    let more = dock_rect(&h2, MORE);
    click(&mut h2, more.center());
    assert!(st(&h2).settings().selection_all_modes);
    let new = dock_rect(&h2, "新規");
    click(&mut h2, new.center());
    let more = dock_rect(&h2, MORE);
    click(&mut h2, more.center());
    assert!(!st(&h2).settings().selection_all_modes);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn choosing_intersect_keeps_all_four_modes_shown_even_when_the_setting_is_folded() {
    let mut h = app(1000.0, 640.0, 256);
    pick_tool(&mut h, Tool::SelectRect);
    click_bar(&mut h, ISECT);
    assert_eq!(st(&h).sel.combine, SelectionCombine::Intersect);
    assert!(!st(&h).settings().selection_all_modes, "設定は畳むのまま");
    assert_eq!(
        dock_modes(&h),
        DOCK_MODES.to_vec(),
        "選んでいる共通は隠さない"
    );
    assert!(dock_lit(&h, "共通") && dock_lit(&h, MORE));
    // 点いたまま押せない: 押しても設定は変わらず、ツールチップに理由が出る
    let target = dock_rect(&h, MORE);
    assert!(dock_disabled(&h, MORE));
    click(&mut h, target.center());
    assert!(!st(&h).settings().selection_all_modes, "押しても変わらない");
    assert_eq!(dock_modes(&h), DOCK_MODES.to_vec());
    // 押した直後はツールチップが隠れるので、いったん離れてから乗せ直す
    move_to(&h, egui::pos2(2.0, 2.0));
    h.run();
    hover_and_wait(&mut h, target.center());
    assert!(
        h.query_by_label("すべての作成方法（共通を選んでいる間）")
            .is_some(),
        "押せない理由"
    );
    move_to(&h, egui::pos2(2.0, 2.0));
    h.run();
    // 別の作成方法へ替えれば、畳む設定の 3 つに戻る
    click_bar(&mut h, NEW);
    assert_eq!(dock_modes(&h), vec!["新規", "追加", "削除"]);
    assert!(!dock_lit(&h, MORE));
    assert!(!dock_disabled(&h, MORE), "共通以外なら押せる");
}

/// 左のドックを狭くしたウィンドウ（ツールプロパティの幅が 170 点ほど）。
fn narrow_props_app(lang: Lang) -> H {
    use egui_dock::{DockState, NodeIndex};
    let mut h = app(1000.0, 640.0, 256);
    h.state_mut().state.lang = lang;
    let mut dock = DockState::new(vec![yolu_app::Tab::Canvas]);
    let surface = dock.main_surface_mut();
    let [_, left] = surface.split_left(NodeIndex::root(), 0.17, vec![yolu_app::Tab::SubTools]);
    surface.split_below(left, 0.3, vec![yolu_app::Tab::ToolProperties]);
    h.state_mut().dock = dock;
    h.state_mut().state.sel.animate = false;
    pick_tool(&mut h, Tool::SelectRect);
    h
}

/// 今の画面に描かれている文字（ボタンの名前など）と、その矩形。
fn drawn_texts(h: &H) -> Vec<(String, egui::Rect)> {
    fn walk(shape: &egui::epaint::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            egui::epaint::Shape::Text(text) => out.push((
                text.galley.job.text.clone(),
                egui::Rect::from_min_size(text.pos, text.galley.size()),
            )),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        walk(&shape.shape, &mut out);
    }
    out
}

/// 帯のボタン（名前 `label`）の中に、その名前の文字が描かれているか（名前を外してアイコンだけにしていれば偽）。
fn name_drawn(h: &H, label: &str) -> bool {
    let button = dock_rect(h, label);
    drawn_texts(h)
        .iter()
        .any(|(text, bounds)| text == label && button.contains_rect(*bounds))
}

#[test]
fn the_names_in_the_bands_are_drawn_only_when_every_button_has_room_for_them() {
    // 既定の幅（約 250 点）
    for (lang, names, shown) in [
        (Lang::Ja, ["新規", "追加", "削除"], true),
        // 英語の "Subtract" は 3 つの帯に入らない: 全部アイコンだけ
        (Lang::En, ["New", "Add", "Subtract"], false),
    ] {
        let mut h = app(1280.0, 800.0, 256);
        h.state_mut().state.lang = lang;
        pick_tool(&mut h, Tool::SelectRect);
        for name in names {
            assert_eq!(name_drawn(&h, name), shown, "{lang:?} {name}");
        }
    }
    // 日本語は 4 つ開いても入る
    let mut h = app(1280.0, 800.0, 256);
    pick_tool(&mut h, Tool::SelectRect);
    let more = dock_rect(&h, MORE);
    click(&mut h, more.center());
    for name in DOCK_MODES {
        assert!(name_drawn(&h, name), "4 つ開いた {name}");
    }
    // 選択ペンの帯は、英語も短い名前（Pen・Eraser）が入る。ツールチップは名前のまま
    for (lang, names) in [
        (Lang::Ja, ["選択ペン", "選択消し"]),
        (Lang::En, ["Pen", "Eraser"]),
    ] {
        let mut h = app(1280.0, 800.0, 256);
        h.state_mut().state.lang = lang;
        pick_tool(&mut h, Tool::SelectPen);
        for name in names {
            assert!(name_drawn(&h, name), "{lang:?} {name}");
        }
    }
    // 狭い幅（約 150 点）: 作成方法は日本語も英語もアイコンだけ、選択ペンの英語は名前が入る
    for lang in Lang::ALL {
        let mut h = narrow_props_app(lang);
        let names = lang.pick(["新規", "追加", "削除"], ["New", "Add", "Subtract"]);
        for name in names {
            assert!(!name_drawn(&h, name), "{lang:?} 狭い {name}");
        }
        pick_tool(&mut h, Tool::SelectPen);
        let pen = lang.pick(["選択ペン", "選択消し"], ["Pen", "Eraser"]);
        let drawn: Vec<bool> = pen.iter().map(|n| name_drawn(&h, n)).collect();
        assert_eq!(drawn[0], drawn[1], "{lang:?}: 名前は全部出すか全部外す");
        if lang == Lang::En {
            assert!(drawn[0], "英語の Pen・Eraser は狭くても入る");
        }
    }
}

#[test]
fn the_creation_row_stays_one_full_width_row_and_drops_the_names_when_narrow() {
    for lang in Lang::ALL {
        let mut h = narrow_props_app(lang);
        let names = lang.pick(["新規", "追加", "削除"], ["New", "Add", "Subtract"]);
        let more = lang.pick("すべての作成方法", "All modes");
        let rects: Vec<egui::Rect> = names
            .into_iter()
            .chain([more])
            .map(|l| dock_rect(&h, l))
            .collect();
        let check = |rects: &[egui::Rect], why: &str| {
            // 1 行（同じ高さ 24・同じ上端）で、左から並び、重ならない
            for r in rects {
                assert_eq!(r.height(), 24.0, "{lang:?} {why}");
                assert_eq!(r.top(), rects[0].top(), "{lang:?} {why}: 折り返さない");
            }
            for pair in rects.windows(2) {
                assert!(pair[0].right() <= pair[1].left(), "{lang:?} {why}");
            }
            // 名前のボタンは同じ幅、「⋯」は細い 28
            let widths: Vec<f32> = rects[..rects.len() - 1].iter().map(|r| r.width()).collect();
            assert!(
                widths.iter().all(|w| (w - widths[0]).abs() < 0.5),
                "{lang:?} {why}: {widths:?}"
            );
            assert_eq!(rects.last().unwrap().width(), 28.0, "{lang:?} {why}");
        };
        check(&rects, "狭い");
        // 幅いっぱい: 右端が「⋯」の右端で、左端は帯の左端（パネルの余白）
        let props_right = rects.last().unwrap().right();
        assert!(props_right > rects[0].left() + 100.0);
        // 4 つ開いても 1 行
        let open = dock_rect(&h, more);
        click(&mut h, open.center());
        let rects: Vec<egui::Rect> = lang
            .pick(
                ["新規", "追加", "削除", "共通"],
                ["New", "Add", "Subtract", "Intersect"],
            )
            .into_iter()
            .chain([more])
            .map(|l| dock_rect(&h, l))
            .collect();
        check(&rects, "4 つ");
        assert_eq!(rects.last().unwrap().right(), props_right, "右端は同じ");
    }
}

#[test]
fn the_selection_pen_shows_the_pen_and_eraser_pair_instead_of_the_creation_modes() {
    let mut h = app(1000.0, 640.0, 256);
    pick_tool(&mut h, Tool::SelectPen);
    // オプションバーはアイコン（名前とキーのツールチップ）、ツールプロパティは幅いっぱいの 2 つのボタン（「⋯」は無い）
    let (pen, eraser) = ("選択ペン（Shift）", "選択消し（Ctrl）");
    let (dock_pen, dock_eraser) = ("選択ペン", "選択消し");
    assert!(h.query_all_by_label(NEW).next().is_none());
    assert!(h.query_all_by_label(MORE).next().is_none());
    assert!(lit(&h, pen) && !lit(&h, eraser));
    assert!(dock_lit(&h, dock_pen) && !dock_lit(&h, dock_eraser));
    let (a, b) = (dock_rect(&h, dock_pen), dock_rect(&h, dock_eraser));
    assert_eq!((a.height(), a.top()), (24.0, b.top()), "1 行");
    assert!(a.right() <= b.left() && (a.width() - b.width()).abs() < 0.5);
    click_bar(&mut h, eraser);
    assert!(st(&h).sel.pen_erase);
    assert!(lit(&h, eraser) && !lit(&h, pen));
    assert!(dock_lit(&h, dock_eraser) && !dock_lit(&h, dock_pen));
    // 帯のボタンでも替えられる
    let at = dock_rect(&h, dock_pen).center();
    click(&mut h, at);
    assert!(!st(&h).sel.pen_erase);
    let at = dock_rect(&h, dock_eraser).center();
    click(&mut h, at);
    assert!(st(&h).sel.pen_erase);
    // 押しているあいだ替わる: Shift は選択ペン（選んでいる選択消しは枠だけ）
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    h.step();
    h.run();
    assert!(lit(&h, pen) && !lit(&h, eraser));
    assert!(dock_lit(&h, dock_pen) && !dock_lit(&h, dock_eraser));
    assert!(st(&h).sel.pen_erase, "選んでいる値は変わらない");
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.step();
    h.run();
    assert!(lit(&h, eraser));
}

// ───────── 選択のツールの設定 ─────────

#[test]
fn headless_anti_alias_off_makes_the_edge_all_or_nothing() {
    let edge_values = |anti_alias: bool| -> Vec<u8> {
        let mut s = AppState::new(64, 64);
        s.sel.antialias = anti_alias;
        run(
            &mut s,
            SelEdit::Ellipse {
                cx: 31.3,
                cy: 30.7,
                rx: 20.4,
                ry: 13.1,
                mode: SelectionCombine::Replace,
            },
        );
        let mut v: Vec<u8> = s
            .doc
            .selection()
            .unwrap()
            .to_canvas_bytes()
            .into_iter()
            .filter(|a| *a != 0 && *a != 255)
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    assert!(
        !edge_values(true).is_empty(),
        "滑らかな縁には途中の量がある"
    );
    assert!(edge_values(false).is_empty(), "切ると 0 か 255 だけ");
    // なげなわ・多角形も同じ
    let mut s = AppState::new(64, 64);
    s.sel.antialias = false;
    run(
        &mut s,
        SelEdit::Polygon {
            points: vec![(5.5, 5.5), (50.3, 10.2), (30.1, 58.9)],
            mode: SelectionCombine::Replace,
        },
    );
    assert!(s
        .doc
        .selection()
        .unwrap()
        .to_canvas_bytes()
        .iter()
        .all(|a| *a == 0 || *a == 255));
}

#[test]
fn a_fixed_ratio_makes_a_square_and_the_setting_works_without_a_modifier_key() {
    let mut h = app(1000.0, 640.0, 512);
    pick_tool(&mut h, Tool::SelectRect);
    h.state_mut().state.sel.animate = false;
    drag_by(&mut h, &[(-100.0, -60.0), (60.0, 20.0)]);
    let (x0, y0, x1, y1) = bounds(st(&h).doc.selection().unwrap()).unwrap();
    assert!(
        (x1 - x0).abs_diff(y1 - y0) > 20,
        "設定が無ければ長方形のまま"
    );
    undo(&mut h);
    h.state_mut().state.sel.fixed_ratio = true;
    drag_by(&mut h, &[(-100.0, -60.0), (60.0, 20.0)]);
    let (x0, y0, x1, y1) = bounds(st(&h).doc.selection().unwrap()).unwrap();
    assert!((x1 - x0).abs_diff(y1 - y0) <= 1, "{x0},{y0} {x1},{y1}");
    // 楕円も正円
    undo(&mut h);
    pick_tool(&mut h, Tool::SelectEllipse);
    drag_by(&mut h, &[(-100.0, -60.0), (60.0, 20.0)]);
    let (x0, y0, x1, y1) = bounds(st(&h).doc.selection().unwrap()).unwrap();
    assert!((x1 - x0).abs_diff(y1 - y0) <= 2, "{x0},{y0} {x1},{y1}");
}

#[test]
fn shift_pressed_after_the_press_holds_the_ratio_but_shift_from_the_press_still_adds() {
    let mut h = app(1000.0, 640.0, 512);
    pick_tool(&mut h, Tool::SelectRect);
    h.state_mut().state.sel.animate = false;
    // 左に 1 つ作っておく
    drag_by(&mut h, &[(-200.0, -60.0), (-120.0, 0.0)]);
    let left = st(&h).doc.selection().unwrap().clone();
    // 押し始めは修飾なし、離すときに Shift: 縦横比の固定（追加にはならず、新規で置き換わる）
    let (p, q) = (at(&h, -50.0, -60.0), at(&h, 80.0, -20.0));
    drag_with(&mut h, &[p, q], Modifiers::NONE, Modifiers::SHIFT);
    let now = st(&h).doc.selection().unwrap().clone();
    let (x0, y0, x1, y1) = bounds(&now).unwrap();
    assert!((x1 - x0).abs_diff(y1 - y0) <= 1, "正方形");
    assert_eq!(now.amount(0, 0), 0);
    assert!(
        left.to_canvas_bytes()
            .iter()
            .zip(now.to_canvas_bytes())
            .all(|(l, n)| *l == 0 || n == 0),
        "前の選択範囲は残らない（追加ではない）"
    );
    // 押し始めから Shift: 追加（縦横比は固定しない）
    undo(&mut h);
    let (p, q) = (at(&h, -50.0, -60.0), at(&h, 80.0, -20.0));
    drag_with(&mut h, &[p, q], Modifiers::SHIFT, Modifiers::SHIFT);
    let both = st(&h).doc.selection().unwrap().clone();
    assert!(left
        .to_canvas_bytes()
        .iter()
        .zip(both.to_canvas_bytes())
        .all(|(l, b)| *l == 0 || b == 255));
    assert!(both.amount(0, 0) == 0);
}

#[test]
fn from_the_center_grows_around_the_press_with_the_setting_or_with_alt() {
    let mut h = app(1000.0, 640.0, 512);
    pick_tool(&mut h, Tool::SelectRect);
    h.state_mut().state.sel.animate = false;
    let start = at(&h, 0.0, 0.0);
    let (sx, sy) = to_canvas(&h, start);
    let centered = |h: &H| -> (f64, f64) {
        let (x0, y0, x1, y1) = bounds(st(h).doc.selection().unwrap()).unwrap();
        ((x0 + x1) as f64 / 2.0, (y0 + y1) as f64 / 2.0)
    };
    // 設定が無ければ、押した点は角
    drag_by(&mut h, &[(0.0, 0.0), (80.0, 50.0)]);
    let (cx, cy) = centered(&h);
    assert!(
        (cx - sx).abs() > 15.0 && (cy - sy).abs() > 8.0,
        "角だった: {cx},{cy} / {sx},{sy}"
    );
    undo(&mut h);
    // 設定: 中心から
    h.state_mut().state.sel.from_center = true;
    drag_by(&mut h, &[(0.0, 0.0), (80.0, 50.0)]);
    let (cx, cy) = centered(&h);
    assert!(
        (cx - sx).abs() <= 1.0 && (cy - sy).abs() <= 1.0,
        "{cx},{cy} / {sx},{sy}"
    );
    undo(&mut h);
    // 設定なしでも、押したあとに Alt を押していれば中心から（押しの始めの Alt は表示を回す組み合わせなので、始めには持たない）
    h.state_mut().state.sel.from_center = false;
    let (p, q) = (at(&h, 0.0, 0.0), at(&h, 80.0, 50.0));
    drag_with(&mut h, &[p, q], Modifiers::NONE, Modifiers::ALT);
    let (cx, cy) = centered(&h);
    assert!(
        (cx - sx).abs() <= 1.0 && (cy - sy).abs() <= 1.0,
        "Alt: {cx},{cy}"
    );
}

#[test]
fn a_rounded_rectangle_cuts_the_corners_and_undo_removes_it_in_one_step() {
    let mut h = app(1000.0, 640.0, 512);
    pick_tool(&mut h, Tool::SelectRect);
    h.state_mut().state.sel.animate = false;
    h.state_mut().state.sel.corner_radius = 40;
    let before = steps(&h);
    drag_by(&mut h, &[(-100.0, -80.0), (100.0, 80.0)]);
    assert_eq!(steps(&h), before + 1);
    let m = st(&h).doc.selection().unwrap().clone();
    let (x0, y0, x1, y1) = bounds(&m).unwrap();
    assert_eq!(m.amount((x0 + x1) / 2, (y0 + y1) / 2), 255, "中は選ばれる");
    assert_eq!(m.amount(x0, y0), 0, "角の画素は外れる");
    assert_eq!(m.amount(x1 - 1, y1 - 1), 0);
    assert_eq!(m.amount((x0 + x1) / 2, y0), 255, "辺の途中は選ばれる");
    assert!(
        m.to_canvas_bytes().iter().any(|a| *a != 0 && *a != 255),
        "縁は滑らか"
    );
    undo(&mut h);
    assert!(st(&h).doc.selection().is_none());
    // アンチエイリアスを切ると縁は 0 か 255 だけ
    h.state_mut().state.sel.antialias = false;
    drag_by(&mut h, &[(-100.0, -80.0), (100.0, 80.0)]);
    let m = st(&h).doc.selection().unwrap();
    assert!(m.to_canvas_bytes().iter().all(|a| *a == 0 || *a == 255));
    assert_eq!(m.amount(bounds(m).unwrap().0, bounds(m).unwrap().1), 0);
    // 角の丸めが 0 なら、これまでどおりの長方形
    undo(&mut h);
    h.state_mut().state.sel.corner_radius = 0;
    drag_by(&mut h, &[(-100.0, -80.0), (100.0, 80.0)]);
    let m = st(&h).doc.selection().unwrap();
    let (x0, y0, ..) = bounds(m).unwrap();
    assert_eq!(m.amount(x0, y0), 255);
}

#[test]
fn the_selection_properties_show_the_tool_settings_in_both_languages() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 800.0, 256);
        h.state_mut().state.lang = lang;
        pick_tool(&mut h, Tool::SelectRect);
        for label in lang.pick(
            ["アンチエイリアス", "縦横比を固定", "中心から", "角の丸め"],
            ["Anti-alias", "Fixed ratio", "From center", "Corner radius"],
        ) {
            h.get_by_label(label);
        }
        // 楕円には角の丸めが無い
        pick_tool(&mut h, Tool::SelectEllipse);
        assert!(h
            .query_by_label(lang.pick("角の丸め", "Corner radius"))
            .is_none());
        h.get_by_label(lang.pick("縦横比を固定", "Fixed ratio"));
        // なげなわは縁の滑らかさだけ
        pick_tool(&mut h, Tool::Lasso);
        h.get_by_label(lang.pick("アンチエイリアス", "Anti-alias"));
        assert!(h
            .query_by_label(lang.pick("縦横比を固定", "Fixed ratio"))
            .is_none());
    }
}

#[test]
fn the_tool_setting_toggles_change_the_state_from_the_properties_panel() {
    let mut h = app(1280.0, 800.0, 256);
    pick_tool(&mut h, Tool::SelectRect);
    h.state_mut().state.sel.corner_radius = 8;
    h.run();
    for (label, read) in [
        (
            "縦横比を固定",
            (|s: &AppState| s.sel.fixed_ratio) as fn(&AppState) -> bool,
        ),
        ("中心から", |s| s.sel.from_center),
    ] {
        assert!(!read(st(&h)));
        h.get_by_label(label).click();
        h.run();
        assert!(read(st(&h)), "{label}");
    }
    assert!(st(&h).sel.antialias);
    h.get_by_label("アンチエイリアス").click();
    h.run();
    assert!(!st(&h).sel.antialias);
}

// ───────── 選択ペン・選択消し ─────────

#[test]
fn the_selection_pen_adds_a_hard_stroke_as_one_undo_and_redo_returns_it() {
    let mut h = app(1000.0, 640.0, 512);
    pick_tool(&mut h, Tool::SelectPen);
    h.state_mut().state.sel.animate = false;
    set_brush(&mut h, 20.0, 1.0, 1.0);
    let before = steps(&h);
    drag_by(
        &mut h,
        &[(-100.0, 0.0), (-40.0, 0.0), (40.0, 0.0), (100.0, 0.0)],
    );
    assert_eq!(steps(&h), before + 1, "1 ストロークが 1 回の Undo");
    assert_eq!(amount_at(&h, at(&h, 0.0, 0.0)), 255, "線の上");
    assert_eq!(amount_at(&h, at(&h, -100.0, 0.0)), 255, "始めの点");
    assert_eq!(amount_at(&h, at(&h, 100.0, 0.0)), 255, "終わりの点");
    assert_eq!(amount_at(&h, at(&h, 0.0, 60.0)), 0, "線から離れた所");
    assert_eq!(amount_at(&h, at(&h, 160.0, 0.0)), 0, "終わりの先");
    assert!(st(&h).modified);
    assert!(
        st(&h).canvas.stroke.is_none() && st(&h).sel.pen.is_none() && st(&h).sel.drag.is_none()
    );
    undo(&mut h);
    assert!(st(&h).doc.selection().is_none());
    redo(&mut h);
    assert_eq!(amount_at(&h, at(&h, 0.0, 0.0)), 255);
    // 画素は描かない
    assert_eq!(canvas_pixel(&h, at(&h, 0.0, 0.0))[3], 0);
}

#[test]
fn the_selection_pen_follows_the_brush_diameter_hardness_and_opacity() {
    let mut h = app(1000.0, 640.0, 512);
    pick_tool(&mut h, Tool::SelectPen);
    h.state_mut().state.sel.animate = false;
    // 直径: 大きいほど太い
    let width_of = |h: &H| -> u32 {
        let (x0, _, x1, _) = bounds(st(h).doc.selection().unwrap()).unwrap();
        x1 - x0
    };
    set_brush(&mut h, 20.0, 1.0, 1.0);
    click_by(&mut h, 0.0, 0.0);
    let thin = width_of(&h);
    undo(&mut h);
    set_brush(&mut h, 60.0, 1.0, 1.0);
    click_by(&mut h, 0.0, 0.0);
    let thick = width_of(&h);
    undo(&mut h);
    assert!(thick > thin * 2, "{thin} {thick}");
    // 硬さ 0: 中心から縁へなめらかに減る（単調）
    set_brush(&mut h, 200.0, 0.0, 1.0);
    click_by(&mut h, 0.0, 0.0);
    let center = to_canvas(&h, at(&h, 0.0, 0.0));
    let (cx, cy) = (center.0 as u32, center.1 as u32);
    let m = st(&h).doc.selection().unwrap().clone();
    let (x0, _, x1, _) = bounds(&m).unwrap();
    let radius = (x1 - x0) / 2;
    let ramp: Vec<u8> = (0..radius.saturating_sub(2))
        .step_by(4)
        .map(|d| m.amount(cx + d, cy))
        .collect();
    assert!(ramp.windows(2).all(|w| w[0] >= w[1]), "{ramp:?}");
    assert!(ramp[0] >= 250 && *ramp.last().unwrap() < 80, "{ramp:?}");
    undo(&mut h);
    // 不透明度 50%: 最大の量が半分
    set_brush(&mut h, 40.0, 1.0, 0.5);
    click_by(&mut h, 0.0, 0.0);
    assert!((amount_at(&h, at(&h, 0.0, 0.0)) as i32 - 128).abs() <= 1);
}

#[test]
fn the_selection_pen_adds_to_the_current_selection_and_the_eraser_removes_from_it() {
    let mut h = app(1000.0, 640.0, 512);
    pick_tool(&mut h, Tool::SelectRect);
    h.state_mut().state.sel.animate = false;
    drag_by(&mut h, &[(-150.0, -60.0), (-40.0, 60.0)]);
    let base = st(&h).doc.selection().unwrap().clone();
    pick_tool(&mut h, Tool::SelectPen);
    set_brush(&mut h, 20.0, 1.0, 1.0);
    // 足す: 元の選択範囲は残り、右に伸びる
    drag_by(&mut h, &[(-100.0, 0.0), (120.0, 0.0)]);
    assert_eq!(amount_at(&h, at(&h, 100.0, 0.0)), 255);
    assert_eq!(
        amount_at(&h, at(&h, -100.0, 40.0)),
        255,
        "元の選択範囲は残る"
    );
    // 消す: 選択消しに替える
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Ui(SelUiOp::PenErase(true))));
    let before = steps(&h);
    drag_by(&mut h, &[(-120.0, 20.0), (-60.0, 20.0)]);
    assert_eq!(steps(&h), before + 1);
    assert_eq!(amount_at(&h, at(&h, -90.0, 20.0)), 0, "なぞった所は外れる");
    assert_eq!(
        amount_at(&h, at(&h, -90.0, -40.0)),
        255,
        "なぞっていない所は残る"
    );
    undo(&mut h);
    assert_eq!(amount_at(&h, at(&h, -90.0, 20.0)), 255);
    undo(&mut h);
    assert_eq!(st(&h).doc.selection(), Some(&base));
}

#[test]
fn ctrl_swaps_the_selection_pen_to_the_eraser_and_shift_back_to_the_pen() {
    let mut h = app(1000.0, 640.0, 512);
    pick_tool(&mut h, Tool::SelectRect);
    h.state_mut().state.sel.animate = false;
    drag_by(&mut h, &[(-150.0, -90.0), (150.0, -10.0)]);
    pick_tool(&mut h, Tool::SelectPen);
    set_brush(&mut h, 20.0, 1.0, 1.0);
    // 選択ペンのまま Ctrl を押して始める: 消す
    let pts = [at(&h, -80.0, -50.0), at(&h, 0.0, -50.0)];
    drag_with(&mut h, &pts, Modifiers::COMMAND, Modifiers::COMMAND);
    assert_eq!(amount_at(&h, at(&h, -40.0, -50.0)), 0);
    assert!(!st(&h).sel.pen_erase, "選んでいる切り替えは変わらない");
    undo(&mut h);
    // 選択消しにして Shift を押して始める: 足す（選択範囲の外に描く）
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Ui(SelUiOp::PenErase(true))));
    let pts = [at(&h, -80.0, 60.0), at(&h, 0.0, 60.0)];
    drag_with(&mut h, &pts, Modifiers::SHIFT, Modifiers::SHIFT);
    assert_eq!(amount_at(&h, at(&h, -40.0, 60.0)), 255);
}

#[test]
fn the_selection_pen_works_with_the_pen_pressure_and_the_eraser_end() {
    let mut h = app(1000.0, 640.0, 512);
    pick_tool(&mut h, Tool::SelectPen);
    h.state_mut().state.sel.animate = false;
    set_brush(&mut h, 60.0, 1.0, 1.0);
    h.state_mut().state.brush.pressure_size = true;
    let line = |h: &H, y: f32| -> Vec<Pos2> {
        (0..=8)
            .map(|i| at(h, -100.0 + 25.0 * i as f32, y))
            .collect()
    };
    let height_of = |h: &H| -> i32 {
        let (_, y0, _, y1) = bounds(st(h).doc.selection().unwrap()).unwrap();
        (y1 - y0) as i32
    };
    let full = line(&h, -40.0);
    pen_stroke(&mut h, &full, 1.0, false);
    let full_height = height_of(&h);
    assert_eq!(steps(&h), 1, "ペンの 1 ストロークも 1 回の Undo");
    undo(&mut h);
    // 筆圧 0.5 の線は、筆圧 1 の線の半分の太さ（直径を筆圧で変える設定）
    pen_stroke(&mut h, &full, 0.5, false);
    let half_height = height_of(&h);
    assert!(
        (full_height - 2 * half_height).abs() <= 4,
        "{full_height} {half_height}"
    );
    undo(&mut h);
    // 筆圧で変えない設定なら同じ太さ
    h.state_mut().state.brush.pressure_size = false;
    pen_stroke(&mut h, &full, 0.5, false);
    assert!((height_of(&h) - full_height).abs() <= 2);
    undo(&mut h);
    h.state_mut().state.brush.pressure_size = true;
    pen_stroke(&mut h, &full, 1.0, false);
    assert_eq!(steps(&h), 1);
    // ペンの消しゴムの端: 消す
    pen_stroke(&mut h, &full, 1.0, true);
    assert_eq!(amount_at(&h, at(&h, 0.0, -40.0)), 0);
    assert!(
        st(&h).doc.selection().is_none(),
        "消し切ると選択範囲が無くなる"
    );
    assert_eq!(steps(&h), 2);
    // 触れていないあいだは何も起きない
    h.state()
        .pen()
        .push(pen_at(at(&h, 0.0, 0.0), false, 0.0, false));
    h.step();
    assert!(st(&h).sel.pen.is_none());
}

#[test]
fn the_selection_pen_and_its_overlay_follow_a_rotated_and_flipped_view() {
    let mut h = app(1000.0, 640.0, 512);
    h.state_mut().state.sel.animate = false;
    h.state_mut().state.view.angle = 30.0;
    h.state_mut().state.view.flip = true;
    pick_tool(&mut h, Tool::SelectPen);
    set_brush(&mut h, 24.0, 1.0, 1.0);
    // 押したまま動かしている途中: 画面のなぞった所の下に被覆の重ねがある（絵を持つタイル、または 1 面の四角）
    let (a, b) = (at(&h, -80.0, -20.0), at(&h, 90.0, 40.0));
    h.event(Event::PointerMoved(a));
    h.event(Event::PointerButton {
        pos: a,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    move_to(&h, at(&h, 5.0, 10.0));
    h.step();
    move_to(&h, b);
    h.step();
    h.step();
    assert!(st(&h).sel.pen_overlay.texture_count() > 0);
    release(&h, b, PointerButton::Primary);
    h.run();
    // 回して反転した表示でも、なぞった線の上が選ばれ、離れた所は選ばれない
    for t in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
        let p = offset(a, (b.x - a.x) * t, (b.y - a.y) * t);
        assert_eq!(amount_at(&h, p), 255, "t={t}");
    }
    assert_eq!(amount_at(&h, offset(a, 0.0, 80.0)), 0);
}

#[test]
fn escape_and_focus_loss_do_not_leave_a_selection_pen_stroke_behind() {
    let mut h = app(1000.0, 640.0, 512);
    pick_tool(&mut h, Tool::SelectPen);
    h.state_mut().state.sel.animate = false;
    set_brush(&mut h, 30.0, 1.0, 1.0);
    let start = at(&h, -50.0, 0.0);
    h.event(Event::PointerMoved(start));
    h.event(Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    move_to(&h, at(&h, 50.0, 0.0));
    h.step();
    h.step();
    assert!(st(&h).sel.pen.is_some(), "描いている途中");
    assert!(st(&h).doc.selection().is_none(), "途中は文書を変えない");
    assert!(
        st(&h).sel.pen_overlay.texture_count() > 0,
        "途中の被覆を重ねて見せる"
    );
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    h.run();
    assert!(st(&h).sel.pen.is_none() && st(&h).sel.drag.is_none());
    release(&h, at(&h, 50.0, 0.0), PointerButton::Primary);
    h.run();
    assert!(
        st(&h).doc.selection().is_none(),
        "Esc で捨てたものは離しても選択範囲にならない"
    );
    assert_eq!(steps(&h), 0);
    // フォーカスを失ったときも取り残さない
    h.event(Event::PointerMoved(start));
    h.event(Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.event(Event::WindowFocused(false));
    h.step();
    h.run();
    assert!(st(&h).sel.pen.is_none() && st(&h).sel.drag.is_none());
    assert!(st(&h).doc.selection().is_none());
}

#[test]
fn headless_the_selection_pen_result_is_refused_while_drawing() {
    let mut s = AppState::new(64, 64);
    s.apply(Action::SelectTool(Tool::SelectPen));
    // 描いている間は選択範囲を変えられない
    s.canvas.stroke = Some(StrokeSource::Mouse);
    s.apply(Action::Sel(SelAction::Edit(SelEdit::Shape {
        mask: SelectionMask::all(&s.doc),
        mode: SelectionCombine::Add,
    })));
    assert!(s.doc.selection().is_none());
    s.canvas.stroke = None;
}

#[test]
fn headless_a_stroke_past_the_budget_is_refused_and_a_normal_stroke_is_not() {
    let doc = yolu_app::engine::Document::new(512, 512).unwrap();
    let params = PenParams {
        radius: 30.0,
        hardness: 1.0,
        opacity: 1.0,
        pressure_size: false,
        pressure_opacity: false,
    };
    // 予算が小さいと、タイルが増えたところで断る
    let tile_bytes = (doc.tile_size() as u64).pow(2) * 3;
    let mut tiny = PenStroke::new(&doc, false, None, params, tile_bytes * 2).unwrap();
    let mut refused = false;
    for i in 0..400 {
        if tiny.add(5.0 + i as f64, 250.0, 1.0) == Err(PenError::TooLarge) {
            refused = true;
            break;
        }
    }
    assert!(refused, "触れたタイルが予算を超えたら断る");
    assert!(tiny.tile_count() <= 2);
    // 既定の予算なら、512² の全面をなぞっても通る
    let mut full = PenStroke::new(
        &doc,
        false,
        None,
        params,
        yolu_app::selection::pen::PEN_BUDGET_BYTES,
    )
    .unwrap();
    for y in (0..512).step_by(20) {
        for x in 0..512 {
            full.add(x as f64, y as f64, 1.0).unwrap();
        }
    }
    let (cover, mode) = full.finish().unwrap();
    assert_eq!(mode, SelectionCombine::Add);
    assert_eq!(cover.amount(250, 250), 255);
}

#[test]
fn headless_the_dab_profile_matches_the_brush_edge_and_the_preview_matches_the_combined_result() {
    // 硬さまでは満量、縁で 0、間は smoothstep（core のブラシの縁と同じ式）
    assert_eq!(dab_coverage(0.0, 0.5), 1.0);
    assert_eq!(dab_coverage(0.5, 0.5), 1.0);
    assert_eq!(dab_coverage(1.0, 0.5), 0.0);
    assert!((dab_coverage(0.75, 0.5) - 0.5).abs() < 1e-12);
    assert_eq!(dab_coverage(1.2, 0.5), 0.0);
    assert_eq!(dab_coverage(0.9999, 1.0), 1.0, "硬さ 1 は縁まで満量");
    // クイックマスクの見た目（始めの選択範囲に被覆を重ねたもの）は、文書で組み合わせた結果と同じ
    let mut s = AppState::new(256, 256);
    run(&mut s, rect(SelectionCombine::Replace, 20, 20, 120, 120));
    let base = s.doc.selection().unwrap().clone();
    for erase in [false, true] {
        let params = PenParams {
            radius: 25.0,
            hardness: 0.3,
            opacity: 0.8,
            pressure_size: false,
            pressure_opacity: false,
        };
        let mut stroke =
            PenStroke::new(&s.doc, erase, Some(base.clone()), params, 1 << 28).unwrap();
        for i in 0..40 {
            stroke
                .add(
                    30.0 + i as f64 * 4.0,
                    60.0 + (i as f64 * 0.3).sin() * 30.0,
                    1.0,
                )
                .unwrap();
        }
        stroke.sync().unwrap();
        let preview = stroke.preview().unwrap().clone();
        let (cover, mode) = stroke.finish().unwrap();
        assert_eq!(
            preview,
            base.combine(&cover, mode).unwrap(),
            "erase={erase}"
        );
    }
}

// ───────── 境界をぼかす ─────────

#[test]
fn the_button_bar_feather_button_asks_for_the_radius_and_applies_it_as_one_undo() {
    let mut h = app(1280.0, 800.0, 256);
    pick_tool(&mut h, Tool::SelectRect);
    h.state_mut().state.sel.animate = false;
    drag_by(&mut h, &[(-60.0, -40.0), (60.0, 40.0)]);
    let original = st(&h).doc.selection().unwrap().clone();
    h.get_by_label("境界をぼかす…").click();
    h.run();
    assert_eq!(st(&h).sel.dialog.map(|d| d.kind), Some(ModifyKind::Feather));
    h.state_mut().state.sel.dialog.as_mut().unwrap().radius = 7;
    let before = steps(&h);
    h.get_by_label("OK").click();
    h.run();
    assert_eq!(steps(&h), before + 1);
    assert_eq!(
        st(&h).doc.selection(),
        Some(
            &original
                .feather(7.0, false, DEFAULT_WORKING_BUDGET_BYTES)
                .unwrap()
        )
    );
    undo(&mut h);
    assert_eq!(st(&h).doc.selection(), Some(&original));
}

#[test]
fn headless_feather_over_the_working_budget_is_refused_and_changes_nothing() {
    let mut s = AppState::new(8192, 8192);
    run(&mut s, SelEdit::All);
    let before = s.doc.undo_count();
    run(
        &mut s,
        SelEdit::Modify {
            kind: ModifyKind::Feather,
            radius: 200,
            edge_lock: false,
        },
    );
    assert_eq!(s.doc.undo_count(), before, "断られたら段を積まない");
    assert!(!s.message.is_empty());
    assert_eq!(s.doc.selection(), Some(&SelectionMask::all(&s.doc)));
}

#[test]
fn the_english_select_menu_has_feather_quick_mask_and_saved_selections() {
    let mut h = app(1000.0, 640.0, 256);
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(Lang::En)));
    run(&mut h.state_mut().state, SelEdit::All);
    h.run();
    let title = menu_title(&h, "Select").center();
    click(&mut h, title);
    for label in [
        "Feather…",
        "Quick Mask",
        "Remembered Selections…",
        "Selection Pen",
    ] {
        popup_item(&h, label);
    }
}

// ───────── 保存と読み込み ─────────

fn save(s: &mut AppState, name: &str) {
    s.apply(Action::Sel(SelAction::Saved(SavedOp::Save(name.into()))));
}

fn recall(s: &mut AppState, index: usize, mode: SelectionCombine) {
    run(s, SelEdit::Recall { index, mode });
}

#[test]
fn headless_saved_selections_recall_with_all_four_modes_each_as_one_undo() {
    let mut s = AppState::new(128, 128);
    run(&mut s, rect(SelectionCombine::Replace, 10, 10, 70, 70));
    save(&mut s, "A");
    assert_eq!(s.saved_selections().len(), 1);
    assert_eq!(s.saved_selections()[0].name, "A");
    // 別の選択範囲を作って、4 つの作成方法で呼び出す
    run(&mut s, rect(SelectionCombine::Replace, 40, 40, 110, 110));
    let current = s.doc.selection().unwrap().clone();
    let saved = SelectionMask::rectangle(&s.doc, 10, 10, 70, 70);
    for mode in [
        SelectionCombine::Replace,
        SelectionCombine::Add,
        SelectionCombine::Subtract,
        SelectionCombine::Intersect,
    ] {
        let steps = s.doc.undo_count();
        recall(&mut s, 0, mode);
        assert_eq!(s.doc.undo_count(), steps + 1, "{mode:?}");
        let want = match mode {
            SelectionCombine::Replace => saved.clone(),
            m => current.combine(&saved, m).unwrap(),
        };
        assert_eq!(s.doc.selection(), Some(&want), "{mode:?}");
        s.apply(Action::Undo);
        assert_eq!(s.doc.selection(), Some(&current), "{mode:?} の Undo");
    }
    // 選択範囲が無い状態へ「削除」で呼ぶと、何も選ばれないまま（段も積まない）
    s.apply(Action::Sel(SelAction::Edit(SelEdit::Clear)));
    let steps = s.doc.undo_count();
    recall(&mut s, 0, SelectionCombine::Subtract);
    assert_eq!(s.doc.undo_count(), steps);
    // 新規なら呼び出せる
    recall(&mut s, 0, SelectionCombine::Replace);
    assert_eq!(s.doc.selection(), Some(&saved));
}

#[test]
fn headless_saving_needs_a_selection_replaces_a_same_name_and_has_a_limit() {
    let mut s = AppState::new(64, 64);
    save(&mut s, "A");
    assert!(
        s.saved_selections().is_empty(),
        "選択範囲が無ければ覚えない"
    );
    assert!(!s.message.is_empty());
    run(&mut s, rect(SelectionCombine::Replace, 0, 0, 20, 20));
    save(&mut s, "  A  ");
    run(&mut s, rect(SelectionCombine::Replace, 0, 0, 40, 40));
    save(&mut s, "A");
    assert_eq!(s.saved_selections().len(), 1, "同じ名前は入れ替える");
    assert_eq!(s.saved_selections()[0].mask.amount(30, 30), 255);
    // 取り消すと、入れ替える前の選択範囲へ戻る（残すのは 1 回の Undo）
    s.apply(Action::Undo);
    assert_eq!(s.saved_selections()[0].mask.amount(30, 30), 0);
    assert_eq!(s.saved_selections()[0].mask.amount(10, 10), 255);
    s.apply(Action::Redo);
    assert_eq!(s.saved_selections()[0].mask.amount(30, 30), 255);
    // 名前が空なら既定の名前（選択範囲 N）
    save(&mut s, "   ");
    assert_eq!(s.saved_selections().len(), 2);
    assert!(s.saved_selections()[1].name.starts_with("選択範囲"));
    // 上限
    for i in 0..(MAX_SAVED + 5) {
        save(&mut s, &format!("n{i}"));
    }
    assert_eq!(s.saved_selections().len(), MAX_SAVED);
    // 消す
    s.apply(Action::Sel(SelAction::Saved(SavedOp::Delete(0))));
    assert_eq!(s.saved_selections().len(), MAX_SAVED - 1);
    assert_ne!(s.saved_selections()[0].name, "A");
    s.apply(Action::Sel(SelAction::Saved(SavedOp::Delete(999))));
    assert_eq!(
        s.saved_selections().len(),
        MAX_SAVED - 1,
        "範囲外は何もしない"
    );
}

#[test]
fn headless_a_saved_selection_follows_a_resize_of_the_document_and_the_undo_of_it() {
    let mut s = AppState::new(64, 64);
    run(&mut s, rect(SelectionCombine::Replace, 0, 0, 32, 32));
    save(&mut s, "A");
    s.doc
        .resize_image(32, 32, yolu_app::engine::CanvasResampling::Nearest)
        .unwrap();
    let saved = s.saved_selections()[0].clone();
    assert_eq!(
        (saved.mask.width(), saved.mask.height()),
        (32, 32),
        "文書の大きさに合わせて作り直す"
    );
    // 呼び出せる（大きさが合う）
    s.apply(Action::Sel(SelAction::Edit(SelEdit::Clear)));
    recall(&mut s, 0, SelectionCombine::Replace);
    assert_eq!(bounds(s.doc.selection().unwrap()), Some((0, 0, 16, 16)));
    // 大きさの変更を取り消すと、元の大きさの選択範囲へ戻る
    s.doc.undo().unwrap();
    s.doc.undo().unwrap();
    s.doc.undo().unwrap();
    assert_eq!(
        (s.doc.width(), s.saved_selections()[0].mask.width()),
        (64, 64)
    );
    // 範囲外の番号は断る
    let steps = s.doc.undo_count();
    recall(&mut s, 5, SelectionCombine::Replace);
    assert_eq!(s.doc.undo_count(), steps);
    assert!(!s.message.is_empty(), "無い番号は断る");
}

fn saved_names(s: &AppState) -> Vec<&str> {
    s.saved_selections()
        .iter()
        .map(|x| x.name.as_str())
        .collect()
}

#[test]
fn headless_saved_selections_belong_to_the_texture_set_and_each_save_is_one_undo_step_of_it() {
    let mut s = AppState::new(64, 64);
    run(&mut s, SelEdit::All);
    let steps = s.doc.undo_count();
    let revision = s.doc.revision();
    s.modified = false;
    save(&mut s, "A");
    assert_eq!(
        s.doc.undo_count(),
        steps + 1,
        "覚えるのは文書の 1 回の取り消し"
    );
    assert!(s.doc.revision() > revision);
    assert!(s.modified, "保存が要る変更として数える");
    // 描いている間は覚えない
    s.canvas.stroke = Some(StrokeSource::Mouse);
    save(&mut s, "B");
    s.canvas.stroke = None;
    assert_eq!(saved_names(&s), ["A"]);
    // 別のセットの一覧には出ない。そこで覚えたものは元のセットには出ず、戻ると見える
    let first = s.sets.current().uid;
    s.add_texture_set().unwrap();
    assert_ne!(s.sets.current().uid, first);
    assert!(
        s.saved_selections().is_empty(),
        "2 つ目のセットには何も無い"
    );
    assert_eq!(s.doc.undo_count(), 0, "取り消しの履歴もセットごと");
    run(&mut s, rect(SelectionCombine::Replace, 0, 0, 16, 16));
    save(&mut s, "B");
    assert_eq!(saved_names(&s), ["B"]);
    s.switch_set(0).unwrap();
    assert_eq!(saved_names(&s), ["A"], "1 つ目のセットには B が出ない");
    s.switch_set(1).unwrap();
    assert_eq!(saved_names(&s), ["B"], "戻ると見える");
    // 呼び出しは今のセットの札（番号 0 は、1 つ目のセットでは A・2 つ目では B）
    run(&mut s, SelEdit::Clear);
    recall(&mut s, 0, SelectionCombine::Replace);
    assert_eq!(bounds(s.doc.selection().unwrap()), Some((0, 0, 16, 16)));
    s.switch_set(0).unwrap();
    run(&mut s, SelEdit::Clear);
    recall(&mut s, 0, SelectionCombine::Replace);
    assert_eq!(bounds(s.doc.selection().unwrap()), Some((0, 0, 64, 64)));
}

#[test]
fn headless_saved_selections_go_with_their_set_and_are_not_carried_to_another_project() {
    let mut s = AppState::new(64, 64);
    run(&mut s, SelEdit::All);
    save(&mut s, "A");
    let first = s.sets.current().uid;
    s.add_texture_set().unwrap();
    let second = s.sets.current().uid;
    run(&mut s, rect(SelectionCombine::Replace, 0, 0, 16, 16));
    save(&mut s, "B");
    // 今のセットを消すと、そのセットの覚えた選択範囲も消える（今のセットは並びの前のセットに替わる）
    s.remove_sets(&[second]).unwrap();
    assert_eq!(s.sets.current().uid, first);
    assert_eq!(saved_names(&s), ["A"], "残ったセットのものは残る");
    // 今でないセットを消しても同じ
    s.add_texture_set().unwrap();
    let third = s.sets.current().uid;
    save(&mut s, "C");
    s.switch_set(0).unwrap();
    s.remove_sets(&[third]).unwrap();
    assert_eq!(saved_names(&s), ["A"]);
    // 別のプロジェクトを開く（セットの並びを丸ごと置き換える）と、前のセットのものは持ち越さない（文書が新しい）
    let doc = Document::new(64, 64).unwrap();
    let sets = TextureSets::first_in(&doc, s.lang);
    s.replace_sets(sets, doc);
    assert!(s.saved_selections().is_empty(), "新しい文書は何も持たない");
    // 消して空になっても、段は取り消せる
    run(&mut s, SelEdit::All);
    save(&mut s, "D");
    s.apply(Action::Sel(SelAction::Saved(SavedOp::Delete(0))));
    assert!(s.saved_selections().is_empty());
    s.apply(Action::Undo);
    assert_eq!(saved_names(&s), ["D"]);
}

#[test]
fn headless_renaming_deleting_and_saving_each_undo_and_redo_and_refuse_what_they_must() {
    let mut s = AppState::new(64, 64);
    s.lang = Lang::Ja;
    run(&mut s, rect(SelectionCombine::Replace, 0, 0, 20, 20));
    save(&mut s, "A");
    run(&mut s, rect(SelectionCombine::Replace, 30, 30, 50, 50));
    save(&mut s, "B");
    let rename = |s: &mut AppState, index: usize, name: &str| {
        s.apply(Action::Sel(SelAction::Saved(SavedOp::Rename {
            index,
            name: name.into(),
        })));
    };
    // 名前を変える: 1 回の取り消し（前後の空白は除く）
    let steps = s.doc.undo_count();
    s.modified = false;
    rename(&mut s, 0, "  前髪 ");
    assert_eq!(saved_names(&s), ["前髪", "B"]);
    assert_eq!(s.doc.undo_count(), steps + 1);
    assert!(s.modified);
    s.apply(Action::Undo);
    assert_eq!(saved_names(&s), ["A", "B"]);
    s.apply(Action::Redo);
    assert_eq!(saved_names(&s), ["前髪", "B"]);
    // 同じ名前・今と同じ名前・範囲外・長すぎる名前・空の名前は断り、段を積まない（理由を言う）
    let steps = s.doc.undo_count();
    rename(&mut s, 0, "B");
    assert_eq!(s.message, "同じ名前の選択範囲があります。");
    rename(&mut s, 0, "前髪");
    rename(&mut s, 9, "x");
    rename(&mut s, 0, &"あ".repeat(yolu_core::MAX_SAVED_NAME_CHARS + 1));
    assert!(s.message.contains("長すぎ"), "{}", s.message);
    rename(&mut s, 0, "   ");
    assert!(!s.message.is_empty());
    assert_eq!(s.doc.undo_count(), steps);
    assert_eq!(saved_names(&s), ["前髪", "B"]);
    s.lang = Lang::En;
    rename(&mut s, 0, "B");
    assert_eq!(s.message, "A saved selection with that name exists.");
    rename(&mut s, 0, &"x".repeat(yolu_core::MAX_SAVED_NAME_CHARS + 1));
    assert!(s.message.contains("too long"), "{}", s.message);
    // 描いている間は変えない
    s.canvas.stroke = Some(StrokeSource::Mouse);
    rename(&mut s, 0, "z");
    s.apply(Action::Sel(SelAction::Saved(SavedOp::Delete(0))));
    s.canvas.stroke = None;
    assert_eq!(saved_names(&s), ["前髪", "B"]);
    // 消す: 取り消せる。呼び戻しは文書の今のものを見る
    s.apply(Action::Sel(SelAction::Saved(SavedOp::Delete(1))));
    assert_eq!(saved_names(&s), ["前髪"]);
    s.apply(Action::Undo);
    assert_eq!(saved_names(&s), ["前髪", "B"]);
    recall(&mut s, 1, SelectionCombine::Replace);
    assert_eq!(bounds(s.doc.selection().unwrap()), Some((30, 30, 50, 50)));
}

#[test]
fn headless_a_read_only_set_refuses_to_change_the_saved_selections() {
    let mut s = AppState::new(64, 64);
    run(&mut s, SelEdit::All);
    save(&mut s, "A");
    let uid = s.sets.current().uid;
    let index = s.sets.index_of(uid).unwrap();
    s.sets.get_mut(index).unwrap().read_only = Some("試験".into());
    let steps = s.doc.undo_count();
    save(&mut s, "B");
    s.apply(Action::Sel(SelAction::Saved(SavedOp::Rename {
        index: 0,
        name: "z".into(),
    })));
    s.apply(Action::Sel(SelAction::Saved(SavedOp::Delete(0))));
    assert_eq!(saved_names(&s), ["A"]);
    assert_eq!(s.doc.undo_count(), steps);
    // ウィンドウを開く・閉じるは、読むだけのセットでもできる
    s.apply(Action::Sel(SelAction::Saved(SavedOp::OpenWindow)));
    assert!(s.sel.saved_window.is_some());
}

#[test]
fn headless_saved_selections_past_the_budget_are_refused_across_all_sets_and_change_nothing() {
    let mut s = AppState::new(64, 64);
    s.lang = Lang::Ja;
    run(&mut s, SelEdit::All);
    let one = s.doc.selection().unwrap().allocated_bytes();
    // 札 1 枚と半分の予算
    s.sel.saved_budget = one + one / 2;
    save(&mut s, "A");
    assert_eq!(saved_names(&s), ["A"]);
    save(&mut s, "B");
    assert_eq!(saved_names(&s), ["A"], "2 枚目は断る（一覧が変わらない）");
    assert_eq!(s.message, "覚えた選択範囲が大きすぎます。");
    // 同じ名前の入れ替えは、置き換わる分を数えないので通る
    save(&mut s, "A");
    assert_eq!(s.message, "選択範囲を覚えました: A");
    // 合計はプロジェクト全体: 別のセットでも、ほかのセットの分を引いた残りで見る
    s.add_texture_set().unwrap();
    run(&mut s, SelEdit::All);
    save(&mut s, "C");
    assert!(s.saved_selections().is_empty());
    assert_eq!(s.message, "覚えた選択範囲が大きすぎます。");
    // 予算を戻せば通る
    s.sel.saved_budget = SAVED_BUDGET_BYTES;
    save(&mut s, "C");
    assert_eq!(saved_names(&s), ["C"]);
}

#[test]
fn the_saved_selections_window_saves_lists_recalls_and_removes() {
    let mut h = app(1280.0, 800.0, 256);
    pick_tool(&mut h, Tool::SelectRect);
    h.state_mut().state.sel.animate = false;
    drag_by(&mut h, &[(-80.0, -50.0), (-10.0, 50.0)]);
    let title = menu_title(&h, "選択範囲").center();
    click(&mut h, title);
    let item = popup_item(&h, "覚えた選択範囲…").center();
    click(&mut h, item);
    assert!(st(&h).sel.saved_window.is_some());
    // ウィンドウはキャンバスの真ん中を覆うので、下へ寄せる
    h.state_mut()
        .state
        .sel
        .saved_window
        .as_mut()
        .unwrap()
        .offset = egui::vec2(0.0, 330.0);
    h.run();
    h.get_by_label("覚える").click();
    h.run();
    assert_eq!(st(&h).saved_selections().len(), 1);
    assert_eq!(st(&h).saved_selections()[0].name, "選択範囲 1");
    // 2 つ目は次の名前が入っている
    drag_by(&mut h, &[(20.0, -50.0), (90.0, 50.0)]);
    h.get_by_label("覚える").click();
    h.run();
    assert_eq!(st(&h).saved_selections()[1].name, "選択範囲 2");
    // 呼び出す: 1 つ目を「追加」で
    let before = steps(&h);
    h.get_by_label("追加: 選択範囲 1").click();
    h.run();
    assert_eq!(steps(&h), before + 1);
    assert_eq!(amount_at(&h, at(&h, -40.0, 0.0)), 255);
    assert_eq!(amount_at(&h, at(&h, 50.0, 0.0)), 255);
    // 消す
    h.get_by_label("消す: 選択範囲 1").click();
    h.run();
    assert_eq!(st(&h).saved_selections().len(), 1);
    assert_eq!(st(&h).saved_selections()[0].name, "選択範囲 2");
    // 名前を変える: 編集のアイコンのボタンで名前の欄になり、Enter で決める（1 回の取り消し）。Esc・外を押すとやめる
    let before = steps(&h);
    h.get_by_label("名前を変える: 選択範囲 2").click();
    h.run();
    assert_eq!(st(&h).sel.saved_window.as_ref().unwrap().rename, Some(0));
    key(&h, Key::A, Modifiers::COMMAND);
    h.step();
    h.event(Event::Text("前髪".to_owned()));
    h.step();
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    h.run();
    assert_eq!(st(&h).saved_selections()[0].name, "前髪");
    assert_eq!(steps(&h), before + 1);
    assert_eq!(
        st(&h).sel.saved_window.as_ref().unwrap().rename,
        None,
        "決めたら元の表示へ"
    );
    assert!(
        h.query_by_label("名前を変える: 前髪").is_some(),
        "ボタンの名前も新しい名前"
    );
    // やめる: 欄を開いて何も打たずに Esc で、名前も段も変わらない
    h.get_by_label("名前を変える: 前髪").click();
    h.run();
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    h.run();
    assert!(
        st(&h).sel.saved_window.is_some(),
        "名前の欄の Esc では、ウィンドウは閉じない"
    );
    assert_eq!(
        st(&h).sel.saved_window.as_ref().unwrap().rename,
        None,
        "欄はやめて元の表示へ"
    );
    assert_eq!(st(&h).saved_selections()[0].name, "前髪");
    assert_eq!(steps(&h), before + 1);
    // 閉じる
    h.get_by_label("閉じる").click();
    h.run();
    assert!(st(&h).sel.saved_window.is_none());
}

#[test]
fn the_saved_selections_window_draws_in_both_languages_and_keeps_names_short() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 800.0, 256);
        h.state_mut().state.lang = lang;
        h.state_mut().state.sel.animate = false;
        run(
            &mut h.state_mut().state,
            rect(SelectionCombine::Replace, 20, 20, 100, 100),
        );
        save(&mut h.state_mut().state, "Alpha");
        h.state_mut()
            .state
            .apply(Action::Sel(SelAction::Saved(SavedOp::OpenWindow)));
        h.run();
        // 何をするかはボタンのツールチップに出る（画面に説明の文は置かない）
        let remember = h
            .get_by_label(lang.pick("覚える", "Remember"))
            .rect()
            .center();
        hover_and_wait(&mut h, remember);
        h.get_by_label_contains(lang.pick("この名前で覚える", "under this name"));
        move_to(&h, egui::pos2(2.0, 2.0));
        h.run();
        h.get_by_label(&format!("{}: Alpha", lang.pick("共通", "Intersect")));
        let w = yolu_app::selection::saved::last_rect(&h.ctx).expect("ウィンドウが開いている");
        assert!(w.width() > 100.0);
        assert_plain(
            "ウィンドウの見出し",
            yolu_app::selection::saved::window_title(lang),
        );
    }
}

// ───────── クイックマスク ─────────

#[test]
fn quick_mask_shows_the_selection_in_red_and_the_brush_and_eraser_edit_it_as_one_undo_each() {
    let mut h = app(1000.0, 640.0, 512);
    pick_tool(&mut h, Tool::SelectRect);
    h.state_mut().state.sel.animate = false;
    drag_by(&mut h, &[(-100.0, -70.0), (30.0, 60.0)]);
    let base = st(&h).doc.selection().unwrap().clone();
    // Shift+Q で入る
    key(&h, Key::Q, Modifiers::SHIFT);
    h.run();
    assert!(st(&h).sel.quick);
    assert!(st(&h).sel.quick_overlay.texture_count() > 0, "赤い重ね");
    assert_eq!(steps(&h), 1, "入るだけでは段を積まない");
    // ブラシで描き足す（画素は描かない）
    pick_tool(&mut h, Tool::Brush);
    set_brush(&mut h, 24.0, 1.0, 1.0);
    let layer_before = canvas_pixel(&h, at(&h, 100.0, 0.0));
    drag_by(&mut h, &[(60.0, 0.0), (140.0, 0.0)]);
    assert_eq!(steps(&h), 2, "1 ストロークが 1 回の Undo");
    assert_eq!(
        amount_at(&h, at(&h, 100.0, 0.0)),
        255,
        "描き足した所が選択範囲に"
    );
    assert_eq!(amount_at(&h, at(&h, -50.0, 0.0)), 255, "元の選択範囲は残る");
    assert_eq!(
        canvas_pixel(&h, at(&h, 100.0, 0.0)),
        layer_before,
        "クイックマスクではレイヤーに描かない"
    );
    assert!(st(&h).canvas.stroke.is_none() && st(&h).sel.pen.is_none());
    // 消しゴムで直す
    pick_tool(&mut h, Tool::Eraser);
    drag_by(&mut h, &[(-90.0, 0.0), (-30.0, 0.0)]);
    assert_eq!(steps(&h), 3);
    assert_eq!(amount_at(&h, at(&h, -60.0, 0.0)), 0, "消した所は外れる");
    assert_eq!(amount_at(&h, at(&h, -60.0, 50.0)), 255);
    // 取り消しはそのまま文書の取り消し
    undo(&mut h);
    assert_eq!(amount_at(&h, at(&h, -60.0, 0.0)), 255);
    undo(&mut h);
    assert_eq!(amount_at(&h, at(&h, 100.0, 0.0)), 0);
    assert_eq!(st(&h).doc.selection(), Some(&base));
    assert!(st(&h).sel.quick, "取り消してもクイックマスクのまま");
    // もう一度 Shift+Q で戻る: 選択範囲は残り、ブラシはふつうに描く
    key(&h, Key::Q, Modifiers::SHIFT);
    h.run();
    assert!(!st(&h).sel.quick);
    assert_eq!(st(&h).doc.selection(), Some(&base));
    assert_eq!(
        st(&h).sel.quick_overlay.texture_count(),
        0,
        "重ねの絵は捨てる"
    );
    pick_tool(&mut h, Tool::Brush);
    drag_by(&mut h, &[(-60.0, -20.0), (-20.0, -20.0)]);
    assert!(
        canvas_pixel(&h, at(&h, -40.0, -20.0))[3] > 0,
        "ふつうに描ける"
    );
}

#[test]
fn quick_mask_with_no_selection_starts_empty_and_a_soft_brush_makes_partial_amounts() {
    let mut h = app(1000.0, 640.0, 512);
    h.state_mut().state.sel.animate = false;
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Ui(SelUiOp::QuickMask(Some(true)))));
    pick_tool(&mut h, Tool::Brush);
    set_brush(&mut h, 80.0, 0.0, 0.6);
    click_by(&mut h, 0.0, 0.0);
    let m = st(&h)
        .doc
        .selection()
        .expect("描いた所が選択範囲になる")
        .clone();
    let max = m.to_canvas_bytes().into_iter().max().unwrap();
    assert!((145..=153).contains(&max), "不透明度 60% が最大の量: {max}");
    assert!(
        m.to_canvas_bytes().iter().any(|a| *a > 0 && *a < 100),
        "縁は柔らかい"
    );
    // 消しゴムで全部消すと、選択範囲が無くなる
    pick_tool(&mut h, Tool::Eraser);
    set_brush(&mut h, 200.0, 1.0, 1.0);
    click_by(&mut h, 0.0, 0.0);
    assert!(st(&h).doc.selection().is_none());
}

#[test]
fn quick_mask_follows_the_pen_pressure_and_the_pen_eraser_end() {
    let mut h = app(1000.0, 640.0, 512);
    h.state_mut().state.sel.animate = false;
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Ui(SelUiOp::QuickMask(Some(true)))));
    pick_tool(&mut h, Tool::Brush);
    set_brush(&mut h, 60.0, 1.0, 1.0);
    h.state_mut().state.brush.pressure_size = true;
    let line: Vec<Pos2> = (0..=8)
        .map(|i| at(&h, -100.0 + 25.0 * i as f32, 0.0))
        .collect();
    let height_of = |h: &H| -> i32 {
        let (_, y0, _, y1) = bounds(st(h).doc.selection().unwrap()).unwrap();
        (y1 - y0) as i32
    };
    pen_stroke(&mut h, &line, 1.0, false);
    let full = height_of(&h);
    assert_eq!(steps(&h), 1, "ペンの 1 ストロークも 1 回の Undo");
    undo(&mut h);
    pen_stroke(&mut h, &line, 0.5, false);
    assert!((full - 2 * height_of(&h)).abs() <= 4, "筆圧で直径が変わる");
    undo(&mut h);
    // ペンの消しゴムの端: 選択範囲を消す
    run(&mut h.state_mut().state, SelEdit::All);
    h.run();
    pen_stroke(&mut h, &line, 1.0, true);
    assert_eq!(amount_at(&h, at(&h, 0.0, 0.0)), 0, "なぞった所は外れる");
    assert_eq!(amount_at(&h, at(&h, 0.0, 120.0)), 255);
    assert_eq!(
        canvas_pixel(&h, at(&h, 0.0, 0.0))[3],
        0,
        "レイヤーには描かない"
    );
}

#[test]
fn quick_mask_strokes_are_cancelled_by_escape_and_committed_by_focus_loss() {
    let mut h = app(1000.0, 640.0, 512);
    h.state_mut().state.sel.animate = false;
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Ui(SelUiOp::QuickMask(Some(true)))));
    pick_tool(&mut h, Tool::Brush);
    set_brush(&mut h, 30.0, 1.0, 1.0);
    let start = at(&h, -50.0, 0.0);
    h.event(Event::PointerMoved(start));
    h.event(Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    move_to(&h, at(&h, 50.0, 0.0));
    h.step();
    assert!(st(&h).is_stroking());
    assert!(st(&h).doc.selection().is_none(), "途中は文書を変えない");
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    h.run();
    assert!(!st(&h).is_stroking());
    assert!(
        st(&h).doc.selection().is_none() && steps(&h) == 0,
        "Esc で捨てる"
    );
    // フォーカスを失うと、そこまでを確定する（描くストロークと同じ）
    h.event(Event::PointerMoved(start));
    h.event(Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    move_to(&h, at(&h, 50.0, 0.0));
    h.step();
    h.event(Event::WindowFocused(false));
    h.step();
    h.run();
    assert!(!st(&h).is_stroking());
    assert_eq!(steps(&h), 1);
    assert_eq!(amount_at(&h, at(&h, 0.0, 0.0)), 255);
}

#[test]
fn quick_mask_is_off_when_the_document_changes_and_refused_while_drawing() {
    let mut s = AppState::new(64, 64);
    s.apply(Action::Sel(SelAction::Ui(SelUiOp::QuickMask(Some(true)))));
    assert!(s.sel.quick);
    s.sel_doc_changed();
    assert!(!s.sel.quick);
    s.canvas.stroke = Some(StrokeSource::Mouse);
    s.apply(Action::Sel(SelAction::Ui(SelUiOp::QuickMask(None))));
    assert!(!s.sel.quick, "描いている間は入らない");
    s.canvas.stroke = None;
    s.apply(Action::Sel(SelAction::Ui(SelUiOp::QuickMask(None))));
    assert!(s.sel.quick);
    s.apply(Action::Sel(SelAction::Ui(SelUiOp::QuickMask(None))));
    assert!(!s.sel.quick, "切り替え");
    // どちらの言語でも名前が出る
    for lang in Lang::ALL {
        s.lang = lang;
        s.apply(Action::Sel(SelAction::Ui(SelUiOp::QuickMask(Some(true)))));
        assert!(
            has_japanese(&s.message) == (lang == Lang::Ja),
            "{}",
            s.message
        );
        s.apply(Action::Sel(SelAction::Ui(SelUiOp::QuickMask(Some(false)))));
    }
}

#[test]
fn the_quick_mask_menu_item_is_checked_while_on_and_toggles() {
    use yolu_app::ui::menu::{Check, Entry};
    let mut h = app(1000.0, 640.0, 256);
    pick_tool(&mut h, Tool::SelectRect);
    let label = "クイックマスク";
    // ツールプロパティにもオプションバーにも置かない（「選択範囲」メニューとキー）
    assert_eq!(h.query_all_by_label("クイックマスク（Shift+Q）").count(), 0);
    // メニューの項目の印（入っているあいだ Checked）
    let mark = |h: &H| {
        yolu_app::selection::menu::select_menu(&h.state().state)
            .into_iter()
            .find_map(|e| match e {
                Entry::Item {
                    label: name, check, ..
                } if name == label => Some(matches!(check, Check::Checked)),
                _ => None,
            })
            .expect("クイックマスクの項目")
    };
    let toggle = |h: &mut H| {
        let title = menu_title(h, "選択範囲").center();
        click(h, title);
        let item = popup_item(h, label).center();
        click(h, item);
    };
    assert!(!st(&h).sel.quick && !mark(&h));
    toggle(&mut h);
    assert!(st(&h).sel.quick && mark(&h), "入っているあいだ印が付く");
    toggle(&mut h);
    assert!(!st(&h).sel.quick && !mark(&h));
}

// ───────── ツールの帯とキー・スナップショット ─────────

#[test]
fn the_selection_pen_is_the_last_selection_tool_in_the_strip_with_its_own_key() {
    let mut h = app(1000.0, 640.0, 256);
    h.get_by_label("選択ペン（S）").click();
    h.run();
    assert_eq!(st(&h).tool, Tool::SelectPen);
    pick_tool(&mut h, Tool::Brush);
    key(&h, Key::S, Modifiers::NONE);
    h.run();
    assert_eq!(st(&h).tool, Tool::SelectPen);
    assert!(Tool::SelectPen.is_select());
    let all = Tool::ALL;
    let at = all.iter().position(|t| *t == Tool::SelectPen).unwrap();
    assert_eq!(all[at - 1], Tool::IdSelect);
    assert_eq!(all[at + 1], Tool::Move);
    // ペンが触れるのは「描くツール」ではない
    assert!(!Tool::SelectPen.paints());
    assert_ne!(
        Tool::SelectPen.name_in(Lang::Ja),
        Tool::SelectPen.name_in(Lang::En)
    );
    // 切り替えのキーは Ctrl+S（保存）に食われない
    key(&h, Key::S, Modifiers::COMMAND);
    h.run();
    pick_tool(&mut h, Tool::Brush);
    assert_eq!(st(&h).tool, Tool::Brush);
}

#[test]
fn snapshots_of_the_creation_buttons_the_pen_options_the_quick_mask_and_the_saved_window() {
    let mut h = app(1280.0, 800.0, 512);
    h.state_mut().state.sel.animate = false;
    pick_tool(&mut h, Tool::SelectRect);
    drag_by(&mut h, &[(-120.0, -70.0), (60.0, 50.0)]);
    // 作成方法の組（Shift を押している間は追加が点き、選んでいる新規は枠だけ）
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    h.step();
    h.run();
    h.snapshot("selection_build_modes_shift");
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.step();
    // 選択ペンのオプションバーとプロパティ
    pick_tool(&mut h, Tool::SelectPen);
    h.snapshot("selection_build_pen");
    // 選択ペンのストロークの途中（足す: 青の重ね）
    set_brush(&mut h, 36.0, 0.6, 1.0);
    let start = at(&h, -90.0, -130.0);
    h.event(Event::PointerMoved(start));
    h.event(Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    for p in [(-40.0, -150.0), (20.0, -120.0), (80.0, -140.0)] {
        move_to(&h, at(&h, p.0, p.1));
        h.step();
    }
    h.step();
    h.snapshot("selection_build_pen_stroke");
    release(&h, at(&h, 80.0, -140.0), PointerButton::Primary);
    h.run();
    // クイックマスクの赤い重ね
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Ui(SelUiOp::QuickMask(Some(true)))));
    h.run();
    h.snapshot("selection_build_quick_mask");
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Ui(SelUiOp::QuickMask(Some(false)))));
    // 覚えた選択範囲のウィンドウ
    save(&mut h.state_mut().state, "選択範囲 1");
    save(&mut h.state_mut().state, "顔まわり");
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Saved(SavedOp::OpenWindow)));
    h.run();
    h.snapshot("selection_build_saved_window");
}

// ───────── 予算の断り・途中の切り替え・対称・ダブの量（レビューの指摘への試験） ─────────

/// 押して動かすところまで進める（離さない）。
fn hold(h: &mut H, points: &[Pos2]) {
    h.event(Event::PointerMoved(points[0]));
    h.event(Event::PointerButton {
        pos: points[0],
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    for p in &points[1..] {
        move_to(h, *p);
        h.step();
    }
    h.step();
}

fn let_go(h: &mut H, at: Pos2) {
    release(h, at, PointerButton::Primary);
    h.run();
}

fn quick_on(h: &mut H) {
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Ui(SelUiOp::QuickMask(Some(true)))));
    h.run();
}

/// ストロークが何も残していない（選択範囲・段・描くストローク・ペンの印）。
fn assert_nothing_left(h: &H, what: &str) {
    let s = st(h);
    assert!(s.doc.selection().is_none(), "{what}: 選択範囲にならない");
    assert_eq!(steps(h), 0, "{what}: 段を積まない");
    assert!(!s.is_stroking(), "{what}: 描いている状態が戻る");
    assert!(
        s.sel.pen.is_none() && s.sel.drag.is_none() && s.sel.pen_down.is_none(),
        "{what}: ペンの印が残らない"
    );
    assert!(!s.doc.has_active_stroke(), "{what}");
}

/// 予算を 1 タイルにして、タイルをまたぐ線をなぞる。断ったあと離しても何も起きず、予算を戻せばまた描ける。
fn a_stroke_past_the_pen_budget_is_dropped(quick: bool, stylus: bool) {
    let what = format!("quick={quick} stylus={stylus}");
    let mut h = app(1000.0, 640.0, 512);
    h.state_mut().state.sel.animate = false;
    if quick {
        quick_on(&mut h);
        pick_tool(&mut h, Tool::Brush);
    } else {
        pick_tool(&mut h, Tool::SelectPen);
    }
    set_brush(&mut h, 20.0, 1.0, 1.0);
    let tile = st(&h).doc.tile_size() as f64;
    h.state_mut().state.sel.pen_budget = (tile as u64).pow(2) * 3;
    let line = [
        at(&h, -100.0, -100.0),
        at(&h, -40.0, -60.0),
        at(&h, 40.0, 40.0),
        at(&h, 100.0, 100.0),
    ];
    let (a, b) = (to_canvas(&h, line[0]), to_canvas(&h, line[3]));
    assert!(
        (a.0 / tile).floor() != (b.0 / tile).floor(),
        "線は別のタイルへまたぐ"
    );
    if stylus {
        for p in line {
            h.state().pen().push(pen_at(p, true, 1.0, false));
            h.step();
        }
        h.step();
    } else {
        hold(&mut h, &line);
    }
    let lang = st(&h).lang;
    assert!(
        st(&h).sel.pen.is_none() && st(&h).sel.drag.is_none(),
        "{what}: 予算を超えたところで捨てる"
    );
    assert!(!st(&h).is_stroking(), "{what}: 描いている状態が戻る");
    assert_eq!(st(&h).message, PenError::TooLarge.text(lang), "{what}");
    // 断ったあとに離しても何も選択範囲にならない
    let last = *line.last().unwrap();
    if stylus {
        h.state().pen().push(pen_at(last, false, 1.0, false));
        h.step();
        h.run();
    } else {
        let_go(&mut h, last);
    }
    assert_nothing_left(&h, &what);
    assert_eq!(
        canvas_pixel(&h, line[0])[3],
        0,
        "{what}: レイヤーにも描かない"
    );
    // 予算を戻せば、同じ線がふつうに 1 回の Undo で通る
    h.state_mut().state.sel.pen_budget = PEN_BUDGET_BYTES;
    if stylus {
        pen_stroke(&mut h, &line, 1.0, false);
    } else {
        drag(&mut h, &line);
    }
    assert_eq!(steps(&h), 1, "{what}: 予算内なら 1 回の Undo");
    assert_eq!(amount_at(&h, line[2]), 255, "{what}");
}

#[test]
fn a_selection_pen_mouse_stroke_past_the_budget_is_dropped_and_releasing_selects_nothing() {
    a_stroke_past_the_pen_budget_is_dropped(false, false);
}

#[test]
fn a_selection_pen_stylus_stroke_past_the_budget_is_dropped_and_releasing_selects_nothing() {
    a_stroke_past_the_pen_budget_is_dropped(false, true);
}

#[test]
fn a_quick_mask_mouse_stroke_past_the_budget_is_dropped_and_releasing_selects_nothing() {
    a_stroke_past_the_pen_budget_is_dropped(true, false);
}

#[test]
fn a_quick_mask_stylus_stroke_past_the_budget_is_dropped_and_releasing_selects_nothing() {
    a_stroke_past_the_pen_budget_is_dropped(true, true);
}

#[test]
fn switching_the_tool_during_a_selection_pen_stroke_drops_it_and_releasing_selects_nothing() {
    let mut h = app(1000.0, 640.0, 512);
    h.state_mut().state.sel.animate = false;
    pick_tool(&mut h, Tool::SelectPen);
    set_brush(&mut h, 30.0, 1.0, 1.0);
    let line = [at(&h, -80.0, 0.0), at(&h, 0.0, 0.0), at(&h, 80.0, 0.0)];
    hold(&mut h, &line);
    assert!(
        st(&h).sel.pen.is_some() && st(&h).sel.drag.is_some(),
        "描いている途中"
    );
    pick_tool(&mut h, Tool::SelectRect);
    assert!(
        st(&h).sel.pen.is_none() && st(&h).sel.drag.is_none(),
        "ツールを替えたら捨てる"
    );
    let_go(&mut h, line[2]);
    assert_nothing_left(&h, "ツールの切り替え");
    for p in line {
        assert_eq!(canvas_pixel(&h, p)[3], 0, "レイヤーにも描かない");
    }
}

#[test]
fn switching_the_texture_set_during_a_selection_pen_stroke_drops_it_in_both_sets() {
    let mut h = app(1000.0, 640.0, 512);
    h.state_mut().state.sel.animate = false;
    pick_tool(&mut h, Tool::SelectPen);
    set_brush(&mut h, 30.0, 1.0, 1.0);
    let line = [at(&h, -80.0, 0.0), at(&h, 0.0, 0.0), at(&h, 80.0, 0.0)];
    hold(&mut h, &line);
    assert!(st(&h).sel.pen.is_some(), "描いている途中");
    // 選択ペンのストロークは描くストロークではないので、セットは替えられる（途中のストロークは捨てる）
    h.state_mut().state.add_texture_set().unwrap();
    h.run();
    assert!(st(&h).sel.pen.is_none() && st(&h).sel.drag.is_none());
    let_go(&mut h, line[2]);
    assert_nothing_left(&h, "2 つ目のセット");
    h.state_mut().state.switch_set(0).unwrap();
    h.run();
    assert_nothing_left(&h, "1 つ目のセット");
}

#[test]
fn during_a_quick_mask_stroke_the_set_stays_and_the_brush_stays_and_a_select_tool_keeps_the_stroke()
{
    let mut h = app(1000.0, 640.0, 512);
    h.state_mut().state.sel.animate = false;
    h.state_mut().state.add_texture_set().unwrap();
    h.state_mut().state.switch_set(0).unwrap();
    h.run();
    quick_on(&mut h);
    pick_tool(&mut h, Tool::Brush);
    set_brush(&mut h, 30.0, 1.0, 1.0);
    let line = [at(&h, -80.0, 0.0), at(&h, 0.0, 0.0), at(&h, 80.0, 0.0)];
    hold(&mut h, &line);
    assert!(st(&h).is_stroking() && st(&h).sel.pen.as_ref().is_some_and(|a| a.quick));
    // セットは替えられない（描いている間）
    assert!(h.state_mut().state.switch_set(1).is_err());
    assert_eq!(st(&h).sets.current_index(), 0);
    // ブラシから消しゴムへも替わらない
    h.state_mut().state.apply(Action::SelectTool(Tool::Eraser));
    assert_eq!(st(&h).tool, Tool::Brush);
    assert!(st(&h).sel.pen.is_some(), "ストロークは続く");
    // 選択のツールへは替わるが、始めたストロークは離しで終わる（取り残さない）
    pick_tool(&mut h, Tool::SelectRect);
    assert!(
        st(&h).sel.pen.is_some(),
        "クイックマスクのストロークはツールでは捨てない"
    );
    let_go(&mut h, line[2]);
    assert!(!st(&h).is_stroking() && st(&h).sel.pen.is_none() && st(&h).sel.drag.is_none());
    assert_eq!(steps(&h), 1, "1 ストロークが 1 回の Undo");
    assert_eq!(amount_at(&h, line[1]), 255);
}

#[test]
fn the_quick_mask_stroke_shows_no_mirrored_cursors_and_does_not_mirror() {
    let mut h = app(1000.0, 640.0, 512);
    h.state_mut().state.sel.animate = false;
    common::rulers::vertical(&mut h.state_mut().state, 256.0);
    pick_tool(&mut h, Tool::Brush);
    set_brush(&mut h, 20.0, 1.0, 1.0);
    // ふつうのブラシのストロークは写しを出し、対称を覚えて残す
    assert!(active_symmetry(st(&h)).is_some());
    drag_by(&mut h, &[(-100.0, -80.0), (-60.0, -80.0)]);
    assert!(
        st(&h).sel.stroke_symmetry.is_some(),
        "前のストロークの対称が残る"
    );
    assert!(active_symmetry(st(&h)).is_some());
    // クイックマスクでは出さない（描いている間も、古い対称を返さない）
    quick_on(&mut h);
    assert!(
        active_symmetry(st(&h)).is_none(),
        "クイックマスクは写しを出さない"
    );
    let line = [at(&h, -100.0, 0.0), at(&h, -60.0, 0.0)];
    hold(&mut h, &line);
    assert!(st(&h).is_stroking());
    assert!(
        active_symmetry(st(&h)).is_none(),
        "ストロークの最中に前の対称を出さない"
    );
    let_go(&mut h, line[1]);
    // 選択範囲は描いた側だけで、映した側には付かない
    assert_eq!(amount_at(&h, at(&h, -80.0, 0.0)), 255);
    assert_eq!(amount_at(&h, at(&h, 80.0, 0.0)), 0, "映さない");
    // 選択ペンのツールも対称を使わない。ブラシに戻れば写しが戻る
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Ui(SelUiOp::QuickMask(Some(false)))));
    pick_tool(&mut h, Tool::SelectPen);
    assert!(active_symmetry(st(&h)).is_none());
    pick_tool(&mut h, Tool::Brush);
    assert!(active_symmetry(st(&h)).is_some());
}

#[test]
fn headless_the_stroke_amount_does_not_depend_on_the_dab_spacing_or_the_overlap() {
    let doc = Document::new(256, 256).unwrap();
    let stroke = |points: &[(f64, f64)], hardness: f64| -> SelectionMask {
        let params = PenParams {
            radius: 8.0,
            hardness,
            opacity: 0.5,
            pressure_size: false,
            pressure_opacity: false,
        };
        let mut st = PenStroke::new(&doc, false, None, params, PEN_BUDGET_BYTES).unwrap();
        for &(x, y) in points {
            st.add(x, y, 1.0).unwrap();
        }
        st.finish().unwrap().0
    };
    // 1 画素おきの密な点・3 点だけの疎な点・同じ線を 3 回（往復を含む）なぞる点
    // 終わりの点はダブの間隔（直径の 5% = 0.8 画素）の倍数に乗せない（ちょうど終点に乗るダブが、浮動小数の丸めで置かれたり置かれなかったりするため）
    let mut dense: Vec<(f64, f64)> = (0..=160).map(|i| (40.25 + i as f64, 100.25)).collect();
    dense.push((200.55, 100.25));
    let sparse = [(40.25, 100.25), (120.25, 100.25), (200.55, 100.25)];
    let mut retraced = dense.clone();
    retraced.extend(dense.iter().rev());
    retraced.extend(dense.iter());
    for hardness in [1.0, 0.4] {
        let once = stroke(&dense, hardness);
        let spaced = stroke(&sparse, hardness);
        let again = stroke(&retraced, hardness);
        for (name, m) in [("密", &once), ("疎", &spaced), ("重ねる", &again)] {
            let max = m.to_canvas_bytes().into_iter().max().unwrap();
            assert_eq!(
                max, 128,
                "不透明度 50% の最大は重ねても 128 のまま（{name}・硬さ {hardness}）"
            );
            assert_eq!(m.amount(120, 100), 128, "{name}・硬さ {hardness}");
        }
        assert_eq!(
            once.to_canvas_bytes(),
            spaced.to_canvas_bytes(),
            "点の間隔は量にも範囲にも効かない（硬さ {hardness}）"
        );
        assert_eq!(
            bounds(&once),
            bounds(&again),
            "重ねても範囲が広がらない（硬さ {hardness}）"
        );
    }
    // 重ねる回数を増やしても量は同じ（5 回）
    let mut many = Vec::new();
    for _ in 0..5 {
        many.extend(dense.iter().copied());
    }
    let five = stroke(&many, 1.0);
    assert_eq!(five.to_canvas_bytes().into_iter().max().unwrap(), 128);
}

#[allow(dead_code)]
fn unused(_: Channel, _: TileCoord) {}
