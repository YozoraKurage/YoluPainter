//! 枠: 頭 12 バイト（"YLNK"・種類 u16・印 u16・中身の長さ u32）と中身。印は今は 0（知らない印は読み飛ばす）。
//! 頭の合言葉が違う・長さが上限を超えるときは、流れの区切りが分からなくなったので、そのつながりを閉じる。

use std::io::{self, Read, Write};

use crate::message::Message;

/// 枠の頭の合言葉。
pub const MAGIC: [u8; 4] = *b"YLNK";
/// 枠の頭のバイト数。
pub const HEADER_LEN: usize = 12;
/// 中身の上限（512 MiB。100 万頂点のメッシュを数十個送れる）。
pub const MAX_PAYLOAD: usize = 512 << 20;

/// 読んだ枠（中身はまだ読んでいない）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub kind: u16,
    pub flags: u16,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn decode(&self) -> Result<Message, crate::wire::DecodeError> {
        Message::decode(self.kind, &self.payload)
    }
}

/// 枠を読めない（このつながりを閉じる）。
#[derive(Debug)]
pub enum FrameError {
    /// 頭の合言葉が違う。
    BadMagic,
    /// 中身が上限を超える。
    TooLarge(usize),
    Io(io::Error),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::BadMagic => write!(f, "枠の頭が YLNK ではありません"),
            FrameError::TooLarge(n) => write!(
                f,
                "枠の中身が大きすぎます（{} MiB。上限 {} MiB）",
                n.div_ceil(1 << 20),
                MAX_PAYLOAD >> 20
            ),
            FrameError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<io::Error> for FrameError {
    fn from(e: io::Error) -> Self {
        FrameError::Io(e)
    }
}

/// 命令を 1 つの枠（頭と中身）にする。中身が上限（`MAX_PAYLOAD`）を超えないと分かっている小さな命令（挨拶・断り・誤り・Bye・
/// タイルの知らせなど）と試験のための口で、超えるとパニックする。モデル・ポーズ・画素を運ぶ命令を送る口は `try_encode_message` を使う。
pub fn encode_message(message: &Message) -> Vec<u8> {
    try_encode_message(message).expect("枠の中身が上限を超えます")
}

/// 命令を 1 つの枠にする。中身が上限（`MAX_PAYLOAD`）を超えるなら `FrameError::TooLarge`（相手の読み手は、上限を超える枠で区切りを失って
/// つながりを閉じるので、送る前に断る）。パニックしない。
pub fn try_encode_message(message: &Message) -> Result<Vec<u8>, FrameError> {
    try_encode_message_within(message, MAX_PAYLOAD)
}

/// `try_encode_message` の、中身の上限を（`MAX_PAYLOAD` 以下に）狭められる形。送り口が、相手に渡す量をもっと小さく絞るときと、試験が
/// 巨大な命令を作らずに断りの流れを確かめるときに使う。
pub fn try_encode_message_within(message: &Message, limit: usize) -> Result<Vec<u8>, FrameError> {
    let payload = message.encode_payload();
    if payload.len() > limit.min(MAX_PAYLOAD) {
        return Err(FrameError::TooLarge(payload.len()));
    }
    try_encode_frame(message.kind() as u16, 0, &payload)
}

/// 枠を作る（知らない種類の命令を試すときにも使う）。中身が上限を超えるとパニックする（`try_encode_frame`）。
pub fn encode_frame(kind: u16, flags: u16, payload: &[u8]) -> Vec<u8> {
    try_encode_frame(kind, flags, payload).expect("枠の中身が上限を超えます")
}

/// 枠を作る。中身が上限を超えるなら、中身を写す前に `FrameError::TooLarge`。
pub fn try_encode_frame(kind: u16, flags: u16, payload: &[u8]) -> Result<Vec<u8>, FrameError> {
    if payload.len() > MAX_PAYLOAD {
        return Err(FrameError::TooLarge(payload.len()));
    }
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

/// 命令を書く（書き終えるまで待つ）。中身が上限を超えるなら、何も書かずに `InvalidInput`。
pub fn write_message(w: &mut impl Write, message: &Message) -> io::Result<()> {
    write_message_within(w, message, MAX_PAYLOAD)
}

/// `write_message` の、中身の上限を（`MAX_PAYLOAD` 以下に）狭められる形（`try_encode_message_within`）。上限を超えるなら、何も書かずに `InvalidInput`。
/// 試験が、512 MiB の命令を作らずに、書く口の断りを確かめるのに使う。
pub fn write_message_within(w: &mut impl Write, message: &Message, limit: usize) -> io::Result<()> {
    let frame = try_encode_message_within(message, limit)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    w.write_all(&frame)?;
    w.flush()
}

/// 読んだバイトを溜めて枠を切り出す。受けの時間切れ（WouldBlock・TimedOut）で読みが途中で返っても、読んだ所を失わない。
#[derive(Default)]
pub struct FrameReader {
    buf: Vec<u8>,
    /// 溜めたバイトの数（buf の先頭から）。
    filled: usize,
}

/// 読みの結果。
#[derive(Debug, PartialEq, Eq)]
pub enum Fill {
    /// 何バイトか読めた。
    Read(usize),
    /// 今は読むものが無い（時間切れ・ノンブロッキング）。
    Idle,
    /// 相手が閉じた。
    Closed,
}

impl FrameReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// 溜めた中から、そろった枠を 1 つ取り出す。
    pub fn next_frame(&mut self) -> Result<Option<Frame>, FrameError> {
        if self.filled < HEADER_LEN {
            return Ok(None);
        }
        if self.buf[0..4] != MAGIC {
            return Err(FrameError::BadMagic);
        }
        let kind = u16::from_le_bytes([self.buf[4], self.buf[5]]);
        let flags = u16::from_le_bytes([self.buf[6], self.buf[7]]);
        let len = u32::from_le_bytes(self.buf[8..12].try_into().unwrap()) as usize;
        if len > MAX_PAYLOAD {
            return Err(FrameError::TooLarge(len));
        }
        if self.filled < HEADER_LEN + len {
            return Ok(None);
        }
        let payload = self.buf[HEADER_LEN..HEADER_LEN + len].to_vec();
        self.buf.copy_within(HEADER_LEN + len..self.filled, 0);
        self.filled -= HEADER_LEN + len;
        // 大きなモデルを読んだ後に何百 MB も抱えたままにしない
        if self.buf.len() > 4 << 20 && self.filled < 1 << 20 {
            self.buf.truncate((self.filled + 64 * 1024).max(64 * 1024));
            self.buf.shrink_to_fit();
        }
        Ok(Some(Frame {
            kind,
            flags,
            payload,
        }))
    }

    /// 読める分だけ読んで溜める。大きな枠は中身の長さを知ってから一度に場所を取って読む。
    pub fn fill(&mut self, r: &mut impl Read) -> Result<Fill, FrameError> {
        let want = if self.filled >= HEADER_LEN {
            let len = u32::from_le_bytes(self.buf[8..12].try_into().unwrap()) as usize;
            if self.buf[0..4] != MAGIC {
                return Err(FrameError::BadMagic);
            }
            if len > MAX_PAYLOAD {
                return Err(FrameError::TooLarge(len));
            }
            (HEADER_LEN + len).max(self.filled + 64 * 1024)
        } else {
            self.filled + 64 * 1024
        };
        if self.buf.len() < want {
            self.buf.resize(want, 0);
        }
        match r.read(&mut self.buf[self.filled..]) {
            Ok(0) => Ok(Fill::Closed),
            Ok(n) => {
                self.filled += n;
                Ok(Fill::Read(n))
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) =>
            {
                Ok(Fill::Idle)
            }
            Err(e) => Err(FrameError::Io(e)),
        }
    }

    /// 枠が 1 つそろうまで読む（ブロックする読み手用。時間切れなら None）。
    pub fn read_frame(&mut self, r: &mut impl Read) -> Result<Option<Frame>, FrameError> {
        loop {
            if let Some(frame) = self.next_frame()? {
                return Ok(Some(frame));
            }
            match self.fill(r)? {
                Fill::Read(_) => continue,
                Fill::Idle => return Ok(None),
                Fill::Closed => {
                    return Err(FrameError::Io(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "相手がつながりを閉じました",
                    )))
                }
            }
        }
    }

    /// 溜めているバイトの数（試験用）。
    pub fn pending(&self) -> usize {
        self.filled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Hello, Message};

    #[test]
    fn a_payload_over_the_limit_is_refused_without_panicking() {
        // 上限を 1 バイト超えると、中身を写す前に断る（相手の読み手は上限を超える枠でつながりを閉じる）。
        // 上限ちょうどの枠は 512 MiB を写すので、ここでは作らない（0 で埋めた領域はまだ物理メモリを使わない）
        let over = vec![0u8; MAX_PAYLOAD + 1];
        let e = try_encode_frame(0x7fff, 0, &over).unwrap_err();
        assert!(
            matches!(e, FrameError::TooLarge(n) if n == MAX_PAYLOAD + 1),
            "{e}"
        );
        assert!(e.to_string().contains("512 MiB"), "理由に上限が入る: {e}");
        // 書く口は、何も書かずに InvalidInput（パニックしない）。512 MiB の命令は作らず、上限を狭めて同じ道を通す
        let hello = Message::Hello(Hello {
            min_version: 1,
            max_version: 1,
            agent: "試験".into(),
            features: 0,
            auth: None,
            versions: None,
        });
        let size = hello.encode_payload().len();
        let mut sink = Vec::new();
        let e = write_message_within(&mut sink, &hello, size - 1).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput, "{e}");
        assert!(e.to_string().contains("512 MiB"), "理由に上限が入る: {e}");
        assert!(sink.is_empty(), "何も書かない");
        // 上限ちょうどは書く。上限を省いた口（既定は 512 MiB）も、小さな命令は書く
        write_message_within(&mut sink, &hello, size).unwrap();
        assert_eq!(sink.len(), HEADER_LEN + size);
        let mut plain = Vec::new();
        write_message(&mut plain, &hello).unwrap();
        assert_eq!(plain, sink);
        // 上限 0 でも、中身の無い命令は書ける（Bye の中身は空）
        let mut bye = Vec::new();
        write_message_within(&mut bye, &Message::Bye, 0).unwrap();
        assert_eq!(bye.len(), HEADER_LEN);
    }

    /// 1 バイトずつ・時間切れを挟みながら返す読み手。
    struct Trickle {
        data: Vec<u8>,
        pos: usize,
        tick: usize,
    }
    impl Read for Trickle {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.tick += 1;
            if self.tick.is_multiple_of(3) {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "時間切れ"));
            }
            if self.pos >= self.data.len() {
                return Ok(0);
            }
            buf[0] = self.data[self.pos];
            self.pos += 1;
            Ok(1)
        }
    }

    #[test]
    fn frames_survive_timeouts_in_the_middle() {
        let a = Message::Hello(Hello {
            min_version: 1,
            max_version: 1,
            agent: "試験".into(),
            features: 0,
            auth: None,
            versions: None,
        });
        let mut data = encode_message(&a);
        data.extend(encode_message(&Message::Bye));
        let mut src = Trickle {
            data,
            pos: 0,
            tick: 0,
        };
        let mut reader = FrameReader::new();
        let mut got = Vec::new();
        for _ in 0..1000 {
            match reader.read_frame(&mut src) {
                Ok(Some(f)) => got.push(f.decode().unwrap()),
                Ok(None) => {}
                Err(_) => break,
            }
        }
        assert_eq!(got, vec![a, Message::Bye]);
        assert_eq!(reader.pending(), 0);
    }

    #[test]
    fn bad_magic_and_huge_lengths_close_the_stream() {
        let mut reader = FrameReader::new();
        let mut src: &[u8] = b"XXXX\x01\x00\x00\x00\x00\x00\x00\x00";
        reader.fill(&mut src).unwrap();
        assert!(matches!(reader.next_frame(), Err(FrameError::BadMagic)));

        let mut reader = FrameReader::new();
        let mut header = MAGIC.to_vec();
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&0u16.to_le_bytes());
        header.extend_from_slice(&u32::MAX.to_le_bytes());
        let mut src: &[u8] = &header;
        reader.fill(&mut src).unwrap();
        assert!(matches!(reader.next_frame(), Err(FrameError::TooLarge(_))));
    }
}
