//! 丸めと 8 bit の値の表。C# の Core の MathUtil と同じ値を出す（バイト一致の土台）。
//!
//! 回転（`Quat`）の長さ 1 への直し方は 2 つ。読む経路（保存した値・Unity の値・行列からの回転）は `Quat::normalize` のまま
//! （前のバージョンが残した値を、同じビットで開くため）。新しい回転を作る編集の経路（オイラー角の入力・ボーンの回転）だけ、
//! 直した値をもう一度直しても変わらない [`stable_normalize`] を使う。

use glam::Quat;

pub(crate) mod simd;

/// `stable_normalize` が探す、各成分のずれの上限（ULP）。
const MAX_SHIFT: i32 = 4;

/// 長さ 1 に直した回転で、もう一度直しても変わらないもの（固定点）。
///
/// `Quat::normalize` で直した回転を、もう一度直すと最後の桁（1 ULP）が変わることがある。長さを求める足す順が SSE2 と NEON で違い、
/// aarch64 では x86-64 より変わる回転が多く、2 つの値の間を行き来するものもある。編集で作った回転を保存して開き直したときにビットまで同じに
/// なるよう、直した値が固定点でなければ、各成分を近い順に [`MAX_SHIFT`] ULP までずらして固定点を探す（固定点なら直した値そのもの。
/// 固定点が見つからないときは `normalize` の値）。長さが 0 や有限でない回転は `normalize` と同じ値（呼ぶ側が先に避ける）。
///
/// `q` と `-q` は同じ回転なので、符号だけが違う結果になる（`stable_normalize(-q) == -stable_normalize(q)`、ビットまで）。
/// 符号を揃えた形（先頭の 0 でない成分が正）で探して、最後に戻す。値が 0 の成分は動かさない（-0 や -1e-45 にしない）。
pub fn stable_normalize(q: Quat) -> Quat {
    // w, x, y, z の順で最初の 0 でない成分が負なら反転して探す
    let a = q.to_array();
    let negative = [a[3], a[0], a[1], a[2]]
        .into_iter()
        .find(|v| *v != 0.0)
        .is_some_and(|v| v < 0.0);
    if negative {
        -settle(-q)
    } else {
        settle(q)
    }
}

fn settle(q: Quat) -> Quat {
    let n = q.normalize();
    if !n.is_finite() {
        return n;
    }
    let settled = |r: Quat| r.normalize() == r;
    if settled(n) {
        return n;
    }
    // 行き来するときの、もう一方の値
    let other = n.normalize();
    if settled(other) {
        return other;
    }
    let shifted = |v: f32, k: i32| {
        if v == 0.0 {
            return v;
        }
        (0..k.unsigned_abs()).fold(v, |v, _| if k < 0 { v.next_down() } else { v.next_up() })
    };
    let range = -MAX_SHIFT..=MAX_SHIFT;
    let own = n.to_array();
    // 近い順（各成分のずれの最大が 1 ULP、2 ULP、…）
    for radius in 1..=MAX_SHIFT {
        for dx in range.clone() {
            for dy in range.clone() {
                for dz in range.clone() {
                    for dw in range.clone() {
                        if [dx, dy, dz, dw].map(i32::abs).into_iter().max() != Some(radius) {
                            continue;
                        }
                        let r = Quat::from_array([
                            shifted(own[0], dx),
                            shifted(own[1], dy),
                            shifted(own[2], dz),
                            shifted(own[3], dw),
                        ]);
                        if settled(r) {
                            return r;
                        }
                    }
                }
            }
        }
    }
    n
}

/// b / 255.0（すべての b について、割り算と同じ double）。
pub(crate) static UNIT: [f64; 256] = make_unit();

const fn make_unit() -> [f64; 256] {
    let mut t = [0.0; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = i as f64 / 255.0;
        i += 1;
    }
    t
}

/// 0〜1 の値を 0〜255 へ、四捨五入（floor(v × 255 + 0.5)）して範囲に収める。NaN は 0。
/// C# の MathUtil.ToByte と同じ比較の順（≥ 255 なら 255、> 0 なら切り捨て、ほかは 0）。
#[inline(always)]
pub(crate) fn to_byte(value: f64) -> u8 {
    let v = value * 255.0 + 0.5;
    if v >= 255.0 {
        255
    } else if v > 0.0 {
        v as u8 // 正の値の切り捨ては floor
    } else {
        0
    }
}

#[inline]
pub(crate) fn clamp01(value: f64) -> f64 {
    // C# の MathUtil.Clamp01 は Math.Max(0, Math.Min(1, v))。NaN を通す値は来ない（入力は有限か検査済み）
    if value < 0.0 {
        0.0
    } else if value > 1.0 {
        1.0
    } else {
        value
    }
}

pub(crate) fn require_finite(value: f64, what: crate::Reason) -> Result<(), crate::CoreError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(crate::CoreError::InvalidArgument(what))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_is_the_division() {
        for (i, u) in UNIT.iter().enumerate() {
            assert_eq!(u.to_bits(), (i as f64 / 255.0).to_bits());
        }
    }

    #[test]
    fn to_byte_rounds_half_up_and_clamps() {
        assert_eq!(to_byte(0.0), 0);
        assert_eq!(to_byte(-1.0), 0);
        assert_eq!(to_byte(f64::NAN), 0);
        assert_eq!(to_byte(1.0), 255);
        assert_eq!(to_byte(2.0), 255);
        assert_eq!(to_byte(f64::INFINITY), 255);
        assert_eq!(to_byte(f64::NEG_INFINITY), 0);
        assert_eq!(to_byte(0.5), 128); // 127.5 + 0.5 = 128
        for (i, u) in UNIT.iter().enumerate() {
            assert_eq!(to_byte(*u) as usize, i);
        }
    }

    /// 乱数（xorshift）の回転。長さは 1 前後から大きくずらした物も混ぜる。
    fn random_rotations(count: usize) -> Vec<Quat> {
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
        };
        (0..count)
            .map(|i| {
                let v: [f64; 4] = std::array::from_fn(|_| next());
                let l = v.iter().map(|a| a * a).sum::<f64>().sqrt();
                // 3 つに 1 つは長さを 1 からずらす（f64 で割って f32 に丸めただけの値、そのまま、10 倍）
                let scale = [1.0 / l, 1.0, 10.0][i % 3];
                Quat::from_array(std::array::from_fn(|k| (v[k] * scale) as f32))
            })
            .collect()
    }

    #[test]
    fn stable_normalize_is_a_fixed_point() {
        let mut moving = 0;
        for q in random_rotations(60_000) {
            let n = q.normalize();
            moving += usize::from(n.normalize() != n);
            let s = stable_normalize(q);
            assert_eq!(
                s.normalize().to_array().map(f32::to_bits),
                s.to_array().map(f32::to_bits),
                "{q:?}"
            );
            assert_eq!(stable_normalize(s), s, "{q:?}");
            // 直しただけの値から、1e-6 も動かない
            assert!((s - n).length() < 1.0e-6, "{q:?}");
        }
        // 直すと変わる回転を試せている（x86-64 でも 100 に 3 つほどある）
        assert!(moving > 0, "直すと変わる回転を試せていない");
    }

    #[test]
    fn stable_normalize_keeps_q_and_minus_q_apart_by_the_sign_only() {
        let bits = |q: Quat| q.to_array().map(f32::to_bits);
        for q in random_rotations(20_000) {
            assert_eq!(
                bits(stable_normalize(-q)),
                bits(-stable_normalize(q)),
                "{q:?}"
            );
        }
        // 0 の成分は 0 のまま
        for q in [
            Quat::from_xyzw(0.0, 0.6, 0.0, 0.8),
            Quat::from_xyzw(0.0, -0.6, 0.0, -0.8),
            Quat::from_xyzw(0.3, 0.0, 0.0, 0.0),
            Quat::from_xyzw(0.0, 0.0, 0.0, -2.0),
        ] {
            let s = stable_normalize(q);
            for (a, b) in q.to_array().into_iter().zip(s.to_array()) {
                assert!(a != 0.0 || b == 0.0, "{q:?} -> {s:?}");
            }
        }
    }
}
