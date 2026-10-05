//! 本物のソケット（Unix は自分だけのフォルダの Unix ソケット、Windows は名前付きパイプ）での往復・知らない命令を断る・版が合わないと断る・
//! 鍵（無い・合わない・使い回し・返事の証しが合わない）・別のプロセスの共有メモリのタイルが届く。

use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};
#[path = "support/wait.rs"]
mod wait;
use wait::{ChildGuard, MessageReader};

use yolu_protocol::frame::encode_frame;
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

/// 鍵を知っているブリッジの挨拶の鍵の欄（nonce は新しく作る）。
fn good_auth(key: &LinkKey) -> HelloAuth {
    let nonce = yolu_protocol::auth::random_bytes().unwrap();
    HelloAuth {
        nonce,
        proof: key.hello_proof(&nonce),
    }
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
    let listener = Server::bind(&name, false).unwrap();
    let server = thread::spawn(move || {
        let stream = listener.accept().unwrap();
        let (conn, reader, hello) =
            accept(stream, "試験のスタンドアロン", 42, &listener.key()).unwrap();
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
        proof: None,
        versions: None,
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
fn a_command_over_the_limit_is_refused_by_send_and_the_link_keeps_working() {
    // 512 MiB の命令は作らず、上限を狭めて送り口の断りを通す（`send` は `send_within` の上限が 512 MiB のもの）
    let name = unique_name("over");
    let listener = Server::bind(&name, false).unwrap();
    let server = thread::spawn(move || {
        let stream = listener.accept().unwrap();
        let (conn, reader, _) = accept(stream, "試験のスタンドアロン", 7, &listener.key()).unwrap();
        let mut reader = MessageReader::new(conn.clone(), reader);
        // 断られた命令は 1 バイトも届かない: 次に読めるのは、そのあとに送った命令（枠が壊れていれば区切りを失う）
        assert_eq!(
            next_message(&mut reader, &conn),
            Received::Message(Message::TextureSetRemoved { set: 3 })
        );
        // 上限ちょうどの命令は届く
        assert_eq!(
            next_message(&mut reader, &conn),
            Received::Message(Message::Model(model(1)))
        );
        assert_eq!(
            next_message(&mut reader, &conn),
            Received::Message(Message::Bye)
        );
    });
    let (conn, _reader, _) = wait::connect(&name, None);
    let big = Message::Model(model(1));
    let size = big.encode_payload().len();
    let e = conn.send_within(&big, size - 1).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput, "{e}");
    let e = conn.send_within(&big, 0).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput, "{e}");
    // 断られたあとも、つながりは使える
    conn.send(&Message::TextureSetRemoved { set: 3 }).unwrap();
    conn.send_within(&big, size).unwrap();
    conn.send(&Message::Bye).unwrap();
    server.join().unwrap();
}

#[test]
fn a_bridge_from_another_version_is_rejected() {
    let name = unique_name("ver");
    let listener = Server::bind(&name, false).unwrap();
    // 鍵は合っていて、版だけが合わないブリッジ
    let key = LinkKey::load(&name).unwrap();
    let server = thread::spawn(move || {
        let stream = listener.accept().unwrap();
        match accept(stream, "s", 1, &listener.key()) {
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
        auth: Some(good_auth(&key)),
        versions: None,
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
fn a_busy_standalone_refuses_after_the_greeting_with_the_reason() {
    let name = unique_name("busy");
    let listener = Server::bind(&name, false).unwrap();
    let server = thread::spawn(move || {
        let stream = listener.accept().unwrap();
        let busy = |_: &Hello| {
            Err(Reject {
                code: RejectCode::Busy,
                text: "ほかの Unity とつながっています".into(),
                detail: None,
            })
        };
        link::accept_with(
            stream,
            "s",
            1,
            &listener.key(),
            link::HANDSHAKE_TIMEOUT,
            &busy,
        )
        .map(|(_, _, hello)| hello)
    });
    match connect_and_greet(&name, "2 つ目のブリッジ") {
        Err(LinkError::Rejected(r)) => {
            assert_eq!(r.code, RejectCode::Busy);
            assert_eq!(r.text, "ほかの Unity とつながっています");
        }
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("断られるはず"),
    }
    assert!(matches!(
        server.join().unwrap(),
        Err(LinkError::Rejected(r)) if r.code == RejectCode::Busy
    ));
}

/// 子のプロセス（同じ試験の実行ファイル）で動かすスタンドアロン役。YLP_LINK_CHILD が無ければ何もしない。
#[test]
fn child_standalone() {
    let Ok(name) = std::env::var("YLP_LINK_CHILD") else {
        return;
    };
    let listener = Server::bind(&name, false).unwrap();
    println!("ready");
    let stream = listener.accept().unwrap();
    let (conn, reader, _) = accept(stream, "子のスタンドアロン", 7, &listener.key()).unwrap();
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

// ───────── 接続の鍵 ─────────

fn hello_with(auth: Option<HelloAuth>) -> Hello {
    Hello {
        min_version: MIN_PROTOCOL_VERSION,
        max_version: PROTOCOL_VERSION,
        agent: "試験の相手".into(),
        features: 0,
        auth,
        versions: None,
    }
}

/// 生のつながりで挨拶を送り、返事の命令を 1 つ読む。
fn raw_greeting(name: &str, hello: &Hello) -> Message {
    use std::io::Write;
    let stream = link::connect(name).unwrap();
    let mut s = &stream;
    s.write_all(&encode_message(&Message::Hello(hello.clone())))
        .unwrap();
    let mut frames = FrameReader::new();
    frames
        .read_frame(&mut s)
        .unwrap()
        .unwrap()
        .decode()
        .unwrap()
}

#[test]
fn a_hello_without_the_key_is_rejected_and_a_used_one_is_not_accepted_twice() {
    let name = unique_name("auth");
    let server = Server::bind(&name, false).unwrap();
    let key = LinkKey::load(&name).unwrap();
    let handle = thread::spawn(move || {
        let server_key = server.key();
        let mut codes = Vec::new();
        for _ in 0..5 {
            let stream = server.accept().unwrap();
            match accept(stream, "試験のスタンドアロン", 5, &server_key) {
                Ok(_) => codes.push(None),
                Err(LinkError::Rejected(r)) => codes.push(Some(r.code)),
                Err(e) => panic!("{e}"),
            }
        }
        codes
    });
    let unauthorized = |m: Message| match m {
        Message::Reject(r) => {
            assert_eq!(r.code, RejectCode::Unauthorized);
            r.text
        }
        other => panic!("{other:?}"),
    };
    // 鍵の欄が無い（古いブリッジ）
    let text = unauthorized(raw_greeting(&name, &hello_with(None)));
    assert!(text.contains("対応していません"), "{text}");
    // 別の鍵（別のスタンドアロンの鍵・ほかのユーザー）
    let other = LinkKey::generate().unwrap();
    let text = unauthorized(raw_greeting(&name, &hello_with(Some(good_auth(&other)))));
    assert!(text.contains("合いません"), "{text}");
    // 正しい鍵は通り、返事の証しは同じ nonce に結ばれている
    let auth = good_auth(&key);
    match raw_greeting(&name, &hello_with(Some(auth.clone()))) {
        Message::Welcome(w) => assert_eq!(
            w.proof,
            Some(key.welcome_proof(&auth.nonce, w.version, w.session))
        ),
        other => panic!("{other:?}"),
    }
    // 同じ挨拶の使い回しは通らない
    let text = unauthorized(raw_greeting(&name, &hello_with(Some(auth))));
    assert!(text.contains("2 度"), "{text}");
    // 鍵の欄が途中で切れた挨拶は、鍵が無いものとして断る
    use std::io::Write;
    let stream = link::connect(&name).unwrap();
    let mut s = &stream;
    let mut payload = Message::Hello(hello_with(Some(good_auth(&key)))).encode_payload();
    payload.truncate(payload.len() - 10);
    s.write_all(&encode_frame(Kind::Hello as u16, 0, &payload))
        .unwrap();
    let mut frames = FrameReader::new();
    let reply = frames
        .read_frame(&mut s)
        .unwrap()
        .unwrap()
        .decode()
        .unwrap();
    unauthorized(reply);
    assert_eq!(
        handle.join().unwrap(),
        vec![
            Some(RejectCode::Unauthorized),
            Some(RejectCode::Unauthorized),
            None,
            Some(RejectCode::Unauthorized),
            Some(RejectCode::Unauthorized)
        ]
    );
}

#[test]
fn a_busy_standalone_does_not_tell_a_stranger_that_it_is_busy() {
    let name = unique_name("busyauth");
    let server = Server::bind(&name, false).unwrap();
    let asked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = asked.clone();
    let handle = thread::spawn(move || {
        let stream = server.accept().unwrap();
        let claim = |_: &Hello| {
            seen.store(true, Ordering::Relaxed);
            Err(Reject {
                code: RejectCode::Busy,
                text: "つながっています".into(),
                detail: None,
            })
        };
        link::accept_with(
            stream,
            "s",
            1,
            &server.key(),
            link::HANDSHAKE_TIMEOUT,
            &claim,
        )
        .map(|_| ())
    });
    let other = LinkKey::generate().unwrap();
    match raw_greeting(&name, &hello_with(Some(good_auth(&other)))) {
        Message::Reject(r) => assert_eq!(r.code, RejectCode::Unauthorized),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        handle.join().unwrap(),
        Err(LinkError::Rejected(r)) if r.code == RejectCode::Unauthorized
    ));
    assert!(
        !asked.load(Ordering::Relaxed),
        "鍵の合わない相手には枠の確かめを呼ばない"
    );
}

/// 鍵を知らないスタンドアロン（名前だけを先に取った別のプログラム）の役: 挨拶を読み、welcome の証しを reply で作って返す。
fn impostor(name: &str, reply: impl FnOnce(&Hello) -> Option<[u8; 32]> + Send + 'static) {
    let server = Server::bind(name, false).unwrap();
    // 本物の鍵は置かれている（このサーバーが置いた）。なりすましは別の鍵で証しを作る
    thread::spawn(move || {
        let stream = server.accept().unwrap();
        let mut frames = FrameReader::new();
        let mut s = &stream;
        let Message::Hello(h) = frames
            .read_frame(&mut s)
            .unwrap()
            .unwrap()
            .decode()
            .unwrap()
        else {
            panic!("Hello が来ない")
        };
        let proof = reply(&h);
        use std::io::Write;
        s.write_all(&encode_message(&Message::Welcome(Welcome {
            version: PROTOCOL_VERSION,
            agent: "なりすまし".into(),
            session: 1,
            features: 0,
            proof,
            versions: None,
        })))
        .unwrap();
        thread::sleep(Duration::from_millis(500));
        drop(server);
    });
}

#[test]
fn the_bridge_does_not_use_a_standalone_that_does_not_prove_the_key() {
    // 別の鍵で証しを作る
    let name = unique_name("fake1");
    impostor(&name, |h| {
        let fake = LinkKey::generate().unwrap();
        Some(fake.welcome_proof(&h.auth.as_ref().unwrap().nonce, PROTOCOL_VERSION, 1))
    });
    match connect_and_greet(&name, "ブリッジ") {
        Err(LinkError::Untrusted(t)) => assert!(t.contains("鍵を知りません"), "{t}"),
        other => panic!("{:?}", other.map(|_| ())),
    }
    // 証しが無い（鍵を確かめない古いスタンドアロン）
    let name = unique_name("fake2");
    impostor(&name, |_| None);
    match connect_and_greet(&name, "ブリッジ") {
        Err(LinkError::Untrusted(t)) => assert!(t.contains("古い版"), "{t}"),
        other => panic!("{:?}", other.map(|_| ())),
    }
}

#[test]
fn a_replayed_welcome_proof_is_not_accepted() {
    // 鍵を読めるなりすまし（同じユーザー）が、前に見た返事の証しを別のつながりで返しても、挨拶の nonce が違うので通らない
    let name = unique_name("replay");
    let key_name = name.clone();
    impostor(&name, move |_| {
        let key = LinkKey::load(&key_name).unwrap();
        Some(key.welcome_proof(&[0u8; 32], PROTOCOL_VERSION, 1))
    });
    assert!(matches!(
        connect_and_greet(&name, "ブリッジ"),
        Err(LinkError::Untrusted(_))
    ));
}

#[test]
fn a_second_standalone_cannot_take_the_name_and_the_key_goes_with_the_first() {
    let name = unique_name("twice");
    let first = Server::bind(&name, false).unwrap();
    let key_path = yolu_protocol::auth::key_path(&name).unwrap();
    let before = std::fs::read(&key_path).unwrap();
    let err = Server::bind(&name, false)
        .err()
        .expect("2 つ目は待ち受けられない");
    assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse, "{err}");
    assert_eq!(
        std::fs::read(&key_path).unwrap(),
        before,
        "最初の鍵は変わらない"
    );
    drop(first);
    assert!(!key_path.exists(), "待ち受けをやめたら鍵を消す");
    // 同じ名前でまた待ち受けられ、鍵は新しくなる
    let again = Server::bind(&name, false).unwrap();
    assert_ne!(std::fs::read(&key_path).unwrap(), before);
    drop(again);
}

#[cfg(unix)]
#[test]
fn the_socket_the_key_and_the_folder_are_only_for_the_user() {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let name = unique_name("perm");
    let _server = Server::bind(&name, false).unwrap();
    let socket = link::socket_path(&name).unwrap();
    let key = yolu_protocol::auth::key_path(&name).unwrap();
    let dir = socket.parent().unwrap();
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().mode() & 0o777;
    assert!(std::fs::symlink_metadata(&socket)
        .unwrap()
        .file_type()
        .is_socket());
    assert_eq!(mode(&socket), 0o600, "ソケット");
    assert_eq!(mode(&key), 0o600, "鍵のファイル");
    assert_eq!(mode(dir), 0o700, "フォルダ");
    // 自分のソケットでないもの（普通のファイル）にはつながない
    let fake = unique_name("notsock");
    let fake_path = link::socket_path(&fake).unwrap();
    std::fs::write(&fake_path, b"x").unwrap();
    let err = link::connect(&fake).expect_err("つなげない");
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied, "{err}");
    std::fs::remove_file(&fake_path).unwrap();
}

/// 子のプロセス（同じ試験の実行ファイル）として待ち受けるだけ。YLP_LINK_LISTEN が無ければ何もしない。落ちた（kill された）スタンドアロンの役。
#[test]
fn child_listener() {
    let Ok(name) = std::env::var("YLP_LINK_LISTEN") else {
        return;
    };
    let _server = Server::bind(&name, false).unwrap();
    println!("ready");
    thread::sleep(Duration::from_secs(60));
}

#[test]
fn a_crashed_standalone_leaves_a_stale_name_that_a_new_one_takes_over() {
    let name = unique_name("crash");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "child_listener",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("YLP_LINK_LISTEN", &name)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let key_path = yolu_protocol::auth::key_path(&name).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !key_path.exists() {
        assert!(Instant::now() < deadline, "子が待ち受けない");
        thread::sleep(Duration::from_millis(20));
    }
    // 生きている間は取れない
    assert_eq!(
        Server::bind(&name, false).err().map(|e| e.kind()),
        Some(std::io::ErrorKind::AddrInUse)
    );
    child.kill().unwrap();
    child.wait().unwrap();
    // 子は片付けずに落ちた: 古い鍵が残っている（Windows ではパイプは OS が消す）。新しいスタンドアロンが同じ名前を使える。
    // 引き継ぐ間も、ブリッジはつなぎ続ける（古い鍵と新しい待ち受けの組で挨拶して、鍵の断りを受けることがない）
    let old = std::fs::read(&key_path).unwrap();
    let bridge = {
        let name = name.clone();
        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(20);
            while Instant::now() < deadline {
                match connect_and_greet(&name, "試験のブリッジ") {
                    Ok((_, _, welcome)) => return Ok(welcome.session),
                    Err(link::LinkError::Rejected(r)) if r.code == RejectCode::Unauthorized => {
                        return Err(r.text)
                    }
                    Err(_) => thread::sleep(Duration::from_micros(200)),
                }
            }
            Err("つなげない".to_owned())
        })
    };
    thread::sleep(Duration::from_millis(20));
    let server = Server::bind(&name, false).expect("落ちた後は同じ名前を使える");
    assert_ne!(std::fs::read(&key_path).unwrap(), old, "鍵は新しくなる");
    // 新しいスタンドアロンに、新しい鍵でつなげる
    let handle = thread::spawn(move || loop {
        let stream = server.accept().unwrap();
        if accept(stream, "新しいスタンドアロン", 3, &server.key()).is_ok() {
            return;
        }
    });
    let session = bridge.join().unwrap().expect("鍵の断りを受けずにつなげる");
    assert_eq!(session, 3);
    handle.join().unwrap();
}

/// 落ちたスタンドアロンの跡（古い鍵と、待ち受けのないソケット）を、新しいスタンドアロンが引き継ぐ間にも、つなぎ続けるブリッジは
/// 鍵の断りを受けない。ブリッジが鍵を読んでからつなぐまでの隙に引き継ぎが入る機会を増やすため、引き継ぎをいくつものスレッドで重ねて
/// 繰り返す（1 回の引き継ぎで隙に入る確率は小さく、1 本の試験では負荷の高い全件の中で 1 度落ちる程度だった）。
#[cfg(unix)]
#[test]
fn a_takeover_never_makes_the_bridge_greet_with_a_key_older_than_the_listener() {
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn takeover() -> Result<(), String> {
        let name = unique_name("takeover");
        let key_path = yolu_protocol::auth::key_path(&name).unwrap();
        let socket = link::socket_path(&name).unwrap();
        // 落ちたプロセスが残すもの: ファイルだけが残ったソケットと、その時の鍵
        LinkKey::generate().unwrap().write_file(&key_path).unwrap();
        drop(UnixListener::bind(&socket).unwrap());
        let reached_stale = Arc::new(AtomicBool::new(false));
        let bridge = {
            let (name, reached_stale) = (name.clone(), reached_stale.clone());
            thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(20);
                while Instant::now() < deadline {
                    match connect_and_greet(&name, "試験のブリッジ") {
                        Ok((_, _, welcome)) => return Ok(welcome.session),
                        Err(link::LinkError::Rejected(r)) if r.code == RejectCode::Unauthorized => {
                            return Err(r.text)
                        }
                        Err(_) => {
                            reached_stale.store(true, Ordering::Release);
                            thread::yield_now();
                        }
                    }
                }
                Err("つなげない".to_owned())
            })
        };
        // ブリッジが古い跡に当たり始めてから引き継ぐ
        while !reached_stale.load(Ordering::Acquire) {
            thread::yield_now();
        }
        let server = Server::bind(&name, false).expect("落ちた後は同じ名前を使える");
        let accepted = thread::spawn(move || loop {
            let stream = server.accept().unwrap();
            if accept(stream, "新しいスタンドアロン", 3, &server.key()).is_ok() {
                return;
            }
        });
        let result = bridge.join().unwrap();
        if result.is_ok() {
            accepted.join().unwrap();
        }
        // 断られたときは、受け付けのスレッドが次のつながりを待ち続ける。試験は落ちるので、そのままにする
        let _ = std::fs::remove_file(&socket);
        let _ = std::fs::remove_file(socket.with_extension("lock"));
        result.map(|_| ())
    }

    const THREADS: usize = 16;
    const TAKEOVERS: usize = 100;
    let failures: Vec<String> = thread::scope(|scope| {
        let workers: Vec<_> = (0..THREADS)
            .map(|_| {
                scope.spawn(|| {
                    (0..TAKEOVERS)
                        .filter_map(|_| takeover().err())
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap())
            .collect()
    });
    assert!(
        failures.is_empty(),
        "{} / {} 回の引き継ぎで鍵の断りを受けた: {:?}",
        failures.len(),
        THREADS * TAKEOVERS,
        failures.first()
    );
}

/// 待ち受けのソケットが現れた時には、鍵はもう新しい（古い鍵 + 新しいソケットの組をブリッジに見せない）。
#[cfg(unix)]
#[test]
fn the_key_is_replaced_before_the_socket_appears() {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    let name = unique_name("order");
    let key_path = yolu_protocol::auth::key_path(&name).unwrap();
    // 落ちたスタンドアロンが残した古い鍵
    LinkKey::generate().unwrap().write_file(&key_path).unwrap();
    let stale = std::fs::read(&key_path).unwrap();
    let socket = link::socket_path(&name).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let watcher = {
        let (key_path, stop) = (key_path.clone(), stop.clone());
        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !socket.exists() {
                if stop.load(Ordering::Relaxed) || Instant::now() > deadline {
                    return None;
                }
                std::hint::spin_loop();
            }
            std::fs::read(&key_path).ok()
        })
    };
    let server = Server::bind(&name, false).unwrap();
    let seen = watcher.join().unwrap().expect("ソケットが現れる");
    stop.store(true, Ordering::Relaxed);
    assert_ne!(seen, stale, "ソケットが現れた時の鍵は古いまま");
    assert_eq!(seen, std::fs::read(&key_path).unwrap());
    drop(server);
}

/// 待ち受けを始められなかったら、置いた鍵は残さない（鍵だけあって待ち受けていない状態にしない）。
#[cfg(unix)]
#[test]
fn a_key_is_not_left_when_the_listener_cannot_start() {
    let name = unique_name("nolisten");
    let key_path = yolu_protocol::auth::key_path(&name).unwrap();
    LinkKey::generate().unwrap().write_file(&key_path).unwrap();
    // ソケットの場所にフォルダがあると、片付けられず、待ち受けを始められない
    let socket = link::socket_path(&name).unwrap();
    std::fs::create_dir(&socket).unwrap();
    let result = Server::bind(&name, false);
    let key_left = key_path.exists();
    std::fs::remove_dir(&socket).unwrap();
    let _ = std::fs::remove_file(&key_path);
    assert!(result.is_err(), "待ち受けられないはず");
    assert!(!key_left, "待ち受けていないのに鍵が残っている");
}

/// 子のプロセスとして、環境変数（TMPDIR）の違うスタンドアロンになる（つながりを 1 つ受けたら終わる）。YLP_LINK_ENV_STANDALONE が無ければ何もしない。
#[test]
fn child_env_standalone() {
    let Ok(name) = std::env::var("YLP_LINK_ENV_STANDALONE") else {
        return;
    };
    let server = Server::bind(&name, false).unwrap();
    let stream = server.accept().unwrap();
    accept(stream, "子のスタンドアロン", 5, &server.key()).unwrap();
}

/// 子のプロセスとして、環境変数（TMPDIR）の違うブリッジになる。YLP_LINK_ENV_BRIDGE が無ければ何もしない。
#[test]
fn child_env_bridge() {
    let Ok(name) = std::env::var("YLP_LINK_ENV_BRIDGE") else {
        return;
    };
    let (_, _, welcome) = wait::connect(&name, None);
    assert_eq!(welcome.session, 5);
}

/// スタンドアロンとブリッジで TMPDIR が違い、XDG_RUNTIME_DIR が無くても、ブリッジが鍵とソケットを見つける
/// （/run/user/<UID> が使えない環境で、一時フォルダの置き場が両者で食い違わない）。
#[cfg(unix)]
#[test]
fn the_bridge_finds_the_standalone_when_tmpdir_differs() {
    let name = unique_name("tmpdir");
    let base = std::env::temp_dir().join(format!("ylp-tmpdir-{}", std::process::id()));
    let (a, b) = (base.join("a"), base.join("b"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let child = |test: &str, var: &str, tmp: &std::path::Path| {
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture", "--test-threads=1"])
            .env(var, &name)
            .env("TMPDIR", tmp)
            .env_remove("XDG_RUNTIME_DIR")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap()
    };
    let mut standalone = child("child_env_standalone", "YLP_LINK_ENV_STANDALONE", &a);
    let mut bridge = child("child_env_bridge", "YLP_LINK_ENV_BRIDGE", &b);
    let bridge_status = bridge.wait().unwrap();
    if !bridge_status.success() {
        let _ = standalone.kill();
    }
    let standalone_status = standalone.wait().unwrap();
    let _ = std::fs::remove_dir_all(&base);
    assert!(bridge_status.success(), "ブリッジが見つけられない");
    assert!(standalone_status.success(), "スタンドアロンが受けられない");
}

/// 落ちたスタンドアロンが残した共有メモリのファイルは、次に待ち受けを始めるときに片付く（生きている書き手のファイルは残る）。
#[test]
fn a_new_standalone_clears_the_images_a_crashed_one_left() {
    use yolu_protocol::shm::{FILE_PREFIX, FILE_SUFFIX};
    let old = std::time::SystemTime::now() - Duration::from_secs(3600);
    let age = |path: &std::path::Path| {
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(old)
            .unwrap();
    };
    // 生きている書き手（このプロセス）のファイルがある置き場に、落ちた書き手のファイルを置く
    let live = yolu_protocol::SharedImageWriter::create(
        &format!("p{}-rsweep-s1-t0-c0-n0", std::process::id()),
        32,
        32,
        16,
        0,
        0,
    )
    .unwrap();
    let dir = live.path().parent().unwrap().to_path_buf();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--list")
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let dead = child.id();
    child.wait().unwrap();
    let stale = dir.join(format!(
        "{FILE_PREFIX}p{dead}-rcrashed-s1-t0-c0-n0{FILE_SUFFIX}"
    ));
    std::fs::write(&stale, vec![0u8; 4096]).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    age(&stale);
    age(live.path());
    let name = unique_name("sweep");
    let server = Server::bind(&name, false).unwrap();
    assert!(!stale.exists(), "落ちた書き手のファイルが残っている");
    assert!(live.path().exists(), "生きている書き手のファイルを消した");
    drop(server);
}

/// 別のユーザー（nobody）として、つなぐ・鍵や画素のファイルを開く、が断られること。sudo が使えない環境では確かめずに戻る。
#[cfg(unix)]
#[test]
fn another_user_can_neither_connect_nor_read_the_files() {
    let sudo_ok = Command::new("sudo")
        .args(["-n", "-u", "nobody", "true"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !sudo_ok {
        eprintln!("sudo で別のユーザーになれないので確かめません");
        return;
    }
    let name = unique_name("other");
    let server = Server::bind(&name, false).unwrap();
    let socket = link::socket_path(&name).unwrap();
    let key = yolu_protocol::auth::key_path(&name).unwrap();
    // 画素のファイル（置き場のフォルダ自体は開けておき、ファイルの権限だけで守られることを確かめる）
    let open_dir = std::env::temp_dir().join(format!("ylp-open-{}", std::process::id()));
    std::fs::create_dir_all(&open_dir).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&open_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let image =
        yolu_protocol::SharedImageWriter::create_in(&open_dir, "other", 32, 32, 16, 0, 0).unwrap();
    let script = r#"
import socket, sys
socket_path, key_path, image_path = sys.argv[1:4]
def denied(f):
    try:
        f()
    except PermissionError:
        return True
    except OSError as e:
        return e.errno in (13, 1)
    return False
def connect():
    s = socket.socket(socket.AF_UNIX)
    s.connect(socket_path)
checks = {
    "socket": denied(connect),
    "key": denied(lambda: open(key_path, "rb")),
    "image": denied(lambda: open(image_path, "rb")),
}
print(checks)
sys.exit(0 if all(checks.values()) else 1)
"#;
    let out = Command::new("sudo")
        .args(["-n", "-u", "nobody", "python3", "-c", script])
        .arg(&socket)
        .arg(&key)
        .arg(image.path())
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    drop(image);
    drop(server);
    let _ = std::fs::remove_dir_all(&open_dir);
    assert!(out.status.success(), "別のユーザーが入れた: {text}");
}

/// 挨拶を送らない接続は時間切れで落とし、次の接続を受ける（Unix は読みの時間切れ、Windows は挨拶の見張りが読みを取り消す）。
#[test]
fn a_connection_that_never_greets_is_dropped_after_the_timeout_and_the_next_one_is_served() {
    let name = unique_name("silent");
    let server = Server::bind(&name, false).unwrap();
    let handle = thread::spawn(move || {
        let started = Instant::now();
        let first = server.accept().unwrap();
        let silent = link::accept_with(
            first,
            "試験のスタンドアロン",
            1,
            &server.key(),
            Duration::from_millis(300),
            &|_| Ok(()),
        );
        let waited = started.elapsed();
        let second = server.accept().unwrap();
        let served = accept(second, "試験のスタンドアロン", 2, &server.key()).map(|_| ());
        (silent.map(|_| ()), waited, served)
    });
    // 何も送らないつなぎ（同じユーザーの行儀の悪いプログラム）
    let silent = link::connect(&name).unwrap();
    // 時間が過ぎた後、本物のブリッジが挨拶できる
    thread::sleep(Duration::from_millis(500));
    let (conn, _reader, welcome) = wait::connect(&name, None);
    assert_eq!(welcome.session, 2);
    let _ = conn.send(&Message::Bye);
    let (first, waited, second) = handle.join().unwrap();
    assert!(
        matches!(first, Err(LinkError::Protocol(ref t)) if t.contains("挨拶が来ません")),
        "{first:?}"
    );
    assert!(
        waited >= Duration::from_millis(250) && waited < Duration::from_secs(5),
        "待った時間 {waited:?}"
    );
    assert!(second.is_ok());
    drop(silent);
}

/// つないでも何も返さない待ち受けには、ブリッジが決めた時間で見切りをつける（返事を待ち続けて、Unity の主のスレッドの裏で止まらない）。
#[test]
fn a_standalone_that_accepts_but_never_answers_makes_the_bridge_give_up_after_the_timeout() {
    let name = unique_name("mute");
    let server = Server::bind(&name, false).unwrap();
    let handle = thread::spawn(move || {
        // つなぎを受けて、何も返さずに持つ（ブリッジが諦めて閉じるまで）
        let stream = server.accept().unwrap();
        thread::sleep(Duration::from_millis(1500));
        drop(stream);
        server
    });
    let started = Instant::now();
    let result = link::connect_and_greet_within(
        &name,
        &Identity::unity("試験のブリッジ"),
        Duration::from_millis(300),
    );
    let waited = started.elapsed();
    assert!(
        matches!(result, Err(LinkError::Protocol(ref t)) if t.contains("挨拶の返事が来ません")),
        "{:?}",
        result.map(|_| ())
    );
    assert!(
        waited >= Duration::from_millis(250) && waited < Duration::from_secs(5),
        "待った時間 {waited:?}"
    );
    let _server = handle.join().unwrap();
}

/// Windows: パイプの相手のプロセスの持ち主を確かめる（自分のプロセスなら通る）。なりすましの段は匿名で、つないだ先は読んだ後でもなりすませない。
#[cfg(windows)]
mod windows_pipe {
    use super::*;
    use interprocess::local_socket::Stream;
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::System::Threading::{GetCurrentThread, OpenThreadToken};
    use yolu_protocol::private::win;

    #[test]
    fn the_owner_of_the_pipe_peer_is_my_own_account() {
        let me = win::user_sid().unwrap();
        // 自分のプロセスの持ち主は、自分のアカウント（相手の番号から持ち主を引ける）
        assert_eq!(win::process_user_sid(std::process::id()).unwrap(), me);
        // 存在しない番号は引けない（確かめられない相手は信じない）
        assert!(win::process_user_sid(0xffff_fff0).is_err());
        // 本物のパイプで、挨拶の前の確かめを通ってつながる（サーバーの側もクライアントの側も、相手は自分のプロセス）
        let name = unique_name("owner");
        let server = Server::bind(&name, false).unwrap();
        let handle = thread::spawn(move || {
            let stream = server.accept().unwrap();
            let r = accept(stream, "試験のスタンドアロン", 1, &server.key()).map(|(c, ..)| c);
            (r, server)
        });
        let (conn, _reader, welcome) = connect_and_greet(&name, "試験のブリッジ").unwrap();
        assert_eq!(welcome.session, 1);
        let _ = conn.send(&Message::Bye);
        let (accepted, _server) = handle.join().unwrap();
        assert!(accepted.is_ok());
    }

    /// つないだ先が、読んだ後に ImpersonateNamedPipeClient しても、こちらのアカウントでは動けない（匿名）。Wine は段を保存しないので確かめない。
    #[test]
    fn the_server_cannot_impersonate_the_bridge_after_reading() {
        let name = unique_name("sqos");
        let server = Server::bind(&name, false).unwrap();
        let handle = thread::spawn(move || {
            use std::io::Read;
            let stream = server.accept().unwrap();
            // クライアントが 1 バイト書くまで待つ（読んだ後でないとなりすませない）
            let mut one = [0u8; 1];
            (&stream).read_exact(&mut one).unwrap();
            let Stream::NamedPipe(pipe) = &stream;
            let guard = pipe.inner().impersonate_client();
            let outcome = match guard {
                // なりすませなかった: 匿名より上の段は得られていない
                Err(e) => format!("failed: {e}"),
                Ok(_guard) => {
                    let mut token = std::ptr::null_mut();
                    let opened = unsafe { OpenThreadToken(GetCurrentThread(), 0x0008, 0, &mut token) };
                    if opened == 0 {
                        format!("anonymous: {}", unsafe { GetLastError() })
                    } else {
                        unsafe { windows_sys::Win32::Foundation::CloseHandle(token) };
                        "token".to_owned()
                    }
                }
            };
            (outcome, server)
        });
        let mut stream = link::connect(&name).unwrap();
        use std::io::Write;
        stream.write_all(&[1]).unwrap();
        let (outcome, _server) = handle.join().unwrap();
        if win::is_wine() {
            eprintln!("Wine はなりすましの段を保存しないので確かめません（{outcome}）");
            return;
        }
        assert!(
            outcome.starts_with("failed") || outcome.starts_with("anonymous"),
            "匿名の段なら、なりすましたトークンを開けない: {outcome}"
        );
    }
}
