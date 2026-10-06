//! 試験が作る一時のフォルダ・ファイルを、試験が終わるときに片づける（Live Link の名前のファイルは `names`）。
//!
//! libtest は試験ごとにスレッドを分けるので、「この試験のスレッドが終わるとき」が試験の終わり。`clean_up_after_test`・`test_dir` が
//! 控えた物を、スレッドの終了の後始末（`thread_local` の Drop）が消す。試験が落ちた（panic した）ときも、スレッドの後始末は走る。
//! 常駐の描画スレッド（`gpu_thread::run`）はスレッドが終わらないので、ジョブの終わりに `sweep` する。
//! 失敗の調べのために残したいときは、環境変数 `YOLUPAINTER_KEEP_TEST_FILES=1` を付けて試験を回す。
//! 消すのは自分が控えた物だけ（フォルダは `remove_dir_all`。控えるのは試験が自分で名前を決めて作ったフォルダに限る）。
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Default)]
struct Sweeper(RefCell<Vec<PathBuf>>);

impl Sweeper {
    fn sweep(&self) {
        let keep = std::env::var_os("YOLUPAINTER_KEEP_TEST_FILES").is_some_and(|v| v != "0");
        for path in self.0.take() {
            if keep {
                continue;
            }
            // フォルダでもファイルでも（無ければ何もしない）
            if path.is_dir() {
                let _ = std::fs::remove_dir_all(&path);
            } else {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

impl Drop for Sweeper {
    fn drop(&mut self) {
        self.sweep();
    }
}

thread_local! {
    static SWEEPER: Sweeper = Sweeper::default();
}

/// この試験（のスレッド）が終わるとき、`path`（フォルダまたはファイル）を消す。試験が一時の物を作った所で 1 回呼ぶ。
pub fn clean_up_after_test(path: &Path) {
    let _ = SWEEPER.try_with(|s| s.0.borrow_mut().push(path.to_path_buf()));
}

/// 今のスレッドが控えた物をすぐ消す（スレッドが終わらない常駐の描画スレッドが、ジョブごとに呼ぶ）。
pub fn sweep() {
    let _ = SWEEPER.try_with(Sweeper::sweep);
}

/// 試験用の一時フォルダ（OS の一時フォルダの下。名前は用途・プロセス番号・通し番号で、並んで走る試験・プロセスと重ならない）を作り、
/// 試験が終わるとき消す。同じ名前の前の残りがあれば、消してから作る。
pub fn test_dir(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "yolu-test-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    clean_up_after_test(&dir);
    dir
}

/// ファイルの更新時刻を、少し前（10 分前）に書き換えて、その時刻を返す。「書き直さない」ことの確かめに使う: 前に控えた時刻と後の時刻を
/// 比べると、ファイルシステムの時刻の粒度（数ミリ秒）より早く書き直されたとき同じ値に見えて、書き直しを見逃す。少し前にしておけば、
/// 書き直されると今の時刻になるので、待たずに必ず食い違う。ずっと前にしないのは、製品に「古い（1 時間以上）ファイルは使うとき更新時刻を
/// 新しくする」規則（絵の覚え `library::cache` の `TOUCH_AFTER`）があり、それを起こしてしまうため。
pub fn backdate(path: &Path) -> std::time::SystemTime {
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(600);
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(old))
        .unwrap_or_else(|e| panic!("{}: 更新時刻を書き換えられない: {e}", path.display()));
    old
}
