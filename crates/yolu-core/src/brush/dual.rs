//! デュアルブラシ（2 つ目の筆先）の、2 つ目のダブごとの見え方: 影響元（[`DualItem`]。サイズ・濃度・間隔・厚さ）と向きの足し
//! （ペンの向き・ペン軸の回転・線の方向・ランダム）。
//!
//! - 入力（筆圧・傾き・速度）は、その 2 つ目のダブを道の上に並べた所の値（区間の両端の間で補間したもの）。ランダムは自分の列
//!   （[`DUAL_INFLUENCE_STREAM`]。ゆらぎ・色・影響元・デュアルの散布・反転・並べ替え・散布効果・色の変化の列と重ならない）から、2 つ目の
//!   ダブを並べるごとに サイズ → 濃度 → 間隔 → 厚さ（ランダムを使う項目だけ）→ 向きのランダム（強さが 0 より大きいとき）の順に引く。
//! - 係数は主の項目と同じ式（使っている入力の応えの積。[`Influence::factor`]）。サイズは半径、濃度は [`DualBrush::flow`]、間隔は
//!   次の 2 つ目のダブまでの間隔（下限は設定の間隔の [`MIN_SPACING_FACTOR`] 倍）、厚さは真円率（下限 0.01）に掛ける。
//! - 向きは主の筆先と同じ規則で、設定の角度に 線の方向 → ペンが倒れた向き → ペン軸の回転 → ランダム（強さ × ±180°）を足す。
//!
//! 2D のストロークと 3D の面のストロークは、2 つ目のダブを同じ順に並べ、同じ [`DualRun::look`] を呼ぶので、同じ設定・同じ入力の点の列なら
//! 同じ見え方の列になる。影響元も向きの足しも使わない 2 つ目の筆先は、何も引かず、いつも [`DualLook::PLAIN`]（今までの画素のまま）。

use std::f64::consts::PI;

use super::*;

/// デュアルの影響元の乱数の列の種へ混ぜる値。
pub(crate) const DUAL_INFLUENCE_STREAM: i32 = 0x6A09_E667;

/// 間隔の影響元の係数の下限（係数 0 で同じ所に置き続けないため）。
pub(crate) const MIN_SPACING_FACTOR: f64 = 0.01;

/// 2 つ目のダブ 1 つの見え方（係数は使わなければ 1、足す向きは 0）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DualLook {
    pub size: f64,
    pub flow: f64,
    /// 次の 2 つ目のダブまでの間隔の係数。
    pub spacing: f64,
    /// 真円率の係数。
    pub roundness: f64,
    /// 設定の角度に足す向き（ラジアン、反時計回り）。
    pub turn: f64,
}

impl DualLook {
    /// 何も変えない見え方。
    pub const PLAIN: DualLook = DualLook {
        size: 1.0,
        flow: 1.0,
        spacing: 1.0,
        roundness: 1.0,
        turn: 0.0,
    };
}

impl DualBrush {
    /// 2 つ目のダブごとに見え方が変わるか（影響元か向きの足しを使う）。
    pub(crate) fn varies(&self, influences: &Influences) -> bool {
        influences.dual_is_on()
            || self.angle_tilt
            || self.angle_rotation
            || self.angle_line
            || self.angle_random > 0.0
    }

    /// 2 つ目のダブの半径: 設定の半径 × サイズの係数 × サイズの連動の比（連動を使わなければ掛けない）。
    pub(crate) fn dab_radius(&self, look: &DualLook, link: f64) -> f64 {
        let mut radius = self.radius;
        if look.size != 1.0 {
            radius *= look.size;
        }
        if self.link_size {
            radius *= link;
        }
        radius
    }
}

/// サイズの連動の比 ratio（主の半径 ÷ 設定の半径、最低 1 画素の前）を、主の描点が実際に置く半径に合わせる: 「最低 1 画素」が入っていれば、
/// 主の半径は設定の半径 radius（文書の画素）のうち 0.5 画素より小さくならないので、比も `0.5 / radius` を下回らない。
#[inline]
pub(crate) fn lifted_link(ratio: f64, min_one_pixel: bool, radius: f64) -> f64 {
    if min_one_pixel && radius > 0.0 {
        f64_max(ratio, MIN_ONE_PIXEL_RADIUS / radius)
    } else {
        ratio
    }
}

/// 間隔 base（呼び手の単位）に間隔の係数 factor を掛けた、次の 2 つ目のダブまでの間隔（係数 1 は base のまま）。
#[inline]
pub(crate) fn step(base: f64, factor: f64) -> f64 {
    if factor == 1.0 {
        base
    } else {
        base * f64_max(MIN_SPACING_FACTOR, factor)
    }
}

/// ストロークの 2 つ目のダブの見え方の進み（2D のストロークと 3D の面のストロークが 1 つずつ持つ）。
#[derive(Clone, Debug, Default)]
pub(crate) struct DualRun {
    /// 見え方が変わるか（変わらなければ何も計算しない）。
    varies: bool,
    /// デュアルの影響元の乱数の列（ランダムを使うときだけ）。
    random: Option<NetRandom>,
}

impl DualRun {
    pub(crate) fn new(brush: &Brush) -> DualRun {
        let Some(d) = &brush.dual else {
            return DualRun::default();
        };
        let inf = &brush.influences;
        let random =
            DualItem::ALL.into_iter().any(|i| inf.dual(i).random.on) || d.angle_random > 0.0;
        DualRun {
            varies: d.varies(inf),
            random: random.then(|| NetRandom::new(brush.seed ^ DUAL_INFLUENCE_STREAM)),
        }
    }

    /// 道の上の位置 dab（入力はその所の値）に並べる 2 つ目のダブの見え方。
    pub(crate) fn look(&mut self, brush: &Brush, dab: &PendingDab) -> DualLook {
        let (true, Some(d)) = (self.varies, &brush.dual) else {
            return DualLook::PLAIN;
        };
        let inf = &brush.influences;
        let tilt = if inf.uses(Input::Tilt) || d.angle_tilt {
            pen_tilt::amount(dab.tilt_x, dab.tilt_y)
        } else {
            0.0
        };
        let slow = if inf.uses(Input::Speed) {
            1.0 - clamp01(dab.speed / inf.speed_max)
        } else {
            1.0
        };
        let x = InputValues {
            pressure: dab.pressure,
            upright: 1.0 - tilt,
            slow,
        };
        let mut f = [1.0; 4];
        for (slot, item) in f.iter_mut().zip(DualItem::ALL) {
            let influence = inf.dual(item);
            let u = if influence.random.on {
                self.random.as_mut().map(|r| r.next_double())
            } else {
                None
            };
            if let Some(v) = influence.factor(&x, u) {
                *slot = v;
            }
        }
        let mut turn = 0.0;
        if d.angle_line {
            turn += dab.direction;
        }
        if d.angle_tilt && tilt > 0.0 {
            turn += pen_tilt::azimuth(dab.tilt_x, dab.tilt_y);
        }
        if d.angle_rotation && dab.rotation != 0.0 {
            turn += dab.rotation;
        }
        if d.angle_random > 0.0 {
            if let Some(r) = self.random.as_mut() {
                turn += (r.next_double() * 2.0 - 1.0) * PI * d.angle_random;
            }
        }
        DualLook {
            size: f[0],
            flow: f[1],
            spacing: f[2],
            roundness: f[3],
            turn,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(pressure: f64) -> PendingDab {
        PendingDab {
            x: 0.0,
            y: 0.0,
            pressure,
            arc: 0.0,
            direction: 0.5,
            tilt_x: 0.0,
            tilt_y: 0.0,
            rotation: 0.25,
            speed: 0.0,
        }
    }

    fn dual_brush(dual: DualBrush) -> Brush {
        Brush {
            dual: Some(dual),
            seed: 5,
            ..Brush::default()
        }
    }

    #[test]
    fn a_plain_dual_brush_draws_nothing_and_looks_plain() {
        let b = dual_brush(DualBrush::default());
        let mut run = DualRun::new(&b);
        assert!(run.random.is_none());
        assert_eq!(run.look(&b, &at(0.3)), DualLook::PLAIN);
    }

    #[test]
    fn the_pressure_scales_the_items_it_is_turned_on_for() {
        let mut b = dual_brush(DualBrush::default());
        b.influences.dual_size.pressure.on = true;
        b.influences.dual_thickness.pressure.on = true;
        let mut run = DualRun::new(&b);
        let look = run.look(&b, &at(0.4));
        assert_eq!(
            (look.size, look.flow, look.spacing, look.roundness),
            (0.4, 1.0, 1.0, 0.4)
        );
    }

    #[test]
    fn the_angle_adds_the_line_the_rotation_and_the_random_turn() {
        let mut b = dual_brush(DualBrush {
            angle_line: true,
            angle_rotation: true,
            ..DualBrush::default()
        });
        let mut run = DualRun::new(&b);
        assert!((run.look(&b, &at(1.0)).turn - 0.75).abs() < 1e-15);
        b.dual.as_mut().unwrap().angle_random = 0.5;
        let mut run = DualRun::new(&b);
        let mut r = NetRandom::new(5 ^ DUAL_INFLUENCE_STREAM);
        let want = 0.75 + (r.next_double() * 2.0 - 1.0) * PI * 0.5;
        assert_eq!(run.look(&b, &at(1.0)).turn, want);
    }

    #[test]
    fn the_spacing_factor_has_a_floor() {
        assert_eq!(step(4.0, 1.0), 4.0);
        assert_eq!(step(4.0, 0.5), 2.0);
        assert_eq!(step(4.0, 0.0), 4.0 * MIN_SPACING_FACTOR);
    }
}
