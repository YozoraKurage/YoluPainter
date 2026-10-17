//! 取り込んだブラシで「表せなかった項目」の名前の一覧。
//!
//! 取り込みの読み手（`yolu_io::brushes`）は、表せない・近似した・読めなかった設定を、値つきの注記（`Unrepresented`）で返す。
//! 画面に出すのは文ではなく項目の名前だけなので、注記を項目に畳む（値は捨てる。名前は言語ごと）。畳んだ項目は ID の文字で
//! ブラシのファイルに残す（`import.gaps`）ので、読み戻しても印と一覧が変わらない。知らない ID は読み飛ばす（情報だけで、
//! 描き方には関わらない）。注記の種類を足すと、ここの対応がコンパイルで止まる（黙って項目なしにしない）。
//! ファイルから写せた項目（`SutMapped`）も、同じく ID の文字でブラシのファイルに残す（`import.mapped`）。取り込みの結果をそのまま残すので、
//! 取り込んだあとに利用者が傾きや筆圧を入れても、ファイルから写した項目には並ばない。

use yolu_io::brushes::{DualNote, SutInput, SutMapped, SutNote, TextureNote, Unrepresented};

use crate::lang::Lang;

use super::ImportMeta;

/// 表せなかった項目。並びは一覧に出す順。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Gap {
    ColorTip,
    HoseSelection,
    HoseDimensions,
    HoseCells,
    TrailingData,
    TipEdge,
    Tip16Bit,
    PresetSettings,
    PatternSection,
    Sections,
    TipKind,
    MinimumDiameter,
    Controls,
    ForegroundBackground,
    FadeLength,
    ScatterAxis,
    CountJitter,
    Noise,
    WetEdges,
    MixerBrush,
    TexturePattern,
    TextureScale,
    TextureMode,
    TextureEachTip,
    TextureDepth,
    TextureBrightness,
    TextureContrast,
    PatternConversion,
    DualTip,
    DualMode,
    DualScatter,
    DualCount,
    DualFlip,
    ImageResolution,
    TipImage,
    TipOrder,
    TextureRotation,
    TipDirection,
    ColorMixing,
    Spray,
    DualBrush,
    StartEnd,
    Stabilizer,
    ColorChange,
    BlendMode,
    PressureCurve,
    TiltCurve,
    StartEndDetail,
    StabilizerStrength,
    PostCorrection,
    VectorTools,
    TipFlip,
    TiltMax,
    SettingsTable,
    AntiAliasing,
    LengthUnit,
    IntervalType,
    ReferenceLines,
}

impl Gap {
    pub const ALL: [Gap; 58] = [
        Gap::ColorTip,
        Gap::HoseSelection,
        Gap::HoseDimensions,
        Gap::HoseCells,
        Gap::TrailingData,
        Gap::TipEdge,
        Gap::Tip16Bit,
        Gap::PresetSettings,
        Gap::PatternSection,
        Gap::Sections,
        Gap::TipKind,
        Gap::MinimumDiameter,
        Gap::Controls,
        Gap::ForegroundBackground,
        Gap::FadeLength,
        Gap::ScatterAxis,
        Gap::CountJitter,
        Gap::Noise,
        Gap::WetEdges,
        Gap::MixerBrush,
        Gap::TexturePattern,
        Gap::TextureScale,
        Gap::TextureMode,
        Gap::TextureEachTip,
        Gap::TextureDepth,
        Gap::TextureBrightness,
        Gap::TextureContrast,
        Gap::PatternConversion,
        Gap::DualTip,
        Gap::DualMode,
        Gap::DualScatter,
        Gap::DualCount,
        Gap::DualFlip,
        Gap::ImageResolution,
        Gap::TipImage,
        Gap::TipOrder,
        Gap::TextureRotation,
        Gap::TipDirection,
        Gap::ColorMixing,
        Gap::Spray,
        Gap::DualBrush,
        Gap::StartEnd,
        Gap::Stabilizer,
        Gap::ColorChange,
        Gap::BlendMode,
        Gap::PressureCurve,
        Gap::TiltCurve,
        Gap::StartEndDetail,
        Gap::StabilizerStrength,
        Gap::PostCorrection,
        Gap::VectorTools,
        Gap::TipFlip,
        Gap::TiltMax,
        Gap::SettingsTable,
        Gap::AntiAliasing,
        Gap::LengthUnit,
        Gap::IntervalType,
        Gap::ReferenceLines,
    ];

    /// ファイルに書く名前。
    pub fn id(self) -> &'static str {
        match self {
            Gap::ColorTip => "color-tip",
            Gap::HoseSelection => "hose-selection",
            Gap::HoseDimensions => "hose-dimensions",
            Gap::HoseCells => "hose-cells",
            Gap::TrailingData => "trailing-data",
            Gap::TipEdge => "tip-edge",
            Gap::Tip16Bit => "tip-16-bit",
            Gap::PresetSettings => "preset-settings",
            Gap::PatternSection => "pattern-section",
            Gap::Sections => "sections",
            Gap::TipKind => "tip-kind",
            Gap::MinimumDiameter => "minimum-diameter",
            Gap::Controls => "controls",
            Gap::ForegroundBackground => "foreground-background",
            Gap::FadeLength => "fade-length",
            Gap::ScatterAxis => "scatter-axis",
            Gap::CountJitter => "count-jitter",
            Gap::Noise => "noise",
            Gap::WetEdges => "wet-edges",
            Gap::MixerBrush => "mixer-brush",
            Gap::TexturePattern => "texture-pattern",
            Gap::TextureScale => "texture-scale",
            Gap::TextureMode => "texture-mode",
            Gap::TextureEachTip => "texture-each-tip",
            Gap::TextureDepth => "texture-depth",
            Gap::TextureBrightness => "texture-brightness",
            Gap::TextureContrast => "texture-contrast",
            Gap::PatternConversion => "pattern-conversion",
            Gap::DualTip => "dual-tip",
            Gap::DualMode => "dual-mode",
            Gap::DualScatter => "dual-scatter",
            Gap::DualCount => "dual-count",
            Gap::DualFlip => "dual-flip",
            Gap::ImageResolution => "image-resolution",
            Gap::TipImage => "tip-image",
            Gap::TipOrder => "tip-order",
            Gap::TextureRotation => "texture-rotation",
            Gap::TipFlip => "tip-flip",
            Gap::ReferenceLines => "reference-lines",
            Gap::TipDirection => "tip-direction",
            Gap::ColorMixing => "color-mixing",
            Gap::Spray => "spray",
            Gap::DualBrush => "dual-brush",
            Gap::StartEnd => "start-end",
            Gap::Stabilizer => "stabilizer",
            Gap::ColorChange => "color-change",
            Gap::BlendMode => "blend-mode",
            Gap::PressureCurve => "pressure-curve",
            Gap::TiltCurve => "tilt-curve",
            Gap::TiltMax => "tilt-max",
            Gap::SettingsTable => "settings-table",
            Gap::AntiAliasing => "anti-aliasing",
            Gap::LengthUnit => "length-unit",
            Gap::IntervalType => "interval-type",
            Gap::StartEndDetail => "start-end-detail",
            Gap::StabilizerStrength => "stabilizer-strength",
            Gap::PostCorrection => "post-correction",
            Gap::VectorTools => "vector-tools",
        }
    }

    pub fn from_id(id: &str) -> Option<Gap> {
        Gap::ALL.into_iter().find(|g| g.id() == id)
    }

    /// 項目の名前（短い名詞句。文にしない）。
    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            Gap::ColorTip => lang.t("brushes-colored-tip-text"),
            Gap::HoseSelection => lang.t("brushes-cell-selection"),
            Gap::HoseDimensions => lang.t("brushes-multi-dimensional-hose"),
            Gap::HoseCells => lang.t("brushes-hose-cell-count"),
            Gap::TrailingData => lang.t("brushes-trailing-data"),
            Gap::TipEdge => lang.t("brushes-tip-edge"),
            Gap::Tip16Bit => lang.t("brushes-16-bit-tip-text"),
            Gap::PresetSettings => lang.t("brushes-brush-settings"),
            Gap::PatternSection => lang.t("brushes-pattern-section"),
            Gap::Sections => lang.t("brushes-unsupported-sections"),
            Gap::TipKind => lang.t("brushes-tip-kind"),
            Gap::MinimumDiameter => lang.t("brushes-minimum-diameter"),
            Gap::Controls => lang.t("brushes-controls"),
            Gap::ForegroundBackground => lang.t("brushes-foreground-background-control"),
            Gap::FadeLength => lang.t("brushes-fade-length"),
            Gap::ScatterAxis => lang.t("brushes-one-axis-scatter"),
            Gap::CountJitter => lang.t("brushes-count-jitter"),
            Gap::Noise => lang.t("brushes-noise"),
            Gap::WetEdges => lang.t("brushes-wet-edges"),
            Gap::MixerBrush => lang.t("brushes-mixer-brush"),
            Gap::TexturePattern => lang.t("brushes-texture-pattern"),
            Gap::TextureScale => lang.t("brushes-texture-scale"),
            Gap::TextureMode => lang.t("brushes-texture-mode"),
            Gap::TextureEachTip => lang.t("brushes-texture-per-dab"),
            Gap::TextureDepth => lang.t("brushes-texture-depth-jitter"),
            Gap::TextureBrightness => lang.t("brushes-texture-brightness"),
            Gap::TextureContrast => lang.t("brushes-texture-contrast"),
            Gap::PatternConversion => lang.t("brushes-pattern-conversion"),
            Gap::DualTip => lang.t("brushes-dual-brush-tip-text"),
            Gap::DualMode => lang.t("brushes-dual-brush-mode"),
            Gap::DualScatter => lang.t("brushes-dual-brush-scatter"),
            Gap::DualCount => lang.t("brushes-dual-brush-count"),
            Gap::DualFlip => lang.t("brushes-dual-brush-flip"),
            Gap::ImageResolution => lang.t("brushes-image-resolution"),
            Gap::TipImage => lang.t("brushes-tip-image"),
            Gap::TipOrder => lang.t("brushes-tip-order"),
            Gap::TextureRotation => lang.t("brushes-texture-rotation"),
            Gap::TipFlip => lang.t("brushes-tip-flip"),
            Gap::ReferenceLines => lang.t("brushes-reference-lines"),
            Gap::TipDirection => lang.t("brushes-tip-direction"),
            Gap::ColorMixing => lang.t("brushes-color-mixing"),
            Gap::Spray => lang.t("brushes-spray"),
            Gap::DualBrush => lang.t("brushes-dual-brush"),
            Gap::StartEnd => lang.t("brushes-start-and-end"),
            Gap::Stabilizer => lang.t("brushes-stabilization"),
            Gap::ColorChange => lang.t("brushes-color-change"),
            Gap::BlendMode => lang.t("brushes-blend-mode"),
            Gap::PressureCurve => lang.t("brushes-pressure-curve"),
            Gap::TiltCurve => lang.t("brushes-tilt-curve"),
            Gap::TiltMax => lang.t("brushes-tilt-max"),
            Gap::SettingsTable => lang.t("brushes-settings-table"),
            Gap::AntiAliasing => lang.t("brushes-anti-aliasing"),
            Gap::LengthUnit => lang.t("brushes-length-unit"),
            Gap::IntervalType => lang.t("brushes-interval-type"),
            Gap::StartEndDetail => lang.t("brushes-start-end-speed-and-ratio"),
            Gap::StabilizerStrength => lang.t("brushes-stabilization-strength"),
            Gap::PostCorrection => lang.t("brushes-post-correction"),
            Gap::VectorTools => lang.t("brushes-vector-eraser-and-magnet"),
        }
    }
}

/// 注記 1 つが指す項目。取り込んだ模様を使えず読み飛ばした注記（`PatternSkipped`）などファイル全体の数の注記は、どのブラシの項目でも
/// ないので None（取り込めなかった数として別に数える）。
pub fn of_note(note: &Unrepresented) -> Option<Gap> {
    use Unrepresented as U;
    Some(match note {
        U::ColorTipAsMask => Gap::ColorTip,
        U::HoseSelection { .. } => Gap::HoseSelection,
        U::HoseDimensions { .. } => Gap::HoseDimensions,
        U::HoseShort { .. } => Gap::HoseCells,
        U::GbrTrailingData { .. } => Gap::TrailingData,
        U::VbrShapeRendered { .. } => Gap::TipEdge,
        U::Tip16Bit => Gap::Tip16Bit,
        U::PresetsUnreadable(_) => Gap::PresetSettings,
        U::PatternsUnreadable(_) => Gap::PatternSection,
        U::SectionSkipped(_) | U::MoreSectionsSkipped(_) => Gap::Sections,
        U::UnknownTipKind(_) => Gap::TipKind,
        U::MinimumDiameter(_) => Gap::MinimumDiameter,
        U::Control { .. } => Gap::Controls,
        U::ForegroundBackgroundControl(_) => Gap::ForegroundBackground,
        U::FadeRange { .. } => Gap::FadeLength,
        U::ScatterOneAxis => Gap::ScatterAxis,
        U::CountJitter => Gap::CountJitter,
        U::Noise => Gap::Noise,
        U::WetEdges => Gap::WetEdges,
        U::MixerBrush => Gap::MixerBrush,
        U::Texture(note) => match note {
            TextureNote::PatternMissing { .. } | TextureNote::PatternRefused { .. } => {
                Gap::TexturePattern
            }
            TextureNote::ScaleClamped { .. } => Gap::TextureScale,
            TextureNote::Mode(_) => Gap::TextureMode,
            TextureNote::EachTip => Gap::TextureEachTip,
            TextureNote::DepthDynamics => Gap::TextureDepth,
            TextureNote::Brightness => Gap::TextureBrightness,
            TextureNote::Contrast => Gap::TextureContrast,
        },
        U::TexturePattern(_) | U::Pattern(_) => Gap::PatternConversion,
        U::Dual(note) => match note {
            DualNote::MissingTip | DualNote::TipNotInFile | DualNote::UnknownTipKind(_) => {
                Gap::DualTip
            }
            DualNote::Mode(_) => Gap::DualMode,
            DualNote::ScatterOneAxis => Gap::DualScatter,
            DualNote::CountJitter => Gap::DualCount,
            DualNote::Flip => Gap::DualFlip,
        },
        U::ClipStudio(note) => match note {
            SutNote::PreviewImage => Gap::ImageResolution,
            SutNote::TipMissing | SutNote::TipGuessed | SutNote::ProprietaryImage => Gap::TipImage,
            SutNote::TipOrder | SutNote::TipOrderApprox => Gap::TipOrder,
            SutNote::TipFlip => Gap::TipFlip,
            SutNote::TextureToneApprox => Gap::TextureBrightness,
            SutNote::TextureMissing | SutNote::TextureGuessed => Gap::TexturePattern,
            SutNote::TextureRotation => Gap::TextureRotation,
            SutNote::TextureBrightness => Gap::TextureBrightness,
            SutNote::TextureContrast => Gap::TextureContrast,
            SutNote::TextureMode => Gap::TextureMode,
            SutNote::TextureEachTip => Gap::TextureEachTip,
            SutNote::Direction | SutNote::ThicknessDirection => Gap::TipDirection,
            SutNote::ColorMixing { .. }
            | SutNote::ColorMixingMethod(_)
            | SutNote::ColorMixingMode(_)
            | SutNote::LuminanceCorrection(_)
            | SutNote::MixBlur(_) => Gap::ColorMixing,
            SutNote::PostCorrection => Gap::PostCorrection,
            SutNote::VectorTools => Gap::VectorTools,
            SutNote::Spray | SutNote::SprayApprox(_) => Gap::Spray,
            SutNote::DualBrush | SutNote::DualApprox(_) => Gap::DualBrush,
            SutNote::StartEnd | SutNote::StartEndTargets(_) => Gap::StartEnd,
            SutNote::Stabilizer => Gap::Stabilizer,
            SutNote::ColorChange | SutNote::ColorApprox(_) => Gap::ColorChange,
            SutNote::BlendMode => Gap::BlendMode,
            SutNote::InfluenceCurve { .. } | SutNote::InfluenceUnreadable(_) => Gap::Controls,
            SutNote::CurveSimplified(_, SutInput::Tilt) => Gap::TiltCurve,
            SutNote::CurveSimplified(..) => Gap::PressureCurve,
            SutNote::TiltCurve(_) => Gap::TiltCurve,
            SutNote::TiltMax(_) => Gap::TiltMax,
            SutNote::StartEndDetail => Gap::StartEndDetail,
            SutNote::StabilizerStrength => Gap::StabilizerStrength,
            SutNote::SettingsMissing => Gap::SettingsTable,
            SutNote::AntiAliasing(_) => Gap::AntiAliasing,
            SutNote::LengthUnit => Gap::LengthUnit,
            SutNote::IntervalType(_) => Gap::IntervalType,
            SutNote::WaterEdgeUnit | SutNote::WaterEdgeRange => Gap::PresetSettings,
            SutNote::ExpandMode(_) | SutNote::ExpandUnit(_) | SutNote::ReferenceRange => {
                Gap::ReferenceLines
            }
            // ファイル全体の数。どのブラシの項目でもない（使えなかった筆先・質感は、そのブラシの `TipMissing`・`TextureMissing` が
            // 項目になり、読まなかったブラシは取り込めなかった数に入れる。素材が上限を超えて読み切れなければ、並びで当てる推定をしない
            // ので、そのブラシの筆先・質感が欠けたことは `TipMissing`・`TextureMissing` に出る）
            SutNote::MaterialsUnreadable(_)
            | SutNote::MaterialsCapped
            | SutNote::BrushesCapped(_) => return None,
        },
        U::PatternSkipped { .. } => return None,
    })
}

/// 写せた項目（`SutMapped`）の並び。ブラシのファイルへ書く順。
pub const MAPPED_ALL: [SutMapped; 26] = [
    SutMapped::TipImage,
    SutMapped::Texture,
    SutMapped::Pressure,
    SutMapped::Tilt,
    SutMapped::StartEnd,
    SutMapped::Stabilizer,
    SutMapped::TipAngle,
    SutMapped::AngleRandom,
    SutMapped::AngleSource,
    SutMapped::ThicknessDirection,
    SutMapped::MinOnePixel,
    SutMapped::SizeByScreen,
    SutMapped::ColorMixing,
    SutMapped::AntiAliasing,
    SutMapped::Speed,
    SutMapped::Random,
    SutMapped::TipOrder,
    SutMapped::TipFlip,
    SutMapped::TipDarken,
    SutMapped::TextureRotation,
    SutMapped::TextureTone,
    SutMapped::ColorChange,
    SutMapped::Spray,
    SutMapped::DualBrush,
    SutMapped::WaterEdge,
    SutMapped::ReferenceLines,
];

/// 写せた項目のファイルに書く名前（項目を足すと、ここの対応がコンパイルで止まる）。
pub fn mapped_id(item: SutMapped) -> &'static str {
    match item {
        SutMapped::TipImage => "tip-image",
        SutMapped::Texture => "texture",
        SutMapped::Pressure => "pressure",
        SutMapped::Tilt => "tilt",
        SutMapped::StartEnd => "start-end",
        SutMapped::Stabilizer => "stabilizer",
        SutMapped::TipAngle => "tip-angle",
        SutMapped::AngleRandom => "angle-random",
        SutMapped::AngleSource => "angle-source",
        SutMapped::ThicknessDirection => "thickness-direction",
        SutMapped::MinOnePixel => "min-one-pixel",
        SutMapped::SizeByScreen => "size-by-screen",
        SutMapped::ColorMixing => "color-mixing",
        SutMapped::AntiAliasing => "anti-aliasing",
        SutMapped::Speed => "speed",
        SutMapped::Random => "random",
        SutMapped::TipOrder => "tip-order",
        SutMapped::TipFlip => "tip-flip",
        SutMapped::TipDarken => "tip-darken",
        SutMapped::TextureRotation => "texture-rotation",
        SutMapped::TextureTone => "texture-tone",
        SutMapped::ColorChange => "color-change",
        SutMapped::Spray => "spray",
        SutMapped::DualBrush => "dual-brush",
        SutMapped::WaterEdge => "water-edge",
        SutMapped::ReferenceLines => "reference-lines",
    }
}

/// ファイルの名前から写せた項目へ（知らない名前は None。新しい版が足した項目は読み飛ばす）。
pub fn mapped_from_id(id: &str) -> Option<SutMapped> {
    MAPPED_ALL.into_iter().find(|m| mapped_id(*m) == id)
}

/// 一覧の行のツールチップ: ブラシの名前（変更ありの印つき）、取り込んだ出どころ、ファイルから写した項目、表せなかった項目。
/// 写した項目と表せなかった項目は、どちらも取り込みのときに決めてファイルに残した一覧をそのまま出す（今のブラシの設定からは導かない）。
pub fn row_tooltip(lang: Lang, name: &str, modified: bool, import: Option<&ImportMeta>) -> String {
    let mut tooltip = if modified {
        lang.f("brushes-modified-tip", &[("name", &name)])
    } else {
        name.to_owned()
    };
    let Some(meta) = import else {
        return tooltip;
    };
    let mut push = |text: &str| {
        tooltip.push('\n');
        tooltip.push_str(text);
    };
    let join = |names: Vec<&str>| names.join(lang.t("brushes-text-tip"));
    if !meta.source.is_empty() {
        push(&meta.source);
    }
    if !meta.mapped.is_empty() {
        let names = meta.mapped.iter().map(|m| {
            let (ja, en) = m.texts();
            lang.pick(ja, en)
        });
        push(&format!(
            "{}{}",
            lang.t("brushes-carried-over-tip"),
            join(names.collect())
        ));
    }
    if !meta.gaps.is_empty() {
        push(&format!(
            "{}{}",
            lang.t("brushes-not-represented-tip"),
            join(meta.gaps.iter().map(|g| g.name(lang)).collect())
        ));
    }
    tooltip
}

/// 注記の並びを、重ならない項目の一覧（`Gap` の並び）にする。
pub fn fold<'a>(notes: impl IntoIterator<Item = &'a Unrepresented>) -> Vec<Gap> {
    let mut gaps: Vec<Gap> = notes.into_iter().filter_map(of_note).collect();
    gaps.sort();
    gaps.dedup();
    gaps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_gap_has_a_distinct_id_and_a_name_in_both_languages() {
        let mut ids: Vec<&str> = Gap::ALL.iter().map(|g| g.id()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), Gap::ALL.len());
        let mut sorted = Gap::ALL;
        sorted.sort();
        assert_eq!(sorted, Gap::ALL, "ALL は並びの順");
        for gap in Gap::ALL {
            assert_eq!(Gap::from_id(gap.id()), Some(gap));
            for lang in Lang::ALL {
                let name = gap.name(lang);
                assert!(
                    !name.is_empty() && !name.ends_with('。') && !name.ends_with('.'),
                    "{gap:?}"
                );
            }
            assert!(
                !Gap::name(gap, Lang::En).chars().any(|c| c > '\u{7f}'),
                "{gap:?}: 英語に日本語"
            );
        }
        assert_eq!(Gap::from_id("no-such-gap"), None);
    }

    #[test]
    fn settings_notes_of_a_sut_fold_to_their_own_items_and_the_old_name_is_still_read() {
        let one = |note| of_note(&Unrepresented::ClipStudio(note));
        assert_eq!(one(SutNote::SettingsMissing), Some(Gap::SettingsTable));
        assert_eq!(one(SutNote::AntiAliasing(9.0)), Some(Gap::AntiAliasing));
        assert_eq!(one(SutNote::LengthUnit), Some(Gap::LengthUnit));
        assert_eq!(one(SutNote::IntervalType(9.0)), Some(Gap::IntervalType));
        // ABR の読めないブラシの設定は、前の名前の項目のまま。前のファイルの `preset-settings` も読み続ける
        assert_eq!(
            of_note(&Unrepresented::PresetsUnreadable(
                yolu_io::brushes::Fault::NotPng
            )),
            Some(Gap::PresetSettings)
        );
        assert_eq!(Gap::from_id("preset-settings"), Some(Gap::PresetSettings));
        assert_eq!(Gap::PresetSettings.id(), "preset-settings");
    }

    #[test]
    fn every_carried_item_has_a_distinct_id_and_a_name_in_both_languages() {
        let mut ids: Vec<&str> = MAPPED_ALL.iter().map(|m| mapped_id(*m)).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), MAPPED_ALL.len());
        let mut sorted = MAPPED_ALL;
        sorted.sort();
        assert_eq!(sorted, MAPPED_ALL, "並びの順");
        for item in MAPPED_ALL {
            assert_eq!(mapped_from_id(mapped_id(item)), Some(item));
            let (ja, en) = item.texts();
            assert!(
                !ja.is_empty() && !en.is_empty() && en.is_ascii(),
                "{item:?}"
            );
        }
        assert_eq!(mapped_from_id("no-such-item"), None);
    }

    #[test]
    fn the_tooltip_lists_the_source_then_what_was_carried_then_what_was_left_out() {
        let meta = ImportMeta::new(
            "CLIP STUDIO .sut".into(),
            false,
            vec![Gap::Spray, Gap::Noise],
        )
        .with_mapped(vec![SutMapped::Pressure]);
        assert_eq!(
            row_tooltip(Lang::En, "A", false, Some(&meta)),
            "A\nCLIP STUDIO .sut\nCarried over: pen pressure\nNot represented: Noise, Spray"
        );
        // 何も写せていない・何も表せなかった行は、その行を出さない
        let bare = ImportMeta::new("GIMP GBR".into(), false, vec![]);
        assert_eq!(
            row_tooltip(Lang::En, "B", true, Some(&bare)),
            "B (modified)\nGIMP GBR"
        );
        assert_eq!(row_tooltip(Lang::Ja, "C", false, None), "C");
    }

    #[test]
    fn notes_fold_into_sorted_distinct_items_and_skipped_patterns_are_not_items() {
        let notes = [
            Unrepresented::WetEdges,
            Unrepresented::ColorTipAsMask,
            Unrepresented::WetEdges,
            Unrepresented::Texture(TextureNote::EachTip),
            Unrepresented::PatternSkipped {
                name: "x".into(),
                reason: yolu_io::brushes::PatternRefusal::Mode(
                    yolu_io::brushes::PatternMode::Other(9),
                ),
            },
        ];
        assert_eq!(
            fold(&notes),
            [Gap::ColorTip, Gap::WetEdges, Gap::TextureEachTip]
        );
        assert_eq!(fold(&[]), Vec::<Gap>::new());
    }

    #[test]
    fn clip_studio_guessed_and_missing_images_fold_into_the_image_items() {
        let sut = |n: SutNote| Unrepresented::ClipStudio(n);
        // 当てずっぽうの推定も、欠けた筆先も「筆先の画像」の項目、質感の推定は「質感の模様」の項目。ファイル全体の数は項目にしない
        let notes = [
            sut(SutNote::TipGuessed),
            sut(SutNote::TipMissing),
            sut(SutNote::TextureGuessed),
            sut(SutNote::MaterialsUnreadable(2)),
            sut(SutNote::BrushesCapped(3)),
        ];
        assert_eq!(fold(&notes), [Gap::TexturePattern, Gap::TipImage]);
    }
}
