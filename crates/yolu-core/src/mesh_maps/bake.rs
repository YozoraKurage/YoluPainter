use super::{
    curvature::Curvature,
    ids::{id_palette_separation, IdTable},
    input::{hash_text, length},
    raster::Raster,
    rays::Rays,
    settings::condition_key_text,
    surface::{interpolate, Frames, Surface},
    *,
};
use rayon::prelude::*;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicU8, Ordering},
    time::Instant,
};
pub const ENGINE_VERSION: i32 = 2;
pub(crate) const SPACE: &str = "SnapshotWorld";
pub(crate) const POSE: &str = "StaticSnapshot";
pub fn base_name(name: &str) -> String {
    let name = name.trim();
    let lower = name.to_lowercase();
    for suffix in ["_low", "_high"] {
        if lower.ends_with(suffix) {
            return name[..name.len() - suffix.len()].into();
        }
    }
    name.into()
}
/// 並列度の上限。1 つの行のまとまりは最大 256 行なので、これより多いスレッドは遊ぶだけで、見積もりも桁あふれしない。
pub const MAX_PARALLELISM: usize = 256;
/// 0 なら論理プロセッサの数。指定は上限として 1〜`MAX_PARALLELISM` に丸める（見積もりも同じ値で数える）。
fn threads(b: &MeshBakeBudget) -> usize {
    let requested = if b.max_degree_of_parallelism > 0 {
        b.max_degree_of_parallelism
    } else {
        std::thread::available_parallelism().map_or(1, usize::from)
    };
    requested.clamp(1, MAX_PARALLELISM)
}
/// .NET の OrdinalIgnoreCase に合わせ、1 文字ずつ大文字にする（`ß` → `SS` のように文字数が変わるものは動かさない）。
fn ordinal_ignore_case(name: &str) -> String {
    name.chars()
        .map(|c| {
            let mut upper = c.to_uppercase();
            match (upper.next(), upper.next()) {
                (Some(u), None) => u,
                _ => c,
            }
        })
        .collect()
}
fn wants_rays(s: &MeshBakeSettings) -> bool {
    s.maps.iter().any(|k| {
        matches!(
            k,
            MeshMapKind::AmbientOcclusion | MeshMapKind::BentNormal | MeshMapKind::Thickness
        )
    })
}
/// 入力の素材の識別（低ポリと高ポリのもの）。MaterialAsset の ID マップの設定の文字列に入るので、
/// `MeshMapExpectation` の `material_identity` にはこれを渡す。
pub fn material_identity(input: &MeshBakeInput, reference: Option<&MeshBakeInput>) -> String {
    hash_text(&format!(
        "{};{}",
        input.material_identity_hash,
        reference.map_or("", |r| r.material_identity_hash.as_str())
    ))
}
/// この条件で焼いたときの由来の鍵（焼いたマップの `MeshMapProvenance::condition_key` と同じ値）。以前のマップを使ってよいかの
/// 照合に使う。MaterialAsset の ID マップの素材の識別も入力から作る。C# の `MeshBaker.ConditionKey` に当たる。
pub fn condition_key(
    input: &MeshBakeInput,
    settings: &MeshBakeSettings,
    kind: MeshMapKind,
    reference: Option<&MeshBakeInput>,
) -> Result<String> {
    settings.validate()?;
    Ok(condition_key_text(
        kind,
        ENGINE_VERSION,
        &input.hash,
        input.uv_channel,
        settings.width,
        settings.height,
        settings.target_slot,
        settings.padding,
        settings.antialiasing,
        &settings.kind_key(kind, &material_identity(input, reference)),
        SPACE,
        POSE,
        &settings.source_key(reference.map(|r| r.hash.as_str())),
        &settings.targets(),
    ))
}
pub fn estimate_bytes(
    input: &MeshBakeInput,
    s: &MeshBakeSettings,
    b: &MeshBakeBudget,
    reference: Option<&MeshBakeInput>,
) -> Result<u64> {
    s.validate()?;
    let texels = s.width as u64 * s.height as u64;
    let triangles = input.triangle_count() as u64;
    let n = s.antialiasing as u64;
    let mut bytes = texels;
    for k in &s.maps {
        bytes += texels * k.channels() as u64 * 2;
    }
    if s.padding > 0 {
        bytes += texels * 5;
    }
    let surface = |t: u64, rays: bool| {
        t * (13
            + if rays { 244 } else { 0 }
            + if s.includes(MeshMapKind::Curvature) {
                192
            } else {
                0
            })
    };
    bytes +=
        surface(triangles, wants_rays(s)) + triangles * (76 + 72) + (s.height as u64 / 8 + 2) * 4;
    let ids = |t: u64| {
        t * (if s.id_source == MeshIdSource::MaterialAsset {
            160
        } else {
            8
        } + if matches!(s.id_source, MeshIdSource::MeshPart | MeshIdSource::UvIsland) {
            200
        } else {
            0
        })
    };
    if s.includes(MeshMapKind::Id) {
        bytes += ids(triangles);
        if !s.manual_id_colors.colors.is_empty() {
            bytes += triangles * 212;
        }
        if let Some(r) = reference {
            if s.id_source != MeshIdSource::UvIsland {
                bytes += ids(r.triangle_count() as u64);
            }
        }
    }
    if let Some(r) = reference {
        bytes += triangles * 36 + surface(r.triangle_count() as u64, true);
        if s.reference_match_by_name {
            bytes += r.triangle_count() as u64 * 244;
        }
    }
    bytes += threads(b) as u64 * s.width as u64 * n * n * 22 + threads(b) as u64 * 2048;
    Ok(bytes)
}
struct Control<'a> {
    cancel: Option<&'a AtomicBool>,
    start: Instant,
    max_seconds: f64,
}
impl Control<'_> {
    fn status(&self) -> MeshBakeStatus {
        if self.cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            MeshBakeStatus::Canceled
        } else if self.max_seconds > 0. && self.start.elapsed().as_secs_f64() > self.max_seconds {
            MeshBakeStatus::TimedOut
        } else {
            MeshBakeStatus::Completed
        }
    }
}
struct Projection {
    cage: Vec<f32>,
    groups: Vec<MeshRayBvh>,
    target: Vec<Option<usize>>,
}
impl Projection {
    fn new(
        low: &MeshBakeInput,
        high: &Surface,
        s: &MeshBakeSettings,
        receivers: &[usize],
    ) -> Result<Self> {
        let cage = if s.reference_average_normals {
            reconstruct_normals(&low.corners, 180.)?
        } else {
            match &low.normals {
                Some(n) => n.clone(),
                None => reconstruct_normals(&low.corners, 0.)?,
            }
        };
        let mut groups = vec![];
        let mut target = vec![None; low.triangle_count()];
        if s.reference_match_by_name {
            let name = |input: &MeshBakeInput, t: usize| {
                ordinal_ignore_case(&base_name(
                    input
                        .renderer_names
                        .as_ref()
                        .and_then(|n| n.get(input.renderers[t] as usize))
                        .map_or("", String::as_str),
                ))
            };
            let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
            for t in 0..high.input.triangle_count() {
                if high.valid[t] {
                    by_name.entry(name(high.input, t)).or_default().push(t);
                }
            }
            let mut built = HashMap::new();
            for &t in receivers {
                let key = name(low, t);
                let group = *built.entry(key.clone()).or_insert_with(|| {
                    by_name.get(&key).map(|triangles| {
                        let index = groups.len();
                        groups.push(MeshRayBvh::build(&high.input.corners, triangles));
                        index
                    })
                });
                target[t] = group;
            }
        }
        Ok(Self {
            cage,
            groups,
            target,
        })
    }
}
/// CPUベイク。進捗コールバックは呼び出したスレッドで実行し、falseで取り消す。
/// 取消と時間切れではマップを返さず、既存の正本を変更しない。
pub fn bake(
    input: &MeshBakeInput,
    settings: &MeshBakeSettings,
    budget: &MeshBakeBudget,
    cancel: Option<&AtomicBool>,
    reference: Option<&MeshBakeInput>,
    mut progress: impl FnMut(f64, &str) -> bool,
) -> Result<MeshBakeResult> {
    settings.validate()?;
    let s = settings;
    let control = Control {
        cancel,
        start: Instant::now(),
        max_seconds: budget.max_seconds,
    };
    let mut report = MeshBakeReport::default();
    let mut receivers = vec![];
    for t in 0..input.triangle_count() {
        if !s.targeted(input.slots[t]) {
            continue;
        }
        let u = &input.uvs[t * 6..t * 6 + 6];
        let area = ((u[2] - u[0]) * (u[5] - u[1]) - (u[4] - u[0]) * (u[3] - u[1])) as f64;
        if area.abs() * s.width as f64 * (s.height as f64) < 1e-9 {
            report.zero_uv_area_triangles += 1;
            continue;
        }
        if !super::surface::face_normal(&input.corners, t).1 {
            continue;
        }
        check(
            u.iter().all(|v| *v >= -1e-6f32 && *v <= 1. + 1e-6f32),
            "UVが0〜1の外です。繰り返し・UDIMのUVはベイクできません",
        )?;
        receivers.push(t);
    }
    check(
        !receivers.is_empty(),
        "対象スロットに焼き込めるUVのある三角形がありません",
    )?;
    let estimate = estimate_bytes(input, s, budget, reference)?;
    report.estimated_bytes = estimate;
    check(
        estimate <= budget.max_bytes,
        "ベイクの見積もりがメモリ予算を超えます",
    )?;
    let remaining = budget.max_bytes - estimate;
    report.receiving_triangles = receivers.len();
    macro_rules! checkpoint {
        ($fraction:expr,$phase:expr) => {{
            let status = control.status();
            if status != MeshBakeStatus::Completed || !progress($fraction, $phase) {
                report.total_seconds = control.start.elapsed().as_secs_f64();
                return Ok(MeshBakeResult {
                    status: if status == MeshBakeStatus::Completed {
                        MeshBakeStatus::Canceled
                    } else {
                        status
                    },
                    maps: vec![],
                    report,
                });
            }
        }};
    }
    checkpoint!(0., "Preparing");
    let low = Surface::new(input);
    report.degenerate_triangles = low.valid.iter().filter(|v| !**v).count();
    let occluders: Vec<_> = (0..input.triangle_count())
        .filter(|&t| {
            low.valid[t]
                && (s.occluders == MeshOccluders::WholeModel
                    || s.target_slot < 0 && s.target_slots.is_empty()
                    || s.targeted(input.slots[t]))
        })
        .collect();
    let low_bvh = if wants_rays(s) {
        report.occluder_triangles = occluders.len();
        MeshRayBvh::build(&input.corners, &occluders)
    } else {
        MeshRayBvh::build(&[], &[])
    };
    checkpoint!(0.01, "Preparing");
    let curvature = if s.includes(MeshMapKind::Curvature) {
        let c = Curvature::new(&low, s.curvature_radius * input.diagonal, remaining)?;
        report.boundary_edges = c.boundary;
        report.non_manifold_edges = c.non_manifold;
        report.inconsistent_winding_edges = c.inconsistent;
        report.curvature_segments = c.segment_count();
        Some(c)
    } else {
        None
    };
    let high = reference.map(Surface::new);
    let high_bvh = high.as_ref().map(|h| {
        let all: Vec<_> = (0..h.input.triangle_count())
            .filter(|t| h.valid[*t])
            .collect();
        report.reference_triangles = all.len();
        MeshRayBvh::build(&h.input.corners, &all)
    });
    checkpoint!(0.02, "Preparing");
    let high_curvature = high
        .as_ref()
        .filter(|_| s.includes(MeshMapKind::Curvature))
        .map(|h| Curvature::new(h, s.curvature_radius * input.diagonal, remaining))
        .transpose()?;
    let projection = high
        .as_ref()
        .map(|h| Projection::new(input, h, s, &receivers))
        .transpose()?;
    let frames = if s.includes(MeshMapKind::TangentNormal) {
        Some(Frames::new(&low, &receivers))
    } else {
        None
    };
    let ids = if s.includes(MeshMapKind::Id) {
        let ids = IdTable::new(
            input,
            reference,
            s.id_source,
            &receivers,
            &s.manual_id_colors,
        )?;
        report.id_parts = ids.parts;
        Some(ids)
    } else {
        None
    };
    let raster = Raster::new(input, s, &receivers, remaining)?;
    let rays = Rays::new(s, input.diagonal);
    let width = s.width as usize;
    let height = s.height as usize;
    let samples = s.antialiasing as usize;
    let mut min = input.min;
    let mut max = input.max;
    if let Some(r) = reference {
        for a in 0..3 {
            min[a] = min[a].min(r.min[a]);
            max[a] = max[a].max(r.max[a]);
        }
    }
    let scale: [f64; 3] = std::array::from_fn(|a| {
        if max[a] - min[a] > 1e-12 * input.diagonal {
            1. / (max[a] - min[a])
        } else {
            0.
        }
    });
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads(budget))
        .build()
        .map_err(|e| MeshMapError(e.to_string()))?;
    let mut coverage = vec![0u8; width * height];
    let mut outputs: Vec<Vec<u16>> = s
        .maps
        .iter()
        .map(|k| vec![0; width * height * k.channels()])
        .collect();
    let ray_count = AtomicU64::new(0);
    let projected = AtomicU64::new(0);
    let missed = AtomicU64::new(0);
    report.prepare_seconds = control.start.elapsed().as_secs_f64();
    checkpoint!(0.05, "Baking");
    let batch = (threads(budget) * 4).clamp(16, 256);
    for y0 in (0..height).step_by(batch) {
        let y1 = (y0 + batch).min(height);
        let mut rows: Vec<Vec<&mut [u16]>> = (y0..y1).map(|_| vec![]).collect();
        for (output, kind) in outputs.iter_mut().zip(&s.maps) {
            for (row, slice) in rows.iter_mut().zip(
                output[y0 * width * kind.channels()..y1 * width * kind.channels()]
                    .chunks_mut(width * kind.channels()),
            ) {
                row.push(slice);
            }
        }
        pool.install(|| {
            coverage[y0 * width..y1 * width]
                .par_chunks_mut(width)
                .zip(rows.into_par_iter())
                .enumerate()
                .for_each(|(dy, (coverage, mut row))| {
                    if control.status() != MeshBakeStatus::Completed {
                        return;
                    }
                    let y = y0 + dy;
                    let owners = raster.row(y, width, samples);
                    let mut local_rays = 0;
                    let mut local_projected = 0;
                    let mut local_missed = 0;
                    for (x, covered_pixel) in coverage.iter_mut().enumerate() {
                        let mut sums = [[0f64; 3]; 10];
                        let mut id_samples = [0u32; 16];
                        let mut count = 0;
                        let mut overlap = false;
                        for j in 0..samples {
                            for i in 0..samples {
                                let Some(sample) = owners.get((j * width + x) * samples + i) else {
                                    continue;
                                };
                                let t = sample.triangle;
                                let (p, g, n) = low.point(t, sample.u, sample.v);
                                let mut point = p;
                                let mut normal = n;
                                let mut face = g;
                                let mut st = t;
                                let mut hit = false;
                                let mut rise = 0.;
                                if let Some(projection) = &projection {
                                    local_projected += 1;
                                    let mut cage =
                                        interpolate(&projection.cage, t, sample.u, sample.v);
                                    let l = length(cage);
                                    cage = if l > 1e-9 { cage.map(|v| v / l) } else { n };
                                    let frontal = s.reference_frontal * input.diagonal;
                                    let rear = s.reference_rear * input.diagonal;
                                    let bvh = if s.reference_match_by_name {
                                        projection.target[t].map(|g| &projection.groups[g])
                                    } else {
                                        high_bvh.as_ref()
                                    };
                                    if let Some(h) = bvh.and_then(|b| {
                                        b.trace(
                                            std::array::from_fn(|a| p[a] + cage[a] * frontal),
                                            cage.map(|v| -v),
                                            frontal + rear,
                                            None,
                                            false,
                                            false,
                                        )
                                    }) {
                                        hit = true;
                                        st = h.triangle;
                                        rise = frontal - h.distance;
                                        (point, face, normal) =
                                            high.as_ref().unwrap().point(st, h.u, h.v);
                                    } else {
                                        local_missed += 1;
                                    }
                                }
                                for a in 0..3 {
                                    sums[0][a] += normal[a];
                                    sums[1][a] += if scale[a] > 0. {
                                        (point[a] - min[a]) * scale[a]
                                    } else {
                                        0.5
                                    };
                                }
                                if let Some(c) = if hit {
                                    high_curvature.as_ref()
                                } else {
                                    curvature.as_ref()
                                } {
                                    sums[3][0] += 0.5 + 0.5 * c.evaluate(point, c.components[st]);
                                }
                                let tangent = if hit {
                                    frames.as_ref().map_or([0., 0., 1.], |f| {
                                        f.tangent_space(t, sample.u, sample.v, normal)
                                    })
                                } else {
                                    [0., 0., 1.]
                                };
                                for (a, v) in tangent.iter().enumerate() {
                                    sums[5][a] += v;
                                }
                                let range = (s.reference_frontal * input.diagonal)
                                    .max(s.reference_rear * input.diagonal);
                                sums[6][0] += 0.5 + 0.5 * (rise / range).clamp(-1., 1.);
                                sums[9][0] += if projection.is_none() || hit { 1. } else { 0. };
                                if wants_rays(s) {
                                    let (ao, th, bent, r) = rays.trace(
                                        if hit {
                                            high_bvh.as_ref().unwrap()
                                        } else {
                                            &low_bvh
                                        },
                                        point,
                                        normal,
                                        face,
                                        st,
                                        x,
                                        y,
                                        j * samples + i,
                                    );
                                    local_rays += r;
                                    sums[2][0] += ao;
                                    sums[4][0] += th;
                                    for (a, v) in bent.iter().enumerate() {
                                        sums[8][a] += v;
                                    }
                                }
                                if let Some(ids) = &ids {
                                    id_samples[count] =
                                        ids.manual.get(t).copied().flatten().unwrap_or_else(|| {
                                            if s.id_source == MeshIdSource::UvIsland || !hit {
                                                ids.low[t]
                                            } else {
                                                ids.high[st]
                                            }
                                        });
                                }
                                count += 1;
                                overlap |= sample.overlap;
                            }
                        }
                        if count == 0 {
                            continue;
                        }
                        *covered_pixel = if overlap { 2 } else { 1 };
                        let inv = 1. / count as f64;
                        for (out, kind) in row.iter_mut().zip(&s.maps) {
                            let values = &sums[*kind as usize];
                            let at = x * kind.channels();
                            match kind {
                                MeshMapKind::WorldNormal
                                | MeshMapKind::TangentNormal
                                | MeshMapKind::BentNormal => {
                                    let l = length(*values);
                                    let n = if l > 1e-12 {
                                        values.map(|v| v / l)
                                    } else {
                                        [0., 0., 1.]
                                    };
                                    for a in 0..3 {
                                        out[at + a] = quantize(n[a] * 0.5 + 0.5);
                                    }
                                }
                                MeshMapKind::Id => {
                                    let mut best = id_samples[0];
                                    let mut best_count = 0;
                                    for &id in &id_samples[..count] {
                                        let c = id_samples[..count]
                                            .iter()
                                            .filter(|&&v| v == id)
                                            .count();
                                        if c > best_count {
                                            best_count = c;
                                            best = id;
                                        }
                                    }
                                    for a in 0..3 {
                                        out[at + a] = (((best >> (16 - 8 * a)) & 255) * 257) as u16;
                                    }
                                }
                                _ => {
                                    for a in 0..kind.channels() {
                                        out[at + a] = quantize(values[a] * inv);
                                    }
                                }
                            }
                        }
                    }
                    ray_count.fetch_add(local_rays, Ordering::Relaxed);
                    projected.fetch_add(local_projected, Ordering::Relaxed);
                    missed.fetch_add(local_missed, Ordering::Relaxed);
                })
        });
        checkpoint!(0.05 + 0.85 * y1 as f64 / height as f64, "Baking");
    }
    report.rays = ray_count.load(Ordering::Relaxed);
    report.projected_samples = projected.load(Ordering::Relaxed);
    report.missed_samples = missed.load(Ordering::Relaxed);
    report.raster_seconds = control.start.elapsed().as_secs_f64() - report.prepare_seconds;
    if s.padding > 0 {
        let source: Vec<AtomicI32> = coverage
            .iter()
            .enumerate()
            .map(|(i, c)| AtomicI32::new(if *c != 0 { i as i32 } else { -1 }))
            .collect();
        let pass: Vec<AtomicU8> = coverage
            .iter()
            .map(|c| AtomicU8::new(if *c != 0 { 0 } else { 255 }))
            .collect();
        // 由来を書いてから段番号を公開する。この段の値は読まず、前の段だけを参照する。
        for p in 1..=s.padding {
            let changed = pool.install(|| {
                (0..source.len())
                    .into_par_iter()
                    .map(|i| {
                        if pass[i].load(Ordering::Acquire) != 255 {
                            return 0usize;
                        }
                        let x = i % width;
                        let y = i / width;
                        let mut best = -1;
                        let mut distance = i64::MAX;
                        for oy in -1..=1 {
                            let ny = y as i32 + oy;
                            if ny < 0 || ny >= s.height {
                                continue;
                            }
                            for ox in -1..=1 {
                                let nx = x as i32 + ox;
                                if nx < 0 || nx >= s.width || (ox == 0 && oy == 0) {
                                    continue;
                                }
                                let j = ny as usize * width + nx as usize;
                                if pass[j].load(Ordering::Acquire) >= p as u8 {
                                    continue;
                                }
                                let from = source[j].load(Ordering::Relaxed);
                                let fx = from as i64 % width as i64 - x as i64;
                                let fy = from as i64 / width as i64 - y as i64;
                                let d = fx * fx + fy * fy;
                                if d < distance || d == distance && from < best {
                                    distance = d;
                                    best = from;
                                }
                            }
                        }
                        if best < 0 {
                            return 0;
                        }
                        source[i].store(best, Ordering::Relaxed);
                        pass[i].store(p as u8, Ordering::Release);
                        1
                    })
                    .sum::<usize>()
            });
            checkpoint!(0.9 + 0.1 * p as f64 / s.padding as f64, "Padding");
            if changed == 0 {
                break;
            }
        }
        for (i, from) in source.iter().enumerate() {
            let from = from.load(Ordering::Relaxed);
            if coverage[i] != 0 || from < 0 {
                continue;
            }
            coverage[i] = 3;
            for (out, k) in outputs.iter_mut().zip(&s.maps) {
                for c in 0..k.channels() {
                    out[i * k.channels() + c] = out[from as usize * k.channels() + c];
                }
            }
        }
    }
    report.padding_seconds =
        control.start.elapsed().as_secs_f64() - report.prepare_seconds - report.raster_seconds;
    for &c in &coverage {
        match c {
            1 => report.covered_texels += 1,
            2 => report.overlap_texels += 1,
            3 => report.padded_texels += 1,
            _ => report.empty_texels += 1,
        }
    }
    report.fallback_triangles = frames.as_ref().map_or(0, |f| f.fallback);
    let mut notes = vec![];
    if report.overlap_texels > 0 {
        notes.push(MeshBakeNote::OverlappingTexels(report.overlap_texels));
    }
    if report.zero_uv_area_triangles > 0 {
        notes.push(MeshBakeNote::ZeroUvAreaTriangles(
            report.zero_uv_area_triangles,
        ));
    }
    if input.normal_source != "authored" {
        notes.push(if input.normal_source == "face" {
            MeshBakeNote::FaceNormals
        } else {
            MeshBakeNote::ReconstructedNormals(input.normal_source.clone())
        });
    }
    if report.fallback_triangles > 0 {
        notes.push(MeshBakeNote::TangentFallback(report.fallback_triangles));
    }
    if report.non_manifold_edges + report.inconsistent_winding_edges > 0 {
        notes.push(MeshBakeNote::IgnoredEdges(
            report.non_manifold_edges + report.inconsistent_winding_edges,
        ));
    }
    if reference.is_some() && report.missed_samples > 0 {
        notes.push(MeshBakeNote::MissedSamples {
            missed: report.missed_samples,
            projected: report.projected_samples,
            by_name: s.reference_match_by_name,
        });
    }
    if ids.is_some() {
        if !s.manual_id_colors.colors.is_empty() {
            notes.push(MeshBakeNote::ManualIdColors(
                s.manual_id_colors.colors.len(),
            ));
        } else if s.id_source == MeshIdSource::VertexColor {
            if input.colors.is_none() {
                notes.push(MeshBakeNote::NoVertexColors);
            }
        } else {
            notes.push(MeshBakeNote::IdParts {
                parts: report.id_parts,
                source: s.id_source,
                separation: id_palette_separation(report.id_parts)?,
            });
        }
    }
    if reference.is_none()
        && (s.includes(MeshMapKind::TangentNormal) || s.includes(MeshMapKind::Height))
    {
        notes.push(MeshBakeNote::NoReference);
    }
    report.diagnostics = notes.iter().map(ToString::to_string).collect();
    report.notes = notes;
    let identity = material_identity(input, reference);
    let coverage = std::sync::Arc::new(coverage);
    let mut maps = vec![];
    for (kind, data) in s.maps.iter().zip(outputs) {
        let provenance = MeshMapProvenance {
            kind: *kind,
            engine_version: ENGINE_VERSION,
            mesh_hash: input.hash.clone(),
            topology_hash: input.topology_hash.clone(),
            uv_channel: input.uv_channel,
            width: s.width,
            height: s.height,
            target_slot: s.target_slot,
            target_slots: s.targets(),
            padding: s.padding,
            antialiasing: s.antialiasing,
            settings_key: s.kind_key(*kind, &identity),
            space: SPACE.into(),
            pose: POSE.into(),
            source: s.source_key(reference.map(|r| r.hash.as_str())),
            bounds_min: min,
            bounds_max: max,
        };
        maps.push(BakedMeshMap::with_coverage(
            provenance,
            data,
            coverage.clone(),
        )?);
    }
    report.total_seconds = control.start.elapsed().as_secs_f64();
    // 完了の通知。もう返す結果は決まっているので、戻り値は C# と同じく見ない。
    let _ = progress(1., "Done");
    Ok(MeshBakeResult {
        status: MeshBakeStatus::Completed,
        maps,
        report,
    })
}
fn quantize(v: f64) -> u16 {
    if v <= 0. {
        0
    } else if v >= 1. {
        65535
    } else {
        (v * 65535. + 0.5) as u16
    }
}
