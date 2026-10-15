//! 初めの設定（既定）の束: プロパティの欄の初めのタブ・スポイトの対象・知らせの位置・保存せずに閉じるときの問い・ブラシサイズの段・最近開いたプロジェクト。
//! テストは運転の口（`common::drive::Drive`）を通す。
use crate::common;

use common::drive::Drive;
use yolu_app::panels::properties::{tab_labels, TAB_LAYER, TAB_ORDER, TAB_STENCIL};
use yolu_app::state::{AppState, Tool};

#[test]
fn the_properties_panel_starts_on_the_layer_tab_which_is_the_left_one() {
    let mut h = AppState::new(32, 32);
    assert_eq!(h.ui().property_tab, TAB_LAYER);
    assert_eq!(TAB_ORDER[0], TAB_LAYER, "左のタブがレイヤー");
    assert_eq!(TAB_ORDER[1], TAB_STENCIL);
    h.set_lang(yolu_app::lang::Lang::Ja);
    assert_eq!(tab_labels(h.app()), ["レイヤー", "ステンシル"]);
    h.set_lang(yolu_app::lang::Lang::En);
    assert_eq!(tab_labels(h.app()), ["Layer", "Stencil"]);
}

#[test]
fn the_properties_tab_strip_shows_layer_on_the_left_and_shows_the_layer_body_first() {
    let mut h = common::app(1600.0, 900.0, 256);
    h.run();
    let props = h.state().tab_rects[&yolu_app::Tab::Properties];
    let in_props = |r: egui::Rect| r.left() > props.left() - 2.0 && r.top() > props.top();
    let layer = common::rect_of(&h, "レイヤー", in_props);
    let stencil = common::rect_of(&h, "ステンシル", in_props);
    assert!(layer.center().x < stencil.center().x);
    assert_eq!(h.ui().property_tab, TAB_LAYER);
    // 番号の意味（ステンシル = 0・レイヤー = 1）は並びを替えても変えない
    assert_eq!((TAB_STENCIL, TAB_LAYER), (0, 1));
    common::click(&mut h, stencil.center());
    assert_eq!(h.ui().property_tab, TAB_STENCIL);
}

#[test]
fn the_eyedropper_starts_on_all_layers_and_the_selected_layer_row_turns_it_off() {
    use yolu_app::subtool::{Key, SubToolAction};
    let mut h = AppState::new(32, 32);
    assert!(h.eyedrop().all_layers);
    let list = h.app().subtool_list(Tool::Eyedropper).unwrap();
    assert_eq!(list.current(), Key::Builtin("all"));
    assert!(!h
        .app()
        .subtool_is_modified(Tool::Eyedropper, list.current()));
    h.act(yolu_app::state::Action::SubTool(SubToolAction::Select(
        Tool::Eyedropper,
        Key::Builtin("layer"),
    )));
    assert!(!h.eyedrop().all_layers);
    h.act(yolu_app::state::Action::SubTool(SubToolAction::Select(
        Tool::Eyedropper,
        Key::Builtin("all"),
    )));
    assert!(h.eyedrop().all_layers);
}

// ───────── 知らせ（トースト）は状態の帯の左の端 ─────────

/// 描いた文字（`Shape::Text`）のうち、条件に合う文の矩形（画面の点）。
fn text_rects(
    h: &egui_kittest::Harness<'_, yolu_app::YoluApp>,
    pick: impl Fn(&str) -> bool,
) -> Vec<egui::Rect> {
    fn find(shape: &egui::Shape, pick: &dyn Fn(&str) -> bool, out: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| find(s, pick, out)),
            egui::Shape::Text(t) if pick(&t.galley.job.text) => {
                out.push(t.galley.rect.translate(t.pos.to_vec2()))
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for s in &h.output().shapes {
        find(&s.shape, &pick, &mut out);
    }
    out
}

fn with_usage(h: &mut egui_kittest::Harness<'static, yolu_app::YoluApp>) {
    h.usage_mut().build = Some("0.1.0 · a1b2c3d".into());
    h.usage_mut().process = Some(812 * 1024 * 1024);
    h.usage_mut().gpu = Some(1536 * 1024 * 1024);
    h.usage_mut().sampled_at = Some(1.0e9);
}

#[test]
fn the_toast_is_one_line_in_the_status_bar_from_its_left_edge_and_stops_before_the_right_items() {
    use yolu_app::notice::Source;
    let mut h = common::app(1000.0, 700.0, 128);
    h.set_lang(yolu_app::lang::Lang::Ja);
    with_usage(&mut h);
    h.event(egui::Event::PointerGone);
    let screen = h.ctx.content_rect();
    let bar_top = screen.bottom() - yolu_app::ui::theme::STATUS_BAR_HEIGHT;

    // 短い文: 帯の左の端に、そのまま 1 行
    h.app_mut()
        .info(Source::Project, "新しいプロジェクトを作りました。");
    h.run();
    let short = text_rects(&h, |s| s == "新しいプロジェクトを作りました。");
    assert_eq!(short.len(), 1);
    assert!(
        short[0].left() < 40.0 && short[0].top() >= bar_top && short[0].bottom() <= screen.bottom(),
        "{:?}",
        short[0]
    );

    // 右の項目の左端（これより左が知らせの置き場）
    let painter = h.ctx.layer_painter(egui::LayerId::background());
    let bar = egui::Rect::from_min_max(egui::pos2(screen.left(), bar_top), screen.max);
    let layout = yolu_app::shell::status_layout(&painter, h.app(), bar);
    assert!(!layout.items.is_empty() && layout.free_right < screen.right() - 40.0);

    // 長い文: 右の項目に重ならない幅で切り、「…」を付ける。全文はホバーのツールチップ
    let full = "とても長い知らせの文です。".repeat(40);
    h.app_mut().info(Source::Project, full.as_str());
    h.run();
    let cut = text_rects(&h, |s| s.starts_with("とても長い") && s.ends_with('…'));
    assert_eq!(cut.len(), 1, "切った 1 行");
    assert!(
        cut[0].left() < 40.0 && cut[0].right() <= layout.free_right + 0.5,
        "{:?} / {}",
        cut[0],
        layout.free_right
    );
    assert!(cut[0].top() >= bar_top, "1 行（帯の中）");
    assert!(
        text_rects(&h, |s| s == full).is_empty(),
        "全文は、ホバーするまで出ない"
    );
    h.event(egui::Event::PointerMoved(cut[0].center()));
    for _ in 0..60 {
        h.step();
    }
    assert_eq!(
        text_rects(&h, |s| s == full).len(),
        1,
        "ホバーで全文のツールチップ"
    );
}

#[test]
fn a_toast_that_fits_has_no_tooltip_and_the_text_readers_see_the_full_sentence() {
    use yolu_app::notice::Source;
    let mut h = common::app(1000.0, 700.0, 128);
    h.event(egui::Event::PointerGone);
    h.app_mut()
        .info(Source::Project, "新しいプロジェクトを作りました。");
    h.run();
    assert_eq!(
        h.message(),
        "新しいプロジェクトを作りました。",
        "テストが読む口は今のまま"
    );
    assert_eq!(
        yolu_app::shell::status_text(h.app()),
        "新しいプロジェクトを作りました。"
    );
    let at = text_rects(&h, |s| s == "新しいプロジェクトを作りました。")[0].center();
    h.event(egui::Event::PointerMoved(at));
    h.run();
    assert_eq!(
        text_rects(&h, |s| s == "新しいプロジェクトを作りました。").len(),
        1,
        "ツールチップは重ねない"
    );
}

// ───────── 保存せずに閉じるときの問い（アプリの中のウィンドウ・3 つのボタン）─────────

fn modified_app() -> egui_kittest::Harness<'static, yolu_app::YoluApp> {
    let mut h = common::app(1000.0, 700.0, 128);
    h.state_mut().set_discard_window(true);
    h.event(egui::Event::PointerGone);
    h.set_modified(true);
    h
}

fn press(h: &mut egui_kittest::Harness<'static, yolu_app::YoluApp>, label: &str) {
    use egui_kittest::kittest::Queryable;
    let at = h.get_by_label(label).rect().center();
    common::click(h, at);
    h.run();
}

#[test]
fn closing_with_unsaved_changes_asks_with_three_buttons_and_the_first_choice_stays_open() {
    use egui_kittest::kittest::Queryable;
    for lang in yolu_app::lang::Lang::ALL {
        let mut h = modified_app();
        h.set_lang(lang);
        h.act(yolu_app::state::Action::Quit);
        h.run();
        assert!(h.discard_ask().is_some(), "{lang:?}: 問いが開く");
        assert!(yolu_app::discard::window_rect(&h.ctx).is_some());
        assert!(!h.state().is_closing(), "答えるまで閉じない");
        let [review, discard, cancel] = yolu_app::discard::labels(lang);
        assert_eq!(
            [review, discard, cancel],
            lang.pick(
                ["変更を確認", "変更を破棄", "キャンセル"],
                ["Review Changes", "Discard Changes", "Cancel"]
            )
        );
        for label in [review, discard, cancel] {
            assert_eq!(h.query_all_by_label(label).count(), 1, "{label}");
        }
        // 文は、ボタンに合う文（捨てるかの問いにしない）
        let text = h.discard_ask().as_ref().unwrap().text.clone();
        assert_eq!(
            text,
            lang.pick(
                "保存していない変更があります。",
                "There are unsaved changes."
            )
        );
        // Enter は「変更を確認」: 閉じずにアプリへ戻る（変更は残る）
        common::key(&h, egui::Key::Enter, egui::Modifiers::NONE);
        h.run();
        assert!(h.discard_ask().is_none() && !h.state().is_closing());
        assert!(h.modified() && !h.quit());
    }
}

#[test]
fn the_discard_button_closes_and_review_and_cancel_and_escape_do_not() {
    // 変更を破棄: 問わずに閉じる
    let mut h = modified_app();
    h.act(yolu_app::state::Action::Quit);
    h.run();
    press(&mut h, "変更を破棄");
    assert!(h.state().is_closing(), "変更を破棄で閉じる");
    assert!(h.discard_ask().is_none());
    // 変更を確認・キャンセル・Esc: 閉じない
    for how in ["変更を確認", "キャンセル", "Esc"] {
        let mut h = modified_app();
        h.act(yolu_app::state::Action::Quit);
        h.run();
        if how == "Esc" {
            common::key(&h, egui::Key::Escape, egui::Modifiers::NONE);
            h.run();
        } else {
            press(&mut h, how);
        }
        assert!(
            !h.state().is_closing() && h.discard_ask().is_none(),
            "{how}"
        );
        assert!(h.modified(), "{how}: 変更は残る");
        // もう一度閉じると、また聞く
        h.act(yolu_app::state::Action::Quit);
        h.run();
        assert!(h.discard_ask().is_some(), "{how}: もう一度聞く");
    }
}

#[test]
fn while_the_question_is_open_undo_save_and_new_keys_do_not_reach_the_document() {
    use egui::{Key, Modifiers};
    let mut h = modified_app();
    let c = common::canvas_rect(&h).center();
    common::drag(&mut h, &[c, common::offset(c, 10.0, 0.0)]);
    assert_eq!(common::canvas_pixel(&h, c)[3], 255, "線が引けた");
    h.act(yolu_app::state::Action::Quit);
    h.run();
    assert!(h.discard_ask().is_some());
    for (key, modifiers) in [
        (Key::Z, Modifiers::COMMAND),
        (Key::S, Modifiers::COMMAND),
        (Key::N, Modifiers::COMMAND),
        (Key::O, Modifiers::COMMAND),
    ] {
        common::key(&h, key, modifiers);
        h.run();
        assert!(h.discard_ask().is_some(), "{key:?}: 問いは開いたまま");
        assert_eq!(
            common::canvas_pixel(&h, c)[3],
            255,
            "{key:?}: 後ろの文書は変わらない"
        );
        assert!(h.modified(), "{key:?}");
        assert!(
            h.dialog_request().is_none(),
            "{key:?}: ファイルの頼みも出ない"
        );
    }
    // 答えたあとは、同じキーが文書に届く
    press(&mut h, "変更を確認");
    common::key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert_eq!(
        common::canvas_pixel(&h, c)[3],
        0,
        "問いが閉じたら元に戻せる"
    );
}

#[test]
fn nothing_is_asked_without_changes_and_the_dialogs_off_form_is_unchanged() {
    let mut h = common::app(1000.0, 700.0, 128);
    h.state_mut().set_discard_window(true);
    h.act(yolu_app::state::Action::Quit);
    h.run();
    assert!(
        h.discard_ask().is_none() && h.state().is_closing(),
        "変更が無ければ聞かない"
    );
    // ウィンドウを開かないテストの形（問いのウィンドウも許さない）は、聞かずに閉じる
    let mut h = common::app(1000.0, 700.0, 128);
    h.set_modified(true);
    h.act(yolu_app::state::Action::Quit);
    h.run();
    assert!(h.discard_ask().is_none() && h.state().is_closing());
}

#[test]
fn opening_a_dropped_project_over_unsaved_changes_asks_the_same_three_buttons() {
    #[derive(Debug)]
    struct Dropped(std::path::PathBuf);
    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            Err("none".into())
        }
    }
    let missing =
        std::env::temp_dir().join(format!("yolu-defaults-missing-{}.ylp", std::process::id()));
    let drop = |h: &mut egui_kittest::Harness<'static, yolu_app::YoluApp>| {
        h.input_mut()
            .dropped_files
            .push(std::sync::Arc::new(Dropped(missing.clone())));
        h.run();
    };
    // 変更を確認: 開かない（何も言わない）
    let mut h = modified_app();
    drop(&mut h);
    assert!(h.discard_ask().is_some());
    press(&mut h, "変更を確認");
    assert!(h.message().is_empty(), "開かない: {}", h.message());
    assert!(h.modified());
    // キャンセル: 開かない
    drop(&mut h);
    press(&mut h, "キャンセル");
    assert!(h.message().is_empty());
    // 変更を破棄: 開く（ここでは無いファイルなので、開けない断りが出る）
    drop(&mut h);
    press(&mut h, "変更を破棄");
    assert!(h.message().contains("を開けません"), "{}", h.message());
}

#[test]
fn the_new_project_request_asks_before_the_window_and_only_discard_goes_on() {
    use yolu_app::state::DialogRequest;
    let mut h = modified_app();
    *h.dialog_request_mut() = Some(DialogRequest::New);
    h.run();
    assert!(h.discard_ask().is_some());
    assert!(
        !h.np().window.is_some(),
        "答えるまで新しいプロジェクトのウィンドウは開かない"
    );
    press(&mut h, "キャンセル");
    assert!(!h.np().window.is_some());
    *h.dialog_request_mut() = Some(DialogRequest::New);
    h.run();
    press(&mut h, "変更を破棄");
    assert!(h.np().window.is_some(), "変更を破棄で進む");
}

#[test]
fn a_question_about_running_jobs_only_has_two_buttons_and_enter_does_not_quit() {
    use yolu_app::psd::PsdAction;
    use yolu_app::state::Action;
    let dir = std::env::temp_dir().join(format!("yolu-defaults-jobs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut h = common::app(1000.0, 700.0, 128);
    h.state_mut().set_discard_window(true);
    h.event(egui::Event::PointerGone);
    h.psd_mut().park_next = true;
    h.act(Action::Psd(PsdAction::Export(dir.join("out.psd"))));
    assert!(h.psd().is_busy() && !h.modified());
    h.act(Action::Quit);
    h.run();
    let ask = h.discard_ask().as_ref().expect("仕事を取り消すので聞く");
    assert_eq!(ask.kind, yolu_app::discard::Kind::JobsOnly);
    {
        use egui_kittest::kittest::Queryable;
        assert_eq!(h.query_all_by_label("終わる").count(), 1);
        assert_eq!(h.query_all_by_label("キャンセル").count(), 1);
        assert_eq!(
            h.query_all_by_label("変更を確認").count(),
            0,
            "変更が無いので「変更を確認」は出さない"
        );
    }
    // Enter は終わらない（キャンセル）
    common::key(&h, egui::Key::Enter, egui::Modifiers::NONE);
    h.run();
    assert!(h.discard_ask().is_none() && !h.state().is_closing() && h.psd().is_busy());
    // 終わる: 仕事を取り消して閉じる
    h.act(Action::Quit);
    h.run();
    press(&mut h, "終わる");
    assert!(h.state().is_closing() && !h.psd().is_busy());
    let _ = std::fs::remove_dir_all(dir);
}

// ───────── ブラシサイズの段 ─────────

#[test]
fn the_brush_size_panel_has_twenty_eight_circles_with_decimal_names_that_set_the_diameter() {
    use yolu_app::panels::brushes::{nearest_size, size_name, SIZES};
    let mut h = common::app(1600.0, 1400.0, 128);
    h.event(egui::Event::PointerGone);
    let cell = |h: &egui_kittest::Harness<'_, yolu_app::YoluApp>, size: f32| {
        common::rect_of(h, &size_name(size), |r| {
            r.left() < 400.0 && r.top() > 100.0 && r.width() < 60.0
        })
    };
    assert_eq!(SIZES.len(), 28);
    for size in SIZES {
        let _ = cell(&h, size);
    }
    for (size, radius) in [
        (1.0f32, 0.5f32),
        (1.5, 0.75),
        (2.5, 1.25),
        (17.0, 8.5),
        (250.0, 125.0),
    ] {
        let at = cell(&h, size).center();
        common::click(&mut h, at);
        assert_eq!(h.brush().radius, radius, "{size}");
        assert_eq!(SIZES[nearest_size(h.brush().radius * 2.0)], size);
    }
}

// ───────── 最近開いたプロジェクト ─────────

fn names(entries: &[yolu_app::ui::menu::Entry<yolu_app::state::Action>]) -> Vec<String> {
    use yolu_app::ui::menu::Entry;
    entries
        .iter()
        .map(|e| match e {
            Entry::Item { label, .. } | Entry::Submenu { label, .. } => label.clone(),
            Entry::Separator => "-".into(),
            Entry::Heading(h) => h.clone(),
        })
        .collect()
}

fn paths(h: &impl Drive) -> Vec<std::path::PathBuf> {
    h.recent_projects().items().to_vec()
}

#[test]
fn the_recent_list_puts_new_files_first_moves_a_repeat_to_the_front_and_keeps_ten() {
    use yolu_app::recent::{Recent, MAX};
    let dir = common::tmp::test_dir("recent-list");
    let mut recent = Recent::load(Some(dir.join("recent.conf")));
    assert!(recent.is_empty());
    let file = |n: usize| dir.join(format!("p{n}.ylp"));
    for n in 0..12 {
        recent.add(&file(n));
    }
    assert_eq!(MAX, 10);
    assert_eq!(recent.items().len(), 10, "10 で切る");
    assert_eq!(recent.items()[0], file(11), "新しい順");
    assert_eq!(recent.items()[9], file(2), "古い 2 つは落ちる");
    // 同じファイルは前へ動かす（増えない）
    recent.add(&file(5));
    assert_eq!(recent.items().len(), 10);
    assert_eq!(recent.items()[0], file(5));
    assert_eq!(recent.items().iter().filter(|p| **p == file(5)).count(), 1);
    // ファイルから読み直しても同じ
    let again = Recent::load(Some(dir.join("recent.conf")));
    assert_eq!(again.items(), recent.items());
    // 相対パスは覚えない（絶対にして覚える）
    let mut other = Recent::default();
    other.add(std::path::Path::new("relative.ylp"));
    assert!(other.items()[0].is_absolute());
}

#[test]
fn a_missing_recent_file_is_refused_and_dropped_and_clear_empties_the_list_and_the_file() {
    use yolu_app::recent::Recent;
    use yolu_app::state::Action;
    let dir = common::tmp::test_dir("recent-missing");
    let conf = dir.join("recent.conf");
    let mut h = AppState::new(32, 32);
    *h.recent_projects_mut() = Recent::load(Some(conf.clone()));
    let gone = dir.join("gone.ylp");
    h.recent_projects_mut().add(&gone);
    assert_eq!(paths(&h), std::slice::from_ref(&gone));
    h.act(Action::OpenRecent(gone.clone()));
    assert!(
        h.message().contains("ファイルがありません"),
        "{}",
        h.message()
    );
    assert!(h.dialog_request().is_none(), "開こうとしない");
    assert!(paths(&h).is_empty(), "一覧から外す");
    assert_eq!(
        Recent::load(Some(conf.clone())).items().len(),
        0,
        "ファイルからも外れる"
    );
    // 英語
    h.set_lang(yolu_app::lang::Lang::En);
    h.act(Action::OpenRecent(gone));
    assert!(h.message().contains("does not exist"), "{}", h.message());
    // 一覧を消す
    for n in 0..3 {
        h.recent_projects_mut().add(&dir.join(format!("p{n}.ylp")));
    }
    h.act(Action::ClearRecent);
    assert!(paths(&h).is_empty());
    assert!(Recent::load(Some(conf)).is_empty());
}

#[test]
fn saving_and_opening_a_project_put_it_first_and_choosing_it_opens_it() {
    use yolu_app::recent::Recent;
    use yolu_app::state::{Action, DialogRequest};
    let dir = common::tmp::test_dir("recent-open");
    let conf = dir.join("recent.conf");
    let mut h = AppState::new(32, 32);
    *h.recent_projects_mut() = Recent::load(Some(conf.clone()));
    let a = dir.join("a.ylp");
    let b = dir.join("b.ylp");
    // 名前を付けて保存も入れる
    h.act(Action::SaveProjectAs(a.clone()));
    h.app_mut().wait_save();
    h.act(Action::SaveProjectAs(b.clone()));
    h.app_mut().wait_save();
    assert_eq!(paths(&h), [b.clone(), a.clone()]);
    // 開いたら前へ動く
    h.act(Action::OpenProject(a.clone()));
    assert!(h.message().contains("開きました"), "{}", h.message());
    assert_eq!(paths(&h), [a.clone(), b.clone()]);
    // 選ぶと、「開く」と同じ流れ（保存していない変更の問い）へ
    h.act(Action::OpenRecent(b.clone()));
    assert_eq!(
        h.dialog_request(),
        &Some(DialogRequest::OpenPath(b.clone()))
    );
    assert_eq!(Recent::load(Some(conf)).items(), [a, b]);
}

/// 最近のプロジェクトを選んだあと、フレームを進めた先を確かめる準備: 中身の違う 2 つのプロジェクトのファイル（`a.ylp`・`b.ylp`）を書き、
/// 問いのウィンドウを許したアプリ（今の文書は「here」のレイヤーを持つ）の一覧に両方を入れる（a が先）。
fn recent_app(
    tag: &str,
) -> (
    egui_kittest::Harness<'static, yolu_app::YoluApp>,
    [std::path::PathBuf; 2],
) {
    use yolu_app::recent::Recent;
    use yolu_app::state::Action;
    let dir = common::tmp::test_dir(tag);
    let files = ["a", "b"].map(|name| dir.join(format!("{name}.ylp")));
    for (file, layer) in files.iter().zip(["from-a", "from-b"]) {
        let mut s = AppState::new(32, 32);
        s.doc_mut().add_layer(layer).unwrap();
        s.apply(Action::SaveProjectAs(file.clone()));
        s.wait_save();
        assert!(file.is_file());
    }
    let mut h = common::app(1000.0, 700.0, 128);
    h.state_mut().set_discard_window(true);
    *h.recent_projects_mut() = Recent::load(Some(dir.join("recent.conf")));
    for file in &files {
        h.recent_projects_mut().add(file);
    }
    h.doc_mut().add_layer("here").unwrap();
    h.run();
    (h, files)
}

fn layer_names(h: &impl Drive) -> Vec<String> {
    h.doc()
        .layers()
        .iter()
        .map(|l| l.name().to_owned())
        .collect()
}

#[test]
fn choosing_a_recent_project_without_changes_opens_it_and_moves_it_to_the_front() {
    use yolu_app::state::Action;
    let (mut h, [a, b]) = recent_app("recent-pick-clean");
    assert_eq!(paths(&h), [b.clone(), a.clone()], "後に足した b が先");
    h.act(Action::OpenRecent(a.clone()));
    h.run();
    assert!(h.discard_ask().is_none(), "変更が無ければ聞かない");
    assert!(h.dialog_request().is_none(), "頼みは当たって消える");
    assert!(layer_names(&h).contains(&"from-a".into()), "a を開いた");
    assert!(!layer_names(&h).contains(&"here".into()));
    assert!(!h.modified());
    assert!(h.message().contains("開きました"), "{}", h.message());
    assert_eq!(paths(&h), [a, b], "開いたプロジェクトが一覧の先頭へ");
}

#[test]
fn choosing_a_recent_project_over_unsaved_changes_asks_and_each_answer_keeps_or_discards_them() {
    use yolu_app::state::Action;
    for answer in ["変更を確認", "キャンセル", "変更を破棄"] {
        let (mut h, [a, b]) = recent_app("recent-pick-dirty");
        h.set_modified(true);
        h.act(Action::OpenRecent(a.clone()));
        h.run();
        assert!(h.discard_ask().is_some(), "{answer}: 開く前に聞く");
        assert!(
            layer_names(&h).contains(&"here".into()) && !layer_names(&h).contains(&"from-a".into()),
            "{answer}: 答えるまで開かない: {:?}",
            layer_names(&h)
        );
        assert_eq!(
            paths(&h),
            [b.clone(), a.clone()],
            "{answer}: 一覧もそのまま"
        );
        press(&mut h, answer);
        assert!(h.discard_ask().is_none(), "{answer}: 問いは閉じる");
        h.run();
        if answer == "変更を破棄" {
            assert!(layer_names(&h).contains(&"from-a".into()), "{answer}: 開く");
            assert!(
                !layer_names(&h).contains(&"here".into()),
                "{answer}: 変更は捨てた"
            );
            assert!(!h.modified());
            assert_eq!(paths(&h), [a, b], "{answer}: 一覧の先頭へ");
        } else {
            assert!(
                layer_names(&h).contains(&"here".into()),
                "{answer}: 変更を保つ"
            );
            assert!(
                !layer_names(&h).contains(&"from-a".into()),
                "{answer}: 開かない"
            );
            assert!(h.modified(), "{answer}");
            assert_eq!(paths(&h), [b, a], "{answer}: 一覧もそのまま");
        }
        assert!(h.dialog_request().is_none(), "{answer}: 頼みは残らない");
    }
}

#[test]
fn the_file_menu_has_a_nested_open_recent_under_open_that_is_off_while_the_list_is_empty() {
    use yolu_app::recent::Recent;
    use yolu_app::ui::menu::Entry;
    let mut h = AppState::new(32, 32);
    for (lang, label, clear) in [
        (
            yolu_app::lang::Lang::Ja,
            "最近開いたプロジェクト",
            "一覧を消す",
        ),
        (yolu_app::lang::Lang::En, "Open Recent", "Clear Recent"),
    ] {
        h.set_lang(lang);
        *h.recent_projects_mut() = Recent::default();
        let file_menu = yolu_app::shell::menu_entries(h.app(), 0);
        let labels = names(&file_menu);
        let at = labels
            .iter()
            .position(|l| l == label)
            .expect("入れ子のメニュー");
        assert!(
            matches!(&file_menu[at - 1], Entry::Item { label, .. } if label.starts_with(lang.pick("開く", "Open"))),
            "「開く…」のすぐ下: {labels:?}"
        );
        assert!(
            matches!(&file_menu[at], Entry::Submenu { enabled: false, .. }),
            "空なら押せない"
        );
        // 12 件足しても 10 件。新しい順・名前はファイル名・ツールチップはフルパス・最後に区切りと一覧を消す
        let dir = std::env::temp_dir();
        for n in 0..12 {
            h.recent_projects_mut()
                .add(&dir.join(format!("menu{n}.ylp")));
        }
        let file_menu = yolu_app::shell::menu_entries(h.app(), 0);
        let Entry::Submenu {
            entries, enabled, ..
        } = &file_menu[at]
        else {
            panic!("入れ子");
        };
        assert!(*enabled);
        assert_eq!(entries.len(), 10 + 2);
        match &entries[0] {
            Entry::Item { label, tooltip, .. } => {
                assert_eq!(label, "menu11.ylp");
                assert_eq!(
                    tooltip.as_deref(),
                    Some(dir.join("menu11.ylp").display().to_string().as_str())
                );
            }
            _ => panic!("項目"),
        }
        let last = names(entries);
        assert_eq!(&last[10..], ["-", clear]);
    }
}

#[test]
fn an_unreadable_or_newer_recent_file_starts_empty_and_is_moved_aside_before_it_is_written() {
    use yolu_app::recent::Recent;
    for (tag, bytes) in [
        ("garbage", vec![0xffu8, 0xfe, 0x00, 0x41]),
        (
            "newer",
            b"yolupainter-recent 2\n/somewhere/a.ylp\n".to_vec(),
        ),
        ("headerless", b"/somewhere/a.ylp\n".to_vec()),
    ] {
        let dir = common::tmp::test_dir(&format!("recent-broken-{tag}"));
        let conf = dir.join("recent.conf");
        std::fs::write(&conf, &bytes).unwrap();
        let mut recent = Recent::load(Some(conf.clone()));
        assert!(recent.is_empty(), "{tag}: 空の一覧");
        assert_eq!(
            std::fs::read(&conf).unwrap(),
            bytes,
            "{tag}: 読んだだけでは触らない"
        );
        recent.add(&dir.join("new.ylp"));
        assert_eq!(
            std::fs::read(dir.join("recent.conf.broken")).unwrap(),
            bytes,
            "{tag}: 前のファイルは別の名前で残る"
        );
        assert_eq!(
            Recent::load(Some(conf)).items(),
            [dir.join("new.ylp")],
            "{tag}"
        );
        // もう一度壊れても、前の移したファイルを上書きしない
        std::fs::write(dir.join("recent.conf"), [0xffu8]).unwrap();
        let mut again = Recent::load(Some(dir.join("recent.conf")));
        again.add(&dir.join("x.ylp"));
        assert_eq!(
            std::fs::read(dir.join("recent.conf.broken")).unwrap(),
            bytes,
            "{tag}"
        );
        assert_eq!(
            std::fs::read(dir.join("recent.conf.broken-2")).unwrap(),
            [0xffu8],
            "{tag}"
        );
    }
}

#[test]
fn a_state_without_a_place_never_touches_the_users_settings_folder() {
    let mut h = AppState::new(32, 32);
    assert_eq!(h.recent_projects().items().len(), 0);
    h.recent_projects_mut()
        .add(&std::env::temp_dir().join("only-in-memory.ylp"));
    assert_eq!(paths(&h).len(), 1, "覚えるのは開いている間だけ");
    // 置き場を持たない一覧は何も書かない（テストの状態の既定）
    let _ = yolu_app::settings::recent_path();
}
