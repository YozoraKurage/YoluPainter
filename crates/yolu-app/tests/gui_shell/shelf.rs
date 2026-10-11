//! アセットの棚（スマートマテリアル・スマートマスク・.ylsmart・.ylp への保存）の試験。`headless_` で始まる試験は画面を描かず、
//! Wine でも回る。どれも「操作 → 棚か文書が変わる → 取り消し（文書）・開き直し（棚）で戻る」。素材はこのリポジトリの人工データ
//! （yolu-io の tests/fixtures/smart）と、試験の中で作ったものだけを使う。
use crate::common;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use yolu_app::engine::{Channel, Document, LayerId, TileCoord};
use yolu_app::lang::Lang;
use yolu_app::m2::Edit;
use yolu_app::shelf::{Block, ItemKind, PlaceTarget, ShelfOp, ShelfState, SHELF_BUDGET};
use yolu_app::state::{Action, AppState, DialogRequest, StrokeSource};
use yolu_io::shelf::Shelf;
use yolu_io::shelf::MAX_RESOURCES;
use yolu_io::Project;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../yolu-io/tests/fixtures/smart")
}

/// 画像・スマートマテリアル・スマートマスク・マテリアル・ブラシの 6 つが入った棚（人工データ）。
fn fixture_shelf() -> Shelf {
    let root = fixtures().join("all");
    let mut files: BTreeMap<String, Arc<[u8]>> = BTreeMap::from([(
        "resources.json".into(),
        Arc::from(std::fs::read(root.join("resources.json")).unwrap()),
    )]);
    for entry in std::fs::read_dir(root.join("resources")).unwrap() {
        let entry = entry.unwrap();
        files.insert(
            format!("resources/{}", entry.file_name().to_string_lossy()),
            Arc::from(std::fs::read(entry.path()).unwrap()),
        );
    }
    Shelf::read(&files, SHELF_BUDGET).unwrap()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yolu-shelf-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 左下のタイルのキャンバスの中を単色にしてレイヤーのチャンネルへ入れる。
fn paint_channel(s: &mut AppState, id: LayerId, channel: Channel, rgba: [u8; 4]) {
    let ts = s.doc.tile_size() as usize;
    let (w, h) = (s.doc.width() as usize, s.doc.height() as usize);
    let mut bytes = vec![0u8; ts * ts * 4];
    for y in 0..h.min(ts) {
        for x in 0..w.min(ts) {
            bytes[(y * ts + x) * 4..][..4].copy_from_slice(&rgba);
        }
    }
    s.doc
        .import_tile(id, channel, TileCoord::new(0, 0), &bytes)
        .unwrap();
}

fn paint(s: &mut AppState, id: LayerId, rgba: [u8; 4]) {
    paint_channel(s, id, Channel::Color, rgba);
}

/// 1 レイヤー（赤）の文書。
fn painted(size: u32) -> (AppState, LayerId) {
    let mut s = AppState::new(size, size);
    let id = s.selected_layer.unwrap();
    paint(&mut s, id, [200, 40, 30, 255]);
    (s, id)
}

fn pixel(s: &AppState, id: LayerId) -> [u8; 4] {
    s.doc
        .layer(id)
        .unwrap()
        .surface(Channel::Color)
        .unwrap()
        .pixel(1, 1)
        .unwrap()
        .to_array()
}

fn ids(s: &AppState, kind: &str) -> Vec<String> {
    s.shelf
        .resources()
        .iter()
        .filter(|r| r.kind == kind)
        .map(|r| r.id.clone())
        .collect()
}

fn place(s: &mut AppState, id: &str) {
    s.apply(Action::Shelf(ShelfOp::Place {
        id: id.into(),
        target: PlaceTarget::Selected,
    }));
}

/// 文書の全体の指紋（レイヤーの並び・親・名前・Color と マスクの全画素）。
fn fingerprint(s: &AppState) -> Vec<String> {
    let doc = &s.doc;
    doc.layers()
        .iter()
        .map(|l| {
            let mut px = Vec::new();
            for y in 0..doc.height() {
                for x in 0..doc.width() {
                    let c = l
                        .surface(Channel::Color)
                        .map(|f| f.pixel(x, y).unwrap().to_array())
                        .unwrap_or([0; 4]);
                    let m = l.mask().map(|m| m.surface().pixel(x, y).unwrap().a);
                    px.push((c, m));
                }
            }
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            px.hash(&mut h);
            format!(
                "{:?} {:?} {:?} {:?} mask={} {:x}",
                l.id(),
                l.parent(),
                l.name(),
                l.kind(),
                l.mask().is_some(),
                h.finish()
            )
        })
        .collect()
}

#[test]
fn headless_a_saved_layer_comes_back_as_one_undo_step() {
    let (mut s, base) = painted(64);
    let before = fingerprint(&s);
    let revision = s.doc.revision();
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    assert_eq!(s.shelf.resources().len(), 1, "{}", s.message);
    let res = &s.shelf.resources()[0];
    assert_eq!(
        (res.kind.as_str(), res.name.as_str()),
        ("smartMaterial", "レイヤー 1")
    );
    assert!(s.modified && s.shelf.changed, "棚の変更は未保存にする");
    assert_eq!(s.doc.revision(), revision, "保存は文書を変えない");
    assert_eq!(fingerprint(&s), before);
    assert_eq!(s.shelf.selected.as_deref(), Some(res.id.as_str()));

    let id = res.id.clone();
    place(&mut s, &id);
    assert_eq!(s.doc.layers().len(), 2, "{}", s.message);
    let placed = s.selected_layer.unwrap();
    assert_ne!(placed, base, "置いたレイヤーは新しい ID");
    assert_eq!(pixel(&s, placed), [200, 40, 30, 255]);
    assert_eq!(s.doc.layers()[1].id(), placed, "選んでいたレイヤーのすぐ上");
    assert!(
        s.message.starts_with("置きました: レイヤー 1"),
        "{}",
        s.message
    );

    s.apply(Action::Undo);
    assert_eq!(fingerprint(&s), before, "1 回の取り消しで元に戻る");
    s.apply(Action::Redo);
    assert_eq!(s.doc.layers().len(), 2);
    assert_eq!(pixel(&s, s.doc.layers()[1].id()), [200, 40, 30, 255]);
}

#[test]
fn headless_a_group_is_saved_with_its_contents_and_placed_as_a_group() {
    let (mut s, _) = painted(32);
    s.apply(Action::M2(Edit::GroupSelected));
    let group = s.selected_layer.unwrap();
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(group)));
    let id = ids(&s, "smartMaterial").pop().expect(&s.message);
    s.apply(Action::NewLayer);
    let on_top = s.selected_layer.unwrap();
    place(&mut s, &id);
    // 元の 2 レイヤー + 置いた 2 レイヤー（グループとその中身）。新しいレイヤーは選んでいたレイヤーの上
    assert_eq!(s.doc.layers().len(), 3 + 2, "{}", s.message);
    let placed = s.selected_layer.unwrap();
    let layer = s.doc.layer(placed).unwrap();
    assert!(layer.is_group());
    assert_eq!(layer.parent(), None);
    let child = s.doc.children_of(Some(placed)).unwrap();
    assert_eq!(child.len(), 1);
    assert_eq!(pixel(&s, child[0]), [200, 40, 30, 255]);
    assert!(s.doc.layer_index(placed) > s.doc.layer_index(on_top));
    s.apply(Action::Undo);
    assert_eq!(s.doc.layers().len(), 3);
}

#[test]
fn headless_a_smart_mask_replaces_the_mask_and_undo_brings_the_old_one_back() {
    let (mut s, base) = painted(32);
    s.apply(Action::M2(Edit::AddMask(base)));
    s.doc.set_mask_pixel(base, 1, 1, 255).unwrap();
    s.apply(Action::Shelf(ShelfOp::SaveMask(base)));
    let id = ids(&s, "smartMask").pop().expect(&s.message);
    assert_eq!(
        s.shelf.resources().last().unwrap().name,
        "レイヤー 1 のマスク"
    );
    // 別のレイヤー（マスクなし）へ: マスクができる
    s.apply(Action::NewLayer);
    let other = s.selected_layer.unwrap();
    place(&mut s, &id);
    assert!(s.message.starts_with("マスクにしました"), "{}", s.message);
    assert_eq!(
        s.doc
            .layer(other)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .pixel(1, 1)
            .unwrap()
            .a,
        255
    );
    assert!(s.m2.edit_mask, "置いたらマスクに描く先を向ける");
    s.apply(Action::Undo);
    assert!(s.doc.layer(other).unwrap().mask().is_none());
    // 持っているマスクの入れ替え: 取り消すと前のマスク
    s.apply(Action::M2(Edit::AddMask(other)));
    s.doc.set_mask_pixel(other, 1, 1, 77).unwrap();
    s.apply(Action::Shelf(ShelfOp::Place {
        id: id.clone(),
        target: PlaceTarget::Mask(other),
    }));
    assert!(
        s.message.starts_with("マスクを入れ替えました"),
        "{}",
        s.message
    );
    assert_eq!(
        s.doc
            .layer(other)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .pixel(1, 1)
            .unwrap()
            .a,
        255
    );
    s.apply(Action::Undo);
    assert_eq!(
        s.doc
            .layer(other)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .pixel(1, 1)
            .unwrap()
            .a,
        77
    );
}

#[test]
fn headless_a_placement_target_chooses_the_group_and_the_position() {
    let (mut s, base) = painted(32);
    s.apply(Action::M2(Edit::GroupSelected));
    let group = s.selected_layer.unwrap();
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    let id = ids(&s, "smartMaterial").pop().unwrap();
    // グループの中の一番上へ
    s.apply(Action::Shelf(ShelfOp::Place {
        id: id.clone(),
        target: PlaceTarget::At {
            parent: Some(group),
            position: None,
        },
    }));
    let inside = s.doc.children_of(Some(group)).unwrap();
    assert_eq!(inside.len(), 2, "{}", s.message);
    assert_eq!(*inside.last().unwrap(), s.selected_layer.unwrap());
    // 最上位の一番下へ
    s.apply(Action::Shelf(ShelfOp::Place {
        id,
        target: PlaceTarget::At {
            parent: None,
            position: Some(0),
        },
    }));
    assert_eq!(s.selected_layer, Some(s.doc.layers()[0].id()));
}

#[test]
fn headless_a_different_sized_document_resamples_and_says_so() {
    let (mut s, base) = painted(32);
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    let id = ids(&s, "smartMaterial").pop().unwrap();
    // 別の大きさの文書（同じ棚）へ置く
    let mut big = AppState::new(64, 64);
    big.shelf = std::mem::take(&mut s.shelf);
    place(&mut big, &id);
    assert!(big.message.contains("32×32 から変更"), "{}", big.message);
    let placed = big.selected_layer.unwrap();
    assert_eq!(pixel(&big, placed), [200, 40, 30, 255]);
    big.apply(Action::Undo);
    assert_eq!(big.doc.layers().len(), 1);
    big.lang = Lang::En;
    place(&mut big, &id);
    assert!(
        big.message.contains("resized from 32×32"),
        "{}",
        big.message
    );
}

#[test]
fn headless_what_cannot_be_saved_or_placed_changes_nothing_and_says_why() {
    let (mut s, base) = painted(32);
    let before = fingerprint(&s);
    // 棚の予算を超える: 棚は空のまま・未保存にならない
    s.shelf = ShelfState::with_shelf(Shelf::new(1));
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    assert!(s.shelf.resources().is_empty());
    assert!(!s.modified && !s.shelf.changed);
    assert!(
        s.message.contains("アセットの予算を超えます"),
        "{}",
        s.message
    );
    s.lang = Lang::En;
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    assert!(s.message.contains("Over the asset budget"), "{}", s.message);
    s.lang = Lang::Ja;
    // ユーザーチャンネルを使うレイヤーは .ylsmart（形式 1）に書けない
    s.shelf = ShelfState::default();
    let channel = s
        .doc
        .add_channel(yolu_app::m2::new_channel_info(
            "AO".into(),
            yolu_app::engine::ChannelKind::Scalar,
        ))
        .unwrap();
    paint_channel(&mut s, base, channel, [200, 200, 200, 255]);
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    assert!(s.shelf.resources().is_empty());
    assert!(
        s.message.contains("ユーザーチャンネルを使っています"),
        "{}",
        s.message
    );
    s.doc.remove_channel(channel).unwrap();
    // マスクの無いレイヤーからはスマートマスクを作れない
    s.apply(Action::Shelf(ShelfOp::SaveMask(base)));
    assert!(s.shelf.resources().is_empty());
    assert!(s.message.contains("マスクがありません"), "{}", s.message);
    // 描いている間はどの操作も断る
    s.canvas.stroke = Some(StrokeSource::Mouse);
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    assert!(s.message.contains("描いている間"), "{}", s.message);
    s.canvas.stroke = None;
    // 読むだけのセットには置けない・そこからは保存しない
    s.sets.get_mut(0).unwrap().read_only = Some("試験".into());
    s.shelf = ShelfState::default();
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    assert!(s.shelf.resources().is_empty());
    assert!(s.message.contains("読むだけ"), "{}", s.message);
    s.sets.get_mut(0).unwrap().read_only = None;
    assert_eq!(fingerprint(&s), before, "断った操作は文書を変えない");
}

#[test]
fn headless_a_placement_over_the_pixel_budget_is_refused_whole() {
    let (mut s, base) = painted(64);
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    let id = ids(&s, "smartMaterial").pop().unwrap();
    let used = s.doc.allocated_bytes();
    s.doc.set_source_budget_bytes(used + 16).unwrap();
    let before = fingerprint(&s);
    place(&mut s, &id);
    assert_eq!(fingerprint(&s), before);
    assert!(
        s.message.contains("レイヤーのメモリの予算を超えます"),
        "{}",
        s.message
    );
}

#[test]
fn headless_brushes_are_listed_but_not_placed_yet_and_materials_are_placed_in_one_undo() {
    let mut s = AppState::new(16, 16);
    s.shelf = ShelfState::with_shelf(fixture_shelf());
    s.shelf.inspect_pending(100);
    let before = fingerprint(&s);
    for id in ids(&s, "brush") {
        place(&mut s, &id);
        assert!(
            s.message.contains("ブラシ") && s.message.contains("置けません"),
            "{}",
            s.message
        );
        assert_eq!(s.shelf.block_of(&id), Some(&Block::Kind(ItemKind::Brush)));
    }
    assert_eq!(fingerprint(&s), before);
    // マテリアル（中身は .ylsmart と同じ形。人工データは 2 レイヤー）は、スマートマテリアルと同じ道で置ける
    let material = ids(&s, "material").pop().unwrap();
    assert_eq!(s.shelf.block_of(&material), None);
    place(&mut s, &material);
    assert!(s.doc.layers().len() >= 3, "{}", s.message);
    assert!(s.message.starts_with("置きました: multi"), "{}", s.message);
    s.apply(Action::Undo);
    assert_eq!(fingerprint(&s), before);
    s.lang = Lang::En;
    let brush = ids(&s, "brush").pop().unwrap();
    place(&mut s, &brush);
    assert!(
        s.message.contains("Brush cannot be placed yet"),
        "{}",
        s.message
    );
}

#[test]
fn headless_an_image_is_placed_as_one_layer_stretched_to_the_document_in_one_undo() {
    // 2×1 の画像（左が透明で RGB を持ち、右がほぼ不透明）を 8×4 の文書へ
    let mut s = AppState::new(8, 4);
    s.shelf = ShelfState::with_shelf(fixture_shelf());
    let image = s
        .shelf
        .resources()
        .iter()
        .find(|r| r.kind == "image" && r.name == "透明画像")
        .unwrap()
        .clone();
    let before = fingerprint(&s);
    place(&mut s, &image.id);
    assert_eq!(s.doc.layers().len(), 2, "{}", s.message);
    let placed = s.selected_layer.unwrap();
    assert_eq!(s.doc.layer(placed).unwrap().name(), image.name);
    assert!(s.message.contains("から変更"), "{}", s.message);
    let layer = s.doc.layer(placed).unwrap();
    let opaque = (0..8)
        .filter(|&x| {
            layer
                .surface(Channel::Color)
                .unwrap()
                .pixel(x, 1)
                .unwrap()
                .a
                > 0
        })
        .count();
    assert!(opaque > 0 && opaque < 8, "右側だけが見える: {opaque}");
    s.apply(Action::Undo);
    assert_eq!(fingerprint(&s), before);
}

#[test]
fn headless_filter_and_search_pick_by_kind_and_name() {
    let mut shelf = ShelfState::with_shelf(fixture_shelf());
    assert_eq!(shelf.visible().len(), 6);
    shelf.filter = Some(ItemKind::Image);
    assert_eq!(shelf.visible().len(), 2);
    shelf.filter = Some(ItemKind::SmartMaterial);
    assert_eq!(shelf.visible().len(), 1);
    shelf.filter = Some(ItemKind::SmartMask);
    assert_eq!(shelf.visible().len(), 1);
    shelf.filter = None;
    shelf.search = "  RASTER ".into();
    let names: Vec<_> = shelf.visible().iter().map(|r| r.name.clone()).collect();
    assert_eq!(
        names,
        ["raster"],
        "大文字小文字を区別せず、前後の空白は無視"
    );
    shelf.search = "ブラ".into();
    assert_eq!(shelf.visible()[0].kind, "brush");
    shelf.search = "存在しない".into();
    assert!(shelf.visible().is_empty());
    shelf.filter = Some(ItemKind::Brush);
    shelf.search.clear();
    assert_eq!(shelf.visible().len(), 1);
}

#[test]
fn headless_items_are_inspected_once_for_thumbnails_details_and_blocks() {
    let mut shelf = ShelfState::with_shelf(fixture_shelf());
    // 1 回に数個ずつ見る（残りがあれば true）
    assert!(shelf.inspect_pending(2));
    assert!(shelf.inspect_pending(2));
    assert!(!shelf.inspect_pending(2));
    let ctx = egui::Context::default();
    for r in shelf.resources().to_vec() {
        let kind = ItemKind::of(&r.kind).unwrap();
        let thumb = shelf.texture(&ctx, &r.id);
        match kind {
            ItemKind::Brush => assert!(thumb.is_none(), "{}", r.name),
            _ => assert!(thumb.is_some(), "{}: サムネイルがある", r.name),
        }
        let block = shelf.block_of(&r.id);
        match kind {
            ItemKind::Brush => assert_eq!(block, Some(&Block::Kind(kind))),
            _ => assert!(block.is_none(), "{}: {block:?}", r.name),
        }
    }
    let raster = shelf
        .resources()
        .iter()
        .find(|r| r.name == "raster")
        .unwrap()
        .clone();
    let detail = shelf.detail(Lang::Ja, &raster);
    assert!(
        detail.contains("レイヤー") && detail.contains("×"),
        "{detail}"
    );
    assert!(shelf.detail(Lang::En, &raster).contains("layer"));
    // 2 度目は作り直さない
    assert!(!shelf.inspect_pending(100));
}

#[test]
fn headless_files_that_cannot_be_placed_say_why_and_broken_ones_are_refused() {
    let dir = temp_dir("import");
    let mut s = AppState::new(16, 16);
    // 画像を同梱した素材: 棚には入るが置けない（理由が出る・文書は変わらない）
    s.apply(Action::Shelf(ShelfOp::ImportFile(
        fixtures().join("images.ylsmart"),
    )));
    let id = ids(&s, "smartMaterial").pop().expect(&s.message);
    s.shelf.inspect_pending(10);
    assert_eq!(s.shelf.block_of(&id), Some(&Block::Images));
    let before = fingerprint(&s);
    place(&mut s, &id);
    assert!(s.message.contains("画像入りは置けません"), "{}", s.message);
    assert_eq!(fingerprint(&s), before);
    s.lang = Lang::En;
    place(&mut s, &id);
    assert!(s.message.contains("Contains images"), "{}", s.message);
    s.lang = Lang::Ja;
    // 壊れたファイル・短いファイル・無いファイル
    let good = std::fs::read(fixtures().join("raster.ylsmart")).unwrap();
    let count = s.shelf.resources().len();
    for (name, bytes) in [
        ("garbage.ylsmart", vec![7u8; 300]),
        ("short.ylsmart", good[..good.len() / 2].to_vec()),
        ("empty.ylsmart", vec![]),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        s.apply(Action::Shelf(ShelfOp::ImportFile(path)));
        assert!(
            s.message.contains("読めません") || s.message.contains("できません"),
            "{name}: {}",
            s.message
        );
        assert_eq!(s.shelf.resources().len(), count, "{name}: 棚は変わらない");
    }
    s.apply(Action::Shelf(ShelfOp::ImportFile(dir.join("none.ylsmart"))));
    assert!(s.message.contains("none.ylsmart"), "{}", s.message);
    assert_eq!(s.shelf.resources().len(), count);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_ylsmart_files_export_and_import_byte_for_byte() {
    let dir = temp_dir("export");
    let (mut s, base) = painted(32);
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    let id = ids(&s, "smartMaterial").pop().unwrap();
    // 書き出す前にファイルを選ぶウィンドウを頼む（ウィンドウを開かない試験では頼みが残る）
    s.apply(Action::Shelf(ShelfOp::ExportDialog(id.clone())));
    assert_eq!(s.dialog_request, Some(DialogRequest::ShelfExport));
    assert_eq!(s.shelf.export_id.as_deref(), Some(id.as_str()));
    let path = dir.join("木目.ylsmart");
    s.apply(Action::Shelf(ShelfOp::ExportFile {
        id: id.clone(),
        path: path.clone(),
    }));
    assert!(s.message.starts_with("書き出しました"), "{}", s.message);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes, s.shelf.shelf().content_bytes(&id).unwrap());
    let left: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("pending") || n.ends_with("tmp~"))
        .collect();
    assert!(left.is_empty(), "一時ファイルを残さない: {left:?}");
    // 別のプロジェクトへ読み込む: 同じ中身・名前・種類。もう一度は「すでにある」
    let mut other = AppState::new(32, 32);
    other.apply(Action::Shelf(ShelfOp::ImportFile(path.clone())));
    assert_eq!(other.shelf.resources().len(), 1, "{}", other.message);
    let got = &other.shelf.resources()[0];
    assert_eq!(
        (got.kind.as_str(), got.name.as_str()),
        ("smartMaterial", "レイヤー 1")
    );
    assert_eq!(other.shelf.shelf().content_bytes(&got.id).unwrap(), bytes);
    assert!(other.modified && other.shelf.changed);
    other.apply(Action::Shelf(ShelfOp::ImportFile(path.clone())));
    assert_eq!(other.shelf.resources().len(), 1);
    assert!(
        other.message.starts_with("すでにアセットにあります"),
        "{}",
        other.message
    );
    // 画像は書き出せない
    let mut s = AppState::new(8, 8);
    s.shelf = ShelfState::with_shelf(fixture_shelf());
    let image = ids(&s, "image").pop().unwrap();
    s.apply(Action::Shelf(ShelfOp::ExportFile {
        id: image,
        path: dir.join("x.ylsmart"),
    }));
    assert!(s.message.contains("スマート素材ではない"), "{}", s.message);
    assert!(!dir.join("x.ylsmart").exists());
    // 書き出し先に書けない（フォルダが無い）: 断る
    s.apply(Action::Shelf(ShelfOp::ExportFile {
        id: ids(&s, "smartMask").pop().unwrap(),
        path: dir.join("no-such-folder").join("m.ylsmart"),
    }));
    assert!(s.message.contains("に書き出せません（"), "{}", s.message);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_the_shelf_survives_save_and_reopen_in_every_kind() {
    let dir = temp_dir("roundtrip");
    let path = dir.join("shelf.ylp");
    let (mut s, base) = painted(32);
    // 画像・ブラシ・マテリアル（人工データ）に、このアプリで作った素材を足す
    s.shelf = ShelfState::with_shelf(fixture_shelf());
    s.shelf.changed = true;
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    s.apply(Action::M2(Edit::AddMask(base)));
    s.apply(Action::Shelf(ShelfOp::SaveMask(base)));
    assert_eq!(s.shelf.resources().len(), 8, "{}", s.message);
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    assert!(!s.shelf.changed, "保存したら未保存の印を下ろす");

    let bytes = std::fs::read(&path).unwrap();
    let project = Project::read(&bytes).unwrap();
    assert_eq!(project.resources().len(), 8);
    assert_eq!(project.info().saved_by.as_ref().unwrap().app, "YoluPainter");
    let mut again = AppState::new(8, 8);
    again.apply(Action::OpenProject(path.clone()));
    assert_eq!(again.shelf.resources().len(), 8, "{}", again.message);
    for r in s.shelf.resources() {
        let got = again
            .shelf
            .get(&r.id)
            .unwrap_or_else(|| panic!("{} が戻らない", r.name));
        assert_eq!(
            (&got.kind, &got.name, &got.content),
            (&r.kind, &r.name, &r.content)
        );
        assert_eq!(
            again.shelf.shelf().content_bytes(&r.id),
            s.shelf.shelf().content_bytes(&r.id),
            "{}: 中身が同じバイト列",
            r.name
        );
    }
    // 開き直した棚から置ける（文書は元の見た目と同じ）
    let material = ids(&again, "smartMaterial");
    assert_eq!(
        material.len(),
        2,
        "人工データの 1 つとこのアプリで作った 1 つ"
    );
    let saved = again
        .shelf
        .resources()
        .iter()
        .find(|r| r.kind == "smartMaterial" && r.name == "レイヤー 1")
        .unwrap()
        .id
        .clone();
    let selected = again.selected_layer.unwrap();
    place(&mut again, &saved);
    let placed = again.selected_layer.unwrap();
    assert_ne!(placed, selected);
    assert_eq!(
        pixel(&again, placed),
        [200, 40, 30, 255],
        "{}",
        again.message
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_an_unchanged_shelf_keeps_the_opened_files_resources_as_they_were() {
    // 棚に触らず文書だけ変えて保存しても、開いたファイルの resources は 1 バイトも変わらない。入力は、ほかのアプリが書いた
    // 形（余白・未知のキーのある resources.json。このアプリの正規の索引ではない）にする。
    // なお、この試験は「変えたときだけ棚を書く」門を外しても通る（読めた棚は原本のバイト列を持ち回るので）。門の効果は、
    // 読めなかった棚で resources を消さないこと（次の試験）と、変えていない棚でアーカイブを組み直さないこと
    let dir = temp_dir("keep");
    let path = dir.join("keep.ylp");
    let (mut s, base) = painted(32);
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    s.apply(Action::SaveProjectAs(path.clone()));
    let mut entries: BTreeMap<String, Vec<u8>> =
        yolu_io::Archive::read(&std::fs::read(&path).unwrap())
            .unwrap()
            .entries()
            .iter()
            .map(|(name, bytes)| (name.clone(), bytes.to_vec()))
            .collect();
    let index = String::from_utf8(entries["resources.json"].clone()).unwrap();
    let loose = format!(
        "\n\n  {}  \n",
        index.trim().replacen(
            "\"resources\"",
            "\"x-other-app\": [1, 2],\n  \"resources\"",
            1
        )
    );
    assert_ne!(loose.trim(), index.trim(), "正規でない索引にした");
    entries.insert("resources.json".into(), loose.clone().into_bytes());
    std::fs::write(
        &path,
        yolu_io::Archive::from_entries(entries)
            .unwrap()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    let first = Project::read(&std::fs::read(&path).unwrap()).unwrap();
    // 開いて、棚に触らず文書だけ変えて保存
    let mut again = AppState::new(8, 8);
    again.apply(Action::OpenProject(path.clone()));
    assert_eq!(again.shelf.resources().len(), 1, "{}", again.message);
    let layer = again.selected_layer.unwrap();
    paint(&mut again, layer, [1, 2, 3, 255]);
    again.apply(Action::NewLayer);
    assert!(!again.shelf.changed);
    again.apply(Action::SaveProject);
    assert!(
        again.message.starts_with("保存しました"),
        "{}",
        again.message
    );
    let second = Project::read(&std::fs::read(&path).unwrap()).unwrap();
    let entries = |p: &Project| -> BTreeMap<String, Vec<u8>> {
        p.migrated_entries()
            .iter()
            .filter(|(n, _)| n.starts_with("resources"))
            .map(|(n, b)| (n.clone(), b.bytes().unwrap().to_vec()))
            .collect()
    };
    assert_eq!(entries(&first), entries(&second));
    assert_eq!(
        entries(&second)["resources.json"],
        loose.as_bytes(),
        "索引は開いたときのバイト列のまま"
    );
    assert!(entries(&second).len() >= 2);
    // 前の版はバックアップに残る
    assert!(dir.join("keep.ylp-backups~").exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_removing_leaves_placed_layers_and_the_saved_file_loses_the_resource() {
    let dir = temp_dir("remove");
    let path = dir.join("remove.ylp");
    let (mut s, base) = painted(32);
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    let id = ids(&s, "smartMaterial").pop().unwrap();
    place(&mut s, &id);
    let layers = s.doc.layers().len();
    // 消す前に確かめるウィンドウを頼む（ウィンドウを開かない試験では頼みが残る。消すのは確かめた後）
    s.apply(Action::Shelf(ShelfOp::AskRemove(id.clone())));
    assert_eq!(s.dialog_request, Some(DialogRequest::ShelfRemove));
    assert_eq!(s.shelf.pending_remove.as_deref(), Some(id.as_str()));
    assert_eq!(s.shelf.resources().len(), 1, "確かめるまでは消さない");
    s.apply(Action::SaveProjectAs(path.clone()));
    assert_eq!(
        Project::read(&std::fs::read(&path).unwrap())
            .unwrap()
            .resources()
            .len(),
        1
    );
    s.apply(Action::Shelf(ShelfOp::Remove(id.clone())));
    assert!(s.shelf.resources().is_empty() && s.shelf.pending_remove.is_none());
    assert_eq!(s.doc.layers().len(), layers, "置いたレイヤーはそのまま");
    assert!(s.modified && s.shelf.changed);
    assert!(s.shelf.selected.is_none());
    s.apply(Action::SaveProject);
    let saved = Project::read(&std::fs::read(&path).unwrap()).unwrap();
    assert!(saved.resources().is_empty());
    assert!(!saved.migrated_entries().contains_key("resources.json"));
    // 無い ID を消しても何も起きない
    s.apply(Action::Shelf(ShelfOp::Remove("none".into())));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_a_new_project_starts_with_an_empty_shelf() {
    let (mut s, base) = painted(16);
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    assert_eq!(s.shelf.resources().len(), 1);
    s.apply(Action::NewProject);
    assert!(s.shelf.resources().is_empty() && !s.shelf.changed);
}

#[test]
fn headless_an_unreadable_shelf_is_not_modified_and_the_files_resources_stay() {
    let dir = temp_dir("broken");
    let path = dir.join("broken.ylp");
    let (mut s, base) = painted(16);
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    s.apply(Action::SaveProjectAs(path.clone()));
    let before = Project::read(&std::fs::read(&path).unwrap()).unwrap();
    let mut again = AppState::new(8, 8);
    again.apply(Action::OpenProject(path.clone()));
    // 読めなかった棚（開くときに `from_project` が作る状態。本物のファイルは `Project::read` が受けたものを `Shelf::read` も
    // 同じ検証で読めるので、読めない棚は結果を差し替えて作る）にする。棚を変える操作は断り、保存は resources を残す
    again.shelf =
        ShelfState::from_read(Err::<Shelf, _>(yolu_io::Error::InvalidData("試験".into())));
    assert_eq!(
        again.shelf.unavailable.as_ref().map(|u| u.reason(Lang::Ja)),
        Some("試験")
    );
    let layer = again.selected_layer.unwrap();
    again.apply(Action::Shelf(ShelfOp::SaveMaterial(layer)));
    assert!(again.shelf.resources().is_empty());
    assert!(
        again
            .message
            .contains("プロジェクトのアセットを読めなかったので、変えられません"),
        "{}",
        again.message
    );
    again.apply(Action::NewLayer);
    again.apply(Action::SaveProject);
    let after = Project::read(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        after.resources().len(),
        before.resources().len(),
        "元の resources は残る"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_a_document_built_by_core_matches_what_the_shelf_places() {
    // 棚に入れて置いた結果は、core の捕まえ方・置き方（同じ文書へ）と画素が一致する
    let (mut s, base) = painted(32);
    s.apply(Action::M2(Edit::NewFill));
    let fill = s.selected_layer.unwrap();
    let mut reference = Document::new(32, 32).unwrap();
    let a = reference.add_layer("レイヤー 1").unwrap();
    let ts = reference.tile_size() as usize;
    let mut bytes = vec![0u8; ts * ts * 4];
    for y in 0..32usize.min(ts) {
        for x in 0..32usize.min(ts) {
            bytes[(y * ts + x) * 4..][..4].copy_from_slice(&[200, 40, 30, 255]);
        }
    }
    reference
        .import_tile(a, Channel::Color, TileCoord::new(0, 0), &bytes)
        .unwrap();
    let material = reference.capture_smart_material(&[a], "x").unwrap();
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    let id = ids(&s, "smartMaterial").pop().unwrap();
    s.selected_layer = Some(fill);
    place(&mut s, &id);
    let placed = s.selected_layer.unwrap();
    reference
        .place_smart_material(&material, &Default::default())
        .unwrap();
    let want = reference
        .layers()
        .last()
        .unwrap()
        .surface(Channel::Color)
        .unwrap()
        .to_canvas_bytes();
    let got = s
        .doc
        .layer(placed)
        .unwrap()
        .surface(Channel::Color)
        .unwrap()
        .to_canvas_bytes();
    assert_eq!(got, want);
}

#[test]
fn headless_a_large_material_is_written_on_another_thread_and_kept_when_done() {
    let (mut s, base) = painted(32);
    s.shelf.async_bytes = 0; // 小さな素材でも別のスレッドへ
    s.shelf.hold_saves(true);
    let revision = s.doc.revision();
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    assert_eq!(s.shelf.saving_name(), Some("レイヤー 1"));
    assert!(s.message.starts_with("保存中"), "{}", s.message);
    assert!(
        s.shelf.resources().is_empty() && !s.modified,
        "終わるまで棚は変わらない"
    );
    // 書き出している間は 2 つ目を始めない。ほかの操作はできる（文書へ置く・描く）
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    assert!(s.message.contains("保存中です"), "{}", s.message);
    s.shelf_poll();
    assert!(s.shelf.saving_name().is_some(), "まだ終わっていない");
    s.shelf.hold_saves(false);
    s.shelf_wait();
    assert_eq!(s.shelf.resources().len(), 1, "{}", s.message);
    assert!(
        s.message.starts_with("アセットに入れました"),
        "{}",
        s.message
    );
    let id = s.shelf.resources()[0].id.clone();
    assert_eq!(s.shelf.selected.as_deref(), Some(id.as_str()));
    assert!(
        s.shelf.info(&id).is_some(),
        "サムネイルと説明は書き出したスレッドで作ってある"
    );
    assert!(s.modified && s.shelf.changed);
    assert_eq!(s.doc.revision(), revision, "保存は文書を変えない");
    place(&mut s, &id);
    assert_eq!(pixel(&s, s.selected_layer.unwrap()), [200, 40, 30, 255]);
}

#[test]
fn headless_cancelling_a_save_keeps_it_off_the_shelf_and_a_refusal_arrives_later() {
    let (mut s, base) = painted(16);
    s.shelf.async_bytes = 0;
    s.shelf.hold_saves(true);
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    s.apply(Action::Shelf(ShelfOp::CancelSave));
    assert!(s.shelf.saving_name().is_none());
    assert!(s.message.starts_with("保存をやめました"), "{}", s.message);
    s.shelf.hold_saves(false);
    s.shelf.wait_idle();
    s.shelf_poll();
    assert!(
        s.shelf.resources().is_empty() && !s.modified,
        "取り消した保存は棚に入れない"
    );
    // やめたあとは新しい保存を始められる
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    s.shelf_wait();
    assert_eq!(s.shelf.resources().len(), 1);
    // 書き出しで断られる素材（ユーザーチャンネル）は、断りが終わったときに届く
    let channel = s
        .doc
        .add_channel(yolu_app::m2::new_channel_info(
            "AO".into(),
            yolu_app::engine::ChannelKind::Scalar,
        ))
        .unwrap();
    paint_channel(&mut s, base, channel, [9, 9, 9, 255]);
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    s.shelf_wait();
    assert_eq!(s.shelf.resources().len(), 1);
    assert!(
        s.message.contains("レイヤー 1") && s.message.contains("ユーザーチャンネルを使っています"),
        "{}",
        s.message
    );
}

#[test]
fn headless_a_save_that_finishes_after_the_project_changed_is_dropped() {
    let (mut s, base) = painted(16);
    s.shelf.async_bytes = 0;
    s.shelf.hold_saves(true);
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    let running = s.shelf.saves_running();
    s.apply(Action::NewProject);
    assert_eq!(s.shelf.saves_running(), running, "書き出しはまだ走っている");
    s.shelf.hold_saves(false);
    s.shelf.wait_idle();
    s.shelf_poll();
    assert!(s.shelf.resources().is_empty() && s.shelf.saving_name().is_none());
}

#[test]
fn headless_the_menus_name_the_shelf_actions_in_both_languages() {
    use yolu_app::state::PopupKind;
    use yolu_app::ui::menu::Entry;
    let labels = |entries: Vec<Entry<Action>>| -> Vec<String> {
        entries
            .into_iter()
            .filter_map(|e| match e {
                Entry::Item { label, .. } => Some(label),
                _ => None,
            })
            .collect()
    };
    let (mut s, base) = painted(16);
    s.apply(Action::M2(Edit::AddMask(base)));
    s.shelf = ShelfState::with_shelf(fixture_shelf());
    s.shelf.selected = Some(ids(&s, "smartMaterial").pop().unwrap());
    let layer_menu = |s: &AppState| {
        labels(yolu_app::shell::popup_entries(
            s,
            PopupKind::LayerContext(base),
        ))
    };
    let shelf_menu = |s: &AppState| labels(yolu_app::shell::popup_entries(s, PopupKind::Shelf));
    let ja = layer_menu(&s);
    assert!(
        ja.contains(&"スマートマテリアルとして保存".to_owned()),
        "{ja:?}"
    );
    assert!(
        ja.contains(&"マスクをスマートマスクとして保存".to_owned()),
        "{ja:?}"
    );
    assert_eq!(
        shelf_menu(&s),
        [
            "置く",
            "書き出す…",
            "ライブラリへ入れる",
            "アセットから消す…"
        ]
    );
    s.shelf.selected = Some(ids(&s, "smartMask").pop().unwrap());
    assert_eq!(shelf_menu(&s)[0], "マスクに適用");
    s.lang = Lang::En;
    let en = layer_menu(&s);
    assert!(en.contains(&"Save as Smart Material".to_owned()), "{en:?}");
    assert!(en.contains(&"Save Mask as Smart Mask".to_owned()), "{en:?}");
    assert_eq!(
        shelf_menu(&s),
        [
            "Apply to Mask",
            "Export…",
            "Put into the library",
            "Remove from the project's assets…"
        ]
    );
    // レイヤーにマスクが無ければ、マスクの保存は出さない
    s.apply(Action::M2(Edit::RemoveMask(base)));
    assert!(!layer_menu(&s).iter().any(|l| l.contains("Mask as Smart")));
}

#[test]
fn headless_cancelling_a_save_keeps_the_next_big_write_waiting_until_the_first_thread_ends() {
    let (mut s, base) = painted(16);
    s.shelf.async_bytes = 0;
    s.shelf.hold_saves(true);
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    assert_eq!(s.shelf.saves_running(), 1);
    s.apply(Action::Shelf(ShelfOp::CancelSave));
    assert!(s.shelf.saving_name().is_none());
    assert_eq!(
        s.shelf.saves_running(),
        1,
        "やめても、書き出しのスレッドは終わるまで数える"
    );
    // やめた直後の保存は断る（2 つ目のスレッドを作らない）
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    assert!(
        s.message.contains("やめた保存を終えています"),
        "{}",
        s.message
    );
    assert!(s.shelf.saving_name().is_none());
    assert_eq!(s.shelf.saves_running(), 1, "スレッドは積み上がらない");
    // 断るのは、写しを捕まえる前（マスクの無いレイヤーでも、理由は書き出しの待ち）
    s.apply(Action::Shelf(ShelfOp::SaveMask(base)));
    assert!(
        s.message.contains("やめた保存を終えています"),
        "{}",
        s.message
    );
    s.lang = Lang::En;
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    assert!(
        s.message.contains("Finishing the cancelled save"),
        "{}",
        s.message
    );
    s.lang = Lang::Ja;
    // 書き出しが終われば、次の保存を始められる（やめた分の結果は棚に入らない）
    s.shelf.hold_saves(false);
    s.shelf.wait_idle();
    s.shelf_poll();
    assert!(s.shelf.resources().is_empty() && !s.modified);
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    s.shelf_wait();
    assert_eq!(s.shelf.resources().len(), 1, "{}", s.message);
    s.shelf.wait_idle();
    assert_eq!(s.shelf.saves_running(), 0);
}

#[test]
fn headless_a_running_save_is_counted_across_a_new_or_opened_project() {
    let dir = temp_dir("running");
    let path = dir.join("running.ylp");
    let (mut s, base) = painted(16);
    s.apply(Action::SaveProjectAs(path.clone()));
    s.shelf.async_bytes = 0;
    s.shelf.hold_saves(true);
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    for open in [false, true] {
        s.apply(if open {
            Action::OpenProject(path.clone())
        } else {
            Action::NewProject
        });
        assert!(s.shelf.saving_name().is_none(), "替えたら待ちは捨てる");
        assert_eq!(s.shelf.saves_running(), 1, "替えても前のスレッドは数える");
        let layer = s.selected_layer.unwrap();
        s.shelf.async_bytes = 0;
        s.apply(Action::Shelf(ShelfOp::SaveMaterial(layer)));
        assert!(
            s.message.contains("やめた保存を終えています"),
            "open={open}: {}",
            s.message
        );
    }
    s.shelf.hold_saves(false);
    s.shelf.wait_idle();
    let layer = s.selected_layer.unwrap();
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(layer)));
    s.shelf_wait();
    assert_eq!(s.shelf.resources().len(), 1, "{}", s.message);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_importing_several_files_keeps_every_refusal_in_the_one_message() {
    let dir = temp_dir("several");
    let bad = dir.join("bad.ylsmart");
    std::fs::write(&bad, vec![7u8; 300]).unwrap();
    let good = |name: &str| fixtures().join(name);
    let mut s = AppState::new(16, 16);
    // 良・不良・良: 入れた件数と、断ったファイルの名前と理由が 1 つの知らせに残る（後ろの成功で消えない）
    s.apply(Action::Shelf(ShelfOp::ImportFiles(vec![
        good("raster.ylsmart"),
        bad.clone(),
        good("mask.ylsmart"),
    ])));
    assert_eq!(s.shelf.resources().len(), 2, "{}", s.message);
    assert!(
        s.message
            .starts_with("「bad.ylsmart」はスマート素材として読めません（")
            && s.message.contains("2 件をアセットに入れました。"),
        "{}",
        s.message
    );
    // 英語
    s.lang = Lang::En;
    s.apply(Action::Shelf(ShelfOp::ImportFiles(vec![
        bad.clone(),
        good("raster.ylsmart"),
        good("multi.ylsmart"),
    ])));
    assert_eq!(s.shelf.resources().len(), 3, "{}", s.message);
    // 3 つのうち raster はすでにあり、multi だけが新しく入る（すでにあった分を「入れました」と数えない）
    assert!(
        s.message
            .starts_with("\"bad.ylsmart\" is not a readable smart asset (")
            && s.message.ends_with(
                "Added 1 to the project's assets. 1 was already in the project's assets."
            ),
        "{}",
        s.message
    );
    // 断りがあって、すべてすでにあったとき: 「入れました」とは言わず、すでにあった件数だけ
    s.lang = Lang::Ja;
    s.apply(Action::Shelf(ShelfOp::ImportFiles(vec![
        bad.clone(),
        good("raster.ylsmart"),
        good("multi.ylsmart"),
    ])));
    assert!(
        s.message
            .starts_with("「bad.ylsmart」はスマート素材として読めません（")
            && s.message.ends_with("2 件はすでにアセットにありました。")
            && !s.message.contains("をアセットに入れました"),
        "{}",
        s.message
    );
    // 断ったものが複数なら全部並べる。成功が無ければ件数は付けない
    s.apply(Action::Shelf(ShelfOp::ImportFiles(vec![
        bad.clone(),
        dir.join("none.ylsmart"),
    ])));
    assert!(
        s.message.contains("bad.ylsmart")
            && s.message.contains("none.ylsmart")
            && !s.message.contains("をアセットに入れました"),
        "{}",
        s.message
    );
    // 断りが無ければ件数だけ。1 つなら名前（今までどおり）。すでにあるものは別に数える
    s.apply(Action::Shelf(ShelfOp::ImportFiles(vec![
        good("raster.ylsmart"),
        good("images.ylsmart"),
    ])));
    assert_eq!(
        s.message, "1 件をアセットに入れました。1 件はすでにアセットにありました。",
        "{}",
        s.message
    );
    s.apply(Action::Shelf(ShelfOp::ImportFiles(vec![good(
        "images.ylsmart",
    )])));
    assert!(
        s.message.starts_with("すでにアセットにあります"),
        "{}",
        s.message
    );
    // 読めない棚には、まとめてでも足さない
    s.shelf = ShelfState::unreadable("試験");
    s.apply(Action::Shelf(ShelfOp::ImportFiles(vec![good(
        "raster.ylsmart",
    )])));
    assert!(
        s.message
            .contains("プロジェクトのアセットを読めなかったので、変えられません"),
        "{}",
        s.message
    );
    assert!(s.shelf.resources().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_the_import_size_limit_refuses_one_byte_over_and_not_at_the_limit() {
    let path = fixtures().join("raster.ylsmart");
    let len = std::fs::metadata(&path).unwrap().len();
    let mut s = AppState::new(16, 16);
    s.shelf.import_limit = len - 1;
    s.apply(Action::Shelf(ShelfOp::ImportFile(path.clone())));
    assert!(s.shelf.resources().is_empty());
    assert!(
        s.message.contains("raster.ylsmart") && s.message.contains("大きすぎます"),
        "{}",
        s.message
    );
    s.lang = Lang::En;
    s.apply(Action::Shelf(ShelfOp::ImportFile(path.clone())));
    assert!(s.message.contains("Too large"), "{}", s.message);
    s.shelf.import_limit = len;
    s.apply(Action::Shelf(ShelfOp::ImportFile(path)));
    assert_eq!(s.shelf.resources().len(), 1, "{}", s.message);
}

#[test]
fn headless_a_smart_mask_without_a_layer_says_the_state_not_an_instruction() {
    let mut s = AppState::new(16, 16);
    s.shelf = ShelfState::with_shelf(fixture_shelf());
    let mask = ids(&s, "smartMask").pop().unwrap();
    s.selected_layer = None;
    place(&mut s, &mask);
    assert!(
        s.message.contains("選んだレイヤーがありません"),
        "{}",
        s.message
    );
    s.lang = Lang::En;
    place(&mut s, &mask);
    assert!(s.message.contains("No layer selected"), "{}", s.message);
}

#[test]
fn headless_an_image_placed_at_its_own_size_keeps_every_byte_including_transparent_rgb() {
    // 人工データの 2×1（左は透明で RGB を持つ）と、その場で作る 5×3（透明・半透明・不透明が RGB を変えて混ざる。
    // 左下原点・straight RGBA8）を、同じ大きさの文書へ。置いたレイヤーの Color は元の画素と 1 バイトも違わない
    let mut synthetic = Vec::new();
    for y in 0..3u8 {
        for x in 0..5u8 {
            let alpha = [0u8, 0, 1, 128, 255][((x + y) % 5) as usize];
            synthetic.extend([x * 40 + 3, y * 70 + 5, 250 - x * 17 - y, alpha]);
        }
    }
    // 棚の索引と PNG（上の行が先）を手で組んで読む（serde_json を依存に足さない）
    let content = yolu_io::shelf::image_hash(&synthetic, 5, 3).unwrap();
    let top_first: Vec<u8> = synthetic.chunks(5 * 4).rev().flatten().copied().collect();
    let mut png = Vec::new();
    image::RgbaImage::from_raw(5, 3, top_first)
        .unwrap()
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let id = "10000000-0000-0000-0000-0000000000aa";
    let index = format!(
        r#"{{"resources":[{{"id":"{id}","kind":"image","name":"合成","content":"{content}","width":5,"height":3,"colorSpace":"srgb","origin":{{"type":"none"}}}}]}}"#
    );
    let files: BTreeMap<String, Arc<[u8]>> = BTreeMap::from([
        ("resources.json".into(), Arc::from(index.into_bytes())),
        (format!("resources/{content}.png"), Arc::from(png)),
    ]);
    let shelf = Shelf::read(&files, SHELF_BUDGET).unwrap();
    let mut s = AppState::new(5, 3);
    s.shelf = ShelfState::with_shelf(shelf);
    place(&mut s, id);
    assert_eq!(s.doc.layers().len(), 2, "{}", s.message);
    assert!(!s.message.contains("から変更"), "{}", s.message);
    let layer = s.doc.layer(s.selected_layer.unwrap()).unwrap();
    for y in 0..3u32 {
        for x in 0..5u32 {
            let at = ((y * 5 + x) * 4) as usize;
            let got = layer
                .surface(Channel::Color)
                .unwrap()
                .pixel(x, y)
                .unwrap()
                .to_array();
            assert_eq!(got, synthetic[at..at + 4], "({x}, {y})");
        }
    }
    assert!(
        synthetic
            .chunks(4)
            .any(|p| p[3] == 0 && p[..3] != [0, 0, 0]),
        "試験の画像は、透明で RGB を持つ画素を含む"
    );
    // 人工データの PNG（2×1）。PNG を直に読んだ画素と一致する
    let mut fixture = AppState::new(2, 1);
    fixture.shelf = ShelfState::with_shelf(fixture_shelf());
    let image = fixture
        .shelf
        .resources()
        .iter()
        .find(|r| r.kind == "image" && r.name == "透明画像")
        .unwrap()
        .id
        .clone();
    let png = fixture
        .shelf
        .shelf()
        .content_bytes(&image)
        .unwrap()
        .to_vec();
    let want = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
        .unwrap()
        .to_rgba8();
    assert!(
        want.pixels().any(|p| p.0[3] == 0 && p.0[..3] != [0, 0, 0]),
        "人工データにも、透明で RGB を持つ画素がある"
    );
    place(&mut fixture, &image);
    let layer = fixture.doc.layer(fixture.selected_layer.unwrap()).unwrap();
    for x in 0..2u32 {
        let got = layer
            .surface(Channel::Color)
            .unwrap()
            .pixel(x, 0)
            .unwrap()
            .to_array();
        assert_eq!(got, want.get_pixel(x, 0).0, "x = {x}");
    }
}

#[test]
fn headless_a_material_over_the_preview_budget_shows_an_icon_and_can_still_be_placed() {
    let ctx = egui::Context::default();
    let mut s = AppState::new(8, 8);
    s.shelf = ShelfState::with_shelf(fixture_shelf());
    s.shelf.inspect_pending(100);
    let material = ids(&s, "smartMaterial").pop().unwrap();
    let mask = ids(&s, "smartMask").pop().unwrap();
    for id in [&material, &mask] {
        assert!(
            s.shelf.texture(&ctx, id).is_some(),
            "既定の予算ではサムネイルが出る"
        );
    }
    // 展開の予算を超える素材: アイコン（サムネイル無し）で、置けない印は付けず、大きさ・レイヤーの数は出る
    let mut over = AppState::new(8, 8);
    over.shelf = ShelfState::with_shelf(fixture_shelf());
    over.shelf.preview_budget = 1;
    over.shelf.inspect_pending(100);
    for id in [&material, &mask] {
        assert_eq!(
            over.shelf.block_of(id),
            None,
            "予算超えは置けない扱いにしない"
        );
        assert!(over.shelf.texture(&ctx, id).is_none());
    }
    let info = over.shelf.info(&material).unwrap();
    assert!(info.width > 0 && info.layers > 0);
    place(&mut over, &material);
    assert!(over.message.starts_with("置きました"), "{}", over.message);
    assert_eq!(over.doc.layers().len(), 2);
    // 予算を超えていても、本当に置けない理由（画像入り）は残る
    over.apply(Action::Shelf(ShelfOp::ImportFile(
        fixtures().join("images.ylsmart"),
    )));
    over.shelf.inspect_pending(100);
    let images = ids(&over, "smartMaterial")
        .into_iter()
        .find(|id| *id != material)
        .unwrap();
    assert_eq!(over.shelf.block_of(&images), Some(&Block::Images));
}

#[test]
fn headless_an_image_over_the_thumbnail_pixel_limit_shows_an_icon_and_can_still_be_placed() {
    let ctx = egui::Context::default();
    let mut s = AppState::new(8, 4);
    s.shelf = ShelfState::with_shelf(fixture_shelf());
    s.shelf.inspect_pending(100);
    let image = ids(&s, "image").remove(0);
    assert!(s.shelf.texture(&ctx, &image).is_some());
    let mut over = AppState::new(8, 4);
    over.shelf = ShelfState::with_shelf(fixture_shelf());
    over.shelf.image_thumb_pixels = 1;
    over.shelf.inspect_pending(100);
    // 2×1 は超える。1×1 は超えない
    assert!(over.shelf.texture(&ctx, &image).is_none());
    assert_eq!(over.shelf.block_of(&image), None);
    let info = over.shelf.info(&image).unwrap();
    assert_eq!((info.width, info.height), (2, 1));
    let small = ids(&over, "image").pop().unwrap();
    assert!(
        over.shelf.texture(&ctx, &small).is_some(),
        "上限ちょうどは出す"
    );
    place(&mut over, &image);
    assert!(over.message.starts_with("置きました"), "{}", over.message);
}

#[test]
fn headless_every_core_refusal_the_shelf_words_comes_from_a_real_core_call() {
    use yolu_app::engine::CoreError;
    use yolu_core::smart::SmartPlacement;
    // 本物の core の呼び出しから断りを取る（core の文言が変わると、ここが落ちる）
    let (mut s, base) = painted(8);
    let mut cases: Vec<(CoreError, &str, &str)> = Vec::new();
    // チャンネルが合わない: 配置先に無いユーザーチャンネルを持つ素材
    let channel = s
        .doc
        .add_channel(yolu_app::m2::new_channel_info(
            "AO".into(),
            yolu_app::engine::ChannelKind::Scalar,
        ))
        .unwrap();
    paint_channel(&mut s, base, channel, [9, 9, 9, 255]);
    let with_user_channel = s.doc.capture_smart_material(&[base], "x").unwrap();
    let mut plain = Document::new(8, 8).unwrap();
    cases.push((
        plain
            .place_smart_material(&with_user_channel, &SmartPlacement::default())
            .unwrap_err(),
        "チャンネルが合いません",
        "Channels do not match",
    ));
    s.doc.remove_channel(channel).unwrap();
    let material = s.doc.capture_smart_material(&[base], "x").unwrap();
    // 画素の予算
    let used = s.doc.allocated_bytes();
    s.doc.set_source_budget_bytes(used + 16).unwrap();
    cases.push((
        s.doc
            .place_smart_material(&material, &SmartPlacement::default())
            .unwrap_err(),
        "レイヤーのメモリの予算を超えます",
        "Over the Layer memory budget",
    ));
    // レイヤーは 2048 まで
    let mut full = Document::new(8, 8).unwrap();
    while full.layers().len() < 2048 {
        full.add_group("g", None).unwrap();
    }
    cases.push((
        full.place_smart_material(&material, &SmartPlacement::default())
            .unwrap_err(),
        "レイヤーが多すぎます",
        "Too many layers",
    ));
    // マスクが無い・レイヤーが無い・名前が使えない・置き先がグループでない
    cases.push((
        s.doc.capture_smart_mask(base, "x").unwrap_err(),
        "マスクがありません",
        "No mask",
    ));
    cases.push((
        s.doc.capture_smart_material(&[], "x").unwrap_err(),
        "レイヤーがありません",
        "Layer not found",
    ));
    cases.push((
        s.doc
            .capture_smart_material(&[LayerId(0xdead)], "x")
            .unwrap_err(),
        "レイヤーがありません",
        "Layer not found",
    ));
    cases.push((
        s.doc.capture_smart_material(&[base], "  ").unwrap_err(),
        "名前が使えません",
        "Name not allowed",
    ));
    s.doc.set_source_budget_bytes(1 << 30).unwrap();
    cases.push((
        s.doc
            .place_smart_material(
                &material,
                &SmartPlacement {
                    parent: Some(base),
                    ..SmartPlacement::default()
                },
            )
            .unwrap_err(),
        "置き先がグループではありません",
        "Target is not a group",
    ));
    // 棚の断りも、ほかの操作と同じ core の誤りの文（`Lang::core_error`。同じ誤りは 1 つの文）
    for (error, ja, en) in &cases {
        assert_eq!(Lang::Ja.core_error(error), *ja, "{error:?}");
        assert_eq!(Lang::En.core_error(error), *en, "{error:?}");
        assert_ne!(Lang::Ja.core_error(error), error.to_string(), "{error:?}");
    }
    assert_eq!(cases.len(), 8);
    // 個別の文の無い断りは core の文のまま
    assert_eq!(
        Lang::Ja.core_error(&CoreError::BatchActive),
        CoreError::BatchActive.to_string()
    );
}

/// 文言の表: yolu-io の断りは文言（定数）を含むかどうかで見分けるので、定数を直に通して、日英の短い文に変わることを確かめる。
/// 本物の呼び出しから断りを取る確かめは次の試験（アーカイブの大きさの上限だけは、本物を作ると棚が 768 MiB を超えるので、この表だけ）。
#[test]
fn headless_every_io_refusal_the_shelf_words_has_a_short_sentence_in_both_languages() {
    use yolu_app::lang::shelf_io_error as io_reason;
    use yolu_io::shelf::{REFUSAL_ARCHIVE_BUDGET, REFUSAL_MEMORY_BUDGET, REFUSAL_RESOURCE_COUNT};
    use yolu_io::smart::{
        REFUSAL_GENERATORS, REFUSAL_IMAGES, REFUSAL_NEW_FILTERS, REFUSAL_RUST_GENERATORS,
        REFUSAL_USER_CHANNELS,
    };
    let cases = [
        (REFUSAL_IMAGES, "画像入りは置けません", "Contains images"),
        (
            REFUSAL_GENERATORS,
            "ジェネレーター付きは置けません",
            "Has pinned generators",
        ),
        (
            REFUSAL_MEMORY_BUDGET,
            "アセットの予算を超えます",
            "Over the asset budget",
        ),
        (
            REFUSAL_ARCHIVE_BUDGET,
            "ファイルの大きさの上限を超えます",
            "Over the file size limit",
        ),
        (
            REFUSAL_RESOURCE_COUNT,
            "アセットがいっぱいです",
            "Assets are full",
        ),
        (
            REFUSAL_USER_CHANNELS,
            "ユーザーチャンネルを使っています",
            "It uses user channels",
        ),
        (
            REFUSAL_RUST_GENERATORS,
            "ノイズ・グランジを使っています",
            "It uses Noise and Grunge",
        ),
        (
            REFUSAL_NEW_FILTERS,
            "0.5.0 で追加したフィルター・ジェネレーターを使っています",
            "It uses filters or generators added in 0.5.0",
        ),
    ];
    for (message, ja, en) in cases {
        let error = yolu_io::Error::InvalidData(message.to_owned());
        assert_eq!(io_reason(Lang::Ja, &error), ja, "{message}");
        assert_eq!(io_reason(Lang::En, &error), en, "{message}");
        // 予算の種類で返っても、ほかの文に包まれても（読み込みの失敗はレイヤーや場所を添える）見分ける
        let budget = yolu_io::Error::Budget(message.to_owned());
        assert_eq!(io_reason(Lang::En, &budget), en, "{message}");
        let wrapped = yolu_io::Error::InvalidData(format!("layers[0]「x」: {message}"));
        assert_eq!(io_reason(Lang::Ja, &wrapped), ja, "{message}");
    }
    // 知らない断りは、日本語は yolu-io の診断のまま、英語は種類ごとの短い文（日本語の診断を英語の画面に出さない）
    let other = yolu_io::Error::InvalidData("未知の失敗".into());
    assert_eq!(io_reason(Lang::Ja, &other), "未知の失敗");
    assert_eq!(
        io_reason(Lang::En, &other),
        "Invalid or unsupported project data"
    );
    let other = yolu_io::Error::Budget("未知の予算".into());
    assert_eq!(
        io_reason(Lang::En, &other),
        "Size, count or memory limit exceeded"
    );
}

/// 本物の呼び出しから断りを取る（io の文言が変わると、ここが落ちる）。画像入り・Generator の再固定・棚の予算・個数の上限・
/// ユーザーチャンネル・レイヤーのロック。
#[test]
fn headless_the_io_refusals_a_shelf_can_really_hit_come_from_real_calls() {
    use yolu_app::lang::shelf_io_error as io_reason;
    use yolu_io::shelf::ResourceKind;
    use yolu_io::smart::SmartFile;
    let raster = std::fs::read(fixtures().join("raster.ylsmart")).unwrap();
    let to_core = |bytes: &[u8]| SmartFile::read(bytes).unwrap().to_core().unwrap_err();
    // 画像入り
    let error = to_core(&std::fs::read(fixtures().join("images.ylsmart")).unwrap());
    assert_eq!(io_reason(Lang::Ja, &error), "画像入りは置けません");
    assert_eq!(io_reason(Lang::En, &error), "Contains images");
    // Generator の再固定（実物: フィルターを Generator に書き換え、索引の repin に入れた素材）
    let error = to_core(&generator_pinned_bytes());
    assert_eq!(
        io_reason(Lang::Ja, &error),
        "ジェネレーター付きは置けません"
    );
    assert_eq!(io_reason(Lang::En, &error), "Has pinned generators");
    // 棚の予算
    let mut tiny = Shelf::new(1);
    let error = tiny
        .add_file_without_origin(
            "20000000-0000-0000-0000-0000000000bb",
            "x",
            ResourceKind::SmartMaterial,
            &raster,
        )
        .unwrap_err();
    assert_eq!(io_reason(Lang::Ja, &error), "アセットの予算を超えます");
    assert_eq!(io_reason(Lang::En, &error), "Over the asset budget");
    // 個数の上限（256 個の棚への 257 個目）。予算とは別の理由
    let mut full = shelf_of_images(MAX_RESOURCES);
    let error = full
        .add_file_without_origin(
            "20000000-0000-0000-0000-0000000000cc",
            "x",
            ResourceKind::SmartMaterial,
            &raster,
        )
        .unwrap_err();
    assert_eq!(io_reason(Lang::Ja, &error), "アセットがいっぱいです");
    assert_eq!(io_reason(Lang::En, &error), "Assets are full");
    // ユーザーチャンネルを持つレイヤーの保存
    let (mut s, base) = painted(8);
    let channel = s
        .doc
        .add_channel(yolu_app::m2::new_channel_info(
            "AO".into(),
            yolu_app::engine::ChannelKind::Scalar,
        ))
        .unwrap();
    paint_channel(&mut s, base, channel, [9, 9, 9, 255]);
    let material = s.doc.capture_smart_material(&[base], "x").unwrap();
    let error = SmartFile::from_core(&material, &yolu_app::project::writer()).unwrap_err();
    assert_eq!(
        io_reason(Lang::Ja, &error),
        "ユーザーチャンネルを使っています"
    );
    assert_eq!(io_reason(Lang::En, &error), "It uses user channels");
    // レイヤーのロックを持つレイヤーは保存できる（ロックも一緒に書く）
    s.doc.remove_channel(channel).unwrap();
    s.doc
        .set_layer_locks(base, yolu_core::LayerLocks::ALL)
        .unwrap();
    let material = s.doc.capture_smart_material(&[base], "x").unwrap();
    let file = SmartFile::from_core(&material, &yolu_app::project::writer()).unwrap();
    assert_eq!(
        file.to_core().unwrap().layers()[0].locks(),
        yolu_core::LayerLocks::ALL
    );
}

/// 同梱の素材はどれもノイズ・グランジを使う。置いたグループを「レイヤーを保存」しても、日英の短い理由で断られ（yolu-io の長い診断も
/// 「Invalid or unsupported project data」も出さない）、棚にも文書の保存状態にも触れない。
#[test]
fn headless_saving_a_placed_bundled_group_is_refused_with_a_short_reason_in_both_languages() {
    for (lang, want) in [
        (
            Lang::Ja,
            "をアセットに保存できません（ノイズ・グランジを使っています）。",
        ),
        (
            Lang::En,
            " to the project's assets (It uses Noise and Grunge).",
        ),
    ] {
        for id in ["builtin:rusty-iron", "builtin:wood"] {
            let mut s = AppState::new(64, 64);
            s.lang = lang;
            place(&mut s, id);
            let group = s.selected_layer.unwrap();
            assert!(
                s.doc.layer(group).unwrap().is_group(),
                "{id}: {}",
                s.message
            );
            s.modified = false;
            let (undo, revision) = (s.doc.undo_count(), s.doc.revision());
            s.apply(Action::Shelf(ShelfOp::SaveMaterial(group)));
            assert!(
                s.message.ends_with(want)
                    && s.message.starts_with(lang.pick("「", "Cannot save \"")),
                "{id}: {}",
                s.message
            );
            assert!(s.shelf.resources().is_empty() && !s.shelf.changed, "{id}");
            assert!(!s.modified && s.shelf.saving_name().is_none(), "{id}");
            assert_eq!((s.doc.undo_count(), s.doc.revision()), (undo, revision));
        }
    }
}

#[test]
fn headless_an_unreadable_shelf_result_keeps_the_reason_and_the_open_notice_names_it() {
    // 読めた結果はそのまま棚に、読めなかった結果は空の棚（棚を変える操作は断る）と理由
    let ok = ShelfState::from_read(Ok::<_, yolu_io::Error>(fixture_shelf()));
    assert!(ok.unavailable.is_none() && ok.resources().len() == 6);
    let broken = ShelfState::from_read(Err::<Shelf, _>(yolu_io::Error::InvalidData(
        "索引が壊れています".into(),
    )));
    let reason = broken.unavailable.as_ref().unwrap();
    assert_eq!(reason.reason(Lang::Ja), "索引が壊れています");
    // 英語の画面に日本語の診断を出さない（種類ごとの短い文）
    assert_eq!(
        reason.reason(Lang::En),
        "Invalid or unsupported project data"
    );
    assert!(broken.resources().is_empty() && !broken.changed);
    // 開いたときの知らせ（日英）。読めた棚には出さない
    let (mut s, _) = painted(8);
    assert_eq!(yolu_app::project::unreadable_shelf_notice(&s), None);
    s.shelf = broken;
    for (lang, want) in [
        (
            Lang::Ja,
            "プロジェクトのアセットを読めません（索引が壊れています）",
        ),
        (
            Lang::En,
            "The project's assets cannot be read (Invalid or unsupported project data)",
        ),
    ] {
        s.lang = lang;
        let notice = yolu_app::project::unreadable_shelf_notice(&s).unwrap();
        assert!(notice.contains(want), "{notice}");
    }
}

#[test]
fn headless_the_grid_metrics_always_reach_the_last_row_whatever_the_width() {
    use yolu_app::panels::assets::{grid_metrics, BAR_W, CELL_H, CELL_W, GAP};
    let columns_for = |usable: f32| (((usable - GAP) / (CELL_W + GAP)).floor() as usize).max(1);
    let mut narrowed_by_the_bar = 0;
    for height in [120.0f32, 200.0, 280.0, 360.0, 500.0] {
        for count in 1..=40usize {
            let mut width = 80.0f32;
            while width < 520.0 {
                let m = grid_metrics(width, height, count);
                // 全体の高さは、実際に並べる列数での行数から（スクロールバーで列が減っても、行数を持ち越さない）
                let rows = count.div_ceil(m.columns);
                let content = GAP + rows as f32 * (CELL_H + GAP);
                assert_eq!(m.content, content, "{width} {height} {count}");
                assert_eq!(m.max_scroll, (content - height).max(0.0));
                // 最後までスクロールすれば、最後の段の下端が格子の下端に収まる
                let bar = content > height;
                let usable = width - if bar { BAR_W } else { 0.0 };
                assert_eq!(m.columns, columns_for(usable), "{width} {height} {count}");
                assert!(GAP + m.columns as f32 * (m.cell_w + GAP) <= usable + 0.01);
                assert!(content - m.max_scroll <= height + 0.01 || !bar);
                if bar && columns_for(width) > m.columns {
                    narrowed_by_the_bar += 1;
                }
                width += 0.5;
            }
        }
    }
    assert!(
        narrowed_by_the_bar > 100,
        "バーで列が減る幅を、たくさん通った"
    );
    // バーの有無で列数が変わる幅（3 → 2 列）で、14 枚が最後の段まで届く
    let m = grid_metrics(255.0, 280.0, 14);
    assert_eq!((m.columns, m.content, m.max_scroll), (2, 692.0, 412.0));
    let m = grid_metrics(175.0, 280.0, 14);
    assert_eq!((m.columns, m.content), (1, 6.0 + 14.0 * 98.0));
}

/// 画像（1×1）だけで `count` 個を埋めた棚（中身はすべて別。個数の上限の試験用）。
fn shelf_of_images(count: usize) -> Shelf {
    let mut shelf = Shelf::new(SHELF_BUDGET);
    for i in 0..count {
        let rgba = [(i % 256) as u8, (i / 256) as u8, 9, 255];
        shelf
            .add_image(
                &format!("30000000-0000-0000-0000-{i:012}"),
                &format!("画像 {i}"),
                &rgba,
                1,
                1,
                "srgb",
                Default::default(),
            )
            .unwrap();
    }
    shelf
}

#[test]
fn headless_the_shelf_takes_256_items_and_refuses_the_257th_without_changing_it() {
    let mut s = AppState::new(16, 16);
    let base = s.selected_layer.unwrap();
    s.shelf = ShelfState::with_shelf(shelf_of_images(MAX_RESOURCES - 1));
    // 256 個目は入る
    s.apply(Action::Shelf(ShelfOp::ImportFile(
        fixtures().join("raster.ylsmart"),
    )));
    assert_eq!(s.shelf.resources().len(), MAX_RESOURCES, "{}", s.message);
    assert!(
        s.message.starts_with("アセットに入れました"),
        "{}",
        s.message
    );
    // 257 個目は、棚も未保存の印も変えずに「いっぱい」と断る（予算超過とは別の理由）
    let entries = s.shelf.shelf().entries().clone();
    s.modified = false;
    for (lang, want) in [
        (Lang::Ja, "アセットがいっぱいです"),
        (Lang::En, "Assets are full"),
    ] {
        s.lang = lang;
        s.apply(Action::Shelf(ShelfOp::ImportFile(
            fixtures().join("mask.ylsmart"),
        )));
        assert!(
            s.message.starts_with(if lang == Lang::Ja {
                "「mask.ylsmart」をアセットに読み込めません"
            } else {
                "Cannot import \"mask.ylsmart\" into the project's assets"
            }) && s.message.contains(want),
            "{}",
            s.message
        );
        assert!(!s.message.contains("予算") && !s.message.contains("budget"));
        assert_eq!(s.shelf.resources().len(), MAX_RESOURCES);
        assert_eq!(s.shelf.shelf().entries(), &entries);
        assert!(!s.modified);
    }
    s.lang = Lang::Ja;
    // 同じ中身は個数を増やさないので、いっぱいの棚でも「すでにある」と言う
    s.apply(Action::Shelf(ShelfOp::ImportFile(
        fixtures().join("raster.ylsmart"),
    )));
    assert!(
        s.message.starts_with("すでにアセットにあります"),
        "{}",
        s.message
    );
    // レイヤーの保存は、写しを作る前に断る（別のスレッドも始めない）
    s.shelf.async_bytes = 0;
    let before = fingerprint(&s);
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    assert!(
        s.message.contains("アセットがいっぱいです"),
        "{}",
        s.message
    );
    assert!(s.shelf.saving_name().is_none() && s.shelf.saves_running() == 0);
    assert_eq!(s.shelf.resources().len(), MAX_RESOURCES);
    assert_eq!(fingerprint(&s), before);
    // 1 つ消せば、また入る
    let first = s.shelf.resources()[0].id.clone();
    s.apply(Action::Shelf(ShelfOp::Remove(first)));
    assert_eq!(s.shelf.resources().len(), MAX_RESOURCES - 1);
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    s.shelf_wait();
    assert_eq!(s.shelf.resources().len(), MAX_RESOURCES, "{}", s.message);
    assert!(
        s.message.starts_with("アセットに入れました"),
        "{}",
        s.message
    );
    // 読み込みも同じ上限: 256 個の棚は .ylp を経て戻り、1 つ多い索引は棚を読めない
    let dir = temp_dir("limit");
    let path = dir.join("full.ylp");
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let mut again = AppState::new(8, 8);
    again.apply(Action::OpenProject(path));
    assert_eq!(
        again.shelf.resources().len(),
        MAX_RESOURCES,
        "{}",
        again.message
    );
    assert!(again.shelf.unavailable.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// 試験用: .ylsmart の正本（native）の値を書き出す（yolu-io の書き方と同じ並べ方）。
fn write_native(value: &yolu_io::NativeValue, out: &mut Vec<u8>) {
    use yolu_io::NativeValue as V;
    match value {
        V::Int(v) => out.extend(v.to_le_bytes()),
        V::Byte(v) => out.push(*v),
        V::Bool(v) => out.push(u8::from(*v)),
        V::Float(v) => out.extend(v.to_le_bytes()),
        V::Guid(v) => out.extend(v),
        V::Text(v) => {
            out.extend((v.len() as i32).to_le_bytes());
            out.extend(v.as_bytes());
        }
        V::Bytes(v) => out.extend(v.iter()),
    }
}

/// Generator の再固定（repin）を持つ .ylsmart の実物。filtered.ylsmart（ぼかしのフィルター 1 つ）のフィルターを Generator（高さ。
/// ベイクの固定は無し）に書き換え、索引の repin にその ID を入れる。yolu-io が検証して読める形にしかしない（読めなければ unwrap で落ちる）。
fn generator_pinned_bytes() -> Vec<u8> {
    use yolu_io::smart::SmartFile;
    use yolu_io::NativeValue as V;
    let file =
        SmartFile::read(&std::fs::read(fixtures().join("filtered.ylsmart")).unwrap()).unwrap();
    let item = "layers[0].filters.items[0]";
    let (mut out, mut stage) = (Vec::new(), [0u8; 16]);
    for f in file.fragment().fields() {
        let mut value = f.value.clone();
        match f.path.strip_prefix(item) {
            Some(".type") => value = V::Int(6),
            Some(".radius") => value = V::Int(0),
            Some(".id") => {
                if let V::Guid(g) = &value {
                    stage = *g;
                }
            }
            _ => {}
        }
        write_native(&value, &mut out);
        if f.path == format!("{item}.output_white") {
            // Generator の中身（種類 0・アルゴリズム 1・低い/高い・柔らかさ・反転・ノイズ・ブレンド・方向・固定 0 個）
            for v in [
                V::Int(0),
                V::Int(1),
                V::Float(0.25),
                V::Float(0.75),
                V::Float(0.1),
                V::Bool(false),
                V::Float(0.0),
                V::Float(0.5),
                V::Int(0),
                V::Int(0),
                V::Int(0),
                V::Float(0.5),
                V::Int(1),
                V::Float(0.0),
                V::Float(1.0),
                V::Float(0.0),
                V::Bool(false),
                V::Int(0),
            ] {
                write_native(&v, &mut out);
            }
        }
    }
    let mut entries = file.entries().clone();
    entries.insert("layers.utpaint".into(), Arc::from(out));
    // .NET の GUID の並びから ID の文字列へ
    let b = stage;
    let id = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[3], b[2], b[1], b[0], b[5], b[4], b[7], b[6], b[8], b[9], b[10], b[11], b[12], b[13],
        b[14], b[15]
    );
    let info = String::from_utf8(file.entries()["smart.json"].to_vec())
        .unwrap()
        .replace("\"repin\": []", &format!("\"repin\": [\"{id}\"]"));
    entries.insert("smart.json".into(), Arc::from(info.into_bytes()));
    SmartFile::from_entries(entries)
        .unwrap()
        .file_bytes()
        .to_vec()
}

#[test]
fn headless_a_smart_asset_core_cannot_hold_is_listed_marked_exported_whole_and_not_placed() {
    let dir = temp_dir("unsupported");
    let pinned = dir.join("pinned.ylsmart");
    std::fs::write(&pinned, generator_pinned_bytes()).unwrap();
    // フィルター入りの素材は、効果のレイヤーが core に入ってから置ける（前は core で扱えない中身として断っていた）
    {
        let mut s = AppState::new(16, 16);
        s.apply(Action::Shelf(ShelfOp::ImportFile(
            fixtures().join("filtered.ylsmart"),
        )));
        let id = ids(&s, "smartMaterial").pop().expect(&s.message);
        s.shelf.inspect_pending(10);
        assert_eq!(s.shelf.block_of(&id), None, "フィルター入りは置ける");
    }
    // 本物の入力: Generator の再固定
    for (file, block, ja, en) in [(
        pinned.clone(),
        "generators",
        "ジェネレーター付きは置けません",
        "Has pinned generators",
    )] {
        let mut s = AppState::new(16, 16);
        s.apply(Action::Shelf(ShelfOp::ImportFile(file.clone())));
        let id = ids(&s, "smartMaterial").pop().expect(&s.message);
        assert!(
            s.message.starts_with("アセットに入れました"),
            "{}",
            s.message
        );
        // 棚に並び、置けない理由と警告の印がつく（名前の帯・一覧の印は block を見る）
        s.shelf.inspect_pending(10);
        match (block, s.shelf.block_of(&id)) {
            ("unsupported", Some(Block::Unsupported(_)))
            | ("generators", Some(Block::Generators)) => {}
            (_, got) => panic!("{file:?}: {got:?}"),
        }
        assert!(s.shelf.warns(&id));
        let b = s.shelf.block_of(&id).unwrap();
        assert_eq!(b.reason(Lang::Ja), ja);
        assert_eq!(b.reason(Lang::En), en);
        // 置けない: 文書は変わらず、理由が日英で出る（「読めません」とは言わない）
        s.modified = false;
        let before = fingerprint(&s);
        for (lang, want) in [(Lang::Ja, ja), (Lang::En, en)] {
            s.lang = lang;
            place(&mut s, &id);
            assert!(s.message.contains(want), "{}", s.message);
            assert!(!s.message.contains("読めません") && !s.message.contains("Unreadable"));
            s.apply(Action::Shelf(ShelfOp::Place {
                id: id.clone(),
                target: PlaceTarget::At {
                    parent: None,
                    position: Some(0),
                },
            }));
            assert!(s.message.contains(want), "{}", s.message);
        }
        s.lang = Lang::Ja;
        assert_eq!(fingerprint(&s), before);
        assert!(!s.modified);
        // 原本のまま書き出せる（Unity 版が使う）
        let out = dir.join("out.ylsmart");
        s.apply(Action::Shelf(ShelfOp::ExportFile {
            id: id.clone(),
            path: out.clone(),
        }));
        assert_eq!(
            std::fs::read(&out).unwrap(),
            std::fs::read(&file).unwrap(),
            "{file:?}"
        );
        // 保存して開き直しても同じ理由で並ぶ
        let path = dir.join("kept.ylp");
        s.apply(Action::SaveProjectAs(path.clone()));
        assert!(s.message.starts_with("保存しました"), "{}", s.message);
        let project = Project::read(&std::fs::read(&path).unwrap()).unwrap();
        let mut reopened = ShelfState::from_project(&project);
        assert_eq!(reopened.resources().len(), 1);
        reopened.inspect_pending(10);
        assert_eq!(
            reopened
                .block_of(&reopened.resources()[0].id.clone())
                .map(|b| b.reason(Lang::En)),
            Some(en.to_owned())
        );
    }
    // 実物は io が読める・ただし編集用には開けない（画像の同梱や core の断りとは別の文言）
    let file = yolu_io::smart::SmartFile::read(&std::fs::read(&pinned).unwrap()).unwrap();
    let error = file.to_core().unwrap_err();
    assert!(
        error
            .to_string()
            .contains(yolu_io::smart::REFUSAL_GENERATORS),
        "{error}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_a_layer_with_locks_is_saved_to_the_shelf_with_its_locks() {
    // レイヤーのロックは正本の版 12 の項目で、.ylsmart にも書く（C# の CloneLayer がロックを写すのと同じ）。同期も別のスレッドも
    let (mut s, base) = painted(16);
    s.doc
        .set_layer_locks(base, yolu_core::LayerLocks::PIXELS)
        .unwrap();
    for (n, async_bytes) in [u64::MAX, 0].into_iter().enumerate() {
        s.shelf.async_bytes = async_bytes;
        s.modified = false;
        s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
        s.shelf_wait();
        assert_eq!(
            s.shelf.resources().len(),
            n + 1,
            "{async_bytes}: {}",
            s.message
        );
        assert!(s.shelf.changed && s.modified, "{}", s.message);
    }
    // 文書のロックは変わらず、棚の素材のロックも持つ
    assert_eq!(
        s.doc.layer(base).unwrap().locks(),
        yolu_core::LayerLocks::PIXELS
    );
}

#[test]
fn headless_a_big_save_adds_to_the_shelf_on_the_worker_and_the_shelf_stays_put_meanwhile() {
    let (mut s, base) = painted(32);
    s.shelf = ShelfState::with_shelf(fixture_shelf());
    let count = s.shelf.resources().len();
    let first = s.shelf.resources()[0].id.clone();
    s.shelf.async_bytes = 0;
    s.shelf.hold_saves(true);
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    assert!(s.shelf.saving_name().is_some(), "{}", s.message);
    // 別のスレッドが棚の写しへ足している間は、棚を変える操作を断る（結果は足した後の棚への差し替えなので、その間の変更が消えない）
    for lang in [Lang::Ja, Lang::En] {
        s.lang = lang;
        for op in [
            ShelfOp::Remove(first.clone()),
            ShelfOp::AskRemove(first.clone()),
            ShelfOp::ImportFile(fixtures().join("raster.ylsmart")),
            ShelfOp::ImportFiles(vec![fixtures().join("raster.ylsmart")]),
            ShelfOp::ImportDialog,
        ] {
            s.apply(Action::Shelf(op.clone()));
            assert!(
                s.message.contains(if lang == Lang::Ja {
                    "保存中です"
                } else {
                    "Already saving"
                }),
                "{op:?}: {}",
                s.message
            );
            assert_eq!(s.shelf.resources().len(), count, "{op:?}");
        }
    }
    s.lang = Lang::Ja;
    assert!(s.dialog_request.is_none() && s.shelf.pending_remove.is_none());
    // 読むだけの操作は通る（棚の素材を置く）
    let raster = ids(&s, "smartMaterial").pop().unwrap();
    place(&mut s, &raster);
    assert!(s.message.starts_with("置きました"), "{}", s.message);
    // 足し終えても、棚に入るのは画面のスレッドが結果を受け取るとき（差し替えだけ）
    s.shelf.hold_saves(false);
    s.shelf.wait_idle();
    assert_eq!(s.shelf.resources().len(), count);
    s.shelf_poll();
    assert_eq!(s.shelf.resources().len(), count + 1, "{}", s.message);
    assert!(s.shelf.saving_name().is_none() && s.shelf.changed && s.modified);
    assert!(
        s.message.starts_with("アセットに入れました"),
        "{}",
        s.message
    );
    // 差し替えた棚は、読み直しても同じ（別のスレッドで足した結果が、画面のスレッドで足した結果と同じ）
    let reread = Shelf::read(s.shelf.shelf().entries(), SHELF_BUDGET).unwrap();
    assert_eq!(reread.used_bytes(), s.shelf.shelf().used_bytes());
    assert_eq!(
        reread.resources().iter().map(|r| &r.id).collect::<Vec<_>>(),
        s.shelf
            .resources()
            .iter()
            .map(|r| &r.id)
            .collect::<Vec<_>>()
    );
    // 保存が終われば、消す・読み込むはまた通る
    s.apply(Action::Shelf(ShelfOp::Remove(first)));
    assert_eq!(s.shelf.resources().len(), count, "{}", s.message);
    // 大きな棚への小さな保存も別のスレッドへ回る（足すときの棚全体の検証が棚の大きさに比例する）。素材 1 つでは回らない小ささでも
    let pixels = s
        .doc
        .capture_smart_material(&[base], "x")
        .unwrap()
        .pixel_bytes();
    let used = s.shelf.shelf().used_bytes();
    assert!(used >= 2);
    s.shelf.async_bytes = pixels + used / 2 + 1;
    s.shelf.hold_saves(true);
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    assert!(s.shelf.saving_name().is_some(), "{}", s.message);
    s.shelf.hold_saves(false);
    s.shelf_wait();
    assert_eq!(s.shelf.resources().len(), count + 1, "{}", s.message);
}

/// 測定用: 全面を塗ったレイヤーの画素（`noisy` は乱数。`seed` から続く。そうでなければなだらかな勾配）。
fn fill_whole_layer(s: &mut AppState, id: LayerId, noisy: bool, seed: &mut u32) {
    let size = s.doc.width();
    let ts = s.doc.tile_size();
    for ty in 0..size.div_ceil(ts) {
        for tx in 0..size.div_ceil(ts) {
            let mut bytes = vec![0u8; (ts * ts * 4) as usize];
            for (i, px) in bytes.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let (x, y) = (tx * ts + i as u32 % ts, ty * ts + i as u32 / ts);
                *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                px.copy_from_slice(&if noisy {
                    (*seed | 0xFF00_0000).to_le_bytes()
                } else {
                    [(x * 255 / size) as u8, (y * 255 / size) as u8, 128, 255]
                });
            }
            s.doc
                .import_tile(id, Channel::Color, TileCoord::new(tx, ty), &bytes)
                .unwrap();
        }
    }
}

fn measure_environment() {
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|t| {
            t.lines()
                .find(|l| l.starts_with("model name"))
                .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_owned())
        })
        .unwrap_or_else(|| "不明".into());
    println!(
        "環境: {cpu} / スレッド {}（最適化 1 の試験用ビルド。本番の最適化では短くなる）",
        std::thread::available_parallelism().map_or(0, |n| n.get())
    );
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

/// 測定（普段は回さない）。全面を塗った 1 レイヤーをスマートマテリアルとして棚へ入れるとき、画面のスレッドが止まる時間と、別のスレッドで
/// 走る時間を分けて、画素の中身（なだらかな勾配・乱数）と大きさ（2048²・4096²）ごとに 3 回ずつの中央値で出す（棚は空）。
/// - `click`: 「レイヤーを保存」を押してから戻るまで（画面のスレッド。画素の写しを捕まえて、別のスレッドを始める）
/// - `encode`: 別のスレッドの変換・圧縮・サムネイルと、棚の写しへの追加（棚全体の検証。画面は止まらない）
/// - `keep`: できた素材を受け取って棚を差し替える（画面のスレッド。足した後の棚を受け取るだけ）
///
/// 回し方: `cargo test -p yolu-app --test gui_shell shelf::measure_ -- --ignored --nocapture --test-threads=1`
/// （`dev` プロファイル。Cargo.toml で最適化つき）
#[test]
#[ignore = "測定。--ignored --nocapture で回す"]
fn measure_how_long_saving_a_full_layer_takes() {
    use std::time::Instant;
    measure_environment();
    for size in [2048u32, 4096] {
        for (content, noisy) in [("勾配", false), ("乱数", true)] {
            let (mut click, mut encode, mut keep) = (Vec::new(), Vec::new(), Vec::new());
            let mut seed = 0x9E37_79B9u32;
            for _ in 0..3 {
                let mut s = AppState::new(size, size);
                let id = s.selected_layer.unwrap();
                fill_whole_layer(&mut s, id, noisy, &mut seed);
                s.shelf.async_bytes = 0; // 必ず別のスレッドで
                let t = Instant::now();
                s.apply(Action::Shelf(ShelfOp::SaveMaterial(id)));
                click.push(t.elapsed().as_secs_f64());
                let t = Instant::now();
                s.shelf.wait_idle();
                encode.push(t.elapsed().as_secs_f64());
                let t = Instant::now();
                s.shelf_poll();
                keep.push(t.elapsed().as_secs_f64());
                assert_eq!(s.shelf.resources().len(), 1, "{}", s.message);
            }
            println!(
                "{size}² {content}: click {:.0} ミリ秒（画面を止める） / encode+add {:.2} 秒（別のスレッド） / keep {:.2} ミリ秒（画面を止める。棚は空）",
                median(&mut click) * 1000.0,
                median(&mut encode),
                median(&mut keep) * 1000.0
            );
        }
    }
}

/// 測定（普段は回さない）。棚を数百 MiB 埋めた状態で、素材を足す・消す・見る・外のファイルから読み込む操作が画面のスレッドを止める時間。
/// 棚の中身は 2048² の乱数のレイヤー（画素 16 MiB・圧縮してもほぼそのまま。棚の使用量はファイルと画素で 1 個およそ 30 MiB）を 4・8・14 個
/// （使用量およそ 120・240・415 MiB。棚の予算は 512 MiB）。
/// 足す・消す・見るは同じ棚の写しで 3 回ずつの中央値、読み込みは 1 回。`inspect` は 1 項目の分（画面は 1 フレームに 2 個まで見る）。
/// 画面のスレッドを止めるのは `keep`（保存の結果を受け取る差し替え）・`remove`・`inspect`・`import`。`add` は別のスレッドで走る分。
#[test]
#[ignore = "測定。--ignored --nocapture で回す"]
fn measure_how_long_a_full_shelf_stalls_the_screen() {
    use std::time::Instant;
    use yolu_io::shelf::ResourceKind;
    measure_environment();
    let dir = temp_dir("measure-full");
    let mut seed = 0x9E37_79B9u32;
    let mut s = AppState::new(2048, 2048);
    let id = s.selected_layer.unwrap();
    fill_whole_layer(&mut s, id, true, &mut seed);
    // 同じレイヤーを捕まえるたびに別の .ylsmart になる。1 つずつ空の棚に入れて索引の項目と中身を集め、棚は 1 回の読み込みで作る
    // （1 つずつ足す作り方は、棚の検証が個数の 2 乗の時間になる）
    let started = Instant::now();
    let mut entries: BTreeMap<String, Arc<[u8]>> = BTreeMap::new();
    let mut index: Vec<String> = Vec::new();
    let mut shelves: Vec<(usize, Shelf)> = Vec::new();
    for k in 1..=14usize {
        let name = format!("素材 {k}");
        let bytes = encode_with(&s, id, &name);
        let mut one = Shelf::new(u64::MAX);
        one.add_file_without_origin(
            &format!("20000000-0000-0000-0000-{k:012}"),
            &name,
            ResourceKind::SmartMaterial,
            &bytes,
        )
        .unwrap();
        let r = one.resources()[0].clone();
        index.push(r.metadata.to_string());
        entries.insert(r.entry.clone(), Arc::from(bytes));
        if [4, 8, 14].contains(&k) {
            let mut files = entries.clone();
            files.insert(
                "resources.json".into(),
                Arc::from(format!("{{\"resources\":[{}]}}", index.join(",")).into_bytes()),
            );
            let t = Instant::now();
            let shelf = Shelf::read(&files, SHELF_BUDGET).unwrap();
            println!(
                "棚 {k} 個（使用 {} MiB）を 1 回の読み込みで作る: {:.2} 秒",
                shelf.used_bytes() >> 20,
                t.elapsed().as_secs_f64()
            );
            shelves.push((k, shelf));
        }
    }
    println!("棚の用意: {:.0} 秒", started.elapsed().as_secs_f64());
    // 足すもの・読み込むもの（棚に無い別の素材。16 MiB）
    let extra = encode_with(&s, id, "足す素材");
    let extra_path = dir.join("extra.ylsmart");
    std::fs::write(&extra_path, &extra).unwrap();
    for (k, shelf) in &shelves {
        let used = shelf.used_bytes() >> 20;
        let first = shelf.resources()[0].id.clone();
        let (mut add, mut remove, mut inspect, mut keep) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for _ in 0..3 {
            // 足す: 別のスレッドの保存が終わった後に画面のスレッドで棚へ入れる部分
            let mut state = AppState::new(2048, 2048);
            state.shelf = ShelfState::with_shelf(shelf.clone());
            state.shelf.async_bytes = 0;
            let layer = state.selected_layer.unwrap();
            fill_whole_layer(&mut state, layer, true, &mut seed);
            state.apply(Action::Shelf(ShelfOp::SaveMaterial(layer)));
            state.shelf.wait_idle();
            let t = Instant::now();
            state.shelf_poll();
            keep.push(t.elapsed().as_secs_f64());
            assert_eq!(state.shelf.resources().len(), k + 1, "{}", state.message);
            // 棚の写しへ足す時間（保存では別のスレッドで走る分。読み込みでは画面のスレッド）
            let mut direct = shelf.clone();
            let t = Instant::now();
            direct
                .add_file_without_origin(
                    "20000000-0000-0000-0000-0000000000aa",
                    "足す素材",
                    ResourceKind::SmartMaterial,
                    &extra,
                )
                .unwrap();
            add.push(t.elapsed().as_secs_f64());
            // 消す
            let mut state = AppState::new(8, 8);
            state.shelf = ShelfState::with_shelf(shelf.clone());
            let t = Instant::now();
            state.apply(Action::Shelf(ShelfOp::Remove(first.clone())));
            remove.push(t.elapsed().as_secs_f64());
            assert_eq!(state.shelf.resources().len(), k - 1, "{}", state.message);
            // 見る（サムネイルのために 1 項目を展開する）
            let mut state = ShelfState::with_shelf(shelf.clone());
            let t = Instant::now();
            state.inspect(&first);
            inspect.push(t.elapsed().as_secs_f64());
        }
        let mut state = AppState::new(8, 8);
        state.shelf = ShelfState::with_shelf(shelf.clone());
        let t = Instant::now();
        state.apply(Action::Shelf(ShelfOp::ImportFile(extra_path.clone())));
        let import = t.elapsed().as_secs_f64();
        assert_eq!(state.shelf.resources().len(), k + 1, "{}", state.message);
        println!(
            "棚 {k} 個（{used} MiB）: 保存の結果の差し替え keep {:.2} ミリ秒 / 棚の写しへ足す add {:.2} 秒（保存では別のスレッド） / 消す remove {:.2} 秒 / 見る inspect {:.2} 秒（1 項目。1 フレーム 2 個まで） / 16 MiB の .ylsmart を読み込む import {:.2} 秒",
            median(&mut keep) * 1000.0,
            median(&mut add),
            median(&mut remove),
            median(&mut inspect),
            import
        );
    }
    // 大きい素材（4096² の乱数。画素 64 MiB）を、およそ 240 MiB の棚へ読み込む・見る
    let (k, shelf) = &shelves[1];
    let mut big = AppState::new(4096, 4096);
    let big_id = big.selected_layer.unwrap();
    fill_whole_layer(&mut big, big_id, true, &mut seed);
    let big_path = dir.join("big.ylsmart");
    std::fs::write(&big_path, encode_with(&big, big_id, "大きい素材")).unwrap();
    let mut state = AppState::new(8, 8);
    state.shelf = ShelfState::with_shelf(shelf.clone());
    let t = Instant::now();
    state.apply(Action::Shelf(ShelfOp::ImportFile(big_path)));
    let import = t.elapsed().as_secs_f64();
    assert_eq!(state.shelf.resources().len(), k + 1, "{}", state.message);
    let big_resource = state.shelf.selected.clone().unwrap();
    let t = Instant::now();
    state.shelf.inspect(&big_resource);
    println!(
        "棚 {k} 個（{} MiB）へ 4096² の .ylsmart: 読み込む import {import:.2} 秒 / 見る inspect {:.2} 秒",
        shelf.used_bytes() >> 20,
        t.elapsed().as_secs_f64()
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// 測定用: レイヤーを捕まえて .ylsmart のバイト列にする（捕まえるたびに別の素材になる）。
fn encode_with(s: &AppState, id: LayerId, name: &str) -> Vec<u8> {
    use yolu_io::smart::SmartFile;
    let material = s.doc.capture_smart_material(&[id], name).unwrap();
    SmartFile::from_core(&material, &yolu_app::project::writer())
        .unwrap()
        .file_bytes()
        .to_vec()
}

// ───────── 画面（egui_kittest） ─────────

mod ui {
    use super::*;
    use common::*;
    use egui::{pos2, Key, Modifiers, PointerButton, Rect};
    use egui_kittest::kittest::{NodeT, Queryable};
    use egui_kittest::Harness;
    use yolu_app::state::PopupKind;
    use yolu_app::YoluApp;

    /// 人工の棚（6 種）を持ったウィンドウ。縦を伸ばして格子が 3 行見えるようにする。
    pub fn window() -> Harness<'static, YoluApp> {
        let mut h = app(1280.0, 1000.0, 64);
        // 左の列の先頭はブラシのパネルなので、棚の「アセット」のタブを前へ出す
        click_tab(&mut h, yolu_app::Tab::Assets);
        // タブを押した時刻から間を空ける（このあとのダブルクリックの 1 回目が、タブの押下との 2 回押しに数えられないように）
        for _ in 0..30 {
            h.step();
        }
        h.state_mut().state.shelf = ShelfState::with_shelf(fixture_shelf());
        // プロジェクトの棚だけの並びを調べる試験のウィンドウ（同梱の素材は別の試験で見る）
        h.state_mut().state.shelf.show_builtin = false;
        h.run();
        settle(&mut h);
        h
    }

    fn st<'a>(h: &'a Harness<'_, YoluApp>) -> &'a AppState {
        &h.state().state
    }

    /// 別のスレッドで作る項目の絵と情報ができるまで待って、描き直す（見えている項目を頼む→できるまで待つ→描く、を 3 回）。
    pub fn settle(h: &mut Harness<'_, YoluApp>) {
        for _ in 0..3 {
            h.run();
            h.state_mut().state.shelf.wait_inspections();
            h.state_mut().state.library.wait_probes();
        }
        h.run();
    }

    fn apply(h: &mut Harness<'_, YoluApp>, a: Action) {
        h.state_mut().state.apply(a);
        h.run();
    }

    /// 「アセット」の格子の中の素材（名前が同じ別の部品 — 絞り込みのアイコン・プロパティのタブ — より下）。
    pub fn card(h: &Harness<'_, YoluApp>, name: &str) -> Rect {
        let body = top_right_body(h);
        rect_of(h, name, |r| {
            body.contains(r.center()) && r.top() > body.top() + 67.0
        })
    }

    /// 「アセット」の格子に、その名前の素材が出ているか。
    fn card_shown(h: &Harness<'_, YoluApp>, name: &str) -> bool {
        let body = top_right_body(h);
        h.query_all_by_label(name)
            .any(|n| body.contains(n.rect().center()) && n.rect().top() > body.top() + 67.0)
    }

    /// レイヤーの組のレイヤーの行。
    fn row(h: &Harness<'_, YoluApp>, name: &str) -> Rect {
        let body = layers_body(h);
        rect_of(h, name, |r| body.contains(r.center()) && r.height() < 40.0)
    }

    fn layers(h: &Harness<'_, YoluApp>) -> Vec<String> {
        st(h)
            .doc
            .layers()
            .iter()
            .map(|l| l.name().to_owned())
            .collect()
    }

    fn undo(h: &mut Harness<'_, YoluApp>) {
        key(h, Key::Z, Modifiers::COMMAND);
        h.run();
    }

    fn right_click(h: &mut Harness<'_, YoluApp>, at: egui::Pos2) {
        press(h, at, PointerButton::Secondary);
        h.step();
        release(h, at, PointerButton::Secondary);
        h.run();
    }

    /// 同じ所を 2 回続けて押す（ダブルクリック）。
    fn double_click(h: &mut Harness<'_, YoluApp>, at: egui::Pos2) {
        for _ in 0..2 {
            press(h, at, PointerButton::Primary);
            release(h, at, PointerButton::Primary);
            h.step();
        }
        h.run();
    }

    /// レイヤーが 3 つ（下から レイヤー 1・2・3）の文書。
    fn three_layers(h: &mut Harness<'_, YoluApp>) {
        apply(h, Action::NewLayer);
        apply(h, Action::NewLayer);
        assert_eq!(layers(h), ["レイヤー 1", "レイヤー 2", "レイヤー 3"]);
    }

    #[test]
    fn the_panel_filters_by_kind_and_searches_by_name() {
        let mut h = window();
        assert_eq!(st(&h).shelf.visible().len(), 6);
        h.get_by_label("スマートマテリアル").click();
        h.run();
        assert_eq!(st(&h).shelf.filter, Some(ItemKind::SmartMaterial));
        assert!(h.query_by_label("raster").is_some());
        assert!(h.query_by_label("mask").is_none());
        // 種類を戻して、名前で探す（打つたびに絞る。大文字小文字は区別しない）
        h.get_by_label("すべて").click();
        h.run();
        let fields: Vec<_> = h
            .get_all_by_role(egui::accesskit::Role::TextInput)
            .map(|n| n.rect())
            .collect();
        let body = top_right_body(&h);
        let search = fields.iter().find(|r| body.contains(r.center())).unwrap();
        click(&mut h, search.center());
        h.event(egui::Event::Text("MAS".into()));
        h.run();
        assert_eq!(st(&h).shelf.search, "MAS");
        let names: Vec<_> = st(&h)
            .shelf
            .visible()
            .iter()
            .map(|r| r.name.clone())
            .collect();
        assert_eq!(names, ["mask"]);
        assert!(h.query_by_label("raster").is_none());
        // 消す（×）
        h.get_by_label("×").click();
        h.run();
        assert!(st(&h).shelf.search.is_empty());
        assert_eq!(st(&h).shelf.visible().len(), 6);
    }

    #[test]
    fn a_click_selects_and_the_footer_button_places_with_one_undo() {
        let mut h = window();
        let before = layers(&h);
        assert!(
            h.get_by_label("置く").accesskit_node().is_disabled(),
            "選ぶまでは置けない"
        );
        let at = card(&h, "raster").center();
        click(&mut h, at);
        assert_eq!(
            st(&h).shelf.selected.as_deref(),
            Some("20000000-0000-0000-0000-000000000001")
        );
        h.get_by_label("置く").click();
        h.run();
        assert_eq!(
            st(&h).doc.layers().len(),
            before.len() + 1,
            "{}",
            st(&h).message
        );
        undo(&mut h);
        assert_eq!(layers(&h), before);
    }

    /// 既定の棚（プロジェクトの棚は空）のウィンドウ。同梱の素材が並ぶ。
    fn window_with_bundled() -> Harness<'static, YoluApp> {
        let mut h = app(1280.0, 1000.0, 64);
        click_tab(&mut h, yolu_app::Tab::Assets);
        for _ in 0..30 {
            h.step();
        }
        h.run();
        settle(&mut h);
        h
    }

    #[test]
    fn a_bundled_card_places_from_the_footer_but_is_not_removed_or_exported() {
        let mut h = window_with_bundled();
        let before = layers(&h);
        let at = card(&h, "錆びた鉄").center();
        click(&mut h, at);
        assert_eq!(st(&h).shelf.selected.as_deref(), Some("builtin:rusty-iron"));
        assert!(!h.get_by_label("置く").accesskit_node().is_disabled());
        // 組み込みは棚に入っていないので、書き出せず消せない（ボタンが効かない）
        assert!(h
            .get_by_label("書き出す…（.ylsmart）")
            .accesskit_node()
            .is_disabled());
        assert!(h
            .get_by_label("アセットから消す（置いたレイヤーはそのまま）")
            .accesskit_node()
            .is_disabled());
        h.get_by_label("置く").click();
        h.run();
        assert!(
            st(&h).doc.layers().len() > before.len() + 3,
            "{}",
            st(&h).message
        );
        assert!(
            st(&h).message.starts_with("置きました: 錆びた鉄"),
            "{}",
            st(&h).message
        );
        undo(&mut h);
        assert_eq!(layers(&h), before);
        // 右クリックのメニューでも、書き出す・消すは効かない（置くだけが効く）
        assert_eq!(
            menu_enabled(&h),
            [
                ("置く".to_owned(), true),
                ("書き出す…".to_owned(), false),
                ("ライブラリへ入れる".to_owned(), false),
                ("アセットから消す…".to_owned(), false),
            ]
        );
    }

    /// 比べる相手: 棚に入っている素材は、右クリックのメニューで 3 つとも効く。
    #[test]
    fn a_shelf_card_has_every_context_menu_entry_enabled() {
        let mut h = window();
        let at = card(&h, "raster").center();
        click(&mut h, at);
        assert_eq!(
            menu_enabled(&h),
            [
                ("置く".to_owned(), true),
                ("書き出す…".to_owned(), true),
                ("ライブラリへ入れる".to_owned(), true),
                ("アセットから消す…".to_owned(), true),
            ]
        );
    }

    /// 選んでいる素材の右クリックのメニューの項目（名前と、効くか）。
    fn menu_enabled(h: &Harness<'_, YoluApp>) -> Vec<(String, bool)> {
        use yolu_app::ui::menu::Entry;
        yolu_app::panels::assets::menu_entries(st(h))
            .into_iter()
            .filter_map(|e| match e {
                Entry::Item { label, enabled, .. } => Some((label, enabled)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn dragging_a_bundled_card_onto_the_layer_list_places_it_between_rows() {
        let mut h = window_with_bundled();
        three_layers(&mut h);
        let from = card(&h, "錆びた鉄").center();
        let r2 = row(&h, "レイヤー 2");
        let to = pos2(r2.left() + 120.0, r2.top() + r2.height() * 0.8);
        press(&h, from, PointerButton::Primary);
        h.step();
        for p in [offset(from, 30.0, 10.0), offset(to, 0.0, -60.0), to] {
            move_to(&h, p);
            h.step();
        }
        h.run();
        release(&h, to, PointerButton::Primary);
        h.run();
        assert!(st(&h).doc.layers().len() > 3 + 4, "{}", st(&h).message);
        let names = layers(&h);
        let group = names
            .iter()
            .position(|n| n == "錆びた鉄")
            .expect("グループが置かれる");
        // レイヤー 2 の行の下の隙間に落としたので、レイヤー 1 の上・レイヤー 2 の下に置かれる
        assert!(
            names[..group].contains(&"レイヤー 1".to_owned()),
            "{names:?}"
        );
        assert!(
            names[group..].contains(&"レイヤー 2".to_owned()),
            "{names:?}"
        );
        assert!(
            names[group..].contains(&"レイヤー 3".to_owned()),
            "{names:?}"
        );
    }

    #[test]
    fn snapshot_shelf_bundled() {
        let mut h = window_with_bundled();
        settle(&mut h);
        h.snapshot("assets_shelf_bundled");
        h.state_mut().state.lang = Lang::En;
        h.state_mut().state.shelf.filter = Some(ItemKind::SmartMaterial);
        h.run();
        let at = card(&h, "Rusty Iron").center();
        click(&mut h, at);
        settle(&mut h);
        h.snapshot("assets_shelf_bundled_english");
    }

    #[test]
    fn a_double_click_places_the_card() {
        let mut h = window();
        let at = card(&h, "raster").center();
        double_click(&mut h, at);
        assert_eq!(st(&h).doc.layers().len(), 2, "{}", st(&h).message);
        assert!(
            st(&h).message.starts_with("置きました"),
            "{}",
            st(&h).message
        );
    }

    #[test]
    fn dragging_a_card_onto_the_layer_list_places_it_between_rows_or_into_a_group() {
        let mut h = window();
        three_layers(&mut h);
        let from = card(&h, "raster").center();
        // レイヤー 3 の行の下半分 = 3 と 2 のあいだの線
        let r3 = row(&h, "レイヤー 3");
        let to = pos2(r3.left() + 120.0, r3.top() + r3.height() * 0.8);
        drag(
            &mut h,
            &[from, offset(from, 30.0, 10.0), offset(to, 0.0, -40.0), to],
        );
        let placed = st(&h).selected_layer.unwrap();
        let name = st(&h).doc.layer(placed).unwrap().name().to_owned();
        assert_eq!(
            layers(&h),
            ["レイヤー 1", "レイヤー 2", name.as_str(), "レイヤー 3"],
            "{}",
            st(&h).message
        );
        assert_eq!(
            st(&h).doc.layers()[2].id(),
            placed,
            "線のすぐ下の行の上（レイヤー 3 の下）"
        );
        undo(&mut h);
        assert_eq!(st(&h).doc.layers().len(), 3);
        assert!(st(&h).shelf.selected.is_some());
        // グループの行の中ほど = グループの中の一番上
        apply(&mut h, Action::M2(Edit::GroupSelected));
        let group = st(&h).selected_layer.unwrap();
        let name = st(&h).doc.layer(group).unwrap().name().to_owned();
        let rg = row(&h, &name);
        let from = card(&h, "raster").center();
        let to = pos2(rg.left() + 120.0, rg.center().y);
        drag(
            &mut h,
            &[from, offset(from, 30.0, 10.0), offset(to, 0.0, -30.0), to],
        );
        let inside = st(&h).doc.children_of(Some(group)).unwrap();
        assert_eq!(inside.len(), 2, "{}", st(&h).message);
        assert_eq!(*inside.last().unwrap(), st(&h).selected_layer.unwrap());
    }

    #[test]
    fn dragging_a_smart_mask_onto_a_row_puts_it_on_that_layers_mask() {
        let mut h = window();
        three_layers(&mut h);
        let r1 = st(&h).doc.layers()[0].id();
        h.state_mut().state.doc.add_layer_mask(r1).unwrap();
        h.state_mut()
            .state
            .doc
            .set_mask_pixel(r1, 1, 1, 255)
            .unwrap();
        apply(&mut h, Action::Shelf(ShelfOp::SaveMask(r1)));
        let name = st(&h).shelf.resources().last().unwrap().name.clone();
        h.state_mut().state.shelf.filter = Some(ItemKind::SmartMask);
        h.run();
        let from = card(&h, &name).center();
        let row2 = row(&h, "レイヤー 2");
        let to = pos2(row2.left() + 120.0, row2.center().y);
        let target = st(&h).doc.layers()[1].id();
        assert!(st(&h).doc.layer(target).unwrap().mask().is_none());
        drag(
            &mut h,
            &[from, offset(from, 30.0, 10.0), offset(to, 0.0, -40.0), to],
        );
        let mask = st(&h)
            .doc
            .layer(target)
            .unwrap()
            .mask()
            .unwrap_or_else(|| panic!("{}", st(&h).message));
        assert_eq!(mask.surface().pixel(1, 1).unwrap().a, 255);
        assert_eq!(st(&h).selected_layer, Some(target));
        undo(&mut h);
        assert!(st(&h).doc.layer(target).unwrap().mask().is_none());
    }

    #[test]
    fn dropping_somewhere_else_places_nothing() {
        let mut h = window();
        let from = card(&h, "raster").center();
        let canvas = canvas_rect(&h).center();
        drag(&mut h, &[from, offset(from, 30.0, 10.0), canvas]);
        assert_eq!(st(&h).doc.layers().len(), 1);
        assert!(
            !egui::DragAndDrop::has_any_payload(&h.ctx),
            "ドラッグの荷物は残さない"
        );
    }

    #[test]
    fn dropping_a_brush_on_the_layer_list_says_why_it_cannot_be_placed() {
        let mut h = window();
        three_layers(&mut h);
        let body = top_right_body(&h);
        let filter = rect_of(&h, "ブラシ", |r| {
            body.contains(r.center()) && r.top() < body.top() + 60.0
        });
        click(&mut h, filter.center());
        assert_eq!(st(&h).shelf.filter, Some(ItemKind::Brush));
        let from = card(&h, "ブラシ").center();
        let r2 = row(&h, "レイヤー 2");
        let to = pos2(r2.left() + 120.0, r2.center().y);
        drag(
            &mut h,
            &[from, offset(from, 30.0, 10.0), offset(to, 0.0, -40.0), to],
        );
        assert_eq!(st(&h).doc.layers().len(), 3);
        assert!(
            st(&h).message.contains("ブラシはまだ置けません"),
            "{}",
            st(&h).message
        );
    }

    #[test]
    fn the_context_menu_places_exports_and_removes() {
        let mut h = window();
        let at = card(&h, "raster").center();
        right_click(&mut h, at);
        assert_eq!(
            st(&h).popup.as_ref().map(|p| p.kind),
            Some(PopupKind::Shelf)
        );
        assert_eq!(
            st(&h).shelf.selected.as_deref(),
            Some("20000000-0000-0000-0000-000000000001")
        );
        let at = popup_item(&h, "置く").center();
        click(&mut h, at);
        assert_eq!(st(&h).doc.layers().len(), 2);
        let at = card(&h, "raster").center();
        right_click(&mut h, at);
        let at = popup_item(&h, "書き出す…").center();
        click(&mut h, at);
        assert_eq!(st(&h).dialog_request, Some(DialogRequest::ShelfExport));
        h.state_mut().state.dialog_request = None;
        let at = card(&h, "raster").center();
        right_click(&mut h, at);
        let at = popup_item(&h, "アセットから消す…").center();
        click(&mut h, at);
        assert_eq!(st(&h).dialog_request, Some(DialogRequest::ShelfRemove));
        assert_eq!(st(&h).shelf.resources().len(), 6, "確かめるまでは消さない");
        // 画像は書き出せない（項目は押せない）
        let at = card(&h, "参照").center();
        right_click(&mut h, at);
        let item = popup_item(&h, "書き出す…");
        assert!(h
            .get_all_by_label("書き出す…")
            .any(|n| n.rect() == item && n.accesskit_node().is_disabled()));
    }

    #[test]
    fn the_layer_menu_and_the_panel_buttons_save_layers_and_masks() {
        let mut h = window();
        h.state_mut().state.shelf = ShelfState::default();
        h.run();
        let id = st(&h).selected_layer.unwrap();
        assert!(
            h.get_by_label("マスクを保存")
                .accesskit_node()
                .is_disabled(),
            "マスクが無ければ保存できない"
        );
        h.get_by_label("レイヤーを保存").click();
        h.run();
        assert_eq!(ids(st(&h), "smartMaterial").len(), 1, "{}", st(&h).message);
        assert!(
            st(&h).message.starts_with("アセットに入れました"),
            "{}",
            st(&h).message
        );
        assert_eq!(
            st(&h).shelf.selected,
            Some(st(&h).shelf.resources()[0].id.clone())
        );
        apply(&mut h, Action::M2(Edit::AddMask(id)));
        h.get_by_label("マスクを保存").click();
        h.run();
        assert_eq!(ids(st(&h), "smartMask").len(), 1, "{}", st(&h).message);
        // レイヤーの右クリック
        let r = row(&h, "レイヤー 1");
        right_click(&mut h, pos2(r.left() + 120.0, r.center().y));
        let at = popup_item(&h, "マスクをスマートマスクとして保存").center();
        click(&mut h, at);
        assert_eq!(
            st(&h).shelf.resources().len(),
            3,
            "保存のたびに新しい素材（写し）になる"
        );
        let r = row(&h, "レイヤー 1");
        right_click(&mut h, pos2(r.left() + 120.0, r.center().y));
        let at = popup_item(&h, "スマートマテリアルとして保存").center();
        click(&mut h, at);
        assert!(st(&h).shelf.changed && st(&h).modified);
    }

    #[test]
    fn a_blocked_item_is_marked_and_cannot_be_placed() {
        let mut h = window();
        apply(
            &mut h,
            Action::Shelf(ShelfOp::ImportFile(fixtures().join("images.ylsmart"))),
        );
        // 項目の情報は別のスレッドで作るので、できるまで待つ
        settle(&mut h);
        let id = st(&h).shelf.selected.clone().unwrap();
        assert_eq!(st(&h).shelf.block_of(&id), Some(&Block::Images));
        assert!(h.get_by_label("置く").accesskit_node().is_disabled());
        // 項目は名前の帯に理由を出す・マスクの項目は「マスクに適用」
        h.get_by_label("スマートマスク").click();
        h.run();
        let at = card(&h, "mask").center();
        click(&mut h, at);
        assert!(h.query_by_label("マスクに適用").is_some());
        assert!(h.query_by_label("置く").is_none());
    }

    #[test]
    fn a_smart_asset_with_filters_can_be_placed_from_the_panel() {
        let mut h = window();
        apply(
            &mut h,
            Action::Shelf(ShelfOp::ImportFile(fixtures().join("filtered.ylsmart"))),
        );
        settle(&mut h);
        let id = st(&h).shelf.selected.clone().unwrap();
        // 効果のレイヤーが core に入ったので、フィルター入りの素材は印なしで置ける（見終わっている）
        assert!(st(&h).shelf.info(&id).is_some());
        assert_eq!(st(&h).shelf.block_of(&id), None);
        assert!(!st(&h).shelf.warns(&id));
        assert!(!h.get_by_label("置く").accesskit_node().is_disabled());
        // 種類で置けないブラシには印を付けない（名前の帯に理由を出す）
        let brush = st(&h)
            .shelf
            .resources()
            .iter()
            .find(|r| r.kind == "brush")
            .unwrap()
            .id
            .clone();
        h.state_mut().state.shelf.inspect(&brush);
        assert!(!st(&h).shelf.warns(&brush));
    }

    #[test]
    fn a_read_only_set_cannot_be_placed_into() {
        let mut h = window();
        h.state_mut().state.sets.get_mut(0).unwrap().read_only = Some("試験".into());
        let at = card(&h, "raster").center();
        click(&mut h, at);
        assert!(h.get_by_label("置く").accesskit_node().is_disabled());
        let at = card(&h, "raster").center();
        double_click(&mut h, at);
        assert_eq!(st(&h).doc.layers().len(), 1);
        assert!(st(&h).message.contains("読むだけ"), "{}", st(&h).message);
    }

    #[test]
    fn the_english_screen_names_the_panel() {
        let mut h = window();
        h.state_mut().state.lang = Lang::En;
        h.run();
        for l in [
            "All",
            "Images",
            "Brushes",
            "Materials",
            "Smart Materials",
            "Smart Masks",
            "Save Layer",
            "Save Mask",
            "Place",
        ] {
            assert!(h.get_all_by_label(l).count() >= 1, "{l}");
        }
        let at = card(&h, "raster").center();
        click(&mut h, at);
        h.get_by_label("Place").click();
        h.run();
        assert!(
            st(&h).message.starts_with("Placed: raster"),
            "{}",
            st(&h).message
        );
        assert!(h.query_by_label("置く").is_none());
    }

    /// ウィンドウに落としたファイル（パスだけ持つ）。
    #[derive(Debug)]
    struct Dropped(PathBuf);
    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            std::fs::read(&self.0).map_err(|e| e.to_string())
        }
    }

    #[test]
    fn a_ylsmart_dropped_on_the_window_goes_to_the_shelf() {
        let dir = temp_dir("drop");
        let path = dir.join("drop.ylsmart");
        std::fs::copy(fixtures().join("raster.ylsmart"), &path).unwrap();
        let mut h = app(1280.0, 800.0, 64);
        h.input_mut()
            .dropped_files
            .push(Arc::new(Dropped(path.clone())));
        h.run();
        assert_eq!(st(&h).shelf.resources().len(), 1, "{}", st(&h).message);
        assert_eq!(st(&h).shelf.resources()[0].kind, "smartMaterial");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_grid_scrolls_to_the_cards_below() {
        let mut h = window();
        let layer = st(&h).selected_layer.unwrap();
        for n in 1..=8 {
            h.state_mut()
                .state
                .doc
                .set_layer_name(layer, &format!("素材 {n}"))
                .unwrap();
            apply(&mut h, Action::Shelf(ShelfOp::SaveMaterial(layer)));
        }
        assert_eq!(st(&h).shelf.resources().len(), 14);
        assert!(!card_shown(&h, "素材 8"), "下の段はまだ見えない");
        let grid_at = card(&h, "raster").center();
        move_to(&h, grid_at);
        h.step();
        for _ in 0..6 {
            h.event(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -200.0),
                modifiers: Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            });
            h.step();
        }
        h.run();
        assert!(st(&h).shelf.scroll > 100.0, "{}", st(&h).shelf.scroll);
        assert!(card_shown(&h, "素材 8"), "スクロールで下の段が出る");
        assert!(!card_shown(&h, "透明画像"), "上の段は隠れる");
        // 絞り込みを替えると先頭へ戻る
        h.get_by_label("スマートマスク").click();
        h.run();
        assert_eq!(st(&h).shelf.scroll, 0.0);
    }

    /// 14 枚（人工データ 6 + 保存した 8）の棚のウィンドウ。
    fn window_with_fourteen() -> Harness<'static, YoluApp> {
        let mut h = window();
        let layer = st(&h).selected_layer.unwrap();
        for n in 1..=8 {
            h.state_mut()
                .state
                .doc
                .set_layer_name(layer, &format!("素材 {n}"))
                .unwrap();
            apply(&mut h, Action::Shelf(ShelfOp::SaveMaterial(layer)));
        }
        assert_eq!(st(&h).shelf.resources().len(), 14);
        h
    }

    #[test]
    fn the_grid_reaches_the_last_card_at_the_widths_where_the_scrollbar_changes_the_columns() {
        use yolu_app::panels::assets::CELL_H;
        let mut h = window_with_fourteen();
        // 左の列の幅はウィンドウの幅の 19%。格子の幅が 3 → 2 列（ウィンドウ 1450〜1510 付近）と 2 → 1 列（1020〜1080 付近）に分かれる所を
        // 4 点ずつ通る（バーが出ると幅が 10 点狭まり、列が減って行が増える）
        let widths: Vec<u32> = (1020..=1080)
            .step_by(4)
            .chain((1450..=1510).step_by(4))
            .collect();
        for width in widths {
            h.set_size(egui::vec2(width as f32, 1000.0));
            h.state_mut().state.shelf.scroll = f32::MAX;
            h.run();
            assert!(
                card_shown(&h, "素材 8"),
                "ウィンドウの幅 {width}: 最後の素材までスクロールで届く"
            );
            let last = card(&h, "素材 8");
            assert!(
                last.height() >= CELL_H - 0.5,
                "ウィンドウの幅 {width}: 最後の素材が欠けずに見える {last:?}"
            );
        }
    }

    #[test]
    fn several_ylsmart_files_dropped_on_the_window_keep_the_refusal_after_a_later_success() {
        let dir = temp_dir("drop-several");
        let (first, bad, last) = (
            dir.join("first.ylsmart"),
            dir.join("bad.ylsmart"),
            dir.join("last.ylsmart"),
        );
        std::fs::copy(fixtures().join("raster.ylsmart"), &first).unwrap();
        std::fs::write(&bad, vec![7u8; 300]).unwrap();
        std::fs::copy(fixtures().join("mask.ylsmart"), &last).unwrap();
        let mut h = app(1280.0, 800.0, 64);
        for path in [&first, &bad, &last] {
            h.input_mut()
                .dropped_files
                .push(Arc::new(Dropped(path.clone())));
        }
        h.run();
        assert_eq!(st(&h).shelf.resources().len(), 2, "{}", st(&h).message);
        let message = &st(&h).message;
        assert!(
            message.contains("bad.ylsmart") && message.contains("2 件をアセットに入れました。"),
            "{message}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unreadable_shelf_is_named_and_cannot_be_changed_from_the_panel() {
        let mut h = app(1280.0, 800.0, 64);
        click_tab(&mut h, yolu_app::Tab::Assets);
        h.state_mut().state.shelf = ShelfState::unreadable("試験の理由");
        h.state_mut().state.shelf.show_builtin = false;
        h.run();
        assert!(h
            .get_by_label("レイヤーを保存")
            .accesskit_node()
            .is_disabled());
        assert!(h
            .get_by_label(".ylsmart を読み込む…")
            .accesskit_node()
            .is_disabled());
        settle(&mut h);
        h.snapshot("assets_shelf_unreadable");
    }

    #[test]
    fn the_panel_shows_a_save_in_progress_and_cancels_it() {
        let mut h = window();
        h.state_mut().state.shelf.async_bytes = 0;
        h.state_mut().state.shelf.hold_saves(true);
        h.get_by_label("レイヤーを保存").click();
        h.run();
        assert_eq!(st(&h).shelf.saving_name(), Some("レイヤー 1"));
        assert!(
            h.query_by_label("レイヤーを保存").is_none(),
            "保存のあいだは名前とやめるに替わる"
        );
        h.get_by_label("やめる").click();
        h.run();
        assert!(st(&h).shelf.saving_name().is_none());
        assert!(h.query_by_label("レイヤーを保存").is_some());
        // やめた保存のスレッドが終わるまでは次の保存を始められない。放して終わらせてから、もう一度保存する
        h.state_mut().state.shelf.hold_saves(false);
        st(&h).shelf.wait_idle();
        h.run();
        // 終わるまで待てば、棚に入って選ばれる（絞り込みで隠れない）
        h.get_by_label("レイヤーを保存").click();
        h.run();
        for _ in 0..200 {
            if st(&h).shelf.saving_name().is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
            h.run();
        }
        assert_eq!(st(&h).shelf.resources().len(), 7, "{}", st(&h).message);
        assert!(st(&h).shelf.selected.is_some());
    }

    #[test]
    fn snapshot_shelf_saving() {
        let mut h = window();
        h.state_mut().state.shelf.async_bytes = 0;
        h.state_mut().state.shelf.hold_saves(true);
        h.get_by_label("レイヤーを保存").click();
        h.run();
        settle(&mut h);
        h.snapshot("assets_shelf_saving");
        h.state_mut()
            .state
            .apply(Action::Shelf(ShelfOp::CancelSave));
        h.state_mut().state.shelf.hold_saves(false);
    }

    #[test]
    fn snapshot_shelf_all_kinds() {
        let mut h = window();
        let at = card(&h, "raster").center();
        click(&mut h, at);
        settle(&mut h);
        h.snapshot("assets_shelf");
    }

    #[test]
    fn snapshot_shelf_english_filtered() {
        let mut h = window();
        h.state_mut().state.lang = Lang::En;
        h.state_mut().state.shelf.filter = Some(ItemKind::SmartMask);
        h.run();
        let at = card(&h, "mask").center();
        click(&mut h, at);
        settle(&mut h);
        h.snapshot("assets_shelf_english");
    }

    #[test]
    fn snapshot_shelf_blocked_and_empty() {
        let mut h = app(1280.0, 800.0, 64);
        click_tab(&mut h, yolu_app::Tab::Assets);
        h.state_mut().state.shelf.show_builtin = false;
        h.run();
        settle(&mut h);
        h.snapshot("assets_shelf_empty");
        apply(
            &mut h,
            Action::Shelf(ShelfOp::ImportFile(fixtures().join("images.ylsmart"))),
        );
        h.run();
        h.run();
        settle(&mut h);
        h.snapshot("assets_shelf_blocked");
    }

    #[test]
    fn snapshot_shelf_dragging() {
        let mut h = window();
        three_layers(&mut h);
        let from = card(&h, "raster").center();
        let r2 = row(&h, "レイヤー 2");
        let to = pos2(r2.left() + 120.0, r2.top() + r2.height() * 0.8);
        press(&h, from, PointerButton::Primary);
        h.step();
        for p in [offset(from, 30.0, 10.0), offset(to, 0.0, -60.0), to] {
            move_to(&h, p);
            h.step();
        }
        h.run();
        settle(&mut h);
        h.snapshot("assets_shelf_dragging");
        release(&h, to, PointerButton::Primary);
        h.run();
    }

    /// 英語のパネル: 読めないときの状態と、取り込んだときの知らせ（日本語の絵は `assets_shelf_unreadable`・`assets_shelf_blocked`）。
    #[test]
    fn snapshot_shelf_english_unreadable_and_notice() {
        let mut h = app(1280.0, 800.0, 64);
        click_tab(&mut h, yolu_app::Tab::Assets);
        h.state_mut().state.lang = Lang::En;
        h.state_mut().state.shelf = ShelfState::unreadable("test reason");
        h.state_mut().state.shelf.show_builtin = false;
        h.run();
        settle(&mut h);
        h.snapshot("assets_shelf_unreadable_english");
        h.state_mut().state.shelf = ShelfState::default();
        h.state_mut().state.shelf.show_builtin = false;
        h.run();
        apply(
            &mut h,
            Action::Shelf(ShelfOp::ImportFile(fixtures().join("images.ylsmart"))),
        );
        h.run();
        h.run();
        settle(&mut h);
        h.snapshot("assets_shelf_blocked_english");
    }

    /// 右クリックのメニュー（日英）。「アセットから消す…」が「ライブラリへ入れる」と並ぶ。
    fn snapshot_context_menu(lang: Lang, name: &str) {
        let mut h = window();
        h.state_mut().state.lang = lang;
        h.run();
        let at = card(&h, "raster").center();
        right_click(&mut h, at);
        settle(&mut h);
        h.snapshot(name);
    }

    #[test]
    fn snapshot_shelf_context_menu() {
        snapshot_context_menu(Lang::Ja, "assets_shelf_menu");
    }

    #[test]
    fn snapshot_shelf_context_menu_english() {
        snapshot_context_menu(Lang::En, "assets_shelf_menu_english");
    }
}
