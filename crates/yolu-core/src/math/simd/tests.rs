//! レーンの演算が、スカラーの式と同じ bit を出すことの試験。AVX2・SSE4.1・NEON（この CPU が持つ道）で走らせる。
#![allow(clippy::needless_range_loop)]

use super::*;
use crate::math::{to_byte as scalar_to_byte, UNIT};

pub(crate) struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    /// 0〜1（端の値を多めに）。
    pub fn unit(&mut self) -> f64 {
        let r = self.next();
        match r & 15 {
            0 => 0.0,
            1 => 1.0,
            2 => 0.5,
            3 => UNIT[(r >> 8) as u8 as usize],
            _ => (r >> 11) as f64 / (1u64 << 53) as f64,
        }
    }
    pub fn byte(&mut self) -> u8 {
        let r = self.next();
        match r & 7 {
            0 => 0,
            1 => 255,
            _ => (r >> 8) as u8,
        }
    }
}

/// 引数に使う値の一覧（NaN・無限・符号つきのゼロ・境目）。
fn specials() -> Vec<f64> {
    let mut v = vec![
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.5,
        -0.5,
        255.0,
        254.999999999,
        255.0000001,
        0.5 / 255.0,
        -0.5 / 255.0,
        1e-300,
        -1e-300,
        f64::MIN_POSITIVE,
        f64::from_bits(1),
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        2.0,
        1.0 - f64::EPSILON,
        1.0 + f64::EPSILON,
        // 前提（0〜255 の整数・0〜1）の外: 負の値、255 や 65535 や 2³¹ を超える値、整数に丸めたとき端になる値、f64 の端
        -255.0,
        256.0,
        65535.0,
        65536.0,
        1e10,
        2_147_483_647.0,
        2_147_483_648.0,
        -2_147_483_649.0,
        4_503_599_627_370_495.5,
        4_503_599_627_370_496.0,
        -4_503_599_627_370_495.5,
        f64::MAX,
        f64::MIN,
        -f64::MIN_POSITIVE,
        // NaN のほかの形（符号つき・ペイロードつき・シグナリング）
        f64::from_bits(0xFFF8_0000_0000_0000),
        f64::from_bits(0x7FF8_0000_0000_5EED),
        f64::from_bits(0x7FF0_0000_0000_0001),
        f64::from_bits(0x7FFF_FFFF_FFFF_FFFF),
    ];
    for b in 0..=255 {
        let u = UNIT[b];
        v.extend([u, f64::from_bits(u.to_bits() + 1)]);
        if b > 0 {
            v.push(f64::from_bits(u.to_bits() - 1));
        }
        // 0.5 を足して floor する境目
        let h = (f64::from(b as u8) + 0.5) / 255.0;
        v.extend([
            h,
            f64::from_bits(h.to_bits() + 1),
            f64::from_bits(h.to_bits() - 1),
        ]);
    }
    v
}

fn same(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

unsafe fn lanes_of<V: Lanes>(values: &[f64]) -> V::F {
    V::from_fn(|k| values[k])
}

unsafe fn each<V: Lanes>(v: V::F, out: &mut Vec<f64>) {
    out.clear();
    out.extend((0..V::N).map(|k| V::lane(v, k)));
}

unsafe fn unit_matches_the_division_for_every_byte_on<V: Lanes>() {
    let mut out = Vec::new();
    let mut b = 0usize;
    while b < 256 {
        let x = V::from_fn(|k| (b + k).min(255) as f64);
        each::<V>(V::unit(x), &mut out);
        for k in 0..V::N {
            let i = (b + k).min(255);
            assert_eq!(out[k].to_bits(), UNIT[i].to_bits(), "{i}");
        }
        b += V::N;
    }
}

#[test]
fn unit_matches_the_division_for_every_byte() {
    on_each_level!(unit_matches_the_division_for_every_byte_on);
    // 定数（1/255 の上の桁と残り）は、有理数の厳密な計算で 256 通りすべて割り算と同じ bit になることを確かめて決めた。ここでは実際に
    // FMA の命令を使う道（上）で確かめる。ソフトウェアの `mul_add` は環境の libm の fma に依るので使わない（MinGW の fma は厳密でない場合がある）。
}

unsafe fn to_byte_matches_the_scalar_on<V: Lanes>() {
    let mut rng = Rng(11);
    let mut values = specials();
    for _ in 0..20000 {
        let r = rng.next();
        values.push(match r & 3 {
            0 => (r >> 11) as f64 / (1u64 << 53) as f64,
            1 => ((r >> 11) as f64 / (1u64 << 53) as f64) * 3.0 - 1.0,
            2 => f64::from_bits(r),
            _ => (rng.byte() as f64 + 0.5) / 255.0,
        });
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i + V::N <= values.len() {
        let x = lanes_of::<V>(&values[i..]);
        each::<V>(to_byte::<V>(x), &mut out);
        for k in 0..V::N {
            assert_eq!(
                out[k] as u8,
                scalar_to_byte(values[i + k]),
                "{}",
                values[i + k]
            );
            assert_eq!(out[k], f64::from(scalar_to_byte(values[i + k])));
        }
        i += V::N;
    }
}

#[test]
fn to_byte_matches_the_scalar() {
    on_each_level!(to_byte_matches_the_scalar_on);
}

unsafe fn arithmetic_and_comparisons_follow_the_scalar_on<V: Lanes>() {
    let values = specials();
    let mut out = Vec::new();
    let mut check = |name: &str, f: &dyn Fn(f64, f64) -> f64, g: &dyn Fn(V::F, V::F) -> V::F| {
        for (i, &a) in values.iter().enumerate() {
            let pairs: Vec<(f64, f64)> = (0..V::N)
                .map(|k| (a, values[(i * 7 + k * 13 + 1) % values.len()]))
                .collect();
            let x = V::from_fn(|k| pairs[k].0);
            let y = V::from_fn(|k| pairs[k].1);
            each::<V>(g(x, y), &mut out);
            for (k, &(p, q)) in pairs.iter().enumerate() {
                let want = f(p, q);
                assert!(same(out[k], want), "{name}({p}, {q}): {} != {want}", out[k]);
            }
        }
    };
    check("add", &|a, b| a + b, &|a, b| V::add(a, b));
    check("sub", &|a, b| a - b, &|a, b| V::sub(a, b));
    check("mul", &|a, b| a * b, &|a, b| V::mul(a, b));
    check("div", &|a, b| a / b, &|a, b| V::div(a, b));
    check("min", &|a, b| if a < b { a } else { b }, &|a, b| {
        V::min(a, b)
    });
    check("max", &|a, b| if a > b { a } else { b }, &|a, b| {
        V::max(a, b)
    });
    let sel = |m: bool| if m { 1.0 } else { 0.0 };
    let one = || V::splat(1.0);
    let zero = || V::splat(0.0);
    check("lt", &|a, b| sel(a < b), &|a, b| {
        V::select(V::lt(a, b), one(), zero())
    });
    check("le", &|a, b| sel(a <= b), &|a, b| {
        V::select(V::le(a, b), one(), zero())
    });
    check("gt", &|a, b| sel(a > b), &|a, b| {
        V::select(V::gt(a, b), one(), zero())
    });
    check("ge", &|a, b| sel(a >= b), &|a, b| {
        V::select(V::ge(a, b), one(), zero())
    });
    check("eq", &|a, b| sel(a == b), &|a, b| {
        V::select(V::eq(a, b), one(), zero())
    });
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    let not_lt = |a: f64, b: f64| sel(!(a < b));
    check("not_lt", &not_lt, &|a, b| {
        V::select(V::not(V::lt(a, b)), one(), zero())
    });
    check("and", &|a, b| sel(a < b && b < 2.0), &|a, b| {
        V::select(V::and(V::lt(a, b), V::lt(b, V::splat(2.0))), one(), zero())
    });
    check("or", &|a, b| sel(a < b || b < 0.0), &|a, b| {
        V::select(V::or(V::lt(a, b), V::lt(b, V::splat(0.0))), one(), zero())
    });
    // 選ばれなかった側が NaN でも、選んだ側の値がそのまま出る
    check(
        "select_picks_one_side",
        &|a, b| if a > 0.0 { a } else { b },
        &|a, b| V::select(V::gt(a, V::splat(0.0)), a, b),
    );
    for &a in &values {
        let x = V::splat(a);
        each::<V>(V::sqrt(x), &mut out);
        assert!(out.iter().all(|&o| same(o, a.sqrt())), "sqrt {a}");
        each::<V>(V::floor(x), &mut out);
        assert!(out.iter().all(|&o| same(o, a.floor())), "floor {a}");
        each::<V>(V::abs(x), &mut out);
        assert!(out.iter().all(|&o| same(o, a.abs())), "abs {a}");
        each::<V>(V::neg(x), &mut out);
        assert!(
            out.iter()
                .all(|&o| o.to_bits() == (-a).to_bits() || (a.is_nan() && o.is_nan())),
            "neg {a}"
        );
    }
}

/// 比較と選択、符号のビットだけの演算（`min`・`max`・`select`・`abs`・`neg`）は、NaN のペイロードや符号つきのゼロも含めて、
/// スカラーの式が選ぶ側のビットがそのまま出る（`vmin`・`vmax` の NaN・±0 の扱いと取り違えていないことの確かめ）。
unsafe fn selection_keeps_every_bit_on<V: Lanes>() {
    const SIGN: u64 = 1 << 63;
    let values = specials();
    let mut out = Vec::new();
    for (i, &a) in values.iter().enumerate() {
        let pairs: Vec<(f64, f64)> = (0..V::N)
            .map(|k| (a, values[(i * 11 + k * 5 + 3) % values.len()]))
            .collect();
        let x = V::from_fn(|k| pairs[k].0);
        let y = V::from_fn(|k| pairs[k].1);
        each::<V>(V::min(x, y), &mut out);
        for (k, &(p, q)) in pairs.iter().enumerate() {
            let want = if p < q { p } else { q };
            assert_eq!(out[k].to_bits(), want.to_bits(), "min({p}, {q})");
        }
        each::<V>(V::max(x, y), &mut out);
        for (k, &(p, q)) in pairs.iter().enumerate() {
            let want = if p > q { p } else { q };
            assert_eq!(out[k].to_bits(), want.to_bits(), "max({p}, {q})");
        }
        each::<V>(V::select(V::lt(x, y), x, y), &mut out);
        for (k, &(p, q)) in pairs.iter().enumerate() {
            let want = if p < q { p } else { q };
            assert_eq!(out[k].to_bits(), want.to_bits(), "select({p}, {q})");
        }
        each::<V>(V::abs(x), &mut out);
        for (k, &(p, _)) in pairs.iter().enumerate() {
            assert_eq!(out[k].to_bits(), p.to_bits() & !SIGN, "abs({p})");
        }
        each::<V>(V::neg(x), &mut out);
        for (k, &(p, _)) in pairs.iter().enumerate() {
            assert_eq!(out[k].to_bits(), p.to_bits() ^ SIGN, "neg({p})");
        }
    }
}

#[test]
fn selection_keeps_every_bit() {
    on_each_level!(selection_keeps_every_bit_on);
}

unsafe fn mask_any_all_on<V: Lanes>() {
    let all_true = V::ge(V::splat(1.0), V::splat(0.0));
    let all_false = V::lt(V::splat(1.0), V::splat(0.0));
    assert!(V::any(all_true) && V::all(all_true));
    assert!(!V::any(all_false) && !V::all(all_false));
    // 1 本だけ真
    for k in 0..V::N {
        let m = V::eq(
            V::from_fn(|i| if i == k { 5.0 } else { 0.0 }),
            V::splat(5.0),
        );
        assert!(V::any(m));
        assert_eq!(V::all(m), V::N == 1);
        assert!(!V::any(V::not(V::or(m, V::not(m)))));
    }
}

#[test]
fn arithmetic_and_comparisons_follow_the_scalar() {
    on_each_level!(arithmetic_and_comparisons_follow_the_scalar_on);
}

#[test]
fn mask_any_all() {
    on_each_level!(mask_any_all_on);
}

unsafe fn pixels_round_trip_on<V: Lanes>() {
    let mut rng = Rng(5);
    for _ in 0..2000 {
        let bytes: Vec<u8> = (0..V::N * 4).map(|_| rng.byte()).collect();
        let v = V::load(&bytes);
        for c in 0..4 {
            for k in 0..V::N {
                assert_eq!(V::lane(v[c], k), f64::from(bytes[k * 4 + c]));
            }
        }
        let mut out = vec![0xEEu8; V::N * 4 + 3];
        V::store(&mut out, v);
        assert_eq!(&out[..V::N * 4], &bytes[..]);
        assert_eq!(&out[V::N * 4..], &[0xEE; 3], "範囲の外は書かない");
        let px = [rng.byte(), rng.byte(), rng.byte(), rng.byte()];
        let s = V::splat_px(px);
        for c in 0..4 {
            for k in 0..V::N {
                assert_eq!(V::lane(s[c], k), f64::from(px[c]));
            }
        }
    }
}

#[test]
fn pixels_round_trip() {
    on_each_level!(pixels_round_trip_on);
}

unsafe fn integer_loads_and_stores_on<V: Lanes>() {
    let mut rng = Rng(6);
    for _ in 0..2000 {
        let u16s: Vec<u16> = (0..V::N * 4)
            .map(|_| match rng.next() & 7 {
                0 => 0,
                1 => 65535,
                _ => (rng.next() >> 8) as u16,
            })
            .collect();
        let v = V::load_u16(&u16s);
        for k in 0..V::N {
            assert_eq!(V::lane(v, k), f64::from(u16s[k]));
        }
        let mut out = vec![0xBEEFu16; V::N + 2];
        V::store_u16(&mut out, v);
        assert_eq!(&out[..V::N], &u16s[..V::N]);
        assert_eq!(&out[V::N..], &[0xBEEF; 2], "範囲の外は書かない");
        // u16 × 4 チャンネル
        let px = V::load_u16x4(&u16s);
        for c in 0..4 {
            for k in 0..V::N {
                assert_eq!(V::lane(px[c], k), f64::from(u16s[k * 4 + c]));
            }
        }
        // u16 × 4 チャンネルの書き出し（読み出しの逆）
        let mut out = vec![0xBEEFu16; V::N * 4 + 3];
        V::store_u16x4(&mut out, px);
        assert_eq!(&out[..V::N * 4], &u16s[..V::N * 4]);
        assert_eq!(&out[V::N * 4..], &[0xBEEF; 3], "範囲の外は書かない");
        // f64
        let f64s: Vec<f64> = (0..V::N).map(|_| f64::from_bits(rng.next())).collect();
        let v = V::load_f64(&f64s);
        for k in 0..V::N {
            assert_eq!(V::lane(v, k).to_bits(), f64s[k].to_bits());
        }
        let mut out = vec![f64::from_bits(0xABCD); V::N + 1];
        V::store_f64(&mut out, v);
        for k in 0..V::N {
            assert_eq!(out[k].to_bits(), f64s[k].to_bits());
        }
        assert_eq!(out[V::N].to_bits(), 0xABCD, "範囲の外は書かない");
        // u32（2³¹ 未満）
        let u32s: Vec<u32> = (0..V::N)
            .map(|_| match rng.next() & 3 {
                0 => 0,
                1 => 0x7FFF_FFFF,
                _ => (rng.next() >> 33) as u32,
            })
            .collect();
        let v = V::load_u32(&u32s);
        for k in 0..V::N {
            assert_eq!(V::lane(v, k), f64::from(u32s[k]));
        }
        let mut out = vec![0xDEAD_BEEFu32; V::N + 1];
        V::store_u32(&mut out, v);
        assert_eq!(&out[..V::N], &u32s[..]);
        assert_eq!(out[V::N], 0xDEAD_BEEF, "範囲の外は書かない");
    }
}

#[test]
fn integer_loads_and_stores() {
    on_each_level!(integer_loads_and_stores_on);
}

#[test]
fn load_and_store_refuse_a_short_slice() {
    unsafe fn short<V: Lanes>() {
        let bytes = vec![0u8; V::N * 4 - 1];
        let r = std::panic::catch_unwind(|| {
            let _ = V::load(&bytes);
        });
        assert!(r.is_err(), "短い読み出しは範囲外を読まずに止まる");
        let mut out = vec![0u8; V::N * 4 - 1];
        let v = V::splat_px([1, 2, 3, 4]);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| V::store(&mut out, v)));
        assert!(r.is_err(), "短い書き込みは範囲外へ書かずに止まる");
        assert!(out.iter().all(|&b| b == 0));
    }
    on_each_level!(short);
}

#[test]
fn the_chosen_level_never_exceeds_the_cpu() {
    assert!(level() <= detect());
    for l in forced::supported() {
        forced::with_level(l, || assert_eq!(level(), l));
    }
}

/// 別の CPU の道の名前（その CPU では知らない値）。
#[cfg(target_arch = "x86_64")]
const FOREIGN_NAMES: &[&str] = &["neon"];
#[cfg(not(target_arch = "x86_64"))]
const FOREIGN_NAMES: &[&str] = &["sse41", "avx2"];

#[test]
fn the_request_only_narrows_the_level() {
    for detected in Level::ALL.iter().copied() {
        assert_eq!(clamp_to_request(None, detected), detected);
        assert_eq!(clamp_to_request(Some(""), detected), detected);
        assert_eq!(clamp_to_request(Some("bogus"), detected), detected);
        assert_eq!(clamp_to_request(Some("scalar"), detected), Level::Scalar);
        for &foreign in FOREIGN_NAMES {
            assert_eq!(
                clamp_to_request(Some(foreign), detected),
                detected,
                "別の CPU の道 {foreign} は知らない値として無視する"
            );
        }
        for wanted in Level::ALL.iter().copied() {
            assert_eq!(
                clamp_to_request(Some(wanted.name()), detected),
                wanted.min(detected),
                "{wanted:?} を {detected:?} の CPU で"
            );
            assert_eq!(
                clamp_to_request(Some(&wanted.name().to_uppercase()), detected),
                detected,
                "綴りは小文字だけ"
            );
        }
    }
}

#[test]
fn the_levels_of_this_cpu_are_listed_narrow_first() {
    let all = Level::ALL;
    assert_eq!(all[0], Level::Scalar);
    assert!(all.windows(2).all(|w| w[0] < w[1]), "{all:?}");
    let names: std::collections::HashSet<_> = all.iter().map(|l| l.name()).collect();
    assert_eq!(names.len(), all.len(), "道の名前は重ならない");
    // この CPU で使える道は、一番広い道（detect()）までの全部
    let supported = forced::supported();
    assert_eq!(supported.last(), Some(&detect()));
    assert!(supported.iter().all(|l| all.contains(l)));
    // aarch64 は NEON があるので、スカラーと NEON の 2 つ
    #[cfg(target_arch = "aarch64")]
    assert_eq!(supported, vec![Level::Scalar, Level::Neon]);
}

/// 環境変数 `YOLU_SIMD` の値が、プロセスで選ぶ道に反映される（値を変えて回すと、道が変わる）。
#[test]
fn the_environment_variable_picks_the_level() {
    let wanted = std::env::var("YOLU_SIMD").ok();
    let expected = match wanted.as_deref() {
        Some("scalar") => Level::Scalar,
        Some(name) => Level::ALL
            .iter()
            .copied()
            .find(|l| l.name() == name && *l <= detect())
            .unwrap_or(detect()),
        None => detect(),
    };
    assert_eq!(from_environment(detect()), expected, "{wanted:?}");
}
