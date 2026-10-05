//! CLIP STUDIO PAINT のサブツール `.sut`（SQLite）。
//!
//! 形式は公開されていないので、公開の解析（出どころと確かな所・推測の所は docs/BRUSH_IMPORT.md）で分かる範囲だけを読む:
//! - ブラシの名前と、現在の設定・既定の設定の `Variant` の番号は `Node` から。設定は `Variant` の 1 行（1 列が 1 つの設定。列の
//!   集合は CLIP STUDIO の版で違うので、決まった名前の列を「在れば」読む）。
//! - 筆先・質感の画像は `MaterialFile.FileData`（無圧縮の tar の中のプレビューの PNG）。どの素材を使うかは `Variant` の参照の
//!   BLOB の名前から決める。名前で当たらない参照は、残った素材と参照の数がちょうど合うときだけ並びで当て、そのときは推定として
//!   [`SutNote`] で知らせる。決められない・取り出せない筆先は使わず、欠けたことを知らせる（黙って別の画像を使わない）。
//! - 影響元設定（`*Effector`）は筆圧だけ（最小値と曲線）。傾き・速さ・ランダムは表せないので注記する。
//! - 表せない設定（色の混ぜ・吹き付け・デュアルブラシ・入り抜き・手ぶれ補正・色の変化・合成モード・筆先の向きなど）は、旗が立って
//!   いるものだけ [`SutNote`] で知らせる。列の名前と単位は公開の解析からの推定を含むので、本物のファイルでの確かめは別に要る。
//!
//! 信頼できないファイルなので、データベースを開く・読む所は `db`（読み取り専用・メモリ上・上限つき）だけが触る。

mod blob;
mod db;
mod material;
mod tar;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use yolu_core::curve::{Curve, CurvePoint};
use yolu_core::{Brush, BrushTip, PaperTexture, PressureResponse, TipSelection};

use super::error::{BrushImportError, Fault};
use super::notes::{Source, SutInput, SutNote, SutTarget, Unrepresented};
use super::png_tip;
use super::reader::Budget;
use super::{short_text, ImportedBrush, ImportedSet, SkipReason, SkippedBrush};
use blob::{parse_effector, parse_refs, Effector, Refs};
use db::{Database, Material, Row, Variants};

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
const START_END: &[&str] = &["brushusein", "brushuseout"];
const STABILIZER: &[&str] = &["brushuserevision"];
const COLOR_CHANGE: &[&str] = &[
    "brushhuechange",
    "brushsaturationchange",
    "brushvaluechange",
    "brushsubcolor",
];
const BLEND_MODE: &[&str] = &["compositemode"];

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
    START_END,
    STABILIZER,
    COLOR_CHANGE,
    BLEND_MODE,
];

/// 割合の設定（0〜100 の百分率か 0〜1 の割合か列によって混ざるので、1 より大きければ百分率として見る）。
fn ratio(value: f64) -> f64 {
    if value > 1.0 {
        value / 100.0
    } else {
        value
    }
}

fn fraction(value: f64) -> f64 {
    ratio(value).clamp(0.0, 1.0)
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

/// 影響元の列 1 つを読んで、筆圧は応えに、ほかの入力は注記にする。
fn effector(
    brush: &mut Brush,
    row: &Row,
    target: SutTarget,
    columns: &[&str],
    notes: &mut Vec<Unrepresented>,
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
        if e.flags & flag != 0 {
            notes.push(Unrepresented::ClipStudio(SutNote::Influence {
                target,
                input,
            }));
        }
    }
    if e.flags & Effector::PRESSURE == 0 {
        return;
    }
    let (points, simplified) = e
        .curves
        .first()
        .map_or((Vec::new(), false), |c| curve_points(c));
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
        SutTarget::Thickness => notes.push(Unrepresented::ClipStudio(SutNote::ThicknessPressure)),
    }
}

// ---------------- 取り込み ----------------

/// 1 回の取り込みの間、素材と、その画像を作った結果を持つ。
struct Library<'a> {
    db: &'a Database,
    materials: Option<(Vec<Material>, bool)>,
    png_budget: u64,
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
            tips: HashMap::new(),
            textures: HashMap::new(),
            unreadable: HashSet::new(),
        }
    }

    /// 素材（`MaterialFile` の行の順）と、素材が上限を超えて読み切れていないか。
    fn materials(&mut self) -> Result<(&[Material], bool), Fault> {
        if self.materials.is_none() {
            self.materials = Some(self.db.materials(&mut self.png_budget)?);
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
        Some(refs) if !refs.items.is_empty() => (
            resolve(refs, library.materials()?.0, order_ok).picks,
            refs.complete,
        ),
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
        resolve(&refs, materials, !uses_pattern && !capped)
            .picks
            .first()
            .copied()
            .flatten()
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
    let depth = row.first_number(TEXTURE_DENSITY).map_or(1.0, fraction);
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

/// `Variant` の 1 行から core のブラシを作る。
fn brush_from_row(
    row: &Row,
    name: &str,
    library: &mut Library<'_>,
    budget: &mut Budget,
) -> Result<(Brush, Vec<Unrepresented>), Fault> {
    let mut notes: Vec<Unrepresented> = Vec::new();
    let mut brush = Brush::default();
    brush.base.pressure_size = false;
    brush.base.pressure_opacity = false;
    brush.base.pressure_flow = false;

    let images = tips(row, name, library, budget, &mut notes)?;
    let mut tip_side = None;
    match images.len() {
        0 => {
            if let Some(h) = row.first_number(HARDNESS) {
                brush.base.hardness = fraction(h);
            }
        }
        1 => {
            tip_side = Some(images[0].width().max(images[0].height()) as f64);
            brush.tip.image = Some(images[0].clone());
        }
        n => {
            tip_side = images
                .iter()
                .map(|t| t.width().max(t.height()) as f64)
                .reduce(f64::max);
            brush.tip.images = images;
            brush.tip.selection = TipSelection::Random;
            let _ = n;
            notes.push(Unrepresented::ClipStudio(SutNote::TipOrder));
        }
    }
    // 大きさの設定も筆先の画像も無ければ、core の既定の半径のまま
    if let Some(diameter) = row.first_number(SIZE).filter(|v| *v > 0.0).or(tip_side) {
        brush.base.radius = (diameter.min(2000.0) / 2.0).max(0.5);
    }
    if let Some(v) = row.first_number(OPACITY) {
        brush.base.opacity = fraction(v);
    }
    if let Some(v) = row.first_number(FLOW) {
        brush.base.flow = fraction(v);
    }
    if let Some(v) = row.first_number(INTERVAL) {
        brush.base.spacing = ratio(v).clamp(0.01, 4.0);
    }
    if let Some(v) = row.first_number(THICKNESS) {
        brush.tip.roundness = fraction(v).max(0.01);
    }

    effector(&mut brush, row, SutTarget::Size, SIZE_EFFECTOR, &mut notes);
    effector(
        &mut brush,
        row,
        SutTarget::Opacity,
        OPACITY_EFFECTOR,
        &mut notes,
    );
    effector(&mut brush, row, SutTarget::Flow, FLOW_EFFECTOR, &mut notes);
    effector(
        &mut brush,
        row,
        SutTarget::Thickness,
        THICKNESS_EFFECTOR,
        &mut notes,
    );

    texture(row, name, library, budget, &mut brush, &mut notes)?;

    // 旗が立っている表せない設定
    if row.first_number(ROTATION).is_some_and(|v| v != 0.0)
        && (brush.tip.image.is_some() || !brush.tip.images.is_empty() || brush.tip.roundness < 1.0)
    {
        notes.push(Unrepresented::ClipStudio(SutNote::Direction));
    }
    // 色の混ぜ: 絵の具量（BrushMixColor）・絵の具濃度（BrushMixAlpha）・色延び（BrushMixColorExtension）を、0〜100 から 0〜1 にして
    // 「絵の具で混ぜる」へ写す（CLIP STUDIO の絵の具量 100 は下の色を拾わない＝こちらの量 1 と同じ向き）。値が無く混色の旗だけが
    // 立っているものは、写せる値が無いので知らせる。
    let (paint, density, stretch) = (
        row.first_number(MIX_PAINT).unwrap_or(0.0),
        row.first_number(MIX_DENSITY).unwrap_or(0.0),
        row.first_number(MIX_STRETCH).unwrap_or(0.0),
    );
    if paint > 0.0 || density > 0.0 || stretch > 0.0 {
        let unit = |v: f64| (v / 100.0).clamp(0.0, 1.0);
        brush.mix.mode = yolu_core::brush::MixMode::Mix;
        brush.mix.paint = unit(paint);
        brush.mix.density = unit(density);
        brush.mix.stretch = unit(stretch);
    } else if row.on(WATER_COLOR) {
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
        (START_END, SutNote::StartEnd),
        (STABILIZER, SutNote::Stabilizer),
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
    Ok((brush, notes))
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
        let (brush, notes) = match &mut variants {
            None => (
                Brush::default(),
                vec![Unrepresented::ClipStudio(SutNote::SettingsMissing)],
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
        set.brushes.push(ImportedBrush::new(
            &name,
            Source::ClipStudioSut,
            brush,
            notes,
        )?);
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
    fn ratios_are_read_as_percent_when_above_one() {
        assert_eq!(fraction(100.0), 1.0);
        assert_eq!(fraction(25.0), 0.25);
        assert_eq!(fraction(0.5), 0.5);
        assert_eq!(fraction(1.0), 1.0);
        assert_eq!(fraction(-3.0), 0.0);
        assert_eq!(fraction(400.0), 1.0);
        assert_eq!(ratio(400.0), 4.0);
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
