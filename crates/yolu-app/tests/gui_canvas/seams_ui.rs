//! レイヤーのフィルターが UV の継ぎ目をまたぐかの設定（効果の欄の切り替え）: モデルがあり、近傍の段を選んだときだけ出る・押すと文書の設定が
//! 変わる（1 回の Undo）・日英の絵。継ぎ目をまたぐ評価そのものは yolu-core の `tests/effects/uv_seam_filters.rs`。
use crate::common;

use common::*;
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, SnapshotResults};
use yolu_app::fx::{FilterKind, FxOp};
use yolu_app::lang::Lang;
use yolu_app::m2::Edit;
use yolu_app::state::Action;
use yolu_app::YoluApp;
use yolu_core::FilterTarget;

fn apply(h: &mut Harness<'_, YoluApp>, action: Action) {
    h.state_mut().state.apply(action);
    h.run();
}

/// 塗りつぶしレイヤーにぼかしを足して選ぶ（`kind` の段）。
fn selected_filter(h: &mut Harness<'_, YoluApp>, kind: FilterKind) {
    apply(h, Action::M2(Edit::NewFill));
    let layer = h.state().state.selected_layer.unwrap();
    apply(
        h,
        Action::Fx(FxOp::AddFilter {
            target: FilterTarget::Content,
            kind,
        }),
    );
    let id = h
        .state()
        .state
        .doc
        .filters_of(layer, FilterTarget::Content)
        .unwrap()[0]
        .id();
    apply(h, Action::Fx(FxOp::SelectFilter { layer, id }));
}

fn label(lang: Lang) -> &'static str {
    lang.pick("UV の継ぎ目をまたぐ", "Across UV seams")
}

#[test]
fn the_seam_switch_needs_a_model_and_turns_the_document_setting() {
    let mut results = SnapshotResults::new();
    for lang in Lang::ALL {
        let mut h = app(1280.0, 1000.0, 128);
        h.state_mut().state.set_language(lang);
        selected_filter(&mut h, FilterKind::Blur);
        // モデルが無ければ出ない（2D だけの文書は今のまま）
        assert_eq!(h.query_all_by_label(label(lang)).count(), 0);
        h.state_mut().state.view3d.load_demo();
        h.run();
        assert!(h.state().state.doc.seams_active());
        let at = rect_of(&h, label(lang), |_| true);
        h.snapshot(format!("fx_seams_switch_{}", lang.pick("ja", "en")));
        results.extend_harness(&mut h);
        let count = h.state().state.doc.undo_count();
        click(&mut h, center(at));
        assert!(!h.state().state.doc.filter_seams());
        assert!(!h.state().state.doc.seams_active());
        assert_eq!(h.state().state.doc.undo_count(), count + 1);
        // 取り消すと入に戻る
        apply(&mut h, Action::Undo);
        assert!(h.state().state.doc.filter_seams());
    }
}

#[test]
fn point_filters_do_not_show_the_seam_switch() {
    let mut h = app(1280.0, 1000.0, 128);
    h.state_mut().state.view3d.load_demo();
    selected_filter(&mut h, FilterKind::Invert);
    assert_eq!(h.query_all_by_label(label(Lang::Ja)).count(), 0);
}
