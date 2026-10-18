//! ウィンドウに落とした .psd（egui_kittest）: ファイルの取り込みのウィンドウの行になる（「ファイル → インポート → PSD を新しいテクスチャセットに」の
//! 取り込みは始めない）、描いている最中は断る、複数を落としたときは全部が行になる、ブラシのファイル・PNG の決まり（ブラシのファイルはブラシとして
//! 取り込む・PNG はブラシの一覧の上だけブラシ、ほかではウィンドウの行）と .ylp が前と変わらない。入れ先と取り込みは `dropin.rs`。
//! 試験の PSD・ブラシのファイルは試験の中で組む。
#[path = "../brush_import_files/mod.rs"]
mod brush_import_files;
use crate::common;
use crate::common::drive::Drive;

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
    h.app()
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

fn wait_brush_import(h: &mut H) {
    let start = Instant::now();
    while st(h).is_brush_importing() && start.elapsed() < Duration::from_secs(60) {
        h.run();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!st(h).is_brush_importing(), "ブラシの取り込みが終わらない");
    h.run();
}

/// 落とした PSD の読み込みが終わるまでフレームを回す。
fn read_dropped(h: &mut H) {
    h.app_mut().wait_dropin();
    h.run();
}

fn row_names(h: &H) -> Vec<String> {
    h.dropin().rows.iter().map(|r| r.name.clone()).collect()
}

#[test]
fn a_dropped_psd_becomes_a_row_of_the_import_files_window_and_starts_no_texture_set_import() {
    let dir = temp_dir("flow");
    let lossy = psd_file(&dir, "Ids.psd", true);
    let mut h = app(1280.0, 800.0, 32);
    assert_eq!(st(&h).sets().len(), 1);
    drop_files(&mut h, &[&lossy]);
    read_dropped(&mut h);
    // 新しいテクスチャセットの取り込み（と、その取り込みの確かめのウィンドウ）は始めない。入れ先を選ぶまで何も入れない
    assert!(!st(&h).psd().is_busy());
    assert!(st(&h).psd().import_check.is_none());
    assert!(yolu_app::windows::window_rect(&h.ctx, "psd-import").is_none());
    assert!(yolu_app::windows::window_rect(&h.ctx, "import-files").is_some());
    assert_eq!(row_names(&h), ["Ids.psd"]);
    assert!(h.dropin().rows[0].is_readable(), "読めた PSD の行");
    assert_eq!(st(&h).sets().len(), 1);
    // やめる: 何も入れない
    h.get_by_label("やめる").click();
    h.run();
    assert!(!h.dropin().is_open());
    assert_eq!(st(&h).sets().len(), 1);
    // 拡張子が大文字でも行になる
    let plain = psd_file(&dir, "Plain.PSD", false);
    drop_files(&mut h, &[&plain]);
    read_dropped(&mut h);
    assert_eq!(row_names(&h), ["Plain.PSD"]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn several_dropped_psds_all_become_rows_of_one_window() {
    for lang in Lang::ALL {
        let dir = temp_dir(&format!("several-{lang:?}"));
        let first = psd_file(&dir, "a.psd", false);
        let second = psd_file(&dir, "b.psd", false);
        let third = psd_file(&dir, "c.psd", false);
        let mut h = app(1280.0, 800.0, 32);
        h.set_lang(lang);
        h.run();
        drop_files(&mut h, &[&first, &second, &third]);
        read_dropped(&mut h);
        assert_eq!(row_names(&h), ["a.psd", "b.psd", "c.psd"], "{lang:?}");
        assert!(h.dropin().rows.iter().all(|r| r.is_readable()), "{lang:?}");
        assert_eq!(st(&h).sets().len(), 1, "{lang:?}: 落としただけでは入れない");
        // 開いているウィンドウへ落とした分は、同じウィンドウの行に足す（同じファイルは 1 行）
        let fourth = psd_file(&dir, "d.psd", false);
        drop_files(&mut h, &[&fourth, &first]);
        read_dropped(&mut h);
        assert_eq!(
            row_names(&h),
            ["a.psd", "b.psd", "c.psd", "d.psd"],
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
        let app = h.app_mut();
        let layer = app.selected_layer().unwrap();
        let settings = app.stroke_settings(false);
        app.doc_mut().begin_stroke(layer, &settings).unwrap()
    };
    *h.stroke_mut() = Some(stroke);
    h.run();
    assert!(st(&h).is_stroking());
    drop_files(&mut h, &[&psd]);
    assert!(!st(&h).psd().is_busy(), "読み始めない");
    assert!(st(&h).psd().import_check.is_none());
    assert_eq!(st(&h).sets().len(), 1);
    assert!(
        st(&h).message().contains("描いている間"),
        "{}",
        st(&h).message()
    );
    assert!(!h.dropin().is_open(), "ウィンドウも出さない");
    // 終わると、落とした PSD はウィンドウの行になる
    let stroke = h.stroke_mut().take().unwrap();
    h.doc_mut().cancel_stroke(stroke);
    h.run();
    drop_files(&mut h, &[&psd]);
    read_dropped(&mut h);
    assert_eq!(row_names(&h), ["a.psd"]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn brush_files_and_pngs_dropped_with_a_psd_follow_the_same_rules_as_before() {
    let dir = temp_dir("brushes");
    let mut h = app(1600.0, 960.0, 32);
    h.app_mut().attach_brush_store(dir.join("brushes"));
    h.run();
    let png = dir.join("tip.png");
    std::fs::write(&png, b"not even a png").unwrap();
    let gbr_path = dir.join("chalk.gbr");
    std::fs::write(&gbr_path, gbr_gray("Chalk")).unwrap();
    let psd = psd_file(&dir, "a.psd", false);
    // PNG は一覧の外ではブラシにしない（PSD と一緒に、ファイルの取り込みのウィンドウの行になる。読めない PNG は理由つきの行）
    let before = st(&h).brush_entries_in(Group::Imported).len();
    drop_files(&mut h, &[&psd, &png]);
    read_dropped(&mut h);
    wait_brush_import(&mut h);
    assert_eq!(row_names(&h), ["a.psd", "tip.png"]);
    assert!(h.dropin().rows[1].is_refused(), "壊れた PNG は読めない行");
    assert_eq!(
        st(&h).brush_entries_in(Group::Imported).len(),
        before,
        "一覧の外の PNG はブラシにしない"
    );
    h.get_by_label("やめる").click();
    h.run();
    // ブラシのファイルはブラシとして取り込み、PSD はウィンドウの行（どちらも落とした場所によらない）
    let again = psd_file(&dir, "b.psd", false);
    drop_files(&mut h, &[&again, &gbr_path]);
    read_dropped(&mut h);
    wait_brush_import(&mut h);
    assert_eq!(row_names(&h), ["b.psd"], "ブラシのファイルは行にしない");
    assert_eq!(st(&h).brush_entries_in(Group::Imported).len(), before + 1);
    h.get_by_label("やめる").click();
    h.run();
    // PSD の入らない落とし方（ブラシのファイルだけ）はウィンドウを出さない
    let sets = st(&h).sets().len();
    let imported = st(&h).brush_entries_in(Group::Imported).len();
    let abr = dir.join("old.abr");
    std::fs::write(&abr, abr_v1()).unwrap();
    drop_files(&mut h, &[&abr]);
    assert!(!h.dropin().is_open());
    wait_brush_import(&mut h);
    assert_eq!(
        st(&h).brush_entries_in(Group::Imported).len(),
        imported + 2,
        "ABR は取り込む（PSD が付かなくても）"
    );
    assert_eq!(st(&h).sets().len(), sets);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_ylp_dropped_with_a_psd_is_still_opened_instead_and_the_psd_is_left_alone() {
    let dir = temp_dir("ylp");
    let psd = psd_file(&dir, "a.psd", false);
    let missing = dir.join("none.ylp");
    let mut h = app(1280.0, 800.0, 32);
    drop_files(&mut h, &[&psd, &missing]);
    // .ylp が先（開けなくても、PSD は取り込まない・ウィンドウの行にもしない）
    assert!(!st(&h).psd().is_busy());
    assert!(!h.dropin().is_open());
    assert_eq!(st(&h).sets().len(), 1);
    assert!(
        st(&h).message().contains("開けません"),
        "{}",
        st(&h).message()
    );
    std::fs::remove_dir_all(dir).unwrap();
}
