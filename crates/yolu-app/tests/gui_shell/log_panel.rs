//! ログのパネル（ドックのタブ「ログ」）: 起動してからの注意と失敗だけを、古い物を上に並べる。絞る・写す・消す・まとめ・上限と、
//! 日英の見た目。
use crate::common;
use egui::{vec2, Event, Key, Modifiers, OutputCommand};
use egui_kittest::{kittest::Queryable, Harness};
use std::time::{Duration, UNIX_EPOCH};
use yolu_app::lang::Lang;
use yolu_app::notice::{set_utc_offset_for_test, Kind, Notice, Source, LOG_LIMIT};
use yolu_app::panels::log;
use yolu_app::state::AppState;
use yolu_app::YoluApp;

fn harness(app: AppState, size: egui::Vec2) -> Harness<'static, AppState> {
    let mut ready = false;
    let mut h = common::gpu_thread::builder()
        .with_size(size)
        .with_pixels_per_point(1.0)
        .renderer(common::shared_gpu::renderer())
        .build_ui_state(
            move |ui, app| {
                if !ready {
                    YoluApp::setup(ui.ctx());
                    ready = true;
                    ui.ctx().request_repaint();
                    return;
                }
                log::show(ui, app);
            },
            app,
        );
    h.run();
    h
}

/// 決まった時刻の知らせ（画面の絵を、走らせた時刻・机の時間帯によらず同じにする）。
fn at(app: &mut AppState, kind: Kind, source: Source, text: &str, seconds: u64) {
    app.notice_log.push(Notice {
        kind,
        source,
        text: text.into(),
        at: UNIX_EPOCH + Duration::from_secs(seconds),
    });
}

/// 絵の見本の知らせ（言語ごと）。
fn sample(lang: Lang) -> AppState {
    set_utc_offset_for_test(9 * 3600);
    let mut app = AppState::new_in(64, 64, lang);
    let base = 1_790_000_000;
    at(
        &mut app,
        Kind::Warning,
        Source::Bake,
        lang.pick(
            "「肌」の 2048×2048 のメッシュマップ 6 枚を焼きました。UV の面積が 0 の三角形があり、その面は焼けません。",
            "Baked 6 map(s) of \"Skin\" at 2048×2048. Some triangles have no UV area; those faces are not baked.",
        ),
        base,
    );
    for i in 0..3 {
        at(
            &mut app,
            Kind::Error,
            Source::Save,
            lang.pick(
                "「作品.ylp」に保存できません（アクセスが拒否されました（OS エラー 5））。",
                "Cannot save to \"work.ylp\" (Access denied (OS error 5)).",
            ),
            base + 60 + i,
        );
    }
    at(
        &mut app,
        Kind::Warning,
        Source::Psd,
        lang.pick(
            "PSD を読み込みました（a.psd・レイヤー 12）。PSD 自体は書き換えません。ほか 2 件は取り込みません（PSD は 1 つずつ）。",
            "Imported a.psd (12 layers). The PSD itself is never rewritten. 2 more not imported (one PSD at a time).",
        ),
        base + 125,
    );
    at(
        &mut app,
        Kind::Error,
        Source::Library,
        lang.pick(
            "「素材」をライブラリに入れられません（名前に使えない文字があります）。",
            "Cannot add \"Asset\" to the library (The name has disallowed characters).",
        ),
        base + 3700,
    );
    app
}

fn row_labels(h: &Harness<'_, AppState>) -> Vec<String> {
    let lang = h.state().lang;
    log::visible_entries(h.state())
        .into_iter()
        .map(|e| log::row_text(lang, e))
        .collect()
}

fn copied(h: &Harness<'_, AppState>) -> Option<String> {
    h.output()
        .platform_output
        .commands
        .iter()
        .find_map(|c| match c {
            OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
}

#[test]
fn log_panel_snapshot() {
    let mut snapshots = egui_kittest::SnapshotResults::new();
    for lang in Lang::ALL {
        let mut h = harness(sample(lang), vec2(420.0, 220.0));
        h.run();
        h.snapshot(lang.pick("log_panel", "log_panel_english"));
        snapshots.extend_harness(&mut h);
    }
}

#[test]
fn notices_add_rows_only_for_warnings_and_errors() {
    let mut app = AppState::new_in(64, 64, Lang::Ja);
    app.info(Source::Save, "保存しました。");
    app.refuse(Source::Edit, "描いている間はできません。");
    let mut h = harness(app, vec2(420.0, 220.0));
    assert!(row_labels(&h).is_empty(), "済んだ知らせと断りは出さない");
    h.state_mut()
        .warn(Source::Bake, "UV が重なる所があります。");
    h.state_mut()
        .fail(Source::Open, "「a.ylp」を開けません（壊れています）。");
    h.run();
    let rows = row_labels(&h);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(
        rows[0].contains("注意 ベイク: UV が重なる所があります。"),
        "{rows:?}"
    );
    assert!(
        rows[1].contains("エラー 開く: 「a.ylp」を開けません（壊れています）。"),
        "{rows:?}"
    );
    // 行の文は画面に出ている（長い文は切って、全文はツールチップと読み上げの名前）
    assert!(h
        .query_by_label_contains("「a.ylp」を開けません（壊れています）。")
        .is_some());
}

#[test]
fn the_same_notice_in_a_row_is_one_line_with_a_count_and_the_oldest_rows_go_past_the_limit() {
    let mut app = AppState::new_in(64, 64, Lang::En);
    for _ in 0..3 {
        app.fail(Source::Save, "Cannot save to \"a.ylp\" (Disk full).");
    }
    let mut h = harness(app, vec2(420.0, 220.0));
    let rows = row_labels(&h);
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].ends_with("Error Save: Cannot save to \"a.ylp\" (Disk full). (×3)"),
        "{rows:?}"
    );
    assert!(h.query_by_label_contains("(×3)").is_some());
    for i in 0..LOG_LIMIT + 10 {
        h.state_mut().warn(Source::Bake, format!("note {i}"));
    }
    h.run();
    assert_eq!(h.state().notice_log.len(), LOG_LIMIT);
    let rows = row_labels(&h);
    assert!(rows[0].contains("note 10"), "{}", rows[0]);
    assert!(rows
        .last()
        .unwrap()
        .contains(&format!("note {}", LOG_LIMIT + 9)));
}

#[test]
fn errors_only_copy_and_clear() {
    for lang in Lang::ALL {
        let mut h = harness(sample(lang), vec2(420.0, 220.0));
        assert_eq!(row_labels(&h).len(), 4);
        // エラーだけに絞る（切り替え）
        h.get_by_label(lang.pick("エラーだけ", "Errors Only"))
            .click();
        h.run();
        assert!(h.state().log_view.errors_only);
        let rows = row_labels(&h);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(rows
            .iter()
            .all(|r| r.contains(lang.pick("エラー", "Error"))));
        // 全部を写す（見えている行）
        h.get_by_label(lang.pick("全部を写す", "Copy All")).click();
        h.step();
        assert_eq!(copied(&h).unwrap(), rows.join("\n"));
        // 行を押して選び、選んだ行を写す
        let first = rows[0].clone();
        h.get_by_label(&first).click();
        h.run();
        assert_eq!(h.state().log_view.selected.len(), 1);
        h.get_by_label(lang.pick("選んだ行を写す", "Copy Selected"))
            .click();
        h.step();
        assert_eq!(copied(&h).unwrap(), first);
        // 一覧を押したあとは Ctrl+C でも写す（選んだ行）
        h.get_by_label(&first).click();
        h.run();
        h.event(Event::Copy);
        h.step();
        assert_eq!(copied(&h).unwrap(), first);
        h.event(Event::Key {
            key: Key::C,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        });
        h.step();
        assert_eq!(copied(&h).unwrap(), first);
        // 絞りを戻すと全部の行、消すと画面の一覧が空になる（記録のファイルは消さない）
        h.get_by_label(lang.pick("エラーだけ", "Errors Only"))
            .click();
        h.run();
        assert_eq!(row_labels(&h).len(), 4);
        h.get_by_label(lang.pick("ログを消す", "Clear Log")).click();
        h.run();
        assert!(h.state().notice_log.is_empty());
        assert!(h.state().log_view.selected.is_empty());
    }
}

#[test]
fn ctrl_c_is_left_to_the_canvas_until_the_list_is_clicked() {
    let mut h = harness(sample(Lang::Ja), vec2(420.0, 220.0));
    // 一覧を押していなければ、Ctrl+C は取らない（キャンバスの画素の写しに残す）
    h.event(Event::Copy);
    h.step();
    assert!(copied(&h).is_none());
}

#[test]
fn shift_and_ctrl_extend_the_selection() {
    let mut h = harness(sample(Lang::En), vec2(420.0, 220.0));
    let rows = row_labels(&h);
    let ids: Vec<u64> = log::visible_entries(h.state())
        .iter()
        .map(|e| e.id)
        .collect();
    log::click_row(h.state_mut(), ids[0], Modifiers::NONE);
    log::click_row(h.state_mut(), ids[2], Modifiers::SHIFT);
    assert_eq!(h.state().log_view.selected.len(), 3);
    log::click_row(h.state_mut(), ids[1], Modifiers::COMMAND);
    assert_eq!(h.state().log_view.selected.len(), 2);
    assert_eq!(
        log::copy_text(h.state(), true),
        [rows[0].clone(), rows[2].clone()].join("\n")
    );
}
