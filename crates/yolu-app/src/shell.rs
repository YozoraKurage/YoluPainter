//! 窓の外枠（Unity 版の Shell）: メニューの中身、オプションバー（今のツールの設定を 1 行で）、ツールの帯、ステータスバー、
//! キーの割り当て。

use egui::{pos2, vec2, Key, Modifiers, Rect, Sense, Ui};

use yolu_core::export::ExportTemplate;

use crate::bake::BakeAction;
use crate::export::ExportAction;
use crate::lang::Lang;
use crate::livelink::{LinkStatus, NoticeLevel};
use crate::m2::{Edit, UiOp};
use crate::psd::{PsdAction, PsdTarget};
use crate::state::{Action, AppState, PopupKind, Tool};
use crate::ui::menu::Entry;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, NumberFormat, SliderSpec};
use crate::view3d::pose::PoseAction;

/// メニューバーの見出し（日本語）。
pub const MENU_TITLES: [&str; 5] = ["ファイル", "編集", "レイヤー", "表示", "ヘルプ"];

/// 言語ごとのメニューバーの見出し。
pub fn menu_titles(lang: Lang) -> [&'static str; 5] {
    [
        lang.pick("ファイル", "File"),
        lang.pick("編集", "Edit"),
        lang.pick("レイヤー", "Layer"),
        lang.pick("表示", "View"),
        lang.pick("ヘルプ", "Help"),
    ]
}

/// ファイルメニューの読み込み（PSD）と書き出し（テンプレートの画像・PSD）。
fn import_export_entries(app: &AppState) -> Vec<Entry<Action>> {
    let free = !app.is_stroking();
    let l = app.lang;
    let mut entries = vec![
        Entry::Separator,
        Entry::Heading(l.pick("読み込み", "Import").to_owned()),
        Entry::item(
            l.pick(
                "PSD を新しいテクスチャセットへ…",
                "PSD as a New Texture Set…",
            ),
            Action::Psd(PsdAction::ImportDialog(PsdTarget::NewSet)),
        )
        .enabled(free),
        Entry::item(
            l.pick(
                "PSD を今のセットの文書へ…",
                "PSD as the Current Set's Document…",
            ),
            Action::Psd(PsdAction::ImportDialog(PsdTarget::CurrentSet)),
        )
        .enabled(free && app.read_only_reason().is_none()),
        Entry::Heading(l.pick("書き出し", "Export").to_owned()),
    ];
    for template in ExportTemplate::built_in() {
        entries.push(
            Entry::item(
                format!("{}: {}…", l.pick("テンプレート", "Template"), template.name),
                Action::Export(ExportAction::Template(template.id)),
            )
            .enabled(free),
        );
    }
    entries.push(
        Entry::item(l.pick("PSD…", "PSD…"), Action::Psd(PsdAction::ExportDialog)).enabled(free),
    );
    entries
}

/// メニューバーの見出しの中身。
pub fn menu_entries(app: &AppState, index: usize) -> Vec<Entry<Action>> {
    let free = !app.is_stroking();
    let l = app.lang;
    match index {
        0 => {
            let mut entries = vec![
                Entry::item(
                    l.pick("新規プロジェクト…", "New Project…"),
                    Action::NewProjectDialog,
                )
                .shortcut("Ctrl+N")
                .enabled(free),
                Entry::item(l.pick("開く…", "Open…"), Action::OpenProjectDialog)
                    .shortcut("Ctrl+O")
                    .enabled(free),
                Entry::Separator,
                Entry::item(l.pick("保存", "Save"), Action::SaveProject)
                    .shortcut("Ctrl+S")
                    .enabled(free),
                Entry::item(
                    l.pick("別名で保存…", "Save As…"),
                    Action::SaveProjectAsDialog,
                )
                .shortcut("Ctrl+Shift+S")
                .enabled(free),
                Entry::Separator,
                Entry::item("Live Link", Action::ToggleLiveLink).checked(app.link.is_on()),
                Entry::Separator,
                Entry::item(l.pick("終了", "Quit"), Action::Quit).shortcut("Ctrl+Q"),
            ];
            // 読み込み・書き出しは Live Link の前の区切りの前に入れる
            if let Some(at) = entries.iter().position(|e| {
                matches!(
                    e,
                    Entry::Item {
                        action: Action::ToggleLiveLink,
                        ..
                    }
                )
            }) {
                entries.splice(at - 1..at - 1, import_export_entries(app));
            }
            entries
        }
        1 => vec![
            Entry::item(l.pick("取り消し", "Undo"), Action::Undo)
                .shortcut("Ctrl+Z")
                .enabled(free && app.can_undo()),
            Entry::item(l.pick("やり直し", "Redo"), Action::Redo)
                .shortcut("Ctrl+Shift+Z / Ctrl+Y")
                .enabled(free && app.can_redo()),
            Entry::Separator,
            Entry::item(Tool::Brush.name_in(l), Action::SelectTool(Tool::Brush))
                .shortcut("B")
                .radio(app.tool == Tool::Brush),
            Entry::item(Tool::Eraser.name_in(l), Action::SelectTool(Tool::Eraser))
                .shortcut("E")
                .radio(app.tool == Tool::Eraser),
            Entry::Separator,
            Entry::item(
                l.pick("メインとサブの色を入れ替え", "Swap Main and Sub Colors"),
                Action::SwapColors,
            )
            .shortcut("X"),
            Entry::item(
                l.pick("初期設定の色", "Default Colors"),
                Action::DefaultColors,
            )
            .shortcut("D"),
        ],
        2 => match app.selected_layer.filter(|id| app.doc.layer(*id).is_some()) {
            // 選んでいるレイヤーの右クリックと同じ項目
            Some(id) => layer_context(app, id),
            None => vec![
                Entry::item(l.pick("新規レイヤー", "New Layer"), Action::NewLayer)
                    .shortcut("Ctrl+Shift+N")
                    .enabled(free),
                Entry::item(
                    l.pick("新規グループ", "New Group"),
                    Action::M2(Edit::NewGroup),
                )
                .enabled(free),
            ],
        },
        3 => vec![
            Entry::item(l.pick("ズームイン", "Zoom In"), Action::ZoomIn).shortcut("Ctrl++"),
            Entry::item(l.pick("ズームアウト", "Zoom Out"), Action::ZoomOut).shortcut("Ctrl+-"),
            Entry::item(l.pick("画面に合わせる", "Fit to Screen"), Action::FitView)
                .shortcut("Ctrl+0")
                .enabled(free),
            Entry::Separator,
            Entry::item(
                l.pick("表示を左に回す", "Rotate View Left"),
                Action::RotateLeft,
            )
            .shortcut("-")
            .enabled(free),
            Entry::item(
                l.pick("表示を右に回す", "Rotate View Right"),
                Action::RotateRight,
            )
            .shortcut("^")
            .enabled(free),
            Entry::item(
                l.pick("回転を戻す", "Reset Rotation"),
                Action::ResetRotation,
            )
            .shortcut("Shift+R")
            .enabled(free && app.view.angle != 0.0),
            Entry::item(l.pick("表示を左右反転", "Flip View"), Action::FlipView)
                .shortcut("H")
                .checked(app.view.flip)
                .enabled(free),
            Entry::Separator,
            Entry::item(
                l.pick(
                    "3D ビューに試しの立方体を読む",
                    "Load a Test Cube in the 3D View",
                ),
                Action::LoadDemoModel,
            )
            .enabled(free),
            Entry::item(
                l.pick("3D ビューに FBX を開く…", "Open an FBX in the 3D View…"),
                Action::Pose(PoseAction::OpenFbx),
            )
            .enabled(free),
            Entry::item(
                l.pick(
                    "3D ビューに試しの人形を読む",
                    "Load a Test Figure in the 3D View",
                ),
                Action::Pose(PoseAction::LoadFigure),
            )
            .enabled(free),
            Entry::item(
                l.pick("ポーズのモード", "Pose Mode"),
                Action::Pose(PoseAction::ToggleMode),
            )
            .checked(app.view3d.pose.mode)
            .enabled(free && app.view3d.pose.session.is_some()),
            Entry::item(
                l.pick(
                    "3D ビューでモデル全体を見る",
                    "Frame the Model in the 3D View",
                ),
                Action::FrameModel,
            )
            .enabled(free && app.view3d.model.is_some()),
            Entry::item(
                l.pick("メッシュマップをベイク…", "Bake Mesh Maps…"),
                Action::Bake(BakeAction::OpenWindow),
            ),
            Entry::Separator,
            Entry::Heading(l.pick("言語", "Language").to_owned()),
            Entry::item(Lang::Ja.name(), Action::M2Ui(UiOp::Language(Lang::Ja)))
                .radio(l == Lang::Ja),
            Entry::item(Lang::En.name(), Action::M2Ui(UiOp::Language(Lang::En)))
                .radio(l == Lang::En),
            Entry::Separator,
            Entry::item(
                l.pick("パネルの並びを戻す", "Reset Panel Layout"),
                Action::ResetLayout,
            ),
        ],
        _ => vec![Entry::item(
            l.pick("YoluPainter について", "About YoluPainter"),
            Action::About,
        )],
    }
}

/// ポップアップの中身（メニューバー・合成モード・レイヤーの右クリック）。
pub fn popup_entries(app: &AppState, kind: PopupKind) -> Vec<Entry<Action>> {
    match kind {
        PopupKind::MenuBar(i) => menu_entries(app, i),
        PopupKind::BlendMode(id) => {
            // 描くチャンネルが自分の合成を持っていれば、そのチャンネルの値を替える（層の値は変えない）
            let channel = app.m2.paint_channel;
            let layer = app.doc.layer(id);
            let own = layer.is_some_and(|l| !l.channel_blend(channel).is_empty());
            let current = layer.map(|l| l.blend_mode_in(channel));
            let group = layer.is_some_and(|l| l.is_group());
            crate::m2::blend_choices(group)
                .into_iter()
                .map(|m| {
                    Entry::item(
                        crate::m2::blend_label(app.lang, m),
                        Action::M2(Edit::BlendMode {
                            id,
                            channel: own.then_some(channel),
                            mode: m,
                        }),
                    )
                    .radio(current == Some(m))
                })
                .collect()
        }
        PopupKind::M2(popup) => crate::m2_menu::entries(app, popup),
        PopupKind::SetContext(uid) => {
            let set = app.sets.by_uid(uid);
            let visible = set.is_some_and(|s| s.visible);
            let writable = set.is_some_and(|s| s.read_only.is_none());
            let l = app.lang;
            vec![
                Entry::item(l.pick("名前を変更", "Rename"), Action::StartRenameSet(uid))
                    .enabled(!app.is_stroking() && writable),
                Entry::item(
                    if visible {
                        l.pick("隠す", "Hide")
                    } else {
                        l.pick("見せる", "Show")
                    },
                    Action::ToggleSetVisible(uid),
                ),
            ]
        }
        PopupKind::LayerContext(id) => layer_context(app, id),
    }
}

/// レイヤーの右クリックのメニュー。
fn layer_context(app: &AppState, id: crate::engine::LayerId) -> Vec<Entry<Action>> {
    use crate::m2::{self, AdjustmentKind, UiOp};
    let lang = app.lang;
    let free = !app.is_stroking();
    let layer = app.doc.layer(id);
    let visible = layer.map(|l| l.visible()).unwrap_or(true);
    let group = layer.is_some_and(|l| l.is_group());
    let has_mask = layer.is_some_and(|l| l.mask().is_some());
    let mask = layer.and_then(|l| l.mask());
    let editing = app.m2.edit_mask && app.selected_layer == Some(id);
    let mut v = vec![
        Entry::item(lang.pick("新規レイヤー", "New Layer"), Action::NewLayer).enabled(free),
        Entry::item(
            lang.pick("新規グループ", "New Group"),
            Action::M2(Edit::NewGroup),
        )
        .enabled(free),
        Entry::item(
            lang.pick("新規塗りつぶしレイヤー", "New Fill Layer"),
            Action::M2(Edit::NewFill),
        )
        .enabled(free),
    ];
    for k in AdjustmentKind::ALL {
        v.push(
            Entry::item(
                format!(
                    "{}: {}",
                    lang.pick("新規調整レイヤー", "New Adjustment Layer"),
                    k.name(lang)
                ),
                Action::M2(Edit::NewAdjustment(k)),
            )
            .enabled(free),
        );
    }
    v.push(Entry::Separator);
    v.push(
        Entry::item(
            lang.pick("複製", "Duplicate"),
            Action::M2(Edit::Duplicate(id)),
        )
        .enabled(free),
    );
    if group {
        v.push(
            Entry::item(
                lang.pick("グループ解除", "Ungroup"),
                Action::M2(Edit::Ungroup(id)),
            )
            .enabled(free),
        );
    } else {
        v.push(
            Entry::item(
                lang.pick("レイヤーをグループ化", "Group Layers"),
                Action::M2(Edit::GroupSelected),
            )
            .enabled(free && app.selected_layer == Some(id)),
        );
    }
    v.push(
        Entry::item(
            lang.pick("クリッピング", "Clipping"),
            Action::M2(Edit::Clipping(id, !layer.is_some_and(|l| l.clipping()))),
        )
        .checked(layer.is_some_and(|l| l.clipping()))
        .enabled(free),
    );
    v.push(Entry::Separator);
    if has_mask {
        v.push(
            Entry::item(
                lang.pick("マスクに描く", "Paint on Mask"),
                Action::M2Ui(UiOp::EditMask(!editing)),
            )
            .checked(editing)
            .enabled(free),
        );
        v.push(
            Entry::item(
                lang.pick("マスクを有効にする", "Enable Mask"),
                Action::M2(Edit::MaskEnabled(id, !mask.is_some_and(|m| m.enabled()))),
            )
            .checked(mask.is_some_and(|m| m.enabled()))
            .enabled(free),
        );
        v.push(
            Entry::item(
                lang.pick("マスクを反転", "Invert Mask"),
                Action::M2(Edit::MaskInverted(id, !mask.is_some_and(|m| m.inverted()))),
            )
            .checked(mask.is_some_and(|m| m.inverted()))
            .enabled(free),
        );
        v.push(
            Entry::item(
                lang.pick("レイヤーマスクを削除", "Delete Layer Mask"),
                Action::M2(Edit::RemoveMask(id)),
            )
            .enabled(free),
        );
    } else {
        v.push(
            Entry::item(
                lang.pick("レイヤーマスクを追加", "Add Layer Mask"),
                Action::M2(Edit::AddMask(id)),
            )
            .enabled(free),
        );
    }
    v.push(Entry::Separator);
    v.push(Entry::item(lang.pick("名前を変更", "Rename"), Action::StartRename(id)).enabled(free));
    v.push(
        Entry::item(
            if visible {
                lang.pick("非表示にする", "Hide")
            } else {
                lang.pick("表示する", "Show")
            },
            Action::ToggleVisible(id),
        )
        .enabled(free),
    );
    v.push(Entry::Separator);
    v.push(
        Entry::item(
            lang.pick("レイヤーを上へ", "Move Layer Up"),
            Action::LayerUp,
        )
        .enabled(free),
    );
    v.push(
        Entry::item(
            lang.pick("レイヤーを下へ", "Move Layer Down"),
            Action::LayerDown,
        )
        .enabled(free),
    );
    v.push(
        Entry::item(
            lang.pick("レイヤーを削除", "Delete Layer"),
            Action::DeleteLayer,
        )
        .enabled(free && app.doc.layers().len() > m2::subtree_len(&app.doc, id)),
    );
    v
}

/// キーの割り当て（文字を打っている間・メニューを開いている間は見ない。メニューは自分でキーを見る）。
pub fn handle_shortcuts(ctx: &egui::Context, app: &mut AppState) {
    if ctx.egui_wants_keyboard_input() || app.popup.is_some() || crate::windows::modal_open(app) {
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
    let l = app.lang;
    let b = &mut app.brush;
    let out = w::slider(
        ui,
        next(150.0),
        "options.size",
        b.radius * 2.0,
        &SliderSpec::new(l.pick("直径", "Size"), 1.0, 256.0, NumberFormat::int(" px"))
            .tooltip(l.pick("ブラシの直径（[ と ]）", "Brush diameter ([ and ])")),
    );
    if out.changed {
        b.radius = (out.value / 2.0).max(0.5);
    }
    let out = w::slider(
        ui,
        next(130.0),
        "options.hardness",
        b.hardness * 100.0,
        &SliderSpec::new(
            l.pick("硬さ", "Hardness"),
            0.0,
            100.0,
            NumberFormat::int("%"),
        ),
    );
    if out.changed {
        b.hardness = out.value / 100.0;
    }
    let out = w::slider(
        ui,
        next(130.0),
        "options.opacity",
        b.opacity * 100.0,
        &SliderSpec::new(
            l.pick("不透明度", "Opacity"),
            0.0,
            100.0,
            NumberFormat::int("%"),
        ),
    );
    if out.changed {
        b.opacity = out.value / 100.0;
    }
    let out = w::slider(
        ui,
        next(120.0),
        "options.flow",
        b.flow * 100.0,
        &SliderSpec::new(l.pick("流量", "Flow"), 0.0, 100.0, NumberFormat::int("%")),
    );
    if out.changed {
        b.flow = out.value / 100.0;
    }
    let out = w::slider(
        ui,
        next(120.0),
        "options.spacing",
        b.spacing * 100.0,
        &SliderSpec::new(
            l.pick("間隔", "Spacing"),
            1.0,
            100.0,
            NumberFormat::int("%"),
        )
        .tooltip(l.pick(
            "ダブの間隔（直径に対する割合）",
            "Distance between dabs (of the diameter)",
        )),
    );
    if out.changed {
        b.spacing = out.value / 100.0;
    }
    if w::icon_button(
        ui,
        next(28.0),
        "options.pressure-size",
        "stylus",
        l.pick("筆圧で直径を変える", "Pen pressure changes the size"),
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
        l.pick("筆圧で不透明度を変える", "Pen pressure changes the opacity"),
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
        let tip = format!("{}（{}）", tool.name_in(app.lang), tool.key());
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
    let right = match app.lang {
        Lang::Ja => format!(
            "{} × {}   レイヤー {:.1} MiB   履歴 {:.1} MiB   上げたタイル {}   CPU で合成",
            app.doc.width(),
            app.doc.height(),
            mib(app.doc.allocated_bytes()),
            mib(app.doc.history_bytes()),
            uploaded
        ),
        Lang::En => format!(
            "{} × {}   Layers {:.1} MiB   History {:.1} MiB   Uploaded tiles {}   CPU compositing",
            app.doc.width(),
            app.doc.height(),
            mib(app.doc.allocated_bytes()),
            mib(app.doc.history_bytes()),
            uploaded
        ),
    };
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
