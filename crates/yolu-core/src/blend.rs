//! 画素の合成の式: source-over と合成モード・クリッピング・調整レイヤーの混ぜ・通過のグループのフェード。
//!
//! 保存したままの RGB の空間で、W3C の source-over（部分的なアルファの項を含む）に、分離できるモードは Photoshop の式
//! （ソフトライトも Photoshop のもの）、色相・彩度・カラー・輝度は W3C の非分離の式（輝度 0.3/0.59/0.11）を使う。
//! 計算は f32 で、結果はレイヤーごとに RGBA8 へ丸める。式は 1 つ（[`lanes`] と行の核の `*_block`）で、画素ごとの関数（[`blend`] など）・
//! 行の核（[`blend_row`] など）のスカラー・SSE4.1・AVX2・NEON の道のどれも同じ関数を通るので、道とスレッド数によらず同じバイトになる
//! （演算は IEEE の四則・平方根・floor・比較・選択だけ。積和の命令は使わない）。
//!
//! この式がこの crate の合成の正本。PSD の取り込みの照らし（yolu-io）もこの関数を呼ぶ。ブラシが描く画素の重ね（Normal）と
//! フェードも、同じ N 画素の式（`blend_block`・`fade_block`）を通る。

use crate::types::{BlendMode, Rgba8};

pub(crate) mod lanes;
mod rows;

#[cfg(test)]
pub(crate) use rows::scalar_steps;
pub(crate) use rows::{blend_block, fade_block, mix_row_at, simd_step};

/// 画素の計算（合成・調整・フィルター・Normal チャンネル）が使っている SIMD の道の名前（x86_64 は `"avx2"`・`"sse41"`、aarch64 は `"neon"`、
/// どの CPU でも `"scalar"`）。診断と計測用。CPU が持つ一番広い道を選び、環境変数 `YOLU_SIMD` で下げられる。
pub fn simd_level_name() -> &'static str {
    crate::math::simd::level().name()
}
pub use rows::{blend_row, clip_row, fade_row, RowAmount};

/// Darker/Lighter Color の和の比較と HardMix の境の余裕（半段）。和は 8 bit の値の和なので、違えば 1/255 以上離れている。
pub(crate) const TIE_MARGIN: f32 = 0.5 / 255.0;
/// これより小さい a·c は非正規化数になり得るので、下が透明のときの近道を使わない（f32 の正規化数の下限 約 1.2e-38 より十分上）。
pub(crate) const MIN_SHORTCUT_ALPHA: f32 = 1e-30;

/// 下（destination）に上（source）を不透明度 opacity・モード mode で重ねる。opacity は 0〜1（f32 へ丸めて使う）。
#[inline]
pub fn blend(destination: Rgba8, source: Rgba8, opacity: f64, mode: BlendMode) -> Rgba8 {
    rows::blend_pixel(destination, source, opacity, mode)
}

/// クリッピングされたレイヤーの色をクリッピングの下地へ重ねる。下地のアルファはそのまま（下地の外へは描かない）。
#[inline]
pub fn clip_onto(group: Rgba8, clipped: Rgba8, amount: f64, mode: BlendMode) -> Rgba8 {
    rows::clip_pixel(group, clipped, amount, mode)
}

/// below + (B(below, over) − below) × amount を成分ごとに 1 回で丸める。アルファは below のもの。量が 0 以下・below が
/// 完全に透明なら below のまま（調整レイヤーの合成）。
#[inline]
pub fn mix_rgb(below: Rgba8, over: Rgba8, amount: f64, mode: BlendMode) -> Rgba8 {
    rows::mix_pixel(below, over, amount, mode)
}

/// 下と中身を amount で混ぜる（プリマルチプライドの補間。透明な側がもう片方を暗くしない）。
#[inline]
pub fn fade(backdrop: Rgba8, inner: Rgba8, amount: f64) -> Rgba8 {
    rows::fade_pixel(backdrop, inner, amount)
}

#[cfg(test)]
mod tests {
    use super::lanes::dispatch_mode;
    use super::*;
    use crate::math::simd::{Lanes32, Scalar1};
    use BlendMode::*;

    /// モードの合成色 B(下, 上) を 1 本のレーンで。
    fn blend_rgb(
        mode: BlendMode,
        dr: f32,
        dg: f32,
        db: f32,
        sr: f32,
        sg: f32,
        sb: f32,
    ) -> (f32, f32, f32) {
        unsafe fn rgb<V: Lanes32, const MODE: u8>(d: [V::F; 3], s: [V::F; 3]) -> [V::F; 3] {
            lanes::blend_rgb::<V, MODE>(d, s)
        }
        // SAFETY: 1 本のレーンは CPU の前提を持たない
        let c = unsafe { dispatch_mode!(mode, rgb::<Scalar1>([dr, dg, db], [sr, sg, sb])) };
        (c[0], c[1], c[2])
    }

    fn gray(mode: BlendMode, d: f32, s: f32) -> f32 {
        blend_rgb(mode, d, d, d, s, s, s).0
    }

    #[test]
    fn separable_formulas() {
        // 分離できるモードの式の値の表（下 d, 上 s → 値）
        let cases: &[(BlendMode, f32, f32, f32)] = &[
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
                (got - want).abs() <= 1e-6,
                "{mode:?} {d},{s}: {got} != {want}"
            );
        }
        assert_eq!(gray(HardMix, 128.0 / 255.0, 127.0 / 255.0), 1.0);
        assert_eq!(gray(HardMix, 128.0 / 255.0, 126.0 / 255.0), 0.0);
        assert_eq!(gray(HardMix, 0.0, 1.0), 1.0);
    }

    #[test]
    fn non_separable_modes() {
        let l = |c: (f32, f32, f32)| 0.3 * c.0 + 0.59 * c.1 + 0.11 * c.2;
        let c = blend_rgb(Color, 0.5, 0.5, 0.5, 0.9, 0.2, 0.1);
        assert!((l(c) - 0.5).abs() < 1e-6 && c.0 > c.1 && c.1 > c.2);
        let c = blend_rgb(Luminosity, 0.8, 0.3, 0.2, 0.25, 0.25, 0.25);
        assert!((l(c) - 0.25).abs() < 1e-6 && c.0 > c.1);
        let c = blend_rgb(Hue, 0.8, 0.3, 0.2, 0.5, 0.5, 0.5);
        let want = l((0.8, 0.3, 0.2));
        assert!(
            (c.0 - want).abs() < 1e-6 && (c.1 - want).abs() < 1e-6 && (c.2 - want).abs() < 1e-6
        );
        let c = blend_rgb(Saturation, 0.8, 0.3, 0.2, 0.9, 0.9, 0.9);
        assert!((c.0 - c.1).abs() < 1e-6 && (c.1 - c.2).abs() < 1e-6);
        let c = blend_rgb(Hue, 0.8, 0.2, 0.2, 0.1, 0.1, 0.9);
        assert!(c.2 > c.0 && (l(c) - l((0.8, 0.2, 0.2))).abs() < 1e-6);
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
        // 合成の式の値（不透明・半透明・下が透明・モード）
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
        // 個数: レイヤーに付けられる 26 と、グループだけの PassThrough。その先の番号は無い
        assert_eq!(stored.len(), 27);
        assert_eq!(BlendMode::LAYER_MODES.len(), 26);
        assert_eq!(
            BlendMode::LAYER_MODES.to_vec(),
            stored[..26].iter().map(|s| s.0).collect::<Vec<_>>(),
            "レイヤーのモードは番号の順"
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
