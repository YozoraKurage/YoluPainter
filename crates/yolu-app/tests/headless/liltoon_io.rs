//! 見た目の設定の保存と書き出し（画面なし）: .ylp に保存して開き直すと戻る、読めない look.json は残して知らせる（見た目を変えて
//! 保存すると上書きを知らせる）、読むだけのセットも理由を言う、復旧の書き置きに入る、lilToon の詰め方の画像が書き出しに足される、
//! ひな形の失敗は巻き戻す、描いている間は断る。
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use yolu_app::bake::{BakeAction, BakeBackend};
use yolu_app::engine::DVec2;
use yolu_app::export::ExportAction;
use yolu_app::fx::FxOp;
use yolu_app::lang::Lang;
use yolu_app::look::LookOp;
use yolu_app::m2::Edit;
use yolu_app::recovery::{RecoveryAction, RecoverySettings};
use yolu_app::state::{Action, AppState};
use yolu_core::look::{
    LookKind, LookValue, MissingImage, PlaneSource, ReceivedImage, ReceivedLook, TextureSource,
};
use yolu_core::mesh_maps::MeshMapKind;
use yolu_core::{Channel, ChannelInfo, ChannelKind, ColorSpace, FilterTarget, Rgba8};

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "yolu-liltoon-{name}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Dir(p)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn lil_state() -> AppState {
    let mut s = AppState::new(32, 32);
    s.apply(Action::Look(LookOp::Template));
    s.apply(Action::Look(LookOp::Value {
        name: "_ShadowBorder",
        value: LookValue::Float(0.3),
        drag: false,
    }));
    s.apply(Action::Look(LookOp::Value {
        name: "_ShadowColor",
        value: LookValue::Color([0.4, 0.3, 0.5, 1.0]),
        drag: false,
    }));
    s
}

#[test]
fn the_look_survives_save_and_open_and_is_not_an_unknown_entry() {
    let dir = Dir::new("save");
    let path = dir.0.join("look.ylp");
    let mut s = lil_state();
    let saved = s.doc.look().clone();
    assert_eq!(saved.kind, LookKind::LilToon);
    yolu_app::project::save_from(&mut s, &path);
    assert!(
        s.message.contains("保存しました") || s.message.contains("Saved"),
        "{}",
        s.message
    );
    let mut t = AppState::new(8, 8);
    yolu_app::project::open_into(&mut t, &path);
    assert_eq!(t.doc.look(), &saved, "{}", t.message);
    assert_eq!(t.doc.undo_count(), 0, "開いただけで Undo の段は増えない");
    assert!(
        !t.message.contains("look.json"),
        "知らないエントリとして知らせない: {}",
        t.message
    );
    let project = yolu_io::Project::read(&std::fs::read(&path).unwrap()).unwrap();
    let id = project.sets()[0].id.clone();
    assert!(project.look(&id).unwrap().is_some());
    // 標準へ戻して保存すると、エントリを消す
    t.apply(Action::Look(LookOp::Kind(LookKind::Standard)));
    t.apply(Action::Look(LookOp::Reset(yolu_app::look::Section::Shadow)));
    let mut look = t.doc.look().clone();
    look.textures.clear();
    look.shader.clear();
    look.properties.clear();
    t.doc.set_look(look, false).unwrap();
    yolu_app::project::save_from(&mut t, &path);
    let project = yolu_io::Project::read(&std::fs::read(&path).unwrap()).unwrap();
    assert!(
        project.look(&id).unwrap().is_none(),
        "既定の設定はエントリを書かない"
    );
}

#[test]
fn a_file_without_a_look_opens_standard_while_new_sets_start_as_liltoon() {
    // 今までの .ylp（look.json が無い）は、開いただけで見た目を変えない（標準）。新しい文書と新しいセットの既定は lilToon
    let dir = Dir::new("old");
    let path = dir.0.join("old.ylp");
    let mut s = AppState::new(32, 32);
    assert_eq!(s.doc.look().kind, LookKind::LilToon, "新しい文書の既定");
    s.doc.restore_look(Default::default()).unwrap();
    yolu_app::project::save_from(&mut s, &path);
    let project = yolu_io::Project::read(&std::fs::read(&path).unwrap()).unwrap();
    let id = project.sets()[0].id.clone();
    assert!(
        project.look(&id).unwrap().is_none(),
        "標準の設定は look.json を書かない（今までの .ylp と同じ形）"
    );
    let mut t = AppState::new(8, 8);
    yolu_app::project::open_into(&mut t, &path);
    assert_eq!(t.doc.look().kind, LookKind::Standard, "{}", t.message);
    assert!(t.doc.look().is_default());
    assert!(!t.modified, "開いただけで変えない");
    // 開いたプロジェクトに足す新しいセットは lilToon
    t.add_texture_set().unwrap();
    assert_eq!(t.doc.look().kind, LookKind::LilToon);
    assert_eq!(t.doc.undo_count(), 0, "既定は Undo に入らない");
}

#[test]
fn an_unreadable_look_opens_standard_says_so_and_stays_in_the_file() {
    let dir = Dir::new("unreadable");
    let path = dir.0.join("broken.ylp");
    let mut s = lil_state();
    yolu_app::project::save_from(&mut s, &path);
    // look.json を新しい形式に差し替える（ほかの書き手が書いた想定）
    let bytes = std::fs::read(&path).unwrap();
    let archive = yolu_io::Archive::read(&bytes).unwrap();
    let mut files: std::collections::BTreeMap<String, Vec<u8>> = archive
        .entries()
        .iter()
        .map(|(k, v)| (k.clone(), v.to_vec()))
        .collect();
    let name = files
        .keys()
        .find(|k| k.ends_with("/look.json"))
        .unwrap()
        .clone();
    let newer = br#"{"format": 7, "kind": "lilToon"}"#.to_vec();
    files.insert(name.clone(), newer.clone());
    std::fs::write(
        &path,
        yolu_io::Archive::from_entries(files)
            .unwrap()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    let mut t = AppState::new(8, 8);
    yolu_app::project::open_into(&mut t, &path);
    assert!(t.doc.look().is_default(), "読めない設定は標準で開く");
    assert!(
        t.message.contains("見た目の設定を読めません"),
        "{}",
        t.message
    );
    // 見た目を変えずに保存しても、読めないエントリは残る
    t.apply(Action::NewLayer);
    yolu_app::project::save_from(&mut t, &path);
    let archive = yolu_io::Archive::read(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(archive.entries()[&name].to_vec(), newer);
}

#[test]
fn the_liltoon_template_export_adds_the_slot_images() {
    let dir = Dir::new("export");
    let mut s = lil_state();
    // 影の強度のマスク（ひな形が作ったユーザーチャンネル）の左上に 0 を塗る
    let mask = match s.doc.look().textures["_ShadowStrengthMask"] {
        TextureSource::Channel(c) => c,
        _ => unreachable!(),
    };
    let layer = s.doc.add_layer("マスク").unwrap();
    s.doc.set_channel_enabled(layer, mask, true).unwrap();
    s.doc
        .set_channel_pixel(layer, mask, 0, 0, Rgba8::new(0, 0, 0, 255))
        .unwrap();
    s.doc
        .add_fill_layer(
            "色",
            &[(Channel::Color, Rgba8::new(200, 100, 50, 255))],
            None,
        )
        .unwrap();
    s.apply(Action::Export(ExportAction::TemplateTo {
        id: "liltoon".into(),
        dir: dir.0.clone(),
        sets: None,
    }));
    s.wait_export();
    let mut files: Vec<String> = std::fs::read_dir(&dir.0)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert!(files.contains(&"Texture_Main.png".to_string()), "{files:?}");
    assert!(
        files.contains(&"Texture_ShadowStrengthMask.png".to_string()),
        "{files:?}"
    );
    // 使っていないチャンネル（影色・AO）のスロットは書かない
    assert!(
        !files
            .iter()
            .any(|f| f.contains("ShadowColor") || f.contains("ShadowBorderMask")),
        "{files:?}"
    );
    let image = image::open(dir.0.join("Texture_ShadowStrengthMask.png"))
        .unwrap()
        .to_rgba8();
    // PNG は上から: 文書の (0, 0) は左下 = PNG の最後の行
    let (w, h) = image.dimensions();
    assert_eq!(image.get_pixel(0, h - 1).0, [0, 0, 0, 255]);
    assert_eq!(
        image.get_pixel(w - 1, 0).0,
        [255, 255, 255, 255],
        "何も描いていない所は既定（白）"
    );
}

/// .ylp の最初のセットの look.json を差し替える（ほかの書き手が新しい形式で書いた想定）。
fn replace_look_entry(path: &Path, bytes: &[u8]) -> String {
    let archive = yolu_io::Archive::read(&std::fs::read(path).unwrap()).unwrap();
    let mut files: std::collections::BTreeMap<String, Vec<u8>> = archive
        .entries()
        .iter()
        .map(|(k, v)| (k.clone(), v.to_vec()))
        .collect();
    let project = yolu_io::Project::read(&std::fs::read(path).unwrap()).unwrap();
    let name = format!("sets/{}/look.json", project.sets()[0].id);
    files.insert(name.clone(), bytes.to_vec());
    std::fs::write(
        path,
        yolu_io::Archive::from_entries(files)
            .unwrap()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    name
}

#[test]
fn changing_the_look_over_an_unreadable_entry_overwrites_it_and_the_save_says_so() {
    let dir = Dir::new("overwrite");
    let path = dir.0.join("newer.ylp");
    let mut s = lil_state();
    yolu_app::project::save_from(&mut s, &path);
    let name = replace_look_entry(
        &path,
        br#"{"format": 7, "kind": "lilToon", "fromTheFuture": [1]}"#,
    );
    let mut t = AppState::new(8, 8);
    yolu_app::project::open_into(&mut t, &path);
    assert!(t.doc.look().is_default());
    // 見た目を変えて保存: 今の設定で書き、読めなかったものを上書きしたことを保存の知らせで言う（黙って消さない）
    t.apply(Action::Look(LookOp::Kind(LookKind::LilToon)));
    let changed = t.doc.look().clone();
    yolu_app::project::save_from(&mut t, &path);
    assert!(t.message.starts_with("保存しました"), "{}", t.message);
    assert!(
        t.message
            .contains("読めなかった見た目の設定を上書きしました"),
        "{}",
        t.message
    );
    let archive = yolu_io::Archive::read(&std::fs::read(&path).unwrap()).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&archive.entries()[&name]).unwrap();
    assert_eq!(json["format"], serde_json::json!(1));
    assert!(
        json.get("fromTheFuture").is_none(),
        "新しい形式のキーを形式 1 に混ぜない: {json}"
    );
    let mut u = AppState::new(8, 8);
    yolu_app::project::open_into(&mut u, &path);
    assert_eq!(u.doc.look(), &changed, "{}", u.message);
    // 次の保存では、もう上書きの知らせは出ない
    u.apply(Action::NewLayer);
    yolu_app::project::save_from(&mut u, &path);
    assert!(!u.message.contains("上書きしました"), "{}", u.message);
    // 英語
    let mut e = AppState::new_in(8, 8, Lang::En);
    let path2 = dir.0.join("newer-en.ylp");
    let mut w = lil_state();
    yolu_app::project::save_from(&mut w, &path2);
    replace_look_entry(&path2, br#"{"format": 7}"#);
    yolu_app::project::open_into(&mut e, &path2);
    e.apply(Action::Look(LookOp::Kind(LookKind::LilToon)));
    yolu_app::project::save_from(&mut e, &path2);
    assert!(
        e.message.contains("Overwrote unreadable look settings"),
        "{}",
        e.message
    );
}

fn settings() -> RecoverySettings {
    RecoverySettings {
        interval_seconds: 15,
        strokes_between: 0,
        generations_to_keep: 3,
        directory: None,
        ..RecoverySettings::default()
    }
}

#[test]
fn the_recovery_checkpoint_carries_the_look_and_opening_it_brings_the_look_back() {
    let dir = Dir::new("recovery");
    let root = dir.0.join("recovery");
    let mut s = AppState::new_in(32, 32, Lang::Ja);
    s.recovery.enable(root.clone(), settings()).unwrap();
    s.apply(Action::Look(LookOp::Template));
    s.apply(Action::Look(LookOp::Value {
        name: "_ShadowBorder",
        value: LookValue::Float(0.25),
        drag: false,
    }));
    assert!(s.modified);
    let look = s.doc.look().clone();
    assert_eq!(look.kind, LookKind::LilToon);
    // 間隔が経つまで進めて書き置く
    let from = Instant::now();
    s.recovery_tick_at(from);
    s.recovery_tick_at(from + Duration::from_secs(16));
    s.recovery_wait();
    // 書き置きの世代に look.json がある
    let store = yolu_io::GenerationStore::new(s.recovery.session_dir().unwrap());
    let mut files = store.load().unwrap().files;
    files.remove(yolu_io::INFO_NAME);
    let project = yolu_io::Project::from_entries(files).unwrap();
    let id = project.sets()[0].id.clone();
    assert_eq!(project.look(&id).unwrap().as_ref(), Some(&look));
    // 落ちた体で起動して、その世代を開くと見た目も戻る
    assert!(s.recovery.is_idle());
    drop(s);
    let mut s2 = AppState::new_in(32, 32, Lang::Ja);
    s2.recovery.enable(root, settings()).unwrap();
    assert!(
        s2.recovery.window.is_some(),
        "落ちた体の起動は復旧のウィンドウを出す"
    );
    s2.recovery_apply(RecoveryAction::Open);
    assert!(s2.message.starts_with("復旧しました"), "{}", s2.message);
    assert_eq!(s2.doc.look(), &look);
    assert!(!s2.doc.can_undo(), "復旧は Undo の段を積まない");
}

/// Live Link で Unity から受けた値（試験が文書へ直に入れる）: 範囲 0.25、マットキャップの絵が届いている。
fn received_from_unity() -> ReceivedLook {
    let mut look = yolu_core::look::MaterialLook {
        kind: LookKind::LilToon,
        shader: "Hidden/lilToonOutline".into(),
        ..Default::default()
    };
    look.properties
        .insert("_UseShadow".into(), LookValue::Float(1.0));
    look.properties
        .insert("_ShadowBorder".into(), LookValue::Float(0.25));
    look.textures
        .insert("_MainTex".into(), TextureSource::Channel(Channel::Color));
    let mut r = ReceivedLook {
        look,
        source: "lilToon 2.3.4 · Standard/Opaque+Outline".into(),
        ..Default::default()
    };
    r.images.insert(
        "_MatCapTex".into(),
        std::sync::Arc::new(ReceivedImage {
            width: 2,
            height: 2,
            srgb: true,
            pixels: [[255u8, 0, 0, 255]; 4].concat().into(),
        }),
    );
    r.missing
        .insert("_ShadowColorTex".into(), MissingImage::OverBudget);
    r
}

/// 保存した形（絵の画素は持たず、絵のあったスロットは絵も理由も無い。絵は Live Link の相手の絵のファイルから読み直す）。
fn as_stored(r: &ReceivedLook) -> ReceivedLook {
    let mut out = r.clone();
    for slot in std::mem::take(&mut out.images).into_keys() {
        out.missing.remove(&slot);
    }
    out
}

#[test]
fn the_recovery_checkpoint_carries_the_received_values_even_when_saving_them_is_off() {
    let dir = Dir::new("recovery-received");
    let root = dir.0.join("recovery");
    let mut s = AppState::new_in(32, 32, Lang::Ja);
    s.recovery.enable(root.clone(), settings()).unwrap();
    // 保存の設定は切（.ylp には書かない）。復旧は開いていた時の見た目に戻すので、設定によらず書く
    s.prefs.settings.livelink_keep_values = false;
    s.doc
        .set_received_look(Some(received_from_unity()))
        .unwrap();
    s.apply(Action::Look(LookOp::Value {
        name: "_ShadowBorder",
        value: LookValue::Float(0.8),
        drag: false,
    }));
    assert!(s.modified);
    let from = Instant::now();
    s.recovery_tick_at(from);
    s.recovery_tick_at(from + Duration::from_secs(16));
    s.recovery_wait();
    // 書き置きの世代の look.json の received は、値と絵のあったスロットの名前だけ（画素は書かない）
    let store = yolu_io::GenerationStore::new(s.recovery.session_dir().unwrap());
    let mut files = store.load().unwrap().files;
    files.remove(yolu_io::INFO_NAME);
    let project = yolu_io::Project::from_entries(files).unwrap();
    let id = project.sets()[0].id.clone();
    let stored = project
        .received_look(&id)
        .unwrap()
        .expect("受けた値が書き置きにある");
    assert_eq!(stored, as_stored(&received_from_unity()));
    assert!(stored.images.is_empty());
    // 落ちた体で起動して、その世代を開くと受けた値も戻る（欄で変えた項目が勝つまま。Undo の段は積まない）
    assert!(s.recovery.is_idle());
    drop(s);
    let mut s2 = AppState::new_in(32, 32, Lang::Ja);
    s2.recovery.enable(root, settings()).unwrap();
    s2.recovery_apply(RecoveryAction::Open);
    assert!(s2.message.starts_with("復旧しました"), "{}", s2.message);
    let r = s2.doc.received_look().expect("受けた値が戻る");
    assert_eq!(r, &as_stored(&received_from_unity()));
    assert!(
        !r.missing.contains_key("_MatCapTex") && !r.images.contains_key("_MatCapTex"),
        "絵は持たず、理由も付けない（絵のファイルから読み直す）"
    );
    assert_eq!(s2.doc.drawn_look().float("_ShadowBorder", 0.5), 0.8);
    assert_eq!(s2.doc.drawn_look().kind, LookKind::LilToon);
    assert!(!s2.doc.can_undo());
}

fn cube() -> AppState {
    let mut s = AppState::new(64, 64);
    s.bake.backend = BakeBackend::Cpu;
    s.apply(Action::LoadDemoModel);
    s.bake.settings.maps = vec![
        MeshMapKind::WorldNormal,
        MeshMapKind::Position,
        MeshMapKind::AmbientOcclusion,
        MeshMapKind::Curvature,
    ];
    s.bake.settings.ao_samples = 8;
    s.bake.settings.padding = 4;
    s
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../yolu-io/tests/fixtures")
        .join(name)
}

#[test]
fn a_read_only_set_says_why_its_look_cannot_be_read() {
    // 最初のセットの正本を core で扱えない中身（反転の調整レイヤーが使わない値 gamma を既定から変えたもの）にして読むだけで開かせ、
    // そのセットに読めない look.json を持たせる
    let dir = Dir::new("readonly");
    let path = dir.0.join("rich.ylp");
    let base = yolu_io::Project::read(&std::fs::read(fixture("format4.ylp")).unwrap())
        .unwrap()
        .upgraded(yolu_app::project::writer())
        .unwrap();
    let rich = crate::common::core_refused::native_core_cannot_hold();
    let first = base.sets()[0].id.clone();
    std::fs::write(
        &path,
        base.with_document(&first, &rich)
            .unwrap()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    replace_look_entry(&path, br#"{"format": 7, "kind": "lilToon"}"#);
    let mut s = AppState::new(64, 64);
    s.apply(Action::OpenProject(path));
    let reason = s
        .sets
        .get(0)
        .unwrap()
        .read_only
        .clone()
        .expect("core で扱えない中身なので読むだけ");
    assert!(reason.contains("編集に対応していない中身"), "{reason}");
    assert!(
        reason.contains("見た目の設定を読めません"),
        "読むだけの理由にも: {reason}"
    );
    assert!(
        s.message.contains("見た目の設定を読めません"),
        "状態の帯にも（通常のセットと同じ）: {}",
        s.message
    );
    assert!(s.set_doc(0).look().is_default());
}

#[test]
fn unlocking_a_set_whose_inputs_arrive_says_why_its_look_cannot_be_read() {
    // Generator の入力（焼いたマップ）がそろわないので読むだけで開くセットに、読めない look.json を持たせる
    let dir = Dir::new("waiting");
    let path = dir.0.join("waiting.ylp");
    let mut s = cube();
    s.apply(Action::M2(Edit::NewFill));
    let layer = s.selected_layer.unwrap();
    s.apply(Action::M2(Edit::AddMask(layer)));
    s.apply(Action::Fx(FxOp::AddGenerator {
        target: FilterTarget::Mask,
        kind: yolu_core::generator::Kind::EdgeWear,
    }));
    s.apply(Action::Bake(BakeAction::Start));
    s.wait_bake();
    s.sync_effects();
    s.apply(Action::Look(LookOp::Kind(LookKind::LilToon)));
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    replace_look_entry(&path, br#"{"format": 7, "kind": "lilToon"}"#);

    let mut again = AppState::new(64, 64);
    again.bake.backend = BakeBackend::Cpu;
    again.apply(Action::OpenProject(path.clone()));
    assert!(again.read_only_reason().is_some(), "入力がそろわない");
    assert!(
        again.message.contains("見た目の設定を読めません"),
        "{}",
        again.message
    );
    // 入力がそろって編集できるようになるときも、同じ理由を言う（標準の見た目のまま）
    again.apply(Action::LoadDemoModel);
    again.sync_effect_inputs_with(true);
    assert!(
        again.read_only_reason().is_none(),
        "{:?} {}",
        again.read_only_reason(),
        again.message
    );
    assert!(again.message.contains("編集できます"), "{}", again.message);
    assert!(
        again.message.contains("見た目の設定を読めません"),
        "{}",
        again.message
    );
    assert!(again.doc.look().is_default());
}

#[test]
fn unlocking_a_set_whose_inputs_arrive_brings_back_the_received_values() {
    // Generator の入力（焼いたマップ）がそろわないので読むだけで開くセットに、Unity から受けた値を保存しておく
    let dir = Dir::new("waiting-received");
    let path = dir.0.join("waiting.ylp");
    let mut s = cube();
    s.apply(Action::M2(Edit::NewFill));
    let layer = s.selected_layer.unwrap();
    s.apply(Action::M2(Edit::AddMask(layer)));
    s.apply(Action::Fx(FxOp::AddGenerator {
        target: FilterTarget::Mask,
        kind: yolu_core::generator::Kind::EdgeWear,
    }));
    s.apply(Action::Bake(BakeAction::Start));
    s.wait_bake();
    s.sync_effects();
    s.doc
        .set_received_look(Some(received_from_unity()))
        .unwrap();
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);

    let mut again = AppState::new(64, 64);
    again.bake.backend = BakeBackend::Cpu;
    again.apply(Action::OpenProject(path));
    assert!(again.read_only_reason().is_some(), "入力がそろわない");
    // 入力がそろって編集できるようになると、受けた値も戻る（開いたときと同じ）
    again.apply(Action::LoadDemoModel);
    again.sync_effect_inputs_with(true);
    assert!(
        again.read_only_reason().is_none(),
        "{:?} {}",
        again.read_only_reason(),
        again.message
    );
    let r = again.doc.received_look().expect("受けた値が戻る");
    assert_eq!(r, &as_stored(&received_from_unity()));
    assert_eq!(again.doc.drawn_look().kind, LookKind::LilToon);
    assert_eq!(again.doc.drawn_look().float("_ShadowBorder", 0.5), 0.25);
    assert!(!again.doc.can_undo(), "戻すのは Undo の段にならない");
}

#[test]
fn a_template_that_cannot_make_its_channels_changes_nothing() {
    let mut s = AppState::new(16, 16);
    // ユーザーチャンネルの空きを 1 つだけ残す（チャンネルは 64 まで）
    let mut n = 0;
    while s.doc.channels().len() < 63 {
        s.doc
            .add_channel(ChannelInfo {
                name: format!("埋め {n}"),
                kind: ChannelKind::Scalar,
                color_space: ColorSpace::Linear,
                default: Rgba8::new(0, 0, 0, 255),
            })
            .unwrap();
        n += 1;
    }
    // 影とリムを入にしておくと、ひな形はマスクのチャンネルを 2 つ以上作ろうとする
    let mut look = s.doc.look().clone();
    look.kind = LookKind::LilToon;
    look.properties
        .insert("_UseShadow".into(), LookValue::Float(1.0));
    look.properties
        .insert("_UseRim".into(), LookValue::Float(1.0));
    s.doc.set_look(look.clone(), false).unwrap();
    let steps = s.doc.undo_count();
    let channels = s.doc.channels().len();
    s.apply(Action::Look(LookOp::Template));
    assert!(s.message.contains("64"), "{}", s.message);
    assert_eq!(
        s.doc.channels().len(),
        channels,
        "作りかけのチャンネルを残さない"
    );
    assert_eq!(s.doc.look(), &look, "割り当ても変えない");
    assert_eq!(s.doc.undo_count(), steps, "段を積まない");
}

#[test]
fn painting_a_slot_that_cannot_get_its_channel_changes_nothing() {
    // lilToon の文書と、標準の見た目の文書（描く口は lilToon にもする。失敗したら lilToon にもしない）
    for standard in [false, true] {
        let mut s = AppState::new(16, 16);
        if standard {
            s.doc
                .restore_look(yolu_core::look::MaterialLook::default())
                .unwrap();
        }
        // チャンネルの空きを無くす（チャンネルは 64 まで）
        let mut n = 0;
        while s.doc.channels().len() < 64 {
            s.doc
                .add_channel(ChannelInfo {
                    name: format!("埋め {n}"),
                    kind: ChannelKind::Scalar,
                    color_space: ColorSpace::Linear,
                    default: Rgba8::new(0, 0, 0, 255),
                })
                .unwrap();
            n += 1;
        }
        let look = s.doc.look().clone();
        let steps = s.doc.undo_count();
        let paint = s.m2.paint_channel;
        s.message.clear();
        s.apply(Action::Look(LookOp::PaintSlot("_RimShadeMask")));
        assert!(s.message.contains("64"), "{standard}: {}", s.message);
        assert_eq!(
            s.doc.channels().len(),
            64,
            "{standard}: 作りかけのチャンネルを残さない"
        );
        assert_eq!(
            s.doc.look(),
            &look,
            "{standard}: 割り当ても描き方も変えない"
        );
        assert_eq!(
            s.doc.drawn_look().kind,
            if standard {
                LookKind::Standard
            } else {
                LookKind::LilToon
            }
        );
        assert_eq!(s.doc.undo_count(), steps, "{standard}: 段を積まない");
        assert_eq!(
            s.m2.paint_channel, paint,
            "{standard}: 描くチャンネルも変えない"
        );
        assert!(!s.modified, "{standard}");
    }
    // 描けないスロット（マットキャップの絵はプロジェクトの画像）と知らない名前も、何も変えずに理由を言う
    let mut s = AppState::new(16, 16);
    let look = s.doc.look().clone();
    let channels = s.doc.channels().len();
    let paint = s.m2.paint_channel;
    for slot in ["_MatCapTex", "_NoSuchSlot"] {
        s.message.clear();
        s.apply(Action::Look(LookOp::PaintSlot(slot)));
        assert!(!s.message.is_empty(), "{slot}");
        assert_eq!(s.doc.look(), &look, "{slot}");
        assert_eq!(s.doc.channels().len(), channels, "{slot}");
        assert_eq!(s.doc.undo_count(), 0, "{slot}");
        assert_eq!(s.m2.paint_channel, paint, "{slot}");
    }
}

/// チャンネルの空きを無くす（チャンネルは 64 まで）。
fn fill_channels(s: &mut AppState) {
    let mut n = 0;
    while s.doc.channels().len() < 64 {
        s.doc
            .add_channel(ChannelInfo {
                name: format!("埋め {n}"),
                kind: ChannelKind::Scalar,
                color_space: ColorSpace::Linear,
                default: Rgba8::new(0, 0, 0, 255),
            })
            .unwrap();
        n += 1;
    }
}

/// ひな形のあと、`_ShadowStrengthMask` を成分ごとの詰め合わせ（R はひな形のチャンネル、G・A は 0、B は 1）にする。詰め合わせの元を返す。
fn pack_shadow_strength(s: &mut AppState) -> [PlaneSource; 4] {
    s.apply(Action::Look(LookOp::Template));
    let Some(TextureSource::Channel(first)) =
        s.doc.look().textures.get("_ShadowStrengthMask").copied()
    else {
        panic!("ひな形が割り当てる: {}", s.message);
    };
    let planes = [
        PlaneSource::Channel {
            channel: first,
            component: 0,
        },
        PlaneSource::Zero,
        PlaneSource::One,
        PlaneSource::Zero,
    ];
    s.apply(Action::Look(LookOp::Texture {
        slot: "_ShadowStrengthMask",
        source: Some(TextureSource::Packed(planes)),
    }));
    assert_eq!(
        s.doc.look().textures["_ShadowStrengthMask"],
        TextureSource::Packed(planes),
        "{}",
        s.message
    );
    planes
}

#[test]
fn a_new_slot_channel_that_cannot_get_its_channel_changes_nothing() {
    // スロット全体の新しいチャンネル（lilToon の文書と標準の見た目の文書。失敗したら lilToon にもしない）と、成分ごとの新しいチャンネル
    for (standard, plane) in [(false, None), (true, None), (false, Some(1u8))] {
        let mut s = AppState::new(16, 16);
        if plane.is_some() {
            pack_shadow_strength(&mut s);
        }
        if standard {
            s.doc
                .restore_look(yolu_core::look::MaterialLook::default())
                .unwrap();
        }
        fill_channels(&mut s);
        let what = format!("標準 {standard}・成分 {plane:?}");
        let look = s.doc.look().clone();
        let drawn = s.doc.drawn_look().kind;
        s.doc.clear_history().unwrap();
        s.modified = false;
        let steps = s.doc.undo_count();
        let paint = s.m2.paint_channel;
        s.message.clear();
        s.apply(Action::Look(LookOp::NewChannel {
            slot: if plane.is_some() {
                "_ShadowStrengthMask"
            } else {
                "_RimShadeMask"
            },
            plane,
        }));
        assert!(s.message.contains("64"), "{what}: {}", s.message);
        assert_eq!(
            s.doc.channels().len(),
            64,
            "{what}: 作りかけのチャンネルを残さない"
        );
        assert_eq!(s.doc.look(), &look, "{what}: 割り当てを変えない");
        assert_eq!(s.doc.drawn_look().kind, drawn, "{what}: 描き方も変えない");
        assert_eq!(s.doc.undo_count(), steps, "{what}: 段を積まない");
        assert_eq!(
            s.m2.paint_channel, paint,
            "{what}: 描くチャンネルも変えない"
        );
        assert!(!s.modified, "{what}");
    }
}

#[test]
fn new_slot_channels_survive_save_and_open_with_their_assignments_and_pixels() {
    let dir = Dir::new("new-channel");
    let path = dir.0.join("new-channel.ylp");
    let mut s = AppState::new(32, 32);
    let planes = pack_shadow_strength(&mut s);
    // 成分ごとの新しいチャンネル（G）と、スロット全体の新しいスカラーのチャンネル
    s.apply(Action::Look(LookOp::NewChannel {
        slot: "_ShadowStrengthMask",
        plane: Some(1),
    }));
    let TextureSource::Packed(now) = s.doc.look().textures["_ShadowStrengthMask"] else {
        panic!("成分ごとのまま: {}", s.message);
    };
    let PlaneSource::Channel {
        channel: plane_made,
        component: 0,
    } = now[1]
    else {
        panic!("G にチャンネルを割り当てる: {now:?}");
    };
    assert_eq!(
        (now[0], now[2], now[3]),
        (planes[0], planes[2], planes[3]),
        "ほかの成分はそのまま"
    );
    s.apply(Action::Look(LookOp::NewChannel {
        slot: "_RimShadeMask",
        plane: None,
    }));
    let TextureSource::Channel(slot_made) = s.doc.look().textures["_RimShadeMask"] else {
        panic!("チャンネルを割り当てる: {}", s.message);
    };
    assert_ne!(plane_made, slot_made);
    // 作ったチャンネルへ描いた画素
    let layer = s.doc.layers()[0].id();
    s.doc
        .set_channel_pixel(layer, plane_made, 3, 4, Rgba8::new(200, 200, 200, 255))
        .unwrap();
    s.doc
        .set_channel_pixel(layer, slot_made, 5, 6, Rgba8::new(90, 90, 90, 255))
        .unwrap();
    let saved = s.doc.look().clone();
    let infos = |d: &yolu_app::engine::Document| {
        d.channels()
            .iter()
            .map(|c| {
                let i = d.channel_info(*c).unwrap();
                (i.name.clone(), i.kind, i.color_space, i.default)
            })
            .collect::<Vec<_>>()
    };
    let before = infos(&s.doc);
    yolu_app::project::save_from(&mut s, &path);
    assert!(
        s.message.contains("保存しました") || s.message.contains("Saved"),
        "{}",
        s.message
    );
    let mut t = AppState::new(8, 8);
    yolu_app::project::open_into(&mut t, &path);
    assert_eq!(t.doc.look(), &saved, "{}", t.message);
    assert_eq!(
        infos(&t.doc),
        before,
        "チャンネルの名前・種類・色空間・既定"
    );
    let layer = t.doc.layers()[0].id();
    assert_eq!(
        t.doc.layer(layer).unwrap().pixel(plane_made, 3, 4).unwrap(),
        Rgba8::new(200, 200, 200, 255)
    );
    assert_eq!(
        t.doc.layer(layer).unwrap().pixel(slot_made, 5, 6).unwrap(),
        Rgba8::new(90, 90, 90, 255)
    );
    assert_eq!(t.doc.undo_count(), 0, "開いただけで Undo の段は増えない");
}

#[test]
fn look_changes_are_refused_while_drawing() {
    let mut s = AppState::new(32, 32);
    // 新しいセットの既定は lilToon。描いている間は、どの見た目の操作も設定を変えない
    let before = s.doc.look().clone();
    assert_eq!(before.kind, LookKind::LilToon);
    let layer = s.selected_layer.unwrap();
    let brush = s.stroke_settings(false);
    let mut stroke = s.doc.begin_stroke(layer, &brush).unwrap();
    stroke
        .add_point(&mut s.doc, 10.0, 10.0, 1.0, DVec2::ZERO)
        .unwrap();
    for op in [
        LookOp::Kind(LookKind::Standard),
        LookOp::PaintSlot("_RimShadeMask"),
        LookOp::NewChannel {
            slot: "_RimShadeMask",
            plane: None,
        },
        LookOp::NewChannel {
            slot: "_ShadowStrengthMask",
            plane: Some(0),
        },
        LookOp::Template,
        LookOp::Value {
            name: "_ShadowBorder",
            value: LookValue::Float(0.2),
            drag: true,
        },
    ] {
        s.message.clear();
        let what = format!("{op:?}");
        s.apply(Action::Look(op));
        assert_eq!(s.message, "描いている間はできません。", "{what}");
        assert_eq!(s.doc.look(), &before, "{what}");
    }
    assert_eq!(
        s.doc.channels().len(),
        yolu_core::Channel::STANDARD_COUNT,
        "描く口もチャンネルを作らない"
    );
    // core も断る（画面の外からの変更も同じ）
    let mut look = s.doc.look().clone();
    look.kind = LookKind::Standard;
    assert!(s.doc.set_look(look, false).is_err());
    s.doc.end_stroke(stroke).unwrap();
    s.apply(Action::Look(LookOp::Kind(LookKind::Standard)));
    assert_eq!(s.doc.look().kind, LookKind::Standard);
}
