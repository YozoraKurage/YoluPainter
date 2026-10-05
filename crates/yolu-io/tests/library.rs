//! 個人のライブラリのフォルダ（`yolu_io::library`）: 一覧・安全な相対パス・読み・書き（同じ中身は 1 つ・名前の衝突・取り消し）・消す。
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use yolu_io::library::{
    add, add_with, hash_file, is_library_path, list, read, remove, resolve, safe_stem, sha256_hex,
    Faults, Kind, SkipReason, MAX_COMPONENT_UNITS, MAX_DEPTH, MAX_ENTRIES, MAX_STEM_BYTES,
    MAX_STEM_CHARS,
};
use yolu_io::Error;

static NEXT: AtomicU32 = AtomicU32::new(0);

/// 試験ごとの使い捨てのフォルダ（終わると消す）。
struct Dir(PathBuf);

impl Dir {
    fn new() -> Dir {
        let path = std::env::temp_dir().join(format!(
            "yolu-library-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Dir(path)
    }
    fn root(&self) -> &Path {
        &self.0
    }
    fn put(&self, rel: &str, bytes: &[u8]) {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn names(dir: &Dir) -> Vec<String> {
    list(dir.root(), None)
        .unwrap()
        .entries
        .into_iter()
        .map(|e| e.rel)
        .collect()
}

fn is_cancelled(e: &Error) -> bool {
    matches!(e, Error::Core(yolu_core::CoreError::Cancelled))
}

#[test]
fn an_absent_folder_is_an_empty_library_and_a_file_is_refused() {
    let dir = Dir::new();
    let missing = dir.root().join("nothing");
    let listing = list(&missing, None).unwrap();
    assert!(listing.entries.is_empty() && listing.skipped.is_empty() && !listing.truncated);
    dir.put("plain.png", b"x");
    assert!(list(&dir.root().join("plain.png"), None).is_err());
}

#[test]
fn the_listing_walks_folders_in_a_stable_name_order_and_names_each_kind() {
    let dir = Dir::new();
    dir.put("b.png", b"1");
    dir.put("A.png", b"2");
    dir.put("Smart/Rusty.ylsmart", b"3");
    dir.put("Smart/deep/Old.YLSMART", b"4");
    dir.put("Brushes/Soft.ylbrush", b"5");
    dir.put("M.ylmaterial", b"6");
    dir.put("notes.txt", b"ignored");
    dir.put("photo.jpg", b"ignored");
    dir.put(".hidden.png", b"ignored");
    dir.put("half.png.tmp~", b"ignored");
    dir.put(".trash/inside.png", b"ignored");
    dir.put("tail~/inside.png", b"ignored");
    let listing = list(dir.root(), None).unwrap();
    let got: Vec<_> = listing
        .entries
        .iter()
        .map(|e| (e.rel.as_str(), e.kind, e.len))
        .collect();
    assert_eq!(
        got,
        vec![
            ("A.png", Kind::Image, 1),
            ("b.png", Kind::Image, 1),
            ("Brushes/Soft.ylbrush", Kind::Brush, 1),
            ("M.ylmaterial", Kind::Material, 1),
            ("Smart/deep/Old.YLSMART", Kind::Smart, 1),
            ("Smart/Rusty.ylsmart", Kind::Smart, 1),
        ]
    );
    assert!(listing.skipped.is_empty() && !listing.truncated);
    assert_eq!(listing.entries[0].name(), "A");
    assert_eq!(listing.entries[4].name(), "Old");
}

#[test]
fn a_listing_stops_at_the_entry_limit_and_at_the_depth_limit_and_says_so() {
    let dir = Dir::new();
    let mut deep = String::new();
    for i in 0..=MAX_DEPTH {
        deep.push_str(&format!("d{i}/"));
    }
    dir.put(&format!("{deep}too-deep.png"), b"x");
    dir.put("shallow.png", b"x");
    let listing = list(dir.root(), None).unwrap();
    assert_eq!(names(&dir), vec!["shallow.png".to_string()]);
    assert!(listing
        .skipped
        .iter()
        .any(|s| s.reason == SkipReason::Deep && s.rel.ends_with(&format!("d{MAX_DEPTH}"))));
    // 深さの上限ちょうどのフォルダの中のファイルは一覧に出る
    let ok = Dir::new();
    let mut at_limit = String::new();
    for i in 0..MAX_DEPTH {
        at_limit.push_str(&format!("d{i}/"));
    }
    ok.put(&format!("{at_limit}fits.png"), b"x");
    assert_eq!(names(&ok).len(), 1);

    let many = Dir::new();
    for i in 0..=MAX_ENTRIES {
        many.put(&format!("{i:05}.png"), b"x");
    }
    let listing = list(many.root(), None).unwrap();
    assert_eq!(listing.entries.len(), MAX_ENTRIES);
    assert!(listing.truncated);
    // いつ読んでも同じ所で打ち切る
    let again = list(many.root(), None).unwrap();
    assert_eq!(
        listing.entries.last().unwrap().rel,
        again.entries.last().unwrap().rel
    );
}

#[test]
fn a_cancelled_listing_stops_without_a_result() {
    let dir = Dir::new();
    dir.put("a.png", b"x");
    let flag = AtomicBool::new(true);
    assert!(is_cancelled(&list(dir.root(), Some(&flag)).unwrap_err()));
}

#[cfg(unix)]
#[test]
fn links_are_never_followed_for_listing_reading_writing_or_removing() {
    use std::os::unix::fs::symlink;
    let outside = Dir::new();
    outside.put("secret.png", b"outside");
    outside.put("inner/secret2.png", b"outside");
    let dir = Dir::new();
    dir.put("real.png", b"real");
    symlink(
        outside.root().join("secret.png"),
        dir.root().join("file-link.png"),
    )
    .unwrap();
    symlink(outside.root().join("inner"), dir.root().join("folder-link")).unwrap();
    let listing = list(dir.root(), None).unwrap();
    assert_eq!(
        listing
            .entries
            .iter()
            .map(|e| e.rel.as_str())
            .collect::<Vec<_>>(),
        vec!["real.png"]
    );
    let mut skipped: Vec<_> = listing
        .skipped
        .iter()
        .map(|s| (s.rel.as_str(), s.reason.clone()))
        .collect();
    skipped.sort_by_key(|s| s.0);
    assert_eq!(
        skipped,
        vec![
            ("file-link.png", SkipReason::Link),
            ("folder-link", SkipReason::Link)
        ]
    );
    for rel in ["file-link.png", "folder-link/secret2.png"] {
        assert!(resolve(dir.root(), rel).is_err(), "{rel}");
        assert!(read(dir.root(), rel, 1 << 20, None).is_err(), "{rel}");
        assert!(remove(dir.root(), rel).is_err(), "{rel}");
    }
    assert!(add(dir.root(), "folder-link", "x", Kind::Image, b"data", None).is_err());
    assert!(!outside.root().join("inner/x.png").exists());
    assert_eq!(
        std::fs::read(outside.root().join("secret.png")).unwrap(),
        b"outside"
    );
    // 根そのものがシンボリックリンクなら、一覧も書き込みも断る
    let root_link = outside.root().join("root-link");
    symlink(dir.root(), &root_link).unwrap();
    assert!(list(&root_link, None).is_err());
    assert!(add(&root_link, "", "x", Kind::Image, b"data", None).is_err());
    assert!(read(&root_link, "real.png", 1 << 20, None).is_err());
}

#[test]
fn unsafe_relative_paths_are_refused_everywhere() {
    let dir = Dir::new();
    dir.put("ok.png", b"ok");
    for bad in [
        "../x.png",
        "/etc/passwd",
        "a/../b.png",
        "a\\b.png",
        "C:x.png",
        "",
    ] {
        assert!(!is_library_path(bad));
        assert!(resolve(dir.root(), bad).is_err(), "{bad}");
        assert!(read(dir.root(), bad, 1 << 20, None).is_err(), "{bad}");
        assert!(remove(dir.root(), bad).is_err(), "{bad}");
        if !bad.is_empty() {
            assert!(
                add(dir.root(), bad, "x", Kind::Image, b"data", None).is_err(),
                "{bad}"
            );
        }
    }
    assert!(resolve(dir.root(), "ok.png").is_ok());
    assert_eq!(std::fs::read(dir.root().join("ok.png")).unwrap(), b"ok");
}

#[test]
fn reading_refuses_a_file_over_the_limit_before_reading_and_honours_the_flag() {
    let dir = Dir::new();
    dir.put("big.ylsmart", &[7u8; 100]);
    assert_eq!(
        read(dir.root(), "big.ylsmart", 100, None).unwrap().len(),
        100
    );
    // 1 バイト足りない上限
    let error = read(dir.root(), "big.ylsmart", 99, None).unwrap_err();
    assert!(matches!(error, Error::Budget(_)), "{error}");
    let error = hash_file(dir.root(), "big.ylsmart", 99, None).unwrap_err();
    assert!(matches!(error, Error::Budget(_)), "{error}");
    let flag = AtomicBool::new(true);
    assert!(is_cancelled(
        &read(dir.root(), "big.ylsmart", 100, Some(&flag)).unwrap_err()
    ));
    assert!(is_cancelled(
        &hash_file(dir.root(), "big.ylsmart", 100, Some(&flag)).unwrap_err()
    ));
    // フォルダと無いファイルは読まない
    dir.put("folder/inner.png", b"x");
    assert!(read(dir.root(), "folder", 100, None).is_err());
    assert!(read(dir.root(), "missing.png", 100, None).is_err());
}

#[test]
fn the_streamed_hash_matches_the_hash_of_the_bytes() {
    let dir = Dir::new();
    let bytes: Vec<u8> = (0..3_000_000u32).map(|i| (i * 31 % 251) as u8).collect();
    dir.put("a.png", &bytes);
    let (hash, len) = hash_file(dir.root(), "a.png", u64::MAX, None).unwrap();
    assert_eq!(hash, sha256_hex(&bytes));
    assert_eq!(len, bytes.len() as u64);
    assert_eq!(read(dir.root(), "a.png", u64::MAX, None).unwrap(), bytes);
}

#[test]
fn adding_puts_a_kind_into_its_folder_and_the_same_bytes_are_kept_once() {
    let dir = Dir::new();
    let first = add(dir.root(), "Images", "Rust", Kind::Image, b"AAAA", None).unwrap();
    assert_eq!(first.rel, "Images/Rust.png");
    assert!(!first.existed);
    assert_eq!(first.sha256, sha256_hex(b"AAAA"));
    assert_eq!(first.len, 4);
    assert_eq!(
        std::fs::read(dir.root().join("Images/Rust.png")).unwrap(),
        b"AAAA"
    );
    // 同じバイト列は、名前が違っても、別のフォルダでも、書かずに今あるものを返す
    let same = add(
        dir.root(),
        "Images",
        "Other name",
        Kind::Image,
        b"AAAA",
        None,
    )
    .unwrap();
    assert_eq!((same.rel.as_str(), same.existed), ("Images/Rust.png", true));
    let elsewhere = add(dir.root(), "", "Top", Kind::Image, b"AAAA", None).unwrap();
    assert_eq!(
        (elsewhere.rel.as_str(), elsewhere.existed),
        ("Images/Rust.png", true)
    );
    assert_eq!(names(&dir), vec!["Images/Rust.png".to_string()]);
    // 同じバイト列でも種類が違えば別のファイル（スマートとマテリアルは取り違えない）
    let material = add(
        dir.root(),
        "Materials",
        "Rust",
        Kind::Material,
        b"AAAA",
        None,
    )
    .unwrap();
    assert_eq!(material.rel, "Materials/Rust.ylmaterial");
    assert!(!material.existed);
    // 空のバイト列は入れない
    assert!(add(dir.root(), "", "empty", Kind::Image, b"", None).is_err());
}

#[test]
fn a_taken_name_gets_a_number_and_the_taken_file_is_never_replaced() {
    let dir = Dir::new();
    dir.put("Images/x.png", b"mine");
    let a = add(dir.root(), "Images", "x", Kind::Image, b"one", None).unwrap();
    let b = add(dir.root(), "Images", "x", Kind::Image, b"two", None).unwrap();
    assert_eq!(a.rel, "Images/x 2.png");
    assert_eq!(b.rel, "Images/x 3.png");
    assert_eq!(
        std::fs::read(dir.root().join("Images/x.png")).unwrap(),
        b"mine"
    );
    // 大文字小文字だけが違う名前も重ねない（大文字小文字を区別しない環境へ持って行っても別々のファイルのまま）
    dir.put("Images/Case.png", b"c");
    let c = add(dir.root(), "Images", "case", Kind::Image, b"d", None).unwrap();
    assert_eq!(c.rel, "Images/case 2.png");
    // 名前に使えない文字・予約名
    let weird = add(dir.root(), "", "a/b:c", Kind::Image, b"e", None).unwrap();
    assert_eq!(weird.rel, "a_b_c.png");
    let reserved = add(dir.root(), "", "NUL", Kind::Image, b"f", None).unwrap();
    assert_eq!(reserved.rel, "_NUL.png");
    // 書き込みの一時ファイルは残らない
    let leftovers: Vec<_> = std::fs::read_dir(dir.root().join("Images"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with('~'))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn an_add_cancelled_before_it_starts_writes_nothing() {
    let dir = Dir::new();
    dir.put("keep.png", b"keep");
    let flag = AtomicBool::new(true);
    let error = add(
        dir.root(),
        "Images",
        "x",
        Kind::Image,
        &vec![1u8; 3 << 20],
        Some(&flag),
    )
    .unwrap_err();
    assert!(is_cancelled(&error), "{error}");
    assert!(!dir.root().join("Images/x.png").exists());
    let every: Vec<_> = walk(dir.root());
    assert_eq!(every, vec!["keep.png".to_string()], "{every:?}");
}

/// 一時ファイルの名前（`.…tmp~`）か。
fn is_temp(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.') && n.ends_with(".tmp~"))
}

#[test]
fn an_add_cancelled_while_it_writes_removes_the_half_written_temporary_file() {
    let dir = Dir::new();
    dir.put("keep.png", b"keep");
    let flag = AtomicBool::new(false);
    let seen = std::sync::Mutex::new(Vec::new());
    // 3 区切り（3 MiB）のうち、1 区切りを書いたところで旗を立てる（書きかけの一時ファイルが本当にある）
    let after_chunk = |path: &Path, written: usize| {
        let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        seen.lock().unwrap().push((is_temp(path), written, len));
        if written == 1 {
            flag.store(true, Ordering::SeqCst);
        }
    };
    let error = add_with(
        dir.root(),
        "Images",
        "x",
        Kind::Image,
        &vec![1u8; 3 << 20],
        Some(&flag),
        &Faults {
            after_chunk: Some(&after_chunk),
            ..Faults::default()
        },
    )
    .unwrap_err();
    assert!(is_cancelled(&error), "{error}");
    // 取り消しは次の区切りの前に効く（1 区切りだけ書いた一時ファイルがあった）
    assert_eq!(*seen.lock().unwrap(), vec![(true, 1, 1 << 20)]);
    assert!(!dir.root().join("Images/x.png").exists());
    let every = walk(dir.root());
    assert_eq!(every, vec!["keep.png".to_string()], "{every:?}");
    // 取り消しのあとも、同じ名前で入れ直せる
    let again = add(dir.root(), "Images", "x", Kind::Image, b"data", None).unwrap();
    assert_eq!(again.rel, "Images/x.png");
}

#[test]
fn an_add_cancelled_after_the_last_chunk_still_leaves_nothing() {
    let dir = Dir::new();
    let flag = AtomicBool::new(false);
    // 1 区切りに収まる小さな中身。書き終えたあと、名前を付ける前に旗が立つ
    let after_chunk = |_: &Path, _: usize| flag.store(true, Ordering::SeqCst);
    let error = add_with(
        dir.root(),
        "Images",
        "x",
        Kind::Image,
        b"small",
        Some(&flag),
        &Faults {
            after_chunk: Some(&after_chunk),
            ..Faults::default()
        },
    )
    .unwrap_err();
    assert!(is_cancelled(&error), "{error}");
    assert!(walk(dir.root()).is_empty(), "{:?}", walk(dir.root()));
}

#[test]
fn without_hard_links_the_name_is_still_given_without_replacing_anything() {
    let dir = Dir::new();
    dir.put("Images/x.png", b"mine");
    let faults = Faults {
        no_hard_link: true,
        ..Faults::default()
    };
    // 付け替えの枝（rename）でも、取られている名前は番号で避け、一時ファイルは残さない
    let a = add_with(
        dir.root(),
        "Images",
        "x",
        Kind::Image,
        b"one",
        None,
        &faults,
    )
    .unwrap();
    assert_eq!(a.rel, "Images/x 2.png");
    let b = add_with(
        dir.root(),
        "Images",
        "fresh",
        Kind::Image,
        b"two",
        None,
        &faults,
    )
    .unwrap();
    assert_eq!(b.rel, "Images/fresh.png");
    assert_eq!(
        std::fs::read(dir.root().join("Images/x.png")).unwrap(),
        b"mine"
    );
    assert_eq!(
        std::fs::read(dir.root().join("Images/x 2.png")).unwrap(),
        b"one"
    );
    assert_eq!(
        std::fs::read(dir.root().join("Images/fresh.png")).unwrap(),
        b"two"
    );
    let every = walk(dir.root());
    assert!(every.iter().all(|n| !n.ends_with('~')), "{every:?}");
    assert_eq!(every.len(), 3);
    // 同じ中身は、この枝でも書かない
    let same = add_with(
        dir.root(),
        "Images",
        "other",
        Kind::Image,
        b"two",
        None,
        &faults,
    )
    .unwrap();
    assert!(same.existed && same.rel == "Images/fresh.png");
}

fn walk(root: &Path) -> Vec<String> {
    fn go(dir: &Path, base: &Path, out: &mut Vec<String>) {
        let mut items: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        items.sort_by_key(|e| e.file_name());
        for e in items {
            let path = e.path();
            if path.is_dir() {
                go(&path, base, out);
            } else {
                out.push(
                    path.strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let mut out = Vec::new();
    go(root, root, &mut out);
    out
}

#[test]
fn removing_takes_one_ordinary_file_and_leaves_the_rest() {
    let dir = Dir::new();
    dir.put("Images/a.png", b"a");
    dir.put("Images/b.png", b"b");
    remove(dir.root(), "Images/a.png").unwrap();
    assert_eq!(walk(dir.root()), vec!["Images/b.png".to_string()]);
    // フォルダ・無いファイルは消さない
    assert!(remove(dir.root(), "Images").is_err());
    assert!(remove(dir.root(), "Images/zzz.png").is_err());
    assert_eq!(walk(dir.root()), vec!["Images/b.png".to_string()]);
}

#[test]
fn a_long_name_is_cut_to_what_the_file_system_holds_and_is_listed_and_found_again() {
    // 日本語（3 バイト）・絵文字（4 バイト）・ASCII の長い名前は、バイト数でも文字数でも上限に収まり、文字の途中で切れない
    for (name, chars) in [
        ("あ".repeat(300), MAX_STEM_BYTES / 3),
        ("😀".repeat(300), MAX_STEM_BYTES / 4),
        ("a".repeat(300), MAX_STEM_CHARS),
    ] {
        let dir = Dir::new();
        let stem = safe_stem(&name);
        assert_eq!(stem.chars().count(), chars.min(MAX_STEM_CHARS), "{name:.6}");
        assert!(stem.len() <= MAX_STEM_BYTES, "{}", stem.len());
        let bytes = [name.as_bytes(), b"-one"].concat();
        let first = add(dir.root(), "Images", &name, Kind::Image, &bytes, None)
            .unwrap_or_else(|e| panic!("{name:.6}: {e}"));
        assert!(!first.existed);
        // 書いたファイルは一覧に出て、読める（一覧と書き込みの上限が食い違わない）
        let listed = list(dir.root(), None).unwrap();
        assert!(listed.skipped.is_empty(), "{:?}", listed.skipped);
        assert_eq!(
            listed
                .entries
                .iter()
                .map(|e| e.rel.as_str())
                .collect::<Vec<_>>(),
            vec![first.rel.as_str()]
        );
        assert_eq!(read(dir.root(), &first.rel, 1 << 20, None).unwrap(), bytes);
        // 同じバイト列を入れ直しても、複製は増えない
        let again = add(dir.root(), "Images", &name, Kind::Image, &bytes, None).unwrap();
        assert!(again.existed);
        assert_eq!(again.rel, first.rel);
        assert_eq!(names(&dir).len(), 1);
        // 別の中身で同じ長い名前は、番号を付けても名前の長さに収まる
        let other = add(dir.root(), "Images", &name, Kind::Image, b"different", None).unwrap();
        assert!(other.rel.ends_with(" 2.png"), "{}", other.rel);
        assert_eq!(names(&dir).len(), 2);
    }
}

#[test]
fn a_component_may_be_as_long_as_the_longest_name_any_file_system_holds() {
    // NTFS・exFAT は 255 UTF-16 単位（日本語なら 255 文字 = 765 バイト）。バイトで数えて断らない
    assert!(is_library_path(&"あ".repeat(MAX_COMPONENT_UNITS)));
    assert!(!is_library_path(&"あ".repeat(MAX_COMPONENT_UNITS + 1)));
    // 絵文字は 2 単位
    assert!(is_library_path(&"😀".repeat(MAX_COMPONENT_UNITS / 2)));
    assert!(!is_library_path(&"😀".repeat(MAX_COMPONENT_UNITS / 2 + 1)));
    assert!(is_library_path(&format!(
        "{}/{}",
        "木".repeat(255),
        "a".repeat(255)
    )));
}

#[test]
fn safe_stems_are_what_the_adder_uses() {
    assert_eq!(safe_stem("  Gold Trim  "), "Gold Trim");
    assert_eq!(safe_stem("a*b?c"), "a_b_c");
}

// ───────── 棚の出どころ `library` ─────────

fn smart_fixture() -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smart/raster.ylsmart"))
        .unwrap()
}

#[test]
fn a_library_origin_is_written_to_the_index_and_read_back() {
    use yolu_io::shelf::{ResourceKind, Shelf};
    let bytes = smart_fixture();
    let sha = sha256_hex(&bytes);
    let mut shelf = Shelf::new(1 << 24);
    let id = shelf
        .add_file_from_library(
            "11111111-1111-4111-8111-111111111111",
            "試験素材",
            ResourceKind::SmartMaterial,
            &bytes,
            "Smart/raster.ylsmart",
            &sha,
        )
        .unwrap();
    let origin = &shelf.resources()[0].metadata["origin"];
    assert_eq!(origin["type"], "library");
    assert_eq!(origin["file"], "Smart/raster.ylsmart");
    assert_eq!(origin["sha256"], sha);
    assert_eq!(origin["length"], bytes.len());
    // 索引を書いて読み直しても、出どころは同じ（Unity 版が読む形）
    let back = Shelf::read(shelf.entries(), 1 << 24).unwrap();
    assert_eq!(back.resources()[0].metadata["origin"], *origin);
    assert_eq!(
        back.canonical_index().unwrap(),
        shelf.canonical_index().unwrap()
    );
    // 同じ中身は 1 つ（出どころが違っても、先にあるものを返す）
    let again = shelf
        .add_file_from_library(
            "22222222-2222-4222-8222-222222222222",
            "別の名前",
            ResourceKind::SmartMaterial,
            &bytes,
            "Other/copy.ylsmart",
            &sha,
        )
        .unwrap();
    assert_eq!(again, id);
    assert_eq!(shelf.resources().len(), 1);
}

#[test]
fn a_library_image_keeps_its_pixels_and_an_unsafe_origin_is_refused_without_changing_the_shelf() {
    use yolu_io::shelf::{image_hash, ResourceKind, Shelf};
    let mut shelf = Shelf::new(1 << 24);
    let rgba = [10u8, 20, 30, 0, 200, 100, 50, 255];
    let sha = sha256_hex(b"the file");
    let id = shelf
        .add_image_from_library(
            "33333333-3333-4333-8333-333333333333",
            "画像",
            &rgba,
            1,
            2,
            "unspecified",
            "Images/pic.png",
            &sha,
            8,
        )
        .unwrap();
    let res = &shelf.resources()[0];
    assert_eq!(res.id, id);
    assert_eq!(res.content, image_hash(&rgba, 1, 2).unwrap());
    assert_eq!(res.metadata["origin"]["length"], 8);
    assert_eq!(res.metadata["colorSpace"], "unspecified");
    let before = shelf.canonical_index().unwrap();
    for rel in ["../x.png", "/x.png", "a\\b.png", "a/../b.png", ""] {
        assert!(
            shelf
                .add_image_from_library(
                    "44444444-4444-4444-8444-444444444444",
                    "x",
                    &[1, 2, 3, 4],
                    1,
                    1,
                    "srgb",
                    rel,
                    &sha,
                    4
                )
                .is_err(),
            "{rel}"
        );
        assert!(
            shelf
                .add_file_from_library(
                    "55555555-5555-4555-8555-555555555555",
                    "x",
                    ResourceKind::SmartMaterial,
                    &smart_fixture(),
                    rel,
                    &sha
                )
                .is_err(),
            "{rel}"
        );
    }
    // SHA-256 が 64 桁の小文字 16 進でなければ断る
    for bad in ["", "abc", &"G".repeat(64), &"A".repeat(64)] {
        assert!(shelf
            .add_file_from_library(
                "66666666-6666-4666-8666-666666666666",
                "x",
                ResourceKind::SmartMaterial,
                &smart_fixture(),
                "Smart/x.ylsmart",
                bad
            )
            .is_err());
    }
    assert_eq!(shelf.canonical_index().unwrap(), before);
    assert_eq!(shelf.resources().len(), 1);
}
