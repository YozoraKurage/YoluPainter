//! 同梱のスマートマテリアル（棚の「組み込み」）とノイズ・グランジ。`headless_` で始まる試験は画面を描かず、Wine でも回る。
//! どれも「操作 → 棚か文書が変わる → 取り消し（文書）・開き直し（保存）で戻る」。素材はコードから組んだもの（外の画像・実データは使わない）。
use yolu_app::engine::Channel;
use yolu_app::lang::Lang;
use yolu_app::m2::UiOp;
use yolu_app::shelf::{is_builtin, ItemKind, PlaceTarget, ShelfOp};
use yolu_app::state::{Action, AppState};
use yolu_core::smart_library;

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new(name: &str) -> TempDir {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "yolu-procedural-{name}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn builtin_ids(s: &AppState) -> Vec<String> {
    s.shelf
        .visible()
        .iter()
        .filter(|r| is_builtin(&r.id))
        .map(|r| r.id.clone())
        .collect()
}
fn place(s: &mut AppState, id: &str) {
    s.apply(Action::Shelf(ShelfOp::Place {
        id: id.into(),
        target: PlaceTarget::Selected,
    }));
}
fn color(s: &AppState) -> Vec<u8> {
    s.doc
        .composite_channel(
            Channel::Color,
            yolu_core::Rect::new(0, 0, s.doc.width(), s.doc.height()),
        )
        .unwrap()
}

#[test]
fn headless_bundled_materials_are_listed_after_the_shelf_in_the_current_language() {
    let mut s = AppState::new(64, 64);
    s.apply(Action::M2Ui(UiOp::Language(Lang::Ja)));
    s.shelf.use_language(Lang::Ja);
    let ids = builtin_ids(&s);
    assert_eq!(ids.len(), smart_library::entries().len());
    assert_eq!(ids.len(), 14);
    let names: Vec<String> = ids
        .iter()
        .map(|i| s.shelf.get(i).unwrap().name.clone())
        .collect();
    assert!(names.contains(&"錆びた鉄".to_owned()), "{names:?}");
    assert!(names.contains(&"塗装の剥げ".to_owned()));
    // どれもスマートマテリアルで、プロジェクトの棚（空）には入っていない
    assert!(ids
        .iter()
        .all(|i| ItemKind::of(&s.shelf.get(i).unwrap().kind) == Some(ItemKind::SmartMaterial)));
    assert!(s.shelf.resources().is_empty());
    assert!(!s.shelf.changed);
    // 種類の絞り込みと名前の検索
    s.shelf.filter = Some(ItemKind::SmartMask);
    assert!(builtin_ids(&s).is_empty());
    s.shelf.filter = Some(ItemKind::SmartMaterial);
    assert_eq!(builtin_ids(&s).len(), 14);
    s.shelf.search = "錆".into();
    assert_eq!(builtin_ids(&s).len(), 1);
    // 言語を替えると名前が替わる（ID は同じ）
    s.shelf.search.clear();
    s.shelf.use_language(Lang::En);
    assert_eq!(builtin_ids(&s), ids);
    assert_eq!(
        s.shelf.get(&ids[0]).unwrap().name,
        smart_library::entries()[0].en
    );
    s.shelf.search = "rusty".into();
    assert_eq!(builtin_ids(&s).len(), 1);
    // 表示を切ると並ばない（棚だけの並びを調べる試験用）
    s.shelf.show_builtin = false;
    assert!(builtin_ids(&s).is_empty());
    assert!(s.shelf.get(&ids[0]).is_none());
}

#[test]
fn headless_a_bundled_material_is_placed_as_a_group_in_one_undo_step() {
    for lang in [Lang::Ja, Lang::En] {
        let mut s = AppState::new(96, 64);
        s.lang = lang;
        s.shelf.use_language(lang);
        let id = builtin_ids(&s).into_iter().next().unwrap();
        let (layers, undo, before) = (s.doc.layers().len(), s.doc.undo_count(), color(&s));
        place(&mut s, &id);
        assert_eq!(s.doc.undo_count(), undo + 1, "{}", s.message);
        let entry = smart_library::entries()[0];
        let group = s.selected_layer.unwrap();
        assert_eq!(
            s.doc.layer(group).unwrap().name(),
            entry.name(lang == Lang::Ja)
        );
        assert!(s.doc.layer(group).unwrap().is_group());
        assert!(s.doc.layers().len() > layers + 3);
        assert_ne!(color(&s), before);
        assert!(s.modified);
        // 大きさに依らない素材なので、「大きさを変えた」とは言わない
        assert!(!s.message.contains("128"), "{}", s.message);
        assert!(
            !s.message.contains("resized") && !s.message.contains("から変更"),
            "{}",
            s.message
        );
        s.apply(Action::Undo);
        assert_eq!(s.doc.layers().len(), layers);
        assert_eq!(color(&s), before);
    }
}

#[test]
fn headless_every_bundled_material_places_into_documents_of_other_sizes() {
    for (w, h) in [(64u32, 64u32), (200, 90), (48, 160)] {
        for id in smart_library::entries()
            .iter()
            .map(|e| format!("builtin:{}", e.id))
        {
            let mut s = AppState::new(w, h);
            s.lang = Lang::En;
            s.shelf.use_language(Lang::En);
            let before = s.doc.layers().len();
            place(&mut s, &id);
            assert!(s.doc.layers().len() > before, "{id} {w}x{h}: {}", s.message);
            assert!(s.message.starts_with("Placed"), "{}", s.message);
            // レイヤーは画素を持たず（値とマスクの Generator）、どの大きさでも保存の予算を使わない
            assert!(
                s.doc
                    .layers()
                    .iter()
                    .all(|l| l.allocated_bytes() == 0 || l.name() == "Layer 1"),
                "{id}"
            );
        }
    }
}

#[test]
fn headless_a_bundled_item_is_never_removed_exported_or_written_into_the_project() {
    let mut s = AppState::new(64, 64);
    s.shelf.use_language(Lang::Ja);
    let id = builtin_ids(&s).into_iter().next().unwrap();
    let dir = TempDir::new("refuse");
    for op in [ShelfOp::Remove(id.clone()), ShelfOp::AskRemove(id.clone())] {
        s.message.clear();
        s.apply(Action::Shelf(op));
        assert_eq!(s.message, "組み込みは消せません。");
        assert!(s.shelf.pending_remove.is_none());
    }
    for op in [
        ShelfOp::ExportDialog(id.clone()),
        ShelfOp::ExportFile {
            id: id.clone(),
            path: dir.0.join("x.ylsmart"),
        },
    ] {
        s.message.clear();
        s.apply(Action::Shelf(op));
        assert_eq!(s.message, "組み込みは書き出せません。");
        assert!(s.shelf.export_id.is_none());
    }
    assert!(!dir.0.join("x.ylsmart").exists());
    assert!(!s.shelf.changed);
    s.lang = Lang::En;
    s.apply(Action::Shelf(ShelfOp::Remove(id.clone())));
    assert_eq!(s.message, "Built-in items cannot be removed.");
    // 保存したプロジェクトの棚には入らず、開き直しても棚は空（組み込みはコードから出る）
    place(&mut s, &id);
    let path = dir.0.join("shelf.ylp");
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(!s.modified, "{}", s.message);
    let mut again = AppState::new(64, 64);
    again.apply(Action::OpenProject(path));
    assert!(again.shelf.resources().is_empty());
    again.shelf.use_language(Lang::En);
    assert_eq!(builtin_ids(&again).len(), 14);
}

#[test]
fn headless_bundled_items_are_inspected_with_layers_and_no_size() {
    let mut s = AppState::new(64, 64);
    s.shelf.use_language(Lang::En);
    for id in builtin_ids(&s) {
        s.shelf.inspect(&id);
        let info = s.shelf.info(&id).expect("見た");
        assert!(info.layers >= 2 && info.block.is_none(), "{id}");
        assert!(!s.shelf.warns(&id) && s.shelf.block_of(&id).is_none());
        let detail = s.shelf.detail(Lang::En, s.shelf.get(&id).unwrap());
        assert!(detail.contains("layers"), "{detail}");
        assert!(!detail.contains('×'), "素材の大きさは出さない: {detail}");
        assert!(detail.contains("Color"), "{detail}");
    }
}

#[test]
fn headless_a_placed_bundled_material_survives_save_and_reopen_and_stays_editable() {
    let dir = TempDir::new("reopen");
    let path = dir.0.join("material.ylp");
    let mut s = AppState::new(128, 96);
    s.shelf.use_language(Lang::Ja);
    for id in ["builtin:rusty-iron", "builtin:wood"] {
        place(&mut s, id);
    }
    let layers: Vec<String> = s.doc.layers().iter().map(|l| l.name().to_owned()).collect();
    let composite = color(&s);
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    // 位置のマップが無くても UV で評価されるので、効かない効果の知らせは出ない
    assert!(!s.message.contains("効いていない"), "{}", s.message);
    let mut again = AppState::new(64, 64);
    again.apply(Action::OpenProject(path));
    let set = again.sets.get(0).unwrap();
    assert!(set.read_only.is_none(), "{:?}", set.read_only);
    let reopened: Vec<String> = again
        .doc
        .layers()
        .iter()
        .map(|l| l.name().to_owned())
        .collect();
    assert_eq!(reopened, layers);
    assert_eq!(color(&again), composite);
    // 開き直した文書も編集できて、1 回の Undo で戻る
    let before = color(&again);
    let top = again.doc.layers().last().unwrap().id();
    again.doc.set_layer_visible(top, false).unwrap();
    assert_ne!(color(&again), before);
    assert!(again.doc.undo().unwrap());
    assert_eq!(color(&again), before);
}
