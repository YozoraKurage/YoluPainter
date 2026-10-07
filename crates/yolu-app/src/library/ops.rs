//! 個人のライブラリの操作（`AppState` の側）: プロジェクトで使う・ライブラリへ入れる・ファイルを足す・消す・置く。
//!
//! 時間のかかる読み書き（ファイルの読み込み・検証・書き込み）は別のスレッドで走り、やめられる。結果は毎フレーム `library_poll` が受け取る。
//! 断られたら何も変えず、理由を短くステータスバーへ出す。ライブラリのフォルダの外は読まない・書かない・消さない
//! （`yolu_io::library` が相対パスとリンクを検査する）。
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;

use yolu_core::smart::SmartKind;
use yolu_io::library as files;
use yolu_io::shelf::ResourceKind;
use yolu_io::smart::SmartFile;

use super::image as png;
use super::{PendingWrite, REFUSAL_PNG, REFUSAL_PNG_SIZE, REFUSAL_UNSUPPORTED};
use crate::lang::Lang;
use crate::notice::{Kind as NoticeKind, Source};
use crate::shelf::{
    image_as_material, is_builtin, smart_material_from, stage_library_file, stage_library_image,
    Attempt, Block, ItemKind, PlaceTarget, Running,
};
use crate::state::{AppState, DialogRequest};

/// 書き込みのスレッドの結果。
pub(crate) enum WriteDone {
    /// 棚の素材 1 つをライブラリへ入れた結果。
    Put {
        name: String,
        result: Result<files::Added, yolu_io::Error>,
    },
    /// 外のファイルをまとめて足した結果（入れたファイルの相対パス・すでにあったファイルの相対パス・断ったファイルの名前と理由の文）。
    Many {
        added: Vec<String>,
        existing: Vec<String>,
        refused: Vec<String>,
    },
}

fn png_error(p: png::Problem) -> yolu_io::Error {
    match p {
        png::Problem::Unreadable => yolu_io::Error::InvalidData(REFUSAL_PNG.into()),
        png::Problem::Size => yolu_io::Error::Budget(REFUSAL_PNG_SIZE.into()),
    }
}

/// ライブラリの外のファイルを、上限を超えずに読む（開いたファイルで長さを確かめ、区切りごとに取り消しの旗を見る）。
fn read_outside(path: &Path, limit: u64, cancel: &AtomicBool) -> Result<Vec<u8>, yolu_io::Error> {
    let mut file = std::fs::File::open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(yolu_io::Error::InvalidData(files::REFUSAL_NOT_FILE.into()));
    }
    if meta.len() > limit {
        return Err(yolu_io::Error::Budget(files::REFUSAL_TOO_LARGE.into()));
    }
    let mut out = Vec::with_capacity(meta.len() as usize);
    let mut buf = vec![0u8; (1 << 20).min(meta.len() as usize).max(1)];
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(yolu_io::Error::Core(yolu_core::CoreError::Cancelled));
        }
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(out);
        }
        if out.len() as u64 + n as u64 > limit {
            return Err(yolu_io::Error::Budget(files::REFUSAL_TOO_LARGE.into()));
        }
        out.extend_from_slice(&buf[..n]);
    }
}

fn stem_of(rel: &str) -> String {
    let leaf = rel.rsplit('/').next().unwrap_or(rel);
    leaf.rsplit_once('.')
        .map_or(leaf, |(stem, _)| stem)
        .to_owned()
}

/// 読み込みの結果の件数（入れた分とすでにあった分。0 の分は出さない）。
fn count_note(lang: Lang, added: usize, existing: usize) -> String {
    let mut parts = Vec::new();
    if added > 0 {
        parts.push(lang.pick(
            format!("{added} 件をライブラリに入れました。"),
            format!("Added {added} to the library."),
        ));
    }
    if existing > 0 {
        parts.push(lang.pick(
            format!("{existing} 件はすでにライブラリにありました。"),
            format!(
                "{existing} {} already in the library.",
                if existing == 1 { "was" } else { "were" }
            ),
        ));
    }
    parts.join(lang.pick("", " "))
}

fn no_location(lang: Lang) -> &'static str {
    lang.pick("ライブラリの場所がありません", "No library location")
}

fn kind_of_extension(path: &Path) -> Option<files::Kind> {
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(files::Kind::of_extension)
}

fn resource_kind(kind: ItemKind) -> Option<(files::Kind, ResourceKind)> {
    Some(match kind {
        ItemKind::Image => return None,
        ItemKind::Brush => (files::Kind::Brush, ResourceKind::Brush),
        ItemKind::Material => (files::Kind::Material, ResourceKind::Material),
        ItemKind::SmartMaterial => (files::Kind::Smart, ResourceKind::SmartMaterial),
        ItemKind::SmartMask => (files::Kind::Smart, ResourceKind::SmartMask),
    })
}

impl AppState {
    /// いまのライブラリのフォルダ（設定の「棚の場所」。設定のフォルダも分からなければ None）。
    pub fn library_root(&self) -> Option<PathBuf> {
        self.prefs.settings.library_folder()
    }

    /// ライブラリへ書き込める状態か（書き込みが走っていれば理由を出して false）。
    pub(crate) fn library_idle(&mut self) -> bool {
        match self.library.busy_reason(self.lang) {
            Some(reason) => {
                self.cannot(
                    NoticeKind::Refusal,
                    Source::Library,
                    Attempt::LibraryAdd,
                    None,
                    reason,
                );
                false
            }
            None => true,
        }
    }

    /// 別のスレッドでライブラリへ書く仕事を始める。`job` は取り消しの旗と言語をもらって結果を返す。
    fn library_spawn_write(
        &mut self,
        name: String,
        job: impl FnOnce(&AtomicBool, Lang) -> WriteDone + Send + 'static,
    ) {
        let lang = self.lang;
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let running = Running::start(&self.library.running);
        let repaint = self.shelf.context.clone();
        let hold = self.library.hold.clone();
        std::thread::spawn(move || {
            while hold.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let done = job(&flag, lang);
            drop(running);
            let _ = tx.send(done);
            if let Some(ctx) = repaint {
                ctx.request_repaint();
            }
        });
        self.info(
            Source::Library,
            format!(
                "{}: {name}",
                lang.pick("ライブラリへ書き込み中", "Writing to the library")
            ),
        );
        self.library.write = Some(PendingWrite { name, rx, cancel });
    }

    /// 別のスレッドの書き込みが終わるまで待って結果を入れる（試験用）。
    pub fn library_wait(&mut self) {
        while self.library.write.is_some() {
            self.library_poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// 毎フレーム: 別のスレッドの書き込みの結果と、読み終えた一覧・見た項目を受け取る。
    pub fn library_poll(&mut self) {
        self.library.poll();
        let lang = self.lang;
        let Some(write) = &self.library.write else {
            return;
        };
        let done = match write.rx.try_recv() {
            Ok(done) => Some(Some(done)),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(None),
        };
        let Some(done) = done else { return };
        // 止まった書き込みの名前（まとめて入れるときは件数）は、断りの文に添えない
        self.library.write.take().expect("上で確かめた");
        self.library.refresh();
        match done {
            Some(WriteDone::Put { name, result }) => match result {
                Ok(added) => {
                    self.info(
                        Source::Library,
                        format!(
                            "{}: {}",
                            if added.existed {
                                lang.pick("すでにライブラリにあります", "Already in the library")
                            } else {
                                lang.pick("ライブラリに入れました", "Added to the library")
                            },
                            added.rel
                        ),
                    );
                }
                Err(e) => {
                    let why = crate::lang::library_io_error(lang, &e);
                    self.cannot(
                        NoticeKind::Error,
                        Source::Library,
                        Attempt::LibraryAdd,
                        Some(&name),
                        &why,
                    );
                }
            },
            Some(WriteDone::Many {
                added,
                existing,
                refused,
            }) => {
                let (new, old) = (added.len(), existing.len());
                if refused.is_empty() {
                    self.info(
                        Source::Library,
                        match (added.as_slice(), existing.as_slice()) {
                            ([name], []) => format!(
                                "{}: {name}",
                                lang.pick("ライブラリに入れました", "Added to the library")
                            ),
                            ([], [name]) => format!(
                                "{}: {name}",
                                lang.pick("すでにライブラリにあります", "Already in the library")
                            ),
                            _ => count_note(lang, new, old),
                        },
                    );
                } else {
                    let mut text = refused.join(lang.pick("", " "));
                    if new + old > 0 {
                        // 一部は入った: 気をつけること。全部断ったなら失敗
                        text += lang.pick("", " ");
                        text += &count_note(lang, new, old);
                        self.warn(Source::Library, text);
                    } else {
                        self.fail(Source::Library, text);
                    }
                }
            }
            None => {
                let why = lang.pick("書き込みが途中で止まりました", "The write stopped midway");
                self.cannot(
                    NoticeKind::Error,
                    Source::Library,
                    Attempt::LibraryAdd,
                    None,
                    why,
                );
            }
        }
    }

    // ───────── ライブラリへ入れる ─────────

    /// 棚の素材（プロジェクトの棚の ID）を、種類のフォルダへ入れる。同じバイト列（画像は、画素が同じ画像）が既にあれば書かない。
    pub(crate) fn library_put(&mut self, id: &str) {
        let lang = self.lang;
        let Some(res) = self.shelf.get(id) else {
            return;
        };
        if is_builtin(id) {
            return self.refuse(
                Source::Library,
                lang.pick(
                    "組み込みはライブラリへ入れられません。",
                    "Built-in items cannot be added to the library.",
                ),
            );
        }
        let Some(kind) = ItemKind::of(&res.kind) else {
            return;
        };
        let name = res.name.clone();
        let content = res.content.clone();
        let origin_file = (res.metadata["origin"]["type"] == "library")
            .then(|| res.metadata["origin"]["file"].as_str().map(str::to_owned))
            .flatten();
        let Some(root) = self.library_root() else {
            return self.cannot(
                NoticeKind::Refusal,
                Source::Library,
                Attempt::LibraryAdd,
                None,
                no_location(lang),
            );
        };
        if !self.library_idle() {
            return;
        }
        let Some(bytes) = self.shelf.shelf().content_arc(id) else {
            return;
        };
        let file_kind = match kind {
            ItemKind::Image => files::Kind::Image,
            _ => resource_kind(kind).expect("画像以外").0,
        };
        // 画素が同じ画像が既にあれば（PNG の書き方が違っても）書かない
        if kind == ItemKind::Image {
            let same = self
                .library
                .find_image_content(&content)
                .map(str::to_owned)
                .or_else(|| {
                    origin_file.filter(|rel| {
                        self.library
                            .info(rel)
                            .is_some_and(|i| !i.content.is_empty() && i.content == content)
                    })
                });
            if let Some(rel) = same {
                self.info(
                    Source::Library,
                    format!(
                        "{}: {rel}",
                        lang.pick("すでにライブラリにあります", "Already in the library")
                    ),
                );
                return;
            }
        }
        let shown = name.clone();
        self.library_spawn_write(shown, move |cancel, _| WriteDone::Put {
            result: files::add(
                &root,
                file_kind.folder(),
                &name,
                file_kind,
                &bytes,
                Some(cancel),
            ),
            name,
        });
    }

    /// 外のファイル（PNG・.ylsmart）を検証して、ライブラリへ足す（まとめて 1 つの知らせ。断ったファイルは名前と理由を残す）。
    pub(crate) fn library_add_files(&mut self, paths: &[PathBuf]) {
        let lang = self.lang;
        if paths.is_empty() {
            return;
        }
        let Some(root) = self.library_root() else {
            return self.cannot(
                NoticeKind::Refusal,
                Source::Library,
                Attempt::LibraryAdd,
                None,
                no_location(lang),
            );
        };
        if !self.library_idle() {
            return;
        }
        let limit = self.library.limits.read;
        let paths = paths.to_vec();
        let name = match paths.as_slice() {
            [one] => one
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            many => match lang {
                Lang::Ja => format!("{} 件", many.len()),
                Lang::En => format!("{} files", many.len()),
            },
        };
        self.library_spawn_write(name, move |cancel, lang| {
            let (mut added, mut existing, mut refused) = (Vec::new(), Vec::new(), Vec::new());
            for path in &paths {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
                let shown = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                match add_outside(&root, path, limit, cancel) {
                    Ok(done) if done.existed => existing.push(done.rel),
                    Ok(done) => added.push(done.rel),
                    Err(e) => refused.push(lang.with_reason(
                        Attempt::LibraryAdd.what(lang, Some(&shown)),
                        crate::lang::library_io_error(lang, &e),
                    )),
                }
            }
            WriteDone::Many {
                added,
                existing,
                refused,
            }
        });
    }

    // ───────── プロジェクトで使う ─────────

    /// ライブラリのファイルをこのプロジェクトで使う＝写しを .ylp の棚へ入れる（出どころは `library`）。別のスレッドで読んで検証し、
    /// できたら棚へ入る。同じ中身が棚に既にあれば、そのまま（棚は変わらない）。
    pub(crate) fn library_use(&mut self, rel: &str) {
        let lang = self.lang;
        if !self.shelf_writable() {
            return;
        }
        if let Some(reason) = self.shelf.busy_reason(lang) {
            return self.cannot(
                NoticeKind::Refusal,
                Source::Library,
                Attempt::LibraryUse,
                None,
                reason,
            );
        }
        let Some(root) = self.library_root() else {
            return self.cannot(
                NoticeKind::Refusal,
                Source::Library,
                Attempt::LibraryUse,
                None,
                no_location(lang),
            );
        };
        let Some(kind) = Path::new(rel)
            .extension()
            .and_then(|e| e.to_str())
            .and_then(files::Kind::of_extension)
        else {
            return;
        };
        let item_kind = self
            .library
            .entry(rel)
            .map_or(ItemKind::SmartMaterial, |e| self.library.kind_of(e));
        let item_kind = match kind {
            files::Kind::Image => ItemKind::Image,
            files::Kind::Brush => ItemKind::Brush,
            files::Kind::Material => ItemKind::Material,
            files::Kind::Smart => item_kind,
        };
        let limit = self.library.limits.read;
        let rel = rel.to_owned();
        let name = stem_of(&rel);
        let display = name.clone();
        self.shelf
            .spawn_import(name.clone(), item_kind, move |cancel, snapshot| {
                let bytes = files::read(&root, &rel, limit, Some(cancel))?;
                let sha = files::sha256_hex(&bytes);
                match kind {
                    files::Kind::Image => {
                        let img = png::decode(&bytes).map_err(png_error)?;
                        let rounded = png::rounds_to_8_bit(&bytes);
                        let (w, h) = img.dimensions();
                        // 文書と同じ向き（下の行が先）で渡す
                        let mut rgba = img.into_raw();
                        png::flip_rows(&mut rgba, w as usize * 4);
                        stage_library_image(
                            snapshot,
                            &display,
                            &rgba,
                            w,
                            h,
                            &rel,
                            &sha,
                            bytes.len() as u64,
                        )
                        .map(|staged| staged.rounded(rounded))
                    }
                    files::Kind::Smart => {
                        let file = SmartFile::read(&bytes)?;
                        let resource = if file.kind() == SmartKind::Mask {
                            ResourceKind::SmartMask
                        } else {
                            ResourceKind::SmartMaterial
                        };
                        let inner = file.info()["name"].as_str().unwrap_or(&display).to_owned();
                        stage_library_file(snapshot, &inner, resource, &bytes, &rel, &sha)
                    }
                    files::Kind::Brush => stage_library_file(
                        snapshot,
                        &display,
                        ResourceKind::Brush,
                        &bytes,
                        &rel,
                        &sha,
                    ),
                    files::Kind::Material => stage_library_file(
                        snapshot,
                        &display,
                        ResourceKind::Material,
                        &bytes,
                        &rel,
                        &sha,
                    ),
                }
            });
        self.info(
            Source::Library,
            format!("{}: {name}", self.shelf.saving_label(lang)),
        );
    }

    /// ライブラリのファイルを、棚へ入れずに、読んで文書へ置く（スマート素材と画像。層の組・マスクの入れ替えは 1 回の Undo）。
    pub(crate) fn library_place(&mut self, rel: &str, target: PlaceTarget) {
        let lang = self.lang;
        let Some(root) = self.library_root() else {
            return self.cannot(
                NoticeKind::Refusal,
                Source::Library,
                Attempt::Place,
                None,
                no_location(lang),
            );
        };
        let Some(kind) = kind_of_extension(Path::new(rel)) else {
            return;
        };
        let name = stem_of(rel);
        if matches!(kind, files::Kind::Brush | files::Kind::Material) {
            let block = Block::Kind(if kind == files::Kind::Brush {
                ItemKind::Brush
            } else {
                ItemKind::Material
            });
            return self.refuse(Source::Library, lang.with_reason(block.reason(lang), ""));
        }
        let limit = self.library.limits.read;
        let bytes = match files::read(&root, rel, limit, None) {
            Ok(b) => b,
            Err(e) => {
                return self.cannot(
                    NoticeKind::Error,
                    Source::Library,
                    Attempt::Place,
                    Some(&name),
                    &crate::lang::library_io_error(lang, &e),
                )
            }
        };
        let made = match kind {
            files::Kind::Image => png::dimensions(&bytes)
                .map_err(|p| crate::lang::library_io_error(lang, &png_error(p)))
                .and_then(|(w, h)| image_as_material(lang, &bytes, w, h, &name)),
            _ => smart_material_from(lang, &bytes),
        };
        match made {
            Ok(material) => {
                let steps = self.doc.undo_count();
                self.place_material(&name, material, target, false);
                // 16 ビットの PNG は 8 ビットへ丸めて置く。置けたときだけ知らせる
                if kind == files::Kind::Image
                    && png::rounds_to_8_bit(&bytes)
                    && self.doc.undo_count() > steps
                {
                    // 置いた知らせに、8 ビットへ丸めた但し書きを添える（気をつけること）
                    self.amend(
                        NoticeKind::Warning,
                        Source::Library,
                        " · ",
                        super::rounded_note(lang),
                    );
                }
            }
            Err(why) => self.cannot(
                NoticeKind::Error,
                Source::Library,
                Attempt::Place,
                Some(&name),
                &why,
            ),
        }
    }

    // ───────── 消す ─────────

    /// ライブラリのファイルを消してよいか確かめる窓を頼む（窓は `YoluApp` が出す）。
    pub(crate) fn library_ask_remove(&mut self, rel: &str) {
        if self.library.entry(rel).is_some() {
            self.library.pending_remove = Some(rel.to_owned());
            self.dialog_request = Some(DialogRequest::LibraryRemove);
        }
    }

    /// ライブラリのファイル 1 つを消す（ファイルだけ。プロジェクトの棚の写しと、置いた層は変わらない）。
    pub(crate) fn library_remove(&mut self, rel: &str) {
        let lang = self.lang;
        if self.library.pending_remove.as_deref() == Some(rel) {
            self.library.pending_remove = None;
        }
        let Some(root) = self.library_root() else {
            return self.cannot(
                NoticeKind::Refusal,
                Source::Library,
                Attempt::LibraryRemove,
                None,
                no_location(lang),
            );
        };
        let name = stem_of(rel);
        match files::remove(&root, rel) {
            Ok(()) => {
                self.library.refresh();
                if self.library.selected.as_deref() == Some(&super::library_id(rel)) {
                    self.library.selected = None;
                }
                self.info(
                    Source::Library,
                    format!(
                        "{}: {name}",
                        lang.pick("ライブラリから消しました", "Removed from the library")
                    ),
                );
            }
            Err(e) => self.cannot(
                NoticeKind::Error,
                Source::Library,
                Attempt::LibraryRemove,
                Some(&name),
                &crate::lang::library_io_error(lang, &e),
            ),
        }
    }
}

/// 外のファイル 1 つを検証して、ライブラリへ足す（PNG は寸法、.ylsmart は読めるかを確かめる）。
fn add_outside(
    root: &Path,
    path: &Path,
    limit: u64,
    cancel: &AtomicBool,
) -> Result<files::Added, yolu_io::Error> {
    let kind = kind_of_extension(path)
        .filter(|k| matches!(k, files::Kind::Image | files::Kind::Smart))
        .ok_or_else(|| yolu_io::Error::InvalidData(REFUSAL_UNSUPPORTED.into()))?;
    let bytes = read_outside(path, limit, cancel)?;
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = match kind {
        files::Kind::Image => {
            // 全部を展開して確かめる（ヘッダーだけ読める壊れた PNG をライブラリへ入れない）
            png::decode(&bytes).map_err(png_error)?;
            stem
        }
        _ => {
            let file = SmartFile::read(&bytes)?;
            file.info()["name"].as_str().unwrap_or(&stem).to_owned()
        }
    };
    files::add(root, kind.folder(), &name, kind, &bytes, Some(cancel))
}

/// ライブラリのフォルダを OS のファイルの窓で開く（無ければ作る）。フォルダでないものは開かない。
pub fn open_folder(path: &Path) -> std::io::Result<()> {
    if !path.is_absolute() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not an absolute path",
        ));
    }
    std::fs::create_dir_all(path)?;
    if !path.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a folder",
        ));
    }
    open_it(path)
}

#[cfg(windows)]
fn open_it(path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{w, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: 文字列は NUL 終端の UTF-16 で、この呼び出しの間生きている。
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // 32 より大きければ成功（ShellExecute の決まり）。
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(windows))]
fn open_it(path: &Path) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let mut child = Command::new(opener)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    // 終わりを受けておく（ゾンビを残さない）。
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
