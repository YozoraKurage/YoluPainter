//! アセットの格子に並ぶ「今のテクスチャセットのベイクしたマップ」と、塗りつぶしレイヤーのチャンネルの源に選ぶ操作の試験（画面は描かない）。
//! 項目はプロジェクトの棚（.ylp）に入らない表示で、焼き直す・セットを替えると一覧が替わる。源はアセットの画像に写さず種類で持つので、
//! 画像を選ぶとマップが外れ、マップを選ぶと画像が外れる（どれも 1 回の Undo）。
use crate::common::drive::Drive;

use yolu_app::bake::kind_label;
use yolu_app::engine::{Channel, LayerId};
use yolu_app::fillfx::{inputs, FillOp};
use yolu_app::lang::Lang;
use yolu_app::m2::Edit;
use yolu_app::m2_menu::Popup;
use yolu_app::panels::fill_props;
use yolu_app::shelf::{baked_id, ItemKind, PlaceTarget, ShelfOp};
use yolu_app::state::{Action, AppState};
use yolu_app::ui::menu::{Check, Entry};
use yolu_core::generator::MapKind;
use yolu_core::mesh_maps::{BakedMeshMap, MeshMapKind, MeshMapProvenance};
use yolu_core::ImageId;

const SIZE: usize = 16;

/// 一様な値 `value`（16 ビット）の、覆いが左半分だけの焼いたマップ。
fn baked(kind: MeshMapKind, value: u16) -> BakedMeshMap {
    let provenance = MeshMapProvenance {
        kind,
        engine_version: 1,
        mesh_hash: String::new(),
        topology_hash: String::new(),
        uv_channel: 0,
        width: SIZE as i32,
        height: SIZE as i32,
        target_slot: 0,
        target_slots: vec![0],
        padding: 0,
        antialiasing: 1,
        settings_key: String::new(),
        space: String::new(),
        pose: String::new(),
        source: String::new(),
        bounds_min: [0.0; 3],
        bounds_max: [1.0; 3],
    };
    let channels = kind.channels();
    let data = vec![value; SIZE * SIZE * channels];
    let coverage = (0..SIZE * SIZE)
        .map(|i| u8::from(i % SIZE < SIZE / 2))
        .collect();
    BakedMeshMap::new(provenance, data, coverage).unwrap()
}

/// 曲率と AO を焼いた状態。
fn baked_app() -> AppState {
    let mut s = AppState::new(32, 32);
    s.sets_mut().get_mut(0).unwrap().mesh_maps.put(vec![
        baked(MeshMapKind::Curvature, 20000),
        baked(MeshMapKind::AmbientOcclusion, 40000),
    ]);
    s.sync_baked_assets();
    s
}

fn baked_ids(s: &AppState) -> Vec<String> {
    s.shelf()
        .visible()
        .iter()
        .filter(|r| ItemKind::of(&r.kind) == Some(ItemKind::BakedMap))
        .map(|r| r.id.clone())
        .collect()
}

fn new_fill(s: &mut AppState) -> LayerId {
    s.act(Action::M2(Edit::NewFill));
    s.selected_layer().expect("足したレイヤーを選ぶ")
}

fn source(s: &mut AppState, layer: LayerId, map: Option<MapKind>) {
    s.act(Action::Fill(FillOp::BakedMap {
        layer,
        channel: Channel::Color,
        map,
    }));
}

fn map_of(s: &AppState, layer: LayerId) -> Option<MapKind> {
    s.doc().layer(layer).unwrap().fill_map(Channel::Color)
}

fn image_of(s: &AppState, layer: LayerId) -> Option<ImageId> {
    s.doc().layer(layer).unwrap().fill_image(Channel::Color)
}

fn shelf_image(s: &mut AppState) -> ImageId {
    let quad = [
        255u8, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ];
    let rid = s
        .shelf_mut()
        .add_image(Lang::Ja, "石の模様", &quad, 2, 2)
        .expect("棚へ入る");
    inputs::image_id(&rid).expect("GUID")
}

#[test]
fn the_grid_lists_one_item_per_baked_kind_and_leaves_out_the_unbaked_ones() {
    let s = baked_app();
    // 焼いた種類の並び（MeshMapKind の順）。名前は画面の言語の種類の名前
    assert_eq!(
        baked_ids(&s),
        [
            baked_id(MeshMapKind::AmbientOcclusion),
            baked_id(MeshMapKind::Curvature)
        ]
    );
    let names: Vec<String> = s
        .shelf()
        .visible()
        .iter()
        .filter(|r| r.kind == "bakedMap")
        .map(|r| r.name.clone())
        .collect();
    assert_eq!(
        names,
        [
            kind_label(Lang::Ja, MeshMapKind::AmbientOcclusion),
            kind_label(Lang::Ja, MeshMapKind::Curvature)
        ]
    );
    // 棚の中身（.ylp の resources）には入らず、棚を「変えた」ことにもならない
    assert!(s.shelf().resources().iter().all(|r| r.kind != "bakedMap"));
    assert!(!s.shelf().changed);
    // 種類の絞り込み
    let mut s = s;
    s.shelf_mut().filter = Some(ItemKind::BakedMap);
    assert_eq!(s.shelf().visible().len(), 2);
    s.shelf_mut().filter = Some(ItemKind::Image);
    assert!(s.shelf().visible().is_empty());
}

#[test]
fn a_new_project_with_no_baked_map_has_no_baked_item() {
    let mut s = AppState::new(32, 32);
    s.sync_baked_assets();
    assert!(baked_ids(&s).is_empty());
}

#[test]
fn rebaking_makes_the_thumbnail_again_and_switching_the_set_swaps_the_list() {
    let mut s = baked_app();
    let id = baked_id(MeshMapKind::Curvature);
    s.shelf_mut().inspect(&id);
    let before = s.shelf().info(&id).unwrap().picture().unwrap().clone();
    // 覆いのある画素は値（20000 → 8 ビットで 78）、覆いの無い画素は透明
    assert_eq!(before.rgba[..4], [78, 78, 78, 255]);
    assert_eq!(
        before.rgba[(before.width as usize - 1) * 4 + 3],
        0,
        "覆いの無い画素（右端）"
    );
    // 同じ状態で写し直しても作り直さない（サムネイルを覚えたまま）
    s.sync_baked_assets();
    assert!(s.shelf().info(&id).is_some());
    // 焼き直す: 同じ種類を置き換える
    s.sets_mut()
        .get_mut(0)
        .unwrap()
        .mesh_maps
        .put(vec![baked(MeshMapKind::Curvature, 60000)]);
    s.sync_baked_assets();
    assert!(s.shelf().info(&id).is_none(), "焼き直したら絵を捨てる");
    s.shelf_mut().inspect(&id);
    let after = s.shelf().info(&id).unwrap().picture().unwrap().clone();
    assert_eq!(after.rgba[..4], [233, 233, 233, 255]);
    assert_ne!(before, after);
    // 焼いていないセットへ替えると一覧が空になり、戻すと戻る
    s.add_texture_set().expect("セットを足す");
    s.sync_baked_assets();
    assert!(baked_ids(&s).is_empty());
    s.switch_set(0).unwrap();
    s.sync_baked_assets();
    assert_eq!(baked_ids(&s).len(), 2);
    // 言語を替えると名前が替わる（サムネイルは作り直さない）
    s.shelf_mut().inspect(&id);
    s.lang = Lang::En;
    s.sync_baked_assets();
    assert!(s.shelf().info(&id).is_some());
    assert_eq!(
        s.shelf().get(&id).unwrap().name,
        kind_label(Lang::En, MeshMapKind::Curvature)
    );
}

#[test]
fn choosing_a_baked_map_sets_the_source_in_one_undo_and_choosing_an_image_takes_it_off() {
    let mut s = baked_app();
    let image = shelf_image(&mut s);
    let layer = new_fill(&mut s);
    let steps = s.doc().undo_count();
    source(&mut s, layer, Some(MapKind::Curvature));
    assert_eq!(
        map_of(&s, layer),
        Some(MapKind::Curvature),
        "{}",
        s.message()
    );
    assert_eq!(s.doc().undo_count(), steps + 1, "1 回の Undo");
    s.act(Action::Undo);
    assert_eq!(map_of(&s, layer), None);
    s.act(Action::Redo);
    assert_eq!(map_of(&s, layer), Some(MapKind::Curvature));
    // 画像を選ぶとマップが外れる（源は画像かマップのどちらか）
    s.act(Action::Fill(FillOp::Image {
        layer,
        channel: Channel::Color,
        image: Some(image),
    }));
    assert_eq!(image_of(&s, layer), Some(image));
    assert_eq!(map_of(&s, layer), None);
    // マップを選ぶと画像が外れる
    source(&mut s, layer, Some(MapKind::AmbientOcclusion));
    assert_eq!(image_of(&s, layer), None);
    assert_eq!(map_of(&s, layer), Some(MapKind::AmbientOcclusion));
    // 外す: 1 回の Undo で戻る
    let steps = s.doc().undo_count();
    source(&mut s, layer, None);
    assert_eq!(map_of(&s, layer), None);
    assert_eq!(s.doc().undo_count(), steps + 1);
    s.act(Action::Undo);
    assert_eq!(map_of(&s, layer), Some(MapKind::AmbientOcclusion));
}

#[test]
fn removing_the_image_does_not_take_a_map_off_so_the_panel_removes_the_map_itself() {
    // 画像を外す操作は、画像が無ければ何もしない（マップは残る）。欄の外すボタンは、源がマップのときマップを外す操作を送る
    let mut s = baked_app();
    let layer = new_fill(&mut s);
    source(&mut s, layer, Some(MapKind::Curvature));
    s.act(Action::Fill(FillOp::Image {
        layer,
        channel: Channel::Color,
        image: None,
    }));
    assert_eq!(map_of(&s, layer), Some(MapKind::Curvature));
}

#[test]
fn the_image_list_has_a_baked_map_section_that_marks_the_current_source() {
    let mut s = baked_app();
    let layer = new_fill(&mut s);
    let list = |s: &AppState| fill_props::entries(s, Popup::FillImage(layer, Channel::Color));
    let labels = |entries: &[Entry<Action>]| -> Vec<(String, bool)> {
        entries
            .iter()
            .filter_map(|e| match e {
                Entry::Item { label, check, .. } => Some((label.clone(), *check == Check::Radio)),
                _ => None,
            })
            .collect()
    };
    let has_heading = |entries: &[Entry<Action>]| {
        entries
            .iter()
            .any(|e| matches!(e, Entry::Heading(h) if h == "焼いたマップ"))
    };
    let entries = list(&s);
    assert!(has_heading(&entries));
    let ao = kind_label(Lang::Ja, MeshMapKind::AmbientOcclusion).to_owned();
    let curvature = kind_label(Lang::Ja, MeshMapKind::Curvature).to_owned();
    let items = labels(&entries);
    assert!(items.contains(&(ao.clone(), false)), "{items:?}");
    assert!(items.contains(&(curvature.clone(), false)), "{items:?}");
    // 焼いていない種類は並べない
    assert!(!items
        .iter()
        .any(|(l, _)| *l == kind_label(Lang::Ja, MeshMapKind::Thickness)));
    // 今の源に印
    source(&mut s, layer, Some(MapKind::Curvature));
    let entries = list(&s);
    let items = labels(&entries);
    assert!(items.contains(&(curvature, true)), "{items:?}");
    assert!(items.contains(&(ao, false)));
    assert!(
        items.iter().any(|(l, _)| l == "焼いたマップを外す"),
        "源がマップのときは外す項目がある: {items:?}"
    );
    // 焼いたマップが無いセットでは、節を出さない
    let mut empty = AppState::new(16, 16);
    let layer = new_fill(&mut empty);
    let entries = fill_props::entries(&empty, Popup::FillImage(layer, Channel::Color));
    assert!(!has_heading(&entries));
}

#[test]
fn placing_a_baked_map_adds_a_fill_layer_with_the_map_as_its_source_in_one_undo() {
    let mut s = baked_app();
    let before = s.doc().layers().len();
    let steps = s.doc().undo_count();
    s.act(Action::Shelf(ShelfOp::Place {
        id: baked_id(MeshMapKind::Curvature),
        target: PlaceTarget::Selected,
    }));
    assert_eq!(s.doc().layers().len(), before + 1, "{}", s.message());
    let layer = s.selected_layer().expect("置いたレイヤーを選ぶ");
    assert_eq!(map_of(&s, layer), Some(MapKind::Curvature));
    assert_eq!(s.doc().undo_count(), steps + 1, "1 回の Undo");
    // 落とした位置（一番下）
    s.act(Action::Shelf(ShelfOp::Place {
        id: baked_id(MeshMapKind::AmbientOcclusion),
        target: PlaceTarget::At {
            parent: None,
            position: Some(0),
        },
    }));
    let bottom = s.doc().children_in_set(s.layer_set(), None).unwrap()[0];
    assert_eq!(map_of(&s, bottom), Some(MapKind::AmbientOcclusion));
    s.act(Action::Undo);
    s.act(Action::Undo);
    assert_eq!(s.doc().layers().len(), before);
}

#[test]
fn a_baked_map_item_cannot_be_removed_exported_or_put_into_the_library() {
    let mut s = baked_app();
    let id = baked_id(MeshMapKind::Curvature);
    for op in [
        ShelfOp::Remove(id.clone()),
        ShelfOp::AskRemove(id.clone()),
        ShelfOp::ExportDialog(id.clone()),
        ShelfOp::ToLibrary(id.clone()),
    ] {
        s.message_mut().clear();
        s.act(Action::Shelf(op.clone()));
        assert!(!s.message().is_empty(), "{op:?} は理由を出して断る");
    }
    assert!(s.shelf().pending_remove.is_none());
    assert!(!s.shelf().changed);
    assert_eq!(baked_ids(&s).len(), 2);
}

#[test]
fn a_source_whose_map_is_not_baked_is_listed_as_an_inactive_effect() {
    let mut s = AppState::new(16, 16);
    let layer = new_fill(&mut s);
    source(&mut s, layer, Some(MapKind::Curvature));
    assert!(s.doc().inactive_effect_list().iter().any(
        |e| e.layer == layer && e.target == yolu_core::InactiveTarget::FillMap(Channel::Color)
    ));
}

#[test]
fn a_pending_thumbnail_does_not_keep_the_map_that_was_baked_over() {
    let mut s = baked_app();
    let ids = baked_ids(&s);
    let id = baked_id(MeshMapKind::Curvature);
    let old = {
        let set = s.sets_mut().get_mut(0).unwrap();
        let map = set
            .mesh_maps
            .iter()
            .find(|m| m.kind() == MeshMapKind::Curvature)
            .cloned()
            .unwrap();
        std::sync::Arc::downgrade(&map)
    };
    // 仕事を頼んだまま走らせない
    s.shelf_mut().hold_inspections(true);
    s.shelf_mut().request_inspections(&ids, None);
    assert!(s.shelf().inspections_pending());
    assert!(old.upgrade().is_some(), "焼き直す前は持ち主が持っている");
    s.sets_mut()
        .get_mut(0)
        .unwrap()
        .mesh_maps
        .put(vec![baked(MeshMapKind::Curvature, 60000)]);
    s.sync_baked_assets();
    assert!(
        old.upgrade().is_none(),
        "待っている仕事が古いマップを持ち続けない"
    );
    // 仕事を走らせても、焼き直す前のマップの絵は入らない
    s.shelf_mut().hold_inspections(false);
    s.shelf_mut().wait_inspections();
    assert!(s.shelf().info(&id).is_none());
    s.shelf_mut().inspect(&id);
    assert_eq!(
        s.shelf().info(&id).unwrap().picture().unwrap().rgba[..4],
        [233, 233, 233, 255]
    );
}

#[test]
fn the_japanese_reason_names_the_map_the_way_the_screen_does() {
    use yolu_core::generator::Inactive as I;
    use yolu_core::InactiveReason as R;
    let k = MapKind::Curvature;
    let name = kind_label(Lang::Ja, MeshMapKind::Curvature);
    for (why, tail) in [
        (R::Generator(I::MissingMap(k)), "のマップがありません"),
        (
            R::Generator(I::StaleMap(k)),
            "のマップが古い条件で焼かれています",
        ),
        (
            R::Generator(I::UnverifiedMap(k)),
            "のマップの条件を照合できません",
        ),
        (
            R::Generator(I::MapSize(k)),
            "のマップの大きさがテクスチャセットと違います",
        ),
        (
            R::Generator(I::PinMismatch(k)),
            "のマップがピンした焼きと違います",
        ),
    ] {
        let text = Lang::Ja.inactive_reason(&why);
        assert_eq!(text, format!("{name}{tail}"));
        assert!(!text.contains("Curvature"), "{text}");
    }
    // 種類に関わらない理由は core の文のまま
    assert_eq!(
        Lang::Ja.inactive_reason(&R::Generator(I::NoIdColors)),
        R::Generator(I::NoIdColors).to_string()
    );
}

#[test]
fn a_saved_project_has_no_baked_map_entry_and_keeps_the_fill_source_by_kind() {
    let dir = crate::common::tmp::test_dir("baked-ylp");
    let path = dir.join("baked.ylp");
    let mut s = baked_app();
    let layer = new_fill(&mut s);
    source(&mut s, layer, Some(MapKind::Curvature));
    // 索引（resources.json）ができるよう、棚に画像を 1 つ入れて保存する
    shelf_image(&mut s);
    s.act(Action::SaveProjectAs(path.clone()));
    s.wait_save();
    let archive = yolu_io::Archive::read(&std::fs::read(&path).unwrap()).unwrap();
    let index = String::from_utf8(archive.entries()["resources.json"].to_vec()).unwrap();
    assert!(!index.contains("bakedMap") && !index.contains(yolu_app::shelf::BAKED_PREFIX));
    assert!(
        archive.entries().keys().all(|n| !n.contains("baked:")),
        "焼いたマップの項目は書かない"
    );
    // 開き直すと、源は種類のまま残り、焼いたマップの項目はテクスチャセットのマップから写し直される（棚の索引からではない）
    let mut t = AppState::new(16, 16);
    t.act(Action::OpenProject(path));
    assert!(!t.message().is_empty());
    t.sync_baked_assets();
    let layer = t
        .doc()
        .layers()
        .iter()
        .find(|l| l.fill_map(Channel::Color).is_some())
        .map(|l| l.id())
        .expect("塗りつぶしレイヤーが戻る");
    assert_eq!(map_of(&t, layer), Some(MapKind::Curvature));
    assert_eq!(baked_ids(&t), baked_ids(&s));
}
