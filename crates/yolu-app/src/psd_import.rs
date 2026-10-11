//! PSD の取り込みの確認のウィンドウ（無視・落とす・変わるものを、レイヤーの名前と機能の名前で並べる）と、取り込めない理由の文。
//! 何を取り込み、何を持たないかの判断は `yolu_io::psd::import_copy`（ここは文にするだけ）。ウィンドウには名前・状態・短い理由だけを書き、
//! 説明はツールチップに置く。

use egui::Vec2;
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

/// ウィンドウの位置（`PsdState` が持つ）。
#[derive(Default)]
pub struct CheckWindow {
    pub offset: Vec2,
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
        title: lang
            .pick("PSD の取り込みの確かめ", "Import PSD Check")
            .into(),
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
                label: lang.pick("やめる", "Cancel").into(),
                primary: false,
                tooltip: None,
            },
            Button {
                label: lang.pick("取り込む", "Import").into(),
                primary: true,
                tooltip: Some(
                    lang.pick(
                        "一覧のとおりに取り込みます。PSD 自体は書き換えません（書き出しは新しい PSD になります）",
                        "Imports as listed. The PSD itself is never rewritten (exports are new PSDs)",
                    )
                    .into(),
                ),
            },
        ],
        close_label: lang.pick("ウィンドウを閉じる", "Close Window").into(),
    };
    let mut offset = app.psd.import_window.offset;
    let mut scroll = 0.0;
    let reply = show_list_with(ctx, &spec, &tips, &mut offset, &mut scroll);
    app.psd.import_window.offset = offset;
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
        ImportAction::Ignored => lang.pick("無視", "Ignored"),
        ImportAction::Dropped => lang.pick("落とす", "Dropped"),
        ImportAction::Changed => lang.pick("変わる", "Changed"),
    }
}

/// 当たったレイヤーの名前（初めの 3 枚と残りの数）。文書の機能は空。
fn layers_text(lang: Lang, note: &ImportNote) -> String {
    const SHOWN: usize = 3;
    if note.layers.is_empty() {
        return String::new();
    }
    let sep = lang.pick("、", ", ");
    let mut text = note
        .layers
        .iter()
        .take(SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(sep);
    let rest = note.count.saturating_sub(SHOWN.min(note.layers.len()));
    if rest > 0 {
        text += &lang.pick(
            format!(" ほか {rest} レイヤー"),
            format!(" and {rest} more"),
        );
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
    names.join(lang.pick("・", ", "))
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
        "hue " => lang
            .pick("色相・彩度（旧式）", "Hue/saturation (legacy)")
            .into(),
        "selc" => lang.pick("特定色域の選択", "Selective color").into(),
        "mixr" => lang.pick("チャンネルミキサー", "Channel mixer").into(),
        "phfl" => lang.pick("フォトフィルター", "Photo filter").into(),
        "vibA" => lang.pick("自然な彩度", "Vibrance").into(),
        "blwh" => lang.pick("白黒", "Black and white").into(),
        "clrL" => lang.pick("カラールックアップ", "Color lookup").into(),
        "expA" => lang.pick("露光量", "Exposure").into(),
        _ => lang.pick("その他の調整", "Other adjustments").into(),
    }
}

/// 取り込めなかった合成モードの名前（知らないキーは「その他」）。
fn blend_name(lang: Lang, key: &str) -> String {
    match key {
        "diss" => lang.pick("ディゾルブ", "Dissolve").into(),
        "pass" => lang
            .pick("通過（グループ以外）", "Pass through (non-group)")
            .into(),
        _ => lang.pick("その他", "Other").into(),
    }
}

/// 機能の名前（真ん中の列。文は名詞句）。
pub fn feature_text(lang: Lang, note: &ImportNote) -> String {
    use ImportFeature as F;
    let keys = keys(note);
    let plain = |ja: &str, en: &str| -> String { lang.pick(ja, en).into() };
    match note.feature {
        F::GlobalMask => plain("全体マスク", "Global mask"),
        F::ImageResources => lang.pick(
            format!("画像リソース {} 件", note.count),
            format!("Image resources: {}", note.count),
        ),
        F::ColorProfile => plain("sRGB 以外のカラープロファイル", "Non-sRGB color profile"),
        F::PixelAspect => plain("画素の縦横比", "Pixel aspect ratio"),
        F::ColorModeData => plain("色モードデータ", "Color mode data"),
        F::DocumentTags => lang.pick(
            format!("ドキュメントのタグ {} 件", note.count),
            format!("Document tags: {}", note.count),
        ),
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
            lang.pick(
                format!("統合画像との差: {shown}% の画素・最大 {max_diff}"),
                format!("Differs from the merged image: {shown}% of pixels, max {max_diff}"),
            )
        }
        F::CompositeUnchecked(why) => {
            let why = match why {
                Unchecked::NoComposite => lang.pick("統合画像なし", "no merged image"),
                Unchecked::Unreadable => lang.pick("統合画像を読めない", "merged image unreadable"),
                Unchecked::Budget => lang.pick("キャンバスが大きい", "canvas too large"),
            };
            lang.pick(
                format!("統合画像との照合を省略（{why}）"),
                format!("Merged image not compared ({why})"),
            )
        }
        F::LayerIds => plain("レイヤー ID の欠落・重複", "Missing or duplicate layer IDs"),
        F::LayerName => plain("レイヤー名の文字", "Layer name characters"),
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
            lang.pick(
                format!("調整レイヤー: {names}"),
                format!("Adjustment layer: {names}"),
            )
        }
        F::BlendMode => {
            let names = named(lang, &keys, blend_name);
            lang.pick(
                format!("合成モード: {names}"),
                format!("Blend mode: {names}"),
            )
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
        F::LayerEffects => lang.pick(
            "レイヤー効果（ドロップシャドウなど）は評価しません。レイヤーの画素のまま取り込むので、合成が PSD の統合画像と変わります",
            "Layer effects (drop shadow and so on) are not evaluated. The layer pixels are imported as they are, so the composite differs from the PSD's merged image",
        ),
        F::SmartObject => lang.pick(
            "スマートオブジェクトの元のデータは持ちません。レイヤーの画素のまま取り込みます",
            "The smart object's source data is not kept. The layer pixels are imported as they are",
        ),
        F::TextLayer => lang.pick(
            "文字のデータは持ちません。レイヤーの画素のまま取り込みます",
            "The text data is not kept. The layer pixels are imported as they are",
        ),
        F::VectorMask => lang.pick(
            "ベクターマスクは評価しません。レイヤーの画素のまま取り込むので、合成が変わります",
            "Vector masks are not evaluated. The layer pixels are imported as they are, so the composite changes",
        ),
        F::FillSettings => lang.pick(
            "グラデーション・パターンの塗りつぶしの設定は持ちません。レイヤーの画素のまま取り込みます",
            "Gradient and pattern fill settings are not kept. The layer pixels are imported as they are",
        ),
        F::UnsupportedAdjustment => lang.pick(
            "この調整レイヤーは取り込めません。画素を持たないので、レイヤーごと落とします",
            "This adjustment layer cannot be imported. It has no pixels, so the whole layer is dropped",
        ),
        F::BlendMode => lang.pick(
            "未対応の合成モードは通常にします",
            "Unsupported blend modes become Normal",
        ),
        F::BlendIf => lang.pick(
            "ブレンド条件は評価しません。合成が変わります",
            "Blend If is not evaluated. The composite changes",
        ),
        F::FillOpacity => lang.pick(
            "塗りの不透明度は、レイヤーの不透明度に掛けます。合成モードによっては Photoshop と少し変わります",
            "Fill opacity is multiplied into the layer opacity. With some blend modes it differs slightly from Photoshop",
        ),
        F::ClippedBlend | F::InteriorBlend | F::Knockout | F::ChannelRestrictions => lang.pick(
            "この設定は評価しません。合成が変わることがあります",
            "This setting is not evaluated. The composite may change",
        ),
        F::MaskFlags | F::MaskFeather | F::UserAndVectorMask => lang.pick(
            "マスクのこの設定は評価しません。マスクの画素のまま取り込みます",
            "This mask setting is not evaluated. The mask pixels are imported as they are",
        ),
        F::MaskDefault => lang.pick(
            "既定値が 0・255 以外のマスクは、近いほうの 0 か 255 にします。マスクの矩形の外の見え方が変わります",
            "A mask default other than 0 or 255 is set to the nearer of the two. Outside the mask rectangle the look changes",
        ),
        F::MaskWithoutPixels => lang.pick(
            "マスクの矩形だけで画素の値がないので、既定値だけのマスクにします。矩形の中の見え方が変わることがあります",
            "The mask has a rectangle but no pixel values, so it becomes a mask of its default value only. Inside the rectangle the look may change",
        ),
        F::OutsideCanvas | F::MaskOutsideCanvas => lang.pick(
            "キャンバスの外にある画素は取り込めないので切り捨てます",
            "Pixels outside the canvas cannot be kept and are cut off",
        ),
        F::LayerIds => lang.pick(
            "レイヤー ID が無い・重複しているレイヤーには新しい ID を振ります。名前では対応付けません",
            "Layers with a missing or duplicate ID get a new ID. Nothing is matched by name",
        ),
        F::ColorProfile => lang.pick(
            "色は変換せず、画素の値のまま取り込みます",
            "Colors are not converted; pixel values are imported as they are",
        ),
        F::PixelAspect => lang.pick(
            "縦横比は持たず、正方形の画素として取り込みます",
            "The aspect ratio is not kept; pixels are imported as squares",
        ),
        F::CompositeDiffers { .. } => lang.pick(
            "取り込んだプロジェクトの合成を、PSD に保存された統合画像と照らした結果です",
            "The imported project's composite compared with the merged image saved in the PSD",
        ),
        F::CompositeUnchecked(_) => lang.pick(
            "統合画像と照らしていないので、合成が PSD と同じかは確かめていません",
            "The composite was not compared with the merged image, so it is unverified",
        ),
        F::MissingChannels => lang.pick(
            "RGB のチャンネルが揃わないレイヤーは取り込めないので、レイヤーごと落とします",
            "A layer without all RGB channels cannot be imported and is dropped",
        ),
        _ => return None,
    })
}

// ───────── 取り込めない理由 ─────────

pub(crate) fn color_mode_name(lang: Lang, mode: u16) -> String {
    match mode {
        0 => lang.pick("ビットマップ", "Bitmap").into(),
        1 => lang.pick("グレースケール", "Grayscale").into(),
        2 => lang.pick("インデックスカラー", "Indexed color").into(),
        3 => "RGB".into(),
        4 => "CMYK".into(),
        7 => lang.pick("マルチチャンネル", "Multichannel").into(),
        8 => lang.pick("ダブルトーン", "Duotone").into(),
        9 => "Lab".into(),
        n => lang.pick(format!("色モード {n}"), format!("Color mode {n}")),
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
