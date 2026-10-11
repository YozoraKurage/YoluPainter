//! パイメニュー（利用者が作れる共通の土台）。描き方と入力は `ui::pie`、ここはパイの中身（項目の種類・最初からあるパイ）と、開く・実行する。
//!
//! - 項目は 8 か所（上から時計回り）。種類は、操作（`commands` の ID）・別のパイ・yolu-ops の命令（JSON）・記録したアクション（アクションの名前）。
//! - 最初からあるパイ: モード（Ctrl+Tab。ペイント・編集・ポーズ）と視点（正面・背面・右・左・上・下・正投影・収める。既定のキーは無い）。
//! - 開くのは `Action::Pie(PieAction::Open)`（キーの表から）。頼みは `PieState::request` に置き、キーの処理（`shell::handle_shortcuts`）が
//!   ポインタの所に開く（押しているキーを覚え、押したまま離したら指している項目を実行する）。開いている間は `PopupKind::Pie` のポップアップとして、
//!   下の入力を止める。

use egui::{Key, Pos2, Rect, Vec2};
use serde_json::Value;

use crate::commands;
use crate::lang::Lang;
use crate::mode::EditorMode;
use crate::notice::Source;
use crate::state::{Action, AppState, OpenPopup, PopupKind};
use crate::ui::menu::PopupState;
use crate::ui::pie::{self as widget, Outcome, Runtime, Slot, SLOTS};

/// パイの項目の行き先。
#[derive(Clone, Debug, PartialEq)]
pub enum PieItem {
    /// 操作（`commands` の ID）。
    Command(String),
    /// 別のパイ（パイの ID。同じ所に開き直す）。
    Pie(String),
    /// yolu-ops の命令（JSON の文。命令 1 つのオブジェクトか、命令の並び。全部を 1 回の取り消しで当てる）。
    Ops(String),
    /// 記録したアクション（アクションのパネルの名前）。
    Recorded(String),
}

/// パイの名前。
#[derive(Clone, Debug, PartialEq)]
pub enum PieName {
    /// 最初からあるパイ（日本語・英語）。
    Builtin(&'static str, &'static str),
    /// 利用者が付けた名前。
    User(String),
}

/// パイ 1 つ。
#[derive(Clone, Debug, PartialEq)]
pub struct PieMenu {
    /// 壊れない ID（最初からあるパイは `mode`・`view`）。
    pub id: String,
    pub name: PieName,
    /// 開く操作（`commands` の ID。キーはこの操作の行）。
    pub command: Option<&'static str>,
    /// 上から時計回りの 8 か所。
    pub slots: [Option<PieItem>; SLOTS],
}

impl PieMenu {
    pub fn name(&self, lang: Lang) -> String {
        match &self.name {
            PieName::Builtin(ja, en) => lang.pick(*ja, *en).to_owned(),
            PieName::User(name) => name.clone(),
        }
    }
}

/// パイの操作。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PieAction {
    /// このパイをポインタの所に開く。
    Open(String),
}

/// 開いているパイ。
#[derive(Clone, Debug, PartialEq)]
pub struct OpenPie {
    pub menu: String,
    pub run: Runtime,
}

/// パイの状態（パイの並びと、開いているもの）。アプリの状態で、.ylp には入れない。
#[derive(Debug)]
pub struct PieState {
    pub menus: Vec<PieMenu>,
    /// 開いているパイ（`PopupKind::Pie` のポップアップと一緒に持つ）。
    pub open: Option<OpenPie>,
    /// 開く頼み（次のキーの処理がポインタの所に開く）。
    pub request: Option<String>,
}

impl Default for PieState {
    fn default() -> Self {
        PieState {
            menus: builtin(),
            open: None,
            request: None,
        }
    }
}

impl PieState {
    pub fn menu(&self, id: &str) -> Option<&PieMenu> {
        self.menus.iter().find(|m| m.id == id)
    }
}

fn command(id: &str) -> Option<PieItem> {
    Some(PieItem::Command(id.to_owned()))
}

/// 最初からあるパイ。
pub fn builtin() -> Vec<PieMenu> {
    vec![
        PieMenu {
            id: "mode".into(),
            name: PieName::Builtin("モード", "Mode"),
            command: Some("mode.pie"),
            // 上: 編集、右: ポーズ、左: ペイント
            slots: [
                command("mode.edit"),
                None,
                command("mode.pose"),
                None,
                None,
                None,
                command("mode.paint"),
                None,
            ],
        },
        PieMenu {
            id: "view".into(),
            name: PieName::Builtin("視点", "View"),
            command: Some("view3d.pie"),
            // 上・右上（正面）・右・右下（正投影）・下・左下（収める）・左・左上（背面）
            slots: [
                command("view3d.view_top"),
                command("view3d.view_front"),
                command("view3d.view_right"),
                command("view3d.ortho"),
                command("view3d.view_bottom"),
                command("view3d.frame_selected"),
                command("view3d.view_left"),
                command("view3d.view_back"),
            ],
        },
    ]
}

/// 新しいパイの ID の数（プロセスの乱数の種と、数え上げから）。
fn random_u32() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(n);
    h.finish() as u32
}

/// 利用者が作ったパイ（開く操作は `pie.<ID>`。キーはショートカットの設定で入れる）。
pub fn user_menu(id: &str, name: String, slots: [Option<PieItem>; SLOTS]) -> PieMenu {
    PieMenu {
        id: id.to_owned(),
        name: PieName::User(name),
        command: Some(crate::keymap::intern(&format!("pie.{id}"))),
        slots,
    }
}

impl PieState {
    /// 利用者のパイを加える（空の 8 か所。名前は「パイメニュー n」）。加えたパイの ID。
    pub fn add_user(&mut self, lang: Lang) -> String {
        let n = (1..)
            .find(|n| {
                let name = lang.pick(format!("パイメニュー {n}"), format!("Pie Menu {n}"));
                !self.menus.iter().any(|m| m.name(lang) == name)
            })
            .unwrap_or(1);
        let id = loop {
            let id = format!("u{:08x}", random_u32());
            if self.menu(&id).is_none() {
                break id;
            }
        };
        let name = lang.pick(format!("パイメニュー {n}"), format!("Pie Menu {n}"));
        self.menus.push(user_menu(&id, name, Default::default()));
        id
    }

    /// 利用者のパイを消す（最初からあるパイは消さない）。消したら true。ほかのパイの、このパイを開く項目は空にする。
    pub fn remove_user(&mut self, id: &str) -> bool {
        let before = self.menus.len();
        self.menus
            .retain(|m| m.id != id || matches!(m.name, PieName::Builtin(..)));
        if self.menus.len() == before {
            return false;
        }
        for m in &mut self.menus {
            for slot in &mut m.slots {
                if matches!(slot, Some(PieItem::Pie(p)) if p == id) {
                    *slot = None;
                }
            }
        }
        true
    }
}

/// 項目の見せ方（名前・アイコン・押せるか・今の状態か・押せない理由）。
pub fn slot_of(app: &AppState, item: &PieItem) -> Slot {
    let lang = app.lang;
    match item {
        PieItem::Command(id) => {
            let found = commands::find(id);
            let label = found
                .and_then(|c| {
                    c.short_label(lang)
                        .map(str::to_owned)
                        .or_else(|| c.label(app))
                })
                .unwrap_or_else(|| id.clone());
            let mode = EditorMode::ALL.into_iter().find(|m| m.command() == id);
            let reason = match (found, mode) {
                (None, _) => Some(unknown_command(lang, id)),
                (Some(_), Some(EditorMode::Pose)) if app.view3d.pose.session.is_none() => {
                    Some(crate::mode::no_rig(lang).to_owned())
                }
                (Some(c), _) => c.unavailable(lang).map(str::to_owned),
            };
            // 入っている切り替え（今のモード・正投影）には印
            let on = mode == Some(app.mode)
                || (id == "view3d.ortho" && app.view3d.camera.is_orthographic());
            Slot {
                label,
                icon: mode.map(EditorMode::icon),
                enabled: reason.is_none(),
                checked: on,
                tooltip: reason,
            }
        }
        PieItem::Pie(id) => {
            let menu = app.pie.menu(id);
            Slot {
                label: menu.map_or_else(|| id.clone(), |m| m.name(lang)),
                icon: None,
                enabled: menu.is_some(),
                checked: false,
                tooltip: menu.is_none().then(|| unknown_pie(lang, id)),
            }
        }
        PieItem::Ops(json) => Slot {
            label: ops_label(json),
            icon: None,
            enabled: true,
            checked: false,
            tooltip: None,
        },
        PieItem::Recorded(name) => {
            let found = app.automation.store.items().iter().any(|a| a.name == *name);
            Slot {
                label: name.clone(),
                icon: Some("play"),
                enabled: found,
                checked: false,
                tooltip: (!found).then(|| no_action(lang, name)),
            }
        }
    }
}

/// yolu-ops の命令の項目の名前（命令の名前。並びなら最初の命令と数）。
fn ops_label(json: &str) -> String {
    let name = |v: &Value| v.get("command").and_then(Value::as_str).map(str::to_owned);
    match serde_json::from_str::<Value>(json) {
        Ok(Value::Array(items)) => match items.first().and_then(name) {
            Some(first) if items.len() > 1 => format!("{first} +{}", items.len() - 1),
            Some(first) => first,
            None => "yolu-ops".into(),
        },
        Ok(v) => name(&v).unwrap_or_else(|| "yolu-ops".into()),
        Err(_) => "yolu-ops".into(),
    }
}

fn unknown_command(lang: Lang, id: &str) -> String {
    lang.pick(
        format!("操作{}がありません", lang.quote(id)),
        format!("No command {}", lang.quote(id)),
    )
}

fn unknown_pie(lang: Lang, id: &str) -> String {
    lang.pick(
        format!("パイメニュー{}がありません", lang.quote(id)),
        format!("No pie menu {}", lang.quote(id)),
    )
}

fn no_action(lang: Lang, name: &str) -> String {
    lang.pick(
        format!("アクション{}がありません", lang.quote(name)),
        format!("No action {}", lang.quote(name)),
    )
}

/// パイを開かない途中の操作か（ポーズのギズモ・形のギズモ・点のドラッグ、3D の視点の操作、範囲のドラッグ、アイランドのメニューの右の押し、
/// 2D の表示の回転・移動・拡縮、右ボタンのスポイト、ステンシルのドラッグ）。開くと、パイの間もポインタの動きでドラッグが進むので、開かずに断る。
pub fn dragging(app: &AppState) -> bool {
    app.view3d.pose.drag.is_some()
        || app.fillfx.drag.is_some()
        || app.fillfx.point_drag.is_some()
        || app.view3d.input.nav.is_some()
        || app.view3d.input.eyedrop.is_some()
        || app.region.drag.is_some()
        || app.bake.menu_press.is_some()
        || app.canvas.rotating.is_some()
        || app.canvas.panning
        || app.canvas.zooming.is_some()
        || app.canvas.eyedrop.is_some()
        || app.stencil.drag.is_some()
}

/// ドラッグの途中で開けないときの断り。
fn refuse_during_drag(app: &mut AppState) {
    let lang = app.lang;
    let text = lang.with_reason(
        lang.pick("パイメニューを開けません", "Cannot open the pie menu"),
        lang.pick("ほかの操作の途中です", "another operation is in progress"),
    );
    app.refuse(Source::Edit, text);
}

impl AppState {
    /// パイの操作を当てる（`Action::Pie`）。開くのは頼みを置くだけ（ポインタの所に開くのは、キーの処理）。描いている間・ドラッグの途中は開かない。
    pub fn pie_apply(&mut self, action: PieAction) {
        match action {
            PieAction::Open(id) => {
                if self.is_stroking() {
                    self.refuse(
                        Source::Edit,
                        crate::lang::refusals::during_stroke(self.lang),
                    );
                    return;
                }
                if dragging(self) {
                    refuse_during_drag(self);
                    return;
                }
                if self.pie.menu(&id).is_none() {
                    let text = self.lang.with_reason(unknown_pie(self.lang, &id), "");
                    self.refuse(Source::Edit, text);
                    return;
                }
                self.pie.request = Some(id);
            }
        }
    }
}

/// 頼まれたパイを、ポインタの所に開く（キーの処理から。パイを開いたキーを今押していれば、それを覚える: 押したまま離したら指している項目を実行する）。
/// マウスのボタンを押している間とドラッグの途中は開かない（パイの間もポインタの動きで下のドラッグが進むので）。
pub fn open_requested(ctx: &egui::Context, app: &mut AppState) {
    let Some(id) = app.pie.request.take() else {
        return;
    };
    let Some(menu) = app.pie.menu(&id) else {
        return;
    };
    if app.is_stroking() {
        return;
    }
    let command = menu.command;
    if dragging(app) || ctx.input(|i| i.pointer.any_down()) {
        refuse_during_drag(app);
        return;
    }
    let key = command.and_then(|command| {
        let map = crate::keymap::current();
        let keys: Vec<Key> = map.rows_of(command).filter_map(|b| b.key()).collect();
        ctx.input(|i| keys.into_iter().find(|k| i.key_down(*k)))
    });
    let center = ctx
        .input(|i| i.pointer.latest_pos())
        .unwrap_or_else(|| ctx.content_rect().center());
    open_at(ctx, app, id, center, key);
}

/// パイを `center` に開く（今のポップアップは閉じる）。
pub fn open_at(
    ctx: &egui::Context,
    app: &mut AppState,
    id: String,
    center: Pos2,
    key: Option<Key>,
) {
    app.pie.open = Some(OpenPie {
        menu: id,
        run: Runtime::new(ctx, center, key),
    });
    app.popup = Some(OpenPopup {
        kind: PopupKind::Pie,
        state: PopupState::new(ctx, Rect::from_center_size(center, Vec2::ZERO)),
    });
}

/// 開いているパイを描いて入力を受ける（`PopupKind::Pie` のポップアップを描くウィンドウのパスから）。開いたままなら true。
/// 選んだ項目は、パイを閉じてから実行する（別のパイなら同じ所に開き直す）。
pub fn show(ctx: &egui::Context, app: &mut AppState, popup: &mut PopupState) -> bool {
    let Some(mut open) = app.pie.open.take() else {
        return false;
    };
    let Some(menu) = app.pie.menu(&open.menu).cloned() else {
        return false;
    };
    let slots: [Option<Slot>; SLOTS] =
        std::array::from_fn(|i| menu.slots[i].as_ref().map(|item| slot_of(app, item)));
    let outcome = widget::show(ctx, egui::Id::new("yolu.pie"), &mut open.run, &slots);
    popup.rect = Rect::from_center_size(open.run.center, Vec2::splat(2.0 * widget::RADIUS));
    match outcome {
        Outcome::Open => {
            app.pie.open = Some(open);
            true
        }
        Outcome::Close => false,
        Outcome::Chosen(i) => match menu.slots[i].clone() {
            Some(PieItem::Pie(next)) if app.pie.menu(&next).is_some() => {
                // 別のパイ: 同じ所に、クリックで選ぶ形で開き直す
                open.menu = next;
                open.run = Runtime::new(ctx, open.run.center, None);
                app.pie.open = Some(open);
                true
            }
            Some(item) => {
                run(app, &item);
                // 実行した操作がパイを開く頼み（別のパイを開く操作）を置いたら、同じ所に開く
                if let Some(next) = app.pie.request.take() {
                    if app.pie.menu(&next).is_some() {
                        app.pie.open = Some(OpenPie {
                            menu: next,
                            run: Runtime::new(ctx, open.run.center, None),
                        });
                        return true;
                    }
                }
                false
            }
            None => false,
        },
    }
}

/// 項目を実行する（パイを閉じたあと）。押せない項目は理由を知らせて何もしない。
pub fn run(app: &mut AppState, item: &PieItem) {
    let lang = app.lang;
    match item {
        PieItem::Command(id) => {
            let Some(command) = commands::find(id) else {
                let text = lang.with_reason(unknown_command(lang, id), "");
                app.refuse(Source::Edit, text);
                return;
            };
            if let Some(reason) = command.unavailable(lang) {
                let what = command.label(app).unwrap_or_else(|| id.clone());
                let text = lang.with_reason(
                    lang.pick(
                        format!("{what}は使えません"),
                        format!("{what} is not available"),
                    ),
                    reason,
                );
                app.refuse(Source::Edit, text);
                return;
            }
            match command.runnable() {
                Some(action) => app.apply(action),
                None => {
                    let text = lang.with_reason(unknown_command(lang, id), "");
                    app.refuse(Source::Edit, text);
                }
            }
        }
        PieItem::Pie(id) => app.apply(Action::Pie(PieAction::Open(id.clone()))),
        PieItem::Ops(json) => run_ops(app, json),
        PieItem::Recorded(name) => {
            match app
                .automation
                .store
                .items()
                .iter()
                .position(|a| a.name == *name)
            {
                Some(index) => app.apply(Action::Automation(
                    crate::automation::AutomationOp::Play(index),
                )),
                None => {
                    let text = lang.with_reason(no_action(lang, name), "");
                    app.refuse(Source::Action, text);
                }
            }
        }
    }
}

/// yolu-ops の命令（JSON）を、今の文書に 1 回の取り消しで当てる（アクションの再生と同じ口）。
fn run_ops(app: &mut AppState, json: &str) {
    let lang = app.lang;
    let what = lang.pick("命令を実行できません", "Cannot run the commands");
    let items = match serde_json::from_str::<Value>(json) {
        Ok(Value::Array(items)) => items,
        Ok(v) => vec![v],
        Err(_) => {
            let text = lang.with_reason(
                what,
                lang.pick("JSON を読めません", "the JSON cannot be read"),
            );
            app.refuse(Source::Ops, text);
            return;
        }
    };
    let commands = match yolu_ops::action::parse_list(&items) {
        Ok(c) => c,
        Err(e) => {
            let text = lang.with_reason(what, e.message.pick(crate::automation::ops_lang(lang)));
            app.refuse(Source::Ops, text);
            return;
        }
    };
    if app.is_stroking() {
        app.refuse(Source::Ops, crate::lang::refusals::during_stroke(lang));
        return;
    }
    // 記録の間は当てない（アクションの再生と同じ。記録に入らない変更を文書に残さない）
    if app.automation.recorder.is_some() {
        let text = lang.with_reason(what, lang.pick("記録中です", "Recording is in progress"));
        app.refuse(Source::Ops, text);
        return;
    }
    let result = {
        let mut host = crate::ops_host::AppHost::new(app);
        yolu_ops::action::run(&mut host, &commands)
    };
    if let Err(e) = result {
        let text = lang.with_reason(what, e.message.pick(crate::automation::ops_lang(lang)));
        match e.code {
            yolu_ops::ErrorCode::Busy | yolu_ops::ErrorCode::ReadOnly => {
                app.refuse(Source::Ops, text)
            }
            _ => app.fail(Source::Ops, text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view3d::pose::PoseAction;
    use serde_json::json;

    fn labels(app: &AppState, id: &str) -> Vec<Option<String>> {
        let menu = app.pie.menu(id).expect("パイ");
        menu.slots
            .iter()
            .map(|s| s.as_ref().map(|item| slot_of(app, item).label))
            .collect()
    }

    #[test]
    fn the_built_in_pies_put_their_items_where_the_design_says() {
        let mut app = AppState::new(32, 32);
        let some = |s: &str| Some(s.to_owned());
        assert_eq!(
            labels(&app, "mode"),
            [
                some("編集"),
                None,
                some("ポーズ"),
                None,
                None,
                None,
                some("ペイント"),
                None
            ]
        );
        assert_eq!(
            labels(&app, "view"),
            ["上", "正面", "右", "正投影", "下", "収める", "左", "背面"].map(some)
        );
        app.lang = Lang::En;
        assert_eq!(
            labels(&app, "mode"),
            [
                some("Edit"),
                None,
                some("Pose"),
                None,
                None,
                None,
                some("Paint"),
                None
            ]
        );
        assert_eq!(
            labels(&app, "view"),
            [
                "Top",
                "Front",
                "Right",
                "Orthographic",
                "Bottom",
                "Frame",
                "Left",
                "Back"
            ]
            .map(some)
        );
        // 開く操作とキー: モードは Ctrl+Tab、視点は既定のキーが無い
        assert_eq!(app.pie.menu("mode").unwrap().command, Some("mode.pie"));
        assert_eq!(app.pie.menu("view").unwrap().command, Some("view3d.pie"));
        assert!(crate::keymap::primary("mode.pie").is_some());
        assert!(crate::keymap::primary("view3d.pie").is_none());
        for c in ["view3d.view_front", "view3d.view_top", "view3d.ortho"] {
            assert!(crate::keymap::primary(c).is_none(), "{c}");
        }
    }

    #[test]
    fn the_mode_pie_marks_the_current_mode_and_the_view_pie_marks_orthographic() {
        let mut app = AppState::new(32, 32);
        let menu = app.pie.menu("mode").cloned().unwrap();
        let slot = |app: &AppState, i: usize| slot_of(app, menu.slots[i].as_ref().unwrap());
        assert!(slot(&app, 6).checked, "ペイント");
        assert!(!slot(&app, 0).checked);
        // ボーンが無ければポーズは押せず、理由を出す
        assert!(!slot(&app, 2).enabled);
        assert_eq!(
            slot(&app, 2).tooltip.as_deref(),
            Some(crate::mode::no_rig(app.lang))
        );
        assert_eq!(slot(&app, 6).icon, Some("paint_brush"));
        app.apply(Action::Pose(PoseAction::LoadFigure));
        assert!(slot(&app, 2).enabled);
        app.set_mode(EditorMode::Edit);
        assert!(slot(&app, 0).checked && !slot(&app, 6).checked);
        // 視点のパイは全部押せる。正投影の項目は、正投影の間だけ印が付く
        let view = app.pie.menu("view").cloned().unwrap();
        for item in view.slots.iter() {
            let s = slot_of(&app, item.as_ref().unwrap());
            assert!(
                s.enabled && s.tooltip.is_none() && !s.checked,
                "{}",
                s.label
            );
        }
        app.view3d.camera.set_orthographic(true);
        for (i, item) in view.slots.iter().enumerate() {
            let s = slot_of(&app, item.as_ref().unwrap());
            assert_eq!(s.checked, i == 3, "{}", s.label);
        }
    }

    #[test]
    fn opening_asks_for_the_pie_and_is_refused_while_drawing_or_for_an_unknown_pie() {
        let mut app = AppState::new(32, 32);
        app.apply(Action::Pie(PieAction::Open("mode".into())));
        assert_eq!(app.pie.request.as_deref(), Some("mode"));
        app.pie.request = None;
        app.apply(Action::Pie(PieAction::Open("nothing".into())));
        assert!(app.pie.request.is_none());
        assert_eq!(app.message, "パイメニュー「nothing」がありません。");
        let layer = app.selected_layer.unwrap();
        let settings = app.stroke_settings(false);
        let stroke = app.doc.begin_stroke(layer, &settings).unwrap();
        app.stroke = Some(stroke);
        app.canvas.stroke = Some(crate::state::StrokeSource::Mouse);
        app.apply(Action::Pie(PieAction::Open("mode".into())));
        assert!(app.pie.request.is_none());
        assert_eq!(app.message, crate::lang::refusals::during_stroke(app.lang));
    }

    #[test]
    fn running_items_of_each_kind() {
        let mut app = AppState::new(32, 32);
        // 操作
        run(&mut app, &PieItem::Command("mode.edit".into()));
        assert_eq!(app.mode, EditorMode::Edit);
        // 正投影の切り替え（3D のモデルがあるとき）
        app.view3d.load_demo();
        run(&mut app, &PieItem::Command("view3d.ortho".into()));
        assert!(app.view3d.camera.is_orthographic());
        run(&mut app, &PieItem::Command("view3d.ortho".into()));
        assert!(!app.view3d.camera.is_orthographic());
        run(&mut app, &PieItem::Command("no.such".into()));
        assert_eq!(app.message, "操作「no.such」がありません。");
        // 別のパイ: 開く頼み
        run(&mut app, &PieItem::Pie("view".into()));
        assert_eq!(app.pie.request.as_deref(), Some("view"));
        app.pie.request = None;
        // yolu-ops の命令: 全部を 1 回の取り消しで
        let layers = app.doc.layers().len();
        let json = json!([
            {"command": "layer.add", "args": {"kind": "paint", "name": "A"}},
            {"command": "layer.add", "args": {"kind": "paint", "name": "B"}},
        ])
        .to_string();
        assert_eq!(
            slot_of(&app, &PieItem::Ops(json.clone())).label,
            "layer.add +1"
        );
        run(&mut app, &PieItem::Ops(json));
        assert_eq!(app.doc.layers().len(), layers + 2, "{}", app.message);
        app.apply(Action::Undo);
        assert_eq!(app.doc.layers().len(), layers, "1 回の取り消しで両方");
        // 記録の間は断る
        app.apply(Action::Automation(
            crate::automation::AutomationOp::StartRecording,
        ));
        let json1 = json!({"command": "layer.add", "args": {"kind": "paint"}}).to_string();
        run(&mut app, &PieItem::Ops(json1));
        assert_eq!(app.message, "命令を実行できません（記録中です）。");
        assert_eq!(app.doc.layers().len(), layers);
        app.automation.recorder = None;
        // 読めない JSON・知らない命令は断る（文書は変えない）
        run(&mut app, &PieItem::Ops("{".into()));
        assert_eq!(app.message, "命令を実行できません（JSON を読めません）。");
        run(
            &mut app,
            &PieItem::Ops(json!({"command": "no.such"}).to_string()),
        );
        assert!(
            app.message.starts_with("命令を実行できません（"),
            "{}",
            app.message
        );
        assert_eq!(app.doc.layers().len(), layers);
        // 記録したアクション: 名前で探す（無ければ断る）
        let missing = PieItem::Recorded("無い".into());
        assert!(!slot_of(&app, &missing).enabled);
        run(&mut app, &missing);
        assert_eq!(app.message, "アクション「無い」がありません。");
    }

    #[test]
    fn a_recorded_action_item_plays_the_action_with_that_name() {
        let mut app = AppState::new(32, 32);
        let dir = std::env::temp_dir().join(format!("yolu-pie-actions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(crate::automation::attach(&mut app, dir.clone()).is_none());
        let commands = yolu_ops::action::parse_list(&[json!(
            {"command": "layer.add", "args": {"kind": "paint", "name": "From a pie"}}
        )])
        .unwrap();
        app.automation.store.add("塗る", commands).unwrap();
        let item = PieItem::Recorded("塗る".into());
        let slot = slot_of(&app, &item);
        assert!(slot.enabled && slot.label == "塗る" && slot.icon == Some("play"));
        let layers = app.doc.layers().len();
        run(&mut app, &item);
        assert_eq!(app.doc.layers().len(), layers + 1, "{}", app.message);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
