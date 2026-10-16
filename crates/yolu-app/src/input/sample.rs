//! マウスのイベントとペンの点を、ビューに渡す前に 1 つの形（`PointerSample`）にする。フレームの入力は `Stream` が、ペンの点を先に、
//! egui のイベントを後に、来た順で 1 本に並べる（マウスの点の時刻は `MouseClock`。描く点になりうるイベントごとに 1 つ進める）。
//! ペンの押しの間（`pen_frame`）は、同じ押しが egui のポインタの押しとしても届くので、マウスの点に印を付ける（ビューはその押しを使わない）。
//! 値は、ペンが OS の点（筆圧は調整済み）・時刻 `time_ms / 1000`、マウスが Touch の力（無ければ 1）・時刻 `MouseClock`。

use egui::{Event, Pos2};

use super::Press;
use crate::engine::Tilt;
use crate::gesture::MouseClock;
use crate::pen::PenSample;
use crate::state::key::{Modifiers, PointerButton};
use crate::state::StrokeSource;

/// 点の段階。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// 押した（マウスのボタン・ペンの最初の接触）。
    Press,
    /// 動いた（ペンは押している間。マウスはボタンによらない）。
    Move,
    /// 離した。`lost` は OS に押しを奪われて補った離し（本物の離しではない。`AppState::pen_release_lost`）。
    Release { lost: bool },
    /// 押しに使わない点（浮いているペン、ほかのペンが押している間のペン）。`contact` は紙に触れているか。
    Idle { contact: bool },
}

impl Phase {
    /// 紙に触れている点か（ペン）。
    pub fn contact(self) -> bool {
        matches!(
            self,
            Phase::Press | Phase::Move | Phase::Idle { contact: true }
        )
    }
}

/// ビューに渡す 1 つの点（マウスもペンも）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerSample {
    pub source: StrokeSource,
    pub phase: Phase,
    /// egui の点（論理の座標）。
    pub pos: Pos2,
    /// 押し・離しのボタン（ペンはサイドボタンで右ボタン、ペン先で左ボタン。マウスの動きは左ボタン）。
    pub button: PointerButton,
    /// マウスの押し・離しはイベントの修飾、マウスの動きとペンはフレームの修飾。
    pub modifiers: Modifiers,
    /// 0〜1。ペンは調整済みの筆圧、マウスは Touch の力（無ければ 1）。
    pub pressure: f32,
    pub tilt: Tilt,
    /// ペンの軸まわりの回転（度。送れないペン・マウスは None）。
    pub rotation: Option<f32>,
    /// ペンの消しゴムの端。
    pub eraser: bool,
    /// 秒。
    pub time: f64,
    /// マウスの点が、ペンの押しの間に来た（同じ押しの egui のポインタの代わりの入力）。
    pub pen_frame: bool,
}

impl PointerSample {
    /// ペンの点（`press` はこのビューの今のペンの押し、`lost` はこの点が補った離しか）。
    pub fn pen(
        s: &PenSample,
        pixels_per_point: f32,
        press: Option<Press>,
        lost: bool,
        modifiers: Modifiers,
    ) -> PointerSample {
        let source = StrokeSource::Pen(s.pointer_id);
        let phase = match press {
            Some(press) if press.source == source => {
                if s.contact {
                    Phase::Move
                } else {
                    Phase::Release { lost }
                }
            }
            None if s.contact => Phase::Press,
            // ほかのペン（別の ID）が押している間・浮いているだけ
            _ => Phase::Idle { contact: s.contact },
        };
        PointerSample {
            source,
            phase,
            pos: s.pos_points(pixels_per_point),
            button: if s.barrel {
                PointerButton::Secondary
            } else {
                PointerButton::Primary
            },
            modifiers,
            pressure: s.pressure,
            tilt: s.tilt,
            rotation: s.rotation,
            eraser: s.eraser,
            time: s.time_ms as f64 / 1000.0,
            pen_frame: true,
        }
    }

    /// マウスの押し・離し・動き（ほかのイベントは None）。`pressure` は Touch の力（無ければ 1）、`frame_modifiers` は動きに添える修飾。
    pub fn mouse(
        event: &Event,
        time: f64,
        pressure: f32,
        frame_modifiers: Modifiers,
        pen_frame: bool,
    ) -> Option<PointerSample> {
        let (phase, pos, button, modifiers) = match event {
            Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers,
            } => (
                if *pressed {
                    Phase::Press
                } else {
                    Phase::Release { lost: false }
                },
                *pos,
                PointerButton::from(*button),
                Modifiers::from(*modifiers),
            ),
            Event::PointerMoved(pos) => {
                (Phase::Move, *pos, PointerButton::Primary, frame_modifiers)
            }
            _ => return None,
        };
        Some(PointerSample {
            source: StrokeSource::Mouse,
            phase,
            pos,
            button,
            modifiers,
            pressure,
            tilt: Tilt::default(),
            rotation: None,
            eraser: false,
            time,
            pen_frame,
        })
    }
}

/// フレームの入力の 1 つ。
#[derive(Clone, Copy, Debug)]
pub enum Item<'a> {
    /// ペンの点（`PointerSample::pen` で、受ける時のペンの押しから段階を決める）。
    Pen(&'a PenSample),
    /// egui のイベント（`time` は、描く点になりうるイベントならその点の時刻、ほかはフレームの時刻）。マウスの点は `PointerSample::mouse`。
    Event { event: &'a Event, time: f64 },
}

/// フレームの入力の並び: ペンの点が先、egui のイベントが後（来た順）。
pub struct Stream<'a> {
    pen: std::slice::Iter<'a, PenSample>,
    events: std::slice::Iter<'a, Event>,
    clock: MouseClock,
    now: f64,
    /// このフレームはペンの押しの間（ペンの押しが続いている・ペンの点が触れている）。
    pub pen_frame: bool,
}

impl<'a> Stream<'a> {
    /// `pen_pressed` はこのビューにペンの押しが続いているか、`now`・`frame_dt` はフレームの時刻と前のフレームからの時間。
    pub fn new(
        pen: &'a [PenSample],
        events: &'a [Event],
        pen_pressed: bool,
        now: f64,
        frame_dt: f64,
    ) -> Stream<'a> {
        Stream {
            pen: pen.iter(),
            events: events.iter(),
            clock: MouseClock::new(
                now,
                frame_dt,
                events
                    .iter()
                    .filter(|e| crate::gesture::is_mouse_sample_event(e))
                    .count(),
            ),
            now,
            pen_frame: pen_pressed || pen.iter().any(|s| s.contact),
        }
    }
}

impl<'a> Iterator for Stream<'a> {
    type Item = Item<'a>;

    fn next(&mut self) -> Option<Item<'a>> {
        if let Some(s) = self.pen.next() {
            return Some(Item::Pen(s));
        }
        let event = self.events.next()?;
        // 描く点になりうるイベントごとに 1 つずつ進める（描かなくても進める。数えたときと同じ数になる）
        let time = if crate::gesture::is_mouse_sample_event(event) {
            self.clock.next_time()
        } else {
            self.now
        };
        Some(Item::Event { event, time })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    fn pen_at(x: f32, contact: bool, id: u32, t: u32) -> PenSample {
        PenSample {
            pos: [x * 2.0, 20.0],
            pressure: 0.25,
            tilt: Tilt { x: 10.0, y: -5.0 },
            rotation: Some(30.0),
            contact,
            eraser: true,
            barrel: false,
            pointer_id: id,
            time_ms: t,
        }
    }

    fn held(id: u32) -> Option<Press> {
        Some(Press {
            source: StrokeSource::Pen(id),
            owner: super::super::Owner::Tool,
            button: PointerButton::Primary,
            last: pos2(0.0, 0.0),
        })
    }

    #[test]
    fn a_pen_point_keeps_its_values_and_takes_its_phase_from_the_press() {
        let s = PointerSample::pen(
            &pen_at(5.0, true, 3, 1250),
            2.0,
            None,
            false,
            Modifiers::NONE,
        );
        assert_eq!(s.phase, Phase::Press);
        assert_eq!(s.source, StrokeSource::Pen(3));
        assert_eq!(s.pos, pos2(5.0, 10.0));
        assert_eq!((s.pressure, s.rotation, s.eraser), (0.25, Some(30.0), true));
        assert_eq!(s.time, 1.25);
        assert!(s.pen_frame);
        let m = |p: &PenSample, press| PointerSample::pen(p, 2.0, press, false, Modifiers::NONE);
        assert_eq!(m(&pen_at(5.0, true, 3, 0), held(3)).phase, Phase::Move);
        assert_eq!(
            m(&pen_at(5.0, false, 3, 0), held(3)).phase,
            Phase::Release { lost: false }
        );
        let lost = PointerSample::pen(
            &pen_at(5.0, false, 3, 0),
            2.0,
            held(3),
            true,
            Modifiers::NONE,
        );
        assert_eq!(lost.phase, Phase::Release { lost: true });
        // 浮いているだけ・ほかのペンが押している間は押しに使わない
        assert_eq!(
            m(&pen_at(5.0, false, 3, 0), None).phase,
            Phase::Idle { contact: false }
        );
        assert_eq!(
            m(&pen_at(5.0, true, 3, 0), held(4)).phase,
            Phase::Idle { contact: true }
        );
        let barrel = PenSample {
            barrel: true,
            ..pen_at(5.0, true, 3, 0)
        };
        assert_eq!(m(&barrel, None).button, PointerButton::Secondary);
    }

    #[test]
    fn a_mouse_point_takes_the_event_modifiers_on_buttons_and_the_frame_modifiers_on_moves() {
        let press = Event::PointerButton {
            pos: pos2(1.0, 2.0),
            button: egui::PointerButton::Middle,
            pressed: true,
            modifiers: egui::Modifiers::SHIFT,
        };
        let s = PointerSample::mouse(&press, 0.5, 0.75, Modifiers::NONE, false).unwrap();
        assert_eq!((s.phase, s.button), (Phase::Press, PointerButton::Middle));
        assert!(s.modifiers.shift);
        assert_eq!((s.pressure, s.time, s.rotation), (0.75, 0.5, None));
        let moved = Event::PointerMoved(pos2(3.0, 4.0));
        let alt = Modifiers {
            alt: true,
            ..Modifiers::NONE
        };
        let s = PointerSample::mouse(&moved, 0.5, 1.0, alt, true).unwrap();
        assert_eq!(s.phase, Phase::Move);
        assert!(s.modifiers.alt && s.pen_frame);
        assert!(PointerSample::mouse(&Event::WindowFocused(false), 0.5, 1.0, alt, false).is_none());
    }

    #[test]
    fn the_stream_gives_the_pen_first_and_spreads_the_mouse_times_over_the_frame() {
        let pen = [pen_at(1.0, true, 1, 0), pen_at(2.0, false, 1, 10)];
        let events = [
            Event::PointerMoved(pos2(0.0, 0.0)),
            Event::WindowFocused(false),
            Event::PointerMoved(pos2(1.0, 0.0)),
        ];
        let mut stream = Stream::new(&pen, &events, false, 10.0, 0.016);
        assert!(stream.pen_frame);
        assert!(matches!(stream.next(), Some(Item::Pen(s)) if s.pos[0] == 2.0));
        assert!(matches!(stream.next(), Some(Item::Pen(s)) if s.pos[0] == 4.0));
        let times: Vec<f64> = stream
            .map(|item| match item {
                Item::Event { time, .. } => time,
                Item::Pen(_) => unreachable!(),
            })
            .collect();
        assert!((times[0] - 9.992).abs() < 1e-9, "{times:?}");
        assert_eq!(times[1], 10.0, "点にならないイベントはフレームの時刻");
        assert!((times[2] - 10.0).abs() < 1e-9, "{times:?}");
        assert!(!Stream::new(&[], &events, false, 10.0, 0.016).pen_frame);
        assert!(Stream::new(&[], &events, true, 10.0, 0.016).pen_frame);
    }
}
