//! 非破壊の効果（フィルターのスタック・Generator の段・Anchor・塗りつぶしの画像と投影・グラデーション）の文書側の型。
//!
//! Unity 版の `PaintDocument.Filters / Generators / Anchors / FillImages / FillGradients` と同じ持ち方で、層と層のマスクが設定を持つ。
//! 評価の式は文書に依らない口（[`crate::filter`]・[`crate::generator`]・[`crate::fill_image`]）にあり、ここは設定の入れ物と検査だけを持つ
//! （式を二重に持たない）。評価の結果は合成のときに作り、保存の正本にしない。
//!
//! - フィルターのスタックは層の内容（チャンネルごとに適用する段を選ぶ）と層のマスク（全チャンネルで共有する 1 つのスカラー）にそれぞれある。
//!   Generator は同じスタックの段（C# の `FilterType.Generator`）で、焼いたメッシュマップ・Anchor から値を作る。
//! - 効果を置けるのは標準のチャンネル（0〜5）だけ。正本の読み手（Unity 版も）が標準のチャンネルだけを許すので、保存で失わないようにここで断る。
//! - メッシュマップ・プロジェクトの画像は文書の外にある。[`EffectInputs`] で渡す（保存も Undo もしない）。

use std::fmt;

use crate::error::CoreError;
use crate::filter::{self, ValueType};
use crate::generator;
use crate::types::{Channel, ChannelKind};

pub mod catalog;
pub(crate) mod inputs;
pub use inputs::{EffectInputs, ImageInput, MapInput, ModelFrame};

/// 1 つのスタックに置ける段の数（C# の `MaxFiltersPerStack`）。
pub const MAX_FILTERS_PER_STACK: usize = filter::MAX_STACK;
/// スタックの到達半径（有効なぼかし・シャープの半径の和。チャンネルごと・マスク）の上限（C# の `MaxFilterStackHalo`）。
pub const MAX_FILTER_STACK_HALO: u32 = filter::MAX_HALO;

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub u128);
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({:032x})"), self.0)
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{:032x}", self.0)
            }
        }
    };
}
id_type!(
    /// スタックの段の ID（C# の Guid と同じ 128 bit。0 は使わない）。文書の中で重ならない。
    FilterId
);
id_type!(
    /// Anchor の ID（0 は使わない）。文書の中で重ならない。
    AnchorId
);
id_type!(
    /// プロジェクトの画像リソースの ID（塗りつぶしの画像が指す。0 は使わない）。
    ImageId
);

/// どちらのスタックか: 層の画素（内容。チャンネルごと）か、層のラスターマスク（隠す量。全チャンネルで共有）か。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FilterTarget {
    Content,
    Mask,
}

/// スタックの 1 つの段の中身。フィルター（ぼかし・シャープ・ノイズ・レベル補正・反転・正規化）か Generator。
#[derive(Clone, Debug, PartialEq)]
pub enum EffectSettings {
    /// Generator の段以外のフィルター（`filter::Settings::Generator` は置けない。Generator は [`EffectSettings::Generator`]）。
    Filter(filter::Settings),
    Generator(Box<generator::Settings>),
}

impl EffectSettings {
    pub fn blur(radius: u32) -> Self {
        Self::Filter(filter::Settings::GaussianBlur { radius })
    }
    pub fn sharpen(radius: u32, amount: f64, threshold: u32) -> Self {
        Self::Filter(filter::Settings::Sharpen {
            radius,
            amount,
            threshold,
        })
    }
    pub fn noise(amount: f64, seed: i32, monochrome: bool) -> Self {
        Self::Filter(filter::Settings::Noise {
            amount,
            seed,
            monochrome,
        })
    }
    pub fn levels(
        input_black: f64,
        input_white: f64,
        gamma: f64,
        output_black: f64,
        output_white: f64,
    ) -> Self {
        Self::Filter(filter::Settings::Levels {
            input_black,
            input_white,
            gamma,
            output_black,
            output_white,
        })
    }
    pub fn invert() -> Self {
        Self::Filter(filter::Settings::Invert)
    }
    pub fn normalize() -> Self {
        Self::Filter(filter::Settings::Normalize)
    }
    pub fn generator(settings: generator::Settings) -> Self {
        Self::Generator(Box::new(settings))
    }
    /// 色調補正の 6 種のどれかの段（`ColorAdjust` から）。
    pub fn from_color_adjust(value: crate::ColorAdjust) -> Self {
        match value {
            crate::ColorAdjust::GradientMap(v) => Self::gradient_map(v),
            crate::ColorAdjust::ToneCurve(v) => Self::tone_curve(v),
            crate::ColorAdjust::ColorBalance(v) => Self::color_balance(v),
            crate::ColorAdjust::BrightnessContrast(v) => Self::brightness_contrast(v),
            crate::ColorAdjust::Threshold(v) => Self::threshold(v),
            crate::ColorAdjust::Posterize(v) => Self::posterize(v),
        }
    }
    /// 色調補正の段なら、その値。ほかの段は None。
    pub fn color_adjust(&self) -> Option<crate::ColorAdjust> {
        use crate::ColorAdjust as C;
        let Self::Filter(f) = self else { return None };
        Some(match f {
            filter::Settings::GradientMap(v) => C::GradientMap(v.clone()),
            filter::Settings::ToneCurve(v) => C::ToneCurve(v.clone()),
            filter::Settings::ColorBalance(v) => C::ColorBalance(*v),
            filter::Settings::BrightnessContrast(v) => C::BrightnessContrast(v.clone()),
            filter::Settings::Threshold(v) => C::Threshold(*v),
            filter::Settings::Posterize(v) => C::Posterize(*v),
            _ => return None,
        })
    }
    /// グラデーションマップ（色のチャンネルだけ。Rust 版だけの種類）。
    pub fn gradient_map(map: crate::GradientMap) -> Self {
        Self::Filter(filter::Settings::GradientMap(map))
    }
    /// トーンカーブ（スカラーでは RGB 全体の曲線だけ。Rust 版だけの種類）。
    pub fn tone_curve(curves: crate::ToneCurves) -> Self {
        Self::Filter(filter::Settings::ToneCurve(curves))
    }
    /// カラーバランス（色のチャンネルだけ。Rust 版だけの種類）。
    pub fn color_balance(balance: crate::ColorBalance) -> Self {
        Self::Filter(filter::Settings::ColorBalance(balance))
    }
    pub fn brightness_contrast(value: crate::BrightnessContrast) -> Self {
        Self::Filter(filter::Settings::BrightnessContrast(value))
    }
    pub fn threshold(value: crate::Threshold) -> Self {
        Self::Filter(filter::Settings::Threshold(value))
    }
    pub fn posterize(value: crate::Posterize) -> Self {
        Self::Filter(filter::Settings::Posterize(value))
    }

    /// 保存形式の段の種類の番号（ぼかし 0・シャープ 1・ノイズ 2・レベル補正 3・反転 4・正規化 5・Generator 6。Rust 版だけの種類は 64 から:
    /// グラデーションマップ 64・トーンカーブ 65・カラーバランス 66・明るさ/コントラスト 67・2 値化 68・ポスタリゼーション 69）。
    pub fn type_index(&self) -> i32 {
        match self {
            Self::Filter(filter::Settings::GaussianBlur { .. }) => 0,
            Self::Filter(filter::Settings::Sharpen { .. }) => 1,
            Self::Filter(filter::Settings::Noise { .. }) => 2,
            Self::Filter(filter::Settings::Levels { .. }) => 3,
            Self::Filter(filter::Settings::Invert) => 4,
            Self::Filter(filter::Settings::Normalize) => 5,
            Self::Filter(filter::Settings::GradientMap(_)) => 64,
            Self::Filter(filter::Settings::ToneCurve(_)) => 65,
            Self::Filter(filter::Settings::ColorBalance(_)) => 66,
            Self::Filter(filter::Settings::BrightnessContrast(_)) => 67,
            Self::Filter(filter::Settings::Threshold(_)) => 68,
            Self::Filter(filter::Settings::Posterize(_)) => 69,
            // 評価器の中の Generator の段（slot と合成）は文書では Generator として持つので、ここへは来ない
            Self::Filter(filter::Settings::Generator { .. }) | Self::Generator(_) => 6,
        }
    }
    pub fn is_generator(&self) -> bool {
        matches!(self, Self::Generator(_))
    }
    /// Anchor を読む Generator か。
    pub fn reads_anchor(&self) -> bool {
        matches!(self, Self::Generator(g) if g.kind == generator::Kind::Anchor)
    }
    pub fn generator_settings(&self) -> Option<&generator::Settings> {
        match self {
            Self::Generator(g) => Some(g),
            Self::Filter(_) => None,
        }
    }
    /// 入力の画素が出力に届く距離（ぼかし・シャープの半径。ほかは 0）。
    pub fn halo(&self) -> u32 {
        match self {
            Self::Filter(f) => f.halo(),
            Self::Generator(_) => 0,
        }
    }
    /// 段への入力の全体で決まる段（正規化）か。
    pub fn is_global(&self) -> bool {
        matches!(self, Self::Filter(filter::Settings::Normalize))
    }
    /// 透明な所へ不透明を広げる段（ぼかしだけ）か。
    pub fn expands_coverage(&self) -> bool {
        matches!(self, Self::Filter(filter::Settings::GaussianBlur { .. }))
    }
    /// マスク（不透明な灰色の画像）で、半径の中がすべて 0 の所が 0 のままか（C# の PreservesZero）。
    pub(crate) fn preserves_zero(&self) -> bool {
        match self {
            Self::Filter(f) => match f {
                filter::Settings::GaussianBlur { .. }
                | filter::Settings::Sharpen { .. }
                | filter::Settings::Normalize => true,
                filter::Settings::Levels { output_black, .. } => *output_black == 0.0,
                _ => false,
            },
            // 見える度合い 1（隠す量 0）は、最大・加算・スクリーンでは 1 のまま
            Self::Generator(g) => matches!(
                g.blend,
                generator::Blend::Max | generator::Blend::Add | generator::Blend::Screen
            ),
        }
    }
    /// 設定が範囲内で、その値の種類に使えるか。使えなければ理由（C# の `RefusalFor` と範囲の検査）。
    pub fn validate(&self, value_type: ValueType) -> Result<(), CoreError> {
        match self {
            Self::Filter(f) => {
                if matches!(f, filter::Settings::Generator { .. }) {
                    return Err(CoreError::InvalidArgument(
                        "ジェネレーターはジェネレーターの設定で置く",
                    ));
                }
                f.validate(value_type).map_err(|e| match e {
                    filter::Error::Invalid(why) => CoreError::InvalidArgument(why),
                    _ => CoreError::InvalidArgument("フィルターの設定"),
                })
            }
            Self::Generator(g) => {
                if value_type == ValueType::TangentNormal {
                    return Err(CoreError::Unsupported(
                        "ジェネレーターは 1 画素に 1 つの値を作るので、接空間の法線には置けない",
                    ));
                }
                g.validate().map_err(|e| match e {
                    generator::Error::Invalid(why) => CoreError::InvalidArgument(why),
                    _ => CoreError::InvalidArgument("ジェネレーターの設定"),
                })
            }
        }
    }
    /// 短い表示の名前。
    pub fn name(&self) -> &'static str {
        match self {
            Self::Filter(f) => match f {
                filter::Settings::GaussianBlur { .. } => "ぼかし",
                filter::Settings::Sharpen { .. } => "シャープ",
                filter::Settings::Noise {
                    monochrome: true, ..
                } => "ノイズ（単色）",
                filter::Settings::Noise { .. } => "ノイズ（色）",
                filter::Settings::Levels { .. } => "レベル補正",
                filter::Settings::Invert => "反転",
                filter::Settings::Normalize => "正規化",
                filter::Settings::GradientMap(_) => "グラデーションマップ",
                filter::Settings::ToneCurve(_) => "トーンカーブ",
                filter::Settings::ColorBalance(_) => "カラーバランス",
                filter::Settings::BrightnessContrast(_) => "明るさ・コントラスト",
                filter::Settings::Threshold(_) => "2 値化",
                filter::Settings::Posterize(_) => "ポスタリゼーション",
                filter::Settings::Generator { .. } => "ジェネレーター",
            },
            Self::Generator(g) => generator_kind_name(g.kind),
        }
    }
}

/// Generator の種類の短い表示の名前。
pub fn generator_kind_name(kind: generator::Kind) -> &'static str {
    match kind {
        generator::Kind::EdgeWear => "エッジの摩耗",
        generator::Kind::Dirt => "汚れ",
        generator::Kind::PositionGradient => "位置のグラデーション",
        generator::Kind::Thickness => "厚み",
        generator::Kind::Direction => "向き",
        generator::Kind::ShapeGradient => "形のグラデーション",
        generator::Kind::IdColor => "ID の色",
        generator::Kind::Anchor => "Anchor",
        generator::Kind::Noise => "ノイズ",
        generator::Kind::Grunge => "グランジ",
        generator::Kind::Image => "画像",
    }
}

/// 値の種類（評価器の型）。標準・ユーザーのどのチャンネルも、種類で決める。
pub(crate) fn value_type_of(kind: ChannelKind) -> ValueType {
    match kind {
        ChannelKind::Color => ValueType::Color,
        ChannelKind::Scalar => ValueType::Scalar,
        ChannelKind::Normal => ValueType::TangentNormal,
    }
}

/// スタックの 1 つの段（C# の `FilterEffect`）: ID・設定・有効・強さ（0〜1。段の入力と結果をこの量で混ぜる）と、内容のスタックでは
/// 適用するチャンネル（番号の順。マスクのスタックでは空）。
#[derive(Clone, Debug, PartialEq)]
pub struct FilterEffect {
    pub(crate) id: FilterId,
    pub(crate) settings: EffectSettings,
    pub(crate) enabled: bool,
    pub(crate) strength: f64,
    pub(crate) channels: Vec<Channel>,
}

impl FilterEffect {
    pub fn id(&self) -> FilterId {
        self.id
    }
    pub fn settings(&self) -> &EffectSettings {
        &self.settings
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn strength(&self) -> f64 {
        self.strength
    }
    /// 内容のスタックの段が適用するチャンネル（番号の順）。マスクのスタックでは空。
    pub fn channels(&self) -> &[Channel] {
        &self.channels
    }
    pub fn applies_to(&self, channel: Channel) -> bool {
        self.channels.contains(&channel)
    }
    /// 結果を変える段か（有効で、強さが 0 より大きい）。
    pub fn is_active(&self) -> bool {
        self.enabled && self.strength > 0.0
    }
}

/// 段を足すときの指定。`FilterSpec::new(設定)` に、必要なものだけ足す。
#[derive(Clone, Debug)]
pub struct FilterSpec {
    pub settings: EffectSettings,
    /// 内容のスタック: 適用するチャンネル。None は設定を受け付ける標準のチャンネル全部。マスクのスタックでは None だけ。
    pub channels: Option<Vec<Channel>>,
    /// スタックの中の位置（0 が最初に当たる）。None は一番上（最後に当たる）。
    pub index: Option<usize>,
    /// 段の ID。None なら文書が決める。
    pub id: Option<FilterId>,
    pub enabled: bool,
    pub strength: f64,
}

impl FilterSpec {
    pub fn new(settings: EffectSettings) -> Self {
        FilterSpec {
            settings,
            channels: None,
            index: None,
            id: None,
            enabled: true,
            strength: 1.0,
        }
    }
    pub fn channels(mut self, channels: &[Channel]) -> Self {
        self.channels = Some(channels.to_vec());
        self
    }
    pub fn at(mut self, index: usize) -> Self {
        self.index = Some(index);
        self
    }
    pub fn with_id(mut self, id: FilterId) -> Self {
        self.id = Some(id);
        self
    }
    pub fn strength(mut self, strength: f64) -> Self {
        self.strength = strength;
        self
    }
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

/// 層・マスクに置く名前の付いた接続点（C# の `AnchorPoint`）。層の Anchor は、その層までのスタックの結果、マスクの Anchor は、
/// マスクが層を見せる量。上の層の Anchor Generator が読む。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub(crate) id: AnchorId,
    pub(crate) name: String,
}

impl Anchor {
    /// 名前の最大の長さ（UTF-16 の単位）。
    pub const MAX_NAME_UTF16: usize = 128;
    pub fn id(&self) -> AnchorId {
        self.id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    /// 名前の検査（空・空白だけ・128 を超える長さは断る）。
    pub fn check_name(name: &str) -> Result<(), CoreError> {
        if name.trim().is_empty() {
            return Err(CoreError::InvalidArgument("Anchor の名前が空"));
        }
        if name.encode_utf16().count() > Self::MAX_NAME_UTF16 {
            return Err(CoreError::InvalidArgument("Anchor の名前が長すぎる"));
        }
        Ok(())
    }
}

/// Anchor の置き場。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AnchorPlacement {
    /// 層: その層までのスタックの結果（全チャンネル）。
    Layer,
    /// 層のマスク: マスクが層を見せる量。
    Mask,
}

/// Anchor と、それがある層（マスクの Anchor はそのマスクの層）。
#[derive(Clone, Copy, Debug)]
pub struct AnchorInfo<'a> {
    pub anchor: &'a Anchor,
    pub layer: crate::layer::LayerId,
    pub placement: AnchorPlacement,
}

/// Anchor の参照が今は使えない理由の種類。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AnchorIssueKind {
    /// まだ選んでいない。
    NotChosen,
    /// 読む Anchor がもう無い（消した・層やマスクを消した）。
    Missing,
    /// Anchor が読む層より下に無い（層を動かした）、または自分の層にある。
    NotBelow,
}

/// 入力をそのまま通している Anchor の Generator と、その理由。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnchorIssue {
    pub layer: crate::layer::LayerId,
    pub filter: FilterId,
    pub target: FilterTarget,
    /// 読む Anchor（まだ選んでいなければ None）。
    pub anchor: Option<AnchorId>,
    pub kind: AnchorIssueKind,
}

/// 評価に使えるか。Generator の段・塗りつぶしのグラデーションが、マップなどが使えなくて入力のまま通している理由。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InactiveReason {
    /// 使うマップが無い・古い・未検証・大きさが違う・ピンと違う・Anchor が使えない など（理由は generator のもの）。
    Generator(generator::Inactive),
    /// 設定が評価の入力として組めない（範囲外・有限でない値）。
    Rejected(String),
}

impl fmt::Display for InactiveReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use generator::Inactive as I;
        match self {
            Self::Generator(I::MissingMap(k)) => {
                write!(f, "{k:?} のマップがありません")
            }
            Self::Generator(I::StaleMap(k)) => {
                write!(f, "{k:?} のマップが古い条件で焼かれています")
            }
            Self::Generator(I::UnverifiedMap(k)) => {
                write!(f, "{k:?} のマップの条件を照合できません")
            }
            Self::Generator(I::MapSize(k)) => {
                write!(f, "{k:?} のマップの大きさがテクスチャセットと違います")
            }
            Self::Generator(I::PinMismatch(k)) => {
                write!(f, "{k:?} のマップがピンした焼きと違います")
            }
            Self::Generator(I::MissingFrame) => f.write_str("モデルのルートの位置が分かりません"),
            Self::Generator(I::EmptyBounds) => f.write_str("Position の境界箱の大きさが 0 です"),
            Self::Generator(I::NoIdColors) => f.write_str("ID の色が選ばれていません"),
            Self::Generator(I::Anchor(generator::anchor::Issue::NotChosen)) => {
                f.write_str("Anchor が選ばれていません")
            }
            Self::Generator(I::Anchor(generator::anchor::Issue::Missing)) => {
                f.write_str("読む Anchor がありません")
            }
            Self::Generator(I::Anchor(generator::anchor::Issue::NotBelow)) => {
                f.write_str("Anchor が自分の層より下にありません")
            }
            Self::Generator(I::NoImage) => f.write_str("画像が選ばれていません"),
            Self::Generator(I::MissingImage) => {
                f.write_str("画像がプロジェクトに無いか、読めません")
            }
            Self::Rejected(why) => write!(f, "設定が使えません: {why}"),
        }
    }
}

/// 効いていない効果が何か（[`InactiveEffect`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InactiveTarget {
    /// フィルターのスタックの Generator の段。`mask` はマスクのスタックか。
    Generator { mask: bool, kind: generator::Kind },
    /// 塗りつぶしのチャンネルのグラデーション（値を見せている）。
    FillGradient(Channel),
    /// 出ていないデカール。
    Decal,
    /// 画像を投影していない（値を見せている）塗りつぶしのチャンネル。
    FillImage(Channel),
}

/// 入力のまま通している効果 1 件（[`crate::Document::inactive_effect_list`]）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InactiveEffect {
    pub layer: crate::layer::LayerId,
    /// 層の名前（利用者が付けた文字列）。
    pub layer_name: String,
    pub target: InactiveTarget,
    pub reason: InactiveReason,
}

impl fmt::Display for InactiveEffect {
    /// 日本語の 1 行（保存・書き出しの知らせ用）。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = &self.layer_name;
        let why = &self.reason;
        match self.target {
            InactiveTarget::Generator { mask, kind } => write!(
                f,
                "「{name}」{}: {} は効いていません。{why}",
                if mask { "（マスク）" } else { "" },
                generator_kind_name(kind),
            ),
            InactiveTarget::FillGradient(c) => {
                write!(
                    f,
                    "「{name}」（{c:?}）: グラデーションは値を見せています。{why}"
                )
            }
            InactiveTarget::Decal => write!(f, "「{name}」（デカール）: 出ていません。{why}"),
            InactiveTarget::FillImage(c) => {
                write!(f, "「{name}」（{c:?}）: 画像を投影していません。{why}")
            }
        }
    }
}

/// 位置のマップが使えなくて UV の空間で評価しているノイズ・グランジの段 1 件（[`crate::Document::fallback_effect_list`]）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FallbackEffect {
    pub layer: crate::layer::LayerId,
    /// 層の名前（利用者が付けた文字列）。
    pub layer_name: String,
    /// マスクのスタックの段か。
    pub mask: bool,
    pub kind: generator::Kind,
    /// 位置のマップが使えない理由。
    pub reason: InactiveReason,
}

impl fmt::Display for FallbackEffect {
    /// 日本語の 1 行。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "「{}」{}: {} は UV の空間で評価しています。{}",
            self.layer_name,
            if self.mask { "（マスク）" } else { "" },
            generator_kind_name(self.kind),
            self.reason,
        )
    }
}

/// 型の ChannelKind の代わりに、標準チャンネルかを確かめる。効果を置けるのは標準のチャンネルだけ。
pub(crate) fn require_standard(channel: Channel) -> Result<(), CoreError> {
    if channel.is_standard() {
        Ok(())
    } else {
        Err(CoreError::Unsupported(
            "効果（フィルター・画像・グラデーション）は標準のチャンネルだけに置ける",
        ))
    }
}

/// 層に付く編集できるパス（C# の `EditablePath`）。層の対象チャンネルの画素はパスから描いた結果で、パスと画素はいつも一緒に変わる
/// （[`crate::Document::set_path`]）。キャンバスの点のパス（2D）か、モデルの三角形の上の点のパス（3D）。
#[derive(Clone, Debug, PartialEq)]
pub enum LayerPath {
    Canvas(crate::paths::CanvasPath),
    Surface(crate::paths::SurfacePath),
}

impl LayerPath {
    pub fn id(&self) -> u128 {
        match self {
            Self::Canvas(p) => p.id,
            Self::Surface(p) => p.id,
        }
    }
    /// 基準のチャンネル（組を持たないパスが描くチャンネル）。
    pub fn channel(&self) -> Channel {
        match self {
            Self::Canvas(p) => p.channel,
            Self::Surface(p) => p.channel,
        }
    }
    /// 組（チャンネルごとの色）。None は基準のチャンネルとブラシの色。
    pub fn material(&self) -> Option<&[crate::paths::ChannelPaint]> {
        match self {
            Self::Canvas(p) => p.material.as_deref(),
            Self::Surface(p) => p.material.as_deref(),
        }
    }
    /// パスが描くチャンネル（組があれば組のチャンネル、無ければ基準のチャンネル）。
    pub fn channels(&self) -> Vec<Channel> {
        match self.material() {
            Some(m) => m.iter().map(|p| p.channel).collect(),
            None => vec![self.channel()],
        }
    }
    pub fn point_count(&self) -> usize {
        match self {
            Self::Canvas(p) => p.points.len(),
            Self::Surface(p) => p.points.len(),
        }
    }
    /// 履歴に積むパスの状態の大きさ（C# の SetPath: 64 + 点の数 × 32 + 組のチャンネルの数 × 8）。
    pub(crate) fn state_cost(&self) -> u64 {
        64 + self.point_count() as u64 * 32 + self.material().map_or(0, <[_]>::len) as u64 * 8
    }
    pub fn is_canvas(&self) -> bool {
        matches!(self, Self::Canvas(_))
    }
    pub fn validate(&self) -> Result<(), CoreError> {
        match self {
            Self::Canvas(p) => p.validate(),
            Self::Surface(p) => p.validate(),
        }
        .map_err(paths_error)
    }
}

/// パスの評価・検査の失敗を、文書の失敗にする。
pub(crate) fn paths_error(e: crate::paths::Error) -> CoreError {
    use crate::paths::Error as E;
    match e {
        E::Invalid(why) => CoreError::InvalidArgument(why),
        E::ModelMismatch => CoreError::Unsupported("モデルの指紋がパスを作ったときと違う"),
        E::MissingTriangle => CoreError::Unsupported("パスが参照する三角形が無い"),
        E::TooManySamples => CoreError::Unsupported("ブラシの間隔に対してパスが長すぎる"),
        E::Canceled => CoreError::Cancelled,
        E::Core(e) => e,
        E::Dab(_) => CoreError::Unsupported("面のダブを拒否した"),
    }
}
