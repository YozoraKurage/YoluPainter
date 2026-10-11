//! 定規（レイヤーとグループに付く、描くときの寄せ先と対称）: 取り消しの単位、表示の範囲、目を閉じた持ち主、効く特殊定規の決め（2D・3D 別）、
//! レイヤーの操作（削除・複製・並べ替え・結合・サイズ変更・スマートマテリアル）との関係、値の確かめと数の上限。

use yolu_core::glam::{DVec2, DVec3};
use yolu_core::smart::SmartPlacement;
use yolu_core::{
    BrushSettings, CanvasResampling, CoreError, Document, HistoryKind, LayerId, MergeRefusal,
    Ruler, RulerId, RulerKind, RulerPlace, RulerScope, RulerSpace, MAX_RULERS_PER_LAYER,
};

const TOLERANCE: u8 = 2;

fn canvas(d: &mut Document, kind: RulerKind, a: (f64, f64), b: (f64, f64)) -> Ruler {
    Ruler::canvas(
        d.new_ruler_id(),
        kind,
        DVec2::new(a.0, a.1),
        DVec2::new(b.0, b.1),
    )
}

fn model(d: &mut Document, kind: RulerKind) -> Ruler {
    Ruler::model(
        d.new_ruler_id(),
        kind,
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::Y,
    )
}

fn line(d: &mut Document) -> Ruler {
    canvas(d, RulerKind::Line, (10.0, 10.0), (50.0, 30.0))
}

fn rulers_of(d: &Document, layer: LayerId) -> Vec<Ruler> {
    d.layer(layer).unwrap().rulers().to_vec()
}

/// 全レイヤーの定規（下から順）。
fn state(d: &Document) -> Vec<(LayerId, Vec<Ruler>)> {
    d.layers()
        .iter()
        .map(|l| (l.id(), l.rulers().to_vec()))
        .collect()
}

fn ids(list: &[yolu_core::RulerRef<'_>]) -> Vec<RulerId> {
    list.iter().map(|r| r.ruler.id).collect()
}

/// 操作が 1 段だけ積まれ、種類が定規で、画素にも合成にも印を付けず、版は進む。戻すと前の状態、やり直すと後の状態。
fn one_step(d: &mut Document, op: impl FnOnce(&mut Document)) {
    let (steps, serial, revision) = (d.undo_count(), d.change_serial(), d.revision());
    let before = state(d);
    op(d);
    let after = state(d);
    assert_ne!(before, after, "操作が何かを変えた");
    assert_eq!(d.undo_count(), steps + 1, "取り消しの段は 1 つ");
    assert_eq!(d.history().last(), Some(HistoryKind::Rulers));
    assert_eq!(d.change_serial(), serial, "画素の変化の印は付けない");
    assert!(d.revision() > revision, "版は進む");
    assert!(d.undo().unwrap());
    assert_eq!(state(d), before, "戻すと前");
    assert!(d.redo().unwrap());
    assert_eq!(state(d), after, "やり直すと後");
}

/// 試験用の木: 下から t1（一番上の段）、a（g1 の中）、c・d（g2 の中、g2 は g1 の中）、t2（一番上の段）。
struct Tree {
    d: Document,
    t1: LayerId,
    a: LayerId,
    c: LayerId,
    d_: LayerId,
    g2: LayerId,
    g1: LayerId,
    t2: LayerId,
}

fn tree() -> Tree {
    let mut d = Document::new(64, 64).unwrap();
    let t1 = d.add_layer("t1").unwrap();
    let a = d.add_layer("a").unwrap();
    let c = d.add_layer("c").unwrap();
    let d_ = d.add_layer("d").unwrap();
    let g2 = d.group_layers(&[c, d_], "g2").unwrap();
    let g1 = d.group_layers(&[a, g2], "g1").unwrap();
    let t2 = d.add_layer("t2").unwrap();
    d.clear_history().unwrap();
    Tree {
        d,
        t1,
        a,
        c,
        d_,
        g2,
        g1,
        t2,
    }
}

impl Tree {
    fn all(&self) -> [(&'static str, LayerId); 7] {
        [
            ("t1", self.t1),
            ("a", self.a),
            ("c", self.c),
            ("d", self.d_),
            ("g2", self.g2),
            ("g1", self.g1),
            ("t2", self.t2),
        ]
    }
    /// 名前の layer の祖先（近い方から）の名前。
    fn ancestors(name: &str) -> &'static [&'static str] {
        match name {
            "a" | "g2" => &["g1"],
            "c" | "d" => &["g2", "g1"],
            _ => &[],
        }
    }
    fn is_group(name: &str) -> bool {
        matches!(name, "g1" | "g2")
    }
}

#[test]
fn the_tree_is_laid_out_as_the_table_says() {
    let t = tree();
    let parent = |id| t.d.layer(id).unwrap().parent();
    assert_eq!(parent(t.t1), None);
    assert_eq!(parent(t.a), Some(t.g1));
    assert_eq!(parent(t.c), Some(t.g2));
    assert_eq!(parent(t.d_), Some(t.g2));
    assert_eq!(parent(t.g2), Some(t.g1));
    assert_eq!(parent(t.g1), None);
    assert_eq!(parent(t.t2), None);
}

// ───────── 取り消しの単位 ─────────

#[test]
fn creating_a_ruler_is_one_undo_step() {
    let mut t = tree();
    let r = line(&mut t.d);
    one_step(&mut t.d, |d| {
        d.set_rulers(t.a, vec![r.clone()], false).unwrap()
    });
    assert_eq!(rulers_of(&t.d, t.a), vec![r]);
}

#[test]
fn a_whole_drag_or_slider_move_is_one_undo_step_and_escape_drops_it() {
    let mut t = tree();
    let mut r = canvas(&mut t.d, RulerKind::Symmetry, (20.0, 20.0), (30.0, 20.0));
    t.d.set_rulers(t.a, vec![r.clone()], false).unwrap();
    t.d.end_coalescing();
    let start = state(&t.d);
    let steps = t.d.undo_count();
    // ドラッグ: 中心を何度も動かす
    for k in 1..=6 {
        r.place = RulerPlace::Canvas {
            a: DVec2::new(20.0 + 3.0 * k as f64, 20.0),
            b: DVec2::new(30.0 + 3.0 * k as f64, 20.0),
        };
        t.d.set_rulers(t.a, vec![r.clone()], true).unwrap();
    }
    t.d.end_coalescing();
    assert_eq!(t.d.undo_count(), steps + 1, "ドラッグ全体で 1 段");
    assert_eq!(t.d.history().last(), Some(HistoryKind::Rulers));
    let moved = state(&t.d);
    assert!(t.d.undo().unwrap());
    assert_eq!(state(&t.d), start, "戻すとドラッグの前");
    assert!(t.d.redo().unwrap());
    assert_eq!(state(&t.d), moved, "やり直すとドラッグの終わり");

    // スライダー（線の本数）も同じ
    r.lines = 2;
    t.d.set_rulers(t.a, vec![r.clone()], true).unwrap();
    for lines in [4u8, 6, 8, 10] {
        r.lines = lines;
        t.d.set_rulers(t.a, vec![r.clone()], true).unwrap();
    }
    t.d.end_coalescing();
    assert_eq!(rulers_of(&t.d, t.a)[0].lines, 10);
    assert_eq!(t.d.undo_count(), steps + 2);

    // Escape: まとめている途中の段を捨てる（取り消しにも残さない）
    let before_drag = state(&t.d);
    let steps = t.d.undo_count();
    for k in 1..=3 {
        r.lines = 10 - 2 * k;
        t.d.set_rulers(t.a, vec![r.clone()], true).unwrap();
    }
    assert!(t.d.cancel_coalescing().unwrap());
    assert_eq!(state(&t.d), before_drag);
    assert_eq!(t.d.undo_count(), steps, "ドラッグの段だけ消えた");
}

#[test]
fn moving_deleting_and_changing_settings_are_each_one_undo_step() {
    let mut t = tree();
    let r1 = line(&mut t.d);
    let r2 = canvas(&mut t.d, RulerKind::Parallel, (0.0, 0.0), (10.0, 5.0));
    t.d.set_rulers(t.a, vec![r1.clone(), r2.clone()], false)
        .unwrap();
    // 動かす（1 回）
    let mut moved = r1.clone();
    moved.place = RulerPlace::Canvas {
        a: DVec2::new(1.0, 2.0),
        b: DVec2::new(40.0, 41.0),
    };
    one_step(&mut t.d, |d| {
        d.set_rulers(t.a, vec![moved.clone(), r2.clone()], false)
            .unwrap()
    });
    // 設定を変える（表示・表示の範囲・本数・線対称・スナップの印）
    let mut changed = r2.clone();
    changed.visible = false;
    changed.scope = RulerScope::Selected;
    changed.snap = true;
    one_step(&mut t.d, |d| {
        d.set_rulers(t.a, vec![moved.clone(), changed.clone()], false)
            .unwrap()
    });
    // 消す（1 つ）
    one_step(&mut t.d, |d| {
        d.set_rulers(t.a, vec![changed.clone()], false).unwrap()
    });
    // 全部消す
    one_step(&mut t.d, |d| d.set_rulers(t.a, Vec::new(), false).unwrap());
}

#[test]
fn moving_rulers_to_another_layer_is_one_undo_step_for_both_layers() {
    let mut t = tree();
    let r1 = line(&mut t.d);
    let r2 = canvas(&mut t.d, RulerKind::Parallel, (0.0, 0.0), (10.0, 5.0));
    let r3 = canvas(&mut t.d, RulerKind::Concentric, (5.0, 5.0), (15.0, 5.0));
    let existing = canvas(&mut t.d, RulerKind::Perspective, (5.0, 5.0), (15.0, 9.0));
    t.d.set_rulers(t.a, vec![r1.clone(), r2.clone(), r3.clone()], false)
        .unwrap();
    t.d.set_rulers(t.g1, vec![existing.clone()], false).unwrap();
    one_step(&mut t.d, |d| {
        d.move_rulers(t.a, t.g1, &[r3.id, r1.id]).unwrap()
    });
    // 移した定規は元の並びのまま、移し先の後ろへ
    assert_eq!(rulers_of(&t.d, t.a), vec![r2.clone()]);
    assert_eq!(
        rulers_of(&t.d, t.g1),
        vec![existing, r1.clone(), r3.clone()]
    );
    // 無い ID・同じレイヤー・無いレイヤーは何も変えずに断る
    let steps = t.d.undo_count();
    let snapshot = state(&t.d);
    assert!(t.d.move_rulers(t.a, t.g1, &[r1.id]).is_err());
    assert!(t.d.move_rulers(t.a, t.a, &[r2.id]).is_err());
    assert!(t.d.move_rulers(t.a, LayerId(5), &[r2.id]).is_err());
    assert_eq!(t.d.undo_count(), steps);
    assert_eq!(state(&t.d), snapshot);
}

#[test]
fn toggling_every_ruler_of_a_layer_is_one_undo_step() {
    let mut t = tree();
    let r1 = line(&mut t.d);
    let r2 = canvas(&mut t.d, RulerKind::Parallel, (0.0, 0.0), (10.0, 5.0));
    t.d.set_rulers(t.a, vec![r1, r2], false).unwrap();
    // Shift＋クリック: そのレイヤーの定規の表示を全部入り切り
    one_step(&mut t.d, |d| {
        let mut list = rulers_of(d, t.a);
        list.iter_mut().for_each(|r| r.visible = false);
        d.set_rulers(t.a, list, false).unwrap();
    });
}

#[test]
fn setting_the_same_list_adds_no_step_and_snap_marking_is_one_step() {
    let mut t = tree();
    let r = canvas(&mut t.d, RulerKind::Parallel, (0.0, 0.0), (10.0, 5.0));
    t.d.set_rulers(t.a, vec![r.clone()], false).unwrap();
    let steps = t.d.undo_count();
    t.d.set_rulers(t.a, vec![r.clone()], false).unwrap();
    assert_eq!(t.d.undo_count(), steps, "同じ一覧は段にしない");
    let p = canvas(&mut t.d, RulerKind::Perspective, (0.0, 0.0), (10.0, 5.0));
    let mut marked = r.clone();
    marked.snap = true;
    t.d.set_rulers(t.a, vec![marked, p.clone()], false).unwrap();
    // 印を付ける（ほかの特殊定規の印を外す）は 1 回
    one_step(&mut t.d, |d| {
        d.set_snap_ruler(t.a, p.id, Some(t.a)).unwrap()
    });
    let list = rulers_of(&t.d, t.a);
    assert!(!list[0].snap && list[1].snap);
    let steps = t.d.undo_count();
    t.d.set_snap_ruler(t.a, p.id, Some(t.a)).unwrap();
    assert_eq!(t.d.undo_count(), steps, "すでに印があれば何もしない");
}

// ───────── 表示の範囲 ─────────

#[test]
fn the_scope_table_holds_for_every_owner_scope_and_drawing_layer() {
    let mut t = tree();
    for scope in [RulerScope::All, RulerScope::Group, RulerScope::Selected] {
        for (owner_name, owner) in t.all() {
            let mut r = line(&mut t.d);
            r.scope = scope;
            t.d.set_rulers(owner, vec![r.clone()], false).unwrap();
            for drawing in std::iter::once(None).chain(t.all().into_iter().map(Some)) {
                let drawing_name = drawing.map(|e| e.0);
                let drawing_id = drawing.map(|e| e.1);
                let inside =
                    |group: &str| drawing_name.is_some_and(|n| Tree::ancestors(n).contains(&group));
                let cluster: Option<&str> = if Tree::is_group(owner_name) {
                    Some(owner_name)
                } else {
                    Tree::ancestors(owner_name).first().copied()
                };
                let expected = match scope {
                    RulerScope::All => true,
                    RulerScope::Group => {
                        drawing_name.is_some()
                            && (drawing_name == Some(owner_name) || cluster.is_none_or(inside))
                    }
                    RulerScope::Selected => {
                        drawing_name.is_some()
                            && (drawing_name == Some(owner_name)
                                || Tree::is_group(owner_name) && inside(owner_name))
                    }
                };
                let seen = t.d.rulers_seen_from(drawing_id);
                assert_eq!(
                    ids(&seen).contains(&r.id),
                    expected,
                    "owner {owner_name}, scope {scope:?}, drawing {drawing_name:?}"
                );
                if expected {
                    assert_eq!(
                        seen.iter().find(|s| s.ruler.id == r.id).unwrap().owner,
                        owner
                    );
                }
            }
            t.d.set_rulers(owner, Vec::new(), false).unwrap();
        }
    }
}

#[test]
fn group_scope_reaches_every_depth_and_a_top_level_owner_reaches_the_whole_document() {
    let mut t = tree();
    // g1 の定規（既定の範囲 = 同じグループの中）は、入れ子の下の c・d・g2 の中にも効く（Q2）
    let r = line(&mut t.d);
    t.d.set_rulers(t.g1, vec![r.clone()], false).unwrap();
    for inside in [t.a, t.c, t.d_] {
        assert_eq!(ids(&t.d.rulers_seen_from(Some(inside))), vec![r.id]);
    }
    for outside in [t.t1, t.t2] {
        assert!(t.d.rulers_seen_from(Some(outside)).is_empty());
    }
    // 一番上のレイヤー t1 の定規は文書の全部に効く（グループの中のレイヤーにも）
    t.d.set_rulers(t.g1, Vec::new(), false).unwrap();
    t.d.set_rulers(t.t1, vec![r.clone()], false).unwrap();
    for any in [t.t1, t.a, t.c, t.d_, t.t2] {
        assert_eq!(ids(&t.d.rulers_seen_from(Some(any))), vec![r.id]);
    }
    // 描くレイヤーが無いときは「すべてのレイヤー」の定規だけ
    assert!(t.d.rulers_seen_from(None).is_empty());
    let mut all = r.clone();
    all.scope = RulerScope::All;
    t.d.set_rulers(t.t1, vec![all], false).unwrap();
    assert_eq!(ids(&t.d.rulers_seen_from(None)), vec![r.id]);
    // g2 の定規は g2 の中（c・d）だけ。a には効かない
    t.d.set_rulers(t.t1, Vec::new(), false).unwrap();
    t.d.set_rulers(t.g2, vec![r.clone()], false).unwrap();
    assert!(!t.d.rulers_seen_from(Some(t.c)).is_empty());
    assert!(t.d.rulers_seen_from(Some(t.a)).is_empty());
}

#[test]
fn a_hidden_ruler_is_seen_in_no_scope() {
    let mut t = tree();
    for scope in [RulerScope::All, RulerScope::Group, RulerScope::Selected] {
        let mut r = line(&mut t.d);
        r.scope = scope;
        r.visible = false;
        t.d.set_rulers(t.a, vec![r], false).unwrap();
        for (_, id) in t.all() {
            assert!(t.d.rulers_seen_from(Some(id)).is_empty(), "{scope:?}");
        }
        assert!(t.d.rulers_seen_from(None).is_empty());
    }
}

#[test]
fn a_closed_eye_on_the_owner_or_its_parent_hides_the_ruler() {
    let mut t = tree();
    let mut r = line(&mut t.d);
    r.scope = RulerScope::All;
    t.d.set_rulers(t.c, vec![r.clone()], false).unwrap();
    assert_eq!(ids(&t.d.rulers_seen_from(Some(t.t2))), vec![r.id]);
    // 持ち主の目を閉じる
    t.d.set_layer_visible(t.c, false).unwrap();
    assert!(t.d.rulers_seen_from(Some(t.t2)).is_empty());
    assert!(t.d.rulers_seen_from(None).is_empty());
    t.d.set_layer_visible(t.c, true).unwrap();
    assert_eq!(ids(&t.d.rulers_seen_from(Some(t.t2))), vec![r.id]);
    // 親（g2）・祖先（g1）の目を閉じても隠れる
    for group in [t.g2, t.g1] {
        t.d.set_layer_visible(group, false).unwrap();
        assert!(t.d.rulers_seen_from(Some(t.t2)).is_empty(), "{group:?}");
        t.d.set_layer_visible(group, true).unwrap();
    }
    // 兄弟（d）や描くレイヤーの目は関係ない
    t.d.set_layer_visible(t.d_, false).unwrap();
    t.d.set_layer_visible(t.t2, false).unwrap();
    assert_eq!(ids(&t.d.rulers_seen_from(Some(t.t2))), vec![r.id]);
    // 目を閉じた持ち主の定規は、特殊定規の取り合いにも出ない
    let mut s = canvas(&mut t.d, RulerKind::Parallel, (0.0, 0.0), (10.0, 0.0));
    s.scope = RulerScope::All;
    s.snap = true;
    t.d.set_rulers(t.c, vec![r, s], false).unwrap();
    assert!(t.d.snapping_special_rulers(Some(t.t2)).canvas.is_some());
    t.d.set_layer_visible(t.c, false).unwrap();
    assert!(t.d.snapping_special_rulers(Some(t.t2)).canvas.is_none());
}

#[test]
fn seen_rulers_are_listed_from_the_lowest_layer_in_list_order() {
    let mut t = tree();
    let all = |d: &mut Document, kind| {
        let mut r = canvas(d, kind, (0.0, 0.0), (10.0, 10.0));
        r.scope = RulerScope::All;
        r
    };
    let upper1 = all(&mut t.d, RulerKind::Line);
    let upper2 = all(&mut t.d, RulerKind::Parallel);
    let lower = all(&mut t.d, RulerKind::Concentric);
    t.d.set_rulers(t.t2, vec![upper1.clone(), upper2.clone()], false)
        .unwrap();
    t.d.set_rulers(t.t1, vec![lower.clone()], false).unwrap();
    assert_eq!(
        ids(&t.d.rulers_seen_from(Some(t.t2))),
        vec![lower.id, upper1.id, upper2.id]
    );
    assert_eq!(
        ids(&t.d.straight_rulers_seen_from(Some(t.t2))),
        vec![upper1.id]
    );
}

// ───────── 効く特殊定規の決め ─────────

#[test]
fn the_snapping_special_ruler_is_chosen_per_space_from_the_marked_candidates() {
    let mut t = tree();
    let special = |d: &mut Document, kind, snap| {
        let mut r = canvas(d, kind, (0.0, 0.0), (10.0, 5.0));
        r.scope = RulerScope::All;
        r.snap = snap;
        r
    };
    let model_special = |d: &mut Document, kind, snap| {
        let mut r = model(d, kind);
        r.scope = RulerScope::All;
        r.snap = snap;
        r
    };
    // 印が無ければ効かない（勝手に選ばない）
    let unmarked = special(&mut t.d, RulerKind::Perspective, false);
    t.d.set_rulers(t.t1, vec![unmarked.clone()], false).unwrap();
    let s = t.d.snapping_special_rulers(Some(t.t2));
    assert!(s.canvas.is_none() && s.model.is_none());
    // 直線定規は印があっても特殊定規にならない（印は直線定規に付けられない）
    let mut bad = line(&mut t.d);
    bad.snap = true;
    assert!(t.d.set_rulers(t.t2, vec![bad], false).is_err());
    // 印のある物が 1 つ: それ
    let low = special(&mut t.d, RulerKind::Parallel, true);
    t.d.set_rulers(t.t1, vec![unmarked.clone(), low.clone()], false)
        .unwrap();
    assert_eq!(
        t.d.snapping_special_rulers(Some(t.t2))
            .canvas
            .unwrap()
            .ruler
            .id,
        low.id
    );
    // 同じレイヤーに複数: 一覧の後ろ
    let later = special(&mut t.d, RulerKind::Concentric, true);
    t.d.set_rulers(
        t.t1,
        vec![unmarked.clone(), low.clone(), later.clone()],
        false,
    )
    .unwrap();
    assert_eq!(
        t.d.snapping_special_rulers(Some(t.t2))
            .canvas
            .unwrap()
            .ruler
            .id,
        later.id
    );
    // 上のレイヤーが勝つ
    let high = special(&mut t.d, RulerKind::Symmetry, true);
    t.d.set_rulers(t.t2, vec![high.clone()], false).unwrap();
    let won = t.d.snapping_special_rulers(Some(t.t2)).canvas.unwrap();
    assert_eq!((won.ruler.id, won.owner), (high.id, t.t2));
    // 上のレイヤーの物が見えなければ（表示の範囲・表示）、下のレイヤーの物が残る
    let mut only_here = high.clone();
    only_here.scope = RulerScope::Selected;
    t.d.set_rulers(t.t2, vec![only_here.clone()], false)
        .unwrap();
    assert_eq!(
        t.d.snapping_special_rulers(Some(t.t1))
            .canvas
            .unwrap()
            .ruler
            .id,
        later.id,
        "t1 を描くとき t2 の「選んでいるときだけ」は見えない"
    );
    assert_eq!(
        t.d.snapping_special_rulers(Some(t.t2))
            .canvas
            .unwrap()
            .ruler
            .id,
        high.id
    );
    let mut hidden = only_here;
    hidden.visible = false;
    t.d.set_rulers(t.t2, vec![hidden], false).unwrap();
    assert_eq!(
        t.d.snapping_special_rulers(Some(t.t2))
            .canvas
            .unwrap()
            .ruler
            .id,
        later.id,
        "隠した物は取り合いに出ない"
    );

    // 3D は 2D と別に数える: 3D の特殊定規が 1 つ効き、2D の物は変わらない
    let m1 = model_special(&mut t.d, RulerKind::Symmetry, true);
    let m2 = model_special(&mut t.d, RulerKind::Perspective, true);
    t.d.set_rulers(t.t2, vec![m1.clone(), m2.clone()], false)
        .unwrap();
    let s = t.d.snapping_special_rulers(Some(t.t2));
    assert_eq!(s.canvas.unwrap().ruler.id, later.id);
    assert_eq!(s.model.unwrap().ruler.id, m2.id, "3D も後ろが勝つ");
    // 2D の対称と 3D の対称の同時（それぞれの空間で 1 つずつ）
    let candidates = |space| ids(&t.d.special_ruler_candidates(Some(t.t2), space));
    assert_eq!(candidates(RulerSpace::Model), vec![m1.id, m2.id]);
    assert_eq!(
        candidates(RulerSpace::Canvas),
        vec![unmarked.id, low.id, later.id],
        "直線定規は候補に出ない"
    );
}

#[test]
fn marking_a_snap_ruler_clears_only_the_other_visible_specials_of_the_same_space() {
    let mut t = tree();
    let mk = |d: &mut Document, kind, snap, scope| {
        let mut r = canvas(d, kind, (0.0, 0.0), (10.0, 5.0));
        r.snap = snap;
        r.scope = scope;
        r
    };
    let a_other = mk(&mut t.d, RulerKind::Parallel, true, RulerScope::Group);
    let a_target = mk(&mut t.d, RulerKind::Perspective, false, RulerScope::Group);
    let t1_all = mk(&mut t.d, RulerKind::Concentric, true, RulerScope::All);
    // t2 の「選んでいるときだけ」は a を描くとき見えない: 印はそのまま
    let t2_unseen = mk(&mut t.d, RulerKind::Symmetry, true, RulerScope::Selected);
    let mut m = model(&mut t.d, RulerKind::Symmetry);
    m.snap = true;
    m.scope = RulerScope::All;
    let mut hidden = mk(&mut t.d, RulerKind::Parallel, true, RulerScope::All);
    hidden.visible = false;
    t.d.set_rulers(t.a, vec![a_other.clone(), a_target.clone()], false)
        .unwrap();
    t.d.set_rulers(t.t1, vec![t1_all.clone(), m.clone(), hidden.clone()], false)
        .unwrap();
    t.d.set_rulers(t.t2, vec![t2_unseen.clone()], false)
        .unwrap();

    one_step(&mut t.d, |d| {
        d.set_snap_ruler(t.a, a_target.id, Some(t.a)).unwrap()
    });
    let snap_of = |d: &Document, layer, id| {
        d.layer(layer)
            .unwrap()
            .rulers()
            .iter()
            .find(|r| r.id == id)
            .unwrap()
            .snap
    };
    assert!(snap_of(&t.d, t.a, a_target.id), "対象に印");
    assert!(
        !snap_of(&t.d, t.a, a_other.id),
        "同じ空間の見える物の印は外れる"
    );
    assert!(!snap_of(&t.d, t.t1, t1_all.id), "別のレイヤーの物も");
    assert!(snap_of(&t.d, t.t2, t2_unseen.id), "見えない物はそのまま");
    assert!(snap_of(&t.d, t.t1, hidden.id), "隠した物はそのまま");
    assert!(
        snap_of(&t.d, t.t1, m.id),
        "3D の物はそのまま（2D と 3D は別に数える）"
    );
    assert_eq!(
        t.d.snapping_special_rulers(Some(t.a))
            .canvas
            .unwrap()
            .ruler
            .id,
        a_target.id
    );
    assert_eq!(
        t.d.snapping_special_rulers(Some(t.a))
            .model
            .unwrap()
            .ruler
            .id,
        m.id
    );

    // 直線定規・無い定規は断る
    let l = line(&mut t.d);
    t.d.set_rulers(t.t2, vec![t2_unseen.clone(), l.clone()], false)
        .unwrap();
    assert!(t.d.set_snap_ruler(t.t2, l.id, Some(t.t2)).is_err());
    assert!(t.d.set_snap_ruler(t.t2, RulerId(99), Some(t.t2)).is_err());
    assert!(t.d.set_snap_ruler(t.a, l.id, Some(t.t2)).is_err());
}

// ───────── レイヤーの操作との関係 ─────────

#[test]
fn deleting_a_layer_takes_its_rulers_and_undo_brings_them_back() {
    let mut t = tree();
    let r = line(&mut t.d);
    t.d.set_rulers(t.t2, vec![r.clone()], false).unwrap();
    t.d.remove_layer(t.t2).unwrap();
    assert!(t.d.layer(t.t2).is_none());
    assert!(t.d.undo().unwrap());
    assert_eq!(rulers_of(&t.d, t.t2), vec![r.clone()]);
    // グループごと（中のレイヤーの定規も）
    let s = canvas(&mut t.d, RulerKind::Parallel, (0.0, 0.0), (3.0, 3.0));
    t.d.set_rulers(t.c, vec![s.clone()], false).unwrap();
    let g_line = line(&mut t.d);
    t.d.set_rulers(t.g1, vec![g_line], false).unwrap();
    let before = state(&t.d);
    t.d.remove_layer(t.g1).unwrap();
    assert!(t.d.layer(t.c).is_none());
    assert!(t.d.undo().unwrap());
    assert_eq!(state(&t.d), before);
    // グループを解除すると、グループの定規はグループと一緒に消える（取り消しで戻る）
    t.d.ungroup(t.g1).unwrap();
    assert!(t.d.layer(t.g1).is_none());
    assert_eq!(rulers_of(&t.d, t.c), vec![s], "中のレイヤーの定規は残る");
    assert!(t.d.undo().unwrap());
    assert_eq!(state(&t.d), before);
}

#[test]
fn reordering_and_regrouping_keep_the_rulers_on_their_layers() {
    let mut t = tree();
    let r = line(&mut t.d);
    let s = canvas(&mut t.d, RulerKind::Parallel, (0.0, 0.0), (3.0, 3.0));
    t.d.set_rulers(t.t1, vec![r.clone()], false).unwrap();
    t.d.set_rulers(t.g2, vec![s.clone()], false).unwrap();
    // 並べ替え・グループへの出し入れ
    t.d.move_layer_to(t.t1, Some(t.g1), 0).unwrap();
    t.d.move_layer_to(t.g2, None, 0).unwrap();
    assert_eq!(rulers_of(&t.d, t.t1), vec![r]);
    assert_eq!(rulers_of(&t.d, t.g2), vec![s]);
    // 「同じグループの中」の意味は動いた先で決まる: t1 は g1 の中に入ったので、外の t2 には効かない
    assert!(t
        .d
        .rulers_seen_from(Some(t.t2))
        .iter()
        .all(|r| r.owner != t.t1));
    assert!(t
        .d
        .rulers_seen_from(Some(t.a))
        .iter()
        .any(|r| r.owner == t.t1));
}

#[test]
fn duplicating_a_layer_copies_its_rulers_with_new_ids() {
    let mut t = tree();
    let mut r = canvas(&mut t.d, RulerKind::Symmetry, (20.0, 20.0), (30.0, 20.0));
    r.lines = 6;
    r.snap = true;
    r.scope = RulerScope::All;
    let s = line(&mut t.d);
    t.d.set_rulers(t.t1, vec![r.clone(), s.clone()], false)
        .unwrap();
    let copy = t.d.duplicate_layer(t.t1, None).unwrap();
    let copied = rulers_of(&t.d, copy);
    assert_eq!(copied.len(), 2);
    for (new, old) in copied.iter().zip([&r, &s]) {
        assert_ne!(new.id, old.id, "新しい ID");
        assert_eq!(
            Ruler {
                id: old.id,
                ..new.clone()
            },
            *old,
            "ID 以外は同じ"
        );
    }
    assert_eq!(
        rulers_of(&t.d, t.t1),
        vec![r.clone(), s.clone()],
        "元は変わらない"
    );
    // 文書の中で重ならない
    let mut all: Vec<RulerId> =
        t.d.layers()
            .iter()
            .flat_map(|l| l.rulers().iter().map(|r| r.id))
            .collect();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 4);
    // 一度の取り消しで消える
    assert!(t.d.undo().unwrap());
    assert!(t.d.layer(copy).is_none());

    // グループごとの複製: 中のレイヤーの定規にも新しい ID
    let (cl, gl) = (line(&mut t.d), line(&mut t.d));
    t.d.set_rulers(t.c, vec![cl], false).unwrap();
    t.d.set_rulers(t.g1, vec![gl], false).unwrap();
    let g_copy = t.d.duplicate_layer(t.g1, None).unwrap();
    assert_eq!(rulers_of(&t.d, g_copy).len(), 1);
    let mut all: Vec<RulerId> =
        t.d.layers()
            .iter()
            .flat_map(|l| l.rulers().iter().map(|r| r.id))
            .collect();
    let n = all.len();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), n, "全部の ID が違う");
    assert_eq!(n, 2 + 2 + 2);
    // 複数選択の複製も同じ
    let copies = t.d.duplicate_layers(&[t.t1, t.t2]).unwrap();
    assert_eq!(copies.len(), 2);
    let mut all: Vec<RulerId> =
        t.d.layers()
            .iter()
            .flat_map(|l| l.rulers().iter().map(|r| r.id))
            .collect();
    let n = all.len();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), n);
}

#[test]
fn merging_moves_every_ruler_of_the_merged_layers_to_the_result() {
    // 下に結合: 上（d）と下（c）の定規が、結果のレイヤーへ（下の順、上の順）
    let mut t = tree();
    let lower = canvas(&mut t.d, RulerKind::Parallel, (0.0, 0.0), (3.0, 3.0));
    let upper = line(&mut t.d);
    let upper2 = canvas(&mut t.d, RulerKind::Perspective, (1.0, 1.0), (7.0, 2.0));
    let outside = canvas(&mut t.d, RulerKind::Concentric, (4.0, 4.0), (9.0, 4.0));
    let in_a = line(&mut t.d);
    t.d.set_rulers(t.c, vec![lower.clone()], false).unwrap();
    t.d.set_rulers(t.d_, vec![upper.clone(), upper2.clone()], false)
        .unwrap();
    t.d.set_rulers(t.t2, vec![outside.clone()], false).unwrap();
    t.d.set_rulers(t.a, vec![in_a.clone()], false).unwrap();
    let before = state(&t.d);
    let report = t.d.merge_down(t.d_, TOLERANCE).unwrap();
    assert!(t.d.layer(t.c).is_none() && t.d.layer(t.d_).is_none());
    assert_eq!(
        rulers_of(&t.d, report.result_id),
        vec![lower.clone(), upper.clone(), upper2.clone()]
    );
    assert_eq!(
        rulers_of(&t.d, t.t2),
        vec![outside.clone()],
        "ほかのレイヤーは変わらない"
    );
    assert!(t.d.undo().unwrap());
    assert_eq!(state(&t.d), before, "取り消しで元の持ち主へ戻る");

    // 表示のレイヤーの結合: 全部の定規が結果へ
    let r = t.d.merge_visible("全部", TOLERANCE).unwrap();
    let mut got = rulers_of(&t.d, r.result_id);
    got.sort_by_key(|r| r.id);
    let mut want: Vec<Ruler> = before.iter().flat_map(|(_, l)| l.clone()).collect();
    want.sort_by_key(|r| r.id);
    assert_eq!(got, want, "全部の定規が結果へ");
    assert!(t.d.undo().unwrap());
    assert_eq!(state(&t.d), before);

    // 選んだレイヤーの結合
    let r = t.d.merge_layers(&[t.c, t.d_], TOLERANCE).unwrap();
    assert_eq!(
        rulers_of(&t.d, r.result_id),
        vec![lower.clone(), upper.clone(), upper2.clone()]
    );
    assert!(t.d.undo().unwrap());
    assert_eq!(state(&t.d), before);

    // グループの結合: グループの定規と中の全部（入れ子の中のレイヤーも）が結果へ
    let gr = canvas(&mut t.d, RulerKind::Line, (3.0, 3.0), (9.0, 9.0));
    t.d.set_rulers(t.g1, vec![gr.clone()], false).unwrap();
    let r = t.d.merge_group(t.g1, TOLERANCE).unwrap();
    let merged = rulers_of(&t.d, r.result_id);
    assert_eq!(merged.len(), 5);
    for want in [&gr, &in_a, &lower, &upper, &upper2] {
        assert!(merged.contains(want));
    }
    assert_eq!(
        rulers_of(&t.d, t.t2),
        vec![outside],
        "グループの外は変わらない"
    );
    assert!(t.d.undo().unwrap());
    assert_eq!(rulers_of(&t.d, t.g1), vec![gr]);
    assert_eq!(rulers_of(&t.d, t.c), vec![lower]);
}

#[test]
fn a_merge_that_would_give_the_result_more_than_the_limit_is_refused_untouched() {
    let mut t = tree();
    for layer in [t.c, t.d_] {
        let list: Vec<Ruler> = (0..40).map(|_| line(&mut t.d)).collect();
        t.d.set_rulers(layer, list, false).unwrap();
    }
    let before = state(&t.d);
    let steps = t.d.undo_count();
    // 断る理由は、結合の前に問い合わせる口（メニュー・命令の事前の確かめ）でも分かる
    assert_eq!(
        t.d.merge_down_refusal(t.d_).unwrap(),
        Some(MergeRefusal::TooManyRulers)
    );
    let err = t.d.merge_down(t.d_, TOLERANCE).unwrap_err();
    assert_eq!(err, CoreError::MergeRefused(MergeRefusal::TooManyRulers));
    assert_eq!(state(&t.d), before);
    assert_eq!(t.d.undo_count(), steps);
    assert!(t.d.layer(t.c).is_some() && t.d.layer(t.d_).is_some());
}

/// 結合の結果がちょうど上限（64 個）になるのは通り、1 つ超えると断られる。
#[test]
fn a_merge_that_gives_the_result_exactly_the_limit_goes_through() {
    let half = MAX_RULERS_PER_LAYER / 2;
    let build = |upper: usize| {
        let mut t = tree();
        for (layer, n) in [(t.c, half), (t.d_, upper)] {
            let list: Vec<Ruler> = (0..n).map(|_| line(&mut t.d)).collect();
            t.d.set_rulers(layer, list, false).unwrap();
        }
        t
    };
    let mut t = build(half);
    t.d.merge_down(t.d_, TOLERANCE).unwrap();
    let merged: Vec<usize> = t.d.layers().iter().map(|l| l.rulers().len()).collect();
    assert_eq!(merged.iter().sum::<usize>(), MAX_RULERS_PER_LAYER);
    assert_eq!(merged.iter().max(), Some(&MAX_RULERS_PER_LAYER));

    let mut t = build(half + 1);
    let before = state(&t.d);
    assert!(t.d.merge_down(t.d_, TOLERANCE).is_err());
    assert_eq!(state(&t.d), before);
}

#[test]
fn resizing_moves_only_the_2d_points() {
    let mut t = tree();
    let two_d = canvas(&mut t.d, RulerKind::Symmetry, (10.0, 20.0), (30.0, 20.0));
    let three_d = Ruler::model(
        t.d.new_ruler_id(),
        RulerKind::Line,
        DVec3::new(1.0, 2.0, 3.0),
        DVec3::new(4.0, 5.0, 6.0),
        DVec3::Z,
    );
    t.d.set_rulers(t.t1, vec![two_d.clone(), three_d.clone()], false)
        .unwrap();
    let before = state(&t.d);
    // 画像のサイズ変更: 横 2 倍・縦 0.5 倍
    t.d.resize_image(128, 32, CanvasResampling::Nearest)
        .unwrap();
    let got = rulers_of(&t.d, t.t1);
    assert!(matches!(
        got[0].place,
        RulerPlace::Canvas { a, b } if a == DVec2::new(20.0, 10.0) && b == DVec2::new(60.0, 10.0)
    ));
    assert_eq!(got[1], three_d, "3D の定規は変えない");
    assert_eq!(
        Ruler {
            place: two_d.place,
            ..got[0].clone()
        },
        two_d,
        "点のほかは同じ"
    );
    assert!(got[0].validate().is_ok());
    assert!(t.d.undo().unwrap());
    assert_eq!(state(&t.d), before, "取り消しで元の点へ");

    // キャンバスのサイズ変更: 画素と同じだけずらす
    t.d.resize_canvas(80, 64, (7, 3)).unwrap();
    let got = rulers_of(&t.d, t.t1);
    assert!(matches!(
        got[0].place,
        RulerPlace::Canvas { a, b } if a == DVec2::new(17.0, 23.0) && b == DVec2::new(37.0, 23.0)
    ));
    assert_eq!(got[1], three_d);
    assert!(t.d.undo().unwrap());
    assert_eq!(state(&t.d), before);
}

#[test]
fn resizing_keeps_every_ruler_valid_even_when_two_points_collapse() {
    let mut t = tree();
    // 2 点が近い定規を縮小すると 0.01 画素を割る: 向きを保ち、最小の間隔まで離して残す（確かめに通らない値を残さない）
    let near = canvas(&mut t.d, RulerKind::Line, (30.0, 30.0), (30.5, 30.0));
    t.d.set_rulers(t.t1, vec![near], false).unwrap();
    let report = t.d.resize_image(1, 1, CanvasResampling::Area).unwrap();
    let got = rulers_of(&t.d, t.t1);
    assert!(got[0].validate().is_ok());
    assert!(
        report.notes.iter().any(|n| n.contains("定規")),
        "{:?}",
        report.notes
    );
    // 編集を続けられる（一覧の全部が確かめに通る）
    t.d.set_rulers(t.t1, got, false).unwrap();
}

#[test]
fn resizing_past_a_corner_keeps_every_ruler_valid_and_says_so_once_per_layer() {
    let mut t = tree();
    // 斜めの角の近く（拡大で範囲の外へ出て、寄せると 2 点が重なる）
    let corner = canvas(&mut t.d, RulerKind::Line, (9e6, 9e6), (9.5e6, 8e6));
    let other = canvas(&mut t.d, RulerKind::Parallel, (-9e6, 9e6), (-9.5e6, 9.9e6));
    let inside = canvas(&mut t.d, RulerKind::Line, (10.0, 10.0), (40.0, 20.0));
    t.d.set_rulers(t.t1, vec![corner, other, inside.clone()], false)
        .unwrap();
    let before = state(&t.d);
    let report =
        t.d.resize_image(128, 128, CanvasResampling::Nearest)
            .unwrap();
    let got = rulers_of(&t.d, t.t1);
    assert!(got.iter().all(|r| r.validate().is_ok()), "{got:?}");
    // 続けて編集できる（一覧の全部が確かめに通る）
    t.d.set_rulers(t.t1, got.clone(), false).unwrap();
    let mut edited = got.clone();
    edited[2].visible = false;
    t.d.set_rulers(t.t1, edited, false).unwrap();
    // 角の外へ出た定規は、角の内へ寄せ、2 点を最小の間隔だけ離して残す（動かす前の点へ戻すのではない）
    let RulerPlace::Canvas { a, b } = got[0].place else {
        panic!("2D");
    };
    assert_eq!(a, DVec2::splat(1e7));
    assert!(a.distance(b) >= 0.01 && a.distance(b) < 0.02, "{a:?} {b:?}");
    // 範囲の中の定規は点が 2 倍になるだけ
    assert!(matches!(
        got[2].place,
        RulerPlace::Canvas { a, b } if a == DVec2::new(20.0, 20.0) && b == DVec2::new(80.0, 40.0)
    ));
    assert_eq!(
        report.notes.iter().filter(|n| n.contains("定規")).count(),
        1,
        "レイヤーごとに 1 つ: {:?}",
        report.notes
    );
    assert!(t.d.undo().unwrap() && t.d.undo().unwrap());
    assert_eq!(state(&t.d), before, "取り消しで元の点へ");
}

#[test]
fn a_smart_material_drops_the_rulers_and_says_so() {
    let mut t = tree();
    let (l1, l2, l3) = (line(&mut t.d), line(&mut t.d), line(&mut t.d));
    t.d.set_rulers(t.t1, vec![l1], false).unwrap();
    t.d.set_rulers(t.g1, vec![l2], false).unwrap();
    t.d.set_rulers(t.c, vec![l3], false).unwrap();
    let material = t.d.capture_smart_material(&[t.t1, t.g1], "素材").unwrap();
    assert!(
        material.layers().iter().all(|l| l.rulers().is_empty()),
        "定規は外れる"
    );
    assert_eq!(
        material
            .notes()
            .iter()
            .filter(|n| n.contains("定規"))
            .count(),
        3,
        "定規のあるレイヤーごとに知らせる: {:?}",
        material.notes()
    );
    // 元の文書は変わらない
    assert_eq!(rulers_of(&t.d, t.t1).len(), 1);
    assert_eq!(rulers_of(&t.d, t.c).len(), 1);
    // 置いたレイヤーに定規は無い
    let placed =
        t.d.place_smart_material(&material, &SmartPlacement::default())
            .unwrap();
    assert!(placed.layers.iter().all(|l| rulers_of(&t.d, *l).is_empty()));
}

// ───────── 値の確かめと数の上限 ─────────

#[test]
fn the_64th_ruler_is_the_last_one_and_the_65th_is_refused_untouched() {
    let mut t = tree();
    let full: Vec<Ruler> = (0..MAX_RULERS_PER_LAYER).map(|_| line(&mut t.d)).collect();
    t.d.set_rulers(t.a, full.clone(), false).unwrap();
    let before = state(&t.d);
    let steps = t.d.undo_count();
    // 65 個目を足す
    let mut over = full.clone();
    over.push(line(&mut t.d));
    let err = t.d.set_rulers(t.a, over, false).unwrap_err();
    assert!(matches!(err, CoreError::InvalidArgument(_)));
    assert_eq!(state(&t.d), before);
    assert_eq!(t.d.undo_count(), steps);
    // 移して 64 を超える場合も断る
    let extra = line(&mut t.d);
    t.d.set_rulers(t.t1, vec![extra.clone()], false).unwrap();
    let steps = t.d.undo_count();
    let before = state(&t.d);
    assert!(t.d.move_rulers(t.t1, t.a, &[extra.id]).is_err());
    assert_eq!(t.d.undo_count(), steps);
    assert_eq!(state(&t.d), before);
    // 1 つ減らせば移せる
    let mut less = full;
    less.pop();
    t.d.set_rulers(t.a, less, false).unwrap();
    t.d.move_rulers(t.t1, t.a, &[extra.id]).unwrap();
    assert_eq!(rulers_of(&t.d, t.a).len(), MAX_RULERS_PER_LAYER);
}

#[test]
fn invalid_lists_are_refused_without_changing_anything() {
    let mut t = tree();
    let ok = line(&mut t.d);
    t.d.set_rulers(t.a, vec![ok.clone()], false).unwrap();
    let before = state(&t.d);
    let steps = t.d.undo_count();
    let revision = t.d.revision();
    let mut bad_cases: Vec<Vec<Ruler>> = Vec::new();
    let mut nan = line(&mut t.d);
    nan.place = RulerPlace::Canvas {
        a: DVec2::new(f64::NAN, 0.0),
        b: DVec2::new(1.0, 1.0),
    };
    bad_cases.push(vec![nan]);
    let mut same_ends = line(&mut t.d);
    same_ends.place = RulerPlace::Canvas {
        a: DVec2::new(5.0, 5.0),
        b: DVec2::new(5.001, 5.0),
    };
    bad_cases.push(vec![same_ends]);
    let mut lines = canvas(&mut t.d, RulerKind::Symmetry, (0.0, 0.0), (1.0, 0.0));
    lines.lines = 3; // 線対称は偶数だけ
    bad_cases.push(vec![lines]);
    let mut empty_id = line(&mut t.d);
    empty_id.id = RulerId(0);
    bad_cases.push(vec![empty_id]);
    bad_cases.push(vec![ok.clone(), ok.clone()]); // 同じ一覧の中で ID が重なる
    let mut zero_up = model(&mut t.d, RulerKind::Line);
    zero_up.place = RulerPlace::Model {
        a: DVec3::ZERO,
        b: DVec3::X,
        up: DVec3::ZERO,
    };
    bad_cases.push(vec![zero_up]);
    for (n, list) in bad_cases.into_iter().enumerate() {
        assert!(t.d.set_rulers(t.a, list, false).is_err(), "case {n} は断る");
        assert_eq!(state(&t.d), before, "case {n}");
        assert_eq!(t.d.undo_count(), steps);
        assert_eq!(t.d.revision(), revision);
    }
    // ほかのレイヤーの定規と ID が重なる
    assert!(t.d.set_rulers(t.t1, vec![ok.clone()], false).is_err());
    assert_eq!(state(&t.d), before);
    // 無いレイヤー
    assert!(t.d.set_rulers(LayerId(42), vec![], false).is_err());
}

#[test]
fn the_up_vector_of_a_3d_ruler_is_stored_as_a_unit_vector() {
    let mut t = tree();
    let r = Ruler::model(
        t.d.new_ruler_id(),
        RulerKind::Concentric,
        DVec3::ZERO,
        DVec3::new(2.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 10.0),
    );
    t.d.set_rulers(t.a, vec![r], false).unwrap();
    let got = &rulers_of(&t.d, t.a)[0];
    assert!(matches!(got.place, RulerPlace::Model { up, .. } if up == DVec3::Z));
    // 単位のものはもう一度通しても同じ値（保存して読んでも変わらない）
    let again = got.clone().normalized();
    assert_eq!(&again, got);
}

#[test]
fn rulers_are_not_edited_while_a_stroke_is_open() {
    let mut t = tree();
    let brush = BrushSettings::default();
    let stroke = t.d.begin_stroke(t.a, &brush).unwrap();
    let r = line(&mut t.d);
    assert!(matches!(
        t.d.set_rulers(t.a, vec![r], false),
        Err(CoreError::StrokeActive)
    ));
    t.d.cancel_stroke(stroke);
}

#[test]
fn new_ruler_ids_never_repeat_inside_a_document() {
    let mut d = Document::new(32, 32).unwrap();
    let mut seen = std::collections::HashSet::new();
    for _ in 0..500 {
        let id = d.new_ruler_id();
        assert_ne!(id.0, 0);
        assert!(seen.insert(id));
    }
    // 使われている ID は避ける
    let l = d.add_layer("x").unwrap();
    let r = Ruler::canvas(RulerId(7), RulerKind::Line, DVec2::ZERO, DVec2::X * 10.0);
    d.set_rulers(l, vec![r], false).unwrap();
    for _ in 0..200 {
        assert_ne!(d.new_ruler_id(), RulerId(7));
    }
}

// ───────── 対称定規の写し ─────────

/// 2D のブラシのストロークを、その対称で 1 回置いた Color の画素。
fn stroke_with(symmetry: yolu_core::CanvasSymmetry, radius: f64, at: (f64, f64)) -> Vec<u8> {
    use yolu_core::{Brush, Channel, Rgba8};
    let mut d = Document::new(64, 64).unwrap();
    let layer = d.add_layer("x").unwrap();
    let mut brush = Brush::from(BrushSettings {
        radius,
        color: Rgba8::new(200, 40, 40, 255),
        ..BrushSettings::default()
    });
    brush.symmetry = symmetry;
    let mut stroke = d.begin_brush_stroke(layer, &brush).unwrap();
    stroke
        .add_point(&mut d, at.0, at.1, 1.0, DVec2::ZERO)
        .unwrap();
    d.end_stroke(stroke).unwrap();
    d.layer(layer)
        .unwrap()
        .surface(Channel::Color)
        .unwrap()
        .canvas_bytes()
        .unwrap()
}

#[test]
fn a_2d_symmetry_ruler_paints_exactly_what_the_old_vertical_horizontal_and_both_painted() {
    use yolu_core::{CanvasSymmetry, SymmetryMode};
    let center = DVec2::new(31.5, 29.5);
    for at in [(10.5, 20.5), (50.2, 5.7), (31.5, 40.0)] {
        for (lines, angle, old) in [
            (2u8, 90.0f64, SymmetryMode::Vertical),
            (2, 0.0, SymmetryMode::Horizontal),
            (4, 0.0, SymmetryMode::Both),
        ] {
            let mut r = Ruler::canvas(
                RulerId(1),
                RulerKind::Symmetry,
                center,
                center
                    + DVec2::new(
                        angle.to_radians().cos().round(),
                        angle.to_radians().sin().round(),
                    ) * 8.0,
            );
            r.lines = lines;
            let from_ruler = r.canvas_symmetry().unwrap();
            // 今と同じ写しになる向きは、今のモードで返る（写しの並びまで同じ）
            let old_mode = old;
            let old = CanvasSymmetry::new(old, center, 2).unwrap();
            assert_eq!(from_ruler.mode, old_mode);
            assert_eq!(from_ruler, old);
            let expected = stroke_with(old, 5.0, at);
            assert_eq!(
                stroke_with(from_ruler, 5.0, at),
                expected,
                "{lines} 本 {angle}° ({at:?}) の画素は今の縦・横・両方と同じバイト"
            );
            // 線対称のままの値でも、ブラシの画素は同じ（集合が同じ）
            let lines_mode = CanvasSymmetry::lines(center, u32::from(lines), angle).unwrap();
            assert_eq!(stroke_with(lines_mode, 5.0, at), expected);
        }
    }
    // 回転対称の定規は今の放射状と同じ
    let mut r = Ruler::canvas(
        RulerId(1),
        RulerKind::Symmetry,
        center,
        center + DVec2::new(1.0, 1.0),
    );
    r.lines = 5;
    r.line_symmetry = false;
    assert_eq!(
        stroke_with(r.canvas_symmetry().unwrap(), 4.0, (45.0, 30.0)),
        stroke_with(
            CanvasSymmetry::new(SymmetryMode::Radial, center, 5).unwrap(),
            4.0,
            (45.0, 30.0)
        )
    );
}

#[test]
fn a_diagonal_mirror_ruler_paints_the_transposed_stroke() {
    // 中心を通る 45 度の鏡: 文書の画素 (x, y) は (y, x) へ写る（中心が正方形の中心のとき、画素の格子ごと入れ替わる）
    let center = DVec2::new(32.0, 32.0);
    let r = Ruler::canvas(
        RulerId(1),
        RulerKind::Symmetry,
        center,
        center + DVec2::new(5.0, 5.0),
    );
    let bytes = stroke_with(r.canvas_symmetry().unwrap(), 4.0, (12.5, 40.5));
    let alpha = |x: usize, y: usize| bytes[(y * 64 + x) * 4 + 3];
    let mut painted = 0;
    for y in 0..64 {
        for x in 0..64 {
            assert_eq!(alpha(x, y), alpha(y, x), "({x}, {y})");
            painted += usize::from(alpha(x, y) > 0);
        }
    }
    assert!(painted > 40, "元と写しの両方が塗られた");
    assert!(alpha(12, 40) > 0 && alpha(40, 12) > 0);
}

#[test]
fn transforming_a_layer_moves_its_pixels_but_not_its_rulers() {
    let mut t = tree();
    let two_d = canvas(&mut t.d, RulerKind::Line, (10.0, 10.0), (30.0, 10.0));
    t.d.set_rulers(t.t1, vec![two_d.clone()], false).unwrap();
    t.d.set_pixel(t.t1, 5, 5, yolu_core::Rgba8::new(255, 0, 0, 255))
        .unwrap();
    t.d.clear_history().unwrap();
    assert!(t
        .d
        .transform_layer(
            t.t1,
            yolu_core::Affine2D::translation(3., 0.),
            yolu_core::Resampling::Nearest,
            true
        )
        .unwrap());
    assert_eq!(
        t.d.layer(t.t1)
            .unwrap()
            .surface(yolu_core::Channel::Color)
            .unwrap()
            .pixel(8, 5)
            .unwrap()
            .a,
        255,
        "画素は動く"
    );
    assert_eq!(
        rulers_of(&t.d, t.t1),
        vec![two_d],
        "定規は文書の座標に付いていて、レイヤーの変形では動かない"
    );
}

#[test]
fn a_group_drawn_on_sees_its_own_rulers_and_all_scope_ones_but_not_those_of_its_children() {
    // グループを選んでいるとき（グループには描けないので、編集の見え方）: グループ自身の定規と「すべてのレイヤー」の定規だけ。
    // 中のレイヤーを選ぶと、グループの定規と、そのレイヤー自身の定規が見える
    let mut t = tree();
    let mut on_group = canvas(&mut t.d, RulerKind::Line, (0.0, 1.0), (9.0, 1.0));
    on_group.scope = RulerScope::Group;
    let mut on_child = canvas(&mut t.d, RulerKind::Line, (0.0, 2.0), (9.0, 2.0));
    on_child.scope = RulerScope::Group;
    let mut on_top = canvas(&mut t.d, RulerKind::Line, (0.0, 3.0), (9.0, 3.0));
    on_top.scope = RulerScope::All;
    t.d.set_rulers(t.g2, vec![on_group.clone()], false).unwrap();
    t.d.set_rulers(t.d_, vec![on_child.clone()], false).unwrap();
    t.d.set_rulers(t.t2, vec![on_top.clone()], false).unwrap();
    let sorted = |mut v: Vec<RulerId>| {
        v.sort();
        v
    };
    assert_eq!(
        sorted(ids(&t.d.rulers_seen_from(Some(t.g2)))),
        sorted(vec![on_group.id, on_top.id]),
        "グループを選んでいる: 自分の定規と「すべてのレイヤー」の定規"
    );
    assert_eq!(
        sorted(ids(&t.d.rulers_seen_from(Some(t.d_)))),
        sorted(vec![on_group.id, on_child.id, on_top.id]),
        "中のレイヤーを選んでいる: グループの定規と自分の定規も"
    );
}
