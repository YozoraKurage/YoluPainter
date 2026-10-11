//! ウィンドウに落とした .psd（egui_kittest）: 「ファイル → インポート → PSD を新しいテクスチャセットに」と同じ取り込みの流れ（読み込み →
//! 落とす・変わるものがあれば取り込みの確認のウィンドウ → 取り込む）に入る、描いている最中は断る、複数を落としたときは最初の 1 つだけ（理由を出す）、
//! ブラシのファイル・PNG の決まり（ブラシのファイルはブラシとして取り込む・PNG はブラシの一覧の上だけ）と .ylp が前と変わらない。
//! 試験の PSD・ブラシのファイルは試験の中で組む。
#[path = "../brush_import_files/mod.rs"]
mod brush_import_files;
use crate::common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use brush_import_files::*;
use common::*;
use egui::pos2;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::brushes::Group;
use yolu_app::lang::Lang;
use yolu_app::state::AppState;
use yolu_app::YoluApp;

type H = Harness<'static, YoluApp>;

fn st(h: &H) -> &AppState {
    &h.state().state
}

fn temp_dir(tag: &str) -> PathBuf {
    common::tmp::test_dir(&format!("psd-drop-{tag}"))
}

/// 2 レイヤーの PSD を書く。`dissolve` のとき、下のレイヤーの合成モードを取り込めない「ディゾルブ」にする（取り込みでは通常になる＝変わる。
/// 取り込みの確認のウィンドウが出る）。
fn psd_file(dir: &Path, name: &str, dissolve: bool) -> PathBuf {
    use yolu_io::psd::{self, Document, Layer, Limits};
    let layer = |id: i32, name: &str, rgba: [u8; 4]| Layer {
        id,
        name: name.into(),
        width: 8,
        height: 8,
        pixels_rgba: rgba.repeat(64),
        ..Layer::default()
    };
    let doc = Document {
        width: 8,
        height: 8,
        layers: vec![
            layer(2, "上", [0, 0, 255, 255]),
            layer(1, "下", [255, 0, 0, 255]),
        ],
        composite_rgba: None,
    };
    let mut bytes = psd::write(&doc, &Limits::default()).unwrap();
    if dissolve {
        let i = bytes
            .windows(8)
            .position(|w| w == b"8BIMnorm")
            .expect("レイヤーの記録の合成モード");
        bytes[i + 4..i + 8].copy_from_slice(b"diss");
    }
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

/// ウィンドウの外（ブラシの一覧の外）にポインタを置いて、ファイルを落とす（落とした 1 フレームだけ進める）。
fn drop_files(h: &mut H, files: &[&Path]) {
    move_to(h, pos2(900.0, 500.0));
    h.step();
    for f in files {
        h.input_mut()
            .dropped_files
            .push(Arc::new(Dropped(f.to_path_buf())));
    }
    h.step();
}

/// PSD の仕事（別のスレッド）が終わるまでフレームを回す。
fn settle(h: &mut H) {
    let start = Instant::now();
    while st(h).psd.is_busy() {
        h.step();
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "PSD の仕事が終わらない（ハング検出上限）"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    h.run();
}

fn wait_brush_import(h: &mut H) {
    let start = Instant::now();
    while st(h).is_brush_importing() && start.elapsed() < Duration::from_secs(60) {
        h.run();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!st(h).is_brush_importing(), "ブラシの取り込みが終わらない");
    h.run();
}

#[test]
fn a_dropped_psd_goes_through_the_import_check_and_becomes_a_new_texture_set() {
    let dir = temp_dir("flow");
    let lossy = psd_file(&dir, "Ids.psd", true);
    let mut h = app(1280.0, 800.0, 32);
    assert_eq!(st(&h).sets.len(), 1);
    drop_files(&mut h, &[&lossy]);
    assert!(
        st(&h).message.contains("Ids.psd"),
        "読む PSD の名前: {}",
        st(&h).message
    );
    settle(&mut h);
    // 変わるものがあるので、取り込みの確認のウィンドウが出て、取り込むまで何も入れない
    let check = st(&h).psd.import_check.as_ref().expect("取り込みの確かめ");
    assert_eq!(check.file(), "Ids.psd");
    assert!(yolu_app::windows::window_rect(&h.ctx, "psd-import").is_some());
    assert_eq!(st(&h).sets.len(), 1, "確かめるまで入れない");
    // 行き先は新しいセット: 今のセットの文書は替わらず、セットが 1 つ増える
    let first_doc = st(&h).doc.id();
    h.get_by_label("取り込む").click();
    h.run();
    assert!(st(&h).psd.import_check.is_none());
    assert_eq!(st(&h).sets.len(), 2);
    assert_eq!(st(&h).sets.current().name, "Ids");
    assert_ne!(st(&h).doc.id(), first_doc, "新しいセットの文書が前に出る");
    assert!(
        st(&h).sets.iter().any(|s| s.name != "Ids"),
        "前のセットは残る"
    );
    // やめる: 何も入れない
    let again = psd_file(&dir, "Again.psd", true);
    drop_files(&mut h, &[&again]);
    settle(&mut h);
    assert!(st(&h).psd.import_check.is_some());
    h.get_by_label("やめる").click();
    h.run();
    assert!(st(&h).psd.import_check.is_none());
    assert_eq!(st(&h).sets.len(), 2);
    // 確かめの要らない PSD（落とす・変わるものが無い）は、拡張子が大文字でも、ウィンドウを出さずにそのまま入る
    let plain = psd_file(&dir, "Plain.PSD", false);
    drop_files(&mut h, &[&plain]);
    settle(&mut h);
    assert!(
        st(&h).psd.import_check.is_none(),
        "確認のウィンドウは出ない"
    );
    assert_eq!(st(&h).sets.len(), 3);
    assert_eq!(st(&h).sets.current().name, "Plain");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn several_dropped_psds_import_only_the_first_and_say_why() {
    for lang in Lang::ALL {
        let dir = temp_dir(&format!("several-{lang:?}"));
        let first = psd_file(&dir, "a.psd", false);
        let second = psd_file(&dir, "b.psd", false);
        let third = psd_file(&dir, "c.psd", false);
        let mut h = app(1280.0, 800.0, 32);
        h.state_mut().state.lang = lang;
        h.run();
        drop_files(&mut h, &[&first, &second, &third]);
        settle(&mut h);
        // 取り込みの仕事が終わった文に、取り込まなかった件数と理由が足される
        let message = st(&h).message.clone();
        assert!(
            message.contains("a.psd"),
            "{lang:?}: 取り込んだファイルの名前: {message}"
        );
        assert!(
            message.contains(lang.pick("ほか 2 件は取り込みません", "2 more not imported")),
            "{lang:?}: 理由と件数: {message}"
        );
        assert_eq!(
            st(&h).message_kind(),
            yolu_app::notice::Kind::Warning,
            "{lang:?}: 取り込めたのに断り・失敗として出さない（一部を取り込まなかった注意）: {message}"
        );
        assert_eq!(st(&h).sets.len(), 2, "{lang:?}: 1 つだけ入る");
        assert_eq!(st(&h).sets.current().name, "a");
        assert!(
            !st(&h).sets.iter().any(|s| s.name == "b" || s.name == "c"),
            "{lang:?}"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn a_psd_dropped_while_stroking_is_refused_and_nothing_starts() {
    let dir = temp_dir("stroking");
    let psd = psd_file(&dir, "a.psd", false);
    let mut h = app(1280.0, 800.0, 32);
    let stroke = {
        let app = &mut h.state_mut().state;
        let layer = app.selected_layer.unwrap();
        let settings = app.stroke_settings(false);
        app.doc.begin_stroke(layer, &settings).unwrap()
    };
    h.state_mut().state.stroke = Some(stroke);
    h.run();
    assert!(st(&h).is_stroking());
    drop_files(&mut h, &[&psd]);
    assert!(!st(&h).psd.is_busy(), "読み始めない");
    assert!(st(&h).psd.import_check.is_none());
    assert_eq!(st(&h).sets.len(), 1);
    assert!(
        st(&h).message.contains("描いている間"),
        "{}",
        st(&h).message
    );
    // 終わると、落とした PSD は取り込める
    let stroke = h.state_mut().state.stroke.take().unwrap();
    h.state_mut().state.doc.cancel_stroke(stroke);
    h.run();
    drop_files(&mut h, &[&psd]);
    settle(&mut h);
    assert_eq!(st(&h).sets.len(), 2);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn brush_files_and_pngs_dropped_with_a_psd_follow_the_same_rules_as_before() {
    let dir = temp_dir("brushes");
    let mut h = app(1600.0, 960.0, 32);
    h.state_mut().state.attach_brush_store(dir.join("brushes"));
    h.run();
    let png = dir.join("tip.png");
    std::fs::write(&png, b"not even a png").unwrap();
    let gbr_path = dir.join("chalk.gbr");
    std::fs::write(&gbr_path, gbr_gray("Chalk")).unwrap();
    let psd = psd_file(&dir, "a.psd", false);
    // PNG は一覧の外ではブラシにしない（PSD だけが動く）
    let before = st(&h).brush_entries_in(Group::Imported).len();
    drop_files(&mut h, &[&psd, &png]);
    settle(&mut h);
    wait_brush_import(&mut h);
    assert_eq!(st(&h).sets.len(), 2, "PSD は読む");
    assert_eq!(
        st(&h).brush_entries_in(Group::Imported).len(),
        before,
        "一覧の外の PNG はブラシにしない"
    );
    assert!(!st(&h).message.contains("tip.png"), "{}", st(&h).message);
    // ブラシのファイルはブラシとして取り込み、PSD は PSD として読む（どちらも落とした場所によらない）
    let again = psd_file(&dir, "b.psd", false);
    drop_files(&mut h, &[&again, &gbr_path]);
    settle(&mut h);
    wait_brush_import(&mut h);
    assert_eq!(st(&h).sets.len(), 3, "PSD も読む");
    assert_eq!(st(&h).brush_entries_in(Group::Imported).len(), before + 1);
    // PSD の入らない落とし方（ブラシのファイルだけ）は PSD の取り込みを起こさない
    let sets = st(&h).sets.len();
    let imported = st(&h).brush_entries_in(Group::Imported).len();
    let abr = dir.join("old.abr");
    std::fs::write(&abr, abr_v1()).unwrap();
    drop_files(&mut h, &[&abr]);
    assert!(!st(&h).psd.is_busy());
    wait_brush_import(&mut h);
    assert_eq!(
        st(&h).brush_entries_in(Group::Imported).len(),
        imported + 2,
        "ABR は取り込む（PSD が付かなくても）"
    );
    assert_eq!(st(&h).sets.len(), sets);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_ylp_dropped_with_a_psd_is_still_opened_instead_and_the_psd_is_left_alone() {
    let dir = temp_dir("ylp");
    let psd = psd_file(&dir, "a.psd", false);
    let missing = dir.join("none.ylp");
    let mut h = app(1280.0, 800.0, 32);
    drop_files(&mut h, &[&psd, &missing]);
    // .ylp が先（開けなくても、PSD は取り込まない）
    assert!(!st(&h).psd.is_busy());
    assert_eq!(st(&h).sets.len(), 1);
    assert!(st(&h).message.contains("開けません"), "{}", st(&h).message);
    std::fs::remove_dir_all(dir).unwrap();
}
