//! 保存の「全エントリの数え」と「書いたものの読み直し」は、エントリごとに並べて動く（rayon）。結果のファイルのバイトも、壊れたファイルを
//! 断る理由も、スレッド数に依らず同じ（名前の順にいちばん前に壊れたエントリの理由）。正本を core の文書から流して作る道（`Made`）と、
//! 今の形・大きな形（`YLP-4`・分けた正本）の両方。
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use yolu_core::{Channel, Document, Rgba8, TileCoord};
use yolu_io::{
    composite_pngs, BackupKeep, DocumentSource, Limits, MaterialRef, Project, SaveTarget, SetSpec, Thresholds,
    WriterInfo,
};

struct Dir(PathBuf);
impl Dir {
    fn new() -> Dir {
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "yolu-io-saveparallel-{}-{}",
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
    WriterInfo { app: "YoluPainter-rs".into(), version: "0.0.0".into(), unity: "standalone".into() }
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
fn painted(seed: u64, layers: usize) -> Document {
    let mut doc = Document::with_tile_size(SIZE, SIZE, TILE).unwrap();
    doc.set_source_budget_bytes(1 << 30).unwrap();
    let n = SIZE / TILE;
    for i in 0..layers {
        let id = doc.add_layer(&format!("層 {i}")).unwrap();
        for ty in 0..n {
            for tx in 0..n {
                if !(tx + ty + i as u32).is_multiple_of(3) {
                    let bytes = noise((TILE * TILE * 4) as usize, seed * 1000 + i as u64 * 31 + (ty * n + tx) as u64);
                    doc.import_tile(id, Channel::Color, TileCoord::new(tx, ty), &bytes).unwrap();
                }
            }
        }
    }
    doc.add_fill_layer("塗り", &[(Channel::Color, Rgba8::new(10, 20, 30, 255))], None).unwrap();
    doc.clear_history().unwrap();
    doc
}
const SETS: [&str; 3] = [
    "0f8fad5b-d9cb-469f-a165-70867728950e",
    "7c9e6679-7425-40de-944b-e07fc1f90ae7",
    "d4b5a1f2-0b5e-4c7d-9a34-6a1f3c2e8b90",
];
/// 3 つのセット（正本は core の文書から流して作る。合成の PNG は圧縮しないエントリ）。
fn project() -> Project {
    let specs: Vec<SetSpec> = SETS
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let doc = painted(i as u64 + 1, 4 + i * 2);
            SetSpec {
                id: (*id).into(),
                name: format!("セット {i}"),
                material: MaterialRef::PendingSlot(i as u16),
                composites: composite_pngs(&doc).unwrap(),
                document: Some(DocumentSource::Core(Arc::new(doc.capture_snapshot().unwrap()))),
            }
        })
        .collect();
    Project::create(writer(), &specs, SETS[0]).unwrap()
}
/// 今の形に収まらない扱い（`YLP-4`・分けた正本・小さな部分）。
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
fn pool(threads: usize) -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap()
}

#[test]
fn the_saved_file_is_the_same_bytes_at_any_thread_count() {
    for (label, thresholds) in [("今の形", Thresholds::REAL), ("大きな形", small())] {
        let dir = Dir::new();
        let mut files = Vec::new();
        // 文書の ID は作るたびに違うので、プロジェクトは 1 つだけ作って、スレッド数だけを替えて保存する（形の決まり方は作るときの閾値）
        let p = thresholds.scoped(project);
        for threads in [1usize, 2, 3, 8] {
            // 閾値はスレッドごとの値なので、保存を動かすスレッド（プールの中）でも決める
            let path = dir.path(&format!("{threads}.ylp"));
            pool(threads).install(|| {
                thresholds.scoped(|| {
                    let mut target = SaveTarget::create(&path).unwrap();
                    target.save_with(&p, BackupKeep::All).unwrap();
                    // 同じ中身を、メモリへ書いたものとも同じ
                    assert_eq!(fs::read(&path).unwrap(), p.to_bytes().unwrap(), "{label} {threads} 本");
                })
            });
            files.push(fs::read(&path).unwrap());
        }
        assert!(files.windows(2).all(|w| w[0] == w[1]), "{label}: スレッド数が違っても同じファイル");
        let version = yolu_io::Package::open(&dir.path("1.ylp"), &Limits::unbounded()).unwrap().manifest_version();
        assert_eq!(version == 4, label == "大きな形", "{label}");
    }
}

/// 壊れたファイルを断る理由は、スレッド数に依らず、名前の順にいちばん前に壊れたエントリのもの。保存の確かめ（書いた一時ファイルの読み直し）も
/// 同じ読み方を使う。
#[test]
fn a_broken_file_is_refused_for_the_same_reason_at_any_thread_count() {
    let dir = Dir::new();
    let path = dir.path("good.ylp");
    small().scoped(|| {
        let p = project();
        SaveTarget::create(&path).unwrap().save_with(&p, BackupKeep::All).unwrap();
    });
    let good = fs::read(&path).unwrap();
    // 圧縮しない合成の PNG の中身を、2 つのセット（名前の順で 2 つ目と 3 つ目）で書き換える
    let mut starts = Vec::new();
    let mut at = 0;
    while let Some(i) = good[at..].windows(8).position(|w| w == [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        starts.push(at + i);
        at += i + 8;
    }
    assert_eq!(starts.len(), 3, "セットごとの合成の PNG");
    let mut bad = good.clone();
    for k in [1usize, 2] {
        bad[starts[k] + 80] ^= 0xff;
    }
    let broken = dir.path("broken.ylp");
    fs::write(&broken, &bad).unwrap();
    let reason = |threads: usize| {
        pool(threads).install(|| {
            (0..8)
                .map(|_| match SaveTarget::open_within(&broken, &Limits::unbounded()) {
                    Ok(_) => panic!("壊れたのに開けた"),
                    Err(e) => format!("{e:?}"),
                })
                .collect::<Vec<_>>()
        })
    };
    let first = reason(1)[0].clone();
    assert!(first.contains(SETS[1]) && !first.contains(SETS[2]), "名前の順で前のセットの PNG が先: {first}");
    for threads in [1, 2, 3, 8] {
        for (i, why) in reason(threads).iter().enumerate() {
            assert_eq!(why, &first, "{threads} 本の {i} 回目");
        }
    }
    // 壊れていないファイルは、どのスレッド数でも開ける
    for threads in [1, 8] {
        pool(threads).install(|| {
            let (opened, _) = SaveTarget::open_within(&path, &Limits::unbounded()).unwrap();
            assert_eq!(opened.sets().len(), 3);
        });
    }
}
