//! レイヤーセット（正本の版 36、頭の `layer_sets`・`route` とレイヤーの末尾の `set`）。既定の形（名前の空の 1 つに全部のチャンネルを割り当てた形）は
//! 前の版のまま同じバイト列。往復・版の選び方・壊れた割り当てと所属の拒否・.ylp への保存を試す。
use yolu_core::{Channel, Document, LayerId, LayerSetId, Rgba8};
use yolu_io::{
    MaterialRef, NativeDocument, NativeValue, Project, SaveTarget, SetSpec, WriterInfo,
    LAYER_SETS_VERSION, RULERS_VERSION, UNITY_NATIVE_VERSION,
};

fn writer() -> WriterInfo {
    WriterInfo {
        app: "YoluPainter-rs".into(),
        version: "0.0.1".into(),
        unity: "none".into(),
    }
}

fn paint(d: &mut Document, id: LayerId, c: Rgba8, x0: u32, x1: u32) {
    for y in 0..d.height() {
        for x in x0..x1 {
            d.set_pixel(id, x, y, c).unwrap();
        }
    }
}

/// 下地と、グループの中の 1 枚（最初のレイヤーセット）。
fn plain() -> (Document, LayerId, LayerId) {
    let mut d = Document::with_tile_size(32, 16, 8).unwrap();
    let a = d.add_layer("下地").unwrap();
    paint(&mut d, a, Rgba8::new(200, 30, 30, 255), 0, 32);
    let b = d.add_layer("上").unwrap();
    paint(&mut d, b, Rgba8::new(20, 30, 200, 128), 4, 20);
    d.group_layers(&[b], "グループ").unwrap();
    d.clear_history().unwrap();
    (d, a, b)
}

/// レイヤーセットを 2 つ（名前つき）にし、Color を 2 つ目から、Height を既定値から取る文書。2 つ目にはグループごとのレイヤーと、新しいレイヤー。
fn with_sets() -> (Document, LayerSetId, LayerSetId) {
    let (mut d, _, b) = plain();
    let first = d.layer_sets()[0].id;
    d.rename_layer_set(first, "マテリアル").unwrap();
    let shadow = d.add_layer_set("影のマスク").unwrap();
    let group = d.layer(b).unwrap().parent().unwrap();
    d.move_layers_to_set(&[group], shadow).unwrap();
    let extra = d.add_layer_above("影", Some(group)).unwrap();
    paint(&mut d, extra, Rgba8::new(0, 0, 0, 90), 10, 30);
    d.set_channel_output(Channel::Color, Some(shadow)).unwrap();
    d.set_channel_output(Channel::Height, None).unwrap();
    (d, first, shadow)
}

fn composites(d: &Document) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for c in d.channels() {
        out.push(d.composite_channel(c, d.bounds()).unwrap());
        for s in d.layer_sets() {
            out.push(d.composite_set_channel(s.id, c, d.bounds()).unwrap());
        }
    }
    out
}

#[test]
fn the_default_form_keeps_the_old_version_and_the_same_bytes() {
    let (mut d, _, _) = plain();
    let before = NativeDocument::from_core(&d).unwrap();
    assert_eq!(before.version(), UNITY_NATIVE_VERSION);
    assert!(before.field("layer_set_count").is_none());
    assert!(before.field("layers[0].set").is_none());
    // 足して消す・名前を付けて戻す・割り当てを変えて戻すと、前と同じバイト列
    let first = d.layer_sets()[0].id;
    let other = d.add_layer_set("x").unwrap();
    assert_eq!(
        NativeDocument::from_core(&d).unwrap().version(),
        LAYER_SETS_VERSION
    );
    d.remove_layer_set(other).unwrap();
    d.rename_layer_set(first, "名前").unwrap();
    assert_eq!(
        NativeDocument::from_core(&d).unwrap().version(),
        LAYER_SETS_VERSION,
        "名前の付いたレイヤーセット"
    );
    d.rename_layer_set(first, "").unwrap();
    d.set_channel_output(Channel::Metallic, None).unwrap();
    assert_eq!(
        NativeDocument::from_core(&d).unwrap().version(),
        LAYER_SETS_VERSION,
        "割り当ての無いチャンネル"
    );
    d.set_channel_output(Channel::Metallic, Some(first))
        .unwrap();
    let after = NativeDocument::from_core(&d).unwrap();
    assert_eq!(after.to_bytes(), before.to_bytes());
    // 前の版の正本は、名前の空のレイヤーセット 1 つに全部が入る形で読む
    let back = before.to_core().unwrap();
    assert!(back.has_default_layer_sets());
    assert_eq!(composites(&back), composites(&d));
}

#[test]
fn layer_sets_round_trip_in_version_36() {
    let (d, first, shadow) = with_sets();
    let native = NativeDocument::from_core(&d).unwrap();
    assert_eq!(native.version(), LAYER_SETS_VERSION);
    const { assert!(LAYER_SETS_VERSION > RULERS_VERSION) };
    let again = NativeDocument::read(&native.to_bytes()).unwrap();
    assert_eq!(again.to_bytes(), native.to_bytes());
    let back = again.to_core().unwrap();
    assert_eq!(back.layer_sets(), d.layer_sets());
    assert_eq!(back.layer_sets()[0].id, first);
    for c in d.channels() {
        assert_eq!(
            back.channel_output(c).unwrap(),
            d.channel_output(c).unwrap()
        );
    }
    assert_eq!(back.channel_output(Channel::Color).unwrap(), Some(shadow));
    assert_eq!(back.channel_output(Channel::Height).unwrap(), None);
    let members = |d: &Document| {
        d.layers()
            .iter()
            .map(|l| (l.id(), l.parent(), l.layer_set()))
            .collect::<Vec<_>>()
    };
    assert_eq!(members(&back), members(&d));
    assert_eq!(composites(&back), composites(&d));
    // 書き直しても同じバイト列
    assert_eq!(
        NativeDocument::from_core(&back).unwrap().to_bytes(),
        native.to_bytes()
    );
}

#[test]
fn broken_routes_and_memberships_are_refused() {
    let (d, _, _) = with_sets();
    let native = NativeDocument::from_core(&d).unwrap();
    let unknown = NativeValue::Guid([7; 16]);
    let set_of = |path: &str| match native.field(path) {
        Some(v) => v.clone(),
        None => panic!("{path}"),
    };
    // グループ（layers[2]）は 2 つ目、その子（layers[1]）を 1 つ目へ
    let first = set_of("layers[0].set");
    assert_ne!(set_of("layers[1].set"), first);
    for (path, value) in [
        ("layers[0].set", unknown.clone()),
        ("route[0].set", unknown.clone()),
        ("layers[1].set", first.clone()),
        ("layer_sets[1].id", set_of("layer_sets[0].id")),
        ("layer_sets[0].id", NativeValue::Guid([0; 16])),
        ("route[1].channel", NativeValue::Int(2)),
        ("layer_sets[1].name", NativeValue::Text("a\u{7}b".into())),
    ] {
        assert!(
            native.with_value(path, value.clone()).is_err(),
            "{path} = {value:?}"
        );
    }
    // 割り当ての数が文書のチャンネルの数と違えば断る
    let mut bytes = native.to_bytes();
    let at = route_count_offset(&native);
    bytes[at..at + 4].copy_from_slice(&5i32.to_le_bytes());
    assert!(NativeDocument::read(&bytes).is_err());
    // 版 36 の欄は、前の版の数では読めない（並びが合わない）
    let mut older = native.to_bytes();
    older[8..12].copy_from_slice(&RULERS_VERSION.to_le_bytes());
    assert!(NativeDocument::read(&older).is_err());
}

/// `route_count` の int の位置（頭の項目のバイト数を足す）。
fn route_count_offset(native: &NativeDocument) -> usize {
    let mut at = 0;
    for f in native.fields() {
        if f.path == "route_count" {
            return at;
        }
        let mut b = Vec::new();
        match &f.value {
            NativeValue::Int(v) => b.extend(v.to_le_bytes()),
            NativeValue::Byte(v) => b.push(*v),
            NativeValue::Bool(v) => b.push(u8::from(*v)),
            NativeValue::Float(v) => b.extend(v.to_le_bytes()),
            NativeValue::Guid(v) => b.extend(v),
            NativeValue::Text(v) => {
                b.extend((v.len() as i32).to_le_bytes());
                b.extend(v.as_bytes());
            }
            NativeValue::Bytes(v) => b.extend(v.iter()),
        }
        at += b.len();
    }
    panic!("route_count が無い")
}

#[test]
fn a_ylp_keeps_the_layer_sets() {
    let (d, _, _) = with_sets();
    let native = NativeDocument::from_core(&d).unwrap();
    let spec = SetSpec {
        id: "0f0f0f0f-0000-4000-8000-000000000036".into(),
        name: "Set".into(),
        material: MaterialRef::Unassigned,
        document: Some(native.clone().into()),
        composites: vec![],
    };
    let project = Project::create(writer(), std::slice::from_ref(&spec), &spec.id).unwrap();
    let dir = std::env::temp_dir().join(format!("yolu-io-layer-sets-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("layer-sets.ylp");
    let _ = std::fs::remove_file(&path);
    SaveTarget::create(&path).unwrap().save(&project).unwrap();
    let (again, _) = SaveTarget::open(&path).unwrap();
    let reopened = &again.sets()[0].document;
    assert_eq!(reopened.version(), LAYER_SETS_VERSION);
    assert_eq!(reopened.to_bytes().unwrap(), native.to_bytes());
    let core = reopened.to_core().unwrap();
    assert_eq!(core.layer_sets(), d.layer_sets());
    assert_eq!(composites(&core), composites(&d));
    std::fs::remove_dir_all(&dir).unwrap();
}
