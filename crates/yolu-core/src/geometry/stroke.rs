//! 3D ビューのストロークの画面の点の並べ方（Unity 版の TexturePaintWindow の AddSurfacePoint・PaintSurfaceSegment・FinishSurfaceCurve）。
//!
//! 入力の点の間を centripetal Catmull-Rom（C# の StrokeCurve）で結び、区間の始まりの面でのブラシの直径から決めた画面の間隔で
//! ダブの位置を出す。最新の点への区間は、その先の点（向きを決める）が来るか離すまで待たせる。1 つの入力で
//! [`SURFACE_DABS_PER_EVENT`] を超えるダブになる区間は描かずに断る（呼ぶ側はストロークを取り消す）。
//! 画面の点の単位は呼ぶ側のもの（Unity 版は GUI の点）。

use glam::Vec2;

/// 1 回の入力（1 区間）に置けるダブの数の上限（Unity 版と同じ 128）。
pub const SURFACE_DABS_PER_EVENT: usize = 128;

/// 手で描いた入力の点を結ぶ曲線（C# の StrokeCurve: centripetal Catmull-Rom、α = 0.5。倍精度）。
pub struct StrokeCurve;

impl StrokeCurve {
    /// 重なった点とみなす距離。
    pub const COINCIDENT_DISTANCE: f64 = 1e-6;

    /// 区間 p1 → p2 の t（0 で p1、1 で p2）の位置（Barry と Goldman のピラミッド形の式）。
    #[allow(clippy::too_many_arguments)]
    pub fn point(
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        x3: f64,
        y3: f64,
        t: f64,
    ) -> (f64, f64) {
        let k01 = knot(x0, y0, x1, y1);
        let k12 = knot(x1, y1, x2, y2);
        let k23 = knot(x2, y2, x3, y3);
        let (t0, t1) = (0.0, k01);
        let t2 = t1 + k12;
        let t3 = t2 + k23;
        let u = t1 + k12 * t;
        let (a1x, a1y) = (mix(x0, x1, t0, t1, u), mix(y0, y1, t0, t1, u));
        let (a2x, a2y) = (mix(x1, x2, t1, t2, u), mix(y1, y2, t1, t2, u));
        let (a3x, a3y) = (mix(x2, x3, t2, t3, u), mix(y2, y3, t2, t3, u));
        let (b1x, b1y) = (mix(a1x, a2x, t0, t2, u), mix(a1y, a2y, t0, t2, u));
        let (b2x, b2y) = (mix(a2x, a3x, t1, t3, u), mix(a2y, a3y, t1, t3, u));
        (mix(b1x, b2x, t1, t2, u), mix(b1y, b2y, t1, t2, u))
    }

    /// 端の外の点: p を about の向こうへ折り返した点（2·about − p）。
    pub fn reflect(x: f64, y: f64, about_x: f64, about_y: f64) -> (f64, f64) {
        (2.0 * about_x - x, 2.0 * about_y - y)
    }

    pub fn coincident(x0: f64, y0: f64, x1: f64, y1: f64) -> bool {
        let (dx, dy) = (x1 - x0, y1 - y0);
        dx * dx + dy * dy < Self::COINCIDENT_DISTANCE * Self::COINCIDENT_DISTANCE
    }
}

fn knot(xa: f64, ya: f64, xb: f64, yb: f64) -> f64 {
    let (dx, dy) = (xb - xa, yb - ya);
    let v = (dx * dx + dy * dy).sqrt().sqrt();
    if 1e-9 > v {
        1e-9
    } else {
        v
    }
}

fn mix(a: f64, b: f64, ta: f64, tb: f64, u: f64) -> f64 {
    (tb - u) / (tb - ta) * a + (u - ta) / (tb - ta) * b
}

/// 区間のダブが 1 つの入力の上限を超えた（ストロークを取り消す）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooManyDabs;

impl std::fmt::Display for TooManyDabs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("1 回の入力のダブが多すぎるので、ストロークを取り消しました。ブラシを小さくするか、ゆっくり動かしてください")
    }
}

impl std::error::Error for TooManyDabs {}

/// 3D のストロークの画面の点（押した点から始める。押した点のダブは呼ぶ側が置く）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenStrokeSampler {
    previous: Vec2,
    previous_pressure: f32,
    before: Vec2,
    has_before: bool,
    held: Vec2,
    held_pressure: f32,
    has_held: bool,
}

/// Unity の Vector2.Lerp（t は 0〜1 に収める）。
fn lerp2(a: Vec2, b: Vec2, t: f32) -> Vec2 {
    let t = super::unity::clamp01(t);
    Vec2::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * super::unity::clamp01(t)
}

fn distance(a: Vec2, b: Vec2) -> f32 {
    super::unity::v2_magnitude(a - b)
}

impl ScreenStrokeSampler {
    pub fn new(at: Vec2, pressure: f32) -> ScreenStrokeSampler {
        ScreenStrokeSampler {
            previous: at,
            previous_pressure: pressure,
            before: at,
            has_before: false,
            held: at,
            held_pressure: pressure,
            has_held: false,
        }
    }

    /// 最後に描いた点。
    pub fn last_point(&self) -> Vec2 {
        self.previous
    }

    /// 新しい入力の点: 待たせていた点までの区間を、この点で向きを決めた曲線にしてダブの位置（画面の点と筆圧）を out に足し、この点を待たせる。
    /// spacing は区間の始まりの点を受け、そこでのダブの間隔（画面の単位）を返す（面が無ければ 1 など）。
    pub fn add(
        &mut self,
        at: Vec2,
        pressure: f32,
        spacing: impl FnMut(Vec2) -> f32,
        out: &mut Vec<(Vec2, f32)>,
    ) -> Result<(), TooManyDabs> {
        if !self.has_held {
            if StrokeCurve::coincident(
                self.previous.x as f64,
                self.previous.y as f64,
                at.x as f64,
                at.y as f64,
            ) {
                self.previous_pressure = pressure; // 動かない入力は筆圧だけ
            } else {
                self.held = at;
                self.held_pressure = pressure;
                self.has_held = true;
            }
            return Ok(());
        }
        if StrokeCurve::coincident(
            self.held.x as f64,
            self.held.y as f64,
            at.x as f64,
            at.y as f64,
        ) {
            self.held_pressure = pressure;
            return Ok(());
        }
        self.segment(at, spacing, out)?;
        self.held = at;
        self.held_pressure = pressure;
        Ok(())
    }

    /// 離したとき: 待たせている最後の区間を、その先を折り返した向きで描く。
    pub fn finish(
        &mut self,
        spacing: impl FnMut(Vec2) -> f32,
        out: &mut Vec<(Vec2, f32)>,
    ) -> Result<(), TooManyDabs> {
        if !self.has_held {
            return Ok(());
        }
        let (x, y) = StrokeCurve::reflect(
            self.previous.x as f64,
            self.previous.y as f64,
            self.held.x as f64,
            self.held.y as f64,
        );
        self.segment(Vec2::new(x as f32, y as f32), spacing, out)?;
        self.has_held = false;
        Ok(())
    }

    /// previous → held の区間（前は before、後は next で向きを決める）に、画面の間隔でダブを置く（C# の PaintSurfaceSegment）。
    fn segment(
        &mut self,
        next: Vec2,
        mut spacing: impl FnMut(Vec2) -> f32,
        out: &mut Vec<(Vec2, f32)>,
    ) -> Result<(), TooManyDabs> {
        let (a, b) = (self.previous, self.held);
        let before = if self.has_before {
            self.before
        } else {
            let (rx, ry) = StrokeCurve::reflect(b.x as f64, b.y as f64, a.x as f64, a.y as f64);
            Vec2::new(rx as f32, ry as f32)
        };
        let gap = spacing(a);
        // 曲線を細かい折れ線にして長さを測り、長さで等分した位置にダブを置く
        let pieces = ((distance(a, b) / 2.0).ceil() as i32).clamp(4, 256) as usize;
        let mut points = vec![Vec2::ZERO; pieces + 1];
        let mut lengths = vec![0f32; pieces + 1];
        points[0] = a;
        for i in 1..=pieces {
            points[i] = if i == pieces {
                b
            } else {
                let (x, y) = StrokeCurve::point(
                    before.x as f64,
                    before.y as f64,
                    a.x as f64,
                    a.y as f64,
                    b.x as f64,
                    b.y as f64,
                    next.x as f64,
                    next.y as f64,
                    i as f64 / pieces as f64,
                );
                Vec2::new(x as f32, y as f32)
            };
            lengths[i] = lengths[i - 1] + distance(points[i - 1], points[i]);
        }
        let total = lengths[pieces];
        let steps = ((total / gap).ceil() as i64).max(1);
        if steps > SURFACE_DABS_PER_EVENT as i64 {
            return Err(TooManyDabs);
        }
        let steps = steps as usize;
        let start_pressure = self.previous_pressure;
        let mut j = 1usize;
        for i in 1..=steps {
            let s = if i == steps {
                total
            } else {
                total * i as f32 / steps as f32
            };
            while j < pieces && lengths[j] < s {
                j += 1;
            }
            let span = lengths[j] - lengths[j - 1];
            let f = if span > 0.0 {
                super::unity::clamp01((s - lengths[j - 1]) / span)
            } else {
                1.0
            };
            let at = if i == steps {
                b
            } else {
                lerp2(points[j - 1], points[j], f)
            };
            let pressure = lerp(
                start_pressure,
                self.held_pressure,
                ((j - 1) as f32 + f) / pieces as f32,
            );
            out.push((at, pressure));
        }
        self.before = a;
        self.has_before = true;
        self.previous = b;
        self.previous_pressure = self.held_pressure;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_input_gives_evenly_spaced_dabs_and_waits_for_the_last_point() {
        let mut s = ScreenStrokeSampler::new(Vec2::new(0.0, 0.0), 1.0);
        let mut out = Vec::new();
        s.add(Vec2::new(10.0, 0.0), 1.0, |_| 2.0, &mut out).unwrap();
        assert!(out.is_empty(), "最初の区間は次の点まで待つ");
        s.add(Vec2::new(20.0, 0.0), 0.5, |_| 2.0, &mut out).unwrap();
        assert_eq!(out.len(), 5);
        for (i, (p, pressure)) in out.iter().enumerate() {
            assert!(
                (p.x - 2.0 * (i + 1) as f32).abs() < 1e-4 && p.y.abs() < 1e-5,
                "{p}"
            );
            assert_eq!(*pressure, 1.0);
        }
        out.clear();
        s.finish(|_| 2.0, &mut out).unwrap();
        assert_eq!(out.len(), 5);
        assert_eq!(out.last().unwrap().0, Vec2::new(20.0, 0.0));
        assert!((out.last().unwrap().1 - 0.5).abs() < 1e-6);
    }

    #[test]
    fn too_many_dabs_refuses_the_segment() {
        let mut s = ScreenStrokeSampler::new(Vec2::ZERO, 1.0);
        let mut out = Vec::new();
        s.add(Vec2::new(1000.0, 0.0), 1.0, |_| 1.0, &mut out)
            .unwrap();
        assert_eq!(s.finish(|_| 1.0, &mut out), Err(TooManyDabs));
        assert!(out.is_empty());
    }

    #[test]
    fn curve_passes_through_the_points() {
        let (x, y) = StrokeCurve::point(0.0, 0.0, 1.0, 1.0, 3.0, 0.0, 4.0, 2.0, 0.0);
        assert!((x - 1.0).abs() < 1e-12 && (y - 1.0).abs() < 1e-12);
        let (x, y) = StrokeCurve::point(0.0, 0.0, 1.0, 1.0, 3.0, 0.0, 4.0, 2.0, 1.0);
        assert!((x - 3.0).abs() < 1e-12 && y.abs() < 1e-12);
    }
}
