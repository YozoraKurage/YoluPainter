//! 3D の面のストロークのリボン（[`crate::brush::ribbon`]。2D と同じ帯の決め方）: 描点の帯を、画面の点の半径で決め、帯ごとに 1 つのダブを
//! 塗る。帯の範囲と画像の位置は半径の単位なので、塗るときの半径（画面の点）にそのまま掛かる。
//!
//! Tangent | Wrap はダブの形を面に沿った長さで読むので、帯も面に沿った長さで切る: 線の長さを、前の描点の中心を今の描点の中心の展開の上で
//! 測った距離を足したモデルの単位の長さにし（[`SurfaceStroke::surface_arc`]）、半径と間隔もモデルの単位にする。画面の長さのままでは、
//! 斜めの面で帯が描点の間の面の長さに足りず隙間ができる。

use super::super::sampling::{SAMPLING_CHART_MAX_TRIANGLES, SAMPLING_CHART_TRIANGLE_BYTES};
use super::super::tangent_dab::MIN_FORESHORTENING;
use super::*;

/// Tangent | Wrap のリボンの、直前の描点の線の長さ（画面の点とモデルの単位）、中心の面の当たり（面に無ければ None）と、その所の線の向きの
/// 伸び（モデルの単位 ÷ 画面の点）と、ダブの間隔の縮め（法線と視線の cos。下限で止めたもの）。
#[derive(Clone, Copy, Debug)]
pub(super) struct RibbonArc {
    stamp: u64,
    screen: f64,
    surface: f64,
    hit: Option<SurfaceHit>,
    stretch: f64,
    foreshortening: f64,
}

/// 前の描点の中心を今の描点の中心の展開の上で測るとき、展開の半径に取る弦の長さの倍（これより遠回りの所は展開に入らず、弦に戻す）。
const UNFOLD_REACH: f64 = 2.0;

impl SurfaceStroke {
    /// 描点 `stamp` の置き方 `plan` を、リボンの帯ごとのダブにして塗る。面にまだ当たっていない描点は帯を決めない（塗らない。次に当たった
    /// 描点の帯は、遅れを 1 間隔までに詰めて続ける）。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn paint_ribbon(
        &mut self,
        doc: &mut Document,
        stroke: &mut Stroke,
        dab: &QueuedDab,
        pending: &PendingDab,
        plan: &DabPlan<'_>,
        controls: &StampControls,
        stamp: u64,
    ) -> Result<(), SurfaceStrokeError> {
        let Some(scale) = dab.scale else {
            self.stats.missed += 1;
            return Ok(());
        };
        // 塗るときの半径（[`SurfaceStroke::paint_plan`] と同じ値。最低 1 ピクセルの引き上げも含む）と、描点の間隔の見込み（`gap` と同じ式）
        let (_, hit) = self.dab_hit(dab, pending, plan);
        let world = self.dab_radius(dab, plan.radius, hit.as_ref());
        let radius = f64::from(scale * world);
        let wanted = 2.0 * scale * self.world_radius * self.spacing;
        let gap = f64::from(fmax(0.5, wanted));
        let (arc, radius, gap) = match self.projection.alignment {
            SurfaceAlignment::TangentWrap => {
                self.surface_arc(dab, stamp, scale, world, gap, f64::from(wanted))
            }
            SurfaceAlignment::Camera => (dab.arc, radius, gap),
        };
        let brush = self.brush.clone();
        let pieces = self.tips.ribbon_pieces(&brush, stamp, arc, radius, gap)?;
        for piece in &pieces {
            let part = brush.ribbon_plan(plan, piece, pending.direction);
            self.paint_plan(doc, stroke, dab, pending, &part, controls)?;
        }
        Ok(())
    }

    /// Tangent | Wrap の描点 `stamp` の帯を切る線の長さ・半径・間隔（モデルの単位）。world はモデルの単位の半径、gap は画面の間隔の見込み
    /// （0.5 で止めたもの）、wanted は止める前の画面の間隔。
    /// 線の長さは、前の描点と今の描点の中心がどちらも面にあれば、前の中心を今の中心の展開の上で測った距離（[`SurfaceStroke::unfolded`]。
    /// 平らな面では 2 点の距離。折れた辺では弦より長い）を足す。どちらかが面に無ければ、画面の長さに線の向きの伸びを掛けて足す。伸びと
    /// 縮めは描点の中心の面の点で測り（[`surface_heading`]）、線の向きがまだ無い描点（押した点・動かさない連続吹き付け）と中心が面に無い描点は
    /// 前の描点の値（無ければ 1）、視線が面に沿う所は縮めの下限とその逆数。間隔は、ダブの間隔の縮め（下限で止まる）より伸びが大きい所では
    /// 伸びに合わせて広げ、実際の描点の間隔（画面の上で 0.5 に止まる。`gaps` と同じ式）の面に沿った長さを下限にする（帯が描点に追いつける
    /// ように）。
    fn surface_arc(
        &mut self,
        dab: &QueuedDab,
        stamp: u64,
        scale: f32,
        world: f32,
        gap: f64,
        wanted: f64,
    ) -> (f64, f64, f64) {
        let scale = f64::from(scale);
        let least = f64::from(MIN_FORESHORTENING);
        let measured = dab
            .hit
            .as_ref()
            .filter(|_| dab.heading != DVec2::ZERO)
            .map(|h| {
                surface_heading(&self.view, h, dab.heading)
                    .map_or((1.0 / least, least), |s| (s.stretch, s.foreshortening))
            });
        let before = self.ribbon_arc;
        let (here, shrink) = match (measured, before) {
            (Some((m, f)), _) => (m / scale, f),
            (None, Some(a)) => (a.stretch, a.foreshortening),
            (None, None) => (1.0 / scale, 1.0),
        };
        let surface = match before {
            Some(a) if a.stamp == stamp => a.surface,
            Some(a) => {
                a.surface
                    + match (a.hit.as_ref(), dab.hit.as_ref()) {
                        (Some(p), Some(q)) => self.unfolded(p, q),
                        _ => (dab.arc - a.screen) * here,
                    }
            }
            None => dab.arc * here,
        };
        self.ribbon_arc = Some(RibbonArc {
            stamp,
            screen: dab.arc,
            surface,
            hit: dab.hit,
            stretch: here,
            foreshortening: shrink,
        });
        let widen = (least * here * scale).max(1.0);
        let spaced = (0.5f64).max(wanted * shrink) * here;
        (surface, f64::from(world), (gap / scale * widen).max(spaced))
    }

    /// 前の描点の中心 p を、今の描点の中心 q の展開（Tangent | Wrap のダブを塗るときと同じ辺の展開）の上で測った距離（モデルの単位）。
    /// 展開に p の三角形が入らない（弦の [`UNFOLD_REACH`] 倍より遠回り・別の面につながらない・図の上限を超える）ときは、2 点の弦の長さ。
    fn unfolded(&self, p: &SurfaceHit, q: &SurfaceHit) -> f64 {
        let chord = (q.position.as_dvec3() - p.position.as_dvec3()).length();
        if p.triangle == q.triangle || !(chord.is_finite() && chord > 0.0) {
            return chord;
        }
        self.geometry
            .build_sampling_chart(
                q,
                (chord * UNFOLD_REACH) as f32,
                Vec3::ZERO,
                SAMPLING_CHART_MAX_TRIANGLES,
                i64::from(SAMPLING_CHART_MAX_TRIANGLES) * SAMPLING_CHART_TRIANGLE_BYTES,
            )
            .ok()
            .and_then(|chart| chart.coordinates(p))
            .map(|at| at.as_dvec2().length())
            .filter(|d| d.is_finite())
            .unwrap_or(chord)
    }
}
