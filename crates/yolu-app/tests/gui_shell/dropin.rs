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
    let png = image_file(&dir, "g.png", [1, 2, 3]);
    let mut h = app(1280.0, 800.0, 32);
    drop_and_read(&mut h, &[&webp, &broken, &text, &png]);
    assert_eq!(h.dropin().rows.len(), 4);
    for name in ["e.webp", "f.png", "notes.txt"] {
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
    for lang in Lang::ALL {
        let dir = temp_dir(&format!("shots-{lang:?}"));
        let psd = psd_file(&dir, "body.psd", 32, true);
        let small = psd_file(&dir, "small.psd", 8, false);
        let png = image_file(&dir, "skin_base.png", [200, 150, 120]);
        let webp = dir.join("cloth.webp");
        std::fs::write(&webp, b"RIFF....WEBP").unwrap();
        let mut h = app(1280.0, 800.0, 32);
        h.app_mut().set_language(lang);
        h.app_mut().add_texture_set().unwrap();
        h.run();
        drop_and_read(&mut h, &[&psd, &small, &png, &webp]);
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
