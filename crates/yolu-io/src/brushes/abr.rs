//! Photoshop のブラシ `.abr`: 版 1・2 の計算で描くブラシと画像の筆先、版 6 以降の筆先（`samp`）とプリセット（`desc`）。
//!
//! 公開された形式の説明（版 1 は Photoshop 6.0 File Formats Specification、記述子は Photoshop File Formats Specification の
//! 「Descriptor structure」、新しい並びはコミュニティの解説）から書いた。このエンジンに無い機能は、ブラシごとの
//! `Unrepresented` に載せる（黙って捨てない）。プリセットの設定は core のブラシで表せるものを写し、表せないものを知らせる。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use yolu_core::{
    brush::MAX_FADE, Brush, BrushSettings, BrushTip, DualBrush, DualBrushMode, PaperTexture,
    PressureResponse, TextureMode,
};

use super::descriptor::{self, Object, Value};
use super::error::{BrushImportError, Counted, Fault, Result, SizedItem, SkippedPattern};
use super::notes::{AbrKind, ControlKind, DualNote, Setting, Source, TextureNote, Unrepresented};
use super::pattern::{self, Pattern};
use super::reader::{decode_packbits_row, Budget, Reader};
use super::{printable, short_text, ImportedBrush, ImportedSet, SkipReason, SkippedBrush};

/// 版 6 以降の筆先（`samp`）の個数の上限（版 1・2 と `.pat` も 10,000）。1×1 の筆先は画素の予算では止まらず、使われない筆先は 1 つごとに
/// ブラシ 1 つになるので、個数でも断る。
const MAX_TIPS: usize = 10_000;

/// ファイル全体の注記に並べる、知らない節の種類の数の上限。超えた分は節の数だけを 1 つの注記で知らせる。
const MAX_SECTION_NOTES: usize = 64;

pub(crate) fn read_abr(
    data: &[u8],
    fallback: Option<&str>,
    budget: &mut Budget,
) -> std::result::Result<ImportedSet, BrushImportError> {
    let mut r = Reader::new(data);
    let version = r.i16().map_err(BrushImportError::from)?;
    match version {
        1 | 2 => read_old(&mut r, version, fallback, budget),
        6..=10 => read_new(&mut r, version, fallback, budget),
        other => Err(Fault::AbrVersion(other).into()),
    }
}

fn clamp_i(value: i16, min: i32, max: i32) -> i32 {
    (value as i32).max(min).min(max)
}

// ---------------- 版 1・2 ----------------

fn read_old(
    r: &mut Reader,
    version: i16,
    fallback: Option<&str>,
    budget: &mut Budget,
) -> std::result::Result<ImportedSet, BrushImportError> {
    let count = r.i16()?;
    if !(0..=10_000).contains(&count) {
        return Err(Fault::AbrBrushCount(count).into());
    }
    let mut set = ImportedSet::default();
    for i in 0..count as usize {
        let kind = r.i16()?;
        let size = r.i32()?;
        let start = r.position();
        if size < 0 || size as usize > r.remaining() {
            return Err(Fault::AbrBrushTruncated { index: i + 1 }.into());
        }
        let label = format!("{} {}", fallback.unwrap_or("Brush"), i + 1);
        match kind {
            1 => {
                r.skip(4)?;
                let (spacing, diameter, roundness, angle, hardness) =
                    (r.i16()?, r.i16()?, r.i16()?, r.i16()?, r.i16()?);
                let mut b = Brush {
                    base: BrushSettings {
                        radius: clamp_i(diameter, 1, 2000) as f64 / 2.0,
                        spacing: clamp_i(spacing, 1, 400) as f64 / 100.0,
                        hardness: clamp_i(hardness, 0, 100) as f64 / 100.0,
                        pressure_opacity: false,
                        ..BrushSettings::default()
                    },
                    ..Brush::default()
                };
                b.tip.roundness = (clamp_i(roundness, 1, 100) as f64 / 100.0).max(0.01);
                b.tip.angle = clamp_i(angle, -180, 180) as f64;
                set.brushes.push(ImportedBrush::new(
                    &label,
                    Source::PhotoshopAbr {
                        version,
                        kind: AbrKind::Computed,
                    },
                    b,
                    Vec::new(),
                )?);
            }
            2 => {
                r.skip(4)?;
                let spacing = r.i16()?;
                let mut name = label.clone();
                if version == 2 {
                    let n = descriptor::unicode_string(r)?;
                    let n = short_text(&n, 128);
                    if !n.is_empty() {
                        name = n;
                    }
                }
                // アンチエイリアスの旗（画像の筆先の画像のなめらかさ）。このアプリのアンチエイリアス（丸い縁の帯と小さなダブの濃さ）とは
                // 意味が違い、写すと取り込んだ筆先の見た目が変わるので読まない（なし のまま）
                r.skip(1)?;
                r.skip(8)?; // 16 bit の範囲（32 bit の範囲が続く）
                let (top, left, bottom, right) = (r.i32()?, r.i32()?, r.i32()?, r.i32()?);
                let depth = r.i16()?;
                let (tip, notes) = read_bitmap(r, &name, top, left, bottom, right, depth, budget)?;
                let mut b = Brush {
                    base: BrushSettings {
                        radius: tip.width().max(tip.height()) as f64 / 2.0,
                        spacing: clamp_i(spacing, 1, 400) as f64 / 100.0,
                        pressure_opacity: false,
                        ..BrushSettings::default()
                    },
                    ..Brush::default()
                };
                b.tip.image = Some(Arc::new(tip));
                set.brushes.push(ImportedBrush::new(
                    &name,
                    Source::PhotoshopAbr {
                        version,
                        kind: AbrKind::Sampled,
                    },
                    b,
                    notes,
                )?);
            }
            other => {
                return Err(Fault::AbrBrushType {
                    index: i + 1,
                    kind: other,
                }
                .into())
            }
        }
        r.set_position(start + size as usize);
    }
    Ok(set)
}

// ---------------- 版 6 以降 ----------------

struct Sample {
    id: String,
    tip: Arc<BrushTip>,
    notes: Vec<Unrepresented>,
}

struct Textures {
    patterns: Vec<Pattern>,
    failures: Vec<SkippedPattern>,
}

fn read_new(
    r: &mut Reader,
    version: i16,
    fallback: Option<&str>,
    budget: &mut Budget,
) -> std::result::Result<ImportedSet, BrushImportError> {
    let subversion = r.i16()?;
    if subversion != 1 && subversion != 2 {
        return Err(Fault::AbrSubversion(subversion).into());
    }
    let mut samples: Vec<Sample> = Vec::new();
    let mut presets: Option<Object> = None;
    let (mut desc_error, mut pattern_error): (Option<Fault>, Option<Fault>) = (None, None);
    let mut textures = Textures {
        patterns: Vec::new(),
        failures: Vec::new(),
    };
    // 知らない節: 種類ごとに 1 回だけ（上限つき）。種類の重複を線形に探さない（節の数は 2 千万を超えうる）。
    let mut section_keys: Vec<String> = Vec::new();
    let mut seen_keys: HashSet<String> = HashSet::new();
    let mut unlisted_sections = 0usize;
    while r.remaining() >= 12 {
        let signature = r.ascii(4)?;
        let key = r.ascii(4)?;
        if signature != "8BIM" {
            return Err(Fault::AbrSection {
                offset: r.position() - 8,
            }
            .into());
        }
        let length = r.counted(Counted::SectionLength)?;
        let start = r.position();
        match key.as_str() {
            "samp" => {
                let mut section = Reader::new(r.bytes(length)?);
                read_samples(&mut section, subversion, &mut samples, fallback, budget)?;
            }
            "desc" => {
                let mut d = Reader::new(r.bytes(length)?);
                // 先端だけでも取り込めるようにする
                match d.u32().and_then(|_| descriptor::read_descriptor(&mut d)) {
                    Ok(o) => presets = Some(o),
                    Err(fault) => desc_error = Some(fault),
                }
            }
            "patt" => {
                let mut section = Reader::new(r.bytes(length)?);
                let mut read = Vec::new();
                // 模様が読めなくても筆先と設定は取り込む
                match pattern::read_framed(&mut section, budget, &mut read, &mut textures.failures)
                {
                    Ok(()) => textures.patterns.extend(read),
                    Err(fault) => pattern_error = Some(fault),
                }
            }
            _ => {
                let key = printable(&key, 32);
                if seen_keys.contains(&key) {
                    // 既に並べた種類
                } else if section_keys.len() < MAX_SECTION_NOTES {
                    seen_keys.insert(key.clone());
                    section_keys.push(key);
                } else {
                    unlisted_sections += 1;
                }
                r.skip(length)?;
            }
        }
        r.set_position(start + length);
        let pad = (4 - length % 4) % 4;
        if pad > 0 && r.remaining() >= pad {
            r.skip(pad)?;
        }
    }
    // ファイル全体の注記はセットに 1 回だけ持つ（ブラシごとに複製すると、ブラシの数との積で増える）。
    let mut set = ImportedSet::default();
    if let Some(fault) = desc_error {
        set.notes.push(Unrepresented::PresetsUnreadable(fault));
    }
    if let Some(fault) = pattern_error {
        set.notes.push(Unrepresented::PatternsUnreadable(fault));
    }
    set.notes
        .extend(section_keys.into_iter().map(Unrepresented::SectionSkipped));
    if unlisted_sections > 0 {
        set.notes
            .push(Unrepresented::MoreSectionsSkipped(unlisted_sections));
    }

    let mut used: HashSet<String> = HashSet::new();
    // 質感の「反転」は模様ごとに 1 回だけ作って、プリセットの間で共有する（プリセットごとに画素を複製しない）
    let mut inverted: HashMap<usize, Arc<BrushTip>> = HashMap::new();
    if let Some(list) = presets.as_ref().and_then(|p| p.list("Brsh")) {
        for item in list {
            if let Value::Object(o) = item {
                match preset(
                    o,
                    &samples,
                    &textures,
                    version,
                    &mut used,
                    &mut inverted,
                    budget,
                )? {
                    Preset::Brush(brush) => set.brushes.push(*brush),
                    Preset::Skipped(skipped) => set.skipped.push(skipped),
                }
            }
        }
    }
    // どのプリセットも使わない筆先（使える desc が無いファイルも）は、既定の設定のブラシにする。
    for sample in samples.iter().filter(|s| !used.contains(&s.id)) {
        let mut b = Brush {
            base: BrushSettings {
                radius: sample.tip.width().max(sample.tip.height()) as f64 / 2.0,
                spacing: 0.25,
                pressure_opacity: false,
                ..BrushSettings::default()
            },
            ..Brush::default()
        };
        b.tip.image = Some(sample.tip.clone());
        set.brushes.push(ImportedBrush::new(
            sample.tip.name(),
            Source::PhotoshopAbr {
                version,
                kind: AbrKind::Tip,
            },
            b,
            sample.notes.clone(),
        )?);
    }
    if set.brushes.is_empty() {
        return Err(Fault::AbrNoBrushes.into());
    }
    Ok(set)
}

fn read_samples(
    r: &mut Reader,
    subversion: i16,
    samples: &mut Vec<Sample>,
    fallback: Option<&str>,
    budget: &mut Budget,
) -> Result<()> {
    while r.remaining() >= 4 {
        if samples.len() >= MAX_TIPS {
            return Err(Fault::AbrTipCount(MAX_TIPS));
        }
        let length = r.counted(Counted::SampleLength)?;
        let start = r.position();
        let id_length = r.u8()? as usize;
        let id = r.ascii(id_length)?;
        r.skip(if subversion == 1 { 10 } else { 264 })?; // 内容は未文書化（固定長であることは実装どうしで一致している）
        let (top, left, bottom, right) = (r.i32()?, r.i32()?, r.i32()?, r.i32()?);
        let depth = r.i16()?;
        let name = format!("{} {}", fallback.unwrap_or("Tip"), samples.len() + 1);
        let (tip, notes) = read_bitmap(r, &name, top, left, bottom, right, depth, budget)?;
        samples.push(Sample {
            id,
            tip: Arc::new(tip),
            notes,
        });
        let mut next = start + length;
        next += (4 - next % 4) % 4;
        if next > r.len() {
            break;
        }
        r.set_position(next);
    }
    Ok(())
}

/// 画像の筆先: 圧縮の 1 バイト、続いて生の行か、行ごとに 16 bit の長さを持つ PackBits の行。行は上から。
#[allow(clippy::too_many_arguments)]
fn read_bitmap(
    r: &mut Reader,
    name: &str,
    top: i32,
    left: i32,
    bottom: i32,
    right: i32,
    depth: i16,
    budget: &mut Budget,
) -> Result<(BrushTip, Vec<Unrepresented>)> {
    let (width, height) = (right as i64 - left as i64, bottom as i64 - top as i64);
    let side = BrushTip::MAX_SIZE as i64;
    if width < 1 || height < 1 || width > side || height > side {
        return Err(Fault::SizeOutOfRange {
            what: SizedItem::Tip,
            width,
            height,
        });
    }
    if depth != 8 && depth != 16 {
        return Err(Fault::TipDepth(depth));
    }
    let compression = r.u8()?;
    let (w, h) = (width as usize, height as usize);
    let bytes_per_pixel = depth as usize / 8;
    let row_bytes = w * bytes_per_pixel;
    if compression > 1 {
        return Err(Fault::TipCompression(compression));
    }
    if compression == 1 && depth == 16 {
        return Err(Fault::Tip16BitCompressed);
    }
    budget.take(w as u64 * h as u64)?;
    let mut alpha = vec![0u8; w * h];
    if compression == 0 {
        for y in 0..h {
            let row = r.bytes(row_bytes)?;
            fill_row(&mut alpha, y, w, h, row, bytes_per_pixel);
        }
    } else {
        let mut lengths = Vec::new();
        for _ in 0..h {
            lengths.push(r.u16()? as usize);
        }
        for (y, length) in lengths.into_iter().enumerate() {
            let row = decode_packbits_row(r.bytes(length)?, row_bytes)?;
            fill_row(&mut alpha, y, w, h, &row, bytes_per_pixel);
        }
    }
    let mut notes = Vec::new();
    if depth == 16 {
        notes.push(Unrepresented::Tip16Bit);
    }
    let tip =
        BrushTip::new(name, w as u32, h as u32, alpha).map_err(|_| Fault::SizeOutOfRange {
            what: SizedItem::Tip,
            width,
            height,
        })?;
    Ok((tip, notes))
}

/// 上から y 行目を、左下原点の筆先の行へ（上位バイトを使う）。
fn fill_row(alpha: &mut [u8], y: usize, w: usize, h: usize, row: &[u8], bytes_per_pixel: usize) {
    let dst = &mut alpha[(h - 1 - y) * w..(h - y) * w];
    for (x, d) in dst.iter_mut().enumerate() {
        *d = row[x * bytes_per_pixel];
    }
}

// ---------------- プリセットの対応づけ ----------------

enum Preset {
    Brush(Box<ImportedBrush>),
    Skipped(SkippedBrush),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    None,
    Size,
    Opacity,
    Flow,
    Angle,
}

fn preset(
    preset: &Object,
    samples: &[Sample],
    textures: &Textures,
    version: i16,
    used: &mut HashSet<String>,
    inverted: &mut HashMap<usize, Arc<BrushTip>>,
    budget: &mut Budget,
) -> std::result::Result<Preset, BrushImportError> {
    let mut notes: Vec<Unrepresented> = Vec::new();
    let name = {
        let n = short_text(preset.text("Nm  ").unwrap_or("Brush"), 128);
        if n.is_empty() {
            "Brush".to_string()
        } else {
            n
        }
    };
    let Some(shape) = preset.object("Brsh") else {
        return Ok(Preset::Skipped(SkippedBrush {
            name,
            reason: SkipReason::NoTipShape,
        }));
    };
    let mut b = Brush::default();
    b.base.pressure_opacity = false;
    b.base.pressure_size = false;
    let (diameter, angle, roundness, spacing, hardness) = (
        shape.number("Dmtr"),
        shape.number("Angl"),
        shape.number("Rndn"),
        shape.number("Spcn"),
        shape.number("Hrdn"),
    );
    let mut tip_size = None;
    if shape.class_id == "sampledBrush" {
        let Some(sample) = shape
            .text("sampledData")
            .and_then(|id| samples.iter().find(|s| s.id == id))
        else {
            return Ok(Preset::Skipped(SkippedBrush {
                name,
                reason: SkipReason::TipNotInFile,
            }));
        };
        used.insert(sample.id.clone());
        notes.extend(sample.notes.iter().cloned());
        tip_size = Some(sample.tip.width().max(sample.tip.height()) as f64);
        b.tip.image = Some(sample.tip.clone());
    } else if shape.class_id == "computedBrush" {
        b.base.hardness = clamp01(hardness.unwrap_or(100.0), 100.0);
    } else {
        notes.push(Unrepresented::UnknownTipKind(short_text(
            &shape.class_id,
            32,
        )));
    }
    b.base.radius = (diameter.unwrap_or(tip_size.unwrap_or(20.0)).min(2000.0) / 2.0).max(0.5);
    b.tip.angle = angle.unwrap_or(0.0).clamp(-180.0, 180.0);
    b.tip.roundness = clamp01(roundness.unwrap_or(100.0), 100.0).max(0.01);
    b.base.spacing = (spacing.unwrap_or(25.0) / 100.0).clamp(0.01, 4.0);
    b.tip.flip_x = shape.bool("flipX") == Some(true);
    b.tip.flip_y = shape.bool("flipY") == Some(true);

    if preset.bool("useTipDynamics") == Some(true) {
        let size = preset.object("szVr");
        b.jitter.size = jitter(size);
        b.base.pressure_size = false;
        control(size, Setting::Size, &mut notes, &mut b, Target::Size);
        if let Some(min) = preset.number("minimumDiameter") {
            if min > 0.0 {
                if b.base.pressure_size {
                    // 筆圧で大きさを変えるなら、最小の直径は筆圧 0 のときの大きさの割合（Photoshop と同じ式: 最小 + (1 − 最小) × 筆圧）
                    if let Ok(r) = PressureResponse::new(clamp01(min, 100.0), Vec::new()) {
                        b.pressure.size = r;
                    }
                } else {
                    // フェード・傾きなどの最小は表せない（効いていない設定の最小も知らせる）
                    notes.push(Unrepresented::MinimumDiameter(min));
                }
            }
        }
        let angle_dynamics = preset.object("angleDynamics");
        b.jitter.angle = jitter(angle_dynamics);
        control(
            angle_dynamics,
            Setting::Angle,
            &mut notes,
            &mut b,
            Target::Angle,
        );
        let roundness_dynamics = preset.object("roundnessDynamics");
        b.jitter.roundness = jitter(roundness_dynamics);
        control(
            roundness_dynamics,
            Setting::Roundness,
            &mut notes,
            &mut b,
            Target::None,
        );
    }
    if preset.bool("useScatter") == Some(true) {
        let scatter = preset.object("scatterDynamics");
        b.jitter.scatter =
            (scatter.and_then(|s| s.number("jitter")).unwrap_or(0.0) / 100.0).clamp(0.0, 10.0);
        control(scatter, Setting::Scatter, &mut notes, &mut b, Target::None);
        b.jitter.count = preset.number("Cnt ").unwrap_or(1.0).clamp(1.0, 16.0) as u32;
        if preset.bool("bothAxes") != Some(true) && b.jitter.scatter > 0.0 {
            notes.push(Unrepresented::ScatterOneAxis);
        }
        if jitter(preset.object("countDynamics")) > 0.0 {
            notes.push(Unrepresented::CountJitter);
        }
    }
    if preset.bool("usePaintDynamics") == Some(true) {
        let opacity = preset.object("opVr");
        let flow = preset.object("prVr");
        b.jitter.opacity = jitter(opacity);
        b.base.pressure_opacity = false;
        control(
            opacity,
            Setting::Opacity,
            &mut notes,
            &mut b,
            Target::Opacity,
        );
        b.jitter.flow = jitter(flow);
        b.base.pressure_flow = false;
        control(flow, Setting::Flow, &mut notes, &mut b, Target::Flow);
    }
    if let Some(options) = preset.object("toolOptions") {
        if let Some(op) = options.number("Opct") {
            b.base.opacity = clamp01(op, 100.0);
        }
        if let Some(fl) = options.number("flow") {
            b.base.flow = clamp01(fl, 100.0);
        }
    }
    if preset.bool("useColorDynamics") == Some(true) {
        color_dynamics(preset, &mut b, &mut notes);
    }
    let dual_block = preset.object("dualBrush");
    if preset.bool("useDualBrush") == Some(true)
        || dual_block.and_then(|d| d.bool("useDualBrush")) == Some(true)
    {
        dual(dual_block, samples, &mut b, &mut notes, used);
    }
    if preset.bool("useTexture") == Some(true) {
        texture(preset, textures, inverted, budget, &mut b, &mut notes)?;
    }
    if preset.bool("usePaintDynamics") == Some(true) && mixer_dynamics(preset) {
        notes.push(Unrepresented::MixerBrush);
    }
    if preset.bool("Nose") == Some(true) {
        notes.push(Unrepresented::Noise);
    }
    if preset.bool("Wtdg") == Some(true) {
        notes.push(Unrepresented::WetEdges);
    }
    Ok(Preset::Brush(Box::new(ImportedBrush::new(
        &name,
        Source::PhotoshopAbr {
            version,
            kind: AbrKind::Preset,
        },
        b,
        notes,
    )?)))
}

/// 混合ブラシのウェット・混合のゆらぎ（`wetnessControl`・`mixControl`。不透明度・流量のゆらぎと同じ形の記述）が、ゆらぎかコントロールを
/// 持つか。基本の値（ウェット・負荷・混合）はブラシの記述に入らないので、あるのはゆらぎだけ。キー名は実物の ABR では確かめていない
/// （試験は同じ想定のキーで作った合成のデータ）。
fn mixer_dynamics(preset: &Object) -> bool {
    ["wetnessControl", "mixControl"].into_iter().any(|key| {
        let d = preset.object(key);
        jitter(d) > 0.0 || d.and_then(|d| d.number("bVTy")).unwrap_or(0.0) as i32 != 0
    })
}

/// カラーダイナミクス: 描画色/背景色のゆらぎ（clVr）、色相（H）・彩度（Strt）・明るさ（Brgh）、純度。背景色はブラシに入っていないので、
/// 画面の副色を使う（`ColorDynamics::secondary` は取り込みでは触らない）。
fn color_dynamics(preset: &Object, b: &mut Brush, notes: &mut Vec<Unrepresented>) {
    let fb = preset.object("clVr");
    b.color.foreground_background = jitter(fb);
    let control_code = fb.and_then(|f| f.number("bVTy")).unwrap_or(0.0) as i32;
    if control_code != 0 {
        notes.push(Unrepresented::ForegroundBackgroundControl(
            ControlKind::from_code(control_code),
        ));
    }
    b.color.hue = clamp01(preset.number("H   ").unwrap_or(0.0), 100.0);
    b.color.saturation = clamp01(preset.number("Strt").unwrap_or(0.0), 100.0);
    b.color.brightness = clamp01(preset.number("Brgh").unwrap_or(0.0), 100.0);
    b.color.purity = (preset.number("purity").unwrap_or(0.0) / 100.0).clamp(-1.0, 1.0);
    if preset.bool("colorDynamicsPerTip") == Some(false) {
        b.color.per_tip = false;
    }
}

/// デュアルブラシ: 2 つ目の筆先（Brsh: 計算か画像。直径・硬さ・角度・真円率・間隔）、描画モード（BlnM）、散布（scatterDynamics の
/// jitter）、数（Cnt）。
fn dual(
    block: Option<&Object>,
    samples: &[Sample],
    b: &mut Brush,
    notes: &mut Vec<Unrepresented>,
    used: &mut HashSet<String>,
) {
    let Some(block) = block else {
        notes.push(Unrepresented::Dual(DualNote::MissingTip));
        return;
    };
    let Some(shape) = block.object("Brsh") else {
        notes.push(Unrepresented::Dual(DualNote::MissingTip));
        return;
    };
    let mut d = DualBrush::default();
    let mut tip_size = None;
    if shape.class_id == "sampledBrush" {
        let Some(sample) = shape
            .text("sampledData")
            .and_then(|id| samples.iter().find(|s| s.id == id))
        else {
            notes.push(Unrepresented::Dual(DualNote::TipNotInFile));
            return;
        };
        used.insert(sample.id.clone());
        tip_size = Some(sample.tip.width().max(sample.tip.height()) as f64);
        d.tip = Some(sample.tip.clone());
    } else if shape.class_id == "computedBrush" {
        d.hardness = clamp01(shape.number("Hrdn").unwrap_or(100.0), 100.0);
    } else {
        notes.push(Unrepresented::Dual(DualNote::UnknownTipKind(short_text(
            &shape.class_id,
            32,
        ))));
    }
    d.radius = (shape
        .number("Dmtr")
        .unwrap_or(tip_size.unwrap_or(20.0))
        .min(2000.0)
        / 2.0)
        .max(0.5);
    d.angle = shape.number("Angl").unwrap_or(0.0).clamp(-180.0, 180.0);
    d.roundness = clamp01(shape.number("Rndn").unwrap_or(100.0), 100.0).max(0.01);
    d.spacing = (shape
        .number("Spcn")
        .or(block.number("Spcn"))
        .unwrap_or(25.0)
        / 100.0)
        .clamp(0.01, 4.0);
    d.mode = match block.enum_value("BlnM") {
        None | Some("Mltp") => DualBrushMode::Multiply,
        Some("Drkn") => DualBrushMode::Darken,
        Some("Ovrl") => DualBrushMode::Overlay,
        Some("CDdg") => DualBrushMode::ColorDodge,
        Some("CBrn") => DualBrushMode::ColorBurn,
        Some("linearBurn") => DualBrushMode::LinearBurn,
        Some("hardMix") => DualBrushMode::HardMix,
        Some("blendSubtraction") | Some("Sbtr") => DualBrushMode::Subtract,
        Some(other) => {
            notes.push(Unrepresented::Dual(DualNote::Mode(short_text(other, 32))));
            DualBrushMode::Multiply
        }
    };
    let scatter = block.object("scatterDynamics");
    d.scatter = (scatter.and_then(|s| s.number("jitter")).unwrap_or(0.0) / 100.0).clamp(0.0, 10.0);
    control(scatter, Setting::DualScatter, notes, b, Target::None);
    d.count = block.number("Cnt ").unwrap_or(1.0).clamp(1.0, 16.0) as u32;
    if block.bool("bothAxes") == Some(false) && d.scatter > 0.0 {
        notes.push(Unrepresented::Dual(DualNote::ScatterOneAxis));
    }
    if jitter(block.object("countDynamics")) > 0.0 {
        notes.push(Unrepresented::Dual(DualNote::CountJitter));
    }
    if block.bool("Flip") == Some(true)
        || shape.bool("flipX") == Some(true)
        || shape.bool("flipY") == Some(true)
    {
        notes.push(Unrepresented::Dual(DualNote::Flip));
    }
    b.dual = Some(d);
}

/// テクスチャ: 模様（Txtr。ID だけで対応づけ、名前では決してしない）、深さ・拡大・反転・合わせ方。質感はストロークの不透明度の天井に
/// 効く紙の質感として使う（Photoshop の「描点ごとにテクスチャを適用」は無し）。
fn texture(
    preset: &Object,
    textures: &Textures,
    inverted: &mut HashMap<usize, Arc<BrushTip>>,
    budget: &mut Budget,
    b: &mut Brush,
    notes: &mut Vec<Unrepresented>,
) -> Result<()> {
    let reference = preset.object("Txtr");
    let id = reference.and_then(|r| r.text("Idnt"));
    let pattern_name = short_text(reference.and_then(|r| r.text("Nm  ")).unwrap_or(""), 128);
    let Some(index) = id.and_then(|id| textures.patterns.iter().position(|p| p.id == id)) else {
        let failure = if pattern_name.is_empty() {
            None
        } else {
            textures.failures.iter().find(|f| f.name == pattern_name)
        };
        notes.push(Unrepresented::Texture(match failure {
            Some(f) => TextureNote::PatternRefused {
                name: pattern_name,
                reason: f.reason.clone(),
            },
            None => TextureNote::PatternMissing { name: pattern_name },
        }));
        return Ok(());
    };
    let pattern = &textures.patterns[index];
    let mut image = pattern.texture.clone();
    if preset.bool("InvT") == Some(true) {
        image = match inverted.get(&index) {
            Some(done) => done.clone(),
            None => {
                // 反転した画素は新しく確保するので、予算から引く（引けなければファイルを断る）
                budget.take(image.width() as u64 * image.height() as u64)?;
                let flipped: Vec<u8> = image.alpha().iter().map(|a| 255 - a).collect();
                let tip = BrushTip::new(image.name(), image.width(), image.height(), flipped)
                    .map_err(|_| Fault::SizeOutOfRange {
                        what: SizedItem::Pattern,
                        width: image.width() as i64,
                        height: image.height() as i64,
                    })?;
                let tip = Arc::new(tip);
                inverted.insert(index, tip.clone());
                tip
            }
        };
    }
    let depth = clamp01(preset.number("textureDepth").unwrap_or(100.0), 100.0);
    let scale = preset.number("textureScale").unwrap_or(100.0) / 100.0;
    let used_scale = scale.clamp(0.05, 64.0);
    if used_scale != scale {
        notes.push(Unrepresented::Texture(TextureNote::ScaleClamped {
            percent: scale * 100.0,
            used_percent: used_scale * 100.0,
        }));
    }
    let mode = match preset.enum_value("textureBlendMode") {
        None | Some("Mltp") => TextureMode::Multiply,
        Some("Sbtr") | Some("blendSubtraction") => TextureMode::Subtract,
        Some("Drkn") => TextureMode::Darken,
        Some("Ovrl") => TextureMode::Overlay,
        Some("CDdg") => TextureMode::ColorDodge,
        Some("CBrn") => TextureMode::ColorBurn,
        Some("linearBurn") => TextureMode::LinearBurn,
        Some("hardMix") => TextureMode::HardMix,
        Some(other) => {
            notes.push(Unrepresented::Texture(TextureNote::Mode(short_text(
                other, 32,
            ))));
            TextureMode::Multiply
        }
    };
    b.texture = Some(PaperTexture {
        image,
        depth,
        scale: used_scale,
        mode,
    });
    if preset.bool("TxtC") == Some(true) {
        notes.push(Unrepresented::Texture(TextureNote::EachTip));
    }
    let depth_dynamics = preset.object("textureDepthDynamics");
    if jitter(depth_dynamics) > 0.0
        || depth_dynamics.and_then(|d| d.number("bVTy")).unwrap_or(0.0) != 0.0
    {
        notes.push(Unrepresented::Texture(TextureNote::DepthDynamics));
    }
    if preset.number("textureBrightness").unwrap_or(0.0) != 0.0 {
        notes.push(Unrepresented::Texture(TextureNote::Brightness));
    }
    if preset.number("textureContrast").unwrap_or(0.0) != 0.0 {
        notes.push(Unrepresented::Texture(TextureNote::Contrast));
    }
    notes.extend(
        pattern
            .notes
            .iter()
            .cloned()
            .map(Unrepresented::TexturePattern),
    );
    Ok(())
}

fn jitter(dynamics: Option<&Object>) -> f64 {
    dynamics.map_or(0.0, |d| clamp01(d.number("jitter").unwrap_or(0.0), 100.0))
}

/// 設定のコントロール（bVTy）: 0 切、1 フェード（fStp 描点）、2 筆圧、3 ペンの傾き、4 スタイラスホイール、5 回転、6 初めの方向、7 方向
/// （KDE の「Krita/Photoshop Mapping Table」と Brushfactory の ABR のメモが一致する順）。大きさ・不透明度・流量は
/// フェード・筆圧・傾きを取り、角度は傾き（ペンの倒れる向き）・方向（線に沿う）・回転（ペンの軸の回転）を取る。ほかは知らせて外す。
fn control(
    dynamics: Option<&Object>,
    setting: Setting,
    notes: &mut Vec<Unrepresented>,
    b: &mut Brush,
    target: Target,
) {
    let Some(d) = dynamics else { return };
    let code = d.number("bVTy").unwrap_or(0.0) as i32;
    if code == 0 {
        return;
    }
    let scalar = matches!(target, Target::Size | Target::Opacity | Target::Flow);
    if code == 2 && scalar {
        match target {
            Target::Size => b.base.pressure_size = true,
            Target::Opacity => b.base.pressure_opacity = true,
            _ => b.base.pressure_flow = true,
        }
        return;
    }
    if code == 1 && scalar {
        let steps = d.number("fStp").or_else(|| d.number("fstp")).unwrap_or(0.0);
        if steps >= 1.0 && steps <= MAX_FADE as f64 {
            let n = steps.round_ties_even() as u32;
            match target {
                Target::Size => b.controls.fade_size = n,
                Target::Opacity => b.controls.fade_opacity = n,
                _ => b.controls.fade_flow = n,
            }
            return;
        }
        notes.push(Unrepresented::FadeRange { setting, steps });
        return;
    }
    if code == 3 && (scalar || target == Target::Angle) {
        match target {
            Target::Size => b.controls.tilt_size = true,
            Target::Opacity => b.controls.tilt_opacity = true,
            Target::Flow => b.controls.tilt_flow = true,
            _ => b.controls.tilt_angle = true,
        }
        return;
    }
    if code == 7 && target == Target::Angle {
        b.tip.follow_direction = true;
        return;
    }
    if code == 5 && target == Target::Angle {
        b.controls.rotation_angle = true;
        return;
    }
    notes.push(Unrepresented::Control {
        setting,
        control: ControlKind::from_code(code),
    });
}

fn clamp01(value: f64, scale: f64) -> f64 {
    (value / scale).clamp(0.0, 1.0)
}
