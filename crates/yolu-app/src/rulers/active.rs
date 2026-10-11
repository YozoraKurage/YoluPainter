//! 今の描く先で効く定規。描くレイヤーから見える定規（文書の `rulers_seen_from`）を、スナップの入り切りで絞り、寄せ先と対称の写しにする。
//! 2D のキャンバスのストロークも 3D ビューのストロークも、定規を読むのはここだけ。ストロークの始めに固める（途中で定規を替えても、そのストロークには効かない）。
//!
//! - 直線定規は、描き始めの点が画面で `SNAP_POINTS` の内にあるものの中の一番近い物へ寄せる（`nearest_line`）。
//! - 特殊定規（平行線・同心円・パース・対称）は、2D と 3D で別に、`snap` の印のある 1 つだけが効く（文書の `snapping_special_rulers`）。
//!   平行線・同心円・パースは寄せ先に、対称は写しになる。どちらの描く先でも、2D の対称定規と 3D の対称定規は同時に効く。
//! - 「定規にスナップ」を切ると直線定規が、「特殊定規にスナップ」を切ると特殊定規が（対称も）効かない。

use egui::Pos2;
use yolu_core::geometry::SurfaceSymmetrySetup;
use yolu_core::glam::DVec2;
use yolu_core::{CanvasSymmetry, LayerId, Ruler, RulerKind, RulerRef, RulerSpace};

use super::SNAP_POINTS;
use crate::canvas::view::CanvasView;
use crate::drafting::Constraint;
use crate::state::AppState;

/// 描く所。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// 2D のキャンバス。
    Canvas,
    /// 3D ビュー。
    View3d,
}

impl Place {
    /// この描く所の寄せ先になる定規の空間。
    pub fn space(self) -> RulerSpace {
        match self {
            Place::Canvas => RulerSpace::Canvas,
            Place::View3d => RulerSpace::Model,
        }
    }
}

/// 描くレイヤーから見て、この描く所で効く定規。
#[derive(Clone, Debug, Default)]
pub struct Active<'a> {
    /// 寄せの候補の直線定規（この描く所の空間のもの。「定規にスナップ」が入のとき）。
    pub straight: Vec<RulerRef<'a>>,
    /// 効いている特殊定規のうち、寄せ先になるもの（平行線・同心円・パース。この描く所の空間のもの）。
    pub special: Option<RulerRef<'a>>,
    /// 効いている 2D の対称定規の写し。
    pub canvas_symmetry: Option<CanvasSymmetry>,
    /// 効いている 3D の対称定規の写し。
    pub surface_symmetry: Option<SurfaceSymmetrySetup>,
}

/// 表示する定規（印つき）。
#[derive(Clone, Copy, Debug)]
pub struct Shown<'a> {
    pub owner: LayerId,
    pub ruler: &'a Ruler,
    /// 寄せ・写しが効いている（直線定規は「定規にスナップ」が入、特殊定規は効いている 1 つで「特殊定規にスナップ」が入）。
    pub effective: bool,
    /// 選んでいるレイヤー（グループ）に付いていて編集できる。
    pub editable: bool,
    /// プロパティで選んでいる定規。
    pub selected: bool,
}

impl AppState {
    /// 選んでいるレイヤーで描くときに効く定規。
    pub fn rulers_active(&self, place: Place) -> Active<'_> {
        self.rulers_active_for(place, self.selected_layer)
    }

    /// `drawing` で描くときに効く定規。
    pub fn rulers_active_for(&self, place: Place, drawing: Option<LayerId>) -> Active<'_> {
        let mut active = Active::default();
        if self.doc.layers().iter().all(|l| l.rulers().is_empty()) {
            return active;
        }
        if self.rulers.snap_ruler {
            active.straight = self
                .doc
                .straight_rulers_seen_from(drawing)
                .into_iter()
                .filter(|r| r.ruler.space() == place.space())
                .collect();
        }
        if self.rulers.snap_special {
            let special = self.doc.snapping_special_rulers(drawing);
            active.canvas_symmetry = special
                .canvas
                .filter(|r| r.ruler.kind == RulerKind::Symmetry)
                .and_then(|r| r.ruler.canvas_symmetry());
            active.surface_symmetry = special
                .model
                .filter(|r| r.ruler.kind == RulerKind::Symmetry)
                .and_then(|r| r.ruler.surface_symmetry());
            active.special = match place {
                Place::Canvas => special.canvas,
                Place::View3d => special.model,
            }
            .filter(|r| r.ruler.kind != RulerKind::Symmetry);
        }
        active
    }

    /// この描く所で表示する定規（見えているものだけ）。
    pub fn rulers_shown(&self, place: Place) -> Vec<Shown<'_>> {
        let drawing = self.selected_layer;
        if self.doc.layers().iter().all(|l| l.rulers().is_empty()) {
            return Vec::new();
        }
        let space = place.space();
        let special = self.doc.snapping_special_rulers(drawing);
        let chosen = match space {
            RulerSpace::Canvas => special.canvas,
            RulerSpace::Model => special.model,
        };
        self.doc
            .rulers_seen_from(drawing)
            .into_iter()
            .filter(|r| r.ruler.space() == space)
            .map(|r| {
                let effective = if r.ruler.kind == RulerKind::Line {
                    self.rulers.snap_ruler
                } else {
                    self.rulers.snap_special && chosen.is_some_and(|c| c.ruler.id == r.ruler.id)
                };
                Shown {
                    owner: r.owner,
                    ruler: r.ruler,
                    effective,
                    editable: drawing == Some(r.owner),
                    selected: self.rulers.selected == Some((r.owner, r.ruler.id)),
                }
            })
            .collect()
    }

    /// 選んでいる定規（持ち主が選んでいるレイヤーのときだけ）。
    pub fn selected_ruler(&self) -> Option<(LayerId, &Ruler)> {
        let (owner, id) = self.rulers.selected?;
        if self.selected_layer != Some(owner) {
            return None;
        }
        let ruler = self
            .doc
            .layer(owner)?
            .rulers()
            .iter()
            .find(|r| r.id == id)?;
        Some((owner, ruler))
    }

    /// 描く所の対称定規が効いていないときの断りの文。「特殊定規にスナップ」が切れているせいで効いていない（入れれば効く）ときはそれを、
    /// 対称定規が無いときは無いことを言う。
    pub fn no_symmetry_reason(&self, place: Place, drawing: LayerId) -> &'static str {
        let lang = self.lang;
        if !self.rulers.snap_special {
            let special = self.doc.snapping_special_rulers(Some(drawing));
            let chosen = match place {
                Place::Canvas => special.canvas,
                Place::View3d => special.model,
            };
            if chosen.is_some_and(|r| r.ruler.kind == RulerKind::Symmetry) {
                return lang.pick(
                    "特殊定規にスナップが切れています",
                    "Snap to Special Ruler is off",
                );
            }
        }
        lang.pick("対称定規がありません", "No symmetry ruler")
    }

    /// 2D のキャンバスの点 `at`（キャンバスの座標）から、バケツ・自動選択が範囲を求める種の点。`snap` が真で、効いている 2D の対称定規が
    /// あれば、`at` を写しの全部の点にして返す（最初は `at` 自身）。キャンバスの外の点は捨て、同じ画素に重なった点は 1 回にする。
    /// 切か対称定規が無ければ、`at` だけ（キャンバスの外なら空）。
    pub fn symmetry_seeds(&self, at: (f64, f64), snap: bool) -> Vec<(f64, f64)> {
        let (w, h) = (self.doc.width() as f64, self.doc.height() as f64);
        let inside =
            |p: &(f64, f64)| p.0.is_finite() && (0.0..w).contains(&p.0) && (0.0..h).contains(&p.1);
        let transforms = snap
            .then(|| self.rulers_active(Place::Canvas).canvas_symmetry)
            .flatten()
            .and_then(|s| s.transforms().ok());
        let mut seeds: Vec<(f64, f64)> = Vec::new();
        let mut pixels: Vec<(i64, i64)> = Vec::new();
        let points = match &transforms {
            // 最初の変換は恒等。押した点そのものを使う（式を通して端数を変えない）
            Some(ts) => ts
                .iter()
                .enumerate()
                .map(|(i, t)| if i == 0 { at } else { t.map(at.0, at.1) })
                .collect(),
            None => vec![at],
        };
        for p in points.into_iter().filter(inside) {
            let pixel = (p.0.floor() as i64, p.1.floor() as i64);
            if !pixels.contains(&pixel) {
                pixels.push(pixel);
                seeds.push(p);
            }
        }
        seeds
    }

    /// 選んでいる定規の持ち主が、選んでいるレイヤーでなくなったら選びを外す（別のレイヤーの定規を指したままにしない）。
    pub fn drop_foreign_ruler_selection(&mut self) {
        if self
            .rulers
            .selected
            .is_some_and(|(owner, _)| self.selected_layer != Some(owner))
        {
            self.rulers.selected = None;
        }
    }

    /// 2D のキャンバスのストロークの始めに決める寄せ先（押した点は画面の点）。直線定規の近く（`SNAP_POINTS`）で押したときはその線へ、
    /// そうでなければ効いている平行線・同心円・パースへ。
    pub fn canvas_ruler_constraint(
        &self,
        view: &CanvasView,
        drawing: Option<LayerId>,
        press: Pos2,
    ) -> Option<Constraint> {
        let active = self.rulers_active_for(Place::Canvas, drawing);
        if active.straight.is_empty() && active.special.is_none() {
            return None;
        }
        let screen = |p: DVec2| view.to_screen(p.x, p.y);
        let lines: Vec<(DVec2, DVec2)> = active
            .straight
            .iter()
            .filter_map(|r| canvas_points(r.ruler))
            .collect();
        if let Some(c) = nearest_line(&lines, &screen, press) {
            return Some(c);
        }
        let special = active.special?;
        let (x, y) = view.to_canvas(press);
        canvas_constraint(special.ruler, DVec2::new(x, y))
    }
}

/// 2D の定規の 2 点。
pub fn canvas_points(r: &Ruler) -> Option<(DVec2, DVec2)> {
    match r.place {
        yolu_core::RulerPlace::Canvas { a, b } => Some((a, b)),
        yolu_core::RulerPlace::Model { .. } => None,
    }
}

/// 2D の定規（直線・平行線・同心円・パース）の、ストロークの始めの点 `start`（文書の座標）での寄せ先（対称・3D の定規は None）。
pub fn canvas_constraint(r: &Ruler, start: DVec2) -> Option<Constraint> {
    let (a, b) = canvas_points(r)?;
    let direction = (b - a).try_normalize().unwrap_or(DVec2::X);
    match r.kind {
        RulerKind::Line => Some(Constraint::Line {
            origin: a,
            direction,
        }),
        RulerKind::Parallel => Some(Constraint::Line {
            origin: start,
            direction,
        }),
        RulerKind::Concentric => Some(Constraint::Circle {
            center: a,
            radius: start.distance(a),
            start,
        }),
        RulerKind::Perspective => Some(Constraint::Perspective {
            start,
            a,
            b: r.two_points.then_some(b),
        }),
        RulerKind::Symmetry => None,
    }
}

/// 押した点（画面の点）から画面で `SNAP_POINTS` の内にある直線のうち、一番近いものへの寄せ先（線は a・b を通って両側へ伸びる。
/// 2 点は文書の座標）。
pub fn nearest_line(
    lines: &[(DVec2, DVec2)],
    screen: &impl Fn(DVec2) -> Pos2,
    press: Pos2,
) -> Option<Constraint> {
    let mut best: Option<(f64, (DVec2, DVec2))> = None;
    for &(a, b) in lines {
        let (sa, sb) = (screen(a), screen(b));
        // 画面で 1 点より縮んだ直線は、向きが決まらないので入れない（3D の特殊定規の向きが取れないときと同じ）
        if sa.distance(sb) < 1.0 {
            continue;
        }
        let d = screen_distance_to_line(sa, sb, press);
        if d <= SNAP_POINTS && best.is_none_or(|(near, _)| d < near) {
            best = Some((d, (a, b)));
        }
    }
    best.map(|(_, (a, b))| Constraint::Line {
        origin: a,
        direction: (b - a).try_normalize().unwrap_or(DVec2::X),
    })
}

/// 画面の点 `p` から、`a`・`b` を通る直線までの距離（a と b が重なっていれば a までの距離）。
fn screen_distance_to_line(a: Pos2, b: Pos2, p: Pos2) -> f64 {
    distance_to_line(
        DVec2::new(a.x as f64, a.y as f64),
        DVec2::new(b.x as f64, b.y as f64),
        DVec2::new(p.x as f64, p.y as f64),
    )
}

/// 点 `p` から、`a`・`b` を通る直線までの距離（a と b が重なっていれば a までの距離）。
fn distance_to_line(a: DVec2, b: DVec2, p: DVec2) -> f64 {
    let (d, q) = (b - a, p - a);
    let length = d.length();
    if length < 1e-9 {
        return q.length();
    }
    d.perp_dot(q).abs() / length
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id_screen(p: DVec2) -> Pos2 {
        Pos2::new(p.x as f32, p.y as f32)
    }

    #[test]
    fn distance_is_measured_to_the_infinite_line() {
        let (a, b) = (Pos2::new(0.0, 0.0), Pos2::new(10.0, 0.0));
        assert!((screen_distance_to_line(a, b, Pos2::new(5.0, 7.0)) - 7.0).abs() < 1e-9);
        assert!(
            (screen_distance_to_line(a, b, Pos2::new(500.0, -3.0)) - 3.0).abs() < 1e-9,
            "端の外でも線の延長までの距離"
        );
        assert!((screen_distance_to_line(a, a, Pos2::new(3.0, 4.0)) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn a_line_shrunk_below_one_screen_point_is_not_a_candidate() {
        // 画面で 0.5 点しか離れていない 2 点の直線は、向きが決まらないので入れない（遠くの定規を押した点の近くとみなさない）
        let tiny = [(DVec2::new(20.0, 100.0), DVec2::new(20.5, 100.0))];
        assert!(nearest_line(&tiny, &id_screen, Pos2::new(20.0, 100.0)).is_none());
        // 1 点ちょうどからは候補
        let one = [(DVec2::new(20.0, 100.0), DVec2::new(21.0, 100.0))];
        assert!(nearest_line(&one, &id_screen, Pos2::new(20.0, 100.0)).is_some());
        // 縮んだ直線は、ほかの直線を邪魔しない
        let both = [tiny[0], (DVec2::new(0.0, 110.0), DVec2::new(50.0, 110.0))];
        let c = nearest_line(&both, &id_screen, Pos2::new(20.0, 100.0)).unwrap();
        assert_eq!(
            c,
            Constraint::Line {
                origin: DVec2::new(0.0, 110.0),
                direction: DVec2::X
            }
        );
    }

    #[test]
    fn the_nearest_line_within_the_snap_distance_wins() {
        let lines = [
            (DVec2::new(0.0, 100.0), DVec2::new(50.0, 100.0)),
            (DVec2::new(0.0, 110.0), DVec2::new(50.0, 110.0)),
        ];
        let c = nearest_line(&lines, &id_screen, Pos2::new(20.0, 108.0)).unwrap();
        assert_eq!(
            c,
            Constraint::Line {
                origin: DVec2::new(0.0, 110.0),
                direction: DVec2::X
            }
        );
        // 26 点ちょうどは寄る、超えたら寄らない
        assert!(nearest_line(&lines[..1], &id_screen, Pos2::new(20.0, 100.0 + 26.0)).is_some());
        assert!(nearest_line(&lines[..1], &id_screen, Pos2::new(20.0, 100.0 + 26.5)).is_none());
        assert!(nearest_line(&[], &id_screen, Pos2::ZERO).is_none());
    }

    use crate::rulers::RulerAction;
    use crate::state::Action;
    use yolu_core::glam::DVec3;
    use yolu_core::RulerId;

    fn state() -> AppState {
        let mut s = AppState::new(64, 64);
        s.doc.clear_history().unwrap();
        s
    }

    fn put(s: &mut AppState, kind: RulerKind, a: (f64, f64), b: (f64, f64)) -> RulerId {
        let r = Ruler::canvas(RulerId(1), kind, DVec2::new(a.0, a.1), DVec2::new(b.0, b.1));
        s.ruler_create(r).unwrap()
    }

    #[test]
    fn the_snaps_gate_straight_rulers_special_rulers_and_the_symmetry_copies() {
        let mut s = state();
        put(&mut s, RulerKind::Line, (0.0, 10.0), (64.0, 10.0));
        put(&mut s, RulerKind::Symmetry, (32.0, 32.0), (32.0, 40.0));
        let active = s.rulers_active(Place::Canvas);
        assert_eq!(active.straight.len(), 1);
        assert!(active.canvas_symmetry.is_some());
        assert!(active.special.is_none(), "対称は寄せ先ではなく写し");
        assert!(active.surface_symmetry.is_none());
        s.apply(Action::Ruler(RulerAction::ToggleSnapRuler));
        let active = s.rulers_active(Place::Canvas);
        assert!(
            active.straight.is_empty(),
            "定規にスナップを切ると直線定規は効かない"
        );
        assert!(active.canvas_symmetry.is_some());
        s.apply(Action::Ruler(RulerAction::ToggleSnapSpecial));
        let active = s.rulers_active(Place::Canvas);
        assert!(
            active.canvas_symmetry.is_none(),
            "特殊定規にスナップを切ると対称も効かない"
        );
    }

    #[test]
    fn the_reason_for_no_symmetry_names_the_switched_off_snap_only_when_turning_it_on_would_help() {
        let mut s = state();
        let layer = s.selected_layer.unwrap();
        let none = s.lang.pick("対称定規がありません", "No symmetry ruler");
        let off = s.lang.pick(
            "特殊定規にスナップが切れています",
            "Snap to Special Ruler is off",
        );
        for place in [Place::Canvas, Place::View3d] {
            assert_eq!(s.no_symmetry_reason(place, layer), none, "{place:?}");
        }
        put(&mut s, RulerKind::Symmetry, (32.0, 32.0), (32.0, 40.0));
        s.ruler_create(Ruler::model(
            RulerId(1),
            RulerKind::Symmetry,
            DVec3::ZERO,
            DVec3::X,
            DVec3::Y,
        ))
        .unwrap();
        s.rulers.snap_special = false;
        for place in [Place::Canvas, Place::View3d] {
            assert_eq!(s.no_symmetry_reason(place, layer), off, "{place:?}");
        }
        // 切れていても、対称が選ばれていなければ（平行線に印が移った 2D）切れていることは理由でない
        put(&mut s, RulerKind::Parallel, (0.0, 0.0), (10.0, 5.0));
        s.rulers.snap_special = false;
        assert_eq!(s.no_symmetry_reason(Place::Canvas, layer), none);
        assert_eq!(s.no_symmetry_reason(Place::View3d, layer), off);
    }

    #[test]
    fn a_2d_and_a_3d_symmetry_both_take_effect_in_both_places_and_other_special_rulers_only_in_their_own(
    ) {
        let mut s = state();
        put(&mut s, RulerKind::Symmetry, (32.0, 32.0), (32.0, 40.0));
        let model = Ruler::model(
            RulerId(1),
            RulerKind::Symmetry,
            DVec3::ZERO,
            DVec3::X,
            DVec3::Y,
        );
        s.ruler_create(model).unwrap();
        for place in [Place::Canvas, Place::View3d] {
            let active = s.rulers_active(place);
            assert!(active.canvas_symmetry.is_some(), "{place:?}");
            assert!(active.surface_symmetry.is_some(), "{place:?}");
        }
        // 2D の平行線は 2D の寄せ先だけ（2D の対称は外れる。同じ空間の特殊定規は 1 つ）
        put(&mut s, RulerKind::Parallel, (0.0, 0.0), (10.0, 5.0));
        let canvas = s.rulers_active(Place::Canvas);
        assert!(canvas
            .special
            .is_some_and(|r| r.ruler.kind == RulerKind::Parallel));
        assert!(
            canvas.canvas_symmetry.is_none(),
            "同じ空間では同時に効かない"
        );
        assert!(canvas.surface_symmetry.is_some(), "3D の対称は別に効く");
        let view3d = s.rulers_active(Place::View3d);
        assert!(
            view3d.special.is_none(),
            "2D の平行線は 3D の寄せ先にならない"
        );
        assert!(view3d.canvas_symmetry.is_none());
    }

    #[test]
    fn what_is_seen_follows_the_scope_the_eye_and_the_drawing_layer() {
        let mut s = state();
        let below = s.selected_layer.unwrap();
        let top = s.doc.add_layer("Top").unwrap();
        let group = s.doc.group_layers(&[top], "G").unwrap();
        s.selected_layer = Some(top);
        s.doc.clear_history().unwrap();
        // 一番上の段のレイヤーに付けた定規は、文書の全部のレイヤーから見える（既定の範囲）
        s.selected_layer = Some(below);
        put(&mut s, RulerKind::Line, (0.0, 10.0), (64.0, 10.0));
        s.selected_layer = Some(top);
        assert_eq!(s.rulers_shown(Place::Canvas).len(), 1);
        assert!(
            !s.rulers_shown(Place::Canvas)[0].editable,
            "選んでいるレイヤーの物ではない"
        );
        // 持ち主の目を閉じると隠れる
        s.doc.set_layer_visible(below, false).unwrap();
        assert!(s.rulers_shown(Place::Canvas).is_empty());
        assert!(s.rulers_active(Place::Canvas).straight.is_empty());
        s.doc.set_layer_visible(below, true).unwrap();
        // グループの目を閉じても、グループの中のレイヤーに付けた定規は隠れる
        put(&mut s, RulerKind::Line, (0.0, 20.0), (64.0, 20.0));
        assert_eq!(s.rulers_shown(Place::Canvas).len(), 2);
        assert!(s.rulers_shown(Place::Canvas).iter().any(|r| r.editable));
        s.doc.set_layer_visible(group, false).unwrap();
        assert_eq!(s.rulers_shown(Place::Canvas).len(), 1);
    }

    #[test]
    fn the_constraint_is_the_nearest_straight_ruler_within_26_points_else_the_special_ruler() {
        let mut s = state();
        let rect = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(640.0, 640.0));
        let view = s.view.view(rect, 64, 64);
        let layer = s.selected_layer;
        put(&mut s, RulerKind::Line, (0.0, 20.0), (64.0, 20.0));
        let at = |x: f64, y: f64| view.to_screen(x, y);
        // 20 の線の上 2.5 画素 = 25 点（内）
        let c = s.canvas_ruler_constraint(&view, layer, at(10.0, 22.5));
        assert!(
            matches!(c, Some(Constraint::Line { origin, .. }) if origin == DVec2::new(0.0, 20.0))
        );
        assert!(
            s.canvas_ruler_constraint(&view, layer, at(10.0, 22.7))
                .is_none(),
            "27 点は外"
        );
        // 平行線（特殊定規）は距離によらず、押した点を通る線
        put(&mut s, RulerKind::Parallel, (0.0, 0.0), (10.0, 0.0));
        let far = at(10.0, 50.0);
        let c = s.canvas_ruler_constraint(&view, layer, far);
        let Some(Constraint::Line { origin, direction }) = c else {
            panic!("{c:?}");
        };
        assert!((origin - DVec2::new(10.0, 50.0)).length() < 1e-6 && direction == DVec2::X);
        // 直線定規の近くでは、直線定規が先
        let near = s.canvas_ruler_constraint(&view, layer, at(10.0, 21.0));
        assert!(
            matches!(near, Some(Constraint::Line { origin, .. }) if origin == DVec2::new(0.0, 20.0))
        );
        // 特殊定規にスナップを切ると、遠くは寄せない
        s.apply(Action::Ruler(RulerAction::ToggleSnapSpecial));
        assert!(s.canvas_ruler_constraint(&view, layer, far).is_none());
    }
}
