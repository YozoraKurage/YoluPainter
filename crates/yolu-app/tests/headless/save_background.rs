//! .ylp の保存を裏のスレッドで行う（アプリの状態だけ。画面は描かない。閉じる流れの画面側は `gui_shell/save_close.rs`）。
//!
//! - 裏の保存の結果は、同じ中身を画面のスレッドで保存した .ylp とバイトまで同じ（今の形・大きな形の両方）。さらに、yolu-io を直に 1 つずつ
//!   呼んで組んだ物差し（`reference_entries`。アプリの保存の組み立てを通らない）とも、新規・上書き・複数のセットで同じ。
//! - 保存の間に描いた分は保存に入らず、保存の後も「変更あり」。保存の間の 2 回目の保存・開く・新規・配布用に保存は理由つきで断る。
//! - 失敗した保存は何も変えずに理由を出し、保存の前の「変更あり」を戻す。外で書き換えられた・置き換えられたときの断りは同期の保存と同じ。
//! - 保存の間は復旧の書き置きの新しい頼みを出さない。進み具合は戻らずに進む。
//!
//! 大きな文書の時間は `#[ignore]` の `timing_of_a_big_document`（`-- --ignored --nocapture`。試験の通常の回しには入れない）。
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use yolu_app::engine::{Channel, DVec2, TileCoord};
use yolu_app::lang::Lang;
use yolu_app::recovery::{DiskSpace, RecoverySettings, SpaceProbe};
use yolu_app::state::{Action, AppState};
use yolu_io::{NativeDocument, Package, Thresholds};

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> TempDir {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "yolu-app-savebg-{tag}-{}-{}",
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

/// 疑似乱数のバイト列（縮まない画素）。
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
/// 縮む画素（なめらかな濃淡に少しだけ乱れ）。描いた絵に近い圧縮の効き方。
fn painted(len: usize, seed: u64) -> Vec<u8> {
    let rough = noise(len, seed);
    (0..len)
        .map(|i| {
            let p = i / 4;
            let base = match i % 4 {
                0 => (p % 128) * 2,
                1 => ((p / 128) % 128) * 2,
                2 => ((seed as usize) * 37) % 256,
                _ => 255,
            };
            if i % 4 == 3 {
                255
            } else {
                ((base + (rough[i] as usize & 3)) & 255) as u8
            }
        })
        .collect()
}
/// 層を足して、層ごとに違う画素を入れる。縮まない層と縮む層を交互に（描いた絵に近い）。
fn fill_layers(doc: &mut yolu_app::engine::Document, layers: usize, seed: u64) {
    let ts = doc.tile_size();
    let (nx, ny) = (doc.width().div_ceil(ts), doc.height().div_ceil(ts));
    for i in 0..layers {
        let id = doc.add_layer(&format!("層 {i}")).unwrap();
        for ty in 0..ny {
            for tx in 0..nx {
                if !(tx + ty + i as u32).is_multiple_of(3) {
                    let s = seed * 10_000 + (i as u64) * 101 + (ty * nx + tx) as u64;
                    let len = (ts * ts * 4) as usize;
                    let bytes = if i % 2 == 1 { painted(len, s) } else { noise(len, s) };
                    doc.import_tile(id, Channel::Color, TileCoord::new(tx, ty), &bytes).unwrap();
                }
            }
        }
    }
}
/// 描いた絵に近い層の組（4 枚に 1 枚は全面の一色の塗り＝一様なタイル、描き込んだ層・縮まない層・数タイルだけの層）。実データの代わりの
/// 疑似の画素で、正本は一様なタイルを全画素に広げて書くので、層の数が多いほど書く量が大きくなる。
fn fill_like_art(doc: &mut yolu_app::engine::Document, layers: usize, seed: u64) {
    let ts = doc.tile_size();
    let (nx, ny) = (doc.width().div_ceil(ts), doc.height().div_ceil(ts));
    let len = (ts * ts * 4) as usize;
    for i in 0..layers {
        let id = doc.add_layer(&format!("層 {i}")).unwrap();
        for ty in 0..ny {
            for tx in 0..nx {
                let n = ty * nx + tx;
                let s = seed * 100_000 + (i as u64) * 1009 + n as u64;
                let bytes = match i % 4 {
                    0 => Some([(i * 37 % 256) as u8, 120, (i * 11 % 256) as u8, 255].repeat((ts * ts) as usize)),
                    1 if n % 5 != 0 => Some(painted(len, s)),
                    2 if n % 7 == 0 => Some(noise(len, s)),
                    3 if n % 61 == 0 => Some(painted(len, s)),
                    _ => None,
                };
                if let Some(bytes) = bytes {
                    doc.import_tile(id, Channel::Color, TileCoord::new(tx, ty), &bytes).unwrap();
                }
            }
        }
    }
}
fn painted_state(size: u32, layers: usize, seed: u64) -> AppState {
    let mut s = AppState::new_in(size, size, Lang::Ja);
    fill_layers(&mut s.doc, layers, seed);
    s.modified = true;
    s
}
/// 1 本のストロークを描いて終える（文書が変わり、保存していない印が付く）。
fn paint(s: &mut AppState, x: f64) {
    let layer = s.selected_layer.unwrap();
    let brush = s.stroke_settings(false);
    let mut stroke = s.doc.begin_stroke(layer, &brush).unwrap();
    stroke.add_point(&mut s.doc, x, 20.0, 1.0, DVec2::ZERO).unwrap();
    s.doc.end_stroke(stroke).unwrap();
    s.modified = true;
}
fn bytes_of(doc: &yolu_app::engine::Document) -> Vec<u8> {
    NativeDocument::from_core(doc).unwrap().to_bytes()
}
/// 今の形に収まらない扱い（合計 256 KiB 超えで `YLP-4`）、正本は 256 KiB を超えたら分ける。部分は 128 KiB まで。
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
fn entries_of(path: &Path) -> Vec<(String, Vec<u8>)> {
    let p = Package::read_bytes(&std::fs::read(path).unwrap(), &yolu_io::Limits::unbounded()).unwrap();
    p.entries()
        .iter()
        .map(|(n, b)| (n.clone(), b.bytes().unwrap().to_vec()))
        .collect()
}
/// エントリの並び・名前・中身が同じことを確かめる（食い違ったら、バイト列を並べずに、どのエントリかを言う）。
fn assert_same_entries(label: &str, left: &[(String, Vec<u8>)], right: &[(String, Vec<u8>)]) {
    let names = |e: &[(String, Vec<u8>)]| e.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
    assert_eq!(names(left), names(right), "{label}: エントリの名前と並び");
    let differing: Vec<&str> = left
        .iter()
        .zip(right)
        .filter(|(l, r)| l.1 != r.1)
        .map(|(l, _)| l.0.as_str())
        .collect();
    assert!(differing.is_empty(), "{label}: 中身が違うエントリ {differing:?}");
}
fn opened(path: &Path) -> AppState {
    let mut s = AppState::new_in(64, 64, Lang::Ja);
    s.apply(Action::OpenProject(path.to_path_buf()));
    assert!(s.message.starts_with("開きました"), "{}", s.message);
    s
}
/// 保存の頼みを出して、終わるまで待つ（裏のスレッドの状態でも、結果が出てから返る）。
fn save_and_settle(s: &mut AppState, action: Action) {
    s.apply(action);
    s.wait_save();
}
/// 保存先に残ってはいけないもの（保存の一時ファイル・ロック）。
fn save_leftovers(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("pending") || n.ends_with(".save.lock~"))
        .collect()
}

/// 物差し: 保存を裏へ移す前の同期の保存と同じ並び（セットごとに写し → 正本の元 → 合成の PNG を 1 つずつ作り、yolu-io の安全な保存で書く）を、
/// アプリの保存の組み立て（`project::capture`・合成の並列）を通さずに、yolu-io を直に 1 つずつ呼んで再現し、書いたファイルのエントリを返す。
/// `path` にファイルがあれば（開いたファイルへの上書き）それを土台に、無ければ新しく作る。すべてのセットを書き直す。選択範囲・見た目・棚・
/// モデルの参照が既定のままの文書で比べる（そのエントリは、アプリの保存でも書かれない）。呼んだスレッドの閾値が効く。
fn reference_entries(s: &AppState, path: &Path) -> Vec<(String, Vec<u8>)> {
    use std::sync::Arc;
    let specs: Vec<yolu_io::SetSpec> = s
        .sets
        .iter()
        .enumerate()
        .map(|(i, set)| {
            let doc = s.set_doc(i);
            let snapshot = Arc::new(doc.capture_snapshot().unwrap());
            yolu_io::SetSpec {
                id: set.id.clone(),
                name: set.name.clone(),
                material: set.material.clone(),
                document: Some(yolu_io::DocumentSource::from_core(snapshot).unwrap()),
                composites: yolu_io::composite_pngs(doc).unwrap(),
            }
        })
        .collect();
    let current = s.sets.current().id.clone();
    let writer = yolu_app::project::writer();
    let (project, mut target) = if path.exists() {
        let (base, target) = yolu_io::SaveTarget::open_within(path, &yolu_io::Limits::unbounded()).unwrap();
        (base.with_sets_dropping(writer, &specs, &current, &[]).unwrap(), target)
    } else {
        (yolu_io::Project::create(writer, &specs, &current).unwrap(), yolu_io::SaveTarget::create(path).unwrap())
    };
    // 見た目の設定（look.json）は、保存を移す前も後も同じ関数が書く。その関数の結果は物差しにも入れる（比べたいのは、正本・合成の PNG・
    // 並び・ファイルの形）
    let looks: Vec<_> = s.sets.iter().enumerate().map(|(i, set)| (set.id.as_str(), s.set_doc(i).look())).collect();
    let (project, _) = yolu_app::look::io::write_into(project, &looks, Lang::Ja).unwrap();
    target.save_with(&project, s.prefs.settings.backups).unwrap();
    entries_of(path)
}

// ───────── 試験 ─────────

/// 裏の保存の結果は、同じ中身を画面のスレッドで保存した .ylp とバイトまで同じ（ZIP の中のエントリも正本も）。今の形（YLP-3）と、分けた
/// 正本・zip64 を通る大きな形の両方で、新しいファイルと、開いたファイルへの上書き（退避つき）の両方。
#[test]
fn a_background_save_writes_the_same_bytes_as_the_inline_save() {
    for (label, thresholds) in [("今の形", Thresholds::REAL), ("大きな形", small())] {
        thresholds.scoped(|| {
            let dir = TempDir::new("same");
            let mut s = painted_state(256, 5, 5);
            // 新しいファイル: 同じ文書を、書き直させて（保存済みの印を外す）画面のスレッドと裏のスレッドで書く
            s.apply(Action::SaveProjectAs(dir.file("first.ylp")));
            assert!(s.message.starts_with("保存しました"), "{label}: {}", s.message);
            s.sets.get_mut(0).unwrap().saved = None;
            s.apply(Action::SaveProjectAs(dir.file("inline.ylp")));
            assert!(s.message.starts_with("保存しました"), "{label}: {}", s.message);
            s.sets.get_mut(0).unwrap().saved = None;
            s.save.background = true;
            s.apply(Action::SaveProjectAs(dir.file("background.ylp")));
            assert!(s.is_saving(), "{label}: 保存の頼みは裏の仕事を残して返る");
            s.wait_save();
            assert!(s.message.starts_with("保存しました"), "{label}: {}", s.message);
            assert!(!s.is_saving());
            let (inline, background) = (dir.file("inline.ylp"), dir.file("background.ylp"));
            assert_eq!(entries_of(&inline), entries_of(&background), "{label}: エントリの中身");
            assert_eq!(std::fs::read(&inline).unwrap(), std::fs::read(&background).unwrap(), "{label}: ファイルのバイト");
            // 組み立てを共有した今の実装どうしの比べだけでは、組み立て自体の退行を見逃す。yolu-io を直に 1 つずつ呼んだ物差しとも同じ
            let reference = reference_entries(&s, &dir.file("reference.ylp"));
            assert_same_entries(&format!("{label}: 物差し（yolu-io を直に逐次）"), &entries_of(&background), &reference);
            assert_eq!(
                std::fs::read(&background).unwrap(),
                std::fs::read(dir.file("reference.ylp")).unwrap(),
                "{label}: 物差しのファイルのバイト"
            );
            let version = Package::open(&background, &yolu_io::Limits::unbounded()).unwrap().manifest_version();
            assert_eq!(version == 4, label == "大きな形", "{label}: 形");
            // 開いたファイルへの上書き（退避つき）も同じ: 同じ中身の 2 つのファイルを開き、同じ変更を加えて、
            // 画面のスレッド（a）と裏のスレッド（b）で上書きする
            let (pa, pb) = (dir.file("a.ylp"), dir.file("b.ylp"));
            std::fs::copy(&background, &pa).unwrap();
            std::fs::copy(&background, &pb).unwrap();
            let (mut a, mut b) = (opened(&pa), opened(&pb));
            for state in [&mut a, &mut b] {
                let layer = state.doc.layers()[0].id();
                state.doc.set_layer_opacity(layer, 0.5, false).unwrap();
                state.modified = true;
            }
            a.apply(Action::SaveProject);
            b.save.background = true;
            b.apply(Action::SaveProject);
            assert!(b.is_saving());
            b.wait_save();
            assert!(a.message.starts_with("保存しました"), "{label}: {}", a.message);
            assert!(b.message.starts_with("保存しました"), "{label}: {}", b.message);
            assert_eq!(entries_of(&pa), entries_of(&pb), "{label}: 上書きのエントリ");
            assert_eq!(bytes_of(&a.doc), bytes_of(&b.doc));
            // 上書きも、yolu-io を直に呼んだ物差し（同じ元のファイルの写しに、変えたセットを重ねる）と同じ
            let pr = dir.file("r.ylp");
            std::fs::copy(&background, &pr).unwrap();
            assert_same_entries(&format!("{label}: 上書きの物差し"), &entries_of(&pb), &reference_entries(&b, &pr));
            assert!(a.message.contains("前の版は") && b.message.contains("前の版は"), "{label}: 退避は同じく残る: {}", b.message);
            assert_eq!(a.message.replace("a.ylp", "x"), b.message.replace("b.ylp", "x"), "{label}: 知らせの文も同じ");
        });
    }
}

/// 保存の間に描いた分は保存に入らず、保存の後も「変更あり」のまま。次の保存はそのセットを書き直す。
#[test]
fn what_is_drawn_during_a_background_save_is_not_in_the_file_and_stays_modified() {
    let dir = TempDir::new("draw");
    let path = dir.file("作品.ylp");
    let mut s = painted_state(256, 3, 2);
    s.save.background = true;
    let before = bytes_of(&s.doc);
    let hold = s.save.hold_next();
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.is_saving());
    assert!(!path.exists(), "止めている間は何も書かない");
    assert!(s.shows_modified(), "保存の間は、保存の結果が出るまで「変更あり」の印を見せる");
    // 保存の間に描く（描ける・見られる）
    paint(&mut s, 30.0);
    let during = bytes_of(&s.doc);
    assert_ne!(during, before);
    hold.release();
    s.wait_save();
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    assert!(s.modified, "保存の間に描いた分は「変更あり」のまま");
    assert!(s.shows_modified());
    // ファイルは、頼んだ時点の中身
    let saved = opened(&path);
    assert_eq!(bytes_of(&saved.doc), before, "保存の間に描いた分は入らない");
    // 次の保存は、そのセットを書き直して、描いた分が入る
    s.apply(Action::SaveProject);
    s.wait_save();
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    assert_eq!(s.rewritten_sets, 1, "描いた分があるセットを書き直す");
    assert!(!s.modified);
    assert_eq!(bytes_of(&opened(&path).doc), during);
    // 何も描かなければ、書き直さない
    s.apply(Action::SaveProject);
    s.wait_save();
    assert_eq!(s.rewritten_sets, 0);
}

/// 保存の間の 2 回目の保存・別名の保存・開く・新規・配布用に保存は、理由つきで断られ、状態を変えない。終わったあとはできる。
#[test]
fn a_second_save_open_new_and_distribution_are_refused_while_saving() {
    let dir = TempDir::new("refuse");
    let path = dir.file("作品.ylp");
    let other = dir.file("別.ylp");
    // 開く先の別の作品
    let mut theirs = painted_state(128, 1, 9);
    theirs.apply(Action::SaveProjectAs(dir.file("開く先.ylp")));
    let mut s = painted_state(256, 2, 4);
    s.save.background = true;
    let hold = s.save.hold_next();
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.is_saving());
    let name = s.project_name.clone();
    let layers = s.doc.layers().len();
    let busy = "保存の途中です";
    s.apply(Action::SaveProject);
    assert!(s.message.starts_with("保存できません") && s.message.contains(busy), "{}", s.message);
    s.apply(Action::SaveProjectAs(other.clone()));
    assert!(s.message.starts_with("保存できません") && s.message.contains(busy), "{}", s.message);
    s.apply(Action::OpenProject(dir.file("開く先.ylp")));
    assert!(s.message.starts_with("開けません") && s.message.contains(busy), "{}", s.message);
    assert_eq!((s.doc.layers().len(), s.project_name.as_str()), (layers, name.as_str()), "開けなかったので何も替わらない");
    s.apply(Action::NewProject);
    assert!(s.message.contains(busy), "{}", s.message);
    assert_eq!(s.doc.layers().len(), layers, "新規は断られて、文書は替わらない");
    s.apply(Action::Distribute(yolu_app::distribute::DistributeAction::Start));
    assert!(s.message.contains(busy), "{}", s.message);
    assert!(!s.distribute.is_open() && !s.distribute.is_busy());
    assert!(s.is_saving(), "断られても、動いている保存はそのまま");
    // メニューの項目も保存の間は無効で、理由をツールチップに出す（描く・見るは止めない）
    let menu = yolu_app::shell::menu_entries(&s, 0);
    for label in ["新規プロジェクト…", "開く…", "保存", "別名で保存…", "配布用に保存…"] {
        let found = menu.iter().find_map(|e| match e {
            yolu_app::ui::menu::Entry::Item { label: l, enabled, tooltip, .. } if l == label => Some((*enabled, tooltip.clone())),
            _ => None,
        });
        assert_eq!(found, Some((false, Some(busy.to_owned()))), "{label}");
    }
    hold.release();
    s.wait_save();
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    assert!(!other.exists(), "断った別名の保存は何も書かない");
    // 終わったあとはできる
    s.apply(Action::SaveProjectAs(other.clone()));
    s.wait_save();
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    assert!(other.exists());
    s.apply(Action::OpenProject(dir.file("開く先.ylp")));
    assert!(s.message.starts_with("開きました"), "{}", s.message);
    assert!(save_leftovers(&dir.0).is_empty(), "{:?}", save_leftovers(&dir.0));
}

/// 保存が失敗したら、何も変えずに理由を出し、保存の前の「変更あり」を戻す。同期の保存と同じ文（外で書き換えられた・置き換えられた・消された）。
#[test]
fn a_failed_background_save_changes_nothing_and_says_the_same_as_the_inline_save() {
    small().scoped(|| {
        for (lang, changed, deleted) in [
            (Lang::Ja, "外部で変更されています", "外部で消されています"),
            (Lang::En, "Save target or backup changed", "The save target was deleted or moved outside"),
        ] {
            let dir = TempDir::new("fail");
            let path = dir.file("original.ylp");
            let mut s = painted_state(256, 4, 3);
            s.apply(Action::SaveProjectAs(path.clone()));
            let original = std::fs::read(&path).unwrap();
            let mut messages = Vec::new();
            for background in [false, true] {
                std::fs::write(&path, &original).unwrap();
                let mut o = AppState::new_in(64, 64, lang);
                o.apply(Action::OpenProject(path.clone()));
                o.save.background = background;
                // 別のファイルに置き換えられた後は、外の変更として断る（外のファイルを潰さない）
                let theirs = dir.file("theirs.ylp");
                let mut other = painted_state(128, 2, 8);
                other.apply(Action::SaveProjectAs(theirs.clone()));
                std::fs::rename(&theirs, &path).unwrap();
                let outside = std::fs::read(&path).unwrap();
                o.modified = true;
                let doc_before = bytes_of(&o.doc);
                save_and_settle(&mut o, Action::SaveProject);
                assert!(o.message.contains(changed), "{lang:?} {background}: {}", o.message);
                assert!(o.modified, "保存していない印のまま");
                assert!(!o.is_saving());
                assert_eq!(std::fs::read(&path).unwrap(), outside, "外のファイルは潰さない");
                assert_eq!(bytes_of(&o.doc), doc_before);
                let replaced_message = o.message.clone();
                // 消された後は、保存先が外で消されたと断る（新しく作らない）
                std::fs::remove_file(&path).unwrap();
                save_and_settle(&mut o, Action::SaveProject);
                assert!(o.message.contains(deleted) && !o.message.contains(changed), "{lang:?} {background}: {}", o.message);
                assert!(!path.exists());
                assert!(o.modified);
                assert!(save_leftovers(&dir.0).is_empty(), "{:?}", save_leftovers(&dir.0));
                messages.push((replaced_message, o.message.clone()));
            }
            assert_eq!(messages[0], messages[1], "{lang:?}: 裏の保存の断りは、画面のスレッドの保存と同じ文");
        }
    });
}

/// 保存が済まなかったとき（保存先を作れない）、保存の前に「変更なし」だったら変更なしのまま、「変更あり」だったら変更ありのまま戻る。
#[test]
fn a_failed_save_restores_the_modified_mark_it_found() {
    let dir = TempDir::new("mark");
    // フォルダーを作れない場所（通常のファイルの下）
    let blocker = dir.file("ファイル");
    std::fs::write(&blocker, b"x").unwrap();
    let target = blocker.join("下").join("a.ylp");
    for background in [false, true] {
        for modified in [false, true] {
            let mut s = painted_state(128, 2, 1);
            s.save.background = background;
            s.modified = modified;
            s.shelf.changed = modified;
            save_and_settle(&mut s, Action::SaveProjectAs(target.clone()));
            assert!(s.message.starts_with("保存できません"), "{}", s.message);
            assert_eq!((s.modified, s.shelf.changed), (modified, modified), "background {background}");
            assert!(s.project.is_none(), "失敗したら開いたファイルにしない");
        }
    }
}

/// 保存の間は、復旧の書き置きの新しい頼みを出さない（開いた .ylp を置き換える保存と、書き置きの読みを重ねない）。保存が終われば出す。
#[test]
fn recovery_does_not_start_a_write_while_saving() {
    let dir = TempDir::new("recovery");
    let path = dir.file("作品.ylp");
    let probe: SpaceProbe = std::sync::Arc::new(|_| Some(DiskSpace { total: 1000 << 30, available: 900 << 30 }));
    let mut s = AppState::new_in(128, 128, Lang::Ja);
    s.recovery.set_space_probe(Some(probe));
    s.recovery
        .enable(
            dir.file("recovery"),
            RecoverySettings { interval_seconds: 15, strokes_between: 0, generations_to_keep: 3, directory: None, ..RecoverySettings::default() },
        )
        .unwrap();
    s.apply(Action::SaveProjectAs(path.clone()));
    s.wait_save();
    s.save.background = true;
    paint(&mut s, 10.0);
    let t0 = Instant::now();
    s.recovery_tick_at(t0);
    let hold = s.save.hold_next();
    s.apply(Action::SaveProject);
    assert!(s.is_saving());
    // 保存の間は、間隔が過ぎても書き置きを頼まない
    s.recovery_tick_at(t0 + Duration::from_secs(60));
    s.recovery_tick_at(t0 + Duration::from_secs(120));
    assert!(s.recovery.is_idle());
    assert_eq!(s.recovery.checkpoints(), 0);
    hold.release();
    s.wait_save();
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    // 保存で「変更なし」に戻ったので、書く物が無い
    s.recovery_tick_at(t0 + Duration::from_secs(200));
    s.recovery_wait();
    assert_eq!(s.recovery.checkpoints(), 0);
    // 保存のあとに描いた分は、間隔が経てば書く
    paint(&mut s, 40.0);
    s.recovery_tick_at(t0 + Duration::from_secs(201));
    s.recovery_tick_at(t0 + Duration::from_secs(230));
    s.recovery_wait();
    assert_eq!(s.recovery.checkpoints(), 1);
}

/// 進み具合は、保存を頼んでいる間だけあり、0 から 1 の間で、戻らずに進む。終われば消える。
#[test]
fn the_progress_is_only_there_while_saving_and_never_goes_back() {
    let dir = TempDir::new("progress");
    let mut s = painted_state(256, 3, 6);
    assert!(s.save_progress().is_none());
    s.save.background = true;
    let hold = s.save.hold_next();
    s.apply(Action::SaveProjectAs(dir.file("作品.ylp")));
    let first = s.save_progress().expect("保存の間は進み具合がある");
    assert_eq!(first.file, "作品.ylp");
    assert!((0.0..=1.0).contains(&first.fraction), "{first:?}");
    hold.release();
    let mut last = first.fraction;
    let start = Instant::now();
    while s.is_saving() {
        if let Some(p) = s.save_progress() {
            assert!(p.fraction >= last - f32::EPSILON && p.fraction <= 1.0, "{last} → {}", p.fraction);
            last = p.fraction;
        }
        s.poll_save();
        assert!(start.elapsed() < Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(s.save_progress().is_none());
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
}

/// 複数のセットの合成の PNG を並べて作っても、結果は 1 つずつ作るのと同じ（セットの並び・内容）。
#[test]
fn several_sets_are_composited_side_by_side_and_come_out_the_same_as_one_at_a_time() {
    let dir = TempDir::new("sets");
    let mut s = painted_state(128, 2, 7);
    for _ in 0..4 {
        s.apply(Action::Project(yolu_app::newproject::NpAction::AddSet));
    }
    assert!(s.sets.len() >= 3, "{}", s.message);
    for i in 0..s.sets.len() {
        let layer = s.set_doc_mut(i).add_layer("絵").unwrap();
        let ts = s.set_doc(i).tile_size();
        let bytes = noise((ts * ts * 4) as usize, 100 + i as u64);
        s.set_doc_mut(i).import_tile(layer, Channel::Color, TileCoord::new(0, 0), &bytes).unwrap();
    }
    s.modified = true;
    s.apply(Action::SaveProjectAs(dir.file("inline.ylp")));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    assert_eq!(s.rewritten_sets, s.sets.len());
    for i in 0..s.sets.len() {
        s.sets.get_mut(i).unwrap().saved = None;
    }
    s.save.background = true;
    s.apply(Action::SaveProjectAs(dir.file("background.ylp")));
    s.wait_save();
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    assert_eq!(entries_of(&dir.file("inline.ylp")), entries_of(&dir.file("background.ylp")));
    let count = entries_of(&dir.file("background.ylp")).iter().filter(|(n, _)| n.ends_with("composite/Color.png")).count();
    assert_eq!(count, s.sets.len(), "セットごとの合成の PNG");
    // 並べて作った結果は、yolu-io を直に呼んで 1 つずつ作った物差しと同じ（並びも内容も）
    assert_same_entries("物差し", &entries_of(&dir.file("background.ylp")), &reference_entries(&s, &dir.file("reference.ylp")));
}

/// 複数のセットを書き直す保存は、合成を並べる別のスレッドにも、頼んだスレッドの書く形の閾値（試験が小さくした値）を引き継ぐ。引き継がないと、
/// 2 つ以上のセットの正本は本物の閾値で計画され、小さくした閾値で通すはずの大きな形（版 26 の分けた正本）の道が黙って通らなくなる。
#[test]
fn several_sets_build_their_source_under_the_thresholds_of_the_save_request() {
    small().scoped(|| {
        let dir = TempDir::new("setsthreshold");
        let mut s = painted_state(256, 5, 8);
        for _ in 0..2 {
            s.apply(Action::Project(yolu_app::newproject::NpAction::AddSet));
        }
        assert!(s.sets.len() >= 3, "{}", s.message);
        for i in 0..s.sets.len() {
            // どのセットの正本も、分ける閾値（256 KiB）を超える大きさにする
            let mut round = 0;
            while bytes_of(s.set_doc(i)).len() <= 300 << 10 {
                fill_layers(s.set_doc_mut(i), 1, 20 + i as u64 * 10 + round);
                round += 1;
            }
        }
        s.modified = true;
        s.save.background = true;
        s.apply(Action::SaveProjectAs(dir.file("作品.ylp")));
        assert!(s.is_saving());
        s.wait_save();
        assert!(s.message.starts_with("保存しました"), "{}", s.message);
        assert_eq!(s.rewritten_sets, s.sets.len());
        let entries = entries_of(&dir.file("作品.ylp"));
        for set in s.sets.iter() {
            let part = format!("sets/{}/document.utpaint.1", set.id);
            assert!(entries.iter().any(|(n, _)| *n == part), "{} の正本が分かれていない（{part}）", set.name);
        }
        assert_same_entries("物差し", &entries, &reference_entries(&s, &dir.file("reference.ylp")));
    });
}

/// 棚を読めなかった（理由を覚えている）ときは、保存で resources に触らない（開いたファイルのバイト列のまま残る）。棚を変える操作は断られるので、
/// 「変えた」印と同時にはまず起きないが、起きても、読めない棚を空の棚で潰さない。
#[test]
fn an_unreadable_shelf_is_left_as_it_was_by_a_save() {
    let dir = TempDir::new("shelf");
    let path = dir.file("作品.ylp");
    let mut s = painted_state(64, 1, 3);
    s.shelf.add_image(Lang::Ja, "画像", &[200, 100, 50, 255].repeat(16), 4, 4).unwrap();
    assert!(s.shelf.changed);
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let resources = |entries: &[(String, Vec<u8>)]| -> Vec<(String, Vec<u8>)> {
        entries.iter().filter(|(n, _)| n.starts_with("resources")).cloned().collect()
    };
    let before = resources(&entries_of(&path));
    assert!(!before.is_empty(), "棚の画像は resources に書かれる");
    let mut reopened = opened(&path);
    reopened.shelf = yolu_app::shelf::ShelfState::unreadable("読めない棚");
    reopened.shelf.changed = true;
    let layer = reopened.doc.layers()[0].id();
    reopened.doc.set_layer_opacity(layer, 0.5, false).unwrap();
    reopened.modified = true;
    reopened.save.background = true;
    save_and_settle(&mut reopened, Action::SaveProject);
    assert!(reopened.message.starts_with("保存しました"), "{}", reopened.message);
    assert_eq!(resources(&entries_of(&path)), before, "読めない棚の resources はそのまま");
}

/// 大きな文書の時間（`--ignored --nocapture`）。段ごとの時間は、yolu-io の段の知らせで測る。実データは使わない（疑似の画素）。
/// 画面のスレッドが止まる長さは、保存の頼みを出すまで（裏のスレッドを使う状態）と、同期の保存の全体を並べる。
#[test]
#[ignore = "時間を測る。通常の回しには入れない"]
fn timing_of_a_big_document() {
    use std::sync::Arc;
    let size: u32 = std::env::var("YOLU_TIMING_SIZE").ok().and_then(|v| v.parse().ok()).unwrap_or(2048);
    let layers: usize = std::env::var("YOLU_TIMING_LAYERS").ok().and_then(|v| v.parse().ok()).unwrap_or(64);
    let dir = TempDir::new("timing");
    let mut s = AppState::new_in(size, size, Lang::Ja);
    s.doc.set_source_budget_bytes(16 << 30).unwrap();
    fill_like_art(&mut s.doc, layers, 7);
    s.modified = true;
    println!("{size}²・{layers} 層、層の画素 {} MiB", s.doc.allocated_bytes() >> 20);
    // 段ごと（yolu-io を直に呼んで測る）
    let snapshot = Arc::new(s.doc.capture_snapshot().unwrap());
    let t = Instant::now();
    let pngs = yolu_io::composite_pngs(&snapshot).unwrap();
    println!("composite_pngs: {:?} ({} PNG)", t.elapsed(), pngs.len());
    let source = yolu_io::DocumentSource::from_core(snapshot.clone()).unwrap();
    let spec = yolu_io::SetSpec {
        id: yolu_app::sets::guid_string(snapshot.id()),
        name: "セット".into(),
        material: yolu_io::MaterialRef::Unassigned,
        document: Some(source),
        composites: pngs,
    };
    let project = yolu_io::Project::create(yolu_app::project::writer(), std::slice::from_ref(&spec), &spec.id).unwrap();
    let mut target = yolu_io::SaveTarget::create(dir.file("timing.ylp")).unwrap();
    let started = Instant::now();
    let mut last = (started, "開始");
    let mut stages: Vec<(String, Duration)> = Vec::new();
    let report = target
        .save_with_progress(&project, yolu_io::BackupKeep::All, &mut |stage| {
            let now = Instant::now();
            stages.push((last.1.to_string(), now - last.0));
            last = (
                now,
                match stage {
                    yolu_io::SaveStage::Counting => "数える",
                    yolu_io::SaveStage::Writing => "書く",
                    yolu_io::SaveStage::Verifying => "確かめる",
                    yolu_io::SaveStage::Replacing => "置き換える",
                },
            );
        })
        .unwrap();
    stages.push((last.1.to_string(), last.0.elapsed()));
    for (name, d) in &stages {
        println!("  段 {name}: {d:?}");
    }
    println!("save_with: {:?} ({} バイト)", started.elapsed(), report.stamp.length);
    drop((project, spec, target));
    // 画面のスレッドでの保存（同期。今の `Action::SaveProjectAs` の全体）
    let t = Instant::now();
    s.apply(Action::SaveProjectAs(dir.file("inline.ylp")));
    println!("画面のスレッドで保存（新規）: 画面が止まる {:?} {}", t.elapsed(), s.message);
    fill_like_art(&mut s.doc, 1, 99);
    s.modified = true;
    let t = Instant::now();
    s.apply(Action::SaveProject);
    println!("画面のスレッドで保存（上書き・退避つき）: 画面が止まる {:?} {}", t.elapsed(), s.message);
    // 裏のスレッド: 画面が止まるのは保存の頼みを出すまで
    s.save.background = true;
    fill_like_art(&mut s.doc, 1, 98);
    s.modified = true;
    let t = Instant::now();
    s.apply(Action::SaveProject);
    let stall = t.elapsed();
    let mut polls = 0u32;
    let mut slowest = Duration::ZERO;
    while s.is_saving() {
        let t = Instant::now();
        s.poll_save();
        slowest = slowest.max(t.elapsed());
        polls += 1;
        std::thread::sleep(Duration::from_millis(5));
    }
    println!(
        "裏のスレッドで保存（上書き）: 画面が止まる {stall:?}（頼みを出すまで）、結果を受ける 1 回の最長 {slowest:?}（{polls} 回見た） 全体 {:?} {}",
        t.elapsed(),
        s.message
    );
}
