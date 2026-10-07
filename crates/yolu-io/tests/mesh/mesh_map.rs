use yolu_core::mesh_maps::MeshMapKind;
use yolu_io::mesh_map;
fn fixture(kind: MeshMapKind, version: i32) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/golden/mesh/{}.v{version}.bin",
        env!("CARGO_MANIFEST_DIR"),
        kind.name()
    ))
    .unwrap()
}
#[test]
fn csharp_all_kinds_all_versions() {
    for kind in MeshMapKind::ALL {
        for version in 1..=3 {
            let bytes = fixture(kind, version);
            let map = mesh_map::read(&bytes).unwrap();
            assert_eq!(map.kind(), kind);
            assert_eq!((map.width(), map.height()), (19, 13));
            assert_eq!(
                map.provenance().antialiasing,
                if version == 1 { 1 } else { 3 }
            );
            assert_eq!(
                map.provenance().target_slots,
                if version == 3 { vec![2, 7] } else { vec![2] }
            );
            let mut state = 0x12345678u32;
            for &v in map.data() {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                assert_eq!(v, state as u16);
            }
            for (i, &v) in map.coverage().iter().enumerate() {
                assert_eq!(v, (i % 4) as u8);
            }
            let encoded = mesh_map::write(&map).unwrap();
            assert_eq!(mesh_map::read(&encoded).unwrap(), map);
            if version == 3 {
                assert_eq!(encoded, bytes, "{kind:?}: 圧縮を含む全バイト");
            }
        }
    }
}
#[test]
fn refuses_truncation_trailing_unknown_and_budget() {
    let bytes = fixture(MeshMapKind::Position, 3);
    for n in 0..bytes.len() {
        assert!(mesh_map::read(&bytes[..n]).is_err(), "{n}");
    }
    let mut bad = bytes.clone();
    bad.push(0);
    assert!(mesh_map::read(&bad).is_err());
    for (offset, value) in [
        (8, 4i32),
        (8, 0),
        (12, 10),
        (16, 0),
        (64, 0),
        (72, -2),
        (80, 5),
        (84, 1),
        (88, -1),
    ] {
        let mut bad = bytes.clone();
        bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert!(mesh_map::read(&bad).is_err(), "{offset}: {value}");
    }
    assert!(mesh_map::read_with_limit(&bytes, 3457).is_err());
    assert!(mesh_map::read_with_limit(&bytes, 3458).is_ok());
}
#[test]
fn entry_names_are_exact() {
    for kind in MeshMapKind::ALL {
        assert_eq!(
            mesh_map::parse_entry_name(&mesh_map::entry_name(kind)),
            Some(kind)
        );
    }
    for s in [
        "meshmap-0.bin",
        "meshmap-position.bin",
        "dir/meshmap-Position.bin",
        "meshmap-Position.bin.bak",
    ] {
        assert_eq!(mesh_map::parse_entry_name(s), None);
    }
}

#[test]
fn project_keeps_document_and_roundtrips_derived_map() {
    let source = std::fs::read(format!(
        "{}/tests/fixtures/m1-mode-00.ylp",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let project = yolu_io::Project::read(&source).unwrap();
    let id = &project.sets()[0].id;
    let map = mesh_map::read(&fixture(MeshMapKind::Position, 3)).unwrap();
    let edited = project.with_mesh_map(id, &map).unwrap();
    assert_eq!(
        edited.mesh_map(id, MeshMapKind::Position, 10000).unwrap(),
        Some(map.clone())
    );
    assert!(project
        .mesh_map(id, MeshMapKind::Position, 10000)
        .unwrap()
        .is_none());
    let reopened = yolu_io::Project::read(&edited.to_bytes().unwrap()).unwrap();
    assert_eq!(
        reopened.mesh_map(id, MeshMapKind::Position, 10000).unwrap(),
        Some(map)
    );
    for (name, bytes) in project.original_archive().entries() {
        assert_eq!(reopened.original_archive().entries().get(name), Some(bytes));
    }
    assert!(project
        .with_mesh_map(
            "missing",
            &mesh_map::read(&fixture(MeshMapKind::Position, 3)).unwrap()
        )
        .is_err());
}

#[test]
fn refuses_compressed_overflow_unknown_coverage_and_invalid_utf8() {
    use std::io::{Read, Write};
    let original = fixture(MeshMapKind::Position, 3);
    let mut bad = original.clone();
    bad[24] = 0xff;
    assert!(mesh_map::read(&bad).is_err());
    let read_int =
        |at: usize| i32::from_le_bytes(original[at..at + 4].try_into().unwrap()) as usize;
    let mut at = 20;
    for _ in 0..2 {
        at += 4 + read_int(at);
    }
    at += 28;
    at += 4 + 4 * read_int(at);
    for _ in 0..4 {
        at += 4 + read_int(at);
    }
    at += 48;
    let mut raw = vec![];
    flate2::read::DeflateDecoder::new(&original[at + 4..])
        .read_to_end(&mut raw)
        .unwrap();
    for case in 0..3 {
        let mut invalid = raw.clone();
        match case {
            0 => invalid.push(0),
            1 => {
                invalid.pop();
            }
            _ => invalid[0] = 4,
        }
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&invalid).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut bytes = original[..at].to_vec();
        bytes.extend((compressed.len() as i32).to_le_bytes());
        bytes.extend(compressed);
        assert!(mesh_map::read(&bytes).is_err(), "case {case}");
    }
}

fn writer() -> yolu_io::WriterInfo {
    yolu_io::WriterInfo {
        app: "試験の書き手".into(),
        version: "0.0.1".into(),
        unity: "standalone".into(),
    }
}
fn project_fixture(name: &str) -> yolu_io::Project {
    yolu_io::Project::read(
        &std::fs::read(format!(
            "{}/tests/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap()
}
const ALL_BUDGET: usize = usize::MAX;

/// Unity が実際に書いた形式 3・5・6 の .ylp（メッシュマップの .bin は版 2・スロット 1 つ）。
#[test]
fn real_unity_files_of_formats_3_5_6_keep_their_maps_and_need_upgrade_to_write() {
    for (file, format, kinds) in [
        (
            "format3.ylp",
            3,
            &[MeshMapKind::Position, MeshMapKind::WorldNormal][..],
        ),
        (
            "format5-shared-materials.ylp",
            5,
            &[MeshMapKind::Position, MeshMapKind::WorldNormal][..],
        ),
        ("format6.ylp", 6, &[MeshMapKind::Position][..]),
    ] {
        let project = project_fixture(file);
        assert_eq!(project.info().format, format, "{file}");
        let upgraded = project.upgraded(writer()).unwrap();
        assert_eq!(upgraded.info().format, 7);
        for (slot, set) in project.sets().iter().enumerate() {
            for kind in MeshMapKind::ALL {
                let found = project.mesh_map(&set.id, kind, ALL_BUDGET).unwrap();
                if !kinds.contains(&kind) {
                    assert!(found.is_none(), "{file} {kind:?}");
                    continue;
                }
                let map = found.unwrap_or_else(|| panic!("{file} {kind:?} がありません"));
                let p = map.provenance();
                assert_eq!(map.kind(), kind);
                assert_eq!((p.width, p.height), (512, 512));
                assert_eq!(p.target_slot, slot as i32, "{file}: 版 2 は 1 つのスロット");
                assert_eq!(p.target_slots, vec![slot as i32]);
                assert_eq!((p.padding, p.antialiasing, p.engine_version), (16, 1, 2));
                assert_eq!(
                    (p.space.as_str(), p.pose.as_str(), p.source.as_str()),
                    ("SnapshotWorld", "StaticSnapshot", "Self")
                );
                assert!(map.coverage().contains(&1), "{file}: 焼いた面がある");
                // 読み直しても同じ。版 3 で書き直しても中身と由来は変わらない
                let rewritten = mesh_map::write(&map).unwrap();
                assert_eq!(&rewritten[8..12], &3i32.to_le_bytes());
                assert_eq!(mesh_map::read(&rewritten).unwrap(), map);
                // 移行しても同じマップが読める
                assert_eq!(
                    upgraded.mesh_map(&set.id, kind, ALL_BUDGET).unwrap(),
                    Some(map.clone()),
                    "{file}: 形式 7 へ移行しても同じ"
                );
                // 旧形式のままの書き込みは断り、元のエントリを変えない
                let error = project.with_mesh_map(&set.id, &map).err().unwrap();
                assert!(error.to_string().contains("形式7"), "{error}");
                // 移行後なら置き換えられ、ほかのセット・種類は変わらない
                let edited = upgraded.with_mesh_map(&set.id, &map).unwrap();
                assert_eq!(edited.info().format, 7);
                for other in project.sets() {
                    for k in kinds {
                        assert_eq!(
                            edited.mesh_map(&other.id, *k, ALL_BUDGET).unwrap(),
                            project.mesh_map(&other.id, *k, ALL_BUDGET).unwrap()
                        );
                    }
                }
            }
        }
    }
}

/// 形式 1・2 は根に meshmap-*.bin がある（Unity が書いた版 2 のバイト列を根へ置いた合成）。
#[test]
fn formats_1_and_2_read_root_entries_and_write_after_upgrade() {
    let donor = project_fixture("format3.ylp");
    let donor_set = &donor.sets()[0].id;
    let original = std::fs::read(format!(
        "{}/tests/fixtures/format3.ylp",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let donor_bytes = yolu_io::Archive::read(&original)
        .unwrap()
        .entries()
        .get(&format!("sets/{donor_set}/meshmap-Position.bin"))
        .unwrap()
        .to_vec();
    let expected = mesh_map::read(&donor_bytes).unwrap();
    for file in ["format1.ylp", "format2.ylp"] {
        let plain = project_fixture(file);
        assert!(plain.info().format < 3);
        let id = plain.sets()[0].id.clone();
        assert!(plain
            .mesh_map(&id, MeshMapKind::Position, ALL_BUDGET)
            .unwrap()
            .is_none());
        let mut entries: std::collections::BTreeMap<String, Vec<u8>> = plain
            .original_archive()
            .entries()
            .iter()
            .map(|(k, v)| (k.clone(), v.bytes().unwrap().to_vec()))
            .collect();
        entries.insert("meshmap-Position.bin".into(), donor_bytes.clone());
        let with_root = yolu_io::Project::read(
            &yolu_io::Archive::from_entries(entries)
                .unwrap()
                .to_bytes()
                .unwrap(),
        )
        .unwrap();
        assert!(with_root.info().format < 3, "{file}");
        let read = with_root
            .mesh_map(&id, MeshMapKind::Position, ALL_BUDGET)
            .unwrap()
            .unwrap_or_else(|| panic!("{file}: 根のエントリを読めません"));
        assert_eq!(read, expected, "{file}");
        assert!(with_root
            .mesh_map(&id, MeshMapKind::WorldNormal, ALL_BUDGET)
            .unwrap()
            .is_none());
        assert!(with_root
            .with_mesh_map(&id, &read)
            .err()
            .unwrap()
            .to_string()
            .contains("形式7"));
        let upgraded = with_root.upgraded(writer()).unwrap();
        assert_eq!(
            upgraded
                .mesh_map(&id, MeshMapKind::Position, ALL_BUDGET)
                .unwrap(),
            Some(expected.clone()),
            "{file}: 移行すると根のエントリはセットの下へ動く"
        );
        let edited = upgraded.with_mesh_map(&id, &expected).unwrap();
        assert_eq!(
            edited
                .mesh_map(&id, MeshMapKind::Position, ALL_BUDGET)
                .unwrap(),
            Some(expected.clone())
        );
    }
}
