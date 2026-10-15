//! 影響元（CLIP STUDIO の「影響元設定」に当たる）: ブラシの項目（大きさ・不透明度・流量・硬さ・真円率・質感の濃さ・絵の具の量・
//! 絵の具の濃さ。[`Item`]）ごとに、入力（筆圧・傾き・速度・ランダム。[`Input`]）を使うかと、入力ごとの応え（最小値と曲線。
//! [`InputResponse`]）を持つ。正本は [`super::Brush::influences`] の 1 か所。
//!
//! - 入力の値 x（0〜1）: 筆圧はそのまま。傾きは 1 − 傾きの量（直立 1・寝かせきって 0。[`super::pen_tilt::amount`]）。速度は
//!   1 − min(1, 速さ ÷ [`Influences::speed_max`])（止まって 1・上限の速さで 0）。ランダムは、描点ごとに、ランダムを使う項目ごとに 1 回
//!   引く一様乱数 [0, 1)（ストロークの種 ^ 影響元の列の種の乱数を、項目の並びの順に引く。どの項目も使わなければ 1 つも引かない）。
//! - 応え: 係数 = 最小値 + (1 − 最小値) × 曲線(x)。最小値は x = 0 のときの係数（0〜1）、曲線は点を通る単調な 3 次エルミート
//!   （共通の値のカーブ [`crate::curve::Curve`] ＝ 塗りつぶしのグラデーション・トーンカーブと同じ PCHIP。点は 2〜16 個、両端は入力 0 と 1、
//!   間隔は 0.02 以上、値は 0〜1）。**既定（最小値 0・直線）は入力をそのまま返す**（浮動小数の演算を 1 つも挟まない）。直線は
//!   「曲線の点が無い」か「(0,0) と (1,1) の 2 点」で、どちらも同じ既定として扱う。
//! - 項目の係数は、使っている入力ごとの応え(x) の積（使っていない入力は掛けない）。切った入力も最小値と曲線を残す（[`InputUse`]）。
//!   掛ける順は項目ごとに決まっている: 大きさは呼び手の半径（2D は半径 × 筆圧 × 入り抜き）× フェード × 傾き × 速度 × ランダム × ゆらぎ、
//!   不透明度・流量は 設定 × (フェード × 傾き × 速度 × ランダム × ゆらぎ) × 筆圧、硬さは 硬さ × 筆圧 × 傾き × 速度 × ランダム、
//!   真円率は 真円率 × 係数（下限 0.01）を真円率のゆらぎの前に、質感の濃さは紙の質感の深さ × 係数をそのダブだけに、絵の具の量・濃さは
//!   量 × (筆圧 × 傾き × 速度 × ランダム)。
//!
//! 簡単な丸いブラシ（パスのブラシ・選択のペン・GPU のダブ）は影響元を持たず、筆圧で大きさ・不透明度・流量を変える切り替えだけを
//! [`PressureSwitches`] に持つ（応えは直線）。

use crate::curve::{Curve, CurvePoint};
use crate::error::CoreError;
use crate::math::{clamp01, require_finite};
use crate::Reason;

use super::random::NetRandom;

/// 直線の曲線の点（両端だけ）。
pub const STRAIGHT: [CurvePoint; 2] =
    [CurvePoint { x: 0.0, y: 0.0 }, CurvePoint { x: 1.0, y: 1.0 }];

/// 曲線の点の数の上限。
pub const MAX_CURVE_POINTS: usize = Curve::MAX_POINTS;

/// 入力 1 つの応え（最小値と曲線）。作るときに検査するので、持っている値はいつも範囲の中。
#[derive(Clone, Debug, PartialEq)]
pub struct InputResponse {
    min: f64,
    /// 直線でない曲線（直線は None）。
    curve: Option<Curve>,
}

impl Default for InputResponse {
    /// 最小値 0・直線（入力をそのまま使う）。
    fn default() -> Self {
        InputResponse::IDENTITY
    }
}

impl InputResponse {
    /// 最小値 0・直線（入力をそのまま返す）。
    pub const IDENTITY: InputResponse = InputResponse {
        min: 0.0,
        curve: None,
    };

    /// 最小値（0〜1）と曲線の点（空か 2 点の直線は直線）から作る。範囲外・有限でない値・形の合わない曲線は断る。
    pub fn new(min: f64, curve: Vec<CurvePoint>) -> Result<InputResponse, CoreError> {
        InputResponse::build(min, curve, 0.0)
    }

    /// 最小値が −1〜1 の応え（色相・彩度・明度の影響元。最小値が負なら、入力 0 で係数が負になる: 最小値 −1 は係数 −1 + 2 × 曲線(x)）。
    pub fn new_signed(min: f64, curve: Vec<CurvePoint>) -> Result<InputResponse, CoreError> {
        InputResponse::build(min, curve, -1.0)
    }

    /// 最小値が lo〜1 の応え（lo は 0 か −1）。
    fn build(min: f64, curve: Vec<CurvePoint>, lo: f64) -> Result<InputResponse, CoreError> {
        let curve = if curve.is_empty() {
            Curve::identity()
        } else {
            Curve::new(curve).map_err(|_| CoreError::InvalidArgument(Reason::InputCurveRange))?
        };
        InputResponse::build_from_curve(min, curve, lo)
    }

    /// 最小値（0〜1）と、検査済みの曲線から作る（編集の部品が返した曲線をそのまま入れる口。直線は直線として扱う）。
    pub fn from_curve(min: f64, curve: Curve) -> Result<InputResponse, CoreError> {
        InputResponse::build_from_curve(min, curve, 0.0)
    }

    fn build_from_curve(min: f64, curve: Curve, lo: f64) -> Result<InputResponse, CoreError> {
        require_finite(min, Reason::InputMinimum)?;
        if !(lo..=1.0).contains(&min) {
            return Err(CoreError::InvalidArgument(if lo < 0.0 {
                Reason::InputMinimumSignedRange
            } else {
                Reason::InputMinimumRange
            }));
        }
        Ok(InputResponse {
            min,
            curve: (!curve.is_identity()).then_some(curve),
        })
    }

    /// 最小値だけを変えた写し（曲線はそのまま。最小値は 0〜1）。
    pub fn with_min(&self, min: f64) -> Result<InputResponse, CoreError> {
        InputResponse::new(min, self.curve().to_vec())
    }

    /// 最小値だけを変えた写し（曲線はそのまま。最小値は −1〜1。色相・彩度・明度の影響元）。
    pub fn with_min_signed(&self, min: f64) -> Result<InputResponse, CoreError> {
        InputResponse::new_signed(min, self.curve().to_vec())
    }

    /// 曲線だけを変えた写し（最小値はそのまま。負の最小値も通る）。
    pub fn with_curve(&self, curve: Vec<CurvePoint>) -> Result<InputResponse, CoreError> {
        InputResponse::build(self.min, curve, -1.0)
    }

    /// 検査済みの曲線へ差し替えた写し（最小値はそのまま。編集の部品が返した曲線を入れる）。
    pub fn with_curve_shape(&self, curve: Curve) -> Result<InputResponse, CoreError> {
        InputResponse::build_from_curve(self.min, curve, -1.0)
    }

    /// 入力 0 のときの係数（0〜1）。
    pub fn min(&self) -> f64 {
        self.min
    }

    /// 曲線の点（直線は空）。
    pub fn curve(&self) -> &[CurvePoint] {
        self.curve.as_ref().map_or(&[], |c| c.points())
    }

    /// 編集の部品へ渡す曲線（直線なら両端の 2 点の直線）。
    pub fn curve_shape(&self) -> Curve {
        self.curve.clone().unwrap_or_default()
    }

    /// 編集の元にする曲線の点（直線なら両端の 2 点）。
    pub fn curve_points(&self) -> Vec<CurvePoint> {
        match &self.curve {
            Some(c) => c.points().to_vec(),
            None => STRAIGHT.to_vec(),
        }
    }

    /// 最小値 0・直線か（このとき [`InputResponse::apply`] は入力をそのまま返す）。
    pub fn is_identity(&self) -> bool {
        self.min == 0.0 && self.curve.is_none()
    }

    /// 持っている値を確かめ直す（作るときに検査するので、ふつうは通る。検査を通らない値は作れない）。
    pub fn validate(&self) -> Result<(), CoreError> {
        InputResponse::new(self.min, self.curve().to_vec()).map(|_| ())
    }

    /// [`InputResponse::validate`] の、最小値が −1〜1 の形（色相・彩度・明度の影響元）。
    pub fn validate_signed(&self) -> Result<(), CoreError> {
        InputResponse::new_signed(self.min, self.curve().to_vec()).map(|_| ())
    }

    /// 入力 x（0〜1）の係数。恒等なら x そのもの。
    #[inline]
    pub fn apply(&self, x: f64) -> f64 {
        if self.is_identity() {
            return x;
        }
        let p = if x.is_finite() { clamp01(x) } else { 0.0 };
        let shaped = match &self.curve {
            Some(curve) => curve.value_unchecked(p),
            None => p,
        };
        self.min + (1.0 - self.min) * shaped
    }

    /// 最小値と曲線の点を、画面の精度（f32）に丸めた写し（保存して読み戻しても同じ値になる形）。丸めて範囲を外れるときは元のまま。
    pub fn rounded_to_f32(&self) -> InputResponse {
        let f = |v: f64| v as f32 as f64;
        let points = self
            .curve()
            .iter()
            .map(|p| CurvePoint {
                x: f(p.x),
                y: f(p.y),
            })
            .collect();
        InputResponse::new_signed(f(self.min), points).unwrap_or_else(|_| self.clone())
    }
}

/// 影響元の入力。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Input {
    /// 筆圧（0〜1）。
    Pressure,
    /// ペンの傾き（直立 1・寝かせきって 0）。
    Tilt,
    /// 筆の速さ（止まって 1・上限の速さで 0）。
    Speed,
    /// 描点ごとの一様乱数 [0, 1)。
    Random,
}

impl Input {
    /// すべての入力（並びは係数を掛ける順・保存の順）。
    pub const ALL: [Input; 4] = [Input::Pressure, Input::Tilt, Input::Speed, Input::Random];

    /// 保存に使う名前。
    pub fn id(self) -> &'static str {
        match self {
            Input::Pressure => "pressure",
            Input::Tilt => "tilt",
            Input::Speed => "speed",
            Input::Random => "random",
        }
    }
}

/// 影響元を持てるブラシの項目。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Item {
    Size,
    Opacity,
    Flow,
    Hardness,
    /// 真円率（筆先を潰す割合）。
    Roundness,
    /// 紙の質感の濃さ（深さ）。
    TextureDensity,
    /// 色の混ぜの絵の具の量。
    MixPaint,
    /// 色の混ぜの絵の具の濃さ。
    MixDensity,
}

impl Item {
    /// すべての項目（並びはランダムを引く順・保存の順）。
    pub const ALL: [Item; 8] = [
        Item::Size,
        Item::Opacity,
        Item::Flow,
        Item::Hardness,
        Item::Roundness,
        Item::TextureDensity,
        Item::MixPaint,
        Item::MixDensity,
    ];

    /// 保存に使う名前。
    pub fn id(self) -> &'static str {
        match self {
            Item::Size => "size",
            Item::Opacity => "opacity",
            Item::Flow => "flow",
            Item::Hardness => "hardness",
            Item::Roundness => "roundness",
            Item::TextureDensity => "texture_density",
            Item::MixPaint => "mix_paint",
            Item::MixDensity => "mix_density",
        }
    }
}

/// 色の変化の影響元を持つ項目（[`Item`] とは別の並び。ランダムの乱数は色の変化の列から、ダブごとにこの順で引く）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorItem {
    /// 先端の色相の変化。
    Hue,
    /// 先端の彩度の変化。
    Saturation,
    /// 先端の明度の変化。
    Value,
    /// サブ描画色混合率。
    SubMix,
}

impl ColorItem {
    /// すべての色の項目（並びは乱数を引く順・保存の順）。
    pub const ALL: [ColorItem; 4] = [
        ColorItem::Hue,
        ColorItem::Saturation,
        ColorItem::Value,
        ColorItem::SubMix,
    ];

    /// 保存に使う名前。
    pub fn id(self) -> &'static str {
        match self {
            ColorItem::Hue => "color_hue",
            ColorItem::Saturation => "color_saturation",
            ColorItem::Value => "color_value",
            ColorItem::SubMix => "color_sub_mix",
        }
    }

    /// 応えの最小値が −1〜1 か（色相・彩度・明度。サブ描画色混合率は 0〜1）。
    pub fn signed(self) -> bool {
        self != ColorItem::SubMix
    }
}

/// 散布効果の影響元を持つ項目（[`Item`] とは別の並び。ランダムの乱数は散布の列から引く: 密度は描点ごと、粒子サイズは粒子ごと）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SprayItem {
    /// 粒子サイズ。
    Size,
    /// 粒子密度。
    Density,
}

impl SprayItem {
    /// すべての散布の項目（並びは保存の順）。
    pub const ALL: [SprayItem; 2] = [SprayItem::Size, SprayItem::Density];

    /// 保存に使う名前。
    pub fn id(self) -> &'static str {
        match self {
            SprayItem::Size => "spray_size",
            SprayItem::Density => "spray_density",
        }
    }
}

/// デュアルブラシ（2 つ目の筆先）の影響元を持つ項目（[`Item`] とは別の並び。入力はその 2 つ目のダブの所の値で、ランダムの乱数は
/// デュアルの影響元の列から、2 つ目のダブごとにこの順で引く）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DualItem {
    /// 2 つ目のブラシサイズ。
    Size,
    /// 2 つ目のブラシ濃度。
    Flow,
    /// 2 つ目の間隔。
    Spacing,
    /// 2 つ目の厚さ（真円率）。
    Thickness,
}

impl DualItem {
    /// すべてのデュアルの項目（並びは乱数を引く順・保存の順）。
    pub const ALL: [DualItem; 4] = [
        DualItem::Size,
        DualItem::Flow,
        DualItem::Spacing,
        DualItem::Thickness,
    ];

    /// 保存に使う名前。
    pub fn id(self) -> &'static str {
        match self {
            DualItem::Size => "dual_size",
            DualItem::Flow => "dual_flow",
            DualItem::Spacing => "dual_spacing",
            DualItem::Thickness => "dual_thickness",
        }
    }
}

/// 1 つの項目の 1 つの入力: 使うかと、その応え。切っても応え（最小値と曲線）は残す。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct InputUse {
    pub on: bool,
    pub response: InputResponse,
}

impl InputUse {
    /// 切っていて、応えが直線の入力。
    pub const OFF: InputUse = InputUse {
        on: false,
        response: InputResponse::IDENTITY,
    };
    /// 使っていて、応えが直線の入力。
    pub const STRAIGHT: InputUse = InputUse {
        on: true,
        response: InputResponse::IDENTITY,
    };

    /// 切っていて、応えも既定か（保存しない形）。
    pub fn is_default(&self) -> bool {
        !self.on && self.response.is_identity()
    }

    /// 使っていれば、入力 x の係数。
    #[inline]
    fn factor(&self, x: f64) -> Option<f64> {
        self.on.then(|| self.response.apply(x))
    }
}

/// 1 つの項目の影響元（4 つの入力）。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Influence {
    pub pressure: InputUse,
    pub tilt: InputUse,
    pub speed: InputUse,
    pub random: InputUse,
}

impl Influence {
    /// どの入力も使わない影響元。
    pub const OFF: Influence = Influence {
        pressure: InputUse::OFF,
        tilt: InputUse::OFF,
        speed: InputUse::OFF,
        random: InputUse::OFF,
    };
    /// 筆圧だけを直線で使う影響元。
    pub const BY_PRESSURE: Influence = Influence {
        pressure: InputUse::STRAIGHT,
        tilt: InputUse::OFF,
        speed: InputUse::OFF,
        random: InputUse::OFF,
    };

    pub fn input(&self, input: Input) -> &InputUse {
        match input {
            Input::Pressure => &self.pressure,
            Input::Tilt => &self.tilt,
            Input::Speed => &self.speed,
            Input::Random => &self.random,
        }
    }

    pub fn input_mut(&mut self, input: Input) -> &mut InputUse {
        match input {
            Input::Pressure => &mut self.pressure,
            Input::Tilt => &mut self.tilt,
            Input::Speed => &mut self.speed,
            Input::Random => &mut self.random,
        }
    }

    /// どれかの入力を使うか。
    pub fn is_on(&self) -> bool {
        Input::ALL.into_iter().any(|i| self.input(i).on)
    }

    /// どの入力も切っていて、応えも既定か。
    pub fn is_default(&self) -> bool {
        Input::ALL.into_iter().all(|i| self.input(i).is_default())
    }

    /// 筆圧の係数（筆圧を使っていなければ None）。
    #[inline]
    pub(crate) fn pressure_factor(&self, pressure: f64) -> Option<f64> {
        self.pressure.factor(pressure)
    }

    /// v に、使っている筆圧でない入力（傾き・速度・ランダム）の係数をこの順に掛ける（使っていない入力は掛けない）。
    #[inline]
    pub(crate) fn times_others(&self, mut v: f64, x: &InputValues, random: Option<f64>) -> f64 {
        if let Some(f) = self.tilt.factor(x.upright) {
            v *= f;
        }
        if let Some(f) = self.speed.factor(x.slow) {
            v *= f;
        }
        if let (Some(r), true) = (random, self.random.on) {
            v *= self.random.response.apply(r);
        }
        v
    }

    /// 使っている筆圧・傾き・速度の係数の積（ランダムを除く。どれも使っていなければ 1）。
    #[inline]
    pub(crate) fn others_product(&self, x: &InputValues) -> f64 {
        let v = self.pressure_factor(x.pressure).unwrap_or(1.0);
        self.times_others(v, x, None)
    }

    /// ランダムの係数（ランダムを使っていなければ 1）。u は [0, 1) の乱数。
    #[inline]
    pub(crate) fn random_factor(&self, u: f64) -> f64 {
        self.random.factor(u).unwrap_or(1.0)
    }

    /// 使っている入力の係数の積（筆圧・傾き・速度・ランダムの順。どの入力も使っていなければ None）。
    #[inline]
    pub(crate) fn factor(&self, x: &InputValues, random: Option<f64>) -> Option<f64> {
        if !self.is_on() {
            return None;
        }
        let v = self.pressure_factor(x.pressure).unwrap_or(1.0);
        Some(self.times_others(v, x, random))
    }

    /// 応えを画面の精度（f32）に丸めた写し。
    pub fn rounded_to_f32(&self) -> Influence {
        let r = |u: &InputUse| InputUse {
            on: u.on,
            response: u.response.rounded_to_f32(),
        };
        Influence {
            pressure: r(&self.pressure),
            tilt: r(&self.tilt),
            speed: r(&self.speed),
            random: r(&self.random),
        }
    }
}

/// ブラシの影響元（項目ごと）と、入力の共通の値。
#[derive(Clone, Debug, PartialEq)]
pub struct Influences {
    pub size: Influence,
    pub opacity: Influence,
    pub flow: Influence,
    pub hardness: Influence,
    /// 真円率（[`super::TipShape::roundness`] × 係数。下限 0.01）。
    pub roundness: Influence,
    /// 紙の質感の濃さ（[`super::PaperTexture::depth`] × 係数。そのダブだけ）。
    pub texture_density: Influence,
    /// 色の混ぜの絵の具の量・濃さ（[`super::ColorMix`] の量・濃さ × 係数。混ぜるブラシだけに効く）。
    pub mix_paint: Influence,
    pub mix_density: Influence,
    /// 色の変化の影響元（[`ColorItem`]。先端の色相・彩度・明度・サブ描画色混合率）。色相・彩度・明度の応えの最小値は −1〜1。
    /// 変化量 = [`super::ColorDynamics`] の値 × 使っている入力の係数の積。
    pub color_hue: Influence,
    pub color_saturation: Influence,
    pub color_value: Influence,
    pub color_sub_mix: Influence,
    /// 散布効果の影響元（[`SprayItem`]。粒子サイズ・粒子密度）。値 = [`super::Spray`] の値 × 使っている入力の係数の積（最小値は 0〜1）。
    pub spray_size: Influence,
    pub spray_density: Influence,
    /// デュアルブラシの影響元（[`DualItem`]。2 つ目のサイズ・濃度・間隔・厚さ）。値 = [`super::DualBrush`] の値 × 使っている入力の
    /// 係数の積（最小値は 0〜1）。
    pub dual_size: Influence,
    pub dual_flow: Influence,
    pub dual_spacing: Influence,
    pub dual_thickness: Influence,
    /// 速度の効きが一杯になる速さ（画素 / 時刻の単位。秒なら画素毎秒）。0 より大きい。速さは手ぶれ補正の後の筆の点の間の距離 ÷ 時刻の差で、
    /// 点の間は線形に補間する。時刻が進まない入力（時刻を渡さない）では最初の 0 のまま。
    pub speed_max: f64,
}

impl Default for Influences {
    /// 筆圧で大きさと不透明度を変える（応えは直線。[`PressureSwitches`] の既定と同じ）。ほかは使わない。
    fn default() -> Self {
        Influences::from_switches(PressureSwitches::default())
    }
}

impl Influences {
    /// 速度の効きが一杯になる速さの既定。
    pub const DEFAULT_SPEED_MAX: f64 = 3000.0;

    /// どの項目も、どの入力も使わない影響元。
    pub fn none() -> Influences {
        Influences {
            size: Influence::OFF,
            opacity: Influence::OFF,
            flow: Influence::OFF,
            hardness: Influence::OFF,
            roundness: Influence::OFF,
            texture_density: Influence::OFF,
            mix_paint: Influence::OFF,
            mix_density: Influence::OFF,
            color_hue: Influence::OFF,
            color_saturation: Influence::OFF,
            color_value: Influence::OFF,
            color_sub_mix: Influence::OFF,
            spray_size: Influence::OFF,
            spray_density: Influence::OFF,
            dual_size: Influence::OFF,
            dual_flow: Influence::OFF,
            dual_spacing: Influence::OFF,
            dual_thickness: Influence::OFF,
            speed_max: Influences::DEFAULT_SPEED_MAX,
        }
    }

    /// 簡単なブラシの筆圧の切り替えだけの影響元（応えは直線）。
    pub fn from_switches(switches: PressureSwitches) -> Influences {
        let mut i = Influences::none();
        i.set_pressure_switches(switches);
        i
    }

    pub fn get(&self, item: Item) -> &Influence {
        match item {
            Item::Size => &self.size,
            Item::Opacity => &self.opacity,
            Item::Flow => &self.flow,
            Item::Hardness => &self.hardness,
            Item::Roundness => &self.roundness,
            Item::TextureDensity => &self.texture_density,
            Item::MixPaint => &self.mix_paint,
            Item::MixDensity => &self.mix_density,
        }
    }

    pub fn get_mut(&mut self, item: Item) -> &mut Influence {
        match item {
            Item::Size => &mut self.size,
            Item::Opacity => &mut self.opacity,
            Item::Flow => &mut self.flow,
            Item::Hardness => &mut self.hardness,
            Item::Roundness => &mut self.roundness,
            Item::TextureDensity => &mut self.texture_density,
            Item::MixPaint => &mut self.mix_paint,
            Item::MixDensity => &mut self.mix_density,
        }
    }

    pub fn color(&self, item: ColorItem) -> &Influence {
        match item {
            ColorItem::Hue => &self.color_hue,
            ColorItem::Saturation => &self.color_saturation,
            ColorItem::Value => &self.color_value,
            ColorItem::SubMix => &self.color_sub_mix,
        }
    }

    pub fn color_mut(&mut self, item: ColorItem) -> &mut Influence {
        match item {
            ColorItem::Hue => &mut self.color_hue,
            ColorItem::Saturation => &mut self.color_saturation,
            ColorItem::Value => &mut self.color_value,
            ColorItem::SubMix => &mut self.color_sub_mix,
        }
    }

    /// 色の項目のどれかが、どれかの入力を使うか。
    pub fn color_is_on(&self) -> bool {
        ColorItem::ALL.into_iter().any(|i| self.color(i).is_on())
    }

    /// 色の項目のどれかがランダムを使うか。
    pub fn color_uses_random(&self) -> bool {
        ColorItem::ALL.into_iter().any(|i| self.color(i).random.on)
    }

    pub fn spray(&self, item: SprayItem) -> &Influence {
        match item {
            SprayItem::Size => &self.spray_size,
            SprayItem::Density => &self.spray_density,
        }
    }

    pub fn spray_mut(&mut self, item: SprayItem) -> &mut Influence {
        match item {
            SprayItem::Size => &mut self.spray_size,
            SprayItem::Density => &mut self.spray_density,
        }
    }

    pub fn dual(&self, item: DualItem) -> &Influence {
        match item {
            DualItem::Size => &self.dual_size,
            DualItem::Flow => &self.dual_flow,
            DualItem::Spacing => &self.dual_spacing,
            DualItem::Thickness => &self.dual_thickness,
        }
    }

    pub fn dual_mut(&mut self, item: DualItem) -> &mut Influence {
        match item {
            DualItem::Size => &mut self.dual_size,
            DualItem::Flow => &mut self.dual_flow,
            DualItem::Spacing => &mut self.dual_spacing,
            DualItem::Thickness => &mut self.dual_thickness,
        }
    }

    /// デュアルの項目のどれかが、どれかの入力を使うか。
    pub fn dual_is_on(&self) -> bool {
        DualItem::ALL.into_iter().any(|i| self.dual(i).is_on())
    }

    /// 大きさ・不透明度・流量を筆圧で変えるか（簡単なブラシの切り替えの形）。
    pub fn pressure_switches(&self) -> PressureSwitches {
        PressureSwitches {
            size: self.size.pressure.on,
            opacity: self.opacity.pressure.on,
            flow: self.flow.pressure.on,
        }
    }

    /// 大きさ・不透明度・流量の筆圧の切り替えだけを入れ替える（応えとほかの入力はそのまま）。
    pub fn set_pressure_switches(&mut self, switches: PressureSwitches) {
        self.size.pressure.on = switches.size;
        self.opacity.pressure.on = switches.opacity;
        self.flow.pressure.on = switches.flow;
    }

    /// どれかの項目が入力 input を使うか（傾き・速度を求めるかの判定。色と散布とデュアルの項目も数える。それらのランダムは自分の乱数の列から
    /// 引くので、影響元の乱数の列を要るかの判定（ランダム）には数えない）。
    pub fn uses(&self, input: Input) -> bool {
        Item::ALL.into_iter().any(|i| self.get(i).input(input).on)
            || (input != Input::Random
                && ColorItem::ALL
                    .into_iter()
                    .any(|i| self.color(i).input(input).on))
            || (input != Input::Random
                && SprayItem::ALL
                    .into_iter()
                    .any(|i| self.spray(i).input(input).on))
            || (input != Input::Random
                && DualItem::ALL
                    .into_iter()
                    .any(|i| self.dual(i).input(input).on))
    }

    /// 範囲を確かめる（応えの最小値と曲線の形、速度の上限は 0 より大きい有限の数）。断ったら何も変えない。
    pub fn validate(&self) -> Result<(), CoreError> {
        for item in Item::ALL {
            for input in Input::ALL {
                self.get(item).input(input).response.validate()?;
            }
        }
        for item in ColorItem::ALL {
            for input in Input::ALL {
                let response = &self.color(item).input(input).response;
                if item.signed() {
                    response.validate_signed()?;
                } else {
                    response.validate()?;
                }
            }
        }
        for item in SprayItem::ALL {
            for input in Input::ALL {
                self.spray(item).input(input).response.validate()?;
            }
        }
        for item in DualItem::ALL {
            for input in Input::ALL {
                self.dual(item).input(input).response.validate()?;
            }
        }
        require_finite(self.speed_max, Reason::SpeedMax)?;
        if self.speed_max <= 0.0 {
            return Err(CoreError::InvalidArgument(
                Reason::MaximumSpeedGreaterThanZero,
            ));
        }
        Ok(())
    }

    /// 色の 4 項目の、筆圧・傾き・速度の係数の積（ランダムを除く。使わない項目は 1）。
    pub(crate) fn color_others(&self, x: &InputValues) -> [f64; 4] {
        let mut out = [1.0; 4];
        if self.color_is_on() {
            for (slot, item) in out.iter_mut().zip(ColorItem::ALL) {
                *slot = self.color(item).others_product(x);
            }
        }
        out
    }

    /// 散布の 2 項目（[`SprayItem::ALL`] の並び）の、筆圧・傾き・速度の係数の積（ランダムを除く。使わない項目は 1）。
    pub(crate) fn spray_others(&self, x: &InputValues) -> [f64; 2] {
        SprayItem::ALL.map(|item| self.spray(item).others_product(x))
    }

    /// 応えを画面の精度（f32）に丸めた写し（速さの上限はそのまま）。
    pub fn rounded_to_f32(&self) -> Influences {
        let mut out = self.clone();
        for item in Item::ALL {
            *out.get_mut(item) = self.get(item).rounded_to_f32();
        }
        for item in ColorItem::ALL {
            *out.color_mut(item) = self.color(item).rounded_to_f32();
        }
        for item in SprayItem::ALL {
            *out.spray_mut(item) = self.spray(item).rounded_to_f32();
        }
        for item in DualItem::ALL {
            *out.dual_mut(item) = self.dual(item).rounded_to_f32();
        }
        out
    }

    /// 描点 1 つの、ランダムを使う項目ごとの乱数（項目の並びの順に 1 つずつ引く。使わない項目は None で、引かない）。
    pub(crate) fn draw_random(&self, random: Option<&mut NetRandom>) -> [Option<f64>; 8] {
        let mut out = [None; 8];
        if let Some(r) = random {
            for (slot, item) in out.iter_mut().zip(Item::ALL) {
                if self.get(item).random.on {
                    *slot = Some(r.next_double());
                }
            }
        }
        out
    }
}

/// 簡単な丸いブラシ（[`super::BrushSettings`] と一緒に使う。パスのブラシ・選択のペン・GPU のダブ）の、筆圧で大きさ・不透明度・流量を
/// 変えるか（それぞれ直線）。全部入りのブラシでは [`Influences`] の筆圧の切り替えに当たる。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PressureSwitches {
    pub size: bool,
    pub opacity: bool,
    pub flow: bool,
}

impl Default for PressureSwitches {
    /// 大きさと不透明度を筆圧で変える（C# の BrushSettings の既定と同じ）。
    fn default() -> Self {
        PressureSwitches {
            size: true,
            opacity: true,
            flow: false,
        }
    }
}

impl PressureSwitches {
    /// 筆圧で何も変えない。
    pub const NONE: PressureSwitches = PressureSwitches {
        size: false,
        opacity: false,
        flow: false,
    };
}

/// 描点 1 つの入力の値（ランダムは項目ごとに引くので含まない）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct InputValues {
    pub pressure: f64,
    /// 1 − 傾きの量。
    pub upright: f64,
    /// 1 − min(1, 速さ ÷ 速さの上限)。
    pub slow: f64,
}

/// 影響元の乱数の列（ストロークの種へ混ぜる値。色・デュアルの列と重ならない）。
pub(crate) const INFLUENCE_STREAM: i32 = 0x1B87_3593;

/// 1 つのダブ（か 1 回の画素の呼び出し）の、筆圧を応えに通した後の不透明度と流量の係数（切っている項目は 1）。
/// 色の混ぜの絵の具の量・濃さの係数も持つ（混ぜない・切っているときは 1。2D の描点では筆圧でない入力の係数も掛けてある）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PressureScale {
    pub opacity: f64,
    pub flow: f64,
    pub mix_paint: f64,
    pub mix_density: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f64, y: f64) -> CurvePoint {
        CurvePoint { x, y }
    }

    #[test]
    fn the_default_and_a_straight_curve_return_the_pressure_untouched() {
        for r in [
            InputResponse::default(),
            InputResponse::new(0.0, vec![]).unwrap(),
            InputResponse::new(0.0, STRAIGHT.to_vec()).unwrap(),
        ] {
            assert!(r.is_identity());
            assert_eq!(r, InputResponse::default());
            for p in [0.0, 1e-300, 0.1234567891011, 0.5, 0.999999999, 1.0] {
                assert_eq!(r.apply(p).to_bits(), p.to_bits());
            }
        }
    }

    #[test]
    fn the_minimum_lifts_pressure_zero_and_keeps_one() {
        let r = InputResponse::new(0.25, vec![]).unwrap();
        assert!(!r.is_identity());
        assert_eq!(r.apply(0.0), 0.25);
        assert_eq!(r.apply(1.0), 1.0);
        assert_eq!(r.apply(0.5), 0.25 + 0.75 * 0.5);
        // 最小値 1 は筆圧を無視して 1
        let full = InputResponse::new(1.0, vec![]).unwrap();
        assert_eq!(full.apply(0.0), 1.0);
        assert_eq!(full.apply(0.3), 1.0);
    }

    #[test]
    fn a_signed_minimum_makes_the_factor_run_from_minus_one_to_one() {
        let r = InputResponse::new_signed(-1.0, vec![]).unwrap();
        assert_eq!(r.apply(0.0), -1.0);
        assert_eq!(r.apply(0.5), 0.0);
        assert_eq!(r.apply(1.0), 1.0);
        assert!(r.validate().is_err() && r.validate_signed().is_ok());
        // 曲線だけ変えても、最小値は保つ
        let bent = r
            .with_curve(vec![pt(0.0, 0.0), pt(0.5, 0.8), pt(1.0, 1.0)])
            .unwrap();
        assert_eq!(bent.min(), -1.0);
        assert!((bent.apply(0.5) - (-1.0 + 2.0 * 0.8)).abs() < 1e-12);
        // f32 に丸めても負の最小値は残る
        let r = InputResponse::new_signed(-0.1, vec![])
            .unwrap()
            .rounded_to_f32();
        assert_eq!(r.min(), -0.1f32 as f64);
        assert!(InputResponse::new_signed(-1.01, vec![]).is_err());
    }

    #[test]
    fn a_curve_bends_the_response_and_the_minimum_applies_after_it() {
        let soft = vec![pt(0.0, 0.0), pt(0.5, 0.8), pt(1.0, 1.0)];
        let r = InputResponse::new(0.0, soft.clone()).unwrap();
        assert!((r.apply(0.5) - 0.8).abs() < 1e-12, "{}", r.apply(0.5));
        assert_eq!(r.apply(0.0), 0.0);
        assert_eq!(r.apply(1.0), 1.0);
        let mut last = 0.0;
        for i in 0..=100 {
            let v = r.apply(i as f64 / 100.0);
            assert!(v >= last - 1e-12 && (0.0..=1.0).contains(&v), "{i}: {v}");
            last = v;
        }
        let both = InputResponse::new(0.2, soft).unwrap();
        assert!((both.apply(0.5) - (0.2 + 0.8 * 0.8)).abs() < 1e-12);
        assert_eq!(both.apply(0.0), 0.2);
        assert_eq!(both.apply(1.0), 1.0);
    }

    #[test]
    fn a_curve_that_ends_below_one_scales_the_top() {
        let r = InputResponse::new(0.0, vec![pt(0.0, 0.0), pt(1.0, 0.6)]).unwrap();
        assert!((r.apply(1.0) - 0.6).abs() < 1e-12);
        assert!((r.apply(0.5) - 0.3).abs() < 1e-12);
    }

    #[test]
    fn bad_minimums_and_curves_are_refused() {
        for min in [-0.1, 1.01, f64::NAN, f64::INFINITY] {
            assert!(InputResponse::new(min, vec![]).is_err(), "{min}");
        }
        let bad: Vec<Vec<CurvePoint>> = vec![
            vec![pt(0.0, 0.0)],                                           // 1 点
            vec![pt(0.0, 0.0), pt(0.5, 0.5)],                             // 終わりが 1 でない
            vec![pt(0.1, 0.0), pt(1.0, 1.0)],                             // 始まりが 0 でない
            vec![pt(0.0, 0.0), pt(0.5, 1.5), pt(1.0, 1.0)],               // 値が 1 を超える
            vec![pt(0.0, 0.0), pt(0.5, -0.1), pt(1.0, 1.0)],              // 値が負
            vec![pt(0.0, 0.0), pt(0.5, f64::NAN), pt(1.0, 1.0)],          // 有限でない
            vec![pt(0.0, 0.0), pt(0.01, 0.5), pt(1.0, 1.0)],              // 間隔が狭い
            vec![pt(0.0, 0.0), pt(0.6, 0.5), pt(0.4, 0.7), pt(1.0, 1.0)], // 入力の順でない
            (0..17)
                .map(|i| pt(i as f64 / 16.0, i as f64 / 16.0))
                .collect(), // 17 点
        ];
        for (i, curve) in bad.into_iter().enumerate() {
            assert!(InputResponse::new(0.0, curve).is_err(), "case {i}");
        }
        let sixteen: Vec<CurvePoint> = (0..16)
            .map(|i| pt(i as f64 / 15.0, i as f64 / 15.0))
            .collect();
        assert!(InputResponse::new(0.0, sixteen).is_ok());
    }

    #[test]
    fn a_checked_curve_goes_in_and_comes_back_out_unchanged() {
        let shaped = Curve::new(vec![pt(0.0, 0.0), pt(0.4, 0.7), pt(1.0, 1.0)]).unwrap();
        let r = InputResponse::from_curve(0.1, shaped.clone()).unwrap();
        assert_eq!(r.curve_shape(), shaped);
        assert_eq!(r.curve().len(), 3);
        assert_eq!(r.min(), 0.1);
        // 点の列から作ったものと同じ（評価も含めて）
        let from_points = InputResponse::new(0.1, shaped.points().to_vec()).unwrap();
        assert_eq!(r, from_points);
        for p in [0.0, 0.2, 0.4, 0.77, 1.0] {
            assert_eq!(r.apply(p).to_bits(), from_points.apply(p).to_bits());
        }
        // 差し替えは最小値を保つ。直線へ戻すと既定の曲線（点なし）に戻る
        let back = r.with_curve_shape(Curve::identity()).unwrap();
        assert_eq!(back.min(), 0.1);
        assert!(back.curve().is_empty());
        assert_eq!(back, InputResponse::new(0.1, vec![]).unwrap());
        assert_eq!(InputResponse::default().curve_shape(), Curve::identity());
        assert!(InputResponse::from_curve(1.5, Curve::identity()).is_err());
        assert!(InputResponse::from_curve(f64::NAN, Curve::identity()).is_err());
    }

    #[test]
    fn rounding_to_f32_is_stable_and_keeps_the_shape() {
        let r = InputResponse::new(
            0.123456789012345,
            vec![
                pt(0.0, 0.0),
                pt(0.3333333333333333, 0.7777777777777777),
                pt(1.0, 1.0),
            ],
        )
        .unwrap();
        let once = r.rounded_to_f32();
        assert_eq!(once.rounded_to_f32(), once);
        assert_eq!(once.min(), 0.123_456_79_f32 as f64);
        assert_eq!(once.curve().len(), 3);
        assert!((once.apply(0.4) - r.apply(0.4)).abs() < 1e-6);
    }

    fn x(pressure: f64, upright: f64, slow: f64) -> InputValues {
        InputValues {
            pressure,
            upright,
            slow,
        }
    }

    fn used(min: f64, curve: Vec<CurvePoint>) -> InputUse {
        InputUse {
            on: true,
            response: InputResponse::new(min, curve).unwrap(),
        }
    }

    #[test]
    fn an_influence_multiplies_the_responses_of_the_inputs_it_uses() {
        let soft = vec![pt(0.0, 0.0), pt(0.5, 0.8), pt(1.0, 1.0)];
        let i = Influence {
            pressure: used(0.2, vec![]),
            tilt: used(0.0, soft.clone()),
            speed: used(0.5, vec![]),
            random: used(0.1, vec![]),
        };
        let v = x(0.5, 0.25, 0.4);
        let expect = (0.2 + 0.8 * 0.5)
            * InputResponse::new(0.0, soft).unwrap().apply(0.25)
            * (0.5 + 0.5 * 0.4)
            * (0.1 + 0.9 * 0.7);
        let got = i.factor(&v, Some(0.7)).unwrap();
        assert!((got - expect).abs() < 1e-12, "{got} {expect}");
        // 筆圧でない入力だけを掛ける口は、筆圧を掛けない
        let others = i.times_others(2.0, &v, Some(0.7));
        assert!((others - 2.0 * expect / (0.2 + 0.8 * 0.5)).abs() < 1e-12);
    }

    #[test]
    fn inputs_that_are_off_are_not_multiplied_even_with_a_response() {
        // 切った入力の応え（最小値と曲線）は残るが、係数には掛けない: 値はビットまで同じ
        let mut i = Influence::BY_PRESSURE;
        i.tilt.response = InputResponse::new(0.3, vec![pt(0.0, 1.0), pt(1.0, 0.0)]).unwrap();
        i.speed.response = InputResponse::new(0.9, vec![]).unwrap();
        i.random.response = InputResponse::new(0.4, vec![]).unwrap();
        for v in [0.0, 1e-300, 0.123_456_789, 0.5, 1.0] {
            let values = x(v, 0.37, 0.61);
            assert_eq!(i.factor(&values, Some(0.9)).unwrap().to_bits(), v.to_bits());
            assert_eq!(i.times_others(v, &values, Some(0.9)).to_bits(), v.to_bits());
        }
        // どの入力も使わなければ係数は無い（呼び手は何も掛けない）
        assert_eq!(Influence::OFF.factor(&x(0.5, 0.5, 0.5), Some(0.5)), None);
        assert_eq!(Influence::OFF.pressure_factor(0.5), None);
        let kept = Influence {
            pressure: InputUse {
                on: false,
                response: InputResponse::new(0.5, vec![]).unwrap(),
            },
            ..Influence::OFF
        };
        assert!(!kept.is_on() && !kept.is_default());
        assert_eq!(kept.factor(&x(0.1, 0.1, 0.1), None), None);
    }

    #[test]
    fn the_default_straight_inputs_return_the_input_itself() {
        // 既定の応えの傾き・速度は、今までの式（1 − 傾き・1 − 速さの割合）をそのまま掛ける
        let i = Influence {
            tilt: InputUse::STRAIGHT,
            speed: InputUse::STRAIGHT,
            ..Influence::OFF
        };
        let v = x(0.0, 1.0 - 0.3, 1.0 - 0.25);
        let fade = 0.875;
        assert_eq!(
            i.times_others(fade, &v, None).to_bits(),
            (fade * (1.0 - 0.3) * (1.0 - 0.25)).to_bits()
        );
    }

    #[test]
    fn random_values_follow_the_seed_and_the_item_order_and_none_are_drawn_when_unused() {
        let mut inf = Influences::none();
        inf.flow.random = InputUse::STRAIGHT;
        inf.texture_density.random = InputUse::STRAIGHT;
        let draws = |seed: i32| {
            let mut r = NetRandom::new(seed);
            (0..3)
                .map(|_| inf.draw_random(Some(&mut r)))
                .collect::<Vec<_>>()
        };
        assert_eq!(draws(7), draws(7));
        assert_ne!(draws(7), draws(8));
        // 使う項目だけ、項目の並びの順に 1 つずつ引く（流量が先、質感の濃さが次）
        let mut r = NetRandom::new(7);
        let mut plain = NetRandom::new(7);
        let first = inf.draw_random(Some(&mut r));
        assert_eq!(first[2], Some(plain.next_double()));
        assert_eq!(first[5], Some(plain.next_double()));
        for (k, v) in first.iter().enumerate() {
            assert_eq!(v.is_some(), k == 2 || k == 5, "{k}");
            if let Some(v) = v {
                assert!((0.0..1.0).contains(v));
            }
        }
        // どの項目もランダムを使わなければ、1 つも引かない（列は進まない）
        let none = Influences::default();
        assert!(!none.uses(Input::Random));
        let mut r = NetRandom::new(11);
        assert_eq!(none.draw_random(Some(&mut r)), [None; 8]);
        assert_eq!(r.next_double(), NetRandom::new(11).next_double());
    }

    #[test]
    fn only_a_spray_source_using_the_tilt_makes_the_influences_use_the_tilt() {
        let mut inf = Influences::none();
        assert!(!inf.uses(Input::Tilt));
        inf.spray_mut(SprayItem::Density).input_mut(Input::Tilt).on = true;
        assert!(inf.uses(Input::Tilt));
        assert!(!inf.uses(Input::Speed));
        // ランダムは自分の乱数の列から引くので、影響元の乱数の列を要るかには数えない
        inf.spray_mut(SprayItem::Size).input_mut(Input::Random).on = true;
        assert!(!inf.uses(Input::Random));
    }

    #[test]
    fn the_default_influences_match_the_simple_brush_switches() {
        let d = Influences::default();
        assert_eq!(d.pressure_switches(), PressureSwitches::default());
        assert!(d.size.pressure.on && d.opacity.pressure.on && !d.flow.pressure.on);
        assert!(Item::ALL
            .into_iter()
            .filter(|i| !matches!(i, Item::Size | Item::Opacity))
            .all(|i| d.get(i).is_default()));
        assert_eq!(d.speed_max, Influences::DEFAULT_SPEED_MAX);
        let mut i = Influences::none();
        i.size.pressure.response = InputResponse::new(0.4, vec![]).unwrap();
        i.set_pressure_switches(PressureSwitches {
            size: true,
            opacity: false,
            flow: true,
        });
        // 切り替えだけを入れ替え、応えは残す
        assert_eq!(i.size.pressure.response.min(), 0.4);
        assert_eq!(
            i.pressure_switches(),
            PressureSwitches {
                size: true,
                opacity: false,
                flow: true
            }
        );
        assert_eq!(
            Influences::from_switches(PressureSwitches::NONE),
            Influences::none()
        );
    }

    #[test]
    fn validate_refuses_a_speed_limit_that_is_not_positive_and_finite() {
        assert!(Influences::default().validate().is_ok());
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let i = Influences {
                speed_max: bad,
                ..Influences::default()
            };
            assert!(i.validate().is_err(), "{bad}");
        }
        // 応えは作るときに検査するので、持っている応えはいつも通る（範囲外・有限でない・形の合わない曲線は作れない）
        let mut i = Influences::none();
        for item in Item::ALL {
            for input in Input::ALL {
                i.get_mut(item).input_mut(input).response =
                    InputResponse::new(0.25, vec![pt(0.0, 1.0), pt(1.0, 0.0)]).unwrap();
            }
        }
        assert!(i.validate().is_ok());
        assert!(InputResponse::new(-0.01, vec![]).is_err());
        assert!(InputResponse::new(f64::INFINITY, vec![]).is_err());
        assert!(InputResponse::new(0.0, vec![pt(0.0, 0.0), pt(0.5, 1.2), pt(1.0, 1.0)]).is_err());
    }
}
