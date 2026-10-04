//! 本物のソケット（Linux は Unix のソケット）での往復・知らない命令を断る・版が合わないと断る・別のプロセスの共有メモリのタイルが届く。

use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
#[path = "support/wait.rs"]
mod wait;
use wait::{ChildGuard, MessageReader};

use yolu_protocol::host::PublishedSet;
use yolu_protocol::link::{self, accept, connect_and_greet, wrong_direction};
use yolu_protocol::*;

fn unique_name(tag: &str) -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    format!(
        "ylp-test-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

fn model(materials: usize) -> Model {
    Model {
        generation: 1,
        name: "試し".into(),
        materials: (0..materials)
            .map(|i| MaterialInfo {
                key: MaterialKey::Material {
                    name: format!("M{i}"),
                    asset: None,
                },
                shader: "Standard".into(),
                textures: vec![],
                routes: vec![ChannelRoute {
                    channel: channel::COLOR,
                    property: "_MainTex".into(),
                }],
            })
            .collect(),
        meshes: vec![MeshData {
            key: "0".into(),
            name: "Tri".into(),
            skinned: false,
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            submeshes: (0..materials)
                .map(|i| Submesh {
                    material: i as u32,
                    indices: vec![0, 1, 2],
                })
                .collect(),
        }],
    }
}

#[track_caller]
fn next_message(reader: &mut MessageReader, _conn: &Connection) -> Received {
    reader.next("次のプロトコル命令（呼び出し元の期待値）", None)
}

#[test]
fn messages_round_trip_and_unknown_commands_are_refused_without_dropping_the_link() {
    let name = unique_name("rt");
    let listener = link::listen(&name).unwrap();
    let server = thread::spawn(move || {
        use interprocess::local_socket::traits::Listener as _;
        let stream = listener.accept().unwrap();
        let (conn, reader, hello) = accept(stream, "試験のスタンドアロン", 42).unwrap();
        let mut reader = MessageReader::new(conn.clone(), reader);
        assert_eq!(hello.max_version, PROTOCOL_VERSION);
        // Model がそのまま届く
        let got = next_message(&mut reader, &conn);
        assert_eq!(got, Received::Message(Message::Model(model(2))));
        conn.send(&Message::TextureSetRemoved { set: 9 }).unwrap();
        // 知らない命令は Error を返して捨て、つながりは保つ
        assert_eq!(next_message(&mut reader, &conn), Received::Unknown(0x7777));
        // 読めない中身（Model の頭だけ）
        assert!(matches!(
            next_message(&mut reader, &conn),
            Received::Malformed(k, _) if k == Kind::Model as u16
        ));
        // 向きの違う命令（スタンドアロンに Welcome）
        let wrong = next_message(&mut reader, &conn);
        let Received::Message(m) = wrong else {
            panic!("{wrong:?}")
        };
        let reply = wrong_direction(&m, true).expect("向きが違う");
        conn.send(&reply).unwrap();
        assert_eq!(
            next_message(&mut reader, &conn),
            Received::Message(Message::Bye)
        );
    });

    let (conn, mut reader, welcome) = wait::connect(&name, None);
    assert_eq!((welcome.version, welcome.session), (PROTOCOL_VERSION, 42));
    conn.send(&Message::Model(model(2))).unwrap();
    assert_eq!(
        next_message(&mut reader, &conn),
        Received::Message(Message::TextureSetRemoved { set: 9 })
    );
    conn.send_raw(0x7777, &[1, 2, 3]).unwrap();
    match next_message(&mut reader, &conn) {
        Received::Message(Message::Error(e)) => {
            assert_eq!((e.code, e.kind), (ErrorCode::UnknownCommand, 0x7777))
        }
        other => panic!("{other:?}"),
    }
    conn.send_raw(Kind::Model as u16, &[1, 0]).unwrap();
    match next_message(&mut reader, &conn) {
        Received::Message(Message::Error(e)) => assert_eq!(e.code, ErrorCode::Malformed),
        other => panic!("{other:?}"),
    }
    conn.send(&Message::Welcome(Welcome {
        version: 1,
        agent: String::new(),
        session: 0,
        features: 0,
    }))
    .unwrap();
    match next_message(&mut reader, &conn) {
        Received::Message(Message::Error(e)) => {
            assert_eq!(e.code, ErrorCode::UnexpectedCommand)
        }
        other => panic!("{other:?}"),
    }
    conn.send(&Message::Bye).unwrap();
    server.join().unwrap();
}

#[test]
fn a_bridge_from_another_version_is_rejected() {
    let name = unique_name("ver");
    let listener = link::listen(&name).unwrap();
    let server = thread::spawn(move || {
        use interprocess::local_socket::traits::Listener as _;
        let stream = listener.accept().unwrap();
        match accept(stream, "s", 1) {
            Err(LinkError::Rejected(r)) => assert_eq!(r.code, RejectCode::VersionMismatch),
            Err(e) => panic!("{e}"),
            Ok(_) => panic!("合わない版を受けた"),
        }
    });
    let stream = link::connect(&name).unwrap();
    let mut s = &stream;
    use std::io::Write;
    s.write_all(&encode_message(&Message::Hello(Hello {
        min_version: PROTOCOL_VERSION + 10,
        max_version: PROTOCOL_VERSION + 20,
        agent: "未来のブリッジ".into(),
        features: 0,
    })))
    .unwrap();
    let mut frames = FrameReader::new();
    let frame = frames.read_frame(&mut s).unwrap().unwrap();
    match frame.decode().unwrap() {
        Message::Reject(r) => assert_eq!(r.code, RejectCode::VersionMismatch),
        other => panic!("{other:?}"),
    }
    server.join().unwrap();
}

#[test]
fn a_busy_standalone_refuses_after_reading_the_hello() {
    let name = unique_name("busy");
    let listener = link::listen(&name).unwrap();
    let server = thread::spawn(move || {
        use interprocess::local_socket::traits::Listener as _;
        let stream = listener.accept().unwrap();
        link::refuse(stream, RejectCode::Busy, "ほかの Unity とつながっています").unwrap()
    });
    match connect_and_greet(&name, "2 つ目のブリッジ") {
        Err(LinkError::Rejected(r)) => {
            assert_eq!(r.code, RejectCode::Busy);
            assert_eq!(r.text, "ほかの Unity とつながっています");
        }
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("断られるはず"),
    }
    let hello = server.join().unwrap();
    assert_eq!(hello.agent, "2 つ目のブリッジ");
}

/// 子のプロセス（同じ試験の実行ファイル）で動かすスタンドアロン役。YLP_LINK_CHILD が無ければ何もしない。
#[test]
fn child_standalone() {
    let Ok(name) = std::env::var("YLP_LINK_CHILD") else {
        return;
    };
    let listener = link::listen(&name).unwrap();
    println!("ready");
    use interprocess::local_socket::traits::Listener as _;
    let stream = listener.accept().unwrap();
    let (conn, reader, _) = accept(stream, "子のスタンドアロン", 7).unwrap();
    let mut reader = MessageReader::new(conn.clone(), reader);
    let Received::Message(Message::Model(m)) = next_message(&mut reader, &conn) else {
        panic!("Model が来ない")
    };
    // マテリアルごとに 300×200（端のタイルが半端）のセットを作り、全部のタイルに模様を書いて知らせる
    let mut sets = Vec::new();
    for (i, _) in m.materials.iter().enumerate() {
        let mut set = PublishedSet::create(
            7,
            i as u32 + 10,
            m.generation,
            i as u32,
            &format!("セット{i}"),
            300,
            200,
            128,
            &[channel::COLOR],
        )
        .unwrap();
        let image: Vec<u8> = (0..300 * 200)
            .flat_map(|p| [(p % 256) as u8, (p / 300 % 256) as u8, i as u8, 255])
            .collect();
        let img = set.image_mut(channel::COLOR).unwrap();
        let mut tiles = Vec::new();
        for y in 0..2 {
            for x in 0..3 {
                img.write_tile_from_image(x, y, &image).unwrap();
                tiles.push(Tile {
                    x: x as u16,
                    y: y as u16,
                });
            }
        }
        conn.send(&set.announce()).unwrap();
        for msg in set.tiles_changed(channel::COLOR, &tiles) {
            conn.send(&msg).unwrap();
        }
        sets.push(set);
    }
    // Bye まで待ってから閉じる（共有メモリのファイルは sets を落とすと消える）
    match next_message(&mut reader, &conn) {
        Received::Message(Message::Bye) => {}
        other => panic!("Byeを待っていた: {other:?}"),
    }
}

#[test]
fn tiles_written_by_another_process_arrive_through_shared_memory() {
    let name = unique_name("proc");
    let mut child = ChildGuard::spawn(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "child_standalone",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("YLP_LINK_CHILD", &name)
            .stdout(std::process::Stdio::null()),
    );
    let (conn, mut reader, welcome) = wait::connect(&name, Some(&mut child));
    assert_eq!(welcome.session, 7);
    conn.send(&Message::Model(model(2))).unwrap();
    let mut got = Vec::new();
    let mut paths = Vec::new();
    while got.len() < 2 {
        match reader.next("別プロセスのセット・タイル通知", Some(&mut child)) {
            Received::Message(Message::TextureSet(s)) => {
                assert_eq!((s.width, s.height, s.tile_size), (300, 200, 128));
                let image =
                    SharedImageReader::open(std::path::Path::new(&s.channels[0].path)).unwrap();
                assert_ne!(
                    image.writer_pid(),
                    std::process::id(),
                    "別のプロセスが書いた"
                );
                paths.push(s.channels[0].path.clone());
                got.push((s, image, Vec::new()));
            }
            Received::Message(Message::TilesChanged(t)) => {
                let entry = got
                    .iter_mut()
                    .find(|g| g.0.set == t.set)
                    .expect("知らせたセット");
                entry.2.extend(t.tiles);
            }
            other => panic!("{other:?}"),
        }
    }
    // 知らせは TextureSet の直後に来る。残りの TilesChanged を受ける
    while got.iter().any(|g| g.2.len() < 6) {
        match reader.next("別プロセスのセット・タイル通知", Some(&mut child)) {
            Received::Message(Message::TilesChanged(t)) => got
                .iter_mut()
                .find(|g| g.0.set == t.set)
                .unwrap()
                .2
                .extend(t.tiles),
            other => panic!("{other:?}"),
        }
    }
    for (set, image, tiles) in &got {
        let mut out = vec![0u8; 300 * 200 * 4];
        for t in tiles {
            assert_eq!(
                image
                    .read_tile_into_image(t.x as u32, t.y as u32, &mut out)
                    .unwrap(),
                TileRead::Complete
            );
        }
        for p in [0usize, 299, 300 * 128 + 129, 300 * 200 - 1] {
            let expected = [
                (p % 256) as u8,
                (p / 300 % 256) as u8,
                set.material as u8,
                255,
            ];
            assert_eq!(out[p * 4..p * 4 + 4], expected, "画素 {p}");
        }
    }
    conn.send(&Message::Bye).unwrap();
    let status = child.finish();
    assert!(status.success());
    // 書き手が閉じた後に読み手を落とすと、ファイルは残らない（Windows では写像している間は名前が残り得る）
    assert!(got.iter().all(|g| g.1.writer_closed()));
    drop(got);
    for p in paths {
        assert!(
            !std::path::Path::new(&p).exists(),
            "書き手が閉じたら消える: {p}"
        );
    }
}
