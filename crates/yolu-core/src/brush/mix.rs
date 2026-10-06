//! 色の混ぜ（厚塗りのブラシ。CLIP STUDIO の混色・絵の具の量・絵の具の濃さ・色延びに当たる拡張）。
//!
//! 今までのブラシは描く色を置くだけで、下の色は拾わない。混ぜるブラシは、ダブごとに下地（打点の前に凍結した枠）の色を読み、
//! 描く色と混ぜてから置く。式は `docs/BRUSH.md` の「色の混ぜ」にも書いてある。色はすべて straight RGBA（0〜255 の倍精度）で扱い、
//! 置くときは今までと同じ道（ストロークの覆い `wash` ＋ 画素ごとの色 ＋ [`super::blend64::blend`]）を通す。
//!
//! - 筆の荷（ブラシが今持っている色）`C`: 最初の打点は描く色。次の打点からは `C = 荷 × 色延び + 描く色 × (1 − 色延び)`
//!   （荷は、前の打点が下地から拾って混ぜた後の色）。色延び 0 なら毎回描く色から、1 なら拾った色を引きずったまま。
//! - 画素の混ぜた色 `M(p)`: 下地の色 `U(p)`（混ぜるは同じ画素、伸ばすは打点の動きの分だけ後ろを小さい箱で平均した所）と荷 `C` を、
//!   絵の具の量 `A` で `mix_colors(U, C, A)` する。**混ぜるはストロークを始める前の絵**を読むので、同じストロークが画素に何度重なっても
//!   混ざり方は変わらない（画素に残るのは最後に覆った打点の `M`）。**伸ばすは今の面**（このストロークが先に置いた分を含む）を読み、
//!   先に引きずった色を後の打点が運ぶ。アルファで重みを付けた補間（効果のブラシの [`super::effects::mix_effect`] と同じ重み）で、
//!   透明な下地の色は混ざらない。アルファは描く色のまま（下地が透けていても薄まらない）。重みが 0 の画素は塗らない。
//! - 打点の後、荷は「混ぜた色の平均」になる: 打点の下地の（被覆 × アルファで重みを付けた）平均色 `Ū` と `mix_colors(Ū, C, A)`。下地が
//!   透明で何も拾えなければ荷は変わらない。
//! - 絵の具の濃さ `D`: ストロークの不透明度の天井（今までの不透明度 × 筆圧 × ゆらぎ…）へ掛ける倍率。置く量。
//! - 絵の具の量と濃さは、筆圧の応え（最小値と曲線。[`super::PressureResponse`]）で変えられる: `A × 応え(筆圧)`、`D × 応え(筆圧)`。
//! - 混ぜるのは色のチャンネルだけ（データのチャンネル・ノーマル・マスクは値が壊れるので混ぜない）。消しゴムと効果のブラシ
//!   （ぼかし・指先・クローン）では使わない（設定は残るが効かない）。
//! - 並列に描くダブも順に描くダブも同じ画素になる: 読む枠は打点の前に凍結し、荷の更新に使う平均はタイルごとの合計を
//!   タイルの順に足したもの（どの経路も同じ足し算の列）。

use super::effects::{byte255, EffectFrame};
use super::pressure::PressureResponse;
use crate::error::CoreError;
use crate::math::require_finite;
use crate::types::Rgba8;

/// 混ぜ方。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MixMode {
    /// 混ぜない（今までのブラシ。ファイルも画素もバイト単位で同じ）。
    #[default]
    Off,
    /// 絵の具で混ぜる: 下の色（同じ画素）と描く色を、絵の具の量で混ぜて置く。
    Mix,
    /// ぼかしのように伸ばす: 打点の動きの分だけ後ろの下の色を、小さい箱で平均して読み、描く色と混ぜて置く。3D の面では、指先と同じく
    /// 展開の図で UV の継ぎ目をまたいで直前の打点の側を読む（箱の平均はなし）。対称（2D・3D）と組むときは、写しごとに動きの向きが違うので
    /// 動きの向きを使わず、同じ画素の周りの箱だけ。
    Smear,
}

impl MixMode {
    /// 保存の順。
    pub const ALL: [MixMode; 3] = [MixMode::Off, MixMode::Mix, MixMode::Smear];
}

/// 混ぜる下地。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MixGround {
    /// 今描いている層だけ。
    #[default]
    Layer,
    /// 見えている層の重なり（今の評価の道で合成したもの）。ストロークの最初の打点の前に凍結するので、このストロークが置いた分は
    /// 読まない。混ぜるでは今の層だけのときと同じ（読むのはストロークを始める前の絵）。伸ばすでは、先に引きずった色が運ばれず、
    /// 動きの後ろの元の絵の色を読む。
    Composite,
}

impl MixGround {
    pub const ALL: [MixGround; 2] = [MixGround::Layer, MixGround::Composite];
}

/// 色の混ぜの設定（ブラシの 1 項目。既定は混ぜない）。
#[derive(Clone, Debug, PartialEq)]
pub struct ColorMix {
    pub mode: MixMode,
    /// 絵の具の量 0〜1（既定 0.5）: 混ぜた色に占める描く色（荷）の割合。1 は下の色を拾わず、0 は下の色だけ（何も足さない）。
    pub paint: f64,
    /// 絵の具の濃さ 0〜1（既定 1）: 置く量。ストロークの不透明度の天井に掛ける。
    pub density: f64,
    /// 色延び 0〜1（既定 0.5）: 前の打点で拾った色を次の打点へ引きずる割合。荷は 1 打点ごとに絵の具の量だけ残り、残りは下地の平均へ入れ替わる
    /// ので、量が多いほど引きずりが長く続く。伸ばすでは、読む位置の遅れの長さにも効く（1 + 2 × 色延び）。
    pub stretch: f64,
    pub ground: MixGround,
    /// 筆圧で絵の具の量・濃さを変えるか（切っている項目は筆圧を使わない）。
    pub pressure_paint: bool,
    pub pressure_density: bool,
    /// 筆圧の応え（最小値と曲線）。切り替えが真のときだけ使う。
    pub response_paint: PressureResponse,
    pub response_density: PressureResponse,
}

impl Default for ColorMix {
    fn default() -> Self {
        ColorMix {
            mode: MixMode::Off,
            paint: 0.5,
            density: 1.0,
            stretch: 0.5,
            ground: MixGround::Layer,
            pressure_paint: false,
            pressure_density: false,
            response_paint: PressureResponse::default(),
            response_density: PressureResponse::default(),
        }
    }
}

impl ColorMix {
    /// 混ぜるか（効くかどうかは、さらにチャンネル・消しゴム・効果で決まる）。
    pub fn is_active(&self) -> bool {
        self.mode != MixMode::Off
    }

    /// 見えている層の重なりを下地にするか（ストロークの前に参照元を凍結する必要がある）。
    pub fn wants_composite(&self) -> bool {
        self.is_active() && self.ground == MixGround::Composite
    }

    /// 範囲を確かめる（断ったら何も変えない）。
    pub fn validate(&self) -> Result<(), CoreError> {
        for (v, what) in [
            (self.paint, "絵の具の量（0〜1）"),
            (self.density, "絵の具の濃さ（0〜1）"),
            (self.stretch, "色延び（0〜1）"),
        ] {
            require_finite(v, "color mix")?;
            if !(0.0..=1.0).contains(&v) {
                return Err(CoreError::InvalidArgument(what));
            }
        }
        Ok(())
    }

    /// 値を画面の精度（f32）に丸めた写し（保存して読み戻しても同じ値になる形）。
    pub fn rounded_to_f32(&self) -> ColorMix {
        let f = |v: f64| v as f32 as f64;
        ColorMix {
            paint: f(self.paint),
            density: f(self.density),
            stretch: f(self.stretch),
            response_paint: self.response_paint.rounded_to_f32(),
            response_density: self.response_density.rounded_to_f32(),
            ..self.clone()
        }
    }

    /// 使っている項目（混ぜ方が切のブラシは、ほかの項目が既定のままでも使っていない）。保存の版を決める。
    pub fn is_default(&self) -> bool {
        *self == ColorMix::default()
    }

    /// 筆圧 p のときの、絵の具の量・濃さの係数（切っている項目は 1。応えが既定なら筆圧そのもの）。
    pub(crate) fn pressure_factors(&self, pressure: f64) -> (f64, f64) {
        (
            if self.pressure_paint {
                self.response_paint.apply(pressure)
            } else {
                1.0
            },
            if self.pressure_density {
                self.response_density.apply(pressure)
            } else {
                1.0
            },
        )
    }
}

/// 打点 1 つ分の混ぜの値（ダブの画素の処理が読む。ワーカーからも読む）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct MixDab {
    pub mode: MixMode,
    /// 筆の荷（straight RGBA、0〜255）。アルファは描く色のもの。
    pub carry: [f64; 4],
    /// 筆圧を掛けた後の絵の具の量。
    pub paint: f64,
    /// 筆圧を掛けた後の絵の具の濃さ（天井への倍率）。
    pub density: f64,
    /// 伸ばすで、読む位置のずれ（画素）と、平均の箱の半径（画素。混ぜるは 0）。
    pub shift: (i64, i64),
    pub blur: i64,
}

/// アルファで重みを付けた補間（straight RGBA、0〜255）: 下地 `ground` と荷 `carry` を、荷の割合 `amount` で混ぜる。重みは
/// 下地 = アルファ × (1 − amount)、荷 = アルファ × amount。アルファは荷のまま。重みの和が 0 なら None（混ぜる色が無い）。
#[inline]
pub(crate) fn mix_colors(ground: [f64; 4], carry: [f64; 4], amount: f64) -> Option<[f64; 4]> {
    let wu = ground[3] * (1.0 - amount);
    let wc = carry[3] * amount;
    let w = wu + wc;
    if w <= 0.0 {
        return None;
    }
    Some([
        (ground[0] * wu + carry[0] * wc) / w,
        (ground[1] * wu + carry[1] * wc) / w,
        (ground[2] * wu + carry[2] * wc) / w,
        carry[3],
    ])
}

#[inline]
fn straight(c: Rgba8) -> [f64; 4] {
    [c.r as f64, c.g as f64, c.b as f64, c.a as f64]
}

#[inline]
fn to_rgba8(c: [f64; 4]) -> Rgba8 {
    Rgba8::new(byte255(c[0]), byte255(c[1]), byte255(c[2]), byte255(c[3]))
}

impl MixDab {
    /// 画素 (px, py) の下地の読み（画布は w × h）: 混ぜるは同じ画素、伸ばすは動きの分だけずらした所の箱の平均（画布の端の外は端の画素）。
    #[inline]
    pub(crate) fn ground_at(&self, frame: &EffectFrame, px: i64, py: i64, w: i64, h: i64) -> Rgba8 {
        match self.mode {
            MixMode::Smear => frame.blur(
                (px + self.shift.0).clamp(0, w - 1),
                (py + self.shift.1).clamp(0, h - 1),
                self.blur,
                w,
                h,
            ),
            _ => frame.pixel(px, py),
        }
    }

    /// 打点の画素の混ぜた色（`ground` は下地の読み。None は混ぜる色が無く、この画素は塗らない）。
    #[inline]
    pub(crate) fn pixel_color(&self, ground: Rgba8) -> Option<Rgba8> {
        mix_colors(straight(ground), self.carry, self.paint).map(to_rgba8)
    }

    /// この打点が終わった後の荷: 下地の平均色と混ぜたもの。何も拾えなかった（下地が透明）なら今の荷のまま。
    pub(crate) fn loaded_after(&self, tally: &MixTally) -> [f64; 4] {
        if tally.cover <= 0.0 || tally.weight <= 0.0 {
            return self.carry;
        }
        let ground = [
            tally.rgb[0] / tally.weight,
            tally.rgb[1] / tally.weight,
            tally.rgb[2] / tally.weight,
            255.0 * tally.weight / tally.cover,
        ];
        mix_colors(ground, self.carry, self.paint).unwrap_or(self.carry)
    }
}

/// 次の打点の荷: 前の荷と描く色を、色延びで混ぜる（最初の打点・色延び 0 は描く色そのもの）。アルファは描く色のまま。
pub(crate) fn next_carry(loaded: Option<[f64; 4]>, draw: Rgba8, stretch: f64) -> [f64; 4] {
    let d = straight(draw);
    match loaded {
        Some(l) if stretch > 0.0 => [
            l[0] * stretch + d[0] * (1.0 - stretch),
            l[1] * stretch + d[1] * (1.0 - stretch),
            l[2] * stretch + d[2] * (1.0 - stretch),
            d[3],
        ],
        _ => d,
    }
}

/// 打点の下地の、被覆とアルファで重みを付けた合計（荷の更新の元。タイルごとに作ってタイルの順に足す）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct MixTally {
    /// 被覆の合計。
    pub cover: f64,
    /// 被覆 × 下地のアルファ（0〜1）の合計。
    pub weight: f64,
    /// 被覆 × 下地のアルファ × 下地の RGB（0〜255）の合計。
    pub rgb: [f64; 3],
}

impl MixTally {
    #[inline]
    pub(crate) fn add(&mut self, coverage: f64, ground: Rgba8) {
        self.cover += coverage;
        let w = coverage * (ground.a as f64 / 255.0);
        self.weight += w;
        self.rgb[0] += w * ground.r as f64;
        self.rgb[1] += w * ground.g as f64;
        self.rgb[2] += w * ground.b as f64;
    }

    /// 別のタイルの合計を足す（呼ぶ順は経路によらずタイルの順）。
    pub(crate) fn merge(&mut self, other: MixTally) {
        self.cover += other.cover;
        self.weight += other.weight;
        for (a, b) in self.rgb.iter_mut().zip(other.rgb) {
            *a += b;
        }
    }
}

/// ストロークが持つ混ぜの状態。
#[derive(Debug, Default)]
pub(crate) struct MixRun {
    /// 前の打点が終わった後の荷（最初の打点の前は None）。
    pub loaded: Option<[f64; 4]>,
    /// 今の（まだ荷へ畳んでいない）打点の値。
    pub dab: Option<MixDab>,
    /// 今の打点の下地の合計。
    pub tally: MixTally,
}

impl MixRun {
    /// 前の打点を荷へ畳む（次の打点を始める前に呼ぶ）。
    pub(crate) fn settle(&mut self) {
        if let Some(prev) = self.dab.take() {
            self.loaded = Some(prev.loaded_after(&self.tally));
        }
        self.tally = MixTally::default();
    }
}

#[cfg(test)]
mod tests {
    use super::super::effects::mix_effect;
    use super::*;

    #[test]
    fn the_default_does_not_mix_and_is_valid() {
        let m = ColorMix::default();
        assert!(!m.is_active());
        assert!(m.is_default());
        assert!(m.validate().is_ok());
        assert!(!m.wants_composite());
        assert_eq!(m.pressure_factors(0.3), (1.0, 1.0));
    }

    #[test]
    fn ranges_are_checked() {
        for bad in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
            for field in 0..3 {
                let mut m = ColorMix::default();
                match field {
                    0 => m.paint = bad,
                    1 => m.density = bad,
                    _ => m.stretch = bad,
                }
                assert!(m.validate().is_err(), "{field} {bad}");
            }
        }
        let edge = ColorMix {
            paint: 0.0,
            density: 1.0,
            stretch: 1.0,
            ..ColorMix::default()
        };
        assert!(edge.validate().is_ok());
    }

    #[test]
    fn pressure_factors_use_the_responses_only_when_switched_on() {
        let mut m = ColorMix {
            response_paint: PressureResponse::new(0.25, vec![]).unwrap(),
            response_density: PressureResponse::new(0.5, vec![]).unwrap(),
            ..ColorMix::default()
        };
        assert_eq!(m.pressure_factors(0.0), (1.0, 1.0));
        m.pressure_paint = true;
        assert_eq!(m.pressure_factors(0.0), (0.25, 1.0));
        m.pressure_density = true;
        assert_eq!(m.pressure_factors(0.0), (0.25, 0.5));
        assert_eq!(m.pressure_factors(1.0), (1.0, 1.0));
    }

    #[test]
    fn mixing_weights_by_alpha_and_keeps_the_brush_alpha() {
        let red = [255.0, 0.0, 0.0, 255.0];
        let blue = [0.0, 0.0, 255.0, 255.0];
        // 量 0.5 の不透明どうし: 真ん中
        assert_eq!(mix_colors(blue, red, 0.5), Some([127.5, 0.0, 127.5, 255.0]));
        // 量 1 は荷そのもの、量 0 は下地そのもの（アルファは荷のまま）
        assert_eq!(mix_colors(blue, red, 1.0), Some([255.0, 0.0, 0.0, 255.0]));
        assert_eq!(mix_colors(blue, red, 0.0), Some([0.0, 0.0, 255.0, 255.0]));
        // 透明な下地の色は混ざらない（量によらず荷の色）
        let clear = [0.0, 255.0, 0.0, 0.0];
        assert_eq!(mix_colors(clear, red, 0.2), Some([255.0, 0.0, 0.0, 255.0]));
        // 半透明の荷は荷の重みが半分になるが、アルファは荷のまま
        let faint = [255.0, 0.0, 0.0, 51.0];
        let m = mix_colors(blue, faint, 0.5).unwrap();
        assert_eq!(m[3], 51.0);
        assert!(m[2] > m[0], "下地の重みの方が大きい {m:?}");
        // 量 0 で透明な下地は何も混ぜる色が無い
        assert_eq!(mix_colors(clear, red, 0.0), None);
    }

    #[test]
    fn the_mixed_pixel_matches_the_effect_mix_on_opaque_inputs() {
        // 効果のブラシの mix_effect（アルファ重みの補間）と、不透明どうしでは同じ重み・同じ丸め
        for (u, c, amount) in [
            (
                Rgba8::new(10, 200, 30, 255),
                Rgba8::new(250, 5, 90, 255),
                0.37,
            ),
            (
                Rgba8::new(0, 0, 0, 255),
                Rgba8::new(255, 255, 255, 255),
                0.5,
            ),
            (
                Rgba8::new(33, 66, 99, 255),
                Rgba8::new(99, 66, 33, 255),
                0.9,
            ),
        ] {
            let dab = MixDab {
                mode: MixMode::Mix,
                carry: straight(c),
                paint: amount,
                density: 1.0,
                shift: (0, 0),
                blur: 0,
            };
            let ours = dab.pixel_color(u).unwrap();
            let theirs = mix_effect(u, c, amount);
            assert_eq!(ours, theirs, "{u:?} {c:?} {amount}");
        }
    }

    #[test]
    fn the_carry_drags_the_picked_colour_by_the_stretch() {
        let draw = Rgba8::new(255, 0, 0, 255);
        // 最初の打点・色延び 0 は描く色
        assert_eq!(next_carry(None, draw, 0.7), [255.0, 0.0, 0.0, 255.0]);
        let loaded = Some([0.0, 0.0, 200.0, 255.0]);
        assert_eq!(next_carry(loaded, draw, 0.0), [255.0, 0.0, 0.0, 255.0]);
        // 色延び 1 は前の荷のまま（アルファは描く色）
        assert_eq!(next_carry(loaded, draw, 1.0), [0.0, 0.0, 200.0, 255.0]);
        assert_eq!(next_carry(loaded, draw, 0.5), [127.5, 0.0, 100.0, 255.0]);
    }

    #[test]
    fn the_load_becomes_the_mixed_average_and_stays_when_nothing_is_picked() {
        let dab = MixDab {
            mode: MixMode::Mix,
            carry: [255.0, 0.0, 0.0, 255.0],
            paint: 0.5,
            density: 1.0,
            shift: (0, 0),
            blur: 0,
        };
        // 何も拾えない（下地が透明）
        let mut empty = MixTally::default();
        empty.add(1.0, Rgba8::new(9, 9, 9, 0));
        empty.add(1.0, Rgba8::new(9, 9, 9, 0));
        assert_eq!(dab.loaded_after(&empty), dab.carry);
        assert_eq!(dab.loaded_after(&MixTally::default()), dab.carry);
        // 青い不透明な下地: 荷 = 赤と青の半々
        let mut blue = MixTally::default();
        blue.add(1.0, Rgba8::new(0, 0, 255, 255));
        blue.add(0.5, Rgba8::new(0, 0, 255, 255));
        assert_eq!(dab.loaded_after(&blue), [127.5, 0.0, 127.5, 255.0]);
        // 被覆の半分が透明なら、拾う重みも半分（青の重み 0.25、赤の重み 0.5）
        let mut half = MixTally::default();
        half.add(1.0, Rgba8::new(0, 0, 255, 255));
        half.add(1.0, Rgba8::new(0, 0, 0, 0));
        let l = dab.loaded_after(&half);
        assert!((l[0] - 255.0 * 0.5 / 0.75).abs() < 1e-9, "{l:?}");
        assert!((l[2] - 255.0 * 0.25 / 0.75).abs() < 1e-9, "{l:?}");
    }

    #[test]
    fn tallies_merge_in_order_like_a_per_tile_sum() {
        let mut a = MixTally::default();
        a.add(0.3, Rgba8::new(10, 20, 30, 200));
        let mut b = MixTally::default();
        b.add(0.7, Rgba8::new(40, 50, 60, 255));
        let mut total = MixTally::default();
        total.merge(a);
        total.merge(b);
        assert_eq!(total.cover, 0.3 + 0.7);
        assert_eq!(total.weight, 0.3 * (200.0 / 255.0) + 0.7);
    }

    #[test]
    fn settle_folds_the_dab_into_the_load_and_clears_the_tally() {
        let mut run = MixRun::default();
        let dab = MixDab {
            mode: MixMode::Mix,
            carry: [255.0, 0.0, 0.0, 255.0],
            paint: 0.0,
            density: 1.0,
            shift: (0, 0),
            blur: 0,
        };
        run.dab = Some(dab);
        run.tally.add(1.0, Rgba8::new(0, 255, 0, 255));
        run.settle();
        assert_eq!(run.loaded, Some([0.0, 255.0, 0.0, 255.0]));
        assert_eq!(run.tally, MixTally::default());
        assert!(run.dab.is_none());
        // 打点が無ければ荷はそのまま
        run.settle();
        assert_eq!(run.loaded, Some([0.0, 255.0, 0.0, 255.0]));
    }
}
