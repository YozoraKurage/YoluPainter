//! 散布効果（CLIP STUDIO の「散布効果」に当たる）: 描点 1 つに、ブラシの届く円の中へ先端の粒子を散らして置く。
//!
//! 散布が入ったブラシの描点は、[`Jitter::scatter`]・[`Jitter::count`] の代わりに、この粒子の道を通る（散布の間、その 2 つは使わない）。
//! 粒子 1 つは 1 つのダブで、不透明度・流量・硬さ・質感・色・筆先の選びは、今のダブと同じ [`Brush::next_dab`] から取り、位置・大きさ・角度
//! だけを [`SprayRun::place`] が差し替える。
//!
//! - 数 = `round(密度 × 密度の影響元の係数 ×` [`DENSITY_PER_STAMP`]`)`（0 なら置かない）。
//! - 位置 = 描点の中心 + 半径 R（そのダブの半径）の円の中の点。角度は一様、中心からの距離は [`spray_distance`]。
//! - 直径 = 粒子サイズ × 粒子サイズの影響元の係数（ブラシサイズに連動なら、さらに そのダブの直径 ÷ 設定の直径）。
//! - 向き = 粒子の向き + 向きの選び（[`SprayDirection`]）+ ランダム（強さ × 一様 ±180°）。
//!
//! 乱数は自分の列（ゆらぎ・色・影響元・デュアル・反転・並べ替え・色の変化の列と重ならない）から、描点ごとに密度のランダム（使うとき）、
//! 粒子ごとに 粒子サイズのランダム（使うとき）→ 距離 → 角度 → 向きのランダム（使うとき）の順に引く。2D の描点と 3D の面のダブは
//! 同じ [`SprayRun`] の関数を呼ぶので、同じ設定・同じ入力の点の列なら同じ粒子の列になる（座標の枠は呼び手のもの）。

use std::f64::consts::PI;

use super::*;

/// 散布の乱数の列の種へ混ぜる値。
const SPRAY_STREAM: i32 = 0x1F83_D9AB;

/// 粒子密度 1 あたりの、描点 1 つの粒子の数。
///
/// 実機で測って決めたい物: この換算（密度が間隔あたりか面積あたりか）、散布偏向の式（[`spray_distance`]）、粒子サイズの単位と連動の基準、
/// 粒子の向きの影響元の旗のビット（`.sut` の取り込み側）、ブラシの半径が 0 の描点（筆圧 0・入り抜きの端・大きさのゆらぎで 0）に粒子を
/// 置くか（今は置かない。半径 0 のダブは [`Brush::next_dab`] が先に None を返す）。
pub const DENSITY_PER_STAMP: f64 = 1.0;

/// 粒子サイズの上限（文書の画素の直径）。
pub const MAX_SPRAY_SIZE: f64 = 10000.0;

/// 散布効果の設定。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spray {
    /// 粒子の直径（文書の画素。0 より大きく [`MAX_SPRAY_SIZE`] まで）。影響元は [`SprayItem::Size`]。
    pub size: f64,
    /// ブラシサイズに連動（粒子の直径に、そのダブの直径 ÷ 設定のブラシの直径を掛ける）。
    pub link_size: bool,
    /// 粒子密度（1〜100）。影響元は [`SprayItem::Density`]。
    pub density: f64,
    /// 散布偏向（−1〜1。+1 で中心に寄り、−1 で縁に寄る。0 は円の中に一様）。
    pub bias: f64,
    /// 粒子の向き（度。−360〜360）。
    pub angle: f64,
    /// 粒子の向きに足す向き。
    pub direction: SprayDirection,
    /// 粒子の向きのランダムの強さ（0〜1。1 で ±180°。None は無し）。ほかの向きと組める。
    pub random_angle: Option<f64>,
}

impl Default for Spray {
    fn default() -> Self {
        Spray {
            size: 10.0,
            link_size: false,
            density: 5.0,
            bias: 0.0,
            angle: 0.0,
            direction: SprayDirection::None,
            random_angle: None,
        }
    }
}

/// 粒子の向きに足す向き。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SprayDirection {
    /// 足さない（粒子の向きのまま）。
    #[default]
    None,
    /// 線の向き。
    Line,
    /// 散布全体の向き（ブラシ先端の角度の設定）。
    Whole,
    /// 粒子の位置から描点の中心への向き。
    Center,
}

impl SprayDirection {
    /// すべての向き（並びは保存の順）。
    pub const ALL: [SprayDirection; 4] = [
        SprayDirection::None,
        SprayDirection::Line,
        SprayDirection::Whole,
        SprayDirection::Center,
    ];

    /// 保存に使う名前。
    pub fn id(self) -> &'static str {
        match self {
            SprayDirection::None => "none",
            SprayDirection::Line => "line",
            SprayDirection::Whole => "whole",
            SprayDirection::Center => "center",
        }
    }
}

impl Spray {
    /// 範囲を確かめる（有限で範囲の中）。
    pub(crate) fn validate(&self) -> Result<(), CoreError> {
        let mut checks = vec![
            (self.density, 1.0, 100.0),
            (self.bias, -1.0, 1.0),
            (self.angle, -360.0, 360.0),
        ];
        if let Some(r) = self.random_angle {
            checks.push((r, 0.0, 1.0));
        }
        require_finite(self.size, Reason::Spray)?;
        if !(self.size > 0.0 && self.size <= MAX_SPRAY_SIZE) {
            return Err(CoreError::InvalidArgument(Reason::SprayRange));
        }
        for (v, lo, hi) in checks {
            require_finite(v, Reason::Spray)?;
            if !(lo..=hi).contains(&v) {
                return Err(CoreError::InvalidArgument(Reason::SprayRange));
            }
        }
        Ok(())
    }
}

/// 一様乱数 u（[0, 1)）から、粒子の中心からの距離（R を 1 とした割合）。`u^(0.5 × 4^bias)`: bias 0 で円の中に一様（面積で一様）、
/// +1 で中心に寄り（指数 2）、−1 で縁に寄る（指数 1/8）。
pub(crate) fn spray_distance(u: f64, bias: f64) -> f64 {
    u.powf(0.5 * 4f64.powf(bias))
}

/// 粒子の置き方を呼び手の枠へ直す値（ダブの半径 R、つまり [`DabPlan::radius`] 1 あたり。位置と大きさの単位が違う呼び手のため）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct SprayFrame {
    /// [`DabPlan::radius`] 1 あたりの、届く半径（位置の単位）。
    pub reach: f64,
    /// [`DabPlan::radius`] 1 あたりの、そのダブの直径 ÷ 設定のブラシの直径（ブラシサイズに連動の係数）。
    pub ratio: f64,
    /// 文書の画素 1 の半径の、[`DabPlan::radius`] の単位での大きさ。
    pub radius: f64,
    /// 粒子の半径の下限（[`DabPlan::radius`] の単位。最低 1 ピクセル。0 は下限なし）。
    pub min_radius: f64,
}

impl SprayFrame {
    /// 位置も大きさも文書の画素の枠（2D の描点）。`radius` は設定のブラシの半径、`min_radius` は粒子の半径の下限。
    pub(crate) fn document(radius: f64, min_radius: f64) -> SprayFrame {
        SprayFrame {
            reach: 1.0,
            ratio: 1.0 / radius,
            radius: 1.0,
            min_radius,
        }
    }
}

/// ストロークの散布の進み（2D のストロークと 3D の面のストロークが 1 つずつ持つ）。
#[derive(Clone, Debug, Default)]
pub(crate) struct SprayRun {
    /// 散布の乱数の列（散布が入ったブラシだけ）。
    random: Option<NetRandom>,
}

impl SprayRun {
    pub(crate) fn new(brush: &Brush) -> SprayRun {
        SprayRun {
            random: brush
                .spray
                .is_some()
                .then(|| NetRandom::new(brush.seed ^ SPRAY_STREAM)),
        }
    }

    /// 描点 1 つに置くダブの数（散布が無ければ [`Jitter::count`]、あれば粒子の数）。
    pub(crate) fn count(&mut self, brush: &Brush, c: &StampControls) -> u32 {
        let (Some(s), Some(r)) = (&brush.spray, self.random.as_mut()) else {
            return brush.jitter.count;
        };
        let inf = &brush.influences.spray_density;
        let mut f = c.spray[1];
        if inf.random.on {
            f *= inf.random_factor(r.next_double());
        }
        let n = (s.density * f * DENSITY_PER_STAMP).round();
        if n > 0.0 {
            n as u32
        } else {
            0
        }
    }

    /// 粒子 1 つの置き方: `plan`（今のダブの置き方）の位置・大きさ・角度を、粒子のものへ差し替える。散布が無ければ `plan` のまま。
    /// 大きさが 0 以下の粒子は None。
    pub(crate) fn place<'a>(
        &mut self,
        brush: &Brush,
        c: &StampControls,
        dab: &PendingDab,
        plan: DabPlan<'a>,
        frame: SprayFrame,
    ) -> Option<DabPlan<'a>> {
        let (Some(s), Some(r)) = (&brush.spray, self.random.as_mut()) else {
            return Some(plan);
        };
        let inf = &brush.influences.spray_size;
        let mut f = c.spray[0];
        if inf.random.on {
            f *= inf.random_factor(r.next_double());
        }
        let distance = plan.radius * frame.reach * spray_distance(r.next_double(), s.bias);
        let theta = r.next_double() * 2.0 * PI;
        let (cos, sin) = cos_sin(theta);
        let (x, y) = (dab.x + distance * cos, dab.y + distance * sin);
        let mut radius = s.size * 0.5 * f * frame.radius;
        if s.link_size {
            radius *= plan.radius * frame.ratio;
        }
        if frame.min_radius > 0.0 {
            radius = f64_max(radius, frame.min_radius);
        }
        let turn = match s.direction {
            SprayDirection::None => 0.0,
            SprayDirection::Line => dab.direction,
            SprayDirection::Whole => brush.tip.angle.to_radians(),
            SprayDirection::Center => (dab.y - y).atan2(dab.x - x),
        };
        let mut angle = s.angle.to_radians() + turn;
        if let Some(k) = s.random_angle {
            angle += (r.next_double() * 2.0 - 1.0) * PI * k;
        }
        if radius <= 0.0 {
            return None;
        }
        Some(DabPlan {
            x,
            y,
            radius,
            angle,
            ..plan
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(direction: f64) -> PendingDab {
        PendingDab {
            x: 100.0,
            y: 50.0,
            pressure: 1.0,
            arc: 0.0,
            direction,
            tilt_x: 0.0,
            tilt_y: 0.0,
            rotation: 0.0,
            speed: 0.0,
        }
    }

    /// 半径 8 の丸いブラシに散布を入れたもの。
    fn sprayed(spray: Spray) -> Brush {
        let mut b = Brush::simple(
            BrushSettings {
                radius: 8.0,
                ..BrushSettings::default()
            },
            PressureSwitches::NONE,
        );
        b.spray = Some(spray);
        b.seed = 3;
        b
    }

    fn pressure_use() -> InputUse {
        InputUse {
            on: true,
            response: InputResponse::new(0.0, vec![]).unwrap(),
        }
    }

    /// 描点を `stamps` 個置いたときの粒子の列（2D の枠。ダブの半径はブラシの半径）。
    fn particles(brush: &Brush, dab: &PendingDab, stamps: usize) -> Vec<(f64, f64, f64, f64)> {
        let mut random = NetRandom::new(brush.seed);
        let mut tips = TipRun::new(brush);
        let frame = SprayFrame::document(brush.base.radius, 0.0);
        let mut out = Vec::new();
        for i in 0..stamps {
            let c = brush.stamp_controls(dab, i as u64, 1.0, None);
            for _ in 0..tips.spray.count(brush, &c) {
                let Some(plan) = brush.next_dab(
                    &c,
                    dab,
                    brush.base.radius * c.size_pressure,
                    0.0,
                    brush.base.radius,
                    &mut random,
                    &mut tips,
                ) else {
                    continue;
                };
                if let Some(p) = tips.spray.place(brush, &c, dab, plan, frame) {
                    out.push((p.x, p.y, p.radius, p.angle));
                }
            }
        }
        out
    }

    fn mean_distance(brush: &Brush) -> f64 {
        let dab = pending(0.0);
        let p = particles(brush, &dab, 400);
        p.iter()
            .map(|(x, y, ..)| (x - dab.x).hypot(y - dab.y))
            .sum::<f64>()
            / p.len() as f64
    }

    #[test]
    fn the_count_follows_the_density_and_its_influence_and_zero_places_nothing() {
        let b = sprayed(Spray {
            density: 7.0,
            ..Spray::default()
        });
        let dab = pending(0.0);
        assert_eq!(particles(&b, &dab, 10).len(), 70);
        let mut half = b.clone();
        half.influences.spray_density.pressure = pressure_use();
        let soft = PendingDab {
            pressure: 0.5,
            ..dab
        };
        // 7 × 0.5 = 3.5 → 4（四捨五入）
        assert_eq!(particles(&half, &soft, 10).len(), 40);
        let none = PendingDab {
            pressure: 0.0,
            ..dab
        };
        assert!(particles(&half, &none, 10).is_empty());
        // 散布の無いブラシは数のゆらぎの数のまま
        let mut plain = b.clone();
        plain.spray = None;
        plain.jitter.count = 3;
        let c = plain.stamp_controls(&dab, 0, 1.0, None);
        assert_eq!(TipRun::new(&plain).spray.count(&plain, &c), 3);
    }

    #[test]
    fn particles_stay_in_the_circle_and_the_bias_pulls_them_in_or_out() {
        for bias in [-1.0, 0.0, 1.0] {
            let b = sprayed(Spray {
                density: 20.0,
                bias,
                ..Spray::default()
            });
            let dab = pending(0.0);
            for (x, y, ..) in particles(&b, &dab, 50) {
                assert!((x - dab.x).hypot(y - dab.y) <= 8.0 + 1e-9, "偏向 {bias}");
            }
        }
        let at = |bias| {
            mean_distance(&sprayed(Spray {
                bias,
                ..Spray::default()
            }))
        };
        let (inner, even, outer) = (at(1.0), at(0.0), at(-1.0));
        // 一様（指数 0.5）の平均は 2R/3、指数 2 は R/3、指数 1/8 は R/9 × 8 = 8R/9
        assert!((even - 8.0 * 2.0 / 3.0).abs() < 0.3, "{even}");
        assert!((inner - 8.0 / 3.0).abs() < 0.3, "{inner}");
        assert!((outer - 8.0 * 8.0 / 9.0).abs() < 0.3, "{outer}");
        assert_eq!(spray_distance(0.25, 0.0), 0.5);
    }

    #[test]
    fn the_particle_size_follows_its_influence_and_the_brush_size_when_linked() {
        let b = sprayed(Spray {
            size: 6.0,
            ..Spray::default()
        });
        let dab = pending(0.0);
        assert!(particles(&b, &dab, 3).iter().all(|p| p.2 == 3.0));
        let mut soft = b.clone();
        soft.influences.spray_size.pressure = pressure_use();
        let half = PendingDab {
            pressure: 0.5,
            ..dab
        };
        assert!(particles(&soft, &half, 3).iter().all(|p| p.2 == 1.5));
        // 連動: 筆圧で半分になったブラシの粒子は半分
        let mut linked = b.clone();
        linked.spray.as_mut().unwrap().link_size = true;
        linked.influences.size.pressure = pressure_use();
        assert!(particles(&linked, &dab, 3).iter().all(|p| p.2 == 3.0));
        assert!(particles(&linked, &half, 3).iter().all(|p| p.2 == 1.5));
        // 連動していなければ、ブラシの大きさが変わっても粒子は同じ
        let mut unlinked = linked.clone();
        unlinked.spray.as_mut().unwrap().link_size = false;
        assert!(particles(&unlinked, &half, 3).iter().all(|p| p.2 == 3.0));
        // 最低 1 ピクセルの下限
        let tiny = sprayed(Spray {
            size: 0.2,
            ..Spray::default()
        });
        let mut random = NetRandom::new(0);
        let mut tips = TipRun::new(&tiny);
        let c = tiny.stamp_controls(&dab, 0, 1.0, None);
        let plan = tiny
            .next_dab(&c, &dab, 8.0, 0.5, 8.0, &mut random, &mut tips)
            .unwrap();
        let p = tips
            .spray
            .place(&tiny, &c, &dab, plan, SprayFrame::document(8.0, 0.5))
            .unwrap();
        assert_eq!(p.radius, 0.5);
    }

    #[test]
    fn a_stamp_with_a_zero_brush_radius_places_no_particles_until_the_radius_returns() {
        let mut b = sprayed(Spray::default());
        b.influences.size.pressure = pressure_use();
        let dab = pending(0.0);
        let none = PendingDab {
            pressure: 0.0,
            ..dab
        };
        assert!(particles(&b, &none, 5).is_empty());
        assert_eq!(particles(&b, &dab, 5).len(), 25);
    }

    #[test]
    fn the_particle_direction_follows_the_choice_and_the_random_strength() {
        let line = 0.7;
        let dab = pending(line);
        let angles = |direction, random_angle| {
            let mut b = sprayed(Spray {
                angle: 30.0,
                direction,
                random_angle,
                ..Spray::default()
            });
            b.tip.angle = 45.0;
            particles(&b, &dab, 4)
        };
        let base = 30f64.to_radians();
        assert!(angles(SprayDirection::None, None)
            .iter()
            .all(|p| p.3 == base));
        assert!(angles(SprayDirection::Line, None)
            .iter()
            .all(|p| (p.3 - base - line).abs() < 1e-12));
        assert!(angles(SprayDirection::Whole, None).iter().all(|p| (p.3
            - base
            - 45f64.to_radians())
        .abs()
            < 1e-12));
        for (x, y, _, a) in angles(SprayDirection::Center, None) {
            let toward = (dab.y - y).atan2(dab.x - x);
            assert!((a - base - toward).abs() < 1e-12);
        }
        let spread = angles(SprayDirection::Line, Some(0.5));
        let turns: Vec<f64> = spread.iter().map(|p| p.3 - base - line).collect();
        assert!(turns.iter().all(|t| t.abs() <= PI * 0.5));
        assert!(turns.iter().any(|t| t.abs() > PI * 0.25), "{turns:?}");
        // ランダムを足しても、最初の粒子の位置は同じ（向きの乱数は粒子の最後に引く）
        let (a, b) = (angles(SprayDirection::Line, None), spread);
        assert_eq!(a.len(), b.len());
        assert!(a
            .iter()
            .zip(&b)
            .take(1)
            .all(|(p, q)| p.0 == q.0 && p.1 == q.1));
    }

    #[test]
    fn the_same_seed_makes_the_same_particles_and_another_seed_others() {
        let b = sprayed(Spray::default());
        let dab = pending(0.0);
        assert_eq!(particles(&b, &dab, 8), particles(&b, &dab, 8));
        let mut other = b.clone();
        other.seed = 4;
        assert_ne!(particles(&b, &dab, 8), particles(&other, &dab, 8));
    }

    #[test]
    fn out_of_range_or_non_finite_values_are_refused() {
        let ok = Spray::default();
        assert!(ok.validate().is_ok());
        for bad in [
            Spray { size: 0.0, ..ok },
            Spray {
                size: MAX_SPRAY_SIZE + 1.0,
                ..ok
            },
            Spray {
                size: f64::NAN,
                ..ok
            },
            Spray { density: 0.5, ..ok },
            Spray {
                density: 101.0,
                ..ok
            },
            Spray { bias: 1.5, ..ok },
            Spray {
                bias: f64::INFINITY,
                ..ok
            },
            Spray { angle: 400.0, ..ok },
            Spray {
                random_angle: Some(1.5),
                ..ok
            },
            Spray {
                random_angle: Some(f64::NAN),
                ..ok
            },
        ] {
            let b = Brush {
                spray: Some(bad),
                ..Brush::default()
            };
            assert!(b.validate().is_err(), "{bad:?}");
        }
    }
}
