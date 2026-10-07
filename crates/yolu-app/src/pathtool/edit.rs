//! パスの点の編集（足す・差し込む・動かす・消す・閉じる・開く・太さ）。点の並びだけを扱う純粋な関数で、文書は変えない
//! （描き直して層へ入れるのは `pathtool` の側）。
//!
//! 閉じたパス = 最後の点が最初の点と同じ場所にある（4 点以上）。core の曲線は点を順に通る開いたもので、閉じているという持ち方が
//! 無く、形式も変えられないので、閉じるときは始めの点をもう 1 つ終わりに足す。閉じたパスは、始めの点と終わりの点をいつも一緒に
//! 動かし、足す点は終わりの点の前に入れ、消すときは輪から外してまた閉じる。

use yolu_core::glam::{DVec2, Vec3};
use yolu_core::paths::{CanvasPath, CanvasPoint, PathPoint, SurfacePath, Tangent, MAX_POINTS};
use yolu_core::LayerPath;

/// 接線の値（2D・3D で共通の形。取っ手は倍精度の 3 成分で、2D は z を使わない。3D は休みの形のモデルの空間）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TangentValue {
    Smooth,
    Corner,
    Handles {
        incoming: [f64; 3],
        outgoing: [f64; 3],
    },
}

/// 2D の点と 3D の点で共通の操作。
pub trait Pt: Copy {
    /// 同じ場所か（筆圧は見ない）。
    fn same_place(&self, other: &Self) -> bool;
    fn width(&self) -> f64;
    fn with_width(self, width: f64) -> Self;
    fn tangent_value(&self) -> TangentValue;
    fn with_tangent_value(self, t: TangentValue) -> Self;
}

impl Pt for CanvasPoint {
    fn same_place(&self, other: &Self) -> bool {
        self.x == other.x && self.y == other.y
    }
    fn width(&self) -> f64 {
        self.pressure
    }
    fn with_width(self, width: f64) -> Self {
        CanvasPoint {
            pressure: width.clamp(0.0, 1.0),
            ..self
        }
    }
    fn tangent_value(&self) -> TangentValue {
        match self.tangent {
            Tangent::Smooth => TangentValue::Smooth,
            Tangent::Corner => TangentValue::Corner,
            Tangent::Handles { incoming, outgoing } => TangentValue::Handles {
                incoming: [incoming.x, incoming.y, 0.0],
                outgoing: [outgoing.x, outgoing.y, 0.0],
            },
        }
    }
    fn with_tangent_value(self, t: TangentValue) -> Self {
        self.with_tangent(match t {
            TangentValue::Smooth => Tangent::Smooth,
            TangentValue::Corner => Tangent::Corner,
            TangentValue::Handles { incoming, outgoing } => Tangent::Handles {
                incoming: DVec2::new(incoming[0], incoming[1]),
                outgoing: DVec2::new(outgoing[0], outgoing[1]),
            },
        })
    }
}

impl Pt for PathPoint {
    fn same_place(&self, other: &Self) -> bool {
        self.triangle == other.triangle && self.u == other.u && self.v == other.v
    }
    fn width(&self) -> f64 {
        self.pressure
    }
    fn with_width(self, width: f64) -> Self {
        PathPoint {
            pressure: width.clamp(0.0, 1.0),
            ..self
        }
    }
    fn tangent_value(&self) -> TangentValue {
        match self.tangent {
            Tangent::Smooth => TangentValue::Smooth,
            Tangent::Corner => TangentValue::Corner,
            Tangent::Handles { incoming, outgoing } => TangentValue::Handles {
                incoming: incoming.as_dvec3().to_array(),
                outgoing: outgoing.as_dvec3().to_array(),
            },
        }
    }
    fn with_tangent_value(self, t: TangentValue) -> Self {
        let v = |a: [f64; 3]| Vec3::new(a[0] as f32, a[1] as f32, a[2] as f32);
        self.with_tangent(match t {
            TangentValue::Smooth => Tangent::Smooth,
            TangentValue::Corner => Tangent::Corner,
            TangentValue::Handles { incoming, outgoing } => Tangent::Handles {
                incoming: v(incoming),
                outgoing: v(outgoing),
            },
        })
    }
}

/// 点の場所（2D は画素の座標、3D は三角形と重心座標）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Place {
    Canvas { x: f64, y: f64 },
    Surface { triangle: u32, u: f64, v: f64 },
}

/// 点の操作。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointOp {
    /// 終わりに足す（閉じていれば閉じる点の前）。太さは前の点と同じ。
    Add(Place),
    /// 区間 `segment`（その点と次の点の間）に差し込む。太さは両側の平均。
    Insert {
        segment: usize,
        place: Place,
    },
    /// 点を動かす（閉じたパスの始め・終わりは一緒に）。太さは変えない。
    Move {
        index: usize,
        place: Place,
    },
    Remove(usize),
    /// 始めの点を終わりに足す（3 点以上で、まだ閉じていないとき）。
    Close,
    /// 終わりの点（始めと同じ場所）を外す。
    Open,
    /// 点の太さ 0〜1（筆圧の代わり。閉じたパスの始め・終わりは一緒に）。
    Width {
        index: usize,
        pressure: f64,
    },
    /// 点の接線（閉じたパスの始め・終わりは一緒に）。
    Tangent {
        index: usize,
        tangent: TangentValue,
    },
    /// 角と滑らかを切り替える（滑らか → 角、角・取っ手 → 滑らか）。
    ToggleCorner(usize),
}

/// 断る理由（文は画面の側が言語で作る）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// 点は 4096 個まで。
    TooMany,
    /// 閉じるには 3 点以上。
    NeedThree,
    AlreadyClosed,
    NotClosed,
    /// その番号の点・区間が無い。
    NoPoint,
    /// 2D の場所を 3D のパスに（またはその逆）。
    OtherKind,
    /// 点の値が範囲外（core の検査の文）。
    Invalid(&'static str),
}

pub fn is_closed<P: Pt>(points: &[P]) -> bool {
    points.len() >= 4 && points[0].same_place(&points[points.len() - 1])
}

/// 「最後の点」: 閉じたパスは、終わりの点（始めの複製）の 1 つ前。
pub fn last_index<P: Pt>(points: &[P]) -> Option<usize> {
    match points.len() {
        0 => None,
        n if is_closed(points) => Some(n - 2),
        n => Some(n - 1),
    }
}

fn append<P: Pt>(points: &[P], point: P) -> Result<(Vec<P>, usize), Refusal> {
    if points.len() >= MAX_POINTS {
        return Err(Refusal::TooMany);
    }
    let mut next = points.to_vec();
    let at = if is_closed(points) {
        next.len() - 1
    } else {
        next.len()
    };
    next.insert(at, point);
    Ok((next, at))
}

fn insert_after<P: Pt>(points: &[P], segment: usize, point: P) -> Result<(Vec<P>, usize), Refusal> {
    if segment + 1 >= points.len() {
        return Err(Refusal::NoPoint);
    }
    if points.len() >= MAX_POINTS {
        return Err(Refusal::TooMany);
    }
    let mut next = points.to_vec();
    next.insert(segment + 1, point);
    Ok((next, segment + 1))
}

fn remove<P: Pt>(points: &[P], index: usize) -> Result<Vec<P>, Refusal> {
    if index >= points.len() {
        return Err(Refusal::NoPoint);
    }
    if !is_closed(points) {
        let mut next = points.to_vec();
        next.remove(index);
        return Ok(next);
    }
    // 輪（終わりの複製を除いたもの）から外して、また閉じる。外した結果 3 点未満なら開いたままにする
    let mut ring = points[..points.len() - 1].to_vec();
    ring.remove(if index == points.len() - 1 { 0 } else { index });
    if ring.len() >= 3 {
        ring.push(ring[0]);
    }
    Ok(ring)
}

fn close<P: Pt>(points: &[P]) -> Result<Vec<P>, Refusal> {
    if is_closed(points) {
        return Err(Refusal::AlreadyClosed);
    }
    if points.len() < 3 {
        return Err(Refusal::NeedThree);
    }
    if points.len() >= MAX_POINTS {
        return Err(Refusal::TooMany);
    }
    let mut next = points.to_vec();
    next.push(points[0]);
    Ok(next)
}

fn open<P: Pt>(points: &[P]) -> Result<Vec<P>, Refusal> {
    if !is_closed(points) {
        return Err(Refusal::NotClosed);
    }
    Ok(points[..points.len() - 1].to_vec())
}

/// 閉じたパスの始め・終わり（同じ点）のもう片方の番号。
fn linked<P: Pt>(points: &[P], index: usize) -> Option<usize> {
    if !is_closed(points) {
        return None;
    }
    match index {
        0 => Some(points.len() - 1),
        i if i == points.len() - 1 => Some(0),
        _ => None,
    }
}

fn run<P: Pt>(
    points: &[P],
    op: &PointOp,
    make: &dyn Fn(Place, f64) -> Result<P, Refusal>,
) -> Result<(Vec<P>, Option<usize>), Refusal> {
    match *op {
        PointOp::Add(place) => {
            let width = last_index(points).map_or(1.0, |i| points[i].width());
            let (next, at) = append(points, make(place, width)?)?;
            Ok((next, Some(at)))
        }
        PointOp::Insert { segment, place } => {
            let (a, b) = (
                points.get(segment).ok_or(Refusal::NoPoint)?,
                points.get(segment + 1).ok_or(Refusal::NoPoint)?,
            );
            let width = (a.width() + b.width()) * 0.5;
            let (next, at) = insert_after(points, segment, make(place, width)?)?;
            Ok((next, Some(at)))
        }
        PointOp::Move { index, place } => {
            let old = points.get(index).ok_or(Refusal::NoPoint)?;
            // 動かしても接線（角・取っ手）は残す
            let moved = make(place, old.width())?.with_tangent_value(old.tangent_value());
            let mut next = points.to_vec();
            next[index] = moved;
            if let Some(other) = linked(points, index) {
                next[other] = moved;
            }
            Ok((next, Some(index)))
        }
        PointOp::Remove(index) => Ok((remove(points, index)?, None)),
        PointOp::Close => {
            let next = close(points)?;
            let at = next.len() - 1;
            Ok((next, Some(at)))
        }
        PointOp::Open => Ok((open(points)?, None)),
        PointOp::Width { index, pressure } => {
            let old = *points.get(index).ok_or(Refusal::NoPoint)?;
            let mut next = points.to_vec();
            next[index] = old.with_width(pressure);
            if let Some(other) = linked(points, index) {
                next[other] = next[index];
            }
            Ok((next, Some(index)))
        }
        PointOp::Tangent { index, tangent } => set_tangent(points, index, tangent),
        PointOp::ToggleCorner(index) => {
            let old = points.get(index).ok_or(Refusal::NoPoint)?;
            let tangent = match old.tangent_value() {
                TangentValue::Smooth => TangentValue::Corner,
                _ => TangentValue::Smooth,
            };
            set_tangent(points, index, tangent)
        }
    }
}

fn set_tangent<P: Pt>(
    points: &[P],
    index: usize,
    tangent: TangentValue,
) -> Result<(Vec<P>, Option<usize>), Refusal> {
    let old = *points.get(index).ok_or(Refusal::NoPoint)?;
    let mut next = points.to_vec();
    next[index] = old.with_tangent_value(tangent);
    if let Some(other) = linked(points, index) {
        next[other] = next[index];
    }
    Ok((next, Some(index)))
}

fn invalid(e: yolu_core::paths::Error) -> Refusal {
    match e {
        yolu_core::paths::Error::Invalid(why) => Refusal::Invalid(why),
        _ => Refusal::Invalid("点の値が範囲外です"),
    }
}

/// パスに点の操作を当てた結果と、そのあと選ぶ点の番号。パス自体は変えない（ブラシ・組・ID・指紋はそのまま）。
pub fn apply(path: &LayerPath, op: &PointOp) -> Result<(LayerPath, Option<usize>), Refusal> {
    match path {
        LayerPath::Canvas(c) => {
            let make = |place: Place, width: f64| match place {
                Place::Canvas { x, y } => CanvasPoint::new(x, y, width).map_err(invalid),
                Place::Surface { .. } => Err(Refusal::OtherKind),
            };
            let (points, at) = run(&c.points, op, &make)?;
            Ok((
                LayerPath::Canvas(CanvasPath {
                    points,
                    ..c.clone()
                }),
                at,
            ))
        }
        LayerPath::Surface(s) => {
            let make = |place: Place, width: f64| match place {
                Place::Surface { triangle, u, v } => {
                    PathPoint::new(triangle, u, v, width).map_err(invalid)
                }
                Place::Canvas { .. } => Err(Refusal::OtherKind),
            };
            let (points, at) = run(&s.points, op, &make)?;
            Ok((
                LayerPath::Surface(SurfacePath {
                    points,
                    ..s.clone()
                }),
                at,
            ))
        }
    }
}

/// パスが閉じているか。
pub fn path_is_closed(path: &LayerPath) -> bool {
    match path {
        LayerPath::Canvas(c) => is_closed(&c.points),
        LayerPath::Surface(s) => is_closed(&s.points),
    }
}

/// 点の接線。
pub fn point_tangent(path: &LayerPath, index: usize) -> Option<TangentValue> {
    match path {
        LayerPath::Canvas(c) => c.points.get(index).map(Pt::tangent_value),
        LayerPath::Surface(s) => s.points.get(index).map(Pt::tangent_value),
    }
}

/// 点の太さ（筆圧の代わり）。
pub fn point_width(path: &LayerPath, index: usize) -> Option<f64> {
    match path {
        LayerPath::Canvas(c) => c.points.get(index).map(Pt::width),
        LayerPath::Surface(s) => s.points.get(index).map(Pt::width),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(x: f64, y: f64) -> CanvasPoint {
        CanvasPoint::new(x, y, 1.0).unwrap()
    }
    fn make(place: Place, width: f64) -> Result<CanvasPoint, Refusal> {
        match place {
            Place::Canvas { x, y } => CanvasPoint::new(x, y, width).map_err(invalid),
            _ => Err(Refusal::OtherKind),
        }
    }
    fn at(x: f64, y: f64) -> Place {
        Place::Canvas { x, y }
    }
    fn run_c(
        points: &[CanvasPoint],
        op: PointOp,
    ) -> Result<(Vec<CanvasPoint>, Option<usize>), Refusal> {
        run(points, &op, &make)
    }
    fn xs(points: &[CanvasPoint]) -> Vec<f64> {
        points.iter().map(|p| p.x).collect()
    }

    #[test]
    fn add_appends_and_selects_the_new_point() {
        let (p, sel) = run_c(&[], PointOp::Add(at(1.0, 1.0))).unwrap();
        assert_eq!((p.len(), sel), (1, Some(0)));
        let (p, sel) = run_c(&p, PointOp::Add(at(2.0, 2.0))).unwrap();
        assert_eq!((xs(&p), sel), (vec![1.0, 2.0], Some(1)));
    }

    #[test]
    fn a_new_point_keeps_the_width_of_the_previous_one_and_an_inserted_one_averages() {
        let (p, _) = run_c(&[], PointOp::Add(at(0.0, 0.0))).unwrap();
        let (p, _) = run_c(
            &p,
            PointOp::Width {
                index: 0,
                pressure: 0.25,
            },
        )
        .unwrap();
        let (p, _) = run_c(&p, PointOp::Add(at(10.0, 0.0))).unwrap();
        assert_eq!(p[1].pressure, 0.25);
        let (p, _) = run_c(
            &p,
            PointOp::Width {
                index: 1,
                pressure: 0.75,
            },
        )
        .unwrap();
        let (p, sel) = run_c(
            &p,
            PointOp::Insert {
                segment: 0,
                place: at(5.0, 1.0),
            },
        )
        .unwrap();
        assert_eq!((xs(&p), sel), (vec![0.0, 5.0, 10.0], Some(1)));
        assert_eq!(p[1].pressure, 0.5);
    }

    #[test]
    fn move_changes_only_the_place() {
        let pts = [c(0.0, 0.0), c(5.0, 5.0), c(9.0, 0.0)];
        let (p, sel) = run_c(
            &pts,
            PointOp::Width {
                index: 1,
                pressure: 0.4,
            },
        )
        .unwrap();
        let (p, sel2) = run_c(
            &p,
            PointOp::Move {
                index: 1,
                place: at(6.0, 7.0),
            },
        )
        .unwrap();
        assert_eq!((sel, sel2), (Some(1), Some(1)));
        assert_eq!((p[1].x, p[1].y, p[1].pressure), (6.0, 7.0, 0.4));
        assert_eq!(
            run_c(
                &p,
                PointOp::Move {
                    index: 3,
                    place: at(0.0, 0.0)
                }
            ),
            Err(Refusal::NoPoint)
        );
    }

    #[test]
    fn closing_needs_three_points_and_links_the_first_and_last() {
        let two = [c(0.0, 0.0), c(5.0, 5.0)];
        assert_eq!(run_c(&two, PointOp::Close), Err(Refusal::NeedThree));
        let three = [c(0.0, 0.0), c(5.0, 5.0), c(9.0, 0.0)];
        let (closed, sel) = run_c(&three, PointOp::Close).unwrap();
        assert!(is_closed(&closed));
        assert_eq!((closed.len(), sel), (4, Some(3)));
        assert_eq!(run_c(&closed, PointOp::Close), Err(Refusal::AlreadyClosed));
        // 始めを動かすと終わりも動く。太さも
        let (moved, _) = run_c(
            &closed,
            PointOp::Move {
                index: 0,
                place: at(1.0, 1.0),
            },
        )
        .unwrap();
        assert!(is_closed(&moved) && moved[3].x == 1.0);
        let (wide, _) = run_c(
            &closed,
            PointOp::Width {
                index: 3,
                pressure: 0.3,
            },
        )
        .unwrap();
        assert_eq!((wide[0].pressure, wide[3].pressure), (0.3, 0.3));
        // 足す点は終わりの点の前に入り、輪は閉じたまま
        let (grown, sel) = run_c(&closed, PointOp::Add(at(20.0, 20.0))).unwrap();
        assert!(is_closed(&grown));
        assert_eq!((grown.len(), sel), (5, Some(3)));
        assert_eq!(last_index(&grown), Some(3));
        // 開く
        let (opened, _) = run_c(&grown, PointOp::Open).unwrap();
        assert!(!is_closed(&opened) && opened.len() == 4);
        assert_eq!(run_c(&opened, PointOp::Open), Err(Refusal::NotClosed));
    }

    #[test]
    fn removing_from_a_ring_keeps_it_closed_until_it_is_too_small() {
        let four = [c(0.0, 0.0), c(5.0, 5.0), c(9.0, 0.0), c(4.0, -4.0)];
        let (ring, _) = run_c(&four, PointOp::Close).unwrap();
        // 始めの点を消すと、次の点が新しい始め・終わり
        let (p, _) = run_c(&ring, PointOp::Remove(0)).unwrap();
        assert!(is_closed(&p));
        assert_eq!(xs(&p), vec![5.0, 9.0, 4.0, 5.0]);
        // 終わりの点（始めの複製）を消しても同じ
        let (q, _) = run_c(&ring, PointOp::Remove(4)).unwrap();
        assert_eq!(xs(&q), xs(&p));
        // 途中の点
        let (r, _) = run_c(&ring, PointOp::Remove(2)).unwrap();
        assert!(is_closed(&r));
        assert_eq!(xs(&r), vec![0.0, 5.0, 4.0, 0.0]);
        // 3 点の輪から 1 点消すと 2 点で開く
        let (small, _) = run_c(&r, PointOp::Remove(1)).unwrap();
        assert!(!is_closed(&small));
        assert_eq!(xs(&small), vec![0.0, 4.0]);
    }

    #[test]
    fn open_paths_remove_plain_and_last_index_skips_the_closing_copy() {
        let pts = [c(0.0, 0.0), c(5.0, 5.0), c(9.0, 0.0)];
        assert_eq!(last_index(&pts), Some(2));
        assert_eq!(last_index::<CanvasPoint>(&[]), None);
        let (p, sel) = run_c(&pts, PointOp::Remove(1)).unwrap();
        assert_eq!((xs(&p), sel), (vec![0.0, 9.0], None));
        assert_eq!(run_c(&pts, PointOp::Remove(3)), Err(Refusal::NoPoint));
        let (empty, _) = run_c(&[c(1.0, 1.0)], PointOp::Remove(0)).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn the_point_limit_and_value_limits_are_refused() {
        let many: Vec<CanvasPoint> = (0..MAX_POINTS).map(|i| c(i as f64 * 0.01, 0.0)).collect();
        assert_eq!(
            run_c(&many, PointOp::Add(at(1.0, 1.0))),
            Err(Refusal::TooMany)
        );
        assert_eq!(
            run_c(
                &many,
                PointOp::Insert {
                    segment: 0,
                    place: at(1.0, 1.0)
                }
            ),
            Err(Refusal::TooMany)
        );
        assert!(matches!(
            run_c(&[], PointOp::Add(at(f64::NAN, 0.0))),
            Err(Refusal::Invalid(_))
        ));
        assert_eq!(
            run_c(
                &[],
                PointOp::Add(Place::Surface {
                    triangle: 0,
                    u: 0.1,
                    v: 0.1
                })
            ),
            Err(Refusal::OtherKind)
        );
    }

    #[test]
    fn apply_works_on_both_kinds_and_keeps_the_rest_of_the_path() {
        use yolu_core::paths::PathBrush;
        let canvas = LayerPath::Canvas(CanvasPath {
            style: Default::default(),
            id: 7,
            channel: yolu_core::Channel::Color,
            brush: PathBrush::default(),
            points: vec![],
            material: None,
        });
        let (next, sel) = apply(&canvas, &PointOp::Add(at(3.0, 4.0))).unwrap();
        assert_eq!((next.id(), next.point_count(), sel), (7, 1, Some(0)));
        assert_eq!(
            apply(
                &next,
                &PointOp::Add(Place::Surface {
                    triangle: 0,
                    u: 0.0,
                    v: 0.0
                })
            ),
            Err(Refusal::OtherKind)
        );
        let surface = LayerPath::Surface(SurfacePath {
            style: Default::default(),
            id: 8,
            channel: yolu_core::Channel::Color,
            brush: PathBrush::default(),
            points: vec![],
            model_fingerprint: "abc".into(),
            material: None,
        });
        let (next, _) = apply(
            &surface,
            &PointOp::Add(Place::Surface {
                triangle: 2,
                u: 0.25,
                v: 0.25,
            }),
        )
        .unwrap();
        let LayerPath::Surface(s) = &next else {
            panic!()
        };
        assert_eq!((s.model_fingerprint.as_str(), s.points.len()), ("abc", 1));
        assert_eq!(point_width(&next, 0), Some(1.0));
        assert!(!path_is_closed(&next));
        assert!(matches!(
            apply(
                &surface,
                &PointOp::Add(Place::Surface {
                    triangle: 0,
                    u: 0.9,
                    v: 0.9
                })
            ),
            Err(Refusal::Invalid(_))
        ));
    }
}
