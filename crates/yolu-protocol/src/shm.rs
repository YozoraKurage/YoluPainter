//! 共有メモリの画像: テクスチャセットの 1 チャンネルを、スタンドアロンが書き、Unity 側のブリッジが読むファイル 1 つ（両方が写像する）。
//!
//! 並び（リトルエンディアン）:
//! - 頭 128 バイト: 合言葉 "YLSHIMG\0"・並びの版・頭の大きさ・幅・高さ・タイルの大きさ・タイルの列と行の数・画素の形・セット・チャンネル・
//!   通し番号の位置・画素の位置・タイルのバイト数・全体のバイト数・書き手のプロセス番号・状態（0 = 作っている、1 = 使える、2 = 書き手が閉じた）。
//! - タイルごとの通し番号（u32、タイルの番号 = y × 列の数 + x）。偶数は書き終えた、奇数は書いている途中（seqlock）。
//! - 画素（4096 バイトの境目から）: タイルごとに tile_size × tile_size × 4 バイト（straight RGBA8、タイルの中は下の行から）。端のタイルも
//!   同じ大きさで、画像の外の所は使わない。
//!
//! 書き手は通し番号を奇数にしてから書き、偶数に戻す。読み手は写す前と後で番号を比べ、違えば（または奇数なら）そのタイルを「ちぎれた」として
//! 次に回す。どちらも待たない（描いている手を Unity の読みで止めない。Unity の主スレッドを書き手で止めない）。
//!
//! ファイルの置き場: Linux は /dev/shm（RAM の上）を先に試し、入らなければ（コンテナでは 64 MB しか無いことが多い）一時フォルダ。
//! Windows は一時フォルダ（FILE_ATTRIBUTE_TEMPORARY で、ディスクへの書き出しを控えさせる）。`YOLUPAINTER_LINK_SHM_DIR` で選べる。

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{fence, AtomicU32, Ordering};

use memmap2::{Mmap, MmapMut};

/// 頭の合言葉。
pub const IMAGE_MAGIC: [u8; 8] = *b"YLSHIMG\0";
/// 並びの版。
pub const IMAGE_LAYOUT_VERSION: u32 = 1;
/// 画素の形: straight RGBA8、タイルの中は下の行から。
pub const FORMAT_RGBA8_STRAIGHT: u32 = 1;
/// 頭のバイト数。
pub const HEADER_BYTES: usize = 128;
/// ファイルの名前の頭と尾（読み手はこれに合わないファイルを開かない）。
pub const FILE_PREFIX: &str = "yolupainter-link-";
pub const FILE_SUFFIX: &str = ".ylimg";

const STATE_CREATING: u32 = 0;
const STATE_READY: u32 = 1;
const STATE_CLOSED: u32 = 2;

const OFF_VERSION: usize = 8;
const OFF_HEADER_BYTES: usize = 12;
const OFF_WIDTH: usize = 16;
const OFF_HEIGHT: usize = 20;
const OFF_TILE: usize = 24;
const OFF_TILES_X: usize = 28;
const OFF_TILES_Y: usize = 32;
const OFF_FORMAT: usize = 36;
const OFF_SET: usize = 40;
const OFF_CHANNEL: usize = 44;
const OFF_SEQ: usize = 48;
const OFF_PIXELS: usize = 56;
const OFF_TILE_BYTES: usize = 64;
const OFF_TOTAL: usize = 72;
const OFF_PID: usize = 80;
const OFF_STATE: usize = 84;

/// タイルの大きさに使える値（16〜1024 の 2 の冪）。
pub fn valid_tile_size(tile_size: u32) -> bool {
    tile_size.is_power_of_two() && (16..=1024).contains(&tile_size)
}

/// 共有メモリの画像の失敗。
#[derive(Debug)]
pub enum ShmError {
    Io(io::Error),
    /// ファイルが決まりに合わない（何が）。
    Invalid(&'static str),
    /// タイルや写し先が範囲の外。
    OutOfRange(&'static str),
}

impl std::fmt::Display for ShmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShmError::Io(e) => write!(f, "共有メモリ: {e}"),
            ShmError::Invalid(what) => {
                write!(f, "共有メモリのファイルが決まりに合いません: {what}")
            }
            ShmError::OutOfRange(what) => write!(f, "範囲の外です: {what}"),
        }
    }
}

impl std::error::Error for ShmError {}

impl From<io::Error> for ShmError {
    fn from(e: io::Error) -> Self {
        ShmError::Io(e)
    }
}

/// 画像の大きさから決まる並び。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageLayout {
    pub width: u32,
    pub height: u32,
    pub tile_size: u32,
    pub tiles_x: u32,
    pub tiles_y: u32,
    pub seq_offset: usize,
    pub pixel_offset: usize,
    pub tile_bytes: usize,
    pub total_bytes: usize,
}

impl ImageLayout {
    pub fn new(width: u32, height: u32, tile_size: u32) -> Result<ImageLayout, ShmError> {
        if width == 0
            || height == 0
            || width > crate::message::MAX_TEXTURE_SIZE
            || height > crate::message::MAX_TEXTURE_SIZE
        {
            return Err(ShmError::Invalid("画像の大きさ"));
        }
        if !valid_tile_size(tile_size) {
            return Err(ShmError::Invalid("タイルの大きさ"));
        }
        let tiles_x = width.div_ceil(tile_size);
        let tiles_y = height.div_ceil(tile_size);
        let tiles = tiles_x as usize * tiles_y as usize;
        let seq_offset = HEADER_BYTES;
        let pixel_offset = (seq_offset + tiles * 4).div_ceil(4096) * 4096;
        let tile_bytes = tile_size as usize * tile_size as usize * 4;
        Ok(ImageLayout {
            width,
            height,
            tile_size,
            tiles_x,
            tiles_y,
            seq_offset,
            pixel_offset,
            tile_bytes,
            total_bytes: pixel_offset + tiles * tile_bytes,
        })
    }

    pub fn tile_count(&self) -> usize {
        self.tiles_x as usize * self.tiles_y as usize
    }

    pub fn tile_index(&self, x: u32, y: u32) -> Option<usize> {
        (x < self.tiles_x && y < self.tiles_y)
            .then(|| y as usize * self.tiles_x as usize + x as usize)
    }

    /// タイルのうち画像の中の所（左下の画素と幅・高さ）。
    pub fn tile_rect(&self, x: u32, y: u32) -> (u32, u32, u32, u32) {
        let x0 = x * self.tile_size;
        let y0 = y * self.tile_size;
        (
            x0,
            y0,
            self.tile_size.min(self.width - x0),
            self.tile_size.min(self.height - y0),
        )
    }
}

/// タイルを読んだ結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileRead {
    /// 書き終えた中身を写した。
    Complete,
    /// 写している間に書き手が書いた（写した中身は混ざっているかもしれない。次にもう一度読む）。
    Torn,
}

/// 置き場の候補（前から試す）。
pub fn candidate_dirs() -> Vec<PathBuf> {
    if let Some(dir) = std::env::var_os("YOLUPAINTER_LINK_SHM_DIR") {
        return vec![PathBuf::from(dir)];
    }
    let mut dirs = Vec::new();
    #[cfg(target_os = "linux")]
    {
        let shm = Path::new("/dev/shm");
        if shm.is_dir() {
            dirs.push(shm.to_path_buf());
        }
    }
    dirs.push(std::env::temp_dir());
    dirs
}

/// ファイルの名前（頭と尾を除いた所）に使える文字か。
pub fn valid_stem(stem: &str) -> bool {
    !stem.is_empty()
        && stem.len() <= 128
        && stem
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

fn open_options(write: bool) -> OpenOptions {
    let mut o = OpenOptions::new();
    o.read(true);
    if write {
        o.write(true).create_new(true);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // 読み・書き・消すのを共有する（相手が開いていても書き手が消せ、最後の写像が閉じたときに消える）
        o.share_mode(7);
        if write {
            o.attributes(0x100); // FILE_ATTRIBUTE_TEMPORARY
        }
    }
    o
}

fn read_u32(base: *const u8, off: usize) -> u32 {
    let mut b = [0u8; 4];
    unsafe { ptr::copy_nonoverlapping(base.add(off), b.as_mut_ptr(), 4) };
    u32::from_le_bytes(b)
}
fn read_u64(base: *const u8, off: usize) -> u64 {
    let mut b = [0u8; 8];
    unsafe { ptr::copy_nonoverlapping(base.add(off), b.as_mut_ptr(), 8) };
    u64::from_le_bytes(b)
}
fn atomic_at(base: *const u8, off: usize) -> &'static AtomicU32 {
    // 写像は 4096 の境目から始まり、off は 4 の倍数。写像より長く使わない（持ち主の型の寿命の中だけで使う）
    unsafe { &*(base.add(off) as *const AtomicU32) }
}

/// 書き手（スタンドアロン）。落とすとファイルを「閉じた」にして消す。
pub struct SharedImageWriter {
    path: PathBuf,
    map: MmapMut,
    layout: ImageLayout,
    set: u32,
    channel: u8,
    _file: File,
}

impl SharedImageWriter {
    /// 置き場の候補に順に作る（入らなければ次へ）。
    pub fn create(
        stem: &str,
        width: u32,
        height: u32,
        tile_size: u32,
        set: u32,
        channel: u8,
    ) -> Result<SharedImageWriter, ShmError> {
        let mut last = None;
        for dir in candidate_dirs() {
            match Self::create_in(&dir, stem, width, height, tile_size, set, channel) {
                Ok(w) => return Ok(w),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or(ShmError::Invalid("置き場が無い")))
    }

    /// dir に作る。場所はゼロを書いて確保する（tmpfs が満ちていたら、写像に触った時の SIGBUS ではなく、ここで失敗させる）。
    pub fn create_in(
        dir: &Path,
        stem: &str,
        width: u32,
        height: u32,
        tile_size: u32,
        set: u32,
        channel: u8,
    ) -> Result<SharedImageWriter, ShmError> {
        if !valid_stem(stem) {
            return Err(ShmError::Invalid("ファイルの名前"));
        }
        let layout = ImageLayout::new(width, height, tile_size)?;
        let path = dir.join(format!("{FILE_PREFIX}{stem}{FILE_SUFFIX}"));
        let mut file = open_options(true).open(&path)?;
        let allocated = (|| -> io::Result<()> {
            if cfg!(windows) {
                file.set_len(layout.total_bytes as u64)
            } else {
                let zeros = vec![0u8; 1 << 20];
                let mut left = layout.total_bytes;
                while left > 0 {
                    let n = left.min(zeros.len());
                    file.write_all(&zeros[..n])?;
                    left -= n;
                }
                file.flush()
            }
        })();
        if let Err(e) = allocated {
            drop(file);
            let _ = std::fs::remove_file(&path);
            return Err(ShmError::Io(e));
        }
        let mut map = match unsafe { MmapMut::map_mut(&file) } {
            Ok(m) => m,
            Err(e) => {
                drop(file);
                let _ = std::fs::remove_file(&path);
                return Err(ShmError::Io(e));
            }
        };
        let h = &mut map[..HEADER_BYTES];
        h[0..8].copy_from_slice(&IMAGE_MAGIC);
        let put32 =
            |h: &mut [u8], off: usize, v: u32| h[off..off + 4].copy_from_slice(&v.to_le_bytes());
        let put64 =
            |h: &mut [u8], off: usize, v: u64| h[off..off + 8].copy_from_slice(&v.to_le_bytes());
        put32(h, OFF_VERSION, IMAGE_LAYOUT_VERSION);
        put32(h, OFF_HEADER_BYTES, HEADER_BYTES as u32);
        put32(h, OFF_WIDTH, width);
        put32(h, OFF_HEIGHT, height);
        put32(h, OFF_TILE, tile_size);
        put32(h, OFF_TILES_X, layout.tiles_x);
        put32(h, OFF_TILES_Y, layout.tiles_y);
        put32(h, OFF_FORMAT, FORMAT_RGBA8_STRAIGHT);
        put32(h, OFF_SET, set);
        put32(h, OFF_CHANNEL, channel as u32);
        put64(h, OFF_SEQ, layout.seq_offset as u64);
        put64(h, OFF_PIXELS, layout.pixel_offset as u64);
        put64(h, OFF_TILE_BYTES, layout.tile_bytes as u64);
        put64(h, OFF_TOTAL, layout.total_bytes as u64);
        put32(h, OFF_PID, std::process::id());
        atomic_at(map.as_ptr(), OFF_STATE).store(STATE_READY, Ordering::Release);
        Ok(SharedImageWriter {
            path,
            map,
            layout,
            set,
            channel,
            _file: file,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn layout(&self) -> &ImageLayout {
        &self.layout
    }
    pub fn set(&self) -> u32 {
        self.set
    }
    pub fn channel(&self) -> u8 {
        self.channel
    }

    /// タイルを書く。f はタイルの領域（tile_size × tile_size × 4、下の行から、行は tile_size × 4 バイト）を受ける。
    pub fn write_tile(
        &mut self,
        x: u32,
        y: u32,
        f: impl FnOnce(&mut [u8]),
    ) -> Result<(), ShmError> {
        let i = self
            .layout
            .tile_index(x, y)
            .ok_or(ShmError::OutOfRange("タイル"))?;
        let base = self.map.as_mut_ptr();
        let seq = atomic_at(base, self.layout.seq_offset + i * 4);
        let s = seq.load(Ordering::Relaxed) & !1;
        seq.store(s.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        let slot = unsafe {
            std::slice::from_raw_parts_mut(
                base.add(self.layout.pixel_offset + i * self.layout.tile_bytes),
                self.layout.tile_bytes,
            )
        };
        f(slot);
        seq.store(s.wrapping_add(2), Ordering::Release);
        Ok(())
    }

    /// 画像全体（width × height × 4、下の行から）の中のタイルの所を書く。
    pub fn write_tile_from_image(&mut self, x: u32, y: u32, image: &[u8]) -> Result<(), ShmError> {
        let (w, h) = (self.layout.width as usize, self.layout.height as usize);
        if image.len() != w * h * 4 {
            return Err(ShmError::OutOfRange("画像の大きさ"));
        }
        let (x0, y0, tw, th) = self.layout.tile_rect(x, y);
        let ts = self.layout.tile_size as usize;
        self.write_tile(x, y, |slot| {
            for row in 0..th as usize {
                let src = ((y0 as usize + row) * w + x0 as usize) * 4;
                slot[row * ts * 4..row * ts * 4 + tw as usize * 4]
                    .copy_from_slice(&image[src..src + tw as usize * 4]);
            }
        })
    }
}

impl Drop for SharedImageWriter {
    fn drop(&mut self) {
        atomic_at(self.map.as_ptr(), OFF_STATE).store(STATE_CLOSED, Ordering::Release);
        let _ = std::fs::remove_file(&self.path);
    }
}

/// 読み手（Unity 側のブリッジ）。読むだけで写像し、ファイルには書かない。落とすとき、書き手がもう閉じていればファイルを消す
/// （Windows では写像している間は名前が消えずに残り得るので、後まで開いていた側が片付ける）。
pub struct SharedImageReader {
    path: PathBuf,
    map: Mmap,
    layout: ImageLayout,
    set: u32,
    channel: u8,
    writer_pid: u32,
}

impl SharedImageReader {
    /// 開いて頭を確かめる（名前の頭と尾・合言葉・版・大きさと並び・ファイルの長さ・状態）。
    pub fn open(path: &Path) -> Result<SharedImageReader, ShmError> {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(ShmError::Invalid("ファイルの名前"))?;
        let stem = name
            .strip_prefix(FILE_PREFIX)
            .and_then(|n| n.strip_suffix(FILE_SUFFIX))
            .ok_or(ShmError::Invalid("ファイルの名前"))?;
        if !valid_stem(stem) {
            return Err(ShmError::Invalid("ファイルの名前"));
        }
        let file = open_options(false).open(path)?;
        if !file.metadata()?.is_file() {
            return Err(ShmError::Invalid("普通のファイルでない"));
        }
        let map = unsafe { Mmap::map(&file)? };
        if map.len() < HEADER_BYTES {
            return Err(ShmError::Invalid("短すぎる"));
        }
        let base = map.as_ptr();
        if map[0..8] != IMAGE_MAGIC {
            return Err(ShmError::Invalid("合言葉"));
        }
        if read_u32(base, OFF_VERSION) != IMAGE_LAYOUT_VERSION {
            return Err(ShmError::Invalid("並びの版"));
        }
        if read_u32(base, OFF_HEADER_BYTES) as usize != HEADER_BYTES {
            return Err(ShmError::Invalid("頭の大きさ"));
        }
        if read_u32(base, OFF_FORMAT) != FORMAT_RGBA8_STRAIGHT {
            return Err(ShmError::Invalid("画素の形"));
        }
        let layout = ImageLayout::new(
            read_u32(base, OFF_WIDTH),
            read_u32(base, OFF_HEIGHT),
            read_u32(base, OFF_TILE),
        )?;
        if read_u32(base, OFF_TILES_X) != layout.tiles_x
            || read_u32(base, OFF_TILES_Y) != layout.tiles_y
            || read_u64(base, OFF_SEQ) != layout.seq_offset as u64
            || read_u64(base, OFF_PIXELS) != layout.pixel_offset as u64
            || read_u64(base, OFF_TILE_BYTES) != layout.tile_bytes as u64
            || read_u64(base, OFF_TOTAL) != layout.total_bytes as u64
        {
            return Err(ShmError::Invalid("並び"));
        }
        if map.len() < layout.total_bytes {
            return Err(ShmError::Invalid("ファイルの長さ"));
        }
        if atomic_at(base, OFF_STATE).load(Ordering::Acquire) == STATE_CREATING {
            return Err(ShmError::Invalid("まだ作っている途中"));
        }
        let channel = read_u32(base, OFF_CHANNEL);
        if channel >= crate::message::channel::COUNT as u32 {
            return Err(ShmError::Invalid("チャンネル"));
        }
        Ok(SharedImageReader {
            path: path.to_path_buf(),
            set: read_u32(base, OFF_SET),
            channel: channel as u8,
            writer_pid: read_u32(base, OFF_PID),
            map,
            layout,
        })
    }

    pub fn layout(&self) -> &ImageLayout {
        &self.layout
    }
    pub fn set(&self) -> u32 {
        self.set
    }
    pub fn channel(&self) -> u8 {
        self.channel
    }
    pub fn writer_pid(&self) -> u32 {
        self.writer_pid
    }
    /// 書き手がファイルを閉じたか（落ちたスタンドアロンは閉じないので、つながりの切れでも見る）。
    pub fn writer_closed(&self) -> bool {
        atomic_at(self.map.as_ptr(), OFF_STATE).load(Ordering::Acquire) == STATE_CLOSED
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// タイルの通し番号（偶数は書き終えた）。
    pub fn tile_sequence(&self, x: u32, y: u32) -> Option<u32> {
        let i = self.layout.tile_index(x, y)?;
        Some(atomic_at(self.map.as_ptr(), self.layout.seq_offset + i * 4).load(Ordering::Acquire))
    }

    /// タイルを dst へ写す。行 r（下から）は dst[origin + r × stride ..] へ。full が真ならタイルの全体（tile_size × tile_size）、
    /// 偽なら画像の中の所だけ（端のタイルは小さい）。
    pub fn read_tile_rows(
        &self,
        x: u32,
        y: u32,
        dst: &mut [u8],
        origin: usize,
        stride: usize,
        full: bool,
    ) -> Result<TileRead, ShmError> {
        let i = self
            .layout
            .tile_index(x, y)
            .ok_or(ShmError::OutOfRange("タイル"))?;
        let ts = self.layout.tile_size as usize;
        let (_, _, tw, th) = self.layout.tile_rect(x, y);
        let (cols, rows) = if full {
            (ts, ts)
        } else {
            (tw as usize, th as usize)
        };
        let row_bytes = cols * 4;
        if rows > 0 && origin + (rows - 1) * stride + row_bytes > dst.len() {
            return Err(ShmError::OutOfRange("写し先"));
        }
        let base = self.map.as_ptr();
        let seq = atomic_at(base, self.layout.seq_offset + i * 4);
        let before = seq.load(Ordering::Acquire);
        if before & 1 == 1 {
            return Ok(TileRead::Torn);
        }
        let src = unsafe { base.add(self.layout.pixel_offset + i * self.layout.tile_bytes) };
        for r in 0..rows {
            unsafe {
                ptr::copy_nonoverlapping(
                    src.add(r * ts * 4),
                    dst.as_mut_ptr().add(origin + r * stride),
                    row_bytes,
                );
            }
        }
        fence(Ordering::Acquire);
        let after = seq.load(Ordering::Relaxed);
        Ok(if before == after {
            TileRead::Complete
        } else {
            TileRead::Torn
        })
    }

    /// タイルを画像全体（width × height × 4、下の行から）の同じ所へ写す。
    pub fn read_tile_into_image(
        &self,
        x: u32,
        y: u32,
        image: &mut [u8],
    ) -> Result<TileRead, ShmError> {
        let (w, h) = (self.layout.width as usize, self.layout.height as usize);
        if image.len() != w * h * 4 {
            return Err(ShmError::OutOfRange("画像の大きさ"));
        }
        let (x0, y0, _, _) = self.layout.tile_rect(x, y);
        self.read_tile_rows(
            x,
            y,
            image,
            (y0 as usize * w + x0 as usize) * 4,
            w * 4,
            false,
        )
    }
}

impl Drop for SharedImageReader {
    fn drop(&mut self) {
        if self.writer_closed() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("ylshm-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn layout_pads_edge_tiles_and_aligns_pixels() {
        let l = ImageLayout::new(300, 129, 128).unwrap();
        assert_eq!((l.tiles_x, l.tiles_y), (3, 2));
        assert_eq!(l.pixel_offset % 4096, 0);
        assert_eq!(l.tile_rect(2, 1), (256, 128, 44, 1));
        assert_eq!(l.total_bytes, l.pixel_offset + 6 * 128 * 128 * 4);
        assert!(ImageLayout::new(0, 1, 128).is_err());
        assert!(ImageLayout::new(16, 16, 100).is_err());
    }

    #[test]
    fn a_written_tile_reads_back_into_the_image() {
        let dir = temp_dir();
        let mut w = SharedImageWriter::create_in(&dir, "t1", 200, 100, 64, 7, 0).unwrap();
        let mut image = vec![0u8; 200 * 100 * 4];
        for (i, p) in image.chunks_exact_mut(4).enumerate() {
            p.copy_from_slice(&[(i % 251) as u8, (i % 7) as u8, 3, 255]);
        }
        for ty in 0..2 {
            for tx in 0..4 {
                w.write_tile_from_image(tx, ty, &image).unwrap();
            }
        }
        let r = SharedImageReader::open(w.path()).unwrap();
        assert_eq!((r.set(), r.channel()), (7, 0));
        let mut out = vec![0u8; 200 * 100 * 4];
        for ty in 0..2 {
            for tx in 0..4 {
                assert_eq!(
                    r.read_tile_into_image(tx, ty, &mut out).unwrap(),
                    TileRead::Complete
                );
            }
        }
        assert!(out == image);
        assert_eq!(r.tile_sequence(3, 1), Some(2));
        let path = w.path().to_path_buf();
        drop(w);
        assert!(r.writer_closed());
        drop(r);
        assert!(!path.exists());
    }

    #[test]
    fn a_tile_being_written_reads_as_torn() {
        let dir = temp_dir();
        let mut w = SharedImageWriter::create_in(&dir, "t2", 64, 64, 64, 0, 0).unwrap();
        let path = w.path().to_path_buf();
        let r = SharedImageReader::open(&path).unwrap();
        let mut out = vec![0u8; 64 * 64 * 4];
        w.write_tile(0, 0, |slot| {
            // 書いている途中に読む
            slot[0] = 9;
            assert_eq!(
                r.read_tile_into_image(0, 0, &mut out).unwrap(),
                TileRead::Torn
            );
        })
        .unwrap();
        assert_eq!(
            r.read_tile_into_image(0, 0, &mut out).unwrap(),
            TileRead::Complete
        );
        assert_eq!(out[0], 9);
    }

    #[test]
    fn the_reader_refuses_foreign_or_broken_files() {
        let dir = temp_dir();
        let bad = dir.join("other.bin");
        std::fs::write(&bad, [0u8; 256]).unwrap();
        assert!(matches!(
            SharedImageReader::open(&bad),
            Err(ShmError::Invalid(_))
        ));
        let named = dir.join(format!("{FILE_PREFIX}broken{FILE_SUFFIX}"));
        std::fs::write(&named, [0u8; 256]).unwrap();
        assert!(matches!(
            SharedImageReader::open(&named),
            Err(ShmError::Invalid("合言葉"))
        ));
        assert!(SharedImageWriter::create_in(&dir, "../escape", 16, 16, 16, 0, 0).is_err());
    }
}
