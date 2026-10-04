//! 3D のストロークがステンシルを読む道（Unity 版の TexturePaintWindow.Stencil の面のダブの部分）: ステンシルは画面に貼り付いているので、
//! 面のテクセルの点をカメラで画面へ写し、ステンシルの置き場（画面の点 → 画像の画素の写し）で画像の点にして、そこを読ませる。
//! ミップマップの段を選ぶ足跡（テクセル 1 つに当たる画像の画素の数）は、当たった三角形の面積と UV の面積からテクセルの大きさを出し、
//! 画面の点にして、画像の画素 / 点を掛ける。ダブの中では一定とみなす。

use glam::Vec3;

use super::camera::CameraView;
use super::{SurfaceGeometry, SurfaceHit};
use crate::brush::{StencilImage, StencilMapping, StencilPoint};
use crate::error::CoreError;

/// 3D のストロークが読むステンシルの置き場。画面の点は 3D の表示域の左上が原点（カメラ・ストロークの点と同じ）。ストロークの間は変えない
/// （置き場はストロークの始めに決める）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceStencil {
    /// 画面の点 (gx, gy) → 画像の画素（左下が原点、y は上向き）: x = xx·gx + xy·gy + x0、y = yx·gx + yy·gy + y0。
    screen_to_image: StencilMapping,
    /// 画面の点 1 つあたりの画像の画素。
    image_per_point: f64,
}

impl SurfaceStencil {
    /// 有限でない値・負の比は断る。
    pub fn new(screen_to_image: StencilMapping, image_per_point: f64) -> Result<Self, CoreError> {
        if !image_per_point.is_finite() || image_per_point < 0.0 {
            return Err(CoreError::InvalidArgument("ステンシルの画像の画素 / 点"));
        }
        Ok(SurfaceStencil {
            screen_to_image,
            image_per_point,
        })
    }

    pub fn screen_to_image(&self) -> StencilMapping {
        self.screen_to_image
    }

    pub fn image_per_point(&self) -> f64 {
        self.image_per_point
    }

    /// ダブの所での、テクセル 1 つに当たる画像の画素の数（当たった三角形が分からない・面積が 0 なら 1）。
    pub(crate) fn footprint(
        &self,
        geometry: &SurfaceGeometry,
        view: &CameraView,
        hit: &SurfaceHit,
        width: i32,
        height: i32,
    ) -> f64 {
        let Some(t) = geometry.triangles().get(hit.triangle as usize) else {
            return 1.0;
        };
        let area = super::unity::cross(t.b - t.a, t.c - t.a);
        let area = super::unity::magnitude(area) as f64 * 0.5;
        let (e1, e2) = (t.uv_b - t.uv_a, t.uv_c - t.uv_a);
        let uv_area = (e1.x as f64 * e2.y as f64 - e1.y as f64 * e2.x as f64).abs()
            * 0.5
            * width as f64
            * height as f64;
        let usable = area.is_finite() && uv_area.is_finite() && area > 0.0 && uv_area > 0.0;
        if !usable {
            return 1.0;
        }
        let texel = (area / uv_area).sqrt() as f32;
        let points = view.world_radius_to_screen(hit.position, texel) as f64;
        let footprint = points * self.image_per_point;
        if footprint.is_finite() && footprint >= 0.0 {
            footprint
        } else {
            1.0
        }
    }

    /// 面のテクセルの点（モデルの空間）が読む、ステンシルの画像の点。カメラの後ろ（見えるテクセルでは起きない）や遠すぎる点は、
    /// 繰り返しでも読まない遠くの点にして塗らない。
    pub(crate) fn point(&self, view: &CameraView, position: Vec3, footprint: f64) -> StencilPoint {
        let far = StencilImage::FAR_AWAY;
        let nowhere = StencilPoint {
            x: -far,
            y: -far,
            footprint,
        };
        let Some(screen) = view.to_screen(position) else {
            return nowhere;
        };
        let (x, y) = self
            .screen_to_image
            .map_point(screen.x as f64, screen.y as f64);
        if x.abs() < far && y.abs() < far {
            StencilPoint { x, y, footprint }
        } else {
            nowhere
        }
    }
}
