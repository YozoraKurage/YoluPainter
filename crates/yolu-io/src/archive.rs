use crate::{check, check_budget, hash, is_hash, valid_id, Error, Result};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    sync::Arc,
};
pub const MAX_ENTRY_BYTES: usize = 512 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: usize = 768 * 1024 * 1024;
const MANIFEST: &str = "manifest.sha256";
pub(crate) type Files = BTreeMap<String, Arc<[u8]>>;

/// manifest と全エントリを検証したアーカイブ。元の manifest の文字列も保持する。
#[derive(Clone, Debug)]
pub struct Archive {
    pub(crate) files: Files,
    pub(crate) manifest: Arc<[u8]>,
    pub(crate) level: u32,
    pub(crate) mime: String,
}
impl Archive {
    pub fn read(bytes: &[u8]) -> Result<Self> {
        Self::read_profile(bytes, "application/x-yolupainter", "YOLUPAINTER-YLP-", 3)
    }
    pub fn entries(&self) -> &Files {
        &self.files
    }
    pub fn manifest(&self) -> &[u8] {
        &self.manifest
    }
    pub fn manifest_version(&self) -> u32 {
        self.level
    }
    pub fn from_entries(files: BTreeMap<String, Vec<u8>>) -> Result<Self> {
        let files: Files = files.into_iter().map(|(k, v)| (k, Arc::from(v))).collect();
        Self::build(files, 3, "application/x-yolupainter", "YOLUPAINTER-YLP-")
    }
    pub(crate) fn build(files: Files, level: u32, mime: &str, prefix: &str) -> Result<Self> {
        check_budget(files.len() <= 1000, "エントリが1000個を超えています")?;
        let mut manifest = format!("{prefix}{level}\n");
        let mut total = 0usize;
        for (name, b) in &files {
            name_check(name, level, prefix)?;
            check(
                name != "mimetype" && name != MANIFEST,
                "予約されたエントリ名です",
            )?;
            total = total
                .checked_add(b.len())
                .ok_or_else(|| Error::Budget("長さが過大です".into()))?;
            check_budget(
                b.len() <= MAX_ENTRY_BYTES && total <= MAX_TOTAL_BYTES,
                "アーカイブの予算超過です",
            )?;
            manifest.push_str(&format!("{} {} {}\n", hash(b), b.len(), name));
        }
        complete(&files, level, prefix)?;
        Ok(Self {
            files,
            manifest: Arc::from(manifest.into_bytes()),
            level,
            mime: mime.into(),
        })
    }
    /// zip の圧縮結果・日時以外のエントリ内容を変えずに書く。
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        let mut count = 0u16;
        for (name, data) in std::iter::once(("mimetype", self.mime.as_bytes()))
            .chain(std::iter::once((MANIFEST, self.manifest.as_ref())))
            .chain(self.files.iter().map(|(n, b)| (n.as_str(), b.as_ref())))
        {
            let mut deflate =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            let compressed = if name != "mimetype" && !name.ends_with(".png") {
                deflate.write_all(data)?;
                deflate.finish()?
            } else {
                Vec::new()
            };
            let (method, stored) = if !compressed.is_empty() && compressed.len() < data.len() {
                (8u16, compressed.as_slice())
            } else {
                (0, data)
            };
            let offset = u32::try_from(out.len())
                .map_err(|_| Error::InvalidData("zip64は未対応です".into()))?;
            let crc = crc32fast::hash(data);
            put32(&mut out, 0x04034b50);
            for n in [20, 0, method, 0, 33] {
                put16(&mut out, n)
            }
            for n in [crc, stored.len() as u32, data.len() as u32] {
                put32(&mut out, n)
            }
            put16(&mut out, name.len() as u16);
            put16(&mut out, 0);
            out.extend(name.as_bytes());
            out.extend(stored);
            put32(&mut central, 0x02014b50);
            for n in [20, 20, 0, method, 0, 33] {
                put16(&mut central, n)
            }
            for n in [crc, stored.len() as u32, data.len() as u32] {
                put32(&mut central, n)
            }
            for n in [name.len() as u16, 0, 0, 0, 0] {
                put16(&mut central, n)
            }
            put32(&mut central, 0);
            put32(&mut central, offset);
            central.extend(name.as_bytes());
            count += 1;
        }
        let start = out.len() as u32;
        out.extend(&central);
        put32(&mut out, 0x06054b50);
        for n in [0, 0, count, count] {
            put16(&mut out, n)
        }
        put32(&mut out, central.len() as u32);
        put32(&mut out, start);
        put16(&mut out, 0);
        Ok(out)
    }
    pub(crate) fn read_profile(b: &[u8], mime: &str, prefix: &str, newest: u32) -> Result<Self> {
        check_budget(
            b.len() <= MAX_TOTAL_BYTES + 2 * 1024 * 1024,
            "圧縮ファイルの予算超過です",
        )?;
        check(
            b.get(0..4) == Some(b"PK\x03\x04")
                && u16at(b, 8)? == 0
                && u32at(b, 18)? as usize == mime.len()
                && u32at(b, 22)? as usize == mime.len()
                && u16at(b, 26)? == 8
                && u16at(b, 28)? == 0
                && b.get(30..38) == Some(b"mimetype")
                && b.get(38..38 + mime.len()) == Some(mime.as_bytes()),
            "先頭に無圧縮のmimetypeがありません",
        )?;
        let end = (b.len().saturating_sub(65557)..b.len().saturating_sub(21))
            .rev()
            .find(|&i| {
                b.get(i..i + 4) == Some(b"PK\x05\x06")
                    && u16at(b, i + 20).is_ok_and(|n| i + 22 + n as usize == b.len())
            })
            .ok_or_else(|| Error::InvalidData("ZIP終端がありません".into()))?;
        check(
            u16at(b, end + 4)? == 0 && u16at(b, end + 6)? == 0,
            "分割ZIPは未対応です",
        )?;
        let count = u16at(b, end + 10)? as usize;
        check(
            (3..=1002).contains(&count) && u16at(b, end + 8)? as usize == count,
            "ZIPエントリ数が不正です",
        )?;
        let mut at = u32at(b, end + 16)? as usize;
        let central_start = at;
        check(
            at.checked_add(u32at(b, end + 12)? as usize) == Some(end),
            "中央ディレクトリが不正またはzip64です",
        )?;
        let mut files = Files::new();
        let mut previous_end = 0usize;
        let mut total = 0usize;
        for index in 0..count {
            check(
                u32at(b, at)? == 0x02014b50,
                "中央ディレクトリが壊れています",
            )?;
            check(
                u16at(b, at + 6)? <= 20,
                "ZIP64または新しいZIP機能は未対応です",
            )?;
            let flags = u16at(b, at + 8)?;
            let method = u16at(b, at + 10)?;
            let crc = u32at(b, at + 16)?;
            let packed = u32at(b, at + 20)? as usize;
            let len = u32at(b, at + 24)? as usize;
            let nl = u16at(b, at + 28)? as usize;
            let xl = u16at(b, at + 30)? as usize;
            let cl = u16at(b, at + 32)? as usize;
            // 立ててよい旗: ビット 3（データ記述子）・ビット 11（UTF-8 の名前）。deflate だけは、圧縮の水準の目安（ビット 1・2）も
            // 立ててよい（Info-ZIP・7-Zip などで詰め直したファイルが立てる。読みには影響せず、C# の System.IO.Compression も無視する。
            // 中身の整合は CRC と長さで見る）。暗号化・強い暗号化・ヘッダーのマスクなどは断る
            let allowed = if method == 8 { 0x080e } else { 0x0808 };
            check(
                u16at(b, at + 34)? == 0 && flags & !allowed == 0 && (method == 0 || method == 8),
                "暗号化・未知の圧縮方式・分割ZIPは未対応です",
            )?;
            let name = std::str::from_utf8(slice(b, at + 46, nl)?)
                .map_err(|_| Error::InvalidData("ZIP名がUTF-8ではありません".into()))?
                .to_string();
            check_extra(slice(b, at + 46 + nl, xl)?)?;
            let local = u32at(b, at + 42)? as usize;
            check(
                (index != 0 || local == 0) && local >= previous_end && local < central_start,
                "ZIPエントリの重なりまたは並びが不正です",
            )?;
            check(
                u32at(b, local)? == 0x04034b50
                    && u16at(b, local + 4)? <= 20
                    && u16at(b, local + 6)? == flags
                    && u16at(b, local + 8)? == method
                    && u16at(b, local + 26)? as usize == nl
                    && slice(b, local + 30, nl)? == name.as_bytes(),
                "ZIPのローカルヘッダーが一致しません",
            )?;
            if flags & 8 == 0 {
                check(
                    u32at(b, local + 14)? == crc
                        && u32at(b, local + 18)? as usize == packed
                        && u32at(b, local + 22)? as usize == len,
                    "ZIPの長さ・CRC宣言が一致しません",
                )?;
            }
            let local_extra = u16at(b, local + 28)? as usize;
            check_extra(slice(b, local + 30 + nl, local_extra)?)?;
            let start = local + 30 + nl + local_extra;
            let finish = start
                .checked_add(packed)
                .ok_or_else(|| Error::Budget("長さが過大です".into()))?;
            check(
                finish <= central_start,
                "ZIPエントリが中央ディレクトリに重なっています",
            )?;
            previous_end = finish;
            if flags & 8 != 0 {
                let descriptor = finish
                    + if u32at(b, finish)? == 0x08074b50 {
                        4
                    } else {
                        0
                    };
                check(
                    descriptor + 12 <= central_start
                        && u32at(b, descriptor)? == crc
                        && u32at(b, descriptor + 4)? as usize == packed
                        && u32at(b, descriptor + 8)? as usize == len,
                    "ZIPのデータ記述子が一致しません",
                )?;
                previous_end = descriptor + 12;
            }
            let limit = if name == MANIFEST {
                1024 * 1024
            } else if name == "mimetype" {
                256
            } else {
                MAX_ENTRY_BYTES
            };
            total = total
                .checked_add(len)
                .ok_or_else(|| Error::Budget("長さが過大です".into()))?;
            check_budget(
                len <= limit && total <= MAX_TOTAL_BYTES + 1024 * 1024 + 256,
                "展開の予算超過です",
            )?;
            let src = slice(b, start, packed)?;
            let data = if method == 0 {
                check(packed == len, "無圧縮ZIPの長さが不正です")?;
                src.to_vec()
            } else {
                let mut d = flate2::read::DeflateDecoder::new(src);
                let mut v = Vec::new();
                // メモリ上の読みなので、ここで起きる失敗はデータの壊れ（Io に見せず、壊れたファイルとして断る）
                d.by_ref()
                    .take(len as u64 + 1)
                    .read_to_end(&mut v)
                    .map_err(|e| {
                        Error::InvalidData(format!("Deflateが壊れています: {name}（{e}）"))
                    })?;
                check(
                    v.len() == len && d.total_in() == packed as u64,
                    format!("Deflateの長さが一致しません: {name}"),
                )?;
                v
            };
            check(
                data.len() == len && crc32fast::hash(&data) == crc,
                format!("CRCまたは長さが一致しません: {name}"),
            )?;
            check(
                index != 0 || name == "mimetype",
                "先頭エントリがmimetypeではありません",
            )?;
            check(
                files.insert(name, Arc::from(data)).is_none(),
                "ZIPに重複エントリがあります",
            )?;
            at += 46 + nl + xl + cl;
        }
        check(at == end, "中央ディレクトリの長さが一致しません")?;
        check(
            files
                .remove("mimetype")
                .is_some_and(|v| v.as_ref() == mime.as_bytes()),
            "MIMEが一致しません",
        )?;
        let manifest = files
            .remove(MANIFEST)
            .ok_or_else(|| Error::InvalidData("manifestがありません".into()))?;
        let text = std::str::from_utf8(&manifest)
            .map_err(|_| Error::InvalidData("manifestがUTF-8ではありません".into()))?;
        let mut lines = text.split('\n');
        let head = lines.next().unwrap_or("");
        let level = head
            .strip_prefix(prefix)
            .and_then(|v| v.parse::<u32>().ok())
            .ok_or_else(|| Error::InvalidData("未知のmanifestです".into()))?;
        check(
            level > 0 && level <= newest && head == format!("{prefix}{level}"),
            format!("manifest {head} は未対応です。対応上限は{newest}です"),
        )?;
        let mut listed = BTreeMap::new();
        let mut total = 0usize;
        for line in lines.filter(|s| !s.is_empty()) {
            let parts: Vec<_> = line.split(' ').collect();
            check(parts.len() == 3, "manifestの行が不正です")?;
            let digest = parts[0];
            let name = parts[2];
            name_check(name, level, prefix)?;
            check(
                is_hash(digest)
                    && !parts[1].is_empty()
                    && parts[1].bytes().all(|b| b.is_ascii_digit()),
                "manifestのハッシュまたは長さが不正です",
            )?;
            let len = parts[1]
                .parse::<usize>()
                .map_err(|_| Error::Budget("長さが過大です".into()))?;
            total = total
                .checked_add(len)
                .ok_or_else(|| Error::Budget("長さが過大です".into()))?;
            check_budget(
                len <= MAX_ENTRY_BYTES && total <= MAX_TOTAL_BYTES,
                "manifestの予算超過です",
            )?;
            check(
                listed.insert(name, (len, digest)).is_none(),
                "manifestに重複があります",
            )?;
        }
        // 行の形・名前・予算は、エントリと突き合わせる前に全部確かめる（予算超過をほかの食い違いより先に言い分ける）
        for (name, (len, digest)) in &listed {
            let data = files
                .get(*name)
                .ok_or_else(|| Error::InvalidData(format!("エントリがありません: {name}")))?;
            check(
                data.len() == *len && hash(data) == *digest,
                format!("SHA-256または長さが一致しません: {name}"),
            )?;
        }
        check(
            files.len() == listed.len(),
            "manifestにないエントリがあります",
        )?;
        complete(&files, level, prefix)?;
        Ok(Self {
            files,
            manifest,
            level,
            mime: mime.into(),
        })
    }
}
fn check_extra(b: &[u8]) -> Result<()> {
    let mut at = 0;
    while at < b.len() {
        let kind = u16at(b, at)?;
        let len = u16at(b, at + 2)? as usize;
        check(kind != 1, "ZIP64は未対応です")?;
        slice(b, at + 4, len)?;
        at += 4 + len;
    }
    Ok(())
}
fn complete(f: &Files, level: u32, prefix: &str) -> Result<()> {
    check(
        if prefix == "YOLUPAINTER-YLP-" {
            f.keys().any(|n| {
                n == "document.utpaint"
                    || level >= 2 && split_set(n).is_some_and(|(_, s)| s == "document.utpaint")
            })
        } else if prefix == "YOLUPAINTER-SMART-" {
            f.contains_key("smart.json") && f.contains_key("layers.utpaint")
        } else {
            f.contains_key("state.json")
        },
        "必要な正本エントリがありません",
    )
}
/// .ylp に要る正本のエントリ（根か、形式 3 からはどれかのセットの下の `document.utpaint`）があるか。
pub(crate) fn complete_ylp<'a>(mut names: impl Iterator<Item = &'a String>, level: u32) -> Result<()> {
    check(
        names.any(|n| {
            n == "document.utpaint"
                || level >= 2 && split_set(n).is_some_and(|(_, s)| s == "document.utpaint")
        }),
        "必要な正本エントリがありません",
    )
}
pub(crate) fn split_set(n: &str) -> Option<(&str, &str)> {
    let s = n.strip_prefix("sets/")?;
    let (id, rest) = s.split_once('/')?;
    valid_id(id).then_some((id, rest))
}
pub(crate) fn name_check(n: &str, level: u32, prefix: &str) -> Result<()> {
    if prefix == "YOLUPAINTER-BRUSH-" {
        return check(
            ["manifest.sha256", "state.json", "texture.png", "dual.png"].contains(&n)
                || n.strip_prefix("tip-")
                    .and_then(|s| s.strip_suffix(".png"))
                    .and_then(|s| s.parse::<u16>().ok())
                    .is_some_and(|i| i < 256),
            format!("未知のブラシエントリです: {n}"),
        );
    }
    let mut leaf = n;
    if prefix == "YOLUPAINTER-YLP-" {
        if level >= 2 {
            if let Some((_, v)) = split_set(n) {
                leaf = v;
            }
        }
        if let Some(v) = leaf.strip_prefix("composite/") {
            leaf = v;
        }
    }
    if level >= 3 || prefix == "YOLUPAINTER-SMART-" {
        if let Some(v) = n.strip_prefix("resources/") {
            leaf = v;
        }
    }
    check(
        !leaf.is_empty()
            && n.len() <= 96
            && !leaf.starts_with('.')
            && !leaf.contains("..")
            && leaf
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b)),
        format!("不正なエントリ名です: {n}"),
    )
}
fn slice(b: &[u8], at: usize, n: usize) -> Result<&[u8]> {
    b.get(
        at..at
            .checked_add(n)
            .ok_or_else(|| Error::Budget("長さが過大です".into()))?,
    )
    .ok_or_else(|| Error::InvalidData("ZIPが途中で切れています".into()))
}
fn u16at(b: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(slice(b, at, 2)?.try_into().unwrap()))
}
fn u32at(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(slice(b, at, 4)?.try_into().unwrap()))
}
fn put16(b: &mut Vec<u8>, v: u16) {
    b.extend(v.to_le_bytes())
}
fn put32(b: &mut Vec<u8>, v: u32) {
    b.extend(v.to_le_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zip64_and_missing_data_descriptor_are_refused() {
        for descriptor in [false, true] {
            let mut b = sample().to_bytes().unwrap();
            let central = u32at(&b, b.len() - 6).unwrap() as usize;
            if descriptor {
                b[6..8].copy_from_slice(&8u16.to_le_bytes());
                b[central + 8..central + 10].copy_from_slice(&8u16.to_le_bytes());
            } else {
                b[4..6].copy_from_slice(&45u16.to_le_bytes());
                b[central + 6..central + 8].copy_from_slice(&45u16.to_le_bytes());
            }
            assert!(Archive::read(&b).is_err());
        }
        assert!(check_extra(&[1, 0, 0, 0]).is_err());
        assert!(check_extra(&[2, 0, 3, 0]).is_err());
    }
    fn sample() -> Archive {
        Archive::from_entries(BTreeMap::from([
            ("document.utpaint".into(), vec![42; 256]),
            ("unknown.bin".into(), vec![0, 1, 2, 3]),
        ]))
        .unwrap()
    }
    #[test]
    fn zip_and_manifest_roundtrip() {
        let a = sample();
        let b = Archive::read(&a.to_bytes().unwrap()).unwrap();
        assert_eq!(a.files, b.files);
        assert_eq!(a.manifest, b.manifest);
    }
    #[test]
    fn unknown_manifest_version_is_refused() {
        let mut a = sample();
        a.manifest = Arc::from(
            String::from_utf8(a.manifest.to_vec())
                .unwrap()
                .replace("YLP-3", "YLP-4")
                .into_bytes(),
        );
        assert!(Archive::read(&a.to_bytes().unwrap())
            .unwrap_err()
            .to_string()
            .contains("未対応"));
    }
    #[test]
    fn unknown_or_missing_manifest_entries_are_refused() {
        let mut a = sample();
        a.files.insert("extra.bin".into(), Arc::from([1u8]));
        assert!(Archive::read(&a.to_bytes().unwrap()).is_err());
        let mut a = sample();
        a.files.remove("unknown.bin");
        assert!(Archive::read(&a.to_bytes().unwrap()).is_err());
    }
    #[test]
    fn manifest_hash_length_duplicate_and_budget_are_checked() {
        let a = sample();
        let text = std::str::from_utf8(&a.manifest).unwrap();
        let line = text.lines().nth(1).unwrap();
        for bad in [
            text.replace(&hash(&a.files["document.utpaint"]), &"0".repeat(64)),
            text.replace(" 256 ", " 255 "),
            text.replace(" 256 ", " 536870913 "),
            format!("{text}{line}\n"),
            text.replace(" 256 ", " -1 "),
        ] {
            let mut a = a.clone();
            a.manifest = Arc::from(bad.into_bytes());
            assert!(Archive::read(&a.to_bytes().unwrap()).is_err());
        }
    }
    #[test]
    fn crc_is_checked_independently_of_manifest() {
        let mut b = sample().to_bytes().unwrap();
        let end = b.len() - 22;
        let central = u32at(&b, end + 16).unwrap() as usize;
        b[14..18].copy_from_slice(&0u32.to_le_bytes());
        b[central + 16..central + 20].copy_from_slice(&0u32.to_le_bytes());
        assert!(Archive::read(&b).unwrap_err().to_string().contains("CRC"));
    }
    #[test]
    fn dangerous_names_and_limits_are_refused() {
        for n in [
            "../bad",
            ".hidden",
            "a..b",
            "/absolute",
            "sets/BAD/document.utpaint",
            "resources/nested/x.png",
            "composite/../x.png",
            "a\\b",
            "mimetype",
            "manifest.sha256",
        ] {
            let mut f = BTreeMap::from([("document.utpaint".into(), vec![1])]);
            f.insert(n.into(), vec![1]);
            assert!(Archive::from_entries(f).is_err(), "{n}");
        }
        let mut a = sample();
        for i in 0..1000 {
            a.files.insert(format!("x{i}"), Arc::from([0]));
        }
        assert!(
            Archive::build(a.files, 3, "application/x-yolupainter", "YOLUPAINTER-YLP-").is_err()
        );
    }
    #[test]
    fn local_header_and_truncated_zip_are_refused() {
        let b = sample().to_bytes().unwrap();
        for n in 0..b.len() {
            assert!(Archive::read(&b[..n]).is_err(), "{n}");
        }
        for (offset, val) in [(8, 8), (26, 9), (28, 1)] {
            let mut bad = b.clone();
            bad[offset] = val;
            assert!(Archive::read(&bad).is_err());
        }
    }
    #[test]
    fn oversized_declared_entry_is_refused_before_inflation() {
        let mut b = sample().to_bytes().unwrap();
        let end = b.len() - 22;
        let mut at = u32at(&b, end + 16).unwrap() as usize;
        at += 46 + u16at(&b, at + 28).unwrap() as usize;
        at += 46 + u16at(&b, at + 28).unwrap() as usize;
        let local = u32at(&b, at + 42).unwrap() as usize;
        let size = (MAX_ENTRY_BYTES as u32 + 1).to_le_bytes();
        b[at + 24..at + 28].copy_from_slice(&size);
        b[local + 22..local + 26].copy_from_slice(&size);
        let error = Archive::read(&b).unwrap_err();
        assert!(matches!(error, Error::Budget(_)), "{error:?}");
        assert!(error.to_string().contains("予算"));
    }

    // ───────────── 壊れた・偽った ZIP を作る道具と、書いた ZIP を読む道具 ─────────────

    /// 試験の間、このスレッドが 1 回で求めた確保の最大の大きさを測る。宣言の大きさだけで先に確保しないことの確認に使う。
    mod alloc_probe {
        use std::alloc::{GlobalAlloc, Layout, System};
        use std::cell::Cell;
        thread_local! { static LARGEST: Cell<usize> = const { Cell::new(0) }; }
        pub struct Probe;
        fn note(n: usize) {
            let _ = LARGEST.try_with(|c| {
                if n > c.get() {
                    c.set(n)
                }
            });
        }
        // SAFETY: 確保そのものは System にそのまま渡し、大きさを覚えるだけ。
        unsafe impl GlobalAlloc for Probe {
            unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
                note(layout.size());
                unsafe { System.alloc(layout) }
            }
            unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
                note(layout.size());
                unsafe { System.alloc_zeroed(layout) }
            }
            unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
                note(new_size);
                unsafe { System.realloc(ptr, layout, new_size) }
            }
            unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
                unsafe { System.dealloc(ptr, layout) }
            }
        }
        #[global_allocator]
        static ALLOCATOR: Probe = Probe;
        /// `f` を実行し、その間に 1 回で求められた確保の最大のバイト数を返す。
        pub fn largest_request<T>(f: impl FnOnce() -> T) -> (T, usize) {
            LARGEST.with(|c| c.set(0));
            let value = f();
            (value, LARGEST.with(|c| c.replace(0)))
        }
    }
    use alloc_probe::largest_request;
    /// ZIP が小さいときに、読みが求めてよい 1 回の確保の上限（宣言の大きさに釣られていない目安）。
    const SMALL_REQUEST: usize = 1 << 20;
    /// 探りが生きている（`#[global_allocator]` が効き、大きな確保を捉える）ことの陽性対照。これが無いと、探りが壊れても
    /// 「確保が小さい」の断言がすべて素通りで通る。新しい確保（0 初期化も）・伸ばす確保・確保の無い処理の 3 通りを見る。
    #[test]
    fn the_allocation_probe_sees_large_requests() {
        use std::hint::black_box;
        const BIG: usize = 4 << 20;
        let (v, largest) = largest_request(|| black_box(vec![1u8; BIG]));
        assert_eq!(v.len(), BIG);
        assert!(largest >= BIG, "新しい確保: {largest}");
        let (v, largest) = largest_request(|| black_box(vec![0u8; BIG]));
        assert_eq!(v.len(), BIG);
        assert!(largest >= BIG, "0 初期化の確保: {largest}");
        let (v, largest) = largest_request(|| {
            let mut v = Vec::<u8>::with_capacity(16);
            v.resize(black_box(BIG), 1);
            v
        });
        assert_eq!(v.len(), BIG);
        assert!(largest >= BIG, "伸ばす確保: {largest}");
        // 小さな処理は小さいまま（測りは前の回の値を引きずらない）
        let (_, largest) = largest_request(|| black_box(vec![1u8; 1024]));
        assert!((1024..SMALL_REQUEST).contains(&largest), "{largest}");
    }

    /// 固定の種の乱数（splitmix64）。
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }
    fn noise(len: usize, seed: u64) -> Vec<u8> {
        let mut rng = Rng(seed);
        (0..len).map(|_| rng.next() as u8).collect()
    }
    /// PNG の署名の後ろが 0 だけ（圧縮すればよく縮むが、名前が .png なので無圧縮で入るはず）。
    fn fake_png(len: usize) -> Vec<u8> {
        let mut b = vec![0u8; len];
        b[..8].copy_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        b
    }
    const NATIVE: &str = "document.utpaint";
    const MIME: &str = "application/x-yolupainter";
    fn full_sample() -> BTreeMap<String, Vec<u8>> {
        BTreeMap::from([
            (NATIVE.into(), noise(3000, 1)),
            ("composite/Color.png".into(), fake_png(2000)),
            (
                "notes.txt".into(),
                "compressible text ".repeat(200).into_bytes(),
            ),
        ])
    }
    fn small_sample() -> BTreeMap<String, Vec<u8>> {
        BTreeMap::from([
            (NATIVE.into(), noise(120, 2)),
            ("composite/Color.png".into(), fake_png(40)),
            ("notes.txt".into(), "abc ".repeat(60).into_bytes()),
        ])
    }
    fn archive_of(files: &BTreeMap<String, Vec<u8>>) -> Archive {
        Archive::from_entries(files.clone()).unwrap()
    }
    fn line(name: &str, data: &[u8]) -> String {
        format!("{} {} {}", hash(data), data.len(), name)
    }
    fn manifest_bytes(header: &str, lines: &[String]) -> Vec<u8> {
        let mut text = format!("{header}\n");
        for l in lines {
            text.push_str(l);
            text.push('\n');
        }
        text.into_bytes()
    }
    fn file_lines(files: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
        files.iter().map(|(n, d)| line(n, d)).collect()
    }
    const DOS_DATE: u16 = (1 << 5) | 1;

    #[derive(Clone)]
    struct Entry {
        name: String,
        data: Vec<u8>,
        payload: Vec<u8>,
        extra: Vec<u8>,
        method: u16,
        flags: u16,
        crc: u32,
        csize: u32,
        usize_: u32,
        /// `Some(署名つきか)` ならデータ記述子（ビット 3）で書く。
        descriptor: Option<bool>,
    }
    impl Entry {
        fn stored(name: &str, data: &[u8]) -> Self {
            Self {
                name: name.into(),
                data: data.to_vec(),
                payload: data.to_vec(),
                extra: Vec::new(),
                method: 0,
                flags: if name.is_ascii() { 0 } else { 0x800 },
                crc: crc32fast::hash(data),
                csize: data.len() as u32,
                usize_: data.len() as u32,
                descriptor: None,
            }
        }
        /// raw deflate で入れる。`force` でなければ、縮まないときは無圧縮にする（`to_bytes` と同じ）。
        fn deflated(name: &str, data: &[u8], force: bool) -> Self {
            let mut e = Self::stored(name, data);
            let mut encoder =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(data).unwrap();
            let packed = encoder.finish().unwrap();
            if force || packed.len() < data.len() {
                e.csize = packed.len() as u32;
                e.payload = packed;
                e.method = 8;
            }
            e
        }
    }
    /// 壊れた・偽った ZIP を作る最小の組み立て。実体の並び（`entries`）と中央ディレクトリの並び（`central_order`）を別々に決められ、
    /// 方式・CRC・宣言する大きさ・拡張フィールドを自由に偽れる。`hidden` は中央ディレクトリに載せないローカルエントリで、先頭に置く。
    #[derive(Default)]
    struct ZipBuilder {
        entries: Vec<Entry>,
        hidden: Vec<Entry>,
        central_order: Option<Vec<usize>>,
    }
    impl ZipBuilder {
        fn index(&self, name: &str) -> usize {
            self.entries.iter().position(|e| e.name == name).unwrap()
        }
        fn write_local(out: &mut Vec<u8>, e: &Entry) {
            let flags = if e.descriptor.is_some() {
                e.flags | 8
            } else {
                e.flags
            };
            put32(out, 0x04034b50);
            for n in [20, flags, e.method, 0, DOS_DATE] {
                put16(out, n);
            }
            let (crc, csize, usize_) = if e.descriptor.is_some() {
                (0, 0, 0)
            } else {
                (e.crc, e.csize, e.usize_)
            };
            for n in [crc, csize, usize_] {
                put32(out, n);
            }
            put16(out, e.name.len() as u16);
            put16(out, e.extra.len() as u16);
            out.extend(e.name.as_bytes());
            out.extend(&e.extra);
            out.extend(&e.payload);
            if let Some(signed) = e.descriptor {
                if signed {
                    put32(out, 0x08074b50);
                }
                for n in [e.crc, e.csize, e.usize_] {
                    put32(out, n);
                }
            }
        }
        fn build(&self) -> Vec<u8> {
            let mut out = Vec::new();
            for e in &self.hidden {
                Self::write_local(&mut out, e);
            }
            let mut offsets = Vec::new();
            for e in &self.entries {
                offsets.push(out.len() as u32);
                Self::write_local(&mut out, e);
            }
            let start = out.len() as u32;
            let order: Vec<usize> = self
                .central_order
                .clone()
                .unwrap_or_else(|| (0..self.entries.len()).collect());
            for &i in &order {
                let e = &self.entries[i];
                let flags = if e.descriptor.is_some() {
                    e.flags | 8
                } else {
                    e.flags
                };
                put32(&mut out, 0x02014b50);
                for n in [20, 20, flags, e.method, 0, DOS_DATE] {
                    put16(&mut out, n);
                }
                for n in [e.crc, e.csize, e.usize_] {
                    put32(&mut out, n);
                }
                put16(&mut out, e.name.len() as u16);
                for _ in 0..4 {
                    put16(&mut out, 0);
                }
                put32(&mut out, 0);
                put32(&mut out, offsets[i]);
                out.extend(e.name.as_bytes());
            }
            let length = out.len() as u32 - start;
            put32(&mut out, 0x06054b50);
            for n in [0, 0, order.len() as u16, order.len() as u16] {
                put16(&mut out, n);
            }
            put32(&mut out, length);
            put32(&mut out, start);
            put16(&mut out, 0);
            out
        }
    }
    /// `to_bytes` と同じ並び（mimetype・manifest・名前順）の .ylp を組み立てで作る。壊すのは呼び出し側。
    fn standard_ylp(files: &BTreeMap<String, Vec<u8>>) -> ZipBuilder {
        standard_ylp_with(files, "YOLUPAINTER-YLP-3", &file_lines(files))
    }
    fn standard_ylp_with(
        files: &BTreeMap<String, Vec<u8>>,
        header: &str,
        lines: &[String],
    ) -> ZipBuilder {
        let mut z = ZipBuilder::default();
        z.entries.push(Entry::stored("mimetype", MIME.as_bytes()));
        z.entries.push(Entry::deflated(
            MANIFEST,
            &manifest_bytes(header, lines),
            false,
        ));
        for (name, data) in files {
            z.entries.push(if name.ends_with(".png") {
                Entry::stored(name, data)
            } else {
                Entry::deflated(name, data, false)
            });
        }
        z
    }

    #[derive(Debug)]
    struct Header {
        name: String,
        method: u16,
        flags: u16,
        time: u16,
        date: u16,
        crc: u32,
        csize: u32,
        usize_: u32,
        offset: usize,
        data_offset: usize,
        local_offset: usize,
        extra: usize,
        comment: usize,
    }
    /// ファイルの先頭からローカルヘッダーを順にたどる（実体の並び）。
    fn local_headers(zip: &[u8]) -> Vec<Header> {
        let mut result = Vec::new();
        let mut at = 0;
        while at + 30 <= zip.len() && u32at(zip, at).unwrap() == 0x04034b50 {
            let name_len = u16at(zip, at + 26).unwrap() as usize;
            let extra = u16at(zip, at + 28).unwrap() as usize;
            let h = Header {
                name: String::from_utf8(zip[at + 30..at + 30 + name_len].to_vec()).unwrap(),
                method: u16at(zip, at + 8).unwrap(),
                flags: u16at(zip, at + 6).unwrap(),
                time: u16at(zip, at + 10).unwrap(),
                date: u16at(zip, at + 12).unwrap(),
                crc: u32at(zip, at + 14).unwrap(),
                csize: u32at(zip, at + 18).unwrap(),
                usize_: u32at(zip, at + 22).unwrap(),
                offset: at,
                data_offset: at + 30 + name_len + extra,
                local_offset: at,
                extra,
                comment: 0,
            };
            at = h.data_offset + h.csize as usize;
            result.push(h);
        }
        result
    }
    /// 末尾の終端レコード（コメント無し）から中央ディレクトリを読む。
    fn central_headers(zip: &[u8]) -> Vec<Header> {
        let end = zip.len() - 22;
        assert_eq!(u32at(zip, end).unwrap(), 0x06054b50);
        let count = u16at(zip, end + 10).unwrap() as usize;
        let mut at = u32at(zip, end + 16).unwrap() as usize;
        let mut result = Vec::new();
        for _ in 0..count {
            assert_eq!(u32at(zip, at).unwrap(), 0x02014b50);
            let name_len = u16at(zip, at + 28).unwrap() as usize;
            let extra = u16at(zip, at + 30).unwrap() as usize;
            let comment = u16at(zip, at + 32).unwrap() as usize;
            result.push(Header {
                name: String::from_utf8(zip[at + 46..at + 46 + name_len].to_vec()).unwrap(),
                method: u16at(zip, at + 10).unwrap(),
                flags: u16at(zip, at + 8).unwrap(),
                time: u16at(zip, at + 12).unwrap(),
                date: u16at(zip, at + 14).unwrap(),
                crc: u32at(zip, at + 16).unwrap(),
                csize: u32at(zip, at + 20).unwrap(),
                usize_: u32at(zip, at + 24).unwrap(),
                offset: at,
                data_offset: 0,
                local_offset: u32at(zip, at + 42).unwrap() as usize,
                extra,
                comment,
            });
            at += 46 + name_len + extra + comment;
        }
        result
    }
    /// 標準の CRC-32（試験の独立した実装。crc32fast を使わない）。
    fn crc32_bitwise(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &b in data {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    0xedb8_8320 ^ (crc >> 1)
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    fn refusal(bytes: &[u8]) -> Error {
        Archive::read(bytes).unwrap_err()
    }
    #[track_caller]
    fn assert_invalid(bytes: &[u8], reason: &str) {
        let error = refusal(bytes);
        assert!(
            matches!(error, Error::InvalidData(_)),
            "{reason}: {error:?}"
        );
        assert!(error.to_string().contains(reason), "{reason}: {error}");
    }

    // ───────────── 書き方: 並び・識別子・圧縮・整合 ─────────────

    #[test]
    fn mimetype_is_the_first_stored_entry_so_the_magic_sits_at_offset_38() {
        let zip = archive_of(&full_sample()).to_bytes().unwrap();
        assert_eq!(
            u32at(&zip, 0).unwrap(),
            0x04034b50,
            "local header signature"
        );
        assert_eq!(u16at(&zip, 8).unwrap(), 0, "method: stored");
        assert_eq!(
            u32at(&zip, 18).unwrap() as usize,
            MIME.len(),
            "compressed size"
        );
        assert_eq!(
            u32at(&zip, 22).unwrap() as usize,
            MIME.len(),
            "uncompressed size"
        );
        assert_eq!(u16at(&zip, 26).unwrap(), 8, "name length");
        assert_eq!(u16at(&zip, 28).unwrap(), 0, "no extra field (ODF)");
        assert_eq!(&zip[30..38], b"mimetype");
        assert_eq!(&zip[38..38 + MIME.len()], MIME.as_bytes());
    }
    #[test]
    fn entries_are_written_mimetype_manifest_then_name_order_in_both_directories() {
        let mut files = full_sample();
        files.insert("Zeta.bin".into(), vec![1]);
        files.insert("alpha.bin".into(), vec![2]);
        let zip = archive_of(&files).to_bytes().unwrap();
        let mut expected = vec!["mimetype".to_string(), MANIFEST.to_string()];
        let mut names: Vec<String> = files.keys().cloned().collect();
        names.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        expected.extend(names);
        let local: Vec<String> = local_headers(&zip).into_iter().map(|h| h.name).collect();
        let central: Vec<String> = central_headers(&zip).into_iter().map(|h| h.name).collect();
        assert_eq!(local, expected, "実体の並び");
        assert_eq!(central, expected, "中央ディレクトリの並び");
        // 大文字は小文字より前（バイト順）
        assert!(
            expected.iter().position(|n| n == "Zeta.bin")
                < expected.iter().position(|n| n == "alpha.bin")
        );
    }
    #[test]
    fn png_is_stored_compressible_is_deflated_incompressible_is_stored() {
        let mut files = full_sample();
        files.insert("noise.bin".into(), noise(4096, 9));
        files.insert("empty.bin".into(), Vec::new());
        files.insert("one.bin".into(), vec![7]);
        let zip = archive_of(&files).to_bytes().unwrap();
        let local: BTreeMap<String, Header> = local_headers(&zip)
            .into_iter()
            .map(|h| (h.name.clone(), h))
            .collect();
        let png = &local["composite/Color.png"];
        assert_eq!(
            png.method, 0,
            "PNG は圧縮済みなので、よく縮む中身でも無圧縮"
        );
        assert_eq!(png.csize as usize, files["composite/Color.png"].len());
        let text = &local["notes.txt"];
        assert_eq!(text.method, 8, "縮む物は deflate");
        assert!((text.csize as usize) < files["notes.txt"].len());
        let random = &local["noise.bin"];
        assert_eq!(random.method, 0, "deflate が膨らませる中身は無圧縮");
        assert_eq!(random.csize, 4096);
        for tiny in ["empty.bin", "one.bin"] {
            assert_eq!(local[tiny].method, 0, "{tiny}");
            assert_eq!(local[tiny].csize, local[tiny].usize_, "{tiny}");
        }
        assert_eq!(local["mimetype"].method, 0);
        // どの方式で書いても読み直せて同じ中身
        let read = Archive::read(&zip).unwrap();
        for (name, data) in &files {
            assert_eq!(read.files[name].as_ref(), data.as_slice(), "{name}");
        }
    }
    /// 他の ZIP 実装が読めるよう、CRC・大きさ・オフセットが独立に計算した値と合い、ローカルヘッダーと中央ディレクトリが食い違わないこと。
    #[test]
    fn written_zip_is_internally_consistent_for_other_zip_readers() {
        assert_eq!(
            crc32_bitwise(b"123456789"),
            0xcbf4_3926,
            "試験の CRC-32 が標準の確認値と合う"
        );
        let mut files = full_sample();
        files.insert("noise.bin".into(), noise(777, 3));
        let archive = archive_of(&files);
        let zip = archive.to_bytes().unwrap();
        assert_eq!(
            zip,
            archive.to_bytes().unwrap(),
            "同じ中身はいつも同じバイト列"
        );
        assert_eq!(zip, archive_of(&files).to_bytes().unwrap());
        let local = local_headers(&zip);
        let central = central_headers(&zip);
        assert_eq!(local.len(), central.len());
        let mut expected_content: BTreeMap<String, Vec<u8>> = files.clone();
        expected_content.insert("mimetype".into(), MIME.as_bytes().to_vec());
        expected_content.insert(MANIFEST.into(), archive.manifest().to_vec());
        let mut next_offset = 0;
        for (l, c) in local.iter().zip(&central) {
            assert_eq!(l.name, c.name);
            assert_eq!(c.local_offset, l.offset, "{}", l.name);
            assert_eq!(l.offset, next_offset, "{}: 実体の間に隙間がない", l.name);
            next_offset = l.data_offset + l.csize as usize;
            assert_eq!(
                (c.method, c.crc, c.csize, c.usize_, c.flags, c.time, c.date),
                (l.method, l.crc, l.csize, l.usize_, l.flags, l.time, l.date),
                "{}",
                l.name
            );
            assert_eq!(
                (l.extra, c.extra, c.comment),
                (0, 0, 0),
                "{}: 追加の欄・コメントなし",
                l.name
            );
            assert_eq!(l.flags & 8, 0, "{}: データ記述子を使わない", l.name);
            let payload = &zip[l.data_offset..l.data_offset + l.csize as usize];
            let content = if l.method == 0 {
                payload.to_vec()
            } else {
                assert_eq!(l.method, 8, "{}", l.name);
                let mut v = Vec::new();
                flate2::read::DeflateDecoder::new(payload)
                    .read_to_end(&mut v)
                    .unwrap();
                v
            };
            assert_eq!(content, expected_content[&l.name], "{}", l.name);
            assert_eq!(l.crc, crc32_bitwise(&content), "{} の CRC-32", l.name);
            assert_eq!(l.usize_ as usize, content.len(), "{}", l.name);
            let (month, day) = ((l.date >> 5) & 15, l.date & 31);
            assert!(
                (1..=12).contains(&month) && (1..=31).contains(&day),
                "{}: 日付",
                l.name
            );
        }
        // 終端レコードは末尾の 22 バイト（コメントなし・後ろに何も足さない）。中央ディレクトリは実体の直後
        let end = zip.len() - 22;
        assert_eq!(u32at(&zip, end).unwrap(), 0x06054b50);
        assert_eq!(u16at(&zip, end + 8).unwrap() as usize, local.len());
        assert_eq!(u16at(&zip, end + 10).unwrap() as usize, local.len());
        assert_eq!(u32at(&zip, end + 16).unwrap() as usize, next_offset);
        assert_eq!(u32at(&zip, end + 12).unwrap() as usize + next_offset, end);
    }
    #[test]
    fn manifest_lists_hash_length_and_name_of_every_entry_in_order() {
        let mut files = full_sample();
        files.insert("Zeta.bin".into(), vec![1, 2, 3]);
        let archive = archive_of(&files);
        let mut names: Vec<&String> = files.keys().collect();
        names.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        let mut expected = String::from("YOLUPAINTER-YLP-3\n");
        for name in names {
            expected.push_str(&line(name, &files[name]));
            expected.push('\n');
        }
        assert_eq!(std::str::from_utf8(archive.manifest()).unwrap(), expected);
        // ZIP の中の manifest も同じバイト列
        let read = Archive::read(&archive.to_bytes().unwrap()).unwrap();
        assert_eq!(read.manifest(), expected.as_bytes());
    }
    #[test]
    fn longest_allowed_names_and_a_thousand_entries_round_trip() {
        let mut files = BTreeMap::from([(NATIVE.to_string(), vec![1u8])]);
        files.insert(format!("composite/{}", "c".repeat(86)), vec![2]); // 96 文字ちょうど
        files.insert("r".repeat(96), vec![3]);
        files.insert(format!("resources/{}", "s".repeat(86)), vec![4]);
        let mut i = 0;
        while files.len() < 1000 {
            files.insert(format!("e{i}"), vec![i as u8]);
            i += 1;
        }
        let zip = archive_of(&files).to_bytes().unwrap();
        let read = Archive::read(&zip).unwrap();
        assert_eq!(read.files.len(), 1000);
        for (name, data) in &files {
            assert_eq!(read.files[name].as_ref(), data.as_slice(), "{name}");
        }
        assert_eq!(local_headers(&zip).len(), 1002);
    }

    // ───────────── 書き出しの拒否 ─────────────

    /// 英数字と . - _ 以外・フォルダー・長すぎる名前など、書いても読んでも断る名前。
    fn unsafe_names() -> Vec<String> {
        let mut names: Vec<String> = [
            "../x",
            "/abs",
            "a\\b",
            "日本語.bin",
            "café.bin",
            ".hidden",
            "composite/.hidden",
            "sub/x.bin",
            "composite/",
            "composite/sub/x.png",
            "composite/../x",
            "composite//x.png",
            "a..b",
            " ",
            "a b",
            "a:b",
            "a\nb",
            "a\0b",
            "a\tb",
            "C:x",
            "x/",
            "./x",
            "COMPOSITE/x.png",
            "resources/",
            "resources/a/b",
            "resources/.x",
            "sets/X/document.utpaint",
            "sets/00000000-0000-0000-0000-000000000000/x",
        ]
        .map(String::from)
        .to_vec();
        names.push("n".repeat(97));
        names.push(format!("composite/{}", "c".repeat(87)));
        names
    }
    #[test]
    fn write_refuses_a_missing_native_document() {
        assert!(Archive::from_entries(BTreeMap::new()).is_err());
        // 名前は大文字・小文字を区別する
        let files = BTreeMap::from([("Document.utpaint".to_string(), vec![1u8])]);
        let error = Archive::from_entries(files).unwrap_err();
        assert!(matches!(error, Error::InvalidData(_)), "{error:?}");
        assert!(error.to_string().contains("正本"));
        // 正本は根かテクスチャセットの下にあればよい
        let set = "sets/01234567-89ab-cdef-0123-456789abcdef/document.utpaint";
        assert!(Archive::from_entries(BTreeMap::from([(set.to_string(), vec![1u8])])).is_ok());
    }
    #[test]
    fn write_refuses_every_unsafe_name() {
        for name in unsafe_names() {
            let mut files = full_sample();
            files.insert(name.clone(), vec![1]);
            let error = Archive::from_entries(files).unwrap_err();
            assert!(
                matches!(error, Error::InvalidData(_)),
                "{name:?}: {error:?}"
            );
            assert!(
                error.to_string().contains("不正なエントリ名"),
                "{name:?}: {error}"
            );
        }
    }
    // ───────────── 読みの拒否: ZIP として壊れている ─────────────

    #[test]
    fn things_that_are_not_zips_are_refused() {
        let inputs: Vec<Vec<u8>> = vec![
            Vec::new(),
            vec![0x50, 0x4b],
            noise(1000, 4),
            MIME.as_bytes().to_vec(),
            fake_png(64),
            vec![0; 22],
            b"PK\x03\x04".to_vec(),
            b"PK\x05\x06".to_vec(),
            [b"PK\x05\x06".as_slice(), &[0; 18]].concat(),
        ];
        for bytes in inputs {
            let error = refusal(&bytes);
            assert!(
                matches!(error, Error::InvalidData(_) | Error::Budget(_)),
                "長さ {}: {error:?}",
                bytes.len()
            );
        }
    }
    #[test]
    fn an_empty_zip_is_refused() {
        assert!(matches!(
            refusal(&ZipBuilder::default().build()),
            Error::InvalidData(_)
        ));
    }
    #[test]
    fn every_truncation_of_a_written_zip_is_refused_with_a_kind_the_screen_can_name() {
        let zip = archive_of(&small_sample()).to_bytes().unwrap();
        for n in 0..zip.len() {
            let error = refusal(&zip[..n]);
            assert!(
                matches!(error, Error::InvalidData(_) | Error::Budget(_)),
                "{n}: {error:?}"
            );
        }
    }

    // ───────────── 読みの拒否: mimetype ─────────────

    #[test]
    fn the_test_builder_makes_files_the_reader_accepts() {
        let files = full_sample();
        let read = Archive::read(&standard_ylp(&files).build()).unwrap();
        assert_eq!(read.files.len(), files.len());
        for (name, data) in &files {
            assert_eq!(read.files[name].as_ref(), data.as_slice(), "{name}");
        }
        // 書き手が作る ZIP と中身（manifest も）が同じ
        assert_eq!(read.manifest(), archive_of(&files).manifest());
    }
    #[test]
    fn a_missing_mimetype_is_refused() {
        let mut z = standard_ylp(&full_sample());
        z.entries.remove(0);
        assert_invalid(&z.build(), "mimetype");
    }
    #[test]
    fn a_mimetype_that_is_not_first_is_refused() {
        let mut z = standard_ylp(&full_sample());
        let mime = z.entries.remove(0);
        z.entries.insert(2, mime);
        assert_invalid(&z.build(), "mimetype");
    }
    #[test]
    fn a_wrong_mimetype_is_refused() {
        for mime in [
            "application/zip",
            "application/x-yolupainter\n",
            "APPLICATION/X-YOLUPAINTER",
            "application/x-yolupainter2",
            "application/x-yolupaintes",
            "",
        ] {
            let mut z = standard_ylp(&full_sample());
            z.entries[0] = Entry::stored("mimetype", mime.as_bytes());
            assert_invalid(&z.build(), "mimetype");
        }
    }
    #[test]
    fn a_second_mimetype_is_refused() {
        let mut z = standard_ylp(&full_sample());
        z.entries.push(Entry::stored("mimetype", MIME.as_bytes()));
        assert_invalid(&z.build(), "重複");
    }
    /// 先頭の mimetype は、識別（38 バイト目）の確認が先に宣言の長さを見る。偽の宣言は、予算ではなく「先頭に無圧縮の
    /// mimetype が無い」として、確保せずに断られる。
    #[test]
    fn a_first_mimetype_with_a_false_declared_size_is_not_the_stored_mimetype() {
        for declared in [300u32, 1 << 20, u32::MAX] {
            let mut z = standard_ylp(&full_sample());
            z.entries[0].usize_ = declared;
            let bytes = z.build();
            let (result, largest) = largest_request(|| Archive::read(&bytes));
            let error = result.unwrap_err();
            assert!(
                matches!(&error, Error::InvalidData(why) if why.contains("先頭に無圧縮のmimetype")),
                "{declared}: {error:?}"
            );
            assert!(largest < SMALL_REQUEST, "{declared}: 確保 {largest}");
        }
    }
    /// 2 つ目の mimetype（物理的に先頭ではない）には 256 バイトの上限がある。宣言だけ大きくても、確保せずに予算超過で断る。
    /// 257 は、上限が mimetype 専用の 256 でなく一般の項目の上限だったら通ってしまう境目（その場合は別の理由で断られ、予算超過にならない）。
    #[test]
    fn a_second_mimetype_declared_larger_than_its_budget_is_refused_without_allocating() {
        for declared in [257u32, 300, 1 << 20, u32::MAX] {
            let mut z = standard_ylp(&full_sample());
            let mut second = Entry::stored("mimetype", MIME.as_bytes());
            second.usize_ = declared;
            z.entries.push(second);
            let bytes = z.build();
            let (result, largest) = largest_request(|| Archive::read(&bytes));
            let error = result.unwrap_err();
            assert!(matches!(error, Error::Budget(_)), "{declared}: {error:?}");
            assert!(largest < SMALL_REQUEST, "{declared}: 確保 {largest}");
        }
        // 上限ちょうどの 256 は予算では断らない（重複として断る）
        let mut z = standard_ylp(&full_sample());
        let mut second = Entry::stored("mimetype", &[b'x'; 256]);
        second.usize_ = 256;
        z.entries.push(second);
        assert_invalid(&z.build(), "重複");
    }
    /// 識別は ODF / OpenRaster と同じく「先頭 30 バイト目に無圧縮・拡張フィールド無しの mimetype、38 バイト目に中身」。
    /// 中央ディレクトリの並びだけ先頭でも、実体が先頭でない・圧縮されている・拡張フィールドがあるファイルは .ylp ではない。
    #[test]
    fn files_whose_magic_is_not_at_offset_38_are_refused() {
        for variant in [
            "deflated",
            "extra-field",
            "not-physically-first",
            "data-before-mimetype",
        ] {
            let mut z = standard_ylp(&full_sample());
            match variant {
                "deflated" => z.entries[0] = Entry::deflated("mimetype", MIME.as_bytes(), true),
                "extra-field" => z.entries[0].extra = vec![0xfe, 0xca, 4, 0, 1, 2, 3, 4],
                "not-physically-first" => {
                    let first = z.entries.remove(0);
                    z.entries.push(first);
                    let n = z.entries.len();
                    z.central_order = Some(std::iter::once(n - 1).chain(0..n - 1).collect());
                }
                _ => z
                    .hidden
                    .push(Entry::stored("hidden.bin", b"not listed anywhere")),
            }
            let bytes = z.build();
            assert_ne!(
                bytes.get(38..38 + MIME.len()),
                Some(MIME.as_bytes()),
                "{variant}: 組んだ物に識別子が無い"
            );
            assert_invalid(&bytes, "mimetype");
        }
    }

    // ───────────── 読みの拒否: manifest ─────────────

    #[test]
    fn a_missing_manifest_is_refused() {
        let mut z = standard_ylp(&full_sample());
        z.entries.remove(1);
        assert_invalid(&z.build(), "manifest");
    }
    #[test]
    fn a_newer_or_unknown_manifest_header_is_refused() {
        for header in [
            "",
            "DOTPAINT-MANIFEST-1",
            "yolupainter-ylp-1",
            "YOLUPAINTER-YLP-0",
            "YOLUPAINTER-YLP-03",
            "YOLUPAINTER-YLP-+3",
            "YOLUPAINTER-YLP--1",
            "YOLUPAINTER-YLP-3 ",
            "YOLUPAINTER-YLP-4",
            "YOLUPAINTER-YLP-99999999999",
            "YOLUPAINTER-SMART-3",
        ] {
            let files = full_sample();
            let z = standard_ylp_with(&files, header, &file_lines(&files));
            let error = refusal(&z.build());
            assert!(
                matches!(error, Error::InvalidData(_)),
                "{header:?}: {error:?}"
            );
            assert!(
                error.to_string().contains("manifest"),
                "{header:?}: {error}"
            );
        }
    }
    #[test]
    fn a_manifest_without_the_native_document_is_refused() {
        let mut files = full_sample();
        files.remove(NATIVE);
        assert_invalid(&standard_ylp(&files).build(), "正本");
    }
    #[test]
    fn malformed_manifest_lines_are_refused() {
        let templates = [
            "{h} {l}",
            "{h} {l} {n} extra",
            "{h}0 {l} {n}",
            "{h7} {l} {n}",
            "{hx} {l} {n}",
            "{h} -1 {n}",
            "{h} x {n}",
            "{h} +5 {n}",
            "{h} {l}x {n}",
            "{h} 536870913 {n}",
            "{h} 99999999999999999999999 {n}",
            "{h} {l} {n}\n{h} {l} {n}",
            "{h} {l} mimetype",
            "{h} {l} manifest.sha256",
            "{h} {l} ../{n}",
            "{h}  {l} {n}",
            "{H} {l} {n}",
        ];
        for template in templates {
            let files = full_sample();
            let doc = &files[NATIVE];
            let h = hash(doc);
            let text = template
                .replace("{hx}", &format!("z{}", &h[1..]))
                .replace("{h7}", &h[1..])
                .replace("{H}", &h.to_uppercase())
                .replace("{h}", &h)
                .replace("{l}", &doc.len().to_string())
                .replace("{n}", NATIVE);
            let mut lines: Vec<String> = files
                .iter()
                .filter(|(n, _)| n.as_str() != NATIVE)
                .map(|(n, d)| line(n, d))
                .collect();
            lines.push(text);
            let error = refusal(&standard_ylp_with(&files, "YOLUPAINTER-YLP-3", &lines).build());
            assert!(
                matches!(error, Error::InvalidData(_) | Error::Budget(_)),
                "{template:?}: {error:?}"
            );
        }
    }
    #[test]
    fn a_manifest_that_is_not_utf8_is_refused() {
        let files = full_sample();
        let mut text = manifest_bytes("YOLUPAINTER-YLP-3", &file_lines(&files));
        text.extend([0xff, 0xfe, b'\n']);
        let mut z = standard_ylp(&files);
        z.entries[1] = Entry::stored(MANIFEST, &text);
        assert_invalid(&z.build(), "UTF-8");
    }
    #[test]
    fn manifest_totals_and_declared_sizes_above_the_budget_are_refused() {
        let files = full_sample();
        // 合計の宣言が 768 MiB を超える（1 つずつは 512 MiB 以下）
        let mut lines = file_lines(&files);
        let empty = hash(&[]);
        lines.push(format!("{empty} 419430400 big1.bin"));
        lines.push(format!("{empty} 419430400 big2.bin"));
        let error = refusal(&standard_ylp_with(&files, "YOLUPAINTER-YLP-3", &lines).build());
        assert!(matches!(error, Error::Budget(_)), "{error:?}");
        // manifest 自体が 1 MiB を超えると宣言する
        for declared in [2 * 1024 * 1024, u32::MAX] {
            let mut z = standard_ylp(&files);
            z.entries[1].usize_ = declared;
            let bytes = z.build();
            let (result, largest) = largest_request(|| Archive::read(&bytes));
            assert!(
                matches!(result, Err(Error::Budget(_))),
                "{declared}: {result:?}"
            );
            assert!(largest < SMALL_REQUEST, "{declared}: 確保 {largest}");
        }
    }

    // ───────────── 読みの拒否: 中身 ─────────────

    #[test]
    fn a_flipped_byte_inside_a_stored_entry_is_refused_naming_the_entry() {
        let zip = archive_of(&full_sample()).to_bytes().unwrap();
        let png = local_headers(&zip)
            .into_iter()
            .find(|h| h.name == "composite/Color.png")
            .unwrap();
        assert_eq!(png.method, 0);
        let mut bad = zip;
        bad[png.data_offset + 20] ^= 0x40;
        let error = refusal(&bad);
        assert!(matches!(error, Error::InvalidData(_)), "{error:?}");
        assert!(error.to_string().contains("composite/Color.png"), "{error}");
    }
    #[test]
    fn a_flipped_byte_inside_a_deflated_entry_is_refused_as_invalid_data() {
        let zip = archive_of(&full_sample()).to_bytes().unwrap();
        let notes = local_headers(&zip)
            .into_iter()
            .find(|h| h.name == "notes.txt")
            .unwrap();
        assert_eq!(notes.method, 8);
        for at in 0..notes.csize as usize {
            let mut bad = zip.clone();
            bad[notes.data_offset + at] ^= 0x10;
            let error = refusal(&bad);
            // 壊れた deflate が「ファイルの読み書きの失敗」（Io）に見えない: 壊れたデータとして断る
            assert!(
                matches!(error, Error::InvalidData(_)),
                "byte {at}: {error:?}"
            );
        }
    }
    #[test]
    fn a_wrong_zip_crc_is_refused_even_when_the_content_matches_the_manifest() {
        let mut z = standard_ylp(&full_sample());
        let i = z.index(NATIVE);
        z.entries[i].crc ^= 1;
        assert_invalid(&z.build(), "CRC");
    }
    #[test]
    fn duplicate_names_in_the_zip_are_refused() {
        for name in [NATIVE, MANIFEST, "composite/Color.png"] {
            let mut z = standard_ylp(&full_sample());
            let original = z.entries[z.index(name)].clone();
            z.entries.push(Entry::stored(name, &original.data));
            assert_invalid(&z.build(), "重複");
        }
    }
    #[test]
    fn unsafe_names_in_the_zip_are_refused() {
        for name in unsafe_names() {
            let files = full_sample();
            let mut z = standard_ylp(&files);
            let data = [1u8, 2, 3];
            z.entries.push(Entry::stored(&name, &data));
            // manifest にも載せて「載っていない」で落ちないようにする（空白・改行を含む名前は manifest の行に書けない）
            let listed = !name.contains(' ') && !name.contains('\n');
            if listed {
                let mut lines = file_lines(&files);
                lines.push(line(&name, &data));
                z.entries[1] = Entry::deflated(
                    MANIFEST,
                    &manifest_bytes("YOLUPAINTER-YLP-3", &lines),
                    false,
                );
            }
            let error = refusal(&z.build());
            assert!(
                matches!(error, Error::InvalidData(_)),
                "{name:?}: {error:?}"
            );
            let expected = if listed {
                "不正なエントリ名"
            } else {
                "manifest"
            };
            assert!(error.to_string().contains(expected), "{name:?}: {error}");
        }
    }
    #[test]
    fn an_unsupported_method_or_flag_is_refused() {
        for method in [1u16, 6, 9, 12, 14, 93, 95, 98, 99] {
            let mut z = standard_ylp(&full_sample());
            let i = z.index(NATIVE);
            z.entries[i].method = method;
            assert_invalid(&z.build(), "未対応");
        }
        // 暗号化・強い暗号化・ヘッダーのマスク・拡張した deflate
        for flags in [0x0001u16, 0x0040, 0x2000, 0x0010] {
            let mut z = standard_ylp(&full_sample());
            let i = z.index(NATIVE);
            z.entries[i].flags = flags;
            assert_invalid(&z.build(), "未対応");
        }
    }
    /// 詰め直したツール（Info-ZIP・7-Zip）は deflate の圧縮の水準の目安（ビット 1・2）を立てる。C# は旗を無視して読むので、
    /// Rust も deflate の項目では受け入れる（無圧縮の項目では意味の無い旗なので断る）。
    #[test]
    fn deflate_level_hints_are_accepted_for_deflated_entries_only() {
        for hint in [0x0002u16, 0x0004, 0x0006, 0x0806] {
            let files = full_sample();
            let mut z = standard_ylp(&files);
            for e in z.entries.iter_mut().filter(|e| e.method == 8) {
                e.flags = hint;
            }
            assert!(z.entries.iter().any(|e| e.method == 8));
            let read = Archive::read(&z.build()).unwrap_or_else(|e| panic!("{hint:#06x}: {e}"));
            for (name, data) in &files {
                assert_eq!(
                    read.files[name].as_ref(),
                    data.as_slice(),
                    "{hint:#06x} {name}"
                );
            }
        }
        // 水準の目安以外の旗（ビット 0・4・5・6・13 など）は deflate でも断る
        for flags in [0x0001u16, 0x0010, 0x0020, 0x0040, 0x2000, 0x0002 | 0x0001] {
            let mut z = standard_ylp(&full_sample());
            let i = z.index("notes.txt");
            assert_eq!(z.entries[i].method, 8);
            z.entries[i].flags = flags;
            assert_invalid(&z.build(), "未対応");
        }
        // 無圧縮の項目に水準の目安を立てたファイルは断る
        for hint in [0x0002u16, 0x0004, 0x0006] {
            let mut z = standard_ylp(&full_sample());
            let i = z.index("composite/Color.png");
            assert_eq!(z.entries[i].method, 0);
            z.entries[i].flags = hint;
            assert_invalid(&z.build(), "未対応");
        }
    }
    #[test]
    fn stored_bytes_longer_than_declared_are_refused() {
        let mut z = standard_ylp(&full_sample());
        let i = z.index("composite/Color.png");
        let mut payload = z.entries[i].data.clone();
        payload.extend([0u8; 100]);
        z.entries[i].csize = payload.len() as u32;
        z.entries[i].payload = payload;
        assert_invalid(&z.build(), "無圧縮");
    }
    #[test]
    fn a_zip_written_with_data_descriptors_is_read_and_a_wrong_descriptor_is_refused() {
        let files = full_sample();
        let mut z = standard_ylp(&files);
        // 先頭の mimetype は識別のために長さを前に書く（記述子は使えない）。残りは署名つき・なしを交互に
        for (i, e) in z.entries.iter_mut().enumerate().skip(1) {
            e.descriptor = Some(i % 2 == 0);
        }
        let read = Archive::read(&z.build()).unwrap();
        for (name, data) in &files {
            assert_eq!(read.files[name].as_ref(), data.as_slice(), "{name}");
        }
        // 記述子の CRC・大きさが中央ディレクトリと食い違えば断る（ローカルヘッダーの 0 を信用しない）
        let mut broken = standard_ylp(&files);
        for e in broken.entries.iter_mut().skip(1) {
            e.descriptor = Some(true);
        }
        // 記述子の中身は Entry の値で書くので、中央ディレクトリ側だけ変えるには組んだあとのバイト列を直す
        let bytes = broken.build();
        let headers = central_headers(&bytes);
        let doc = headers.iter().find(|h| h.name == NATIVE).unwrap();
        let descriptor = doc.local_offset + 30 + NATIVE.len() + doc.csize as usize;
        let mut bad = bytes;
        assert_eq!(u32at(&bad, descriptor).unwrap(), 0x08074b50);
        bad[descriptor + 4] ^= 1;
        assert_invalid(&bad, "データ記述子");
    }

    // ───────────── 展開爆弾・宣言を信用しない ─────────────

    /// 小さく宣言して大きく膨らむ（zip bomb）。宣言どおりの長さとそのハッシュを manifest に書いても通らず、宣言の長さを超えて展開しない。
    #[test]
    fn an_entry_that_inflates_far_beyond_its_declared_size_is_stopped_early() {
        let huge = vec![0u8; 16 * 1024 * 1024];
        let declared = huge[..64].to_vec();
        let mut files = full_sample();
        files.insert(NATIVE.into(), declared.clone());
        let mut z = standard_ylp(&files);
        let mut bomb = Entry::deflated(NATIVE, &huge, true);
        assert!(bomb.payload.len() < 64 * 1024, "組んだ圧縮データは小さい");
        bomb.crc = crc32fast::hash(&declared);
        bomb.usize_ = 64;
        let i = z.index(NATIVE);
        z.entries[i] = bomb;
        let bytes = z.build();
        let (result, largest) = largest_request(|| Archive::read(&bytes));
        let error = result.unwrap_err();
        assert!(matches!(error, Error::InvalidData(_)), "{error:?}");
        assert!(error.to_string().contains("Deflate"), "{error}");
        assert!(
            largest < SMALL_REQUEST,
            "16 MiB まで展開せず途中で止める: 確保 {largest}"
        );
    }
    /// 宣言より 1 バイト多く・少なく展開する物は断り、ちょうどの物は通る（境目）。
    #[test]
    fn inflating_one_byte_more_or_less_than_declared_is_refused_and_exact_is_read() {
        for (actual, accepted) in [(63usize, false), (64, true), (65, false)] {
            let declared = vec![5u8; 64];
            let mut files = full_sample();
            files.insert(NATIVE.into(), declared.clone());
            let mut z = standard_ylp(&files);
            let mut entry = Entry::deflated(NATIVE, &vec![5u8; actual], true);
            entry.usize_ = 64;
            entry.crc = crc32fast::hash(&declared);
            let i = z.index(NATIVE);
            z.entries[i] = entry;
            assert_eq!(Archive::read(&z.build()).is_ok(), accepted, "{actual}");
        }
    }
    /// 中身は 10 バイトなのに中央ディレクトリと manifest で大きく宣言する（予算内の嘘）。数百バイトのファイルで、
    /// 宣言どおりの大きさを先に確保させられない（宣言を信用しない）。
    #[test]
    fn a_tiny_entry_with_a_large_declared_size_does_not_preallocate() {
        for declared in [32u32 << 20, 400 << 20] {
            for deflate in [false, true] {
                let data = [9u8; 10];
                let mut files = full_sample();
                files.insert(NATIVE.into(), data.to_vec());
                let mut lines = file_lines(&files);
                *lines.iter_mut().find(|l| l.ends_with(NATIVE)).unwrap() =
                    format!("{} {declared} {NATIVE}", hash(&data));
                let mut z = standard_ylp_with(&files, "YOLUPAINTER-YLP-3", &lines);
                let i = z.index(NATIVE);
                z.entries[i] = if deflate {
                    Entry::deflated(NATIVE, &data, true)
                } else {
                    Entry::stored(NATIVE, &data)
                };
                z.entries[i].usize_ = declared;
                let bytes = z.build();
                assert!(bytes.len() < 16 * 1024);
                let (result, largest) = largest_request(|| Archive::read(&bytes));
                let error = result.unwrap_err();
                assert!(
                    matches!(error, Error::InvalidData(_)),
                    "{declared} {deflate}: {error:?}"
                );
                assert!(
                    largest < SMALL_REQUEST,
                    "{declared} {deflate}: {} バイトの ZIP で確保 {largest}",
                    bytes.len()
                );
            }
        }
    }
    #[test]
    fn a_manifest_that_inflates_beyond_its_budget_is_stopped_early() {
        let files = full_sample();
        let mut z = standard_ylp(&files);
        let mut manifest = manifest_bytes("YOLUPAINTER-YLP-3", &file_lines(&files));
        manifest.extend(std::iter::repeat_n(b'\n', 3 * 1024 * 1024));
        let mut bomb = Entry::deflated(MANIFEST, &manifest, true);
        bomb.usize_ = 1000;
        z.entries[1] = bomb;
        let bytes = z.build();
        let (result, largest) = largest_request(|| Archive::read(&bytes));
        let error = result.unwrap_err();
        assert!(matches!(error, Error::InvalidData(_)), "{error:?}");
        assert!(error.to_string().contains("Deflate"), "{error}");
        assert!(largest < SMALL_REQUEST, "確保 {largest}");
    }

    // ───────────── 壊れ方の網羅: 拒むか、同じ中身を返すか ─────────────

    /// 壊した ZIP を読み、拒む（`InvalidData`・`Budget`）か、元と同じ中身を返すかのどちらかであること。
    /// 違う中身を黙って返す・ほかの種類の失敗（Io など）・パニック・巨大な確保は、問題として溜める。
    fn judge(
        bad: &[u8],
        original: &Archive,
        what: impl Fn() -> String,
        problems: &mut Vec<String>,
    ) {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            largest_request(|| Archive::read(bad))
        }));
        match outcome {
            Err(_) => problems.push(format!("{}: パニック", what())),
            Ok((result, largest)) => {
                if largest >= SMALL_REQUEST {
                    problems.push(format!("{}: 確保 {largest}", what()));
                }
                match result {
                    Ok(read) => {
                        if read.files != original.files
                            || read.manifest != original.manifest
                            || read.level != original.level
                        {
                            problems.push(format!("{}: 違う中身で受理", what()));
                        }
                    }
                    Err(Error::InvalidData(_) | Error::Budget(_)) => {}
                    Err(other) => problems.push(format!("{}: {other:?}", what())),
                }
            }
        }
        // 流して読む新しい読み手（`Package`）も、今の形のファイルには同じ判定をする（受けるなら同じ中身、断るなら壊れた・予算の種類）
        let old = Archive::read(bad).ok();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            largest_request(|| crate::Package::read_bytes(bad, &crate::Limits::default()))
        }));
        match outcome {
            Err(_) => problems.push(format!("{}: 新しい読み手がパニック", what())),
            Ok((result, largest)) => {
                if largest >= SMALL_REQUEST {
                    problems.push(format!("{}: 新しい読み手の確保 {largest}", what()));
                }
                match (result, old) {
                    (Ok(read), Some(old)) => {
                        let same = read.level == old.level
                            && read.read_manifest() == Some(&old.manifest[..])
                            && read.files.len() == old.files.len()
                            && read
                                .files
                                .iter()
                                .all(|(k, v)| old.files.get(k).is_some_and(|o| v.in_memory() == Some(&o[..])));
                        if !same {
                            problems.push(format!("{}: 新しい読み手が違う中身で受理", what()));
                        }
                    }
                    (Ok(_), None) => problems.push(format!("{}: 新しい読み手だけが受理", what())),
                    (Err(e), Some(_)) => {
                        problems.push(format!("{}: 新しい読み手だけが拒否 {e:?}", what()))
                    }
                    (Err(Error::InvalidData(_) | Error::Budget(_)), None) => {}
                    (Err(other), None) => {
                        problems.push(format!("{}: 新しい読み手 {other:?}", what()))
                    }
                }
            }
        }
    }
    fn assert_no_problems(problems: &[String], started: std::time::Instant) {
        assert!(
            problems.is_empty(),
            "{} 件: {:?}",
            problems.len(),
            &problems[..problems.len().min(10)]
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
    }
    /// `YLP-4`（zip64 の位置と終端の記録つき）の壊し方の網羅: 1 バイトずつ・極端な値・乱数の複数バイト。断るか、同じ中身で読むか
    /// で、パニック・大きな確保・壊れたもの以外の種類の断りは無い。
    #[test]
    fn every_damage_of_a_ylp_4_zip64_file_is_refused_or_harmless() {
        let started = std::time::Instant::now();
        let t = crate::Thresholds {
            classic_total_bytes: 10,
            zip64_offset_at: 64,
            zip64_count_above: 2,
            ..crate::Thresholds::REAL
        };
        let files: crate::package::Files = small_sample()
            .into_iter()
            .map(|(k, v)| (k, crate::Blob::from(v)))
            .collect();
        let zip = t.scoped(|| crate::Package::build(files, 3).unwrap().to_bytes().unwrap());
        assert!(zip.windows(4).any(|w| w == b"PK\x06\x06"), "zip64 の終端がある");
        let limits = crate::Limits::default();
        let original = crate::Package::read_bytes(&zip, &limits).unwrap();
        assert_eq!(original.manifest_version(), 4);
        let mut problems = Vec::new();
        let (mut accepted, mut refused) = (0usize, 0usize);
        let mut judge4 = |bad: &[u8], what: &dyn Fn() -> String| {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                largest_request(|| crate::Package::read_bytes(bad, &limits))
            }));
            match outcome {
                Err(_) => problems.push(format!("{}: パニック", what())),
                Ok((result, largest)) => {
                    if largest >= SMALL_REQUEST {
                        problems.push(format!("{}: 確保 {largest}", what()));
                    }
                    match result {
                        Ok(read) => {
                            accepted += 1;
                            let same = read.read_manifest() == original.read_manifest()
                                && read.files.len() == original.files.len()
                                && read.files.iter().all(|(k, v)| {
                                    original.files.get(k).is_some_and(|o| v.in_memory() == o.in_memory())
                                });
                            if !same {
                                problems.push(format!("{}: 違う中身で受理", what()));
                            }
                        }
                        Err(Error::InvalidData(_) | Error::Budget(_)) => refused += 1,
                        Err(other) => problems.push(format!("{}: {other:?}", what())),
                    }
                }
            }
        };
        for mask in [0xffu8, 0x01, 0x80] {
            for i in 0..zip.len() {
                let mut bad = zip.clone();
                bad[i] ^= mask;
                judge4(&bad, &|| format!("offset {i} ^ {mask:#x}"));
            }
        }
        let patterns: [&[u8]; 4] = [&[0xff, 0xff, 0xff, 0xff], &[0, 0, 0, 0], &[0xff; 8], &[0; 8]];
        for pattern in patterns {
            for i in 0..=zip.len() - pattern.len() {
                let mut bad = zip.clone();
                bad[i..i + pattern.len()].copy_from_slice(pattern);
                judge4(&bad, &|| format!("offset {i} {pattern:02x?}"));
            }
        }
        let mut rng = Rng(20_261_005);
        for round in 0..3000 {
            let mut bad = zip.clone();
            for _ in 0..1 + rng.below(6) {
                let i = rng.below(bad.len());
                bad[i] = rng.next() as u8;
            }
            judge4(&bad, &|| format!("乱数 {round}"));
        }
        for n in 0..zip.len() {
            judge4(&zip[..n], &|| format!("切れた {n}"));
        }
        // 網羅が読み手を通っている（ほとんどを断り、時刻の欄などの無害な変更は受ける）
        assert!(refused > 10_000 && accepted > 10, "断った {refused}、受けた {accepted}");
        assert_no_problems(&problems, started);
    }
    /// 全バイト位置の網羅（上の 3 本）が対象にする ZIP の大きさ。「小さな ZIP」の主張を測った範囲に結びつけ、
    /// 見本を大きくして網羅が重くなったり、網羅の意味（全位置）が薄まったりしたら気づく。
    #[test]
    fn the_zip_swept_byte_by_byte_stays_about_a_kilobyte() {
        let zip = archive_of(&small_sample()).to_bytes().unwrap();
        assert!((512..=1024).contains(&zip.len()), "{} バイト", zip.len());
    }
    /// 1 バイトをどこで変えても、拒むか同じ中身を返すか。時刻や属性など中身に関わらない欄は変えても読めてよい。
    #[test]
    fn every_single_byte_change_is_refused_or_harmless() {
        let started = std::time::Instant::now();
        let original = archive_of(&small_sample());
        let zip = original.to_bytes().unwrap();
        let mut problems = Vec::new();
        for mask in [0xffu8, 0x01, 0x80] {
            for i in 0..zip.len() {
                let mut bad = zip.clone();
                bad[i] ^= mask;
                judge(
                    &bad,
                    &original,
                    || format!("offset {i} ^ {mask:#x}"),
                    &mut problems,
                );
            }
        }
        assert_no_problems(&problems, started);
    }
    /// どの位置にも、大きさ・オフセット・個数の欄で問題になりやすい値（0、zip64 の印 0xFFFFFFFF、符号の境目）を書き込む。
    #[test]
    fn extreme_field_values_anywhere_are_refused_or_harmless() {
        let started = std::time::Instant::now();
        let original = archive_of(&small_sample());
        let zip = original.to_bytes().unwrap();
        let patterns: [&[u8]; 6] = [
            &[0xff, 0xff, 0xff, 0xff],
            &[0, 0, 0, 0],
            &[0xff, 0xff, 0xff, 0x7f],
            &[0, 0, 0, 0x80],
            &[0xff, 0xff],
            &[0, 0],
        ];
        let mut problems = Vec::new();
        for pattern in patterns {
            for i in 0..=zip.len() - pattern.len() {
                let mut bad = zip.clone();
                bad[i..i + pattern.len()].copy_from_slice(pattern);
                judge(
                    &bad,
                    &original,
                    || format!("offset {i} {pattern:02x?}"),
                    &mut problems,
                );
            }
        }
        assert_no_problems(&problems, started);
    }
    /// 固定の種で、1〜6 バイトを同時に変えた版を 3000 通り読む。
    #[test]
    fn random_multi_byte_damage_is_refused_or_harmless() {
        let started = std::time::Instant::now();
        let original = archive_of(&small_sample());
        let zip = original.to_bytes().unwrap();
        let mut rng = Rng(20_261_002);
        let mut problems = Vec::new();
        for round in 0..3000 {
            let mut bad = zip.clone();
            let mut at = Vec::new();
            for _ in 0..1 + rng.below(6) {
                let i = rng.below(bad.len());
                bad[i] = rng.next() as u8;
                at.push(i);
            }
            judge(
                &bad,
                &original,
                || format!("round {round} at {at:?}"),
                &mut problems,
            );
        }
        assert_no_problems(&problems, started);
    }
    /// 書いた全項目の入った大きめの ZIP（deflate・無圧縮・PNG の混在）でも、同じ方針で数を絞って壊す。
    #[test]
    fn random_damage_of_a_larger_zip_is_refused_or_harmless() {
        let started = std::time::Instant::now();
        let original = archive_of(&full_sample());
        let zip = original.to_bytes().unwrap();
        let mut rng = Rng(7);
        let mut problems = Vec::new();
        for round in 0..300 {
            let mut bad = zip.clone();
            for _ in 0..1 + rng.below(4) {
                let i = rng.below(bad.len());
                bad[i] = rng.next() as u8;
            }
            judge(&bad, &original, || format!("round {round}"), &mut problems);
        }
        assert_no_problems(&problems, started);
    }
}
