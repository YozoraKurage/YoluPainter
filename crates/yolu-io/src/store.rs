use crate::atomic::{
    is_busy, is_leftover_name, pending_name, retry_busy, retry_busy_within, BUSY_BUDGET,
};
use crate::{
    check, is_hash,
    package::{file_digest, release_at, Limits, Package},
    Error, Project, Result,
};

fn conflict(ok: bool, reason: impl Into<String>) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error::SaveConflict(reason.into()))
    }
}
use std::{
    cmp::Ordering as Cmp,
    fs::{self, File, OpenOptions, TryLockError},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
/// 保存先の名前の終わり（大文字小文字は問わない）。
const EXTENSION: &str = ".ylp";
/// 退避の保持数の上限（設定が受ける上限。保存そのものはこれを超える数も受ける）。
pub const MAX_BACKUPS_TO_KEEP: u32 = 1000;
/// 上書きで置き換えた前の版（退避）をいくつ残すか。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BackupKeep {
    /// すべて残す（消さない）。
    #[default]
    All,
    /// 新しい順にこの数だけ残す。今回の退避も数に入り、必ず残る。0 は退避しない（すでにある退避は消さない）。
    Count(u32),
}
/// 保存の段（`SaveTarget::save_with_progress` が各段の始めに知らせる。進み具合の表示のため）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SaveStage {
    /// 全エントリの長さと SHA-256 を数える（manifest を作る）。
    Counting,
    /// 一時ファイルへ流して書く。
    Writing,
    /// 書いた一時ファイルを読み直して確かめる。
    Verifying,
    /// 前の版を退避して、保存先へ 1 回の置換で確定する。
    Replacing,
}
impl SaveStage {
    /// 段の数。
    pub const COUNT: usize = 4;
    /// 0 から数えた段の番号（`COUNT` 未満）。
    pub fn index(self) -> usize {
        self as usize
    }
}
/// 保存の結果。置換で確定した後の整理が一部失敗しても保存は成功のままで、理由を `prune_failures` に載せる。
#[derive(Debug)]
pub struct SaveReport {
    /// 保存した版の印。
    pub stamp: FileStamp,
    /// 保存したプロジェクト（中身は書いたファイルから読む。次の保存・書き置きは、変わらないエントリをこのファイルから写す）。
    pub project: Option<Project>,
    /// 今回退避した前の版（新規の保存と、退避しない設定では作らない）。
    pub backup: Option<PathBuf>,
    /// 保持数を超えた古い退避のうち消せなかったもの（退避のフォルダーを読めなかったときは、そのフォルダー）。
    pub prune_failures: Vec<PruneFailure>,
}
/// 消せなかった退避と理由。
#[derive(Debug)]
pub struct PruneFailure {
    pub path: PathBuf,
    pub error: io::Error,
}
/// 退避のフォルダーの名前の終わり（元のファイルの名前に続く）。
const BACKUP_FOLDER_SUFFIX: &str = "-backups~";
/// 退避のフォルダー（保存先と同じフォルダーの `<ファイル名>-backups~`）。
pub fn backup_folder(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    path.with_file_name(format!("{name}{BACKUP_FOLDER_SUFFIX}"))
}
/// 退避のフォルダーの中の退避（`backups` が挙げる形の名前のファイル）なら、退避した元のファイル（退避のフォルダーと同じ場所の
/// `<ファイル名>`。元のファイルが今あるかは見ない）。退避でなければ None: 利用者が名前を変えて置いたファイル・フォルダーの名前が
/// 合わないもの・別のファイルの退避の名前のもの。退避は元のファイルの前の版なので、元のファイルからの相対の参照（view.json のモデルの
/// 場所など）は元のファイルの場所から解く。
pub fn backup_origin(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    let folder = path.parent()?;
    let owner = folder
        .file_name()?
        .to_str()?
        .strip_suffix(BACKUP_FOLDER_SUFFIX)?;
    // 元のファイルの名前は保存先の決まり（.ylp で終わる。大文字小文字は問わない）に合う
    if owner.len() <= EXTENSION.len()
        || !owner.as_bytes()[owner.len() - EXTENSION.len()..]
            .eq_ignore_ascii_case(EXTENSION.as_bytes())
    {
        return None;
    }
    let stem = Path::new(owner).file_stem()?.to_str()?;
    let legacy = name.strip_suffix(EXTENSION).is_some_and(is_hash);
    (parse_stamped(stem, name).is_some() || legacy).then(|| folder.with_file_name(owner))
}
/// 保存が退避した版（新しい順）。名前が `<名前>-<UTC の時刻>.ylp` の形のものと、以前の版が SHA-256 名で残したもの
/// （時刻の名前の後ろに、互いは更新時刻の順）だけで、利用者が名前を変えて残したファイルは含めない（整理の対象にもしない）。
pub fn backups(path: &Path) -> io::Result<Vec<PathBuf>> {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or_default();
    Ok(list_backups(&backup_folder(path), &stem)?
        .into_iter()
        .map(|b| b.path)
        .collect())
}
/// 開いた・保存した時点のファイルの印。外からの書き換えは内容（`sha256`・`length`）で見分け、
/// 更新時刻だけが変わった（touch・同期ツール）ものは書き換えとして扱わない（`same_content`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub sha256: String,
    pub length: u64,
    pub modified: SystemTime,
}
impl FileStamp {
    /// 内容が同じか（更新時刻は見ない）。保存の「外で変更されていないか」の判定はこちら。
    fn same_content(&self, other: &Self) -> bool {
        self.sha256 == other.sha256 && self.length == other.length
    }
}
/// 開いた時点の印を持つ保存先。前の版を退避し、検証済み一時ファイルから一度だけ置換する。
/// 同じ保存先への保存は OS のロックで 1 つずつ（残ったロックのファイルは邪魔にならない）。置換の後の失敗は失敗として
/// 返すが、新しい版は確定していて `stamp()` も新しい版になる。
///
/// 複製できる（持つのは保存先の名前と開いた・保存した時の印だけ）: 別のスレッドで保存するとき、元は呼び手に残したまま複製を渡し、
/// 保存した後の印は戻ってきた複製で置き換える。保存のスレッドが結果を返さずに止まったときは、元の印が古いまま残るので、
/// 次の保存は外からの書き換えとして断る（印を取り違えて上書きしない）。
#[derive(Clone, Debug)]
pub struct SaveTarget {
    path: PathBuf,
    expected: Option<FileStamp>,
}
impl SaveTarget {
    /// 既定の予算の上限（`Limits::default`）で開く。
    pub fn open(path: impl AsRef<Path>) -> Result<(Project, Self)> {
        Self::open_within(path, &Limits::default())
    }
    /// 上限（設定の予算から）を渡して開く。ファイルは流して読み（全エントリを確かめる）、正本の画素はメモリに読まない。
    pub fn open_within(path: impl AsRef<Path>, limits: &Limits) -> Result<(Project, Self)> {
        let path = path.as_ref().to_path_buf();
        // 先にプロジェクトとして読んで確かめ（壊れた・大きすぎるファイルは、全体のハッシュを数える前に断る）、それから印を数える。
        // 読む間に書き換えられていないことは、長さと更新時刻で見る
        let before = fs::symlink_metadata(&path)?;
        let p = Project::open(&path, limits)?;
        let stamp = read_stamp(&path)?;
        conflict(
            before.len() == stamp.length && before.modified()? == stamp.modified,
            "読み込み中にファイルが変更されました",
        )?;
        Ok((
            p,
            Self {
                path,
                expected: Some(stamp),
            },
        ))
    }
    /// 新規の保存先。名前が `.ylp` で終わっていなければ断る（フォルダーは保存のときに、なければ作る）。
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        check_name(&path)?;
        conflict(
            absent(&path)?,
            "保存先が既にあります。上書きにはopenで印を取得してください",
        )?;
        Ok(Self {
            path,
            expected: None,
        })
    }
    pub fn stamp(&self) -> Option<&FileStamp> {
        self.expected.as_ref()
    }
    /// すべての退避を残して保存する（`save_with(project, BackupKeep::All)` の印だけ）。
    pub fn save(&mut self, project: &Project) -> Result<FileStamp> {
        self.save_with(project, BackupKeep::All).map(|r| r.stamp)
    }
    /// 退避の保持数を決めて保存する。整理は置換で確定した後だけ、保持数を超えた古い退避を消し、消せなくても保存は成功のまま
    /// （`SaveReport::prune_failures`）。
    pub fn save_with(&mut self, project: &Project, keep: BackupKeep) -> Result<SaveReport> {
        self.save_with_progress(project, keep, &mut |_| {})
    }
    /// `save_with` の、段の始まりを知らせる形（別のスレッドで保存して、進み具合を出すため）。`progress` は保存を動かしているスレッドで、
    /// 各段の始めに 1 度ずつ呼ばれる（段の順は [`SaveStage`]。失敗した保存は、そこまでの段だけ）。
    pub fn save_with_progress(
        &mut self,
        project: &Project,
        keep: BackupKeep,
        progress: &mut dyn FnMut(SaveStage),
    ) -> Result<SaveReport> {
        progress(SaveStage::Counting);
        self.save_core(
            project,
            keep,
            &mut Seams {
                try_lock: &mut File::try_lock,
                move_new: &mut move_without_replacing,
                remove: &mut remove_file,
                phase: &mut |name| {
                    match name {
                        "memory-verified" => progress(SaveStage::Writing),
                        "flushed" => progress(SaveStage::Verifying),
                        "disk-verified" => progress(SaveStage::Replacing),
                        _ => {}
                    }
                    Ok(())
                },
                rename: &mut rename_file,
                busy: &is_busy,
                sleep: &mut std::thread::sleep,
            },
        )
    }
    fn save_core(
        &mut self,
        project: &Project,
        keep: BackupKeep,
        seams: &mut Seams<'_>,
    ) -> Result<SaveReport> {
        check_name(&self.path)?;
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let name = self
            .path
            .file_name()
            .ok_or_else(|| Error::InvalidData("保存先がファイルではありません".into()))?
            .to_string_lossy();
        let stem = self
            .path
            .file_stem()
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();
        // 中身はディスクに触る前に数えて確かめる（1 回目: 全エントリの長さと SHA-256、作る正本は読み手で読み直す。不正な中身のために
        // フォルダーを作ったり、ロックを持ったりしない）
        let plan = project.original.plan()?;
        (seams.phase)("memory-verified")?;
        // 新規の保存先のフォルダーは、なければ作る。このあとの失敗で、作ったフォルダーは空なら消す（ロックより後に手放す順）
        let mut created = match &self.expected {
            None => CreatedDirs::make(parent)?,
            Some(_) => {
                conflict(
                    parent.exists(),
                    "保存先が外部で消されています。上書きしません",
                )?;
                CreatedDirs::default()
            }
        };
        let lock = SaveLock::acquire_with(
            parent.join(format!(".{name}.save.lock~")),
            &mut *seams.try_lock,
            |_| {},
        )?;
        self.check_expected()?;
        // 排他ロックを持てたなら、この保存先へ保存している者は他にいない: 前の保存が強制終了などで残した一時ファイルは誰のものでも
        // ないので、自分の一時ファイルを作る前に片付ける（退避の置き場の分は、退避を作るときに）
        if lock.exclusive {
            sweep_leftovers(parent, &name);
        }
        let mut pending = Pending::create(parent.join(pending_name(&name)))?;
        // 2 回目: 流して書く（ファイル全体をメモリに組まない）
        let f = pending.file.take().expect("作ったばかり");
        let mut out = io::BufWriter::with_capacity(1 << 20, f);
        project.original.write_with(&plan, &mut out)?;
        let f = out.into_inner().map_err(|e| e.into_error())?;
        f.sync_all()?;
        drop(f);
        (seams.phase)("flushed")?;
        // 書いたものを読み直して確かめる（全エントリの長さ・CRC・SHA-256、manifest が書こうとしたものと同じこと、プロジェクトとして
        // 読めること）。ファイル全体の印と、エントリごとの確かめは互いに独立なので、並べて読む（断る理由の順は今までと同じ: 印が先）
        let keep_in_memory = crate::package::Thresholds::current().keep_in_memory;
        let (opened, stamped) = rayon::join(
            || Package::open_keeping(&pending.path, &Limits::unbounded(), keep_in_memory),
            || read_stamp(&pending.path),
        );
        let new_stamp = stamped?;
        let written = opened?;
        conflict(
            written.read_manifest() == Some(&plan.manifest[..]),
            "一時ファイルの内容が変化しました",
        )?;
        let saved = project.rehomed(written)?;
        (seams.phase)("disk-verified")?;
        // 前の版（退避する版）。置き場が塞がれていたら、保存先に触る前に断る。保存先の周りの事情なので、プロジェクトのデータの不正
        // （InvalidData）ではなく SaveConflict で、画面が言い分けられるようにする。退避しない設定では置き場に触らない
        let mut to_back_up = None;
        if let Some(expected) = &self.expected {
            if keep != BackupKeep::Count(0) {
                let folder = parent.join(format!("{name}{BACKUP_FOLDER_SUFFIX}"));
                check_backup_folder(&folder)?;
                // 退避の置き場に残った、強制終了された保存の一時ファイル。置き場に触れるのは、退避を作る保存だけ
                if lock.exclusive {
                    sweep_backup_leftovers(&folder, &name);
                }
                to_back_up = Some((folder, expected.clone()));
            }
        }
        (seams.phase)("before-replace")?;
        // 置き換える直前に、保存先が開いた・保存した時の中身のままかを確かめる。退避するなら、退避へ写しながら数えて確かめる（大きな
        // ファイルを 2 度読まない）。退避は置換の直前に作る（置換に失敗したら、作った退避は消す。途中で止まった保存が退避を溜めない）
        let backup = match to_back_up {
            Some((folder, previous)) => {
                present(&self.path)?;
                Some(Backup::make(
                    &folder, &name, &stem, &self.path, &previous, seams,
                )?)
            }
            None => {
                self.check_expected()?;
                None
            }
        };
        (seams.phase)("backed-up")?;
        // 一時ファイルの隠し属性（Windows）を、移す直前に外す（付けたまま移すと、保存した .ylp が隠しファイルになる）
        show(&pending.path)?;
        match &self.expected {
            // 上書きが目的なので、置き換える移動。削除してから移動する代替手順は使わない。置換の失敗時は元を残す。Windows では、同期・
            // ウイルス対策・Unity の取り込みが保存先や一時ファイルを一時的に掴んでいると失敗するので、短くやり直す（`replace_file`）
            Some(_) => replace_file(&pending.path, &self.path, seams)?,
            // 新規の保存先は、置き換えない移動。最後の確かめから移動までの間に外から作られたものを上書きしない
            None => match retry_busy(seams.busy, &mut *seams.sleep, || {
                (seams.move_new)(&pending.path, &self.path)
            })? {
                Moved::Done => {}
                Moved::Occupied => return Err(occupied()),
                // 置き換えない移動の使えないファイルシステム。確かめ直してから置き換える移動に落とす（その間に作られる競合は
                // 防げない。ロックの使えない場所で排他なしに続けるのと同じ考え方）
                Moved::Unsupported => {
                    conflict(absent(&self.path)?, occupied_reason())?;
                    retry_busy(seams.busy, &mut *seams.sleep, || {
                        (seams.rename)(&pending.path, &self.path)
                    })?;
                }
            },
        }
        // 一時ファイルの名前は `pending` の後始末が最後に消す（移動で無くなっていれば何もしない。リンクで移して元の名前を
        // 消せなかったときは、保存先と同じ実体を指す隠しファイルをここで消し直す）
        created.keep();
        let backup = backup.map(Backup::keep);
        self.expected = Some(new_stamp.clone());
        // 置換の後に失敗しても新しい版は確定している（印も新しい）。呼び出し側へは失敗として返し、
        // 次の保存は新しい印のまま衝突せずに通る。ロックと一時ファイルの後始末はここを抜けても働く
        (seams.phase)("after-replace")?;
        // 整理は確定した後だけ。今回の退避は数に入れて必ず残し、消せなくても保存は成功のまま理由を返す
        let prune_failures = match (keep, &backup) {
            (BackupKeep::Count(n), Some(made)) if n > 0 => {
                prune(&stem, n, made, &mut *seams.remove)
            }
            _ => Vec::new(),
        };
        Ok(SaveReport {
            stamp: new_stamp,
            project: Some(saved.moved_to(&self.path)),
            backup,
            prune_failures,
        })
    }
    fn check_expected(&self) -> Result<()> {
        match &self.expected {
            Some(want) => {
                let actual = read_target(&self.path)?;
                conflict(
                    actual.same_content(want),
                    "保存先が外部で変更されています。上書きしません",
                )
            }
            None => conflict(absent(&self.path)?, occupied_reason()),
        }
    }
}
/// 保存の外との接点。本物は `save_with` が組む。試験が差し込む口は、ロックの使えないファイルシステム・置き換えない移動の使えない
/// ファイルシステムと外から先に作られる競合・消せない退避・各段での失敗。
struct Seams<'a> {
    try_lock: &'a mut dyn FnMut(&File) -> std::result::Result<(), TryLockError>,
    /// 置き換えない移動（新規の保存先と、退避の最終の名前への移動）。
    move_new: &'a mut dyn FnMut(&Path, &Path) -> io::Result<Moved>,
    remove: &'a mut dyn FnMut(&Path) -> io::Result<()>,
    phase: &'a mut dyn FnMut(&str) -> Result<()>,
    /// 置き換える移動（既にある保存先の上書き）。試験が、共有違反などの失敗を差し込む。
    rename: &'a mut dyn FnMut(&Path, &Path) -> io::Result<()>,
    /// 置き換えのやり直しに値する失敗か（本物は Windows の共有違反・アクセス拒否だけ）。
    busy: &'a dyn Fn(&io::Error) -> bool,
    /// やり直しの間の待ち（試験は待たずに記録する）。
    sleep: &'a mut dyn FnMut(Duration),
}
fn occupied_reason() -> &'static str {
    "新規保存先が外部で作られました。上書きしません"
}
fn occupied() -> Error {
    Error::SaveConflict(occupied_reason().into())
}
/// 保存先の名前が `.ylp` で終わる（大文字小文字は問わず、名前の部分が空でない）ことを確かめる。
fn check_name(path: &Path) -> Result<()> {
    let ok = path.file_name().is_some_and(|n| {
        let n = n.to_string_lossy();
        let n = n.as_bytes();
        n.len() > EXTENSION.len()
            && n[n.len() - EXTENSION.len()..].eq_ignore_ascii_case(EXTENSION.as_bytes())
    });
    conflict(ok, "保存先の名前が .ylp で終わっていません")
}
fn remove_file(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        // 先に消されていた（同じ目的の別の整理など）なら、消せたのと同じ
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}
/// 置き換える移動（保存先が既にあれば置き換える）。
fn rename_file(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}
/// 保存の一時ファイル・ロックを作る開き方に、隠し属性（Windows）を付ける。Windows は先頭の '.' では隠れない。付けたまま `rename` すると
/// 保存した .ylp が隠しファイルになるので、移す前に [`show`] で外す。Unix は先頭の '.' で足りる。
fn hide(options: &mut OpenOptions) -> &mut OpenOptions {
    #[cfg(windows)]
    {
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        std::os::windows::fs::OpenOptionsExt::attributes(options, FILE_ATTRIBUTE_HIDDEN);
    }
    options
}
/// [`hide`] で作った一時ファイルの隠し属性を外す（普通の新しいファイルと同じ、アーカイブの印だけにする。Windows のみ）。
#[cfg(windows)]
fn show(path: &Path) -> Result<()> {
    #[link(name = "kernel32")]
    extern "system" {
        fn SetFileAttributesW(file: *const u16, attributes: u32) -> i32;
    }
    const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x20;
    let wide = wide(path)?;
    // SAFETY: NUL で終わる UTF-16 で、呼び出しの間生きている
    if unsafe { SetFileAttributesW(wide.as_ptr(), FILE_ATTRIBUTE_ARCHIVE) } == 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(())
}
#[cfg(not(windows))]
fn show(_: &Path) -> Result<()> {
    Ok(())
}
/// 置換が共有違反のとき、自分のハンドルを手放す前に、手放さずに待つ合計（[`BUSY_BUDGET`] の内側）。NTFS では、外のツール（ウイルス対策・
/// 同期のツール・Unity の取り込み）が掴んでいるのが普通の原因で、自分のハンドルは置換を妨げない。たいていはこの間に手放されるので、
/// 手放さずに通れば、保存前のプロジェクトの写しは読めたまま残る。
const RELEASE_AFTER: Duration = Duration::from_millis(300);
/// 一時ファイルを保存先へ置き換えて移す。共有違反・アクセス拒否のあいだは短くやり直す（[`retry_busy`]）。[`RELEASE_AFTER`] たっても通ら
/// ないときだけ、このプロセスが開いている保存先のハンドル（開いた .ylp の位置読み）を手放して、残りの待ちでやり直す: 置き換えの規則が
/// POSIX でないファイルシステム（FAT・exFAT・一部のネットワーク）は、開いているファイルを置き換えられないので、自分のハンドルが原因の
/// ことがある（NTFS と Unix は開いたままでも通る）。外のツールが長く掴んでいるだけのときも、手放してから通れば同じ結果になる（原因を
/// 見分けられない）。置き換えられなければ掴み直す。置き換えられたら手放したまま（古い版の写しは、読もうとすると断る）。
fn replace_file(from: &Path, to: &Path, seams: &mut Seams<'_>) -> io::Result<()> {
    let busy = seams.busy;
    let rename = &mut *seams.rename;
    match retry_busy_within(RELEASE_AFTER, busy, &mut *seams.sleep, || rename(from, to)) {
        Err(e) if busy(&e) => {
            let released = release_at(to);
            let result =
                retry_busy_within(BUSY_BUDGET - RELEASE_AFTER, busy, &mut *seams.sleep, || {
                    rename(from, to)
                });
            if result.is_err() {
                released.reacquire();
            }
            result
        }
        other => other,
    }
}
/// `folder` の中の、この保存先の一時ファイルの残り（強制終了された保存が残したもの）を消す。誰も保存していないと確かめた（保存先の排他
/// ロックを持っている）ときだけ呼ぶ。普通のファイルだけで、リンク・フォルダーには触らない。消せなくても無視する（次の保存でやり直す）。
fn sweep_leftovers(folder: &Path, name: &str) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let found = entry.file_name();
        let is_file = entry.file_type().is_ok_and(|t| t.is_file());
        if is_file && found.to_str().is_some_and(|f| is_leftover_name(name, f)) {
            let _ = fs::remove_file(entry.path());
        }
    }
}
/// 退避の置き場の中の、この保存先の一時ファイルの残りを消す（[`sweep_leftovers`]）。置き場が本物のフォルダーのときだけ: リンクの先や
/// 普通のファイルには触らない（`check_backup_folder` で断る形。確かめてから開くまでの間に差し替えられた場合も、ここで見直す）。
fn sweep_backup_leftovers(folder: &Path, name: &str) {
    if fs::symlink_metadata(folder).is_ok_and(|m| m.is_dir()) {
        sweep_leftovers(folder, name);
    }
}
/// 新規の保存で作ったフォルダー（浅い方から）。保存が確定しなかったときは、空のまま残っているものだけを深い方から消す
/// （中に何かが入っていれば消さない）。もとからあったフォルダーには触らない。
#[derive(Default)]
struct CreatedDirs(Vec<PathBuf>);
impl CreatedDirs {
    /// 無いフォルダーを 1 段ずつ作る。作る途中で失敗したら、作った分は消える。
    fn make(dir: &Path) -> Result<Self> {
        let mut missing = Vec::new();
        let mut at = dir;
        while !at.as_os_str().is_empty() {
            match fs::symlink_metadata(at) {
                Ok(_) => break,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    missing.push(at.to_path_buf());
                    match at.parent() {
                        Some(up) => at = up,
                        None => break,
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }
        let mut created = Self::default();
        for d in missing.into_iter().rev() {
            match fs::create_dir(&d) {
                Ok(()) => created.0.push(d),
                // 間に別の保存が作った。それは自分が作ったものではない
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(created)
    }
    fn keep(&mut self) {
        self.0.clear();
    }
}
impl Drop for CreatedDirs {
    fn drop(&mut self) {
        for d in self.0.drain(..).rev() {
            let _ = fs::remove_dir(d);
        }
    }
}
/// 前の版の置き場が、使えない形（普通のファイル・シンボリックリンク）で塞がれていないか。
fn check_backup_folder(folder: &Path) -> Result<()> {
    match fs::symlink_metadata(folder) {
        Ok(m) if m.file_type().is_symlink() => {
            conflict(false, "バックアップ先がシンボリックリンクです")
        }
        Ok(m) => conflict(m.is_dir(), "バックアップ先がフォルダーではありません"),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
/// 作った退避。`keep` で確定するまでは、落ちた保存の分として（作ったフォルダーごと）消える。
///
/// 写しは一時の名前（`.{名前}.{pid}-{通し番号}.pending~`）へ書いて同期し、検証してから最終の名前へ移す（保存の一時ファイルと同じ形）。
/// 保存が強制終了されて（`Drop` が走らない）切れた写しが残っても、一時の名前なので退避として数えられず（整理は名前の形だけで数える）、
/// 次の保存の前に片付く（`sweep_backup_leftovers`）。退避は、保存先とは独立したバイト列の写し。
struct Backup {
    temp: Option<PathBuf>,
    path: Option<PathBuf>,
    created_folder: Option<PathBuf>,
}
impl Backup {
    /// 前の版（`source` の今の中身。`expected` と同じ中身であること）を、一時のファイルへ流して写し、新しい名前へ移す（同じ名前が先に
    /// あれば、別の名前でやり直す）。写しながら数えた中身が `expected` と違えば、外部の変更として断る（写しは消す）。
    fn make(
        folder: &Path,
        name: &str,
        stem: &str,
        source: &Path,
        expected: &FileStamp,
        seams: &mut Seams<'_>,
    ) -> Result<Self> {
        let created = match fs::create_dir(folder) {
            Ok(()) => true,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => false,
            Err(e) => return Err(e.into()),
        };
        let mut backup = Self {
            temp: None,
            path: None,
            created_folder: created.then(|| folder.to_path_buf()),
        };
        // 作る間に置き場がリンクやファイルに替えられていないか
        check_backup_folder(folder)?;
        let temp = folder.join(pending_name(name));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let f = hide(&mut options).open(&temp)?;
        backup.temp = Some(temp.clone());
        let mut from = io::BufReader::with_capacity(1 << 20, File::open(source)?);
        let mut to = io::BufWriter::with_capacity(1 << 20, f);
        let mut sha = <sha2::Sha256 as sha2::Digest>::new();
        let mut buf = vec![0u8; 1 << 20];
        let mut length = 0u64;
        loop {
            let n = match from.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            };
            sha2::Digest::update(&mut sha, &buf[..n]);
            to.write_all(&buf[..n])?;
            length += n as u64;
        }
        let f = to.into_inner().map_err(|e| e.into_error())?;
        f.sync_all()?;
        drop(f);
        conflict(
            length == expected.length
                && format!("{:x}", sha2::Digest::finalize(sha)) == expected.sha256,
            "保存先が外部で変更されています",
        )?;
        (seams.phase)("backup-copied")?;
        show(&temp)?;
        for _ in 0..BACKUP_NAME_ATTEMPTS {
            let path = folder.join(backup_name(folder, stem, SystemTime::now())?);
            // 置き換えない移動: 同じ名前が先にあれば（別のプロセスが同じ時刻に作った）断られるので、別の名前でやり直す
            let moved = retry_busy(seams.busy, &mut *seams.sleep, || {
                (seams.move_new)(&temp, &path)
            })?;
            let done = match moved {
                Moved::Done => true,
                Moved::Occupied => false,
                // 置き換えない移動の使えないファイルシステム。確かめ直してから移す
                Moved::Unsupported if absent(&path)? => {
                    retry_busy(seams.busy, &mut *seams.sleep, || {
                        (seams.rename)(&temp, &path)
                    })?;
                    true
                }
                Moved::Unsupported => false,
            };
            if done {
                backup.path = Some(path);
                return Ok(backup);
            }
        }
        Err(Error::SaveConflict("退避の名前を決められません".into()))
    }
    fn keep(mut self) -> PathBuf {
        self.created_folder = None;
        self.path.take().expect("作ってある")
    }
}
impl Drop for Backup {
    fn drop(&mut self) {
        // 一時の名前は、移した後なら無い（リンクで移して元の名前を消せなかったときは、ここで消し直す）
        if let Some(temp) = self.temp.take() {
            let _ = fs::remove_file(temp);
        }
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
        if let Some(folder) = self.created_folder.take() {
            let _ = fs::remove_dir(folder);
        }
    }
}
/// 退避の名前を決めるやり直しの回数（同じ名前に当たるのは、別のプロセスが同じ時刻に作ったときだけ）。
const BACKUP_NAME_ATTEMPTS: usize = 16;
/// 時刻の名前の時刻の部分の長さ（`yyyyMMddTHHmmssfffZ`）。
const STAMP_LEN: usize = 19;
/// 退避のフォルダーの中にある、この文書の退避。
struct Found {
    path: PathBuf,
    /// 時刻の名前なら（時刻、下線の数）。以前の版の SHA-256 名なら None。
    stamp: Option<(String, usize)>,
    modified: SystemTime,
}
/// `<名前>-<時刻>[_…].ylp` の（時刻、下線の数）。時刻は固定の幅の数字なので、文字列の順が時刻の順。
fn parse_stamped(stem: &str, name: &str) -> Option<(String, usize)> {
    let middle = name
        .strip_prefix(stem)?
        .strip_prefix('-')?
        .strip_suffix(EXTENSION)?;
    let (stamp, rest) = (middle.get(..STAMP_LEN)?, middle.get(STAMP_LEN..)?);
    let b = stamp.as_bytes();
    let ok = b[..8].iter().all(u8::is_ascii_digit)
        && b[8] == b'T'
        && b[9..18].iter().all(u8::is_ascii_digit)
        && b[18] == b'Z'
        && rest.bytes().all(|c| c == b'_');
    ok.then(|| (stamp.to_owned(), rest.len()))
}
/// この文書の退避を新しい順に。フォルダーが無ければ空。普通のファイルだけ（フォルダー・リンクは含めない）。
fn list_backups(folder: &Path, stem: &str) -> io::Result<Vec<Found>> {
    let entries = match fs::read_dir(folder) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry?;
        let is_file = match entry.file_type() {
            Ok(kind) => kind.is_file(),
            // 読む間に別のプロセス（Unity 版の整理など）が消した
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        let name = entry.file_name();
        let Some(name) = name.to_str().filter(|_| is_file) else {
            continue;
        };
        let stamp = parse_stamped(stem, name);
        let legacy = name.strip_suffix(EXTENSION).is_some_and(is_hash);
        if stamp.is_none() && !legacy {
            continue;
        }
        // 更新時刻は、時刻の名前を持たない以前の退避の順にだけ使う
        let modified = match stamp {
            Some(_) => UNIX_EPOCH,
            None => entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH),
        };
        found.push(Found {
            path: entry.path(),
            stamp,
            modified,
        });
    }
    found.sort_by(|a, b| {
        let by_name = || b.path.file_name().cmp(&a.path.file_name());
        match (&a.stamp, &b.stamp) {
            (Some(x), Some(y)) => y.cmp(x).then_with(by_name),
            (Some(_), None) => Cmp::Less,
            (None, Some(_)) => Cmp::Greater,
            (None, None) => b.modified.cmp(&a.modified).then_with(by_name),
        }
    });
    Ok(found)
}
/// 新しい退避の名前 `<名前>-<UTC の時刻（ミリ秒）>.ylp`。すでにある退避と並べたとき必ず最後（最新）になるようにする:
/// 同じ時刻の版があればいちばん多い下線より 1 つ多い下線を付け、時計が戻っていてもいちばん新しい版の時刻に下線を足す
/// （整理で古い版を消した後に同じ名前を使い直したり、新しい版が古い版より前に並んだりしない）。
fn backup_name(folder: &Path, stem: &str, now: SystemTime) -> io::Result<String> {
    let now = stamp_of(now);
    let latest = list_backups(folder, stem)?
        .into_iter()
        .filter_map(|b| b.stamp)
        .max();
    let (stamp, underscores) = match latest {
        Some((stamp, n)) if stamp >= now => (stamp, n + 1),
        _ => (now, 0),
    };
    Ok(format!(
        "{stem}-{stamp}{}{EXTENSION}",
        "_".repeat(underscores)
    ))
}
/// `yyyyMMddTHHmmssfffZ`（UTC）。
fn stamp_of(time: SystemTime) -> String {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let (days, secs) = ((since.as_secs() / 86_400) as i64, since.as_secs() % 86_400);
    // 暦（グレゴリオ暦の、1970-01-01 からの日数から年月日へ）
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}{:03}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60,
        since.subsec_millis()
    )
}
/// 保持数を超えた古い退避を消す（新しい順に、今回の退避を除いて `keep - 1` 件を残す）。消せなかったものと理由を返す。
fn prune(
    stem: &str,
    keep: u32,
    just_made: &Path,
    remove: &mut dyn FnMut(&Path) -> io::Result<()>,
) -> Vec<PruneFailure> {
    let folder = just_made.parent().unwrap_or(Path::new("."));
    let found = match list_backups(folder, stem) {
        Ok(found) => found,
        Err(error) => {
            return vec![PruneFailure {
                path: folder.to_path_buf(),
                error,
            }]
        }
    };
    found
        .into_iter()
        .filter(|b| b.path != just_made)
        .skip(keep.saturating_sub(1) as usize)
        .filter_map(|b| {
            remove(&b.path).err().map(|error| PruneFailure {
                path: b.path,
                error,
            })
        })
        .collect()
}
/// 置き換えない移動の結果。
#[derive(Debug, PartialEq, Eq)]
enum Moved {
    Done,
    /// 移動先に何かがある（何も変えていない）。
    Occupied,
    /// このファイルシステムでは置き換えない移動ができない（何も変えていない）。Windows の `MoveFileExW` は
    /// どのファイルシステムでも使えるので、Windows では本物は返さない（試験が差し込む）。
    #[cfg_attr(windows, allow(dead_code))]
    Unsupported,
}
/// `from` を `to` へ、すでに `to` があれば置き換えずに移す（あれば `Occupied`）。確かめてから移すのではなく、移す操作そのものが
/// 断る。Linux（glibc）は `renameat2` の RENAME_NOREPLACE、それが使えない・ほかの Unix は `hard_link` してから `from` を消す
/// （どちらも、ある名前を作らずに断る）。Windows は `MoveFileExW` を置き換えのフラグなしで呼ぶ。どれも使えない
/// ファイルシステムは `Unsupported`（呼ぶ側が確かめ直して移す）。
#[cfg(unix)]
fn move_without_replacing(from: &Path, to: &Path) -> io::Result<Moved> {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    match rename_noreplace(from, to) {
        Ok(()) => return Ok(Moved::Done),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(Moved::Occupied),
        // フラグを知らないカーネル・ファイルシステム（ENOSYS・EINVAL・EOPNOTSUPP）。リンクに落とす
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::Unsupported | io::ErrorKind::InvalidInput
            ) => {}
        Err(e) => return Err(e),
    }
    link_then_unlink(from, to)
}
/// `hard_link` は移動先が先にあれば断る（作る操作そのものが断る）。元（か移動先のフォルダー）が無いのは本当の失敗なので `Err`（`Unsupported` にすると、
/// 置き換えない移動の失敗として見えず、呼ぶ側が確かめ直してから置き換える移動へ進んで、同じ失敗がそちらで返る）。そのほかの、リンクを作れない場所は
/// `Unsupported`（本当の失敗は、呼ぶ側の置き換える移動が同じ形で返す）。リンクを作れたら元の名前を消す。消せなくても、確定した保存は戻さず `Done` を返す
/// （元の名前は保存先と同じ実体を指す。呼ぶ側の `Pending` が最後に消し直す）。
#[cfg(unix)]
fn link_then_unlink(from: &Path, to: &Path) -> io::Result<Moved> {
    link_then_unlink_with(from, to, &mut remove_file)
}
#[cfg(unix)]
fn link_then_unlink_with(
    from: &Path,
    to: &Path,
    remove: &mut dyn FnMut(&Path) -> io::Result<()>,
) -> io::Result<Moved> {
    match fs::hard_link(from, to) {
        Ok(()) => {
            let _ = remove(from);
            Ok(Moved::Done)
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(Moved::Occupied),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(e),
        Err(_) => Ok(Moved::Unsupported),
    }
}
#[cfg(all(unix, target_os = "linux", target_env = "gnu"))]
fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    extern "C" {
        fn renameat2(
            old_dir: i32,
            old: *const std::ffi::c_char,
            new_dir: i32,
            new: *const std::ffi::c_char,
            flags: u32,
        ) -> i32;
    }
    const AT_FDCWD: i32 = -100;
    const RENAME_NOREPLACE: u32 = 1;
    let cstr = |p: &Path| {
        CString::new(p.as_os_str().as_bytes())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
    };
    let (from, to) = (cstr(from)?, cstr(to)?);
    // SAFETY: どちらも NUL で終わる C 文字列で、呼び出しの間生きている。カレントのフォルダーからの相対（AT_FDCWD）
    let rc = unsafe {
        renameat2(
            AT_FDCWD,
            from.as_ptr(),
            AT_FDCWD,
            to.as_ptr(),
            RENAME_NOREPLACE,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
#[cfg(windows)]
fn move_without_replacing(from: &Path, to: &Path) -> io::Result<Moved> {
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    const ERROR_FILE_EXISTS: i32 = 80;
    const ERROR_ALREADY_EXISTS: i32 = 183;
    let (from, to) = (wide(from)?, wide(to)?);
    // SAFETY: どちらも NUL で終わる UTF-16 で、呼び出しの間生きている。フラグ 0 は MOVEFILE_REPLACE_EXISTING なし
    // （移動先があれば ERROR_ALREADY_EXISTS で断る）
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } != 0 {
        return Ok(Moved::Done);
    }
    let e = io::Error::last_os_error();
    match e.raw_os_error() {
        Some(ERROR_FILE_EXISTS | ERROR_ALREADY_EXISTS) => Ok(Moved::Occupied),
        _ => Err(e),
    }
}
/// NUL で終わる UTF-16 のパス。MAX_PATH に収まらない長さは、`std::fs` と同じく正規化した絶対パスを `\\?\` 形式にする。
#[cfg(windows)]
fn wide(path: &Path) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    const MAX_PATH: usize = 260;
    let mut units: Vec<u16> = path.as_os_str().encode_wide().collect();
    if units.contains(&0) {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    let verbatim = |prefix: &str, rest: &[u16]| -> Vec<u16> {
        prefix.encode_utf16().chain(rest.iter().copied()).collect()
    };
    if units.len() + 1 >= MAX_PATH && !units.starts_with(&verbatim(r"\\?\", &[])) {
        let absolute: Vec<u16> = std::path::absolute(path)?
            .as_os_str()
            .encode_wide()
            .collect();
        units = if absolute.starts_with(&verbatim(r"\\?\", &[])) {
            absolute
        } else if absolute.starts_with(&verbatim(r"\\", &[])) {
            verbatim(r"\\?\UNC\", &absolute[2..])
        } else {
            verbatim(r"\\?\", &absolute)
        };
    }
    units.push(0);
    Ok(units)
}
fn absent(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(e.into()),
    }
}
/// 開いたあとの保存先がまだあるか（消されていたら外部の変更として断る。中身は見ない）。
fn present(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(Error::SaveConflict(
            "保存先が外部で消されています。上書きしません".into(),
        )),
        Err(e) => Err(e.into()),
        Ok(_) => Ok(()),
    }
}
/// 開いたあとの保存先の印を数える。消されていたら外部の変更として断る（無いファイルの代わりに書き始めない）。
fn read_target(path: &Path) -> Result<FileStamp> {
    match read_stamp(path) {
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Err(Error::SaveConflict(
            "保存先が外部で消されています。上書きしません".into(),
        )),
        other => other,
    }
}
/// ファイルの印（SHA-256・長さ・更新時刻）を、流して数える（ファイル全体をメモリに読まない）。
fn read_stamp(path: &Path) -> Result<FileStamp> {
    let metadata = fs::symlink_metadata(path)?;
    check(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "通常のファイルではありません",
    )?;
    let (sha256, length) = file_digest(path)?;
    let after = fs::metadata(path)?;
    conflict(
        length == metadata.len()
            && after.len() == metadata.len()
            && after.modified()? == metadata.modified()?,
        "読み込み中にファイルが変更されました",
    )?;
    Ok(FileStamp {
        sha256,
        length,
        modified: metadata.modified()?,
    })
}
/// 保存の一時ファイル。名前は保存ごとに一意（pid と通し番号）なので、保存が確定したあとも落とすときに必ず消しにいく。
/// 置き換える移動で無くなっていれば `NotFound` で無害、リンクで移して元の名前を消せなかったときの残りはここで片付く。
struct Pending {
    path: PathBuf,
    file: Option<File>,
}
impl Pending {
    fn create(path: PathBuf) -> Result<Self> {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        let file = hide(&mut options).open(&path)?;
        Ok(Self {
            path,
            file: Some(file),
        })
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.file.take();
        let _ = fs::remove_file(&self.path);
    }
}
/// 保存のロック。「ロックのファイルがあるか」ではなく、開いたハンドルが OS の排他ロックを持っているかで決める。
/// 保存の途中でクラッシュして残ったファイルは誰も持っていないので次の保存の邪魔にならず、終わると消える。
struct SaveLock {
    path: PathBuf,
    file: Option<File>,
    /// OS の排他ロックを実際に持っている（ロックの使えないファイルシステムでは持たずに続けるので false）。
    exclusive: bool,
}
/// 取る間に他の保存がロックのファイルを消して作り直した場合にやり直す回数（超えたら衝突として断る）。
const LOCK_ATTEMPTS: usize = 8;
impl SaveLock {
    /// 本物の `try_lock` で取る（保存は `save_locking` が `acquire_with` を通る。ここは試験の入口）。
    #[cfg(test)]
    fn acquire(path: PathBuf) -> Result<Self> {
        Self::acquire_with(path, File::try_lock, |_| {})
    }
    /// `try_lock` は排他ロックを取る操作（試験が、ロックの使えないファイルシステムを差し込む口）。`locked` は排他ロックを取った
    /// 直後（パスが同じファイルを指すかの確認の前）に試行の番号で呼ばれる。試験が競合を作る口。
    fn acquire_with(
        path: PathBuf,
        mut try_lock: impl FnMut(&File) -> std::result::Result<(), TryLockError>,
        mut locked: impl FnMut(usize),
    ) -> Result<Self> {
        for attempt in 0..LOCK_ATTEMPTS {
            if let Ok(m) = fs::symlink_metadata(&path) {
                conflict(
                    !m.file_type().is_symlink(),
                    "ロックのファイルがシンボリックリンクです",
                )?;
            }
            let mut options = OpenOptions::new();
            options.read(true).write(true).truncate(false);
            // Windows は先頭の '.' で隠れないので、隠し属性を付けて作る
            hide(&mut options);
            // Windows は DELETE を共有しない: 開いている間は他から消せない（消すのは手放してから）
            #[cfg(windows)]
            std::os::windows::fs::OpenOptionsExt::share_mode(&mut options, 0x1 | 0x2);
            let file = match options.create(true).open(&path) {
                Ok(file) => file,
                // Windows: 別の保存が手放して消す途中のファイルは、消し終わるまで開けず共有違反になる。持っているのは別の保存
                // （本当のアクセス拒否・場所の不具合とは違うので、ここだけを「進行中」にする）
                #[cfg(windows)]
                Err(e) if e.raw_os_error() == Some(32) => return Err(in_progress()),
                // Windows: 消している途中（削除の保留）のファイルを開くと ERROR_ACCESS_DENIED（5）になる。消し終わるまでの
                // 短い間だけなので、少し待ってやり直す。何度でも拒否されるなら本当のアクセス拒否として返す
                #[cfg(windows)]
                Err(e) if e.raw_os_error() == Some(5) && attempt + 1 < LOCK_ATTEMPTS => {
                    std::thread::sleep(std::time::Duration::from_millis(1 << attempt.min(4)));
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            let exclusive = match try_lock(&file) {
                Ok(()) => true,
                // 持っているのは別の保存。そのファイルは消さずに断る
                Err(TryLockError::WouldBlock) => return Err(in_progress()),
                // ロックの使えない場所（ロックを取る操作そのものが失敗する FUSE・一部のネットワークのファイルシステム）。
                // C#（.NET の FileShare.None）と同じく排他なしで続ける。断ると、その場所へは二度と保存できない。
                // 置換の前の印の確かめは残る
                Err(TryLockError::Error(_)) => false,
            };
            locked(attempt);
            // 取る間に消されて作り直されていたら、いま持っているのは誰も見ないファイル。手放してやり直す
            if names_the_same_file(&file, &path) {
                return Ok(Self {
                    path,
                    file: Some(file),
                    exclusive,
                });
            }
        }
        Err(in_progress())
    }
}
impl Drop for SaveLock {
    fn drop(&mut self) {
        let Some(file) = self.file.take() else {
            return;
        };
        // Unix はロックを持ったまま消してから手放す（消す前に取った他の保存は、手放した後の同一性の確認でやり直す）。
        // 取ってから別のファイルに替わっていたら、他の保存のロックなので消さない
        #[cfg(unix)]
        {
            if names_the_same_file(&file, &self.path) {
                let _ = fs::remove_file(&self.path);
            }
            drop(file);
        }
        // Windows は手放してから消す。その間に他が開いていれば消えずに失敗するので、そのまま残す
        #[cfg(not(unix))]
        {
            drop(file);
            let _ = fs::remove_file(&self.path);
        }
    }
}
fn in_progress() -> Error {
    Error::SaveConflict("別の保存が進行中です".into())
}
/// 開いたハンドルとパスが同じファイルか。パスのシンボリックリンクは辿らない（辿らないので別のファイルとして断る）。
/// Unix 以外は、持っている間は他から消せない（共有しない）ので常に同じ。
#[cfg(unix)]
fn names_the_same_file(file: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (file.metadata(), fs::symlink_metadata(path)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}
#[cfg(not(unix))]
fn names_the_same_file(_: &File, _: &Path) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{hash, Thresholds};
    #[cfg(all(unix, target_os = "linux", target_env = "gnu"))]
    use std::os::unix::ffi::OsStrExt;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    /// 試験の作業フォルダの通し番号。
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let p = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/io-save-tests")
                .join(format!(
                    "{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn file(&self) -> PathBuf {
            self.0.join("sample.ylp")
        }
        fn lock(&self) -> PathBuf {
            self.0.join(".sample.ylp.save.lock~")
        }
        fn backups(&self) -> PathBuf {
            self.0.join("sample.ylp-backups~")
        }
        /// フォルダー直下の名前（並べ替え済み）。何も残っていないことの確認に使う。
        fn names(&self) -> Vec<String> {
            let mut names: Vec<String> = self
                .0
                .read_dir()
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
        /// 保存が残してはいけないもの（一時ファイルとロック）。
        fn leftovers(&self) -> Vec<String> {
            self.names()
                .into_iter()
                .filter(|n| n.ends_with("pending~") || n.ends_with(".save.lock~"))
                .collect()
        }
        /// 退避したものの名前（新しい順）。
        fn kept_names(&self) -> Vec<String> {
            backups(&self.file())
                .unwrap()
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect()
        }
        /// 退避したものの中身（新しい順）。
        fn kept(&self) -> Vec<Vec<u8>> {
            backups(&self.file())
                .unwrap()
                .iter()
                .map(|p| fs::read(p).unwrap())
                .collect()
        }
        /// 退避のフォルダーの中の名前（並べ替え済み。退避と関係の無いものも含む）。
        fn backup_dir_names(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(self.backups())
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    const ORIGINAL: &[u8] = include_bytes!("../tests/fixtures/format1.ylp");
    fn project() -> Project {
        Project::read(ORIGINAL).unwrap()
    }
    /// レイヤーの名前だけを変えた別の版（保存するとバイト列が変わる）。
    fn changed(p: &Project, name: &str) -> Project {
        let d = p.sets()[0]
            .document
            .with_value("layers[0].name", crate::NativeValue::Text(name.into()))
            .unwrap();
        p.with_document(&p.sets()[0].id, &d).unwrap()
    }
    /// バイト列の一致。外れたときに全バイトを出さず、長さとハッシュだけを出す。
    #[track_caller]
    fn assert_bytes(actual: &[u8], expected: &[u8], what: &str) {
        assert!(
            actual == expected,
            "{what}: 長さ {} と {}、SHA-256 {} と {}",
            actual.len(),
            expected.len(),
            hash(actual),
            hash(expected)
        );
    }
    /// 指定の段で保存に失敗を注入する。
    fn fail_at(point: &'static str) -> impl FnMut(&str) -> Result<()> {
        move |phase| {
            if phase == point {
                Err(Error::InvalidData("注入した保存障害".into()))
            } else {
                Ok(())
            }
        }
    }
    impl SaveTarget {
        /// 本物のロックと移動で、各段で `phase` を呼ぶ。
        fn save_inner(
            &mut self,
            project: &Project,
            mut phase: impl FnMut(&str) -> Result<()>,
        ) -> Result<FileStamp> {
            self.save_core(
                project,
                BackupKeep::All,
                &mut Seams {
                    try_lock: &mut File::try_lock,
                    move_new: &mut move_without_replacing,
                    remove: &mut remove_file,
                    phase: &mut phase,
                    rename: &mut rename_file,
                    busy: &is_busy,
                    sleep: &mut std::thread::sleep,
                },
            )
            .map(|r| r.stamp)
        }
        /// ロックを取る操作を差し込む（ロックの使えないファイルシステム）。
        fn save_locking(
            &mut self,
            project: &Project,
            mut try_lock: impl FnMut(&File) -> std::result::Result<(), TryLockError>,
            mut phase: impl FnMut(&str) -> Result<()>,
        ) -> Result<FileStamp> {
            self.save_core(
                project,
                BackupKeep::All,
                &mut Seams {
                    try_lock: &mut try_lock,
                    move_new: &mut move_without_replacing,
                    remove: &mut remove_file,
                    phase: &mut phase,
                    rename: &mut rename_file,
                    busy: &is_busy,
                    sleep: &mut std::thread::sleep,
                },
            )
            .map(|r| r.stamp)
        }
        /// 新規の保存先への置き換えない移動を差し込む（使えないファイルシステム・外から先に作られる競合）。
        fn save_moving(
            &mut self,
            project: &Project,
            mut move_new: impl FnMut(&Path, &Path) -> io::Result<Moved>,
            mut phase: impl FnMut(&str) -> Result<()>,
        ) -> Result<FileStamp> {
            self.save_core(
                project,
                BackupKeep::All,
                &mut Seams {
                    try_lock: &mut File::try_lock,
                    move_new: &mut move_new,
                    remove: &mut remove_file,
                    phase: &mut phase,
                    rename: &mut rename_file,
                    busy: &is_busy,
                    sleep: &mut std::thread::sleep,
                },
            )
            .map(|r| r.stamp)
        }
        /// 置き換える移動・やり直しに値する失敗の見分け・待ちを差し込む（Windows の共有違反を模す）。
        fn save_replacing(
            &mut self,
            project: &Project,
            rename: impl FnMut(&Path, &Path) -> io::Result<()>,
            busy: impl Fn(&io::Error) -> bool,
            sleep: impl FnMut(Duration),
        ) -> Result<FileStamp> {
            self.save_replacing_report(project, rename, busy, sleep)
                .map(|r| r.stamp)
        }
        /// `save_replacing` の、保存の報告（保存後のプロジェクト）を返す形。
        fn save_replacing_report(
            &mut self,
            project: &Project,
            mut rename: impl FnMut(&Path, &Path) -> io::Result<()>,
            busy: impl Fn(&io::Error) -> bool,
            mut sleep: impl FnMut(Duration),
        ) -> Result<SaveReport> {
            self.save_core(
                project,
                BackupKeep::All,
                &mut Seams {
                    try_lock: &mut File::try_lock,
                    move_new: &mut move_without_replacing,
                    remove: &mut remove_file,
                    phase: &mut |_| Ok(()),
                    rename: &mut rename,
                    busy: &busy,
                    sleep: &mut sleep,
                },
            )
        }
        /// 置き換えない移動（新規の保存先と、退避の最終の名前への移動）・やり直しに値する失敗の見分け・待ちを差し込む。
        fn save_moving_busy(
            &mut self,
            project: &Project,
            mut move_new: impl FnMut(&Path, &Path) -> io::Result<Moved>,
            busy: impl Fn(&io::Error) -> bool,
            mut sleep: impl FnMut(Duration),
        ) -> Result<SaveReport> {
            self.save_core(
                project,
                BackupKeep::All,
                &mut Seams {
                    try_lock: &mut File::try_lock,
                    move_new: &mut move_new,
                    remove: &mut remove_file,
                    phase: &mut |_| Ok(()),
                    rename: &mut rename_file,
                    busy: &busy,
                    sleep: &mut sleep,
                },
            )
        }
        /// 退避を消す操作を差し込む（消せない退避）。
        fn save_removing(
            &mut self,
            project: &Project,
            keep: BackupKeep,
            mut remove: impl FnMut(&Path) -> io::Result<()>,
        ) -> Result<SaveReport> {
            self.save_core(
                project,
                keep,
                &mut Seams {
                    try_lock: &mut File::try_lock,
                    move_new: &mut move_without_replacing,
                    remove: &mut remove,
                    phase: &mut |_| Ok(()),
                    rename: &mut rename_file,
                    busy: &is_busy,
                    sleep: &mut std::thread::sleep,
                },
            )
        }
    }
    fn open_original(s: &Scratch) -> (Project, SaveTarget) {
        fs::write(s.file(), ORIGINAL).unwrap();
        SaveTarget::open(s.file()).unwrap()
    }
    /// ロックのファイルを別のハンドルで開いて排他ロックを持つ（別の保存が進行中の状態）。
    fn hold(path: &Path) -> File {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .unwrap();
        file.try_lock().unwrap();
        file
    }

    #[test]
    fn create_save_open_and_replace_keep_previous_backup() {
        let s = Scratch::new();
        let p = project();
        let mut t = SaveTarget::create(s.file()).unwrap();
        let stamp = t.save(&p).unwrap();
        let before = fs::read(s.file()).unwrap();
        let (p, mut t) = SaveTarget::open(s.file()).unwrap();
        let d = p.sets()[0]
            .document
            .with_value(
                "layers[0].name",
                crate::NativeValue::Text("変更したレイヤー".into()),
            )
            .unwrap();
        let changed = p.with_document(&p.sets()[0].id, &d).unwrap();
        let after = t.save(&changed).unwrap();
        assert_ne!(stamp.sha256, after.sha256);
        assert_eq!(s.kept(), [before]);
        assert!(s.0.read_dir().unwrap().all(|p| !p
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with("pending~")));
    }
    #[test]
    fn failure_at_every_precommit_phase_preserves_original() {
        for phase in [
            "memory-verified",
            "flushed",
            "disk-verified",
            "before-replace",
            "backup-copied",
            "backed-up",
        ] {
            let s = Scratch::new();
            let (p, mut t) = open_original(&s);
            let stamp = t.stamp().unwrap().clone();
            let error = t.save_inner(&p, fail_at(phase));
            assert!(error.is_err());
            assert_bytes(&fs::read(s.file()).unwrap(), ORIGINAL, "保存先");
            assert!(!s.lock().exists());
            assert!(s.leftovers().is_empty(), "{phase}");
            // 置換に至らなかった保存は、退避も（作ったフォルダーごと）残さない
            assert!(!s.backups().exists(), "{phase}: {:?}", s.names());
            // 失敗した保存は印を変えず、次の保存がそのまま通る
            assert_eq!(t.stamp(), Some(&stamp), "{phase}");
            assert!(t.save(&p).is_ok(), "{phase}");
        }
    }
    #[test]
    fn phases_run_in_order_and_the_replace_is_the_last_step_before_after_replace() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let mut seen = Vec::new();
        t.save_inner(&changed(&p, "段の順"), |phase| {
            seen.push(phase.to_string());
            Ok(())
        })
        .unwrap();
        assert_eq!(
            seen,
            [
                "memory-verified",
                "flushed",
                "disk-verified",
                "before-replace",
                "backup-copied",
                "backed-up",
                "after-replace"
            ]
        );
    }
    #[test]
    fn a_failure_after_replacing_still_commits_the_new_file() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let before = t.stamp().unwrap().clone();
        let next = changed(&p, "置換のあとで失敗");
        let want = next.to_bytes().unwrap();
        let error = t.save_inner(&next, fail_at("after-replace"));
        // 失敗として返る（承認を失った状態）が、新しい版は確定している
        assert!(matches!(error, Err(Error::InvalidData(_))), "{error:?}");
        assert_bytes(&fs::read(s.file()).unwrap(), &want, "保存先");
        // 印は新しい版。外からの書き換えと見なさず、次の保存がそのまま通る
        let stamp = t.stamp().unwrap().clone();
        assert_ne!(stamp.sha256, before.sha256);
        assert_eq!(stamp.sha256, hash(&want));
        assert_eq!(stamp.length, want.len() as u64);
        assert_eq!(stamp, read_stamp(&s.file()).unwrap());
        // 置換された前の版は退避されている。ロックと一時ファイルは残らない
        assert_eq!(s.kept(), [ORIGINAL.to_vec()]);
        assert!(s.leftovers().is_empty());
        let third = t.save(&changed(&p, "もう一度")).unwrap();
        assert_ne!(third.sha256, stamp.sha256);
        assert_eq!(s.kept(), [want.clone(), ORIGINAL.to_vec()], "新しい順");
        assert!(s.leftovers().is_empty());
    }
    #[test]
    fn a_failure_at_every_phase_of_a_new_save_leaves_nothing() {
        for phase in [
            "memory-verified",
            "flushed",
            "disk-verified",
            "before-replace",
            "backed-up",
        ] {
            let s = Scratch::new();
            let mut t = SaveTarget::create(s.file()).unwrap();
            assert!(t.save_inner(&project(), fail_at(phase)).is_err());
            // 目的のファイルも一時ファイルもロックも、退避のフォルダーも残らない
            assert!(s.names().is_empty(), "{phase}: {:?}", s.names());
            assert!(t.stamp().is_none(), "{phase}");
            // 同じ保存先へそのまま保存できる
            t.save(&project()).unwrap();
            assert_eq!(s.names(), ["sample.ylp"], "{phase}");
        }
    }
    #[test]
    fn a_failure_after_replacing_a_new_file_still_commits_it() {
        let s = Scratch::new();
        let mut t = SaveTarget::create(s.file()).unwrap();
        let p = project();
        assert!(t.save_inner(&p, fail_at("after-replace")).is_err());
        let want = p.to_bytes().unwrap();
        assert_bytes(&fs::read(s.file()).unwrap(), &want, "保存先");
        assert_eq!(t.stamp(), Some(&read_stamp(&s.file()).unwrap()));
        assert_eq!(s.names(), ["sample.ylp"]);
        // 以後は上書きとして、前の版を残して保存できる
        let next = t.save(&changed(&p, "新規のあとの上書き")).unwrap();
        assert_eq!(s.names(), ["sample.ylp", "sample.ylp-backups~"]);
        assert_ne!(next.sha256, hash(&want));
        assert_eq!(s.kept(), [want]);
    }
    #[test]
    fn external_change_just_before_replace_is_refused() {
        let s = Scratch::new();
        fs::write(s.file(), ORIGINAL).unwrap();
        let (p, mut t) = SaveTarget::open(s.file()).unwrap();
        assert!(t
            .save_inner(&p, |phase| {
                if phase == "before-replace" {
                    fs::write(s.file(), b"external")?;
                }
                Ok(())
            })
            .is_err());
        assert_eq!(fs::read(s.file()).unwrap(), b"external");
    }
    #[test]
    fn an_outside_change_during_the_save_is_caught_before_replacing() {
        for point in [
            "memory-verified",
            "flushed",
            "disk-verified",
            "before-replace",
        ] {
            let s = Scratch::new();
            let (p, mut t) = open_original(&s);
            let outside = b"external bytes";
            let error = t.save_inner(&p, |phase| {
                if phase == point {
                    fs::write(s.file(), outside)?;
                }
                Ok(())
            });
            assert!(
                matches!(error, Err(Error::SaveConflict(_))),
                "{point}: {error:?}"
            );
            assert_eq!(fs::read(s.file()).unwrap(), outside, "{point}");
            // 外で書いた中身を潰さず、前の版の退避も作らない
            assert!(!s.backups().exists(), "{point}");
            assert!(s.leftovers().is_empty(), "{point}");
        }
    }
    #[test]
    fn a_rewrite_with_the_same_length_and_time_is_still_refused() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let stamp = t.stamp().unwrap().clone();
        let mut bytes = fs::read(s.file()).unwrap();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 1;
        fs::write(s.file(), &bytes).unwrap();
        OpenOptions::new()
            .write(true)
            .open(s.file())
            .unwrap()
            .set_modified(stamp.modified)
            .unwrap();
        let now = read_stamp(&s.file()).unwrap();
        assert_eq!((now.length, now.modified), (stamp.length, stamp.modified));
        // 長さも更新時刻も同じ。印は内容のハッシュなので見逃さない
        let error = t.save(&p).unwrap_err();
        assert!(matches!(error, Error::SaveConflict(_)), "{error:?}");
        assert_bytes(&fs::read(s.file()).unwrap(), &bytes, "保存先");
        assert!(!s.backups().exists());
    }
    #[test]
    fn touching_the_file_without_changing_it_does_not_block_saving() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let stamp = t.stamp().unwrap().clone();
        OpenOptions::new()
            .write(true)
            .open(s.file())
            .unwrap()
            .set_modified(stamp.modified + std::time::Duration::from_secs(3600))
            .unwrap();
        // 更新時刻だけが変わった（同期ツールなど）のは外部の書き換えではない
        let next = t.save(&changed(&p, "更新時刻だけ変わったあと")).unwrap();
        assert_ne!(next.sha256, stamp.sha256);
        assert_eq!(s.kept(), [ORIGINAL.to_vec()]);
    }
    #[test]
    fn a_new_target_created_outside_is_not_overwritten() {
        let s = Scratch::new();
        let mut t = SaveTarget::create(s.file()).unwrap();
        fs::write(s.file(), b"external").unwrap();
        let error = t.save(&project()).unwrap_err();
        assert!(matches!(error, Error::SaveConflict(_)), "{error:?}");
        assert!(SaveTarget::create(s.file()).is_err());
        assert_eq!(fs::read(s.file()).unwrap(), b"external");
        assert!(s.leftovers().is_empty());
        // 外の物が無くなれば、同じ保存先へ保存できる
        fs::remove_file(s.file()).unwrap();
        t.save(&project()).unwrap();
        assert_bytes(
            &fs::read(s.file()).unwrap(),
            &project().to_bytes().unwrap(),
            "保存先",
        );
    }
    #[test]
    fn a_deleted_file_is_refused_when_a_stamp_is_expected() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        fs::remove_file(s.file()).unwrap();
        let error = t.save(&p).unwrap_err();
        assert!(matches!(error, Error::SaveConflict(_)), "{error:?}");
        // 無いファイルの代わりに書き始めない。ロック・一時ファイル・退避も残さない
        assert!(s.names().is_empty(), "{:?}", s.names());
    }
    #[test]
    fn a_file_deleted_during_the_save_is_not_recreated() {
        for point in ["flushed", "disk-verified", "before-replace"] {
            let s = Scratch::new();
            let (p, mut t) = open_original(&s);
            let error = t.save_inner(&p, |phase| {
                if phase == point {
                    fs::remove_file(s.file())?;
                }
                Ok(())
            });
            assert!(
                matches!(error, Err(Error::SaveConflict(_))),
                "{point}: {error:?}"
            );
            assert!(!s.file().exists(), "{point}");
            assert!(s.leftovers().is_empty(), "{point}");
        }
    }
    #[test]
    fn a_stale_lock_file_from_a_crash_does_not_block_saving_and_is_removed() {
        // 既存の保存先
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        fs::write(s.lock(), b"crashed saver: 12345").unwrap();
        t.save(&changed(&p, "残ったロックのあと")).unwrap();
        assert!(!s.lock().exists());
        assert!(s.leftovers().is_empty());
        // 新規の保存先
        let s = Scratch::new();
        let mut t = SaveTarget::create(s.file()).unwrap();
        fs::write(s.lock(), b"crashed saver: 12345").unwrap();
        t.save(&project()).unwrap();
        assert_eq!(s.names(), ["sample.ylp"]);
    }
    #[test]
    fn a_held_lock_makes_save_fail_without_touching_the_target() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let stamp = t.stamp().unwrap().clone();
        let holder = hold(&s.lock());
        let error = t.save(&p).unwrap_err();
        assert!(
            matches!(&error, Error::SaveConflict(why) if why.contains("進行中")),
            "{error:?}"
        );
        // 保存先も印も変わらず、一時ファイルも作らない。持っている側のロックのファイルは消さない
        assert_bytes(&fs::read(s.file()).unwrap(), ORIGINAL, "保存先");
        assert_eq!(t.stamp(), Some(&stamp));
        assert_eq!(s.names(), [".sample.ylp.save.lock~", "sample.ylp"]);
        // 手放されたら通り、終わるとロックのファイルは消える
        drop(holder);
        t.save(&changed(&p, "手放されたあと")).unwrap();
        assert_eq!(s.names(), ["sample.ylp", "sample.ylp-backups~"]);
        // 新規の保存先も、持たれている間は断る
        let s = Scratch::new();
        let mut t = SaveTarget::create(s.file()).unwrap();
        let holder = hold(&s.lock());
        assert!(matches!(t.save(&project()), Err(Error::SaveConflict(_))));
        assert!(!s.file().exists());
        drop(holder);
        t.save(&project()).unwrap();
    }
    #[test]
    fn only_one_saver_holds_the_lock_at_a_time() {
        let s = Scratch::new();
        let path = s.lock();
        let inside = AtomicUsize::new(0);
        let entered = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..6 {
                scope.spawn(|| {
                    let mut done = 0;
                    while done < 25 {
                        match SaveLock::acquire(path.clone()) {
                            Ok(lock) => {
                                assert_eq!(
                                    inside.fetch_add(1, Ordering::SeqCst),
                                    0,
                                    "ロックの中に 2 つの保存がいる"
                                );
                                std::thread::yield_now();
                                entered.fetch_add(1, Ordering::SeqCst);
                                inside.fetch_sub(1, Ordering::SeqCst);
                                drop(lock);
                                done += 1;
                            }
                            Err(Error::SaveConflict(_)) => std::thread::yield_now(),
                            Err(e) => panic!("{e:?}"),
                        }
                    }
                });
            }
        });
        assert_eq!(entered.load(Ordering::SeqCst), 150);
        assert!(!path.exists(), "最後の保存が消す");
    }
    /// ロックの使えないファイルシステム（`try_lock` が WouldBlock 以外で失敗する）を差し込む。
    fn unsupported_lock(_: &File) -> std::result::Result<(), TryLockError> {
        Err(TryLockError::Error(std::io::Error::from_raw_os_error(38)))
    }
    #[test]
    fn a_file_system_without_locks_saves_without_exclusion_and_leaves_nothing() {
        // C#（.NET の FileShare.None）と同じく、ロックの使えない場所でも保存は通り、ロックのファイルも一時ファイルも残さない
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let mut phases = Vec::new();
        t.save_locking(
            &changed(&p, "ロックの無い場所"),
            unsupported_lock,
            |phase| {
                phases.push(phase.to_string());
                Ok(())
            },
        )
        .unwrap();
        assert!(phases.iter().any(|x| x == "after-replace"), "{phases:?}");
        assert_eq!(s.names(), ["sample.ylp", "sample.ylp-backups~"]);
        // 置換の前の印の確かめは残る: 外で変えられていれば断る
        fs::write(s.file(), b"changed outside").unwrap();
        assert!(matches!(
            t.save_locking(&p, unsupported_lock, |_| Ok(())),
            Err(Error::SaveConflict(_))
        ));
        assert_eq!(fs::read(s.file()).unwrap(), b"changed outside");
        // 新規の保存先も同じ
        let s = Scratch::new();
        let mut t = SaveTarget::create(s.file()).unwrap();
        t.save_locking(&project(), unsupported_lock, |_| Ok(()))
            .unwrap();
        assert_eq!(s.names(), ["sample.ylp"]);
    }
    #[test]
    fn a_stale_lock_file_is_cleared_even_where_locks_are_unavailable() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        fs::write(s.lock(), b"left by a crash").unwrap();
        t.save_locking(&changed(&p, "残ったロック"), unsupported_lock, |_| {
            Ok(())
        })
        .unwrap();
        assert!(s.leftovers().is_empty(), "{:?}", s.leftovers());
    }
    #[test]
    fn a_lock_file_taken_by_someone_else_in_the_meantime_is_not_removed() {
        // 作った直後に別の保存が持った（WouldBlock）なら、そのファイルは持ち主のもの。自分が作っていても消さない
        let s = Scratch::new();
        let error = SaveLock::acquire_with(
            s.lock(),
            |_| Err(TryLockError::WouldBlock),
            |_| unreachable!(),
        )
        .err()
        .unwrap();
        assert!(
            matches!(&error, Error::SaveConflict(why) if why.contains("進行中")),
            "{error:?}"
        );
        assert!(s.lock().exists());
    }
    #[cfg(unix)]
    #[test]
    fn a_lock_taken_on_a_replaced_file_is_given_up_and_taken_again() {
        let s = Scratch::new();
        let mut attempts = Vec::new();
        let lock = SaveLock::acquire_with(s.lock(), File::try_lock, |attempt| {
            attempts.push(attempt);
            if attempt < 2 {
                // 取る間に、他の保存が消して作り直した
                fs::remove_file(s.lock()).unwrap();
                fs::write(s.lock(), b"recreated").unwrap();
            }
        })
        .unwrap();
        assert_eq!(attempts, [0, 1, 2]);
        // いま持っているのはパスのファイル。別のハンドルでは取れない
        let other = OpenOptions::new()
            .read(true)
            .write(true)
            .open(s.lock())
            .unwrap();
        assert!(matches!(other.try_lock(), Err(TryLockError::WouldBlock)));
        drop(other);
        drop(lock);
        assert!(!s.lock().exists());
    }
    #[cfg(unix)]
    #[test]
    fn a_lock_file_that_keeps_being_replaced_is_refused_and_left_alone() {
        let s = Scratch::new();
        let mut attempts = 0;
        let error = SaveLock::acquire_with(s.lock(), File::try_lock, |_| {
            attempts += 1;
            fs::remove_file(s.lock()).unwrap();
            fs::write(s.lock(), b"someone else's").unwrap();
        })
        .err()
        .unwrap();
        assert!(matches!(error, Error::SaveConflict(_)), "{error:?}");
        assert_eq!(attempts, LOCK_ATTEMPTS);
        // 他の保存のファイルを消さない
        assert_eq!(fs::read(s.lock()).unwrap(), b"someone else's");
    }
    #[cfg(unix)]
    #[test]
    fn a_lock_file_replaced_while_held_is_not_removed_by_the_release() {
        let s = Scratch::new();
        let lock = SaveLock::acquire(s.lock()).unwrap();
        fs::remove_file(s.lock()).unwrap();
        fs::write(s.lock(), b"another saver").unwrap();
        drop(lock);
        assert_eq!(fs::read(s.lock()).unwrap(), b"another saver");
    }
    #[cfg(unix)]
    #[test]
    fn a_symlinked_lock_path_is_refused() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let elsewhere = s.0.join("elsewhere.txt");
        fs::write(&elsewhere, b"precious").unwrap();
        std::os::unix::fs::symlink(&elsewhere, s.lock()).unwrap();
        let error = t.save(&p).unwrap_err();
        assert!(
            matches!(&error, Error::SaveConflict(why) if why.contains("ロックのファイル")),
            "{error:?}"
        );
        assert_eq!(fs::read(&elsewhere).unwrap(), b"precious");
        assert!(fs::symlink_metadata(s.lock())
            .unwrap()
            .file_type()
            .is_symlink());
        assert_bytes(&fs::read(s.file()).unwrap(), ORIGINAL, "保存先");
    }
    #[test]
    fn a_backup_folder_blocked_by_a_file_fails_without_touching_the_target() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let stamp = t.stamp().unwrap().clone();
        fs::write(s.backups(), b"a file where the folder should be").unwrap();
        let error = t.save(&changed(&p, "塞がれた退避先")).unwrap_err();
        // プロジェクトのデータの不正（InvalidData）ではなく、保存先の周りの事情として言い分けられる
        assert!(
            matches!(&error, Error::SaveConflict(why) if why.contains("バックアップ先がフォルダーではありません")),
            "{error:?}"
        );
        assert_bytes(&fs::read(s.file()).unwrap(), ORIGINAL, "保存先");
        assert_eq!(
            fs::read(s.backups()).unwrap(),
            b"a file where the folder should be"
        );
        assert_eq!(t.stamp(), Some(&stamp));
        assert!(s.leftovers().is_empty());
        // 塞ぎを外せば、そのまま保存できて前の版が退避される
        fs::remove_file(s.backups()).unwrap();
        t.save(&changed(&p, "塞ぎを外したあと")).unwrap();
        assert_eq!(s.kept(), [ORIGINAL.to_vec()]);
    }
    #[cfg(unix)]
    #[test]
    fn a_symlinked_backup_folder_is_refused_without_writing_through_it() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let elsewhere = s.0.join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, s.backups()).unwrap();
        let error = t.save(&p).unwrap_err();
        assert!(
            matches!(&error, Error::SaveConflict(why) if why.contains("バックアップ先がシンボリックリンク")),
            "{error:?}"
        );
        assert_eq!(elsewhere.read_dir().unwrap().count(), 0);
        assert_bytes(&fs::read(s.file()).unwrap(), ORIGINAL, "保存先");
        assert!(s.leftovers().is_empty());
    }
    #[cfg(unix)]
    #[test]
    fn dangling_symlink_is_not_a_new_target() {
        let s = Scratch::new();
        std::os::unix::fs::symlink(s.0.join("missing.ylp"), s.file()).unwrap();
        assert!(SaveTarget::create(s.file()).is_err());
        assert!(fs::symlink_metadata(s.file())
            .unwrap()
            .file_type()
            .is_symlink());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_save_target_is_refused() {
        let s = Scratch::new();
        let original = s.0.join("original.ylp");
        fs::write(&original, ORIGINAL).unwrap();
        std::os::unix::fs::symlink(&original, s.file()).unwrap();
        assert!(SaveTarget::open(s.file()).is_err());
    }

    // ───────────── 退避の保持数 ─────────────

    /// 初めの版（ORIGINAL）から `n` 回、レイヤーの名前を変えて保存し、保存のたびのファイルの中身を返す（先頭は ORIGINAL）。
    fn save_versions(s: &Scratch, keep: BackupKeep, n: usize) -> Vec<Vec<u8>> {
        let (p, mut t) = open_original(s);
        let mut versions = vec![ORIGINAL.to_vec()];
        for i in 1..=n {
            t.save_with(&changed(&p, &format!("版 {i}")), keep).unwrap();
            versions.push(fs::read(s.file()).unwrap());
        }
        versions
    }
    fn at(seconds: u64, millis: u32) -> SystemTime {
        UNIX_EPOCH + std::time::Duration::new(seconds, millis * 1_000_000)
    }
    fn touch(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    #[test]
    fn the_default_keeps_every_backup() {
        assert_eq!(BackupKeep::default(), BackupKeep::All);
        let s = Scratch::new();
        let versions = save_versions(&s, BackupKeep::default(), 6);
        let mut want: Vec<Vec<u8>> = versions[..6].to_vec();
        want.reverse();
        assert_eq!(s.kept(), want, "6 回の上書きで 6 つ、新しい順");
    }
    #[test]
    fn keeping_n_keeps_the_n_newest_and_the_one_just_made() {
        let s = Scratch::new();
        let versions = save_versions(&s, BackupKeep::Count(2), 5);
        // 5 回目の保存の退避は版 4。今回の退避も数に入るので、残るのは版 4 と版 3
        assert_eq!(s.kept(), [versions[4].clone(), versions[3].clone()]);
        assert_eq!(s.backup_dir_names().len(), 2);
    }
    #[test]
    fn keeping_one_keeps_only_the_backup_just_made() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let mut last = ORIGINAL.to_vec();
        for i in 1..=4 {
            let report = t
                .save_with(&changed(&p, &format!("版 {i}")), BackupKeep::Count(1))
                .unwrap();
            assert!(report.prune_failures.is_empty());
            // 返した退避は実在し、直前の版そのもの
            let made = report.backup.unwrap();
            assert_eq!(fs::read(&made).unwrap(), last);
            assert_eq!(backups(&s.file()).unwrap(), [made]);
            last = fs::read(s.file()).unwrap();
        }
    }
    #[test]
    fn keeping_zero_makes_no_backup_and_never_touches_the_place() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let report = t
            .save_with(&changed(&p, "退避しない"), BackupKeep::Count(0))
            .unwrap();
        assert!(report.backup.is_none() && report.prune_failures.is_empty());
        assert!(!s.backups().exists(), "{:?}", s.names());
        assert_eq!(s.names(), ["sample.ylp"]);
        // 置き場が塞がれていても、退避しないなら関係なく保存できる
        fs::write(s.backups(), b"a file where the folder should be").unwrap();
        let next = changed(&p, "塞がれていても").to_bytes().unwrap();
        t.save_with(&changed(&p, "塞がれていても"), BackupKeep::Count(0))
            .unwrap();
        assert_bytes(&fs::read(s.file()).unwrap(), &next, "保存先");
        assert_eq!(
            fs::read(s.backups()).unwrap(),
            b"a file where the folder should be"
        );
    }
    #[test]
    fn keeping_zero_does_not_delete_the_backups_that_already_exist() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        t.save(&changed(&p, "一つ目")).unwrap();
        t.save(&changed(&p, "二つ目")).unwrap();
        let before = s.kept();
        assert_eq!(before.len(), 2);
        t.save_with(&changed(&p, "三つ目"), BackupKeep::Count(0))
            .unwrap();
        // 0 は「これから退避しない」で、すでにある版を消す設定ではない
        assert_eq!(s.kept(), before);
    }
    #[test]
    fn lowering_the_limit_prunes_older_backups_but_leaves_foreign_files() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        for i in 1..=3 {
            t.save(&changed(&p, &format!("版 {i}"))).unwrap();
        }
        let dir = s.backups();
        // この文書の退避ではないもの: 別の文書の時刻の名前・利用者が名前を変えて残した版・メモ・フォルダー・形の崩れた名前
        let foreign = [
            "notes.txt",
            "other-20200101T000000000Z.ylp",
            "sample-keep.ylp",
            "sample-20200101T00000000Z.ylp",
            "sample-20200101T000000000z.ylp",
            "sample-20200101T000000000Z.YLP",
            "sample-20200101T000000000Z_x.ylp",
        ];
        for name in foreign {
            touch(&dir.join(name), b"mine");
        }
        fs::create_dir(dir.join("sample-20190101T000000000Z.ylp")).unwrap();
        #[cfg(unix)]
        {
            let outside = s.0.join("outside.txt");
            fs::write(&outside, b"precious").unwrap();
            std::os::unix::fs::symlink(&outside, dir.join("sample-20180101T000000000Z.ylp"))
                .unwrap();
        }
        assert_eq!(s.kept().len(), 3);
        let last = fs::read(s.file()).unwrap();
        let report = t
            .save_with(&changed(&p, "版 5"), BackupKeep::Count(1))
            .unwrap();
        assert!(report.prune_failures.is_empty());
        assert_eq!(s.kept(), [last], "残るのは今回の退避だけ");
        for name in foreign {
            assert_eq!(fs::read(dir.join(name)).unwrap(), b"mine", "{name}");
        }
        assert!(dir.join("sample-20190101T000000000Z.ylp").is_dir());
        #[cfg(unix)]
        {
            assert!(
                fs::symlink_metadata(dir.join("sample-20180101T000000000Z.ylp"))
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(fs::read(s.0.join("outside.txt")).unwrap(), b"precious");
        }
    }
    #[test]
    fn the_backup_just_made_survives_pruning_next_to_a_renamed_backup() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        touch(&s.backups().join("sample-keep.ylp"), b"renamed by the user");
        let report = t
            .save_with(&changed(&p, "名前を変えた版の隣"), BackupKeep::Count(1))
            .unwrap();
        let made = report.backup.unwrap();
        assert_eq!(fs::read(&made).unwrap(), ORIGINAL);
        assert_eq!(
            fs::read(s.backups().join("sample-keep.ylp")).unwrap(),
            b"renamed by the user"
        );
    }
    #[test]
    fn a_save_that_does_not_reach_the_replace_prunes_nothing_and_leaves_no_new_backup() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        for i in 1..=3 {
            t.save(&changed(&p, &format!("版 {i}"))).unwrap();
        }
        let before = s.backup_dir_names();
        for point in [
            "memory-verified",
            "flushed",
            "disk-verified",
            "before-replace",
            "backup-copied",
            "backed-up",
        ] {
            let error = t.save_core(
                &changed(&p, "止まる保存"),
                BackupKeep::Count(1),
                &mut Seams {
                    try_lock: &mut File::try_lock,
                    move_new: &mut move_without_replacing,
                    remove: &mut |_| panic!("置換の前に整理しない"),
                    phase: &mut fail_at(point),
                    rename: &mut rename_file,
                    busy: &is_busy,
                    sleep: &mut std::thread::sleep,
                },
            );
            assert!(error.is_err(), "{point}");
            assert_eq!(s.backup_dir_names(), before, "{point}");
        }
    }
    #[test]
    fn a_folder_that_was_there_stays_when_a_save_fails_after_the_backup() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        fs::create_dir(s.backups()).unwrap();
        assert!(t
            .save_inner(&changed(&p, "止まる"), fail_at("backed-up"))
            .is_err());
        // もとからあった置き場は空でも残し、作った退避だけを消す
        assert!(s.backups().is_dir());
        assert!(s.backup_dir_names().is_empty());
    }
    #[test]
    fn pruning_that_fails_keeps_the_save_and_reports_why() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        for i in 1..=4 {
            t.save(&changed(&p, &format!("版 {i}"))).unwrap();
        }
        let listed = backups(&s.file()).unwrap();
        assert_eq!(listed.len(), 4);
        // 保存の外で、消せない（権限）・先に消されている・消せる、が混ざる
        let (stuck, gone) = (listed[2].clone(), listed[3].clone());
        let mut tried = Vec::new();
        let next = changed(&p, "版 5");
        let report = t
            .save_removing(&next, BackupKeep::Count(2), |path| {
                tried.push(path.to_path_buf());
                if path == stuck {
                    Err(io::Error::from(io::ErrorKind::PermissionDenied))
                } else {
                    remove_file(path)
                }
            })
            .unwrap();
        // 保存は成功し、新しい版が確定していて、印も新しい
        assert_bytes(
            &fs::read(s.file()).unwrap(),
            &next.to_bytes().unwrap(),
            "保存先",
        );
        assert_eq!(t.stamp(), Some(&report.stamp));
        assert_eq!(report.stamp, read_stamp(&s.file()).unwrap());
        // 消せなかったものと理由を返し、ほかの古い版は消えている
        assert_eq!(report.prune_failures.len(), 1);
        assert_eq!(report.prune_failures[0].path, stuck);
        assert_eq!(
            report.prune_failures[0].error.kind(),
            io::ErrorKind::PermissionDenied
        );
        // 新しい方から 1 つ（今回の退避と合わせて 2 つ）を残し、残りを古い方へ消そうとする。消せないものがあっても続ける
        assert_eq!(
            tried,
            listed[1..],
            "消せないものがあっても、残りの整理を続ける"
        );
        assert!(stuck.exists() && !gone.exists() && listed[0].exists() && !listed[1].exists());
        assert!(s.leftovers().is_empty());
        // 次の保存はそのまま通る（消せなかったものは、また整理の対象になる）
        t.save_with(&changed(&p, "版 6"), BackupKeep::Count(2))
            .unwrap();
        assert_eq!(s.kept().len(), 2);
        assert!(!stuck.exists());
    }
    #[test]
    fn an_unreadable_backup_folder_is_reported_not_fatal() {
        // 整理のとき退避のフォルダーを読めなければ、そのフォルダーと理由を返す（保存は成功のまま）
        let s = Scratch::new();
        let failures = prune(
            "sample",
            1,
            &s.0.join("no-such-folder").join("x.ylp"),
            &mut |_| Ok(()),
        );
        // フォルダーが無いのは「退避が無い」なので失敗ではない
        assert!(failures.is_empty());
        let file = s.0.join("a-file");
        fs::write(&file, b"x").unwrap();
        let failures = prune("sample", 1, &file.join("x.ylp"), &mut |_| Ok(()));
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].path, file);
    }
    #[test]
    fn backups_made_by_an_earlier_version_with_hash_names_count_as_the_oldest() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let dir = s.backups();
        let old = |c: char| format!("{}.ylp", c.to_string().repeat(64));
        for (c, seconds) in [('a', 1_000), ('b', 2_000), ('c', 3_000)] {
            touch(&dir.join(old(c)), c.to_string().as_bytes());
            OpenOptions::new()
                .write(true)
                .open(dir.join(old(c)))
                .unwrap()
                .set_modified(at(seconds, 0))
                .unwrap();
        }
        // 64 桁の 16 進ではない .ylp は、この文書の退避ではない
        touch(&dir.join("zzzz.ylp"), b"mine");
        // 時刻の名前のものが 1 つあれば、それが新しい。以前の名前は更新時刻の順に古い方から消える
        t.save(&changed(&p, "時刻の名前 1")).unwrap();
        let names = s.kept_names();
        assert_eq!(names.len(), 4);
        assert!(names[0].starts_with("sample-"), "{names:?}");
        assert_eq!(names[1..], [old('c'), old('b'), old('a')]);
        let report = t
            .save_with(&changed(&p, "時刻の名前 2"), BackupKeep::Count(3))
            .unwrap();
        assert!(report.prune_failures.is_empty());
        let names = s.kept_names();
        assert_eq!(names.len(), 3);
        assert!(
            names[0].starts_with("sample-") && names[1].starts_with("sample-"),
            "{names:?}"
        );
        assert_eq!(names[2], old('c'));
        assert_eq!(fs::read(dir.join("zzzz.ylp")).unwrap(), b"mine");
    }
    #[test]
    fn rapid_saves_keep_the_backups_newest_first() {
        let s = Scratch::new();
        // 同じミリ秒に重なる保存も、並びが新しい順のまま
        let versions = save_versions(&s, BackupKeep::All, 8);
        let mut want: Vec<Vec<u8>> = versions[..8].to_vec();
        want.reverse();
        assert_eq!(s.kept(), want);
        let names = s.kept_names();
        let mut sorted = names.clone();
        sorted.sort_by_key(|n| n.replace('_', ""));
        sorted.reverse();
        assert_eq!(
            names.iter().map(|n| n.replace('_', "")).collect::<Vec<_>>(),
            sorted
                .iter()
                .map(|n| n.replace('_', ""))
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn backup_names_use_the_utc_time_in_milliseconds() {
        assert_eq!(stamp_of(at(0, 0)), "19700101T000000000Z");
        assert_eq!(stamp_of(at(1_000_000_000, 123)), "20010909T014640123Z");
        assert_eq!(stamp_of(at(946_684_799, 999)), "19991231T235959999Z");
        // うるう日・世紀の年（2000 年はうるう年、2100 年は平年）・2038 年問題の境
        assert_eq!(stamp_of(at(951_782_400, 0)), "20000229T000000000Z");
        assert_eq!(stamp_of(at(1_709_210_096, 789)), "20240229T123456789Z");
        assert_eq!(stamp_of(at(4_107_542_399, 1)), "21000228T235959001Z");
        assert_eq!(stamp_of(at(4_107_542_400, 0)), "21000301T000000000Z");
        assert_eq!(stamp_of(at(2_147_483_648, 0)), "20380119T031408000Z");
        // 時計が 1970 年より前でも落ちない
        assert_eq!(
            stamp_of(UNIX_EPOCH - std::time::Duration::from_secs(5)),
            "19700101T000000000Z"
        );
    }
    #[test]
    fn colliding_backup_names_sort_newest_first_like_the_unity_version() {
        let s = Scratch::new();
        for name in [
            "sample-20260101T000000000Z.ylp",
            "sample-20260101T000000000Z_.ylp",
            "sample-20260101T000000000Z__.ylp",
            "sample-20260101T000000001Z.ylp",
            "sample-20251231T235959999Z_.ylp",
        ] {
            touch(&s.backups().join(name), &[1]);
        }
        assert_eq!(
            s.kept_names(),
            [
                "sample-20260101T000000001Z.ylp",
                "sample-20260101T000000000Z__.ylp",
                "sample-20260101T000000000Z_.ylp",
                "sample-20260101T000000000Z.ylp",
                "sample-20251231T235959999Z_.ylp",
            ]
        );
    }
    #[test]
    fn a_new_backup_name_always_sorts_after_the_existing_ones() {
        let s = Scratch::new();
        let dir = s.backups();
        let now = at(1_767_225_600, 0); // 2026-01-01T00:00:00.000Z
        let first = backup_name(&dir, "sample", now).unwrap();
        assert_eq!(first, "sample-20260101T000000000Z.ylp");
        touch(&dir.join(&first), &[1]);
        // 同じミリ秒は下線を足す。名前が空くまで使い回さない
        let second = backup_name(&dir, "sample", now).unwrap();
        assert_eq!(second, "sample-20260101T000000000Z_.ylp");
        touch(&dir.join(&second), &[2]);
        assert_eq!(
            backup_name(&dir, "sample", now).unwrap(),
            "sample-20260101T000000000Z__.ylp"
        );
        fs::remove_file(dir.join(&first)).unwrap(); // 整理で古い方を消しても、空いた名前を使い直さない
        assert_eq!(
            backup_name(&dir, "sample", now).unwrap(),
            "sample-20260101T000000000Z__.ylp"
        );
        // 時計が戻っていても、いちばん新しい版の時刻に下線を足して、新しい版が先に並ぶ
        let back = at(1_767_225_600 - 86_400, 0);
        assert_eq!(
            backup_name(&dir, "sample", back).unwrap(),
            "sample-20260101T000000000Z__.ylp"
        );
        // 進んでいれば、その時刻
        let later = at(1_767_225_600, 1);
        assert_eq!(
            backup_name(&dir, "sample", later).unwrap(),
            "sample-20260101T000000001Z.ylp"
        );
        // 別の文書の退避の名前は数えない
        touch(&dir.join("other-20990101T000000000Z.ylp"), &[3]);
        assert_eq!(
            backup_name(&dir, "sample", later).unwrap(),
            "sample-20260101T000000001Z.ylp"
        );
    }
    #[test]
    fn a_backup_made_while_the_clock_is_behind_still_sorts_as_the_newest() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        // 未来の時刻の退避（時計が戻った・別のマシンで作った）がある
        touch(
            &s.backups().join("sample-29990101T000000000Z.ylp"),
            b"from the future",
        );
        t.save(&changed(&p, "時計が戻っている")).unwrap();
        let names = s.kept_names();
        assert_eq!(names.len(), 2);
        assert_eq!(
            names[0], "sample-29990101T000000000Z_.ylp",
            "今回の退避が先頭"
        );
        assert_eq!(fs::read(s.backups().join(&names[0])).unwrap(), ORIGINAL);
        // 整理しても、今回の退避を消さずに古い方を消す
        t.save_with(&changed(&p, "もう一度"), BackupKeep::Count(1))
            .unwrap();
        assert_eq!(s.kept_names(), ["sample-29990101T000000000Z__.ylp"]);
    }
    #[test]
    fn a_backup_tells_which_file_it_is_a_backup_of_and_nothing_else_does() {
        let s = Scratch::new();
        // 実際の保存が作った退避は、元のファイルを指す（大文字の拡張子も）
        for file in [s.file(), s.0.join("Doc.YLP")] {
            let mut t = SaveTarget::create(&file).unwrap();
            let p = project();
            t.save(&p).unwrap();
            t.save(&changed(&p, "二つ目")).unwrap();
            let listed = backups(&file).unwrap();
            assert_eq!(listed.len(), 1, "{file:?}");
            assert_eq!(backup_origin(&listed[0]), Some(file.clone()));
        }
        let at = |folder: &str, name: &str| backup_origin(&s.0.join(folder).join(name));
        let stamped = "sample-20260101T000000000Z.ylp";
        let legacy = format!("{}.ylp", "ab".repeat(32));
        // 時刻の名前・重なったときの下線つきの名前・以前の版の SHA-256 の名前
        assert_eq!(at("sample.ylp-backups~", stamped), Some(s.file()));
        assert_eq!(
            at("sample.ylp-backups~", "sample-20260101T000000000Z__.ylp"),
            Some(s.file())
        );
        assert_eq!(at("sample.ylp-backups~", &legacy), Some(s.file()));
        // 利用者が名前を変えて置いたもの・別のファイルの退避の名前・退避のフォルダーでない所・拡張子の無い元の名前は退避ではない
        assert_eq!(at("sample.ylp-backups~", "sample-keep.ylp"), None);
        assert_eq!(
            at("sample.ylp-backups~", "other-20260101T000000000Z.ylp"),
            None
        );
        assert_eq!(
            at("sample.ylp-backups~", "sample-20260101T000000000Z.png"),
            None
        );
        assert_eq!(at("sample.ylp-backups", stamped), None);
        assert_eq!(
            at("sample-backups~", "sample-20260101T000000000Z.ylp"),
            None
        );
        assert_eq!(at("elsewhere", stamped), None);
        assert_eq!(backup_origin(&s.0.join(stamped)), None);
        assert_eq!(backup_origin(Path::new("x.ylp")), None);
    }
    #[test]
    fn an_uppercase_extension_is_accepted_and_its_backups_are_listed() {
        let s = Scratch::new();
        let upper = s.0.join("Doc.YLP");
        let mut t = SaveTarget::create(&upper).unwrap();
        let p = project();
        t.save(&p).unwrap();
        t.save(&changed(&p, "二つ目")).unwrap();
        let listed = backups(&upper).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].parent().unwrap(), backup_folder(&upper));
        assert!(listed[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("Doc-"));
        assert_eq!(backup_folder(&upper), s.0.join("Doc.YLP-backups~"));
    }

    // ───────────── 新規の保存先を置き換えない ─────────────

    #[test]
    fn a_new_target_created_between_the_last_check_and_the_move_is_not_overwritten() {
        // 最後の確かめの後、移動の直前に外から作られる。確かめてから移すのではなく、移す操作が断る
        let s = Scratch::new();
        let mut t = SaveTarget::create(s.file()).unwrap();
        let error = t
            .save_moving(
                &project(),
                |from, to| {
                    fs::write(to, b"external").unwrap();
                    move_without_replacing(from, to)
                },
                |_| Ok(()),
            )
            .unwrap_err();
        assert!(
            matches!(&error, Error::SaveConflict(why) if why.contains("外部で作られました")),
            "{error:?}"
        );
        // 外のファイルをそのまま残し、一時ファイルもロックも残さず、印も付けない
        assert_eq!(fs::read(s.file()).unwrap(), b"external");
        assert_eq!(s.names(), ["sample.ylp"]);
        assert!(t.stamp().is_none());
        // 外の物が無くなれば、同じ保存先へ保存できる
        fs::remove_file(s.file()).unwrap();
        t.save(&project()).unwrap();
        assert_bytes(
            &fs::read(s.file()).unwrap(),
            &project().to_bytes().unwrap(),
            "保存先",
        );
    }
    #[test]
    fn a_folder_or_a_dangling_link_created_in_that_gap_is_not_replaced_either() {
        let s = Scratch::new();
        let mut t = SaveTarget::create(s.file()).unwrap();
        let error = t.save_moving(
            &project(),
            |from, to| {
                fs::create_dir(to).unwrap();
                fs::write(to.join("inside.txt"), b"mine").unwrap();
                move_without_replacing(from, to)
            },
            |_| Ok(()),
        );
        assert!(matches!(error, Err(Error::SaveConflict(_))), "{error:?}");
        assert_eq!(fs::read(s.file().join("inside.txt")).unwrap(), b"mine");
        assert!(s.leftovers().is_empty());
        #[cfg(unix)]
        {
            let s = Scratch::new();
            let mut t = SaveTarget::create(s.file()).unwrap();
            let error = t.save_moving(
                &project(),
                |from, to| {
                    std::os::unix::fs::symlink(s.0.join("missing.ylp"), to).unwrap();
                    move_without_replacing(from, to)
                },
                |_| Ok(()),
            );
            assert!(matches!(error, Err(Error::SaveConflict(_))), "{error:?}");
            assert!(fs::symlink_metadata(s.file())
                .unwrap()
                .file_type()
                .is_symlink());
            assert!(!s.0.join("missing.ylp").exists(), "リンクの先に書かない");
            assert!(s.leftovers().is_empty());
        }
    }
    #[test]
    fn the_move_that_does_not_replace_moves_when_free_and_refuses_when_taken() {
        let s = Scratch::new();
        let (from, to) = (s.0.join("from.bin"), s.0.join("to.bin"));
        fs::write(&from, b"new").unwrap();
        assert_eq!(move_without_replacing(&from, &to).unwrap(), Moved::Done);
        assert!(!from.exists());
        assert_eq!(fs::read(&to).unwrap(), b"new");
        // 先がある: ファイル・フォルダー。何も変えない
        fs::write(&from, b"newer").unwrap();
        assert_eq!(move_without_replacing(&from, &to).unwrap(), Moved::Occupied);
        assert_eq!(
            (fs::read(&from).unwrap(), fs::read(&to).unwrap()),
            (b"newer".to_vec(), b"new".to_vec())
        );
        let dir = s.0.join("dir.bin");
        fs::create_dir(&dir).unwrap();
        assert_eq!(
            move_without_replacing(&from, &dir).unwrap(),
            Moved::Occupied
        );
        assert!(dir.is_dir() && from.exists());
        // 元が無い（本当の失敗）は、占有とも未対応とも言わず失敗として返す
        assert!(move_without_replacing(&s.0.join("missing"), &s.0.join("free")).is_err());
    }
    #[cfg(all(unix, target_os = "linux", target_env = "gnu"))]
    #[test]
    fn renameat2_refuses_a_taken_name_with_already_exists_and_moves_when_free() {
        // 1 回の操作が断る（確かめてから移さない）ことの確認。この試験の置き場がフラグを知らないファイルシステムなら、
        // 本番はリンクに落ちる（次の試験）ので、ここは見送る
        let s = Scratch::new();
        let (from, to) = (s.0.join("from.bin"), s.0.join("to.bin"));
        fs::write(&from, b"new").unwrap();
        fs::write(&to, b"old").unwrap();
        match rename_noreplace(&from, &to) {
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::Unsupported | io::ErrorKind::InvalidInput
                ) =>
            {
                eprintln!("renameat2 の置き換えない移動は、この置き場では使えないので見送る: {e}");
                return;
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            (fs::read(&from).unwrap(), fs::read(&to).unwrap()),
            (b"new".to_vec(), b"old".to_vec())
        );
        fs::remove_file(&to).unwrap();
        rename_noreplace(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(fs::read(&to).unwrap(), b"new");
        // NUL を含む名前は、C の文字列にできないので断る
        let nul = Path::new(std::ffi::OsStr::from_bytes(b"a\0b"));
        assert_eq!(
            rename_noreplace(nul, &to).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
    #[cfg(unix)]
    #[test]
    fn the_hard_link_way_moves_when_free_and_refuses_when_taken() {
        // renameat2 の使えないファイルシステム・Linux 以外の Unix が通る道
        let s = Scratch::new();
        let (from, to) = (s.0.join("from.bin"), s.0.join("to.bin"));
        fs::write(&from, b"new").unwrap();
        assert_eq!(link_then_unlink(&from, &to).unwrap(), Moved::Done);
        assert!(!from.exists(), "元の名前は消える");
        assert_eq!(fs::read(&to).unwrap(), b"new");
        fs::write(&from, b"newer").unwrap();
        assert_eq!(link_then_unlink(&from, &to).unwrap(), Moved::Occupied);
        assert_eq!(
            (fs::read(&from).unwrap(), fs::read(&to).unwrap()),
            (b"newer".to_vec(), b"new".to_vec())
        );
        std::os::unix::fs::symlink(s.0.join("missing"), s.0.join("dangling")).unwrap();
        assert_eq!(
            link_then_unlink(&from, &s.0.join("dangling")).unwrap(),
            Moved::Occupied
        );
        // 元が無いのは本当の失敗（未対応として呼び出し側に任せない）
        assert_eq!(
            link_then_unlink(&s.0.join("missing"), &s.0.join("free"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
    }
    #[cfg(unix)]
    #[test]
    fn the_hard_link_way_returns_a_missing_source_as_a_failure_and_other_refusals_as_unsupported() {
        let s = Scratch::new();
        let mut removed = 0;
        let mut remove = |_: &Path| {
            removed += 1;
            Ok(())
        };
        // 元が無い: 失敗。何も作らず、元の名前を消しにも行かない
        let error = link_then_unlink_with(&s.0.join("missing"), &s.0.join("free"), &mut remove)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(!s.0.join("free").exists());
        // 移動先のフォルダーが無いのも、本当の失敗
        let (from, to) = (s.0.join("from.bin"), s.0.join("none").join("to.bin"));
        fs::write(&from, b"new").unwrap();
        let error = link_then_unlink_with(&from, &to, &mut remove).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(fs::read(&from).unwrap(), b"new", "元は変えない");
        // ほかの断り（フォルダーへのリンクは作れない）は、今までどおり未対応
        let dir = s.0.join("dir");
        fs::create_dir(&dir).unwrap();
        assert_eq!(
            link_then_unlink_with(&dir, &s.0.join("linked"), &mut remove).unwrap(),
            Moved::Unsupported
        );
        assert_eq!(removed, 0);
    }
    #[cfg(unix)]
    #[test]
    fn a_hard_link_move_that_cannot_remove_the_old_name_still_commits_and_leaves_no_hidden_file() {
        // リンクは作れたが元の名前を消せない: 保存先は確定している（Done）ので保存は成功し、保存先と同じ実体を指す隠しファイルは
        // 落とすときの後始末が消す
        let s = Scratch::new();
        let mut t = SaveTarget::create(s.file()).unwrap();
        let mut refused = 0;
        t.save_moving(
            &project(),
            |from, to| {
                link_then_unlink_with(from, to, &mut |path| {
                    assert!(path.exists(), "消そうとした時点で元の名前はまだある");
                    refused += 1;
                    Err(io::Error::from(io::ErrorKind::PermissionDenied))
                })
            },
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(refused, 1);
        assert_eq!(s.names(), ["sample.ylp"], "隠しファイルが残らない");
        assert_bytes(
            &fs::read(s.file()).unwrap(),
            &project().to_bytes().unwrap(),
            "保存先",
        );
        // 単体でも: 消せなくても Done で、リンクは両方の名前に残る（消し直すのは呼ぶ側）
        let (from, to) = (s.0.join("from.bin"), s.0.join("to.bin"));
        fs::write(&from, b"new").unwrap();
        let moved = link_then_unlink_with(&from, &to, &mut |_| {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        });
        assert_eq!(moved.unwrap(), Moved::Done);
        assert_eq!(
            (fs::read(&from).unwrap(), fs::read(&to).unwrap()),
            (b"new".to_vec(), b"new".to_vec())
        );
    }
    #[test]
    fn a_file_system_that_cannot_move_without_replacing_still_saves_and_still_checks() {
        // 置き換えない移動の使えない場所でも、断らずに保存できる（確かめ直してから置き換える移動）
        let s = Scratch::new();
        let mut t = SaveTarget::create(s.file()).unwrap();
        t.save_moving(&project(), |_, _| Ok(Moved::Unsupported), |_| Ok(()))
            .unwrap();
        assert_eq!(s.names(), ["sample.ylp"]);
        assert_bytes(
            &fs::read(s.file()).unwrap(),
            &project().to_bytes().unwrap(),
            "保存先",
        );
        // 確かめ直しは働く: 未対応と分かった後に外から作られていても上書きしない
        let s = Scratch::new();
        let mut t = SaveTarget::create(s.file()).unwrap();
        let error = t.save_moving(
            &project(),
            |_, to| {
                fs::write(to, b"external").unwrap();
                Ok(Moved::Unsupported)
            },
            |_| Ok(()),
        );
        assert!(matches!(error, Err(Error::SaveConflict(_))), "{error:?}");
        assert_eq!(fs::read(s.file()).unwrap(), b"external");
        assert!(s.leftovers().is_empty());
        // 移動そのものの本当の失敗は失敗として返し、何も残さない
        let s = Scratch::new();
        let mut t = SaveTarget::create(s.file()).unwrap();
        let error = t.save_moving(
            &project(),
            |_, _| Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            |_| Ok(()),
        );
        assert!(
            matches!(&error, Err(Error::Io(e)) if e.kind() == io::ErrorKind::PermissionDenied),
            "{error:?}"
        );
        assert!(s.names().is_empty(), "{:?}", s.names());
    }
    /// 保証の射程: 同じ新規の保存先へ同時に着いた保存のうち、勝つのは 1 つで中身は無傷（実際の保存はロックで直列になり、負けた側は
    /// 進行中か保存先の占有で断られる）。最後の確かめから置き換えない移動までの隙間に作られた保存先を上書きしないことは、
    /// この試験では見えず、`a_new_target_created_between_the_last_check_and_the_move_is_not_overwritten` が担う。
    #[test]
    fn several_savers_racing_for_one_new_target_leave_exactly_one_winner_intact() {
        const SAVERS: usize = 6;
        let s = Scratch::new();
        let base = project();
        // 保存先を取る（`create`）は全員ぶん先に済ませる。先に勝った保存が終わってから着いた側が「保存先が既にあります」で
        // create の時点で落ちると、競争ではなくなる。全員が同じ「まだ無い」保存先を持って、Barrier で揃えてから保存を始める
        let mut targets: Vec<(SaveTarget, Project)> = (0..SAVERS)
            .map(|i| {
                (
                    SaveTarget::create(s.file()).unwrap(),
                    changed(&base, &format!("競争 {i}")),
                )
            })
            .collect();
        let gate = std::sync::Barrier::new(SAVERS);
        let results: Vec<(Result<FileStamp>, Vec<u8>)> = std::thread::scope(|scope| {
            let handles: Vec<_> = targets
                .iter_mut()
                .map(|(target, mine)| {
                    let gate = &gate;
                    scope.spawn(move || {
                        gate.wait();
                        (target.save(mine), mine.to_bytes().unwrap())
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let winners: Vec<_> = results.iter().filter(|(r, _)| r.is_ok()).collect();
        assert_eq!(
            winners.len(),
            1,
            "{:?}",
            results.iter().map(|(r, _)| r.is_ok()).collect::<Vec<_>>()
        );
        // 負けた側は外部の変更・進行中として断られ、勝った側の中身がそのまま残る
        for (r, _) in &results {
            if let Err(e) = r {
                assert!(matches!(e, Error::SaveConflict(_)), "{e:?}");
            }
        }
        assert_bytes(&fs::read(s.file()).unwrap(), &winners[0].1, "保存先");
        assert_eq!(s.names(), ["sample.ylp"]);
    }
    #[test]
    fn overwriting_an_existing_target_still_replaces() {
        // 置き換えない移動は、新規の保存先と退避の最終の名前だけ。開いた保存先への上書きは、印を確かめた上で置き換える
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let next = changed(&p, "上書き");
        let backups = s.backups();
        t.save_moving(
            &next,
            |from, to| {
                assert_eq!(
                    to.parent(),
                    Some(backups.as_path()),
                    "上書きは置き換えない移動を通らない: {to:?}"
                );
                move_without_replacing(from, to)
            },
            |_| Ok(()),
        )
        .unwrap();
        assert_bytes(
            &fs::read(s.file()).unwrap(),
            &next.to_bytes().unwrap(),
            "保存先",
        );
    }
    #[cfg(windows)]
    #[test]
    fn a_new_target_with_a_path_longer_than_max_path_is_saved() {
        let s = Scratch::new();
        let mut dir = s.0.clone();
        for i in 0..12 {
            dir.push(format!("deep-folder-name-{i:02}"));
        }
        assert!(dir.as_os_str().len() > 260);
        let target = dir.join("sample.ylp");
        let mut t = SaveTarget::create(&target).unwrap();
        t.save(&project()).unwrap();
        assert_bytes(
            &fs::read(&target).unwrap(),
            &project().to_bytes().unwrap(),
            "保存先",
        );
        // 長い名前でも、先にあれば置き換えずに断る
        let mut again = SaveTarget::create(dir.join("other.ylp")).unwrap();
        let error = again.save_moving(
            &project(),
            |from, to| {
                fs::write(to, b"external").unwrap();
                move_without_replacing(from, to)
            },
            |_| Ok(()),
        );
        assert!(matches!(error, Err(Error::SaveConflict(_))), "{error:?}");
        assert_eq!(fs::read(dir.join("other.ylp")).unwrap(), b"external");
    }

    // ───────────── 拡張子とフォルダー ─────────────

    #[test]
    fn a_name_that_does_not_end_in_ylp_is_refused_before_touching_disk() {
        let s = Scratch::new();
        for bad in [
            "doc.txt",
            "doc.ylp.bak",
            "doc",
            "doc.ylp~",
            "doc.yl",
            ".ylp",
            "doc.ylpx",
            "doc.ylp ",
        ] {
            let path = s.0.join("sub").join(bad);
            let error = SaveTarget::create(&path).unwrap_err();
            assert!(
                matches!(&error, Error::SaveConflict(why) if why.contains("名前が .ylp で終わっていません")),
                "{bad}: {error:?}"
            );
        }
        assert!(SaveTarget::create(Path::new("")).is_err());
        assert!(!s.0.join("sub").exists(), "フォルダーも作らない");
        assert!(s.names().is_empty());
        // 大文字小文字は問わない
        for ok in ["a.ylp", "a.YLP", "a.Ylp", "日本語の名前.ylp", "a.b.ylp"] {
            SaveTarget::create(s.0.join(ok)).unwrap();
        }
    }
    #[test]
    fn opening_a_file_with_another_name_is_allowed_but_saving_it_is_refused() {
        // C# の Load は名前を見ない。保存だけが名前を確かめる
        let s = Scratch::new();
        let path = s.0.join("renamed.bin");
        fs::write(&path, ORIGINAL).unwrap();
        let (p, mut t) = SaveTarget::open(&path).unwrap();
        let error = t.save(&p).unwrap_err();
        assert!(
            matches!(&error, Error::SaveConflict(why) if why.contains(".ylp")),
            "{error:?}"
        );
        assert_eq!(fs::read(&path).unwrap(), ORIGINAL);
        assert_eq!(s.names(), ["renamed.bin"]);
    }
    #[test]
    fn a_missing_folder_is_created_for_a_new_target() {
        let s = Scratch::new();
        let nested = s.0.join("a").join("b").join("doc.ylp");
        let mut t = SaveTarget::create(&nested).unwrap();
        assert!(!s.0.join("a").exists(), "create はまだ作らない");
        t.save(&project()).unwrap();
        assert_bytes(
            &fs::read(&nested).unwrap(),
            &project().to_bytes().unwrap(),
            "保存先",
        );
        assert_eq!(
            fs::read_dir(nested.parent().unwrap()).unwrap().count(),
            1,
            "ロックも一時ファイルも残らない"
        );
        // 開き直せて、上書きでは退避する
        let (p, mut t) = SaveTarget::open(&nested).unwrap();
        t.save(&changed(&p, "入れ子の上書き")).unwrap();
        assert_eq!(backups(&nested).unwrap().len(), 1);
    }
    #[test]
    fn a_failed_new_save_removes_the_folders_it_created_but_nothing_else() {
        for point in ["flushed", "disk-verified", "before-replace", "backed-up"] {
            let s = Scratch::new();
            fs::create_dir(s.0.join("a")).unwrap();
            let target = s.0.join("a").join("b").join("c").join("doc.ylp");
            let mut t = SaveTarget::create(&target).unwrap();
            assert!(t.save_inner(&project(), fail_at(point)).is_err(), "{point}");
            // もとからあった a は残し、作った b・c は消える
            assert_eq!(s.names(), ["a"], "{point}");
            assert_eq!(fs::read_dir(s.0.join("a")).unwrap().count(), 0, "{point}");
            // そのまま保存できる
            t.save(&project()).unwrap();
            assert!(target.exists(), "{point}");
        }
    }
    #[test]
    fn invalid_content_is_refused_before_any_folder_is_made() {
        let s = Scratch::new();
        let target = s.0.join("never").join("deeper").join("doc.ylp");
        let mut t = SaveTarget::create(&target).unwrap();
        assert!(t
            .save_inner(&project(), fail_at("memory-verified"))
            .is_err());
        assert!(!s.0.join("never").exists());
        assert!(s.names().is_empty());
    }
    #[test]
    fn a_folder_that_got_something_inside_is_not_removed_after_a_failure() {
        let s = Scratch::new();
        let dir = s.0.join("a");
        let target = dir.join("doc.ylp");
        let mut t = SaveTarget::create(&target).unwrap();
        let error = t.save_inner(&project(), |phase| {
            if phase == "flushed" {
                fs::write(dir.join("mine.txt"), b"added meanwhile")?;
                return Err(Error::InvalidData("止める".into()));
            }
            Ok(())
        });
        assert!(error.is_err());
        // 中に他のものが入ったフォルダーは消さない。保存が置いたロックと一時ファイルは消える
        assert_eq!(
            fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            ["mine.txt"]
        );
    }
    #[test]
    fn a_parent_that_is_a_file_fails_and_leaves_nothing() {
        let s = Scratch::new();
        fs::write(s.0.join("blocker"), b"a file").unwrap();
        let target = s.0.join("blocker").join("sub").join("doc.ylp");
        // 親が普通のファイルでは作れない。create は有無を確かめるだけなので通り、保存が理由を返す
        let mut t = match SaveTarget::create(&target) {
            Ok(t) => t,
            Err(e) => {
                assert!(matches!(e, Error::Io(_)), "{e:?}");
                return;
            }
        };
        let error = t.save(&project()).unwrap_err();
        assert!(matches!(error, Error::Io(_)), "{error:?}");
        assert_eq!(s.names(), ["blocker"]);
        assert_eq!(fs::read(s.0.join("blocker")).unwrap(), b"a file");
    }
    #[test]
    fn the_folder_of_an_opened_target_removed_outside_is_an_outside_change() {
        let s = Scratch::new();
        let dir = s.0.join("project");
        fs::create_dir(&dir).unwrap();
        let path = dir.join("sample.ylp");
        fs::write(&path, ORIGINAL).unwrap();
        let (p, mut t) = SaveTarget::open(&path).unwrap();
        fs::remove_dir_all(&dir).unwrap();
        let error = t.save(&p).unwrap_err();
        // 開いたファイルの場所は作り直さない（外で消されたものを、無かったことにして書き始めない）
        assert!(
            matches!(&error, Error::SaveConflict(why) if why.contains("外部で消されています")),
            "{error:?}"
        );
        assert!(!dir.exists());
    }

    // ───────── 強制終了された保存の残骸・Windows の置換の再試行 ─────────

    /// 共有違反を模す失敗（本物の判定は Windows の OS のエラー番号。ここでは種類で見分ける）。
    fn pretend_busy() -> io::Error {
        io::Error::from(io::ErrorKind::PermissionDenied)
    }
    fn is_pretend_busy(e: &io::Error) -> bool {
        e.kind() == io::ErrorKind::PermissionDenied
    }
    #[test]
    fn retrying_waits_in_growing_steps_and_never_beyond_the_budget() {
        // 4 回失敗してから通る: 待ちは 10・20・40・80 ms
        let mut slept = Vec::new();
        let mut left = 4;
        let done = retry_busy(&is_pretend_busy, &mut |d| slept.push(d), || {
            if left > 0 {
                left -= 1;
                Err(pretend_busy())
            } else {
                Ok("通った")
            }
        });
        assert_eq!(done.unwrap(), "通った");
        assert_eq!(slept, [10, 20, 40, 80].map(Duration::from_millis));
        // ずっと失敗: 待ちの合計は予算ちょうどで、その失敗を返す（待ちの 1 回ごとは 300 ms まで）
        let (mut slept, mut calls) = (Vec::new(), 0);
        let error = retry_busy::<()>(&is_pretend_busy, &mut |d| slept.push(d), || {
            calls += 1;
            Err(pretend_busy())
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(slept.iter().sum::<Duration>(), BUSY_BUDGET);
        assert!(
            slept.iter().all(|d| *d <= Duration::from_millis(300)),
            "{slept:?}"
        );
        assert_eq!(calls, slept.len() + 1);
        // 別の失敗は待たずに返す
        let (mut slept, mut calls) = (Vec::new(), 0);
        let error = retry_busy::<()>(&is_pretend_busy, &mut |d| slept.push(d), || {
            calls += 1;
            Err(io::Error::from(io::ErrorKind::NotFound))
        })
        .unwrap_err();
        assert_eq!(
            (error.kind(), slept.len(), calls),
            (io::ErrorKind::NotFound, 0, 1)
        );
        // 本物の判定は Windows の共有違反・ロック違反・アクセス拒否だけ（Unix の置換は開いているファイルに妨げられない）
        let os = |code| io::Error::from_raw_os_error(code);
        assert_eq!(
            [os(5), os(32), os(33), os(2)].map(|e| is_busy(&e)),
            [cfg!(windows), cfg!(windows), cfg!(windows), false]
        );
    }
    #[test]
    fn a_busy_replace_is_retried_until_it_goes_through() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let next = changed(&p, "共有違反のあと");
        let (mut busy_left, mut slept) = (3, Vec::new());
        let stamp = t
            .save_replacing(
                &next,
                |from, to| {
                    if busy_left > 0 {
                        busy_left -= 1;
                        return Err(pretend_busy());
                    }
                    fs::rename(from, to)
                },
                is_pretend_busy,
                |d| slept.push(d),
            )
            .unwrap();
        assert_eq!(slept, [10, 20, 40].map(Duration::from_millis));
        assert_eq!(read_stamp(&s.file()).unwrap(), stamp);
        assert!(s.leftovers().is_empty(), "{:?}", s.names());
        assert_eq!(s.kept(), [ORIGINAL.to_vec()]);
    }
    #[test]
    fn a_replace_that_stays_busy_fails_cleanly_and_keeps_the_original() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let before = t.stamp().unwrap().clone();
        let mut slept = Vec::new();
        let error = t
            .save_replacing(
                &changed(&p, "通らない"),
                |_, _| Err(pretend_busy()),
                is_pretend_busy,
                |d| slept.push(d),
            )
            .unwrap_err();
        assert!(
            matches!(&error, Error::Io(e) if is_pretend_busy(e)),
            "{error:?}"
        );
        assert_eq!(slept.iter().sum::<Duration>(), BUSY_BUDGET);
        assert_bytes(&fs::read(s.file()).unwrap(), ORIGINAL, "保存先");
        assert_eq!(
            t.stamp().unwrap(),
            &before,
            "印は変えない（次の保存がそのまま通る）"
        );
        assert!(s.leftovers().is_empty(), "{:?}", s.names());
        assert!(
            !s.backups().exists(),
            "置換に至らなかった保存は退避を残さない"
        );
        t.save(&changed(&p, "次の保存")).unwrap();
    }
    /// ファイルの位置で持つ形で開いたとき（`open_by_handle`）の閾値。保存のあとのプロジェクトも、同じ形で持たせる。
    const BY_HANDLE: Thresholds = Thresholds {
        keep_in_memory: 0,
        ..Thresholds::REAL
    };
    /// 開いたエントリをファイルの位置で持つ形（メモリに残さない）で開く。
    fn open_by_handle(s: &Scratch) -> (Project, SaveTarget, String) {
        fs::write(s.file(), ORIGINAL).unwrap();
        let (p, t) = BY_HANDLE.scoped(|| SaveTarget::open(s.file()).unwrap());
        let name = p
            .original_archive()
            .entries()
            .iter()
            .find(|(_, b)| !b.is_empty() && b.in_memory().is_none())
            .map(|(n, _)| n.clone())
            .expect("ファイルの位置で持つエントリ");
        (p, t, name)
    }
    #[test]
    fn our_own_handles_are_let_go_only_after_a_wait_and_taken_again_when_the_replace_fails() {
        // 置き換えの規則が POSIX でないファイルシステム（FAT など）は、開いているファイルを置き換えられない: 手放さずに待っても通らない
        // ので、`RELEASE_AFTER` 待ったところで自分のハンドルを手放し、残りの待ちでやり直す。置き換えられたら、古い写しは読もうとすると
        // 断る（保存した後のプロジェクトは新しいファイルを読める）
        BY_HANDLE.scoped(|| {
            let s = Scratch::new();
            let (p, mut t, name) = open_by_handle(&s);
            let blob = p.original_archive().entries()[&name].clone();
            let want = blob.bytes().unwrap();
            let mut slept = Vec::new();
            let report = t
                .save_replacing_report(
                    &changed(&p, "手放す"),
                    |from, to| {
                        // 自分のハンドルが開いている間は、置き換えられない
                        if blob.bytes().is_ok() {
                            return Err(pretend_busy());
                        }
                        fs::rename(from, to)
                    },
                    is_pretend_busy,
                    |d| slept.push(d),
                )
                .unwrap();
            assert_eq!(
                slept.iter().sum::<Duration>(),
                RELEASE_AFTER,
                "手放す前に、手放さずに待った合計"
            );
            let refused = blob.bytes().unwrap_err();
            assert!(
                refused.to_string().contains(crate::SOURCE_RELEASED),
                "{refused:?}"
            );
            let saved = report.project.unwrap();
            let from_new = saved.original_archive().entries()[&name].bytes().unwrap();
            assert_eq!(
                from_new, want,
                "保存した後のプロジェクトは新しいファイルから同じ中身を読む"
            );
            // 置き換えられなかったときは、全部の待ちを使い切って、掴み直して今までどおり読める
            let s = Scratch::new();
            let (p, mut t, name) = open_by_handle(&s);
            let blob = p.original_archive().entries()[&name].clone();
            let want = blob.bytes().unwrap();
            let mut slept = Vec::new();
            let error = t.save_replacing(
                &changed(&p, "通らない"),
                |_, _| Err(pretend_busy()),
                is_pretend_busy,
                |d| slept.push(d),
            );
            assert!(error.is_err());
            assert_eq!(slept.iter().sum::<Duration>(), BUSY_BUDGET);
            assert_eq!(blob.bytes().unwrap(), want, "掴み直して読める");
            t.save(&changed(&p, "次の保存")).unwrap();
        });
    }
    #[test]
    fn a_replace_that_goes_through_within_the_wait_keeps_our_handles() {
        // NTFS で外のツール（ウイルス対策・同期・Unity の取り込み）が一時的に掴んでいるだけなら、自分のハンドルは原因ではない: 手放さずに
        // 待って通り、保存前のプロジェクトの写しも読めたまま
        BY_HANDLE.scoped(|| {
            let s = Scratch::new();
            let (p, mut t, name) = open_by_handle(&s);
            let blob = p.original_archive().entries()[&name].clone();
            let want = blob.bytes().unwrap();
            let (mut busy_left, mut slept, mut readable) = (3, Vec::new(), Vec::new());
            let report = t
                .save_replacing_report(
                    &changed(&p, "外のツールが掴んでいた"),
                    |from, to| {
                        readable.push(blob.bytes().is_ok());
                        if busy_left > 0 {
                            busy_left -= 1;
                            return Err(pretend_busy());
                        }
                        fs::rename(from, to)
                    },
                    is_pretend_busy,
                    |d| slept.push(d),
                )
                .unwrap();
            assert_eq!(slept, [10, 20, 40].map(Duration::from_millis));
            assert_eq!(readable, [true; 4], "どの試みでも手放していない");
            assert_eq!(blob.bytes().unwrap(), want, "保存前の写しも読める");
            let saved = report.project.unwrap();
            assert_eq!(
                saved.original_archive().entries()[&name].bytes().unwrap(),
                want
            );
        });
    }
    #[test]
    fn the_project_a_save_returns_is_found_again_by_the_next_busy_replace() {
        // 保存後のプロジェクトのハンドルは、置き場の名前を保存先へ付け替えてある（`Project::moved_to`）。付け替えが落ちると、次の保存で、
        // 置換の邪魔になっているこのハンドルを手放す相手として見つけられず、FAT・exFAT・SMB では 2 回目以降の上書き保存が待ちを使い切って
        // 必ず失敗する（掴み直しも、もう無い一時ファイルの名前を開くので掴めない）
        BY_HANDLE.scoped(|| {
            for first_busy in [false, true] {
                let s = Scratch::new();
                let (p, mut t, name) = open_by_handle(&s);
                let before = p.original_archive().entries()[&name].clone();
                let want = before.bytes().unwrap();
                // 1 回目の保存（普通の保存と、最初の置換が共有違反で手放した保存）
                let first = if first_busy {
                    t.save_replacing_report(
                        &changed(&p, "1 回目"),
                        |from, to| if before.bytes().is_ok() { Err(pretend_busy()) } else { fs::rename(from, to) },
                        is_pretend_busy,
                        |_| {},
                    )
                } else {
                    t.save_with(&changed(&p, "1 回目"), BackupKeep::All)
                }
                .unwrap();
                let saved = first.project.unwrap();
                let entry = saved.original_archive().entries()[&name].clone();
                assert!(entry.in_memory().is_none(), "ファイルの位置で持つ");
                assert_eq!(entry.bytes().unwrap(), want, "first_busy={first_busy}");
                // 2 回目の保存: 保存後のハンドルが開いている間は置き換えられない（FAT のように）。見つけて手放せば通る
                let mut released_before_the_replace = false;
                let second = t
                    .save_replacing_report(
                        &changed(&saved, "2 回目"),
                        |from, to| {
                            if entry.bytes().is_ok() {
                                return Err(pretend_busy());
                            }
                            released_before_the_replace = true;
                            fs::rename(from, to)
                        },
                        is_pretend_busy,
                        |_| {},
                    )
                    .expect("保存後のハンドルを手放す相手として見つけられない（置き場の名前の付け替えが落ちている）");
                assert!(released_before_the_replace, "first_busy={first_busy}");
                let refused = entry.bytes().unwrap_err();
                assert!(refused.to_string().contains(crate::SOURCE_RELEASED), "{refused:?}");
                let latest = second.project.unwrap();
                assert_eq!(latest.original_archive().entries()[&name].bytes().unwrap(), want, "first_busy={first_busy}");
            }
        });
    }
    #[test]
    fn a_small_project_does_not_keep_its_file_open_but_a_big_one_does() {
        // 小さなエントリだけの .ylp は、開いたあとファイルを持たない（Windows で外の改名・削除・上書きを妨げない）。大きなエントリが
        // あるときだけ、開いたハンドルを持つ
        let s = Scratch::new();
        let (p, _t) = open_original(&s);
        assert!(
            release_at(&s.file()).is_empty(),
            "小さなエントリだけ: ハンドルを持たない"
        );
        drop(p);
        let s = Scratch::new();
        let (p, _t, _) = open_by_handle(&s);
        let released = release_at(&s.file());
        assert!(
            !released.is_empty(),
            "大きなエントリがあれば、ハンドルを持つ"
        );
        released.reacquire();
        drop(p);
    }
    #[test]
    fn a_busy_move_of_a_new_target_is_retried_and_leaves_nothing() {
        let s = Scratch::new();
        let p = project();
        let mut t = SaveTarget::create(s.file()).unwrap();
        let (mut busy_left, mut slept) = (3, Vec::new());
        t.save_moving_busy(
            &p,
            |from, to| {
                if busy_left > 0 {
                    busy_left -= 1;
                    return Err(pretend_busy());
                }
                move_without_replacing(from, to)
            },
            is_pretend_busy,
            |d| slept.push(d),
        )
        .unwrap();
        assert_eq!(slept, [10, 20, 40].map(Duration::from_millis));
        assert_bytes(
            &fs::read(s.file()).unwrap(),
            &p.to_bytes().unwrap(),
            "保存先",
        );
        assert_eq!(s.names(), ["sample.ylp"], "一時ファイルとロックは残らない");
        // ずっと共有違反: 予算を使い切って、何も作らず失敗する
        let s = Scratch::new();
        let mut t = SaveTarget::create(s.file()).unwrap();
        let mut slept = Vec::new();
        let error = t
            .save_moving_busy(
                &p,
                |_, _| Err(pretend_busy()),
                is_pretend_busy,
                |d| slept.push(d),
            )
            .unwrap_err();
        assert!(
            matches!(&error, Error::Io(e) if is_pretend_busy(e)),
            "{error:?}"
        );
        assert_eq!(slept.iter().sum::<Duration>(), BUSY_BUDGET);
        assert!(s.names().is_empty(), "{:?}", s.names());
    }
    #[test]
    fn a_busy_move_of_a_backup_is_retried_and_a_failure_leaves_no_backup() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let backups = s.backups();
        let (mut busy_left, mut slept) = (3, Vec::new());
        t.save_moving_busy(
            &changed(&p, "退避の移動"),
            |from, to| {
                // 置き換える移動（保存先の上書き）は別の口。ここへ来るのは退避の最終の名前への移動だけ
                assert_eq!(to.parent(), Some(backups.as_path()), "{to:?}");
                if busy_left > 0 {
                    busy_left -= 1;
                    return Err(pretend_busy());
                }
                move_without_replacing(from, to)
            },
            is_pretend_busy,
            |d| slept.push(d),
        )
        .unwrap();
        assert_eq!(slept, [10, 20, 40].map(Duration::from_millis));
        assert_eq!(s.kept(), [ORIGINAL.to_vec()]);
        assert_eq!(
            s.backup_dir_names(),
            s.kept_names(),
            "退避の一時ファイルは残らない"
        );
        assert!(s.leftovers().is_empty(), "{:?}", s.names());
        // ずっと共有違反: 予算を使い切って失敗し、退避（作ったフォルダーごと）も保存先の変更も残さない
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let before = t.stamp().unwrap().clone();
        let mut slept = Vec::new();
        let error = t
            .save_moving_busy(
                &changed(&p, "退避が移せない"),
                |_, _| Err(pretend_busy()),
                is_pretend_busy,
                |d| slept.push(d),
            )
            .unwrap_err();
        assert!(
            matches!(&error, Error::Io(e) if is_pretend_busy(e)),
            "{error:?}"
        );
        assert_eq!(slept.iter().sum::<Duration>(), BUSY_BUDGET);
        assert!(!s.backups().exists(), "{:?}", s.names());
        assert_bytes(&fs::read(s.file()).unwrap(), ORIGINAL, "保存先");
        assert_eq!(t.stamp().unwrap(), &before);
        assert_eq!(s.names(), ["sample.ylp"]);
        t.save(&changed(&p, "次の保存")).unwrap();
    }
    #[test]
    fn leftover_names_are_only_this_targets_own_temporary_files() {
        for (found, ours) in [
            (".sample.ylp.123-4.pending~", true),
            (".sample.ylp.1-0.pending~", true),
            (".sample.ylp.123.pending~", false),
            (".sample.ylp.a-4.pending~", false),
            (".sample.ylp.123-.pending~", false),
            ("sample.ylp.123-4.pending~", false),
            (".other.ylp.123-4.pending~", false),
            (".sample.ylp.123-4.pending", false),
            (".sample.ylp.save.lock~", false),
            (".sample.ylp.notes.pending~", false),
            ("..sample.ylp.123-4.pending~", false),
        ] {
            assert_eq!(is_leftover_name("sample.ylp", found), ours, "{found}");
        }
    }
    /// 前の保存の残骸（保存の一時ファイル・退避の一時ファイル）と、触ってはいけない似た名前のファイルを置く。
    fn plant_leftovers(s: &Scratch) -> (Vec<PathBuf>, Vec<PathBuf>) {
        fs::create_dir_all(s.backups()).unwrap();
        let stale = vec![
            s.0.join(".sample.ylp.4242-1.pending~"),
            s.backups().join(".sample.ylp.4242-2.pending~"),
        ];
        let others = vec![
            s.0.join(".other.ylp.4242-1.pending~"),
            s.0.join(".sample.ylp.notes.pending~"),
            s.0.join("sample.ylp.4242-1.pending~"),
            s.backups().join(".other.ylp.4242-2.pending~"),
        ];
        for p in stale.iter().chain(&others) {
            fs::write(p, b"cut off").unwrap();
        }
        (stale, others)
    }
    #[test]
    fn the_next_save_clears_this_targets_leftovers_and_only_those() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let (stale, others) = plant_leftovers(&s);
        t.save(&changed(&p, "残骸のあと")).unwrap();
        assert!(stale.iter().all(|f| !f.exists()), "{:?}", s.names());
        assert!(
            others.iter().all(|f| f.exists()),
            "別の保存先・別の形・利用者のファイルには触らない"
        );
        assert_eq!(fs::read(&others[0]).unwrap(), b"cut off");
        assert_eq!(s.kept(), [ORIGINAL.to_vec()]);
    }
    #[test]
    fn leftovers_stay_while_someone_else_holds_the_lock_or_locks_are_unavailable() {
        // 別の保存が進行中（ロックを持っている）: 断られ、その保存の一時ファイルかもしれないものには触らない
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let (stale, _) = plant_leftovers(&s);
        let _held = hold(&s.lock());
        assert!(matches!(
            t.save(&changed(&p, "断られる")),
            Err(Error::SaveConflict(_))
        ));
        assert!(stale.iter().all(|f| f.exists()));
        drop(_held);
        // ロックの使えないファイルシステム: 誰も保存していないと確かめられないので、触らない（保存は通る）
        t.save_locking(
            &changed(&p, "ロックの無い場所"),
            unsupported_lock,
            |_| Ok(()),
        )
        .unwrap();
        assert!(stale.iter().all(|f| f.exists()), "{:?}", s.names());
        // ロックが取れれば片付く
        t.save(&changed(&p, "取れる")).unwrap();
        assert!(stale.iter().all(|f| !f.exists()), "{:?}", s.names());
    }
    #[cfg(unix)]
    #[test]
    fn a_symlinked_backup_folder_is_never_swept() {
        // リンクの先は利用者のフォルダー: 保存は断り、退避しない設定で保存しても、中の一時ファイルの形のものを消さない
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let elsewhere = s.0.join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        let theirs = elsewhere.join(".sample.ylp.4242-2.pending~");
        fs::write(&theirs, b"someone else's").unwrap();
        std::os::unix::fs::symlink(&elsewhere, s.backups()).unwrap();
        assert!(matches!(
            t.save(&changed(&p, "断られる")),
            Err(Error::SaveConflict(_))
        ));
        assert_eq!(fs::read(&theirs).unwrap(), b"someone else's");
        t.save_with(&changed(&p, "退避しない"), BackupKeep::Count(0))
            .unwrap();
        assert_eq!(fs::read(&theirs).unwrap(), b"someone else's");
        // 確かめたあとでリンクに替えられても、見直して触らない
        sweep_backup_leftovers(&s.backups(), "sample.ylp");
        assert_eq!(fs::read(&theirs).unwrap(), b"someone else's");
        assert!(fs::symlink_metadata(s.backups())
            .unwrap()
            .file_type()
            .is_symlink());
    }
    #[test]
    fn a_save_that_makes_no_backup_leaves_the_backup_place_alone() {
        // 退避しない設定では置き場に触らない: 一時ファイルの形の残りも、退避を作る保存が来るまで置いたまま（保存先の隣の残りは片付く）
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let (stale, others) = plant_leftovers(&s);
        let in_place = fs::read_dir(s.backups()).unwrap().count();
        t.save_with(&changed(&p, "退避しない"), BackupKeep::Count(0))
            .unwrap();
        assert!(!stale[0].exists(), "保存先の隣の残りは片付く");
        assert!(
            stale[1].exists() && others.iter().all(|f| f.exists()),
            "{:?}",
            s.backup_dir_names()
        );
        assert_eq!(fs::read_dir(s.backups()).unwrap().count(), in_place);
        // 退避を作る保存になれば片付く
        t.save(&changed(&p, "退避する")).unwrap();
        assert!(!stale[1].exists(), "{:?}", s.backup_dir_names());
        assert!(others.iter().all(|f| f.exists()));
        // 新規の保存先には前の版が無く、置き場には触らない
        let s = Scratch::new();
        fs::create_dir(s.backups()).unwrap();
        let theirs = s.backups().join(".sample.ylp.4242-2.pending~");
        fs::write(&theirs, b"cut off").unwrap();
        SaveTarget::create(s.file())
            .unwrap()
            .save(&project())
            .unwrap();
        assert!(theirs.exists());
    }
    #[test]
    fn a_backup_is_written_under_a_temporary_name_and_only_the_finished_one_counts() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let mut at_copied = None;
        t.save_inner(&changed(&p, "退避の途中"), |phase| {
            if phase == "backup-copied" {
                at_copied = Some((s.backup_dir_names(), s.kept_names()));
            }
            Ok(())
        })
        .unwrap();
        // 写し終えた時点では、一時の名前（隠れる名前）だけがあり、退避としては数えられない
        let (names, kept) = at_copied.expect("通った");
        assert_eq!(names.len(), 1, "{names:?}");
        assert!(
            names[0].starts_with(".sample.ylp.") && names[0].ends_with(".pending~"),
            "{names:?}"
        );
        assert!(kept.is_empty(), "{kept:?}");
        // 終わると、最終の名前の退避が 1 つだけ（一時の名前は残らない）
        assert_eq!(s.kept_names().len(), 1);
        assert_eq!(s.backup_dir_names(), s.kept_names());
        assert_eq!(s.kept(), [ORIGINAL.to_vec()]);
    }

    const ABORT_AT: &str = "YOLU_STORE_ABORT_AT";
    const ABORT_DIR: &str = "YOLU_STORE_ABORT_DIR";
    /// 子プロセスの本体（次の試験が起こす。環境変数が無ければ何もしない）: 保存の途中で `abort` する（`Drop` は走らない）。
    #[test]
    fn killed_save_child() {
        let (Ok(at), Ok(dir)) = (std::env::var(ABORT_AT), std::env::var(ABORT_DIR)) else {
            return;
        };
        let (p, mut t) = SaveTarget::open(Path::new(&dir).join("sample.ylp")).unwrap();
        let _ = t.save_inner(&changed(&p, "落ちる保存"), |phase| {
            if phase == at {
                std::process::abort();
            }
            Ok(())
        });
        // 途中で落ちなかった
        std::process::exit(3);
    }
    #[test]
    fn a_save_killed_midway_leaves_leftovers_that_the_next_save_clears() {
        for at in ["flushed", "backup-copied"] {
            let s = Scratch::new();
            let (p, mut t) = open_original(&s);
            // 退避を 1 つ作っておく（保持数の 1 枠。切れた写しがこの数を食わないこと）
            t.save(&changed(&p, "1 回目")).unwrap();
            let before = fs::read(s.file()).unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "store::tests::killed_save_child",
                    "--test-threads=1",
                ])
                .env(ABORT_AT, at)
                .env(ABORT_DIR, &s.0)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(
                !status.success() && status.code() != Some(3),
                "{at}: 途中で落ちる子: {status:?}"
            );
            // 落ちた子の残骸: 保存先は前の版のまま、保存の一時ファイルとロックのファイルが残る
            assert_eq!(fs::read(s.file()).unwrap(), before, "{at}");
            let left = s.leftovers();
            assert!(
                left.iter().any(|n| n.ends_with(".pending~")),
                "{at}: {left:?}"
            );
            assert!(
                left.iter().any(|n| n.ends_with(".save.lock~")),
                "{at}: {left:?}"
            );
            // 退避の一時ファイル（写し終えたあとで落ちた形）は退避として数えない
            assert_eq!(s.kept_names().len(), 1, "{at}: {:?}", s.backup_dir_names());
            if at == "backup-copied" {
                assert!(
                    s.backup_dir_names()
                        .iter()
                        .any(|n| n.ends_with(".pending~")),
                    "{at}"
                );
            }
            // 次の保存が、誰も保存していないと確かめて（ロックが取れる）片付ける
            t.save(&changed(&p, "3 回目")).unwrap();
            assert!(s.leftovers().is_empty(), "{at}: {:?}", s.names());
            let mut kept = s.kept_names();
            kept.sort();
            assert_eq!(s.backup_dir_names(), kept, "{at}: 一時の名前は残らない");
            assert_eq!(kept.len(), 2, "{at}");
        }
    }

    /// （Windows でだけ回る。Linux では、`cargo check --target x86_64-pc-windows-gnu` で型まで確かめる）一時ファイルとロックは隠し属性つきで
    /// 作り、保存した .ylp と退避は隠さない。
    #[cfg(windows)]
    #[test]
    fn temporary_files_and_the_lock_are_hidden_and_the_saved_files_are_not() {
        use std::os::windows::fs::MetadataExt;
        const HIDDEN: u32 = 0x2;
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let mut seen = Vec::new();
        t.save_inner(&changed(&p, "隠し属性"), |phase| {
            if phase == "backup-copied" {
                for entry in fs::read_dir(&s.0)
                    .unwrap()
                    .chain(fs::read_dir(s.backups()).unwrap())
                {
                    let entry = entry.unwrap();
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.ends_with(".pending~") || name.ends_with(".save.lock~") {
                        seen.push((
                            name,
                            entry.metadata().unwrap().file_attributes() & HIDDEN != 0,
                        ));
                    }
                }
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(
            seen.len(),
            3,
            "保存の一時ファイル・ロック・退避の一時ファイル: {seen:?}"
        );
        assert!(seen.iter().all(|(_, hidden)| *hidden), "{seen:?}");
        for path in std::iter::once(s.file()).chain(backups(&s.file()).unwrap()) {
            assert_eq!(
                fs::metadata(&path).unwrap().file_attributes() & HIDDEN,
                0,
                "{path:?}"
            );
        }
    }
    /// （Windows でだけ回る）外のツールが削除を共有せずに保存先を開いているあいだは置き換えが共有違反になる。手放されるまで待ってやり直し、通る。
    #[cfg(windows)]
    #[test]
    fn a_real_sharing_violation_on_the_target_is_waited_out() {
        use std::os::windows::fs::OpenOptionsExt;
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let holder = OpenOptions::new()
            .read(true)
            .share_mode(0x1 | 0x2)
            .open(s.file())
            .unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            drop(holder);
        });
        let started = std::time::Instant::now();
        t.save(&changed(&p, "共有違反のあと")).unwrap();
        assert!(
            started.elapsed() >= Duration::from_millis(300),
            "手放されるまで待った: {:?}",
            started.elapsed()
        );
        release.join().unwrap();
        assert_ne!(fs::read(s.file()).unwrap(), ORIGINAL);
        assert!(s.leftovers().is_empty(), "{:?}", s.names());
    }
    /// `save_with_progress` は、各段の始めを 1 度ずつ順に知らせる。外で書き換えられていて断る保存は、断るところまでの段だけ。
    #[test]
    fn progress_tells_each_stage_once_in_order_and_stops_where_a_save_is_refused() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        let mut seen = Vec::new();
        t.save_with_progress(
            &changed(&p, "段の知らせ"),
            BackupKeep::All,
            &mut |stage| seen.push(stage),
        )
        .unwrap();
        assert_eq!(
            seen,
            [
                SaveStage::Counting,
                SaveStage::Writing,
                SaveStage::Verifying,
                SaveStage::Replacing
            ]
        );
        assert_eq!(
            seen.iter().map(|s| s.index()).collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
        assert_eq!(SaveStage::COUNT, 4);
        // 外で書き換えられた保存先: 数えて、ロックのあとの確かめで断る（書く段へは進まない）
        fs::write(s.file(), b"someone else").unwrap();
        let mut seen = Vec::new();
        let refused = t
            .save_with_progress(&changed(&p, "断る"), BackupKeep::All, &mut |stage| {
                seen.push(stage)
            })
            .unwrap_err();
        assert!(matches!(refused, Error::SaveConflict(_)), "{refused:?}");
        assert!(
            seen.len() <= 2 && seen.first() == Some(&SaveStage::Counting),
            "{seen:?}"
        );
        assert_bytes(
            &fs::read(s.file()).unwrap(),
            b"someone else",
            "外のファイルは潰さない",
        );
        assert!(s.leftovers().is_empty(), "{:?}", s.names());
    }
    /// 保存先の複製は、持ち主の印を動かさない。複製で保存した後、元の印のまま保存すると、外で変えられたものとして断る（印を取り違えて
    /// 上書きしない）。保存した複製の印に置き換えれば、次の保存は通る。
    #[test]
    fn a_cloned_target_keeps_the_owners_stamp_until_it_is_replaced_by_the_saved_one() {
        let s = Scratch::new();
        let (p, mut owner) = open_original(&s);
        let mut copy = owner.clone();
        assert_eq!(owner.stamp(), copy.stamp());
        let after = changed(&p, "複製で保存");
        copy.save(&after).unwrap();
        assert_ne!(owner.stamp(), copy.stamp());
        let written = fs::read(s.file()).unwrap();
        let refused = owner.save(&changed(&after, "元の印のまま")).unwrap_err();
        assert!(
            matches!(&refused, Error::SaveConflict(why) if why.contains("外部で変更")),
            "{refused:?}"
        );
        assert_bytes(
            &fs::read(s.file()).unwrap(),
            &written,
            "断ったので、複製の保存のまま",
        );
        owner = copy;
        owner
            .save(&changed(&after, "複製の印で置き換えた"))
            .unwrap();
        assert_ne!(fs::read(s.file()).unwrap(), written);
        assert!(s.leftovers().is_empty(), "{:?}", s.names());
    }
    /// 書いた一時ファイルが（書いた直後に）壊れていたら、置換の前に断る。断る理由は、確かめを動かすスレッド数に依らず同じで、保存先は前の
    /// ままで、一時ファイルもロックも残さない。
    #[test]
    fn a_broken_temporary_file_is_refused_before_the_replace_for_the_same_reason_at_any_thread_count(
    ) {
        let reasons: Vec<String> = [1usize, 2, 8]
            .into_iter()
            .map(|threads| {
                let s = Scratch::new();
                let (p, mut t) = open_original(&s);
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .unwrap();
                let folder = s.0.clone();
                let error = pool
                    .install(|| {
                        t.save_inner(&changed(&p, "壊す"), |phase| {
                            if phase == "flushed" {
                                // 書き終えた一時ファイルの真ん中あたりの 1 バイトを書き換える
                                let pending = fs::read_dir(&folder)
                                    .unwrap()
                                    .map(|e| e.unwrap().path())
                                    .find(|p| p.to_string_lossy().ends_with(".pending~"))
                                    .expect("書いている一時ファイル");
                                let mut bytes = fs::read(&pending).unwrap();
                                let at = bytes.len() / 2;
                                bytes[at] ^= 0xff;
                                fs::write(&pending, bytes).unwrap();
                            }
                            Ok(())
                        })
                    })
                    .expect_err("壊れた一時ファイルは置換しない");
                assert!(matches!(error, Error::InvalidData(_)), "{error:?}");
                assert_bytes(&fs::read(s.file()).unwrap(), ORIGINAL, "保存先は前のまま");
                assert!(s.leftovers().is_empty(), "{:?}", s.names());
                assert!(!s.backups().exists(), "{:?}", s.names());
                error.to_string()
            })
            .collect();
        assert!(reasons.windows(2).all(|w| w[0] == w[1]), "{reasons:?}");
    }
}
