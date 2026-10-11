//! ハードウェアの ray query（RT コア）の道（wgpu の実験機能 `EXPERIMENTAL_RAY_QUERY`）。BVH の代わりに、三角形の並びから作る
//! 加速構造（BLAS・TLAS）をシェーダーでたどる。既定は入（`GpuBakeOptions::ray_query`。アプリの設定と環境変数 `YOLUPAINTER_BAKE_RAY_QUERY=0`
//! で切れる）。使えないアダプターでは作らず、作れても自己照合（同じレイを compute の道と ray query の道で飛ばして答えを比べる）に
//! 通らなければ compute の道に戻り、戻った場所と理由は `GpuBakeStats::ray_query_note` に残る。実行して確かめたのは Vulkan（RTX 3050
//! Laptop）と Metal（Apple M4）。DX12 は DXC（新しいシェーダーコンパイラー）が無いと ray query が出ないので、Windows のアダプターは
//! Vulkan を先に選ぶ。固まる・応答なし（TDR）になるドライバーは、自己照合が同じ道の中で行われるので compute に戻せない（設定で切る）。
use super::pack::{Packed, NONE};
use wgpu::util::DeviceExt;

pub(super) const SHADER: &str = include_str!("trace_rq.wgsl");
/// `bake.wgsl` の `//@EXTRA_BINDINGS@` に入る、加速構造の束縛。
pub(super) const BINDINGS: &str = "@group(0) @binding(12) var tlas_low: acceleration_structure;\n@group(0) @binding(13) var tlas_high: acceleration_structure;";
/// 自己照合のレイの数。
pub(super) const CHECK_RAYS: u32 = 4096;
/// 自己照合の結果のレイ 1 本ぶんのワード数: [当たったか, t, 三角形, u, v, 0, 0, 0]（`bake.wgsl` の `selfcheck`）。
pub(super) const CHECK_STRIDE: usize = 8;
/// 自己照合の結果の大きさ（バイト）。
pub(super) const CHECK_BYTES: u64 = CHECK_RAYS as u64 * CHECK_STRIDE as u64 * 4;

pub(super) fn source(common: &str) -> String {
    format!(
        "enable wgpu_ray_query;\n{}\n{SHADER}",
        common.replace("//@EXTRA_BINDINGS@", BINDINGS)
    )
}

/// compute の束縛（0〜11）に、加速構造の 2 つ（低ポリ・高ポリ）を足した並び。
pub(super) fn layout_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    let mut entries = super::layout_entries();
    for binding in [12, 13] {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::AccelerationStructure {
                vertex_return: false,
            },
            count: None,
        });
    }
    entries
}

pub(super) struct Accel {
    pub tlas_low: wgpu::Tlas,
    pub tlas_high: wgpu::Tlas,
    _blas: Vec<wgpu::Blas>,
    _vertices: Vec<wgpu::Buffer>,
}

/// ray query を使える形か（BVH の名前ごとの組は対象外。三角形の数が上限以内）。使えなければ理由。
/// `spare` は予算の残り（入力と出力の帯のあと）。加速構造はドライバーの内部で、頂点（36 バイト）と同じくらいの大きさを数える
/// 見積もりで三角形 1 つあたり 100 バイト。
pub(super) fn applicable(packed: &Packed, limits: &wgpu::Limits, spare: u64) -> Result<(), String> {
    if packed.has_groups {
        return Err("名前の対応（BVH が名前ごと）は ray query の対象外".into());
    }
    if packed.params.low_bvh == NONE && packed.params.high_bvh == NONE {
        return Err("レイも投影も無いので加速構造は要らない".into());
    }
    let mut triangles = 0u64;
    for b in [packed.params.low_bvh, packed.params.high_bvh] {
        if let Some((_, count)) = table_entry(packed, b) {
            triangles += u64::from(count);
            if count > limits.max_blas_primitive_count {
                return Err(format!(
                    "三角形 {count} 個が加速構造の上限 {} を超える",
                    limits.max_blas_primitive_count
                ));
            }
        }
    }
    if triangles * 100 > spare {
        return Err(format!(
            "加速構造の見積もり {} MiB が予算の残り {} MiB を超える",
            (triangles * 100) >> 20,
            spare >> 20
        ));
    }
    Ok(())
}

/// BVH `b` の (面の先頭, 面の数)。
fn table_entry(packed: &Packed, b: u32) -> Option<(u32, u32)> {
    (b != NONE).then(|| {
        let at = packed.params.bvh_table_off as usize + b as usize * 4;
        (packed.idx[at + 1], packed.idx[at + 3])
    })
}

/// 低ポリ・高ポリそれぞれの BLAS と TLAS を作って構築し、完了を待つ。片方しか無いときは同じものを両方の束縛に渡す。
/// `flip` は三角形の頂点の並び（始点・辺 1・辺 2）のうち辺 1 と辺 2 の頂点を入れ替える（ハードウェアの裏表の規約が compute の
/// 道と逆のとき。重心座標の u と v も入れ替わるので、シェーダーは `F_RQ_FLIP` で戻す）。
///
/// 構築の完了は短い間隔で待ち、そのたびに `stop`（取消・時間切れの確認）を呼ぶ。真が返ったら `BuildError::Stopped`。
pub(super) fn build(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    packed: &Packed,
    flip: bool,
    stop: &mut dyn FnMut() -> bool,
) -> Result<Accel, BuildError> {
    let make = |b: u32| -> Option<(wgpu::Buffer, u32)> {
        let (base, count) = table_entry(packed, b)?;
        if count == 0 {
            return None;
        }
        let mut vertices = Vec::with_capacity(count as usize * 9);
        for t in base..base + count {
            let w = &packed.bvh_tris[t as usize * 12..t as usize * 12 + 12];
            let f = |i: usize| f32::from_bits(w[i]);
            let a = [f(0), f(1), f(2)];
            let (e1, e2) = ([f(4), f(5), f(6)], [f(8), f(9), f(10)]);
            let (second, third) = if flip { (e2, e1) } else { (e1, e2) };
            for e in [[0.0; 3], second, third] {
                vertices.extend([a[0] + e[0], a[1] + e[1], a[2] + e[2]]);
            }
        }
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("加速構造の頂点"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::BLAS_INPUT,
        });
        Some((buffer, count))
    };
    let low = make(packed.params.low_bvh);
    let high = make(packed.params.high_bvh);
    if low.is_none() && high.is_none() {
        return Err(BuildError::Failed("三角形が無い".into()));
    }
    let identity = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let flags = wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE;
    let mode = wgpu::AccelerationStructureUpdateMode::Build;
    let mut sizes = vec![];
    let mut blas = vec![];
    let mut tlas = vec![];
    for (label, part) in [("低ポリ", &low), ("高ポリ", &high)] {
        let Some((_, count)) = part else { continue };
        let size = wgpu::BlasTriangleGeometrySizeDescriptor {
            vertex_format: wgpu::VertexFormat::Float32x3,
            vertex_count: count * 3,
            index_format: None,
            index_count: None,
            flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
        };
        let b = device.create_blas(
            &wgpu::CreateBlasDescriptor {
                label: Some(label),
                flags,
                update_mode: mode,
            },
            wgpu::BlasGeometrySizeDescriptors::Triangles {
                descriptors: vec![size.clone()],
            },
        );
        let mut t = device.create_tlas(&wgpu::CreateTlasDescriptor {
            label: Some(label),
            max_instances: 1,
            flags,
            update_mode: mode,
        });
        t[0] = Some(wgpu::TlasInstance::new(&b, identity, 0, 0xff));
        sizes.push(size);
        blas.push(b);
        tlas.push(t);
    }
    {
        let buffers: Vec<&wgpu::Buffer> = [&low, &high]
            .into_iter()
            .flatten()
            .map(|(b, _)| b)
            .collect();
        let entries: Vec<_> = blas
            .iter()
            .zip(&sizes)
            .zip(&buffers)
            .map(|((b, size), buffer)| (b, size, *buffer))
            .collect();
        let build_entries: Vec<wgpu::BlasBuildEntry<'_>> = entries
            .iter()
            .map(|(b, size, buffer)| wgpu::BlasBuildEntry {
                blas: b,
                geometry: wgpu::BlasGeometries::TriangleGeometries(vec![
                    wgpu::BlasTriangleGeometry {
                        size,
                        vertex_buffer: buffer,
                        first_vertex: 0,
                        vertex_stride: 12,
                        index_buffer: None,
                        first_index: None,
                        transform_buffer: None,
                        transform_buffer_offset: None,
                    },
                ]),
            })
            .collect();
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.build_acceleration_structures(build_entries.iter(), tlas.iter());
        let submission = queue.submit([encoder.finish()]);
        let started = std::time::Instant::now();
        loop {
            match device.poll(wgpu::PollType::Wait {
                submission_index: Some(submission.clone()),
                timeout: Some(BUILD_POLL),
            }) {
                Ok(_) => break,
                Err(wgpu::PollError::Timeout) => {
                    if stop() {
                        return Err(BuildError::Stopped);
                    }
                    if started.elapsed() >= BUILD_WAIT {
                        return Err(BuildError::Failed(format!(
                            "加速構造の構築が {} 秒たっても終わらない",
                            BUILD_WAIT.as_secs()
                        )));
                    }
                }
                Err(e) => return Err(BuildError::Failed(e.to_string())),
            }
        }
    }
    let vertices: Vec<wgpu::Buffer> = [low, high].into_iter().flatten().map(|(b, _)| b).collect();
    let mut tlas = tlas.into_iter();
    let first = tlas.next().expect("1 つはある");
    let second = tlas.next();
    // 片方しか無いときは同じものを両方の束縛へ（使われない側）。
    let (tlas_low, tlas_high) = match (low_exists(packed), second) {
        (true, Some(second)) => (first, second),
        (true, None) => (first.clone(), first),
        (false, _) => (first.clone(), first),
    };
    Ok(Accel {
        tlas_low,
        tlas_high,
        _blas: blas,
        _vertices: vertices,
    })
}
fn low_exists(packed: &Packed) -> bool {
    table_entry(packed, packed.params.low_bvh).is_some_and(|(_, n)| n > 0)
}

/// 加速構造の構築の完了を待つ 1 回の長さ（そのたびに取消・時間切れを確認する）と、待つ合計の上限。
const BUILD_POLL: std::time::Duration = std::time::Duration::from_millis(100);
const BUILD_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// 加速構造を作れなかった理由。
pub(super) enum BuildError {
    Failed(String),
    /// 取消・時間切れ・進捗の中止（結果は呼び出し側が持つ）。
    Stopped,
}

/// 自己照合の判定。
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    /// 通った（説明）。
    Pass(String),
    /// 裏面を飛ばさないレイは合うのに、裏面を飛ばすレイだけが合わない。ハードウェアの裏表の規約が compute の道と逆の疑い。
    CullReversed(String),
    /// 通らない（理由）。
    Fail(String),
}

/// レイの番号のビットが性質（値で 2・4・8）: 2 = 面の下から、4 = 裏面を飛ばす、8 = 最初の当たりで止める（`bake.wgsl` の `selfcheck`）。
const CULL: usize = 4;
const ANY_HIT: usize = 8;
/// 重心座標の許容差（compute は f32 の演算、ハードウェアは固定小数点まじりで、1e-5 程度は揺れる）。
const UV_TOLERANCE: f32 = 1e-3;

#[derive(Default)]
struct Tally {
    rays: usize,
    hits: usize,
    found_wrong: usize,
    other_wrong: usize,
    worst_t: f32,
    worst_uv: f32,
}
impl Tally {
    fn allowed(&self) -> usize {
        (self.rays / 200).max(2)
    }
    fn passes(&self) -> bool {
        self.found_wrong <= self.allowed() && self.other_wrong <= self.allowed() && self.hits > 0
    }
    fn describe(&self, label: &str) -> String {
        format!(
            "{label}: レイ {} 本、当たり {}、当たりの食い違い {}、三角形・距離・重心座標の食い違い {}、距離の最大差 {:.2e}（上限比）、重心座標の最大差 {:.2e}、許す数 {}",
            self.rays,
            self.hits,
            self.found_wrong,
            self.other_wrong,
            self.worst_t,
            self.worst_uv,
            self.allowed()
        )
    }
}

/// `cull` が真なら裏面を飛ばすレイだけ、偽ならそれ以外のレイだけを比べる。
fn tally(compute: &[u32], rq: &[u32], limit: f32, cull: bool) -> Tally {
    let n = (compute.len() / CHECK_STRIDE).min(rq.len() / CHECK_STRIDE);
    let mut t = Tally::default();
    for i in 0..n {
        if (i & CULL != 0) != cull {
            continue;
        }
        t.rays += 1;
        let (c, r) = (
            &compute[i * CHECK_STRIDE..(i + 1) * CHECK_STRIDE],
            &rq[i * CHECK_STRIDE..(i + 1) * CHECK_STRIDE],
        );
        if c[0] != r[0] {
            t.found_wrong += 1;
            continue;
        }
        if c[0] == 0 {
            continue;
        }
        t.hits += 1;
        // 最初の当たりで止めるレイは、どの当たりが先に見つかるかが実装で違うので、当たったかだけを見る
        if i & ANY_HIT != 0 {
            continue;
        }
        let f = |w: u32| f32::from_bits(w);
        let dt = (f(c[1]) - f(r[1])).abs() / limit.max(1e-30);
        t.worst_t = t.worst_t.max(dt);
        let same_triangle = c[2] == r[2];
        if dt.is_nan() || (!same_triangle && dt > 1e-4) || dt > 1e-3 {
            t.other_wrong += 1;
            continue;
        }
        // 同じ三角形に当たったなら、重心座標（高ポリの位置・法線の補間に使う）も同じでなければならない
        if same_triangle {
            let duv = (f(c[3]) - f(r[3])).abs().max((f(c[4]) - f(r[4])).abs());
            t.worst_uv = t.worst_uv.max(duv);
            if duv.is_nan() || duv > UV_TOLERANCE {
                t.other_wrong += 1;
            }
        }
    }
    t
}

/// 自己照合の結果（レイごとに `CHECK_STRIDE` ワード）を比べる。裏面を飛ばすレイは `need_cull`（この焼きが裏面を飛ばす設定）のときだけ
/// 課す: そうでないとき、ray query の道は裏面の旗を使わないので、その規約の食い違いは結果に影響しない。
pub(super) fn compare(compute: &[u32], rq: &[u32], limit: f32, need_cull: bool) -> Verdict {
    let plain = tally(compute, rq, limit, false);
    if !plain.passes() {
        return Verdict::Fail(format!(
            "自己照合に通らない（{}）",
            plain.describe("裏面を飛ばさないレイ")
        ));
    }
    if !need_cull {
        return Verdict::Pass(format!(
            "自己照合: {}",
            plain.describe("裏面を飛ばさないレイ")
        ));
    }
    let culled = tally(compute, rq, limit, true);
    let both = format!(
        "{} / {}",
        plain.describe("裏面を飛ばさないレイ"),
        culled.describe("裏面を飛ばすレイ")
    );
    if culled.passes() {
        Verdict::Pass(format!("自己照合: {both}"))
    } else {
        Verdict::CullReversed(format!("自己照合に通らない（{both}）"))
    }
}
