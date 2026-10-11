//! 色のウィンドウ: 色の値を決める欄（塗りつぶしレイヤーの値・lilToon の色・グラデーションの分岐点の色・ブラシのエミッション・UV ワイヤーフレームの色・部品の ID の色など）を
//! 押すと出る、動かせるウィンドウ。中身は色相の円とその中の四角（カラーのパネルと同じ見た目と操作）・16 進の欄（相手がアルファを持てばアルファも）・
//! 描画色を入れるボタン・今のカラーセットの色の列。
//!
//! - ウィンドウは 1 つだけ。開いている間は、押した欄が「相手」で、欄には `t::ACCENT` の枠が出る（マスクを選んでいる枠と同じ形）。別の色の欄を押すと、
//!   ウィンドウはそのままで相手が替わる。
//! - 見出しの帯をドラッグして動かす。閉じるボタンか Esc で閉じる。外を押しても閉じない（ほかの欄を触りながら色を変えられる）。位置はこの
//!   セッションの間覚え、次に開くときは前の位置（画面の外なら中へ戻す）。初めて開くときは、欄の列の左（入らなければ列の右、
//!   どちらにも入らなければ欄の下か上）。
//! - 色を変えると、相手の欄が毎フレーム [`take`]（[`field`] が呼ぶ）で受け取って値へ当てる。円・四角・アルファのドラッグ 1 回、16 進の 1 回の
//!   入力、ボタン・色の列の 1 回の押しが、それぞれ 1 回の取り消しになるよう、[`Update`] が「前の変更とまとめてよいか」と「1 回の操作の終わり」を持つ。
//! - Esc は、開いたとき（相手が替わったとき）の色へ戻して閉じる。ただし、ほかの物が使った Esc は使わない: 欄の文字（16 進の欄など）を
//!   打っている途中の Esc はその入力をやめるだけで、ウィンドウも相手の値もそのまま。ウィンドウより先に描く部品が使った Esc（塗りつぶしの仕事の取消・
//!   色の名前の変更をやめる）も同じ。色のほかに状態を持つ相手（部品の ID の色の「自動」）は、[`Update::reverted`] を見て開いたときの
//!   状態へ戻す。
//! - 相手の欄が描かれなくなった（レイヤーを消した・セットを替えた・欄が隠れた）ら閉じる。
//! - 描画色（`ColorState`）は動かさない（描画色は「入れる」ときの元に使うだけ）。
//! - 色相は選びの途中で覚え、彩度や明度が 0 になっても失わない。外から値が変わった（Undo・スポイト）ときは、選びの表示を合わせる。
//!
//! ウィンドウはフレームの終わりに 1 回描く（[`show`]。アプリの中では [`show_in_app`]）。相手の欄が変更を受け取るのは次のフレーム。

use std::sync::{Arc, Mutex};

use egui::{pos2, vec2, Color32, Context, Id, Order, Pos2, Rect, Sense, Ui, Vec2};

use super::color::{hue_at, in_ring, wheel_square, ColorTextures, RING_THICKNESS};
use crate::colorsets::Palette;
use crate::lang::Lang;
use crate::state::{hsv_to_rgb, parse_hex, rgb_to_hsv, to_hex, AppState};
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, NumberFormat, SliderSpec};
use crate::ui::window::HEADER_HEIGHT;

/// ウィンドウの幅。
pub const WIDTH: f32 = 204.0;
const PAD: f32 = 8.0;
const WHEEL: f32 = WIDTH - 2.0 * PAD;
const ROW: f32 = 22.0;
const GAP: f32 = 6.0;
/// カラーセットの色の 1 マスと間。
const CELL: f32 = 16.0;
const CELL_GAP: f32 = 2.0;
/// カラーセットの色を見せる行の上限（多ければホイールで送る）。
const MAX_SET_ROWS: usize = 4;
/// カラーセットの名前の行の高さ。
const SET_NAME: f32 = 16.0;
/// 16 進とアルファを 1 行に並べるときの 16 進の幅の割合（カラーのパネルと同じ）。
const HEX_SHARE: f32 = 0.52;
/// 相手の欄が描かれなくなってから閉じるまでのフレーム数。
const GRACE: u64 = 2;
/// ウィンドウと画面の端の余白。
const SCREEN_MARGIN: f32 = 4.0;

/// 欄の色（0〜255 の RGB と、相手が持っていればアルファ）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pick {
    pub rgb: [u8; 3],
    /// 相手がアルファを持たなければ None（ウィンドウにアルファの欄を出さない）。
    pub alpha: Option<u8>,
}

impl Pick {
    /// アルファを持たない色。
    pub fn rgb(rgb: [u8; 3]) -> Pick {
        Pick { rgb, alpha: None }
    }

    /// 0〜1 の RGBA から。`alpha` が false ならアルファを持たない。
    pub fn from_floats(c: [f32; 4], alpha: bool) -> Pick {
        Pick {
            rgb: [w::to_byte(c[0]), w::to_byte(c[1]), w::to_byte(c[2])],
            alpha: alpha.then(|| w::to_byte(c[3])),
        }
    }

    /// 0〜1 の RGBA（アルファを持たなければ 1）。
    pub fn floats(self) -> [f32; 4] {
        [
            f32::from(self.rgb[0]) / 255.0,
            f32::from(self.rgb[1]) / 255.0,
            f32::from(self.rgb[2]) / 255.0,
            f32::from(self.alpha.unwrap_or(255)) / 255.0,
        ]
    }

    /// 0〜1 の RGB。
    pub fn rgb_floats(self) -> [f32; 3] {
        let c = self.floats();
        [c[0], c[1], c[2]]
    }
}

/// ウィンドウから相手の欄への変更（相手は値へ当てる）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Update {
    pub pick: Pick,
    /// 円・四角・アルファのドラッグの途中か、その終わり。相手は前の変更とまとめてよい（取り消しの 1 回にまとめる）。
    pub dragging: bool,
    /// 1 回の操作の終わり（ドラッグを離した・16 進を決めた・ボタン・色の列・Esc）。相手はまとめを切る。
    pub done: bool,
    /// Esc で開いたとき（相手が替わったとき）の色へ戻した。`pick` はそのときの色。色のほかに「自動」のような状態を持つ相手は、開いた
    /// ときの状態へ戻す（`pick` が今と同じでも来る）。
    pub reverted: bool,
}

#[derive(Clone)]
struct State {
    target: Id,
    name: String,
    /// 押した欄（初めて開くときに、そのそばへ置く）。
    anchor: Rect,
    /// 欄の列（初めて開くときに、入ればその左か右へ置く）。
    column: Rect,
    /// 開いた（相手が替わった）ときの色。Esc で戻す。
    original: Pick,
    /// 今出している色。
    shown: Pick,
    hue: f32,
    sat: f32,
    val: f32,
    /// 相手の欄が最後に描かれたフレーム。
    seen: u64,
    /// 円・四角・アルファをドラッグしている間 true。
    dragging: bool,
    /// 開いて（相手が替わって）から、相手へ変更を出したか（Esc で、色が開いたときと同じでも戻しを出す）。
    touched: bool,
    /// 相手を替える前の相手とその色、替えたフレーム（替えた押しで 16 進の欄の入力が決まったときは、前の相手へ当てる）。
    prior: Option<(Id, Pick)>,
    switched: u64,
}

impl State {
    /// 外から変わった色に、選びの表示を合わせる（灰色では色相を残し、黒では彩度も残す）。
    fn sync(&mut self, pick: Pick) {
        let (h, s, v) = hsv(pick.rgb);
        if s > 1e-4 && v > 1e-4 {
            self.hue = h;
        }
        if v > 1e-4 {
            self.sat = s;
        }
        self.val = v;
        self.shown = pick;
    }
}

/// ウィンドウから出た、相手がまだ受け取っていない変更（相手・変更・出したフレーム）。
type Pending = Vec<(Id, Update, u64)>;

fn state_id() -> Id {
    Id::new("yolu.color-window")
}

fn pos_id() -> Id {
    state_id().with("pos")
}

fn rect_id() -> Id {
    state_id().with("rect")
}

fn pending_id() -> Id {
    state_id().with("pending")
}

/// どれかの欄がフォーカス（文字の入力）を持っていたかを、ウィンドウを描いた最後のフレームの番号つきで覚える場所。
fn focus_id() -> Id {
    state_id().with("focus")
}

fn hsv(rgb: [u8; 3]) -> (f32, f32, f32) {
    rgb_to_hsv(
        f32::from(rgb[0]) / 255.0,
        f32::from(rgb[1]) / 255.0,
        f32::from(rgb[2]) / 255.0,
    )
}

fn bytes(rgb: (f32, f32, f32)) -> [u8; 3] {
    [w::to_byte(rgb.0), w::to_byte(rgb.1), w::to_byte(rgb.2)]
}

fn textures(ctx: &Context) -> Arc<Mutex<ColorTextures>> {
    let key = state_id().with("textures");
    if let Some(found) = ctx.data(|d| d.get_temp::<Arc<Mutex<ColorTextures>>>(key)) {
        return found;
    }
    let made = Arc::new(Mutex::new(ColorTextures::default()));
    ctx.data_mut(|d| d.insert_temp(key, made.clone()));
    made
}

/// ウィンドウを `target` の欄へ向けて開く（開いていれば、ウィンドウはそのままで相手を替える）。`name` は見出しに出す欄の名前、`anchor` は押した欄、
/// `column` は欄の列（初めて開くときに、入ればその左か右へ置く）、`current` は欄の今の色。同じ相手で呼び直しても何もしない（開いたときの色を残す）。
pub fn open(ctx: &Context, target: Id, name: &str, anchor: Rect, column: Rect, current: Pick) {
    let frame = ctx.cumulative_frame_nr();
    let before = ctx.data(|d| d.get_temp::<State>(state_id()));
    if before.as_ref().is_some_and(|s| s.target == target) {
        return;
    }
    let prior = before.as_ref().map(|s| (s.target, s.shown));
    let (mut hue, sat, val) = hsv(current.rgb);
    // 灰色の色へ替えたときは、前の色相を残す（円の印が赤へ飛ばない）
    if sat <= 1e-4 || val <= 1e-4 {
        if let Some(s) = &before {
            hue = s.hue;
        }
    }
    let state = State {
        target,
        name: name.to_owned(),
        anchor,
        column,
        original: current,
        shown: current,
        hue,
        sat,
        val,
        seen: frame,
        dragging: false,
        touched: false,
        prior,
        switched: frame,
    };
    ctx.data_mut(|d| d.insert_temp(state_id(), state));
    ctx.request_repaint();
}

/// ウィンドウを閉じる（今の色のまま）。
pub fn close(ctx: &Context) {
    ctx.data_mut(|d| d.remove::<State>(state_id()));
}

pub fn is_open(ctx: &Context) -> bool {
    target(ctx).is_some()
}

/// ウィンドウの相手（開いていれば）。
pub fn target(ctx: &Context) -> Option<Id> {
    ctx.data(|d| d.get_temp::<State>(state_id()))
        .map(|s| s.target)
}

/// `id` の欄がウィンドウの相手か（欄に印を出す）。
pub fn is_target(ctx: &Context, id: Id) -> bool {
    target(ctx) == Some(id)
}

/// `id` の欄の色を、ウィンドウの円・四角・アルファでドラッグしている間 true（設定のファイルへ書くのを離すまで待つ欄が読む）。
pub fn dragging(ctx: &Context, id: Id) -> bool {
    ctx.data(|d| d.get_temp::<State>(state_id()))
        .is_some_and(|s| s.target == id && s.dragging)
}

/// 最後に描いたウィンドウの場所（開いていれば。試験が中の部品の場所を知るために読む）。
pub fn rect(ctx: &Context) -> Option<Rect> {
    if !is_open(ctx) {
        return None;
    }
    ctx.data(|d| d.get_temp::<Rect>(rect_id()))
}

/// 相手の欄が毎フレーム呼ぶ: 欄がまだあることをウィンドウに知らせ、外から変わった色（`current`）にウィンドウの表示を合わせ、ウィンドウからの変更があれば返す。
/// 変更を当てたあとの色は、次のフレームの `current` で知らせる。
pub fn take(ctx: &Context, id: Id, current: Pick) -> Option<Update> {
    let frame = ctx.cumulative_frame_nr();
    let pending = ctx.data_mut(|d| {
        let list = d.get_temp_mut_or_default::<Pending>(pending_id());
        list.iter()
            .position(|(target, _, _)| *target == id)
            .map(|i| list.remove(i).1)
    });
    if let Some(mut state) = ctx.data(|d| d.get_temp::<State>(state_id())) {
        if state.target == id {
            state.seen = frame;
            // 受け取る変更がある間とドラッグの間は、まだ当たる前の色なので合わせない
            if pending.is_none() && !state.dragging && current != state.shown {
                state.sync(current);
            }
            ctx.data_mut(|d| d.insert_temp(state_id(), state));
        }
    }
    pending
}

/// 色の値の欄（色の見本）。押すと、ウィンドウをこの欄へ向けて開く（開いていれば相手を替える）。ウィンドウの相手なら印を出す。ウィンドウからの変更があれば返す
/// （呼び手は値へ当てる。`Update` の `dragging`・`done` で取り消しをまとめる）。`target` は欄の値ごとに違う名前（レイヤー・チャンネル・文書などを含める。
/// 同じ名前のまま別の値を指すと、ウィンドウが別の値へ前の値の変更を当ててしまう）。`enabled` が false の間（描いている間・読むだけのセット）は押しても
/// 開かず、ウィンドウからの変更は捨てる（ウィンドウは開いたままで、次のフレームに欄の値へ表示を戻す）。
#[allow(clippy::too_many_arguments)]
pub fn field(
    ui: &mut Ui,
    rect: Rect,
    target: Id,
    name: &str,
    current: Pick,
    tooltip: &str,
    enabled: bool,
) -> Option<Update> {
    let ctx = ui.ctx().clone();
    let update = take(&ctx, target, current).filter(|_| enabled);
    let response = w::color_swatch(ui, rect, target, current.floats(), tooltip, enabled);
    if is_target(&ctx, target) {
        w::outline(ui.painter(), rect.expand(2.0), t::ACCENT, 2.0, 3.0);
    }
    if enabled && response.clicked() {
        open(&ctx, target, name, rect, ui.clip_rect(), current);
    }
    update
}

/// ウィンドウを初めて置く場所。欄の列 `column` の左に入ればそこ、入らなければ列の右（どちらも欄の操作を隠さない）に、`anchor` の高さのあたりへ。
/// どちらにも入らなければ `anchor` の下（入らなければ上）へ、欄に重ねずに。どれも画面の中に収める。
pub fn place(anchor: Rect, column: Rect, size: Vec2, screen: Rect) -> Rect {
    let gap = 8.0;
    let area = screen.shrink(SCREEN_MARGIN);
    let top = (anchor.center().y - size.y * 0.5)
        .clamp(area.top(), (area.bottom() - size.y).max(area.top()));
    let left = column.left() - size.x - gap;
    if left >= area.left() {
        return Rect::from_min_size(pos2(left, top), size);
    }
    let right = column.right() + gap;
    if right + size.x <= area.right() {
        return Rect::from_min_size(pos2(right, top), size);
    }
    let below = anchor.bottom() + gap;
    let y = if below + size.y <= area.bottom() || anchor.top() - gap - size.y < area.top() {
        below
    } else {
        anchor.top() - gap - size.y
    };
    Rect::from_min_size(clamp_into(pos2(anchor.left(), y), size, screen), size)
}

/// ウィンドウの左上を、ウィンドウが画面に収まる所へ寄せる（収まらないほど画面が小さければ左上を合わせる）。
pub fn clamp_into(min: Pos2, size: Vec2, screen: Rect) -> Pos2 {
    let area = screen.shrink(SCREEN_MARGIN);
    let x = min
        .x
        .clamp(area.left(), (area.right() - size.x).max(area.left()));
    let y = min
        .y
        .clamp(area.top(), (area.bottom() - size.y).max(area.top()));
    pos2(x, y)
}

/// 円の外接の正方形（ウィンドウの場所から）。
pub fn wheel_of(window: Rect) -> Rect {
    Rect::from_min_size(
        pos2(window.left() + PAD, window.top() + HEADER_HEIGHT + PAD),
        vec2(WHEEL, WHEEL),
    )
}

/// 16 進とアルファの行（ウィンドウの場所から）。
fn value_row(window: Rect) -> Rect {
    Rect::from_min_size(
        pos2(window.left() + PAD, wheel_of(window).bottom() + GAP),
        vec2(WHEEL, ROW),
    )
}

/// 16 進の欄の場所（ウィンドウの場所と、相手がアルファを持つか）。
pub fn hex_of(window: Rect, alpha: bool) -> Rect {
    let row = value_row(window);
    if alpha {
        Rect::from_min_size(row.min, vec2((row.width() * HEX_SHARE).round(), ROW))
    } else {
        row
    }
}

/// アルファのスライダーの場所。
fn alpha_of(window: Rect) -> Rect {
    let row = value_row(window);
    let hex = hex_of(window, true);
    Rect::from_min_max(pos2(hex.right() + GAP, row.top()), row.max)
}

/// 描画色を入れるボタンの場所。
pub fn paint_of(window: Rect) -> Rect {
    Rect::from_min_size(
        pos2(window.left() + PAD, value_row(window).bottom() + GAP),
        vec2(WHEEL, ROW),
    )
}

/// カラーセットの色の列の行数（見せる行の上限まで）と、全部の行数。
fn set_rows(count: usize) -> (usize, usize) {
    let columns = columns(WHEEL);
    let rows = count.div_ceil(columns);
    (rows.min(MAX_SET_ROWS), rows)
}

fn columns(width: f32) -> usize {
    (((width + CELL_GAP) / (CELL + CELL_GAP)).floor() as usize).max(1)
}

fn grid_height(rows: usize) -> f32 {
    if rows == 0 {
        0.0
    } else {
        rows as f32 * (CELL + CELL_GAP) - CELL_GAP
    }
}

/// ウィンドウの大きさ（カラーセットの色の数で高さが変わる）。
fn size_for(colors: usize) -> Vec2 {
    let (shown, _) = set_rows(colors);
    let set = if shown == 0 {
        0.0
    } else {
        GAP + SET_NAME + grid_height(shown)
    };
    vec2(
        WIDTH,
        HEADER_HEIGHT + PAD + WHEEL + GAP + ROW + GAP + ROW + set + PAD,
    )
}

/// ウィンドウが描くときに読む物（描画色・今のカラーセット）。
pub struct Sources<'a> {
    pub lang: Lang,
    /// 描画色（0〜1 の RGBA。「描画色を入れる」の元）。
    pub main: [f32; 4],
    /// 今のカラーセット（色の列）。
    pub set: Option<&'a Palette>,
}

fn marker(p: &egui::Painter, at: Pos2, radius: f32) {
    p.circle_stroke(at, radius, egui::Stroke::new(2.0, Color32::BLACK));
    p.circle_stroke(at, radius - 1.0, egui::Stroke::new(1.5, Color32::WHITE));
}

fn push(ctx: &Context, target: Id, update: Update) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| {
        let list = d.get_temp_mut_or_default::<Pending>(pending_id());
        // まだ受け取られていない同じ相手への変更は、1 つにまとめる（色と戻しの印は新しいほう、終わりの印は残す）
        if let Some(slot) = list.iter_mut().find(|(id, _, _)| *id == target) {
            slot.1 = Update {
                pick: update.pick,
                dragging: update.dragging,
                done: slot.1.done || update.done,
                reverted: update.reverted,
            };
            slot.2 = frame;
        } else {
            list.push((target, update, frame));
        }
    });
    ctx.request_repaint();
}

/// アプリの中のウィンドウ（描画色と今のカラーセットを読む。16 進が読めなければ知らせる）。フレームの終わりに 1 回呼ぶ。
pub fn show_in_app(ctx: &Context, app: &mut AppState) {
    // 確認のウィンドウ（モーダル）の間は描かない（そのウィンドウより上に出て、下の欄を変えられないように）
    if crate::windows::modal_open(app) {
        if is_open(ctx) {
            // 欄はモーダルの下で描かれ続けるので、開いたまま待つ
            ctx.request_repaint();
        }
        return;
    }
    let failure = {
        let sources = Sources {
            lang: app.lang,
            main: app.color.main,
            set: Some(app.colorsets.palette()),
        };
        show(ctx, &sources)
    };
    if let Some(message) = failure {
        app.fail(crate::notice::Source::Color, message);
    }
}

/// ウィンドウを描く（開いていなければ何もしない）。フレームの終わりに 1 回呼ぶ。16 進の欄に読めない文字を決めたときは、知らせの文を返す。
pub fn show(ctx: &Context, sources: &Sources<'_>) -> Option<String> {
    let frame = ctx.cumulative_frame_nr();
    // 受け取られないまま古くなった変更（相手が描かれなくなった）を捨てる
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Pending>(pending_id())
            .retain(|(_, _, at)| frame <= at + GRACE + 1)
    });
    let mut state = ctx.data(|d| d.get_temp::<State>(state_id()))?;
    if frame > state.seen + GRACE {
        close(ctx);
        return None;
    }
    if frame > state.seen {
        // 相手の欄が描かれたかを、次のフレームでも確かめる（描かれなければ閉じる）
        ctx.request_repaint();
    }
    let lang = sources.lang;
    let colors: &[crate::colorsets::Swatch] = sources.set.map_or(&[], |s| &s.colors);
    let size = size_for(colors.len());
    let screen = ctx.content_rect();
    let remembered = ctx.data(|d| d.get_temp::<Pos2>(pos_id()));
    let mut min = remembered.unwrap_or_else(|| place(state.anchor, state.column, size, screen).min);
    min = clamp_into(min, size, screen);
    let window = Rect::from_min_size(min, size);
    let tex = textures(ctx);
    let target = state.target;
    let mut updates: Vec<(Id, Update)> = Vec::new();
    let mut failure = None;
    let mut closed = false;
    let mut moved = Vec2::ZERO;
    let alpha = state.shown.alpha.is_some();
    egui::Area::new(state_id().with("area"))
        .order(Order::Foreground)
        .fixed_pos(window.min)
        .constrain(false)
        .show(ctx, |ui| {
            // ウィンドウの上の押下は下へ通さない
            ui.allocate_exact_size(window.size(), Sense::click_and_drag());
            let p = ui.painter().clone();
            for i in (1..=6).rev() {
                let f = i as f32;
                p.rect_filled(
                    window.expand(f).translate(vec2(0.0, 2.0)),
                    6.0 + f,
                    Color32::from_black_alpha(14),
                );
            }
            w::rounded(&p, window, t::PANEL_BG, 6.0);
            // 見出しの帯（浮いたウィンドウと同じ形: アイコン・欄の名前・閉じる）
            let header = Rect::from_min_size(window.min, vec2(window.width(), HEADER_HEIGHT));
            w::rounded(&p, header, t::PANEL_HEADER, 6.0);
            w::fill(
                &p,
                Rect::from_min_max(pos2(header.left(), header.bottom() - 6.0), header.max),
                t::PANEL_HEADER,
            );
            w::hline(
                &p,
                header.left(),
                header.right(),
                header.bottom() - 1.0,
                t::BORDER,
            );
            w::outline(&p, window, t::SEPARATOR, 1.0, 6.0);
            let icon = Rect::from_min_size(
                pos2(header.left() + 8.0, header.top()),
                vec2(20.0, header.height()),
            );
            w::icon(&p, icon, "palette", t::TEXT_DIM, 16.0);
            let close = Rect::from_min_size(
                pos2(header.right() - 28.0, header.top() + 2.0),
                vec2(24.0, header.height() - 4.0),
            );
            let title_left = icon.right() + 6.0;
            let title = w::fit(&p, &state.name, close.left() - title_left - 4.0, t::HEADER);
            w::text(
                &p,
                Rect::from_min_max(
                    pos2(title_left, header.top()),
                    pos2(close.left() - 4.0, header.bottom()),
                ),
                &title,
                t::HEADER,
                Align::Left,
            );
            let drag = ui.interact(
                Rect::from_min_max(header.min, pos2(close.left(), header.bottom())),
                state_id().with("drag"),
                Sense::drag(),
            );
            if drag.dragged() {
                moved += drag.drag_delta();
            }
            if w::icon_button(
                ui,
                close,
                state_id().with("close"),
                "close",
                lang.pick("閉じる", "Close"),
                false,
                true,
                15.0,
            )
            .clicked()
            {
                closed = true;
            }

            // 円と中の四角
            let wheel = wheel_of(window);
            let sq = wheel_square(wheel);
            let wheel_id = state_id().with("wheel");
            let response = ui.interact(wheel, wheel_id, Sense::click_and_drag());
            // 押した所が輪なら色相、中の四角なら彩度と明度（ドラッグの間は押したほうのまま）
            let mode_id = wheel_id.with("mode");
            if response.is_pointer_button_down_on() && ui.input(|i| i.pointer.any_pressed()) {
                let origin = ui.input(|i| i.pointer.press_origin());
                let mode = origin
                    .map(|o| {
                        if in_ring(wheel, o) {
                            1u8
                        } else if sq.contains(o) {
                            2
                        } else {
                            0
                        }
                    })
                    .unwrap_or(0);
                ui.data_mut(|d| d.insert_temp(mode_id, mode));
            }
            let wheel_down = response.is_pointer_button_down_on();
            let mut picked = false;
            if wheel_down {
                let mode: u8 = ui.data(|d| d.get_temp(mode_id).unwrap_or(0));
                if let Some(at) = response.interact_pointer_pos() {
                    match mode {
                        1 => {
                            state.hue = hue_at(wheel, at);
                            picked = true;
                        }
                        2 => {
                            state.sat = ((at.x - sq.left()) / sq.width()).clamp(0.0, 1.0);
                            state.val = (1.0 - (at.y - sq.top()) / sq.height()).clamp(0.0, 1.0);
                            picked = true;
                        }
                        _ => {}
                    }
                }
            }
            if picked {
                let next = Pick {
                    rgb: bytes(hsv_to_rgb(state.hue, state.sat, state.val)),
                    alpha: state.shown.alpha,
                };
                if next != state.shown {
                    state.shown = next;
                    updates.push((
                        target,
                        Update {
                            pick: next,
                            dragging: true,
                            done: false,
                            reverted: false,
                        },
                    ));
                }
            }
            let (ring, square) = {
                let mut tex = tex.lock().unwrap_or_else(|e| e.into_inner());
                (tex.ring(ui.ctx()), tex.sv(ui.ctx(), state.hue))
            };
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            p.image(ring, wheel, uv, Color32::WHITE);
            let radius = wheel.width() * 0.5 * (1.0 - RING_THICKNESS * 0.5);
            let a = state.hue * std::f32::consts::TAU;
            marker(
                &p,
                pos2(
                    wheel.center().x + a.sin() * radius,
                    wheel.center().y - a.cos() * radius,
                ),
                wheel.width() * RING_THICKNESS * super::color::MARKER_SCALE,
            );
            p.image(square, sq, uv, Color32::WHITE);
            w::outline(&p, sq, t::BORDER, 1.0, 0.0);
            marker(
                &p,
                pos2(
                    sq.left() + state.sat * sq.width(),
                    sq.top() + (1.0 - state.val) * sq.height(),
                ),
                6.0,
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Other,
                    true,
                    lang.pick("色相の円", "Hue wheel"),
                )
            });

            // 16 進
            let hex = format!(
                "#{}",
                to_hex([
                    f32::from(state.shown.rgb[0]) / 255.0,
                    f32::from(state.shown.rgb[1]) / 255.0,
                    f32::from(state.shown.rgb[2]) / 255.0,
                    1.0,
                ])
            );
            let typed = w::text_field(
                ui,
                hex_of(window, alpha),
                state_id().with("hex"),
                &hex,
                Some(lang.pick("16 進の色（#RRGGBB）", "Hex color (#RRGGBB)")),
                false,
            );
            // 別の欄を押して相手を替えたフレームに決まった入力は、打っていたときの相手（前の相手）へ当てる
            let prior = state.prior.filter(|_| state.switched == frame);
            if let (Some(text), Some((id, shown))) = (typed.committed.as_ref(), prior) {
                if let Some(rgb) = parse_hex(text) {
                    let next = Pick {
                        rgb: bytes((rgb[0], rgb[1], rgb[2])),
                        alpha: shown.alpha,
                    };
                    if next != shown {
                        updates.push((
                            id,
                            Update {
                                pick: next,
                                dragging: false,
                                done: true,
                                reverted: false,
                            },
                        ));
                    }
                }
            } else if let Some(text) = typed.committed {
                if let Some(rgb) = parse_hex(&text) {
                    let next = Pick {
                        rgb: bytes((rgb[0], rgb[1], rgb[2])),
                        alpha: state.shown.alpha,
                    };
                    let (h, s, v) = rgb_to_hsv(rgb[0], rgb[1], rgb[2]);
                    if s > 1e-4 && v > 1e-4 {
                        state.hue = h;
                    }
                    if v > 1e-4 {
                        state.sat = s;
                    }
                    state.val = v;
                    if next != state.shown {
                        state.shown = next;
                        updates.push((
                            target,
                            Update {
                                pick: next,
                                dragging: false,
                                done: true,
                                reverted: false,
                            },
                        ));
                    }
                } else {
                    let typed = lang.quote(&text);
                    failure = Some(lang.pick(
                        format!("{typed}は 16 進の色として読めません。"),
                        format!("{typed} is not a valid hex color."),
                    ));
                }
            }

            // アルファ（相手が持つときだけ）
            let mut alpha_active = false;
            if let Some(a) = state.shown.alpha {
                let spec = SliderSpec::new("A", 0.0, 100.0, NumberFormat::int("%"))
                    .tooltip(lang.pick("不透明度", "Opacity"));
                let out = w::slider(
                    ui,
                    alpha_of(window),
                    state_id().with("alpha"),
                    f32::from(a) / 255.0 * 100.0,
                    &spec,
                );
                alpha_active = out.active;
                if out.changed {
                    let next = Pick {
                        rgb: state.shown.rgb,
                        alpha: Some(w::to_byte(out.value / 100.0)),
                    };
                    if next != state.shown {
                        state.shown = next;
                        updates.push((
                            target,
                            Update {
                                pick: next,
                                dragging: out.active,
                                done: !out.active,
                                reverted: false,
                            },
                        ));
                    }
                }
            }

            // 描画色を入れる
            let paint = paint_of(window);
            let main = Pick {
                rgb: [
                    w::to_byte(sources.main[0]),
                    w::to_byte(sources.main[1]),
                    w::to_byte(sources.main[2]),
                ],
                alpha: state.shown.alpha,
            };
            if paint_button(
                ui,
                paint,
                main,
                lang.pick("描画色を入れる", "Use Paint Color"),
            ) && main != state.shown
            {
                state.sync(main);
                updates.push((
                    target,
                    Update {
                        pick: main,
                        dragging: false,
                        done: true,
                        reverted: false,
                    },
                ));
            }

            // 今のカラーセットの色
            if let Some(set) = sources.set.filter(|s| !s.colors.is_empty()) {
                let name = Rect::from_min_size(
                    pos2(window.left() + PAD, paint.bottom() + GAP),
                    vec2(WHEEL, SET_NAME),
                );
                let fitted = w::fit(&p, &set.name, name.width(), t::LABEL_DIM);
                w::text(&p, name, &fitted, t::LABEL_DIM, Align::Left);
                let (shown_rows, _) = set_rows(set.colors.len());
                let view = Rect::from_min_size(
                    pos2(name.left(), name.bottom()),
                    vec2(WHEEL, grid_height(shown_rows)),
                );
                if let Some(rgb) = set_grid(ui, view, set) {
                    let next = Pick {
                        rgb,
                        alpha: state.shown.alpha,
                    };
                    if next != state.shown {
                        state.sync(next);
                        updates.push((
                            target,
                            Update {
                                pick: next,
                                dragging: false,
                                done: true,
                                reverted: false,
                            },
                        ));
                    }
                }
            }
            // ドラッグを離した: 1 回の操作の終わり
            let now_dragging = wheel_down || alpha_active;
            if state.dragging && !now_dragging {
                updates.push((
                    target,
                    Update {
                        pick: state.shown,
                        dragging: true,
                        done: true,
                        reverted: false,
                    },
                ));
            }
            state.dragging = now_dragging;
        });
    crate::ui::window::note_open(ctx);
    if updates.iter().any(|(id, _)| *id == target) {
        state.touched = true;
    }
    for (id, update) in updates {
        push(ctx, id, update);
    }
    // Esc: 開いたときの色へ戻して閉じる。色が開いたときと同じでも、変更を出していれば戻しを出す（「自動」のような状態を持つ相手が、
    // 開いたときの状態へ戻せるように）。この Esc を、ほかの物が使ったときは使わない:
    // - 欄の文字を打っている途中の Esc は、その入力をやめるだけ。egui は Esc を受けたフレームの初めに欄のフォーカスを外すので、
    //   今のフォーカスでは分からない。前のフレームの終わりにフォーカスがあったかで見る（キャンバスの `typed_last` と同じ）
    // - ウィンドウより先に描く部品が使った Esc（塗りつぶしの仕事の取消・色の名前の変更をやめる）は、`escape_taken` の印で見る
    let focused_now = ctx.memory(|m| m.focused().is_some());
    let typed_last = ctx
        .data(|d| d.get_temp::<(u64, bool)>(focus_id()))
        .is_some_and(|(at, focused)| focused && frame <= at + 1);
    ctx.data_mut(|d| d.insert_temp(focus_id(), (frame, focused_now)));
    let taken = typed_last || crate::ui::window::escape_taken(ctx);
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !taken {
        if state.touched || state.original != state.shown {
            push(
                ctx,
                target,
                Update {
                    pick: state.original,
                    dragging: false,
                    done: true,
                    reverted: true,
                },
            );
        } else if state.dragging {
            push(
                ctx,
                target,
                Update {
                    pick: state.shown,
                    dragging: true,
                    done: true,
                    reverted: false,
                },
            );
        }
        close(ctx);
        return failure;
    }
    let placed = clamp_into(window.min + moved, size, screen);
    ctx.data_mut(|d| {
        d.insert_temp(pos_id(), placed);
        d.insert_temp(rect_id(), Rect::from_min_size(placed, size));
    });
    if closed {
        if state.dragging {
            push(
                ctx,
                target,
                Update {
                    pick: state.shown,
                    dragging: true,
                    done: true,
                    reverted: false,
                },
            );
        }
        close(ctx);
        return failure;
    }
    ctx.data_mut(|d| d.insert_temp(state_id(), state));
    failure
}

/// 「描画色を入れる」のボタン（左に描画色の小さな見本）。押されたら true。
fn paint_button(ui: &mut Ui, r: Rect, main: Pick, label: &str) -> bool {
    let response = ui.interact(r, state_id().with("paint"), Sense::click());
    let hover = response.hovered();
    let pressed = response.is_pointer_button_down_on();
    let p = ui.painter();
    w::rounded(
        p,
        r,
        if pressed {
            t::CONTROL_ACTIVE
        } else if hover {
            t::CONTROL_HOVER
        } else {
            t::PANEL_HEADER
        },
        4.0,
    );
    w::outline(p, r, t::BORDER, 1.0, 4.0);
    let chip = Rect::from_min_size(pos2(r.left() + 5.0, r.center().y - 7.0), vec2(14.0, 14.0));
    let c = main.rgb;
    w::rounded(p, chip, Color32::from_rgb(c[0], c[1], c[2]), 2.0);
    w::outline(p, chip, t::BORDER, 1.0, 2.0);
    w::text(
        p,
        Rect::from_min_max(pos2(chip.right() + 6.0, r.top()), r.max),
        label,
        t::LABEL,
        Align::Left,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    response.clicked()
}

/// カラーセットの色の列（多ければホイールで送る）。押された色を返す。
fn set_grid(ui: &mut Ui, view: Rect, set: &Palette) -> Option<[u8; 3]> {
    let offset_id = state_id().with("set-scroll");
    let mut offset: f32 = ui.data(|d| d.get_temp(offset_id).unwrap_or(0.0));
    let (_, all_rows) = set_rows(set.colors.len());
    let content = grid_height(all_rows);
    let scroll = Scroll::begin(ui, view, content, &mut offset);
    let width = view.width() - scroll.reserved();
    let columns = columns(width);
    let clip = ui.clip_rect();
    let p = ui.painter().with_clip_rect(view.intersect(clip));
    let mut chosen = None;
    for (i, swatch) in set.colors.iter().enumerate() {
        let (row, col) = (i / columns, i % columns);
        let cell = Rect::from_min_size(
            pos2(
                view.left() + col as f32 * (CELL + CELL_GAP),
                view.top() + row as f32 * (CELL + CELL_GAP) - offset,
            ),
            vec2(CELL, CELL),
        );
        if cell.bottom() < view.top() || cell.top() > view.bottom() {
            continue;
        }
        let hit = cell.intersect(view);
        let response = ui.interact(hit, offset_id.with(i), Sense::click());
        let [r, g, b, a] = swatch.rgba.map(w::to_byte);
        w::checker(&p, cell, CELL * 0.5);
        p.rect_filled(cell, 1.0, Color32::from_rgba_unmultiplied(r, g, b, a));
        w::outline(
            &p,
            cell,
            if response.hovered() {
                t::ACCENT
            } else {
                t::BORDER
            },
            1.0,
            1.0,
        );
        let label = swatch.label();
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
        if response.on_hover_text(label).clicked() {
            chosen = Some([r, g, b]);
        }
    }
    scroll.end(ui, offset_id.with("bar"), &mut offset);
    ui.data_mut(|d| d.insert_temp(offset_id, offset));
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_goes_beside_the_column_when_it_fits_and_off_the_field_when_it_does_not() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 1000.0));
        let size = size_for(0);
        let anchor = Rect::from_center_size(pos2(1160.0, 860.0), vec2(10.0, 10.0));
        let column = Rect::from_min_size(pos2(1070.0, 800.0), vec2(200.0, 0.0));
        let w = place(anchor, column, size, screen);
        assert!(w.right() <= column.left(), "欄の左: {w:?}");
        assert!(screen.contains_rect(w));
        // 画面の上下の端では、収まるように寄せる
        let high = Rect::from_center_size(pos2(1160.0, 20.0), vec2(10.0, 10.0));
        assert!(place(high, column, size, screen).top() >= 0.0);
        let low = Rect::from_center_size(pos2(1160.0, 990.0), vec2(10.0, 10.0));
        assert!(place(low, column, size, screen).bottom() <= 1000.0);
        // 欄の列が画面の左に寄っていて、左に入らないときは、列の右
        let left_column = Rect::from_min_size(pos2(8.0, 100.0), vec2(280.0, 0.0));
        let near = Rect::from_center_size(pos2(100.0, 200.0), vec2(10.0, 10.0));
        let w = place(near, left_column, size, screen);
        assert!(
            w.left() >= left_column.right() && screen.contains_rect(w),
            "{w:?}"
        );
        // 列が画面の幅いっぱいなら、欄の下。下に入らなければ上。どちらも欄に重ねない
        let wide = Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 0.0));
        let w = place(near, wide, size, screen);
        assert!(w.top() >= near.bottom() && screen.contains_rect(w), "{w:?}");
        let low = Rect::from_center_size(pos2(100.0, 900.0), vec2(10.0, 10.0));
        let w = place(low, wide, size, screen);
        assert!(w.bottom() <= low.top() && screen.contains_rect(w), "{w:?}");
    }

    #[test]
    fn a_remembered_place_outside_the_screen_comes_back_inside() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let size = size_for(30);
        for at in [
            pos2(-500.0, -300.0),
            pos2(2000.0, 1500.0),
            pos2(700.0, 10.0),
        ] {
            let min = clamp_into(at, size, screen);
            assert!(
                screen.contains_rect(Rect::from_min_size(min, size)),
                "{at:?} → {min:?}"
            );
        }
        // 中にあれば動かさない
        assert_eq!(
            clamp_into(pos2(100.0, 50.0), size, screen),
            pos2(100.0, 50.0)
        );
    }

    #[test]
    fn the_set_rows_are_capped_and_the_window_grows_with_them() {
        assert_eq!(set_rows(0), (0, 0));
        let columns = columns(WHEEL);
        assert_eq!(set_rows(1), (1, 1));
        assert_eq!(set_rows(columns), (1, 1));
        assert_eq!(set_rows(columns + 1), (2, 2));
        assert_eq!(set_rows(columns * 9), (MAX_SET_ROWS, 9));
        assert!(size_for(0).y < size_for(1).y);
        assert_eq!(size_for(columns * 4), size_for(columns * 20));
        // 中の部品はウィンドウの中に収まる
        let window = Rect::from_min_size(pos2(0.0, 0.0), size_for(columns * 20));
        for part in [
            wheel_of(window),
            hex_of(window, true),
            alpha_of(window),
            paint_of(window),
        ] {
            assert!(window.contains_rect(part), "{part:?}");
        }
    }

    #[test]
    fn a_pick_round_trips_through_floats() {
        let p = Pick {
            rgb: [10, 200, 255],
            alpha: Some(128),
        };
        assert_eq!(Pick::from_floats(p.floats(), true), p);
        assert_eq!(Pick::from_floats(p.floats(), false).alpha, None);
        assert_eq!(Pick::rgb([1, 2, 3]).floats()[3], 1.0);
    }
}

#[cfg(test)]
mod marker_room_tests {
    use super::*;

    /// 色相の選ぶ印の輪が円の外へはみ出す量は、円の置き場の余白（PAD）に収まる（ウィンドウの縁・見出しへ出ない）。
    #[test]
    fn the_marker_ring_stays_within_the_padding_around_the_wheel() {
        assert!(PAD >= super::super::color::marker_overhang(WHEEL));
    }
}
