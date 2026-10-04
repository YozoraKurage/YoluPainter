//! 復旧用の世代の置き場（`GenerationStore`）。Unity 版 `RecoveryGenerationTests` の 16 ケースと同じ振る舞い（置換の順序・途中で
//! 落ちた世代・共有・整理・壊れた世代・予算）に、Rust 版が足した口（一覧・世代の指定・捨てる・外の書き手・ロック）の試験を足す。
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use yolu_io::{
    generation_time_ms, utc_stamp, CommitOptions, Fault, Files, GenerationStore, StoreError,
};

struct Dir(PathBuf);
impl Dir {
    fn new() -> Dir {
        static N: AtomicU32 = AtomicU32::new(0);
        Dir(std::env::temp_dir().join(format!(
            "yolu-io-generation-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        )))
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

const SET_A: &str = "11111111-1111-1111-1111-111111111111";
const SET_B: &str = "22222222-2222-2222-2222-222222222222";

fn bytes(b: &[u8]) -> Arc<[u8]> {
    Arc::from(b.to_vec())
}
fn files(value: u8) -> Files {
    let mut f = Files::new();
    f.insert("document.utpaint".into(), bytes(&[value]));
    f.insert("unchanged.bin".into(), bytes(&vec![0u8; 16384]));
    f
}
fn share(keep: Option<usize>, expected: Option<&str>) -> CommitOptions<'_> {
    CommitOptions {
        expected,
        keep,
        share: true,
    }
}
fn fault_at(stage: &'static str) -> Fault {
    Arc::new(move |s: &str| {
        if s.starts_with(stage) {
            Err(io::Error::other("注入した障害"))
        } else {
            Ok(())
        }
    })
}
fn pointer(root: &Path, name: &str) -> String {
    fs::read_to_string(root.join(name)).unwrap().trim().to_owned()
}
fn generations(root: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(root.join("generations"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}
fn contents(root: &Path) -> usize {
    fs::read_dir(root.join("contents")).unwrap().count()
}
fn hash_of(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(b))
}

#[test]
fn only_generations_beyond_the_chosen_count_are_removed() {
    for keep in [2usize, 3, 5] {
        let dir = Dir::new();
        let store = GenerationStore::new(&dir.0);
        let mut saved: Option<yolu_io::Committed> = None;
        let mut previous = None;
        for i in 0..9u8 {
            previous = saved.as_ref().map(|s| s.id.clone());
            let token = saved.as_ref().map(|s| s.token.clone());
            saved = Some(
                store
                    .commit(&files(i), &share(Some(keep), token.as_deref()))
                    .unwrap(),
            );
        }
        let saved = saved.unwrap();
        let previous = previous.unwrap();
        let kept = generations(&dir.0);
        assert_eq!(kept.len(), keep, "keep = {keep}");
        assert!(kept.contains(&saved.id) && kept.contains(&previous));
        assert_eq!(pointer(&dir.0, "current"), saved.id);
        assert_eq!(pointer(&dir.0, "previous"), previous);
        assert_eq!(&*store.load().unwrap().files["document.utpaint"], &[8u8]);
        assert_eq!(
            contents(&dir.0),
            keep + 1,
            "共有した変わらない 1 つと、残した世代ごとの正本"
        );
    }
}

#[test]
fn an_invalid_retention_count_writes_nothing() {
    let dir = Dir::new();
    let store = GenerationStore::new(dir.path("root"));
    assert!(matches!(
        store.commit(&files(0), &share(Some(1), None)),
        Err(StoreError::InvalidArgument(_))
    ));
    assert!(!dir.path("root").exists());
}

#[test]
fn files_in_the_generations_folder_that_are_not_generations_neither_stop_nor_suffer_the_cleanup() {
    let dir = Dir::new();
    let store = GenerationStore::new(&dir.0);
    let mut saved: Option<yolu_io::Committed> = None;
    for i in 0..3u8 {
        let token = saved.as_ref().map(|s| s.token.clone());
        saved = Some(store.commit(&files(i), &share(Some(3), token.as_deref())).unwrap());
    }
    // OS やユーザーが置いたもの（ファイル・名前が世代の形でないフォルダ・世代に似た名前のファイル）
    let folder = dir.path("generations");
    fs::write(folder.join(".DS_Store"), b"x").unwrap();
    fs::write(folder.join("desktop.ini"), b"x").unwrap();
    fs::write(folder.join("20200101T000000000-abc"), b"a file, not a folder").unwrap();
    fs::create_dir(folder.join("notes")).unwrap();
    fs::write(folder.join("notes").join("a.txt"), b"mine").unwrap();
    for i in 3..9u8 {
        let token = saved.as_ref().map(|s| s.token.clone());
        saved = Some(store.commit(&files(i), &share(Some(3), token.as_deref())).unwrap());
    }
    let saved = saved.unwrap();
    let real: Vec<String> = generations(&dir.0)
        .into_iter()
        .filter(|n| generation_time_ms(n).is_some() && folder.join(n).is_dir())
        .collect();
    assert_eq!(real.len(), 3, "置かれたものがあっても、設定の数まで整理する: {real:?}");
    assert!(real.contains(&saved.id));
    assert_eq!(&*store.load().unwrap().files["document.utpaint"], &[8u8]);
    // 世代でないものは、消さない
    assert!(folder.join(".DS_Store").is_file() && folder.join("desktop.ini").is_file());
    assert!(folder.join("20200101T000000000-abc").is_file());
    assert_eq!(fs::read(folder.join("notes").join("a.txt")).unwrap(), b"mine");
}

#[test]
fn a_commit_over_the_write_budget_is_refused_before_anything_is_written_and_the_previous_generation_stays() {
    let dir = Dir::new();
    // 予算を小さくして、巨大な領域を確保せずに、書く側の上限（1 エントリ・合計）を確かめる
    let small = GenerationStore::new(dir.path("root")).with_budget(1024, 4096);
    let tiny = |value: u8| {
        let mut f = Files::new();
        f.insert("document.utpaint".into(), bytes(&[value]));
        f.insert("part.bin".into(), bytes(&[value; 100]));
        f
    };
    let big_entry = {
        let mut f = tiny(1);
        f.insert("big.bin".into(), bytes(&vec![7u8; 2048]));
        f
    };
    let mut many = Files::new();
    for i in 0..5 {
        many.insert(if i == 0 { "document.utpaint".to_owned() } else { format!("part{i}.bin") }, bytes(&vec![i as u8; 1000]));
    }
    // 何も書いていない置き場: 断っても何も作らない
    for (what, f) in [("1 エントリの上限", &big_entry), ("合計の上限", &many)] {
        match small.commit(f, &share(None, None)) {
            Err(StoreError::Budget(why)) => {
                assert!(!why.chars().any(|c| c.is_ascii_digit()), "{what}: 理由に開発用の数を出さない: {why}");
            }
            other => panic!("{what}: 予算で断るはず: {other:?}"),
        }
        assert!(!dir.path("root").exists(), "{what}");
    }
    // 前の世代がある置き場: 断っても current・previous・世代の数・作りかけは変わらない
    let store = GenerationStore::new(dir.path("root"));
    let first = store.commit(&tiny(1), &share(Some(3), None)).unwrap();
    let snapshot = |root: &Path| {
        let mut names: Vec<String> = fs::read_dir(root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        (names, generations(root), contents(root), pointer(root, "current"))
    };
    let before = snapshot(&dir.path("root"));
    let small = GenerationStore::new(dir.path("root")).with_budget(1024, 4096);
    for f in [&big_entry, &many] {
        assert!(matches!(
            small.commit(f, &share(Some(3), Some(&first.token))),
            Err(StoreError::Budget(_))
        ));
        assert_eq!(snapshot(&dir.path("root")), before);
    }
    assert_eq!(store.load().unwrap().token, first.token);
    // 予算の中なら、同じ置き場へ確定できる（予算は既定より大きくならない）
    let ok = small.commit(&tiny(2), &share(Some(3), Some(&first.token))).unwrap();
    assert_eq!(store.load().unwrap().token, ok.token);
    let wide = GenerationStore::new(dir.path("root")).with_budget(u64::MAX, u64::MAX);
    assert!(wide.commit(&files(3), &share(Some(3), Some(&ok.token))).is_ok(), "予算は広げても既定の上限まで。小さくしていない置き場と同じく、通常の書き込みは通る");
}

#[test]
fn an_empty_or_nativeless_commit_is_refused_before_touching_the_disk() {
    let dir = Dir::new();
    let store = GenerationStore::new(dir.path("root"));
    assert!(matches!(
        store.commit(&Files::new(), &share(None, None)),
        Err(StoreError::InvalidArgument(_))
    ));
    let mut only_info = Files::new();
    only_info.insert("recovery.json".into(), bytes(b"{}"));
    assert!(matches!(
        store.commit(&only_info, &share(None, None)),
        Err(StoreError::InvalidArgument(_))
    ));
    assert!(!dir.path("root").exists());
}

#[test]
fn an_unchanged_set_writes_no_content_again() {
    let dir = Dir::new();
    let store = GenerationStore::new(&dir.0);
    let (a_name, b_name) = (
        format!("sets/{SET_A}/document.utpaint"),
        format!("sets/{SET_B}/document.utpaint"),
    );
    let mut f = Files::new();
    f.insert(a_name.clone(), bytes(b"set A, version 1"));
    f.insert(b_name.clone(), bytes(b"set B, never changes"));
    let first = store.commit(&f, &share(Some(2), None)).unwrap();
    assert_eq!((first.reused_files, first.written_bytes > 0), (0, true));
    let untouched = dir
        .path("contents")
        .join(format!("{}.bin", hash_of(b"set B, never changes")));
    let sentinel = UNIX_EPOCH + Duration::from_secs(978_307_200); // 2001-01-01
    fs::File::options()
        .write(true)
        .open(&untouched)
        .unwrap()
        .set_modified(sentinel)
        .unwrap();
    let changed = bytes(b"set A, version 2 (changed)");
    f.insert(a_name, changed.clone());
    let second = store
        .commit(&f, &share(Some(2), Some(&first.token)))
        .unwrap();
    assert_eq!(second.written_bytes, changed.len() as u64);
    assert_eq!(second.reused_files, 1);
    assert_eq!(
        fs::metadata(&untouched).unwrap().modified().unwrap(),
        sentinel,
        "変わらないセットの中身は書き直さない"
    );
    assert_eq!(contents(&dir.0), 3);
    let third = store
        .commit(&f, &share(Some(2), Some(&second.token)))
        .unwrap();
    assert_eq!((third.written_bytes, third.reused_files), (0, 2));
    assert_eq!(store.load().unwrap().files, f);
}

#[test]
fn cleanup_failure_does_not_turn_a_committed_save_into_failure() {
    for point in ["prune-generation:", "prune-content:"] {
        let dir = Dir::new();
        let store = GenerationStore::new(&dir.0);
        let first = store.commit(&files(1), &share(Some(2), None)).unwrap();
        let second = store
            .commit(&files(2), &share(Some(2), Some(&first.token)))
            .unwrap();
        let hits = Arc::new(Mutex::new(0));
        let counter = hits.clone();
        let faulty = store.clone().with_fault(Arc::new(move |s: &str| {
            if s.starts_with(point) {
                *counter.lock().unwrap() += 1;
                Err(io::Error::other("掃除を断る"))
            } else {
                Ok(())
            }
        }));
        let third = faulty
            .commit(&files(3), &share(Some(2), Some(&second.token)))
            .unwrap();
        assert!(*hits.lock().unwrap() > 0, "{point}");
        assert_eq!(store.load().unwrap().token, third.token);
        let kept = generations(&dir.0);
        assert!(kept.contains(&third.id) && kept.contains(&second.id), "{point}");
        let fourth = store
            .commit(&files(4), &share(Some(2), Some(&third.token)))
            .unwrap();
        assert_eq!(generations(&dir.0).len(), 2, "次の確定で整理し直す: {point}");
        assert_eq!(store.load().unwrap().token, fourth.token);
        assert_eq!(contents(&dir.0), 3);
    }
}

#[test]
fn shared_content_save_failure_keeps_current_and_previous() {
    for point in ["file:document.utpaint", "verified", "generation-renamed", "before-pointer"] {
        let dir = Dir::new();
        let store = GenerationStore::new(&dir.0);
        let first = store.commit(&files(1), &share(Some(3), None)).unwrap();
        let second = store
            .commit(&files(2), &share(Some(3), Some(&first.token)))
            .unwrap();
        let faulty = store.clone().with_fault(fault_at(point));
        let result = faulty.commit(&files(3), &share(Some(2), Some(&second.token)));
        assert!(matches!(result, Err(StoreError::Io(_))), "{point}");
        assert_eq!(store.load().unwrap().token, second.token, "{point}");
        assert_eq!(pointer(&dir.0, "previous"), first.id, "{point}");
        let kept = generations(&dir.0);
        assert!(kept.contains(&first.id) && kept.contains(&second.id), "{point}");
        // 確定していない作りかけは片付け、残った世代は読める
        assert!(
            fs::read_dir(&dir.0)
                .unwrap()
                .all(|e| !e.unwrap().file_name().to_string_lossy().starts_with(".staging-")),
            "{point}: 作りかけが残っている"
        );
        assert!(
            fs::read_dir(&dir.0)
                .unwrap()
                .all(|e| !e.unwrap().file_name().to_string_lossy().starts_with(".current-")),
            "{point}: 一時のポインタが残っている"
        );
        // そのあと普通に確定できる
        let next = store
            .commit(&files(4), &share(Some(3), Some(&second.token)))
            .unwrap();
        assert_eq!(store.load().unwrap().token, next.token);
    }
}

#[test]
fn current_is_replaced_last_and_previous_follows() {
    let dir = Dir::new();
    let store = GenerationStore::new(&dir.0);
    let first = store.commit(&files(1), &share(None, None)).unwrap();
    let log: Arc<Mutex<Vec<(String, String, bool)>>> = Arc::default();
    let seen = log.clone();
    let root = dir.0.clone();
    let spy = store.clone().with_fault(Arc::new(move |stage: &str| {
        let current = fs::read_to_string(root.join("current")).unwrap_or_default();
        let manifests = fs::read_dir(root.join("generations"))
            .unwrap()
            .filter(|e| e.as_ref().unwrap().path().join("manifest.sha256").exists())
            .count();
        seen.lock()
            .unwrap()
            .push((stage.to_owned(), current.trim().to_owned(), manifests == 2));
        Ok(())
    }));
    let second = spy
        .commit(&files(2), &share(None, Some(&first.token)))
        .unwrap();
    let log = log.lock().unwrap();
    let stages: Vec<&str> = log.iter().map(|(s, _, _)| s.as_str()).collect();
    assert_eq!(
        stages,
        [
            "file:document.utpaint",
            "file:unchanged.bin",
            "verified",
            "generation-renamed",
            "before-pointer",
            "after-pointer"
        ]
    );
    for (stage, current, renamed) in log.iter() {
        let after = stage == "after-pointer";
        assert_eq!(
            current,
            if after { &second.id } else { &first.id },
            "{stage}: current は最後の 1 回だけ替わる"
        );
        assert_eq!(
            *renamed,
            matches!(stage.as_str(), "generation-renamed" | "before-pointer" | "after-pointer"),
            "{stage}: 世代の名前への改名は確かめた後"
        );
    }
    assert_eq!(pointer(&dir.0, "previous"), first.id);
}

#[test]
fn a_generation_the_process_died_building_is_ignored_and_the_store_stays_readable() {
    let dir = Dir::new();
    let store = GenerationStore::new(&dir.0);
    let first = store.commit(&files(1), &share(Some(3), None)).unwrap();
    // 落ちた確定の残り: 作りかけ（manifest 無し）と一時のポインタ
    let staged = dir.path(".staging-died");
    fs::create_dir_all(&staged).unwrap();
    fs::write(staged.join("partial.pending"), b"half").unwrap();
    fs::write(dir.path(".current-abandoned"), b"20990101T000000000-ff\n").unwrap();
    assert_eq!(store.load().unwrap().token, first.token);
    assert_eq!(store.list().unwrap().len(), 1, "作りかけは一覧に出ない");
    let second = store
        .commit(&files(2), &share(Some(3), Some(&first.token)))
        .unwrap();
    assert_eq!(store.load().unwrap().token, second.token);
    assert!(staged.exists(), "ほかの確定の作りかけには触らない");
}

#[test]
fn staging_folders_and_their_shared_content_remain_protected() {
    let dir = Dir::new();
    let store = GenerationStore::new(&dir.0);
    let first = store.commit(&files(1), &share(None, None)).unwrap();
    let staged = dir.path(".staging-protected");
    fs::create_dir_all(&staged).unwrap();
    let orphan = [42u8];
    let digest = hash_of(&orphan);
    fs::write(dir.path("contents").join(format!("{digest}.bin")), orphan).unwrap();
    fs::write(
        staged.join("manifest.sha256"),
        format!("DOTPAINT-MANIFEST-2\n{digest} 1 document.utpaint\n"),
    )
    .unwrap();
    let mut token = first.token;
    for i in 2..4u8 {
        token = store
            .commit(&files(i), &share(Some(2), Some(&token)))
            .unwrap()
            .token;
    }
    assert!(staged.exists());
    assert!(dir.path("contents").join(format!("{digest}.bin")).exists());
    fs::remove_file(staged.join("manifest.sha256")).unwrap();
    let again = store
        .commit(&files(4), &share(Some(2), Some(&token)))
        .unwrap();
    assert_eq!(again.token, store.load().unwrap().token);
    assert!(
        dir.path("contents").join(format!("{digest}.bin")).exists(),
        "作りかけ（manifest 無し）があるあいだは、共有の中身を集めない"
    );
}

#[test]
fn old_flat_generations_load_and_remain_as_previous_after_sharing_starts() {
    let dir = Dir::new();
    let store = GenerationStore::new(&dir.0);
    let flat = CommitOptions::default();
    let first = store.commit(&files(1), &flat).unwrap();
    assert_eq!(store.load().unwrap().files, files(1));
    let second = store
        .commit(&files(2), &share(Some(2), Some(&first.token)))
        .unwrap();
    assert!(dir
        .path("generations")
        .join(&first.id)
        .join("document.utpaint")
        .exists());
    assert_eq!(pointer(&dir.0, "previous"), first.id);
    store
        .commit(&files(3), &share(Some(2), Some(&second.token)))
        .unwrap();
    assert!(!dir.path("generations").join(&first.id).exists());
}

#[test]
fn a_flat_generation_keeps_nested_names_inside_its_own_folder() {
    let dir = Dir::new();
    let store = GenerationStore::new(&dir.0);
    let mut f = Files::new();
    f.insert(format!("sets/{SET_A}/document.utpaint"), bytes(b"doc"));
    f.insert(format!("sets/{SET_A}/composite/Color.png"), bytes(b"png"));
    f.insert("resources/shelf.json".into(), bytes(b"{}"));
    let saved = store.commit(&f, &CommitOptions::default()).unwrap();
    assert!(dir
        .path("generations")
        .join(&saved.id)
        .join("sets")
        .join(SET_A)
        .join("composite")
        .join("Color.png")
        .exists());
    assert_eq!(store.load().unwrap().files, f);
}

#[test]
fn tampered_shared_content_is_refused_before_another_save() {
    let dir = Dir::new();
    let store = GenerationStore::new(&dir.0);
    let first = store.commit(&files(1), &share(Some(3), None)).unwrap();
    fs::write(
        dir.path("contents").join(format!("{}.bin", hash_of(&[1]))),
        [2u8],
    )
    .unwrap();
    assert!(matches!(store.load(), Err(StoreError::Corrupt(_))));
    let refused = store.commit(&files(3), &share(Some(3), Some(&first.token)));
    assert!(matches!(refused, Err(StoreError::Changed(_))), "{refused:?}");
    assert_eq!(pointer(&dir.0, "current"), first.id);
}

#[test]
fn small_metadata_reads_are_verified_and_bounded_in_both_layouts() {
    let dir = Dir::new();
    let store = GenerationStore::new(&dir.0);
    let mut f = files(1);
    f.insert("recovery.json".into(), bytes(b"{}"));
    let first = store.commit(&f, &CommitOptions::default()).unwrap();
    assert_eq!(
        store.read_file(None, "recovery.json", 32).unwrap().unwrap(),
        b"{}"
    );
    let second = store
        .commit(&f, &share(None, Some(&first.token)))
        .unwrap();
    assert_eq!(
        store.read_file(None, "recovery.json", 32).unwrap().unwrap(),
        b"{}"
    );
    assert_eq!(
        store
            .read_file(Some(&first.id), "recovery.json", 32)
            .unwrap()
            .unwrap(),
        b"{}",
        "古い世代も名前で読める"
    );
    assert!(matches!(
        store.read_file(Some(&second.id), "recovery.json", 1),
        Err(StoreError::Budget(_))
    ));
    assert!(store.read_file(None, "missing.json", 32).unwrap().is_none());
}

#[test]
fn a_shared_pointer_cannot_escape_the_content_folder() {
    let dir = Dir::new();
    let store = GenerationStore::new(&dir.0);
    let first = store.commit(&files(1), &share(None, None)).unwrap();
    let manifest = dir.path("generations").join(&first.id).join("manifest.sha256");
    fs::write(
        manifest,
        format!("DOTPAINT-MANIFEST-2\n{} 1 document.utpaint\n", ".".repeat(64)),
    )
    .unwrap();
    assert!(matches!(store.load(), Err(StoreError::Corrupt(_))));
}

#[test]
fn manifests_with_unsafe_lines_are_refused() {
    let dir = Dir::new();
    let store = GenerationStore::new(&dir.0);
    let first = store.commit(&files(1), &share(None, None)).unwrap();
    let manifest = dir.path("generations").join(&first.id).join("manifest.sha256");
    let good = hash_of(&[1]);
    let cases = [
        ("見出しが違う", "DOTPAINT-MANIFEST-9\n".to_owned() + &format!("{good} 1 document.utpaint\n")),
        ("上の階へ出る名前", format!("DOTPAINT-MANIFEST-2\n{good} 1 ../document.utpaint\n")),
        ("大文字の札", format!("DOTPAINT-MANIFEST-2\n{} 1 document.utpaint\n", good.to_uppercase())),
        ("名前に空白", format!("DOTPAINT-MANIFEST-2\n{good} 1 document .utpaint\n")),
        ("重複", format!("DOTPAINT-MANIFEST-2\n{good} 1 document.utpaint\n{good} 1 document.utpaint\n")),
        ("manifest の名前", format!("DOTPAINT-MANIFEST-2\n{good} 1 manifest.sha256\n{good} 1 document.utpaint\n")),
        ("正本が無い", format!("DOTPAINT-MANIFEST-2\n{good} 1 other.bin\n")),
        ("1 つの長さが上限超え", format!("DOTPAINT-MANIFEST-2\n{good} {} document.utpaint\n", 600u64 << 20)),
        ("長さが数でない", format!("DOTPAINT-MANIFEST-2\n{good} x document.utpaint\n")),
    ];
    for (why, text) in cases {
        fs::write(&manifest, text).unwrap();
        assert!(
            matches!(store.load(), Err(StoreError::Corrupt(_))),
            "{why}: {:?}",
            store.load().err()
        );
    }
    // 合計が予算を超える manifest は、壊れたものとは別に予算の拒否
    fs::write(
        &manifest,
        format!(
            "DOTPAINT-MANIFEST-2\n{good} {} a.bin\n{good} {} document.utpaint\n",
            500u64 << 20,
            300u64 << 20
        ),
    )
    .unwrap();
    assert!(matches!(store.load(), Err(StoreError::Budget(_))));
}

#[test]
fn entry_names_that_could_escape_are_refused_before_anything_is_written() {
    let dir = Dir::new();
    let store = GenerationStore::new(dir.path("root"));
    for name in [
        "../escape",
        "a/../b",
        "sets/x/..",
        "with space",
        ".hidden",
        "manifest.sha256",
        "sets/a/b/c/d/e",
        "",
        "日本語",
    ] {
        let mut f = files(1);
        f.insert(name.into(), bytes(b"x"));
        assert!(
            matches!(store.commit(&f, &share(None, None)), Err(StoreError::Corrupt(_))),
            "{name:?}"
        );
    }
    assert!(!dir.path("root").exists());
}

#[test]
fn an_external_change_to_current_is_refused_not_adopted() {
    let dir = Dir::new();
    let store = GenerationStore::new(&dir.0);
    let first = store.commit(&files(1), &share(Some(3), None)).unwrap();
    // 別の書き手（同じ置き場を別に開いたもの）が先に確定した
    let other = GenerationStore::new(&dir.0);
    let external = other
        .commit(&files(9), &share(Some(3), Some(&first.token)))
        .unwrap();
    assert!(store.has_external_change(&first.token));
    assert!(!store.has_external_change(&external.token));
    for _ in 0..2 {
        let refused = store.commit(&files(2), &share(Some(3), Some(&first.token)));
        assert!(matches!(refused, Err(StoreError::Changed(_))), "{refused:?}");
        assert_eq!(store.load().unwrap().token, external.token);
    }
    // 札が無いまま、世代のある置き場へは確定しない
    assert!(matches!(
        store.commit(&files(2), &share(Some(3), None)),
        Err(StoreError::AlreadyExists)
    ));
    // 札を読み直した書き手は確定できる
    store
        .commit(&files(2), &share(Some(3), Some(&external.token)))
        .unwrap();
}

#[test]
fn a_missing_current_is_an_external_change_for_a_writer_that_expects_one() {
    let dir = Dir::new();
    let store = GenerationStore::new(&dir.0);
    let first = store.commit(&files(1), &share(None, None)).unwrap();
    fs::remove_file(dir.path("current")).unwrap();
    assert!(matches!(store.load(), Err(StoreError::NoGeneration)));
    assert!(matches!(
        store.commit(&files(2), &share(None, Some(&first.token))),
        Err(StoreError::Changed(_))
    ));
    assert!(store.has_external_change(&first.token));
}

#[test]
fn a_second_writer_gets_busy_while_the_lock_is_held() {
    let dir = Dir::new();
    let store = GenerationStore::new(&dir.0);
    let first = store.commit(&files(1), &share(None, None)).unwrap();
    let (reached_tx, reached_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Mutex::new(release_rx);
    let reached_tx = Mutex::new(reached_tx);
    let slow = store.clone().with_fault(Arc::new(move |stage: &str| {
        if stage == "verified" {
            reached_tx.lock().unwrap().send(()).unwrap();
            release_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .map_err(io::Error::other)?;
        }
        Ok(())
    }));
    let token = first.token.clone();
    let writer = std::thread::spawn(move || slow.commit(&files(2), &share(Some(3), Some(&token))));
    reached_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let blocked = store.commit(&files(3), &share(Some(3), Some(&first.token)));
    assert!(matches!(blocked, Err(StoreError::Busy)), "{blocked:?}");
    assert_eq!(
        store.remove_generation(&first.id).err().map(|e| matches!(e, StoreError::Busy)),
        Some(true),
        "捨てるのも、書いているあいだは待たずに断る"
    );
    release_tx.send(()).unwrap();
    let done = writer.join().unwrap().unwrap();
    assert_eq!(store.load().unwrap().token, done.token);
    // ロックは終われば手放す（残ったファイルは邪魔にならない）
    store
        .commit(&files(4), &share(Some(3), Some(&done.token)))
        .unwrap();
}

#[test]
fn generation_ids_are_strictly_increasing_even_within_one_millisecond() {
    let dir = Dir::new();
    let store = GenerationStore::new(&dir.0);
    let mut token: Option<String> = None;
    let mut ids = Vec::new();
    for i in 0..30u8 {
        let saved = store
            .commit(&files(i), &share(None, token.as_deref()))
            .unwrap();
        token = Some(saved.token);
        ids.push(saved.id);
    }
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "名前の順が確定の順");
    sorted.dedup();
    assert_eq!(sorted.len(), 30);
}

#[test]
fn the_listing_is_newest_first_counts_documents_and_names_the_reason_for_broken_generations() {
    let dir = Dir::new();
    let store = GenerationStore::new(&dir.0);
    assert!(store.list().unwrap().is_empty(), "置き場がまだ無い");
    let named = |value: u8| {
        let mut f = Files::new();
        f.insert(format!("sets/{SET_A}/document.utpaint"), bytes(&[value; 10]));
        f.insert(format!("sets/{SET_B}/document.utpaint"), bytes(&[value; 20]));
        f.insert("recovery.json".into(), bytes(b"{}"));
        f
    };
    let before = SystemTime::now();
    let mut token: Option<String> = None;
    let mut ids = Vec::new();
    for i in 0..4u8 {
        let saved = store
            .commit(&named(i), &share(Some(10), token.as_deref()))
            .unwrap();
        token = Some(saved.token);
        ids.push(saved.id);
    }
    // 古い方から 2 つ壊す: 中身の長さを変える・manifest を消す
    fs::write(
        dir.path("contents").join(format!("{}.bin", hash_of(&[0u8; 10]))),
        b"cut",
    )
    .unwrap();
    fs::remove_file(dir.path("generations").join(&ids[1]).join("manifest.sha256")).unwrap();
    let list = store.list().unwrap();
    let order: Vec<&str> = list.iter().map(|g| g.id.as_str()).collect();
    assert_eq!(order, [&ids[3], &ids[2], &ids[1], &ids[0]]);
    assert!(list[0].is_current && !list[1].is_current);
    for good in &list[..2] {
        assert_eq!((good.documents, good.entries, good.shared), (2, 3, true));
        assert_eq!(good.bytes, 10 + 20 + 2);
        assert!(good.problem.is_none());
        let age = good.time_ms.unwrap() as i64
            - before.duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        assert!((-1000..60_000).contains(&age), "{age}");
    }
    assert!(list[2].problem.as_deref().unwrap().contains("manifest"));
    assert!(list[3].problem.as_deref().unwrap().contains("長さ"));
    // 壊れた世代があっても、ほかの世代は名前で読める。壊れた世代は理由で断る
    assert_eq!(store.load_generation(&ids[2]).unwrap().id, ids[2]);
    assert!(matches!(store.load_generation(&ids[1]), Err(StoreError::Corrupt(_))));
    assert!(matches!(store.load_generation(&ids[0]), Err(StoreError::Corrupt(_))));
    assert!(matches!(
        store.load_generation("../../etc"),
        Err(StoreError::Corrupt(_))
    ));
}

#[test]
fn removing_a_generation_repoints_the_pointers_and_collects_unused_content() {
    let dir = Dir::new();
    let store = GenerationStore::new(&dir.0);
    let first = store.commit(&files(1), &share(Some(5), None)).unwrap();
    let second = store
        .commit(&files(2), &share(Some(5), Some(&first.token)))
        .unwrap();
    let third = store
        .commit(&files(3), &share(Some(5), Some(&second.token)))
        .unwrap();
    assert_eq!(contents(&dir.0), 4);
    // 真ん中（previous の指す先ではない）を捨てる
    store.remove_generation(&first.id).unwrap();
    assert_eq!(generations(&dir.0).len(), 2);
    assert_eq!(contents(&dir.0), 3, "first だけが使っていた中身を消す");
    // current の世代を捨てると、残りで一番新しい世代へ付け替える
    store.remove_generation(&third.id).unwrap();
    assert_eq!(pointer(&dir.0, "current"), second.id);
    assert_eq!(store.load().unwrap().id, second.id);
    assert_eq!(pointer(&dir.0, "previous"), second.id);
    // 最後の 1 つを捨てると、ポインタも無くなる
    store.remove_generation(&second.id).unwrap();
    assert!(!dir.path("current").exists() && !dir.path("previous").exists());
    assert!(matches!(store.load(), Err(StoreError::NoGeneration)));
    assert_eq!(contents(&dir.0), 0);
    // もう無い世代・不正な名前
    store.remove_generation(&second.id).unwrap();
    assert!(store.remove_generation("../x").is_err());
    // 捨てたあとの置き場へ、また確定できる
    let again = store.commit(&files(4), &share(Some(5), None)).unwrap();
    assert_eq!(store.load().unwrap().token, again.token);
}

#[test]
fn a_store_written_in_the_unity_layout_is_read_back() {
    // Unity 版の GenerationStore が書く形（見出し -2・contents/<札>.bin・current）を手で作って読む
    let dir = Dir::new();
    let id = "20261003T120000000-0123456789abcdef0123456789abcdef";
    let native = b"native bytes";
    let digest = hash_of(native);
    let info = b"{\"title\":\"x\"}";
    fs::create_dir_all(dir.path("contents")).unwrap();
    fs::create_dir_all(dir.path("generations").join(id)).unwrap();
    fs::write(dir.path("contents").join(format!("{digest}.bin")), native).unwrap();
    fs::write(
        dir.path("contents").join(format!("{}.bin", hash_of(info))),
        info,
    )
    .unwrap();
    fs::write(
        dir.path("generations").join(id).join("manifest.sha256"),
        format!(
            "DOTPAINT-MANIFEST-2\n{digest} {} sets/{SET_A}/document.utpaint\n{} {} recovery.json\n",
            native.len(),
            hash_of(info),
            info.len()
        ),
    )
    .unwrap();
    fs::write(dir.path("current"), format!("{id}\n")).unwrap();
    let store = GenerationStore::new(&dir.0);
    let loaded = store.load().unwrap();
    assert_eq!(loaded.id, id);
    let expected: BTreeMap<_, _> = [
        (format!("sets/{SET_A}/document.utpaint"), native.to_vec()),
        ("recovery.json".to_owned(), info.to_vec()),
    ]
    .into_iter()
    .collect();
    let got: BTreeMap<_, _> = loaded.files.iter().map(|(k, v)| (k.clone(), v.to_vec())).collect();
    assert_eq!(got, expected);
    assert_eq!(generation_time_ms(id), Some(1_791_028_800_000));
    // Rust 版が確定を重ねても、その世代は previous として残る
    let next = store
        .commit(&files(1), &share(Some(2), Some(&loaded.token)))
        .unwrap();
    assert_eq!(pointer(&dir.0, "previous"), id);
    assert_eq!(store.load().unwrap().token, next.token);
}

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}
fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[test]
fn a_store_written_by_the_unity_code_is_read_back_listed_and_carried_on() {
    // `tools/io-fixtures` が Unity 版の `GenerationStore.Commit` で書いた 2 世代（変わらない中身は共有）。置き場を触るので、写しで試す
    let dir = Dir::new();
    copy_dir(&fixture_path("unity-generation"), &dir.0);
    let store = GenerationStore::new(&dir.0);
    let newest = store.load().unwrap();
    let names: Vec<&str> = newest.files.keys().map(String::as_str).collect();
    assert_eq!(names.len(), 4, "{names:?}");
    assert!(names.iter().any(|n| n.starts_with("sets/") && n.ends_with("/document.utpaint")));
    assert!(names.iter().any(|n| n.starts_with("sets/") && n.ends_with("/selection.bin")));
    assert!(names.iter().any(|n| n.starts_with("resources/") && n.ends_with(".png")));
    assert!(String::from_utf8_lossy(&newest.files["recovery.json"]).contains("second"));
    let listed = store.list().unwrap();
    assert_eq!(listed.len(), 2);
    assert!(listed[0].is_current && listed[0].shared && listed[0].documents == 1 && listed[0].problem.is_none());
    assert!(listed[1].problem.is_none());
    assert_eq!(pointer(&dir.0, "previous"), listed[1].id, "Unity 版の previous の書き方（File.Replace）も同じ形");
    let older = store.load_generation(&listed[1].id).unwrap();
    assert!(String::from_utf8_lossy(&older.files["recovery.json"]).contains("first"));
    assert_eq!(contents(&dir.0), 6, "選択範囲と resources の中身は 2 世代で共有");
    // Rust 版が確定を重ねると、Unity 版の current は previous になり、2 つ前は整理される（共有していた中身は残る）
    let mut next = Files::new();
    for (name, data) in &newest.files {
        next.insert(name.clone(), data.clone());
    }
    next.insert("recovery.json".into(), bytes(b"{\"title\":\"third\"}"));
    let committed = store.commit(&next, &share(Some(2), Some(&newest.token))).unwrap();
    assert_eq!(pointer(&dir.0, "previous"), newest.id);
    assert_eq!(generations(&dir.0).len(), 2);
    assert!(!generations(&dir.0).contains(&listed[1].id));
    assert_eq!(committed.reused_files, 3);
    assert_eq!(store.load().unwrap().token, committed.token);
}

#[test]
fn names_inside_a_set_folder_are_wider_here_than_in_unity_and_the_difference_is_recorded() {
    // Unity 版（`GenerationStore.ValidateName`）は `sets/<ID>/` の下に `/` を含む名前を断る。この置き場は `.ylp` のエントリ名と
    // 同じ範囲（合成の PNG `sets/<ID>/composite/Color.png` など）を通す。開いた `.ylp` のエントリをそのまま世代に入れた置き場を
    // Unity 版に読ませた記録（`rust-generation.unity.txt`）と、同じ入力でこの置き場が読めることを突き合わせる
    let verdict = fs::read_to_string(fixture_path("rust-generation.unity.txt")).unwrap();
    let line = |folder: &str| verdict.lines().find(|l| l.starts_with(folder)).unwrap_or_else(|| panic!("{folder}: {verdict}"));
    assert!(line("as-opened:").contains("Unsafe generation filename"), "{verdict}");
    assert!(line("flat-names:").contains("読めた"), "{verdict}");
    let project = yolu_io::Project::read(&fs::read(fixture_path("format6.ylp")).unwrap()).unwrap();
    let entries = project.original_archive().entries().clone();
    let nested: Vec<&String> = entries.keys().filter(|n| n.matches('/').count() > 2).collect();
    assert!(!nested.is_empty() && nested.iter().all(|n| n.contains("/composite/")), "{nested:?}");
    let flat: Files = entries
        .iter()
        .filter(|(n, _)| n.matches('/').count() <= 2)
        .map(|(n, d)| (n.clone(), d.clone()))
        .collect();
    for (what, files) in [("開いたままの全エントリ", &entries), ("入れ子を除いたもの", &flat)] {
        let dir = Dir::new();
        let store = GenerationStore::new(&dir.0);
        store.commit(files, &share(None, None)).unwrap();
        assert_eq!(store.load().unwrap().files, *files, "{what}");
    }
}

#[test]
fn utc_stamps_round_trip_and_malformed_names_have_no_time() {
    for ms in [0u64, 951_782_400_000 /* 2000-02-29 */, 1_709_164_800_123 /* 2024-02-29 */, 1_791_028_800_999, 4_102_444_799_000] {
        let stamp = utc_stamp(ms);
        assert_eq!(stamp.len(), 18, "{stamp}");
        assert_eq!(generation_time_ms(&format!("{stamp}-abcd")), Some(ms), "{stamp}");
    }
    assert_eq!(utc_stamp(0), "19700101T000000000");
    assert_eq!(utc_stamp(1_709_164_800_123), "20240229T000000123");
    for bad in ["", "x", "20261003T120000-aa", "2026100312000000-aa", "20261301T120000000-aa", "20261003T250000000-aa", "20261003X120000000-aa"] {
        assert_eq!(generation_time_ms(bad), None, "{bad:?}");
    }
}

#[test]
fn recovery_info_round_trips_and_tolerates_unknown_or_missing_keys() {
    let info = yolu_io::RecoveryInfo {
        title: "作品".into(),
        project_path: "/tmp/作品.ylp".into(),
        project_token: "abc".into(),
        unchanged: true,
        sets: 3,
    };
    assert_eq!(yolu_io::RecoveryInfo::from_bytes(&info.to_bytes()).unwrap(), info);
    // Unity 版の recovery.json（sets 無し）と、知らないキー
    let unity = br#"{"title":"x","projectPath":"","projectToken":"","unchanged":false,"future":[1,2]}"#;
    let read = yolu_io::RecoveryInfo::from_bytes(unity).unwrap();
    assert_eq!((read.title.as_str(), read.sets, read.unchanged), ("x", 0, false));
    assert!(yolu_io::RecoveryInfo::from_bytes(b"{}").unwrap().title.is_empty());
    for bad in [&b"not json"[..], b"[1]", b"\"s\"", b""] {
        assert!(matches!(
            yolu_io::RecoveryInfo::from_bytes(bad),
            Err(StoreError::Corrupt(_))
        ));
    }
    assert!(matches!(
        yolu_io::RecoveryInfo::from_bytes(&vec![b' '; 65 * 1024]),
        Err(StoreError::Budget(_))
    ));
}

#[test]
fn a_project_survives_the_store_and_opens_from_its_entries() {
    use yolu_core::{BrushSettings, Document, Rgba8};
    use yolu_io::{composite_pngs, MaterialRef, NativeDocument, Project, SetSpec, WriterInfo};
    let writer = WriterInfo {
        app: "試験".into(),
        version: "0.0.1".into(),
        unity: "standalone".into(),
    };
    let mut doc = Document::new(64, 64).unwrap();
    let layer = doc.add_layer("絵").unwrap();
    let brush = BrushSettings {
        color: Rgba8::new(10, 200, 30, 255),
        radius: 6.0,
        ..BrushSettings::default()
    };
    let mut stroke = doc.begin_stroke(layer, &brush).unwrap();
    stroke
        .add_point(&mut doc, 20.0, 20.0, 1.0, Default::default())
        .unwrap();
    doc.end_stroke(stroke).unwrap();
    let id = format!("{:08x}-0000-0000-0000-000000000001", 7);
    let spec = SetSpec {
        id: id.clone(),
        name: "セット".into(),
        material: MaterialRef::PendingSlot(0),
        document: Some(NativeDocument::from_core(&doc).unwrap()),
        composites: composite_pngs(&doc).unwrap(),
    };
    let project = Project::create(writer, &[spec], &id).unwrap();
    let entries = project.original_archive().entries().clone();
    let dir = Dir::new();
    let store = GenerationStore::new(&dir.0);
    let mut with_info = entries.clone();
    with_info.insert(yolu_io::INFO_NAME.into(), bytes(b"{}"));
    store.commit(&with_info, &share(Some(3), None)).unwrap();
    let mut loaded = store.load().unwrap().files;
    loaded.remove(yolu_io::INFO_NAME);
    assert_eq!(loaded, entries);
    let reopened = Project::from_entries(loaded).unwrap();
    assert_eq!(reopened.info().format, project.info().format);
    assert_eq!(reopened.current_set(), id);
    assert_eq!(reopened.sets().len(), 1);
    assert_eq!(
        reopened.sets()[0].document.to_bytes(),
        project.sets()[0].document.to_bytes()
    );
    // 世代の名前に使えないエントリ（`.ylp` の名前の範囲の外）を含む Project は作れない
    let mut bad = entries.clone();
    bad.insert("brushes/../x".into(), bytes(b"x"));
    assert!(Project::from_entries(bad).is_err());
}
