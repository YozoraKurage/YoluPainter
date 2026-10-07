//! 大きなプロジェクトの保存・開き直し・復旧（アプリの状態だけ。画面は描かない）。閾値（`Thresholds`）を小さくして、小さな文書で
//! 分けた正本（版 26）と `YLP-4` の道を通す。巨大な文書は作らない。
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use yolu_app::engine::{Channel, TileCoord};
use yolu_app::lang::Lang;
use yolu_app::recovery::{DiskSpace, RecoveryAction, RecoverySettings, SpaceProbe};
use yolu_app::state::{Action, AppState};
use yolu_io::{GenerationStore, NativeDocument, Package, Project, Thresholds, INFO_NAME};

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> TempDir {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "yolu-app-bigdoc-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
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
const SIZE: u32 = 512;
/// 今の形に収まらない扱い（合計 256 KiB 超えで `YLP-4`）、正本は 256 KiB を超えたら分ける。部分は 128 KiB まで。開いたファイルの
/// エントリはメモリに残さない。
fn small() -> Thresholds {
    Thresholds {
        classic_total_bytes: 256 << 10,
        split_above: 256 << 10,
        part_bytes: 128 << 10,
        part_min: 64 << 10,
        keep_in_memory: 0,
        ..Thresholds::REAL
    }
}
/// 層を足して、層ごとに違う画素を入れる（文書が変わり、保存していない印が付く）。
fn paint_layers(s: &mut AppState, layers: usize, seed: u64) {
    fill_layers(&mut s.doc, layers, seed);
    s.modified = true;
}
fn fill_layers(doc: &mut yolu_app::engine::Document, layers: usize, seed: u64) {
    let ts = doc.tile_size();
    let n = SIZE / ts;
    for i in 0..layers {
        let id = doc.add_layer(&format!("層 {i}")).unwrap();
        for ty in 0..n {
            for tx in 0..n {
                if !(tx + ty + i as u32).is_multiple_of(3) {
                    let bytes = noise(
                        (ts * ts * 4) as usize,
                        seed * 10_000 + (i as u64) * 101 + (ty * n + tx) as u64,
                    );
                    doc.import_tile(id, Channel::Color, TileCoord::new(tx, ty), &bytes)
                        .unwrap();
                }
            }
        }
    }
}
fn bytes_of(doc: &yolu_app::engine::Document) -> Vec<u8> {
    NativeDocument::from_core(doc).unwrap().to_bytes()
}
fn package(path: &Path) -> Package {
    Package::read_bytes(&std::fs::read(path).unwrap(), &yolu_io::Limits::default()).unwrap()
}
fn parts(p: &Package) -> Vec<String> {
    p.entries()
        .keys()
        .filter(|k| k.contains("document.utpaint."))
        .cloned()
        .collect()
}

#[test]
fn a_big_document_is_saved_split_reopened_and_saved_again() {
    small().scoped(|| {
        let dir = TempDir::new("save");
        let path = dir.0.join("大きな作品.ylp");
        let mut s = AppState::new_in(SIZE, SIZE, Lang::Ja);
        paint_layers(&mut s, 6, 1);
        let painted = bytes_of(&s.doc);
        s.apply(Action::SaveProjectAs(path.clone()));
        assert!(s.message.starts_with("保存しました"), "{}", s.message);
        assert!(!s.modified);
        let saved = package(&path);
        assert_eq!(saved.manifest_version(), 4, "今の上限に収まらない形");
        let first_parts = parts(&saved);
        assert!(first_parts.len() >= 3, "{first_parts:?}");
        // 開き直すと画素まで同じ
        let mut again = AppState::new_in(64, 64, Lang::Ja);
        again.apply(Action::OpenProject(path.clone()));
        assert!(again.message.starts_with("開きました"), "{}", again.message);
        assert_eq!(bytes_of(&again.doc), painted);
        // 層を減らして保存し直す: 古い部分は残らない
        let ids: Vec<_> = again.doc.layers().iter().map(|l| l.id()).collect();
        again.doc.remove_layer(ids[0]).unwrap();
        again.doc.remove_layer(ids[1]).unwrap();
        again.modified = true;
        let fewer = bytes_of(&again.doc);
        again.apply(Action::SaveProject);
        assert!(
            again.message.starts_with("保存しました"),
            "{}",
            again.message
        );
        let resaved = package(&path);
        assert!(parts(&resaved).len() < first_parts.len());
        let mut third = AppState::new_in(64, 64, Lang::Ja);
        third.apply(Action::OpenProject(path.clone()));
        assert_eq!(bytes_of(&third.doc), fewer);
        // 開いたまま、ほかの変更なしにもう 1 度保存しても読める（変わらない部分はファイルから写す）
        third
            .doc
            .set_layer_opacity(third.doc.layers()[0].id(), 0.5, false)
            .unwrap();
        third.modified = true;
        let half = bytes_of(&third.doc);
        third.apply(Action::SaveProject);
        assert!(
            third.message.starts_with("保存しました"),
            "{}",
            third.message
        );
        let mut fourth = AppState::new_in(64, 64, Lang::Ja);
        fourth.apply(Action::OpenProject(path));
        assert_eq!(bytes_of(&fourth.doc), half);
    });
}

fn plenty() -> SpaceProbe {
    Arc::new(|_| {
        Some(DiskSpace {
            total: 1000 << 30,
            available: 900 << 30,
        })
    })
}
fn session(root: &Path) -> AppState {
    let mut state = AppState::new_in(SIZE, SIZE, Lang::Ja);
    state.recovery.set_space_probe(Some(plenty()));
    state
        .recovery
        .enable(
            root.to_path_buf(),
            RecoverySettings {
                interval_seconds: 15,
                strokes_between: 0,
                generations_to_keep: 3,
                directory: None,
                ..RecoverySettings::default()
            },
        )
        .unwrap();
    state
}
fn write_after(s: &mut AppState, from: Instant) -> Instant {
    s.recovery_tick_at(from);
    let due = from + Duration::from_secs(s.recovery.settings().interval_seconds as u64 + 1);
    s.recovery_tick_at(due);
    s.recovery_wait();
    due
}

#[test]
fn the_checkpoint_of_a_big_document_shares_unchanged_parts_and_opens_back() {
    small().scoped(|| {
        let dir = TempDir::new("recovery");
        let root = dir.0.join("recovery");
        let mut s = session(&root);
        paint_layers(&mut s, 6, 2);
        let t = write_after(&mut s, Instant::now());
        let store = GenerationStore::new(s.recovery.session_dir().unwrap());
        let first = store.load().unwrap();
        let first_parts: Vec<_> = first
            .files
            .iter()
            .filter(|(k, _)| k.contains("document.utpaint."))
            .map(|(k, b)| (k.clone(), b.sha256().unwrap()))
            .collect();
        assert!(first_parts.len() >= 3, "書き置きも分けた正本");
        // 1 つの層の 1 タイルだけを変える: 変わらない層の部分は同じ中身（共有）
        let ts = s.doc.tile_size();
        let last = s.doc.layers()[s.doc.layers().len() - 1].id();
        s.doc
            .import_tile(
                last,
                Channel::Color,
                TileCoord::new(0, 0),
                &noise((ts * ts * 4) as usize, 4242),
            )
            .unwrap();
        s.modified = true;
        let painted = bytes_of(&s.doc);
        write_after(&mut s, t + Duration::from_secs(1));
        let second = store.load().unwrap();
        let same = first_parts
            .iter()
            .filter(|(k, sha)| {
                second
                    .files
                    .get(k)
                    .is_some_and(|b| b.sha256().unwrap() == *sha)
            })
            .count();
        assert!(
            same >= first_parts.len() - 2,
            "{same} / {}",
            first_parts.len()
        );
        let mut files = second.files.clone();
        files.remove(INFO_NAME);
        let project = Project::from_entries(files).unwrap();
        assert_eq!(
            bytes_of(&project.sets()[0].document.to_core().unwrap()),
            painted
        );
        // 落ちた体から、復旧の窓で開く
        drop(store);
        assert!(s.recovery.is_idle());
        drop(s);
        let mut s2 = session(&root);
        s2.recovery_apply(RecoveryAction::Open);
        assert!(s2.message.starts_with("復旧しました"), "{}", s2.message);
        assert_eq!(bytes_of(&s2.doc), painted);
        // 復旧したものを保存して開き直す
        let saved = dir.0.join("復旧した.ylp");
        s2.apply(Action::SaveProjectAs(saved.clone()));
        assert!(s2.message.starts_with("保存しました"), "{}", s2.message);
        let mut s3 = AppState::new_in(64, 64, Lang::Ja);
        s3.apply(Action::OpenProject(saved));
        assert_eq!(bytes_of(&s3.doc), painted);
    });
}

/// 開いた .ylp を、外で動かす（`moved`）・消す（`deleted`）・別のファイルを同じ名前へ置き換える（`replaced`。同期の道具・もう 1 つの窓の
/// 保存のように、同じ inode を書き換えるのではない）。そのあと外に残っているファイル（動かした先・置き換えたもの）を返す。
fn change_outside(dir: &TempDir, path: &Path, how: &str) -> Option<PathBuf> {
    match how {
        "moved" => {
            let moved = dir.0.join("動かした.ylp");
            std::fs::rename(path, &moved).unwrap();
            Some(moved)
        }
        "deleted" => {
            std::fs::remove_file(path).unwrap();
            None
        }
        _ => {
            let mut other = AppState::new_in(SIZE, SIZE, Lang::Ja);
            paint_layers(&mut other, 2, 9);
            let theirs = dir.0.join("別の作品.ylp");
            other.apply(Action::SaveProjectAs(theirs.clone()));
            std::fs::rename(&theirs, path).unwrap();
            Some(path.to_path_buf())
        }
    }
}
/// 保存先に残ってはいけないもの（保存の一時ファイル・ロック・退避の置き場）。
fn save_leftovers(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("pending") || n.ends_with(".save.lock~") || n.ends_with("-backups~"))
        .collect()
}

/// 開いた .ylp はファイルを持ち続けて読む: 外で動かされても・消されても・別のファイルに置き換えられても、別名の保存はできて、中身は
/// 開いたときのまま。同じ名前への上書きは、保存先が外で消された・変わったと断り、何も作らない・残さない。
#[test]
fn a_moved_deleted_or_replaced_original_still_saves_under_another_name() {
    small().scoped(|| {
        let mut painted = Vec::new();
        for how in ["moved", "deleted", "replaced"] {
            let dir = TempDir::new(how);
            let path = dir.0.join("元.ylp");
            let mut s = AppState::new_in(SIZE, SIZE, Lang::Ja);
            paint_layers(&mut s, 4, 3);
            painted = bytes_of(&s.doc);
            s.apply(Action::SaveProjectAs(path.clone()));
            assert!(s.message.starts_with("保存しました"), "{}", s.message);
            let mut opened = AppState::new_in(64, 64, Lang::Ja);
            opened.apply(Action::OpenProject(path.clone()));
            assert!(
                opened.message.starts_with("開きました"),
                "{}",
                opened.message
            );
            // 外で: 動かす・消す・別のファイルを同じ名前へ置き換える。動かした・置き換えたファイルには、このあと触らない
            let outside = change_outside(&dir, &path, how);
            let before = outside.as_ref().map(|p| std::fs::read(p).unwrap());
            // 変えていないセットの中身は、開いたファイルから写す。別名の保存はできて、開き直すと開いたときの中身のまま
            let to = dir.0.join("別のフォルダ").join("別名.ylp");
            opened.apply(Action::SaveProjectAs(to.clone()));
            assert!(
                opened.message.starts_with("保存しました"),
                "{how}: {}",
                opened.message
            );
            let mut check = AppState::new_in(64, 64, Lang::Ja);
            check.apply(Action::OpenProject(to));
            assert!(
                check.message.starts_with("開きました"),
                "{how}: {}",
                check.message
            );
            assert_eq!(
                bytes_of(&check.doc),
                painted,
                "{how}: 開いたときの中身のまま"
            );
            if let (Some(p), Some(before)) = (&outside, &before) {
                assert_eq!(&std::fs::read(p).unwrap(), before, "{how}");
            }
            let left: Vec<String> = std::fs::read_dir(&dir.0)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|n| n.contains("pending"))
                .collect();
            assert!(left.is_empty(), "{how}: {left:?}");
        }
        assert!(!painted.is_empty());
    });
}

/// 開いたファイルが外で消された・変わったあとの、同じ名前への上書きの保存は、外の変更として断る（新しく作らない・外のファイルを潰さない）。
/// 理由は、置き換えられた（変わった）のと消された（動かされた）のとで言い分け、英語の画面でも出す。
#[test]
fn saving_over_a_moved_or_replaced_original_is_refused_and_leaves_nothing() {
    small().scoped(|| {
        for (lang, changed, deleted, prefix) in [
            (
                Lang::Ja,
                "外部で変更されています",
                "外部で消されています",
                "保存できません",
            ),
            (
                Lang::En,
                "Save target or backup changed",
                "The save target was deleted or moved outside",
                "Cannot save",
            ),
        ] {
            let dir = TempDir::new("over");
            // 英語の画面の文には、ファイル名の日本語が入らないよう、名前は ASCII にする
            let path = dir.0.join("original.ylp");
            let mut s = AppState::new_in(SIZE, SIZE, Lang::Ja);
            paint_layers(&mut s, 4, 3);
            s.apply(Action::SaveProjectAs(path.clone()));
            let mut opened = AppState::new_in(64, 64, lang);
            opened.apply(Action::OpenProject(path.clone()));
            // 別のファイルに置き換えられた後は、外の変更として断る
            let outside = change_outside(&dir, &path, "replaced").unwrap();
            let theirs = std::fs::read(&outside).unwrap();
            opened.modified = true;
            opened.apply(Action::SaveProject);
            assert!(
                opened.message.contains(prefix),
                "{lang:?}: {}",
                opened.message
            );
            assert!(
                opened.message.contains(changed),
                "{lang:?}: {}",
                opened.message
            );
            assert!(
                !opened.message.contains(deleted),
                "{lang:?}: {}",
                opened.message
            );
            assert!(opened.modified, "保存していない印のまま");
            assert_eq!(std::fs::read(&path).unwrap(), theirs);
            // 消された後は、保存先が外で消されたと断る（新しく作らない）
            std::fs::remove_file(&path).unwrap();
            opened.apply(Action::SaveProject);
            assert!(
                opened.message.contains(prefix),
                "{lang:?}: {}",
                opened.message
            );
            assert!(
                opened.message.contains(deleted),
                "{lang:?}: {}",
                opened.message
            );
            assert!(
                !opened.message.contains(changed),
                "{lang:?}: {}",
                opened.message
            );
            assert_eq!(
                lang == Lang::En,
                opened.message.is_ascii(),
                "{lang:?}: {}",
                opened.message
            );
            assert!(!path.exists());
            assert!(opened.modified, "保存していない印のまま");
            assert!(
                save_leftovers(&dir.0).is_empty(),
                "{:?}",
                save_leftovers(&dir.0)
            );
        }
    });
}

/// 復旧の世代から開いたあとで読むだけになったセット（効果の入力がそろわない）の正本は、世代の外に置き直してある: 開いた世代を復旧の
/// 窓で捨ててから保存しても、正本は元のバイト列のまま書かれる。置き直した中身は、プロジェクトを手放すと片付く。
#[test]
fn a_set_locked_after_recovery_is_saved_even_after_its_generation_is_discarded() {
    let t = Thresholds {
        split_above: 64 << 10,
        part_bytes: 32 << 10,
        part_min: 0,
        ..small()
    };
    t.scoped(|| {
        use yolu_io::{MaterialRef, SaveTarget, SetSpec, WriterInfo};
        let dir = TempDir::new("held");
        let root = dir.0.join("recovery");
        let generators = NativeDocument::read(
            &std::fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../yolu-io/tests/fixtures/effects-generators.utpaint"),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(
            generators.core_issues().is_empty(),
            "core へは変換できる（入力がそろわないだけ）"
        );
        let (blank, _) = yolu_app::state::blank_document(SIZE, SIZE);
        let editable = yolu_app::sets::guid_string(blank.id());
        let locked = generators.id().to_owned();
        let spec = |id: &str, name: &str, slot: u16, doc: yolu_io::DocumentSource| SetSpec {
            id: id.to_owned(),
            name: name.into(),
            material: MaterialRef::PendingSlot(slot),
            document: Some(doc),
            composites: vec![],
        };
        let project = Project::create(
            WriterInfo {
                app: "試験".into(),
                version: "0".into(),
                unity: "standalone".into(),
            },
            &[
                spec(&editable, "描ける", 0, blank.into()),
                spec(&locked, "効果", 1, generators.clone().into()),
            ],
            &editable,
        )
        .unwrap();
        let path = dir.0.join("効果.ylp");
        SaveTarget::create(&path).unwrap().save(&project).unwrap();
        assert!(
            !parts(&package(&path)).is_empty(),
            "読むだけになるセットの正本は分けて置く"
        );

        let mut s = session(&root);
        s.apply(Action::OpenProject(path.clone()));
        assert!(s.sets.get(1).unwrap().read_only.is_some(), "{}", s.message);
        paint_layers(&mut s, 2, 5);
        write_after(&mut s, Instant::now());
        assert_eq!(s.recovery.checkpoints(), 1, "{}", s.message);
        drop(s);

        let mut s2 = session(&root);
        s2.recovery_apply(RecoveryAction::Open);
        assert!(s2.message.starts_with("復旧しました"), "{}", s2.message);
        assert!(
            s2.sets.get(1).unwrap().read_only.is_some(),
            "開いたあとで入力がそろわず読むだけ"
        );
        let held = held_dirs(&root);
        assert_eq!(held.len(), 1, "{held:?}");
        // 開いた世代を復旧の窓で捨てる（落ちた実行のプールごと消える）
        s2.recovery_apply(RecoveryAction::OpenWindow);
        let rows = &s2.recovery.window.as_ref().unwrap().rows;
        assert_eq!(rows.len(), 1);
        let pool = rows[0].pool.clone();
        s2.recovery_apply(RecoveryAction::Select(0));
        s2.recovery_apply(RecoveryAction::Discard);
        s2.recovery_apply(RecoveryAction::ConfirmDiscard);
        assert!(!pool.exists(), "捨てた世代の置き場");
        // 保存して読み直すと、読むだけのセットの正本は元のまま
        let saved = dir.0.join("復旧した.ylp");
        s2.apply(Action::SaveProjectAs(saved.clone()));
        assert!(s2.message.starts_with("保存しました"), "{}", s2.message);
        let reopened = Project::read(&std::fs::read(&saved).unwrap()).unwrap();
        let kept = reopened.sets().iter().find(|x| x.id == locked).unwrap();
        assert_eq!(kept.document.to_bytes().unwrap(), generators.to_bytes());
        // 置き直した中身は、プロジェクトを手放すと消える（落ちて残ったものは次の起動が片付ける）
        drop(s2);
        assert!(held_dirs(&root)
            .iter()
            .all(|d| std::fs::read_dir(d).unwrap().next().is_none()));
        let _s3 = session(&root);
        assert!(held_dirs(&root).is_empty());
    });
}
/// 復旧の置き場の根の下の、置き直した中身の置き場。
fn held_dirs(root: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".held~"))
        .collect()
}

/// 2 つのセット（描く 0 番が名前の順で先）の .ylp。1 番のセットにも画素を入れる。返すのは、ファイルとセットの ID。
fn two_sets(dir: &TempDir, name: &str) -> (PathBuf, Vec<String>) {
    use yolu_io::{MaterialRef, SaveTarget, SetSpec, WriterInfo};
    let mut docs: Vec<_> = (0..2)
        .map(|_| yolu_app::state::blank_document(SIZE, SIZE).0)
        .collect();
    docs.sort_by_key(|d| yolu_app::sets::guid_string(d.id()));
    fill_layers(&mut docs[1], 2, 9);
    let ids: Vec<String> = docs
        .iter()
        .map(|d| yolu_app::sets::guid_string(d.id()))
        .collect();
    let specs: Vec<SetSpec> = docs
        .iter()
        .enumerate()
        .map(|(k, doc)| SetSpec {
            id: ids[k].clone(),
            name: format!("セット{k}"),
            material: MaterialRef::PendingSlot(k as u16),
            document: Some(NativeDocument::from_core(doc).unwrap().into()),
            composites: vec![],
        })
        .collect();
    let project = Project::create(
        WriterInfo {
            app: "試験".into(),
            version: "0".into(),
            unity: "standalone".into(),
        },
        &specs,
        &ids[0],
    )
    .unwrap();
    let path = dir.0.join(name);
    SaveTarget::create(&path).unwrap().save(&project).unwrap();
    (path, ids)
}
/// `root` の下の全部のファイルとフォルダーの名前。
fn names_under(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            found.push(entry.file_name().to_string_lossy().into_owned());
            if entry.file_type().unwrap().is_dir() {
                todo.push(entry.path());
            }
        }
    }
    found
}

/// 復旧の書き置きも、開いた .ylp が外で動かされた・消された・別のファイルに置き換えられたあとで、描いていないセットの正本を開いたときの
/// 中身のまま写し、書き置きの途中のものを残さない。落ちたあとの復旧で開いても、描いたとおりに戻る。
#[test]
fn a_checkpoint_is_written_from_the_opened_contents_after_the_file_moved_or_changed_outside() {
    small().scoped(|| {
        for how in ["moved", "deleted", "replaced"] {
            let dir = TempDir::new(how);
            let root = dir.0.join("recovery");
            let (path, ids) = two_sets(&dir, "二つ.ylp");
            let base = Project::read(&std::fs::read(&path).unwrap())
                .unwrap()
                .original_archive()
                .entries()
                .clone();
            let mut s = session(&root);
            s.apply(Action::OpenProject(path.clone()));
            assert!(s.message.starts_with("開きました"), "{how}: {}", s.message);
            let outside = change_outside(&dir, &path, how);
            let theirs = outside.as_ref().map(|p| std::fs::read(p).unwrap());
            // 描くセット（0 番）だけを変える。描かないセット（1 番）の正本は、開いたファイルから写す
            paint_layers(&mut s, 2, 6);
            let painted = bytes_of(&s.doc);
            write_after(&mut s, Instant::now());
            assert_eq!(s.recovery.checkpoints(), 1, "{how}: {}", s.message);
            let mut files = GenerationStore::new(s.recovery.session_dir().unwrap())
                .load()
                .unwrap()
                .files;
            let prefix = format!("sets/{}/", ids[1]);
            let untouched: Vec<&String> = base.keys().filter(|k| k.starts_with(&prefix)).collect();
            assert!(
                untouched.iter().any(|k| k.contains("document.utpaint")),
                "{how}: {untouched:?}"
            );
            for name in untouched {
                assert_eq!(
                    files[name].bytes().unwrap(),
                    base[name].bytes().unwrap(),
                    "{how}: {name} は開いたときのまま"
                );
            }
            files.remove(INFO_NAME);
            let written = Project::from_entries(files).unwrap();
            let drawn = written.sets().iter().find(|x| x.id == ids[0]).unwrap();
            assert_eq!(
                bytes_of(&drawn.document.to_core().unwrap()),
                painted,
                "{how}"
            );
            // 外のファイルには触らず、書き置きの途中のものも、保存先の隣の残骸も残さない
            if let (Some(p), Some(theirs)) = (&outside, &theirs) {
                assert_eq!(&std::fs::read(p).unwrap(), theirs, "{how}");
            }
            let left: Vec<String> = names_under(&root)
                .into_iter()
                .filter(|n| n.contains("pending") || n.contains("staging"))
                .collect();
            assert!(left.is_empty(), "{how}: {left:?}");
            assert!(
                save_leftovers(&dir.0).is_empty(),
                "{how}: {:?}",
                save_leftovers(&dir.0)
            );
            // 落ちたあと、復旧で開くと描いたとおりに戻る
            drop(s);
            let mut s2 = session(&root);
            s2.recovery_apply(RecoveryAction::Open);
            assert!(
                s2.message.starts_with("復旧しました"),
                "{how}: {}",
                s2.message
            );
            assert_eq!(bytes_of(&s2.doc), painted, "{how}");
        }
    });
}

/// 開いた .ylp を置き換える保存は、動いている書き置き（そのファイルの位置から読む）が終わるまで待つ: 書き置きは前のファイルを読み
/// 終えて確定し、保存も通る。配布用に保存の写しがある間は保存しない。
#[test]
fn saving_over_the_open_file_waits_for_a_running_checkpoint() {
    use std::sync::atomic::AtomicBool;
    use yolu_app::distribute::DistributeAction;
    use yolu_io::{MaterialRef, SaveTarget, SetSpec, WriterInfo};
    small().scoped(|| {
        let dir = TempDir::new("overlap");
        let root = dir.0.join("recovery");
        // 2 つのセット: 描くセットが名前の順で先（書き直すと、描かないセットのエントリの位置がずれる）。描かないセットにも画素を入れる
        let mut docs: Vec<_> = (0..2)
            .map(|_| yolu_app::state::blank_document(SIZE, SIZE).0)
            .collect();
        docs.sort_by_key(|d| yolu_app::sets::guid_string(d.id()));
        fill_layers(&mut docs[1], 2, 9);
        let ids: Vec<String> = docs
            .iter()
            .map(|d| yolu_app::sets::guid_string(d.id()))
            .collect();
        let specs: Vec<SetSpec> = docs
            .iter()
            .enumerate()
            .map(|(k, doc)| SetSpec {
                id: ids[k].clone(),
                name: format!("セット{k}"),
                material: MaterialRef::PendingSlot(k as u16),
                document: Some(NativeDocument::from_core(doc).unwrap().into()),
                composites: vec![],
            })
            .collect();
        let project = Project::create(
            WriterInfo {
                app: "試験".into(),
                version: "0".into(),
                unity: "standalone".into(),
            },
            &specs,
            &ids[0],
        )
        .unwrap();
        let path = dir.0.join("重なり.ylp");
        SaveTarget::create(&path).unwrap().save(&project).unwrap();
        let mut s = session(&root);
        s.apply(Action::OpenProject(path.clone()));
        assert!(s.message.starts_with("開きました"), "{}", s.message);
        // 書き置きのスレッドを、開いた .ylp から読む前で止める
        let (entered, wait_entered) = std::sync::mpsc::channel();
        let entered = std::sync::Mutex::new(entered);
        let release = Arc::new(AtomicBool::new(false));
        let armed = Arc::new(AtomicBool::new(true));
        let (r, a) = (release.clone(), armed.clone());
        s.recovery.set_fault(Some(Arc::new(move |stage: &str| {
            if stage == "snapshot" && a.swap(false, Ordering::SeqCst) {
                let _ = entered.lock().unwrap().send(());
                while !r.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            Ok(())
        })));
        paint_layers(&mut s, 2, 6);
        let t0 = Instant::now();
        s.recovery_tick_at(t0);
        s.recovery_tick_at(t0 + Duration::from_secs(16));
        wait_entered
            .recv_timeout(Duration::from_secs(30))
            .expect("書き置きが始まる");
        let releaser = {
            let r = release.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                r.store(true, Ordering::SeqCst);
            })
        };
        // 上書きの保存: 書き置きが終わるのを待ってから置き換える
        s.apply(Action::SaveProject);
        assert!(s.message.starts_with("保存しました"), "{}", s.message);
        releaser.join().unwrap();
        s.recovery_wait();
        assert_eq!(
            s.recovery.checkpoints(),
            1,
            "書き置きは前のファイルを読み終えて確定する: {}",
            s.message
        );
        let mut again = AppState::new_in(64, 64, Lang::Ja);
        again.apply(Action::OpenProject(path.clone()));
        assert!(again.message.starts_with("開きました"), "{}", again.message);
        // 配布用に保存の写しがある間は保存しない（写しは開いたファイルの位置から読む）
        s.apply(Action::Distribute(DistributeAction::Start));
        s.wait_distribute();
        assert!(s.distribute.is_open());
        s.modified = true;
        s.apply(Action::SaveProject);
        assert!(s.message.contains("保存できません"), "{}", s.message);
        assert!(s.message.contains("配布用に保存の途中"), "{}", s.message);
        s.apply(Action::Distribute(DistributeAction::CancelWindow));
        s.dialog_request = None;
        s.apply(Action::SaveProject);
        assert!(s.message.starts_with("保存しました"), "{}", s.message);
    });
}
