use super::render::Painter;
use super::*;
use crate::geometry::{
    unity::{cross, dot, magnitude, normalized},
    Ray, SurfaceGeometry, SurfaceHit,
};
use glam::Vec3;
use sha2::{Digest, Sha256};

/// C# と同じ: 三角形数・レンダラー・スロット・UV の little endian 列の SHA-256 の先頭 16 バイト。
/// 位置・ポーズ・世代・マテリアル番号は含めない。
pub fn fingerprint(g: &SurfaceGeometry) -> String {
    let mut h = Sha256::new();
    h.update((g.triangle_count() as i32).to_le_bytes());
    for t in g.triangles() {
        h.update(t.renderer.to_le_bytes());
        h.update(t.material_slot.to_le_bytes());
        for uv in [t.uv_a, t.uv_b, t.uv_c] {
            h.update(uv.x.to_le_bytes());
            h.update(uv.y.to_le_bytes());
        }
    }
    h.finalize()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn render_surface(
    path: &SurfacePath,
    g: &SurfaceGeometry,
    o: &Options<'_>,
) -> Result<Rendered, Error> {
    path.validate()?;
    o.check()?;
    check_model(path, g)?;
    let mut painter = Painter::new(o)?;
    let (samples, dabs, gaps) = draw_surface(&mut painter, path, g, o)?;
    painter.finish(samples, dabs, gaps)
}
/// パスがこの形（指紋・三角形の数）に結び付くか。
pub(super) fn check_model(path: &SurfacePath, g: &SurfaceGeometry) -> Result<(), Error> {
    if path.model_fingerprint != fingerprint(g) {
        return Err(Error::ModelMismatch);
    }
    if path
        .points
        .iter()
        .any(|p| p.triangle as usize >= g.triangle_count())
    {
        return Err(Error::MissingTriangle);
    }
    Ok(())
}
/// 3D のパスの曲線の点（休みの形のモデルの空間）と、その点の法線・筆圧・面へ投影するレイの届く距離。
pub(super) struct CurvePoint {
    pub position: Vec3,
    pub normal: Vec3,
    pub pressure: f64,
    pub reach: f32,
}

/// 3D のパスの曲線を `spacing`（モデルの単位）ほどの間隔で辿った点（描くストロークと同じ曲線・法線の補間・筆圧の補間）。
/// 点が 1 つなら、その点だけ。
pub(super) fn surface_curve(
    path: &SurfacePath,
    g: &SurfaceGeometry,
    spacing: f32,
    o: &Options<'_>,
) -> Result<Vec<CurvePoint>, Error> {
    let ps = &path.points;
    let b = path.brush.0;
    let mut out = Vec::new();
    let place = |p: &PathPoint| {
        let t = &g.triangles()[p.triangle as usize];
        (
            t.a * (1.0 - p.u - p.v) as f32 + t.b * p.u as f32 + t.c * p.v as f32,
            t.normal(),
        )
    };
    let positions: Vec<(Vec3, Vec3)> = ps.iter().map(place).collect();
    if ps.len() == 1 {
        out.push(CurvePoint {
            position: positions[0].0,
            normal: positions[0].1,
            pressure: ps[0].pressure,
            reach: (b.radius as f32 * 4.0).max(1e-5),
        });
        return Ok(out);
    }
    let spacing = spacing.max(1e-7);
    for s in 0..ps.len().saturating_sub(1) {
        o.check()?;
        let p = [
            positions[s.saturating_sub(1)].0,
            positions[s].0,
            positions[s + 1].0,
            positions[(s + 2).min(ps.len() - 1)].0,
        ];
        let bezier = super::bezier::surface_controls(p, [ps[s].tangent, ps[s + 1].tangent]);
        let chord = bezier.map_or(magnitude(p[2] - p[1]), super::bezier::length3);
        let substeps = (chord / spacing).ceil().max(1.0);
        if substeps > 1_000_000.0 || out.len() + substeps as usize > 4_000_000 {
            return Err(Error::TooManySamples);
        }
        let reach = match path.style.depth {
            Some(d) => (b.radius * d) as f32,
            None => (spacing * 4.0).max(chord * 0.25).max(b.radius as f32 * 4.0),
        };
        for k in usize::from(s > 0)..=substeps as usize {
            let t = k as f32 / substeps;
            let q = match (k, bezier) {
                (0, _) => p[1],
                (_, None) => curve(p, t),
                (_, Some(c)) => super::bezier::eval3(c, t),
            };
            out.push(CurvePoint {
                position: q,
                normal: normalized(slerp(positions[s].1, positions[s + 1].1, t)),
                pressure: ps[s].pressure + (ps[s + 1].pressure - ps[s].pressure) * t as f64,
                reach,
            });
        }
    }
    Ok(out)
}

/// 曲線の点を面へ投影する（描くストロークと同じレイ: 法線の上から下へ、当たらなければ下から上へ。向きが法線と 0.2 より合う面だけ）。
pub(super) fn project(g: &SurfaceGeometry, q: &CurvePoint) -> Option<SurfaceHit> {
    let (normal, reach) = (q.normal, q.reach);
    g.raycast(
        Ray::new(q.position + normal * reach, -normal),
        true,
        reach * 2.0,
    )
    .filter(|h| dot(h.normal, normal) > 0.2)
    .or_else(|| {
        g.raycast(
            Ray::new(q.position - normal * reach, normal),
            false,
            reach * 2.0,
        )
        .filter(|h| dot(h.normal, normal) > 0.2)
    })
}

/// 3D のパスを作業面へ描く（ストロークを始めて終える）。鏡の対称なら、映したパスも続けて描く（映せない点があれば、その写しは
/// 描かず、点の数を欠落に数える）。サンプル・ダブ・欠落の数を返す。
pub(super) fn draw_surface(
    painter: &mut Painter<'_, '_>,
    path: &SurfacePath,
    g: &SurfaceGeometry,
    o: &Options<'_>,
) -> Result<(usize, usize, usize), Error> {
    let (mut samples, mut dabs, mut gaps) = draw_surface_one(painter, path, g, o)?;
    if let PathSymmetry::Mirror { point, normal } = path.style.symmetry {
        o.check()?;
        match mirrored_surface(path, g, point, normal) {
            Some(copy) => {
                let (s, d, k) = draw_surface_one(painter, &copy, g, o)?;
                samples += s;
                dabs += d;
                gaps += k;
            }
            None => gaps += path.points.len(),
        }
    }
    Ok((samples, dabs, gaps))
}

/// 鏡の面で映したパス（点は映した位置のいちばん近い面へ。同じマテリアル・向きの合う面・許す距離の内側。置けない点があれば None）。
fn mirrored_surface(
    path: &SurfacePath,
    g: &SurfaceGeometry,
    point: Vec3,
    normal: Vec3,
) -> Option<SurfacePath> {
    let plane = crate::geometry::MirrorPlane::new(point, normal, Vec3::X, Vec3::Y).ok()?;
    let tolerance = super::rebind_tolerance(g, path);
    let mut points = Vec::with_capacity(path.points.len());
    for p in &path.points {
        let t = g.triangles().get(p.triangle as usize)?;
        let (position, n) = super::point_position(g, p)?;
        let hit = g
            .find_closest_point(
                plane.reflect(position),
                tolerance,
                plane.reflect_direction(n),
                super::REBIND_MAX_NODE_VISITS,
                t.material,
            )
            .ok()??;
        let tangent = match p.tangent {
            Tangent::Handles { incoming, outgoing } => Tangent::Handles {
                incoming: plane.reflect_direction(incoming),
                outgoing: plane.reflect_direction(outgoing),
            },
            other => other,
        };
        points.push(
            super::point_of(&hit, p.pressure)
                .ok()?
                .with_tangent(tangent),
        );
    }
    Some(SurfacePath {
        points,
        style: PathStyle {
            symmetry: PathSymmetry::None,
            ..path.style.clone()
        },
        ..path.clone()
    })
}

fn draw_surface_one(
    painter: &mut Painter<'_, '_>,
    path: &SurfacePath,
    g: &SurfaceGeometry,
    o: &Options<'_>,
) -> Result<(usize, usize, usize), Error> {
    match path.style.kind {
        PathKind::Fill => return super::fill::draw_surface(painter, path, g, o),
        PathKind::Ribbon(r) => return super::ribbon::draw_surface(painter, path, g, o, r),
        PathKind::Stroke | PathKind::Smudge { .. } | PathKind::Erase => {}
    }
    let smudge = matches!(path.style.kind, PathKind::Smudge { .. });
    let b = path.brush.0;
    painter.begin(
        paints(path.channel, path.brush, &path.material)?,
        &super::render::kind_brush(BrushSettings { radius: 1.0, ..b }, &path.style),
    )?;
    // 指先: 直前のダブの中心（キャンバスの画素）と、ダブの画素の広がり（継ぎ目をまたいだかを見る）
    let mut previous: Option<glam::DVec2> = None;
    let ps = &path.points;
    let positions: Vec<_> = ps
        .iter()
        .map(|p| {
            let t = &g.triangles()[p.triangle as usize];
            t.a * (1.0 - p.u - p.v) as f32 + t.b * p.u as f32 + t.c * p.v as f32
        })
        .collect();
    let normals: Vec<_> = ps
        .iter()
        .map(|p| g.triangles()[p.triangle as usize].normal())
        .collect();
    let (mut dabs, mut gaps, mut samples) = (0, 0, 0);
    let tip = path.style.tip.clone();
    let mut dab = |hit: SurfaceHit, pressure: f64, direction: Vec3| -> Result<(), Error> {
        o.check()?;
        if b.pressure_size && pressure <= 0.0 {
            return Ok(());
        }
        let radius = (b.radius
            * if b.pressure_size {
                pressure.max(0.001)
            } else {
                1.0
            }) as f32;
        // 筆先の画像は正方形にかかるので、角まで届く球（半径 × √2）の画素を集め、覆いは筆先から読む
        let reach = if tip.is_some() {
            radius * std::f32::consts::SQRT_2
        } else {
            radius
        };
        // 丸い筆先は縁のアンチエイリアス（筆先の画像は覆いを筆先から読むので、今のまま）
        let mut result = g.build_surface_dabs_anti_aliased(
            &hit,
            reach,
            o.width as i32,
            o.height as i32,
            hit.position + hit.normal * (reach * 4.0).max(1e-5),
            if tip.is_some() {
                1.0
            } else {
                b.hardness as f32
            },
            if tip.is_some() {
                crate::AntiAlias::None
            } else {
                b.anti_alias
            },
            &o.surface_budget,
            None,
            false,
        );
        if let Some(e) = result.refusal {
            return Err(Error::Dab(e));
        }
        if let Some(tip) = &tip {
            let frame = TipFrame::new(hit.normal, direction, &path.style, radius);
            result.pixels.retain_mut(|p| {
                p.coverage = frame.coverage(tip, p.position - hit.position) as f32;
                p.coverage > 0.0
            });
        }
        if smudge {
            // 指先は、ダブの画素をまとめて塗る（読み元を凍結してから、中心の動きで引きずる）。中心が前のダブから、ダブの広がりの
            // 2 倍より遠くへ飛んだら UV の継ぎ目をまたいだので、前の位置を忘れる
            let center = glam::DVec2::new(
                hit.uv.x as f64 * o.width as f64,
                hit.uv.y as f64 * o.height as f64,
            );
            let (lo, hi) = result.pixels.iter().fold(
                ((i32::MAX, i32::MAX), (i32::MIN, i32::MIN)),
                |(lo, hi), p| {
                    (
                        (lo.0.min(p.x), lo.1.min(p.y)),
                        (hi.0.max(p.x), hi.1.max(p.y)),
                    )
                },
            );
            let size = (hi.0 - lo.0).max(hi.1 - lo.1).max(1) as f64;
            if previous.is_some_and(|c| c.distance(center) > size * 2.0) {
                painter.reset_direction()?;
            }
            previous = Some(center);
            let pixels: Vec<crate::BrushPixel> = result
                .pixels
                .iter()
                .map(|p| crate::BrushPixel {
                    x: p.x as i64,
                    y: p.y as i64,
                    coverage: p.coverage as f64,
                })
                .collect();
            painter.dab(&pixels, center, pressure)?;
        } else {
            for p in result.pixels {
                painter.pixel(p.x, p.y, p.coverage as f64, pressure)?;
            }
        }
        dabs += 1;
        Ok(())
    };
    if ps.len() == 1 {
        let p = ps[0];
        let t = &g.triangles()[p.triangle as usize];
        dab(
            SurfaceHit {
                revision: g.revision(),
                renderer: t.renderer,
                material_slot: t.material_slot,
                material: t.material,
                triangle: p.triangle,
                position: positions[0],
                normal: normals[0],
                barycentric: Vec3::new((1.0 - p.u - p.v) as f32, p.u as f32, p.v as f32),
                uv: t.uv_a * (1.0 - p.u - p.v) as f32 + t.uv_b * p.u as f32 + t.uv_c * p.v as f32,
                distance: 0.0,
            },
            p.pressure,
            Vec3::ZERO,
        )?;
        samples = 1;
    } else if ps.len() > 1 {
        let step = (b.radius * 2.0 * b.spacing).max(1e-6);
        let mut carried = step;
        for s in 0..ps.len() - 1 {
            o.check()?;
            let p = [
                positions[s.saturating_sub(1)],
                positions[s],
                positions[s + 1],
                positions[(s + 2).min(ps.len() - 1)],
            ];
            let chord = magnitude(p[2] - p[1]);
            // 両端が滑らかなら今までの Catmull–Rom（同じバイト）、角・取っ手があればベジェ（分割の数と届く距離は制御多角形の長さで）
            let bezier = super::bezier::surface_controls(p, [ps[s].tangent, ps[s + 1].tangent]);
            let chord = bezier.map_or(chord, super::bezier::length3);
            let substeps = (chord as f64 / (step * 0.25)).ceil().max(1.0);
            if substeps > 1_000_000.0 {
                return Err(Error::TooManySamples);
            }
            let mut previous = p[1];
            for k in 0..=substeps as usize {
                o.check()?;
                samples += 1;
                let t = k as f32 / substeps as f32;
                let q = match (k, bezier) {
                    (0, _) => p[1],
                    (_, None) => curve(p, t),
                    (_, Some(c)) => super::bezier::eval3(c, t),
                };
                carried += if k == 0 {
                    0.0
                } else {
                    magnitude(previous - q) as f64
                };
                // 筆先をパスに沿わせる向き（区間の始めは区間の向き）
                let direction = if k == 0 { p[2] - p[1] } else { q - previous };
                previous = q;
                if carried < step {
                    continue;
                }
                carried = 0.0;
                let normal = normalized(slerp(normals[s], normals[s + 1], t));
                let reach = match path.style.depth {
                    Some(d) => (b.radius * d) as f32,
                    None => ((step as f32) * 4.0)
                        .max(chord * 0.25)
                        .max((b.radius as f32) * 4.0),
                };
                let hit = g
                    .raycast(Ray::new(q + normal * reach, -normal), true, reach * 2.0)
                    .filter(|h| dot(h.normal, normal) > 0.2)
                    .or_else(|| {
                        g.raycast(Ray::new(q - normal * reach, normal), false, reach * 2.0)
                            .filter(|h| dot(h.normal, normal) > 0.2)
                    });
                if let Some(h) = hit {
                    dab(
                        h,
                        ps[s].pressure + (ps[s + 1].pressure - ps[s].pressure) * t as f64,
                        direction,
                    )?;
                } else {
                    gaps += 1;
                }
            }
        }
    }
    painter.end()?;
    Ok((samples, dabs, gaps))
}
/// 3D の筆先の向き: 面の法線に直交する面で、筆先の横の軸（`follow` ならパスの進む向き、そうでなければモデルの +Y を面に落とした
/// 向き）を角度だけ回した座標。
struct TipFrame {
    x: Vec3,
    y: Vec3,
    radius: f32,
}

impl TipFrame {
    fn new(normal: Vec3, direction: Vec3, style: &PathStyle, radius: f32) -> TipFrame {
        let flat = |v: Vec3| v - normal * dot(v, normal);
        let mut reference = if style.follow {
            flat(direction)
        } else {
            flat(Vec3::Y)
        };
        for fallback in [Vec3::Y, Vec3::X, Vec3::Z] {
            if magnitude(reference) > 1e-6 {
                break;
            }
            reference = flat(fallback);
        }
        let x0 = normalized(reference);
        let y0 = cross(normal, x0);
        let (sin, cos) = (style.angle.to_radians() as f32).sin_cos();
        TipFrame {
            x: x0 * cos + y0 * sin,
            y: y0 * cos - x0 * sin,
            radius: radius.max(1e-12),
        }
    }

    /// ダブの中心からの向き `d` の点の覆い（0〜1）。筆先の長い辺が直径にかかる。
    fn coverage(&self, tip: &crate::BrushTip, d: Vec3) -> f64 {
        let (w, h) = (tip.width() as f64, tip.height() as f64);
        let long = w.max(h);
        let lx = (dot(d, self.x) / self.radius) as f64;
        let ly = (dot(d, self.y) / self.radius) as f64;
        tip.sample((lx * long / w + 1.0) * 0.5, (ly * long / h + 1.0) * 0.5)
    }
}

fn curve(p: [Vec3; 4], t: f32) -> Vec3 {
    let knot = |a: Vec3, b: Vec3| magnitude(a - b).sqrt().max(1e-6);
    let t1 = knot(p[0], p[1]);
    let t2 = t1 + knot(p[1], p[2]);
    let t3 = t2 + knot(p[2], p[3]);
    let u = t1 + (t2 - t1) * t;
    let l = |a: Vec3, b: Vec3, ta: f32, tb: f32| {
        a * ((tb - u) / (tb - ta)) + b * ((u - ta) / (tb - ta))
    };
    let a1 = l(p[0], p[1], 0.0, t1);
    let a2 = l(p[1], p[2], t1, t2);
    let a3 = l(p[2], p[3], t2, t3);
    l(l(a1, a2, 0.0, t2), l(a2, a3, t1, t3), t1, t2)
}
// Unity のネイティブ Slerp は Mono 単体で呼べない。平行以外の法線では三角関数の最下位ビットまでの一致は保証しない。
fn slerp(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    let ma = magnitude(a);
    let mb = magnitude(b);
    if ma < 1e-6 || mb < 1e-6 {
        return a + (b - a) * t;
    }
    let an = a / ma;
    let bn = b / mb;
    let d = dot(an, bn).clamp(-1.0, 1.0);
    if d > 0.9995 {
        return a + (b - a) * t;
    }
    let theta = d.acos();
    let relative = bn - an * d;
    let direction = if magnitude(relative) > 1e-6 {
        normalized(relative)
    } else {
        normalized(cross(
            an,
            if an.x.abs() < an.y.abs() {
                Vec3::X
            } else {
                Vec3::Y
            },
        ))
    };
    (an * (theta * t).cos() + direction * (theta * t).sin()) * (ma + (mb - ma) * t)
}
