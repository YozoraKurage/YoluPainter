//! 版 26（分けた正本）と、流して作る・読む正本の試験。巨大な文書は作らず、閾値（`Thresholds`）を小さくして同じ道を通す。
use super::*;
use crate::{Limits, NativeDocument, Package};

fn fixture(name: &str) -> NativeDocument {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    NativeDocument::read(&std::fs::read(path).unwrap()).unwrap()
}
/// `Bytes` の値（色・画素）を持つ項目の種類を全部含む見本（タイル・マスク・塗りつぶしの色・ランプ・パス・ユーザーチャンネルの既定）。
const FIXTURES: [&str; 8] = [
    "native-rich-v21.utpaint",
    "m2-masks.utpaint",
    "m2-groups.utpaint",
    "effects-paths.utpaint",
    "effects-generators.utpaint",
    "effects-fills.utpaint",
    "user-channels-v22.utpaint",
    "adjust-v24.utpaint",
];
/// 小さな閾値（数 KiB の文書を分ける）。
fn tiny(part_bytes: u64, part_min: u64) -> Thresholds {
    Thresholds {
        split_above: 1000,
        part_bytes,
        part_min,
        ..Thresholds::REAL
    }
}
fn without_values(fields: &[NativeField]) -> Vec<NativeField> {
    fields
        .iter()
        .filter(|f| !matches!(f.value, NativeValue::Bytes(_)))
        .cloned()
        .collect()
}
fn stored(entries: &[(String, Blob)]) -> StoredDoc {
    StoredDoc {
        header: entries[0].1.clone(),
        parts: entries[1..].iter().map(|(_, b)| b.clone()).collect(),
    }
}
fn largest_tile(doc: &NativeDocument) -> u64 {
    doc.fields()
        .iter()
        .filter_map(|f| match &f.value {
            NativeValue::Bytes(b) => Some(b.len() as u64),
            _ => None,
        })
        .max()
        .unwrap_or(4)
}

#[test]
fn splitting_a_native_document_reads_back_the_same_fields() {
    for name in FIXTURES {
        let doc = fixture(name);
        for (part, min) in [
            (largest_tile(&doc), 0),
            (largest_tile(&doc) * 3, 1 << 30),
            (1 << 20, 64),
        ] {
            let t = tiny(part, min);
            let entries = t.scoped(|| native_entries(&doc, "sets/x/"));
            assert!(entries.len() >= 2, "{name}: 分けていない");
            assert_eq!(entries[0].0, "sets/x/document.utpaint");
            for (k, (n, b)) in entries[1..].iter().enumerate() {
                assert_eq!(n, &format!("sets/x/document.utpaint.{}", k + 1));
                assert!(
                    !b.is_empty() && b.len() <= part.max(largest_tile(&doc)),
                    "{name}: 部分 {k} の長さ {}",
                    b.len()
                );
            }
            let header = entries[0].1.bytes().unwrap();
            assert_eq!(&header[..8], b"DOTPAINT");
            assert_eq!(i32::from_le_bytes(header[8..12].try_into().unwrap()), 26);
            assert_eq!(
                i32::from_le_bytes(header[12..16].try_into().unwrap()),
                doc.version()
            );
            // 分けた正本より前の読み手は版の数で断る（0.4.x の上限は 25、Unity 版 0.2.0 は 21）。今の読み手は 26 を分けた正本の識別として先に見る
            // （機能の版は 26 を飛ばして 27〜30・32・33）
            const { assert!(SPLIT_VERSION > crate::MIXING_VERSION) };
            // 中の版は意味の決まった版（1〜25・27〜30・32・33）だけ。分けた正本の識別（26）と、決めていない版（31）は、中身を読む前に断る
            for bad in [SPLIT_VERSION, 31] {
                let mut bytes = header.to_vec();
                bytes[12..16].copy_from_slice(&bad.to_le_bytes());
                let mut changed = stored(&entries);
                changed.header = bytes.into();
                let Err(e) = changed.to_native() else {
                    panic!("{name}: 中の版 {bad} を読めた");
                };
                assert!(
                    e.to_string().contains(&format!("中の版 {bad}")),
                    "{name}: 中の版 {bad} の断り: {e}"
                );
            }
            let back = stored(&entries).to_native().unwrap();
            assert_eq!(back.fields(), doc.fields(), "{name}");
            assert_eq!(back.to_bytes(), doc.to_bytes(), "{name}");
            // 骨組み（中身を読む・長さだけ）
            let s = stored(&entries);
            assert_eq!(
                s.skeleton(true).unwrap().fields(),
                &without_values(doc.fields())[..],
                "{name}"
            );
            assert_eq!(
                s.skeleton(false).unwrap().fields(),
                &without_values(doc.fields())[..],
                "{name}"
            );
        }
        // 閾値より小さければ分けない（今と同じ 1 つのエントリ・同じバイト列）
        let entries = native_entries(&doc, "");
        assert_eq!(entries.len(), 1);
        assert_eq!(&entries[0].1.bytes().unwrap()[..], &doc.to_bytes()[..]);
    }
}

#[test]
fn a_core_document_streams_the_same_bytes_as_from_core() {
    for name in FIXTURES {
        let native = fixture(name);
        let Ok(core) = native.to_core() else { continue };
        let expected = NativeDocument::from_core(&core).unwrap();
        let doc = Arc::new(core);
        // 分けない: 1 つのエントリが from_core と同じバイト列、SHA-256 も同じ
        let made = CoreDoc::new(doc.clone()).unwrap();
        let entries = made.entries("sets/x/");
        assert_eq!(entries.len(), 1, "{name}");
        let bytes = entries[0].1.bytes().unwrap();
        assert_eq!(&bytes[..], &expected.to_bytes()[..], "{name}");
        assert_eq!(entries[0].1.sha256().unwrap(), crate::hash(&bytes));
        assert_eq!(entries[0].1.len(), bytes.len() as u64);
        assert_eq!(
            made.skeleton().unwrap().fields(),
            &without_values(expected.fields())[..]
        );
        // 分ける: メモリの正本を分けたものとバイトまで同じ（同じ区切り）
        for (part, min) in [
            (largest_tile(&expected), 0),
            (largest_tile(&expected) * 5, 1 << 30),
        ] {
            let t = tiny(part, min);
            let (made, from_native) = t.scoped(|| {
                (
                    CoreDoc::new(doc.clone()).unwrap(),
                    native_entries(&expected, "sets/x/"),
                )
            });
            let entries = made.entries("sets/x/");
            assert_eq!(entries.len(), from_native.len(), "{name}");
            for ((n, b), (m, c)) in entries.iter().zip(&from_native) {
                assert_eq!(n, m);
                let written = b.bytes().unwrap();
                assert_eq!(written, c.bytes().unwrap(), "{name} {n}");
                assert_eq!(b.len(), written.len() as u64, "{name} {n}");
                assert_eq!(b.sha256().unwrap(), crate::hash(&written), "{name} {n}");
            }
            assert_eq!(
                stored(&entries).to_native().unwrap().fields(),
                expected.fields()
            );
        }
    }
}

/// 層の後の手動の ID の色の `tag`（`Bytes` の値）は最後の層の値として数える。区切りは層の始まりで、その層の値の合計を閾値と比べて決めるので、
/// 数え方が 2 つの作り方（core の文書から流す・メモリの正本を分ける）で違うと、閾値によって区切りが食い違う（`tag` だけを別の層の始まりと
/// 見る・最後の層の合計が境目で変わる）。閾値を細かく動かして、どちらも同じバイト列になることを見る。
#[test]
fn the_tag_after_the_layers_counts_for_the_last_layer_in_both_ways_of_splitting() {
    let native = fixture("native-rich-v21.utpaint");
    let core = native.to_core().unwrap();
    assert!(!core.id_colors().colors().is_empty());
    let expected = NativeDocument::from_core(&core).unwrap();
    let doc = Arc::new(core);
    let mut splits = std::collections::BTreeSet::new();
    for part_min in (0..=600).chain([1 << 12, 1 << 14, 1 << 20]) {
        let t = tiny(1 << 20, part_min);
        let (made, from_native) = t.scoped(|| {
            (
                CoreDoc::new(doc.clone()).unwrap(),
                native_entries(&expected, "sets/x/"),
            )
        });
        let entries = made.entries("sets/x/");
        splits.insert(entries.len());
        assert_eq!(entries.len(), from_native.len(), "part_min {part_min}");
        for ((n, b), (_, c)) in entries.iter().zip(&from_native) {
            assert_eq!(
                b.bytes().unwrap(),
                c.bytes().unwrap(),
                "part_min {part_min} {n}"
            );
        }
    }
    // 閾値によって部分の数が変わるところまで試した
    assert!(splits.len() > 1, "{splits:?}");
}

#[test]
fn streaming_into_core_matches_reading_the_whole_document() {
    for name in FIXTURES {
        let native = fixture(name);
        let Ok(core) = native.to_core() else { continue };
        let expected = NativeDocument::from_core(&core).unwrap().to_bytes();
        // 置いてある正本（分けない・分けた）から流して
        for t in [Thresholds::REAL, tiny(largest_tile(&native), 0)] {
            let entries = t.scoped(|| native_entries(&native, ""));
            let s = stored(&entries);
            let skeleton = s.skeleton(true).unwrap();
            let streamed = s.to_core(&skeleton, None).unwrap();
            assert_eq!(
                NativeDocument::from_core(&streamed).unwrap().to_bytes(),
                expected,
                "{name}"
            );
        }
        // core の文書から作る正本を、そのまま流して
        let made = tiny(largest_tile(&native), 0).scoped(|| CoreDoc::new(Arc::new(core)).unwrap());
        let again = made.to_core(None).unwrap();
        assert_eq!(
            NativeDocument::from_core(&again).unwrap().to_bytes(),
            expected,
            "{name}"
        );
    }
}

#[test]
fn the_pixel_budget_stops_a_streamed_read() {
    let native = fixture("m2-masks.utpaint");
    let entries = tiny(largest_tile(&native), 0).scoped(|| native_entries(&native, ""));
    let s = stored(&entries);
    let skeleton = s.skeleton(true).unwrap();
    let Err(e) = s.to_core(&skeleton, Some(1)) else {
        panic!("予算で止まらない")
    };
    assert!(matches!(e, Error::Budget(_)), "{e:?}");
}

/// 部分の過不足・境目・余り・空の部分を断る。
#[test]
fn wrong_parts_are_refused() {
    let doc = fixture("native-rich-v21.utpaint");
    let t = tiny(largest_tile(&doc), 0);
    let (header, parts) = split_fields(doc.fields(), doc.version(), &t);
    assert!(parts.len() >= 3);
    let read = |header: &[u8], parts: &[Vec<u8>]| {
        let refs: Vec<&[u8]> = parts.iter().map(|p| &p[..]).collect();
        NativeDocument::read_split(header, &refs)
    };
    read(&header, &parts).unwrap();
    // 足りない・余計な部分（ヘッダーの数と合わない）
    assert!(read(&header, &parts[..parts.len() - 1]).is_err());
    let mut extra = parts.clone();
    extra.push(vec![0; 4]);
    assert!(read(&header, &extra).is_err());
    // 部分の数を偽ると、値が足りない・余る
    for count in [parts.len() as i32 - 1, parts.len() as i32 + 1] {
        let mut h = header.clone();
        h[16..20].copy_from_slice(&count.to_le_bytes());
        let n = (count as usize).min(parts.len());
        assert!(read(&h, &parts[..n]).is_err(), "数 {count}");
    }
    // 境目をずらす（値が 2 つの部分にまたがる）
    let mut moved = parts.clone();
    let b = moved[0].pop().unwrap();
    moved[1].insert(0, b);
    let e = read(&header, &moved).unwrap_err().to_string();
    assert!(e.contains("境目"), "{e}");
    // 最後の部分の余り
    let mut tail = parts.clone();
    tail.last_mut().unwrap().push(0);
    assert!(read(&header, &tail)
        .unwrap_err()
        .to_string()
        .contains("余り"));
    // 空の部分
    let mut empty = parts.clone();
    empty.insert(1, Vec::new());
    let mut h = header.clone();
    h[16..20].copy_from_slice(&(empty.len() as i32).to_le_bytes());
    assert!(read(&h, &empty).unwrap_err().to_string().contains("空"));
    // 分けていない正本に部分を添える・分けた正本に部分が無い
    assert!(NativeDocument::read_split(&doc.to_bytes(), &[&parts[0]]).is_err());
    assert!(NativeDocument::read(&header).is_err());
    // 中の版が分けた正本の版・割り振られていない版（31）・読み手より新しい版・古すぎる版
    for inner in [26i32, 31, crate::MAX_NATIVE_VERSION + 1, 20, 0] {
        let mut h = header.clone();
        h[12..16].copy_from_slice(&inner.to_le_bytes());
        let e = read(&h, &parts).unwrap_err().to_string();
        assert!(
            e.contains(&format!("中の版 {inner} は未対応")),
            "{inner}: {e}"
        );
    }
    // 長さだけの骨組みの読みも、数と境目は確かめる
    let s = StoredDoc {
        header: Blob::from(header.clone()),
        parts: moved.iter().map(|p| Blob::from(p.clone())).collect(),
    };
    assert!(s.skeleton(false).is_err());
}

#[test]
fn undefined_inner_versions_are_refused_too() {
    // 色調補正のフィルターだけの文書は、版 24 の並びが 25・27・28 と同じなので、中の版の数だけを書き換えても中身は読める（対照）。
    // 31 は番号だけで意味が決まっておらず、範囲の中でも断る（版 25 の並びとして読み進めない）
    let mut core = uniform_layers(16, 8, 1);
    let layer = core.layers()[0].id();
    let posterize =
        yolu_core::EffectSettings::from_catalog("posterize", &Default::default()).unwrap();
    core.add_filter(
        layer,
        yolu_core::FilterTarget::Content,
        yolu_core::FilterSpec::new(posterize).channels(&[yolu_core::Channel::Color]),
    )
    .unwrap();
    let doc = NativeDocument::from_core(&core).unwrap();
    assert_eq!(doc.version(), crate::ADJUST_VERSION);
    let t = tiny(largest_tile(&doc), 0);
    let (header, parts) = split_fields(doc.fields(), doc.version(), &t);
    let read = |inner: i32| {
        let mut h = header.clone();
        h[12..16].copy_from_slice(&inner.to_le_bytes());
        let refs: Vec<&[u8]> = parts.iter().map(|p| &p[..]).collect();
        NativeDocument::read_split(&h, &refs)
    };
    for inner in [
        crate::ADJUST_VERSION,
        crate::MIXING_VERSION,
        crate::PATHS_VERSION,
        crate::EFFECTS_VERSION,
        crate::POINT_GRADIENT_VERSION,
        crate::TEXT_VERSION,
    ] {
        assert_eq!(read(inner).unwrap().version(), inner);
    }
    // 意味の決まっていない版（31）は断る
    let inner = 31;
    let e = read(inner).err().map(|e| e.to_string());
    assert!(
        e.is_some_and(|e| e.contains(&inner.to_string())),
        "版 {inner} を断らなかった"
    );
}

#[test]
fn cut_groups_small_layers_keeps_big_layers_apart_and_never_splits_a_value() {
    let t = Thresholds {
        part_bytes: 100,
        part_min: 30,
        ..Thresholds::REAL
    };
    // 層の前の値は層 0 と同じ部分。小さな層（10）は 30 になるまでまとめる。大きな層（40）はひとりで。100 を超える手前で区切る
    let values = [
        (0, 4),
        (0, 10),
        (1, 10),
        (2, 10),
        (3, 10),
        (4, 40),
        (5, 10),
        (6, 60),
        (6, 60),
        (7, 5),
    ];
    let ranges = cut(&values, &t);
    assert_eq!(
        ranges,
        vec![
            (0, 34),
            (34, 44),
            (44, 84),
            (84, 94),
            (94, 154),
            (154, 214),
            (214, 219)
        ]
    );
    for w in ranges.windows(2) {
        assert_eq!(w[0].1, w[1].0);
    }
    assert!(ranges.iter().all(|(s, e)| e > s));
    // 区切りは値の境目だけ
    let mut edges = vec![0u64];
    for (_, n) in values {
        edges.push(edges.last().unwrap() + n as u64);
    }
    assert!(ranges
        .iter()
        .all(|(s, e)| edges.contains(s) && edges.contains(e)));
    assert!(cut(&[], &t).is_empty());
}

#[test]
fn a_document_with_nothing_but_plain_values_has_one_part_or_none() {
    // 値の無い文書（層 0・ユーザーチャンネル無し）は分ける理由が無い（閾値を超えないので分けない）
    let doc = Arc::new(yolu_core::Document::new(16, 16).unwrap());
    let made = tiny(1 << 20, 0).scoped(|| CoreDoc::new(doc).unwrap());
    assert!(made.plan().split.is_none());
}

/// 一様なタイル（core では 4 バイト、正本では全画素）だけの層を `layers` 枚持つ文書。
fn uniform_layers(size: u32, tile: u32, layers: usize) -> yolu_core::Document {
    let mut doc = yolu_core::Document::with_tile_size(size, size, tile).unwrap();
    let bytes = [10u8, 20, 30, 255].repeat((tile * tile) as usize);
    for i in 0..layers {
        let id = doc.add_layer(&format!("L{i}")).unwrap();
        for ty in 0..size / tile {
            for tx in 0..size / tile {
                doc.import_tile(
                    id,
                    yolu_core::Channel::Color,
                    yolu_core::TileCoord::new(tx, ty),
                    &bytes,
                )
                .unwrap();
            }
        }
    }
    doc.clear_history().unwrap();
    doc
}

/// メモリの正本は 512 MiB まで: 超える正本は、中身を読まずに予算で断る（大きな正本は流して読む）。
#[test]
fn a_document_too_big_for_memory_is_refused_before_reading() {
    // 置いてある正本: 長さだけで断る（ファイルは無い。読みに行けば NotFound になる）
    let missing = std::env::temp_dir().join("yolu-bigdoc-missing-part.bin");
    let fake = |len: u64| {
        Blob::file(
            missing.clone(),
            "sets/x/document.utpaint.1",
            len,
            "0".repeat(64),
        )
    };
    let s = StoredDoc {
        header: fake(100),
        parts: vec![fake(MAX_ONE_ENTRY / 2), fake(MAX_ONE_ENTRY / 2)],
    };
    let Err(e) = s.to_native() else {
        panic!("断らない")
    };
    assert!(
        matches!(&e, Error::Budget(why) if why == TOO_BIG_FOR_MEMORY),
        "{e:?}"
    );
    // core の文書から作る正本: 一様なタイルの層で正本だけが 512 MiB を超える（core の画素は小さい）
    let doc = uniform_layers(1024, 512, 130);
    assert!(doc.allocated_bytes() < 1 << 20);
    let made = CoreDoc::new(Arc::new(doc)).unwrap();
    assert!(made.plan().total() > MAX_ONE_ENTRY);
    let Err(e) = made.to_native() else {
        panic!("断らない")
    };
    assert!(matches!(e, Error::Budget(_)), "{e:?}");
}

/// 読み手の上限（予算から）を、保存の見積もり（`Limits::check`）が同じ数え方で言い当てる: 一様なタイルの多い文書は core の画素が
/// 予算の 4 分の 1 より十分小さくても、正本がセットごとの上限を超えうる。
#[test]
fn the_save_side_check_predicts_what_the_reader_refuses() {
    let doc = uniform_layers(128, 64, 10);
    let allocated = doc.allocated_bytes();
    let small = Thresholds {
        classic_total_bytes: 1 << 10,
        split_above: 64 << 10,
        part_bytes: 64 << 10,
        part_min: 0,
        ..Thresholds::REAL
    };
    let id = crate::guid(&crate::core_bridge::native_id(doc.id()));
    let writer = crate::WriterInfo {
        app: "試験".into(),
        version: "0".into(),
        unity: "standalone".into(),
    };
    let spec = crate::SetSpec {
        id: id.clone(),
        name: "一様".into(),
        material: crate::MaterialRef::PendingSlot(0),
        document: Some(DocumentSource::from_core(Arc::new(doc)).unwrap()),
        composites: Vec::new(),
    };
    let (project, bytes) = small.scoped(|| {
        let p = crate::Project::create(writer, &[spec], &id).unwrap();
        let b = p.to_bytes().unwrap();
        (p, b)
    });
    let document: u64 = project
        .original_archive()
        .entries()
        .iter()
        .filter(|(n, _)| n.contains("document.utpaint"))
        .map(|(_, b)| b.len())
        .sum();
    assert!(
        document > 4 * allocated * 1000,
        "正本 {document}・core {allocated}"
    );
    let tight = Limits {
        document_bytes: document - 1,
        other_bytes: crate::package::OTHER_BYTES,
    };
    let entries = || {
        project
            .original_archive()
            .entries()
            .iter()
            .map(|(n, b)| (n.as_str(), b.len()))
            .collect::<Vec<_>>()
    };
    let predicted = tight.check(entries()).unwrap_err();
    let refused = Package::read_bytes(&bytes, &tight).unwrap_err();
    assert_eq!(predicted.to_string(), refused.to_string());
    assert!(
        refused
            .to_string()
            .contains(crate::OVER_LAYER_PIXELS_DOCUMENT),
        "{refused}"
    );
    assert!(
        !refused.to_string().contains("MiB") && !refused.to_string().contains(&id),
        "{refused}"
    );
    // 足りる上限なら、見積もりも読み手も通る
    let enough = Limits {
        document_bytes: document,
        other_bytes: crate::package::OTHER_BYTES,
    };
    enough.check(entries()).unwrap();
    Package::read_bytes(&bytes, &enough).unwrap();
}

/// 保存した後のプロジェクト（書いたファイルを指す）は、外側・移行後のエントリ・セットの正本が同じエントリを持つ: 次の作り直し
/// （書き置き・保存の `with_sets`）で、変わっていない分けない正本の骨組みを読み直さない。
#[test]
fn after_a_save_an_unchanged_document_is_reused_without_reading_it_again() {
    let dir = std::env::temp_dir().join(format!("yolu-bigdoc-reuse-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let native = fixture("m2-masks.utpaint");
    let id = native.id().to_owned();
    let writer = crate::WriterInfo {
        app: "試験".into(),
        version: "0".into(),
        unity: "standalone".into(),
    };
    let spec = |document: Option<DocumentSource>| crate::SetSpec {
        id: id.clone(),
        name: "見本".into(),
        material: crate::MaterialRef::PendingSlot(0),
        document,
        composites: Vec::new(),
    };
    // 開いたファイルのエントリはメモリに残さない（分けない正本も .ylp の中の位置で持つ）
    let keep_out = Thresholds {
        keep_in_memory: 0,
        ..Thresholds::REAL
    };
    keep_out.scoped(|| {
        let project =
            crate::Project::create(writer.clone(), &[spec(Some(native.clone().into()))], &id)
                .unwrap();
        let path = dir.join("再利用.ylp");
        let mut target = crate::SaveTarget::create(&path).unwrap();
        let saved = target
            .save_with(&project, crate::BackupKeep::All)
            .unwrap()
            .project
            .unwrap();
        let document = &saved.sets()[0].document;
        assert!(!document.is_split());
        let entry = &saved.original_archive().entries()[&format!("sets/{id}/document.utpaint")];
        assert!(entry.in_memory().is_none());
        assert!(document.placed_as(&[entry]), "外側と正本が同じエントリ");
        let rebuilt = saved.with_sets(writer.clone(), &[spec(None)], &id).unwrap();
        assert!(
            rebuilt.sets()[0].document.same_as(document),
            "骨組みを読み直さずに同じ正本を使う"
        );
        // もう 1 度保存しても同じ（保存した後のプロジェクトどうしでも）
        let again = target
            .save_with(&rebuilt, crate::BackupKeep::Count(0))
            .unwrap()
            .project
            .unwrap();
        let next = again.with_sets(writer, &[spec(None)], &id).unwrap();
        assert!(next.sets()[0].document.same_as(&again.sets()[0].document));
        assert_eq!(
            next.sets()[0].document.to_bytes().unwrap(),
            native.to_bytes()
        );
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// テキストレイヤーのある文書（版 30）も分けて書け、中の版 30 のまま同じ項目に読み戻せる。
#[test]
fn a_text_document_splits_with_inner_version_30() {
    use yolu_core::text::{TextFont, TextSettings};
    let mut core = yolu_core::Document::with_tile_size(64, 64, 16).unwrap();
    let id = core.add_layer("文字").unwrap();
    for x in 0..40 {
        core.set_pixel(id, x, 20, yolu_core::Rgba8::new(10, 20, 30, 255))
            .unwrap();
    }
    core.clear_history().unwrap();
    let text = TextSettings::new(
        "分ける",
        TextFont::Bundled("biz-udpgothic".into()),
        2.0,
        40.0,
    );
    core.set_text_for_load(id, text.clone()).unwrap();
    let doc = NativeDocument::from_core(&core).unwrap();
    assert_eq!(doc.version(), crate::TEXT_VERSION);
    let t = tiny(largest_tile(&doc), 0);
    let entries = t.scoped(|| native_entries(&doc, "sets/x/"));
    assert!(entries.len() >= 2, "分けていない");
    let header = entries[0].1.bytes().unwrap();
    assert_eq!(
        i32::from_le_bytes(header[12..16].try_into().unwrap()),
        crate::TEXT_VERSION
    );
    let back = stored(&entries).to_native().unwrap();
    assert_eq!(back.to_bytes(), doc.to_bytes());
    assert_eq!(
        back.to_core().unwrap().layer(id).unwrap().text(),
        Some(&text)
    );
}
