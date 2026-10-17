//! はみ出し防止（`.sut` の「はみ出し防止」）の取り込み: 参照レイヤーの線をはみ出さない・色の誤差・領域拡縮（量・方法）。
//!
//! 列: 旗は `BrushUseReferLayer`、色の誤差は `FillColorMargin`（0〜100）、領域拡縮の旗は `FillUseExpand`、量は `FillExpandLength`
//! （単位の列 `FillExpandLengthUnit` が 0 のとき画素）、方法は `FillExpandType`（1 丸く拡張・2 最も濃いピクセルまで拡張。1 項目ずつ変えた
//! 書き出しの比べで決まった。0 は四角く拡張と見ているが確かめていない）。ベクターの中心線（`FillReferVectorCenter`）は読まない。
//!
//! 旗が切なら何も写さない。読めない値（方法の知らない値・確かめていない 0、単位が画素でない、範囲の外）は、近い値で写して知らせる。

use yolu_core::{Brush, ExpandMode, ReferenceLines, MAX_REFERENCE_EXPAND, MAX_REFERENCE_MARGIN};

use super::super::notes::{SutMapped, SutNote, Unrepresented};
use super::db::Row;

const FLAG: &[&str] = &["brushusereferlayer"];
const MARGIN: &[&str] = &["fillcolormargin"];
const EXPAND: &[&str] = &["filluseexpand"];
const LENGTH: &[&str] = &["fillexpandlength"];
const LENGTH_UNIT: &[&str] = &["fillexpandlengthunit"];
const TYPE: &[&str] = &["fillexpandtype"];

/// 読む列の名前の全部。
pub(super) const KNOWN: &[&[&str]] = &[FLAG, MARGIN, EXPAND, LENGTH, LENGTH_UNIT, TYPE];

/// はみ出し防止を写す。
pub(super) fn reference_lines(
    row: &Row,
    brush: &mut Brush,
    notes: &mut Vec<Unrepresented>,
    mapped: &mut Vec<SutMapped>,
) {
    if !row.on(FLAG) {
        return;
    }
    let d = ReferenceLines::default();
    let mut out_of_range = false;
    let margin = match row.first_number(MARGIN) {
        Some(v) if v.is_finite() => {
            out_of_range |= !(0.0..=MAX_REFERENCE_MARGIN).contains(&v);
            v.clamp(0.0, MAX_REFERENCE_MARGIN)
        }
        Some(_) => {
            out_of_range = true;
            d.margin
        }
        None => d.margin,
    };
    let expand = row.on(EXPAND);
    let amount = match row.first_number(LENGTH) {
        Some(v) if v.is_finite() => {
            out_of_range |= expand && !(-MAX_REFERENCE_EXPAND..=MAX_REFERENCE_EXPAND).contains(&v);
            v.clamp(-MAX_REFERENCE_EXPAND, MAX_REFERENCE_EXPAND)
        }
        Some(_) => {
            out_of_range |= expand;
            d.amount
        }
        None => d.amount,
    };
    let kind = row.first_number(TYPE);
    let mode = match kind {
        Some(1.0) => ExpandMode::Round,
        Some(2.0) => ExpandMode::Darkest,
        Some(0.0) => ExpandMode::Square,
        _ => d.mode,
    };
    if expand {
        // 方法: 確かめたのは 1 と 2 だけ（0 は四角くと見て写す）
        if let Some(v) = kind.filter(|&v| v != 1.0 && v != 2.0) {
            notes.push(Unrepresented::ClipStudio(SutNote::ExpandMode(v)));
        }
        if let Some(u) = row.first_number(LENGTH_UNIT).filter(|&u| u != 0.0) {
            notes.push(Unrepresented::ClipStudio(SutNote::ExpandUnit(u)));
        }
    }
    if out_of_range {
        notes.push(Unrepresented::ClipStudio(SutNote::ReferenceRange));
    }
    brush.reference_lines = ReferenceLines {
        enabled: true,
        margin,
        expand,
        amount,
        mode,
    };
    mapped.push(SutMapped::ReferenceLines);
}
