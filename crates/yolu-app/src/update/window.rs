//! 更新の窓 2 つ: 初回の問い（起動時に更新を確かめるか）と、ダウンロードが済んだあとの準備（更新して再起動するか）。
//! 窓には、名前・状態・短い理由だけを書く。説明はボタンのツールチップに置く。

use crate::state::{Action, AppState};
use crate::update::UpdateAction;
use crate::windows::{show_list, Button, ListSpec, Reply, Row};

/// 毎フレーム、開いている更新の窓を描き、押された操作を当てる。
pub fn show(ctx: &egui::Context, app: &mut AppState) {
    if !app.update.enabled() {
        return;
    }
    if app.update.is_asking() {
        ask(ctx, app);
    }
    if app.update.is_ready_open() {
        ready(ctx, app);
    }
}

/// 窓の名前（`windows::window_rect` で矩形を引く名前）。
pub const ASK: &str = "update-ask";
pub const READY: &str = "update-ready";

fn ask(ctx: &egui::Context, app: &mut AppState) {
    let lang = app.lang;
    let spec = ListSpec {
        id: ASK,
        title: lang.pick("更新の確認", "Update Check").into(),
        icon: "info",
        modal: true,
        width: 400.0,
        summary: Some((
            lang.pick(
                "起動時に更新を確かめますか？",
                "Check for updates at startup?",
            )
            .into(),
            false,
        )),
        rows: Vec::new(),
        buttons: vec![
            Button {
                label: lang.pick("いいえ", "No").into(),
                primary: false,
                tooltip: Some(
                    lang.pick(
                        "自動では確かめません。ヘルプのメニューから手で確かめられます",
                        "No automatic check. You can check from the Help menu",
                    )
                    .into(),
                ),
            },
            Button {
                label: lang.pick("はい", "Yes").into(),
                primary: true,
                tooltip: Some(
                    lang.pick(
                        "起動のたびに GitHub へ最新の版を問い合わせます。ほかの情報は送りません",
                        "Asks GitHub for the latest version at each startup. Nothing else is sent",
                    )
                    .into(),
                ),
            },
        ],
        close_label: lang.pick("ウィンドウを閉じる", "Close Window").into(),
    };
    let mut offset = app.update.ask_offset;
    let reply = show_list(ctx, &spec, &mut offset, &mut 0.0);
    app.update.ask_offset = offset;
    // 閉じる・Esc は「いいえ」（選ばずに通信はしない。ヘルプのメニューでいつでも変えられる）
    match reply {
        Some(Reply::Button(1)) => app.apply(Action::Update(UpdateAction::Answer(true))),
        Some(_) => app.apply(Action::Update(UpdateAction::Answer(false))),
        None => {}
    }
}

fn ready(ctx: &egui::Context, app: &mut AppState) {
    let lang = app.lang;
    let Some(ready) = app.update.ready() else {
        return;
    };
    let version = ready.version.clone();
    let modified = app.modified;
    let button = |label: &str, primary: bool, tooltip: Option<&str>| Button {
        label: label.into(),
        primary,
        tooltip: tooltip.map(Into::into),
    };
    let mut buttons = vec![button(lang.pick("あとで", "Later"), false, None)];
    if modified {
        buttons.push(button(
            lang.pick("保存せずに更新", "Update without saving"),
            false,
            Some(lang.pick(
                "保存せずにインストーラーを実行し、終わったらアプリを起こし直します",
                "Runs the installer without saving, then restarts the app",
            )),
        ));
        buttons.push(button(
            lang.pick("保存して更新", "Save and update"),
            true,
            Some(lang.pick(
                "保存してからインストーラーを実行し、終わったらアプリを起こし直します",
                "Saves, runs the installer, then restarts the app",
            )),
        ));
    } else {
        buttons.push(button(
            lang.pick("更新して再起動", "Update and restart"),
            true,
            Some(lang.pick(
                "インストーラーを実行し、終わったらアプリを起こし直します",
                "Runs the installer, then restarts the app",
            )),
        ));
    }
    let spec = ListSpec {
        id: READY,
        title: lang.pick("更新", "Update").into(),
        icon: "info",
        modal: true,
        width: 420.0,
        summary: Some((
            format!(
                "YoluPainter {version} — {}",
                lang.pick("ダウンロード済み", "Downloaded")
            ),
            false,
        )),
        rows: if modified {
            vec![Row::text(
                lang.pick("保存していない変更があります", "Unsaved changes"),
                true,
            )]
        } else {
            Vec::new()
        },
        buttons,
        close_label: lang.pick("ウィンドウを閉じる", "Close Window").into(),
    };
    let mut offset = app.update.ready_offset;
    let reply = show_list(ctx, &spec, &mut offset, &mut 0.0);
    app.update.ready_offset = offset;
    let run = |save| Action::Update(UpdateAction::Run { save });
    match (reply, modified) {
        (Some(Reply::Button(1)), _) => app.apply(run(false)),
        (Some(Reply::Button(2)), true) => app.apply(run(true)),
        (Some(_), _) => app.apply(Action::Update(UpdateAction::Later)),
        (None, _) => {}
    }
}
