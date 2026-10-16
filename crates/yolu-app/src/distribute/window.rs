//! 配布用に保存のウィンドウ（除く物を種類ごとに並べ、種類ごとに外せる）と、置き換えの確認のウィンドウ。ウィンドウには名前・状態・短い理由だけを書き、
//! 説明はツールチップに置く。0 件の種類は出さない。数・容量・説明の文は出さない。

use egui::{pos2, vec2, Id, Key, Rect};
use yolu_io::Removal;

use super::DistributeAction;
use crate::lang::Lang;
use crate::state::{Action, AppState};
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};
use crate::ui::window::{self, Spec};
use crate::windows::{show_list, Button, ListSpec, Reply, Row};

/// ウィンドウの名前（`windows::window_rect` で矩形を引く名前）。
pub const WINDOW: &str = "distribute";
pub const CONFIRM: &str = "distribute-replace";

const ROW_HEIGHT: f32 = 24.0;
const MAX_ROWS: usize = 18;
const FOOTER: f32 = 48.0;
/// 1 つの種類に並べる名前の上限（超えたら「…」）。
const MAX_NAMES: usize = 40;

/// 種類の名前。
pub fn label(lang: Lang, removal: Removal) -> &'static str {
    match removal {
        Removal::PsdOriginals => lang.t("distribute-original-psds"),
        Removal::UnusedShelf => lang.t("distribute-unused-assets"),
        Removal::SourcePaths => lang.t("distribute-asset-source-paths"),
        Removal::ModelReference => lang.t("distribute-model-reference"),
        Removal::MeshMaps => lang.t("distribute-mesh-maps"),
        Removal::UnityValues => lang.t("distribute-unity-material-values"),
        Removal::SavedSelections => lang.t("distribute-remembered-selections"),
        Removal::StaleEntries => lang.t("distribute-old-thumbnail-and-state"),
        Removal::UnknownEntries => lang.t("distribute-unknown-entries"),
    }
}

/// 種類のツールチップ（なぜ除くか。説明はここに置く）。
pub fn tooltip(lang: Lang, removal: Removal) -> &'static str {
    match removal {
        Removal::PsdOriginals => lang.t("distribute-the-original-bytes-of-imported-tip"),
        Removal::UnusedShelf => lang.t("distribute-project-assets-images-smart-assets-tip"),
        Removal::SourcePaths => lang.t("distribute-where-the-assets-were-imported-tip"),
        Removal::ModelReference => lang.t("distribute-the-location-of-the-model-tip"),
        Removal::MeshMaps => lang.t("distribute-maps-baked-from-the-model-tip"),
        Removal::UnityValues => lang.t("distribute-values-received-from-unity-materials-tip"),
        Removal::SavedSelections => lang.t("distribute-selections-saved-under-a-name-tip"),
        Removal::StaleEntries => lang.t("distribute-thumbnail-brush-settings-and-similar-tip"),
        Removal::UnknownEntries => lang.t("distribute-entries-this-app-does-not-tip"),
    }
}

/// 「保存…」のツールチップ。
fn save_tooltip(lang: Lang) -> &'static str {
    lang.t("distribute-choose-where-to-save-a-tip")
}

/// ウィンドウの 1 行。
enum Item {
    /// 種類（切り替え）。
    Kind(Removal, bool),
    /// その種類に当たる物の名前。
    Name(String),
    /// 名前が多いときの続き。
    More,
}

// ───────── ウィンドウ ─────────

/// 毎フレーム、開いているウィンドウを描き、押された操作を当てる。
pub fn show(ctx: &egui::Context, app: &mut AppState) {
    if !app.distribute.window_visible() {
        return;
    }
    let lang = app.lang;
    let Some(window) = app.distribute.window() else {
        return;
    };
    let mut items: Vec<Item> = Vec::new();
    for found in &window.inventory().found {
        items.push(Item::Kind(
            found.removal,
            window.selected().contains(&found.removal),
        ));
        for name in found.names.iter().take(MAX_NAMES) {
            items.push(Item::Name(name.clone()));
        }
        if found.names.len() > MAX_NAMES {
            items.push(Item::More);
        }
    }
    let visible = items.len().min(MAX_ROWS);
    let height = window::HEADER_HEIGHT + 8.0 + visible as f32 * ROW_HEIGHT + 10.0 + FOOTER;
    let title = lang.t("distribute-save-for-distribution");
    let spec = Spec {
        title,
        icon: Some("save"),
        size: vec2(420.0, height),
        modal: true,
        close_label: lang.t("distribute-close-window"),
    };
    let id = Id::new(("yolu.window", WINDOW));
    let mut offset = app.ui.distribute_window_offset;
    let mut scroll = app.distribute.window_scroll;
    let mut actions: Vec<DistributeAction> = Vec::new();
    let mut esc = false;
    let closed = window::show(ctx, id, &spec, &mut offset, false, |ui, frame| {
        esc = ui.input(|i| i.key_pressed(Key::Escape));
        let body = frame.body;
        let list = Rect::from_min_size(
            pos2(body.left(), body.top() + 8.0),
            vec2(body.width(), visible as f32 * ROW_HEIGHT),
        );
        let content = items.len() as f32 * ROW_HEIGHT;
        let bar = Scroll::begin(ui, list, content, &mut scroll);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list));
        child.set_clip_rect(list.intersect(ui.clip_rect()));
        for (i, item) in items.iter().enumerate() {
            let r = Rect::from_min_size(
                pos2(
                    list.left() + 14.0,
                    list.top() + i as f32 * ROW_HEIGHT - scroll,
                ),
                vec2(list.width() - 28.0 - bar.reserved(), ROW_HEIGHT),
            );
            if r.bottom() < list.top() || r.top() > list.bottom() {
                continue;
            }
            match item {
                Item::Kind(removal, on) => {
                    let next = w::toggle(
                        &mut child,
                        r,
                        ("removal", i),
                        label(lang, *removal),
                        *on,
                        Some(tooltip(lang, *removal)),
                        true,
                    );
                    if next != *on {
                        actions.push(DistributeAction::Toggle(*removal));
                    }
                }
                Item::Name(name) => {
                    let p = child.painter().clone();
                    let r = Rect::from_min_max(pos2(r.left() + 23.0, r.top()), r.max);
                    w::text(&p, r, name, t::LABEL_DIM, Align::Left);
                }
                Item::More => {
                    let p = child.painter().clone();
                    let r = Rect::from_min_max(pos2(r.left() + 23.0, r.top()), r.max);
                    w::text(&p, r, "…", t::LABEL_DIM, Align::Left);
                }
            }
        }
        bar.end(ui, id.with("list-scroll"), &mut scroll);
        // 下の帯
        let p = ui.painter().clone();
        let footer = Rect::from_min_max(pos2(body.left(), body.bottom() - FOOTER), body.max);
        w::fill(&p, footer, t::PANEL_HEADER);
        w::hline(&p, footer.left(), footer.right(), footer.top(), t::BORDER);
        let mut x = footer.right() - 14.0;
        let buttons = [
            (
                lang.t("distribute-cancel"),
                false,
                None,
                DistributeAction::CancelWindow,
            ),
            (
                lang.t("distribute-save"),
                true,
                Some(save_tooltip(lang)),
                DistributeAction::ChooseFile,
            ),
        ];
        for (i, (label, primary, tip, action)) in buttons.into_iter().enumerate().rev() {
            let bw = w::text_width(&p, label, t::LABEL) + 32.0;
            let r = Rect::from_min_size(pos2(x - bw, footer.top() + 10.0), vec2(bw, 28.0));
            x = r.left() - 8.0;
            if w::button(
                ui,
                r,
                id.with(("button", i)),
                label,
                primary,
                true,
                tip,
                None,
            )
            .clicked()
            {
                actions.push(action);
            }
        }
    });
    app.ui.distribute_window_offset = offset;
    app.distribute.window_scroll = scroll;
    if (closed || esc) && actions.is_empty() {
        actions.push(DistributeAction::CancelWindow);
    }
    for action in actions {
        app.apply(Action::Distribute(action));
    }
}

// ───────── 置き換えの確かめ ─────────

/// 毎フレーム、置き換えの確認のウィンドウを描き、押された操作を当てる（既にあるファイルを選んだとき）。
pub fn show_replace(ctx: &egui::Context, app: &mut AppState) {
    let Some(path) = app.distribute.replace.clone() else {
        return;
    };
    let lang = app.lang;
    let spec = ListSpec {
        id: CONFIRM,
        title: lang.t("distribute-file-to-replace").into(),
        icon: "warning",
        modal: true,
        width: 460.0,
        summary: Some((
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            true,
        )),
        rows: vec![Row::text(path.display().to_string(), false)],
        buttons: vec![
            Button {
                label: lang.t("distribute-cancel").into(),
                primary: false,
                tooltip: None,
            },
            Button {
                label: lang.t("distribute-replace").into(),
                primary: true,
                tooltip: Some(lang.t("distribute-replaces-the-file-with-the-tip").into()),
            },
        ],
        close_label: lang.t("distribute-close-window").into(),
    };
    let mut offset = app.ui.distribute_replace_offset;
    let mut scroll = 0.0;
    let reply = show_list(ctx, &spec, &mut offset, &mut scroll);
    app.ui.distribute_replace_offset = offset;
    match reply {
        Some(Reply::Button(1)) => app.apply(Action::Distribute(DistributeAction::ConfirmReplace)),
        Some(_) => app.apply(Action::Distribute(DistributeAction::CancelReplace)),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ツールチップは、読み手を実際にある版（0.3.x のスタンドアロン版・0.4.x までの Unity 版）で言い、どの種類にも日英がある。
    /// 「覚えた選択範囲」は形式 7 に戻ること、「モデルの参照」は Live Link で開いたモデルの記録も除くことを言う。
    #[test]
    fn the_tooltips_name_the_readers_and_what_each_kind_takes_out() {
        for lang in [Lang::Ja, Lang::En] {
            for removal in Removal::ALL {
                assert!(!tooltip(lang, removal).is_empty(), "{removal:?}");
                assert!(!label(lang, removal).is_empty(), "{removal:?}");
            }
        }
        let saved = tooltip(Lang::Ja, Removal::SavedSelections);
        assert!(saved.contains("0.4.x") && saved.contains(" 7 "), "{saved}");
        let saved = tooltip(Lang::En, Removal::SavedSelections);
        assert!(
            saved.contains("0.4.x") && saved.contains("format 7"),
            "{saved}"
        );
        assert!(tooltip(Lang::Ja, Removal::SourcePaths).contains("フォント"));
        assert!(tooltip(Lang::En, Removal::SourcePaths).contains("font"));
        assert!(tooltip(Lang::Ja, Removal::ModelReference).contains("Live Link"));
        assert!(tooltip(Lang::En, Removal::ModelReference).contains("Live Link"));
    }
}
