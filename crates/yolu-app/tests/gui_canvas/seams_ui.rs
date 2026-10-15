//! レイヤーのフィルターが UV の継ぎ目をまたぐかの設定（効果の欄の切り替え）: モデルがあり、近傍の段を選んだときだけ出る・押すと文書の設定が
//! 変わる（1 回の Undo）・日英の絵。継ぎ目をまたぐ評価そのものは yolu-core の `tests/effects/uv_seam_filters.rs`。
use crate::common;
use crate::common::drive::Drive;

use common::*;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::fx::{FilterKind, FxOp};
use yolu_app::lang::Lang;
use yolu_app::m2::Edit;
use yolu_app::state::Action;
use yolu_app::YoluApp;
use yolu_core::FilterTarget;

fn apply(h: &mut Harness<'_, YoluApp>, action: Action) {
    h.act(action);
    h.run();
}

/// 塗りつぶしレイヤーにぼかしを足して選ぶ（`kind` の段）。
fn selected_filter(h: &mut Harness<'_, YoluApp>, kind: FilterKind) {
    apply(h, Action::M2(Edit::NewFill));
    let layer = h.selected_layer().unwrap();
    apply(
        h,
        Action::Fx(FxOp::AddFilter {
            target: FilterTarget::Content,
            kind,
        }),
    );
    let id = h.doc().filters_of(layer, FilterTarget::Content).unwrap()[0].id();
    apply(h, Action::Fx(FxOp::SelectFilter { layer, id }));
}

fn label(lang: Lang) -> &'static str {
    lang.pick("UV の継ぎ目をまたぐ", "Across UV seams")
}

#[test]
fn the_seam_switch_needs_a_model_and_turns_the_document_setting() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 1000.0, 128);
        h.app_mut().set_language(lang);
        selected_filter(&mut h, FilterKind::Blur);
        // モデルが無ければ出ない（2D だけの文書は今のまま）
        assert_eq!(h.query_all_by_label(label(lang)).count(), 0);
        h.view3d_mut().load_demo();
        h.run();
        assert!(h.doc().seams_active());
        let at = rect_of(&h, label(lang), |_| true);
        // 英語の絵は撮らない（行の並びは文字の長さで変わらない）
        if lang == Lang::Ja {
            let rect = crate::group_rect(&h, yolu_app::Tab::Properties, None);
            common::shot::region(&mut h, "fx_seams_switch_ja", rect);
        }
        let count = h.doc().undo_count();
        click(&mut h, center(at));
        assert!(!h.doc().filter_seams());
        assert!(!h.doc().seams_active());
        assert_eq!(h.doc().undo_count(), count + 1);
        // 取り消すと入に戻る
        apply(&mut h, Action::Undo);
        assert!(h.doc().filter_seams());
    }
}

#[test]
fn point_filters_do_not_show_the_seam_switch() {
    let mut h = app(1280.0, 1000.0, 128);
    h.view3d_mut().load_demo();
    selected_filter(&mut h, FilterKind::Invert);
    assert_eq!(h.query_all_by_label(label(Lang::Ja)).count(), 0);
}
