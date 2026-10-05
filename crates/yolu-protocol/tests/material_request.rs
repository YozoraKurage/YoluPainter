//! 頼みの命令（MaterialRequest）と、元の絵の印・`Cached`: 読み書きの上限・知らない bit・要る機能の印・古い相手には頼まない理由・
//! 印の無い古い送り手の元の絵との行き来。

use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

use yolu_protocol::compat::RequestUnavailable;
use yolu_protocol::link::{self, accept_as, connect_and_greet_as};
use yolu_protocol::wire::DecodeError;
use yolu_protocol::*;

fn unique_name(tag: &str) -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    format!(
        "ylp-request-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

fn request(items: Vec<MaterialWant>) -> MaterialRequest {
    MaterialRequest {
        generation: 4,
        items,
    }
}

fn original(state: OriginalState, bytes: usize, stamp: u64) -> MaterialOriginal {
    MaterialOriginal {
        generation: 4,
        material: 1,
        slot: "_MainTex".into(),
        state,
        read: OriginalRead::File,
        compressed: false,
        width: 2,
        height: bytes as u32 / 8,
        srgb: true,
        pixels: vec![9; bytes],
        stamp,
    }
}

#[test]
fn the_request_has_its_own_kind_mark_and_direction() {
    assert_eq!(Kind::MaterialRequest as u16, 0x0113);
    assert_eq!(Kind::from_u16(0x0113), Some(Kind::MaterialRequest));
    assert_eq!(feature::MATERIAL_REQUEST, 1 << 5);
    assert_eq!(
        Kind::MaterialRequest.required_feature(),
        feature::MATERIAL_REQUEST
    );
    assert_eq!(Kind::MaterialRequest.direction(), Direction::ToUnity);
    assert_ne!(feature::KNOWN & feature::MATERIAL_REQUEST, 0);
    // 今までの印とは重ならない
    assert_eq!(
        feature::MATERIAL_VALUES | feature::ASSETS | feature::PROJECT_TRANSFER | feature::ANIMATION | feature::ORIGINAL_TEXTURES,
        0b1_1111
    );
    assert_eq!(
        feature::known_bits(feature::MATERIAL_REQUEST | feature::MATERIAL_VALUES | 1 << 40),
        vec![feature::MATERIAL_VALUES, feature::MATERIAL_REQUEST]
    );
    // 印が無ければ送らない。値・元の絵の印だけでは足りない（別の印）
    let m = Message::MaterialRequest(request(vec![MaterialWant::values(0)]));
    assert!(!compat::accepts(0, &m));
    assert!(!compat::accepts(
        feature::MATERIAL_VALUES | feature::ORIGINAL_TEXTURES,
        &m
    ));
    assert!(compat::accepts(feature::MATERIAL_REQUEST, &m));
    // 向き: スタンドアロンへ来たら断る
    assert!(link::wrong_direction(&m, false).is_none());
    assert!(matches!(
        link::wrong_direction(&m, true),
        Some(Message::Error(ErrorMessage {
            code: ErrorCode::UnexpectedCommand,
            kind: 0x0113,
            ..
        }))
    ));
}

#[test]
fn a_request_round_trips() {
    for items in [
        vec![],
        vec![MaterialWant::values(0)],
        vec![MaterialWant::original(3, "_MainTex", 0)],
        vec![
            MaterialWant::original(3, "_MainTex", u64::MAX),
            MaterialWant::values(3),
            MaterialWant {
                material: 7,
                wants: WANT_VALUES | WANT_ORIGINAL,
                slot: "_BaseMap".into(),
                have: 42,
            },
        ],
    ] {
        let m = Message::MaterialRequest(request(items));
        assert_eq!(Message::decode(0x0113, &m.encode_payload()).unwrap(), m);
    }
    // 上限ちょうどの項目の数は読める
    let full = Message::MaterialRequest(request(
        (0..MAX_REQUEST_ITEMS as u32).map(MaterialWant::values).collect(),
    ));
    assert_eq!(Message::decode(0x0113, &full.encode_payload()).unwrap(), full);
}

#[test]
fn a_broken_or_oversized_request_is_refused() {
    // 項目の数が上限を超える（読む前に断る: 数の欄だけが大きい壊れた中身で、大きな領域を取らない）
    let mut payload = Message::MaterialRequest(request(vec![MaterialWant::values(0)])).encode_payload();
    payload[4..8].copy_from_slice(&(MAX_REQUEST_ITEMS as u32 + 1).to_le_bytes());
    assert_eq!(
        Message::decode(0x0113, &payload),
        Err(DecodeError::TooLarge("頼みの項目の数"))
    );
    payload[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        Message::decode(0x0113, &payload),
        Err(DecodeError::TooLarge("頼みの項目の数"))
    );
    // 数の欄が実際より多い（途中で終わっている）
    payload[4..8].copy_from_slice(&2u32.to_le_bytes());
    assert_eq!(Message::decode(0x0113, &payload), Err(DecodeError::Truncated));
    // 何も頼まない項目（wants が 0）は決まりに合わない
    let nothing = Message::MaterialRequest(request(vec![MaterialWant {
        wants: 0,
        ..MaterialWant::values(1)
    }]));
    assert_eq!(
        Message::decode(0x0113, &nothing.encode_payload()),
        Err(DecodeError::Invalid("頼むもの"))
    );
    // スロットの名前の長さの上限
    let long = Message::MaterialRequest(request(vec![MaterialWant::original(
        0,
        "S".repeat(MAX_VALUE_NAME_BYTES + 1),
        0,
    )]));
    assert_eq!(
        Message::decode(0x0113, &long.encode_payload()),
        Err(DecodeError::TooLarge("スロットの名前"))
    );
    // 中身が途中で終わっている
    let ok = Message::MaterialRequest(request(vec![MaterialWant::original(0, "_MainTex", 5)])).encode_payload();
    for cut in [1, 4, 9, 13] {
        assert_eq!(
            Message::decode(0x0113, &ok[..ok.len() - cut]),
            Err(DecodeError::Truncated),
            "{cut} バイト足りない"
        );
    }
    assert_eq!(Message::decode(0x0113, &[]), Err(DecodeError::Truncated));
}

#[test]
fn unknown_request_bits_and_trailing_fields_are_kept_or_skipped() {
    // 知らない bit は項目に残す（後ろの版が足した頼み）が、知っている bit の読み方は変えない
    let m = Message::MaterialRequest(request(vec![MaterialWant {
        material: 1,
        wants: WANT_ORIGINAL | 0b1000_0000,
        slot: "_MainTex".into(),
        have: 9,
    }]));
    let Message::MaterialRequest(r) = Message::decode(0x0113, &m.encode_payload()).unwrap() else {
        panic!("頼み")
    };
    assert!(r.items[0].wants_original());
    assert!(!r.items[0].wants_values());
    assert_eq!(r.items[0].wants & !WANT_KNOWN, 0b1000_0000);
    // 後ろに足された欄は読み飛ばす
    let mut payload = m.encode_payload();
    payload.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(Message::decode(0x0113, &payload).unwrap(), m);
}

#[test]
fn an_original_carries_its_stamp_and_a_cached_one_has_no_pixels() {
    // 画素の付いた絵の印
    for stamp in [0, 1, u64::MAX] {
        let m = Message::MaterialOriginal(original(OriginalState::Image, 16, stamp));
        assert_eq!(Message::decode(0x0016, &m.encode_payload()).unwrap(), m, "{stamp}");
    }
    // 画素を付けない Cached: 印は 0 でない・画素は付けない・大きさは Unity のテクスチャの大きさ
    let cached = Message::MaterialOriginal(MaterialOriginal {
        width: 4096,
        height: 4096,
        ..original(OriginalState::Cached, 0, 0xfeed)
    });
    assert_eq!(Message::decode(0x0016, &cached.encode_payload()).unwrap(), cached);
    let no_stamp = Message::MaterialOriginal(original(OriginalState::Cached, 0, 0));
    assert_eq!(
        Message::decode(0x0016, &no_stamp.encode_payload()),
        Err(DecodeError::Invalid("元の絵の印"))
    );
    let with_pixels = Message::MaterialOriginal(original(OriginalState::Cached, 16, 5));
    assert_eq!(
        Message::decode(0x0016, &with_pixels.encode_payload()),
        Err(DecodeError::Invalid("絵の付かない元の絵の画素"))
    );
    assert_eq!(OriginalState::from_u8(4), OriginalState::Cached);
    assert_eq!(OriginalState::from_u8(5), OriginalState::Unreadable);
}

#[test]
fn an_original_without_a_stamp_from_an_older_sender_reads_as_stampless() {
    // 印の欄の無い（0.3.x の Unity が送る）元の絵は、印 0（手元に残さない・頼みに印を付けない）として読む
    let m = Message::MaterialOriginal(original(OriginalState::Image, 16, 0x55));
    let mut payload = m.encode_payload();
    payload.truncate(payload.len() - 8);
    let Message::MaterialOriginal(o) = Message::decode(0x0016, &payload).unwrap() else {
        panic!("元の絵")
    };
    assert_eq!(o.stamp, 0);
    assert_eq!(o.pixels.len(), 16);
    // 印の欄の後ろに足された欄は読み飛ばす
    let mut longer = m.encode_payload();
    longer.extend_from_slice(&[7; 12]);
    assert_eq!(Message::decode(0x0016, &longer).unwrap(), m);
    // 古い読み手（印を読まない）が新しい送り手の絵を読む: 先頭から画素までは同じ並びで、後ろの印は読み飛ばされる
    let new_bytes = m.encode_payload();
    assert_eq!(&new_bytes[..new_bytes.len() - 8], &payload[..]);
}

#[test]
fn a_model_knows_its_payload_length() {
    let model = Model {
        generation: 2,
        name: "モデル".into(),
        materials: vec![
            MaterialInfo {
                key: MaterialKey::Unassigned,
                shader: String::new(),
                textures: vec![],
                routes: vec![],
            },
            MaterialInfo {
                key: MaterialKey::Material {
                    name: "素材".into(),
                    asset: Some(("0123456789abcdef0123456789abcdef".into(), -7)),
                },
                shader: "lilToon".into(),
                textures: vec![TextureProperty {
                    name: "_MainTex".into(),
                    width: 8,
                    height: 8,
                }],
                routes: vec![ChannelRoute {
                    channel: channel::COLOR,
                    property: "_MainTex".into(),
                }],
            },
            MaterialInfo {
                key: MaterialKey::Material {
                    name: "素材 2".into(),
                    asset: None,
                },
                shader: "Standard".into(),
                textures: vec![],
                routes: vec![],
            },
        ],
        meshes: vec![MeshData {
            key: "0/1".into(),
            name: "メッシュ".into(),
            skinned: true,
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            uv0: vec![[0.0, 0.0]; 4],
            submeshes: vec![
                Submesh {
                    material: 1,
                    indices: vec![0, 1, 2],
                },
                Submesh {
                    material: 2,
                    indices: vec![1, 3, 2],
                },
            ],
        }],
    };
    assert_eq!(
        model.payload_len(),
        Message::Model(model.clone()).encode_payload().len() as u64
    );
    let empty = Model {
        generation: 1,
        name: String::new(),
        materials: vec![],
        meshes: vec![],
    };
    assert_eq!(
        empty.payload_len(),
        Message::Model(empty.clone()).encode_payload().len() as u64
    );
}

fn greet(
    standalone_marks: u64,
    unity_marks: u64,
) -> (
    Connection,
    ConnectionReader,
    Connection,
    ConnectionReader,
    Server,
) {
    let name = unique_name("link");
    let listener = Server::bind(&name, false).unwrap();
    let key = listener.key();
    let server = thread::spawn(move || {
        let stream = listener.accept().unwrap();
        let (conn, reader, _) = accept_as(
            stream,
            &Identity::standalone("s")
                .with_version(Some(AppVersion::new(0, 4, 0)))
                .with_features(standalone_marks),
            3,
            &key,
            link::HANDSHAKE_TIMEOUT,
            &|_| Ok(()),
        )
        .unwrap();
        (conn, reader, listener)
    });
    let (unity, unity_reader, _) = connect_and_greet_as(
        &name,
        &Identity::unity("u")
            .with_version(Some(AppVersion::new(0, 3, 2)))
            .with_features(unity_marks),
    )
    .unwrap();
    let (standalone, reader, listener) = server.join().unwrap();
    (standalone, reader, unity, unity_reader, listener)
}

#[test]
fn a_request_is_not_sent_to_a_peer_without_the_mark_and_the_reason_names_the_version() {
    // Unity が印なし（頼みを知らない 0.3.x の役）なら、スタンドアロンは頼まず、理由（Unity を 0.4.0 以上に）を返す。ほかの命令は今までどおり届く
    for (unity_marks, delivered) in [
        (0, false),
        (feature::MATERIAL_VALUES | feature::ORIGINAL_TEXTURES, false),
        (feature::MATERIAL_REQUEST, true),
        (
            feature::MATERIAL_VALUES | feature::ORIGINAL_TEXTURES | feature::MATERIAL_REQUEST,
            true,
        ),
    ] {
        let (standalone, _s_reader, _unity, mut unity_reader, _listener) =
            greet(feature::MATERIAL_REQUEST | feature::MATERIAL_VALUES, unity_marks);
        let info = standalone.link_info().unwrap().clone();
        let message = Message::MaterialRequest(request(vec![MaterialWant::values(0)]));
        assert_eq!(
            standalone.send_gated(&message).unwrap(),
            delivered,
            "相手の印 {unity_marks:#b}"
        );
        match info.request_support() {
            Ok(()) => assert!(delivered),
            Err(reason) => {
                assert!(!delivered);
                assert_eq!(
                    reason,
                    RequestUnavailable {
                        peer_version: Some(AppVersion::new(0, 3, 2)),
                        update_to: REQUEST_SINCE,
                        own_has_mark: true,
                    }
                );
                assert_eq!(reason.update_to, AppVersion::new(0, 4, 0));
            }
        }
        // 印の要らない命令は、どちらでも届く
        standalone
            .send_gated(&Message::TextureSetRemoved { set: 9 })
            .unwrap();
        let mut got = Vec::new();
        loop {
            match unity_reader
                .next_within(&_unity, Duration::from_secs(5))
                .unwrap()
            {
                Received::Message(m) => {
                    let k = m.kind();
                    got.push(k);
                    if k == Kind::TextureSetRemoved {
                        break;
                    }
                }
                Received::Idle => panic!("届かない: {got:?}"),
                other => panic!("{other:?}"),
            }
        }
        let expected = if delivered {
            vec![Kind::MaterialRequest, Kind::TextureSetRemoved]
        } else {
            vec![Kind::TextureSetRemoved]
        };
        assert_eq!(got, expected, "相手の印 {unity_marks:#b}");
    }
}

#[test]
fn the_reason_also_says_when_this_side_has_no_mark() {
    // 自分に印が無い（頼みを知らない側）なら、相手に印があっても頼めない。理由にそれが出る
    let (standalone, _r, _unity, _ur, _l) = greet(0, feature::MATERIAL_REQUEST);
    let info = standalone.link_info().unwrap();
    assert!(!info.has_feature(feature::MATERIAL_REQUEST));
    let reason = info.request_support().unwrap_err();
    assert!(!reason.own_has_mark);
    assert_eq!(reason.peer_version, Some(AppVersion::new(0, 3, 2)));
    // 版を名乗らない古い相手は、版が None
    let (standalone, _r, _unity, _ur, _l) = greet(feature::MATERIAL_REQUEST, 0);
    let reason = standalone.link_info().unwrap().request_support().unwrap_err();
    assert!(reason.own_has_mark);
}

#[test]
fn a_request_and_its_answers_cross_a_real_link() {
    let (standalone, mut s_reader, unity, mut u_reader, _listener) = greet(
        feature::MATERIAL_REQUEST | feature::ORIGINAL_TEXTURES,
        feature::MATERIAL_REQUEST | feature::ORIGINAL_TEXTURES,
    );
    standalone.link_info().unwrap().request_support().unwrap();
    let ask = Message::MaterialRequest(request(vec![
        MaterialWant::original(1, "_MainTex", 0xabcd),
        MaterialWant::values(2),
    ]));
    assert!(standalone.send_gated(&ask).unwrap());
    let Received::Message(got) = u_reader.next_within(&unity, Duration::from_secs(5)).unwrap() else {
        panic!("頼みが届かない")
    };
    assert_eq!(got, ask);
    // Unity の答え: 印が同じなら画素なし。スタンドアロンは受けて読める
    let answer = Message::MaterialOriginal(MaterialOriginal {
        width: 1024,
        height: 1024,
        ..original(OriginalState::Cached, 0, 0xabcd)
    });
    assert!(unity.send_gated(&answer).unwrap());
    let Received::Message(got) = s_reader
        .next_within(&standalone, Duration::from_secs(5))
        .unwrap()
    else {
        panic!("答えが届かない")
    };
    assert_eq!(got, answer);
}
