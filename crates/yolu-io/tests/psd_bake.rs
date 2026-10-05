//! PSD の焼き込み書き出し（チャンネル × 方式）。機能ごとに: 文書を作る → 計画（注記）→ 構築 → 書く → 読み戻す（EditableRaster・診断は注記だけ）
//! → 読み戻した合成が書き出したチャンネルの今の合成と全バイト一致。焼いた層の名前・属性・並びが元と同じで、文書は 1 バイトも変わらない。
//! Photoshop・CLIP STUDIO の実物では確かめていない（読み直しはこの crate の読みだけ）。
#[path = "../../yolu-core/tests/attach_support/mod.rs"]
mod attach_support;
use attach_support::*;
use std::sync::atomic::AtomicBool;
use yolu_core::curve::{Curve, CurvePoint};
use yolu_core::fill_image::{Projection, ProjectionMode};
use yolu_core::generator::{self, anchor::ReadMode, ColorStop, OpacityStop, Ramp, Settings};
use yolu_core::{
    AdjustmentSettings, AdjustmentType, AnchorId, AnchorPlacement, BalanceRange,
    BlendMode as CoreBlend, BrightnessContrast, Channel, ChannelBlend, ChannelInfo, ChannelKind,
    ColorBalance, ColorSpace, Document, EffectSettings, FilterId, FilterSpec, FilterTarget,
    GradientMap, LayerId, LayerKind as CoreKind, NormalSettings, NormalYDirection, Posterize,
    Rgba8, Threshold, ToneChannel, ToneCurves,
};
use yolu_io::psd::{
    self, Adjustment, BlendMode, CompatibilityMode as Mode, Document as Psd, ExportControl,
    ExportMode, ExportNote, ExportOptions, Exported, GradientExpansion, Layer, LayerKind, Limits,
    NoteAction, Refusal, RoundedParameter,
};
use yolu_io::NativeDocument;

// ───────── 道具 ─────────

fn options(channel: Channel) -> ExportOptions {
    ExportOptions::new(channel, ExportMode::Bake)
}

/// 計画 → 構築。計画と構築の注記は同じ。
fn exported(d: &Document, channel: Channel) -> Exported {
    let ctl = ExportControl::default();
    let plan = psd::plan_export(d, &options(channel), &ctl).unwrap();
    assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
    let out = plan.build(d, &ctl).unwrap();
    assert_eq!(out.notes, plan.notes, "計画と構築の注記");
    out
}

/// 文書の写し（アプリが別のスレッドへ渡すもの）から書いても、文書そのものから書いたのと同じ PSD になる（評価のキャッシュ・Anchor の署名を写さない写しでも）。
fn assert_same_from_a_snapshot(d: &Document, channel: Channel, out: &Exported) {
    let copy = d.capture_snapshot().unwrap();
    let again = exported(&copy, channel);
    assert_eq!(again.document, out.document, "写しからの書き出し");
    assert_eq!(again.notes, out.notes);
}

/// 書いて、読み戻して core の文書へ。EditableRaster で、診断は注記（NotCarriedIntoExport・CompositeDiffers）だけ。
fn read_back(psd: &Psd) -> Document {
    let bytes = psd::write(psd, &Limits::default()).unwrap();
    let read = psd::read(&bytes, &Limits::default()).unwrap();
    assert_eq!(
        read.mode(),
        Mode::EditableRaster,
        "{:?}",
        read.diagnostics()
    );
    assert!(
        read.diagnostics().iter().all(|d| d.is_informational()),
        "{:?}",
        read.diagnostics()
    );
    read.to_core().unwrap()
}

fn look(d: &Document, c: Channel) -> Vec<u8> {
    d.composite_channel(c, d.bounds()).unwrap()
}

/// 丸め（刻みへ寄せた・値のカーブを停止点へ展開した）の注記か。
fn is_rounding(action: &NoteAction) -> bool {
    matches!(
        action,
        NoteAction::Rounded { .. } | NoteAction::ExpandedGradientCurve { .. }
    )
}

/// 丸めの注記の、合成の最大の差。
fn rounding_diff(action: &NoteAction) -> Option<u8> {
    match action {
        NoteAction::Rounded { max_diff, .. }
        | NoteAction::ExpandedGradientCurve { max_diff, .. } => Some(*max_diff),
        _ => None,
    }
}

/// 刻みへ丸めた調整があれば、その設定だけを（読み戻した PSD の値で）替えた写しの、書き出したチャンネルの合成。丸めた調整が無ければ今の合成。
/// 調整は層の名前で引く（名前が重ならない文書で）。
fn rounded_look(d: &Document, channel: Channel, out: &Exported, back: &Document) -> Vec<u8> {
    let replaced: Vec<(LayerId, AdjustmentSettings)> = out
        .notes
        .iter()
        .filter(|n| is_rounding(&n.action))
        .map(|n| {
            let written = back.layers().iter().find(|l| l.name() == n.layer);
            let settings = written
                .and_then(|l| l.adjustment())
                .unwrap_or_else(|| panic!("読み戻した調整「{}」が無い", n.layer));
            (id_of(d, &n.layer), settings.clone())
        })
        .collect();
    if replaced.is_empty() {
        look(d, channel)
    } else {
        look(&d.with_adjustments_replaced(&replaced).unwrap(), channel)
    }
}

/// 書き出して読み戻した合成が、書き出したチャンネルの今の合成（丸めた調整があれば、丸めた設定の合成）と全バイト一致する。読み戻した文書を返す。
fn round_trip(d: &Document, channel: Channel) -> (Exported, Document) {
    round_trip_within(d, channel, 0)
}

/// `round_trip` で、各バイトの差が `tolerance` までなら一致とする。反転したマスクを画素に焼くと、マスクの値の反転が `1 - h` と `(255 - 隠す量) / 255` の
/// 浮動小数の最後の桁で違い、まれに 1 画素が 1 ずれる（`inverted_mask_can_differ_in_the_last_digit`）。それ以外は全バイト一致。
fn round_trip_within(d: &Document, channel: Channel, tolerance: u8) -> (Exported, Document) {
    let out = exported(d, channel);
    let back = read_back(&out.document);
    let want = rounded_look(d, channel, &out, &back);
    let got = look(&back, Channel::Color);
    let off = want
        .iter()
        .zip(&got)
        .filter(|(a, b)| a.abs_diff(**b) > tolerance)
        .count();
    assert_eq!(
        off, 0,
        "読み戻した合成が {channel:?} の合成と違う（許す差 {tolerance}）"
    );
    if tolerance > 0 {
        let differing = want.iter().zip(&got).filter(|(a, b)| a != b).count();
        assert!(
            differing * 100 <= want.len(),
            "ずれがまれでない: {differing} / {}",
            want.len()
        );
    }
    (out, back)
}

/// 層に反転したマスクがあるか（焼くと、合成の最後の桁がずれ得る）。
fn has_inverted_mask(d: &Document) -> bool {
    d.layers()
        .iter()
        .any(|l| l.mask().is_some_and(|m| m.inverted()))
}

fn notes_of(out: &Exported) -> Vec<(&str, &NoteAction)> {
    out.notes
        .iter()
        .map(|n| (n.layer.as_str(), &n.action))
        .collect()
}

fn find<'a>(layers: &'a [Layer], name: &str) -> &'a Layer {
    for l in layers {
        if l.name == name {
            return l;
        }
        if let LayerKind::Group { children, .. } = &l.kind {
            if let Some(found) = children.iter().find_map(|c| find_in(c, name)) {
                return found;
            }
        }
    }
    panic!("PSD に層「{name}」が無い")
}
fn find_in<'a>(l: &'a Layer, name: &str) -> Option<&'a Layer> {
    if l.name == name {
        return Some(l);
    }
    match &l.kind {
        LayerKind::Group { children, .. } => children.iter().find_map(|c| find_in(c, name)),
        _ => None,
    }
}

/// PSD の層の画素を画布の大きさ（下から上）に広げる（外は透明）。
fn expand(l: &Layer, width: u32, height: u32) -> Vec<u8> {
    let mut out = vec![0u8; (width * height * 4) as usize];
    for r in 0..l.height {
        let y_up = height as i64 - 1 - (i64::from(l.top) + i64::from(r));
        for c in 0..l.width {
            let x = i64::from(l.left) + i64::from(c);
            if (0..i64::from(width)).contains(&x) && (0..i64::from(height)).contains(&y_up) {
                let to = ((y_up as u32 * width + x as u32) * 4) as usize;
                let from = ((r * l.width + c) * 4) as usize;
                out[to..to + 4].copy_from_slice(&l.pixels_rgba[from..from + 4]);
            }
        }
    }
    out
}

/// 行（`row_bytes` バイト）の並びを上下に入れ替える。
fn rows_reversed(bytes: &[u8], row_bytes: usize) -> Vec<u8> {
    bytes
        .chunks_exact(row_bytes)
        .rev()
        .flatten()
        .copied()
        .collect()
}

/// 完全に透明な画素の RGB を 0 にそろえる（合成は透明の下の色を持たない）。
fn quiet(mut bytes: Vec<u8>) -> Vec<u8> {
    for p in bytes.as_chunks_mut::<4>().0 {
        if p[3] == 0 {
            p.fill(0);
        }
    }
    bytes
}

/// 文書の状態（正本のバイト・版・Undo・確保量・Color の合成）。書き出しの前後で 1 バイトも変わらない。
fn state(d: &Document) -> (Vec<u8>, u64, usize, bool, u64, Vec<u8>) {
    (
        NativeDocument::from_core(d).unwrap().to_bytes(),
        d.revision(),
        d.undo_count(),
        d.can_redo(),
        d.allocated_bytes(),
        look(d, Channel::Color),
    )
}

fn blur(d: &mut Document, layer: LayerId, channels: &[Channel]) -> FilterId {
    d.add_filter(
        layer,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(3)).channels(channels),
    )
    .unwrap()
}

/// 層の PSD での属性（書き出すチャンネルの値）: 名前・不透明度・モード・クリッピング・ロック・表示。
type Attrs = (String, u8, BlendMode, bool, u32, bool);
fn expected_attrs(d: &Document, l: LayerId, c: Channel, visible: bool) -> Attrs {
    let l = d.layer(l).unwrap();
    (
        l.name().to_owned(),
        (l.opacity_in(c) * 255.0).round_ties_even() as u8,
        BlendMode::ALL[l.blend_mode_in(c) as usize],
        l.clipping(),
        0,
        visible,
    )
}
fn attrs(l: &Layer) -> Attrs {
    (
        l.name.clone(),
        l.opacity,
        l.blend_mode,
        l.clipping,
        l.locks,
        l.visible,
    )
}

/// 層を上から下に並べたときの名前（グループの中身はグループの次）。
fn names(layers: &[Layer]) -> Vec<String> {
    let mut out = Vec::new();
    for l in layers {
        out.push(l.name.clone());
        if let LayerKind::Group { children, .. } = &l.kind {
            out.extend(names(children));
        }
    }
    out
}

/// `world()` から塗りつぶしを外した 3 枚（土台・中・上）の文書。
fn three() -> (Document, LayerId, LayerId, LayerId) {
    let (mut d, l) = world();
    d.remove_layer(l[3]).unwrap();
    (d, l[0], l[1], l[2])
}

// ───────── 層の中身のフィルター・Generator ─────────

#[test]
fn filters_and_generators_are_baked_into_the_layers_pixels() {
    let (mut d, base, mid, top) = three();
    blur(&mut d, base, &[Channel::Color]);
    let mut wear = Settings::new(generator::Kind::EdgeWear);
    wear.blend = generator::Blend::Multiply;
    d.add_filter(
        top,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::generator(wear.clone())).channels(&[Channel::Color]),
    )
    .unwrap();
    d.add_filter(
        top,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::noise(0.4, 5, false)).channels(&[Channel::Color]),
    )
    .unwrap();
    d.set_layer_opacity(top, 200.0 / 255.0, false).unwrap();
    d.set_layer_blend_mode(top, CoreBlend::Multiply).unwrap();
    d.add_layer_mask(top).unwrap();
    for x in 0..W {
        d.set_mask_pixel(top, x, 6, (x * 5) as u8).unwrap();
    }
    d.set_layer_clipping(top, true).unwrap();
    d.set_layer_locks(base, yolu_core::LayerLocks::POSITION)
        .unwrap();
    let before = state(&d);

    let (out, back) = round_trip(&d, Channel::Color);
    assert_eq!(
        notes_of(&out),
        [
            (
                "上",
                &NoteAction::BakedFilters(vec![
                    EffectSettings::generator(wear),
                    EffectSettings::noise(0.4, 5, false)
                ])
            ),
            (
                "土台",
                &NoteAction::BakedFilters(vec![EffectSettings::blur(3)])
            ),
        ]
    );
    // 層の名前・並び・属性は元と同じ（ロックも）。焼いた画素は層の出力そのもの
    assert_eq!(names(&out.document.layers), ["上", "中", "土台"]);
    for (name, id) in [("上", top), ("中", mid), ("土台", base)] {
        let l = find(&out.document.layers, name);
        let mut want = expected_attrs(&d, id, Channel::Color, true);
        want.4 = if id == base { 4 } else { 0 };
        assert_eq!(attrs(l), want, "{name}");
        assert!(matches!(l.kind, LayerKind::Raster), "{name}");
    }
    for (name, id) in [("上", top), ("土台", base)] {
        assert_eq!(
            quiet(expand(find(&out.document.layers, name), W, H)),
            quiet(d.layer_output(id, Channel::Color, d.bounds()).unwrap()),
            "{name} の画素はフィルターを通した出力"
        );
    }
    // 中は保存した画素のまま（焼かない）
    assert_eq!(
        quiet(expand(find(&out.document.layers, "中"), W, H)),
        quiet(d.layer_output(mid, Channel::Color, d.bounds()).unwrap())
    );
    // 読み戻した層は効果の無い文書の層で、ロックと名前を持つ
    assert!(back.layers().iter().all(|l| l.filters().is_empty()));
    assert_eq!(
        back.layers()
            .iter()
            .map(|l| l.name().to_owned())
            .collect::<Vec<_>>(),
        ["土台", "中", "上"]
    );
    assert_eq!(state(&d), before, "文書は 1 バイトも変わらない");
    assert_same_from_a_snapshot(&d, Channel::Color, &out);
    // 厳密な書き出しは、焼かないので今までどおり断る
    assert!(Psd::from_core(&d).is_err());
}

#[test]
fn the_ids_and_order_of_baked_layers_are_the_same_as_when_they_are_written_as_they_are() {
    let (mut d, base, _, top) = three();
    let plain = Psd::from_core(&d).unwrap();
    blur(&mut d, base, &[Channel::Color]);
    blur(&mut d, top, &[Channel::Color]);
    let baked = exported(&d, Channel::Color).document;
    assert_eq!(
        baked
            .layers
            .iter()
            .map(|l| (l.id, l.name.clone()))
            .collect::<Vec<_>>(),
        plain
            .layers
            .iter()
            .map(|l| (l.id, l.name.clone()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn filters_that_are_off_or_for_other_channels_are_dropped_or_ignored_without_baking() {
    let (mut d, base, _, top) = three();
    let off = blur(&mut d, base, &[Channel::Color]);
    d.set_filter_enabled(base, off, false).unwrap();
    // Height だけの段は、Color の PSD に関わらない
    blur(&mut d, top, &[Channel::Height]);
    let (out, _) = round_trip(&d, Channel::Color);
    assert_eq!(
        notes_of(&out),
        [(
            "土台",
            &NoteAction::DroppedFilters(vec![EffectSettings::blur(3)])
        )]
    );
    // 焼かない: 画素は保存した画素の外接矩形（タイルの）のまま
    let kept = find(&out.document.layers, "土台");
    assert_eq!(
        quiet(expand(kept, W, H)),
        quiet(d.layer_output(base, Channel::Color, d.bounds()).unwrap())
    );
    // Height の PSD では、Height の段が焼かれる
    let (height, _) = round_trip(&d, Channel::Height);
    assert!(height
        .notes
        .iter()
        .any(|n| n.layer == "上" && matches!(n.action, NoteAction::BakedFilters(_))));
}

// ───────── マスク ─────────

#[test]
fn mask_filters_and_inverted_masks_are_baked_into_the_masks_pixels() {
    let (mut d, base, mid, top) = three();
    // 土台: マスクのフィルター。中: 反転。上: 反転とフィルター（無効のマスク・濃度つき）
    d.add_layer_mask(base).unwrap();
    d.add_layer_mask(mid).unwrap();
    d.add_layer_mask(top).unwrap();
    for (id, y) in [(base, 3), (mid, 9), (top, 14)] {
        for x in 0..W {
            d.set_mask_pixel(id, x, y, (x * 6 + 10) as u8).unwrap();
        }
    }
    d.add_filter(
        base,
        FilterTarget::Mask,
        FilterSpec::new(EffectSettings::noise(0.6, 3, true)),
    )
    .unwrap();
    d.set_layer_mask_inverted(mid, true).unwrap();
    d.set_layer_mask_density(mid, 200.0 / 255.0, false).unwrap();
    d.set_layer_mask_inverted(top, true).unwrap();
    d.add_filter(
        top,
        FilterTarget::Mask,
        FilterSpec::new(EffectSettings::invert()),
    )
    .unwrap();
    d.set_layer_mask_density(top, 128.0 / 255.0, false).unwrap();
    let before = state(&d);

    let (out, back) = round_trip(&d, Channel::Color);
    assert_eq!(
        notes_of(&out),
        [
            (
                "上",
                &NoteAction::BakedMaskFilters(vec![EffectSettings::invert()])
            ),
            ("上", &NoteAction::BakedInvertedMask),
            ("中", &NoteAction::BakedInvertedMask),
            (
                "土台",
                &NoteAction::BakedMaskFilters(vec![EffectSettings::noise(0.6, 3, true)])
            ),
        ]
    );
    // 読み戻したマスクは反転もフィルターも持たない（焼いた値）。濃度は元のまま
    for l in back.layers() {
        let m = l.mask().unwrap();
        assert!(!m.inverted() && m.filters().is_empty(), "{}", l.name());
        assert!(m.enabled());
    }
    let density = |name: &str| {
        let l = back.layers().iter().find(|l| l.name() == name).unwrap();
        (l.mask().unwrap().density() * 255.0).round() as u8
    };
    assert_eq!(
        (density("土台"), density("中"), density("上")),
        (255, 200, 128)
    );
    assert_eq!(state(&d), before);
}

/// 反転したマスクを画素に焼く（反転した値を、反転しないマスクの画素にする）と、不透明度に掛ける値は、実数では同じでも、浮動小数の最後の桁で
/// 違う画素値がある（`1 - h` と `(255 - 隠す量) / 255`）。保証の射程は「全バイト一致」より少し狭く、反転したマスクを焼いた層が重なる所で、
/// まれに 1 画素が 1 ずれ得る。ずれの大きさは最後の桁（1e-15 未満）で、見た目に出る差ではない。
#[test]
fn an_inverted_mask_baked_into_values_differs_from_the_original_only_in_the_last_digit() {
    let (mut d, base, mid, _) = three();
    for id in [base, mid] {
        d.add_layer_mask(id).unwrap();
    }
    d.set_layer_mask_inverted(base, true).unwrap();
    let inverted = d.layer(base).unwrap().mask().unwrap();
    let plain = d.layer(mid).unwrap().mask().unwrap();
    let mut differing = 0;
    for hide in 0..=255u8 {
        // 元: 隠す量 `hide` の反転したマスク。焼いたあと: 反転した値（255 - hide）の、反転しないマスク
        let (a, b) = (inverted.factor(hide), plain.factor(255 - hide));
        assert!((a - b).abs() < 1e-15, "{hide}: {a} {b}");
        differing += usize::from(a != b);
    }
    assert!(differing > 0, "この差が無ければ、全バイト一致と言える");
}

#[test]
fn a_disabled_mask_with_filters_is_baked_and_stays_disabled() {
    let (mut d, base, _, _) = three();
    d.add_layer_mask(base).unwrap();
    for x in 0..W {
        d.set_mask_pixel(base, x, 5, 255).unwrap();
    }
    d.add_filter(
        base,
        FilterTarget::Mask,
        FilterSpec::new(EffectSettings::invert()),
    )
    .unwrap();
    d.set_layer_mask_enabled(base, false).unwrap();
    let (out, back) = round_trip(&d, Channel::Color);
    assert_eq!(out.notes.len(), 1);
    let m = back
        .layers()
        .iter()
        .find(|l| l.name() == "土台")
        .unwrap()
        .mask()
        .unwrap();
    assert!(!m.enabled(), "有効の印はそのまま");
    // 有効にすると、文書で有効にしたときと同じ（焼いた値）
    let mut on = d.capture_snapshot().unwrap();
    let mut back_on = read_back(&out.document);
    on.set_layer_mask_enabled(base, true).unwrap();
    let id = back_on
        .layers()
        .iter()
        .find(|l| l.name() == "土台")
        .unwrap()
        .id();
    back_on.set_layer_mask_enabled(id, true).unwrap();
    assert_eq!(look(&on, Channel::Color), look(&back_on, Channel::Color));
}

// ───────── 塗りつぶし ─────────

#[test]
fn a_translucent_fill_value_is_baked_and_an_opaque_one_stays_a_solid_colour() {
    let (mut d, base, _, _) = three();
    d.add_fill_layer(
        "ガラス",
        &[(Channel::Color, Rgba8::new(10, 90, 200, 120))],
        None,
    )
    .unwrap();
    d.add_fill_layer(
        "単色",
        &[(Channel::Color, Rgba8::new(200, 30, 60, 255))],
        None,
    )
    .unwrap();
    let _ = base;
    let (out, back) = round_trip(&d, Channel::Color);
    assert_eq!(
        notes_of(&out),
        [("ガラス", &NoteAction::BakedTranslucentFill)]
    );
    assert!(matches!(
        find(&out.document.layers, "単色").kind,
        LayerKind::SolidColor([200, 30, 60])
    ));
    let glass = find(&out.document.layers, "ガラス");
    assert!(matches!(glass.kind, LayerKind::Raster));
    assert_eq!(
        (glass.left, glass.top, glass.width, glass.height),
        (0, 0, W, H),
        "全面"
    );
    let id = back.layers().iter().find(|l| l.name() == "ガラス").unwrap();
    assert_eq!(id.kind(), CoreKind::Raster);
}

#[test]
fn fills_that_read_an_image_a_projection_or_a_gradient_are_baked_per_channel() {
    let (mut d, l) = world();
    let fill = l[3];
    // Color は画像（平面の投影）、Height はグラデーション。Roughness は不透明の値だけ
    d.set_fill_value(
        fill,
        Channel::Roughness,
        Some(Rgba8::new(90, 90, 90, 255)),
        false,
    )
    .unwrap();
    d.set_fill_image(fill, Channel::Color, Some(image(0)))
        .unwrap();
    d.set_fill_projection(
        fill,
        Projection {
            mode: ProjectionMode::Planar,
            tiles: [2.0, 2.0],
            ..Default::default()
        },
        false,
    )
    .unwrap();
    let mut gradient = Settings::new(generator::Kind::ShapeGradient);
    gradient.ramp = Some(generator::Ramp::default());
    gradient.blend = generator::Blend::Replace;
    d.set_fill_gradient(fill, Channel::Height, Some(gradient), false)
        .unwrap();
    let before = state(&d);

    let (color, back) = round_trip(&d, Channel::Color);
    let note = color.notes.iter().find(|n| n.layer == "塗り").unwrap();
    match &note.action {
        NoteAction::BakedFill(s) => {
            assert!(s.image && !s.gradient && !s.decal);
            assert_eq!(s.projection, Some(ProjectionMode::Planar));
        }
        other => panic!("{other:?}"),
    }
    let baked = find(&color.document.layers, "塗り");
    assert!(matches!(baked.kind, LayerKind::Raster));
    assert_eq!(
        quiet(expand(baked, W, H)),
        quiet(d.layer_output(fill, Channel::Color, d.bounds()).unwrap())
    );
    assert_eq!(
        back.layers()
            .iter()
            .filter(|l| l.kind() == CoreKind::Fill)
            .count(),
        0
    );

    let (height, _) = round_trip(&d, Channel::Height);
    let note = height.notes.iter().find(|n| n.layer == "塗り").unwrap();
    assert!(matches!(&note.action, NoteAction::BakedFill(s) if s.gradient && !s.image));
    // 値だけのチャンネルは、同じ層でも単色の塗りつぶしのまま（注記なし）
    let (rough, _) = round_trip(&d, Channel::Roughness);
    assert!(!rough.notes.iter().any(|n| n.layer == "塗り"));
    assert!(matches!(
        find(&rough.document.layers, "塗り").kind,
        LayerKind::SolidColor([90, 90, 90])
    ));
    assert_eq!(state(&d), before);
}

#[test]
fn a_decal_is_baked_as_pixels_in_every_channel_it_has_a_value_for() {
    let (mut d, l) = world();
    let fill = l[3];
    d.set_fill_image(fill, Channel::Color, Some(image(0)))
        .unwrap();
    d.set_fill_projection(
        fill,
        Projection {
            mode: ProjectionMode::Decal,
            ..Default::default()
        },
        false,
    )
    .unwrap();
    let (color, _) = round_trip(&d, Channel::Color);
    let note = color.notes.iter().find(|n| n.layer == "塗り").unwrap();
    assert!(matches!(&note.action, NoteAction::BakedFill(s) if s.image && s.decal));
    // デカールは値のあるチャンネル全部が画素から来る（Height の値だけでも）
    let (height, _) = round_trip(&d, Channel::Height);
    let note = height.notes.iter().find(|n| n.layer == "塗り").unwrap();
    assert!(matches!(&note.action, NoteAction::BakedFill(s) if s.decal && !s.image));
    assert_same_from_a_snapshot(&d, Channel::Height, &height);
}

// ───────── Anchor・パス ─────────

#[test]
fn anchors_are_dropped_and_the_generators_that_read_them_are_baked() {
    let (mut d, base, _, top) = three();
    d.add_layer_mask(base).unwrap();
    for x in 0..W {
        d.set_mask_pixel(base, x, 8, 200).unwrap();
    }
    let a_layer: AnchorId = d
        .add_anchor(base, AnchorPlacement::Layer, None, None)
        .unwrap();
    let a_mask: AnchorId = d
        .add_anchor(base, AnchorPlacement::Mask, None, None)
        .unwrap();
    for (anchor, read) in [(a_layer, Channel::Color), (a_mask, Channel::Color)] {
        let mut g = Settings::new(generator::Kind::Anchor);
        g.blend = generator::Blend::Multiply;
        let id = d
            .add_filter(
                top,
                FilterTarget::Content,
                FilterSpec::new(EffectSettings::generator(g)).channels(&[Channel::Color]),
            )
            .unwrap();
        d.set_generator_anchor(top, id, Some(anchor), read, ReadMode::Value, false)
            .unwrap();
    }
    let before = state(&d);
    let (out, _) = round_trip(&d, Channel::Color);
    assert_same_from_a_snapshot(&d, Channel::Color, &out);
    let names: Vec<_> = notes_of(&out)
        .into_iter()
        .map(|(n, a)| (n, std::mem::discriminant(a)))
        .collect();
    assert_eq!(
        names,
        [
            (
                "上",
                std::mem::discriminant(&NoteAction::BakedFilters(vec![]))
            ),
            ("土台", std::mem::discriminant(&NoteAction::DroppedAnchor)),
            (
                "土台",
                std::mem::discriminant(&NoteAction::DroppedMaskAnchor)
            ),
        ]
    );
    assert_eq!(state(&d), before);
}

#[test]
fn a_path_layer_keeps_its_rasterized_pixels_and_says_the_path_is_not_carried() {
    use yolu_core::paths::{CanvasPath, CanvasPoint, PathBrush};
    let (mut d, _, _, _) = three();
    let layer = d.add_layer("パス").unwrap();
    d.set_canvas_path(
        layer,
        CanvasPath {
            id: 0x77,
            channel: Channel::Color,
            brush: PathBrush(yolu_core::BrushSettings {
                radius: 3.0,
                spacing: 0.17,
                color: Rgba8::new(201, 37, 89, 219),
                ..yolu_core::BrushSettings::default()
            }),
            points: vec![
                CanvasPoint::new(4.5, 7.25, 0.3).unwrap(),
                CanvasPoint::new(29.5, 21.5, 0.9).unwrap(),
            ],
            material: None,
        },
    )
    .unwrap();
    let before = state(&d);
    let (out, back) = round_trip(&d, Channel::Color);
    assert_eq!(notes_of(&out), [("パス", &NoteAction::BakedPath)]);
    assert!(back.layers().iter().all(|l| l.path().is_none()));
    // Height の PSD には、パスの Color は関わらない
    let (height, _) = round_trip(&d, Channel::Height);
    assert!(height.notes.is_empty());
    assert_eq!(state(&d), before);
}

// ───────── クリッピングされたグループ ─────────

#[test]
fn a_clipped_group_becomes_one_raster_layer_with_the_groups_attributes() {
    let (mut d, base, mid, top) = three();
    blur(&mut d, mid, &[Channel::Color]);
    // 下地の上に、通過のグループ（クリッピング・不透明度・マスクつき）。中は 2 枚
    let g = d.group_layers(&[mid, top], "覆い").unwrap();
    d.set_layer_clipping(g, true).unwrap();
    d.set_layer_opacity(g, 180.0 / 255.0, false).unwrap();
    d.add_layer_mask(g).unwrap();
    for x in 0..W {
        d.set_mask_pixel(g, x, 10, 255).unwrap();
    }
    d.set_layer_locks(g, yolu_core::LayerLocks::TRANSPARENCY)
        .unwrap();
    // 別のグループ（クリッピングされていない）は、そのままグループ
    let plain = d.add_layer("普通").unwrap();
    paint(&mut d, plain, Channel::Color, 9);
    let other = d.group_layers(&[plain], "普通のグループ").unwrap();
    let _ = (base, other);
    let before = state(&d);

    let (out, back) = round_trip(&d, Channel::Color);
    assert_eq!(notes_of(&out), [("覆い", &NoteAction::BakedClippedGroup)]);
    let layer = find(&out.document.layers, "覆い");
    assert!(matches!(layer.kind, LayerKind::Raster), "1 枚のラスター層");
    assert_eq!(
        (
            layer.opacity,
            layer.clipping,
            layer.blend_mode,
            layer.visible,
            layer.locks
        ),
        (180, true, BlendMode::Normal, true, 1),
        "通過は使えないので通常。ほかの属性は元のまま"
    );
    assert!(layer.mask.is_some());
    assert_eq!(
        names(&out.document.layers),
        ["普通のグループ", "普通", "覆い", "土台"]
    );
    assert!(out
        .document
        .layers
        .iter()
        .any(|l| matches!(l.kind, LayerKind::Group { .. })));
    assert_eq!(back.layers().iter().filter(|l| l.is_group()).count(), 1);
    // グループの出力そのもの
    assert_eq!(
        quiet(expand(layer, W, H)),
        quiet(d.group_output(g, Channel::Color, d.bounds()).unwrap())
    );
    assert_eq!(state(&d), before);
}

#[test]
fn a_clipped_group_inside_a_group_and_an_isolated_one_are_baked_in_place() {
    let (mut d, base, mid, top) = three();
    d.set_layer_blend_mode(top, CoreBlend::Overlay).unwrap();
    let inner = d.group_layers(&[mid, top], "内").unwrap();
    d.set_layer_blend_mode(inner, CoreBlend::Screen).unwrap();
    d.set_layer_clipping(inner, true).unwrap();
    let outer = d.group_layers(&[base, inner], "外").unwrap();
    let (out, _) = round_trip(&d, Channel::Color);
    assert_eq!(notes_of(&out), [("内", &NoteAction::BakedClippedGroup)]);
    assert_eq!(names(&out.document.layers), ["外", "内", "土台"]);
    let l = find(&out.document.layers, "内");
    assert_eq!(
        (l.blend_mode, l.clipping),
        (BlendMode::Screen, true),
        "分離のモードはそのまま"
    );
    let _ = outer;
}

/// 兄弟の一番下にあるグループのクリッピングの印は、何にもクリッピングされない（合成は普通のグループとして重ねる）。通過のグループの調整・モードは
/// 下へ効くので、1 枚の画素に焼くと見た目が変わる。印が効いているグループだけを焼く。効いていない印は、確かめていない「印の付いたフォルダー」の形を
/// 書かないよう外し（見た目は同じ）、注記に出す。Unity 版は印だけを見て断るので、厳密な書き出しも同じ理由で断る。
#[test]
fn a_clipping_mark_that_clips_nothing_is_dropped_and_the_group_stays_a_group() {
    let (mut d, base, mid, top) = three();
    let _ = (base, mid, top);
    // 親の通過のグループの一番下にある、印つきの通過のグループ。中の調整は、親の外の下の層（土台など）へ効く
    let adj = d
        .add_adjustment_layer("反転", AdjustmentSettings::invert(), None, None)
        .unwrap();
    let inner = d.group_layers(&[adj], "内の組").unwrap();
    d.set_layer_clipping(inner, true).unwrap();
    let _parent = d.group_layers(&[inner], "親の組").unwrap();
    let (out, back) = round_trip(&d, Channel::Color);
    assert_eq!(
        notes_of(&out),
        [("内の組", &NoteAction::DroppedClippingMark)],
        "焼かず、印を外すことだけを注記に出す"
    );
    let written = find(&out.document.layers, "内の組");
    assert!(matches!(written.kind, LayerKind::Group { .. }));
    assert!(!written.clipping, "印の付いたフォルダーは書かない");
    assert!(
        back.layers().iter().all(|l| !l.clipping()),
        "読み戻した文書にも印は無い"
    );
    assert!(d.layer(inner).unwrap().clipping(), "文書の印は残る");
    // 厳密な書き出し（Unity 版の PsdBridge と同じ）は、印だけを見て同じ層を同じ理由で断る
    let blockers = psd::export_blockers(&d);
    assert_eq!(
        blockers,
        vec![psd::Blocker {
            layer: "内の組".into(),
            refusal: Refusal::ClippedGroup
        }]
    );
    match Psd::from_core(&d).expect_err("印だけで断る") {
        yolu_io::Error::InvalidData(message) => assert_eq!(message, blockers[0].message()),
        other => panic!("{other}"),
    }
    // 効いている印（上に兄弟がいる）は焼く
    let (mut e, base, mid, top) = three();
    let g = e.group_layers(&[mid], "組").unwrap();
    e.set_layer_clipping(g, true).unwrap();
    let _ = (base, top);
    let (out, _) = round_trip(&e, Channel::Color);
    assert!(out
        .notes
        .iter()
        .any(|n| n.action == NoteAction::BakedClippedGroup));
}

/// 合成が落とす（中身が無い）クリッピングされたグループを、画素の層にして残すと、下地のグループがクリッピングの組を持って通過でなくなる。
/// 落とす層は隠した層にして、下地の通過を保つ。
#[test]
fn an_empty_clipped_group_is_written_hidden_so_its_base_group_keeps_passing_through() {
    let (mut d, base, mid, top) = three();
    d.set_layer_blend_mode(top, CoreBlend::Screen).unwrap();
    // 下地: 通過のグループ（中の Screen の層が、下の土台と混ざる）。その上に、何も出さないクリッピングされたグループ
    let base_group = d.group_layers(&[top], "下地の組").unwrap();
    let empty = d.add_group("空の組", None).unwrap();
    d.set_layer_clipping(empty, true).unwrap();
    let _ = (base, mid, base_group);
    let (out, _) = round_trip(&d, Channel::Color);
    assert!(out
        .notes
        .iter()
        .any(|n| n.action == NoteAction::BakedClippedGroup));
    assert!(!find(&out.document.layers, "空の組").visible);
}

// ───────── 刻みの間の調整 ─────────

fn adjusted(settings: AdjustmentSettings) -> Document {
    let (mut d, _, _, _) = three();
    d.add_adjustment_layer("調整", settings, None, None)
        .unwrap();
    d
}

/// 丸めた設定で組んだ文書の合成と、読み戻した合成が全バイト一致し、注記の最大の差は、今の合成と丸めた文書の合成の最大の差。
fn check_rounding(
    original: AdjustmentSettings,
    rounded: AdjustmentSettings,
    changed: &[(RoundedParameter, f64, f64)],
) {
    let d = adjusted(original.clone());
    let twin = adjusted(rounded.clone());
    let out = exported(&d, Channel::Color);
    let [note] = out.notes.as_slice() else {
        panic!("{:?}", out.notes)
    };
    assert_eq!(note.layer, "調整");
    let NoteAction::Rounded {
        kind,
        changes,
        max_diff,
    } = &note.action
    else {
        panic!("{:?}", note.action)
    };
    assert_eq!(*kind, original.kind());
    let got: Vec<(RoundedParameter, f64, f64)> = changes
        .iter()
        .map(|c| (c.parameter, c.from, c.to))
        .collect();
    assert_eq!(got.len(), changed.len(), "{got:?}");
    for ((p, from, to), (wp, wfrom, wto)) in got.iter().zip(changed) {
        assert_eq!(p, wp);
        assert!(
            (from - wfrom).abs() < 1e-9 && (to - wto).abs() < 1e-9,
            "{p:?}: {from} → {to}"
        );
    }
    let back = read_back(&out.document);
    assert_eq!(
        look(&back, Channel::Color),
        look(&twin, Channel::Color),
        "丸めた文書と読み戻しが一致"
    );
    let truth = look(&d, Channel::Color)
        .iter()
        .zip(look(&twin, Channel::Color))
        .map(|(a, b)| a.abs_diff(b))
        .max()
        .unwrap();
    assert_eq!(*max_diff, truth, "合成の最大の差");
    let a = back
        .layers()
        .iter()
        .find(|l| l.name() == "調整")
        .unwrap()
        .adjustment()
        .unwrap();
    assert_eq!(*a, rounded, "読み戻した調整は丸めた設定");
}

#[test]
fn adjustments_between_psd_steps_are_rounded_to_the_nearest_step_with_the_difference() {
    use RoundedParameter::*;
    // 入力の黒 76.5 は偶数へ（76）
    check_rounding(
        AdjustmentSettings::levels(0.3, 1.0, 1.0, 0.0, 1.0).unwrap(),
        AdjustmentSettings::levels(76.0 / 255.0, 1.0, 1.0, 0.0, 1.0).unwrap(),
        &[(InputBlack, 76.5, 76.0)],
    );
    check_rounding(
        AdjustmentSettings::levels(0.0, 1.0, 1.234, 0.0, 1.0).unwrap(),
        AdjustmentSettings::levels(0.0, 1.0, 1.23, 0.0, 1.0).unwrap(),
        &[(Gamma, 1.234, 1.23)],
    );
    check_rounding(
        AdjustmentSettings::levels(0.101, 0.899, 2.0, 0.203, 0.797).unwrap(),
        AdjustmentSettings::levels(
            26.0 / 255.0,
            229.0 / 255.0,
            2.0,
            52.0 / 255.0,
            203.0 / 255.0,
        )
        .unwrap(),
        &[
            (InputBlack, 25.755, 26.0),
            (InputWhite, 229.245, 229.0),
            (OutputBlack, 51.765, 52.0),
            (OutputWhite, 203.235, 203.0),
        ],
    );
    check_rounding(
        AdjustmentSettings::hue_saturation(10.5, 0.333, -0.1234).unwrap(),
        AdjustmentSettings::hue_saturation(10.0, 0.33, -0.12).unwrap(),
        &[
            (Hue, 10.5, 10.0),
            (Saturation, 33.3, 33.0),
            (Lightness, -12.34, -12.0),
        ],
    );
    // PSD の入力の範囲の外（黒は 0〜253、白はその上の 2〜255）は範囲の中へ寄せる
    check_rounding(
        AdjustmentSettings::levels(0.0, 1.0 / 255.0, 1.0, 0.0, 1.0).unwrap(),
        AdjustmentSettings::levels(0.0, 2.0 / 255.0, 1.0, 0.0, 1.0).unwrap(),
        &[(InputWhite, 1.0, 2.0)],
    );
    check_rounding(
        AdjustmentSettings::levels(253.5 / 255.0, 1.0, 1.0, 0.0, 1.0).unwrap(),
        AdjustmentSettings::levels(253.0 / 255.0, 1.0, 1.0, 0.0, 1.0).unwrap(),
        &[(InputBlack, 253.5, 253.0)],
    );
}

#[test]
fn exact_adjustments_have_no_note_and_a_hidden_rounded_one_has_no_difference() {
    let exact = adjusted(
        AdjustmentSettings::levels(
            20.0 / 255.0,
            230.0 / 255.0,
            1.37,
            10.0 / 255.0,
            240.0 / 255.0,
        )
        .unwrap(),
    );
    assert!(exported(&exact, Channel::Color).notes.is_empty());
    let (mut d, _, _, _) = three();
    let a = d
        .add_adjustment_layer(
            "隠す",
            AdjustmentSettings::hue_saturation(0.5, 0.0, 0.0).unwrap(),
            None,
            None,
        )
        .unwrap();
    d.set_layer_visible(a, false).unwrap();
    let out = exported(&d, Channel::Color);
    assert!(matches!(
        out.notes.as_slice(),
        [ExportNote {
            action: NoteAction::Rounded { max_diff: 0, .. },
            ..
        }]
    ));
    assert!(!find(&out.document.layers, "隠す").visible);
    // 丸めても層の属性は元のまま
    let back = read_back(&out.document);
    assert!(back
        .layers()
        .iter()
        .any(|l| l.name() == "隠す" && !l.visible()));
}

// ───────── 色調補正 6 種の刻みの間 ─────────

fn curve_of(points: &[(f64, f64)]) -> Curve {
    Curve::new(points.iter().map(|&(x, y)| CurvePoint { x, y }).collect()).unwrap()
}
fn color_stop(position: f64, midpoint: f64, rgb: [u8; 3]) -> ColorStop {
    ColorStop {
        position,
        color: Rgba8::new(rgb[0], rgb[1], rgb[2], 255),
        midpoint,
    }
}
fn opacity_stop(position: f64, opacity: f64, midpoint: f64) -> OpacityStop {
    OpacityStop {
        position,
        opacity,
        midpoint,
    }
}
fn flat_opacities() -> Vec<OpacityStop> {
    vec![opacity_stop(0.0, 1.0, 0.5), opacity_stop(1.0, 1.0, 0.5)]
}
fn gradient_map(
    colors: Vec<ColorStop>,
    opacities: Vec<OpacityStop>,
    curve: Option<&[(f64, f64)]>,
    reverse: bool,
) -> AdjustmentSettings {
    let curve = curve.map(|c| c.iter().map(|&(x, y)| CurvePoint { x, y }).collect());
    AdjustmentSettings::gradient_map(GradientMap::new(
        Ramp::new(colors, opacities, curve).unwrap(),
        reverse,
    ))
}

#[test]
fn the_new_adjustments_between_psd_steps_are_rounded_to_the_nearest_step_with_the_difference() {
    use RoundedParameter::*;
    // カラーバランス（整数の刻み。33.5 は偶数へ）
    check_rounding(
        AdjustmentSettings::color_balance(
            ColorBalance::new([0.0, 0.0, 12.4], [-7.6, 0.0, 0.0], [0.0, 33.5, 0.0], false).unwrap(),
        ),
        AdjustmentSettings::color_balance(
            ColorBalance::new([0.0, 0.0, 12.0], [-8.0, 0.0, 0.0], [0.0, 34.0, 0.0], false).unwrap(),
        ),
        &[
            (
                Balance {
                    range: BalanceRange::Shadows,
                    axis: 2,
                },
                12.4,
                12.0,
            ),
            (
                Balance {
                    range: BalanceRange::Midtones,
                    axis: 0,
                },
                -7.6,
                -8.0,
            ),
            (
                Balance {
                    range: BalanceRange::Highlights,
                    axis: 1,
                },
                33.5,
                34.0,
            ),
        ],
    );
    // 明るさ・コントラスト（整数の刻み。10.5 は偶数へ）
    check_rounding(
        AdjustmentSettings::brightness_contrast(BrightnessContrast::new(10.5, -0.25).unwrap()),
        AdjustmentSettings::brightness_contrast(BrightnessContrast::new(10.0, 0.0).unwrap()),
        &[(Brightness, 10.5, 10.0), (Contrast, -0.25, 0.0)],
    );
    // トーンカーブ（入力・出力とも 0〜255 の整数）。R の曲線は隣の点が近すぎて、四捨五入だけでは PSD の間隔（6）に足りない
    // （入力 5・10 → 6・12 へ離す）
    check_rounding(
        AdjustmentSettings::tone_curve(
            ToneCurves::identity()
                .with_curve(
                    ToneChannel::Composite,
                    curve_of(&[(0.0, 0.0), (0.301, 0.31), (0.702, 0.65), (1.0, 1.0)]),
                )
                .with_curve(
                    ToneChannel::Red,
                    curve_of(&[(0.0, 0.0), (0.02, 0.101), (0.04, 0.202), (1.0, 1.0)]),
                ),
        ),
        AdjustmentSettings::tone_curve(
            ToneCurves::identity()
                .with_curve(
                    ToneChannel::Composite,
                    curve_of(&[
                        (0.0, 0.0),
                        (77.0 / 255.0, 79.0 / 255.0),
                        (179.0 / 255.0, 166.0 / 255.0),
                        (1.0, 1.0),
                    ]),
                )
                .with_curve(
                    ToneChannel::Red,
                    curve_of(&[
                        (0.0, 0.0),
                        (6.0 / 255.0, 26.0 / 255.0),
                        (12.0 / 255.0, 52.0 / 255.0),
                        (1.0, 1.0),
                    ]),
                ),
        ),
        &[
            (
                CurveInput {
                    curve: ToneChannel::Composite,
                    point: 1,
                },
                76.755,
                77.0,
            ),
            (
                CurveOutput {
                    curve: ToneChannel::Composite,
                    point: 1,
                },
                79.05,
                79.0,
            ),
            (
                CurveInput {
                    curve: ToneChannel::Composite,
                    point: 2,
                },
                179.01,
                179.0,
            ),
            (
                CurveOutput {
                    curve: ToneChannel::Composite,
                    point: 2,
                },
                165.75,
                166.0,
            ),
            (
                CurveInput {
                    curve: ToneChannel::Red,
                    point: 1,
                },
                5.1,
                6.0,
            ),
            (
                CurveOutput {
                    curve: ToneChannel::Red,
                    point: 1,
                },
                25.755,
                26.0,
            ),
            (
                CurveInput {
                    curve: ToneChannel::Red,
                    point: 2,
                },
                10.2,
                12.0,
            ),
            (
                CurveOutput {
                    curve: ToneChannel::Red,
                    point: 2,
                },
                51.51,
                52.0,
            ),
        ],
    );
    // グラデーションマップ（位置 1/4096・中点 1%・不透明度 1/255。最後の分岐点の中点は使われないので数えない）
    let at = |n: f64| n / 4096.0 * 100.0;
    check_rounding(
        gradient_map(
            vec![
                color_stop(0.0, 0.5, [10, 20, 30]),
                color_stop(0.3, 0.333, [200, 120, 40]),
                color_stop(1.0, 0.37, [250, 250, 90]),
            ],
            vec![
                opacity_stop(0.0, 1.0, 0.5),
                opacity_stop(0.7, 0.5, 0.25),
                opacity_stop(1.0, 1.0, 0.5),
            ],
            None,
            false,
        ),
        gradient_map(
            vec![
                color_stop(0.0, 0.5, [10, 20, 30]),
                color_stop(1229.0 / 4096.0, 33.0 / 100.0, [200, 120, 40]),
                color_stop(1.0, 0.5, [250, 250, 90]),
            ],
            vec![
                opacity_stop(0.0, 1.0, 0.5),
                opacity_stop(2867.0 / 4096.0, 128.0 / 255.0, 0.25),
                opacity_stop(1.0, 1.0, 0.5),
            ],
            None,
            false,
        ),
        &[
            (ColorStopPosition(1), 30.0, at(1229.0)),
            (ColorStopMidpoint(1), 33.3, 33.0),
            (OpacityStopPosition(1), 70.0, at(2867.0)),
            (OpacityStopValue(1), 50.0, 128.0 / 255.0 * 100.0),
        ],
    );
    // 近い 2 つの分岐点が同じ刻みに寄るときは、1 刻みだけ離す（PSD は同じ位置を許さない）
    check_rounding(
        gradient_map(
            vec![
                color_stop(0.0, 0.5, [0, 0, 0]),
                color_stop(0.3, 0.5, [255, 0, 0]),
                color_stop(0.30015, 0.5, [0, 255, 0]),
                color_stop(1.0, 0.5, [255, 255, 255]),
            ],
            flat_opacities(),
            None,
            true,
        ),
        gradient_map(
            vec![
                color_stop(0.0, 0.5, [0, 0, 0]),
                color_stop(1229.0 / 4096.0, 0.5, [255, 0, 0]),
                color_stop(1230.0 / 4096.0, 0.5, [0, 255, 0]),
                color_stop(1.0, 0.5, [255, 255, 255]),
            ],
            flat_opacities(),
            None,
            true,
        ),
        &[
            (ColorStopPosition(1), 30.0, at(1229.0)),
            (ColorStopPosition(2), 30.015, at(1230.0)),
        ],
    );
}

/// 2 値化・ポスタリゼーションは整数だけで、刻みの間が無い（注記なし）。
#[test]
fn threshold_and_posterize_have_no_between_steps_and_no_note() {
    for s in [
        AdjustmentSettings::threshold(Threshold::new(1).unwrap()),
        AdjustmentSettings::threshold(Threshold::new(128).unwrap()),
        AdjustmentSettings::posterize(Posterize::new(2).unwrap()),
        AdjustmentSettings::posterize(Posterize::new(7).unwrap()),
        AdjustmentSettings::posterize(Posterize::new(255).unwrap()),
    ] {
        let d = adjusted(s.clone());
        let (out, back) = round_trip(&d, Channel::Color);
        assert!(out.notes.is_empty(), "{:?}", out.notes);
        let a = back
            .layers()
            .iter()
            .find(|l| l.name() == "調整")
            .and_then(|l| l.adjustment())
            .unwrap();
        assert_eq!(*a, s);
    }
}

/// 値のカーブを展開した調整を書き出して読み戻し、展開の注記・停止点の数・合成の差を確かめる。
fn check_expansion(label: &str, settings: AdjustmentSettings, allowed: u8) -> (usize, usize, u8) {
    let d = adjusted(settings);
    // 厳密な書き出しは値のカーブを断る（C# の断りと対）。焼き込みは展開して書く
    assert!(
        psd::export_blockers(&d)
            .iter()
            .any(|b| b.refusal == Refusal::GradientMapCurve),
        "{label}"
    );
    let (out, back) = round_trip(&d, Channel::Color);
    let [note] = out.notes.as_slice() else {
        panic!("{label}: {:?}", out.notes)
    };
    assert_eq!(note.layer, "調整");
    let NoteAction::ExpandedGradientCurve {
        cause,
        colors,
        opacities,
        max_diff,
    } = note.action
    else {
        panic!("{label}: {:?}", note.action)
    };
    assert_eq!(cause, GradientExpansion::Curve, "{label}: 値のカーブだけ");
    // 書いた調整は値のカーブを持たず、停止点の数は注記のとおり（PSD の上限 32 以下）
    let written = back
        .layers()
        .iter()
        .find(|l| l.name() == "調整")
        .and_then(|l| l.adjustment())
        .unwrap();
    let ramp = written.gradient_map_value().unwrap().ramp();
    assert!(ramp.value_curve().is_identity(), "{label}");
    assert_eq!(
        (ramp.colors().len(), ramp.opacities().len()),
        (colors, opacities),
        "{label}"
    );
    assert!(colors <= 32 && opacities <= 32, "{label}");
    // 元の合成との最大の差は注記のとおりで、許す差以下（下の層の色によらない最悪の値で数えている。不透明度が曲線に沿って動くと 1 では
    // 足りないことがあり、そのときは 2・4 まで緩める）
    let truth = look(&d, Channel::Color)
        .iter()
        .zip(look(&back, Channel::Color))
        .map(|(a, b)| a.abs_diff(b))
        .max()
        .unwrap();
    assert_eq!(max_diff, truth, "{label}: 合成の最大の差");
    assert!(max_diff <= allowed, "{label}: {max_diff}");
    (colors, opacities, max_diff)
}

#[test]
fn a_gradient_maps_value_curve_is_expanded_into_stops_with_the_difference_in_the_note() {
    // 白黒のランプに曲線
    let (colors, opacities, _) = check_expansion(
        "白黒",
        gradient_map(
            vec![
                color_stop(0.0, 0.5, [0, 0, 0]),
                color_stop(1.0, 0.5, [255, 255, 255]),
            ],
            flat_opacities(),
            Some(&[(0.0, 0.0), (0.5, 0.7), (1.0, 1.0)]),
            false,
        ),
        1,
    );
    assert!(colors > 2, "曲線は 2 点では表せない");
    assert_eq!(opacities, 2, "不透明度は一定なので 2 点のまま");
    // 色の分岐点が 3 つ・中点・不透明度の分岐点・逆向き・S 字の曲線
    check_expansion(
        "S 字",
        gradient_map(
            vec![
                color_stop(0.0, 0.5, [20, 10, 80]),
                color_stop(0.4, 0.3, [220, 60, 30]),
                color_stop(1.0, 0.5, [250, 240, 160]),
            ],
            vec![
                opacity_stop(0.0, 1.0, 0.5),
                opacity_stop(0.6, 0.4, 0.5),
                opacity_stop(1.0, 1.0, 0.5),
            ],
            Some(&[(0.0, 0.0), (0.25, 0.1), (0.5, 0.5), (0.75, 0.9), (1.0, 1.0)]),
            true,
        ),
        2,
    );
    // 急な区間（ほとんど階段）
    check_expansion(
        "急",
        gradient_map(
            vec![
                color_stop(0.0, 0.5, [0, 0, 0]),
                color_stop(1.0, 0.5, [255, 255, 255]),
            ],
            flat_opacities(),
            Some(&[(0.0, 0.0), (0.4, 0.02), (0.45, 0.98), (1.0, 1.0)]),
            false,
        ),
        1,
    );
    // 刻みの間の位置・中点・不透明度も、展開した停止点が刻みの上に作り直す
    check_expansion(
        "刻みの間",
        gradient_map(
            vec![
                color_stop(0.0, 0.5, [10, 20, 30]),
                color_stop(0.3, 0.333, [200, 120, 40]),
                color_stop(1.0, 0.5, [250, 250, 90]),
            ],
            vec![opacity_stop(0.0, 0.5, 0.5), opacity_stop(1.0, 1.0, 0.5)],
            Some(&[(0.0, 0.0), (0.5, 0.3), (1.0, 1.0)]),
            false,
        ),
        2,
    );
}

#[test]
fn a_value_curve_that_does_not_fit_the_stop_limit_is_refused_by_layer_name() {
    // 黒と白が 32 回入れ替わるランプを曲線でゆがめる: 山と谷のすべてに停止点が要り、上限（32）では足りない
    let colors: Vec<ColorStop> = (0..32)
        .map(|i| {
            let v = if i % 2 == 0 { 0 } else { 255 };
            color_stop(f64::from(i) / 31.0, 0.5, [v, v, v])
        })
        .collect();
    let d = adjusted(gradient_map(
        colors,
        flat_opacities(),
        Some(&[(0.0, 0.0), (0.5, 0.6), (1.0, 1.0)]),
        false,
    ));
    let plan = psd::plan_export(&d, &options(Channel::Color), &ExportControl::default()).unwrap();
    assert_eq!(
        plan.blockers
            .iter()
            .map(|b| (b.layer.as_str(), b.refusal.clone()))
            .collect::<Vec<_>>(),
        [("調整", Refusal::GradientMapCurveStops)]
    );
    let err = psd::export_core(&d, &options(Channel::Color), &ExportControl::default())
        .unwrap_err()
        .to_string();
    assert!(err.contains("調整"), "{err}");
    // 値のカーブが無ければ（同じランプ）停止点が 32 個あっても、刻みへ丸めて書ける
    let plain = adjusted(gradient_map(
        (0..32)
            .map(|i| {
                let v = if i % 2 == 0 { 0 } else { 255 };
                color_stop(f64::from(i) / 31.0, 0.5, [v, v, v])
            })
            .collect(),
        flat_opacities(),
        None,
        false,
    ));
    let out = exported(&plain, Channel::Color);
    assert!(matches!(
        out.notes.as_slice(),
        [ExportNote {
            action: NoteAction::Rounded { .. },
            ..
        }]
    ));
}

/// 展開できない値のカーブ（黒と白が 32 回入れ替わるランプ + 曲線）のグラデーションマップ。
fn unfit_gradient_map() -> AdjustmentSettings {
    gradient_map(
        (0..32)
            .map(|i| {
                let v = if i % 2 == 0 { 0 } else { 255 };
                color_stop(f64::from(i) / 31.0, 0.5, [v, v, v])
            })
            .collect(),
        flat_opacities(),
        Some(&[(0.0, 0.0), (0.5, 0.6), (1.0, 1.0)]),
        false,
    )
}

fn blocked(d: &Document, channel: Channel) -> Vec<(String, Refusal)> {
    psd::plan_export(d, &options(channel), &ExportControl::default())
        .unwrap()
        .blockers
        .into_iter()
        .map(|b| (b.layer, b.refusal))
        .collect()
}

/// 展開できない値のカーブで断るのは、書き出すチャンネルの合成に出る層だけ。表示を切った層・そのチャンネルに効かない層・そのチャンネルで無効の層は
/// 合成に出ないので、最良の展開を隠した層で書く（注記の差は 0）。
#[test]
fn an_unfit_value_curve_is_refused_only_where_the_layer_shows_in_the_exported_channel() {
    let (mut d, base, _, _) = three();
    paint(&mut d, base, Channel::Roughness, 5);
    let map = d
        .add_adjustment_layer("調整", unfit_gradient_map(), Some(&[Channel::Color]), None)
        .unwrap();
    // Color の合成に出る: 断る（Color だけ。ほかのチャンネルは書ける）
    assert_eq!(
        blocked(&d, Channel::Color),
        [("調整".to_owned(), Refusal::GradientMapCurveStops)]
    );
    // Roughness にグラデーションマップは効かない: 断らず、隠した層で書く
    let (out, back) = round_trip(&d, Channel::Roughness);
    assert!(!find(&out.document.layers, "調整").visible);
    let note = out.notes.iter().find(|n| n.layer == "調整").unwrap();
    let NoteAction::ExpandedGradientCurve {
        cause,
        colors,
        opacities,
        max_diff,
    } = note.action
    else {
        panic!("{:?}", note.action)
    };
    assert_eq!(cause, GradientExpansion::Curve);
    assert_eq!(max_diff, 0, "合成に出ないので差は無い");
    assert!((2..=32).contains(&colors) && (2..=32).contains(&opacities));
    let written = back
        .layers()
        .iter()
        .find(|l| l.name() == "調整")
        .and_then(|l| l.adjustment())
        .unwrap();
    let ramp = written.gradient_map_value().unwrap().ramp();
    assert!(ramp.value_curve().is_identity());
    assert_eq!(
        (ramp.colors().len(), ramp.opacities().len()),
        (colors, opacities)
    );
    // 表示を切った層: Color の PSD でも断らない
    d.set_layer_visible(map, false).unwrap();
    assert_eq!(blocked(&d, Channel::Color), []);
    let (hidden, _) = round_trip(&d, Channel::Color);
    assert!(!find(&hidden.document.layers, "調整").visible);
    assert!(matches!(
        hidden.notes.as_slice(),
        [ExportNote {
            action: NoteAction::ExpandedGradientCurve { max_diff: 0, .. },
            ..
        }]
    ));
    // 表示を戻し、Color のチャンネルを無効にした層: 合成に出ないので断らない
    d.set_layer_visible(map, true).unwrap();
    assert_eq!(blocked(&d, Channel::Color).len(), 1);
    d.set_channel_enabled(map, Channel::Color, false).unwrap();
    assert_eq!(blocked(&d, Channel::Color), []);
    let (disabled, _) = round_trip(&d, Channel::Color);
    assert!(!find(&disabled.document.layers, "調整").visible);
    // 厳密な書き出し（C# の断りと対）は、収まる・収まらないに関わらず値のカーブを GradientMapCurve で断る。収まらない値のカーブも同じ理由
    d.set_channel_enabled(map, Channel::Color, true).unwrap();
    assert_eq!(
        psd::export_blockers(&d)
            .iter()
            .map(|b| b.refusal.clone())
            .collect::<Vec<_>>(),
        [Refusal::GradientMapCurve]
    );
}

/// 展開の差の基準（層 1 枚が通常の合成モード・不透明度 100% で当たったときの出力の差）は、文書の合成の差の上限ではない。合成モードが通常でない層でも、
/// 注記の最大の差は文書の合成の実測と一致し、基準を超えても断らない（超えるかは合成モードと下の色による）。
#[test]
fn the_expansion_note_reports_the_documents_measured_difference_in_any_blend_mode() {
    let curved = || {
        gradient_map(
            vec![
                color_stop(0.0, 0.5, [20, 10, 80]),
                color_stop(0.4, 0.3, [220, 60, 30]),
                color_stop(1.0, 0.5, [250, 240, 160]),
            ],
            vec![
                opacity_stop(0.0, 1.0, 0.5),
                opacity_stop(0.6, 0.4, 0.5),
                opacity_stop(1.0, 1.0, 0.5),
            ],
            Some(&[(0.0, 0.0), (0.25, 0.1), (0.5, 0.5), (0.75, 0.9), (1.0, 1.0)]),
            true,
        )
    };
    let (mut normal, mut worst) = (0, 0);
    for mode in [
        CoreBlend::Normal,
        CoreBlend::ColorDodge,
        CoreBlend::ColorBurn,
        CoreBlend::Divide,
        CoreBlend::Multiply,
    ] {
        let mut d = adjusted(curved());
        let id = id_of(&d, "調整");
        d.set_layer_blend_mode(id, mode).unwrap();
        let (out, back) = round_trip(&d, Channel::Color);
        let [note] = out.notes.as_slice() else {
            panic!("{mode:?}: {:?}", out.notes)
        };
        let NoteAction::ExpandedGradientCurve { max_diff, .. } = note.action else {
            panic!("{mode:?}: {:?}", note.action)
        };
        let truth = look(&d, Channel::Color)
            .iter()
            .zip(look(&back, Channel::Color))
            .map(|(a, b)| a.abs_diff(b))
            .max()
            .unwrap();
        assert_eq!(max_diff, truth, "{mode:?}: 注記の差は文書の合成の実測");
        if mode == CoreBlend::Normal {
            normal = max_diff;
        }
        worst = worst.max(max_diff);
    }
    // 基準の差が保証するのは、通常の合成モード・不透明度 100% の層の出力まで。ほかの合成モードは傾きの大きいもので差が増え、それでも断らない
    assert!(normal <= 4, "通常の合成モード: {normal}");
    assert!(
        worst > 4,
        "この曲線は、合成モードによって基準（4）を超える: {worst}"
    );
}

/// スカラーのチャンネルのトーンカーブは RGB 全体の曲線だけが効く。PSD のトーンカーブは R・G・B の曲線も当てるので、そのチャンネルの PSD には
/// 直線で書き、書いた PSD の合成がそのチャンネルの合成と一致する。Color の PSD には R・G・B の曲線も書く。
#[test]
fn a_tone_curve_in_a_scalar_channel_is_written_without_the_channel_curves_it_ignores() {
    let (mut d, _, _, _) = three();
    let base = d.layers()[0].id();
    paint(&mut d, base, Channel::Roughness, 5);
    let curves = ToneCurves::identity()
        .with_curve(
            ToneChannel::Composite,
            curve_of(&[(0.0, 0.0), (64.0 / 255.0, 100.0 / 255.0), (1.0, 1.0)]),
        )
        // Roughness には効かない R・G・B の曲線。刻みの間の値でも、そのチャンネルの PSD の断り・丸めの理由にならない
        .with_curve(
            ToneChannel::Red,
            curve_of(&[(0.0, 0.0), (0.301, 0.9), (1.0, 1.0)]),
        )
        .with_curve(ToneChannel::Blue, curve_of(&[(0.0, 0.2), (1.0, 1.0)]));
    d.add_adjustment_layer(
        "曲線",
        AdjustmentSettings::tone_curve(curves),
        Some(&[Channel::Roughness, Channel::Color]),
        None,
    )
    .unwrap();
    let (out, _) = round_trip(&d, Channel::Roughness);
    assert!(out.notes.is_empty(), "{:?}", out.notes);
    let LayerKind::Adjustment(Adjustment::ToneCurve {
        composite,
        red,
        green,
        blue,
    }) = &find(&out.document.layers, "曲線").kind
    else {
        panic!("トーンカーブで書く")
    };
    assert_eq!(composite, &vec![[0, 0], [64, 100], [255, 255]]);
    let line = vec![[0, 0], [255, 255]];
    assert_eq!((red, green, blue), (&line, &line, &line));
    // Color の PSD は R・G・B の曲線も書く（刻みの間の R は丸める）
    let (color, _) = round_trip(&d, Channel::Color);
    let [note] = color.notes.as_slice() else {
        panic!("{:?}", color.notes)
    };
    assert!(matches!(
        note.action,
        NoteAction::Rounded {
            kind: AdjustmentType::ToneCurve,
            ..
        }
    ));
    let LayerKind::Adjustment(Adjustment::ToneCurve { blue, .. }) =
        &find(&color.document.layers, "曲線").kind
    else {
        panic!("トーンカーブで書く")
    };
    assert_eq!(blue, &vec![[0, 51], [255, 255]]);
}

/// 隠した調整（表示を切った）は合成に出ないので、丸めても展開しても合成の差は 0。
#[test]
fn a_hidden_new_adjustment_that_is_rounded_or_expanded_has_no_difference() {
    let (mut d, _, _, _) = three();
    let balance = d
        .add_adjustment_layer(
            "隠すバランス",
            AdjustmentSettings::color_balance(
                ColorBalance::new([0.0, 0.0, 12.4], [0.0; 3], [0.0; 3], true).unwrap(),
            ),
            None,
            None,
        )
        .unwrap();
    let map = d
        .add_adjustment_layer(
            "隠すマップ",
            gradient_map(
                vec![
                    color_stop(0.0, 0.5, [0, 0, 0]),
                    color_stop(1.0, 0.5, [255, 255, 255]),
                ],
                flat_opacities(),
                Some(&[(0.0, 0.0), (0.5, 0.7), (1.0, 1.0)]),
                false,
            ),
            None,
            None,
        )
        .unwrap();
    d.set_layer_visible(balance, false).unwrap();
    d.set_layer_visible(map, false).unwrap();
    let out = exported(&d, Channel::Color);
    assert_eq!(
        out.notes
            .iter()
            .map(|n| rounding_diff(&n.action))
            .collect::<Vec<_>>(),
        [Some(0), Some(0)]
    );
    assert!(!find(&out.document.layers, "隠すバランス").visible);
    assert!(!find(&out.document.layers, "隠すマップ").visible);
    round_trip(&d, Channel::Color);
}

// ───────── チャンネルごと ─────────

/// チャンネルごとに値の違う文書: 土台は Color・Roughness・Height、重ねは Color だけ（Roughness の合成だけ別）、塗りつぶしは Roughness の値、
/// 調整は反転（全チャンネル）と色相/彩度（色のチャンネルだけ）、ユーザーチャンネルの層。
fn per_channel() -> (Document, Channel) {
    let mut d = Document::with_tile_size(W, H, 8).unwrap();
    let user = d
        .add_channel(ChannelInfo {
            name: "Custom".into(),
            kind: ChannelKind::Scalar,
            color_space: ColorSpace::Linear,
            default: Rgba8::new(0, 0, 0, 255),
        })
        .unwrap();
    let base = d.add_layer("下地").unwrap();
    for c in [Channel::Color, Channel::Roughness, Channel::Height, user] {
        paint(&mut d, base, c, 3 + c.index() as u32);
    }
    let tint = d.add_layer("重ね").unwrap();
    paint(&mut d, tint, Channel::Color, 11);
    d.set_layer_blend_mode(tint, CoreBlend::Multiply).unwrap();
    d.set_layer_opacity(tint, 200.0 / 255.0, false).unwrap();
    // Roughness に面を足し、そのチャンネルの合成だけ別
    paint(&mut d, tint, Channel::Roughness, 13);
    d.set_channel_blend(
        tint,
        Channel::Roughness,
        ChannelBlend::new(Some(CoreBlend::Screen), Some(100.0 / 255.0)),
        false,
    )
    .unwrap();
    d.add_fill_layer(
        "塗り",
        &[
            (Channel::Roughness, Rgba8::new(60, 60, 60, 255)),
            (Channel::Height, Rgba8::new(200, 200, 200, 255)),
        ],
        None,
    )
    .unwrap();
    d.add_adjustment_layer("反転", AdjustmentSettings::invert(), None, None)
        .unwrap();
    d.add_adjustment_layer(
        "色相",
        AdjustmentSettings::hue_saturation(40.0, 0.2, 0.0).unwrap(),
        None,
        None,
    )
    .unwrap();
    let only_user = d.add_layer("独自").unwrap();
    paint(&mut d, only_user, user, 17);
    (d, user)
}

#[test]
fn every_channel_writes_a_psd_that_composites_like_that_channel() {
    let (d, user) = per_channel();
    let before = state(&d);
    for channel in [
        Channel::Color,
        Channel::Roughness,
        Channel::Metallic,
        Channel::Height,
        Channel::Emission,
        user,
    ] {
        let (out, back) = round_trip(&d, channel);
        // 層の並び・名前は同じ。合成モード・不透明度は、そのチャンネルの値
        assert_eq!(
            names(&out.document.layers),
            ["独自", "色相", "反転", "塗り", "重ね", "下地"],
            "{channel:?}"
        );
        for l in d.layers() {
            let psd = find(&out.document.layers, l.name());
            assert_eq!(
                psd.blend_mode,
                BlendMode::ALL[l.blend_mode_in(channel) as usize],
                "{channel:?} {}",
                l.name()
            );
            assert_eq!(
                psd.opacity,
                (l.opacity_in(channel) * 255.0).round_ties_even() as u8,
                "{channel:?} {}",
                l.name()
            );
        }
        assert_eq!(back.layers().len(), d.layers().len());
    }
    assert_eq!(state(&d), before);
}

#[test]
fn layers_that_do_not_show_in_a_channel_are_written_hidden() {
    let (d, user) = per_channel();
    let shown = |out: &Exported| -> Vec<(String, bool)> {
        names(&out.document.layers)
            .into_iter()
            .map(|n| {
                let v = find(&out.document.layers, &n).visible;
                (n, v)
            })
            .collect()
    };
    let rough = exported(&d, Channel::Roughness);
    // Roughness: 下地・重ね・塗り・反転は出る。色相は色のチャンネルだけなので隠す。独自（ユーザーチャンネルだけ）は面が無いので隠す
    assert_eq!(
        shown(&rough),
        [
            ("独自", false),
            ("色相", false),
            ("反転", true),
            ("塗り", true),
            ("重ね", true),
            ("下地", true)
        ]
        .map(|(n, v)| (n.to_owned(), v))
    );
    let color = exported(&d, Channel::Color);
    assert_eq!(
        shown(&color),
        [
            ("独自", true),
            ("色相", true),
            ("反転", true),
            ("塗り", false),
            ("重ね", true),
            ("下地", true)
        ]
        .map(|(n, v)| (n.to_owned(), v)),
        "塗りつぶしは Color の値が無いので隠す。独自は Color の面（空）を持つので出る"
    );
    // 値の無い塗りつぶしの隠した単色は、チャンネルの既定の色
    assert!(matches!(
        find(&color.document.layers, "塗り").kind,
        LayerKind::SolidColor([255, 255, 255])
    ));
    let custom = exported(&d, user);
    assert!(find(&custom.document.layers, "独自").visible);
    assert!(find(&custom.document.layers, "下地").visible);
    // 面の無い層は 1×1 の透明
    let empty = find(&color.document.layers, "独自");
    assert_eq!(
        (empty.width, empty.height, empty.pixels_rgba.clone()),
        (1, 1, vec![0; 4])
    );
}

#[test]
fn a_layer_disabled_in_the_channel_is_written_hidden_with_its_pixels() {
    let (mut d, base, _, _) = three();
    d.set_channel_enabled(base, Channel::Color, false).unwrap();
    let (out, back) = round_trip(&d, Channel::Color);
    let layer = find(&out.document.layers, "土台");
    assert!(!layer.visible);
    assert!(layer.pixels_rgba.iter().any(|b| *b != 0), "画素は残す");
    assert!(back
        .layers()
        .iter()
        .any(|l| l.name() == "土台" && !l.visible()));
}

#[test]
fn a_per_channel_blend_that_differs_from_color_no_longer_blocks_a_color_export() {
    let (d, _) = per_channel();
    assert!(psd::export_blockers(&d).iter().all(|b| b.layer != "重ね"));
    let plan = psd::plan_export(&d, &options(Channel::Color), &ExportControl::default()).unwrap();
    assert!(
        plan.blockers.is_empty() && plan.notes.is_empty(),
        "{:?}",
        plan.notes
    );
}

// ───────── Normal ─────────

fn normal_doc(direction: NormalYDirection) -> Document {
    let mut d = Document::with_tile_size(W, H, 8).unwrap();
    d.set_normal_settings(
        NormalSettings::default().with_file_direction(direction),
        false,
    )
    .unwrap();
    let a = d.add_layer("法線").unwrap();
    for y in 0..H {
        for x in 0..W {
            let n = if (x / 8 + y / 8) % 2 == 0 {
                Rgba8::new(128, 128, 255, 255)
            } else {
                Rgba8::new(40 + x as u8, 200 - y as u8, 230, 255)
            };
            d.set_channel_pixel(a, Channel::Normal, x, y, n).unwrap();
        }
    }
    d.add_fill_layer(
        "塗り",
        &[(Channel::Normal, Rgba8::new(100, 60, 240, 255))],
        None,
    )
    .unwrap();
    d
}

#[test]
fn a_normal_psd_is_written_in_the_documents_file_direction() {
    for direction in [NormalYDirection::OpenGL, NormalYDirection::DirectX] {
        let d = normal_doc(direction);
        let out = exported(&d, Channel::Normal);
        let flip = direction == NormalYDirection::DirectX;
        let g = |v: u8| if flip { 255 - v } else { v };
        // ラスターの緑は向きに合わせて反転、塗りつぶしの単色も
        let a = find(&out.document.layers, "法線");
        let source = d
            .layer_output(d.layers()[0].id(), Channel::Normal, d.bounds())
            .unwrap();
        let got = expand(a, W, H);
        for (i, (s, p)) in source
            .as_chunks::<4>()
            .0
            .iter()
            .zip(got.as_chunks::<4>().0)
            .enumerate()
        {
            assert_eq!(
                [s[0], g(s[1]), s[2], s[3]],
                [p[0], p[1], p[2], p[3]],
                "{direction:?} {i}"
            );
        }
        assert!(matches!(
            find(&out.document.layers, "塗り").kind,
            LayerKind::SolidColor([100, v, 240]) if v == g(60)
        ));
        // 統合画像は、書いた層を PSD と同じ色の式で重ねたもの（読み戻した合成と同じ）。層が 2 枚あるので、法線の重ね方の注記が付く
        let back = read_back(&out.document);
        let merged = out.document.composite_rgba.as_ref().unwrap();
        let top_down = rows_reversed(&look(&back, Channel::Color), d.width() as usize * 4);
        assert_eq!(merged, &top_down, "{direction:?}");
        assert!(
            matches!(out.notes.as_slice(), [ExportNote { action: NoteAction::NormalBlend, layer }] if layer == "Normal"),
            "{:?}",
            out.notes
        );
    }
}

#[test]
fn a_flat_normal_reads_back_the_same_in_the_opengl_direction() {
    let mut d = Document::with_tile_size(W, H, 8).unwrap();
    let a = d.add_layer("平ら").unwrap();
    for y in 0..H {
        for x in 0..W {
            if x % 3 != 0 {
                d.set_channel_pixel(a, Channel::Normal, x, y, Rgba8::new(128, 128, 255, 255))
                    .unwrap();
            }
        }
    }
    let (_, back) = round_trip(&d, Channel::Normal);
    assert_eq!(back.layers().len(), 1);
}

/// 法線の層が重なると、Yolu の重ねと PSD の色の式の重ねは違う。統合画像を Yolu の合成にすると、層と食い違って、書いた PSD を Yolu が編集できる
/// PSD として読めなくなる。統合画像は書いた層を色の式で重ねたものにして、読み戻せることを確かめる（注記で知らせる）。
#[test]
fn overlapping_normals_still_read_back_as_an_editable_psd_and_say_how_they_blend() {
    for direction in [NormalYDirection::OpenGL, NormalYDirection::DirectX] {
        let mut d = Document::with_tile_size(W, H, 8).unwrap();
        d.set_normal_settings(
            NormalSettings::default().with_file_direction(direction),
            false,
        )
        .unwrap();
        for (name, shift) in [("下", 0u32), ("上", 40)] {
            let l = d.add_layer(name).unwrap();
            for y in 0..H {
                for x in 0..W {
                    let n = Rgba8::new(
                        (60 + shift + x * 3) as u8,
                        (210 - shift - y * 4) as u8,
                        (180 + (x + y) % 40) as u8,
                        if (x + y) % 5 == 0 { 120 } else { 230 },
                    );
                    d.set_channel_pixel(l, Channel::Normal, x, y, n).unwrap();
                }
            }
        }
        let out = exported(&d, Channel::Normal);
        assert!(matches!(
            out.notes.as_slice(),
            [ExportNote {
                action: NoteAction::NormalBlend,
                ..
            }]
        ));
        let back = read_back(&out.document); // 編集できる PSD として読める（診断は注記だけ）
        let top_down = rows_reversed(&look(&back, Channel::Color), W as usize * 4);
        assert_eq!(out.document.composite_rgba.as_ref().unwrap(), &top_down);
    }
    // 1 枚だけなら重ならないので、注記は付かない
    let mut one = Document::with_tile_size(W, H, 8).unwrap();
    let l = one.add_layer("一枚").unwrap();
    one.set_channel_pixel(l, Channel::Normal, 3, 3, Rgba8::new(90, 120, 240, 255))
        .unwrap();
    assert!(exported(&one, Channel::Normal).notes.is_empty());
}

#[test]
fn a_directx_normal_refuses_levels_it_cannot_carry_but_takes_invert() {
    let mut d = normal_doc(NormalYDirection::DirectX);
    d.add_adjustment_layer("反転", AdjustmentSettings::invert(), None, None)
        .unwrap();
    assert!(exported(&d, Channel::Normal)
        .notes
        .iter()
        .all(|n| n.action == NoteAction::NormalBlend));
    d.add_adjustment_layer(
        "レベル",
        AdjustmentSettings::levels(
            20.0 / 255.0,
            230.0 / 255.0,
            1.37,
            10.0 / 255.0,
            240.0 / 255.0,
        )
        .unwrap(),
        None,
        None,
    )
    .unwrap();
    let plan = psd::plan_export(&d, &options(Channel::Normal), &ExportControl::default()).unwrap();
    assert_eq!(plan.blockers.len(), 1);
    assert_eq!(plan.blockers[0].layer, "レベル");
    assert_eq!(plan.blockers[0].refusal, Refusal::NormalLevels);
    assert!(plan.build(&d, &ExportControl::default()).is_err());
    // OpenGL の向きなら、レベル補正は書ける
    let mut gl = normal_doc(NormalYDirection::OpenGL);
    gl.add_adjustment_layer(
        "レベル",
        AdjustmentSettings::levels(
            20.0 / 255.0,
            230.0 / 255.0,
            1.37,
            10.0 / 255.0,
            240.0 / 255.0,
        )
        .unwrap(),
        None,
        None,
    )
    .unwrap();
    assert!(exported(&gl, Channel::Normal)
        .notes
        .iter()
        .all(|n| n.action == NoteAction::NormalBlend));
}

/// Normal のチャンネルに、焼き込みの機能を全部入れた文書（下から）: 土台（そのまま）・フィルターつきのラスター（反転とフィルターと濃度つきのマスク）・
/// 半透明の塗りつぶし・クリッピングされたグループ（中は 2 枚のラスター。反転したマスクと不透明度つき）。
fn normal_features(direction: NormalYDirection) -> Document {
    let mut d = Document::with_tile_size(W, H, 8).unwrap();
    d.set_normal_settings(
        NormalSettings::default().with_file_direction(direction),
        false,
    )
    .unwrap();
    d.set_filter_block_pixels(16).unwrap();
    let base = d.add_layer("土台").unwrap();
    paint(&mut d, base, Channel::Normal, 1);
    let soft = d.add_layer("ぼかし").unwrap();
    paint(&mut d, soft, Channel::Normal, 2);
    d.add_filter(
        soft,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(3)).channels(&[Channel::Normal]),
    )
    .unwrap();
    d.add_layer_mask(soft).unwrap();
    for y in [4, 9, 15] {
        for x in 0..W {
            d.set_mask_pixel(soft, x, y, (x * 6 + 10) as u8).unwrap();
        }
    }
    d.set_layer_mask_inverted(soft, true).unwrap();
    d.add_filter(
        soft,
        FilterTarget::Mask,
        FilterSpec::new(EffectSettings::noise(0.6, 3, true)),
    )
    .unwrap();
    d.set_layer_mask_density(soft, 200.0 / 255.0, false)
        .unwrap();
    d.add_fill_layer(
        "半透明",
        &[(Channel::Normal, Rgba8::new(100, 60, 240, 128))],
        None,
    )
    .unwrap();
    let inner_low = d.add_layer("覆い下").unwrap();
    paint(&mut d, inner_low, Channel::Normal, 3);
    let inner_high = d.add_layer("覆い上").unwrap();
    paint(&mut d, inner_high, Channel::Normal, 4);
    let group = d.group_layers(&[inner_low, inner_high], "覆い").unwrap();
    d.set_layer_clipping(group, true).unwrap();
    d.set_layer_opacity(group, 180.0 / 255.0, false).unwrap();
    d.add_layer_mask(group).unwrap();
    for y in [6, 12] {
        for x in 0..W {
            d.set_mask_pixel(group, x, y, (255 - x * 5) as u8).unwrap();
        }
    }
    d.set_layer_mask_inverted(group, true).unwrap();
    d
}

/// 緑を反転する（`quiet` した値の向きを替える。透明な画素は RGB を持たないので触らない）。
fn flipped(mut v: Vec<u8>) -> Vec<u8> {
    for p in v.as_chunks_mut::<4>().0 {
        if p[3] != 0 {
            p[1] = 255 - p[1]
        }
    }
    v
}

fn id_of(d: &Document, name: &str) -> LayerId {
    d.layers().iter().find(|l| l.name() == name).unwrap().id()
}

/// 焼き込みの機能は、Normal のチャンネルでも、書いた層の画素が文書の向き（OpenGL）の値から、ファイルの向きに合わせて並ぶ（DirectX は緑を反転）。
/// フィルターつきのラスター・半透明の塗りつぶし・クリッピングされたグループ（1 枚に焼く）のどれも。マスクは向きにもチャンネルにも依らない。
#[test]
fn every_baked_feature_lands_in_a_normal_psd_in_the_documents_file_direction() {
    let gl = normal_features(NormalYDirection::OpenGL);
    let dx = normal_features(NormalYDirection::DirectX);
    let before = (state(&gl), state(&dx));
    let out_gl = exported(&gl, Channel::Normal);
    let out_dx = exported(&dx, Channel::Normal);
    let color = exported(&gl, Channel::Color);

    // 注記: 焼いたものと、層が重なる所の注意
    for out in [&out_gl, &out_dx] {
        let notes = notes_of(out);
        let mask_noise = EffectSettings::noise(0.6, 3, true);
        for (layer, action) in [
            (
                "ぼかし",
                NoteAction::BakedFilters(vec![EffectSettings::blur(3)]),
            ),
            ("ぼかし", NoteAction::BakedInvertedMask),
            ("ぼかし", NoteAction::BakedMaskFilters(vec![mask_noise])),
            ("半透明", NoteAction::BakedTranslucentFill),
            ("覆い", NoteAction::BakedClippedGroup),
            ("覆い", NoteAction::BakedInvertedMask),
            ("Normal", NoteAction::NormalBlend),
        ] {
            assert!(notes.contains(&(layer, &action)), "{layer} {action:?}");
        }
        assert_eq!(
            names(&out.document.layers),
            ["覆い", "半透明", "ぼかし", "土台"],
            "並びは変えない。グループは 1 枚になる"
        );
    }

    // 画素: OpenGL は文書の値のまま、DirectX は緑だけ反転
    for name in ["土台", "ぼかし", "半透明", "覆い"] {
        let source = |d: &Document| {
            let id = id_of(d, name);
            if name == "覆い" {
                d.group_output(id, Channel::Normal, d.bounds()).unwrap()
            } else {
                d.layer_output(id, Channel::Normal, d.bounds()).unwrap()
            }
        };
        let want = quiet(source(&gl));
        assert_eq!(want, quiet(source(&dx)), "{name}: 文書の値は向きに依らない");
        assert!(
            want.as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[3] != 0 && p[1] != 128),
            "{name}: 緑が反転で変わる画素がある"
        );
        let layer_gl = find(&out_gl.document.layers, name);
        let layer_dx = find(&out_dx.document.layers, name);
        if name == "半透明" {
            assert!(
                matches!(layer_gl.kind, LayerKind::Raster),
                "半透明は画素に焼く"
            );
        }
        assert_eq!(quiet(expand(layer_gl, W, H)), want, "OpenGL {name}");
        assert_eq!(
            quiet(expand(layer_dx, W, H)),
            flipped(want),
            "DirectX {name}"
        );
        assert_eq!(
            attrs(layer_gl),
            attrs(layer_dx),
            "{name}: 属性は向きに依らない"
        );
        if name != "覆い" {
            assert_eq!(
                attrs(layer_gl),
                expected_attrs(&gl, id_of(&gl, name), Channel::Normal, true),
                "{name}"
            );
        }
    }
    let group = find(&out_dx.document.layers, "覆い");
    assert_eq!(
        (group.opacity, group.clipping, group.blend_mode),
        (180, true, BlendMode::Normal)
    );

    // マスク: 向きにもチャンネルにも依らない（Color の PSD の同じ層のマスクと同じ）
    for name in ["ぼかし", "覆い"] {
        let mask = find(&out_gl.document.layers, name).mask.as_ref();
        assert!(mask.is_some(), "{name}");
        assert_eq!(
            mask,
            find(&out_dx.document.layers, name).mask.as_ref(),
            "{name}"
        );
        assert_eq!(
            mask,
            find(&color.document.layers, name).mask.as_ref(),
            "{name}"
        );
    }

    // どちらの向きも、編集できる PSD として読み戻せ、統合画像は書いた層を重ねたものと同じ
    for out in [&out_gl, &out_dx] {
        let back = read_back(&out.document);
        assert_eq!(back.layers().iter().filter(|l| l.is_group()).count(), 0);
        assert_eq!(
            out.document.composite_rgba.as_ref().unwrap(),
            &rows_reversed(&look(&back, Channel::Color), W as usize * 4)
        );
    }
    assert_same_from_a_snapshot(&gl, Channel::Normal, &out_gl);
    assert_same_from_a_snapshot(&dx, Channel::Normal, &out_dx);
    assert_eq!((state(&gl), state(&dx)), before, "文書は変わらない");
}

/// 層を書いたあとの統合画像（PSD の色の式の重ね）も、取消の旗に従う。大きな画布の Normal で、重ねている途中に旗を立てると、重ね終わるのを待たずに
/// `Cancelled` で戻る（旗を見ない重ねは、取消が効かない時間が画布の大きさに比例して長くなる）。
#[test]
fn a_cancel_during_the_normal_composite_of_a_big_canvas_stops_it_without_a_result() {
    use std::time::{Duration, Instant};
    let mut d = Document::new(4096, 4096).unwrap();
    for (name, v) in [("塗りA", 100), ("塗りB", 140)] {
        d.add_fill_layer(
            name,
            &[(Channel::Normal, Rgba8::new(v, 60, 240, 255))],
            None,
        )
        .unwrap();
    }
    let run = |cancel_after: Option<Duration>| {
        let flag = AtomicBool::new(false);
        let started = Instant::now();
        let (result, flag_raised) = std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                let ctl = ExportControl {
                    cancel: Some(&flag),
                    ..ExportControl::default()
                };
                psd::plan_export(&d, &options(Channel::Normal), &ctl)?.build(&d, &ctl)
            });
            let mut raised = None;
            if let Some(after) = cancel_after {
                std::thread::sleep(after);
                flag.store(true, std::sync::atomic::Ordering::Relaxed);
                raised = Some(Instant::now());
            }
            (worker.join().unwrap(), raised)
        });
        (result, started.elapsed(), flag_raised.map(|t| t.elapsed()))
    };
    let (full, whole, _) = run(None);
    assert!(full.is_ok(), "旗が立たなければ書ける");
    assert!(
        whole > Duration::from_millis(40),
        "重ねに時間がかかる画布でなければ、この試験は意味が無い: {whole:?}"
    );
    let (cut, _, since_raised) = run(Some(whole / 5));
    assert!(
        matches!(
            cut,
            Err(yolu_io::Error::Core(yolu_core::CoreError::Cancelled))
        ),
        "{:?}",
        cut.map(|o| o.notes)
    );
    // 旗を立ててから戻るまでが、全部を重ねる時間より十分に短い（行ごとに見る）
    assert!(
        since_raised.unwrap() < whole / 2,
        "旗のあとも重ね続けた: {:?} / {whole:?}",
        since_raised
    );
}

// ───────── 平らに 1 枚 ─────────

#[test]
fn flat_mode_writes_the_composite_as_one_layer_in_any_channel() {
    let (mut d, user) = per_channel();
    let first = d.layers()[0].id();
    blur(&mut d, first, &[Channel::Color]);
    for channel in [Channel::Color, Channel::Roughness, user] {
        let ctl = ExportControl::default();
        let plan =
            psd::plan_export(&d, &ExportOptions::new(channel, ExportMode::Flat), &ctl).unwrap();
        assert!(plan.notes.is_empty() && plan.blockers.is_empty());
        let out = plan.build(&d, &ctl).unwrap();
        assert_eq!(out.document.layers.len(), 1);
        let back = read_back(&out.document);
        assert_eq!(back.layers().len(), 1);
        assert_eq!(look(&d, channel), look(&back, Channel::Color));
    }
}

// ───────── 文書・予算・取消 ─────────

#[test]
fn a_stroke_in_progress_refuses_and_nothing_changes() {
    let (mut d, _, _, top) = three();
    let stroke = d
        .begin_stroke(top, &yolu_core::BrushSettings::default())
        .unwrap();
    assert!(psd::plan_export(&d, &options(Channel::Color), &ExportControl::default()).is_err());
    d.cancel_stroke(stroke);
    assert!(psd::plan_export(&d, &options(Channel::Color), &ExportControl::default()).is_ok());
    // 文書に無いチャンネル
    let gone = Channel::from_index(40).unwrap();
    assert!(psd::plan_export(&d, &options(gone), &ExportControl::default()).is_err());
}

#[test]
fn baked_layers_count_against_the_pixel_budget_and_are_refused_with_the_reason() {
    // 4096² の全面の層を 3 枚焼くと、画素の予算（128 MiB）を超える。1 枚の焼き込みで画布 1 枚ぶんを作り、足し上げて断る
    let mut d = Document::new(4096, 4096).unwrap();
    for n in 0..3 {
        d.add_fill_layer(
            &format!("ガラス{n}"),
            &[(Channel::Color, Rgba8::new(10, 20, 30, 100))],
            None,
        )
        .unwrap();
    }
    let ctl = ExportControl::default();
    let plan = psd::plan_export(&d, &options(Channel::Color), &ctl).unwrap();
    assert_eq!(plan.notes.len(), 3);
    let err = plan.build(&d, &ctl).unwrap_err();
    assert!(matches!(err, yolu_io::Error::Budget(_)), "{err}");
    assert!(err.to_string().contains("予算"), "{err}");
}

#[test]
fn a_cancel_flag_stops_planning_and_building_without_a_result() {
    let (mut d, _, _, _) = three();
    d.add_adjustment_layer(
        "丸め",
        AdjustmentSettings::levels(0.3, 1.0, 1.0, 0.0, 1.0).unwrap(),
        None,
        None,
    )
    .unwrap();
    let (base, _) = (d.layers()[0].id(), ());
    blur(&mut d, base, &[Channel::Color]);
    let stop = AtomicBool::new(true);
    let ctl = ExportControl {
        cancel: Some(&stop),
        ..ExportControl::default()
    };
    let is_cancelled =
        |e: yolu_io::Error| matches!(e, yolu_io::Error::Core(yolu_core::CoreError::Cancelled));
    assert!(is_cancelled(
        psd::plan_export(&d, &options(Channel::Color), &ctl).unwrap_err()
    ));
    let plan = psd::plan_export(&d, &options(Channel::Color), &ExportControl::default()).unwrap();
    assert!(is_cancelled(plan.build(&d, &ctl).unwrap_err()));
    assert!(is_cancelled(
        psd::export_core(&d, &options(Channel::Color), &ctl).unwrap_err()
    ));
}

/// 取り込んだ層（PSD → core）を書き出し直しても、焼くものが無ければ今までの書き出しとバイト一致。
#[test]
fn a_document_with_nothing_to_bake_writes_the_same_bytes_as_the_strict_export() {
    let (mut d, base, _, top) = three();
    d.add_layer_mask(base).unwrap();
    d.set_layer_mask_enabled(base, true).unwrap();
    d.set_layer_clipping(top, true).unwrap();
    d.add_fill_layer("単色", &[(Channel::Color, Rgba8::new(1, 2, 3, 255))], None)
        .unwrap();
    let strict = Psd::from_core(&d).unwrap();
    let out = exported(&d, Channel::Color);
    assert!(out.notes.is_empty());
    assert_eq!(out.document, strict);
    assert_eq!(
        psd::write(&out.document, &Limits::default()).unwrap(),
        psd::write(&strict, &Limits::default()).unwrap()
    );
}

/// 不透明度が 1/255 の刻みの外（0.5）の層を重ねても、書いた PSD は編集できる PSD として読める（統合画像は Yolu の合成で、層の値は刻みに丸めるので、
/// 数段重なっても差が読みの許容（1）に収まる）。
#[test]
fn off_step_opacities_still_read_back_as_an_editable_psd() {
    let mut d = Document::with_tile_size(W, H, 8).unwrap();
    for (name, seed) in [("a", 1u32), ("b", 2), ("c", 3)] {
        let l = d.add_layer(name).unwrap();
        paint(&mut d, l, Channel::Color, seed);
        d.set_layer_opacity(l, 0.5, false).unwrap();
    }
    let out = exported(&d, Channel::Color);
    let _ = read_back(&out.document);
}

// ───────── 乱数で組んだ文書 ─────────

/// 種から 1 つの文書を組む: ラスター・塗りつぶし・調整・グループを重ね、層ごとに不透明度（1/255 の刻み）・合成モード・表示・クリッピング・
/// チャンネルごとの合成・マスク（反転・フィルター）・内容のフィルター・Generator・Anchor を乱数で付ける。PSD の刻みに乗る調整だけ（丸めは別の試験）。
fn random_document(seed: u64) -> (Document, Channel) {
    let mut rng = Rng(seed);
    let mut d = Document::with_tile_size(W, H, 8).unwrap();
    d.set_filter_block_pixels(16).unwrap();
    d.set_effect_inputs(inputs(0)).unwrap();
    let user = d
        .add_channel(ChannelInfo {
            name: "Custom".into(),
            kind: ChannelKind::Scalar,
            color_space: ColorSpace::Linear,
            default: Rgba8::new(0, 0, 0, 255),
        })
        .unwrap();
    let channels = [
        Channel::Color,
        Channel::Roughness,
        Channel::Height,
        Channel::Normal,
        user,
    ];
    let modes = [
        CoreBlend::Normal,
        CoreBlend::Multiply,
        CoreBlend::Screen,
        CoreBlend::Overlay,
        CoreBlend::Difference,
    ];
    let steps = [255u32, 200, 128, 64, 255, 255];
    let mut ids: Vec<LayerId> = Vec::new();
    let mut anchors: Vec<AnchorId> = Vec::new();
    for n in 0..3 + rng.below(5) {
        let id = match rng.below(10) {
            0 | 1 => {
                let translucent = rng.below(2) == 0;
                let a = if translucent {
                    90 + rng.below(100) as u8
                } else {
                    255
                };
                let id = d
                    .add_fill_layer(
                        &format!("塗り{n}"),
                        &[
                            (
                                Channel::Color,
                                Rgba8::new(rng.below(256) as u8, 120, 200, a),
                            ),
                            (Channel::Roughness, Rgba8::new(90, 90, 90, a)),
                            (Channel::Normal, Rgba8::new(100, 60, 240, a)),
                            (user, Rgba8::new(70, 70, 70, a)),
                        ],
                        None,
                    )
                    .unwrap();
                if rng.below(3) == 0 {
                    d.set_fill_image(id, Channel::Color, Some(image(0)))
                        .unwrap();
                }
                id
            }
            2 => {
                let settings = match rng.below(14) {
                    0 => AdjustmentSettings::invert(),
                    1 => AdjustmentSettings::levels(
                        20.0 / 255.0,
                        230.0 / 255.0,
                        1.37,
                        10.0 / 255.0,
                        240.0 / 255.0,
                    )
                    .unwrap(),
                    2 => AdjustmentSettings::hue_saturation(-73.0, 0.42, -0.18).unwrap(),
                    // 刻みの間（PSD の刻みへ丸める。読み戻した合成は丸めた設定の合成と比べる）
                    3 => AdjustmentSettings::levels(0.3, 0.9, 1.234, 0.05, 0.97).unwrap(),
                    4 => AdjustmentSettings::hue_saturation(10.5, 0.333, -0.123).unwrap(),
                    // 色調補正の 6 種（刻みの上のものと、刻みの間・値のカーブのもの）
                    5 => AdjustmentSettings::tone_curve(ToneCurves::identity().with_curve(
                        ToneChannel::Composite,
                        curve_of(&[(0.0, 0.0), (64.0 / 255.0, 100.0 / 255.0), (1.0, 1.0)]),
                    )),
                    6 => AdjustmentSettings::tone_curve(
                        ToneCurves::identity()
                            .with_curve(
                                ToneChannel::Composite,
                                curve_of(&[(0.0, 0.05), (0.301, 0.31), (0.702, 0.65), (1.0, 0.97)]),
                            )
                            .with_curve(
                                ToneChannel::Green,
                                curve_of(&[(0.0, 0.0), (0.02, 0.101), (0.04, 0.202), (1.0, 1.0)]),
                            ),
                    ),
                    7 => AdjustmentSettings::color_balance(
                        ColorBalance::new(
                            [10.0, -20.0, 0.0],
                            [0.0, 30.0, -15.0],
                            [5.0, 0.0, 40.0],
                            true,
                        )
                        .unwrap(),
                    ),
                    8 => AdjustmentSettings::color_balance(
                        ColorBalance::new(
                            [10.4, -20.6, 0.0],
                            [0.5, 30.2, -15.5],
                            [5.0, 0.0, 40.0],
                            false,
                        )
                        .unwrap(),
                    ),
                    9 => AdjustmentSettings::brightness_contrast(
                        BrightnessContrast::new(rng.below(60) as f64 - 20.0 + 0.5, 20.0).unwrap(),
                    ),
                    10 => AdjustmentSettings::threshold(
                        Threshold::new(1 + rng.below(255) as u32).unwrap(),
                    ),
                    11 => AdjustmentSettings::posterize(
                        Posterize::new(2 + rng.below(8) as u32).unwrap(),
                    ),
                    12 => gradient_map(
                        vec![
                            color_stop(0.0, 0.5, [20, 10, 80]),
                            color_stop(0.3, 0.333, [220, 60, 30]),
                            color_stop(1.0, 0.5, [250, 240, 160]),
                        ],
                        vec![opacity_stop(0.0, 1.0, 0.5), opacity_stop(1.0, 0.5, 0.5)],
                        None,
                        rng.below(2) == 0,
                    ),
                    _ => gradient_map(
                        vec![
                            color_stop(0.0, 0.5, [20, 10, 80]),
                            color_stop(1.0, 0.5, [250, 240, 160]),
                        ],
                        flat_opacities(),
                        Some(&[(0.0, 0.0), (0.5, 0.7), (1.0, 1.0)]),
                        rng.below(2) == 0,
                    ),
                };
                d.add_adjustment_layer(&format!("調整{n}"), settings, None, None)
                    .unwrap()
            }
            _ => {
                let id = d.add_layer(&format!("層{n}")).unwrap();
                for c in channels {
                    if c == Channel::Color || rng.below(2) == 0 {
                        paint(
                            &mut d,
                            id,
                            c,
                            (seed as u32).wrapping_add(n as u32 * 7 + c.index() as u32),
                        );
                    }
                }
                id
            }
        };
        let layer_kind = d.layer(id).unwrap().kind();
        d.set_layer_opacity(id, f64::from(steps[rng.below(steps.len())]) / 255.0, false)
            .unwrap();
        if layer_kind != CoreKind::Adjustment {
            d.set_layer_blend_mode(id, modes[rng.below(modes.len())])
                .unwrap();
        } else if rng.below(3) == 0 {
            d.set_layer_blend_mode(id, CoreBlend::Multiply).unwrap();
        }
        if rng.below(10) == 0 {
            d.set_layer_visible(id, false).unwrap();
        }
        if rng.below(10) < 3 && !ids.is_empty() {
            d.set_layer_clipping(id, true).unwrap();
        }
        // そのチャンネルでは有効でない層（そのチャンネルの PSD では隠した層になる）
        if rng.below(10) < 2 {
            let off = [Channel::Roughness, Channel::Normal, Channel::Height, user];
            d.set_channel_enabled(id, off[rng.below(off.len())], false)
                .unwrap();
        }
        if rng.below(10) < 2 {
            d.set_channel_blend(
                id,
                Channel::Roughness,
                ChannelBlend::new(Some(CoreBlend::Screen), Some(128.0 / 255.0)),
                false,
            )
            .unwrap();
        }
        if rng.below(10) < 3 {
            d.add_layer_mask(id).unwrap();
            for y in (rng.below(H as usize) as u32..H).step_by(5) {
                for x in 0..W {
                    d.set_mask_pixel(id, x, y, (x * 6) as u8).unwrap();
                }
            }
            if rng.below(2) == 0 {
                d.set_layer_mask_inverted(id, true).unwrap();
            }
            if rng.below(3) == 0 {
                d.add_filter(
                    id,
                    FilterTarget::Mask,
                    FilterSpec::new(EffectSettings::noise(0.5, rng.below(9) as i32, true)),
                )
                .unwrap();
            }
            if rng.below(6) == 0 {
                d.set_layer_mask_density(id, 128.0 / 255.0, false).unwrap();
            }
            if rng.below(8) == 0 {
                d.set_layer_mask_enabled(id, false).unwrap();
            }
        }
        if matches!(layer_kind, CoreKind::Raster | CoreKind::Fill) {
            match rng.below(7) {
                0 => {
                    blur(&mut d, id, &[Channel::Color]);
                }
                4 => {
                    blur(&mut d, id, &[Channel::Normal]);
                }
                1 => {
                    d.add_filter(
                        id,
                        FilterTarget::Content,
                        FilterSpec::new(EffectSettings::noise(0.3, 4, true))
                            .channels(&[Channel::Color, Channel::Height]),
                    )
                    .unwrap();
                }
                2 => {
                    let f = d
                        .add_filter(
                            id,
                            FilterTarget::Content,
                            FilterSpec::new(EffectSettings::blur(2))
                                .channels(&[Channel::Roughness]),
                        )
                        .unwrap();
                    if rng.below(2) == 0 {
                        d.set_filter_enabled(id, f, false).unwrap();
                    }
                }
                3 => {
                    let mut wear = Settings::new(generator::Kind::EdgeWear);
                    wear.blend = generator::Blend::Multiply;
                    d.add_filter(
                        id,
                        FilterTarget::Content,
                        FilterSpec::new(EffectSettings::generator(wear))
                            .channels(&[Channel::Color, Channel::Height]),
                    )
                    .unwrap();
                }
                _ => {}
            }
            if rng.below(6) == 0 {
                if let Ok(a) = d.add_anchor(id, AnchorPlacement::Layer, None, None) {
                    anchors.push(a);
                }
            }
        }
        if layer_kind == CoreKind::Raster && !anchors.is_empty() && rng.below(5) == 0 {
            let mut g = Settings::new(generator::Kind::Anchor);
            g.blend = generator::Blend::Multiply;
            if let Ok(stage) = d.add_filter(
                id,
                FilterTarget::Content,
                FilterSpec::new(EffectSettings::generator(g)).channels(&[Channel::Color]),
            ) {
                let a = anchors[rng.below(anchors.len())];
                let _ = d.set_generator_anchor(
                    id,
                    stage,
                    Some(a),
                    Channel::Color,
                    ReadMode::Value,
                    false,
                );
            }
        }
        ids.push(id);
        // ときどき直前の数枚をグループにする（クリッピングされたグループも）
        if ids.len() >= 2 && rng.below(4) == 0 {
            let take = 2.min(ids.len());
            let members: Vec<LayerId> = ids.split_off(ids.len() - take);
            if members
                .iter()
                .all(|m| d.layer(*m).unwrap().parent().is_none())
            {
                let g = d.group_layers(&members, &format!("組{n}")).unwrap();
                match rng.below(4) {
                    0 => d.set_layer_clipping(g, true).unwrap(),
                    1 => d.set_layer_blend_mode(g, CoreBlend::Multiply).unwrap(),
                    _ => {}
                }
                d.set_layer_opacity(g, f64::from(steps[rng.below(steps.len())]) / 255.0, false)
                    .unwrap();
                ids.push(g);
            } else {
                ids.extend(members);
            }
        }
    }
    (d, user)
}

/// 書き出して読み戻した合成が、書き出したチャンネルの今の合成と違うか。
fn mismatch(d: &Document, channel: Channel) -> bool {
    let ctl = ExportControl::default();
    let Ok(plan) = psd::plan_export(d, &options(channel), &ctl) else {
        return false;
    };
    let Ok(out) = plan.build(d, &ctl) else {
        return false;
    };
    let bytes = psd::write(&out.document, &Limits::default()).unwrap();
    let Ok(back) = psd::read(&bytes, &Limits::default()).unwrap().to_core() else {
        return false;
    };
    let tolerance = u8::from(has_inverted_mask(d));
    rounded_look(d, channel, &out, &back)
        .iter()
        .zip(&look(&back, Channel::Color))
        .any(|(a, b)| a.abs_diff(*b) > tolerance)
}

/// 食い違う文書から層を 1 つずつ外して、食い違いが残るかぎり小さくし、層の一覧にして返す（食い違ったときの調べ物の道具）。
fn minimal_mismatch(d: &Document, channel: Channel) -> String {
    let mut d = d.capture_snapshot().unwrap();
    'again: loop {
        for l in d.layers().to_vec() {
            let mut t = d.capture_snapshot().unwrap();
            if t.remove_layer(l.id()).is_ok() && !t.layers().is_empty() && mismatch(&t, channel) {
                d = t;
                continue 'again;
            }
        }
        break;
    }
    let mut out = format!("{channel:?}:");
    for (i, l) in d.layers().iter().enumerate() {
        out += &format!(
            "\n  {i} {:?} {:?} parent={:?} visible={} opacity={} mode={:?} clip={} mask={:?} filters={:?} fill={:?}",
            l.name(),
            l.kind(),
            l.parent().and_then(|p| d.layer_index(p)),
            l.visible(),
            l.opacity_in(channel),
            l.blend_mode_in(channel),
            l.clipping(),
            l.mask().map(|m| (m.enabled(), m.inverted(), m.density(), m.filters().len())),
            l.filters().iter().map(|f| f.settings().name()).collect::<Vec<_>>(),
            l.fill_value(channel),
        );
    }
    out
}

/// PSD の層を、グループの中も含めて上から下へ（グループの次にその中身）並べる。
fn flat(layers: &[Layer]) -> Vec<&Layer> {
    let mut out = Vec::new();
    for l in layers {
        out.push(l);
        if let LayerKind::Group { children, .. } = &l.kind {
            out.extend(flat(children))
        }
    }
    out
}

/// 名前の層（グループの中も）。無ければ None（グループに焼かれた層など）。
fn find_opt<'a>(layers: &'a [Layer], name: &str) -> Option<&'a Layer> {
    layers.iter().find_map(|l| find_in(l, name))
}

/// Normal は、どちらのファイルの向きでも、書いた層の画素が文書の値から並び（OpenGL はそのまま）、読み戻せて、統合画像は書いた層を重ねたものと同じ。
/// DirectX は OpenGL の書き出しと緑だけが違う（層・マスク・属性・並びは同じ）。DirectX のレベル補正は、PSD の 1 つのレベル補正で書けないので断る。
/// 層が重なる所の合成は Yolu の法線の重ね方と違い得るので、チャンネルの合成との一致は言わない（注記で知らせる）。
fn check_normal(d: &Document, label: &str) {
    let with = |direction| {
        let mut copy = d.capture_snapshot().unwrap();
        copy.set_normal_settings(
            NormalSettings::default().with_file_direction(direction),
            false,
        )
        .unwrap();
        copy
    };
    let (gl, dx) = (
        with(NormalYDirection::OpenGL),
        with(NormalYDirection::DirectX),
    );
    let ctl = ExportControl::default();
    let plan_gl = psd::plan_export(&gl, &options(Channel::Normal), &ctl).unwrap();
    assert!(
        plan_gl.blockers.is_empty(),
        "{label}: {:?}",
        plan_gl.blockers
    );
    let out_gl = plan_gl.build(&gl, &ctl).unwrap();
    // OpenGL: 層の画素は、文書の層（グループは焼いた合成）の出力と同じ
    for l in gl.layers() {
        let Some(written) = find_opt(&out_gl.document.layers, l.name()) else {
            continue; // クリッピングされたグループの中身は、グループの 1 枚になった
        };
        if !written.visible || !matches!(written.kind, LayerKind::Raster) {
            continue;
        }
        let source = if l.is_group() {
            gl.group_output(l.id(), Channel::Normal, gl.bounds())
        } else {
            gl.layer_output(l.id(), Channel::Normal, gl.bounds())
        }
        .unwrap();
        assert_eq!(
            quiet(expand(written, W, H)),
            quiet(source),
            "{label}: OpenGL の層「{}」",
            l.name()
        );
    }
    let back = read_back(&out_gl.document);
    assert_eq!(
        out_gl.document.composite_rgba.as_ref().unwrap(),
        &rows_reversed(&look(&back, Channel::Color), W as usize * 4),
        "{label}: OpenGL の統合画像は書いた層の重ね"
    );
    // DirectX: レベル補正が効く層があれば断る。無ければ、OpenGL と緑だけが違う
    let plan_dx = psd::plan_export(&dx, &options(Channel::Normal), &ctl).unwrap();
    if !plan_dx.blockers.is_empty() {
        assert!(
            plan_dx
                .blockers
                .iter()
                .all(|b| b.refusal == Refusal::NormalLevels),
            "{label}: {:?}",
            plan_dx.blockers
        );
        assert!(plan_dx.build(&dx, &ctl).is_err(), "{label}");
        return;
    }
    let out_dx = plan_dx.build(&dx, &ctl).unwrap();
    assert_eq!(out_gl.notes, out_dx.notes, "{label}: 注記は向きに依らない");
    let (a, b) = (flat(&out_gl.document.layers), flat(&out_dx.document.layers));
    assert_eq!(a.len(), b.len(), "{label}");
    for (x, y) in a.iter().zip(&b) {
        let what = format!("{label}: 「{}」", x.name);
        assert_eq!(attrs(x), attrs(y), "{what}");
        assert_eq!(x.mask, y.mask, "{what}: マスクは向きに依らない");
        assert_eq!(
            (x.id, x.left, x.top, x.width, x.height),
            (y.id, y.left, y.top, y.width, y.height),
            "{what}"
        );
        match (&x.kind, &y.kind) {
            (LayerKind::Raster, LayerKind::Raster) => assert_eq!(
                quiet(y.pixels_rgba.clone()),
                flipped(quiet(x.pixels_rgba.clone())),
                "{what}: DirectX は緑だけ反転"
            ),
            (LayerKind::SolidColor(p), LayerKind::SolidColor(q)) => {
                assert_eq!(*q, [p[0], 255 - p[1], p[2]], "{what}")
            }
            (LayerKind::Group { divider_id: p, .. }, LayerKind::Group { divider_id: q, .. }) => {
                assert_eq!(p, q, "{what}")
            }
            (p, q) => assert_eq!(p, q, "{what}"),
        }
    }
    let back = read_back(&out_dx.document);
    assert_eq!(
        out_dx.document.composite_rgba.as_ref().unwrap(),
        &rows_reversed(&look(&back, Channel::Color), W as usize * 4),
        "{label}: DirectX の統合画像は書いた層の重ね"
    );
}

/// 丸めた調整が 1 つだけなら、注記の「合成の最大の差」は、書き出したチャンネルの今の合成と丸めた設定の合成の最大の差と同じ。
fn assert_rounding_difference(d: &Document, channel: Channel, out: &Exported, back: &Document) {
    let rounded: Vec<&ExportNote> = out
        .notes
        .iter()
        .filter(|n| is_rounding(&n.action))
        .collect();
    if let [note] = rounded.as_slice() {
        let max_diff = rounding_diff(&note.action).expect("丸めの注記");
        let (now, rounded) = (look(d, channel), rounded_look(d, channel, out, back));
        let worst = now.iter().zip(&rounded).map(|(a, b)| a.abs_diff(*b)).max();
        assert_eq!(
            Some(max_diff),
            worst,
            "{channel:?}「{}」の最大の差",
            note.layer
        );
    }
}

/// 乱数で組んだ文書は、どのチャンネル（Color・Roughness・Height・ユーザー）でも、書き出して読み戻した合成が書き出したチャンネルの今の合成
/// （刻みの間の調整は丸めた設定の合成）と全バイト一致し、丸めた差は注記のとおりで、文書は変わらず、文書の写しから書いても同じ PSD になる。
/// Normal は上の合成の一致を言わず（層が重なる所は色の式）、`check_normal` の性質を、OpenGL と DirectX の両方で確かめる。
/// 食い違ったら、層を外して小さくした文書を言う。
#[test]
fn random_documents_bake_to_the_same_look_in_every_channel() {
    let mut baked = 0;
    let mut rounded = 0;
    // 種の数は環境変数 YOLU_PSD_SEEDS で増やせる（既定 300）
    let seeds: u64 = std::env::var("YOLU_PSD_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    for seed in 0..seeds {
        let (d, user) = random_document(seed * 7919 + 1);
        let before = state(&d);
        for channel in [Channel::Color, Channel::Roughness, Channel::Height, user] {
            assert!(
                !mismatch(&d, channel),
                "seed {seed}: {}",
                minimal_mismatch(&d, channel)
            );
            // 反転したマスクがある文書は、浮動小数の最後の桁で 1 だけずれ得る（それ以外は全バイト一致）
            let (out, back) = round_trip_within(&d, channel, u8::from(has_inverted_mask(&d)));
            baked += out.notes.len();
            rounded += out.notes.iter().filter(|n| is_rounding(&n.action)).count();
            assert_rounding_difference(&d, channel, &out, &back);
            assert_same_from_a_snapshot(&d, channel, &out);
        }
        check_normal(&d, &format!("seed {seed}"));
        assert_eq!(state(&d), before, "seed {seed}");
    }
    assert!(
        baked > seeds as usize * 2 / 3,
        "焼く・落とすものが十分にある: {baked}"
    );
    assert!(
        rounded > seeds as usize / 4,
        "刻みの間の調整が十分にある: {rounded}"
    );
}

/// 混色（混色モード・混合率曲線）を持つグラデーションマップ。
fn mixing_map(
    colors: Vec<ColorStop>,
    mode: generator::MixMode,
    correction: generator::LuminanceCorrection,
    segment: Option<(usize, &[(f64, f64)])>,
) -> AdjustmentSettings {
    let mut ramp = Ramp::new(colors, flat_opacities(), None)
        .unwrap()
        .with_mixing(mode, correction);
    if let Some((k, points)) = segment {
        let curve = Curve::new(points.iter().map(|&(x, y)| CurvePoint { x, y }).collect()).unwrap();
        ramp = ramp.with_segment_curve(k, Some(curve)).unwrap();
    }
    AdjustmentSettings::gradient_map(GradientMap::new(ramp, false))
}

#[test]
fn a_gradient_maps_mixing_is_refused_strictly_and_expanded_into_stops_when_baking() {
    use generator::{LuminanceCorrection, MixMode};
    let three_colors = || {
        vec![
            color_stop(0.0, 0.5, [20, 10, 120]),
            color_stop(0.5, 0.4, [220, 60, 30]),
            color_stop(1.0, 0.5, [250, 240, 160]),
        ]
    };
    let cases = [
        (
            "知覚的",
            mixing_map(
                three_colors(),
                MixMode::Perceptual,
                LuminanceCorrection::High,
                None,
            ),
        ),
        (
            "リニア",
            mixing_map(
                three_colors(),
                MixMode::Linear,
                LuminanceCorrection::default(),
                None,
            ),
        ),
        (
            "混合率曲線",
            mixing_map(
                three_colors(),
                MixMode::Standard,
                LuminanceCorrection::default(),
                Some((0, &[(0.0, 0.0), (0.3, 0.8), (1.0, 1.0)])),
            ),
        ),
    ];
    for (label, settings) in cases {
        let d = adjusted(settings);
        // 厳密な書き出しは、混色があれば断る（PSD のグラデーションに形が無い）
        assert!(
            psd::export_blockers(&d)
                .iter()
                .any(|b| b.refusal == Refusal::GradientMapMixing),
            "{label}"
        );
        // 焼き込みは停止点へ展開して書く
        let (out, back) = round_trip(&d, Channel::Color);
        let [note] = out.notes.as_slice() else {
            panic!("{label}: {:?}", out.notes)
        };
        assert_eq!(note.layer, "調整");
        let NoteAction::ExpandedGradientCurve {
            cause,
            colors,
            opacities,
            max_diff,
        } = note.action
        else {
            panic!("{label}: {:?}", note.action)
        };
        assert_eq!(
            cause,
            GradientExpansion::Mixing,
            "{label}: 混色だけ（値のカーブは無い）"
        );
        let written = back
            .layers()
            .iter()
            .find(|l| l.name() == "調整")
            .and_then(|l| l.adjustment())
            .unwrap();
        let ramp = written.gradient_map_value().unwrap().ramp();
        assert!(
            !ramp.uses_mixing(),
            "{label}: 書いた調整は PSD の形（混色なし）"
        );
        assert!(ramp.value_curve().is_identity(), "{label}");
        assert_eq!(
            (ramp.colors().len(), ramp.opacities().len()),
            (colors, opacities)
        );
        assert!(
            colors > 3 && colors <= 32,
            "{label}: 停止点が増える {colors}"
        );
        let truth = look(&d, Channel::Color)
            .iter()
            .zip(look(&back, Channel::Color))
            .map(|(a, b)| a.abs_diff(b))
            .max()
            .unwrap();
        assert_eq!(max_diff, truth, "{label}: 合成の最大の差は注記のとおり");
        assert!(max_diff <= 4, "{label}: {max_diff}");
    }
}

#[test]
fn a_gradient_map_with_a_value_curve_and_mixing_says_both_in_the_note() {
    use generator::{LuminanceCorrection, MixMode};
    let bent = Curve::new(vec![
        CurvePoint { x: 0.0, y: 0.0 },
        CurvePoint { x: 0.5, y: 0.7 },
        CurvePoint { x: 1.0, y: 1.0 },
    ])
    .unwrap();
    let ramp = Ramp::new(
        vec![
            color_stop(0.0, 0.5, [20, 10, 120]),
            color_stop(0.5, 0.4, [220, 60, 30]),
            color_stop(1.0, 0.5, [250, 240, 160]),
        ],
        flat_opacities(),
        None,
    )
    .unwrap()
    .with_value_curve(bent)
    .with_mixing(MixMode::Perceptual, LuminanceCorrection::default());
    let d = adjusted(AdjustmentSettings::gradient_map(GradientMap::new(
        ramp, false,
    )));
    // 厳密な書き出しは、値のカーブで断る（混色の断りより先）
    assert!(psd::export_blockers(&d)
        .iter()
        .any(|b| b.refusal == Refusal::GradientMapCurve));
    let (out, _) = round_trip(&d, Channel::Color);
    let [note] = out.notes.as_slice() else {
        panic!("{:?}", out.notes)
    };
    let NoteAction::ExpandedGradientCurve { cause, .. } = note.action else {
        panic!("{:?}", note.action)
    };
    assert_eq!(cause, GradientExpansion::CurveAndMixing);
    assert!(
        note.message().contains("値のカーブと混色"),
        "{}",
        note.message()
    );
}

/// 展開の注記の文は、使っているものだけを言う（値のカーブだけなら混色の語は出ない。混色だけならカーブの語は出ない）。
#[test]
fn the_expansion_note_names_only_what_the_gradient_map_used() {
    let note = |cause| ExportNote {
        layer: "調整".to_owned(),
        action: NoteAction::ExpandedGradientCurve {
            cause,
            colors: 5,
            opacities: 2,
            max_diff: 1,
        },
    };
    let curve = note(GradientExpansion::Curve).message();
    assert!(
        curve.contains("値のカーブを") && !curve.contains("混色"),
        "{curve}"
    );
    let mixing = note(GradientExpansion::Mixing).message();
    assert!(
        mixing.contains("混色") && !mixing.contains("カーブ"),
        "{mixing}"
    );
    let both = note(GradientExpansion::CurveAndMixing).message();
    assert!(both.contains("値のカーブと混色"), "{both}");
}

#[test]
fn a_gradient_map_without_mixing_is_written_as_before() {
    // 混色を使わなければ、値のカーブも無い昔のグラデーションマップは展開せず、そのまま PSD の停止点にする
    let d = adjusted(gradient_map(
        vec![
            color_stop(0.0, 0.5, [20, 10, 120]),
            color_stop(1.0, 0.5, [250, 240, 160]),
        ],
        flat_opacities(),
        None,
        false,
    ));
    assert!(psd::export_blockers(&d).is_empty());
    let (out, _) = round_trip(&d, Channel::Color);
    assert!(out.notes.is_empty(), "{:?}", out.notes);
}
