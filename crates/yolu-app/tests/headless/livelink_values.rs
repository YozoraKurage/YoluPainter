//! Live Link のマテリアルの値（スタンドアロンの側、画面なし）: Unity の役（yolu-protocol で直につなぐ試験のスレッド）が lilToon の値と
//! 描いていないスロットの絵を送ると、そのマテリアルのテクスチャセットが lilToon の見た目になる（Undo の段は増えない）。欄で変えた項目
//! だけが Unity の値に勝ち、切っても最後の値が残る。値を送らない古い Unity からは何も来ない。保存するかは設定で選べる。
use crate::common::wait;
use wait::WATCHDOG;

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use yolu_app::livelink::{LinkStatus, LiveLink};
use yolu_app::look::{LookOp, Section};
use yolu_app::state::{Action, AppState};
use yolu_core::look::{
    LookKind, LookValue, MissingImage, ReceivedImage, ReceivedLook, TextureSource,
};
use yolu_core::Channel;
use yolu_protocol::link::connect_and_greet_as;
use yolu_protocol::*;

fn unique_name(tag: &str) -> String {
    crate::common::names::unique_name("ylvalues", tag)
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

    fn until(&mut self, what: &str, mut cond: impl FnMut(&AppState) -> bool) {
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
}

/// Unity の役。
struct FakeUnity {
    conn: Connection,
    _rx: mpsc::Receiver<()>,
}

impl FakeUnity {
    fn connect(name: &str, features: u64) -> FakeUnity {
        let identity = Identity::unity("試験の Unity")
            .with_version(Some(AppVersion::new(0, 3, 0)))
            .with_features(features);
        let (conn, mut reader, _) = connect_and_greet_as(name, &identity).unwrap();
        // スタンドアロンからの命令（セットの知らせなど）は読み捨てる
        let (tx, rx) = mpsc::channel();
        let reply = conn.clone();
        std::thread::spawn(move || loop {
            match reader.next(&reply) {
                Ok(Received::Idle) | Ok(_) => {}
                Err(_) => {
                    let _ = tx.send(());
                    break;
                }
            }
        });
        FakeUnity { conn, _rx: rx }
    }

    fn send(&self, m: Message) {
        self.conn.send(&m).unwrap();
    }
}

fn lil_material() -> MaterialInfo {
    MaterialInfo {
        key: MaterialKey::Material {
            name: "Body".into(),
            asset: Some(("0123456789abcdef0123456789abcdef".into(), 2)),
        },
        shader: "Hidden/lilToonOutline".into(),
        textures: vec![TextureProperty {
            name: "_MainTex".into(),
            width: 64,
            height: 64,
        }],
        routes: vec![ChannelRoute {
            channel: channel::COLOR,
            property: "_MainTex".into(),
        }],
    }
}

fn model(generation: u32) -> Model {
    Model {
        generation,
        name: "試しの四角".into(),
        materials: vec![lil_material()],
        meshes: vec![MeshData {
            key: "0".into(),
            name: "Quad".into(),
            skinned: false,
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]],
            normals: vec![],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
            submeshes: vec![Submesh {
                material: 0,
                indices: vec![0, 2, 1, 1, 2, 3],
            }],
        }],
    }
}

fn values(generation: u32, border: f32, shadow: [f32; 4]) -> MaterialValues {
    MaterialValues {
        generation,
        material: 0,
        kind: ValuesKind::LilToon,
        shader: "Hidden/lilToonOutline".into(),
        source: "lilToon 2.3.4 · Standard/Opaque+Outline".into(),
        properties: vec![
            PropertyEntry {
                name: "_UseShadow".into(),
                value: PropertyValue::Int(1),
            },
            PropertyEntry {
                name: "_ShadowBorder".into(),
                value: PropertyValue::Float(border),
            },
            PropertyEntry {
                name: "_ShadowColor".into(),
                value: PropertyValue::Color(shadow),
            },
            PropertyEntry {
                name: "_MainTex_ST".into(),
                value: PropertyValue::Vector([1.0, 1.0, 0.0, 0.0]),
            },
        ],
        keywords: vec![],
        slots: vec![
            SlotTexture {
                name: "_MatCapTex".into(),
                state: SlotState::Follows,
                width: 2,
                height: 2,
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

fn matcap(generation: u32) -> MaterialTexture {
    MaterialTexture {
        generation,
        material: 0,
        slot: "_MatCapTex".into(),
        width: 2,
        height: 2,
        srgb: true,
        pixels: [[255, 0, 0, 255]; 4].concat(),
    }
}

fn received(a: &AppState) -> Option<&ReceivedLook> {
    a.doc.received_look()
}

#[test]
fn headless_unity_values_draw_the_set_as_liltoon_without_an_undo_step() {
    let (mut a, name) = Headless::listen("draw");
    let unity = FakeUnity::connect(&name, feature::MATERIAL_VALUES);
    a.until("つながり", |s| {
        matches!(s.link.status, LinkStatus::Connected { .. })
    });
    assert_ne!(a.state.link.common_features() & feature::MATERIAL_VALUES, 0);
    unity.send(Message::Model(model(1)));
    a.until("セットの結び付け", |s| {
        s.sets.current().bound == Some(0)
    });
    let steps = a.doc_steps();
    unity.send(Message::MaterialValues(values(
        1,
        0.3,
        [0.4, 0.3, 0.5, 1.0],
    )));
    unity.send(Message::MaterialTexture(matcap(1)));
    a.until("値と絵", |s| {
        received(s).is_some_and(|r| r.images.contains_key("_MatCapTex"))
    });
    // 描く見た目は Unity の値（lilToon）。利用者の設定は既定のまま、Undo の段も増えない・変更の印も付かない
    let drawn = a.state.doc.drawn_look().clone();
    assert_eq!(drawn.kind, LookKind::LilToon);
    assert_eq!(drawn.shader, "Hidden/lilToonOutline");
    assert_eq!(drawn.float("_ShadowBorder", 0.5), 0.3);
    assert_eq!(
        drawn.textures["_MainTex"],
        TextureSource::Channel(Channel::Color)
    );
    // 利用者の設定は、新しいセットの既定（lilToon）のまま変わらない
    assert_eq!(a.state.doc.look(), &yolu_app::look::new_set_look());
    assert_eq!(a.doc_steps(), steps, "受け取りは Undo に入らない");
    assert!(!a.state.modified, "受け取りは編集ではない");
    let r = received(&a.state).unwrap();
    assert_eq!(r.source, "lilToon 2.3.4 · Standard/Opaque+Outline");
    assert_eq!(r.missing["_ShadowColorTex"], MissingImage::OverBudget);
    assert_eq!(r.images["_MatCapTex"].pixels[..4], [255, 0, 0, 255]);

    // 欄で 1 項目だけ変える: その項目だけが勝つ（1 回の Undo）
    a.state.apply(Action::Look(LookOp::Value {
        name: "_ShadowBorder",
        value: LookValue::Float(0.7),
        drag: false,
    }));
    assert_eq!(a.doc_steps(), steps + 1);
    assert_eq!(a.state.doc.drawn_look().float("_ShadowBorder", 0.5), 0.7);
    // Unity で値が変わる: 欄で変えた項目は残り、ほかは新しい値（絵は来る途中のあいだ前の絵）
    unity.send(Message::MaterialValues(values(
        1,
        0.1,
        [0.2, 0.2, 0.2, 1.0],
    )));
    a.until("新しい値", |s| {
        s.doc.drawn_look().vec4("_ShadowColor", [0.0; 4]) == [0.2, 0.2, 0.2, 1.0]
    });
    assert_eq!(a.state.doc.drawn_look().float("_ShadowBorder", 0.5), 0.7);
    assert!(received(&a.state)
        .unwrap()
        .images
        .contains_key("_MatCapTex"));
    // 節を既定に戻すと、その節は Unity の値
    a.state.apply(Action::Look(LookOp::Reset(Section::Shadow)));
    assert_eq!(a.state.doc.drawn_look().float("_ShadowBorder", 0.5), 0.1);
    // 標準を選ぶと受けた値があっても標準で描き、Unity に合わせると戻る
    a.state
        .apply(Action::Look(LookOp::Kind(LookKind::Standard)));
    assert_eq!(a.state.doc.drawn_look().kind, LookKind::Standard);
    a.state.apply(Action::Look(LookOp::FollowReceived));
    assert_eq!(a.state.doc.drawn_look().kind, LookKind::LilToon);
    // Undo は利用者の操作だけを戻し、受けた値はそのまま
    while a.state.doc.undo_count() > steps {
        a.state.apply(Action::Undo);
    }
    assert_eq!(a.state.doc.drawn_look().float("_ShadowBorder", 0.5), 0.1);
    assert!(received(&a.state).is_some());

    // 切っても最後の値は残る
    unity.send(Message::Bye);
    a.until("切れた", |s| s.link.status == LinkStatus::Listening);
    assert_eq!(a.state.doc.drawn_look().float("_ShadowBorder", 0.5), 0.1);
    assert_eq!(a.state.doc.drawn_look().kind, LookKind::LilToon);
}

#[test]
fn headless_a_material_that_is_no_longer_liltoon_drops_the_received_values() {
    let (mut a, name) = Headless::listen("none");
    let unity = FakeUnity::connect(&name, feature::MATERIAL_VALUES);
    a.until("つながり", |s| {
        matches!(s.link.status, LinkStatus::Connected { .. })
    });
    unity.send(Message::Model(model(1)));
    a.until("セットの結び付け", |s| {
        s.sets.current().bound == Some(0)
    });
    unity.send(Message::MaterialValues(values(
        1,
        0.3,
        [0.4, 0.3, 0.5, 1.0],
    )));
    a.until("値", |s| received(s).is_some());
    let mut none = values(1, 0.3, [0.0; 4]);
    none.kind = ValuesKind::None;
    unity.send(Message::MaterialValues(none));
    a.until("値を外す", |s| received(s).is_none());
    // 受けた値を外すと、利用者の設定（新しいセットの既定の lilToon）で描く
    assert_eq!(a.state.doc.drawn_look(), &yolu_app::look::new_set_look());
    // 古い世代の値は使わない（新しいモデルを受けたあとに、前のモデルの値が遅れて着いた）
    unity.send(Message::Model(model(2)));
    a.until("新しいモデル", |s| {
        s.model.as_ref().is_some_and(|m| m.generation == 2)
    });
    let shown = a.state.message.clone();
    unity.send(Message::MaterialValues(values(
        1,
        0.3,
        [0.4, 0.3, 0.5, 1.0],
    )));
    // 前の世代の絵・モデルに無いマテリアルの値も断るが、Unity への返事だけで画面の知らせには出さない（開発の診断の文と世代の番号）
    unity.send(Message::MaterialTexture(matcap(1)));
    let mut outside = values(2, 0.3, [0.4, 0.3, 0.5, 1.0]);
    outside.material = 1;
    unity.send(Message::MaterialValues(outside));
    unity.send(Message::MaterialValues(values(
        2,
        0.6,
        [0.4, 0.3, 0.5, 1.0],
    )));
    a.until("新しい世代の値", |s| received(s).is_some());
    assert_eq!(a.state.doc.drawn_look().float("_ShadowBorder", 0.5), 0.6);
    assert_eq!(a.state.message, shown, "断った命令は画面に出さない");
}

#[test]
fn headless_values_that_arrive_together_with_the_bye_are_kept() {
    let (mut a, name) = Headless::listen("bye");
    let unity = FakeUnity::connect(&name, feature::MATERIAL_VALUES);
    a.until("つながり", |s| {
        matches!(s.link.status, LinkStatus::Connected { .. })
    });
    unity.send(Message::Model(model(1)));
    a.until("セットの結び付け", |s| {
        s.sets.current().bound == Some(0)
    });
    // 値・絵・切断を続けて送り、着くのを待ってから 1 回のフレームで読む（同じ読みの中で、値のあとに切れる）
    unity.send(Message::MaterialValues(values(
        1,
        0.3,
        [0.4, 0.3, 0.5, 1.0],
    )));
    unity.send(Message::MaterialTexture(matcap(1)));
    unity.send(Message::Bye);
    std::thread::sleep(Duration::from_millis(300));
    a.until("切れた", |s| s.link.status == LinkStatus::Listening);
    let r = received(&a.state).expect("切れる前に着いた値は残る");
    assert!(r.images.contains_key("_MatCapTex"));
    assert_eq!(a.state.doc.drawn_look().float("_ShadowBorder", 0.5), 0.3);
}

#[test]
fn headless_main_2nd_is_not_drawn_over_everything_when_unity_does_not_list_its_texture() {
    use yolu_app::view3d::look_gpu::{feature as bit, features_of};
    let (mut a, name) = Headless::listen("later");
    let unity = FakeUnity::connect(&name, feature::MATERIAL_VALUES);
    a.until("つながり", |s| {
        matches!(s.link.status, LinkStatus::Connected { .. })
    });
    unity.send(Message::Model(model(1)));
    a.until("セットの結び付け", |s| {
        s.sets.current().bound == Some(0)
    });
    // 送るスロットの一覧が古い Unity: メインカラー 2nd は入（色は既定の白・A=1）だが、そのテクスチャのスロットを知らせない
    let mut old = values(1, 0.3, [0.4, 0.3, 0.5, 1.0]);
    old.properties.push(PropertyEntry {
        name: "_UseMain2ndTex".into(),
        value: PropertyValue::Int(1),
    });
    unity.send(Message::MaterialValues(old.clone()));
    a.until("値", |s| received(s).is_some());
    // 白のテクスチャのまま全面に重ねない: 描く機能に入らない。欄のスロットの行は「読めない」
    let drawn = a.state.doc.drawn_look().clone();
    assert_eq!(drawn.kind, LookKind::LilToon);
    assert_eq!(features_of(&drawn) & (1 << bit::MAIN2), 0);
    assert_eq!(features_of(&drawn) & (1 << bit::SHADOW), 1 << bit::SHADOW, "ほかの機能は描く");
    assert_eq!(received(&a.state).unwrap().missing["_Main2ndTex"], MissingImage::Unreadable);
    // スロットを知らせる Unity（テクスチャの有り無しを言う）なら、Unity と同じく描く
    let mut new = old;
    new.slots.push(SlotTexture {
        name: "_Main2ndTex".into(),
        state: SlotState::Follows,
        width: 2,
        height: 2,
    });
    new.slots.push(SlotTexture {
        name: "_Main2ndBlendMask".into(),
        state: SlotState::Empty,
        width: 0,
        height: 0,
    });
    unity.send(Message::MaterialValues(new));
    unity.send(Message::MaterialTexture(MaterialTexture {
        generation: 1,
        material: 0,
        slot: "_Main2ndTex".into(),
        width: 2,
        height: 2,
        srgb: true,
        pixels: [[0, 0, 0, 0]; 4].concat(),
    }));
    a.until("2nd の絵", |s| {
        received(s).is_some_and(|r| r.images.contains_key("_Main2ndTex"))
    });
    let drawn = a.state.doc.drawn_look().clone();
    assert_ne!(features_of(&drawn) & (1 << bit::MAIN2), 0);
    assert!(!received(&a.state).unwrap().missing.contains_key("_Main2ndTex"));
}

#[test]
fn headless_an_older_unity_without_the_mark_gets_and_sends_no_values() {
    let (mut a, name) = Headless::listen("old");
    // 値の印を出さない Unity（この機能より古いパッケージ）: 共通の印が無く、値の命令は送られない
    let unity = FakeUnity::connect(&name, 0);
    a.until("つながり", |s| {
        matches!(s.link.status, LinkStatus::Connected { .. })
    });
    assert_eq!(a.state.link.common_features(), 0);
    unity.send(Message::Model(model(1)));
    a.until("セットの結び付け", |s| {
        s.sets.current().bound == Some(0)
    });
    assert!(!unity
        .conn
        .send_gated(&Message::MaterialValues(values(
            1,
            0.3,
            [0.4, 0.3, 0.5, 1.0]
        )))
        .unwrap());
    for _ in 0..20 {
        a.frame();
    }
    assert!(received(&a.state).is_none());
    assert_eq!(
        a.state.doc.drawn_look(),
        &yolu_app::look::new_set_look(),
        "今までどおり（値が来ないだけ。見た目は新しいセットの既定）"
    );
}

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let p = std::env::temp_dir().join(unique_name(name));
        std::fs::create_dir_all(&p).unwrap();
        Dir(p)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sample_received() -> ReceivedLook {
    let mut r = yolu_app::look::link::received_look(
        &values(1, 0.25, [0.4, 0.3, 0.5, 1.0]),
        &lil_material().routes,
        &Default::default(),
        &Default::default(),
    );
    r.images.insert(
        "_MatCapTex".into(),
        std::sync::Arc::new(ReceivedImage {
            width: 1,
            height: 1,
            srgb: true,
            pixels: vec![255, 0, 0, 255].into(),
        }),
    );
    r.missing.remove("_MatCapTex");
    r
}

#[test]
fn the_received_values_are_saved_only_when_the_setting_says_so() {
    let dir = Dir::new("save");
    let path = dir.0.join("values.ylp");
    let mut s = AppState::new(32, 32);
    s.doc.set_received_look(Some(sample_received())).unwrap();
    // 欄で変えた項目もいっしょに保存する（受けた値とは別に持つ）
    s.apply(Action::Look(LookOp::Value {
        name: "_ShadowBorder",
        value: LookValue::Float(0.8),
        drag: false,
    }));
    assert!(s.prefs.settings.livelink_keep_values, "既定は保存する");
    yolu_app::project::save_from(&mut s, &path);
    let mut t = AppState::new(8, 8);
    yolu_app::project::open_into(&mut t, &path);
    let r = t.doc.received_look().expect("受けた値が戻る");
    assert_eq!(r.look, sample_received().look);
    assert!(r.images.is_empty(), "絵の画素は保存しない");
    assert_eq!(r.missing["_MatCapTex"], MissingImage::Pending);
    assert_eq!(
        t.doc.drawn_look().float("_ShadowBorder", 0.5),
        0.8,
        "欄で変えた項目が勝つまま"
    );
    assert_eq!(t.doc.drawn_look().kind, LookKind::LilToon);
    assert_eq!(t.doc.undo_count(), 0, "開いただけで Undo の段は増えない");
    // 保存しない設定: 次の保存で外す（欄で変えた項目は残る）
    t.prefs.settings.livelink_keep_values = false;
    yolu_app::project::save_from(&mut t, &path);
    let mut u = AppState::new(8, 8);
    yolu_app::project::open_into(&mut u, &path);
    assert!(u.doc.received_look().is_none());
    assert_eq!(u.doc.look().float("_ShadowBorder", 0.5), 0.8);
}

impl Headless {
    fn doc_steps(&self) -> usize {
        self.state.doc.undo_count()
    }
}

/// 本物のブリッジ（yolu-bridge の C の口。Unity の C# が呼ぶもの）を試験のスレッドで動かし、モデルと lilToon の値と絵を送る。
fn bridge_sends_values(name: String, done: mpsc::Receiver<()>) {
    use yolu_bridge::*;
    let deadline = Instant::now() + WATCHDOG;
    unsafe {
        let (agent, version) = ("試験のブリッジ", "0.3.0");
        let h = ylb_connect_with(
            name.as_ptr(),
            name.len() as i32,
            agent.as_ptr(),
            agent.len() as i32,
            version.as_ptr(),
            version.len() as i32,
        );
        assert_ne!(h, 0);
        while ylb_status(h) != 1 {
            assert!(
                Instant::now() < deadline && ylb_status(h) == 0,
                "つながらない"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_ne!(ylb_common_features(h) & feature::MATERIAL_VALUES, 0);
        let n = |s: &str| (s.as_ptr(), s.len() as i32);
        let (p, l) = n("試しの四角");
        assert_eq!(ylb_model_begin(h, p, l), 0);
        let ((mp, ml), (gp, _), (sp, sl), (tp, tl)) = (
            n("Body"),
            n("0123456789abcdef0123456789abcdef"),
            n("Hidden/lilToonOutline"),
            n("_MainTex"),
        );
        assert_eq!(ylb_model_material(h, 0, mp, ml, gp, 32, 2, sp, sl), 0);
        assert_eq!(ylb_model_material_texture(h, 0, tp, tl, 64, 64), 0);
        assert_eq!(ylb_model_material_route(h, 0, 0, tp, tl), 0);
        let pos: [f32; 12] = [0., 0., 0., 1., 0., 0., 0., 1., 0., 1., 1., 0.];
        let uv: [f32; 8] = [0., 0., 1., 0., 0., 1., 1., 1.];
        let (kp, kl) = n("0");
        let (qp, ql) = n("Quad");
        let mesh = ylb_model_mesh(
            h,
            kp,
            kl,
            qp,
            ql,
            0,
            pos.as_ptr(),
            std::ptr::null(),
            uv.as_ptr(),
            4,
        );
        assert_eq!(mesh, 0);
        let idx: [i32; 6] = [0, 2, 1, 1, 2, 3];
        assert_eq!(ylb_model_submesh(h, mesh, 0, idx.as_ptr(), 6), 0);
        assert!(ylb_model_send(h) >= 1);
        // 値（C# の LiveLinkMaterialValues.Send と同じ並び: 始め・プロパティ・キーワード・スロット・送る・絵）
        let (op, ol) = n("lilToon 2.3.4 · Standard/Opaque+Outline");
        assert_eq!(ylb_values_begin(h, 0, 1, sp, sl, op, ol), 0);
        let (p, l) = n("_UseShadow");
        assert_eq!(ylb_values_float(h, p, l, 1.0), 0);
        let (p, l) = n("_ShadowBorder");
        assert_eq!(ylb_values_float(h, p, l, 0.35), 0);
        let (p, l) = n("_ShadowColor");
        assert_eq!(ylb_values_color(h, p, l, 0.4, 0.3, 0.5, 1.0), 0);
        let (p, l) = n("_MatCapTex");
        assert_eq!(ylb_values_slot(h, p, l, 1, 2, 2), 0);
        assert_eq!(ylb_values_send(h), 1);
        let pixels = [[0u8, 200, 0, 255]; 4].concat();
        assert_eq!(
            ylb_texture_send(h, 0, p, l, 2, 2, 1, pixels.as_ptr(), pixels.len() as i32),
            1
        );
        // スタンドアロンが受けたのを見届けてから切る
        let _ = done.recv_timeout(WATCHDOG);
        assert_eq!(ylb_disconnect(h), 0);
    }
}

#[test]
fn headless_values_sent_through_the_real_bridge_draw_the_set() {
    let (mut a, name) = Headless::listen("bridge");
    let (tx, rx) = mpsc::channel();
    let unity = std::thread::spawn(move || bridge_sends_values(name, rx));
    a.until("値と絵", |s| {
        received(s).is_some_and(|r| r.images.contains_key("_MatCapTex"))
    });
    let drawn = a.state.doc.drawn_look().clone();
    assert_eq!(drawn.kind, LookKind::LilToon);
    assert_eq!(drawn.shader, "Hidden/lilToonOutline");
    assert_eq!(drawn.float("_ShadowBorder", 0.5), 0.35);
    assert_eq!(drawn.vec4("_ShadowColor", [0.0; 4]), [0.4, 0.3, 0.5, 1.0]);
    let r = received(&a.state).unwrap();
    assert_eq!(r.images["_MatCapTex"].pixels[..4], [0, 200, 0, 255]);
    assert!(r.images["_MatCapTex"].srgb);
    assert_eq!(a.state.doc.undo_count(), 0);
    tx.send(()).unwrap();
    unity.join().unwrap();
    a.until("切れた", |s| s.link.status == LinkStatus::Listening);
    assert!(received(&a.state).is_some(), "最後の値が残る");
}
