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
    let b = path.brush.0;
    let mut painter = Painter::new(
        paints(path.channel, path.brush, &path.material)?,
        BrushSettings { radius: 1.0, ..b },
        o,
    )?;
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
    let mut dab = |hit: SurfaceHit, pressure: f64| -> Result<(), Error> {
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
        let result = g.build_surface_dabs(
            &hit,
            radius,
            o.width as i32,
            o.height as i32,
            hit.position + hit.normal * (radius * 4.0).max(1e-5),
            b.hardness as f32,
            &o.surface_budget,
            None,
            false,
        );
        if let Some(e) = result.refusal {
            return Err(Error::Dab(e));
        }
        for p in result.pixels {
            painter.pixel(p.x, p.y, p.coverage as f64, pressure)?;
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
            let substeps = (chord as f64 / (step * 0.25)).ceil().max(1.0);
            if substeps > 1_000_000.0 {
                return Err(Error::TooManySamples);
            }
            let mut previous = p[1];
            for k in 0..=substeps as usize {
                o.check()?;
                samples += 1;
                let t = k as f32 / substeps as f32;
                let q = if k == 0 { p[1] } else { curve(p, t) };
                carried += if k == 0 {
                    0.0
                } else {
                    magnitude(previous - q) as f64
                };
                previous = q;
                if carried < step {
                    continue;
                }
                carried = 0.0;
                let normal = normalized(slerp(normals[s], normals[s + 1], t));
                let reach = ((step as f32) * 4.0)
                    .max(chord * 0.25)
                    .max((b.radius as f32) * 4.0);
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
                    )?;
                } else {
                    gaps += 1;
                }
            }
        }
    }
    painter.finish(samples, dabs, gaps)
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
