//! 窓の外枠（Unity 版の Shell）: メニューの中身、オプションバー（今のツールの設定を 1 行で）、ツールの帯、ステータスバー、
//! キーの割り当て。

use egui::{pos2, vec2, Key, Modifiers, Rect, Sense, Ui};

use yolu_core::export::ExportTemplate;

use crate::bake::BakeAction;
use crate::export::ExportAction;
use crate::lang::Lang;
use crate::layerops::{lock_name, Xform, LOCK_FLAGS};
use crate::livelink::LinkIndicator;
use crate::m2::{Edit, UiOp};
use crate::pathtool::PathAction;
use crate::psd::{PsdAction, PsdTarget};
use crate::selection::{SelAction, SelEdit};
use crate::shelf::ShelfOp;
use crate::state::{Action, AppState, PopupKind, Tool};
use crate::ui::menu::Entry;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, NumberFormat, SliderSpec};
use crate::update::UpdateAction;
use crate::view3d::pose::PoseAction;

/// メニューバーの見出し（日本語）。
pub const MENU_TITLES: [&str; 6] = ["ファイル", "編集", "レイヤー", "選択範囲", "表示", "ヘルプ"];

/// ヘルプの見出しの番号。
pub const HELP_MENU: usize = 5;

/// 言語ごとのメニューバーの見出し。
pub fn menu_titles(lang: Lang) -> [&'static str; 6] {
    [
        lang.pick("ファイル", "File"),
        lang.pick("編集", "Edit"),
        lang.pick("レイヤー", "Layer"),
        lang.pick("選択範囲", "Select"),
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
        Entry::item(
            l.pick("チャンネルを PNG…", "Channel as PNG…"),
            Action::Export(ExportAction::ChannelDialog),
        )
        .enabled(free),
        Entry::item(
            l.pick("全チャンネルを画像に…", "All Channels as Images…"),
            Action::Export(ExportAction::ChannelsDialog),
        )
        .enabled(free),
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
                Entry::item(
                    l.pick("復旧…", "Recovery…"),
                    Action::Recovery(crate::recovery::RecoveryAction::OpenWindow),
                )
                .enabled(free && app.recovery.is_enabled()),
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
                Entry::item(
                    l.pick("プロジェクト設定…", "Project Configuration…"),
                    Action::Project(crate::newproject::NpAction::OpenConfigure),
                )
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
        1 => {
            let mut entries = vec![
                Entry::item(l.pick("取り消し", "Undo"), Action::Undo)
                    .shortcut("Ctrl+Z")
                    .enabled(free && app.can_undo()),
                Entry::item(l.pick("やり直し", "Redo"), Action::Redo)
                    .shortcut("Ctrl+Shift+Z / Ctrl+Y")
                    .enabled(free && app.can_redo()),
                Entry::Separator,
            ];
            entries.extend(crate::clipboard::menu_entries(app));
            entries.push(Entry::Separator);
            entries.extend([
                Entry::item(Tool::Brush.name_in(l), Action::SelectTool(Tool::Brush))
                    .shortcut("B")
                    .radio(app.tool == Tool::Brush),
                Entry::item(Tool::Eraser.name_in(l), Action::SelectTool(Tool::Eraser))
                    .shortcut("E")
                    .radio(app.tool == Tool::Eraser),
                Entry::item(Tool::Fill.name_in(l), Action::SelectTool(Tool::Fill))
                    .shortcut("G")
                    .radio(app.tool == Tool::Fill),
                Entry::item(Tool::Gradient.name_in(l), Action::SelectTool(Tool::Gradient))
                    .shortcut("Shift+G")
                    .radio(app.tool == Tool::Gradient),
                Entry::item(
                    Tool::PolygonFill.name_in(l),
                    Action::SelectTool(Tool::PolygonFill),
                )
                .shortcut("4")
                .radio(app.tool == Tool::PolygonFill),
                Entry::item(Tool::Move.name_in(l), Action::SelectTool(Tool::Move))
                    .shortcut("V")
                    .radio(app.tool == Tool::Move),
                Entry::Separator,
                transform_entry(l, Xform::Flip { horizontal: true }, free),
                transform_entry(l, Xform::Flip { horizontal: false }, free),
                transform_entry(l, Xform::Rotate90 { clockwise: true }, free),
                transform_entry(l, Xform::Rotate90 { clockwise: false }, free),
                Entry::item(
                    Tool::Eyedropper.name_in(l),
                    Action::SelectTool(Tool::Eyedropper),
                )
                .shortcut("I")
                .radio(app.tool == Tool::Eyedropper),
                Entry::item(Tool::Path.name_in(l), Action::SelectTool(Tool::Path))
                    .shortcut("P")
                    .radio(app.tool == Tool::Path),
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
            ]);
            entries
        }
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
        3 => crate::selection::menu::select_menu(app),
        4 => vec![
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
                l.pick("設定…", "Settings…"),
                Action::Prefs(crate::prefs::PrefsAction::Open),
            ),
            Entry::item(
                l.pick("パネルの並びを戻す", "Reset Panel Layout"),
                Action::ResetLayout,
            ),
        ],
        _ => help_entries(app),
    }
}

/// ヘルプのメニュー。更新の項目は、公開鍵を組み込んだビルドだけに出る（新しい版があれば先頭に）。
fn help_entries(app: &AppState) -> Vec<Entry<Action>> {
    let l = app.lang;
    let about = Entry::item(
        l.pick("YoluPainter について", "About YoluPainter"),
        Action::About,
    );
    if !app.update.enabled() {
        return vec![about];
    }
    let busy = app.update.is_busy();
    let mut entries = Vec::new();
    if let Some(label) = app.update.install_label(l) {
        entries.push(
            Entry::item(label, Action::Update(UpdateAction::Install))
                .enabled(!busy && !app.is_stroking()),
        );
        entries.push(Entry::Separator);
    }
    entries.push(
        Entry::item(
            l.pick("更新を確かめる…", "Check for Updates…"),
            Action::Update(UpdateAction::Check),
        )
        .enabled(!busy),
    );
    let on = app.update.preference() == crate::update::Preference::On;
    entries.push(
        Entry::item(
            l.pick("起動時に更新を確かめる", "Check for Updates at Startup"),
            Action::Update(UpdateAction::SetCheckOnStartup(!on)),
        )
        .checked(on),
    );
    entries.push(Entry::Separator);
    entries.push(about);
    entries
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
                Entry::Separator,
                Entry::item(
                    l.pick("消す…", "Remove…"),
                    Action::Project(crate::newproject::NpAction::RemoveSets(vec![uid])),
                )
                .enabled(!app.is_stroking() && app.sets.len() > 1),
            ]
        }
        PopupKind::LayerContext(id) => layer_context(app, id),
        PopupKind::Shelf => crate::panels::assets::menu_entries(app),
        PopupKind::View3dShading => crate::view3d::display::entries(app),
        PopupKind::Symmetry => crate::selection::menu::symmetry_menu(app),
        PopupKind::LiveLink => link_entries(app),
    }
}

/// 変形のメニューの項目（左右反転・上下反転・90° 回転）。
fn transform_entry(l: Lang, x: Xform, free: bool) -> Entry<Action> {
    let name = match x {
        Xform::Flip { horizontal: true } => l.pick("左右反転", "Flip Horizontal"),
        Xform::Flip { horizontal: false } => l.pick("上下反転", "Flip Vertical"),
        Xform::Rotate90 { clockwise: true } => {
            l.pick("時計回りに 90° 回転", "Rotate 90° Clockwise")
        }
        _ => l.pick("反時計回りに 90° 回転", "Rotate 90° Counter-clockwise"),
    };
    Entry::item(name, Action::M2(Edit::Transform(x))).enabled(free)
}

/// レイヤーの右クリックのメニュー（複数選んでいれば、複製・グループ化・結合・ロック・変形・表示・削除は選んだ層の全部に効く）。
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
    let selected = app.selected_layers();
    let multi = selected.len() > 1 && selected.contains(&id);
    let members = app.doc.topmost_of(&selected).unwrap_or_default();
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
    let duplicate = Entry::item(
        lang.pick("複製", "Duplicate"),
        Action::M2(Edit::DuplicateSelected),
    )
    .enabled(free);
    // 選択範囲があるあいだの Ctrl+J は「コピーして新しいレイヤー」（選択範囲のメニューと帯）。複製のキーは、選択範囲が無いときだけ出す
    v.push(if app.doc.selection().is_some() {
        duplicate
    } else {
        duplicate.shortcut("Ctrl+J")
    });
    if group && !multi {
        v.push(
            Entry::item(
                lang.pick("グループ解除", "Ungroup"),
                Action::M2(Edit::Ungroup(id)),
            )
            .shortcut("Ctrl+Shift+G")
            .enabled(free),
        );
    } else {
        v.push(
            Entry::item(
                lang.pick("レイヤーをグループ化", "Group Layers"),
                Action::M2(Edit::GroupSelected),
            )
            .shortcut("Ctrl+G")
            .enabled(free && app.selected_layer == Some(id)),
        );
    }
    // 結合（できない理由は、押したあとに短い文で言う。複数選んでいればその層を、グループならグループを、そうでなければ下の層と）
    let merge_label = if members.len() > 1 {
        lang.pick("レイヤーを結合", "Merge Layers")
    } else if group {
        lang.pick("グループを結合", "Merge Group")
    } else {
        lang.pick("下のレイヤーと結合", "Merge Down")
    };
    v.push(
        Entry::item(merge_label, Action::M2(Edit::MergeDown))
            .shortcut("Ctrl+E")
            .enabled(free && app.selected_layer == Some(id)),
    );
    v.push(
        Entry::item(
            lang.pick("表示レイヤーを結合", "Merge Visible"),
            Action::M2(Edit::MergeVisible),
        )
        .shortcut("Ctrl+Shift+E")
        .enabled(free),
    );
    if layer.is_some_and(|l| l.path().is_some()) {
        v.push(
            Entry::item(
                lang.pick("パスをラスタライズ", "Rasterize Path"),
                Action::Path(PathAction::Rasterize(id)),
            )
            .enabled(free),
        );
    }
    // アセットの棚へ（層のまとまり・マスク）
    v.push(
        Entry::item(
            lang.pick("スマートマテリアルとして保存", "Save as Smart Material"),
            Action::Shelf(ShelfOp::SaveMaterial(id)),
        )
        .enabled(free),
    );
    if has_mask {
        v.push(
            Entry::item(
                lang.pick(
                    "マスクをスマートマスクとして保存",
                    "Save Mask as Smart Mask",
                ),
                Action::Shelf(ShelfOp::SaveMask(id)),
            )
            .enabled(free),
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
    // ロック（選んでいる層の全部に効く。持っているロックにチェック）
    let targets = if multi { selected.clone() } else { vec![id] };
    v.push(Entry::Separator);
    v.push(Entry::Heading(lang.pick("ロック", "Lock").to_owned()));
    for flag in LOCK_FLAGS {
        let own = targets
            .iter()
            .all(|t| app.doc.layer(*t).is_some_and(|l| l.locks().contains(flag)));
        v.push(
            Entry::item(
                lock_name(lang, flag),
                Action::M2(Edit::Lock {
                    ids: targets.clone(),
                    flag,
                    on: !own,
                }),
            )
            .checked(own)
            .enabled(free),
        );
    }
    v.push(Entry::Separator);
    v.push(Entry::Heading(
        lang.pick("変形", "Transform").to_owned(),
    ));
    for x in [
        Xform::Flip { horizontal: true },
        Xform::Flip { horizontal: false },
        Xform::Rotate90 { clockwise: true },
        Xform::Rotate90 { clockwise: false },
    ] {
        v.push(transform_entry(lang, x, free && app.selected_layer == Some(id)));
    }
    v.push(Entry::Separator);
    v.push(Entry::item(lang.pick("名前を変更", "Rename"), Action::StartRename(id)).enabled(free));
    if multi {
        v.push(
            Entry::item(
                lang.pick("表示を切り替え", "Toggle Visibility"),
                Action::M2(Edit::ToggleSelectedVisible),
            )
            .enabled(free),
        );
    } else {
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
    }
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
    let removed: usize = members
        .iter()
        .map(|m| m2::subtree_len(&app.doc, *m))
        .sum::<usize>()
        .max(m2::subtree_len(&app.doc, id));
    v.push(
        Entry::item(
            lang.pick("レイヤーを削除", "Delete Layer"),
            Action::DeleteLayer,
        )
        .enabled(free && app.doc.layers().len() > removed),
    );
    v
}

fn sel_edit(edit: SelEdit) -> Action {
    Action::Sel(SelAction::Edit(edit))
}

/// キーの割り当て（文字を打っている間・メニューを開いている間は見ない。メニューは自分でキーを見る）。
pub fn handle_shortcuts(ctx: &egui::Context, app: &mut AppState) {
    if ctx.egui_wants_keyboard_input()
        || app.popup.is_some()
        || app.sel.dialog.is_some()
        || crate::windows::modal_open(app)
    {
        ctx.input(|i| crate::clipboard::keys::observe_blocked(i, &mut app.clip));
        return;
    }
    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    let has_selection = app.doc.selection().is_some();
    let mut actions = Vec::new();
    // 移動・変形の道具: 矢印キーで 1 画素（Shift で 10）。ドラッグの途中・描いている間は動かさない。キャンバスのタブが後ろにあって
    // 見えていない（3D ビューなどが前）ときも動かさない（このフレームの前に描いていなければ後ろ。複数パスの同じフレームは前）
    let canvas_shown = app
        .canvas_frame
        .is_some_and(|f| ctx.cumulative_frame_nr().saturating_sub(f) <= 1);
    let arrows_move = app.tool == Tool::Move
        && canvas_shown
        && !app.is_stroking()
        && app.transform.drag.is_none();
    let mut arrows: Vec<((f64, f64), bool)> = Vec::new();
    ctx.input_mut(|i| {
        // コピー・カット・ペースト（X などの修飾なしのキーより先に取る）
        actions.extend(crate::clipboard::keys::shortcut_actions(i, &mut app.clip));
        if arrows_move {
            for (k, dir) in [
                (Key::ArrowLeft, (-1.0, 0.0)),
                (Key::ArrowRight, (1.0, 0.0)),
                (Key::ArrowUp, (0.0, -1.0)),
                (Key::ArrowDown, (0.0, 1.0)),
            ] {
                if i.consume_key(Modifiers::SHIFT, k) {
                    arrows.push((dir, true));
                } else if i.consume_key(Modifiers::NONE, k) {
                    arrows.push((dir, false));
                }
            }
        }
        let mut key = |m: Modifiers, k: Key, a: Action| {
            if i.consume_key(m, k) {
                actions.push(a);
            }
        };
        // Shift 付きを先に取る（consume_key は書いていない Shift を気にしない。取った押下は消えるので、次の Ctrl+Z には残らない）
        key(cmd_shift, Key::E, Action::M2(Edit::MergeVisible));
        key(cmd_shift, Key::G, Action::M2(Edit::UngroupSelected));
        key(Modifiers::COMMAND, Key::E, Action::M2(Edit::MergeDown));
        // Ctrl+J: 選択範囲があれば、その画素を新しいレイヤーへ（Photoshop の「コピーしたレイヤー」）。無ければレイヤーの複製
        if has_selection {
            key(Modifiers::COMMAND, Key::J, sel_edit(SelEdit::ToNewLayer));
        }
        key(Modifiers::COMMAND, Key::J, Action::M2(Edit::DuplicateSelected));
        key(Modifiers::COMMAND, Key::G, Action::M2(Edit::GroupSelected));
        key(cmd_shift, Key::I, sel_edit(SelEdit::Invert));
        key(cmd_shift, Key::Z, Action::Redo);
        key(Modifiers::COMMAND, Key::A, sel_edit(SelEdit::All));
        key(Modifiers::COMMAND, Key::D, sel_edit(SelEdit::Clear));
        // 選択範囲があるときだけ: 消去（Delete）
        if has_selection {
            key(Modifiers::NONE, Key::Delete, sel_edit(SelEdit::Erase));
        }
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
        // Shift 付きの道具を先に（consume_key は書いていない Shift を気にしない）
        key(
            Modifiers::SHIFT,
            Key::M,
            Action::SelectTool(Tool::SelectEllipse),
        );
        key(Modifiers::SHIFT, Key::L, Action::SelectTool(Tool::Polygon));
        key(Modifiers::SHIFT, Key::W, Action::SelectTool(Tool::IdSelect));
        key(Modifiers::SHIFT, Key::G, Action::SelectTool(Tool::Gradient));
        key(
            Modifiers::NONE,
            Key::M,
            Action::SelectTool(Tool::SelectRect),
        );
        key(Modifiers::NONE, Key::V, Action::SelectTool(Tool::Move));
        key(Modifiers::NONE, Key::L, Action::SelectTool(Tool::Lasso));
        key(Modifiers::NONE, Key::W, Action::SelectTool(Tool::Wand));
        key(Modifiers::NONE, Key::B, Action::SelectTool(Tool::Brush));
        key(Modifiers::NONE, Key::E, Action::SelectTool(Tool::Eraser));
        key(Modifiers::NONE, Key::G, Action::SelectTool(Tool::Fill));
        key(Modifiers::NONE, Key::Num4, Action::SelectTool(Tool::PolygonFill));
        key(Modifiers::NONE, Key::I, Action::SelectTool(Tool::Eyedropper));
        key(Modifiers::NONE, Key::P, Action::SelectTool(Tool::Path));
        // パスの道具: 選んでいる点（無ければ最後の点）を消す
        if app.tool == Tool::Path {
            key(Modifiers::NONE, Key::Delete, Action::Path(PathAction::DeleteSelected));
            key(Modifiers::NONE, Key::Backspace, Action::Path(PathAction::DeleteSelected));
        }
        key(
            Modifiers::NONE,
            Key::Q,
            Action::Fill(crate::fillfx::FillOp::ToggleHandles),
        );
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
    for (dir, shift) in arrows {
        crate::transform::canvas::arrow(app, dir, shift);
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
    if app.tool.is_select() {
        crate::selection::props::select_options(ui, app, r, x + 4.0);
        return;
    }
    if app.tool == Tool::Move {
        crate::transform::props::options(ui, app, r, x + 4.0);
        return;
    }
    if app.tool == Tool::Eyedropper {
        crate::eyedrop::options(ui, app, r, x + 4.0);
        return;
    }
    // パスの道具は、点の太さ・閉じる・点を消す・ラスタライズ
    if app.tool.is_path() {
        crate::panels::path_props::options(ui, app, r, x);
        return;
    }
    if app.tool == Tool::Gradient {
        crate::gradient::props::options(ui, app, r, x);
        return;
    }
    // 範囲の道具（バケツ・ポリゴン塗りつぶし・ID の色で選択）は、その道具の設定
    if app.tool.is_region() {
        crate::panels::region_props::options(ui, app, r, x);
        return;
    }
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
    // 対称（右端。左の部品に重なるほど狭ければ出さない）
    crate::selection::props::symmetry_options(ui, app, r, x);
}

/// ツールの帯（左）。
pub fn tool_strip(ui: &mut Ui, app: &mut AppState, r: Rect) {
    let p = ui.painter().clone();
    w::fill(&p, r, t::PANEL_BG);
    w::vline(&p, r.right() - 1.0, r.top(), r.bottom(), t::BORDER);
    let mut y = r.top() + 6.0;
    for tool in Tool::ALL {
        if tool == Tool::SelectRect || tool == Tool::Move {
            // 描く道具と選ぶ道具・選ぶ道具と動かす道具（移動・変形とパス）の区切り
            w::strip_separator(
                &p,
                Rect::from_min_size(pos2(r.left(), y), vec2(r.width(), 9.0)),
            );
            y += 9.0;
        }
        let at = Rect::from_min_size(pos2(r.left() + 5.0, y), vec2(r.width() - 10.0, 32.0));
        let tip = app.lang.pick(
            format!("{}（{}）", tool.name_in(app.lang), tool.key()),
            format!("{} ({})", tool.name_in(app.lang), tool.key()),
        );
        if w::tool_button(ui, at, tool.id(), &tip, app.tool == tool).clicked() {
            app.apply(Action::SelectTool(tool));
        }
        y += 34.0;
    }
}

/// 状態の帯が出す文字（直前の操作の結果と理由だけ。文書の大きさ・メモリ・上げたタイル・合成の方式・Live Link の様子は出さない。
/// 画面のどこにも出さない開発用の数は、試験が `AppState` から読む）。
pub fn status_text(app: &AppState) -> &str {
    &app.message
}

/// ステータスバー: 直前の操作の結果と理由だけを左に出す。
pub fn status_bar(ui: &mut Ui, app: &AppState, r: Rect) {
    let p = ui.painter().clone();
    w::fill(&p, r, t::MENU_BG);
    w::hline(&p, r.left(), r.right(), r.top(), t::BORDER);
    let text = status_text(app);
    let area = Rect::from_min_max(pos2(r.left() + 8.0, r.top()), pos2(r.right() - 8.0, r.bottom()));
    let shown = w::fit(&p, text, area.width(), t::LABEL_DIM);
    w::text(&p, area, &shown, t::LABEL_DIM, Align::Left);
    if shown != text {
        ui.interact(area, ui.id().with("status.message"), Sense::hover())
            .on_hover_text(text);
    }
}

/// Live Link の入口の印の色。
pub fn link_indicator_color(indicator: LinkIndicator) -> egui::Color32 {
    match indicator {
        LinkIndicator::Off => t::TEXT_DISABLED,
        LinkIndicator::Waiting => t::ACCENT_DIM,
        LinkIndicator::Connected => t::OK,
        LinkIndicator::Mismatch => t::WARNING,
        LinkIndicator::Failed => t::ERROR,
    }
}

/// メニューバーの右端の Live Link の入口（Unity の印）の結果。
#[derive(Clone, Copy, Debug)]
pub struct LinkIcon {
    pub rect: Rect,
    /// このフレームで押された（窓を開いているあいだは受け皿が上にあるので、生の入力で見る）。
    pub pressed: bool,
}

/// Live Link の入口の印の幅と、名前の左端からその印の左端までの幅（印の幅と、名前との間の 8）。
const LINK_ICON_WIDTH: f32 = 24.0;
pub const LINK_ICON_SLOT: f32 = LINK_ICON_WIDTH + 8.0;

/// メニューバーの右端、プロジェクトの名前の左に、Live Link の入口の印を置く。`left_of` はプロジェクトの名前の左端の x。
pub fn link_icon(ui: &mut Ui, bar: Rect, left_of: f32, app: &AppState, open: bool) -> LinkIcon {
    let rect = Rect::from_min_size(
        pos2(left_of - LINK_ICON_SLOT, bar.top() + 1.0),
        vec2(LINK_ICON_WIDTH, bar.height() - 3.0),
    );
    let link = &app.link;
    let tip = link.tooltip(app.lang);
    let response = ui.interact(rect, ui.id().with("menubar.livelink"), Sense::hover());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, open, &tip)
    });
    let hover = response.hovered();
    let pressed = ui.input(|i| {
        i.pointer.primary_pressed() && i.pointer.press_origin().is_some_and(|at| rect.contains(at))
    });
    let p = ui.painter().clone();
    if hover || open {
        w::rounded(&p, rect, t::CONTROL_HOVER, 3.0);
    }
    w::unity_mark(&p, rect, link_indicator_color(link.indicator()));
    response.on_hover_text(tip);
    LinkIcon { rect, pressed }
}

/// Live Link の窓（入口の印を押すと開く）の中身: 状態・つながっている Unity・受け取ったモデルの名前と、待つ／切るの切り替え。
/// 文は名前と状態だけ（手順は README）。
pub fn link_entries(app: &AppState) -> Vec<Entry<Action>> {
    let l = app.lang;
    let link = &app.link;
    let on = link.is_on();
    let mut entries = vec![Entry::Heading(link.state_label(l).to_owned())];
    if let Some(unity) = link.unity_name() {
        entries.push(Entry::Heading(unity));
    }
    if let Some(model) = app.model.as_ref().filter(|m| m.is_link() && m.live) {
        entries.push(Entry::Heading(format!(
            "{}: {}",
            l.pick("モデル", "Model"),
            model.name
        )));
    }
    entries.push(Entry::Separator);
    entries.push(
        Entry::item(l.pick("待つ", "Wait"), Action::ToggleLiveLink)
            .radio(on)
            .enabled(!on),
    );
    entries.push(
        Entry::item(l.pick("切る", "Stop"), Action::ToggleLiveLink)
            .radio(!on)
            .enabled(on),
    );
    entries
}
