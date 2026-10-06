//! 一時ファイルへ書いて、最後の 1 回の置換で確定する書き込み（設定・ブラシ・プリセット・素材・キャッシュ・ダウンロード）。途中で止まっても
//! 前のファイルは壊れない。
//!
//! - 一時ファイルは置き先と同じフォルダの `.{ファイル名}.{pid}-{通し番号}.pending~`（.ylp の保存と同じ形。[`is_leftover_name`]）。
//!   `create_new` で作るので、同じプロセスで同時に書いても、ほかのプロセスの一時ファイルにも重ならない。
//!   ファイル名が [`STEM_MAX`] バイトより長いときは、先頭を切って全体の指紋を付けた名前にする（一時ファイルが OS の名前の長さの上限
//!   〔255〕を超えて、置き先は作れるのに書けない、を避ける）。
//! - 書いたら `sync_all`。置換は `rename`。Windows の共有違反・ロック違反・アクセス拒否（同期の道具・ウイルス対策が一時的に掴んでいる）
//!   のあいだは短くやり直す（[`retry_busy`]。合計 1.5 秒まで）。
//! - 置換のあと、フォルダは同期しない（.ylp の保存と同じ。置換そのものの耐久は OS に任せる）。
//! - 失敗したら（途中の `?` でも）一時ファイルを消す。置き先は前のまま。

use std::borrow::Cow;
use std::cell::Cell;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// 一時ファイルの通し番号（このプロセスの中で重ならない。.ylp の保存と退避の写しも同じ番号を使う）。
static NEXT: AtomicU64 = AtomicU64::new(0);

/// 書いた一時ファイルの読み戻しを確かめる関数（真なら置き換える）。
pub type Verify<'a> = &'a dyn Fn(&[u8]) -> bool;

/// 置換の書き方。
#[derive(Clone, Copy, Default)]
pub struct ReplaceOptions<'a> {
    /// 書いたファイルの大きさの上限（`verify` の読み戻しもここまで）。超えれば置き換えず [`Rejected::TooLarge`]。
    pub limit: Option<u64>,
    /// 書いた一時ファイルを読み戻して確かめる（偽なら置き換えず [`Rejected::Mismatch`]）。
    pub verify: Option<Verify<'a>>,
    /// 置き先のフォルダが無ければ作る。
    pub create_dirs: bool,
}

/// 書いた一時ファイルを確かめて、置き換えなかった理由（`io::Error` の中身。[`rejected`] で取り出す）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejected {
    /// `limit` より大きい。
    TooLarge,
    /// 読み戻しが `verify` に合わない。
    Mismatch,
}

impl fmt::Display for Rejected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Rejected::TooLarge => "written file exceeds the limit",
            Rejected::Mismatch => "written file does not read back the same",
        })
    }
}

impl std::error::Error for Rejected {}

/// 置換の失敗が、書いた一時ファイルを確かめて断ったものなら、その理由。
pub fn rejected(error: &io::Error) -> Option<Rejected> {
    error.get_ref()?.downcast_ref::<Rejected>().copied()
}

fn reject(why: Rejected) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why)
}

/// `bytes` を `path` へ置く（一時ファイルへ書いて同期し、1 回の置換で確定）。
pub fn replace_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    replace_with(path, &ReplaceOptions::default(), |f| {
        io::Write::write_all(f, bytes)
    })
}

/// `write` が書いた中身を `path` へ置く（一時ファイルへ書いて同期し、`opts` のとおり確かめてから 1 回の置換で確定）。
pub fn replace_with(
    path: &Path,
    opts: &ReplaceOptions<'_>,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    replace_with_seams(
        path,
        opts,
        write,
        &mut Seams {
            rename: &mut |from, to| fs::rename(from, to),
            busy: &is_busy,
            sleep: &mut std::thread::sleep,
        },
    )
}

/// 置換の差し込み口（試験が共有違反を模す）。
struct Seams<'a> {
    rename: &'a mut dyn FnMut(&Path, &Path) -> io::Result<()>,
    busy: &'a dyn Fn(&io::Error) -> bool,
    sleep: &'a mut dyn FnMut(Duration),
}

fn replace_with_seams(
    path: &Path,
    opts: &ReplaceOptions<'_>,
    write: impl FnOnce(&mut File) -> io::Result<()>,
    seams: &mut Seams<'_>,
) -> io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a file path"))?
        .to_string_lossy()
        .into_owned();
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if opts.create_dirs {
        fs::create_dir_all(parent)?;
    }
    let mut temp = Temp {
        path: parent.join(pending_name(&name)),
        armed: true,
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp.path)?;
    write(&mut file)?;
    file.sync_all()?;
    drop(file);
    if let Some(limit) = opts.limit {
        if fs::metadata(&temp.path)?.len() > limit {
            return Err(reject(Rejected::TooLarge));
        }
    }
    if let Some(verify) = opts.verify {
        let mut read = Vec::new();
        File::open(&temp.path)?
            .take(opts.limit.map_or(u64::MAX, |l| l + 1))
            .read_to_end(&mut read)?;
        if opts.limit.is_some_and(|l| read.len() as u64 > l) {
            return Err(reject(Rejected::TooLarge));
        }
        if !verify(&read) {
            return Err(reject(Rejected::Mismatch));
        }
    }
    if FAIL_BEFORE_REPLACE.with(Cell::get) {
        return Err(io::Error::other("replace failed (test)"));
    }
    let rename = &mut *seams.rename;
    retry_busy(seams.busy, &mut *seams.sleep, || rename(&temp.path, path))?;
    temp.armed = false;
    Ok(())
}

/// 書きかけの一時ファイル（落とすと消す。置換で確定したら `armed` を下ろす）。
struct Temp {
    path: PathBuf,
    armed: bool,
}

impl Drop for Temp {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

thread_local! {
    static FAIL_BEFORE_REPLACE: Cell<bool> = const { Cell::new(false) };
}

/// 試験用: `f` の間、このスレッドの置換を、一時ファイルを書いたあと・置き換える前に失敗させる（置き先が前のまま残り、一時ファイルも
/// 残らないことを確かめる）。
#[doc(hidden)]
pub fn failing<R>(f: impl FnOnce() -> R) -> R {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            FAIL_BEFORE_REPLACE.with(|c| c.set(self.0));
        }
    }
    let _reset = Reset(FAIL_BEFORE_REPLACE.with(|c| c.replace(true)));
    f()
}

/// 一時ファイルの名前に入れる置き先のファイル名（以下「幹」）の最長（バイト）。一時ファイルの名前は `.{幹}.{pid}-{通し番号}.pending~`
/// で、pid（10 桁まで）・通し番号（20 桁まで）・区切りと拡張子の分 42 バイトを足しても 242 バイトで、OS の上限（255。Windows は UTF-16 の
/// 255 単位で、UTF-8 のバイト数より多くはならない）に収まる。
pub const STEM_MAX: usize = 200;

/// 置き先のファイル名から、一時ファイルの名前に入れる幹を作る。[`STEM_MAX`] 以下ならそのまま。長ければ先頭（文字の境目で切る）に
/// `~` と全体の FNV-1a（16 桁）を付けて [`STEM_MAX`] に収める（同じ先頭を持つ別の長い名前の一時ファイルと取り違えない）。
fn stem(name: &str) -> Cow<'_, str> {
    if name.len() <= STEM_MAX {
        return Cow::Borrowed(name);
    }
    let hash = name.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    let mut end = STEM_MAX - 17;
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    Cow::Owned(format!("{}~{hash:016x}", &name[..end]))
}

/// `name`（置き先のファイル名）の一時ファイルの名前（`.{幹}.{pid}-{通し番号}.pending~`。幹は [`STEM_MAX`] まで `name` そのまま）。
pub(crate) fn pending_name(name: &str) -> String {
    format!(
        ".{}.{}-{}.pending~",
        stem(name),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// 一時ファイル `.{幹}.{pid}-{通し番号}.pending~`（幹は [`STEM_MAX`] まで `name` そのまま）の名前か。`name` は置き先のファイル名。
/// 形が違うもの（別の置き先の一時ファイル・利用者のファイル）は含めない。
pub fn is_leftover_name(name: &str, found: &str) -> bool {
    let Some(rest) = found
        .strip_prefix('.')
        .and_then(|r| r.strip_prefix(&*stem(name)))
        .and_then(|r| r.strip_prefix('.'))
        .and_then(|r| r.strip_suffix(".pending~"))
    else {
        return false;
    };
    is_serial(rest)
}

/// 一時ファイルの名前なら、その置き先のファイル名（強制終了で残った一時ファイルを、置き先の名前の形で見分ける片付けのため）。
/// [`STEM_MAX`] より長い置き先の分は、置き先そのものでなく幹（切った名前）が返る。
pub fn leftover_target(found: &str) -> Option<&str> {
    let rest = found.strip_prefix('.')?.strip_suffix(".pending~")?;
    let (name, serial) = rest.rsplit_once('.')?;
    (!name.is_empty() && is_serial(serial)).then_some(name)
}

/// `{pid}-{通し番号}`（どちらも 1 桁以上の数字）。
fn is_serial(s: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
    s.split_once('-')
        .is_some_and(|(pid, nonce)| digits(pid) && digits(nonce))
}

/// 置き換えの失敗のうち、やり直しに値するもの。Windows の共有違反（32）・ロック違反（33）・アクセス拒否（5）: 同期の道具・ウイルス
/// 対策・Unity の .ylp の取り込み（読む間は他の書き換えを許さない）などが一時的にファイルを掴んでいるとき。アクセス拒否は本当の
/// 権限の不足でも出るが、そのときは待っても通らないだけ（待つのは [`BUSY_BUDGET`] まで）。Unix の `rename` は開いているファイルに
/// 妨げられないので、やり直さない。
#[cfg(windows)]
pub(crate) fn is_busy(e: &io::Error) -> bool {
    matches!(e.raw_os_error(), Some(5 | 32 | 33))
}
#[cfg(not(windows))]
pub(crate) fn is_busy(_: &io::Error) -> bool {
    false
}

/// やり直しの間に待つ合計の上限。保存は主のスレッドで回るので、長く待つと画面が固まって見える。掴んでいる側はたいてい 1 秒以内に手放す
/// ので 1.5 秒で諦め、本当の失敗として返す（待ちは 10・20・40…ms と倍にし、300 ms で頭打ち）。
pub(crate) const BUSY_BUDGET: Duration = Duration::from_millis(1500);

/// `op` を、`busy` と言える失敗のあいだ、合計 [`BUSY_BUDGET`] まで待ちながらやり直す。ほかの失敗は待たずに返す。
pub(crate) fn retry_busy<T>(
    busy: &dyn Fn(&io::Error) -> bool,
    sleep: &mut dyn FnMut(Duration),
    op: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    retry_busy_within(BUSY_BUDGET, busy, sleep, op)
}

/// [`retry_busy`] の、待つ合計の上限を渡す形（待ちを 2 つに分ける置換が使う）。上限まで待って通らなければ、最後の失敗を返す。
pub(crate) fn retry_busy_within<T>(
    budget: Duration,
    busy: &dyn Fn(&io::Error) -> bool,
    sleep: &mut dyn FnMut(Duration),
    mut op: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    let mut waited = Duration::ZERO;
    let mut next = Duration::from_millis(10);
    loop {
        match op() {
            Err(e) if busy(&e) && waited < budget => {
                let pause = next.min(budget - waited);
                sleep(pause);
                waited += pause;
                next = (next * 2).min(Duration::from_millis(300));
            }
            other => return other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new(tag: &str) -> Scratch {
            let p = std::env::temp_dir().join(format!(
                "yolu-io-atomic-{tag}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&p).unwrap();
            Scratch(p)
        }
        fn names(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(&self.0)
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

    #[test]
    fn replacing_writes_the_bytes_and_leaves_no_temp_file() {
        let s = Scratch::new("ok");
        let path = s.0.join("a.conf");
        replace_bytes(&path, b"one").unwrap();
        replace_bytes(&path, b"two").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        assert_eq!(s.names(), ["a.conf"]);
        // フォルダが無ければ、選んだときだけ作る
        let nested = s.0.join("x/y/b.conf");
        assert!(replace_bytes(&nested, b"b").is_err());
        let opts = ReplaceOptions {
            create_dirs: true,
            ..Default::default()
        };
        replace_with(&nested, &opts, |f| io::Write::write_all(f, b"b")).unwrap();
        assert_eq!(fs::read(&nested).unwrap(), b"b");
    }

    #[test]
    fn a_failure_midway_keeps_the_old_file_and_leaves_no_temp_file() {
        let s = Scratch::new("fail");
        let path = s.0.join("a.conf");
        replace_bytes(&path, b"old").unwrap();
        // 書く途中の失敗（`?` で抜ける）
        let error = replace_with(&path, &ReplaceOptions::default(), |f| {
            io::Write::write_all(f, b"half")?;
            Err(io::Error::other("書けない"))
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "書けない");
        // 置き換える前の失敗
        assert!(failing(|| replace_bytes(&path, b"new")).is_err());
        assert!(
            replace_bytes(&path, b"after").is_ok(),
            "失敗させるのは f の間だけ"
        );
        assert_eq!(fs::read(&path).unwrap(), b"after");
        assert_eq!(s.names(), ["a.conf"]);
    }

    #[test]
    fn a_false_verify_or_an_oversized_file_does_not_replace() {
        let s = Scratch::new("verify");
        let path = s.0.join("a.bin");
        replace_bytes(&path, b"old").unwrap();
        let seen = std::cell::RefCell::new(Vec::new());
        let check = |read: &[u8]| {
            seen.borrow_mut().push(read.to_vec());
            read == b"good"
        };
        let opts = ReplaceOptions {
            limit: Some(8),
            verify: Some(&check),
            create_dirs: false,
        };
        let error = replace_with(&path, &opts, |f| io::Write::write_all(f, b"bad")).unwrap_err();
        assert_eq!(rejected(&error), Some(Rejected::Mismatch));
        let error =
            replace_with(&path, &opts, |f| io::Write::write_all(f, b"too large!")).unwrap_err();
        assert_eq!(rejected(&error), Some(Rejected::TooLarge));
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert_eq!(s.names(), ["a.bin"]);
        replace_with(&path, &opts, |f| io::Write::write_all(f, b"good")).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"good");
        assert_eq!(
            seen.borrow().as_slice(),
            [b"bad".to_vec(), b"good".to_vec()]
        );
        assert_eq!(rejected(&io::Error::other("x")), None);
    }

    #[test]
    fn two_writers_in_one_process_do_not_collide_on_the_temp_name() {
        let s = Scratch::new("two");
        let path = s.0.join("same.conf");
        // 1 つ目の一時ファイルを開いたまま、2 つ目を書き切る
        let inner = replace_with(&path, &ReplaceOptions::default(), |f| {
            io::Write::write_all(f, b"outer")?;
            replace_bytes(&path, b"inner")
        });
        inner.unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"outer", "後に確定した方");
        assert_eq!(s.names(), ["same.conf"]);
        // スレッドから同時に
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || replace_bytes(&path, format!("{i}").as_bytes()))
            })
            .collect();
        for t in threads {
            t.join().unwrap().unwrap();
        }
        assert_eq!(s.names(), ["same.conf"]);
    }

    /// 共有違反を模す失敗（本物の判定は Windows の OS のエラー番号。ここでは種類で見分ける）。
    fn pretend_busy() -> io::Error {
        io::Error::from(io::ErrorKind::PermissionDenied)
    }
    fn is_pretend_busy(e: &io::Error) -> bool {
        e.kind() == io::ErrorKind::PermissionDenied
    }

    #[test]
    fn a_busy_replace_is_retried_until_it_goes_through_or_the_budget_runs_out() {
        let s = Scratch::new("busy");
        let path = s.0.join("a.conf");
        replace_bytes(&path, b"old").unwrap();
        // 3 回掴まれてから通る
        let (mut left, mut slept) = (3, Vec::new());
        replace_with_seams(
            &path,
            &ReplaceOptions::default(),
            |f| io::Write::write_all(f, b"new"),
            &mut Seams {
                rename: &mut |from, to| {
                    if left > 0 {
                        left -= 1;
                        Err(pretend_busy())
                    } else {
                        fs::rename(from, to)
                    }
                },
                busy: &is_pretend_busy,
                sleep: &mut |d| slept.push(d),
            },
        )
        .unwrap();
        assert_eq!(slept, [10, 20, 40].map(Duration::from_millis));
        assert_eq!(fs::read(&path).unwrap(), b"new");
        // ずっと掴まれている: 待ちの合計は予算ちょうどで、その失敗を返す。前のファイルのまま、一時ファイルも残らない
        let mut slept = Vec::new();
        let error = replace_with_seams(
            &path,
            &ReplaceOptions::default(),
            |f| io::Write::write_all(f, b"never"),
            &mut Seams {
                rename: &mut |_, _| Err(pretend_busy()),
                busy: &is_pretend_busy,
                sleep: &mut |d| slept.push(d),
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(slept.iter().sum::<Duration>(), BUSY_BUDGET);
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(s.names(), ["a.conf"]);
        // 本物の判定は Windows の共有違反・ロック違反・アクセス拒否だけ
        let os = |code| io::Error::from_raw_os_error(code);
        assert_eq!(
            [os(5), os(32), os(33), os(2)].map(|e| is_busy(&e)),
            [cfg!(windows), cfg!(windows), cfg!(windows), false]
        );
    }

    #[test]
    fn leftover_names_are_told_apart_from_other_files() {
        let name = pending_name("a.conf");
        assert!(is_leftover_name("a.conf", &name), "{name}");
        assert_eq!(leftover_target(&name), Some("a.conf"));
        assert_eq!(
            leftover_target(".yolupainter-1.2.3-x.exe.12-3.pending~"),
            Some("yolupainter-1.2.3-x.exe")
        );
        for other in [
            "a.conf",
            ".a.conf.12.pending~",
            ".a.conf.x-3.pending~",
            "a.conf.12-3.pending~",
            ".a.conf.12-3.pending",
            "..12-3.pending~",
        ] {
            assert_eq!(leftover_target(other), None, "{other}");
            assert!(!is_leftover_name("a.conf", other), "{other}");
        }
    }

    /// 置き先の名前の長さ（255 バイトまで）によらず、一時ファイルの名前が OS の上限に収まり、片付けの見分けも通る。
    #[test]
    fn a_long_target_name_gets_a_temp_name_within_the_os_limit() {
        // 200 バイトちょうどまではそのまま、超えたら切る
        let exact = "a".repeat(STEM_MAX - 5) + ".conf";
        assert_eq!(exact.len(), STEM_MAX);
        assert!(pending_name(&exact).starts_with(&format!(".{exact}.")));
        let over = "a".repeat(STEM_MAX - 4) + ".conf";
        assert!(!pending_name(&over).starts_with(&format!(".{over}.")));
        // ASCII と、3 バイトの文字（切る位置が文字の途中に当たる）で、上限いっぱいの名前
        let ascii = "a".repeat(250) + ".conf";
        let wide = "あ".repeat(83) + ".conf";
        let wide_shifted = "x".to_string() + &"あ".repeat(83) + ".conf";
        for name in [&ascii, &wide, &wide_shifted] {
            assert!(name.len() <= 255, "{}", name.len());
            let temp = pending_name(name);
            // pid・通し番号が最も長い形でも 255 に収まる
            let widest = format!(".{}.{}-{}.pending~", stem(name), u32::MAX, u64::MAX);
            assert!(temp.len() <= 255 && widest.len() <= 255, "{}", widest.len());
            assert!(is_leftover_name(name, &temp), "{temp}");
            assert_eq!(leftover_target(&temp), Some(&*stem(name)));
        }
        // 先頭が同じでも、別の置き先の一時ファイルは取り違えない（短い名前の一時ファイルも）
        let sibling = "a".repeat(250) + ".bak";
        assert!(!is_leftover_name(&ascii, &pending_name(&sibling)));
        assert!(!is_leftover_name(&ascii, &pending_name("a.conf")));
        assert!(!is_leftover_name("a.conf", &pending_name(&ascii)));
    }

    /// 前は一時ファイルの名前が 255 バイトを超え、置き先は作れるのに書けなかった名前（ほかの OS・ファイルシステムの上限は確かめていない）。
    #[cfg(unix)]
    #[test]
    fn a_target_name_near_the_os_limit_can_be_replaced() {
        let s = Scratch::new("long");
        for name in ["a".repeat(250) + ".conf", "あ".repeat(83) + ".conf"] {
            let path = s.0.join(&name);
            replace_bytes(&path, b"one").unwrap();
            replace_bytes(&path, b"two").unwrap();
            assert_eq!(fs::read(&path).unwrap(), b"two");
            let error = failing(|| replace_bytes(&path, b"three")).unwrap_err();
            assert_eq!(error.to_string(), "replace failed (test)");
            fs::remove_file(&path).unwrap();
            assert!(
                s.names().is_empty(),
                "一時ファイルが残っている: {:?}",
                s.names()
            );
        }
    }
}
