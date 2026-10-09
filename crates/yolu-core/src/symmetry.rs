//! 2D の対称（C# の CanvasSymmetry・BrushStroke.Symmetry）。
//!
//! 文書の画素の座標で、中心のまわりの直交変換（鏡映と回転）の組を作り、ブラシの各ダブを全部の写しへ置く。写しの画素は、画素の
//! 中心を逆に写した元のダブの上の位置で被覆率を測る（筆先の画像を補間し直さない）。写しが重なる画素は大きい方の被覆率で 1 回だけ
//! 塗る。放射状の写しの角度は、四分の一周の倍数なら正確な値（cos・sin を通らない）、それ以外は libm の `cos`・`sin` を通る。
//! C# とのバイト一致を確かめたのは同じ libm（Linux の glibc）の上で、別の libm では 1 ULP ずれ得る。鏡映・四分の一周の倍数
//! だけの組（縦・横・両方、放射状の 2・4）は正確な値だけを通る。
//! ここは文書の画素の上（UV の平面）の対称だけを扱う。3D の面のストロークは画素ごとに `apply_pixel` で塗り、`Brush.symmetry`
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
}

/// 対称の設定（C# の CanvasSymmetrySettings）。中心はキャンバスの画素の座標（画素の中心は整数 + 0.5）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasSymmetry {
    pub mode: SymmetryMode,
    pub center: DVec2,
    /// 放射状の写しの数（2〜16。ほかの種類でも範囲は確かめる、C# と同じ）。
    pub count: u32,
}

impl Default for CanvasSymmetry {
    fn default() -> Self {
        CanvasSymmetry {
            mode: SymmetryMode::None,
            center: DVec2::ZERO,
            count: 2,
        }
    }
}

/// 中心の座標の範囲（C# と同じ）。
const MAX_CENTER: f64 = 10_000_000.0;

impl CanvasSymmetry {
    /// 確かめてから作る。
    pub fn new(mode: SymmetryMode, center: DVec2, count: u32) -> Result<Self, CoreError> {
        let s = CanvasSymmetry {
            mode,
            center,
            count,
        };
        s.validate()?;
        Ok(s)
    }

    /// 対称が効くか（None 以外）。
    pub fn enabled(&self) -> bool {
        self.mode != SymmetryMode::None
    }

    /// 中心が有限で ±1e7 の中、写しの数が 2〜16。
    pub fn validate(&self) -> Result<(), CoreError> {
        require_finite(self.center.x, "symmetry center")?;
        require_finite(self.center.y, "symmetry center")?;
        if self.center.x.abs() > MAX_CENTER || self.center.y.abs() > MAX_CENTER {
            return Err(CoreError::InvalidArgument("対称の中心（±1e7）"));
        }
        if !(2..=16).contains(&self.count) {
            return Err(CoreError::InvalidArgument("放射状の写しの数（2〜16）"));
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
                let (c, s) = if 4 * i % n == 0 {
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
                };
                v.push(t(c, -s, s, c));
            }
        }
        Ok(v)
    }
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
