//! 配布用に保存の窓（除く物を種類ごとに並べ、種類ごとに外せる）と、置き換えの確かめの窓。窓には名前・状態・短い理由だけを書き、
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

/// 窓の名前（`windows::window_rect` で矩形を引く名前）。
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
        Removal::PsdOriginals => lang.pick("PSD の原本", "Original PSDs"),
        Removal::UnusedShelf => lang.pick("使っていないアセット", "Unused assets"),
        Removal::SourcePaths => lang.pick("素材の出どころのパス", "Asset source paths"),
        Removal::ModelReference => lang.pick("モデルの参照", "Model reference"),
        Removal::MeshMaps => lang.pick("メッシュマップ", "Mesh maps"),
        Removal::UnityValues => lang.pick("Unity のマテリアルの値", "Unity material values"),
        Removal::SavedSelections => lang.pick("覚えた選択範囲", "Remembered selections"),
        Removal::StaleEntries => lang.pick("古いサムネイルなど", "Old thumbnail and state"),
        Removal::UnknownEntries => lang.pick("知らないエントリ", "Unknown entries"),
    }
}

/// 種類のツールチップ（なぜ除くか。説明はここに置く）。
pub fn tooltip(lang: Lang, removal: Removal) -> &'static str {
    match removal {
        Removal::PsdOriginals => lang.pick(
            "取り込んだ PSD の原本のバイト列。写しには入れません",
            "The original bytes of imported PSDs. They are left out of the copy",
        ),
        Removal::UnusedShelf => lang.pick(
            "どの層からも、見た目の設定からも使われていないアセット（画像・スマート素材・ブラシ）。使っているアセットは残します",
            "Project assets (images, smart assets and brushes) that no layer or look setting uses. Assets in use stay",
        ),
        Removal::SourcePaths => lang.pick(
            "素材を取り込んだ元のファイルやフォルダーの場所。使っている素材は残し、出どころだけを外します",
            "Where the assets were imported from. Assets in use stay; only their source is dropped",
        ),
        Removal::ModelReference => lang.pick(
            "開いていたモデルの場所と Unity のモデルの GUID、モデルのポーズ",
            "The location of the model that was open, the Unity model GUID and the model's pose",
        ),
        Removal::MeshMaps => lang.pick(
            "モデルの形から焼いたマップ。Generator が使うマップは、開いたあとにモデルから焼き直します",
            "Maps baked from the model's shape. Maps that generators use are baked again from the model after opening",
        ),
        Removal::UnityValues => lang.pick(
            "Live Link で Unity のマテリアルから受けた値。見た目の設定は残ります",
            "Values received from Unity materials over Live Link. Look settings stay",
        ),
        Removal::SavedSelections => lang.pick(
            "名前を付けて残した選択範囲。除くと、写しは Unity 版でも開ける形式になります（今の選択範囲は残ります）",
            "Selections saved under a name. Without them the copy can be opened by the Unity version too (the current selection stays)",
        ),
        Removal::StaleEntries => lang.pick(
            "Unity 版が残したサムネイル・ブラシの設定など。スタンドアロン版は更新しません",
            "Thumbnail, brush settings and similar entries left by the Unity version. The standalone version does not update them",
        ),
        Removal::UnknownEntries => lang.pick(
            "このアプリが知らないエントリ（新しい版が書いたものなど）",
            "Entries this app does not know (for example written by a newer version)",
        ),
    }
}

/// 「保存…」のツールチップ。
fn save_tooltip(lang: Lang) -> &'static str {
    lang.pick(
        "保存先を選びます。開いている作業用のファイルとは別の名前で、チェックした物を除いた写しを書きます",
        "Choose where to save. A copy without the checked things is written under a name other than the open working file",
    )
}

/// 窓の 1 行。
enum Item {
    /// 種類（切り替え）。
    Kind(Removal, bool),
    /// その種類に当たる物の名前。
    Name(String),
    /// 名前が多いときの続き。
    More,
}

// ───────── 窓 ─────────

/// 毎フレーム、開いている窓を描き、押された操作を当てる。
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
    let title = lang.pick("配布用に保存", "Save for Distribution");
    let spec = Spec {
        title,
        icon: Some("save"),
        size: vec2(420.0, height),
        modal: true,
        close_label: lang.pick("ウィンドウを閉じる", "Close Window"),
    };
    let id = Id::new(("yolu.window", WINDOW));
    let mut offset = app.distribute.window_offset;
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
                lang.pick("やめる", "Cancel"),
                false,
                None,
                DistributeAction::CancelWindow,
            ),
            (
                lang.pick("保存…", "Save…"),
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
    app.distribute.window_offset = offset;
    app.distribute.window_scroll = scroll;
    if (closed || esc) && actions.is_empty() {
        actions.push(DistributeAction::CancelWindow);
    }
    for action in actions {
        app.apply(Action::Distribute(action));
    }
}

// ───────── 置き換えの確かめ ─────────

/// 毎フレーム、置き換えの確かめの窓を描き、押された操作を当てる（既にあるファイルを選んだとき）。
pub fn show_replace(ctx: &egui::Context, app: &mut AppState) {
    let Some(path) = app.distribute.replace.clone() else {
        return;
    };
    let lang = app.lang;
    let spec = ListSpec {
        id: CONFIRM,
        title: lang
            .pick("置き換えるファイル", "File to Replace")
            .into(),
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
                label: lang.pick("やめる", "Cancel").into(),
                primary: false,
                tooltip: None,
            },
            Button {
                label: lang.pick("置き換える", "Replace").into(),
                primary: true,
                tooltip: Some(
                    lang.pick(
                        "同じ名前のファイルを、書き出す写しに置き換えます。前のファイルは残しません",
                        "Replaces the file with the copy. The previous file is not kept",
                    )
                    .into(),
                ),
            },
        ],
        close_label: lang.pick("ウィンドウを閉じる", "Close Window").into(),
    };
    let mut offset = app.distribute.replace_offset;
    let mut scroll = 0.0;
    let reply = show_list(ctx, &spec, &mut offset, &mut scroll);
    app.distribute.replace_offset = offset;
    match reply {
        Some(Reply::Button(1)) => app.apply(Action::Distribute(DistributeAction::ConfirmReplace)),
        Some(_) => app.apply(Action::Distribute(DistributeAction::CancelReplace)),
        None => {}
    }
}
