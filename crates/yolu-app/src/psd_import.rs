//! PSD の取り込みの確認のウィンドウ（無視・落とす・変わるものを、レイヤーの名前と機能の名前で並べる）と、取り込めない理由の文。
//! 何を取り込み、何を持たないかの判断は `yolu_io::psd::import_copy`（ここは文にするだけ）。ウィンドウには名前・状態・短い理由だけを書き、
//! 説明はツールチップに置く。

use yolu_io::psd::{ImportAction, ImportDetail, ImportFeature, ImportNote, Unchecked};

use crate::lang::Lang;
use crate::psd::PsdAction;
use crate::state::{Action, AppState};
use crate::windows::{show_list_with, Button, ListSpec, Reply, Row};

/// ウィンドウの名前（`windows::window_rect` で矩形を引く名前）。
pub const CHECK: &str = "psd-import";

/// 取り込みの確かめの待ち（読んだ文書と、その知らせ）。
pub struct ImportCheck {
    pub(crate) doc: Box<yolu_core::Document>,
    pub(crate) notes: Vec<ImportNote>,
    pub(crate) file: String,
    pub(crate) target: crate::psd::PsdTarget,
    pub(crate) guard: Option<(u32, u128, u64)>,
}

impl ImportCheck {
    /// 読んだ PSD のファイル名。
    pub fn file(&self) -> &str {
        &self.file
    }
    /// 知らせの並び（落とす・変わる・無視の順）。
    pub fn notes(&self) -> &[ImportNote] {
        &self.notes
    }
}

fn rank(action: ImportAction) -> u8 {
    match action {
        ImportAction::Dropped => 0,
        ImportAction::Changed => 1,
        ImportAction::Ignored => 2,
    }
}

/// ウィンドウに並べる順（落とす・変わる・無視。同じ扱いの中は取り込みの順）。
pub fn sorted(notes: &[ImportNote]) -> Vec<&ImportNote> {
    let mut out: Vec<&ImportNote> = notes.iter().collect();
    out.sort_by_key(|n| rank(n.action));
    out
}

/// 1 行目の状態（扱いごとの件数）。
fn summary(lang: Lang, notes: &[ImportNote]) -> String {
    let count = |a: ImportAction| notes.iter().filter(|n| n.action == a).count();
    let mut parts = Vec::new();
    for (action, ja, en) in [
        (ImportAction::Dropped, "落とす", "Drop"),
        (ImportAction::Changed, "変わる", "Change"),
        (ImportAction::Ignored, "無視", "Ignore"),
    ] {
        let n = count(action);
        if n > 0 {
            parts.push(format!("{} {n}", lang.pick(ja, en)));
        }
    }
    parts.join(" · ")
}

/// 毎フレーム、取り込みの確認のウィンドウを描き、押された操作を当てる。
pub fn show_check(ctx: &egui::Context, app: &mut AppState) {
    let Some(check) = app.psd.import_check.as_ref() else {
        return;
    };
    let lang = app.lang;
    let notes = sorted(check.notes());
    let rows: Vec<Row> = notes
        .iter()
        .map(|n| Row {
            left: layers_text(lang, n),
            middle: feature_text(lang, n),
            right: action_text(lang, n.action).into(),
            warning: n.action != ImportAction::Ignored,
        })
        .collect();
    let tips: Vec<Option<String>> = notes
        .iter()
        .map(|n| feature_tooltip(lang, n).map(str::to_owned))
        .collect();
    let spec = ListSpec {
        id: CHECK,
        title: lang.t("psd_import-import-psd-check").into(),
        icon: "warning",
        modal: true,
        width: 700.0,
        summary: Some((
            format!("{} · {}", check.file(), summary(lang, check.notes())),
            true,
        )),
        rows,
        buttons: vec![
            Button {
                label: lang.t("psd_import-cancel").into(),
                primary: false,
                tooltip: None,
            },
            Button {
                label: lang.t("psd_import-import").into(),
                primary: true,
                tooltip: Some(lang.t("psd_import-imports-as-listed-the-psd-tip").into()),
            },
        ],
        close_label: lang.t("psd_import-close-window").into(),
    };
    let mut offset = app.ui.psd_import_window_offset;
    let mut scroll = 0.0;
    let reply = show_list_with(ctx, &spec, &tips, &mut offset, &mut scroll);
    app.ui.psd_import_window_offset = offset;
    match reply {
        Some(Reply::Button(1)) => app.apply(Action::Psd(PsdAction::ConfirmImport)),
        Some(_) => app.apply(Action::Psd(PsdAction::CancelImport)),
        None => {}
    }
}

// ───────── 文 ─────────

/// 扱いの名前（右の列）。
pub fn action_text(lang: Lang, action: ImportAction) -> &'static str {
    match action {
        ImportAction::Ignored => lang.t("psd_import-ignored"),
        ImportAction::Dropped => lang.t("psd_import-dropped"),
        ImportAction::Changed => lang.t("psd_import-changed"),
    }
}

/// 当たったレイヤーの名前（初めの 3 枚と残りの数）。文書の機能は空。
fn layers_text(lang: Lang, note: &ImportNote) -> String {
    const SHOWN: usize = 3;
    if note.layers.is_empty() {
        return String::new();
    }
    let sep = lang.t("psd_import-text");
    let mut text = note
        .layers
        .iter()
        .take(SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(sep);
    let rest = note.count.saturating_sub(SHOWN.min(note.layers.len()));
    if rest > 0 {
        text += &lang.f("psd_import-and-more", &[("rest", &rest)]);
    }
    text
}

fn keys(note: &ImportNote) -> Vec<String> {
    note.details
        .iter()
        .map(|d| match d {
            ImportDetail::Key(k) => String::from_utf8_lossy(k).to_string(),
        })
        .collect()
}

/// 4 文字のキーを名前にして、同じ名前は 1 つにまとめて並べる（PSD の内部のキーは画面に出さない）。
fn named(lang: Lang, keys: &[String], name: fn(Lang, &str) -> String) -> String {
    let mut names: Vec<String> = Vec::new();
    for key in keys {
        let n = name(lang, key);
        if !names.contains(&n) {
            names.push(n)
        }
    }
    names.join(lang.t("psd_import-text-2"))
}

/// 未対応の調整の名前。落とした調整のキーは全部、機能の名前にする（知らないキーは「その他の調整」）。
fn adjustment_name(lang: Lang, key: &str) -> String {
    use crate::m2::AdjustmentKind as K;
    match key {
        "nvrt" => K::Invert.name(lang).into(),
        "levl" => K::Levels.name(lang).into(),
        "hue2" => K::HueSaturation.name(lang).into(),
        "grdm" => K::GradientMap.name(lang).into(),
        "curv" => K::ToneCurve.name(lang).into(),
        "blnc" => K::ColorBalance.name(lang).into(),
        "brit" | "CgEd" => K::BrightnessContrast.name(lang).into(),
        "thrs" => K::Threshold.name(lang).into(),
        "post" => K::Posterize.name(lang).into(),
        "hue " => lang.t("psd_import-hue-saturation-legacy").into(),
        "selc" => lang.t("psd_import-selective-color").into(),
        "mixr" => lang.t("psd_import-channel-mixer").into(),
        "phfl" => lang.t("psd_import-photo-filter").into(),
        "vibA" => lang.t("psd_import-vibrance").into(),
        "blwh" => lang.t("psd_import-black-and-white").into(),
        "clrL" => lang.t("psd_import-color-lookup").into(),
        "expA" => lang.t("psd_import-exposure").into(),
        _ => lang.t("psd_import-other-adjustments").into(),
    }
}

/// 取り込めなかった合成モードの名前（知らないキーは「その他」）。
fn blend_name(lang: Lang, key: &str) -> String {
    match key {
        "diss" => lang.t("psd_import-dissolve").into(),
        "pass" => lang.t("psd_import-pass-through-non-group").into(),
        _ => lang.t("psd_import-other").into(),
    }
}

/// 機能の名前（真ん中の列。文は名詞句）。
pub fn feature_text(lang: Lang, note: &ImportNote) -> String {
    use ImportFeature as F;
    let keys = keys(note);
    let plain = |ja: &str, en: &str| -> String { lang.pick(ja, en).into() };
    match note.feature {
        F::GlobalMask => plain("全体マスク", "Global mask"),
        F::ImageResources => lang.f("psd_import-image-resources", &[("note", &note.count)]),
        F::ColorProfile => plain("sRGB 以外のカラープロファイル", "Non-sRGB color profile"),
        F::PixelAspect => plain("画素の縦横比", "Pixel aspect ratio"),
        F::ColorModeData => plain("色モードデータ", "Color mode data"),
        F::DocumentTags => lang.f("psd_import-document-tags", &[("note", &note.count)]),
        F::ExtraChannel => plain(
            "統合画像の追加チャンネル",
            "Extra channel in the merged image",
        ),
        F::LayerInfoTail => plain("レイヤー情報の余り", "Trailing layer data"),
        F::CompositeDiffers {
            max_diff,
            differing,
            total,
        } => {
            let percent = if total == 0 {
                0.0
            } else {
                differing as f64 * 100.0 / total as f64
            };
            let shown = if percent > 0.0 && percent < 0.1 {
                "<0.1".to_owned()
            } else {
                format!("{percent:.1}")
            };
            lang.f(
                "psd_import-differs-from-the-merged-image",
                &[("shown", &shown), ("max_diff", &max_diff)],
            )
        }
        F::CompositeUnchecked(why) => {
            let why = match why {
                Unchecked::NoComposite => lang.t("psd_import-no-merged-image"),
                Unchecked::Unreadable => lang.t("psd_import-merged-image-unreadable"),
                Unchecked::Budget => lang.t("psd_import-canvas-too-large"),
            };
            lang.f("psd_import-merged-image-not-compared", &[("why", &why)])
        }
        F::LayerIds => plain("レイヤー ID の欠落・重複", "Missing or duplicate layer IDs"),
        F::LayerName => plain("レイヤー名の文字", "Layer name characters"),
        F::LayerNameTruncated => plain("長すぎるレイヤー名", "Layer name too long"),
        F::LayerEffects => plain("レイヤー効果", "Layer effects"),
        F::BlendIf => plain("ブレンド条件", "Blend If"),
        F::FillOpacity => plain("塗りの不透明度", "Fill opacity"),
        F::ClippedBlend => plain(
            "クリップしたレイヤーのグループ合成",
            "Blend clipped layers as group",
        ),
        F::InteriorBlend => plain("内部効果のグループ合成", "Blend interior effects as group"),
        F::Knockout => plain("ノックアウト", "Knockout"),
        F::TransparencyShapes => plain("透明部分が形を決める設定", "Transparency shapes layer"),
        F::ChannelRestrictions => plain("チャンネルの合成制限", "Channel restrictions"),
        F::SmartObject => plain("スマートオブジェクトの元", "Smart object source"),
        F::TextLayer => plain("テキストのデータ", "Text data"),
        F::VectorMask => plain("ベクターマスク", "Vector mask"),
        F::FillSettings => plain("塗りつぶしの設定", "Fill settings"),
        F::UnsupportedAdjustment => {
            let names = named(lang, &keys, adjustment_name);
            lang.f("psd_import-adjustment-layer", &[("names", &names)])
        }
        F::BlendMode => {
            let names = named(lang, &keys, blend_name);
            lang.f("psd_import-blend-mode", &[("names", &names)])
        }
        F::MaskFlags => plain("マスクのフラグ", "Mask flags"),
        F::MaskFeather => plain("マスクのぼかし", "Mask feather"),
        F::MaskDefault => plain("マスクの既定値", "Mask default value"),
        F::MaskWithoutPixels => plain("画素のないマスク", "Mask without pixels"),
        F::UserAndVectorMask => plain("ユーザーマスクとベクターマスク", "User and vector mask"),
        F::OutsideCanvas => plain("キャンバス外の画素", "Pixels outside the canvas"),
        F::MaskOutsideCanvas => plain("キャンバス外のマスク", "Mask outside the canvas"),
        F::GroupPixels => plain("グループ自身の画素", "Group pixels"),
        F::MissingChannels => plain("RGB チャンネルの不足", "Missing RGB channels"),
        F::LayerChannels => plain("レイヤーの追加チャンネル", "Extra layer channels"),
        F::LayerMetadata => plain("レイヤーのメタデータ", "Layer metadata"),
        F::LayerColorLabel => plain("色ラベル", "Color label"),
        F::CollapsedGroup => plain("グループの開閉", "Group open state"),
        F::LayerLockBits => plain("ロックのビット", "Lock bits"),
        F::LayerFlags => plain("レイヤーのフラグ", "Layer flags"),
    }
}

/// 機能のツールチップ（説明はここに置く。無視する機能は名前だけ）。
pub fn feature_tooltip(lang: Lang, note: &ImportNote) -> Option<&'static str> {
    use ImportFeature as F;
    Some(match note.feature {
        F::LayerEffects => lang.t("psd_import-layer-effects-drop-shadow-and-tip"),
        F::SmartObject => lang.t("psd_import-the-smart-object-s-source-tip"),
        F::TextLayer => lang.t("psd_import-the-text-data-is-not-tip"),
        F::VectorMask => lang.t("psd_import-vector-masks-are-not-evaluated-tip"),
        F::FillSettings => lang.t("psd_import-gradient-and-pattern-fill-settings-tip"),
        F::UnsupportedAdjustment => lang.t("psd_import-this-adjustment-layer-cannot-be-tip"),
        F::BlendMode => lang.t("psd_import-unsupported-blend-modes-become-normal-tip"),
        F::BlendIf => lang.t("psd_import-blend-if-is-not-evaluated-tip"),
        F::FillOpacity => lang.t("psd_import-fill-opacity-is-multiplied-into-tip"),
        F::ClippedBlend | F::InteriorBlend | F::Knockout | F::ChannelRestrictions => {
            lang.t("psd_import-this-setting-is-not-evaluated-tip")
        }
        F::MaskFlags | F::MaskFeather | F::UserAndVectorMask => {
            lang.t("psd_import-this-mask-setting-is-not-tip")
        }
        F::MaskDefault => lang.t("psd_import-a-mask-default-other-than-tip"),
        F::MaskWithoutPixels => lang.t("psd_import-the-mask-has-a-rectangle-tip"),
        F::OutsideCanvas | F::MaskOutsideCanvas => {
            lang.t("psd_import-pixels-outside-the-canvas-cannot-tip")
        }
        F::LayerIds => lang.t("psd_import-layers-with-a-missing-or-tip"),
        F::ColorProfile => lang.t("psd_import-colors-are-not-converted-pixel-tip"),
        F::PixelAspect => lang.t("psd_import-the-aspect-ratio-is-not-tip"),
        F::CompositeDiffers { .. } => lang.t("psd_import-the-imported-project-s-composite-tip"),
        F::CompositeUnchecked(_) => lang.t("psd_import-the-composite-was-not-compared-tip"),
        F::MissingChannels => lang.t("psd_import-a-layer-without-all-rgb-tip"),
        _ => return None,
    })
}

// ───────── 取り込めない理由 ─────────

pub(crate) fn color_mode_name(lang: Lang, mode: u16) -> String {
    match mode {
        0 => lang.t("psd_import-bitmap").into(),
        1 => lang.t("psd_import-grayscale").into(),
        2 => lang.t("psd_import-indexed-color").into(),
        3 => "RGB".into(),
        4 => "CMYK".into(),
        7 => lang.t("psd_import-multichannel").into(),
        8 => lang.t("psd_import-duotone").into(),
        9 => "Lab".into(),
        n => lang.f("psd_import-color-mode", &[("n", &n)]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_io::psd::CopyRefusal;

    fn has_japanese(text: &str) -> bool {
        text.chars()
            .any(|c| matches!(c, '\u{3000}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}' | '\u{ff00}'..='\u{ffef}'))
    }

    fn every_feature() -> Vec<ImportNote> {
        use ImportFeature as F;
        // 調整のキーを複数（知らないキーも）持たせて、どの機能の文にも内部のキーが出ないことを確かめる
        let keys: Vec<ImportDetail> = [*b"selc", *b"levl", *b"brit", *b"zzzz"]
            .map(ImportDetail::Key)
            .into();
        [
            F::GlobalMask,
            F::ImageResources,
            F::ColorProfile,
            F::PixelAspect,
            F::ColorModeData,
            F::DocumentTags,
            F::ExtraChannel,
            F::LayerInfoTail,
            F::CompositeDiffers {
                max_diff: 9,
                differing: 3,
                total: 1000,
            },
            F::CompositeUnchecked(Unchecked::NoComposite),
            F::CompositeUnchecked(Unchecked::Unreadable),
            F::CompositeUnchecked(Unchecked::Budget),
            F::LayerIds,
            F::LayerName,
            F::LayerNameTruncated,
            F::LayerEffects,
            F::BlendIf,
            F::FillOpacity,
            F::ClippedBlend,
            F::InteriorBlend,
            F::Knockout,
            F::TransparencyShapes,
            F::ChannelRestrictions,
            F::SmartObject,
            F::TextLayer,
            F::VectorMask,
            F::FillSettings,
            F::UnsupportedAdjustment,
            F::BlendMode,
            F::MaskFlags,
            F::MaskFeather,
            F::MaskDefault,
            F::MaskWithoutPixels,
            F::UserAndVectorMask,
            F::OutsideCanvas,
            F::MaskOutsideCanvas,
            F::GroupPixels,
            F::MissingChannels,
            F::LayerChannels,
            F::LayerMetadata,
            F::LayerColorLabel,
            F::CollapsedGroup,
            F::LayerLockBits,
            F::LayerFlags,
        ]
        .into_iter()
        .map(|feature| ImportNote {
            feature,
            action: ImportAction::Changed,
            layers: vec!["下".into(), "上".into()],
            count: 2,
            details: keys.clone(),
        })
        .collect()
    }

    #[test]
    fn every_feature_has_a_name_in_both_languages_and_english_has_no_japanese() {
        for note in every_feature() {
            let ja = feature_text(Lang::Ja, &note);
            let en = feature_text(Lang::En, &note);
            assert!(!ja.is_empty() && !en.is_empty(), "{:?}", note.feature);
            assert!(has_japanese(&ja), "{ja}");
            assert!(!has_japanese(&en), "{:?}: {en}", note.feature);
            for raw in ["selc", "levl", "brit", "zzzz"] {
                assert!(
                    !ja.contains(raw) && !en.contains(raw),
                    "{:?}: {ja} / {en}",
                    note.feature
                );
            }
            if let Some(tip) = feature_tooltip(Lang::En, &note) {
                assert!(!has_japanese(tip), "{tip}");
                assert!(has_japanese(feature_tooltip(Lang::Ja, &note).unwrap()));
            }
        }
        for action in [
            ImportAction::Ignored,
            ImportAction::Dropped,
            ImportAction::Changed,
        ] {
            assert!(!has_japanese(action_text(Lang::En, action)));
            assert!(has_japanese(action_text(Lang::Ja, action)));
        }
    }

    /// 画面の文に PSD の内部の 4 文字のキーが出ていないこと（英数字だけの 4 文字の語を探す）。
    fn shows_a_raw_key(text: &str, keys: &[[u8; 4]]) -> bool {
        keys.iter()
            .any(|k| text.contains(String::from_utf8_lossy(k).trim()))
    }

    #[test]
    fn every_adjustment_that_can_be_dropped_is_named_not_shown_as_its_key() {
        let note = |keys: &[[u8; 4]]| ImportNote {
            feature: ImportFeature::UnsupportedAdjustment,
            action: ImportAction::Dropped,
            layers: vec!["レイヤー".into()],
            count: 1,
            details: keys.iter().map(|k| ImportDetail::Key(*k)).collect(),
        };
        // 落ちうる調整の全部のキーに、日英の名前がある（キーのままの文にならない）
        for k in yolu_io::psd::ADJUSTMENT_TAG_KEYS {
            let n = note(&[k]);
            let (ja, en) = (feature_text(Lang::Ja, &n), feature_text(Lang::En, &n));
            let key = String::from_utf8_lossy(&k).to_string();
            assert!(
                !ja.contains(key.trim()) && !en.contains(key.trim()),
                "{key}: {ja} / {en}"
            );
            assert!(
                has_japanese(&ja) && !has_japanese(&en),
                "{key}: {ja} / {en}"
            );
            assert!(
                !ja.contains("その他") && !en.contains("Other"),
                "{key} が「その他」になっている: {ja} / {en}"
            );
        }
        // 複数のキー（明るさ・コントラストの旧式と新式は 1 つにまとまる）
        let n = note(&[*b"levl", *b"curv", *b"brit", *b"CgEd"]);
        assert_eq!(
            feature_text(Lang::Ja, &n),
            "調整レイヤー: レベル補正・トーンカーブ・明るさ・コントラスト"
        );
        assert_eq!(
            feature_text(Lang::En, &n),
            "Adjustment layer: Levels, Tone Curve, Brightness / Contrast"
        );
        // 知らないキーは、4 文字のままでなく「その他」
        let n = note(&[*b"zzzz", *b"yyyy"]);
        assert_eq!(feature_text(Lang::Ja, &n), "調整レイヤー: その他の調整");
        assert_eq!(
            feature_text(Lang::En, &n),
            "Adjustment layer: Other adjustments"
        );
    }

    #[test]
    fn blend_modes_documents_tags_and_metadata_never_show_internal_keys() {
        let keys = [*b"diss", *b"pass", *b"lnsr", *b"Patt", *b"zzzz"];
        let with = |feature, details: Vec<ImportDetail>| ImportNote {
            feature,
            action: ImportAction::Changed,
            layers: vec!["レイヤー".into()],
            count: 3,
            details,
        };
        let all: Vec<ImportDetail> = keys.iter().map(|k| ImportDetail::Key(*k)).collect();
        for feature in [
            ImportFeature::BlendMode,
            ImportFeature::DocumentTags,
            ImportFeature::LayerMetadata,
        ] {
            for lang in [Lang::Ja, Lang::En] {
                let text = feature_text(lang, &with(feature, all.clone()));
                assert!(!shows_a_raw_key(&text, &keys), "{feature:?}: {text}");
            }
        }
        assert_eq!(
            feature_text(Lang::Ja, &with(ImportFeature::BlendMode, all)),
            "合成モード: ディゾルブ・通過（グループ以外）・その他"
        );
        assert_eq!(
            feature_text(Lang::En, &with(ImportFeature::DocumentTags, vec![])),
            "Document tags: 3"
        );
    }

    #[test]
    fn layers_are_listed_by_name_with_the_rest_counted() {
        let mut n = every_feature().remove(0);
        n.layers = vec!["a".into(), "b".into(), "c".into(), "d".into()];
        n.count = 9;
        assert_eq!(layers_text(Lang::Ja, &n), "a、b、c ほか 6 レイヤー");
        assert_eq!(layers_text(Lang::En, &n), "a, b, c and 6 more");
        n.layers.clear();
        assert_eq!(layers_text(Lang::En, &n), "");
    }

    #[test]
    fn the_composite_difference_names_its_share_and_maximum() {
        let note = |differing| ImportNote {
            feature: ImportFeature::CompositeDiffers {
                max_diff: 12,
                differing,
                total: 100_000,
            },
            action: ImportAction::Changed,
            layers: vec![],
            count: 1,
            details: vec![],
        };
        assert_eq!(
            feature_text(Lang::En, &note(2500)),
            "Differs from the merged image: 2.5% of pixels, max 12"
        );
        assert!(feature_text(Lang::En, &note(1)).contains("<0.1%"));
        assert!(feature_text(Lang::Ja, &note(2500)).contains("2.5%"));
    }

    #[test]
    fn refusals_are_worded_in_both_languages_and_the_budget_ones_carry_a_hint() {
        let reasons = [
            CopyRefusal::Malformed("壊れている".into()),
            CopyRefusal::LargeDocument,
            CopyRefusal::ColorFormat { depth: 16, mode: 3 },
            CopyRefusal::ColorFormat { depth: 8, mode: 4 },
            CopyRefusal::NoLayers,
            CopyRefusal::TooManyLayers {
                count: 300,
                limit: 256,
            },
            CopyRefusal::CanvasTooLarge {
                width: 9000,
                height: 9000,
            },
            CopyRefusal::LayerTooLarge { layer: "L".into() },
            CopyRefusal::BudgetExceeded { layer: "L".into() },
            CopyRefusal::LayerDataTooLarge { layer: "L".into() },
            CopyRefusal::EdgeOverLimit {
                width: 9000,
                height: 9000,
                limit: 8192,
            },
            CopyRefusal::LayerCountOverLimit {
                count: 2100,
                limit: 2048,
            },
            CopyRefusal::NestingTooDeep { limit: 64 },
        ];
        for why in &reasons {
            let en = crate::lang::psd_copy_refusal(Lang::En, why);
            assert!(!has_japanese(&en), "{en}");
            assert!(has_japanese(&crate::lang::psd_copy_refusal(Lang::Ja, why)));
            assert_eq!(
                crate::lang::psd_copy_refusal_tooltip(Lang::En, why).is_some(),
                why.raised_by_budget(),
                "{why:?}"
            );
            if let Some(h) = crate::lang::psd_copy_refusal_tooltip(Lang::En, why) {
                assert!(!has_japanese(&h) && h.contains("Layer memory"));
            }
        }
        // 利用者の名前（レイヤーの名前）だけは、英語の画面でもそのまま
        assert!(crate::lang::psd_copy_refusal(
            Lang::En,
            &CopyRefusal::BudgetExceeded {
                layer: "下".into()
            }
        )
        .contains('下'));
    }

    #[test]
    fn notes_are_sorted_dropped_then_changed_then_ignored_and_summarized() {
        let notes = [
            (ImportFeature::GlobalMask, ImportAction::Ignored),
            (ImportFeature::LayerEffects, ImportAction::Changed),
            (ImportFeature::SmartObject, ImportAction::Dropped),
            (ImportFeature::ImageResources, ImportAction::Ignored),
        ]
        .map(|(feature, action)| ImportNote {
            feature,
            action,
            layers: vec![],
            count: 1,
            details: vec![],
        });
        let order: Vec<_> = sorted(&notes).iter().map(|n| n.feature).collect();
        assert_eq!(
            order,
            [
                ImportFeature::SmartObject,
                ImportFeature::LayerEffects,
                ImportFeature::GlobalMask,
                ImportFeature::ImageResources
            ]
        );
        assert_eq!(summary(Lang::Ja, &notes), "落とす 1 · 変わる 1 · 無視 2");
        assert_eq!(summary(Lang::En, &notes), "Drop 1 · Change 1 · Ignore 2");
    }
}
