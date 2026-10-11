//! 2D の対称（C# の CanvasSymmetry・BrushStroke.Symmetry）。
//!
//! 文書の画素の座標で、中心のまわりの直交変換（鏡映と回転）の組を作り、ブラシの各ダブを全部の写しへ置く。写しの画素は、画素の
//! 中心を逆に写した元のダブの上の位置で被覆率を測る（筆先の画像を補間し直さない）。写しが重なる画素は大きい方の被覆率で 1 回だけ
//! 塗る。放射状の写しの角度は、四分の一周の倍数なら正確な値（cos・sin を通らない）、それ以外は libm の `cos`・`sin` を通る。
//! C# とのバイト一致を確かめたのは同じ libm（Linux の glibc）の上で、別の libm では 1 ULP ずれ得る。鏡映・四分の一周の倍数
//! だけの組（縦・横・両方、放射状の 2・4）は正確な値だけを通る。
//!
//! 線対称（[`SymmetryMode::Lines`]。Rust 版だけ）は、`count` 本（偶数）の線を中心から等しい角度で引き、最初の線を `angle`（度、+X から
//! 反時計回り）に置く。写しは二面体群の `count` 個で、`count / 2` 枚の鏡（軸の角度 `angle + 180 k / (count / 2)` 度）と `count / 2` 個の回転
//! （`360 k / (count / 2)` 度、k = 0 は恒等）。`Lines(2, 90°)` が縦、`Lines(2, 0°)` が横、`Lines(4, 0°)` が両方と同じ写しの集合になる。
//! 鏡映の係数は cos 2φ・sin 2φ なので、φ が 45° の倍数のときは正確な 0・±1 を使う（放射状の四分の一周と同じ扱い）。
//! ここは文書の画素の上（UV の平面）の対称だけを扱う。3D の面のストロークはダブの画素をまとめて `apply_dab` で塗り、`Brush.symmetry`
//! を見ない。3D の面のストロークに 2D の対称を当てるときは、面のストロークが塗る UV の画素（元と 3D の写し）を、同じ変換で UV の
//! 平面の上に写す（`geometry::uv_symmetry` の `copy_by_canvas`。面のストロークの `SurfaceStrokeOptions::canvas_symmetry`）。
//!
//! ```
//! use yolu_core::{Brush, BrushSettings, CanvasSymmetry, Document, SymmetryMode, glam::DVec2};
//! let mut doc = Document::new(64, 64).unwrap();
//! let layer = doc.add_layer("a").unwrap();
//! let mut brush = Brush::from(BrushSettings { radius: 3.0, ..BrushSettings::default() });
//! brush.symmetry = CanvasSymmetry::new(SymmetryMode::Vertical, DVec2::new(32.0, 32.0), 2).unwrap();
//! let mut stroke = doc.begin_brush_stroke(layer, &brush).unwrap();
//! stroke.add_point(&mut doc, 10.5, 20.5, 1.0, DVec2::ZERO).unwrap();
//! doc.end_stroke(stroke).unwrap();
//! let l = doc.layer(layer).unwrap().surface(yolu_core::Channel::Color).unwrap();
//! assert_eq!(l.pixel(10, 20).unwrap(), l.pixel(53, 20).unwrap()); // 縦の軸 x = 32 で鏡に写る
//! ```

use glam::DVec2;

use crate::error::CoreError;
use crate::math::require_finite;

/// 対称の種類（C# の CanvasSymmetryMode と同じ並び）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum SymmetryMode {
    /// 対称なし。
    #[default]
    None,
    /// 縦の軸（x = 中心）で左右に写す。
    Vertical,
    /// 横の軸（y = 中心）で上下に写す。
    Horizontal,
    /// 縦と横の両方（4 つ）。
    Both,
    /// 中心のまわりに count 個（2〜16）に回す。
    Radial,
    /// 線対称: 中心を通る `count` 本（偶数、2〜16）の線を等しい角度で引き、最初の線の角度が `angle`。写しは `count` 個（鏡と回転が半分ずつ）。
    /// Rust 版だけで、.ylp ではパスの対称の種類 5（正本の版 35）。
    Lines,
}

/// 対称の設定（C# の CanvasSymmetrySettings）。中心はキャンバスの画素の座標（画素の中心は整数 + 0.5）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasSymmetry {
    pub mode: SymmetryMode,
    pub center: DVec2,
    /// 放射状の写しの数（2〜16。ほかの種類でも範囲は確かめる、C# と同じ）。線対称では線の本数（偶数）。
    pub count: u32,
    /// 線対称の最初の線（鏡の軸）の角度。度、+X から反時計回り、−360〜360。線対称のほかの種類では写しに効かない（0 のままにする）。
    pub angle: f64,
}

impl Default for CanvasSymmetry {
    fn default() -> Self {
        CanvasSymmetry {
            mode: SymmetryMode::None,
            center: DVec2::ZERO,
            count: 2,
            angle: 0.0,
        }
    }
}

/// 中心の座標の範囲（C# と同じ）。
const MAX_CENTER: f64 = 10_000_000.0;
/// 線対称の角度の範囲（度）。
pub const MAX_ANGLE: f64 = 360.0;

impl CanvasSymmetry {
    /// 確かめてから作る（`angle` は 0。線対称は [`CanvasSymmetry::lines`]）。
    pub fn new(mode: SymmetryMode, center: DVec2, count: u32) -> Result<Self, CoreError> {
        let s = CanvasSymmetry {
            mode,
            center,
            count,
            angle: 0.0,
        };
        s.validate()?;
        Ok(s)
    }

    /// 線対称を確かめてから作る（`count` は偶数の 2〜16、`angle` は度）。
    pub fn lines(center: DVec2, count: u32, angle: f64) -> Result<Self, CoreError> {
        let s = CanvasSymmetry {
            mode: SymmetryMode::Lines,
            center,
            count,
            angle,
        };
        s.validate()?;
        Ok(s)
    }

    /// 対称が効くか（None 以外）。
    pub fn enabled(&self) -> bool {
        self.mode != SymmetryMode::None
    }

    /// 中心が有限で ±1e7 の中、写しの数が 2〜16（線対称は偶数）、角度が有限で ±360 度の中。
    pub fn validate(&self) -> Result<(), CoreError> {
        require_finite(self.center.x, "symmetry center")?;
        require_finite(self.center.y, "symmetry center")?;
        if self.center.x.abs() > MAX_CENTER || self.center.y.abs() > MAX_CENTER {
            return Err(CoreError::InvalidArgument("対称の中心（±1e7）"));
        }
        if !(2..=16).contains(&self.count) {
            return Err(CoreError::InvalidArgument("放射状の写しの数（2〜16）"));
        }
        if self.mode == SymmetryMode::Lines && !self.count.is_multiple_of(2) {
            return Err(CoreError::InvalidArgument("線対称の線の本数（偶数）"));
        }
        require_finite(self.angle, "symmetry angle")?;
        if self.angle.abs() > MAX_ANGLE {
            return Err(CoreError::InvalidArgument("対称の角度（±360 度）"));
        }
        Ok(())
    }

    /// 写しの変換（最初は恒等）。C# の Transforms と同じ並び・同じ係数（四分の一周の倍数は正確な 0・±1）。
    pub fn transforms(&self) -> Result<Vec<SymmetryTransform>, CoreError> {
        self.validate()?;
        let (cx, cy) = (self.center.x, self.center.y);
        let t = |a, b, c, d| SymmetryTransform { cx, cy, a, b, c, d };
        let mut v = vec![t(1.0, 0.0, 0.0, 1.0)];
        let m = self.mode;
        if m == SymmetryMode::Vertical || m == SymmetryMode::Both {
            v.push(t(-1.0, 0.0, 0.0, 1.0));
        }
        if m == SymmetryMode::Horizontal || m == SymmetryMode::Both {
            v.push(t(1.0, 0.0, 0.0, -1.0));
        }
        if m == SymmetryMode::Both {
            v.push(t(-1.0, 0.0, 0.0, -1.0));
        }
        if m == SymmetryMode::Radial {
            let n = self.count as i64;
            for i in 1..n {
                let (c, s) = turn(i, n);
                v.push(t(c, -s, s, c));
            }
        }
        if m == SymmetryMode::Lines {
            // m 個の回転（k = 0 は恒等）と m 枚の鏡（軸の角度 angle + 180 k / m 度）
            let m = (self.count / 2) as i64;
            for i in 1..m {
                let (c, s) = turn(i, m);
                v.push(t(c, -s, s, c));
            }
            for k in 0..m {
                // 鏡映の行列は [[cos 2φ, sin 2φ], [sin 2φ, −cos 2φ]]。2φ = 2 angle + 360 k / m 度
                let two_phi = 2.0 * self.angle + 360.0 * k as f64 / m as f64;
                let (c, s) = quarter_or_angle(two_phi);
                v.push(t(c, s, s, 0.0 - c));
            }
        }
        Ok(v)
    }
}

/// n 分の i 周（反時計回り）の (cos, sin)。四分の一周の倍数は正確な値（cos・sin を通らない）。
fn turn(i: i64, n: i64) -> (f64, f64) {
    if 4 * i % n == 0 {
        let q = 4 * i / n;
        (
            if q == 2 { -1.0 } else { 0.0 },
            match q {
                1 => 1.0,
                3 => -1.0,
                _ => 0.0,
            },
        )
    } else {
        let a = 2.0 * std::f64::consts::PI * i as f64 / n as f64;
        (a.cos(), a.sin())
    }
}

/// 角度（度）の (cos, sin)。90 度の倍数（誤差 1e-9 以内）は正確な 0・±1、それ以外は libm の `cos`・`sin`。
fn quarter_or_angle(degrees: f64) -> (f64, f64) {
    let q = degrees / 90.0;
    if (q - q.round()).abs() < 1e-9 {
        return match (q.round() as i64).rem_euclid(4) {
            0 => (1.0, 0.0),
            1 => (0.0, 1.0),
            2 => (-1.0, 0.0),
            _ => (0.0, -1.0),
        };
    }
    let a = degrees.to_radians();
    (a.cos(), a.sin())
}

/// 中心のまわりの直交変換（C# の CanvasSymmetryTransform）。逆写しは転置。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SymmetryTransform {
    cx: f64,
    cy: f64,
    a: f64,
    b: f64,
    c: f64,
    d: f64,
}

impl SymmetryTransform {
    /// 点を写す。
    #[inline]
    pub fn map(&self, x: f64, y: f64) -> (f64, f64) {
        let (x, y) = (x - self.cx, y - self.cy);
        (
            self.cx + self.a * x + self.b * y,
            self.cy + self.c * x + self.d * y,
        )
    }
    /// 写した点を元へ戻す（転置）。
    #[inline]
    pub fn inverse(&self, u: f64, v: f64) -> (f64, f64) {
        let (u, v) = (u - self.cx, v - self.cy);
        (
            self.cx + self.a * u + self.c * v,
            self.cy + self.b * u + self.d * v,
        )
    }

    /// 中心と、中心のまわりの 2×2 の係数（行の順: a b / c d）。
    pub(crate) fn parts(&self) -> (DVec2, [f64; 4]) {
        (
            DVec2::new(self.cx, self.cy),
            [self.a, self.b, self.c, self.d],
        )
    }

    /// 恒等（写さない）。
    pub(crate) fn identity() -> SymmetryTransform {
        SymmetryTransform {
            cx: 0.0,
            cy: 0.0,
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
        }
    }

    /// 恒等（写さない）か。
    pub(crate) fn is_identity(&self) -> bool {
        self.a == 1.0 && self.b == 0.0 && self.c == 0.0 && self.d == 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarter_turns_are_exact_and_others_use_the_angle() {
        let s = CanvasSymmetry::new(SymmetryMode::Radial, DVec2::new(10.0, 20.0), 4).unwrap();
        let t = s.transforms().unwrap();
        assert_eq!(t.len(), 4);
        assert_eq!(t[1].map(11.0, 20.0), (10.0, 21.0)); // 90°
        assert_eq!(t[2].map(11.0, 20.0), (9.0, 20.0));
        assert_eq!(t[3].map(11.0, 20.0), (10.0, 19.0));
        let three = CanvasSymmetry::new(SymmetryMode::Radial, DVec2::ZERO, 3).unwrap();
        let (x, y) = three.transforms().unwrap()[1].map(1.0, 0.0);
        assert!((x + 0.5).abs() < 1e-12 && (y - 0.75f64.sqrt()).abs() < 1e-12);
        for tr in &t {
            let (u, v) = tr.map(3.25, -7.5);
            let (x, y) = tr.inverse(u, v);
            assert!((x - 3.25).abs() < 1e-12 && (y + 7.5).abs() < 1e-12);
        }
    }

    /// 写しの行列（中心を引いた 2×2）を、並びによらず比べられる形にする。
    fn matrices(s: &CanvasSymmetry) -> Vec<[f64; 4]> {
        let mut v: Vec<[f64; 4]> = s
            .transforms()
            .unwrap()
            .iter()
            .map(|t| t.parts().1)
            .collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v
    }

    #[test]
    fn two_lines_at_90_and_0_and_four_lines_equal_the_old_vertical_horizontal_and_both() {
        for c in [DVec2::ZERO, DVec2::new(30.5, 12.0), DVec2::new(-4.0, 99.0)] {
            let same = |lines: CanvasSymmetry, old: SymmetryMode| {
                let old = CanvasSymmetry::new(old, c, 2).unwrap();
                // 係数は正確に同じ（正確な 0・±1）で、中心も同じ
                assert_eq!(matrices(&lines), matrices(&old), "{:?}", lines.angle);
                let mut a: Vec<_> = lines
                    .transforms()
                    .unwrap()
                    .iter()
                    .map(|t| t.parts())
                    .collect();
                let mut b: Vec<_> = old
                    .transforms()
                    .unwrap()
                    .iter()
                    .map(|t| t.parts())
                    .collect();
                a.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap());
                b.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap());
                assert_eq!(a, b);
            };
            same(
                CanvasSymmetry::lines(c, 2, 90.0).unwrap(),
                SymmetryMode::Vertical,
            );
            same(
                CanvasSymmetry::lines(c, 2, 0.0).unwrap(),
                SymmetryMode::Horizontal,
            );
            same(
                CanvasSymmetry::lines(c, 4, 0.0).unwrap(),
                SymmetryMode::Both,
            );
            // 向きが逆でも、-90 度・180 度・270 度でも同じ軸
            same(
                CanvasSymmetry::lines(c, 2, -90.0).unwrap(),
                SymmetryMode::Vertical,
            );
            same(
                CanvasSymmetry::lines(c, 2, 180.0).unwrap(),
                SymmetryMode::Horizontal,
            );
            same(
                CanvasSymmetry::lines(c, 2, -180.0).unwrap(),
                SymmetryMode::Horizontal,
            );
            same(
                CanvasSymmetry::lines(c, 4, 90.0).unwrap(),
                SymmetryMode::Both,
            );
        }
    }

    #[test]
    fn lines_values_are_exact_on_multiples_of_45_degrees_and_use_the_angle_elsewhere() {
        let c = DVec2::new(10.0, 20.0);
        // 45 度の鏡: x と y を入れ替える
        let diagonal = CanvasSymmetry::lines(c, 2, 45.0)
            .unwrap()
            .transforms()
            .unwrap();
        assert_eq!(diagonal.len(), 2);
        assert_eq!(diagonal[0].map(13.0, 20.0), (13.0, 20.0), "最初は恒等");
        assert_eq!(diagonal[1].map(13.0, 20.0), (10.0, 23.0));
        assert_eq!(diagonal[1].map(10.0, 25.0), (15.0, 20.0));
        // -45 度の鏡: (dx, dy) → (−dy, −dx)
        let anti = CanvasSymmetry::lines(c, 2, -45.0)
            .unwrap()
            .transforms()
            .unwrap();
        assert_eq!(anti[1].map(13.0, 20.0), (10.0, 17.0));
        // 8 本: 回転は 90 度の倍数が正確、鏡は 22.5 度おき
        let eight = CanvasSymmetry::lines(c, 8, 0.0)
            .unwrap()
            .transforms()
            .unwrap();
        assert_eq!(eight.len(), 8);
        assert_eq!(eight[1].map(11.0, 20.0), (10.0, 21.0), "回転 90 度は正確");
        assert_eq!(eight[2].map(11.0, 20.0), (9.0, 20.0));
        // 角度つきの鏡は軸の上の点を動かさず、軸に直交する点を反対側へ
        for degrees in [10.0f64, 30.0, 77.0, -123.0, 200.0] {
            let s = CanvasSymmetry::lines(c, 2, degrees).unwrap();
            let m = s.transforms().unwrap()[1];
            let (ax, ay) = (degrees.to_radians().cos(), degrees.to_radians().sin());
            let on_axis = m.map(c.x + 5.0 * ax, c.y + 5.0 * ay);
            assert!(
                (on_axis.0 - c.x - 5.0 * ax).abs() < 1e-12
                    && (on_axis.1 - c.y - 5.0 * ay).abs() < 1e-12
            );
            let across = m.map(c.x - 3.0 * ay, c.y + 3.0 * ax);
            assert!(
                (across.0 - c.x - 3.0 * ay).abs() < 1e-12
                    && (across.1 - c.y + 3.0 * ax).abs() < 1e-12
            );
        }
    }

    #[test]
    fn lines_are_distinct_and_closed_under_composition() {
        let close = |a: &[f64; 4], b: &[f64; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9);
        let mul = |a: &[f64; 4], b: &[f64; 4]| {
            [
                a[0] * b[0] + a[1] * b[2],
                a[0] * b[1] + a[1] * b[3],
                a[2] * b[0] + a[3] * b[2],
                a[2] * b[1] + a[3] * b[3],
            ]
        };
        for count in (2..=16).step_by(2) {
            for degrees in [
                0.0,
                0.3 * 180.0 / std::f64::consts::PI,
                45.0,
                90.0,
                -37.5,
                360.0,
            ] {
                let s = CanvasSymmetry::lines(DVec2::new(3.0, 4.0), count, degrees).unwrap();
                let m = matrices(&s);
                assert_eq!(m.len(), count as usize, "写しは線の本数と同じ数");
                for i in 0..m.len() {
                    for j in 0..i {
                        assert!(
                            !close(&m[i], &m[j]),
                            "{count} 本 {degrees}°: {i} と {j} が同じ"
                        );
                    }
                }
                for a in &m {
                    for b in &m {
                        let ab = mul(a, b);
                        assert!(
                            m.iter().any(|c| close(c, &ab)),
                            "{count} 本 {degrees}°: 閉じない"
                        );
                    }
                }
                // 直交（逆は転置）
                for t in s.transforms().unwrap() {
                    let (u, v) = t.map(7.25, -1.5);
                    let (x, y) = t.inverse(u, v);
                    assert!((x - 7.25).abs() < 1e-12 && (y + 1.5).abs() < 1e-12);
                }
            }
        }
    }

    #[test]
    fn lines_refuse_odd_counts_and_bad_angles() {
        let c = DVec2::new(5.0, 5.0);
        assert!(CanvasSymmetry::lines(c, 3, 0.0).is_err(), "奇数");
        assert!(CanvasSymmetry::lines(c, 0, 0.0).is_err());
        assert!(CanvasSymmetry::lines(c, 18, 0.0).is_err(), "17 以上");
        assert!(CanvasSymmetry::lines(c, 4, f64::NAN).is_err());
        assert!(CanvasSymmetry::lines(c, 4, 360.5).is_err());
        assert!(CanvasSymmetry::lines(c, 4, -360.0).is_ok());
        assert!(CanvasSymmetry::lines(DVec2::new(f64::INFINITY, 0.0), 4, 0.0).is_err());
        // 放射状は奇数もよく、角度は写しに効かない（0 のまま）
        let mut radial = CanvasSymmetry::new(SymmetryMode::Radial, c, 5).unwrap();
        assert_eq!(radial.transforms().unwrap().len(), 5);
        radial.angle = 400.0;
        assert!(radial.validate().is_err(), "角度はどの種類でも確かめる");
        assert!(CanvasSymmetry::lines(c, 6, 15.0).unwrap().enabled());
    }

    #[test]
    fn modes_list_their_copies_and_bad_settings_are_refused() {
        let c = DVec2::new(5.0, 5.0);
        let n = |m| {
            CanvasSymmetry::new(m, c, 2)
                .unwrap()
                .transforms()
                .unwrap()
                .len()
        };
        assert_eq!(n(SymmetryMode::None), 1);
        assert_eq!(n(SymmetryMode::Vertical), 2);
        assert_eq!(n(SymmetryMode::Horizontal), 2);
        assert_eq!(n(SymmetryMode::Both), 4);
        assert_eq!(n(SymmetryMode::Radial), 2);
        assert!(CanvasSymmetry::new(SymmetryMode::Radial, c, 1).is_err());
        assert!(CanvasSymmetry::new(SymmetryMode::Radial, c, 17).is_err());
        assert!(CanvasSymmetry::new(SymmetryMode::Vertical, DVec2::new(f64::NAN, 0.0), 2).is_err());
        assert!(CanvasSymmetry::new(SymmetryMode::Vertical, DVec2::new(2e7, 0.0), 2).is_err());
    }
}
