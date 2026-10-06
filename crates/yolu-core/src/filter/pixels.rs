use super::*;
use crate::math::UNIT;

/// 取消の確認。行・段の境界で呼ぶ。試験では呼ばれた回数と取り消す位置を差し替える。
pub(super) type Check<'a> = &'a dyn Fn() -> Result<(), Error>;

pub(super) fn hash(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846ca68b);
    h ^ (h >> 16)
}
pub(super) fn lerp(a: u8, b: u8, t: f64) -> u8 {
    (f64::from(a) + (f64::from(b) - f64::from(a)) * t + 0.5).floor() as u8
}
fn normalize(v: [f64; 3]) -> [f64; 3] {
    let l2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if l2 < 1e-12 {
        [0.0, 0.0, 1.0]
    } else {
        let l = l2.sqrt();
        [v[0] / l, v[1] / l, v[2] / l]
    }
}
fn encode(v: [f64; 3], a: u8) -> [u8; 4] {
    let v = normalize(v);
    [
        to_byte(v[0] * 0.5 + 0.5),
        to_byte(v[1] * 0.5 + 0.5),
        to_byte(v[2] * 0.5 + 0.5),
        a,
    ]
}
pub(super) fn mix(input: [u8; 4], output: [u8; 4], t: f64, normal: bool) -> [u8; 4] {
    if t >= 1.0 {
        return output;
    }
    if t <= 0.0 {
        return input;
    }
    if !normal && input[3] == output[3] {
        return [
            lerp(input[0], output[0], t),
            lerp(input[1], output[1], t),
            lerp(input[2], output[2], t),
            input[3],
        ];
    }
    let ba = UNIT[input[3] as usize] * (1.0 - t);
    let ia = UNIT[output[3] as usize] * t;
    let a = ba + ia;
    if a <= 0.0 {
        return [0; 4];
    }
    if normal {
        let b = normalize([
            UNIT[input[0] as usize] * 2.0 - 1.0,
            UNIT[input[1] as usize] * 2.0 - 1.0,
            UNIT[input[2] as usize] * 2.0 - 1.0,
        ]);
        let i = normalize([
            UNIT[output[0] as usize] * 2.0 - 1.0,
            UNIT[output[1] as usize] * 2.0 - 1.0,
            UNIT[output[2] as usize] * 2.0 - 1.0,
        ]);
        encode(
            [
                ba * b[0] + ia * i[0],
                ba * b[1] + ia * i[1],
                ba * b[2] + ia * i[2],
            ],
            to_byte(a),
        )
    } else {
        [
            to_byte((UNIT[input[0] as usize] * ba + UNIT[output[0] as usize] * ia) / a),
            to_byte((UNIT[input[1] as usize] * ba + UNIT[output[1] as usize] * ia) / a),
            to_byte((UNIT[input[2] as usize] * ba + UNIT[output[2] as usize] * ia) / a),
            to_byte(a),
        ]
    }
}
#[allow(clippy::too_many_arguments)]
pub(super) fn neighborhood(
    buf: &[u8],
    cur: Rect,
    next: Rect,
    w: u32,
    h: u32,
    s: &Stage,
    ty: ValueType,
    check: Check<'_>,
) -> Result<Vec<u8>, Error> {
    let mut q = zeros::<u16>(buf.len())?;
    rows::premultiply_at(crate::math::simd::level(), buf, &mut q);
    let radius = s.settings.halo();
    let mut remaining = radius;
    let mut bounds = cur;
    for r in [radius.div_ceil(3), (radius + 1) / 3, radius / 3] {
        if r == 0 {
            continue;
        }
        check()?;
        remaining -= r;
        let target = grow(next, remaining, w, h);
        q = box_blur(&q, bounds, target, r, w, h, check)?;
        bounds = target;
    }
    let level = crate::math::simd::level();
    let mut out = zeros::<u8>(area(next) * 4)?;
    let nw = next.width as usize;
    for y in 0..next.height {
        check()?;
        let o = y as usize * nw * 4;
        let si =
            ((y + next.y - cur.y) as usize * cur.width as usize + (next.x - cur.x) as usize) * 4;
        let input_row = &buf[si..si + nw * 4];
        let q_row = &q[o..o + nw * 4];
        let out_row = &mut out[o..o + nw * 4];
        if rows::finish_row_at(
            level,
            &s.settings,
            ty,
            s.strength,
            input_row,
            q_row,
            out_row,
        ) {
            continue;
        }
        // 接空間法線のぼかし: ベクトルを再正規化するので 1 画素ずつ
        for x in 0..nw {
            let i = x * 4;
            let input: [u8; 4] = input_row[i..i + 4].try_into().unwrap();
            let qa = u32::from(q_row[i + 3]);
            let mut f = input;
            let a = ((qa + 127) / 255) as u8;
            if a == 0 {
                f[3] = 0;
            } else {
                f = encode(
                    [
                        2.0 * f64::from(q_row[i]) / f64::from(qa) - 1.0,
                        2.0 * f64::from(q_row[i + 1]) / f64::from(qa) - 1.0,
                        2.0 * f64::from(q_row[i + 2]) / f64::from(qa) - 1.0,
                    ],
                    a,
                );
            }
            f = mix(input, f, s.strength, true);
            out_row[i..i + 4].copy_from_slice(&f);
        }
    }
    Ok(out)
}
fn edge(v: i64, n: u32) -> u32 {
    v.clamp(0, i64::from(n) - 1) as u32
}
/// 箱ぼかし 1 回（横・縦）。SIMD の道があればそれを、なければ（またはスカラーを選んだとき）下の画素ごとの実装を使う。
fn box_blur(
    src: &[u16],
    a: Rect,
    b: Rect,
    r: u32,
    w: u32,
    h: u32,
    check: Check<'_>,
) -> Result<Vec<u16>, Error> {
    match rows::box_blur_at(crate::math::simd::level(), src, a, b, r, w, h, check) {
        Some(result) => result,
        None => box_blur_scalar(src, a, b, r, w, h, check),
    }
}
/// 箱ぼかし 1 回の画素ごとの実装（SIMD の道の基準）。
pub(super) fn box_blur_scalar(
    src: &[u16],
    a: Rect,
    b: Rect,
    r: u32,
    w: u32,
    h: u32,
    check: Check<'_>,
) -> Result<Vec<u16>, Error> {
    let aw = a.width as usize;
    let bw = b.width as usize;
    let mut mid = zeros::<u16>(a.height as usize * bw * 4)?;
    let mut dst = zeros::<u16>(area(b) * 4)?;
    let inv = ((1u64 << 40) + u64::from(2 * r)) / (u64::from(2 * r) + 1);
    for y in 0..a.height as usize {
        check()?;
        let mut sum = [0u32; 4];
        for d in -(i64::from(r))..=i64::from(r) {
            let ix = (edge(i64::from(b.x) + d, w) - a.x) as usize;
            for c in 0..4 {
                sum[c] += u32::from(src[(y * aw + ix) * 4 + c]);
            }
        }
        for x in 0..bw {
            for c in 0..4 {
                mid[(y * bw + x) * 4 + c] = ((u64::from(sum[c] + r) * inv) >> 40) as u16;
            }
            if x + 1 == bw {
                break;
            }
            let leave = (edge(i64::from(b.x) + x as i64 - i64::from(r), w) - a.x) as usize;
            let enter = (edge(i64::from(b.x) + x as i64 + i64::from(r) + 1, w) - a.x) as usize;
            for c in 0..4 {
                sum[c] = sum[c] + u32::from(src[(y * aw + enter) * 4 + c])
                    - u32::from(src[(y * aw + leave) * 4 + c]);
            }
        }
    }
    let mut sums = zeros::<u32>(bw * 4)?;
    for d in -(i64::from(r))..=i64::from(r) {
        let row = (edge(i64::from(b.y) + d, h) - a.y) as usize * bw * 4;
        for (j, s) in sums.iter_mut().enumerate() {
            *s += u32::from(mid[row + j]);
        }
    }
    for y in 0..b.height as usize {
        check()?;
        let row = y * bw * 4;
        for (j, s) in sums.iter().enumerate() {
            dst[row + j] = ((u64::from(*s + r) * inv) >> 40) as u16;
        }
        if y + 1 == b.height as usize {
            break;
        }
        let leave = (edge(i64::from(b.y) + y as i64 - i64::from(r), h) - a.y) as usize * bw * 4;
        let enter = (edge(i64::from(b.y) + y as i64 + i64::from(r) + 1, h) - a.y) as usize * bw * 4;
        for (j, s) in sums.iter_mut().enumerate() {
            *s = *s + u32::from(mid[enter + j]) - u32::from(mid[leave + j]);
        }
    }
    Ok(dst)
}
fn combine(blend: GeneratorBlend, s: f64, v: f64, t: f64) -> f64 {
    let c = match blend {
        GeneratorBlend::Multiply => s * v,
        GeneratorBlend::Replace => v,
        GeneratorBlend::Screen => 1.0 - (1.0 - s) * (1.0 - v),
        GeneratorBlend::Max => s.max(v),
        GeneratorBlend::Min => s.min(v),
        GeneratorBlend::Add => (s + v).min(1.0),
        GeneratorBlend::Subtract => (s - v).max(0.0),
    };
    if t >= 1.0 {
        c
    } else {
        s + (c - s) * t
    }
}
pub(super) fn generate(p: &mut [u8], g: Generated, blend: GeneratorBlend, t: f64, mask: bool) {
    if mask {
        let v = match g {
            Generated::Scalar(v) => v,
            Generated::Mapped(m) => UNIT[m[0] as usize] * UNIT[m[3] as usize],
        };
        let hide = 255 - to_byte(combine(blend, 1.0 - UNIT[p[0] as usize], v, t));
        p[..3].fill(hide);
    } else {
        for c in 0..3 {
            let v = match g {
                Generated::Scalar(v) => v,
                Generated::Mapped(m) => UNIT[m[c] as usize],
            };
            p[c] = to_byte(combine(blend, UNIT[p[c] as usize], v, t));
        }
        if let Generated::Mapped(m) = g {
            p[3] = to_byte(UNIT[p[3] as usize] * (1.0 - t + t * UNIT[m[3] as usize]));
        }
    }
}
