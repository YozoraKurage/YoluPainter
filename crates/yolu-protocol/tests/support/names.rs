//! Live Link の名前（試験ごとに重ならない名前）。待ち受けると、鍵・ソケット・ロックのファイルが Live Link のフォルダに置かれ、
//! ロックのファイルは製品が消さない（名前ごとの `flock` の置き場）。試験が終わるとき、その名前のファイルを消す。
//!
//! 「試験が終わるとき」は、この試験のスレッドの終わり（libtest は試験ごとにスレッドを分ける）。サーバー・子のプロセスを持っている
//! 試験の本体が終わって、持ち物が片づいた後にスレッドの後始末が走る。失敗の調べのために残したいときは、環境変数
//! `YOLUPAINTER_KEEP_TEST_FILES=1` を付けて試験を回す。
#![allow(dead_code)]
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Default)]
struct Sweeper(RefCell<Vec<PathBuf>>);

impl Drop for Sweeper {
    fn drop(&mut self) {
        if std::env::var_os("YOLUPAINTER_KEEP_TEST_FILES").is_some_and(|v| v != "0") {
            return;
        }
        for path in self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

thread_local! {
    static SWEEPER: Sweeper = Sweeper::default();
}

/// `prefix-tag-プロセス番号-通し番号` の名前。この試験が終わるとき、鍵・ソケット・ロックのファイルを消す。
pub fn unique_name(prefix: &str, tag: &str) -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    let name = format!(
        "{prefix}-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    );
    if let Ok(dir) = yolu_protocol::private::link_dir() {
        let _ = SWEEPER.try_with(|s| {
            let mut files = s.0.borrow_mut();
            for extension in ["lock", "key", "sock"] {
                files.push(dir.join(format!("{name}.{extension}")));
            }
        });
    }
    name
}
