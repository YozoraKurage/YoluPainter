//! 手動の ID の色（正本の版 19 の末尾の塊 `YLID`）の書き込みと読み込み。レイヤーの後に、`tag`（`Bytes` の値）・`count`・`binding`・色の並びを書き、
//! 空なら塊を書かず、色のために版を上げない。読むときは文書へ戻す（指紋が今のモデルと合うかは使う側が見る）。正解の 1 つは C# の書き手が作った
//! `native-rich-v21.utpaint`（`m2_bridge.rs` が、読んで書き戻して同じバイト列になることを確かめる）。
use std::collections::BTreeMap;
use std::sync::Arc;

use yolu_core::mesh_maps::IdColorAssignments;
use yolu_core::{Channel, ChannelInfo, ChannelKind, ColorSpace, Document, Rgba8, TileCoord};
use yolu_io::{
    BackupKeep, CommitOptions, DocumentSource, GenerationStore, MaterialRef, NativeDocument,
    Package, Project, Removal, SaveTarget, SetSpec, Thresholds, WriterInfo, UNITY_NATIVE_VERSION,
};

fn binding() -> String {
    "0123456789abcdef".repeat(4)
}

fn assigned(pairs: &[(usize, u32)]) -> IdColorAssignments {
    IdColorAssignments::new(binding(), pairs.iter().copied().collect()).unwrap()
}

/// 塊の中身（`tag` の後。`count`・`binding`・色の並び）。
fn block_after_tag(binding: &str, pairs: &[(i32, i32)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend((pairs.len() as i32).to_le_bytes());
    out.extend((binding.len() as i32).to_le_bytes());
    out.extend(binding.as_bytes());
    for (part, rgb) in pairs {
        out.extend(part.to_le_bytes());
        out.extend(rgb.to_le_bytes());
    }
    out
}

fn writer() -> WriterInfo {
    WriterInfo {
        app: "試験の書き手".into(),
        version: "0.0.1".into(),
        unity: "standalone".into(),
    }
}

fn same_colors(a: &Document, b: &IdColorAssignments) {
    assert_eq!(a.id_colors().binding(), b.binding());
    assert_eq!(a.id_colors().colors(), b.colors());
}

#[test]
fn colors_are_written_after_the_layers_as_a_ylid_block_and_read_back_the_same() {
    let mut doc = Document::new(16, 16).unwrap();
    doc.add_layer("下").unwrap();
    doc.add_layer("上").unwrap();
    let plain = NativeDocument::from_core(&doc).unwrap().to_bytes();
    // 番号は歯抜け・端（0 と 3999999）・色も端（0 と 0xffffff）
    let pairs = [(0usize, 0x000000u32), (2, 0xff8040), (3_999_999, 0xffffff)];
    let colors = assigned(&pairs);
    doc.set_id_colors(colors.clone(), false).unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    let bytes = native.to_bytes();
    // レイヤーの後ろにだけ足す。前の並びは変わらず、版も変わらない（版 19 以上のどの版でも塊を置ける）
    assert!(bytes.starts_with(&plain));
    let mut tail = b"YLID".to_vec();
    tail.extend(block_after_tag(
        &binding(),
        &[(0, 0), (2, 0xff8040), (3_999_999, 0xffffff)],
    ));
    assert_eq!(&bytes[plain.len()..], &tail[..]);
    assert_eq!(native.version(), UNITY_NATIVE_VERSION);
    // 読み直すと、項目も core の文書も同じ
    let reread = NativeDocument::read(&bytes).unwrap();
    assert!(
        reread.core_issues().is_empty(),
        "{:?}",
        reread.core_issues()
    );
    assert_eq!(reread.fields(), native.fields());
    let mut back = reread.to_core().unwrap();
    same_colors(&back, &colors);
    assert!(
        !back.can_undo() && !back.can_redo(),
        "読み込みは履歴に残さない"
    );
    assert_eq!(NativeDocument::from_core(&back).unwrap().to_bytes(), bytes);
    // 戻した文書から先の編集は取り消せる（戻した色がその前の状態）
    back.set_id_colors(IdColorAssignments::default(), false)
        .unwrap();
    assert!(back.id_colors().colors().is_empty());
    back.undo().unwrap();
    same_colors(&back, &colors);
}

#[test]
fn an_empty_assignment_writes_no_block_and_only_the_other_features_decide_the_version() {
    for user_channel in [false, true] {
        let mut doc = Document::new(16, 16).unwrap();
        doc.add_layer("レイヤー").unwrap();
        if user_channel {
            doc.add_channel(ChannelInfo {
                name: "Extra".into(),
                kind: ChannelKind::Scalar,
                color_space: ColorSpace::Linear,
                default: Rgba8::new(5, 5, 5, 255),
            })
            .unwrap();
        }
        let plain = NativeDocument::from_core(&doc).unwrap();
        let version = plain.version();
        assert_eq!(version, if user_channel { 22 } else { 21 });
        assert!(plain.field("manual_id_colors.tag").is_none());
        let plain_bytes = plain.to_bytes();
        doc.set_id_colors(assigned(&[(1, 0x102030)]), false)
            .unwrap();
        let with = NativeDocument::from_core(&doc).unwrap();
        assert_eq!(with.version(), version, "色は版を決めない");
        assert!(with.version() >= 19);
        assert!(with.to_bytes().len() > plain_bytes.len());
        // 全部外すと、色を持たなかったときと同じバイト列に戻る（塊も版の変更も残らない）
        doc.set_id_colors(IdColorAssignments::default(), false)
            .unwrap();
        assert_eq!(
            NativeDocument::from_core(&doc).unwrap().to_bytes(),
            plain_bytes
        );
    }
}

#[test]
fn a_project_with_colors_keeps_the_outer_format_and_the_version_the_unity_reader_reads() {
    let mut doc = Document::new(32, 32).unwrap();
    doc.add_layer("レイヤー").unwrap();
    doc.set_id_colors(assigned(&[(0, 0x112233), (5, 0x445566)]), false)
        .unwrap();
    let project = project_of(&[(A, &doc)]);
    // 中身の形式は上げない（Unity 版が開けなくなるのを、色を使うセットにも広げない）
    assert_eq!(project.info().format, 7);
    let set = &project.sets()[0].document;
    // 塊は版 19 から。Unity 版の読み手が読める版（21 まで）の範囲に収まる
    assert!((19..=UNITY_NATIVE_VERSION).contains(&set.version()));
    assert!(set.core_issues().is_empty());
    let bytes = project.to_bytes().unwrap();
    let package = Package::read_bytes(&bytes, &yolu_io::Limits::default()).unwrap();
    assert_eq!(package.manifest_version(), 3);
    same_colors(&set.to_core().unwrap(), doc.id_colors());
}

#[test]
fn the_largest_assignment_round_trips_and_a_damaged_block_is_refused() {
    // 上限（4096）。それを超える色・範囲外の値・不正な指紋は core が作らない（書き手に届かない）
    let many: BTreeMap<usize, u32> = (0..4096)
        .map(|i| (i * 7, (i as u32 * 4093) & 0xff_ffff))
        .collect();
    let colors = IdColorAssignments::new(binding(), many.clone()).unwrap();
    let mut doc = Document::new(16, 16).unwrap();
    doc.set_id_colors(colors.clone(), false).unwrap();
    let bytes = NativeDocument::from_core(&doc).unwrap().to_bytes();
    let back = NativeDocument::read(&bytes).unwrap().to_core().unwrap();
    same_colors(&back, &colors);
    let mut over = many;
    over.insert(4096 * 7, 1);
    assert!(IdColorAssignments::new(binding(), over).is_err());
    assert!(IdColorAssignments::new(binding(), BTreeMap::from([(4_000_000, 1)])).is_err());
    assert!(IdColorAssignments::new(binding(), BTreeMap::from([(1, 0x100_0000)])).is_err());
    assert!(IdColorAssignments::new("A".repeat(64), BTreeMap::from([(1, 1)])).is_err());
    assert!(IdColorAssignments::new("0".repeat(63), BTreeMap::from([(1, 1)])).is_err());

    // 壊れた塊は読まずに断る（一部だけ読んで黙って捨てない）
    let mut small = Document::new(16, 16).unwrap();
    small
        .set_id_colors(assigned(&[(1, 0x112233), (5, 0x445566)]), false)
        .unwrap();
    let good = NativeDocument::from_core(&small).unwrap().to_bytes();
    assert!(NativeDocument::read(&good).is_ok());
    let block = 4 + 4 + 4 + 64 + 16;
    let at = good.len() - block;
    let patched = |offset: usize, value: &[u8]| {
        let mut bad = good.clone();
        bad[at + offset..at + offset + value.len()].copy_from_slice(value);
        bad
    };
    // 塊の中の位置（先頭は `tag`）
    const COUNT: usize = 4;
    const BINDING: usize = 12;
    const PART_0: usize = BINDING + 64;
    const RGB_0: usize = PART_0 + 4;
    const PART_1: usize = RGB_0 + 4;
    const RGB_1: usize = PART_1 + 4;
    for (what, bad) in [
        ("tag", patched(0, b"YLIX")),
        ("count 0", patched(COUNT, &0i32.to_le_bytes())),
        ("count 4097", patched(COUNT, &4097i32.to_le_bytes())),
        ("count が足りない", patched(COUNT, &3i32.to_le_bytes())),
        ("指紋が大文字", patched(BINDING, b"A")),
        ("指紋が 16 進でない", patched(BINDING, b"g")),
        ("番号が同じ", patched(PART_1, &1i32.to_le_bytes())),
        ("番号が逆", patched(PART_1, &0i32.to_le_bytes())),
        ("番号が範囲外", patched(PART_0, &4_000_000i32.to_le_bytes())),
        ("番号が負", patched(PART_0, &(-1i32).to_le_bytes())),
        ("色が範囲外", patched(RGB_0, &0x100_0000i32.to_le_bytes())),
        ("色が負", patched(RGB_1, &(-1i32).to_le_bytes())),
        ("末尾に余り", [&good[..], &[0]].concat()),
    ] {
        assert!(NativeDocument::read(&bad).is_err(), "{what}");
    }
    // 塊の途中で切れたものも断る（塊の手前までなら、塊の無い文書として読める）
    for cut in 1..block {
        assert!(
            NativeDocument::read(&good[..good.len() - cut]).is_err(),
            "{cut}"
        );
    }
    assert!(NativeDocument::read(&good[..at]).is_ok());
}

// ───────── プロジェクト（保存・開き直し・分けた正本・復旧・配布用） ─────────

const A: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";
const B: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

fn spec(id: &str, slot: u16, source: DocumentSource) -> SetSpec {
    SetSpec {
        id: id.into(),
        name: format!("セット {slot}"),
        material: MaterialRef::PendingSlot(slot),
        document: Some(source),
        composites: Vec::new(),
    }
}

fn project_of(docs: &[(&str, &Document)]) -> Project {
    let specs: Vec<SetSpec> = docs
        .iter()
        .enumerate()
        .map(|(i, (id, d))| {
            let snapshot = Arc::new(d.capture_snapshot().unwrap());
            spec(id, i as u16, DocumentSource::from_core(snapshot).unwrap())
        })
        .collect();
    Project::create(writer(), &specs, docs[0].0).unwrap()
}

fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

const TILE: u32 = 32;

/// レイヤーごとに違う画素のある文書（小さな閾値で分けられる大きさ）。手動の ID の色つき。
fn painted_with_colors(layers: usize) -> Document {
    let mut doc = Document::with_tile_size(128, 128, TILE).unwrap();
    doc.set_source_budget_bytes(1 << 30).unwrap();
    for i in 0..layers {
        let id = doc.add_layer(&format!("レイヤー {i}")).unwrap();
        for ty in 0..4u32 {
            for tx in 0..4u32 {
                if !(tx + ty + i as u32).is_multiple_of(3) {
                    let bytes = noise(
                        (TILE * TILE * 4) as usize,
                        i as u64 * 31 + (ty * 4 + tx) as u64,
                    );
                    doc.import_tile(id, Channel::Color, TileCoord::new(tx, ty), &bytes)
                        .unwrap();
                }
            }
        }
    }
    doc.set_id_colors(
        assigned(&[(0, 0xff0000), (3, 0x00ff00), (4, 0x0000ff)]),
        false,
    )
    .unwrap();
    doc.clear_history().unwrap();
    doc
}

/// 合計 64 KiB を超えたら `YLP-4`・正本は分け、部分は 16 KiB まで（`projects/bigdoc.rs` と同じ小ささ）。
fn small() -> Thresholds {
    Thresholds {
        classic_total_bytes: 64 << 10,
        split_above: 64 << 10,
        part_bytes: 16 << 10,
        part_min: 8 << 10,
        keep_in_memory: 0,
        ..Thresholds::REAL
    }
}

struct Dir(std::path::PathBuf);
impl Dir {
    fn new() -> Dir {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "yolu-io-idcolors-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }
    fn path(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn entry(project: &Project, name: &str) -> Vec<u8> {
    project.original_archive().entries()[name]
        .bytes()
        .unwrap()
        .to_vec()
}

#[test]
fn a_split_document_keeps_the_colors_in_the_header_and_the_parts_through_every_path() {
    let dir = Dir::new();
    let doc = painted_with_colors(7);
    small().scoped(|| {
        // core の文書から流して作る道と、メモリの正本を分ける道が、同じバイト列のエントリを作る
        let from_core = project_of(&[(A, &doc)]);
        let native = NativeDocument::from_core(&doc).unwrap();
        let from_native = Project::create(
            writer(),
            &[spec(A, 0, DocumentSource::Native(native.clone()))],
            A,
        )
        .unwrap();
        assert!(from_core.sets()[0].document.is_split());
        let names: Vec<String> = from_core
            .original_archive()
            .entries()
            .keys()
            .filter(|n| n.starts_with(&format!("sets/{A}/document.utpaint")))
            .cloned()
            .collect();
        assert!(names.len() >= 4, "{names:?}");
        for name in &names {
            assert_eq!(entry(&from_core, name), entry(&from_native, name), "{name}");
        }
        // 平の値（count・binding・色）はヘッダーの末尾、`tag` は `Bytes` の値なので最後の部分の末尾
        let header = entry(&from_core, &format!("sets/{A}/document.utpaint"));
        let tail = block_after_tag(&binding(), &[(0, 0xff0000), (3, 0x00ff00), (4, 0x0000ff)]);
        assert!(header.ends_with(&tail));
        let last = names
            .iter()
            .max_by_key(|n| n.rsplit('.').next().unwrap().parse::<u32>().unwrap_or(0))
            .unwrap();
        assert!(entry(&from_core, last).ends_with(b"YLID"));
        // 流して読む道（置いてある正本）も、メモリの正本を読む道も、同じ色
        assert!(from_core.sets()[0].document.core_issues().is_empty());
        let back = from_core.sets()[0].document.to_core().unwrap();
        same_colors(&back, doc.id_colors());
        assert!(!back.can_undo());
        assert_eq!(
            NativeDocument::from_core(&back).unwrap().to_bytes(),
            native.to_bytes()
        );
        // ファイルに保存して開き直す（部分はファイルの位置から読む）
        let path = dir.path("split.ylp");
        let mut target = SaveTarget::create(&path).unwrap();
        target.save_with(&from_core, BackupKeep::All).unwrap();
        let (opened, _) = SaveTarget::open(&path).unwrap();
        let reopened = &opened.sets()[0].document;
        assert!(reopened.is_split());
        same_colors(&reopened.to_core().unwrap(), doc.id_colors());
    });
}

#[test]
fn colors_survive_save_open_and_an_unchanged_save_again() {
    let dir = Dir::new();
    let colored = painted_with_colors(2);
    let mut plain = Document::new(32, 32).unwrap();
    plain.add_layer("レイヤー").unwrap();
    let project = project_of(&[(A, &colored), (B, &plain)]);
    let path = dir.path("colors.ylp");
    let mut target = SaveTarget::create(&path).unwrap();
    target.save_with(&project, BackupKeep::All).unwrap();
    let (opened, mut target) = SaveTarget::open(&path).unwrap();
    let docs = |p: &Project| {
        p.sets()
            .iter()
            .map(|s| s.document.to_core().unwrap())
            .collect::<Vec<_>>()
    };
    let first = docs(&opened);
    same_colors(&first[0], colored.id_colors());
    assert!(
        first[1].id_colors().colors().is_empty(),
        "色の無いセットに塊は無い"
    );
    assert!(!first[0].can_undo());
    // 変えずに保存し直す（セットの正本は開いたファイルから写す）: 正本のバイト列も色も変わらない
    let keep: Vec<SetSpec> = opened
        .sets()
        .iter()
        .enumerate()
        .map(|(i, s)| SetSpec {
            document: None,
            ..spec(
                &s.id,
                i as u16,
                DocumentSource::Native(NativeDocument::from_core(&plain).unwrap()),
            )
        })
        .collect();
    let next = opened.with_sets(writer(), &keep, A).unwrap();
    for set in [A, B] {
        let name = format!("sets/{set}/document.utpaint");
        assert_eq!(entry(&next, &name), entry(&opened, &name), "{set}");
    }
    target.save_with(&next, BackupKeep::All).unwrap();
    let (again, _) = SaveTarget::open(&path).unwrap();
    same_colors(&docs(&again)[0], colored.id_colors());
    // 色を全部外して保存し直すと、そのセットの正本から塊が消える
    let mut cleared = first[0].capture_snapshot().unwrap();
    cleared
        .set_id_colors(IdColorAssignments::default(), false)
        .unwrap();
    let specs = vec![
        spec(A, 0, DocumentSource::from_core(Arc::new(cleared)).unwrap()),
        SetSpec {
            document: None,
            ..spec(
                B,
                1,
                DocumentSource::Native(NativeDocument::from_core(&plain).unwrap()),
            )
        },
    ];
    let removed = opened.with_sets(writer(), &specs, A).unwrap();
    assert!(removed.sets()[0]
        .document
        .to_core()
        .unwrap()
        .id_colors()
        .colors()
        .is_empty());
}

#[test]
fn the_recovery_generation_and_the_distribution_copy_keep_the_colors() {
    let dir = Dir::new();
    for big in [false, true] {
        let doc = painted_with_colors(if big { 7 } else { 1 });
        let thresholds = if big { small() } else { Thresholds::REAL };
        thresholds.scoped(|| {
            let project = project_of(&[(A, &doc)]);
            assert_eq!(project.sets()[0].document.is_split(), big);
            // 復旧の世代へ書いて読み戻す
            let store = GenerationStore::new(dir.path(&format!("store-{big}")));
            store
                .commit(
                    project.original_archive().entries(),
                    &CommitOptions {
                        expected: None,
                        keep: Some(3),
                        share: true,
                    },
                )
                .unwrap();
            let loaded = store.load().unwrap();
            let restored = Project::from_entries(loaded.files.clone()).unwrap();
            let back = restored.sets()[0].document.to_core().unwrap();
            same_colors(&back, doc.id_colors());
            assert!(!back.can_undo());
            // 配布用の写し（モデルの参照・派生物などを除いても、色は正本の一部として残る）
            let copy = project.for_distribution(writer(), &Removal::ALL).unwrap();
            same_colors(&copy.sets()[0].document.to_core().unwrap(), doc.id_colors());
            let path = dir.path(&format!("copy-{big}.ylp"));
            SaveTarget::create(&path).unwrap().save(&copy).unwrap();
            let (opened, _) = SaveTarget::open(&path).unwrap();
            same_colors(
                &opened.sets()[0].document.to_core().unwrap(),
                doc.id_colors(),
            );
        });
    }
}
