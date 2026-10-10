//! core の文書 → PSD の書き出し（チャンネルごと・焼き込み）。
//!
//! 1 つの PSD は 1 つのチャンネルの写し。レイヤーの合成モード・不透明度・表示はそのチャンネルの値で書く。PSD に形の無いものは、機能ごとに
//! 決めた形で書き、した事はレイヤーの名前つきの注記（[`ExportNote`]）にして返す（黙って捨てない）:
//!
//! | 機能 | 書き出し |
//! |---|---|
//! | レイヤーの中身のフィルター（Generator も）・塗りつぶしの画像/投影/デカール/グラデーション・半透明の塗りつぶし | 評価した画素のラスターレイヤー |
//! | マスクのフィルター・反転したマスク | 評価・反転した値をマスクの画素に |
//! | クリッピングされたグループ | グループの合成を 1 枚のラスターレイヤーに（クリッピングの印は付けたまま） |
//! | 印だけ付いた何にもクリッピングされないグループ（兄弟の一番下） | 印を外した普通のグループ（見た目は同じ） |
//! | 効いていないフィルターの段・Anchor | 落とす（画素は変わらない） |
//! | パス | 画素のまま（パスの情報を落とす） |
//! | 刻みの間の調整（レベル補正・色相/彩度・トーンカーブ・カラーバランス・明るさ/コントラスト・グラデーションマップ） | 最寄りの刻みへ丸めた調整レイヤー（値と合成の最大の差を注記に） |
//! | グラデーションマップの値のカーブ | カーブを通した色・不透明度の停止点（各 32 個まで）に展開した調整レイヤー（停止点の数と合成の最大の差を注記に。収まらなければ断る） |
//! | そのチャンネルで効かない・有効でないレイヤー | 隠したレイヤー |
//!
//! 焼いたレイヤー・マスクは元のレイヤーの名前・ID・合成モード・不透明度・表示・クリッピング・ロック・マスクの有効と濃度を持ち、レイヤーの並びと入れ子は変えない。
//! 画素は `Document::layer_output`・`mask_output`・`group_output`（合成と同じ評価の道）から取る。計画（[`plan_export`]）は画素を作らず、
//! 構築（[`ExportPlan::build`]）が画素を作る。同じ文書・同じ設定なら、計画と構築は同じ判断をする。
//!
//! 2 値化・ポスタリゼーションは整数の設定だけで、刻みの間が無い（丸めも注記も無い）。スカラーのチャンネルのトーンカーブは RGB 全体の曲線だけが効く
//! ので、そのチャンネルの PSD には R・G・B の曲線を直線で書く（`effective_adjustment`。書いた PSD の合成がそのチャンネルの合成と一致する）。
//!
//! 刻みへ丸める調整があるとき、画素は丸めた設定の文書から焼く（Anchor のように合成を読む画素も、PSD に書く丸めた調整の上の見た目になる）。
//!
//! 保証の射程: 焼いた PSD を読み戻した合成は、書き出したチャンネルの今の合成と全バイト一致する（丸めた調整・不透明度と濃度の 1/255 の
//! 刻みの外の値・Normal のレイヤーが重なる所を除く）。PSD のレイヤーは不透明度・マスクの濃度を 1/255 の刻みで持つので、刻みの外の値はこれまでどおり
//! 丸めて書く（注記には出ない）。Normal は PSD の合成が色の式で重ねるので、重なる所の結果は Yolu の法線の重ね方と違い得る（注記
//! `NormalBlend`）。Normal の焼き込みで書く統合画像は、書いたレイヤーを PSD と同じ色の式で重ねたもの（レイヤーと統合画像を食い違わせない）で、Yolu の
//! 合成を使うのは平らの方式だけ。ファイルの向きが DirectX の Normal は、レイヤーの緑を反転して書き、レベル補正は PSD の 1 つのレベル補正で書けないので断る。
//! 反転したマスクを画素に焼くと、不透明度に掛ける値が実数では同じでも浮動小数の最後の桁で違う画素値があり、反転したマスクを焼いたレイヤーが重なる所で
//! まれに 1 画素が 1 ずれ得る（見た目に出る差ではない）。
use super::bridge::{
    channel_label, core_adjustment, divider_part, layer_part, psd_adjustment, psd_locks, unique_id,
    Blocker, Refusal, PIXEL_BUDGET,
};
use super::write::{MaskRegion, Region, StreamOptions, Supplied, XResult};
use super::*;
use crate::{check, check_budget, Error, Result};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::io::{Seek, Write};
use std::sync::atomic::AtomicBool;
use yolu_core::curve::Curve;
use yolu_core::generator::Ramp;
use yolu_core::{
    AdjustmentSettings, AdjustmentType, BalanceRange, Channel, ChannelKind, CoreError,
    Document as CoreDocument, EffectSettings, GradientMap, Layer as CoreLayer, LayerId,
    LayerKind as CoreKind, NormalYDirection, RasterMask, Rect, Rgba8, RowOrder, ToneChannel,
    ToneCurves,
};

/// 書き出しの方式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExportMode {
    /// レイヤーを残し、PSD に形の無いものを焼いて書く。
    #[default]
    Bake,
    /// 合成だけを 1 枚のラスターレイヤーに書く。
    Flat,
}

/// 何を書き出すか: チャンネルと方式。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportOptions {
    pub channel: Channel,
    pub mode: ExportMode,
}
impl ExportOptions {
    pub fn new(channel: Channel, mode: ExportMode) -> Self {
        Self { channel, mode }
    }
    /// Color を、レイヤーを残して焼いて書く。
    pub fn color() -> Self {
        Self::new(Channel::Color, ExportMode::Bake)
    }
}

/// 書き出しを呼ぶ側の指定: 長い仕事の取消（立つと評価を止めて `Cancelled` で戻る。途中の結果は公開しない）と、書き出しに許す予算。
#[derive(Clone, Copy, Debug, Default)]
pub struct ExportControl<'a> {
    pub cancel: Option<&'a AtomicBool>,
    /// 書き出しに許すレイヤーの画素のバイト数（設定の「レイヤーのメモリ」。取り込みの `CopyOptions::source_budget` と同じ値）。レイヤーの記録の数（予算 1 MiB につき 1 件）・
    /// キャンバス・レイヤー 1 枚の画素の上限をここから決める（[`Limits::for_export`]）。`None` は C# の書き手と対の固定の上限（画素の合計 128 MiB と既定の
    /// [`Limits`]。厳密な書き出し `from_core` はいつもこれ）。
    pub source_budget: Option<u64>,
    /// 書くファイルの大きさの上限（バイト）。既定と上限は PSD の 2 GiB で、それより小さい値だけ指定できる。
    pub max_file_bytes: Option<u64>,
}
impl ExportControl<'_> {
    pub(super) fn limits(&self) -> Limits {
        match self.source_budget {
            Some(budget) => Limits::for_export(budget),
            None => Limits::default(),
        }
    }
    /// 1 枚の作業の領域と、メモリに全レイヤーを組むときの画素の合計に許すバイト数。
    pub(super) fn cap(&self) -> u64 {
        self.source_budget.unwrap_or(PIXEL_BUDGET)
    }
}

/// 塗りつぶしの画素が何から来るか（PSD の単色の塗りつぶしに形が無いもの）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FillSources {
    /// 画像を読む。
    pub image: bool,
    /// グラデーション（ワールドスペースなど。ランプ付きの形のグラデーション）。
    pub gradient: bool,
    /// デカール。
    pub decal: bool,
    /// 画像・デカールの投影。
    pub projection: Option<yolu_core::fill_image::ProjectionMode>,
}
impl FillSources {
    fn of(l: &CoreLayer, c: Channel) -> Self {
        let image = l.fill_image(c).is_some();
        let decal = l.is_decal() && l.fill_value(c).is_some();
        Self {
            image,
            gradient: l.fill_gradient(c).is_some(),
            decal,
            projection: (image || decal).then(|| l.projection().mode),
        }
    }
    fn any(&self) -> bool {
        self.image || self.gradient || self.decal
    }
}

/// 丸めた調整の値 1 つ。値は PSD の単位（レベル補正の黒・白は 0〜255、ガンマはそのまま、色相は度、彩度・明度は %、カラーバランスは −100〜100、
/// トーンカーブの点は 0〜255、グラデーションの位置・中点・不透明度は %）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoundedValue {
    pub parameter: RoundedParameter,
    pub from: f64,
    pub to: f64,
}
/// 丸める値の名前。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundedParameter {
    InputBlack,
    InputWhite,
    OutputBlack,
    OutputWhite,
    Gamma,
    Hue,
    Saturation,
    Lightness,
    /// カラーバランスの範囲ごとのスライダー（`axis` は 0 = シアン/レッド・1 = マゼンタ/グリーン・2 = イエロー/ブルー）。
    Balance {
        range: BalanceRange,
        axis: u8,
    },
    Brightness,
    Contrast,
    /// トーンカーブの点（曲線と点の番号。0 から）の入力・出力。隣の点が近すぎる点は、PSD が許す間隔まで離す（入力が動く）。
    CurveInput {
        curve: ToneChannel,
        point: u8,
    },
    CurveOutput {
        curve: ToneChannel,
        point: u8,
    },
    /// グラデーションマップの色の分岐点（番号は 0 から）の位置・中点。
    ColorStopPosition(u8),
    ColorStopMidpoint(u8),
    /// グラデーションマップの不透明度の分岐点（番号は 0 から）の位置・中点・値。
    OpacityStopPosition(u8),
    OpacityStopMidpoint(u8),
    OpacityStopValue(u8),
}

/// した事（レイヤーごとの注記の中身）。
#[derive(Clone, Debug, PartialEq)]
pub enum NoteAction {
    /// レイヤーの中身のフィルター・Generator の段（名前の並び）を、レイヤーの画素へ焼いた。
    BakedFilters(Vec<EffectSettings>),
    /// 塗りつぶしの画像・投影・デカール・グラデーションを、レイヤーの画素へ焼いた。
    BakedFill(FillSources),
    /// 半透明の塗りつぶしを、レイヤーの画素へ焼いた（PSD の単色の塗りつぶしは不透明だけ）。
    BakedTranslucentFill,
    /// パスレイヤー。画素はパスから描いた結果のままで、パスの情報は PSD に残らない。
    BakedPath,
    /// マスクのフィルター・Generator の段を、マスクの画素へ焼いた。
    BakedMaskFilters(Vec<EffectSettings>),
    /// 反転したマスクを、反転した値のマスクの画素へ焼いた。
    BakedInvertedMask,
    /// クリッピングされたグループを、1 枚のラスターレイヤーにした。
    BakedClippedGroup,
    /// 兄弟の一番下のグループのクリッピングの印（何にもクリッピングされない）を外し、普通のグループとして書いた。見た目は変わらない。
    DroppedClippingMark,
    /// 効いていない（無効・強さ 0）フィルターの段を落とした。画素は変わらない。
    DroppedFilters(Vec<EffectSettings>),
    /// 効いていないマスクのフィルターの段を落とした。
    DroppedMaskFilters(Vec<EffectSettings>),
    /// Anchor を落とした（Anchor を読む Generator は焼いたので、見た目は同じ）。
    DroppedAnchor,
    /// マスクの Anchor を落とした。
    DroppedMaskAnchor,
    /// Normal のレイヤーが重なる所は、PSD では色の式で重なる（Yolu は法線のベクトルとして重ねる）。統合画像は、書いたレイヤーを色の式で重ねたもの。
    /// 注記のレイヤーの名前は、チャンネルの名前。
    NormalBlend,
    /// 調整を PSD の刻みへ丸めた。`max_diff` は、書き出したチャンネルの合成の最大の差（0〜255。隠した調整は 0）。
    Rounded {
        kind: AdjustmentType,
        changes: Vec<RoundedValue>,
        max_diff: u8,
    },
    /// グラデーションマップの値のカーブか混色（混色モード・区間の混合率曲線。PSD のグラデーションに形が無い）を、それを通した色・不透明度の停止点の列へ展開した（近似）。
    /// `cause` は展開した理由（どちらを使っていたか）、`colors`・`opacities` は停止点の数、`max_diff` は書き出したチャンネルの合成の最大の差（0〜255。隠した調整は 0）。
    /// `max_diff` は文書の合成の実測で、展開の基準（レイヤー 1 枚の出力の差 `EXPANSION_DIFFS`。通常の合成モード・不透明度 100% のとき）を超えることがある（超えても断らない）。
    ExpandedGradientCurve {
        cause: GradientExpansion,
        colors: usize,
        opacities: usize,
        max_diff: u8,
    },
    /// テキストレイヤー。画素はテキストの値から描いた結果のままで、テキストの値は PSD に残らない（PSD のテキストレイヤーは書かない）。
    BakedText,
}

/// グラデーションマップを停止点へ展開した理由（PSD のグラデーションに形が無いもののうち、使っていたもの）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientExpansion {
    /// 値のカーブだけ。
    Curve,
    /// 混色（混色モード・区間の混合率曲線）だけ。
    Mixing,
    /// 値のカーブと混色の両方。
    CurveAndMixing,
}

/// レイヤー 1 枚の注記。
#[derive(Clone, Debug, PartialEq)]
pub struct ExportNote {
    pub layer: String,
    pub action: NoteAction,
}
impl ExportNote {
    /// 日本語の 1 行（レイヤーの名前つき。画面は種類から画面の言語の文を作る）。
    pub fn message(&self) -> String {
        let name = &self.layer;
        let names = |v: &[EffectSettings]| {
            v.iter()
                .map(EffectSettings::name)
                .collect::<Vec<_>>()
                .join("・")
        };
        match &self.action {
            NoteAction::BakedFilters(v) => {
                format!("レイヤー「{name}」のフィルター（{}）を画素に焼きました", names(v))
            }
            NoteAction::BakedFill(_) => {
                format!("塗りつぶし「{name}」の画像・投影・グラデーションを画素に焼きました")
            }
            NoteAction::BakedTranslucentFill => {
                format!("塗りつぶし「{name}」の半透明の値を画素に焼きました")
            }
            NoteAction::BakedPath => {
                format!("レイヤー「{name}」のパスの情報は PSD に残りません。画素はそのままです")
            }
            NoteAction::BakedMaskFilters(v) => format!(
                "レイヤー「{name}」のマスクのフィルター（{}）をマスクの画素に焼きました",
                names(v)
            ),
            NoteAction::BakedInvertedMask => {
                format!("レイヤー「{name}」の反転したマスクを、反転した値のマスクの画素に焼きました")
            }
            NoteAction::BakedClippedGroup => {
                format!("グループ「{name}」はクリッピングされているので、1 枚の画素のレイヤーにしました")
            }
            NoteAction::DroppedClippingMark => format!(
                "グループ「{name}」のクリッピングの印を外しました（何にもクリッピングされないので、見た目は同じです）"
            ),
            NoteAction::DroppedFilters(v) => format!(
                "レイヤー「{name}」の効いていないフィルター（{}）を落としました",
                names(v)
            ),
            NoteAction::DroppedMaskFilters(v) => format!(
                "レイヤー「{name}」のマスクの効いていないフィルター（{}）を落としました",
                names(v)
            ),
            NoteAction::DroppedAnchor => format!("レイヤー「{name}」の Anchor を落としました"),
            NoteAction::DroppedMaskAnchor => {
                format!("レイヤー「{name}」のマスクの Anchor を落としました")
            }
            NoteAction::NormalBlend => {
                format!("チャンネル「{name}」のレイヤーが重なる所は、PSD では色の式で重なります（Yolu は法線のベクトルとして重ねます）")
            }
            NoteAction::Rounded { max_diff, .. } => {
                format!("調整「{name}」を PSD の刻みへ丸めました（合成の最大の差 {max_diff}）")
            }
            NoteAction::ExpandedGradientCurve {
                cause,
                colors,
                opacities,
                max_diff,
            } => {
                let what = match cause {
                    GradientExpansion::Curve => "値のカーブ",
                    GradientExpansion::Mixing => "混色（混色モード・混合率曲線）",
                    GradientExpansion::CurveAndMixing => "値のカーブと混色（混色モード・混合率曲線）",
                };
                format!(
                    "調整「{name}」のグラデーションマップの{what}を、色 {colors} 個・不透明度 {opacities} 個の停止点に展開しました（合成の最大の差 {max_diff}）"
                )
            }
            NoteAction::BakedText => {
                format!("テキストレイヤー「{name}」は画素のレイヤーとして書きました。テキストの値は PSD に残りません")
            }
        }
    }
}

/// レイヤーがチャンネルで必要とすること（そのままは書けない理由）。
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Need {
    ContentFilters(Vec<EffectSettings>),
    FillPixels(FillSources),
    TranslucentFill,
    Path,
    MaskFilters(Vec<EffectSettings>),
    InvertedMask,
    ClippedGroup,
    /// グループの印だけがあり、何にもクリッピングされない（兄弟の一番下）。
    IdleClippingMark,
    DroppedFilters(Vec<EffectSettings>),
    DroppedMaskFilters(Vec<EffectSettings>),
    Anchor,
    MaskAnchor,
    Round(Box<Rounding>),
    /// 焼いても丸めても書けない（断る）。
    Hard(Refusal),
    /// テキストレイヤー（Color のチャンネルの画素はテキストの値から描いた結果）。
    Text,
}
impl Need {
    /// そのまま書く（焼かない・丸めない・落とさない）ときに断る理由。
    pub(super) fn refusal(&self) -> Refusal {
        match self {
            Need::ContentFilters(_)
            | Need::DroppedFilters(_)
            | Need::MaskFilters(_)
            | Need::DroppedMaskFilters(_) => Refusal::Effects,
            Need::FillPixels(_) => Refusal::FillPixels,
            Need::TranslucentFill => Refusal::FillTranslucent,
            Need::Path => Refusal::Path,
            Need::InvertedMask => Refusal::InvertedMask,
            // Unity 版は印だけを見て断る（印が付いたフォルダーという確かめていない形を書かない）
            Need::ClippedGroup | Need::IdleClippingMark => Refusal::ClippedGroup,
            Need::Anchor | Need::MaskAnchor => Refusal::Anchor,
            Need::Round(r) => r.why.clone(),
            // 展開できない値のカーブ: 厳密な書き出しは展開をしないので、収まる曲線と同じ理由（C# の断りと対）で断る
            Need::Hard(Refusal::GradientMapCurveStops) => Refusal::GradientMapCurve,
            Need::Hard(Refusal::GradientMapMixingStops) => Refusal::GradientMapMixing,
            Need::Hard(r) => r.clone(),
            Need::Text => Refusal::Text,
        }
    }
    /// 焼き込みの書き出しが断る理由。`Hard` は計画の `blockers` にある理由そのまま（展開できなかった理由を言う）。ほかは `refusal`。
    fn blocking_refusal(&self) -> Refusal {
        match self {
            Need::Hard(r) => r.clone(),
            other => other.refusal(),
        }
    }
    fn bakes_content(&self) -> bool {
        matches!(
            self,
            Need::ContentFilters(_) | Need::FillPixels(_) | Need::TranslucentFill
        )
    }
    fn bakes_mask(&self) -> bool {
        matches!(self, Need::MaskFilters(_) | Need::InvertedMask)
    }
}

/// 刻みの間の調整を最寄りの刻みへ丸めた（グラデーションの値のカーブは停止点へ展開した）結果。
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Rounding {
    pub kind: AdjustmentType,
    pub changes: Vec<RoundedValue>,
    /// 値のカーブか混色を展開したときの、展開した理由と停止点の数（色・不透明度）。丸めではないので `changes` は空。
    pub stops: Option<(GradientExpansion, usize, usize)>,
    /// 丸めた設定（PSD の刻みの値から取り込みと同じ割り算で作ったもの。読み戻した文書と同じ設定）。
    pub settings: AdjustmentSettings,
    pub psd: Adjustment,
    /// そのまま書けなかった理由（`psd_adjustment` の断り）。
    pub why: Refusal,
}

fn step(v: f64, low: f64, high: f64) -> f64 {
    v.round_ties_even().clamp(low, high)
}
fn changed(parameter: RoundedParameter, from: f64, to: f64) -> Option<RoundedValue> {
    ((from - to).abs() > 1e-9).then_some(RoundedValue {
        parameter,
        from,
        to,
    })
}
/// 並び `v`（昇順）を、隣との間隔が `gap` 以上・全体が `lo..=hi` に収まるように最小限だけ動かす（`hi - lo` が足りる前提）。
/// 前から押し上げて、はみ出したら後ろから押し戻す。端の値が `lo`・`hi` ちょうどなら動かない。
fn spread(v: &mut [i64], gap: i64, lo: i64, hi: i64) {
    let n = v.len();
    if n == 0 {
        return;
    }
    v[0] = v[0].max(lo);
    for k in 1..n {
        v[k] = v[k].max(v[k - 1] + gap);
    }
    v[n - 1] = v[n - 1].min(hi);
    for k in (0..n - 1).rev() {
        v[k] = v[k].min(v[k + 1] - gap);
    }
}

/// PSD のトーンカーブの隣り合う点の入力の間隔の下限（0〜255 の整数。core の曲線の下限 `Curve::MIN_GAP` 以上になる最小の整数）。
fn curve_gap() -> i64 {
    (Curve::MIN_GAP * 255.0).ceil() as i64
}

/// グラデーションの停止点の数の上限（PSD の読みと core のランプの上限）。
const STOP_LIMIT: usize = 32;
/// 値のカーブの展開が許す、調整レイヤー 1 枚の出力の差（0〜255 の値）の段階。測るのは [`mix_diff`] の値で、そのレイヤーが通常の合成モード・不透明度 100% で、
/// 下の色 0〜255 のどれに当たっても出る最悪の差。まず 1 以下を目指し、停止点が足りなければ緩める（不透明度がカーブに沿って動くと、不透明度の
/// 1 の誤差が下の色によっては出力で 1 を超えるので、1 では足りないことがある）。最後の段階でも足りなければ、合成に出るレイヤーは断る（合成に出ない
/// レイヤーは、最後の段階の最良の展開を隠したレイヤーで書く）。
/// 文書の合成の差の上限ではない: レイヤーの合成モード（傾きの大きいもの）・不透明度・上のレイヤー（2 値化・レベル補正など）が、この差を増やすことがある。
/// 文書の合成の差は、計画が丸めた文書の合成と今の合成を比べて測り、注記に出す（断る基準にはしない）。
const EXPANSION_DIFFS: [u32; 3] = [1, 2, 4];

/// 入力 `k / 255`（輝度 `k`）に対する停止点の位置（PSD の 0〜4096。四捨五入）。両端は 0 と 4096 ちょうど。
fn knot_location(k: usize) -> u16 {
    ((k * 4096 * 2 + 255) / 510) as u16
}
/// 調整の表（輝度 → 色・不透明度）が `want` と `have` で違うとき、そのレイヤーが通常の合成モード・不透明度 100% で、下の色 `0..=255` のどれに当てても出る
/// 出力の最大の差。
fn mix_diff(want: Rgba8, have: Rgba8) -> (u32, u32, u32) {
    if want == have {
        return (0, 0, 0);
    }
    let mix = |orig: u32, mapped: u8, a: u8| {
        (orig * (255 - u32::from(a)) + u32::from(mapped) * u32::from(a) + 127) / 255
    };
    let mut worst = 0;
    for orig in 0..=255u32 {
        for (w, h) in [(want.r, have.r), (want.g, have.g), (want.b, have.b)] {
            worst = worst.max(mix(orig, w, want.a).abs_diff(mix(orig, h, have.a)));
        }
    }
    let colour = [
        want.r.abs_diff(have.r),
        want.g.abs_diff(have.g),
        want.b.abs_diff(have.b),
    ]
    .into_iter()
    .max()
    .unwrap_or(0);
    (worst, u32::from(colour), u32::from(want.a.abs_diff(have.a)))
}
/// 値のカーブの展開（停止点の列）。
struct Expansion {
    adjustment: Adjustment,
    colors: usize,
    opacities: usize,
    /// [`EXPANSION_DIFFS`] のどれかの段に収まったか（収まらないとき、これは最後の段の最良の展開）。
    fits: bool,
}

/// グラデーションマップの値のカーブを、カーブを通した色・不透明度の停止点の列（中点は 50%）へ展開する。差の段階（[`EXPANSION_DIFFS`]）を
/// 1 から順に緩めて、初めて停止点が [`STOP_LIMIT`] に収まった展開を返す。どの段階でも収まらなければ、最後の段階の最良の展開（`fits` が false）。
/// 展開の組み立てそのものができないとき（起きない想定）だけ None。
fn expand_curve(g: &GradientMap) -> Option<Expansion> {
    let mut last = None;
    for tolerance in EXPANSION_DIFFS {
        let expansion = expand_within(g, tolerance)?;
        if expansion.fits {
            return Some(expansion);
        }
        last = Some(expansion);
    }
    last
}

/// 調整が当たる輝度は `k / 255`（k = 0〜255）の 256 点だけなので、停止点は輝度の点に置き、その位置の元の色・不透明度（カーブを通した値）を
/// 持たせる。誤差の一番大きい輝度に、色か不透明度の停止点を 1 つずつ足し、レイヤー 1 枚の出力の差（[`mix_diff`]。通常モード・不透明度 100%・下の色によらない
/// 最悪）が `tolerance` 以下になった所で止める。停止点が [`STOP_LIMIT`] を超えて足りなければ、足せなくなった所の展開を `fits: false` で返す。
/// 位置は 1/4096 の刻みに寄るので、急なカーブでは輝度の点の隣にも足すことがある。
fn expand_within(g: &GradientMap, tolerance: u32) -> Option<Expansion> {
    let ramp = g.ramp();
    let sample = |r: &Ramp, k: usize| {
        r.evaluate(k as f64 / 255.0, false)
            .expect("輝度は有限なのでランプの評価は失敗しない")
    };
    let want: Vec<Rgba8> = (0..256).map(|k| sample(ramp, k)).collect();
    let mut colors = vec![0usize, 255];
    let mut opacities = vec![0usize, 255];
    let build = |colors: &[usize], opacities: &[usize]| {
        let at = |k: usize| {
            ramp.evaluate(f64::from(knot_location(k)) / 4096.0, false)
                .expect("位置は有限なのでランプの評価は失敗しない")
        };
        Adjustment::GradientMap {
            reverse: g.reverse(),
            colors: colors
                .iter()
                .map(|&k| {
                    let c = at(k);
                    GradientColorStop {
                        location: knot_location(k),
                        midpoint: 50,
                        rgb: [c.r, c.g, c.b],
                    }
                })
                .collect(),
            opacities: opacities
                .iter()
                .map(|&k| GradientOpacityStop {
                    location: knot_location(k),
                    midpoint: 50,
                    opacity: at(k).a,
                })
                .collect(),
        }
    };
    loop {
        let adjustment = build(&colors, &opacities);
        let settings = core_adjustment(&adjustment).ok()?;
        let got = settings.gradient_map_value()?.ramp();
        // 輝度ごとの (合成の最大の差, 色の差, 不透明度の差)
        let diffs: Vec<(u32, u32, u32)> = (0..256)
            .map(|k| mix_diff(want[k], sample(got, k)))
            .collect();
        if diffs.iter().all(|d| d.0 <= tolerance) {
            return Some(Expansion {
                colors: colors.len(),
                opacities: opacities.len(),
                adjustment,
                fits: true,
            });
        }
        let mut order: Vec<usize> = (0..256).collect();
        order.sort_by_key(|&k| std::cmp::Reverse(diffs[k].0));
        let mut added = false;
        'search: for k in order {
            let (worst, colour, alpha) = diffs[k];
            if worst <= tolerance {
                break;
            }
            // 差の大きい方の列から足す（その列に既にあるか、満杯か、差が 0 なら、もう一方へ）
            let colour_first = colour >= alpha;
            for to_colour in [colour_first, !colour_first] {
                let (list, diff) = if to_colour {
                    (&mut colors, colour)
                } else {
                    (&mut opacities, alpha)
                };
                if diff == 0 || list.len() >= STOP_LIMIT || list.contains(&k) {
                    continue;
                }
                let at = list.partition_point(|&x| x < k);
                list.insert(at, k);
                added = true;
                break 'search;
            }
        }
        if !added {
            return Some(Expansion {
                colors: colors.len(),
                opacities: opacities.len(),
                adjustment,
                fits: false,
            });
        }
    }
}

/// 刻みの間（PSD の範囲の外も）の調整を、最寄りの刻みへ。グラデーションマップの値のカーブは停止点へ展開する。種類を足すときは、ここに腕を足す
/// （その種類の `psd_adjustment` が断る値を、PSD の刻みの整数へ寄せる。寄せられない種類は断りのまま）。反転・2 値化・ポスタリゼーションは常に
/// 刻みの上にある（断りが無い）。丸めても書けないときは、断る理由を返す。
/// `shown` は、そのレイヤーが書き出すチャンネルの合成に出るか。値のカーブが停止点の上限・差の段階に収まらないとき、合成に出るレイヤーは断る（`GradientMapCurveStops`）が、
/// 合成に出ないレイヤー（表示を切った・そのチャンネルに効かない・無効）は、展開の失敗で断る理由が無いので、最後の段階の最良の展開を隠したレイヤーで書く。
fn rounding(
    s: &AdjustmentSettings,
    why: Refusal,
    shown: bool,
) -> std::result::Result<Rounding, Refusal> {
    let mut stops = None;
    let (psd, changes) = match s.kind() {
        AdjustmentType::Invert | AdjustmentType::Threshold | AdjustmentType::Posterize => {
            return Err(why)
        }
        AdjustmentType::Levels => {
            let ib = step(s.input_black() * 255.0, 0.0, 253.0);
            let iw = step(s.input_white() * 255.0, 2.0, 255.0).max(ib + 1.0);
            let ob = step(s.output_black() * 255.0, 0.0, 255.0);
            let ow = step(s.output_white() * 255.0, 0.0, 255.0);
            let g = step(s.gamma() * 100.0, 10.0, 999.0);
            let changes = [
                changed(RoundedParameter::InputBlack, s.input_black() * 255.0, ib),
                changed(RoundedParameter::InputWhite, s.input_white() * 255.0, iw),
                changed(RoundedParameter::OutputBlack, s.output_black() * 255.0, ob),
                changed(RoundedParameter::OutputWhite, s.output_white() * 255.0, ow),
                changed(RoundedParameter::Gamma, s.gamma(), g / 100.0),
            ]
            .into_iter()
            .flatten()
            .collect();
            (
                Adjustment::Levels {
                    input_black: ib as u16,
                    input_white: iw as u16,
                    output_black: ob as u16,
                    output_white: ow as u16,
                    gamma: g as u16,
                },
                changes,
            )
        }
        AdjustmentType::HueSaturation => {
            let hue = step(s.hue(), -180.0, 180.0);
            let saturation = step(s.saturation() * 100.0, -100.0, 100.0);
            let lightness = step(s.lightness() * 100.0, -100.0, 100.0);
            let changes = [
                changed(RoundedParameter::Hue, s.hue(), hue),
                changed(
                    RoundedParameter::Saturation,
                    s.saturation() * 100.0,
                    saturation,
                ),
                changed(
                    RoundedParameter::Lightness,
                    s.lightness() * 100.0,
                    lightness,
                ),
            ]
            .into_iter()
            .flatten()
            .collect();
            (
                Adjustment::HueSaturation {
                    hue: hue as i16,
                    saturation: saturation as i16,
                    lightness: lightness as i16,
                },
                changes,
            )
        }
        AdjustmentType::ToneCurve => {
            let curves = s.tone_curve_value().ok_or(why.clone())?;
            let gap = curve_gap();
            let mut changes = Vec::new();
            let mut written: Vec<Vec<[u8; 2]>> = Vec::new();
            for channel in ToneChannel::ALL {
                let points = curves.curve(channel).points();
                let mut xs: Vec<i64> = points
                    .iter()
                    .map(|p| step(p.x * 255.0, 0.0, 255.0) as i64)
                    .collect();
                spread(&mut xs, gap, 0, 255);
                let mut list = Vec::new();
                for (i, (p, x)) in points.iter().zip(&xs).enumerate() {
                    let y = step(p.y * 255.0, 0.0, 255.0);
                    let point = i as u8;
                    changes.extend(changed(
                        RoundedParameter::CurveInput {
                            curve: channel,
                            point,
                        },
                        p.x * 255.0,
                        *x as f64,
                    ));
                    changes.extend(changed(
                        RoundedParameter::CurveOutput {
                            curve: channel,
                            point,
                        },
                        p.y * 255.0,
                        y,
                    ));
                    list.push([*x as u8, y as u8]);
                }
                written.push(list);
            }
            let [composite, red, green, blue]: [Vec<[u8; 2]>; 4] =
                written.try_into().expect("曲線は 4 本");
            (
                Adjustment::ToneCurve {
                    composite,
                    red,
                    green,
                    blue,
                },
                changes,
            )
        }
        AdjustmentType::ColorBalance => {
            let v = s.color_balance_value().ok_or(why.clone())?;
            let mut ranges = [[0i16; 3]; 3];
            let mut changes = Vec::new();
            for (i, range) in BalanceRange::ALL.into_iter().enumerate() {
                for (axis, value) in v.values(range).into_iter().enumerate() {
                    let to = step(value, -100.0, 100.0);
                    ranges[i][axis] = to as i16;
                    changes.extend(changed(
                        RoundedParameter::Balance {
                            range,
                            axis: axis as u8,
                        },
                        value,
                        to,
                    ));
                }
            }
            (
                Adjustment::ColorBalance {
                    shadows: ranges[0],
                    midtones: ranges[1],
                    highlights: ranges[2],
                    preserve_luminosity: v.preserve_luminosity(),
                },
                changes,
            )
        }
        AdjustmentType::BrightnessContrast => {
            let v = s.brightness_contrast_value().ok_or(why.clone())?;
            let brightness = step(v.brightness(), -150.0, 150.0);
            let contrast = step(v.contrast(), -50.0, 100.0);
            let changes = [
                changed(RoundedParameter::Brightness, v.brightness(), brightness),
                changed(RoundedParameter::Contrast, v.contrast(), contrast),
            ]
            .into_iter()
            .flatten()
            .collect();
            (
                Adjustment::BrightnessContrast {
                    brightness: brightness as i16,
                    contrast: contrast as i16,
                },
                changes,
            )
        }
        AdjustmentType::GradientMap => {
            let g = s.gradient_map_value().ok_or(why.clone())?;
            let curved = !g.ramp().value_curve().is_identity();
            if curved || g.ramp().uses_mixing() {
                // 値のカーブ・混色（混色モード・混合率曲線）: 停止点の列へ展開する（刻みの間の位置・中点・不透明度も、展開した停止点が刻みの上に
                // 作り直す）。PSD のグラデーションは sRGB の値の線形な補間（中点つき）だけなので、混ぜ方の違いは停止点を増やして近づける
                let expansion = expand_curve(g)
                    .filter(|e| e.fits || !shown)
                    .ok_or(if curved {
                        Refusal::GradientMapCurveStops
                    } else {
                        Refusal::GradientMapMixingStops
                    })?;
                let cause = match (curved, g.ramp().uses_mixing()) {
                    (true, true) => GradientExpansion::CurveAndMixing,
                    (true, false) => GradientExpansion::Curve,
                    _ => GradientExpansion::Mixing,
                };
                stops = Some((cause, expansion.colors, expansion.opacities));
                (expansion.adjustment, Vec::new())
            } else {
                let ramp = g.ramp();
                let mut changes = Vec::new();
                // 位置は 1/4096 の刻み。近い 2 点が同じ刻みに寄らないよう、1 刻みだけ離す
                let place = |positions: Vec<f64>| -> Vec<i64> {
                    let mut v: Vec<i64> = positions
                        .iter()
                        .map(|p| step(p * 4096.0, 0.0, 4096.0) as i64)
                        .collect();
                    spread(&mut v, 1, 0, 4096);
                    v
                };
                let color_at = place(ramp.colors().iter().map(|c| c.position).collect());
                let opacity_at = place(ramp.opacities().iter().map(|o| o.position).collect());
                let last_color = ramp.colors().len() - 1;
                let last_opacity = ramp.opacities().len() - 1;
                let mut colors = Vec::new();
                for (i, (c, at)) in ramp.colors().iter().zip(&color_at).enumerate() {
                    let index = i as u8;
                    changes.extend(changed(
                        RoundedParameter::ColorStopPosition(index),
                        c.position * 100.0,
                        *at as f64 / 4096.0 * 100.0,
                    ));
                    // 最後の分岐点の中点は使われないので 50 で書く（変えた値としては数えない）
                    let midpoint = if i == last_color {
                        50.0
                    } else {
                        let m = step(c.midpoint * 100.0, 1.0, 99.0);
                        changes.extend(changed(
                            RoundedParameter::ColorStopMidpoint(index),
                            c.midpoint * 100.0,
                            m,
                        ));
                        m
                    };
                    colors.push(GradientColorStop {
                        location: *at as u16,
                        midpoint: midpoint as u8,
                        rgb: [c.color.r, c.color.g, c.color.b],
                    });
                }
                let mut opacities = Vec::new();
                for (i, (o, at)) in ramp.opacities().iter().zip(&opacity_at).enumerate() {
                    let index = i as u8;
                    changes.extend(changed(
                        RoundedParameter::OpacityStopPosition(index),
                        o.position * 100.0,
                        *at as f64 / 4096.0 * 100.0,
                    ));
                    let midpoint = if i == last_opacity {
                        50.0
                    } else {
                        let m = step(o.midpoint * 100.0, 1.0, 99.0);
                        changes.extend(changed(
                            RoundedParameter::OpacityStopMidpoint(index),
                            o.midpoint * 100.0,
                            m,
                        ));
                        m
                    };
                    let value = step(o.opacity * 255.0, 0.0, 255.0);
                    changes.extend(changed(
                        RoundedParameter::OpacityStopValue(index),
                        o.opacity * 100.0,
                        value / 255.0 * 100.0,
                    ));
                    opacities.push(GradientOpacityStop {
                        location: *at as u16,
                        midpoint: midpoint as u8,
                        opacity: value as u8,
                    });
                }
                (
                    Adjustment::GradientMap {
                        reverse: g.reverse(),
                        colors,
                        opacities,
                    },
                    changes,
                )
            }
        }
    };
    let settings = core_adjustment(&psd).map_err(|_| why.clone())?;
    Ok(Rounding {
        kind: s.kind(),
        changes,
        stops,
        settings,
        psd,
        why,
    })
}

/// 調整が、書き出すチャンネルで実際に効く設定。スカラーのチャンネル（Roughness・Height・ユーザーチャンネルなど）のトーンカーブは RGB 全体の曲線だけを
/// 使い（`apply_in`）、R・G・B の曲線は効かない。PSD のトーンカーブは R・G・B の曲線も当てるので、スカラーのチャンネルの PSD には直線で書く
/// （書いた PSD の合成がそのチャンネルの合成と一致する）。ほかの調整・色のチャンネルは設定のまま。
fn effective_adjustment<'a>(
    s: &'a AdjustmentSettings,
    d: &CoreDocument,
    c: Channel,
) -> std::borrow::Cow<'a, AdjustmentSettings> {
    use std::borrow::Cow;
    let scalar = d
        .channel_info(c)
        .is_some_and(|i| i.kind == ChannelKind::Scalar);
    match s.tone_curve_value() {
        Some(curves)
            if scalar
                && ![ToneChannel::Red, ToneChannel::Green, ToneChannel::Blue]
                    .into_iter()
                    .all(|t| curves.curve(t).is_identity()) =>
        {
            Cow::Owned(AdjustmentSettings::tone_curve(
                ToneCurves::identity().with_curve(
                    ToneChannel::Composite,
                    curves.curve(ToneChannel::Composite).clone(),
                ),
            ))
        }
        _ => Cow::Borrowed(s),
    }
}

/// レイヤーが書き出すチャンネルで必要とすること。そのチャンネルに効かないレイヤー（無効・面の無いレイヤー）も、持っている情報の扱いは同じ判断にする
/// （隠したレイヤーとして書く）。
pub(super) fn needs(d: &CoreDocument, l: &CoreLayer, c: Channel) -> Vec<Need> {
    let kind = l.kind();
    let mut out = Vec::new();
    // レイヤーの中身のフィルター: 効く段は焼き、効いていない段（無効・強さ 0）は落とす。ほかのチャンネルだけの段は、この PSD に関わらない
    let mut baked_content = false;
    if matches!(kind, CoreKind::Raster | CoreKind::Fill) {
        let (active, idle): (Vec<_>, Vec<_>) = l
            .filters()
            .iter()
            .filter(|e| e.applies_to(c))
            .partition(|e| e.is_active());
        if !active.is_empty() {
            baked_content = true;
            out.push(Need::ContentFilters(
                active.iter().map(|e| e.settings().clone()).collect(),
            ));
        }
        if !idle.is_empty() {
            out.push(Need::DroppedFilters(
                idle.iter().map(|e| e.settings().clone()).collect(),
            ));
        }
    }
    if kind == CoreKind::Fill {
        let sources = FillSources::of(l, c);
        if sources.any() {
            out.push(Need::FillPixels(sources))
        } else if !baked_content && l.fill_value(c).is_some_and(|v| v.a != 255) {
            out.push(Need::TranslucentFill)
        }
    }
    if l.path().is_some_and(|p| p.channels().contains(&c)) {
        out.push(Need::Path)
    }
    if c == Channel::Color && l.text().is_some() {
        out.push(Need::Text)
    }
    if l.anchor().is_some() {
        out.push(Need::Anchor)
    }
    if let Some(m) = l.mask() {
        let (active, idle): (Vec<_>, Vec<_>) = m.filters().iter().partition(|e| e.is_active());
        if !active.is_empty() {
            out.push(Need::MaskFilters(
                active.iter().map(|e| e.settings().clone()).collect(),
            ));
        }
        if !idle.is_empty() {
            out.push(Need::DroppedMaskFilters(
                idle.iter().map(|e| e.settings().clone()).collect(),
            ));
        }
        if m.inverted() {
            out.push(Need::InvertedMask)
        }
        if m.anchor().is_some() {
            out.push(Need::MaskAnchor)
        }
    }
    // クリッピングの印が効いているグループは 1 枚に焼く。兄弟の一番下の印は何にもクリッピングされず、普通のグループとして重なる（通過の
    // グループの調整・モードが下へ効くので、1 枚に焼くと見た目が変わる）ので、焼かずに印を外して普通のグループで書く
    if kind == CoreKind::Group {
        if d.layer_index(l.id())
            .is_some_and(|i| d.is_effectively_clipped(i))
        {
            out.push(Need::ClippedGroup)
        } else if l.clipping() {
            out.push(Need::IdleClippingMark)
        }
    }
    if let Some(original) = l.adjustment() {
        let s = &effective_adjustment(original, d, c);
        match psd_adjustment(s) {
            Ok(_) => {}
            Err(why) => match rounding(s, why, shown_in(d, l, c)) {
                Ok(r) => out.push(Need::Round(Box::new(r))),
                Err(refusal) => out.push(Need::Hard(refusal)),
            },
        }
        // Normal の PSD をファイルの Y の向き（DirectX）で書くと、緑を反転した値がレイヤーに入る。反転と、チャンネルごとに同じ式の調整は順序を
        // 入れ替えても同じだが、レベル補正は緑だけ別の曲線になるので、PSD の 1 つのレベル補正では書けない
        if s.kind() == AdjustmentType::Levels
            && l.visible()
            && l.is_channel_enabled(c)
            && flips_green(d, c)
        {
            out.push(Need::Hard(Refusal::NormalLevels))
        }
    }
    out
}

/// 法線のチャンネルか（レイヤーを単位ベクトルとして重ねる）。
fn is_normal(d: &CoreDocument, c: Channel) -> bool {
    d.channel_info(c)
        .is_some_and(|i| i.kind == ChannelKind::Normal)
}

/// このチャンネルのレイヤーの値に、ファイルの Y の向きを掛けるか（Normal で、文書のファイルの向きが DirectX）。
fn flips_green(d: &CoreDocument, c: Channel) -> bool {
    is_normal(d, c) && d.normal_settings().file_direction() == NormalYDirection::DirectX
}

/// 書き出しの計画（画素は作らない）。注記は上のレイヤーから。`blockers` があれば、構築は断る。
#[derive(Clone, Debug)]
pub struct ExportPlan {
    pub options: ExportOptions,
    pub notes: Vec<ExportNote>,
    /// 焼いても丸めても書けないレイヤーとその理由（今は、ファイルの向きが DirectX の Normal のレベル補正と、PSD の記録の無い種類の調整）。
    pub blockers: Vec<Blocker>,
    /// 注記に出した、刻みへ丸める調整レイヤーと丸めた設定（構築が、画素を丸めた設定の写しから焼くために使う）。
    rounded: Vec<(LayerId, AdjustmentSettings)>,
}
/// 書き出した結果（PSD のレイヤーと、した事の注記）。
#[derive(Clone, Debug)]
pub struct Exported {
    pub document: Document,
    pub notes: Vec<ExportNote>,
}

/// 親ごとの子（下から上）。
struct Tree<'a> {
    children: HashMap<Option<LayerId>, Vec<&'a CoreLayer>>,
}
impl<'a> Tree<'a> {
    fn new(d: &'a CoreDocument) -> Self {
        let mut children: HashMap<Option<LayerId>, Vec<&CoreLayer>> = HashMap::new();
        for l in d.layers() {
            children.entry(l.parent()).or_default().push(l)
        }
        Self { children }
    }
    /// 1 つの段（None は一番上）を上から下の並びで。
    fn level(&self, parent: Option<LayerId>) -> Vec<&'a CoreLayer> {
        self.children
            .get(&parent)
            .map(|v| v.iter().rev().copied().collect())
            .unwrap_or_default()
    }
}

/// 書き出しの前に、文書そのものが書き出せるか（ストロークの確定・チャンネルがある・キャンバス・レイヤーの数の予算）を安く確かめる。計画・構築も同じ検査を
/// 先頭で行う。予算で断るときは `ExportError::Overrun`（画面が理由と、設定で上げられることを言う）。
pub fn check_exportable(
    d: &CoreDocument,
    channel: Channel,
    ctl: &ExportControl,
) -> std::result::Result<(), ExportError> {
    precheck(d, channel, &ctl.limits())
}

fn precheck(d: &CoreDocument, channel: Channel, limits: &Limits) -> XResult<()> {
    check(
        !d.has_active_stroke(),
        "ストロークを確定・取消してからPSDを書き出してください",
    )?;
    check(
        d.channel_info(channel).is_some(),
        "書き出すチャンネルがプロジェクトにありません",
    )?;
    if d.width() > limits.max_dimension || d.height() > limits.max_dimension {
        return Err(Overrun::Side {
            width: d.width(),
            height: d.height(),
            limit: limits.max_dimension,
        }
        .into());
    }
    if u64::from(d.width()) * u64::from(d.height()) > limits.max_canvas_pixels {
        return Err(Overrun::Canvas {
            width: d.width(),
            height: d.height(),
        }
        .into());
    }
    check_budget(!d.layers().is_empty(), "PSD レイヤー数の予算超過")?;
    if d.layers().len() > limits.max_layers {
        return Err(Overrun::Layers {
            count: d.layers().len(),
            limit: limits.max_layers,
        }
        .into());
    }
    Ok(())
}

fn max_diff(a: &[u8], b: &[u8]) -> u8 {
    a.iter()
        .zip(b)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

fn composite(d: &CoreDocument, c: Channel, cancel: Option<&AtomicBool>) -> Result<Vec<u8>> {
    let mut out = vec![0u8; d.width() as usize * d.height() as usize * 4];
    d.composite_into_cancellable(c, d.bounds(), &mut out, RowOrder::BottomUp, cancel)?;
    Ok(out)
}

/// 書き出しの計画: 何を焼き・丸め・落とすかをレイヤーの名前つきの注記にする（丸める調整は、丸めた文書と今の文書の合成の最大の差も測る）。
/// 断るものは `blockers` に。文書は変えない。
pub fn plan_export(
    d: &CoreDocument,
    options: &ExportOptions,
    ctl: &ExportControl,
) -> Result<ExportPlan> {
    precheck(d, options.channel, &ctl.limits())?;
    if ctl
        .cancel
        .is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed))
    {
        return Err(Error::Core(CoreError::Cancelled));
    }
    let mut plan = ExportPlan {
        options: options.clone(),
        notes: Vec::new(),
        blockers: Vec::new(),
        rounded: Vec::new(),
    };
    if options.mode == ExportMode::Flat {
        return Ok(plan);
    }
    let c = options.channel;
    let tree = Tree::new(d);
    // (注記の番号, レイヤー, 丸め, 合成に出るか)
    let mut rounded: Vec<(usize, LayerId, Rounding, bool)> = Vec::new();
    fn walk<'a>(
        tree: &Tree<'a>,
        d: &CoreDocument,
        c: Channel,
        parent: Option<LayerId>,
        plan: &mut ExportPlan,
        rounded: &mut Vec<(usize, LayerId, Rounding, bool)>,
    ) {
        for l in tree.level(parent) {
            let needs = needs(d, l, c);
            let clipped_group = needs.iter().any(|n| matches!(n, Need::ClippedGroup));
            for need in needs {
                let layer = l.name().to_owned();
                let action = match need {
                    Need::Hard(refusal) => {
                        plan.blockers.push(Blocker { layer, refusal });
                        continue;
                    }
                    Need::ContentFilters(v) => NoteAction::BakedFilters(v),
                    Need::FillPixels(s) => NoteAction::BakedFill(s),
                    Need::TranslucentFill => NoteAction::BakedTranslucentFill,
                    Need::Path => NoteAction::BakedPath,
                    Need::MaskFilters(v) => NoteAction::BakedMaskFilters(v),
                    Need::InvertedMask => NoteAction::BakedInvertedMask,
                    Need::ClippedGroup => NoteAction::BakedClippedGroup,
                    Need::IdleClippingMark => NoteAction::DroppedClippingMark,
                    Need::DroppedFilters(v) => NoteAction::DroppedFilters(v),
                    Need::DroppedMaskFilters(v) => NoteAction::DroppedMaskFilters(v),
                    Need::Anchor => NoteAction::DroppedAnchor,
                    Need::MaskAnchor => NoteAction::DroppedMaskAnchor,
                    Need::Text => NoteAction::BakedText,
                    Need::Round(r) => {
                        rounded.push((plan.notes.len(), l.id(), (*r).clone(), shown_in(d, l, c)));
                        match r.stops {
                            Some((cause, colors, opacities)) => NoteAction::ExpandedGradientCurve {
                                cause,
                                colors,
                                opacities,
                                max_diff: 0,
                            },
                            None => NoteAction::Rounded {
                                kind: r.kind,
                                changes: r.changes,
                                max_diff: 0,
                            },
                        }
                    }
                };
                plan.notes.push(ExportNote { layer, action });
            }
            if l.is_group() && !clipped_group {
                walk(tree, d, c, Some(l.id()), plan, rounded);
            }
        }
    }
    walk(&tree, d, c, None, &mut plan, &mut rounded);
    plan.rounded = rounded
        .iter()
        .map(|(_, id, r, _)| (*id, r.settings.clone()))
        .collect();
    // Normal のレイヤーが 2 枚以上重なると、PSD の色の式の重ねは Yolu の法線の重ねと違い得る
    if is_normal(d, c)
        && d.layers()
            .iter()
            .filter(|l| matches!(l.kind(), CoreKind::Raster | CoreKind::Fill) && shown_in(d, l, c))
            .count()
            >= 2
    {
        plan.notes.push(ExportNote {
            layer: channel_label(d, c),
            action: NoteAction::NormalBlend,
        })
    }
    // 丸めの差: 丸めた設定だけを替えた写しの合成と、今の合成を比べる（隠した調整は合成に出ないので 0）
    let mut base: Option<Vec<u8>> = None;
    for (note, id, rounding, shown) in rounded {
        if !shown {
            continue;
        }
        if base.is_none() {
            base = Some(composite(d, c, ctl.cancel)?);
        }
        let twin = d.with_adjustments_replaced(&[(id, rounding.settings)])?;
        let after = composite(&twin, c, ctl.cancel)?;
        let diff = max_diff(base.as_deref().expect("上で作った"), &after);
        if let NoteAction::Rounded { max_diff, .. }
        | NoteAction::ExpandedGradientCurve { max_diff, .. } = &mut plan.notes[note].action
        {
            *max_diff = diff
        }
    }
    Ok(plan)
}

/// レイヤーが書き出すチャンネルの合成に出るか（調整・塗りつぶし・ラスターは有効の印と中身、表示。グループは表示）。
fn shown_in(d: &CoreDocument, l: &CoreLayer, c: Channel) -> bool {
    if !l.visible() {
        return false;
    }
    match l.kind() {
        CoreKind::Group => true,
        CoreKind::Raster => l.is_channel_enabled(c) && l.surface(c).is_some(),
        CoreKind::Fill => l.is_channel_enabled(c) && l.fill_value(c).is_some(),
        CoreKind::Adjustment => {
            let kind = d.channel_info(c).map_or(ChannelKind::Color, |i| i.kind);
            l.is_channel_enabled(c) && l.adjustment().is_some_and(|a| a.applies_to(kind))
        }
    }
}

impl ExportPlan {
    /// 刻みへ丸める調整があれば、丸めた設定の写しから作る。Anchor のように合成を読んで焼く画素も、PSD に書く丸めた調整の上の見た目で焼くので、
    /// 書いた PSD の合成は「丸めた設定の文書」の合成と同じになる（元の設定で焼くと、丸めた調整の上で画素がずれる）。
    fn source(&self, d: &CoreDocument) -> Result<Option<CoreDocument>> {
        if let Some(b) = self.blockers.first() {
            return Err(Error::InvalidData(b.message()));
        }
        if self.rounded.is_empty() {
            Ok(None)
        } else {
            Ok(Some(d.with_adjustments_replaced(&self.rounded)?))
        }
    }

    /// 計画どおりに PSD のレイヤーを作る（画素を評価する。全レイヤーの画素をメモリに組むので、画素の合計は予算に入る範囲だけ）。断るものがあれば断る。
    /// 取消・予算超過はエラー。実物の大きさの文書は、レイヤーを 1 枚ずつ流して書く [`write_psd`](Self::write_psd) で書く。
    pub fn build(&self, d: &CoreDocument, ctl: &ExportControl) -> Result<Exported> {
        let twin = self.source(d)?;
        let source = twin.as_ref().unwrap_or(d);
        let document = Builder::new(source, &self.options, ctl, false)?.run(Compression::Raw)?;
        Ok(Exported {
            document,
            notes: self.notes.clone(),
        })
    }

    /// 計画どおりに、PSD を `out`（書き始めが先頭の、Seek できる出力）へ流して書く。レイヤーの画素は PSD の記録の順に 1 枚ずつ評価して、圧縮して書いたらすぐ
    /// 捨てるので、メモリにはレイヤー 1 枚ぶんと記録の表しか持たない（画素の合計に上限は無く、ファイルの 2 GiB が上限）。Normal の焼き込みと平らの 1 枚だけは、
    /// 全レイヤーをメモリに組む（合計は予算で止める）。書きかけを消すのは呼び手の仕事（途中で失敗・取消すると、`out` には書きかけが残る）。
    pub fn write_psd<W: Write + Seek>(
        &self,
        d: &CoreDocument,
        ctl: &ExportControl,
        out: &mut W,
        compression: Compression,
    ) -> std::result::Result<Written, ExportError> {
        let twin = self.source(d)?;
        let source = twin.as_ref().unwrap_or(d);
        Builder::new(source, &self.options, ctl, false)?.write(out, compression)
    }
}

/// 計画して構築する（確認のウィンドウを挟まない呼び出し向け）。
pub fn export_core(
    d: &CoreDocument,
    options: &ExportOptions,
    ctl: &ExportControl,
) -> Result<Exported> {
    plan_export(d, options, ctl)?.build(d, ctl)
}

/// 焼いた画素の領域（PSD の座標: 上から下）。
struct Baked {
    left: i32,
    top: i32,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}
impl Baked {
    fn into_region(self) -> Region<'static> {
        Region {
            left: self.left,
            top: self.top,
            width: self.width,
            height: self.height,
            rgba: Cow::Owned(self.pixels),
        }
    }
}

/// レイヤーの画素の出どころ。レイヤーの構造（記録）を先に作り、画素は PSD の記録の順に 1 枚ずつ作る（流して書くときは、書いたらすぐ捨てる）。
#[derive(Clone, Copy)]
enum Source<'a> {
    /// 保存した画素（面のあるタイルの外接矩形）。
    Stored(&'a CoreLayer),
    /// 評価して焼くレイヤー（フィルター・塗りつぶしの画像など）。
    Baked(&'a CoreLayer),
    /// 評価して 1 枚にしたクリッピングされたグループ。
    BakedGroup(&'a CoreLayer),
}
/// レイヤーの構造を作ったあとの、画素の出どころの表（PSD のレイヤー ID から）。
#[derive(Default)]
struct Slots<'a> {
    content: HashMap<i32, Source<'a>>,
    /// マスクのあるレイヤー: (レイヤー, マスク, 評価して焼くか)。
    masks: HashMap<i32, (&'a CoreLayer, &'a RasterMask, bool)>,
}

/// 書き出したチャンネルの合成（上の行から。ファイルの向きが DirectX の Normal は緑を反転）。統合画像になる。
fn merged_composite(
    d: &CoreDocument,
    c: Channel,
    flip: bool,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<u8>> {
    let mut merged = vec![0u8; d.width() as usize * d.height() as usize * 4];
    d.composite_into_cancellable(c, d.bounds(), &mut merged, RowOrder::TopDown, cancel)?;
    if flip {
        yolu_core::normal::flip_green(&mut merged);
    }
    Ok(merged)
}

struct Builder<'a> {
    d: &'a CoreDocument,
    c: Channel,
    mode: ExportMode,
    flip: bool,
    tree: Tree<'a>,
    used: HashSet<i32>,
    limits: Limits,
    /// 1 枚の作業の領域と、メモリに全レイヤーを組むときの画素の合計に許すバイト数。
    cap: u64,
    /// 画素の合計を数えて `cap` で止めるか（メモリに全レイヤーを組む書き出し。流して書くときはレイヤー 1 枚ぶんしか持たないので数えない）。
    accumulate: bool,
    budget: u64,
    max_file_bytes: u64,
    cancel: Option<&'a AtomicBool>,
    /// そのままは書けないものが 1 つでもあれば断る（`from_core`）。
    strict: bool,
}

impl<'a> Builder<'a> {
    pub(super) fn new(
        d: &'a CoreDocument,
        options: &ExportOptions,
        ctl: &ExportControl<'a>,
        strict: bool,
    ) -> XResult<Self> {
        let limits = ctl.limits();
        precheck(d, options.channel, &limits)?;
        let max_file_bytes = ctl
            .max_file_bytes
            .unwrap_or(limits.max_output_bytes as u64)
            .min(limits.max_output_bytes as u64);
        Ok(Self {
            d,
            c: options.channel,
            mode: options.mode,
            flip: flips_green(d, options.channel),
            tree: Tree::new(d),
            used: HashSet::new(),
            limits,
            cap: ctl.cap(),
            accumulate: true,
            budget: 0,
            max_file_bytes,
            cancel: ctl.cancel,
            strict,
        })
    }

    /// メモリに全レイヤーを組む。レイヤーの画素は PSD の記録の順に 1 枚ずつ作ってレイヤーへ入れる。`compression` は、このあとの書き方（RLE は圧縮したあとの長さを書きながら
    /// 上限と比べるので、無圧縮で書いたときの長さは確かめない。無圧縮はその長さも上限に入るか確かめる）。
    pub(super) fn run(mut self, compression: Compression) -> XResult<Document> {
        let d = self.d;
        let layers = match self.mode {
            ExportMode::Bake => {
                let mut slots = Slots::default();
                let mut layers = self.level(None, &mut slots)?;
                self.fill_pixels(&mut layers, &slots)?;
                layers
            }
            ExportMode::Flat => Vec::new(),
        };
        // Normal のレイヤーを重ねた統合画像は、書いたレイヤーを PSD と同じ色の式で重ねたもの（読み戻したとき、レイヤーと統合画像が食い違って編集できない PSD に
        // ならない）。Yolu の法線の重ねとは、重なる所で違い得る（注記 `NormalBlend`）
        if self.mode == ExportMode::Bake && is_normal(d, self.c) {
            let mut out = Document {
                width: d.width(),
                height: d.height(),
                layers,
                composite_rgba: None,
            };
            self.check_cancel()?;
            out.composite_rgba = Some(super::composite::composite_cancellable(&out, self.cancel)?);
            super::write::validate_for(&out, &self.limits, compression)?;
            return Ok(out);
        }
        let merged = merged_composite(d, self.c, self.flip, self.cancel)?;
        let layers = match self.mode {
            ExportMode::Bake => layers,
            ExportMode::Flat => {
                self.add("", merged.len() as u64)?;
                vec![Layer {
                    id: 1,
                    name: channel_label(d, self.c),
                    width: d.width(),
                    height: d.height(),
                    pixels_rgba: merged.clone(),
                    ..Layer::default()
                }]
            }
        };
        let out = Document {
            width: d.width(),
            height: d.height(),
            layers,
            composite_rgba: Some(merged),
        };
        super::write::validate_for(&out, &self.limits, compression)?;
        Ok(out)
    }

    /// 流して書く。レイヤーの画素は PSD の記録の順に 1 枚ずつ作り、圧縮して書いたらすぐ捨てる（メモリにはレイヤー 1 枚ぶんと記録の表）。統合画像はレイヤーを書いたあとで
    /// 作る。Normal の焼き込み（統合画像を書いたレイヤーから作る）と平らの 1 枚は、全レイヤーをメモリに組んでから書く（画素の合計は予算で止める）。
    pub(super) fn write<W: Write + Seek>(
        mut self,
        out: &mut W,
        compression: Compression,
    ) -> XResult<Written> {
        let d = self.d;
        let streaming = self.mode == ExportMode::Bake && !is_normal(d, self.c);
        if !streaming {
            let (limits, cancel, max_file_bytes) =
                (self.limits.clone(), self.cancel, self.max_file_bytes);
            let document = self.run(compression)?;
            return super::write::stream_document(
                out,
                &document,
                &limits,
                compression,
                max_file_bytes,
                cancel,
            );
        }
        self.accumulate = false;
        let mut slots = Slots::default();
        let layers = self.level(None, &mut slots)?;
        let skeleton = Document {
            width: d.width(),
            height: d.height(),
            layers,
            composite_rgba: None,
        };
        let count = skeleton_count(&skeleton.layers);
        if count > self.limits.max_layers {
            return Err(Overrun::Layers {
                count,
                limit: self.limits.max_layers,
            }
            .into());
        }
        let records = super::write::skeleton_records(&skeleton, &self.limits)?;
        let limits = self.limits.clone();
        let opts = StreamOptions {
            compression,
            limits: &limits,
            max_file_bytes: self.max_file_bytes,
            cancel: self.cancel,
        };
        let (c, flip, cancel) = (self.c, self.flip, self.cancel);
        let (bytes, checksum) = super::write::stream(
            out,
            d.width(),
            d.height(),
            &records,
            &opts,
            |i| {
                let r = &records[i];
                self.supply(&slots, r.layer.id, r.raster(), r.mask().is_some())
            },
            move || Ok(Cow::Owned(merged_composite(d, c, flip, cancel)?)),
        )?;
        Ok(Written {
            bytes,
            layers: records.iter().filter(|r| !r.divider).count(),
            checksum,
        })
    }

    fn check_cancel(&self) -> Result<()> {
        if self
            .cancel
            .is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed))
        {
            Err(Error::Core(CoreError::Cancelled))
        } else {
            Ok(())
        }
    }

    /// 1 枚の作業の領域（`bytes`）が予算に入るか確かめる。
    fn work(&self, layer: &str, bytes: u64) -> XResult<()> {
        if bytes > self.cap {
            return Err(Overrun::Memory {
                layer: layer.to_owned(),
            }
            .into());
        }
        Ok(())
    }
    /// メモリに組む画素に `bytes` を足す（全レイヤーをメモリに組むときだけ合計を数えて、予算で断る）。
    fn add(&mut self, layer: &str, bytes: u64) -> XResult<()> {
        self.work(layer, bytes)?;
        if self.accumulate {
            self.budget += bytes;
            if self.budget > self.cap {
                return Err(Overrun::Memory {
                    layer: layer.to_owned(),
                }
                .into());
            }
        }
        Ok(())
    }

    /// 1 つの段（None は一番上）のレイヤーの構造。上から下の並びで返す（画素は作らない）。
    fn level(&mut self, parent: Option<LayerId>, slots: &mut Slots<'a>) -> XResult<Vec<Layer>> {
        let mut out = Vec::new();
        for l in self.tree.level(parent) {
            out.push(self.layer(l, slots)?)
        }
        Ok(out)
    }

    /// レイヤーの PSD での合成モードと不透明度（書き出すチャンネルの値。PSD のレイヤーは 1 組しか持てない）。
    fn blend(&self, l: &CoreLayer, baked_group: bool) -> (BlendMode, u8) {
        let mode = BlendMode::ALL[l.blend_mode_in(self.c) as usize];
        // 焼いたグループはラスターレイヤーになり、通過は使えない
        let mode = if baked_group && mode == BlendMode::PassThrough {
            BlendMode::Normal
        } else {
            mode
        };
        (mode, (l.opacity_in(self.c) * 255.0).round_ties_even() as u8)
    }

    /// レイヤーが PSD で表示か。そのチャンネルで合成に出ないレイヤー（有効でない・面や値が無い・効かない調整）は隠して書く。
    fn visible(&self, l: &CoreLayer, baked_group: bool) -> bool {
        match l.kind() {
            // 画素にしたグループは、合成が落とす（中身が無い・不透明度 0）なら隠す。残すと、クリッピングの組に入って、下地のグループの
            // 通過（組が無いときだけ通過）を妨げる
            CoreKind::Group if baked_group => {
                l.visible() && self.d.contributes(l.id(), self.c).unwrap_or(true)
            }
            CoreKind::Group => l.visible(),
            _ => shown_in(self.d, l, self.c),
        }
    }

    /// レイヤーの記録（画素を除く）。画素・マスクの値の出どころは `slots` に入れる。
    fn layer(&mut self, l: &'a CoreLayer, slots: &mut Slots<'a>) -> XResult<Layer> {
        self.check_cancel()?;
        let needs = needs(self.d, l, self.c);
        let refused = if self.strict {
            needs.first()
        } else {
            needs.iter().find(|n| matches!(n, Need::Hard(_)))
        };
        if let Some(need) = refused {
            return Err(Error::InvalidData(
                Blocker {
                    layer: l.name().into(),
                    refusal: if self.strict {
                        need.refusal()
                    } else {
                        need.blocking_refusal()
                    },
                }
                .message(),
            )
            .into());
        }
        let bake_content = needs.iter().any(Need::bakes_content);
        let bake_mask = needs.iter().any(Need::bakes_mask);
        let baked_group = needs.iter().any(|n| matches!(n, Need::ClippedGroup));
        let idle_mark = needs.iter().any(|n| matches!(n, Need::IdleClippingMark));
        // マスクの矩形・既定値・値は画素のときに決まる。ここでは有効と濃度だけ
        let mask = l.mask().map(|m| Mask {
            left: 0,
            top: 0,
            width: 0,
            height: 0,
            default_color: 255,
            enabled: m.enabled(),
            density: (m.density() * 255.0).round_ties_even() as u8,
            pixels: Vec::new(),
        });
        let (blend_mode, opacity) = self.blend(l, baked_group);
        let id = unique_id(layer_part(l.id()), &mut self.used);
        let mut layer = Layer {
            id,
            name: l.name().into(),
            opacity,
            visible: self.visible(l, baked_group),
            blend_mode,
            clipping: l.clipping() && !idle_mark,
            mask,
            locks: psd_locks(l.locks()),
            ..Layer::default()
        };
        if let Some(m) = l.mask() {
            slots.masks.insert(id, (l, m, bake_mask));
        }
        match l.kind() {
            CoreKind::Raster => {
                slots.content.insert(
                    id,
                    if bake_content {
                        Source::Baked(l)
                    } else {
                        Source::Stored(l)
                    },
                );
            }
            CoreKind::Fill => self.fill(l, &mut layer, bake_content, slots),
            CoreKind::Adjustment => {
                let settings = l.adjustment().expect("調整レイヤーは設定を持つ");
                let adjustment = needs
                    .iter()
                    .find_map(|n| match n {
                        Need::Round(r) => Some(r.psd.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| {
                        psd_adjustment(&effective_adjustment(settings, self.d, self.c))
                            .expect("確かめた")
                    });
                layer.kind = LayerKind::Adjustment(adjustment)
            }
            CoreKind::Group if baked_group => {
                slots.content.insert(id, Source::BakedGroup(l));
            }
            CoreKind::Group => {
                let divider_id = unique_id(divider_part(l.id()), &mut self.used);
                layer.kind = LayerKind::Group {
                    children: self.level(Some(l.id()), slots)?,
                    divider_id,
                }
            }
        }
        Ok(layer)
    }

    /// 塗りつぶし: 値だけの不透明な塗りつぶしは単色の塗りつぶし。評価が要る・半透明のものは画素に焼く（画素は `slots` の出どころから）。値の無い
    /// チャンネルは、隠した単色の塗りつぶし（チャンネルの既定の色）。
    fn fill(&mut self, l: &'a CoreLayer, layer: &mut Layer, baked: bool, slots: &mut Slots<'a>) {
        if baked {
            slots.content.insert(layer.id, Source::Baked(l));
            return;
        }
        let value = l
            .fill_value(self.c)
            .or_else(|| self.d.channel_info(self.c).map(|i| i.default))
            .unwrap_or(yolu_core::Rgba8::new(0, 0, 0, 255));
        let mut rgba = [value.r, value.g, value.b, 255];
        self.flip_pixels(&mut rgba);
        layer.kind = LayerKind::SolidColor([rgba[0], rgba[1], rgba[2]]);
    }

    /// 構造のレイヤーへ、PSD の記録の順（下から上。グループは中身のあとにグループ自身）に画素とマスクの値を 1 枚ずつ作って入れる。
    fn fill_pixels(&mut self, layers: &mut [Layer], slots: &Slots<'a>) -> XResult<()> {
        for l in layers.iter_mut().rev() {
            if let LayerKind::Group { children, .. } = &mut l.kind {
                self.fill_pixels(children, slots)?
            }
            let raster = matches!(l.kind, LayerKind::Raster);
            let supplied = self.supply(slots, l.id, raster, l.mask.is_some())?;
            if let Some(r) = supplied.raster {
                l.left = r.left;
                l.top = r.top;
                l.width = r.width;
                l.height = r.height;
                l.pixels_rgba = r.rgba.into_owned();
            }
            if let (Some(m), Some(dto)) = (supplied.mask, l.mask.as_mut()) {
                dto.left = m.left;
                dto.top = m.top;
                dto.width = m.width;
                dto.height = m.height;
                dto.default_color = m.default_color;
                dto.pixels = m.values.into_owned();
            }
        }
        Ok(())
    }

    /// 記録 1 つの画素とマスクの値を作る（`id` は PSD のレイヤー ID）。
    fn supply(
        &mut self,
        slots: &Slots<'a>,
        id: i32,
        raster: bool,
        has_mask: bool,
    ) -> XResult<Supplied<'static>> {
        let mut supplied = Supplied::default();
        if raster {
            let source = *slots
                .content
                .get(&id)
                .expect("ラスターレイヤーには出どころがある");
            supplied.raster = Some(self.content(source)?.into_region());
        }
        if has_mask {
            let (l, m, baked) = *slots
                .masks
                .get(&id)
                .expect("マスクのあるレイヤーには出どころがある");
            supplied.mask = Some(self.mask(l, m, baked)?);
        }
        Ok(supplied)
    }

    fn content(&mut self, source: Source<'a>) -> XResult<Baked> {
        let d = self.d;
        let (c, cancel) = (self.c, self.cancel);
        let mut region = match source {
            Source::Stored(l) => return self.stored(l),
            Source::Baked(l) => {
                let id = l.id();
                self.bake(l.name(), |rect, out| {
                    d.layer_output_into(id, c, rect, out, RowOrder::TopDown, cancel)
                })?
            }
            Source::BakedGroup(l) => {
                let id = l.id();
                self.bake(l.name(), |rect, out| {
                    d.group_output_into(id, c, rect, out, RowOrder::TopDown, cancel)
                })?
            }
        };
        self.flip_pixels(&mut region.pixels);
        Ok(region)
    }

    /// 領域を帯ごとに評価して集め（上の帯から。各帯は上から下）、透明でない画素の外接矩形へ切り詰める（無ければ 1×1 の透明）。
    /// 作業のバッファはキャンバス 1 枚ぶんで、予算を先に見る。
    fn bake(
        &mut self,
        name: &str,
        produce: impl Fn(Rect, &mut [u8]) -> std::result::Result<(), CoreError>,
    ) -> XResult<Baked> {
        let d = self.d;
        let (w, h) = (d.width() as usize, d.height() as usize);
        self.work(name, (w * h * 4) as u64)?;
        let ts = d.tile_size();
        let band = (256 / ts).max(1) * ts;
        let mut pixels = vec![0u8; w * h * 4];
        let mut top = d.height();
        let mut at = 0usize;
        while top > 0 {
            self.check_cancel()?;
            let y0 = (top - 1) / band * band;
            let rect = Rect::new(0, y0, d.width(), top - y0);
            let len = w * (top - y0) as usize * 4;
            produce(rect, &mut pixels[at..at + len]).map_err(Error::from)?;
            at += len;
            top = y0;
        }
        // 透明でない画素の外接矩形
        let (mut x0, mut x1, mut y0, mut y1) = (w, 0usize, h, 0usize);
        for (row, line) in pixels.chunks_exact(w * 4).enumerate() {
            let line = line.as_chunks::<4>().0;
            let Some(first) = line.iter().position(|p| p[3] != 0) else {
                continue;
            };
            let last = line.iter().rposition(|p| p[3] != 0).expect("あった");
            x0 = x0.min(first);
            x1 = x1.max(last + 1);
            y0 = y0.min(row);
            y1 = y1.max(row + 1);
        }
        let region = if x1 <= x0 || y1 <= y0 {
            Baked {
                left: 0,
                top: 0,
                width: 1,
                height: 1,
                pixels: vec![0; 4],
            }
        } else {
            let (rw, rh) = (x1 - x0, y1 - y0);
            for r in 0..rh {
                let from = ((y0 + r) * w + x0) * 4;
                pixels.copy_within(from..from + rw * 4, r * rw * 4);
            }
            pixels.truncate(rw * rh * 4);
            pixels.shrink_to_fit();
            Baked {
                left: x0 as i32,
                top: y0 as i32,
                width: rw as u32,
                height: rh as u32,
                pixels,
            }
        };
        self.add(name, u64::from(region.width) * u64::from(region.height) * 4)?;
        Ok(region)
    }

    fn flip_pixels(&self, pixels: &mut [u8]) {
        if self.flip {
            yolu_core::normal::flip_green(pixels)
        }
    }

    /// 保存した画素: 面のあるタイルの外接矩形（面の無いチャンネルは 1×1 の透明）。
    fn stored(&mut self, l: &CoreLayer) -> XResult<Baked> {
        let d = self.d;
        let (mut left, mut bottom, mut right, mut top) = (d.width(), d.height(), 0, 0);
        let surface = l.surface(self.c);
        for c in surface.map(|s| s.tile_coords()).unwrap_or_default() {
            left = left.min(c.x * d.tile_size());
            bottom = bottom.min(c.y * d.tile_size());
            right = d.width().min(right.max((c.x + 1) * d.tile_size()));
            top = d.height().min(top.max((c.y + 1) * d.tile_size()))
        }
        if right <= left || top <= bottom {
            left = 0;
            bottom = 0;
            right = 1;
            top = 1
        }
        let width = right - left;
        let height = top - bottom;
        self.add(l.name(), u64::from(width) * u64::from(height) * 4)?;
        let mut pixels = vec![0; width as usize * height as usize * 4];
        if let Some(surface) = surface {
            // タイルごとに行を写す（面の無いタイルは透明のまま）。1 画素ずつ引くと、全面のレイヤーでタイルの探索が画素の数だけ要る
            let ts = d.tile_size() as usize;
            let (canvas_w, canvas_h) = (d.width() as usize, d.height() as usize);
            let mut tile = vec![0u8; surface.tile_bytes()];
            for c in surface.tile_coords() {
                self.check_cancel()?;
                surface.copy_tile(c, &mut tile).map_err(Error::from)?;
                let (x0, y0) = (c.x as usize * ts, c.y as usize * ts);
                let n = ts.min(canvas_w - x0);
                for row in 0..ts.min(canvas_h - y0) {
                    // core の行（下から）を、レイヤーの行（上から）へ
                    let at =
                        (top as usize - 1 - (y0 + row)) * width as usize + (x0 - left as usize);
                    pixels[at * 4..(at + n) * 4]
                        .copy_from_slice(&tile[row * ts * 4..(row * ts + n) * 4])
                }
            }
        }
        self.flip_pixels(&mut pixels);
        Ok(Baked {
            left: left as i32,
            top: (d.height() - top) as i32,
            width,
            height,
            pixels,
        })
    }

    /// core のマスク（隠す量をアルファに持ち、左下原点）→ PSD のマスク（255 が見える、上から下）。焼くときは、フィルターを通した隠す量を
    /// 使い、反転は値を反転して画素にする（有効と濃度はそのまま）。矩形は既定の値と違う画素の外接矩形で、既定の値は 255 と 0 のうち
    /// 矩形が小さくなるほう（同じなら 255）。キャンバスのどこでも同じ値になる。
    fn mask(&mut self, l: &CoreLayer, m: &RasterMask, baked: bool) -> XResult<MaskRegion<'static>> {
        let d = self.d;
        let (w, h, ts) = (
            d.width() as usize,
            d.height() as usize,
            d.tile_size() as usize,
        );
        self.add(l.name(), (w * h) as u64)?;
        let values = if baked {
            let mut hide = vec![0u8; w * h];
            d.mask_output_into(
                l.id(),
                d.bounds(),
                &mut hide,
                RowOrder::TopDown,
                self.cancel,
            )
            .map_err(Error::from)?;
            if !m.inverted() {
                for v in &mut hide {
                    *v = 255 - *v
                }
            }
            hide
        } else {
            // 無いタイルは何も隠さない（見える = 255）
            let mut values = vec![255u8; w * h];
            let surface = m.surface();
            let mut tile = vec![0u8; surface.tile_bytes()];
            for c in surface.tile_coords() {
                surface.copy_tile(c, &mut tile).map_err(Error::from)?;
                let (x0, y0) = (c.x as usize * ts, c.y as usize * ts);
                for row in 0..ts.min(h - y0) {
                    let psd_row = (h - 1 - (y0 + row)) * w + x0;
                    for col in 0..ts.min(w - x0) {
                        values[psd_row + col] = 255 - tile[(row * ts + col) * 4 + 3]
                    }
                }
            }
            values
        };
        let white = bounds(&values, w, 255);
        let black = bounds(&values, w, 0);
        let area = |b: (usize, usize, usize, usize)| b.2 * b.3;
        let background = if area(black) < area(white) { 0 } else { 255 };
        let mut rect = if background == 0 { black } else { white };
        // 一様なマスクも 1×1 の矩形を書く（空の矩形を誤って読む書き手がある）
        if rect.2 == 0 || rect.3 == 0 {
            rect = (0, 0, 1, 1)
        }
        let mut pixels = Vec::with_capacity(rect.2 * rect.3);
        for y in 0..rect.3 {
            pixels.extend_from_slice(&values[(rect.1 + y) * w + rect.0..][..rect.2])
        }
        Ok(MaskRegion {
            left: rect.0 as i32,
            top: rect.1 as i32,
            width: rect.2 as u32,
            height: rect.3 as u32,
            default_color: background,
            values: Cow::Owned(pixels),
        })
    }
}

/// 構造のレイヤーの並びの記録の数（グループは区切りの記録も 1 つ要る）。
fn skeleton_count(layers: &[Layer]) -> usize {
    layers
        .iter()
        .map(|l| match &l.kind {
            LayerKind::Group { children, .. } => 2 + skeleton_count(children),
            _ => 1,
        })
        .sum()
}

/// 背景と違う値の外接矩形（左・上・幅・高さ。無ければ幅 0）。
fn bounds(values: &[u8], w: usize, background: u8) -> (usize, usize, usize, usize) {
    let h = values.len() / w;
    let (mut left, mut top, mut right, mut bottom) = (w, h, 0, 0);
    for (y, row) in values.chunks_exact(w).enumerate() {
        let Some(first) = row.iter().position(|v| *v != background) else {
            continue;
        };
        let last = row.iter().rposition(|v| *v != background).expect("あった");
        left = left.min(first);
        right = right.max(last + 1);
        top = top.min(y);
        bottom = bottom.max(y + 1);
    }
    if right <= left {
        (0, 0, 0, 0)
    } else {
        (left, top, right - left, bottom - top)
    }
}

/// 厳密な書き出し（Color）の断りの理由（下のレイヤーから。そのまま書けるなら空）。`from_core` が断るのと同じ判断。
pub fn export_blockers(d: &CoreDocument) -> Vec<Blocker> {
    let mut out = Vec::new();
    for l in d.layers() {
        for need in needs(d, l, Channel::Color) {
            out.push(Blocker {
                layer: l.name().into(),
                refusal: need.refusal(),
            })
        }
    }
    out
}

/// 厳密な書き出し（Color）: そのまま書けないもの（焼く・丸める・落とす）が 1 つでもあれば、レイヤーの名前と理由で断る。使う上限の値だけ `ctl` から
/// （既定は C# と対の固定の上限）。振る舞い（何を断るか）は変わらない。
pub(super) fn from_core_strict(d: &CoreDocument, ctl: &ExportControl) -> Result<Document> {
    Ok(Builder::new(d, &ExportOptions::color(), ctl, true)?.run(Compression::Raw)?)
}
