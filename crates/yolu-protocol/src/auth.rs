//! Live Link の鍵: 同じ PC のほかのユーザー（とその人のプログラム）がつなげないようにする。
//!
//! - 待ち受ける側（スタンドアロン）が待ち受けを始めるたびに 32 バイトの乱数の鍵を OS の乱数から作り、自分だけのフォルダ
//!   （`private::link_dir`）の `<つなぎ先の名前>.key` に自分だけが読める形で置く（待ち受けをやめると消す）。
//! - Unity 側のブリッジはそれを読み、挨拶（Hello）の後ろに足した欄で鍵を知っていることを示す。鍵そのものは流さない:
//!   挨拶は乱数の nonce（32 バイト）と HMAC-SHA256(鍵, "YLNK hello" ‖ nonce)。返事（Welcome）は
//!   HMAC-SHA256(鍵, "YLNK welcome" ‖ nonce ‖ 版 ‖ つながりの番号)。Unity はスタンドアロンも鍵を知っていることを確かめてから
//!   モデルを送る（名前を先に取った別のプログラムには、鍵もモデルも渡らない）。
//! - スタンドアロンは同じ nonce の挨拶を 2 度は受けない（覚えるのは待ち受けごとに新しい 4096 個）。
//!
//! 守れないこと: 同じユーザーのほかのプログラムは鍵のファイルを読めるので、つなげる（OS の上で同じユーザーは区別しない）。

use std::collections::{HashSet, VecDeque};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sha2::{Digest, Sha256};

/// 鍵・nonce・証しのバイト数。
pub const KEY_BYTES: usize = 32;
pub const NONCE_BYTES: usize = 32;
pub const PROOF_BYTES: usize = 32;

/// 鍵のファイルの頭（形の版）。
const KEY_FILE_MAGIC: &str = "YLKEY1 ";
const HELLO_LABEL: &[u8] = b"YLNK hello\0";
const WELCOME_LABEL: &[u8] = b"YLNK welcome\0";
/// 覚えておく使った nonce の数。
const SEEN_LIMIT: usize = 4096;

/// 鍵（表示しない）。
#[derive(Clone, PartialEq, Eq)]
pub struct LinkKey([u8; KEY_BYTES]);

impl std::fmt::Debug for LinkKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LinkKey(…)")
    }
}

/// OS の乱数で埋める。
pub fn random_bytes<const N: usize>() -> io::Result<[u8; N]> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| io::Error::other(format!("OS の乱数を取れません: {e}")))?;
    Ok(b)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != N * 2 || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; N];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// HMAC-SHA256（RFC 2104。鍵は 64 バイトまで）。
pub fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let mut h = Sha256::new();
    h.update(ipad);
    for p in parts {
        h.update(p);
    }
    let inner = h.finalize();
    let mut h = Sha256::new();
    h.update(opad);
    h.update(inner);
    h.finalize().into()
}

/// 長さの同じバイトを、中身によらない時間で比べる。
pub fn same_bytes(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

impl LinkKey {
    /// 新しい鍵（OS の乱数）。
    pub fn generate() -> io::Result<LinkKey> {
        Ok(LinkKey(random_bytes()?))
    }

    pub fn from_bytes(b: [u8; KEY_BYTES]) -> LinkKey {
        LinkKey(b)
    }

    /// 挨拶の証し。
    pub fn hello_proof(&self, nonce: &[u8; NONCE_BYTES]) -> [u8; PROOF_BYTES] {
        hmac_sha256(&self.0, &[HELLO_LABEL, nonce])
    }

    /// 返事の証し（挨拶の nonce・選んだ版・つながりの番号に結ぶ）。
    pub fn welcome_proof(
        &self,
        nonce: &[u8; NONCE_BYTES],
        version: u16,
        session: u64,
    ) -> [u8; PROOF_BYTES] {
        hmac_sha256(
            &self.0,
            &[
                WELCOME_LABEL,
                nonce,
                &version.to_le_bytes(),
                &session.to_le_bytes(),
            ],
        )
    }

    /// 鍵のファイルの中身。
    fn file_text(&self) -> String {
        format!("{KEY_FILE_MAGIC}{}\n", hex(&self.0))
    }

    /// 鍵のファイルを読む（Unix では、自分だけが読めるファイルでなければ断る）。
    pub fn read_file(path: &Path) -> io::Result<LinkKey> {
        let mut file = crate::private::open_private_file(path)?;
        crate::private::check_private_file(&file, path)?;
        let mut text = String::new();
        Read::take(&mut file, 256).read_to_string(&mut text)?;
        text.trim_end()
            .strip_prefix(KEY_FILE_MAGIC)
            .and_then(unhex::<KEY_BYTES>)
            .map(LinkKey)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{} は Live Link の鍵の形ではありません", path.display()),
                )
            })
    }

    /// 鍵のファイルを、自分だけが読める形で置く（隣に作ってから置き換える。読み手が書きかけを読まない）。
    pub fn write_file(&self, path: &Path) -> io::Result<()> {
        let tmp = path.with_extension(format!("key-{}.tmp", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        let written = (|| {
            let mut f = crate::private::create_private_file(&tmp, false)?;
            f.write_all(self.file_text().as_bytes())?;
            f.sync_all()
        })();
        if let Err(e) = written.and_then(|_| std::fs::rename(&tmp, path)) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        Ok(())
    }

    /// つなぎ先の名前の鍵を読む（ブリッジ。スタンドアロンが待ち受けていなければ、ファイルが無い）。
    pub fn load(name: &str) -> io::Result<LinkKey> {
        let path = find_key_path(name)?;
        LinkKey::read_file(&path).map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("スタンドアロンが「{name}」で待ち受けていません（鍵のファイル {} がありません）", path.display()),
                )
            } else {
                e
            }
        })
    }
}

/// つなぎ先の名前の鍵のファイル（スタンドアロンが置く所。自分だけのフォルダの中）。
pub fn key_path(name: &str) -> io::Result<PathBuf> {
    Ok(crate::private::link_dir()?.join(format!("{name}.key")))
}

/// つなぎ先の名前の鍵のファイルを探す（ブリッジ。作らない）。
pub fn find_key_path(name: &str) -> io::Result<PathBuf> {
    Ok(crate::private::find_link_dir(name)?.join(format!("{name}.key")))
}

/// 使った nonce の覚え（新しい `SEEN_LIMIT` 個）。
#[derive(Debug, Default)]
struct SeenNonces {
    set: HashSet<[u8; NONCE_BYTES]>,
    order: VecDeque<[u8; NONCE_BYTES]>,
}

/// 待ち受ける側の鍵（使った nonce を覚える）。
#[derive(Debug)]
pub struct ServerKey {
    key: LinkKey,
    seen: Mutex<SeenNonces>,
}

/// 挨拶を確かめた結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelloCheck {
    Ok,
    /// 鍵の欄が無い（古いブリッジ）。
    Missing,
    /// 証しが鍵と合わない。
    Wrong,
    /// 同じ nonce の挨拶をもう受けた。
    Replayed,
}

impl ServerKey {
    pub fn new(key: LinkKey) -> ServerKey {
        ServerKey {
            key,
            seen: Mutex::new(SeenNonces::default()),
        }
    }

    pub fn key(&self) -> &LinkKey {
        &self.key
    }

    /// 挨拶の鍵の欄を確かめ、合えば nonce を使ったことにする。
    pub fn check_hello(&self, auth: Option<&crate::message::HelloAuth>) -> HelloCheck {
        let Some(auth) = auth else {
            return HelloCheck::Missing;
        };
        if !same_bytes(&self.key.hello_proof(&auth.nonce), &auth.proof) {
            return HelloCheck::Wrong;
        }
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        if !seen.set.insert(auth.nonce) {
            return HelloCheck::Replayed;
        }
        seen.order.push_back(auth.nonce);
        if seen.order.len() > SEEN_LIMIT {
            if let Some(old) = seen.order.pop_front() {
                seen.set.remove(&old);
            }
        }
        HelloCheck::Ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::HelloAuth;

    #[test]
    fn hmac_matches_rfc_4231() {
        // 試験 1・2（RFC 4231）
        assert_eq!(
            hex(&hmac_sha256(&[0x0b; 20], &[b"Hi There"])),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        assert_eq!(
            hex(&hmac_sha256(
                b"Jefe",
                &[b"what do ya ", b"want for nothing?"]
            )),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // 試験 6（鍵が 131 バイト）
        assert_eq!(
            hex(&hmac_sha256(
                &[0xaa; 131],
                &[b"Test Using Larger Than Block-Size Key - Hash Key First"]
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn a_hello_is_accepted_once_and_a_wrong_or_missing_proof_is_not() {
        let key = LinkKey::generate().unwrap();
        let server = ServerKey::new(key.clone());
        let nonce = random_bytes().unwrap();
        let good = HelloAuth {
            nonce,
            proof: key.hello_proof(&nonce),
        };
        assert_eq!(server.check_hello(None), HelloCheck::Missing);
        let other = LinkKey::generate().unwrap();
        let wrong = HelloAuth {
            nonce,
            proof: other.hello_proof(&nonce),
        };
        assert_eq!(server.check_hello(Some(&wrong)), HelloCheck::Wrong);
        assert_eq!(server.check_hello(Some(&good)), HelloCheck::Ok);
        assert_eq!(server.check_hello(Some(&good)), HelloCheck::Replayed);
        // 返事の証しは nonce・版・番号に結ぶ
        assert_ne!(
            key.welcome_proof(&nonce, 1, 7),
            key.welcome_proof(&nonce, 1, 8)
        );
        assert_ne!(
            key.welcome_proof(&nonce, 1, 7),
            other.welcome_proof(&nonce, 1, 7)
        );
    }

    #[test]
    fn the_key_file_round_trips_and_is_private() {
        let dir = std::env::temp_dir().join(format!("ylp-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        crate::private::ensure_private_dir(&dir).unwrap();
        let path = dir.join("t.key");
        let key = LinkKey::generate().unwrap();
        key.write_file(&path).unwrap();
        assert_eq!(LinkKey::read_file(&path).unwrap(), key);
        // 置き換えも同じ形
        let next = LinkKey::generate().unwrap();
        next.write_file(&path).unwrap();
        assert_eq!(LinkKey::read_file(&path).unwrap(), next);
        assert!(format!("{next:?}").contains('…'), "鍵を表示しない");
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(
                LinkKey::read_file(&path).is_err(),
                "ほかの人も読める鍵は使わない"
            );
        }
        std::fs::write(dir.join("bad.key"), "YLKEY1 xyz\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.join("bad.key"), std::fs::Permissions::from_mode(0o600))
                .unwrap();
        }
        assert_eq!(
            LinkKey::read_file(&dir.join("bad.key")).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
