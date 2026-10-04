//! 3D ビューの左のポーズの欄（スキンのあるモデルを読んでいるときだけ出す）: 頭に操作のボタン（ポーズのモード・FBX を開く・
//! ポーズを戻す・取り消し・やり直し）、骨の木（開閉・選ぶ）、選んだ骨の回転、BlendShape のスライダー（メッシュごと）。
//! 文言は名前と状態だけ（操作の説明はツールチップ）。

use egui::{pos2, vec2, Rect, Sense, Ui, UiBuilder};

use crate::state::{Action, AppState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, NumberFormat, SliderSpec};
use crate::view3d::pose::{self, PoseAction};

/// 欄の幅（点）。
pub const WIDTH: f32 = 260.0;
const TOOLBAR: f32 = 30.0;
const BONE_ROW: f32 = 22.0;
const INDENT: f32 = 12.0;

/// 3D ビューの中身を、ポーズの欄と 3D の表示域に分ける（欄を出さないなら None と全部）。
pub fn split(app: &AppState, content: Rect) -> (Option<Rect>, Rect) {
    if app.view3d.pose.session.is_none() && !app.view3d.pose.is_loading() {
        return (None, content);
    }
    let width = WIDTH.min((content.width() * 0.5).floor());
    let panel = Rect::from_min_max(content.min, pos2(content.left() + width, content.bottom()));
    let view = Rect::from_min_max(pos2(panel.right() + 1.0, content.top()), content.max);
    (Some(panel), view)
}

/// 木で見えている骨（深さ優先、開いた骨の子だけ）と深さ。
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

pub fn show(ui: &mut Ui, app: &mut AppState, r: Rect) {
    let p = ui.painter().clone();
    w::fill(&p, r, t::PANEL_BG);
    w::vline(&p, r.right(), r.top(), r.bottom(), t::BORDER);
    let free = !app.is_stroking();
    let editing = app.view3d.pose.drag.is_some()
        || app
            .view3d
            .pose
            .session
            .as_ref()
            .is_some_and(|s| s.is_editing());
    let has = app.view3d.pose.session.is_some();

    // 頭のボタン
    let bar = Rect::from_min_size(r.min, vec2(r.width(), TOOLBAR));
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
        "ポーズのモード（3D ビューの左ドラッグでギズモの輪を回す・面を押して骨を選ぶ）",
        app.view3d.pose.mode,
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
        "FBX を開く",
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
        "ポーズを戻す（ファイルのポーズへ）",
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
        "取り消し",
        false,
        free && !editing && can_undo,
        Some("ポーズの取り消し（ポーズのモードでは Ctrl+Z）"),
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
        "やり直し",
        false,
        free && !editing && can_redo,
        Some("ポーズのやり直し（ポーズのモードでは Ctrl+Shift+Z）"),
        None,
    )
    .clicked()
    {
        action = Some(PoseAction::Redo);
    }
    if let Some(a) = action {
        app.apply(Action::Pose(a));
    }

    let mut y = bar.bottom() + 4.0;
    let row =
        |y: f32, h: f32| Rect::from_min_max(pos2(r.left() + 8.0, y), pos2(r.right() - 8.0, y + h));
    if let Some(name) = app.view3d.pose.loading_name() {
        w::text(
            &p,
            row(y, BONE_ROW),
            &format!("読み込み中: {name}"),
            t::LABEL_DIM,
            Align::Left,
        );
        y += BONE_ROW;
        ui.ctx().request_repaint();
    }
    let Some(s) = app.view3d.pose.session.as_ref() else {
        return;
    };
    // 名前と数・知らせ
    let info = format!(
        "{} · 骨 {} · BlendShape {}",
        s.rig.name(),
        s.rig.bones().len(),
        s.rig.blend_shape_count()
    );
    let info_rect = row(y, BONE_ROW);
    w::text(
        &p,
        info_rect,
        &w::fit(&p, &info, info_rect.width(), t::LABEL_DIM),
        t::LABEL_DIM,
        Align::Left,
    );
    y += BONE_ROW;
    if !s.warnings.is_empty() {
        let at = row(y, BONE_ROW);
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
            &format!("知らせ {} 件", s.warnings.len()),
            t::LABEL_DIM,
            Align::Left,
        );
        let tip = s.warnings.join("\n");
        ui.interact(at, ui.make_persistent_id("pose.warnings"), Sense::hover())
            .on_hover_text(tip);
        y += BONE_ROW;
    }

    // 骨の木と BlendShape の高さ
    let shapes: Vec<(usize, usize)> = s
        .rig
        .meshes()
        .iter()
        .enumerate()
        .flat_map(|(m, mesh)| (0..mesh.blend_shapes.len()).map(move |k| (m, k)))
        .collect();
    let selected_row = BONE_ROW;
    let header = t::ROW_HEIGHT;
    let available = (r.bottom() - y - 4.0).max(0.0);
    let tree_height = if shapes.is_empty() {
        available - header - selected_row
    } else {
        ((available - 2.0 * header - selected_row) * 0.55).max(BONE_ROW * 3.0)
    }
    .max(0.0);

    w::subsection_header(ui, row(y, header), "pose.bones", "骨", true);
    y += header;
    let list = Rect::from_min_size(pos2(r.left() + 4.0, y), vec2(r.width() - 8.0, tree_height));
    bone_tree(ui, app, list);
    y = list.bottom();

    // 選んだ骨の回転（ローカル、XYZ のオイラー角）
    if let Some(s) = app.view3d.pose.session.as_ref() {
        let at = row(y + 2.0, selected_row);
        if let Some(b) = s.selected {
            let q = s.pose().locals[b].rotation;
            let (ex, ey, ez) = q.to_euler(yolu_core::glam::EulerRot::XYZ);
            // −0° と出さない
            let deg = |r: f32| {
                let d = r.to_degrees().round();
                if d == 0.0 {
                    0.0
                } else {
                    d
                }
            };
            let (ex, ey, ez) = (deg(ex), deg(ey), deg(ez));
            w::text(
                &p,
                at,
                &w::fit(
                    &p,
                    &format!("{}  {ex:.0}° {ey:.0}° {ez:.0}°", s.rig.bones()[b].name),
                    at.width(),
                    t::LABEL_SMALL,
                ),
                t::LABEL_SMALL,
                Align::Left,
            );
        }
    }
    y += selected_row + 2.0;

    if !shapes.is_empty() {
        w::subsection_header(ui, row(y, header), "pose.shapes", "BlendShape", true);
        y += header;
        let list = Rect::from_min_max(
            pos2(r.left() + 4.0, y),
            pos2(r.right() - 4.0, r.bottom() - 4.0),
        );
        blend_shapes(ui, app, list, &shapes);
    }
}

fn bone_tree(ui: &mut Ui, app: &mut AppState, list: Rect) {
    let painter = ui.painter_at(list);
    w::fill(&painter, list, t::CONTROL_BG);
    let free = !app.is_stroking() && app.view3d.pose.drag.is_none();
    let Some(s) = app.view3d.pose.session.as_mut() else {
        return;
    };
    let rows = visible_bones(s);
    let content = rows.len() as f32 * BONE_ROW;
    let max_scroll = (content - list.height()).max(0.0);
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
    if ui.rect_contains_pointer(list) {
        s.tree_scroll -= ui.input(|i| i.smooth_scroll_delta.y);
    }
    s.tree_scroll = s.tree_scroll.clamp(0.0, max_scroll);
    let mut select = None;
    let mut toggle = None;
    for (i, &(b, depth)) in rows.iter().enumerate() {
        let top = list.top() + i as f32 * BONE_ROW - s.tree_scroll;
        let row = Rect::from_min_size(
            pos2(list.left(), top),
            vec2(
                list.width() - if max_scroll > 0.0 { 8.0 } else { 0.0 },
                BONE_ROW,
            ),
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
        // 骨を選んだらポーズのモードへ（輪が出る）
        app.view3d.pose.mode = true;
    }
    if max_scroll > 0.0 {
        let s = app
            .view3d
            .pose
            .session
            .as_ref()
            .map(|s| s.tree_scroll)
            .unwrap_or(0.0);
        let bar_h = (list.height() * list.height() / content).max(16.0);
        let bar_y = list.top() + (list.height() - bar_h) * s / max_scroll;
        w::rounded(
            &painter,
            Rect::from_min_size(pos2(list.right() - 6.0, bar_y), vec2(4.0, bar_h)),
            t::CONTROL_ACTIVE,
            2.0,
        );
    }
}

fn blend_shapes(ui: &mut Ui, app: &mut AppState, list: Rect, shapes: &[(usize, usize)]) {
    enum Row {
        Mesh(usize),
        Shape(usize, usize),
    }
    let free = !app.is_stroking() && app.view3d.pose.drag.is_none();
    let Some(s) = app.view3d.pose.session.as_mut() else {
        return;
    };
    // メッシュが 2 つ以上なら、メッシュの名前の行を挟む
    let several = s
        .rig
        .meshes()
        .iter()
        .filter(|m| !m.blend_shapes.is_empty())
        .count()
        > 1;
    let mut rows = Vec::new();
    for (i, &(m, k)) in shapes.iter().enumerate() {
        if several && (i == 0 || shapes[i - 1].0 != m) {
            rows.push(Row::Mesh(m));
        }
        rows.push(Row::Shape(m, k));
    }
    let height = |r: &Row| match r {
        Row::Mesh(_) => BONE_ROW,
        Row::Shape(..) => t::SLIDER_ROW_HEIGHT,
    };
    let content: f32 = rows.iter().map(height).sum();
    let max_scroll = (content - list.height()).max(0.0);
    if ui.rect_contains_pointer(list) {
        s.shapes_scroll -= ui.input(|i| i.smooth_scroll_delta.y);
    }
    s.shapes_scroll = s.shapes_scroll.clamp(0.0, max_scroll);
    let scroll = s.shapes_scroll;
    let painter = ui.painter_at(list);
    let mut child = ui.new_child(UiBuilder::new().max_rect(list));
    child.set_clip_rect(list.intersect(ui.clip_rect()));
    let width = list.width() - 8.0 - if max_scroll > 0.0 { 6.0 } else { 0.0 };
    let mut y = list.top() - scroll;
    let mut change = None;
    for r in &rows {
        let h = height(r);
        let at = Rect::from_min_size(pos2(list.left() + 4.0, y), vec2(width, h - 4.0));
        y += h;
        if at.bottom() < list.top() || at.top() > list.bottom() {
            continue;
        }
        let Some(s) = app.view3d.pose.session.as_ref() else {
            return;
        };
        match *r {
            Row::Mesh(m) => w::text(
                &painter,
                at,
                &s.rig.meshes()[m].mesh.name,
                t::LABEL_DIM,
                Align::Left,
            ),
            Row::Shape(m, k) => {
                let shape = &s.rig.meshes()[m].blend_shapes[k];
                let value = s.pose().blend_weights[m][k];
                let out = w::slider(
                    &mut child,
                    at,
                    ("pose.shape", m, k),
                    value,
                    &SliderSpec::new(&shape.name, 0.0, 100.0, NumberFormat::int("")).enabled(free),
                );
                if out.changed || out.released {
                    change = Some((m, k, out.value, out.active, out.released));
                }
            }
        }
    }
    if max_scroll > 0.0 {
        let bar_h = (list.height() * list.height() / content).max(16.0);
        let bar_y = list.top() + (list.height() - bar_h) * scroll / max_scroll;
        w::rounded(
            &painter,
            Rect::from_min_size(pos2(list.right() - 6.0, bar_y), vec2(4.0, bar_h)),
            t::CONTROL_ACTIVE,
            2.0,
        );
    }
    if let Some((m, k, value, active, released)) = change {
        set_weight(app, m, k, value, active, released);
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
                app.message = e;
                return;
            }
        }
        if let Err(e) = pose::edit(&mut app.view3d, next) {
            app.message = e;
        }
    }
    if released || !active {
        pose::end_edit(&mut app.view3d, true);
    }
}
