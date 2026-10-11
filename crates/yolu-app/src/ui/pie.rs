//! パイメニューの部品: 8 方向（上・右上・右・右下・下・左下・左・左上）に項目を並べる。真ん中に項目は置かない。
//!
//! - キーで開いたら（`Runtime::key`）、押したまま項目の方へ動かして離すと実行。開いた所から真ん中の遊び（`DEAD_ZONE`）より動かさずに
//!   すぐ離したら（`TAP_SECONDS` より短い）開いたままにして、クリックで選ぶ（画面の端で真ん中をずらし、動かさずにポインタが項目を指していても
//!   選ばない）。動かさずに長く押して離したら閉じる。
//! - 開いている間は 1〜8 の数字（上から時計回り）でも選べ、Esc・右クリックで閉じる。左クリックは指している項目を実行し、真ん中なら閉じる。
//! - 開いている間は画面全体に入力の受け皿を置き、下の部品（キャンバス・3D ビュー・パネル）へ渡さない（右クリックのメニューと同じ）。
//! - 指す項目は、ポインタが真ん中の遊び（`DEAD_ZONE`）の外にあるとき、向きがいちばん近い項目（67.5° 以内）。項目の箱の上なら、その項目。

use egui::{
    pos2, vec2, Color32, Id, Key, Order, Pos2, Rect, Sense, Stroke, Vec2, WidgetInfo, WidgetType,
};

use super::theme as t;
use super::widgets::{self as w, Align};

/// 項目の数（方向の数）。
pub const SLOTS: usize = 8;
/// 真ん中から項目の置き場までの距離（点）。
pub const RADIUS: f32 = 100.0;
/// 真ん中の遊び（この内側では、どの項目も指さない）。
pub const DEAD_ZONE: f32 = 24.0;
/// これより短く押して離したら、開いたままにしてクリックで選ぶ（秒）。
pub const TAP_SECONDS: f64 = 0.25;
/// 項目の箱の高さ。
pub const ITEM_HEIGHT: f32 = 30.0;
/// 向きで指せる角度（項目の向きからの差。度）。
const REACH_DEGREES: f32 = 67.5;
/// 数字で選ぶキー（上から時計回り）。
pub const DIGITS: [Key; SLOTS] = [
    Key::Num1,
    Key::Num2,
    Key::Num3,
    Key::Num4,
    Key::Num5,
    Key::Num6,
    Key::Num7,
    Key::Num8,
];

/// 項目 1 つの見せ方。
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub label: String,
    pub icon: Option<&'static str>,
    pub enabled: bool,
    /// 今の状態の項目（今のモードなど。青く出す）。
    pub checked: bool,
    /// 乗せると出る説明（押せない理由など）。
    pub tooltip: Option<String>,
}

/// 開いているパイの途中の状態。
#[derive(Clone, Debug, PartialEq)]
pub struct Runtime {
    /// 開いた所（真ん中）。画面の端では、項目が収まるようにずらす。
    pub center: Pos2,
    /// 開いたときのポインタの位置（ずらす前の真ん中。押したまま離したとき、ここから動かしたかを見る）。
    pub origin: Pos2,
    /// 開いたキー（押したまま離したら、指している項目を実行する）。キーで開いていない・離したら None。
    pub key: Option<Key>,
    /// 開いた時刻（`InputState::time`）。
    pub opened_at: f64,
    /// 開いたフレーム（そのフレームの押しは数えない）。
    pub opened_frame: u64,
    /// 指している項目。
    pub hovered: Option<usize>,
}

impl Runtime {
    pub fn new(ctx: &egui::Context, center: Pos2, key: Option<Key>) -> Runtime {
        Runtime {
            center,
            key,
            origin: center,
            opened_at: ctx.input(|i| i.time),
            opened_frame: ctx.cumulative_frame_nr(),
            hovered: None,
        }
    }
}

/// パイの結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Open,
    Close,
    /// この番号の項目を選んだ。
    Chosen(usize),
}

/// 番号の項目の向き（上が 0、時計回りに 45° ずつ。画面の y は下向き）。
pub fn direction(slot: usize) -> Vec2 {
    let angle = (slot as f32 * 45.0).to_radians();
    vec2(angle.sin(), -angle.cos())
}

/// 項目の箱の大きさ。
fn item_size(painter: &egui::Painter, slot: &Slot) -> Vec2 {
    let text = w::text_width(painter, &slot.label, t::LABEL);
    let icon = if slot.icon.is_some() { 24.0 } else { 0.0 };
    vec2((text + icon + 24.0).max(48.0), ITEM_HEIGHT)
}

/// 真ん中に対する項目の箱（上・下は真ん中にそろえ、横の向きの項目は真ん中に近い側の縁を置き場にそろえる）。
pub fn item_rect(center: Pos2, slot: usize, size: Vec2) -> Rect {
    let d = direction(slot);
    let at = center + d * RADIUS;
    let x = if d.x > 0.3 {
        at.x
    } else if d.x < -0.3 {
        at.x - size.x
    } else {
        at.x - size.x / 2.0
    };
    Rect::from_min_size(pos2(x, at.y - size.y / 2.0), size)
}

/// 画面の中に項目が全部入るように真ん中をずらす。
fn fit_center(center: Pos2, rects: &[Rect], screen: Rect) -> Pos2 {
    let Some(all) = rects.iter().copied().reduce(|a, b| a.union(b)) else {
        return center;
    };
    let inner = screen.shrink(4.0);
    let mut shift = Vec2::ZERO;
    if all.left() < inner.left() {
        shift.x = inner.left() - all.left();
    } else if all.right() > inner.right() {
        shift.x = inner.right() - all.right();
    }
    if all.top() < inner.top() {
        shift.y = inner.top() - all.top();
    } else if all.bottom() > inner.bottom() {
        shift.y = inner.bottom() - all.bottom();
    }
    center + shift
}

/// ポインタの位置で指す項目（項目の箱の上ならその項目、そうでなければ遊びの外の向きにいちばん近い項目）。押せない項目を指しているときは
/// None（隣の押せる項目に替えない）。
pub fn pointed(
    center: Pos2,
    pointer: Pos2,
    slots: &[Option<Slot>; SLOTS],
    rects: &[Option<Rect>; SLOTS],
) -> Option<usize> {
    let enabled = |i: usize| slots[i].as_ref().is_some_and(|s| s.enabled);
    if let Some(i) = (0..SLOTS).find(|&i| rects[i].is_some_and(|r| r.contains(pointer))) {
        return enabled(i).then_some(i);
    }
    let offset = pointer - center;
    if offset.length() < DEAD_ZONE {
        return None;
    }
    // 上から時計回りの角度
    let angle = offset.x.atan2(-offset.y).to_degrees().rem_euclid(360.0);
    (0..SLOTS)
        .filter(|&i| slots[i].is_some())
        .map(|i| {
            let diff = (angle - i as f32 * 45.0).rem_euclid(360.0);
            (i, diff.min(360.0 - diff))
        })
        .filter(|(_, diff)| *diff <= REACH_DEGREES)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
        .filter(|&i| enabled(i))
}

/// パイを描いて、入力を受ける。
pub fn show(
    ctx: &egui::Context,
    id: Id,
    run: &mut Runtime,
    slots: &[Option<Slot>; SLOTS],
) -> Outcome {
    let screen = ctx.content_rect();
    // 開いているあいだはキーをパイが受ける（ほかの部品のフォーカスを外す）
    if let Some(focused) = ctx.memory(|m| m.focused()) {
        ctx.memory_mut(|m| m.surrender_focus(focused));
    }
    // 入力の受け皿（外を押しても下の部品へ渡さない）
    egui::Area::new(id.with("blocker"))
        .order(Order::Foreground)
        .fixed_pos(screen.min)
        .constrain(false)
        .interactable(true)
        .show(ctx, |ui| {
            ui.interact(screen, id.with("blocker-hit"), Sense::click_and_drag());
        });
    let painter = ctx.layer_painter(egui::LayerId::new(Order::Tooltip, id));
    let sizes: Vec<Option<Vec2>> = slots
        .iter()
        .map(|s| s.as_ref().map(|s| item_size(&painter, s)))
        .collect();
    let rects_at = |center: Pos2| -> [Option<Rect>; SLOTS] {
        std::array::from_fn(|i| sizes[i].map(|size| item_rect(center, i, size)))
    };
    let placed: Vec<Rect> = rects_at(run.center).into_iter().flatten().collect();
    run.center = fit_center(run.center, &placed, screen);
    let rects = rects_at(run.center);

    let frame = ctx.cumulative_frame_nr();
    let (pointer, time, digit, escape, released, primary, secondary, focus_lost) = ctx.input(|i| {
        let digit = DIGITS.iter().position(|k| i.key_pressed(*k));
        let released = run.key.is_some_and(|key| {
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::Key { key: k, pressed: false, .. } if *k == key))
        });
        (
            i.pointer.latest_pos(),
            i.time,
            digit,
            i.key_pressed(Key::Escape),
            released,
            i.pointer.primary_pressed(),
            i.pointer.secondary_pressed(),
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::WindowFocused(false))),
        )
    });
    run.hovered = pointer.and_then(|p| pointed(run.center, p, slots, &rects));
    let mut outcome = Outcome::Open;
    let enabled = |i: usize| slots[i].as_ref().is_some_and(|s| s.enabled);
    if escape || focus_lost {
        outcome = Outcome::Close;
    } else if let Some(i) = digit {
        if enabled(i) {
            outcome = Outcome::Chosen(i);
        }
    } else if released {
        run.key = None;
        // 開いた所から遊びより動かしていなければ、指している項目は選ばない（画面の端で真ん中をずらすと、動かさずに項目を指すことがある）
        let moved = pointer.is_some_and(|p| (p - run.origin).length() >= DEAD_ZONE);
        match run.hovered.filter(|_| moved) {
            Some(i) => outcome = Outcome::Chosen(i),
            // すぐ離した: 開いたままにして、クリックで選ぶ
            None if time - run.opened_at < TAP_SECONDS => {}
            None => outcome = Outcome::Close,
        }
    } else if frame != run.opened_frame && secondary {
        outcome = Outcome::Close;
    } else if frame != run.opened_frame && primary {
        outcome = match run.hovered {
            Some(i) => Outcome::Chosen(i),
            None => Outcome::Close,
        };
    }

    // 描く
    egui::Area::new(id)
        .order(Order::Tooltip)
        .fixed_pos(screen.min)
        .constrain(false)
        .show(ctx, |ui| {
            let p = ui.painter().clone();
            // 真ん中の小さな輪と、指している向きの点
            p.circle_stroke(run.center, 10.0, Stroke::new(2.0, t::SEPARATOR));
            if let Some(at) = pointer.filter(|at| (*at - run.center).length() >= DEAD_ZONE) {
                let d = (at - run.center).normalized();
                p.circle_filled(run.center + d * 10.0, 3.5, t::ACCENT);
            }
            for i in 0..SLOTS {
                let (Some(slot), Some(rect)) = (slots[i].as_ref(), rects[i]) else {
                    continue;
                };
                let pointed = run.hovered == Some(i);
                let fill = if pointed {
                    t::ACCENT
                } else if slot.checked {
                    t::ACCENT_DIM
                } else {
                    t::PANEL_HEADER
                };
                w::rounded(&p, rect, fill, 6.0);
                if !pointed && !slot.checked {
                    w::outline(&p, rect, t::BORDER, 1.0, 6.0);
                }
                let color = if !slot.enabled {
                    t::TEXT_DISABLED
                } else if pointed || slot.checked {
                    Color32::WHITE
                } else {
                    t::TEXT
                };
                let mut text_left = rect.left() + 12.0;
                if let Some(icon) = slot.icon {
                    w::icon(
                        &p,
                        Rect::from_min_size(
                            pos2(rect.left() + 8.0, rect.top()),
                            vec2(20.0, rect.height()),
                        ),
                        icon,
                        color,
                        18.0,
                    );
                    text_left += 22.0;
                }
                w::text(
                    &p,
                    Rect::from_min_max(
                        pos2(text_left, rect.top()),
                        pos2(rect.right() - 8.0, rect.bottom()),
                    ),
                    &slot.label,
                    t::LABEL.with_color(color),
                    Align::Left,
                );
                // 試験・読み上げのための名前（押しはこの部品でなく、上の入力の判定が受ける）と、乗せると出る説明
                let response = ui.interact(rect, id.with(("slot", i)), Sense::hover());
                response.widget_info(|| {
                    WidgetInfo::selected(
                        WidgetType::Button,
                        slot.enabled,
                        slot.checked,
                        &slot.label,
                    )
                });
                if let Some(tip) = slot.tooltip.as_deref().filter(|s| !s.is_empty()) {
                    response.on_hover_text(tip);
                }
            }
        });
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(label: &str) -> Option<Slot> {
        Some(Slot {
            label: label.into(),
            icon: None,
            enabled: true,
            checked: false,
            tooltip: None,
        })
    }

    #[test]
    fn slots_go_clockwise_from_the_top() {
        let near = |a: Vec2, b: Vec2| (a - b).length() < 1e-5;
        assert!(near(direction(0), vec2(0.0, -1.0)));
        assert!(near(direction(2), vec2(1.0, 0.0)));
        assert!(near(direction(4), vec2(0.0, 1.0)));
        assert!(near(direction(6), vec2(-1.0, 0.0)));
        let d = direction(1);
        assert!(d.x > 0.0 && d.y < 0.0, "右上: {d:?}");
        let d = direction(7);
        assert!(d.x < 0.0 && d.y < 0.0, "左上: {d:?}");
    }

    #[test]
    fn side_items_keep_their_inner_edge_on_the_ring_and_top_and_bottom_are_centered() {
        let c = pos2(500.0, 400.0);
        let size = vec2(80.0, ITEM_HEIGHT);
        let right = item_rect(c, 2, size);
        assert_eq!(right.left(), c.x + RADIUS);
        let left = item_rect(c, 6, size);
        assert_eq!(left.right(), c.x - RADIUS);
        let top = item_rect(c, 0, size);
        assert_eq!(top.center().x, c.x);
        assert!(top.bottom() < c.y - DEAD_ZONE);
        let bottom = item_rect(c, 4, size);
        assert_eq!(bottom.center().x, c.x);
        assert!(bottom.top() > c.y + DEAD_ZONE);
    }

    #[test]
    fn the_pointer_picks_the_nearest_filled_direction_outside_the_dead_zone() {
        let c = pos2(500.0, 400.0);
        // 上・右・左だけ（モードのパイと同じ並び）
        let mut slots: [Option<Slot>; SLOTS] = Default::default();
        slots[0] = slot("上");
        slots[2] = slot("右");
        slots[6] = slot("左");
        let rects: [Option<Rect>; SLOTS] = std::array::from_fn(|i| {
            slots[i]
                .as_ref()
                .map(|_| item_rect(c, i, vec2(80.0, ITEM_HEIGHT)))
        });
        // 遊びの中は何も指さない
        assert_eq!(pointed(c, c + vec2(5.0, -5.0), &slots, &rects), None);
        assert_eq!(pointed(c, c + vec2(0.0, -40.0), &slots, &rects), Some(0));
        assert_eq!(pointed(c, c + vec2(40.0, 0.0), &slots, &rects), Some(2));
        assert_eq!(pointed(c, c + vec2(-40.0, 0.0), &slots, &rects), Some(6));
        // 右下（135°）は右に近い（45° の差）。真下（180°）はどれからも 90° 離れているので指さない
        assert_eq!(pointed(c, c + vec2(40.0, 40.0), &slots, &rects), Some(2));
        assert_eq!(pointed(c, c + vec2(0.0, 40.0), &slots, &rects), None);
        // 押せない項目は指さない（隣の押せる項目にも替えない。右下は右の向きなので、右が押せなければ何も指さない）
        slots[2].as_mut().unwrap().enabled = false;
        assert_eq!(pointed(c, c + vec2(40.0, 0.0), &slots, &rects), None);
        assert_eq!(pointed(c, c + vec2(40.0, 40.0), &slots, &rects), None);
        let right = rects[2].unwrap();
        assert_eq!(pointed(c, right.center(), &slots, &rects), None);
        // 項目の箱の上なら、向きにかかわらずその項目
        let top = rects[0].unwrap();
        assert_eq!(
            pointed(c, top.right_top() + vec2(-2.0, 2.0), &slots, &rects),
            Some(0)
        );
    }

    #[test]
    fn a_pie_at_the_screen_edge_moves_inside() {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let size = vec2(80.0, ITEM_HEIGHT);
        let c = pos2(5.0, 5.0);
        let rects: Vec<Rect> = (0..SLOTS).map(|i| item_rect(c, i, size)).collect();
        let moved = fit_center(c, &rects, screen);
        for i in 0..SLOTS {
            let r = item_rect(moved, i, size);
            assert!(screen.contains_rect(r), "{i}: {r:?}");
        }
        // 真ん中に開いたものは動かさない
        let c = screen.center();
        let rects: Vec<Rect> = (0..SLOTS).map(|i| item_rect(c, i, size)).collect();
        assert_eq!(fit_center(c, &rects, screen), c);
    }
}
