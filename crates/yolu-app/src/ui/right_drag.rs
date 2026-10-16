//! 右ボタンのドラッグの持ち主。押し始めた所が振る舞いを決める: 一覧（`scroll::Scroll`、egui の `ScrollArea`）で押し始めたらその一覧が
//! スクロールし、ツールの帯で押し始めたら帯の上を通ったツールを選ぶ。押し始めた所（持ち主）は押したフレームに 1 つに決まり、
//! 離すまで変わらない（ポインタが持ち主の外へ出ても、持ち主のまま）。キャンバス・3D ビューの右ボタン（`input::owner`）は別の道で、ここは通らない。
//!
//! 持ち主の決め方: 押したフレームに、押した点を含み、その点の一番上のレイヤーが自分の ui のレイヤーである欄が名乗りを上げ、最初の 1 つが勝つ。
//! 入れ子の一覧は内側が先に名乗る（内側の `end` が外側の `end` より先に呼ばれる）ので内側が勝つ。
//! 動かしたとみなすのは、押した点から egui のクリックの遊び（`max_click_dist`）を超えて離れたとき。動かさずに離せば、egui の
//! `secondary_clicked` が立つ（今の右クリックのメニュー・スポイト）。動かしたあとは立たない。
//! 押しの記録はウィンドウ（viewport）ごと。離す・Esc・ウィンドウのフォーカスの喪失で捨てる。

use egui::{Id, PointerButton, Pos2, Rect, Ui, Vec2};

/// 持ち主が動かされているときの、ポインタの位置と、前のフレームからの動き。
#[derive(Clone, Copy, Debug)]
pub struct Drag {
    pub pos: Pos2,
    /// 前のフレームからの動き。遊びを超えた最初のフレームは 0（遊びの分は数えない）。
    pub delta: Vec2,
}

/// 今の右ボタンの押し。egui の一時の記憶に、ウィンドウごとに 1 つだけ置く。
#[derive(Clone, Copy)]
struct Press {
    /// 押したフレーム。
    frame: u64,
    /// 押した点。
    origin: Pos2,
    /// 持ち主（押したフレームに名乗った最初の欄）。
    owner: Option<Id>,
    /// 遊びを超えて動かしたか。
    moved: bool,
    /// 持ち主が前に見たポインタの位置と、それを見たフレーム、そのときの動き。
    last: Option<Pos2>,
    seen: u64,
    delta: Vec2,
}

fn key(ctx: &egui::Context) -> Id {
    Id::new(("yolu.right_drag", ctx.viewport_id()))
}

/// `id` の欄（見える範囲 `area`）が、右ボタンを押したまま動かされているとき、その動きを返す。
/// 欄が見えている間は毎フレーム呼ぶ（押したフレームの名乗りもここ）。1 フレームに何度呼んでもよい。
/// 押し始めた所が `area` の外・別のレイヤー・ほかの欄が先に名乗った、のときは常に `None`。
pub fn drag(ui: &Ui, id: Id, area: Rect) -> Option<Drag> {
    let ctx = ui.ctx();
    let key = key(ctx);
    let frame = ctx.cumulative_frame_nr();
    let (pressed, down, now, origin) = ui.input(|i| {
        (
            i.pointer.button_pressed(PointerButton::Secondary),
            i.pointer.button_down(PointerButton::Secondary),
            i.pointer.latest_pos(),
            i.pointer.press_origin(),
        )
    });
    let mut press: Option<Press> = ctx.data(|d| d.get_temp(key));
    if pressed && press.is_none_or(|p| p.frame != frame) {
        press = origin.map(|origin| Press {
            frame,
            origin,
            owner: None,
            moved: false,
            last: None,
            seen: 0,
            delta: Vec2::ZERO,
        });
    }
    let mut press = press?;
    if !down {
        ctx.data_mut(|d| d.remove::<Press>(key));
        return None;
    }
    if press.frame == frame
        && press.owner.is_none()
        && area.contains(press.origin)
        && ctx.layer_id_at(press.origin) == Some(ui.layer_id())
    {
        press.owner = Some(id);
    }
    // Esc とウィンドウのフォーカスの喪失は、持ち主がいる押しをやめる（離したのを受け取れないことがある）。Esc は使い切る
    if press.owner.is_some() {
        let escaped = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let focus_lost = ui.input(|i| {
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::WindowFocused(false)))
        });
        if escaped || focus_lost {
            ctx.data_mut(|d| d.remove::<Press>(key));
            return None;
        }
    }
    if let Some(now) = now {
        let slop = ctx.options(|o| o.input_options.max_click_dist);
        press.moved |= now.distance(press.origin) > slop;
    }
    let mut out = None;
    if press.owner == Some(id) && press.moved {
        if let Some(now) = now {
            if press.seen != frame {
                press.delta = press.last.map_or(Vec2::ZERO, |last| now - last);
                press.last = Some(now);
                press.seen = frame;
            }
            out = Some(Drag {
                pos: now,
                delta: press.delta,
            });
        }
    }
    ctx.data_mut(|d| d.insert_temp(key, press));
    out
}

/// 右ドラッグで通った項目 `now` が、前のフレームから替わった時だけ `Some(項目)` を返す（ドラッグしていない・項目の外のときは `None` を渡す。
/// 毎フレーム呼ぶ）。同じ項目の上に居続けても、替わるまでは返さない。
pub fn entered<T>(ui: &Ui, id: Id, now: Option<T>) -> Option<T>
where
    T: Copy + PartialEq + Send + Sync + 'static,
{
    let id = id.with("entered");
    let last: Option<T> = ui.data(|d| d.get_temp::<Option<T>>(id)).flatten();
    ui.data_mut(|d| d.insert_temp::<Option<T>>(id, now));
    now.filter(|n| Some(*n) != last)
}
