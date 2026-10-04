//! 一時の調査（コミットしない）: pen_tilt の入力で、std（OS の libm）と libm クレートの tan・atan・atan2 を並べて書き出す。
#![allow(clippy::all)]
use std::io::Write;

struct SplitMix(u64);
impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn u01(&mut self) -> f64 {
        (self.next() >> 11) as f64 * (1.0 / 9007199254740992.0)
    }
}

const MAX_ANGLE: f64 = std::f64::consts::FRAC_PI_2;

#[test]
fn probe() {
    let path = std::env::var("PROBE_OUT").unwrap();
    let mut out = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut rng = SplitMix(3003);
    let value = |rng: &mut SplitMix| {
        if rng.next() % 7 == 0 { 0.0 } else { (rng.u01() * 2.0 - 1.0) * MAX_ANGLE }
    };
    let limit = |v: f64| {
        let hi = if MAX_ANGLE - 1e-9 < v { MAX_ANGLE - 1e-9 } else { v };
        if -MAX_ANGLE + 1e-9 > hi { -MAX_ANGLE + 1e-9 } else { hi }
    };
    let b = |x: f64| format!("{:016x}", x.to_bits());
    for _ in 0..65536 {
        let tx = value(&mut rng);
        let ty = value(&mut rng);
        let (ax, ay) = (tx.abs(), ty.abs());
        // amount
        if !(ax <= 0.0 && ay <= 0.0) && !(ax >= MAX_ANGLE - 1e-9 || ay >= MAX_ANGLE - 1e-9) {
            writeln!(out, "tan {} {} {}", b(ax), b(ax.tan()), b(libm::tan(ax))).unwrap();
            writeln!(out, "tan {} {} {}", b(ay), b(ay.tan()), b(libm::tan(ay))).unwrap();
            let (sx, sy) = (ax.tan(), ay.tan());
            let r = (sx * sx + sy * sy).sqrt();
            writeln!(out, "atan {} {} {}", b(r), b(r.atan()), b(libm::atan(r))).unwrap();
        }
        if !(tx == 0.0 && ty == 0.0) {
            let (lx, ly) = (limit(tx), limit(ty));
            writeln!(out, "tan {} {} {}", b(lx), b(lx.tan()), b(libm::tan(lx))).unwrap();
            writeln!(out, "tan {} {} {}", b(ly), b(ly.tan()), b(libm::tan(ly))).unwrap();
            let (y, x) = (ly.tan(), lx.tan());
            writeln!(out, "atan2 {} {} {} {}", b(y), b(x), b(y.atan2(x)), b(libm::atan2(y, x))).unwrap();
        }
    }
}
