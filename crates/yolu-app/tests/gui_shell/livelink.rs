//! Live Link（スタンドアロンの側）: 本物のソケットと共有メモリで、Unity の役からモデルを受け、マテリアルごとのセットを出し、描いた所の
//! タイルだけを返す。細かい振る舞いは試験のスレッドが Unity の役（yolu-protocol）をする。通しの試験は、本物のブリッジ（yolu-bridge の
//! C の口。Unity の C# が呼ぶもの）を別のプロセス（この試験の実行ファイルを子として起こす）で動かす。
use crate::common;
use crate::common::wait;
use wait::{ChildGuard, WATCHDOG};

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use common::*;
use egui_kittest::Harness;
use yolu_app::engine::composite_pixel;
use yolu_app::livelink::LinkStatus;
use yolu_app::state::Action;
use yolu_app::YoluApp;
use yolu_protocol::link::{connect_and_greet, connect_and_greet_as, LinkError};
use yolu_protocol::*;

fn unique_name(tag: &str) -> String {
    crate::common::names::unique_name("ylapp", tag)
}

/// 通知をUIへ反映するためフレームを進める。期限は性能条件ではなく、通信のハング検出用。
fn step_until(h: &mut Harness<'_, YoluApp>, what: &str, mut cond: impl FnMut(&YoluApp) -> bool) {
    let deadline = Instant::now() + WATCHDOG;
    loop {
        h.step();
        if cond(h.state()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what} を待ったが来ない: 接続={:?}, メッセージ={}",
            h.state().state.link.status,
            h.state().state.message
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn listen(h: &mut Harness<'_, YoluApp>, tag: &str) -> String {
    let name = unique_name(tag);
    h.state_mut().link_mut().set_name(&name).unwrap();
    h.state_mut().state.apply(Action::ToggleLiveLink);
    h.run();
    assert_eq!(h.state().state.link.status, LinkStatus::Listening);
    name
}

fn material(name: &str, size: u32, color_route: bool) -> MaterialInfo {
    MaterialInfo {
        key: MaterialKey::Material {
            name: name.into(),
            asset: Some(("0123456789abcdef0123456789abcdef".into(), name.len() as i64)),
        },
        shader: "Standard".into(),
        textures: vec![TextureProperty {
            name: "_MainTex".into(),
            width: size,
            height: size,
        }],
        routes: if color_route {
            vec![ChannelRoute {
                channel: channel::COLOR,
                property: "_MainTex".into(),
            }]
        } else {
            vec![]
        },
    }
}

fn model(generation: u32, materials: Vec<MaterialInfo>) -> Model {
    let n = materials.len() as u32;
    Model {
        generation,
        name: "試しの四角".into(),
        materials,
        meshes: vec![MeshData {
            key: "0".into(),
            name: "Quad".into(),
            skinned: true,
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]],
            normals: vec![],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
            submeshes: (0..n)
                .map(|m| Submesh {
                    material: m,
                    indices: if m % 2 == 0 {
                        vec![0, 2, 1]
                    } else {
                        vec![1, 2, 3]
                    },
                })
                .collect(),
        }],
    }
}

/// 試験の四角（`model`）は Unity の Quad と同じく −Z 側が表。モデルを受けた直後の既定のカメラはモデルの前（+Z 側）から見るので、
/// 裏から見て面が隠れる。四角の表を斜めに見る位置（これまでの既定と同じ yaw 25°・pitch 10°）へ明示する（カメラはモデルを替えるまで動かない）。
fn look_at_the_quad_front(h: &mut Harness<'_, YoluApp>) {
    let camera = &mut h.state_mut().state.view3d.camera;
    camera.yaw = 25.0;
    camera.pitch = 10.0;
    h.run();
}

/// 1 フレーム進めるもの（画面ありの Harness と、画面なしの Headless）。
trait Frames {
    fn next_frame(&mut self);
}

impl Frames for Harness<'_, YoluApp> {
    fn next_frame(&mut self) {
        self.step();
    }
}

/// Unity の役（試験のスレッド。yolu-protocol で直につなぐ）。
struct FakeUnity {
    conn: Connection,
    /// 読むスレッドが受けたもの（Windows は受けの時間切れが効かないので、読みは裏のスレッドで待つ）。
    rx: mpsc::Receiver<Result<Received, String>>,
}

impl FakeUnity {
    fn connect(name: &str) -> FakeUnity {
        // 版を名乗る Unity（名乗らない古いブリッジだと、入口の印は版のずれの警告の色になる。link_version.rs）
        // 機能の印はスタンドアロンと同じ（印のずれも警告になるので、印を立てる機能が増えても色は変わらない）
        let identity = Identity::unity("試験の Unity")
            .with_version(Some(AppVersion::new(0, 3, 0)))
            .with_features(yolu_app::livelink::FEATURES);
        let (conn, mut reader, welcome) = connect_and_greet_as(name, &identity).unwrap();
        assert_eq!(welcome.version, PROTOCOL_VERSION);
        assert!(welcome.agent.starts_with("YoluPainter"));
        let (tx, rx) = mpsc::channel();
        let reply = conn.clone();
        std::thread::spawn(move || loop {
            match reader.next(&reply) {
                Ok(Received::Idle) => {}
                Ok(r) => {
                    if tx.send(Ok(r)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    break;
                }
            }
        });
        FakeUnity { conn, rx }
    }

    /// 送る。モデルのあとには、元の絵を送る Unity と同じに、来るはずの元の絵（Color の流し込み先に絵のあるマテリアル）の様子も送る
    /// （この試験の Unity の役は元の絵を読めない: 画素なし。元の絵を待つセットが、いつまでも Unity に出ないままにならない）。
    fn send(&self, m: Message) {
        self.conn.send(&m).unwrap();
        if let Message::Model(model) = &m {
            for (i, info) in model.materials.iter().enumerate() {
                if let Some(slot) = yolu_app::livelink_base::expected_slot(info) {
                    self.conn
                        .send(&Message::MaterialOriginal(MaterialOriginal {
                            generation: model.generation,
                            material: i as u32,
                            slot,
                            state: OriginalState::Unreadable,
                            read: OriginalRead::File,
                            compressed: false,
                            width: 0,
                            height: 0,
                            srgb: true,
                            pixels: Vec::new(),
                            stamp: 0,
                        }))
                        .unwrap();
                }
            }
        }
    }

    /// フレームを進めながら、条件に合う命令が来るまで集める（集めた全部を返す）。
    fn collect_until(
        &mut self,
        h: &mut impl Frames,
        what: &str,
        mut done: impl FnMut(&[Message]) -> bool,
    ) -> Vec<Message> {
        let deadline = Instant::now() + WATCHDOG;
        let mut got = Vec::new();
        let mut closed = None;
        loop {
            h.next_frame();
            while let Ok(r) = self.rx.try_recv() {
                match r {
                    Ok(Received::Message(m)) => got.push(m),
                    Ok(other) => panic!("{other:?}"),
                    Err(e) => closed = Some(e),
                }
            }
            if done(&got) {
                return got;
            }
            assert!(
                closed.is_none() && Instant::now() < deadline,
                "{what} が来ない: {got:?}（切れた: {closed:?}）"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

fn read_image(path: &str, w: u32, h: u32) -> Vec<u8> {
    let img = SharedImageReader::open(std::path::Path::new(path)).unwrap();
    let ts = img.layout().tile_size;
    let mut image = vec![0u8; (w * h * 4) as usize];
    for y in 0..h.div_ceil(ts) {
        for x in 0..w.div_ceil(ts) {
            // 通知の発行までフレームを進めた後は、次のフレームまで書き手は更新しない。
            assert_eq!(
                img.read_tile_into_image(x, y, &mut image).unwrap(),
                TileRead::Complete
            );
        }
    }
    image
}

fn pixel(image: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * w + x) * 4) as usize;
    [image[i], image[i + 1], image[i + 2], image[i + 3]]
}

#[test]
fn a_model_becomes_texture_sets_and_strokes_come_back_as_changed_tiles() {
    let mut h = app(1280.0, 800.0, 256);
    // つなぐ前に描いておく（つないだら全部を出す）
    let c = canvas_rect(&h).center();
    drag(&mut h, &[offset(c, -40.0, 0.0), offset(c, 40.0, 0.0)]);
    let name = listen(&mut h, "model");
    let mut unity = FakeUnity::connect(&name);
    step_until(&mut h, "つながった", |a| {
        matches!(a.state.link.status, LinkStatus::Connected { .. })
    });
    assert!(h.state().state.message.contains("つながりました"));

    unity.send(Message::Model(model(
        1,
        vec![
            material("Body", 512, true),
            material("NoColor", 1024, false),
        ],
    )));
    let got = unity.collect_until(&mut h, "テクスチャセット", |m| {
        m.iter().any(|m| matches!(m, Message::TextureSet(_)))
    });
    look_at_the_quad_front(&mut h);
    let s = &h.state().state;
    let names: Vec<&str> = s.sets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        ["Body", "NoColor"],
        "最初のセットは Body に付き、NoColor は新しいセット"
    );
    assert_eq!(
        s.set_doc(1).width(),
        yolu_app::state::DEFAULT_DOCUMENT_SIZE,
        "Color の流し込み先が無ければ、どのテクスチャの大きさか分からないので既定の大きさ"
    );
    let sets: Vec<&TextureSet> = got
        .iter()
        .filter_map(|m| match m {
            Message::TextureSet(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(
        sets.len(),
        1,
        "Color の流し込み先の無いマテリアルは出さない: {got:?}"
    );
    let set = sets[0].clone();
    assert_eq!(
        (set.generation, set.material, set.width, set.tile_size),
        (1, 0, 256, 128)
    );
    assert_eq!(set.name, "Body");
    assert!(
        !got.iter().any(|m| matches!(m, Message::TilesChanged(_))),
        "知らせる前に全部を書いてあるので、タイルの知らせは要らない"
    );
    let image = read_image(&set.channels[0].path, 256, 256);
    assert_eq!(
        pixel(&image, 256, 128, 128),
        [0, 0, 0, 255],
        "つなぐ前に描いた線"
    );
    assert_eq!(pixel(&image, 256, 5, 5), [0, 0, 0, 0]);
    // 画面: Body は Unity に見せている（同期のアイコン）、NoColor は流し込み先が無い（注意）。状態の帯はつながっている
    assert_eq!(
        yolu_app::panels::texture_sets::set_state(&h.state().state, 0).map(|s| s.icon),
        Some("sync")
    );
    h.run();
    h.snapshot("livelink_connected");

    // 描くと、変わったタイルだけが返る
    let p = offset(c, 50.0, 60.0);
    drag(&mut h, &[p, offset(p, 4.0, 0.0)]);
    let got = unity.collect_until(&mut h, "変わったタイル", |m| {
        m.iter().any(|m| matches!(m, Message::TilesChanged(_)))
    });
    let tiles: Vec<Tile> = got
        .iter()
        .filter_map(|m| match m {
            Message::TilesChanged(t) => {
                assert_eq!((t.set, t.channel), (set.set, channel::COLOR));
                Some(t.tiles.clone())
            }
            _ => None,
        })
        .flatten()
        .collect();
    assert!((1..=2).contains(&tiles.len()), "{tiles:?}");
    let image = read_image(&set.channels[0].path, 256, 256);
    let doc = &h.state().state.doc;
    for t in &tiles {
        for (x, y) in [(0, 0), (64, 64), (127, 127)] {
            let (x, y) = (t.x as u32 * 128 + x, t.y as u32 * 128 + y);
            assert_eq!(pixel(&image, 256, x, y), composite_pixel(doc, x, y));
        }
    }
    assert!(h.state().state.link.tiles_sent >= 4);

    // ポーズ: 3D ビューの形に当て、合わないものは何も変えずに断る
    let before = h.state().state.view3d.model.as_ref().unwrap().revision();
    unity.send(Message::Pose(Pose {
        generation: 1,
        meshes: vec![MeshPose {
            mesh: 0,
            positions: vec![[2.0; 3]; 4],
            normals: vec![],
        }],
    }));
    step_until(&mut h, "ポーズ", |a| {
        a.state.view3d.model.as_ref().unwrap().revision() != before
    });
    let pos = |h: &Harness<'_, YoluApp>| {
        h.state().state.view3d.model.as_ref().unwrap().meshes[0].positions[3]
    };
    assert_eq!(pos(&h), yolu_core::glam::Vec3::splat(2.0));
    // Unity へ返す誤りの返事は、画面の言語（ここは English）に依らず日本語の診断に固定
    h.state_mut().state.lang = yolu_app::lang::Lang::En;
    unity.send(Message::Pose(Pose {
        generation: 1,
        meshes: vec![MeshPose {
            mesh: 0,
            positions: vec![[3.0; 3]; 3],
            normals: vec![],
        }],
    }));
    let got = unity.collect_until(&mut h, "ポーズを断る知らせ", |m| {
        m.iter().any(|m| matches!(m, Message::Error(_)))
    });
    assert!(got.iter().any(|m| matches!(
        m,
        Message::Error(e) if e.code == ErrorCode::Refused && e.text == "ポーズの頂点の数がメッシュと違います"
    )), "{got:?}");
    h.state_mut().state.lang = yolu_app::lang::Lang::Ja;
    assert_eq!(pos(&h), yolu_core::glam::Vec3::splat(2.0));

    // 目を閉じると Unity から外し、開くとまた出す
    let uid = h.state().state.sets.get(0).unwrap().uid;
    h.state_mut().state.apply(Action::ToggleSetVisible(uid));
    let got = unity.collect_until(&mut h, "外す知らせ", |m| {
        m.iter()
            .any(|m| matches!(m, Message::TextureSetRemoved { .. }))
    });
    assert!(got.contains(&Message::TextureSetRemoved { set: set.set }));
    h.state_mut().state.apply(Action::ToggleSetVisible(uid));
    unity.collect_until(&mut h, "出し直し", |m| {
        m.iter().any(|m| matches!(m, Message::TextureSet(_)))
    });

    // 知らない命令は断ってつながりを保つ
    unity.conn.send_raw(0x7777, &[1, 2, 3]).unwrap();
    let got = unity.collect_until(&mut h, "知らない命令の知らせ", |m| {
        m.iter().any(|m| matches!(m, Message::Error(_)))
    });
    assert!(got.iter().any(
        |m| matches!(m, Message::Error(e) if e.code == ErrorCode::UnknownCommand && e.kind == 0x7777)
    ));
    // 返事は読むスレッドがすぐ返す。画面の知らせは次のフレームで読む
    step_until(&mut h, "知らない命令の知らせ（画面）", |a| {
        a.state.message.contains("知らない命令")
    });

    // 送り直したモデル（世代 2）では、同じセットを新しい世代で知らせ直す（鍵で同じマテリアルと分かる）
    unity.send(Message::Model(model(
        2,
        vec![material("NoColor", 1024, true), material("Body", 512, true)],
    )));
    let got = unity.collect_until(&mut h, "世代 2 のセット", |m| {
        m.iter()
            .filter(|m| matches!(m, Message::TextureSet(t) if t.generation == 2))
            .count()
            == 2
    });
    for m in &got {
        if let Message::TextureSet(t) = m {
            match t.name.as_str() {
                "Body" => assert_eq!((t.set, t.material), (set.set, 1)),
                "NoColor" => assert_eq!((t.material, t.width), (0, 2048)),
                other => panic!("{other}"),
            }
        }
    }
    assert_eq!(h.state().state.sets.len(), 2, "セットは増えない");

    // モデルを閉じたらセットを外す（セットは残す）
    unity.send(Message::ModelClosed { generation: 2 });
    let got = unity.collect_until(&mut h, "モデルを閉じた", |m| {
        m.iter()
            .filter(|m| matches!(m, Message::TextureSetRemoved { .. }))
            .count()
            == 2
    });
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(h.state().state.model.is_none());
    assert_eq!(h.state().state.sets.len(), 2);

    // Unity が切ったら待ち受けに戻る
    unity.send(Message::Bye);
    step_until(&mut h, "切れた", |a| {
        a.state.link.status == LinkStatus::Listening
    });
    assert!(h.state().state.message.contains("Unity が切りました"));
}

/// 鍵を知っているブリッジの挨拶の鍵の欄。
fn hello_auth(key: &yolu_protocol::LinkKey) -> yolu_protocol::HelloAuth {
    let nonce = yolu_protocol::auth::random_bytes().unwrap();
    yolu_protocol::HelloAuth {
        nonce,
        proof: key.hello_proof(&nonce),
    }
}

#[test]
fn a_connection_without_the_right_key_is_refused_and_noted_while_the_link_stays_up() {
    let mut h = app(1280.0, 800.0, 256);
    let name = listen(&mut h, "keys");
    let first = FakeUnity::connect(&name);
    step_until(&mut h, "つながった", |a| {
        matches!(a.state.link.status, LinkStatus::Connected { .. })
    });
    // 鍵の欄が無い（古いブリッジ）・別の鍵の挨拶は、つながっていても鍵の断りで返す（Busy を教えない）
    for auth in [
        None,
        Some(hello_auth(&yolu_protocol::LinkKey::generate().unwrap())),
    ] {
        let stream = link::connect(&name).unwrap();
        let mut s = &stream;
        s.write_all(&encode_message(&Message::Hello(Hello {
            min_version: 1,
            max_version: PROTOCOL_VERSION,
            agent: "知らない相手".into(),
            features: 0,
            auth,
            versions: None,
        })))
        .unwrap();
        let mut frames = FrameReader::new();
        let frame = frames.read_frame(&mut s).unwrap().unwrap();
        assert!(
            matches!(frame.decode().unwrap(), Message::Reject(r) if r.code == RejectCode::Unauthorized)
        );
    }
    step_until(&mut h, "鍵の断りの知らせ", |a| {
        a.state.message.contains("鍵の合わない")
    });
    assert!(
        matches!(h.state().state.link.status, LinkStatus::Connected { .. }),
        "つながっている Unity は切れない"
    );
    assert!(h.state().state.link.mismatch.is_none(), "版の不一致とは別");
    drop(first);
}

#[test]
fn a_connection_that_never_greets_does_not_hold_the_slot_for_unity() {
    let mut h = app(1280.0, 800.0, 256);
    let name = listen(&mut h, "silent");
    // 挨拶を送らないつなぎ（同じユーザーの行儀の悪いプログラム）が先にいても、Unity はつながれる（枠は挨拶が済んでから取る）
    let silent = link::connect(&name).unwrap();
    let unity = FakeUnity::connect(&name);
    step_until(&mut h, "つながった", |a| {
        matches!(a.state.link.status, LinkStatus::Connected { .. })
    });
    // つながっている間の 2 つ目は、鍵を知っていれば Busy で断る
    match connect_and_greet(&name, "2 つ目の Unity") {
        Err(LinkError::Rejected(r)) => assert_eq!(r.code, RejectCode::Busy),
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("2 つ目はつながらない"),
    }
    drop(unity);
    drop(silent);
}

#[test]
fn version_mismatch_and_a_second_unity_are_refused_and_shown() {
    let mut h = app(1280.0, 800.0, 256);
    let name = listen(&mut h, "refuse");
    // 版が合わない
    let stream = link::connect(&name).unwrap();
    let mut s = &stream;
    s.write_all(&encode_message(&Message::Hello(Hello {
        min_version: PROTOCOL_VERSION + 10,
        max_version: PROTOCOL_VERSION + 20,
        agent: "未来の Unity".into(),
        features: 0,
        auth: Some(hello_auth(&yolu_protocol::LinkKey::load(&name).unwrap())),
        versions: None,
    })))
    .unwrap();
    let mut frames = FrameReader::new();
    let frame = frames.read_frame(&mut s).unwrap().unwrap();
    assert!(
        matches!(frame.decode().unwrap(), Message::Reject(r) if r.code == RejectCode::VersionMismatch)
    );
    step_until(&mut h, "版の不一致", |a| {
        a.state.link.mismatch.is_some()
    });
    let s = &h.state().state;
    assert_eq!(s.link.status, LinkStatus::Listening, "待ち受けは続ける");
    assert!(s.message.contains("版の合わない"));
    // 入口の印は警告の色で、理由はツールチップ（状態の帯には出さない）
    assert_eq!(s.link.indicator(), yolu_app::livelink::LinkIndicator::Mismatch);
    assert_eq!(
        yolu_app::shell::link_indicator_color(s.link.indicator()),
        yolu_app::ui::theme::WARNING
    );
    let text = s.link.tooltip(s.lang);
    assert!(text.contains("版の合わない"), "{text}");
    h.snapshot("status_version_mismatch");

    // 1 つ目はつながり（不一致の印は消える）、2 つ目は Busy で断る
    let first = FakeUnity::connect(&name);
    step_until(&mut h, "つながった", |a| {
        matches!(a.state.link.status, LinkStatus::Connected { .. })
    });
    assert!(h.state().state.link.mismatch.is_none());
    match connect_and_greet(&name, "2 つ目の Unity") {
        Err(LinkError::Rejected(r)) => assert_eq!(r.code, RejectCode::Busy),
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("2 つ目はつながらない"),
    }
    step_until(&mut h, "断った知らせ", |a| {
        a.state.message.contains("2 つ目の Unity")
    });
    assert!(matches!(
        h.state().state.link.status,
        LinkStatus::Connected { .. }
    ));

    // メニューで切ると、Unity に Bye が届き、共有メモリも片付く
    first.send(Message::Model(model(1, vec![material("Body", 256, true)])));
    let mut first = first;
    let got = first.collect_until(&mut h, "セット", |m| {
        m.iter().any(|m| matches!(m, Message::TextureSet(_)))
    });
    let path = got
        .iter()
        .find_map(|m| match m {
            Message::TextureSet(t) => Some(t.channels[0].path.clone()),
            _ => None,
        })
        .unwrap();
    assert!(std::path::Path::new(&path).exists());
    let at = menu_title(&h, "ファイル").center();
    click(&mut h, at);
    let at = popup_item(&h, "Live Link").center();
    click(&mut h, at);
    let got = first.collect_until(&mut h, "Bye", |m| m.contains(&Message::Bye));
    assert_eq!(got.last(), Some(&Message::Bye));
    assert_eq!(h.state().state.link.status, LinkStatus::Off);
    assert!(
        !std::path::Path::new(&path).exists(),
        "共有メモリのファイルを消す"
    );
    assert!(link::connect(&name).is_err(), "待ち受けもやめた");
}

/// 子のプロセスの Unity の役（本物のブリッジの C の口を、Unity の C# と同じ順で呼ぶ）。YLAPP_CHILD_UNITY が無ければ何もしない。
/// 親とは標準入出力で話す（子は「UNITY: …」の行を書き、親の行を読んで次へ進む）。
#[test]
fn child_unity() {
    let Ok(name) = std::env::var("YLAPP_CHILD_UNITY") else {
        return;
    };
    use yolu_bridge::*;
    let say = |s: String| {
        println!("UNITY: {s}");
        std::io::stdout().flush().unwrap();
    };
    let mut stdin = BufReader::new(std::io::stdin());
    let mut wait_line = |want: &str| {
        let mut line = String::new();
        stdin.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), want);
    };
    let poll = |h: u64, what: &str, mut f: Box<dyn FnMut() -> bool + '_>| {
        let deadline = Instant::now() + WATCHDOG;
        while !f() {
            assert!(
                Instant::now() < deadline && matches!(ylb_status(h), 0 | 1),
                "{what}: 接続状態={}, 受信番号={}, セット数={}",
                ylb_status(h),
                ylb_serial(h),
                ylb_set_count(h)
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    unsafe {
        assert_eq!(ylb_abi_version(), 7);
        let agent = "子の Unity";
        // 本物の C の口で、Unity のパッケージの版を名乗る
        let version = "0.3.0";
        let h = ylb_connect_with(
            name.as_ptr(),
            name.len() as i32,
            agent.as_ptr(),
            agent.len() as i32,
            version.as_ptr(),
            version.len() as i32,
        );
        assert_ne!(h, 0);
        poll(h, "つながる", Box::new(|| ylb_status(h) == 1));
        say("connected".into());
        // モデル: Body（流し込み先あり・256）と Hair（流し込み先あり・512）
        let mname = "子のモデル";
        assert_eq!(ylb_model_begin(h, mname.as_ptr(), mname.len() as i32), 0);
        for (i, (m, size)) in [("Body", 256u32), ("Hair", 512)].iter().enumerate() {
            let (shader, guid, prop) = ("Standard", "fedcba9876543210fedcba9876543210", "_MainTex");
            let idx = ylb_model_material(
                h,
                0,
                m.as_ptr(),
                m.len() as i32,
                guid.as_ptr(),
                32,
                2100000 + i as i64,
                shader.as_ptr(),
                shader.len() as i32,
            );
            assert_eq!(idx, i as i32);
            assert_eq!(
                ylb_model_material_texture(h, idx, prop.as_ptr(), prop.len() as i32, *size, *size),
                0
            );
            assert_eq!(
                ylb_model_material_route(h, idx, 0, prop.as_ptr(), prop.len() as i32),
                0
            );
        }
        let pos: [f32; 12] = [0., 0., 0., 1., 0., 0., 0., 1., 0., 1., 1., 0.];
        let uv: [f32; 8] = [0., 0., 1., 0., 0., 1., 1., 1.];
        let (key, quad) = ("0", "Quad");
        let mesh = ylb_model_mesh(
            h,
            key.as_ptr(),
            1,
            quad.as_ptr(),
            quad.len() as i32,
            1,
            pos.as_ptr(),
            std::ptr::null(),
            uv.as_ptr(),
            4,
        );
        assert_eq!(mesh, 0);
        assert_eq!(ylb_model_submesh(h, 0, 0, [0, 2, 1].as_ptr(), 3), 0);
        assert_eq!(ylb_model_submesh(h, 0, 1, [1, 2, 3].as_ptr(), 3), 0);
        assert_eq!(ylb_model_send(h), 1);
        // スタンドアロンは、元の絵を待たせたセットのマテリアルを頼む（C の口の頼みの取り出しまで届く。世代 1・元の絵・手元の印なし）。
        // Unity は元の絵を自分から押し出さず、頼まれた分を送る
        let mut asked: Vec<(u32, u32, u32, u64, String)> = Vec::new();
        poll(
            h,
            "頼み",
            Box::new(|| {
                loop {
                    let mut r = YlbRequest::default();
                    let mut slot = [0u8; 64];
                    if ylb_next_request(h, &mut r, slot.as_mut_ptr(), slot.len() as i32) != 1 {
                        break;
                    }
                    if r.wants & 2 != 0 {
                        asked.push((
                            r.generation,
                            r.material,
                            r.wants,
                            r.have,
                            String::from_utf8(slot[..r.slot_len as usize].to_vec()).unwrap(),
                        ));
                    }
                }
                asked.len() >= 2
            }),
        );
        asked.sort();
        assert_eq!(
            asked,
            vec![
                (1, 0, 2, 0, "_MainTex".to_owned()),
                (1, 1, 2, 0, "_MainTex".to_owned())
            ]
        );
        // 元の絵（原本のファイルから読んだ、同じ大きさの平らな絵）。揃うまで、スタンドアロンはセットを出さない
        for (i, (size, color)) in [(256u32, [200u8, 100, 50, 255]), (512, [10, 200, 90, 255])]
            .iter()
            .enumerate()
        {
            let (slot, pixels) = ("_MainTex", color.repeat((size * size) as usize));
            assert_eq!(
                ylb_original_send(
                    h,
                    i as i32,
                    slot.as_ptr(),
                    slot.len() as i32,
                    0,
                    0,
                    0,
                    *size,
                    *size,
                    1,
                    0xA0 + i as u64,
                    pixels.as_ptr(),
                    pixels.len() as i32,
                ),
                1
            );
        }
        poll(h, "2 つのセット", Box::new(|| ylb_set_count(h) == 2));

        let mut infos = Vec::new();
        for i in 0..2 {
            let mut info = YlbSetInfo::default();
            assert_eq!(ylb_set_info(h, i, &mut info), 0);
            infos.push(info);
        }
        let copy = |info: &YlbSetInfo, image: &mut Vec<u8>| -> u32 {
            let mut r = YlbCopyResult::default();
            let n = ylb_copy_dirty(
                h,
                info.set,
                0,
                image.as_mut_ptr(),
                image.len() as u64,
                std::ptr::null_mut(),
                0,
                0,
                std::ptr::null_mut(),
                &mut r,
            );
            assert!(n >= 0, "{n}");
            n as u32
        };
        let mut images: Vec<Vec<u8>> = infos
            .iter()
            .map(|i| vec![0u8; (i.width * i.height * 4) as usize])
            .collect();
        for (info, image) in infos.iter().zip(images.iter_mut()) {
            let mut name = vec![0u8; 64];
            let n = ylb_set_name(h, info.set, name.as_mut_ptr(), 64) as usize;
            let tiles = copy(info, image);
            say(format!(
                "set {} {} {}x{} material {} tiles {tiles}",
                info.set,
                String::from_utf8_lossy(&name[..n]),
                info.width,
                info.height,
                info.material
            ));
        }
        // 親が Hair（2 つ目のセット）の (x, y) に描くのを待ち、そのタイルだけを写す
        wait_line("painted");
        let hair = infos[1];
        poll(
            h,
            "汚れたタイル",
            Box::new(|| ylb_channel_dirty(h, hair.set, 0) > 0),
        );
        let tiles = copy(&hair, &mut images[1]);
        let w = hair.width;
        let px = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            format!(
                "{} {} {} {}",
                images[1][i],
                images[1][i + 1],
                images[1][i + 2],
                images[1][i + 3]
            )
        };
        say(format!("hair tiles {tiles} center {}", px(w / 2, w / 2)));
        // ポーズ
        assert_eq!(ylb_pose_begin(h), 0);
        let moved: [f32; 12] = [0., 0., 1., 1., 0., 1., 0., 1., 1., 1., 1., 1.];
        assert_eq!(ylb_pose_mesh(h, 0, moved.as_ptr(), std::ptr::null(), 4), 0);
        assert_eq!(ylb_pose_send(h), 1);
        say("posed".into());
        wait_line("bye");
        assert_eq!(ylb_disconnect(h), 0);
        say("done".into());
    }
}

/// 子の試験（`child_unity`）の、libtest が使う名前。束の中ではモジュールの道筋（`livelink::`）がつく（クレート名の束の名前は除く）。
/// `--exact` で子を起こすので、名前が合わないと子は 1 件も走らずに終わる。
fn child_unity_name() -> String {
    match module_path!().split_once("::") {
        Some((_, inner)) => format!("{inner}::child_unity"),
        None => "child_unity".to_string(),
    }
}

/// 子のプロセスの Unity の役を起こす。返すのは子・子の標準入力・子の「UNITY: …」の行。
fn spawn_child_unity(name: &str) -> (ChildGuard, std::process::ChildStdin, mpsc::Receiver<String>) {
    let mut child = ChildGuard::spawn(
        Command::new(std::env::current_exe().unwrap())
            .args([child_unity_name().as_str(), "--exact", "--nocapture", "--test-threads=1"])
            .env("YLAPP_CHILD_UNITY", name)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped()),
    );
    let to_child = child.stdin.take().unwrap();
    let (tx, rx) = mpsc::channel::<String>();
    let out = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines() {
            let Ok(line) = line else { break };
            // libtest の「test child_unity ... 」と同じ行に付くことがある
            if let Some((_, rest)) = line.split_once("UNITY: ") {
                let _ = tx.send(rest.trim_end().to_owned());
            }
        }
    });
    (child, to_child, rx)
}

fn next_child_line(
    frames: &mut impl Frames,
    rx: &mpsc::Receiver<String>,
    child: &mut ChildGuard,
    what: &str,
) -> String {
    let deadline = Instant::now() + WATCHDOG;
    loop {
        frames.next_frame();
        match rx.try_recv() {
            Ok(line) => return line,
            Err(mpsc::TryRecvError::Disconnected) => panic!(
                "{what}: 子の標準出力が閉じた、pid={}, 状態={:?}",
                child.id(),
                child.try_wait()
            ),
            Err(mpsc::TryRecvError::Empty) => {}
        }
        // 子が終了していても、標準出力の読み手が最後の行を送るまで待つ。
        let status = child.try_wait().expect("子の状態を調べる");
        assert!(
            Instant::now() < deadline,
            "{what}: 子の通知が来ない、pid={}, 状態={status:?}",
            child.id()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_unity_bridge_in_another_process_sees_the_painted_tiles() {
    let mut h = app(1280.0, 800.0, 256);
    let name = listen(&mut h, "proc");
    let (mut child, mut to_child, rx) = spawn_child_unity(&name);
    // 子の行を待つあいだもフレームを進める（知らせを読み、タイルを出すのは画面のスレッド）
    assert_eq!(
        next_child_line(&mut h, &rx, &mut child, "接続完了"),
        "connected"
    );
    // 本物の C の口（ylb_connect_with）で名乗った Unity のパッケージの版が、挨拶でスタンドアロンに届く
    let peer = h.state().state.link.link.as_ref().map(|l| l.peer.app_version());
    assert_eq!(peer, Some(Some(AppVersion::new(0, 3, 0))));
    // 版は揃っている（子のブリッジの機能の印は、ブリッジが出す印で、スタンドアロンの印とは別に決まる）
    assert!(h
        .state()
        .state
        .link
        .skew()
        .is_none_or(|s| s.update_peer.is_none() && s.update_self.is_none()));
    let body = next_child_line(&mut h, &rx, &mut child, "Bodyの初回セット");
    let hair = next_child_line(&mut h, &rx, &mut child, "Hairの初回セット");
    let s = &h.state().state;
    assert_eq!(s.sets.len(), 2);
    assert_eq!(s.model.as_ref().unwrap().name, "子のモデル");
    let (body_uid, hair_uid) = (s.sets.get(0).unwrap().uid, s.sets.get(1).unwrap().uid);
    assert_eq!(
        body,
        format!("set {body_uid} Body 256x256 material 0 tiles 4"),
        "セットの番号はアプリのセットの uid。最初は全部のタイル"
    );
    assert_eq!(
        hair,
        format!("set {hair_uid} Hair 512x512 material 1 tiles 16")
    );

    // モデルが届くと 3D ビューのタブが前に出る（キャンバスは裏）
    assert!(h.state().view3d_rect().is_some(), "モデルが届いたら 3D ビューを前に出す");
    // Hair を選んで、キャンバスのタブを前に戻し、真ん中に描く
    h.state_mut().state.apply(Action::SelectSet(hair_uid));
    h.run();
    click_tab(&mut h, yolu_app::Tab::Canvas);
    assert_eq!(h.state().state.doc.width(), 512);
    let c = canvas_rect(&h).center();
    drag(&mut h, &[offset(c, -3.0, 0.0), offset(c, 3.0, 0.0)]);
    let expected = composite_pixel(&h.state().state.doc, 256, 256);
    assert_eq!(expected[3], 255);
    writeln!(to_child, "painted").unwrap();
    let line = next_child_line(&mut h, &rx, &mut child, "Hairの描画タイル");
    let (tiles, center) = line
        .strip_prefix("hair tiles ")
        .and_then(|r| r.split_once(" center "))
        .unwrap_or_else(|| panic!("{line}"));
    let tiles: u32 = tiles.parse().unwrap();
    assert!((1..=4).contains(&tiles), "描いた所のタイルだけ: {tiles}");
    assert_eq!(
        center,
        format!(
            "{} {} {} {}",
            expected[0], expected[1], expected[2], expected[3]
        )
    );
    assert_eq!(
        next_child_line(&mut h, &rx, &mut child, "ポーズ送信完了"),
        "posed"
    );
    step_until(&mut h, "ポーズ", |a| {
        a.state.view3d.model.as_ref().unwrap().meshes[0].positions[0]
            == yolu_core::glam::Vec3::new(0.0, 0.0, 1.0)
    });
    writeln!(to_child, "bye").unwrap();
    assert_eq!(next_child_line(&mut h, &rx, &mut child, "切断完了"), "done");
    step_until(&mut h, "切れた", |a| {
        a.state.link.status == LinkStatus::Listening
    });
    assert!(child.finish().success());
}

/// 画面（wgpu）を使わずに、AppState と LiveLink を画面のフレームと同じ順（頼み → 受ける → 出す）で回す。GPU の無い所・Windows 向けに
/// 組んで wine で回すとき用（名前が headless_ で始まる試験）。
struct Headless {
    ctx: egui::Context,
    state: yolu_app::state::AppState,
    link: yolu_app::livelink::LiveLink,
}

impl Frames for Headless {
    fn next_frame(&mut self) {
        self.frame();
    }
}

impl Headless {
    fn listen(size: u32, tag: &str) -> (Headless, String) {
        let name = unique_name(tag);
        let mut a = Headless {
            ctx: egui::Context::default(),
            state: yolu_app::state::AppState::new(size, size),
            link: yolu_app::livelink::LiveLink::new(),
        };
        a.link.set_name(&name).unwrap();
        a.state.apply(Action::ToggleLiveLink);
        a.frame();
        assert_eq!(a.state.link.status, LinkStatus::Listening);
        (a, name)
    }

    fn frame(&mut self) {
        if let Some(r) = self.state.link_request.take() {
            self.link.request(r, &self.ctx, &mut self.state);
        }
        self.link.poll(&mut self.state);
        self.link.publish(&mut self.state);
        self.state.link = self.link.view();
    }

    fn until(&mut self, what: &str, mut cond: impl FnMut(&yolu_app::state::AppState) -> bool) {
        let deadline = Instant::now() + WATCHDOG;
        loop {
            self.frame();
            if cond(&self.state) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{what} を待ったが来ない: 接続={:?}, メッセージ={}",
                self.state.link.status,
                self.state.message
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// 今のセットの選んだレイヤーに、文書の座標で線を引く。
    fn paint(&mut self, from: (f64, f64), to: (f64, f64)) {
        use yolu_app::engine::DVec2;
        let layer = self.state.selected_layer.unwrap();
        let brush = self.state.stroke_settings(false);
        let doc = &mut self.state.doc;
        let mut stroke = doc.begin_stroke(layer, &brush).unwrap();
        stroke
            .add_point(doc, from.0, from.1, 1.0, DVec2::ZERO)
            .unwrap();
        stroke.add_point(doc, to.0, to.1, 1.0, DVec2::ZERO).unwrap();
        doc.end_stroke(stroke).unwrap();
    }
}

#[test]
fn headless_the_unity_bridge_in_another_process_sees_the_painted_tiles() {
    let (mut a, name) = Headless::listen(256, "hproc");
    let (mut child, mut to_child, rx) = spawn_child_unity(&name);
    assert_eq!(
        next_child_line(&mut a, &rx, &mut child, "接続完了"),
        "connected"
    );
    let body = next_child_line(&mut a, &rx, &mut child, "Bodyの初回セット");
    let hair = next_child_line(&mut a, &rx, &mut child, "Hairの初回セット");
    let (body_uid, hair_uid) = (
        a.state.sets.get(0).unwrap().uid,
        a.state.sets.get(1).unwrap().uid,
    );
    assert_eq!(
        body,
        format!("set {body_uid} Body 256x256 material 0 tiles 4")
    );
    assert_eq!(
        hair,
        format!("set {hair_uid} Hair 512x512 material 1 tiles 16")
    );
    a.state.apply(Action::SelectSet(hair_uid));
    a.paint((250.0, 256.0), (262.0, 256.0));
    let expected = composite_pixel(&a.state.doc, 256, 256);
    assert_eq!(expected[3], 255);
    a.frame();
    writeln!(to_child, "painted").unwrap();
    let line = next_child_line(&mut a, &rx, &mut child, "Hairの描画タイル");
    let (tiles, center) = line
        .strip_prefix("hair tiles ")
        .and_then(|r| r.split_once(" center "))
        .unwrap_or_else(|| panic!("{line}"));
    assert!((1..=4).contains(&tiles.parse::<u32>().unwrap()), "{line}");
    assert_eq!(
        center,
        format!(
            "{} {} {} {}",
            expected[0], expected[1], expected[2], expected[3]
        )
    );
    assert_eq!(
        next_child_line(&mut a, &rx, &mut child, "ポーズ送信完了"),
        "posed"
    );
    a.until("ポーズ", |s| {
        s.view3d.model.as_ref().unwrap().meshes[0].positions[0]
            == yolu_core::glam::Vec3::new(0.0, 0.0, 1.0)
    });
    writeln!(to_child, "bye").unwrap();
    assert_eq!(next_child_line(&mut a, &rx, &mut child, "切断完了"), "done");
    a.until("切れた", |s| s.link.status == LinkStatus::Listening);
    assert!(child.finish().success());
    // やめると待ち受けも消える
    a.state.apply(Action::ToggleLiveLink);
    a.frame();
    assert_eq!(a.state.link.status, LinkStatus::Off);
    assert!(link::connect(&name).is_err());
}

#[test]
fn headless_opening_a_file_while_linked_swaps_the_published_sets() {
    let (mut a, name) = Headless::listen(256, "hopen");
    let mut unity = FakeUnity::connect(&name);
    a.until("つながった", |s| {
        matches!(s.link.status, LinkStatus::Connected { .. })
    });
    // スロット 0・1・2 のマテリアル（サブメッシュの順）
    unity.send(Message::Model(model(
        1,
        vec![
            material("Skin", 512, true),
            material("Cloth", 512, true),
            material("Other", 512, true),
        ],
    )));
    let got = unity.collect_until(&mut a, "3 つのセット", |m| {
        m.iter()
            .filter(|m| matches!(m, Message::TextureSet(_)))
            .count()
            == 3
    });
    let old: Vec<u32> = got
        .iter()
        .filter_map(|m| match m {
            Message::TextureSet(t) => Some(t.set),
            _ => None,
        })
        .collect();
    // 形式 6 のファイル（スロット 0・1・2 の鍵のセット 3 つ）を開く
    let dir = std::env::temp_dir().join(format!("ylapp-open-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    crate::common::tmp::clean_up_after_test(&dir);
    let path = dir.join("format6.ylp");
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../yolu-io/tests/fixtures/format6.ylp"),
        &path,
    )
    .unwrap();
    a.state.apply(Action::OpenProject(path));
    let got = unity.collect_until(&mut a, "差し替え", |m| {
        m.iter()
            .filter(|m| matches!(m, Message::TextureSet(_)))
            .count()
            == 3
    });
    let _ = std::fs::remove_dir_all(&dir);
    let removed: Vec<u32> = got
        .iter()
        .filter_map(|m| match m {
            Message::TextureSetRemoved { set } => Some(*set),
            _ => None,
        })
        .collect();
    assert_eq!(removed, old, "前のセットを外す");
    let names: Vec<&str> = a.state.sets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Skin", "Cloth", "Skin 2"]);
    for (i, set) in a.state.sets.iter().enumerate() {
        assert_eq!(set.bound, Some(i as u32), "スロットの番号の鍵で付く");
        assert!(
            !old.contains(&set.uid),
            "開き直したセットの番号は前と重ならない"
        );
        assert!(set.read_only.is_none(), "core が持つ中身だけなので描ける");
    }
    for m in &got {
        if let Message::TextureSet(t) = m {
            let index = a.state.sets.index_of(t.set).expect("開いたセット");
            assert_eq!((t.material, t.width), (index as u32, 512));
            // 開いたセットの絵（ファイルの正本から作った core の文書）を Unity に見せる
            let image = read_image(&t.channels[0].path, 512, 512);
            let doc = a.state.set_doc(index);
            for (x, y) in [(100, 100), (256, 300), (400, 50)] {
                assert_eq!(pixel(&image, 512, x, y), composite_pixel(doc, x, y));
            }
        }
    }
    unity.send(Message::Bye);
    a.until("切れた", |s| s.link.status == LinkStatus::Listening);
}

#[test]
fn strokes_in_the_3d_view_paint_the_set_and_go_back_to_unity() {
    let mut h = app(1280.0, 800.0, 256);
    let name = listen(&mut h, "3d");
    let mut unity = FakeUnity::connect(&name);
    unity.send(Message::Model(model(
        1,
        vec![material("Body", 256, true), material("Cloth", 256, true)],
    )));
    let got = unity.collect_until(&mut h, "2 つのセット", |m| {
        m.iter()
            .filter(|m| matches!(m, Message::TextureSet(_)))
            .count()
            == 2
    });
    let body = got
        .iter()
        .find_map(|m| match m {
            Message::TextureSet(t) if t.name == "Body" => Some(t.clone()),
            _ => None,
        })
        .unwrap();
    // 3D ビューは Live Link のモデルの形で、描くのは今のセット（Body = マテリアル 0）の面
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    look_at_the_quad_front(&mut h);
    let s = &h.state().state;
    assert_eq!(s.view3d.model.as_ref().unwrap().name, "試しの四角");
    assert_eq!(s.view3d.material, 0);
    let rect = h.state().view3d_rect().unwrap();
    let at = |p: yolu_core::glam::Vec3, h: &Harness<'_, YoluApp>| {
        let view = yolu_app::view3d::input::camera_view(&h.state().state, rect);
        let s = view.to_screen(p).expect("カメラの前");
        egui::pos2(rect.left() + s.x, rect.top() + s.y)
    };
    // Body の三角形（左下の半分）の中をなぞる
    let a = at(yolu_core::glam::Vec3::new(0.15, 0.2, 0.0), &h);
    let b = at(yolu_core::glam::Vec3::new(0.3, 0.2, 0.0), &h);
    drag(&mut h, &[a, b]);
    assert!(
        h.state().state.doc.can_undo(),
        "{}",
        h.state().state.message
    );
    let got = unity.collect_until(&mut h, "3D で描いたタイル", |m| {
        m.iter().any(|m| matches!(m, Message::TilesChanged(_)))
    });
    for m in &got {
        if let Message::TilesChanged(t) = m {
            assert_eq!(t.set, body.set, "今のセットのタイルだけが返る");
        }
    }
    let image = read_image(&body.channels[0].path, 256, 256);
    let doc = &h.state().state.doc;
    let painted = (0..256 * 256)
        .map(|i| (i % 256, i / 256))
        .filter(|&(x, y)| composite_pixel(doc, x, y)[3] > 0)
        .collect::<Vec<_>>();
    assert!(!painted.is_empty());
    for &(x, y) in painted.iter().step_by(37) {
        assert_eq!(pixel(&image, 256, x, y), composite_pixel(doc, x, y));
    }
    // Cloth の目を閉じると、3D から Cloth の面を除く（Unity からも外す）
    let cloth = h.state().state.sets.get(1).unwrap().uid;
    h.state_mut().state.apply(Action::ToggleSetVisible(cloth));
    h.run();
    let shown = h.state().state.view3d.model.clone().unwrap();
    assert_eq!(shown.triangle_count(), 1);
    assert!(shown.geometry.triangles().iter().all(|t| t.material == 0));
}

#[test]
fn headless_new_document_gets_material_sets_and_painted_document_is_preserved() {
    for painted in [false, true] {
        let (mut app, name) = Headless::listen(256, if painted { "keep" } else { "new" });
        assert!(app.state.is_pristine());
        if painted { app.paint((100.0, 128.0), (140.0, 128.0)); }
        let id = app.state.doc.id();
        let revision = app.state.doc.revision();
        let pixel = composite_pixel(&app.state.doc, 128, 128);
        let undo = app.state.doc.can_undo();
        let unity = FakeUnity::connect(&name);
        app.until("接続", |s| matches!(s.link.status, LinkStatus::Connected { .. }));
        for generation in [1, 2] {
            unity.send(Message::Model(model(generation, vec![
                material("First", 256, true), material("Second", 512, true),
            ])));
            app.until("モデル受信", |s| s.model.as_ref().is_some_and(|m| m.generation == generation));
            assert_eq!(app.state.sets.len(), 2);
            assert_eq!(app.state.sets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["First", "Second"]);
            assert_eq!(app.state.doc.id(), id);
            assert_eq!(app.state.doc.revision(), revision);
            assert_eq!(app.state.doc.can_undo(), undo);
            assert_eq!(composite_pixel(&app.state.doc, 128, 128), pixel);
            assert_eq!(app.state.set_doc(1).width(), 512);
        }
        unity.send(Message::Bye);
        app.until("切断", |s| s.link.status == LinkStatus::Listening);
    }
}

/// 窓の状態（隠れている・見えている）を持たせて、`App::logic` だけを 1 回呼ぶ（`ui` は回さない）。
fn run_logic(h: &mut Harness<'_, YoluApp>, hidden: bool) {
    let ctx = h.ctx.clone();
    let mut input = egui::RawInput {
        viewport_id: egui::ViewportId::ROOT,
        ..Default::default()
    };
    input.viewports.insert(
        egui::ViewportId::ROOT,
        egui::ViewportInfo {
            minimized: Some(hidden),
            occluded: Some(false),
            ..Default::default()
        },
    );
    let mut frame = eframe::Frame::_new_kittest();
    let app = h.state_mut();
    let _ = ctx.run_logic(&input, |ctx| eframe::App::logic(app, ctx, &mut frame));
}

/// 最小化・隠れた窓を模す。本物の eframe は、窓が隠れている間（Windows の最小化・macOS の覆われた窓）は egui のパスを回さず、`App::ui` を
/// 呼ばずに `Context::run_logic` で `App::logic` だけを呼ぶ。呼ぶのは**描き直しの頼みがあるときだけ**で（Live Link の裏のスレッドは知らせの
/// たびに頼む）、頼みが無ければ何も回さない。ここでも同じにして、頼みの立て忘れ（裏のスレッドの知らせ・待ちの描き直し）が試験に出るようにする。
/// `ui` は一度も呼ばない。
struct Hidden<'a, 'b>(&'a mut Harness<'b, YoluApp>);

impl Frames for Hidden<'_, '_> {
    fn next_frame(&mut self) {
        if self.0.ctx.has_requested_repaint() {
            run_logic(self.0, true);
        }
    }
}

/// 最小化の間も Live Link が動く: つながる・モデルを受ける・セットを出す・描いたタイルを返す・切れるが、画面のフレームを 1 つも回さずに通る
/// （Unity で Play に入る・スクリプトをリロードしたときの再接続が、最小化したままでも絵を出せる）。隠れた窓の `logic` は描き直しの頼みが
/// あるときだけ回る形で（`Hidden`）、裏のスレッドの知らせが頼みを立てることも通る。窓が見えている間は、受け取り待ちの知らせも出す物も
/// `logic` は扱わない（`ui` が扱う）。
#[test]
fn a_minimized_window_still_serves_the_live_link() {
    let mut h = app(1280.0, 800.0, 256);
    let name = listen(&mut h, "min");
    // ここから画面のフレーム（`ui`）は回さない
    let mut unity = FakeUnity::connect(&name);
    // 窓が見えている間は、積まれた知らせ（Unity のつなぎ）を `logic` は読まない
    for _ in 0..40 {
        run_logic(&mut h, false);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(h.state().state.link.status, LinkStatus::Listening, "見えている間は読まない");
    // 見えている間の `logic` が頼みを使い切ったので、窓が隠れたあとの次の頼み（積まれた知らせはそのまま残っている）で回る
    h.ctx.request_repaint();
    let deadline = Instant::now() + WATCHDOG;
    while !matches!(h.state().state.link.status, LinkStatus::Connected { .. }) {
        Hidden(&mut h).next_frame();
        assert!(Instant::now() < deadline, "最小化の間につながらない: {:?}", h.state().state.link.status);
        std::thread::sleep(Duration::from_millis(5));
    }
    unity.send(Message::Model(model(1, vec![material("Body", 512, true)])));
    let got = unity.collect_until(&mut Hidden(&mut h), "テクスチャセット", |m| {
        m.iter().any(|m| matches!(m, Message::TextureSet(_)))
    });
    let set = got
        .iter()
        .find_map(|m| match m {
            Message::TextureSet(t) => Some(t.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!((set.material, set.width), (0, 256));
    assert!(h.state().state.model.is_some(), "モデルは状態に入る");
    assert_eq!(h.state().state.link.published.len(), 1);
    // 描くと、変わったタイルが返る（描くのは状態を直に。最小化の間はペンも届かない。本物は描く操作が描き直しを頼む）
    {
        use yolu_app::engine::DVec2;
        let s = &mut h.state_mut().state;
        let layer = s.selected_layer.unwrap();
        let brush = s.stroke_settings(false);
        let doc = &mut s.doc;
        let mut stroke = doc.begin_stroke(layer, &brush).unwrap();
        stroke.add_point(doc, 100.0, 100.0, 1.0, DVec2::ZERO).unwrap();
        stroke.add_point(doc, 110.0, 100.0, 1.0, DVec2::ZERO).unwrap();
        doc.end_stroke(stroke).unwrap();
    }
    h.ctx.request_repaint();
    // 窓が見えている間は、出す物（描いたタイル）があっても `logic` は出さない
    for _ in 0..40 {
        run_logic(&mut h, false);
        std::thread::sleep(Duration::from_millis(5));
    }
    while let Ok(r) = unity.rx.try_recv() {
        assert!(!matches!(r, Ok(Received::Message(Message::TilesChanged(_)))), "見えている間は出さない: {r:?}");
    }
    h.ctx.request_repaint();
    let got = unity.collect_until(&mut Hidden(&mut h), "変わったタイル", |m| {
        m.iter().any(|m| matches!(m, Message::TilesChanged(_)))
    });
    assert!(got.iter().any(|m| matches!(m, Message::TilesChanged(t) if t.set == set.set)));
    // Unity が切ると、状態は待機に戻る
    unity.send(Message::Bye);
    while h.state().state.link.status != LinkStatus::Listening {
        Hidden(&mut h).next_frame();
        assert!(Instant::now() < deadline, "切れたのを受けない: {:?}", h.state().state.link.status);
        std::thread::sleep(Duration::from_millis(5));
    }
}
