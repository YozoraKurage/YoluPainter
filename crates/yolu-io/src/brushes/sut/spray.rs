//! 散布効果（`.sut` の「散布効果」）の取り込み。
//!
//! 列と単位: 旗は `BrushUseSpray`、粒子サイズは `BrushSpraySize`（単位の列 `BrushSpraySizeUnit` が 0 のとき文書の画素の直径）、
//! ブラシサイズに連動は `BrushSpraySizeSyncBrushSize`、粒子密度は `BrushSprayDensity`、散布偏向は `BrushSprayBias`（−100〜100）、
//! 粒子の向きは `BrushRotationInSpray`（度）、向きの影響元は `BrushRotationEffectorInSpray`（整数の旗。0x01 はいつも立つ、0x40 が
//! 線の方向・0x80 がランダム・0x100 が散布全体の向き・0x200 が散布中心に集中。並びはデュアルブラシの同じ旗と同じと見ている。
//! ランダムの強さは `BrushRotationRandomInSpray`（0〜100））。粒子サイズ・粒子密度の影響元は `BrushSpraySizeEffector`・`BrushSprayDensityEffector`
//! （ほかの影響元と同じ形の BLOB。最小値は 0〜1）。固定点（`BrushSprayUseFixedPoint`・`BrushSprayFixedPointArray`）は読まない。
//!
//! 旗が立っていても、粒子サイズ・粒子密度の列が無い・粒子密度が 1 未満・単位が画素でない行は写さない（[`SutNote::Spray`] で知らせる）。

use yolu_core::{Brush, Spray, SprayDirection, MAX_SPRAY_SIZE};

use super::super::notes::{SprayNote, SutMapped, SutNote, SutTarget, Unrepresented};
use super::db::Row;
use super::effector;

const FLAG: &[&str] = &["brushusespray"];
const SIZE: &[&str] = &["brushspraysize"];
const SIZE_UNIT: &[&str] = &["brushspraysizeunit"];
const LINK_SIZE: &[&str] = &["brushspraysizesyncbrushsize"];
const DENSITY: &[&str] = &["brushspraydensity"];
const BIAS: &[&str] = &["brushspraybias"];
const ROTATION: &[&str] = &["brushrotationinspray"];
const ROTATION_EFFECTOR: &[&str] = &["brushrotationeffectorinspray"];
const ROTATION_RANDOM: &[&str] = &["brushrotationrandominspray"];
const SIZE_EFFECTOR: &[&str] = &["brushspraysizeeffector"];
const DENSITY_EFFECTOR: &[&str] = &["brushspraydensityeffector"];

/// 向きの影響元の旗のうち、ランダムのビット。
const ROTATION_RANDOM_BIT: u32 = 0x80;
/// 向きの影響元の旗のうち、いつも立っていて知らせずに読み飛ばすビット。
const ROTATION_PLAIN_BIT: u32 = 0x01;
/// 向きの影響元の旗の、向きを足すビット（線の方向・散布全体の向き・散布中心に集中）。
const ROTATION_DIRECTIONS: [(u32, SprayDirection); 3] = [
    (0x40, SprayDirection::Line),
    (0x100, SprayDirection::Whole),
    (0x200, SprayDirection::Center),
];

/// 読む列の名前の全部（旗の列は `sut.rs` の `KNOWN` にもある）。
pub(super) const KNOWN: &[&[&str]] = &[
    SIZE,
    SIZE_UNIT,
    LINK_SIZE,
    DENSITY,
    BIAS,
    ROTATION,
    ROTATION_EFFECTOR,
    ROTATION_RANDOM,
    SIZE_EFFECTOR,
    DENSITY_EFFECTOR,
];

/// 散布効果の列の名前（主は `Brush…`、2 つ目は `Dual…`）。
pub(super) struct SprayColumns {
    pub flag: &'static [&'static str],
    pub size: &'static [&'static str],
    pub size_unit: &'static [&'static str],
    pub link_size: &'static [&'static str],
    pub density: &'static [&'static str],
    pub bias: &'static [&'static str],
    pub rotation: &'static [&'static str],
    pub rotation_effector: &'static [&'static str],
    pub rotation_random: &'static [&'static str],
}

const MAIN: SprayColumns = SprayColumns {
    flag: FLAG,
    size: SIZE,
    size_unit: SIZE_UNIT,
    link_size: LINK_SIZE,
    density: DENSITY,
    bias: BIAS,
    rotation: ROTATION,
    rotation_effector: ROTATION_EFFECTOR,
    rotation_random: ROTATION_RANDOM,
};

/// 散布効果を写す。写せたら `SutMapped::Spray` と、近い動きの知らせ（[`SprayNote`]）。写せなければ何もしない（旗の知らせは
/// `unrepresented_flags` が出す）。
pub(super) fn spray(
    row: &Row,
    brush: &mut Brush,
    notes: &mut Vec<Unrepresented>,
    mapped: &mut Vec<SutMapped>,
) {
    let Some((s, approx)) = read(row, &MAIN) else {
        return;
    };
    brush.spray = Some(s);
    mapped.push(SutMapped::Spray);
    for (target, columns) in [
        (SutTarget::SpraySize, SIZE_EFFECTOR),
        (SutTarget::SprayDensity, DENSITY_EFFECTOR),
    ] {
        effector(brush, row, target, columns, notes, mapped);
    }
    for note in approx {
        notes.push(Unrepresented::ClipStudio(SutNote::SprayApprox(note)));
    }
}

/// 散布効果の列 `cols` を読む（主と 2 つ目で同じ処理）。旗が切・写せない（粒子サイズ・粒子密度が無い・粒子密度が 1 未満・単位が画素でない）
/// なら None。写せたら、散布と近い動きの知らせ（粒子密度はいつも、散布偏向・向きは当たるとき）。
pub(super) fn read(row: &Row, cols: &SprayColumns) -> Option<(Spray, Vec<SprayNote>)> {
    if !row.on(cols.flag) || row.on(cols.size_unit) {
        return None;
    }
    let (Some(size), Some(density)) = (row.first_number(cols.size), row.first_number(cols.density))
    else {
        return None;
    };
    // 密度 0 は「粒子を置かない」かもしれず、1 に収めると見た目が変わるので、1 未満は写さない。
    if !(size.is_finite() && size > 0.0 && density.is_finite() && density >= 1.0) {
        return None;
    }
    let number = |columns, scale: f64, lo: f64, hi: f64, missing: f64| {
        let v = row.first_number(columns).map_or(missing, |v| v / scale);
        if v.is_finite() {
            v.clamp(lo, hi)
        } else {
            missing
        }
    };
    let flags = row.first_number(cols.rotation_effector).unwrap_or(0.0);
    let flags = if flags.is_finite() && flags >= 0.0 {
        flags as u32
    } else {
        0
    };
    let random_angle = (flags & ROTATION_RANDOM_BIT != 0)
        .then(|| number(cols.rotation_random, 100.0, 0.0, 1.0, 1.0));
    let bias = number(cols.bias, 100.0, -1.0, 1.0, 0.0);
    // 向きを足すビットが 1 つだけのときそれを使う（複数は選べないので足さずに知らせる）
    let mut directions = ROTATION_DIRECTIONS
        .iter()
        .filter(|(bit, _)| flags & bit != 0);
    let direction = match (directions.next(), directions.next()) {
        (Some(&(_, d)), None) => d,
        _ => SprayDirection::None,
    };
    let known = ROTATION_RANDOM_BIT | ROTATION_PLAIN_BIT;
    let known = ROTATION_DIRECTIONS
        .iter()
        .fold(known, |k, (bit, _)| k | bit);
    let spray = Spray {
        size: size.min(MAX_SPRAY_SIZE),
        link_size: row.on(cols.link_size),
        density: density.min(100.0),
        bias,
        angle: number(cols.rotation, 1.0, -360.0, 360.0, 0.0),
        direction,
        random_angle,
    };
    let mut approx = vec![SprayNote::Density];
    if bias != 0.0 {
        approx.push(SprayNote::Bias);
    }
    let extra = flags & !known != 0;
    if extra
        || (direction == SprayDirection::None
            && flags & !(ROTATION_RANDOM_BIT | ROTATION_PLAIN_BIT) != 0)
    {
        approx.push(SprayNote::Direction);
    }
    Some((spray, approx))
}
