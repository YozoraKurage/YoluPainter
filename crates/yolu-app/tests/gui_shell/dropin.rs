//! ファイルの取り込みのウィンドウ（egui_kittest）: 落とした PSD と画像が 1 つのウィンドウの行になる、入れ先が全部決まるまで取り込めない、
//! 「すべて」の当て方、画像・PSD を入れ先（アセット・新しいレイヤーセット・既存のレイヤーセット）へ入れる、大きさの違う PSD の断り、
//! 入れ先の文書ごとに 1 回の Undo、描いている最中の断り、テクスチャセットが 2 つのプロジェクトへの通しの取り込み、ウィンドウのスクリーンショット。
//! テストの PSD・画像はテストの中で作る。
use crate::common;
use crate::common::drive::{Drive, DriveApp};

use std::path::{Path, PathBuf};

use common::*;
use egui::pos2;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::dropin::{Content, Dest, DropOp, MenuFor};
use yolu_app::lang::Lang;
use yolu_app::state::Action;
use yolu_app::YoluApp;

type H = Harness<'static, YoluApp>;

fn temp_dir(tag: &str) -> PathBuf {
    common::tmp::test_dir(&format!("dropin-{tag}"))
}

/// `side` × `side` の 2 レイヤーの PSD を書く。`dissolve` のとき、下のレイヤーの合成モードを取り込めない「ディゾルブ」にする（取り込みでは
/// 通常になる＝変わる。取り込みの確かめに並ぶ）。
fn psd_file(dir: &Path, name: &str, side: u32, dissolve: bool) -> PathBuf {
    use yolu_io::psd::{self, Document, Layer, Limits};
    let n = (side * side) as usize;
    let layer = |id: i32, name: &str, rgba: [u8; 4]| Layer {
        id,
        name: name.into(),
        width: side,
        height: side,
        pixels_rgba: rgba.repeat(n),
        ..Layer::default()
    };
    let doc = Document {
        width: side,
        height: side,
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

/// 16 × 8 の画像（形式は拡張子から）。
fn image_file(dir: &Path, name: &str, rgb: [u8; 3]) -> PathBuf {
    let path = dir.join(name);
    let img = image::RgbImage::from_pixel(16, 8, image::Rgb(rgb));
    image::DynamicImage::ImageRgb8(img).save(&path).unwrap();
    path
}

/// ウィンドウの外（ブラシの一覧・ライブラリの外）にポインタを置いて落とし、読み終わるまで待つ。
fn drop_and_read(h: &mut H, files: &[&Path]) {
    h.drop_files_at(pos2(900.0, 500.0), files);
    h.app_mut().wait_dropin();
    h.run();
}

fn row_id(h: &H, name: &str) -> u64 {
    h.dropin()
        .rows
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("{name} の行が無い"))
        .id
}

fn set_dest(h: &mut H, name: &str, dest: Dest) {
    let id = row_id(h, name);
    h.act(Action::DropIn(DropOp::SetDest(id, dest)));
    h.run();
}

fn import_disabled(h: &H) -> bool {
    h.get_by_label("取り込む").accesskit_node().is_disabled()
}

fn current_uid(h: &H) -> u32 {
    h.sets().current().uid
}

fn first_layer_set(h: &H) -> yolu_app::engine::LayerSetId {
    h.doc().layer_sets()[0].id
}

#[test]
fn psds_and_images_dropped_together_make_one_window_that_waits_for_every_destination() {
    let dir = temp_dir("rows");
    let a = psd_file(&dir, "a.psd", 32, false);
    let b = psd_file(&dir, "b.psd", 32, true);
    let png = image_file(&dir, "c.png", [10, 200, 30]);
    let jpg = image_file(&dir, "d.jpg", [200, 10, 30]);
    let mut h = app(1280.0, 800.0, 32);
    drop_and_read(&mut h, &[&a, &b, &png, &jpg]);
    assert!(yolu_app::windows::window_rect(&h.ctx, "import-files").is_some());
    let names: Vec<&str> = h.dropin().rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        ["a.psd", "b.psd", "c.png", "d.jpg"],
        "1 つのウィンドウに 4 行"
    );
    assert!(h.dropin().rows.iter().all(|r| r.is_readable()));
    assert!(
        h.dropin().rows.iter().all(|r| r.dest.is_none()),
        "既定は何も選んでいない"
    );
    assert_eq!(h.sets().len(), 1, "落としただけでは何も入れない");
    // 入れ先が全部決まるまで取り込めない
    assert!(import_disabled(&h));
    let uid = current_uid(&h);
    let set = first_layer_set(&h);
    set_dest(&mut h, "a.psd", Dest::Set(uid, set));
    set_dest(&mut h, "b.psd", Dest::NewSet(uid));
    set_dest(&mut h, "c.png", Dest::Asset);
    assert!(import_disabled(&h), "1 行がまだ");
    // 「すべて」でアセットを選んでも、PSD の行には当てない（残った画像の行にだけ当たる）
    h.act(Action::DropIn(DropOp::SetAll(Dest::Asset)));
    h.run();
    let dest = |h: &H, name: &str| h.dropin().row(row_id(h, name)).unwrap().dest;
    assert_eq!(dest(&h, "a.psd"), Some(Dest::Set(uid, set)));
    assert_eq!(dest(&h, "b.psd"), Some(Dest::NewSet(uid)));
    assert_eq!(dest(&h, "d.jpg"), Some(Dest::Asset));
    assert!(!import_disabled(&h));
    // 一覧から外すと、その行は入れない。全部外すとウィンドウを閉じる
    for name in ["a.psd", "b.psd", "c.png", "d.jpg"] {
        let id = row_id(&h, name);
        h.act(Action::DropIn(DropOp::Remove(id)));
    }
    h.run();
    assert!(!h.dropin().is_open());
    assert_eq!(h.sets().len(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn files_that_cannot_be_read_are_listed_with_a_reason_and_take_no_destination() {
    let dir = temp_dir("refused");
    let webp = dir.join("e.webp");
    std::fs::write(&webp, b"RIFF....WEBP").unwrap();
    let broken = dir.join("f.png");
    std::fs::write(&broken, b"not even a png").unwrap();
    let text = dir.join("notes.txt");
    std::fs::write(&text, b"hello").unwrap();
    let dds = dir.join("h.dds");
    std::fs::write(&dds, b"DDS ....").unwrap();
    let png = image_file(&dir, "g.png", [1, 2, 3]);
    let mut h = app(1280.0, 800.0, 32);
    drop_and_read(&mut h, &[&webp, &broken, &text, &dds, &png]);
    assert_eq!(h.dropin().rows.len(), 5);
    // 読めない形式（DDS）は形式の名前と「未対応の形式」、壊れた WebP・PNG は読もうとして断られた理由
    let Content::Refused { format, reason } = &h.dropin().row(row_id(&h, "h.dds")).unwrap().content
    else {
        panic!("h.dds は読めない行");
    };
    assert_eq!(format, "DDS");
    assert_eq!(reason, "未対応の形式");
    let Content::Refused { format, reason } =
        &h.dropin().row(row_id(&h, "e.webp")).unwrap().content
    else {
        panic!("壊れた e.webp は読めない行");
    };
    assert_eq!(format, "WebP");
    assert_eq!(reason, "画像として読めません");
    for name in ["e.webp", "f.png", "notes.txt", "h.dds"] {
        let row = h.dropin().row(row_id(&h, name)).unwrap();
        let Content::Refused { reason, .. } = &row.content else {
            panic!("{name} は読めない行");
        };
        assert!(!reason.is_empty(), "{name}: 短い理由");
        let id = row.id;
        h.act(Action::DropIn(DropOp::SetDest(id, Dest::Asset)));
        assert_eq!(
            h.dropin().row(id).unwrap().dest,
            None,
            "{name}: 入れ先を選べない"
        );
    }
    // 読める行の入れ先が決まれば取り込める（読めない行は数えない）
    set_dest(&mut h, "g.png", Dest::Asset);
    assert!(!import_disabled(&h));
    // テキストだけを落としてもウィンドウは出ない（今のまま）
    h.act(Action::DropIn(DropOp::Cancel));
    h.run();
    h.drop_files_at(pos2(900.0, 500.0), &[&text]);
    assert!(!h.dropin().is_open());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn images_go_to_the_assets_a_new_layer_set_or_the_top_of_a_layer_set() {
    let dir = temp_dir("images");
    let only = image_file(&dir, "only.png", [10, 20, 30]);
    let fresh = image_file(&dir, "fresh.tga", [40, 50, 60]);
    let top = image_file(&dir, "top.jpg", [70, 80, 90]);
    let mut h = app(1280.0, 800.0, 32);
    let uid = current_uid(&h);
    let set = first_layer_set(&h);
    let layers = h.doc().layers().len();
    let assets = h.shelf().resources().len();
    drop_and_read(&mut h, &[&only, &fresh, &top]);
    set_dest(&mut h, "only.png", Dest::Asset);
    set_dest(&mut h, "fresh.tga", Dest::NewSet(uid));
    set_dest(&mut h, "top.jpg", Dest::Set(uid, set));
    h.get_by_label("取り込む").click();
    h.run();
    assert!(!h.dropin().is_open(), "{}", h.message());
    // 3 つともアセットに入る（PNG 以外も）
    assert_eq!(h.shelf().resources().len(), assets + 3, "{}", h.message());
    let doc = h.doc();
    assert_eq!(
        doc.layers().len(),
        layers + 2,
        "アセットだけの画像はレイヤーにしない"
    );
    // 新しいレイヤーセット（ファイルの名前）と、その中の画像の塗りつぶしレイヤー
    assert_eq!(doc.layer_sets().len(), 2);
    assert_eq!(doc.layer_sets()[1].name, "fresh");
    let new_set = doc.layer_sets()[1].id;
    let fill = |name: &str| {
        doc.layers()
            .iter()
            .find(|l| l.name() == name)
            .unwrap_or_else(|| panic!("{name} のレイヤー"))
    };
    let in_new = fill("fresh");
    assert_eq!(in_new.layer_set(), new_set);
    assert_eq!(in_new.kind(), yolu_app::engine::LayerKind::Fill);
    assert!(
        in_new.image_ids().next().is_some(),
        "画像の塗りつぶしレイヤー"
    );
    // 既存のレイヤーセットの一番上
    let on_top = fill("top");
    assert_eq!(on_top.layer_set(), set);
    assert_eq!(
        yolu_app::m2::layersets::top_layer_of(doc, set),
        Some(on_top.id())
    );
    // 同じテクスチャセットへの 2 つのファイルは 1 回の Undo
    h.act(Action::Undo);
    h.run();
    assert_eq!(h.doc().layers().len(), layers);
    assert_eq!(h.doc().layer_sets().len(), 1);
    assert!(
        !h.doc().can_undo(),
        "{:?}",
        h.doc().history().collect::<Vec<_>>()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn psds_go_through_the_import_check_into_a_new_or_an_existing_layer_set_and_other_sizes_are_refused(
) {
    let dir = temp_dir("psds");
    let lossy = psd_file(&dir, "lossy.psd", 32, true);
    let plain = psd_file(&dir, "plain.psd", 32, false);
    let small = psd_file(&dir, "small.psd", 8, false);
    let mut h = app(1280.0, 800.0, 32);
    let uid = current_uid(&h);
    let set = first_layer_set(&h);
    let layers = h.doc().layers().len();
    drop_and_read(&mut h, &[&lossy, &plain, &small]);
    // 取り込みの確かめ（変わるもの）は行を開いて見る
    let id = row_id(&h, "lossy.psd");
    let Content::Psd(psd) = &h.dropin().row(id).unwrap().content else {
        panic!("PSD の行");
    };
    assert!(
        psd.notes()
            .iter()
            .any(|n| n.action == yolu_io::psd::ImportAction::Changed),
        "変わるものが並ぶ"
    );
    // 大きさの違う PSD は、このテクスチャセットへは入れられない（行を押せない・「すべて」で当てない）
    let small_id = row_id(&h, "small.psd");
    for dest in [Dest::NewSet(uid), Dest::Set(uid, set), Dest::Asset] {
        assert!(h.app().dropin_problem(small_id, dest).is_some(), "{dest:?}");
    }
    assert_eq!(
        h.app()
            .dropin_problem(small_id, Dest::NewSet(uid))
            .as_deref(),
        Some("大きさが違う")
    );
    h.act(Action::DropIn(DropOp::SetAll(Dest::NewSet(uid))));
    h.run();
    assert_eq!(h.dropin().row(small_id).unwrap().dest, None);
    assert!(import_disabled(&h), "大きさの違う PSD は入れ先が決まらない");
    h.act(Action::DropIn(DropOp::Remove(small_id)));
    set_dest(&mut h, "plain.psd", Dest::Set(uid, set));
    h.get_by_label("取り込む").click();
    h.run();
    assert!(!h.dropin().is_open(), "{}", h.message());
    let doc = h.doc();
    // lossy は新しいレイヤーセット（ファイルの名前）、plain は既存のレイヤーセットの一番上
    assert_eq!(doc.layer_sets().len(), 2);
    assert_eq!(doc.layer_sets()[1].name, "lossy");
    let lossy_set = doc.layer_sets()[1].id;
    let in_set = |s| doc.layers().iter().filter(|l| l.layer_set() == s).count();
    assert_eq!(in_set(lossy_set), 2);
    assert_eq!(doc.layers().len(), layers + 4);
    let top = yolu_app::m2::layersets::top_layer_of(doc, set).unwrap();
    assert_eq!(doc.layer(top).unwrap().name(), "上");
    // 取り込んだ PSD の名前は、同じファイルへ書き出すときの確かめのために覚える（前の取り込みと同じ）
    // 最後の入れ先のレイヤーセットへ切り替わる
    assert_eq!(h.app().layer_set(), set);
    h.act(Action::Undo);
    h.run();
    assert_eq!(h.doc().layers().len(), layers, "1 回の Undo で両方戻る");
    assert_eq!(h.doc().layer_sets().len(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn dropping_or_importing_while_stroking_is_refused_and_changes_nothing() {
    let dir = temp_dir("stroking");
    let psd = psd_file(&dir, "a.psd", 32, false);
    let mut h = app(1280.0, 800.0, 32);
    let begin = |h: &mut H| {
        let stroke = {
            let app = h.app_mut();
            let layer = app.selected_layer().unwrap();
            let settings = app.stroke_settings(false);
            app.doc_mut().begin_stroke(layer, &settings).unwrap()
        };
        *h.stroke_mut() = Some(stroke);
        h.run();
    };
    let end = |h: &mut H| {
        let stroke = h.stroke_mut().take().unwrap();
        h.doc_mut().cancel_stroke(stroke);
        h.run();
    };
    begin(&mut h);
    h.drop_files_at(pos2(900.0, 500.0), &[&psd]);
    assert!(!h.dropin().is_open(), "落としても読み始めない");
    assert!(h.message().contains("描いている間"), "{}", h.message());
    end(&mut h);
    drop_and_read(&mut h, &[&psd]);
    let uid = current_uid(&h);
    set_dest(&mut h, "a.psd", Dest::NewSet(uid));
    // 取り込む前に描き始めた（ウィンドウの外の道）: 取り込むで断り、ウィンドウは残す
    begin(&mut h);
    h.act(Action::DropIn(DropOp::Import));
    assert!(h.message().contains("描いている間"), "{}", h.message());
    assert!(h.dropin().is_open());
    assert_eq!(h.doc().layer_sets().len(), 1);
    end(&mut h);
    h.act(Action::DropIn(DropOp::Import));
    h.run();
    assert_eq!(h.doc().layer_sets().len(), 2);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_psd_and_two_images_go_to_two_texture_sets_and_the_assets_and_undo_per_set() {
    let dir = temp_dir("through");
    let psd = psd_file(&dir, "body.psd", 32, false);
    let png = image_file(&dir, "skin.png", [200, 150, 120]);
    let jpg = image_file(&dir, "photo.jpg", [20, 30, 40]);
    let mut h = app(1280.0, 800.0, 32);
    let first = current_uid(&h);
    let second = h.app_mut().add_texture_set().unwrap();
    h.run();
    let second_index = h.sets().index_of(second).unwrap();
    let first_index = h.sets().index_of(first).unwrap();
    let second_set = h.app().set_doc(second_index).layer_sets()[0].id;
    let second_layers = h.app().set_doc(second_index).layers().len();
    let first_layers = h.app().set_doc(first_index).layers().len();
    let assets = h.shelf().resources().len();
    drop_and_read(&mut h, &[&psd, &png, &jpg]);
    set_dest(&mut h, "body.psd", Dest::NewSet(first));
    set_dest(&mut h, "skin.png", Dest::Set(second, second_set));
    set_dest(&mut h, "photo.jpg", Dest::Asset);
    h.get_by_label("取り込む").click();
    h.run();
    assert!(!h.dropin().is_open(), "{}", h.message());
    // テクスチャセット 1 に PSD の名前のレイヤーセット
    let one = h.app().set_doc(first_index);
    assert_eq!(one.layer_sets().len(), 2);
    assert_eq!(one.layer_sets()[1].name, "body");
    assert_eq!(one.layers().len(), first_layers + 2);
    // テクスチャセット 2 の今のレイヤーセットの一番上に、画像の塗りつぶしレイヤー
    let two = h.app().set_doc(second_index);
    assert_eq!(two.layers().len(), second_layers + 1);
    let top = yolu_app::m2::layersets::top_layer_of(two, second_set).unwrap();
    assert_eq!(two.layer(top).unwrap().name(), "skin");
    assert_eq!(
        two.layer(top).unwrap().kind(),
        yolu_app::engine::LayerKind::Fill
    );
    // アセットに 2 つ（PNG と JPEG）
    assert_eq!(h.shelf().resources().len(), assets + 2);
    assert!(h.shelf().resources().iter().any(|r| r.name == "photo"));
    // 最後の入れ先（テクスチャセット 2）へ切り替わる
    assert_eq!(current_uid(&h), second);
    // テクスチャセット 1 で Ctrl+Z 1 回で、PSD のレイヤーセットが消える（テクスチャセット 2 はそのまま）
    h.act(Action::SelectSet(first));
    h.run();
    assert_eq!(current_uid(&h), first);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run();
    assert_eq!(h.doc().layer_sets().len(), 1, "{}", h.message());
    assert_eq!(h.doc().layers().len(), first_layers);
    assert_eq!(
        h.app().set_doc(second_index).layers().len(),
        second_layers + 1
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// ウィンドウのスクリーンショット: 入れ先の一覧を開いた形（画像の行・PSD の行）と、PSD の行の取り込みの確かめを開いた形（日英）。
#[test]
fn the_import_files_window_looks_right() {
    for lang in Lang::JA_EN {
        let dir = temp_dir(&format!("shots-{lang:?}"));
        let psd = psd_file(&dir, "body.psd", 32, true);
        let small = psd_file(&dir, "small.psd", 8, false);
        let png = image_file(&dir, "skin_base.png", [200, 150, 120]);
        let webp = image_file(&dir, "cloth.webp", [90, 120, 200]);
        let dds = dir.join("normal.dds");
        std::fs::write(&dds, b"DDS ....").unwrap();
        let mut h = app(1280.0, 800.0, 32);
        h.app_mut().set_language(lang);
        h.app_mut().add_texture_set().unwrap();
        h.run();
        drop_and_read(&mut h, &[&psd, &small, &png, &webp, &dds]);
        let suffix = if lang == Lang::Ja { "" } else { "_english" };
        // ポインタは一覧の外（行のホバーをスクリーンショットに残さない）
        move_to(&h, pos2(4.0, 790.0));
        h.run();
        let png_id = row_id(&h, "skin_base.png");
        h.dropin_mut().menu = Some(MenuFor::Row(png_id));
        h.run();
        shot_with_menu(&mut h, &format!("dropin_window_menu{suffix}"));
        let small_id = row_id(&h, "small.psd");
        h.dropin_mut().menu = Some(MenuFor::Row(small_id));
        h.run();
        shot_with_menu(&mut h, &format!("dropin_window_menu_psd{suffix}"));
        h.dropin_mut().menu = None;
        let psd_id = row_id(&h, "body.psd");
        h.act(Action::DropIn(DropOp::ToggleCheck(psd_id)));
        let uid = current_uid(&h);
        h.act(Action::DropIn(DropOp::SetDest(psd_id, Dest::NewSet(uid))));
        h.act(Action::DropIn(DropOp::SetDest(png_id, Dest::Asset)));
        h.run();
        crate::common::shot::window_shot(
            &mut h,
            "import-files",
            &format!("dropin_window_psd_check{suffix}"),
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// ウィンドウと、開いている入れ先の一覧を合わせた範囲を撮る。
fn shot_with_menu(h: &mut H, name: &str) {
    let window = yolu_app::windows::window_rect(&h.ctx, "import-files").expect("ウィンドウ");
    let menu = h
        .ctx
        .memory(|m| {
            m.areas()
                .visible_layer_ids()
                .into_iter()
                .filter(|l| l.order == egui::Order::Tooltip)
                .filter_map(|l| m.area_rect(l.id))
                .reduce(|a, b| a.union(b))
        })
        .unwrap_or(window);
    crate::common::shot::clean_region(h, name, window.union(menu));
}

/// 色の画像（浮動小数）を書く。形式は拡張子から（EXR・HDR）。
fn float_file(dir: &Path, name: &str, rgb: [f32; 3]) -> PathBuf {
    let path = dir.join(name);
    let img = image::Rgb32FImage::from_pixel(16, 8, image::Rgb(rgb));
    image::DynamicImage::ImageRgb32F(img).save(&path).unwrap();
    path
}

/// WebP・TIFF・BMP・EXR・HDR・GIF も、PNG・JPEG・TGA と同じく行になって読まれ、アセットに入る（EXR・HDR は色として 8 ビットへ）。
#[test]
fn webp_tiff_bmp_exr_hdr_and_gif_are_read_as_rows_and_go_to_the_assets() {
    let dir = temp_dir("new-formats");
    let files = [
        (image_file(&dir, "a.webp", [10, 20, 30]), "WebP"),
        (image_file(&dir, "b.tiff", [40, 50, 60]), "TIFF"),
        (image_file(&dir, "c.bmp", [70, 80, 90]), "BMP"),
        (float_file(&dir, "d.exr", [0.5, 0.25, 1.0]), "EXR"),
        (float_file(&dir, "e.hdr", [0.2, 0.4, 0.8]), "HDR"),
        (image_file(&dir, "f.gif", [100, 110, 120]), "GIF"),
        (image_file(&dir, "g.TIF", [130, 140, 150]), "TIFF"),
    ];
    let mut h = app(1280.0, 800.0, 32);
    let assets = h.shelf().resources().len();
    let paths: Vec<&Path> = files.iter().map(|(p, _)| p.as_path()).collect();
    drop_and_read(&mut h, &paths);
    assert_eq!(h.dropin().rows.len(), files.len());
    for (path, label) in &files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let row = h.dropin().row(row_id(&h, &name)).unwrap();
        let Content::Image(img) = &row.content else {
            panic!("{name} は読める画像の行: {}", h.message());
        };
        assert_eq!(
            (img.format, img.width, img.height),
            (*label, 16, 8),
            "{name}"
        );
    }
    h.act(Action::DropIn(DropOp::SetAll(Dest::Asset)));
    h.run();
    assert!(!import_disabled(&h));
    h.get_by_label("取り込む").click();
    h.run();
    assert!(!h.dropin().is_open(), "{}", h.message());
    assert_eq!(
        h.shelf().resources().len(),
        assets + files.len(),
        "{}",
        h.message()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// 棚の格子（アセットのタブ）の範囲を出して返す。
fn open_assets(h: &mut H) -> egui::Rect {
    click_tab(h, yolu_app::Tab::Assets);
    for _ in 0..30 {
        h.step();
    }
    h.run();
    h.ui().library_grid_rect.expect("格子を描いている")
}

/// プロジェクトの棚の格子の上へ落とした画像は、取り込みのウィンドウに並べず、文書の画像として棚に足す。読めない形式は理由つきで断る。
#[test]
fn images_dropped_on_the_project_shelf_grid_are_added_to_the_shelf_and_unreadable_ones_are_refused()
{
    let dir = temp_dir("on-project-shelf");
    let webp = image_file(&dir, "wood.webp", [10, 20, 30]);
    let exr = float_file(&dir, "sky.exr", [0.5, 0.5, 0.5]);
    let png = image_file(&dir, "tip.png", [50, 60, 70]);
    let dds = dir.join("normal.dds");
    std::fs::write(&dds, b"DDS ....").unwrap();
    let mut h = app(1280.0, 1400.0, 64);
    let grid = open_assets(&mut h);
    let assets = h.shelf().resources().len();
    h.drop_files_at(grid.center(), &[&webp, &exr, &png]);
    h.app_mut().wait_image_reads();
    h.run();
    assert!(!h.dropin().is_open(), "ウィンドウには並べない");
    assert_eq!(h.shelf().resources().len(), assets + 3, "{}", h.message());
    // 読めない形式は断り、棚は変わらない
    h.drop_files_at(grid.center(), &[&dds]);
    h.app_mut().wait_image_reads();
    h.run();
    assert!(!h.dropin().is_open());
    assert_eq!(h.shelf().resources().len(), assets + 3);
    assert!(
        h.message().contains("normal") && h.message().contains("未対応の形式"),
        "{}",
        h.message()
    );
    // 格子の外へ落とした同じ画像は、今までどおりウィンドウの行になる
    let outside = pos2(grid.right() + 40.0, grid.center().y);
    assert!(!grid.contains(outside));
    h.drop_files_at(outside, &[&webp]);
    h.app_mut().wait_dropin();
    h.run();
    assert!(h.dropin().is_open());
    assert_eq!(h.shelf().resources().len(), assets + 3);
    std::fs::remove_dir_all(dir).unwrap();
}

/// 3 枚（BaseColor・Normal・Roughness）と印の無い 1 枚をプロジェクトの棚の格子へ一度に落とすと、マテリアルが 1 つと、画像が足される。
/// 取り込みのウィンドウは開かない。置くと塗りつぶしの各チャンネルにその画像が入る。
fn pbr_files(dir: &Path) -> Vec<PathBuf> {
    vec![
        image_file(dir, "Wood_BaseColor.png", [120, 80, 40]),
        image_file(dir, "Wood_Normal.png", [128, 128, 255]),
        image_file(dir, "Wood_Roughness.png", [90, 90, 90]),
        image_file(dir, "tip.png", [50, 60, 70]),
    ]
}

#[test]
fn several_images_dropped_on_the_project_shelf_grid_with_pbr_marks_become_a_material() {
    let dir = temp_dir("pbr-on-grid");
    let files = pbr_files(&dir);
    let mut h = app(1280.0, 1400.0, 64);
    let grid = open_assets(&mut h);
    let count = |h: &H, kind: &str| {
        h.shelf()
            .resources()
            .iter()
            .filter(|r| r.kind == kind)
            .count()
    };
    let (materials, images) = (count(&h, "material"), count(&h, "image"));
    let paths: Vec<&Path> = files.iter().map(PathBuf::as_path).collect();
    h.drop_files_at(grid.center(), &paths);
    h.app_mut().wait_image_reads();
    h.run();
    assert!(!h.dropin().is_open(), "ウィンドウには並べない");
    assert_eq!(count(&h, "material"), materials + 1, "{}", h.message());
    // 使う 3 枚も、印の無い 1 枚も、画像としてアセットに並ぶ
    assert_eq!(count(&h, "image"), images + 4, "{}", h.message());
    assert!(
        h.message().contains("マテリアル") && h.message().contains("Wood"),
        "{}",
        h.message()
    );
    let wood = h
        .shelf()
        .resources()
        .iter()
        .find(|r| r.kind == "material" && r.name == "Wood")
        .expect("マテリアル Wood")
        .id
        .clone();
    h.act(Action::Shelf(yolu_app::shelf::ShelfOp::Place {
        id: wood,
        target: yolu_app::shelf::PlaceTarget::Selected,
    }));
    h.run();
    let layer = h.doc().layer(h.selected_layer().unwrap()).unwrap();
    let channels: Vec<_> = layer.fill_images().map(|(c, _)| c).collect();
    assert_eq!(
        channels,
        [
            yolu_app::engine::Channel::Color,
            yolu_app::engine::Channel::Roughness,
            yolu_app::engine::Channel::Normal
        ]
    );
    // 1 枚だけ落としたときは、印があっても今までどおり画像
    let single = image_file(&dir, "Stone_Normal.png", [120, 130, 250]);
    let (materials, images) = (count(&h, "material"), count(&h, "image"));
    h.drop_files_at(grid.center(), &[&single]);
    h.app_mut().wait_image_reads();
    h.run();
    assert_eq!(count(&h, "material"), materials);
    assert_eq!(count(&h, "image"), images + 1);
    std::fs::remove_dir_all(dir).unwrap();
}

/// マテリアルの項目が足された格子（ja・en）。
#[test]
fn the_assets_grid_after_dropping_pbr_images_shows_the_material_item() {
    let dir = temp_dir("pbr-shot");
    let files = pbr_files(&dir);
    for lang in Lang::JA_EN {
        let mut h = app(1280.0, 1400.0, 64);
        h.app_mut().set_lang(lang);
        h.shelf_mut().show_builtin = false;
        let grid = open_assets(&mut h);
        let paths: Vec<&Path> = files.iter().map(PathBuf::as_path).collect();
        h.drop_files_at(grid.center(), &paths);
        h.app_mut().wait_image_reads();
        for _ in 0..30 {
            h.step();
        }
        h.run();
        let panel = common::shot::group(&h, yolu_app::Tab::Assets);
        common::shot::clean_region(
            &mut h,
            &format!("assets_pbr_material_{}", lang.pick("ja", "en")),
            panel,
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// ライブラリの格子の上へ落とした PNG 以外の画像は、PNG に直してライブラリに置く。読めない形式は断る。
#[test]
fn images_of_other_formats_dropped_on_the_library_grid_become_pngs_in_the_library() {
    let dir = temp_dir("on-library-formats");
    let library = dir.join("lib");
    std::fs::create_dir_all(&library).unwrap();
    let webp = image_file(&dir, "wood.webp", [10, 20, 30]);
    let exr = float_file(&dir, "sky.exr", [0.5, 0.5, 0.5]);
    let dds = dir.join("normal.dds");
    std::fs::write(&dds, b"DDS ....").unwrap();
    let mut h = app(1280.0, 1400.0, 64);
    click_tab(&mut h, yolu_app::Tab::Assets);
    for _ in 0..30 {
        h.step();
    }
    h.prefs_mut().settings.library_folder = Some(library.clone());
    h.shelf_mut().show_builtin = false;
    h.run();
    h.get_by_label("ライブラリ").click();
    h.run();
    let grid = h.ui().library_grid_rect.expect("格子を描いている");
    h.drop_files_at(grid.center(), &[&webp, &exr]);
    h.app_mut().library_wait();
    h.run();
    assert!(!h.dropin().is_open());
    let read = |name: &str| {
        let bytes = std::fs::read(library.join("Images").join(name))
            .unwrap_or_else(|e| panic!("{name}: {e} {}", h.message()));
        image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            .unwrap()
            .to_rgba8()
    };
    let wood = read("wood.png");
    assert_eq!((wood.width(), wood.height()), (16, 8));
    assert_eq!(wood.get_pixel(0, 0).0, [10, 20, 30, 255]);
    // EXR は色として 0〜1 で切って sRGB へ（線形の 0.5 は 188）
    assert_eq!(read("sky.png").get_pixel(3, 3).0, [188, 188, 188, 255]);
    // 読めない形式は置かず、理由を出す
    h.drop_files_at(grid.center(), &[&dds]);
    h.app_mut().library_wait();
    h.run();
    assert!(!library.join("Images/normal.dds").exists());
    assert!(std::fs::read_dir(library.join("Images")).unwrap().all(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        != "normal.png"));
    assert!(h.message().contains("normal"), "{}", h.message());
    std::fs::remove_dir_all(dir).unwrap();
}

/// メニュー「ファイル → インポート → 画像…」は、画像を選ぶウィンドウを頼む。選んだファイルは落としたときと同じ取り込みのウィンドウの行になる
/// （描いている最中は頼まない）。
#[test]
fn the_import_images_menu_asks_for_files_and_the_chosen_ones_open_the_window_like_a_drop() {
    use yolu_app::state::DialogRequest;
    let dir = temp_dir("menu");
    let webp = image_file(&dir, "wood.webp", [10, 20, 30]);
    let tga = image_file(&dir, "mask.tga", [1, 2, 3]);
    let mut h = app(1280.0, 800.0, 32);
    h.act(Action::DropIn(DropOp::ImportDialog));
    assert_eq!(*h.dialog_request(), Some(DialogRequest::DropInImages));
    assert!(!h.dropin().is_open(), "選ぶまでウィンドウは出ない");
    // ウィンドウ（OS）で選んだあとにアプリが当てる操作と同じ（テストでは OS のウィンドウを開かない）
    *h.dialog_request_mut() = None;
    h.act(Action::DropIn(DropOp::Open(vec![webp, tga])));
    h.app_mut().wait_dropin();
    h.run();
    let names: Vec<&str> = h.dropin().rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["wood.webp", "mask.tga"]);
    assert!(h.dropin().rows.iter().all(|r| r.is_readable()));
    std::fs::remove_dir_all(dir).unwrap();
}

/// 描いている最中は、画像を選ぶウィンドウを頼まない。
#[test]
fn the_import_images_menu_is_refused_while_stroking() {
    let mut h = app(1280.0, 800.0, 32);
    let stroke = {
        let app = h.app_mut();
        let layer = app.selected_layer().unwrap();
        let settings = app.stroke_settings(false);
        app.doc_mut().begin_stroke(layer, &settings).unwrap()
    };
    *h.stroke_mut() = Some(stroke);
    h.run();
    h.act(Action::DropIn(DropOp::ImportDialog));
    assert!(h.dialog_request().is_none());
    assert!(!h.message().is_empty());
}

/// ライブラリの格子の上へ落とした PNG はライブラリが受ける（ファイルの取り込みのウィンドウには並べない）。
#[test]
fn a_png_dropped_on_the_library_grid_goes_to_the_library_and_not_to_the_window() {
    let dir = temp_dir("on-library");
    let library = dir.join("lib");
    std::fs::create_dir_all(&library).unwrap();
    let png = image_file(&dir, "tip.png", [10, 20, 30]);
    let mut h = app(1280.0, 1400.0, 64);
    click_tab(&mut h, yolu_app::Tab::Assets);
    for _ in 0..30 {
        h.step();
    }
    h.prefs_mut().settings.library_folder = Some(library.clone());
    h.shelf_mut().show_builtin = false;
    h.run();
    h.get_by_label("ライブラリ").click();
    h.run();
    let grid = h.ui().library_grid_rect.expect("格子を描いている");
    h.drop_files_at(grid.center(), &[&png]);
    h.app_mut().library_wait();
    h.run();
    assert!(!h.dropin().is_open(), "ウィンドウには並べない");
    assert!(library.join("Images/tip.png").exists(), "{}", h.message());
    // 格子の外へ落とした同じ PNG は、ライブラリへは入らず、ウィンドウの行になる
    let outside = pos2(grid.right() + 40.0, grid.center().y);
    assert!(!grid.contains(outside));
    std::fs::remove_file(library.join("Images/tip.png")).unwrap();
    h.drop_files_at(outside, &[&png]);
    h.app_mut().wait_dropin();
    h.run();
    assert!(h.dropin().is_open());
    assert!(!library.join("Images/tip.png").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

/// ステンシルの画像の箱の上へ落とした PNG はステンシルが受ける（ファイルの取り込みのウィンドウには並べない）。
#[test]
fn a_png_dropped_on_the_stencil_image_box_loads_as_the_stencil_and_not_to_the_window() {
    let dir = temp_dir("on-stencil");
    let png = image_file(&dir, "mask.png", [10, 20, 30]);
    let mut h = app(1280.0, 1000.0, 128);
    h.ui_mut().property_tab = 0;
    h.run();
    let boxed = h.get_by_label("画像: なし").rect();
    h.drop_files_at(boxed.center(), &[&png]);
    h.run();
    assert!(!h.dropin().is_open(), "ウィンドウには並べない");
    assert_eq!(
        h.stencil().image.as_ref().map(|i| i.name.as_str()),
        Some("mask.png"),
        "{}",
        h.message()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// 2 つ目の入れ先を文書の側が断る（レイヤーセットの数が上限）と、入れ終えた 1 つ目は残り（1 回の Undo で戻る）、入れられなかった入れ先の行は
/// 一覧に残ってウィンドウは開いたまま。
#[test]
fn when_a_later_destination_is_refused_the_finished_ones_stay_and_the_rest_stay_listed() {
    let dir = temp_dir("partial");
    let first_png = image_file(&dir, "one.png", [10, 20, 30]);
    let second_png = image_file(&dir, "two.png", [40, 50, 60]);
    let mut h = app(1280.0, 800.0, 32);
    let first = current_uid(&h);
    let second = h.app_mut().add_texture_set().unwrap();
    h.run();
    let first_index = h.sets().index_of(first).unwrap();
    let second_index = h.sets().index_of(second).unwrap();
    // 2 つ目のテクスチャセットのレイヤーセットを上限まで作る（新しいレイヤーセットを作れない）
    let doc = h.app_mut().set_doc_mut(second_index);
    let mut n = 0;
    while doc.add_layer_set(&format!("s{n}")).is_ok() {
        n += 1;
    }
    let second_sets = h.app().set_doc(second_index).layer_sets().len();
    let first_sets = h.app().set_doc(first_index).layer_sets().len();
    drop_and_read(&mut h, &[&first_png, &second_png]);
    set_dest(&mut h, "one.png", Dest::NewSet(first));
    set_dest(&mut h, "two.png", Dest::NewSet(second));
    h.act(Action::DropIn(DropOp::Import));
    h.run();
    // 1 つ目は入っている
    assert_eq!(
        h.app().set_doc(first_index).layer_sets().len(),
        first_sets + 1
    );
    // 2 つ目は変わらず、その行だけが残って、ウィンドウは開いたまま
    assert_eq!(
        h.app().set_doc(second_index).layer_sets().len(),
        second_sets
    );
    assert!(h.dropin().is_open());
    let names: Vec<&str> = h.dropin().rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["two.png"], "{}", h.message());
    assert_eq!(
        h.dropin().rows[0].dest,
        Some(Dest::NewSet(second)),
        "選んだ入れ先も残る"
    );
    // 入れ終えた分は、そのテクスチャセットの 1 回の Undo で戻る
    h.act(Action::SelectSet(first));
    h.run();
    h.act(Action::Undo);
    h.run();
    assert_eq!(h.doc().layer_sets().len(), first_sets);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_image_into_a_layer_set_uses_a_channel_of_its_set_and_a_set_without_channels_is_refused() {
    let dir = temp_dir("set-channels");
    let png = image_file(&dir, "mask.png", [10, 20, 30]);
    let mut h = app(1280.0, 800.0, 32);
    let uid = current_uid(&h);
    let set = first_layer_set(&h);
    assert_eq!(h.m2().paint_channel, yolu_app::engine::Channel::Color);
    // 描くチャンネル（カラー）を組から外したレイヤーセットへ: 組の最初のチャンネルに置く（組の外に置くと合成に出ない）
    let bits = h.doc().layer_set_channels(set).unwrap() & !yolu_app::engine::Channel::Color.bit();
    h.doc_mut().set_layer_set_channels(set, bits).unwrap();
    drop_and_read(&mut h, &[&png]);
    set_dest(&mut h, "mask.png", Dest::Set(uid, set));
    h.act(Action::DropIn(DropOp::Import));
    h.run();
    assert!(!h.dropin().is_open(), "{}", h.message());
    let doc = h.doc();
    let fill = doc.layers().iter().find(|l| l.name() == "mask").unwrap();
    let first = doc
        .channels()
        .into_iter()
        .find(|c| bits & c.bit() != 0)
        .unwrap();
    assert!(fill.is_channel_enabled(first));
    assert!(!fill.is_channel_enabled(yolu_app::engine::Channel::Color));
    // 組が空のレイヤーセット: 置いても合成に出ないので断り、行を残す
    h.act(Action::Undo);
    h.run();
    h.doc_mut().set_layer_set_channels(set, 0).unwrap();
    let layers = h.doc().layers().len();
    drop_and_read(&mut h, &[&png]);
    set_dest(&mut h, "mask.png", Dest::Set(uid, set));
    h.act(Action::DropIn(DropOp::Import));
    h.run();
    assert_eq!(h.doc().layers().len(), layers);
    assert!(h.dropin().is_open());
    assert!(
        h.message()
            .contains("レイヤーセットがそのチャンネルを扱わない"),
        "{}",
        h.message()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// 測定（普段は回さない）。8192² の EXR を棚の格子へ落としたとき、画面のスレッドの 1 フレームの最大の時間と、棚に入るまでの時間。
/// 回し方: `cargo test --release -p yolu-app --test gui_shell dropin::measure_ -- --ignored --nocapture --test-threads=1`
#[test]
#[ignore = "測定。--ignored --nocapture で回す"]
fn measure_how_long_dropping_a_huge_exr_stalls_the_screen() {
    use std::time::Instant;
    let dir = temp_dir("measure-exr");
    let path = dir.join("big.exr");
    {
        use exr::prelude::*;
        write_rgba_file(&path, 8192, 8192, |x, y| {
            (
                (x % 251) as f32 / 251.0,
                (y % 241) as f32 / 241.0,
                ((x + y) % 239) as f32 / 239.0,
                1.0,
            )
        })
        .unwrap();
    }
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|t| {
            t.lines()
                .find(|l| l.starts_with("model name"))
                .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_owned())
        })
        .unwrap_or_else(|| "不明".into());
    println!(
        "環境: {cpu} / スレッド {}",
        std::thread::available_parallelism().map_or(0, |n| n.get())
    );
    let mut h = app(1280.0, 1400.0, 64);
    let grid = open_assets(&mut h);
    let assets = h.shelf().resources().len();
    h.event(egui::Event::PointerMoved(grid.center()));
    h.step();
    h.input_mut()
        .dropped_files
        .push(std::sync::Arc::new(DroppedPathForMeasure(path.clone())));
    let started = Instant::now();
    let mut frames = Vec::new();
    while h.shelf().resources().len() == assets && started.elapsed().as_secs() < 120 {
        let t = Instant::now();
        h.step();
        frames.push(t.elapsed().as_secs_f64() * 1000.0);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let total = started.elapsed().as_secs_f64();
    let mut sorted = frames.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    println!(
        "8192² の EXR を落とす: 棚に入るまで {total:.2} 秒・{} フレーム。1 フレームは中央値 {:.2} ミリ秒・最大 {:.0} ミリ秒",
        frames.len(),
        sorted[sorted.len() / 2],
        sorted[sorted.len() - 1]
    );
    println!(
        "フレームの時間（ミリ秒、時間順の上位 5 つ）: {:?}",
        {
            let mut top: Vec<f64> = frames.clone();
            top.sort_by(|a, b| b.total_cmp(a));
            top.truncate(5);
            top.iter().map(|v| v.round() as i64).collect::<Vec<_>>()
        }
    );
    assert_eq!(h.shelf().resources().len(), assets + 1, "{}", h.message());
    std::fs::remove_dir_all(dir).unwrap();
}

/// 測定（普段は回さない）。4096² の画像 4 枚（BaseColor・Normal・Roughness・Metallic）の組を棚の格子へ落とし、マテリアルまで足すときの、
/// 画面のスレッドの 1 フレームの最大の時間。
/// 回し方: `cargo test --release -p yolu-app --test gui_shell dropin::measure_ -- --ignored --nocapture --test-threads=1`
#[test]
#[ignore = "測定。--ignored --nocapture で回す"]
fn measure_how_long_dropping_a_four_image_set_stalls_the_screen() {
    use std::time::Instant;
    let dir = temp_dir("measure-set");
    let side = 4096u32;
    let mut paths = Vec::new();
    for (i, name) in [
        "Wood_BaseColor.png",
        "Wood_Normal.png",
        "Wood_Roughness.png",
        "Wood_Metallic.png",
    ]
    .iter()
    .enumerate()
    {
        let path = dir.join(name);
        let img = image::RgbaImage::from_fn(side, side, |x, y| {
            image::Rgba([
                ((x + i as u32 * 17) % 251) as u8,
                (y % 241) as u8,
                ((x + y) % 239) as u8,
                255,
            ])
        });
        img.save(&path).unwrap();
        paths.push(path);
    }
    let mut h = app(1280.0, 1400.0, 64);
    let grid = open_assets(&mut h);
    let assets = h.shelf().resources().len();
    h.event(egui::Event::PointerMoved(grid.center()));
    h.step();
    for path in &paths {
        h.input_mut()
            .dropped_files
            .push(std::sync::Arc::new(DroppedPathForMeasure(path.clone())));
    }
    let started = Instant::now();
    let mut frames = Vec::new();
    // 4 枚の画像とマテリアル 1 つ
    while h.shelf().resources().len() < assets + 5 && started.elapsed().as_secs() < 120 {
        let t = Instant::now();
        h.step();
        frames.push(t.elapsed().as_secs_f64() * 1000.0);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let total = started.elapsed().as_secs_f64();
    let mut top: Vec<f64> = frames.clone();
    top.sort_by(|a, b| b.total_cmp(a));
    println!(
        "4096² の組 4 枚を落とす: 棚に入るまで {total:.2} 秒・{} フレーム。1 フレームの最大 {:.0} ミリ秒（時間順の上位 5 つ {:?}）",
        frames.len(),
        top[0],
        top.iter().take(5).map(|v| v.round() as i64).collect::<Vec<_>>()
    );
    assert_eq!(h.shelf().resources().len(), assets + 5, "{}", h.message());
    std::fs::remove_dir_all(dir).unwrap();
}

#[derive(Debug)]
struct DroppedPathForMeasure(PathBuf);

impl egui::DroppedFile for DroppedPathForMeasure {
    fn path(&self) -> &Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

/// 棚の格子へ落とした画像は、読んでいる間も画面が動き（棚は変わらない）、状態の札に「画像を読み込み中」と取消が出る。取り消すと足さない。
/// 読み終われば足される。
#[test]
fn images_dropped_on_the_shelf_grid_show_a_card_while_they_are_read_and_can_be_canceled() {
    let dir = temp_dir("read-card");
    let png = image_file(&dir, "tip.png", [50, 60, 70]);
    let other = image_file(&dir, "wood.png", [10, 20, 30]);
    for lang in Lang::JA_EN {
        let mut h = app(1280.0, 1400.0, 64);
        h.app_mut().set_lang(lang);
        let grid = open_assets(&mut h);
        let assets = h.shelf().resources().len();
        h.shelf().hold_image_reads(true);
        h.drop_files_at(grid.center(), &[&png]);
        for _ in 0..5 {
            h.step();
        }
        assert!(h.shelf().is_reading_images());
        assert_eq!(
            h.shelf().resources().len(),
            assets,
            "読んでいる間は変わらない"
        );
        let cards = yolu_app::jobs::cards(h.app(), lang);
        assert_eq!(cards.len(), 1);
        assert_eq!(
            cards[0].1.text,
            lang.pick("画像を読み込み中", "Reading images")
        );
        assert!(matches!(
            cards[0].1.cancel,
            Some(Action::Fill(yolu_app::fillfx::FillOp::CancelImageRead))
        ));
        let rect = h
            .ctx
            .memory(|m| m.area_rect(egui::Id::new("yolu.jobs")))
            .expect("札を描いている");
        common::shot::clean_region(
            &mut h,
            &format!("image_read_card_{}", lang.pick("ja", "en")),
            rect,
        );
        // 取り消すと足さない
        h.act(Action::Fill(yolu_app::fillfx::FillOp::CancelImageRead));
        h.shelf().hold_image_reads(false);
        std::thread::sleep(std::time::Duration::from_millis(100));
        h.run();
        assert!(!h.shelf().is_reading_images());
        assert_eq!(h.shelf().resources().len(), assets);
        // 読み終われば足される
        h.drop_files_at(grid.center(), &[&other]);
        h.app_mut().wait_image_reads();
        h.run();
        assert_eq!(h.shelf().resources().len(), assets + 1, "{}", h.message());
    }
    std::fs::remove_dir_all(dir).unwrap();
}
