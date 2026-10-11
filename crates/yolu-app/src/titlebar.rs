//! 上の帯とウィンドウの枠（Windows だけ。Mac・Linux は OS の枠のまま）。Windows では OS のタイトルバーを外し、メニューの帯の右端に
//! 小さな最小化・最大化（最大化中は元に戻す）・閉じるを置く。ウィンドウを動かす・大きさを変える・最大化を切り替えるのは、枠を外したウィンドウが
//! 自分で OS に頼む（`ViewportCommand`）。
//!
//! - 動かす: 帯の何も無い所を押して引くと `StartDrag`（OS の移動なので、画面の端へのスナップ・別のモニターへの移動が効く）。
//!   ダブルクリックで最大化と元に戻すを切り替える。メニューの見出し・Live Link の印・クラッシュの印を押したときは動かさない。
//!   ペン・指の押しの引きは `StartDrag` に渡さず、アプリの側でウィンドウを動かす（`send_drag_commands`。スナップは効かない）。マウスの引きで渡した `StartDrag` は、輪が
//!   始まらないままなら見張りが戻す。
//! - 大きさ: ウィンドウの縁（`EDGE`）を押すと `BeginResize`。最大化中・全画面中は無い。縁を押す前に、押した所の部品に譲る
//!   （メニューの見出し・スクロールのつまみなど、縁と同じ所にある細い部品。egui の押しを持たない Live Link の印は矩形で渡す）、
//!   浮かせたウィンドウ・メニューが上にあれば何もしない、ペン・タッチの押しは受けない（ペンは `contact`）。縁の押しはビューの押しとして
//!   使わせない（`edge_press_held`）。
//! - 閉じる: メニューの「終了」と同じ道（保存していない変更の確かめ）。
//!
//! 最大化中の内側は、自動で隠すタスクバーのある辺を 1 画素空ける（そうしないと、端へ寄せてもタスクバーが出てこない）。これはウィンドウの
//! プロシージャの側（`windowpos::native`）で行い、ここの帯と縁の描き方は変わらない。
//!
//! 別ウィンドウ（`detach`。パネルを外へ出したウィンドウ）も同じ決まりで枠を外す: タブの並ぶ行の何も無い所が帯の代わり（`drag_zone_with`）、
//! 行の右端に閉じるだけ（`close_button`。最小化・最大化は置かない）、縁は `edges_with`。縁の押しの印はウィンドウごとに持つ。
//!
//! ウィンドウの枠を外すのは `main.rs` が `CUSTOM_FRAME`（Windows だけ true）を見て決める。ここの関数は OS に依らず動くので、試験は
//! Linux でも Windows の帯を描いて確かめられる（アプリは `YoluApp::set_custom_frame` で帯を切り替える。設定には出さない）。

use egui::{
    Color32, Context, CursorIcon, Event, Id, Order, PointerButton, Pos2, Rect, ResizeDirection,
    Response, Sense, TouchPhase, Ui, ViewportCommand, WidgetInfo, WidgetType,
};

use crate::lang::Lang;
use crate::pen::PenInput;
use crate::ui::theme as t;
use crate::ui::widgets as w;

/// 実際のウィンドウで OS の枠を外すか（Windows だけ）。
pub const CUSTOM_FRAME: bool = cfg!(windows);

/// ボタン 1 つの幅（高さは帯いっぱい）。
pub const BUTTON_WIDTH: f32 = 30.0;
/// 3 つのボタンの幅の合計。帯の内容は、この分だけ右を空ける。
pub const BUTTONS_WIDTH: f32 = BUTTON_WIDTH * 3.0;
/// ボタンのアイコンの大きさ（論理の点）。
pub const ICON_SIZE: f32 = 14.0;
/// ウィンドウの縁の、大きさを変えられる幅（点）。
pub const EDGE: f32 = 5.0;
/// 角の、2 方向に変えられる長さ（点）。
pub const CORNER: f32 = 12.0;
/// 縁の押しを譲る部品の、縁をまたぐ向きの大きさの上限（点）。メニューの見出し（高さ 20）・スクロールのつまみの溝（幅 10）が入り、
/// ウィンドウの端まで広がるキャンバス・一覧の行は入らない。
pub const YIELD_SIZE: f32 = 24.0;

/// 帯の何も無い所（動かす・最大化）の部品の名前。
const DRAG_ID: &str = "yolu.titlebar.drag";
/// 縁の押しを譲らない自分の部品（帯の何も無い所と 3 つのボタン、`own` の部品。右上の角でも、縁が先に押しを受ける）。
fn is_own(id: Id, own: &[Id]) -> bool {
    id == Id::new(DRAG_ID) || Button::ALL.iter().any(|b| id == button_id(*b)) || own.contains(&id)
}

fn button_id(button: Button) -> Id {
    Id::new(("yolu.titlebar.button", button as u8))
}
/// 縁の押しを受けている間の印（ビューが、その押しを自分のものにしないため）。ウィンドウ（viewport）ごと。
const EDGE_PRESS: &str = "yolu.titlebar.edge_press";

fn edge_press_id(ctx: &Context) -> Id {
    Id::new((EDGE_PRESS, ctx.viewport_id()))
}

/// 帯の右端の 3 つのボタン。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Minimize,
    /// 最大化中は「元に戻す」。
    Maximize,
    Close,
}

impl Button {
    /// 左から並ぶ順。
    pub const ALL: [Button; 3] = [Button::Minimize, Button::Maximize, Button::Close];

    /// 名前（ツールチップと読み上げ。説明は付けない）。
    pub fn name(self, lang: Lang, maximized: bool) -> &'static str {
        match self {
            Button::Minimize => lang.pick("最小化", "Minimize"),
            Button::Maximize if maximized => lang.pick("元に戻す", "Restore"),
            Button::Maximize => lang.pick("最大化", "Maximize"),
            Button::Close => lang.pick("閉じる", "Close"),
        }
    }

    /// 同梱のアイコンの名前（Fluent。`assets/icons/THIRD-PARTY-NOTICES.md`）。
    pub fn icon(self, maximized: bool) -> &'static str {
        match self {
            Button::Minimize => "window_minimize",
            Button::Maximize if maximized => "window_restore",
            Button::Maximize => "window_maximize",
            Button::Close => "close",
        }
    }

    /// 押したときにウィンドウへ送る頼み。閉じるはウィンドウを直に閉じず、メニューの「終了」と同じ道（保存の確かめ）を通るので無い。
    pub fn command(self, maximized: bool) -> Option<ViewportCommand> {
        match self {
            Button::Minimize => Some(ViewportCommand::Minimized(true)),
            Button::Maximize => Some(ViewportCommand::Maximized(!maximized)),
            Button::Close => None,
        }
    }
}

/// 帯のうち、ボタンを除いた左側（`custom` でなければ帯そのまま）。名前・印はこの右端に寄せる。
pub fn content_rect(bar: Rect, custom: bool) -> Rect {
    if custom {
        Rect::from_min_max(
            bar.min,
            egui::pos2(bar.right() - BUTTONS_WIDTH, bar.bottom()),
        )
    } else {
        bar
    }
}

/// 3 つのボタンの矩形（左から最小化・最大化・閉じる）。帯の下の線の上までの高さ。
pub fn button_rects(bar: Rect) -> [Rect; 3] {
    let left = bar.right() - BUTTONS_WIDTH;
    let bottom = bar.bottom() - 1.0;
    std::array::from_fn(|i| {
        let x = left + i as f32 * BUTTON_WIDTH;
        Rect::from_min_max(
            egui::pos2(x, bar.top()),
            egui::pos2(x + BUTTON_WIDTH, bottom),
        )
    })
}

/// 3 つのボタンを描いて、押された（離した）ものを返す。
pub fn buttons(ui: &mut Ui, bar: Rect, maximized: bool, lang: Lang) -> Option<Button> {
    let mut clicked = None;
    for (button, rect) in Button::ALL.into_iter().zip(button_rects(bar)) {
        let name = button.name(lang, maximized);
        let response = paint_button(
            ui,
            rect,
            button_id(button),
            button.icon(maximized),
            name,
            button == Button::Close,
        );
        if response.clicked() {
            clicked = Some(button);
        }
    }
    clicked
}

/// 行（別ウィンドウのタブの並ぶ行）の右端の閉じるの矩形。帯の閉じると同じ幅で、行の下の線の上までの高さ。
pub fn close_rect(row: Rect) -> Rect {
    Rect::from_min_max(
        egui::pos2(row.right() - BUTTON_WIDTH, row.top()),
        egui::pos2(row.right(), row.bottom() - 1.0),
    )
}

/// 閉じるを 1 つ描く（帯の閉じると同じ見た目。`name` はツールチップと読み上げ）。押された（離した）なら true。
pub fn close_button(ui: &mut Ui, rect: Rect, id: Id, name: &str) -> bool {
    paint_button(ui, rect, id, Button::Close.icon(false), name, true).clicked()
}

/// ボタン 1 つを描く（乗せる・押すと地の色が変わる。閉じるは赤）。ツールチップは名前だけ。
fn paint_button(ui: &mut Ui, rect: Rect, id: Id, icon: &str, name: &str, close: bool) -> Response {
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, name));
    let hover = response.hovered();
    let down = response.is_pointer_button_down_on();
    let fill = match (hover, down, close) {
        (_, true, true) => Some(t::ERROR.gamma_multiply(0.75)),
        (true, _, true) => Some(t::ERROR),
        (_, true, false) => Some(t::CONTROL_ACTIVE),
        (true, _, false) => Some(t::CONTROL_HOVER),
        _ => None,
    };
    let p = ui.painter();
    if let Some(fill) = fill {
        w::fill(p, rect, fill);
    }
    let color = if hover || down {
        Color32::WHITE
    } else {
        t::TEXT
    };
    w::icon(p, rect, icon, color, ICON_SIZE);
    response.on_hover_text(name)
}

/// 帯の何も無い所の部品。メニューの見出しなどより先に（下に）作る。
pub fn drag_zone(ui: &mut Ui, zone: Rect) -> Response {
    drag_zone_with(ui, zone, Id::new(DRAG_ID))
}

/// 別ウィンドウ（`window` はウィンドウごとの番号）の、帯の代わりの部品の名前。
pub fn drag_id(window: u64) -> Id {
    Id::new((DRAG_ID, window))
}

/// 帯の代わりの部品（別ウィンドウのタブの並ぶ行）。タブより先に（下に）作る。
pub fn drag_zone_with(ui: &mut Ui, zone: Rect, id: Id) -> Response {
    ui.interact(zone, id, Sense::click_and_drag())
}

/// 帯の何も無い所の操作がウィンドウへ頼むこと: 引き始めで `StartDrag`、ダブルクリックで最大化と元に戻すの切り替え。`blockers`（メニューの見出し・
/// Live Link の印など、自分の押しを持つ部品の矩形）の上で押したものは、帯の操作にしない。
pub fn drag_commands(
    response: &Response,
    blockers: &[Rect],
    maximized: bool,
) -> Vec<ViewportCommand> {
    let at = response
        .ctx
        .input(|i| i.pointer.press_origin().or(i.pointer.interact_pos()));
    if at.is_some_and(|at| blockers.iter().any(|b| b.contains(at))) {
        return Vec::new();
    }
    if response.double_clicked_by(PointerButton::Primary) {
        vec![ViewportCommand::Maximized(!maximized)]
    } else if response.drag_started_by(PointerButton::Primary) {
        vec![ViewportCommand::StartDrag]
    } else {
        Vec::new()
    }
}

/// 帯の押しが Touch（winit が `WM_POINTER` のペン・指から作る）から来たか。egui の左ボタンの押しの前に、同じ入力の中で Touch の始まりがあれば、ペン・指の押し。
/// マウスの押しには Touch が付かない。押しの無いフレームは、前の値のまま。
pub fn press_from_touch(previous: bool, events: &[Event]) -> bool {
    let mut touch_start = false;
    let mut origin = previous;
    for event in events {
        match event {
            Event::Touch {
                phase: TouchPhase::Start,
                ..
            } => touch_start = true,
            Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                ..
            } => {
                origin = touch_start;
                touch_start = false;
            }
            _ => {}
        }
    }
    origin
}

/// 押しの出どころ（ペン・指か）を覚える部品の名前。ウィンドウ（viewport）ごと。
const TOUCH_PRESS: &str = "yolu.titlebar.touch_press";

/// 帯のあるウィンドウが毎フレーム呼ぶ: `drag_commands` が返した頼みを送る。
/// - ペン・指の押しの引き始め（`StartDrag`）は OS の移動の輪に渡さず、アプリの側でウィンドウを動かす（`pen`。触れているポインタが分からず動かせなくても、輪には渡さない）。
///   OS の移動の輪はマウスのボタンの押しで始めるもので、ペン・指の押しで始めると離しが届かず、winit の「動かしている最中」の印が残ってマウスでも動かせなくなりうる。
///   画面の端へのスナップは効かない。ウィンドウをアプリで動かす手が無いとき（Windows 以外・繋ぐ前）は、今までどおり `StartDrag`。
/// - マウスの引き始めは `StartDrag` を送り、`pen` の見張りに渡したと伝える（輪が始まらないまま 1 秒たったら、見張りが印を戻す）。
pub fn send_drag_commands(ctx: &Context, commands: Vec<ViewportCommand>, pen: &PenInput) {
    let id = Id::new((TOUCH_PRESS, ctx.viewport_id()));
    let previous = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let touch = ctx.input(|i| press_from_touch(previous, &i.events));
    ctx.data_mut(|d| d.insert_temp(id, touch));
    pen.poll_window();
    for command in commands {
        if matches!(command, ViewportCommand::StartDrag) {
            if touch && pen.moves_window() {
                pen.begin_window_move();
                continue;
            }
            pen.start_drag_handed();
        }
        ctx.send_viewport_cmd(command);
    }
}

// ───────── ウィンドウの縁 ─────────

/// ウィンドウの縁（`window` の内側 `EDGE`）の位置 `at` が指す、大きさを変える向き。角は `CORNER` まで 2 方向。縁でなければ None。
pub fn resize_direction(window: Rect, at: Pos2) -> Option<ResizeDirection> {
    if !window.contains(at) {
        return None;
    }
    let (dl, dr) = (at.x - window.left(), window.right() - at.x);
    let (dt, db) = (at.y - window.top(), window.bottom() - at.y);
    let near_left = dl < EDGE;
    let near_right = dr < EDGE;
    let near_top = dt < EDGE;
    let near_bottom = db < EDGE;
    let corner_left = dl < CORNER;
    let corner_right = dr < CORNER;
    let corner_top = dt < CORNER;
    let corner_bottom = db < CORNER;
    use ResizeDirection::*;
    if (near_top && corner_left) || (near_left && corner_top) {
        Some(NorthWest)
    } else if (near_top && corner_right) || (near_right && corner_top) {
        Some(NorthEast)
    } else if (near_bottom && corner_left) || (near_left && corner_bottom) {
        Some(SouthWest)
    } else if (near_bottom && corner_right) || (near_right && corner_bottom) {
        Some(SouthEast)
    } else if near_top {
        Some(North)
    } else if near_bottom {
        Some(South)
    } else if near_left {
        Some(West)
    } else if near_right {
        Some(East)
    } else {
        None
    }
}

/// 向きに合うポインタの形。
pub fn cursor_for(direction: ResizeDirection) -> CursorIcon {
    use ResizeDirection::*;
    match direction {
        North | South => CursorIcon::ResizeVertical,
        East | West => CursorIcon::ResizeHorizontal,
        NorthWest | SouthEast => CursorIcon::ResizeNwSe,
        NorthEast | SouthWest => CursorIcon::ResizeNeSw,
    }
}

/// 縁の押しを譲る部品か: `rect` の、縁をまたぐ向きの大きさが `YIELD_SIZE` 以下（左右の縁なら幅、上下の縁なら高さ。角はどちらか）。
fn yields_to(direction: ResizeDirection, rect: Rect) -> bool {
    use ResizeDirection::*;
    let horizontal = matches!(
        direction,
        East | West | NorthEast | NorthWest | SouthEast | SouthWest
    );
    let vertical = matches!(
        direction,
        North | South | NorthEast | NorthWest | SouthEast | SouthWest
    );
    (horizontal && rect.width() <= YIELD_SIZE) || (vertical && rect.height() <= YIELD_SIZE)
}

/// ポインタの下で、縁の押しを譲る部品が押しを受けるか（egui の当たり判定の結果: 押し・つまみを受ける部品のうち、細いもの）。
fn control_under_pointer(ctx: &Context, direction: ResizeDirection, own: &[Id]) -> bool {
    let hovered = ctx.interaction_snapshot(|s| s.hovered.clone());
    hovered.into_iter().any(|id| {
        if is_own(id, own) {
            return false;
        }
        ctx.read_response(id).is_some_and(|r| {
            (r.sense.senses_click() || r.sense.senses_drag()) && yields_to(direction, r.rect)
        })
    })
}

/// 浮かせたウィンドウ・メニューなど、帯とパネルの上にあるレイヤーが `at` を覆っているか。
fn covered(ctx: &Context, at: Pos2) -> bool {
    ctx.layer_id_at(at)
        .is_some_and(|layer| layer.order != Order::Background)
}

/// 縁の押しを受けている間か（ビューは、この押しを自分のものにしない）。
pub fn edge_press_held(ctx: &Context) -> bool {
    let id = edge_press_id(ctx);
    ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false)
}

/// フレームの始めに呼ぶ。縁を押したら `BeginResize` を送り、ポインタが縁に乗っているときはその向きを返す（ポインタの形は、ほかの部品が
/// 決めたあとに `edge_cursor` で上書きする）。`busy` は描いている最中・ペンが触れている最中（縁の押しを受けない）。`press_rects` は、
/// egui の押しを持たず生の押しで動く部品（Live Link の印）の矩形で、縁の押しはその上では譲る（egui の当たり判定に出ないので矩形で渡す）。
pub fn edges(ctx: &Context, busy: bool, press_rects: &[Rect]) -> Option<ResizeDirection> {
    edges_with(ctx, busy, press_rects, &[])
}

/// `edges` に、縁の押しを譲らない自分の部品（別ウィンドウの帯の代わりの部品と閉じる）を足したもの。`ctx` のウィンドウ（viewport）の縁を見る。
pub fn edges_with(
    ctx: &Context,
    busy: bool,
    press_rects: &[Rect],
    own: &[Id],
) -> Option<ResizeDirection> {
    let any_down = ctx.input(|i| i.pointer.any_down());
    let press_id = edge_press_id(ctx);
    if !any_down {
        ctx.data_mut(|d| d.remove::<bool>(press_id));
    }
    let (maximized, fullscreen) = ctx.input(|i| {
        let v = i.viewport();
        (v.maximized.unwrap_or(false), v.fullscreen.unwrap_or(false))
    });
    if maximized || fullscreen {
        return None;
    }
    let (hover, press, touched) = ctx.input(|i| {
        (
            i.pointer.hover_pos(),
            if i.pointer.primary_pressed() {
                i.pointer.press_origin()
            } else {
                None
            },
            i.events.iter().any(|e| matches!(e, Event::Touch { .. })),
        )
    });
    let at = press.or(hover)?;
    let direction = resize_direction(ctx.content_rect(), at)?;
    if covered(ctx, at)
        || control_under_pointer(ctx, direction, own)
        || press_rects.iter().any(|r| r.contains(at))
    {
        return None;
    }
    if press.is_some() {
        if busy || touched {
            return None;
        }
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
        ctx.data_mut(|d| d.insert_temp(press_id, true));
        return Some(direction);
    }
    // 別の操作でボタンを押している最中は、ポインタの形を変えない
    (!any_down).then_some(direction)
}

/// `edges` が返した向きのポインタの形を出す（フレームの終わりで。キャンバスなどが決めた形を上書きする）。
pub fn edge_cursor(ctx: &Context, direction: ResizeDirection) {
    ctx.set_cursor_icon(cursor_for(direction));
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};
    use ResizeDirection::*;

    fn window() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 700.0))
    }

    #[test]
    fn the_four_sides_and_corners_resize_in_their_directions() {
        let w = window();
        let cases = [
            (pos2(500.0, 1.0), North),
            (pos2(500.0, 699.0), South),
            (pos2(1.0, 350.0), West),
            (pos2(999.0, 350.0), East),
            (pos2(1.0, 1.0), NorthWest),
            (pos2(999.0, 1.0), NorthEast),
            (pos2(1.0, 699.0), SouthWest),
            (pos2(999.0, 699.0), SouthEast),
            // 辺に乗ったまま角の近く（角の長さの内側）は角、外側は辺
            (pos2(11.0, 1.0), NorthWest),
            (pos2(13.0, 1.0), North),
            (pos2(1.0, 11.0), NorthWest),
            (pos2(1.0, 13.0), West),
        ];
        for (at, want) in cases {
            assert_eq!(resize_direction(w, at), Some(want), "{at:?}");
        }
    }

    #[test]
    fn the_inside_and_the_outside_do_not_resize() {
        let w = window();
        for at in [
            pos2(500.0, 350.0),
            pos2(EDGE + 0.5, 350.0),
            pos2(500.0, EDGE + 0.5),
            pos2(-1.0, 350.0),
            pos2(1001.0, 10.0),
        ] {
            assert_eq!(resize_direction(w, at), None, "{at:?}");
        }
    }

    #[test]
    fn each_direction_has_a_matching_cursor() {
        assert_eq!(cursor_for(North), CursorIcon::ResizeVertical);
        assert_eq!(cursor_for(South), CursorIcon::ResizeVertical);
        assert_eq!(cursor_for(East), CursorIcon::ResizeHorizontal);
        assert_eq!(cursor_for(West), CursorIcon::ResizeHorizontal);
        assert_eq!(cursor_for(NorthWest), CursorIcon::ResizeNwSe);
        assert_eq!(cursor_for(SouthEast), CursorIcon::ResizeNwSe);
        assert_eq!(cursor_for(NorthEast), CursorIcon::ResizeNeSw);
        assert_eq!(cursor_for(SouthWest), CursorIcon::ResizeNeSw);
    }

    #[test]
    fn only_thin_controls_take_the_press_from_the_edge() {
        // 右の縁: 幅 10 のつまみの溝は譲り、幅の広い一覧の行・キャンバスは譲らない
        let thumb = Rect::from_min_size(pos2(990.0, 100.0), vec2(10.0, 300.0));
        let row = Rect::from_min_size(pos2(700.0, 100.0), vec2(300.0, 22.0));
        let canvas = Rect::from_min_size(pos2(300.0, 60.0), vec2(700.0, 600.0));
        assert!(yields_to(East, thumb));
        assert!(!yields_to(East, row));
        assert!(!yields_to(East, canvas));
        // 上の縁: 高さ 20 のメニューの見出しは譲る
        let menu = Rect::from_min_size(pos2(6.0, 2.0), vec2(60.0, 20.0));
        assert!(yields_to(North, menu));
        assert!(!yields_to(North, canvas));
    }

    #[test]
    fn the_buttons_fill_the_right_end_of_the_bar_above_its_bottom_line() {
        let bar = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, t::MENU_BAR_HEIGHT));
        let rects = button_rects(bar);
        assert_eq!(rects[2].right(), 1000.0);
        assert_eq!(rects[0].left(), 1000.0 - BUTTONS_WIDTH);
        for pair in rects.windows(2) {
            assert_eq!(pair[0].right(), pair[1].left());
        }
        for r in rects {
            assert_eq!(r.top(), 0.0);
            assert_eq!(r.bottom(), t::MENU_BAR_HEIGHT - 1.0, "下の線は覆わない");
        }
        assert_eq!(content_rect(bar, true).right(), 1000.0 - BUTTONS_WIDTH);
        assert_eq!(content_rect(bar, false), bar);
    }

    #[test]
    fn buttons_name_and_command_follow_the_maximized_state() {
        for (maximized, name_en, icon) in [
            (false, "Maximize", "window_maximize"),
            (true, "Restore", "window_restore"),
        ] {
            assert_eq!(Button::Maximize.name(Lang::En, maximized), name_en);
            assert_eq!(Button::Maximize.icon(maximized), icon);
            assert!(
                matches!(Button::Maximize.command(maximized), Some(ViewportCommand::Maximized(m)) if m == !maximized)
            );
        }
        assert!(matches!(
            Button::Minimize.command(false),
            Some(ViewportCommand::Minimized(true))
        ));
        assert!(
            Button::Close.command(false).is_none(),
            "閉じるは終了の道を通る"
        );
        assert_eq!(Button::Close.name(Lang::Ja, false), "閉じる");
    }
}
