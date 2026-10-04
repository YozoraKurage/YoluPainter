//! スタンドアロン側の手伝い: テクスチャセット（マテリアル 1 つ）のチャンネルごとの共有メモリを作り、知らせの命令を作る。

use std::time::{SystemTime, UNIX_EPOCH};

use crate::message::{ChannelImage, Message, TextureSet, Tile, TilesChanged};
use crate::shm::{SharedImageWriter, ShmError};

/// 出しているテクスチャセット 1 つ。落とすと共有メモリのファイルを消す。
pub struct PublishedSet {
    pub set: u32,
    pub generation: u32,
    pub material: u32,
    pub name: String,
    images: Vec<SharedImageWriter>,
    width: u32,
    height: u32,
    tile_size: u32,
}

impl PublishedSet {
    /// チャンネルごとに共有メモリを作る。ファイルの名前は、プロセス（番号と、起動ごとの乱数）・つながり・セット・チャンネル・作り直しの
    /// 番号で重ならない。乱数を入れるのは、落ちたプロセスのファイルが一時フォルダに残り、後で同じプロセス番号（Windows はすぐ使い回す）の
    /// 書き手が同じ名前を作れなくなる（作るのは新しいファイルだけ）のを避けるため。
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        session: u64,
        set: u32,
        generation: u32,
        material: u32,
        name: &str,
        width: u32,
        height: u32,
        tile_size: u32,
        channels: &[u8],
    ) -> Result<PublishedSet, ShmError> {
        static NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut images = Vec::with_capacity(channels.len());
        for &ch in channels {
            let stem = format!(
                "p{}-r{:08x}-s{session}-t{set}-c{ch}-n{nonce}",
                std::process::id(),
                process_token()
            );
            images.push(SharedImageWriter::create(
                &stem, width, height, tile_size, set, ch,
            )?);
        }
        Ok(PublishedSet {
            set,
            generation,
            material,
            name: name.to_owned(),
            images,
            width,
            height,
            tile_size,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn tile_size(&self) -> u32 {
        self.tile_size
    }

    /// TextureSet の知らせ。
    pub fn announce(&self) -> Message {
        Message::TextureSet(TextureSet {
            set: self.set,
            generation: self.generation,
            material: self.material,
            name: self.name.clone(),
            width: self.width,
            height: self.height,
            tile_size: self.tile_size,
            channels: self
                .images
                .iter()
                .map(|i| ChannelImage {
                    channel: i.channel(),
                    path: i.path().to_string_lossy().into_owned(),
                })
                .collect(),
        })
    }

    pub fn image_mut(&mut self, channel: u8) -> Option<&mut SharedImageWriter> {
        self.images.iter_mut().find(|i| i.channel() == channel)
    }

    /// 変わったタイルの知らせ（上限ごとに分ける）。
    pub fn tiles_changed(&self, channel: u8, tiles: &[Tile]) -> Vec<Message> {
        let stamp_us = now_us();
        tiles
            .chunks(crate::message::MAX_TILES_PER_MESSAGE)
            .map(|chunk| {
                Message::TilesChanged(TilesChanged {
                    set: self.set,
                    channel,
                    stamp_us,
                    tiles: chunk.to_vec(),
                })
            })
            .collect()
    }
}

/// 起動ごとに変わる 32 ビットの乱数（共有メモリのファイルの名前に入れる）。
fn process_token() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    static TOKEN: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *TOKEN.get_or_init(|| {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        );
        h.write_u32(std::process::id());
        h.finish() as u32
    })
}

/// UNIX 時刻のマイクロ秒（遅れを測るためだけ）。
pub fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}
