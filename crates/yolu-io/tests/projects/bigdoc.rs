//! 大きなプロジェクトの .ylp と復旧: 流して読み書きする外側（`YLP-4`・zip64）、分けた正本（版 26）、設定の予算からの上限。巨大な
//! 文書は作らず、閾値（`Thresholds`）を小さくして、小さな文書で大きな形の道を全部通す。
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use yolu_core::{Channel, Document, Rgba8, TileCoord};
use yolu_io::{
    Archive, Blob, CommitOptions, DocumentSource, GenerationStore, Limits, MaterialRef,
    NativeDocument, Project, Removal, SaveTarget, SetSpec, Thresholds, WriterInfo,
};

struct Dir(PathBuf);
impl Dir {
    fn new() -> Dir {
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "yolu-io-bigdoc-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&d).unwrap();
        Dir(d)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn writer() -> WriterInfo {
    WriterInfo {
        app: "YoluPainter-rs".into(),
        version: "0.0.0".into(),
        unity: "standalone".into(),
    }
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
const SIZE: u32 = 128;
const TILE: u32 = 32;
/// 層ごとに違う画素の文書（ラスター・マスク・塗りつぶし）。
fn painted(seed: u64, layers: usize) -> Document {
    let mut doc = Document::with_tile_size(SIZE, SIZE, TILE).unwrap();
    doc.set_source_budget_bytes(1 << 30).unwrap();
    let n = SIZE / TILE;
    for i in 0..layers {
        let id = doc.add_layer(&format!("層 {i}")).unwrap();
        for ty in 0..n {
            for tx in 0..n {
                if !(tx + ty + i as u32).is_multiple_of(3) {
                    let bytes = noise(
                        (TILE * TILE * 4) as usize,
                        seed * 1000 + i as u64 * 31 + (ty * n + tx) as u64,
                    );
                    doc.import_tile(id, Channel::Color, TileCoord::new(tx, ty), &bytes)
                        .unwrap();
                }
            }
        }
        if i % 3 == 1 {
            doc.add_layer_mask(id).unwrap();
            let mut m = noise((TILE * TILE * 4) as usize, seed + 77 + i as u64);
            for p in m.as_chunks_mut::<4>().0 {
                p[..3].fill(0);
            }
            doc.import_mask_tile(id, TileCoord::new(1, 1), &m).unwrap();
        }
    }
    doc.add_fill_layer(
        "塗り",
        &[(Channel::Color, Rgba8::new(10, 20, 30, 255))],
        None,
    )
    .unwrap();
    doc.clear_history().unwrap();
    doc
}
fn bytes_of(doc: &Document) -> Vec<u8> {
    NativeDocument::from_core(doc).unwrap().to_bytes()
}
const SET_A: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";
const SET_B: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
fn spec(id: &str, name: &str, slot: u16, doc: &Document) -> SetSpec {
    SetSpec {
        id: id.into(),
        name: name.into(),
        material: MaterialRef::PendingSlot(slot),
        document: Some(DocumentSource::Core(Arc::new(
            doc.capture_snapshot().unwrap(),
        ))),
        composites: Vec::new(),
    }
}
/// 今の形に収まらない扱い（合計 64 KiB 超えで `YLP-4`）、正本は 64 KiB を超えたら分け、部分は 16 KiB まで、小さな層は 8 KiB までまとめる。
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
fn project_of(docs: &[(&str, &Document)]) -> Project {
    let specs: Vec<SetSpec> = docs
        .iter()
        .enumerate()
        .map(|(i, (id, d))| spec(id, &format!("セット {i}"), i as u16, d))
        .collect();
    Project::create(writer(), &specs, docs[0].0).unwrap()
}
fn names(p: &Project) -> Vec<String> {
    p.original_archive().entries().keys().cloned().collect()
}

#[test]
fn a_big_project_is_split_saved_as_ylp_4_and_opens_back_with_every_pixel() {
    let dir = Dir::new();
    let a = painted(1, 7);
    let b = painted(2, 1);
    small().scoped(|| {
        let p = project_of(&[(SET_A, &a), (SET_B, &b)]);
        // 大きなセットは版 26 のヘッダーと部分、小さなセットは今のまま
        assert!(p.sets()[0].document.is_split());
        assert!(!p.sets()[1].document.is_split());
        let parts: Vec<_> = names(&p)
            .into_iter()
            .filter(|n| n.starts_with(&format!("sets/{SET_A}/document.utpaint.")))
            .collect();
        assert!(parts.len() >= 3, "{parts:?}");
        assert!(!names(&p)
            .iter()
            .any(|n| n.starts_with(&format!("sets/{SET_B}/document.utpaint."))));
        let path = dir.path("big.ylp");
        let mut target = SaveTarget::create(&path).unwrap();
        let report = target.save_with(&p, yolu_io::BackupKeep::All).unwrap();
        // 置き換えた後のファイルを指すプロジェクト（正本はファイルの位置から読む）
        let saved = report.project.expect("保存したプロジェクト");
        assert_eq!(
            bytes_of(&saved.sets()[0].document.to_core().unwrap()),
            bytes_of(&a)
        );
        let file = fs::read(&path).unwrap();
        // 外側は YLP-4、今の読み手は断る
        let pkg = yolu_io::Package::read_bytes(&file, &Limits::default()).unwrap();
        assert_eq!(pkg.manifest_version(), 4);
        let old = Archive::read(&file).unwrap_err().to_string();
        assert!(old.contains("YOLUPAINTER-YLP-4"), "{old}");
        // 開き直す: 画素はメモリに読まず（ファイルの位置）、core にすると同じ
        let (opened, _) = SaveTarget::open(&path).unwrap();
        for name in names(&opened)
            .iter()
            .filter(|n| n.contains("document.utpaint."))
        {
            assert!(
                opened.original_archive().entries()[name]
                    .in_memory()
                    .is_none(),
                "{name}"
            );
        }
        let set_a = opened.sets().iter().find(|s| s.id == SET_A).unwrap();
        let set_b = opened.sets().iter().find(|s| s.id == SET_B).unwrap();
        assert!(set_a.document.is_split());
        assert_eq!(set_a.document.version(), 21);
        assert_eq!(bytes_of(&set_a.document.to_core().unwrap()), bytes_of(&a));
        assert_eq!(bytes_of(&set_b.document.to_core().unwrap()), bytes_of(&b));
        // 予算（1 層ずつ止める）
        let Err(e) = set_a.document.to_core_within(Some(1)) else {
            panic!("予算で止まらない")
        };
        assert!(matches!(e, yolu_io::Error::Budget(_)), "{e:?}");
    });
}

#[test]
fn a_project_within_the_classic_limits_is_written_byte_for_byte_as_before() {
    let dir = Dir::new();
    // 実物のフィクスチャ（形式 1〜6）: 読んで書き戻すと、今の書き手（Archive）と同じバイト列
    for n in [1, 2, 3, 4, 5, 6] {
        let bytes = fs::read(format!(
            "{}/tests/fixtures/format{n}.ylp",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let p = Project::read(&bytes).unwrap();
        let old = Archive::read(&bytes).unwrap().to_bytes().unwrap();
        assert_eq!(p.to_bytes().unwrap(), old, "形式{n}");
        // ファイルへの保存も同じ
        let path = dir.path(&format!("f{n}.ylp"));
        fs::write(&path, &bytes).unwrap();
        let (opened, mut target) = SaveTarget::open(&path).unwrap();
        target.save(&opened).unwrap();
        assert_eq!(fs::read(&path).unwrap(), old, "形式{n} の保存");
    }
    // core の文書から作る正本（流して書く）も、メモリで作った正本と同じバイト列
    let a = painted(3, 4);
    let streamed = project_of(&[(SET_A, &a)]);
    let mut specs = vec![spec(SET_A, "セット 0", 0, &a)];
    specs[0].document = Some(NativeDocument::from_core(&a).unwrap().into());
    let in_memory = Project::create(writer(), &specs, SET_A).unwrap();
    assert_eq!(streamed.to_bytes().unwrap(), in_memory.to_bytes().unwrap());
    assert!(!streamed.sets()[0].document.is_split());
}

#[test]
fn ylp_4_files_are_opened_within_the_layer_pixel_budget() {
    let dir = Dir::new();
    let a = painted(4, 6);
    let path = dir.path("budget.ylp");
    small().scoped(|| {
        let p = project_of(&[(SET_A, &a)]);
        SaveTarget::create(&path).unwrap().save(&p).unwrap();
    });
    let tight = Limits {
        document_bytes: 10 << 10,
        other_bytes: 768 << 20,
    };
    let e = SaveTarget::open_within(&path, &tight).unwrap_err();
    assert!(matches!(e, yolu_io::Error::Budget(_)), "{e:?}");
    assert!(e.to_string().contains("レイヤーのメモリ"), "{e}");
    SaveTarget::open_within(&path, &Limits::from_layer_pixels(256 << 20)).unwrap();
}

#[test]
fn missing_extra_and_out_of_order_parts_are_refused() {
    let a = painted(5, 6);
    let p = small().scoped(|| project_of(&[(SET_A, &a)]));
    let files = p.original_archive().entries().clone();
    let part = |n: usize| format!("sets/{SET_A}/document.utpaint.{n}");
    let count = files
        .keys()
        .filter(|k| k.contains("document.utpaint."))
        .count();
    // 1 つ欠けた（最後・途中）、余計な部分、番号の飛び
    let mut last = files.clone();
    last.remove(&part(count));
    let mut middle = files.clone();
    middle.remove(&part(2));
    let mut extra = files.clone();
    extra.insert(part(count + 1), Blob::from(vec![0u8; 16]));
    let mut gap = files.clone();
    let moved = gap.remove(&part(count)).unwrap();
    gap.insert(part(count + 1), moved);
    // 部分の中身の入れ替え（長さは合っても、値が境目をまたぐ・SHA が合わない）
    let mut swapped = files.clone();
    let (x, y) = (swapped[&part(1)].clone(), swapped[&part(2)].clone());
    swapped.insert(part(1), y);
    swapped.insert(part(2), x);
    for (why, f) in [
        ("最後が無い", last),
        ("途中が無い", middle),
        ("余計", extra),
        ("飛び", gap),
    ] {
        assert!(Project::from_entries(f).is_err(), "{why}");
    }
    // 入れ替えは骨組み（長さだけ）では見つからないこともあるので、中身を読むと断る
    match Project::from_entries(swapped) {
        Err(_) => {}
        Ok(q) => assert!(q.sets()[0].document.to_core().is_err()),
    }
}

#[test]
fn recovery_shares_the_parts_of_unchanged_layers_between_generations() {
    let dir = Dir::new();
    let mut a = painted(6, 8);
    let store = GenerationStore::new(dir.path("store")).with_limits(Limits::default());
    small().scoped(|| {
        let p = project_of(&[(SET_A, &a)]);
        let first = store
            .commit(
                p.original_archive().entries(),
                &CommitOptions {
                    expected: None,
                    keep: Some(3),
                    share: true,
                },
            )
            .unwrap();
        // 1 つの層の 1 タイルだけを変える
        let last = a.layers()[5].id();
        a.import_tile(
            last,
            Channel::Color,
            TileCoord::new(0, 0),
            &noise((TILE * TILE * 4) as usize, 999),
        )
        .unwrap();
        let q = project_of(&[(SET_A, &a)]);
        let parts_total: u64 = q
            .original_archive()
            .entries()
            .iter()
            .filter(|(k, _)| k.contains("document.utpaint."))
            .map(|(_, b)| b.len())
            .sum();
        let second = store
            .commit(
                q.original_archive().entries(),
                &CommitOptions {
                    expected: Some(&first.token),
                    keep: Some(3),
                    share: true,
                },
            )
            .unwrap();
        // 変わった層の部分と小さなエントリ（ヘッダー・project.json など）だけを書いた
        assert!(second.reused_files >= 2, "{second:?}");
        assert!(
            second.written_bytes < parts_total / 2,
            "{second:?} / {parts_total}"
        );
        // 戻すと画素まで同じ（部分は置き場のファイルを指し、メモリに読まない）
        let loaded = store.load().unwrap();
        let back = Project::from_entries(loaded.files.clone()).unwrap();
        assert_eq!(
            bytes_of(&back.sets()[0].document.to_core().unwrap()),
            bytes_of(&a)
        );
        assert!(loaded
            .files
            .iter()
            .filter(|(k, _)| k.contains("document.utpaint."))
            .all(|(_, b)| b.in_memory().is_none()));
        // 量の数え方: 共有の中身は 1 度だけ数える（2 つの世代の合計より小さい）
        let footprint = store.footprint().unwrap();
        let shared: u64 = {
            let mut seen = BTreeMap::new();
            for g in &footprint.generations {
                for (k, n) in &g.contents {
                    seen.insert(k.clone(), *n);
                }
            }
            seen.values().sum()
        };
        let naive: u64 = footprint
            .generations
            .iter()
            .flat_map(|g| g.contents.iter().map(|(_, n)| *n))
            .sum();
        assert!(shared < naive, "{shared} {naive}");
    });
}

#[test]
fn a_distribution_copy_of_a_big_project_keeps_the_split_document() {
    let dir = Dir::new();
    let a = painted(7, 6);
    small().scoped(|| {
        let p = project_of(&[(SET_A, &a)]);
        let copy = p.for_distribution(writer(), &Removal::ALL).unwrap();
        assert!(copy.sets()[0].document.is_split());
        let path = dir.path("copy.ylp");
        SaveTarget::create(&path).unwrap().save(&copy).unwrap();
        let (opened, _) = SaveTarget::open(&path).unwrap();
        assert_eq!(
            bytes_of(&opened.sets()[0].document.to_core().unwrap()),
            bytes_of(&a)
        );
    });
}

#[test]
fn saving_again_copies_unchanged_sets_from_the_file_and_drops_old_parts() {
    let dir = Dir::new();
    let a = painted(8, 7);
    let b = painted(9, 7);
    let path = dir.path("again.ylp");
    small().scoped(|| {
        let p = project_of(&[(SET_A, &a), (SET_B, &b)]);
        let mut target = SaveTarget::create(&path).unwrap();
        let saved = target
            .save_with(&p, yolu_io::BackupKeep::All)
            .unwrap()
            .project
            .unwrap();
        // セット A の層を減らして保存し直す: 部分は減り、古い部分は残らない。セット B はファイルから写す
        let mut fewer = a.capture_snapshot().unwrap();
        let ids: Vec<_> = fewer.layers().iter().map(|l| l.id()).collect();
        for id in &ids[..4] {
            fewer.remove_layer(*id).unwrap();
        }
        let before = names(&saved);
        let specs = vec![
            spec(SET_A, "セット 0", 0, &fewer),
            SetSpec {
                document: None,
                ..spec(SET_B, "セット 1", 1, &b)
            },
        ];
        let next = saved.with_sets(writer(), &specs, SET_A).unwrap();
        let after = names(&next);
        let count = |names: &[String], set: &str| {
            names
                .iter()
                .filter(|n| n.starts_with(&format!("sets/{set}/document.utpaint.")))
                .count()
        };
        assert!(count(&after, SET_A) < count(&before, SET_A));
        assert_eq!(count(&after, SET_B), count(&before, SET_B));
        let report = target.save_with(&next, yolu_io::BackupKeep::All).unwrap();
        let (opened, _) = SaveTarget::open(&path).unwrap();
        let get = |id: &str| {
            opened
                .sets()
                .iter()
                .find(|s| s.id == id)
                .unwrap()
                .document
                .to_core()
                .unwrap()
        };
        assert_eq!(bytes_of(&get(SET_A)), bytes_of(&fewer));
        assert_eq!(bytes_of(&get(SET_B)), bytes_of(&b));
        assert_eq!(names(&report.project.unwrap()), names(&opened));
    });
}

/// 開いた .ylp は、開いたときのファイルを持ち続けて読む（パスで開き直さない）: 外で動かされても・消されても・別のファイルに置き換え
/// られても、変えていないセットの中身は開いたときのまま、別の名前の保存へ写せる。同じ名前への保存は、外の変更として断り、何も残さない。
#[test]
fn an_opened_file_moved_deleted_or_replaced_outside_still_saves_under_another_name() {
    let a = painted(11, 7);
    let b = painted(12, 7);
    let other = painted(13, 6);
    for how in ["moved", "deleted", "replaced"] {
        let dir = Dir::new();
        small().scoped(|| {
            let p = project_of(&[(SET_A, &a), (SET_B, &b)]);
            let path = dir.path("open.ylp");
            SaveTarget::create(&path).unwrap().save(&p).unwrap();
            let (opened, mut target) = SaveTarget::open(&path).unwrap();
            // 開いたエントリはメモリに残さず、ファイルの位置で持つ
            assert!(opened
                .original_archive()
                .entries()
                .values()
                .any(|b| !b.is_empty() && b.in_memory().is_none()));
            let mut kept = dir.path("kept.ylp");
            let untouched = fs::read(&path).unwrap();
            match how {
                "moved" => {
                    kept = dir.path("moved.ylp");
                    fs::rename(&path, &kept).unwrap();
                }
                "deleted" => fs::remove_file(&path).unwrap(),
                _ => {
                    // 同期の道具のように、別のファイルを同じ名前へ置き換える（同じ inode を書き換えるのではない）
                    let elsewhere = project_of(&[(SET_A, &other)]);
                    let temp = dir.path("elsewhere.ylp");
                    SaveTarget::create(&temp).unwrap().save(&elsewhere).unwrap();
                    // 開いているファイルを置き換えられない OS（POSIX の置換の無い Windows・古い Wine）では、外の道具もこの置き換えができない:
                    // 確かめることが無い（Unix は必ず通る）
                    if let Err(e) = fs::rename(&temp, &path) {
                        if cfg!(unix) {
                            panic!("{e}");
                        }
                        return;
                    }
                    kept = path.clone();
                }
            }
            let outside = if how == "deleted" {
                None
            } else {
                Some(fs::read(&kept).unwrap())
            };
            // 別の名前へ: 中身は開いたときのまま（外で何をされても）
            let copy = dir.path(&format!("copy-{how}.ylp"));
            SaveTarget::create(&copy)
                .unwrap()
                .save(&opened)
                .unwrap_or_else(|e| panic!("{how}: {e}"));
            let (again, _) = SaveTarget::open(&copy).unwrap();
            let core = |p: &Project, id: &str| {
                bytes_of(
                    &p.sets()
                        .iter()
                        .find(|s| s.id == id)
                        .unwrap()
                        .document
                        .to_core()
                        .unwrap(),
                )
            };
            assert_eq!(core(&again, SET_A), bytes_of(&a), "{how}");
            assert_eq!(core(&again, SET_B), bytes_of(&b), "{how}");
            // 外の変更は、書き換えも消しもしない
            if let Some(outside) = outside {
                assert_eq!(fs::read(&kept).unwrap(), outside, "{how}");
            }
            // 同じ名前への保存は断る（動かした・消した: 保存先が無い。置き換えた: 中身が違う）。一時ファイルも残さない。消したときは、名前がすぐ
            // 消える OS（Unix・今の Windows の NTFS の POSIX の削除）だけを見る: 開いたままの削除で名前が残る OS（古い Wine など）では、
            // OS が「まだある」と答えるので、保存先が消された判定そのものが働かない
            if how != "deleted" || cfg!(unix) {
                let refused = target.save(&opened).expect_err(&format!(
                    "{how}: 外で変わったあとの同じ名前への保存が通った"
                ));
                assert!(
                    matches!(refused, yolu_io::Error::SaveConflict(_)),
                    "{how}: {refused:?}"
                );
                if how == "replaced" {
                    assert_ne!(fs::read(&path).unwrap(), untouched);
                } else {
                    assert!(!path.exists(), "{how}: 新しく作らない");
                }
            }
            let left: Vec<_> = fs::read_dir(&dir.0)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|n| n.contains("pending"))
                .collect();
            assert!(left.is_empty(), "{how}: {left:?}");
        });
    }
}

#[test]
fn a_document_source_written_into_a_package_from_memory_round_trips() {
    // メモリの正本（`DocumentSource::Native`）も、閾値を超えれば同じ版 26 で分ける
    let a = painted(10, 6);
    let native = NativeDocument::from_core(&a).unwrap();
    small().scoped(|| {
        let mut s = spec(SET_A, "セット 0", 0, &a);
        s.document = Some(native.clone().into());
        let p = Project::create(writer(), &[s], SET_A).unwrap();
        assert!(p.sets()[0].document.is_split());
        let back = Project::read(&p.to_bytes().unwrap()).unwrap();
        assert_eq!(
            back.sets()[0].document.to_bytes().unwrap(),
            native.to_bytes()
        );
    });
}

/// 測る（手で回す: `cargo test -p yolu-io --release --test projects bigdoc::measure -- --ignored --nocapture`）。4096²・60 層の合成の文書で、
/// 保存・開く・書き置きの時間と、最大 RSS の増え方。
#[test]
#[ignore]
fn measure_a_large_document() {
    use std::time::Instant;
    let size = 4096u32;
    let tile = Document::DEFAULT_TILE_SIZE;
    let layers = 60;
    let mut doc = Document::with_tile_size(size, size, tile).unwrap();
    doc.set_source_budget_bytes(64 << 30).unwrap();
    let n = size / tile;
    let started = Instant::now();
    // 層の種類: 塗り（ゆるやかな色の変化と小さな揺らぎ。画素の大半）、線画（透明の上の細い線。疎）、雑音（縮まない最悪の場合）
    for i in 0..layers {
        let id = doc.add_layer(&format!("層 {i}")).unwrap();
        let kind = match i % 12 {
            0 => "雑音",
            1..=3 => "線画",
            _ => "塗り",
        };
        for ty in 0..n {
            for tx in 0..n {
                let bytes = match kind {
                    "塗り" if (tx * 7 + ty * 3 + i) % 3 != 0 => {
                        painted_tile(tile, tx, ty, i as u64)
                    }
                    "線画" if (tx + ty * 5 + i) % 4 == 0 => line_tile(tile, tx, ty, i as u64),
                    "雑音" if (tx + ty) % 2 == 0 => noise(
                        (tile * tile * 4) as usize,
                        (i * 100_000 + ty * n + tx) as u64,
                    ),
                    _ => continue,
                };
                doc.import_tile(id, Channel::Color, TileCoord::new(tx, ty), &bytes)
                    .unwrap();
            }
        }
    }
    doc.clear_history().unwrap();
    let snapshot = Arc::new(doc.capture_snapshot().unwrap());
    println!("文書を作った: {:?}", started.elapsed());
    let pixels: u64 = doc
        .layers()
        .iter()
        .flat_map(|l| {
            l.surface_channels()
                .into_iter()
                .map(move |c| l.surface(c).unwrap().allocated_bytes())
        })
        .sum();
    println!("層の画素 {} MiB", pixels >> 20);
    let dir = Dir::new();
    let path = dir.path("measure.ylp");
    let rss = Rss::new();
    // 保存（新規）
    rss.reset();
    let t = Instant::now();
    let spec = SetSpec {
        id: SET_A.into(),
        name: "セット".into(),
        material: MaterialRef::PendingSlot(0),
        document: Some(DocumentSource::Core(snapshot.clone())),
        composites: Vec::new(),
    };
    let p = Project::create(writer(), &[spec], SET_A).unwrap();
    let mut target = SaveTarget::create(&path).unwrap();
    let report = target.save_with(&p, yolu_io::BackupKeep::Count(0)).unwrap();
    drop(p);
    println!(
        "保存: {:?}、ファイル {} MiB、最大 RSS の増え {} MiB",
        t.elapsed(),
        fs::metadata(&path).unwrap().len() >> 20,
        rss.grew() >> 20
    );
    let saved = report.project.unwrap();
    // 開く（外側の確かめ・骨組み）と core へ
    drop(saved);
    rss.reset();
    let t = Instant::now();
    let (opened, mut target) =
        SaveTarget::open_within(&path, &Limits::from_layer_pixels(64 << 30)).unwrap();
    let opened_in = t.elapsed();
    let core = opened.sets()[0]
        .document
        .to_core_within(Some(64 << 30))
        .unwrap();
    println!(
        "開く: 確かめ {:?}、core へ {:?}、最大 RSS の増え {} MiB（core の文書を含む）",
        opened_in,
        t.elapsed() - opened_in,
        rss.grew() >> 20
    );
    // 1 つの層を変えて保存し直す（変わらないセットの写し・変わった正本の作り直し）
    let mut edited = core;
    let id = edited.layers()[10].id();
    edited
        .import_tile(
            id,
            Channel::Color,
            TileCoord::new(0, 0),
            &noise((tile * tile * 4) as usize, 7),
        )
        .unwrap();
    let snap = Arc::new(edited.capture_snapshot().unwrap());
    rss.reset();
    let t = Instant::now();
    let spec = SetSpec {
        id: SET_A.into(),
        name: "セット".into(),
        material: MaterialRef::PendingSlot(0),
        document: Some(DocumentSource::Core(snap.clone())),
        composites: Vec::new(),
    };
    let next = opened
        .with_sets(writer(), std::slice::from_ref(&spec), SET_A)
        .unwrap();
    target
        .save_with(&next, yolu_io::BackupKeep::Count(0))
        .unwrap();
    println!(
        "上書き保存: {:?}、最大 RSS の増え {} MiB",
        t.elapsed(),
        rss.grew() >> 20
    );
    // 書き置き（1 回目は全部、2 回目は 1 つの層だけ違う）
    let store = GenerationStore::new(dir.path("store"));
    rss.reset();
    let t = Instant::now();
    let first = store
        .commit(
            next.original_archive().entries(),
            &CommitOptions {
                expected: None,
                keep: Some(3),
                share: true,
            },
        )
        .unwrap();
    println!(
        "書き置き 1 回目: {:?}、書いた {} MiB、最大 RSS の増え {} MiB",
        t.elapsed(),
        first.written_bytes >> 20,
        rss.grew() >> 20
    );
    edited
        .import_tile(
            id,
            Channel::Color,
            TileCoord::new(1, 0),
            &noise((tile * tile * 4) as usize, 8),
        )
        .unwrap();
    let snap = Arc::new(edited.capture_snapshot().unwrap());
    let again = opened
        .with_sets(
            writer(),
            &[SetSpec {
                document: Some(DocumentSource::Core(snap)),
                ..spec
            }],
            SET_A,
        )
        .unwrap();
    rss.reset();
    let t = Instant::now();
    let second = store
        .commit(
            again.original_archive().entries(),
            &CommitOptions {
                expected: Some(&first.token),
                keep: Some(3),
                share: true,
            },
        )
        .unwrap();
    println!(
        "書き置き 2 回目: {:?}、書いた {} MiB（共有 {}）、最大 RSS の増え {} MiB",
        t.elapsed(),
        second.written_bytes >> 20,
        second.reused_files,
        rss.grew() >> 20
    );
}

/// 塗りのタイル: 位置でゆるやかに変わる色に、下の 2 ビットの揺らぎ。
fn painted_tile(tile: u32, tx: u32, ty: u32, seed: u64) -> Vec<u8> {
    let jitter = noise((tile * tile) as usize, seed * 7919 + (tx * 131 + ty) as u64);
    let mut out = Vec::with_capacity((tile * tile * 4) as usize);
    for y in 0..tile {
        for x in 0..tile {
            let j = jitter[(y * tile + x) as usize] & 3;
            let gx = ((tx * tile + x) / 16) as u8;
            let gy = ((ty * tile + y) / 16) as u8;
            out.extend_from_slice(&[
                gx.wrapping_add(j),
                gy.wrapping_add(seed as u8),
                128u8.wrapping_add(j),
                255,
            ]);
        }
    }
    out
}
/// 線画のタイル: 透明の上に、ななめの細い線。
fn line_tile(tile: u32, tx: u32, ty: u32, seed: u64) -> Vec<u8> {
    let mut out = vec![0u8; (tile * tile * 4) as usize];
    let phase = (tx * 3 + ty * 5 + seed as u32) % tile;
    for y in 0..tile {
        for w in 0..3 {
            let x = (y + phase + w) % tile;
            let at = ((y * tile + x) * 4) as usize;
            out[at..at + 4].copy_from_slice(&[20, 18, 30, if w == 1 { 255 } else { 128 }]);
        }
    }
    out
}
/// 最大 RSS（Linux の VmHWM）。`reset` で今の RSS に戻す（`/proc/self/clear_refs` に 5）。
struct Rss;
impl Rss {
    fn new() -> Self {
        Self
    }
    fn read(key: &str) -> u64 {
        fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with(key))
                    .and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
            })
            .unwrap_or(0)
            * 1024
    }
    fn reset(&self) -> u64 {
        let _ = fs::write("/proc/self/clear_refs", "5");
        let now = Self::read("VmRSS:");
        BASE.store(now, Ordering::Relaxed);
        now
    }
    fn grew(&self) -> u64 {
        Self::read("VmHWM:").saturating_sub(BASE.load(Ordering::Relaxed))
    }
}
static BASE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[allow(dead_code)]
fn unused(_: &Path) {}
