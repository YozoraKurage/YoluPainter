//! Live Link の頼み（スタンドアロンの側、画面なし）: 元の絵を待たせたセットのマテリアルを、スタンドアロンから Unity に頼む。頼みを知らない古い
//! Unity には頼まず、元の絵は今までどおり Unity が送る（値が来ないまま頼めないときは、理由を 1 度だけ知らせる）。値が来ない・絵が揃わない
//! マテリアルも頼み、回数に上限がある。Unity が「印が同じ」と答えたら手元の絵を使い、合わなければ使わずに頼み直す。
//! ポーズは読むスレッドがメッシュごとの最新だけを溜め、止まった画面のスレッドが戻っても 1 つ分だけを、モデルより後の順で当てる。
use crate::common::wait;
use wait::WATCHDOG;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use yolu_app::livelink::{LinkStatus, LiveLink};
use yolu_app::state::{Action, AppState};
use yolu_protocol::link::connect_and_greet_as;
use yolu_protocol::*;

fn unique_name(tag: &str) -> String {
    crate::common::names::unique_name("ylreq", tag)
}

/// 画面を使わずに、AppState と LiveLink を画面のフレームと同じ順（頼み → 受ける → 出す）で回す。
struct Headless {
    ctx: egui::Context,
    state: AppState,
    link: LiveLink,
}

impl Headless {
    fn listen(tag: &str) -> (Headless, String) {
        let name = unique_name(tag);
        let mut a = Headless {
            ctx: egui::Context::default(),
            state: AppState::new(64, 64),
            link: LiveLink::new(),
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

    fn until(&mut self, what: &str, mut cond: impl FnMut(&Headless) -> bool) {
        let deadline = Instant::now() + WATCHDOG;
        loop {
            self.frame();
            if cond(self) {
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

    /// 何も起きないことを見るために、フレームを少し進める。
    fn settle(&mut self) {
        for _ in 0..40 {
            self.frame();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn connect(&mut self, name: &str, features: u64) -> FakeUnity {
        let unity = FakeUnity::connect(name, features);
        self.until("つながり", |a| {
            matches!(a.state.link.status, LinkStatus::Connected { .. })
        });
        unity
    }

    fn layer_names(&self, index: usize) -> Vec<String> {
        self.state
            .set_doc(index)
            .layers()
            .iter()
            .map(|l| l.name().to_owned())
            .collect()
    }

    fn set_index(&self, name: &str) -> usize {
        self.state
            .sets
            .iter()
            .position(|s| s.name == name)
            .unwrap_or_else(|| panic!("セット {name} が無い"))
    }
}

/// Unity の役。スタンドアロンから来たものを覚える。
struct FakeUnity {
    conn: Connection,
    got: Arc<Mutex<Vec<Message>>>,
}

impl FakeUnity {
    fn connect(name: &str, features: u64) -> FakeUnity {
        let identity = Identity::unity("試験の Unity")
            .with_version(Some(AppVersion::new(0, 4, 0)))
            .with_features(features);
        let (conn, mut reader, _) = connect_and_greet_as(name, &identity).unwrap();
        let got = Arc::new(Mutex::new(Vec::new()));
        let (reply, sink) = (conn.clone(), got.clone());
        std::thread::spawn(move || loop {
            match reader.next(&reply) {
                Ok(Received::Idle) => {}
                Ok(Received::Message(m)) => sink.lock().unwrap().push(m),
                Ok(_) => {}
                Err(_) => break,
            }
        });
        FakeUnity { conn, got }
    }

    fn send(&self, m: Message) {
        self.conn.send(&m).unwrap();
    }

    /// これまでに来た頼み（来た順の項目）。
    fn requests(&self) -> Vec<(u32, MaterialWant)> {
        self.got
            .lock()
            .unwrap()
            .iter()
            .filter_map(|m| match m {
                Message::MaterialRequest(r) => Some(r.items.iter().map(|i| (r.generation, i.clone())).collect::<Vec<_>>()),
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn request_messages(&self) -> usize {
        self.got
            .lock()
            .unwrap()
            .iter()
            .filter(|m| matches!(m, Message::MaterialRequest(_)))
            .count()
    }

    fn set_of(&self, material: u32) -> Option<TextureSet> {
        self.got.lock().unwrap().iter().rev().find_map(|m| match m {
            Message::TextureSet(t) if t.material == material => Some(t.clone()),
            _ => None,
        })
    }

    fn errors(&self) -> Vec<ErrorMessage> {
        self.got
            .lock()
            .unwrap()
            .iter()
            .filter_map(|m| match m {
                Message::Error(e) => Some(e.clone()),
                _ => None,
            })
            .collect()
    }
}

fn material(name: &str, texture: Option<u32>) -> MaterialInfo {
    MaterialInfo {
        key: MaterialKey::Material {
            name: name.into(),
            asset: None,
        },
        shader: "Standard".into(),
        textures: texture
            .map(|size| {
                vec![TextureProperty {
                    name: "_MainTex".into(),
                    width: size,
                    height: size,
                }]
            })
            .unwrap_or_default(),
        routes: vec![ChannelRoute {
            channel: channel::COLOR,
            property: "_MainTex".into(),
        }],
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
            skinned: false,
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]],
            normals: vec![],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
            submeshes: (0..n)
                .map(|m| Submesh {
                    material: m,
                    indices: if m % 2 == 0 { vec![0, 2, 1] } else { vec![1, 2, 3] },
                })
                .collect(),
        }],
    }
}

fn picture(size: u32) -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in 0..size {
        for x in 0..size {
            pixels.extend_from_slice(&[x as u8, y as u8, (x + y) as u8, 255]);
        }
    }
    pixels
}

fn original(generation: u32, material: u32, size: u32, stamp: u64) -> Message {
    Message::MaterialOriginal(MaterialOriginal {
        generation,
        material,
        slot: "_MainTex".into(),
        state: OriginalState::Image,
        read: OriginalRead::File,
        compressed: false,
        width: size,
        height: size,
        srgb: true,
        pixels: picture(size),
        stamp,
    })
}

fn cached(generation: u32, material: u32, size: u32, stamp: u64) -> Message {
    Message::MaterialOriginal(MaterialOriginal {
        generation,
        material,
        slot: "_MainTex".into(),
        state: OriginalState::Cached,
        read: OriginalRead::File,
        compressed: false,
        width: size,
        height: size,
        srgb: true,
        pixels: Vec::new(),
        stamp,
    })
}

const OLD: u64 = feature::MATERIAL_VALUES | feature::ORIGINAL_TEXTURES;
const NEW: u64 = OLD | feature::MATERIAL_REQUEST;

#[test]
fn headless_the_sets_that_wait_for_an_original_are_asked_for_once_and_the_answer_goes_under_the_layers() {
    let (mut a, name) = Headless::listen("ask");
    let unity = a.connect(&name, NEW);
    assert_ne!(a.state.link.common_features() & feature::MATERIAL_REQUEST, 0);
    unity.send(Message::Model(model(
        1,
        vec![material("Body", Some(256)), material("Hair", Some(128)), material("Plain", None)],
    )));
    a.until("頼み", |_| unity.request_messages() >= 1);
    a.settle();
    // 絵のあるマテリアルだけを、1 つの頼みで、印なしで（手元に何も無い）
    assert_eq!(
        unity.requests(),
        vec![
            (1, MaterialWant::original(0, "_MainTex", 0)),
            (1, MaterialWant::original(1, "_MainTex", 0)),
        ]
    );
    assert_eq!(unity.request_messages(), 1);
    assert_eq!(a.link.requests_sent(), 1);
    assert_eq!(a.link.originals_waiting(), 2);
    // Unity は頼まれた分を（印つきで）答える。入るまで Unity に出さなかったセットが出る
    unity.send(original(1, 0, 256, 0x51));
    unity.send(original(1, 1, 128, 0x52));
    a.until("セット", |_| unity.set_of(0).is_some() && unity.set_of(1).is_some());
    assert_eq!(a.link.originals_waiting(), 0);
    let (body, hair) = (a.set_index("Body"), a.set_index("Hair"));
    for set in [body, hair] {
        assert_eq!(a.layer_names(set), ["元の絵", "レイヤー 1"]);
    }
    // 手元に残した（次の頼みの have）
    assert_eq!(a.link.original_cache().len(), 2);
    assert!(unity.errors().is_empty(), "{:?}", unity.errors());
    // 描いたセットのあるモデルの送り直しでは、新しいセットだけを頼む
    unity.send(Message::Model(model(
        2,
        vec![material("Body", Some(256)), material("Hair", Some(128)), material("Extra", Some(64))],
    )));
    a.until("2 度目の頼み", |_| unity.request_messages() >= 2);
    a.settle();
    assert_eq!(unity.request_messages(), 2, "Extra のセットだけ");
    assert_eq!(
        unity.requests()[2..],
        [(2, MaterialWant::original(2, "_MainTex", 0))],
        "Body・Hair のセットは元の絵を持っている。頼まない"
    );
}

#[test]
fn headless_a_unity_without_the_mark_is_not_asked_and_pushes_the_originals_as_before() {
    let (mut a, name) = Headless::listen("old");
    let unity = a.connect(&name, OLD);
    assert_eq!(a.state.link.common_features() & feature::MATERIAL_REQUEST, 0);
    unity.send(Message::Model(model(1, vec![material("Body", Some(256)), material("Hair", Some(128))])));
    a.until("元の絵を待つ", |a| a.link.originals_waiting() == 2);
    a.settle();
    assert_eq!(unity.request_messages(), 0, "印の無い Unity には頼まない");
    assert_eq!(a.link.requests_sent(), 0);
    // 元の絵は Unity が押し出す（今までどおり）。印は付いていない（0.3.x）
    unity.send(original(1, 0, 256, 0));
    unity.send(original(1, 1, 128, 0));
    a.until("セット", |_| unity.set_of(0).is_some() && unity.set_of(1).is_some());
    assert_eq!(a.link.original_cache().len(), 0, "印の無い絵は手元に残さない");
    assert!(unity.errors().is_empty());
}

#[test]
fn headless_a_material_whose_values_never_arrive_is_asked_for_and_a_unity_that_cannot_be_asked_is_told_why() {
    // 頼める Unity: 値が来ないマテリアルを頼む（絵の無い・マテリアルの無い組は頼まない）。答え（値なし）が来たら、もう頼まない
    let (mut a, name) = Headless::listen("values");
    a.link.set_ask_timing(Duration::ZERO, Duration::from_millis(50));
    let unity = a.connect(&name, NEW);
    let unassigned = MaterialInfo {
        key: MaterialKey::Unassigned,
        shader: String::new(),
        textures: vec![],
        routes: vec![],
    };
    unity.send(Message::Model(model(
        1,
        vec![material("Body", None), material("Hair", None), unassigned],
    )));
    a.until("値の頼み", |_| {
        unity
            .requests()
            .iter()
            .any(|(_, w)| w.wants_values() && w.material == 0)
    });
    a.until("値の頼み", |_| {
        unity.requests().iter().any(|(_, w)| w.wants_values() && w.material == 1)
    });
    assert!(
        unity.requests().iter().all(|(g, w)| *g == 1 && w.material < 2 && w.wants == WANT_VALUES),
        "マテリアルの無い組は頼まない: {:?}",
        unity.requests()
    );
    // マテリアル 0 に「値なし」が来たら、マテリアル 0 はもう頼まない。マテリアル 1 は答えが無いので、回数の上限まで頼み直す
    let none = |material| {
        Message::MaterialValues(MaterialValues {
            generation: 1,
            material,
            kind: ValuesKind::None,
            shader: String::new(),
            source: String::new(),
            properties: vec![],
            keywords: vec![],
            slots: vec![],
        })
    };
    unity.send(none(0));
    a.until("頼み直し", |_| {
        unity.requests().iter().filter(|(_, w)| w.material == 1).count() >= 3
    });
    a.settle();
    std::thread::sleep(Duration::from_millis(200));
    a.settle();
    let count = |m: u32| unity.requests().iter().filter(|(_, w)| w.material == m).count();
    assert_eq!(count(1), 3, "回数の上限（3 回）で止まる");
    assert!(count(0) <= 1, "値が来たマテリアルは頼み直さない: {}", count(0));

    // 頼めない Unity（印なし）: 頼まず、値が来ないままなら理由（Unity のパッケージの版）を 1 度だけ知らせる
    let (mut a, name) = Headless::listen("values-old");
    a.link.set_ask_timing(Duration::ZERO, Duration::from_millis(50));
    let unity = a.connect(&name, OLD);
    unity.send(Message::Model(model(1, vec![material("Body", None)])));
    a.until("理由の知らせ", |a| {
        a.state.link.notice.as_ref().is_some_and(|(_, t)| t.contains("取り直せません"))
    });
    let text = a.state.link.notice.clone().unwrap().1;
    assert!(text.contains("Unity のパッケージを 0.4.0 以上に上げる必要があります"), "{text}");
    a.settle();
    assert_eq!(unity.request_messages(), 0);
    assert_eq!(a.link.requests_sent(), 0);
    // 1 度だけ: 知らせを別の知らせで置き換えたあと、同じ理由で知らせ直さない
    a.state.message.clear();
    a.settle();
    assert!(!a.state.message.contains("取り直せません"));
}

#[test]
fn headless_a_cached_answer_uses_the_original_in_hand_and_a_changed_stamp_is_asked_for_again() {
    let (mut a, name) = Headless::listen("cached");
    let unity = a.connect(&name, NEW);
    unity.send(Message::Model(model(1, vec![material("Body", Some(64)), material("Hair", Some(64))])));
    a.until("最初の頼み", |_| unity.request_messages() >= 1);
    unity.send(original(1, 0, 64, 0xAA));
    unity.send(original(1, 1, 64, 0xBB));
    a.until("セット", |_| unity.set_of(0).is_some() && unity.set_of(1).is_some());
    assert_eq!(a.link.original_cache().len(), 2);
    // 利用者が Hair のセットを消す。Unity がモデルを送り直す（世代 2）と、Hair のセットが新しく作られ、元の絵をまた頼む。
    // 手元に Hair の絵があるので、頼みに印が付く
    let hair_uid = a.state.sets.get(a.set_index("Hair")).unwrap().uid;
    a.state.remove_sets(&[hair_uid]).unwrap();
    a.frame();
    let before = unity.request_messages();
    unity.send(Message::Model(model(2, vec![material("Body", Some(64)), material("Hair", Some(64))])));
    a.until("2 度目の頼み", |_| unity.request_messages() > before);
    assert_eq!(
        unity.requests().last().unwrap(),
        &(2, MaterialWant::original(1, "_MainTex", 0xBB)),
        "手元の絵の印が付く"
    );
    // Unity の答え: 印が同じ（画素なし）。手元の絵が入る
    unity.send(cached(2, 1, 64, 0xBB));
    a.until("手元の絵のセット", |_| {
        unity.set_of(1).is_some_and(|t| t.generation == 2)
    });
    let hair = a.set_index("Hair");
    assert_eq!(a.layer_names(hair), ["元の絵", "レイヤー 1"]);
    let doc = a.state.set_doc(hair);
    assert_eq!(
        yolu_app::engine::layer_pixel(&doc.layers()[0], 255, 255),
        [63, 63, 126, 255],
        "手元の絵の画素（64 の絵を 256 のセットへ拡大した端）"
    );
    assert!(unity.errors().is_empty(), "{:?}", unity.errors());

    // もう一度消して送り直す: 今度は Unity の印が変わった（絵が差し替わった）。手元の絵は使わず、印なしで頼み直し、新しい絵で入る
    let hair_uid = a.state.sets.get(a.set_index("Hair")).unwrap().uid;
    a.state.remove_sets(&[hair_uid]).unwrap();
    a.frame();
    let before = unity.request_messages();
    unity.send(Message::Model(model(3, vec![material("Body", Some(64)), material("Hair", Some(64))])));
    a.until("3 度目の頼み", |_| unity.request_messages() > before);
    assert_eq!(unity.requests().last().unwrap().1.have, 0xBB);
    unity.send(cached(3, 1, 64, 0xCC));
    let again = unity.request_messages();
    a.until("頼み直し", |_| unity.request_messages() > again);
    assert_eq!(
        unity.requests().last().unwrap(),
        &(3, MaterialWant::original(1, "_MainTex", 0)),
        "古い絵は使わず、印なしで頼み直す"
    );
    assert!(!unity.set_of(1).is_some_and(|t| t.generation == 3), "古い絵では出さない");
    let mut fresh = picture(64);
    fresh[0] = 200;
    unity.send(Message::MaterialOriginal(MaterialOriginal {
        generation: 3,
        material: 1,
        slot: "_MainTex".into(),
        state: OriginalState::Image,
        read: OriginalRead::File,
        compressed: false,
        width: 64,
        height: 64,
        srgb: true,
        pixels: fresh,
        stamp: 0xCC,
    }));
    a.until("新しい絵のセット", |_| unity.set_of(1).is_some_and(|t| t.generation == 3));
    let hair = a.set_index("Hair");
    let doc = a.state.set_doc(hair);
    assert_eq!(yolu_app::engine::layer_pixel(&doc.layers()[0], 0, 0)[0], 200);
    assert_eq!(a.link.original_cache().len(), 2);
}

#[test]
fn headless_a_pile_of_poses_while_the_screen_is_stopped_applies_as_the_newest_one_after_the_model() {
    let (mut a, name) = Headless::listen("pose");
    let unity = a.connect(&name, OLD);
    unity.send(Message::Model(model(1, vec![material("Body", None)])));
    a.until("モデル", |a| a.state.view3d.model.is_some());
    let pos = |a: &Headless| a.state.view3d.model.as_ref().unwrap().meshes[0].positions[3];
    // 画面のスレッドが止まっている（フレームを回さない）あいだに、ポーズが大量に届く。読むスレッドが最新だけを溜める
    for i in 1..=300u32 {
        unity.send(Message::Pose(Pose {
            generation: 1,
            meshes: vec![MeshPose {
                mesh: 0,
                positions: vec![[i as f32; 3]; 4],
                normals: vec![],
            }],
        }));
    }
    // 受け切るまで待つ（読むスレッドが溜め場へ入れる。フレームは回さない）
    let deadline = Instant::now() + WATCHDOG;
    loop {
        if a.link.pending_pose_bytes() > 0 {
            std::thread::sleep(Duration::from_millis(100));
            break;
        }
        assert!(Instant::now() < deadline, "ポーズが溜め場に届かない");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        a.link.pending_pose_bytes(),
        4 * 12,
        "300 回受けても、溜め場には 1 つのメッシュの 1 回分（位置 4 頂点）だけ"
    );
    // 画面が戻る: 最新の 1 つだけが当たる
    a.until("最新のポーズ", |a| pos(a) == yolu_core::glam::Vec3::splat(300.0));
    assert_eq!(a.link.pending_pose_bytes(), 0);

    // 順番: モデル（世代 2）のすぐ後ろのポーズは、新しいモデルに当たる（前のモデルに当てて断られない）
    unity.send(Message::Model(model(2, vec![material("Body", None)])));
    unity.send(Message::Pose(Pose {
        generation: 2,
        meshes: vec![MeshPose {
            mesh: 0,
            positions: vec![[7.0; 3]; 4],
            normals: vec![],
        }],
    }));
    a.until("世代 2 のポーズ", |a| pos(a) == yolu_core::glam::Vec3::splat(7.0));
    assert!(unity.errors().is_empty(), "断られていない: {:?}", unity.errors());
    // 前の世代のポーズが溜まったまま次のモデルが来ても、前のポーズは前のモデルに先に当たり、新しいポーズは新しいモデルに当たる
    unity.send(Message::Pose(Pose {
        generation: 2,
        meshes: vec![MeshPose {
            mesh: 0,
            positions: vec![[8.0; 3]; 4],
            normals: vec![],
        }],
    }));
    unity.send(Message::Model(model(3, vec![material("Body", None)])));
    unity.send(Message::Pose(Pose {
        generation: 3,
        meshes: vec![MeshPose {
            mesh: 0,
            positions: vec![[9.0; 3]; 4],
            normals: vec![],
        }],
    }));
    a.until("世代 3 のポーズ", |a| pos(a) == yolu_core::glam::Vec3::splat(9.0));
    assert!(unity.errors().is_empty(), "断られていない: {:?}", unity.errors());

    // 順番: ポーズのすぐ後ろの「モデルを閉じた」。ポーズは閉じる前に当たる（閉じたあとのモデルへ当てて、断る返事を返さない）。
    // 画面のスレッドが止まっている間に、読むスレッドが「閉じた」を列へ積む前にポーズを先に流す
    unity.send(Message::Pose(Pose {
        generation: 3,
        meshes: vec![MeshPose {
            mesh: 0,
            positions: vec![[11.0; 3]; 4],
            normals: vec![],
        }],
    }));
    unity.send(Message::ModelClosed { generation: 3 });
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(a.link.pending_pose_bytes(), 0, "「閉じた」の前に、溜めたポーズは列へ流れた");
    a.until("モデルが閉じる", |a| a.state.model.is_none());
    a.settle();
    assert!(unity.errors().is_empty(), "閉じたあとのモデルに当てて断られた: {:?}", unity.errors());
}

#[test]
fn headless_poses_around_new_models_that_pile_up_while_the_screen_is_stopped_apply_in_order() {
    let (mut a, name) = Headless::listen("poseorder");
    let unity = a.connect(&name, OLD);
    unity.send(Message::Model(model(1, vec![material("Body", None)])));
    a.until("モデル", |a| a.state.view3d.model.is_some());
    let pos = |a: &Headless| a.state.view3d.model.as_ref().unwrap().meshes[0].positions[3];
    let send_pose = |generation: u32, x: f32| {
        unity.send(Message::Pose(Pose {
            generation,
            meshes: vec![MeshPose {
                mesh: 0,
                positions: vec![[x; 3]; 4],
                normals: vec![],
            }],
        }));
    };
    // 画面のスレッドが止まっている（フレームを回さない）あいだに、ポーズ・モデル・ポーズ・モデル・ポーズが続けて届く。
    // どのポーズも、自分の前にあったモデルに当たり（先の世代のモデルに当てて断られない）、最後は世代 3 の最新のポーズになる
    send_pose(1, 2.0);
    send_pose(1, 3.0);
    unity.send(Message::Model(model(2, vec![material("Body", None)])));
    send_pose(2, 4.0);
    unity.send(Message::Model(model(3, vec![material("Body", None)])));
    send_pose(3, 5.0);
    send_pose(3, 6.0);
    // 読むスレッドが全部を受け切るまで待つ（フレームは回さない）
    let deadline = Instant::now() + WATCHDOG;
    while a.link.pending_pose_bytes() != 4 * 12 {
        assert!(Instant::now() < deadline, "世代 3 のポーズが溜め場に届かない");
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(100));
    a.until("世代 3 の最新のポーズ", |a| pos(a) == yolu_core::glam::Vec3::splat(6.0));
    assert_eq!(a.link.pending_pose_bytes(), 0);
    a.settle();
    assert!(unity.errors().is_empty(), "どのポーズも断られていない: {:?}", unity.errors());
}
