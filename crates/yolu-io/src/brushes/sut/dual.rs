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
//! 複数の筆先は `DualPatternImageArray` の全部（1 つ目は `DualBrush::tip` にも置く）、繰り返し方法・反転は `DualPatternOrderType`・
//! `DualPatternReverseHorizontal`・`…Vertical`（主と同じ表と同じ関数 `tip_order_and_flips`）、先端同士の比較暗は `DualBlendPatternByDarken`、
//! 紙質は `DualTextureImage`・`DualTexture…`（主と同じ関数 `paper`。拡大率は `DualTextureScale`）、散布効果は `DualUseSpray`・`DualSpray…`・
//! `DualRotation…InSpray`（主と同じ関数 `spray::read`）。
//!
//! 2 つ目のサイズの列が無い行と、1 つ目の筆先の素材を決められない行は、デュアルブラシごと写さない（[`SutNote::DualBrush`]）。
//! 散布の旧形（`DualBrush::scatter`・`count`）は使わない。RGB 値の適用・連続吹き付け・リボン・紙質の濃度強調と描点ごとに適用・
//! 散布の固定点・紙質濃度と粒子の影響元・比較暗の 2 つ目の列（`DualBlendPatternByDarken2`）は写さずに知らせる（[`SutDualNote`]）。

use std::sync::Arc;

use yolu_core::{AntiAlias, Brush, BrushTip, DualBrush, DualBrushMode, PaperTexture, Spray};

use super::super::notes::{SutDualNote, SutMapped, SutNote, SutTarget, Unrepresented};
use super::blob::{parse_effector, parse_refs, Effector, Refs};
use super::db::Row;
use super::material::Kind;
use super::spray::SprayColumns;
use super::{
    by_kind_picks, effector, interval_spacing, paper, percent, resolve, tip_order_and_flips,
    Budget, Fault, Library, Pick, TextureColumns, TextureEvent, TipEvent, TipTarget,
    DUAL_PATTERN_ARRAY, DUAL_TEXTURE_IMAGE, DUAL_USE_PATTERN, MAX_TIPS, PATTERN_ARRAY,
    TEXTURE_IMAGE, USE_PATTERN,
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
const ORDER: &[&str] = &["dualpatternordertype"];
const REVERSE_H: &[&str] = &["dualpatternreversehorizontal"];
const REVERSE_V: &[&str] = &["dualpatternreversevertical"];
const DARKEN: &[&str] = &["dualblendpatternbydarken"];
const TEXTURE_MODE: &[&str] = &["dualtexturecompositemode"];
const TEXTURE_REVERSE: &[&str] = &["dualtexturereversedensity"];
const TEXTURE_SCALE: &[&str] = &["dualtexturescale"];
const TEXTURE_ROTATE: &[&str] = &["dualtexturerotate"];
const TEXTURE_BRIGHTNESS: &[&str] = &["dualtexturebrightness"];
const TEXTURE_CONTRAST: &[&str] = &["dualtexturecontrast"];
const TEXTURE_EACH_TIP: &[&str] = &["dualtextureforplot"];
const TEXTURE_DENSITY: &[&str] = &["dualtexturedensity"];
const TEXTURE_STRESS: &[&str] = &["dualtexturestressdensity"];
const TEXTURE_DENSITY_EFFECTOR: &[&str] = &["dualtexturedensityeffector"];
const SPRAY: &[&str] = &["dualusespray"];
const SPRAY_SIZE: &[&str] = &["dualspraysize"];
const SPRAY_SIZE_UNIT: &[&str] = &["dualspraysizeunit"];
const SPRAY_LINK_SIZE: &[&str] = &["dualspraysizesyncbrushsize"];
const SPRAY_DENSITY: &[&str] = &["dualspraydensity"];
const SPRAY_BIAS: &[&str] = &["dualspraybias"];
const SPRAY_ROTATION: &[&str] = &["dualrotationinspray"];
const SPRAY_ROTATION_EFFECTOR: &[&str] = &["dualrotationeffectorinspray"];
const SPRAY_ROTATION_RANDOM: &[&str] = &["dualrotationrandominspray"];
const SPRAY_SIZE_EFFECTOR: &[&str] = &["dualspraysizeeffector"];
const SPRAY_DENSITY_EFFECTOR: &[&str] = &["dualspraydensityeffector"];
// 写さずに知らせる設定の旗・値
const RGB: &[&str] = &["changergbbydual"];
const CONTINUOUS: &[&str] = &["dualcontinuousplot"];
const RIBBON: &[&str] = &["dualribbon"];
const FIXED_POINT: &[&str] = &["dualsprayusefixedpoint"];
const DARKEN_2: &[&str] = &["dualblendpatternbydarken2"];

/// 2 つ目の紙質の列（主の紙質と同じ並び）。
const TEXTURE: TextureColumns = TextureColumns {
    reverse: TEXTURE_REVERSE,
    brightness: TEXTURE_BRIGHTNESS,
    contrast: TEXTURE_CONTRAST,
    density: TEXTURE_DENSITY,
    scale: TEXTURE_SCALE,
    mode: TEXTURE_MODE,
    rotate: TEXTURE_ROTATE,
    each_tip: TEXTURE_EACH_TIP,
};

/// 2 つ目の散布効果の列（主の散布効果と同じ並び）。
const SPRAY_COLUMNS: SprayColumns = SprayColumns {
    flag: SPRAY,
    size: SPRAY_SIZE,
    size_unit: SPRAY_SIZE_UNIT,
    link_size: SPRAY_LINK_SIZE,
    density: SPRAY_DENSITY,
    bias: SPRAY_BIAS,
    rotation: SPRAY_ROTATION,
    rotation_effector: SPRAY_ROTATION_EFFECTOR,
    rotation_random: SPRAY_ROTATION_RANDOM,
};

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
    TEXTURE_MODE,
    TEXTURE_REVERSE,
    TEXTURE_SCALE,
    TEXTURE_ROTATE,
    TEXTURE_BRIGHTNESS,
    TEXTURE_CONTRAST,
    TEXTURE_EACH_TIP,
    TEXTURE_DENSITY,
    TEXTURE_STRESS,
    TEXTURE_DENSITY_EFFECTOR,
    SPRAY_SIZE,
    SPRAY_SIZE_UNIT,
    SPRAY_LINK_SIZE,
    SPRAY_DENSITY,
    SPRAY_BIAS,
    SPRAY_ROTATION,
    SPRAY_ROTATION_EFFECTOR,
    SPRAY_ROTATION_RANDOM,
    SPRAY_SIZE_EFFECTOR,
    SPRAY_DENSITY_EFFECTOR,
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
    /// 素材の画像（参照の順。並びで当てた物があるか・使えない素材があったか）。
    Images(Vec<Arc<BrushTip>>, bool, bool),
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
    let (tips, missing) = match dual_tip(row, name, library, budget, notes)? {
        DualTip::Round => (Vec::new(), false),
        DualTip::Images(tips, guessed, missing) => {
            if guessed {
                notes.push(Unrepresented::ClipStudio(SutNote::TipGuessed));
            }
            (tips, missing)
        }
        DualTip::Missing => return Ok(false),
    };
    let mut approx = |note| notes.push(Unrepresented::ClipStudio(SutNote::DualApprox(note)));
    if missing {
        approx(SutDualNote::Tips);
    }
    // 1 つ目は筆先にも置く（並びが 2 枚以上なら、並びが筆先に代わる）
    let mut d = DualBrush {
        radius: (size / 2.0).clamp(0.5, 65536.0),
        tip: tips.first().cloned(),
        tips: if tips.len() > 1 { tips } else { Vec::new() },
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
    tip_rules(row, &mut d, &mut approx);
    // 2 つ目の溜まりはいつも大きい方を残す（比較暗と同じ）ので、旗は写すだけで描き方は変わらない
    d.darken_overlap = row.on(DARKEN);
    d.texture = texture(row, name, library, budget, notes)?;
    d.spray = spray(row, notes);
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
    // 一巡のみ・ランダムに一巡の 1 枚は、筆先から並びへ移っている
    if d.tip.is_some() || !d.tips.is_empty() {
        notes.push(Unrepresented::ClipStudio(SutNote::DualApprox(
            SutDualNote::ThicknessDirection,
        )));
        return;
    }
    d.angle = (d.angle + 90.0 + 180.0).rem_euclid(360.0) - 180.0;
}

/// 写さない設定（RGB 値の適用・連続吹き付け・リボン・散布の固定点・比較暗の 2 つ目の列）が入っていれば知らせる。
fn unmapped(row: &Row, notes: &mut Vec<Unrepresented>) {
    let set = |columns: &[&str]| row.first_number(columns).is_some_and(|v| v != 0.0);
    for (on, note) in [
        (set(RGB), SutDualNote::Rgb),
        (set(CONTINUOUS), SutDualNote::Continuous),
        (set(RIBBON), SutDualNote::Ribbon),
        (set(FIXED_POINT), SutDualNote::SprayFixedPoint),
        (set(DARKEN_2), SutDualNote::Darken),
    ] {
        let note = Unrepresented::ClipStudio(SutNote::DualApprox(note));
        if on && !notes.contains(&note) {
            notes.push(note);
        }
    }
}

/// 2 つ目の筆先の繰り返し方法・反転（主と同じ関数）。知らない値・近い動きは知らせる。
fn tip_rules(row: &Row, d: &mut DualBrush, approx: &mut impl FnMut(SutDualNote)) {
    let events = tip_order_and_flips(
        row,
        ORDER,
        [REVERSE_H, REVERSE_V],
        TipTarget {
            image: &mut d.tip,
            images: &mut d.tips,
            selection: &mut d.selection,
            flip_x: (&mut d.flip_x, &mut d.flip_x_rule),
            flip_y: (&mut d.flip_y, &mut d.flip_y_rule),
        },
    );
    for event in events {
        match event {
            TipEvent::Order | TipEvent::Flip => {}
            TipEvent::OrderApprox => approx(SutDualNote::TipOrderApprox),
            TipEvent::OrderUnknown | TipEvent::FlipUnknown => approx(SutDualNote::TipRules),
        }
    }
}

/// 2 つ目の紙質（主と同じ関数 [`paper`]。素材の決め方だけが違う）。濃度強調・描点ごとに適用・紙質濃度の影響元は知らせる。
fn texture(
    row: &Row,
    name: &str,
    library: &mut Library<'_>,
    budget: &mut Budget,
    notes: &mut Vec<Unrepresented>,
) -> Result<Option<PaperTexture>, Fault> {
    let Some(pick) = texture_pick(row, library)? else {
        return Ok(None);
    };
    let (texture, events) = paper(
        row,
        &TEXTURE,
        pick,
        &format!("{name} texture 2"),
        library,
        budget,
    )?;
    for event in events {
        let note = match event {
            TextureEvent::Missing => SutNote::DualApprox(SutDualNote::Texture),
            TextureEvent::Proprietary => SutNote::ProprietaryImage,
            TextureEvent::Guessed => SutNote::DualApprox(SutDualNote::TextureGuessed),
            TextureEvent::Mode => SutNote::DualApprox(SutDualNote::TextureMode),
            TextureEvent::Mapped | TextureEvent::Rotated => continue,
            TextureEvent::Toned => SutNote::DualApprox(SutDualNote::TextureTone),
            TextureEvent::Preview => SutNote::PreviewImage,
            TextureEvent::EachTip => SutNote::DualApprox(SutDualNote::TextureEachTip),
        };
        let note = Unrepresented::ClipStudio(note);
        if !(event.once() && notes.contains(&note)) {
            notes.push(note);
        }
    }
    if texture.is_some() {
        let mut approx = |note| notes.push(Unrepresented::ClipStudio(SutNote::DualApprox(note)));
        if row.on(TEXTURE_STRESS) {
            approx(SutDualNote::TextureEmphasis);
        }
        if effector_on(row, TEXTURE_DENSITY_EFFECTOR) {
            approx(SutDualNote::Sources);
        }
    }
    Ok(texture)
}

/// 2 つ目の紙質の素材（参照の列が無ければ None、決められなければ `Some(None)`）。名前で当て、当たらなければ、素材の種類が全部分かる
/// ファイルで、メインとデュアルの紙質の参照の数と紙質の素材の数がちょうど合うときだけ種類ごとの並びで当てる（主の紙質と同じ規則）。
/// 参照を読めないときは決めない（主の紙質は、素材が 1 つだけならそれにするが、2 つ目は主の素材と取り違えうる）。
fn texture_pick(row: &Row, library: &mut Library<'_>) -> Result<Option<Option<Pick>>, Fault> {
    let Some(refs) = row.first_blob(DUAL_TEXTURE_IMAGE).map(parse_refs) else {
        return Ok(None);
    };
    if refs.items.is_empty() {
        return Ok(Some(None));
    }
    let (materials, capped) = library.materials()?;
    let named = resolve(&refs, materials, false)
        .picks
        .first()
        .copied()
        .flatten();
    if named.is_some() || capped {
        return Ok(Some(named));
    }
    let main = row.first_blob(TEXTURE_IMAGE).map_or(
        Refs {
            items: Vec::new(),
            complete: true,
        },
        parse_refs,
    );
    let by_kind = (main.complete && refs.complete && materials.iter().all(|m| m.kind.is_some()))
        .then(|| by_kind_picks(&main, Some(&refs), materials, Kind::Texture))
        .flatten()
        .and_then(|p| p.get(main.items.len()).copied().flatten());
    Ok(Some(by_kind))
}

/// 2 つ目の散布効果（主と同じ関数 `spray::read`）。旗が立っていて写せなければ、粒子サイズ・粒子密度の影響元は知らせる。
fn spray(row: &Row, notes: &mut Vec<Unrepresented>) -> Option<Spray> {
    let mut approx = |note| notes.push(Unrepresented::ClipStudio(SutNote::DualApprox(note)));
    let Some((spray, close)) = super::spray::read(row, &SPRAY_COLUMNS) else {
        if row.on(SPRAY) {
            approx(SutDualNote::Spray);
        }
        return None;
    };
    for note in close {
        approx(SutDualNote::SprayApprox(note));
    }
    if effector_on(row, SPRAY_SIZE_EFFECTOR) || effector_on(row, SPRAY_DENSITY_EFFECTOR) {
        approx(SutDualNote::Sources);
    }
    Some(spray)
}

/// 影響元の列が入力を 1 つでも使うか（読めない影響元も、使っているかもしれないので true）。
fn effector_on(row: &Row, columns: &[&str]) -> bool {
    let inputs = Effector::PRESSURE | Effector::TILT | Effector::SPEED | Effector::RANDOM;
    match row.first_blob(columns) {
        Some(bytes) => parse_effector(bytes).is_none_or(|e| e.flags & inputs != 0),
        None => row.is_oversized(columns),
    }
}

/// 2 つ目の筆先の素材を決める: 参照ごとに名前で当て、当たらなければ、素材の種類が全部分かるファイルで、メインとデュアルの参照の数と
/// 筆先の素材の数がちょうど合うときだけ種類ごとの並びで当てる（主の筆先と同じ規則。並びで当てたものは推定）。1 つ目を決められなければ
/// `Missing`。2 つ目より後の決められない・取り出せない・上限（主と同じ 256 枚）を超えた素材は使わず、欠けたと返す。
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
    let picks = tip_picks(row, &refs, library)?;
    let mut missing = !refs.complete || picks.len() > MAX_TIPS;
    let (mut tips, mut guessed, mut preview) = (Vec::new(), false, false);
    for (n, pick) in picks.into_iter().take(MAX_TIPS).enumerate() {
        // 1 つ目の名前は今までと同じ（画像の名前）
        let label = if n == 0 {
            format!("{name} 2")
        } else {
            format!("{name} 2-{}", n + 1)
        };
        let tip = match pick {
            Some((index, by_order)) => library
                .tip(index, &label, budget)?
                .map(|(tip, is_preview)| (tip, is_preview, by_order)),
            None => None,
        };
        match tip {
            Some((tip, is_preview, by_order)) => {
                tips.push(tip);
                guessed |= by_order;
                preview |= is_preview;
            }
            None if n == 0 => return Ok(DualTip::Missing),
            None => missing = true,
        }
    }
    let preview_note = Unrepresented::ClipStudio(SutNote::PreviewImage);
    if preview && !notes.contains(&preview_note) {
        notes.push(preview_note);
    }
    Ok(DualTip::Images(tips, guessed, missing))
}

/// 2 つ目の筆先の参照ごとの素材（[`dual_tip`] の当て方。決められない参照は None）。
fn tip_picks(
    row: &Row,
    refs: &Refs,
    library: &mut Library<'_>,
) -> Result<Vec<Option<Pick>>, Fault> {
    let main = match row.first_blob(PATTERN_ARRAY) {
        Some(blob) if row.first_number(USE_PATTERN).is_none_or(|v| v != 0.0) => parse_refs(blob),
        _ => Refs {
            items: Vec::new(),
            complete: true,
        },
    };
    let (materials, capped) = library.materials()?;
    let named = resolve(refs, materials, false).picks;
    let by_kind = (named.iter().any(Option::is_none)
        && !capped
        && main.complete
        && refs.complete
        && materials.iter().all(|m| m.kind.is_some()))
    .then(|| by_kind_picks(&main, Some(refs), materials, Kind::Tip))
    .flatten();
    Ok(named
        .into_iter()
        .enumerate()
        .map(|(i, pick)| {
            pick.or_else(|| {
                by_kind
                    .as_ref()
                    .and_then(|p| p.get(main.items.len() + i).copied().flatten())
            })
        })
        .collect())
}
