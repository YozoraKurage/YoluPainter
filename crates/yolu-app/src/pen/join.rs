//! Windows Ink のペンの浮きと本物のマウスが、egui の 1 つのポインタに混ざる所を直す（`PointerJoin`。`eframe::App::raw_input_hook` が、
//! Windows のメインウィンドウで毎フレーム、ペンの記録のあとに呼ぶ）。
//!
//! winit は浮いているペンの WM_POINTERUPDATE も Touch の Moved にし、egui-winit は Touch を積んだ直後に、同じ位置のポインタの動き（`PointerMoved`）も積む。
//! winit は WM_POINTER を既定の処理へ渡さないので、OS のカーソルはペンについて来ない。ほかのデバイスの本物のマウスのボタン・動きは、止まったカーソルの位置で届く
//! （押しの位置は egui-winit の共有のポインタの位置なので、直前に浮いていたペンの位置になることもある）。egui のポインタはペンの位置とカーソルの位置を行き来し、
//! ビューを動かす操作（前の位置との差を積む）は「最後の位置 − 押した所」になって、カーソルの位置の点が来るたびに元へ戻る。
//!
//! 本物のマウスのボタンを押している間（ペンは触れていない）で、押したフレームか、2D・3D のビューがその押しで回す・パン・拡縮している間だけ、
//! ポインタの動きを出どころ（ペンの Touch・マウス）ごとの差に分け、egui に渡す位置へ積む（押したフレームは、押しの行き先をまだ知らないので、押しのあとの点から積む。
//! 次のフレームからは、押しがビューを動かしているときだけ）。止まったカーソルの点は差が 0 なので戻らない。浮いたペンの動きと、本物のマウスの動きは、
//! それぞれの分だけ積む。ただし、直近のペンの点（`GAP` 秒の間の最後の `RECENT` 点）のどれかから `FOLLOW` 点以内にあるマウスの点は、差を 0 にする
//! （カーソルがペンについて来る形で、同じ動きを二重に積まない）。ペンの点は、前のペンの点から `GAP` 秒より空いていれば差を 0 にする
//! （範囲の外から戻ったペンで、大きく動かさない）。空いたかは、フレームの時刻と、ペンの受け口の点の OS の時刻の両方で見る（どちらかが近ければ続き。
//! 1 フレームが長くても、続けて浮いていたペンの差を捨てない）。ペンの点が来なければ、マウスの位置はそのまま。
//! それ以外（ボタンを押していない・ペンが触れている・押しがビューを動かしていない）は、何も変えない。

use egui::{Event, Pos2, TouchId, TouchPhase, Vec2};

/// 点の出どころ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    /// egui-winit が Touch（Windows Ink のペン・指）から作った動き。
    Touch,
    /// 本物のマウス。
    Mouse,
}

/// イベントの見分け。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Move(Origin),
    /// 本物のマウスのボタン（Touch から作った押し・離しではない物）。
    MouseButton {
        bit: u8,
        pressed: bool,
    },
    /// egui-winit がポインタにする Touch の始まり（触れた）。
    TouchStart(TouchId),
    /// Touch の終わり・取りやめ（離した）。
    TouchEnd(TouchId),
    FocusLost,
    Other,
}

/// フレームのイベントを見分ける。Touch から作った動きは、直前の `Event::Touch`（始まり・動き）と同じ位置の `PointerMoved`
/// （egui-winit は同じ Touch の位置から両方を作るので、値まで同じ）。Touch から作った押しは「Touch の始まり → 同じ位置の動き → 押し」の並び、
/// 離しは Touch の終わりの直後。
fn classify(events: &[Event]) -> Vec<Kind> {
    let mut kinds: Vec<Kind> = Vec::with_capacity(events.len());
    for (i, event) in events.iter().enumerate() {
        let previous = i.checked_sub(1).map(|j| &events[j]);
        let kind = match event {
            Event::PointerMoved(p) => match previous {
                Some(Event::Touch {
                    phase: TouchPhase::Start | TouchPhase::Move,
                    pos,
                    ..
                }) if pos == p => Kind::Move(Origin::Touch),
                _ => Kind::Move(Origin::Mouse),
            },
            Event::PointerButton {
                button, pressed, ..
            } => {
                let from_touch = if *pressed {
                    kinds.last() == Some(&Kind::Move(Origin::Touch))
                        && i >= 2
                        && matches!(
                            events[i - 2],
                            Event::Touch {
                                phase: TouchPhase::Start,
                                ..
                            }
                        )
                } else {
                    matches!(
                        previous,
                        Some(Event::Touch {
                            phase: TouchPhase::End,
                            ..
                        })
                    )
                };
                if from_touch {
                    Kind::Other
                } else {
                    Kind::MouseButton {
                        bit: 1 << (*button as u8),
                        pressed: *pressed,
                    }
                }
            }
            Event::Touch {
                phase: TouchPhase::Start,
                pos,
                id,
                ..
            } if matches!(events.get(i + 1), Some(Event::PointerMoved(p)) if p == pos) => {
                Kind::TouchStart(*id)
            }
            Event::Touch {
                phase: TouchPhase::End | TouchPhase::Cancel,
                id,
                ..
            } => Kind::TouchEnd(*id),
            Event::WindowFocused(false) => Kind::FocusLost,
            _ => Kind::Other,
        };
        kinds.push(kind);
    }
    kinds
}

/// ペンの点 1 つ（egui の点・フレームの時刻・何回目の `apply` で来たか）。
#[derive(Clone, Copy, Debug)]
struct PenPoint {
    pos: Pos2,
    time: f64,
    frame: u64,
}

/// ペンの浮きと本物のマウスの動きを、本物のマウスのボタンがビューを動かしている間だけ、出どころごとの差にして 1 本の位置に積む。
#[derive(Clone, Copy, Debug, Default)]
pub struct PointerJoin {
    /// 押している本物のマウスのボタン（`PointerButton` の番号のビット）。
    held: u8,
    /// egui-winit がポインタにした Touch の番号（触れた始まりから、その番号の終わり・取りやめまで。ほかの指の離しでは下ろさない）。
    touching: Option<TouchId>,
    /// 直近のペンの点（新しい順。マウスの点がペンについて来たかを見る）。
    recent: [Option<PenPoint>; Self::RECENT],
    /// マウスの最後の点の位置。
    mouse: Option<Pos2>,
    /// egui に渡した最後のポインタの位置。
    out: Option<Pos2>,
    /// `apply` の回数（フレームの番号）。
    frame: u64,
    /// 前までのフレームの、ペンの受け口の最後の点の OS の時刻（ミリ秒）。
    pen_ms: Option<u32>,
}

impl PointerJoin {
    /// 前のペンの点からこれより空いていなければ、ペンは続けて範囲にいる（秒）。空いたペンの点は差を 0 にする。浮いたペンは、これよりずっと短い間で点を送る。
    pub const GAP: f64 = 0.2;
    /// マウスの点がペンについて来たかを見る、直近のペンの点の数（マウスとペンのメッセージの順が前後しても拾う）。
    pub const RECENT: usize = 4;
    /// マウスの点がペンの点からこれ以内なら、カーソルがペンについて来たと見る（egui の論理の点）。winit は Touch の位置に画素の端数を残し
    /// （`floor` した画素に `fract` を足す）、WM_MOUSEMOVE は整数の画素なので、同じ所でも 1 画素まで違う。ドライバーと OS の座標の写しの丸めで
    /// もう 1〜2 画素ずれうるので、高い DPI でも収まる 4 点にした（本物のマウスの動きを捨てるのは、ペンの点のすぐそばを通る時だけ）。
    pub const FOLLOW: f32 = 4.0;

    /// events を直す。`moving_view` は、本物のマウスのボタンの押しが 2D・3D のビューを動かしているか（前のフレームの終わりの状態。
    /// 押したフレームは、押しのあとの点から積む）。`now` はフレームの時刻（秒）。`pen_ms` は、このフレームのペンの受け口の点の、最初と最後の OS の時刻
    /// （ミリ秒。点が無ければ None）。
    pub fn apply(
        &mut self,
        events: &mut [Event],
        moving_view: bool,
        now: f64,
        pen_ms: Option<(u32, u32)>,
    ) {
        self.frame += 1;
        // OS の時刻で、前のフレームのペンの点から続けて浮いている（長いフレームでも、ペンの点が続いていれば差を捨てない）
        let os_fresh = match (pen_ms, self.pen_ms) {
            (Some((first, _)), Some(last)) => {
                f64::from(first.wrapping_sub(last)) / 1000.0 <= Self::GAP
            }
            _ => false,
        };
        let kinds = classify(events);
        let mut pressed_here = false;
        for (event, kind) in events.iter_mut().zip(kinds) {
            let joining =
                self.held != 0 && self.touching.is_none() && (moving_view || pressed_here);
            match (kind, event) {
                (Kind::Move(origin), Event::PointerMoved(p)) => {
                    *p = self.moved(*p, origin, now, os_fresh, joining);
                }
                (Kind::MouseButton { bit, pressed }, Event::PointerButton { pos, .. }) => {
                    // 積んでいる間の押し・離しは、積んだ位置で（ビューの押しの所・離しの所が、動きと同じ道になる）
                    if joining {
                        if let Some(out) = self.out {
                            *pos = out;
                        }
                    }
                    if pressed {
                        self.held |= bit;
                        pressed_here = true;
                    } else {
                        self.held &= !bit;
                    }
                }
                (Kind::TouchStart(id), _) => self.touching = Some(id),
                (Kind::TouchEnd(id), _) => {
                    if self.touching == Some(id) {
                        self.touching = None;
                    }
                }
                (Kind::FocusLost, _) => {
                    self.held = 0;
                    self.touching = None;
                }
                _ => {}
            }
        }
        if let Some((_, last)) = pen_ms {
            self.pen_ms = Some(last);
        }
    }

    /// ペンの点 `q` が、このフレームから見て続きか（同じフレーム・`GAP` 秒以内・OS の時刻で続いている前のフレーム）。
    fn fresh(&self, q: &PenPoint, now: f64, os_fresh: bool) -> bool {
        q.frame == self.frame
            || now - q.time <= Self::GAP
            || (os_fresh && q.frame + 1 == self.frame)
    }

    /// 動きの点 `p` を受け、egui に渡す位置を返す。
    fn moved(&mut self, p: Pos2, origin: Origin, now: f64, os_fresh: bool, joining: bool) -> Pos2 {
        let d = match origin {
            Origin::Touch => {
                let before = self.recent[0].filter(|q| self.fresh(q, now, os_fresh));
                self.recent.rotate_right(1);
                self.recent[0] = Some(PenPoint {
                    pos: p,
                    time: now,
                    frame: self.frame,
                });
                before.map_or(Vec2::ZERO, |q| p - q.pos)
            }
            Origin::Mouse => {
                let before = self.mouse.replace(p);
                // カーソルがペンについて来た点（直近のペンの点のそば）は、ペンの動きと同じなので積まない
                let follows = self.recent.iter().flatten().any(|q| {
                    (q.frame == self.frame || now - q.time <= Self::GAP)
                        && q.pos.distance(p) <= Self::FOLLOW
                });
                if follows {
                    Vec2::ZERO
                } else {
                    before.map_or(Vec2::ZERO, |q| p - q)
                }
            }
        };
        let out = match (joining, self.out) {
            (true, Some(out)) => out + d,
            _ => p,
        };
        self.out = Some(out);
        out
    }

    /// 本物のマウスのボタンを押している（テスト用）。
    pub fn mouse_held(&self) -> bool {
        self.held != 0
    }
}

impl crate::state::AppState {
    /// 本物のマウスのボタンの押しが、2D のキャンバスか 3D ビューを動かしている（回す・パン・拡縮。ペンが動かしている物は除く）。
    pub fn mouse_moves_view(&self) -> bool {
        let pen_view = |p: &Option<crate::input::Press>| {
            p.is_some_and(|p| p.owner == crate::input::Owner::View)
        };
        (self.view3d.view.input.nav.is_some() && !pen_view(&self.view3d.view.input.pen_press))
            || (self.canvas.nav_button.is_some() && !pen_view(&self.canvas.pen_press))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, Modifiers, PointerButton, TouchDeviceId};

    fn touch(phase: TouchPhase, at: Pos2) -> Event {
        Event::Touch {
            device_id: TouchDeviceId(0),
            id: TouchId(5),
            phase,
            pos: at,
            force: None,
        }
    }

    /// ペンの浮きの点（egui-winit が積む Touch と、同じ位置のポインタの動き）。
    fn hover(at: Pos2) -> [Event; 2] {
        [touch(TouchPhase::Move, at), Event::PointerMoved(at)]
    }

    fn button(at: Pos2, button: PointerButton, pressed: bool) -> Event {
        Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    fn moves(events: &[Event]) -> Vec<Pos2> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::PointerMoved(p) => Some(*p),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_pointer_move_right_after_a_touch_at_the_same_place_is_the_touch_and_others_are_the_mouse()
    {
        let a = pos2(10.0, 10.0);
        let events = [
            touch(TouchPhase::Move, a),
            Event::PointerMoved(a),
            Event::PointerMoved(pos2(1.0, 1.0)),
            touch(TouchPhase::Move, a),
            Event::PointerMoved(pos2(11.0, 10.0)),
            touch(TouchPhase::Start, a),
            Event::PointerMoved(a),
            button(a, PointerButton::Primary, true),
            button(a, PointerButton::Secondary, true),
            touch(TouchPhase::End, a),
            button(a, PointerButton::Primary, false),
            button(a, PointerButton::Secondary, false),
        ];
        let kinds = classify(&events);
        assert_eq!(kinds[1], Kind::Move(Origin::Touch));
        assert_eq!(kinds[2], Kind::Move(Origin::Mouse));
        assert_eq!(
            kinds[4],
            Kind::Move(Origin::Mouse),
            "位置の違う動きはマウス"
        );
        assert_eq!(kinds[5], Kind::TouchStart(TouchId(5)));
        assert_eq!(kinds[7], Kind::Other, "Touch から作った押し");
        assert_eq!(
            kinds[8],
            Kind::MouseButton {
                bit: 2,
                pressed: true
            }
        );
        assert_eq!(kinds[9], Kind::TouchEnd(TouchId(5)));
        assert_eq!(kinds[10], Kind::Other, "Touch から作った離し");
        assert_eq!(
            kinds[11],
            Kind::MouseButton {
                bit: 2,
                pressed: false
            }
        );
    }

    #[test]
    fn while_a_mouse_button_moves_the_view_each_origin_adds_its_own_motion_and_a_following_cursor_adds_nothing(
    ) {
        let b = pos2(100.0, 100.0);
        let mut join = PointerJoin::default();
        // 止まったカーソル B で右ボタンを押し、ペンの浮き（A）と B の点が交互に来る
        let mut events = vec![
            Event::PointerMoved(b),
            button(b, PointerButton::Secondary, true),
        ];
        join.apply(&mut events, false, 0.0, None);
        assert!(join.mouse_held());
        let mut frame = Vec::new();
        frame.extend(hover(pos2(300.0, 100.0)));
        frame.push(Event::PointerMoved(b));
        frame.extend(hover(pos2(310.0, 100.0)));
        frame.push(Event::PointerMoved(b));
        frame.extend(hover(pos2(320.0, 105.0)));
        join.apply(&mut frame, true, 0.01, None);
        // ペンが範囲に入った最初の点は差 0（前のペンの点が無い）、そのあとペンの分だけ。B の点は差 0
        assert_eq!(
            moves(&frame),
            vec![
                b,
                b,
                pos2(110.0, 100.0),
                pos2(110.0, 100.0),
                pos2(120.0, 105.0)
            ]
        );
        // カーソルがペンについて来る（ペンの点のそばのマウスの点。1 点遅れても）: 二重に積まない
        let mut frame: Vec<Event> = [
            hover(pos2(330.0, 105.0)).to_vec(),
            vec![Event::PointerMoved(pos2(330.4, 105.0))],
            hover(pos2(340.0, 105.0)).to_vec(),
            hover(pos2(350.0, 106.0)).to_vec(),
            vec![Event::PointerMoved(pos2(339.0, 104.0))],
        ]
        .concat();
        join.apply(&mut frame, true, 0.02, None);
        assert_eq!(
            moves(&frame),
            vec![
                pos2(130.0, 105.0),
                pos2(130.0, 105.0),
                pos2(140.0, 105.0),
                pos2(150.0, 106.0),
                pos2(150.0, 106.0)
            ]
        );
        // ペンが範囲を出たら、本物のマウスを動かした分は動く
        let t = 0.02 + PointerJoin::GAP + 0.01;
        let mut frame = vec![Event::PointerMoved(pos2(343.0, 102.0))];
        join.apply(&mut frame, true, t, None);
        assert_eq!(moves(&frame), vec![pos2(154.0, 104.0)]);
        // 離しは積んだ位置で。離したあとは何も変えない
        let mut frame = vec![
            button(pos2(320.0, 105.0), PointerButton::Secondary, false),
            Event::PointerMoved(pos2(321.0, 105.0)),
        ];
        join.apply(&mut frame, true, t + 0.01, None);
        assert!(matches!(frame[0], Event::PointerButton { pos, .. } if pos == pos2(154.0, 104.0)));
        assert_eq!(moves(&frame), vec![pos2(321.0, 105.0)]);
        assert!(!join.mouse_held());
    }

    /// 浮いたペンのすぐあとの本物の押しは、egui-winit の共有の位置（ペンの位置 A）で届く。そのあと止まったカーソル B の点が来ても、A から B へ飛ばない。
    #[test]
    fn a_press_at_the_hovering_pen_does_not_jump_to_the_still_cursor() {
        let a = pos2(300.0, 100.0);
        let b = pos2(100.0, 100.0);
        let mut join = PointerJoin::default();
        let mut frame = vec![Event::PointerMoved(b)];
        join.apply(&mut frame, false, 0.0, None);
        let mut frame: Vec<Event> = [
            hover(a).to_vec(),
            vec![
                button(a, PointerButton::Secondary, true),
                Event::PointerMoved(b),
            ],
            hover(pos2(306.0, 100.0)).to_vec(),
        ]
        .concat();
        join.apply(&mut frame, false, 0.1, None);
        assert!(matches!(frame[2], Event::PointerButton { pos, .. } if pos == a));
        assert_eq!(moves(&frame), vec![a, a, pos2(306.0, 100.0)]);
    }

    /// ペンを範囲に置いたまま止めて（同じ位置の浮きの点が来続ける）、離れた所でマウスを動かすと、マウスの分だけ動く。
    #[test]
    fn the_mouse_moves_the_view_while_a_still_pen_keeps_hovering_elsewhere() {
        let a = pos2(300.0, 300.0);
        let b = pos2(100.0, 100.0);
        let mut join = PointerJoin::default();
        let mut events = vec![
            Event::PointerMoved(b),
            button(b, PointerButton::Secondary, true),
        ];
        join.apply(&mut events, false, 0.0, None);
        let mut out = Vec::new();
        for (i, m) in [pos2(104.0, 98.0), pos2(108.0, 96.0), pos2(112.0, 94.0)]
            .into_iter()
            .enumerate()
        {
            let mut frame: Vec<Event> = [hover(a).to_vec(), vec![Event::PointerMoved(m)]].concat();
            join.apply(&mut frame, true, 0.01 * (i + 1) as f64, None);
            out.extend(moves(&frame));
        }
        assert_eq!(
            out,
            vec![
                b,
                pos2(104.0, 98.0),
                pos2(104.0, 98.0),
                pos2(108.0, 96.0),
                pos2(108.0, 96.0),
                pos2(112.0, 94.0)
            ]
        );
    }

    /// 1 フレームが `GAP` 秒より長くても、ペンの受け口の点の OS の時刻が続いていれば、ペンの差は捨てない。OS の時刻も空いていれば捨てる。
    #[test]
    fn a_long_frame_keeps_the_pen_motion_when_the_os_times_go_on() {
        let b = pos2(100.0, 100.0);
        let mut join = PointerJoin::default();
        let mut events = vec![
            Event::PointerMoved(b),
            button(b, PointerButton::Middle, true),
        ];
        join.apply(&mut events, false, 0.0, None);
        let mut frame: Vec<Event> = hover(pos2(300.0, 100.0)).to_vec();
        join.apply(&mut frame, true, 0.01, Some((1000, 1000)));
        // 長いフレームのあと（フレームの時刻で 0.5 秒）。ペンの点は 8 ms ごとに来ていた
        let mut frame: Vec<Event> = hover(pos2(310.0, 100.0)).to_vec();
        join.apply(&mut frame, true, 0.51, Some((1008, 1400)));
        assert_eq!(moves(&frame), vec![pos2(110.0, 100.0)]);
        // OS の時刻も空いた（範囲の外から戻った）
        let mut frame: Vec<Event> = hover(pos2(600.0, 400.0)).to_vec();
        join.apply(&mut frame, true, 1.2, Some((2100, 2100)));
        assert_eq!(moves(&frame), vec![pos2(110.0, 100.0)]);
    }

    /// ポインタにした Touch（ペン）が触れている間に、ほかの指の Touch が離れても、触れている扱いは続く（積みを始めない）。
    #[test]
    fn another_touch_ending_does_not_end_the_pen_contact() {
        let b = pos2(100.0, 100.0);
        let a = pos2(300.0, 100.0);
        let mut join = PointerJoin::default();
        let mut events = vec![
            Event::PointerMoved(b),
            button(b, PointerButton::Secondary, true),
        ];
        join.apply(&mut events, false, 0.0, None);
        let other = |phase| Event::Touch {
            device_id: TouchDeviceId(0),
            id: TouchId(9),
            phase,
            pos: pos2(50.0, 50.0),
            force: None,
        };
        let mut frame = vec![
            touch(TouchPhase::Start, a),
            Event::PointerMoved(a),
            button(a, PointerButton::Primary, true),
            other(TouchPhase::Start),
            other(TouchPhase::End),
            Event::PointerMoved(pos2(105.0, 100.0)),
            touch(TouchPhase::Move, pos2(320.0, 100.0)),
            Event::PointerMoved(pos2(320.0, 100.0)),
        ];
        let before = frame.clone();
        join.apply(&mut frame, true, 0.01, None);
        assert_eq!(frame, before, "ペンが触れている間はそのまま");
    }

    #[test]
    fn a_pen_point_after_a_gap_adds_nothing() {
        let b = pos2(100.0, 100.0);
        let mut join = PointerJoin::default();
        let mut events = vec![
            Event::PointerMoved(b),
            button(b, PointerButton::Middle, true),
        ];
        join.apply(&mut events, false, 0.0, None);
        let mut frame: Vec<Event> = [hover(pos2(300.0, 100.0)), hover(pos2(305.0, 100.0))].concat();
        join.apply(&mut frame, true, 0.01, None);
        assert_eq!(moves(&frame), vec![b, pos2(105.0, 100.0)]);
        // ペンが範囲の外へ出て、離れた所から戻る: 戻った最初の点は差 0
        let mut frame: Vec<Event> = [hover(pos2(600.0, 400.0)), hover(pos2(602.0, 400.0))].concat();
        join.apply(&mut frame, true, 0.01 + PointerJoin::GAP + 0.05, None);
        assert_eq!(moves(&frame), vec![pos2(105.0, 100.0), pos2(107.0, 100.0)]);
    }

    /// ペンの点が来ないなら、ボタンを押してビューを動かしている間も（間をあけて動かしても）、マウスの位置はそのまま。
    #[test]
    fn the_mouse_alone_keeps_its_positions_while_it_moves_the_view() {
        let b = pos2(100.0, 100.0);
        let mut join = PointerJoin::default();
        let mut events = vec![
            Event::PointerMoved(b),
            button(b, PointerButton::Secondary, true),
            Event::PointerMoved(pos2(103.0, 100.0)),
        ];
        let before = events.clone();
        join.apply(&mut events, false, 0.0, None);
        assert_eq!(events, before);
        for (t, p) in [(0.01, pos2(110.0, 104.0)), (1.0, pos2(160.0, 90.0))] {
            let mut frame = vec![Event::PointerMoved(p)];
            join.apply(&mut frame, true, t, None);
            assert_eq!(moves(&frame), vec![p]);
        }
    }

    #[test]
    fn without_a_mouse_button_or_with_the_pen_touching_or_without_a_view_move_nothing_changes() {
        let b = pos2(100.0, 100.0);
        // ボタンを押していない: ペンの浮きもマウスもそのまま（ペンのホバー）
        let mut join = PointerJoin::default();
        let mut frame: Vec<Event> = [
            hover(pos2(300.0, 100.0)).to_vec(),
            vec![Event::PointerMoved(b)],
            hover(pos2(310.0, 100.0)).to_vec(),
        ]
        .concat();
        let before = frame.clone();
        join.apply(&mut frame, true, 0.0, None);
        assert_eq!(frame, before);
        // 押しがビューを動かしていない（パネルの右クリックなど）: 押したフレームのあとは、そのまま
        let mut events = vec![button(b, PointerButton::Secondary, true)];
        join.apply(&mut events, false, 0.0, None);
        let mut frame: Vec<Event> = [hover(pos2(300.0, 100.0)), hover(pos2(310.0, 100.0))].concat();
        let before = frame.clone();
        join.apply(&mut frame, false, 0.01, None);
        assert_eq!(frame, before);
        // ペンが触れている間は、そのまま（ペンの押しはペンの点が持つ）
        let a = pos2(300.0, 100.0);
        let mut frame = vec![
            touch(TouchPhase::Start, a),
            Event::PointerMoved(a),
            button(a, PointerButton::Primary, true),
            touch(TouchPhase::Move, pos2(320.0, 100.0)),
            Event::PointerMoved(pos2(320.0, 100.0)),
        ];
        let before = frame.clone();
        join.apply(&mut frame, true, 0.02, None);
        assert_eq!(frame, before);
        // 離して浮くと、ペンの分を触れていた最後の点から積む
        let mut frame: Vec<Event> = [
            vec![
                touch(TouchPhase::End, pos2(320.0, 100.0)),
                button(pos2(320.0, 100.0), PointerButton::Primary, false),
                Event::PointerGone,
            ],
            hover(pos2(325.0, 100.0)).to_vec(),
        ]
        .concat();
        join.apply(&mut frame, true, 0.03, None);
        assert_eq!(moves(&frame), vec![pos2(325.0, 100.0)]);
        // フォーカスを失ったら、押しを忘れる
        let mut frame = vec![Event::WindowFocused(false)];
        join.apply(&mut frame, true, 0.04, None);
        assert!(!join.mouse_held());
    }
}
