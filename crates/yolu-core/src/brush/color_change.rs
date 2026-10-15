//! 色の変化（CLIP STUDIO の「色の変化」に当たる。先端の色相・彩度・明度、サブ描画色混合率、ストロークごとの変化）。
//!
//! Photoshop の形の色のゆらぎ（[`ColorDynamics::next`]）の結果の色に、ダブごとに次の順で当てる:
//! 1. サブ描画色へ `t` だけ寄せる（RGB を同じ割合で混ぜる。アルファは描画色のまま）。
//! 2. HSV で色相（周）・彩度・明度を足す（彩度と明度は 0〜1 に丸める。色相は 1 周で回す）。
//!
//! 変化量は、先端は 値 × その項目の影響元の係数の積（[`ColorItem`]。どの入力も使わなければ値のまま一定）、ストロークごとは始めに 1 回だけ
//! 引く（色相・彩度・明度は 値 × (2u − 1)、サブ描画色混合率は 値 × u）。先端とストロークごとの変化は足し算で、サブ描画色混合率は足してから
//! 0〜1 に丸める。変化対象が「サブ」だけのとき、先端とストロークごとの色相・彩度・明度・ストロークごとのサブは当てない
//! （このアプリはいつもメイン描画色で描く）。サブ描画色混合率（先端）は変化対象によらず効く。
//!
//! 乱数は 2 本の列を持つ（どちらも Photoshop の形の色・影響元・デュアルの列と重ならない）: ダブごとの列は、ランダムを使う色の項目ごとに項目の順に
//! 1 つずつ（値が 0 の項目も引く）、ストロークごとの列は始めに色相・彩度・明度・サブの順に 4 つ。2D の描点と 3D の面のダブは同じ
//! [`StrokeState::next_dab_color`] を呼ぶので、同じ設定・同じ入力の点の列なら同じ色の列になる。

use super::dynamics::{hsv_to_rgb, mix, rgb_to_hsv};
use super::*;
use crate::math::to_byte;

/// ダブごとの色の変化の乱数の列の種へ混ぜる値。
const COLOR_CHANGE_STREAM: i32 = 0x3C6E_F372;
/// ストロークごとの色の変化の乱数の列の種へ混ぜる値。
const COLOR_STROKE_STREAM: i32 = 0x510E_527F;

/// 色の変化量（HSV の足し算とサブ描画色混合率）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Shift {
    /// 色相（周。1 で 360 度）。
    hue: f64,
    saturation: f64,
    value: f64,
    /// サブ描画色混合率（足した後に 0〜1 へ丸める）。
    sub: f64,
}

/// ストロークの色の変化の進み。
#[derive(Clone, Debug, Default)]
pub(crate) struct ColorChange {
    /// 色の変化の前のストロークの色（Photoshop の形の色のストロークの色）。
    base: Rgba8,
    /// ダブごとに Photoshop の形の次の色を引くか。
    ps_per_dab: bool,
    /// 色の変化が効くか（効かなければ、色はそのまま返す）。
    active: bool,
    /// ダブごとに先端の変化を当てるか（先端の変化が入っている、またはサブ描画色混合率が影響元で動く）。
    per_dab: bool,
    /// サブ描画色混合率が影響元で動くか（動くなら、ダブごとの変化量へ足す。動かなければ一定でストロークごとの変化量に入れる）。
    sub_varies: bool,
    /// ストロークの始めに決めた変化量（ストロークごとの変化と、一定のサブ描画色混合率）。
    stroke: Shift,
    /// ダブごとの乱数の列（ダブごとの色の変化で、ランダムを使う項目があるときだけ）。
    random: Option<NetRandom>,
}

impl ColorChange {
    /// ストロークの始め。`base` は Photoshop の形の色のストロークの色、`ps_per_dab` はダブごとにその色を引くか、`painting` は色を塗るか
    /// （消しゴム・効果のブラシは色を変えない）。
    pub(crate) fn start(
        brush: &Brush,
        base: Rgba8,
        ps_per_dab: bool,
        painting: bool,
    ) -> ColorChange {
        let d = &brush.color;
        let mut run = ColorChange {
            base,
            ps_per_dab,
            ..ColorChange::default()
        };
        if !painting || !d.change_active() {
            return run;
        }
        run.active = true;
        run.sub_varies = d.sub_mix > 0.0 && brush.influences.color_sub_mix.is_on();
        run.per_dab = d.tip_active() || run.sub_varies;
        if let (true, Some(s)) = (d.stroke_active(), &d.stroke) {
            let mut r = NetRandom::new(brush.seed ^ COLOR_STROKE_STREAM);
            let mut two_sided = |v: f64| v * (r.next_double() * 2.0 - 1.0);
            run.stroke.hue = two_sided(s.hue) / 360.0;
            run.stroke.saturation = two_sided(s.saturation);
            run.stroke.value = two_sided(s.value);
            run.stroke.sub = s.sub_mix * r.next_double();
        }
        if !run.sub_varies {
            run.stroke.sub += d.sub_mix;
        }
        if run.per_dab && brush.influences.color_uses_random() {
            run.random = Some(NetRandom::new(brush.seed ^ COLOR_CHANGE_STREAM));
        }
        run
    }

    /// ダブごとの色が要るか（先端の変化・動くサブ描画色混合率）。
    pub(crate) fn per_dab(&self) -> bool {
        self.per_dab
    }

    /// ストロークの色（ダブごとの変化を当てない。パスの塗りなど、ダブを始めない塗りが使う）。
    pub(crate) fn stroke_color(&self, brush: &Brush) -> Rgba8 {
        if self.active {
            shift_color(self.base, brush.color.secondary, self.stroke)
        } else {
            self.base
        }
    }

    /// 今のダブの変化量。ランダムを使う項目は、項目の順に乱数を 1 つずつ引く。
    fn dab_shift(&mut self, brush: &Brush, factors: &[f64; 4]) -> Shift {
        let mut f = *factors;
        if let Some(r) = self.random.as_mut() {
            for (slot, item) in f.iter_mut().zip(ColorItem::ALL) {
                let influence = brush.influences.color(item);
                if influence.random.on {
                    *slot *= influence.random_factor(r.next_double());
                }
            }
        }
        let d = &brush.color;
        let mut shift = self.stroke;
        if let (true, Some(t)) = (d.tip_active(), &d.tip) {
            shift.hue += t.hue / 360.0 * f[0];
            shift.saturation += t.saturation * f[1];
            shift.value += t.value * f[2];
        }
        if self.sub_varies {
            shift.sub += d.sub_mix * f[3];
        }
        shift
    }
}

/// 色 c に、サブ描画色 secondary への寄せと HSV の足し算を当てる（変化が 0 ならそのまま返す）。
fn shift_color(c: Rgba8, secondary: Rgba8, shift: Shift) -> Rgba8 {
    let t = crate::math::clamp01(shift.sub);
    let mut c = c;
    if t > 0.0 {
        c = Rgba8::new(
            mix(c.r, secondary.r, t),
            mix(c.g, secondary.g, t),
            mix(c.b, secondary.b, t),
            c.a,
        );
    }
    if shift.hue == 0.0 && shift.saturation == 0.0 && shift.value == 0.0 {
        return c;
    }
    let (mut h, s, v) = rgb_to_hsv(c.r as f64 / 255.0, c.g as f64 / 255.0, c.b as f64 / 255.0);
    h += shift.hue;
    h -= h.floor();
    let (r, g, b) = hsv_to_rgb(
        h,
        crate::math::clamp01(s + shift.saturation),
        crate::math::clamp01(v + shift.value),
    );
    Rgba8::new(to_byte(r), to_byte(g), to_byte(b), c.a)
}

impl StrokeState {
    /// 次のダブの色。Photoshop の形の色（ダブごとなら次の色、でなければストロークの色）に、色の変化を当てる。`factors` は描点の色の項目の
    /// 筆圧・傾き・速度の係数の積（[`StampControls::color`]）。2D の描点のダブのループと 3D の面のダブ（`begin_surface_dab`）が呼ぶ。
    pub(crate) fn next_dab_color(&mut self, factors: &[f64; 4]) -> Rgba8 {
        let brush = self.brush.clone();
        let mut base = self.color_change.base;
        if self.color_change.ps_per_dab {
            let r = self.color_random.as_mut().expect("色の乱数");
            base = brush.color.next(brush.base.color, r);
        }
        let change = &mut self.color_change;
        if !change.active {
            return base;
        }
        let shift = if change.per_dab {
            change.dab_shift(&brush, factors)
        } else {
            change.stroke
        };
        shift_color(base, brush.color.secondary, shift)
    }
}
