//! 設定の窓（退避を残す数）。選んだ値は `AppState::prefs` に入り、保存（`project::save_from`）が読み、設定のファイル
//! （`settings`）へは次のフレームで書かれる。窓は浮いた窓の骨組み（`ui::window`）で、見出しをドラッグして動かせる。

use egui::{pos2, vec2, Id, Rect, Vec2};
use yolu_io::{BackupKeep, MAX_BACKUPS_TO_KEEP};

use crate::state::{Action, AppState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, SliderSpec};
use crate::ui::window::{self, Spec};

const WIDTH: f32 = 320.0;
/// 「すべて残す」を切ったときにスライダーが戻る数（まだ数を選んでいないとき）。
const DEFAULT_COUNT: u32 = 10;

fn window_id() -> Id {
    Id::new("yolu.prefs")
}

/// 最後に描いた窓の矩形（画面の点。閉じても残り、一度も開いていなければ None）。試験が位置を知るために読む。
pub fn last_rect(ctx: &egui::Context) -> Option<Rect> {
    window::last_rect(ctx, window_id())
}

/// 設定の窓の操作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefsAction {
    Open,
    Close,
    /// 退避を残す数を選ぶ（上限を超える数は上限にする）。
    SetBackups(BackupKeep),
}

/// 設定の窓の状態と、いま選んでいる設定。
#[derive(Debug)]
pub struct PrefsState {
    /// 上書き保存で置き換えた前の版（退避）をいくつ残すか。
    pub backups: BackupKeep,
    pub open: bool,
    /// 退避の数のスライダーをドラッグしている最中。設定のファイルへは、離すまで退避の数を書かない（ドラッグの間じゅう
    /// フレームごとに同期付きの書き込みをしない。Esc で戻した値は、書いてある値と同じなので書かない）。
    pub dragging: bool,
    offset: Vec2,
    /// 「すべて残す」のあいだスライダーに見せる数（最後に選んだ数）。
    remembered: u32,
}

impl Default for PrefsState {
    fn default() -> Self {
        Self {
            backups: BackupKeep::All,
            open: false,
            dragging: false,
            offset: Vec2::ZERO,
            remembered: DEFAULT_COUNT,
        }
    }
}

impl PrefsState {
    /// スライダーに出す数。
    fn shown_count(&self) -> u32 {
        match self.backups {
            BackupKeep::Count(n) => n,
            BackupKeep::All => self.remembered,
        }
    }
}

impl AppState {
    pub fn prefs_apply(&mut self, action: PrefsAction) {
        match action {
            PrefsAction::Open => self.prefs.open = true,
            PrefsAction::Close => {
                self.prefs.open = false;
                self.prefs.dragging = false;
            }
            PrefsAction::SetBackups(keep) => {
                let keep = match keep {
                    BackupKeep::Count(n) => BackupKeep::Count(n.min(MAX_BACKUPS_TO_KEEP)),
                    all => all,
                };
                if let BackupKeep::Count(n) = keep {
                    self.prefs.remembered = n;
                }
                self.prefs.backups = keep;
            }
        }
    }
}

/// 開いていれば窓を描き、選んだ値を `Action` として当てる。
pub fn show(ctx: &egui::Context, app: &mut AppState) {
    if !app.prefs.open {
        app.prefs.dragging = false;
        return;
    }
    let lang = app.lang;
    let all = app.prefs.backups == BackupKeep::All;
    let count = app.prefs.shown_count();
    let height = window::HEADER_HEIGHT + 14.0 + t::SLIDER_ROW_HEIGHT + 8.0 + 22.0 + 14.0;
    let spec = Spec {
        title: lang.pick("設定", "Settings"),
        icon: Some("tune"),
        size: vec2(WIDTH, height),
        modal: false,
        close_label: lang.pick("閉じる", "Close"),
    };
    let id = window_id();
    let mut offset = app.prefs.offset;
    let mut chosen = None;
    let mut dragging = false;
    let closed = window::show(ctx, id, &spec, &mut offset, false, |ui, frame| {
        let left = frame.body.left() + 14.0;
        let width = frame.body.width() - 28.0;
        let top = frame.body.top() + 14.0;
        let slider_rect = Rect::from_min_size(pos2(left, top), vec2(width, t::SLIDER_ROW_HEIGHT));
        let slider = SliderSpec::new(
            lang.pick("退避を残す数", "Backups to keep"),
            0.0,
            MAX_BACKUPS_TO_KEEP as f32,
            NumberFormat::int(""),
        )
        .tooltip(lang.pick(
            "上書き保存で置き換えた前の版を、新しい順にいくつ残すか。超えた古い版は消します。0 は残しません",
            "How many replaced versions to keep per file, newest first. Older ones beyond this are deleted. 0 keeps none",
        ))
        .enabled(!all);
        let out = w::slider(ui, slider_rect, id.with("count"), count as f32, &slider);
        dragging = out.active;
        if out.changed {
            let n = out.value.round().clamp(0.0, MAX_BACKUPS_TO_KEEP as f32) as u32;
            chosen = Some(BackupKeep::Count(n));
        }
        let row = Rect::from_min_size(
            pos2(left, top + t::SLIDER_ROW_HEIGHT + 8.0),
            vec2(width, 22.0),
        );
        let next = w::toggle(
            ui,
            row,
            id.with("all"),
            lang.pick("すべて残す", "Keep all"),
            all,
            Some(lang.pick("退避を消さない", "Never delete backups")),
            true,
        );
        if next != all {
            chosen = Some(if next { BackupKeep::All } else { BackupKeep::Count(count) });
        }
    });
    app.prefs.offset = offset;
    app.prefs.dragging = dragging;
    if let Some(keep) = chosen {
        app.apply(Action::Prefs(PrefsAction::SetBackups(keep)));
    }
    if closed {
        app.apply(Action::Prefs(PrefsAction::Close));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choosing_clamps_to_the_limit_and_remembers_the_count_while_keeping_all() {
        let mut app = AppState::new(8, 8);
        assert_eq!(app.prefs.backups, BackupKeep::All);
        assert_eq!(app.prefs.shown_count(), DEFAULT_COUNT);
        app.prefs_apply(PrefsAction::SetBackups(BackupKeep::Count(5000)));
        assert_eq!(app.prefs.backups, BackupKeep::Count(MAX_BACKUPS_TO_KEEP));
        app.prefs_apply(PrefsAction::SetBackups(BackupKeep::Count(3)));
        app.prefs_apply(PrefsAction::SetBackups(BackupKeep::All));
        assert_eq!(app.prefs.backups, BackupKeep::All);
        assert_eq!(app.prefs.shown_count(), 3, "すべて残す間も、最後に選んだ数をスライダーに見せる");
        app.prefs_apply(PrefsAction::SetBackups(BackupKeep::Count(0)));
        assert_eq!(app.prefs.backups, BackupKeep::Count(0));
        app.prefs_apply(PrefsAction::Open);
        assert!(app.prefs.open);
        app.prefs_apply(PrefsAction::Close);
        assert!(!app.prefs.open);
    }
}
