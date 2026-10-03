//! つなぎ方: interprocess の local socket（Linux は抽象名前空間の Unix ソケット、Windows は名前付きパイプ `\\.\pipe\<名前>`）。
//! スタンドアロンが待ち受け、Unity 側のブリッジがつなぐ。
//!
//! 1 つのつながりを、読む側（1 つのスレッド）と書く側（どのスレッドからでも。枠が混ざらないよう錠で 1 つずつ）に分けて使う。
//! Windows の interprocess は名前付きパイプを重ねた I/O で開くので、読みと書きを別のスレッドで同時にしてよい。Windows には受けの
//! 時間切れが無いので、読むスレッドを止めるのは「Bye を送る → 相手が閉じる」か、相手のプロセスが終わったとき。

use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use interprocess::local_socket::{
    prelude::*, GenericNamespaced, Listener, ListenerOptions, Name, Stream,
};

use crate::frame::{encode_frame, encode_message, FrameError, FrameReader};
use crate::message::{
    ErrorCode, ErrorMessage, Hello, Kind, Message, Reject, RejectCode, Welcome,
    MIN_PROTOCOL_VERSION, PROTOCOL_VERSION,
};
use crate::wire::DecodeError;

/// 既定のつなぎ先の名前。
pub const DEFAULT_LINK_NAME: &str = "yolupainter-livelink";

/// つなぎ先の名前に使える形（1〜64 文字の英数字と . _ -）。
pub fn valid_link_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'_' || c == b'-')
}

fn socket_name(name: &str) -> io::Result<Name<'static>> {
    if !valid_link_name(name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "つなぎ先の名前は 1〜64 文字の英数字と . _ - です",
        ));
    }
    name.to_owned().to_ns_name::<GenericNamespaced>()
}

/// 待ち受ける（スタンドアロン）。
pub fn listen(name: &str) -> io::Result<Listener> {
    ListenerOptions::new()
        .name(socket_name(name)?)
        .create_sync()
}

/// 待ち受ける。accept は来ていなければすぐに WouldBlock を返す（止める合図を見回るため。来たつながりは待つ読み書き）。
pub fn listen_polling(name: &str) -> io::Result<Listener> {
    use interprocess::local_socket::ListenerNonblockingMode;
    ListenerOptions::new()
        .name(socket_name(name)?)
        .nonblocking(ListenerNonblockingMode::Accept)
        .create_sync()
}

/// つなぐ（ブリッジ）。相手がいなければすぐに失敗する。
pub fn connect(name: &str) -> io::Result<Stream> {
    Stream::connect(socket_name(name)?)
}

/// つながりの失敗。
#[derive(Debug)]
pub enum LinkError {
    Io(io::Error),
    Frame(FrameError),
    /// 相手が断った。
    Rejected(Reject),
    /// 決まりと違う流れ（挨拶の前に別の命令が来た など）。
    Protocol(String),
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkError::Io(e) => write!(f, "{e}"),
            LinkError::Frame(e) => write!(f, "{e}"),
            LinkError::Rejected(r) => write!(f, "相手が断りました: {}", r.text),
            LinkError::Protocol(t) => write!(f, "{t}"),
        }
    }
}

impl std::error::Error for LinkError {}

impl From<io::Error> for LinkError {
    fn from(e: io::Error) -> Self {
        LinkError::Io(e)
    }
}
impl From<FrameError> for LinkError {
    fn from(e: FrameError) -> Self {
        match e {
            FrameError::Io(e) => LinkError::Io(e),
            e => LinkError::Frame(e),
        }
    }
}

/// 書く側（複製して別のスレッドから使える）。
#[derive(Clone)]
pub struct Connection {
    stream: Arc<Stream>,
    write_lock: Arc<Mutex<()>>,
}

impl Connection {
    fn new(stream: Stream) -> (Connection, ConnectionReader) {
        let stream = Arc::new(stream);
        (
            Connection {
                stream: stream.clone(),
                write_lock: Arc::new(Mutex::new(())),
            },
            ConnectionReader {
                stream,
                frames: FrameReader::new(),
            },
        )
    }

    /// 命令を送る（書き終えるまで待つ）。
    pub fn send(&self, message: &Message) -> io::Result<()> {
        self.send_frame(&encode_message(message))
    }

    /// 作った枠をそのまま送る。
    pub fn send_frame(&self, frame: &[u8]) -> io::Result<()> {
        let _guard = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = &*self.stream;
        s.write_all(frame)?;
        s.flush()
    }

    /// 試験用: 種類の番号を選んで生の枠を送る（知らない命令を試す）。
    pub fn send_raw(&self, kind: u16, payload: &[u8]) -> io::Result<()> {
        self.send_frame(&encode_frame(kind, 0, payload))
    }
}

/// 受けたもの。
#[derive(Debug, PartialEq)]
pub enum Received {
    Message(Message),
    /// 知らない種類の命令（`Error` を返して捨てた）。
    Unknown(u16),
    /// 読めない中身（`Error` を返して捨てた）。
    Malformed(u16, DecodeError),
    /// 時間切れで何も来なかった。
    Idle,
}

/// 読む側（1 つのスレッドで使う）。
pub struct ConnectionReader {
    stream: Arc<Stream>,
    frames: FrameReader,
}

impl ConnectionReader {
    /// 受けの時間切れ（Windows では効かないので、無視する）。
    pub fn set_timeout(&self, timeout: Option<Duration>) {
        let _ = self.stream.set_recv_timeout(timeout);
    }

    /// 次の命令を待つ。知らない・読めない命令には reply で `Error` を返し、`Unknown`・`Malformed` を返す。
    pub fn next(&mut self, reply: &Connection) -> Result<Received, LinkError> {
        let mut s = &*self.stream;
        let frame = match self.frames.read_frame(&mut s)? {
            Some(f) => f,
            None => return Ok(Received::Idle),
        };
        match frame.decode() {
            Ok(m) => Ok(Received::Message(m)),
            Err(DecodeError::UnknownKind(kind)) => {
                let _ = reply.send(&error_message(
                    ErrorCode::UnknownCommand,
                    kind,
                    format!(
                        "この版（{PROTOCOL_VERSION}）では知らない命令です（種類 0x{kind:04x}）"
                    ),
                ));
                Ok(Received::Unknown(kind))
            }
            Err(e) => {
                let _ = reply.send(&error_message(
                    ErrorCode::Malformed,
                    frame.kind,
                    format!("中身を読めません: {e}"),
                ));
                Ok(Received::Malformed(frame.kind, e))
            }
        }
    }
}

/// 誤りの命令を作る。
pub fn error_message(code: ErrorCode, kind: u16, text: String) -> Message {
    Message::Error(ErrorMessage { code, kind, text })
}

/// 受けた命令の向きが違えば、返す `Error`。
pub fn wrong_direction(message: &Message, receiver_is_standalone: bool) -> Option<Message> {
    use crate::message::Direction;
    let ok = match message.kind().direction() {
        Direction::Both => true,
        Direction::ToStandalone => receiver_is_standalone,
        Direction::ToUnity => !receiver_is_standalone,
    };
    (!ok).then(|| {
        error_message(
            ErrorCode::UnexpectedCommand,
            message.kind() as u16,
            format!("{:?} はこちらへ送る命令ではありません", message.kind()),
        )
    })
}

/// 版の取り決め: 両方の読める一番新しい版。重ならなければ断りの中身。
pub fn negotiate(hello: &Hello) -> Result<u16, Reject> {
    let lo = hello.min_version.max(MIN_PROTOCOL_VERSION);
    let hi = hello.max_version.min(PROTOCOL_VERSION);
    if lo <= hi {
        Ok(hi)
    } else {
        Err(Reject {
            code: RejectCode::VersionMismatch,
            text: format!(
                "プロトコルの版が合いません（Unity 側 {}〜{}、スタンドアロン {}〜{}）。どちらかを更新してください",
                hello.min_version, hello.max_version, MIN_PROTOCOL_VERSION, PROTOCOL_VERSION
            ),
        })
    }
}

/// スタンドアロン: 来たつながりの挨拶を受け、版を決めて返す（合わなければ断って閉じる）。
pub fn accept(
    stream: Stream,
    agent: &str,
    session: u64,
) -> Result<(Connection, ConnectionReader, Hello), LinkError> {
    let (conn, mut reader) = Connection::new(stream);
    reader.set_timeout(Some(Duration::from_secs(10)));
    let hello = match reader.next(&conn)? {
        Received::Message(Message::Hello(h)) => h,
        Received::Message(other) => {
            let _ = conn.send(&error_message(
                ErrorCode::UnexpectedCommand,
                other.kind() as u16,
                "最初は Hello を送ってください".into(),
            ));
            return Err(LinkError::Protocol(format!(
                "挨拶の前に {:?} が来ました",
                other.kind()
            )));
        }
        Received::Idle => return Err(LinkError::Protocol("挨拶が来ません".into())),
        Received::Unknown(_) | Received::Malformed(..) => {
            return Err(LinkError::Protocol("挨拶を読めません".into()))
        }
    };
    reader.set_timeout(None);
    match negotiate(&hello) {
        Ok(version) => {
            conn.send(&Message::Welcome(Welcome {
                version,
                agent: agent.to_owned(),
                session,
                features: 0,
            }))?;
            Ok((conn, reader, hello))
        }
        Err(reject) => {
            let _ = conn.send(&Message::Reject(reject.clone()));
            Err(LinkError::Rejected(reject))
        }
    }
}

/// ブリッジ: つないで挨拶し、返事を待つ。
pub fn connect_and_greet(
    name: &str,
    agent: &str,
) -> Result<(Connection, ConnectionReader, Welcome), LinkError> {
    let stream = connect(name)?;
    let (conn, mut reader) = Connection::new(stream);
    conn.send(&Message::Hello(Hello {
        min_version: MIN_PROTOCOL_VERSION,
        max_version: PROTOCOL_VERSION,
        agent: agent.to_owned(),
        features: 0,
    }))?;
    reader.set_timeout(Some(Duration::from_secs(10)));
    let welcome = match reader.next(&conn)? {
        Received::Message(Message::Welcome(w)) => w,
        Received::Message(Message::Reject(r)) => return Err(LinkError::Rejected(r)),
        Received::Idle => return Err(LinkError::Protocol("挨拶の返事が来ません".into())),
        other => {
            return Err(LinkError::Protocol(format!(
                "挨拶の返事の代わりに {other:?} が来ました"
            )))
        }
    };
    reader.set_timeout(None);
    if !(MIN_PROTOCOL_VERSION..=PROTOCOL_VERSION).contains(&welcome.version) {
        return Err(LinkError::Protocol(format!(
            "スタンドアロンが読めない版 {} を選びました",
            welcome.version
        )));
    }
    Ok((conn, reader, welcome))
}

/// 種類が向きに合うか（試験・ログ用）。
pub fn kind_name(kind: u16) -> String {
    Kind::from_u16(kind)
        .map(|k| format!("{k:?}"))
        .unwrap_or_else(|| format!("0x{kind:04x}"))
}
