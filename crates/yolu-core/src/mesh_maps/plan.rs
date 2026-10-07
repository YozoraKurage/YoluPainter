//! ベイクを別の実行場所（GPU）で行うための入口。CPU の `bake` と同じ準備（`prepare`）と後始末（`finish`）を共有し、
//! 行ごとの計算だけを呼び手が行う。呼び手は `MeshBakePlan::scene` の平らな配列から画素を焼いて `MeshBakeRaw` にし、
//! `finish` に戻す。式の正本は CPU（`bake` の行ごとの計算）で、`scene` はその式が読む値を並べ直しただけ。
use super::{
    bake::{finish, prepare, Control, Prep, Prepared},
    bvh::FlatBvh,
    check,
    curvature::FlatCurvature,
    ids::IdTable,
    rays::MeshRayScene,
    MeshBakeBudget, MeshBakeInput, MeshBakeRaw, MeshBakeReport, MeshBakeResult, MeshBakeSettings,
    MeshBakeStatus, MeshMapError, MeshMapKind, Result,
};
use std::{sync::atomic::AtomicBool, time::Instant};

/// 準備まで済んだベイク。`scene` が実行場所へ渡す平らな入力、`finish` が後始末。
pub struct MeshBakePlan<'a> {
    prep: Box<Prepared<'a>>,
    control: Control<'a>,
}
/// 準備の結果。途中で取消・時間切れ・進捗コールバックの false があれば、CPU の `bake` と同じ空の結果を返す。
/// 1 回のベイクに 1 度だけ作る値なので、`Stopped` の大きさの差（空の結果）は気にしない。
#[allow(clippy::large_enum_variant)]
pub enum MeshBakePlanOutcome<'a> {
    Ready(MeshBakePlan<'a>),
    Stopped(MeshBakeResult),
}

/// 「持たない」ことを表す値（手動の ID 色・投影の対象の BVH がない受け手）。
pub const SCENE_NONE: u32 = u32::MAX;

/// 受け手（焼く三角形）ごとの配列は、`receivers` と同じ順（重なったテクセルを先に取る順。既定の決め方では元の三角形の番号の昇順。
/// `settings.overlap` で変わる）。同じテクセルを覆う三角形のうち、この並びで先のものが持ち主になる。
#[derive(Clone, Debug)]
pub struct MeshBakeScene<'a> {
    pub width: u32,
    pub height: u32,
    pub antialiasing: u32,
    pub maps: Vec<MeshMapKind>,
    /// 低ポリの対角線の長さ（距離の設定はこれの比）。
    pub diagonal: f64,
    /// 位置のマップの正規化: `(p - min) * scale`（`scale` が 0 の軸は 0.5）。
    pub min: [f64; 3],
    pub scale: [f64; 3],
    /// 高さのマップの正規化の幅: `max(reference_frontal, reference_rear) * diagonal`（高ポリが無くても設定から決まる）。
    pub height_range: f64,
    pub receivers: Vec<u32>,
    /// 受け手ごとの 3 つの角の位置（9）・頂点法線（9。無ければ `None`）・面の法線（3）。
    pub corners: Vec<f32>,
    pub normals: Option<Vec<f32>>,
    pub face: Vec<f32>,
    /// 受け手ごとの UV の画素への写し（6: 原点 x y と、重心の u v への行列 m00 m01 m10 m11。`Raster::row` と同じ）。
    pub raster_affine: Vec<f64>,
    /// 受け手ごとの覆う範囲（4: x0 x1 y0 y1。y1 < y0 なら空）。
    pub raster_bounds: Vec<i32>,
    /// 接空間法線のときの、受け手ごとの接線・従接線・法線（各 9、合わせて 27）。
    pub frames: Option<Vec<f32>>,
    /// 曲率のとき、受け手ごとの連結成分。
    pub low_component: Vec<u32>,
    /// レイのマップ（AO・ベントノーマル・厚み）があるときの、低ポリの遮蔽物の BVH。
    pub low_bvh: Option<FlatBvh>,
    /// 曲率のマップがあるときの、低ポリの曲率。
    pub curvature: Option<FlatCurvature>,
    /// 高ポリからの投影。
    pub projection: Option<MeshBakeProjection<'a>>,
    pub ids: Option<MeshBakeIds>,
    pub rays: MeshRayScene,
}
#[derive(Clone, Debug)]
pub struct MeshBakeProjection<'a> {
    /// 受け手ごとの、ケージの向き（角ごとの 9。正規化は呼び手が補間のあとに行う）。
    pub cage: Vec<f32>,
    /// 前方・後方の距離（対角線をかけたもの）。
    pub frontal: f64,
    pub rear: f64,
    pub by_name: bool,
    /// 高ポリの有効な三角形全部の BVH（名前の対応のときも、当たったあとのレイで使う）。
    pub bvh: FlatBvh,
    /// 名前の対応のときの、名前ごとの BVH と、受け手ごとのその番号（`SCENE_NONE` は対応なし＝投影しない）。
    pub groups: Vec<FlatBvh>,
    pub target: Vec<u32>,
    /// 高ポリの三角形（元の番号）ごとの 9 つの位置・頂点法線・面の法線（3）。
    pub high_corners: &'a [f32],
    pub high_normals: Option<&'a [f32]>,
    pub high_face: &'a [f32],
    pub curvature: Option<FlatCurvature>,
}
#[derive(Clone, Debug)]
pub struct MeshBakeIds {
    /// 受け手ごとの色（0xRRGGBB）と手動の色（`SCENE_NONE` は手動なし）。
    pub low: Vec<u32>,
    pub manual: Vec<u32>,
    /// 高ポリの三角形（元の番号）ごとの色。
    pub high: Vec<u32>,
    /// UV アイランドのときは、当たった高ポリの色を使わず低ポリの色にする。
    pub uv_island: bool,
}

impl<'a> MeshBakePlan<'a> {
    /// `bake` と同じ検証・見積もり・準備。拒否は `Err`。進捗は "Preparing" まで呼ぶ。
    pub fn prepare(
        input: &'a MeshBakeInput,
        settings: &'a MeshBakeSettings,
        budget: &MeshBakeBudget,
        cancel: Option<&'a AtomicBool>,
        reference: Option<&'a MeshBakeInput>,
        progress: &mut dyn FnMut(f64, &str) -> bool,
    ) -> Result<MeshBakePlanOutcome<'a>> {
        settings.validate()?;
        let control = Control {
            cancel,
            start: Instant::now(),
            max_seconds: budget.max_seconds,
        };
        Ok(
            match prepare(input, settings, budget, &control, reference, progress)? {
                Prep::Ready(prep) => MeshBakePlanOutcome::Ready(Self { prep, control }),
                Prep::Stopped(result) => MeshBakePlanOutcome::Stopped(result),
            },
        )
    }
    pub fn settings(&self) -> &MeshBakeSettings {
        self.prep.s
    }
    /// 準備でわかった数（受け手・遮蔽物・高ポリの数、見積もり）。
    pub fn report(&self) -> &MeshBakeReport {
        &self.prep.report
    }
    /// 取消・時間切れかどうか（続けてよければ `Completed`）。
    pub fn status(&self) -> MeshBakeStatus {
        self.control.status()
    }
    /// 区切りの確認（CPU の `bake` と同じ）。止めるべきなら空の結果（既存の正本を変えない）、続けてよければ `None`。
    pub fn checkpoint(
        &mut self,
        fraction: f64,
        phase: &str,
        progress: &mut dyn FnMut(f64, &str) -> bool,
    ) -> Option<MeshBakeResult> {
        self.control
            .checkpoint(&mut self.prep.report, progress, fraction, phase)
    }
    /// 実行場所へ渡す平らな入力。
    pub fn scene(&self) -> MeshBakeScene<'_> {
        let p = &*self.prep;
        let n = p.receivers.len();
        let mut corners = Vec::with_capacity(n * 9);
        let mut normals = p
            .low
            .input
            .normals
            .as_ref()
            .map(|_| Vec::with_capacity(n * 9));
        let mut face = Vec::with_capacity(n * 3);
        let mut affine = Vec::with_capacity(n * 6);
        let mut bounds = Vec::with_capacity(n * 4);
        for (r, &t) in p.receivers.iter().enumerate() {
            corners.extend_from_slice(&p.low.input.corners[t * 9..t * 9 + 9]);
            if let (Some(out), Some(all)) = (normals.as_mut(), p.low.input.normals.as_ref()) {
                out.extend_from_slice(&all[t * 9..t * 9 + 9]);
            }
            face.extend_from_slice(&p.low.face[t]);
            let rt = &p.raster.triangles[r];
            debug_assert_eq!(rt.original, t);
            affine.extend_from_slice(&rt.affine);
            bounds.extend_from_slice(&[rt.x0, rt.x1, rt.y0, rt.y1]);
        }
        let per_receiver = |values: &[f32], stride: usize| {
            let mut out = Vec::with_capacity(n * stride);
            for &t in &p.receivers {
                out.extend_from_slice(&values[t * stride..(t + 1) * stride]);
            }
            out
        };
        let frames = p.frames.as_ref().map(|f| {
            let (t, b, nn) = f.arrays();
            let (t, b, nn) = (per_receiver(t, 9), per_receiver(b, 9), per_receiver(nn, 9));
            let mut out = Vec::with_capacity(n * 27);
            for r in 0..n {
                out.extend_from_slice(&t[r * 9..r * 9 + 9]);
                out.extend_from_slice(&b[r * 9..r * 9 + 9]);
                out.extend_from_slice(&nn[r * 9..r * 9 + 9]);
            }
            out
        });
        let curvature = p.curvature.as_ref().map(|c| c.flatten());
        let low_component = curvature.as_ref().map_or(vec![], |c| {
            p.receivers.iter().map(|&t| c.components[t]).collect()
        });
        let projection = match (&p.projection, &p.high, &p.high_bvh) {
            (Some(projection), Some(high), Some(bvh)) => Some(MeshBakeProjection {
                cage: per_receiver(&projection.cage, 9),
                frontal: p.s.reference_frontal * p.input.diagonal,
                rear: p.s.reference_rear * p.input.diagonal,
                by_name: p.s.reference_match_by_name,
                bvh: bvh.flatten(),
                groups: projection.groups.iter().map(|g| g.flatten()).collect(),
                target: p
                    .receivers
                    .iter()
                    .map(|&t| projection.target[t].map_or(SCENE_NONE, |g| g as u32))
                    .collect(),
                high_corners: &high.input.corners,
                high_normals: high.input.normals.as_deref(),
                high_face: high.face.as_flattened(),
                curvature: p.high_curvature.as_ref().map(|c| c.flatten()),
            }),
            _ => None,
        };
        let ids = p.ids.as_ref().map(|ids: &IdTable| MeshBakeIds {
            low: p.receivers.iter().map(|&t| ids.low[t]).collect(),
            manual: p
                .receivers
                .iter()
                .map(|&t| ids.manual.get(t).copied().flatten().unwrap_or(SCENE_NONE))
                .collect(),
            high: ids.high.clone(),
            uv_island: p.s.id_source == super::MeshIdSource::UvIsland,
        });
        MeshBakeScene {
            width: p.s.width as u32,
            height: p.s.height as u32,
            antialiasing: p.s.antialiasing as u32,
            maps: p.s.maps.clone(),
            diagonal: p.input.diagonal,
            min: p.min,
            scale: p.scale,
            height_range: (p.s.reference_frontal * p.input.diagonal)
                .max(p.s.reference_rear * p.input.diagonal),
            receivers: p.receivers.iter().map(|&t| t as u32).collect(),
            corners,
            normals,
            face,
            raster_affine: affine,
            raster_bounds: bounds,
            frames,
            low_component,
            low_bvh: super::bake::wants_rays(p.s).then(|| p.low_bvh.flatten()),
            curvature,
            projection,
            ids,
            rays: p.rays.scene(),
        }
    }
    /// 焼いた画素から余白・記録・由来を作って結果にする（CPU の `bake` と同じ後始末）。画素の並び・長さを確かめ、
    /// 合わなければ拒否する。
    pub fn finish(
        self,
        raw: MeshBakeRaw,
        progress: &mut dyn FnMut(f64, &str) -> bool,
    ) -> Result<MeshBakeResult> {
        let s = self.prep.s;
        let texels = s.width as usize * s.height as usize;
        check(
            raw.coverage.len() == texels
                && raw.coverage.iter().all(|c| *c <= 2)
                && raw.outputs.len() == s.maps.len()
                && raw
                    .outputs
                    .iter()
                    .zip(&s.maps)
                    .all(|(o, k)| o.len() == texels * k.channels()),
            "焼いた画素の長さまたは由来が設定と合いません",
        )?;
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.prep.threads)
            .build()
            .map_err(|e| MeshMapError(e.to_string()))?;
        finish(*self.prep, raw, &self.control, &pool, progress)
    }
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
