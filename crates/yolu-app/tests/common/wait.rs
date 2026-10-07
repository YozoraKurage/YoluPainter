//! 実際のファイル・ソケット・子のプロセスの待ち。上限は性能の合否ではなく停止した試験の検出用。
#![allow(dead_code)]
use std::time::Duration;

pub const WATCHDOG: Duration = Duration::from_secs(120);
