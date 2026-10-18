//! 近い色の比べ（バケツの「近い色」と、ブラシの「参照レイヤーの線をはみ出さない」が同じ式を使う）。
//!
//! 2 つの straight RGBA8 の色を、アルファの差と色の差の 2 つで比べる。アルファの差は許し幅を独立した上限にし、色の差は
//! 各成分の差の最大（[`ColorDistance::Rgb`]）か、赤の平均で重みを変える整数の知覚近似（[`ColorDistance::Perceptual`]）。
//! 整数だけの式なので、道・スレッド・OS によらず同じ答え。

use crate::Rgba8;

/// 色の差の測り方。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorDistance {
    /// 各成分（R・G・B）の差の最大。
    #[default]
    Rgb,
    /// 赤の平均に応じて重みを変える、整数の知覚近似。
    Perceptual,
}

/// `a` と `b` が許し幅 `tolerance`（0〜255）の内で同じ色か。アルファの差は独立した上限。
#[inline]
pub fn similar(a: Rgba8, b: Rgba8, tolerance: u8, distance: ColorDistance) -> bool {
    if a.a.abs_diff(b.a) > tolerance {
        return false;
    }
    let d = [
        a.r.abs_diff(b.r) as u64,
        a.g.abs_diff(b.g) as u64,
        a.b.abs_diff(b.b) as u64,
    ];
    match distance {
        ColorDistance::Rgb => d.into_iter().max().unwrap_or(0) <= tolerance as u64,
        ColorDistance::Perceptual => {
            let red = (a.r as u64 + b.r as u64) / 2;
            (512 + red) * d[0] * d[0] + 1024 * d[1] * d[1] + (767 - red) * d[2] * d[2]
                <= 2303 * (tolerance as u64).pow(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_is_an_independent_limit() {
        let clear = Rgba8::TRANSPARENT;
        assert!(!similar(
            clear,
            Rgba8::new(255, 255, 255, 20),
            26,
            ColorDistance::Rgb
        ));
        assert!(similar(
            clear,
            Rgba8::new(0, 0, 0, 26),
            26,
            ColorDistance::Rgb
        ));
        assert!(!similar(
            clear,
            Rgba8::new(0, 0, 0, 27),
            26,
            ColorDistance::Rgb
        ));
    }

    #[test]
    fn perceptual_weights_green_more_than_blue() {
        let base = Rgba8::new(100, 100, 100, 255);
        let green = Rgba8::new(100, 140, 100, 255);
        let blue = Rgba8::new(100, 100, 140, 255);
        assert!(!similar(base, green, 25, ColorDistance::Perceptual));
        assert!(similar(base, blue, 25, ColorDistance::Perceptual));
    }
}
