//! セットの見た目の設定（`sets/<ID>/look.json`）: 書く・読む・消す、ほかのエントリをバイト列のまま残す、読めないものは残して断る、
//! セットを消すと一緒に消える。
use std::collections::BTreeMap;

use yolu_core::look::{LookKind, LookValue, MaterialLook, PlaneSource, TextureSource};
use yolu_core::{Channel, Document};
use yolu_io::{MaterialRef, NativeDocument, Note, Project, SetSpec, WriterInfo};

fn writer() -> WriterInfo {
    WriterInfo {
        app: "試験の書き手".into(),
        version: "0.0.1".into(),
        unity: "standalone".into(),
    }
}

const A: &str = "0f1e2d3c-4b5a-4978-8796-a5b4c3d2e1f0";
const B: &str = "11111111-2222-4333-8444-555555555555";

fn spec(id: &str, name: &str) -> SetSpec {
    let doc = Document::new(32, 32).unwrap();
    SetSpec {
        id: id.into(),
        name: name.into(),
        material: MaterialRef::Material {
            name: name.into(),
            asset: None,
        },
        document: Some(NativeDocument::from_core(&doc).unwrap()),
        composites: Vec::new(),
    }
}

fn project() -> Project {
    Project::create(writer(), &[spec(A, "Skin"), spec(B, "Hair")], A).unwrap()
}

fn lil() -> MaterialLook {
    let mut look = MaterialLook {
        kind: LookKind::LilToon,
        shader: "Hidden/lilToonOutline".into(),
        ..MaterialLook::default()
    };
    look.properties.insert("_UseShadow".into(), LookValue::Int(1));
    look.properties
        .insert("_ShadowColor".into(), LookValue::Color([0.8, 0.7, 0.9, 1.0]));
    look.textures
        .insert("_MainTex".into(), TextureSource::Channel(Channel::Color));
    look.textures.insert(
        "_ShadowBorderMask".into(),
        TextureSource::Packed([
            PlaneSource::Channel {
                channel: Channel::from_index(6).unwrap(),
                component: 0,
            },
            PlaneSource::One,
            PlaneSource::One,
            PlaneSource::One,
        ]),
    );
    look
}

fn entries(p: &Project) -> BTreeMap<String, Vec<u8>> {
    p.migrated_entries()
        .iter()
        .map(|(k, v)| (k.clone(), v.to_vec()))
        .collect()
}

#[test]
fn a_look_is_written_read_back_and_removed_without_touching_the_rest() {
    let p = project();
    assert_eq!(p.look(A).unwrap(), None);
    let before = entries(&p);
    let q = p.with_look(A, Some(&lil())).unwrap();
    let name = format!("sets/{A}/look.json");
    // ほかのエントリはバイト列のまま
    let after = entries(&q);
    for (k, v) in &before {
        assert_eq!(after.get(k), Some(v), "{k}");
    }
    assert!(after.contains_key(&name));
    assert_eq!(after.len(), before.len() + 1);
    // 知っているエントリ（知らないエントリの知らせに出ない）
    assert!(q.unknown_entries().is_empty(), "{:?}", q.unknown_entries());
    assert!(!q.notes().iter().any(|n| matches!(n, Note::UnknownEntryKept(_))));
    // ファイルにして読み直しても同じ
    let reread = Project::read(&q.to_bytes().unwrap()).unwrap();
    assert_eq!(reread.look(A).unwrap(), Some(lil()));
    assert_eq!(reread.look(B).unwrap(), None);
    // 消す
    let r = reread.with_look(A, None).unwrap();
    assert_eq!(r.look(A).unwrap(), None);
    assert_eq!(entries(&r), before);
}

#[test]
fn an_unreadable_look_is_refused_and_its_bytes_are_kept() {
    let p = project().with_look(A, Some(&lil())).unwrap();
    // 新しい形式の look.json を差し込む（ほかの書き手が書いた想定）
    let mut files: BTreeMap<String, Vec<u8>> = p
        .original_archive()
        .entries()
        .iter()
        .map(|(k, v)| (k.clone(), v.to_vec()))
        .collect();
    let name = format!("sets/{A}/look.json");
    let newer = br#"{"format": 9, "kind": "lilToon", "fromTheFuture": true}"#.to_vec();
    files.insert(name.clone(), newer.clone());
    let archive = yolu_io::Archive::from_entries(files).unwrap();
    let q = Project::read(&archive.to_bytes().unwrap()).unwrap();
    assert!(q.look(A).is_err(), "読めない設定は断る（黙って既定にしない）");
    // 開けて、ほかのセットや正本は読める。エントリはバイト列のまま残る
    assert_eq!(q.sets().len(), 2);
    assert_eq!(q.migrated_entries()[&name].to_vec(), newer);
    let reread = Project::read(&q.to_bytes().unwrap()).unwrap();
    assert_eq!(reread.migrated_entries()[&name].to_vec(), newer);
}

#[test]
fn rewriting_keeps_unknown_keys_only_from_the_same_format() {
    let p = project().with_look(A, Some(&lil())).unwrap();
    let name = format!("sets/{A}/look.json");
    let with_entry = |bytes: &[u8]| {
        let mut files: BTreeMap<String, Vec<u8>> = p
            .original_archive()
            .entries()
            .iter()
            .map(|(k, v)| (k.clone(), v.to_vec()))
            .collect();
        files.insert(name.clone(), bytes.to_vec());
        Project::read(&yolu_io::Archive::from_entries(files).unwrap().to_bytes().unwrap()).unwrap()
    };
    let json = |p: &Project| -> serde_json::Value { serde_json::from_slice(&p.migrated_entries()[&name]).unwrap() };
    // 同じ形式の中で足されたキーは、書き直しても残る
    let same = with_entry(br#"{"format": 1, "kind": "lilToon", "addedLater": [1, 2]}"#);
    let r = same.with_look(A, Some(&lil())).unwrap();
    assert_eq!(json(&r)["addedLater"], serde_json::json!([1, 2]));
    assert_eq!(r.look(A).unwrap(), Some(lil()));
    // 新しい形式（読めない）のエントリを上書きするときは、その形式のキーを残さない（形式 1 の本体と混ぜない）
    let newer = with_entry(br#"{"format": 9, "kind": "lilToon", "fromTheFuture": true, "properties": {"_X": {"half": 1}}}"#);
    assert!(newer.look(A).is_err());
    let r = newer.with_look(A, Some(&lil())).unwrap();
    let v = json(&r);
    assert_eq!(v["format"], serde_json::json!(1));
    assert!(v.get("fromTheFuture").is_none(), "{v}");
    assert_eq!(r.look(A).unwrap(), Some(lil()));
    // 壊れたエントリ（JSON でない）も同じ
    let broken = with_entry(b"not json");
    let r = broken.with_look(A, Some(&lil())).unwrap();
    assert_eq!(r.look(A).unwrap(), Some(lil()));
}

#[test]
fn dropping_a_set_drops_its_look() {
    let p = project()
        .with_look(A, Some(&lil()))
        .unwrap()
        .with_look(B, Some(&lil()))
        .unwrap();
    let mut keep = spec(A, "Skin");
    keep.document = None;
    let q = p.with_sets_dropping(writer(), &[keep], A, &[B]).unwrap();
    assert!(!q
        .migrated_entries()
        .contains_key(&format!("sets/{B}/look.json")));
    assert_eq!(q.look(A).unwrap(), Some(lil()));
    assert!(q.look(B).is_err(), "消したセットは無い");
}

#[test]
fn the_look_is_refused_for_missing_sets_and_kept_across_set_edits() {
    let p = project().with_look(A, Some(&lil())).unwrap();
    assert!(p.with_look("33333333-3333-4333-8333-333333333333", Some(&lil())).is_err());
    // セットの名前・並びを変えても残る
    let mut a = spec(A, "Skin 2");
    a.document = None;
    let mut b = spec(B, "Hair");
    b.document = None;
    let q = p.with_sets(writer(), &[b, a], B).unwrap();
    assert_eq!(q.look(A).unwrap(), Some(lil()));
    // 形の検査に通らない設定は書かない
    let mut bad = lil();
    bad.properties
        .insert("_X".into(), LookValue::Float(f32::INFINITY));
    assert!(p.with_look(A, Some(&bad)).is_err());
}

fn received() -> yolu_core::look::ReceivedLook {
    let mut look = lil();
    look.shader = "Hidden/lilToonTransparent".into();
    look.properties
        .insert("_ShadowBorder".into(), LookValue::Float(0.3));
    let mut r = yolu_core::look::ReceivedLook {
        look,
        source: "lilToon 2.3.4 · Standard/Transparent".into(),
        ..Default::default()
    };
    r.images.insert(
        "_MatCapTex".into(),
        std::sync::Arc::new(yolu_core::look::ReceivedImage {
            width: 1,
            height: 1,
            srgb: true,
            pixels: vec![1, 2, 3, 4].into(),
        }),
    );
    r.missing.insert(
        "_ShadowColorTex".into(),
        yolu_core::look::MissingImage::OverBudget,
    );
    r
}

#[test]
fn received_values_are_kept_beside_the_users_look_and_images_are_not_written() {
    use yolu_core::look::MissingImage;
    let p = project();
    let name = format!("sets/{A}/look.json");
    // 受けた見た目だけ: エントリを作り、利用者の設定は既定のまま
    let q = p.with_received_look(A, Some(&received())).unwrap();
    let back = q.received_look(A).unwrap().expect("受けた見た目");
    assert_eq!(back.look, received().look);
    assert_eq!(back.source, received().source);
    assert!(back.images.is_empty(), "絵の画素は書かない");
    assert_eq!(back.missing["_MatCapTex"], MissingImage::Pending, "絵のあったスロットは届いていない");
    assert_eq!(back.missing["_ShadowColorTex"], MissingImage::OverBudget);
    assert!(q.look(A).unwrap().is_none_or(|l| l.is_default()));
    assert_eq!(q.received_look(B).unwrap(), None);
    // 利用者の設定を書いても、受けた見た目は残る。既定に戻しても（None）、受けた見た目があればエントリは残る
    let mine = MaterialLook {
        kind_chosen: true,
        ..MaterialLook::default()
    };
    let r = q.with_look(A, Some(&mine)).unwrap();
    assert_eq!(r.look(A).unwrap(), Some(mine));
    assert_eq!(r.received_look(A).unwrap(), Some(back.clone()));
    let r = r.with_look(A, None).unwrap();
    assert!(entries(&r).contains_key(&name));
    assert_eq!(r.received_look(A).unwrap(), Some(back.clone()));
    // 受けた見た目を外すと、利用者の設定が既定ならエントリごと消える
    let gone = r.with_received_look(A, None).unwrap();
    assert!(!entries(&gone).contains_key(&name));
    // 利用者の設定があれば、受けた見た目だけを外す
    let kept = q
        .with_look(A, Some(&lil()))
        .unwrap()
        .with_received_look(A, None)
        .unwrap();
    assert_eq!(kept.look(A).unwrap(), Some(lil()));
    assert_eq!(kept.received_look(A).unwrap(), None);
}

#[test]
fn a_broken_received_section_is_refused_and_kept() {
    let p = project().with_received_look(A, Some(&received())).unwrap();
    let name = format!("sets/{A}/look.json");
    let mut v: serde_json::Value = serde_json::from_slice(&entries(&p)[&name]).unwrap();
    v["received"]["missing"]["_MatCapTex"] = serde_json::json!("lost");
    let broken = serde_json::to_vec(&v).unwrap();
    assert!(yolu_io::look::read_received(&broken).is_err());
    // 利用者の設定は読める（壊れているのは受けた見た目だけ）
    assert!(yolu_io::look::read(&broken).is_ok());
    v["received"] = serde_json::json!(3);
    assert!(yolu_io::look::read_received(&serde_json::to_vec(&v).unwrap()).is_err());
    // 受けた見た目の無いエントリ
    assert_eq!(
        yolu_io::look::read_received(&yolu_io::look::write(&lil(), None).unwrap()).unwrap(),
        None
    );
}
