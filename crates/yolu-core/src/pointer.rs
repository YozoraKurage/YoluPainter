//! 参照レイヤー（[`crate::LayerKind::Pointer`]）: 画素を持たず「指す先」の合成を読むレイヤー。読んだ結果の上に、ふつうのレイヤーと同じく
//! マスク・フィルター・合成モード・不透明度が掛かる。
//!
//! - 指す先は 3 つ: 同じ文書のレイヤー・グループ（その部分木だけを透明から重ねた結果。下のスタックを含めない。指したレイヤー自身のマスク・
//!   不透明度・合成モードは掛かり、クリッピングは見ない）、同じ文書のレイヤーセット（そのレイヤーセットの合成。組に無いチャンネルは透明）、
//!   ほかのテクスチャセットのレイヤーセット（[`ExternalSet`]。文書の外の鍵で、合成はアプリが向こうの文書で作って
//!   [`crate::EffectInputs::with_pointer`] で渡す。UV で読み、大きさが違えば双線形で引き伸ばす）。
//! - チャンネルごとに読む元のチャンネルを選べる（既定は自分と同じチャンネル。値は保存したままのバイトで、色空間・種類の変換はしない）。
//! - 循環（指す先の中に自分か、自分を読むレイヤーがある）は作らない。参照の連なりは [`MAX_POINTER_DEPTH`] 段まで。どちらも、作る・指す先を
//!   替える・並びを変える・読み込むで断る。ほかのテクスチャセットをまたぐ循環と深さは、core は 1 つの文書しか見ないので、アプリが
//!   [`crate::Document::pointer_reach`] と [`crate::Document::external_pointers`] で判定する。
//! - 指す先が無い（消した・渡されていない）・読む元のチャンネルが無いときは、そのチャンネルの既定値を出し、効いていない効果
//!   （[`crate::InactiveTarget::Pointer`]）に理由を出す。

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use crate::error::CoreError;
use crate::layer::{LayerId, LayerSetId};
use crate::types::{Channel, Rgba8};
use crate::Reason;

/// 参照の連なりの上限（参照レイヤーが指す先の中に、また参照レイヤーがある段の数。参照レイヤー 1 枚で 1 段。ほかのテクスチャセットを
/// またぐ段も数える）。
pub const MAX_POINTER_DEPTH: usize = 4;

/// 外から渡す参照の入力 1 枚の辺の上限（画素）。
pub const MAX_POINTER_INPUT_EDGE: u32 = 8192;

/// ほかのテクスチャセットのレイヤーセットを指す鍵。core は意味を知らない（アプリが .ylp のテクスチャセットの ID と、その文書の
/// レイヤーセットの ID を入れる）。
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ExternalSet {
    pub texture_set: u128,
    pub layer_set: u128,
}

impl fmt::Debug for ExternalSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ExternalSet({:032x}/{:032x})",
            self.texture_set, self.layer_set
        )
    }
}

/// 参照レイヤーの指す先。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PointerTarget {
    /// 同じ文書のレイヤー・グループ（その部分木だけ）。
    Layer(LayerId),
    /// 同じ文書のレイヤーセット（その合成）。
    LayerSet(LayerSetId),
    /// ほかのテクスチャセットのレイヤーセット（アプリが渡す入力を読む）。
    External(ExternalSet),
}

impl PointerTarget {
    /// 保存の番号（0 レイヤー・グループ、1 レイヤーセット、2 ほかのテクスチャセットのレイヤーセット）。
    pub const fn index(self) -> u8 {
        match self {
            Self::Layer(_) => 0,
            Self::LayerSet(_) => 1,
            Self::External(_) => 2,
        }
    }
}

/// 参照レイヤーの設定（指す先と、チャンネルごとの読む元）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PointerSettings {
    pub(crate) target: PointerTarget,
    /// チャンネルごとの読む元（無いチャンネルは自分と同じチャンネルを読む。同じチャンネルを指す項目は持たない）。
    pub(crate) sources: BTreeMap<Channel, Channel>,
}

impl PointerSettings {
    /// 指す先だけの設定（どのチャンネルも自分と同じチャンネルを読む）。
    pub fn new(target: PointerTarget) -> Self {
        PointerSettings {
            target,
            sources: BTreeMap::new(),
        }
    }
    /// 読む元を 1 つ替えた設定（`source` が None か `channel` と同じなら、既定の自分と同じチャンネルへ戻す）。
    pub fn with_source(mut self, channel: Channel, source: Option<Channel>) -> Self {
        match source {
            Some(s) if s != channel => {
                self.sources.insert(channel, s);
            }
            _ => {
                self.sources.remove(&channel);
            }
        }
        self
    }
    pub fn target(&self) -> PointerTarget {
        self.target
    }
    /// チャンネル `channel` に出す値を、指す先のどのチャンネルから読むか。
    pub fn source(&self, channel: Channel) -> Channel {
        self.sources.get(&channel).copied().unwrap_or(channel)
    }
    /// 既定（同じチャンネル）から替えた読む元（チャンネルの番号の順）。
    pub fn sources(&self) -> impl Iterator<Item = (Channel, Channel)> + '_ {
        self.sources.iter().map(|(c, s)| (*c, *s))
    }
}

/// 外から渡す参照の入力 1 枚: ほかのテクスチャセットのレイヤーセットの、1 つのチャンネルの合成（straight RGBA8、左下原点、行優先）。
#[derive(Clone, Debug)]
pub struct PointerInput {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
}

impl PointerInput {
    /// 大きさ（1〜8192）と長さを確かめて作る。
    pub fn new(width: u32, height: u32, pixels: Arc<[u8]>) -> Result<Self, CoreError> {
        if width == 0
            || height == 0
            || width > MAX_POINTER_INPUT_EDGE
            || height > MAX_POINTER_INPUT_EDGE
            || pixels.len() as u64 != u64::from(width) * u64::from(height) * 4
        {
            return Err(CoreError::InvalidArgument(
                Reason::ImageSizeAndPixelLengthDoNotMatch,
            ));
        }
        Ok(PointerInput {
            width,
            height,
            pixels,
        })
    }
    /// 画素のバイト数（メモリの予算に数える量）。
    pub fn bytes(&self) -> u64 {
        self.pixels.len() as u64
    }
    /// 同じ大きさ・画素か（共有していれば中身を見ない）。
    pub(crate) fn same_pixels(&self, other: &PointerInput) -> bool {
        self.width == other.width
            && self.height == other.height
            && (Arc::ptr_eq(&self.pixels, &other.pixels) || self.pixels == other.pixels)
    }
    /// 文書の画素 (x, y)（大きさ `dims`）に当たる値。同じ大きさなら同じ画素、違えば UV（画素の中心）で双線形に引く。混ぜるのは
    /// アルファを掛けた値で（透明な画素の RGB を周りへにじませない）、最後に straight へ戻して丸める。
    pub(crate) fn sample(&self, x: u32, y: u32, dims: (u32, u32)) -> Rgba8 {
        let (w, h) = (self.width, self.height);
        let at = |px: u32, py: u32| -> [u8; 4] {
            let i = (py as usize * w as usize + px as usize) * 4;
            [
                self.pixels[i],
                self.pixels[i + 1],
                self.pixels[i + 2],
                self.pixels[i + 3],
            ]
        };
        if (w, h) == dims {
            return Rgba8::from_slice(&at(x, y));
        }
        let u = (f64::from(x) + 0.5) / f64::from(dims.0) * f64::from(w) - 0.5;
        let v = (f64::from(y) + 0.5) / f64::from(dims.1) * f64::from(h) - 0.5;
        let (x0, fx) = split(u, w);
        let (y0, fy) = split(v, h);
        let x1 = (x0 + 1).min(w - 1);
        let y1 = (y0 + 1).min(h - 1);
        let taps = [
            (at(x0, y0), (1.0 - fx) * (1.0 - fy)),
            (at(x1, y0), fx * (1.0 - fy)),
            (at(x0, y1), (1.0 - fx) * fy),
            (at(x1, y1), fx * fy),
        ];
        let mut acc = [0.0f64; 4];
        for (p, wgt) in taps {
            let a = f64::from(p[3]) * wgt;
            acc[0] += f64::from(p[0]) * a;
            acc[1] += f64::from(p[1]) * a;
            acc[2] += f64::from(p[2]) * a;
            acc[3] += a;
        }
        if acc[3] <= 0.0 {
            return Rgba8::TRANSPARENT;
        }
        let byte = |v: f64| v.round().clamp(0.0, 255.0) as u8;
        Rgba8::new(
            byte(acc[0] / acc[3]),
            byte(acc[1] / acc[3]),
            byte(acc[2] / acc[3]),
            byte(acc[3]),
        )
    }
}

/// 座標を、端で止めた整数の画素と端数に分ける。
fn split(v: f64, len: u32) -> (u32, f64) {
    let max = f64::from(len - 1);
    let c = v.clamp(0.0, max);
    let i = c.floor();
    (i as u32, c - i)
}

/// 参照レイヤーが今は指す先を読めない理由（そのチャンネルの既定値を出している）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerIssue {
    /// 指したレイヤー・グループ・レイヤーセットが文書に無い（消した）。
    MissingTarget,
    /// ほかのテクスチャセットの合成がまだ渡されていない（向こうのセット・レイヤーセットが無い・読めない）。
    MissingInput,
    /// 渡された合成がメモリの予算に収まらない。
    InputOverBudget,
    /// 読む元のチャンネルが文書に無い。
    MissingChannel(Channel),
    /// 指す先の中に、自分か自分を読むレイヤーがある。
    Cycle,
    /// 参照の連なりが [`MAX_POINTER_DEPTH`] 段を超える。
    TooDeep,
}

impl fmt::Display for PointerIssue {
    /// 日本語の理由の文。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTarget => f.write_str("指す先がありません"),
            Self::MissingInput => f.write_str("ほかのテクスチャセットの合成がまだありません"),
            Self::InputOverBudget => {
                f.write_str("ほかのテクスチャセットの合成がメモリの予算に収まりません")
            }
            Self::MissingChannel(c) => write!(f, "読む元のチャンネル {c:?} がありません"),
            Self::Cycle => f.write_str("指す先が循環しています"),
            Self::TooDeep => write!(f, "ポインターの連なりが {MAX_POINTER_DEPTH} 段を超えます"),
        }
    }
}

/// レイヤーセット（またはレイヤー）から辿れる参照の連なり（[`crate::Document::pointer_reach`]）。アプリが、テクスチャセットをまたぐ
/// 深さと循環を判定する材料。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PointerReach {
    /// この文書の中だけで数えた、一番長い参照の連なりの段の数（参照レイヤーが無ければ 0。ほかのテクスチャセットを指す参照レイヤーは 1 段に数え、
    /// その先は数えない）。
    pub depth: usize,
    /// 辿った先にある、ほかのテクスチャセットを指す参照レイヤー（同じ鍵は、段の多い方だけ）。
    pub external: Vec<ExternalReach>,
}

/// 辿った先にある、ほかのテクスチャセットを指す参照レイヤー 1 つ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExternalReach {
    pub layer: LayerId,
    pub key: ExternalSet,
    /// そこまでの段の数（その参照レイヤー自身を含む）。全体の深さは、これに向こうのレイヤーセットの深さを足したもの。
    pub levels: usize,
}
