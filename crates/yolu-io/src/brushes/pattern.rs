//! Photoshop のパターン（模様）を筆先の質感として読む: ABR の `patt` 節の記録（それぞれ 32 bit の長さで枠取りされ、4 バイトに
//! 揃えて詰める。PSD の `Patt` ブロックと同じ）と、単体の `.pat`（`8BPT`・版 1・個数・枠なしの記録）。
//!
//! 並びは Photoshop File Formats Specification の「Patterns」「Virtual Memory Array List」を psd-tools の実装に沿って読んだもの。
//! グレーの 8 bit はバイトのまま、RGB とインデックスはグレーにし（注記する）、16 bit は上位バイト（注記する）。
//! ほかのモード・ZIP・筆先より大きい模様は使えない理由つきで断る。1 つの模様の画素は予算（`MAX_DECODED_BYTES`）から引く。

use std::sync::Arc;

use yolu_core::{Brush, BrushTip, PaperTexture};

use super::error::{
    BrushImportError, Counted, Fault, PatternMode, PatternRefusal, Result, SizedItem,
    SkippedPattern,
};
use super::notes::{PatternNote, Source, Unrepresented};
use super::reader::{ascii, decode_packbits_row, Budget, Reader};
use super::{short_text, ImportedBrush, ImportedSet};

/// 筆先の質感にできる模様。濃淡 0〜255（白が一番塗れる。Photoshop の「高い所」）で、行は下から。
#[derive(Debug)]
pub(crate) struct Pattern {
    pub name: String,
    pub id: String,
    pub texture: Arc<BrushTip>,
    pub notes: Vec<PatternNote>,
}

enum Outcome {
    Usable(Pattern),
    /// 読めたが使えない（記録の最後まで読み進めてある）。
    Refused {
        name: String,
        reason: PatternRefusal,
    },
}

const MAX_PATTERNS: u32 = 10_000;

/// `.pat` の模様を全部。構造が壊れていれば断り、使えない模様は理由つきで `skipped` に入れる。
/// 枠が無いので、1 つ構造が壊れると後ろの位置も分からない（読めたところまでは返さず、断る）。
pub(crate) fn read_pat(
    data: &[u8],
    budget: &mut Budget,
) -> Result<(Vec<Pattern>, Vec<SkippedPattern>)> {
    let mut r = Reader::new(data);
    if r.remaining() < 4 || r.ascii(4)? != "8BPT" {
        return Err(Fault::NotPat);
    }
    let version = r.u16()?;
    if version != 1 {
        return Err(Fault::PatVersion(version));
    }
    let count = r.u32()?;
    if count > MAX_PATTERNS {
        return Err(Fault::PatCount(count));
    }
    let (mut patterns, mut skipped) = (Vec::new(), Vec::new());
    for _ in 0..count {
        match read_pattern(&mut r, budget)? {
            Outcome::Usable(p) => patterns.push(p),
            Outcome::Refused { name, reason } => skipped.push(SkippedPattern { name, reason }),
        }
    }
    Ok((patterns, skipped))
}

/// `.pat` を、模様ごとのブラシにする。丸いブラシで、模様が深さ 1 の紙の質感（ほかのブラシの質感としても選べる）。
/// 使えなかった模様はファイル全体の注記（`ImportedSet::notes`）に 1 回ずつ入れる（ブラシごとには複製しない）。
/// 使える模様が 1 つも無ければ断る。
pub(crate) fn read_pat_brushes(
    data: &[u8],
    budget: &mut Budget,
) -> std::result::Result<ImportedSet, BrushImportError> {
    let (patterns, skipped) = read_pat(data, budget)?;
    if patterns.is_empty() {
        return Err(BrushImportError::NoUsablePattern(skipped));
    }
    let mut set = ImportedSet {
        notes: skipped
            .into_iter()
            .map(|s| Unrepresented::PatternSkipped {
                name: s.name,
                reason: s.reason,
            })
            .collect(),
        ..ImportedSet::default()
    };
    for p in patterns {
        let notes: Vec<Unrepresented> = p.notes.into_iter().map(Unrepresented::Pattern).collect();
        let brush = Brush {
            texture: Some(PaperTexture::new(p.texture, 1.0)),
            ..Brush::default()
        };
        set.brushes.push(ImportedBrush::new(
            &p.name,
            Source::PhotoshopPattern,
            brush,
            notes,
        )?);
    }
    Ok(set)
}

/// ABR の `patt` 節の記録。
pub(crate) fn read_framed(
    r: &mut Reader,
    budget: &mut Budget,
    patterns: &mut Vec<Pattern>,
    skipped: &mut Vec<SkippedPattern>,
) -> Result<()> {
    let mut records = 0u32;
    while r.remaining() >= 4 {
        // 記録の数にも上限（`.pat` の個数と同じ）。使えない模様も 1 件ずつ失敗の一覧に残るので、数えずには増やさない
        records += 1;
        if records > MAX_PATTERNS {
            return Err(Fault::PatCount(records));
        }
        let length = r.counted(Counted::PatternLength)?;
        if length == 0 {
            return Err(Fault::PatternEmptyRecord);
        }
        let mut record = Reader::new(r.bytes(length)?);
        match read_pattern(&mut record, budget)? {
            Outcome::Usable(p) => patterns.push(p),
            Outcome::Refused { name, reason } => skipped.push(SkippedPattern { name, reason }),
        }
        let pad = (4 - length % 4) % 4;
        if pad > 0 && r.remaining() >= pad {
            r.skip(pad)?;
        }
    }
    Ok(())
}

/// 模様 1 つ。構造の破れは断り、読めたが使えない記録は理由つきで返す。
fn read_pattern(r: &mut Reader, budget: &mut Budget) -> Result<Outcome> {
    let version = r.i32()?;
    if version != 1 {
        return Err(Fault::PatternVersion(version));
    }
    let mode = r.i32()?;
    let height = r.i16()? as i64;
    let width = r.i16()? as i64;
    let name = super::descriptor::unicode_string(r)?;
    let id_length = r.u8()? as usize;
    let id = ascii(r.bytes(id_length)?);
    let palette = if mode == 2 {
        let p = r.bytes(256 * 3)?;
        r.skip(4)?;
        Some(p)
    } else {
        None
    };
    let list_version = r.i32()?;
    if list_version != 3 {
        return Err(Fault::PatternDataVersion(list_version));
    }
    let list_length = r.counted(Counted::PatternDataLength)?;
    let mut list = Reader::new(r.bytes(list_length)?);
    let (top, left, bottom, right) = (
        list.i32()? as i64,
        list.i32()? as i64,
        list.i32()? as i64,
        list.i32()? as i64,
    );
    let channel_count = list.i32()?;
    if !(0..=56).contains(&channel_count) {
        return Err(Fault::PatternChannelCount(channel_count));
    }
    let needed: usize = match mode {
        1 | 2 => 1,
        3 => 3,
        _ => 0,
    };
    let (w, h) = (right - left, bottom - top);
    let cleaned = short_text(&name, 128);
    let display_name = if cleaned.is_empty() {
        "Pattern".to_string()
    } else {
        cleaned.clone()
    };
    let refuse = |reason: PatternRefusal| {
        Ok(Outcome::Refused {
            name: cleaned.clone(),
            reason,
        })
    };
    let mut channels: Vec<Vec<u8>> = Vec::new();
    let (mut extra, mut depth_seen) = (0usize, 8i16);
    for c in 0..(channel_count as usize + 2) {
        let written = list.u32()?;
        if written == 0 {
            continue;
        }
        let length = list.counted(Counted::PatternChannelLength)?;
        if length == 0 {
            continue;
        }
        if length < 23 {
            return Err(Fault::PatternChannelLengthTooShort(length as u64));
        }
        let mut body = Reader::new(list.bytes(length)?);
        if needed == 0 || c >= needed {
            if needed > 0 {
                extra += 1;
            }
            continue;
        }
        body.i32()?;
        let (ct, cl, cb, cr) = (
            body.i32()? as i64,
            body.i32()? as i64,
            body.i32()? as i64,
            body.i32()? as i64,
        );
        let depth = body.i16()?;
        let compression = body.u8()?;
        if cr - cl != w || cb - ct != h {
            return refuse(PatternRefusal::ChannelsDiffer);
        }
        let side = BrushTip::MAX_SIZE as i64;
        if w < 1 || h < 1 || w > side || h > side {
            return refuse(PatternRefusal::TooLarge {
                width: w,
                height: h,
            });
        }
        if depth != 8 && depth != 16 {
            return refuse(PatternRefusal::Depth(depth));
        }
        if compression > 1 {
            return refuse(PatternRefusal::Zip);
        }
        depth_seen = depth;
        let (wu, hu) = (w as usize, h as usize);
        let bytes_per_pixel = depth as usize / 8;
        let row_bytes = wu * bytes_per_pixel;
        budget.take(wu as u64 * hu as u64)?;
        let mut plane = vec![0u8; wu * hu];
        let mut rle_lengths: Option<Vec<usize>> = None;
        for y in 0..hu {
            let decoded;
            let row: &[u8] = if compression == 1 {
                if rle_lengths.is_none() {
                    let mut lengths = Vec::new();
                    for _ in 0..hu {
                        lengths.push(body.u16()? as usize);
                    }
                    rle_lengths = Some(lengths);
                }
                let len = rle_lengths.as_ref().map(|l| l[y]).unwrap_or(0);
                decoded = decode_packbits_row(body.bytes(len)?, row_bytes)?;
                &decoded
            } else {
                body.bytes(row_bytes)?
            };
            for x in 0..wu {
                plane[(hu - 1 - y) * wu + x] = row[x * bytes_per_pixel]; // 上位バイト。上から下の行を左下原点へ
            }
        }
        channels.push(plane);
    }
    if needed == 0 {
        return refuse(PatternRefusal::Mode(PatternMode::from_code(mode)));
    }
    if channels.len() < needed {
        return refuse(PatternRefusal::ChannelsMissing);
    }
    let mut notes = Vec::new();
    if height != h || width != w {
        notes.push(PatternNote::SizeDiffers {
            stated: (width, height),
            data: (w, h),
        });
    }
    if depth_seen == 16 {
        notes.push(PatternNote::Reduced16Bit);
    }
    if extra > 0 {
        notes.push(PatternNote::ExtraChannelsIgnored(extra));
    }
    let grey = if mode == 1 {
        channels.swap_remove(0)
    } else {
        let mut grey = vec![0u8; (w * h) as usize];
        for (i, g) in grey.iter_mut().enumerate() {
            let (rr, gg, bb) = match palette {
                Some(p) if mode == 2 => {
                    let k = channels[0][i] as usize * 3;
                    (p[k] as u32, p[k + 1] as u32, p[k + 2] as u32)
                }
                _ => (
                    channels[0][i] as u32,
                    channels[1][i] as u32,
                    channels[2][i] as u32,
                ),
            };
            *g = ((rr * 299 + gg * 587 + bb * 114 + 500) / 1000) as u8;
        }
        notes.push(PatternNote::GreyConversion { indexed: mode == 2 });
        grey
    };
    let texture = BrushTip::new(&display_name, w as u32, h as u32, grey).map_err(|_| {
        Fault::SizeOutOfRange {
            what: SizedItem::Pattern,
            width: w,
            height: h,
        }
    })?;
    Ok(Outcome::Usable(Pattern {
        name: display_name,
        id,
        texture: Arc::new(texture),
        notes,
    }))
}
