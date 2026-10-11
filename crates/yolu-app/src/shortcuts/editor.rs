//! ショートカットの設定（設定のウィンドウの「ショートカット」の区分。`prefs::window` が左の区分・上の探す欄・右の欄を持ち、この中身を出す）。
//! 左（設定のウィンドウの区分の下）にショートカットの区分（どこでも・視点／ペイント／編集／ポーズ／途中の操作／パイメニュー）、右に表（操作の名前・キー・マウス）。
//! 表の上に名前で探す欄と「キーで探す」。
//!
//! - キーの欄を押すと、次に押したキー（修飾つき）をその欄の割り当てにする。Esc でやめ、Backspace で外す。修飾だけの押しでは決めない。マウスの
//!   欄は、次にその欄の上で押したボタン（修飾つき。ボタンと修飾が変わる。回す間の刻みの行は修飾だけ）。押している間は
//!   `PopupKind::KeyCapture` を開いて、キーの表・キャンバス・3D ビューのキーと Esc を止める（パイと G/R/S と同じ受け皿）。決めたキーは、
//!   離すまで繰り返しの押しを表へ渡さない（`swallow_held`）。
//! - ぶつかり（同じ範囲・同じ場面で同じ入力が別の操作）は行を赤くして相手の名前をツールチップに出し、残っている間はファイルに書かない。
//! - `locked` の操作（クリップボード）と画面の部品の決まったキー（Esc・Enter・Backspace）は変えられない。
//! - 変えた行の右端の印で、その行を既定に戻す。下の帯で、全部を既定に戻す・書き出し・読み込み。

use std::collections::HashMap;

use egui::{
    pos2, vec2, Event, Id, Key, Modifiers, PointerButton, Rect, Sense, Ui, UiBuilder, WidgetInfo,
    WidgetType,
};

use crate::commands::{self, Kind};
use crate::keyconfig::{self, Combo, GroupKey};
use crate::keymap::{self, Scope, Trigger, When, GESTURES};
use crate::lang::Lang;
use crate::pie::{PieItem, PieName};
use crate::state::{Action, AppState, DialogRequest, OpenPopup, PopupKind};
use crate::toolkeys::ToolKeyMode;
use crate::ui::menu::{Entry, PopupState};
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};

const ROW: f32 = 31.0;
const FOOTER: f32 = 52.0;
const TOP: f32 = 52.0;
const HEAD: f32 = 26.0;

/// 左の区分。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Everywhere,
    Paint,
    Edit,
    Pose,
    During,
    Pies,
}

impl Section {
    pub const ALL: [Section; 6] = [
        Section::Everywhere,
        Section::Paint,
        Section::Edit,
        Section::Pose,
        Section::During,
        Section::Pies,
    ];

    pub fn label(self, lang: Lang) -> &'static str {
        match self {
            Section::Everywhere => lang.pick("どこでも・視点", "Everywhere & View"),
            Section::Paint => lang.pick("ペイント", "Paint"),
            Section::Edit => lang.pick("編集", "Edit"),
            Section::Pose => lang.pick("ポーズ", "Pose"),
            Section::During => lang.pick("途中の操作", "During an Operation"),
            Section::Pies => lang.pick("パイメニュー", "Pie Menus"),
        }
    }

    /// キーの範囲の区分。
    pub fn of_scope(scope: Scope) -> Section {
        match scope {
            Scope::Everywhere => Section::Everywhere,
            Scope::Paint => Section::Paint,
            Scope::Edit => Section::Edit,
            Scope::Pose => Section::Pose,
            Scope::During => Section::During,
        }
    }
}

/// 割り当てを待っている欄。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// キーの欄（`chip` は置き換える入力の番号。None は加える）。
    Key {
        group: GroupKey,
        chip: Option<usize>,
    },
    /// マウスの欄（既定の表の番号）。
    Mouse(u8),
    /// 「キーで探す」。
    Search,
}

/// 割り当てを待っている間。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capture {
    pub target: Target,
    /// 始めたフレーム（そのフレームの押しは数えない）。
    pub opened: u64,
}

/// 表の 1 行。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Line {
    /// キーの行（操作と範囲）。`locked` は変えられない。
    Key { group: GroupKey, locked: bool },
    /// 画面の部品の決まったキー（変えられない）。
    Fixed { command: &'static str, key: Key },
    /// マウスの組み合わせ（既定の表の番号）。
    Mouse(u8),
    /// 受け口がボタンを決めている操作（変えられない。編集のモードの「選ぶ」の左クリック）。
    FixedMouse,
}

/// パイの項目を選ぶ所（8 か所のどこか・種類・名前で探す文字）。
#[derive(Clone, Debug, PartialEq)]
pub struct Picker {
    pub slot: usize,
    pub kind: PickKind,
    pub query: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickKind {
    Command,
    Pie,
    Ops,
    Recorded,
}

impl PickKind {
    const ALL: [PickKind; 4] = [
        PickKind::Command,
        PickKind::Pie,
        PickKind::Ops,
        PickKind::Recorded,
    ];

    fn label(self, lang: Lang) -> &'static str {
        match self {
            PickKind::Command => lang.pick("操作", "Command"),
            PickKind::Pie => lang.pick("別のパイ", "Another Pie"),
            PickKind::Ops => lang.pick("yolu-ops の命令", "yolu-ops Command"),
            PickKind::Recorded => lang.pick("記録したアクション", "Recorded Action"),
        }
    }
}

/// ウィンドウの状態（アプリの状態。保存しない）。
#[derive(Default)]
pub struct ShortcutWindow {
    scroll: f32,
    pub section: Section,
    pub search: String,
    /// 「キーで探す」で押したキー（その入力の行だけを出す。全部の区分から）。
    pub key_filter: Option<Trigger>,
    pub capture: Option<Capture>,
    /// 割り当てに使ったキー（離すまで、繰り返しの押しを表へ渡さない）。
    pub swallow: Option<Key>,
    /// マウスの欄に入れた押しの、離しのクリックを数えない（ボタンを離すまで）。
    ignore_click: bool,
    /// このフレームに描いた欄（試験が押す所を知る・マウスの欄の押しの当たり）。
    pub chips: Vec<(Target, Rect)>,
    /// このフレームに描いた、ツールのキーの動き方を選ぶ箱（ツールを選ぶ操作の ID と矩形。試験が押す所を知る）。
    pub mode_boxes: Vec<(&'static str, Rect)>,
    /// 名前で探す欄・パイの 8 か所の欄・項目を探す欄の矩形（試験が押す所を知る）。
    pub search_rect: Option<Rect>,
    pub slot_rects: Vec<Rect>,
    pub pick_rect: Option<Rect>,
    pick_scroll: f32,
    /// パイメニューの面で選んでいるパイ（並びの番号）。
    pub pie: usize,
    pub picker: Option<Picker>,
    names: Option<(Lang, HashMap<&'static str, String>)>,
}

impl ShortcutWindow {
    /// このフレームに描いた欄の矩形。
    pub fn chip_rect(&self, target: Target) -> Option<Rect> {
        self.chips
            .iter()
            .find(|(t, _)| *t == target)
            .map(|(_, r)| *r)
    }

    /// 割り当てを待っているか。
    pub fn capturing(&self) -> bool {
        self.capture.is_some()
    }
}

// ───────── 名前・行 ─────────

/// 操作の名前（メニューと同じ名前。利用者のパイを開く操作は、そのパイの名前）。
pub fn command_name(app: &AppState, command: &str) -> String {
    if let Some(id) = keymap::pie_of(command) {
        let name = app
            .pie
            .menu(id)
            .map_or_else(|| id.to_owned(), |m| m.name(app.lang));
        return app.lang.pick(
            format!("パイメニュー{}", app.lang.quote(&name)),
            format!("Pie Menu {}", app.lang.quote(&name)),
        );
    }
    commands::find(command)
        .and_then(|c| c.label(app))
        .unwrap_or_else(|| command.to_owned())
}

fn cached_name(window: &mut ShortcutWindow, app: &AppState, command: &'static str) -> String {
    if window.names.as_ref().is_none_or(|(l, _)| *l != app.lang) {
        window.names = Some((app.lang, HashMap::new()));
    }
    let (_, names) = window.names.as_mut().expect("今作った");
    names
        .entry(command)
        .or_insert_with(|| command_name(app, command))
        .clone()
}

/// 編集のモードの「選ぶ」（印を左クリック）。
fn lang_select(lang: Lang) -> &'static str {
    lang.pick("選ぶ", "Select")
}

/// 場面の名前（場面の行だけ）。
pub(crate) fn when_label(lang: Lang, when: When) -> Option<String> {
    Some(match when {
        When::Always => return None,
        When::HasSelection => lang.pick("選択範囲があるとき", "With a selection").into(),
        When::Tool(tool) => tool.name_in(lang).into(),
        When::Windows => "Windows".into(),
        When::PointSelected => lang
            .pick("点を選んでいるとき", "With a point selected")
            .into(),
    })
}

/// マウスの組み合わせの行の名前（操作と、どこで）。
pub(crate) fn mouse_name(lang: Lang, index: u8) -> String {
    let g = &GESTURES[index as usize];
    let place = match g.scope {
        "canvas" => lang.pick("2D", "2D"),
        "view3d" => lang.pick("3D", "3D"),
        "stencil" => lang.pick("ステンシル", "Stencil"),
        _ => lang.pick("選択範囲", "Selection"),
    };
    let (open, close) = lang.pick(("（", "）"), (" (", ")"));
    format!("{}{open}{place}{close}", g.operation.label(lang))
}

/// 修飾なしの 1 つのキーだけを受けるまとまり（押している間だけ効くキーと、途中の操作のキー）。
pub fn single_key(group: GroupKey) -> bool {
    group.1 == Scope::During || commands::find(group.0).is_some_and(|c| c.kind == Kind::Hold)
}

/// 既定の行が無い操作の置き場の範囲（ツール・色・ブラシ・ステンシル・パス・塗りつぶしはペイント、物は編集、ほかはどこでも）。
fn home_scope(command: &str) -> Scope {
    let paint = [
        "tool.",
        "color.",
        "brush.",
        "stencil.",
        "path.",
        "fill.",
        "transform.",
    ];
    if paint.iter().any(|p| command.starts_with(p)) {
        Scope::Paint
    } else if command.starts_with("object.") {
        Scope::Edit
    } else {
        Scope::Everywhere
    }
}

/// 表の全部のまとまり（既定の行・変えて加えた行・まだ入力の無い操作。利用者のパイを開く操作はパイメニューの面）。
pub fn all_groups(app: &AppState) -> Vec<GroupKey> {
    let mut out = keyconfig::default_groups();
    for key in app.keys.changed_groups() {
        if !out.contains(&key) {
            out.push(key);
        }
    }
    // 利用者のパイを開く操作（どこでも。キーで探す・名前で探すにも出る）
    for m in &app.pie.menus {
        if let Some(command) = m.command.filter(|c| keymap::pie_of(c).is_some()) {
            let key = (command, Scope::Everywhere);
            if !out.contains(&key) {
                out.push(key);
            }
        }
    }
    for c in commands::all() {
        let listed = matches!(c.kind, Kind::Press | Kind::Hold)
            && c.unavailable.is_none()
            && (c.action.is_some() || c.run.is_some() || c.kind == Kind::Hold);
        if listed && !out.iter().any(|(id, _)| *id == c.id) {
            out.push((c.id, home_scope(c.id)));
        }
    }
    out
}

/// 区分の行（名前で探す文字・キーで探すキーで絞る）。キーで探している間は、全部の区分から。
pub fn lines(window: &mut ShortcutWindow, app: &AppState, section: Section) -> Vec<Line> {
    let query = window.search.trim().to_lowercase();
    let filter = window.key_filter;
    let mut out = Vec::new();
    for group in all_groups(app) {
        if filter.is_none() && Section::of_scope(group.1) != section {
            continue;
        }
        if let Some(f) = filter {
            if !app
                .keys
                .triggers(group)
                .iter()
                .any(|t| keymap::same_input(t, &f))
            {
                continue;
            }
        }
        let name = cached_name(window, app, group.0);
        if !query.is_empty() && !name.to_lowercase().contains(&query) && !group.0.contains(&query) {
            continue;
        }
        let locked = commands::find(group.0).is_some_and(|c| c.locked);
        out.push(Line::Key { group, locked });
    }
    if filter.is_none() && section == Section::Everywhere {
        for c in commands::all() {
            if let Kind::Fixed(key) = c.kind {
                let name = c.static_label(app.lang).unwrap_or(c.id);
                if query.is_empty() || name.to_lowercase().contains(&query) {
                    out.push(Line::Fixed { command: c.id, key });
                }
            }
        }
    }
    if filter.is_none() && section == Section::Edit {
        let name = lang_select(app.lang);
        if query.is_empty() || name.to_lowercase().contains(&query) {
            out.push(Line::FixedMouse);
        }
    }
    if filter.is_none() {
        for g in GESTURES.iter() {
            if Section::of_scope(g.mode) != section {
                continue;
            }
            let name = mouse_name(app.lang, g.index);
            if query.is_empty() || name.to_lowercase().contains(&query) {
                out.push(Line::Mouse(g.index));
            }
        }
    }
    out
}

// ───────── 文字 ─────────

/// 入力の文字（「Ctrl+S」。Mac は Cmd）。
pub fn trigger_label(t: &Trigger) -> String {
    super::key_label(&keymap::KeyBinding {
        trigger: *t,
        command: "",
        scope: Scope::Everywhere,
        when: When::Always,
    })
}

/// マウスの組み合わせの文字（押しながらのキー・修飾・ボタン、動かさずに離すかドラッグか）。
pub fn combo_label(lang: Lang, index: u8, combo: Combo) -> String {
    let g = &GESTURES[index as usize];
    let held = g.held.and_then(keymap::hold_key);
    super::gestures::key_label(
        &super::gestures::Binding {
            scope: g.scope,
            held,
            modifiers: Modifiers {
                alt: combo.alt,
                shift: combo.shift,
                ctrl: combo.ctrl,
                command: combo.ctrl,
                ..Modifiers::NONE
            },
            button: combo.button,
            operation: g.operation,
            click: g.click,
        },
        lang,
    )
}

/// 修飾だけ押している間の欄の文字（「Ctrl+…」）。
fn held_modifiers(m: &Modifiers) -> String {
    let mut s = String::new();
    if m.command || m.ctrl || m.mac_cmd {
        s.push_str(if cfg!(target_os = "macos") && (m.command || m.mac_cmd) {
            "Cmd+"
        } else {
            "Ctrl+"
        });
    }
    if m.alt {
        s.push_str("Alt+");
    }
    if m.shift {
        s.push_str("Shift+");
    }
    s.push('…');
    s
}

// ───────── 割り当てを待つ ─────────

/// 割り当てを待ち始める（キーの表・キャンバス・3D ビューのキーを止める受け皿を開く）。
fn begin_capture(ctx: &egui::Context, app: &mut AppState, target: Target, at: Rect) {
    app.shortcuts.capture = Some(Capture {
        target,
        opened: ctx.cumulative_frame_nr(),
    });
    app.popup = Some(OpenPopup {
        kind: PopupKind::KeyCapture,
        state: PopupState::new(ctx, at),
    });
}

/// 待つのをやめる（受け皿を閉じる）。
pub fn end_capture(app: &mut AppState) {
    app.shortcuts.capture = None;
    if matches!(
        app.popup.as_ref().map(|p| p.kind),
        Some(PopupKind::KeyCapture)
    ) {
        app.popup = None;
    }
}

/// 割り当てに使ったキーを、離すまで取り除く（キーの表へ繰り返しの押しを渡さない）。キーの処理の初めに呼ぶ。
pub fn swallow_held(ctx: &egui::Context, app: &mut AppState) {
    let Some(key) = app.shortcuts.swallow else {
        return;
    };
    let mut released = false;
    ctx.input_mut(|i| {
        i.events.retain(|e| match e {
            Event::Key {
                key: k, pressed, ..
            } if *k == key => {
                if !*pressed {
                    released = true;
                }
                false
            }
            _ => true,
        });
        if !i.key_down(key) {
            released = true;
        }
    });
    if released {
        app.shortcuts.swallow = None;
    }
}

/// 待っている間の入力を受ける（キーかボタンで決める・Esc でやめる・Backspace で外す）。待つのが終わったら true。
fn take_capture(ctx: &egui::Context, app: &mut AppState) -> bool {
    let Some(capture) = app.shortcuts.capture else {
        return false;
    };
    // 受け皿が外から閉じられた（メニューバーを開いたなど）: やめる
    if !matches!(
        app.popup.as_ref().map(|p| p.kind),
        Some(PopupKind::KeyCapture)
    ) {
        app.shortcuts.capture = None;
        return true;
    }
    let frame = ctx.cumulative_frame_nr();
    let (keys, presses) = ctx.input(|i| {
        let keys: Vec<(Key, Modifiers)> = i
            .events
            .iter()
            .filter_map(|e| match e {
                Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } => Some((*key, *modifiers)),
                _ => None,
            })
            .collect();
        let presses: Vec<(PointerButton, egui::Pos2, Modifiers)> = i
            .events
            .iter()
            .filter_map(|e| match e {
                Event::PointerButton {
                    button,
                    pressed: true,
                    pos,
                    modifiers,
                } => Some((*button, *pos, *modifiers)),
                _ => None,
            })
            .collect();
        (keys, presses)
    });
    // 待っている間のキーは、ほかの部品（入力欄の Tab・Enter）に渡さない
    ctx.input_mut(|i| i.events.retain(|e| !matches!(e, Event::Key { .. })));
    if let Some(focused) = ctx.memory(|m| m.focused()) {
        ctx.memory_mut(|m| m.surrender_focus(focused));
    }
    if let Some((key, m)) = keys.first().copied() {
        let plain = !m.any();
        if key == Key::Escape && plain {
            // この Esc は待つのをやめるのに使った（後で描くウィンドウが閉じない）
            crate::ui::window::note_escape_taken(ctx);
            end_capture(app);
            app.shortcuts.swallow = Some(key);
            return true;
        }
        if key == Key::Backspace && plain {
            match capture.target {
                Target::Key { group, chip } => {
                    if let Some(i) = chip {
                        let mut triggers = app.keys.triggers(group);
                        if i < triggers.len() {
                            triggers.remove(i);
                            app.keys.set(group, triggers);
                            app.keys_changed();
                        }
                    }
                }
                Target::Mouse(index) => {
                    app.keys.set_combo(index, None);
                    app.keys_changed();
                }
                Target::Search => app.shortcuts.key_filter = None,
            }
            end_capture(app);
            app.shortcuts.swallow = Some(key);
            return true;
        }
        match capture.target {
            Target::Key { group, chip } => {
                let single = single_key(group);
                // 押している間だけ効くキーと途中の操作のキーは、修飾なしの 1 つのキー（読む側は最初のキーを修飾なしで見る）
                let trigger = if single {
                    Trigger::Key {
                        modifiers: Modifiers::NONE,
                        key,
                    }
                } else {
                    keyconfig::trigger_of(key, &m)
                };
                let mut triggers = app.keys.triggers(group);
                match chip {
                    _ if single => triggers = vec![trigger],
                    Some(i) if i < triggers.len() => triggers[i] = trigger,
                    _ => {
                        if !triggers.iter().any(|t| keymap::same_input(t, &trigger)) {
                            triggers.push(trigger);
                        }
                    }
                }
                app.keys.set(group, triggers);
                app.keys_changed();
                end_capture(app);
                app.shortcuts.swallow = Some(key);
                return true;
            }
            Target::Search => {
                app.shortcuts.key_filter = Some(keyconfig::trigger_of(key, &m));
                app.shortcuts.scroll = 0.0;
                end_capture(app);
                app.shortcuts.swallow = Some(key);
                return true;
            }
            // マウスの欄はボタンで決める（キーは数えない）
            Target::Mouse(_) => {}
        }
    }
    for (button, pos, m) in presses {
        if frame == capture.opened {
            continue;
        }
        let chip = app.shortcuts.chip_rect(capture.target);
        match capture.target {
            Target::Mouse(index) if chip.is_some_and(|r| r.contains(pos)) => {
                // 始めたあとに効く修飾の行（ステンシルの回転の刻み）は、修飾だけを変える（ボタンは、回す操作のボタン。修飾の無い押しは
                // 数えず、待ち続ける）
                let Some(combo) = Combo::pressed(button, &m).fit(&GESTURES[index as usize]) else {
                    continue;
                };
                app.keys.set_combo(index, Some(combo));
                app.keys_changed();
                app.shortcuts.ignore_click = true;
                end_capture(app);
                return true;
            }
            _ => {
                // ほかの所を押した: やめる（押した所の働きは、そのまま）
                end_capture(app);
                return true;
            }
        }
    }
    false
}

// ───────── 描く ─────────

/// 欄 1 つ（入力の文字の札）。押したら true。
#[allow(clippy::too_many_arguments)]
fn chip(
    ui: &mut Ui,
    app: &mut AppState,
    at: egui::Pos2,
    target: Target,
    text: &str,
    capturing: bool,
    conflict: bool,
    enabled: bool,
    tooltip: Option<&str>,
    name: &str,
) -> (bool, f32) {
    let width = if text.is_empty() {
        44.0
    } else {
        w::text_width(ui.painter(), text, t::LABEL) + 18.0
    }
    .max(28.0);
    let r = Rect::from_min_size(at, vec2(width, 20.0));
    app.shortcuts.chips.push((target, r));
    let id = Id::new(("yolu.shortcuts.chip", format!("{target:?}")));
    let response = ui.interact(
        r,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let p = ui.painter();
    let hover = enabled && response.hovered();
    w::rounded(
        p,
        r,
        if hover {
            t::CONTROL_HOVER
        } else {
            t::CONTROL_BG
        },
        3.0,
    );
    let edge = if capturing {
        t::ACCENT
    } else if conflict {
        t::ERROR
    } else {
        t::SEPARATOR
    };
    w::outline(
        p,
        r,
        edge,
        if capturing || conflict { 1.5 } else { 1.0 },
        3.0,
    );
    w::text(
        p,
        r,
        text,
        t::LABEL.with_color(if enabled { t::TEXT } else { t::TEXT_DISABLED }),
        Align::Center,
    );
    if conflict {
        w::icon(
            p,
            Rect::from_min_size(pos2(r.right() + 3.0, r.top()), vec2(16.0, r.height())),
            "warning",
            t::ERROR,
            14.0,
        );
    }
    let label = if text.is_empty() {
        name.to_owned()
    } else {
        format!("{name}: {text}")
    };
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, &label));
    let clicked = response.clicked();
    if let Some(tip) = tooltip {
        response.on_hover_text(tip);
    }
    (clicked, width + if conflict { 20.0 } else { 0.0 })
}

/// 設定のウィンドウの「ショートカット」の区分を描く前に、毎フレーム呼ぶ（待っている入力を受ける）。割り当てを決めた・やめたフレームなら true
/// （その Esc・Enter でウィンドウを閉じない・入力欄へフォーカスを移さない）。ほかの区分を見ているときは、待つのを終えるだけ。
pub(crate) fn begin_frame(ctx: &egui::Context, app: &mut AppState, shown: bool) -> bool {
    if !shown {
        if app.shortcuts.capturing() {
            end_capture(app);
        }
        return false;
    }
    let finished = take_capture(ctx, app);
    if app.shortcuts.swallow.is_some() || finished {
        if let Some(focused) = ctx.memory(|m| m.focused()) {
            ctx.memory_mut(|m| m.surrender_focus(focused));
        }
    }
    if app.shortcuts.ignore_click && !ctx.input(|i| i.pointer.any_down()) {
        // 離しのフレームまでは数えない
        if !ctx.input(|i| i.pointer.any_released()) {
            app.shortcuts.ignore_click = false;
        }
    }
    finished
}

/// 「ショートカット」の区分を離れる・設定のウィンドウを閉じる: 待っているキーをやめ、選びかけのパイの項目を捨てる。
pub(crate) fn leave(app: &mut AppState) {
    end_capture(app);
    app.shortcuts.picker = None;
    // 描いていない欄の位置は残さない
    app.shortcuts.chips.clear();
    app.shortcuts.mode_boxes.clear();
    app.shortcuts.search_rect = None;
    app.shortcuts.slot_rects.clear();
    app.shortcuts.pick_rect = None;
}

/// ぶつかるキーが残っていて保存していないことを知らせる（設定のウィンドウを閉じたときに、どの区分を見ていても出す）。
pub(crate) fn warn_unsaved(app: &mut AppState) {
    if app.keys.unsaved {
        let reason = unsaved_reason(app.lang);
        app.warn(crate::notice::Source::Settings, reason);
    }
}

/// 設定のウィンドウの左で、ショートカットの区分（どこでも・視点／ペイント…）を選ぶ（「キーで探す」の絞り込みは外す）。
pub(crate) fn select_section(app: &mut AppState, section: Section) {
    app.shortcuts.section = section;
    app.shortcuts.key_filter = None;
    app.shortcuts.scroll = 0.0;
    app.shortcuts.picker = None;
}

fn unsaved_reason(lang: Lang) -> &'static str {
    lang.pick(
        "ぶつかるキーがあるので、キーの設定を保存していません",
        "Key settings are not saved while keys conflict",
    )
}

/// 右の欄の中身（`pane` は設定のウィンドウの右の欄のうち、上の探す欄より下）: 表かパイメニューの編集と、下の帯。
pub(crate) fn content(ui: &mut Ui, app: &mut AppState, pane: Rect) {
    app.shortcuts.chips.clear();
    app.shortcuts.mode_boxes.clear();
    let main = Rect::from_min_max(pane.min, pos2(pane.right(), pane.bottom() - FOOTER));
    if app.shortcuts.section == Section::Pies && app.shortcuts.key_filter.is_none() {
        pies(ui, app, main.shrink2(vec2(16.0, 10.0)));
    } else {
        table(ui, app, main.shrink2(vec2(16.0, 10.0)));
    }
    footer(
        ui,
        app,
        Rect::from_min_max(pos2(pane.left(), pane.bottom() - FOOTER), pane.max),
    );
}

fn footer(ui: &mut Ui, app: &mut AppState, r: Rect) {
    let lang = app.lang;
    let p = ui.painter().clone();
    w::fill(&p, r, t::PANEL_HEADER);
    w::hline(&p, r.left(), r.right(), r.top(), t::BORDER);
    let mut x = r.left() + 16.0;
    let y = r.top() + 12.0;
    let mut button = |ui: &mut Ui, label: &str, salt: &str| {
        let bw = w::text_width(ui.painter(), label, t::LABEL) + 30.0;
        let b = Rect::from_min_size(pos2(x, y), vec2(bw, 28.0));
        x = b.right() + 10.0;
        w::button(
            ui,
            b,
            ("yolu.shortcuts", salt),
            label,
            false,
            true,
            None,
            None,
        )
        .clicked()
    };
    if button(ui, lang.pick("既定に戻す", "Reset to Default"), "reset") {
        end_capture(app);
        app.keys_reset_all();
    }
    if button(ui, lang.pick("書き出し…", "Export…"), "export") {
        app.dialog_request = Some(DialogRequest::KeymapExport);
    }
    if button(ui, lang.pick("読み込み…", "Import…"), "import") {
        app.dialog_request = Some(DialogRequest::KeymapImport);
    }
    if app.keys.unsaved || app.keys.has_conflicts() {
        let reason = unsaved_reason(lang);
        let tr = Rect::from_min_max(pos2(x + 6.0, r.top()), pos2(r.right() - 16.0, r.bottom()));
        let shown = w::fit(&p, reason, tr.width(), t::LABEL);
        w::text(&p, tr, &shown, t::LABEL.with_color(t::WARNING), Align::Left);
        ui.interact(tr, Id::new("yolu.shortcuts.unsaved"), Sense::hover())
            .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, reason));
    }
}

/// 上の帯（名前で探す欄と「キーで探す」）。
fn search_bar(ui: &mut Ui, app: &mut AppState, r: Rect) {
    let lang = app.lang;
    let ctx = ui.ctx().clone();
    let button_label = match app.shortcuts.key_filter {
        Some(t) => trigger_label(&t),
        None => lang.pick("キーで探す", "Find by Key").to_owned(),
    };
    let searching = matches!(
        app.shortcuts.capture,
        Some(Capture {
            target: Target::Search,
            ..
        })
    );
    let bw = w::text_width(ui.painter(), &button_label, t::LABEL).max(60.0) + 30.0;
    let b = Rect::from_min_size(pos2(r.right() - bw, r.top()), vec2(bw, r.height()));
    let field = Rect::from_min_max(r.min, pos2(b.left() - 10.0, r.bottom()));
    {
        let p = ui.painter();
        w::rounded(p, field, t::CONTROL_BG, 3.0);
        w::outline(p, field, t::BORDER, 1.0, 3.0);
        w::icon(
            p,
            Rect::from_min_size(field.min + vec2(6.0, 0.0), vec2(18.0, field.height())),
            "search",
            t::TEXT_DIM,
            16.0,
        );
    }
    let inner = Rect::from_min_max(field.min + vec2(30.0, 1.0), field.max - vec2(6.0, 1.0));
    app.shortcuts.search_rect = Some(inner);
    let mut text = app.shortcuts.search.clone();
    let response = ui.put(
        inner,
        egui::TextEdit::singleline(&mut text)
            .id(Id::new("yolu.shortcuts.search"))
            .frame(egui::Frame::NONE)
            .font(t::LABEL.font())
            .text_color(t::TEXT)
            .hint_text(lang.pick("探す", "Search"))
            .margin(egui::Margin::ZERO)
            .vertical_align(egui::Align::Center),
    );
    if response.changed() {
        app.shortcuts.search = text;
        app.shortcuts.scroll = 0.0;
    }
    if w::button(
        ui,
        b,
        ("yolu.shortcuts", "find_key"),
        &button_label,
        searching || app.shortcuts.key_filter.is_some(),
        true,
        None,
        None,
    )
    .clicked()
    {
        if app.shortcuts.key_filter.take().is_none() && !searching {
            begin_capture(&ctx, app, Target::Search, b);
        } else if searching {
            end_capture(app);
        }
        app.shortcuts.scroll = 0.0;
    }
}

fn table(ui: &mut Ui, app: &mut AppState, r: Rect) {
    let lang = app.lang;
    let ctx = ui.ctx().clone();
    search_bar(ui, app, Rect::from_min_size(r.min, vec2(r.width(), 28.0)));
    let head = Rect::from_min_size(pos2(r.left(), r.top() + TOP - 14.0), vec2(r.width(), HEAD));
    let col_key = head.left() + head.width() * 0.46;
    let col_mouse = head.left() + head.width() * 0.74;
    let section = app.shortcuts.section;
    let mut window = std::mem::take(&mut app.shortcuts);
    let lines = lines(&mut window, app, section);
    app.shortcuts = window;
    // マウスの欄には、ツールの行では、キーの動き方の箱が並ぶ（その行があれば、見出しも）
    let has_tool_rows = lines.iter().any(
        |l| matches!(l, Line::Key { group, locked: false } if commands::selected_tool(group.0).is_some()),
    );
    {
        let p = ui.painter();
        w::text(
            p,
            Rect::from_min_max(
                pos2(head.left() + 10.0, head.top()),
                pos2(col_key, head.bottom()),
            ),
            lang.pick("操作", "Action"),
            t::LABEL_DIM,
            Align::Left,
        );
        w::text(
            p,
            Rect::from_min_max(pos2(col_key, head.top()), pos2(col_mouse, head.bottom())),
            lang.pick("キー", "Key"),
            t::LABEL_DIM,
            Align::Left,
        );
        w::text(
            p,
            Rect::from_min_max(pos2(col_mouse, head.top()), head.max),
            if has_tool_rows {
                lang.pick("マウス・動き方", "Mouse / Behavior")
            } else {
                lang.pick("マウス", "Mouse")
            },
            t::LABEL_DIM,
            Align::Left,
        );
        w::hline(p, head.left(), head.right(), head.bottom(), t::SEPARATOR);
    }
    let list = Rect::from_min_max(pos2(r.left(), head.bottom() + 1.0), r.max);
    let mut scroll = app.shortcuts.scroll;
    let bar = Scroll::begin(ui, list, lines.len() as f32 * ROW, &mut scroll);
    let mut child = ui.new_child(UiBuilder::new().max_rect(list));
    child.set_clip_rect(list.intersect(ui.clip_rect()));
    let capture = app.shortcuts.capture;
    let held = ctx.input(|i| i.modifiers);
    for (i, line) in lines.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(list.left(), list.top() + i as f32 * ROW - scroll),
            vec2(list.width() - bar.reserved(), ROW),
        );
        if row.bottom() < list.top() || row.top() > list.bottom() {
            continue;
        }
        draw_line(
            &mut child, app, &ctx, *line, row, col_key, col_mouse, capture, &held,
        );
    }
    bar.end(ui, Id::new("yolu.shortcuts.scroll"), &mut scroll);
    app.shortcuts.scroll = scroll;
}

#[allow(clippy::too_many_arguments)]
fn draw_line(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    line: Line,
    row: Rect,
    col_key: f32,
    col_mouse: f32,
    capture: Option<Capture>,
    held: &Modifiers,
) {
    let lang = app.lang;
    let p = ui.painter().clone();
    let (name, when, changed) = match line {
        Line::Key { group, .. } => {
            let rows: Vec<keymap::KeyBinding> = app
                .keys
                .map()
                .rows()
                .iter()
                .filter(|b| (b.command, b.scope) == group)
                .copied()
                .collect();
            let when = rows
                .first()
                .map(|b| b.when)
                .or_else(|| {
                    keymap::default_map()
                        .rows()
                        .iter()
                        .find(|b| (b.command, b.scope) == group)
                        .map(|b| b.when)
                })
                .and_then(|w| when_label(lang, w));
            (command_name(app, group.0), when, app.keys.is_changed(group))
        }
        Line::Fixed { command, .. } => (
            commands::find(command)
                .and_then(|c| c.static_label(lang))
                .unwrap_or(command)
                .to_owned(),
            None,
            false,
        ),
        Line::Mouse(index) => (mouse_name(lang, index), None, app.keys.combo_changed(index)),
        Line::FixedMouse => (lang_select(lang).to_owned(), None, false),
    };
    // ぶつかりの有無（行を赤く）
    let conflict_in_row = match line {
        Line::Key { group, .. } => {
            (0..app.keys.triggers(group).len()).any(|i| app.keys.conflict(group, i).is_some())
        }
        Line::Mouse(index) => app.keys.combo_conflict(index).is_some(),
        Line::Fixed { .. } | Line::FixedMouse => false,
    };
    if conflict_in_row {
        w::fill(
            &p,
            row,
            egui::Color32::from_rgba_unmultiplied(229, 83, 75, 36),
        );
    }
    w::hline(
        &p,
        row.left(),
        row.right(),
        row.bottom() - 0.5,
        t::SEPARATOR,
    );
    let name_rect = Rect::from_min_max(
        pos2(row.left() + 10.0, row.top()),
        pos2(col_key - 8.0, row.bottom()),
    );
    let shown = w::fit(&p, &name, name_rect.width(), t::LABEL);
    w::text(&p, name_rect, &shown, t::LABEL, Align::Left);
    if let Some(when) = when {
        let nw = w::text_width(&p, &shown, t::LABEL);
        let wr = Rect::from_min_max(pos2(name_rect.left() + nw + 8.0, row.top()), name_rect.max);
        if wr.width() > 20.0 {
            let ws = w::fit(&p, &when, wr.width(), t::LABEL_DIM);
            w::text(&p, wr, &ws, t::LABEL_DIM, Align::Left);
        }
    }
    let chip_y = row.top() + (ROW - 20.0) / 2.0;
    match line {
        Line::Key { group, locked } => {
            let triggers = app.keys.triggers(group);
            let mut x = col_key;
            for (i, trigger) in triggers.iter().enumerate() {
                let target = Target::Key {
                    group,
                    chip: Some(i),
                };
                let capturing = capture.map(|c| c.target) == Some(target);
                let text = if capturing {
                    held_modifiers(held)
                } else {
                    trigger_label(trigger)
                };
                let conflict = app.keys.conflict(group, i);
                let tip = if locked {
                    Some(
                        lang.pick("このキーは変えられません", "This key cannot be changed")
                            .to_owned(),
                    )
                } else if let Some(other) = conflict {
                    Some(same_key_tip(app, other))
                } else if let Some(other) = app.keys.shadowed_by(group, i) {
                    Some(first_tip(app, other))
                } else {
                    // 取る側（モードの段）にも、先に取る相手を出す
                    app.keys
                        .takes_before(group, i)
                        .map(|other| takes_tip(app, group, other))
                };
                let (clicked, width) = chip(
                    ui,
                    app,
                    pos2(x, chip_y),
                    target,
                    &text,
                    capturing,
                    conflict.is_some(),
                    !locked,
                    tip.as_deref(),
                    &name,
                );
                if clicked && !app.shortcuts.ignore_click {
                    let r = app.shortcuts.chip_rect(target).unwrap_or(row);
                    begin_capture(ctx, app, target, r);
                }
                x += width + 6.0;
            }
            // 押している間だけ効くキーと途中の操作のキーは 1 つだけ（加える欄は出さない）
            if !locked && x < col_mouse - 30.0 && (triggers.is_empty() || !single_key(group)) {
                // 入力を加える（入力が無い行は空の欄）
                let target = Target::Key { group, chip: None };
                let capturing = capture.map(|c| c.target) == Some(target);
                if triggers.is_empty() || capturing {
                    let text = if capturing {
                        held_modifiers(held)
                    } else {
                        String::new()
                    };
                    let (clicked, _) = chip(
                        ui,
                        app,
                        pos2(x, chip_y),
                        target,
                        &text,
                        capturing,
                        false,
                        true,
                        None,
                        &name,
                    );
                    if clicked && !app.shortcuts.ignore_click {
                        let r = app.shortcuts.chip_rect(target).unwrap_or(row);
                        begin_capture(ctx, app, target, r);
                    }
                } else {
                    let r = Rect::from_min_size(pos2(x, chip_y), vec2(20.0, 20.0));
                    app.shortcuts.chips.push((target, r));
                    if w::icon_button(
                        ui,
                        r,
                        ("yolu.shortcuts.add", group.0, group.1 as u8),
                        "add",
                        lang.pick("キーを追加", "Add Key"),
                        false,
                        true,
                        14.0,
                    )
                    .clicked()
                    {
                        begin_capture(ctx, app, target, r);
                    }
                }
            }
            // ツールのキーの動き方（押すと切り替え・押している間だけ・短く押すと切り替え／長押しで押している間だけ）
            if !locked && commands::selected_tool(group.0).is_some() {
                tool_mode_box(ui, app, ctx, group.0, row, col_mouse, chip_y);
            }
        }
        Line::Fixed { key, .. } => {
            let tip = lang.pick("このキーは変えられません", "This key cannot be changed");
            let target = Target::Key {
                group: ("", Scope::Everywhere),
                chip: None,
            };
            let _ = chip(
                ui,
                app,
                pos2(col_key, chip_y),
                target,
                super::key_text_of(key),
                false,
                false,
                false,
                Some(tip),
                &name,
            );
        }
        Line::FixedMouse => {
            let tip = lang.pick(
                "このボタンは変えられません",
                "This button cannot be changed",
            );
            let _ = chip(
                ui,
                app,
                pos2(col_mouse, chip_y),
                Target::Key {
                    group: ("", Scope::Edit),
                    chip: None,
                },
                lang.pick("左ボタン", "Left Button"),
                false,
                false,
                false,
                Some(tip),
                &name,
            );
        }
        Line::Mouse(index) => {
            let target = Target::Mouse(index);
            let capturing = capture.map(|c| c.target) == Some(target);
            let text = match (capturing, app.keys.combo(index)) {
                (true, _) => held_modifiers(held),
                (false, Some(c)) => combo_label(lang, index, c),
                (false, None) => String::new(),
            };
            let conflict = app.keys.combo_conflict(index);
            let tip = conflict.map(|c| match c {
                keyconfig::ComboConflict::With(other) => {
                    let (open, close) = lang.pick(("「", "」"), ("“", "”"));
                    lang.pick(
                        format!("{open}{}{close}とぶつかります", mouse_name(lang, other)),
                        format!("Conflicts with {open}{}{close}", mouse_name(lang, other)),
                    )
                }
                keyconfig::ComboConflict::Painting => lang
                    .pick("描く操作とぶつかります", "Conflicts with painting")
                    .to_owned(),
            });
            let (clicked, _) = chip(
                ui,
                app,
                pos2(col_mouse, chip_y),
                target,
                &text,
                capturing,
                conflict.is_some(),
                true,
                tip.as_deref(),
                &name,
            );
            if clicked && !app.shortcuts.ignore_click && !capturing {
                let r = app.shortcuts.chip_rect(target).unwrap_or(row);
                begin_capture(ctx, app, target, r);
            }
        }
    }
    if changed {
        let r = Rect::from_min_size(pos2(row.right() - 26.0, row.top() + 5.0), vec2(22.0, 21.0));
        let salt = format!("{line:?}");
        if w::icon_button(
            ui,
            r,
            ("yolu.shortcuts.reset", salt),
            "restart_alt",
            lang.pick("既定に戻す", "Reset to Default"),
            false,
            true,
            15.0,
        )
        .clicked()
        {
            end_capture(app);
            match line {
                Line::Key { group, .. } => app.keys.reset(group),
                Line::Mouse(index) => app.keys.reset_combo(index),
                Line::Fixed { .. } | Line::FixedMouse => {}
            }
            app.keys_changed();
        }
    }
}

/// ツールのキーの動き方を選ぶ箱（マウスの欄の位置。押すと、動き方の一覧を開く）。
fn tool_mode_box(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    command: &'static str,
    row: Rect,
    col_mouse: f32,
    chip_y: f32,
) {
    let lang = app.lang;
    let mode = app.keys.tool_mode(command);
    let width = (row.right() - 36.0 - col_mouse).clamp(90.0, 190.0);
    let r = Rect::from_min_size(pos2(col_mouse, chip_y), vec2(width, 20.0));
    app.shortcuts.mode_boxes.push((command, r));
    let open = matches!(
        app.popup.as_ref().map(|p| p.kind),
        Some(PopupKind::ToolKeyMode(c)) if c == command
    );
    let (response, _) = w::dropdown(
        ui,
        r,
        ("yolu.shortcuts.mode", command),
        None,
        mode.short_label(lang),
        Some(mode.label(lang)),
        true,
        0.0,
    );
    if response.clicked() {
        if open {
            app.popup = None;
        } else {
            let anchor =
                Rect::from_min_size(pos2(r.left(), r.bottom() + 2.0), vec2(r.width(), 0.0));
            let mut state = PopupState::new(ctx, anchor).with_min_width(r.width());
            state.selected = ToolKeyMode::ALL.iter().position(|m| *m == mode);
            app.popup = Some(OpenPopup {
                kind: PopupKind::ToolKeyMode(command),
                state,
            });
        }
    }
}

/// ツールのキーの動き方を選ぶ一覧の項目（今の動き方に印）。
pub fn tool_mode_entries(app: &AppState, command: &'static str) -> Vec<Entry<Action>> {
    let current = app.keys.tool_mode(command);
    ToolKeyMode::ALL
        .into_iter()
        .map(|mode| {
            Entry::item(mode.label(app.lang), Action::ToolKeyMode(command, mode))
                .radio(mode == current)
        })
        .collect()
}

/// 「同じキー」のツールチップ（相手の名前と区分）。
fn same_key_tip(app: &AppState, other: GroupKey) -> String {
    let lang = app.lang;
    let (open, close) = lang.pick(("「", "」"), ("“", "”"));
    let name = command_name(app, other.0);
    let section = Section::of_scope(other.1).label(lang);
    lang.pick(
        format!("{open}{name}{close}（{section}）と同じキー"),
        format!("Same key as {open}{name}{close} ({section})"),
    )
}

/// 取る側（モードの段）の「先に取る」のツールチップ。
fn takes_tip(app: &AppState, group: GroupKey, other: GroupKey) -> String {
    let lang = app.lang;
    let (open, close) = lang.pick(("「", "」"), ("“", "”"));
    let name = command_name(app, other.0);
    let mine = Section::of_scope(group.1).label(lang);
    let section = Section::of_scope(other.1).label(lang);
    lang.pick(
        format!("{mine}のモードでは、{section}の{open}{name}{close}より先に取ります"),
        format!("In {mine} mode, takes this key before {open}{name}{close} ({section})"),
    )
}

/// 「先に取る」のツールチップ（モードの段の同じキーが先に取る。ぶつかりではない）。
fn first_tip(app: &AppState, other: GroupKey) -> String {
    let lang = app.lang;
    let (open, close) = lang.pick(("「", "」"), ("“", "”"));
    let name = command_name(app, other.0);
    let section = Section::of_scope(other.1).label(lang);
    lang.pick(
        format!("{section}のモードでは{open}{name}{close}が先に取ります"),
        format!("In {section} mode, {open}{name}{close} takes this key first"),
    )
}

// ───────── パイメニュー ─────────

/// 8 か所の名前（上から時計回り）。
pub fn slot_name(lang: Lang, slot: usize) -> &'static str {
    [
        lang.pick("上", "Top"),
        lang.pick("右上", "Top Right"),
        lang.pick("右", "Right"),
        lang.pick("右下", "Bottom Right"),
        lang.pick("下", "Bottom"),
        lang.pick("左下", "Bottom Left"),
        lang.pick("左", "Left"),
        lang.pick("左上", "Top Left"),
    ][slot]
}

fn pies(ui: &mut Ui, app: &mut AppState, r: Rect) {
    let lang = app.lang;
    let ctx = ui.ctx().clone();
    if app.shortcuts.pie >= app.pie.menus.len() {
        app.shortcuts.pie = 0;
    }
    // パイの並び（押して選ぶ）と、追加・削除
    let mut x = r.left();
    let y = r.top();
    let names: Vec<String> = app.pie.menus.iter().map(|m| m.name(lang)).collect();
    for (i, name) in names.iter().enumerate() {
        let bw = w::text_width(ui.painter(), name, t::LABEL) + 26.0;
        let b = Rect::from_min_size(pos2(x, y), vec2(bw, 26.0));
        if w::button(
            ui,
            b,
            ("yolu.shortcuts.pie", i),
            name,
            app.shortcuts.pie == i,
            true,
            None,
            None,
        )
        .clicked()
        {
            app.shortcuts.pie = i;
            app.shortcuts.picker = None;
        }
        x = b.right() + 6.0;
    }
    let add = Rect::from_min_size(pos2(x, y + 2.0), vec2(22.0, 22.0));
    if w::icon_button(
        ui,
        add,
        "yolu.shortcuts.pie_add",
        "add",
        lang.pick("パイメニューを追加", "Add Pie Menu"),
        false,
        true,
        15.0,
    )
    .clicked()
    {
        app.pie.add_user(lang);
        app.shortcuts.pie = app.pie.menus.len() - 1;
        app.shortcuts.picker = None;
        app.keys_changed();
    }
    let index = app.shortcuts.pie;
    let menu = app.pie.menus[index].clone();
    let user = matches!(menu.name, PieName::User(_));
    let remove = Rect::from_min_size(pos2(add.right() + 4.0, y + 2.0), vec2(22.0, 22.0));
    let remove_tip = if user {
        lang.pick("このパイメニューを削除", "Delete This Pie Menu")
            .to_owned()
    } else {
        lang.with_reason(
            lang.pick("削除できません", "Cannot delete"),
            lang.pick("最初からあるパイメニューです", "it is a built-in pie menu"),
        )
    };
    if w::icon_button(
        ui,
        remove,
        "yolu.shortcuts.pie_remove",
        "delete",
        &remove_tip,
        false,
        user,
        15.0,
    )
    .clicked()
    {
        if let Some(command) = menu.command {
            app.keys.forget_command(command);
        }
        app.pie.remove_user(&menu.id);
        app.shortcuts.pie = 0;
        app.shortcuts.picker = None;
        app.keys_changed();
        return;
    }
    // 名前とキー
    let mut y = y + 40.0;
    let label_w = 90.0;
    {
        let p = ui.painter();
        w::text(
            p,
            Rect::from_min_size(pos2(r.left(), y), vec2(label_w, 24.0)),
            lang.pick("名前", "Name"),
            t::LABEL_DIM,
            Align::Left,
        );
    }
    let name_rect = Rect::from_min_size(pos2(r.left() + label_w, y), vec2(260.0, 24.0));
    if user {
        let out = w::text_field(
            ui,
            name_rect,
            ("yolu.shortcuts.pie_name", &menu.id),
            &menu.name(lang),
            None,
            false,
        );
        if let Some(name) = out
            .committed
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty())
        {
            app.pie.menus[index].name = PieName::User(name);
            app.shortcuts.names = None;
            app.keys_changed();
        }
    } else {
        let p = ui.painter();
        w::text(p, name_rect, &menu.name(lang), t::LABEL, Align::Left);
    }
    y += 32.0;
    {
        let p = ui.painter();
        w::text(
            p,
            Rect::from_min_size(pos2(r.left(), y), vec2(label_w, 24.0)),
            lang.pick("キー", "Key"),
            t::LABEL_DIM,
            Align::Left,
        );
    }
    if let Some(command) = menu.command {
        let held = ctx.input(|i| i.modifiers);
        let group = (command, Scope::Everywhere);
        let capture = app.shortcuts.capture;
        let triggers = app.keys.triggers(group);
        let mut x = r.left() + label_w;
        let name = command_name(app, command);
        for (i, trigger) in triggers.iter().enumerate() {
            let target = Target::Key {
                group,
                chip: Some(i),
            };
            let capturing = capture.map(|c| c.target) == Some(target);
            let text = if capturing {
                held_modifiers(&held)
            } else {
                trigger_label(trigger)
            };
            let conflict = app.keys.conflict(group, i);
            let tip = conflict.map(|o| same_key_tip(app, o));
            let (clicked, width) = chip(
                ui,
                app,
                pos2(x, y + 2.0),
                target,
                &text,
                capturing,
                conflict.is_some(),
                true,
                tip.as_deref(),
                &name,
            );
            if clicked {
                let rr = app.shortcuts.chip_rect(target).unwrap_or(r);
                begin_capture(&ctx, app, target, rr);
            }
            x += width + 6.0;
        }
        if triggers.is_empty() {
            let target = Target::Key { group, chip: None };
            let capturing = capture.map(|c| c.target) == Some(target);
            let text = if capturing {
                held_modifiers(&held)
            } else {
                String::new()
            };
            let (clicked, _) = chip(
                ui,
                app,
                pos2(x, y + 2.0),
                target,
                &text,
                capturing,
                false,
                true,
                None,
                &name,
            );
            if clicked {
                let rr = app.shortcuts.chip_rect(target).unwrap_or(r);
                begin_capture(&ctx, app, target, rr);
            }
        }
    }
    y += 40.0;
    // 8 か所
    app.shortcuts.slot_rects.clear();
    for slot in 0..crate::ui::pie::SLOTS {
        let row = Rect::from_min_size(pos2(r.left(), y), vec2(r.width(), 28.0));
        let p = ui.painter().clone();
        w::text(
            &p,
            Rect::from_min_size(row.min, vec2(label_w, row.height())),
            slot_name(lang, slot),
            t::LABEL_DIM,
            Align::Left,
        );
        let value = Rect::from_min_max(
            pos2(row.left() + label_w, row.top() + 2.0),
            pos2(row.right() - 30.0, row.bottom() - 2.0),
        );
        let item = app.pie.menus[index].slots[slot].clone();
        let label = item
            .as_ref()
            .map(|i| crate::pie::slot_of(app, i).label)
            .unwrap_or_default();
        let picking = app
            .shortcuts
            .picker
            .as_ref()
            .is_some_and(|p| p.slot == slot);
        app.shortcuts.slot_rects.push(value);
        if w::button(
            ui,
            value,
            ("yolu.shortcuts.slot", slot),
            &label,
            picking,
            true,
            None,
            None,
        )
        .clicked()
        {
            app.shortcuts.picker = if picking {
                None
            } else {
                Some(Picker {
                    slot,
                    kind: match item {
                        Some(PieItem::Pie(_)) => PickKind::Pie,
                        Some(PieItem::Ops(_)) => PickKind::Ops,
                        Some(PieItem::Recorded(_)) => PickKind::Recorded,
                        _ => PickKind::Command,
                    },
                    query: String::new(),
                })
            };
        }
        let clear =
            Rect::from_min_size(pos2(row.right() - 26.0, row.top() + 3.0), vec2(22.0, 22.0));
        if item.is_some()
            && w::icon_button(
                ui,
                clear,
                ("yolu.shortcuts.slot_clear", slot),
                "close",
                lang.pick("空にする", "Clear"),
                false,
                true,
                13.0,
            )
            .clicked()
        {
            app.pie.menus[index].slots[slot] = None;
            app.keys_changed();
        }
        y += 30.0;
    }
    if app.shortcuts.picker.is_some() {
        picker(
            ui,
            app,
            index,
            Rect::from_min_max(pos2(r.left(), y + 8.0), r.max),
        );
    }
}

/// 項目を選ぶ所（種類のタブ・名前で探す欄・候補の並び。命令は JSON の欄）。
fn picker(ui: &mut Ui, app: &mut AppState, index: usize, r: Rect) {
    let lang = app.lang;
    let Some(mut picker) = app.shortcuts.picker.clone() else {
        return;
    };
    let labels: Vec<&str> = PickKind::ALL.iter().map(|k| k.label(lang)).collect();
    let active = PickKind::ALL
        .iter()
        .position(|k| *k == picker.kind)
        .unwrap_or(0);
    // 帯・探す欄・一覧は、設定の区分の右端（上の探す欄と同じ端）まで
    let tabs = Rect::from_min_size(r.min, vec2(r.width(), 26.0));
    let chosen = w::tab_strip(
        ui,
        tabs,
        "yolu.shortcuts.pick_kind",
        &labels,
        &["tune", "grid_dots", "edit", "record"],
        active,
    );
    picker.kind = PickKind::ALL[chosen];
    let mut y = tabs.bottom() + 6.0;
    let field = Rect::from_min_size(pos2(r.left(), y), vec2(tabs.width(), 24.0));
    if picker.kind == PickKind::Ops {
        // yolu-ops の命令（JSON）。Enter で入れる
        let current = match &app.pie.menus[index].slots[picker.slot] {
            Some(PieItem::Ops(text)) => text.clone(),
            _ => String::new(),
        };
        let out = w::text_field(
            ui,
            field,
            ("yolu.shortcuts.ops", picker.slot),
            &current,
            Some("yolu-ops"),
            false,
        );
        if let Some(text) = out.committed {
            let valid = serde_json::from_str::<serde_json::Value>(&text).is_ok();
            if valid {
                app.pie.menus[index].slots[picker.slot] = Some(PieItem::Ops(text));
                app.keys_changed();
                app.shortcuts.picker = None;
                return;
            }
            app.refuse(
                crate::notice::Source::Settings,
                lang.with_reason(
                    lang.pick("命令を入れられません", "Cannot set the command"),
                    lang.pick("JSON として読めません", "it is not valid JSON"),
                ),
            );
        }
        app.shortcuts.picker = Some(picker);
        return;
    }
    let mut query = picker.query.clone();
    app.shortcuts.pick_rect = Some(field);
    {
        let p = ui.painter();
        w::rounded(p, field, t::CONTROL_BG, 3.0);
        w::outline(p, field, t::BORDER, 1.0, 3.0);
    }
    let response = ui.put(
        field.shrink2(vec2(6.0, 1.0)),
        egui::TextEdit::singleline(&mut query)
            .id(Id::new("yolu.shortcuts.pick_query"))
            .frame(egui::Frame::NONE)
            .font(t::LABEL.font())
            .text_color(t::TEXT)
            .hint_text(lang.pick("探す", "Search"))
            .margin(egui::Margin::ZERO)
            .vertical_align(egui::Align::Center),
    );
    if response.changed() {
        picker.query = query.clone();
    }
    y = field.bottom() + 4.0;
    let q = query.trim().to_lowercase();
    let candidates: Vec<(String, PieItem)> = match picker.kind {
        PickKind::Command => commands::all()
            .iter()
            .filter(|c| c.runnable().is_some() && c.unavailable.is_none())
            .map(|c| (command_name(app, c.id), PieItem::Command(c.id.to_owned())))
            .collect(),
        PickKind::Pie => app
            .pie
            .menus
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .map(|(_, m)| (m.name(lang), PieItem::Pie(m.id.clone())))
            .collect(),
        PickKind::Recorded => app
            .automation
            .store
            .items()
            .iter()
            .map(|a| (a.name.clone(), PieItem::Recorded(a.name.clone())))
            .collect(),
        PickKind::Ops => Vec::new(),
    };
    let shown: Vec<(String, PieItem)> = candidates
        .into_iter()
        .filter(|(name, _)| q.is_empty() || name.to_lowercase().contains(&q))
        .collect();
    let list = Rect::from_min_max(pos2(r.left(), y), pos2(r.left() + tabs.width(), r.bottom()));
    let row_h = 24.0;
    if response.changed() {
        app.shortcuts.pick_scroll = 0.0;
    }
    let mut scroll = app.shortcuts.pick_scroll;
    let bar = Scroll::begin(ui, list, shown.len() as f32 * row_h, &mut scroll);
    let mut child = ui.new_child(UiBuilder::new().max_rect(list));
    child.set_clip_rect(list.intersect(ui.clip_rect()));
    let mut chosen = None;
    for (i, (name, item)) in shown.iter().enumerate() {
        let rr = Rect::from_min_size(
            pos2(list.left(), list.top() + i as f32 * row_h - scroll),
            vec2(list.width() - bar.reserved(), row_h),
        );
        if rr.bottom() < list.top() || rr.top() > list.bottom() {
            continue;
        }
        let response = child.interact(rr, Id::new(("yolu.shortcuts.pick", i)), Sense::click());
        let p = child.painter();
        if response.hovered() {
            w::fill(p, rr, t::CONTROL_HOVER);
        }
        // 切れるときは「…」で詰める
        let shown = w::fit(p, name, rr.width() - 16.0, t::LABEL);
        w::text(p, rr.shrink2(vec2(8.0, 0.0)), &shown, t::LABEL, Align::Left);
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, name));
        if response.clicked() {
            chosen = Some(item.clone());
        }
    }
    bar.end(ui, Id::new("yolu.shortcuts.pick_scroll"), &mut scroll);
    app.shortcuts.pick_scroll = scroll;
    if let Some(item) = chosen {
        app.pie.menus[index].slots[picker.slot] = Some(item);
        app.keys_changed();
        app.shortcuts.picker = None;
        return;
    }
    app.shortcuts.picker = Some(picker);
}
