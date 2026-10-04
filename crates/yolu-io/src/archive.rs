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
            let offset = u32::try_from(out.len()).map_err(|_| Error::InvalidData("zip64は未対応です".into()))?;
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
            check(
                u16at(b, at + 34)? == 0 && flags & !0x0808 == 0 && (method == 0 || method == 8),
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
                d.by_ref().take(len as u64 + 1).read_to_end(&mut v)?;
                check(
                    v.len() == len && d.total_in() == packed as u64,
                    "Deflateの長さが一致しません",
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
                listed.insert(name, ()).is_none(),
                "manifestに重複があります",
            )?;
            let data = files
                .get(name)
                .ok_or_else(|| Error::InvalidData(format!("エントリがありません: {name}")))?;
            check(
                data.len() == len && hash(data) == digest,
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
pub(crate) fn split_set(n: &str) -> Option<(&str, &str)> {
    let s = n.strip_prefix("sets/")?;
    let (id, rest) = s.split_once('/')?;
    valid_id(id).then_some((id, rest))
}
fn name_check(n: &str, level: u32, prefix: &str) -> Result<()> {
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
}
