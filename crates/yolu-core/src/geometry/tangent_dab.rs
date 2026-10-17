//! Tangent | Wrap のダブ（3D のストロークのアライメントの 1 つ。[`SurfaceAlignment`]）: ダブの形を、画面ではなく面に沿った局所の展開の
//! 座標で読む。
//!
//! - ダブの中心（カメラのレイが当たった塗るセットの面の点と、その三角形の法線）から、[`SurfaceGeometry::build_sampling_chart`] で、届く
//!   半径（筆先の縁の帯までの画面の半径を、中心の奥行きでモデルの単位に直したもの）の内側を平らに展開する。展開の三角形ごとに、UV の中の
//!   テクセルの中心を重心座標で展開の点へ写し、その点の中心からのずれ（面に沿った距離）を画面の単位へ戻して、投影の塗りと同じ覆いの式
//!   （[`ScreenCover`]）で読む。
//! - 筆先の上の向き: カメラの上（回転の Y）を中心の接平面へ写したもの（接平面とほぼ直交するときはカメラの前の向きを写したもの）。右は、
//!   上と法線に直交してカメラの右と同じ側の向き。筆先の角度（ストロークの向きに沿わせる角度も）は、この右と上の枠で、画面の上と同じ角度だけ
//!   回す（面がカメラを向いていれば画面の上の向きと同じ。斜めの面では、画面の向きを接平面へ写した角度とはずれる）。
//! - 遮蔽はカメラから見ない（面に沿って届く所は、カメラから見えない側も塗る）。中心の法線との内積が負の面（[`FACING_TOLERANCE`] より小さい）は
//!   塗らない。直角の面（立方体の隣の面）は塗る。裏の面を塗る切り替えが入っていれば、向きによらず塗る。隠れた所・面の向きの弱めの切り替えは
//!   使わない。
//! - 展開は辺でつながった三角形を辿るので、UV アイランドの継ぎ目をまたいで続く。別のスロット（`material_slot`）・別のレンダラー・非多様体の辺へは
//!   進まない（隣り合わせがその間に無い）。展開は半径の内側だけで、同じ三角形は初めに届いた道の展開だけを使う（細い筒を 1 周より大きく
//!   巻くダブは、巻いた先を塗らない）。
//! - 継ぎ目のにじみは投影の塗りと同じ: 塗る三角形の UV アイランドの縁の辺から外へ N テクセルの、塗るセットのどの三角形の UV にも入らない
//!   テクセルを、辺の上のいちばん近い点の覆いで塗る。
//! - 予算: 展開の図（1 三角形 128 バイト）と候補のテクセル（三角形ごとの箱の和）のバイトを、呼び手の残り（1 回の操作のメモリから、ストロークの
//!   間の一覧などを引いたもの）に収める。三角形の数・テクセルの数はバイトだけで抑える（3D の指先・クローン・ぼかしの展開の図と同じ。投影の
//!   塗りも、ダブの大きさはメモリだけで抑える）。超えたら画素を返さずに `MemoryBudget` で断る（呼び手がストロークを取り消す）。
//! - 決定的: 三角形ごとの候補は三角形と設定だけで決まり、同じテクセルは覆いの大きいほう（同じなら三角形の番号の小さいほう、続けて辺）の 1 つに
//!   して下の行から並べる（並列の度合いによらない）。計算は四則と平方根だけ（展開の図は単精度、テクセルの点と覆いは倍精度）。
//! - 対称（鏡映・放射）: 写しの点（[`find_copy`]）で、写した上と右の向きを写しの点の接平面へ写した枠で同じ塗りをする（鏡映では左右が入れ替わる）。

use std::sync::Arc;

use glam::{DVec2, DVec3, Vec3};
use rayon::prelude::*;

use super::camera::CameraView;
use super::dab::{DabRefusal, SurfaceDabResult, SurfacePixel};
use super::materials::{admits, MaterialSet};
use super::paint::SurfaceSymmetrySetup;
use super::project::{ProjectionSettings, ScreenCover, Shared, UvFace, SEAM0};
use super::sampling::{SamplingChart, SamplingError};
use super::symmetry::{find_copy, union_dabs, CopyHit, MirrorOutcome};
use super::{SurfaceGeometry, SurfaceHit, SurfaceTriangle};
use crate::brush::TexelMetric;

/// 3D の塗りのダブの形を、何の上で読むか（画面から写すか、面に沿って読むか）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SurfaceAlignment {
    /// 画面の上のダブの形をカメラから面へ投影する（[`super::SurfaceProjector`]）。
    #[default]
    Camera,
    /// 面に沿った局所の展開の上でダブの形を読む（このモジュール）。
    TangentWrap,
}

impl SurfaceAlignment {
    pub const ALL: [SurfaceAlignment; 2] =
        [SurfaceAlignment::Camera, SurfaceAlignment::TangentWrap];
}

/// 裏の面とみなす、中心の法線との内積の境（これより小さい面は、裏の面を塗る切り替えが無ければ塗らない）。直角の面の内積は丸めで 0 の
/// まわりに散るので、少しだけ負の側へずらす。
pub const FACING_TOLERANCE: f64 = -1e-6;

/// 斜めの面で、ダブの間隔を縮める係数の下限（面と視線がおよそ 81 度より寝ていても、間隔はこれより縮めない。ダブの数が増えすぎない
/// ように）。
pub const MIN_FORESHORTENING: f32 = 0.15;

/// 画面の点 at の下の面で、画面の上の長さに対する面に沿った長さの縮み（法線と視線の cos の絶対値。下限 [`MIN_FORESHORTENING`]）。
/// 面に当たらなければ 1。ダブの間隔（画面の長さ）に掛けて、面に沿った間隔にする（線の向きによらず、いちばん縮む向きの値）。
pub(crate) fn foreshortening(geometry: &SurfaceGeometry, view: &CameraView, at: glam::Vec2) -> f32 {
    let Some(hit) = super::paint::pick(geometry, view, at) else {
        return 1.0;
    };
    let to = view.to_viewer(hit.position).as_dvec3();
    let (n, len2) = (hit.normal.as_dvec3(), to.length_squared());
    if !(len2.is_finite() && len2 > 0.0) {
        return 1.0;
    }
    let cos = (n.dot(to) / len2.sqrt()).abs();
    if cos.is_finite() {
        (cos as f32).clamp(MIN_FORESHORTENING, 1.0)
    } else {
        1.0
    }
}

/// 候補のテクセルを、三角形ごとに並列に集める下限（候補の箱の和）。直列でも結果は同じ。
const PARALLEL_CANDIDATES: u64 = 1 << 15;

/// 候補 1 つ（同じテクセルを 1 つにする前）。
#[derive(Clone, Copy)]
struct Candidate {
    key: i64,
    coverage: f32,
    face: u32,
    rank: u8,
    position: Vec3,
}

/// 候補 1 つの名目のバイト（予算に数える）。
const CANDIDATE_BYTES: u64 = std::mem::size_of::<Candidate>() as u64;
/// 画素 1 つの名目のバイト（写しを足すときに、それまでの画素を予算に数える）。
const PIXEL_BYTES: u64 = std::mem::size_of::<SurfacePixel>() as u64;

/// ダブの中心の接平面の枠（単位の右と上。モデルの空間）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TangentFrame {
    right: DVec3,
    up: DVec3,
    /// 中心の法線（単位）。
    normal: DVec3,
    /// 上の手がかりが法線と重なるときに、上を向ける側（カメラの前の向き。写しでは写したもの）。
    toward: DVec3,
}

impl TangentFrame {
    /// カメラの上を、法線 normal の接平面へ写した枠（法線が潰れていれば None）。
    pub(crate) fn of_camera(view: &CameraView, normal: Vec3) -> Option<TangentFrame> {
        let right = (view.rotation * Vec3::X).as_dvec3();
        let up = (view.rotation * Vec3::Y).as_dvec3();
        TangentFrame::new(normal.as_dvec3(), up, right, view.forward.as_dvec3())
    }

    /// 上の手がかり up を接平面へ写して上にし、右は上と法線に直交して手がかり right と同じ側。up が接平面とほぼ直交するときは、right を
    /// 接平面へ写して右にし、上は toward の側。
    fn new(normal: DVec3, up: DVec3, right: DVec3, toward: DVec3) -> Option<TangentFrame> {
        let n = unit(normal)?;
        let onto = |v: DVec3| v - n * v.dot(n);
        let hint = onto(up);
        let (right, up) =
            match unit(hint).filter(|_| hint.length_squared() > 1e-6 * up.length_squared()) {
                Some(up) => {
                    let side = up.cross(n);
                    (if side.dot(right) < 0.0 { -side } else { side }, up)
                }
                None => {
                    let r = unit(onto(right)).unwrap_or_else(|| n.any_orthonormal_vector());
                    let side = n.cross(r);
                    (r, if side.dot(toward) < 0.0 { -side } else { side })
                }
            };
        Some(TangentFrame {
            right,
            up,
            normal: n,
            toward,
        })
    }

    /// 写しの側の枠（写した上・右・控えの向きを、写しの点の法線の接平面へ）。
    fn mapped(&self, map: impl Fn(Vec3) -> Vec3, normal: Vec3) -> Option<TangentFrame> {
        let m = |d: DVec3| map(d.as_vec3()).as_dvec3();
        TangentFrame::new(normal.as_dvec3(), m(self.up), m(self.right), m(self.toward))
    }

    /// 展開の座標の y（法線 × 展開の横軸 = 右）を、上の向きに合わせる符号。
    fn up_sign(&self) -> f64 {
        if self.normal.cross(self.right).dot(self.up) >= 0.0 {
            1.0
        } else {
            -1.0
        }
    }
}

fn unit(v: DVec3) -> Option<DVec3> {
    let len2 = v.length_squared();
    (len2.is_finite() && len2 > 0.0).then(|| v / len2.sqrt())
}

/// 展開の点（中心が原点、右・展開の y）を、覆いの式の画面のずれ（画面の単位、y は下向き）へ。
#[derive(Clone, Copy)]
struct ToCover {
    scale: f64,
    /// 展開の y に掛けて、下向きの画面の y にする係数（−scale × 上の符号）。
    down: f64,
}

impl ToCover {
    #[inline]
    fn point(&self, q: DVec2) -> (f64, f64) {
        (q.x * self.scale, q.y * self.down)
    }
}

/// ストロークの間の Tangent | Wrap の塗り（カメラによらない一覧は、投影の塗りと同じものを共有する）。
pub(crate) struct TangentPainter {
    shared: Arc<Shared>,
    settings: ProjectionSettings,
    material: Option<MaterialSet>,
    width: i32,
    height: i32,
}

impl TangentPainter {
    pub(crate) fn new(
        geometry: &SurfaceGeometry,
        material: Option<&MaterialSet>,
        width: i32,
        height: i32,
        settings: ProjectionSettings,
    ) -> Result<TangentPainter, DabRefusal> {
        if width <= 0 || height <= 0 || width > 32768 || height > 32768 {
            return Err(DabRefusal::InvalidArguments);
        }
        Ok(TangentPainter {
            shared: Shared::get(geometry, material, width, height),
            settings: settings.sanitized(),
            material: material.cloned(),
            width,
            height,
        })
    }

    /// ストロークの間ずっと持つ一覧（UV の覆い・アイランドの縁）のバイト。
    pub(crate) fn fixed_bytes(&self) -> u64 {
        self.shared.bytes()
    }

    /// テクセルが塗るセットのどれかの三角形の UV に入るか（文書の外は入らない）。
    pub(crate) fn covered(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.width && y < self.height && self.shared.covered(x, y)
    }

    /// 面の点 hit のテクセル 1 つ分が、覆いの式の画面の単位でどれだけか（y は下向き。UV が潰れていれば None）。scale はモデルの単位 1 の
    /// 画面の大きさ。
    pub(crate) fn metric_at(
        &self,
        geometry: &SurfaceGeometry,
        view: &CameraView,
        hit: &SurfaceHit,
        scale: f32,
    ) -> Option<TexelMetric> {
        let t = geometry.triangles().get(hit.triangle as usize)?;
        let frame = TangentFrame::of_camera(view, hit.normal)?;
        let uv = UvFace::new(t, self.width, self.height)?;
        let flat = |p: Vec3| {
            let d = p.as_dvec3() - hit.position.as_dvec3();
            DVec2::new(d.dot(frame.right), d.dot(frame.up))
        };
        let to = ToCover {
            scale: scale as f64,
            down: -(scale as f64),
        };
        let m = metric(&uv, [flat(t.a), flat(t.b), flat(t.c)], to);
        let m = TexelMetric {
            xx: m[0] as f64,
            xy: m[1] as f64,
            yy: m[2] as f64,
        };
        m.usable().then_some(m)
    }

    /// 中心 hit のダブと、対称があればその写し（全部を画素ごとに大きいほうの覆いで 1 つにしたもの）。scale はモデルの単位 1 の画面の大きさ、
    /// cover は画面の単位の覆いの式、radius は写しの点を探す距離（モデルの単位のダブの半径）、room はこのダブに使ってよいバイト。
    /// 塗った写しの数と、塗られなかった写しの最後の理由も返す。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dab_around<C: ScreenCover>(
        &self,
        geometry: &SurfaceGeometry,
        view: &CameraView,
        hit: &SurfaceHit,
        scale: f32,
        cover: &C,
        symmetry: Option<&SurfaceSymmetrySetup>,
        radius: f32,
        room: u64,
    ) -> (SurfaceDabResult, usize, Option<MirrorOutcome>) {
        // 法線の潰れた三角形には塗らない（ストロークは続ける）
        let Some(frame) = TangentFrame::of_camera(view, hit.normal) else {
            return (SurfaceDabResult::default(), 0, None);
        };
        let mut result = self.dab(geometry, hit, &frame, scale, cover, room);
        let Some(sym) = symmetry.filter(|_| result.refusal.is_none()) else {
            return (result, 0, None);
        };
        let mut copies = 0;
        let mut outcome = MirrorOutcome::OnPlane;
        let mut positions = vec![hit.position];
        let count = sym.radial.map_or(1, |r| r.count);
        for copy in 0..count {
            for reflected in [false, true] {
                if (copy == 0 && !reflected) || (reflected && sym.mirror.is_none()) {
                    continue;
                }
                let found = match find_copy(
                    geometry,
                    hit,
                    self.material.as_ref(),
                    sym.mirror.as_ref(),
                    sym.radial.as_ref(),
                    copy,
                    reflected,
                    radius,
                    &mut positions,
                ) {
                    CopyHit::Duplicate => continue,
                    CopyHit::BudgetExceeded => {
                        return (result.reject(DabRefusal::BvhBudget), copies, None);
                    }
                    CopyHit::NoSurface => {
                        outcome = MirrorOutcome::NoSurface;
                        continue;
                    }
                    CopyHit::OtherMaterial => {
                        outcome = MirrorOutcome::OtherSlot;
                        continue;
                    }
                    CopyHit::Found(h) => h,
                };
                copies += 1;
                let map = |d: Vec3| {
                    let d = match (reflected, sym.mirror.as_ref()) {
                        (true, Some(m)) => m.reflect_direction(d),
                        _ => d,
                    };
                    match sym.radial.as_ref() {
                        Some(r) => r.rotate_direction(d, copy),
                        None => d,
                    }
                };
                let Some(copy_frame) = frame.mapped(map, found.normal) else {
                    outcome = MirrorOutcome::NoSurface;
                    continue;
                };
                let used = result.pixels.len() as u64 * PIXEL_BYTES;
                let dab = self.dab(
                    geometry,
                    &found,
                    &copy_frame,
                    scale,
                    cover,
                    room.saturating_sub(used),
                );
                if let Some(why) = dab.refusal {
                    return (result.reject(why), copies, None);
                }
                if dab.pixels.is_empty() {
                    outcome = MirrorOutcome::Hidden;
                } else if outcome == MirrorOutcome::OnPlane {
                    outcome = MirrorOutcome::Painted;
                }
                result = union_dabs(result, dab, self.width);
            }
        }
        let note = matches!(
            outcome,
            MirrorOutcome::NoSurface | MirrorOutcome::OtherSlot | MirrorOutcome::Hidden
        )
        .then_some(outcome);
        (result, copies, note)
    }

    /// 中心 center・枠 frame のダブ 1 つ（行優先・下の行から）。
    fn dab<C: ScreenCover>(
        &self,
        geometry: &SurfaceGeometry,
        center: &SurfaceHit,
        frame: &TangentFrame,
        scale: f32,
        cover: &C,
        room: u64,
    ) -> SurfaceDabResult {
        let mut result = SurfaceDabResult::default();
        let reach = cover.reach();
        let scale64 = scale as f64;
        if !(scale64.is_finite() && scale64 > 0.0 && reach.is_finite() && reach > 0.0) {
            return result.reject(DabRefusal::InvalidArguments);
        }
        // 届く半径（モデルの単位）。展開の図は単精度
        let radius = reach / scale64;
        let max_bytes = room.min(i64::MAX as u64) as i64;
        let chart = match geometry.build_sampling_chart(
            center,
            radius as f32,
            frame.right.as_vec3(),
            i32::MAX,
            max_bytes,
        ) {
            Ok(c) => c,
            Err(e) => {
                return result.reject(match e {
                    SamplingError::SnapshotChanged => DabRefusal::SnapshotChanged,
                    SamplingError::BindingMismatch => DabRefusal::BindingMismatch,
                    // 三角形の数は抑えないので、図のバイト
                    SamplingError::ChartBudget => DabRefusal::MemoryBudget,
                    _ => DabRefusal::InvalidArguments,
                });
            }
        };
        result.visited_triangles = chart.triangle_count().min(i32::MAX as usize) as i32;
        let to = ToCover {
            scale: scale64,
            down: -scale64 * frame.up_sign(),
        };
        let triangles = geometry.triangles();
        let (faces, total) = self.gather_faces(triangles, &chart, frame, radius);
        result.candidate_pixels = total.min(i32::MAX as u64) as i32;
        let need = (chart.nominal_bytes().max(0) as u64).saturating_add(total * CANDIDATE_BYTES);
        if need > room {
            return result.reject(DabRefusal::MemoryBudget);
        }
        let candidates = self.collect_candidates(&faces, total, triangles, to, cover);
        result.pixels = into_pixels(candidates, self.width);
        result
    }

    /// 展開の図のうち塗る三角形と、その候補の箱（展開の図に入れた順）。候補の箱の和も返す。
    fn gather_faces(
        &self,
        triangles: &[SurfaceTriangle],
        chart: &SamplingChart<'_>,
        frame: &TangentFrame,
        radius: f64,
    ) -> (Vec<Face>, u64) {
        let bleed = self.settings.seam_bleed;
        let mut faces: Vec<Face> = Vec::new();
        let mut total: u64 = 0;
        for (face, flat) in chart.flat_triangles() {
            let t = &triangles[face as usize];
            if !admits(self.material.as_ref(), t.material) {
                continue;
            }
            if !self.settings.paint_backfaces
                && t.normal().as_dvec3().dot(frame.normal) < FACING_TOLERANCE
            {
                continue;
            }
            let Some(uv) = UvFace::new(t, self.width, self.height) else {
                continue;
            };
            let flat = flat.map(|p| p.as_dvec2());
            let mut f = Face::new(face, uv, flat, radius, self.width, self.height);
            if bleed > 0 {
                let seams = self.shared.seams(face as usize);
                for edge in 0..3 {
                    if seams & (SEAM0 << edge) != 0 {
                        f.bleed_box(edge, bleed as f64, radius, self.width, self.height);
                    }
                }
            }
            total += f.count();
            faces.push(f);
        }
        (faces, total)
    }

    /// 全部の三角形の候補（同じテクセルは重なったまま）。total が大きければ三角形ごとに並列で集める（結果は同じ）。
    fn collect_candidates<C: ScreenCover>(
        &self,
        faces: &[Face],
        total: u64,
        triangles: &[SurfaceTriangle],
        to: ToCover,
        cover: &C,
    ) -> Vec<Candidate> {
        let collect = |f: &Face| self.face_candidates(f, triangles, to, cover);
        if faces.len() > 1 && total > PARALLEL_CANDIDATES && rayon::current_num_threads() > 1 {
            faces
                .par_iter()
                .with_min_len(4)
                .map(collect)
                .collect::<Vec<_>>()
                .concat()
        } else {
            faces.iter().flat_map(collect).collect()
        }
    }

    /// 三角形 f の候補のテクセル（三角形の中と、にじみの辺の外）。
    fn face_candidates<C: ScreenCover>(
        &self,
        f: &Face,
        triangles: &[SurfaceTriangle],
        to: ToCover,
        cover: &C,
    ) -> Vec<Candidate> {
        let width = self.width as i64;
        let bleed = self.settings.seam_bleed;
        let t = &triangles[f.face as usize];
        let model = [t.a.as_dvec3(), t.b.as_dvec3(), t.c.as_dvec3()];
        let m = metric(&f.uv, f.flat, to);
        let mut out = Vec::new();
        let mut push = |w: DVec3, x: i64, y: i64, rank: u8| {
            let q = f.flat[0] * w.x + f.flat[1] * w.y + f.flat[2] * w.z;
            let (dx, dy) = to.point(q);
            let c = cover.cover(dx, dy, x as u16, y as u16, m);
            if c > 0.0 {
                let p = model[0] * w.x + model[1] * w.y + model[2] * w.z;
                out.push(Candidate {
                    key: y * width + x,
                    coverage: c,
                    face: f.face,
                    rank,
                    position: p.as_vec3(),
                });
            }
        };
        if let Some((x0, x1, y0, y1)) = f.inner {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let w = f.uv.bary(x as i32, y as i32);
                    if UvFace::inside(w) {
                        push(w, x, y, 0);
                    }
                }
            }
        }
        let n = bleed as f64;
        for &(edge, (x0, x1, y0, y1)) in &f.bleeds {
            let (i, j) = (edge, (edge + 1) % 3);
            let (ua, ub) = (f.uv.corner(i), f.uv.corner(j));
            let e = ub - ua;
            let len2 = e.length_squared();
            for y in y0..=y1 {
                for x in x0..=x1 {
                    if self.shared.covered(x as i32, y as i32) {
                        continue;
                    }
                    let texel = DVec2::new(x as f64 + 0.5, y as f64 + 0.5);
                    let u = ((texel - ua).dot(e) / len2).clamp(0.0, 1.0);
                    if (texel - (ua + e * u)).length_squared() > n * n {
                        continue;
                    }
                    let mut w = DVec3::ZERO;
                    w[i] = 1.0 - u;
                    w[j] = u;
                    push(w, x, y, edge as u8 + 1);
                }
            }
        }
        out
    }
}

/// 候補を、同じテクセルは覆いの大きいほう（同じなら三角形の番号の小さいほう、続けて辺の番号）の 1 つにして、下の行から並べた画素にする
/// （投影の塗りと同じ決まり）。
fn into_pixels(mut candidates: Vec<Candidate>, width: i32) -> Vec<SurfacePixel> {
    if candidates.len() > PARALLEL_CANDIDATES as usize {
        candidates.par_sort_by_key(|c| c.key);
    } else {
        candidates.sort_by_key(|c| c.key);
    }
    candidates.dedup_by(|next, kept| {
        if next.key != kept.key {
            return false;
        }
        if next
            .coverage
            .total_cmp(&kept.coverage)
            .reverse()
            .then(next.face.cmp(&kept.face))
            .then(next.rank.cmp(&kept.rank))
            .is_lt()
        {
            *kept = *next;
        }
        true
    });
    let width = width as i64;
    candidates
        .into_iter()
        .map(|c| SurfacePixel {
            x: (c.key % width) as i32,
            y: (c.key / width) as i32,
            coverage: c.coverage,
            triangle: Some(c.face),
            position: c.position,
        })
        .collect()
}

/// テクセルの x・y の 1 つ分の、覆いの式の画面の動きの J·Jᵀ（[`TexelMetric`] の xx・xy・yy。y は下向き）。三角形の上で一定。
fn metric(uv: &UvFace, flat: [DVec2; 3], to: ToCover) -> [f32; 3] {
    // UV の画素 → 重心座標の係数（UvFace::new が潰れていないことを確かめてある）
    let (p0, p1, p2) = (uv.corner(0), uv.corner(1), uv.corner(2));
    let (e1, e2) = (p1 - p0, p2 - p0);
    let det = e1.x * e2.y - e1.y * e2.x;
    let (c11, c12, c21, c22) = (e2.y / det, -e2.x / det, -e1.y / det, e1.x / det);
    let (f1, f2) = (flat[1] - flat[0], flat[2] - flat[0]);
    let (sx, sy) = (f1 * c11 + f2 * c21, f1 * c12 + f2 * c22);
    let (jx, jy) = (to.point(sx), to.point(sy));
    let m = TexelMetric::from_columns(jx, jy);
    [m.xx as f32, m.xy as f32, m.yy as f32]
}

/// テクセルの箱（x0, x1, y0, y1。両端を含む）。
type TexelBox = (i64, i64, i64, i64);

/// 塗る三角形 1 つ: 展開の頂点・UV と、候補のテクセルの箱（三角形の中と、にじみの辺）。
struct Face {
    face: u32,
    uv: UvFace,
    flat: [DVec2; 3],
    inner: Option<TexelBox>,
    bleeds: Vec<(usize, TexelBox)>,
}

impl Face {
    /// 展開の三角形 flat のうち、中心の正方形 [−radius, radius]² に入る部分の UV の箱（入らなければ None）。
    fn new(face: u32, uv: UvFace, flat: [DVec2; 3], radius: f64, width: i32, height: i32) -> Face {
        let clipped = clip_to_square(&flat, radius);
        let mut lo = DVec2::splat(f64::INFINITY);
        let mut hi = DVec2::splat(f64::NEG_INFINITY);
        for q in &clipped {
            let Some(w) = weights(*q, flat) else {
                continue;
            };
            let u = uv.point(w);
            if u.is_finite() {
                lo = lo.min(u);
                hi = hi.max(u);
            }
        }
        let inner = (lo.x <= hi.x && lo.y <= hi.y)
            .then(|| {
                // 投影の塗りの区画と同じ余白（丸めで端のテクセルを落とさない）
                texel_box(lo - DVec2::splat(1.0), hi, width, height)
            })
            .flatten();
        Face {
            face,
            uv,
            flat,
            inner,
            bleeds: Vec::new(),
        }
    }

    /// 辺 edge のにじみの箱（辺のうち中心の正方形に入る部分から n テクセルの外まで）。
    fn bleed_box(&mut self, edge: usize, n: f64, radius: f64, width: i32, height: i32) {
        let (i, j) = (edge, (edge + 1) % 3);
        let Some((t0, t1)) = segment_in_square(self.flat[i], self.flat[j], radius) else {
            return;
        };
        let (ua, ub) = (self.uv.corner(i), self.uv.corner(j));
        let e = ub - ua;
        let len2 = e.length_squared();
        if len2.is_nan() || len2 <= 0.0 {
            return;
        }
        let (pa, pb) = (ua + e * t0, ua + e * t1);
        let lo = pa.min(pb) - DVec2::splat(n + 1.0);
        let hi = pa.max(pb) + DVec2::splat(n + 1.0);
        if let Some(b) = texel_box(lo, hi, width, height) {
            self.bleeds.push((edge, b));
        }
    }

    /// 候補の箱のテクセルの数。
    fn count(&self) -> u64 {
        let area = |b: &TexelBox| ((b.1 - b.0 + 1) * (b.3 - b.2 + 1)) as u64;
        self.inner.as_ref().map_or(0, area) + self.bleeds.iter().map(|(_, b)| area(b)).sum::<u64>()
    }
}

/// UV の画素の空間の箱 [lo, hi] にかかるテクセル（中心が lo − 0.5 から hi + 0.5 の間。文書の中に収める）。
fn texel_box(lo: DVec2, hi: DVec2, width: i32, height: i32) -> Option<TexelBox> {
    if !(lo.is_finite() && hi.is_finite()) {
        return None;
    }
    let x0 = (lo.x - 0.5).floor().max(0.0);
    let x1 = (hi.x + 0.5).ceil().min(width as f64 - 1.0);
    let y0 = (lo.y - 0.5).floor().max(0.0);
    let y1 = (hi.y + 0.5).ceil().min(height as f64 - 1.0);
    (x0 <= x1 && y0 <= y1).then_some((x0 as i64, x1 as i64, y0 as i64, y1 as i64))
}

/// 点 p の、三角形 t に対する重み（退化していれば None）。
fn weights(p: DVec2, t: [DVec2; 3]) -> Option<DVec3> {
    let (u, v, q) = (t[1] - t[0], t[2] - t[0], p - t[0]);
    let det = u.x * v.y - u.y * v.x;
    if !det.is_finite() || det.abs() < 1e-30 {
        return None;
    }
    let y = (q.x * v.y - q.y * v.x) / det;
    let z = (u.x * q.y - u.y * q.x) / det;
    Some(DVec3::new(1.0 - y - z, y, z))
}

/// 多角形を正方形 [−r, r]² で切る（Sutherland–Hodgman）。
fn clip_to_square(points: &[DVec2; 3], r: f64) -> Vec<DVec2> {
    let mut poly: Vec<DVec2> = points.to_vec();
    // 半平面 axis · p ≤ r（axis は ±x・±y）
    for (axis, sign) in [(0usize, 1.0f64), (0, -1.0), (1, 1.0), (1, -1.0)] {
        if poly.is_empty() {
            break;
        }
        let side = |p: DVec2| sign * p[axis] - r;
        let mut out = Vec::with_capacity(poly.len() + 1);
        for k in 0..poly.len() {
            let (a, b) = (poly[k], poly[(k + 1) % poly.len()]);
            let (da, db) = (side(a), side(b));
            if da <= 0.0 {
                out.push(a);
            }
            if (da <= 0.0) != (db <= 0.0) {
                let t = da / (da - db);
                out.push(a + (b - a) * t);
            }
        }
        poly = out;
    }
    poly
}

/// 線分 a → b のうち、正方形 [−r, r]² に入る部分の割合の区間（入らなければ None。Liang–Barsky）。
fn segment_in_square(a: DVec2, b: DVec2, r: f64) -> Option<(f64, f64)> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [
        (-d.x, a.x + r),
        (d.x, r - a.x),
        (-d.y, a.y + r),
        (d.y, r - a.y),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let t = q / p;
        if p < 0.0 {
            t0 = t0.max(t);
        } else {
            t1 = t1.min(t);
        }
    }
    (t0 <= t1 && t0.is_finite() && t1.is_finite()).then_some((t0, t1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_follows_the_camera_up_and_keeps_right_on_the_camera_right() {
        // 正面（−Z）を向く面を、−Z の側から見る（Unity と同じ左手系: 右 X・上 Y・前 Z）
        let f = TangentFrame::new(DVec3::NEG_Z, DVec3::Y, DVec3::X, DVec3::Z).unwrap();
        assert!((f.up - DVec3::Y).length() < 1e-12);
        assert!((f.right - DVec3::X).length() < 1e-12);
        // 斜めの面でも、上は接平面の中で、右はカメラの右の側
        let n = DVec3::new(0.0, 0.6, -0.8);
        let f = TangentFrame::new(n, DVec3::Y, DVec3::X, DVec3::Z).unwrap();
        assert!(f.up.dot(n).abs() < 1e-12 && f.right.dot(n).abs() < 1e-12);
        assert!(f.up.y > 0.0 && f.right.x > 0.0);
        // 上が法線と重なる（真上を向く面を水平に見る）: 右はカメラの右、上は奥（前の向き）
        let f = TangentFrame::new(DVec3::Y, DVec3::Y, DVec3::X, DVec3::Z).unwrap();
        assert!((f.right - DVec3::X).length() < 1e-12);
        assert!((f.up - DVec3::Z).length() < 1e-12);
    }

    #[test]
    fn a_mirrored_frame_swaps_left_and_right() {
        let f = TangentFrame::new(DVec3::NEG_Z, DVec3::Y, DVec3::X, DVec3::Z).unwrap();
        let mirror = |d: Vec3| Vec3::new(-d.x, d.y, d.z);
        let m = f.mapped(mirror, Vec3::NEG_Z).unwrap();
        assert!((m.right - DVec3::NEG_X).length() < 1e-6);
        assert_eq!(m.up_sign(), -f.up_sign());
    }

    #[test]
    fn clipping_keeps_the_part_inside_the_square() {
        let t = [
            DVec2::new(-2.0, -0.5),
            DVec2::new(2.0, -0.5),
            DVec2::new(0.0, 2.0),
        ];
        let p = clip_to_square(&t, 1.0);
        assert!(p.len() >= 4);
        assert!(p
            .iter()
            .all(|q| q.x.abs() <= 1.0 + 1e-12 && q.y.abs() <= 1.0 + 1e-12));
        assert!(clip_to_square(&t.map(|q| q + DVec2::splat(10.0)), 1.0).is_empty());
        assert_eq!(
            segment_in_square(DVec2::new(-2.0, 0.0), DVec2::new(2.0, 0.0), 1.0),
            Some((0.25, 0.75))
        );
        assert_eq!(
            segment_in_square(DVec2::new(-2.0, 3.0), DVec2::new(2.0, 3.0), 1.0),
            None
        );
    }
}
