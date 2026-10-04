//! 窓の外枠（Unity 版の Shell）: メニューの中身、オプションバー（今のツールの設定を 1 行で）、ツールの帯、ステータスバー、
//! キーの割り当て。

use egui::{pos2, vec2, Key, Modifiers, Rect, Sense, Ui};

use crate::engine::BlendMode;
use crate::livelink::{LinkStatus, NoticeLevel};
use crate::state::{blend_name, Action, AppState, PopupKind, Tool};
use crate::ui::menu::Entry;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, NumberFormat, SliderSpec};

/// メニューバーの見出し。
pub const MENU_TITLES: [&str; 5] = ["ファイル", "編集", "レイヤー", "表示", "ヘルプ"];

/// メニューバーの見出しの中身。
pub fn menu_entries(app: &AppState, index: usize) -> Vec<Entry<Action>> {
    let free = !app.is_stroking();
    let selected = app.selected_layer.and_then(|id| app.doc.layer(id));
    match index {
        0 => {
            vec![
                Entry::item("新規プロジェクト…", Action::NewProjectDialog)
                    .shortcut("Ctrl+N")
                    .enabled(free),
                Entry::item("開く…", Action::OpenProjectDialog)
                    .shortcut("Ctrl+O")
                    .enabled(free),
                Entry::Separator,
                Entry::item("保存", Action::SaveProject)
                    .shortcut("Ctrl+S")
                    .enabled(free),
                Entry::item("別名で保存…", Action::SaveProjectAsDialog)
                    .shortcut("Ctrl+Shift+S")
                    .enabled(free),
                Entry::Separator,
                Entry::item("Live Link", Action::ToggleLiveLink).checked(app.link.is_on()),
                Entry::Separator,
                Entry::item("終了", Action::Quit).shortcut("Ctrl+Q"),
            ]
        }
        1 => vec![
            Entry::item("取り消し", Action::Undo)
                .shortcut("Ctrl+Z")
                .enabled(free && app.doc.can_undo()),
            Entry::item("やり直し", Action::Redo)
                .shortcut("Ctrl+Shift+Z / Ctrl+Y")
                .enabled(free && app.doc.can_redo()),
            Entry::Separator,
            Entry::item("ブラシ", Action::SelectTool(Tool::Brush))
                .shortcut("B")
                .radio(app.tool == Tool::Brush),
            Entry::item("消しゴム", Action::SelectTool(Tool::Eraser))
                .shortcut("E")
                .radio(app.tool == Tool::Eraser),
            Entry::Separator,
            Entry::item("メインとサブの色を入れ替え", Action::SwapColors).shortcut("X"),
            Entry::item("初期設定の色", Action::DefaultColors).shortcut("D"),
        ],
        2 => vec![
            Entry::item("新規レイヤー", Action::NewLayer)
                .shortcut("Ctrl+Shift+N")
                .enabled(free),
            Entry::item("レイヤーを削除", Action::DeleteLayer)
                .enabled(free && selected.is_some() && app.doc.layers().len() > 1),
            Entry::Separator,
            Entry::item("レイヤーを上へ", Action::LayerUp).enabled(free && selected.is_some()),
            Entry::item("レイヤーを下へ", Action::LayerDown).enabled(free && selected.is_some()),
            Entry::Separator,
            match selected {
                Some(l) => Entry::item(
                    if l.visible() {
                        "非表示にする"
                    } else {
                        "表示する"
                    },
                    Action::ToggleVisible(l.id()),
                )
                .enabled(free),
                None => Entry::item("表示する", Action::About).enabled(false),
            },
        ],
        3 => vec![
            Entry::item("ズームイン", Action::ZoomIn).shortcut("Ctrl++"),
            Entry::item("ズームアウト", Action::ZoomOut).shortcut("Ctrl+-"),
            Entry::item("画面に合わせる", Action::FitView)
                .shortcut("Ctrl+0")
                .enabled(free),
            Entry::Separator,
            Entry::item("表示を左に回す", Action::RotateLeft)
                .shortcut("-")
                .enabled(free),
            Entry::item("表示を右に回す", Action::RotateRight)
                .shortcut("^")
                .enabled(free),
            Entry::item("回転を戻す", Action::ResetRotation)
                .shortcut("Shift+R")
                .enabled(free && app.view.angle != 0.0),
            Entry::item("表示を左右反転", Action::FlipView)
                .shortcut("H")
                .checked(app.view.flip)
                .enabled(free),
            Entry::Separator,
            Entry::item("3D ビューに試しの立方体を読む", Action::LoadDemoModel).enabled(free),
            Entry::item("3D ビューでモデル全体を見る", Action::FrameModel)
                .enabled(free && app.view3d.model.is_some()),
            Entry::Separator,
            Entry::item("パネルの並びを戻す", Action::ResetLayout),
        ],
        _ => vec![Entry::item("YoluPainter について", Action::About)],
    }
}

/// ポップアップの中身（メニューバー・合成モード・レイヤーの右クリック）。
pub fn popup_entries(app: &AppState, kind: PopupKind) -> Vec<Entry<Action>> {
    match kind {
        PopupKind::MenuBar(i) => menu_entries(app, i),
        PopupKind::BlendMode(id) => {
            let current = app.doc.layer(id).map(|l| l.blend_mode());
            BlendMode::LAYER_MODES
                .iter()
                .map(|m| {
                    Entry::item(blend_name(*m), Action::SetBlend(id, *m)).radio(current == Some(*m))
                })
                .collect()
        }
        PopupKind::SetContext(uid) => {
            let set = app.sets.by_uid(uid);
            let visible = set.is_some_and(|s| s.visible);
            let writable = set.is_some_and(|s| s.read_only.is_none());
            vec![
                Entry::item("名前を変更", Action::StartRenameSet(uid))
                    .enabled(!app.is_stroking() && writable),
                Entry::item(
                    if visible { "隠す" } else { "見せる" },
                    Action::ToggleSetVisible(uid),
                ),
            ]
        }
        PopupKind::LayerContext(id) => {
            let free = !app.is_stroking();
            let visible = app.doc.layer(id).map(|l| l.visible()).unwrap_or(true);
            vec![
                Entry::item("新規レイヤー", Action::NewLayer).enabled(free),
                Entry::item("レイヤーを削除", Action::DeleteLayer)
                    .enabled(free && app.doc.layers().len() > 1),
                Entry::Separator,
                Entry::item("名前を変更", Action::StartRename(id)).enabled(free),
                Entry::item(
                    if visible {
                        "非表示にする"
                    } else {
                        "表示する"
                    },
                    Action::ToggleVisible(id),
                )
                .enabled(free),
                Entry::Separator,
                Entry::item("レイヤーを上へ", Action::LayerUp).enabled(free),
                Entry::item("レイヤーを下へ", Action::LayerDown).enabled(free),
            ]
        }
    }
}

/// キーの割り当て（文字を打っている間・メニューを開いている間は見ない。メニューは自分でキーを見る）。
pub fn handle_shortcuts(ctx: &egui::Context, app: &mut AppState) {
    if ctx.egui_wants_keyboard_input() || app.popup.is_some() {
        return;
    }
    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    let mut actions = Vec::new();
    ctx.input_mut(|i| {
        let mut key = |m: Modifiers, k: Key, a: Action| {
            if i.consume_key(m, k) {
                actions.push(a);
            }
        };
        // Shift 付きを先に取る（consume_key は書いていない Shift を気にしない。取った押下は消えるので、次の Ctrl+Z には残らない）
        key(cmd_shift, Key::Z, Action::Redo);
        key(Modifiers::COMMAND, Key::Z, Action::Undo);
        key(Modifiers::COMMAND, Key::Y, Action::Redo);
        key(cmd_shift, Key::N, Action::NewLayer);
        key(cmd_shift, Key::S, Action::SaveProjectAsDialog);
        key(Modifiers::COMMAND, Key::S, Action::SaveProject);
        key(Modifiers::COMMAND, Key::O, Action::OpenProjectDialog);
        key(Modifiers::COMMAND, Key::N, Action::NewProjectDialog);
        key(Modifiers::COMMAND, Key::Num0, Action::FitView);
        key(Modifiers::COMMAND, Key::Plus, Action::ZoomIn);
        key(Modifiers::COMMAND, Key::Equals, Action::ZoomIn);
        key(Modifiers::COMMAND, Key::Minus, Action::ZoomOut);
        key(Modifiers::COMMAND, Key::Q, Action::Quit);
        key(Modifiers::SHIFT, Key::R, Action::ResetRotation);
        key(Modifiers::NONE, Key::B, Action::SelectTool(Tool::Brush));
        key(Modifiers::NONE, Key::E, Action::SelectTool(Tool::Eraser));
        key(Modifiers::NONE, Key::X, Action::SwapColors);
        key(Modifiers::NONE, Key::D, Action::DefaultColors);
        key(Modifiers::NONE, Key::OpenBracket, Action::BrushSmaller);
        key(Modifiers::NONE, Key::CloseBracket, Action::BrushLarger);
        key(Modifiers::NONE, Key::H, Action::FlipView);
        key(Modifiers::NONE, Key::Minus, Action::RotateLeft);
        key(Modifiers::NONE, Key::Equals, Action::RotateRight);
        // ^ はキーの位置が配列で違うので文字で見る（JIS の ^ のキー、US の Shift+6）
        if i.events
            .iter()
            .any(|e| matches!(e, egui::Event::Text(s) if s == "^"))
        {
            actions.push(Action::RotateRight);
        }
    });
    for a in actions {
        app.apply(a);
    }
}

/// オプションバー（今のツールの設定。ブラシと消しゴムは直径・硬さ・不透明度・流量・間隔と筆圧の切り替え）。
pub fn options_bar(ui: &mut Ui, app: &mut AppState, r: Rect) {
    let p = ui.painter().clone();
    w::fill(&p, r, t::PANEL_BG);
    w::hline(&p, r.left(), r.right(), r.bottom() - 1.0, t::BORDER);
    let mut x = r.left() + 8.0;
    let (y, h) = (r.top() + 6.0, r.height() - 12.0);
    w::icon(
        &p,
        Rect::from_min_size(pos2(x, r.top()), vec2(22.0, r.height())),
        &format!("tools/{}", app.tool.id()),
        t::TEXT,
        20.0,
    );
    x += 30.0;
    w::vline(&p, x - 4.0, r.top() + 6.0, r.bottom() - 6.0, t::SEPARATOR);
    let mut next = |width: f32| {
        let at = Rect::from_min_size(pos2(x + 4.0, y), vec2(width, h));
        x += width + 8.0;
        at
    };
    let b = &mut app.brush;
    let out = w::slider(
        ui,
        next(150.0),
        "options.size",
        b.radius * 2.0,
        &SliderSpec::new("直径", 1.0, 256.0, NumberFormat::int(" px"))
            .tooltip("ブラシの直径（[ と ]）"),
    );
    if out.changed {
        b.radius = (out.value / 2.0).max(0.5);
    }
    let out = w::slider(
        ui,
        next(130.0),
        "options.hardness",
        b.hardness * 100.0,
        &SliderSpec::new("硬さ", 0.0, 100.0, NumberFormat::int("%")),
    );
    if out.changed {
        b.hardness = out.value / 100.0;
    }
    let out = w::slider(
        ui,
        next(130.0),
        "options.opacity",
        b.opacity * 100.0,
        &SliderSpec::new("不透明度", 0.0, 100.0, NumberFormat::int("%")),
    );
    if out.changed {
        b.opacity = out.value / 100.0;
    }
    let out = w::slider(
        ui,
        next(120.0),
        "options.flow",
        b.flow * 100.0,
        &SliderSpec::new("流量", 0.0, 100.0, NumberFormat::int("%")),
    );
    if out.changed {
        b.flow = out.value / 100.0;
    }
    let out = w::slider(
        ui,
        next(120.0),
        "options.spacing",
        b.spacing * 100.0,
        &SliderSpec::new("間隔", 1.0, 100.0, NumberFormat::int("%"))
            .tooltip("ダブの間隔（直径に対する割合）"),
    );
    if out.changed {
        b.spacing = out.value / 100.0;
    }
    if w::icon_button(
        ui,
        next(28.0),
        "options.pressure-size",
        "stylus",
        "筆圧で直径を変える",
        b.pressure_size,
        true,
        20.0,
    )
    .clicked()
    {
        b.pressure_size = !b.pressure_size;
    }
    if w::icon_button(
        ui,
        next(28.0),
        "options.pressure-opacity",
        "opacity",
        "筆圧で不透明度を変える",
        b.pressure_opacity,
        true,
        20.0,
    )
    .clicked()
    {
        b.pressure_opacity = !b.pressure_opacity;
    }
}

/// ツールの帯（左）。
pub fn tool_strip(ui: &mut Ui, app: &mut AppState, r: Rect) {
    let p = ui.painter().clone();
    w::fill(&p, r, t::PANEL_BG);
    w::vline(&p, r.right() - 1.0, r.top(), r.bottom(), t::BORDER);
    let mut y = r.top() + 6.0;
    for tool in Tool::ALL {
        let at = Rect::from_min_size(pos2(r.left() + 5.0, y), vec2(r.width() - 10.0, 32.0));
        let tip = format!("{}（{}）", tool.name(), tool.key());
        if w::tool_button(ui, at, tool.id(), &tip, app.tool == tool).clicked() {
            app.apply(Action::SelectTool(tool));
        }
        y += 34.0;
    }
}

/// ステータスバーの Live Link の欄の色と文（状態の帯）。
pub fn link_segment(app: &AppState) -> (egui::Color32, String) {
    let link = &app.link;
    let color = match &link.status {
        LinkStatus::Off => t::TEXT_DISABLED,
        LinkStatus::Failed(_) => t::ERROR,
        LinkStatus::Listening if link.mismatch.is_some() => t::ERROR,
        LinkStatus::Listening => t::TEXT_DIM,
        LinkStatus::Connected { .. } => match link.notice {
            Some((NoticeLevel::Error, _)) => t::WARNING,
            _ => t::ACCENT,
        },
    };
    (color, link.summary())
}

/// ステータスバー（左に知らせ、右に Live Link と文書の大きさとメモリと合成の経路）。
pub fn status_bar(ui: &mut Ui, app: &AppState, r: Rect, uploaded: usize) {
    let p = ui.painter().clone();
    w::fill(&p, r, t::MENU_BG);
    w::hline(&p, r.left(), r.right(), r.top(), t::BORDER);
    // Live Link（切っている間は出さない）
    let mut r = r;
    if app.link.status != LinkStatus::Off || app.link.notice.is_some() {
        let (color, text) = link_segment(app);
        let tw = w::text_width(&p, &text, t::LABEL_SMALL);
        let seg = Rect::from_min_max(pos2(r.right() - tw - 30.0, r.top()), r.max);
        p.circle_filled(pos2(seg.left() + 10.0, seg.center().y), 4.0, color);
        w::text(
            &p,
            Rect::from_min_max(pos2(seg.left() + 20.0, seg.top()), seg.max),
            &text,
            t::LABEL_SMALL.with_color(if color == t::TEXT_DISABLED {
                t::TEXT_DIM
            } else {
                color
            }),
            Align::Left,
        );
        w::vline(
            &p,
            seg.left(),
            r.top() + 4.0,
            r.bottom() - 4.0,
            t::SEPARATOR,
        );
        let tip = match (&app.link.status, &app.link.mismatch, &app.link.notice) {
            (LinkStatus::Failed(e), _, _) => e.clone(),
            (_, Some(m), _) => format!("版が合いません: {m}"),
            (_, _, Some((_, n))) => n.clone(),
            _ => text.clone(),
        };
        ui.interact(seg, ui.id().with("status.link"), Sense::hover())
            .on_hover_text(tip);
        r = Rect::from_min_max(r.min, pos2(seg.left(), r.bottom()));
    }
    let mib = |b: u64| b as f64 / 1048576.0;
    let right = format!(
        "{} × {}   レイヤー {:.1} MiB   履歴 {:.1} MiB   上げたタイル {}   CPU で合成",
        app.doc.width(),
        app.doc.height(),
        mib(app.doc.allocated_bytes()),
        mib(app.doc.history_bytes()),
        uploaded
    );
    let rw = (r.width() * 0.6).min(w::text_width(&p, &right, t::LABEL_SMALL) + 16.0);
    w::text(
        &p,
        Rect::from_min_max(
            pos2(r.left() + 8.0, r.top()),
            pos2(r.right() - rw - 8.0, r.bottom()),
        ),
        &app.message,
        t::LABEL_DIM,
        Align::Left,
    );
    w::text(
        &p,
        Rect::from_min_max(
            pos2(r.right() - rw - 8.0, r.top()),
            pos2(r.right() - 8.0, r.bottom()),
        ),
        &right,
        t::LABEL_SMALL,
        Align::Right,
    );
}
