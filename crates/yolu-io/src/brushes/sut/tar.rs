//! 素材（`MaterialFile.FileData`）の中の無圧縮 tar の最小の読み手。
//!
//! 信頼できない入力なので、ファイルに触れず（メモリ上の切り出しだけを返す）、宣言された大きさはデータの残りに入ると確かめてから
//! 使い、項目の数に上限を置き、壊れていればそこまでの項目で止める（パニックしない）。pax・GNU の拡張ヘッダーは、長い名前
//! （`L`）だけを使い、残りは読み飛ばす。

const BLOCK: usize = 512;

/// 1 つの tar から数える項目の上限。
pub(super) const MAX_ENTRIES: usize = 4096;

/// 長い名前の上限（バイト）。
const MAX_NAME: usize = 1024;

pub(super) struct Entry<'a> {
    pub name: String,
    pub data: &'a [u8],
}

/// 8 進数の欄（NUL か空白で終わる）。読めなければ None。
fn octal(field: &[u8]) -> Option<u64> {
    let text: Vec<u8> = field
        .iter()
        .copied()
        .skip_while(|&b| b == b' ')
        .take_while(|&b| (b'0'..=b'7').contains(&b))
        .collect();
    if text.is_empty() {
        // 全部が空白・NUL の欄は 0
        return field.iter().all(|&b| b == 0 || b == b' ').then_some(0);
    }
    if text.len() > 22 {
        return None;
    }
    let mut value: u64 = 0;
    for b in text {
        value = value.checked_mul(8)?.checked_add((b - b'0') as u64)?;
    }
    Some(value)
}

fn checksum_ok(block: &[u8]) -> bool {
    let Some(stored) = octal(&block[148..156]) else {
        return false;
    };
    let sum: u64 = block
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            if (148..156).contains(&i) {
                b' ' as u64
            } else {
                b as u64
            }
        })
        .sum();
    sum == stored
}

fn text(field: &[u8]) -> String {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

/// 先頭が tar のヘッダーか（検査の合計が合う）。
pub(super) fn looks_like_tar(data: &[u8]) -> bool {
    data.len() >= BLOCK && data[..BLOCK].iter().any(|&b| b != 0) && checksum_ok(&data[..BLOCK])
}

/// 普通のファイルの項目を、壊れた所までの分だけ返す。
pub(super) fn entries(data: &[u8]) -> Vec<Entry<'_>> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    let mut long_name: Option<String> = None;
    let mut headers = 0usize;
    while headers < MAX_ENTRIES {
        let Some(block) = data.get(pos..pos + BLOCK) else {
            break;
        };
        if block.iter().all(|&b| b == 0) || !checksum_ok(block) {
            break;
        }
        headers += 1;
        let Some(size) = octal(&block[124..136]) else {
            break;
        };
        let start = pos + BLOCK;
        let Some(end) = usize::try_from(size)
            .ok()
            .and_then(|size| start.checked_add(size))
            .filter(|&end| end <= data.len())
        else {
            break;
        };
        let body = &data[start..end];
        match block[156] {
            0 | b'0' => {
                let mut name = text(&block[0..100]);
                if &block[257..262] == b"ustar" {
                    let prefix = text(&block[345..500]);
                    if !prefix.is_empty() {
                        name = format!("{prefix}/{name}");
                    }
                }
                out.push(Entry {
                    name: long_name.take().unwrap_or(name),
                    data: body,
                });
            }
            b'L' => {
                long_name = Some(text(&body[..body.len().min(MAX_NAME)]));
            }
            _ => long_name = None,
        }
        let padded = (size as usize).div_ceil(BLOCK).checked_mul(BLOCK);
        match padded.and_then(|p| start.checked_add(p)) {
            Some(next) => pos = next,
            None => break,
        }
    }
    out
}

#[cfg(test)]
pub(super) mod build {
    //! 試験用の tar の組み立て（ustar）。

    use super::BLOCK;

    pub fn header(name: &str, size: usize, typeflag: u8) -> [u8; BLOCK] {
        let mut h = [0u8; BLOCK];
        let n = name.as_bytes();
        h[..n.len().min(100)].copy_from_slice(&n[..n.len().min(100)]);
        h[100..107].copy_from_slice(b"0000644");
        h[108..115].copy_from_slice(b"0000000");
        h[116..123].copy_from_slice(b"0000000");
        h[124..135].copy_from_slice(format!("{size:011o}").as_bytes());
        h[136..147].copy_from_slice(b"00000000000");
        h[156] = typeflag;
        h[257..262].copy_from_slice(b"ustar");
        h[263..265].copy_from_slice(b"00");
        for b in &mut h[148..156] {
            *b = b' ';
        }
        let sum: u32 = h.iter().map(|&b| b as u32).sum();
        h[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        h[155] = b' ';
        h
    }

    pub fn tar(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, data) in files {
            out.extend_from_slice(&header(name, data.len(), b'0'));
            out.extend_from_slice(data);
            out.resize(out.len().div_ceil(BLOCK) * BLOCK, 0);
        }
        out.extend_from_slice(&[0u8; BLOCK * 2]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;

    #[test]
    fn reads_regular_files_and_stops_at_the_end_blocks() {
        let data = tar(&[("thumbnail/thumbnail.png", b"abc"), ("b.txt", &[7u8; 600])]);
        assert!(looks_like_tar(&data));
        let e = entries(&data);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].name, "thumbnail/thumbnail.png");
        assert_eq!(e[0].data, b"abc");
        assert_eq!(e[1].data.len(), 600);
    }

    #[test]
    fn a_size_that_does_not_fit_or_a_bad_checksum_stops_the_reading_without_panicking() {
        let mut data = tar(&[("a", b"xyz"), ("b", b"123")]);
        // 2 つ目の大きさを巨大にする（検査の合計が合わなくなるので、2 つ目で止まる）
        data[512 + 512 + 124] = b'7';
        assert_eq!(entries(&data).len(), 1);
        // 1 つ目の大きさが残りより大きい（合計は直す）
        let mut h = header("a", 3, b'0');
        h[124..135].copy_from_slice(b"77777777777");
        for b in &mut h[148..156] {
            *b = b' ';
        }
        let sum: u32 = h.iter().map(|&b| b as u32).sum();
        h[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        let mut broken = h.to_vec();
        broken.extend_from_slice(b"xyz");
        assert!(entries(&broken).is_empty());
        for len in 0..data.len() {
            let _ = entries(&data[..len]);
        }
        assert!(entries(&[]).is_empty());
        assert!(!looks_like_tar(&[0u8; 512]));
    }

    #[test]
    fn a_long_name_entry_names_the_next_file() {
        let name = "x".repeat(150);
        let mut data = Vec::new();
        data.extend_from_slice(&header("././@LongLink", name.len() + 1, b'L'));
        let mut body = name.as_bytes().to_vec();
        body.push(0);
        data.extend_from_slice(&body);
        data.resize(data.len().div_ceil(512) * 512, 0);
        data.extend_from_slice(&header("short", 2, b'0'));
        data.extend_from_slice(b"ok");
        data.resize(data.len().div_ceil(512) * 512, 0);
        data.extend_from_slice(&[0u8; 1024]);
        let e = entries(&data);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].name, name);
        assert_eq!(e[0].data, b"ok");
    }
}
