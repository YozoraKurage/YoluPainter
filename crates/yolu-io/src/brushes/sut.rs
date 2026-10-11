//! CLIP STUDIO PAINT のサブツール `.sut`（SQLite）。
//!
//! 形式は公開されていないので、公開の解析（出どころと確かな所・推測の所は docs/BRUSH_IMPORT.md）で分かる範囲だけを読む:
//! - ブラシの名前と、現在の設定・既定の設定の `Variant` の番号は `Node` から。設定は `Variant` の 1 行（1 列が 1 つの設定。列の
//!   集合は CLIP STUDIO の版で違うので、決まった名前の列を「在れば」読む）。
//! - 筆先・質感の画像は `MaterialFile.FileData`（無圧縮の tar）。原寸の画像は tar の中の独自の入れ物 C2F（`c2f`）から、読めなければ
//!   プレビューの PNG。どの素材を使うかは `Variant` の参照の BLOB の名前から決める。名前で当たらない参照は、残った素材（素材の種類が
//!   分かるときは種類ごと）と参照の数がちょうど合うときだけ並びで当て、そのときは推定として [`SutNote`] で知らせる。決められない・
//!   取り出せない筆先は使わず、欠けたことを知らせる（黙って別の画像を使わない）。
//! - 影響元設定（`*Effector`）は筆圧（最小値と曲線）と、傾き（大きさ・不透明度・流量へ。曲線は直線に近似）。速さ・ランダムは表せない
//!   ので注記する。入り抜き（大きさへ。長さは画素）・手ぶれ補正・筆先の角度・角度のランダムも写す。写せたものは [`SutMapped`]、
//!   近似したものは [`SutNote`] にも載せる。
//! - 表せない設定（色の混ぜの旗だけのもの・吹き付け・デュアルブラシ・色の変化・合成モードなど）は、旗が立っているものだけ
//!   [`SutNote`] で知らせる。列の名前と単位は公開の解析からの推定を含むので、本物のファイルでの確かめは別に要る（確かめた所と
//!   推測の所は docs/BRUSH_IMPORT.md）。
//!
//! 信頼できないファイルなので、データベースを開く・読む所は `db`（読み取り専用・メモリ上・上限つき）だけが触る。

mod blob;
mod c2f;
mod db;
mod material;
mod tar;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use yolu_core::brush::MAX_STROKE_ASSIST;
use yolu_core::curve::{Curve, CurvePoint};
use yolu_core::{AntiAlias, Brush, BrushTip, PaperTexture, PressureResponse, TipSelection};

use super::error::{BrushImportError, Fault};
use super::notes::{Source, SutInput, SutMapped, SutNote, SutTarget, Unrepresented};
use super::png_tip;
use super::reader::Budget;
use super::{short_text, ImportedBrush, ImportedSet, SkipReason, SkippedBrush};
use blob::{parse_effector, parse_refs, Effector, Refs};
use db::{Cell, Database, Material, Row, Variants};
use material::Kind;

/// 取り出した画像（PNG の元のバイト列）の合計の上限（バイト）。
const MAX_PNG_TOTAL: u64 = 128 * 1024 * 1024;

/// 1 つのブラシの筆先の枚数の上限（core の `Brush::validate` と同じ。超える分は使わずに知らせる）。
const MAX_TIPS: usize = 256;

// `Variant` の列の名前（小文字。在れば読む。先に書いたものを優先）。読む列はここに挙げたものだけで、`KNOWN` に全部入れる
// （`db` はこの名前の列だけを選ぶ）。
const SIZE: &[&str] = &["brushsize"];
const OPACITY: &[&str] = &["opacity", "brushopacity"];
const FLOW: &[&str] = &["brushflow"];
const HARDNESS: &[&str] = &["brushhardness"];
const INTERVAL: &[&str] = &["brushinterval"];
const THICKNESS: &[&str] = &["brushthickness"];
const ROTATION: &[&str] = &["brushrotation"];
const USE_PATTERN: &[&str] = &["brushusepatternimage"];
const PATTERN_ARRAY: &[&str] = &["brushpatternimagearray"];
const TEXTURE_IMAGE: &[&str] = &["textureimage"];
// デュアルブラシの筆先・質感の参照（デュアルブラシ自体は表せないが、ファイルにはその素材も入るので、素材の数え合わせに使う）
const DUAL_PATTERN_ARRAY: &[&str] = &["dualpatternimagearray"];
const DUAL_USE_PATTERN: &[&str] = &["dualusepatternimage"];
const DUAL_TEXTURE_IMAGE: &[&str] = &["dualtextureimage"];
const TEXTURE_SCALE: &[&str] = &["texturescale2", "texturescale"];
const TEXTURE_DENSITY: &[&str] = &["texturedensity"];
const TEXTURE_REVERSE: &[&str] = &["texturereversedensity"];
const TEXTURE_ROTATE: &[&str] = &["texturerotate"];
const TEXTURE_BRIGHTNESS: &[&str] = &["texturebrightness"];
const TEXTURE_CONTRAST: &[&str] = &["texturecontrast"];
const TEXTURE_MODE: &[&str] = &["texturecompositemode"];
const TEXTURE_EACH_TIP: &[&str] = &["textureforplot"];
const SIZE_EFFECTOR: &[&str] = &["brushsizeeffector"];
const OPACITY_EFFECTOR: &[&str] = &["opacityeffector", "brushopacityeffector"];
const FLOW_EFFECTOR: &[&str] = &["brushfloweffector"];
const THICKNESS_EFFECTOR: &[&str] = &["brushthicknesseffector"];
const WATER_COLOR: &[&str] = &["brushusewatercolor", "brushusewatercolor2"];
const MIX_PAINT: &[&str] = &["brushmixcolor"];
const MIX_DENSITY: &[&str] = &["brushmixalpha"];
const MIX_STRETCH: &[&str] = &["brushmixcolorextension"];
const WATER_EDGE: &[&str] = &["brushusewateredge"];
const SPRAY: &[&str] = &["brushusespray"];
const DUAL: &[&str] = &["usedualbrush"];
// 入り抜き: 入り・抜きそれぞれの旗・長さ・長さの単位（0 が画素）。影響先は `BrushInOutType`（0 が既定）と、`BrushInOutTarget`（BLOB）。
const USE_IN: &[&str] = &["brushusein"];
const USE_OUT: &[&str] = &["brushuseout"];
const IN_LENGTH: &[&str] = &["brushinlength"];
const OUT_LENGTH: &[&str] = &["brushoutlength"];
const IN_UNIT: &[&str] = &["brushinlengthunit"];
const OUT_UNIT: &[&str] = &["brushoutlengthunit"];
const IN_OUT_TYPE: &[&str] = &["brushinouttype"];
const IN_OUT_TARGET: &[&str] = &["brushinouttarget"];
// 手ぶれ補正の旗と強さ（段階）
const USE_REVISION: &[&str] = &["brushuserevision"];
const REVISION: &[&str] = &["brushrevision"];
// 筆先の向きの影響元（旗の整数。0x80 がランダム。0x10・0x20・0x40 は影響元の旗と同じ並び）と、ランダムの強さ（百分率の整数）
const ROTATION_EFFECTOR: &[&str] = &["brushrotationeffector"];
const ROTATION_RANDOM: &[&str] = &["brushrotationrandomscale"];
const COLOR_CHANGE: &[&str] = &[
    "brushhuechange",
    "brushsaturationchange",
    "brushvaluechange",
    "brushsubcolor",
];
const BLEND_MODE: &[&str] = &["compositemode"];
// アンチエイリアス（整数。0 なし・1 弱・2 中・3 強と推定: 画面の選びの順。手元の書き出しでは 0・1・2 を見た）。デュアルブラシの
// `DualAntiAlias` はデュアルブラシごと表せないので読まない
const ANTI_ALIAS: &[&str] = &["antialias"];

/// 読む列の名前の全部。
const KNOWN: &[&[&str]] = &[
    SIZE,
    OPACITY,
    FLOW,
    HARDNESS,
    INTERVAL,
    THICKNESS,
    ROTATION,
    USE_PATTERN,
    PATTERN_ARRAY,
    TEXTURE_IMAGE,
    DUAL_PATTERN_ARRAY,
    DUAL_USE_PATTERN,
    DUAL_TEXTURE_IMAGE,
    TEXTURE_SCALE,
    TEXTURE_DENSITY,
    TEXTURE_REVERSE,
    TEXTURE_ROTATE,
    TEXTURE_BRIGHTNESS,
    TEXTURE_CONTRAST,
    TEXTURE_MODE,
    TEXTURE_EACH_TIP,
    SIZE_EFFECTOR,
    OPACITY_EFFECTOR,
    FLOW_EFFECTOR,
    THICKNESS_EFFECTOR,
    WATER_COLOR,
    MIX_PAINT,
    MIX_DENSITY,
    MIX_STRETCH,
    WATER_EDGE,
    SPRAY,
    DUAL,
    USE_IN,
    USE_OUT,
    IN_LENGTH,
    OUT_LENGTH,
    IN_UNIT,
    OUT_UNIT,
    IN_OUT_TYPE,
    IN_OUT_TARGET,
    USE_REVISION,
    REVISION,
    ROTATION_EFFECTOR,
    ROTATION_RANDOM,
    COLOR_CHANGE,
    BLEND_MODE,
    ANTI_ALIAS,
];

/// 割合の設定（0〜100 の百分率か 0〜1 の割合か列によって混ざるので、1 より大きければ百分率として見る）。
fn ratio(value: f64) -> f64 {
    if value > 1.0 {
        value / 100.0
    } else {
        value
    }
}

/// 割合の列を割合（0〜1 の外も許す）で。整数の列は、実物の .sut ではどれも 0〜100 の百分率（`Opacity`・`BrushFlow`・`BrushHardness`・
/// `BrushThickness`・`TextureDensity` などが整数で宣言され、硬さの 1 は 1%）なので、いつも 100 で割る。実数の列は `ratio`
/// （1 より大きければ百分率、以下なら割合）。
fn percent(row: &Row, names: &[&str]) -> Option<f64> {
    names.iter().find_map(|n| match row.get(n)? {
        Cell::Int(i) => Some(*i as f64 / 100.0),
        Cell::Real(r) if r.is_finite() => Some(ratio(*r)),
        _ => None,
    })
}

// ---------------- 素材の特定 ----------------

/// 参照と素材の文字列を突き合わせるための印: 場所と拡張子を落とした小文字。
fn key(text: &str) -> String {
    let base = text.rsplit(['/', '\\']).next().unwrap_or(text);
    let stem = match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && ext.len() <= 5 => stem,
        _ => base,
    };
    stem.trim().to_lowercase()
}

/// どの素材にも付いている一般的な名前（突き合わせに使わない）。
const GENERIC: [&str; 4] = ["thumbnail", "material", "catalog", "layer"];

fn same_material(reference: &str, text: &str) -> bool {
    let (a, b) = (key(reference), key(text));
    if a.len() < 3 || b.len() < 3 || GENERIC.contains(&a.as_str()) || GENERIC.contains(&b.as_str())
    {
        return false;
    }
    a == b || (a.len().min(b.len()) >= 8 && (a.contains(&b) || b.contains(&a)))
}

/// 参照 1 つの素材: 素材の番号（素材の並びの中の位置）と、名前で当たらず並びで当てたか。
type Pick = (usize, bool);

/// 参照の並びから決めた素材。
struct Resolved {
    /// 参照ごとの素材。決められなかった参照は None（参照の並びと同じ長さ。別の参照の素材を詰めない）。
    picks: Vec<Option<Pick>>,
}

/// 参照の並びから、使う素材を決める。名前で当たらなかった参照は、当たった素材を除いた素材を並び（先頭から）で当てる。ただし
/// 並びで決めてよいのは `order_ok`（そのブラシの素材の参照が 1 種類だけで、素材を読み切れている）で、参照の個数が最後まで
/// 読めていて、名前で当たらなかった参照の数と、まだ使われていない素材の数がちょうど同じときだけ（ファイルには、読まない設定
/// 〔デュアルブラシなど〕の素材も入りうるので、素材が多いときは決めない）。決められなければ、名前で当たった分だけにする。
fn resolve(refs: &Refs, materials: &[Material], order_ok: bool) -> Resolved {
    let found: Vec<Option<usize>> = refs
        .items
        .iter()
        .map(|item| {
            materials.iter().position(|m| {
                item.names
                    .iter()
                    .any(|n| m.texts.iter().any(|t| same_material(n, t)))
            })
        })
        .collect();
    let named = |found: Vec<Option<usize>>| Resolved {
        picks: found.into_iter().map(|f| f.map(|i| (i, false))).collect(),
    };
    let unmatched = found.iter().filter(|f| f.is_none()).count();
    if unmatched == 0 || !order_ok || !refs.complete {
        return named(found);
    }
    let used: HashSet<usize> = found.iter().flatten().copied().collect();
    let unused: Vec<usize> = (0..materials.len()).filter(|i| !used.contains(i)).collect();
    if unmatched != unused.len() {
        return named(found);
    }
    let mut rest = unused.into_iter();
    Resolved {
        picks: found
            .into_iter()
            .map(|f| match f {
                Some(i) => Some((i, false)),
                None => rest.next().map(|i| (i, true)),
            })
            .collect(),
    }
}

/// 素材の種類（筆先・質感）が全部の素材で分かるファイルで、種類ごとに並びで当てる。
///
/// デュアルブラシを使う設定のファイルには、メインの筆先・質感の素材と、デュアルブラシの筆先・質感の素材が種類ごとにまとまって入る
/// （本物の 1 つのファイルで、現在の設定が参照する 4 つの素材だけが入り、種類の中ではメインが先だった）。そこで、その種類の参照
/// （メイン + デュアル）のうち名前で当たらなかったものの数と、まだ使われていないその種類の素材の数が**ちょうど同じ**ときに限って、
/// 参照の順（メインが先）と素材の並びで当てる。返すのはメインの参照の分だけ。素材が多い・少ないとき（別のブラシの素材も入っている
/// など）、種類の分からない素材があるときは決めない（None）。並びで当てたものは推定として知らせる。
fn resolve_by_kind(
    main: &Refs,
    dual: Option<&Refs>,
    materials: &[Material],
    kind: Kind,
) -> Option<Resolved> {
    if main.items.is_empty() || !main.complete || materials.iter().any(|m| m.kind.is_none()) {
        return None;
    }
    if dual.is_some_and(|d| !d.complete) {
        return None;
    }
    let candidates: Vec<usize> = (0..materials.len())
        .filter(|&i| materials[i].kind == Some(kind))
        .collect();
    let items: Vec<&blob::RefItem> = main
        .items
        .iter()
        .chain(dual.map_or(&[][..], |d| &d.items[..]))
        .collect();
    let found: Vec<Option<usize>> = items
        .iter()
        .map(|item| {
            candidates.iter().copied().find(|&i| {
                item.names
                    .iter()
                    .any(|n| materials[i].texts.iter().any(|t| same_material(n, t)))
            })
        })
        .collect();
    let used: HashSet<usize> = found.iter().flatten().copied().collect();
    let unused: Vec<usize> = candidates
        .iter()
        .copied()
        .filter(|i| !used.contains(i))
        .collect();
    if found.iter().filter(|f| f.is_none()).count() != unused.len() {
        return None;
    }
    let mut rest = unused.into_iter();
    let picks = found
        .into_iter()
        .map(|f| match f {
            Some(i) => Some((i, false)),
            None => rest.next().map(|i| (i, true)),
        })
        .take(main.items.len())
        .collect();
    Some(Resolved { picks })
}

/// デュアルブラシを使う設定の、デュアルの素材の参照。使わない設定・参照の無い設定は None。
fn dual_refs(row: &Row, column: &[&str], use_flag: Option<&[&str]>) -> Option<Refs> {
    if !row.on(DUAL) || use_flag.is_some_and(|flag| !row.on(flag)) {
        return None;
    }
    row.first_blob(column).map(parse_refs)
}

// ---------------- 影響元の曲線 ----------------

/// 影響元の曲線の点を core の筆圧の曲線の形（点 2〜16・両端は入力 0 と 1・間隔 0.02 以上・値 0〜1）にする。CLIP STUDIO の曲線は
/// 点を通る曲線として扱う。16 点を超えるときは、16 点に取り直して `true` を返す。点が足りなければ空（直線）。
fn curve_points(raw: &[(f64, f64)]) -> (Vec<CurvePoint>, bool) {
    let mut pts: Vec<(f64, f64)> = raw
        .iter()
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .map(|&(x, y)| (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)))
        .collect();
    if pts.len() < 2 {
        return (Vec::new(), false);
    }
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let gap = Curve::MIN_GAP + 1e-9;
    if pts[0].0 < gap {
        pts[0].0 = 0.0;
    } else {
        pts.insert(0, (0.0, pts[0].1));
    }
    let last = pts.len() - 1;
    if 1.0 - pts[last].0 < gap {
        pts[last].0 = 1.0;
    } else {
        pts.push((1.0, pts[last].1));
    }
    let end = pts[pts.len() - 1];
    let mut kept = vec![pts[0]];
    for &p in &pts[1..pts.len() - 1] {
        if p.0 - kept[kept.len() - 1].0 >= gap && end.0 - p.0 >= gap {
            kept.push(p);
        }
    }
    kept.push(end);
    let simplified = kept.len() > Curve::MAX_POINTS;
    if simplified {
        let n = Curve::MAX_POINTS;
        let sample = |x: f64| {
            let i = kept
                .windows(2)
                .position(|w| x <= w[1].0)
                .unwrap_or(kept.len() - 2);
            let (a, b) = (kept[i], kept[i + 1]);
            let t = if b.0 > a.0 {
                (x - a.0) / (b.0 - a.0)
            } else {
                0.0
            };
            a.1 + (b.1 - a.1) * t.clamp(0.0, 1.0)
        };
        kept = (0..n)
            .map(|i| {
                let x = if i == n - 1 {
                    1.0
                } else {
                    i as f64 / (n - 1) as f64
                };
                (x, sample(x).clamp(0.0, 1.0))
            })
            .collect();
    }
    (
        kept.into_iter().map(|(x, y)| CurvePoint { x, y }).collect(),
        simplified,
    )
}

/// 傾きの影響の曲線が、直線（直立で 1・寝かせきって 0）から外れてよい最大の幅。これ以内なら近似の注記を付けない。
const TILT_STRAIGHT: f64 = 0.1;

/// 曲線（点を通る折れ線。x の昇順でなくてもよい）の x での値。点が 2 つに満たない・有限でない点だけなら None。
fn curve_value(points: &[(f64, f64)], x: f64) -> Option<f64> {
    let mut pts: Vec<(f64, f64)> = points
        .iter()
        .filter(|(px, py)| px.is_finite() && py.is_finite())
        .map(|&(px, py)| (px.clamp(0.0, 1.0), py.clamp(0.0, 1.0)))
        .collect();
    if pts.len() < 2 {
        return None;
    }
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    if x <= pts[0].0 {
        return Some(pts[0].1);
    }
    let last = pts[pts.len() - 1];
    if x >= last.0 {
        return Some(last.1);
    }
    let i = pts.windows(2).position(|w| x <= w[1].0)?;
    let (a, b) = (pts[i], pts[i + 1]);
    let t = if b.0 > a.0 {
        (x - a.0) / (b.0 - a.0)
    } else {
        0.0
    };
    Some(a.1 + (b.1 - a.1) * t)
}

/// 傾きの曲線が「直立で 1・寝かせきって 0 へ下がる」形なら、直線 1 − x からの最大の外れ。そうでなければ（上がる・途中で止まる・
/// 読めない）None。このアプリの傾きの影響は、大きさ・不透明度・流量へ 1 − 傾き（直立 0〜寝かせきって 1）を掛けるだけで、
/// 曲線も最小値も持たないので、全幅で下がる曲線だけを近似する。
fn tilt_fit(points: &[(f64, f64)]) -> Option<f64> {
    let (top, bottom) = (curve_value(points, 0.0)?, curve_value(points, 1.0)?);
    if top < 0.9 || bottom > 0.1 {
        return None;
    }
    (0..=20)
        .map(|i| {
            let x = i as f64 / 20.0;
            curve_value(points, x).map(|y| (y - (1.0 - x)).abs())
        })
        .try_fold(0.0f64, |worst, d| d.map(|d| worst.max(d)))
}

/// 傾きの影響を、大きさ・不透明度・流量の傾きの切り替えに写す。写せたら true（写せなければ呼び出し側が「表せない」と知らせる）。
/// 速さも使う設定は、筆圧でない 2 つ目の曲線がどちらの入力のものか決められないので写さない。太さ（真円率）には傾きの影響が無い。
fn map_tilt(
    brush: &mut Brush,
    e: &Effector,
    target: SutTarget,
    notes: &mut Vec<Unrepresented>,
    mapped: &mut Vec<SutMapped>,
) -> bool {
    if e.flags & Effector::SPEED != 0 {
        return false;
    }
    let Some(deviation) = e.tilt_curve().and_then(tilt_fit) else {
        return false;
    };
    let controls = &mut brush.controls;
    match target {
        SutTarget::Size => controls.tilt_size = true,
        SutTarget::Opacity => controls.tilt_opacity = true,
        SutTarget::Flow => controls.tilt_flow = true,
        SutTarget::Thickness => return false,
    }
    mapped.push(SutMapped::Tilt);
    if deviation > TILT_STRAIGHT {
        notes.push(Unrepresented::ClipStudio(SutNote::TiltCurve(target)));
    }
    true
}

/// 影響元の列 1 つを読んで、筆圧は応えに、傾きは傾きの切り替えに、ほかの入力は注記にする。
fn effector(
    brush: &mut Brush,
    row: &Row,
    target: SutTarget,
    columns: &[&str],
    notes: &mut Vec<Unrepresented>,
    mapped: &mut Vec<SutMapped>,
) {
    let Some(bytes) = row.first_blob(columns) else {
        if row.is_oversized(columns) {
            notes.push(Unrepresented::ClipStudio(SutNote::InfluenceUnreadable(
                target,
            )));
        }
        return;
    };
    let Some(e) = parse_effector(bytes) else {
        notes.push(Unrepresented::ClipStudio(SutNote::InfluenceUnreadable(
            target,
        )));
        return;
    };
    for (flag, input) in [
        (Effector::TILT, SutInput::Tilt),
        (Effector::SPEED, SutInput::Speed),
        (Effector::RANDOM, SutInput::Random),
    ] {
        if e.flags & flag == 0 {
            continue;
        }
        if input == SutInput::Tilt && map_tilt(brush, &e, target, notes, mapped) {
            continue;
        }
        notes.push(Unrepresented::ClipStudio(SutNote::Influence {
            target,
            input,
        }));
    }
    if e.flags & Effector::PRESSURE == 0 {
        return;
    }
    let (points, simplified) = e.pressure_curve().map_or((Vec::new(), false), curve_points);
    if simplified {
        notes.push(Unrepresented::ClipStudio(SutNote::CurveSimplified(target)));
    }
    let response = PressureResponse::new(e.pressure_min, points)
        .or_else(|_| PressureResponse::new(e.pressure_min, Vec::new()));
    let Ok(response) = response else {
        return;
    };
    match target {
        SutTarget::Size => {
            brush.base.pressure_size = true;
            brush.pressure.size = response;
        }
        SutTarget::Opacity => {
            brush.base.pressure_opacity = true;
            brush.pressure.opacity = response;
        }
        SutTarget::Flow => {
            brush.base.pressure_flow = true;
            brush.pressure.flow = response;
        }
        SutTarget::Thickness => {
            notes.push(Unrepresented::ClipStudio(SutNote::ThicknessPressure));
            return;
        }
    }
    mapped.push(SutMapped::Pressure);
}

/// 入り抜きの影響先が既定（大きさだけ）か。`BrushInOutType` が 0 で、`BrushInOutTarget`（ビッグエンディアンの整数: 12・項目の数・12 のあと、
/// 項目ごとに「項目の番号・旗・旗」）の旗がすべて 0 のとき。実物の .sut では、項目は 21 個で、旗はどれも 0 だった。読めない・形が違う・
/// 旗が立っているものは、大きさへの入り抜きと決められないので false。影響先の列が無い版は既定（大きさ）として扱う。
fn start_end_targets_size(row: &Row) -> bool {
    if row.first_number(IN_OUT_TYPE).is_some_and(|v| v != 0.0) {
        return false;
    }
    let Some(blob) = row.first_blob(IN_OUT_TARGET) else {
        return !row.is_oversized(IN_OUT_TARGET);
    };
    let words: Vec<u32> = blob
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_be_bytes(*c))
        .collect();
    if blob.len() % 4 != 0 || words.len() < 3 || words[0] != 12 || words[2] != 12 {
        return false;
    }
    let count = words[1] as usize;
    words.len() == 3 + count * 3
        && words[3..]
            .as_chunks::<3>()
            .0
            .iter()
            .all(|item| item[1] == 0 && item[2] == 0)
}

/// 入り抜き: 大きさにだけ効く、長さが画素のものを `assist.taper_in`・`taper_out` へ。影響先・長さの単位が合わないものは写さずに知らせる。
/// 速さに応じた長さ・割合は読まず、写したときに知らせる。
fn start_end(
    row: &Row,
    brush: &mut Brush,
    notes: &mut Vec<Unrepresented>,
    mapped: &mut Vec<SutMapped>,
) {
    let sides = [
        (USE_IN, IN_LENGTH, IN_UNIT, true),
        (USE_OUT, OUT_LENGTH, OUT_UNIT, false),
    ];
    if !sides.iter().any(|(flag, ..)| row.on(flag)) {
        return;
    }
    let mut refused = !start_end_targets_size(row);
    let mut taken = false;
    if !refused {
        for (flag, length, unit, is_in) in sides {
            if !row.on(flag) {
                continue;
            }
            let pixels = row.first_number(length).filter(|l| *l >= 0.0);
            let in_pixels = row.first_number(unit).is_none_or(|u| u == 0.0);
            match pixels {
                Some(0.0) => {}
                Some(l) if in_pixels => {
                    let l = l.min(MAX_STROKE_ASSIST);
                    if is_in {
                        brush.assist.taper_in = l;
                    } else {
                        brush.assist.taper_out = l;
                    }
                    taken = true;
                }
                _ => refused = true,
            }
        }
    }
    if refused {
        notes.push(Unrepresented::ClipStudio(SutNote::StartEnd));
    }
    if taken {
        mapped.push(SutMapped::StartEnd);
        notes.push(Unrepresented::ClipStudio(SutNote::StartEndDetail));
    }
}

/// 手ぶれ補正: 強さ（段階）を糸の長さ（画素）へ 1 対 1 で写す（換算は推定）。強さが読めなければ写さずに知らせる。
fn stabilizer(
    row: &Row,
    brush: &mut Brush,
    notes: &mut Vec<Unrepresented>,
    mapped: &mut Vec<SutMapped>,
) {
    if !row.on(USE_REVISION) {
        return;
    }
    match row.first_number(REVISION).filter(|l| *l > 0.0) {
        Some(level) => {
            brush.assist.stabilizer = level.min(MAX_STROKE_ASSIST);
            mapped.push(SutMapped::Stabilizer);
            notes.push(Unrepresented::ClipStudio(SutNote::StabilizerStrength));
        }
        None => notes.push(Unrepresented::ClipStudio(SutNote::Stabilizer)),
    }
}

/// 筆先の向き: 角度（度）を `tip.angle` へ（向きの正負は確かめていないので、エンジンと同じ向きとして写す）、向きの影響元のランダム
/// （旗 0x80 と強さ）を `jitter.angle` へ。筆圧・傾き・速さ（旗 0x10・0x20・0x40）と、読めない強さのランダムは表せないので知らせる。旗の下の 2 ビットは意味を
/// 確かめられていない（実物ではどのブラシにも立っていた）ので読まない。
fn tip_direction(
    row: &Row,
    brush: &mut Brush,
    notes: &mut Vec<Unrepresented>,
    mapped: &mut Vec<SutMapped>,
) {
    if let Some(angle) = row.first_number(ROTATION).filter(|a| *a != 0.0) {
        brush.tip.angle = (angle + 180.0).rem_euclid(360.0) - 180.0;
        mapped.push(SutMapped::TipAngle);
    }
    let influences = row.first_number(ROTATION_EFFECTOR).map_or(0, |v| v as i64);
    let mut unmapped = influences & 0x70 != 0;
    if influences & 0x80 != 0 {
        // ランダムの強さが読めなければ写せない（強さ 0 は、ゆらがないので写すものが無い）
        match percent(row, ROTATION_RANDOM).map(|r| r.clamp(0.0, 1.0)) {
            Some(random) if random > 0.0 => {
                brush.jitter.angle = random;
                mapped.push(SutMapped::AngleRandom);
            }
            Some(_) => {}
            None => unmapped = true,
        }
    }
    if unmapped {
        notes.push(Unrepresented::ClipStudio(SutNote::Direction));
    }
}

// ---------------- 取り込み ----------------

/// 1 回の取り込みの間、素材と、その画像を作った結果を持つ。
struct Library<'a> {
    db: &'a Database,
    materials: Option<(Vec<Material>, bool)>,
    png_budget: u64,
    /// 素材の C2F を読む仕事の残り（全部の素材で共有）。
    c2f_work: c2f::Work,
    tips: HashMap<usize, Option<Arc<BrushTip>>>,
    textures: HashMap<(usize, bool), Option<Arc<BrushTip>>>,
    /// 画像を取り出せなかった素材（使おうとしたもの）。
    unreadable: HashSet<usize>,
}

impl<'a> Library<'a> {
    fn new(db: &'a Database) -> Library<'a> {
        Library {
            db,
            materials: None,
            png_budget: MAX_PNG_TOTAL,
            c2f_work: c2f::Work::new(),
            tips: HashMap::new(),
            textures: HashMap::new(),
            unreadable: HashSet::new(),
        }
    }

    /// 素材（`MaterialFile` の行の順）と、素材が上限を超えて読み切れていないか。
    fn materials(&mut self) -> Result<(&[Material], bool), Fault> {
        if self.materials.is_none() {
            self.materials = Some(self.db.materials(&mut self.png_budget, &self.c2f_work)?);
        }
        let (materials, capped) = self.materials.as_ref().expect("読み込み済み");
        Ok((materials, *capped))
    }

    /// 素材の画像を筆先（暗いほど塗り）にする。取り出せない・読めない画像は None（`unreadable` に数える）。
    fn tip(
        &mut self,
        index: usize,
        name: &str,
        budget: &mut Budget,
    ) -> Result<Option<(Arc<BrushTip>, bool)>, Fault> {
        if let Some(done) = self.tips.get(&index) {
            return Ok(done.clone().map(|t| (t, self.preview(index))));
        }
        let png = self.materials()?.0[index]
            .image
            .as_ref()
            .map(|i| i.png.clone());
        let tip = match png.map(|png| png_tip::read_png_tip(&png, name)) {
            Some(Ok(tip)) => {
                budget.take(tip.width() as u64 * tip.height() as u64)?;
                Some(Arc::new(tip))
            }
            _ => {
                self.unreadable.insert(index);
                None
            }
        };
        self.tips.insert(index, tip.clone());
        Ok(tip.map(|t| (t, self.preview(index))))
    }

    /// 素材の画像を質感（白が塗れる）にする。
    fn texture(
        &mut self,
        index: usize,
        name: &str,
        invert: bool,
        budget: &mut Budget,
    ) -> Result<Option<(Arc<BrushTip>, bool)>, Fault> {
        if let Some(done) = self.textures.get(&(index, invert)) {
            return Ok(done.clone().map(|t| (t, self.preview(index))));
        }
        let png = self.materials()?.0[index]
            .image
            .as_ref()
            .map(|i| i.png.clone());
        let tip = match png.map(|png| png_tip::read_png_texture(&png, name, invert)) {
            Some(Ok(tip)) => {
                budget.take(tip.width() as u64 * tip.height() as u64)?;
                Some(Arc::new(tip))
            }
            _ => {
                self.unreadable.insert(index);
                None
            }
        };
        self.textures.insert((index, invert), tip.clone());
        Ok(tip.map(|t| (t, self.preview(index))))
    }

    /// 画像が CLIP STUDIO 独自の入れ物にだけあって読めない素材が 1 つでもあるか。
    fn any_proprietary(&mut self) -> Result<bool, Fault> {
        Ok(self.materials()?.0.iter().any(|m| m.proprietary))
    }

    fn preview(&self, index: usize) -> bool {
        self.materials
            .as_ref()
            .and_then(|(m, _)| m.get(index))
            .and_then(|m| m.image.as_ref())
            .is_some_and(|i| i.preview)
    }
}

/// 筆先の素材を決めて、筆先にする（`image`・`images`・向き・選び方は呼び出し側が `Brush` へ入れる）。使えない筆先（決められない・
/// 画像を取り出せない・数の上限を超えた）は使わず、1 枚でも欠けたら `TipMissing`、並びで当てた筆先を使ったら `TipGuessed` を積む。
fn tips(
    row: &Row,
    name: &str,
    library: &mut Library<'_>,
    budget: &mut Budget,
    notes: &mut Vec<Unrepresented>,
) -> Result<Vec<Arc<BrushTip>>, Fault> {
    let array = row.first_blob(PATTERN_ARRAY);
    let used = match row.first_number(USE_PATTERN) {
        Some(v) => v != 0.0,
        None => array.is_some(),
    };
    if !used {
        return Ok(Vec::new());
    }
    let refs = array.map(parse_refs);
    let has_texture = row.first_blob(TEXTURE_IMAGE).is_some();
    let (all, capped) = {
        let (materials, capped) = library.materials()?;
        (materials.len(), capped)
    };
    // 並びで決めてよいのは、素材の参照が筆先だけで、素材を読み切れているとき
    let order_ok = !has_texture && !capped;
    // 参照ごとの素材と、参照を最後まで読めたか
    let (picks, complete) = match &refs {
        Some(refs) if !refs.items.is_empty() => {
            let materials = library.materials()?.0;
            let mut resolved = resolve(refs, materials, order_ok);
            // 名前でも並びでも決まらなかった参照は、素材の種類が分かれば種類ごとに当てる
            if !capped && resolved.picks.iter().any(Option::is_none) {
                let dual = dual_refs(row, DUAL_PATTERN_ARRAY, Some(DUAL_USE_PATTERN));
                if let Some(by_kind) = resolve_by_kind(refs, dual.as_ref(), materials, Kind::Tip) {
                    resolved = by_kind;
                }
            }
            (resolved.picks, refs.complete)
        }
        // 参照を読めない（形が違う）。素材の参照が筆先だけのブラシなら、素材はすべてそのブラシの筆先とみなす（推定）
        _ if order_ok => ((0..all).map(|i| Some((i, true))).collect(), true),
        _ => (Vec::new(), true),
    };
    let mut missing = !complete || picks.iter().any(Option::is_none);
    let mut out = Vec::new();
    let mut preview = false;
    let mut guessed = false;
    for (n, (index, by_order)) in picks.into_iter().flatten().enumerate() {
        if out.len() >= MAX_TIPS {
            missing = true;
            break;
        }
        let label = format!("{name} {}", n + 1);
        match library.tip(index, &label, budget)? {
            Some((tip, is_preview)) => {
                preview |= is_preview;
                guessed |= by_order;
                out.push(tip);
            }
            None => missing = true,
        }
    }
    if out.is_empty() || missing {
        notes.push(Unrepresented::ClipStudio(SutNote::TipMissing));
        if library.any_proprietary()? {
            notes.push(Unrepresented::ClipStudio(SutNote::ProprietaryImage));
        }
    }
    if !out.is_empty() {
        if guessed {
            notes.push(Unrepresented::ClipStudio(SutNote::TipGuessed));
        }
        if preview {
            notes.push(Unrepresented::ClipStudio(SutNote::PreviewImage));
        }
    }
    Ok(out)
}

/// 質感を決めて `Brush` へ入れる。
fn texture(
    row: &Row,
    name: &str,
    library: &mut Library<'_>,
    budget: &mut Budget,
    brush: &mut Brush,
    notes: &mut Vec<Unrepresented>,
    mapped: &mut Vec<SutMapped>,
) -> Result<(), Fault> {
    let Some(bytes) = row.first_blob(TEXTURE_IMAGE) else {
        return Ok(());
    };
    let refs = parse_refs(bytes);
    let uses_pattern = match row.first_number(USE_PATTERN) {
        Some(v) => v != 0.0,
        None => row.first_blob(PATTERN_ARRAY).is_some(),
    };
    // 質感の素材は 1 つ目の参照だけ（2 つ目以降が当たっても、1 つ目が決まらないときに別の素材を質感にしない）
    let pick: Option<Pick> = if refs.items.is_empty() {
        // 参照を読めない: 素材が 1 つだけで筆先に使っていないなら、それ（推定）
        let (materials, _) = library.materials()?;
        (!uses_pattern && materials.len() == 1).then_some((0, true))
    } else {
        let (materials, capped) = library.materials()?;
        let named = resolve(&refs, materials, !uses_pattern && !capped)
            .picks
            .first()
            .copied()
            .flatten();
        if named.is_some() || capped {
            named
        } else {
            // 名前でも並びでも決まらなければ、素材の種類が分かれば種類ごとに当てる
            let dual = dual_refs(row, DUAL_TEXTURE_IMAGE, None);
            resolve_by_kind(&refs, dual.as_ref(), materials, Kind::Texture)
                .and_then(|r| r.picks.first().copied().flatten())
        }
    };
    let invert = row.on(TEXTURE_REVERSE);
    let found = match pick {
        Some((index, by_order)) => library
            .texture(index, &format!("{name} texture"), invert, budget)?
            .map(|(image, preview)| (image, preview, by_order)),
        None => None,
    };
    let Some((image, preview, guessed)) = found else {
        notes.push(Unrepresented::ClipStudio(SutNote::TextureMissing));
        let note = Unrepresented::ClipStudio(SutNote::ProprietaryImage);
        if library.any_proprietary()? && !notes.contains(&note) {
            notes.push(note);
        }
        return Ok(());
    };
    if guessed {
        notes.push(Unrepresented::ClipStudio(SutNote::TextureGuessed));
    }
    let depth = percent(row, TEXTURE_DENSITY).map_or(1.0, |d| d.clamp(0.0, 1.0));
    let scale = row
        .first_number(TEXTURE_SCALE)
        .map(ratio)
        .filter(|s| *s > 0.0)
        .unwrap_or(1.0)
        .clamp(0.05, 64.0);
    brush.texture = Some(PaperTexture {
        image,
        depth,
        scale,
        mode: yolu_core::TextureMode::Multiply,
    });
    mapped.push(SutMapped::Texture);
    if preview && !notes.contains(&Unrepresented::ClipStudio(SutNote::PreviewImage)) {
        notes.push(Unrepresented::ClipStudio(SutNote::PreviewImage));
    }
    for (columns, note) in [
        (TEXTURE_ROTATE, SutNote::TextureRotation),
        (TEXTURE_BRIGHTNESS, SutNote::TextureBrightness),
        (TEXTURE_CONTRAST, SutNote::TextureContrast),
        (TEXTURE_MODE, SutNote::TextureMode),
        (TEXTURE_EACH_TIP, SutNote::TextureEachTip),
    ] {
        if row.on(columns) {
            notes.push(Unrepresented::ClipStudio(note));
        }
    }
    Ok(())
}

/// `Variant` の 1 行から core のブラシを作る。写せた設定の一覧と、表せなかった設定の一覧を添える。
fn brush_from_row(
    row: &Row,
    name: &str,
    library: &mut Library<'_>,
    budget: &mut Budget,
) -> Result<(Brush, Vec<Unrepresented>, Vec<SutMapped>), Fault> {
    let mut notes: Vec<Unrepresented> = Vec::new();
    let mut mapped: Vec<SutMapped> = Vec::new();
    let mut brush = Brush::default();
    brush.base.pressure_size = false;
    brush.base.pressure_opacity = false;
    brush.base.pressure_flow = false;

    let images = tips(row, name, library, budget, &mut notes)?;
    let mut tip_side = None;
    match images.len() {
        0 => {
            if let Some(h) = percent(row, HARDNESS) {
                brush.base.hardness = h.clamp(0.0, 1.0);
            }
        }
        1 => {
            tip_side = Some(images[0].width().max(images[0].height()) as f64);
            brush.tip.image = Some(images[0].clone());
            mapped.push(SutMapped::TipImage);
        }
        _ => {
            tip_side = images
                .iter()
                .map(|t| t.width().max(t.height()) as f64)
                .reduce(f64::max);
            brush.tip.images = images;
            brush.tip.selection = TipSelection::Random;
            mapped.push(SutMapped::TipImage);
            notes.push(Unrepresented::ClipStudio(SutNote::TipOrder));
        }
    }
    // 大きさの設定も筆先の画像も無ければ、core の既定の半径のまま
    if let Some(diameter) = row.first_number(SIZE).filter(|v| *v > 0.0).or(tip_side) {
        brush.base.radius = (diameter.min(2000.0) / 2.0).max(0.5);
    }
    if let Some(v) = percent(row, OPACITY) {
        brush.base.opacity = v.clamp(0.0, 1.0);
    }
    if let Some(v) = percent(row, FLOW) {
        brush.base.flow = v.clamp(0.0, 1.0);
    }
    if let Some(v) = row.first_number(INTERVAL) {
        brush.base.spacing = ratio(v).clamp(0.01, 4.0);
    }
    if let Some(v) = percent(row, THICKNESS) {
        brush.tip.roundness = v.clamp(0.0, 1.0).max(0.01);
    }
    // アンチエイリアス: 0〜3 を なし・弱・中・強へ。知らない値は写さずに（なし のまま）知らせる
    if let Some(v) = row.first_number(ANTI_ALIAS) {
        let level = (v.fract() == 0.0 && (0.0..=3.0).contains(&v))
            .then(|| AntiAlias::from_index(v as u8))
            .flatten();
        match level {
            Some(level) => {
                brush.base.anti_alias = level;
                mapped.push(SutMapped::AntiAliasing);
            }
            None => notes.push(Unrepresented::ClipStudio(SutNote::AntiAliasing(v))),
        }
    }

    for (target, columns) in [
        (SutTarget::Size, SIZE_EFFECTOR),
        (SutTarget::Opacity, OPACITY_EFFECTOR),
        (SutTarget::Flow, FLOW_EFFECTOR),
        (SutTarget::Thickness, THICKNESS_EFFECTOR),
    ] {
        effector(&mut brush, row, target, columns, &mut notes, &mut mapped);
    }

    texture(
        row,
        name,
        library,
        budget,
        &mut brush,
        &mut notes,
        &mut mapped,
    )?;
    tip_direction(row, &mut brush, &mut notes, &mut mapped);
    start_end(row, &mut brush, &mut notes, &mut mapped);
    stabilizer(row, &mut brush, &mut notes, &mut mapped);

    // 色の混ぜ: 混色の旗（BrushUseWaterColor）が立っているときだけ、絵の具量（BrushMixColor）・絵の具濃度（BrushMixAlpha）・色延び
    // （BrushMixColorExtension）を、0〜100 から 0〜1 にして「絵の具で混ぜる」へ写す（CLIP STUDIO の絵の具量 100 は下の色を拾わない＝
    // こちらの量 1 と同じ向き）。CLIP STUDIO は混色が切のブラシ（G ペンなど）にも既定の値（50 など）を書くので、値だけでは決めない。
    // 旗が立っていて写せる値が無いものは知らせる。
    let (paint, density, stretch) = (
        row.first_number(MIX_PAINT).unwrap_or(0.0),
        row.first_number(MIX_DENSITY).unwrap_or(0.0),
        row.first_number(MIX_STRETCH).unwrap_or(0.0),
    );
    let mixing = row.on(WATER_COLOR);
    if mixing && (paint > 0.0 || density > 0.0 || stretch > 0.0) {
        let unit = |v: f64| (v / 100.0).clamp(0.0, 1.0);
        brush.mix.mode = yolu_core::brush::MixMode::Mix;
        brush.mix.paint = unit(paint);
        brush.mix.density = unit(density);
        brush.mix.stretch = unit(stretch);
        mapped.push(SutMapped::ColorMixing);
    } else if mixing {
        notes.push(Unrepresented::ClipStudio(SutNote::ColorMixing {
            paint,
            density,
            stretch,
        }));
    }
    if row.on(WATER_EDGE) {
        notes.push(Unrepresented::WetEdges);
    }
    for (columns, note) in [
        (SPRAY, SutNote::Spray),
        (DUAL, SutNote::DualBrush),
        (COLOR_CHANGE, SutNote::ColorChange),
        (BLEND_MODE, SutNote::BlendMode),
    ] {
        // 旗の列も、色の変化の値の列も、「0 でない数が 1 つでもある」で知る
        if columns
            .iter()
            .any(|c| row.number(c).is_some_and(|v| v != 0.0))
        {
            notes.push(Unrepresented::ClipStudio(note));
        }
    }
    Ok((brush, notes, mapped))
}

pub(crate) fn read_sut(
    bytes: &[u8],
    fallback: Option<&str>,
    budget: &mut Budget,
) -> Result<ImportedSet, BrushImportError> {
    let db = Database::open(bytes)?;
    let (nodes, capped) = db.nodes()?;
    if nodes.is_empty() {
        return Err(Fault::SutNoBrushes.into());
    }
    let known: Vec<&str> = KNOWN
        .iter()
        .flat_map(|names| names.iter().copied())
        .collect();
    let mut variants: Option<Variants<'_>> = db.variants(&known)?;
    let mut library = Library::new(&db);
    let mut set = ImportedSet::default();
    for node in nodes {
        let name = if node.name.is_empty() {
            short_text(fallback.unwrap_or(""), 128)
        } else {
            node.name.clone()
        };
        let (brush, notes, mapped) = match &mut variants {
            None => (
                Brush::default(),
                vec![Unrepresented::ClipStudio(SutNote::SettingsMissing)],
                Vec::new(),
            ),
            Some(variants) => {
                let mut row = None;
                for id in [node.variant, node.init_variant] {
                    if id != 0 {
                        row = variants.get(id)?;
                        if row.is_some() {
                            break;
                        }
                    }
                }
                let Some(row) = row else {
                    set.skipped.push(SkippedBrush {
                        name,
                        reason: SkipReason::SettingsNotInFile,
                    });
                    continue;
                };
                brush_from_row(&row, &name, &mut library, budget)?
            }
        };
        set.brushes.push(
            ImportedBrush::new(&name, Source::ClipStudioSut, brush, notes)?.with_mapped(mapped),
        );
    }
    if set.brushes.is_empty() {
        return Err(Fault::SutNoBrushes.into());
    }
    if !library.unreadable.is_empty() {
        set.notes
            .push(Unrepresented::ClipStudio(SutNote::MaterialsUnreadable(
                library.unreadable.len(),
            )));
    }
    if capped > 0 {
        set.notes
            .push(Unrepresented::ClipStudio(SutNote::BrushesCapped(capped)));
    }
    if library.materials.as_ref().is_some_and(|(_, more)| *more) {
        set.notes
            .push(Unrepresented::ClipStudio(SutNote::MaterialsCapped));
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_ratios_are_percent_when_above_one_and_integers_are_always_percent() {
        assert_eq!(ratio(100.0), 100.0 / 100.0);
        assert_eq!(ratio(25.0), 0.25);
        assert_eq!(ratio(0.5), 0.5);
        assert_eq!(ratio(1.0), 1.0);
        assert_eq!(ratio(400.0), 4.0);
        let row = Row::default()
            .with("realone", Cell::Real(1.0))
            .with("intone", Cell::Int(1))
            .with("realhalf", Cell::Real(0.5))
            .with("intfull", Cell::Int(100))
            .with("text", Cell::Text("x".into()));
        // 実物の .sut の整数の列（硬さ 1 は 1%）。実数の 1 は割合の 1
        assert_eq!(percent(&row, &["intone"]), Some(0.01));
        assert_eq!(percent(&row, &["realone"]), Some(1.0));
        assert_eq!(percent(&row, &["realhalf"]), Some(0.5));
        assert_eq!(percent(&row, &["intfull"]), Some(1.0));
        assert_eq!(percent(&row, &["text", "missing"]), None);
        assert_eq!(
            percent(&row, &["missing", "intone"]),
            Some(0.01),
            "先に書いた名前の、読める列"
        );
    }

    #[test]
    fn tilt_curves_are_fitted_only_when_they_fall_over_the_full_range() {
        // 直線 1 − x: 外れ 0
        let straight = tilt_fit(&[(0.0, 1.0), (1.0, 0.0)]).unwrap();
        assert!(straight < 1e-9);
        // 途中まで 1 のまま落ちる曲線は、直線から大きく外れる（近似の注記が付く幅）
        let plateau = tilt_fit(&[(0.0, 1.0), (0.364, 1.0), (0.609, 0.0), (1.0, 0.0)]).unwrap();
        assert!(plateau > TILT_STRAIGHT);
        // 上がる・途中までしか下がらない・点が足りない・有限でない点だけは写さない
        assert_eq!(tilt_fit(&[(0.0, 0.0), (1.0, 1.0)]), None);
        assert_eq!(tilt_fit(&[(0.0, 1.0), (1.0, 0.5)]), None);
        assert_eq!(tilt_fit(&[(0.0, 0.5), (1.0, 0.0)]), None);
        assert_eq!(tilt_fit(&[(0.0, 1.0)]), None);
        assert_eq!(tilt_fit(&[(f64::NAN, 1.0), (1.0, f64::INFINITY)]), None);
        // x の昇順でない点・範囲外の値も読める
        assert!(tilt_fit(&[(1.0, -3.0), (0.0, 4.0)]).unwrap() < 1e-9);
    }

    #[test]
    fn curves_become_valid_core_curves() {
        let straight = |p: &[(f64, f64)]| {
            let (points, simplified) = curve_points(p);
            assert!(!simplified);
            points
        };
        // 端が 0 と 1 でなければ足す・近い端は寄せる・近すぎる点は落とす・並べ替える
        let p = straight(&[(0.5, 0.9), (0.0, 0.0), (1.0, 1.0), (0.505, 0.2)]);
        assert_eq!(p.len(), 3);
        assert!(Curve::new(p).is_ok());
        let p = straight(&[(0.3, 0.2), (0.7, 0.8)]);
        assert_eq!((p[0].x, p[0].y, p[3].x, p[3].y), (0.0, 0.2, 1.0, 0.8));
        assert!(Curve::new(p).is_ok());
        let p = straight(&[(0.01, 0.1), (0.995, 0.9)]);
        assert_eq!((p[0].x, p[1].x), (0.0, 1.0));
        // 範囲外の値は収める
        let p = straight(&[(-1.0, -2.0), (2.0, 3.0)]);
        assert_eq!((p[0].x, p[0].y, p[1].x, p[1].y), (0.0, 0.0, 1.0, 1.0));
        // 点が 1 つ・有限でない点は直線（空）
        assert!(curve_points(&[(0.5, 0.5)]).0.is_empty());
        assert!(curve_points(&[(f64::NAN, 0.5), (0.5, f64::INFINITY)])
            .0
            .is_empty());
        assert!(curve_points(&[]).0.is_empty());
        // 16 を超えたら 16 点に取り直す
        let many: Vec<(f64, f64)> = (0..40)
            .map(|i| (i as f64 / 39.0, (i as f64 / 39.0).powi(2)))
            .collect();
        let (points, simplified) = curve_points(&many);
        assert!(simplified);
        assert_eq!(points.len(), 16);
        assert!(Curve::new(points).is_ok());
    }

    #[test]
    fn any_curve_a_file_can_hold_becomes_a_valid_response_or_a_straight_line() {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let special = [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            -1.0,
            0.0,
            1.0,
            2.0,
            1e-9,
            0.019_999,
            0.02,
            0.98,
            0.999_999,
            1e300,
        ];
        let mut straight = 0;
        for round in 0..30_000 {
            let n = (next() % 70) as usize;
            let mut pick = || {
                let r = next();
                if r % 5 == 0 {
                    special[(r / 5) as usize % special.len()]
                } else {
                    (r % 10_000) as f64 / 10_000.0
                }
            };
            let pts: Vec<(f64, f64)> = (0..n).map(|_| (pick(), pick())).collect();
            let (points, _) = curve_points(&pts);
            if points.is_empty() {
                straight += 1;
            }
            let response = PressureResponse::new((next() % 101) as f64 / 100.0, points);
            assert!(response.is_ok(), "round {round}: {pts:?}");
        }
        assert!(straight > 0, "点が足りない・有限でない場合も通る");
    }

    #[test]
    fn material_keys_ignore_places_and_extensions() {
        assert_eq!(key("C:\\Mats\\Tip_One.PNG"), "tip_one");
        assert_eq!(key("cat/uuid-1234"), "uuid-1234");
        assert_eq!(key("noext"), "noext");
        assert_eq!(key("a.b.c"), "a.b");
        assert_eq!(key(".hidden"), ".hidden");
        assert!(same_material("x/tip_one.png", "TIP_ONE"));
        assert!(!same_material("tip_one", "tip_two"));
        // 長い印（8 文字以上）は、片方がもう片方を含めば同じ素材とみなす。場所の途中の名前では当てない
        assert!(same_material("{12345678-aaaa}", "x/{12345678-aaaa}-v2.png"));
        assert!(!same_material(
            "c:/users/me/tip_a.png",
            "c:/users/me/tip_b.png"
        ));
    }

    fn item(names: &[&str]) -> blob::RefItem {
        blob::RefItem {
            names: names.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn material(order: i64, texts: &[&str]) -> Material {
        Material {
            order,
            texts: texts.iter().map(|s| s.to_string()).collect(),
            image: None,
            proprietary: false,
            kind: None,
        }
    }

    fn refs_of(items: Vec<blob::RefItem>) -> Refs {
        Refs {
            items,
            complete: true,
        }
    }

    #[test]
    fn references_match_by_name_and_the_order_is_used_only_when_the_counts_agree() {
        let materials = [
            material(1, &["{aaaa-1111-bbbb}"]),
            material(2, &["C:\\x\\tip_two.png"]),
        ];
        let refs = refs_of(vec![
            item(&["", "cat/tip_two", "x"]),
            item(&["", "{AAAA-1111-BBBB}", ""]),
        ]);
        assert_eq!(
            resolve(&refs, &materials, false).picks,
            [Some((1, false)), Some((0, false))],
            "名前で当たる（順序は参照の順）"
        );
        // 当たらない: 並びで決めてよく、残りの素材と参照がちょうど同数のときだけ先頭から（推定）
        let one = [material(1, &["zzz-only"])];
        let unknown = refs_of(vec![item(&["nothing-known"])]);
        assert_eq!(resolve(&unknown, &one, true).picks, [Some((0, true))]);
        assert_eq!(resolve(&unknown, &one, false).picks, [None]);
        let incomplete = Refs {
            items: vec![item(&["nothing-known"])],
            complete: false,
        };
        assert_eq!(resolve(&incomplete, &one, true).picks, [None]);
        // 素材が参照より多い（ほかの設定の素材が混ざりうる）: 先頭の素材を当てずっぽうで使わない
        assert_eq!(resolve(&unknown, &materials, true).picks, [None]);
        // 一部だけ当たるとき、当たらなかった参照は、当たった素材を除いた素材を並びで当てる（残りと同数のとき）
        let mixed = refs_of(vec![item(&["nothing-known"]), item(&["x/tip_two"])]);
        assert_eq!(
            resolve(&mixed, &materials, true).picks,
            [Some((0, true)), Some((1, false))]
        );
        let mixed = refs_of(vec![item(&["x/tip_two"]), item(&["nothing-known"])]);
        assert_eq!(
            resolve(&mixed, &materials, true).picks,
            [Some((1, false)), Some((0, true))],
            "当たった素材は 1 つ目、残りの 0 番を 2 つ目に"
        );
        assert_eq!(
            resolve(&mixed, &materials, false).picks,
            [Some((1, false)), None],
            "並びで決めてはいけないときは当たった分だけ。当たらなかった参照は詰めずに空のまま"
        );
        // 同じ素材を指す参照が重なると、残りの素材が参照より多くなる: 決めない
        let twice = refs_of(vec![
            item(&["x/tip_two"]),
            item(&["x/tip_two"]),
            item(&["nothing-known"]),
        ]);
        let three = [
            material(1, &["C:\\x\\tip_two.png"]),
            material(2, &["m-one"]),
            material(3, &["m-two"]),
        ];
        assert_eq!(
            resolve(&twice, &three, true).picks,
            [Some((0, false)), Some((0, false)), None]
        );
        // 一般的な名前・短い名前では当てない
        let generic = refs_of(vec![item(&["thumbnail", "ab"])]);
        assert_eq!(
            resolve(&generic, &[material(1, &["thumbnail", "ab"])], false).picks,
            [None]
        );
        // 素材が足りない
        let two = refs_of(vec![item(&["zzz1"]), item(&["zzz2"])]);
        assert_eq!(resolve(&two, &materials[..1], true).picks, [None, None]);
    }
}
