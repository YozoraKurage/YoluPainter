//! ライブラリのファイル 1 つを見る（別のスレッドで走る）: 中身の札（SHA-256）・画像の画素の札・寸法・層の数・チャンネル・置けない理由・
//! サムネイル。同じ中身の絵はディスクのキャッシュから読み、作った絵はそこへ覚える。読めないファイルは、止まらずに理由を返す。
use std::path::PathBuf;

use yolu_core::smart::SmartKind;
use yolu_io::library as files;

use super::cache::Cache;
use super::image as png;
use super::service::Cancel;
use super::{reason, LibInfo};
use crate::lang::Lang;
use crate::shelf::{
    from_cached, image_picture, inspect_smart_kind, remembered, to_cached, Block, Inspected,
    ItemKind, Unreadable,
};

/// 見るときの上限（試験で小さくする）。
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// 読むファイルの大きさ（バイト）。
    pub read: u64,
    /// スマート素材のサムネイルのために展開してよい画素の量（バイト）。
    pub preview: u64,
    /// 画像のサムネイルを作る画素数。
    pub thumb_pixels: u64,
}

/// 見るファイル。
#[derive(Clone, Debug)]
pub struct Target {
    pub root: PathBuf,
    pub rel: String,
    pub kind: files::Kind,
    pub len: u64,
}

fn unreadable(e: &yolu_io::Error) -> Unreadable {
    Unreadable::pair(&reason(Lang::Ja, e), &reason(Lang::En, e))
}

fn broken(kind: ItemKind, why: Unreadable) -> LibInfo {
    LibInfo {
        kind,
        inspected: Inspected::bare(Some(Block::Unreadable(why))),
        sha256: String::new(),
        content: String::new(),
    }
}

fn problem(p: png::Problem) -> Unreadable {
    match p {
        png::Problem::Unreadable => Unreadable::pair("PNG として読めません", "Not a readable PNG"),
        png::Problem::Size => Unreadable::pair(
            "画像の 1 辺は 1〜8192 です",
            "Image sides must be 1 to 8192",
        ),
    }
}

fn line<'a>(info: &'a str, key: &str) -> Option<&'a str> {
    info.lines()
        .find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
}

/// ファイルを見る。ブラシ・マテリアルは中身を読まない（置けない種類として並べるだけ）。
pub fn probe(target: &Target, limits: Limits, cache: Option<&Cache>, cancel: &Cancel) -> LibInfo {
    match target.kind {
        files::Kind::Brush => LibInfo {
            kind: ItemKind::Brush,
            inspected: Inspected::bare(Some(Block::Kind(ItemKind::Brush))),
            sha256: String::new(),
            content: String::new(),
        },
        files::Kind::Material => LibInfo {
            kind: ItemKind::Material,
            inspected: Inspected::bare(Some(Block::Kind(ItemKind::Material))),
            sha256: String::new(),
            content: String::new(),
        },
        files::Kind::Image => probe_image(target, limits, cache, cancel),
        files::Kind::Smart => probe_smart(target, limits, cache, cancel),
    }
}

fn too_large() -> Unreadable {
    Unreadable::pair("大きすぎます", "Too large")
}

fn hashed(target: &Target, limits: Limits, cancel: &Cancel) -> Result<String, Unreadable> {
    if target.len > limits.read {
        return Err(too_large());
    }
    files::hash_file(&target.root, &target.rel, limits.read, Some(cancel.flag()))
        .map(|(hash, _)| hash)
        .map_err(|e| unreadable(&e))
}

fn probe_image(target: &Target, limits: Limits, cache: Option<&Cache>, cancel: &Cancel) -> LibInfo {
    let sha = match hashed(target, limits, cancel) {
        Ok(sha) => sha,
        Err(why) => return broken(ItemKind::Image, why),
    };
    let key = Cache::key("library-image", &sha);
    if let Some(found) = cache.and_then(|c| c.get(&key)) {
        let content = line(&found.info, "content").unwrap_or("").to_owned();
        let (inspected, _) = from_cached(found);
        return LibInfo {
            kind: ItemKind::Image,
            inspected,
            sha256: sha,
            content,
        };
    }
    let bytes = match files::read(&target.root, &target.rel, limits.read, Some(cancel.flag())) {
        Ok(b) => b,
        Err(e) => return broken(ItemKind::Image, unreadable(&e)),
    };
    let (w, h) = match png::dimensions(&bytes) {
        Ok(d) => d,
        Err(p) => return broken(ItemKind::Image, problem(p)),
    };
    let mut inspected = Inspected::bare(None);
    inspected.width = w;
    inspected.height = h;
    // 大きすぎる画像は、寸法だけ（画素の札・サムネイルは作らない。使うときには展開できる）
    if w as u64 * h as u64 > limits.thumb_pixels {
        return LibInfo {
            kind: ItemKind::Image,
            inspected,
            sha256: sha,
            content: String::new(),
        };
    }
    let img = match png::decode(&bytes) {
        Ok(i) => i,
        Err(p) => return broken(ItemKind::Image, problem(p)),
    };
    let inspected = inspected.with_picture(image_picture(&img));
    // 画素の札（棚の画像と同じ。文書と同じ向き＝下の行が先で数える）
    let mut raw = img.into_raw();
    png::flip_rows(&mut raw, w as usize * 4);
    let content = yolu_io::shelf::image_hash(&raw, w, h).unwrap_or_default();
    drop(raw);
    if let (Some(cache), false) = (cache, cancel.is_set()) {
        if let Some(mut cached) = to_cached(&inspected, None) {
            cached.info += &format!("content={content}\n");
            cache.put(&key, &cached);
        }
    }
    LibInfo {
        kind: ItemKind::Image,
        inspected,
        sha256: sha,
        content,
    }
}

fn probe_smart(target: &Target, limits: Limits, cache: Option<&Cache>, cancel: &Cancel) -> LibInfo {
    let sha = match hashed(target, limits, cancel) {
        Ok(sha) => sha,
        Err(why) => return broken(ItemKind::SmartMaterial, why),
    };
    let key = Cache::key("library-smart", &sha);
    let (inspected, kind) = remembered(
        cache,
        &key,
        || cancel.is_set(),
        || match files::read(&target.root, &target.rel, limits.read, Some(cancel.flag())) {
            Ok(bytes) => {
                let (mut inspected, kind) = inspect_smart_kind(Some(&bytes), limits.preview);
                // 壊れたファイル・形式が違うファイルは、短い言い方で（診断の本文は日本語で、英語の画面には出せない）
                if matches!(inspected.block, Some(Block::Unreadable(_))) {
                    inspected.block = Some(Block::Unreadable(Unreadable::pair(
                        "形式が合いません",
                        "Wrong format",
                    )));
                }
                (inspected, kind)
            }
            Err(e) => (
                Inspected::bare(Some(Block::Unreadable(unreadable(&e)))),
                None,
            ),
        },
    );
    // 読めないファイルは、スマートマテリアルの方に並べる（理由つきで）
    LibInfo {
        kind: match kind {
            Some(SmartKind::Mask) => ItemKind::SmartMask,
            _ => ItemKind::SmartMaterial,
        },
        inspected,
        content: sha.clone(),
        sha256: sha,
    }
}
