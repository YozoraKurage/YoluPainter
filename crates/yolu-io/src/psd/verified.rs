//! 検証つきの PSD の書き出し: 同じフォルダの一時ファイルへ流して書き（1 MiB の緩衝。メモリには層 1 枚ぶんだけ）、同期し、最後まで流して
//! 読み戻して確かめ（[`verify_stream`]: 全層・マスク・統合画像の全行を復号し、長さと層の数が書いたものと合う）、書いたバイト列と
//! CRC-32 と長さで照らしてから確定する。読めない・書いたものと違う PSD は確定しない。途中の失敗・取消・panic の巻き戻しでは、
//! 一時ファイルを消し、元のファイルは変えない。
//!
//! 確定の仕方は 2 つ（[`Commit`]）: 置き換える `rename` と、「あれば失敗」の `hard_link`。`hard_link` の使えないファイルシステム
//! （FAT・exFAT など）では、新しく作る確定は [`WriteError::Commit`] で失敗する（置き換えない移動へ落とさない）。
//! 一時ファイルの名前は `.{ファイル名}.{pid}-{通し番号}.pending~`（[`crate::atomic`] と同じ形）。

use std::cell::Cell;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use yolu_core::Document as CoreDocument;

use std::sync::atomic::AtomicBool;

use super::{verify_stream, Compression, ExportControl, ExportError, ExportPlan, Written};
use crate::Error;

/// 確かめた一時ファイルの確定の仕方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Commit {
    /// 置き換える（`rename`。行き先が無ければ作る）。
    Replace,
    /// 新しく作る。同じ名前が先にあれば失敗する（`hard_link`。確かめたあとに外で作られたファイルも上書きしない）。
    CreateNew,
}

/// 検証つきの書き出しの失敗（どの段か。呼び手が段ごとに理由を言い分ける）。
#[derive(Debug)]
pub enum WriteError {
    /// 行き先が通常のファイルでない（フォルダ・リンクなど）。
    NotAFile,
    /// 一時ファイルを作れない。
    CreateTemp { temp: PathBuf, source: io::Error },
    /// PSD を書けない（予算・形式の上限・取消・書き込みの失敗）。
    Write(ExportError),
    /// 書いた中身を一時ファイルへ出し切れない・同期できない。
    Sync { temp: PathBuf, source: io::Error },
    /// 読み戻すために先頭へ戻せない。
    Rewind { temp: PathBuf, source: io::Error },
    /// 流して読み戻せない（読めない PSD・読み込みの失敗・取消）。
    ReadBack(Error),
    /// 書いたバイト列と照らす読み込みの失敗（取消も）。
    Compare(Error),
    /// 読み戻しが書いたものと違う（長さ・層の数・CRC-32）。
    Mismatch,
    /// 確定（置き換える・新しく作る）の失敗。
    Commit(io::Error),
    /// [`Commit::CreateNew`] で、同じ名前のファイルが先にあった。
    Exists,
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAFile => f.write_str("not a regular file"),
            Self::CreateTemp { source, .. } => {
                write!(f, "cannot create the temporary file: {source}")
            }
            Self::Write(e) => e.fmt(f),
            Self::Sync { source, .. } => write!(f, "cannot finish the temporary file: {source}"),
            Self::Rewind { source, .. } => {
                write!(f, "cannot read the temporary file back: {source}")
            }
            Self::ReadBack(e) | Self::Compare(e) => e.fmt(f),
            Self::Mismatch => f.write_str("the written file does not read back identically"),
            Self::Commit(e) => e.fmt(f),
            Self::Exists => f.write_str("the file already exists"),
        }
    }
}

impl std::error::Error for WriteError {}

/// 一時ファイルへ書いて確かめた、まだ確定していないファイル（落とすと一時ファイルも消える）。
#[derive(Debug)]
pub struct Staged {
    path: PathBuf,
    temp: Option<PathBuf>,
    len: u64,
}

impl Staged {
    /// 行き先。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 書いたファイルの大きさ（バイト）。
    pub fn len(&self) -> u64 {
        self.len
    }

    /// 空のファイルか。
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// 一時ファイルの場所（試験用: 確定の失敗を作る）。
    #[doc(hidden)]
    pub fn temp_path(&self) -> &Path {
        self.temp.as_deref().expect("確定していない一時ファイル")
    }

    /// 確定する。失敗したら一時ファイルは消え、行き先は前のまま。
    pub fn commit(mut self, how: Commit) -> Result<(), WriteError> {
        let temp = self.temp.as_deref().expect("確定していない一時ファイル");
        match how {
            Commit::Replace => {
                fs::rename(temp, &self.path).map_err(WriteError::Commit)?;
                // 移したので、一時ファイルの名前はもう無い
                self.temp = None;
            }
            // 一時ファイルの名前は、落とすときに消える
            Commit::CreateNew => match fs::hard_link(temp, &self.path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    return Err(WriteError::Exists)
                }
                Err(e) => return Err(WriteError::Commit(e)),
            },
        }
        Ok(())
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if let Some(temp) = self.temp.take() {
            let _ = fs::remove_file(temp);
        }
    }
}

/// `plan` の PSD を `path` の一時ファイルへ書いて確かめる（確定はしない。複数のファイルを全部確かめてから確定する呼び手のため）。
/// 取消は `ctl.cancel` で、書く途中と読み戻しの途中に効く。
pub fn stage_verified(
    path: &Path,
    plan: &ExportPlan,
    doc: &CoreDocument,
    ctl: &ExportControl<'_>,
) -> Result<Staged, WriteError> {
    let written = Cell::new(None);
    stage_with(
        path,
        |out| {
            written.set(Some(
                plan.write_psd(doc, ctl, out, Compression::Rle)
                    .map_err(WriteError::Write)?,
            ));
            Ok(())
        },
        |temp, file| check_written(temp, file, written.get().expect("書いた"), ctl.cancel),
        |e| e,
    )
}

/// 書いた PSD（`path` の `file`）を読み戻して確かめる。まず最後まで流して読み戻す（壊れている理由はここで言える）。つぎに、書いた
/// バイト列と一致するか（書きながら数えた CRC-32 と長さ。構造を壊さない画素のビット化けも見つける）。照合の CRC-32 は、読み戻しが
/// 読むバイトをその場で数え、読み戻しが飛ばした区間と残りだけを読み足して数える（ファイルを 2 度読まない。[`Checksum::recount`]）。
///
/// [`Checksum::recount`]: super::Checksum::recount
pub fn check_written(
    temp: &Path,
    file: &mut File,
    written: Written,
    cancel: Option<&AtomicBool>,
) -> Result<(), WriteError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|source| WriteError::Rewind {
            temp: temp.to_path_buf(),
            source,
        })?;
    let mut reader = BufReader::with_capacity(256 * 1024, written.checksum.recount(&mut *file));
    let verified = verify_stream(&mut reader, cancel).map_err(WriteError::ReadBack)?;
    if verified.bytes != written.bytes || verified.layers != written.layers {
        return Err(WriteError::Mismatch);
    }
    if reader
        .into_inner()
        .finish(cancel)
        .map_err(WriteError::Compare)?
    {
        Ok(())
    } else {
        Err(WriteError::Mismatch)
    }
}

/// `plan` の PSD を `path` へ、一時ファイルへ書いて確かめてから `how` で確定する。書いた大きさ（バイト）を返す。
pub fn write_verified(
    path: &Path,
    plan: &ExportPlan,
    doc: &CoreDocument,
    ctl: &ExportControl<'_>,
    how: Commit,
) -> Result<u64, WriteError> {
    let staged = stage_verified(path, plan, doc, ctl)?;
    let len = staged.len();
    staged.commit(how)?;
    Ok(len)
}

/// 一時ファイルへ `fill` で書き（1 MiB の緩衝。終わりまで出して同期する）、`check` で確かめる（一時ファイルの場所と、先頭へ戻した
/// 読み書きのハンドル）。途中で失敗・取消・panic したら一時ファイルを消す。通常のファイル以外の行き先は断る。ファイルの操作の失敗は
/// `fail` で呼び手の誤りにする。PSD でない中身の試験にも使う。
#[doc(hidden)]
pub fn stage_with<E>(
    path: &Path,
    fill: impl FnOnce(&mut BufWriter<&mut File>) -> Result<(), E>,
    check: impl FnOnce(&Path, &mut File) -> Result<(), E>,
    fail: impl Fn(WriteError) -> E,
) -> Result<Staged, E> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| fail(WriteError::NotAFile))?
        .to_string_lossy();
    if let Ok(meta) = fs::symlink_metadata(path) {
        if !meta.is_file() {
            return Err(fail(WriteError::NotAFile));
        }
    }
    let temp_path = dir.join(crate::atomic::pending_name(&name));
    // 作れたら、ここから先は持ち主（`Staged`）が消す
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .map_err(|source| {
            fail(WriteError::CreateTemp {
                temp: temp_path.clone(),
                source,
            })
        })?;
    let mut staged = Staged {
        path: path.to_path_buf(),
        temp: Some(temp_path.clone()),
        len: 0,
    };
    let sync = |source| {
        fail(WriteError::Sync {
            temp: temp_path.clone(),
            source,
        })
    };
    {
        let mut out = BufWriter::with_capacity(1 << 20, &mut file);
        fill(&mut out)?;
        out.flush().map_err(sync)?;
    }
    file.sync_all().map_err(sync)?;
    file.seek(SeekFrom::Start(0)).map_err(|source| {
        fail(WriteError::Rewind {
            temp: temp_path.clone(),
            source,
        })
    })?;
    check(&temp_path, &mut file)?;
    drop(file);
    staged.len = fs::metadata(&temp_path).map_err(sync)?.len();
    Ok(staged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct Dir(PathBuf);
    impl Dir {
        fn new(tag: &str) -> Dir {
            static N: AtomicU32 = AtomicU32::new(0);
            let p = std::env::temp_dir().join(format!(
                "yolu-io-psd-verified-{tag}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&p).unwrap();
            Dir(p)
        }
        fn files(&self) -> Vec<String> {
            let mut v: Vec<String> = fs::read_dir(&self.0)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn painted() -> (CoreDocument, ExportPlan) {
        let mut doc = CoreDocument::new(16, 16).unwrap();
        let layer = doc.add_layer("下絵").unwrap();
        for y in 0..8 {
            for x in 0..8 {
                let color = yolu_core::Rgba8 {
                    r: 200,
                    g: 10 * x as u8,
                    b: 10,
                    a: 255,
                };
                doc.set_pixel(layer, x, y, color).unwrap();
            }
        }
        let plan = super::super::plan_export(
            &doc,
            &super::super::ExportOptions::new(
                yolu_core::Channel::Color,
                super::super::ExportMode::Bake,
            ),
            &ExportControl::default(),
        )
        .unwrap();
        (doc, plan)
    }

    /// 前は一時ファイルの名前が 255 バイトを超え、置き先は作れるのに「一時ファイルを作れません」で失敗した名前。
    #[cfg(unix)]
    #[test]
    fn a_verified_psd_with_a_name_near_the_os_limit_is_written() {
        let dir = Dir::new("long");
        let (doc, plan) = painted();
        let ctl = ExportControl::default();
        for name in ["a".repeat(251) + ".psd", "あ".repeat(83) + ".psd"] {
            let path = dir.0.join(&name);
            write_verified(&path, &plan, &doc, &ctl, Commit::CreateNew).unwrap();
            write_verified(&path, &plan, &doc, &ctl, Commit::Replace).unwrap();
            assert_eq!(dir.files(), std::slice::from_ref(&name));
            fs::remove_file(&path).unwrap();
        }
    }

    #[test]
    fn a_verified_psd_replaces_the_old_file_and_leaves_no_temp_file() {
        let dir = Dir::new("replace");
        let path = dir.0.join("a.psd");
        fs::write(&path, b"old").unwrap();
        let (doc, plan) = painted();
        let len = write_verified(
            &path,
            &plan,
            &doc,
            &ExportControl::default(),
            Commit::Replace,
        )
        .unwrap();
        assert_eq!(fs::metadata(&path).unwrap().len(), len);
        assert_eq!(dir.files(), ["a.psd"]);
        let verified = verify_stream(&mut File::open(&path).unwrap(), None).unwrap();
        assert_eq!(verified.bytes, len);
    }

    #[test]
    fn create_new_fails_when_the_file_exists_and_changes_nothing() {
        let dir = Dir::new("create");
        let path = dir.0.join("a.psd");
        let (doc, plan) = painted();
        write_verified(
            &path,
            &plan,
            &doc,
            &ExportControl::default(),
            Commit::CreateNew,
        )
        .unwrap();
        let first = fs::read(&path).unwrap();
        assert_eq!(dir.files(), ["a.psd"], "一時ファイルは消える");
        let err = write_verified(
            &path,
            &plan,
            &doc,
            &ExportControl::default(),
            Commit::CreateNew,
        )
        .unwrap_err();
        assert!(matches!(err, WriteError::Exists), "{err:?}");
        assert_eq!(fs::read(&path).unwrap(), first);
        assert_eq!(dir.files(), ["a.psd"]);
    }

    #[test]
    fn a_read_back_that_does_not_match_is_not_committed() {
        let dir = Dir::new("mismatch");
        let path = dir.0.join("a.psd");
        fs::write(&path, b"old").unwrap();
        let (doc, plan) = painted();
        // 書いたあと、確かめる前に統合画像の最後の 1 バイトを変える（読み戻しか照らし合わせのどちらかで断る）
        let written = std::cell::Cell::new(None);
        let err = stage_with(
            &path,
            |out| {
                written.set(Some(
                    plan.write_psd(&doc, &ExportControl::default(), out, Compression::Rle)
                        .map_err(WriteError::Write)?,
                ));
                Ok(())
            },
            |temp, file| {
                let len = file.metadata().unwrap().len();
                file.seek(SeekFrom::Start(len - 1)).unwrap();
                let mut last = [0u8];
                std::io::Read::read_exact(file, &mut last).unwrap();
                file.seek(SeekFrom::Start(len - 1)).unwrap();
                file.write_all(&[last[0] ^ 0x01]).unwrap();
                file.seek(SeekFrom::Start(0)).unwrap();
                check_written(temp, file, written.get().unwrap(), None)
            },
            |e| e,
        )
        .unwrap_err();
        assert!(
            matches!(err, WriteError::Mismatch | WriteError::ReadBack(_)),
            "{err:?}"
        );
        assert_eq!(fs::read(&path).unwrap(), b"old", "確定しない");
        assert_eq!(dir.files(), ["a.psd"], "一時ファイルは消える");
    }

    #[test]
    fn a_folder_is_not_a_destination() {
        let dir = Dir::new("folder");
        let folder = dir.0.join("f.psd");
        fs::create_dir(&folder).unwrap();
        let (doc, plan) = painted();
        let err = write_verified(
            &folder,
            &plan,
            &doc,
            &ExportControl::default(),
            Commit::Replace,
        )
        .unwrap_err();
        assert!(matches!(err, WriteError::NotAFile), "{err:?}");
        assert_eq!(dir.files(), ["f.psd"]);
    }

    /// 照合を別の読みで行う形（読み戻してから、先頭に戻ってファイルをもう 1 度読み、CRC-32 と長さを照らす）。
    fn check_twice(file: &mut File, written: Written) -> Result<(), WriteError> {
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut reader = BufReader::with_capacity(256 * 1024, &mut *file);
        let verified = verify_stream(&mut reader, None).map_err(WriteError::ReadBack)?;
        if verified.bytes != written.bytes || verified.layers != written.layers {
            return Err(WriteError::Mismatch);
        }
        file.seek(SeekFrom::Start(0)).unwrap();
        if written
            .checksum
            .matches(file, None)
            .map_err(WriteError::Compare)?
        {
            Ok(())
        } else {
            Err(WriteError::Mismatch)
        }
    }

    /// 段（どこで断ったか）と理由。
    fn outcome(r: &Result<(), WriteError>) -> String {
        match r {
            Ok(()) => "ok".into(),
            Err(WriteError::ReadBack(e)) => format!("read back: {e:?}"),
            Err(WriteError::Compare(e)) => format!("compare: {e:?}"),
            Err(WriteError::Mismatch) => "mismatch".into(),
            Err(e) => format!("other: {e:?}"),
        }
    }

    /// 書いた PSD のバイト列と、書いた記録。
    fn written_bytes(doc: &CoreDocument, plan: &ExportPlan) -> (Vec<u8>, Written) {
        let mut out = std::io::Cursor::new(Vec::new());
        let written = plan
            .write_psd(doc, &ExportControl::default(), &mut out, Compression::Rle)
            .unwrap();
        (out.into_inner(), written)
    }

    /// 化けたヘッダーの寸法・層の矩形が大きなキャンバスを言っても、読み戻しはその大きさの場所を先に取らずに断る（ファイルの残りに入らない
    /// 統合画像・チャンネルは、確保の前に断る。確かめは画素を持たずに行を展開する）。
    #[test]
    fn a_corrupted_size_is_refused_without_reserving_room_for_it() {
        let (doc, plan) = painted();
        let (bytes, written) = written_bytes(&doc, &plan);
        let be = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        // 最初の層の記録（上・左・下・右）: ヘッダー 26 バイト・色モードデータ・画像リソースのあと、2 つの長さと層の数
        let mut at = 26;
        for _ in 0..2 {
            at += 4 + be(at);
        }
        let record = at + 4 + 4 + 2;
        let cases: [&[(usize, u32)]; 6] = [
            // 高さ・幅の上の桁の 1 ビット（2^30 を超える）・u32 の最大・形式の上限ちょうど
            &[(14, (1 << 30) | 16)],
            &[(18, (1 << 30) | 16)],
            &[(14, u32::MAX), (18, u32::MAX)],
            &[(14, 30000), (18, 30000)],
            &[(14, 30001)],
            // 層の矩形（下・右）を形式の上限まで
            &[(record + 8, 30000), (record + 12, 30000)],
        ];
        let dir = Dir::new("size");
        let path = dir.0.join("a.psd");
        for edits in cases {
            let mut b = bytes.clone();
            for &(at, v) in edits {
                b[at..at + 4].copy_from_slice(&v.to_be_bytes());
            }
            let err = verify_stream(&mut std::io::Cursor::new(&b), None).unwrap_err();
            assert!(matches!(err, Error::InvalidData(_)), "{edits:?}: {err:?}");
            fs::write(&path, &b).unwrap();
            let mut file = OpenOptions::new().read(true).open(&path).unwrap();
            let checked = check_written(&path, &mut file, written, None);
            assert!(
                matches!(checked, Err(WriteError::ReadBack(Error::InvalidData(_)))),
                "{edits:?}: {checked:?}"
            );
        }
    }

    /// 読み戻しの読みで照合も数える形が、壊れた書き込みを、ファイルをもう 1 度読む形と同じ段・同じ理由で断る。小さな PSD の全部のバイトを
    /// 1 つずつ化けさせる（読み戻しが飛ばす区間・書き直した先頭・層の画素・統合画像のどれも）。長い・短いファイルも。
    #[test]
    fn the_single_read_check_refuses_every_corruption_like_the_two_read_check() {
        let dir = Dir::new("flip");
        let path = dir.0.join("a.psd");
        let (doc, plan) = painted();
        let (bytes, written) = written_bytes(&doc, &plan);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        let mut put = |b: &[u8]| {
            file.set_len(0).unwrap();
            file.seek(SeekFrom::Start(0)).unwrap();
            file.write_all(b).unwrap();
            let one = outcome(&check_written(&path, &mut file, written, None));
            let two = outcome(&check_twice(&mut file, written));
            (one, two)
        };
        let (one, two) = put(&bytes);
        assert_eq!((one.as_str(), two.as_str()), ("ok", "ok"));
        let mut refused = 0;
        // 高さ・幅の上の桁の化けも（大きな寸法は、統合画像の場所を取る前に断る）
        for i in 0..bytes.len() {
            // 下の桁と上の桁を交互に（長さの欄の符号の桁も化けさせる）
            let bit = if i % 2 == 0 { 0x01u8 } else { 0x80 };
            let mut b = bytes.clone();
            b[i] ^= bit;
            let (one, two) = put(&b);
            assert_eq!(one, two, "{i} バイト目の {bit:#x}");
            assert_ne!(one, "ok", "{i} バイト目の {bit:#x}: 化けを見逃した");
            refused += 1;
        }
        for b in [
            [bytes.clone(), vec![0]].concat(),
            [bytes.clone(), vec![7; 300 * 1024]].concat(),
            bytes[..bytes.len() - 1].to_vec(),
        ] {
            let (one, two) = put(&b);
            assert_eq!(one, two, "{} バイト", b.len());
            assert_ne!(one, "ok");
        }
        assert_eq!(refused, bytes.len());
    }

    /// 大きめの PSD（読みの緩衝を何度もまたぐ・緩衝より大きな読み）でも、化けた所によらず同じ断り。読み戻しが飛ばす区間（画像リソース）を含む。
    #[test]
    fn the_single_read_check_matches_on_a_larger_file() {
        let dir = Dir::new("large");
        let path = dir.0.join("a.psd");
        let mut doc = CoreDocument::new(700, 500).unwrap();
        for k in 0..3u32 {
            let layer = doc.add_layer(&format!("l{k}")).unwrap();
            for y in (0..500).step_by(3) {
                for x in (k * 40..700).step_by(2) {
                    let v = (x * 7 + y * 13 + k * 31) as u8;
                    doc.set_pixel(layer, x, y, yolu_core::Rgba8::new(v, v ^ 0x55, 200, 255))
                        .unwrap();
                }
            }
        }
        let plan = super::super::plan_export(
            &doc,
            &super::super::ExportOptions::new(
                yolu_core::Channel::Color,
                super::super::ExportMode::Bake,
            ),
            &ExportControl::default(),
        )
        .unwrap();
        let (bytes, written) = written_bytes(&doc, &plan);
        assert!(bytes.len() > 512 * 1024, "{}", bytes.len());
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        file.write_all(&bytes).unwrap();
        assert!(check_written(&path, &mut file, written, None).is_ok());
        let cancel = AtomicBool::new(true);
        let cancelled = check_written(&path, &mut file, written, Some(&cancel));
        assert!(
            matches!(
                &cancelled,
                Err(WriteError::ReadBack(crate::Error::Core(
                    yolu_core::CoreError::Cancelled
                )))
            ),
            "{cancelled:?}"
        );
        let step = bytes.len() / 97;
        for i in (0..bytes.len())
            .step_by(step)
            .chain([30, 34, 40, bytes.len() - 1])
        {
            file.seek(SeekFrom::Start(i as u64)).unwrap();
            file.write_all(&[bytes[i] ^ 0x10]).unwrap();
            let one = outcome(&check_written(&path, &mut file, written, None));
            let two = outcome(&check_twice(&mut file, written));
            assert_eq!(one, two, "{i} バイト目");
            assert_ne!(one, "ok", "{i} バイト目");
            file.seek(SeekFrom::Start(i as u64)).unwrap();
            file.write_all(&[bytes[i]]).unwrap();
        }
        assert!(check_written(&path, &mut file, written, None).is_ok());
    }
}
