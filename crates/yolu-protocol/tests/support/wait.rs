//! 実ソケット・子プロセスの待ち。上限は性能の合否ではなく停止した試験の検出用。
#![allow(dead_code)]
use std::process::{Child, Command};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use yolu_protocol::{link::connect_and_greet, Connection, ConnectionReader, Received, Welcome};

pub const WATCHDOG: Duration = Duration::from_secs(120);

pub struct ChildGuard(pub Child);
impl std::ops::Deref for ChildGuard {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.0
    }
}
impl std::ops::DerefMut for ChildGuard {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
impl ChildGuard {
    pub fn spawn(command: &mut Command) -> Self {
        Self(command.spawn().expect("子を起動する"))
    }
    pub fn finish(&mut self) -> std::process::ExitStatus {
        let deadline = Instant::now() + WATCHDOG;
        loop {
            if let Some(status) = self.try_wait().expect("子の終了状態") {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "終了通知を待つ子が生存したまま: pid={}",
                self.id()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    pub fn check(&mut self, what: &str) {
        let status = self.try_wait().expect("子の状態を調べる");
        assert!(
            status.is_none(),
            "{what} を待つ途中で子が終了: pid={}, 状態={status:?}",
            self.id()
        );
    }
}

pub fn connect(
    name: &str,
    mut child: Option<&mut ChildGuard>,
) -> (Connection, MessageReader, Welcome) {
    let deadline = Instant::now() + WATCHDOG;
    loop {
        if let Some(child) = child.as_deref_mut() {
            child.check("接続・挨拶");
        }
        match connect_and_greet(name, "試験のブリッジ") {
            Ok((conn, reader, welcome)) => {
                return (conn.clone(), MessageReader::new(conn, reader), welcome)
            }
            Err(e) => {
                assert!(
                    Instant::now() < deadline,
                    "接続・挨拶が完了しない: 子pid={:?}, 最後のエラー={e}",
                    child.as_ref().map(|c| c.id())
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

pub struct MessageReader {
    rx: Receiver<Result<Received, String>>,
}
impl MessageReader {
    pub fn new(conn: Connection, mut reader: ConnectionReader) -> Self {
        let (tx, rx) = mpsc::channel();
        // Windows のソケット読み取りも、試験側の期限付き受信で監視する。
        std::thread::spawn(move || loop {
            let result = reader.next(&conn).map_err(|e| e.to_string());
            if matches!(result, Ok(Received::Idle)) {
                continue;
            }
            let failed = result.is_err()
                || matches!(result, Ok(Received::Message(yolu_protocol::Message::Bye)));
            if tx.send(result).is_err() || failed {
                break;
            }
        });
        Self { rx }
    }
    #[track_caller]
    pub fn next(&mut self, what: &str, mut child: Option<&mut ChildGuard>) -> Received {
        let deadline = Instant::now() + WATCHDOG;
        loop {
            match self.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(message)) => return message,
                Ok(Err(e)) => panic!(
                    "{what} の受信失敗: {e}; 子の状態={:?}",
                    child.as_deref_mut().map(|c| c.try_wait())
                ),
                Err(mpsc::RecvTimeoutError::Disconnected) => panic!("{what}: 受信スレッドが終了"),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // 子の終了直後も、OSの受信バッファに最後の通知が残り得る。
                    // 終了だけでは失敗にせず、受信スレッドがEOFまで排出するのを待つ。
                    let status = child.as_deref_mut().map(|c| c.try_wait());
                    assert!(
                        Instant::now() < deadline,
                        "{what} が来ない: 子pid={:?}, 状態={status:?}, 受信スレッドのチャンネルは接続中",
                        child.as_ref().map(|c| c.id())
                    );
                }
            }
        }
    }
}
