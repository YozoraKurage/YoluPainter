//! ディスクキャッシュ（`yolu_core::tile_cache`）の入ったアプリ: タイルの中身をディスクへ逃がしても、セットの切り替え・保存して開き直す・
//! 書き出しが同じバイトになる。キャッシュのファイルは名前を残さない。読めないキャッシュは、そのセットを読むだけにして、保存で中身を
//! 書き換えない（開いたときの中身のまま）。
//!
//! 逃がすのはプロセス全体の係（`tile_cache::evict_now`）なので、同じ束のほかの試験のタイルも逃がす（逃がしても同じバイトになるのが約束）。
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use yolu_app::engine::{Channel, DVec2, Document, TileCoord};
use yolu_app::export::ExportAction;
use yolu_app::lang::Lang;
use yolu_app::newproject::NpAction;
use yolu_app::recovery::{DiskSpace, RecoverySettings, SpaceProbe};
use yolu_app::state::{Action, AppState};
use yolu_core::tile_cache::{self, CacheSettings};
use yolu_core::CoreError;
use yolu_io::NativeDocument;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> TempDir {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "yolu-app-diskcache-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 読んでいない中身を全部ディスクへ逃がす（置き場所は試験のフォルダ。裏の書き手は起こさない）。
fn evict_all(folder: &Path) {
    tile_cache::configure(&CacheSettings {
        enabled: false,
        folder: Some(folder.to_path_buf()),
        memory_limit: u64::MAX,
        disk_limit: 1 << 40,
    });
    tile_cache::evict_now(0);
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

/// 層を 2 枚足して、全タイルに違う画素を入れる。
fn fill(doc: &mut Document, seed: u64) {
    let ts = doc.tile_size();
    let (nx, ny) = (doc.width().div_ceil(ts), doc.height().div_ceil(ts));
    for i in 0..2u64 {
        let id = doc.add_layer(&format!("層 {i}")).unwrap();
        for ty in 0..ny {
            for tx in 0..nx {
                let s = seed * 1000 + i * 101 + (ty * nx + tx) as u64;
                let bytes = noise((ts * ts * 4) as usize, s);
                doc.import_tile(id, Channel::Color, TileCoord::new(tx, ty), &bytes)
                    .unwrap();
            }
        }
    }
}

fn bytes_of(doc: &Document) -> Vec<u8> {
    NativeDocument::from_core(doc).unwrap().to_bytes()
}

/// セットごとの、ディスクにだけある（メモリに無い）全画素のタイルの数。
fn on_disk(doc: &Document) -> usize {
    doc.layers()
        .iter()
        .filter_map(|l| l.surface(Channel::Color))
        .map(|s| s.evicted_tile_count())
        .sum()
}

fn opened(path: &Path) -> AppState {
    let mut s = AppState::new_in(64, 64, Lang::Ja);
    s.apply(Action::OpenProject(path.to_path_buf()));
    assert!(s.message.starts_with("開きました"), "{}", s.message);
    s
}

fn paint(s: &mut AppState, x: f64) {
    let layer = s.selected_layer.unwrap();
    let brush = s.stroke_settings(false);
    let mut stroke = s.doc.begin_stroke(layer, &brush).unwrap();
    stroke
        .add_point(&mut s.doc, x, 20.0, 1.0, DVec2::ZERO)
        .unwrap();
    s.doc.end_stroke(stroke).unwrap();
    s.modified = true;
}

#[test]
fn tiles_on_disk_give_the_same_bytes_when_switching_sets_saving_reopening_and_exporting() {
    let dir = TempDir::new("same");
    let cache = TempDir::new("same-cache");
    let mut s = AppState::new_in(256, 256, Lang::Ja);
    s.apply(Action::Project(NpAction::AddSet));
    assert_eq!(s.sets.len(), 2, "{}", s.message);
    for i in 0..2 {
        fill(s.set_doc_mut(i), 10 + i as u64);
    }
    s.modified = true;
    let expected: Vec<Vec<u8>> = (0..2).map(|i| bytes_of(s.set_doc(i))).collect();
    let composite = s.doc.composite(s.doc.bounds()).unwrap();
    let png = dir.file("before.png");
    s.apply(Action::Export(ExportAction::ChannelTo(png.clone())));
    s.wait_export();
    let exported = std::fs::read(&png).unwrap();

    // セットを切り替える（しまった文書のタイルも、今の文書のタイルもディスクにある）
    evict_all(&cache.0);
    assert!(
        (0..2).all(|i| on_disk(s.set_doc(i)) > 0),
        "どちらのセットもディスクへ"
    );
    let current = s.sets.current_index();
    s.switch_set(1 - current).unwrap();
    evict_all(&cache.0);
    s.switch_set(current).unwrap();
    evict_all(&cache.0);
    assert_eq!(s.doc.composite(s.doc.bounds()).unwrap(), composite);
    for (i, want) in expected.iter().enumerate() {
        evict_all(&cache.0);
        assert!(bytes_of(s.set_doc(i)) == *want, "セット {i} の画素");
    }
    // 書き出す
    evict_all(&cache.0);
    let after = dir.file("after.png");
    s.apply(Action::Export(ExportAction::ChannelTo(after.clone())));
    s.wait_export();
    assert!(std::fs::read(&after).unwrap() == exported, "書き出した PNG");
    // 保存して開き直す
    evict_all(&cache.0);
    let path = dir.file("作品.ylp");
    s.apply(Action::SaveProjectAs(path.clone()));
    s.wait_save();
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let back = opened(&path);
    assert_eq!(back.sets.len(), 2);
    for (i, want) in expected.iter().enumerate() {
        assert!(bytes_of(back.set_doc(i)) == *want, "開き直したセット {i}");
    }
    // キャッシュのファイルは開いている間も名前を残さない（Unix は開いた直後に消す。Windows は閉じたら消える印で開く）
    if cfg!(unix) {
        let created = tile_cache::created_files();
        assert!(!created.is_empty());
        assert!(created.iter().all(|p| !p.exists()), "{created:?}");
        let names: Vec<_> = std::fs::read_dir(&cache.0).unwrap().flatten().collect();
        assert!(names.is_empty(), "置き場所に何も残らない");
    }
}

#[test]
fn an_unreadable_cache_makes_the_set_read_only_and_saving_keeps_what_was_opened() {
    let dir = TempDir::new("unreadable");
    let cache = TempDir::new("unreadable-cache");
    let mut s = AppState::new_in(256, 128, Lang::Ja);
    fill(&mut s.doc, 3);
    s.modified = true;
    let path = dir.file("作品.ylp");
    s.apply(Action::SaveProjectAs(path.clone()));
    s.wait_save();
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let original = bytes_of(&opened(&path).doc);
    // 描いて変えたあと、そのタイルをディスクへ逃がし、キャッシュを読めなくする
    paint(&mut s, 30.0);
    evict_all(&cache.0);
    for l in s.doc.layers() {
        if let Some(surface) = l.surface(Channel::Color) {
            surface.fail_tile_reads_for_test();
        }
    }
    assert_eq!(
        s.doc.composite(s.doc.bounds()).map(|_| ()),
        Err(CoreError::TileUnreadable)
    );
    assert!(s.read_only_reason().is_none());
    s.check_tile_cache();
    let reason = s.read_only_reason().expect("読むだけ").to_owned();
    assert!(reason.contains("キャッシュ"), "{reason}");
    // 知らせはどのセットか・最後に保存した後の編集は保存できないことを言う
    let name = s.sets.current().name.clone();
    assert!(s.message.contains("読むだけ"), "{}", s.message);
    assert!(s.message.contains(&format!("「{name}」")), "{}", s.message);
    assert!(
        s.message.contains("最後に保存した後の編集"),
        "{}",
        s.message
    );
    assert!(!s.can_edit());
    // 保存しても、読むだけのセットは開いたときの中身のまま（欠けた中身を書かない）
    s.apply(Action::SaveProject);
    s.wait_save();
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    assert!(
        bytes_of(&opened(&path).doc) == original,
        "保存した中身は開いたときのまま"
    );
    // 保存したことの無いセットは、元の中身が無いので保存を断る
    let mut fresh = AppState::new_in(256, 128, Lang::En);
    fill(&mut fresh.doc, 4);
    fresh.modified = true;
    evict_all(&cache.0);
    for l in fresh.doc.layers() {
        if let Some(surface) = l.surface(Channel::Color) {
            surface.fail_tile_reads_for_test();
        }
    }
    assert!(fresh.doc.composite(fresh.doc.bounds()).is_err());
    fresh.check_tile_cache();
    assert!(fresh.read_only_reason().is_some());
    // 保存したことの無いセットは、最後の保存の後の編集ではなく、プロジェクトが保存できないことを言う
    assert!(
        fresh.message.contains("is now read-only")
            && fresh.message.contains("has never been saved")
            && !fresh.message.contains("Edits since the last save"),
        "{}",
        fresh.message
    );
    let other = dir.file("新しい.ylp");
    fresh.apply(Action::SaveProjectAs(other.clone()));
    fresh.wait_save();
    assert!(!other.exists(), "{}", fresh.message);
}

/// 空きがたっぷりあるディスク（復旧の置き場のディスクの本当の空きに左右されないように）。
fn plenty() -> SpaceProbe {
    Arc::new(|_| {
        Some(DiskSpace {
            total: 1 << 40,
            available: 1 << 39,
        })
    })
}

/// 復旧用の書き置きを 1 回頼んで、書き込みが終わるまで待つ。
fn checkpoint(s: &mut AppState) {
    let from = Instant::now();
    s.recovery_tick_at(from);
    s.recovery_tick_at(from + Duration::from_secs(16));
    s.recovery_wait();
}

/// そのセットの全層をディスクへ逃がし、読めなくして、読もうとして失敗させる（読めないタイルを持つ印が付く）。
fn make_unreadable(s: &AppState, set: usize, cache: &Path) {
    evict_all(cache);
    for l in s.set_doc(set).layers() {
        if let Some(surface) = l.surface(Channel::Color) {
            surface.fail_tile_reads_for_test();
        }
    }
    assert!(s.set_doc(set).composite(s.set_doc(set).bounds()).is_err());
}

#[test]
fn a_set_never_saved_that_becomes_unreadable_stops_the_whole_project_from_saving_and_the_notice_says_so(
) {
    let dir = TempDir::new("unsaved-set");
    let cache = TempDir::new("unsaved-set-cache");
    let mut s = AppState::new_in(256, 128, Lang::Ja);
    s.recovery.set_space_probe(Some(plenty()));
    s.recovery
        .enable(
            dir.file("recovery"),
            RecoverySettings {
                directory: None,
                ..RecoverySettings::default()
            },
        )
        .unwrap();
    s.apply(Action::Project(NpAction::AddSet));
    assert_eq!(s.sets.len(), 2, "{}", s.message);
    for i in 0..2 {
        fill(s.set_doc_mut(i), 20 + i as u64);
    }
    s.modified = true;
    // 2 つ目のセットだけ読めなくなる（プロジェクトは 1 度も保存していない）
    make_unreadable(&s, 1, &cache.0);
    assert!(!s.set_doc(0).has_unreadable_tiles());
    s.check_tile_cache();
    assert!(s.sets.get(0).unwrap().read_only.is_none());
    assert!(s.sets.get(1).unwrap().read_only.is_some());
    let name = s.sets.get(1).unwrap().name.clone();
    assert!(
        s.message
            .contains(&format!("「{name}」は保存したことが無い")),
        "{}",
        s.message
    );
    assert!(
        s.message.contains("保存も復旧用の書き置きもできません"),
        "{}",
        s.message
    );
    assert!(
        !s.message.contains("最後に保存した後の編集"),
        "{}",
        s.message
    );
    // 言ったとおり、読めるセットを含めて保存は断られ、何も書かれない
    let path = dir.file("新しい.ylp");
    s.apply(Action::SaveProjectAs(path.clone()));
    s.wait_save();
    assert!(!path.exists(), "{}", s.message);
    assert!(s.message.contains("元の文書がありません"), "{}", s.message);
    // 復旧用の書き置きも、全体が断られる
    checkpoint(&mut s);
    assert!(
        s.message.starts_with("復旧用の書き置きに失敗"),
        "{}",
        s.message
    );
    assert_eq!(s.recovery.checkpoints(), 0);
}

#[test]
fn saved_sets_keep_what_was_opened_and_only_the_new_set_is_said_to_block_saving() {
    let dir = TempDir::new("mixed-sets");
    let cache = TempDir::new("mixed-sets-cache");
    let mut s = AppState::new_in(256, 128, Lang::En);
    s.apply(Action::Project(NpAction::AddSet));
    for i in 0..2 {
        fill(s.set_doc_mut(i), 30 + i as u64);
    }
    s.modified = true;
    let path = dir.file("work.ylp");
    s.apply(Action::SaveProjectAs(path.clone()));
    s.wait_save();
    assert!(s.message.starts_with("Saved"), "{}", s.message);
    // 保存の後に、3 つ目のセットを追加する
    s.apply(Action::Project(NpAction::AddSet));
    assert_eq!(s.sets.len(), 3, "{}", s.message);
    fill(s.set_doc_mut(2), 32);
    // 保存済みの 1 つ目と、保存したことの無い 3 つ目が読めなくなる
    make_unreadable(&s, 0, &cache.0);
    make_unreadable(&s, 2, &cache.0);
    s.check_tile_cache();
    let (saved, new) = (
        s.sets.get(0).unwrap().name.clone(),
        s.sets.get(2).unwrap().name.clone(),
    );
    assert!(
        s.message
            .contains(&format!("\"{saved}\", \"{new}\" is now read-only")),
        "{}",
        s.message
    );
    assert!(
        s.message
            .contains("Edits since the last save cannot be saved"),
        "{}",
        s.message
    );
    assert!(
        s.message.contains(&format!(
            "\"{new}\" has never been saved, so the project cannot be saved or checkpointed"
        )),
        "{}",
        s.message
    );
    // 保存したことの無い読むだけのセットがあるので、保存全体が断られ、保存済みのファイルはそのまま
    let before = std::fs::read(&path).unwrap();
    s.apply(Action::SaveProject);
    s.wait_save();
    assert!(
        s.message.contains("Original document missing"),
        "{}",
        s.message
    );
    assert!(std::fs::read(&path).unwrap() == before);
}
