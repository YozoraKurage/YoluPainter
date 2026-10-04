use crate::{check, check_budget, hash, Error, Project, Result, MAX_TOTAL_BYTES};

fn conflict(ok: bool, reason: impl Into<String>) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error::SaveConflict(reason.into()))
    }
}
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};
static NEXT: AtomicU64 = AtomicU64::new(0);
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
#[derive(Debug)]
pub struct SaveTarget {
    path: PathBuf,
    expected: Option<FileStamp>,
}
impl SaveTarget {
    pub fn open(path: impl AsRef<Path>) -> Result<(Project, Self)> {
        let path = path.as_ref().to_path_buf();
        let (b, stamp) = read_stamp(&path)?;
        let p = Project::read(&b)?;
        Ok((
            p,
            Self {
                path,
                expected: Some(stamp),
            },
        ))
    }
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
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
    pub fn save(&mut self, project: &Project) -> Result<FileStamp> {
        self.save_inner(project, |_| Ok(()))
    }
    fn save_inner(
        &mut self,
        project: &Project,
        phase: impl FnMut(&str) -> Result<()>,
    ) -> Result<FileStamp> {
        self.save_locking(project, File::try_lock, phase)
    }
    /// `try_lock` は保存のロックを取る操作（試験が、ロックの使えないファイルシステムを差し込む口）。
    fn save_locking(
        &mut self,
        project: &Project,
        try_lock: impl FnMut(&File) -> std::result::Result<(), TryLockError>,
        mut phase: impl FnMut(&str) -> Result<()>,
    ) -> Result<FileStamp> {
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
        let _lock =
            SaveLock::acquire_with(parent.join(format!(".{name}.save.lock~")), try_lock, |_| {})?;
        self.check_expected()?;
        let bytes = project.to_bytes()?;
        Project::read(&bytes)?;
        phase("memory-verified")?;
        let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
        let mut pending = Pending::create(
            parent.join(format!(".{name}.{}-{nonce}.pending~", std::process::id())),
        )?;
        let f = pending.file.as_mut().unwrap();
        f.write_all(&bytes)?;
        f.sync_all()?;
        phase("flushed")?;
        pending.file.take();
        let (written, new_stamp) = read_stamp(&pending.path)?;
        conflict(written == bytes, "一時ファイルの内容が変化しました")?;
        Project::read(&written)?;
        phase("disk-verified")?;
        if let Some(expected) = &self.expected {
            let (previous, stamp) = read_target(&self.path)?;
            conflict(
                stamp.same_content(expected),
                "保存先が外部で変更されています",
            )?;
            let backups = parent.join(format!("{name}-backups~"));
            // 前の版の置き場が普通のファイルで塞がれていたら、保存先に触る前に断る。保存先の周りの事情なので、
            // プロジェクトのデータの不正（InvalidData）ではなく SaveConflict で、画面が言い分けられるようにする
            match fs::symlink_metadata(&backups) {
                Ok(m) if !m.file_type().is_symlink() => {
                    conflict(m.is_dir(), "バックアップ先がフォルダーではありません")?
                }
                _ => {}
            }
            fs::create_dir_all(&backups)?;
            conflict(
                !fs::symlink_metadata(&backups)?.file_type().is_symlink(),
                "バックアップ先がシンボリックリンクです",
            )?;
            let backup = backups.join(format!("{}.ylp", expected.sha256));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&backup)
            {
                Ok(mut f) => {
                    f.write_all(&previous)?;
                    f.sync_all()?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let (b, _) = read_stamp(&backup)?;
                    conflict(b == previous, "既存のバックアップが一致しません")?;
                }
                Err(e) => return Err(e.into()),
            };
        }
        phase("before-replace")?;
        self.check_expected()?;
        // 削除してから移動する代替手順は使わない。置換の失敗時は元を残す。
        fs::rename(&pending.path, &self.path)?;
        pending.committed = true;
        self.expected = Some(new_stamp.clone());
        // 置換の後に失敗しても新しい版は確定している（印も新しい）。呼び出し側へは失敗として返し、
        // 次の保存は新しい印のまま衝突せずに通る。ロックと一時ファイルの後始末はここを抜けても働く
        phase("after-replace")?;
        Ok(new_stamp)
    }
    fn check_expected(&self) -> Result<()> {
        match &self.expected {
            Some(want) => {
                let (_, actual) = read_target(&self.path)?;
                conflict(
                    actual.same_content(want),
                    "保存先が外部で変更されています。上書きしません",
                )
            }
            None => conflict(
                absent(&self.path)?,
                "新規保存先が外部で作られました。上書きしません",
            ),
        }
    }
}
fn absent(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(e.into()),
    }
}
/// 開いたあとの保存先を読む。消されていたら外部の変更として断る（無いファイルの代わりに書き始めない）。
fn read_target(path: &Path) -> Result<(Vec<u8>, FileStamp)> {
    match read_stamp(path) {
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Err(Error::SaveConflict(
            "保存先が外部で消されています。上書きしません".into(),
        )),
        other => other,
    }
}
fn read_stamp(path: &Path) -> Result<(Vec<u8>, FileStamp)> {
    let metadata = fs::symlink_metadata(path)?;
    check(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "通常のファイルではありません",
    )?;
    check_budget(
        metadata.len() <= (MAX_TOTAL_BYTES + 2 * 1024 * 1024) as u64,
        "ファイルの読み込み予算超過です",
    )?;
    let f = File::open(path)?;
    let mut b = Vec::new();
    f.take((MAX_TOTAL_BYTES + 2 * 1024 * 1024 + 1) as u64)
        .read_to_end(&mut b)?;
    let after = fs::metadata(path)?;
    conflict(
        b.len() as u64 == metadata.len()
            && after.len() == metadata.len()
            && after.modified()? == metadata.modified()?,
        "読み込み中にファイルが変更されました",
    )?;
    let stamp = FileStamp {
        sha256: hash(&b),
        length: b.len() as u64,
        modified: metadata.modified()?,
    };
    Ok((b, stamp))
}
struct Pending {
    path: PathBuf,
    file: Option<File>,
    committed: bool,
}
impl Pending {
    fn create(path: PathBuf) -> Result<Self> {
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;
        Ok(Self {
            path,
            file: Some(file),
            committed: false,
        })
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.file.take();
        if !self.committed {
            let _ = fs::remove_file(&self.path);
        }
    }
}
/// 保存のロック。「ロックのファイルがあるか」ではなく、開いたハンドルが OS の排他ロックを持っているかで決める。
/// 保存の途中でクラッシュして残ったファイルは誰も持っていないので次の保存の邪魔にならず、終わると消える。
struct SaveLock {
    path: PathBuf,
    file: Option<File>,
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
            // Windows は DELETE を共有しない: 開いている間は他から消せない（消すのは手放してから）
            #[cfg(windows)]
            std::os::windows::fs::OpenOptionsExt::share_mode(&mut options, 0x1 | 0x2);
            let file = options.create(true).open(&path)?;
            match try_lock(&file) {
                Ok(()) => {}
                // 持っているのは別の保存。そのファイルは消さずに断る
                Err(TryLockError::WouldBlock) => return Err(in_progress()),
                // ロックの使えない場所（ロックを取る操作そのものが失敗する FUSE・一部のネットワークのファイルシステム）。
                // C#（.NET の FileShare.None）と同じく排他なしで続ける。断ると、その場所へは二度と保存できない。
                // 置換の前の印の確かめは残る
                Err(TryLockError::Error(_)) => {}
            }
            locked(attempt);
            // 取る間に消されて作り直されていたら、いま持っているのは誰も見ないファイル。手放してやり直す
            if names_the_same_file(&file, &path) {
                return Ok(Self {
                    path,
                    file: Some(file),
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
    use std::sync::atomic::AtomicUsize;
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
    /// 層の名前だけを変えた別の版（保存するとバイト列が変わる）。
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
                crate::NativeValue::Text("変更した層".into()),
            )
            .unwrap();
        let changed = p.with_document(&p.sets()[0].id, &d).unwrap();
        let after = t.save(&changed).unwrap();
        assert_ne!(stamp.sha256, after.sha256);
        let backup =
            s.0.join("sample.ylp-backups~")
                .join(format!("{}.ylp", stamp.sha256));
        assert_eq!(fs::read(backup).unwrap(), before);
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
        ] {
            let s = Scratch::new();
            let (p, mut t) = open_original(&s);
            let stamp = t.stamp().unwrap().clone();
            let error = t.save_inner(&p, fail_at(phase));
            assert!(error.is_err());
            assert_bytes(&fs::read(s.file()).unwrap(), ORIGINAL, "保存先");
            assert!(!s.lock().exists());
            assert!(s.leftovers().is_empty(), "{phase}");
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
        assert_eq!(stamp, read_stamp(&s.file()).unwrap().1);
        // 置換された前の版は退避されている。ロックと一時ファイルは残らない
        assert_bytes(
            &fs::read(s.backups().join(format!("{}.ylp", before.sha256))).unwrap(),
            ORIGINAL,
            "退避",
        );
        assert!(s.leftovers().is_empty());
        let third = t.save(&changed(&p, "もう一度")).unwrap();
        assert_ne!(third.sha256, stamp.sha256);
        assert_bytes(
            &fs::read(s.backups().join(format!("{}.ylp", stamp.sha256))).unwrap(),
            &want,
            "退避",
        );
        assert!(s.leftovers().is_empty());
    }
    #[test]
    fn a_failure_at_every_phase_of_a_new_save_leaves_nothing() {
        for phase in [
            "memory-verified",
            "flushed",
            "disk-verified",
            "before-replace",
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
        assert_eq!(t.stamp(), Some(&read_stamp(&s.file()).unwrap().1));
        assert_eq!(s.names(), ["sample.ylp"]);
        // 以後は上書きとして、前の版を残して保存できる
        let next = t.save(&changed(&p, "新規のあとの上書き")).unwrap();
        assert_eq!(s.names(), ["sample.ylp", "sample.ylp-backups~"]);
        assert_ne!(next.sha256, hash(&want));
        assert_bytes(
            &fs::read(s.backups().join(format!("{}.ylp", hash(&want)))).unwrap(),
            &want,
            "退避",
        );
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
        for point in ["memory-verified", "flushed", "disk-verified"] {
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
        let now = read_stamp(&s.file()).unwrap().1;
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
        assert_bytes(
            &fs::read(s.backups().join(format!("{}.ylp", stamp.sha256))).unwrap(),
            ORIGINAL,
            "退避",
        );
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
        t.save_locking(&changed(&p, "ロックの無い場所"), unsupported_lock, |phase| {
            phases.push(phase.to_string());
            Ok(())
        })
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
        t.save_locking(&project(), unsupported_lock, |_| Ok(())).unwrap();
        assert_eq!(s.names(), ["sample.ylp"]);
    }
    #[test]
    fn a_stale_lock_file_is_cleared_even_where_locks_are_unavailable() {
        let s = Scratch::new();
        let (p, mut t) = open_original(&s);
        fs::write(s.lock(), b"left by a crash").unwrap();
        t.save_locking(&changed(&p, "残ったロック"), unsupported_lock, |_| Ok(()))
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
    fn damaged_existing_backup_aborts_without_replacing() {
        let s = Scratch::new();
        fs::write(s.file(), ORIGINAL).unwrap();
        let (p, mut t) = SaveTarget::open(s.file()).unwrap();
        let dir = s.0.join("sample.ylp-backups~");
        fs::create_dir(&dir).unwrap();
        fs::write(
            dir.join(format!("{}.ylp", t.stamp().unwrap().sha256)),
            b"broken",
        )
        .unwrap();
        assert!(t.save(&p).is_err());
        assert_bytes(&fs::read(s.file()).unwrap(), ORIGINAL, "保存先");
        assert!(s.leftovers().is_empty());
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
        assert_bytes(
            &fs::read(s.backups().join(format!("{}.ylp", stamp.sha256))).unwrap(),
            ORIGINAL,
            "退避",
        );
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
}
