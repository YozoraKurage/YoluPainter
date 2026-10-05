//! 画素 1 つの合成の式（C# の CpuCompositor.BlendUnchecked・BlendRgb・ClipOnto・MixRgb・Fade）。
//!
//! 保存したままの RGB の空間で、W3C の source-over（部分的なアルファの項を含む）に、分離できるモードは Photoshop の式
//! （ソフトライトも Photoshop のもの）、色相・彩度・カラー・輝度は W3C の非分離の式（輝度 0.3/0.59/0.11）を使う。
//! 結果は層ごとに RGBA8 へ丸める。演算の順（左から右）も C# と同じにしてあり、同じ double を経てバイトが一致する
//! （Rust は FMA へまとめないので、C# の Mono と同じ IEEE の倍精度の演算になる）。

use std::sync::OnceLock;

use crate::math::{to_byte, UNIT};
use crate::types::{BlendMode, Rgba8};

pub(crate) mod lanes;
mod rows;

pub(crate) use rows::mix_row_at;

/// 画素の計算（合成・調整・フィルター・Normal チャンネル）が使っている SIMD の道の名前（`"avx2"`・`"sse41"`・`"scalar"`）。
/// 診断と計測用。CPU が持つ一番広い道を選び、環境変数 `YOLU_SIMD` で下げられる。
pub fn simd_level_name() -> &'static str {
    use crate::math::simd::Level;
    match crate::math::simd::level() {
        Level::Avx2 => "avx2",
        Level::Sse41 => "sse41",
        Level::Scalar => "scalar",
    }
}
pub use rows::{blend_row, clip_row, fade_row, RowAmount};

/// Darker/Lighter Color の和の比較と HardMix の境の余裕（半段）。和は 8 bit の値の和なので、違えば 1/255 以上離れている。
pub(crate) const TIE_MARGIN: f64 = 0.5 / 255.0;
/// これより小さい a·c は非正規化数になり得るので、下が透明のときの近道を使わない（C# の MinShortcutAlpha）。
pub(crate) const MIN_SHORTCUT_ALPHA: f64 = 1e-300;

/// 下（destination）に上（source）を不透明度 opacity・モード mode で重ねる（C# の BlendUnchecked）。opacity は 0〜1。
#[inline]
pub fn blend(destination: Rgba8, source: Rgba8, opacity: f64, mode: BlendMode) -> Rgba8 {
    let simple = mode == BlendMode::Normal || mode == BlendMode::PassThrough;
    // 不透明な画素を通常・量 1 で重ねると、式の結果は上の画素そのもの
    if source.a == 255 && opacity == 1.0 && simple {
        return source;
    }
    let sa = UNIT[source.a as usize] * opacity;
    let da = UNIT[destination.a as usize];
    if sa <= 0.0 {
        return destination;
    }
    let a = sa + da * (1.0 - sa);
    if a <= 0.0 {
        return Rgba8::TRANSPARENT;
    }
    let (dr, dg, db) = (
        UNIT[destination.r as usize],
        UNIT[destination.g as usize],
        UNIT[destination.b as usize],
    );
    let (sr, sg, sb) = (
        UNIT[source.r as usize],
        UNIT[source.g as usize],
        UNIT[source.b as usize],
    );
    let (br, bg, bb) = if simple {
        (sr, sg, sb)
    } else {
        blend_rgb(mode, dr, dg, db, sr, sg, sb)
    };
    let wd = (1.0 - sa) * da;
    let ws = (1.0 - da) * sa;
    let wb = da * sa;
    Rgba8::new(
        to_byte((wd * dr + ws * sr + wb * br) / a),
        to_byte((wd * dg + ws * sg + wb * bg) / a),
        to_byte((wd * db + ws * sb + wb * bb) / a),
        to_byte(a),
    )
}

/// モードの合成色 B(下, 上)。成分は 0〜1、結果も 0〜1 に収める（C# の BlendRgb）。
#[inline]
pub fn blend_rgb(
    mode: BlendMode,
    dr: f64,
    dg: f64,
    db: f64,
    sr: f64,
    sg: f64,
    sb: f64,
) -> (f64, f64, f64) {
    match mode {
        BlendMode::Normal | BlendMode::PassThrough => (sr, sg, sb),
        BlendMode::Hue => {
            let (tr, tg, tb) = set_sat(sr, sg, sb, sat(dr, dg, db));
            set_lum(tr, tg, tb, lum(dr, dg, db))
        }
        BlendMode::Saturation => {
            let (tr, tg, tb) = set_sat(dr, dg, db, sat(sr, sg, sb));
            set_lum(tr, tg, tb, lum(dr, dg, db))
        }
        BlendMode::Color => set_lum(sr, sg, sb, lum(dr, dg, db)),
        BlendMode::Luminosity => set_lum(dr, dg, db, lum(sr, sg, sb)),
        BlendMode::DarkerColor => {
            if sr + sg + sb < dr + dg + db - TIE_MARGIN {
                (sr, sg, sb)
            } else {
                (dr, dg, db)
            }
        }
        BlendMode::LighterColor => {
            if sr + sg + sb > dr + dg + db + TIE_MARGIN {
                (sr, sg, sb)
            } else {
                (dr, dg, db)
            }
        }
        _ => (
            separable(mode, dr, sr),
            separable(mode, dg, sg),
            separable(mode, db, sb),
        ),
    }
}

/// 分離できるモードの 1 成分（C# の Separable）。
#[inline]
pub(crate) fn separable(mode: BlendMode, d: f64, s: f64) -> f64 {
    let v = match mode {
        BlendMode::Multiply => d * s,
        BlendMode::Screen => d + s - d * s,
        BlendMode::Overlay => {
            if d <= 0.5 {
                2.0 * d * s
            } else {
                1.0 - 2.0 * (1.0 - d) * (1.0 - s)
            }
        }
        BlendMode::Darken => min(d, s),
        BlendMode::Lighten => max(d, s),
        BlendMode::ColorDodge => dodge(d, s),
        BlendMode::ColorBurn => burn(d, s),
        BlendMode::LinearDodge => d + s,
        BlendMode::LinearBurn => d + s - 1.0,
        BlendMode::HardLight => {
            if s <= 0.5 {
                2.0 * d * s
            } else {
                1.0 - 2.0 * (1.0 - d) * (1.0 - s)
            }
        }
        BlendMode::SoftLight => {
            if s <= 0.5 {
                d - (1.0 - 2.0 * s) * d * (1.0 - d)
            } else {
                d + (2.0 * s - 1.0) * (d.sqrt() - d)
            }
        }
        BlendMode::VividLight => {
            if s <= 0.5 {
                burn(d, 2.0 * s)
            } else {
                dodge(d, 2.0 * s - 1.0)
            }
        }
        BlendMode::LinearLight => d + 2.0 * s - 1.0,
        BlendMode::PinLight => {
            if s <= 0.5 {
                min(d, 2.0 * s)
            } else {
                max(d, 2.0 * s - 1.0)
            }
        }
        BlendMode::HardMix => {
            if d + s >= 1.0 - TIE_MARGIN {
                1.0
            } else {
                0.0
            }
        }
        BlendMode::Difference => (d - s).abs(),
        BlendMode::Exclusion => d + s - 2.0 * d * s,
        BlendMode::Subtract => d - s,
        BlendMode::Divide => {
            if s <= 0.0 {
                if d <= 0.0 {
                    0.0
                } else {
                    1.0
                }
            } else {
                d / s
            }
        }
        _ => s,
    };
    if v < 0.0 {
        0.0
    } else if v > 1.0 {
        1.0
    } else {
        v
    }
}

/// C# の Math.Min / Math.Max（NaN の来ない値だけを比べる）。
#[inline(always)]
fn min(a: f64, b: f64) -> f64 {
    if a < b {
        a
    } else {
        b
    }
}
#[inline(always)]
fn max(a: f64, b: f64) -> f64 {
    if a > b {
        a
    } else {
        b
    }
}
#[inline]
fn dodge(d: f64, s: f64) -> f64 {
    if d <= 0.0 {
        0.0
    } else if s >= 1.0 {
        1.0
    } else {
        min(1.0, d / (1.0 - s))
    }
}
#[inline]
fn burn(d: f64, s: f64) -> f64 {
    if d >= 1.0 {
        1.0
    } else if s <= 0.0 {
        0.0
    } else {
        1.0 - min(1.0, (1.0 - d) / s)
    }
}
#[inline]
fn lum(r: f64, g: f64, b: f64) -> f64 {
    0.3 * r + 0.59 * g + 0.11 * b
}
#[inline]
fn sat(r: f64, g: f64, b: f64) -> f64 {
    let mut mx = if g > b { g } else { b };
    if r > mx {
        mx = r;
    }
    let mut mn = if g < b { g } else { b };
    if r < mn {
        mn = r;
    }
    mx - mn
}
/// W3C の SetLum の後に ClipColor と 0〜1 への切り詰め。
#[inline]
fn set_lum(cr: f64, cg: f64, cb: f64, l: f64) -> (f64, f64, f64) {
    let delta = l - lum(cr, cg, cb);
    let (mut r, mut g, mut b) = (cr + delta, cg + delta, cb + delta);
    let lm = lum(r, g, b);
    let mut n = if g < b { g } else { b };
    let mut x = if g > b { g } else { b };
    if r < n {
        n = r;
    }
    if r > x {
        x = r;
    }
    if n < 0.0 && lm - n > 1e-12 {
        r = lm + (r - lm) * lm / (lm - n);
        g = lm + (g - lm) * lm / (lm - n);
        b = lm + (b - lm) * lm / (lm - n);
    }
    if x > 1.0 && x - lm > 1e-12 {
        r = lm + (r - lm) * (1.0 - lm) / (x - lm);
        g = lm + (g - lm) * (1.0 - lm) / (x - lm);
        b = lm + (b - lm) * (1.0 - lm) / (x - lm);
    }
    (c01(r), c01(g), c01(b))
}
/// W3C の SetSat: 一番大きい成分を s、一番小さい成分を 0 に、中間は比を保つ。
#[inline]
fn set_sat(r: f64, g: f64, b: f64, s: f64) -> (f64, f64, f64) {
    let mut mx = if g > b { g } else { b };
    if r > mx {
        mx = r;
    }
    let mut mn = if g < b { g } else { b };
    if r < mn {
        mn = r;
    }
    if mx - mn <= 1e-12 {
        return (0.0, 0.0, 0.0);
    }
    let f = |c: f64| {
        if c == mx {
            s
        } else if c == mn {
            0.0
        } else {
            (c - mn) * s / (mx - mn)
        }
    };
    (f(r), f(g), f(b))
}
#[inline(always)]
fn c01(v: f64) -> f64 {
    if v < 0.0 {
        0.0
    } else if v > 1.0 {
        1.0
    } else {
        v
    }
}

/// クリッピングされた層の色をクリッピングの下地へ重ねる（C# の ClipOnto）。下地のアルファはそのまま（下地の外へは描かない）。
#[inline]
pub fn clip_onto(group: Rgba8, clipped: Rgba8, amount: f64, mode: BlendMode) -> Rgba8 {
    let t = UNIT[clipped.a as usize] * amount;
    if t <= 0.0 || group.a == 0 {
        return group;
    }
    mix_rgb(group, clipped, t, mode)
}

/// below + (B(below, over) − below) × amount を成分ごとに 1 回で丸める。アルファは below のもの（C# の MixRgb）。
#[inline]
pub(crate) fn mix_rgb(below: Rgba8, over: Rgba8, amount: f64, mode: BlendMode) -> Rgba8 {
    let (dr, dg, db) = (
        UNIT[below.r as usize],
        UNIT[below.g as usize],
        UNIT[below.b as usize],
    );
    let (r, g, b) = blend_rgb(
        mode,
        dr,
        dg,
        db,
        UNIT[over.r as usize],
        UNIT[over.g as usize],
        UNIT[over.b as usize],
    );
    Rgba8::new(
        to_byte(dr + (r - dr) * amount),
        to_byte(dg + (g - dg) * amount),
        to_byte(db + (b - db) * amount),
        below.a,
    )
}

/// 下と中身を amount で混ぜる（プリマルチプライドの補間。透明な側がもう片方を暗くしない。C# の Fade）。
#[inline]
pub fn fade(backdrop: Rgba8, inner: Rgba8, amount: f64) -> Rgba8 {
    if amount >= 1.0 {
        return inner;
    }
    if amount <= 0.0 {
        return backdrop;
    }
    let ba = UNIT[backdrop.a as usize] * (1.0 - amount);
    let ia = UNIT[inner.a as usize] * amount;
    let a = ba + ia;
    if a <= 0.0 {
        return Rgba8::TRANSPARENT;
    }
    Rgba8::new(
        to_byte((UNIT[backdrop.r as usize] * ba + UNIT[inner.r as usize] * ia) / a),
        to_byte((UNIT[backdrop.g as usize] * ba + UNIT[inner.g as usize] * ia) / a),
        to_byte((UNIT[backdrop.b as usize] * ba + UNIT[inner.b as usize] * ia) / a),
        to_byte(a),
    )
}

/// 分離できるモードの B(下, 上) の 256×256 の表（下 << 8 | 上）。separable そのものの値なので、引いても計算しても同じ double。
/// モードごとに初めて使うときに 1 回だけ作る（1 つ 512 KiB）。
pub(crate) fn separable_table(mode: BlendMode) -> Option<&'static [f64]> {
    if !mode.is_separable() {
        return None;
    }
    static TABLES: [OnceLock<Box<[f64]>>; 27] = [const { OnceLock::new() }; 27];
    Some(TABLES[mode as usize].get_or_init(|| {
        let mut t = vec![0.0f64; 65536];
        for d in 0..256 {
            for s in 0..256 {
                t[d << 8 | s] = separable(mode, UNIT[d], UNIT[s]);
            }
        }
        t.into_boxed_slice()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use BlendMode::*;

    fn gray(mode: BlendMode, d: f64, s: f64) -> f64 {
        blend_rgb(mode, d, d, d, s, s, s).0
    }

    #[test]
    fn separable_formulas() {
        // C# BlendModeTests の表（下 d, 上 s → 値）
        let cases: &[(BlendMode, f64, f64, f64)] = &[
            (Normal, 0.3, 0.8, 0.8),
            (Multiply, 0.5, 0.5, 0.25),
            (Screen, 0.5, 0.5, 0.75),
            (Overlay, 0.2, 0.8, 0.32),
            (Overlay, 0.8, 0.2, 0.68),
            (Darken, 0.3, 0.6, 0.3),
            (Lighten, 0.3, 0.6, 0.6),
            (ColorDodge, 0.25, 0.5, 0.5),
            (ColorDodge, 0.5, 0.5, 1.0),
            (ColorDodge, 0.0, 1.0, 0.0),
            (ColorBurn, 0.75, 0.5, 0.5),
            (ColorBurn, 0.5, 0.5, 0.0),
            (ColorBurn, 1.0, 0.0, 1.0),
            (LinearDodge, 0.3, 0.4, 0.7),
            (LinearDodge, 0.6, 0.6, 1.0),
            (LinearBurn, 0.6, 0.6, 0.2),
            (LinearBurn, 0.2, 0.3, 0.0),
            (HardLight, 0.2, 0.8, 0.68),
            (HardLight, 0.8, 0.2, 0.32),
            (SoftLight, 0.25, 0.75, 0.375),
            (SoftLight, 0.25, 0.25, 0.15625),
            (SoftLight, 0.25, 0.5, 0.25),
            (VividLight, 0.5, 0.25, 0.0),
            (VividLight, 0.25, 0.75, 0.5),
            (LinearLight, 0.5, 0.75, 1.0),
            (LinearLight, 0.5, 0.6, 0.7),
            (PinLight, 0.5, 0.1, 0.2),
            (PinLight, 0.5, 0.9, 0.8),
            (PinLight, 0.5, 0.5, 0.5),
            (Difference, 0.2, 0.7, 0.5),
            (Exclusion, 0.5, 0.5, 0.5),
            (Exclusion, 0.2, 0.5, 0.5),
            (Subtract, 0.7, 0.2, 0.5),
            (Subtract, 0.2, 0.7, 0.0),
            (Divide, 0.25, 0.5, 0.5),
            (Divide, 0.5, 0.0, 1.0),
            (Divide, 0.0, 0.0, 0.0),
            (Divide, 0.5, 0.25, 1.0),
        ];
        for &(mode, d, s, want) in cases {
            let got = gray(mode, d, s);
            assert!(
                (got - want).abs() <= 1e-12,
                "{mode:?} {d},{s}: {got} != {want}"
            );
        }
        assert_eq!(gray(HardMix, 128.0 / 255.0, 127.0 / 255.0), 1.0);
        assert_eq!(gray(HardMix, 128.0 / 255.0, 126.0 / 255.0), 0.0);
        assert_eq!(gray(HardMix, 0.0, 1.0), 1.0);
    }

    #[test]
    fn non_separable_modes() {
        let l = |c: (f64, f64, f64)| 0.3 * c.0 + 0.59 * c.1 + 0.11 * c.2;
        let c = blend_rgb(Color, 0.5, 0.5, 0.5, 0.9, 0.2, 0.1);
        assert!((l(c) - 0.5).abs() < 1e-9 && c.0 > c.1 && c.1 > c.2);
        let c = blend_rgb(Luminosity, 0.8, 0.3, 0.2, 0.25, 0.25, 0.25);
        assert!((l(c) - 0.25).abs() < 1e-9 && c.0 > c.1);
        let c = blend_rgb(Hue, 0.8, 0.3, 0.2, 0.5, 0.5, 0.5);
        let want = l((0.8, 0.3, 0.2));
        assert!(
            (c.0 - want).abs() < 1e-9 && (c.1 - want).abs() < 1e-9 && (c.2 - want).abs() < 1e-9
        );
        let c = blend_rgb(Saturation, 0.8, 0.3, 0.2, 0.9, 0.9, 0.9);
        assert!((c.0 - c.1).abs() < 1e-12 && (c.1 - c.2).abs() < 1e-12);
        let c = blend_rgb(Hue, 0.8, 0.2, 0.2, 0.1, 0.1, 0.9);
        assert!(c.2 > c.0 && (l(c) - l((0.8, 0.2, 0.2))).abs() < 1e-9);
        // 色の和の比較: 同点は下を保つ
        assert_eq!(
            blend_rgb(DarkerColor, 0.9, 0.1, 0.1, 0.3, 0.3, 0.3),
            (0.3, 0.3, 0.3)
        );
        assert_eq!(
            blend_rgb(LighterColor, 0.9, 0.1, 0.1, 0.3, 0.3, 0.3),
            (0.9, 0.1, 0.1)
        );
        let d = (51.0 / 255.0, 1.0, 153.0 / 255.0);
        let s = (1.0, 204.0 / 255.0, 0.0);
        assert_eq!(blend_rgb(DarkerColor, d.0, d.1, d.2, s.0, s.1, s.2), d);
        assert_eq!(blend_rgb(LighterColor, d.0, d.1, d.2, s.0, s.1, s.2), d);
    }

    #[test]
    fn blend_reference_values() {
        // C# CoreTests・BlendModeTests の Blend の値
        let src = Rgba8::new(128, 64, 32, 128);
        assert_eq!(blend(Rgba8::TRANSPARENT, src, 1.0, Multiply), src);
        assert_eq!(
            blend(
                Rgba8::new(255, 0, 0, 255),
                Rgba8::new(0, 0, 255, 128),
                1.0,
                Normal
            ),
            Rgba8::new(127, 0, 128, 255)
        );
        assert_eq!(
            blend(
                Rgba8::new(128, 128, 128, 255),
                Rgba8::new(128, 128, 128, 255),
                1.0,
                Multiply
            )
            .r,
            64
        );
        assert_eq!(
            blend(
                Rgba8::new(128, 128, 128, 255),
                Rgba8::new(128, 128, 128, 255),
                1.0,
                Screen
            )
            .r,
            192
        );
        let over = Rgba8::new(200, 100, 50, 255);
        assert_eq!(blend(Rgba8::TRANSPARENT, over, 1.0, Multiply), over);
        let r = blend(Rgba8::new(100, 200, 255, 255), over, 0.5, Difference);
        assert_eq!((r.a, r.r), (255, 100));
    }

    #[test]
    fn tables_hold_the_formula() {
        for mode in BlendMode::LAYER_MODES {
            let Some(t) = separable_table(mode) else {
                continue;
            };
            for d in (0..256).step_by(7) {
                for s in (0..256).step_by(5) {
                    assert_eq!(
                        t[d << 8 | s].to_bits(),
                        separable(mode, UNIT[d], UNIT[s]).to_bits()
                    );
                }
            }
        }
    }

    /// 保存形式（.ylp の正本・PSD の取り込みの番号）に入る合成モードの数値は並べ替えず、個数も変えない（既存のファイルの意味が変わらない）。
    /// 末尾に足すときは、この表と個数を意図して書き換える。C# の BlendModeTests.TheStoredValuesOfTheFirstModesNeverChange は
    /// 0・1・2・25・26 と個数だけを見ているが、ここは 27 個の全部を C# の列挙の名前と並べて固定する。
    #[test]
    fn the_stored_values_of_every_mode_never_change() {
        let stored: [(BlendMode, u8, &str); 27] = [
            (Normal, 0, "Normal"),
            (Multiply, 1, "Multiply"),
            (Screen, 2, "Screen"),
            (Overlay, 3, "Overlay"),
            (Darken, 4, "Darken"),
            (Lighten, 5, "Lighten"),
            (ColorDodge, 6, "ColorDodge"),
            (ColorBurn, 7, "ColorBurn"),
            (LinearDodge, 8, "LinearDodge"),
            (LinearBurn, 9, "LinearBurn"),
            (HardLight, 10, "HardLight"),
            (SoftLight, 11, "SoftLight"),
            (VividLight, 12, "VividLight"),
            (LinearLight, 13, "LinearLight"),
            (PinLight, 14, "PinLight"),
            (HardMix, 15, "HardMix"),
            (Difference, 16, "Difference"),
            (Exclusion, 17, "Exclusion"),
            (Subtract, 18, "Subtract"),
            (Divide, 19, "Divide"),
            (Hue, 20, "Hue"),
            (Saturation, 21, "Saturation"),
            (Color, 22, "Color"),
            (Luminosity, 23, "Luminosity"),
            (DarkerColor, 24, "DarkerColor"),
            (LighterColor, 25, "LighterColor"),
            (PassThrough, 26, "PassThrough"),
        ];
        for (mode, value, name) in stored {
            assert_eq!(mode as u8, value, "{name} の保存値");
            assert_eq!(mode.name(), name);
            assert_eq!(BlendMode::from_index(value), Some(mode), "{name}");
            assert_eq!(BlendMode::from_name(name), Some(mode), "{name}");
        }
        // 個数: 層に付けられる 26 と、グループだけの PassThrough。その先の番号は無い
        assert_eq!(stored.len(), 27);
        assert_eq!(BlendMode::LAYER_MODES.len(), 26);
        assert_eq!(
            BlendMode::LAYER_MODES.to_vec(),
            stored[..26].iter().map(|s| s.0).collect::<Vec<_>>(),
            "層のモードは番号の順"
        );
        assert!(BlendMode::LAYER_MODES.iter().all(|m| *m != PassThrough));
        for value in 27..=255u8 {
            assert_eq!(BlendMode::from_index(value), None, "{value}");
        }
        assert_eq!(BlendMode::from_name("Normal "), None);
        assert_eq!(BlendMode::from_name("normal"), None);
        assert_eq!(BlendMode::default(), Normal);
        // 成分ごとのモードは Multiply から Divide の 19 個（PassThrough・Hue 以降・Normal は成分ごとではない）
        assert_eq!(stored.iter().filter(|s| s.0.is_separable()).count(), 19);
    }
}
