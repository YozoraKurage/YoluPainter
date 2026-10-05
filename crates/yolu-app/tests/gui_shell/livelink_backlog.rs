//! Live Link の受け取りの列の上限: 画面のスレッドが取り出さない間に Unity が大きな命令を送り続けても、積む量は上限（と手前で読んだ 1 つ）
//! までで、読むスレッドは待ち、Unity の送りは待たされる。画面が取り出せば再開し、全部の命令が送った順に届く。
use crate::common;
use crate::common::wait;
use wait::WATCHDOG;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use common::*;
use egui_kittest::Harness;
use yolu_app::livelink::LinkStatus;
use yolu_app::state::Action;
use yolu_app::YoluApp;
use yolu_protocol::link::connect_and_greet_as;
use yolu_protocol::*;

fn unique_name(tag: &str) -> String {
    crate::common::names::unique_name("ylbl", tag)
}

/// Unity の役: つないで、届く命令を裏のスレッドで集める。
struct FakeUnity {
    conn: Connection,
    rx: mpsc::Receiver<Message>,
}

impl FakeUnity {
    fn connect(name: &str) -> FakeUnity {
        let identity = Identity::unity("試験の Unity")
            .with_version(Some(AppVersion::new(0, 3, 0)))
            .with_features(yolu_app::livelink::FEATURES);
        let (conn, mut reader, _welcome) = connect_and_greet_as(name, &identity).unwrap();
        let (tx, rx) = mpsc::channel();
        let reply = conn.clone();
        std::thread::spawn(move || loop {
            match reader.next(&reply) {
                Ok(Received::Idle) => {}
                Ok(Received::Message(m)) => {
                    if tx.send(m).is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        });
        FakeUnity { conn, rx }
    }
}

/// 1 MiB の画素を持つ「スロットの絵」。世代の欄に通し番号を入れる（モデルが無いので、どれも世代の食い違いで断られ、その断りの文に番号が出る）。
fn texture(serial: u32) -> Message {
    Message::MaterialTexture(MaterialTexture {
        generation: serial,
        material: 0,
        slot: "_MainTex".into(),
        width: 512,
        height: 512,
        srgb: true,
        pixels: vec![7; 512 * 512 * 4],
    })
}

/// 断りの文（「スロットの絵の世代 N は…」）の通し番号。
fn serial_of(message: &Message) -> Option<u32> {
    match message {
        Message::Error(e) if e.kind == Kind::MaterialTexture as u16 => e
            .text
            .split("世代 ")
            .nth(1)?
            .split_whitespace()
            .next()?
            .parse()
            .ok(),
        _ => None,
    }
}

fn step_until(h: &mut Harness<'_, YoluApp>, what: &str, mut cond: impl FnMut(&mut Harness<'_, YoluApp>) -> bool) {
    let deadline = Instant::now() + WATCHDOG;
    loop {
        h.step();
        if cond(h) {
            return;
        }
        assert!(Instant::now() < deadline, "{what} を待ったが来ない");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_flood_of_large_commands_waits_at_the_limit_and_every_command_arrives_in_order() {
    const LIMIT: usize = 2 << 20;
    const COMMANDS: u32 = 24;
    let mut h = app(1280.0, 800.0, 256);
    let name = unique_name("flood");
    h.state_mut().link_mut().set_name(&name).unwrap();
    h.state_mut().state.apply(Action::ToggleLiveLink);
    h.run();
    h.state_mut().link_mut().set_queue_limit(LIMIT);
    let unity = FakeUnity::connect(&name);
    step_until(&mut h, "つながる", |h| matches!(h.state().state.link.status, LinkStatus::Connected { .. }));

    // 画面のフレームを回さない間に、1 MiB の命令を 24 個（上限の 12 倍）送る。送るのは別のスレッド（待たされるので）
    let conn = unity.conn.clone();
    let sender = std::thread::spawn(move || {
        for serial in 0..COMMANDS {
            conn.send(&texture(1000 + serial)).unwrap();
        }
    });
    // 上限に達するまで積む（フレームは回さない）。達したあと、さらに待っても増えない
    let one = 512 * 512 * 4;
    let deadline = Instant::now() + WATCHDOG;
    while h.state().link().queued_bytes() < LIMIT {
        assert!(Instant::now() < deadline, "積まれない: {}", h.state().link().queued_bytes());
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(400));
    let queued = h.state().link().queued_bytes();
    assert!(queued <= LIMIT + one + 4096, "積みは上限と、手前で読んだ 1 つまで: {queued}");
    assert!(!sender.is_finished(), "上限に達している間は、Unity の送りが待たされる");

    // 画面が取り出し始めると、読むのが再開して、全部が送った順に届く（捨てない）
    let mut serials = Vec::new();
    step_until(&mut h, "全部の命令", |_| {
        while let Ok(m) = unity.rx.try_recv() {
            serials.extend(serial_of(&m));
        }
        serials.len() as u32 >= COMMANDS
    });
    sender.join().unwrap();
    assert_eq!(serials, (1000..1000 + COMMANDS).collect::<Vec<_>>(), "全部・送った順に当たる");
    assert_eq!(h.state().link().queued_bytes(), 0, "取り出したら帳簿は空");
}

/// 上限に達して読むスレッドが待っているあいだに Unity が切っても、画面のスレッドが取り出せば、積んだ分の後で切れたことを受ける。
/// 読むスレッドは待ちっぱなしにならず、窓を落とす（Live Link が落ちる）と待ちが解ける。
#[test]
fn a_reader_waiting_at_the_limit_ends_when_the_link_is_dropped() {
    let mut h = app(1280.0, 800.0, 256);
    let name = unique_name("drop");
    h.state_mut().link_mut().set_name(&name).unwrap();
    h.state_mut().state.apply(Action::ToggleLiveLink);
    h.run();
    h.state_mut().link_mut().set_queue_limit(1 << 20);
    let unity = FakeUnity::connect(&name);
    step_until(&mut h, "つながる", |h| matches!(h.state().state.link.status, LinkStatus::Connected { .. }));
    let conn = unity.conn.clone();
    let sender = std::thread::spawn(move || {
        for serial in 0..16u32 {
            if conn.send(&texture(2000 + serial)).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + WATCHDOG;
    while h.state().link().queued_bytes() < 1 << 20 {
        assert!(Instant::now() < deadline, "積まれない");
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(300));
    assert!(!sender.is_finished());
    // Live Link を止める（つながりを切る）。待っていた読むスレッドは、今のつながりでなくなったので読むのをやめ、つながりを閉じる
    h.state_mut().state.apply(Action::ToggleLiveLink);
    h.run();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !sender.is_finished() {
        h.step();
        assert!(Instant::now() < deadline, "切ったあとも Unity の送りが待たされたまま");
        std::thread::sleep(Duration::from_millis(5));
    }
    sender.join().unwrap();
}
