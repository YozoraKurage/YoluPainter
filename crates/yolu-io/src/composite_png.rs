use crate::package::ParallelDeflate;
use crate::{check_budget, Result};
use flate2::{write::ZlibEncoder, Compression};
use rayon::prelude::*;
use std::io::Write;
use yolu_core::{Channel, Document};

/// Normal の出力が確保してよい作業のバイト数（Unity 版 `NormalMaps.DefaultWorkingBudgetBytes` と同じ）。
const NORMAL_WORKING_BYTES: u64 = 256 * 1024 * 1024;

/// 現在のColor合成をRGBA8 PNGへ書く。上下の向き、行フィルター、チャンク配置は
/// Unity版RgbaPngと同じ。圧縮バイトはdeflate実装の版にも依存し、フィルターした流れが 1 MiB を超える絵は
/// かたまりに分けて並べて圧縮する（`encode_export`）。
pub fn composite_png(doc: &Document) -> Result<Vec<u8>> {
    check_budget(
        doc.width() <= 8192 && doc.height() <= 8192,
        "PNGの寸法の上限は8192です",
    )?;
    let rgba = doc.composite(doc.bounds())?;
    encode_export(&rgba, doc.width(), doc.height())
}
/// 使っている標準チャンネルごとの合成の PNG（`composite/<チャンネル>.png` の中身。番号の順）。Unity 版の `YlpContent.Composites` と
/// 同じ選び方で、どれかの層が有効にしているチャンネルと、Height → Normal が有効で Height を使っているときの Normal。Normal は
/// Unity 向けの出力（OpenGL の向き・不透明・塗っていない所は平ら）。Color は使う層が無くても必ず含める。ユーザーチャンネルの
/// PNG は作らない（Unity 版のインポーターが名前を知らない）。
pub fn composite_pngs(doc: &Document) -> Result<Vec<(Channel, Vec<u8>)>> {
    check_budget(
        doc.width() <= 8192 && doc.height() <= 8192,
        "PNGの寸法の上限は8192です",
    )?;
    let mut out = Vec::new();
    for channel in Channel::ALL {
        let used = channel == Channel::Color
            || doc.layers().iter().any(|l| l.is_channel_enabled(channel))
            || channel == Channel::Normal && doc.derives_normal();
        if !used {
            continue;
        }
        let rgba = if channel == Channel::Normal {
            doc.normal_output(NORMAL_WORKING_BYTES)?
        } else {
            doc.composite_channel(channel, doc.bounds())?
        };
        out.push((channel, encode(&rgba, doc.width(), doc.height())?));
    }
    Ok(out)
}
/// 行のフィルターを選ぶ帯の大きさ（フィルターした行のバイト数の目安）。帯の行はワーカーへ分け、前の帯を圧縮している間に次の帯を選ぶ。
const FILTER_BAND_BYTES: usize = 1 << 20;

/// 文書の中に入れる PNG（.ylp の合成・アセットの画像）: 圧縮は 1 本の流れで、バイトは前の版と同じ。
pub(crate) fn encode(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let zlib = zlib_in_one_stream(rgba, width as usize * 4, height as usize)?;
    Ok(png(width, height, &zlib))
}

/// 書き出す PNG（テクスチャの書き出し・合成の PNG）: フィルターした流れが 1 MiB を超えると、1 MiB ずつのかたまりに分けて並べて圧縮し、
/// 1 本の zlib の流れにつなぐ（`ParallelDeflate::block`。かたまりは前の 32 KiB を辞書に、最後でなければ Sync の flush で終える）。
/// 画素は `encode` と同じで、どの読み手にも普通の PNG。1 MiB 以下の絵は `encode` とバイトまで同じ。大きな絵のバイトはスレッドの数に
/// 依らない（かたまりの境目は流れの中のバイトの位置で決まる）が、`encode` より少し大きい（前のかたまりの外を指せない・塊の終わりの区切り）。
pub(crate) fn encode_export(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let (stride, h) = (width as usize * 4, height as usize);
    let zlib = if (stride + 1) * h <= ParallelDeflate::BLOCK {
        zlib_in_one_stream(rgba, stride, h)?
    } else {
        zlib_in_blocks(h, stride + 1, |first, out| {
            filter_band(rgba, h, stride, first, out)
        })?
    };
    Ok(png(width, height, &zlib))
}

/// rgba の first 行目（上から数えて。rgba は下の行から）からの行を、out（フィルターした行が並ぶ）へ並べてフィルターする。行のフィルターは、
/// その行と 1 つ上の行の生の画素だけで決まるので、行ごとに並べても 1 行ずつ選ぶのと同じバイト。
fn filter_band(rgba: &[u8], h: usize, stride: usize, first: usize, out: &mut [u8]) {
    out.par_chunks_mut(stride + 1).enumerate().for_each_init(
        || vec![0u8; stride + 1],
        |row, (k, best)| filter_row(rgba, h, stride, h - 1 - (first + k), row, best),
    )
}

/// 1 本の zlib の流れ。帯の行のフィルターを並べ、前の帯の圧縮と重ねる（帯の切れ目で区切らないので、圧縮したバイトも 1 行ずつ書くのと同じ）。
fn zlib_in_one_stream(rgba: &[u8], stride: usize, h: usize) -> Result<Vec<u8>> {
    let line = stride + 1;
    let mut compressed = ZlibEncoder::new(Vec::new(), Compression::default());
    let band_rows = (FILTER_BAND_BYTES / line).clamp(1, h.max(1));
    let mut current = vec![0u8; band_rows * line];
    let mut next = vec![0u8; band_rows * line];
    filter_band(rgba, h, stride, 0, &mut current[..band_rows.min(h) * line]);
    let mut first = 0;
    while first < h {
        let rows = band_rows.min(h - first);
        let following = band_rows.min(h - first - rows);
        let (written, ()) = rayon::join(
            || compressed.write_all(&current[..rows * line]),
            || filter_band(rgba, h, stride, first + rows, &mut next[..following * line]),
        );
        written?;
        std::mem::swap(&mut current, &mut next);
        first += rows;
    }
    Ok(compressed.finish()?)
}

/// かたまりに分けて並べて圧縮した zlib の流れ（line バイトの行が h 行（1 行以上）。fill(最初の行, 場所) が行を並べて書く）。行を、かたまりの数が
/// ワーカーの 2 倍（8 以上）になるまでためてから、たまったかたまりを並べて圧縮する（持つのは、そのぶんの行と、前のかたまりの終わりの
/// 32 KiB だけ）。かたまりの境目は流れの中のバイトの位置で決まり、行の途中でもよい。
fn zlib_in_blocks(
    h: usize,
    line: usize,
    mut fill: impl FnMut(usize, &mut [u8]),
) -> std::io::Result<Vec<u8>> {
    const BLOCK: usize = ParallelDeflate::BLOCK;
    const DICT: usize = ParallelDeflate::DICT;
    let group_rows = ((rayon::current_num_threads() * 2).max(8) * BLOCK / line).max(1);
    // 水準 6・32 KiB の窓の zlib の頭（`ZlibEncoder` の既定と同じ）
    let mut zlib = vec![0x78, 0x9c];
    let mut adler = 1u32;
    // buf: [辞書（前のかたまりの終わり。最初は空）][まだ圧縮していない行（かたまりの境目から）]
    let mut buf: Vec<u8> = Vec::new();
    let mut dict = 0;
    let mut row = 0;
    while row < h {
        let rows = group_rows.min(h - row);
        let at = buf.len();
        buf.resize(at + rows * line, 0);
        fill(row, &mut buf[at..]);
        row += rows;
        let end = row == h;
        let pending = buf.len() - dict;
        let blocks = if end {
            pending.div_ceil(BLOCK)
        } else {
            pending / BLOCK
        };
        let packed: Vec<std::io::Result<(Vec<u8>, u32, usize)>> = (0..blocks)
            .into_par_iter()
            .map(|k| {
                let start = dict + k * BLOCK;
                let data = &buf[start..(start + BLOCK).min(buf.len())];
                let before = &buf[start.saturating_sub(DICT)..start];
                let last = end && k + 1 == blocks;
                let bytes =
                    ParallelDeflate::block(data, (!before.is_empty()).then_some(before), last)?;
                Ok((bytes, adler32(data), data.len()))
            })
            .collect();
        for block in packed {
            let (bytes, sum, len) = block?;
            zlib.extend_from_slice(&bytes);
            adler = adler32_combine(adler, sum, len as u64);
        }
        if blocks > 0 && !end {
            let used = dict + blocks * BLOCK;
            buf.drain(..used - DICT);
            dict = DICT;
        }
    }
    zlib.extend(adler.to_be_bytes());
    Ok(zlib)
}

/// zlib の流れの Adler-32（RFC 1950）。
fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    // 和が u32 にあふれない長さ（zlib の NMAX）
    const RUN: usize = 5552;
    let (mut a, mut b) = (1u32, 0u32);
    for run in data.chunks(RUN) {
        for &x in run {
            a += u32::from(x);
            b += a;
        }
        a %= MOD;
        b %= MOD;
    }
    (b << 16) | a
}

/// 続けた 2 つの区間の Adler-32（前の区間の値・後の区間の値と長さから。zlib の `adler32_combine` と同じ式）。
fn adler32_combine(first: u32, second: u32, second_len: u64) -> u32 {
    const MOD: u64 = 65521;
    let rem = second_len % MOD;
    let a1 = u64::from(first & 0xffff);
    let b1 = u64::from(first >> 16);
    let a2 = u64::from(second & 0xffff);
    let b2 = u64::from(second >> 16);
    let a = (a1 + a2 + MOD - 1) % MOD;
    let b = (rem * a1 % MOD + b1 + b2 + MOD - rem) % MOD;
    ((b << 16) | a) as u32
}

/// IHDR（RGBA8）・zlib の流れを 256 KiB ずつに分けた IDAT・IEND の PNG。
fn png(width: u32, height: u32, zlib: &[u8]) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::from(width.to_be_bytes());
    header.extend(height.to_be_bytes());
    header.extend([8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &header);
    for bytes in zlib.chunks(1 << 18) {
        chunk(&mut out, b"IDAT", bytes);
    }
    chunk(&mut out, b"IEND", &[]);
    out
}
/// rgba の y 行目（下から）を、5 つのフィルターの中で差の絶対値の和が一番小さいものでフィルターした行（先頭にフィルターの番号）を best へ。
/// 和が今までの一番を超えたフィルターは途中でやめる。同じ和なら番号の小さいほう。row は作業の場所。
fn filter_row(
    rgba: &[u8],
    height: usize,
    stride: usize,
    y: usize,
    row: &mut [u8],
    best: &mut [u8],
) {
    let mut best_score = u64::MAX;
    for filter in 0..=4 {
        row[0] = filter;
        let mut score = 0;
        for i in 0..stride {
            let x = rgba[y * stride + i];
            let a = if i >= 4 { rgba[y * stride + i - 4] } else { 0 };
            let b = if y + 1 < height {
                rgba[(y + 1) * stride + i]
            } else {
                0
            };
            let c = if i >= 4 && y + 1 < height {
                rgba[(y + 1) * stride + i - 4]
            } else {
                0
            };
            let predicted = match filter {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                _ => paeth(a, b, c),
            };
            let v = x.wrapping_sub(predicted);
            row[i + 1] = v;
            score += u64::from(v).min(256 - u64::from(v));
            if score >= best_score {
                break;
            }
        }
        if score < best_score {
            best_score = score;
            best.copy_from_slice(row);
        }
    }
}
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i32::from(a) + i32::from(b) - i32::from(c);
    let da = (p - i32::from(a)).abs();
    let db = (p - i32::from(b)).abs();
    let dc = (p - i32::from(c)).abs();
    if da <= db && da <= dc {
        a
    } else if db <= dc {
        b
    } else {
        c
    }
}
fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    out.extend(kind);
    out.extend(data);
    let mut crc = crc32fast::Hasher::new();
    crc.update(kind);
    crc.update(data);
    out.extend(crc.finalize().to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1 行ずつフィルターを選んで、1 行ずつ圧縮の流れへ書く形（帯に分けて並べる前の形）。
    fn one_row_at_a_time(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
        let (stride, h) = (width as usize * 4, height as usize);
        let mut compressed = ZlibEncoder::new(Vec::new(), Compression::default());
        let (mut row, mut best) = (vec![0; stride + 1], vec![0; stride + 1]);
        for y in (0..h).rev() {
            filter_row(rgba, h, stride, y, &mut row, &mut best);
            compressed.write_all(&best).unwrap();
        }
        compressed.finish().unwrap()
    }

    /// 雑音・横と縦のグラデーション・一様・透明の帯を混ぜた絵（5 つのフィルターがどれも選ばれる）。
    fn picture(width: u32, height: u32) -> Vec<u8> {
        let mut seed = 0x1234_5678_9abc_def0u64;
        let mut out = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let p = match (y / 40) % 5 {
                    0 => [
                        seed as u8,
                        (seed >> 8) as u8,
                        (seed >> 16) as u8,
                        (seed >> 24) as u8,
                    ],
                    1 => [x as u8, (x / 2) as u8, 200, 255],
                    2 => [(y * 3) as u8, 90, (x ^ y) as u8, 255],
                    3 => [17, 34, 51, 128],
                    _ => [0, 0, 0, 0],
                };
                out.extend(p);
            }
        }
        out
    }

    /// 帯に分けて並べても、1 行ずつの形と圧縮したバイトまで同じ（帯の境目をまたぐ大きさ・1 帯に収まる大きさ・1 行・1 画素、スレッド 1・4）。
    #[test]
    fn banded_parallel_filtering_writes_the_same_bytes() {
        for threads in [1, 4] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            for (w, h) in [(700, 900), (300, 200), (513, 1), (1, 1)] {
                let rgba = picture(w, h);
                let png = pool.install(|| encode(&rgba, w, h)).unwrap();
                let expected = one_row_at_a_time(&rgba, w, h);
                assert!(idat(&png) == expected, "{w}×{h} スレッド {threads}");
            }
        }
    }

    /// IDAT の中身をつないだもの。
    fn idat(png: &[u8]) -> Vec<u8> {
        let mut idat = Vec::new();
        let mut at = 8;
        while at < png.len() {
            let n = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            if &png[at + 4..at + 8] == b"IDAT" {
                idat.extend_from_slice(&png[at + 8..at + 8 + n]);
            }
            at += 12 + n;
        }
        idat
    }

    fn pool(threads: usize) -> rayon::ThreadPool {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
    }

    /// zlib の流れを戻す（読み手は Adler-32 も確かめる）。
    fn inflate(zlib: &[u8]) -> Vec<u8> {
        let mut back = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(zlib), &mut back).unwrap();
        back
    }

    /// png で復号した寸法と画素（上の行から）。
    fn decode_png(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut r = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        let (w, h) = (r.info().width, r.info().height);
        assert_eq!(r.info().color_type, png::ColorType::Rgba);
        let mut out = vec![0; w as usize * h as usize * 4];
        r.next_frame(&mut out).unwrap();
        r.finish().unwrap();
        (w, h, out)
    }

    /// Adler-32 が RFC 1950 の値（Wikipedia の例）で、区間に分けて合わせた値が全体の値と同じ。
    #[test]
    fn adler32_matches_and_combines() {
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        assert_eq!(adler32(b""), 1);
        let data: Vec<u8> = (0..200_000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
            .collect();
        for cut in [0, 1, 5551, 5552, 65521, 65522, 100_000, data.len()] {
            let (a, b) = data.split_at(cut);
            let joined = adler32_combine(adler32(a), adler32(b), b.len() as u64);
            assert_eq!(joined, adler32(&data), "{cut}");
        }
        // 0xff ばかりの長い列（和が法を何度も回る）
        let ff = vec![0xffu8; 3 * 65521 + 17];
        let (a, b) = ff.split_at(70_000);
        assert_eq!(
            adler32_combine(adler32(a), adler32(b), b.len() as u64),
            adler32(&ff)
        );
    }

    /// かたまりの境目（1 行 1 バイトの流れで、ちょうど 1 MiB・1 MiB+1・2 MiB−1・ためる束の境目をまたぐ長さ。行が長い流れで、行がかたまりと
    /// 束の境目をまたぐ）で、元に戻り
    /// （Adler-32 も合う）、スレッドの数（1・4・8）に依らず同じバイト。
    #[test]
    fn block_streams_inflate_back_and_do_not_depend_on_threads() {
        const MIB: usize = 1 << 20;
        // 1 MiB を超える周期の模様と、ところどころの雑音
        let data: Vec<u8> = (0..18 * MIB + 3)
            .map(|i| ((i * 7 / 3) % 251) as u8 ^ if i % 4099 == 0 { 0x5a } else { 0 })
            .collect();
        for (line, rows) in [
            (1, MIB),
            (1, MIB + 1),
            (1, 2 * MIB - 1),
            (1, 9 * MIB + 5),
            (1, 18 * MIB + 3),
            (4081, 257),
            (3000, 6000),
        ] {
            let len = line * rows;
            let packed: Vec<Vec<u8>> = [1, 4, 8]
                .iter()
                .map(|&threads| {
                    pool(threads)
                        .install(|| {
                            zlib_in_blocks(rows, line, |first, out| {
                                out.copy_from_slice(&data[first * line..first * line + out.len()])
                            })
                        })
                        .unwrap()
                })
                .collect();
            assert!(packed.iter().all(|p| *p == packed[0]), "{line}×{rows}");
            assert!(inflate(&packed[0]) == data[..len], "{line}×{rows}");
            // 前のかたまりの終わりを辞書にしているので、模様は縮んだまま
            assert!(
                packed[0].len() < len / 4,
                "{line}×{rows}: {}",
                packed[0].len()
            );
        }
    }

    /// 書き出しの PNG は、png で復号すると元の画素（上下は PNG の向き）。フィルターした流れ（(4·幅+1)·高さ バイト）が 1 MiB 以下の絵は
    /// 文書の中の PNG とバイトまで同じ。1 MiB を超える絵は、境目（1 MiB−1・1 MiB+1 ちょうどの縦長と横長・1 MiB+241）・行がかたまりの
    /// 境目をまたぐ・束をまたぐ絵で、スレッドの数（1・4・8）に依らず同じバイト。1 行が奇数バイトなので、ちょうど 1 MiB の絵は作れない。
    #[test]
    fn exported_png_decodes_to_the_same_pixels() {
        for (w, h) in [
            (256, 1023),  // 1 MiB−1（かたまり 1 つ）
            (4, 61_681),  // 1 MiB+1（縦長）
            (15_420, 17), // 1 MiB+1（横長）
            (1020, 257),  // 1 MiB+241
            (700, 900),
            (1500, 1500),
            (1, 1),
        ] {
            let rgba = picture(w, h);
            let one = encode(&rgba, w, h).unwrap();
            let outs: Vec<Vec<u8>> = [1, 4, 8]
                .iter()
                .map(|&threads| {
                    pool(threads)
                        .install(|| encode_export(&rgba, w, h))
                        .unwrap()
                })
                .collect();
            assert!(outs.iter().all(|o| *o == outs[0]), "{w}×{h}");
            let small = (w as usize * 4 + 1) * h as usize <= 1 << 20;
            assert_eq!(outs[0] == one, small, "{w}×{h}");
            let stride = w as usize * 4;
            let top_down: Vec<u8> = rgba.chunks(stride).rev().flatten().copied().collect();
            for png in [&one, &outs[0]] {
                let (pw, ph, pixels) = decode_png(png);
                assert_eq!((pw, ph), (w, h));
                assert!(pixels == top_down, "{w}×{h}");
            }
        }
    }
}
