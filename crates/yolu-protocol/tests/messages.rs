//! 命令の往復（書いて読むと同じ）と、読めない中身を断ること。

use yolu_protocol::frame::{encode_frame, FrameReader};
use yolu_protocol::wire::DecodeError;
use yolu_protocol::*;

fn sample_model() -> Model {
    Model {
        generation: 3,
        name: "試しのモデル".into(),
        materials: vec![
            MaterialInfo {
                key: MaterialKey::Material {
                    name: "Body".into(),
                    asset: Some(("0123456789abcdef0123456789abcdef".into(), -7_000_000_000)),
                },
                shader: "Standard".into(),
                textures: vec![TextureProperty {
                    name: "_MainTex".into(),
                    width: 1024,
                    height: 512,
                }],
                routes: vec![ChannelRoute {
                    channel: channel::COLOR,
                    property: "_MainTex".into(),
                }],
            },
            MaterialInfo {
                key: MaterialKey::Unassigned,
                shader: String::new(),
                textures: vec![],
                routes: vec![],
            },
        ],
        meshes: vec![MeshData {
            key: "0/2".into(),
            name: "Quad".into(),
            skinned: true,
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
            ],
            normals: vec![[0.0, 0.0, -1.0]; 4],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
            submeshes: vec![
                Submesh {
                    material: 0,
                    indices: vec![0, 2, 1],
                },
                Submesh {
                    material: 1,
                    indices: vec![1, 2, 3],
                },
            ],
        }],
    }
}

fn sample_values() -> MaterialValues {
    MaterialValues {
        generation: 3,
        material: 0,
        kind: ValuesKind::LilToon,
        shader: "Hidden/lilToonCutoutOutline".into(),
        source: "lilToon 2.3.4 · Standard/Cutout+Outline".into(),
        properties: vec![
            PropertyEntry {
                name: "_ShadowBorder".into(),
                value: PropertyValue::Float(0.25),
            },
            PropertyEntry {
                name: "_UseShadow".into(),
                value: PropertyValue::Int(1),
            },
            PropertyEntry {
                name: "_ShadowColor".into(),
                value: PropertyValue::Color([0.82, 0.76, 0.85, 1.0]),
            },
            PropertyEntry {
                name: "_MainTex_ST".into(),
                value: PropertyValue::Vector([2.0, 1.0, 0.5, 0.0]),
            },
        ],
        keywords: vec!["_EMISSION".into()],
        slots: vec![
            SlotTexture {
                name: "_MatCapTex".into(),
                state: SlotState::Follows,
                width: 256,
                height: 256,
            },
            SlotTexture {
                name: "_ShadowColorTex".into(),
                state: SlotState::OverBudget,
                width: 4096,
                height: 4096,
            },
        ],
    }
}

fn all_messages() -> Vec<Message> {
    vec![
        Message::Hello(Hello {
            min_version: 1,
            max_version: 4,
            agent: "unity".into(),
            features: 5,
            auth: Some(HelloAuth {
                nonce: [7; 32],
                proof: [9; 32],
            }),
            versions: Some(VersionInfo {
                app: AppVersion::new(0, 3, 1),
                min_peer: AppVersion::new(0, 1, 0),
            }),
        }),
        Message::Hello(Hello {
            min_version: 1,
            max_version: 1,
            agent: "版の欄の無いブリッジ".into(),
            features: 0,
            auth: Some(HelloAuth {
                nonce: [1; 32],
                proof: [2; 32],
            }),
            versions: None,
        }),
        Message::Hello(Hello {
            min_version: 1,
            max_version: 1,
            agent: "鍵の欄の無い古いブリッジ".into(),
            features: 0,
            auth: None,
            versions: None,
        }),
        Message::Bye,
        Message::Model(sample_model()),
        Message::Pose(Pose {
            generation: 3,
            meshes: vec![MeshPose {
                mesh: 0,
                positions: vec![[0.5, 0.25, -1.0]; 4],
                normals: vec![],
            }],
        }),
        Message::Materials(MaterialsUpdate {
            generation: 3,
            materials: sample_model().materials,
        }),
        Message::ModelClosed { generation: 3 },
        Message::MaterialValues(sample_values()),
        Message::MaterialValues(MaterialValues {
            generation: 3,
            material: 1,
            kind: ValuesKind::None,
            shader: "Standard".into(),
            source: String::new(),
            properties: vec![],
            keywords: vec![],
            slots: vec![],
        }),
        Message::MaterialTexture(MaterialTexture {
            generation: 3,
            material: 0,
            slot: "_MatCapTex".into(),
            width: 2,
            height: 3,
            srgb: true,
            pixels: (0..24).collect(),
        }),
        Message::Welcome(Welcome {
            version: 1,
            agent: "standalone".into(),
            session: 99,
            features: 0,
            proof: Some([3; 32]),
            versions: Some(VersionInfo {
                app: AppVersion::new(0, 1, 0),
                min_peer: AppVersion::new(0, 3, 0),
            }),
        }),
        Message::Welcome(Welcome {
            version: 1,
            agent: "版の欄の無いスタンドアロン".into(),
            session: 2,
            features: 3,
            proof: Some([4; 32]),
            versions: None,
        }),
        Message::Welcome(Welcome {
            version: 1,
            agent: "鍵を確かめない古いスタンドアロン".into(),
            session: 1,
            features: 0,
            proof: None,
            versions: None,
        }),
        Message::Reject(Reject {
            code: RejectCode::VersionMismatch,
            text: "版".into(),
            detail: None,
        }),
        Message::Reject(Reject {
            code: RejectCode::VersionMismatch,
            text: "版（詳しい欄つき）".into(),
            detail: Some(RejectDetail {
                min_version: 2,
                max_version: 5,
                min_peer: AppVersion::new(1, 2, 3),
                peer_min_version: 1,
                peer_max_version: 1,
                peer_min_peer: AppVersion::new(0, 3, 0),
            }),
        }),
        Message::Reject(Reject {
            code: RejectCode::Unauthorized,
            text: "鍵".into(),
            detail: None,
        }),
        Message::TextureSet(TextureSet {
            set: 4,
            generation: 3,
            material: 1,
            name: "Body".into(),
            width: 4096,
            height: 2048,
            tile_size: 128,
            channels: vec![
                ChannelImage {
                    channel: channel::COLOR,
                    path: "/dev/shm/yolupainter-link-a.ylimg".into(),
                },
                ChannelImage {
                    channel: channel::ROUGHNESS,
                    path: "/tmp/yolupainter-link-b.ylimg".into(),
                },
            ],
        }),
        Message::TextureSetRemoved { set: 4 },
        Message::TilesChanged(TilesChanged {
            set: 4,
            channel: channel::COLOR,
            stamp_us: 1_700_000_000_000_000,
            tiles: vec![Tile { x: 0, y: 0 }, Tile { x: 31, y: 15 }],
        }),
        Message::Error(ErrorMessage {
            code: ErrorCode::UnknownCommand,
            kind: 0x7777,
            text: "知らない".into(),
        }),
    ]
}

#[test]
fn every_message_round_trips_through_a_frame() {
    for m in all_messages() {
        let bytes = encode_message(&m);
        let mut reader = FrameReader::new();
        let mut src: &[u8] = &bytes;
        reader.fill(&mut src).unwrap();
        let frame = reader.next_frame().unwrap().expect("枠がそろう");
        assert_eq!(frame.kind, m.kind() as u16);
        assert_eq!(frame.decode().unwrap(), m, "{:?}", m.kind());
    }
}

#[test]
fn fields_added_at_the_end_by_a_newer_version_are_skipped() {
    for m in all_messages() {
        let mut payload = m.encode_payload();
        payload.extend_from_slice(&[1, 2, 3, 4, 5]);
        assert_eq!(Message::decode(m.kind() as u16, &payload).unwrap(), m);
    }
}

#[test]
fn unknown_kinds_and_broken_payloads_are_refused() {
    assert_eq!(
        Message::decode(0x7777, &[]),
        Err(DecodeError::UnknownKind(0x7777))
    );
    // 途中で切れた中身
    for m in all_messages() {
        let payload = m.encode_payload();
        if payload.is_empty() {
            continue;
        }
        // 挨拶の鍵の欄・返事の証しは後ろに足した欄。途中で切れていれば欄が無いものとして読む（受け手が鍵無しとして断る）
        // 版の欄はさらに後ろに足した欄。途中で切れていれば版を名乗らない古い相手として読む（鍵の欄は残る）
        match &m {
            Message::Hello(h) if h.versions.is_some() => {
                let cut = Message::decode(m.kind() as u16, &payload[..payload.len() - 1]).unwrap();
                assert!(matches!(
                    cut,
                    Message::Hello(Hello { auth: Some(_), versions: None, .. })
                ));
                continue;
            }
            Message::Hello(h) if h.auth.is_some() => {
                let cut = Message::decode(m.kind() as u16, &payload[..payload.len() - 1]).unwrap();
                assert!(matches!(cut, Message::Hello(Hello { auth: None, versions: None, .. })));
                continue;
            }
            Message::Welcome(w) if w.versions.is_some() => {
                let cut = Message::decode(m.kind() as u16, &payload[..payload.len() - 1]).unwrap();
                assert!(matches!(
                    cut,
                    Message::Welcome(Welcome { proof: Some(_), versions: None, .. })
                ));
                continue;
            }
            Message::Welcome(w) if w.proof.is_some() => {
                let cut = Message::decode(m.kind() as u16, &payload[..payload.len() - 1]).unwrap();
                assert!(matches!(cut, Message::Welcome(Welcome { proof: None, versions: None, .. })));
                continue;
            }
            // 断りの詳しい欄も後ろに足した欄（途中で切れていれば古い相手の断り）
            Message::Reject(r) if r.detail.is_some() => {
                let cut = Message::decode(m.kind() as u16, &payload[..payload.len() - 1]).unwrap();
                assert!(matches!(cut, Message::Reject(Reject { detail: None, .. })));
                continue;
            }
            _ => {}
        }
        assert!(
            Message::decode(m.kind() as u16, &payload[..payload.len() - 1]).is_err(),
            "{:?} の切れた中身を読んでしまった",
            m.kind()
        );
    }
    // 頂点の数を超える添字
    let mut model = sample_model();
    model.meshes[0].submeshes[0].indices = vec![0, 1, 9];
    assert_eq!(
        Message::decode(Kind::Model as u16, &Message::Model(model).encode_payload()),
        Err(DecodeError::Invalid("三角形の添字"))
    );
    // 3 の倍数でない添字の数
    let mut model = sample_model();
    model.meshes[0].submeshes[0].indices = vec![0, 1];
    assert!(Message::decode(Kind::Model as u16, &Message::Model(model).encode_payload()).is_err());
    // 無いマテリアル
    let mut model = sample_model();
    model.meshes[0].submeshes[0].material = 2;
    assert_eq!(
        Message::decode(Kind::Model as u16, &Message::Model(model).encode_payload()),
        Err(DecodeError::Invalid("サブメッシュのマテリアル"))
    );
    // 法線の数が頂点と違う
    let mut model = sample_model();
    model.meshes[0].normals.pop();
    assert_eq!(
        Message::decode(Kind::Model as u16, &Message::Model(model).encode_payload()),
        Err(DecodeError::Invalid("法線の数"))
    );
    // 大文字の GUID
    let mut model = sample_model();
    model.materials[0].key = MaterialKey::Material {
        name: "x".into(),
        asset: Some(("0123456789ABCDEF0123456789ABCDEF".into(), 1)),
    };
    assert_eq!(
        Message::decode(Kind::Model as u16, &Message::Model(model).encode_payload()),
        Err(DecodeError::Invalid("GUID"))
    );
    // 大きさ 0・タイルの大きさが 2 の冪でない・同じチャンネルが 2 つ
    let set = |w, ts, dup: bool| {
        let mut channels = vec![ChannelImage {
            channel: 0,
            path: "a".into(),
        }];
        if dup {
            channels.push(channels[0].clone());
        }
        Message::TextureSet(TextureSet {
            set: 0,
            generation: 0,
            material: 0,
            name: "s".into(),
            width: w,
            height: 16,
            tile_size: ts,
            channels,
        })
        .encode_payload()
    };
    assert!(Message::decode(Kind::TextureSet as u16, &set(0, 128, false)).is_err());
    assert!(Message::decode(Kind::TextureSet as u16, &set(64, 100, false)).is_err());
    assert!(Message::decode(Kind::TextureSet as u16, &set(64, 128, true)).is_err());
    assert!(Message::decode(Kind::TextureSet as u16, &set(64, 128, false)).is_ok());
    // 版の範囲が逆
    let hello = Message::Hello(Hello {
        min_version: 3,
        max_version: 1,
        agent: String::new(),
        features: 0,
        auth: None,
        versions: None,
    });
    assert!(Message::decode(Kind::Hello as u16, &hello.encode_payload()).is_err());
    // 枠の頭は種類を問わず作れる（知らない種類は読む側で断る）
    assert_eq!(encode_frame(0x7777, 0, &[1, 2]).len(), 14);
}

/// 版の欄は鍵・証しの欄の後ろの位置で決まるので、鍵・証しの欄が無ければ書かない（古い読み手が版の欄を鍵の欄と読み違えない）。
#[test]
fn the_version_fields_are_not_written_without_the_key_fields() {
    let versions = Some(VersionInfo {
        app: AppVersion::new(1, 0, 0),
        min_peer: AppVersion::new(0, 1, 0),
    });
    let hello = Hello {
        min_version: 1,
        max_version: 1,
        agent: "x".into(),
        features: 0,
        auth: None,
        versions,
    };
    let bare = Message::Hello(Hello { versions: None, ..hello.clone() }).encode_payload();
    assert_eq!(Message::Hello(hello).encode_payload(), bare);
    let welcome = Welcome {
        version: 1,
        agent: "x".into(),
        session: 1,
        features: 0,
        proof: None,
        versions,
    };
    let bare = Message::Welcome(Welcome { versions: None, ..welcome.clone() }).encode_payload();
    assert_eq!(Message::Welcome(welcome).encode_payload(), bare);
}

/// 版の欄・機能の印を持つ新しい挨拶は、後ろの欄を知らない古い読み手にも同じ前半として読める（後ろを読み飛ばす）。
#[test]
fn an_old_reader_reads_the_front_of_a_greeting_with_the_new_fields() {
    let new = Message::Hello(Hello {
        min_version: 1,
        max_version: 1,
        agent: "新しいブリッジ".into(),
        features: feature::MATERIAL_VALUES,
        auth: Some(HelloAuth { nonce: [5; 32], proof: [6; 32] }),
        versions: Some(VersionInfo {
            app: AppVersion::new(0, 3, 0),
            min_peer: AppVersion::new(0, 1, 0),
        }),
    });
    let payload = new.encode_payload();
    // 古い読み手 = 鍵の欄まで読んで、残りを読み飛ばす読み手。ここでは版の欄の 12 バイトを切り落として読んで、前半が同じことを見る
    let old = Message::decode(Kind::Hello as u16, &payload[..payload.len() - 12]).unwrap();
    match (new, old) {
        (Message::Hello(n), Message::Hello(o)) => {
            assert_eq!((n.min_version, n.max_version, &n.agent, n.features, &n.auth), (o.min_version, o.max_version, &o.agent, o.features, &o.auth));
            assert_eq!(o.versions, None);
        }
        other => panic!("{other:?}"),
    }
}
