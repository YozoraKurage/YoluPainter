//! ウィンドウの外枠（Unity 版の Shell）: メニューの中身、オプションバー（今のツールの設定を 1 行で）、ツールの帯、ステータスバー、
//! キーの割り当て。

use egui::{pos2, vec2, Rect, Sense, Ui};

use crate::export::ExportAction;
use crate::lang::Lang;
use crate::layerops::{menu_lock_name, Xform, LOCK_FLAGS};
use crate::livelink::LinkIndicator;
use crate::m2::Edit;
use crate::pathtool::PathAction;
use crate::psd::{PsdAction, PsdTarget};
use crate::shelf::ShelfOp;
use crate::state::{Action, AppState, PopupKind, Tool};
use crate::ui::menu::Entry;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};
use crate::update::UpdateAction;

/// メニューバーの見出し（日本語）。
pub const MENU_TITLES: [&str; 8] = [
    "ファイル",
    "編集",
    "レイヤー",
    "選択範囲",
    "フィルター",
    "表示",
    "ウィンドウ",
    "ヘルプ",
];

/// 「ウィンドウ」の見出しの番号。
pub const WINDOW_MENU: usize = 6;

/// ヘルプの見出しの番号。
pub const HELP_MENU: usize = 7;

/// 言語ごとのメニューバーの見出し。
pub fn menu_titles(lang: Lang) -> [&'static str; 8] {
    [
        lang.pick("ファイル", "File"),
        lang.pick("編集", "Edit"),
        lang.pick("レイヤー", "Layer"),
        lang.pick("選択範囲", "Select"),
        lang.pick("フィルター", "Filter"),
        lang.pick("表示", "View"),
        lang.pick("ウィンドウ", "Window"),
        lang.pick("ヘルプ", "Help"),
    ]
}

/// ファイルメニューのインポート（入れ子のメニュー。PSD を新しいテクスチャセットに・今のテクスチャセットに）と書き出し（テクスチャを書き出す・PSD）。
/// 見出しは置かず、インポートと書き出しは同じ組に並べる。
fn import_export_entries(app: &AppState) -> Vec<Entry<Action>> {
    let free = !app.is_stroking();
    let l = app.lang;
    vec![
        Entry::submenu(
            l.pick("インポート", "Import"),
            vec![
                Entry::item(
                    l.pick(
                        "PSD を新しいテクスチャセットに…",
                        "PSD as a New Texture Set…",
                    ),
                    Action::Psd(PsdAction::ImportDialog(PsdTarget::NewSet)),
                )
                .enabled(free),
                Entry::item(
                    l.pick(
                        "PSD を今のテクスチャセットに…",
                        "PSD into the Current Texture Set…",
                    ),
                    Action::Psd(PsdAction::ImportDialog(PsdTarget::CurrentSet)),
                )
                .enabled(free && app.read_only_reason().is_none()),
            ],
        )
        .enabled(free),
        Entry::item(
            l.pick("テクスチャを書き出す…", "Export Textures…"),
            Action::Export(ExportAction::OpenWindow),
        )
        .command_key("file.export")
        .enabled(free),
        Entry::item(
            l.pick("PSD を書き出す…", "Export PSD…"),
            Action::Psd(PsdAction::ExportDialog),
        )
        .enabled(free),
    ]
}

/// メニューバーの見出しの中身。
pub fn menu_entries(app: &AppState, index: usize) -> Vec<Entry<Action>> {
    // 項目のキーの文字は、今のモードで効く行から（編集のモードの H は印を隠すので、表示を左右反転に H を添えない）
    let _mode = crate::shortcuts::menu_mode(app.mode);
    let free = !app.is_stroking();
    // 保存の間は、プロジェクトを入れ替える・もう 1 度保存する・配布用に保存するは断る（描く・見るは止めない）
    let idle = free && !app.is_saving();
    let l = app.lang;
    // 保存のために押せない項目は、理由をツールチップに出す
    let why = |entry: Entry<Action>| match app.is_saving() {
        true => entry.tooltip(crate::lang::refusals::saving(l)),
        false => entry,
    };
    match index {
        0 => {
            let mut entries = vec![
                why(Entry::item(
                    l.pick("新規プロジェクト…", "New Project…"),
                    Action::NewProjectDialog,
                )
                .command_key("file.new_project")
                .enabled(idle)),
                why(
                    Entry::item(l.pick("開く…", "Open…"), Action::OpenProjectDialog)
                        .command_key("file.open")
                        .enabled(idle),
                ),
                why(Entry::item(
                    l.pick("復旧…", "Recovery…"),
                    Action::Recovery(crate::recovery::RecoveryAction::OpenWindow),
                )
                .enabled(idle && app.recovery.is_enabled())),
                Entry::Separator,
                why(Entry::item(l.pick("保存", "Save"), Action::SaveProject)
                    .command_key("file.save")
                    .enabled(idle)),
                why(Entry::item(
                    l.pick("別名で保存…", "Save As…"),
                    Action::SaveProjectAsDialog,
                )
                .command_key("file.save_as")
                .enabled(idle)),
                why(Entry::item(
                    l.pick("配布用に保存…", "Save for Distribution…"),
                    Action::Distribute(crate::distribute::DistributeAction::Start),
                )
                .enabled(idle && !app.distribute.is_open() && !app.distribute.is_busy())),
                Entry::Separator,
            ];
            // インポート・書き出し。そのあと、プロジェクト設定は Live Link のすぐ上
            entries.extend(import_export_entries(app));
            entries.extend([
                Entry::Separator,
                Entry::item(
                    l.pick("プロジェクト設定…", "Project Configuration…"),
                    Action::Project(crate::newproject::NpAction::OpenConfigure),
                )
                .enabled(free),
                Entry::Separator,
                Entry::item("Live Link", Action::ToggleLiveLink).checked(app.link.is_on()),
                Entry::Separator,
                Entry::item(l.pick("終了", "Quit"), Action::Quit).command_key("app.quit"),
            ]);
            entries
        }
        1 => {
            let mut entries = vec![
                Entry::item(l.pick("取り消し", "Undo"), Action::Undo)
                    .command_key("edit.undo")
                    .enabled(free && app.can_undo()),
                Entry::item(l.pick("やり直し", "Redo"), Action::Redo)
                    .command_key("edit.redo")
                    .enabled(free && app.can_redo()),
                Entry::Separator,
            ];
            entries.extend(crate::clipboard::menu_entries(app));
            entries.extend(crate::screen_pick::menu_entries(app));
            entries.push(Entry::Separator);
            // ツールの項目は、名前もキーもツールの表（`tools`）のとおり（メニューにキーを重ねて書かない）
            let tool_entry = |tool: Tool| {
                Entry::item(tool.name_in(l), Action::SelectTool(tool))
                    .command_key(crate::commands::tool_command(tool))
                    .radio(app.tool == tool)
            };
            entries.extend([
                tool_entry(Tool::Brush),
                tool_entry(Tool::Eraser),
                tool_entry(Tool::Fill),
                tool_entry(Tool::Shape),
                tool_entry(Tool::Ruler),
                tool_entry(Tool::Gradient),
                tool_entry(Tool::PolygonFill),
                tool_entry(Tool::Move),
                Entry::Separator,
                transform_entry(l, Xform::Flip { horizontal: true }, free),
                transform_entry(l, Xform::Flip { horizontal: false }, free),
                transform_entry(l, Xform::Rotate90 { clockwise: true }, free),
                transform_entry(l, Xform::Rotate90 { clockwise: false }, free),
                tool_entry(Tool::Eyedropper),
                tool_entry(Tool::Path),
                tool_entry(Tool::Text),
                Entry::Separator,
                Entry::item(
                    l.pick("メインとサブの色を入れ替え", "Swap Main and Sub Colors"),
                    Action::SwapColors,
                )
                .command_key("color.swap"),
                Entry::item(
                    l.pick("初期設定の色", "Default Colors"),
                    Action::DefaultColors,
                )
                .command_key("color.default"),
                // 設定は、Unity の Edit ▸ Preferences と同じく編集のメニューの一番下（区切りの後）
                Entry::Separator,
                Entry::item(
                    l.pick("設定…", "Settings…"),
                    Action::Prefs(crate::prefs::PrefsAction::Open),
                )
                .command_key("app.settings"),
            ]);
            entries
        }
        // 選んでいるレイヤーの右クリックと同じ項目（選んでいなければ、足す項目とグループだけ）
        2 => layer_menu(app, app.selected_layer),
        3 => crate::selection::menu::select_menu(app),
        4 => crate::fx::menu::menu_entries(app),
        5 => vec![
            crate::uv_wireframe::menu_entry(app),
            crate::uv_wireframe::overlap::menu_entry(app),
            Entry::item(l.pick("ズームイン", "Zoom In"), Action::ZoomIn)
                .command_key("view.zoom_in"),
            Entry::item(l.pick("ズームアウト", "Zoom Out"), Action::ZoomOut)
                .command_key("view.zoom_out"),
            Entry::item(l.pick("画面に合わせる", "Fit to Screen"), Action::FitView)
                .command_key("view.fit")
                .enabled(free),
            Entry::Separator,
            Entry::item(
                l.pick("表示を左に回す", "Rotate View Left"),
                Action::RotateLeft,
            )
            .command_key("view.rotate_left")
            .enabled(free),
            Entry::item(
                l.pick("表示を右に回す", "Rotate View Right"),
                Action::RotateRight,
            )
            .command_key("view.rotate_right")
            .enabled(free),
            Entry::item(
                l.pick("回転を戻す", "Reset Rotation"),
                Action::ResetRotation,
            )
            .command_key("view.reset_rotation")
            .enabled(free && app.view.angle != 0.0),
            Entry::item(l.pick("表示を左右反転", "Flip View"), Action::FlipView)
                .command_key("view.flip")
                .checked(app.view.flip)
                .enabled(free),
            Entry::Separator,
            Entry::item(
                l.pick("定規にスナップ", "Snap to Ruler"),
                Action::Ruler(crate::rulers::RulerAction::ToggleSnapRuler),
            )
            .command_key("view.ruler_snap")
            .checked(app.rulers.snap_ruler)
            .enabled(free),
            Entry::item(
                l.pick("特殊定規にスナップ", "Snap to Special Ruler"),
                Action::Ruler(crate::rulers::RulerAction::ToggleSnapSpecial),
            )
            .command_key("view.special_ruler_snap")
            .checked(app.rulers.snap_special)
            .enabled(free),
            Entry::item(
                l.pick(
                    "スナップする特殊定規の切り替え",
                    "Switch the Snapping Special Ruler",
                ),
                Action::Ruler(crate::rulers::RulerAction::SwitchSpecial),
            )
            .command_key("view.switch_special_ruler")
            .enabled(free),
            Entry::Separator,
            crate::mode::view_menu_entry(app),
            Entry::item(
                l.pick(
                    "3D ビューでモデル全体を見る",
                    "Frame the Model in the 3D View",
                ),
                Action::FrameModel,
            )
            .enabled(free && app.view3d.model.is_some()),
        ],
        WINDOW_MENU => crate::detach::menu::window_entries(app),
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
        return vec![
            Entry::item(
                l.pick("ログのフォルダを開く", "Open Log Folder"),
                Action::OpenLogFolder,
            ),
            about,
        ];
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
    entries.push(Entry::Separator);
    entries.push(Entry::item(
        l.pick("ログのフォルダを開く", "Open Log Folder"),
        Action::OpenLogFolder,
    ));
    entries.push(about);
    entries
}

/// ポップアップの中身（メニューバー・合成モード・レイヤーの右クリック）。
pub fn popup_entries(app: &AppState, kind: PopupKind) -> Vec<Entry<Action>> {
    match kind {
        PopupKind::MenuBar(i) => menu_entries(app, i),
        PopupKind::BlendMode(id) => {
            // 描くチャンネルが自分の合成を持っていれば、そのチャンネルの値を替える（レイヤーの値は変えない）
            let channel = app.m2.paint_channel;
            let layer = app.doc.layer(id);
            let own = layer.is_some_and(|l| !l.channel_blend(channel).is_empty());
            let current = layer.map(|l| l.blend_mode_in(channel));
            let group = layer.is_some_and(|l| l.is_group());
            // 頭: 描くチャンネルだけの合成モードと不透明度にする・レイヤーの値に戻す
            let name = crate::m2::channel_name(app.lang, &app.doc, channel);
            let lang = app.lang;
            let own_item = Entry::item(
                lang.pick(format!("{name} だけの値"), format!("{name} only")),
                Action::M2(Edit::OwnBlend {
                    id,
                    channel,
                    own: !own,
                }),
            )
            .checked(own)
            .tooltip(if own {
                lang.pick(
                    format!("{name} だけの合成モードと不透明度（押すとレイヤーの値に戻す）"),
                    format!(
                        "{name} only: its own blend mode and opacity (click to follow the layer)"
                    ),
                )
            } else {
                lang.pick(
                    format!("レイヤーの合成モードと不透明度（押すと {name} 専用にする）"),
                    format!("The layer's blend mode and opacity (click to give {name} its own)"),
                )
            });
            [own_item, Entry::Separator]
                .into_iter()
                .chain(crate::m2::blend_choices(group).into_iter().map(|m| {
                    Entry::item(
                        crate::m2::blend_label(lang, m),
                        Action::M2(Edit::BlendMode {
                            id,
                            channel: own.then_some(channel),
                            mode: m,
                        }),
                    )
                    .radio(current == Some(m))
                }))
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
                // 下の帯のベイクのボタンは、帯が狭いと隠れる。ここからも開ける（ベイクのウィンドウで、焼くセットを選ぶ）
                Entry::item(
                    l.pick("メッシュマップをベイク…", "Bake Mesh Maps…"),
                    Action::Bake(crate::bake::BakeAction::OpenWindow),
                )
                .command_key("bake.open"),
                Entry::Separator,
                Entry::item(
                    l.pick("消す…", "Remove…"),
                    Action::Project(crate::newproject::NpAction::RemoveSets(vec![uid])),
                )
                .enabled(!app.is_stroking() && app.sets.len() > 1),
            ]
        }
        PopupKind::LayerContext(id) => layer_context(app, id),
        PopupKind::RulerLayer(id) => crate::rulers::layer_icon::menu_entries(app, id),
        PopupKind::Shelf => crate::panels::assets::menu_entries(app),
        PopupKind::View3dShading => crate::view3d::display::entries(app),
        PopupKind::LiveLink => link_entries(app),
        PopupKind::BakeIsland {
            set, island, map, ..
        } => crate::bake::overlap::menu_entries(app, set, island, map),
        PopupKind::DockTab(tab) => crate::detach::menu::tab_entries(app, tab),
        PopupKind::Mode => crate::mode::entries(app),
        PopupKind::ToolKeyMode(command) => {
            crate::shortcuts::editor::tool_mode_entries(app, command)
        }
        // パイと G/R/S は項目の並びでなく、自分で描く（`pie::show`・`objects::transform::show`）
        PopupKind::Pie | PopupKind::Transform | PopupKind::KeyCapture => Vec::new(),
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

/// レイヤーの右クリックのメニュー（複数選んでいれば、複製・グループ化・結合・ロック・表示・削除は選んだレイヤーの全部に効く）。
fn layer_context(app: &AppState, id: crate::engine::LayerId) -> Vec<Entry<Action>> {
    layer_menu(app, Some(id))
}

/// 「レイヤー」のメニューの全体（メニューバーの「レイヤー」・レイヤーの右クリック・一覧の空白の右クリックが同じ関数を使う）。
/// 並びは 足す（新規レイヤー・塗りつぶし ▸・調整 ▸）→ 効果（フィルター ▸・ジェネレーター ▸・アンカー）→ グループ → 複製・結合など →
/// 属性（参照レイヤー・クリッピング・マスク・ロック）→ 名前・表示 → 順序・削除（左右反転・回転は「編集」のメニュー）。選んだレイヤーが無い（`None`）ときは、レイヤーに要らない
/// 先頭の足す項目とグループだけ。
pub fn layer_menu(app: &AppState, id: Option<crate::engine::LayerId>) -> Vec<Entry<Action>> {
    use crate::m2::{self, UiOp};
    let lang = app.lang;
    let free = !app.is_stroking();
    let mut v = crate::layermenu::creation_entries(app);
    let new_group = Entry::item(
        lang.pick("新規グループ", "New Group"),
        Action::M2(Edit::NewGroup),
    )
    .enabled(free);
    let Some(id) = id.filter(|id| app.doc.layer(*id).is_some()) else {
        v.push(Entry::Separator);
        v.push(new_group);
        return v;
    };
    let layer = app.doc.layer(id);
    let visible = layer.map(|l| l.visible()).unwrap_or(true);
    let group = layer.is_some_and(|l| l.is_group());
    let has_mask = layer.is_some_and(|l| l.mask().is_some());
    let mask = layer.and_then(|l| l.mask());
    let editing = app.m2.edit_mask && app.selected_layer == Some(id);
    let selected = app.selected_layers();
    let multi = selected.len() > 1 && selected.contains(&id);
    let members = app.doc.topmost_of(&selected).unwrap_or_default();
    // 効果（選んでいるレイヤーに足す。アンカーを置く・外す）
    v.push(Entry::Separator);
    v.extend(crate::fx::menu::layer_entries(app, id));
    // グループ（新規グループはレイヤーではないので、足す組でなくここ）
    v.push(Entry::Separator);
    v.push(new_group);
    if group && !multi {
        let ungroup = Entry::item(
            lang.pick("グループ解除", "Ungroup"),
            Action::M2(Edit::Ungroup(id)),
        )
        .enabled(free);
        // Ctrl+Shift+G は選んでいるレイヤーのグループ解除なので、右クリックしたグループが選んでいるレイヤーと同じときだけ出す
        v.push(if app.selected_layer == Some(id) {
            ungroup.command_key("layer.ungroup")
        } else {
            ungroup
        });
    } else {
        v.push(
            Entry::item(
                lang.pick("レイヤーをグループ化", "Group Layers"),
                Action::M2(Edit::GroupSelected),
            )
            .command_key("layer.group")
            .enabled(free && app.selected_layer == Some(id)),
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
        duplicate.command_key("layer.duplicate")
    });
    // 結合（できない理由は、押したあとに短い文で言う。複数選んでいればそのレイヤーを、グループならグループを、そうでなければ下のレイヤーと）
    let merge_label = if members.len() > 1 {
        lang.pick("レイヤーを結合", "Merge Layers")
    } else if group {
        lang.pick("グループを結合", "Merge Group")
    } else {
        lang.pick("下のレイヤーと結合", "Merge Down")
    };
    v.push(
        Entry::item(merge_label, Action::M2(Edit::MergeDown))
            .command_key("layer.merge_down")
            .enabled(free && app.selected_layer == Some(id)),
    );
    v.push(
        Entry::item(
            lang.pick("表示レイヤーを結合", "Merge Visible"),
            Action::M2(Edit::MergeVisible),
        )
        .command_key("layer.merge_visible")
        .enabled(free),
    );
    if layer.is_some_and(|l| l.path().is_some()) {
        // 塗りつぶしレイヤーのパスは画素にできない（パスの欄のボタンと同じ条件）
        let can = app.path_can_rasterize(id);
        let mut entry = Entry::item(
            lang.pick("パスをラスタライズ", "Rasterize Path"),
            Action::Path(PathAction::Rasterize(id)),
        )
        .enabled(free && can);
        if !can {
            entry = entry.tooltip(lang.pick(
                "塗りつぶしレイヤーのパスは画素にできません",
                "A path on a fill layer cannot become pixels",
            ));
        }
        v.push(entry);
    }
    if layer.is_some_and(|l| l.text().is_some()) {
        v.push(
            Entry::item(
                lang.pick("テキストをラスタライズ", "Rasterize Text"),
                Action::Text(crate::textlayer::TextAction::Rasterize(id)),
            )
            .enabled(free),
        );
    }
    // アセットの棚へ（レイヤーのまとまり・マスク）
    v.push(
        Entry::item(
            lang.pick("スマートマテリアルとして保存", "Save as Smart Material"),
            Action::Shelf(ShelfOp::SaveMaterial(id)),
        )
        .enabled(free),
    );
    // 塗りつぶしレイヤーだけ、マテリアルとしてライブラリへ（ほかの種類には出さない。保存できない塗りつぶしは押せなくし、理由はツールチップ）
    if let Some(l) = layer.filter(|l| l.kind() == crate::engine::LayerKind::Fill) {
        let refusal = crate::library::ops::material_refusal(lang, l);
        let entry = Entry::item(
            lang.pick("マテリアルとして保存", "Save as Material"),
            Action::Shelf(ShelfOp::SaveAsMaterial(id)),
        )
        .enabled(free && refusal.is_none());
        v.push(match refusal {
            Some(reason) => entry.tooltip(reason),
            None => entry,
        });
    }
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
    // 属性: 参照レイヤー・クリッピング、マスク、ロック
    v.push(Entry::Separator);
    v.push(
        Entry::item(
            lang.pick("参照レイヤー", "Reference Layer"),
            Action::Region(crate::region::RegionAction::ReferenceLayer(id)),
        )
        .checked(app.region.references.contains(&(app.doc.id(), id)))
        .enabled(free),
    );
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
    // ロック（選んでいるレイヤーの全部に効く。持っているロックにチェック）
    let targets = if multi { selected.clone() } else { vec![id] };
    v.push(Entry::Separator);
    for flag in LOCK_FLAGS {
        let own = targets
            .iter()
            .all(|t| app.doc.layer(*t).is_some_and(|l| l.locks().contains(flag)));
        v.push(
            Entry::item(
                menu_lock_name(lang, flag),
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

/// キーの割り当て（文字を打っている間・メニューを開いている間は見ない。メニューは自分でキーを見る）。割り当ては `keymap` の表。
pub fn handle_shortcuts(ctx: &egui::Context, app: &mut AppState) {
    // ツールのキーを押している間だけの切り替え: 離したか、戻す条件が揃ったかを先に見る（下の早い戻りでも離しを見落とさない）
    crate::toolkeys::update(ctx, app);
    // ショートカットの設定で割り当てに使ったキーは、離すまで繰り返しの押しを表へ渡さない
    crate::shortcuts::editor::swallow_held(ctx, app);
    // メニューなどから頼まれたパイを、ポインタの所に開く。外から閉じられた G/R/S はそこまでで決め、頼まれた G/R/S を始める
    crate::pie::open_requested(ctx, app);
    crate::objects::transform::settle(app);
    crate::objects::transform::start_requested(ctx, app);
    if ctx.egui_wants_keyboard_input()
        || app.popup.is_some()
        || app.sel.dialog.is_some()
        || crate::windows::modal_open(app)
    {
        ctx.input(|i| crate::clipboard::keys::observe_blocked(i, &mut app.clip));
        return;
    }
    let mut actions = Vec::new();
    // ツールのキーのうち、押している間だけの動き方のもの（切り替えと離しの戻しは `toolkeys` が持つ）
    let mut holds = Vec::new();
    // 移動・変形のツール: 矢印キーで 1 画素（Shift で 10）。ドラッグの途中・描いている間は動かさない。キャンバスのタブが後ろにあって
    // 見えていない（3D ビューなどが前）ときも動かさない（このフレームの前に描いていなければ後ろ。複数パスの同じフレームは前）
    let canvas_shown = app.ui.canvas_frame.is_some_and(|f| {
        ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT)
            .saturating_sub(f)
            <= 1
    });
    let arrows_move = app.tool == Tool::Move
        && canvas_shown
        && !app.is_stroking()
        && app.transform.drag.is_none();
    let mut arrows: Vec<((f64, f64), bool)> = Vec::new();
    // 右ボタンを押して 3D の視点を動かしている間は、W/A/S/D/Q/E を視点の移動に使う（ツールの切り替えなどの表のキーに渡さない）
    let flying = crate::view3d::navigation::flying(app);
    ctx.input_mut(|i| {
        // キーの段 1「途中の操作」: 右を押している間の W/A/S/D/Q/E は視点の移動（表の `Scope::During` の行。下の段へ渡さない）
        crate::keymap::take_fly_keys(i, &mut app.view3d.input.fly_held, flying);
        // ツールのキーを押している間の、そのキーの繰り返しの押し（修飾を先に離したあとも）は、表のほかの割り当てへ渡さない
        crate::toolkeys::swallow_repeats(i, app);
        // コピー・カット・ペースト（X などの修飾なしのキーより先に取る）。画素を変えるカット・ペーストは、ペイントのモードだけ
        let paints = app.mode.paints();
        actions.extend(
            crate::clipboard::keys::shortcut_actions(i, &mut app.clip)
                .into_iter()
                .filter(|a| {
                    paints || !matches!(a, Action::Clip(c) if crate::keymap::changes_pixels(*c))
                }),
        );
        if arrows_move {
            for nudge in crate::keymap::NUDGES {
                if crate::keymap::consume_command(i, app, nudge.command) {
                    arrows.push((nudge.direction, nudge.shift));
                }
            }
        }
        // キーの段 2〜4「場面」「モード」「どこでも」（判定の順は `keymap::Keymap::order`）。^ の文字の入力（キーの位置が配列で違う）も、表の行として判定に入る
        for fired in crate::keymap::dispatch_fired(&crate::keymap::current(), i, app) {
            if fired.mode == crate::toolkeys::ToolKeyMode::Tap {
                actions.push(fired.action);
            } else {
                holds.push(fired);
            }
        }
    });
    for a in actions {
        app.apply(a);
    }
    for fired in holds {
        crate::toolkeys::press(ctx, app, fired);
    }
    // 押したのと同じフレームで離していたら、ここで戻す条件を見る
    crate::toolkeys::update(ctx, app);
    // キーで開いたパイ（Ctrl+Tab など）は、そのキーを押している間に開く（押したまま離すと、指している項目を実行する）。G/R/S はポインタの所から始める
    crate::pie::open_requested(ctx, app);
    crate::objects::transform::start_requested(ctx, app);
    for (dir, shift) in arrows {
        crate::transform::canvas::arrow(app, dir, shift);
    }
}

/// オプションバー（今のツールの、よく使う 2〜3 個。ブラシと消しゴムは直径・不透明度と対称。ツールプロパティと同じ値を見せる）。
pub fn options_bar(ui: &mut Ui, app: &mut AppState, r: Rect) {
    let p = ui.painter().clone();
    w::fill(&p, r, t::PANEL_BG);
    w::hline(&p, r.left(), r.right(), r.bottom() - 1.0, t::BORDER);
    // 左端はモードのドロップダウン
    let mut x = crate::mode::dropdown(ui, app, r, r.left() + 8.0) + 8.0;
    w::vline(&p, x - 4.0, r.top() + 6.0, r.bottom() - 6.0, t::SEPARATOR);
    x += 4.0;
    if !app.mode.paints() {
        // 編集・ポーズ: 今のツールのアイコン（ツールの設定は無い）
        w::icon(
            &p,
            Rect::from_min_size(pos2(x, r.top()), vec2(22.0, r.height())),
            crate::mode::EditTool::shown(app).icon(),
            t::TEXT,
            20.0,
        );
        x += 30.0;
        w::vline(&p, x - 4.0, r.top() + 6.0, r.bottom() - 6.0, t::SEPARATOR);
        // G/R/S のスナップ（常にする・刻み）
        crate::mode::snap_options(ui, app, r, x);
        return;
    }
    w::icon(
        &p,
        Rect::from_min_size(pos2(x, r.top()), vec2(22.0, r.height())),
        &format!("tools/{}", app.tool.id()),
        t::TEXT,
        20.0,
    );
    x += 30.0;
    w::vline(&p, x - 4.0, r.top() + 6.0, r.bottom() - 6.0, t::SEPARATOR);
    // ツールごとの項目はツールの表（`tools`）が持つ
    (app.tool.def().options)(ui, app, r, x);
}

/// ツールの帯（左）。
pub fn tool_strip(ui: &mut Ui, app: &mut AppState, r: Rect) {
    let p = ui.painter().clone();
    w::fill(&p, r, t::PANEL_BG);
    w::vline(&p, r.right() - 1.0, r.top(), r.bottom(), t::BORDER);
    // ツールの列はツールの並び（`toolset`）のとおり（区切りはツールごとの「前に区切り」）。帯の下の端に付く 2 枚の色の分の高さを先に取る
    let bottom = r.bottom() - crate::panels::color_swatch::reserved_height();
    if app.mode.paints() {
        crate::toolset::ui::strip(ui, app, r, bottom);
        crate::panels::color_swatch::draw(ui, app, crate::panels::color_swatch::area(r));
    } else {
        // 編集・ポーズ: 選択・移動・回転・拡縮。色の 2 枚は暗くして押せない
        crate::mode::edit_strip(ui, app, r, bottom);
        let area = crate::panels::color_swatch::area(r);
        w::enabled_scope(ui, "mode.swatch", false, |ui| {
            crate::panels::color_swatch::draw(ui, app, area)
        });
        crate::mode::dimmed_reason(ui, app, area, "swatch");
    }
}

/// 直前の操作の結果と理由（`message`）。状態の帯の左には出さず、小さな知らせ（`toast`）が短く出して消す。試験が読む口はここ。
pub fn status_text(app: &AppState) -> &str {
    &app.message
}

/// 状態の帯: 左は何も出さない。右端に、版とビルド・使っているメモリ（`usage` が決める項目。ツールチップに内訳。実際のウィンドウだけで、測れない値は出さない）。
pub fn status_bar(ui: &mut Ui, app: &AppState, r: Rect) {
    let p = ui.painter().clone();
    w::fill(&p, r, t::MENU_BG);
    w::hline(&p, r.left(), r.right(), r.top(), t::BORDER);
    let mut right = r.right() - 10.0;
    // 外からの操作を受けている間だけ、右端に小さな丸（待っている・つながっている・受けられない。色が状態。説明はツールチップ）
    if let Some(indicator) = app.ops.indicator() {
        let tip = app.ops.tooltip(app.lang);
        let dot = Rect::from_center_size(
            pos2(right - OPS_DOT / 2.0, r.center().y),
            vec2(OPS_DOT, OPS_DOT),
        );
        p.circle_filled(dot.center(), OPS_DOT / 2.0, ops_indicator_color(indicator));
        let response = ui.interact(dot.expand(4.0), ui.id().with("status.ops"), Sense::hover());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &tip));
        response.on_hover_text(&tip);
        right = dot.left() - 16.0;
    }
    for item in app.usage.items(app.lang) {
        let width = w::text_width(&p, &item.text, t::LABEL_DIM);
        let at = Rect::from_min_max(pos2(right - width, r.top()), pos2(right, r.bottom()));
        w::text(&p, at, &item.text, t::LABEL_DIM, Align::Right);
        let response = ui.interact(
            at.expand2(vec2(4.0, 0.0)),
            ui.id().with(("status", item.key)),
            Sense::hover(),
        );
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &item.text));
        response.on_hover_text(&item.tip);
        right = at.left() - 16.0;
    }
}

/// 状態の帯の、外からの操作の印（丸）の直径。
const OPS_DOT: f32 = 8.0;

/// 外からの操作の印の色。
pub fn ops_indicator_color(indicator: crate::mcp_server::OpsIndicator) -> egui::Color32 {
    use crate::mcp_server::OpsIndicator;
    match indicator {
        OpsIndicator::Waiting => t::ACCENT_DIM,
        OpsIndicator::Connected => t::OK,
        OpsIndicator::Failed => t::ERROR,
    }
}

/// Live Link の入口の印の色。
pub fn link_indicator_color(indicator: LinkIndicator) -> egui::Color32 {
    match indicator {
        LinkIndicator::Off => t::TEXT_DISABLED,
        LinkIndicator::Waiting => t::ACCENT_DIM,
        LinkIndicator::Linked => t::OK,
        LinkIndicator::Problems => t::WARNING,
        LinkIndicator::Failed => t::ERROR,
    }
}

/// メニューバーの右端の Live Link の入口（Unity の印）の結果。
#[derive(Clone, Copy, Debug)]
pub struct LinkIcon {
    pub rect: Rect,
    /// このフレームで押された（ウィンドウを開いているあいだは受け皿が上にあるので、生の入力で見る）。
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
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, open, &tip));
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

/// Live Link のウィンドウ（入口の印を押すと開く）の中身: 「Live Link: 状態」・開いている Unity のオブジェクト（「Unity: 名前」）と、受け付ける／
/// 受け付けないの切り替え。文は名前と状態だけ（合わなかった物の理由は入口の印のツールチップ）。
pub fn link_entries(app: &AppState) -> Vec<Entry<Action>> {
    let l = app.lang;
    let link = &app.link;
    let on = link.is_on();
    let mut entries = vec![Entry::Heading(link.heading(l))];
    if let Some(target) = link.target_line() {
        entries.push(Entry::Heading(target));
    }
    entries.push(Entry::Separator);
    entries.push(
        Entry::item(l.pick("受け付ける", "Accept"), Action::ToggleLiveLink)
            .radio(on)
            .enabled(!on),
    );
    entries.push(
        Entry::item(
            l.pick("受け付けない", "Don't accept"),
            Action::ToggleLiveLink,
        )
        .radio(!on)
        .enabled(on),
    );
    entries
}
