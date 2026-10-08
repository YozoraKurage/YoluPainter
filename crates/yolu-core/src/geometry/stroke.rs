//! 3D ビューのストロークの画面の点の並べ方（Unity 版の TexturePaintWindow の AddSurfacePoint・PaintSurfaceSegment・FinishSurfaceCurve）。
//!
//! 入力の点の間を centripetal Catmull-Rom（C# の StrokeCurve）で結び、区間の始まりの面でのブラシの直径から決めた画面の間隔で
//! ダブの位置を出す。最新の点への区間は、その先の点（向きを決める）が来るか離すまで待たせる。区間のダブの数は
//! 入力の速さで決まり、上限は [`SURFACE_DABS_PER_SEGMENT`] だけ（ありえない長さの区間を断る）。1 回の入力で塗る数の
//! 区切り（[`SURFACE_DABS_PER_EVENT`]）は塗る側（`SurfaceStroke`）が持ち、超えた分は持ち越して後で塗る。
//! 画面の点の単位は呼ぶ側のもの（Unity 版は GUI の点）。

use glam::{DVec2, Vec2};

/// 1 回の入力・1 フレームに塗るダブの数（Unity 版の 1 回の入力の上限と同じ 128）。超えた分は捨てずに持ち越す（`SurfaceStroke`）。
pub const SURFACE_DABS_PER_EVENT: usize = 128;

/// 1 つの区間のダブの数の上限。間隔は画面の 0.5 点より狭くならないので、画面の 32768 点を超える長さの区間（有限でない点など）
/// だけが当たる。超えたら区間を描かずに断る（呼ぶ側はストロークを取り消す）。
pub const SURFACE_DABS_PER_SEGMENT: usize = 65_536;

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

/// 区間のダブが [`SURFACE_DABS_PER_SEGMENT`] を超えた（ストロークを取り消す）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooManyDabs;

impl std::fmt::Display for TooManyDabs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("1 つの区間のダブが多すぎるので、ストロークを取り消しました")
    }
}

impl std::error::Error for TooManyDabs {}

/// 3D のストロークの道の点（手ぶれ補正の後の画面の点）と、そこでのペンの状態。傾きは画面の右・上へ倒れる向きのラジアン、回転は画面で
/// 反時計回りのラジアン、速さは画面の点 / 秒（情報の無い入力は 0）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenPoint {
    pub at: Vec2,
    pub pressure: f32,
    pub tilt: DVec2,
    pub rotation: f64,
    pub speed: f64,
}

impl ScreenPoint {
    /// 位置と筆圧だけの点（傾き・回転・速さは 0）。
    pub fn new(at: Vec2, pressure: f32) -> ScreenPoint {
        ScreenPoint {
            at,
            pressure,
            tilt: DVec2::ZERO,
            rotation: 0.0,
            speed: 0.0,
        }
    }
}

/// 道の上のダブの位置: 画面の点・筆圧・押した点からの線の長さ（画面の点）・その所の線の向き（ラジアン、画面の右から反時計回り。
/// y は上向きに測る）・傾き・回転・速さ（区間の両端の点の間で補間）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenDab {
    pub at: Vec2,
    pub pressure: f32,
    pub arc: f64,
    pub direction: f64,
    pub tilt: DVec2,
    pub rotation: f64,
    pub speed: f64,
}

/// 3D のストロークの画面の点（押した点から始める。押した点のダブは呼ぶ側が置く）。曲線（`curve`）なら入力の点の間を centripetal
/// Catmull-Rom で結び、最新の点への区間は、その先の点が来るか離すまで待たせる。曲線でなければ、点が来るたびに直線で結ぶ。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenStrokeSampler {
    previous: ScreenPoint,
    before: Vec2,
    has_before: bool,
    held: ScreenPoint,
    has_held: bool,
    curve: bool,
    /// 描いた区間の長さの合計（画面の点）。
    length: f64,
    /// 最後に測った線の向き（長さ 0 の区間では変えない）。
    direction: f64,
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

/// 区間の間隔: ダブの間隔と、デュアルブラシの 2 つ目のダブの間隔（無ければ None）。どちらも画面の単位。
pub type SegmentGaps = (f32, Option<f32>);

impl ScreenStrokeSampler {
    /// 曲線で結ぶ道（今までの 3D のストローク）。
    pub fn new(at: Vec2, pressure: f32) -> ScreenStrokeSampler {
        ScreenStrokeSampler::with_curve(ScreenPoint::new(at, pressure), true)
    }

    /// 押した点と、曲線で結ぶか（ブラシの「曲線」）。
    pub fn with_curve(first: ScreenPoint, curve: bool) -> ScreenStrokeSampler {
        ScreenStrokeSampler {
            previous: first,
            before: first.at,
            has_before: false,
            held: first,
            has_held: false,
            curve,
            length: 0.0,
            direction: 0.0,
        }
    }

    /// 最後に描いた点。
    pub fn last_point(&self) -> Vec2 {
        self.previous.at
    }

    /// 描いた区間の長さの合計（画面の点。待たせている区間は入らない）。
    pub fn length(&self) -> f64 {
        self.length
    }

    /// 新しい入力の点（[`ScreenStrokeSampler::add_point`] の、位置と筆圧だけで、ダブの位置と筆圧だけを返す形）。
    pub fn add(
        &mut self,
        at: Vec2,
        pressure: f32,
        mut spacing: impl FnMut(Vec2) -> f32,
        out: &mut Vec<(Vec2, f32)>,
    ) -> Result<(), TooManyDabs> {
        let mut dabs = Vec::new();
        self.add_point(
            ScreenPoint::new(at, pressure),
            |a| (spacing(a), None),
            &mut dabs,
            &mut Vec::new(),
        )?;
        out.extend(dabs.iter().map(|d| (d.at, d.pressure)));
        Ok(())
    }

    /// 離したとき（[`ScreenStrokeSampler::finish_points`] の、ダブの位置と筆圧だけを返す形）。
    pub fn finish(
        &mut self,
        mut spacing: impl FnMut(Vec2) -> f32,
        out: &mut Vec<(Vec2, f32)>,
    ) -> Result<(), TooManyDabs> {
        let mut dabs = Vec::new();
        self.finish_points(|a| (spacing(a), None), &mut dabs, &mut Vec::new())?;
        out.extend(dabs.iter().map(|d| (d.at, d.pressure)));
        Ok(())
    }

    /// 新しい入力の点。曲線なら、待たせていた点までの区間を、この点で向きを決めた曲線にしてダブの位置を out に足し、この点を待たせる。
    /// 曲線でなければ、前の点からこの点までの線分にダブを置く。gaps は区間の始まりの点を受け、そこでのダブの間隔と、デュアルブラシの
    /// 2 つ目のダブの間隔（画面の単位）を返す。2 つ目のダブの位置と線の長さは dual に足す。
    pub fn add_point(
        &mut self,
        point: ScreenPoint,
        gaps: impl FnMut(Vec2) -> SegmentGaps,
        out: &mut Vec<ScreenDab>,
        dual: &mut Vec<(Vec2, f64)>,
    ) -> Result<(), TooManyDabs> {
        if !self.curve {
            if StrokeCurve::coincident(
                self.previous.at.x as f64,
                self.previous.at.y as f64,
                point.at.x as f64,
                point.at.y as f64,
            ) {
                self.previous = ScreenPoint {
                    at: self.previous.at,
                    ..point
                }; // 動かない入力は筆圧などだけ
                return Ok(());
            }
            let (a, b) = (self.previous, point);
            self.place(a, b, &[a.at, b.at], gaps, out, dual)?;
            self.previous = b;
            return Ok(());
        }
        if !self.has_held {
            if StrokeCurve::coincident(
                self.previous.at.x as f64,
                self.previous.at.y as f64,
                point.at.x as f64,
                point.at.y as f64,
            ) {
                self.previous = ScreenPoint {
                    at: self.previous.at,
                    ..point
                }; // 動かない入力は筆圧などだけ
            } else {
                self.held = point;
                self.has_held = true;
            }
            return Ok(());
        }
        if StrokeCurve::coincident(
            self.held.at.x as f64,
            self.held.at.y as f64,
            point.at.x as f64,
            point.at.y as f64,
        ) {
            self.held = ScreenPoint {
                at: self.held.at,
                ..point
            };
            return Ok(());
        }
        self.segment(point.at, gaps, out, dual)?;
        self.held = point;
        Ok(())
    }

    /// 離したとき: 曲線なら、待たせている最後の区間を、その先を折り返した向きで描く（曲線でなければ待たせている区間は無い）。
    pub fn finish_points(
        &mut self,
        gaps: impl FnMut(Vec2) -> SegmentGaps,
        out: &mut Vec<ScreenDab>,
        dual: &mut Vec<(Vec2, f64)>,
    ) -> Result<(), TooManyDabs> {
        if !self.has_held {
            return Ok(());
        }
        let (x, y) = StrokeCurve::reflect(
            self.previous.at.x as f64,
            self.previous.at.y as f64,
            self.held.at.x as f64,
            self.held.at.y as f64,
        );
        self.segment(Vec2::new(x as f32, y as f32), gaps, out, dual)?;
        self.has_held = false;
        Ok(())
    }

    /// previous → held の区間（前は before、後は next で向きを決める）に、画面の間隔でダブを置く（C# の PaintSurfaceSegment）。
    fn segment(
        &mut self,
        next: Vec2,
        gaps: impl FnMut(Vec2) -> SegmentGaps,
        out: &mut Vec<ScreenDab>,
        dual: &mut Vec<(Vec2, f64)>,
    ) -> Result<(), TooManyDabs> {
        let (a, b) = (self.previous, self.held);
        let before = if self.has_before {
            self.before
        } else {
            let (rx, ry) =
                StrokeCurve::reflect(b.at.x as f64, b.at.y as f64, a.at.x as f64, a.at.y as f64);
            Vec2::new(rx as f32, ry as f32)
        };
        // 曲線を細かい折れ線にする（長さを測り、長さで等分した位置にダブを置く）
        let pieces = ((distance(a.at, b.at) / 2.0).ceil() as i32).clamp(4, 256) as usize;
        let mut points = vec![Vec2::ZERO; pieces + 1];
        points[0] = a.at;
        for (i, p) in points.iter_mut().enumerate().skip(1) {
            *p = if i == pieces {
                b.at
            } else {
                let (x, y) = StrokeCurve::point(
                    before.x as f64,
                    before.y as f64,
                    a.at.x as f64,
                    a.at.y as f64,
                    b.at.x as f64,
                    b.at.y as f64,
                    next.x as f64,
                    next.y as f64,
                    i as f64 / pieces as f64,
                );
                Vec2::new(x as f32, y as f32)
            };
        }
        self.place(a, b, &points, gaps, out, dual)?;
        self.before = a.at;
        self.has_before = true;
        self.previous = b;
        Ok(())
    }

    /// 折れ線 points（a.at から b.at まで）を長さで等分した位置にダブを置く（区間の始まりの点での間隔）。デュアルブラシの 2 つ目のダブも
    /// 同じ折れ線に自分の間隔で等分して置く。筆圧・傾き・回転・速さは折れ線の上の割合で a から b へ補間する。
    fn place(
        &mut self,
        a: ScreenPoint,
        b: ScreenPoint,
        points: &[Vec2],
        mut gaps: impl FnMut(Vec2) -> SegmentGaps,
        out: &mut Vec<ScreenDab>,
        dual: &mut Vec<(Vec2, f64)>,
    ) -> Result<(), TooManyDabs> {
        let pieces = points.len() - 1;
        let mut lengths = vec![0f32; pieces + 1];
        for i in 1..=pieces {
            lengths[i] = lengths[i - 1] + distance(points[i - 1], points[i]);
        }
        let total = lengths[pieces];
        let (gap, dual_gap) = gaps(a.at);
        let steps = ((total / gap).ceil() as i64).max(1);
        if !total.is_finite() || steps > SURFACE_DABS_PER_SEGMENT as i64 {
            return Err(TooManyDabs);
        }
        let dual_steps = match dual_gap {
            Some(g) => {
                let n = ((total / g).ceil() as i64).max(1);
                if n > SURFACE_DABS_PER_SEGMENT as i64 {
                    return Err(TooManyDabs);
                }
                n as usize
            }
            None => 0,
        };
        // 長さで等分した i 番目（全部で steps）の位置: 折れ線の何本目（j）と、その中の割合（f）
        let locate = |i: usize, steps: usize, j: &mut usize| -> (Vec2, f32) {
            let s = if i == steps {
                total
            } else {
                total * i as f32 / steps as f32
            };
            while *j < pieces && lengths[*j] < s {
                *j += 1;
            }
            let span = lengths[*j] - lengths[*j - 1];
            let f = if span > 0.0 {
                super::unity::clamp01((s - lengths[*j - 1]) / span)
            } else {
                1.0
            };
            let at = if i == steps {
                b.at
            } else {
                lerp2(points[*j - 1], points[*j], f)
            };
            (at, f)
        };
        let base = self.length;
        let mut j = 1usize;
        for i in 1..=dual_steps {
            let (at, _) = locate(i, dual_steps, &mut j);
            let s = if i == dual_steps {
                total
            } else {
                total * i as f32 / dual_steps as f32
            };
            dual.push((at, base + s as f64));
        }
        let steps = steps as usize;
        let mut j = 1usize;
        for i in 1..=steps {
            let (at, f) = locate(i, steps, &mut j);
            let s = if i == steps {
                total
            } else {
                total * i as f32 / steps as f32
            };
            let t = ((j - 1) as f32 + f) / pieces as f32;
            // 線の向きは、その所の折れ線の 1 本の向き（画面の y は下向きなので、上向きに直して測る）
            let d = points[j] - points[j - 1];
            if d.x != 0.0 || d.y != 0.0 {
                self.direction = (-(d.y as f64)).atan2(d.x as f64);
            }
            let tl = super::unity::clamp01(t) as f64;
            out.push(ScreenDab {
                at,
                pressure: lerp(a.pressure, b.pressure, t),
                arc: base + s as f64,
                direction: self.direction,
                tilt: a.tilt + (b.tilt - a.tilt) * tl,
                rotation: crate::brush::lerp_angle(a.rotation, b.rotation, tl),
                speed: a.speed + (b.speed - a.speed) * tl,
            });
        }
        self.length = base + total as f64;
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

    /// 速い入力の長い区間（1 回の入力で塗る数を超えるダブ）も、全部の位置を出す。断るのは、ありえない長さの区間だけ。
    #[test]
    fn a_long_segment_gives_all_its_dabs_and_only_an_absurd_one_is_refused() {
        let mut s = ScreenStrokeSampler::new(Vec2::ZERO, 1.0);
        let mut out = Vec::new();
        s.add(Vec2::new(1000.0, 0.0), 1.0, |_| 1.0, &mut out)
            .unwrap();
        s.finish(|_| 1.0, &mut out).unwrap();
        assert_eq!(out.len(), 1000);
        assert!(out.len() > SURFACE_DABS_PER_EVENT);
        assert_eq!(out.last().unwrap().0, Vec2::new(1000.0, 0.0));

        let mut s = ScreenStrokeSampler::new(Vec2::ZERO, 1.0);
        let mut out = Vec::new();
        let far = SURFACE_DABS_PER_SEGMENT as f32 + 10.0;
        s.add(Vec2::new(far, 0.0), 1.0, |_| 1.0, &mut out).unwrap();
        assert_eq!(s.finish(|_| 1.0, &mut out), Err(TooManyDabs));
        assert!(out.is_empty());
        let mut s = ScreenStrokeSampler::new(Vec2::ZERO, 1.0);
        s.add(Vec2::new(f32::MAX, 0.0), 1.0, |_| 1.0, &mut out)
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
