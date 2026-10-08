//! ランプの隣り合う色の分岐点の間の混ぜ方（混色モード）と、知覚的な混色の輝度の補正。
//!
//! CLIP STUDIO PAINT のグラデーションの混色モードの考え方（通常 = 昔の混ぜ方、知覚的 = 鮮やかで絵の具に近い混ぜ方、リニア）と、
//! 輝度の補正 5 段階（知覚的のときだけ。補正が強いほど混ぜた色が明るくなる）に合わせた**このアプリの式**で、CLIP STUDIO や Photoshop の
//! 同名のモードと画素まで一致するとは言わない（あちらの式は公開されていない）。
//!
//! - [`MixMode::Standard`]: sRGB の 8 bit の値をそのまま線形に補間する。これまでの（版 24 までの）混ぜ方で、PSD のグラデーションと同じ。
//! - [`MixMode::Linear`]: sRGB を線形の光へ戻して補間し、sRGB へ戻す（暗い色どうしを混ぜても沈まない）。
//! - [`MixMode::Perceptual`]: 線形の光から Oklab（Björn Ottosson）へ移して L・a・b を補間し、戻す。色相が濁りにくく、補色どうしの
//!   途中が灰色へ沈む量は Oklab の a・b の補間が決める。補間した色が sRGB の外へ出たら、各チャンネルを 0〜1 に収める（彩度を残す
//!   ガマットの写像はしない）。
//!
//! 輝度の補正は、知覚的の混色で「両端の彩度の混ぜ合わせ」より実際の彩度が落ちた量（ΔC = (1−t)·C0 + t·C1 − C）に比例して L を持ち上げる:
//! `L' = min(1, L + k·ΔC)`、k は段階ごとに 0・0.25・0.5・0.75・1。補色どうしの真ん中のように彩度が落ちる所ほど明るくなり、端（t = 0 と 1）と、
//! 彩度が落ちない混ぜ（同じ色相どうし）では何も変わらない。
//!
//! **式は + − × ÷ と sqrt だけ**（OS の数学の関数 `powf`・`cbrt`・`hypot` を使わない）。分岐点の色の線形の光は 256 値の表（sRGB の式の値。
//! 作ったのは glibc の `pow`）、立方根は二分の一の 3 乗の段で [1/8, 1] へ寄せてからのニュートン法 4 回（正しく丸めた値から 1 ULP 以内）、
//! `c^(1/2.4)` は `r·√√r`（r = ∛c）、彩度は `√(a² + b²)`。だから画素ごとの式（[`mix`]）とレーンの式（[`mix_lanes`]）は同じ演算を同じ順に
//! 並べられ、**スカラー・SSE4.1・AVX2・NEON で同じバイト**になり、OS の数学の関数の違い（Windows と Linux）にも左右されない。分岐点ごとの値
//! （線形の光・Oklab・彩度）は [`StopColor`] に前もって作り、画素ごとには計算しない。
use crate::math::clamp01;
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use crate::math::simd::{self, Lanes};
use crate::Rgba8;

/// 色の混ぜ方。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MixMode {
    /// sRGB の値を線形に補間する（版 24 までの混ぜ方）。
    #[default]
    Standard,
    /// 線形の光で補間する。
    Linear,
    /// Oklab で補間する（輝度の補正つき）。
    Perceptual,
}

impl MixMode {
    pub const ALL: [Self; 3] = [Self::Standard, Self::Perceptual, Self::Linear];

    /// 保存の値（0 通常・1 知覚的・2 リニア）。
    pub fn index(self) -> u8 {
        match self {
            Self::Standard => 0,
            Self::Perceptual => 1,
            Self::Linear => 2,
        }
    }

    pub fn from_index(index: i64) -> Option<Self> {
        match index {
            0 => Some(Self::Standard),
            1 => Some(Self::Perceptual),
            2 => Some(Self::Linear),
            _ => None,
        }
    }
}

/// 輝度の補正の強さ（5 段階。知覚的な混色のときだけ働く）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LuminanceCorrection {
    None,
    Low,
    Medium,
    High,
    Max,
}

impl Default for LuminanceCorrection {
    /// CLIP STUDIO の知覚的の既定に合わせて「高」。
    fn default() -> Self {
        Self::High
    }
}

impl LuminanceCorrection {
    pub const ALL: [Self; 5] = [Self::None, Self::Low, Self::Medium, Self::High, Self::Max];

    /// 保存の値（0〜4）。
    pub fn index(self) -> u8 {
        self as u8
    }

    pub fn from_index(index: i64) -> Option<Self> {
        usize::try_from(index)
            .ok()
            .and_then(|i| Self::ALL.get(i).copied())
    }

    /// 彩度の落ちた量に掛ける係数（0・0.25・0.5・0.75・1）。
    pub fn strength(self) -> f64 {
        f64::from(self.index()) * 0.25
    }
}

/// sRGB の 8 bit の値から線形の光（`c ≤ 0.04045` は `c / 12.92`、ほかは `((c + 0.055) / 1.055)^2.4`、c = 値 / 255）の f64 のビット。
/// OS の `pow` の違いに左右されないように表で持つ（値は glibc の `pow` で作った。試験が式と 1 ULP 以内を確かめる）。
#[rustfmt::skip]
const SRGB_TO_LINEAR: [u64; 256] = [
    0x0000000000000000, 0x3f33e45677c176f7, 0x3f43e45677c176f7, 0x3f4dd681b3a23272,
    0x3f53e45677c176f7, 0x3f58dd6c15b1d4b4, 0x3f5dd681b3a23272, 0x3f6167cba8c94818,
    0x3f63e45677c176f7, 0x3f6660e146b9a5d5, 0x3f68dd6c15b1d4b4, 0x3f6b6a31b5259c99,
    0x3f6e1e31d70c99dd, 0x3f707c38bf8583a9, 0x3f71fcc2beed6421, 0x3f7390ffaf95e279,
    0x3f753936cc7bc928, 0x3f76f5addb50c915, 0x3f78c6a94031b561, 0x3f7aac6c0fb97351,
    0x3f7ca7381f9f602b, 0x3f7eb74e160978d0, 0x3f806e76bbda92b8, 0x3f818c2a5a8a8044,
    0x3f82b4e09b3f0ae3, 0x3f83e8b7b3bde965, 0x3f8527cd60af8b85, 0x3f86723eea8d3709,
    0x3f87c8292a3db6b3, 0x3f8929a88d67b521, 0x3f8a96d91a8016bd, 0x3f8c0fd67499fab6,
    0x3f8d94bbdefd740e, 0x3f8f25a44089883f, 0x3f9061551372c694, 0x3f9135f3e4c2cce2,
    0x3f9210bb8642b172, 0x3f92f1b8c1ae46bd, 0x3f93d8f839b79c0b, 0x3f94c6866b3e9fa4,
    0x3f95ba6fae794313, 0x3f96b4c0380d2dee, 0x3f97b5841a1bf3ac, 0x3f98bcc74542addb,
    0x3f99ca95898dc8b5, 0x3f9adefa9761c020, 0x3f9bfa0200597bd9, 0x3f9d1bb7381aec1f,
    0x3f9e442595227bca, 0x3f9f73585185e1b5, 0x3fa054ad45d76878, 0x3fa0f31ba386ff26,
    0x3fa194fcb663747b, 0x3fa23a55e62a662a, 0x3fa2e32c8e148d11, 0x3fa38f85fd21eacf,
    0x3fa43f67766310ff, 0x3fa4f2d6313fa8d0, 0x3fa5a9d759ba5ed0, 0x3fa6647010b254ee,
    0x3fa722a56c2239ee, 0x3fa7e47c775d2427, 0x3fa8a9fa33494b07, 0x3fa973239698b9cc,
    0x3faa3ffd8e001389, 0x3fab108cfc6b7fbc, 0x3fabe4d6bb31d522, 0x3facbcdf9a4616f2,
    0x3fad98ac60675833, 0x3fae7841cb4f16df, 0x3faf5ba48fde2048, 0x3fb0216cad240765,
    0x3fb096f2671eb815, 0x3fb10e65c38a5192, 0x3fb187c90bf8bce2, 0x3fb2031e85f5d6da,
    0x3fb28068731a1952, 0x3fb2ffa9111cb94b, 0x3fb380e299e53f92, 0x3fb40417439ca10f,
    0x3fb4894940bddbfb, 0x3fb5107ac0261e59, 0x3fb599aded247aac, 0x3fb624e4ef892ed4,
    0x3fb6b221ebb4817e, 0x3fb7416702a539d1, 0x3fb7d2b65206b527, 0x3fb86611f43e9e6a,
    0x3fb8fb7c007a4a70, 0x3fb992f68abbbc89, 0x3fba2c83a3e6566d, 0x3fbac82559cb3644,
    0x3fbb65ddb7354604, 0x3fbc05aec3f4fe5e, 0x3fbca79a84ebe030, 0x3fbd4ba2fc17a6a5,
    0x3fbdf1ca289d34b8, 0x3fbe9a1206d34003, 0x3fbf447c904cbb4e, 0x3fbff10bbbe302c2,
    0x3fc04fe0bedfe5f1, 0x3fc0a84fe3b36d8f, 0x3fc101d443dfc06f, 0x3fc15c6ed58eefdf,
    0x3fc1b8208da5fef0, 0x3fc214ea5fc9514a, 0x3fc272cd3e610123, 0x3fc2d1ca1a9d1cfb,
    0x3fc331e1e479cdf5, 0x3fc393158ac3674e, 0x3fc3f565fb1a5fd5, 0x3fc458d421f735df,
    0x3fc4bd60eaae3e73, 0x3fc5230d3f736034, 0x3fc589da095dbaa1, 0x3fc5f1c8306b3a3c,
    0x3fc65ad89b841a2b, 0x3fc6c50c307e53bf, 0x3fc73063d420fc80, 0x3fc79ce06a279303,
    0x3fc80a82d5453b5d, 0x3fc8794bf727eb3f, 0x3fc8e93cb07b8679, 0x3fc95a55e0ecec0b,
    0x3fc9cc98672cf47e, 0x3fca400520f3619c, 0x3fcab49ceb01c003, 0x3fcb2a60a1263b0a,
    0x3fcba1511e3e632d, 0x3fcc196f3c39e76f, 0x3fcc92bbd41d41fe, 0x3fcd0d37be045851,
    0x3fcd88e3d1250f68, 0x3fce05c0e3d1d3e0, 0x3fce83cfcb7c16f0, 0x3fcf03115cb6bfd3,
    0x3fcf83866b38924d, 0x3fd00297e4ef4553, 0x3fd044072557177a, 0x3fd086115f6beb3a,
    0x3fd0c8b6fb5c735e, 0x3fd10bf860ef039a, 0x3fd14fd5f782a5a6, 0x3fd1945026102997,
    0x3fd1d967532b31b1, 0x3fd21f1be50339e7, 0x3fd2656e41649ae3, 0x3fd2ac5ecdb988f8,
    0x3fd2f3edef0b0ed8, 0x3fd33c1c0a020438, 0x3fd384e982e800b1, 0x3fd3ce56bda84a81,
    0x3fd418641dd0c1bc, 0x3fd463120692c7af, 0x3fd4ae60dac4229d, 0x3fd4fa50fcdfde15,
    0x3fd546e2cf0727a9, 0x3fd59416b3022858, 0x3fd5e1ed0a40daab, 0x3fd6306635dbdd7b,
    0x3fd67f82969543a2, 0x3fd6cf428cd96079, 0x3fd71fa678bf915d, 0x3fd770aeba0b042a,
    0x3fd7c25bb02b7ac5, 0x3fd814adba3e0bd9, 0x3fd867a5370de0b1, 0x3fd8bb428514f067,
    0x3fd90f86027cb84e, 0x3fd964700d1ef1b1, 0x3fd9ba0102864521, 0x3fda10393feefafd,
    0x3fda67192247a9be, 0x3fdabea10631e195, 0x3fdb16d14802d5ca, 0x3fdb6faa43c403bb,
    0x3fdbc92c5533d785, 0x3fdc2357d7c64e5d, 0x3fdc7e2d26a596de, 0x3fdcd9ac9cb2aef2,
    0x3fdd35d69485ffc5, 0x3fdd92ab686ff782, 0x3fddf02b7279a10d, 0x3fde4e570c6539c5,
    0x3fdead2e8faec526, 0x3fdf0cb2558c9ea4, 0x3fdf6ce2b6f00983, 0x3fdfcdc00c85bec2,
    0x3fe017a5575b3cb2, 0x3fe048c17ad3c04b, 0x3fe07a349c9d9837, 0x3fe0abfee888c050,
    0x3fe0de208a4444c8, 0x3fe11099ad5e83eb, 0x3fe1436a7d456eef, 0x3fe176932546ca12,
    0x3fe1aa13d0906bda, 0x3fe1ddecaa307b85, 0x3fe2121ddd15aece, 0x3fe246a7940f86d1,
    0x3fe27b89f9ce8c4b, 0x3fe2b0c538e48b07, 0x3fe2e6597bc4cca0, 0x3fe31c46ecc4528d,
    0x3fe3528db61a0f73, 0x3fe3892e01df1fcc, 0x3fe3c027fa0f01eb, 0x3fe3f77bc887cd3b,
    0x3fe42f29970a68f8, 0x3fe467318f3ac22d, 0x3fe49f93daa00113, 0x3fe4d850a2a4bde1,
    0x3fe51168109734e5, 0x3fe54ada4da97a1b, 0x3fe584a782f1ac23, 0x3fe5becfd96a2698,
    0x3fe5f95379f1b3ed, 0x3fe634328d4bbe97, 0x3fe66f6d3c2081cf, 0x3fe6ab03aefd39aa,
    0x3fe6e6f60e5452b1, 0x3fe72344827d98f6, 0x3fe75fef33b6669b, 0x3fe79cf64a21d1e2,
    0x3fe7da59edc8dab0, 0x3fe8181a469a9787, 0x3fe856377c6c6224, 0x3fe894b1b6fa0377,
    0x3fe8d3891de5df49, 0x3fe912bdd8b91f45, 0x3fe952500ee3dda5, 0x3fe9923fe7bd4f67,
    0x3fe9d28d8a83edfc, 0x3fea13391e5da09f, 0x3fea5442ca57e52e, 0x3fea95aab567f88f,
    0x3fead771066afec2, 0x3feb1995e4262a69, 0x3feb5c197546e3f8, 0x3feb9efbe062f086,
    0x3febe23d4bf8981b, 0x3fec25ddde6ecbbb, 0x3fec69ddbe154af1, 0x3fecae3d1124c90b,
    0x3fecf2fbfdbf11f1, 0x3fed381aa9ef2e82, 0x3fed7d993ba988d4, 0x3fedc377d8cc0fd5,
    0x3fee09b6a71e5aa6, 0x3fee5055cc51cbb4, 0x3fee97556e01b351, 0x3feedeb5b1b37216,
    0x3fef2676bcd69ade, 0x3fef6e98b4c51466, 0x3fefb71bbec33ab2, 0x3ff0000000000000,
];

fn to_linear(byte: u8) -> f64 {
    f64::from_bits(SRGB_TO_LINEAR[byte as usize])
}

/// `if a < b { a } else { b }`（レーンの `min` と同じ。f64::min とは符号つきの 0 の扱いが違い得るので使わない）
#[inline(always)]
fn min(a: f64, b: f64) -> f64 {
    if a < b {
        a
    } else {
        b
    }
}
/// `if a > b { a } else { b }`（レーンの `max` と同じ）
#[inline(always)]
fn max(a: f64, b: f64) -> f64 {
    if a > b {
        a
    } else {
        b
    }
}

/// 立方根の初めの値（[1/8, 1] の 3 点のチェビシェフの節で cbrt を通る 2 次式。相対の誤差 4% 以内）。
const CBRT_GUESS: [f64; 3] = [
    0.406_897_374_098_315_17,
    0.945_007_553_967_131_7,
    -0.357_079_896_434_314_8,
];
/// ニュートン法の回数（初めの 4% から、4 回で正しく丸めた値の 1 ULP 以内）。
const CBRT_STEPS: usize = 4;

/// 0 以上の x の立方根（+ − × ÷ だけ）。8 倍・1/8 倍（立方根は 2 倍・1/2 倍。どれも丸めの無い演算）で [1/8, 1] へ寄せ、ニュートン法を 4 回。
/// 画素ごとに呼ぶ範囲（`to_srgb` の 0.0031308〜1）では寄せは 2 回までで、レーンの式（[`cbrt_lanes`]）と同じ値になる。
fn cbrt(x: f64) -> f64 {
    if x <= 0. {
        return 0.;
    }
    let (mut v, mut scale) = (x, 1.);
    while v > 1. {
        v *= 0.125;
        scale *= 2.;
    }
    while v < 0.125 {
        v *= 8.;
        scale *= 0.5;
    }
    let [c0, c1, c2] = CBRT_GUESS;
    let mut y = c0 + v * (c1 + v * c2);
    for _ in 0..CBRT_STEPS {
        y = (2. * y + v / (y * y)) / 3.;
    }
    y * scale
}

/// 線形の光（0〜1 の外は収める）から sRGB の 0〜255 の実数。`c^(1/2.4) = c^(5/12) = r·√√r`（r = ∛c）。
fn to_srgb(linear: f64) -> f64 {
    let c = clamp01(linear);
    let v = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        let r = cbrt(c);
        1.055 * (r * r.sqrt().sqrt()) - 0.055
    };
    v * 255.0
}

/// 線形の sRGB から Oklab（L, a, b）。
pub(crate) fn linear_to_oklab([r, g, b]: [f64; 3]) -> [f64; 3] {
    let l = cbrt(0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b);
    let m = cbrt(0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b);
    let s = cbrt(0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b);
    [
        0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
    ]
}

/// Oklab から線形の sRGB（範囲の外も出る）。
pub(crate) fn oklab_to_linear([lightness, a, b]: [f64; 3]) -> [f64; 3] {
    let cube = |v: f64| v * v * v;
    let l = cube(lightness + 0.396_337_777_4 * a + 0.215_803_757_3 * b);
    let m = cube(lightness - 0.105_561_345_8 * a - 0.063_854_172_8 * b);
    let s = cube(lightness - 0.089_484_177_5 * a - 1.291_485_548_0 * b);
    [
        4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
        -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
        -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701_0 * s,
    ]
}

/// Oklab の a・b からの彩度。
fn chroma(a: f64, b: f64) -> f64 {
    (a * a + b * b).sqrt()
}

/// 色の分岐点の色を、混色の式が読む形に前もって直したもの（分岐点ごとに 1 度。画素ごとには計算しない）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StopColor {
    /// sRGB の値（0〜255）。
    pub srgb: [f64; 3],
    /// 線形の光。
    pub linear: [f64; 3],
    pub oklab: [f64; 3],
    /// Oklab の彩度（√(a² + b²)）。
    pub chroma: f64,
}

impl StopColor {
    pub(crate) fn new(c: Rgba8) -> Self {
        let linear = [to_linear(c.r), to_linear(c.g), to_linear(c.b)];
        let oklab = linear_to_oklab(linear);
        StopColor {
            srgb: [f64::from(c.r), f64::from(c.g), f64::from(c.b)],
            linear,
            oklab,
            chroma: chroma(oklab[1], oklab[2]),
        }
    }
}

/// 色 `a`（重み 0）から `b`（重み 1）へ、重み `t` の所の色（R・G・B を 0〜255 の実数で。丸めは呼び手）。
/// `Standard` は昔のままの sRGB の値の線形補間。両端（t ≤ 0・t ≥ 1）は、どの混ぜ方でも端の色そのもの。
pub(crate) fn mix(
    a: &StopColor,
    b: &StopColor,
    t: f64,
    mode: MixMode,
    correction: LuminanceCorrection,
) -> [f64; 3] {
    let (ca, cb) = (a.srgb, b.srgb);
    if t <= 0.0 {
        return ca;
    }
    if t >= 1.0 {
        return cb;
    }
    match mode {
        MixMode::Standard => std::array::from_fn(|i| ca[i] + (cb[i] - ca[i]) * t),
        MixMode::Linear => {
            let (la, lb) = (a.linear, b.linear);
            std::array::from_fn(|i| to_srgb(la[i] + (lb[i] - la[i]) * t))
        }
        MixMode::Perceptual => {
            let (oa, ob) = (a.oklab, b.oklab);
            let mut m: [f64; 3] = std::array::from_fn(|i| oa[i] + (ob[i] - oa[i]) * t);
            let k = correction.strength();
            if k > 0.0 {
                let lost = (1.0 - t) * a.chroma + t * b.chroma - chroma(m[1], m[2]);
                m[0] = min(m[0] + k * max(lost, 0.0), 1.0);
            }
            oklab_to_linear(m).map(to_srgb)
        }
    }
}

// ───────── レーン（N 画素を 1 組で。画素ごとの式と同じ演算を同じ順に） ─────────

/// N 本のレーンの分岐点の色（レーンごとに別の区間の端でよい）。
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[derive(Clone, Copy)]
pub(crate) struct StopLanes<V: Lanes> {
    pub srgb: [V::F; 3],
    pub linear: [V::F; 3],
    pub oklab: [V::F; 3],
    pub chroma: V::F,
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
impl<V: Lanes> StopLanes<V> {
    /// レーン l に `stop(l)` の色を集める。
    ///
    /// # Safety
    /// `V` の命令を持つ CPU で、その命令を有効にした `#[target_feature]` 付きの入口の中から呼ぶ。
    #[inline(always)]
    pub(crate) unsafe fn gather<'a>(stop: impl Fn(usize) -> &'a StopColor) -> Self {
        StopLanes {
            srgb: [
                V::from_fn(|l| stop(l).srgb[0]),
                V::from_fn(|l| stop(l).srgb[1]),
                V::from_fn(|l| stop(l).srgb[2]),
            ],
            linear: [
                V::from_fn(|l| stop(l).linear[0]),
                V::from_fn(|l| stop(l).linear[1]),
                V::from_fn(|l| stop(l).linear[2]),
            ],
            oklab: [
                V::from_fn(|l| stop(l).oklab[0]),
                V::from_fn(|l| stop(l).oklab[1]),
                V::from_fn(|l| stop(l).oklab[2]),
            ],
            chroma: V::from_fn(|l| stop(l).chroma),
        }
    }
}

/// [`cbrt`] の N 本ぶん。入力は 0.0031308〜1（`to_srgb` の冪の側）の範囲で同じ値。範囲の外（`select` で捨てる側）でも NaN にはならない。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn cbrt_lanes<V: Lanes>(x: V::F) -> V::F {
    let (eighth, eight, half) = (V::splat(0.125), V::splat(8.), V::splat(0.5));
    let (mut v, mut scale) = (x, V::splat(1.));
    // 0.0031308·8² ≥ 1/8 なので 2 回で足りる（3 回目は 0.0031308 より小さい値だけに効き、それは捨てる側）
    for _ in 0..3 {
        let low = V::lt(v, eighth);
        v = V::select(low, V::mul(v, eight), v);
        scale = V::select(low, V::mul(scale, half), scale);
    }
    let [c0, c1, c2] = CBRT_GUESS;
    let mut y = V::add(
        V::splat(c0),
        V::mul(v, V::add(V::splat(c1), V::mul(v, V::splat(c2)))),
    );
    let (two, three) = (V::splat(2.), V::splat(3.));
    for _ in 0..CBRT_STEPS {
        y = V::div(V::add(V::mul(two, y), V::div(v, V::mul(y, y))), three);
    }
    V::mul(y, scale)
}

/// [`to_srgb`] の N 本ぶん。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn to_srgb_lanes<V: Lanes>(linear: V::F) -> V::F {
    let c = simd::clamp01::<V>(linear);
    // 0 の立方根の側（捨てる側）でニュートン法の割り算が 0 にならないよう、捨てる側の値は 1 にしておく
    let low = V::le(c, V::splat(0.003_130_8));
    let r = cbrt_lanes::<V>(V::select(low, V::splat(1.), c));
    let high = V::sub(
        V::mul(V::splat(1.055), V::mul(r, V::sqrt(V::sqrt(r)))),
        V::splat(0.055),
    );
    V::mul(
        V::select(low, V::mul(c, V::splat(12.92)), high),
        V::splat(255.0),
    )
}

/// `p + (q − p)·t`
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn lerp<V: Lanes>(p: V::F, q: V::F, t: V::F) -> V::F {
    V::add(p, V::mul(V::sub(q, p), t))
}

/// 3 成分ぶんの [`lerp`]。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[allow(clippy::needless_range_loop)] // 3 つの並びを同じ番号で読む
unsafe fn lerp3<V: Lanes>(p: [V::F; 3], q: [V::F; 3], t: V::F) -> [V::F; 3] {
    let mut out = p;
    for i in 0..3 {
        out[i] = lerp::<V>(p[i], q[i], t);
    }
    out
}

/// `v·v·v`（左から）
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn cube<V: Lanes>(v: V::F) -> V::F {
    V::mul(V::mul(v, v), v)
}

/// `k·v`
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn scale<V: Lanes>(k: f64, v: V::F) -> V::F {
    V::mul(V::splat(k), v)
}

/// [`mix`] の N 本ぶんのうち、リニア・知覚的の混ぜ（`to_srgb` の冪を含む重い式）。結果は R・G・B の 0〜255 の実数（両端の扱いは [`mix_lanes`]）。
/// 2 つの混ぜ方は、線形の光までを別々に出し、sRGB への変換（[`to_srgb_lanes`]）の 3 回を共有する。`Standard` は `mix_lanes` が直に補間するので
/// ここへは来ないが、来ても同じ値を返す。
/// レーンの演算を含む式の中にクロージャを置かない（`math::simd` の決まり）。
///
/// `inline(always)` はここまで（この関数と、中の `to_srgb_lanes`・`cbrt_lanes`）。呼ぶのは道ごとの入口（[`MixLanes::mix_curved`]）の中だけで、
/// ramp・合成の入口へは畳み込まない（畳むと、合成 7 種 × 2 か所 × 2 つの道の全部に同じ式が写されて、ビルドの時間と命令の置き場が増える）。
///
/// # Safety
/// `V` の命令を持つ CPU で、その命令を有効にした `#[target_feature]` 付きの入口の中から呼ぶ。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn mix_curved_lanes<V: Lanes>(
    a: &StopLanes<V>,
    b: &StopLanes<V>,
    t: V::F,
    mode: MixMode,
    correction: LuminanceCorrection,
) -> [V::F; 3] {
    // 線形の光（sRGB への変換の前）
    let linear = match mode {
        MixMode::Standard => return lerp3::<V>(a.srgb, b.srgb, t),
        MixMode::Linear => lerp3::<V>(a.linear, b.linear, t),
        MixMode::Perceptual => {
            let mut m = lerp3::<V>(a.oklab, b.oklab, t);
            let k = correction.strength();
            if k > 0.0 {
                let c = V::sqrt(V::add(V::mul(m[1], m[1]), V::mul(m[2], m[2])));
                let lost = V::sub(
                    V::add(
                        V::mul(V::sub(V::splat(1.0), t), a.chroma),
                        V::mul(t, b.chroma),
                    ),
                    c,
                );
                m[0] = V::min(
                    V::add(m[0], scale::<V>(k, V::max(lost, V::splat(0.0)))),
                    V::splat(1.0),
                );
            }
            let l = cube::<V>(V::add(
                V::add(m[0], scale::<V>(0.396_337_777_4, m[1])),
                scale::<V>(0.215_803_757_3, m[2]),
            ));
            let mm = cube::<V>(V::sub(
                V::sub(m[0], scale::<V>(0.105_561_345_8, m[1])),
                scale::<V>(0.063_854_172_8, m[2]),
            ));
            let s = cube::<V>(V::sub(
                V::sub(m[0], scale::<V>(0.089_484_177_5, m[1])),
                scale::<V>(1.291_485_548_0, m[2]),
            ));
            [
                V::add(
                    V::sub(
                        scale::<V>(4.076_741_662_1, l),
                        scale::<V>(3.307_711_591_3, mm),
                    ),
                    scale::<V>(0.230_969_929_2, s),
                ),
                V::sub(
                    V::add(
                        scale::<V>(-1.268_438_004_6, l),
                        scale::<V>(2.609_757_401_1, mm),
                    ),
                    scale::<V>(0.341_319_396_5, s),
                ),
                V::add(
                    V::sub(
                        scale::<V>(-0.004_196_086_3, l),
                        scale::<V>(0.703_418_614_7, mm),
                    ),
                    scale::<V>(1.707_614_701_0, s),
                ),
            ]
        }
    };
    [
        to_srgb_lanes::<V>(linear[0]),
        to_srgb_lanes::<V>(linear[1]),
        to_srgb_lanes::<V>(linear[2]),
    ]
}

/// リニア・知覚的の混ぜの入口（[`mix_curved_lanes`]）を、道ごとに 1 つずつ持つ型。ramp・合成の道は `V: MixLanes` で受け、
/// 重い式の写しを道ごとに 1 つ（AVX2・SSE4.1・NEON）にとどめる。入口は `inline(never)` で、`V::F`・[`StopLanes`] は参照・メモリ渡しになる
/// （1 回の呼びの手間は、中の `to_srgb` の冪 3〜6 回に比べて小さい）。
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub(crate) trait MixLanes: Lanes {
    /// [`mix_curved_lanes`]。
    ///
    /// # Safety
    /// `Self` の命令を持つ CPU で、その命令を有効にした `#[target_feature]` 付きの入口の中から呼ぶ。
    unsafe fn mix_curved(
        a: &StopLanes<Self>,
        b: &StopLanes<Self>,
        t: Self::F,
        mode: MixMode,
        correction: LuminanceCorrection,
    ) -> [Self::F; 3];
}
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
#[inline(never)]
unsafe fn mix_curved_avx2(
    a: &StopLanes<simd::Avx2>,
    b: &StopLanes<simd::Avx2>,
    t: <simd::Avx2 as Lanes>::F,
    mode: MixMode,
    correction: LuminanceCorrection,
) -> [<simd::Avx2 as Lanes>::F; 3] {
    mix_curved_lanes::<simd::Avx2>(a, b, t, mode, correction)
}
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
#[inline(never)]
unsafe fn mix_curved_sse41(
    a: &StopLanes<simd::Sse41>,
    b: &StopLanes<simd::Sse41>,
    t: <simd::Sse41 as Lanes>::F,
    mode: MixMode,
    correction: LuminanceCorrection,
) -> [<simd::Sse41 as Lanes>::F; 3] {
    mix_curved_lanes::<simd::Sse41>(a, b, t, mode, correction)
}
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline(never)]
unsafe fn mix_curved_neon(
    a: &StopLanes<simd::Neon>,
    b: &StopLanes<simd::Neon>,
    t: <simd::Neon as Lanes>::F,
    mode: MixMode,
    correction: LuminanceCorrection,
) -> [<simd::Neon as Lanes>::F; 3] {
    mix_curved_lanes::<simd::Neon>(a, b, t, mode, correction)
}
#[cfg(target_arch = "x86_64")]
impl MixLanes for simd::Avx2 {
    #[inline(always)]
    unsafe fn mix_curved(
        a: &StopLanes<Self>,
        b: &StopLanes<Self>,
        t: Self::F,
        mode: MixMode,
        correction: LuminanceCorrection,
    ) -> [Self::F; 3] {
        mix_curved_avx2(a, b, t, mode, correction)
    }
}
#[cfg(target_arch = "x86_64")]
impl MixLanes for simd::Sse41 {
    #[inline(always)]
    unsafe fn mix_curved(
        a: &StopLanes<Self>,
        b: &StopLanes<Self>,
        t: Self::F,
        mode: MixMode,
        correction: LuminanceCorrection,
    ) -> [Self::F; 3] {
        mix_curved_sse41(a, b, t, mode, correction)
    }
}
#[cfg(target_arch = "aarch64")]
impl MixLanes for simd::Neon {
    #[inline(always)]
    unsafe fn mix_curved(
        a: &StopLanes<Self>,
        b: &StopLanes<Self>,
        t: Self::F,
        mode: MixMode,
        correction: LuminanceCorrection,
    ) -> [Self::F; 3] {
        mix_curved_neon(a, b, t, mode, correction)
    }
}

/// [`mix`] の N 本ぶん（`Linear`・`Perceptual`。`Standard` は直に補間する）。結果は R・G・B の 0〜255 の実数。
/// 重い式は道ごとの入口 [`MixLanes::mix_curved`] に出してあり、ここは呼びと両端の選び方だけ。
///
/// # Safety
/// `V` の命令を持つ CPU で、その命令を有効にした `#[target_feature]` 付きの入口の中から呼ぶ。
#[inline(always)]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[allow(clippy::needless_range_loop)] // 3 つの並び（端の色）を同じ番号で読む
pub(crate) unsafe fn mix_lanes<V: MixLanes>(
    a: &StopLanes<V>,
    b: &StopLanes<V>,
    t: V::F,
    mode: MixMode,
    correction: LuminanceCorrection,
) -> [V::F; 3] {
    let mut out = if mode == MixMode::Standard {
        lerp3::<V>(a.srgb, b.srgb, t)
    } else {
        V::mix_curved(a, b, t, mode, correction)
    };
    // 両端（t ≤ 0・t ≥ 1）は端の色そのもの
    let (zero, one) = (V::splat(0.0), V::splat(1.0));
    let (low, high) = (V::le(t, zero), V::ge(t, one));
    for i in 0..3 {
        out[i] = V::select(low, a.srgb[i], V::select(high, b.srgb[i], out[i]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(r: u8, g: u8, b: u8) -> StopColor {
        StopColor::new(Rgba8::new(r, g, b, 255))
    }
    fn mix(
        a: StopColor,
        b: StopColor,
        t: f64,
        mode: MixMode,
        correction: LuminanceCorrection,
    ) -> [f64; 3] {
        super::mix(&a, &b, t, mode, correction)
    }

    #[test]
    fn oklab_round_trips_and_has_the_known_anchors() {
        for rgb in [
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [1.0, 0.0, 0.0],
            [0.2, 0.5, 0.8],
        ] {
            let back = oklab_to_linear(linear_to_oklab(rgb));
            for i in 0..3 {
                assert!((back[i] - rgb[i]).abs() < 1e-6, "{rgb:?} {back:?}");
            }
        }
        // 白は L = 1・a = b = 0、黒は 0
        let white = linear_to_oklab([1.0, 1.0, 1.0]);
        assert!((white[0] - 1.0).abs() < 1e-4 && white[1].abs() < 1e-4 && white[2].abs() < 1e-4);
        assert!(linear_to_oklab([0.0; 3]).iter().all(|v| v.abs() < 1e-9));
        // 赤は Oklab の公開の値（L 0.628、a 0.2249、b 0.1258）
        let red = linear_to_oklab([1.0, 0.0, 0.0]);
        assert!((red[0] - 0.6280).abs() < 1e-3, "{red:?}");
        assert!((red[1] - 0.2249).abs() < 1e-3, "{red:?}");
        assert!((red[2] - 0.1258).abs() < 1e-3, "{red:?}");
    }

    #[test]
    fn every_mode_returns_the_end_colours_exactly() {
        let (a, b) = (c(12, 200, 99), c(250, 3, 180));
        for mode in MixMode::ALL {
            for correction in LuminanceCorrection::ALL {
                assert_eq!(mix(a, b, 0.0, mode, correction), [12.0, 200.0, 99.0]);
                assert_eq!(mix(a, b, 1.0, mode, correction), [250.0, 3.0, 180.0]);
            }
        }
    }

    #[test]
    fn standard_is_the_old_straight_srgb_interpolation() {
        let m = mix(
            c(0, 100, 255),
            c(100, 200, 55),
            0.25,
            MixMode::Standard,
            LuminanceCorrection::High,
        );
        assert_eq!(m, [25.0, 125.0, 205.0]);
    }

    #[test]
    fn linear_light_mixing_is_brighter_than_srgb_mixing_for_black_and_white() {
        let m = mix(
            c(0, 0, 0),
            c(255, 255, 255),
            0.5,
            MixMode::Linear,
            LuminanceCorrection::None,
        );
        // 線形の 0.5 は sRGB で約 188
        assert!((m[0] - 188.0).abs() < 1.0, "{m:?}");
        assert!(m[0] > 127.5);
    }

    #[test]
    fn perceptual_mixing_keeps_a_grey_ramp_grey_and_lighter_correction_lightens_complements() {
        // 灰色どうしは a = b = 0 のまま（色が付かない）
        let g = mix(
            c(20, 20, 20),
            c(230, 230, 230),
            0.5,
            MixMode::Perceptual,
            LuminanceCorrection::None,
        );
        assert!(
            (g[0] - g[1]).abs() < 0.5 && (g[1] - g[2]).abs() < 0.5,
            "{g:?}"
        );
        // 補色どうし（青と黄）の真ん中は、補正が強いほど明るい
        let (blue, yellow) = (c(20, 40, 220), c(250, 230, 30));
        let lum = |m: [f64; 3]| 0.2126 * m[0] + 0.7152 * m[1] + 0.0722 * m[2];
        let mut last = -1.0;
        for level in LuminanceCorrection::ALL {
            let m = mix(blue, yellow, 0.5, MixMode::Perceptual, level);
            let l = lum(m);
            assert!(l >= last, "{level:?}: {l} < {last}");
            last = l;
        }
        let none = lum(mix(
            blue,
            yellow,
            0.5,
            MixMode::Perceptual,
            LuminanceCorrection::None,
        ));
        let max = lum(mix(
            blue,
            yellow,
            0.5,
            MixMode::Perceptual,
            LuminanceCorrection::Max,
        ));
        assert!(max > none + 5.0, "{none} {max}");
        // 同じ色相どうし（彩度が落ちない）では、補正しても変わらない
        let (dark, light) = (c(40, 0, 0), c(255, 120, 120));
        let a = mix(
            dark,
            light,
            0.5,
            MixMode::Perceptual,
            LuminanceCorrection::None,
        );
        let b = mix(
            dark,
            light,
            0.5,
            MixMode::Perceptual,
            LuminanceCorrection::Max,
        );
        assert!((0..3).all(|i| (a[i] - b[i]).abs() < 12.0), "{a:?} {b:?}");
    }

    #[test]
    fn out_of_gamut_results_are_clamped_to_the_byte_range() {
        for t in [0.1, 0.3, 0.5, 0.7, 0.9] {
            for mode in MixMode::ALL {
                let m = mix(
                    c(0, 255, 255),
                    c(255, 0, 255),
                    t,
                    mode,
                    LuminanceCorrection::Max,
                );
                assert!(
                    m.iter().all(|v| (0.0..=255.0).contains(v)),
                    "{mode:?} {t}: {m:?}"
                );
            }
        }
    }

    #[test]
    fn saved_indices_round_trip_and_unknown_ones_are_refused() {
        for mode in MixMode::ALL {
            assert_eq!(MixMode::from_index(i64::from(mode.index())), Some(mode));
        }
        assert_eq!(MixMode::from_index(3), None);
        assert_eq!(MixMode::from_index(-1), None);
        for level in LuminanceCorrection::ALL {
            assert_eq!(
                LuminanceCorrection::from_index(i64::from(level.index())),
                Some(level)
            );
        }
        assert_eq!(LuminanceCorrection::from_index(5), None);
        assert_eq!(LuminanceCorrection::default(), LuminanceCorrection::High);
    }

    /// 0.4.x の式（OS の `powf`・`cbrt`・`hypot` を使う）。今の式との差を測る参照。
    mod libm {
        use super::super::{LuminanceCorrection, MixMode};
        use crate::math::clamp01;
        fn to_linear(byte: f64) -> f64 {
            let c = byte / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }
        fn to_srgb(linear: f64) -> f64 {
            let c = clamp01(linear);
            let v = if c <= 0.003_130_8 {
                c * 12.92
            } else {
                1.055 * c.powf(1.0 / 2.4) - 0.055
            };
            v * 255.0
        }
        fn linear_to_oklab([r, g, b]: [f64; 3]) -> [f64; 3] {
            let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
            let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
            let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
            [
                0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
                1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
                0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
            ]
        }
        fn oklab_to_linear([lightness, a, b]: [f64; 3]) -> [f64; 3] {
            let l = (lightness + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
            let m = (lightness - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
            let s = (lightness - 0.089_484_177_5 * a - 1.291_485_548_0 * b).powi(3);
            [
                4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
                -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
                -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701_0 * s,
            ]
        }
        pub fn mix(
            ca: [f64; 3],
            cb: [f64; 3],
            t: f64,
            mode: MixMode,
            correction: LuminanceCorrection,
        ) -> [f64; 3] {
            if t <= 0.0 {
                return ca;
            }
            if t >= 1.0 {
                return cb;
            }
            match mode {
                MixMode::Standard => std::array::from_fn(|i| ca[i] + (cb[i] - ca[i]) * t),
                MixMode::Linear => {
                    let (la, lb) = (ca.map(to_linear), cb.map(to_linear));
                    std::array::from_fn(|i| to_srgb(la[i] + (lb[i] - la[i]) * t))
                }
                MixMode::Perceptual => {
                    let oa = linear_to_oklab(ca.map(to_linear));
                    let ob = linear_to_oklab(cb.map(to_linear));
                    let mut m: [f64; 3] = std::array::from_fn(|i| oa[i] + (ob[i] - oa[i]) * t);
                    let k = correction.strength();
                    if k > 0.0 {
                        let chroma = |o: [f64; 3]| o[1].hypot(o[2]);
                        let lost = (1.0 - t) * chroma(oa) + t * chroma(ob) - chroma(m);
                        m[0] = (m[0] + k * lost.max(0.0)).min(1.0);
                    }
                    oklab_to_linear(m).map(to_srgb)
                }
            }
        }
    }

    /// 今の式（+ − × ÷ sqrt だけ）と 0.4.x の式（OS の数学の関数）の、丸めた 8 bit の値の差。分岐点の色の組 × 重みを密に。
    #[test]
    fn libm_free_formulas_match_the_previous_bytes() {
        let round = |v: f64| (v + 0.5).floor() as i32;
        let mut state = 0x1357_9bdfu64;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as u32
        };
        let mut pairs: Vec<(Rgba8, Rgba8)> = Vec::new();
        let edge = [0u8, 1, 2, 10, 11, 127, 128, 254, 255];
        for &p in &edge {
            for &q in &edge {
                pairs.push((
                    Rgba8::new(p, q, 255 - p, 255),
                    Rgba8::new(q, 255 - q, p, 255),
                ));
            }
        }
        for _ in 0..3000 {
            let mut c = || Rgba8::new(next() as u8, next() as u8, next() as u8, 255);
            pairs.push((c(), c()));
        }
        let (mut values, mut differ, mut worst) = (0u64, 0u64, 0);
        for (a, b) in &pairs {
            let (sa, sb) = (StopColor::new(*a), StopColor::new(*b));
            for mode in [MixMode::Linear, MixMode::Perceptual] {
                for correction in [
                    LuminanceCorrection::None,
                    LuminanceCorrection::High,
                    LuminanceCorrection::Max,
                ] {
                    for k in 0..=512 {
                        let t = f64::from(k) / 512.0;
                        let now = super::mix(&sa, &sb, t, mode, correction);
                        let before = libm::mix(sa.srgb, sb.srgb, t, mode, correction);
                        for i in 0..3 {
                            let d = (round(now[i]) - round(before[i])).abs();
                            values += 1;
                            if d > 0 {
                                differ += 1;
                                worst = worst.max(d);
                            }
                        }
                    }
                }
            }
        }
        println!("値 {values}・違う値 {differ}・最大の差 {worst} 段");
        // この機械（glibc）では違う値は 0。OS の数学の関数が違えば、参照の側が丸めの境で 1 段ずれることがある
        assert!(worst <= 1, "最大の差 {worst}");
        assert!(differ * 100_000 <= values, "違う値 {differ} / {values}");
    }

    /// sRGB → 線形の光の表は式の値（OS の `powf` と 1 ULP 以内。glibc ではビットまで同じ）。
    #[test]
    fn srgb_table_is_the_formula() {
        for b in 0..=255u8 {
            let c = f64::from(b) / 255.0;
            let want = if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            };
            let got = to_linear(b);
            assert!(
                (got.to_bits() as i64 - want.to_bits() as i64).abs() <= 1,
                "{b}: {got} {want}"
            );
        }
        assert_eq!(to_linear(0), 0.0);
        assert_eq!(to_linear(255), 1.0);
    }

    /// 立方根は OS の `cbrt` と 2 ULP 以内（どちらも正しく丸めた値から 1 ULP 以内）、0 は 0、寄せる段の境（1/8・1・8 の冪）でも同じ。
    #[test]
    fn cube_root_is_within_two_ulps_of_libm() {
        let mut xs: Vec<f64> = (0..=200_000)
            .map(|i| f64::from(i) / 200_000.0 * 1.02)
            .collect();
        xs.extend((-30..=3).map(|e| 8f64.powi(e)));
        xs.extend((-30..=3).map(|e| 8f64.powi(e) * 0.999_999_9));
        xs.extend([1e-300, 1e-12, 0.003_130_8, 0.04045]);
        for x in xs {
            let (got, want) = (cbrt(x), x.cbrt());
            assert!(
                (got.to_bits() as i64 - want.to_bits() as i64).abs() <= 2,
                "{x}: {got} {want}"
            );
        }
        assert_eq!(cbrt(0.0), 0.0);
    }

    /// `to_srgb` のレーンの式は、画素ごとの式と同じビット（0 と 1 の外・冪と直線の境の前後・密な掃引を、この CPU が持つ全部の道で）。
    #[test]
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    fn srgb_lanes_equal_the_scalar_formula_on_every_simd_level() {
        unsafe fn check<V: Lanes>() {
            let edge = 0.003_130_8f64;
            let mut xs: Vec<f64> = (0..=100_000).map(|i| f64::from(i) / 100_000.0).collect();
            xs.extend([
                -1.0,
                -0.0,
                0.0,
                1.0,
                1.5,
                f64::MIN_POSITIVE,
                edge,
                f64::from_bits(edge.to_bits() + 1),
                f64::from_bits(edge.to_bits() - 1),
                0.125,
                0.015_625,
                1.0 - f64::EPSILON,
            ]);
            while !xs.len().is_multiple_of(V::N) {
                xs.push(0.5);
            }
            for chunk in xs.chunks(V::N) {
                let mut got = [0.0; 4];
                V::store_f64(&mut got, to_srgb_lanes::<V>(V::load_f64(chunk)));
                for (g, x) in got.iter().zip(chunk) {
                    assert_eq!(g.to_bits(), to_srgb(*x).to_bits(), "{x}");
                }
            }
        }
        crate::math::simd::on_each_level!(check);
    }
}
