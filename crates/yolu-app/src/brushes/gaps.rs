//! 取り込んだブラシで「表せなかった項目」の名前の一覧。
//!
//! 取り込みの読み手（`yolu_io::brushes`）は、表せない・近似した・読めなかった設定を、値つきの注記（`Unrepresented`）で返す。
//! 画面に出すのは文ではなく項目の名前だけなので、注記を項目に畳む（値は捨てる。名前は言語ごと）。畳んだ項目は ID の文字で
//! ブラシのファイルに残す（`import.gaps`）ので、読み戻しても印と一覧が変わらない。知らない ID は読み飛ばす（情報だけで、
//! 描き方には関わらない）。注記の種類を足すと、ここの対応がコンパイルで止まる（黙って項目なしにしない）。

use yolu_io::brushes::{DualNote, TextureNote, Unrepresented};

use crate::lang::Lang;

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
}

impl Gap {
    pub const ALL: [Gap; 32] = [
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
        }
    }

    pub fn from_id(id: &str) -> Option<Gap> {
        Gap::ALL.into_iter().find(|g| g.id() == id)
    }

    /// 項目の名前（短い名詞句。文にしない）。
    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            Gap::ColorTip => lang.pick("色つきの筆先", "Colored tip"),
            Gap::HoseSelection => lang.pick("セルの選び方", "Cell selection"),
            Gap::HoseDimensions => lang.pick("多次元のホース", "Multi-dimensional hose"),
            Gap::HoseCells => lang.pick("ホースのセル数", "Hose cell count"),
            Gap::TrailingData => lang.pick("末尾のデータ", "Trailing data"),
            Gap::TipEdge => lang.pick("筆先の縁", "Tip edge"),
            Gap::Tip16Bit => lang.pick("16 bit の筆先", "16-bit tip"),
            Gap::PresetSettings => lang.pick("ブラシの設定", "Brush settings"),
            Gap::PatternSection => lang.pick("模様の節", "Pattern section"),
            Gap::Sections => lang.pick("未対応の節", "Unsupported sections"),
            Gap::TipKind => lang.pick("筆先の種類", "Tip kind"),
            Gap::MinimumDiameter => lang.pick("最小の直径", "Minimum diameter"),
            Gap::Controls => lang.pick("コントロール", "Controls"),
            Gap::ForegroundBackground => lang.pick(
                "描画色/背景色のコントロール",
                "Foreground/background control",
            ),
            Gap::FadeLength => lang.pick("フェードの長さ", "Fade length"),
            Gap::ScatterAxis => lang.pick("1 軸の散布", "One-axis scatter"),
            Gap::CountJitter => lang.pick("数のゆらぎ", "Count jitter"),
            Gap::Noise => lang.pick("ノイズ", "Noise"),
            Gap::WetEdges => lang.pick("ウェットエッジ", "Wet edges"),
            Gap::TexturePattern => lang.pick("質感の模様", "Texture pattern"),
            Gap::TextureScale => lang.pick("質感の拡大", "Texture scale"),
            Gap::TextureMode => lang.pick("質感の合わせ方", "Texture mode"),
            Gap::TextureEachTip => lang.pick("描点ごとの質感", "Texture per dab"),
            Gap::TextureDepth => lang.pick("質感の深さのゆらぎ", "Texture depth jitter"),
            Gap::TextureBrightness => lang.pick("質感の明るさ", "Texture brightness"),
            Gap::TextureContrast => lang.pick("質感のコントラスト", "Texture contrast"),
            Gap::PatternConversion => lang.pick("模様の変換", "Pattern conversion"),
            Gap::DualTip => lang.pick("デュアルブラシの筆先", "Dual brush tip"),
            Gap::DualMode => lang.pick("デュアルブラシの合わせ方", "Dual brush mode"),
            Gap::DualScatter => lang.pick("デュアルブラシの散布", "Dual brush scatter"),
            Gap::DualCount => lang.pick("デュアルブラシの数", "Dual brush count"),
            Gap::DualFlip => lang.pick("デュアルブラシの反転", "Dual brush flip"),
        }
    }
}

/// 注記 1 つが指す項目。取り込んだ模様を使えず読み飛ばした注記（`PatternSkipped`）はどのブラシの項目でもないので None
/// （取り込めなかった数として別に数える）。
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
        U::PatternSkipped { .. } => return None,
    })
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
}
