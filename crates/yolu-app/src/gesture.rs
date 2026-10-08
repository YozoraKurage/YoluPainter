//! ビュー（2D のキャンバス・3D ビュー）への押しの振り分けの、マウスとペンで共通の部分。
//!
//! - 押しの持ち主: 押した所の部品（ドックのタブの見出し・分け目・隅のアイコンなど）が egui の押しを受けているとき、その押しは
//!   ビューのものではない。押し始めを決めたら、離すまで変えない（見出しをつかんだままペンをキャンバスへ動かしても、そこから描き始めない）。
//! - Ctrl+Space の拡縮（Photoshop・CLIP STUDIO と同じ）: 押しながら左右にドラッグすると拡大・縮小、動かさずに離すと拡大、Alt も押していれば縮小。

use egui::{Event, Modifiers, PointerButton, Pos2, Response};

/// 動かさずに離したとみなす、押してから離すまでに動いてよい距離（画面の点）。
pub const CLICK_MOVE: f32 = 4.0;

/// Ctrl（Mac の Command も）を押しているか。
pub fn ctrl(m: &Modifiers) -> bool {
    m.ctrl || m.command
}

/// ペン先の接触を描かずに捨てるか（描くツールで Ctrl を押している。マウスの Ctrl は描く。2D のキャンバスも 3D ビューも同じ）。
pub fn pen_holds_off(paints: bool, m: &Modifiers) -> bool {
    paints && ctrl(m)
}

/// Ctrl+Space の拡縮の組み合わせか（`space` は Space を押しているか）。
pub fn zoom_chord(m: &Modifiers, space: bool) -> bool {
    space && ctrl(m)
}

/// この押しを egui が別の部品（ドックの見出し・分け目・ビューの上に重ねたアイコンなど）の押しとして受けているか。`response` は
/// ビューの入力の応答。egui が何の押しも受けていなければ（ペンの代わりのポインタが無いときなど）偽で、ビューのものとして扱う。
pub fn foreign_press(ctx: &egui::Context, response: &Response) -> bool {
    // ウィンドウの縁の押し（大きさを変える）は、縁がビューの端に重なっていても、ビューのものではない
    crate::titlebar::edge_press_held(ctx)
        || (ctx.egui_is_using_pointer() && !response.is_pointer_button_down_on())
}

/// Ctrl+Space のドラッグ（押した点・前の位置・動いた距離）。押した点は 2D の拡縮の中心。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomDrag {
    pub anchor: Pos2,
    pub last: Pos2,
    /// 押してから動いた距離の合計（クリックかドラッグかを分ける）。
    pub moved: f32,
    /// Alt を押して押した（クリックなら縮小）。
    pub out: bool,
}

impl ZoomDrag {
    pub fn new(anchor: Pos2, out: bool) -> ZoomDrag {
        ZoomDrag {
            anchor,
            last: anchor,
            moved: 0.0,
            out,
        }
    }

    /// 位置が動いた。横に動いた量（右が正）を返す。
    pub fn moved_to(&mut self, pos: Pos2) -> f32 {
        let d = pos - self.last;
        self.last = pos;
        self.moved += d.length();
        d.x
    }

    /// 動かさずに離した（クリック）か。
    pub fn is_click(&self) -> bool {
        self.moved < CLICK_MOVE
    }
}

/// 2D の拡縮: ドラッグ 1 点あたりの対数の拡大率（100 点で約 2.2 倍）。
pub const ZOOM_PER_POINT: f32 = 0.008;
/// クリック 1 回の拡大・縮小の倍率。
pub const CLICK_ZOOM: f32 = 2.0;
/// 3D の拡縮: ドラッグの点数を、ホイールの目盛りに直す割り（40 点で 1 目盛り）。
pub const POINTS_PER_NOTCH: f32 = 40.0;
/// 3D のクリック 1 回の寄る・引く（ホイールの目盛り）。
pub const CLICK_NOTCHES: f32 = 3.0;

/// マウスで描く点になりうるイベント（押す・動く）。
pub fn is_mouse_sample_event(event: &Event) -> bool {
    matches!(
        event,
        Event::PointerMoved(_)
            | Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                ..
            }
    )
}

/// マウスの点の時刻（2D のキャンバスと 3D ビューで共通）。egui のイベントには時刻が無く、1 フレームの全イベントに `now` を付けると、時刻が進まない点では core が速さを
/// 前の値のままにするので、1 フレームに N 個のイベントがあれば速さが本当の約 1/N になる。そこで前のフレームから `now` までを
/// そのフレームのイベントの数で等分し、単調に増える時刻を付ける（最後のイベントが `now`）。長く止まったあとの最初のフレームで
/// 速さが極端に遅く見えないよう、間隔は `MAX_FRAME_GAP` までに抑える。
pub struct MouseClock {
    now: f64,
    start: f64,
    step: f64,
    index: usize,
}

impl MouseClock {
    pub const MAX_FRAME_GAP: f64 = 0.1;

    pub fn new(now: f64, frame_dt: f64, events: usize) -> MouseClock {
        let dt = frame_dt.clamp(0.0, Self::MAX_FRAME_GAP);
        MouseClock {
            now,
            start: now - dt,
            step: dt / events.max(1) as f64,
            index: 0,
        }
    }

    pub fn next_time(&mut self) -> f64 {
        self.index += 1;
        (self.start + self.step * self.index as f64).min(self.now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    #[test]
    fn ctrl_means_ctrl_or_command() {
        assert!(ctrl(&Modifiers::CTRL));
        assert!(ctrl(&Modifiers::COMMAND));
        assert!(!ctrl(&Modifiers::SHIFT));
        assert!(zoom_chord(&Modifiers::CTRL, true));
        assert!(!zoom_chord(&Modifiers::CTRL, false), "Space が要る");
        assert!(!zoom_chord(&Modifiers::NONE, true), "Ctrl が要る");
    }

    #[test]
    fn a_zoom_drag_tells_a_click_from_a_drag() {
        let mut z = ZoomDrag::new(pos2(10.0, 10.0), false);
        assert!(z.is_click());
        assert_eq!(z.moved_to(pos2(12.0, 10.0)), 2.0);
        assert!(z.is_click(), "4 点より少ない動きはクリック");
        assert_eq!(z.moved_to(pos2(20.0, 10.0)), 8.0);
        assert!(!z.is_click());
        // 戻ってきても、動いた距離は減らない
        z.moved_to(pos2(10.0, 10.0));
        assert!(!z.is_click());
    }

    #[test]
    fn mouse_times_spread_over_the_frame_and_end_at_now() {
        let mut clock = MouseClock::new(10.0, 0.016, 4);
        let times: Vec<f64> = (0..4).map(|_| clock.next_time()).collect();
        assert!(times.windows(2).all(|w| w[1] > w[0]), "{times:?}");
        assert!((times[0] - 9.988).abs() < 1e-9, "{times:?}");
        assert!((times[3] - 10.0).abs() < 1e-9, "{times:?}");
    }

    #[test]
    fn a_long_pause_does_not_make_the_first_frame_look_slow() {
        let mut clock = MouseClock::new(100.0, 30.0, 2);
        let (a, b) = (clock.next_time(), clock.next_time());
        assert!(b - a <= MouseClock::MAX_FRAME_GAP, "{a} {b}");
        assert!((b - 100.0).abs() < 1e-9);
    }

    #[test]
    fn no_events_or_a_negative_gap_stay_at_now() {
        let mut clock = MouseClock::new(5.0, -1.0, 0);
        assert_eq!(clock.next_time(), 5.0);
        assert_eq!(
            clock.next_time(),
            5.0,
            "数えた数より多く呼んでも now を越えない"
        );
    }
}
