//! 個人のライブラリ（設定の「棚の場所」のフォルダ）とアセットの欄の試験。`headless_` で始まる試験は画面を描かず、Wine でも回る。
//! 一覧・項目を見る（別のスレッド）・サムネイルのキャッシュ・プロジェクトで使う・ライブラリへ入れる・足す・消す・置く。
//! 素材はこのリポジトリの人工データ（yolu-io の tests/fixtures/smart）と、試験の中で作ったものだけを使う。
use crate::common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use yolu_app::engine::{Channel, LayerId, LayerKind, Rgba8, TileCoord};
use yolu_app::lang::Lang;
use yolu_app::library::cache::Cache;
use yolu_app::library::{self, Source};
use yolu_app::shelf::{Block, ItemKind, PlaceTarget, ShelfOp, ShelfState, SHELF_BUDGET};
use yolu_app::state::{Action, AppState, DialogRequest};
use yolu_io::library as files;
use yolu_io::shelf::{image_hash, Shelf, MAX_RESOURCES};
use yolu_io::smart::SmartFile;
use yolu_io::{Project, UNITY_NATIVE_VERSION};

static NEXT: AtomicU32 = AtomicU32::new(0);

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../yolu-io/tests/fixtures/smart")
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixtures().join(name)).unwrap()
}

/// 試験ごとの使い捨てのフォルダ（終わると消す）。
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let path = std::env::temp_dir().join(format!(
            "yolu-libapp-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Dir(path)
    }

    /// ライブラリのフォルダ（この中に作る）。
    fn library(&self) -> PathBuf {
        self.0.join("library")
    }

    fn put(&self, rel: &str, bytes: &[u8]) {
        let path = self.library().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn read(&self, rel: &str) -> Vec<u8> {
        std::fs::read(self.library().join(rel)).unwrap()
    }

    fn exists(&self, rel: &str) -> bool {
        self.library().join(rel).exists()
    }

    /// ライブラリの中のファイル（一時ファイルを除く）の相対パス。
    fn files(&self) -> Vec<String> {
        fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
            let Ok(read) = std::fs::read_dir(dir) else {
                return;
            };
            let mut items: Vec<_> = read.filter_map(|e| e.ok()).collect();
            items.sort_by_key(|e| e.file_name());
            for e in items {
                let path = e.path();
                if path.is_dir() {
                    walk(&path, base, out);
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
        walk(&self.library(), &self.library(), &mut out);
        out
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut img = image::RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            img.put_pixel(x, y, image::Rgba(f(x, y)));
        }
    }
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// 3×2 の画像（透明な画素にも色を持たせる）。
fn sample_png() -> Vec<u8> {
    png(3, 2, |x, y| {
        [
            x as u8 * 80 + 1,
            y as u8 * 100 + 2,
            3,
            if (x + y) % 2 == 0 { 0 } else { 255 },
        ]
    })
}

/// `sample_png` の画素（文書と同じ向き＝下の行が先）の札。
fn sample_content() -> String {
    let mut rgba = Vec::new();
    for y in (0..2u32).rev() {
        for x in 0..3u32 {
            rgba.extend_from_slice(&[
                x as u8 * 80 + 1,
                y as u8 * 100 + 2,
                3,
                if (x + y) % 2 == 0 { 0 } else { 255 },
            ]);
        }
    }
    image_hash(&rgba, 3, 2).unwrap()
}

/// 16 ビット（RGBA）の PNG。1 つの値は上位バイトと下位バイトが同じ（8 ビットの値の 257 倍）なので、丸め方に依らず 8 ビットの値が決まる。
fn png16(w: u32, h: u32, samples: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Sixteen);
    let mut writer = encoder.write_header().unwrap();
    let data: Vec<u8> = samples.iter().flat_map(|v| [*v, *v]).collect();
    writer.write_image_data(&data).unwrap();
    writer.finish().unwrap();
    out
}

/// パレット（8 ビット）の PNG（1×1。色は (10, 20, 30)）。
fn png_palette() -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, 1, 1);
    encoder.set_color(png::ColorType::Indexed);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_palette(vec![10, 20, 30]);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&[0]).unwrap();
    writer.finish().unwrap();
    out
}

/// ライブラリのフォルダをこの試験のものにした状態（同梱の素材は並べない）。
fn state(dir: &Dir) -> AppState {
    let mut s = AppState::new(16, 16);
    s.prefs.settings.library_folder = Some(dir.library());
    s.shelf.show_builtin = false;
    s.library.source = Source::Library;
    s
}

/// 一覧を読み直し、全部の項目を見終わるまで待つ。
fn scan(s: &mut AppState) {
    let folder = s.library_root();
    s.library.refresh();
    s.library.prepare(folder);
    s.library.wait_probes();
    let rels: Vec<String> = s
        .library
        .visible(None, "")
        .iter()
        .map(|i| i.rel.clone())
        .collect();
    s.library.request_probes(&rels);
    s.library.wait_probes();
}

fn kinds(s: &AppState) -> BTreeMap<String, ItemKind> {
    s.library
        .visible(None, "")
        .into_iter()
        .map(|i| (i.rel, i.kind))
        .collect()
}

fn use_file(s: &mut AppState, rel: &str) {
    s.apply(Action::Shelf(ShelfOp::UseFromLibrary(rel.into())));
    s.shelf_wait();
}

fn put_in_library(s: &mut AppState, id: &str) {
    s.apply(Action::Shelf(ShelfOp::ToLibrary(id.into())));
    s.library_wait();
}

fn add_files(s: &mut AppState, paths: Vec<PathBuf>) {
    s.apply(Action::Shelf(ShelfOp::LibraryAddFiles(paths)));
    s.library_wait();
}

fn paint(s: &mut AppState, id: LayerId, rgba: [u8; 4]) {
    let ts = s.doc.tile_size() as usize;
    let (w, h) = (s.doc.width() as usize, s.doc.height() as usize);
    let mut bytes = vec![0u8; ts * ts * 4];
    for y in 0..h.min(ts) {
        for x in 0..w.min(ts) {
            bytes[(y * ts + x) * 4..][..4].copy_from_slice(&rgba);
        }
    }
    s.doc
        .import_tile(id, Channel::Color, TileCoord::new(0, 0), &bytes)
        .unwrap();
}

fn images_shelf(count: usize) -> Shelf {
    let mut shelf = Shelf::new(SHELF_BUDGET);
    for i in 0..count {
        let n = (i as u32).to_le_bytes();
        shelf
            .add_image_without_origin(
                &format!("00000000-0000-4000-8000-{i:012x}"),
                &format!("画像 {i}"),
                &[n[0], n[1], n[2], 255],
                1,
                1,
                "srgb",
            )
            .unwrap();
    }
    shelf
}

/// 1 つの素材を持つプロジェクトの棚。
fn fixture_shelf() -> Shelf {
    let root = fixtures().join("all");
    let mut files: BTreeMap<String, Arc<[u8]>> = BTreeMap::from([(
        "resources.json".into(),
        Arc::from(std::fs::read(root.join("resources.json")).unwrap()),
    )]);
    for entry in std::fs::read_dir(root.join("resources")).unwrap() {
        let entry = entry.unwrap();
        files.insert(
            format!("resources/{}", entry.file_name().to_string_lossy()),
            Arc::from(std::fs::read(entry.path()).unwrap()),
        );
    }
    Shelf::read(&files, SHELF_BUDGET).unwrap()
}

// ───────── 一覧と、項目を見る ─────────

#[test]
fn headless_the_library_lists_files_by_kind_and_looks_at_each_one() {
    let dir = Dir::new("kinds");
    dir.put("Smart/raster.ylsmart", &fixture("raster.ylsmart"));
    dir.put("Smart/mask.ylsmart", &fixture("mask.ylsmart"));
    dir.put("Smart/multi.ylsmart", &fixture("multi.ylsmart"));
    dir.put("Images/pic.png", &sample_png());
    dir.put("Brushes/brush.ylbrush", &fixture("brush.ylbrush"));
    dir.put("broken.ylsmart", b"not a smart file");
    dir.put("notes.txt", b"ignored");
    dir.put(".hidden.png", b"ignored");
    let mut s = state(&dir);
    scan(&mut s);
    assert!(s.library.is_listed() && s.library.problem().is_none());
    assert_eq!(
        kinds(&s),
        BTreeMap::from([
            ("Brushes/brush.ylbrush".to_owned(), ItemKind::Brush),
            ("Images/pic.png".to_owned(), ItemKind::Image),
            ("Smart/mask.ylsmart".to_owned(), ItemKind::SmartMask),
            ("Smart/multi.ylsmart".to_owned(), ItemKind::SmartMaterial),
            ("Smart/raster.ylsmart".to_owned(), ItemKind::SmartMaterial),
            ("broken.ylsmart".to_owned(), ItemKind::SmartMaterial),
        ])
    );
    let info = |rel: &str| {
        s.library
            .info(rel)
            .unwrap_or_else(|| panic!("{rel} を見ていない"))
    };
    // スマートマテリアル: レイヤーの数・大きさ・チャンネル・絵。中身の札はファイルの札
    let raster = info("Smart/raster.ylsmart");
    let bytes = fixture("raster.ylsmart");
    assert_eq!(
        (
            raster.inspected.layers,
            raster.inspected.width,
            raster.inspected.height
        ),
        (1, 3, 2)
    );
    assert_eq!(raster.inspected.channels, [Channel::Color]);
    assert!(raster.inspected.has_thumbnail() && raster.inspected.block.is_none());
    assert_eq!(raster.sha256, files::sha256_hex(&bytes));
    assert_eq!(raster.content, raster.sha256);
    let multi = info("Smart/multi.ylsmart");
    assert_eq!(multi.inspected.layers, 2);
    assert_eq!(
        multi.inspected.channels,
        [Channel::Color, Channel::Roughness]
    );
    // スマートマスク: 種類はファイルの中の印。絵がある
    let mask = info("Smart/mask.ylsmart");
    assert_eq!(mask.kind, ItemKind::SmartMask);
    assert!(mask.inspected.has_thumbnail() && mask.inspected.block.is_none());
    // 画像: 大きさ・絵・棚の画像と同じ画素の札
    let pic = info("Images/pic.png");
    assert_eq!((pic.inspected.width, pic.inspected.height), (3, 2));
    assert!(pic.inspected.has_thumbnail());
    assert_eq!(pic.content, sample_content());
    assert_eq!(pic.sha256, files::sha256_hex(&sample_png()));
    // ブラシのファイルは、まだ置けない種類として並べる
    assert_eq!(
        info("Brushes/brush.ylbrush").inspected.block,
        Some(Block::Kind(ItemKind::Brush))
    );
    // 読めないファイルは、並べて理由を出す（読み飛ばして止まらない）
    let broken = info("broken.ylsmart");
    let Some(Block::Unreadable(why)) = &broken.inspected.block else {
        panic!("{:?}", broken.inspected.block)
    };
    assert!(!why.reason(Lang::Ja).is_empty() && why.reason(Lang::En).is_ascii());
    assert!(!broken.inspected.has_thumbnail());
    // 種類・名前・フォルダで絞る
    let names = |kind, text: &str| -> Vec<String> {
        s.library
            .visible(kind, text)
            .into_iter()
            .map(|i| i.rel)
            .collect()
    };
    assert_eq!(names(Some(ItemKind::SmartMask), ""), ["Smart/mask.ylsmart"]);
    assert_eq!(names(None, "MULTI"), ["Smart/multi.ylsmart"]);
    assert_eq!(names(None, "images"), ["Images/pic.png"]);
    assert_eq!(names(Some(ItemKind::Image), "smart"), Vec::<String>::new());
    assert_eq!(names(Some(ItemKind::Material), ""), Vec::<String>::new());
}

#[test]
fn headless_asking_for_pictures_never_waits_for_them() {
    let dir = Dir::new("never-waits");
    for i in 0..6u8 {
        dir.put(
            &format!("Images/{i}.png"),
            &png(256, 256, |x, y| [x as u8 ^ i, y as u8, i, 255]),
        );
    }
    dir.put("Smart/raster.ylsmart", &fixture("raster.ylsmart"));
    let mut s = state(&dir);
    let folder = s.library_root();
    s.library.prepare(folder);
    s.library.wait_probes();
    let rels: Vec<String> = s
        .library
        .visible(None, "")
        .iter()
        .map(|i| i.rel.clone())
        .collect();
    assert_eq!(rels.len(), 7);
    // 別のスレッドを止めておいても、頼む・受け取る・描くための問い合わせは返る（仕事の終わりを待たない）
    s.library.hold_probes(true);
    s.library.request_probes(&rels);
    assert_eq!(s.library.probes_pending(), 7);
    assert!(!s.library.poll());
    assert!(rels.iter().all(|r| s.library.info(r).is_none()));
    assert!(s.library.is_busy());
    // 放すと、できた分が受け取れる
    s.library.hold_probes(false);
    s.library.wait_probes();
    assert!(rels.iter().all(|r| s.library.info(r).is_some()));
    assert!(!s.library.is_busy());
}

#[test]
fn headless_items_nobody_looks_at_any_more_are_not_looked_at() {
    let dir = Dir::new("stale");
    dir.put("Smart/raster.ylsmart", &fixture("raster.ylsmart"));
    dir.put("Smart/mask.ylsmart", &fixture("mask.ylsmart"));
    let mut s = state(&dir);
    let folder = s.library_root();
    s.library.prepare(folder);
    s.library.wait_probes();
    s.library.hold_probes(true);
    s.library.request_probes(&[
        "Smart/raster.ylsmart".to_owned(),
        "Smart/mask.ylsmart".to_owned(),
    ]);
    // スクロールして、mask だけ見えているフレームが続く
    for _ in 0..8 {
        s.library.begin_frame();
        s.library.request_probes(&["Smart/mask.ylsmart".to_owned()]);
    }
    s.library.hold_probes(false);
    s.library.wait_probes();
    assert!(s.library.info("Smart/mask.ylsmart").is_some());
    assert!(
        s.library.info("Smart/raster.ylsmart").is_none(),
        "見えなくなった項目の仕事は始めない"
    );
    // また見えれば作る
    s.library
        .request_probes(&["Smart/raster.ylsmart".to_owned()]);
    s.library.wait_probes();
    assert!(s.library.info("Smart/raster.ylsmart").is_some());
}

#[test]
fn headless_pictures_are_remembered_by_content_and_a_broken_cache_is_rebuilt() {
    let dir = Dir::new("cache");
    dir.put("Smart/raster.ylsmart", &fixture("raster.ylsmart"));
    dir.put("Smart/mask.ylsmart", &fixture("mask.ylsmart"));
    dir.put("Images/pic.png", &sample_png());
    dir.put("broken.ylsmart", b"not a smart file");
    let cache_dir = dir.0.join("cache");
    let pictures = |s: &AppState| -> BTreeMap<String, Option<yolu_app::shelf::Picture>> {
        s.library
            .visible(None, "")
            .into_iter()
            .map(|i| {
                let p = s
                    .library
                    .info(&i.rel)
                    .and_then(|k| k.inspected.picture().cloned());
                (i.rel, p)
            })
            .collect()
    };
    let mut first = state(&dir);
    first.library.attach_cache(Some(cache_dir.clone()));
    scan(&mut first);
    let made = pictures(&first);
    assert!(made["Smart/raster.ylsmart"].is_some() && made["Images/pic.png"].is_some());
    assert!(made["broken.ylsmart"].is_none());
    // 絵のある 3 つだけ覚える（読めなかったものは覚えない）
    let cache = Cache::new(cache_dir.clone());
    assert_eq!(cache.usage().0, 3);
    let stamps: Vec<_> = std::fs::read_dir(&cache_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        // 更新時刻を前にしておく（書き直されたら今の時刻になって食い違う。時刻の粒度で同じ値に見えて見逃さない）
        .map(|e| (e.file_name(), common::tmp::backdate(&e.path())))
        .collect();

    // 別のセッション: 覚えた絵を使う（同じ絵・同じ説明。ファイルは書き直さない）
    let mut second = state(&dir);
    second.library.attach_cache(Some(cache_dir.clone()));
    scan(&mut second);
    assert_eq!(pictures(&second), made);
    let again = |rel: &str| {
        let i = second.library.info(rel).unwrap();
        (
            i.inspected.layers,
            i.inspected.width,
            i.inspected.height,
            i.kind,
            i.content.clone(),
        )
    };
    assert_eq!(again("Smart/mask.ylsmart").3, ItemKind::SmartMask);
    assert_eq!(again("Images/pic.png").4, sample_content());
    assert_eq!(again("Smart/raster.ylsmart").0, 1);
    for (name, modified) in &stamps {
        let now = std::fs::metadata(cache_dir.join(name))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(
            &now, modified,
            "{name:?}: 覚えた絵を使うだけで、書き直さない"
        );
    }

    // 壊れた覚えは捨てて作り直す（絵は同じ）
    for e in std::fs::read_dir(&cache_dir)
        .unwrap()
        .filter_map(|e| e.ok())
    {
        std::fs::write(e.path(), b"broken").unwrap();
    }
    let mut third = state(&dir);
    third.library.attach_cache(Some(cache_dir.clone()));
    scan(&mut third);
    assert_eq!(pictures(&third), made);
    assert_eq!(
        Cache::new(cache_dir.clone()).usage().0,
        3,
        "作り直して覚え直す"
    );
}

#[test]
fn headless_a_changed_file_is_looked_at_again_and_a_removed_one_goes_away() {
    let dir = Dir::new("changes");
    dir.put("Images/pic.png", &sample_png());
    dir.put("Smart/raster.ylsmart", &fixture("raster.ylsmart"));
    let mut s = state(&dir);
    scan(&mut s);
    assert_eq!(s.library.info("Images/pic.png").unwrap().inspected.width, 3);
    s.library.selected = Some(library::library_id("Images/pic.png"));
    // 同じ名前で中身が替わった
    dir.put(
        "Images/pic.png",
        &png(5, 4, |x, y| [x as u8, y as u8, 0, 255]),
    );
    scan(&mut s);
    let now = s.library.info("Images/pic.png").unwrap();
    assert_eq!((now.inspected.width, now.inspected.height), (5, 4));
    // 消えた
    std::fs::remove_file(dir.library().join("Images/pic.png")).unwrap();
    scan(&mut s);
    assert!(s.library.info("Images/pic.png").is_none());
    assert_eq!(s.library.visible(None, "").len(), 1);
    assert_eq!(s.library.selected, None, "消えた項目の選びは外れる");
}

#[test]
fn headless_the_limits_keep_big_files_from_being_read_or_expanded() {
    let dir = Dir::new("limits");
    dir.put("Smart/raster.ylsmart", &fixture("raster.ylsmart"));
    dir.put("Images/pic.png", &sample_png());
    // 上限を変えた新しい状態で見る
    let looked = |change: &dyn Fn(&mut AppState)| {
        let mut s = state(&dir);
        change(&mut s);
        scan(&mut s);
        s
    };
    // 読む大きさの上限（1 バイト足りない・ちょうど）
    let len = fixture("raster.ylsmart").len() as u64;
    let short = looked(&|s| s.library.limits.read = len - 1);
    assert!(matches!(
        short
            .library
            .info("Smart/raster.ylsmart")
            .unwrap()
            .inspected
            .block,
        Some(Block::Unreadable(_))
    ));
    let exact = looked(&|s| s.library.limits.read = len);
    assert!(exact
        .library
        .info("Smart/raster.ylsmart")
        .unwrap()
        .inspected
        .block
        .is_none());
    // 画像の画素の上限: 大きさだけを見て、絵と札は作らない
    let over = looked(&|s| s.library.limits.thumb_pixels = 5);
    let pic = over.library.info("Images/pic.png").unwrap();
    assert_eq!((pic.inspected.width, pic.inspected.height), (3, 2));
    assert!(!pic.inspected.has_thumbnail() && pic.content.is_empty());
    let fits = looked(&|s| s.library.limits.thumb_pixels = 6);
    assert!(fits
        .library
        .info("Images/pic.png")
        .unwrap()
        .inspected
        .has_thumbnail());
    // スマート素材の展開の予算: 絵は出さないが、読めて置ける
    let budget = looked(&|s| s.library.limits.preview = 0);
    let raster = budget.library.info("Smart/raster.ylsmart").unwrap();
    assert!(!raster.inspected.has_thumbnail() && raster.inspected.block.is_none());
}

#[test]
fn headless_a_reload_looks_again_at_files_that_could_not_be_read() {
    let dir = Dir::new("reload");
    dir.put("Smart/raster.ylsmart", &fixture("raster.ylsmart"));
    let mut s = state(&dir);
    let len = fixture("raster.ylsmart").len() as u64;
    s.library.limits.read = len - 1;
    scan(&mut s);
    assert!(matches!(
        s.library
            .info("Smart/raster.ylsmart")
            .unwrap()
            .inspected
            .block,
        Some(Block::Unreadable(_))
    ));
    // 失敗の理由が消えた（ファイルは変わっていない）。読み直すと、見直す
    s.library.limits.read = len;
    scan(&mut s);
    assert!(s
        .library
        .info("Smart/raster.ylsmart")
        .unwrap()
        .inspected
        .block
        .is_none());
}

#[test]
fn headless_a_library_location_that_is_not_a_folder_says_so_and_a_missing_one_is_empty() {
    let dir = Dir::new("problems");
    // 無いフォルダ: 空
    let mut s = state(&dir);
    scan(&mut s);
    assert!(s.library.is_listed() && s.library.problem().is_none());
    assert!(s.library.visible(None, "").is_empty());
    // フォルダでない場所
    std::fs::write(dir.0.join("plain"), b"x").unwrap();
    s.prefs.settings.library_folder = Some(dir.0.join("plain"));
    scan(&mut s);
    let why = s.library.problem().expect("フォルダでない");
    assert!(
        why.reason(Lang::Ja).contains("フォルダ"),
        "{}",
        why.reason(Lang::Ja)
    );
    assert!(why.reason(Lang::En).is_ascii());
    assert!(s.library.visible(None, "").is_empty());
    // 場所を替えれば、前の場所の一覧は残らない
    dir.put("Smart/raster.ylsmart", &fixture("raster.ylsmart"));
    s.prefs.settings.library_folder = Some(dir.library());
    scan(&mut s);
    assert!(s.library.problem().is_none());
    assert_eq!(s.library.visible(None, "").len(), 1);
    std::fs::create_dir_all(dir.0.join("other")).unwrap();
    s.prefs.settings.library_folder = Some(dir.0.join("other"));
    scan(&mut s);
    assert!(s.library.visible(None, "").is_empty());
}

#[cfg(unix)]
#[test]
fn headless_links_in_the_library_are_skipped_and_never_followed() {
    use std::os::unix::fs::symlink;
    let dir = Dir::new("links");
    let outside = dir.0.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.png"), sample_png()).unwrap();
    dir.put("Images/real.png", &sample_png());
    symlink(
        outside.join("secret.png"),
        dir.library().join("Images/link.png"),
    )
    .unwrap();
    symlink(&outside, dir.library().join("folder-link")).unwrap();
    let mut s = state(&dir);
    scan(&mut s);
    assert_eq!(
        s.library
            .visible(None, "")
            .into_iter()
            .map(|i| i.rel)
            .collect::<Vec<_>>(),
        ["Images/real.png"]
    );
    assert_eq!(s.library.skipped().len(), 2);
    // 置く・使う・消すも、リンクの先へは届かない
    for rel in ["Images/link.png", "folder-link/secret.png"] {
        use_file(&mut s, rel);
        assert!(s.shelf.resources().is_empty(), "{rel}");
        s.apply(Action::Shelf(ShelfOp::LibraryRemove(rel.into())));
        assert!(outside.join("secret.png").exists(), "{rel}");
        s.apply(Action::Shelf(ShelfOp::Place {
            id: library::library_id(rel),
            target: PlaceTarget::Selected,
        }));
        assert_eq!(s.doc.layers().len(), 1, "{rel}");
    }
}

#[test]
fn headless_a_big_library_keeps_the_information_it_remembers_within_a_limit() {
    let dir = Dir::new("big");
    let total = library::MAX_KNOWN + 200;
    for i in 0..total {
        dir.put(
            &format!("Images/{i:05}.png"),
            &png(2, 2, |x, y| {
                [(i % 251) as u8, (i / 251) as u8, x as u8 + y as u8, 255]
            }),
        );
    }
    let mut s = state(&dir);
    let folder = s.library_root();
    s.library.prepare(folder);
    s.library.wait_probes();
    let rels: Vec<String> = s
        .library
        .visible(None, "")
        .iter()
        .map(|i| i.rel.clone())
        .collect();
    assert_eq!(rels.len(), total);
    // 見えている所だけ頼む（スクロールするように、20 個ずつ全部を順に見る）
    assert_eq!(s.library.known_count(), 0);
    for window in rels.chunks(20) {
        s.library.begin_frame();
        s.library.request_probes(window);
        s.library.wait_probes();
        assert!(window.iter().all(|r| s.library.info(r).is_some()));
        assert!(
            s.library.known_count() <= library::MAX_KNOWN,
            "{}",
            s.library.known_count()
        );
    }
    // 古いものは捨てて、いま見える分は残る。捨てたものは、また見えれば作り直す
    assert!(s.library.info(&rels[0]).is_none());
    let last = rels.last().unwrap();
    assert!(s.library.info(last).is_some());
    s.library.request_probes(&rels[..20]);
    s.library.wait_probes();
    assert!(s.library.info(&rels[0]).is_some());
}

// ───────── プロジェクトで使う ─────────

#[test]
fn headless_using_a_library_file_copies_it_into_the_project_with_its_origin() {
    let dir = Dir::new("use");
    let raster = fixture("raster.ylsmart");
    dir.put("Smart/raster.ylsmart", &raster);
    let mut s = state(&dir);
    scan(&mut s);
    assert!(!s
        .library
        .in_project(s.shelf.shelf(), "Smart/raster.ylsmart"));
    use_file(&mut s, "Smart/raster.ylsmart");
    assert_eq!(s.shelf.resources().len(), 1, "{}", s.message);
    let res = &s.shelf.resources()[0];
    assert_eq!(
        (res.kind.as_str(), res.name.as_str()),
        ("smartMaterial", "試験素材")
    );
    assert_eq!(res.content, files::sha256_hex(&raster));
    assert_eq!(
        s.shelf.shelf().content_bytes(&res.id),
        Some(raster.as_slice()),
        "写しはファイルと同じバイト列"
    );
    let origin = &res.metadata["origin"];
    assert_eq!(origin["type"], "library");
    assert_eq!(origin["file"], "Smart/raster.ylsmart");
    assert_eq!(origin["sha256"], files::sha256_hex(&raster));
    assert_eq!(origin["length"], raster.len());
    assert!(s.modified && s.shelf.changed);
    assert!(
        s.message
            .starts_with("プロジェクトに取り込みました: raster"),
        "{}",
        s.message
    );
    assert_eq!(s.shelf.selected.as_deref(), Some(res.id.as_str()));
    assert!(s
        .library
        .in_project(s.shelf.shelf(), "Smart/raster.ylsmart"));
    // もう一度は、何も足さない
    s.modified = false;
    use_file(&mut s, "Smart/raster.ylsmart");
    assert_eq!(s.shelf.resources().len(), 1);
    assert!(!s.modified);
    assert!(
        s.message.starts_with("すでにプロジェクトにあります"),
        "{}",
        s.message
    );
    // ライブラリを消しても、プロジェクトの写しは残り、置ける
    std::fs::remove_file(dir.library().join("Smart/raster.ylsmart")).unwrap();
    let id = s.shelf.resources()[0].id.clone();
    s.apply(Action::Shelf(ShelfOp::Place {
        id,
        target: PlaceTarget::Selected,
    }));
    assert_eq!(s.doc.layers().len(), 2, "{}", s.message);
}

#[test]
fn headless_a_used_file_survives_saving_and_opening_the_project_without_the_library() {
    let dir = Dir::new("roundtrip");
    dir.put("Smart/raster.ylsmart", &fixture("raster.ylsmart"));
    dir.put("Images/pic.png", &sample_png());
    dir.put("Brushes/brush.ylbrush", &fixture("brush.ylbrush"));
    let mut s = state(&dir);
    scan(&mut s);
    for rel in [
        "Smart/raster.ylsmart",
        "Images/pic.png",
        "Brushes/brush.ylbrush",
    ] {
        use_file(&mut s, rel);
    }
    assert_eq!(s.shelf.resources().len(), 3, "{}", s.message);
    let path = dir.0.join("project.ylp");
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    // ライブラリの無い所でも開ける（写しを持つ）。出どころは覚えている
    std::fs::remove_dir_all(dir.library()).unwrap();
    let project = Project::read(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(project.resources().len(), 3);
    let mut again = AppState::new(8, 8);
    again.apply(Action::OpenProject(path));
    assert_eq!(again.shelf.resources().len(), 3, "{}", again.message);
    for r in s.shelf.resources() {
        let got = again.shelf.get(&r.id).expect("戻る");
        assert_eq!(got.metadata["origin"], r.metadata["origin"], "{}", r.name);
        assert_eq!(
            again.shelf.shelf().content_bytes(&r.id),
            s.shelf.shelf().content_bytes(&r.id)
        );
    }
    let origins: Vec<String> = again
        .shelf
        .resources()
        .iter()
        .map(|r| r.metadata["origin"]["file"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        origins,
        [
            "Smart/raster.ylsmart",
            "Images/pic.png",
            "Brushes/brush.ylbrush"
        ]
    );
}

#[test]
fn headless_a_used_image_keeps_every_pixel_and_the_same_picture_is_one_resource() {
    let dir = Dir::new("use-image");
    let bytes = sample_png();
    dir.put("Images/pic.png", &bytes);
    // 同じ画素を別の PNG の書き方で
    dir.put("Images/same-pixels.png", &{
        let img = image::load_from_memory(&bytes).unwrap().to_rgba8();
        let mut out = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new_with_quality(
            &mut out,
            image::codecs::png::CompressionType::Best,
            image::codecs::png::FilterType::NoFilter,
        );
        image::ImageEncoder::write_image(
            encoder,
            img.as_raw(),
            3,
            2,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
        out
    });
    let mut s = state(&dir);
    scan(&mut s);
    use_file(&mut s, "Images/pic.png");
    assert_eq!(s.shelf.resources().len(), 1, "{}", s.message);
    let res = s.shelf.resources()[0].clone();
    assert_eq!(res.kind, "image");
    assert_eq!(
        res.content,
        sample_content(),
        "透明な画素の RGB も含む画素の札"
    );
    assert_eq!(
        (
            res.metadata["width"].as_u64(),
            res.metadata["height"].as_u64()
        ),
        (Some(3), Some(2))
    );
    assert_eq!(res.metadata["colorSpace"], "unspecified");
    assert_eq!(res.metadata["origin"]["length"], bytes.len());
    // 画素が同じなら、PNG の書き方が違っても 1 つ
    use_file(&mut s, "Images/same-pixels.png");
    assert_eq!(s.shelf.resources().len(), 1);
    assert!(
        s.message.starts_with("すでにプロジェクトにあります"),
        "{}",
        s.message
    );
    // 両方とも、プロジェクトにあるしるしが付く（画素の札で見分ける）
    scan(&mut s);
    assert!(s.library.in_project(s.shelf.shelf(), "Images/pic.png"));
    assert!(s
        .library
        .in_project(s.shelf.shelf(), "Images/same-pixels.png"));
}

#[test]
fn headless_a_file_that_cannot_be_used_says_why_and_changes_nothing() {
    let dir = Dir::new("use-refused");
    dir.put("broken.ylsmart", b"not a smart file");
    dir.put("Images/cut.png", &{
        let mut b = sample_png();
        b.truncate(b.len() - 20);
        b
    });
    dir.put("Images/big.png", &sample_png());
    let mut s = state(&dir);
    scan(&mut s);
    for (rel, expect) in [
        (
            "broken.ylsmart",
            "「broken」をライブラリから取り込めません（",
        ),
        (
            "Images/cut.png",
            "「cut」をライブラリから取り込めません（PNG として読めません）。",
        ),
        (
            "missing.ylsmart",
            "「missing」をライブラリから取り込めません（",
        ),
        (
            "../outside.ylsmart",
            "「outside」をライブラリから取り込めません（名前に使えない文字があります）。",
        ),
        (
            "Images/..\\x.png",
            "をライブラリから取り込めません（名前に使えない文字があります）。",
        ),
    ] {
        s.message.clear();
        use_file(&mut s, rel);
        assert!(s.message.contains(expect), "{rel}: {}", s.message);
        assert!(s.shelf.resources().is_empty(), "{rel}");
        assert!(!s.modified && !s.shelf.changed, "{rel}");
    }
    // 大きさの上限（1 バイト足りない）
    s.library.limits.read = sample_png().len() as u64 - 1;
    use_file(&mut s, "Images/big.png");
    assert!(
        s.message
            .starts_with("「big」をライブラリから取り込めません（大きすぎます）"),
        "{}",
        s.message
    );
    assert!(s.shelf.resources().is_empty());
    // 英語の画面では英語の理由
    s.lang = Lang::En;
    use_file(&mut s, "Images/cut.png");
    assert_eq!(
        s.message,
        "Cannot import \"cut\" from the library (Not a readable PNG)."
    );
}

#[test]
fn headless_using_a_file_is_refused_when_the_shelf_is_full_or_unreadable() {
    let dir = Dir::new("use-full");
    dir.put("Smart/raster.ylsmart", &fixture("raster.ylsmart"));
    let mut s = state(&dir);
    s.shelf = ShelfState::with_shelf(images_shelf(MAX_RESOURCES));
    use_file(&mut s, "Smart/raster.ylsmart");
    assert!(
        s.message.contains("アセットがいっぱいです"),
        "{}",
        s.message
    );
    assert_eq!(s.shelf.resources().len(), MAX_RESOURCES);
    assert!(!s.modified);
    s.shelf = ShelfState::unreadable("試験の理由");
    use_file(&mut s, "Smart/raster.ylsmart");
    assert!(s.message.contains("試験の理由"), "{}", s.message);
    assert!(s.shelf.resources().is_empty());
}

#[test]
fn headless_a_slow_import_can_be_cancelled_and_one_for_a_closed_project_is_dropped() {
    let dir = Dir::new("use-cancel");
    dir.put("Smart/raster.ylsmart", &fixture("raster.ylsmart"));
    dir.put("Smart/mask.ylsmart", &fixture("mask.ylsmart"));
    let mut s = state(&dir);
    s.shelf.hold_saves(true);
    s.apply(Action::Shelf(ShelfOp::UseFromLibrary(
        "Smart/raster.ylsmart".into(),
    )));
    assert_eq!(s.shelf.saving_name(), Some("raster"));
    assert_eq!(s.shelf.saving_label(Lang::Ja), "取り込み中");
    // 走っている間は、次の取り込み・保存は断る
    s.apply(Action::Shelf(ShelfOp::UseFromLibrary(
        "Smart/mask.ylsmart".into(),
    )));
    assert!(s.message.contains("取り込み中です"), "{}", s.message);
    // やめる: できても棚へ入れない
    s.apply(Action::Shelf(ShelfOp::CancelSave));
    assert!(
        s.message.starts_with("取り込みをやめました: raster"),
        "{}",
        s.message
    );
    assert!(s.shelf.saving_name().is_none());
    s.shelf.hold_saves(false);
    s.shelf.wait_idle();
    s.shelf_poll();
    assert!(s.shelf.resources().is_empty());
    // 終わるまでに別のプロジェクトへ替えたら、結果は捨てる
    s.shelf.hold_saves(true);
    s.apply(Action::Shelf(ShelfOp::UseFromLibrary(
        "Smart/raster.ylsmart".into(),
    )));
    s.apply(Action::NewProject);
    s.shelf.hold_saves(false);
    s.shelf.wait_idle();
    s.shelf_poll();
    assert!(s.shelf.resources().is_empty() && s.shelf.saving_name().is_none());
}

// ───────── ライブラリへ入れる ─────────

#[test]
fn headless_putting_shelf_items_into_the_library_writes_their_bytes_once() {
    let dir = Dir::new("put");
    let mut s = state(&dir);
    s.shelf = ShelfState::with_shelf(fixture_shelf());
    let items: Vec<_> = s.shelf.resources().to_vec();
    assert_eq!(items.len(), 6);
    for r in &items {
        put_in_library(&mut s, &r.id);
        assert!(
            s.message.ends_with("」をライブラリに入れました。"),
            "{}: {}",
            r.name,
            s.message
        );
    }
    let written = dir.files();
    assert_eq!(written.len(), 6, "{written:?}");
    for r in &items {
        let (folder, ext) = match r.kind.as_str() {
            "image" => ("Images", "png"),
            "smartMaterial" | "smartMask" => ("Smart", "ylsmart"),
            "brush" => ("Brushes", "ylbrush"),
            "material" => ("Materials", "ylmaterial"),
            other => panic!("{other}"),
        };
        let rel = format!("{folder}/{}.{ext}", r.name);
        assert!(dir.exists(&rel), "{rel} が無い: {written:?}");
        assert_eq!(
            Some(dir.read(&rel).as_slice()),
            s.shelf.shelf().content_bytes(&r.id),
            "{rel}: 棚の写しと同じバイト列"
        );
    }
    // 同じ素材をもう一度入れても、書かない
    put_in_library(&mut s, &items[2].id);
    assert!(
        s.message.ends_with("」はすでにライブラリにあります。"),
        "{}",
        s.message
    );
    assert_eq!(dir.files().len(), 6);
    // 一覧に出て、見られる（棚の素材は変えない）
    scan(&mut s);
    assert_eq!(s.library.visible(None, "").len(), 6);
    assert!(!s.shelf.changed);
    // 入れた素材は、プロジェクトの棚と同じ中身なので、プロジェクトにあるしるしが付く
    let index = s.library.project_index(s.shelf.shelf());
    let rel = "Smart/raster.ylsmart";
    let content = &s.library.info(rel).unwrap().content;
    assert!(index.contains(files::Kind::Smart, rel, content));
}

#[test]
fn headless_a_taken_name_gets_a_number_and_what_is_there_is_never_replaced() {
    let dir = Dir::new("put-names");
    dir.put("Smart/raster.ylsmart", b"somebody else's file");
    let mut s = state(&dir);
    s.shelf = ShelfState::with_shelf(fixture_shelf());
    let raster = s
        .shelf
        .resources()
        .iter()
        .find(|r| r.name == "raster")
        .unwrap()
        .id
        .clone();
    put_in_library(&mut s, &raster);
    assert!(
        s.message.contains("Smart/raster 2.ylsmart"),
        "{}",
        s.message
    );
    assert_eq!(dir.read("Smart/raster.ylsmart"), b"somebody else's file");
    assert_eq!(
        Some(dir.read("Smart/raster 2.ylsmart").as_slice()),
        s.shelf.shelf().content_bytes(&raster)
    );
    // 大文字小文字だけが違う名前も重ねない
    dir.put("Smart/MASK.ylsmart", b"other");
    let mask = s
        .shelf
        .resources()
        .iter()
        .find(|r| r.name == "mask")
        .unwrap()
        .id
        .clone();
    put_in_library(&mut s, &mask);
    assert!(s.message.contains("Smart/mask 2.ylsmart"), "{}", s.message);
}

#[test]
fn headless_an_image_whose_pixels_are_already_in_the_library_is_not_written_again() {
    let dir = Dir::new("put-pixels");
    dir.put("Images/mine.png", &sample_png());
    let mut s = state(&dir);
    scan(&mut s);
    // 同じ画素の画像を、棚へ別のやり方で入れる（PNG の書き方が違う）
    let mut rgba = Vec::new();
    for y in (0..2u32).rev() {
        for x in 0..3u32 {
            rgba.extend_from_slice(&[
                x as u8 * 80 + 1,
                y as u8 * 100 + 2,
                3,
                if (x + y) % 2 == 0 { 0 } else { 255 },
            ]);
        }
    }
    let id = s
        .shelf
        .add_image(Lang::Ja, "同じ画素", &rgba, 3, 2)
        .unwrap();
    put_in_library(&mut s, &id);
    assert!(
        s.message == "「Images/mine.png」はすでにライブラリにあります。",
        "{}",
        s.message
    );
    assert_eq!(dir.files(), ["Images/mine.png"]);
    // まだ見ていないライブラリでは、PNG のバイト列が違えば、別のファイルになる（見ていない中身は比べない）
    let mut fresh = state(&dir);
    let id = fresh
        .shelf
        .add_image(Lang::Ja, "同じ画素", &rgba, 3, 2)
        .unwrap();
    put_in_library(&mut fresh, &id);
    assert_eq!(dir.files().len(), 2, "{:?}", dir.files());
}

#[test]
fn headless_built_in_items_and_unreadable_ones_are_not_put_into_the_library() {
    let dir = Dir::new("put-refused");
    let mut s = state(&dir);
    s.shelf.show_builtin = true;
    s.shelf.use_language(Lang::Ja);
    s.apply(Action::Shelf(ShelfOp::ToLibrary(
        "builtin:rusty-iron".into(),
    )));
    assert!(
        s.message.contains("組み込みはライブラリへ入れられません"),
        "{}",
        s.message
    );
    s.apply(Action::Shelf(ShelfOp::ToLibrary("no-such-item".into())));
    assert!(dir.files().is_empty());
}

#[test]
fn headless_a_slow_write_can_be_cancelled_and_leaves_nothing_behind() {
    // 旗が仕事のスレッドへ届くこと・画面側の状態（走っている間は次を断る・やめた書き込みのスレッドが終わるまで待つ）の試験。
    // 書き込みの途中（一時ファイルを書いている間）の取り消しと、その後始末は yolu-io の試験
    // （an_add_cancelled_while_it_writes_removes_the_half_written_temporary_file）。ここでは仕事が始まる前に旗が立つ
    let dir = Dir::new("put-cancel");
    let mut s = state(&dir);
    s.shelf = ShelfState::with_shelf(fixture_shelf());
    let ids: Vec<String> = s.shelf.resources().iter().map(|r| r.id.clone()).collect();
    s.library.hold_writes(true);
    s.apply(Action::Shelf(ShelfOp::ToLibrary(ids[2].clone())));
    assert!(s.library.writing_name().is_some());
    // 走っている間は、次の書き込みを断る
    s.apply(Action::Shelf(ShelfOp::ToLibrary(ids[3].clone())));
    assert!(
        s.message.contains("ライブラリへ書き込み中です"),
        "{}",
        s.message
    );
    s.apply(Action::Shelf(ShelfOp::CancelSave));
    assert!(
        s.message.starts_with("書き込みをやめました"),
        "{}",
        s.message
    );
    assert!(s.library.writing_name().is_none());
    // やめた書き込みのスレッドが終わるまでは、次を始めない
    assert!(s.library.busy_reason(Lang::Ja).is_some());
    s.library.hold_writes(false);
    s.library.wait_idle();
    s.library_poll();
    assert!(dir.files().is_empty(), "{:?}", dir.files());
    assert!(s.library.busy_reason(Lang::Ja).is_none());
    put_in_library(&mut s, &ids[2]);
    assert_eq!(dir.files().len(), 1);
}

// ───────── ファイルを足す・消す ─────────

#[test]
fn headless_adding_files_checks_each_one_and_tells_the_refused_ones_by_name() {
    let dir = Dir::new("add");
    let outside = dir.0.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let make = |name: &str, bytes: &[u8]| {
        let path = outside.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    };
    let good_smart = make("木目.ylsmart", &fixture("raster.ylsmart"));
    let good_png = make("pic.png", &sample_png());
    let junk = make("junk.ylsmart", b"not a smart file");
    let cut = make("cut.png", &sample_png()[..40]);
    let text = make("notes.txt", b"hello");
    let mut s = state(&dir);
    add_files(&mut s, vec![good_smart.clone()]);
    assert!(
        s.message == "「Smart/試験素材.ylsmart」をライブラリに入れました。",
        "{}",
        s.message
    );
    assert_eq!(
        dir.read("Smart/試験素材.ylsmart"),
        fixture("raster.ylsmart")
    );
    // 同じファイルをもう一度（別の名前でも）は、足さない
    let copy = make("another name.ylsmart", &fixture("raster.ylsmart"));
    add_files(&mut s, vec![copy]);
    assert!(
        s.message == "「Smart/試験素材.ylsmart」はすでにライブラリにあります。",
        "{}",
        s.message
    );
    // まとめて: 入れた分・すでにあった分・断った理由が 1 つの知らせに
    add_files(&mut s, vec![good_png, good_smart, junk, cut, text]);
    let message = s.message.clone();
    assert!(
        message.starts_with("「junk.ylsmart」をライブラリに入れられません（"),
        "{message}"
    );
    for expect in [
        "「cut.png」をライブラリに入れられません（PNG として読めません）。",
        "「notes.txt」をライブラリに入れられません（PNG と .ylsmart だけです）。",
    ] {
        assert!(message.contains(expect), "{expect}: {message}");
    }
    assert!(
        message.contains("1 件をライブラリに入れました。"),
        "{message}"
    );
    assert!(
        message.contains("1 件はすでにライブラリにありました。"),
        "{message}"
    );
    assert_eq!(dir.files(), ["Images/pic.png", "Smart/試験素材.ylsmart"]);
    // 大きさの上限（1 バイト足りない）
    s.library.limits.read = sample_png().len() as u64 - 1;
    let big = make("big.png", &sample_png());
    add_files(&mut s, vec![big]);
    assert!(
        s.message
            .contains("「big.png」をライブラリに入れられません（大きすぎます）"),
        "{}",
        s.message
    );
    // 英語
    s.lang = Lang::En;
    let cut = outside.join("cut.png");
    add_files(&mut s, vec![cut]);
    assert_eq!(
        s.message,
        "Cannot add \"cut.png\" to the library (Not a readable PNG)."
    );
    s.library.limits.read = sample_png().len() as u64;
    add_files(&mut s, vec![make("again.png", &sample_png())]);
    assert_eq!(s.message, "\"Images/pic.png\" is already in the library.");
}

#[test]
fn headless_removing_takes_only_the_file_after_asking_and_keeps_the_projects_copy() {
    let dir = Dir::new("remove");
    dir.put("Smart/raster.ylsmart", &fixture("raster.ylsmart"));
    dir.put("Smart/mask.ylsmart", &fixture("mask.ylsmart"));
    let victim = dir.0.join("victim.png");
    std::fs::write(&victim, sample_png()).unwrap();
    let mut s = state(&dir);
    scan(&mut s);
    use_file(&mut s, "Smart/raster.ylsmart");
    s.apply(Action::Shelf(ShelfOp::Place {
        id: library::library_id("Smart/mask.ylsmart"),
        target: PlaceTarget::Selected,
    }));
    let layers = s.doc.layers().len();
    // 確認のウィンドウを頼む（まだ消さない）
    s.apply(Action::Shelf(ShelfOp::LibraryAskRemove(
        "Smart/raster.ylsmart".into(),
    )));
    assert_eq!(s.dialog_request, Some(DialogRequest::LibraryRemove));
    assert_eq!(
        s.library.pending_remove.as_deref(),
        Some("Smart/raster.ylsmart")
    );
    assert!(dir.exists("Smart/raster.ylsmart"));
    s.dialog_request = None;
    // 一覧に無いものは確かめない
    s.apply(Action::Shelf(ShelfOp::LibraryAskRemove(
        "Smart/none.ylsmart".into(),
    )));
    assert_eq!(s.dialog_request, None);
    // 消す
    s.library.selected = Some(library::library_id("Smart/raster.ylsmart"));
    s.apply(Action::Shelf(ShelfOp::LibraryRemove(
        "Smart/raster.ylsmart".into(),
    )));
    assert!(
        s.message.starts_with("ライブラリから消しました: raster"),
        "{}",
        s.message
    );
    assert_eq!(dir.files(), ["Smart/mask.ylsmart"]);
    assert!(s.library.pending_remove.is_none() && s.library.selected.is_none());
    // プロジェクトの写しと、置いたレイヤーは変わらない
    assert_eq!(s.shelf.resources().len(), 1);
    assert_eq!(s.doc.layers().len(), layers);
    // ライブラリの外のファイルは消せない
    for rel in [
        "../victim.png",
        "/etc/hostname",
        "Smart/..\\..\\victim.png",
        "Smart",
    ] {
        s.apply(Action::Shelf(ShelfOp::LibraryRemove(rel.into())));
        assert!(
            s.message.contains("をライブラリから消せません（"),
            "{rel}: {}",
            s.message
        );
    }
    assert!(victim.exists());
    assert_eq!(dir.files(), ["Smart/mask.ylsmart"]);
}

// ───────── 置く ─────────

#[test]
fn headless_a_library_file_is_placed_without_going_through_the_shelf_in_one_undo_step() {
    let dir = Dir::new("place");
    dir.put("Smart/raster.ylsmart", &fixture("raster.ylsmart"));
    dir.put("Smart/mask.ylsmart", &fixture("mask.ylsmart"));
    dir.put("Images/pic.png", &sample_png());
    dir.put("Brushes/brush.ylbrush", &fixture("brush.ylbrush"));
    dir.put("broken.ylsmart", b"not a smart file");
    let mut s = state(&dir);
    let base = s.selected_layer.unwrap();
    paint(&mut s, base, [200, 40, 30, 255]);
    let place = |s: &mut AppState, rel: &str| {
        s.apply(Action::Shelf(ShelfOp::Place {
            id: library::library_id(rel),
            target: PlaceTarget::Selected,
        }))
    };
    let before = s.doc.layers().len();
    place(&mut s, "Smart/raster.ylsmart");
    assert_eq!(s.doc.layers().len(), before + 1, "{}", s.message);
    assert!(s.message.starts_with("置きました: raster"), "{}", s.message);
    assert!(s.shelf.resources().is_empty(), "棚へは入れない");
    s.apply(Action::Undo);
    assert_eq!(s.doc.layers().len(), before);
    // 画像は 1 枚のレイヤー
    place(&mut s, "Images/pic.png");
    assert_eq!(s.doc.layers().len(), before + 1, "{}", s.message);
    s.apply(Action::Undo);
    // スマートマスクは選んだレイヤーのマスクへ
    s.selected_layer = Some(base);
    place(&mut s, "Smart/mask.ylsmart");
    assert!(s.doc.layer(base).unwrap().mask().is_some(), "{}", s.message);
    s.apply(Action::Undo);
    assert!(s.doc.layer(base).unwrap().mask().is_none());
    // 置けないもの: 理由を出して何も変えない
    let layers = s.doc.layers().len();
    for (rel, expect) in [
        ("Brushes/brush.ylbrush", "ブラシ"),
        ("broken.ylsmart", "「broken」を置けません（"),
        ("missing.png", "「missing」を置けません（"),
        (
            "../x.png",
            "「x」を置けません（名前に使えない文字があります）",
        ),
    ] {
        s.message.clear();
        place(&mut s, rel);
        assert!(s.message.contains(expect), "{rel}: {}", s.message);
        assert_eq!(s.doc.layers().len(), layers, "{rel}");
    }
}

#[test]
fn headless_a_16_bit_png_is_added_unchanged_and_used_with_a_note_that_it_was_reduced() {
    let dir = Dir::new("png16");
    let outside = dir.0.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let samples = [10u8, 128, 200, 255, 0, 255, 77, 0];
    let bytes = png16(2, 1, &samples);
    std::fs::write(outside.join("height.png"), &bytes).unwrap();
    let mut s = state(&dir);
    // 足すときは、ファイルのバイト列のまま（丸めない）
    add_files(&mut s, vec![outside.join("height.png")]);
    assert_eq!(s.message, "「Images/height.png」をライブラリに入れました。");
    assert_eq!(dir.read("Images/height.png"), bytes);
    scan(&mut s);
    // 使うと 8 ビットになり、その知らせを出す（黙って落とさない）
    use_file(&mut s, "Images/height.png");
    assert_eq!(s.shelf.resources().len(), 1, "{}", s.message);
    assert_eq!(
        s.message,
        "プロジェクトに取り込みました: height · 16 bit を 8 bit にしました"
    );
    let res = s.shelf.resources()[0].clone();
    assert_eq!(res.content, image_hash(&samples, 2, 1).unwrap());
    // 英語の画面
    s.apply(Action::Shelf(ShelfOp::Remove(res.id.clone())));
    s.lang = Lang::En;
    use_file(&mut s, "Images/height.png");
    assert_eq!(
        s.message,
        "Added to the project: height · Reduced from 16 to 8 bits"
    );
    // すでにプロジェクトにあるときは、取り込んでいないので、丸めた知らせは足さない
    use_file(&mut s, "Images/height.png");
    assert_eq!(s.message, "Already in the project: height");
    // 置くときも知らせる（置けたときだけ）
    let before = s.doc.layers().len();
    s.apply(Action::Shelf(ShelfOp::Place {
        id: library::library_id("Images/height.png"),
        target: PlaceTarget::Selected,
    }));
    assert_eq!(s.doc.layers().len(), before + 1, "{}", s.message);
    assert!(
        s.message.ends_with(" · Reduced from 16 to 8 bits"),
        "{}",
        s.message
    );
}

#[test]
fn headless_png_files_that_read_exactly_are_used_without_a_rounding_note() {
    let dir = Dir::new("png8");
    dir.put("Images/pic.png", &sample_png());
    dir.put("Images/palette.png", &png_palette());
    let mut s = state(&dir);
    scan(&mut s);
    for (rel, name) in [("Images/pic.png", "pic"), ("Images/palette.png", "palette")] {
        use_file(&mut s, rel);
        assert_eq!(
            s.message,
            format!("プロジェクトに取り込みました: {name}"),
            "{rel}"
        );
    }
    // パレットの色は 8 ビットの RGBA へそのまま広がる
    let palette = s
        .shelf
        .resources()
        .iter()
        .find(|r| r.name == "palette")
        .unwrap()
        .clone();
    assert_eq!(
        palette.content,
        image_hash(&[10, 20, 30, 255], 1, 1).unwrap()
    );
    let layers = s.doc.layers().len();
    s.apply(Action::Shelf(ShelfOp::Place {
        id: library::library_id("Images/pic.png"),
        target: PlaceTarget::Selected,
    }));
    assert_eq!(s.doc.layers().len(), layers + 1);
    assert!(s.message.starts_with("置きました: pic"), "{}", s.message);
    assert!(!s.message.contains("16 bit"), "{}", s.message);
}

#[test]
fn headless_the_smart_filters_keep_files_not_yet_looked_at_until_they_are_looked_at() {
    let dir = Dir::new("smart-filter");
    dir.put("Smart/raster.ylsmart", &fixture("raster.ylsmart"));
    dir.put("Smart/multi.ylsmart", &fixture("multi.ylsmart"));
    dir.put("Smart/mask.ylsmart", &fixture("mask.ylsmart"));
    dir.put("Images/pic.png", &sample_png());
    let mut s = state(&dir);
    // 一覧を読み終えただけ（どのファイルも見ていない）
    let folder = s.library_root();
    s.library.refresh();
    s.library.prepare(folder);
    s.library.wait_probes();
    assert!(s.library.is_listed() && s.library.known_count() == 0);
    let rels = |s: &AppState, filter| -> Vec<String> {
        s.library
            .visible(filter, "")
            .into_iter()
            .map(|i| i.rel)
            .collect()
    };
    // マスクの絞り込みで、見る前の .ylsmart は除かれない（除くと、見に行かないので、マスクが一度も出ない）。画像は除く
    let masks = rels(&s, Some(ItemKind::SmartMask));
    assert_eq!(
        masks,
        [
            "Smart/mask.ylsmart",
            "Smart/multi.ylsmart",
            "Smart/raster.ylsmart"
        ]
    );
    assert!(s
        .library
        .visible(Some(ItemKind::SmartMask), "")
        .iter()
        .all(|i| i.kind == ItemKind::SmartMask));
    // マテリアルの絞り込みも同じ
    assert_eq!(rels(&s, Some(ItemKind::SmartMaterial)), masks);
    assert_eq!(rels(&s, Some(ItemKind::Image)), ["Images/pic.png"]);
    // 絞り込み後の一覧を画面と同じように見に行くと、見た種類で絞り直される
    s.library.request_probes(&masks);
    s.library.wait_probes();
    assert_eq!(rels(&s, Some(ItemKind::SmartMask)), ["Smart/mask.ylsmart"]);
    assert_eq!(
        rels(&s, Some(ItemKind::SmartMaterial)),
        ["Smart/multi.ylsmart", "Smart/raster.ylsmart"]
    );
    // 名前の検索と組み合わせても同じ
    assert_eq!(
        s.library.visible(Some(ItemKind::SmartMask), "mask").len(),
        1
    );
    assert!(s
        .library
        .visible(Some(ItemKind::SmartMask), "raster")
        .is_empty());
}

// ───────── 断りの言葉 ─────────

#[test]
fn headless_every_library_refusal_has_a_short_sentence_in_both_languages() {
    let markers = [
        files::REFUSAL_ROOT_LINK,
        files::REFUSAL_ROOT_NOT_FOLDER,
        files::REFUSAL_LINK,
        files::REFUSAL_PATH,
        files::REFUSAL_NOT_FILE,
        files::REFUSAL_TOO_LARGE,
        files::REFUSAL_EMPTY,
        files::REFUSAL_NO_NAME,
        library::REFUSAL_UNSUPPORTED,
        library::REFUSAL_PNG,
        library::REFUSAL_PNG_SIZE,
    ];
    for lang in Lang::ALL {
        let texts: Vec<String> = markers
            .iter()
            .map(|m| {
                yolu_app::lang::library_io_error(lang, &yolu_io::Error::InvalidData((*m).into()))
            })
            .collect();
        for (i, a) in texts.iter().enumerate() {
            assert!(!a.is_empty(), "{lang:?} {}", markers[i]);
            assert_eq!(a.is_ascii(), lang == Lang::En, "{lang:?} {a}");
            assert!(a.chars().count() < 40, "短い理由: {a}");
            for b in &texts[i + 1..] {
                assert_ne!(a, b, "{lang:?}");
            }
        }
    }
    // 棚の断り（予算・個数）は、棚の言い方のまま
    let budget = yolu_io::Error::Budget(yolu_io::shelf::REFUSAL_MEMORY_BUDGET.into());
    assert_eq!(
        yolu_app::lang::library_io_error(Lang::Ja, &budget),
        "アセットの予算を超えます"
    );
}

// ───────── 棚（プロジェクト）のサムネイルも別のスレッドで ─────────

fn project_ids(shelf: &ShelfState) -> Vec<String> {
    shelf.resources().iter().map(|r| r.id.clone()).collect()
}

#[test]
fn headless_the_shelfs_pictures_are_made_on_another_thread_and_equal_the_direct_ones() {
    let mut direct = ShelfState::with_shelf(fixture_shelf());
    direct.show_builtin = false;
    let ids = project_ids(&direct);
    for id in &ids {
        direct.inspect(id);
    }
    let mut later = ShelfState::with_shelf(fixture_shelf());
    later.show_builtin = false;
    later.hold_inspections(true);
    later.request_inspections(&ids, None);
    // 別のスレッドを止めている間は、何も無い（頼んでも待たない。ブラシは見る所が無いので、その場で決まる）
    for r in later.resources() {
        let slow = r.kind != "brush";
        assert_eq!(later.info(&r.id).is_none(), slow, "{}", r.name);
    }
    assert!(later.inspections_pending());
    assert_eq!(later.poll_inspections(100), 0);
    later.hold_inspections(false);
    later.wait_inspections();
    assert!(!later.inspections_pending());
    for id in &ids {
        let (a, b) = (direct.info(id).unwrap(), later.info(id).unwrap());
        assert_eq!(
            (a.width, a.height, a.layers, &a.channels, &a.block),
            (b.width, b.height, b.layers, &b.channels, &b.block),
            "{id}"
        );
        assert_eq!(a.picture(), b.picture(), "{id}");
    }
    assert!(ids.iter().any(|id| later.info(id).unwrap().has_thumbnail()));
}

#[test]
fn headless_the_shelfs_pictures_are_remembered_across_sessions_and_a_removed_item_is_dropped() {
    let dir = Dir::new("shelf-cache");
    let cache = Arc::new(Cache::new(dir.0.join("cache")));
    let mut first = ShelfState::with_shelf(fixture_shelf());
    first.show_builtin = false;
    let ids = project_ids(&first);
    first.request_inspections(&ids, Some(&cache));
    first.wait_inspections();
    let kept = cache.usage().0;
    assert!(kept >= 3, "絵のある項目を覚える: {kept}");
    let mut second = ShelfState::with_shelf(fixture_shelf());
    second.show_builtin = false;
    second.request_inspections(&ids, Some(&cache));
    second.wait_inspections();
    assert_eq!(cache.usage().0, kept, "覚えた絵を使うだけ");
    for id in &ids {
        assert_eq!(
            first.info(id).unwrap().picture(),
            second.info(id).unwrap().picture(),
            "{id}"
        );
        assert_eq!(
            first.info(id).unwrap().layers,
            second.info(id).unwrap().layers,
            "{id}"
        );
    }
}

#[test]
fn headless_a_bundled_items_picture_is_made_on_another_thread_too() {
    let mut s = ShelfState::default();
    s.use_language(Lang::Ja);
    let ids: Vec<String> = s.visible().iter().map(|r| r.id.clone()).take(4).collect();
    assert_eq!(ids.len(), 4);
    s.request_inspections(&ids, None);
    s.wait_inspections();
    for id in &ids {
        let info = s.info(id).unwrap_or_else(|| panic!("{id}"));
        assert!(info.has_thumbnail() && info.block.is_none(), "{id}");
    }
}

#[test]
fn headless_an_item_removed_from_the_shelf_while_being_looked_at_is_not_brought_back() {
    let mut s = AppState::new(16, 16);
    s.shelf = ShelfState::with_shelf(fixture_shelf());
    s.shelf.show_builtin = false;
    let ids = project_ids(&s.shelf);
    s.shelf.hold_inspections(true);
    s.shelf.request_inspections(&ids, None);
    let gone = ids
        .iter()
        .find(|id| s.shelf.get(id).is_some_and(|r| r.name == "raster"))
        .unwrap()
        .clone();
    s.apply(Action::Shelf(ShelfOp::Remove(gone.clone())));
    assert!(s.shelf.get(&gone).is_none(), "{}", s.message);
    s.shelf.hold_inspections(false);
    s.shelf.wait_inspections();
    assert!(s.shelf.info(&gone).is_none());
    assert!(ids
        .iter()
        .filter(|id| **id != gone)
        .all(|id| s.shelf.info(id).is_some()));
}

#[test]
fn headless_the_documents_pixels_survive_a_library_round_trip_of_a_saved_layer() {
    // レイヤーをスマートマテリアルとして棚へ保存し、ライブラリへ入れ、別のプロジェクトで使って置くと、同じ画素になる
    let dir = Dir::new("journey");
    let mut a = state(&dir);
    let base = a.selected_layer.unwrap();
    paint(&mut a, base, [200, 40, 30, 255]);
    a.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    let id = a.shelf.resources()[0].id.clone();
    put_in_library(&mut a, &id);
    assert_eq!(dir.files().len(), 1);
    let mut b = state(&dir);
    scan(&mut b);
    let rel = b.library.visible(None, "")[0].rel.clone();
    use_file(&mut b, &rel);
    assert_eq!(b.shelf.resources().len(), 1, "{}", b.message);
    let used = b.shelf.resources()[0].id.clone();
    let before = b.selected_layer.unwrap();
    b.apply(Action::Shelf(ShelfOp::Place {
        id: used,
        target: PlaceTarget::Selected,
    }));
    let placed = b.selected_layer.unwrap();
    assert_ne!(placed, before);
    let pixel = b
        .doc
        .layer(placed)
        .unwrap()
        .surface(Channel::Color)
        .unwrap()
        .pixel(1, 1)
        .unwrap()
        .to_array();
    assert_eq!(pixel, [200, 40, 30, 255]);
}

// ───────── マテリアル（塗りつぶしレイヤー） ─────────

const IRON: Rgba8 = Rgba8::new(90, 80, 70, 255);
const WHITE: Rgba8 = Rgba8::new(255, 255, 255, 255);

/// Color と Metallic の値を持ち、マスクの付いた塗りつぶしレイヤーを 1 つ足して選ぶ。
fn fill_layer(s: &mut AppState, name: &str) -> LayerId {
    let id = s
        .doc
        .add_fill_layer(
            name,
            &[(Channel::Color, IRON), (Channel::Metallic, WHITE)],
            None,
        )
        .unwrap();
    s.doc.add_layer_mask(id).unwrap();
    s.selected_layer = Some(id);
    id
}

fn save_as_material(s: &mut AppState, id: LayerId) {
    s.apply(Action::Shelf(ShelfOp::SaveAsMaterial(id)));
    s.library_wait();
}

/// レイヤーの種類・Color と Metallic の値・マスクの有無。
fn fill_of(s: &AppState, id: LayerId) -> (LayerKind, Option<Rgba8>, Option<Rgba8>, bool) {
    let l = s.doc.layer(id).unwrap();
    (
        l.kind(),
        l.fill_value(Channel::Color),
        l.fill_value(Channel::Metallic),
        l.mask().is_some(),
    )
}

fn place_here(s: &mut AppState, id: String) {
    s.apply(Action::Shelf(ShelfOp::Place {
        id,
        target: PlaceTarget::Selected,
    }));
}

#[test]
fn headless_a_fill_layer_saved_as_a_material_is_placed_as_a_fill_layer_and_kept_in_the_ylp() {
    let dir = Dir::new("material");
    let mut s = state(&dir);
    let fill = fill_layer(&mut s, "鉄");
    let (undo, revision) = (s.doc.undo_count(), s.doc.revision());
    save_as_material(&mut s, fill);
    assert_eq!(
        s.message,
        "「Materials/鉄.ylmaterial」をライブラリに入れました。"
    );
    assert_eq!(dir.files(), ["Materials/鉄.ylmaterial"]);
    // 文書とプロジェクトのアセットは変えない
    assert_eq!((s.doc.undo_count(), s.doc.revision()), (undo, revision));
    assert!(s.shelf.resources().is_empty() && !s.shelf.changed && !s.modified);
    // 中身は塗りつぶしレイヤー 1 つの .ylsmart と同じ形で、Unity 版（0.2.0）が読む範囲（smart.json の形式 1・種類 smartMaterial・正本の版 21 まで）
    let bytes = dir.read("Materials/鉄.ylmaterial");
    let file = SmartFile::read(&bytes).unwrap();
    assert_eq!(file.info()["format"], 1);
    assert_eq!(file.info()["kind"], "smartMaterial");
    assert_eq!(file.info()["name"], "鉄");
    assert_eq!(file.info()["layers"], 1);
    assert!(
        file.fragment().version() <= UNITY_NATIVE_VERSION,
        "{}",
        file.fragment().version()
    );
    // 同じ名前でもう一度: 番号の付いた別のファイル（前のファイルは置き換えない）
    save_as_material(&mut s, fill);
    assert_eq!(
        dir.files(),
        ["Materials/鉄 2.ylmaterial", "Materials/鉄.ylmaterial"],
        "{}",
        s.message
    );
    assert_eq!(dir.read("Materials/鉄.ylmaterial"), bytes);
    std::fs::remove_file(dir.library().join("Materials/鉄 2.ylmaterial")).unwrap();
    // ライブラリにマテリアルとして並び、置ける（絵がある）
    scan(&mut s);
    let rel = "Materials/鉄.ylmaterial";
    let info = s.library.info(rel).unwrap();
    assert_eq!(info.kind, ItemKind::Material);
    assert!(info.inspected.block.is_none(), "{:?}", info.inspected.block);
    assert!(info.inspected.has_thumbnail());
    assert_eq!(info.inspected.layers, 1);
    assert_eq!(info.inspected.channels, [Channel::Color, Channel::Metallic]);
    let listed: Vec<String> = s
        .library
        .visible(Some(ItemKind::Material), "")
        .into_iter()
        .map(|i| i.rel)
        .collect();
    assert_eq!(listed, [rel]);
    // ライブラリから置く: 選んだレイヤーの上に、マスクの無い塗りつぶしレイヤーが 1 つ。1 回の取り消しで戻る
    let layers = s.doc.layers().len();
    place_here(&mut s, library::library_id(rel));
    assert_eq!(s.doc.layers().len(), layers + 1, "{}", s.message);
    assert!(s.message.starts_with("置きました: 鉄"), "{}", s.message);
    let placed = s.selected_layer.unwrap();
    assert_ne!(placed, fill);
    assert_eq!(
        fill_of(&s, placed),
        (LayerKind::Fill, Some(IRON), Some(WHITE), false)
    );
    s.apply(Action::Undo);
    assert_eq!(s.doc.layers().len(), layers);
    // プロジェクトで使う: マテリアルの種類でアセットに入り（バイト列はファイルのまま）、そこから置ける
    use_file(&mut s, rel);
    let res = s.shelf.resources()[0].clone();
    assert_eq!((res.kind.as_str(), res.name.as_str()), ("material", "鉄"));
    assert_eq!(s.shelf.shelf().content_bytes(&res.id).unwrap(), bytes);
    s.shelf.inspect(&res.id);
    assert_eq!(s.shelf.block_of(&res.id), None);
    place_here(&mut s, res.id.clone());
    assert_eq!(s.doc.layers().len(), layers + 1, "{}", s.message);
    let placed = s.selected_layer.unwrap();
    // 保存して開き直す: アセットのマテリアルと置いたレイヤーが同じに戻る。.ylp の中身の形式は上げない（形式 6 の種類）
    let path = dir.0.join("material.ylp");
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let project = Project::read(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(project.info().format, 7);
    assert_eq!(project.resources()[0].kind, "material");
    let mut again = state(&dir);
    again.apply(Action::OpenProject(path));
    let got = again
        .shelf
        .get(&res.id)
        .unwrap_or_else(|| panic!("{}", again.message))
        .clone();
    assert_eq!((got.kind.as_str(), got.name.as_str()), ("material", "鉄"));
    assert_eq!(again.shelf.shelf().content_bytes(&res.id).unwrap(), bytes);
    assert_eq!(fill_of(&again, placed), fill_of(&s, placed));
    let layers = again.doc.layers().len();
    again.selected_layer = Some(placed);
    place_here(&mut again, res.id);
    assert_eq!(again.doc.layers().len(), layers + 1, "{}", again.message);
    let twice = again.selected_layer.unwrap();
    assert_eq!(
        fill_of(&again, twice),
        (LayerKind::Fill, Some(IRON), Some(WHITE), false)
    );
    again.apply(Action::Undo);
    assert_eq!(again.doc.layers().len(), layers);
}

#[test]
fn headless_only_a_fill_layer_is_saved_as_a_material_and_offered_in_its_menu() {
    use yolu_app::state::PopupKind;
    use yolu_app::ui::menu::Entry;
    let dir = Dir::new("material-refused");
    let mut s = state(&dir);
    let raster = s.selected_layer.unwrap();
    let group = s.doc.add_group("組", None).unwrap();
    let fill = fill_layer(&mut s, "鉄");
    let labels = |s: &AppState, id| -> Vec<String> {
        yolu_app::shell::popup_entries(s, PopupKind::LayerContext(id))
            .into_iter()
            .filter_map(|e| match e {
                Entry::Item { label, .. } => Some(label),
                _ => None,
            })
            .collect()
    };
    assert!(labels(&s, fill).contains(&"マテリアルとして保存".to_owned()));
    for id in [raster, group] {
        let menu = labels(&s, id);
        assert!(
            !menu.iter().any(|l| l == "マテリアルとして保存"),
            "{menu:?}"
        );
        assert!(menu.contains(&"スマートマテリアルとして保存".to_owned()));
    }
    let name = s.doc.layer(raster).unwrap().name().to_owned();
    let fingerprint = |s: &AppState| (s.doc.layers().len(), s.doc.undo_count(), s.doc.revision());
    let before = fingerprint(&s);
    save_as_material(&mut s, raster);
    assert_eq!(
        s.message,
        format!(
            "「{name}」をマテリアルとして保存できません（塗りつぶしのレイヤーではありません）。"
        )
    );
    save_as_material(&mut s, group);
    assert_eq!(
        s.message,
        "「組」をマテリアルとして保存できません（塗りつぶしのレイヤーではありません）。"
    );
    assert_eq!(fingerprint(&s), before);
    assert!(s.shelf.resources().is_empty() && !s.modified);
    // アセットの画像を使う塗りつぶしは、項目を押せなくして理由をツールチップに。押しても断る
    let rid = s
        .shelf
        .add_image(Lang::Ja, "四色", &[1, 2, 3, 255], 1, 1)
        .unwrap();
    let textured = fill_layer(&mut s, "絵");
    s.apply(Action::Fill(yolu_app::fillfx::FillOp::Image {
        layer: textured,
        channel: Channel::Color,
        image: yolu_app::fillfx::inputs::image_id(&rid),
    }));
    assert!(s
        .doc
        .layer(textured)
        .unwrap()
        .fill_image(Channel::Color)
        .is_some());
    let item = yolu_app::shell::popup_entries(&s, PopupKind::LayerContext(textured))
        .into_iter()
        .find(|e| e.label() == Some("マテリアルとして保存"))
        .unwrap();
    let Entry::Item {
        enabled, tooltip, ..
    } = item
    else {
        panic!("項目")
    };
    assert!(!enabled);
    assert_eq!(tooltip.as_deref(), Some("画像を使っています"));
    let before = fingerprint(&s);
    save_as_material(&mut s, textured);
    assert_eq!(
        s.message,
        "「絵」をマテリアルとして保存できません（画像を使っています）。"
    );
    assert_eq!(fingerprint(&s), before);
    s.lang = Lang::En;
    assert!(labels(&s, fill).contains(&"Save as Material".to_owned()));
    save_as_material(&mut s, raster);
    assert_eq!(
        s.message,
        format!("Cannot save \"{name}\" as a material (Not a fill layer).")
    );
    save_as_material(&mut s, textured);
    assert_eq!(
        s.message,
        "Cannot save \"絵\" as a material (It uses images)."
    );
    assert!(dir.files().is_empty());
    assert_eq!(fingerprint(&s), before);
    assert!(s.shelf.resources().len() == 1);
    // 英語の済んだ知らせも、名前を引用符で文に入れる
    save_as_material(&mut s, fill);
    assert_eq!(
        s.message,
        "Added \"Materials/鉄.ylmaterial\" to the library."
    );
}

/// 書き込みのスレッドで `.ylsmart` にできないと断られるレイヤー（画素のフィルターのノイズ・グランジ、ユーザーチャンネルの値）は、
/// 押した操作の名前（「マテリアルとして保存できません」）で理由を知らせ、ライブラリにも文書にも何も残さない。
#[test]
fn headless_a_fill_layer_the_writer_cannot_hold_is_refused_as_a_material_not_as_a_library_add() {
    use yolu_core::generator::{Kind, Settings};
    use yolu_core::{EffectSettings, FilterSpec, FilterTarget};
    let dir = Dir::new("material-writer-refused");
    let mut s = state(&dir);
    let noisy = fill_layer(&mut s, "鉄");
    s.doc
        .add_filter(
            noisy,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(Settings::new(Kind::Noise)))
                .channels(&[Channel::Color]),
        )
        .unwrap();
    let channel = s
        .doc
        .add_channel(yolu_app::m2::new_channel_info(
            "AO".into(),
            yolu_app::engine::ChannelKind::Scalar,
        ))
        .unwrap();
    let user = s
        .doc
        .add_fill_layer("遮蔽", &[(channel, WHITE)], None)
        .unwrap();
    let fingerprint = |s: &AppState| (s.doc.layers().len(), s.doc.undo_count(), s.doc.revision());
    let before = fingerprint(&s);
    for (id, name, ja, en) in [
        (
            noisy,
            "鉄",
            "ノイズ・グランジを使っています",
            "It uses Noise and Grunge",
        ),
        (
            user,
            "遮蔽",
            "ユーザーチャンネルを使っています",
            "It uses user channels",
        ),
    ] {
        for lang in [Lang::Ja, Lang::En] {
            s.lang = lang;
            save_as_material(&mut s, id);
            let want = match lang {
                Lang::Ja => format!("「{name}」をマテリアルとして保存できません（{ja}）。"),
                Lang::En => format!("Cannot save \"{name}\" as a material ({en})."),
            };
            assert_eq!(s.message, want);
            assert!(
                !s.message.contains("ライブラリに入れられません")
                    && !s.message.contains("to the library"),
                "{}",
                s.message
            );
        }
    }
    assert!(dir.files().is_empty(), "{:?}", dir.files());
    assert_eq!(fingerprint(&s), before);
    assert!(s.shelf.resources().is_empty() && !s.modified);
}

#[test]
fn headless_a_material_file_that_cannot_be_placed_says_why_and_changes_nothing() {
    let dir = Dir::new("material-broken");
    dir.put("Materials/broken.ylmaterial", b"not a smart file");
    dir.put("Materials/mask.ylmaterial", &fixture("mask.ylsmart"));
    dir.put("Materials/raster.ylmaterial", &fixture("raster.ylsmart"));
    let mut s = state(&dir);
    let base = s.selected_layer.unwrap();
    paint(&mut s, base, [200, 40, 30, 255]);
    scan(&mut s);
    // 壊れたファイルと、中身がスマートマスクのファイルは、マテリアルとして並べて理由を出す
    for rel in ["Materials/broken.ylmaterial", "Materials/mask.ylmaterial"] {
        let info = s.library.info(rel).unwrap();
        assert_eq!(info.kind, ItemKind::Material, "{rel}");
        let Some(Block::Unreadable(why)) = &info.inspected.block else {
            panic!("{rel}: {:?}", info.inspected.block)
        };
        assert_eq!(
            (why.reason(Lang::Ja), why.reason(Lang::En)),
            ("形式が合いません", "Wrong format")
        );
    }
    let (layers, undo) = (s.doc.layers().len(), s.doc.undo_count());
    let place = |s: &mut AppState, rel: &str| place_here(s, library::library_id(rel));
    place(&mut s, "Materials/broken.ylmaterial");
    assert!(
        s.message.starts_with("「broken」を置けません（"),
        "{}",
        s.message
    );
    place(&mut s, "Materials/mask.ylmaterial");
    assert_eq!(s.message, "「mask」を置けません（形式が合いません）。");
    // レイヤーの画素の予算を超える: 置く前に断る
    s.doc
        .set_source_budget_bytes(s.doc.allocated_bytes() + 16)
        .unwrap();
    place(&mut s, "Materials/raster.ylmaterial");
    assert!(
        s.message.contains("レイヤーのメモリの予算を超えます"),
        "{}",
        s.message
    );
    // 読む大きさの上限を超える: 読まずに断る
    s.library.limits.read = fixture("raster.ylsmart").len() as u64 - 1;
    place(&mut s, "Materials/raster.ylmaterial");
    assert_eq!(s.message, "「raster」を置けません（大きすぎます）。");
    assert_eq!(
        (s.doc.layers().len(), s.doc.undo_count()),
        (layers, undo),
        "断った操作は文書と取り消しに残らない"
    );
    // 英語の画面では英語の理由
    s.lang = Lang::En;
    place(&mut s, "Materials/mask.ylmaterial");
    assert_eq!(s.message, "Cannot place \"mask\" (Wrong format).");
    // プロジェクトで使う: 形式の合わないファイルはアセットへ入らない
    use_file(&mut s, "Materials/mask.ylmaterial");
    assert!(s.shelf.resources().is_empty(), "{}", s.message);
    assert!(
        s.message
            .starts_with("Cannot import \"mask\" from the library ("),
        "{}",
        s.message
    );
}

// ───────── 画面（egui_kittest） ─────────

mod ui {
    use super::*;
    use common::*;
    use egui::{pos2, Modifiers, PointerButton, Rect};
    use egui_kittest::kittest::{NodeT, Queryable};
    use egui_kittest::Harness;
    use yolu_app::brushes::BrushAction;
    use yolu_app::state::PopupKind;
    use yolu_app::YoluApp;

    /// ウィンドウに落としたファイル（パスだけ持つ）。
    #[derive(Debug)]
    struct Dropped(PathBuf);
    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            std::fs::read(&self.0).map_err(|e| e.to_string())
        }
    }

    /// 項目を見終わるまで待って描く（見えている項目を頼む→できるまで待つ→描く、を 3 回）。
    fn settle(h: &mut Harness<'_, YoluApp>) {
        for _ in 0..3 {
            h.run();
            h.state_mut().state.shelf.wait_inspections();
            h.state_mut().state.library.wait_probes();
        }
        h.run();
    }

    fn st<'a>(h: &'a Harness<'_, YoluApp>) -> &'a AppState {
        &h.state().state
    }

    /// 「アセット」の格子の中の素材。
    fn card(h: &Harness<'_, YoluApp>, name: &str) -> Rect {
        let body = top_right_body(h);
        rect_of(h, name, |r| {
            body.contains(r.center()) && r.top() > body.top() + 67.0
        })
    }

    fn card_shown(h: &Harness<'_, YoluApp>, name: &str) -> bool {
        let body = top_right_body(h);
        h.query_all_by_label(name)
            .any(|n| body.contains(n.rect().center()) && n.rect().top() > body.top() + 67.0)
    }

    fn row(h: &Harness<'_, YoluApp>, name: &str) -> Rect {
        let body = layers_body(h);
        rect_of(h, name, |r| body.contains(r.center()) && r.height() < 40.0)
    }

    fn double_click(h: &mut Harness<'_, YoluApp>, at: egui::Pos2) {
        for _ in 0..2 {
            press(h, at, PointerButton::Primary);
            release(h, at, PointerButton::Primary);
            h.step();
        }
        h.run();
    }

    fn right_click(h: &mut Harness<'_, YoluApp>, at: egui::Pos2) {
        press(h, at, PointerButton::Secondary);
        h.step();
        release(h, at, PointerButton::Secondary);
        h.run();
    }

    /// 人工のライブラリ（スマートマテリアル 2・スマートマスク・画像・ブラシのファイル・読めないファイル）を見せているウィンドウ。
    fn window(dir: &Dir) -> Harness<'static, YoluApp> {
        dir.put("Smart/raster.ylsmart", &fixture("raster.ylsmart"));
        dir.put("Smart/multi.ylsmart", &fixture("multi.ylsmart"));
        dir.put("Smart/mask.ylsmart", &fixture("mask.ylsmart"));
        dir.put("Images/pic.png", &sample_png());
        dir.put("Brushes/brush.ylbrush", &fixture("brush.ylbrush"));
        dir.put("broken.ylsmart", b"not a smart file");
        let mut h = app(1280.0, 1400.0, 64);
        click_tab(&mut h, yolu_app::Tab::Assets);
        // タブを押した時刻から間を空ける（このあとのダブルクリックの 1 回目が、タブの押下との 2 回押しに数えられないように）
        for _ in 0..30 {
            h.step();
        }
        h.state_mut().state.prefs.settings.library_folder = Some(dir.library());
        h.state_mut().state.shelf.show_builtin = false;
        h.run();
        h.get_by_label("ライブラリ").click();
        settle(&mut h);
        // 押した時刻から間を空ける（このあとのダブルクリックの 1 回目が、この押下との 2 回押しに数えられないように）
        wait(&mut h);
        h
    }

    fn wait(h: &mut Harness<'_, YoluApp>) {
        for _ in 0..30 {
            h.step();
        }
    }

    #[test]
    fn the_source_switch_shows_the_library_and_the_project_and_remembers_the_choice() {
        let dir = Dir::new("ui-switch");
        let mut h = window(&dir);
        assert_eq!(st(&h).library.source, Source::Library);
        for name in ["raster", "multi", "mask", "pic", "broken"] {
            assert!(card_shown(&h, name), "{name}");
        }
        // 「レイヤーを保存」はプロジェクトの棚のもの。ライブラリでは出ない
        assert!(h.query_by_label("レイヤーを保存").is_none());
        h.get_by_label("プロジェクト").click();
        settle(&mut h);
        assert_eq!(st(&h).library.source, Source::Project);
        assert!(!card_shown(&h, "raster"));
        assert!(h.query_by_label("レイヤーを保存").is_some());
        // 英語の画面
        h.state_mut().state.lang = Lang::En;
        h.run();
        h.get_by_label("Library").click();
        settle(&mut h);
        assert!(card_shown(&h, "raster"));
        assert!(h.query_by_label("Reload the list").is_some());
    }

    #[test]
    fn a_library_card_places_from_the_footer_with_one_undo_and_the_shelf_stays_empty() {
        let dir = Dir::new("ui-place");
        let mut h = window(&dir);
        let before = st(&h).doc.layers().len();
        assert!(h.get_by_label("置く").accesskit_node().is_disabled());
        let at = card(&h, "raster").center();
        click(&mut h, at);
        assert_eq!(
            st(&h).library.selected.as_deref(),
            Some("library:Smart/raster.ylsmart")
        );
        h.get_by_label("置く").click();
        h.run();
        assert_eq!(st(&h).doc.layers().len(), before + 1, "{}", st(&h).message);
        assert!(st(&h).shelf.resources().is_empty());
        key(&h, egui::Key::Z, Modifiers::COMMAND);
        h.run();
        assert_eq!(st(&h).doc.layers().len(), before);
        // ダブルクリックでも置く
        wait(&mut h);
        let at = card(&h, "multi").center();
        double_click(&mut h, at);
        assert!(st(&h).doc.layers().len() > before, "{}", st(&h).message);
        // スマートマスクは「マスクに適用」
        let at = card(&h, "mask").center();
        click(&mut h, at);
        assert!(h.query_by_label("マスクに適用").is_some());
        assert!(h.query_by_label("置く").is_none());
    }

    #[test]
    fn a_broken_file_is_marked_and_cannot_be_placed_or_used() {
        let dir = Dir::new("ui-broken");
        let mut h = window(&dir);
        let at = card(&h, "broken").center();
        click(&mut h, at);
        assert!(h.get_by_label("置く").accesskit_node().is_disabled());
        assert!(h
            .get_by_label("プロジェクトで使う（写しを .ylp に入れる）")
            .accesskit_node()
            .is_disabled());
        // 消すことはできる
        assert!(!h
            .get_by_label("ライブラリから消す（プロジェクトの写しと置いたレイヤーはそのまま）")
            .accesskit_node()
            .is_disabled());
        let info = st(&h).library.info("broken.ylsmart").unwrap();
        assert!(matches!(info.inspected.block, Some(Block::Unreadable(_))));
    }

    #[test]
    fn the_footer_icons_use_in_the_project_ask_before_removing_and_open_the_folder() {
        let dir = Dir::new("ui-footer");
        let mut h = window(&dir);
        let at = card(&h, "raster").center();
        click(&mut h, at);
        // プロジェクトで使う（別のスレッドで読んで棚へ入る）
        h.get_by_label("プロジェクトで使う（写しを .ylp に入れる）")
            .click();
        h.run();
        h.state_mut().state.shelf_wait();
        h.run();
        assert_eq!(st(&h).shelf.resources().len(), 1, "{}", st(&h).message);
        settle(&mut h);
        // プロジェクトにあるしるし（チェック）と、ツールチップの文
        assert!(st(&h)
            .library
            .in_project(st(&h).shelf.shelf(), "Smart/raster.ylsmart"));
        // 消す前に確かめる
        h.get_by_label("ライブラリから消す（プロジェクトの写しと置いたレイヤーはそのまま）")
            .click();
        h.run();
        assert_eq!(st(&h).dialog_request, Some(DialogRequest::LibraryRemove));
        assert_eq!(
            st(&h).library.pending_remove.as_deref(),
            Some("Smart/raster.ylsmart")
        );
        assert!(dir.exists("Smart/raster.ylsmart"), "確かめる前は消さない");
        h.state_mut().state.dialog_request = None;
        // フォルダを開く（ウィンドウを頼む。上の帯と下の帯の 2 か所にある）
        let nodes: Vec<_> = h.get_all_by_label("ライブラリのフォルダを開く").collect();
        assert_eq!(nodes.len(), 2);
        nodes
            .into_iter()
            .max_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
            .unwrap()
            .click();
        h.run();
        assert_eq!(st(&h).dialog_request, Some(DialogRequest::LibraryReveal));
    }

    #[test]
    fn the_context_menu_of_a_library_card_names_its_actions_in_both_languages() {
        use yolu_app::ui::menu::Entry;
        let dir = Dir::new("ui-menu");
        let mut h = window(&dir);
        let at = card(&h, "raster").center();
        right_click(&mut h, at);
        let labels = |h: &Harness<'_, YoluApp>| -> Vec<(String, bool)> {
            yolu_app::shell::popup_entries(st(h), PopupKind::Shelf)
                .into_iter()
                .filter_map(|e| match e {
                    Entry::Item { label, enabled, .. } => Some((label, enabled)),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(
            labels(&h),
            [
                ("置く".to_owned(), true),
                ("プロジェクトで使う".to_owned(), true),
                ("ライブラリのフォルダを開く".to_owned(), true),
                ("ライブラリから消す…".to_owned(), true),
            ]
        );
        // 開いているメニューは、次の押下で閉じる（その押下は選ばない）
        h.state_mut().state.popup = None;
        h.run();
        let at = card(&h, "mask").center();
        click(&mut h, at);
        assert_eq!(labels(&h)[0].0, "マスクに適用");
        let at = card(&h, "brush").center();
        click(&mut h, at);
        assert_eq!(
            labels(&h)[0].0,
            "使う",
            "ブラシのファイルはまだ置けない種類"
        );
        let at = card(&h, "broken").center();
        click(&mut h, at);
        assert_eq!(
            labels(&h).iter().map(|l| l.1).collect::<Vec<_>>(),
            [false, false, true, true],
            "読めないファイルは置けず・使えず、消せる"
        );
        h.state_mut().state.lang = Lang::En;
        h.run();
        let at = card(&h, "raster").center();
        click(&mut h, at);
        assert_eq!(
            labels(&h).into_iter().map(|l| l.0).collect::<Vec<_>>(),
            [
                "Place",
                "Use in this project",
                "Open the library folder",
                "Remove from the library…"
            ]
        );
        // メニューの「プロジェクトで使う」は棚へ入れる
        let popup = yolu_app::shell::popup_entries(st(&h), PopupKind::Shelf);
        let use_it = popup
            .into_iter()
            .find_map(|e| match e {
                Entry::Item { label, action, .. } if label == "Use in this project" => Some(action),
                _ => None,
            })
            .unwrap();
        h.state_mut().state.apply(use_it);
        h.state_mut().state.shelf_wait();
        assert_eq!(st(&h).shelf.resources().len(), 1);
    }

    #[test]
    fn user_brushes_are_listed_with_their_names_and_made_current_from_the_library() {
        let dir = Dir::new("ui-brush");
        let mut h = window(&dir);
        h.state_mut().state.apply(Action::Brush(BrushAction::Add));
        h.run();
        let (key, name) = st(&h)
            .brushes
            .lib
            .entries()
            .iter()
            .find(|e| e.key.is_user())
            .map(|e| (e.key, e.name.clone()))
            .expect("追加したブラシ");
        // 今のブラシを標準へ戻しておく
        h.state_mut().state.apply(Action::Brush(BrushAction::Select(
            yolu_app::brushes::BrushKey::Builtin(yolu_app::brushes::builtin::STANDARD),
        )));
        settle(&mut h);
        assert!(card_shown(&h, &name), "{name}");
        assert_ne!(st(&h).brushes.lib.current(), key);
        let at = card(&h, &name).center();
        click(&mut h, at);
        h.get_by_label("使う").click();
        h.run();
        assert_eq!(st(&h).brushes.lib.current(), key);
        // 種類の絞り込みのブラシ（ファイルのブラシと一緒）。素材の絞り込みでは出ない
        h.get_by_label("スマートマテリアル").click();
        settle(&mut h);
        assert!(!card_shown(&h, &name));
        // ダブルクリックでも今のブラシにする
        h.state_mut().state.apply(Action::Brush(BrushAction::Select(
            yolu_app::brushes::BrushKey::Builtin(yolu_app::brushes::builtin::STANDARD),
        )));
        h.get_by_label("すべて").click();
        settle(&mut h);
        wait(&mut h);
        let at = card(&h, &name).center();
        double_click(&mut h, at);
        assert_eq!(st(&h).brushes.lib.current(), key);
    }

    #[test]
    fn dragging_a_library_card_onto_the_layer_list_places_it_between_rows() {
        let dir = Dir::new("ui-drag");
        let mut h = window(&dir);
        for _ in 0..2 {
            h.state_mut().state.apply(Action::NewLayer);
            h.run();
        }
        let from = card(&h, "multi").center();
        let r2 = row(&h, "レイヤー 2");
        let to = pos2(r2.left() + 120.0, r2.top() + r2.height() * 0.8);
        press(&h, from, PointerButton::Primary);
        h.step();
        for p in [offset(from, 30.0, 10.0), offset(to, 0.0, -60.0), to] {
            move_to(&h, p);
            h.step();
        }
        h.run();
        release(&h, to, PointerButton::Primary);
        h.run();
        let names: Vec<String> = st(&h)
            .doc
            .layers()
            .iter()
            .map(|l| l.name().to_owned())
            .collect();
        assert!(names.len() > 3, "{names:?} {}", st(&h).message);
        assert!(
            st(&h).message.starts_with("置きました: multi"),
            "{}",
            st(&h).message
        );
        assert!(st(&h).shelf.resources().is_empty());
    }

    #[test]
    fn a_big_library_looks_only_at_the_cards_on_screen_and_more_as_it_is_scrolled() {
        let dir = Dir::new("ui-big");
        for i in 0..400u32 {
            dir.put(
                &format!("Images/{i:04}.png"),
                &png(4, 4, |x, y| {
                    [(i % 251) as u8, x as u8 * 60, y as u8 * 60, 255]
                }),
            );
        }
        let mut h = app(1280.0, 1000.0, 64);
        click_tab(&mut h, yolu_app::Tab::Assets);
        h.state_mut().state.prefs.settings.library_folder = Some(dir.library());
        h.state_mut().state.shelf.show_builtin = false;
        h.run();
        h.get_by_label("ライブラリ").click();
        settle(&mut h);
        assert_eq!(st(&h).library.visible(None, "").len(), 400);
        // 画面に収まる枚数（と先読みの 1 行）だけ見ている
        let known = st(&h).library.known_count();
        assert!(known > 0 && known < 60, "{known}");
        assert!(st(&h).library.info("Images/0399.png").is_none());
        // スクロールすると、見えるようになった所を見る
        h.state_mut().state.shelf.scroll = 1.0e6;
        settle(&mut h);
        assert!(st(&h).library.info("Images/0399.png").is_some());
        assert!(
            st(&h).library.info("Images/0000.png").is_none() || st(&h).library.known_count() < 160
        );
        // 名前で絞ると、その分だけ
        h.state_mut().state.shelf.search = "0123".into();
        h.state_mut().state.shelf.scroll = 0.0;
        settle(&mut h);
        assert!(st(&h).library.info("Images/0123.png").is_some());
    }

    /// 測定。大きな画像を並べたライブラリを見せている間の、1 フレームの長さ（画面のスレッドが仕事の終わりを待たないこと）。
    #[test]
    #[ignore = "測定。--ignored --nocapture で回す"]
    fn measure_frames_while_a_library_of_big_images_is_being_looked_at() {
        let dir = Dir::new("ui-measure");
        for i in 0..40u32 {
            dir.put(
                &format!("Images/{i:03}.png"),
                &png(2048, 2048, |x, y| {
                    let n = x.wrapping_mul(2654435761) ^ y.wrapping_mul(40503) ^ i;
                    [n as u8, (n >> 8) as u8, (n >> 16) as u8, 255]
                }),
            );
        }
        let mut h = app(1280.0, 1000.0, 64);
        click_tab(&mut h, yolu_app::Tab::Assets);
        h.state_mut().state.prefs.settings.library_folder = Some(dir.library());
        h.state_mut().state.shelf.show_builtin = false;
        h.run();
        h.get_by_label("ライブラリ").click();
        let started = std::time::Instant::now();
        let mut frames = Vec::new();
        while st(&h).library.known_count() < 12 && started.elapsed().as_secs() < 120 {
            let t = std::time::Instant::now();
            h.step();
            frames.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        let total = started.elapsed().as_secs_f64();
        frames.sort_by(|a, b| a.total_cmp(b));
        println!(
            "2048² の PNG 40 枚のライブラリ: 見えている 12 枚ができるまで {total:.2} 秒・{} フレーム。1 フレームは中央値 {:.2} ミリ秒・最大 {:.2} ミリ秒",
            frames.len(),
            frames[frames.len() / 2],
            frames[frames.len() - 1]
        );
    }

    /// ポインタを `at` に置いてから、ファイルをウィンドウに落とす（落とした時点のポインタの位置で、落とし先が決まる）。
    fn drop_at(h: &mut Harness<'_, YoluApp>, at: egui::Pos2, files: &[&Path]) {
        move_to(h, at);
        h.step();
        for f in files {
            h.input_mut()
                .dropped_files
                .push(Arc::new(Dropped(f.to_path_buf())));
        }
        h.run();
    }

    /// ウィンドウに落とす PNG と、PNG でないファイル。
    fn files_to_drop(dir: &Dir, png_name: &str) -> (PathBuf, PathBuf) {
        let outside = dir.0.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let png_path = outside.join(png_name);
        std::fs::write(
            &png_path,
            png(2, 2, |x, y| [x as u8 * 90, y as u8 * 90, 5, 255]),
        )
        .unwrap();
        let notes = outside.join("notes.txt");
        std::fs::write(&notes, b"x").unwrap();
        (png_path, notes)
    }

    #[test]
    fn files_dropped_on_the_library_grid_go_to_the_library() {
        let dir = Dir::new("ui-drop");
        let mut h = window(&dir);
        let grid = st(&h).library.grid_rect.expect("格子を描いている");
        let (png_path, notes) = files_to_drop(&dir, "dropped.png");
        drop_at(&mut h, grid.center(), &[&png_path, &notes]);
        h.state_mut().state.library_wait();
        assert!(dir.exists("Images/dropped.png"), "{}", st(&h).message);
        assert!(
            st(&h).shelf.resources().is_empty(),
            "プロジェクトの棚へは入れない"
        );
        // プロジェクトを見せている間は、PNG は取り込まない
        h.get_by_label("プロジェクト").click();
        h.run();
        assert!(st(&h).library.grid_rect.is_none());
        let again = dir.0.join("outside/again.png");
        std::fs::write(&again, png(2, 2, |_, _| [1, 2, 3, 255])).unwrap();
        drop_at(&mut h, pos2(150.0, 400.0), &[&again]);
        assert!(!dir.exists("Images/again.png"));
    }

    #[test]
    fn a_png_dropped_outside_the_library_grid_is_not_written_to_the_library() {
        let dir = Dir::new("ui-drop-outside");
        let mut h = window(&dir);
        let grid = st(&h).library.grid_rect.expect("格子を描いている");
        let before = dir.files();
        let (png_path, _) = files_to_drop(&dir, "tip.png");
        // ウィンドウのほかの場所（格子の外）へ落とした PNG は、筆先・ステンシルなどほかの落とし先のもの
        for at in [
            pos2(grid.right() + 40.0, grid.center().y),
            pos2(grid.center().x, grid.top() - 8.0),
            pos2(640.0, 500.0),
        ] {
            assert!(!grid.contains(at), "{at:?}");
            drop_at(&mut h, at, &[&png_path]);
            h.state_mut().state.library_wait();
            assert_eq!(dir.files(), before, "{at:?}");
        }
        // ブラシのタブを開いている間（ライブラリの置き場のまま）、ブラシの一覧の上へ落とした PNG は筆先の取り込みへ行き、
        // ライブラリには何も書かない
        // （アセットの組の前をチャンネルにして、ライブラリの格子を隠す）
        click_tab(&mut h, yolu_app::Tab::Channels);
        h.run();
        assert_eq!(st(&h).library.source, Source::Library);
        assert!(st(&h).library.grid_rect.is_none(), "格子は描いていない");
        h.state_mut()
            .state
            .attach_brush_store(dir.0.join("brush-store"));
        h.run();
        let brushes = st(&h).brushes.lib.user_count();
        let list = st(&h)
            .brushes
            .ui
            .list_rect
            .expect("ブラシの一覧を描いている");
        drop_at(&mut h, list.center(), &[&png_path]);
        let started = std::time::Instant::now();
        while st(&h).is_brush_importing() && started.elapsed().as_secs() < 60 {
            h.run();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        h.state_mut().state.library_wait();
        assert_eq!(dir.files(), before, "ライブラリには何も書かない");
        assert_eq!(
            st(&h).brushes.lib.user_count(),
            brushes + 1,
            "筆先として取り込まれる: {}",
            st(&h).message
        );
        // 棚のタブへ戻って、前の位置（ブラシの一覧があった所）へ落としても、格子の外は書かない
        click_tab(&mut h, yolu_app::Tab::Assets);
        h.run();
        let grid = st(&h).library.grid_rect.expect("格子を描いている");
        drop_at(
            &mut h,
            pos2(grid.left() + 4.0, grid.top() - 30.0),
            &[&png_path],
        );
        assert_eq!(dir.files(), before);
    }

    #[test]
    fn a_ylsmart_dropped_outside_the_library_grid_goes_to_the_project_shelf_as_before() {
        let dir = Dir::new("ui-drop-smart");
        let mut h = window(&dir);
        let grid = st(&h).library.grid_rect.expect("格子を描いている");
        let before = dir.files();
        let smart = dir.0.join("outside-smart.ylsmart");
        std::fs::write(&smart, fixture("raster.ylsmart")).unwrap();
        // 格子の外へ落とした .ylsmart はプロジェクトの棚へ（ライブラリには書かない）
        drop_at(&mut h, pos2(640.0, 500.0), &[&smart]);
        h.state_mut().state.shelf_wait();
        assert_eq!(dir.files(), before);
        assert_eq!(st(&h).shelf.resources().len(), 1, "{}", st(&h).message);
        // 格子の上へ落とした .ylsmart はライブラリへ
        let other = dir.0.join("outside-filtered.ylsmart");
        std::fs::write(&other, fixture("filtered.ylsmart")).unwrap();
        drop_at(&mut h, grid.center(), &[&other]);
        h.state_mut().state.library_wait();
        let after = dir.files();
        assert_eq!(
            after.len(),
            before.len() + 1,
            "{after:?}\n{}",
            st(&h).message
        );
        assert!(
            after
                .iter()
                .any(|f| !before.contains(f) && f.starts_with("Smart/")),
            "{after:?}"
        );
    }

    #[test]
    fn the_mask_filter_shows_masks_in_a_library_that_was_never_looked_at() {
        let dir = Dir::new("ui-mask-filter");
        dir.put("Smart/raster.ylsmart", &fixture("raster.ylsmart"));
        dir.put("Smart/multi.ylsmart", &fixture("multi.ylsmart"));
        dir.put("Smart/mask.ylsmart", &fixture("mask.ylsmart"));
        dir.put("Images/pic.png", &sample_png());
        let mut h = app(1280.0, 1000.0, 64);
        click_tab(&mut h, yolu_app::Tab::Assets);
        h.state_mut().state.prefs.settings.library_folder = Some(dir.library());
        h.state_mut().state.shelf.show_builtin = false;
        // 絞り込みを先に決めておく（一覧を初めて見るときから、スマートマスクだけ）
        h.state_mut().state.shelf.filter = Some(ItemKind::SmartMask);
        h.run();
        h.get_by_label("ライブラリ").click();
        settle(&mut h);
        assert!(card_shown(&h, "mask"));
        for other in ["raster", "multi", "pic"] {
            assert!(!card_shown(&h, other), "{other}");
        }
        h.state_mut().state.shelf.filter = Some(ItemKind::SmartMaterial);
        settle(&mut h);
        assert!(card_shown(&h, "raster") && card_shown(&h, "multi"));
        assert!(!card_shown(&h, "mask"));
    }

    #[test]
    fn a_write_in_progress_shows_its_name_and_can_be_cancelled_from_the_panel() {
        let dir = Dir::new("ui-write");
        let mut h = window(&dir);
        let before = dir.files();
        h.get_by_label("プロジェクト").click();
        settle(&mut h);
        h.state_mut().state.shelf = ShelfState::with_shelf(fixture_shelf());
        h.state_mut().state.shelf.show_builtin = false;
        settle(&mut h);
        h.state_mut().state.library.hold_writes(true);
        let at = card(&h, "raster").center();
        click(&mut h, at);
        h.get_by_label("ライブラリへ入れる").click();
        h.run();
        assert_eq!(st(&h).library.writing_name(), Some("raster"));
        assert!(
            h.query_by_label("レイヤーを保存").is_none(),
            "保存のボタンは名前とやめるに替わる"
        );
        h.get_by_label("やめる").click();
        h.run();
        assert!(st(&h).library.writing_name().is_none());
        h.state_mut().state.library.hold_writes(false);
        h.state_mut().state.library.wait_idle();
        h.run();
        assert_eq!(dir.files(), before, "やめた書き込みは何も残さない");
        assert!(h.query_by_label("レイヤーを保存").is_some());
    }

    #[test]
    fn a_library_location_that_cannot_be_read_is_named_in_the_grid() {
        let dir = Dir::new("ui-problem");
        let mut h = window(&dir);
        std::fs::write(dir.0.join("not-a-folder"), b"x").unwrap();
        h.state_mut().state.prefs.settings.library_folder = Some(dir.0.join("not-a-folder"));
        settle(&mut h);
        // 状態の文字は格子に描き、理由はツールチップ（読めなかった理由は状態に残る）
        let why = st(&h).library.problem().expect("フォルダでない");
        assert!(why.reason(Lang::Ja).contains("フォルダ"));
        assert!(!card_shown(&h, "raster"));
        assert_eq!(st(&h).library.visible(None, "").len(), 0);
    }

    #[test]
    fn snapshot_library() {
        let dir = Dir::new("ui-snap");
        let mut h = window(&dir);
        // プロジェクトにもあるしるし（チェック）と、読めないファイルの警告
        h.state_mut()
            .state
            .apply(Action::Shelf(ShelfOp::UseFromLibrary(
                "Smart/multi.ylsmart".into(),
            )));
        h.state_mut().state.shelf_wait();
        settle(&mut h);
        let at = card(&h, "raster").center();
        click(&mut h, at);
        settle(&mut h);
        h.snapshot("assets_library");
        h.state_mut().state.lang = Lang::En;
        h.state_mut().state.shelf.filter = Some(ItemKind::SmartMaterial);
        let at = card(&h, "broken").center();
        click(&mut h, at);
        settle(&mut h);
        h.snapshot("assets_library_english");
    }

    /// 塗りつぶしレイヤーを「マテリアルとして保存」したあと: ライブラリのマテリアルのカードを選び、そのレイヤーの右クリックのメニューを開いた所。
    /// アセットの画像を使う塗りつぶしでは、項目を押せず、理由がツールチップに出る（日英）。
    #[test]
    fn snapshot_material_menu_and_card() {
        let mut results = egui_kittest::SnapshotResults::new();
        for lang in Lang::ALL {
            let suffix = lang.pick("ja", "en");
            let dir = Dir::new("ui-material");
            let mut h = window(&dir);
            let name = lang.pick("鉄", "Iron");
            {
                let s = &mut h.state_mut().state;
                s.lang = lang;
                let fill = fill_layer(s, name);
                s.apply(Action::Shelf(ShelfOp::SaveAsMaterial(fill)));
                s.library_wait();
            }
            settle(&mut h);
            let at = card(&h, name).center();
            click(&mut h, at);
            settle(&mut h);
            assert!(st(&h).library.selected.is_some());
            let at = row(&h, name).center();
            right_click(&mut h, at);
            settle(&mut h);
            let save = lang.pick("マテリアルとして保存", "Save as Material");
            assert!(!popup_item(&h, save).is_negative());
            h.snapshot(format!("assets_material_{suffix}"));
            results.extend_harness(&mut h);
            key(&h, egui::Key::Escape, Modifiers::NONE);
            h.run();
            // アセットの画像を使う塗りつぶし
            let textured = lang.pick("絵", "Picture");
            {
                let s = &mut h.state_mut().state;
                let rid = s
                    .shelf
                    .add_image(lang, textured, &[200, 120, 40, 255], 1, 1)
                    .unwrap();
                let layer = fill_layer(s, textured);
                s.apply(Action::Fill(yolu_app::fillfx::FillOp::Image {
                    layer,
                    channel: Channel::Color,
                    image: yolu_app::fillfx::inputs::image_id(&rid),
                }));
            }
            settle(&mut h);
            let at = row(&h, textured).center();
            right_click(&mut h, at);
            settle(&mut h);
            let at = popup_item(&h, save).center();
            hover_and_wait(&mut h, at);
            h.snapshot(format!("assets_material_images_{suffix}"));
            results.extend_harness(&mut h);
        }
    }

    #[test]
    fn snapshot_library_unreadable_and_empty() {
        let dir = Dir::new("ui-snap-empty");
        let mut h = window(&dir);
        h.state_mut().state.prefs.settings.library_folder = Some(dir.0.join("nothing-here"));
        settle(&mut h);
        h.snapshot("assets_library_empty");
        std::fs::write(dir.0.join("file"), b"x").unwrap();
        h.state_mut().state.prefs.settings.library_folder = Some(dir.0.join("file"));
        settle(&mut h);
        h.snapshot("assets_library_unreadable");
    }
}
