//! 元の絵の命令（MaterialOriginal）: 読み書きの上限・知らない番号の読み方・要る機能の印・印の無い相手へ送らない。

use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

use yolu_protocol::link::{self, accept_as, connect_and_greet_as};
use yolu_protocol::wire::DecodeError;
use yolu_protocol::*;

fn unique_name(tag: &str) -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    format!(
        "ylp-original-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

fn original(width: u32, height: u32, bytes: usize) -> MaterialOriginal {
    MaterialOriginal {
        generation: 3,
        material: 2,
        slot: "_MainTex".into(),
        state: OriginalState::Image,
        read: OriginalRead::File,
        compressed: false,
        width,
        height,
        srgb: true,
        pixels: vec![200; bytes],
    }
}

fn declined(state: OriginalState) -> MaterialOriginal {
    MaterialOriginal {
        state,
        read: OriginalRead::Gpu,
        width: 16384,
        height: 16384,
        pixels: Vec::new(),
        ..original(1, 1, 0)
    }
}

#[test]
fn the_original_command_has_its_own_kind_mark_and_direction() {
    assert_eq!(Kind::MaterialOriginal as u16, 0x0016);
    assert_eq!(Kind::from_u16(0x0016), Some(Kind::MaterialOriginal));
    assert_eq!(
        Kind::MaterialOriginal.required_feature(),
        feature::ORIGINAL_TEXTURES
    );
    assert_eq!(feature::ORIGINAL_TEXTURES, 1 << 4);
    assert_eq!(Kind::MaterialOriginal.direction(), Direction::ToStandalone);
    // 第 5 波の予約（bit1〜3）には触れない
    assert_eq!(feature::ASSETS | feature::PROJECT_TRANSFER | feature::ANIMATION, 0b1110);
    assert_ne!(feature::KNOWN & feature::ORIGINAL_TEXTURES, 0);
    assert_eq!(
        feature::known_bits(feature::ORIGINAL_TEXTURES | feature::MATERIAL_VALUES | 1 << 40),
        vec![feature::MATERIAL_VALUES, feature::ORIGINAL_TEXTURES]
    );
    let m = Message::MaterialOriginal(original(2, 2, 16));
    assert!(!compat::accepts(0, &m));
    // 値の印だけでは送らない（元の絵は別の印）
    assert!(!compat::accepts(feature::MATERIAL_VALUES, &m));
    assert!(compat::accepts(feature::ORIGINAL_TEXTURES, &m));
    assert!(link::wrong_direction(&m, true).is_none());
    assert!(matches!(
        link::wrong_direction(&m, false),
        Some(Message::Error(ErrorMessage {
            code: ErrorCode::UnexpectedCommand,
            kind: 0x0016,
            ..
        }))
    ));
}

#[test]
fn an_original_round_trips_with_how_it_was_read() {
    for (read, compressed) in [
        (OriginalRead::File, false),
        (OriginalRead::Imported, false),
        (OriginalRead::Imported, true),
        (OriginalRead::Gpu, true),
    ] {
        let m = Message::MaterialOriginal(MaterialOriginal {
            read,
            compressed,
            srgb: false,
            pixels: (0..32).collect(),
            ..original(4, 2, 0)
        });
        assert_eq!(
            Message::decode(0x0016, &m.encode_payload()).unwrap(),
            m,
            "{read:?} {compressed}"
        );
    }
    // 透明な画素の RGB もそのまま運ぶ
    let mut o = original(2, 1, 8);
    o.pixels = vec![9, 8, 7, 0, 1, 2, 3, 255];
    let m = Message::MaterialOriginal(o);
    assert_eq!(Message::decode(0x0016, &m.encode_payload()).unwrap(), m);
}

#[test]
fn a_declined_original_carries_its_reason_and_no_pixels() {
    for state in [
        OriginalState::Unreadable,
        OriginalState::TooLarge,
        OriginalState::OverBudget,
    ] {
        let m = Message::MaterialOriginal(declined(state));
        assert_eq!(Message::decode(0x0016, &m.encode_payload()).unwrap(), m, "{state:?}");
    }
    // 絵の付かない様子に画素が付いていたら読まない（来ない絵を待たせない・大きな領域を取らない）
    let mut with_pixels = declined(OriginalState::OverBudget);
    with_pixels.pixels = vec![1; 16];
    assert_eq!(
        Message::decode(0x0016, &Message::MaterialOriginal(with_pixels).encode_payload()),
        Err(DecodeError::Invalid("絵の付かない元の絵の画素"))
    );
}

#[test]
fn sizes_and_pixel_counts_are_checked_and_the_limit_is_the_edge_of_an_image_resource() {
    assert_eq!(MAX_ORIGINAL_SIZE, 8192);
    let short = Message::MaterialOriginal(original(4, 2, 31)).encode_payload();
    assert_eq!(
        Message::decode(0x0016, &short),
        Err(DecodeError::Invalid("元の絵の画素の数"))
    );
    let big = MAX_ORIGINAL_SIZE + 1;
    for (w, h) in [(big, 1), (1, big), (0, 4), (4, 0)] {
        let payload = Message::MaterialOriginal(original(w, h, 0)).encode_payload();
        assert_eq!(
            Message::decode(0x0016, &payload),
            Err(DecodeError::Invalid("元の絵の大きさ")),
            "{w}×{h}"
        );
    }
    // 上限ちょうどの辺は読める
    let edge = MAX_ORIGINAL_SIZE;
    let full = Message::MaterialOriginal(original(edge, 1, edge as usize * 4));
    assert_eq!(Message::decode(0x0016, &full.encode_payload()).unwrap(), full);
    // 領域は読む前に断る（長さの欄だけが大きい壊れた中身で、大きな領域を取らない）
    let mut payload = Message::MaterialOriginal(original(2, 2, 16)).encode_payload();
    let len_at = payload.len() - 16 - 4;
    payload[len_at..len_at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        Message::decode(0x0016, &payload),
        Err(DecodeError::TooLarge("元の絵の画素"))
    );
    // スロットの名前の長さの上限
    let mut o = original(1, 1, 4);
    o.slot = "S".repeat(MAX_VALUE_NAME_BYTES + 1);
    assert_eq!(
        Message::decode(0x0016, &Message::MaterialOriginal(o).encode_payload()),
        Err(DecodeError::TooLarge("スロットの名前"))
    );
    // 中身が途中で終わっている
    let payload = Message::MaterialOriginal(original(2, 2, 16)).encode_payload();
    assert_eq!(
        Message::decode(0x0016, &payload[..payload.len() - 1]),
        Err(DecodeError::Truncated)
    );
}

#[test]
fn unknown_states_reads_and_flags_are_read_as_the_least_certain_thing() {
    let mut payload = Message::MaterialOriginal(original(2, 2, 16)).encode_payload();
    // generation 4 + material 4 + スロットの名前（長さ 4 + "_MainTex" 8）の後が、様子・読み方・印
    let at = 4 + 4 + 4 + "_MainTex".len();
    // 新しい送り手が足した様子（9）は読めないとして読み、画素は付かない
    payload[at] = 9;
    payload[at + 1] = 7; // 知らない読み方は GPU を通して（原本の確かな値と言わない）
    payload[at + 2] = 0b1000_0001; // 知らない bit は読み飛ばし、圧縮の bit だけ読む
    // 画素（16 バイト）を外し、その長さの欄を 0 にする
    payload.truncate(payload.len() - 16);
    let len_at = payload.len() - 4;
    payload[len_at..].copy_from_slice(&0u32.to_le_bytes());
    let Message::MaterialOriginal(o) = Message::decode(0x0016, &payload).unwrap() else {
        panic!("元の絵の命令")
    };
    assert_eq!(o.state, OriginalState::Unreadable);
    assert_eq!(o.read, OriginalRead::Gpu);
    assert!(o.compressed);
    assert!(o.pixels.is_empty());
}

#[test]
fn an_original_is_not_sent_to_a_peer_without_the_mark() {
    // スタンドアロンが印なし（元の絵を知らない古い版の役）なら、Unity は元の絵を送らず、ほかの命令は今までどおり届く。
    // 値の印だけを持つ相手にも送らない（別の印）
    for (standalone_marks, delivered) in [
        (0, false),
        (feature::MATERIAL_VALUES, false),
        (feature::ORIGINAL_TEXTURES, true),
        (feature::MATERIAL_VALUES | feature::ORIGINAL_TEXTURES, true),
    ] {
        let name = unique_name("gate");
        let listener = Server::bind(&name, false).unwrap();
        let server = thread::spawn(move || {
            let stream = listener.accept().unwrap();
            let (conn, reader, _) = accept_as(
                stream,
                &Identity::standalone("s").with_features(standalone_marks),
                3,
                &listener.key(),
                link::HANDSHAKE_TIMEOUT,
                &|_| Ok(()),
            )
            .unwrap();
            (conn, reader, listener)
        });
        let (unity, _unity_reader, _) = connect_and_greet_as(
            &name,
            &Identity::unity("u")
                .with_features(feature::MATERIAL_VALUES | feature::ORIGINAL_TEXTURES),
        )
        .unwrap();
        let (standalone, mut reader, _listener) = server.join().unwrap();
        assert_eq!(
            unity
                .send_gated(&Message::MaterialOriginal(original(2, 2, 16)))
                .unwrap(),
            delivered,
            "相手の印 {standalone_marks:#b}"
        );
        assert!(unity
            .send_gated(&Message::ModelClosed { generation: 3 })
            .unwrap());
        let mut got = Vec::new();
        loop {
            match reader
                .next_within(&standalone, Duration::from_secs(5))
                .unwrap()
            {
                Received::Message(m) => {
                    let k = m.kind();
                    got.push(k);
                    if k == Kind::ModelClosed {
                        break;
                    }
                }
                Received::Idle => panic!("届かない: {got:?}"),
                other => panic!("{other:?}"),
            }
        }
        let expected = if delivered {
            vec![Kind::MaterialOriginal, Kind::ModelClosed]
        } else {
            vec![Kind::ModelClosed]
        };
        assert_eq!(got, expected, "相手の印 {standalone_marks:#b}");
    }
}
