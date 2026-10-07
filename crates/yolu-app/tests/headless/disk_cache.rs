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
use yolu_app::recovery::{DiskSpace, RecoveryAction, RecoverySettings, SpaceProbe};
use yolu_app::state::{Action, AppState};
use yolu_core::tile_cache::{self, CacheSettings};
use yolu_core::CoreError;
use yolu_io::{GenerationStore, NativeDocument, Project, INFO_NAME};

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
pub(crate) fn fill(doc: &mut Document, seed: u64) {
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
    // 保存したことの無いセットだけのプロジェクトは、書けるセットが無いので保存を断る
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
    // 保存したことの無いセットは、最後の保存の後の編集ではなく、保存に入らないことを言う
    assert!(
        fresh.message.contains("is now read-only")
            && fresh
                .message
                .contains("has never been saved, so it will be left out of the save")
            && !fresh.message.contains("Edits since the last save"),
        "{}",
        fresh.message
    );
    let other = dir.file("新しい.ylp");
    fresh.apply(Action::SaveProjectAs(other.clone()));
    fresh.wait_save();
    assert!(!other.exists(), "{}", fresh.message);
    assert!(
        fresh.message.contains("No texture set can be saved"),
        "{}",
        fresh.message
    );
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
    checkpoint_from(s, Instant::now());
}

/// `checkpoint` の、時刻を渡せる形（前の頼みの 16 秒あとから続けるとき）。
fn checkpoint_from(s: &mut AppState, from: Instant) {
    s.recovery_tick_at(from);
    s.recovery_tick_at(from + Duration::from_secs(16));
    s.recovery_wait();
}

/// そのセットの全層をディスクへ逃がし、読めなくして、読もうとして失敗させる（読めないタイルを持つ印が付く）。
pub(crate) fn make_unreadable(s: &AppState, set: usize, cache: &Path) {
    evict_all(cache);
    for l in s.set_doc(set).layers() {
        if let Some(surface) = l.surface(Channel::Color) {
            surface.fail_tile_reads_for_test();
        }
    }
    assert!(s.set_doc(set).composite(s.set_doc(set).bounds()).is_err());
}

/// 復旧用の書き置きを入れた状態（置き場は試験のフォルダ）。
fn with_recovery(root: PathBuf, lang: Lang) -> AppState {
    let mut s = AppState::new_in(256, 128, lang);
    s.recovery.set_space_probe(Some(plenty()));
    s.recovery
        .enable(
            root,
            RecoverySettings {
                directory: None,
                ..RecoverySettings::default()
            },
        )
        .unwrap();
    s
}

/// この実行の置き場の最新の世代（書き置き）を、プロジェクトとして読む。
fn newest_checkpoint(s: &AppState) -> Project {
    let store = GenerationStore::new(s.recovery.session_dir().unwrap());
    let mut files = store.load().unwrap().files;
    files.remove(INFO_NAME);
    Project::from_entries(files).unwrap()
}

#[test]
fn a_never_saved_set_that_becomes_unreadable_is_left_out_and_the_other_sets_are_saved() {
    let dir = TempDir::new("left-out");
    let cache = TempDir::new("left-out-cache");
    let mut s = AppState::new_in(256, 128, Lang::Ja);
    s.apply(Action::Project(NpAction::AddSet));
    s.apply(Action::Project(NpAction::AddSet));
    assert_eq!(s.sets.len(), 3, "{}", s.message);
    for i in 0..3 {
        fill(s.set_doc_mut(i), 40 + i as u64);
    }
    s.modified = true;
    let expected: Vec<Vec<u8>> = (0..3).map(|i| bytes_of(s.set_doc(i))).collect();
    let ids: Vec<String> = (0..3).map(|i| s.sets.get(i).unwrap().id.clone()).collect();
    let name = s.sets.get(1).unwrap().name.clone();
    // 真ん中の（今の）セットだけが読めなくなる。プロジェクトは 1 度も保存していない
    s.switch_set(1).unwrap();
    make_unreadable(&s, 1, &cache.0);
    s.check_tile_cache();
    assert!(s.sets.get(0).unwrap().read_only.is_none());
    assert!(s.sets.get(1).unwrap().read_only.is_some());
    assert!(s.sets.get(2).unwrap().read_only.is_none());
    assert!(
        s.message.contains(&format!(
            "「{name}」は保存したことが無いため、保存に入りません"
        )),
        "{}",
        s.message
    );
    assert!(
        !s.message.contains("書き置き") && !s.message.contains("最後に保存した後の編集"),
        "{}",
        s.message
    );
    // 読めるセットだけが保存され、知らせが入れなかったセットを言う。今のセットを入れなかったので、今のセットは並びで最初の残るセット
    let path = dir.file("作品.ylp");
    s.apply(Action::SaveProjectAs(path.clone()));
    s.wait_save();
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    assert!(
        s.message.contains(&format!(
            "テクスチャセット「{name}」は読めないため、保存に入れていません"
        )),
        "{}",
        s.message
    );
    let back = opened(&path);
    assert_eq!(back.sets.len(), 2);
    assert_eq!(back.sets.get(0).unwrap().id, ids[0]);
    assert_eq!(back.sets.get(1).unwrap().id, ids[2]);
    assert_eq!(back.sets.current().id, ids[0], "最初の残るセット");
    assert!(bytes_of(back.set_doc(0)) == expected[0], "残ったセット 0");
    assert!(bytes_of(back.set_doc(1)) == expected[2], "残ったセット 2");
    // 保存のあとも、入れなかったセットは保存していないまま（読むだけで、保存の印も付かない）
    assert!(s.sets.get(1).unwrap().read_only.is_some());
    assert!(s.sets.get(1).unwrap().saved.is_none());
    // 次の保存でも、また入れずに知らせる。今のセットが残るセットなら、そのセットが今のセットのまま
    s.switch_set(2).unwrap();
    paint(&mut s, 30.0);
    let edited = bytes_of(s.set_doc(2));
    assert!(edited != expected[2]);
    s.apply(Action::SaveProject);
    s.wait_save();
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    assert!(
        s.message.contains(&format!(
            "テクスチャセット「{name}」は読めないため、保存に入れていません"
        )),
        "{}",
        s.message
    );
    let again = opened(&path);
    assert_eq!(again.sets.len(), 2);
    assert_eq!(again.sets.current().id, ids[2]);
    assert!(bytes_of(again.set_doc(0)) == expected[0]);
    assert!(bytes_of(again.set_doc(1)) == edited, "編集した残るセット");
}

/// 入れなかったセットは、画面にあってファイルに無い。保存が成功しても「保存していない変更」の印（`modified`。閉じる・開き直す・
/// 捨てるときの問い、題名の印、外からの操作の「保存していない」が読む）を残す。入れた物だけの保存は、今までどおり印を下ろす。
/// 閉じるときに実際に聞かれることは `gui_shell/save_close.rs`。
#[test]
fn a_save_that_left_a_set_out_keeps_the_unsaved_mark_until_nothing_is_left_out() {
    let dir = TempDir::new("left-out-mark");
    let cache = TempDir::new("left-out-mark-cache");
    let mut s = AppState::new_in(256, 128, Lang::Ja);
    s.apply(Action::Project(NpAction::AddSet));
    for i in 0..2 {
        fill(s.set_doc_mut(i), 70 + i as u64);
    }
    s.modified = true;
    let name = s.sets.get(1).unwrap().name.clone();
    make_unreadable(&s, 1, &cache.0);
    s.check_tile_cache();
    let path = dir.file("作品.ylp");
    s.apply(Action::SaveProjectAs(path.clone()));
    s.wait_save();
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    assert!(s.message.contains(&format!("「{name}」")), "{}", s.message);
    assert!(s.modified && s.shows_modified(), "保存が成功しても印は残る");
    // あとの知らせで上書きされても、印は残る
    s.message = "別の知らせ".into();
    assert!(s.modified);
    // 何も編集せずもう 1 度保存しても、また入れずに知らせ、印は残る
    s.apply(Action::SaveProject);
    s.wait_save();
    assert!(s.message.contains(&format!("「{name}」")), "{}", s.message);
    assert!(s.modified, "次の保存でも残る");
    // 入れた物だけの保存は、今までどおり印を下ろす
    let mut plain = AppState::new_in(256, 128, Lang::Ja);
    fill(plain.set_doc_mut(0), 72);
    plain.modified = true;
    plain.apply(Action::SaveProjectAs(dir.file("普通.ylp")));
    plain.wait_save();
    assert!(
        plain.message.starts_with("保存しました"),
        "{}",
        plain.message
    );
    assert!(!plain.modified && !plain.shows_modified());
}

#[test]
fn when_no_set_can_be_written_the_save_is_refused_and_no_checkpoint_is_made_until_a_readable_set_exists(
) {
    let dir = TempDir::new("none-left");
    let cache = TempDir::new("none-left-cache");
    let mut s = with_recovery(dir.file("recovery"), Lang::Ja);
    s.apply(Action::Project(NpAction::AddSet));
    assert_eq!(s.sets.len(), 2, "{}", s.message);
    for i in 0..2 {
        fill(s.set_doc_mut(i), 50 + i as u64);
    }
    s.modified = true;
    // どちらのセットも保存したことが無く、読めなくなる
    make_unreadable(&s, 0, &cache.0);
    make_unreadable(&s, 1, &cache.0);
    s.check_tile_cache();
    let names = (
        s.sets.get(0).unwrap().name.clone(),
        s.sets.get(1).unwrap().name.clone(),
    );
    // 保存は断られ、何も書かれない
    let path = dir.file("新しい.ylp");
    s.apply(Action::SaveProjectAs(path.clone()));
    s.wait_save();
    assert!(!path.exists(), "{}", s.message);
    assert!(
        s.message.contains("保存できるテクスチャセットがありません")
            && s.message
                .contains(&format!("「{}」「{}」", names.0, names.1)),
        "{}",
        s.message
    );
    // 復旧用の書き置きも作らず、失敗の知らせも出さない（保存の知らせで足りる）
    checkpoint(&mut s);
    assert!(
        !s.message.starts_with("復旧用の書き置きに失敗"),
        "{}",
        s.message
    );
    assert_eq!(s.recovery.checkpoints(), 0);
    // 読めるセットを追加すると、そのセットだけの書き置きができる
    s.apply(Action::Project(NpAction::AddSet));
    assert_eq!(s.sets.len(), 3, "{}", s.message);
    fill(s.set_doc_mut(2), 52);
    s.modified = true;
    checkpoint_from(&mut s, Instant::now() + Duration::from_secs(60));
    assert_eq!(s.recovery.checkpoints(), 1, "{}", s.message);
    let project = newest_checkpoint(&s);
    assert_eq!(project.sets().len(), 1);
    assert_eq!(project.sets()[0].id, s.sets.get(2).unwrap().id);
}

#[test]
fn saved_unreadable_sets_keep_what_was_opened_while_a_never_saved_one_is_left_out_and_a_readable_one_is_saved_as_edited(
) {
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
    let opened_bytes: Vec<Vec<u8>> = {
        let back = opened(&path);
        (0..2).map(|i| bytes_of(back.set_doc(i))).collect()
    };
    // 保存の後に、3 つ目のセットを追加する。保存済みの 2 つ目は、そのあと編集する
    s.apply(Action::Project(NpAction::AddSet));
    assert_eq!(s.sets.len(), 3, "{}", s.message);
    fill(s.set_doc_mut(2), 32);
    s.switch_set(1).unwrap();
    paint(&mut s, 30.0);
    let edited = bytes_of(s.set_doc(1));
    assert!(edited != opened_bytes[1]);
    let ids: Vec<String> = (0..2).map(|i| s.sets.get(i).unwrap().id.clone()).collect();
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
            "\"{new}\" has never been saved, so it will be left out of the save"
        )),
        "{}",
        s.message
    );
    // 保存済みの読むだけのセットは開いたときの中身、保存したことの無いセットは入らず、読めるセットは今の編集
    s.apply(Action::SaveProject);
    s.wait_save();
    assert!(s.message.starts_with("Saved"), "{}", s.message);
    assert!(
        s.message.contains(&format!(
            "Texture set \"{new}\" could not be read and was left out of the save"
        )),
        "{}",
        s.message
    );
    assert!(
        !s.message
            .contains(&format!("\"{saved}\" could not be read")),
        "{}",
        s.message
    );
    let back = opened(&path);
    assert_eq!(back.sets.len(), 2);
    assert_eq!(back.sets.get(0).unwrap().id, ids[0]);
    assert_eq!(back.sets.get(1).unwrap().id, ids[1]);
    assert!(
        bytes_of(back.set_doc(0)) == opened_bytes[0],
        "保存済みの読むだけのセットは開いたときの中身"
    );
    assert!(
        bytes_of(back.set_doc(1)) == edited,
        "読めるセットは今の編集"
    );
}

#[test]
fn a_checkpoint_leaves_out_a_never_saved_unreadable_set_and_restores_the_readable_one() {
    let dir = TempDir::new("checkpoint-left-out");
    let cache = TempDir::new("checkpoint-left-out-cache");
    let root = dir.file("recovery");
    let mut s = with_recovery(root.clone(), Lang::Ja);
    s.apply(Action::Project(NpAction::AddSet));
    assert_eq!(s.sets.len(), 2, "{}", s.message);
    for i in 0..2 {
        fill(s.set_doc_mut(i), 60 + i as u64);
    }
    s.modified = true;
    let kept = bytes_of(s.set_doc(0));
    let id = s.sets.get(0).unwrap().id.clone();
    // 今のセット（2 つ目）が読めなくなる。プロジェクトは保存していない
    make_unreadable(&s, 1, &cache.0);
    s.check_tile_cache();
    checkpoint(&mut s);
    assert!(
        !s.message.starts_with("復旧用の書き置きに失敗"),
        "{}",
        s.message
    );
    assert_eq!(s.recovery.checkpoints(), 1);
    let project = newest_checkpoint(&s);
    assert_eq!(project.sets().len(), 1, "読めないセットは入らない");
    assert_eq!(project.sets()[0].id, id);
    assert_eq!(project.current_set(), id, "今のセットは残るセット");
    // 落ちたあとの起動で戻すと、読めたセットの編集がある
    assert!(s.recovery.is_idle());
    drop(s);
    let mut next = with_recovery(root, Lang::Ja);
    assert!(next.recovery.window.is_some(), "落ちた体の起動で窓が出る");
    next.recovery_apply(RecoveryAction::Open);
    assert_eq!(next.sets.len(), 1, "{}", next.message);
    assert_eq!(next.sets.get(0).unwrap().id, id);
    assert!(bytes_of(next.set_doc(0)) == kept, "戻したセットの中身");
}
