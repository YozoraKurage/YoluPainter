//! Live Link の元の絵（スタンドアロンの側、画面なし）: Unity の役（yolu-protocol で直につなぐ試験のスレッド）が元のテクスチャを送ると、
//! 新しく作ったテクスチャセットの一番下に「元の絵」の層として入る。入るまで、そのセットは Unity に出さない（Undo の段は増えない・保存と
//! 復元で普通の層）。描いたセット・編集したセット・利用者が開いたプロジェクトのセット・描いている最中の文書には入れない。大きさが違えば
//! セットの大きさへ拡大縮小する。元の絵を送らない（印の無い）Unity とは今までどおり。
//! 何も触っていない最初のセットは、元の絵の大きさで作り直してから入れる（触ったセット・開いたプロジェクトは作り直さない。予算に入らなければ
//! 今の大きさへ縮めて入れる）。絵の無いマテリアルのセットは、白い「元の絵」で始まる（読めない・大きすぎる・予算超えは白にしない）。
#[path = "../../yolu-protocol/tests/support/wait.rs"]
mod wait;
use wait::WATCHDOG;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use yolu_app::engine::{composite_pixel, layer_pixel, BrushSettings, Channel, PixelClipboard};
use yolu_app::livelink::{LinkStatus, LiveLink, NoticeLevel};
use yolu_app::newproject::NpAction;
use yolu_app::psd::{PsdAction, PsdTarget};
use yolu_app::state::{Action, AppState};
use yolu_protocol::link::connect_and_greet_as;
use yolu_protocol::*;

fn unique_name(tag: &str) -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    format!(
        "ylbase-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

/// 画面を使わずに、AppState と LiveLink を画面のフレームと同じ順（頼み → 受ける → 出す）で回す。
struct Headless {
    ctx: egui::Context,
    state: AppState,
    link: LiveLink,
}

impl Headless {
    fn listen(tag: &str) -> (Headless, String) {
        Self::listen_with(tag, 64)
    }

    /// 最初のセットの文書が `side` の大きさのアプリで待ち受ける。
    fn listen_with(tag: &str, side: u32) -> (Headless, String) {
        let name = unique_name(tag);
        let mut a = Headless {
            ctx: egui::Context::default(),
            state: AppState::new(side, side),
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
        for _ in 0..30 {
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

    /// 名前のセットの位置。
    fn set_index(&self, name: &str) -> usize {
        self.state
            .sets
            .iter()
            .position(|s| s.name == name)
            .unwrap_or_else(|| panic!("セット {name} が無い"))
    }

    fn layer_names(&self, index: usize) -> Vec<String> {
        self.state
            .set_doc(index)
            .layers()
            .iter()
            .map(|l| l.name().to_owned())
            .collect()
    }

    /// 文書に画素のある層を足す（描いたことにする）。
    fn paint(&mut self, index: usize) {
        let doc = self.state.set_doc_mut(index);
        let clip = PixelClipboard::from_image(
            doc.width(),
            doc.height(),
            [90u8, 80, 70, 255].repeat((doc.width() * doc.height()) as usize),
            Channel::Color,
        )
        .unwrap();
        doc.paste_as_layer(&clip, Channel::Color, Some("描いた"), None)
            .unwrap();
    }
}

/// Unity の役。スタンドアロンから来たもの（セットの知らせ・誤りの返事）を覚える。
struct FakeUnity {
    conn: Connection,
    got: Arc<Mutex<Vec<Message>>>,
}

impl FakeUnity {
    fn connect(name: &str, features: u64) -> FakeUnity {
        let identity = Identity::unity("試験の Unity")
            .with_version(Some(AppVersion::new(0, 3, 0)))
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

    /// これまでに知らされたテクスチャセット（マテリアルの番号ごと。最後の知らせ）。
    fn sets(&self) -> Vec<TextureSet> {
        let mut out: Vec<TextureSet> = Vec::new();
        for m in self.got.lock().unwrap().iter() {
            if let Message::TextureSet(t) = m {
                out.retain(|x| x.material != t.material);
                out.push(t.clone());
            }
        }
        out
    }

    fn set_of(&self, material: u32) -> Option<TextureSet> {
        self.sets().into_iter().find(|t| t.material == material)
    }

    /// このセットについて知らされたタイルの更新の数。
    fn tile_updates(&self, set: u32) -> usize {
        self.got
            .lock()
            .unwrap()
            .iter()
            .filter(|m| matches!(m, Message::TilesChanged(t) if t.set == set))
            .count()
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

/// Color の流し込み先のテクスチャの項目があるが、絵が付いていない（Unity は大きさ 0 で知らせる）マテリアル。
fn empty_slot(name: &str) -> MaterialInfo {
    material(name, Some(0))
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

/// 画素 (x, y) が [x, y, x + y, 255] の絵。(3, 3) は RGB を持つ透明な画素。
fn picture(size: u32) -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in 0..size {
        for x in 0..size {
            pixels.extend_from_slice(&if (x, y) == (3, 3) {
                [9, 8, 7, 0]
            } else {
                [x as u8, y as u8, (x + y) as u8, 255]
            });
        }
    }
    pixels
}

fn pixel(image: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * w + x) * 4) as usize;
    [image[i], image[i + 1], image[i + 2], image[i + 3]]
}

fn original(generation: u32, material: u32, size: u32, read: OriginalRead, compressed: bool) -> MaterialOriginal {
    MaterialOriginal {
        generation,
        material,
        slot: "_MainTex".into(),
        state: OriginalState::Image,
        read,
        compressed,
        width: size,
        height: size,
        srgb: true,
        pixels: picture(size),
    }
}

fn declined(generation: u32, material: u32, state: OriginalState) -> MaterialOriginal {
    MaterialOriginal {
        generation,
        material,
        slot: "_MainTex".into(),
        state,
        read: OriginalRead::File,
        compressed: false,
        width: 16384,
        height: 16384,
        srgb: true,
        pixels: Vec::new(),
    }
}

fn read_image(path: &str, w: u32, h: u32) -> Vec<u8> {
    let img = SharedImageReader::open(std::path::Path::new(path)).unwrap();
    let ts = img.layout().tile_size;
    let mut image = vec![0u8; (w * h * 4) as usize];
    for y in 0..h.div_ceil(ts) {
        for x in 0..w.div_ceil(ts) {
            assert_eq!(
                img.read_tile_into_image(x, y, &mut image).unwrap(),
                TileRead::Complete
            );
        }
    }
    image
}

const MARKS: u64 = feature::MATERIAL_VALUES | feature::ORIGINAL_TEXTURES;

#[test]
fn headless_a_new_set_waits_for_its_original_and_gets_it_under_the_first_layer() {
    let (mut a, name) = Headless::listen("under");
    let unity = a.connect(&name, MARKS);
    assert_ne!(a.state.link.common_features() & feature::ORIGINAL_TEXTURES, 0);
    // Body は何も触っていない最初のセット（64 の文書）に付き、Hair は新しいセット（大きさは 256）、Plain は絵が無い
    unity.send(Message::Model(model(
        1,
        vec![
            material("Body", Some(256)),
            material("Hair", Some(128)),
            material("Plain", None),
        ],
    )));
    a.until("絵の無いマテリアルのセット", |_| unity.set_of(2).is_some());
    assert_eq!(a.link.originals_waiting(), 2, "絵のある 2 つのセットは元の絵を待つ");
    a.settle();
    assert!(
        unity.set_of(0).is_none() && unity.set_of(1).is_none(),
        "元の絵が入るまで、空の絵で Unity の表示を置き換えない: {:?}",
        unity.sets().iter().map(|s| s.material).collect::<Vec<_>>()
    );
    assert!(!a.state.link.published.contains(&a.state.sets.get(0).unwrap().uid));
    let (body, hair) = (a.set_index("Body"), a.set_index("Hair"));
    assert_eq!(a.state.set_doc(hair).width(), 256);
    assert_eq!(a.layer_names(body), ["レイヤー 1"]);
    assert_eq!(a.state.set_doc(body).width(), 64, "最初のセットは、元の絵が届くまで作り直さない");
    let before_message = a.state.message.clone();

    // 元の絵が届く: Body は 256（原本のファイルから）、Hair は半分の大きさ（GPU を通して・圧縮から）
    unity.send(Message::MaterialOriginal(original(1, 0, 256, OriginalRead::File, false)));
    unity.send(Message::MaterialOriginal(original(1, 1, 128, OriginalRead::Gpu, true)));
    a.until("元の絵を入れたセット", |_| {
        unity.set_of(0).is_some() && unity.set_of(1).is_some()
    });
    assert_eq!(a.link.originals_waiting(), 0);
    assert!(unity.errors().is_empty(), "{:?}", unity.errors());

    // 何も触っていない最初のセットは、元の絵の大きさ（256）で作り直した文書に替わる（Hair と同じ大きさ）
    assert_eq!(a.state.set_doc(body).width(), 256);
    assert_eq!(a.state.set_doc(body).height(), 256);
    assert_eq!(a.state.sets.get(body).unwrap().id, yolu_app::sets::guid_string(a.state.set_doc(body).id()));
    // 一番下に「元の絵」、その上に最初の層。Undo の段は増えず、編集の印も付かない
    for set in [body, hair] {
        assert_eq!(a.layer_names(set), ["元の絵", "レイヤー 1"], "{set}");
        let doc = a.state.set_doc(set);
        assert!(!doc.can_undo(), "初期化は履歴に入らない");
        assert_eq!(doc.undo_count(), 0);
    }
    assert!(!a.state.modified);
    assert_eq!(a.state.message, before_message, "入れたことは知らせない（入れなかったときだけ）");
    // 同じ大きさ: 画素をそのまま（透明な画素の RGB も）
    let doc = a.state.set_doc(body);
    let bottom = &doc.layers()[0];
    let expected = picture(256);
    for (x, y) in [(0, 0), (5, 5), (3, 3), (255, 255), (10, 40), (200, 100)] {
        assert_eq!(layer_pixel(bottom, x, y), pixel(&expected, 256, x, y), "({x}, {y})");
    }
    assert_eq!(composite_pixel(doc, 5, 5), pixel(&expected, 256, 5, 5), "合成にも出る");
    assert!(
        !yolu_app::engine::layer_has_pixels(&doc.layers()[1]),
        "上の層は空のまま"
    );
    // Unity に出したセットは、最初の知らせから元の絵を含む
    let announced = unity.set_of(0).unwrap();
    assert_eq!((announced.width, announced.height), (256, 256));
    let image = read_image(&announced.channels[0].path, 256, 256);
    assert_eq!(pixel(&image, 256, 5, 5), pixel(&expected, 256, 5, 5));
    assert_eq!(pixel(&image, 256, 10, 40), pixel(&expected, 256, 10, 40));
    // 大きさの違う絵: セットの大きさ（256）へ拡大。端は元の画素
    let doc = a.state.set_doc(hair);
    let bottom = &doc.layers()[0];
    let source = picture(128);
    assert_eq!(layer_pixel(bottom, 0, 0), pixel(&source, 128, 0, 0));
    assert_eq!(layer_pixel(bottom, 255, 255), pixel(&source, 128, 127, 127));
    let mid = layer_pixel(bottom, 128, 128);
    assert_eq!(mid[3], 255);
    assert!((60..=68).contains(&mid[0]), "{mid:?}");
    // 層の欄の印: 読み方・拡大縮小の理由（原本のそのままの値には出さない）
    let marks = &a.state.link_originals;
    let body_mark = marks
        .get(a.state.set_doc(body).id(), a.state.set_doc(body).layers()[0].id())
        .unwrap();
    assert!(!body_mark.is_noted(), "{body_mark:?}");
    let hair_mark = marks
        .get(a.state.set_doc(hair).id(), a.state.set_doc(hair).layers()[0].id())
        .unwrap();
    assert!(hair_mark.gpu && hair_mark.compressed && !hair_mark.converted);
    assert_eq!(hair_mark.resized_from, Some((128, 128)));
    assert!(hair_mark.is_noted());
}

#[test]
fn headless_the_exported_png_holds_the_original_under_the_painting() {
    let (mut a, name) = Headless::listen("export");
    let unity = a.connect(&name, MARKS);
    unity.send(Message::Model(model(1, vec![material("Body", Some(256))])));
    a.until("結び付け", |a| a.state.sets.current().bound == Some(0));
    unity.send(Message::MaterialOriginal(original(1, 0, 256, OriginalRead::File, false)));
    a.until("元の絵", |_| unity.set_of(0).is_some());
    // 元の絵の上に描いた層（半分透明の赤）を足す: 書き出しは、下に元の絵が見える
    let doc = &mut a.state.doc;
    let layer = doc.layers()[1].id();
    let clip = PixelClipboard::from_image(256, 256, [255u8, 0, 0, 128].repeat(256 * 256), Channel::Color).unwrap();
    doc.paste_as_layer(&clip, Channel::Color, Some("赤"), Some(layer)).unwrap();
    let png = yolu_io::composite_png(&a.state.doc).unwrap();
    let image = image::load_from_memory(&png).unwrap().to_rgba8();
    let expected = picture(256);
    let (x, y) = (10u32, 20u32);
    // PNG の行は上から。画素は元の絵に半分透明の赤を重ねた値（元の絵が下にある）
    let got = image.get_pixel(x, 255 - y).0;
    let source = pixel(&expected, 256, x, y);
    for (c, want) in [
        (0, (255.0 * 128.0 + source[0] as f32 * 127.0) / 255.0),
        (1, source[1] as f32 * 127.0 / 255.0),
        (2, source[2] as f32 * 127.0 / 255.0),
    ] {
        assert!((got[c] as f32 - want).abs() <= 2.0, "チャンネル {c}: {got:?} は {want}");
    }
    assert_eq!(got[3], 255, "下の元の絵は不透明");
    // .ylp に入る合成も同じ（セットの合成の PNG）
    let composites = yolu_io::composite_pngs(&a.state.doc).unwrap();
    assert_eq!(composites[0].0, Channel::Color);
    assert_eq!(composites[0].1, png);
}

#[test]
fn headless_a_linear_original_is_stored_as_srgb_pixels() {
    let (mut a, name) = Headless::listen("linear");
    let unity = a.connect(&name, MARKS);
    unity.send(Message::Model(model(1, vec![material("Body", Some(64))])));
    a.until("結び付け", |a| a.state.sets.current().bound == Some(0));
    let mut o = original(1, 0, 64, OriginalRead::Imported, false);
    o.srgb = false;
    o.pixels = [55u8, 0, 255, 200].repeat(64 * 64);
    unity.send(Message::MaterialOriginal(o));
    a.until("元の絵", |_| unity.set_of(0).is_some());
    let doc = &a.state.doc;
    // リニアの 55 は sRGB の 128 に（A はそのまま）
    assert_eq!(layer_pixel(&doc.layers()[0], 7, 7), [128, 0, 255, 200]);
    let mark = a
        .state
        .link_originals
        .get(doc.id(), doc.layers()[0].id())
        .unwrap();
    assert!(mark.converted && mark.is_noted());
}

#[test]
fn headless_originals_leave_painted_sets_and_what_the_user_opened_alone() {
    let (mut a, name) = Headless::listen("painted");
    let unity = a.connect(&name, MARKS);
    // つなぐ前に描いてある最初のセット（Body に付く）には入れない。Hair は新しいセット
    a.paint(0);
    let painted = layer_pixel(&a.state.doc.layers()[1], 5, 5);
    unity.send(Message::Model(model(
        1,
        vec![material("Body", Some(64)), material("Hair", Some(64))],
    )));
    a.until("描いたセット", |_| unity.set_of(0).is_some());
    assert_eq!(a.link.originals_waiting(), 1, "待つのは新しいセットだけ（描いたセットは待たせず出す）");
    unity.send(Message::MaterialOriginal(original(1, 0, 64, OriginalRead::File, false)));
    unity.send(Message::MaterialOriginal(original(1, 1, 64, OriginalRead::File, false)));
    a.until("Hair のセット", |_| unity.set_of(1).is_some());
    a.settle();
    assert_eq!(a.layer_names(a.set_index("Body")), ["レイヤー 1", "描いた"], "描いたセットは変えない");
    assert_eq!(a.state.set_doc(a.set_index("Body")).width(), 64, "描いた最初のセットは作り直さない");
    assert_eq!(layer_pixel(&a.state.doc.layers()[1], 5, 5), painted);
    assert_eq!(a.layer_names(a.set_index("Hair")), ["元の絵", "レイヤー 1"]);
    assert!(
        unity.errors().is_empty(),
        "待たせていないマテリアルの元の絵は黙って捨てる（誤りで返さない）: {:?}",
        unity.errors()
    );
}

#[test]
fn headless_a_set_edited_while_waiting_does_not_get_the_original() {
    let (mut a, name) = Headless::listen("edited");
    let unity = a.connect(&name, MARKS);
    unity.send(Message::Model(model(
        1,
        vec![material("Body", Some(64)), material("Hair", Some(64))],
    )));
    a.until("結び付け", |a| a.state.sets.len() == 2);
    assert_eq!(a.link.originals_waiting(), 2);
    // 元の絵が届く前に、利用者が Hair に描いた
    let hair = a.set_index("Hair");
    a.paint(hair);
    unity.send(Message::MaterialOriginal(original(1, 0, 64, OriginalRead::File, false)));
    unity.send(Message::MaterialOriginal(original(1, 1, 64, OriginalRead::File, false)));
    a.until("2 つのセット", |_| unity.set_of(0).is_some() && unity.set_of(1).is_some());
    assert_eq!(a.layer_names(hair), ["レイヤー 1", "描いた"], "描いたものを黙って変えない");
    assert_eq!(a.layer_names(a.set_index("Body")), ["元の絵", "レイヤー 1"]);
    let (level, text) = a.state.link.notice.clone().unwrap();
    assert_eq!(level, NoticeLevel::Warning);
    assert!(text.contains("元の絵を入れませんでした") && text.contains("Hair"), "{text}");
    assert!(text.contains("すでに編集されています"), "{text}");
    // 描いた内容で出る（元の絵は含まない）
    let announced = unity.set_of(1).unwrap();
    let side = announced.width;
    let image = read_image(&announced.channels[0].path, side, side);
    assert_eq!(pixel(&image, side, 5, 5), [90, 80, 70, 255]);
}

#[test]
fn headless_a_declined_original_lets_the_set_out_empty_with_the_reason() {
    let (mut a, name) = Headless::listen("declined");
    let unity = a.connect(&name, MARKS);
    unity.send(Message::Model(model(
        1,
        vec![material("Body", Some(64)), material("Hair", Some(64))],
    )));
    a.until("結び付け", |a| a.state.sets.len() == 2);
    unity.send(Message::MaterialOriginal(original(1, 0, 64, OriginalRead::File, false)));
    unity.send(Message::MaterialOriginal(declined(1, 1, OriginalState::TooLarge)));
    a.until("2 つのセット", |_| unity.set_of(0).is_some() && unity.set_of(1).is_some());
    assert_eq!(a.layer_names(a.set_index("Hair")), ["レイヤー 1"], "絵が付かなければ層を足さない");
    assert_eq!(
        composite_pixel(a.state.set_doc(a.set_index("Hair")), 5, 5),
        [0, 0, 0, 0],
        "絵が付かなくても白で埋めない"
    );
    let (level, text) = a.state.link.notice.clone().unwrap();
    assert_eq!(level, NoticeLevel::Warning);
    assert!(text.contains("Hair") && text.contains("大きすぎます（16384×16384）"), "{text}");
    assert!(!text.contains("Body"), "{text}");
    // 理由ごとの文（日本語と英語）
    for (state, ja, en) in [
        (OriginalState::Unreadable, "Unity が読めませんでした", "Unity could not read it"),
        (OriginalState::OverBudget, "一度に送れる量を超えました", "Over the amount Unity sends at once"),
    ] {
        for (lang, want) in [(yolu_app::lang::Lang::Ja, ja), (yolu_app::lang::Lang::En, en)] {
            let (mut b, name) = Headless::listen("reason");
            b.state.lang = lang;
            let unity = b.connect(&name, MARKS);
            unity.send(Message::Model(model(1, vec![material("Body", Some(64))])));
            b.until("結び付け", |b| b.state.sets.current().bound == Some(0));
            unity.send(Message::MaterialOriginal(declined(1, 0, state)));
            b.until("セット", |_| unity.set_of(0).is_some());
            let (_, text) = b.state.link.notice.clone().unwrap();
            assert!(text.contains(want), "{lang:?} {state:?}: {text}");
            // 最初のセットも、読めない・大きすぎる・予算超えの絵では作り直さず、白で埋めない
            assert_eq!(b.state.doc.width(), 64, "{lang:?} {state:?}");
            assert_eq!(b.layer_names(0).len(), 1, "{lang:?} {state:?}");
            assert_eq!(composite_pixel(&b.state.doc, 5, 5), [0, 0, 0, 0], "{lang:?} {state:?}");
        }
    }
}

#[test]
fn headless_a_stroke_in_progress_delays_the_original_until_it_ends() {
    let (mut a, name) = Headless::listen("stroke");
    let unity = a.connect(&name, MARKS);
    unity.send(Message::Model(model(1, vec![material("Body", Some(64))])));
    a.until("結び付け", |a| a.state.sets.current().bound == Some(0));
    // 描き始めたところ（まだ何も変えていない）へ元の絵が届く: 描いている最中は文書を変えず、セットも出さない
    let layer = a.state.doc.layers()[0].id();
    let stroke = a.state.doc.begin_stroke(layer, &BrushSettings::default()).unwrap();
    unity.send(Message::MaterialOriginal(original(1, 0, 64, OriginalRead::File, false)));
    a.settle();
    assert_eq!(a.layer_names(0), ["レイヤー 1"]);
    assert_eq!(a.link.originals_waiting(), 1, "届いたが入れるのを待っている");
    assert!(unity.set_of(0).is_none());
    // 描くのをやめる（何も描かなかった）と、そのあとのフレームで入る
    a.state.doc.cancel_stroke(stroke);
    a.until("元の絵を入れたセット", |_| unity.set_of(0).is_some());
    assert_eq!(a.layer_names(0), ["元の絵", "レイヤー 1"]);
}

#[test]
fn headless_a_unity_without_the_mark_gets_nothing_held_and_a_stray_original_is_refused() {
    let (mut a, name) = Headless::listen("nomark");
    // 元の絵の印を出さない Unity（この機能より古いパッケージ）: セットは待たせずに出し、元の絵は入れない
    let unity = a.connect(&name, feature::MATERIAL_VALUES);
    assert_eq!(a.state.link.common_features() & feature::ORIGINAL_TEXTURES, 0);
    unity.send(Message::Model(model(1, vec![material("Body", Some(64)), material("Hair", Some(64))])));
    a.until("2 つのセット", |_| unity.set_of(0).is_some() && unity.set_of(1).is_some());
    assert_eq!(a.link.originals_waiting(), 0);
    assert!(
        !unity
            .conn
            .send_gated(&Message::MaterialOriginal(original(1, 0, 64, OriginalRead::File, false)))
            .unwrap(),
        "印の無い相手へは送らない"
    );
    // 守らずに送ってきても入れない（命令の食い違いとして、誤りで返す）
    unity.send(Message::MaterialOriginal(original(1, 1, 64, OriginalRead::File, false)));
    a.until("誤りの返事", |_| unity.errors().len() == 1);
    assert_eq!(unity.errors()[0].code, ErrorCode::Refused);
    assert_eq!(a.layer_names(a.set_index("Hair")), ["レイヤー 1"]);
}

#[test]
fn headless_a_mismatched_original_is_answered_with_an_error_and_changes_nothing() {
    let (mut a, name) = Headless::listen("mismatch");
    let unity = a.connect(&name, MARKS);
    unity.send(Message::Model(model(2, vec![material("Body", Some(64)), material("Hair", Some(64))])));
    a.until("結び付け", |a| a.state.sets.len() == 2);
    let shown = a.state.message.clone();
    // 古い世代・知らせていないスロット: 何も変えずに断る（画面の知らせには出さない）
    unity.send(Message::MaterialOriginal(original(1, 0, 64, OriginalRead::File, false)));
    let mut other_slot = original(2, 1, 64, OriginalRead::File, false);
    other_slot.slot = "_OtherTex".into();
    unity.send(Message::MaterialOriginal(other_slot));
    a.until("誤りの返事", |_| unity.errors().len() == 2);
    assert!(unity
        .errors()
        .iter()
        .all(|e| e.code == ErrorCode::Refused && e.kind == Kind::MaterialOriginal as u16));
    assert_eq!(a.link.originals_waiting(), 2, "待ちは変わらない");
    assert_eq!(a.state.message, shown);
    assert_eq!(a.layer_names(0), ["レイヤー 1"]);
    // 正しい元の絵はそのあとも入る
    unity.send(Message::MaterialOriginal(original(2, 0, 64, OriginalRead::File, false)));
    unity.send(Message::MaterialOriginal(original(2, 1, 64, OriginalRead::File, false)));
    a.until("2 つのセット", |_| unity.set_of(0).is_some() && unity.set_of(1).is_some());
    assert_eq!(a.layer_names(a.set_index("Hair")), ["元の絵", "レイヤー 1"]);
}

#[test]
fn headless_an_original_command_that_cannot_be_read_lets_every_waiting_set_out_at_once() {
    let (mut a, name) = Headless::listen("unreadable");
    let unity = a.connect(&name, MARKS);
    unity.send(Message::Model(model(1, vec![material("Body", Some(64)), material("Hair", Some(64))])));
    a.until("結び付け", |a| a.state.sets.len() == 2);
    assert_eq!(a.link.originals_waiting(), 2);
    // 元の絵でない命令が読めなくても、待ちは変わらない（読めなかった命令の種類を見て、元の絵のときだけ出す）
    unity.conn.send_raw(Kind::MaterialValues as u16, &[1, 0]).unwrap();
    a.until("値の命令の誤り", |_| {
        unity.errors().iter().any(|e| e.kind == Kind::MaterialValues as u16)
    });
    a.settle();
    assert_eq!(a.link.originals_waiting(), 2, "元の絵の命令でない誤りでは出さない");
    assert!(unity.set_of(0).is_none() && unity.set_of(1).is_none());
    // 元の絵の命令が読めない（画素が途中で切れている）: どの絵が欠けたか分からないので、待たずに全部出す。STALL（30 秒）を待った出し方と
    // 区別するため、その半分の時間のうちに出ることを見る
    let mut payload = Message::MaterialOriginal(original(1, 0, 64, OriginalRead::File, false)).encode_payload();
    payload.truncate(payload.len() - 100);
    let sent = Instant::now();
    unity.conn.send_raw(Kind::MaterialOriginal as u16, &payload).unwrap();
    a.until("2 つのセット", |_| unity.set_of(0).is_some() && unity.set_of(1).is_some());
    assert!(
        sent.elapsed() < yolu_app::livelink_base::STALL / 2,
        "待たずに出る: {:?}",
        sent.elapsed()
    );
    assert_eq!(a.link.originals_waiting(), 0);
    assert!(
        unity.errors().iter().any(|e| e.kind == Kind::MaterialOriginal as u16),
        "読めない命令は Unity へ誤りで返る"
    );
    // 画面の知らせは、読めなかった命令の警告（STALL の「届きませんでした」ではない）
    let (level, text) = a.state.link.notice.clone().unwrap();
    assert_eq!(level, NoticeLevel::Warning);
    assert!(text.contains("MaterialOriginal") && text.contains("読めません"), "{text}");
    assert!(!text.contains("届きませんでした"), "{text}");
    // 元の絵は入らず、出したあとに届いた元の絵も（待たせていないので）入れない
    assert_eq!(a.layer_names(a.set_index("Body")), ["レイヤー 1"]);
    assert_eq!(a.layer_names(a.set_index("Hair")), ["レイヤー 1"]);
    unity.send(Message::MaterialOriginal(original(1, 0, 64, OriginalRead::File, false)));
    a.settle();
    assert_eq!(a.layer_names(a.set_index("Body")), ["レイヤー 1"]);
}

#[test]
fn headless_a_model_sent_again_keeps_waiting_and_an_installed_original_is_not_replaced() {
    let (mut a, name) = Headless::listen("again");
    let unity = a.connect(&name, MARKS);
    unity.send(Message::Model(model(1, vec![material("Body", Some(64)), material("Hair", Some(64))])));
    a.until("結び付け", |a| a.state.sets.len() == 2);
    // 元の絵が揃う前に、Unity がモデルを送り直した（構造の変化）: 待ちは持ち越され、新しい世代の元の絵を待つ
    unity.send(Message::Model(model(2, vec![material("Body", Some(64)), material("Hair", Some(64))])));
    a.until("新しい世代", |a| a.state.model.as_ref().is_some_and(|m| m.generation == 2));
    assert_eq!(a.link.originals_waiting(), 2);
    a.settle();
    assert!(unity.set_of(0).is_none() && unity.set_of(1).is_none());
    unity.send(Message::MaterialOriginal(original(2, 0, 64, OriginalRead::File, false)));
    unity.send(Message::MaterialOriginal(original(2, 1, 64, OriginalRead::File, false)));
    a.until("2 つのセット", |_| unity.set_of(0).is_some() && unity.set_of(1).is_some());
    // 入れたあとの送り直し: 元の絵がまた届いても、描いていなくても 2 枚目は入らない
    unity.send(Message::Model(model(3, vec![material("Body", Some(64)), material("Hair", Some(64))])));
    a.until("3 世代目", |a| a.state.model.as_ref().is_some_and(|m| m.generation == 3));
    assert_eq!(a.link.originals_waiting(), 0, "入れたセットは文書が変わっているので待たせない");
    unity.send(Message::MaterialOriginal(original(3, 0, 64, OriginalRead::File, false)));
    unity.send(Message::MaterialOriginal(original(3, 1, 64, OriginalRead::File, false)));
    a.settle();
    assert_eq!(a.layer_names(a.set_index("Body")), ["元の絵", "レイヤー 1"]);
    assert_eq!(a.layer_names(a.set_index("Hair")), ["元の絵", "レイヤー 1"]);
    assert_eq!(a.state.set_doc(a.set_index("Body")).width(), 256, "持ち越した待ちも、最初のセットを作り直して入れる");
    assert!(unity.errors().is_empty(), "{:?}", unity.errors());
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

#[test]
fn headless_the_original_is_an_ordinary_layer_after_saving_and_opening() {
    let dir = Dir::new("save");
    let path = dir.0.join("元の絵.ylp");
    let (mut a, name) = Headless::listen("save");
    let unity = a.connect(&name, MARKS);
    unity.send(Message::Model(model(1, vec![material("Body", Some(256)), material("Hair", Some(128))])));
    a.until("結び付け", |a| a.state.sets.len() == 2);
    unity.send(Message::MaterialOriginal(original(1, 0, 256, OriginalRead::File, false)));
    unity.send(Message::MaterialOriginal(original(1, 1, 128, OriginalRead::Imported, false)));
    a.until("2 つのセット", |_| unity.set_of(0).is_some() && unity.set_of(1).is_some());
    a.state.apply(Action::SaveProjectAs(path.clone()));
    assert!(a.state.message.starts_with("保存しました"), "{}", a.state.message);

    // 開き直す: 普通のピクセルレイヤーとして復元される（描いた層の下、画素は保存したまま）
    let mut b = AppState::new(64, 64);
    b.apply(Action::OpenProject(path.clone()));
    assert_eq!(b.sets.len(), 2);
    for (i, expected) in [(0usize, picture(256)), (1, picture(128))] {
        let set = b.sets.iter().position(|s| s.name == a.state.sets.get(i).unwrap().name).unwrap();
        let doc = b.set_doc(set);
        let names: Vec<&str> = doc.layers().iter().map(|l| l.name()).collect();
        assert_eq!(names, ["元の絵", "レイヤー 1"]);
        assert_eq!(layer_pixel(&doc.layers()[0], 0, 0), pixel(&expected, if i == 0 { 256 } else { 128 }, 0, 0));
        if i == 0 {
            // 同じ大きさで入れた絵は、透明な画素の RGB も保存される
            assert_eq!(layer_pixel(&doc.layers()[0], 3, 3), [9, 8, 7, 0]);
        }
        assert!(doc.undo_count() == 0);
    }
    // 開き直した後の印は無い（層は普通のピクセルレイヤー）
    assert!(b.link_originals.is_empty());
    // 開いたプロジェクトのセットには、Unity につなぎ直しても元の絵を入れない
    let (mut c, name) = Headless::listen("reopen");
    c.state.apply(Action::OpenProject(path));
    let unity = c.connect(&name, MARKS);
    unity.send(Message::Model(model(1, vec![material("Body", Some(256)), material("Hair", Some(128))])));
    c.until("セット", |_| unity.set_of(0).is_some() && unity.set_of(1).is_some());
    assert_eq!(c.link.originals_waiting(), 0, "開いたプロジェクトのセットは待たせない");
    unity.send(Message::MaterialOriginal(original(1, 0, 256, OriginalRead::File, false)));
    c.settle();
    assert_eq!(c.layer_names(c.set_index("Body")), ["元の絵", "レイヤー 1"], "二重に入らない");
}

#[test]
fn headless_an_original_over_the_documents_pixel_budget_is_refused_with_the_reason() {
    let (mut a, name) = Headless::listen("budget");
    let unity = a.connect(&name, MARKS);
    unity.send(Message::Model(model(1, vec![material("Body", Some(64))])));
    a.until("結び付け", |a| a.state.sets.current().bound == Some(0));
    // 層の画素の予算が足りない文書（予算を超えて入れない。黙って切り詰めない）
    a.state.doc.set_source_budget_bytes(1024).unwrap();
    unity.send(Message::MaterialOriginal(original(1, 0, 64, OriginalRead::File, false)));
    a.until("セット", |_| unity.set_of(0).is_some());
    assert_eq!(a.layer_names(0), ["レイヤー 1"]);
    let (level, text) = a.state.link.notice.clone().unwrap();
    assert_eq!(level, NoticeLevel::Warning);
    assert!(text.contains("元の絵を入れませんでした") && text.contains("予算"), "{text}");
}

/// PSD を書く（赤で塗った 1 枚のレイヤーを持つ、size × size の文書）。
fn write_red_psd(dir: &Dir, size: u32) -> PathBuf {
    let mut src = AppState::new(size, size);
    let clip = PixelClipboard::from_image(
        size,
        size,
        [255u8, 0, 0, 255].repeat((size * size) as usize),
        Channel::Color,
    )
    .unwrap();
    src.doc
        .paste_as_layer(&clip, Channel::Color, Some("赤"), None)
        .unwrap();
    let path = dir.0.join("Paint.psd");
    src.apply(Action::Psd(PsdAction::Export(path.clone())));
    src.wait_psd();
    assert!(path.exists(), "{}", src.message);
    path
}

#[test]
fn headless_a_psd_imported_after_linking_replaces_the_set_unity_shows_it_and_the_project_saves() {
    let dir = Dir::new("psd");
    let psd = write_red_psd(&dir, 256);
    let (mut a, name) = Headless::listen("psd");
    let unity = a.connect(&name, MARKS);
    unity.send(Message::Model(model(1, vec![material("Body", Some(256)), material("Hair", Some(64))])));
    a.until("結び付け", |a| a.state.sets.len() == 2);
    unity.send(Message::MaterialOriginal(original(1, 0, 256, OriginalRead::File, false)));
    unity.send(Message::MaterialOriginal(original(1, 1, 64, OriginalRead::File, false)));
    a.until("2 つのセット", |_| unity.set_of(0).is_some() && unity.set_of(1).is_some());
    let body = a.set_index("Body");
    assert_eq!(a.state.sets.current_index(), body);
    let updates = unity.tile_updates(unity.set_of(0).unwrap().set);

    // つないだ後から PSD を、今のセット（Body。Unity の Body のマテリアルに付いている）の文書へ取り込む
    a.state.apply(Action::Psd(PsdAction::Import {
        path: psd.clone(),
        target: PsdTarget::CurrentSet,
    }));
    a.state.wait_psd();
    assert!(a.state.message.contains("PSD を読み込みました"), "{}", a.state.message);
    assert_eq!(a.layer_names(body), ["レイヤー 1", "赤"], "PSD の層に替わる（元の絵の層は、文書ごと替わる）");
    // Unity にも届く（同じセットの全タイルが新しい中身で書き直される）
    a.until("書き直し", |_| unity.tile_updates(unity.set_of(0).unwrap().set) > updates);
    let announced = unity.set_of(0).unwrap();
    let image = read_image(&announced.channels[0].path, 256, 256);
    assert_eq!(pixel(&image, 256, 5, 5), [255, 0, 0, 255]);
    assert!(unity.errors().is_empty(), "{:?}", unity.errors());

    // 新しいテクスチャセットとしても取り込める（モデルのマテリアルに付けなければ Unity には出ない）
    let sets = a.state.sets.len();
    a.state.apply(Action::Psd(PsdAction::Import {
        path: psd,
        target: PsdTarget::NewSet,
    }));
    a.state.wait_psd();
    assert_eq!(a.state.sets.len(), sets + 1);
    a.settle();
    assert_eq!(unity.sets().len(), 2, "名前の合うマテリアルが無ければ、Unity に出さない");

    // .ylp に保存して開き直すと、PSD の層も Live Link で作ったセットもそのまま
    let path = dir.0.join("リンクのあと.ylp");
    a.state.apply(Action::SaveProjectAs(path.clone()));
    assert!(a.state.message.starts_with("保存しました"), "{}", a.state.message);
    let mut b = AppState::new(64, 64);
    b.apply(Action::OpenProject(path));
    assert_eq!(b.sets.len(), 3);
    let reopened = b.sets.iter().position(|s| s.name == "Body").unwrap();
    assert_eq!(composite_pixel(b.set_doc(reopened), 5, 5), [255, 0, 0, 255]);
    let hair = b.sets.iter().position(|s| s.name == "Hair").unwrap();
    assert_eq!(
        b.set_doc(hair).layers().iter().map(|l| l.name().to_owned()).collect::<Vec<_>>(),
        ["元の絵", "レイヤー 1"],
        "つないで作ったセットの元の絵も保存される"
    );
}

// ───────── 何も触っていない最初のセットを、元の絵の大きさで作り直す ─────────

#[test]
fn headless_the_untouched_first_set_is_rebuilt_at_the_originals_size() {
    // 最初のセットは既定の大きさ（ここでは 2048）。元の絵は 512 なので、512 の文書に作り直す
    let (mut a, name) = Headless::listen_with("rebuild", 2048);
    let unity = a.connect(&name, MARKS);
    let first = a.state.doc.id();
    let uid = a.state.sets.current().uid;
    unity.send(Message::Model(model(1, vec![material("Body", Some(512))])));
    a.until("結び付け", |a| a.state.sets.current().bound == Some(0));
    assert_eq!(a.link.originals_waiting(), 1);
    assert_eq!(a.state.doc.width(), 2048, "元の絵が届くまでは作り直さない");
    unity.send(Message::MaterialOriginal(original(1, 0, 512, OriginalRead::File, false)));
    a.until("元の絵を入れたセット", |_| unity.set_of(0).is_some());

    let doc = &a.state.doc;
    assert_eq!((doc.width(), doc.height()), (512, 512));
    assert_ne!(doc.id(), first, "文書は作り直した");
    assert_eq!(a.state.sets.current().uid, uid, "セットは同じ（uid・名前・マテリアルへの結び付け）");
    assert_eq!(a.state.sets.current().bound, Some(0));
    assert_eq!(a.state.sets.current().id, yolu_app::sets::guid_string(doc.id()));
    assert_eq!(a.layer_names(0), ["元の絵", "レイヤー 1"]);
    // 同じ大きさなので画素はそのまま（透明な画素の RGB も）。印も付かない
    let expected = picture(512);
    for (x, y) in [(0, 0), (5, 5), (3, 3), (511, 511), (300, 77)] {
        assert_eq!(layer_pixel(&doc.layers()[0], x, y), pixel(&expected, 512, x, y), "({x}, {y})");
    }
    assert_eq!(layer_pixel(&doc.layers()[0], 3, 3), [9, 8, 7, 0]);
    let mark = a.state.link_originals.get(doc.id(), doc.layers()[0].id()).unwrap();
    assert!(!mark.is_noted(), "{mark:?}");
    // 作った直後の初期化: Undo の段にならず、変更の印も付かない
    assert!(!doc.can_undo());
    assert_eq!(doc.undo_count(), 0);
    assert!(!a.state.modified);
    assert_eq!(a.state.selected_layer, Some(doc.layers()[1].id()), "選んでいる層は新しい文書の最初の層");
    // Unity には、作り直した大きさで、元の絵を含んで出る（空の 2048 は出さない）
    let announced = unity.set_of(0).unwrap();
    assert_eq!((announced.width, announced.height), (512, 512));
    let image = read_image(&announced.channels[0].path, 512, 512);
    assert_eq!(pixel(&image, 512, 300, 77), pixel(&expected, 512, 300, 77));
    assert!(unity.errors().is_empty(), "{:?}", unity.errors());
}

#[test]
fn headless_the_first_set_gets_the_same_size_as_a_new_set_for_the_same_texture() {
    // 600 の絵: 新しいセットは辺を 2 の冪に丸めた 1024。何も触っていない最初のセットも同じ大きさで、拡大した印が付く
    let (mut a, name) = Headless::listen("round");
    let unity = a.connect(&name, MARKS);
    unity.send(Message::Model(model(
        1,
        vec![material("Body", Some(600)), material("Hair", Some(600))],
    )));
    a.until("結び付け", |a| a.state.sets.len() == 2);
    let hair = a.set_index("Hair");
    assert_eq!(a.state.set_doc(hair).width(), 1024, "新しいセットは作るときに丸める");
    unity.send(Message::MaterialOriginal(original(1, 0, 600, OriginalRead::File, false)));
    unity.send(Message::MaterialOriginal(original(1, 1, 600, OriginalRead::File, false)));
    a.until("2 つのセット", |_| unity.set_of(0).is_some() && unity.set_of(1).is_some());
    let body = a.set_index("Body");
    for set in [body, hair] {
        let doc = a.state.set_doc(set);
        assert_eq!((doc.width(), doc.height()), (1024, 1024), "{set}");
        let mark = a.state.link_originals.get(doc.id(), doc.layers()[0].id()).unwrap();
        assert_eq!(mark.resized_from, Some((600, 600)), "{set}");
    }
}

#[test]
fn headless_a_4096_original_no_longer_shrinks_into_a_default_sized_first_set() {
    // 既定の 2048 の最初のセットへ縮んでいた 4096 の絵: 4096 の文書に作り直して、画素をそのまま入れる（印も付かない）
    let (mut a, name) = Headless::listen_with("big", 2048);
    let unity = a.connect(&name, MARKS);
    unity.send(Message::Model(model(1, vec![material("Body", Some(4096))])));
    a.until("結び付け", |a| a.state.sets.current().bound == Some(0));
    let mut o = original(1, 0, 4096, OriginalRead::File, false);
    o.pixels = [30u8, 60, 90, 255].repeat(4096 * 4096);
    unity.send(Message::MaterialOriginal(o));
    a.until("元の絵を入れたセット", |_| unity.set_of(0).is_some());
    let doc = &a.state.doc;
    assert_eq!((doc.width(), doc.height()), (4096, 4096));
    assert_eq!(layer_pixel(&doc.layers()[0], 4095, 4095), [30, 60, 90, 255]);
    assert_eq!(layer_pixel(&doc.layers()[0], 100, 3000), [30, 60, 90, 255]);
    let mark = a.state.link_originals.get(doc.id(), doc.layers()[0].id()).unwrap();
    assert!(!mark.is_noted(), "縮めていないので印は付かない: {mark:?}");
    assert!(unity.errors().is_empty(), "{:?}", unity.errors());
}

#[test]
fn headless_a_first_set_edited_while_waiting_is_neither_rebuilt_nor_given_the_original() {
    let (mut a, name) = Headless::listen("first-edited");
    let unity = a.connect(&name, MARKS);
    unity.send(Message::Model(model(1, vec![material("Body", Some(512))])));
    a.until("結び付け", |a| a.state.sets.current().bound == Some(0));
    assert_eq!(a.link.originals_waiting(), 1);
    // 元の絵が届く前に、利用者が最初のセットに描いた
    a.paint(0);
    unity.send(Message::MaterialOriginal(original(1, 0, 512, OriginalRead::File, false)));
    a.until("セット", |_| unity.set_of(0).is_some());
    assert_eq!(a.state.doc.width(), 64, "触ったセットは作り直さない");
    assert_eq!(a.layer_names(0), ["レイヤー 1", "描いた"]);
    let (level, text) = a.state.link.notice.clone().unwrap();
    assert_eq!(level, NoticeLevel::Warning);
    assert!(text.contains("Body") && text.contains("すでに編集されています"), "{text}");
    assert_eq!(layer_pixel(&a.state.doc.layers()[1], 5, 5), [90, 80, 70, 255]);
}

#[test]
fn headless_a_first_set_with_only_history_is_not_waited_for_or_rebuilt() {
    // 画素は無いが履歴がある（層を足した）最初のセットは、触ったセット: 待たせず、作り直さず、元の絵も入れない
    let (mut a, name) = Headless::listen("first-history");
    let unity = a.connect(&name, MARKS);
    a.state.doc.add_layer("手を付けた").unwrap();
    unity.send(Message::Model(model(1, vec![material("Body", Some(512))])));
    a.until("セット", |_| unity.set_of(0).is_some());
    assert_eq!(a.link.originals_waiting(), 0);
    unity.send(Message::MaterialOriginal(original(1, 0, 512, OriginalRead::File, false)));
    a.settle();
    assert_eq!(a.state.doc.width(), 64);
    assert_eq!(a.layer_names(0), ["レイヤー 1", "手を付けた"]);
    assert!(unity.errors().is_empty(), "待たせていない元の絵は黙って捨てる: {:?}", unity.errors());
}

#[test]
fn headless_the_first_set_of_a_project_the_user_opened_is_not_rebuilt() {
    let dir = Dir::new("opened");
    let path = dir.0.join("開いた.ylp");
    let mut src = AppState::new(64, 64);
    src.apply(Action::SaveProjectAs(path.clone()));
    assert!(src.message.starts_with("保存しました"), "{}", src.message);
    let (mut c, name) = Headless::listen("opened");
    c.state.apply(Action::OpenProject(path));
    assert_eq!(c.state.doc.width(), 64);
    let unity = c.connect(&name, MARKS);
    unity.send(Message::Model(model(1, vec![material("Body", Some(512))])));
    c.until("セット", |_| unity.set_of(0).is_some());
    assert_eq!(c.link.originals_waiting(), 0, "開いたプロジェクトのセットは待たせない");
    unity.send(Message::MaterialOriginal(original(1, 0, 512, OriginalRead::File, false)));
    c.settle();
    assert_eq!(c.state.doc.width(), 64, "開いたプロジェクトのセットは作り直さない");
    assert_eq!(c.layer_names(0), ["レイヤー 1"]);
}

#[test]
fn headless_an_original_that_does_not_fit_at_its_own_size_is_shrunk_into_the_current_first_set() {
    // 512 の絵（1 MiB）が入らない予算。64 の最初のセットには入る（これまでどおり縮めて入れ、拡大縮小の印を出す）
    for kind in ["画素", "1 回の操作"] {
        let (mut a, name) = Headless::listen("fit");
        let unity = a.connect(&name, MARKS);
        unity.send(Message::Model(model(1, vec![material("Body", Some(512))])));
        a.until("結び付け", |a| a.state.sets.current().bound == Some(0));
        match kind {
            "画素" => a.state.doc.set_source_budget_bytes(300 << 10).unwrap(),
            _ => a.state.doc.set_stroke_budget_bytes(300 << 10).unwrap(),
        }
        let first = a.state.doc.id();
        unity.send(Message::MaterialOriginal(original(1, 0, 512, OriginalRead::File, false)));
        a.until("セット", |_| unity.set_of(0).is_some());
        let doc = &a.state.doc;
        assert_eq!(doc.id(), first, "{kind}: 作り直した文書には入らないので、今の文書へ入れる");
        assert_eq!((doc.width(), doc.height()), (64, 64), "{kind}");
        assert_eq!(a.layer_names(0), ["元の絵", "レイヤー 1"], "{kind}");
        let bottom = layer_pixel(&doc.layers()[0], 30, 30);
        assert_eq!(bottom[3], 255, "{kind}: {bottom:?}");
        assert!((240..=247).contains(&bottom[0]), "{kind}: 8 × 8 の平均 {bottom:?}");
        let mark = a.state.link_originals.get(doc.id(), doc.layers()[0].id()).unwrap();
        assert_eq!(mark.resized_from, Some((512, 512)), "{kind}");
        assert!(mark.is_noted());
        // 入ったので、入れなかった知らせは出さない
        let (level, _) = a.state.link.notice.clone().unwrap();
        assert_eq!(level, NoticeLevel::Info, "{kind}");
        assert!(!doc.can_undo(), "{kind}");
    }
}

#[test]
fn headless_a_first_set_that_is_not_the_current_one_is_rebuilt_too() {
    let (mut a, name) = Headless::listen("stashed");
    let unity = a.connect(&name, MARKS);
    unity.send(Message::Model(model(
        1,
        vec![material("Body", Some(512)), material("Hair", Some(128))],
    )));
    a.until("結び付け", |a| a.state.sets.len() == 2);
    // 元の絵が届く前に、利用者が Hair へ切り替えた（最初のセットはしまわれた文書になる）
    let (body, hair) = (a.set_index("Body"), a.set_index("Hair"));
    a.state.switch_set(hair).unwrap();
    unity.send(Message::MaterialOriginal(original(1, 0, 512, OriginalRead::File, false)));
    unity.send(Message::MaterialOriginal(original(1, 1, 128, OriginalRead::File, false)));
    a.until("2 つのセット", |_| unity.set_of(0).is_some() && unity.set_of(1).is_some());
    assert_eq!(a.state.sets.current_index(), hair);
    assert_eq!(a.state.set_doc(body).width(), 512);
    assert_eq!(a.layer_names(body), ["元の絵", "レイヤー 1"]);
    // 戻ると、作り直した文書が今の文書になる（表示は新しい大きさ）
    a.state.switch_set(body).unwrap();
    assert_eq!((a.state.doc.width(), a.state.doc.height()), (512, 512));
    assert_eq!(a.state.selected_layer, Some(a.state.doc.layers()[1].id()));
    assert!(!a.state.modified);
}

#[test]
fn headless_a_rebuilt_first_set_gets_the_effect_inputs_passed_to_its_new_document() {
    // 効果の入力を渡した覚えの鍵はセットの uid に付いて、文書の ID を含まない。作り直した文書にも、同じ入力を渡し直す
    let (mut a, name) = Headless::listen_with("rebuild-inputs", 2048);
    let unity = a.connect(&name, MARKS);
    unity.send(Message::Model(model(1, vec![material("Body", Some(512))])));
    a.until("結び付け", |a| a.state.sets.current().bound == Some(0));
    // モデルが入ったあと、画面のフレームが同期を回す（最初のセットの文書へ入力が渡る）
    a.state.sync_effects();
    a.state.sync_effects();
    let before = a.state.doc.effect_inputs().clone();
    let first = a.state.doc.id();
    let passed = a.state.fx.inputs.passed;
    assert!(passed > 0, "作り直す前に、最初のセットへ入力が渡っている");
    unity.send(Message::MaterialOriginal(original(1, 0, 512, OriginalRead::File, false)));
    a.until("元の絵を入れたセット", |_| unity.set_of(0).is_some());
    assert_ne!(a.state.doc.id(), first, "文書は作り直した");
    a.state.sync_effects();
    assert!(
        a.state.fx.inputs.passed > passed,
        "作り直した文書へ入力を渡し直した（鍵が同じだからと飛ばさない）"
    );
    assert!(
        a.state.doc.effect_inputs().same_as(&before),
        "作り直した文書の入力は、元の文書と同じ"
    );
    // 渡したあとは、同じ入力を何度も渡さない
    let settled = a.state.fx.inputs.passed;
    a.state.sync_effects();
    a.state.sync_effects();
    assert_eq!(a.state.fx.inputs.passed, settled);
}

#[test]
fn headless_an_original_the_size_of_the_first_set_goes_into_the_same_document() {
    // 最初のセットの大きさが、元の絵の大きさ（辺を丸めたもの）と同じ: 作り直さず、今の文書へ画素をそのまま入れる
    let (mut a, name) = Headless::listen_with("same-size", 256);
    let unity = a.connect(&name, MARKS);
    let first = a.state.doc.id();
    let set_id = a.state.sets.current().id.clone();
    let uid = a.state.sets.current().uid;
    unity.send(Message::Model(model(1, vec![material("Body", Some(256))])));
    a.until("結び付け", |a| a.state.sets.current().bound == Some(0));
    assert_eq!(a.link.originals_waiting(), 1);
    let selected = a.state.selected_layer;
    unity.send(Message::MaterialOriginal(original(1, 0, 256, OriginalRead::File, false)));
    a.until("元の絵を入れたセット", |_| unity.set_of(0).is_some());

    let doc = &a.state.doc;
    assert_eq!(doc.id(), first, "同じ大きさなので文書は作り直さない");
    assert_eq!(a.state.sets.current().id, set_id, "セットの ID も変わらない");
    assert_eq!(a.state.sets.current().uid, uid);
    assert_eq!((doc.width(), doc.height()), (256, 256));
    assert_eq!(a.layer_names(0), ["元の絵", "レイヤー 1"]);
    let expected = picture(256);
    for (x, y) in [(0, 0), (5, 5), (3, 3), (255, 255), (200, 77)] {
        assert_eq!(layer_pixel(&doc.layers()[0], x, y), pixel(&expected, 256, x, y), "({x}, {y})");
    }
    assert_eq!(layer_pixel(&doc.layers()[0], 3, 3), [9, 8, 7, 0], "透明な画素の RGB もそのまま");
    let mark = a.state.link_originals.get(doc.id(), doc.layers()[0].id()).unwrap();
    assert_eq!(mark.resized_from, None, "拡大縮小していないので印は付かない");
    assert!(!mark.is_noted(), "{mark:?}");
    assert!(!doc.can_undo());
    assert_eq!(doc.undo_count(), 0);
    assert!(!a.state.modified);
    assert_eq!(a.state.selected_layer, selected, "選んでいる層はそのまま");
    let announced = unity.set_of(0).unwrap();
    assert_eq!((announced.width, announced.height), (256, 256));
    assert!(unity.errors().is_empty(), "{:?}", unity.errors());
}

#[test]
fn headless_a_project_made_in_the_new_project_window_keeps_the_chosen_resolution() {
    // 新規プロジェクトの窓で解像度を選んで 1 セットだけ作った（描いていない・保存していない）プロジェクトは、元の絵の大きさに作り直さない
    // （選んだ大きさのまま、元の絵を縮めて入れる）
    let (mut a, name) = Headless::listen("window-size");
    let unity = a.connect(&name, MARKS);
    unity.send(Message::Model(model(
        1,
        vec![material("Body", Some(1024)), material("Hair", Some(1024))],
    )));
    a.until("2 つのセット", |a| a.state.sets.len() == 2);
    a.state.apply(Action::Project(NpAction::OpenNew));
    a.state.apply(Action::Project(NpAction::Material(1, false)));
    a.state.apply(Action::Project(NpAction::Resolution(512)));
    a.state.apply(Action::Project(NpAction::Submit));
    assert!(a.state.np.window.is_none(), "{}", a.state.message);
    assert_eq!(a.state.sets.len(), 1);
    assert_eq!(a.state.sets.current().name, "Body");
    assert!(a.state.resolution_chosen);
    assert!(a.state.is_pristine(), "窓で作った 1 セットは、何も触っていないプロジェクトとして元の絵を受ける");
    let first = a.state.doc.id();
    // Unity が送り直し、元の絵（1024）が届く
    unity.send(Message::Model(model(
        2,
        vec![material("Body", Some(1024)), material("Hair", Some(1024))],
    )));
    a.until("新しい世代", |a| a.state.model.as_ref().is_some_and(|m| m.generation == 2));
    assert_eq!(a.link.originals_waiting(), 2, "窓で作った Body と、今回新しくできた Hair");
    unity.send(Message::MaterialOriginal(original(2, 0, 1024, OriginalRead::File, false)));
    a.until("元の絵が入る", |a| a.layer_names(0).first().is_some_and(|n| n == "元の絵"));

    let doc = &a.state.doc;
    assert_eq!(doc.id(), first, "選んだ解像度の文書のまま");
    assert_eq!((doc.width(), doc.height()), (512, 512), "選んだ 512 を元の絵の 1024 で上書きしない");
    let mark = a.state.link_originals.get(doc.id(), doc.layers()[0].id()).unwrap();
    assert_eq!(mark.resized_from, Some((1024, 1024)), "元の絵を選んだ大きさへ縮めて入れた");
    assert!(!doc.can_undo());
    // 既定の大きさの「新規」で作り直せば、窓で選んだ印は外れる
    yolu_app::project::new_into(&mut a.state);
    assert!(!a.state.resolution_chosen);
}

// ───────── 絵の無いマテリアルは、白い「元の絵」で始める ─────────

#[test]
fn headless_a_material_without_a_texture_starts_from_a_white_original() {
    let dir = Dir::new("white");
    let path = dir.0.join("白.ylp");
    let (mut a, name) = Headless::listen("white");
    let unity = a.connect(&name, MARKS);
    // Body = 何も触っていない最初のセット（絵が無い）、Cloth = 新しいセット（絵が無い）、Plain = テクスチャの項目が無い（絵が無いと
    // 言い切れない）、Glow = Color の流し込み先が無い
    let mut glow = empty_slot("Glow");
    glow.routes.clear();
    unity.send(Message::Model(model(
        1,
        vec![empty_slot("Body"), empty_slot("Cloth"), material("Plain", None), glow],
    )));
    a.until("白で始める 2 つのセット", |_| unity.set_of(0).is_some() && unity.set_of(1).is_some());
    assert_eq!(a.link.originals_waiting(), 0, "来る絵は無いので、待たない");
    assert!(unity.errors().is_empty(), "{:?}", unity.errors());

    let (body, cloth, plain) = (a.set_index("Body"), a.set_index("Cloth"), a.set_index("Plain"));
    for set in [body, cloth] {
        let doc = a.state.set_doc(set);
        assert_eq!(a.layer_names(set), ["元の絵", "レイヤー 1"], "{set}");
        let (w, h) = (doc.width(), doc.height());
        for (x, y) in [(0, 0), (5, 5), (w - 1, h - 1), (w / 2, 3)] {
            assert_eq!(layer_pixel(&doc.layers()[0], x, y), [255, 255, 255, 255], "{set} ({x}, {y})");
            assert_eq!(composite_pixel(doc, x, y), [255, 255, 255, 255], "{set} 合成にも出る");
        }
        assert!(!yolu_app::engine::layer_has_pixels(&doc.layers()[1]), "上の層は空のまま");
        // Undo の段にならない・層の欄の印は付けない（Unity が描くのと同じ値）
        assert!(!doc.can_undo() && doc.undo_count() == 0, "{set}");
        assert!(a.state.link_originals.get(doc.id(), doc.layers()[0].id()).is_none(), "{set}");
    }
    assert!(a.state.link_originals.is_empty());
    assert!(!a.state.modified);
    // 最初のセットは大きさを変えない（作り直すのは、届いた元の絵の大きさに合わせるとき）。新しいセットは新しいセットの既定の大きさ
    assert_eq!(a.state.set_doc(body).width(), 64);
    assert_eq!(a.state.set_doc(cloth).width(), 2048);
    // 絵の無いと言い切れないもの・流し込み先の無いものは、今までどおり空
    assert_eq!(a.layer_names(plain), ["レイヤー 1"]);
    assert_eq!(composite_pixel(a.state.set_doc(plain), 5, 5), [0, 0, 0, 0]);
    let glow = a.set_index("Glow");
    assert_eq!(a.layer_names(glow), ["レイヤー 1"]);
    // 知らせは受けた旨だけ（入れなかった理由ではない）
    let (level, _) = a.state.link.notice.clone().unwrap();
    assert_eq!(level, NoticeLevel::Info);
    // Unity には白い絵で出る（空の絵で元の見た目を置き換えない）
    let announced = unity.set_of(0).unwrap();
    let image = read_image(&announced.channels[0].path, announced.width, announced.height);
    assert_eq!(pixel(&image, announced.width, 5, 5), [255, 255, 255, 255]);
    let plain_set = unity.set_of(2).unwrap();
    let image = read_image(&plain_set.channels[0].path, plain_set.width, plain_set.height);
    assert_eq!(pixel(&image, plain_set.width, 5, 5), [0, 0, 0, 0], "項目の無いマテリアルは空のまま");

    // 絵が無いと知らせたマテリアルの元の絵が（食い違って）届いても、入れない・誤りにもしない
    unity.send(Message::MaterialOriginal(original(1, 0, 64, OriginalRead::File, false)));
    a.settle();
    assert_eq!(a.layer_names(body), ["元の絵", "レイヤー 1"]);
    assert_eq!(layer_pixel(&a.state.set_doc(body).layers()[0], 5, 5), [255, 255, 255, 255]);
    assert!(unity.errors().is_empty(), "{:?}", unity.errors());

    // 保存と復元: 普通のピクセルレイヤー（白）。履歴は空
    a.state.apply(Action::SaveProjectAs(path.clone()));
    assert!(a.state.message.starts_with("保存しました"), "{}", a.state.message);
    let mut b = AppState::new(64, 64);
    b.apply(Action::OpenProject(path));
    for name in ["Body", "Cloth"] {
        let set = b.sets.iter().position(|s| s.name == name).unwrap();
        let doc = b.set_doc(set);
        let names: Vec<&str> = doc.layers().iter().map(|l| l.name()).collect();
        assert_eq!(names, ["元の絵", "レイヤー 1"], "{name}");
        assert_eq!(layer_pixel(&doc.layers()[0], 5, 5), [255, 255, 255, 255], "{name}");
        assert_eq!(doc.undo_count(), 0);
    }
    assert!(b.link_originals.is_empty());
}

#[test]
fn headless_a_white_original_waits_for_a_stroke_to_end_and_keeps_the_set_back_meanwhile() {
    let (mut a, name) = Headless::listen("white-stroke");
    let unity = a.connect(&name, MARKS);
    // 描き始めたところ（まだ何も変えていない）へ、絵の無いマテリアルのモデルが届く
    let layer = a.state.doc.layers()[0].id();
    let stroke = a.state.doc.begin_stroke(layer, &BrushSettings::default()).unwrap();
    unity.send(Message::Model(model(1, vec![empty_slot("Body")])));
    a.until("結び付け", |a| a.state.sets.current().bound == Some(0));
    a.settle();
    assert_eq!(a.layer_names(0), ["レイヤー 1"], "描いている最中は文書を変えない");
    assert_eq!(a.link.originals_waiting(), 1);
    assert!(unity.set_of(0).is_none(), "白が入るまで、空の絵で出さない");
    a.state.doc.cancel_stroke(stroke);
    a.until("白で始めるセット", |_| unity.set_of(0).is_some());
    assert_eq!(a.layer_names(0), ["元の絵", "レイヤー 1"]);
    assert_eq!(layer_pixel(&a.state.doc.layers()[0], 5, 5), [255, 255, 255, 255]);
}

#[test]
fn headless_a_painted_first_set_and_an_old_unity_get_no_white() {
    // 描いた最初のセット: 白い層を足さない（描いたものを黙って変えない）
    let (mut a, name) = Headless::listen("white-painted");
    let unity = a.connect(&name, MARKS);
    a.paint(0);
    unity.send(Message::Model(model(1, vec![empty_slot("Body"), empty_slot("Cloth")])));
    a.until("2 つのセット", |_| unity.set_of(0).is_some() && unity.set_of(1).is_some());
    assert_eq!(a.layer_names(a.set_index("Body")), ["レイヤー 1", "描いた"]);
    assert_eq!(a.layer_names(a.set_index("Cloth")), ["元の絵", "レイヤー 1"], "新しいセットには入る");
    // 元の絵の印の無い Unity: 今までどおり（白も入れない）
    let (mut b, name) = Headless::listen("white-old");
    let unity = b.connect(&name, feature::MATERIAL_VALUES);
    unity.send(Message::Model(model(1, vec![empty_slot("Body"), empty_slot("Cloth")])));
    b.until("2 つのセット", |_| unity.set_of(0).is_some() && unity.set_of(1).is_some());
    assert_eq!(b.layer_names(b.set_index("Body")), ["レイヤー 1"]);
    assert_eq!(b.layer_names(b.set_index("Cloth")), ["レイヤー 1"]);
}
