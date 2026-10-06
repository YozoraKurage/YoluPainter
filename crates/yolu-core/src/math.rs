//! 丸めと 8 bit の値の表。C# の Core の MathUtil と同じ値を出す（バイト一致の土台）。

pub(crate) mod simd;

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

pub(crate) fn require_finite(value: f64, what: &'static str) -> Result<(), crate::CoreError> {
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
}
