//! ポーズの欄（ドックのタブ「ポーズ」。ほかのタブと同じく動かせる・別のウィンドウに出せる。スキンのあるモデルを読んでいるときだけ中身が出る）:
//! 頭に操作のボタン（ポーズのモード・FBX を開く・ポーズを戻す・取り消し・やり直し）、その下は縦にスクロールする節。
//! 「ボーン」はボーンの木（開閉・選ぶ）と、選んだボーンのインスペクター（位置・回転・大きさを数値で直す・項目ごとに戻す）、
//! 「テイク」は FBX の中のテイクとフレームを選んでポーズにする（テイクのあるモデルだけ）、
//! 「ポーズのプリセット」は今のポーズに名前を付けて残す・当てる・左右を反転して当てる・上書き・名前を変える・消す、
//! 「面を隠す」はボーンの影響で面を隠す項目と隠し方のプリセット、「BlendShape」はスライダー（メッシュごと・1 つずつ戻す）。
//! 節の開閉は `AppState::sections` が覚える。文言は名前と状態だけ（操作の説明はツールチップ）。

mod hide_ui;
mod inspector;
mod preset_ui;
mod take_ui;

pub use take_ui::entries as take_entries;

use egui::{pos2, vec2, Rect, Sense, Ui};
use egui_dock::DockState;

use crate::state::{Action, AppState};
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, NumberFormat, Rows, SliderSpec};
use crate::view3d::pose::edit::{self, Reset};
use crate::view3d::pose::{self, PoseAction};

const TOOLBAR: f32 = 30.0;
const BONE_ROW: f32 = 22.0;
const INDENT: f32 = 12.0;
/// 木が見せる行の数（これより多いときは木の中でスクロールする）。
const TREE_MAX_ROWS: usize = 12;
const TREE_MIN_ROWS: usize = 3;
/// BlendShape の 1 行の戻すボタンの幅。
const SHAPE_RESET_W: f32 = 24.0;

/// スキンのあるモデルを読んでいて、ポーズのタブがドックのどこにも無ければ、プロパティと同じ組へ足す（前へは出さない。プロパティが見えたまま）。
/// 足したタブは、動かしても別のウィンドウへ出しても、そのまま残る（モデルが替わって欄が空になっても、タブはある）。ドックの配置を初めに戻したときは、
/// 次のフレームで足し直す。
pub fn ensure_tab(app: &AppState, dock: &mut DockState<crate::Tab>) {
    if app.view3d.pose.session.is_none() || dock.find_tab(&crate::Tab::Pose).is_some() {
        return;
    }
    // レイヤーと同じ組へ（プロパティの組はヒストリーもあり、3 つ並べると最小のウィンドウで名前が欠ける）
    let target = dock.find_tab(&crate::Tab::Layers).map(|p| p.node_path());
    match target.and_then(|path| dock.leaf_mut(path).ok()) {
        Some(leaf) => leaf.tabs.push(crate::Tab::Pose),
        None => dock.push_to_first_leaf(crate::Tab::Pose),
    }
}

/// 木で見えているボーン（深さ優先、開いたボーンの子だけ）と深さ。
pub fn visible_bones(s: &pose::PoseSession) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut stack: Vec<(usize, usize)> = s.rig.roots().map(|r| (r, 0)).collect();
    stack.reverse();
    while let Some((b, depth)) = stack.pop() {
        out.push((b, depth));
        if s.expanded.contains(&b) {
            for &c in s.rig.children(b).iter().rev() {
                stack.push((c as usize, depth + 1));
            }
        }
    }
    out
}

pub fn show(ui: &mut Ui, app: &mut AppState) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    let bar = Rect::from_min_size(r.min, vec2(r.width(), TOOLBAR));
    toolbar(ui, app, bar);
    let body = Rect::from_min_max(pos2(r.left(), bar.bottom()), r.max);
    let lang = app.lang;
    let loading = app.view3d.pose.loading_name().map(str::to_owned);
    if loading.is_some() {
        ui.ctx().request_repaint();
    }
    if app.view3d.pose.session.is_none() {
        if let Some(name) = loading {
            // 読み込み中: 名前・割合の帯・取り消しのボタン（取り消すと、読みかけは捨てる）
            let fraction = app.view3d.pose.loading_fraction();
            let row = Rect::from_min_size(
                pos2(body.left() + t::PADDING, body.top() + 4.0),
                vec2(body.width() - 2.0 * t::PADDING, BONE_ROW),
            );
            let cancel =
                Rect::from_min_size(pos2(row.right() - 24.0, row.top()), vec2(24.0, BONE_ROW));
            let shown = match fraction {
                Some(f) => format!(
                    "{}: {name} {}%",
                    lang.pick("読み込み中", "Loading"),
                    (f * 100.0).floor() as u32
                ),
                None => format!("{}: {name}", lang.pick("読み込み中", "Loading")),
            };
            let label = Rect::from_min_max(row.min, pos2(cancel.left() - 6.0, row.bottom()));
            w::text(ui.painter(), label, &shown, t::LABEL_DIM, Align::Left);
            let bar = Rect::from_min_max(
                pos2(row.left(), row.bottom() + 2.0),
                pos2(label.right(), row.bottom() + 7.0),
            );
            w::progress_bar(ui.painter(), bar, fraction, ui.input(|i| i.time));
            if w::icon_button(
                ui,
                cancel,
                "pose.cancel-load",
                "close",
                lang.pick("読み込みを取り消す", "Cancel loading"),
                false,
                true,
                15.0,
            )
            .clicked()
            {
                app.apply(Action::Pose(PoseAction::CancelLoad));
            }
        }
        return;
    }

    // 縦のスクロール（中身の高さは前のフレームのもの。はみ出していれば右端に細い帯）
    let scroller = Scroll::new(
        body,
        app.view3d.pose.panel_content,
        &mut app.view3d.pose.panel_scroll,
    );
    let scroll = app.view3d.pose.panel_scroll;
    let area = Rect::from_min_max(
        pos2(body.left(), body.top() - scroll),
        pos2(body.right() - scroller.reserved(), body.bottom()),
    );
    let outer_clip = ui.clip_rect();
    ui.set_clip_rect(body.intersect(outer_clip));
    let mut rows = Rows::new(area, 4.0);
    let wheel_used = content(ui, app, &mut rows, body, loading.as_deref());
    rows.indent = 0.0;
    rows.space(8.0);
    app.view3d.pose.panel_content = rows.used();
    ui.set_clip_rect(outer_clip);
    // 木が使わなかったホイールは、欄全体のスクロールへ
    if !wheel_used && ui.rect_contains_pointer(body) {
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        if wheel != 0.0 {
            app.view3d.pose.panel_scroll = (scroll - wheel).clamp(0.0, scroller.max);
        }
    }
    // スライダー・数値の欄のドラッグを離したら、続けて変えていた操作を 1 つの取り消しの段にする
    edit::finish_live_edit(app, ui.input(|i| i.pointer.any_down()));
    scroller.end(ui, "pose.panel.scroll", &mut app.view3d.pose.panel_scroll);
}

/// 頭のボタンの帯。
fn toolbar(ui: &mut Ui, app: &mut AppState, bar: Rect) {
    let p = ui.painter().clone();
    let free = !app.is_stroking();
    let editing = app.view3d.pose.drag.is_some()
        || app
            .view3d
            .pose
            .session
            .as_ref()
            .is_some_and(|s| s.is_editing());
    let has = app.view3d.pose.session.is_some();
    let lang = app.lang;
    w::fill(&p, bar, t::PANEL_HEADER);
    w::hline(&p, bar.left(), bar.right(), bar.bottom() - 1.0, t::BORDER);
    let mut x = bar.left() + 4.0;
    let mut next = |width: f32| {
        let at = Rect::from_min_size(pos2(x, bar.top() + 3.0), vec2(width, 24.0));
        x += width + 3.0;
        at
    };
    let mut action = None;
    if w::icon_button(
        ui,
        next(26.0),
        "pose.mode",
        "accessibility",
        lang.pick(
            "ポーズのモード（3D ビューの左ドラッグでギズモの輪を回す・面を押してボーンを選ぶ）",
            "Pose mode (drag a gizmo ring in the 3D View to rotate; click a surface to pick a bone)",
        ),
        app.mode == crate::mode::EditorMode::Pose,
        free && has,
        18.0,
    )
    .clicked()
    {
        action = Some(PoseAction::ToggleMode);
    }
    if w::icon_button(
        ui,
        next(26.0),
        "pose.open",
        "folder_open",
        lang.pick("FBX を開く", "Open FBX"),
        false,
        free,
        18.0,
    )
    .clicked()
    {
        action = Some(PoseAction::OpenFbx);
    }
    let posed = app
        .view3d
        .pose
        .session
        .as_ref()
        .is_some_and(|s| s.is_posed());
    if w::icon_button(
        ui,
        next(26.0),
        "pose.reset",
        "restart_alt",
        lang.pick(
            "ポーズを戻す（ファイルのポーズへ）",
            "Reset pose (to the file's pose)",
        ),
        false,
        free && !editing && posed,
        18.0,
    )
    .clicked()
    {
        action = Some(PoseAction::Reset);
    }
    let (can_undo, can_redo) = app
        .view3d
        .pose
        .session
        .as_ref()
        .map(|s| (s.can_undo(), s.can_redo()))
        .unwrap_or((false, false));
    let rest = bar.width() - 8.0 - 3.0 * 29.0; // アイコンのボタン 3 つの後
    let half = ((rest - 3.0) / 2.0).max(0.0);
    if w::button(
        ui,
        next(half),
        "pose.undo",
        lang.pick("取り消し", "Undo"),
        false,
        free && !editing && can_undo,
        Some(&pose_tip(
            lang,
            lang.pick("ポーズの取り消し", "Undo pose"),
            "edit.undo",
        )),
        None,
    )
    .clicked()
    {
        action = Some(PoseAction::Undo);
    }
    if w::button(
        ui,
        next(half),
        "pose.redo",
        lang.pick("やり直し", "Redo"),
        false,
        free && !editing && can_redo,
        Some(&pose_tip(
            lang,
            lang.pick("ポーズのやり直し", "Redo pose"),
            "edit.redo",
        )),
        None,
    )
    .clicked()
    {
        action = Some(PoseAction::Redo);
    }
    if let Some(a) = action {
        app.apply(Action::Pose(a));
    }
}

/// 節の並び（読み込み中の知らせ・名前と知らせ・ボーン・テイク・ポーズのプリセット・面を隠す・BlendShape）。木がホイールを使ったら true。
fn content(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    body: Rect,
    loading: Option<&str>,
) -> bool {
    let lang = app.lang;
    if let Some(name) = loading {
        let at = rows.row(BONE_ROW, 0.0);
        w::text(
            ui.painter(),
            at,
            &format!("{}: {name}", lang.pick("読み込み中", "Loading")),
            t::LABEL_DIM,
            Align::Left,
        );
    }
    let Some(s) = app.view3d.pose.session.as_ref() else {
        return false;
    };
    // 名前と知らせ
    let at = rows.row(BONE_ROW, 0.0);
    w::text(
        ui.painter(),
        at,
        &w::fit(ui.painter(), s.rig.name(), at.width(), t::LABEL_DIM),
        t::LABEL_DIM,
        Align::Left,
    );
    // Live Link の相手のモデル: 送り直すと、ここで動かした分も Unity のポーズで上書きする
    if app.link_target.is_some() && app.model.as_ref().is_some_and(|m| m.is_live_link()) {
        ui.interact(at, ui.id().with("pose.livelink"), egui::Sense::hover())
            .on_hover_text(lang.pick(
                "Unity から送り直すと、ポーズは Unity の値で上書きされます（ここで動かした分も）",
                "Resending from Unity overwrites the pose with Unity's values (including changes made here)",
            ));
    }
    if !s.warnings.is_empty() {
        let at = rows.row(BONE_ROW, 0.0);
        warning_row(
            ui,
            at,
            "pose.warnings",
            s.warnings.len(),
            &s.warnings.join("\n"),
            lang,
        );
    }
    let has_shapes = s.rig.blend_shape_count() > 0;

    let (open, reset_all) = super::properties::section(
        ui,
        app,
        rows,
        "pose.bones",
        lang.pick("ボーン", "Bones"),
        "accessibility",
        Some(lang.pick(
            "すべてのボーンを戻す（BlendShape はそのまま）",
            "Reset all bones (BlendShapes stay)",
        )),
    );
    if reset_all {
        edit::reset(app, Reset::AllBones);
    }
    let mut wheel_used = false;
    if open {
        wheel_used = bones_section(ui, app, rows, body);
    }
    rows.indent = 0.0;

    let has_takes = app
        .view3d
        .pose
        .session
        .as_ref()
        .is_some_and(|s| !s.takes.is_empty());
    if has_takes {
        let (open, _) = super::properties::section(
            ui,
            app,
            rows,
            "pose.takes",
            lang.pick("テイク", "Takes"),
            "video_clip",
            None,
        );
        if open {
            take_ui::show(ui, app, rows);
        }
        rows.indent = 0.0;
    }

    let (open, _) = super::properties::section(
        ui,
        app,
        rows,
        "pose.presets",
        lang.pick("ポーズのプリセット", "Pose Presets"),
        "save",
        None,
    );
    if open {
        preset_ui::show(ui, app, rows);
    }
    rows.indent = 0.0;

    let (open, show_all) = super::properties::section(
        ui,
        app,
        rows,
        "pose.hide",
        lang.pick("面を隠す", "Hide Surfaces"),
        "visibility_off",
        Some(lang.pick("すべて表示する", "Show everything")),
    );
    if show_all {
        crate::view3d::pose::hide::show_all(app);
    }
    if open {
        hide_ui::show(ui, app, rows);
    }
    rows.indent = 0.0;

    if has_shapes {
        let (open, reset_shapes) = super::properties::section(
            ui,
            app,
            rows,
            "pose.shapes",
            "BlendShape",
            "tune",
            Some(lang.pick(
                "すべての BlendShape を戻す（ボーンはそのまま）",
                "Reset all BlendShapes (bones stay)",
            )),
        );
        if reset_shapes {
            edit::reset(app, Reset::AllShapes);
        }
        if open {
            blend_shapes(ui, app, rows, body);
        }
        rows.indent = 0.0;
    }
    wheel_used
}

/// 知らせの行（警告の印と件数。中身はツールチップ）。
pub(crate) fn warning_row(
    ui: &mut Ui,
    at: Rect,
    id: &'static str,
    count: usize,
    tip: &str,
    lang: crate::lang::Lang,
) {
    let p = ui.painter().clone();
    w::icon(
        &p,
        Rect::from_min_size(at.min, vec2(16.0, at.height())),
        "warning",
        t::WARNING,
        14.0,
    );
    w::text(
        &p,
        Rect::from_min_max(pos2(at.left() + 20.0, at.top()), at.max),
        &lang.pick(format!("知らせ {count} 件"), format!("Notices {count}")),
        t::LABEL_DIM,
        Align::Left,
    );
    ui.interact(at, ui.make_persistent_id(id), Sense::hover())
        .on_hover_text(tip);
}

/// 「ボーン」の節の中身: 木と、選んだボーンのインスペクター。木がホイールを使ったら true。
fn bones_section(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, body: Rect) -> bool {
    let Some(s) = app.view3d.pose.session.as_ref() else {
        return false;
    };
    let count = visible_bones(s).len();
    let selected = s.selected;
    // 木の高さは、欄の高さの半分まで（狭いドックでも、下のインスペクターを動かさずに見られるように）
    let fit = ((body.height() * 0.5) / BONE_ROW).floor() as usize;
    let height =
        count.clamp(TREE_MIN_ROWS, fit.clamp(TREE_MIN_ROWS, TREE_MAX_ROWS)) as f32 * BONE_ROW;
    let list = rows.row(height, 6.0);
    let used = bone_tree(ui, app, list);
    if let Some(bone) = selected {
        inspector::show(ui, app, rows, bone);
    }
    used
}

/// ボーンの木。ホイールで木を動かしたら true（端まで来ていれば使わず、欄全体のスクロールへ回す）。
fn bone_tree(ui: &mut Ui, app: &mut AppState, list: Rect) -> bool {
    let painter = ui.painter_at(list);
    w::fill(&painter, list, t::CONTROL_BG);
    let free = !app.is_stroking() && app.view3d.pose.drag.is_none();
    let Some(s) = app.view3d.pose.session.as_mut() else {
        return false;
    };
    let rows = visible_bones(s);
    let content = rows.len() as f32 * BONE_ROW;
    let max_scroll = (content - list.height()).max(0.0);
    let reserved = if max_scroll > 0.0 {
        crate::ui::scroll::BAR_WIDTH
    } else {
        0.0
    };
    if let Some(b) = s.reveal.take() {
        if let Some(i) = rows.iter().position(|(r, _)| *r == b) {
            let top = i as f32 * BONE_ROW;
            if top < s.tree_scroll {
                s.tree_scroll = top;
            } else if top + BONE_ROW > s.tree_scroll + list.height() {
                s.tree_scroll = top + BONE_ROW - list.height();
            }
        }
    }
    let mut wheel_used = false;
    if max_scroll > 0.0 && ui.rect_contains_pointer(list) {
        let next = (s.tree_scroll - ui.input(|i| i.smooth_scroll_delta.y)).clamp(0.0, max_scroll);
        if next != s.tree_scroll {
            s.tree_scroll = next;
            wheel_used = true;
        }
    }
    let tree_bar = Scroll::new(list, content, &mut s.tree_scroll);
    let mut select = None;
    let mut toggle = None;
    for (i, &(b, depth)) in rows.iter().enumerate() {
        let top = list.top() + i as f32 * BONE_ROW - s.tree_scroll;
        let row = Rect::from_min_size(
            pos2(list.left(), top),
            vec2(list.width() - reserved, BONE_ROW),
        );
        if row.bottom() < list.top() || row.top() > list.bottom() {
            continue;
        }
        let hit = row.intersect(list);
        let x = row.left() + 4.0 + depth as f32 * INDENT;
        let chevron = Rect::from_min_size(pos2(x, row.top()), vec2(14.0, BONE_ROW));
        let has_children = !s.rig.children(b).is_empty();
        let id = ui.make_persistent_id(("pose.bone", b));
        let response = ui.interact(hit, id, if free { Sense::click() } else { Sense::hover() });
        let selected = s.selected == Some(b);
        if selected {
            w::fill(&painter, row, t::ACCENT_SOFT);
            w::fill(
                &painter,
                Rect::from_min_size(row.min, vec2(3.0, row.height())),
                t::ACCENT,
            );
        } else if response.hovered() {
            w::fill(&painter, row, t::CONTROL_HOVER);
        }
        if has_children {
            w::icon(
                &painter,
                chevron,
                if s.expanded.contains(&b) {
                    "expand_more"
                } else {
                    "chevron_right"
                },
                t::TEXT_DIM,
                13.0,
            );
        }
        let bone = &s.rig.bones()[b];
        let color = if s.rig.is_deforming(b) {
            t::TEXT
        } else {
            t::TEXT_DIM
        };
        let name_rect = Rect::from_min_max(pos2(chevron.right() + 2.0, row.top()), row.max);
        let name = w::fit(&painter, &bone.name, name_rect.width(), t::LABEL);
        w::text(
            &painter,
            name_rect,
            &name,
            t::LABEL.with_color(color),
            Align::Left,
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                free,
                selected,
                &bone.name,
            )
        });
        if response.clicked() {
            let on_chevron = response
                .interact_pointer_pos()
                .is_some_and(|p| chevron.contains(p));
            if on_chevron && has_children {
                toggle = Some(b);
            } else {
                select = Some(b);
            }
        }
        if response.double_clicked() && has_children {
            toggle = Some(b);
        }
    }
    if let Some(b) = toggle {
        if !s.expanded.remove(&b) {
            s.expanded.insert(b);
        }
    }
    if let Some(b) = select {
        s.selected = Some(b);
    }
    if select.is_some() {
        // ボーンを選んだらポーズのモードへ（輪が出る）
        app.set_mode(crate::mode::EditorMode::Pose);
    }
    if let Some(s) = app.view3d.pose.session.as_mut() {
        tree_bar.end(ui, "pose.tree.scroll", &mut s.tree_scroll);
    }
    wheel_used
}

/// 「BlendShape」の節の中身: スライダー（メッシュが 2 つ以上なら、メッシュの名前の行を挟む）と、1 つずつ戻すボタン。
fn blend_shapes(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, body: Rect) {
    let free = !app.is_stroking() && app.view3d.pose.drag.is_none();
    let lang = app.lang;
    let Some(s) = app.view3d.pose.session.as_ref() else {
        return;
    };
    let several = s
        .rig
        .meshes()
        .iter()
        .filter(|m| !m.blend_shapes.is_empty())
        .count()
        > 1;
    let rest = s.rig.rest_pose();
    let mut change = None;
    let mut reset = None;
    for (m, mesh) in s.rig.meshes().iter().enumerate() {
        if mesh.blend_shapes.is_empty() {
            continue;
        }
        if several {
            let at = rows.row(BONE_ROW, 0.0);
            if at.bottom() >= body.top() && at.top() <= body.bottom() {
                w::text(
                    ui.painter(),
                    at,
                    &w::fit(ui.painter(), &mesh.mesh.name, at.width(), t::LABEL_DIM),
                    t::LABEL_DIM,
                    Align::Left,
                );
            }
        }
        for (k, shape) in mesh.blend_shapes.iter().enumerate() {
            let at = rows.slider_row();
            if at.bottom() < body.top() || at.top() > body.bottom() {
                continue;
            }
            let value = s.pose().blend_weights[m][k];
            let out = w::slider(
                ui,
                at,
                ("pose.shape", m, k),
                value,
                &SliderSpec::new(&shape.name, 0.0, 100.0, NumberFormat::int(""))
                    .enabled(free)
                    .inset(SHAPE_RESET_W + 4.0),
            );
            if out.changed || out.released {
                change = Some((m, k, out.value, out.active, out.released));
            }
            let button = Rect::from_min_size(
                pos2(at.right() - SHAPE_RESET_W, at.bottom() - 20.0),
                vec2(SHAPE_RESET_W, 20.0),
            );
            let differs = value != rest.blend_weights[m][k];
            if w::icon_button(
                ui,
                button,
                ("pose.shape.reset", m, k),
                "restart_alt",
                lang.pick("この BlendShape を戻す", "Reset this BlendShape"),
                false,
                free && differs && !s.is_editing(),
                14.0,
            )
            .clicked()
            {
                reset = Some(Reset::Shape(m, k));
            }
        }
    }
    if let Some((m, k, value, active, released)) = change {
        set_weight(app, m, k, value, active, released);
    }
    if let Some(what) = reset {
        edit::reset(app, what);
    }
}

/// BlendShape の重みを変える（ドラッグの間は 1 つの操作。離したら取り消しに 1 つ）。
fn set_weight(app: &mut AppState, m: usize, k: usize, value: f32, active: bool, released: bool) {
    let Some(s) = app.view3d.pose.session.as_ref() else {
        return;
    };
    let mut next = s.pose().clone();
    let changed = next.blend_weights[m][k] != value;
    next.blend_weights[m][k] = value;
    if changed {
        if !s.is_editing() {
            if let Err(e) = pose::begin_edit(&mut app.view3d) {
                app.notify(
                    e.notice_kind(),
                    crate::notice::Source::Pose,
                    app.lang.view_error(&e),
                );
                return;
            }
        }
        if let Err(e) = pose::edit(&mut app.view3d, next) {
            app.notify(
                e.notice_kind(),
                crate::notice::Source::Pose,
                app.lang.view_error(&e),
            );
        }
    }
    if released || !active {
        pose::end_edit(&mut app.view3d, true);
    }
}

/// ポーズの取り消し・やり直しのツールチップ（ポーズのモードのキーを添える。設定で変えたキーに付いてくる）。
fn pose_tip(lang: crate::lang::Lang, name: &str, command: &str) -> String {
    match crate::shortcuts::key_in(command, crate::mode::EditorMode::Pose) {
        Some(key) => lang.pick(
            format!("{name}（ポーズのモードでは {key}）"),
            format!("{name} ({key} in pose mode)"),
        ),
        None => name.to_owned(),
    }
}
