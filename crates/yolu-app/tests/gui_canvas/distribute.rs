//! 配布用に保存: 開いているものを変えずに、作った人が気づかないまま残る物（PSD の原本・使っていないアセット・出どころのパス・モデルの参照・
//! メッシュマップ・古い状態・知らないエントリ）を除いた写しを書く。ウィンドウ・切り替え・保存先・置き換え・断る理由・日英・ウィンドウの画像。
//! 取り込み直した PSD の古い原本が、いつもの保存に残らないことも確かめる。`headless_` で始まる試験は画面を描かない。
//! Unity 版が作った .ylp（PSD の原本・file と unityAsset の出どころ・thumbnail・brush.json・view.json）は試験の中で組み立てる。
use crate::common;
use crate::common::drive::Drive;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use egui_kittest::Harness;
use yolu_app::bake::{BakeAction, BakeBackend};
use yolu_app::distribute::DistributeAction;
use yolu_app::lang::Lang;
use yolu_app::psd::{PsdAction, PsdTarget};
use yolu_app::recovery::{DiskSpace, RecoveryAction, RecoverySettings, SpaceProbe};
use yolu_app::shelf::ShelfOp;
use yolu_app::state::{Action, AppState, DialogRequest};
use yolu_app::YoluApp;
use yolu_core::look::{LookKind, LookValue, MaterialLook, ReceivedLook};
use yolu_core::mesh_maps::MeshMapKind;
use yolu_core::{Channel, Document, EffectInputs, ImageColorSpace, ImageId, ImageInput, Rgba8};
use yolu_io::{
    composite_pngs, shelf::Shelf, Archive, GenerationStore, MaterialRef, NativeDocument, Project,
    Removal, SaveTarget, SetSpec, Thresholds, WriterInfo, INFO_NAME,
};

/// 試験ごとの一時フォルダ（終わったら消す）。
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "yolu-app-distribute-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn files(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(&self.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const SENTINELS: [&str; 5] = [
    "C:\\Users\\tester\\textures\\scratch.png",
    "Assets/Textures/Rust.png",
    "C:\\Users\\tester\\models\\Prop.fbx",
    "0123456789abcdef0123456789abcdef",
    "Prop.fbx",
];

fn writer() -> WriterInfo {
    WriterInfo {
        app: "YoluPainter".into(),
        version: "0.2.0".into(),
        unity: "2022.3.22f1".into(),
    }
}

fn rid(n: u32) -> String {
    format!("{n:08x}-0000-4000-8000-{n:012x}")
}

fn pixels(seed: u32) -> Vec<u8> {
    (0..16u32).map(|i| (seed * 37 + i * 11) as u8 | 1).collect()
}

fn entries_of(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    Project::read(bytes)
        .unwrap()
        .original_archive()
        .entries()
        .iter()
        .map(|(n, b)| (n.clone(), b.bytes().unwrap().to_vec()))
        .collect()
}

fn read_entries(path: &Path) -> BTreeMap<String, Vec<u8>> {
    entries_of(&std::fs::read(path).unwrap())
}

/// Unity 版が作ったような .ylp をディスクに作る: 1 つのセット（描いた画素 1 つ）に、PSD の原本・メッシュマップ・サムネイル・ブラシの設定・
/// モデルの参照のある view.json・知らないエントリ、使っていない棚の画像 2 つ（file と unityAsset の出どころ）。返すのはセットの ID。
fn unity_style(path: &Path) -> String {
    let mut s = AppState::new(64, 64);
    let layer = s.selected_layer().unwrap();
    s.doc_mut()
        .set_pixel(layer, 3, 4, Rgba8::new(200, 10, 20, 255))
        .unwrap();
    let uid = s.sets().current().uid;
    s.rename_set(uid, "Body").unwrap();
    s.act(Action::SaveProjectAs(path.to_path_buf()));
    assert!(s.message().starts_with("保存しました"), "{}", s.message());
    let project = Project::read(&std::fs::read(path).unwrap()).unwrap();
    let id = project.sets()[0].id.clone();
    let mut shelf = Shelf::new(1 << 30);
    shelf
        .add_image(
            &rid(1),
            "Scratch",
            &pixels(1),
            2,
            2,
            "srgb",
            serde_json::json!({"type":"file","path":SENTINELS[0],"sha256":"a".repeat(64),"length":10}),
        )
        .unwrap();
    shelf
        .add_image(
            &rid(2),
            "Rust",
            &pixels(2),
            2,
            2,
            "srgb",
            serde_json::json!({"type":"unityAsset","guid":SENTINELS[3],"path":SENTINELS[1],"stamp":"s","readThroughGpu":false}),
        )
        .unwrap();
    let project = project.with_shelf(&shelf, writer()).unwrap();
    let mut files: BTreeMap<String, Vec<u8>> = project
        .original_archive()
        .entries()
        .iter()
        .map(|(n, b)| (n.clone(), b.bytes().unwrap().to_vec()))
        .collect();
    files.insert(
        format!("sets/{id}/imported-original.psd"),
        b"8BPS-fake-original".to_vec(),
    );
    files.insert(format!("sets/{id}/meshmap-Id.bin"), b"mesh".to_vec());
    files.insert("extra.dat".into(), b"unknown".to_vec());
    files.insert("thumbnail.png".into(), b"old-thumbnail".to_vec());
    files.insert("brush.json".into(), b"{\"schema\":3}".to_vec());
    let view = serde_json::json!({
        "modelAssetGuid": SENTINELS[3],
        "selectedChannel": 0,
        "standaloneModel": { "path": "models/Prop.fbx" },
    });
    files.insert(
        "view.json".into(),
        serde_json::to_vec_pretty(&view).unwrap(),
    );
    std::fs::write(
        path,
        Archive::from_entries(files).unwrap().to_bytes().unwrap(),
    )
    .unwrap();
    id
}

/// 開いた状態（Unity 版が作った .ylp を開き、まだ保存していない変更を 1 つ足したもの）。
fn opened(dir: &TempDir) -> (AppState, PathBuf) {
    let path = dir.path("Work.ylp");
    unity_style(&path);
    let mut s = AppState::new(32, 32);
    s.bake_mut().backend = yolu_app::bake::BakeBackend::Cpu;
    s.act(Action::OpenProject(path.clone()));
    assert!(!s.modified(), "{}", s.message());
    assert!(
        s.project().as_ref().is_some_and(|p| p.is_file()),
        "{}",
        s.message()
    );
    let layer = s
        .selected_layer()
        .unwrap_or_else(|| s.doc().layers()[0].id());
    s.doc_mut()
        .set_pixel(layer, 9, 9, Rgba8::new(1, 2, 3, 255))
        .unwrap();
    s.set_modified(true);
    (s, path)
}

/// ウィンドウを開く（準備が終わるまで待つ）。
fn start(s: &mut AppState) {
    s.act(Action::Distribute(DistributeAction::Start));
    s.wait_distribute();
}

fn kinds(s: &AppState) -> Vec<Removal> {
    s.distribute()
        .window()
        .expect("ウィンドウがある")
        .inventory()
        .kinds()
}

/// 開いているものの目印（文書・Undo・未保存の印・開いているファイル）。
fn fingerprint(s: &AppState, open_path: &Path) -> (u128, u64, bool, bool, bool, PathBuf, Vec<u8>) {
    (
        s.doc().id(),
        s.doc().revision(),
        s.doc().can_undo(),
        s.doc().can_redo(),
        s.modified(),
        s.project().as_ref().unwrap().path().to_path_buf(),
        std::fs::read(open_path).unwrap(),
    )
}

fn text_of(entries: &BTreeMap<String, Vec<u8>>) -> String {
    String::from_utf8_lossy(&entries.values().flatten().copied().collect::<Vec<u8>>()).into_owned()
}

fn paint_psd(path: &Path) {
    let mut s = AppState::new(32, 32);
    s.bake_mut().backend = yolu_app::bake::BakeBackend::Cpu;
    let layer = s.selected_layer().unwrap();
    for y in 0..32 {
        for x in 0..16 {
            s.doc_mut()
                .set_pixel(layer, x, y, Rgba8::new(255, 0, 0, 255))
                .unwrap();
        }
    }
    s.act(Action::Psd(PsdAction::Export(path.to_path_buf())));
    s.wait_psd();
    assert!(path.exists(), "{}", s.message());
}

// ───────── いつもの保存: 取り込み直した PSD の古い原本 ─────────

#[test]
fn headless_a_psd_imported_into_the_current_set_leaves_no_stale_original_in_the_next_save() {
    let dir = TempDir::new("stale");
    let path = dir.path("Work.ylp");
    let id = unity_style(&path);
    let original = format!("sets/{id}/imported-original.psd");
    let mut s = AppState::new(32, 32);
    s.bake_mut().backend = yolu_app::bake::BakeBackend::Cpu;
    s.act(Action::OpenProject(path.clone()));
    assert!(read_entries(&path).contains_key(&original));
    // 描き足して保存しても、同じ文書なので原本は残る（Unity 版が取り込んだ原本のまま）
    let layer = s.doc().layers()[0].id();
    s.doc_mut()
        .set_pixel(layer, 1, 1, Rgba8::new(9, 9, 9, 255))
        .unwrap();
    s.set_modified(true);
    s.act(Action::SaveProject);
    assert!(s.message().starts_with("保存しました"), "{}", s.message());
    assert!(
        read_entries(&path).contains_key(&original),
        "同じ文書の保存では原本を持ち越す"
    );
    // テクスチャセットの大きさを変えても同じ文書（Undo の段として中身を交換する）なので、原本は残る
    let before_resize = s.doc().id();
    s.doc_mut()
        .resize_image(48, 48, yolu_core::CanvasResampling::Area)
        .unwrap();
    assert_eq!(s.doc().id(), before_resize);
    s.set_modified(true);
    s.act(Action::SaveProject);
    assert!(s.message().starts_with("保存しました"), "{}", s.message());
    assert!(
        read_entries(&path).contains_key(&original),
        "大きさの変更の保存でも原本を持ち越す"
    );
    // 今のセットの文書を別の PSD に替える（取り込みの確かめが出れば「取り込む」）
    let psd = dir.path("Other.psd");
    paint_psd(&psd);
    let old_doc = s.doc().id();
    s.act(Action::Psd(PsdAction::Import {
        path: psd,
        target: PsdTarget::CurrentSet,
    }));
    s.wait_psd();
    if s.psd().import_check.is_some() {
        s.act(Action::Psd(PsdAction::ConfirmImport));
    }
    assert_ne!(s.doc().id(), old_doc, "文書が替わった: {}", s.message());
    // 替えた文書の保存には、古い PSD の原本が無い（新しい文書の原本ではない）。セットの ID・名前は同じで、文書は新しい物
    s.act(Action::SaveProject);
    assert!(s.message().starts_with("保存しました"), "{}", s.message());
    let saved = read_entries(&path);
    assert!(
        !saved.contains_key(&original),
        "古い原本が残っている: {:?}",
        saved.keys()
    );
    let project = Project::read(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(project.sets().len(), 1);
    assert_eq!(project.sets()[0].id, id);
    assert_eq!(
        project.sets()[0].document.to_bytes().unwrap(),
        NativeDocument::from_core(s.doc()).unwrap().to_bytes(),
        "保存した文書は取り込んだ文書"
    );
    // 前の版は退避（既定はすべて残す）に残っているので、原本は退避から取り戻せる
    let backups: Vec<PathBuf> = std::fs::read_dir(dir.path("Work.ylp-backups~"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert!(
        backups
            .iter()
            .any(|b| read_entries(b).contains_key(&original)),
        "退避に古い原本が残る"
    );
    // もう一度保存しても同じ（原本は戻らない）
    let first = s.doc().layers()[0].id();
    s.doc_mut()
        .set_pixel(first, 2, 2, Rgba8::new(5, 5, 5, 255))
        .unwrap();
    s.set_modified(true);
    s.act(Action::SaveProject);
    assert!(!read_entries(&path).contains_key(&original));
}

// ───────── 配布用の写し ─────────

#[test]
fn headless_the_copy_has_no_leftovers_and_nothing_that_is_open_changes() {
    let dir = TempDir::new("copy");
    let (mut s, path) = opened(&dir);
    let before = fingerprint(&s, &path);
    let dir_before = dir.files();
    start(&mut s);
    // 当たる種類だけが並ぶ（Unity の値は無いので出ない）
    assert_eq!(
        kinds(&s),
        [
            Removal::PsdOriginals,
            Removal::SourcePaths,
            Removal::ModelReference,
            Removal::MeshMaps,
            Removal::StaleEntries,
            Removal::UnknownEntries
        ]
    );
    let window = s.distribute().window().unwrap();
    assert_eq!(window.removed(), Removal::ALL, "既定は、どの種類も含めない");
    // アセットは 2 つとも使っていないので、既定は外す
    assert_eq!(window.inventory().shelf.len(), 2);
    assert!(window.inventory().shelf.iter().all(|i| !i.used));
    assert!(window.excluded_resources().len() == 2);
    assert!(s.distribute().window_visible());
    let dest = dir.path("Work-dist.ylp");
    s.act(Action::Distribute(DistributeAction::Save(dest.clone())));
    assert!(s.distribute().is_busy(), "{}", s.message());
    s.wait_distribute();
    assert!(
        s.message().starts_with("配布用に保存しました"),
        "{}",
        s.message()
    );
    assert!(!s.distribute().is_open());
    // 開いているものは 1 バイトも変わらない（文書・Undo・未保存の印・開いているファイル）
    assert_eq!(fingerprint(&s, &path), before);
    // 写しには、除くものが無い。レイヤーの画素は元と同じ（保存していない変更も入る）
    let copy = read_entries(&dest);
    let text = text_of(&copy);
    for sentinel in SENTINELS {
        assert!(
            !text.contains(sentinel) && !text.contains(&sentinel.replace('\\', "\\\\")),
            "{sentinel}"
        );
    }
    for name in copy.keys() {
        assert!(
            !name.ends_with(".psd")
                && !name.contains("meshmap-")
                && !["thumbnail.png", "brush.json", "extra.dat", "resources.json"]
                    .contains(&name.as_str())
                && !name.starts_with("resources/"),
            "{name}"
        );
    }
    let project = Project::read(&std::fs::read(&dest).unwrap()).unwrap();
    assert_eq!(project.info().format, 7);
    assert_eq!(project.sets().len(), 1);
    assert_eq!(
        project.sets()[0].document.to_bytes().unwrap(),
        NativeDocument::from_core(s.doc()).unwrap().to_bytes()
    );
    assert!(project.unknown_entries().is_empty());
    assert_eq!(project.view_model().unwrap(), None);
    // 配布用の写しの隣に -backups~ もロックも一時ファイルも作らない
    let mut want = dir_before;
    want.push("Work-dist.ylp".into());
    want.sort();
    assert_eq!(dir.files(), want);
}

/// 開いた .ylp が外で動かされた・消された・別のファイルに置き換えられたあとでも、配布用の写しは開いたときの中身から書ける（残す選びにした
/// PSD の原本・メッシュマップ・知らないエントリは、開いたファイルのバイト列のまま）。開いているものも外のファイルも変えず、何も残さない。
#[test]
fn headless_the_copy_is_written_from_the_opened_contents_after_the_file_moved_or_changed_outside() {
    // 開いたエントリはメモリに残さず、ファイルのハンドルで持つ（パスで開き直す形は、外で変わると読めなくなる）
    Thresholds {
        keep_in_memory: 0,
        ..Thresholds::REAL
    }
    .scoped(|| {
        for how in ["moved", "deleted", "replaced"] {
            let dir = TempDir::new(how);
            let (mut s, path) = opened(&dir);
            let base = read_entries(&path);
            let outside = match how {
                "moved" => {
                    let to = dir.path("Moved.ylp");
                    std::fs::rename(&path, &to).unwrap();
                    Some(to)
                }
                "deleted" => {
                    std::fs::remove_file(&path).unwrap();
                    None
                }
                _ => {
                    plain_project(&dir, "Other.ylp");
                    std::fs::rename(dir.path("Other.ylp"), &path).unwrap();
                    Some(path.clone())
                }
            };
            let theirs = outside.as_ref().map(|p| std::fs::read(p).unwrap());
            let dir_before = dir.files();
            let dest = dir.path("Copy.ylp");
            write_copy(
                &mut s,
                &dest,
                &[
                    Removal::PsdOriginals,
                    Removal::MeshMaps,
                    Removal::UnknownEntries,
                ],
            );
            let copy = read_entries(&dest);
            let kept: Vec<&String> = base
                .keys()
                .filter(|n| {
                    n.ends_with("/imported-original.psd")
                        || n.contains("meshmap-")
                        || *n == "extra.dat"
                })
                .collect();
            assert_eq!(kept.len(), 3, "{how}: {kept:?}");
            for name in kept {
                assert!(
                    copy.get(name) == Some(&base[name]),
                    "{how}: {name} は開いたときのバイト列のまま"
                );
            }
            let project = Project::read(&std::fs::read(&dest).unwrap()).unwrap();
            assert_eq!(
                project.sets()[0].document.to_bytes().unwrap(),
                NativeDocument::from_core(s.doc()).unwrap().to_bytes(),
                "{how}: 保存していない変更も入る"
            );
            // 外のファイルには触らず、写しの隣に一時ファイル・ロック・退避の置き場を作らない
            if let (Some(p), Some(theirs)) = (&outside, &theirs) {
                assert_eq!(&std::fs::read(p).unwrap(), theirs, "{how}");
            }
            let mut want = dir_before;
            want.push("Copy.ylp".into());
            want.sort();
            assert_eq!(dir.files(), want, "{how}");
            assert!(s.modified(), "{how}: 開いているものは変わらない");
        }
    });
}

#[test]
fn headless_the_toggles_keep_what_is_unchecked_and_the_copy_matches_a_plain_save_when_nothing_is_removed(
) {
    let dir = TempDir::new("toggle");
    let (mut s, _path) = opened(&dir);
    start(&mut s);
    // PSD の原本とメッシュマップを残す
    s.act(Action::Distribute(DistributeAction::Toggle(
        Removal::PsdOriginals,
    )));
    s.act(Action::Distribute(DistributeAction::Toggle(
        Removal::MeshMaps,
    )));
    assert!(s
        .distribute()
        .window()
        .unwrap()
        .includes(Removal::PsdOriginals));
    let dest = dir.path("Keep.ylp");
    s.act(Action::Distribute(DistributeAction::Save(dest.clone())));
    s.wait_distribute();
    let copy = read_entries(&dest);
    assert!(
        copy.keys().any(|n| n.ends_with("/imported-original.psd")),
        "{:?}",
        copy.keys()
    );
    assert!(copy.keys().any(|n| n.contains("meshmap-")));
    assert!(!copy.contains_key("thumbnail.png") && !copy.contains_key("extra.dat"));
    // 全部の切り替えを外した写しは、いつもの保存と同じ中身（保存の並びと食い違わない）。いつもの保存は同じフォルダの別の名前へ
    start(&mut s);
    for removal in s.distribute().window().unwrap().removed() {
        s.act(Action::Distribute(DistributeAction::Toggle(removal)));
    }
    include_all_assets(&mut s);
    assert!(s.distribute().window().unwrap().removed().is_empty());
    assert!(s
        .distribute()
        .window()
        .unwrap()
        .excluded_resources()
        .is_empty());
    let plain = dir.path("Plain-copy.ylp");
    s.act(Action::Distribute(DistributeAction::Save(plain.clone())));
    s.wait_distribute();
    let saved = dir.path("Plain-save.ylp");
    s.act(Action::SaveProjectAs(saved.clone()));
    assert!(s.message().starts_with("保存しました"), "{}", s.message());
    let (a, b) = (read_entries(&plain), read_entries(&saved));
    assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>());
    for (name, bytes) in &a {
        assert!(bytes == &b[name], "{name} が違う");
    }
}

#[test]
fn headless_the_open_file_is_never_written_and_an_existing_file_is_replaced_only_after_the_yes() {
    let dir = TempDir::new("replace");
    let (mut s, path) = opened(&dir);
    let before = std::fs::read(&path).unwrap();
    start(&mut s);
    // 開いている作業用のファイル（別の書き方でも同じ場所）には書けない
    for same in [path.clone(), dir.0.join(".").join("Work.ylp")] {
        s.message_mut().clear();
        s.act(Action::Distribute(DistributeAction::Save(same)));
        assert_eq!(s.message(), "開いているファイルには書けません");
        assert!(!s.distribute().is_busy() && s.distribute().replace.is_none());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    s.set_lang(Lang::En);
    s.act(Action::Distribute(DistributeAction::Save(path.clone())));
    assert_eq!(s.message(), "Cannot write over the open file");
    s.set_lang(Lang::Ja);
    // 名前に .ylp が無ければ足す
    let plain_name = dir.path("Release");
    s.act(Action::Distribute(DistributeAction::Save(plain_name)));
    s.wait_distribute();
    assert!(dir.path("Release.ylp").exists(), "{:?}", dir.files());
    // 既にあるファイル: 確認のウィンドウ。やめるとファイルはそのまま、ウィンドウは戻る
    start(&mut s);
    let dest = dir.path("Release.ylp");
    let old = std::fs::read(&dest).unwrap();
    s.act(Action::Distribute(DistributeAction::Toggle(
        Removal::StaleEntries,
    )));
    s.act(Action::Distribute(DistributeAction::Save(dest.clone())));
    assert_eq!(s.distribute().replace.as_deref(), Some(dest.as_path()));
    assert!(!s.distribute().is_busy());
    assert!(
        !s.distribute().window_visible(),
        "確かめの間は除く物のウィンドウを出さない"
    );
    assert_eq!(std::fs::read(&dest).unwrap(), old);
    s.act(Action::Distribute(DistributeAction::CancelReplace));
    assert!(s.distribute().replace.is_none() && s.distribute().window_visible());
    assert_eq!(std::fs::read(&dest).unwrap(), old);
    // 置き換える: 新しい中身になり、前の写しは残さない（-backups~ を作らない）
    s.act(Action::Distribute(DistributeAction::Save(dest.clone())));
    s.act(Action::Distribute(DistributeAction::ConfirmReplace));
    s.wait_distribute();
    assert!(
        s.message().starts_with("配布用に保存しました"),
        "{}",
        s.message()
    );
    let replaced = std::fs::read(&dest).unwrap();
    assert_ne!(replaced, old);
    assert!(
        entries_of(&replaced).contains_key("thumbnail.png"),
        "切り替えを外した物は残る"
    );
    assert!(
        dir.files().iter().all(|n| !n.ends_with("-backups~")),
        "{:?}",
        dir.files()
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    // .ylp として読めないファイルは、確かめても置き換えない（中身はそのまま）
    start(&mut s);
    let junk = dir.path("Junk.ylp");
    std::fs::write(&junk, b"not a ylp").unwrap();
    s.act(Action::Distribute(DistributeAction::Save(junk.clone())));
    s.act(Action::Distribute(DistributeAction::ConfirmReplace));
    s.wait_distribute();
    assert!(
        s.message()
            .starts_with("配布用に保存できません（置き換える先を .ylp として読めません（"),
        "{}",
        s.message()
    );
    assert_eq!(std::fs::read(&junk).unwrap(), b"not a ylp");
    assert!(s.distribute().window_visible(), "書けなかったら選び直せる");
    s.act(Action::Distribute(DistributeAction::CancelWindow));
    assert!(!s.distribute().is_open());
}

#[test]
fn headless_it_refuses_while_drawing_and_a_canceled_job_writes_nothing() {
    let dir = TempDir::new("refuse");
    let (mut s, path) = opened(&dir);
    let before = std::fs::read(&path).unwrap();
    // 描いている間は始められない
    let layer = s.doc().layers()[0].id();
    s.set_selected_layer(Some(layer));
    let stroke = s.begin_paint_stroke(layer, false).unwrap();
    assert!(s.is_stroking());
    s.act(Action::Distribute(DistributeAction::Start));
    assert_eq!(s.message(), "描いている間はできません。");
    assert!(!s.distribute().is_busy() && !s.distribute().is_open());
    s.set_lang(Lang::En);
    s.act(Action::Distribute(DistributeAction::Start));
    assert_eq!(s.message(), "Not while drawing.");
    s.set_lang(Lang::Ja);
    s.doc_mut().end_stroke(stroke).unwrap();
    // ウィンドウが開いたあとに描き始めても、書かない
    start(&mut s);
    let stroke = s.begin_paint_stroke(layer, false).unwrap();
    let dest = dir.path("Never.ylp");
    s.act(Action::Distribute(DistributeAction::Save(dest.clone())));
    assert_eq!(s.message(), "描いている間はできません。");
    assert!(!dest.exists() && !s.distribute().is_busy());
    s.doc_mut().end_stroke(stroke).unwrap();
    // 書いている途中の取消: 何も書かず、ウィンドウに戻る。一時ファイルも残さない
    s.distribute_mut().park_write = true;
    s.act(Action::Distribute(DistributeAction::Save(dest.clone())));
    assert!(s.distribute().is_busy());
    assert!(
        !s.distribute().window_visible(),
        "書いている間はウィンドウを出さない"
    );
    s.act(Action::Distribute(DistributeAction::CancelJob));
    s.wait_distribute();
    assert!(s.message().contains("取り消しました"), "{}", s.message());
    assert!(!dest.exists());
    assert!(s.distribute().window_visible());
    assert_eq!(dir.files(), ["Work.ylp"]);
    s.act(Action::Distribute(DistributeAction::CancelWindow));
    assert!(!s.distribute().is_open());
    // 準備の途中の取消: ウィンドウは出ない
    s.distribute_mut().park_next = true;
    s.act(Action::Distribute(DistributeAction::Start));
    assert!(s.distribute().is_busy());
    s.act(Action::Distribute(DistributeAction::CancelJob));
    s.wait_distribute();
    assert!(s.message().contains("取り消しました"), "{}", s.message());
    assert!(!s.distribute().is_open());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn headless_a_project_with_nothing_to_remove_goes_straight_to_the_file_and_english_messages_are_short(
) {
    let dir = TempDir::new("plain");
    let mut s = AppState::new(32, 32);
    s.bake_mut().backend = yolu_app::bake::BakeBackend::Cpu;
    let base = dir.path("Plain.ylp");
    s.act(Action::SaveProjectAs(base.clone()));
    s.set_lang(Lang::En);
    start(&mut s);
    // 除く物が無いので、ウィンドウを出さずにすぐ保存先を選ぶ
    assert!(s.distribute().is_open() && !s.distribute().window_visible());
    assert!(s.distribute().window().unwrap().inventory().is_empty());
    assert_eq!(*s.dialog_request(), Some(DialogRequest::DistributeSave));
    *s.dialog_request_mut() = None;
    let dest = dir.path("Plain-dist.ylp");
    s.act(Action::Distribute(DistributeAction::Save(dest.clone())));
    s.wait_distribute();
    assert!(
        s.message().starts_with("Saved for distribution: "),
        "{}",
        s.message()
    );
    assert!(!s.distribute().is_open());
    Project::read(&std::fs::read(&dest).unwrap()).unwrap();
    // 名前の既定: 今の名前に短い接尾辞（開いている作業用のファイルと区別する）
    assert_eq!(yolu_app::distribute::default_name(&s), "Plain-dist.ylp");
}

// ───────── いつもの保存と同じ組み立て（状態ごと）・ウィンドウなしの流れの断り・復旧 ─────────

/// アセットを全部、写しに含める選びにする（ウィンドウを開いているとき）。
fn include_all_assets(s: &mut AppState) {
    let mut kinds = Vec::new();
    for item in &s
        .distribute()
        .window()
        .expect("ウィンドウがある")
        .inventory()
        .shelf
    {
        if !kinds.contains(&item.kind) {
            kinds.push(item.kind);
        }
    }
    for kind in kinds {
        s.act(Action::Distribute(DistributeAction::SetKind(kind, true)));
    }
}

/// ウィンドウを開いて、`keep` の種類は写しに含める選びにして、`dest` へ書く（除く物が無くウィンドウなしで保存先に進む流れも通す）。
/// アセットは既定（使っている物を含め、使っていない物を外す）のまま。
fn write_copy(s: &mut AppState, dest: &Path, keep: &[Removal]) {
    write_copy_with(s, dest, keep, false);
}

/// `write_copy` で、`all_assets` ならアセットも全部含める。
fn write_copy_with(s: &mut AppState, dest: &Path, keep: &[Removal], all_assets: bool) {
    start(s);
    for removal in keep {
        let window = s.distribute().window().expect("ウィンドウがある");
        if !window.includes(*removal) {
            s.act(Action::Distribute(DistributeAction::Toggle(*removal)));
        }
    }
    if all_assets {
        include_all_assets(s);
    }
    *s.dialog_request_mut() = None;
    s.act(Action::Distribute(DistributeAction::Save(
        dest.to_path_buf(),
    )));
    s.wait_distribute();
    assert!(
        s.message().starts_with("配布用に保存しました"),
        "{}",
        s.message()
    );
}

/// 何も除かない写しは、いつもの「別名で保存」と全エントリが同じ（保存の並びと食い違わない）。写しを先に書き、そのあとで保存する
/// （保存は開いているファイルをつなぎ替えるので、1 つの状態で 1 回だけ）。
fn assert_copy_matches_plain_save(s: &mut AppState, dir: &TempDir, sub: &str) {
    std::fs::create_dir_all(dir.path(sub)).unwrap();
    let copy = dir.path(&format!("{sub}/Copy.ylp"));
    write_copy_with(s, &copy, &Removal::ALL, true);
    let plain = dir.path(&format!("{sub}/Plain.ylp"));
    s.act(Action::SaveProjectAs(plain.clone()));
    assert!(s.message().starts_with("保存しました"), "{}", s.message());
    let (a, b) = (read_entries(&copy), read_entries(&plain));
    assert_eq!(
        a.keys().collect::<Vec<_>>(),
        b.keys().collect::<Vec<_>>(),
        "{sub}"
    );
    for (name, bytes) in &a {
        assert!(bytes == &b[name], "{sub}: {name} が違う");
    }
}

fn plain_project(dir: &TempDir, name: &str) -> AppState {
    let mut s = AppState::new(32, 32);
    s.bake_mut().backend = BakeBackend::Cpu;
    s.act(Action::SaveProjectAs(dir.path(name)));
    assert!(s.message().starts_with("保存しました"), "{}", s.message());
    s
}

fn received_values() -> ReceivedLook {
    let mut look = MaterialLook {
        kind: LookKind::LilToon,
        shader: "Hidden/lilToonOutline".into(),
        ..Default::default()
    };
    look.properties
        .insert("_UseShadow".into(), LookValue::Float(1.0));
    ReceivedLook {
        look,
        source: "lilToon 2.3.4".into(),
        ..Default::default()
    }
}

#[test]
fn headless_a_changed_shelf_is_written_like_a_plain_save_and_the_unused_items_go() {
    let dir = TempDir::new("shelf");
    let (mut s, _path) = opened(&dir);
    let layer = s.selected_layer().unwrap();
    s.act(Action::Shelf(ShelfOp::SaveMaterial(layer)));
    assert!(s.shelf().changed, "{}", s.message());
    let resources = s.shelf().resources().len();
    assert_eq!(
        resources, 3,
        "開いた棚の 2 つに、今のレイヤーのスマートマテリアル"
    );
    // 除く選びなら、使っていない 3 つとも消えて棚の索引も無い。残す選びなら新しい素材ごと残る（いつもの保存と同じ）
    let removed = dir.path("Removed.ylp");
    write_copy(&mut s, &removed, &[]);
    let entries = read_entries(&removed);
    assert!(
        !entries.contains_key("resources.json")
            && entries.keys().all(|n| !n.starts_with("resources/")),
        "{:?}",
        entries.keys()
    );
    let kept = dir.path("Kept.ylp");
    write_copy_with(&mut s, &kept, &[], true);
    assert_eq!(
        Project::read(&std::fs::read(&kept).unwrap())
            .unwrap()
            .resources()
            .len(),
        resources
    );
    assert!(s.shelf().changed, "開いているものは変わらない");
    assert_copy_matches_plain_save(&mut s, &dir, "plain");
}

#[test]
fn headless_a_mesh_map_that_is_not_in_the_file_yet_is_written_like_a_plain_save_and_goes() {
    let dir = TempDir::new("maps");
    let mut s = plain_project(&dir, "Base.ylp");
    s.act(Action::LoadDemoModel);
    s.bake_mut().settings.maps = vec![MeshMapKind::WorldNormal, MeshMapKind::AmbientOcclusion];
    s.bake_mut().settings.ao_samples = 8;
    s.bake_mut().settings.padding = 4;
    s.act(Action::Bake(BakeAction::Start));
    s.wait_bake();
    assert_eq!(s.sets().current().mesh_maps.unsaved().len(), 2);
    // 除く選びでは、焼いたマップは写しに入らない。書いても、開いているセットのマップは未保存のまま（保存済みにしない）
    let removed = dir.path("Removed.ylp");
    write_copy(&mut s, &removed, &[]);
    assert!(
        read_entries(&removed)
            .keys()
            .all(|n| !n.contains("meshmap-")),
        "{:?}",
        read_entries(&removed).keys()
    );
    assert_eq!(s.sets().current().mesh_maps.unsaved().len(), 2);
    // 残す選びなら、いつもの保存と同じ（マップのエントリが入る）
    assert_copy_matches_plain_save(&mut s, &dir, "plain");
    assert!(read_entries(&dir.path("plain/Copy.ylp"))
        .keys()
        .any(|n| n.contains("meshmap-")));
}

#[test]
fn headless_the_received_values_follow_the_setting_and_the_choice() {
    // 保存する設定（既定）: 残す選びはいつもの保存と同じ（Unity の値が入る）。除く選びでは look.json の received が無い
    let dir = TempDir::new("received");
    let mut s = plain_project(&dir, "Base.ylp");
    s.doc_mut()
        .set_received_look(Some(received_values()))
        .unwrap();
    let first = s.doc().layers()[0].id();
    s.doc_mut()
        .set_pixel(first, 1, 1, Rgba8::new(1, 2, 3, 255))
        .unwrap();
    s.set_modified(true);
    start(&mut s);
    assert_eq!(kinds(&s), [Removal::UnityValues]);
    s.act(Action::Distribute(DistributeAction::CancelWindow));
    let id = s.sets().current().id.clone();
    let removed = dir.path("Removed.ylp");
    write_copy(&mut s, &removed, &[]);
    let copy = Project::read(&std::fs::read(&removed).unwrap()).unwrap();
    assert_eq!(copy.received_look(&id).unwrap(), None);
    assert_copy_matches_plain_save(&mut s, &dir, "plain");
    let plain = Project::read(&std::fs::read(dir.path("plain/Plain.ylp")).unwrap()).unwrap();
    assert!(
        plain.received_look(&id).unwrap().is_some(),
        "いつもの保存は Unity の値を残す"
    );
    // 保存しない設定: 値は保存にも写しにも入らず、除く種類も出ない
    let off = TempDir::new("received-off");
    let mut t = plain_project(&off, "Base.ylp");
    t.prefs_mut().settings.livelink_keep_values = false;
    t.doc_mut()
        .set_received_look(Some(received_values()))
        .unwrap();
    let first = t.doc().layers()[0].id();
    t.doc_mut()
        .set_pixel(first, 1, 1, Rgba8::new(1, 2, 3, 255))
        .unwrap();
    t.set_modified(true);
    start(&mut t);
    assert!(t.distribute().window().unwrap().inventory().is_empty());
    t.act(Action::Distribute(DistributeAction::CancelWindow));
    assert_copy_matches_plain_save(&mut t, &off, "plain");
    let id = t.sets().current().id.clone();
    let plain = Project::read(&std::fs::read(off.path("plain/Plain.ylp")).unwrap()).unwrap();
    assert_eq!(plain.received_look(&id).unwrap(), None);
}

/// 手動の ID の色は文書の一部なので、配布用の写しでも残る（モデルの参照などを除いても消えない）。何も除かない写しは、いつもの保存と全エントリが同じ。
#[test]
fn headless_manual_id_colors_stay_in_the_distribution_copy() {
    let dir = TempDir::new("id-colors");
    let mut s = plain_project(&dir, "Plain.ylp");
    let colors = yolu_core::mesh_maps::IdColorAssignments::new(
        "0123456789abcdef".repeat(4),
        [(0usize, 0x112233u32), (2, 0xffeedd)].into_iter().collect(),
    )
    .unwrap();
    s.doc_mut().set_id_colors(colors.clone(), false).unwrap();
    s.set_modified(true);
    // 除く選びの初めのまま（モデルの参照・メッシュマップなどを除く）でも、色は残る
    let copy = dir.path("Copy.ylp");
    write_copy(&mut s, &copy, &[]);
    let mut opened = AppState::new(32, 32);
    opened.act(Action::OpenProject(copy));
    assert_eq!(opened.doc().id_colors().binding(), colors.binding());
    assert_eq!(opened.doc().id_colors().colors(), colors.colors());
    assert!(!opened.doc().can_undo());
    assert!(opened.sets().get(0).unwrap().read_only.is_none());
    // 何も除かない写しは、いつもの保存と同じ（色の塊も）
    assert_copy_matches_plain_save(&mut s, &dir, "keep");
}

/// 描けるセット 1 つと、core で扱えない中身で読むだけになるセット 1 つ（どちらも PSD の原本つき）の .ylp。返すのは（描けるセット・
/// 読むだけのセット）の ID。
fn read_only_style(path: &Path) -> (String, String) {
    let rich = crate::common::core_refused::native_core_cannot_hold();
    assert!(
        !rich.core_issues().is_empty(),
        "core で扱えない中身がある正本"
    );
    let (blank, _) = yolu_app::state::blank_document(64, 64);
    let editable = yolu_app::sets::guid_string(blank.id());
    let readonly = "00000000-0000-4000-8000-0000000000aa".to_owned();
    let spec = |id: &str, name: &str, slot: u16, doc: NativeDocument| SetSpec {
        id: id.into(),
        name: name.into(),
        materials: vec![MaterialRef::PendingSlot(slot)],
        document: Some(doc.into()),
        composites: vec![],
    };
    let project = Project::create(
        writer(),
        &[
            spec(
                &editable,
                "Paintable",
                0,
                NativeDocument::from_core(&blank).unwrap(),
            ),
            spec(&readonly, "Preview", 1, rich),
        ],
        &editable,
    )
    .unwrap();
    SaveTarget::create(path).unwrap().save(&project).unwrap();
    let mut files: BTreeMap<String, Vec<u8>> = read_entries(path);
    files.insert(
        format!("sets/{editable}/imported-original.psd"),
        b"8BPS-original-paintable".to_vec(),
    );
    files.insert(
        format!("sets/{readonly}/imported-original.psd"),
        b"8BPS-original-preview".to_vec(),
    );
    std::fs::write(
        path,
        Archive::from_entries(files).unwrap().to_bytes().unwrap(),
    )
    .unwrap();
    (editable, readonly)
}

#[test]
fn headless_a_read_only_set_keeps_its_original_in_every_save_and_the_copy_matches_a_plain_save() {
    let dir = TempDir::new("readonly");
    let path = dir.path("Mixed.ylp");
    let (editable, readonly) = read_only_style(&path);
    let kept = |name: &str| format!("sets/{name}/imported-original.psd");
    let open = || {
        let mut s = AppState::new(32, 32);
        s.bake_mut().backend = BakeBackend::Cpu;
        s.act(Action::OpenProject(path.clone()));
        assert_eq!(s.sets().len(), 2, "{}", s.message());
        assert!(
            s.sets().get(1).unwrap().read_only.is_some(),
            "{}",
            s.message()
        );
        s
    };
    // いつもの保存: 見せるだけの写しの文書は毎回新しい ID だが、読むだけのセットの原本は落とさない（PSD の未対応情報を黙って捨てない）
    let mut s = open();
    let first = s.doc().layers()[0].id();
    s.doc_mut()
        .set_pixel(first, 2, 2, Rgba8::new(9, 9, 9, 255))
        .unwrap();
    s.set_modified(true);
    let saved = dir.path("Saved.ylp");
    s.act(Action::SaveProjectAs(saved.clone()));
    assert!(s.message().starts_with("保存しました"), "{}", s.message());
    let entries = read_entries(&saved);
    assert!(
        entries.contains_key(&kept(&readonly)),
        "読むだけのセットの原本"
    );
    assert!(
        entries.contains_key(&kept(&editable)),
        "同じ文書の保存では原本を持ち越す"
    );
    // 配布用の写し: PSD の原本を残す選びでは、読むだけのセットの原本も残る。除く選びでは（利用者の選びで）両方とも消える
    let mut s = open();
    let first = s.doc().layers()[0].id();
    s.doc_mut()
        .set_pixel(first, 2, 2, Rgba8::new(9, 9, 9, 255))
        .unwrap();
    s.set_modified(true);
    let kept_copy = dir.path("Kept.ylp");
    write_copy(&mut s, &kept_copy, &[Removal::PsdOriginals]);
    let entries = read_entries(&kept_copy);
    assert!(
        entries.contains_key(&kept(&readonly)),
        "{:?}",
        entries.keys()
    );
    assert!(entries.contains_key(&kept(&editable)));
    let removed = dir.path("Removed.ylp");
    write_copy(&mut s, &removed, &[]);
    let entries = read_entries(&removed);
    assert!(!entries.contains_key(&kept(&readonly)) && !entries.contains_key(&kept(&editable)));
    // 読むだけのセットは作り直さず、正本のバイト列のまま。何も除かない写しはいつもの保存と同じ
    let rich = read_entries(&path)[&format!("sets/{readonly}/document.utpaint")].clone();
    assert_eq!(entries[&format!("sets/{readonly}/document.utpaint")], rich);
    assert_copy_matches_plain_save(&mut s, &dir, "plain");
}

#[test]
fn headless_a_read_only_set_without_its_original_document_is_left_out_of_the_copy_and_a_copy_of_nothing_is_refused(
) {
    let dir = TempDir::new("readonly-missing");
    let path = dir.path("Mixed.ylp");
    let (editable, readonly) = read_only_style(&path);
    let mut s = AppState::new(32, 32);
    s.act(Action::OpenProject(path.clone()));
    assert!(
        s.sets().get(1).unwrap().read_only.is_some(),
        "{}",
        s.message()
    );
    // 開いたファイルとのつながりが無い（元の正本が無い）: 読むだけのセットだけを写しに入れず、書けるセットは入れる。知らせが入れなかったセットを言う
    *s.project_mut() = None;
    let copy = dir.path("Copy.ylp");
    write_copy(&mut s, &copy, &[]);
    assert_eq!(
        s.message(),
        format!(
            "配布用に保存しました: {}。 テクスチャセット「Preview」は読めないため、保存に入れていません。",
            copy.display()
        )
    );
    let entries = read_entries(&copy);
    assert!(entries
        .keys()
        .any(|k| k.starts_with(&format!("sets/{editable}/"))));
    assert!(
        !entries.keys().any(|k| k.contains(&readonly)),
        "{:?}",
        entries.keys()
    );
    s.set_lang(Lang::En);
    let english = dir.path("English.ylp");
    start(&mut s);
    *s.dialog_request_mut() = None;
    s.act(Action::Distribute(DistributeAction::Save(english.clone())));
    s.wait_distribute();
    assert_eq!(
        s.message(),
        format!(
            "Saved for distribution: {}. Texture set \"Preview\" could not be read and was left out of the save.",
            english.display()
        )
    );
    // 書けるセットが 1 つも無ければ、断る（ウィンドウも開かない）
    s.sets_mut().get_mut(0).unwrap().read_only = Some("reason".into());
    s.act(Action::Distribute(DistributeAction::Start));
    assert_eq!(
        s.message(),
        "Cannot save for distribution (\"Paintable\", \"Preview\" cannot be read and have never been saved)."
    );
    assert!(!s.distribute().is_open() && !s.distribute().is_busy());
    s.set_lang(Lang::Ja);
    s.act(Action::Distribute(DistributeAction::Start));
    assert_eq!(
        s.message(),
        "配布用に保存できません（「Paintable」「Preview」は保存したことが無く、読めません）。"
    );
    assert!(!s.distribute().is_open());
}

#[test]
fn headless_an_old_format_file_is_upgraded_in_the_copy_like_a_plain_save() {
    let dir = TempDir::new("format6");
    let path = dir.path("format6.ylp");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../yolu-io/tests/fixtures/format6.ylp"),
        &path,
    )
    .unwrap();
    let before = std::fs::read(&path).unwrap();
    let mut s = AppState::new(32, 32);
    s.bake_mut().backend = BakeBackend::Cpu;
    s.act(Action::OpenProject(path.clone()));
    assert_eq!(s.project().as_ref().unwrap().format(), 6, "{}", s.message());
    let first = s.doc().layers()[0].id();
    s.doc_mut()
        .set_pixel(first, 5, 5, Rgba8::new(7, 8, 9, 255))
        .unwrap();
    s.set_modified(true);
    let copy = dir.path("Copy7.ylp");
    write_copy(&mut s, &copy, &[]);
    let project = Project::read(&std::fs::read(&copy).unwrap()).unwrap();
    assert_eq!(project.info().format, 7, "写しは形式 7 に移行して書く");
    assert_eq!(project.sets().len(), 3);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "開いている形式 6 のファイルは変わらない"
    );
    assert_eq!(s.project().as_ref().unwrap().format(), 6);
    assert_copy_matches_plain_save(&mut s, &dir, "plain");
}

#[test]
fn headless_the_model_reference_is_pointed_again_from_where_the_copy_is_written() {
    let dir = TempDir::new("model");
    let (mut s, _path) = opened(&dir);
    s.np_mut().model_file = Some(dir.path("models/Prop.fbx"));
    // 除かない選び: 保存先のフォルダからの相対のパスに付け直す（同じフォルダ・1 つ下のフォルダ・ちがうフォルダ）
    for (sub, want) in [
        ("", "models/Prop.fbx"),
        ("out", "../models/Prop.fbx"),
        ("a/b", "../../models/Prop.fbx"),
    ] {
        std::fs::create_dir_all(dir.path(sub)).unwrap();
        let dest = dir.path(sub).join("Model.ylp");
        write_copy(&mut s, &dest, &[Removal::ModelReference]);
        let project = Project::read(&std::fs::read(&dest).unwrap()).unwrap();
        assert_eq!(
            project.view_model().unwrap().as_deref(),
            Some(want),
            "{sub}"
        );
    }
    // いつもの保存を同じ場所へ書いたものと、view.json のバイト列まで同じ
    std::fs::create_dir_all(dir.path("out")).unwrap();
    let copy = dir.path("out/Same.ylp");
    write_copy(&mut s, &copy, &Removal::ALL);
    let plain = dir.path("out/Plain.ylp");
    s.act(Action::SaveProjectAs(plain.clone()));
    assert!(s.message().starts_with("保存しました"), "{}", s.message());
    assert_eq!(
        read_entries(&copy)["view.json"],
        read_entries(&plain)["view.json"]
    );
    // 除く選び: 参照も GUID も無い
    let removed = dir.path("Removed.ylp");
    let (mut t, _) = opened(&TempDir::new("model-removed"));
    t.np_mut().model_file = Some(dir.path("models/Prop.fbx"));
    write_copy(&mut t, &removed, &[]);
    let project = Project::read(&std::fs::read(&removed).unwrap()).unwrap();
    assert_eq!(project.view_model().unwrap(), None);
    assert!(!text_of(&read_entries(&removed)).contains("Prop.fbx"));
}

/// テキストレイヤーのフォントのファイル（利用者が入れたフォントの場所）は、写しではファイル名だけになる。作業用のファイルには場所が入るが、
/// 写しのどのエントリ（JSON・文書・画像・名前）にも、作った人の道（試験の一時のフォルダー・ホーム・ユーザー名）が無い。フォントの SHA-256・名前は残り、
/// 写しを開くと名前でフォントを探せる。
#[test]
fn headless_a_font_file_of_a_text_layer_keeps_only_its_file_name_and_no_entry_holds_the_authors_paths(
) {
    use yolu_core::text::{file_font, TextFont, TextSettings};
    const FONT: &[u8] = include_bytes!("../../assets/fonts/BIZUDPGothic-Regular.ttf");
    let dir = TempDir::new("font");
    let font_file = dir.path("Fonts/Example Face.ttf");
    std::fs::create_dir_all(font_file.parent().unwrap()).unwrap();
    std::fs::write(&font_file, FONT).unwrap();
    let (mut s, _path) = opened(&dir);
    let font = file_font(&font_file.to_string_lossy(), 0, FONT);
    let TextFont::File { sha256, names, .. } = font.clone() else {
        unreachable!()
    };
    s.doc_mut()
        .add_text_layer(
            "文字",
            TextSettings::new("Text", font, 4.0, 10.0),
            FONT,
            None,
            false,
        )
        .unwrap();
    s.set_modified(true);
    let authors_paths = {
        let mut v = vec![dir.0.to_string_lossy().into_owned(), "tester".to_owned()];
        v.extend(std::env::var("HOME").ok().filter(|h| h.len() > 1));
        v.extend(std::env::var("USERPROFILE").ok().filter(|h| h.len() > 1));
        v
    };
    let holds = |entries: &BTreeMap<String, Vec<u8>>, needle: &str| {
        let escaped = needle.replace('\\', "\\\\");
        entries.keys().any(|n| n.contains(needle))
            || text_of(entries).contains(needle)
            || text_of(entries).contains(&escaped)
    };
    // 保存した作業用のファイルには、フォントの場所が入る（ここは作った人の作業のファイル）
    let plain = dir.path("Plain.ylp");
    s.act(Action::SaveProjectAs(plain.clone()));
    assert!(s.message().starts_with("保存しました"), "{}", s.message());
    assert!(holds(&read_entries(&plain), &dir.0.to_string_lossy()));
    // 配布用の写し（全部除く）: どのエントリにも作った人の道が無い
    let dest = dir.path("Font-dist.ylp");
    write_copy(&mut s, &dest, &[]);
    let copy = read_entries(&dest);
    for needle in &authors_paths {
        assert!(!holds(&copy, needle), "{needle} が写しに残る");
    }
    // フォントの項目はファイル名だけ。SHA-256・名前は残る
    let project = Project::read(&std::fs::read(&dest).unwrap()).unwrap();
    let doc = project.sets()[0].document.to_native().unwrap();
    let text_of_field = |suffix: &str| {
        doc.fields()
            .iter()
            .find(|f| f.path.ends_with(suffix))
            .map(|f| f.value.clone())
    };
    assert_eq!(
        text_of_field(".text.font_path"),
        Some(yolu_io::NativeValue::Text("Example Face.ttf".into()))
    );
    assert_eq!(
        text_of_field(".text.font_family"),
        Some(yolu_io::NativeValue::Text(names.family.clone()))
    );
    let hex: String = sha256.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        text_of_field(".text.font_sha256"),
        Some(yolu_io::NativeValue::Text(hex))
    );
    // 開いて、名前でフォントを探せる（ファイル名だけの道は使わない）
    let list = yolu_io::fonts::SystemFonts::from_dirs(&[font_file.parent().unwrap()]);
    let mut t = AppState::new(32, 32);
    t.act(Action::OpenProject(dest));
    let layer = t
        .doc()
        .layers()
        .iter()
        .find(|l| l.text().is_some())
        .unwrap();
    let wanted = layer.text().unwrap().font.clone();
    assert!(
        matches!(
            yolu_io::fonts::find(&wanted, &list),
            yolu_io::fonts::Lookup::Same { .. }
        ),
        "{wanted:?}"
    );
}

/// ウィンドウなしの流れ（除く物が 1 つも無い）で保存先を決めたところから、断られて戻る。
fn windowless(dir: &TempDir) -> (AppState, PathBuf) {
    let s = plain_project(dir, "Plain.ylp");
    (s, dir.path("Plain.ylp"))
}

fn open_windowless(s: &mut AppState) {
    s.act(Action::Distribute(DistributeAction::Start));
    s.wait_distribute();
    assert!(
        s.distribute().is_open() && !s.distribute().window_visible(),
        "{}",
        s.message()
    );
    assert_eq!(*s.dialog_request(), Some(DialogRequest::DistributeSave));
    *s.dialog_request_mut() = None;
}

#[test]
fn headless_a_refusal_without_the_window_closes_it_and_the_menu_works_again() {
    let dir = TempDir::new("windowless");
    let (mut s, open_path) = windowless(&dir);
    let layer = s.doc().layers()[0].id();
    s.set_selected_layer(Some(layer));
    // 開いているファイルを選んだ
    open_windowless(&mut s);
    s.act(Action::Distribute(DistributeAction::Save(
        open_path.clone(),
    )));
    assert_eq!(s.message(), "開いているファイルには書けません");
    assert!(!s.distribute().is_open(), "断ったら準備した写しを閉じる");
    // 描いている最中に保存先が決まった
    open_windowless(&mut s);
    let stroke = s.begin_paint_stroke(layer, false).unwrap();
    s.act(Action::Distribute(DistributeAction::Save(
        dir.path("Never.ylp"),
    )));
    assert_eq!(s.message(), "描いている間はできません。");
    assert!(!s.distribute().is_open() && !dir.path("Never.ylp").exists());
    s.doc_mut().end_stroke(stroke).unwrap();
    // 置き換える確かめのあと、描いている最中に「置き換える」
    open_windowless(&mut s);
    let existing = dir.path("Existing.ylp");
    std::fs::write(&existing, b"x").unwrap();
    s.act(Action::Distribute(DistributeAction::Save(existing.clone())));
    assert_eq!(s.distribute().replace.as_deref(), Some(existing.as_path()));
    let stroke = s.begin_paint_stroke(layer, false).unwrap();
    s.act(Action::Distribute(DistributeAction::ConfirmReplace));
    assert_eq!(s.message(), "描いている間はできません。");
    assert!(!s.distribute().is_open() && s.distribute().replace.is_none());
    assert_eq!(std::fs::read(&existing).unwrap(), b"x");
    s.doc_mut().end_stroke(stroke).unwrap();
    // 保存先を選ばなかった（ウィンドウなしの流れ）
    open_windowless(&mut s);
    s.act(Action::Distribute(DistributeAction::CancelWindow));
    assert!(!s.distribute().is_open());
    // どの断りのあとも、メニューからやり直せて、書ける
    open_windowless(&mut s);
    let dest = dir.path("Dist.ylp");
    s.act(Action::Distribute(DistributeAction::Save(dest.clone())));
    s.wait_distribute();
    assert!(
        s.message().starts_with("配布用に保存しました"),
        "{}",
        s.message()
    );
    assert!(dest.exists() && !s.distribute().is_open());
}

#[test]
fn headless_a_refusal_with_the_window_shows_the_window_again() {
    let dir = TempDir::new("windowed-refusal");
    let (mut s, path) = opened(&dir);
    let before = std::fs::read(&path).unwrap();
    start(&mut s);
    assert!(s.distribute().window_visible());
    s.act(Action::Distribute(DistributeAction::Save(path.clone())));
    assert_eq!(s.message(), "開いているファイルには書けません");
    assert!(s.distribute().window_visible(), "選び直せる");
    let layer = s.doc().layers()[0].id();
    s.set_selected_layer(Some(layer));
    let existing = dir.path("Existing.ylp");
    std::fs::write(&existing, b"x").unwrap();
    s.act(Action::Distribute(DistributeAction::Save(existing.clone())));
    assert!(!s.distribute().window_visible());
    let stroke = s.begin_paint_stroke(layer, false).unwrap();
    s.act(Action::Distribute(DistributeAction::ConfirmReplace));
    assert_eq!(s.message(), "描いている間はできません。");
    assert!(
        s.distribute().window_visible(),
        "確かめを閉じてウィンドウに戻る"
    );
    assert_eq!(std::fs::read(&existing).unwrap(), b"x");
    s.doc_mut().end_stroke(stroke).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

fn plenty() -> SpaceProbe {
    const GIB: u64 = 1 << 30;
    Arc::new(|_| {
        Some(DiskSpace {
            total: 1000 * GIB,
            available: 900 * GIB,
        })
    })
}

fn recovery_session(root: &Path) -> AppState {
    let mut s = AppState::new_in(32, 32, Lang::Ja);
    s.bake_mut().backend = BakeBackend::Cpu;
    s.recovery_mut().set_space_probe(Some(plenty()));
    s.recovery_enable(
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
    s
}

#[test]
fn headless_a_checkpoint_after_a_psd_reimport_has_no_stale_original_and_neither_has_the_save_after_recovery(
) {
    let dir = TempDir::new("recovered");
    let root = dir.path("recovery");
    let path = dir.path("Work.ylp");
    let id = unity_style(&path);
    let original = format!("sets/{id}/imported-original.psd");
    let mut s = recovery_session(&root);
    s.act(Action::OpenProject(path.clone()));
    assert!(read_entries(&path).contains_key(&original));
    // 今のセットの文書を別の PSD に替える
    let psd = dir.path("Other.psd");
    paint_psd(&psd);
    let old_doc = s.doc().id();
    s.act(Action::Psd(PsdAction::Import {
        path: psd,
        target: PsdTarget::CurrentSet,
    }));
    s.wait_psd();
    if s.psd().import_check.is_some() {
        s.act(Action::Psd(PsdAction::ConfirmImport));
    }
    assert_ne!(s.doc().id(), old_doc, "文書が替わった: {}", s.message());
    // 書き置き: 古い原本を持ち越さない（開き直したときの次の保存の元になるため）
    let t0 = Instant::now();
    s.recovery_tick_at(t0);
    s.recovery_tick_at(t0 + Duration::from_secs(16));
    s.recovery_wait();
    assert_eq!(s.recovery().checkpoints(), 1, "{}", s.message());
    let store = GenerationStore::new(s.recovery().session_dir().unwrap());
    let mut files = store.load().unwrap().files;
    files.remove(INFO_NAME);
    let checkpoint = Project::from_entries(files).unwrap();
    assert!(
        !checkpoint
            .original_archive()
            .entries()
            .contains_key(&original),
        "書き置きに古い原本が残っている"
    );
    assert_eq!(
        checkpoint.sets()[0].document.to_bytes().unwrap(),
        NativeDocument::from_core(s.doc()).unwrap().to_bytes()
    );
    // 落ちて、復旧で開いて保存しても、古い原本は戻らない
    assert!(s.recovery().is_idle());
    drop(s);
    let mut s2 = recovery_session(&root);
    s2.recovery_apply(RecoveryAction::Open);
    assert!(s2.message().starts_with("復旧しました"), "{}", s2.message());
    let saved = dir.path("Recovered.ylp");
    s2.act(Action::SaveProjectAs(saved.clone()));
    assert!(s2.message().starts_with("保存しました"), "{}", s2.message());
    let entries = read_entries(&saved);
    assert!(
        !entries.contains_key(&original),
        "復旧のあとの保存に古い原本が戻った: {:?}",
        entries.keys()
    );
    // 元の作業用のファイルには手を付けていない（古い原本はそこに残る）
    assert!(read_entries(&path).contains_key(&original));
}

// ───────── ウィンドウ（画面） ─────────

fn settle(h: &mut Harness<'_, YoluApp>) {
    let start = Instant::now();
    loop {
        h.step();
        if !h.distribute().is_busy() {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "仕事が終わらない（ハング検出上限）"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    h.run();
}

fn apply(h: &mut Harness<'_, YoluApp>, action: Action) {
    h.act(action);
    h.run();
}

fn window_texts(h: &Harness<'_, YoluApp>, window: &str) -> Vec<String> {
    use egui::epaint::Shape;
    fn walk(shape: &Shape, area: egui::Rect, out: &mut Vec<String>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, area, out)),
            Shape::Text(text)
                if area
                    .expand(1.0)
                    .contains_rect(egui::Rect::from_min_size(text.pos, text.galley.size())) =>
            {
                out.push(text.galley.job.text.clone())
            }
            _ => {}
        }
    }
    let area = yolu_app::windows::window_rect(&h.ctx, window)
        .unwrap_or_else(|| panic!("{window} を描いていない"));
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        walk(&shape.shape, area, &mut out);
    }
    out
}

fn in_window(h: &Harness<'_, YoluApp>, window: &str, label: &str) -> egui::Pos2 {
    let area = yolu_app::windows::window_rect(&h.ctx, window)
        .unwrap_or_else(|| panic!("{window} を描いていない"));
    rect_of(h, label, |r| area.contains_rect(r)).center()
}

/// ウィンドウを開いた画面（Unity 版が作った .ylp を開いて、メニューから）。
fn window_app(dir: &TempDir) -> (Harness<'static, YoluApp>, PathBuf) {
    let path = dir.path("Work.ylp");
    unity_style(&path);
    let mut h = app(1280.0, 800.0, 32);
    apply(&mut h, Action::OpenProject(path.clone()));
    (h, path)
}

#[test]
fn the_window_lists_the_things_per_kind_with_names_and_toggles_them() {
    let dir = TempDir::new("window");
    let (mut h, _path) = window_app(&dir);
    // メニューから始める
    let at = menu_title(&h, "ファイル").center();
    click(&mut h, at);
    let at = popup_item(&h, "配布用に保存…").center();
    click(&mut h, at);
    settle(&mut h);
    let texts = window_texts(&h, "distribute");
    for want in [
        "配布用に保存",
        // アセットの種類の区分と、その中のアセット（使っていない物は「未使用」の印）
        "画像",
        "Scratch",
        "Rust",
        "未使用",
        // アセット以外の種類（1 行ずつ）
        "PSD の原本",
        "素材の出どころのパス",
        "モデルの参照",
        "メッシュマップ",
        "古いサムネイルなど",
        "知らないエントリ",
        "やめる",
        "保存…",
    ] {
        assert!(texts.iter().any(|t| t == want), "{want}: {texts:?}");
    }
    // 0 件の種類は出ない（Unity の値は無い）。説明の文・開発用の数は出さない（大きさは KB・MB で短く）
    assert!(
        !texts.iter().any(|t| t == "Unity のマテリアルの値"),
        "{texts:?}"
    );
    for t in &texts {
        assert_plain("配布用に保存のウィンドウ", t);
        assert!(
            !t.contains("MiB") && !t.chars().all(|c| c.is_ascii_digit()),
            "{t}"
        );
    }
    crate::common::shot::window_shot(&mut h, "distribute", "distribute_window");
    // 切り替え: チェックを入れると写しに含める種類になる（既定は、どの種類も含めない）
    let all = Removal::ALL.to_vec();
    assert_eq!(h.distribute().window().unwrap().removed(), all);
    let at = in_window(&h, "distribute", "PSD の原本");
    click(&mut h, at);
    assert!(h
        .distribute()
        .window()
        .unwrap()
        .includes(Removal::PsdOriginals));
    let at = in_window(&h, "distribute", "PSD の原本");
    click(&mut h, at);
    assert_eq!(h.distribute().window().unwrap().removed(), all);
    // 「保存…」は保存先を選ぶウィンドウを頼む（ウィンドウは開いたまま）
    let at = in_window(&h, "distribute", "保存…");
    click(&mut h, at);
    assert_eq!(*h.dialog_request(), Some(DialogRequest::DistributeSave));
    assert!(h.distribute().window_visible());
    *h.dialog_request_mut() = None;
    // 英語: ウィンドウの文字に日本語が残らない
    h.set_lang(Lang::En);
    h.run();
    let texts = window_texts(&h, "distribute");
    for want in [
        "Save for Distribution",
        "Images",
        "Unused",
        "Original PSDs",
        "Asset source paths",
        "Model reference",
        "Mesh maps",
        "Old thumbnail and state",
        "Unknown entries",
        "Cancel",
        "Save…",
    ] {
        assert!(texts.iter().any(|t| t == want), "{want}: {texts:?}");
    }
    for t in &texts {
        assert_plain("distribute window", t);
    }
    // 名前（ファイル名・素材の名前）は言語に依らない。日本語が出るのは名前に日本語がある物だけ
    assert!(
        texts.iter().filter(|t| has_japanese(t)).count() == 0,
        "{texts:?}"
    );
    crate::common::shot::window_shot(&mut h, "distribute", "distribute_window_english");
    // やめる: ウィンドウを閉じて準備した写しを捨てる
    let at = in_window(&h, "distribute", "Cancel");
    click(&mut h, at);
    assert!(!h.distribute().is_open());
}

#[test]
fn the_replace_window_names_the_file_and_cancel_returns_to_the_list() {
    let dir = TempDir::new("confirm");
    let (mut h, _path) = window_app(&dir);
    apply(&mut h, Action::Distribute(DistributeAction::Start));
    settle(&mut h);
    let existing = dir.path("Existing.ylp");
    std::fs::write(&existing, b"x").unwrap();
    apply(&mut h, Action::Distribute(DistributeAction::Save(existing)));
    let texts = window_texts(&h, "distribute-replace");
    for want in ["置き換えるファイル", "Existing.ylp", "やめる", "置き換える"] {
        assert!(texts.iter().any(|t| t == want), "{want}: {texts:?}");
    }
    // ウィンドウの画像は撮らない（行の保存先の絶対パスが、実行ごとの一時フォルダで揺れる）
    h.set_lang(Lang::En);
    h.run();
    let texts = window_texts(&h, "distribute-replace");
    for want in ["File to Replace", "Cancel", "Replace"] {
        assert!(texts.iter().any(|t| t == want), "{want}: {texts:?}");
    }
    let at = in_window(&h, "distribute-replace", "Cancel");
    click(&mut h, at);
    assert!(h.distribute().replace.is_none());
    // ウィンドウに戻る。やめれば準備した写しを捨てる
    assert!(h.distribute().window_visible());
    apply(&mut h, Action::Distribute(DistributeAction::CancelWindow));
    assert!(!h.distribute().is_open());
}

#[test]
fn headless_the_remembered_selections_are_a_kind_and_the_copy_without_them_is_format_7() {
    use yolu_app::selection::saved::SavedOp;
    use yolu_app::selection::{SelAction, SelEdit};
    use yolu_core::SelectionCombine;
    let dir = TempDir::new("remembered");
    let (mut s, _path) = opened(&dir);
    // 残した選択範囲を持つ（開いている文書の変更として）
    s.act(Action::Sel(SelAction::Edit(SelEdit::Rect {
        x0: 2,
        y0: 2,
        x1: 20,
        y1: 20,
        mode: SelectionCombine::Replace,
    })));
    s.act(Action::Sel(SelAction::Saved(SavedOp::Save("髪".into()))));
    assert_eq!(s.saved_selections().len(), 1);
    start(&mut s);
    assert!(
        kinds(&s).contains(&Removal::SavedSelections),
        "{:?}",
        kinds(&s)
    );
    assert!(
        !s.distribute()
            .window()
            .unwrap()
            .includes(Removal::SavedSelections),
        "既定は含めない"
    );
    // 既定（除く）: 写しは形式 7 で、残した選択範囲のエントリが無い。今の選択範囲は絵の一部として残る
    let dest = dir.path("Dist.ylp");
    s.act(Action::Distribute(DistributeAction::Save(dest.clone())));
    s.wait_distribute();
    let copy = Project::read(&std::fs::read(&dest).unwrap()).unwrap();
    assert_eq!(copy.info().format, 7, "Unity 版が開ける形式");
    let id = copy.sets()[0].id.clone();
    assert!(copy.saved_selections(&id).unwrap().items.is_empty());
    assert!(copy.sets()[0].selection.is_some());
    // 切り替えを外すと残り、形式は 8
    start(&mut s);
    s.act(Action::Distribute(DistributeAction::Toggle(
        Removal::SavedSelections,
    )));
    let kept = dir.path("Kept.ylp");
    s.act(Action::Distribute(DistributeAction::Save(kept.clone())));
    s.wait_distribute();
    let copy = Project::read(&std::fs::read(&kept).unwrap()).unwrap();
    assert_eq!(copy.info().format, 8);
    assert_eq!(copy.saved_selections(&id).unwrap().items.len(), 1);
    // 開いているプロジェクトの残した選択範囲は、どちらでも変わらない
    assert_eq!(s.saved_selections().len(), 1);
    // ウィンドウの種類の名前は日英で出る
    for lang in Lang::ALL {
        assert!(!yolu_app::distribute::window::label(lang, Removal::SavedSelections).is_empty());
        assert!(!yolu_app::distribute::window::tooltip(lang, Removal::SavedSelections).is_empty());
    }
}

// ───────── アセットを 1 つずつ含める・外す ─────────

const ASSET_SET: &str = "0f1e2d3c-4b5a-4978-8796-a5b4c3d2e1f0";
/// アセットの名前（1・2 は塗りつぶしレイヤーが読む＝使っている。3・4 はどこからも使っていない）。
const ASSET_NAMES: [&str; 4] = ["Skin", "Cloth", "Hair", "Mask"];

fn image_id(n: u32) -> ImageId {
    ImageId(u128::from_str_radix(&rid(n).replace('-', ""), 16).unwrap())
}

/// アセットの画像が 4 つ（`ASSET_NAMES`）の .ylp をディスクに作る。塗りつぶしレイヤーが 1 と 2 を（色と高さで）読む。
fn four_assets(path: &Path) {
    let mut doc = Document::new(64, 64).unwrap();
    let mut inputs = EffectInputs::new();
    for n in [1, 2] {
        inputs = inputs.with_image(
            image_id(n),
            ImageInput::new(2, 2, pixels(n), ImageColorSpace::Srgb).unwrap(),
        );
    }
    doc.set_effect_inputs(inputs).unwrap();
    let layer = doc
        .add_fill_layer(
            "塗り",
            &[
                (Channel::Color, Rgba8::new(10, 20, 30, 255)),
                (Channel::Height, Rgba8::new(255, 255, 255, 255)),
            ],
            None,
        )
        .unwrap();
    doc.set_fill_image(layer, Channel::Color, Some(image_id(1)))
        .unwrap();
    doc.set_fill_image(layer, Channel::Height, Some(image_id(2)))
        .unwrap();
    let base = Project::create(
        writer(),
        &[SetSpec {
            id: ASSET_SET.into(),
            name: "Body".into(),
            materials: vec![MaterialRef::Material {
                name: "Body".into(),
                asset: None,
            }],
            document: Some(NativeDocument::from_core(&doc).unwrap().into()),
            composites: composite_pngs(&doc).unwrap(),
        }],
        ASSET_SET,
    )
    .unwrap();
    let mut shelf = Shelf::new(1 << 30);
    for (i, name) in ASSET_NAMES.iter().enumerate() {
        let n = i as u32 + 1;
        shelf
            .add_image(
                &rid(n),
                name,
                &pixels(n),
                2,
                2,
                "srgb",
                serde_json::json!({"type":"none"}),
            )
            .unwrap();
    }
    let project = base.with_shelf(&shelf, writer()).unwrap();
    std::fs::write(path, project.to_bytes().unwrap()).unwrap();
}

fn ids_of(path: &Path) -> Vec<String> {
    Project::read(&std::fs::read(path).unwrap())
        .unwrap()
        .resources()
        .iter()
        .map(|r| r.id.clone())
        .collect()
}

fn open_assets(dir: &TempDir) -> (AppState, PathBuf) {
    let path = dir.path("Assets.ylp");
    four_assets(&path);
    let mut s = AppState::new(32, 32);
    s.bake_mut().backend = BakeBackend::Cpu;
    s.act(Action::OpenProject(path.clone()));
    assert!(
        s.project().as_ref().is_some_and(|p| p.is_file()),
        "{}",
        s.message()
    );
    assert_eq!(s.shelf().resources().len(), 4, "{}", s.message());
    (s, path)
}

fn write_to(s: &mut AppState, dest: &Path) {
    *s.dialog_request_mut() = None;
    s.act(Action::Distribute(DistributeAction::Save(
        dest.to_path_buf(),
    )));
    s.wait_distribute();
    assert!(
        s.message().starts_with("配布用に保存しました"),
        "{}",
        s.message()
    );
}

#[test]
fn headless_the_assets_start_with_the_used_ones_and_each_can_be_included_or_left_out() {
    let dir = TempDir::new("assets");
    let (mut s, path) = open_assets(&dir);
    let before = std::fs::read(&path).unwrap();
    start(&mut s);
    let window = s.distribute().window().unwrap();
    // 既定: 使っている 2 つを含め、使っていない 2 つを外す
    let shelf = &window.inventory().shelf;
    assert_eq!(
        shelf
            .iter()
            .map(|i| (i.name.as_str(), i.used))
            .collect::<Vec<_>>(),
        [
            ("Skin", true),
            ("Cloth", true),
            ("Hair", false),
            ("Mask", false)
        ]
    );
    assert!(shelf.iter().all(|i| i.bytes > 0));
    for n in 1..=4 {
        assert_eq!(window.includes_resource(&rid(n)), n <= 2, "{n}");
    }
    assert_eq!(
        window.excluded_resources(),
        [rid(3), rid(4)].into_iter().collect()
    );
    // 1 行ずつの切り替え
    s.act(Action::Distribute(DistributeAction::ToggleResource(rid(3))));
    assert!(s.distribute().window().unwrap().includes_resource(&rid(3)));
    s.act(Action::Distribute(DistributeAction::ToggleResource(rid(3))));
    assert!(!s.distribute().window().unwrap().includes_resource(&rid(3)));
    // プロジェクトに無い ID は無視する
    s.act(Action::Distribute(DistributeAction::ToggleResource(rid(
        99,
    ))));
    assert!(!s.distribute().window().unwrap().includes_resource(&rid(99)));
    // 区分の一括（種類は画像）: 全部含める・全部外す
    let kind = s.distribute().window().unwrap().inventory().shelf[0].kind;
    s.act(Action::Distribute(DistributeAction::SetKind(kind, true)));
    assert!((1..=4).all(|n| s.distribute().window().unwrap().includes_resource(&rid(n))));
    s.act(Action::Distribute(DistributeAction::SetKind(kind, false)));
    assert!((1..=4).all(|n| !s.distribute().window().unwrap().includes_resource(&rid(n))));
    // 既定のまま 1 つ（使っていない物）を足して書く: 使っている 2 つとその 1 つだけが写しのアセットに入る
    s.act(Action::Distribute(DistributeAction::CancelWindow));
    start(&mut s);
    s.act(Action::Distribute(DistributeAction::ToggleResource(rid(3))));
    let one = dir.path("One.ylp");
    write_to(&mut s, &one);
    assert_eq!(ids_of(&one), [rid(1), rid(2), rid(3)]);
    // 既定のまま書いた写しは、使っている 2 つだけ（今までの既定と同じ結果）
    start(&mut s);
    let usual = dir.path("Usual.ylp");
    write_to(&mut s, &usual);
    assert_eq!(ids_of(&usual), [rid(1), rid(2)]);
    // 外した物の中身は、写しのどこにも無い
    let copy = read_entries(&usual);
    let original = read_entries(&path);
    let index = Project::read(&std::fs::read(&path).unwrap()).unwrap();
    for r in index
        .resources()
        .iter()
        .filter(|r| [rid(3), rid(4)].contains(&r.id))
    {
        assert!(original.contains_key(&r.entry) && !copy.contains_key(&r.entry));
    }
    // 開いているファイルと、開いているプロジェクトのアセットは変わらない
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(s.shelf().resources().len(), 4);
}

#[test]
fn headless_a_used_asset_left_out_still_writes_a_copy_that_opens_with_the_asset_missing() {
    let dir = TempDir::new("in-use");
    let (mut s, _path) = open_assets(&dir);
    start(&mut s);
    // 使っているアセット（1）を外し、使っていない 1 つ（3）を含める
    s.act(Action::Distribute(DistributeAction::ToggleResource(rid(1))));
    s.act(Action::Distribute(DistributeAction::ToggleResource(rid(3))));
    let dest = dir.path("Gap.ylp");
    write_to(&mut s, &dest);
    assert_eq!(ids_of(&dest), [rid(2), rid(3)]);
    // 開き直せる: 知らせは失敗でなく、アセットはその 2 つ、レイヤーは残る
    let mut t = AppState::new(32, 32);
    t.bake_mut().backend = BakeBackend::Cpu;
    t.act(Action::OpenProject(dest.clone()));
    assert!(
        t.project().as_ref().is_some_and(|p| p.is_file()),
        "{}",
        t.message()
    );
    assert_eq!(
        t.shelf()
            .resources()
            .iter()
            .map(|r| r.id.clone())
            .collect::<Vec<_>>(),
        [rid(2), rid(3)]
    );
    // 画像が無いと効果の入力がそろわないので、そのセットは読むだけで開く（保存した合成を見せる。断らず、落ちない）。
    // 編集できるセットのままにはならない（描いて保存すると、欠けた画像を指したまま残るため）
    let reason = t
        .sets()
        .current()
        .read_only
        .clone()
        .expect("読むだけで開く");
    assert!(reason.contains("プロジェクトに画像が無い"), "{reason}");
    assert!(t.message().starts_with("開きました"), "{}", t.message());
    assert_eq!(t.doc().layers().len(), 1);
    // 見せる画像は、書いた時点の合成の PNG（画像のあった見た目）
    let shown = t
        .doc()
        .composite_channel(Channel::Color, t.doc().bounds())
        .unwrap();
    let before = s
        .doc()
        .composite_channel(Channel::Color, s.doc().bounds())
        .unwrap();
    assert_eq!(shown[..4], before[..4]);
}

fn assets_window(dir: &TempDir) -> Harness<'static, YoluApp> {
    let path = dir.path("Assets.ylp");
    four_assets(&path);
    let mut h = app(1280.0, 800.0, 32);
    apply(&mut h, Action::OpenProject(path));
    apply(&mut h, Action::Distribute(DistributeAction::Start));
    settle(&mut h);
    h
}

#[test]
fn the_asset_rows_toggle_one_by_one_the_kind_row_toggles_all_and_folds() {
    let dir = TempDir::new("rows");
    let mut h = assets_window(&dir);
    let texts = window_texts(&h, "distribute");
    for want in ["画像", "Skin", "Cloth", "Hair", "Mask", "未使用"] {
        assert!(texts.iter().any(|t| t == want), "{want}: {texts:?}");
    }
    assert_eq!(
        texts.iter().filter(|t| *t == "未使用").count(),
        2,
        "使っていない 2 つだけに印"
    );
    // 1 行ずつ: 名前の行を押すと含める・外すが替わる
    let included = |h: &Harness<'_, YoluApp>, n: u32| {
        h.distribute().window().unwrap().includes_resource(&rid(n))
    };
    assert!(included(&h, 1) && included(&h, 2) && !included(&h, 3));
    let at = in_window(&h, "distribute", "Hair");
    click(&mut h, at);
    assert!(included(&h, 3) && !included(&h, 4));
    let at = in_window(&h, "distribute", "Skin");
    click(&mut h, at);
    assert!(
        !included(&h, 1) && included(&h, 2),
        "使っているアセットも外せる"
    );
    crate::common::shot::window_shot(&mut h, "distribute", "distribute_window_assets");
    // 区分の行: 一部だけのときは押すと全部含める。全部のときは押すと全部外す
    let at = in_window(&h, "distribute", "画像");
    click(&mut h, at);
    assert!((1..=4).all(|n| included(&h, n)));
    click(&mut h, at);
    assert!((1..=4).all(|n| !included(&h, n)));
    // たたむと、アセットの行が消える（含める・外すは変わらない）。もう一度で戻る
    let at = in_window(&h, "distribute", "閉じる");
    click(&mut h, at);
    let texts = window_texts(&h, "distribute");
    assert!(texts.iter().any(|t| t == "画像"), "{texts:?}");
    assert!(
        !texts.iter().any(|t| t == "Skin" || t == "Mask"),
        "{texts:?}"
    );
    let at = in_window(&h, "distribute", "開く");
    click(&mut h, at);
    let texts = window_texts(&h, "distribute");
    assert!(texts.iter().any(|t| t == "Skin"), "{texts:?}");
    // 英語
    h.set_lang(Lang::En);
    h.run();
    let texts = window_texts(&h, "distribute");
    for want in ["Images", "Skin", "Unused"] {
        assert!(texts.iter().any(|t| t == want), "{want}: {texts:?}");
    }
    for t in &texts {
        assert_plain("distribute window", t);
    }
    crate::common::shot::window_shot(&mut h, "distribute", "distribute_window_assets_english");
}
