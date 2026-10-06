//! PSD を、原本を持たない「写し」として core の文書へ取り込む（`import_copy`）。
//!
//! 原本を保つ読み（[`super::read`]）は、PSD の情報を 1 つも黙って捨てないために、編集できない中身が 1 つでもあれば原本の保持だけにする。
//! 写しとしての取り込みは、元の PSD へ書き戻さない（書き出しはいつも新しい PSD）ので、情報を捨てても原本は壊れない。そこで、
//! 理由を 3 つに仕分ける（表は `docs/PSD.md`）。
//!
//! - **合成にも内容にも効かないもの**（全体マスクの表示の設定・解像度などの画像リソース・色ラベル・メタデータ・効果の無い層の `tsly`）
//!   は持たない（[`ImportAction::Ignored`]）。層 ID の欠落・重複は、名前で補修せず新しい ID を振る。
//! - **合成に効くのに評価できないもの**（レイヤー効果・ベクターマスク・未対応の合成モードなど）と、**層の画素は持っているが設定を
//!   持たないもの**（スマートオブジェクトの元・テキスト・グラデーション/パターンの塗りつぶしの設定）は、層の画素のまま取り込む
//!   （[`ImportAction::Changed`]・[`ImportAction::Dropped`]）。画素を持たない未対応の調整だけは層ごと落とす。
//! - **どうしても取れないもの**（PSB・RGB8 以外・予算を超える・壊れている）だけを断る（[`CopyRefusal`]）。
//!
//! 取り込んだ文書の合成は、PSD の統合画像と照らして最大の差と差のある画素の数を知らせる。
//!
//! 読みは `Read + Seek` から流す。付加情報は層ごと・タグごとに読んで捨て、層の画素は 1 枚ずつ復号して core へ入れてすぐ捨てる
//! （全層の復号を同時に持たない）。層の数・画素・層 1 枚の付加情報の予算は、呼び手が渡す「レイヤーのメモリ」の予算（`source_budget`）で決める。
use super::binary::Reader;
use super::read::{self, State};
use super::*;
use crate::{Error, Result};
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};

// ───────── 結果の型 ─────────

/// 取り込みで、その機能をどう扱ったか。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImportAction {
    /// 持たない。見え方にも内容にも効かない情報（無視）。
    Ignored,
    /// 持たない。層の画素は残るが、その機能の設定・元のデータは取り込まない（層ごと落とすものも含む）。
    Dropped,
    /// 取り込む形が変わる。合成が PSD の統合画像と変わり得る（評価できない効果・値が変わるもの）。
    Changed,
}

/// 取り込みの知らせの機能。層ごとの機能は `ImportNote::layers` に層の名前が付く。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImportFeature {
    // ── 文書 ──
    /// 全体マスク（表示の設定）。
    GlobalMask,
    /// 画像リソース（解像度・サムネイル・チャンネル名など。件数が `count`）。
    ImageResources,
    /// sRGB 以外の ICC プロファイル（色は変換せず値のまま）。
    ColorProfile,
    /// 正方形でない画素の縦横比。
    PixelAspect,
    /// RGB 色モードで意味のない色モードデータ。
    ColorModeData,
    /// 文書のタグ（キーは `details`）。
    DocumentTags,
    /// 統合画像の 4 番目のチャンネルが透明度でない（アルファチャンネル）。
    ExtraChannel,
    /// レイヤー情報の末尾の未知の余り。
    LayerInfoTail,
    /// 取り込んだ文書の合成が、保存された統合画像と違う。
    CompositeDiffers {
        /// 色（と、統合画像が持つ透明度）の最大の差（0〜255）。
        max_diff: u8,
        /// 差が 1 を超える画素の数。
        differing: u64,
        /// 画素の総数。
        total: u64,
    },
    /// 統合画像と照らしていない。
    CompositeUnchecked(Unchecked),
    // ── 層 ──
    /// 層 ID の欠落・重複・不正（新しい ID を振った。名前では補修しない）。
    LayerIds,
    /// 層の名前の中の NUL・Unicode 名のない非 ASCII 名。
    LayerName,
    /// レイヤー効果（`lfx2` など）。評価しない。
    LayerEffects,
    /// 既定値以外のブレンド条件（Blend If）。
    BlendIf,
    /// 塗りの不透明度（不透明度に掛けた）。
    FillOpacity,
    /// 「クリップした層をグループとして合成」の設定。
    ClippedBlend,
    /// 「内部効果をグループとして合成」の設定。
    InteriorBlend,
    /// ノックアウト。
    Knockout,
    /// 「透明部分が形を決める」の設定（`tsly`）。
    TransparencyShapes,
    /// チャンネルの合成制限。
    ChannelRestrictions,
    /// スマートオブジェクト（元のデータは持たず、層の画素のまま）。
    SmartObject,
    /// テキスト（文字のデータは持たず、層の画素のまま）。
    TextLayer,
    /// ベクターマスク（評価しない）。
    VectorMask,
    /// グラデーション・パターンの塗りつぶし（設定は持たず、層の画素のまま）。
    FillSettings,
    /// 未対応の調整レイヤー（画素を持たないので層ごと落とす。キーは `details`）。
    UnsupportedAdjustment,
    /// 未対応の合成モード（通常にした。キーは `details`）。
    BlendMode,
    /// マスクの未対応のフラグ（位置・反転・描画由来）。
    MaskFlags,
    /// マスクのぼかし・ベクターマスクの濃度。
    MaskFeather,
    /// マスクの既定値が 0・255 以外（近いほうの 0 か 255 に寄せた）。
    MaskDefault,
    /// マスクの定義だけがあって画素の値が無い（矩形を持たず、既定値だけのマスクにした）。
    MaskWithoutPixels,
    /// ユーザーマスクとベクターマスクの組（実マスク）。
    UserAndVectorMask,
    /// 画布の外の画素（切り捨てた）。
    OutsideCanvas,
    /// 画布の外のマスクの値（切り捨てた）。
    MaskOutsideCanvas,
    /// グループ自身の画素。
    GroupPixels,
    /// RGB のチャンネルが揃わない層（層ごと落とす）。
    MissingChannels,
    /// 層の未対応のチャンネル。
    LayerChannels,
    /// 層のメタデータ（キーは `details`）。
    LayerMetadata,
    /// 層の色ラベル。
    LayerColorLabel,
    /// 閉じたグループ（開閉の状態）。
    CollapsedGroup,
    /// 未対応のロックのビット。
    LayerLockBits,
    /// 層のフラグ・クリッピング値の未知の値。
    LayerFlags,
}

/// 統合画像と照らせなかった理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unchecked {
    /// 統合画像が無い。
    NoComposite,
    /// 統合画像の圧縮が未対応・壊れている。
    Unreadable,
    /// 画布が大きく、照らす 2 枚が予算に入らない。
    Budget,
}

/// 知らせの細目（キー・番号）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImportDetail {
    /// タグ・合成モードの 4 文字のキー。
    Key([u8; 4]),
}

/// 取り込みの知らせ 1 件（機能と扱いごとに 1 つ）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportNote {
    pub feature: ImportFeature,
    pub action: ImportAction,
    /// 当たった層の名前（初めの `ImportNote::MAX_LAYERS` 枚。文書の機能では空）。
    pub layers: Vec<String>,
    /// 当たった層の数（文書の機能・画像リソースなどは件数）。
    pub count: usize,
    /// 細目（重複なし・初めの `ImportNote::MAX_DETAILS` 個）。
    pub details: Vec<ImportDetail>,
}
impl ImportNote {
    pub const MAX_LAYERS: usize = 8;
    pub const MAX_DETAILS: usize = 8;
}

/// 取り込めない理由。画面は種類から言語ごとの文を作る（`message` は日本語の診断）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CopyRefusal {
    /// PSD として壊れている（日本語の診断）。
    Malformed(String),
    /// PSB（大きな文書の形式）。
    LargeDocument,
    /// RGB 8 bit 以外（色モードと深さ）。
    ColorFormat { depth: u16, mode: u16 },
    /// レイヤーが無い（統合画像だけ）。
    NoLayers,
    /// 層の数が予算から決めた上限を超える。
    TooManyLayers { count: usize, limit: usize },
    /// 画布 1 枚ぶんの画素が予算を超える。
    CanvasTooLarge { width: u32, height: u32 },
    /// 1 枚の層の画素が予算を超える。
    LayerTooLarge { layer: String },
    /// 層を足すと文書の画素が予算を超える。
    BudgetExceeded { layer: String },
    /// 層 1 枚の付加情報（効果・スマートオブジェクトの中身など）が予算を超える。層の名前は読む前なので `#番号`。
    LayerDataTooLarge { layer: String },
    /// 辺が .ylp の上限（`MAX_DOCUMENT_EDGE`）を超える。取り込めても保存できない文書になるので、予算を上げても取り込めない。
    EdgeOverLimit { width: u32, height: u32, limit: u32 },
    /// 層（グループも数える。区切りの記録は数えない）が .ylp の上限（`MAX_DOCUMENT_LAYERS`）を超える。予算を上げても取り込めない。
    LayerCountOverLimit { count: usize, limit: usize },
    /// グループの入れ子が上限（`yolu_core::MAX_GROUP_DEPTH`）を超える。合成の再帰がスタックを使い切るので、予算を上げても取り込めない。
    NestingTooDeep { limit: usize },
}
impl CopyRefusal {
    /// 日本語の診断（試験・ログ用）。
    pub fn message(&self) -> String {
        match self {
            Self::Malformed(why) => why.clone(),
            Self::LargeDocument => "PSB は取り込めません".into(),
            Self::ColorFormat { depth, mode } => {
                format!("RGB 8 bit 以外は取り込めません（色モード {mode}・深さ {depth}）")
            }
            Self::NoLayers => "レイヤーがありません".into(),
            Self::TooManyLayers { count, limit } => {
                format!("レイヤーが多すぎます（{count} 枚、上限 {limit} 枚）")
            }
            Self::CanvasTooLarge { width, height } => {
                format!("キャンバスが大きすぎます（{width}×{height}）")
            }
            Self::LayerTooLarge { layer } => format!("レイヤー「{layer}」の画素が予算を超えます"),
            Self::BudgetExceeded { layer } => {
                format!("レイヤー「{layer}」を足すと画素の予算を超えます")
            }
            Self::LayerDataTooLarge { layer } => {
                format!("レイヤー「{layer}」の付加情報が大きすぎます")
            }
            Self::EdgeOverLimit {
                width,
                height,
                limit,
            } => {
                format!("キャンバスが大きすぎます（{width}×{height}、上限 {limit}）")
            }
            Self::LayerCountOverLimit { count, limit } => {
                format!("レイヤーが多すぎます（{count} 枚、保存できるのは {limit} 枚まで）")
            }
            Self::NestingTooDeep { limit } => {
                format!("グループの入れ子が深すぎます（上限 {limit} 段）")
            }
        }
    }
    /// 設定の「レイヤーのメモリ」の予算を上げると取り込めるようになる理由か。
    pub fn raised_by_budget(&self) -> bool {
        matches!(
            self,
            Self::TooManyLayers { .. }
                | Self::CanvasTooLarge { .. }
                | Self::LayerTooLarge { .. }
                | Self::BudgetExceeded { .. }
                | Self::LayerDataTooLarge { .. }
        )
    }
}

/// 取り込みの設定。
#[derive(Clone, Copy, Debug)]
pub struct CopyOptions<'a> {
    /// 文書の層の画素に許すバイト数（設定の「レイヤーのメモリ」）。層の数・画布・層の画素の上限をここから決める。
    pub source_budget: u64,
    /// 立てると、層の間・統合画像と照らす間に止めて `Error::Core(Cancelled)` で戻る。
    pub cancel: Option<&'a AtomicBool>,
}
impl CopyOptions<'_> {
    /// 取り込める層の数（予算 1 MiB につき 1 枚。PSD の上限 32767 と 256 の間）。
    pub fn max_layers(&self) -> usize {
        ((self.source_budget / (1024 * 1024)) as usize).clamp(256, 32767)
    }
}

/// 取り込んだ結果。
pub struct CopyImport {
    pub document: yolu_core::Document,
    pub notes: Vec<ImportNote>,
}
/// 取り込みの結果（断った理由か、文書と知らせ）。
pub enum CopyOutcome {
    Imported(Box<CopyImport>),
    Refused(CopyRefusal),
}

// ───────── 読み手の状態（`read.rs` の `State` が持つ） ─────────

/// 写しとしての取り込みの状態。層ごとの知らせは、層の名前が決まる（`luni` を読み終える）まで `pending` に置く。
#[derive(Default)]
pub(super) struct CopyState {
    notes: Vec<ImportNote>,
    /// 読んでいる層の知らせ。
    pending: Vec<(ImportFeature, ImportAction, Vec<ImportDetail>)>,
    /// 層を読んでいる間か（知らせを層の知らせにするか、文書の知らせにするか）。
    pub(super) in_layer: bool,
    /// 読んでいる層を、層ごと落とす理由（画素を持たない未対応の調整など）。
    pub(super) drop_layer: bool,
    /// 取り込めない理由（見つかったら、読み手は次の区切りで止める）。
    pub(super) refusal: Option<CopyRefusal>,
}
impl CopyState {
    pub(super) fn new() -> Self {
        Self::default()
    }
    pub(super) fn into_notes(self) -> Vec<ImportNote> {
        self.notes
    }
    pub(super) fn take_refusal(&mut self) -> Option<CopyRefusal> {
        self.refusal.take()
    }
    /// 層の知らせを層の名前つきで文書の知らせへ移す。
    pub(super) fn flush_layer(&mut self, name: &str) {
        for (feature, action, details) in std::mem::take(&mut self.pending) {
            self.add(feature, action, details, Some(name));
        }
    }
    /// 層の画素を入れるときに分かった知らせ（層の名前つき）。
    pub(super) fn add_layer_note(
        &mut self,
        feature: ImportFeature,
        action: ImportAction,
        name: &str,
    ) {
        self.add(feature, action, Vec::new(), Some(name))
    }
    pub(super) fn discard_layer(&mut self) {
        self.pending.clear();
        self.drop_layer = false;
    }
    fn add(
        &mut self,
        feature: ImportFeature,
        action: ImportAction,
        details: Vec<ImportDetail>,
        layer: Option<&str>,
    ) {
        let note = match self
            .notes
            .iter_mut()
            .find(|n| n.feature == feature && n.action == action)
        {
            Some(n) => n,
            None => {
                self.notes.push(ImportNote {
                    feature,
                    action,
                    layers: Vec::new(),
                    count: 0,
                    details: Vec::new(),
                });
                self.notes.last_mut().unwrap()
            }
        };
        note.count += 1;
        if let Some(name) = layer {
            if note.layers.len() < ImportNote::MAX_LAYERS {
                note.layers.push(name.to_owned());
            }
        }
        for d in details {
            if note.details.len() < ImportNote::MAX_DETAILS && !note.details.contains(&d) {
                note.details.push(d);
            }
        }
    }
}
impl State<'_> {
    pub(super) fn copy_note(
        &mut self,
        feature: ImportFeature,
        action: ImportAction,
        detail: Option<ImportDetail>,
    ) {
        let Some(c) = self.copy.as_mut() else { return };
        if c.in_layer {
            // 1 枚の層の中で同じ機能は 1 度だけ数える（細目は足す）
            if let Some(p) = c
                .pending
                .iter_mut()
                .find(|p| p.0 == feature && p.1 == action)
            {
                p.2.extend(detail);
                return;
            }
            c.pending
                .push((feature, action, detail.into_iter().collect()))
        } else {
            c.add(feature, action, detail.into_iter().collect(), None)
        }
    }
    fn refuse(&mut self, why: CopyRefusal) {
        if let Some(c) = self.copy.as_mut() {
            c.refusal.get_or_insert(why);
        }
    }
}

/// 原本を保つ読みが「保つだけ」にする理由（診断のコード）を、写しとしての取り込みでどう扱うかを決める表（`docs/PSD.md` の表と同じ）。
pub(super) fn sort_preserve(s: &mut State, code: &str) {
    use ImportAction::{Changed, Dropped, Ignored};
    use ImportFeature as F;
    let key = ImportDetail::Key(s.key);
    let (feature, action, detail) = match code {
        // 取れないもの（読み手が理由を見て断る。ヘッダーの種類は呼び手が付ける）
        "PSB" | "ColorFormat" | "NoLayers" => return,
        // 画素の無い層は、ふつうにある（空のレイヤー）
        "EmptyLayer" => return,
        "Compression" | "CompositeCompression" => {
            return s.refuse(CopyRefusal::Malformed(
                "未対応の圧縮方式です（RAW・RLE・ZIP だけ読めます）".into(),
            ))
        }
        // 層 ID は、全部の層を読んでから振り直す（`LayerIds`）
        "LayerIdentity" => return,
        // 合成にも内容にも効かない
        "ColorData" => (F::ColorModeData, Ignored, None),
        "LayerInfoTail" | "UnknownTail" => (F::LayerInfoTail, Ignored, None),
        "GlobalMask" => (F::GlobalMask, Ignored, None),
        "ExtraAlpha" => (F::ExtraChannel, Ignored, None),
        "UnicodeName" | "UnicodeNameTail" | "UnicodeNameNull" | "NameNull"
        | "LegacyNameEncoding" => (F::LayerName, Ignored, None),
        "LayerLocks" | "SheetColor" | "SectionDivider" | "GroupBlend" | "DividerPixels"
        | "DividerMask" | "AdjustmentPixels" | "MaskTail" => (F::LayerMetadata, Ignored, None),
        "LayerFlags" | "Clipping" => (F::LayerFlags, Ignored, None),
        "LayerChannel" => (F::LayerChannels, Ignored, None),
        "TransparencyShapes" => (F::TransparencyShapes, Ignored, None),
        // 合成が変わる（層の画素のまま取り込む）
        "BlendIf" => (F::BlendIf, Changed, None),
        "BlendMode" => (F::BlendMode, Changed, Some(key)),
        "FillOpacity" => (F::FillOpacity, Changed, None),
        "ClippedBlend" => (F::ClippedBlend, Changed, None),
        "InteriorBlend" => (F::InteriorBlend, Changed, None),
        "Knockout" => (F::Knockout, Changed, None),
        "ChannelRestrictions" => (F::ChannelRestrictions, Changed, None),
        "MaskPosition" | "MaskInvert" | "MaskFromRender" | "MaskFlags" => {
            (F::MaskFlags, Changed, None)
        }
        "MaskFeather" | "VectorMaskDensity" | "VectorMaskFeather" | "MaskParameters" => {
            (F::MaskFeather, Changed, None)
        }
        "UserAndVectorMask" | "RealUserMask" => (F::UserAndVectorMask, Changed, None),
        // 取り込めない形（グループ自身の画素）
        "GroupPixels" => (F::GroupPixels, Dropped, None),
        // RGB のチャンネルが揃わない層は、層ごと落とす
        "LayerChannels" => {
            if let Some(c) = s.copy.as_mut() {
                c.drop_layer = true
            }
            (F::MissingChannels, Dropped, None)
        }
        // タグ・調整・塗りつぶし: 読んでいるタグのキーで決める
        "TaggedBlock" | "Adjustment" | "Levels" | "HueSaturation" | "FillLayer" => {
            return sort_tag(s, s.key)
        }
        // 表に無い理由は、情報を持たないだけとして知らせる（黙って捨てない）
        _ => (F::LayerMetadata, Ignored, None),
    };
    s.copy_note(feature, action, detail)
}

/// 調整レイヤーのタグのキー（読めたもの・読めなかったもの・未対応のもの）。読めない内容なら層ごと落とし、
/// 知らせの細目にこのキーが付く（画面は名前にする。`ImportFeature::UnsupportedAdjustment`）。
/// 取り込んだ文書の合成と PSD の統合画像の差のうち、合成の 8 bit の丸めの違いとして知らせないもの（0〜255）。
/// CLIP STUDIO が書き出した実物の PSD 4 つ（4096²・7〜62 層）で、合成の最大の差は 2（差のある画素は 0.001% 以下）だった。
const COMPOSITE_TOLERANCE: u8 = 2;

pub const ADJUSTMENT_TAG_KEYS: [[u8; 4]; 18] = [
    *b"nvrt", *b"levl", *b"hue2", *b"hue ", *b"thrs", *b"post", *b"blnc", *b"curv", *b"grdm",
    *b"brit", *b"CgEd", *b"selc", *b"mixr", *b"phfl", *b"vibA", *b"blwh", *b"clrL", *b"expA",
];
fn adjustment_key(key: [u8; 4]) -> bool {
    ADJUSTMENT_TAG_KEYS.contains(&key)
}

/// 層のタグのキーから、取り込みでの扱いを決める。
fn sort_tag(s: &mut State, key: [u8; 4]) {
    use ImportAction::{Changed, Dropped, Ignored};
    use ImportFeature as F;
    let detail = Some(ImportDetail::Key(key));
    if adjustment_key(key) {
        // 画素を持たない調整は、表せなければ層ごと落とす
        if let Some(c) = s.copy.as_mut() {
            c.drop_layer = true
        }
        return s.copy_note(F::UnsupportedAdjustment, Dropped, detail);
    }
    let (feature, action) = match &key {
        b"lfx2" | b"lrFX" | b"lmfx" => (F::LayerEffects, Changed),
        b"TySh" | b"tySh" => (F::TextLayer, Dropped),
        b"SoLd" | b"PlLd" | b"plLd" | b"SoLE" => (F::SmartObject, Dropped),
        b"vmsk" | b"vsms" => (F::VectorMask, Changed),
        b"GdFl" | b"PtFl" | b"SoCo" => (F::FillSettings, Dropped),
        _ => return s.copy_note(F::LayerMetadata, Ignored, detail),
    };
    s.copy_note(feature, action, None)
}

// ───────── 取り込み ─────────

/// 画像リソースの ICC を読む上限。
const ICC_CAP: u32 = 16 * 1024 * 1024;
/// 統合画像と照らすときに、一度に合成する行の数。
const BAND_ROWS: u32 = 256;

/// 取り込みの途中の止まり方。
enum Stop {
    Refused(CopyRefusal),
    Error(Error),
}
impl From<Error> for Stop {
    fn from(e: Error) -> Self {
        Self::Error(e)
    }
}
impl From<std::io::Error> for Stop {
    fn from(e: std::io::Error) -> Self {
        Self::Error(Error::Io(e))
    }
}
impl From<yolu_core::CoreError> for Stop {
    fn from(e: yolu_core::CoreError) -> Self {
        Self::Error(Error::Core(e))
    }
}
type Step<T> = std::result::Result<T, Stop>;

/// どの層の読み込みで失敗したかを添える（壊れた PSD の理由を、層の番号つきで見られるように）。
fn in_layer(e: Error, number: usize) -> Stop {
    match e {
        Error::InvalidData(why) | Error::Budget(why) => {
            Stop::Error(Error::InvalidData(format!("レイヤー #{number}: {why}")))
        }
        other => Stop::Error(other),
    }
}
fn malformed<T>(why: impl Into<String>) -> Step<T> {
    Err(Stop::Refused(CopyRefusal::Malformed(why.into())))
}
fn cancelled(cancel: Option<&AtomicBool>) -> Step<()> {
    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
        Err(Stop::Error(Error::Core(yolu_core::CoreError::Cancelled)))
    } else {
        Ok(())
    }
}
fn read_u16<R: Read>(r: &mut R) -> Step<u16> {
    let mut b = [0; 2];
    r.read_exact(&mut b)?;
    Ok(u16::from_be_bytes(b))
}
fn read_u32<R: Read>(r: &mut R) -> Step<u32> {
    let mut b = [0; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_be_bytes(b))
}
fn tell<R: Seek>(r: &mut R) -> Step<u64> {
    Ok(r.stream_position()?)
}
fn skip<R: Seek>(r: &mut R, n: u64) -> Step<()> {
    let n = i64::try_from(n)
        .map_err(|_| Stop::Error(Error::InvalidData("区間長が大きすぎます".into())))?;
    r.seek(SeekFrom::Current(n))?;
    Ok(())
}

/// PSD（RGB8）を、原本を持たない写しとして core の文書へ取り込む。読めた文書と知らせ、または取り込めない理由を返す。
/// 取消の旗が立っていれば `Error::Core(Cancelled)`、ファイルの読み込みの失敗は `Error::Io`。壊れた PSD は `CopyRefusal::Malformed`。
pub fn import_copy<R: Read + Seek>(reader: &mut R, options: &CopyOptions) -> Result<CopyOutcome> {
    match run(reader, options) {
        Ok(outcome) => Ok(outcome),
        Err(Stop::Refused(why)) => Ok(CopyOutcome::Refused(why)),
        Err(Stop::Error(e)) => match e {
            Error::Core(yolu_core::CoreError::Cancelled) => Err(e),
            Error::Io(io) if io.kind() == std::io::ErrorKind::UnexpectedEof => Ok(
                CopyOutcome::Refused(CopyRefusal::Malformed("PSD が途中で切れています".into())),
            ),
            Error::InvalidData(why) | Error::Budget(why) => {
                Ok(CopyOutcome::Refused(CopyRefusal::Malformed(why)))
            }
            other => Err(other),
        },
    }
}

/// 書いた PSD の読み戻しの確かめの結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verified {
    /// 読み戻した層の数（グループの区切りの記録を除く。取り込んだ文書の層の数と同じ）。
    pub layers: usize,
    /// ファイルの大きさ。
    pub bytes: u64,
}

/// 書いた PSD を、最後まで流して読み戻して確かめる。`import_copy` と同じ読み手（層の記録・チャンネルの復号・統合画像の読み）を通すが、core の文書は
/// 作らない（層 1 枚ぶんのチャンネルしかメモリに持たず、焼いた画素が文書の予算を超えても確かめは止まらない）。全層・マスク・統合画像のすべての行を
/// 復号し、記録の長さがデータの長さと一致し、ファイルの終わりが統合画像の終わりであることを確かめる。取り込みで落とす・変わるものがあれば、
/// 自分の書き手が書いた PSD ではない（書き手の不具合）ので断る。壊れている・途中で切れているときは `Error::InvalidData`、取消は `Error::Core(Cancelled)`。
pub fn verify_stream<R: Read + Seek>(
    reader: &mut R,
    cancel: Option<&AtomicBool>,
) -> Result<Verified> {
    match verify(reader, cancel) {
        Ok(v) => Ok(v),
        Err(Stop::Refused(why)) => Err(Error::InvalidData(why.message())),
        Err(Stop::Error(Error::Io(io))) if io.kind() == std::io::ErrorKind::UnexpectedEof => {
            Err(Error::InvalidData("PSD が途中で切れています".into()))
        }
        Err(Stop::Error(e)) => Err(e),
    }
}

fn verify<R: Read + Seek>(r: &mut R, cancel: Option<&AtomicBool>) -> Step<Verified> {
    let limits = Limits {
        max_source_bytes: i32::MAX as usize,
        max_output_bytes: i32::MAX as usize,
        max_dimension: 30000,
        max_canvas_pixels: u64::MAX,
        max_layers: 32767,
        max_decoded_bytes: u64::MAX,
        max_metadata_bytes: usize::MAX,
        max_name_code_units: 4096,
        max_diagnostics: 128,
        max_group_depth: 1000,
    };
    let mut s = State {
        limits: &limits,
        cancel,
        notes: Vec::new(),
        unsupported: false,
        metadata: 0,
        pixels: 0,
        omitted: Vec::new(),
        key: [0; 4],
        copy: Some(CopyState::new()),
    };
    let total = r.seek(SeekFrom::End(0))?;
    r.seek(SeekFrom::Start(0))?;
    let mut head = [0u8; 26];
    r.read_exact(&mut head)?;
    let mut h = Reader::new(&head);
    if h.key()? != *b"8BPS" || h.u16()? != 1 {
        return malformed("PSD シグネチャ・版が不正です");
    }
    h.zeros(6)?;
    let channels = usize::from(h.u16()?);
    let (height, width) = (h.u32()?, h.u32()?);
    let (depth, mode) = (h.u16()?, h.u16()?);
    if depth != 8 || mode != 3 || !(3..=56).contains(&channels) || width == 0 || height == 0 {
        return malformed("PSD の寸法・色の形式が不正です");
    }
    for _ in 0..2 {
        // 色モードデータ・画像リソース
        let n = read_u32(r)?;
        if tell(r)? + u64::from(n) > total {
            return malformed("PSD が途中で切れています");
        }
        skip(r, u64::from(n))?;
    }
    let lm_len = read_u32(r)?;
    let lm_end = tell(r)? + u64::from(lm_len);
    let info_len = read_u32(r)?;
    let info_end = tell(r)? + u64::from(info_len);
    if lm_end > total || info_end > lm_end || info_len == 0 {
        return malformed("レイヤー情報の区間が不正です");
    }
    let signed = read_u16(r)? as i16;
    let count = i32::from(signed).unsigned_abs() as usize;
    if count == 0 || count > limits.max_layers {
        return malformed("層の数が不正です");
    }
    let mut records: Vec<read::Record> = Vec::with_capacity(count);
    for i in 0..count {
        cancelled(cancel)?;
        let mut head = [0u8; 18];
        r.read_exact(&mut head)?;
        let n = u16::from_be_bytes([head[16], head[17]]) as usize;
        if !(1..=56).contains(&n) {
            return malformed("レイヤーチャンネル数が不正です");
        }
        let mut mid = vec![0u8; n * 6 + 16];
        r.read_exact(&mut mid)?;
        let extra = u32::from_be_bytes(mid[mid.len() - 4..].try_into().unwrap());
        if tell(r)? + u64::from(extra) > info_end {
            return malformed("レイヤーの付加情報が区間を超えています");
        }
        let mut buf = Vec::with_capacity(head.len() + mid.len() + extra as usize);
        buf.extend(head);
        buf.extend(&mid);
        let at = buf.len();
        buf.resize(at + extra as usize, 0);
        r.read_exact(&mut buf[at..])?;
        let c = s.copy.as_mut().unwrap();
        c.in_layer = true;
        c.drop_layer = false;
        s.key = [0; 4];
        let mut rd = Reader::new(&buf);
        let rec = read::record(&mut rd, &mut s).map_err(|e| in_layer(e, i + 1))?;
        let c = s.copy.as_mut().unwrap();
        c.in_layer = false;
        if let Some(why) = c.take_refusal() {
            return Err(Stop::Refused(why));
        }
        // 層ごとの知らせは文書の知らせへ移す（取り込みで落とす・変わるものがあれば、最後に断る）。区切りは層ではない
        if rec.section == 3 {
            c.discard_layer();
        } else {
            c.flush_layer(&rec.layer.name);
            c.drop_layer = false;
        }
        records.push(rec);
    }
    // 全層・マスクのチャンネルを、最後の行まで復号する（層 1 枚・チャンネル 1 本ぶんだけ持つ）
    let mut plane: Vec<u8> = Vec::new();
    let mut buf: Vec<u8> = Vec::new();
    for (i, rec) in records.iter().enumerate() {
        cancelled(cancel)?;
        let (w, hh) = (rec.layer.width, rec.layer.height);
        let (mw, mh) = rec
            .layer
            .mask
            .as_ref()
            .map_or((0, 0), |m| (m.width, m.height));
        for &(id, len) in &rec.channels {
            let (cw, ch) = if id == -2 { (mw, mh) } else { (w, hh) };
            let bound = u64::from(cw) * u64::from(ch) * 2 + u64::from(ch) * 4 + 1024;
            if len < 2 || len as u64 > bound {
                return malformed(format!("レイヤー #{}: チャンネルの長さが不正です", i + 1));
            }
            if tell(r)? + len as u64 > info_end {
                return malformed("チャンネルが層の情報を超えています");
            }
            buf.resize(len, 0);
            r.read_exact(&mut buf)?;
            plane.clear();
            plane.resize(cw as usize * ch as usize, 0);
            read::decode(Reader::new(&buf), cw, ch, &mut plane, 0, 1, &mut s)
                .map_err(|e| in_layer(e, i + 1))?;
            if let Some(why) = s.copy.as_mut().unwrap().take_refusal() {
                return Err(Stop::Refused(why));
            }
        }
    }
    drop((plane, buf));
    let here = tell(r)?;
    // レイヤー情報は偶数の長さにそろえる（1 バイトの余白だけ許す）
    if here > info_end || info_end - here > 1 {
        return malformed("チャンネルのデータの長さが記録と一致しません");
    }
    r.seek(SeekFrom::Start(info_end))?;
    // 全体のレイヤーマスク情報（4 バイトの長さ）のあと、レイヤーとマスクの情報が終わる
    let global = read_u32(r)?;
    if tell(r)? + u64::from(global) != lm_end {
        return malformed("レイヤーとマスクの情報の長さが一致しません");
    }
    r.seek(SeekFrom::Start(lm_end))?;
    // 統合画像: 全チャンネルの全行を読み、ファイルの終わりで終わる
    let (w, hh) = (width as usize, height as usize);
    if read_merged(r, w, hh, channels, total, cancel)?.is_none() || tell(r)? != total {
        return malformed("統合画像が不正です（読めない・長さが一致しない）");
    }
    let notes = s.copy.take().unwrap().into_notes();
    // 画面の文にそのまま出るので、内部の種類の名前は入れない
    if notes.iter().any(|n| n.action != ImportAction::Ignored) {
        return malformed("読み戻した PSD に、取り込みで落とす・変わるものがあります");
    }
    Ok(Verified {
        layers: records.iter().filter(|r| r.section != 3).count(),
        bytes: total,
    })
}

/// 層 1 枚の読み込みの途中の姿（記録と、層ごとの落とす印）。
struct Parsed {
    rec: read::Record,
    dropped: bool,
}

fn run<R: Read + Seek>(r: &mut R, o: &CopyOptions) -> Step<CopyOutcome> {
    let limits = Limits {
        max_source_bytes: i32::MAX as usize,
        max_output_bytes: i32::MAX as usize,
        max_dimension: 30000,
        max_canvas_pixels: u64::MAX,
        max_layers: o.max_layers(),
        max_decoded_bytes: u64::MAX,
        max_metadata_bytes: usize::MAX,
        max_name_code_units: 4096,
        max_diagnostics: 128,
        max_group_depth: yolu_core::MAX_GROUP_DEPTH,
    };
    let mut s = State {
        limits: &limits,
        cancel: o.cancel,
        notes: Vec::new(),
        unsupported: false,
        metadata: 0,
        pixels: 0,
        omitted: Vec::new(),
        key: [0; 4],
        copy: Some(CopyState::new()),
    };
    let total = r.seek(SeekFrom::End(0))?;
    r.seek(SeekFrom::Start(0))?;

    // ヘッダー
    let mut head = [0u8; 26];
    r.read_exact(&mut head)?;
    let mut h = Reader::lenient(&head);
    if h.key()? != *b"8BPS" {
        return malformed("PSD シグネチャが不正です");
    }
    let version = h.u16()?;
    if version != 1 && version != 2 {
        return malformed("PSD 版が不正です");
    }
    h.zeros(6)?;
    let channels = h.u16()?;
    let height = h.u32()?;
    let width = h.u32()?;
    let depth = h.u16()?;
    let mode = h.u16()?;
    let max_edge = if version == 2 { 300000 } else { 30000 };
    if !(1..=56).contains(&channels)
        || width == 0
        || height == 0
        || width > max_edge
        || height > max_edge
    {
        return malformed("PSD 寸法・チャンネル数が不正です");
    }
    if version == 2 {
        return Err(Stop::Refused(CopyRefusal::LargeDocument));
    }
    // RGB 8 bit。アルファチャンネル（選択範囲など）で 4 チャンネルを超えるものも、統合画像の追加チャンネルとして読み飛ばして取り込む
    if depth != 8 || mode != 3 || channels < 3 {
        return Err(Stop::Refused(CopyRefusal::ColorFormat { depth, mode }));
    }
    // .ylp に保存できない大きさは、取り込んだあとで保存だけが止まる文書になる。予算とは別の上限なので、読む前に断る
    if width > crate::MAX_DOCUMENT_EDGE || height > crate::MAX_DOCUMENT_EDGE {
        return Err(Stop::Refused(CopyRefusal::EdgeOverLimit {
            width,
            height,
            limit: crate::MAX_DOCUMENT_EDGE,
        }));
    }
    let canvas_bytes = u64::from(width) * u64::from(height) * 4;
    if canvas_bytes > o.source_budget {
        return Err(Stop::Refused(CopyRefusal::CanvasTooLarge { width, height }));
    }

    // 色モードデータ
    let n = read_u32(r)?;
    if n > 0 {
        skip(r, u64::from(n))?;
        s.preserve("ColorData", "", 0, 0);
    }
    // 画像リソース
    let n = read_u32(r)?;
    let end = tell(r)? + u64::from(n);
    if end > total {
        return malformed("PSD が途中で切れています");
    }
    resources(r, end, &mut s)?;

    // レイヤーとマスクの情報
    let lm_len = read_u32(r)?;
    let lm_start = tell(r)?;
    let lm_end = lm_start + u64::from(lm_len);
    if lm_len == 0 {
        return Err(Stop::Refused(CopyRefusal::NoLayers));
    }
    if lm_end > total {
        return malformed("PSD が途中で切れています");
    }
    let info_len = read_u32(r)?;
    let info_start = tell(r)?;
    let info_end = info_start + u64::from(info_len);
    if info_len == 0 {
        return Err(Stop::Refused(CopyRefusal::NoLayers));
    }
    if info_end > lm_end {
        return malformed("レイヤー情報が区間を超えています");
    }
    let signed = read_u16(r)? as i16;
    // 統合画像の透明度は、層の数が負で、4 つ目のチャンネルがあるとき
    let merged_alpha = signed < 0 && channels >= 4;
    if usize::from(channels) > 3 + usize::from(merged_alpha) {
        s.preserve("ExtraAlpha", "", 0, 0);
    }
    let count = i32::from(signed).unsigned_abs() as usize;
    if count == 0 {
        return Err(Stop::Refused(CopyRefusal::NoLayers));
    }
    if count > limits.max_layers {
        return Err(Stop::Refused(CopyRefusal::TooManyLayers {
            count,
            limit: limits.max_layers,
        }));
    }

    // 層の記録（付加情報は 1 枚ずつ読んで、知らせだけ残す）
    let mut parsed: Vec<Parsed> = Vec::with_capacity(count);
    for i in 0..count {
        cancelled(o.cancel)?;
        let mut head = [0u8; 18];
        r.read_exact(&mut head)?;
        let n = u16::from_be_bytes([head[16], head[17]]) as usize;
        if !(1..=56).contains(&n) {
            return malformed("レイヤーチャンネル数が不正です");
        }
        let mut mid = vec![0u8; n * 6 + 16];
        r.read_exact(&mut mid)?;
        let extra = u32::from_be_bytes(mid[mid.len() - 4..].try_into().unwrap());
        if tell(r)? + u64::from(extra) > info_end {
            return malformed("レイヤーの付加情報が区間を超えています");
        }
        // 付加情報は層ごとに全部メモリへ読む。固定の上限でなく、層の画素と同じ予算（設定の「レイヤーのメモリ」）に合わせる
        if u64::from(extra) > o.source_budget {
            return Err(Stop::Refused(CopyRefusal::LayerDataTooLarge {
                layer: format!("#{}", i + 1),
            }));
        }
        let mut buf = Vec::with_capacity(head.len() + mid.len() + extra as usize);
        buf.extend(head);
        buf.extend(&mid);
        let at = buf.len();
        buf.resize(at + extra as usize, 0);
        r.read_exact(&mut buf[at..])?;
        let c = s.copy.as_mut().unwrap();
        c.in_layer = true;
        c.drop_layer = false;
        s.key = [0; 4];
        let mut rd = Reader::lenient(&buf);
        let mut rec = read::record(&mut rd, &mut s).map_err(|e| in_layer(e, i + 1))?;
        let c = s.copy.as_mut().unwrap();
        if let Some(why) = c.take_refusal() {
            return Err(Stop::Refused(why));
        }
        // マスクの定義があってチャンネルが無ければ、値は分からないので、矩形を空にして既定値だけにする
        // （矩形が空なら元から既定値だけなので、何も失わず知らせない）
        let mut mask_values_lost = false;
        if !rec.channels.iter().any(|c| c.0 == -2) {
            if let Some(m) = rec.layer.mask.as_mut() {
                mask_values_lost = m.width > 0 && m.height > 0;
                m.width = 0;
                m.height = 0;
            }
        }
        let plain = !matches!(rec.section, 1..=3);
        let mut dropped = c.drop_layer && plain;
        c.drop_layer = false;
        if mask_values_lost && !dropped {
            s.copy_note(
                ImportFeature::MaskWithoutPixels,
                ImportAction::Changed,
                None,
            );
        }
        s.copy.as_mut().unwrap().in_layer = false;
        // 形のあるシェイプ（塗りつぶし + ベクターマスク）は、塗りの設定でなく層の画素（描画された形）で取り込む
        if rec.vector_mask && rec.fill_seen && matches!(rec.layer.kind, LayerKind::SolidColor(_)) {
            rec.layer.kind = LayerKind::Raster;
            s.copy.as_mut().unwrap().in_layer = true;
            s.copy_note(ImportFeature::FillSettings, ImportAction::Dropped, None);
            s.copy.as_mut().unwrap().in_layer = false;
        }
        if rec.section == 3 {
            // 区切り: 層ではない（知らせは出さない）
            s.copy.as_mut().unwrap().discard_layer();
            dropped = false;
        } else {
            if rec.fill_opacity != 255 {
                let v = (u32::from(rec.layer.opacity) * u32::from(rec.fill_opacity) + 127) / 255;
                rec.layer.opacity = v as u8;
            }
            let name = rec.layer.name.clone();
            s.copy.as_mut().unwrap().flush_layer(&name);
        }
        parsed.push(Parsed { rec, dropped });
    }

    // 層の並び（下から上）と、グループの親。落とした層は並びに入れない
    let mut items: Vec<usize> = Vec::new();
    let mut item_of: Vec<Option<usize>> = vec![None; parsed.len()];
    let mut parent: Vec<Option<usize>> = Vec::new();
    let mut group_divider: Vec<i32> = Vec::new();
    let mut open: Vec<(Vec<usize>, i32)> = Vec::new();
    for (i, p) in parsed.iter().enumerate() {
        match p.rec.section {
            3 => {
                if open.len() >= limits.max_group_depth {
                    return Err(Stop::Refused(CopyRefusal::NestingTooDeep {
                        limit: limits.max_group_depth,
                    }));
                }
                open.push((Vec::new(), p.rec.layer.id))
            }
            1 | 2 => {
                let Some((children, divider)) = open.pop() else {
                    return malformed("区切りのないグループ");
                };
                let g = items.len();
                items.push(i);
                item_of[i] = Some(g);
                parent.push(None);
                group_divider.push(divider);
                for c in children {
                    parent[c] = Some(g)
                }
                if let Some((siblings, _)) = open.last_mut() {
                    siblings.push(g)
                }
            }
            _ if p.dropped => {}
            _ => {
                let k = items.len();
                items.push(i);
                item_of[i] = Some(k);
                parent.push(None);
                group_divider.push(0);
                if let Some((siblings, _)) = open.last_mut() {
                    siblings.push(k)
                }
            }
        }
    }
    if !open.is_empty() {
        return malformed("閉じていないグループ区切り");
    }
    // 文書の層の数（区切りの記録と、落とした層を除く）。.ylp に書ける数を超えれば、画素を読む前に断る
    if items.len() > crate::MAX_DOCUMENT_LAYERS {
        return Err(Stop::Refused(CopyRefusal::LayerCountOverLimit {
            count: items.len(),
            limit: crate::MAX_DOCUMENT_LAYERS,
        }));
    }

    // 層 ID: 欠落・重複・不正だけに新しい ID を振る（名前では補修しない）。
    // 先に全部の層を見て、正で最初に出た一意な ID（区切りの ID を含む）をすべて取っておき、そのあとで振る。
    // 見ながら振ると、あとに出てくる有効な ID と振った ID がぶつかって、有効な層まで振り直されてしまう
    let mut used: std::collections::HashSet<i32> = parsed
        .iter()
        .filter(|p| p.rec.section == 3 && p.rec.layer.id > 0)
        .map(|p| p.rec.layer.id)
        .collect();
    let keep: Vec<bool> = items
        .iter()
        .map(|&i| {
            let id = parsed[i].rec.layer.id;
            id > 0 && used.insert(id)
        })
        .collect();
    let mut next = 1i32;
    let mut renumbered: Vec<usize> = Vec::new();
    for (&i, keep) in items.iter().zip(keep) {
        if keep {
            continue;
        }
        while !used.insert(next) {
            next += 1
        }
        parsed[i].rec.layer.id = next;
        renumbered.push(i);
    }
    {
        let c = s.copy.as_mut().unwrap();
        for i in renumbered {
            let name = parsed[i].rec.layer.name.clone();
            c.add_layer_note(ImportFeature::LayerIds, ImportAction::Ignored, &name);
        }
    }

    // 層の画素: 1 枚ずつ復号して core へ入れ、すぐ捨てる
    let mut d = yolu_core::Document::new(width, height)?;
    d.set_source_budget_bytes(o.source_budget)?;
    let canvas = Document {
        width,
        height,
        layers: Vec::new(),
        composite_rgba: None,
    };
    let salt = d.id() & ((1u128 << 64) - 1);
    let mut made: Vec<yolu_core::LayerId> = Vec::with_capacity(items.len());
    for (i, p) in parsed.iter_mut().enumerate() {
        cancelled(o.cancel)?;
        let kept = item_of[i].is_some();
        let channel_list = p.rec.channels.clone();
        let l = &mut p.rec.layer;
        let raster = kept && matches!(l.kind, LayerKind::Raster);
        let (w, hh) = (l.width, l.height);
        let layer_bytes = u64::from(w) * u64::from(hh) * 4;
        if raster && layer_bytes > o.source_budget {
            return Err(Stop::Refused(CopyRefusal::LayerTooLarge {
                layer: l.name.clone(),
            }));
        }
        if raster && layer_bytes > 0 {
            l.pixels_rgba = vec![0; layer_bytes as usize];
            for px in l.pixels_rgba.as_chunks_mut::<4>().0 {
                px[3] = 255
            }
        }
        if let Some(m) = &mut l.mask {
            let n = u64::from(m.width) * u64::from(m.height);
            if kept && n > o.source_budget {
                return Err(Stop::Refused(CopyRefusal::LayerTooLarge {
                    layer: l.name.clone(),
                }));
            }
            if kept && n > 0 {
                m.pixels = vec![0; n as usize]
            }
        }
        for (id, len) in channel_list {
            let wanted = match id {
                -2 => kept && l.mask.as_ref().is_some_and(|m| m.width > 0 && m.height > 0),
                -1..=2 => raster && layer_bytes > 0,
                _ => false,
            };
            if !wanted {
                skip(r, len as u64)?;
                continue;
            }
            let (cw, ch) = if id == -2 {
                let m = l.mask.as_ref().unwrap();
                (m.width, m.height)
            } else {
                (w, hh)
            };
            let bound = u64::from(cw) * u64::from(ch) * 2 + u64::from(ch) * 4 + 1024;
            if len as u64 > bound {
                return malformed("チャンネルの長さが不正です");
            }
            if tell(r)? + len as u64 > info_end {
                return malformed("チャンネルが層の情報を超えています");
            }
            let mut buf = vec![0u8; len];
            r.read_exact(&mut buf)?;
            if buf.len() < 2 {
                continue;
            }
            let decoded = if id == -2 {
                let m = l.mask.as_mut().unwrap();
                read::decode(Reader::lenient(&buf), cw, ch, &mut m.pixels, 0, 1, &mut s)
            } else {
                let component = if id == -1 { 3 } else { id as usize };
                read::decode(
                    Reader::lenient(&buf),
                    cw,
                    ch,
                    &mut l.pixels_rgba,
                    component,
                    4,
                    &mut s,
                )
            };
            decoded.map_err(|e| in_layer(e, i + 1))?;
            if let Some(why) = s.copy.as_mut().unwrap().take_refusal() {
                return Err(Stop::Refused(why));
            }
        }
        if !kept {
            continue;
        }
        let name = l.name.clone();
        let budget_stop = |e: Error| match e {
            Error::Core(yolu_core::CoreError::SourceBudgetExceeded) => {
                Stop::Refused(CopyRefusal::BudgetExceeded {
                    layer: name.clone(),
                })
            }
            other => Stop::Error(other),
        };
        let mut note = Vec::new();
        let id = match &l.kind {
            LayerKind::Raster => {
                let id = d.add_layer(&l.name)?;
                if canvas
                    .import_pixels_clipped(&mut d, id, l)
                    .map_err(budget_stop)?
                {
                    note.push(ImportFeature::OutsideCanvas)
                }
                id
            }
            LayerKind::Group { .. } => d.add_group(&l.name, None)?,
            LayerKind::SolidColor([cr, cg, cb]) => d.add_fill_layer(
                &l.name,
                &[(
                    yolu_core::Channel::Color,
                    yolu_core::Rgba8::new(*cr, *cg, *cb, 255),
                )],
                None,
            )?,
            LayerKind::Adjustment(a) => {
                d.add_adjustment_layer(&l.name, super::bridge::core_adjustment(a)?, None, None)?
            }
        };
        d.set_layer_visible(id, l.visible)?;
        d.set_layer_opacity(id, f64::from(l.opacity) / 255.0, false)?;
        d.set_layer_blend_mode(
            id,
            yolu_core::BlendMode::from_index(l.blend_mode as u8).unwrap(),
        )?;
        d.set_layer_clipping(id, l.clipping)?;
        if let Some(m) = &l.mask {
            if super::bridge::mask_off_canvas(m, width, height) {
                note.push(ImportFeature::MaskOutsideCanvas)
            }
            canvas.import_mask(&mut d, id, m).map_err(budget_stop)?;
        }
        made.push(id);
        // 復号した画素は、core へ入れたのでもう要らない
        l.pixels_rgba = Vec::new();
        if let Some(m) = &mut l.mask {
            m.pixels = Vec::new()
        }
        let c = s.copy.as_mut().unwrap();
        for feature in note {
            c.add_layer_note(feature, ImportAction::Dropped, &name);
        }
    }
    let here = tell(r)?;
    if here > info_end {
        return malformed("チャンネルのデータがレイヤー情報を超えています");
    }
    if info_end - here > 1 {
        s.preserve("LayerInfoTail", "", 0, 0);
    }
    r.seek(SeekFrom::Start(info_end))?;

    // 構造・ロック・ID
    let parents: Vec<Option<yolu_core::LayerId>> =
        parent.iter().map(|p| p.map(|p| made[p])).collect();
    if parents.iter().any(Option::is_some) {
        d.set_structure_for_load(&parents)?;
    }
    for (k, &i) in items.iter().enumerate() {
        let locks = parsed[i].rec.layer.locks;
        if locks != 0 {
            d.set_locks_for_load(made[k], super::bridge::core_locks(locks))?;
        }
    }
    let ids: Vec<yolu_core::LayerId> = items
        .iter()
        .enumerate()
        .map(|(k, &i)| {
            let divider = match parsed[i].rec.layer.kind {
                LayerKind::Group { .. } => super::bridge::divider_bits(group_divider[k]),
                _ => 0,
            };
            yolu_core::LayerId(((parsed[i].rec.layer.id as u128) << 96) | divider | salt)
        })
        .collect();
    let doc_id = d.id();
    let d = d.with_persistent_ids(doc_id, &ids)?;
    drop(parsed);

    // 全体マスクと文書のタグ（持たない。読み飛ばす）
    let mut here = tell(r)?;
    if lm_end.saturating_sub(here) >= 4 {
        let n = read_u32(r)?;
        here += 4;
        if n > 0 {
            s.preserve("GlobalMask", "", 0, 0);
        }
        if here + u64::from(n) > lm_end {
            return malformed("全体マスクが区間を超えています");
        }
        skip(r, u64::from(n))?;
        here += u64::from(n);
    }
    while lm_end.saturating_sub(here) >= 12 {
        let mut tag = [0u8; 12];
        r.read_exact(&mut tag)?;
        let n = u64::from(u32::from_be_bytes(tag[8..12].try_into().unwrap()));
        let size = n + (n & 1);
        here += 12;
        if here + n > lm_end {
            break;
        }
        s.key = tag[4..8].try_into().unwrap();
        s.copy_note(
            ImportFeature::DocumentTags,
            ImportAction::Ignored,
            Some(ImportDetail::Key(s.key)),
        );
        let step = size.min(lm_end - here);
        skip(r, step)?;
        here += step;
    }
    r.seek(SeekFrom::Start(lm_end))?;

    // 統合画像と照らす
    check_composite(r, &d, channels, merged_alpha, total, o, &mut s)?;
    let notes = s.copy.take().unwrap().into_notes();
    Ok(CopyOutcome::Imported(Box::new(CopyImport {
        document: d,
        notes,
    })))
}

/// 画像リソースを読み飛ばす（色に効くものだけ中身を見る）。
fn resources<R: Read + Seek>(r: &mut R, end: u64, s: &mut State) -> Step<()> {
    let mut here = tell(r)?;
    while end.saturating_sub(here) >= 12 {
        let mut head = [0u8; 7];
        r.read_exact(&mut head)?;
        if head[..4] != *b"8BIM" {
            // 画像リソースの形でない並び（色に効かない領域）: 読み進めず、区間ごと持たない
            s.copy_note(ImportFeature::ImageResources, ImportAction::Ignored, None);
            break;
        }
        let id = u16::from_be_bytes([head[4], head[5]]);
        let name = u64::from(head[6]);
        // 名前（1 バイトの長さを含めて偶数にそろえる）
        skip(r, name + u64::from((name + 1) % 2 != 0))?;
        let size = read_u32(r)?;
        here = tell(r)?;
        if here + u64::from(size) > end {
            return malformed("画像リソースが区間を超えています");
        }
        let pad = u64::from(size) % 2;
        match id {
            1039 if size <= ICC_CAP => {
                let mut body = vec![0u8; size as usize];
                r.read_exact(&mut body)?;
                if super::read::icc(&Reader::new(&body)).as_deref() != Some("sRGB IEC61966-2.1") {
                    s.copy_note(ImportFeature::ColorProfile, ImportAction::Changed, None)
                }
            }
            1064 if size == 12 => {
                let mut body = [0u8; 12];
                r.read_exact(&mut body)?;
                if f64::from_be_bytes(body[4..12].try_into().unwrap()) != 1.0 {
                    s.copy_note(ImportFeature::PixelAspect, ImportAction::Changed, None)
                }
            }
            _ => {
                skip(r, u64::from(size))?;
                s.copy_note(ImportFeature::ImageResources, ImportAction::Ignored, None)
            }
        }
        skip(r, pad)?;
        here = tell(r)?;
    }
    r.seek(SeekFrom::Start(end))?;
    Ok(())
}

/// 取り込んだ文書の合成を、PSD の統合画像と照らす（行の帯ごとに合成して、2 枚目の画像を持たない）。
fn check_composite<R: Read + Seek>(
    r: &mut R,
    d: &yolu_core::Document,
    channels: u16,
    merged_alpha: bool,
    total: u64,
    o: &CopyOptions,
    s: &mut State,
) -> Step<()> {
    let (w, h) = (d.width() as usize, d.height() as usize);
    let unchecked = |s: &mut State, why: Unchecked| -> Step<()> {
        s.copy_note(
            ImportFeature::CompositeUnchecked(why),
            ImportAction::Ignored,
            None,
        );
        Ok(())
    };
    // 統合画像（1 枚）と、帯の合成が予算に入るか
    let band = (w * 4 * h.min(BAND_ROWS as usize)) as u64;
    if (w * h * 4) as u64 + band > o.source_budget {
        return unchecked(s, Unchecked::Budget);
    }
    let here = tell(r)?;
    if total.saturating_sub(here) < 2 {
        return unchecked(s, Unchecked::NoComposite);
    }
    let Some(merged) = read_merged(r, w, h, usize::from(channels), total, o.cancel)? else {
        return unchecked(s, Unchecked::Unreadable);
    };
    let compared_channels = if merged_alpha { 4 } else { 3 };
    let (mut max_diff, mut differing) = (0u8, 0u64);
    let mut y = 0u32;
    while (y as usize) < h {
        cancelled(o.cancel)?;
        let rows = BAND_ROWS.min(d.height() - y);
        let mut out = d.composite(yolu_core::Rect::new(0, y, d.width(), rows))?;
        super::composite::matte(&mut out);
        for row in 0..rows as usize {
            // core の行は下から、統合画像は上から
            let psd_row = h - 1 - (y as usize + row);
            let ours = &out[row * w * 4..(row + 1) * w * 4];
            let theirs = &merged[psd_row * w * 4..(psd_row + 1) * w * 4];
            for (a, b) in ours
                .as_chunks::<4>()
                .0
                .iter()
                .zip(theirs.as_chunks::<4>().0)
            {
                let worst = (0..compared_channels)
                    .map(|c| a[c].abs_diff(b[c]))
                    .max()
                    .unwrap_or(0);
                max_diff = max_diff.max(worst);
                differing += u64::from(worst > COMPOSITE_TOLERANCE);
            }
        }
        y += rows;
    }
    if max_diff > COMPOSITE_TOLERANCE {
        s.copy_note(
            ImportFeature::CompositeDiffers {
                max_diff,
                differing,
                total: (w * h) as u64,
            },
            ImportAction::Changed,
            None,
        );
    }
    Ok(())
}

/// 統合画像を RGBA の 1 枚（上の行から）へ。圧縮は RAW・RLE・ZIP。読めなければ None。
fn read_merged<R: Read + Seek>(
    r: &mut R,
    w: usize,
    h: usize,
    channels: usize,
    total: u64,
    cancel: Option<&AtomicBool>,
) -> Step<Option<Vec<u8>>> {
    let compression = read_u16(r)?;
    let mut out = vec![0u8; w * h * 4];
    for px in out.as_chunks_mut::<4>().0 {
        px[3] = 255
    }
    // 色（と透明度）の先頭の 4 面だけ読む。残りのチャンネル（アルファチャンネル）は要らない
    let planes = channels.min(4);
    match compression {
        0 => {
            let need = (w * h * planes) as u64;
            if total.saturating_sub(tell(r)?) < need {
                return Ok(None);
            }
            let mut row = vec![0u8; w];
            for c in 0..planes {
                for y in 0..h {
                    if y % 64 == 0 {
                        cancelled(cancel)?;
                    }
                    r.read_exact(&mut row)?;
                    for (x, v) in row.iter().enumerate() {
                        out[(y * w + x) * 4 + c] = *v
                    }
                }
            }
        }
        1 => {
            // 表と各行が残りの長さに入らなければ（途中で切れている）、RAW・ZIP と同じく読めない統合画像として扱う
            let mut left = total.saturating_sub(tell(r)?);
            let table_len = (channels * h * 2) as u64;
            if left < table_len {
                return Ok(None);
            }
            left -= table_len;
            let mut table = vec![0u8; table_len as usize];
            r.read_exact(&mut table)?;
            let mut data = Vec::new();
            for c in 0..planes {
                cancelled(cancel)?;
                for y in 0..h {
                    let i = (c * h + y) * 2;
                    let n = usize::from(u16::from_be_bytes([table[i], table[i + 1]]));
                    if n as u64 > left {
                        return Ok(None);
                    }
                    left -= n as u64;
                    data.resize(n, 0);
                    r.read_exact(&mut data)?;
                    let mut line = Reader::lenient(&data);
                    if unpack_row(&mut line, w, &mut out, y * w * 4 + c).is_err() {
                        return Ok(None);
                    }
                }
            }
        }
        2 | 3 => {
            // ZIP は全チャンネルを 1 本の流れで持つ（予測つきは行ごとに差分）
            let left = total.saturating_sub(tell(r)?);
            let mut z = flate2::read::ZlibDecoder::new(r.by_ref().take(left));
            let mut row = vec![0u8; w];
            for c in 0..planes {
                for y in 0..h {
                    if y % 64 == 0 {
                        cancelled(cancel)?;
                    }
                    if z.read_exact(&mut row).is_err() {
                        return Ok(None);
                    }
                    if compression == 3 {
                        for x in 1..w {
                            row[x] = row[x].wrapping_add(row[x - 1])
                        }
                    }
                    for (x, v) in row.iter().enumerate() {
                        out[(y * w + x) * 4 + c] = *v
                    }
                }
            }
        }
        _ => return Ok(None),
    }
    Ok(Some(out))
}

/// PackBits の 1 行を、RGBA の 1 つの成分へ（`read::packbits` と同じ形で、幅を満たさなければ失敗）。
fn unpack_row(r: &mut Reader, w: usize, out: &mut [u8], offset: usize) -> Result<()> {
    let mut x = 0;
    while r.remaining() > 0 {
        let control = r.u8()? as i8;
        if control == -128 {
            continue;
        }
        let n = if control >= 0 {
            control as usize + 1
        } else {
            (1 - i16::from(control)) as usize
        };
        crate::check(n <= w - x, "RLE 行が幅を超えています")?;
        if control >= 0 {
            for b in r.take(n)? {
                out[offset + x * 4] = *b;
                x += 1
            }
        } else {
            let b = r.u8()?;
            for _ in 0..n {
                out[offset + x * 4] = b;
                x += 1
            }
        }
    }
    crate::check(x == w, "RLE 行が幅を満たしていません")
}
