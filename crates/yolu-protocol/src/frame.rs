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
            FrameError::TooLarge(n) => write!(f, "枠の中身が大きすぎます（{n} バイト）"),
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

/// 命令を 1 つの枠（頭と中身）にする。
pub fn encode_message(message: &Message) -> Vec<u8> {
    let payload = message.encode_payload();
    encode_frame(message.kind() as u16, 0, &payload)
}

/// 枠を作る（知らない種類の命令を試すときにも使う）。
pub fn encode_frame(kind: u16, flags: u16, payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() <= MAX_PAYLOAD, "枠の中身が上限を超えます");
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// 命令を書く（書き終えるまで待つ）。
pub fn write_message(w: &mut impl Write, message: &Message) -> io::Result<()> {
    w.write_all(&encode_message(message))?;
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
