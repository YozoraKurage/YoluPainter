//! 操作の一覧。キー・メニュー・ショートカットの一覧のウィンドウが指す、壊れない文字の ID と、その操作の種類・実行する `Action`・名前をここだけに書く。
//!
//! `Action` はデータを持つ列挙で、保存できる ID を持たない。キーの割り当て（`keymap`）・メニューのキーの文字（`shortcuts::menu_key`）・利用者の設定は
//! この ID を指す。**一度出した ID は変えない**（保存した設定が指す）。ID の形は `^[a-z0-9]+(\.[a-z0-9_]+)+$`（分野 `.` 名前。ツールは `tool.<ツールの id>`、
//! ツールの id の `-` は `_`）。
//!
//! 種類: `Press`（押して 1 回。`action` があれば `keymap::dispatch` が実行し、無ければ押しを読むビューが ID で引く）・`Hold`（押している間だけ効くキー）・
//! `Gesture`（マウスの組み合わせ。割り当ては `keymap::GESTURES`）・`Fixed`（画面の部品の決まった働きのキー）。割り当てを替えさせない操作は `locked`
//! （`Fixed` と、クリップボードの 4 つ。クリップボードは egui-winit が Ctrl+C・X・V を `Event::Copy` などに置き換えるので、割り当てを替えても効かない）。

use std::collections::HashMap;
use std::sync::OnceLock;

use yolu_core::geometry::AxisView;

use crate::clipboard::ClipAction;
use crate::engine::LayerId;
use crate::keymap::Operation;
use crate::lang::Lang;
use crate::m2::Edit;
use crate::mode::{EditorMode, ModeAction};
use crate::objects::{Kind as TransformKind, ObjectAction};
use crate::pathtool::PathAction;
use crate::pie::PieAction;
use crate::prefs::PrefsAction;
use crate::selection::{SelAction, SelEdit, SelUiOp};
use crate::state::key::Key;
use crate::state::{Action, AppState, Tool};
use crate::view3d::navigation::NavOp;

/// 操作の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// 押して 1 回。
    Press,
    /// 押している間だけ効く（Space・Y・N・3D で右ボタンを押している間の視点の移動キー。R は、割り当てがあれば 2D の回転）。
    Hold,
    /// マウスの組み合わせ（`keymap::GESTURES` の `Operation` ごと）。
    Gesture,
    /// 画面の部品の決まった働き（Esc・Enter・Backspace）。キーは操作の定義が持ち、割り当てを替えさせない。
    Fixed(Key),
}

/// 名前（日本語と英語）の引き方。
#[derive(Clone, Copy, Debug)]
pub enum Name {
    /// 実行する `Action` が自分の表で持つ名前（ツールの選びはツールの表、画面の色の取り方はその名前）。
    OfAction,
    /// 選んでいるレイヤーで言い方が変わる名前（レイヤーのメニューの項目も同じ関数で名前を付ける。引数は、メニューを出すレイヤー）。
    ByLayer(fn(&AppState, Option<LayerId>) -> &'static str),
    /// マウスの組み合わせの名前（`Operation::label`）。
    Mouse(Operation),
    /// 言語ごとの名前（`.ftl` の文）。
    Text(fn(Lang) -> &'static str),
}

/// 操作 1 つ。
#[derive(Clone, Copy, Debug)]
pub struct Command {
    pub id: &'static str,
    pub kind: Kind,
    /// キーで実行する `Action`。`Press` で無いものは、押しを読むビューが ID でキーを引く（`keymap::consume_command`）。
    pub action: Option<fn() -> Action>,
    pub name: Name,
    /// 割り当てを替えさせない（設定の画面で替えられない印）。
    pub locked: bool,
    /// パイ・メニューから実行するときの `Action`（キーをビューが読む `Press` で、ビューの外からも実行できるもの。`action` を持つものは要らない）。
    pub run: Option<fn() -> Action>,
    /// パイメニューの項目に出す短い名前（無ければ `label`）。
    pub short: Option<fn(Lang) -> &'static str>,
    /// このバージョンでは実行できない理由。パイの項目は押せず、ツールチップに理由を出す。
    pub unavailable: Option<fn(Lang) -> &'static str>,
    /// キーを押し続けたときの繰り返しの押しでも実行する（パイを開く操作は偽: 押したまま項目を選んで閉じたあと、繰り返しで開き直さない）。
    pub repeats: bool,
}

impl Command {
    /// 利用者が割り当てを替えてよいか。
    pub fn rebindable(&self) -> bool {
        !self.locked
    }

    /// 名前（メニューにある操作は、メニューと同じ名前。`app` は言語と、選んでいるレイヤーで変わる名前に使う）。
    pub fn label(&self, app: &AppState) -> Option<String> {
        match self.name {
            Name::OfAction => self
                .action
                .and_then(|make| action_name(app.lang, &make()))
                .map(Into::into),
            Name::ByLayer(name) => Some(name(app, app.session.selected_layer).into()),
            Name::Mouse(operation) => Some(operation.label(app.lang).into()),
            Name::Text(name) => Some(name(app.lang).into()),
        }
    }

    /// パイ・メニューから実行する `Action`（`action` か `run`。どちらも無ければ None）。
    pub fn runnable(&self) -> Option<Action> {
        self.action.or(self.run).map(|make| make())
    }

    /// パイメニューの項目の短い名前（無ければ None。呼ぶ側は `label` を使う）。
    pub fn short_label(&self, lang: Lang) -> Option<&'static str> {
        self.short.map(|name| name(lang))
    }

    /// このバージョンでは実行できない理由（実行できれば None）。
    pub fn unavailable(&self, lang: Lang) -> Option<&'static str> {
        self.unavailable.map(|reason| reason(lang))
    }

    /// 言語だけで引ける名前（選んでいるレイヤーで変わる名前は `AppState` が要るので None）。
    pub fn static_label(&self, lang: Lang) -> Option<&'static str> {
        match self.name {
            Name::OfAction => self.action.and_then(|make| action_name(lang, &make())),
            Name::ByLayer(_) => None,
            Name::Mouse(operation) => Some(operation.label(lang)),
            Name::Text(name) => Some(name(lang)),
        }
    }
}

/// `Action` が自分の表で持つ名前（ツールの選び・画面の色の取り方）。
fn action_name(lang: Lang, action: &Action) -> Option<&'static str> {
    match action {
        Action::SelectTool(tool) => Some(tool.name_in(lang)),
        Action::ScreenPick(mode) => Some(mode.label(lang)),
        _ => None,
    }
}

/// 操作（`Press`）の名前（メニューの項目はこの名前を使う）。言語だけで決まる名前を持たない操作は None。
pub fn name(id: &str, lang: Lang) -> Option<&'static str> {
    find(id)?.static_label(lang)
}

/// 言語だけで決まる名前を持つ操作の名前（表に無い・選んでいるレイヤーで変わる名前の ID は、書き間違いなのでここで落とす）。
pub fn label_of(id: &str, lang: Lang) -> &'static str {
    name(id, lang).unwrap_or_else(|| panic!("{id}: 言語だけで決まる名前を持つ操作"))
}

/// 「複製」の名前（レイヤーを選んでいるときの言い方。選んでいなければ「レイヤーを複製」）。
pub fn duplicate_name(app: &AppState, layer: Option<LayerId>) -> &'static str {
    if layer.is_some_and(|id| app.session.doc.layer(id).is_some()) {
        app.lang.t("commands-duplicate")
    } else {
        app.lang.t("commands-duplicate-layer")
    }
}

/// 結合の名前（複数選んでいればそのレイヤーを、グループならグループを、そうでなければ下のレイヤーと。レイヤーを選んでいなければ「レイヤーを結合」）。
pub fn merge_down_name(app: &AppState, layer: Option<LayerId>) -> &'static str {
    let l = app.lang;
    let Some(layer) = layer.and_then(|id| app.session.doc.layer(id)) else {
        return l.t("commands-merge-layers");
    };
    let members = app
        .session
        .doc
        .topmost_of(&app.selected_layers())
        .unwrap_or_default();
    if members.len() > 1 {
        l.t("commands-merge-layers")
    } else if layer.is_group() {
        l.t("commands-merge-group")
    } else {
        l.t("commands-merge-down")
    }
}

const fn press(id: &'static str, make: fn() -> Action) -> Command {
    Command {
        id,
        kind: Kind::Press,
        action: Some(make),
        name: Name::OfAction,
        locked: false,
        run: None,
        short: None,
        unavailable: None,
        repeats: true,
    }
}

/// 名前を表で持つ `Press`（メニューに同じ `Action` の項目があれば、その項目もこの名前を使う）。
const fn press_named(
    id: &'static str,
    name: fn(Lang) -> &'static str,
    make: fn() -> Action,
) -> Command {
    Command {
        name: Name::Text(name),
        ..press(id, make)
    }
}

/// パイメニューに短い名前で出る `Press`。
const fn press_short(
    id: &'static str,
    name: fn(Lang) -> &'static str,
    short: fn(Lang) -> &'static str,
    make: fn() -> Action,
) -> Command {
    Command {
        short: Some(short),
        ..press_named(id, name, make)
    }
}

/// 割り当てを替えさせない `Press`（キーの受け口が決まっているもの）。
const fn press_locked(
    id: &'static str,
    name: fn(Lang) -> &'static str,
    make: fn() -> Action,
) -> Command {
    Command {
        locked: true,
        ..press_named(id, name, make)
    }
}

/// 選んでいるレイヤーで言い方が変わる名前を持つ `Press`（レイヤーのメニューの項目も同じ関数で名前を付ける）。
const fn by_layer(
    id: &'static str,
    name: fn(&AppState, Option<LayerId>) -> &'static str,
    make: fn() -> Action,
) -> Command {
    Command {
        name: Name::ByLayer(name),
        ..press(id, make)
    }
}

/// 押しを読むビューが ID でキーを引く `Press`（`Action` を持たない）。
const fn read_by_view(id: &'static str, name: fn(Lang) -> &'static str) -> Command {
    Command {
        id,
        kind: Kind::Press,
        action: None,
        name: Name::Text(name),
        locked: false,
        run: None,
        short: None,
        unavailable: None,
        repeats: true,
    }
}

const fn hold(id: &'static str, name: fn(Lang) -> &'static str) -> Command {
    Command {
        kind: Kind::Hold,
        ..read_by_view(id, name)
    }
}

const fn gesture(id: &'static str, operation: Operation) -> Command {
    Command {
        kind: Kind::Gesture,
        name: Name::Mouse(operation),
        ..read_by_view(id, |_| "")
    }
}

const fn fixed(id: &'static str, key: Key, name: fn(Lang) -> &'static str) -> Command {
    Command {
        kind: Kind::Fixed(key),
        locked: true,
        ..read_by_view(id, name)
    }
}

fn mode(m: EditorMode) -> Action {
    Action::Mode(ModeAction::Set(m))
}

fn axis(view: AxisView) -> Action {
    Action::View3dNav(NavOp::Axis(view))
}

/// キーの繰り返しの押しでは実行しない操作にする（切り替え・消す・G/R/S を始める）。
const fn no_repeat(c: Command) -> Command {
    Command {
        repeats: false,
        ..c
    }
}

fn object(a: ObjectAction) -> Action {
    Action::Object(a)
}

fn pie(id: &str) -> Action {
    Action::Pie(PieAction::Open(id.to_owned()))
}

fn edit(e: SelEdit) -> Action {
    Action::Sel(SelAction::Edit(e))
}

/// 操作の全部（ID の重複・形・名前・表の行の指す先は試験が見る）。
pub static COMMANDS: &[Command] = &[
    // ファイル
    press_named(
        "file.new_project",
        |l| l.t("commands-new-project"),
        || Action::NewProjectDialog,
    ),
    press_named(
        "file.open",
        |l| l.t("commands-open"),
        || Action::OpenProjectDialog,
    ),
    press_named(
        "file.save",
        |l| l.t("commands-save"),
        || Action::SaveProject,
    ),
    press_named(
        "file.save_as",
        |l| l.t("commands-save-as"),
        || Action::SaveProjectAsDialog,
    ),
    // 書き出しのウィンドウ。既定のキーは無い（Ctrl+Shift+E は表示レイヤーの結合が使っている）。設定の画面で割り当てられる
    press_named(
        "file.export",
        |l| l.t("commands-export-textures"),
        || Action::Export(crate::export::ExportAction::OpenWindow),
    ),
    press_named("app.quit", |l| l.t("commands-quit"), || Action::Quit),
    press_named(
        "app.settings",
        |l| l.t("commands-settings"),
        || Action::Prefs(PrefsAction::Open),
    ),
    // メッシュマップをベイクするウィンドウ（既定のキーは無い。割り当てると、パイメニューからも呼べる。入口は、テクスチャセットの帯のボタンと、セットの右クリック）
    no_repeat(press_named(
        "bake.open",
        |l| l.t("commands-bake-mesh-maps"),
        || Action::Bake(crate::bake::BakeAction::OpenWindow),
    )),
    // 編集
    press_named("edit.undo", |l| l.t("commands-undo"), || Action::Undo),
    press_named("edit.redo", |l| l.t("commands-redo"), || Action::Redo),
    // レイヤー
    press_named(
        "layer.new",
        |l| l.t("commands-new-layer"),
        || Action::NewLayer,
    ),
    by_layer("layer.duplicate", duplicate_name, || {
        Action::M2(Edit::DuplicateSelected)
    }),
    press_named(
        "layer.group",
        |l| l.t("commands-group-layers"),
        || Action::M2(Edit::GroupSelected),
    ),
    press_named(
        "layer.ungroup",
        |l| l.t("commands-ungroup"),
        || Action::M2(Edit::UngroupSelected),
    ),
    by_layer("layer.merge_down", merge_down_name, || {
        Action::M2(Edit::MergeDown)
    }),
    press_named(
        "layer.merge_visible",
        |l| l.t("commands-merge-visible"),
        || Action::M2(Edit::MergeVisible),
    ),
    // 選択範囲
    press_named(
        "selection.all",
        |l| l.t("commands-select-all"),
        || edit(SelEdit::All),
    ),
    press_named(
        "selection.deselect",
        |l| l.t("commands-deselect"),
        || edit(SelEdit::Clear),
    ),
    press_named(
        "selection.invert",
        |l| l.t("commands-invert-selection"),
        || edit(SelEdit::Invert),
    ),
    press_named(
        "selection.erase",
        |l| l.t("commands-erase-selection"),
        || edit(SelEdit::Erase),
    ),
    press_named(
        "selection.to_new_layer",
        |l| l.t("commands-copy-to-a-new-layer"),
        || edit(SelEdit::ToNewLayer),
    ),
    press_named(
        "selection.quick_mask",
        |l| l.t("commands-quick-mask"),
        || Action::Sel(SelAction::Ui(SelUiOp::QuickMask(None))),
    ),
    // クリップボード（キーの受け口は `clipboard::keys`。割り当てを替えさせない）
    press_locked(
        "clip.copy_merged",
        |l| l.t("commands-copy-merged"),
        || Action::Clip(ClipAction::CopyMerged),
    ),
    press_locked(
        "clip.copy",
        |l| l.t("commands-copy"),
        || Action::Clip(ClipAction::Copy),
    ),
    press_locked(
        "clip.cut",
        |l| l.t("commands-cut"),
        || Action::Clip(ClipAction::Cut),
    ),
    press_locked(
        "clip.paste",
        |l| l.t("commands-paste"),
        || Action::Clip(ClipAction::Paste),
    ),
    // 表示
    press_named(
        "view.fit",
        |l| l.t("commands-fit-to-screen"),
        || Action::FitView,
    ),
    press_named(
        "view.zoom_in",
        |l| l.t("commands-zoom-in"),
        || Action::ZoomIn,
    ),
    press_named(
        "view.zoom_out",
        |l| l.t("commands-zoom-out"),
        || Action::ZoomOut,
    ),
    press_named(
        "view.rotate_left",
        |l| l.t("commands-rotate-view-left"),
        || Action::RotateLeft,
    ),
    press_named(
        "view.rotate_right",
        |l| l.t("commands-rotate-view-right"),
        || Action::RotateRight,
    ),
    press_named(
        "view.reset_rotation",
        |l| l.t("commands-reset-rotation"),
        || Action::ResetRotation,
    ),
    press_named(
        "view.flip",
        |l| l.t("commands-flip-view"),
        || Action::FlipView,
    ),
    press_named(
        "view.ruler_snap",
        |l| l.t("commands-snap-to-ruler"),
        || Action::Ruler(crate::rulers::RulerAction::ToggleSnapRuler),
    ),
    press_named(
        "view.special_ruler_snap",
        |l| l.t("commands-snap-to-special-ruler"),
        || Action::Ruler(crate::rulers::RulerAction::ToggleSnapSpecial),
    ),
    press_named(
        "view.switch_special_ruler",
        |l| l.t("commands-switch-the-snapping-special-ruler"),
        || Action::Ruler(crate::rulers::RulerAction::SwitchSpecial),
    ),
    // 色・ブラシ・塗りつぶし・パス
    press_named(
        "color.swap",
        |l| l.t("commands-swap-main-and-sub-colors"),
        || Action::SwapColors,
    ),
    press_named(
        "color.default",
        |l| l.t("commands-default-colors"),
        || Action::DefaultColors,
    ),
    press("color.pick_screen", || {
        Action::ScreenPick(crate::screen_pick::Mode::Visible)
    }),
    press("color.pick_screen_hidden", || {
        Action::ScreenPick(crate::screen_pick::Mode::HideWindow)
    }),
    press_named(
        "brush.smaller",
        |l| l.t("commands-smaller-brush"),
        || Action::BrushSmaller,
    ),
    press_named(
        "brush.larger",
        |l| l.t("commands-larger-brush"),
        || Action::BrushLarger,
    ),
    // 消すモード（既定のキーは無い。設定の画面で割り当てる）。切り替えは、押したままの繰り返しで出たり消えたりしない
    no_repeat(press_named(
        "brush.erase_mode",
        |l| l.t("commands-erase-mode"),
        || Action::ToggleEraseMode,
    )),
    // 切り替えは、押したままの繰り返しで出たり消えたりしない
    no_repeat(press_named(
        "fill.toggle_handles",
        |l| l.t("commands-projection-handles"),
        || Action::Fill(crate::fillfx::FillOp::ToggleHandles),
    )),
    press_named(
        "fill.delete_point",
        |l| l.t("commands-delete-gradient-point"),
        || Action::Fill(crate::fillfx::FillOp::DeletePoint),
    ),
    press_named(
        "path.delete_point",
        |l| l.t("commands-delete-path-point"),
        || Action::Path(PathAction::DeleteSelected),
    ),
    press_named(
        "path.finish",
        |l| l.t("commands-finish-path"),
        || Action::Path(PathAction::SelectPath(None)),
    ),
    // ツール（`tool.<ツールの id>`。ツールのキーはツールの表のとおり）
    press("tool.brush", || Action::SelectTool(Tool::Brush)),
    press("tool.eraser", || Action::SelectTool(Tool::Eraser)),
    press("tool.fill", || Action::SelectTool(Tool::Fill)),
    press("tool.gradient", || Action::SelectTool(Tool::Gradient)),
    press("tool.shape", || Action::SelectTool(Tool::Shape)),
    press("tool.ruler", || Action::SelectTool(Tool::Ruler)),
    press("tool.polygon_fill", || {
        Action::SelectTool(Tool::PolygonFill)
    }),
    press("tool.eyedropper", || Action::SelectTool(Tool::Eyedropper)),
    press("tool.select_rectangle", || {
        Action::SelectTool(Tool::SelectRect)
    }),
    press("tool.select_ellipse", || {
        Action::SelectTool(Tool::SelectEllipse)
    }),
    press("tool.lasso", || Action::SelectTool(Tool::Lasso)),
    press("tool.select_polygon", || Action::SelectTool(Tool::Polygon)),
    press("tool.magic_wand", || Action::SelectTool(Tool::Wand)),
    press("tool.id_select", || Action::SelectTool(Tool::IdSelect)),
    press("tool.select_pen", || Action::SelectTool(Tool::SelectPen)),
    press("tool.move", || Action::SelectTool(Tool::Move)),
    press("tool.liquify", || Action::SelectTool(Tool::Liquify)),
    press("tool.path", || Action::SelectTool(Tool::Path)),
    press("tool.text", || Action::SelectTool(Tool::Text)),
    // ビューが押しを読む `Press`
    Command {
        run: Some(|| Action::View3dNav(NavOp::FrameSelected)),
        short: Some(|l| l.t("commands-frame")),
        ..read_by_view("view3d.frame_selected", |l| {
            l.t("commands-frame-selected-set")
        })
    },
    read_by_view("transform.nudge_left", |l| l.t("commands-move-1-px")),
    read_by_view("transform.nudge_right", |l| l.t("commands-move-1-px-2")),
    read_by_view("transform.nudge_up", |l| l.t("commands-move-1-px-3")),
    read_by_view("transform.nudge_down", |l| l.t("commands-move-1-px-4")),
    read_by_view("transform.nudge_left_10", |l| l.t("commands-move-10-px")),
    read_by_view("transform.nudge_right_10", |l| l.t("commands-move-10-px-2")),
    read_by_view("transform.nudge_up_10", |l| l.t("commands-move-10-px-4")),
    read_by_view("transform.nudge_down_10", |l| l.t("commands-move-10-px-3")),
    // 押している間だけ効くキー（`view.rotate_hold` は既定では割り当てが無い）
    hold("view.rotate_hold", |l| l.t("commands-rotate")),
    hold("view.pan_hold", |l| l.t("commands-pan-ctrl-zoom")),
    hold("stencil.transform_hold", |l| l.t("commands-stencil")),
    hold("stencil.bypass_hold", |l| l.t("commands-bypass-stencil")),
    // 3D ビューで右ボタン（ペンのサイドボタン）を押している間の視点の移動
    hold("view3d.fly_forward", |l| {
        l.t("commands-move-forward-while-right-button")
    }),
    hold("view3d.fly_back", |l| {
        l.t("commands-move-back-while-right-button")
    }),
    hold("view3d.fly_left", |l| {
        l.t("commands-move-left-while-right-button")
    }),
    hold("view3d.fly_right", |l| {
        l.t("commands-move-right-while-right-button")
    }),
    hold("view3d.fly_down", |l| {
        l.t("commands-move-down-while-right-button")
    }),
    hold("view3d.fly_up", |l| {
        l.t("commands-move-up-while-right-button")
    }),
    // マウスの組み合わせ（`keymap::GESTURES`）
    gesture("view.orbit", Operation::Orbit),
    gesture("view.pan", Operation::Pan),
    gesture("view.zoom", Operation::Zoom),
    gesture("view.rotate", Operation::Rotate),
    // `color.eyedrop_temporary` は、描くツールで Alt + 左を押すと色を取る組み合わせだった ID。ショートカットの設定（段 6）で割り当てを戻せるように残す（今は右ボタンで色を取る）
    gesture("color.eyedrop_temporary", Operation::Pick),
    gesture("view.snap_orbit", Operation::SnapOrbit),
    gesture("brush.clone_source", Operation::CloneSource),
    gesture("selection.combine_add", Operation::SelectionAdd),
    gesture("selection.combine_subtract", Operation::SelectionSubtract),
    gesture("selection.combine_intersect", Operation::SelectionIntersect),
    gesture("stencil.move", Operation::MoveStencil),
    gesture("stencil.rotate", Operation::RotateStencil),
    gesture("stencil.scale", Operation::ScaleStencil),
    gesture("stencil.snap_rotation", Operation::SnapStencilRotation),
    // 画面の部品の決まった働き
    fixed("canvas.cancel", Key::Escape, |l| {
        l.t("commands-cancel-operation-deselect")
    }),
    fixed("canvas.confirm", Key::Enter, |l| {
        l.t("commands-confirm-transform-polygon-selection")
    }),
    fixed("canvas.remove_last_point", Key::Backspace, |l| {
        l.t("commands-delete-last-polygon-selection-point")
    }),
    fixed("view3d.cancel", Key::Escape, |l| {
        l.t("commands-cancel-operation")
    }),
    fixed("stencil.cancel", Key::Escape, |l| {
        l.t("commands-cancel-operation")
    }),
    // モード（ペイント・編集・ポーズ）とモードのパイ
    press_short(
        "mode.paint",
        |l| l.t("commands-paint-mode"),
        |l| l.t("commands-paint"),
        || mode(EditorMode::Paint),
    ),
    press_short(
        "mode.edit",
        |l| l.t("commands-edit-mode"),
        |l| l.t("commands-edit"),
        || mode(EditorMode::Edit),
    ),
    press_short(
        "mode.pose",
        |l| l.t("commands-pose-mode"),
        |l| l.t("commands-pose"),
        || mode(EditorMode::Pose),
    ),
    Command {
        repeats: false,
        ..press_named(
            "mode.pie",
            |l| l.t("commands-mode-pie-menu"),
            || pie("mode"),
        )
    },
    // 3D の視点（視点のパイ。既定のキーは無い）
    Command {
        repeats: false,
        ..press_named(
            "view3d.pie",
            |l| l.t("commands-view-pie-menu"),
            || pie("view"),
        )
    },
    press_short(
        "view3d.view_front",
        |l| l.t("commands-front-view"),
        |l| l.t("commands-front"),
        || axis(AxisView::Front),
    ),
    press_short(
        "view3d.view_back",
        |l| l.t("commands-back-view"),
        |l| l.t("commands-back"),
        || axis(AxisView::Back),
    ),
    press_short(
        "view3d.view_right",
        |l| l.t("commands-right-view"),
        |l| l.t("commands-right"),
        || axis(AxisView::Right),
    ),
    press_short(
        "view3d.view_left",
        |l| l.t("commands-left-view"),
        |l| l.t("commands-left"),
        || axis(AxisView::Left),
    ),
    press_short(
        "view3d.view_top",
        |l| l.t("commands-top-view"),
        |l| l.t("commands-top"),
        || axis(AxisView::Top),
    ),
    press_short(
        "view3d.view_bottom",
        |l| l.t("commands-bottom-view"),
        |l| l.t("commands-bottom"),
        || axis(AxisView::Bottom),
    ),
    // 編集・ポーズのモードの物（選んだ物・ボーン。キーは編集とポーズの段）
    Command {
        repeats: false,
        ..press_named(
            "object.grab",
            |l| l.t("commands-move-selected"),
            || object(ObjectAction::Transform(TransformKind::Grab)),
        )
    },
    Command {
        repeats: false,
        ..press_named(
            "object.rotate",
            |l| l.t("commands-rotate-selected"),
            || object(ObjectAction::Transform(TransformKind::Rotate)),
        )
    },
    Command {
        repeats: false,
        ..press_named(
            "object.scale",
            |l| l.t("commands-scale-selected"),
            || object(ObjectAction::Transform(TransformKind::Scale)),
        )
    },
    press_named(
        "object.reset_position",
        |l| l.t("commands-reset-position-of-selected"),
        || object(ObjectAction::Reset(TransformKind::Grab)),
    ),
    press_named(
        "object.reset_rotation",
        |l| l.t("commands-reset-rotation-of-selected"),
        || object(ObjectAction::Reset(TransformKind::Rotate)),
    ),
    press_named(
        "object.reset_scale",
        |l| l.t("commands-reset-scale-of-selected"),
        || object(ObjectAction::Reset(TransformKind::Scale)),
    ),
    no_repeat(press_named(
        "object.snap_toggle",
        |l| l.t("commands-toggle-snapping"),
        || object(ObjectAction::ToggleSnap),
    )),
    no_repeat(press_named(
        "object.hide",
        |l| l.t("commands-hide-selected-marker"),
        || object(ObjectAction::Hide),
    )),
    no_repeat(press_named(
        "object.reveal",
        |l| l.t("commands-reveal-hidden-markers"),
        || object(ObjectAction::Reveal),
    )),
    no_repeat(press_named(
        "object.delete",
        |l| l.t("commands-delete-selected"),
        || object(ObjectAction::Delete),
    )),
    // 透視と正投影の切り替え（視点のパイ・3D ビューの軸の印の真ん中。既定のキーは無い）
    no_repeat(press_short(
        "view3d.ortho",
        |l| l.t("commands-toggle-orthographic"),
        |l| l.t("commands-orthographic"),
        || Action::View3dNav(NavOp::ToggleOrthographic),
    )),
];

/// 操作の全部。
pub fn all() -> &'static [Command] {
    COMMANDS
}

/// ID の操作。
pub fn find(id: &str) -> Option<&'static Command> {
    static INDEX: OnceLock<HashMap<&'static str, &'static Command>> = OnceLock::new();
    INDEX
        .get_or_init(|| COMMANDS.iter().map(|c| (c.id, c)).collect())
        .get(id)
        .copied()
}

/// この `Action` をキーで実行する操作（ツールの選び・ズームなど。データを持つ `Action` は持った値まで同じものだけ）。
pub fn for_action(action: &Action) -> Option<&'static Command> {
    COMMANDS
        .iter()
        .find(|c| c.action.is_some_and(|make| make() == *action))
}

/// ツールを選ぶ操作が選ぶツール（ほかの操作は None。ツールのキーの動き方を選べる操作）。
pub fn selected_tool(command: &str) -> Option<Tool> {
    match find(command)?.action?() {
        Action::SelectTool(tool) => Some(tool),
        _ => None,
    }
}

/// ツールを選ぶ操作の ID（`tool.<ツールの id>`）。
pub fn tool_command(tool: Tool) -> &'static str {
    match tool {
        Tool::Brush => "tool.brush",
        Tool::Eraser => "tool.eraser",
        Tool::Fill => "tool.fill",
        Tool::Gradient => "tool.gradient",
        Tool::Shape => "tool.shape",
        Tool::Ruler => "tool.ruler",
        Tool::PolygonFill => "tool.polygon_fill",
        Tool::Eyedropper => "tool.eyedropper",
        Tool::SelectRect => "tool.select_rectangle",
        Tool::SelectEllipse => "tool.select_ellipse",
        Tool::Lasso => "tool.lasso",
        Tool::Polygon => "tool.select_polygon",
        Tool::Wand => "tool.magic_wand",
        Tool::IdSelect => "tool.id_select",
        Tool::SelectPen => "tool.select_pen",
        Tool::Move => "tool.move",
        Tool::Liquify => "tool.liquify",
        Tool::Path => "tool.path",
        Tool::Text => "tool.text",
    }
}

/// ID の形（英小文字・数字の分野、`.`、英小文字・数字・`_` の名前を 1 つ以上）。
pub fn is_valid_id(id: &str) -> bool {
    let mut parts = id.split('.');
    let Some(domain) = parts.next() else {
        return false;
    };
    let word = |part: &str, underscore: bool| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || (underscore && b == b'_'))
    };
    let mut names = 0;
    for part in parts {
        if !word(part, true) {
            return false;
        }
        names += 1;
    }
    word(domain, false) && names >= 1
}

impl Operation {
    /// この組み合わせの操作の ID。
    pub fn command(self) -> &'static str {
        COMMANDS
            .iter()
            .find(|c| matches!(c.name, Name::Mouse(operation) if operation == self))
            .map(|c| c.id)
            .expect("すべての組み合わせに操作がある（試験が見る）")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{self, GESTURES};
    use std::collections::BTreeSet;

    #[test]
    fn ids_are_unique_and_well_formed() {
        let mut seen = BTreeSet::new();
        for c in COMMANDS {
            assert!(is_valid_id(c.id), "ID の形が違います: {}", c.id);
            assert!(seen.insert(c.id), "ID が重なっています: {}", c.id);
        }
        assert_eq!(seen.len(), COMMANDS.len());
    }

    #[test]
    fn the_id_validator_accepts_the_documented_shape_only() {
        for ok in [
            "file.save",
            "tool.select_pen",
            "view3d.frame_selected",
            "transform.nudge_left_10",
            "a.b.c",
        ] {
            assert!(is_valid_id(ok), "{ok}");
        }
        for bad in [
            "",
            "file",
            ".save",
            "file.",
            "File.save",
            "file.Save",
            "file.save-as",
            "tool.select-pen",
            "my_domain.save",
            "file..save",
            "file.save as",
            "ファイル.保存",
        ] {
            assert!(!is_valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn every_press_has_an_action_except_the_keys_a_view_reads_by_id() {
        let read_by_view: BTreeSet<&str> = COMMANDS
            .iter()
            .filter(|c| c.kind == Kind::Press && c.action.is_none())
            .map(|c| c.id)
            .collect();
        let expected: BTreeSet<&str> = [
            "view3d.frame_selected",
            "transform.nudge_left",
            "transform.nudge_right",
            "transform.nudge_up",
            "transform.nudge_down",
            "transform.nudge_left_10",
            "transform.nudge_right_10",
            "transform.nudge_up_10",
            "transform.nudge_down_10",
        ]
        .into_iter()
        .collect();
        assert_eq!(read_by_view, expected);
        // パイから実行できる口を持つのは、3D の . で収める操作だけ。この版で使えない操作は無い（正投影も実行できる）
        let run: Vec<&str> = COMMANDS
            .iter()
            .filter(|c| c.run.is_some())
            .map(|c| c.id)
            .collect();
        assert_eq!(run, ["view3d.frame_selected"]);
        let unavailable: Vec<&str> = COMMANDS
            .iter()
            .filter(|c| c.unavailable.is_some())
            .map(|c| c.id)
            .collect();
        assert!(unavailable.is_empty(), "{unavailable:?}");
        for c in COMMANDS {
            assert_eq!(
                c.runnable().is_some(),
                c.action.is_some() || c.id == "view3d.frame_selected",
                "{}",
                c.id
            );
        }
        for c in COMMANDS.iter().filter(|c| c.kind != Kind::Press) {
            assert!(
                c.action.is_none(),
                "{}: Press 以外は Action を持たない",
                c.id
            );
        }
    }

    #[test]
    fn no_two_commands_run_the_same_action() {
        for (i, a) in COMMANDS.iter().enumerate() {
            let Some(make) = a.action else { continue };
            for b in &COMMANDS[i + 1..] {
                if let Some(other) = b.action {
                    assert_ne!(make(), other(), "{} と {} が同じ Action", a.id, b.id);
                }
            }
        }
    }

    #[test]
    fn every_command_has_a_japanese_and_an_english_name_and_english_has_no_japanese() {
        let mut app = AppState::new(32, 32);
        for lang in Lang::ALL {
            app.lang = lang;
            app.session.selected_layer = None;
            for c in COMMANDS {
                let label = c
                    .label(&app)
                    .unwrap_or_else(|| panic!("名前がありません: {}", c.id));
                assert!(!label.trim().is_empty(), "{}", c.id);
                if lang == Lang::En {
                    assert!(
                        !label
                            .chars()
                            .any(|ch| ('\u{3000}'..='\u{9fff}').contains(&ch)),
                        "{}: {label}",
                        c.id
                    );
                }
            }
        }
    }

    #[test]
    fn tool_commands_follow_the_tool_table() {
        use crate::tools::TOOLS;
        for def in &TOOLS {
            let id = format!("tool.{}", def.id.replace('-', "_"));
            assert_eq!(tool_command(def.tool), id, "{:?}", def.tool);
            let command = find(&id).unwrap_or_else(|| panic!("ツールの操作がありません: {id}"));
            assert_eq!(command.kind, Kind::Press);
            assert_eq!(
                (command.action.expect("Action"))(),
                Action::SelectTool(def.tool),
                "{id}"
            );
        }
        let tools = COMMANDS
            .iter()
            .filter(|c| c.id.starts_with("tool."))
            .count();
        assert_eq!(tools, TOOLS.len());
    }

    #[test]
    fn every_mouse_combination_has_a_gesture_command() {
        for operation in Operation::ALL {
            let id = operation.command();
            assert_eq!(find(id).map(|c| c.kind), Some(Kind::Gesture), "{id}");
        }
        assert_eq!(
            COMMANDS.iter().filter(|c| c.kind == Kind::Gesture).count(),
            Operation::ALL.len()
        );
        for g in &GESTURES {
            assert!(Operation::ALL.contains(&g.operation));
            if let Some(held) = g.held {
                assert_eq!(find(held).map(|c| c.kind), Some(Kind::Hold), "{held}");
            }
        }
    }

    #[test]
    fn every_key_row_points_at_a_command_of_the_matching_kind() {
        for row in keymap::bindings() {
            let command = find(row.command)
                .unwrap_or_else(|| panic!("表の行の操作が一覧にありません: {}", row.command));
            assert!(
                matches!(command.kind, Kind::Press | Kind::Hold),
                "{}: 表の行は Press か Hold",
                row.command
            );
        }
        // 押している間のキーは表に行がある（既定で割り当てを外した `view.rotate_hold` を除く）。ほかの種類は行を持たない
        for c in COMMANDS.iter().filter(|c| c.kind != Kind::Press) {
            let has_row = keymap::bindings().iter().any(|b| b.command == c.id);
            assert_eq!(
                has_row,
                c.kind == Kind::Hold && c.id != "view.rotate_hold",
                "{}",
                c.id
            );
        }
        // 既定のキーが無い Press: 液化（ツールの帯から）・モードを 1 つずつ選ぶ操作（ドロップダウン・パイ・メニューから）・視点のパイと
        // その中身（設定で割り当てる）・メッシュマップをベイクするウィンドウ（帯のボタンとセットの右クリックから。設定で割り当てる）・
        // 書き出しのウィンドウ（Ctrl+Shift+E は表示レイヤーの結合が使っているので、設定で割り当てる）
        const NO_DEFAULT_KEY: [&str; 15] = [
            "bake.open",
            "brush.erase_mode",
            "file.export",
            "tool.liquify",
            "mode.paint",
            "mode.edit",
            "mode.pose",
            "view3d.pie",
            "view3d.view_front",
            "view3d.view_back",
            "view3d.view_right",
            "view3d.view_left",
            "view3d.view_top",
            "view3d.view_bottom",
            "view3d.ortho",
        ];
        for c in COMMANDS {
            if c.kind == Kind::Press && !NO_DEFAULT_KEY.contains(&c.id) {
                assert!(
                    keymap::bindings().iter().any(|b| b.command == c.id),
                    "{}: 割り当てがない",
                    c.id
                );
            }
        }
    }

    #[test]
    fn the_action_lookup_finds_the_command_that_runs_it() {
        for c in COMMANDS {
            if let Some(make) = c.action {
                assert_eq!(for_action(&make()).map(|f| f.id), Some(c.id));
            }
        }
        assert!(for_action(&Action::About).is_none());
    }

    #[test]
    fn only_the_fixed_keys_and_the_clipboard_cannot_be_rebound() {
        let locked: BTreeSet<&str> = COMMANDS.iter().filter(|c| c.locked).map(|c| c.id).collect();
        let expected: BTreeSet<&str> = [
            "clip.copy_merged",
            "clip.copy",
            "clip.cut",
            "clip.paste",
            "canvas.cancel",
            "canvas.confirm",
            "canvas.remove_last_point",
            "view3d.cancel",
            "stencil.cancel",
        ]
        .into_iter()
        .collect();
        assert_eq!(locked, expected);
        for c in COMMANDS {
            assert_eq!(c.rebindable(), !c.locked, "{}", c.id);
            if matches!(c.kind, Kind::Fixed(_)) {
                assert!(c.locked, "{}: 画面の部品の働きは替えさせない", c.id);
            }
        }
    }

    #[test]
    fn the_command_kinds_add_up() {
        let count = |f: fn(&Command) -> bool| COMMANDS.iter().filter(|c| f(c)).count();
        assert_eq!(count(|c| c.kind == Kind::Press && c.action.is_some()), 88);
        assert_eq!(count(|c| c.kind == Kind::Press && c.action.is_none()), 9);
        assert_eq!(count(|c| c.kind == Kind::Hold), 10);
        assert_eq!(count(|c| c.kind == Kind::Gesture), 14);
        assert_eq!(count(|c| matches!(c.kind, Kind::Fixed(_))), 5);
        assert_eq!(COMMANDS.len(), 126);
    }
}
