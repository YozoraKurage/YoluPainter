//! 更新のウィンドウ 2 つ: 初回の問い（起動時に更新を確かめるか）と、ダウンロードが済んだあとの準備（更新して再起動するか）。
//! ウィンドウには、名前・状態・短い理由だけを書く。説明はボタンのツールチップに置く。

use crate::state::{Action, AppState};
use crate::update::UpdateAction;
use crate::windows::{show_list, Button, ListSpec, Reply, Row};

/// 毎フレーム、開いている更新のウィンドウを描き、押された操作を当てる。
pub fn show(ctx: &egui::Context, app: &mut AppState) {
    if !app.services.update.enabled() {
        return;
    }
    if app.services.update.is_asking() {
        ask(ctx, app);
    }
    if app.services.update.is_ready_open() {
        ready(ctx, app);
    }
}

/// ウィンドウの名前（`windows::window_rect` で矩形を引く名前）。
pub const ASK: &str = "update-ask";
pub const READY: &str = "update-ready";

fn ask(ctx: &egui::Context, app: &mut AppState) {
    let lang = app.lang;
    let spec = ListSpec {
        id: ASK,
        title: lang.t("update-update-check").into(),
        icon: "info",
        modal: true,
        width: 400.0,
        summary: Some((lang.t("update-check-for-updates-at-startup").into(), false)),
        rows: Vec::new(),
        buttons: vec![
            Button {
                label: lang.t("update-no").into(),
                primary: false,
                tooltip: Some(lang.t("update-no-automatic-check-you-can-tip").into()),
            },
            Button {
                label: lang.t("update-yes").into(),
                primary: true,
                tooltip: Some(lang.t("update-asks-github-for-the-latest-tip").into()),
            },
        ],
        close_label: lang.t("update-close-window").into(),
    };
    let mut offset = app.ui.update_ask_offset;
    let reply = show_list(ctx, &spec, &mut offset, &mut 0.0);
    app.ui.update_ask_offset = offset;
    // 閉じる・Esc は「いいえ」（選ばずに通信はしない。ヘルプのメニューでいつでも変えられる）
    match reply {
        Some(Reply::Button(1)) => app.apply(Action::Update(UpdateAction::Answer(true))),
        Some(_) => app.apply(Action::Update(UpdateAction::Answer(false))),
        None => {}
    }
}

fn ready(ctx: &egui::Context, app: &mut AppState) {
    let lang = app.lang;
    let Some(ready) = app.services.update.ready() else {
        return;
    };
    let version = ready.version.clone();
    // 保存の途中は、保存の頼みが下ろした「変更あり」の印ではなく、頼む前の印で見せる（保存が失敗すれば変更は残る）
    let modified = app.shows_modified();
    let blocked = app.services.update.ready_blocked;
    let button = |label: &str, primary: bool, tooltip: Option<&str>| Button {
        label: label.into(),
        primary,
        tooltip: tooltip.map(Into::into),
    };
    let mut buttons = vec![button(lang.t("update-later-tip"), false, None)];
    if modified {
        buttons.push(button(
            lang.t("update-update-without-saving-tip"),
            false,
            Some(lang.t("update-runs-the-installer-without-saving-tip")),
        ));
        buttons.push(button(
            lang.t("update-save-and-update-tip"),
            true,
            Some(lang.t("update-saves-runs-the-installer-then-tip")),
        ));
    } else {
        buttons.push(button(
            lang.t("update-update-and-restart-tip"),
            true,
            Some(lang.t("update-runs-the-installer-then-restarts-tip")),
        ));
    }
    let spec = ListSpec {
        id: READY,
        title: lang.t("update-update-tip").into(),
        icon: "info",
        modal: true,
        width: 420.0,
        summary: Some((
            format!(
                "YoluPainter {version} — {}",
                lang.t("update-downloaded-tip")
            ),
            false,
        )),
        rows: {
            let mut rows = Vec::new();
            if modified {
                rows.push(Row::text(lang.t("update-unsaved-changes-tip"), true));
            }
            if blocked {
                rows.push(Row::text(super::blocked_text(lang), true));
            }
            rows
        },
        buttons,
        close_label: lang.t("update-close-window-tip").into(),
    };
    let mut offset = app.ui.update_ready_offset;
    let reply = show_list(ctx, &spec, &mut offset, &mut 0.0);
    app.ui.update_ready_offset = offset;
    let run = |save| Action::Update(UpdateAction::Run { save });
    match (reply, modified) {
        (Some(Reply::Button(1)), _) => app.apply(run(false)),
        (Some(Reply::Button(2)), true) => app.apply(run(true)),
        (Some(_), _) => app.apply(Action::Update(UpdateAction::Later)),
        (None, _) => {}
    }
}
