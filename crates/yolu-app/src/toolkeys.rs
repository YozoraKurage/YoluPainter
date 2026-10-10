//! ツールのキーの動き方と、キーを押している間だけの切り替え。
//!
//! ツールのキー（`tool.<ツールの id>` の割り当て）は、キーごとに動き方を選べる（ショートカットの設定。keymap.json の `tool_keys`）。
//! - `Tap`: 押すと切り替える（既定。今までの動き）。
//! - `Hold`: 押している間だけそのツール。離すと、押す前のツールに戻る。
//! - `TapOrHold`: 短く押して離したら、そのツールに切り替えたまま。長押しにして離す（`HOLD_AFTER_SECS` より長い）か、押している間に描いたら、
//!   押す前のツールに戻る。
//!
//! 切り替えは押した瞬間（`press`）で、離しの判定は毎フレーム（`update`）。ツールを替える口は `AppState::switch_to` 1 つのまま使い、押す前の
//! ツール（ツールの列のどれか・今のブラシも）を覚えて戻す。
//! - 一時の切り替えは積み重ねる。押している間に別の「押している間だけ」のキーを押すと、そのとき使っているツールが戻る先になる。上の段を離すと
//!   下の段のツールへ戻り、下の段を先に離したときは戻さず、上の段の戻る先を下の段の戻る先へ付け替える（上の段を離すと、最初のツールへ戻る）。
//! - 描いている最中に離したら、描き終えてから戻す（線・選択の形・多角形の途中を切らない）。同じ線のうちに同じツールのキーを押し直したら、続ける。
//! - 次のときは、戻す前提を捨てる（その手段で選んだツールが残る）: 押している間に別の手段（ツールバー・別のキー・パイ・ブラシの一覧）でツールを
//!   選んだ。モードが替わる直前は、離したことにして戻してから替える。フォーカスを失った・ポップアップ・ダイアログ・文字の入力が開いたときも、離したことにする。
//! - 押した時刻は、アプリ共通の時計（メインウィンドウの時刻。外へ出したウィンドウの時刻は、ウィンドウごとに始まりが違う）で持つ。

use egui::{Event, Key};

use crate::brushes::BrushKey;
use crate::commands;
use crate::keymap::Fired;
use crate::lang::Lang;
use crate::state::{Action, AppState, Tool};
use crate::toolset::{SlotId, ToolsetAction};

/// 長押しと数える時間（秒）。これより長く押して離したら、`TapOrHold` のキーも前のツールに戻る。
pub const HOLD_AFTER_SECS: f64 = 0.25;

/// ツールのキーの動き方。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ToolKeyMode {
    /// 押すと切り替え（既定）。
    #[default]
    Tap,
    /// 押している間だけ。
    Hold,
    /// 短く押すと切り替え、長押しで押している間だけ。
    TapOrHold,
}

impl ToolKeyMode {
    pub const ALL: [ToolKeyMode; 3] = [Self::Tap, Self::Hold, Self::TapOrHold];

    /// 設定のファイル（keymap.json の `tool_keys`）の値。
    pub fn key(self) -> &'static str {
        match self {
            Self::Tap => "tap",
            Self::Hold => "hold",
            Self::TapOrHold => "tap_or_hold",
        }
    }

    pub fn from_key(key: &str) -> Option<ToolKeyMode> {
        Self::ALL.into_iter().find(|m| m.key() == key)
    }

    /// 選ぶ一覧の名前。
    pub fn label(self, lang: Lang) -> &'static str {
        match self {
            Self::Tap => lang.pick("押すと切り替え", "Switch on Press"),
            Self::Hold => lang.pick("押している間だけ", "Only While Held"),
            Self::TapOrHold => lang.pick(
                "短く押すと切り替え・長押しで押している間だけ",
                "Tap to Switch, Hold for Temporary",
            ),
        }
    }

    /// 設定の表の欄に出す短い名前（長い名前は、欄に入りきらない）。
    pub fn short_label(self, lang: Lang) -> &'static str {
        match self {
            Self::TapOrHold => lang.pick("短押し／長押し", "Tap / Hold"),
            _ => self.label(lang),
        }
    }
}

/// ツールの場所（ツール・ツールの列のどれか・今のブラシ）。戻る先と、切り替えたあとの場所に使う。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolPoint {
    pub tool: Tool,
    pub slot: Option<SlotId>,
    pub brush: BrushKey,
}

impl ToolPoint {
    pub fn of(app: &AppState) -> ToolPoint {
        ToolPoint {
            tool: app.tool,
            slot: app.toolset.set.active(),
            brush: app.brushes.lib.current(),
        }
    }
}

/// 一時の切り替えの 1 段（ツールのキーを 1 つ押している間）。
#[derive(Debug)]
struct Level {
    /// 押しているキー（離しを見る）。
    key: Key,
    mode: ToolKeyMode,
    /// 押す前の場所（離したとき戻る先）。
    back: ToolPoint,
    /// 切り替えた直後の場所（これと違えば、別の手段で選ばれた）。
    to: ToolPoint,
    /// 切り替えた直後の `AppState::tool_epoch`（同じツールを選び直されたときも見つける）。
    epoch: u64,
    /// 押した時刻（アプリ共通の時計。秒）。
    started: f64,
    /// 押した時点の文書の版（押している間に文書が変わったか）。
    revision: u64,
    /// 押している間にツールで描いた・ドラッグした。
    drew: bool,
    /// 離した後なら、押していた秒数（描いている途中なら、描き終えるのを待っている）。
    held: Option<f64>,
}

/// キーを押している間の一時の切り替えの状態（`AppState::temp_tool`）。保存しない。
#[derive(Debug, Default)]
pub struct TempTool {
    /// 積み重ね（先頭が先に押したキー、最後が今のツールの切り替え）。
    levels: Vec<Level>,
    /// 最後に見たアプリ共通の時計（秒）。モードの切り替えのように、時刻を渡せない所が、離しの長さを出すのに使う。
    clock: f64,
}

impl TempTool {
    /// 一時の切り替えの間か。
    pub fn is_active(&self) -> bool {
        !self.levels.is_empty()
    }

    /// 積み重ねの段の数。
    pub fn depth(&self) -> usize {
        self.levels.len()
    }

    /// 今の段を離したときに戻る先（一時の切り替えの間だけ）。
    pub fn returns_to(&self) -> Option<ToolPoint> {
        self.levels.last().map(|l| l.back)
    }

    /// 一時の切り替えを捨てる（今のツールのまま、戻さない）。
    pub fn forget(&mut self) {
        self.levels.clear();
    }
}

impl AppState {
    /// 離したときに戻る先の、ツールの列の項目（ツールバーの印を付ける所）。列に無いツールなら、そのツールの最初の項目。
    pub fn temp_tool_return_slot(&self) -> Option<SlotId> {
        let back = self.temp_tool.returns_to()?;
        let set = &self.toolset.set;
        back.slot
            .filter(|s| set.slot(*s).is_some())
            .or_else(|| set.first_of(back.tool))
    }
}

/// アプリ共通の時計（秒）。外へ出したウィンドウは、ウィンドウごとに時刻の始まりが違うので、いつもメインウィンドウの時刻を見る。
fn clock(ctx: &egui::Context) -> f64 {
    ctx.input_for(egui::ViewportId::ROOT, |i| i.time)
}

/// 一番上の段が切り替えたあとに、別の手段でツールが選ばれた・モードが替わったか。
fn stale(app: &AppState) -> bool {
    app.temp_tool.levels.last().is_some_and(|top| {
        app.tool_epoch != top.epoch || ToolPoint::of(app) != top.to || !app.mode.paints()
    })
}

/// 押しの事象 1 つを実行する。動き方が `Hold`/`TapOrHold` のツールのキーなら、一時の切り替えを始める。
pub fn press(ctx: &egui::Context, app: &mut AppState, fired: Fired) {
    // 離した後の残りを塗っている 3D のストローク（確定待ち）は、先に確定する（離した直後の「押している間だけ」の切り替えも効かせる）
    crate::view3d::input::settle(app);
    let now = clock(ctx);
    app.temp_tool.clock = now;
    let (Some(tool), Some(key)) = (commands::selected_tool(fired.command), fired.key) else {
        app.apply(fired.action);
        return;
    };
    // 同じフレームの先の操作で、別の手段でツールが替わっていたら、積み重ねを捨てる
    if stale(app) {
        app.temp_tool.levels.clear();
    }
    let slot = app.toolset.set.slot_for_tool(tool);
    if tool == app.tool && slot == app.toolset.set.active() {
        // もうそのツール: 何も替えない（`apply` すると、ツールを選び直した印が付いて、戻す前提を捨ててしまう）。
        // キーを離して描き終えるのを待っている間の押し直しなら、押し直しとして続ける
        let here = ToolPoint::of(app);
        if let Some(top) = app.temp_tool.levels.last_mut() {
            if top.held.is_some() && top.to == here {
                top.key = key;
                top.mode = fired.mode;
                top.started = now;
                top.held = None;
            }
        }
        return;
    }
    // 描いている最中は、ツールを替えない（ブラシの切り替えは断る）。一時の切り替えも始めず、押すと切り替えと同じ扱い
    if drawing(app) {
        app.apply(fired.action);
        return;
    }
    // 積み重ね: 今使っているツールが戻る先
    let back = ToolPoint::of(app);
    app.apply(fired.action);
    if app.tool != tool {
        return;
    }
    app.temp_tool.levels.push(Level {
        key,
        mode: fired.mode,
        back,
        to: ToolPoint::of(app),
        epoch: app.tool_epoch,
        started: now,
        revision: app.doc.revision(),
        drew: false,
        held: None,
    });
}

/// 一時の切り替えの間、押しているキーの繰り返しの押しを取り除く（修飾キーを先に離しても、キーの表の別の割り当てに当たらない）。
pub fn swallow_repeats(i: &mut egui::InputState, app: &AppState) {
    if app.temp_tool.levels.is_empty() {
        return;
    }
    let keys: Vec<Key> = app.temp_tool.levels.iter().map(|l| l.key).collect();
    i.events.retain(|e| {
        !matches!(
            e,
            Event::Key { key, pressed: true, repeat: true, .. } if keys.contains(key)
        )
    });
}

/// フレームごとに、キーを離したか・戻す条件が揃ったかを見る（`shell::handle_shortcuts` の初めと、押しを実行した後）。
pub fn update(ctx: &egui::Context, app: &mut AppState) {
    if app.temp_tool.levels.is_empty() {
        return;
    }
    let now = clock(ctx);
    app.temp_tool.clock = now;
    // 別の手段でツールを選ばれた・モードが替わった: 戻すのをやめる（選んだツールが残る）
    if stale(app) {
        app.temp_tool.levels.clear();
        return;
    }
    let focused = ctx.input(|i| i.focused);
    // ポップアップ・ダイアログ・文字の入力が開いたら、離したことにする（そのあとのキーの離しは、表へ届かない）
    let blocked = ctx.egui_wants_keyboard_input()
        || app.popup.is_some()
        || app.sel.dialog.is_some()
        || crate::windows::modal_open(app);
    let mut levels = std::mem::take(&mut app.temp_tool.levels);
    for level in &mut levels {
        if used(app, level.revision) {
            level.drew = true;
        }
        if level.held.is_none() && (blocked || !focused || !ctx.input(|i| i.key_down(level.key))) {
            level.held = Some((now - level.started).max(0.0));
        }
    }
    settle(app, &mut levels, true);
    app.temp_tool.levels = levels;
}

/// モードを替える直前（まだペイントのうち）に呼ぶ: 一時の切り替えを、離したものとして済ませる（描いている途中の形は、モードを替えるときに
/// 確定か取りやめになるので、待たない）。
pub fn leave_paint(app: &mut AppState) {
    if app.temp_tool.levels.is_empty() {
        return;
    }
    if stale(app) {
        app.temp_tool.levels.clear();
        return;
    }
    let now = app.temp_tool.clock;
    let mut levels = std::mem::take(&mut app.temp_tool.levels);
    for level in &mut levels {
        if used(app, level.revision) {
            level.drew = true;
        }
        if level.held.is_none() {
            level.held = Some((now - level.started).max(0.0));
        }
    }
    settle(app, &mut levels, false);
}

/// その段を離したとき、押す前のツールへ戻すか（`TapOrHold` は、短く押して描かなかったら切り替えのまま）。
fn goes_back(level: &Level) -> bool {
    match level.mode {
        ToolKeyMode::Hold => true,
        ToolKeyMode::TapOrHold => level.drew || level.held.unwrap_or(0.0) > HOLD_AFTER_SECS,
        ToolKeyMode::Tap => false,
    }
}

/// 離された段を片づける。`wait` なら、描いている最中の一番上の段は、描き終えるまで残す。
/// - 下の段が離された: ツールは替えない。戻すなら、上の段の戻る先を、その段の戻る先へ付け替える。切り替えのまま残すなら、その段より下の段は、
///   そのツールに上書きされたものとして捨てる。
/// - 一番上の段が離された: 戻すなら戻る先へ替え（すぐ下の段が続く）、切り替えのまま残すなら、下の段も捨てる。
fn settle(app: &mut AppState, levels: &mut Vec<Level>, wait: bool) {
    let mut i = levels.len().saturating_sub(1);
    while i > 0 {
        i -= 1;
        if levels[i].held.is_none() {
            continue;
        }
        let gone = levels.remove(i);
        if goes_back(&gone) {
            levels[i].back = gone.back;
        } else {
            levels.drain(..i);
            break;
        }
    }
    while levels.last().is_some_and(|top| top.held.is_some()) {
        if wait && drawing(app) {
            break;
        }
        let top = levels.pop().expect("一番上の段");
        if !goes_back(&top) {
            levels.clear();
            break;
        }
        restore(app, top.back);
        // 戻したのは自分なので、下の段は、今の場所を切り替えた直後の場所にする
        if let Some(next) = levels.last_mut() {
            next.to = ToolPoint::of(app);
            next.epoch = app.tool_epoch;
        }
    }
}

/// ツールで描いている・ドラッグしている最中か（ストローク、キャンバスのツールのドラッグ（移動と変形・図形・グラデーション・パスと点の矩形・
/// 選択の形・テキストの押し）、3D ビューのグラデーション・図形・定規・選択の形のドラッグ、3D のパスの矩形、多角形選択の途中（2D も 3D も））。戻すのは、これが
/// 終わってから（ツールを替えると、途中の形を捨てるため）。
/// ポインタを押している印（`canvas.tool_button` など）は、フォーカスを失ったあとに残りうるので、待つ条件には使わない。
fn drawing(app: &AppState) -> bool {
    app.is_stroking()
        || crate::tools::input::CanvasKind::ALL
            .iter()
            .any(|kind| kind.handler().dragging(app, None))
        || crate::view3d::draft::dragging(app)
        || crate::view3d::select::dragging(app)
        || app.path.rect.is_some()
        || app.sel.pen.is_some()
        || app.sel_has_polygon_point()
}

/// 押している間に、ツールで描いた・ドラッグしたか（描いている最中・描き終えて文書が変わった・ツールの押しが始まった）。
fn used(app: &AppState, revision: u64) -> bool {
    drawing(app)
        || app.doc.revision() != revision
        || app.canvas.tool_button.is_some()
        || app.canvas.pen_press.is_some()
        || app.view3d.input.pen_press.is_some()
}

/// 戻る先のツールへ替える（ツールバーで選ぶのと同じ道。列から消えていたら、そのツールの最初の項目）。
fn restore(app: &mut AppState, back: ToolPoint) {
    let slot = back.slot.filter(|s| {
        app.toolset
            .set
            .slot(*s)
            .is_some_and(|slot| slot.tool == back.tool)
    });
    match slot {
        Some(slot) => app.apply(Action::Tools(ToolsetAction::Select(slot))),
        None => app.apply(Action::SelectTool(back.tool)),
    }
}

#[cfg(test)]
mod tests;
