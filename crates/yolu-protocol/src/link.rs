//! つなぎ方: interprocess の local socket。スタンドアロンが待ち受け、Unity 側のブリッジがつなぐ。
//!
//! - Unix（Linux・macOS）: 自分だけのフォルダ（`private::link_dir`、0700）の中の Unix ソケット `<名前>.sock`（0600）。Linux の抽象名前空間の
//!   ソケットは権限を持たず、同じ PC のどのユーザーもつなげるので使わない。
//! - Windows: 名前付きパイプ `\\.\pipe\yolupainter-<ユーザーの印>-<名前>`。自分だけを許す DACL を付け、遠くの PC からのつなぎは断る
//!   （interprocess の既定）。ユーザーの印を名前に入れるので、同じ PC の別のユーザーが同じ名前で待ち受けても重ならない。
//! - どちらも、つないだ後の挨拶で鍵（`auth`）を確かめる。ソケット・パイプの権限だけに頼らず、名前を先に取った別のプログラムにも
//!   鍵とモデルを渡さない（ブリッジは返事の証しを確かめてからモデルを送る）。
//!
//! 1 つのつながりを、読む側（1 つのスレッド）と書く側（どのスレッドからでも。枠が混ざらないよう錠で 1 つずつ）に分けて使う。
//! Windows の interprocess は名前付きパイプを重ねた I/O で開くので、読みと書きを別のスレッドで同時にしてよい。Windows には受けの
//! 時間切れが無いので、読むスレッドを止めるのは「Bye を送る → 相手が閉じる」か、相手のプロセスが終わったとき。

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(unix)]
use interprocess::local_socket::GenericFilePath;
#[cfg(windows)]
use interprocess::local_socket::GenericNamespaced;
use interprocess::local_socket::{prelude::*, Listener, ListenerOptions, Name, Stream};

use crate::auth::{random_bytes, same_bytes, HelloCheck, LinkKey, ServerKey};
use crate::frame::{encode_frame, encode_message, FrameError, FrameReader};
use crate::message::{
    ErrorCode, ErrorMessage, Hello, HelloAuth, Kind, Message, Reject, RejectCode, Welcome,
    MIN_PROTOCOL_VERSION, PROTOCOL_VERSION,
};
use crate::wire::DecodeError;

/// 既定のつなぎ先の名前。
pub const DEFAULT_LINK_NAME: &str = "yolupainter-livelink";

/// 挨拶を断る理由の文（鍵が無い・合わない）。
const MISSING_KEY_TEXT: &str = "このブリッジは Live Link の鍵に対応していません";
const WRONG_KEY_TEXT: &str = "Live Link の鍵が合いません";
const REPLAYED_TEXT: &str = "同じ挨拶を 2 度受けました";

/// 挨拶（最初の 1 つ）を待つ時間（Unix。Windows には受けの時間切れが無い。`ConnectionReader::next_within`）。
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// つなぎ先の名前に使える形（1〜64 文字の英数字と . _ -）。
pub fn valid_link_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'_' || c == b'-')
}

fn invalid_name() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "つなぎ先の名前は 1〜64 文字の英数字と . _ - です",
    )
}

/// Unix ソケットのパスの上限（sun_path は macOS 104・Linux 108 バイト）。
#[cfg(unix)]
const MAX_SOCKET_PATH: usize = 100;

/// つなぎ先の名前のソケットのファイル（スタンドアロンが置く所。自分だけのフォルダの中）。
#[cfg(unix)]
pub fn socket_path(name: &str) -> io::Result<PathBuf> {
    socket_in(&crate::private::link_dir()?, name)
}

/// ブリッジがつなぐソケットのファイル（鍵のあるフォルダの中。作らない）。
#[cfg(unix)]
fn find_socket_path(name: &str) -> io::Result<PathBuf> {
    socket_in(&crate::private::find_link_dir(name)?, name)
}

#[cfg(unix)]
fn socket_in(dir: &std::path::Path, name: &str) -> io::Result<PathBuf> {
    if !valid_link_name(name) {
        return Err(invalid_name());
    }
    let path = dir.join(format!("{name}.sock"));
    if path.as_os_str().len() > MAX_SOCKET_PATH {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "ソケットのパスが長すぎます（{} バイト。上限 {MAX_SOCKET_PATH}）",
                path.as_os_str().len()
            ),
        ));
    }
    Ok(path)
}

#[cfg(unix)]
fn socket_name(name: &str) -> io::Result<Name<'static>> {
    socket_path(name)?.to_fs_name::<GenericFilePath>()
}

/// ブリッジがつなぐ名前（Unix は鍵のあるフォルダのソケット。Windows は `socket_name` と同じ）。
#[cfg(unix)]
fn client_name(name: &str) -> io::Result<Name<'static>> {
    find_socket_path(name)?.to_fs_name::<GenericFilePath>()
}

#[cfg(not(unix))]
fn client_name(name: &str) -> io::Result<Name<'static>> {
    socket_name(name)
}

/// 名前付きパイプの名前（ユーザーの印を入れる）。
#[cfg(windows)]
pub fn pipe_name(name: &str) -> io::Result<String> {
    if !valid_link_name(name) {
        return Err(invalid_name());
    }
    Ok(format!(
        "yolupainter-{}-{name}",
        crate::private::win::user_tag()?
    ))
}

#[cfg(windows)]
fn socket_name(name: &str) -> io::Result<Name<'static>> {
    pipe_name(name)?.to_ns_name::<GenericNamespaced>()
}

#[cfg(not(any(unix, windows)))]
fn socket_name(_name: &str) -> io::Result<Name<'static>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "この OS では Live Link を使えません",
    ))
}

/// 待ち受ける側（スタンドアロン）。待ち受けている間、鍵のファイルを置き、やめると消す。
///
/// 同じ名前で 2 つめが待ち受けようとすると `AddrInUse` で失敗する（Unix は名前ごとのロックのファイル、Windows はパイプの最初の
/// 1 つめの印）。落ちたスタンドアロンが残した古いソケットは、ロックが取れれば誰も待っていないと分かるので片付けて使う。
pub struct Server {
    listener: Listener,
    key: Arc<ServerKey>,
    key_path: PathBuf,
    /// 名前のロック（Unix。閉じると外れる）。
    _lock: Option<std::fs::File>,
}

impl Server {
    /// 待ち受ける。polling なら accept は来ていなければすぐに WouldBlock を返す（止める合図を見回るため。来たつながりは待つ読み書き）。
    pub fn bind(name: &str, polling: bool) -> io::Result<Server> {
        if !valid_link_name(name) {
            return Err(invalid_name());
        }
        let key_path = crate::auth::key_path(name)?;
        let lock = lock_name(name)?;
        // 落ちたスタンドアロンが残した共有メモリのファイルを片付ける（Windows の置き場は OS の掃除の対象でない）
        crate::shm::sweep_stale_images();
        // 鍵は待ち受けを始める前に置く。落ちたスタンドアロンは古い鍵を残すので、先にソケットを作ると、引き継ぐ新しいスタンドアロンが
        // 鍵を置き換えるまでの間に、ブリッジが「古い鍵 + 新しいソケット」の組を見つけて挨拶し、本物のスタンドアロンに断られる。
        // ロックを取った後なので、先に待ち受けている別のスタンドアロンの鍵は壊れない。待ち受けを始められなければ、置いた鍵は消す
        let key = ServerKey::new(LinkKey::generate()?);
        key.key().write_file(&key_path)?;
        let listener = match create_listener(name, polling) {
            Ok(l) => l,
            Err(e) => {
                let _ = std::fs::remove_file(&key_path);
                return Err(e);
            }
        };
        Ok(Server {
            listener,
            key: Arc::new(key),
            key_path,
            _lock: lock,
        })
    }

    /// 来たつながりを 1 つ受ける（挨拶はまだ。`accept` で鍵を確かめる）。
    pub fn accept(&self) -> io::Result<Stream> {
        use interprocess::local_socket::traits::Listener as _;
        self.listener.accept()
    }

    /// 挨拶を確かめる鍵（つながりごとのスレッドへ渡す）。
    pub fn key(&self) -> Arc<ServerKey> {
        self.key.clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.key_path);
    }
}

#[cfg(unix)]
fn lock_name(name: &str) -> io::Result<Option<std::fs::File>> {
    use std::os::unix::io::AsRawFd;
    let dir = crate::private::link_dir()?;
    let path = dir.join(format!("{name}.lock"));
    let file = match crate::private::create_private_file(&path, false) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let f = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)?;
            crate::private::check_private_file(&f, &path)?;
            f
        }
        Err(e) => return Err(e),
    };
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let err = io::Error::last_os_error();
        return Err(if err.kind() == io::ErrorKind::WouldBlock {
            io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("「{name}」ではほかのスタンドアロンが待ち受けています"),
            )
        } else {
            err
        });
    }
    // 取れた = 誰も待っていない。前の（落ちた）スタンドアロンのソケットが残っていれば片付ける
    let socket = socket_path(name)?;
    if let Ok(m) = std::fs::symlink_metadata(&socket) {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        if m.file_type().is_socket() && m.uid() == unsafe { libc::geteuid() } {
            let _ = std::fs::remove_file(&socket);
        }
    }
    Ok(Some(file))
}

/// Windows: 名前ごとのファイルを、ほかの誰も開けない形（共有なし）で開いておく。落ちたプロセスのハンドルは OS が閉じるので、古い印は残らない。
/// 名前付きパイプの「最初の 1 つ」の印（FILE_FLAG_FIRST_PIPE_INSTANCE）だけに頼らないのは、2 つ目の待ち受けの失敗を同じ形で試験できるように。
#[cfg(windows)]
fn lock_name(name: &str) -> io::Result<Option<std::fs::File>> {
    use std::os::windows::fs::OpenOptionsExt;
    let path = crate::private::link_dir()?.join(format!("{name}.lock"));
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(&path)
    {
        Ok(f) => Ok(Some(f)),
        // ERROR_SHARING_VIOLATION
        Err(e) if e.raw_os_error() == Some(32) => Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("「{name}」ではほかのスタンドアロンが待ち受けています"),
        )),
        Err(e) => Err(e),
    }
}

#[cfg(not(any(unix, windows)))]
fn lock_name(_name: &str) -> io::Result<Option<std::fs::File>> {
    Ok(None)
}

#[cfg(unix)]
fn create_listener(name: &str, polling: bool) -> io::Result<Listener> {
    use interprocess::local_socket::ListenerNonblockingMode;
    use interprocess::os::unix::local_socket::ListenerOptionsExt;
    let options = |with_mode: bool| -> io::Result<ListenerOptions<'static>> {
        let mut o = ListenerOptions::new().name(socket_name(name)?);
        if polling {
            o = o.nonblocking(ListenerNonblockingMode::Accept);
        }
        if with_mode {
            o = o.mode(0o600);
        }
        Ok(o)
    };
    match options(true)?.create_sync() {
        // 権限を選べない OS（macOS など）では、0700 のフォルダの中に置くことで守る
        Err(e) if e.kind() == io::ErrorKind::Unsupported => options(false)?.create_sync(),
        other => other,
    }
}

#[cfg(windows)]
fn create_listener(name: &str, polling: bool) -> io::Result<Listener> {
    use interprocess::local_socket::ListenerNonblockingMode;
    use interprocess::os::windows::{
        local_socket::ListenerOptionsExt, security_descriptor::SecurityDescriptor,
    };
    let sddl = crate::private::win::private_sddl(false)?;
    let sd = SecurityDescriptor::deserialize(
        &widestring::U16CString::from_str(sddl)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
    )?;
    let mut o = ListenerOptions::new()
        .name(socket_name(name)?)
        .security_descriptor(sd);
    if polling {
        o = o.nonblocking(ListenerNonblockingMode::Accept);
    }
    o.create_sync()
}

#[cfg(not(any(unix, windows)))]
fn create_listener(name: &str, _polling: bool) -> io::Result<Listener> {
    ListenerOptions::new()
        .name(socket_name(name)?)
        .create_sync()
}

/// つなぐ（ブリッジ）。相手がいなければすぐに失敗する。Unix では、ソケットのファイルが自分のものであることを確かめてからつなぎ、
/// つないだ相手のプロセスが自分と同じユーザーであることも確かめる。
pub fn connect(name: &str) -> io::Result<Stream> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        let path = find_socket_path(name)?;
        let m = std::fs::symlink_metadata(&path)?;
        if !m.file_type().is_socket() || m.uid() != unsafe { libc::geteuid() } {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{} は自分のソケットではありません", path.display()),
            ));
        }
    }
    let stream = Stream::connect(client_name(name)?)?;
    check_peer(&stream)?;
    Ok(stream)
}

/// つないだ相手のプロセスが自分と同じユーザーか（Unix。分からない OS では確かめない。Windows はパイプの DACL が守る）。
fn check_peer(stream: &Stream) -> io::Result<()> {
    #[cfg(unix)]
    {
        if let Ok(creds) = stream.peer_creds() {
            if let Some(euid) = creds.euid() {
                if euid != unsafe { libc::geteuid() } {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "つないだ相手が別のユーザーのプロセスです",
                    ));
                }
            }
        }
    }
    let _ = stream;
    Ok(())
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
    /// 相手が鍵を知らない（ブリッジが、返事の証しが合わないスタンドアロンを使わない）。
    Untrusted(String),
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkError::Io(e) => write!(f, "{e}"),
            LinkError::Frame(e) => write!(f, "{e}"),
            LinkError::Rejected(r) => write!(f, "相手が断りました: {}", r.text),
            LinkError::Protocol(t) | LinkError::Untrusted(t) => write!(f, "{t}"),
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

    /// 次の命令を、時間内に待つ（来なければ `Idle`）。Unix は受けの時間切れを使う。Windows の名前付きパイプには受けの時間切れが無く、
    /// ノンブロッキングの印（PIPE_NOWAIT）は Microsoft が使わないよう勧める古い仕組みなので使わない: Windows では相手が来るか閉じるまで待つ
    /// （挨拶を送らない接続が「つなげる Unity は 1 つ」の枠を塞がないよう、枠は挨拶が済んでから取る。`accept_with` の claim）。
    pub fn next_within(
        &mut self,
        reply: &Connection,
        within: Duration,
    ) -> Result<Received, LinkError> {
        self.set_timeout(Some(within));
        let result = self.next(reply);
        self.set_timeout(None);
        result
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
                "プロトコルの版が合いません（Unity 側 {}〜{}、スタンドアロン {}〜{}）",
                hello.min_version, hello.max_version, MIN_PROTOCOL_VERSION, PROTOCOL_VERSION
            ),
        })
    }
}

/// 挨拶の鍵を確かめた結果の断り（確かめられれば None）。
fn auth_reject(key: &ServerKey, hello: &Hello) -> Option<Reject> {
    let text = match key.check_hello(hello.auth.as_ref()) {
        HelloCheck::Ok => return None,
        HelloCheck::Missing => MISSING_KEY_TEXT,
        HelloCheck::Wrong => WRONG_KEY_TEXT,
        HelloCheck::Replayed => REPLAYED_TEXT,
    };
    Some(Reject {
        code: RejectCode::Unauthorized,
        text: text.to_owned(),
    })
}

/// 挨拶を待って読む（来ない・最初が Hello でないなら失敗）。
fn read_hello(
    conn: &Connection,
    reader: &mut ConnectionReader,
    timeout: Duration,
) -> Result<Hello, LinkError> {
    let hello = match reader.next_within(conn, timeout)? {
        Received::Message(Message::Hello(h)) => h,
        Received::Message(other) => {
            let _ = conn.send(&error_message(
                ErrorCode::UnexpectedCommand,
                other.kind() as u16,
                "最初の命令は Hello です".into(),
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
    Ok(hello)
}

/// 挨拶が済んだ（鍵と版が合った）相手を受けてよいかを決める関数（スタンドアロンが「つなげる Unity は 1 つ」の枠を取る）。
/// 断るなら理由を返す。鍵の合わない相手にも、挨拶を送らない相手にも呼ばれない。
pub type Claim<'a> = &'a dyn Fn(&Hello) -> Result<(), Reject>;

/// スタンドアロン: 来たつながりの挨拶を受け、鍵を確かめ、版を決めて返す（鍵が合わない・版が合わなければ断って閉じる）。
/// 鍵を先に確かめるので、鍵を知らない相手には版の範囲も教えない。
pub fn accept(
    stream: Stream,
    agent: &str,
    session: u64,
    key: &ServerKey,
) -> Result<(Connection, ConnectionReader, Hello), LinkError> {
    accept_with(stream, agent, session, key, HANDSHAKE_TIMEOUT, &|_| Ok(()))
}

/// `accept`（挨拶を待つ時間と、受けてよいかの確かめ `claim` を選べる）。鍵と版が合ってから `claim` を呼び、断られたらその理由を返して閉じる
/// （ほかの Unity とつながっている、など。つながっていることは、鍵を知っている相手にだけ教える）。
pub fn accept_with(
    stream: Stream,
    agent: &str,
    session: u64,
    key: &ServerKey,
    timeout: Duration,
    claim: Claim<'_>,
) -> Result<(Connection, ConnectionReader, Hello), LinkError> {
    check_peer(&stream)?;
    let (conn, mut reader) = Connection::new(stream);
    let hello = read_hello(&conn, &mut reader, timeout)?;
    if let Some(reject) = auth_reject(key, &hello) {
        let _ = conn.send(&Message::Reject(reject.clone()));
        return Err(LinkError::Rejected(reject));
    }
    let version = match negotiate(&hello) {
        Ok(v) => v,
        Err(reject) => {
            let _ = conn.send(&Message::Reject(reject.clone()));
            return Err(LinkError::Rejected(reject));
        }
    };
    if let Err(reject) = claim(&hello) {
        let _ = conn.send(&Message::Reject(reject.clone()));
        return Err(LinkError::Rejected(reject));
    }
    let nonce = hello.auth.as_ref().map(|a| a.nonce).unwrap_or_default();
    conn.send(&Message::Welcome(Welcome {
        version,
        agent: agent.to_owned(),
        session,
        features: 0,
        proof: Some(key.key().welcome_proof(&nonce, version, session)),
    }))?;
    Ok((conn, reader, hello))
}

/// ブリッジ: 鍵を読み、つないで挨拶し、返事を待つ。返事の証し（スタンドアロンも同じ鍵を知っている）が合わなければ、モデルを渡さずに失敗する。
pub fn connect_and_greet(
    name: &str,
    agent: &str,
) -> Result<(Connection, ConnectionReader, Welcome), LinkError> {
    let key = LinkKey::load(name)?;
    let stream = connect(name)?;
    let (conn, mut reader) = Connection::new(stream);
    let nonce = random_bytes()?;
    conn.send(&Message::Hello(Hello {
        min_version: MIN_PROTOCOL_VERSION,
        max_version: PROTOCOL_VERSION,
        agent: agent.to_owned(),
        features: 0,
        auth: Some(HelloAuth {
            nonce,
            proof: key.hello_proof(&nonce),
        }),
    }))?;
    let welcome = match reader.next_within(&conn, HANDSHAKE_TIMEOUT)? {
        Received::Message(Message::Welcome(w)) => w,
        Received::Message(Message::Reject(r)) => return Err(LinkError::Rejected(r)),
        Received::Idle => return Err(LinkError::Protocol("挨拶の返事が来ません".into())),
        other => {
            return Err(LinkError::Protocol(format!(
                "挨拶の返事の代わりに {other:?} が来ました"
            )))
        }
    };
    match &welcome.proof {
        None => {
            return Err(LinkError::Untrusted(
                "スタンドアロンが鍵を確かめない古い版です".into(),
            ))
        }
        Some(p)
            if !same_bytes(
                p,
                &key.welcome_proof(&nonce, welcome.version, welcome.session),
            ) =>
        {
            return Err(LinkError::Untrusted(
                "つないだ相手が Live Link の鍵を知りません".into(),
            ))
        }
        Some(_) => {}
    }
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
