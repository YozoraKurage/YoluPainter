//! 外側（`Package`）の試験: 今の形のバイト一致、`YLP-4`・zip64 の往復、上限と予算、ファイルの位置から流して読むこと。
use super::*;
use crate::Archive;

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
fn fake_png(len: usize) -> Vec<u8> {
    let mut b = vec![0u8; len];
    b[..8].copy_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    b
}
const SET: &str = "sets/0f8fad5b-d9cb-469f-a165-70867728950e";
fn sample() -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([
        (format!("{SET}/document.utpaint"), noise(3000, 1)),
        (format!("{SET}/composite/Color.png"), fake_png(2000)),
        ("project.json".into(), b"{}".to_vec()),
        ("empty.bin".into(), Vec::new()),
        ("notes.txt".into(), "compressible text ".repeat(300).into_bytes()),
    ])
}
fn package_of(files: &BTreeMap<String, Vec<u8>>, level: u32) -> Package {
    Package::build(
        files.iter().map(|(k, v)| (k.clone(), Blob::from(v.clone()))).collect(),
        level,
    )
    .unwrap()
}
/// 小さな閾値（今の形に収まらない扱いにして `YLP-4` で書く）。
fn small(classic_total: u64) -> Thresholds {
    Thresholds {
        classic_total_bytes: classic_total,
        ..Thresholds::REAL
    }
}
fn same_entries(a: &Files, b: &Files) {
    assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>());
    for (k, v) in a {
        assert_eq!(v.bytes().unwrap(), b[k].bytes().unwrap(), "{k}");
    }
}

#[test]
fn the_classic_form_is_byte_identical_to_the_archive_writer() {
    let mut cases = vec![sample()];
    // 1000 エントリ・一番長い名前・縮まない中身
    let mut many = BTreeMap::from([("document.utpaint".to_owned(), noise(50, 9))]);
    for i in 0..999 {
        many.insert(format!("x{i:03}-{}", "a".repeat(80)), noise(i % 7, i as u64));
    }
    cases.push(many);
    for files in &cases {
        let old = Archive::from_entries(files.clone()).unwrap().to_bytes().unwrap();
        let new = package_of(files, 3).to_bytes().unwrap();
        assert_eq!(new, old);
    }
    // 古い版の manifest（YLP-1・2）も、その版のまま同じバイト列
    let flat = BTreeMap::from([
        ("document.utpaint".to_owned(), noise(500, 3)),
        ("composite/Color.png".to_owned(), fake_png(300)),
    ]);
    for level in [1, 2] {
        let old = Archive::build(
            flat.iter().map(|(k, v)| (k.clone(), Arc::<[u8]>::from(v.clone()))).collect(),
            level,
            YLP_MIME,
            YLP_PREFIX,
        )
        .unwrap()
        .to_bytes()
        .unwrap();
        assert_eq!(package_of(&flat, level).to_bytes().unwrap(), old, "YLP-{level}");
    }
}

#[test]
fn reading_and_writing_back_a_classic_file_keeps_every_byte() {
    let bytes = Archive::from_entries(sample()).unwrap().to_bytes().unwrap();
    let p = Package::read_bytes(&bytes, &Limits::default()).unwrap();
    assert_eq!(p.manifest_version(), 3);
    assert_eq!(p.to_bytes().unwrap(), bytes);
}

/// ファイルから流した中身（小さく分けて書く）を deflate しても、メモリから 1 度に書いたのと同じバイト列になる（今の形のバイト一致の前提）。
#[test]
fn deflating_a_streamed_entry_gives_the_same_bytes_as_one_write() {
    let dir = tempdir();
    let mut files = sample();
    files.insert("big.bin".into(), {
        let mut v = "repetitive ".repeat(40_000).into_bytes();
        v.extend(noise(300_000, 4));
        v
    });
    let bytes = package_of(&files, 3).to_bytes().unwrap();
    let path = dir.join("a.ylp");
    std::fs::write(&path, &bytes).unwrap();
    // 全部をファイルの位置で持つ（メモリに残さない）
    let p = read(&Source::open_file(&path).unwrap(), &Limits::default(), 0).unwrap();
    assert!(p.files.values().filter(|b| !b.is_empty()).all(|b| b.in_memory().is_none()));
    assert_eq!(p.to_bytes().unwrap(), bytes);
}

#[test]
fn a_project_beyond_the_classic_limits_is_written_as_ylp_4_and_old_readers_refuse_it() {
    let files = sample();
    let bytes = small(1000).scoped(|| package_of(&files, 3).to_bytes().unwrap());
    let p = Package::read_bytes(&bytes, &Limits::default()).unwrap();
    assert_eq!(p.manifest_version(), 4);
    assert!(p.read_manifest().unwrap().starts_with(b"YOLUPAINTER-YLP-4\n"));
    same_entries(&p.files, &package_of(&files, 3).files);
    // 今の読み手（`Archive::read`）は manifest の版で断る
    let refused = Archive::read(&bytes).unwrap_err().to_string();
    assert!(refused.contains("YOLUPAINTER-YLP-4") && refused.contains("未対応"), "{refused}");
    // 書き戻すと、本物の閾値なら今の形に戻る（収まるので）
    let back = p.to_bytes().unwrap();
    assert_eq!(back, package_of(&files, 3).to_bytes().unwrap());
}

#[test]
fn zip64_offsets_and_end_records_are_written_only_when_needed_and_read_back() {
    let files = sample();
    // 位置の zip64（中央ディレクトリから拡張で指す）と、数の zip64（終端の記録）
    for (offset_at, count_above) in [(200, 0xFFFF), (0xFFFF_FFFF, 3), (100, 2)] {
        let t = Thresholds {
            zip64_offset_at: offset_at,
            zip64_count_above: count_above,
            ..small(1000)
        };
        let bytes = t.scoped(|| package_of(&files, 3).to_bytes().unwrap());
        let has_record = bytes.windows(4).any(|w| w == b"PK\x06\x06");
        assert_eq!(has_record, count_above < 0xFFFF || offset_at < 0xFFFF_FFFF && bytes.len() as u64 > offset_at);
        let p = Package::read_bytes(&bytes, &Limits::default()).unwrap();
        same_entries(&p.files, &package_of(&files, 3).files);
        assert!(Archive::read(&bytes).is_err());
    }
    // 今の形には zip64 を使わない（閾値を下げても）
    let t = Thresholds {
        zip64_offset_at: 10,
        zip64_count_above: 1,
        ..Thresholds::REAL
    };
    let bytes = t.scoped(|| package_of(&files, 3).to_bytes().unwrap());
    assert_eq!(bytes, Archive::from_entries(files).unwrap().to_bytes().unwrap());
}

#[test]
fn a_classic_manifest_with_zip64_structures_is_refused() {
    let files = sample();
    let p = package_of(&files, 3);
    // zip の構造は zip64（位置を拡張で指し、終端の記録もある）、manifest の版だけが 3
    let t = Thresholds {
        zip64_offset_at: 100,
        zip64_count_above: 2,
        ..Thresholds::REAL
    };
    let mut plan = p.plan().unwrap();
    plan.thresholds = t;
    plan.level = 4;
    let mut out = Cursor::new(Vec::new());
    p.write_with(&plan, &mut out).unwrap();
    let bytes = out.into_inner();
    assert!(bytes.windows(4).any(|w| w == b"PK\x06\x06"));
    let e = Package::read_bytes(&bytes, &Limits::default()).unwrap_err();
    assert!(e.to_string().contains("ZIP64"), "{e}");
    // 同じ構造で manifest の版が 4 なら読める
    let mut plan = small(1000).scoped(|| p.plan().unwrap());
    plan.thresholds = t;
    let mut out = Cursor::new(Vec::new());
    p.write_with(&plan, &mut out).unwrap();
    Package::read_bytes(&out.into_inner(), &Limits::default()).unwrap();
}

#[test]
fn ylp_4_limits_come_from_the_layer_pixel_budget() {
    let mut files = sample();
    files.insert(format!("{SET}/document.utpaint.1"), noise(4000, 5));
    let bytes = small(1000).scoped(|| package_of(&files, 3).to_bytes().unwrap());
    // 正本（ヘッダー 3000 ＋ 部分 4000）が予算の 4 倍を超えれば断る（どの予算かを言う）
    let tight = Limits {
        document_bytes: 6999,
        other_bytes: OTHER_BYTES,
    };
    let e = Package::read_bytes(&bytes, &tight).unwrap_err();
    assert!(matches!(e, Error::Budget(_)), "{e:?}");
    assert!(e.to_string().contains("レイヤーのメモリ"), "{e}");
    let enough = Limits {
        document_bytes: 7000,
        other_bytes: OTHER_BYTES,
    };
    Package::read_bytes(&bytes, &enough).unwrap();
    // 全体: 正本のあるセットの数 × 正本の予算 ＋ ほか
    let others: u64 = files
        .iter()
        .filter(|(k, _)| document_owner(k).is_none())
        .map(|(_, v)| v.len() as u64)
        .sum();
    let total_tight = Limits {
        document_bytes: 7000,
        other_bytes: others - 1,
    };
    let e = Package::read_bytes(&bytes, &total_tight).unwrap_err();
    assert!(matches!(e, Error::Budget(_)), "{e:?}");
    // 1 つの部分は 256 MiB まで（宣言だけで、展開の前に断る）
    assert_eq!(entry_limit(&format!("{SET}/document.utpaint.7")), MAX_PART_BYTES);
    assert_eq!(entry_limit(&format!("{SET}/document.utpaint")), MAX_ONE_ENTRY);
    // 予算の 4 倍・既定の下限
    assert_eq!(Limits::from_layer_pixels(1).document_bytes, 4 * (256 << 20));
    assert_eq!(Limits::from_layer_pixels(2048 << 20).document_bytes, 4 * (2048 << 20));
}

#[test]
fn ylp_4_limits_hold_at_the_largest_layer_memory_without_overflow() {
    // 設定の最大（レイヤーのメモリ 65536 MiB = 64 GiB）でも、4 倍は桁あふれせず、境目の 1 バイトで断る
    let top = Limits::from_layer_pixels(65536 << 20);
    assert_eq!(top.document_bytes, 4 * (65536u64 << 20));
    let document = format!("{SET}/document.utpaint");
    top.check([(document.as_str(), top.document_bytes)]).unwrap();
    let e = top.check([(document.as_str(), top.document_bytes + 1)]).unwrap_err();
    assert!(matches!(e, Error::Budget(_)), "{e:?}");
    // 全体はセットの数 × 正本の上限（2 つ目のセットも同じだけ読める）＋ほか
    let other = "sets/1a8fad5b-d9cb-469f-a165-70867728950f/document.utpaint".to_owned();
    top.check([(document.as_str(), top.document_bytes), (other.as_str(), top.document_bytes)]).unwrap();
    // 予算が u64 の端でも掛け算で壊れない（飽和して、断る理由が出る代わりに何でも通る）
    let huge = Limits::from_layer_pixels(u64::MAX);
    assert_eq!(huge.document_bytes, u64::MAX);
    huge.check([(document.as_str(), u64::MAX), (other.as_str(), u64::MAX)]).unwrap();
}

#[test]
fn part_names_follow_the_rules() {
    for (name, n) in [
        ("document.utpaint.1", Some(1)),
        ("sets/0f8fad5b-d9cb-469f-a165-70867728950e/document.utpaint.12", Some(12)),
        ("document.utpaint.0", None),
        ("document.utpaint.01", None),
        ("document.utpaint.", None),
        ("document.utpaint.1a", None),
        ("document.utpaint.1234567890", None),
        ("document.utpaint", None),
        ("composite/document.utpaint.1", None),
    ] {
        assert_eq!(part_number(name), n, "{name}");
    }
}

#[test]
fn a_damaged_entry_is_found_when_streamed_after_opening() {
    let dir = tempdir();
    let files = sample();
    let bytes = package_of(&files, 3).to_bytes().unwrap();
    let path = dir.join("b.ylp");
    std::fs::write(&path, &bytes).unwrap();
    let p = read(&Source::open_file(&path).unwrap(), &Limits::default(), 0).unwrap();
    // 開いた後に外で中身を壊す（無圧縮の PNG の中の 1 バイト）
    let png = bytes.windows(8).position(|w| w == [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]).unwrap();
    let mut damaged = bytes.clone();
    damaged[png + 100] ^= 1;
    std::fs::write(&path, &damaged).unwrap();
    let blob = &p.files[&format!("{SET}/composite/Color.png")];
    let e = blob.bytes().unwrap_err();
    assert!(matches!(e, Error::InvalidData(_)), "{e:?}");
}

/// 開いたファイルのハンドルは、外で改名・削除・別のファイルへの置き換えをされても、開いたときの中身を読む。置き場の名前の付け替え
/// （保存で置き換えた後の名前）は、エントリを作り直さない（同じエントリのまま）。
#[test]
fn an_opened_file_is_read_through_its_handle_not_its_name() {
    let dir = tempdir();
    let files = sample();
    let bytes = package_of(&files, 3).to_bytes().unwrap();
    let path = dir.join("h.ylp");
    std::fs::write(&path, &bytes).unwrap();
    let p = read(&Source::open_file(&path).unwrap(), &Limits::default(), 0).unwrap();
    let name = format!("{SET}/composite/Color.png");
    let blob = &p.files[&name];
    assert!(blob.in_memory().is_none());
    // 動かす・別のファイルを同じ名前へ置き換える・消す（どれも、開いたときの中身を読む）
    let moved = dir.join("moved.ylp");
    std::fs::rename(&path, &moved).unwrap();
    assert_eq!(&blob.bytes().unwrap()[..], &files[&name][..]);
    std::fs::write(dir.join("other.ylp"), b"not a ylp at all").unwrap();
    std::fs::rename(dir.join("other.ylp"), &path).unwrap();
    assert_eq!(&blob.bytes().unwrap()[..], &files[&name][..]);
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_file(&moved).unwrap();
    assert_eq!(&blob.bytes().unwrap()[..], &files[&name][..]);
    assert_eq!(p.to_bytes().unwrap(), bytes, "消えたあとでも、全部を同じバイト列で書き直せる");
    // 置き場の名前の付け替えは、エントリを作り直さない
    let before = p.files[&name].clone();
    p.note_path(&dir.join("c.ylp"));
    assert!(p.files[&name].same(&before));
    assert_eq!(&p.files[&name].bytes().unwrap()[..], &files[&name][..]);
}

/// 全部のエントリが小さい .ylp は、開いたあとファイルを持たない（Windows で、開いている間の改名・削除・上書きを妨げない。ハンドルを
/// 持つのは、メモリに残さない大きなエントリがあるときだけ）。
#[test]
fn a_package_of_only_small_entries_does_not_keep_its_file_open() {
    use crate::package::release_at;
    let dir = tempdir();
    let files = sample();
    let path = dir.join("small.ylp");
    std::fs::write(&path, package_of(&files, 3).to_bytes().unwrap()).unwrap();
    // 全部が小さい: 全部メモリに残り、手放す相手（開いたままのハンドル）がいない
    let p = Package::open(&path, &Limits::default()).unwrap();
    assert!(p.files.values().all(|b| b.in_memory().is_some()));
    assert!(release_at(&path).is_empty(), "全部が小さい .ylp はハンドルを持たない");
    // 比べる相手: 大きなエントリがあれば（メモリに残す大きさを下げて）、ハンドルを持つ
    let big = Thresholds { keep_in_memory: 1000, ..Thresholds::REAL }
        .scoped(|| Package::open(&path, &Limits::default()).unwrap());
    assert!(big.files.values().any(|b| b.in_memory().is_none()));
    let released = release_at(&path);
    assert!(!released.is_empty(), "大きなエントリを持つ .ylp は開いたハンドルを持つ");
    released.reacquire();
    drop(big);
    assert!(release_at(&path).is_empty(), "持ち主が手放せば、ハンドルも閉じる");
}

/// 同じファイルを何本が同時に読んでも、互いの読む位置を動かさない（ファイルのカーソルを使わず、位置を指定して読む）。
#[test]
fn readers_of_one_handle_do_not_move_each_other() {
    let dir = tempdir();
    let mut files = sample();
    files.insert("a.bin".into(), noise(300_000, 21));
    files.insert("b.bin".into(), noise(300_000, 22));
    let bytes = package_of(&files, 3).to_bytes().unwrap();
    let path = dir.join("two.ylp");
    std::fs::write(&path, &bytes).unwrap();
    let p = read(&Source::open_file(&path).unwrap(), &Limits::default(), 0).unwrap();
    let mut a = p.files["a.bin"].reader().unwrap();
    let mut b = p.files["b.bin"].reader().unwrap();
    let (mut got_a, mut got_b) = (Vec::<u8>::new(), Vec::<u8>::new());
    let mut chunk = [0u8; 4096];
    loop {
        let n = a.read(&mut chunk).unwrap();
        got_a.extend(&chunk[..n]);
        let m = b.read(&mut chunk).unwrap();
        got_b.extend(&chunk[..m]);
        if n == 0 && m == 0 {
            break;
        }
    }
    assert_eq!(got_a, files["a.bin"]);
    assert_eq!(got_b, files["b.bin"]);
}

/// 置換のために手放した（置換の規則が POSIX でないファイルシステム）ハンドルは、置換に失敗したら掴み直し、置換できたら断る。
#[test]
fn a_released_handle_is_taken_again_if_the_replace_fails_and_refused_if_it_succeeds() {
    use crate::package::release_at;
    let dir = tempdir();
    let files = sample();
    let bytes = package_of(&files, 3).to_bytes().unwrap();
    let path = dir.join("r.ylp");
    std::fs::write(&path, &bytes).unwrap();
    let p = read(&Source::open_file(&path).unwrap(), &Limits::default(), 0).unwrap();
    let name = format!("{SET}/composite/Color.png");
    // 関係の無い名前は手放さない
    assert!(release_at(&dir.join("elsewhere.ylp")).is_empty());
    assert!(p.files[&name].bytes().is_ok());
    // 手放すと読めない（読もうとすると、手放したと断る）。置換に失敗したら元のファイルが残っているので、掴み直して読める
    let released = release_at(&path);
    assert!(!released.is_empty());
    let refused = p.files[&name].bytes().unwrap_err();
    assert!(refused.to_string().contains(crate::SOURCE_RELEASED), "{refused:?}");
    released.reacquire();
    assert_eq!(&p.files[&name].bytes().unwrap()[..], &files[&name][..]);
    // 置換できた（別のファイルが同じ名前にある）なら、掴み直さない
    let released = release_at(&path);
    std::fs::write(dir.join("new.ylp"), b"replaced").unwrap();
    std::fs::rename(dir.join("new.ylp"), &path).unwrap();
    released.reacquire();
    let refused = p.files[&name].bytes().unwrap_err();
    assert!(refused.to_string().contains(crate::SOURCE_RELEASED), "{refused:?}");
}

#[test]
fn a_held_entry_outlives_its_old_file_and_its_folder_goes_with_the_last_holder() {
    struct Owner(PathBuf);
    impl Drop for Owner {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = tempdir();
    let files = sample();
    let bytes = package_of(&files, 3).to_bytes().unwrap();
    let path = dir.join("held.ylp");
    std::fs::write(&path, &bytes).unwrap();
    let p = read(&Source::open_file(&path).unwrap(), &Limits::default(), 0).unwrap();
    // 置き場の中身のファイル（復旧の世代）は、ハードリンクで置く
    let store = dir.join("contents");
    std::fs::create_dir_all(&store).unwrap();
    let name = format!("{SET}/document.utpaint");
    let content = &files[&name];
    let stored = store.join(format!("{}.bin", hash(content)));
    std::fs::write(&stored, content).unwrap();
    let file = Blob::file(stored.clone(), &name, content.len() as u64, hash(content));
    let held_dir = dir.join("held");
    std::fs::create_dir_all(&held_dir).unwrap();
    let keep: Keep = Arc::new(Owner(held_dir.clone()));
    let held_file = file.hold_in(&held_dir, &keep).unwrap();
    // .ylp の中の位置のエントリは流して写す（確かめながら）。メモリの中身はそのまま
    let zipped = &p.files[&format!("{SET}/composite/Color.png")];
    let held_zip = zipped.hold_in(&held_dir, &keep).unwrap();
    let memory = Blob::from(vec![1u8, 2, 3]);
    assert!(memory.hold_in(&held_dir, &keep).unwrap().same(&memory));
    drop(keep);
    // 元が消えても読める（世代の整理・破棄、.ylp を動かした後）
    std::fs::remove_file(&stored).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(&held_file.bytes().unwrap()[..], &content[..]);
    assert_eq!(&held_zip.bytes().unwrap()[..], &files[&format!("{SET}/composite/Color.png")][..]);
    assert_eq!(held_file.sha256().unwrap(), hash(content));
    // 読み手が残っている間は、エントリを手放しても片付けない
    let reader = held_file.reader().unwrap();
    drop((held_file, held_zip));
    assert!(held_dir.is_dir());
    drop(reader);
    assert!(!held_dir.exists(), "最後の持ち主が手放すと片付く");
}

#[test]
fn blobs_compare_by_content() {
    let a = Blob::from(vec![1, 2, 3]);
    assert_eq!(a, Blob::from(&[1u8, 2, 3][..]));
    assert_ne!(a, Blob::from(vec![1, 2]));
    assert_eq!(a.sha256().unwrap(), hash(&[1, 2, 3]));
    assert_eq!(a.len(), 3);
}

/// 試験の一時フォルダ（落とすと消える）。
struct TempDir(PathBuf);
impl std::ops::Deref for TempDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn tempdir() -> TempDir {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "yolu-package-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    TempDir(dir)
}

/// 外の zip の読み手（Python の zipfile・7-Zip など）で確かめるための見本を `YOLU_DUMP` のフォルダへ書く（手で回す）。
#[test]
#[ignore]
fn dump_zip64_samples_for_an_external_reader() {
    let Some(dir) = std::env::var_os("YOLU_DUMP") else { return };
    let files = sample();
    for (name, t) in [
        ("classic.ylp", Thresholds::REAL),
        ("ylp4.ylp", small(1000)),
        (
            "ylp4-zip64.ylp",
            Thresholds {
                zip64_offset_at: 100,
                zip64_count_above: 2,
                ..small(1000)
            },
        ),
    ] {
        let bytes = t.scoped(|| package_of(&files, 3).to_bytes().unwrap());
        std::fs::write(Path::new(&dir).join(name), bytes).unwrap();
    }
}

#[test]
fn ylp_4_copies_compressed_bytes_of_unchanged_entries_and_stores_incompressible_ones() {
    let dir = tempdir();
    let mut files = sample();
    // 4 MiB を超える、縮まないものと縮むもの
    files.insert("noise.bin".into(), noise(5 << 20, 11));
    files.insert("calm.bin".into(), "abcdefgh".repeat(700_000).into_bytes());
    let bytes = small(1000).scoped(|| package_of(&files, 3).to_bytes().unwrap());
    let method = |bytes: &[u8], name: &str| -> u16 {
        let at = bytes
            .windows(name.len() + 30)
            .position(|w| w.starts_with(b"PK\x03\x04") && &w[30..] == name.as_bytes())
            .unwrap();
        u16::from_le_bytes([bytes[at + 8], bytes[at + 9]])
    };
    assert_eq!(method(&bytes, "noise.bin"), 0, "縮まないものは圧縮しない");
    assert_eq!(method(&bytes, "calm.bin"), 8);
    // ファイルから開いて書き直すと、全部のエントリの圧縮したバイト列をそのまま写す（同じ形なら同じバイト列）
    let path = dir.join("raw.ylp");
    std::fs::write(&path, &bytes).unwrap();
    let opened = read(&Source::open_file(&path).unwrap(), &Limits::default(), 0).unwrap();
    let again = small(1000).scoped(|| opened.to_bytes().unwrap());
    assert_eq!(again, bytes);
    // 中身が同じなら、エントリでない元（外したエントリ）からも写す
    let mut fresh = Package::build(
        files.iter().map(|(k, v)| (k.clone(), Blob::from(v.clone()))).collect(),
        3,
    )
    .unwrap();
    fresh.remember_donors(opened.files.values());
    let from_donors = small(1000).scoped(|| fresh.to_bytes().unwrap());
    assert_eq!(from_donors, bytes);
    // 今の形（YLP-3）は写さず、いつも今の書き手と同じ
    let classic = opened.to_bytes().unwrap();
    assert_eq!(classic, Archive::from_entries(files).unwrap().to_bytes().unwrap());
}

/// 並べて圧縮した Deflate の流れは、普通の読み手で元に戻り、前のかたまりを辞書にして縮む（かたまりの境目の前後・ちょうどの長さ）。
#[test]
fn parallel_deflate_makes_one_stream_any_reader_can_inflate() {
    use std::io::Read as _;
    for len in [4 << 20, (4 << 20) + 1, (5 << 20) + 12345, 3 << 20] {
        // 1 MiB を超える周期の模様（前のかたまりの終わりを辞書にしないと縮みにくい）と、ところどころの雑音
        let mut data: Vec<u8> = (0..len).map(|i| ((i * 7 / 3) % 251) as u8).collect();
        for (k, b) in noise(len / 64, len as u64).into_iter().enumerate() {
            data[k * 64] = b;
        }
        let blob = Blob::from(data.clone());
        let mut out = Vec::new();
        let (crc, n, packed) = ParallelDeflate::run(&blob, &mut out).unwrap();
        assert_eq!(n, len as u64);
        assert_eq!(packed, out.len() as u64);
        assert_eq!(crc, crc32fast::hash(&data));
        let mut back = Vec::new();
        flate2::read::DeflateDecoder::new(&out[..]).read_to_end(&mut back).unwrap();
        assert_eq!(back, data, "{len}");
        assert!(out.len() < len / 4, "{len}: {}", out.len());
        // 同じ入力は同じバイト列（スレッドの進み方に依らない）
        let mut again = Vec::new();
        ParallelDeflate::run(&blob, &mut again).unwrap();
        assert_eq!(again, out);
    }
}

/// `YLP-4` でも名前の決まりは今と同じ（危ない名前・予約の名前・正本の無いものは、書く前にも読むときにも断る）。
#[test]
fn ylp_4_keeps_the_name_rules() {
    for bad in ["../x", ".hidden", "a\\b", "sets/BAD/document.utpaint", "resources/a/b.png", "mimetype", "manifest.sha256"] {
        let mut files = sample();
        files.insert(bad.into(), vec![1]);
        let built = Package::build(
            files.iter().map(|(k, v)| (k.clone(), Blob::from(v.clone()))).collect(),
            4,
        );
        assert!(built.is_err(), "{bad}");
    }
    let mut no_document = sample();
    no_document.retain(|k, _| !k.ends_with("document.utpaint"));
    let built = Package::build(
        no_document.iter().map(|(k, v)| (k.clone(), Blob::from(v.clone()))).collect(),
        4,
    );
    assert!(built.is_err());
    // 書いた YLP-4 の manifest の名前を危ない名前に変えると、読むときに断る（manifest は圧縮しない大きさにして書き換える）
    let mut files = sample();
    files.insert("ab.bin".into(), vec![7]);
    let p = package_of(&files, 3);
    let mut plan = small(1000).scoped(|| p.plan().unwrap());
    let text = String::from_utf8(plan.manifest.to_vec()).unwrap().replace(" ab.bin\n", " a/.b\n");
    plan.manifest = Arc::from(text.into_bytes());
    let mut out = Cursor::new(Vec::new());
    p.write_with(&plan, &mut out).unwrap();
    assert!(Package::read_bytes(&out.into_inner(), &Limits::default()).is_err());
}

fn pool(threads: usize) -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap()
}

/// エントリごとの仕事を並べても、結果は 1 つずつかけたときと同じ（並び・断った理由）。どのスレッド数でも、はじめに断った要素より後ろの
/// 理由は返らない（前の要素のほうが遅くても）。
#[test]
fn work_in_parallel_returns_the_results_in_order_and_the_first_failure_in_order() {
    let items: Vec<u32> = (0..200).collect();
    for threads in [1, 2, 3, 8] {
        let ok = pool(threads).install(|| first_failure_in_order(&items, |n| Ok(n * 2)).unwrap());
        assert_eq!(ok, items.iter().map(|n| n * 2).collect::<Vec<_>>(), "{threads}");
        for _ in 0..10 {
            let failed = pool(threads).install(|| {
                first_failure_in_order(&items, |n| {
                    // 前の断り（37）は遅く、後ろの断り（55・150）は速い
                    if *n == 37 {
                        std::thread::sleep(std::time::Duration::from_millis(30));
                    }
                    match n {
                        37 | 55 | 150 => Err(Error::InvalidData(format!("断り {n}"))),
                        _ => Ok(*n),
                    }
                })
            });
            let Err(Error::InvalidData(why)) = failed else {
                panic!("断らなかった: {failed:?}");
            };
            assert_eq!(why, "断り 37", "{threads} 本");
        }
    }
    // 空
    assert!(first_failure_in_order::<u32, u32>(&[], |_| unreachable!()).unwrap().is_empty());
}

/// 1 回目の数え（全エントリの長さと SHA-256・manifest）と、書いたものの読み直し（全エントリの確かめ）は、スレッド数に依らず同じ結果・
/// 同じ断りの理由（名前の順にいちばん前に壊れたエントリ）。壊れたものは、名前の順で前のエントリの理由が先に出る。
#[test]
fn planning_and_verifying_do_not_depend_on_the_thread_count() {
    let mut files = BTreeMap::new();
    for i in 0..24u64 {
        files.insert(format!("{SET}/layer-{i:02}.bin"), noise(2000 + (i as usize) * 777, i + 5));
    }
    files.insert(format!("{SET}/document.utpaint"), noise(4000, 99));
    files.insert(format!("{SET}/composite/Color.png"), fake_png(2000));
    files.insert("project.json".into(), b"{}".to_vec());
    let p = package_of(&files, 3);
    let reference = pool(1).install(|| p.to_bytes().unwrap());
    for threads in [2, 3, 8] {
        assert_eq!(pool(threads).install(|| p.to_bytes().unwrap()), reference, "{threads} 本の plan と書き込み");
    }
    // 読み直し: 同じ中身（名前・長さ・SHA-256）
    let digest = |threads: usize, bytes: &[u8]| {
        pool(threads).install(|| {
            let read = Package::read_bytes(bytes, &Limits::default()).unwrap();
            read.entries().iter().map(|(n, b)| (n.clone(), b.len(), b.sha256().unwrap())).collect::<Vec<_>>()
        })
    };
    let expected = digest(1, &reference);
    assert_eq!(expected.len(), files.len());
    for threads in [2, 8] {
        assert_eq!(digest(threads, &reference), expected, "{threads}");
    }
    // 壊す: 圧縮しないエントリ（PNG。ファイルの中の位置を印から探せる）を増やして、その中身の 2 つを書き換える
    let mut with_pngs = files.clone();
    for i in 0..6u64 {
        with_pngs.insert(format!("{SET}/composite/Layer{i}.png"), fake_png(1500 + i as usize * 100));
    }
    let p = package_of(&with_pngs, 3);
    let good = p.to_bytes().unwrap();
    let find = |bytes: &[u8], from: usize, len: usize| -> usize {
        // fake_png の始まり（PNG の印）を探す
        (from..bytes.len() - len)
            .find(|&i| bytes[i..i + 8] == [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a])
            .expect("PNG の印")
    };
    let mut starts = Vec::new();
    let mut at = 0;
    for _ in 0..7 {
        at = find(&good, at, 8);
        starts.push(at);
        at += 8;
    }
    // PNG はファイルの中でも名前の順（Color.png・Layer0.png…）。Layer2 と Layer4 を壊す（名前の順で Layer2 が先に断られる）
    let mut bad = good.clone();
    for k in [3usize, 5] {
        bad[starts[k] + 100] ^= 0xff;
    }
    let reason = |threads: usize| {
        pool(threads).install(|| {
            (0..5)
                .map(|_| match Package::read_bytes(&bad, &Limits::default()) {
                    Ok(_) => panic!("壊れたのに読めた"),
                    Err(e) => e.to_string(),
                })
                .collect::<Vec<_>>()
        })
    };
    let first = reason(1)[0].clone();
    assert!(first.contains("Layer2.png"), "{first}");
    for threads in [1, 2, 3, 8] {
        for (i, why) in reason(threads).iter().enumerate() {
            assert_eq!(why, &first, "{threads} 本の {i} 回目");
        }
    }
}
