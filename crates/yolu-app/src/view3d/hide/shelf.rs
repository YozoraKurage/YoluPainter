//! 3D ビューの左下のシェルフ（ペイント・ハイド・ポーズのモードで、ビューごとに 1 つ。動かせない・持ち手なし）: たたんだ形は目と ▽ だけ。
//! 目はそのビューの隠し方を当てる・外す（ハイドのモードの隠し方と同じ物。閉じれば何も隠さない）、▽ は上へ開いて、ハイドのプリセットと
//! ポーズのプリセットを選ぶ 2 つのドロップダウンを出す。どちらもそのビューだけに効き、保存しない。
//!
//! 重ねる部品は別のレイヤー（`egui::Area` の `Order::Middle`）に置くので、シェルフの上の押しは下のビューの入力（描く・回す）を始めない。

use std::sync::Arc;

use egui::{pos2, vec2, Color32, Order, Rect, Sense, Ui};

use super::{HideAction, PoseChoice, ViewPose};
use crate::lang::Lang;
use crate::mode::{EditorMode, ModeAction};
use crate::notice::Source;
use crate::state::{Action, AppState, OpenPopup, PopupKind};
use crate::ui::menu::{Entry, PopupState};
use crate::ui::theme as t;
use crate::ui::widgets as w;
use crate::view3d::ViewId;

const PILL_HEIGHT: f32 = 26.0;
const EYE_WIDTH: f32 = 30.0;
const ARROW_WIDTH: f32 = 22.0;
const BODY_WIDTH: f32 = 196.0;
const BODY_ROW: f32 = 26.0;
/// ビューの左下の隅からの間。
const MARGIN: f32 = 10.0;

/// シェルフを出すモードか（編集のモードでは出さない）。
pub fn shown_in(mode: EditorMode) -> bool {
    matches!(
        mode,
        EditorMode::Paint | EditorMode::Hide | EditorMode::Pose
    )
}

/// シェルフの矩形（たたんだ形と、開いていれば上の欄）。
pub fn rects(view: Rect, open: bool) -> (Rect, Option<Rect>) {
    let pill = Rect::from_min_size(
        pos2(view.left() + MARGIN, view.bottom() - MARGIN - PILL_HEIGHT),
        vec2(EYE_WIDTH + ARROW_WIDTH, PILL_HEIGHT),
    );
    let body = open.then(|| {
        let h = 2.0 * (BODY_ROW + 4.0) + 4.0;
        Rect::from_min_size(pos2(pill.left(), pill.top() - 4.0 - h), vec2(BODY_WIDTH, h))
    });
    (pill, body)
}

/// このビューのハイドのプリセットのドロップダウンの文字（何も隠さない・プリセット 1 つ・ほかの隠し方）。
fn hide_label(app: &AppState, id: ViewId) -> String {
    let lang = app.lang;
    let Some(h) = app.view3d.view_of(id).map(|v| &v.hide) else {
        return String::new();
    };
    if h.is_clear() {
        return lang.t("hide-show-everything").to_owned();
    }
    match h.bones.presets.as_slice() {
        [one] if h.islands.is_empty() && h.bones.entries.is_empty() => app
            .view3d
            .pose
            .hide_presets
            .get(*one)
            .map(|p| p.name.clone())
            .unwrap_or_default(),
        _ => lang.t("hide-hidden").to_owned(),
    }
}

/// このビューのポーズのドロップダウンの文字。
fn pose_label(app: &AppState, id: ViewId) -> String {
    let lang = app.lang;
    match app.view3d.view_of(id).map(|v| &v.pose) {
        Some(ViewPose::Rest(_)) => lang.t("hide-rest-pose").to_owned(),
        Some(ViewPose::Preset(p, _)) => app
            .view3d
            .pose
            .pose_presets
            .get(*p)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| lang.t("hide-pose-preset").to_owned()),
        _ => lang.t("hide-current-pose").to_owned(),
    }
}

/// 左下のたたんだ形の部品の置き場と今の状態（目・▽ の矩形、隠し方を当てているか、上の欄が開いているか）。
#[derive(Clone, Copy)]
struct Pill {
    id: ViewId,
    pill: Rect,
    eye: Rect,
    arrow: Rect,
    applied: bool,
    open: bool,
}

/// 上の欄の 1 行（ドロップダウンの前のアイコン・今の値・ツールチップ・開くメニュー・開いているか・押せるか）。
struct BodyRow<'a> {
    icon: &'static str,
    value: &'a str,
    tip: &'a str,
    kind: PopupKind,
    is_open: bool,
    enabled: bool,
}

/// シェルフを描く（`live` は今のビュー。今のビューでないビューのシェルフは見せるだけで、押せばそのビューが今のビューになる）。
pub fn show(ui: &mut Ui, app: &mut AppState, view: Rect, live: bool) {
    if !shown_in(app.mode) || app.view3d.full_model().is_none() {
        return;
    }
    let lang = app.lang;
    let id = app.view3d.active_view();
    let (applied, open) = {
        let h = &app.view3d.view.hide;
        (h.applied, h.shelf_open)
    };
    let (pill, body) = rects(view, open);
    let eye = Rect::from_min_size(pill.min, vec2(EYE_WIDTH, PILL_HEIGHT));
    let arrow = Rect::from_min_max(pos2(eye.right(), pill.top()), pill.max);
    let geometry = Pill {
        id,
        pill,
        eye,
        arrow,
        applied,
        open,
    };
    let has_rig = app.view3d.pose.session.is_some();
    let hide_text = hide_label(app, id);
    let pose_text = pose_label(app, id);
    let menu_open = |kind: PopupKind| app.ui.popup.as_ref().is_some_and(|p| p.kind == kind);
    let rows = [
        BodyRow {
            icon: "visibility_off",
            value: &hide_text,
            tip: lang.t("hide-hide-preset"),
            kind: PopupKind::HidePreset(id),
            is_open: menu_open(PopupKind::HidePreset(id)),
            enabled: true,
        },
        BodyRow {
            icon: "accessibility",
            value: &pose_text,
            tip: lang.t("hide-pose-preset"),
            kind: PopupKind::PosePreset(id),
            is_open: menu_open(PopupKind::PosePreset(id)),
            enabled: has_rig,
        },
    ];
    let mut action = None;
    let mut menu: Option<(PopupKind, Rect)> = None;
    let full = body.map_or(pill, |b| b.union(pill));
    egui::Area::new(egui::Id::new(("yolu.view3d.shelf", id.0)))
        .order(Order::Middle)
        .fixed_pos(full.min)
        .constrain(false)
        .show(ui.ctx(), |ui| {
            // シェルフの矩形の全部を取る（部品の間の押しも、下のビューに落とさない）
            ui.allocate_exact_size(full.size(), Sense::hover());
            let _ = ui.interact(full, ui.id().with("shelf.block"), Sense::click_and_drag());
            action = draw_pill(ui, lang, &geometry);
            if let Some(body) = body {
                menu = draw_body(ui, id, body, &rows);
            }
        });
    if !live {
        return;
    }
    if let Some(a) = action {
        app.apply(Action::Hide(a));
    }
    if let Some((kind, b)) = menu {
        // 押した箱の上に開く（シェルフはビューの下の端にあるので、下には入りきらない）
        let ctx = ui.ctx().clone();
        let anchor = Rect::from_min_size(pos2(b.left(), b.bottom() + 2.0), vec2(b.width(), 0.0));
        app.ui.popup = Some(OpenPopup {
            kind,
            state: PopupState::new(&ctx, anchor).with_min_width(b.width()),
        });
    }
}

/// たたんだ形（目と ▽）を描き、押された方の操作を返す。
fn draw_pill(ui: &mut Ui, lang: Lang, g: &Pill) -> Option<HideAction> {
    let Pill {
        id,
        pill,
        eye,
        arrow,
        applied,
        open,
    } = *g;
    let p = ui.painter().clone();
    w::rounded(
        &p,
        pill,
        if applied {
            Color32::from_black_alpha(120)
        } else {
            Color32::from_black_alpha(200)
        },
        6.0,
    );
    let eye_r = ui.interact(eye, ui.id().with(("shelf.eye", id.0)), Sense::click());
    let arrow_r = ui.interact(arrow, ui.id().with(("shelf.arrow", id.0)), Sense::click());
    let corners = |nw: u8, ne: u8| egui::CornerRadius {
        nw,
        sw: nw,
        ne,
        se: ne,
    };
    for (part, resp, c, lit) in [
        (eye, &eye_r, corners(6, 0), false),
        (arrow, &arrow_r, corners(0, 6), open),
    ] {
        let bg = if lit {
            Some(t::ACCENT_DIM)
        } else if resp.is_pointer_button_down_on() {
            Some(Color32::from_black_alpha(220))
        } else if resp.hovered() {
            Some(Color32::from_black_alpha(120))
        } else {
            None
        };
        if let Some(bg) = bg {
            p.rect_filled(part, c, bg);
        }
    }
    let fg = if applied { t::TEXT } else { t::TEXT_DIM };
    let hot = |r: &egui::Response| if r.hovered() { Color32::WHITE } else { fg };
    let eye_icon = if applied {
        "visibility"
    } else {
        "visibility_off"
    };
    w::icon(&p, eye, eye_icon, hot(&eye_r), 16.0);
    w::vline(
        &p,
        arrow.left(),
        pill.top() + 5.0,
        pill.bottom() - 5.0,
        Color32::from_white_alpha(50),
    );
    let arrow_color = if open || arrow_r.hovered() {
        Color32::WHITE
    } else {
        fg
    };
    w::icon(&p, arrow, "arrow_drop_down", arrow_color, 18.0);
    eye_r.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Checkbox,
            true,
            applied,
            lang.t("hide-hide-in-this-view-tip"),
        )
    });
    arrow_r.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            lang.t("hide-choose-presets-tip"),
        )
    });
    let mut action = None;
    if eye_r
        .on_hover_text(lang.t("hide-hide-in-this-view-tip"))
        .clicked()
    {
        action = Some(HideAction::ToggleApplied(id));
    }
    if arrow_r
        .on_hover_text(lang.t("hide-choose-presets-tip"))
        .clicked()
    {
        action = Some(HideAction::ToggleShelf(id));
    }
    action
}

/// 上に開いた欄（ハイドのプリセットとポーズのプリセットのドロップダウン）を描き、押された箱があればそのメニューの種類と箱の矩形を返す。
fn draw_body(ui: &mut Ui, id: ViewId, body: Rect, rows: &[BodyRow]) -> Option<(PopupKind, Rect)> {
    let p = ui.painter().clone();
    w::rounded(&p, body, Color32::from_black_alpha(170), 6.0);
    w::outline(&p, body, t::BORDER, 1.0, 6.0);
    let mut menu = None;
    for (k, row) in rows.iter().enumerate() {
        let y = body.top() + 4.0 + k as f32 * (BODY_ROW + 4.0);
        w::icon(
            &p,
            Rect::from_min_size(pos2(body.left() + 6.0, y), vec2(18.0, BODY_ROW)),
            row.icon,
            t::TEXT_DIM,
            15.0,
        );
        let dd = Rect::from_min_size(
            pos2(body.left() + 28.0, y + 1.0),
            vec2(body.width() - 34.0, BODY_ROW - 2.0),
        );
        let (response, boxr) = w::dropdown(
            ui,
            dd,
            ("shelf.dropdown", id.0, k),
            None,
            row.value,
            Some(row.tip),
            row.enabled,
            0.0,
        );
        if response.clicked() && !row.is_open {
            menu = Some((row.kind, boxr));
        }
    }
    menu
}

/// ハイドのプリセットのメニュー（何も隠さない・プリセットの全部・プリセットを追加）。
pub fn hide_entries(app: &AppState, id: ViewId, free: bool) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let Some(h) = app.view3d.view_of(id).map(|v| &v.hide) else {
        return Vec::new();
    };
    let only =
        |p: u32| h.islands.is_empty() && h.bones.entries.is_empty() && h.bones.presets == [p];
    let mut out = vec![Entry::item(
        lang.t("hide-show-everything"),
        Action::Hide(HideAction::ChoosePreset(id, None)),
    )
    .radio(h.is_clear())
    .enabled(free)];
    for p in app.view3d.pose.hide_presets.items() {
        out.push(
            Entry::item(
                p.name.clone(),
                Action::Hide(HideAction::ChoosePreset(id, Some(p.id))),
            )
            .radio(only(p.id))
            .enabled(free && app.view3d.pose.session.is_some()),
        );
    }
    out.push(Entry::Separator);
    out.push(
        Entry::item(
            lang.t("hide-add-preset"),
            Action::Mode(ModeAction::Set(EditorMode::Hide)),
        )
        .icon("add", false)
        .enabled(free),
    );
    out
}

/// ポーズのメニュー（今のポーズ・初期ポーズ・ポーズのプリセットの全部）。
pub fn pose_entries(app: &AppState, id: ViewId, free: bool) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let now = app.view3d.view_of(id).map(|v| &v.pose);
    let mut out = vec![
        Entry::item(
            lang.t("hide-current-pose"),
            Action::Hide(HideAction::ChoosePose(id, PoseChoice::Current)),
        )
        .radio(matches!(now, Some(ViewPose::Current)))
        .enabled(free),
        Entry::item(
            lang.t("hide-rest-pose"),
            Action::Hide(HideAction::ChoosePose(id, PoseChoice::Rest)),
        )
        .radio(matches!(now, Some(ViewPose::Rest(_))))
        .enabled(free),
    ];
    let presets = app.view3d.pose.pose_presets.items();
    if !presets.is_empty() {
        out.push(Entry::Separator);
    }
    for p in presets {
        out.push(
            Entry::item(
                p.name.clone(),
                Action::Hide(HideAction::ChoosePose(id, PoseChoice::Preset(p.id))),
            )
            .radio(matches!(now, Some(ViewPose::Preset(q, _)) if *q == p.id))
            .enabled(free),
        );
    }
    out
}

/// このビューのポーズを選ぶ（今のポーズ・初期ポーズ・ポーズのプリセット）。描いている最中は断る。ほかのビューが同じポーズなら、その形を
/// 共有する。プリセットのボーンが 1 つも合わなければ、ポーズを変えず理由を知らせる。
pub fn choose_pose(app: &mut AppState, id: ViewId, choice: PoseChoice) {
    let lang = app.lang;
    if super::refuse_while_stroking(app, Source::Pose) {
        return;
    }
    if !app.view3d.has_view(id) {
        return;
    }
    let next = match choice {
        PoseChoice::Current => ViewPose::Current,
        PoseChoice::Rest => {
            let Some(s) = app.view3d.pose.session.as_ref() else {
                return;
            };
            ViewPose::Rest(s.rig.rest_pose())
        }
        PoseChoice::Preset(p) => {
            let Some(preset) = app.view3d.pose.pose_presets.get(p).cloned() else {
                return;
            };
            let Some(s) = app.view3d.pose.session.as_ref() else {
                return;
            };
            let built = crate::view3d::pose::presets::build_pose(
                &s.rig,
                s.pose(),
                &preset.name,
                &preset.entries,
                false,
            );
            if built.applied == 0 && !preset.entries.is_empty() {
                app.refuse(
                    Source::Pose,
                    lang.f(
                        "view3d-no-bone-fits-pose",
                        &[("name", &preset.name), ("tail", &"")],
                    ),
                );
                return;
            }
            ViewPose::Preset(p, built.pose)
        }
    };
    let model = match next.pose() {
        None => None,
        Some(pose) => {
            // 同じポーズのビューがあれば、その形を使う
            let shared = app
                .view3d
                .views()
                .filter(|v| v.id != id && v.pose.pose() == Some(pose))
                .find_map(|v| app.view3d.view_pose_model(v.id).cloned());
            match shared {
                Some(m) => Some(m),
                None => match crate::view3d::pose::posed_model(&mut app.view3d, pose) {
                    Ok(m) => Some(Arc::new(m)),
                    Err(e) => {
                        let text = lang.view_error(&e);
                        app.notify(e.notice_kind(), Source::Pose, text);
                        return;
                    }
                },
            }
        }
    };
    if let Some(v) = app.view3d.view_of_mut(id) {
        v.pose = next;
    }
    app.view3d.set_view_pose_model(id, model);
}
