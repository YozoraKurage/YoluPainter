use crate::{check, hash, Error, Project, Result, MAX_TOTAL_BYTES};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};
static NEXT: AtomicU64 = AtomicU64::new(0);
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub sha256: String,
    pub length: u64,
    pub modified: SystemTime,
}
/// 開いた時点の印を持つ保存先。前の版を退避し、検証済み一時ファイルから一度だけ置換する。
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
        check(
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
            .ok_or_else(|| Error("保存先がファイルではありません".into()))?
            .to_string_lossy();
        let lock_path = parent.join(format!(".{name}.save.lock~"));
        let _lock = Pending::create(lock_path)?;
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
        check(written == bytes, "一時ファイルの内容が変化しました")?;
        Project::read(&written)?;
        phase("disk-verified")?;
        if let Some(expected) = &self.expected {
            let (previous, stamp) = read_stamp(&self.path)?;
            check(&stamp == expected, "保存先が外部で変更されています")?;
            let backups = parent.join(format!("{name}-backups~"));
            fs::create_dir_all(&backups)?;
            check(
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
                    check(b == previous, "既存のバックアップが一致しません")?;
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
        Ok(new_stamp)
    }
    fn check_expected(&self) -> Result<()> {
        match &self.expected {
            Some(want) => {
                let (_, actual) = read_stamp(&self.path)?;
                check(
                    &actual == want,
                    "保存先が外部で変更されています。上書きしません",
                )
            }
            None => check(
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
fn read_stamp(path: &Path) -> Result<(Vec<u8>, FileStamp)> {
    let metadata = fs::symlink_metadata(path)?;
    check(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "通常のファイルではありません",
    )?;
    check(
        metadata.len() <= (MAX_TOTAL_BYTES + 2 * 1024 * 1024) as u64,
        "ファイルの読み込み予算超過です",
    )?;
    let f = File::open(path)?;
    let mut b = Vec::new();
    f.take((MAX_TOTAL_BYTES + 2 * 1024 * 1024 + 1) as u64)
        .read_to_end(&mut b)?;
    let after = fs::metadata(path)?;
    check(
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

#[cfg(test)]
mod tests {
    use super::*;
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
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn project() -> Project {
        Project::read(include_bytes!("../tests/fixtures/format1.ylp")).unwrap()
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
            let original = include_bytes!("../tests/fixtures/format1.ylp");
            fs::write(s.file(), original).unwrap();
            let (p, mut t) = SaveTarget::open(s.file()).unwrap();
            let error = t.save_inner(&p, |p| {
                if p == phase {
                    Err(Error("注入した保存障害".into()))
                } else {
                    Ok(())
                }
            });
            assert!(error.is_err());
            assert_eq!(fs::read(s.file()).unwrap(), original);
            assert!(!s.0.join(".sample.ylp.save.lock~").exists());
            assert!(s.0.read_dir().unwrap().all(|p| !p
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with("pending~")));
        }
    }
    #[test]
    fn external_change_just_before_replace_is_refused() {
        let s = Scratch::new();
        fs::write(s.file(), include_bytes!("../tests/fixtures/format1.ylp")).unwrap();
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
    fn new_target_and_existing_lock_do_not_get_overwritten() {
        let s = Scratch::new();
        let mut t = SaveTarget::create(s.file()).unwrap();
        fs::write(s.file(), b"external").unwrap();
        assert!(t.save(&project()).is_err());
        assert!(SaveTarget::create(s.file()).is_err());
        assert_eq!(fs::read(s.file()).unwrap(), b"external");
        fs::remove_file(s.file()).unwrap();
        fs::write(s.0.join(".sample.ylp.save.lock~"), b"locked").unwrap();
        assert!(t.save(&project()).is_err());
        assert!(!s.file().exists());
    }
    #[test]
    fn damaged_existing_backup_aborts_without_replacing() {
        let s = Scratch::new();
        fs::write(s.file(), include_bytes!("../tests/fixtures/format1.ylp")).unwrap();
        let (p, mut t) = SaveTarget::open(s.file()).unwrap();
        let dir = s.0.join("sample.ylp-backups~");
        fs::create_dir(&dir).unwrap();
        fs::write(
            dir.join(format!("{}.ylp", t.stamp().unwrap().sha256)),
            b"broken",
        )
        .unwrap();
        assert!(t.save(&p).is_err());
        assert_eq!(
            fs::read(s.file()).unwrap(),
            include_bytes!("../tests/fixtures/format1.ylp")
        );
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
        fs::write(&original, include_bytes!("../tests/fixtures/format1.ylp")).unwrap();
        std::os::unix::fs::symlink(&original, s.file()).unwrap();
        assert!(SaveTarget::open(s.file()).is_err());
    }
}
