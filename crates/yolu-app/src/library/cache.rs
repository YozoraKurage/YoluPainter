//! サムネイルのディスクのキャッシュ（利用者のキャッシュのフォルダ）。中身の札（SHA-256）から決めた名前で 1 枚ずつ覚え、
//! 数と大きさに上限を持つ。キャッシュは作り直せる写しなので、壊れた・古い・読めないものは黙って捨てて作り直す（保存の正本ではない）。
//!
//! ファイルは `<札>.thumb`: `YLTH`・形式の版・説明の長さ・PNG の長さ（little-endian の u32 を 2 つ）・説明（UTF-8）・PNG（RGBA8）。
//! 書き込みは一時ファイルを置換する 1 回で確定する（読む側は、書きかけのファイルを見ない）。上限を超えたら、触った時刻が古い順に
//! 上限の 9 割まで消す。名前が札なので、同じ中身は 1 枚。札には、絵の作り方の版（`RECIPE`）を混ぜるので、作り方を変えたら版を上げれば、
//! 前の版の絵は使われずに、古いものから消えていく。
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// 絵の作り方の版（サムネイルの描き方を変えたら上げる）。
pub const RECIPE: u32 = 1;
/// 覚える枚数の上限（整理は `TRIM_EVERY` 回の書き込みごとなので、次の整理までのあいだは `TRIM_EVERY - 1` 枚ぶんまで超えうる）。
pub const MAX_FILES: usize = 4096;
/// 覚える大きさの上限（バイト。枚数と同じく、次の整理までのあいだは、`TRIM_EVERY - 1` 枚ぶん＝最大 `MAX_ENTRY_BYTES` ずつ超えうる）。
pub const MAX_BYTES: u64 = 64 * 1024 * 1024;
/// 1 枚の大きさの上限（バイト。これを超える絵は覚えない）。
pub const MAX_ENTRY_BYTES: usize = 256 * 1024;
/// 説明の長さの上限（バイト）。
const MAX_INFO: usize = 2048;
/// 絵の 1 辺の上限（画素。サムネイルはこれより小さい）。
const MAX_SIDE: u32 = 256;
const MAGIC: &[u8; 4] = b"YLTH";
const VERSION: u8 = 1;
const EXTENSION: &str = "thumb";
/// 使っているのに触った時刻を更新しない間隔（頻繁な書き込みを避ける）。
const TOUCH_AFTER: Duration = Duration::from_secs(3600);
/// 一時ファイルが残っていたら消すまでの時間。
const TEMP_LIFETIME: Duration = Duration::from_secs(3600);
/// 整理を走らせる書き込みの間隔（最初の書き込みでも走る）。書き込みのたびには走らせない（フォルダを毎回読まない）ので、上限は
/// 書き込みのあいだ、この回数 − 1 枚ぶんまで超えうる。
const TRIM_EVERY: usize = 64;

/// 利用者のキャッシュのフォルダ（Windows は `LOCALAPPDATA`、Mac は `~/Library/Caches`、そのほかは `XDG_CACHE_HOME` か `~/.cache`）。
/// 環境変数が絶対パスでなければ使わない（相対パスのキャッシュを作業フォルダの中に作らない）。
fn cache_base(os: &str, env: impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    let absolute = |key| env(key).filter(|p| p.is_absolute());
    match os {
        "windows" => absolute("LOCALAPPDATA"),
        "macos" => absolute("HOME").map(|p| p.join("Library/Caches")),
        _ => absolute("XDG_CACHE_HOME").or_else(|| absolute("HOME").map(|p| p.join(".cache"))),
    }
}

/// 既定のサムネイルのキャッシュのフォルダ（利用者のキャッシュのフォルダの下の `YoluPainter/thumbnails`）。
pub fn default_dir() -> Option<PathBuf> {
    cache_base(std::env::consts::OS, |key| {
        std::env::var_os(key).map(PathBuf::from)
    })
    .map(|base| base.join("YoluPainter").join("thumbnails"))
}

/// 設定のファイルに合わせたキャッシュのフォルダ。既定の場所の設定のファイルなら利用者のキャッシュのフォルダ、そうでなければ
/// （試験・別の場所の設定）、設定のファイルの隣の `thumbnails~`（利用者のキャッシュに試験の絵を混ぜない）。
pub fn dir_for(settings: &Path) -> Option<PathBuf> {
    if crate::settings::path().as_deref() == Some(settings) {
        default_dir()
    } else {
        settings.parent().map(|dir| dir.join("thumbnails~"))
    }
}

/// 覚える絵 1 枚。`info` は呼び出し側が決める短い説明（寸法やレイヤーの数など。`key=value` の行）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cached {
    pub info: String,
    pub width: u32,
    pub height: u32,
    /// straight RGBA8（画像の上の行から）。
    pub rgba: Vec<u8>,
}

pub struct Cache {
    dir: PathBuf,
    max_files: usize,
    max_bytes: u64,
    puts: AtomicUsize,
    trimming: Mutex<()>,
}

impl Cache {
    /// 既定の上限で、フォルダ `dir`（無ければ書くときに作る）のキャッシュ。
    pub fn new(dir: PathBuf) -> Cache {
        Cache::with_limits(dir, MAX_FILES, MAX_BYTES)
    }

    pub fn with_limits(dir: PathBuf, max_files: usize, max_bytes: u64) -> Cache {
        Cache {
            dir,
            max_files,
            max_bytes,
            puts: AtomicUsize::new(0),
            trimming: Mutex::new(()),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 札（64 桁の小文字 16 進）。種類（`kind`）と中身の札（`content`）と作り方の版から決まる。
    pub fn key(kind: &str, content: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut sha = Sha256::new();
        sha.update(b"yolupainter-thumbnail");
        sha.update(RECIPE.to_le_bytes());
        for part in [kind, content] {
            sha.update((part.len() as u64).to_le_bytes());
            sha.update(part.as_bytes());
        }
        format!("{:x}", sha.finalize())
    }

    fn path_of(&self, key: &str) -> Option<PathBuf> {
        let ok = key.len() == 64
            && key
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        ok.then(|| self.dir.join(format!("{key}.{EXTENSION}")))
    }

    /// 覚えている絵を読む。無い・壊れている・形式が違うときは None（壊れたファイルは消す）。
    pub fn get(&self, key: &str) -> Option<Cached> {
        let path = self.path_of(key)?;
        let meta = std::fs::symlink_metadata(&path).ok()?;
        if !meta.is_file() {
            return None;
        }
        if meta.len() > MAX_ENTRY_BYTES as u64 + MAX_INFO as u64 + 16 {
            let _ = std::fs::remove_file(&path);
            return None;
        }
        let bytes = std::fs::read(&path).ok()?;
        let Some(cached) = decode(&bytes) else {
            let _ = std::fs::remove_file(&path);
            return None;
        };
        if meta
            .modified()
            .ok()
            .and_then(|m| m.elapsed().ok())
            .is_some_and(|age| age > TOUCH_AFTER)
        {
            if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&path) {
                let _ = file.set_modified(SystemTime::now());
            }
        }
        Some(cached)
    }

    /// 絵を覚える（大きすぎる・書けないときは何もしない。置き換えは 1 回）。
    pub fn put(&self, key: &str, cached: &Cached) {
        let Some(path) = self.path_of(key) else {
            return;
        };
        let Some(bytes) = encode(cached) else {
            return;
        };
        let opts = yolu_io::atomic::ReplaceOptions {
            create_dirs: true,
            ..Default::default()
        };
        if yolu_io::atomic::replace_with(&path, &opts, |f| f.write_all(&bytes)).is_err() {
            return;
        }
        let n = self.puts.fetch_add(1, Ordering::Relaxed);
        if n.is_multiple_of(TRIM_EVERY) {
            self.trim();
        }
    }

    /// 上限を超えていれば、触った時刻が古い順に上限の 9 割まで消す。古い一時ファイルも消す。別のスレッドが整理していれば何もしない。
    pub fn trim(&self) {
        let Ok(_guard) = self.trimming.try_lock() else {
            return;
        };
        let Ok(read) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let mut files: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
        for entry in read.filter_map(|e| e.ok()) {
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let path = entry.path();
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            match path.extension().and_then(|e| e.to_str()) {
                Some(EXTENSION) => files.push((modified, meta.len(), path)),
                // 一時ファイル（今の `.{名前}.{pid}-{番号}.pending~` と、前の版の `.tmp`）
                Some("pending~" | "tmp")
                    if modified.elapsed().is_ok_and(|age| age > TEMP_LIFETIME) =>
                {
                    let _ = std::fs::remove_file(&path);
                }
                _ => {}
            }
        }
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        if files.len() <= self.max_files && total <= self.max_bytes {
            return;
        }
        files.sort();
        let (keep_files, keep_bytes) = (self.max_files * 9 / 10, self.max_bytes / 10 * 9);
        let mut count = files.len();
        for (_, len, path) in files {
            if count <= keep_files && total <= keep_bytes {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                count -= 1;
                total -= len;
            }
        }
    }

    /// 覚えている枚数と大きさ（試験用）。
    pub fn usage(&self) -> (usize, u64) {
        let Ok(read) = std::fs::read_dir(&self.dir) else {
            return (0, 0);
        };
        read.filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == EXTENSION))
            .filter_map(|e| e.metadata().ok())
            .fold((0, 0), |(n, b), m| (n + 1, b + m.len()))
    }
}

fn encode(cached: &Cached) -> Option<Vec<u8>> {
    let (w, h) = (cached.width, cached.height);
    if w == 0
        || h == 0
        || w > MAX_SIDE
        || h > MAX_SIDE
        || cached.rgba.len() != w as usize * h as usize * 4
        || cached.info.len() > MAX_INFO
    {
        return None;
    }
    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&cached.rgba).ok()?;
        writer.finish().ok()?;
    }
    if png_bytes.len() > MAX_ENTRY_BYTES {
        return None;
    }
    let mut out = Vec::with_capacity(13 + cached.info.len() + png_bytes.len());
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&(cached.info.len() as u32).to_le_bytes());
    out.extend_from_slice(&(png_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(cached.info.as_bytes());
    out.extend_from_slice(&png_bytes);
    Some(out)
}

fn decode(bytes: &[u8]) -> Option<Cached> {
    if bytes.len() < 13 || &bytes[..4] != MAGIC || bytes[4] != VERSION {
        return None;
    }
    let info_len = u32::from_le_bytes(bytes[5..9].try_into().ok()?) as usize;
    let png_len = u32::from_le_bytes(bytes[9..13].try_into().ok()?) as usize;
    if info_len > MAX_INFO || png_len > MAX_ENTRY_BYTES || bytes.len() != 13 + info_len + png_len {
        return None;
    }
    let info = std::str::from_utf8(&bytes[13..13 + info_len])
        .ok()?
        .to_owned();
    let mut decoder = png::Decoder::new(std::io::Cursor::new(&bytes[13 + info_len..]));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info().ok()?;
    let header = reader.info();
    if header.color_type != png::ColorType::Rgba
        || header.bit_depth != png::BitDepth::Eight
        || header.width == 0
        || header.height == 0
        || header.width > MAX_SIDE
        || header.height > MAX_SIDE
    {
        return None;
    }
    let (width, height) = (header.width, header.height);
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    let frame = reader.next_frame(&mut rgba).ok()?;
    // 末尾（IEND）まで読んで確かめる（途中で切れた・末尾だけ壊れたファイルを通さない）
    reader.finish().ok()?;
    (frame.buffer_size() == rgba.len()).then_some(Cached {
        info,
        width,
        height,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    static NEXT: AtomicU32 = AtomicU32::new(0);

    struct Dir(PathBuf);
    impl Dir {
        fn new() -> Dir {
            let path = std::env::temp_dir().join(format!(
                "yolu-thumbs-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = std::fs::remove_dir_all(&path);
            Dir(path)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn sample(seed: u8) -> Cached {
        Cached {
            info: format!("w=64\nh=2\nseed={seed}\n"),
            width: 2,
            height: 2,
            rgba: (0..16).map(|i| i as u8 ^ seed).collect(),
        }
    }

    #[test]
    fn the_cache_folder_follows_each_os_and_rejects_relative_paths() {
        let base = std::env::temp_dir();
        let env = |key: &str| match key {
            "LOCALAPPDATA" => Some(base.join("local")),
            "XDG_CACHE_HOME" => Some(PathBuf::from("relative")),
            "HOME" => Some(base.join("home")),
            _ => None,
        };
        assert_eq!(cache_base("windows", env), Some(base.join("local")));
        assert_eq!(
            cache_base("macos", env),
            Some(base.join("home/Library/Caches"))
        );
        // 相対パスの XDG_CACHE_HOME は使わず、ホームの .cache へ
        assert_eq!(cache_base("linux", env), Some(base.join("home/.cache")));
        assert_eq!(cache_base("linux", |_| None), None);
        // 別の場所の設定のファイルは、その隣
        let other = base.join("somewhere/settings.conf");
        assert_eq!(dir_for(&other), Some(base.join("somewhere/thumbnails~")));
    }

    #[test]
    fn keys_depend_on_kind_and_content_and_are_hex() {
        let a = Cache::key("image", "abc");
        assert_eq!(a.len(), 64);
        assert!(a
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        assert_eq!(a, Cache::key("image", "abc"));
        assert_ne!(a, Cache::key("smart", "abc"));
        assert_ne!(a, Cache::key("image", "abd"));
        // 区切りが違えば別の札（「ab」+「c」と「a」+「bc」）
        assert_ne!(Cache::key("ab", "c"), Cache::key("a", "bc"));
    }

    #[test]
    fn a_picture_comes_back_exactly_and_a_missing_one_is_none() {
        let dir = Dir::new();
        let cache = Cache::new(dir.0.clone());
        let key = Cache::key("image", "one");
        assert_eq!(cache.get(&key), None);
        cache.put(&key, &sample(1));
        assert_eq!(cache.get(&key), Some(sample(1)));
        // 同じ札へ置き直すと置き換わる
        cache.put(&key, &sample(2));
        assert_eq!(cache.get(&key), Some(sample(2)));
        assert_eq!(cache.usage().0, 1);
        // 札でない名前は読まず・書かず、フォルダの外へ出ない
        for bad in ["../x", "abc", &"G".repeat(64), &key.to_uppercase()] {
            assert_eq!(cache.get(bad), None);
            cache.put(bad, &sample(3));
        }
        assert_eq!(cache.usage().0, 1);
    }

    #[test]
    fn transparent_pixels_keep_their_rgb() {
        let dir = Dir::new();
        let cache = Cache::new(dir.0.clone());
        let key = Cache::key("image", "alpha");
        let cached = Cached {
            info: String::new(),
            width: 1,
            height: 2,
            rgba: vec![10, 20, 30, 0, 200, 100, 50, 255],
        };
        cache.put(&key, &cached);
        assert_eq!(cache.get(&key), Some(cached));
    }

    #[test]
    fn a_broken_or_foreign_file_is_dropped_and_read_as_a_miss() {
        let dir = Dir::new();
        let cache = Cache::new(dir.0.clone());
        let key = Cache::key("image", "broken");
        cache.put(&key, &sample(7));
        let path = dir.0.join(format!("{key}.thumb"));
        let good = std::fs::read(&path).unwrap();
        let cases: Vec<Vec<u8>> = vec![
            Vec::new(),
            b"not a thumbnail at all".to_vec(),
            good[..good.len() - 5].to_vec(),
            {
                let mut v = good.clone();
                v[4] = 99; // 知らない版
                v
            },
            {
                let mut v = good.clone();
                let last = v.len() - 1;
                v[last] ^= 0xff; // PNG の末尾が壊れている
                v
            },
            {
                let mut v = good.clone();
                v.push(0); // 長さが合わない
                v
            },
        ];
        for bytes in cases {
            std::fs::write(&path, &bytes).unwrap();
            assert_eq!(cache.get(&key), None);
            assert!(!path.exists(), "壊れたファイルは消す");
        }
    }

    #[test]
    fn too_big_or_malformed_pictures_are_not_kept() {
        let dir = Dir::new();
        let cache = Cache::new(dir.0.clone());
        let key = Cache::key("image", "big");
        let big = Cached {
            info: String::new(),
            width: MAX_SIDE + 1,
            height: 1,
            rgba: vec![0; (MAX_SIDE as usize + 1) * 4],
        };
        cache.put(&key, &big);
        let wrong_length = Cached {
            info: String::new(),
            width: 2,
            height: 2,
            rgba: vec![0; 3],
        };
        cache.put(&key, &wrong_length);
        let long_info = Cached {
            info: "x".repeat(MAX_INFO + 1),
            ..sample(0)
        };
        cache.put(&key, &long_info);
        assert_eq!(cache.usage().0, 0);
    }

    #[test]
    fn the_oldest_pictures_go_first_when_the_count_or_size_limit_is_passed() {
        let dir = Dir::new();
        let cache = Cache::with_limits(dir.0.clone(), 10, MAX_BYTES);
        let keys: Vec<String> = (0..14)
            .map(|i| Cache::key("image", &i.to_string()))
            .collect();
        for (i, key) in keys.iter().enumerate() {
            cache.put(key, &sample(i as u8));
            // 触った時刻に差をつける（古いものから順に）
            let path = dir.0.join(format!("{key}.thumb"));
            let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            let when = SystemTime::now() - Duration::from_secs(1000 - i as u64 * 10);
            file.set_modified(when).unwrap();
        }
        cache.trim();
        // 上限 10 の 9 割 = 9 枚まで。新しい 9 枚が残る
        let (count, _) = cache.usage();
        assert_eq!(count, 9);
        for (i, key) in keys.iter().enumerate() {
            assert_eq!(cache.get(key).is_some(), i >= 5, "{i}");
        }
        // 大きさの上限
        let one = std::fs::metadata(dir.0.join(format!("{}.thumb", keys[13])))
            .unwrap()
            .len();
        let small = Cache::with_limits(dir.0.clone(), 100, one * 4);
        small.trim();
        let (count, bytes) = small.usage();
        assert!(bytes <= one * 4 / 10 * 9 && count <= 3, "{count} {bytes}");
        // いちばん新しいものが残る
        assert!(cache.get(&keys[13]).is_some());
    }

    #[test]
    fn writes_alone_keep_the_count_within_the_limit_plus_one_trim_interval() {
        let dir = Dir::new();
        let limit = 10;
        let cache = Cache::with_limits(dir.0.clone(), limit, MAX_BYTES);
        let mut most = 0;
        for i in 0..400u32 {
            cache.put(&Cache::key("image", &i.to_string()), &sample(i as u8));
            most = most.max(cache.usage().0);
        }
        // 整理は 64 回の書き込みごと。そのあいだは上限を超えうるが、次の整理までの分（63 枚）を超えない
        assert!(most > limit, "整理のあいだは上限を超える: {most}");
        assert!(most < limit + TRIM_EVERY, "{most}");
        // 整理すれば上限に収まる
        cache.trim();
        assert!(cache.usage().0 <= limit, "{}", cache.usage().0);
    }

    #[test]
    fn the_first_write_trims_a_folder_that_was_already_over_the_limit() {
        let dir = Dir::new();
        let loose = Cache::with_limits(dir.0.clone(), 1000, MAX_BYTES);
        for i in 0..20 {
            loose.put(&Cache::key("image", &format!("old{i}")), &sample(i as u8));
        }
        let tight = Cache::with_limits(dir.0.clone(), 5, MAX_BYTES);
        tight.put(&Cache::key("image", "fresh"), &sample(99));
        assert!(tight.usage().0 <= 5);
        assert!(tight.get(&Cache::key("image", "fresh")).is_some());
    }

    #[test]
    fn a_stale_temporary_file_is_swept_and_a_fresh_one_is_left() {
        let dir = Dir::new();
        let cache = Cache::new(dir.0.clone());
        std::fs::create_dir_all(&dir.0).unwrap();
        let stale = dir.0.join(".stale.tmp");
        let fresh = dir.0.join(".fresh.tmp");
        std::fs::write(&stale, b"x").unwrap();
        std::fs::write(&fresh, b"x").unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_modified(SystemTime::now() - TEMP_LIFETIME - Duration::from_secs(60))
            .unwrap();
        cache.trim();
        assert!(!stale.exists());
        assert!(fresh.exists());
    }
}
