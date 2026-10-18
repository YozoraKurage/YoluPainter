//! 色の変化（`.sut` の「ブラシ先端色の変化」「サブ描画色混合率」「ストロークごとのランダム変化」「変化対象」）の取り込み。
//!
//! 列と単位: 先端は `BrushChangePatternColor`（旗）・`BrushHueChange`（−360〜360 度）・`BrushSaturationChange`・`BrushValueChange`
//! （−100〜100 ポイント）、サブ描画色混合率は `BrushSubColor`（0〜100。先端の旗によらず効く）、ストロークごとは
//! `BrushChangeStrokeColor`（旗）・`BrushStrokeHueChange`（0〜180 度）・`BrushStrokeSaturationChange`・`BrushStrokeValueChange`・
//! `BrushStrokeSubColor`（0〜100）、変化対象は `BrushChangeColorTarget`（0 メインとサブ・1 メイン・2 サブ）。影響元は
//! `Brush{Hue,Saturation,Value}ChangeEffector`・`BrushSubColorEffector`（他の影響元と同じ形の BLOB。色相・彩度・明度の既定は
//! ランダム・最小値 −100）。影響元は、それを使う値が入っているものだけ写す（切のブラシに影響元だけ残さない）。

use yolu_core::brush::ColorItem;
use yolu_core::{Brush, ColorTarget, StrokeColorChange, TipColorChange};

use super::super::notes::{ColorNote, SutMapped, SutNote, SutTarget, Unrepresented};
use super::db::Row;
use super::effector;

const TIP_FLAG: &[&str] = &["brushchangepatterncolor"];
const HUE: &[&str] = &["brushhuechange"];
const SATURATION: &[&str] = &["brushsaturationchange"];
const VALUE: &[&str] = &["brushvaluechange"];
const SUB: &[&str] = &["brushsubcolor"];
const STROKE_FLAG: &[&str] = &["brushchangestrokecolor"];
const STROKE_HUE: &[&str] = &["brushstrokehuechange"];
const STROKE_SATURATION: &[&str] = &["brushstrokesaturationchange"];
const STROKE_VALUE: &[&str] = &["brushstrokevaluechange"];
const STROKE_SUB: &[&str] = &["brushstrokesubcolor"];
const TARGET: &[&str] = &["brushchangecolortarget"];
const HUE_EFFECTOR: &[&str] = &["brushhuechangeeffector"];
const SATURATION_EFFECTOR: &[&str] = &["brushsaturationchangeeffector"];
const VALUE_EFFECTOR: &[&str] = &["brushvaluechangeeffector"];
const SUB_EFFECTOR: &[&str] = &["brushsubcoloreffector"];

/// 読む列の名前の全部。
pub(super) const KNOWN: &[&[&str]] = &[
    TIP_FLAG,
    HUE,
    SATURATION,
    VALUE,
    SUB,
    STROKE_FLAG,
    STROKE_HUE,
    STROKE_SATURATION,
    STROKE_VALUE,
    STROKE_SUB,
    TARGET,
    HUE_EFFECTOR,
    SATURATION_EFFECTOR,
    VALUE_EFFECTOR,
    SUB_EFFECTOR,
];

/// 列の値を `scale` で割り、lo〜hi に収める（読めない値・有限でない値は 0）。
fn value(row: &Row, columns: &[&str], scale: f64, lo: f64, hi: f64) -> f64 {
    let v = row.first_number(columns).unwrap_or(0.0) / scale;
    if v.is_finite() {
        v.clamp(lo, hi)
    } else {
        0.0
    }
}

/// 色の変化を写す。写せたら `SutMapped::ColorChange`、近い動き・当たらない設定・使っていない値は `SutNote` で知らせる。
pub(super) fn color_change(
    row: &Row,
    brush: &mut Brush,
    notes: &mut Vec<Unrepresented>,
    mapped: &mut Vec<SutMapped>,
) {
    let tip_on = row.on(TIP_FLAG);
    let stroke_on = row.on(STROKE_FLAG);
    let c = &mut brush.color;
    // 変化対象は、先端かストロークごとの色の変化を写すときだけ写す（使わないブラシはメインのままで、.ylbrush がバージョン 8 にならない）
    if tip_on || stroke_on {
        c.target = match row.first_number(TARGET) {
            Some(0.0) => ColorTarget::MainAndSub,
            Some(2.0) => ColorTarget::Sub,
            _ => ColorTarget::Main,
        };
    }
    if tip_on {
        c.tip = Some(TipColorChange {
            hue: value(row, HUE, 1.0, -360.0, 360.0),
            saturation: value(row, SATURATION, 100.0, -1.0, 1.0),
            value: value(row, VALUE, 100.0, -1.0, 1.0),
        });
    }
    c.sub_mix = value(row, SUB, 100.0, 0.0, 1.0);
    if stroke_on {
        c.stroke = Some(StrokeColorChange {
            hue: value(row, STROKE_HUE, 1.0, 0.0, 180.0),
            saturation: value(row, STROKE_SATURATION, 100.0, 0.0, 1.0),
            value: value(row, STROKE_VALUE, 100.0, 0.0, 1.0),
            sub_mix: value(row, STROKE_SUB, 100.0, 0.0, 1.0),
        });
    }
    let (sub_mix, tip, stroke, target) = (c.sub_mix, c.tip, c.stroke, c.target);
    // 値があっても切り替えが切のものは使っていない（CLIP STUDIO も当てない）と知らせる
    let unused_tip = !tip_on && [HUE, SATURATION, VALUE].iter().any(|c| row.on(c));
    let unused_stroke = !stroke_on
        && [STROKE_HUE, STROKE_SATURATION, STROKE_VALUE, STROKE_SUB]
            .iter()
            .any(|c| row.on(c));
    if unused_tip || unused_stroke {
        notes.push(Unrepresented::ClipStudio(SutNote::ColorChange));
    }
    if tip.is_none() && sub_mix <= 0.0 && stroke.is_none() {
        return;
    }
    mapped.push(SutMapped::ColorChange);
    for (used, target, columns) in [
        (tip.is_some(), SutTarget::ColorHue, HUE_EFFECTOR),
        (
            tip.is_some(),
            SutTarget::ColorSaturation,
            SATURATION_EFFECTOR,
        ),
        (tip.is_some(), SutTarget::ColorValue, VALUE_EFFECTOR),
        (sub_mix > 0.0, SutTarget::ColorSubMix, SUB_EFFECTOR),
    ] {
        if used {
            effector(brush, row, target, columns, notes, mapped);
        }
    }
    let uses_motion = ColorItem::ALL.into_iter().any(|item| {
        let i = brush.influences.color(item);
        i.tilt.on || i.speed.on
    });
    if uses_motion {
        notes.push(Unrepresented::ClipStudio(SutNote::ColorApprox(
            ColorNote::MotionInfluence,
        )));
    }
    if target == ColorTarget::Sub && (tip.is_some() || stroke.is_some()) {
        notes.push(Unrepresented::ClipStudio(SutNote::ColorApprox(
            ColorNote::TargetSub,
        )));
    }
    let both_hues = tip.is_some_and(|t| t.hue != 0.0) && stroke.is_some_and(|s| s.hue > 0.0);
    if both_hues && target != ColorTarget::Sub {
        notes.push(Unrepresented::ClipStudio(SutNote::ColorApprox(
            ColorNote::HueSum,
        )));
    }
}
