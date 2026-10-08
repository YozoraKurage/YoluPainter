//! クローンの元と、その設定（揃える・全レイヤーから読むか）。2D のキャンバスと 3D ビューで同じ設定を共有し、元の点は 2D が文書の点、
//! 3D がモデルの面の点を別々に持つ。元は Alt を押したクリック（動かさずに離す）で決める。文書を替えたら（テクスチャセットの切り替えも）、
//! どちらの元も忘れる。
//!
//! - 3D: 面の点（決めたときのモデルの当たり。世代つきで、モデルが作り直されたらもう使えない）。揃えるなら、確定して画素が変わった前の
//!   ストロークの先の基準の点を、次のストロークの最初の点に対応させる。
//! - 2D: 文書の点。揃えるなら、元を決めた後の最初のストロークの始めの点 P0 で `offset = 元 − P0` を決め（ブラシの `Clone { offset }` に入れる）、
//!   そのストロークが確定して画素が変わったら、その後のストロークは同じ offset で続ける（取りやめた・何も変わらなかったストロークの後は、
//!   次のストロークの始めで決め直す。3D の先の基準と同じ）。決まった offset はブラシの外（ここ）にも持ち、ブラシを読み直しても（消しゴムとの
//!   往復・選び直し・元に戻す）続ける（3D の先の基準がブラシの読み直しで消えないのと同じ）。揃えないなら、ストロークごとに始めの点で
//!   `offset = 元 − 始めの点`（毎回、始めが元に重なる）。元を決めていない 2D は、今までどおりブラシの offset で写す。

use egui::{Color32, Painter, Pos2, Stroke};
use yolu_core::geometry::{SurfaceGeometry, SurfaceHit};
use yolu_core::glam::DVec2;

use crate::engine::BrushEffect;
use crate::state::AppState;

/// 2D の元（文書の点）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasSource {
    /// 元を決めた文書（ほかのテクスチャセット・別の文書では使わない）。
    pub document: u128,
    /// 元の点（文書の座標）。
    pub at: (f64, f64),
    /// 揃えるとき、元を決めた後のストロークで決まった offset（決まっていれば、以後のストロークはこの offset で続ける。欄で直した値も）。
    anchored: Option<DVec2>,
}

/// 元から決めてブラシの `Clone { offset }` に入れた offset と、入れる前のブラシの offset。
#[derive(Clone, Copy, Debug, PartialEq)]
struct Applied {
    offset: DVec2,
    /// ブラシ（サブツール）の持つ offset。ブラシの「変えた」の判定と、元を忘れたときに戻す値。
    own: DVec2,
}

/// クローンの元と、その設定。
#[derive(Clone, Debug, PartialEq)]
pub struct CloneState {
    /// 3D の写し元の面の点（決めていなければ None）。
    pub source: Option<SurfaceHit>,
    /// 3D の揃える: 前のストロークの先の基準の点（次のストロークの最初の点をこれに対応させる）。
    pub destination: Option<SurfaceHit>,
    /// 2D の写し元の点（決めていなければ None）。
    canvas: Option<CanvasSource>,
    /// 元から決めて、ブラシの `Clone { offset }` に入れた offset（元の点は、ブラシの設定でなく、そのときの作業の位置なので、ブラシの
    /// 「変えた」に数えない）。欄で直したら外し、ブラシ・効果の種類を替えたら、読んだブラシの offset で覚え直す。
    applied: Option<Applied>,
    /// 揃える（前のストロークと同じ位置関係で続ける）。切ると、ストロークごとに最初の点が元に重なる。
    pub aligned: bool,
    /// 描くレイヤーだけでなく、見えているレイヤーの重なりを読む。
    pub all_layers: bool,
}

impl Default for CloneState {
    fn default() -> Self {
        CloneState {
            source: None,
            destination: None,
            canvas: None,
            applied: None,
            aligned: true,
            all_layers: false,
        }
    }
}

impl CloneState {
    /// 3D の元が今のモデルの面として使えるか（世代が同じ）。
    pub fn source_for(&self, geometry: &SurfaceGeometry) -> Option<SurfaceHit> {
        self.source.filter(|s| {
            s.revision == geometry.revision() && (s.triangle as usize) < geometry.triangle_count()
        })
    }

    /// 3D の次のストロークの先の基準（揃えるときだけ。前のストロークと同じモデルのもの）。
    pub fn destination_for(&self, geometry: &SurfaceGeometry) -> Option<SurfaceHit> {
        if !self.aligned {
            return None;
        }
        self.destination.filter(|d| {
            d.revision == geometry.revision() && (d.triangle as usize) < geometry.triangle_count()
        })
    }

    /// 3D の元を決める（先の基準は決め直す）。
    pub fn set_source(&mut self, hit: SurfaceHit) {
        self.source = Some(hit);
        self.destination = None;
    }

    /// 2D の元（文書 `document` の点）。別の文書で決めたものは使えない。
    pub fn canvas_source_for(&self, document: u128) -> Option<(f64, f64)> {
        self.canvas.filter(|c| c.document == document).map(|c| c.at)
    }

    /// 2D の元を決める（次のストロークの始めで offset を決め直す）。
    pub fn set_canvas_source(&mut self, document: u128, at: (f64, f64)) {
        self.canvas = Some(CanvasSource {
            document,
            at,
            anchored: None,
        });
    }

    /// 揃えるを切り替える（入れ直したら、3D は前の先の基準を、2D は決め済みの offset を捨てて、次のストロークから揃え直す）。
    pub fn set_aligned(&mut self, aligned: bool) {
        self.aligned = aligned;
        self.destination = None;
        if let Some(c) = self.canvas.as_mut() {
            c.anchored = None;
        }
    }

    /// ブラシを読んだ・効果の種類を替えた（`effect` は読んだブラシの効果）: 前に入れた offset はもう数えない。読んだブラシがクローンで、
    /// 揃える offset が決まっていれば、それを入れ直し、読んだ offset をブラシの持つ offset として覚える（揃える位置関係は、ブラシを読み直しても
    /// 続ける）。
    pub fn brush_loaded(&mut self, effect: &mut BrushEffect) {
        self.applied = None;
        let BrushEffect::Clone { offset } = effect else {
            return;
        };
        let Some(kept) = self
            .canvas
            .and_then(|c| c.anchored)
            .filter(|_| self.aligned)
        else {
            return;
        };
        self.applied = Some(Applied {
            offset: kept,
            own: *offset,
        });
        *offset = kept;
    }

    /// 欄で offset を `value` に直した: 直した値を次のストロークから使う（元からの決め直しをしない）。ブラシの設定を変えたので、「変えた」に数える。
    pub fn offset_edited(&mut self, value: DVec2) {
        if let Some(c) = self.canvas.as_mut() {
            c.anchored = Some(value);
        }
        self.applied = None;
    }

    /// 2D のストロークの始め（始めの点 `start`、文書の座標）で使う offset。`current` は今ブラシに入っている offset。元を決めていなければ
    /// None（ブラシの offset のまま）。揃えるなら、決まった offset（`canvas_stroke_kept`・欄で直した値）があればそれ、無ければ元から決める。
    /// 揃えないなら毎回元から決める。
    pub fn canvas_offset(
        &mut self,
        document: u128,
        start: (f64, f64),
        current: DVec2,
    ) -> Option<DVec2> {
        let c = self.canvas.filter(|c| c.document == document)?;
        let offset = match c.anchored.filter(|_| self.aligned) {
            Some(kept) => kept,
            None => DVec2::new(c.at.0 - start.0, c.at.1 - start.1),
        };
        // ブラシの持つ offset は、前に元から入れた offset のままなら、そのとき覚えた値
        let own = match self.applied {
            Some(a) if a.offset == current => a.own,
            _ => current,
        };
        self.applied = Some(Applied { offset, own });
        Some(offset)
    }

    /// 元から offset（`offset`）を決めた 2D のストロークが確定して、画素が変わった: 揃えるなら、以後のストロークは同じ offset で続ける。
    pub fn canvas_stroke_kept(&mut self, document: u128, offset: DVec2) {
        if !self.aligned {
            return;
        }
        if let Some(c) = self.canvas.as_mut().filter(|c| c.document == document) {
            c.anchored = Some(offset);
        }
    }

    /// ブラシの `Clone { offset }` が、元から決めて入れた offset のままなら、ブラシの持つ offset（ブラシの「変えた」の判定はこれで比べる）。
    pub fn brush_offset(&self, current: DVec2) -> Option<DVec2> {
        self.applied.filter(|a| a.offset == current).map(|a| a.own)
    }

    /// 元を決めた 2D で、ストロークごとに offset を元から決め直すか（揃えない）。このあいだ offset の欄を直しても、次のストロークで上書きされる。
    pub fn canvas_resets_offset(&self, document: u128) -> bool {
        !self.aligned && self.canvas_source_for(document).is_some()
    }

    /// 2D と 3D の元を忘れる（揃える・全レイヤーからの設定は残す）。ブラシに元から入れた offset があれば、戻す値を返す。
    fn forget_sources(&mut self) -> Option<Applied> {
        self.source = None;
        self.destination = None;
        self.canvas = None;
        self.applied.take()
    }
}

impl AppState {
    /// 文書を替えた（テクスチャセットの切り替え・開き直しも）: 2D の元（前の文書の点）と 3D の元（前の文書を読む面の点）を忘れる。
    /// 元から決めた offset がブラシに入ったままなら、ブラシの持つ offset に戻す（元の無い 2D は、ブラシの offset で写す）。
    pub(crate) fn forget_clone_sources(&mut self) {
        let Some(applied) = self.clone.forget_sources() else {
            return;
        };
        if let BrushEffect::Clone { offset } = &mut self.m2.brush.effect {
            if *offset == applied.offset {
                *offset = applied.own;
            }
        }
    }
}

/// 今のツールがクローンのブラシか（元を決められる）。編集・ポーズのモードでは描かないので偽。
pub fn active(app: &AppState) -> bool {
    app.mode.paints()
        && app.tool.paints()
        && !app.tool.erases()
        && matches!(app.m2.brush.effect, BrushEffect::Clone { .. })
}

/// 元の印（十字。黒い縁の上に強調色）を `at`（画面の点）に描く。
pub fn paint_mark(painter: &Painter, at: Pos2) {
    let accent = crate::ui::theme::ACCENT;
    let shade = Color32::from_black_alpha(160);
    for (half, stroke) in [
        (7.0, Stroke::new(3.0, shade)),
        (6.0, Stroke::new(1.5, accent)),
    ] {
        painter.line_segment(
            [at - egui::vec2(half, 0.0), at + egui::vec2(half, 0.0)],
            stroke,
        );
        painter.line_segment(
            [at - egui::vec2(0.0, half), at + egui::vec2(0.0, half)],
            stroke,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: u128 = 7;
    const OWN: DVec2 = DVec2::new(64.0, 0.0);

    #[test]
    fn aligned_decides_the_offset_until_a_stroke_keeps_it_and_unaligned_every_stroke() {
        let mut clone = CloneState::default();
        // 元を決めていないあいだは、ブラシの offset のまま
        assert_eq!(clone.canvas_offset(DOC, (10.0, 10.0), OWN), None);
        clone.set_canvas_source(DOC, (100.0, 50.0));
        // 揃える: 元を決めた後の最初のストロークの始めの点で決める
        let first = DVec2::new(90.0, 30.0);
        assert_eq!(clone.canvas_offset(DOC, (10.0, 20.0), OWN), Some(first));
        assert_eq!(
            clone.brush_offset(first),
            Some(OWN),
            "ブラシの持つ offset を覚える"
        );
        // 取りやめた・何も変わらなかったストロークの後は、次の始めで決め直す
        let again = DVec2::new(80.0, 30.0);
        assert_eq!(clone.canvas_offset(DOC, (20.0, 20.0), first), Some(again));
        assert_eq!(
            clone.brush_offset(again),
            Some(OWN),
            "入れた offset の上に入れても、ブラシの値は前のまま"
        );
        // 確定して変わったら、以後は同じ offset で続ける
        clone.canvas_stroke_kept(DOC, again);
        assert_eq!(clone.canvas_offset(DOC, (40.0, 70.0), again), Some(again));
        // 揃えるを入れ直したら、次のストロークから揃え直す（ブラシの値は覚えたまま）
        clone.set_aligned(false);
        clone.set_aligned(true);
        let realigned = DVec2::new(60.0, -20.0);
        assert_eq!(
            clone.canvas_offset(DOC, (40.0, 70.0), again),
            Some(realigned)
        );
        assert_eq!(clone.brush_offset(realigned), Some(OWN));
        // 揃えない: ストロークごとに始めの点が元に重なる（確定しても）
        clone.set_aligned(false);
        assert!(clone.canvas_resets_offset(DOC));
        assert_eq!(
            clone.canvas_offset(DOC, (40.0, 70.0), realigned),
            Some(realigned)
        );
        clone.canvas_stroke_kept(DOC, realigned);
        assert_eq!(
            clone.canvas_offset(DOC, (0.0, 0.0), realigned),
            Some(DVec2::new(100.0, 50.0))
        );
    }

    #[test]
    fn a_source_belongs_to_the_document_it_was_set_in() {
        let mut clone = CloneState::default();
        clone.set_canvas_source(DOC, (1.0, 2.0));
        assert_eq!(clone.canvas_source_for(DOC), Some((1.0, 2.0)));
        assert_eq!(clone.canvas_source_for(DOC + 1), None);
        assert_eq!(clone.canvas_offset(DOC + 1, (0.0, 0.0), OWN), None);
        assert!(!clone.canvas_resets_offset(DOC + 1));
        let offset = clone.canvas_offset(DOC, (0.0, 0.0), OWN).unwrap();
        // 忘れると、2D も 3D も元が無くなり、ブラシに戻す値を返す（設定は残す）
        clone.all_layers = true;
        let applied = clone.forget_sources().expect("入れた offset");
        assert_eq!((applied.offset, applied.own), (offset, OWN));
        assert_eq!(clone.canvas_source_for(DOC), None);
        assert_eq!(clone.source, None);
        assert_eq!(clone.brush_offset(offset), None);
        assert!(clone.all_layers && clone.aligned);
    }

    #[test]
    fn a_kept_offset_outlives_reloading_the_brush_and_an_edited_offset_is_the_brush_own() {
        let mut clone = CloneState::default();
        clone.set_canvas_source(DOC, (100.0, 50.0));
        let decided = DVec2::new(90.0, 40.0);
        assert_eq!(clone.canvas_offset(DOC, (10.0, 10.0), OWN), Some(decided));
        clone.canvas_stroke_kept(DOC, decided);
        // ブラシを読み直す（消しゴムとの往復・選び直し）: 揃える offset を入れ直し、読んだ offset をブラシの値として覚える
        let mut eraser = BrushEffect::Paint;
        clone.brush_loaded(&mut eraser);
        assert_eq!(eraser, BrushEffect::Paint, "クローンでないブラシは変えない");
        let mut reloaded = BrushEffect::Clone { offset: OWN };
        clone.brush_loaded(&mut reloaded);
        assert_eq!(reloaded, BrushEffect::Clone { offset: decided });
        assert_eq!(clone.brush_offset(decided), Some(OWN), "変えたに数えない");
        assert_eq!(
            clone.canvas_offset(DOC, (30.0, 30.0), decided),
            Some(decided)
        );
        // 決まる前（揃えない・決め直す前）は、読んだ offset のまま
        clone.set_aligned(false);
        let mut plain = BrushEffect::Clone { offset: OWN };
        clone.brush_loaded(&mut plain);
        assert_eq!(plain, BrushEffect::Clone { offset: OWN });
        assert_eq!(clone.brush_offset(OWN), None);
        clone.set_aligned(true);
        // 欄で直した値は、次のストロークから使い、ブラシの値として数える（入れた offset は数えない）
        let edited = DVec2::new(5.0, 6.0);
        clone.offset_edited(edited);
        assert_eq!(clone.brush_offset(edited), None);
        assert_eq!(clone.canvas_offset(DOC, (10.0, 10.0), edited), Some(edited));
        assert_eq!(clone.brush_offset(edited), Some(edited));
        // 元を決め直したら、決め直す。入れた offset はブラシの値として数えないまま
        clone.set_canvas_source(DOC, (0.0, 0.0));
        let next = DVec2::new(-5.0, -5.0);
        assert_eq!(clone.canvas_offset(DOC, (5.0, 5.0), OWN), Some(next));
        clone.set_canvas_source(DOC, (1.0, 1.0));
        assert_eq!(
            clone.brush_offset(next),
            Some(OWN),
            "元を決め直しても、入れた offset は変えたに数えない"
        );
    }
}
