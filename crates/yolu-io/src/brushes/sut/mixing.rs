//! 下地混色（`.sut` の「インク」の色の混ぜ）の取り込み: 方式（色混ぜ・色重ね・にじみ）・絵の具量・絵の具濃度・色延び・混色モード・輝度の補正・
//! にじむ強さ・絵の具量と濃度の影響元。
//!
//! 列と値（1 項目ずつ変えた書き出しの比べで決まった）: `BrushUseWaterColor`（と `…2`。下地混色の旗）・`BrushWaterColor`（0 色混ぜ・1 にじみ・2 色重ね）・
//! `BrushMixColor`（絵の具量 0〜100）・`BrushMixAlpha`（絵の具濃度）・`BrushMixColorExtension`（色延び）・`BrushColorMixingMode`（0 標準・1 知覚的）・
//! `BrushLMSLinearity`（輝度の補正 0 なし〜4 最高）・`BrushBlurLinkSize`（0 自動設定・1 固定値）・`BrushBlur`（固定値。単位は `BrushBlurUnit`、0 が画素）。
//! 混色モードと輝度の補正の列は 187 列のバージョンにだけある（無ければ標準・なし）。
//!
//! 方式は core の [`MixMode`] へ: 色混ぜ = 混ぜる、色重ね = 重ねる、にじみ = 伸ばす。**値は 1 対 1 で写さず、エンジンの式の値へ換算する**
//! （[`convert`]）。エンジンの絵の具の量・色延びは「打点ごとの荷の更新の割合」で、CLIP STUDIO のものとは単位が違う（同じ数でも線が違う）。
//! 換算は方式ごとの近似の式（値は 1 対 1 で写らない）。

use yolu_core::brush::{MixBlur, MixMode, MixSpace, MAX_MIX_BLUR, MAX_MIX_CORRECTION};
use yolu_core::Brush;

use super::super::notes::{SutMapped, SutNote, SutTarget, Unrepresented};
use super::db::Row;
use super::{
    effector, BLUR, BLUR_LINK_SIZE, BLUR_UNIT, MIX_DENSITY, MIX_DENSITY_EFFECTOR, MIX_LUMINANCE,
    MIX_MODE, MIX_PAINT, MIX_PAINT_EFFECTOR, MIX_STRETCH, WATER_COLOR, WATER_COLOR_KIND,
};

/// CLIP STUDIO の絵の具量・絵の具濃度・色延び（0〜1）を、エンジンの値へ換算したもの。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Converted {
    pub paint: f64,
    pub density: f64,
    pub stretch: f64,
    pub load: f64,
}

/// 色延びを、エンジンの「打点ごとに荷に残る割合」へ（`exp(−κ · 間隔 / 色延び)`。間隔は直径に対する割合）。色延び 0 は荷を引きずらない。
/// 荷が下の色を覚えている**距離**が色延びで決まり、打点の間隔によらない形にしてある（打点ごとの割合のまま写すと、間隔を変えただけで引きずる
/// 距離が変わる）。κ は近似の換算の定数。
fn persistence(kappa: f64, stretch: f64, spacing: f64) -> f64 {
    if stretch <= 0.0 {
        0.0
    } else {
        (-kappa * spacing / stretch).exp().clamp(0.0, 1.0)
    }
}

/// 近似の換算（`spacing` はブラシの間隔、直径に対する割合）。両端（0 と 1）が自然な向きになる形にしてある。
pub(super) fn convert(
    mode: MixMode,
    space: MixSpace,
    paint: f64,
    density: f64,
    stretch: f64,
    spacing: f64,
) -> Converted {
    // 絵の具濃度は、50 で全部置く・それ未満で薄くなる（近似）
    let density_dab = (density * 2.0).min(1.0);
    match mode {
        // 色混ぜ: 打点ごとの置く色は、荷（描く色と下の色の記憶の混ざり）にほぼ寄せる。荷に占める描く色の釣り合い（色混ぜの釣り合い
        // k = 量 + 色延び × (1 − 量)）になるように、エンジンの絵の具の量を決める。標準の混ぜでは、描き始めが薄く立ち上がる
        // （最初の荷の描く色の割合 = 絵の具濃度の 0.47 倍）。知覚的では立ち上がらない。
        MixMode::Mix | MixMode::Off => {
            let keep = persistence(MIX_KAPPA, stretch, spacing);
            let balance = paint + stretch * (1.0 - paint);
            let denominator = 1.0 - keep * (1.0 - balance);
            Converted {
                paint: if denominator > 0.0 {
                    (balance / denominator).clamp(0.0, 1.0)
                } else {
                    1.0
                },
                density: density_dab,
                stretch: keep,
                load: if space == MixSpace::Perceptual {
                    1.0
                } else {
                    (0.47 * density).clamp(0.0, 1.0)
                },
            }
        }
        // 色重ね: 絵の具濃度は効かない（画面でも使えない）ので全部置く。描き始めは描く色だけ。
        MixMode::Layer => Converted {
            paint: 1.0 - LAYER_SLOPE * (1.0 - paint),
            density: 1.0,
            stretch: persistence(LAYER_KAPPA, stretch, spacing),
            load: 1.0,
        },
        // にじみ: 下の色が強く効く（量が小さいほど描く色は薄いまま、にじませて置く）。色延びは、読む位置の遅れへ効くので、ほとんど 0。
        MixMode::Smear => Converted {
            paint: paint.powf(SMEAR_PAINT_POWER),
            density: density_dab,
            stretch: stretch.powf(SMEAR_STRETCH_POWER),
            load: 1.0,
        },
    }
}

const MIX_KAPPA: f64 = 0.1;
const LAYER_KAPPA: f64 = 0.1;
const LAYER_SLOPE: f64 = 0.554;
const SMEAR_PAINT_POWER: f64 = 2.9;
const SMEAR_STRETCH_POWER: f64 = 2.8;

/// 下地混色を写す（絵の具の量・濃さの影響元も）。
pub(super) fn color_mixing(
    row: &Row,
    brush: &mut Brush,
    notes: &mut Vec<Unrepresented>,
    mapped: &mut Vec<SutMapped>,
) {
    // CLIP STUDIO は混色が切のブラシ（G ペンなど）にも既定の値（50 など）を書くので、値だけでは決めない（旗が立っているときだけ）。
    // 旗が立っていて写せる値が無いものは知らせる。
    let (paint, density, stretch) = (
        row.first_number(MIX_PAINT).unwrap_or(0.0),
        row.first_number(MIX_DENSITY).unwrap_or(0.0),
        row.first_number(MIX_STRETCH).unwrap_or(0.0),
    );
    if !row.on(WATER_COLOR) {
        return;
    }
    // 方式は 0 色混ぜ・1 にじみ・2 色重ね。列が無ければ色混ぜ
    let kind = row.first_number(WATER_COLOR_KIND).unwrap_or(0.0);
    let mode = if kind == 0.0 {
        MixMode::Mix
    } else if kind == 1.0 {
        MixMode::Smear
    } else if kind == 2.0 {
        MixMode::Layer
    } else {
        notes.push(Unrepresented::ClipStudio(SutNote::ColorMixingMethod(kind)));
        return;
    };
    if paint <= 0.0 && density <= 0.0 && stretch <= 0.0 {
        notes.push(Unrepresented::ClipStudio(SutNote::ColorMixing {
            paint,
            density,
            stretch,
        }));
        return;
    }
    let unit = |v: f64| (v / 100.0).clamp(0.0, 1.0);
    let space = if row.first_number(MIX_MODE) == Some(1.0) {
        MixSpace::Perceptual
    } else {
        MixSpace::Standard
    };
    let c = convert(
        mode,
        space,
        unit(paint),
        unit(density),
        unit(stretch),
        brush.base.spacing,
    );
    brush.mix.mode = mode;
    brush.mix.paint = c.paint;
    brush.mix.density = c.density;
    brush.mix.stretch = c.stretch;
    brush.mix.load = c.load;
    mixing_space(row, brush, notes);
    if mode == MixMode::Smear {
        smear_blur(row, brush, notes);
    }
    mapped.push(SutMapped::ColorMixing);
    // 絵の具の量・濃さの影響元は、混ぜを写せたブラシだけ（混ぜないブラシには効く所が無い）
    for (target, columns) in [
        (SutTarget::MixPaint, MIX_PAINT_EFFECTOR),
        (SutTarget::MixDensity, MIX_DENSITY_EFFECTOR),
    ] {
        effector(brush, row, target, columns, notes, mapped);
    }
}

/// 混色モード（標準・知覚的）と輝度の補正。
fn mixing_space(row: &Row, brush: &mut Brush, notes: &mut Vec<Unrepresented>) {
    let Some(v) = row.first_number(MIX_MODE) else {
        return;
    };
    if v == 1.0 {
        brush.mix.space = MixSpace::Perceptual;
        // 輝度の補正は知覚的のときだけ働く（標準では画面に選べない）。0〜4 の整数のほかは収めて知らせる
        if let Some(l) = row.first_number(MIX_LUMINANCE) {
            let level = l.round().clamp(0.0, f64::from(MAX_MIX_CORRECTION));
            if level != l {
                notes.push(Unrepresented::ClipStudio(SutNote::LuminanceCorrection(l)));
            }
            brush.mix.correction = level as u8;
        }
    } else if v != 0.0 {
        notes.push(Unrepresented::ClipStudio(SutNote::ColorMixingMode(v)));
    }
}

/// にじむ強さ（にじみだけに効く）: 自動設定は大きさに連動して広い形（[`MixBlur::Linked`]）、固定値は `BrushBlur`（画素）。
fn smear_blur(row: &Row, brush: &mut Brush, notes: &mut Vec<Unrepresented>) {
    // 列が無ければ自動設定（0 が自動設定、1 が固定値）。固定値が読めないときも自動設定
    brush.mix.blur = MixBlur::Linked;
    if row.first_number(BLUR_LINK_SIZE).unwrap_or(0.0) != 1.0 {
        return;
    }
    let value = row.first_number(BLUR);
    let unit_px = row.first_number(BLUR_UNIT).unwrap_or(0.0) == 0.0;
    match value.filter(|v| v.is_finite()) {
        Some(v) => {
            let px = v.clamp(0.0, MAX_MIX_BLUR);
            if !unit_px || px != v {
                notes.push(Unrepresented::ClipStudio(SutNote::MixBlur(Some(v))));
            }
            brush.mix.blur = MixBlur::Fixed(px);
        }
        // 固定値が読めない: 自動設定のまま知らせる
        None => notes.push(Unrepresented::ClipStudio(SutNote::MixBlur(None))),
    }
}
