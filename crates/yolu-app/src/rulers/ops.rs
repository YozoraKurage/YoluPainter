//! 定規を変える操作。文書を変えるものは 1 つが 1 回の取り消し（`Document::set_rulers`・`move_rulers`・`set_snap_ruler`。新しいレイヤーに作るときは
//! `batch` でレイヤーと定規を 1 回に）。描いている間と読むだけのセットでは `Action::apply` が先に断る。

use yolu_core::{CoreError, LayerId, Ruler, RulerId, RulerKind, RulerScope, RulerSpace};

use super::{Place, RulerState};
use crate::notice::{Kind, Source};
use crate::state::AppState;

/// `Action::Ruler` の中身。
#[derive(Clone, Debug, PartialEq)]
pub enum RulerAction {
    /// 定規を選ぶ（画面だけ）。
    Select(Option<(LayerId, RulerId)>),
    /// 「定規にスナップ」「特殊定規にスナップ」の入り切り（画面だけ）。
    ToggleSnapRuler,
    ToggleSnapSpecial,
    /// 新しい定規を作る（ID は付け直す。作る先はツールの「編集レイヤーに作成」。特殊定規は印を付けて選ぶ）。
    Create(Ruler),
    /// 持ち主の一覧の同じ ID の定規を、この値に替える。`coalesce` ならドラッグ・スライダーの途中を 1 回の取り消しにまとめる。
    Replace {
        owner: LayerId,
        ruler: Ruler,
        coalesce: bool,
    },
    /// 定規を消す。
    Delete {
        owner: LayerId,
        ids: Vec<RulerId>,
    },
    /// そのレイヤーの定規の表示を全部入り切りする・表示の範囲を全部に当てる。
    SetAllVisible {
        owner: LayerId,
        visible: bool,
    },
    SetAllScope {
        owner: LayerId,
        scope: RulerScope,
    },
    /// そのレイヤーの定規を全部、別のレイヤー（グループ）へ移す。
    MoveAll {
        from: LayerId,
        to: LayerId,
    },
    /// 同じ空間で見えるほかの特殊定規の印を外して、この定規をスナップする特殊定規にする。
    MarkSnap {
        owner: LayerId,
        id: RulerId,
    },
    /// スナップする特殊定規を、見えている特殊定規の中で次へ回す（今の描いているビューの空間で）。
    SwitchSpecial,
}

impl RulerAction {
    /// 文書を変えるか（描いている間・読むだけのセットでは断る）。
    pub fn edits_document(&self) -> bool {
        !matches!(
            self,
            RulerAction::Select(_) | RulerAction::ToggleSnapRuler | RulerAction::ToggleSnapSpecial
        )
    }
}

impl AppState {
    /// 定規の操作を当てる（`Action::Ruler`）。
    pub fn ruler_action(&mut self, action: RulerAction) {
        match action {
            RulerAction::Select(selected) => {
                self.rulers.selected = selected;
                // 編集のモードでは、3D の定規を選ぶと編集のモードの選び（印・ギズモ）も同じ定規になる
                if let Some((layer, id)) = selected {
                    let is_3d = self
                        .doc
                        .layer(layer)
                        .and_then(|l| l.rulers().iter().find(|r| r.id == id))
                        .is_some_and(|r| r.space() == RulerSpace::Model);
                    if is_3d {
                        crate::objects::select_when_editing(
                            self,
                            crate::objects::Object::Ruler { layer, id },
                        );
                    }
                }
            }
            RulerAction::ToggleSnapRuler => {
                if !self.is_stroking() {
                    self.rulers.snap_ruler = !self.rulers.snap_ruler;
                }
            }
            RulerAction::ToggleSnapSpecial => {
                if !self.is_stroking() {
                    self.rulers.snap_special = !self.rulers.snap_special;
                }
            }
            edit => {
                let before = self.doc.revision();
                if let Err(e) = self.ruler_edit(edit) {
                    self.notify(Kind::of_core(&e), Source::Ruler, self.lang.core_error(&e));
                }
                if self.doc.revision() != before {
                    self.modified = true;
                }
            }
        }
    }

    fn ruler_edit(&mut self, action: RulerAction) -> Result<(), CoreError> {
        match action {
            RulerAction::Create(ruler) => self.ruler_create(ruler).map(|_| ()),
            RulerAction::Replace {
                owner,
                ruler,
                coalesce,
            } => self.ruler_put(owner, ruler, coalesce),
            RulerAction::Delete { owner, ids } => {
                let mut list = self.ruler_list(owner)?;
                list.retain(|r| !ids.contains(&r.id));
                self.doc.set_rulers(owner, list, false)?;
                if self
                    .rulers
                    .selected
                    .is_some_and(|(o, id)| o == owner && ids.contains(&id))
                {
                    self.rulers.selected = None;
                }
                Ok(())
            }
            RulerAction::SetAllVisible { owner, visible } => {
                let mut list = self.ruler_list(owner)?;
                for r in &mut list {
                    r.visible = visible;
                }
                self.doc.set_rulers(owner, list, false)
            }
            RulerAction::SetAllScope { owner, scope } => {
                let mut list = self.ruler_list(owner)?;
                for r in &mut list {
                    r.scope = scope;
                    r.visible = true;
                }
                self.doc.set_rulers(owner, list, false)
            }
            RulerAction::MoveAll { from, to } => {
                let ids: Vec<RulerId> = self.ruler_list(from)?.iter().map(|r| r.id).collect();
                self.doc.move_rulers(from, to, &ids)?;
                if let Some((owner, id)) = self.rulers.selected {
                    if owner == from {
                        self.rulers.selected = Some((to, id));
                    }
                }
                Ok(())
            }
            RulerAction::MarkSnap { owner, id } => {
                self.doc.set_snap_ruler(owner, id, self.selected_layer)?;
                self.rulers.snap_special = true;
                Ok(())
            }
            RulerAction::SwitchSpecial => self.ruler_switch_special(),
            RulerAction::Select(_)
            | RulerAction::ToggleSnapRuler
            | RulerAction::ToggleSnapSpecial => Ok(()),
        }
    }

    /// 持ち主の一覧の同じ ID の定規を `ruler` に替える（`coalesce` ならドラッグの途中を 1 回の取り消しにまとめる）。編集のモードの G/R/S・ギズモ・端の点も
    /// 続けて変える操作として通る。
    pub fn ruler_put(
        &mut self,
        owner: LayerId,
        ruler: Ruler,
        coalesce: bool,
    ) -> Result<(), CoreError> {
        let mut list = self.ruler_list(owner)?;
        let slot = list
            .iter_mut()
            .find(|r| r.id == ruler.id)
            .ok_or(CoreError::InvalidArgument("その定規がそのレイヤーに無い"))?;
        *slot = ruler;
        let before = self.doc.revision();
        self.doc.set_rulers(owner, list, coalesce)?;
        if self.doc.revision() != before {
            self.modified = true;
        }
        Ok(())
    }

    /// レイヤーの定規の一覧の写し。
    fn ruler_list(&self, owner: LayerId) -> Result<Vec<Ruler>, CoreError> {
        Ok(self
            .doc
            .layer(owner)
            .ok_or(CoreError::LayerNotFound)?
            .rulers()
            .to_vec())
    }

    /// 新しい定規を作る。作る先は、ツールの「編集レイヤーに作成」が入なら選んでいるレイヤー（グループ）、切なら選んでいるレイヤーのすぐ上の
    /// 新しいレイヤー「定規」（レイヤーと定規で 1 回の取り消し。選んでいるレイヤーはそのまま）。特殊定規は、描くレイヤーから見えるほかの特殊定規の
    /// 印を外して印を付け、「特殊定規にスナップ」を入れる。選んでいるレイヤーに作ったときだけ、作った定規を選ぶ。
    pub fn ruler_create(&mut self, mut ruler: Ruler) -> Result<RulerId, CoreError> {
        let Some(drawing) = self.selected_layer else {
            return Err(CoreError::LayerNotFound);
        };
        let on_layer = self.rulers.on_layer;
        let name = self.lang.pick("定規", "Ruler");
        let special = ruler.is_special();
        ruler.snap = false;
        let (owner, id) = self.doc.batch(|d| {
            let owner = if on_layer {
                drawing
            } else {
                d.add_layer_above(name, Some(drawing))?
            };
            ruler.id = d.new_ruler_id();
            let mut list = d
                .layer(owner)
                .ok_or(CoreError::LayerNotFound)?
                .rulers()
                .to_vec();
            list.push(ruler.clone());
            d.set_rulers(owner, list, false)?;
            if special {
                d.set_snap_ruler(owner, ruler.id, Some(drawing))?;
            }
            Ok((owner, ruler.id))
        })?;
        // 別のレイヤー（新しい「定規」）に作ったときは、選んでいるレイヤーは替えないので、その定規は選べない（つまみも出ない）
        self.rulers.selected = on_layer.then_some((owner, id));
        if special {
            self.rulers.snap_special = true;
        }
        Ok(id)
    }

    /// 「スナップする特殊定規の切り替え」の空間のビュー: ポインタが乗っているビュー、無ければ最後に描いたビュー、どちらも無ければ描ける先が 3D だけなら 3D、そうでなければ 2D。
    pub fn ruler_view_place(&self) -> Place {
        self.rulers
            .pointer_in
            .or(self.rulers.last_drew)
            .unwrap_or(if self.paints_only_in_3d() {
                Place::View3d
            } else {
                Place::Canvas
            })
    }

    /// スナップする特殊定規を、見えている特殊定規（`ruler_view_place` のビューの空間）の中で次へ回す。候補が無いときは何もしない。
    fn ruler_switch_special(&mut self) -> Result<(), CoreError> {
        let drawing = self.selected_layer;
        let space = self.ruler_view_place().space();
        let candidates: Vec<(LayerId, RulerId, bool)> = self
            .doc
            .special_ruler_candidates(drawing, space)
            .into_iter()
            .map(|r| (r.owner, r.ruler.id, r.ruler.snap))
            .collect();
        if candidates.is_empty() {
            return Ok(());
        }
        // 効いているもの（上のレイヤー、同じレイヤーなら後ろ）の、次の候補へ
        let current = candidates.iter().rposition(|c| c.2);
        let next = match current {
            Some(i) => (i + 1) % candidates.len(),
            None => 0,
        };
        let (owner, id, _) = candidates[next];
        self.doc.set_snap_ruler(owner, id, drawing)?;
        self.rulers.snap_special = true;
        Ok(())
    }
}

/// これから作る定規にツールの設定（パースの点の数、対称の線の本数と線対称）を当てる。
pub fn configure_new(state: &RulerState, mut r: Ruler) -> Ruler {
    r.two_points = state.kind == RulerKind::Perspective && state.two_points;
    if state.kind == RulerKind::Symmetry {
        r.lines = super::fit_lines(state.lines, state.line_symmetry);
        r.line_symmetry = state.line_symmetry;
    }
    r
}

/// これから作る 2D の定規（ツールの設定と、引いた 2 点）。ID は作るときに付け直す。
pub fn new_ruler(
    state: &RulerState,
    a: yolu_core::glam::DVec2,
    b: yolu_core::glam::DVec2,
) -> Ruler {
    configure_new(state, Ruler::canvas(RulerId(1), state.kind, a, b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Action;
    use yolu_core::glam::{DVec2, DVec3};

    fn state() -> AppState {
        let mut s = AppState::new(64, 64);
        s.doc.clear_history().unwrap();
        s
    }

    fn line(a: (f64, f64), b: (f64, f64)) -> Ruler {
        Ruler::canvas(
            RulerId(1),
            RulerKind::Line,
            DVec2::new(a.0, a.1),
            DVec2::new(b.0, b.1),
        )
    }

    fn symmetry() -> Ruler {
        Ruler::canvas(
            RulerId(1),
            RulerKind::Symmetry,
            DVec2::new(32.0, 32.0),
            DVec2::new(32.0, 40.0),
        )
    }

    fn rulers(s: &AppState, layer: LayerId) -> Vec<Ruler> {
        s.doc.layer(layer).unwrap().rulers().to_vec()
    }

    #[test]
    fn creating_on_the_selected_layer_is_one_step_selects_it_and_marks_a_special_ruler() {
        let mut s = state();
        let layer = s.selected_layer.unwrap();
        s.rulers.snap_special = false;
        s.apply(Action::Ruler(RulerAction::Create(symmetry())));
        let list = rulers(&s, layer);
        assert_eq!(list.len(), 1, "{}", s.message);
        assert!(list[0].snap, "特殊定規には印が付く");
        assert!(
            s.rulers.snap_special,
            "特殊定規を作ると「特殊定規にスナップ」が入る"
        );
        assert_eq!(s.rulers.selected, Some((layer, list[0].id)));
        assert_eq!(s.doc.undo_count(), 1);
        assert!(s.modified);
        // 直線定規は印が付かない
        s.apply(Action::Ruler(RulerAction::Create(line(
            (0.0, 5.0),
            (50.0, 5.0),
        ))));
        let list = rulers(&s, layer);
        assert!(!list[1].snap);
        assert_eq!(s.doc.undo_count(), 2);
        // 取り消しで 1 つずつ
        s.apply(Action::Undo);
        assert_eq!(rulers(&s, layer).len(), 1);
        s.apply(Action::Undo);
        assert!(rulers(&s, layer).is_empty());
    }

    #[test]
    fn creating_with_on_layer_off_makes_a_ruler_layer_above_in_one_step_and_keeps_the_selection() {
        let mut s = state();
        let below = s.selected_layer.unwrap();
        let top = s.doc.add_layer("Top").unwrap();
        s.selected_layer = Some(below);
        s.doc.clear_history().unwrap();
        s.rulers.on_layer = false;
        s.apply(Action::Ruler(RulerAction::Create(symmetry())));
        let layers: Vec<LayerId> = s.doc.layers().iter().map(|l| l.id()).collect();
        assert_eq!(layers.len(), 3, "{}", s.message);
        assert_eq!(layers[0], below);
        assert_eq!(layers[2], top);
        let ruler_layer = layers[1];
        assert_eq!(
            s.doc.layer(ruler_layer).unwrap().name(),
            s.lang.pick("定規", "Ruler")
        );
        assert!(rulers(&s, below).is_empty());
        assert_eq!(rulers(&s, ruler_layer).len(), 1);
        assert_eq!(
            s.selected_layer,
            Some(below),
            "選んでいるレイヤーはそのまま"
        );
        assert_eq!(s.doc.undo_count(), 1, "レイヤーと定規で 1 回");
        assert_eq!(
            s.rulers.selected, None,
            "別のレイヤーに作った定規は、選んでいるレイヤーのものでないので選ばない"
        );
        // 下のレイヤーに描くと、上に作ったレイヤーの対称が「同じグループの中」で効く
        assert!(s.canvas_symmetry().enabled());
        s.apply(Action::Undo);
        assert_eq!(s.doc.layers().len(), 2);
    }

    #[test]
    fn a_new_special_ruler_takes_the_mark_only_from_special_rulers_of_its_own_space() {
        let mut s = state();
        let layer = s.selected_layer.unwrap();
        s.apply(Action::Ruler(RulerAction::Create(symmetry())));
        let model = Ruler::model(
            RulerId(1),
            RulerKind::Symmetry,
            DVec3::ZERO,
            DVec3::X,
            DVec3::Y,
        );
        s.apply(Action::Ruler(RulerAction::Create(model)));
        let first_2d = rulers(&s, layer)[0].id;
        assert!(
            rulers(&s, layer).iter().all(|r| r.snap),
            "2D と 3D は別に 1 つずつ"
        );
        // 2D の特殊定規をもう 1 つ作ると、2D の印だけが移る
        let mut parallel = line((0.0, 0.0), (10.0, 0.0));
        parallel.kind = RulerKind::Parallel;
        s.apply(Action::Ruler(RulerAction::Create(parallel)));
        let list = rulers(&s, layer);
        assert!(!list.iter().find(|r| r.id == first_2d).unwrap().snap);
        assert!(list.iter().filter(|r| r.snap).count() == 2);
        assert_eq!(s.doc.undo_count(), 3, "印の付け替えも作る 1 回の中");
    }

    #[test]
    fn switching_the_special_ruler_steps_through_the_visible_candidates_of_the_view_space() {
        let mut s = state();
        let layer = s.selected_layer.unwrap();
        for kind in [
            RulerKind::Parallel,
            RulerKind::Concentric,
            RulerKind::Symmetry,
        ] {
            let mut r = line((0.0, 0.0), (10.0, 0.0));
            r.kind = kind;
            if kind == RulerKind::Symmetry {
                r = symmetry();
            }
            s.apply(Action::Ruler(RulerAction::Create(r)));
        }
        let marked = |s: &AppState| -> RulerKind {
            rulers(s, layer).into_iter().find(|r| r.snap).unwrap().kind
        };
        assert_eq!(marked(&s), RulerKind::Symmetry, "最後に作った物");
        let steps = s.doc.undo_count();
        s.apply(Action::Ruler(RulerAction::SwitchSpecial));
        assert_eq!(marked(&s), RulerKind::Parallel, "一覧の次（回って先頭）");
        s.apply(Action::Ruler(RulerAction::SwitchSpecial));
        assert_eq!(marked(&s), RulerKind::Concentric);
        s.apply(Action::Ruler(RulerAction::SwitchSpecial));
        assert_eq!(marked(&s), RulerKind::Symmetry);
        assert_eq!(s.doc.undo_count(), steps + 3, "1 回ずつ取り消せる");
        // 印が 1 つも無ければ、先頭から
        let list: Vec<Ruler> = rulers(&s, layer)
            .into_iter()
            .map(|mut r| {
                r.snap = false;
                r
            })
            .collect();
        s.doc.set_rulers(layer, list, false).unwrap();
        s.apply(Action::Ruler(RulerAction::SwitchSpecial));
        assert_eq!(marked(&s), RulerKind::Parallel);
    }

    #[test]
    fn edits_move_and_delete_are_each_one_step_and_are_refused_while_stroking_or_read_only() {
        let mut s = state();
        let a = s.selected_layer.unwrap();
        let b = s.doc.add_layer("B").unwrap();
        s.selected_layer = Some(a);
        s.doc.clear_history().unwrap();
        s.apply(Action::Ruler(RulerAction::Create(line(
            (0.0, 5.0),
            (50.0, 5.0),
        ))));
        s.apply(Action::Ruler(RulerAction::Create(symmetry())));
        let ids: Vec<RulerId> = rulers(&s, a).iter().map(|r| r.id).collect();
        let steps = s.doc.undo_count();
        // 値を替える（ドラッグ・スライダーをまとめる）
        let mut moved = rulers(&s, a)[0].clone();
        moved.visible = false;
        for _ in 0..3 {
            s.apply(Action::Ruler(RulerAction::Replace {
                owner: a,
                ruler: moved.clone(),
                coalesce: true,
            }));
        }
        assert_eq!(s.doc.undo_count(), steps + 1, "まとめて 1 回");
        s.doc.end_coalescing();
        // 全部を別のレイヤーへ
        s.apply(Action::Ruler(RulerAction::MoveAll { from: a, to: b }));
        assert!(rulers(&s, a).is_empty());
        assert_eq!(rulers(&s, b).iter().map(|r| r.id).collect::<Vec<_>>(), ids);
        assert_eq!(s.doc.undo_count(), steps + 2);
        // 消す
        s.apply(Action::Ruler(RulerAction::Delete {
            owner: b,
            ids: vec![ids[0]],
        }));
        assert_eq!(rulers(&s, b).len(), 1);
        assert_eq!(s.doc.undo_count(), steps + 3);
        // 描いている間は断る（文書を変えない）
        let stroke = s.begin_canvas_stroke(a, false, None).unwrap();
        s.apply(Action::Ruler(RulerAction::Delete {
            owner: b,
            ids: vec![ids[1]],
        }));
        assert_eq!(rulers(&s, b).len(), 1);
        assert!(s.message.contains("描いている間"), "{}", s.message);
        s.doc.cancel_stroke(stroke);
        // 読むだけのセットでも断る
        s.sets.get_mut(0).unwrap().read_only = Some("テスト".into());
        s.apply(Action::Ruler(RulerAction::Delete {
            owner: b,
            ids: vec![ids[1]],
        }));
        assert_eq!(rulers(&s, b).len(), 1);
        assert!(s.message.contains("読むだけ"), "{}", s.message);
        // 画面だけの操作（スナップの入り切り・選び）は、読むだけでも効く
        s.apply(Action::Ruler(RulerAction::ToggleSnapRuler));
        assert!(!s.rulers.snap_ruler);
        // 取り消しで 1 つずつ戻る
        s.sets.get_mut(0).unwrap().read_only = None;
        for _ in 0..3 {
            s.apply(Action::Undo);
        }
        assert_eq!(rulers(&s, a).len(), 2);
    }

    #[test]
    fn the_overflow_of_a_move_is_refused_with_the_core_reason_and_changes_nothing() {
        let mut s = state();
        let a = s.selected_layer.unwrap();
        let b = s.doc.add_layer("B").unwrap();
        let many: Vec<Ruler> = (0..62)
            .map(|i| {
                let mut r = line((0.0, i as f64), (10.0, i as f64));
                r.id = s.doc.new_ruler_id();
                r
            })
            .collect();
        s.doc.set_rulers(b, many, false).unwrap();
        let few: Vec<Ruler> = (0..3)
            .map(|i| {
                let mut r = line((0.0, i as f64), (10.0, i as f64));
                r.id = s.doc.new_ruler_id();
                r
            })
            .collect();
        s.doc.set_rulers(a, few, false).unwrap();
        let steps = s.doc.undo_count();
        s.apply(Action::Ruler(RulerAction::MoveAll { from: a, to: b }));
        assert_eq!(rulers(&s, a).len(), 3);
        assert_eq!(rulers(&s, b).len(), 62);
        assert_eq!(s.doc.undo_count(), steps);
        assert!(s.message.contains("64"), "{}", s.message);
    }

    #[test]
    fn document_editing_ruler_actions_are_refused_while_a_handle_is_being_dragged() {
        use crate::rulers::{Grab, Handle, RulerDrag};
        use crate::state::StrokeSource;
        let mut s = state();
        let layer = s.selected_layer.unwrap();
        s.apply(Action::Ruler(RulerAction::Create(symmetry())));
        let mut parallel = line((0.0, 0.0), (10.0, 0.0));
        parallel.kind = RulerKind::Parallel;
        s.apply(Action::Ruler(RulerAction::Create(parallel)));
        let before = rulers(&s, layer);
        let (steps, revision) = (s.doc.undo_count(), s.doc.revision());
        s.rulers.drag = Some(RulerDrag {
            source: StrokeSource::Mouse,
            start: DVec2::ZERO,
            current: DVec2::new(5.0, 5.0),
            shift: false,
            grab: Some(Grab {
                owner: layer,
                original: before[0].clone(),
                handle: Handle::A,
            }),
        });
        assert!(s.is_stroking());
        for op in [
            RulerAction::SwitchSpecial,
            RulerAction::SetAllVisible {
                owner: layer,
                visible: false,
            },
            RulerAction::Delete {
                owner: layer,
                ids: vec![before[0].id],
            },
            RulerAction::Create(line((0.0, 3.0), (9.0, 3.0))),
        ] {
            s.message.clear();
            s.apply(Action::Ruler(op.clone()));
            assert!(s.message.contains("描いている間"), "{op:?}: {}", s.message);
            assert_eq!(rulers(&s, layer), before, "{op:?}");
            assert_eq!(s.doc.revision(), revision, "{op:?}");
            assert_eq!(s.doc.undo_count(), steps, "{op:?}");
        }
        // 離したあと（ドラッグが無い）は通る
        s.rulers.drag = None;
        s.apply(Action::Ruler(RulerAction::SwitchSpecial));
        assert_eq!(s.doc.undo_count(), steps + 1);
    }

    #[test]
    fn delete_takes_the_selected_ruler_2d_or_3d_and_a_foreign_selection_is_dropped() {
        let mut s = state();
        let a = s.selected_layer.unwrap();
        let b = s.doc.add_layer("B").unwrap();
        s.selected_layer = Some(a);
        assert!(!s.has_ruler(), "選んだ定規が無ければ消せない");
        s.apply(Action::Ruler(RulerAction::Create(line(
            (0.0, 5.0),
            (50.0, 5.0),
        ))));
        assert!(s.selected_ruler().is_some());
        assert!(s.has_ruler());
        s.delete_rulers();
        assert!(rulers(&s, a).is_empty());
        assert!(!s.has_ruler());
        // 3D の定規も、選んでいれば同じ口で消せる
        let model = Ruler::model(RulerId(1), RulerKind::Line, DVec3::ZERO, DVec3::X, DVec3::Y);
        s.apply(Action::Ruler(RulerAction::Create(model)));
        assert!(s.has_ruler());
        s.delete_rulers();
        assert!(rulers(&s, a).is_empty());
        // 別のレイヤーを選んだら、前のレイヤーの定規を指す選びは外れ、削除は何もしない（前のレイヤーの定規は残る）
        s.apply(Action::Ruler(RulerAction::Create(line(
            (0.0, 6.0),
            (50.0, 6.0),
        ))));
        assert_eq!(s.rulers.selected.map(|(o, _)| o), Some(a));
        s.selected_layer = Some(b);
        s.apply(Action::Ruler(RulerAction::ToggleSnapRuler));
        assert_eq!(
            s.rulers.selected, None,
            "別のレイヤーの定規を指す選びは外す"
        );
        s.delete_rulers();
        assert_eq!(rulers(&s, a).len(), 1);
        assert!(!s.has_ruler());
    }
}
