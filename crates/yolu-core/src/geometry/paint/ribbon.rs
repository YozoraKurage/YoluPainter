//! 3D の面のストロークのリボン（[`crate::brush::ribbon`]。2D と同じ帯の決め方）: 描点の帯を、画面の点の半径で決め、帯ごとに 1 つのダブを
//! 塗る。帯の範囲と画像の位置は半径の単位なので、塗るときの半径（画面の点）にそのまま掛かる。

use super::*;

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
        let radius = f64::from(scale * self.dab_radius(dab, plan.radius, hit.as_ref()));
        let gap = f64::from(fmax(0.5, 2.0 * scale * self.world_radius * self.spacing));
        let brush = self.brush.clone();
        let pieces = self
            .tips
            .ribbon_pieces(&brush, stamp, dab.arc, radius, gap)?;
        for piece in &pieces {
            let part = brush.ribbon_plan(plan, piece, pending.direction);
            self.paint_plan(doc, stroke, dab, pending, &part, controls)?;
        }
        Ok(())
    }
}
