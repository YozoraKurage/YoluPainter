//! 調べるための記録（環境変数 `YOLU_PEN_LOG` にファイルのパスがあるときだけ、1 行ずつ追記する。無いときは何もしない）の口。
//! ペンの合図（`win_tab`・`win_ink`）、egui のポインターのイベント、ビューを動かす操作（3D の視点・2D のキャンバス）が同じファイルに入るので、
//! ファイルを開く所は 1 つにして、行の順がそのまま起きた順になるようにする。行の先頭は時刻のミリ秒（Windows は OS の `GetTickCount`、
//! ほかの OS はプロセスの始まりから）。行を作る関数は純関数にして、形が変わったら試験が落ちる。

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::sync::{Mutex, OnceLock};

use egui::{Event, Modifiers, Pos2};

fn file() -> Option<&'static Mutex<BufWriter<File>>> {
    static LOG: OnceLock<Option<Mutex<BufWriter<File>>>> = OnceLock::new();
    LOG.get_or_init(|| {
        let path = std::env::var_os("YOLU_PEN_LOG").filter(|p| !p.is_empty())?;
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
            .map(|f| Mutex::new(BufWriter::new(f)))
    })
    .as_ref()
}

/// 記録が有効か（行を作る前に見る。無効なら行を作らない）。
pub fn enabled() -> bool {
    file().is_some()
}

#[cfg(windows)]
fn tick() -> u32 {
    // SAFETY: 引数無し。
    unsafe { windows::Win32::System::SystemInformation::GetTickCount() }
}

#[cfg(not(windows))]
fn tick() -> u32 {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u32
}

/// 記録に 1 行（先頭は時刻のミリ秒）。書き出しは `flush` まで溜めてよい。
pub(super) fn line(text: &str) {
    if let Some(file) = file() {
        let mut w = file.lock().unwrap_or_else(|e| e.into_inner());
        let _ = writeln!(w, "{} {text}", tick());
    }
}

pub(super) fn flush() {
    if let Some(file) = file() {
        let _ = file.lock().unwrap_or_else(|e| e.into_inner()).flush();
    }
}

/// 記録に 1 行を書いて出す（記録が有効なときにだけ行を作る）。
pub fn event(make: impl FnOnce() -> String) {
    if enabled() {
        line(&make());
        flush();
    }
}

// ───────── egui のポインターのイベント ─────────

fn point(p: Pos2) -> String {
    format!("({:.2},{:.2})", p.x, p.y)
}

fn mods(m: &Modifiers) -> String {
    let mut out = Vec::new();
    if m.ctrl {
        out.push("ctrl");
    }
    if m.shift {
        out.push("shift");
    }
    if m.alt {
        out.push("alt");
    }
    if m.mac_cmd {
        out.push("cmd");
    }
    if out.is_empty() {
        "none".to_string()
    } else {
        out.join("+")
    }
}

/// egui の入力のうちポインターに関わるイベント（`PointerMoved`・`PointerButton`・`Touch`・`PointerGone`・`MouseWheel`）の 1 行。ほかは None。
/// `frame` は完了したフレームの数（`Context::cumulative_frame_nr_for`）、`root` は主のウィンドウのイベントか。
pub fn egui_event_line(frame: u64, root: bool, event: &Event) -> Option<String> {
    let vp = if root { "root" } else { "other" };
    Some(match event {
        Event::PointerMoved(pos) => format!("egui frame={frame} vp={vp} PointerMoved pos={}", point(*pos)),
        Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers,
        } => format!(
            "egui frame={frame} vp={vp} PointerButton button={button:?} pressed={pressed} pos={} mods={}",
            point(*pos),
            mods(modifiers)
        ),
        Event::Touch {
            device_id,
            id,
            phase,
            pos,
            force,
        } => format!(
            "egui frame={frame} vp={vp} Touch device={} id={} phase={phase:?} pos={} force={}",
            device_id.0,
            id.0,
            point(*pos),
            force.map_or_else(|| "none".to_string(), |f| format!("{f:.4}"))
        ),
        Event::PointerGone => format!("egui frame={frame} vp={vp} PointerGone"),
        Event::MouseWheel {
            unit,
            delta,
            modifiers,
            ..
        } => format!(
            "egui frame={frame} vp={vp} MouseWheel unit={unit:?} delta=({:.2},{:.2}) mods={}",
            delta.x,
            delta.y,
            mods(modifiers)
        ),
        _ => return None,
    })
}

/// 1 フレーム分のイベントを記録に出す（`eframe::App::raw_input_hook` から。ペンのサイドボタンの直しのあとの、egui が受けるイベント）。
pub fn egui_events(frame: u64, root: bool, events: &[Event]) {
    if !enabled() {
        return;
    }
    for event in events {
        if let Some(text) = egui_event_line(frame, root, event) {
            line(&text);
        }
    }
    flush();
}

// ───────── ビューを動かす操作（3D の視点・2D のキャンバス） ─────────

/// そのフレームのペンの状態の 1 行。`pen_frame` はペンの押しが続いているか触れた点があるか（立つと、同じ押しの egui のポインターの入力をビューが使わない）、
/// `nav` はそのとき動かしているビューの操作の名前（無ければ "none"）。
pub fn frame_line(
    view: &str,
    frame: u64,
    pen_frame: bool,
    pen_press: bool,
    pen_points: usize,
    nav: &str,
) -> String {
    format!(
        "{view} frame={frame} pen_frame={pen_frame} pen_press={pen_press} pen_points={pen_points} nav={nav}"
    )
}

/// 押しや動きの 1 行に共通の、どこで起きたか。
#[derive(Clone, Copy)]
pub struct Site<'a> {
    /// "view3d" か "canvas"。
    pub view: &'a str,
    /// "pen"（ペンの点が持つ押し）か "mouse"（egui のポインターの押し）。
    pub src: &'a str,
    pub frame: u64,
    pub pen_frame: bool,
}

/// 押し始めの 1 行。`kind` は `press_kind` の結果か、マウスの押しで引いた操作。`skipped` が真のとき、押しは `pen_frame` のために使わなかった。
pub fn press_line(site: Site, kind: &str, pos: Pos2, button: &str, skipped: bool) -> String {
    format!(
        "{} press src={} frame={} kind={kind} pen_frame={} button={button} pos={} skipped={skipped}",
        site.view,
        site.src,
        site.frame,
        site.pen_frame,
        point(pos)
    )
}

/// ビューを動かす `nav_move` に入る位置と差の 1 行。`applied` が偽のとき、動きは `pen_frame` のために使わなかった（入らなかった）。
pub fn move_line(site: Site, pos: Pos2, previous: Pos2, nav: &str, applied: bool) -> String {
    format!(
        "{} nav_move src={} frame={} pos={} prev={} delta=({:.2},{:.2}) pen_frame={} nav={nav} applied={applied}",
        site.view,
        site.src,
        site.frame,
        point(pos),
        point(previous),
        pos.x - previous.x,
        pos.y - previous.y,
        site.pen_frame
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{PointerButton, TouchDeviceId, TouchId, TouchPhase};

    fn pos(x: f32, y: f32) -> Pos2 {
        Pos2::new(x, y)
    }

    #[test]
    fn pointer_events_become_one_line_each() {
        let moved = egui_event_line(7, true, &Event::PointerMoved(pos(10.0, 20.5)));
        assert_eq!(
            moved.as_deref(),
            Some("egui frame=7 vp=root PointerMoved pos=(10.00,20.50)")
        );
        let button = egui_event_line(
            7,
            false,
            &Event::PointerButton {
                pos: pos(1.0, 2.0),
                button: PointerButton::Secondary,
                pressed: true,
                modifiers: Modifiers {
                    shift: true,
                    ctrl: true,
                    ..Modifiers::NONE
                },
            },
        );
        assert_eq!(
            button.as_deref(),
            Some("egui frame=7 vp=other PointerButton button=Secondary pressed=true pos=(1.00,2.00) mods=ctrl+shift")
        );
        let touch = egui_event_line(
            8,
            true,
            &Event::Touch {
                device_id: TouchDeviceId(3),
                id: TouchId(4),
                phase: TouchPhase::Move,
                pos: pos(5.0, 6.0),
                force: Some(0.25),
            },
        );
        assert_eq!(
            touch.as_deref(),
            Some(
                "egui frame=8 vp=root Touch device=3 id=4 phase=Move pos=(5.00,6.00) force=0.2500"
            )
        );
        let no_force = egui_event_line(
            8,
            true,
            &Event::Touch {
                device_id: TouchDeviceId(3),
                id: TouchId(4),
                phase: TouchPhase::End,
                pos: pos(5.0, 6.0),
                force: None,
            },
        );
        assert!(no_force
            .unwrap()
            .ends_with("phase=End pos=(5.00,6.00) force=none"));
        assert_eq!(
            egui_event_line(9, true, &Event::PointerGone).as_deref(),
            Some("egui frame=9 vp=root PointerGone")
        );
        let wheel = egui_event_line(
            9,
            true,
            &Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, -1.0),
                phase: TouchPhase::Move,
                modifiers: Modifiers::NONE,
            },
        );
        assert_eq!(
            wheel.as_deref(),
            Some("egui frame=9 vp=root MouseWheel unit=Line delta=(0.00,-1.00) mods=none")
        );
    }

    #[test]
    fn other_events_are_not_logged() {
        assert!(egui_event_line(1, true, &Event::WindowFocused(true)).is_none());
        assert!(egui_event_line(1, true, &Event::Copy).is_none());
    }

    #[test]
    fn view_lines_carry_the_decision_and_the_delta() {
        assert_eq!(
            frame_line("view3d", 5, true, false, 2, "Orbit"),
            "view3d frame=5 pen_frame=true pen_press=false pen_points=2 nav=Orbit"
        );
        assert_eq!(
            press_line(
                Site { view: "canvas", src: "mouse", frame: 5, pen_frame: true },
                "Pan",
                pos(1.5, 2.0),
                "Middle",
                true
            ),
            "canvas press src=mouse frame=5 kind=Pan pen_frame=true button=Middle pos=(1.50,2.00) skipped=true"
        );
        assert_eq!(
            move_line(
                Site { view: "view3d", src: "pen", frame: 6, pen_frame: true },
                pos(110.0, 50.0),
                pos(100.0, 60.0),
                "Orbit",
                true
            ),
            "view3d nav_move src=pen frame=6 pos=(110.00,50.00) prev=(100.00,60.00) delta=(10.00,-10.00) pen_frame=true nav=Orbit applied=true"
        );
    }
}
