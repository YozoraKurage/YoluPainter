//! マテリアルの値の命令（MaterialValues・MaterialTexture）: 読み書きの上限・知らない番号の読み方・要る機能の印・印の無い相手へ送らない。

use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

use yolu_protocol::link::{self, accept_as, connect_and_greet_as};
use yolu_protocol::wire::DecodeError;
use yolu_protocol::*;

fn unique_name(tag: &str) -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    format!(
        "ylp-values-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

fn values() -> MaterialValues {
    MaterialValues {
        generation: 1,
        material: 0,
        kind: ValuesKind::LilToon,
        shader: "lilToon".into(),
        source: "lilToon 2.3.4".into(),
        properties: vec![PropertyEntry {
            name: "_ShadowBorder".into(),
            value: PropertyValue::Float(0.4),
        }],
        keywords: vec![],
        slots: vec![SlotTexture {
            name: "_MatCapTex".into(),
            state: SlotState::Follows,
            width: 4,
            height: 4,
        }],
    }
}

fn texture(width: u32, height: u32, bytes: usize) -> MaterialTexture {
    MaterialTexture {
        generation: 1,
        material: 0,
        slot: "_MatCapTex".into(),
        width,
        height,
        srgb: true,
        pixels: vec![128; bytes],
    }
}

#[test]
fn the_value_commands_need_the_material_values_mark_and_go_to_the_standalone() {
    for kind in [Kind::MaterialValues, Kind::MaterialTexture] {
        assert_eq!(
            kind.required_feature(),
            feature::MATERIAL_VALUES,
            "{kind:?}"
        );
        assert_eq!(kind.direction(), Direction::ToStandalone, "{kind:?}");
        assert_eq!(Kind::from_u16(kind as u16), Some(kind));
    }
    assert_eq!(Kind::MaterialValues as u16, 0x0014);
    assert_eq!(Kind::MaterialTexture as u16, 0x0015);
    let m = Message::MaterialValues(values());
    assert!(!compat::accepts(0, &m));
    assert!(!compat::accepts(feature::ASSETS, &m));
    assert!(compat::accepts(
        feature::MATERIAL_VALUES | feature::ASSETS,
        &m
    ));
    // スタンドアロンが受けるもの。Unity 側へ来れば向きの違う命令として断る
    assert!(link::wrong_direction(&m, true).is_none());
    assert!(matches!(
        link::wrong_direction(&m, false),
        Some(Message::Error(ErrorMessage {
            code: ErrorCode::UnexpectedCommand,
            kind: 0x0014,
            ..
        }))
    ));
}

#[test]
fn texture_sizes_and_pixel_counts_are_checked() {
    let ok = Message::MaterialTexture(texture(4, 2, 32));
    assert_eq!(Message::decode(0x0015, &ok.encode_payload()).unwrap(), ok);
    // 画素の数が大きさと合わない
    let short = Message::MaterialTexture(texture(4, 2, 31)).encode_payload();
    assert_eq!(
        Message::decode(0x0015, &short),
        Err(DecodeError::Invalid("スロットの絵の画素の数"))
    );
    // 辺の上限を超える・大きさ 0
    let big = MAX_SLOT_TEXTURE_SIZE + 1;
    for (w, h) in [(big, 1), (1, big), (0, 4), (4, 0)] {
        let payload = Message::MaterialTexture(texture(w, h, 0)).encode_payload();
        assert_eq!(
            Message::decode(0x0015, &payload),
            Err(DecodeError::Invalid("スロットの絵の大きさ")),
            "{w}×{h}"
        );
    }
    // 上限ちょうどは読める
    let edge = MAX_SLOT_TEXTURE_SIZE;
    let full = Message::MaterialTexture(texture(edge, 1, edge as usize * 4));
    assert_eq!(
        Message::decode(0x0015, &full.encode_payload()).unwrap(),
        full
    );
}

#[test]
fn values_refuse_non_finite_numbers_unknown_types_and_too_many_entries() {
    let mut v = values();
    v.properties[0].value = PropertyValue::Color([0.0, f32::NAN, 0.0, 1.0]);
    assert_eq!(
        Message::decode(0x0014, &Message::MaterialValues(v).encode_payload()),
        Err(DecodeError::Invalid("プロパティの値"))
    );
    let mut v = values();
    v.properties[0].value = PropertyValue::Float(f32::INFINITY);
    assert!(Message::decode(0x0014, &Message::MaterialValues(v).encode_payload()).is_err());
    // 型の番号 9 は知らない: 中身を読めない（値の大きさが分からないので、後ろを読み飛ばせない）
    let mut payload = Message::MaterialValues(values()).encode_payload();
    let at = payload
        .windows(b"_ShadowBorder".len())
        .position(|w| w == b"_ShadowBorder")
        .unwrap()
        + b"_ShadowBorder".len();
    payload[at] = 9;
    assert_eq!(
        Message::decode(0x0014, &payload),
        Err(DecodeError::Invalid("プロパティの型"))
    );
    // プロパティの数の上限
    let mut v = values();
    v.properties = (0..=MAX_VALUE_PROPERTIES)
        .map(|i| PropertyEntry {
            name: format!("_P{i}"),
            value: PropertyValue::Int(0),
        })
        .collect();
    assert_eq!(
        Message::decode(0x0014, &Message::MaterialValues(v).encode_payload()),
        Err(DecodeError::TooLarge("プロパティの数"))
    );
    // 名前の長さの上限
    let mut v = values();
    v.keywords = vec!["K".repeat(MAX_VALUE_NAME_BYTES + 1)];
    assert_eq!(
        Message::decode(0x0014, &Message::MaterialValues(v).encode_payload()),
        Err(DecodeError::TooLarge("キーワード"))
    );
}

#[test]
fn unknown_kinds_and_slot_states_are_read_as_nothing_to_draw() {
    // 見た目の種類 7（新しい送り手が足したもの）は値なし、スロットの様子 9 は読めないとして読む（描けない値・来ない絵を待たない）
    let mut payload = Message::MaterialValues(values()).encode_payload();
    payload[8] = 7;
    let at = payload
        .windows(b"_MatCapTex".len())
        .position(|w| w == b"_MatCapTex")
        .unwrap()
        + b"_MatCapTex".len();
    payload[at] = 9;
    let Message::MaterialValues(v) = Message::decode(0x0014, &payload).unwrap() else {
        panic!("値の命令")
    };
    assert_eq!(v.kind, ValuesKind::None);
    assert_eq!(v.slots[0].state, SlotState::Unreadable);
    assert_eq!(v.properties, values().properties);
}

#[test]
fn the_values_are_not_sent_to_a_peer_without_the_mark() {
    // スタンドアロンは印なし（値を知らない古い版の役）、Unity は印あり: Unity は値を送らず、ほかの命令は今までどおり届く
    for (standalone_marks, delivered) in [(0, false), (feature::MATERIAL_VALUES, true)] {
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
            &Identity::unity("u").with_features(feature::MATERIAL_VALUES),
        )
        .unwrap();
        let (standalone, mut reader, _listener) = server.join().unwrap();
        assert_eq!(
            unity
                .send_gated(&Message::MaterialValues(values()))
                .unwrap(),
            delivered
        );
        assert_eq!(
            unity
                .send_gated(&Message::MaterialTexture(texture(4, 4, 64)))
                .unwrap(),
            delivered
        );
        assert!(unity
            .send_gated(&Message::ModelClosed { generation: 1 })
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
            vec![
                Kind::MaterialValues,
                Kind::MaterialTexture,
                Kind::ModelClosed,
            ]
        } else {
            vec![Kind::ModelClosed]
        };
        assert_eq!(got, expected);
    }
}
