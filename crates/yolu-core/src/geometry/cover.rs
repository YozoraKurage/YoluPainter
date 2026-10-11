//! 3D ビューの選択ペン（クイックマスクのブラシ・消しゴム）のストローク: 画面の点の円の中の、カメラから見える面のテクセルの覆いを
//! 投影の塗り（[`SurfaceProjector`]）で集めて、文書の画素ごとの量（0〜255）にして返すだけ（文書の画素にも選択範囲にも書かない。
//! 量を選択範囲の被覆へ積むのは呼ぶ側）。
//!
//! - ダブの形は 2D の選択ペンと同じ: 丸い筆先・硬さ・ダブの最大の量（不透明度）。筆圧で大きさ（下限は半径の 5%）と量を変える。
//!   間隔は直径の 5%（画面の 0.5 点以上）で、入力の点の間を直線で結ぶ（2D の選択ペンと同じく、ブラシの曲線・手ぶれ補正・ゆらぎ・
//!   対称は使わない）。
//! - 3D の決まりは塗るストローク（`SurfaceStroke`）と同じ: 半径はブラシの半径（文書の画素）をモデルの単位に直したもの
//!   （[`world_radius`]）、間隔は区間の始まりの面の奥行きで画面へ直す。覆いは投影の塗りの切り替え（隠れた所・裏の面・面の向きの弱め・
//!   継ぎ目のにじみ）のまま。中心が塗るテクスチャセットの面に無いダブは、直前に面に当たった所の大きさで描く（まだ一度も当たって
//!   いなければ置かない）。
//! - 投影の塗りの覚えは、呼ぶたびに渡すバイト（呼ぶ側の 1 ストロークの作業の予算の残り）に収める。1 つのダブに要る区画だけで入らない
//!   ときと、区間のダブがありえない数のときは `Err`（呼ぶ側がストロークを捨てる）。

use std::sync::Arc;

use glam::Vec2;

use super::camera::CameraView;
use super::dab::SurfaceDabResult;
use super::paint::{gap, pick, world_radius, SurfaceStrokeError};
use super::project::{ProjectionSettings, SurfaceProjector};
use super::stroke::{ScreenPoint, ScreenStrokeSampler};
use super::SurfaceGeometry;

/// 選択ペンのダブの形（ストロークの始めに固める。2D の選択ペンと同じ値）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoverParams {
    /// 半径（文書の画素）。
    pub radius: f64,
    /// 0〜1。この割合までは満量で、縁へなめらかに減る。
    pub hardness: f64,
    /// 0〜1。ダブの最大の量。
    pub opacity: f64,
    /// 筆圧で半径を変える・量を変える。
    pub pressure_size: bool,
    pub pressure_opacity: bool,
}

/// 選択ペンの間隔（直径に対する割合。2D の選択ペンと同じ 5%）。
const COVER_SPACING: f32 = 0.05;

/// 画素 (x, y) の量（0〜255。0 は返さない）。
pub type CoverPixel = (i32, i32, u8);

/// 進行中の 3D の選択ペンのストローク。
pub struct SurfaceCoverStroke {
    geometry: Arc<SurfaceGeometry>,
    view: CameraView,
    material: Option<i32>,
    sampler: ScreenStrokeSampler,
    projection: ProjectionSettings,
    projector: Option<SurfaceProjector>,
    params: CoverParams,
    world_radius: f32,
    /// モデルの単位 1 が画面で何単位か（直前に塗るセットの面に当たった所の奥行きで）。
    screen_scale: Option<f32>,
    width: i32,
    height: i32,
    dabs: u64,
}

impl SurfaceCoverStroke {
    /// ストロークを始め、押した点のダブの量を返す。material は描くテクスチャセット、width・height は文書の大きさ、room は投影の塗りに
    /// 使ってよいバイト。
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        geometry: Arc<SurfaceGeometry>,
        view: CameraView,
        material: Option<i32>,
        params: CoverParams,
        projection: ProjectionSettings,
        width: u32,
        height: u32,
        at: Vec2,
        pressure: f32,
        room: u64,
    ) -> Result<(SurfaceCoverStroke, Vec<CoverPixel>), SurfaceStrokeError> {
        let world_radius = world_radius(&geometry, params.radius, width);
        let mut s = SurfaceCoverStroke {
            geometry,
            view,
            material,
            sampler: ScreenStrokeSampler::with_curve(ScreenPoint::new(at, pressure), false),
            projection: projection.sanitized(),
            projector: None,
            params,
            world_radius,
            screen_scale: None,
            width: width as i32,
            height: height as i32,
            dabs: 0,
        };
        let mut out = Vec::new();
        s.dab(at, pressure, room, &mut out)?;
        Ok((s, out))
    }

    /// 新しい入力の点。前の点から直線で結び、間隔ごとのダブの量を返す（同じ画素は大きい方）。
    pub fn add(
        &mut self,
        at: Vec2,
        pressure: f32,
        room: u64,
    ) -> Result<Vec<CoverPixel>, SurfaceStrokeError> {
        let (geometry, view, radius, scale) = (
            self.geometry.clone(),
            self.view,
            self.world_radius,
            self.screen_scale,
        );
        let mut points = Vec::new();
        self.sampler
            .add(
                at,
                pressure,
                |a| gap(&geometry, &view, radius, COVER_SPACING, scale, a),
                &mut points,
            )
            .map_err(|_| SurfaceStrokeError::TooManyDabs)?;
        let mut out = Vec::new();
        for (p, pressure) in points {
            self.dab(p, pressure, room, &mut out)?;
        }
        Ok(out)
    }

    /// 置いたダブの数（試験用）。
    pub fn dabs(&self) -> u64 {
        self.dabs
    }

    /// 投影の塗りが今持っているバイト（区画の一覧・UV の覆い・覚えた投影の画素）。
    pub fn projection_bytes(&self) -> u64 {
        self.projector
            .as_ref()
            .map_or(0, |p| p.shared_bytes() + p.fixed_bytes() + p.cached_bytes())
    }

    /// ダブ 1 つの量を out に加える。
    fn dab(
        &mut self,
        at: Vec2,
        pressure: f32,
        room: u64,
        out: &mut Vec<CoverPixel>,
    ) -> Result<(), SurfaceStrokeError> {
        let p = pressure.clamp(0.0, 1.0) as f64;
        let size = if self.params.pressure_size {
            p.max(0.05)
        } else {
            1.0
        };
        let opacity = if self.params.pressure_opacity {
            self.params.opacity * p
        } else {
            self.params.opacity
        };
        if !(at.x >= 0.0 && at.x < self.view.width && at.y >= 0.0 && at.y < self.view.height) {
            return Ok(());
        }
        let hit = pick(&self.geometry, &self.view, at)
            .filter(|h| self.material.is_none_or(|m| m == h.material));
        if let Some(h) = &hit {
            let scale = self.view.world_radius_to_screen(h.position, 1.0);
            if scale.is_finite() && scale > 0.0 {
                self.screen_scale = Some(scale);
            }
        }
        let Some(scale) = self.screen_scale else {
            return Ok(());
        };
        let screen_radius = scale * self.world_radius * size as f32;
        if self.projector.is_none() {
            let projector = SurfaceProjector::new(
                self.geometry.clone(),
                &self.view,
                self.material,
                self.width,
                self.height,
                scale * self.world_radius,
                self.projection,
            )
            .map_err(SurfaceStrokeError::Dab)?;
            self.projector = Some(projector);
        }
        let projector = self.projector.as_mut().expect("作った");
        let limit = room.saturating_sub(projector.shared_bytes() + projector.fixed_bytes());
        let dab: SurfaceDabResult =
            projector.dab(at, screen_radius, self.params.hardness as f32, limit);
        if let Some(why) = dab.refusal {
            return Err(SurfaceStrokeError::Dab(why));
        }
        self.dabs += 1;
        if opacity <= 0.0 {
            return Ok(());
        }
        out.reserve(dab.pixels.len());
        for q in &dab.pixels {
            if q.x < 0 || q.y < 0 || q.x >= self.width || q.y >= self.height {
                continue;
            }
            let a = (255.0 * (q.coverage as f64).clamp(0.0, 1.0) * opacity).round() as u8;
            if a > 0 {
                out.push((q.x, q.y, a));
            }
        }
        Ok(())
    }
}
