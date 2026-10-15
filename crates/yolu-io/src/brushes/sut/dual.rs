//! デュアルブラシ（`.sut` の「デュアルブラシ」）の取り込み。
//!
//! 列と単位: 旗は `UseDualBrush`。2 つ目のサイズは `DualSize`（文書の画素の直径。単位の列 `DualSizeUnit` は 0 が画素）、
//! メインのブラシサイズに連動は `SyncDualBrushSize`、ブラシ濃度は `DualFlow`（0〜100）、硬さは `DualHardness`、間隔は
//! `DualAutoIntervalType`・`DualInterval`（主の間隔と同じ表）、厚さは `DualThickness`、厚さの適用方向は `DualVerticalThicknes`
//! （0 水平・1 垂直）、向きは `DualRotation`（度）・`DualRotationEffector`（主と同じ旗: 下位 0x03 は常に立つ・0x10 ペンの向き・
//! 0x20 ペン軸の回転・0x40 線の方向・0x80 ランダム）・`DualRotationRandomScale`（0〜100）、合成モードは `DualBrushCompositeMode`
//! （0 通常・1 乗算・2 加算(発光)・3 減算・4 比較(暗)・5 比較(明)・6 スクリーン・7 オーバーレイ・8 覆い焼きカラー・9 焼き込みカラー・
//! 10 焼き込み(リニア)・11 ハードミックス・12 高さ(リニア)）、アンチエイリアスは `DualAntiAlias`（0〜3。ほかの値は知らせる）、先端同士の比較暗は `DualBlendPatternByDarken`・`DualBlendPatternByDarken2`（写さずに知らせる）、先端形状は
//! `DualUsePatternImage`・`DualPatternImageArray`。影響元は `DualSizeEffector`・`DualFlowEffector`・`DualIntervalEffector`・
//! `DualThicknessEffector`（ほかの影響元と同じ形の BLOB）。
//!
//! 2 つ目のサイズの列が無い行と、筆先の素材を決められない行は、デュアルブラシごと写さない（[`SutNote::DualBrush`]）。
//! 散布の旧形（`DualBrush::scatter`・`count`）は使わない。RGB 値の適用・連続吹き付け・リボン・複数の筆先・繰り返し方法と反転・
//! 紙質・散布効果・先端同士の比較暗は写さずに知らせる（[`SutDualNote`]）。

use std::sync::Arc;

use yolu_core::{AntiAlias, Brush, BrushTip, DualBrush, DualBrushMode};

use super::super::notes::{SutDualNote, SutMapped, SutNote, SutTarget, Unrepresented};
use super::blob::{parse_refs, Refs};
use super::db::Row;
use super::material::Kind;
use super::{
    by_kind_picks, effector, interval_spacing, percent, resolve, Budget, Fault, Library,
    DUAL_PATTERN_ARRAY, DUAL_TEXTURE_IMAGE, DUAL_USE_PATTERN, PATTERN_ARRAY, USE_PATTERN,
};

const SIZE: &[&str] = &["dualsize"];
const SIZE_UNIT: &[&str] = &["dualsizeunit"];
const LINK_SIZE: &[&str] = &["syncdualbrushsize"];
const FLOW: &[&str] = &["dualflow"];
const HARDNESS: &[&str] = &["dualhardness"];
const INTERVAL_TYPE: &[&str] = &["dualautointervaltype"];
const INTERVAL: &[&str] = &["dualinterval"];
const THICKNESS: &[&str] = &["dualthickness"];
const THICKNESS_DIRECTION: &[&str] = &["dualverticalthicknes", "dualverticalthickness"];
const ROTATION: &[&str] = &["dualrotation"];
const ROTATION_EFFECTOR: &[&str] = &["dualrotationeffector"];
const ROTATION_RANDOM: &[&str] = &["dualrotationrandomscale"];
const MODE: &[&str] = &["dualbrushcompositemode"];
const ANTI_ALIAS: &[&str] = &["dualantialias"];
const SIZE_EFFECTOR: &[&str] = &["dualsizeeffector"];
const FLOW_EFFECTOR: &[&str] = &["dualfloweffector"];
const INTERVAL_EFFECTOR: &[&str] = &["dualintervaleffector"];
const THICKNESS_EFFECTOR: &[&str] = &["dualthicknesseffector"];
// 写さずに知らせる設定の旗・値
const RGB: &[&str] = &["changergbbydual"];
const CONTINUOUS: &[&str] = &["dualcontinuousplot"];
const RIBBON: &[&str] = &["dualribbon"];
const ORDER: &[&str] = &["dualpatternordertype"];
const REVERSE_H: &[&str] = &["dualpatternreversehorizontal"];
const REVERSE_V: &[&str] = &["dualpatternreversevertical"];
const SPRAY: &[&str] = &["dualusespray"];
const FIXED_POINT: &[&str] = &["dualsprayusefixedpoint"];
const DARKEN: &[&str] = &["dualblendpatternbydarken"];
const DARKEN_2: &[&str] = &["dualblendpatternbydarken2"];

/// 読む列の名前の全部（旗と素材の参照の列は `sut.rs` の `KNOWN` にある）。
pub(super) const KNOWN: &[&[&str]] = &[
    SIZE,
    SIZE_UNIT,
    LINK_SIZE,
    FLOW,
    HARDNESS,
    INTERVAL_TYPE,
    INTERVAL,
    THICKNESS,
    THICKNESS_DIRECTION,
    ROTATION,
    ROTATION_EFFECTOR,
    ROTATION_RANDOM,
    MODE,
    ANTI_ALIAS,
    SIZE_EFFECTOR,
    FLOW_EFFECTOR,
    INTERVAL_EFFECTOR,
    THICKNESS_EFFECTOR,
    RGB,
    CONTINUOUS,
    RIBBON,
    ORDER,
    REVERSE_H,
    REVERSE_V,
    SPRAY,
    FIXED_POINT,
    DARKEN,
    DARKEN_2,
];

/// `DualBrushCompositeMode` の値の順のモード。
const MODES: [DualBrushMode; 13] = [
    DualBrushMode::Normal,
    DualBrushMode::Multiply,
    DualBrushMode::Add,
    DualBrushMode::Subtract,
    DualBrushMode::Darken,
    DualBrushMode::Lighten,
    DualBrushMode::Screen,
    DualBrushMode::Overlay,
    DualBrushMode::ColorDodge,
    DualBrushMode::ColorBurn,
    DualBrushMode::LinearBurn,
    DualBrushMode::HardMix,
    DualBrushMode::LinearHeight,
];

/// 2 つ目の筆先の決まり方。
enum DualTip {
    /// 円形（素材を使わない）。
    Round,
    /// 素材の画像（並びで当てたか・素材が複数か）。
    Image(Arc<BrushTip>, bool, bool),
    /// 素材を決められない・取り出せない。
    Missing,
}

/// デュアルブラシを写す。写せたら `SutMapped::DualBrush` と、近い動き・写せない設定の知らせ（[`SutDualNote`]）を積み、true を返す。
/// 旗が切か、写せなければ何もせず false（旗の知らせは `unrepresented_flags` が出す）。
pub(super) fn dual(
    row: &Row,
    name: &str,
    library: &mut Library<'_>,
    budget: &mut Budget,
    brush: &mut Brush,
    notes: &mut Vec<Unrepresented>,
    mapped: &mut Vec<SutMapped>,
) -> Result<bool, Fault> {
    if !row.on(super::DUAL) {
        return Ok(false);
    }
    let Some(size) = row.first_number(SIZE).filter(|v| v.is_finite() && *v > 0.0) else {
        return Ok(false);
    };
    let (tip, several) = match dual_tip(row, name, library, budget, notes)? {
        DualTip::Round => (None, false),
        DualTip::Image(tip, guessed, several) => {
            if guessed {
                notes.push(Unrepresented::ClipStudio(SutNote::TipGuessed));
            }
            (Some(tip), several)
        }
        DualTip::Missing => return Ok(false),
    };
    let mut approx = |note| notes.push(Unrepresented::ClipStudio(SutNote::DualApprox(note)));
    if several {
        approx(SutDualNote::Tips);
    }
    let mut d = DualBrush {
        radius: (size / 2.0).clamp(0.5, 65536.0),
        tip,
        link_size: row.on(LINK_SIZE),
        ..DualBrush::default()
    };
    if row.on(SIZE_UNIT) {
        approx(SutDualNote::SizeUnit);
    }
    if let Some(v) = percent(row, FLOW) {
        d.flow = v.clamp(0.0, 1.0);
    }
    if let Some(v) = percent(row, HARDNESS) {
        d.hardness = v.clamp(0.0, 1.0);
    }
    match interval_spacing(row, INTERVAL_TYPE, INTERVAL) {
        Ok(Some(v)) => d.spacing = v.clamp(0.01, 4.0),
        Ok(None) => {}
        Err(other) => approx(SutDualNote::IntervalType(other)),
    }
    if let Some(v) = percent(row, THICKNESS) {
        d.roundness = v.clamp(0.0, 1.0).max(0.01);
    }
    d.mode = mode(row, &mut approx);
    d.anti_alias = anti_alias(row, brush.base.anti_alias, &mut approx);
    direction(row, &mut d, &mut approx);
    brush.dual = Some(d);
    for (target, columns) in [
        (SutTarget::DualSize, SIZE_EFFECTOR),
        (SutTarget::DualFlow, FLOW_EFFECTOR),
        (SutTarget::DualSpacing, INTERVAL_EFFECTOR),
        (SutTarget::DualThickness, THICKNESS_EFFECTOR),
    ] {
        effector(brush, row, target, columns, notes, mapped);
    }
    thickness_direction(row, brush, notes);
    unmapped(row, notes);
    mapped.push(SutMapped::DualBrush);
    Ok(true)
}

/// 合成モード（列が無ければ乗算。知らない値は乗算にして知らせる。高さ(リニア)は近い式なので知らせる）。
fn mode(row: &Row, approx: &mut impl FnMut(SutDualNote)) -> DualBrushMode {
    let Some(v) = row.first_number(MODE) else {
        return DualBrushMode::Multiply;
    };
    let found = (v.fract() == 0.0 && v >= 0.0)
        .then(|| MODES.get(v as usize).copied())
        .flatten();
    match found {
        Some(DualBrushMode::LinearHeight) => {
            approx(SutDualNote::LinearHeight);
            DualBrushMode::LinearHeight
        }
        Some(m) => m,
        None => {
            approx(SutDualNote::Mode(v));
            DualBrushMode::Multiply
        }
    }
}

/// 2 つ目のアンチエイリアス（0〜3 を なし・弱・中・強へ）。列が無ければ主に従う。主と同じ段も主に従う既定のままにする。
/// 値があるのに 0〜3 の整数でなければ、主に従ったまま知らせる。
fn anti_alias(
    row: &Row,
    main: AntiAlias,
    approx: &mut impl FnMut(SutDualNote),
) -> Option<AntiAlias> {
    let v = row.first_number(ANTI_ALIAS)?;
    let level = (v.fract() == 0.0 && (0.0..=3.0).contains(&v))
        .then(|| AntiAlias::from_index(v as u8))
        .flatten();
    if level.is_none() {
        approx(SutDualNote::AntiAlias(v));
    }
    level.filter(|l| *l != main)
}

/// 向き: 角度（度）と、向きの影響元の旗（主の [`super::tip_direction`] と同じ旗と規則）。知らない旗・強さの読めないランダムは知らせる。
fn direction(row: &Row, d: &mut DualBrush, approx: &mut impl FnMut(SutDualNote)) {
    if let Some(angle) = row
        .first_number(ROTATION)
        .filter(|a| a.is_finite() && *a != 0.0)
    {
        d.angle = (angle + 180.0).rem_euclid(360.0) - 180.0;
    }
    let flags = row
        .first_number(ROTATION_EFFECTOR)
        .filter(|v| v.is_finite())
        .map_or(0, |v| v as i64);
    let mut unmapped = flags & !(0x03 | 0x10 | 0x20 | 0x40 | 0x80) != 0;
    d.angle_tilt = flags & 0x10 != 0;
    d.angle_rotation = flags & 0x20 != 0;
    d.angle_line = flags & 0x40 != 0;
    if flags & 0x80 != 0 {
        match percent(row, ROTATION_RANDOM).map(|r| r.clamp(0.0, 1.0)) {
            Some(r) => d.angle_random = r,
            None => unmapped = true,
        }
    }
    if unmapped {
        approx(SutDualNote::Direction);
    }
}

/// 厚さの適用方向（0 水平・1 垂直）: 主の [`super::thickness_direction`] と同じく、水平は丸い筆先の角度に 90 度を足す。画像の筆先は
/// 写さずに知らせる。潰していない（真円率 1 で厚さの影響元も無い）ときは形に出ないので何もしない。
fn thickness_direction(row: &Row, brush: &mut Brush, notes: &mut Vec<Unrepresented>) {
    let inf_on = brush.influences.dual_thickness.is_on();
    let Some(d) = brush.dual.as_mut() else {
        return;
    };
    let horizontal = row.first_number(THICKNESS_DIRECTION) == Some(0.0);
    if !horizontal || (d.roundness >= 1.0 && !inf_on) {
        return;
    }
    if d.tip.is_some() {
        notes.push(Unrepresented::ClipStudio(SutNote::DualApprox(
            SutDualNote::ThicknessDirection,
        )));
        return;
    }
    d.angle = (d.angle + 90.0 + 180.0).rem_euclid(360.0) - 180.0;
}

/// 写さない設定（RGB 値の適用・連続吹き付け・リボン・繰り返し方法と反転・紙質・散布効果と固定点・先端同士の比較暗）が入っていれば知らせる。
fn unmapped(row: &Row, notes: &mut Vec<Unrepresented>) {
    let set = |columns: &[&str]| row.first_number(columns).is_some_and(|v| v != 0.0);
    for (on, note) in [
        (set(RGB), SutDualNote::Rgb),
        (set(CONTINUOUS), SutDualNote::Continuous),
        (set(RIBBON), SutDualNote::Ribbon),
        (
            set(ORDER) || set(REVERSE_H) || set(REVERSE_V),
            SutDualNote::TipRules,
        ),
        (
            row.first_blob(DUAL_TEXTURE_IMAGE).is_some(),
            SutDualNote::Texture,
        ),
        (set(SPRAY) || set(FIXED_POINT), SutDualNote::Spray),
        (set(DARKEN) || set(DARKEN_2), SutDualNote::Darken),
    ] {
        if on {
            notes.push(Unrepresented::ClipStudio(SutNote::DualApprox(note)));
        }
    }
}

/// 2 つ目の筆先の素材を決める: 名前で当て、当たらなければ、素材の種類が全部分かるファイルで、メインとデュアルの参照の数と筆先の素材の数が
/// ちょうど合うときだけ種類ごとの並びで当てる（主の筆先と同じ規則。並びで当てたものは推定）。複数の筆先は 1 つ目だけを使う。
fn dual_tip(
    row: &Row,
    name: &str,
    library: &mut Library<'_>,
    budget: &mut Budget,
    notes: &mut Vec<Unrepresented>,
) -> Result<DualTip, Fault> {
    if !row.on(DUAL_USE_PATTERN) {
        return Ok(DualTip::Round);
    }
    let Some(refs) = row.first_blob(DUAL_PATTERN_ARRAY).map(parse_refs) else {
        return Ok(DualTip::Missing);
    };
    if refs.items.is_empty() {
        return Ok(DualTip::Missing);
    }
    let main = match row.first_blob(PATTERN_ARRAY) {
        Some(blob) if row.first_number(USE_PATTERN).is_none_or(|v| v != 0.0) => parse_refs(blob),
        _ => Refs {
            items: Vec::new(),
            complete: true,
        },
    };
    let (materials, capped) = library.materials()?;
    let mut pick = resolve(&refs, materials, false).picks[0];
    if pick.is_none()
        && !capped
        && main.complete
        && refs.complete
        && materials.iter().all(|m| m.kind.is_some())
    {
        pick = by_kind_picks(&main, Some(&refs), materials, Kind::Tip)
            .and_then(|p| p.get(main.items.len()).copied().flatten());
    }
    let Some((index, guessed)) = pick else {
        return Ok(DualTip::Missing);
    };
    let Some((tip, preview)) = library.tip(index, &format!("{name} 2"), budget)? else {
        return Ok(DualTip::Missing);
    };
    let preview_note = Unrepresented::ClipStudio(SutNote::PreviewImage);
    if preview && !notes.contains(&preview_note) {
        notes.push(preview_note);
    }
    Ok(DualTip::Image(tip, guessed, refs.items.len() > 1))
}
