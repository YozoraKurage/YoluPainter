//! 効果の行（レイヤーの行の下）と、選んだ効果の欄・メニューの画面（egui_kittest）。行を押す・目・上へ・下へ・消す・右クリック・メニュー、
//! 日英の見た目（説明の文を置かない）、スナップショット。文書の操作そのものは `headless/effects.rs`（画面なし）。
use crate::common;

use common::*;
use egui::{epaint::Shape, pos2, PointerButton, Rect};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use egui_kittest::SnapshotResults;
use yolu_app::fx::{FilterKind, FxOp, Selected};
use yolu_app::lang::Lang;
use yolu_app::m2::{Edit, UiOp};
use yolu_app::state::{Action, PopupKind};
use yolu_app::YoluApp;
use yolu_core::generator::Kind;
use yolu_core::{AnchorPlacement, FilterTarget};

fn apply(h: &mut Harness<'_, YoluApp>, action: Action) {
    h.state_mut().state.apply(action);
    h.run();
}

fn fx(h: &mut Harness<'_, YoluApp>, op: FxOp) {
    apply(h, Action::Fx(op));
}

/// 効果の行を並べた文書: 下地のレイヤー（ぼかし・階調の反転・アンカー）と、マスクにマップを読む Generator。
fn effect_document(h: &mut Harness<'_, YoluApp>) {
    apply(h, Action::M2(Edit::NewFill));
    let layer = h.state().state.selected_layer.unwrap();
    fx(
        h,
        FxOp::AddAnchor {
            layer,
            placement: AnchorPlacement::Layer,
        },
    );
    fx(
        h,
        FxOp::AddFilter {
            target: FilterTarget::Content,
            kind: FilterKind::Blur,
        },
    );
    fx(
        h,
        FxOp::AddFilter {
            target: FilterTarget::Content,
            kind: FilterKind::Invert,
        },
    );
    apply(h, Action::M2(Edit::AddMask(layer)));
    fx(
        h,
        FxOp::AddGenerator {
            target: FilterTarget::Mask,
            kind: Kind::EdgeWear,
        },
    );
}

/// プロパティの欄の横に長いボタン（レイヤーのパネルの帯の同じ名前の小さいボタンと取り違えない）。
fn props_button(h: &Harness<'_, YoluApp>, label: &str) -> Rect {
    rect_of(h, label, |r| r.width() > 60.0)
}

fn selected(h: &Harness<'_, YoluApp>) -> Option<Selected> {
    h.state().state.fx.selected
}

#[test]
fn snapshot_effect_rows_and_the_generator_that_has_no_maps() {
    let mut results = SnapshotResults::new();
    for lang in Lang::ALL {
        let mut h = app(1280.0, 1400.0, 128);
        h.state_mut().state.set_language(lang);
        effect_document(&mut h);
        h.snapshot(format!("fx_rows_generator_{}", lang.pick("ja", "en")));
        results.extend_harness(&mut h);
    }
}

#[test]
fn snapshot_a_selected_filter_and_a_selected_anchor() {
    let mut h = app(1280.0, 1400.0, 128);
    effect_document(&mut h);
    let layer = h.state().state.selected_layer.unwrap();
    let blur = h
        .state()
        .state
        .doc
        .filters_of(layer, FilterTarget::Content)
        .unwrap()[0]
        .id();
    fx(&mut h, FxOp::SelectFilter { layer, id: blur });
    h.snapshot("fx_rows_blur_selected");
    let anchor = h
        .state()
        .state
        .doc
        .layer(layer)
        .unwrap()
        .anchor()
        .unwrap()
        .id();
    fx(&mut h, FxOp::SelectAnchor(anchor));
    h.snapshot("fx_rows_anchor_selected");
}

#[test]
fn snapshot_the_add_menu() {
    let mut h = app(1280.0, 1400.0, 128);
    effect_document(&mut h);
    fx(&mut h, FxOp::Deselect);
    h.state_mut().state.ui.property_tab = yolu_app::panels::properties::TAB_ICONS.len() - 1; // レイヤーのタブ（最後）
    h.run();
    let at = props_button(&h, "フィルターを追加").center();
    click(&mut h, at);
    assert!(matches!(
        h.state().state.popup.as_ref().map(|p| p.kind),
        Some(PopupKind::M2(yolu_app::m2_menu::Popup::AddFilter(
            FilterTarget::Content
        )))
    ));
    h.snapshot("fx_add_menu");
}

// ───────── レイヤーとマスクの対象の選び分け ─────────

/// マスクのサムネイル（一覧に 1 つだけ）。
fn mask_thumb(h: &Harness<'_, YoluApp>) -> Rect {
    let lang = h.state().state.lang;
    h.get_by_label(lang.pick(
        "レイヤーマスク（押すとマスクが対象）",
        "Layer mask (click to target the mask)",
    ))
    .rect()
}

/// マスクのあるレイヤーの、レイヤーのサムネイル（マスクと同じ行）。
fn pixels_thumb(h: &Harness<'_, YoluApp>) -> Rect {
    let lang = h.state().state.lang;
    let mask = mask_thumb(h);
    rect_of(h, lang.pick("レイヤーの画素", "Layer pixels"), |r| {
        (r.center().y - mask.center().y).abs() < 2.0 && r.right() < mask.left()
    })
}

/// レイヤーの行の名前の所（行の真ん中）。
fn name_of(h: &Harness<'_, YoluApp>, layer: yolu_app::engine::LayerId) -> egui::Pos2 {
    let name = h.state().state.doc.layer(layer).unwrap().name().to_owned();
    rect_of(h, &name, |r| r.width() > 100.0 && r.left() > rx()).center()
}

fn editing_mask(h: &Harness<'_, YoluApp>) -> bool {
    h.state().state.m2.edit_mask
}

/// 青い枠の付いたレイヤーのサムネイル（選んだ状態の「レイヤーの画素」）。
fn framed_pixels_thumbs(h: &Harness<'_, YoluApp>) -> Vec<Rect> {
    let lang = h.state().state.lang;
    h.get_all_by_label(lang.pick("レイヤーの画素", "Layer pixels"))
        .filter(|n| n.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True))
        .map(|n| n.rect())
        .collect()
}

#[test]
fn the_thumbnails_choose_the_pixels_or_the_mask_and_the_name_keeps_the_choice() {
    let mut h = app(1280.0, 1400.0, 128);
    let base = h.state().state.selected_layer.unwrap();
    effect_document(&mut h);
    let layer = h.state().state.selected_layer.unwrap();
    assert_ne!(base, layer);
    // レイヤーのサムネイル: レイヤーの画素が対象（もう一度押しても同じ）。選んでいた効果の欄は閉じる
    for _ in 0..2 {
        let at = pixels_thumb(&h).center();
        click(&mut h, at);
        assert!(!editing_mask(&h));
        assert_eq!(h.state().state.selected_layer, Some(layer));
        assert_eq!(selected(&h), None);
        assert_eq!(
            yolu_app::fx::menu::target(&h.state().state),
            FilterTarget::Content
        );
        assert_eq!(framed_pixels_thumbs(&h), vec![pixels_thumb(&h)]);
    }
    // マスクのサムネイル: マスクが対象（切り替えではないので、もう一度押してもマスクのまま）。レイヤーのサムネイルの枠は消える
    for _ in 0..2 {
        let at = mask_thumb(&h).center();
        click(&mut h, at);
        assert!(editing_mask(&h));
        assert_eq!(
            yolu_app::fx::menu::target(&h.state().state),
            FilterTarget::Mask
        );
        assert!(framed_pixels_thumbs(&h).is_empty());
    }
    // 同じレイヤーの名前を押しても対象はそのまま
    let at = name_of(&h, layer);
    click(&mut h, at);
    assert!(editing_mask(&h), "同じレイヤーの名前は今の対象のまま");
    // 別のレイヤーの名前は、そのレイヤーの画素
    let at = name_of(&h, base);
    click(&mut h, at);
    assert_eq!(h.state().state.selected_layer, Some(base));
    assert!(!editing_mask(&h));
    // マスクの無いレイヤーでも、画素が対象ならレイヤーのサムネイルに枠（選んだレイヤーの 1 つだけ）
    let framed = framed_pixels_thumbs(&h);
    assert_eq!(framed.len(), 1, "{framed:?}");
    assert!((framed[0].center().y - name_of(&h, base).y).abs() < 2.0);
    // 選んでいないレイヤーのマスクのサムネイルは、そのレイヤーを選んでマスクを対象にする
    let at = mask_thumb(&h).center();
    click(&mut h, at);
    assert_eq!(h.state().state.selected_layer, Some(layer));
    assert!(editing_mask(&h));
    // 選んでいないレイヤーの、レイヤーのサムネイルは、そのレイヤーを選んで画素を対象にする
    let at = name_of(&h, base);
    click(&mut h, at);
    let at = pixels_thumb(&h).center();
    click(&mut h, at);
    assert_eq!(h.state().state.selected_layer, Some(layer));
    assert!(!editing_mask(&h));
}

#[test]
fn the_mask_thumbnail_menu_adds_to_the_mask_and_switches_the_mask() {
    let mut h = app(1280.0, 1400.0, 128);
    effect_document(&mut h);
    let layer = h.state().state.selected_layer.unwrap();
    let at = pixels_thumb(&h).center();
    click(&mut h, at);
    assert!(!editing_mask(&h));
    let open_menu = |h: &mut Harness<'_, YoluApp>| {
        let at = mask_thumb(h).center();
        press(h, at, PointerButton::Secondary);
        h.step();
        release(h, at, PointerButton::Secondary);
        h.run();
        assert_eq!(
            h.state().state.popup.as_ref().map(|p| p.kind),
            Some(PopupKind::M2(yolu_app::m2_menu::Popup::MaskContext(layer)))
        );
    };
    open_menu(&mut h);
    let before = h
        .state()
        .state
        .doc
        .filters_of(layer, FilterTarget::Mask)
        .unwrap()
        .len();
    let content = h
        .state()
        .state
        .doc
        .filters_of(layer, FilterTarget::Content)
        .unwrap()
        .len();
    let add = popup_item(&h, "フィルターを追加");
    move_to(&h, pos2(640.0, 400.0));
    h.step();
    move_to(&h, add.center());
    h.run();
    let blur = h.get_by_label("ぼかし（ガウス）").rect();
    click(&mut h, blur.center());
    assert_eq!(
        h.state()
            .state
            .doc
            .filters_of(layer, FilterTarget::Mask)
            .unwrap()
            .len(),
        before + 1,
        "レイヤーの画素が対象でも、マスクのメニューはマスクへ足す"
    );
    assert_eq!(
        h.state()
            .state
            .doc
            .filters_of(layer, FilterTarget::Content)
            .unwrap()
            .len(),
        content
    );
    // 反転
    open_menu(&mut h);
    let item = popup_item(&h, "反転");
    click(&mut h, item.center());
    assert!(h
        .state()
        .state
        .doc
        .layer(layer)
        .unwrap()
        .mask()
        .unwrap()
        .inverted());
    // 取り消すと元へ
    apply(&mut h, Action::Undo);
    assert!(!h
        .state()
        .state
        .doc
        .layer(layer)
        .unwrap()
        .mask()
        .unwrap()
        .inverted());
}

// ───────── 効果の行は、選んだレイヤーでは対象の側だけ ─────────

/// レイヤーのサムネイルと同じ行の、そのレイヤーのマスクのサムネイル（マスクのあるレイヤーが複数あるときの取り分け）。
fn mask_thumb_of(h: &Harness<'_, YoluApp>, layer: yolu_app::engine::LayerId) -> Rect {
    let lang = h.state().state.lang;
    let y = name_of(h, layer).y;
    rect_of(
        h,
        lang.pick(
            "レイヤーマスク（押すとマスクが対象）",
            "Layer mask (click to target the mask)",
        ),
        |r| (r.center().y - y).abs() < 2.0,
    )
}

fn pixels_thumb_of(h: &Harness<'_, YoluApp>, layer: yolu_app::engine::LayerId) -> Rect {
    let lang = h.state().state.lang;
    let y = name_of(h, layer).y;
    rect_of(h, lang.pick("レイヤーの画素", "Layer pixels"), |r| {
        (r.center().y - y).abs() < 2.0
    })
}

/// 効果の行の名前（一覧に出す文字）。
fn label_of(h: &Harness<'_, YoluApp>, id: yolu_core::FilterId) -> String {
    let s = &h.state().state;
    let (_, effect, target) = s.doc.find_filter(id).expect("効果がある");
    yolu_app::fx::names::effect_label(s.lang, effect, target, s.m2.paint_channel, |c| {
        yolu_app::m2::channel_name(s.lang, &s.doc, c)
    })
}

/// その効果の行の矩形（効果の行の高さで、レイヤーのパネルの列にあるもの。出ていなければ None）。
fn row_rect_if_shown(h: &Harness<'_, YoluApp>, id: yolu_core::FilterId) -> Option<Rect> {
    let label = label_of(h, id);
    let rects: Vec<Rect> = h.query_all_by_label(&label).map(|n| n.rect()).collect();
    rects.into_iter().find(|r| {
        (r.height() - yolu_app::panels::effect_rows::EFFECT_ROW_HEIGHT).abs() < 0.5
            && r.left() > rx()
    })
}

/// その効果の行が一覧に出ているか。
fn row_shown(h: &Harness<'_, YoluApp>, id: yolu_core::FilterId) -> bool {
    row_rect_if_shown(h, id).is_some()
}

fn row_rect(h: &Harness<'_, YoluApp>, id: yolu_core::FilterId) -> Rect {
    row_rect_if_shown(h, id).unwrap_or_else(|| panic!("{} の行が出ていない", label_of(h, id)))
}

/// その効果の行の中のボタン（上へ・消すなどは、選んだ行とマウスの乗った行に出るので、行の高さで取り分ける）。
fn row_button(h: &Harness<'_, YoluApp>, id: yolu_core::FilterId, label: &str) -> Rect {
    let y = row_rect(h, id).center().y;
    rect_of(h, label, |r| (r.center().y - y).abs() < 2.0)
}

fn ids_of(
    h: &Harness<'_, YoluApp>,
    layer: yolu_app::engine::LayerId,
    target: FilterTarget,
) -> Vec<yolu_core::FilterId> {
    h.state()
        .state
        .doc
        .filters_of(layer, target)
        .unwrap()
        .iter()
        .map(|e| e.id())
        .collect()
}

/// 下のレイヤー（画素にシャープ、マスクにノイズ）と、上のレイヤー（画素にぼかし・反転とアンカー、マスクにエッジの摩耗とレベル補正）。
/// 上のレイヤーを選んでいて、マスクの効果の行を選んだ状態（マスクが対象）で返る。
fn both_stacks(
    h: &mut Harness<'_, YoluApp>,
) -> (yolu_app::engine::LayerId, yolu_app::engine::LayerId) {
    let base = h.state().state.selected_layer.unwrap();
    apply(h, Action::M2(Edit::AddMask(base)));
    fx(
        h,
        FxOp::AddFilter {
            target: FilterTarget::Mask,
            kind: FilterKind::NoiseMono,
        },
    );
    fx(
        h,
        FxOp::AddFilter {
            target: FilterTarget::Content,
            kind: FilterKind::Sharpen,
        },
    );
    effect_document(h);
    let layer = h.state().state.selected_layer.unwrap();
    fx(
        h,
        FxOp::AddFilter {
            target: FilterTarget::Mask,
            kind: FilterKind::Levels,
        },
    );
    (base, layer)
}

#[test]
fn the_effect_rows_show_only_the_target_side_of_the_selected_layer() {
    let mut h = app(1280.0, 1400.0, 128);
    let (base, layer) = both_stacks(&mut h);
    let (base_content, base_mask) = (
        ids_of(&h, base, FilterTarget::Content),
        ids_of(&h, base, FilterTarget::Mask),
    );
    let (content, mask) = (
        ids_of(&h, layer, FilterTarget::Content),
        ids_of(&h, layer, FilterTarget::Mask),
    );
    assert_eq!((content.len(), mask.len()), (2, 2));
    let shown = |h: &Harness<'_, YoluApp>, ids: &[yolu_core::FilterId]| -> Vec<bool> {
        ids.iter().map(|id| row_shown(h, *id)).collect()
    };
    // 最後に足したマスクの効果を選んだ状態: マスクが対象で、選んだ行はそのまま見えている
    assert!(editing_mask(&h));
    assert_eq!(selected(&h), Some(Selected::Filter { layer, id: mask[1] }));
    assert_eq!(shown(&h, &mask), [true, true]);
    assert_eq!(shown(&h, &content), [false, false]);
    // レイヤーの画素が対象: レイヤーの効果だけ
    let at = pixels_thumb_of(&h, layer).center();
    click(&mut h, at);
    assert!(!editing_mask(&h));
    assert_eq!(shown(&h, &content), [true, true]);
    assert_eq!(shown(&h, &mask), [false, false]);
    // 選んでいないレイヤーの下は、レイヤーの効果（マスクの効果は出ない）
    assert_eq!(shown(&h, &base_content), [true]);
    assert_eq!(shown(&h, &base_mask), [false]);
    // マスクが対象: マスクの効果だけ。ほかのレイヤーの下は変わらない
    let at = mask_thumb_of(&h, layer).center();
    click(&mut h, at);
    assert!(editing_mask(&h));
    assert_eq!(shown(&h, &mask), [true, true]);
    assert_eq!(shown(&h, &content), [false, false]);
    assert_eq!(shown(&h, &base_content), [true]);
    assert_eq!(shown(&h, &base_mask), [false]);
    // レイヤーの画素へ戻る
    let at = pixels_thumb_of(&h, layer).center();
    click(&mut h, at);
    assert_eq!(shown(&h, &content), [true, true]);
    assert_eq!(shown(&h, &mask), [false, false]);
    // レイヤーの効果の行を押すと選ぶ（レイヤーの画素が対象のまま）
    let at = row_rect(&h, content[0]).center();
    click(&mut h, at);
    assert_eq!(
        selected(&h),
        Some(Selected::Filter {
            layer,
            id: content[0]
        })
    );
    assert!(!editing_mask(&h));
    assert!(yolu_app::fx::props_visible(&h.state().state));
    assert_eq!(shown(&h, &content), [true, true]);
    // マスクへ移ってマスクの効果の行を押すと、選んだ行は残り、マスクが対象のまま（レイヤーの効果の行へ戻らない）
    let at = mask_thumb_of(&h, layer).center();
    click(&mut h, at);
    assert_eq!(selected(&h), None);
    let at = row_rect(&h, mask[0]).center();
    click(&mut h, at);
    assert_eq!(selected(&h), Some(Selected::Filter { layer, id: mask[0] }));
    assert!(editing_mask(&h));
    assert!(yolu_app::fx::props_visible(&h.state().state));
    assert_eq!(shown(&h, &mask), [true, true]);
    assert_eq!(shown(&h, &content), [false, false]);
    // 目・下へ（先に掛かる）・消す・取り消し: マスクの行でも今までどおり、そのたびにマスクが対象のまま
    let eye = row_button(&h, mask[0], "フィルターを無効にする");
    click(&mut h, eye.center());
    assert!(!h
        .state()
        .state
        .doc
        .find_filter(mask[0])
        .unwrap()
        .1
        .enabled());
    // 一番下（先に掛かる）のマスクの効果を、上へ（後から掛かる）
    let up = row_button(&h, mask[0], "上へ（後から掛かる）");
    click(&mut h, up.center());
    assert_eq!(
        ids_of(&h, layer, FilterTarget::Mask),
        vec![mask[1], mask[0]]
    );
    assert!(editing_mask(&h));
    let remove = row_button(&h, mask[0], "フィルターを削除");
    click(&mut h, remove.center());
    assert_eq!(ids_of(&h, layer, FilterTarget::Mask), vec![mask[1]]);
    assert_eq!(selected(&h), None, "消した行の選びは外れる");
    assert!(
        editing_mask(&h),
        "消してもマスクが対象のまま（一覧がレイヤーの効果へ替わらない）"
    );
    assert_eq!(shown(&h, &[mask[1]]), [true]);
    assert_eq!(shown(&h, &content), [false, false]);
    apply(&mut h, Action::Undo);
    assert_eq!(
        ids_of(&h, layer, FilterTarget::Mask),
        vec![mask[1], mask[0]]
    );
    assert!(editing_mask(&h));
    assert_eq!(shown(&h, &mask), [true, true]);
    // 別のレイヤーの名前を押すと、そのレイヤーの画素が対象。前のレイヤー（マスクの効果を選んでいた）の下はレイヤーの効果
    let at = name_of(&h, base);
    click(&mut h, at);
    assert_eq!(h.state().state.selected_layer, Some(base));
    assert!(!editing_mask(&h));
    assert_eq!(selected(&h), None);
    assert_eq!(shown(&h, &content), [true, true]);
    assert_eq!(shown(&h, &mask), [false, false]);
    assert_eq!(shown(&h, &base_content), [true]);
    assert_eq!(shown(&h, &base_mask), [false]);
    // 選んでいないレイヤーのマスクのサムネイル: そのレイヤーを選んでマスクが対象（もう一方のレイヤーはレイヤーの効果）
    let at = mask_thumb_of(&h, layer).center();
    click(&mut h, at);
    assert_eq!(h.state().state.selected_layer, Some(layer));
    assert_eq!(shown(&h, &mask), [true, true]);
    assert_eq!(shown(&h, &base_content), [true]);
    // マスクの効果の行の右クリックは、その行を選んでマスクが対象のまま
    let at = row_rect(&h, mask[1]).center();
    right_click(&mut h, at);
    assert_eq!(selected(&h), Some(Selected::Filter { layer, id: mask[1] }));
    assert!(editing_mask(&h));
}

#[test]
fn adding_to_the_mask_keeps_the_mask_as_the_target_and_the_new_row_in_view() {
    let mut h = app(1280.0, 1400.0, 128);
    let (_, layer) = both_stacks(&mut h);
    let at = pixels_thumb_of(&h, layer).center();
    click(&mut h, at);
    // マスクを対象にして足すと、マスクが対象のまま、足した行が見える
    let at = mask_thumb_of(&h, layer).center();
    click(&mut h, at);
    assert!(editing_mask(&h));
    fx(
        &mut h,
        FxOp::AddFilter {
            target: FilterTarget::Mask,
            kind: FilterKind::Threshold,
        },
    );
    let mask = ids_of(&h, layer, FilterTarget::Mask);
    assert_eq!(mask.len(), 3);
    assert!(editing_mask(&h));
    assert_eq!(selected(&h), Some(Selected::Filter { layer, id: mask[2] }));
    assert!(row_shown(&h, mask[2]));
    // レイヤーの画素が対象で足すと、レイヤーの画素が対象のまま
    let at = pixels_thumb_of(&h, layer).center();
    click(&mut h, at);
    fx(
        &mut h,
        FxOp::AddFilter {
            target: FilterTarget::Content,
            kind: FilterKind::Normalize,
        },
    );
    let content = ids_of(&h, layer, FilterTarget::Content);
    assert!(!editing_mask(&h));
    assert!(row_shown(&h, *content.last().unwrap()));
    assert!(!row_shown(&h, mask[2]));
}

#[test]
fn a_mark_on_the_mask_thumbnail_tells_that_the_mask_effects_are_hidden() {
    let mut h = app(1280.0, 1400.0, 128);
    let (base, layer) = both_stacks(&mut h);
    // マスクはあるが効果の無いレイヤー（印は出ない）
    apply(&mut h, Action::NewLayer);
    let empty = h.state().state.selected_layer.unwrap();
    apply(&mut h, Action::M2(Edit::AddMask(empty)));
    for lang in Lang::ALL {
        h.state_mut().state.set_language(lang);
        h.run();
        let tip = |n: usize| yolu_app::panels::layers::mask_effects_tip(lang, n);
        let marks = |h: &Harness<'_, YoluApp>, n: usize| -> Vec<Rect> {
            h.query_all_by_label(&tip(n))
                .map(|node| node.rect())
                .collect()
        };
        let any_mark = |h: &Harness<'_, YoluApp>| {
            h.query_all_by_label_contains(lang.pick("マスクに効果", "on the mask"))
                .count()
        };
        // レイヤーの画素が対象: マスクの効果のあるレイヤーの、マスクのサムネイルの角に印（数は 2 つと 1 つ。効果の無いマスクには出ない）
        let at = pixels_thumb_of(&h, layer).center();
        click(&mut h, at);
        assert_eq!(any_mark(&h), 2, "{lang:?}");
        let thumb = mask_thumb_of(&h, layer);
        let two = marks(&h, 2);
        assert_eq!(two.len(), 1, "{lang:?}");
        assert!(
            two[0].center().x > thumb.center().x
                && two[0].center().y > thumb.center().y
                && thumb.expand(8.0).contains_rect(two[0]),
            "{lang:?}: 印は上のレイヤーのマスクのサムネイルの右下の角: {:?} {thumb:?}",
            two[0]
        );
        let one = marks(&h, 1);
        assert_eq!(one.len(), 1, "{lang:?}");
        let base_thumb = mask_thumb_of(&h, base);
        assert!(base_thumb.expand(8.0).contains_rect(one[0]), "{lang:?}");
        // 乗せるとツールチップにも同じ数（印の部品の名前と同じ文）
        move_to(&h, pos2(10.0, 10.0));
        h.run();
        hover_and_wait(&mut h, two[0].center());
        let count_text = tip(2);
        let with_count: Vec<_> = h.query_all_by_label_contains(&count_text).collect();
        assert_eq!(with_count.len(), 2, "{lang:?}: 印の名前とツールチップ");
        assert_eq!(
            with_count
                .iter()
                .filter(|n| n.accesskit_node().value().is_some_and(|l| l != count_text))
                .count(),
            1,
            "{lang:?}: ツールチップは、マスクのサムネイルの文に数の行を足した 1 つ"
        );
        // 印の上を押すと、下のマスクのサムネイルが受けてマスクが対象になる
        click(&mut h, two[0].center());
        assert!(editing_mask(&h), "{lang:?}");
        assert_eq!(h.state().state.selected_layer, Some(layer));
        // マスクが対象: そのレイヤーの印は消える（行が出ている）。ほかのレイヤー（レイヤーの効果を出している）の印は残る
        assert!(marks(&h, 2).is_empty(), "{lang:?}");
        assert_eq!(marks(&h, 1).len(), 1, "{lang:?}");
        assert_eq!(any_mark(&h), 1, "{lang:?}");
        // レイヤーの画素へ戻すと、また出る
        let at = pixels_thumb_of(&h, layer).center();
        click(&mut h, at);
        assert_eq!(marks(&h, 2).len(), 1, "{lang:?}");
    }
    // マスクの効果が無くなれば印は消える
    let lang = h.state().state.lang;
    let at = pixels_thumb_of(&h, layer).center();
    click(&mut h, at);
    assert_eq!(
        h.query_all_by_label(&yolu_app::panels::layers::mask_effects_tip(lang, 2))
            .count(),
        1
    );
    for id in ids_of(&h, layer, FilterTarget::Mask) {
        fx(&mut h, FxOp::Remove { layer, id });
    }
    assert_eq!(
        h.query_all_by_label_contains(lang.pick("マスクに効果", "on the mask"))
            .count(),
        1,
        "上のレイヤーのマスクが空になって、下のレイヤーの印だけが残る"
    );
    let _ = empty;
}

#[test]
fn dragging_a_layer_row_drops_in_the_same_place_whichever_side_is_the_target() {
    let order = |h: &Harness<'_, YoluApp>| -> Vec<yolu_app::engine::LayerId> {
        h.state()
            .state
            .doc
            .layers()
            .iter()
            .map(|l| l.id())
            .collect()
    };
    for mask_side in [false, true] {
        let what = if mask_side {
            "マスクが対象"
        } else {
            "レイヤーの画素が対象"
        };
        let mut h = app(1280.0, 1400.0, 128);
        let (base, layer) = both_stacks(&mut h);
        // 下のレイヤーと上のレイヤーの間に、レイヤーの効果を持つレイヤーを 1 つ挟む（選んでいないレイヤーの下はレイヤーの効果）
        let at = name_of(&h, base);
        click(&mut h, at);
        apply(&mut h, Action::NewLayer);
        let middle = h.state().state.selected_layer.unwrap();
        fx(
            &mut h,
            FxOp::AddFilter {
                target: FilterTarget::Content,
                kind: FilterKind::Threshold,
            },
        );
        assert_eq!(order(&h), vec![base, middle, layer]);
        let at = name_of(&h, layer);
        click(&mut h, at);
        let at = if mask_side {
            mask_thumb_of(&h, layer).center()
        } else {
            pixels_thumb_of(&h, layer).center()
        };
        click(&mut h, at);
        assert_eq!(editing_mask(&h), mask_side, "{what}");
        // 選んでいるレイヤー（効果の行が下に並ぶ）をつかんで、真ん中のレイヤーの行の下半分（その下の線）へ運ぶ。落とす先は、見えている行の位置で
        // 決まる（マスクが対象のときは、レイヤーの効果の行を数えた配置で決めると、1 つ上の線になる）
        let from = name_of(&h, layer);
        let to = pos2(from.x, name_of(&h, middle).y + 11.0);
        drag(&mut h, &[from, offset(from, 0.0, 12.0), to]);
        assert!(h.state().state.ui.layer_drag.is_none(), "{what}");
        assert_eq!(order(&h), vec![base, layer, middle], "{what}");
        assert_eq!(
            editing_mask(&h),
            mask_side,
            "{what}: 運んだレイヤーが選んでいるレイヤーなら対象はそのまま"
        );
        apply(&mut h, Action::Undo);
        assert_eq!(order(&h), vec![base, middle, layer], "{what}");
        // 選んでいないレイヤーをつかんで一番上へ運ぶ。つかんだ瞬間に対象がレイヤーの画素へ替わって効果の行が並び替わるので、
        // 並び替わった後の配置を見て落とす先へ動かす
        let from = name_of(&h, middle);
        press(&h, from, PointerButton::Primary);
        h.step();
        move_to(&h, offset(from, 0.0, 12.0));
        h.step();
        let to = pos2(from.x, name_of(&h, layer).y - 11.0);
        move_to(&h, to);
        h.step();
        release(&h, to, PointerButton::Primary);
        h.step();
        h.run();
        assert_eq!(
            order(&h),
            vec![base, layer, middle],
            "{what}: 選んでいないレイヤーを運ぶ"
        );
        assert!(
            !editing_mask(&h),
            "{what}: つかんだレイヤーの画素が対象になる"
        );
        assert_eq!(h.state().state.selected_layer, Some(middle));
    }
}

/// 右ボタンの 1 回の押し離し。
fn right_click(h: &mut Harness<'_, YoluApp>, at: egui::Pos2) {
    press(h, at, PointerButton::Secondary);
    h.step();
    release(h, at, PointerButton::Secondary);
    h.run();
}

#[test]
fn a_right_click_on_the_layer_thumbnail_opens_the_layer_menu_like_the_row() {
    let mut h = app(1280.0, 1400.0, 128);
    let base = h.state().state.selected_layer.unwrap();
    effect_document(&mut h);
    let layer = h.state().state.selected_layer.unwrap();
    assert_ne!(base, layer);
    let layer_menu = PopupKind::LayerContext(layer);
    // 選んでいないレイヤー（下地）を選んだ状態から、マスクのあるレイヤーの本体のサムネイルを右クリック: そのレイヤーを選んでレイヤーのメニュー
    let at = name_of(&h, base);
    click(&mut h, at);
    assert_eq!(h.state().state.selected_layer, Some(base));
    let at = pixels_thumb(&h).center();
    right_click(&mut h, at);
    assert_eq!(
        h.state().state.popup.as_ref().map(|p| p.kind),
        Some(layer_menu)
    );
    assert_eq!(h.state().state.selected_layer, Some(layer));
    // メニューにはレイヤーのメニューの項目（フィルターを追加 ▸）が並ぶ
    let _ = popup_item(&h, "フィルターを追加");
    // 選んだレイヤーの本体のサムネイルでも同じ（開くだけでマスクの対象は変えない）
    h.state_mut().state.popup = None;
    h.run();
    let at = pixels_thumb(&h).center();
    right_click(&mut h, at);
    assert_eq!(
        h.state().state.popup.as_ref().map(|p| p.kind),
        Some(layer_menu)
    );
    // 行の名前の所の右クリックも、今までどおり同じメニュー
    h.state_mut().state.popup = None;
    h.run();
    let at = name_of(&h, layer);
    right_click(&mut h, at);
    assert_eq!(
        h.state().state.popup.as_ref().map(|p| p.kind),
        Some(layer_menu)
    );
}

#[test]
fn snapshot_the_layer_and_mask_targets_and_the_add_entrances_in_both_languages() {
    let mut results = SnapshotResults::new();
    for lang in Lang::ALL {
        let suffix = lang.pick("ja", "en");
        let mut h = app(1280.0, 1400.0, 128);
        h.state_mut().state.set_language(lang);
        effect_document(&mut h);
        // マスクの効果を 2 つにする（マスクが対象のときの行が 2 行になる）
        fx(
            &mut h,
            FxOp::AddFilter {
                target: FilterTarget::Mask,
                kind: FilterKind::Levels,
            },
        );
        // レイヤーの画素が対象（レイヤーのサムネイルに青い枠）。プロパティはレイヤーのタブ（フィルターとジェネレーターの 2 つの入り口）。
        // レイヤーの効果の行が並び、効果の付いたマスクのサムネイルの角に印
        let at = pixels_thumb(&h).center();
        click(&mut h, at);
        h.state_mut().state.ui.property_tab = yolu_app::panels::properties::TAB_ICONS.len() - 1;
        move_to(&h, pos2(640.0, 500.0));
        h.run();
        h.snapshot(format!("layers_target_pixels_{suffix}"));
        // 印に乗せるとツールチップに効果の数
        let tip = yolu_app::panels::layers::mask_effects_tip(lang, 2);
        let mark = h.get_by_label(&tip).rect().center();
        hover_and_wait(&mut h, mark);
        h.snapshot(format!("layers_mask_mark_tooltip_{suffix}"));
        move_to(&h, pos2(640.0, 500.0));
        h.run();
        // 「ジェネレーターを追加」のポップアップ（ジェネレーターだけ）
        let at = props_button(&h, lang.pick("ジェネレーターを追加", "Add Generator")).center();
        click(&mut h, at);
        assert_eq!(
            h.state().state.popup.as_ref().map(|p| p.kind),
            Some(PopupKind::M2(yolu_app::m2_menu::Popup::AddGenerator(
                FilterTarget::Content
            )))
        );
        h.snapshot(format!("fx_add_generator_menu_{suffix}"));
        common::key(&h, egui::Key::Escape, egui::Modifiers::NONE);
        h.run();
        // マスクが対象（マスクのサムネイルに青い枠）
        let at = mask_thumb(&h).center();
        click(&mut h, at);
        move_to(&h, pos2(640.0, 500.0));
        h.run();
        h.snapshot(format!("layers_target_mask_{suffix}"));
        // マスクのサムネイルの右クリック
        let at = mask_thumb(&h).center();
        press(&h, at, PointerButton::Secondary);
        h.step();
        release(&h, at, PointerButton::Secondary);
        h.run();
        h.snapshot(format!("layers_mask_menu_{suffix}"));
        results.extend_harness(&mut h);
    }
}

#[test]
fn rows_select_toggle_move_and_remove_from_the_panel() {
    let mut h = app(1280.0, 1400.0, 128);
    apply(&mut h, Action::M2(Edit::NewFill));
    let layer = h.state().state.selected_layer.unwrap();
    fx(
        &mut h,
        FxOp::AddFilter {
            target: FilterTarget::Content,
            kind: FilterKind::Blur,
        },
    );
    fx(
        &mut h,
        FxOp::AddFilter {
            target: FilterTarget::Content,
            kind: FilterKind::Invert,
        },
    );
    let ids: Vec<_> = h
        .state()
        .state
        .doc
        .filters_of(layer, FilterTarget::Content)
        .unwrap()
        .iter()
        .map(|e| e.id())
        .collect();
    // レイヤーの行を押すと、選んでいた効果は外れる
    let layer_name = h.state().state.doc.layer(layer).unwrap().name().to_owned();
    h.get_by_label(&layer_name).click();
    h.run();
    assert_eq!(selected(&h), None);
    // 効果の行を押すと選ぶ（そのレイヤーのまま）
    let blur_label = "ぼかし（ガウス）  4 px";
    h.get_by_label(blur_label).click();
    h.run();
    assert_eq!(selected(&h), Some(Selected::Filter { layer, id: ids[0] }));
    // 上へ（後から掛かる）: ぼかしが 1 つ上の段になる。1 回の Undo
    let steps = h.state().state.doc.undo_count();
    h.get_by_label("上へ（後から掛かる）").click();
    h.run();
    let order: Vec<_> = h
        .state()
        .state
        .doc
        .filters_of(layer, FilterTarget::Content)
        .unwrap()
        .iter()
        .map(|e| e.id())
        .collect();
    assert_eq!(order, vec![ids[1], ids[0]]);
    assert_eq!(h.state().state.doc.undo_count(), steps + 1);
    // 目で無効にする
    h.get_all_by_label("フィルターを無効にする")
        .next()
        .unwrap()
        .click();
    h.run();
    assert!(
        !h.state().state.doc.find_filter(ids[0]).unwrap().1.enabled(),
        "一番上の行（ぼかし）の目で無効になる"
    );
    // 消す
    h.get_by_label("フィルターを削除").click();
    h.run();
    assert_eq!(
        h.state()
            .state
            .doc
            .filters_of(layer, FilterTarget::Content)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(selected(&h), None, "消した行の選びは外れる");
    // Undo（Ctrl+Z）で戻る
    key(&h, egui::Key::Z, egui::Modifiers::COMMAND);
    h.run();
    assert_eq!(
        h.state()
            .state
            .doc
            .filters_of(layer, FilterTarget::Content)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn the_context_menu_of_a_row_acts_on_that_row() {
    let mut h = app(1280.0, 1400.0, 128);
    apply(&mut h, Action::M2(Edit::NewFill));
    let layer = h.state().state.selected_layer.unwrap();
    fx(
        &mut h,
        FxOp::AddFilter {
            target: FilterTarget::Content,
            kind: FilterKind::Blur,
        },
    );
    let id = h
        .state()
        .state
        .doc
        .filters_of(layer, FilterTarget::Content)
        .unwrap()[0]
        .id();
    fx(&mut h, FxOp::Deselect);
    let at = h.get_by_label("ぼかし（ガウス）  4 px").rect().center();
    press(&h, at, PointerButton::Secondary);
    h.step();
    release(&h, at, PointerButton::Secondary);
    h.run();
    assert_eq!(
        selected(&h),
        Some(Selected::Filter { layer, id }),
        "右クリックで選ぶ"
    );
    assert!(matches!(
        h.state().state.popup.as_ref().map(|p| p.kind),
        Some(PopupKind::M2(yolu_app::m2_menu::Popup::EffectContext))
    ));
    // 行の消すボタンと同じ名前なので、メニューの項目（幅のある矩形）を選ぶ
    let item = rect_of(&h, "フィルターを削除", |r| r.width() > 80.0);
    click(&mut h, item.center());
    assert!(h
        .state()
        .state
        .doc
        .filters_of(layer, FilterTarget::Content)
        .unwrap()
        .is_empty());
}

#[test]
fn a_filter_is_added_from_the_properties_button_and_the_filter_menu() {
    let mut h = app(1280.0, 1400.0, 128);
    let layer = h.state().state.selected_layer.unwrap();
    h.state_mut().state.ui.property_tab = yolu_app::panels::properties::TAB_ICONS.len() - 1; // レイヤーのタブ（最後）
    h.run();
    let at = props_button(&h, "フィルターを追加").center();
    click(&mut h, at);
    let item = popup_item(&h, "シャープ（アンシャープマスク）");
    click(&mut h, item.center());
    assert_eq!(
        h.state()
            .state
            .doc
            .filters_of(layer, FilterTarget::Content)
            .unwrap()
            .len(),
        1
    );
    // メニューバーの「フィルター」から
    let title = menu_title(&h, "フィルター");
    click(&mut h, title.center());
    assert!(matches!(
        h.state().state.popup.as_ref().map(|p| p.kind),
        Some(PopupKind::MenuBar(4))
    ));
    let item = popup_item(&h, "階調の反転");
    click(&mut h, item.center());
    assert_eq!(
        h.state()
            .state
            .doc
            .filters_of(layer, FilterTarget::Content)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn dragging_a_slider_in_the_effect_panel_is_one_undo_step() {
    let mut h = app(1280.0, 1400.0, 128);
    let layer = h.state().state.selected_layer.unwrap();
    fx(
        &mut h,
        FxOp::AddFilter {
            target: FilterTarget::Content,
            kind: FilterKind::Blur,
        },
    );
    let id = h
        .state()
        .state
        .doc
        .filters_of(layer, FilterTarget::Content)
        .unwrap()[0]
        .id();
    let steps = h.state().state.doc.undo_count();
    let slider = rect_of(&h, "半径", |r| r.left() > rx());
    let y = slider.center().y + 8.0;
    drag(
        &mut h,
        &[
            pos2(slider.left() + 30.0, y),
            pos2(slider.left() + 80.0, y),
            pos2(slider.left() + 120.0, y),
        ],
    );
    let radius = match h.state().state.doc.find_filter(id).unwrap().1.settings() {
        yolu_core::EffectSettings::Filter(yolu_core::filter::Settings::GaussianBlur { radius }) => {
            *radius
        }
        other => panic!("{other:?}"),
    };
    assert_ne!(radius, 4, "つまみで半径が変わる");
    assert_eq!(
        h.state().state.doc.undo_count(),
        steps + 1,
        "ドラッグは 1 回の Undo"
    );
    let _ = UiOp::EditMask(false);
}

// ───────── 0.5.0 のフィルターの欄 ─────────

/// 0.5.0 のフィルター（目録から欄を並べる種類）。
const NEW_FILTERS: [FilterKind; 10] = [
    FilterKind::HistogramScan,
    FilterKind::HistogramRange,
    FilterKind::SlopeBlur,
    FilterKind::DirectionalBlur,
    FilterKind::Warp,
    FilterKind::Morphology,
    FilterKind::EdgeDetect,
    FilterKind::HighPass,
    FilterKind::Median,
    FilterKind::Glow,
];

/// `kind` を描くチャンネル（スカラーだけの種類は Roughness）の画素に足して選んだウィンドウ。
fn with_new_filter(lang: Lang, kind: FilterKind) -> Harness<'static, YoluApp> {
    let mut h = app(1280.0, 1400.0, 128);
    h.state_mut().state.set_language(lang);
    let scalar = matches!(
        kind,
        FilterKind::HistogramScan
            | FilterKind::HistogramRange
            | FilterKind::Morphology
            | FilterKind::EdgeDetect
    );
    if scalar {
        apply(
            &mut h,
            Action::M2Ui(UiOp::PaintChannel(yolu_core::Channel::Roughness)),
        );
    }
    fx(
        &mut h,
        FxOp::AddFilter {
            target: FilterTarget::Content,
            kind,
        },
    );
    h
}

fn selected_settings(h: &Harness<'_, YoluApp>) -> yolu_core::EffectSettings {
    let (_, effect, _) = h.state().state.fx.filter(&h.state().state.doc).unwrap();
    effect.settings().clone()
}

#[test]
fn the_new_filters_show_every_value_by_name_in_both_languages() {
    for lang in Lang::ALL {
        for kind in NEW_FILTERS {
            let h = with_new_filter(lang, kind);
            assert!(selected(&h).is_some(), "{kind:?}: 足したものを選ぶ");
            let settings = selected_settings(&h);
            let id = kind.catalog_id().unwrap();
            let texts = shown_texts(&h);
            for (name, _) in settings.catalog_values() {
                let label = yolu_app::fx::names::param_label(lang, id, name);
                assert!(
                    h.query_all_by_label_contains(label)
                        .any(|n| n.rect().left() > rx())
                        || texts.iter().any(|(t, r)| t == label && r.left() > rx()),
                    "{lang:?} {kind:?}: 欄「{label}」が無い"
                );
            }
            for (text, _) in texts
                .iter()
                .filter(|(_, r)| r.left() > rx() + 20.0 && r.top() > 280.0 && r.bottom() < 2376.0)
            {
                assert!(!text.contains('。'), "{lang:?} {kind:?}: 文の形 {text:?}");
                if lang == Lang::En {
                    assert!(!has_japanese(text), "{kind:?}: 英語の画面に日本語 {text:?}");
                }
            }
        }
    }
}

#[test]
fn a_choice_button_and_a_slider_of_a_new_filter_are_one_undo_step_each() {
    let mut h = with_new_filter(Lang::Ja, FilterKind::SlopeBlur);
    let steps = h.state().state.doc.undo_count();
    // 合わせ方のボタン（最大）
    let max = rect_of(&h, "最大", |r| r.left() > rx());
    click(&mut h, max.center());
    assert!(matches!(
        selected_settings(&h),
        yolu_core::EffectSettings::Filter(yolu_core::filter::Settings::SlopeBlur {
            mode: yolu_core::filter::SlopeMode::Max,
            ..
        })
    ));
    assert_eq!(h.state().state.doc.undo_count(), steps + 1);
    // 長さのスライダーのドラッグは 1 回の取り消し
    let slider = rect_of(&h, "長さ", |r| r.left() > rx());
    let y = slider.center().y + 8.0;
    drag(
        &mut h,
        &[
            pos2(slider.left() + 30.0, y),
            pos2(slider.left() + 80.0, y),
            pos2(slider.left() + 120.0, y),
        ],
    );
    let intensity = match selected_settings(&h) {
        yolu_core::EffectSettings::Filter(yolu_core::filter::Settings::SlopeBlur {
            intensity,
            ..
        }) => intensity,
        other => panic!("{other:?}"),
    };
    assert_ne!(intensity, 8.0, "つまみで長さが変わる");
    assert_eq!(h.state().state.doc.undo_count(), steps + 2);
    apply(&mut h, Action::Undo);
    apply(&mut h, Action::Undo);
    assert_eq!(
        selected_settings(&h),
        FilterKind::SlopeBlur.settings(),
        "2 回の取り消しで足したときの値"
    );
}

#[test]
fn snapshot_three_new_filter_panels_in_both_languages() {
    let mut results = SnapshotResults::new();
    for lang in Lang::ALL {
        for (kind, name) in [
            (FilterKind::SlopeBlur, "slope_blur"),
            (FilterKind::Morphology, "morphology"),
            (FilterKind::Glow, "glow"),
        ] {
            let mut h = with_new_filter(lang, kind);
            move_to(&h, pos2(640.0, 500.0));
            h.run();
            h.snapshot(format!("fx_props_{name}_{}", lang.pick("ja", "en")));
            results.extend_harness(&mut h);
        }
    }
}

/// 0.5.0 の Generator（模様・ライト・マスクの組み立て）を画素に足して選んだウィンドウ。
fn with_new_generator(lang: Lang, kind: Kind, height: f32) -> Harness<'static, YoluApp> {
    let mut h = app(1280.0, height, 128);
    h.state_mut().state.set_language(lang);
    fx(
        &mut h,
        FxOp::AddGenerator {
            target: FilterTarget::Content,
            kind,
        },
    );
    h
}

#[test]
fn the_new_generators_show_their_own_values_and_the_common_rows_in_both_languages() {
    for lang in Lang::ALL {
        for (kind, id) in [
            (Kind::Pattern, "pattern"),
            (Kind::Light, "light"),
            (Kind::MaskBuilder, "mask_builder"),
            (Kind::UvIslandVariation, "uv_island_variation"),
        ] {
            // 欄の全部がウィンドウに収まる高さ
            let h = with_new_generator(lang, kind, 2400.0);
            assert!(selected(&h).is_some(), "{kind:?}");
            let settings = selected_settings(&h);
            let texts = shown_texts(&h);
            let shown = |label: &str| {
                h.query_all_by_label_contains(label)
                    .any(|n| n.rect().left() > rx())
                    || texts.iter().any(|(t, r)| t == label && r.left() > rx())
            };
            for (name, _) in settings.catalog_values() {
                if ["low", "high", "invert", "blend"].contains(&name)
                    || (matches!(id, "mask_builder" | "uv_island_variation") && name == "softness")
                {
                    continue;
                }
                let label = yolu_app::fx::names::param_label(lang, id, name);
                assert!(shown(label), "{lang:?} {kind:?}: 欄「{label}」が無い");
            }
            // 共通の範囲の行は出る。模様・ライトの共通のやわらかさと、崩し（重ねるノイズ）は出ない
            assert!(shown(lang.pick("下限", "Low")), "{kind:?}");
            assert!(!shown(lang.pick("崩し", "Breakup")), "{kind:?}");
            if matches!(kind, Kind::Pattern | Kind::Light) {
                assert!(!shown(lang.pick("やわらかさ", "Softness")), "{kind:?}");
            } else {
                assert!(shown(lang.pick("やわらかさ", "Softness")), "{kind:?}");
            }
            for (text, _) in texts
                .iter()
                .filter(|(_, r)| r.left() > rx() + 20.0 && r.top() > 280.0 && r.bottom() < 2376.0)
            {
                assert!(!text.contains('。'), "{lang:?} {kind:?}: 文の形 {text:?}");
                if lang == Lang::En {
                    assert!(!has_japanese(text), "{kind:?}: 英語の画面に日本語 {text:?}");
                }
            }
        }
    }
}

#[test]
fn a_pattern_shape_button_changes_the_generator_in_one_undo_step() {
    let mut h = with_new_generator(Lang::Ja, Kind::Pattern, 1000.0);
    let steps = h.state().state.doc.undo_count();
    let dots = rect_of(&h, "水玉", |r| r.left() > rx());
    click(&mut h, dots.center());
    let shape = selected_settings(&h)
        .generator_settings()
        .unwrap()
        .pattern
        .shape;
    assert_eq!(shape, yolu_core::generator::PatternShape::Dots);
    assert_eq!(h.state().state.doc.undo_count(), steps + 1);
    apply(&mut h, Action::Undo);
    assert_eq!(
        selected_settings(&h)
            .generator_settings()
            .unwrap()
            .pattern
            .shape,
        yolu_core::generator::PatternShape::Stripes
    );
}

#[test]
fn snapshot_the_pattern_and_mask_builder_panels_in_both_languages() {
    let mut results = SnapshotResults::new();
    for lang in Lang::ALL {
        for (kind, name) in [
            (Kind::Pattern, "pattern"),
            (Kind::MaskBuilder, "mask_builder"),
        ] {
            let mut h = with_new_generator(lang, kind, 1000.0);
            move_to(&h, pos2(640.0, 500.0));
            h.run();
            h.snapshot(format!("fx_props_{name}_{}", lang.pick("ja", "en")));
            results.extend_harness(&mut h);
        }
    }
}

/// 2×2 の 4 枚の板（3D で離れていて、UV の別の所。アイランドは 4 つ）。
fn four_plates() -> yolu_app::view3d::model::ViewModel {
    use yolu_core::geometry::{ModelMesh, Submesh};
    use yolu_core::glam::{Vec2, Vec3};
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    for (k, (u0, v0)) in [(0.05f32, 0.05f32), (0.55, 0.05), (0.05, 0.55), (0.55, 0.55)]
        .into_iter()
        .enumerate()
    {
        let base = positions.len() as u32;
        for (x, y) in [(0., 0.), (1., 0.), (0., 1.), (1., 1.)] {
            positions.push(Vec3::new(
                x + (k % 2) as f32 * 1.5,
                y + (k / 2) as f32 * 1.5,
                0.,
            ));
            uvs.push(Vec2::new(u0 + 0.4 * x, v0 + 0.4 * y));
        }
        indices.extend([0, 1, 2, 2, 1, 3].map(|i| base + i));
    }
    let mesh = ModelMesh {
        name: "板".into(),
        positions,
        normals: Vec::new(),
        uvs,
        submeshes: vec![Submesh {
            material: 0,
            indices,
        }],
    };
    yolu_app::view3d::model::ViewModel::new("板", vec![mesh], vec![Some("材".into())], 1).unwrap()
}

/// アイランドごとのばらつきの欄と、アイランドごとに違う値の 2D の絵（4 つのアイランドのモデルを読み、塗ったレイヤーに置き換えで足す）。
#[test]
fn snapshot_the_uv_island_variation_panel_and_canvas_in_both_languages() {
    let mut results = SnapshotResults::new();
    for lang in Lang::ALL {
        let mut h = app(1280.0, 1400.0, 128);
        h.state_mut().state.set_language(lang);
        {
            let s = &mut h.state_mut().state;
            let layer = s.selected_layer.unwrap();
            for y in 0..128 {
                for x in 0..128 {
                    s.doc
                        .set_pixel(layer, x, y, yolu_core::Rgba8::new(196, 120, 64, 255))
                        .unwrap();
                }
            }
            s.view3d.set_model(four_plates());
            s.sync_effect_inputs_with(true);
        }
        fx(
            &mut h,
            FxOp::AddGenerator {
                target: FilterTarget::Content,
                kind: Kind::UvIslandVariation,
            },
        );
        let Some(Selected::Filter { layer, id }) = selected(&h) else {
            panic!("{lang:?}")
        };
        let mut g = selected_settings(&h).generator_settings().unwrap().clone();
        g.blend = yolu_core::generator::Blend::Replace;
        fx(
            &mut h,
            FxOp::SetSettings {
                layer,
                id,
                settings: yolu_core::EffectSettings::generator(g),
                coalesce: false,
            },
        );
        assert_eq!(
            h.state().state.doc.generator_inactive(layer, id).unwrap(),
            None,
            "{lang:?}: モデルを読んでいる"
        );
        // ブラシの円をアイランドに重ねない（指はキャンバスの外）
        move_to(&h, pos2(1270.0, 990.0));
        h.run();
        h.snapshot(format!(
            "fx_props_uv_island_variation_{}",
            lang.pick("ja", "en")
        ));
        results.extend_harness(&mut h);
    }
}

/// 描いた文字（アイコンの頭文字は除く）の一覧。
fn shown_texts(h: &Harness<'_, YoluApp>) -> Vec<(String, Rect)> {
    fn walk(shape: &Shape, out: &mut Vec<(String, Rect)>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            Shape::Text(text) => {
                out.push((
                    text.galley.job.text.clone(),
                    Rect::from_min_size(text.pos, text.galley.size()),
                ));
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        walk(&shape.shape, &mut out);
    }
    out
}

#[test]
fn the_effect_screens_have_no_instruction_text_and_the_english_one_no_japanese() {
    for lang in Lang::ALL {
        for select in 0..3 {
            let mut h = app(1280.0, 1400.0, 128);
            h.state_mut().state.set_language(lang);
            effect_document(&mut h);
            let layer = h.state().state.selected_layer.unwrap();
            let stage = h
                .state()
                .state
                .doc
                .filters_of(layer, FilterTarget::Content)
                .unwrap()[0]
                .id();
            match select {
                0 => {}
                1 => fx(&mut h, FxOp::SelectFilter { layer, id: stage }),
                _ => {
                    let anchor = h
                        .state()
                        .state
                        .doc
                        .layer(layer)
                        .unwrap()
                        .anchor()
                        .unwrap()
                        .id();
                    fx(&mut h, FxOp::SelectAnchor(anchor));
                }
            }
            // 開いたメニューも
            let texts = shown_texts(&h);
            // レイヤーの一覧とプロパティの欄（右の列。状態の帯の知らせは状態なので除く）
            for (text, rect) in texts.iter().filter(|(_, r)| {
                r.left() > rx() + 20.0 && r.top() > 280.0 && r.bottom() < 1400.0 - 24.0
            }) {
                assert!(
                    !text.contains('。') && !text.ends_with('.'),
                    "{lang:?}/{select}: 文の形の文字 {text:?}"
                );
                assert!(
                    text.chars().count() <= 40,
                    "{lang:?}/{select}: 長い文字 {text:?} {rect:?}"
                );
            }
            if lang == Lang::En {
                for (text, _) in &texts {
                    assert!(!has_japanese(text), "英語の画面に日本語 {text:?}");
                }
            }
        }
    }
}

#[test]
fn the_add_menu_in_english_has_no_japanese_and_in_japanese_every_name_is_localised() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 1400.0, 128);
        h.state_mut().state.set_language(lang);
        h.state_mut().state.ui.property_tab = yolu_app::panels::properties::TAB_ICONS.len() - 1;
        h.run();
        let at = props_button(&h, lang.pick("フィルターを追加", "Add Filter")).center();
        click(&mut h, at);
        for (text, _) in shown_texts(&h) {
            if lang == Lang::En {
                assert!(!has_japanese(&text), "{text:?}");
            }
        }
        let mut entries =
            yolu_app::fx::menu::add_filter_entries(&h.state().state, FilterTarget::Content);
        entries.extend(yolu_app::fx::menu::add_generator_entries(
            &h.state().state,
            FilterTarget::Content,
        ));
        let names: Vec<String> = yolu_app::ui::menu::leaves(&entries)
            .into_iter()
            .filter_map(|e| match e {
                yolu_app::ui::menu::Entry::Item { label, .. } => Some(label.clone()),
                _ => None,
            })
            .collect();
        assert!(names.len() >= 15, "{names:?}");
        if lang == Lang::En {
            assert!(names.iter().all(|n| !has_japanese(n)), "{names:?}");
        }
    }
}

#[test]
fn a_generator_row_without_maps_has_a_mark_whose_tooltip_gives_the_reason() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 1400.0, 128);
        h.state_mut().state.set_language(lang);
        effect_document(&mut h);
        let label = format!(
            "{}  {}",
            lang.pick("エッジの摩耗", "Edge Wear"),
            lang.pick("乗算", "Multiply")
        );
        let row = h.get_by_label(&label).rect();
        // 選んでいる行には上へ・下へ・消すの 3 つのボタンがあり、その左に印
        let at = pos2(row.right() - 4.0 - 60.0 - 10.0, row.center().y);
        move_to(&h, at);
        for _ in 0..90 {
            h.step();
        }
        let reason = lang.pick("Curvature のマップがありません", "No Curvature map");
        let texts = shown_texts(&h);
        // 欄の警告の行にも同じ理由が出ているので、ポインタの近く（行のすぐ下）に出た文字を見る
        assert!(
            texts.iter().any(|(t, r)| t.contains(reason)
                && r.top() > row.top()
                && r.top() < row.top() + 90.0),
            "{lang:?}: {texts:?}"
        );
    }
}

/// 行の下（ポインタのすぐ下）に出ている文字のうち、理由を含む物。
fn reason_tooltips(h: &Harness<'_, YoluApp>, row: Rect, reason: &str) -> Vec<String> {
    shown_texts(h)
        .into_iter()
        .filter(|(t, r)| t.contains(reason) && r.top() > row.top() && r.top() < row.top() + 90.0)
        .map(|(t, _)| t)
        .collect()
}

#[test]
fn over_the_warning_mark_only_the_mark_tooltip_shows_and_elsewhere_on_the_row_the_row_tooltip() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 1000.0, 128);
        h.state_mut().state.set_language(lang);
        effect_document(&mut h);
        let label = format!(
            "{}  {}",
            lang.pick("エッジの摩耗", "Edge Wear"),
            lang.pick("乗算", "Multiply")
        );
        let row = h.get_by_label(&label).rect();
        let reason = lang.pick("Curvature のマップがありません", "No Curvature map");
        // 印の上: 理由だけの 1 つ（行のツールチップ「名前＋理由」は出さない）
        move_to(&h, pos2(row.right() - 4.0 - 60.0 - 10.0, row.center().y));
        for _ in 0..90 {
            h.step();
        }
        let on_mark = reason_tooltips(&h, row, reason);
        assert_eq!(on_mark, vec![reason.to_owned()], "{lang:?}: 印の上");
        // 印でも名前でもない所（行の左の端）: 行のツールチップ 1 つ（名前＋理由）
        move_to(&h, pos2(row.left() + 40.0, row.center().y));
        for _ in 0..90 {
            h.step();
        }
        let on_row = reason_tooltips(&h, row, reason);
        assert_eq!(
            on_row,
            vec![format!("{label}\n{reason}")],
            "{lang:?}: 行の上"
        );
    }
}

fn procedural(h: &Harness<'_, YoluApp>) -> yolu_core::generator::Settings {
    let (_, effect, _) = h.state().state.fx.filter(&h.state().state.doc).unwrap();
    match effect.settings() {
        yolu_core::EffectSettings::Generator(g) => *g.clone(),
        _ => panic!("Generator"),
    }
}

#[test]
fn procedural_controls_and_presets_are_one_undo_and_bilingual() {
    use yolu_core::generator::{CellOutput, GrungePreset, NoiseBasis};
    for lang in Lang::ALL {
        let mut h = app(1280.0, 1800.0, 32);
        h.state_mut().state.set_language(lang);
        fx(
            &mut h,
            FxOp::AddGenerator {
                target: FilterTarget::Content,
                kind: Kind::Noise,
            },
        );
        let steps = h.state().state.doc.undo_count();
        h.get_by_label(lang.pick("振り直す", "Reroll")).click();
        h.run();
        assert_ne!(procedural(&h).procedural.seed, 0);
        assert_eq!(h.state().state.doc.undo_count(), steps + 1);
        apply(&mut h, Action::Undo);
        assert_eq!(procedural(&h).procedural.seed, 0);
        let slider = rect_of(&h, lang.pick("模様の大きさ", "Pattern Size"), |r| {
            r.left() > rx()
        });
        let y = slider.center().y + 8.0;
        drag(
            &mut h,
            &[
                pos2(slider.left() + 30.0, y),
                pos2(slider.left() + 80.0, y),
                pos2(slider.left() + 120.0, y),
            ],
        );
        assert_ne!(procedural(&h).procedural.scale, 0.1);
        assert_eq!(h.state().state.doc.undo_count(), steps + 1);
        apply(&mut h, Action::Undo);
        assert_eq!(procedural(&h).procedural.scale, 0.1);
        let basis = rect_of(&h, lang.pick("基底: Perlin", "Basis: Perlin"), |r| {
            r.left() > rx()
        });
        click(&mut h, basis.center());
        let choice = popup_item(&h, "Worley");
        click(&mut h, choice.center());
        assert_eq!(procedural(&h).procedural.basis, NoiseBasis::Worley);
        let cell = rect_of(
            &h,
            lang.pick("セルの出力: F1", "Cell Output: F1"),
            |r| r.left() > rx(),
        );
        click(&mut h, cell.center());
        let choice = popup_item(&h, "F2−F1");
        click(&mut h, choice.center());
        assert_eq!(procedural(&h).procedural.cell_output, CellOutput::F2MinusF1);
        let basis = rect_of(&h, lang.pick("基底: Worley", "Basis: Worley"), |r| {
            r.left() > rx()
        });
        click(&mut h, basis.center());
        let choice = popup_item(&h, "Perlin");
        click(&mut h, choice.center());
        assert_eq!(procedural(&h).procedural.cell_output, CellOutput::F1);
        fx(
            &mut h,
            FxOp::AddGenerator {
                target: FilterTarget::Content,
                kind: Kind::Grunge,
            },
        );
        let steps = h.state().state.doc.undo_count();
        h.get_by_label(lang.pick("布目", "Weave")).click();
        h.run();
        assert_eq!(procedural(&h).procedural.preset, GrungePreset::Weave);
        assert_eq!(h.state().state.doc.undo_count(), steps + 1);
        apply(&mut h, Action::Undo);
        assert_eq!(procedural(&h).procedural.preset, GrungePreset::Stain);
        if lang == Lang::En {
            assert!(!shown_texts(&h).iter().any(|(t, _)| has_japanese(t)));
        }
    }
}

/// ノイズ・グランジの欄の、見えている操作を全部触る。どれも core に断られず（状態の帯が空のまま）、設定が変わり、1 回の Undo になる。
fn touch_every_procedural_control(h: &mut Harness<'_, YoluApp>, lang: Lang, kind: Kind) {
    use egui::{Event, Modifiers};
    use yolu_core::generator::{CellOutput, FractalMode, NoiseBasis, ProceduralSpace};
    let column = |r: Rect| r.left() > rx();
    // 1 つの操作の後に、断られていない・設定が変わった・1 回の Undo、を確かめる
    fn accepted(
        h: &Harness<'_, YoluApp>,
        what: &str,
        steps: usize,
        before: &yolu_core::generator::Settings,
    ) {
        let state = &h.state().state;
        assert!(state.message.is_empty(), "{what}: {}", state.message);
        assert_ne!(&procedural(h), before, "{what}: 設定が変わらない");
        assert_eq!(state.doc.undo_count(), steps + 1, "{what}: 1 回の Undo");
    }
    let run =
        |h: &mut Harness<'_, YoluApp>, what: &str, act: &dyn Fn(&mut Harness<'_, YoluApp>)| {
            h.state_mut().state.message.clear();
            let (steps, before) = (h.state().state.doc.undo_count(), procedural(h));
            act(h);
            accepted(h, what, steps, &before);
        };
    let slide_to = |h: &mut Harness<'_, YoluApp>, label: &str, end: f32| {
        let r = rect_of(h, label, column);
        let y = r.center().y + 8.0;
        drag(
            h,
            &[
                pos2(r.left() + 30.0, y),
                pos2(r.left() + (30.0 + end) / 2.0, y),
                pos2(r.left() + end, y),
            ],
        );
    };
    let slide = |h: &mut Harness<'_, YoluApp>, label: &str| slide_to(h, label, 120.0);
    let choose = |h: &mut Harness<'_, YoluApp>, shown: &str, item: &str| {
        let r = rect_of(h, shown, column);
        click(h, r.center());
        let choice = popup_item(h, item);
        click(h, choice.center());
    };
    let t = |ja: &'static str, en: &'static str| lang.pick(ja, en);
    // 共通
    run(h, "空間", &|h| {
        choose(
            h,
            &format!("{}: {}", t("空間", "Space"), t("位置", "Position")),
            "UV",
        )
    });
    assert_eq!(procedural(h).procedural.space, ProceduralSpace::Uv);
    run(h, "空間（トライプラナー）", &|h| {
        choose(
            h,
            &format!("{}: UV", t("空間", "Space")),
            t("トライプラナー", "Triplanar"),
        )
    });
    run(h, "シード", &|h| {
        let field = h
            .get_all_by_role(egui::accesskit::Role::TextInput)
            .map(|n| n.rect())
            .find(|r| column(*r))
            .expect("右の列の数の入力欄はシードだけ");
        click(h, field.center());
        key(h, egui::Key::A, Modifiers::COMMAND);
        h.step();
        h.event(Event::Text("7".into()));
        h.step();
        key(h, egui::Key::Enter, Modifiers::NONE);
        h.run();
    });
    assert_eq!(procedural(h).procedural.seed, 7);
    run(h, "振り直す", &|h| {
        h.get_by_label(t("振り直す", "Reroll")).click();
        h.run();
    });
    for label in [
        t("模様の大きさ", "Pattern Size"),
        t("にじみ", "Bleed"),
        t("トライプラナーの幅", "Triplanar Width"),
        &format!("{} X", t("回転", "Rotation")),
        &format!("{} Y", t("回転", "Rotation")),
        &format!("{} Z", t("回転", "Rotation")),
    ] {
        run(h, label, &|h| slide(h, label));
    }
    match kind {
        Kind::Noise => {
            run(h, "基底", &|h| {
                choose(h, &format!("{}: Perlin", t("基底", "Basis")), "Worley")
            });
            assert_eq!(procedural(h).procedural.basis, NoiseBasis::Worley);
            run(h, "セルの出力", &|h| {
                choose(
                    h,
                    &format!("{}: F1", t("セルの出力", "Cell Output")),
                    "F2−F1",
                )
            });
            assert_eq!(procedural(h).procedural.cell_output, CellOutput::F2MinusF1);
            run(h, "重ね方", &|h| {
                choose(h, &format!("{}: fBm", t("重ね方", "Fractal")), "ridged")
            });
            assert_eq!(procedural(h).procedural.fractal, FractalMode::Ridged);
            // オクターブは初期値（5）が 1〜8 の真ん中より少し上なので、手前へ動かす
            run(h, "オクターブ", &|h| {
                slide_to(h, t("オクターブ", "Octaves"), 50.0)
            });
            for label in [t("ラクナリティ", "Lacunarity"), t("ゲイン", "Gain")] {
                run(h, label, &|h| slide(h, label));
            }
            // Worley から戻すとセルの出力も既定へ戻り、core に断られない
            run(h, "基底（戻す）", &|h| {
                choose(h, &format!("{}: Worley", t("基底", "Basis")), "Perlin")
            });
            assert_eq!(procedural(h).procedural.cell_output, CellOutput::F1);
        }
        _ => {
            run(h, "プリセット", &|h| {
                h.get_by_label(t("布目", "Weave")).click();
                h.run();
            });
        }
    }
    // 範囲・反転・合成・強さ（上限を先に動かすので、下限も動く）
    for label in [
        t("上限", "High"),
        t("下限", "Low"),
        t("やわらかさ", "Softness"),
    ] {
        run(h, label, &|h| slide(h, label));
    }
    run(h, "反転", &|h| {
        let r = rect_of(h, t("反転", "Invert"), column);
        click(h, pos2(r.left() + 8.0, r.center().y));
    });
    assert!(procedural(h).invert);
    run(h, "合成", &|h| {
        choose(
            h,
            &format!("{}: {}", t("合成", "Combine"), t("乗算", "Multiply")),
            t("加算", "Add"),
        )
    });
    let (steps, strength) = (h.state().state.doc.undo_count(), selected_strength(h));
    slide(h, t("強さ", "Strength"));
    assert_ne!(selected_strength(h), strength);
    assert_eq!(h.state().state.doc.undo_count(), steps + 1);
    assert!(
        h.state().state.message.is_empty(),
        "{}",
        h.state().state.message
    );
}

fn selected_strength(h: &Harness<'_, YoluApp>) -> f64 {
    let (_, effect, _) = h.state().state.fx.filter(&h.state().state.doc).unwrap();
    effect.strength()
}

#[test]
fn procedural_panels_have_no_breakup_rows_and_every_visible_control_is_accepted() {
    for lang in Lang::ALL {
        for kind in [Kind::Noise, Kind::Grunge] {
            let mut h = app(1280.0, 3200.0, 32);
            h.state_mut().state.set_language(lang);
            fx(
                &mut h,
                FxOp::AddGenerator {
                    target: FilterTarget::Content,
                    kind,
                },
            );
            if kind == Kind::Grunge {
                while !yolu_app::panels::grunge_picker::ready() {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                h.run();
            }
            // 崩しの組（量・シード・大きさ・置き場）は、重ねるノイズを持たない種類には出さない（core が断るので、同じ名前のシードも 1 つだけ）
            let texts = shown_texts(&h);
            let column: Vec<&String> = texts
                .iter()
                .filter(|(_, r)| r.left() > rx())
                .map(|(t, _)| t)
                .collect();
            for gone in [
                lang.pick("崩し", "Breakup"),
                lang.pick("量", "Amount"),
                lang.pick("置き場", "Placed"),
            ] {
                assert!(
                    !column
                        .iter()
                        .any(|t| t.as_str() == gone || t.starts_with(&format!("{gone}: "))),
                    "{lang:?} {kind:?}: {gone}"
                );
            }
            assert!(
                !column
                    .iter()
                    .any(|t| t.as_str() == lang.pick("大きさ", "Size")),
                "{lang:?} {kind:?}: 崩しの大きさ"
            );
            assert_eq!(
                column
                    .iter()
                    .filter(|t| t.as_str() == lang.pick("シード", "Seed"))
                    .count(),
                1,
                "{lang:?} {kind:?}"
            );
            touch_every_procedural_control(&mut h, lang, kind);
        }
    }
}

#[test]
fn shape_generator_edit_button_toggles_the_3d_target() {
    let mut h = app(1280.0, 1800.0, 32);
    apply(&mut h, Action::LoadDemoModel);
    fx(
        &mut h,
        FxOp::AddGenerator {
            target: FilterTarget::Content,
            kind: Kind::ShapeGradient,
        },
    );
    let Some(Selected::Filter { layer, id }) = selected(&h) else {
        panic!("選択")
    };
    let steps = h.state().state.doc.undo_count();
    h.get_by_label("3D ビューで編集").click();
    h.run();
    assert_eq!(h.state().state.fillfx.edit_filter, Some((layer, id)));
    assert_eq!(
        yolu_app::fillfx::gizmo::target(&h.state().state),
        Some(yolu_app::fillfx::gizmo::Target::Filter(layer, id))
    );
    h.get_by_label("3D ビューで編集").click();
    h.run();
    assert_eq!(h.state().state.fillfx.edit_filter, None);
    assert_eq!(h.state().state.doc.undo_count(), steps);
}

#[test]
fn the_shape_row_of_a_generator_tells_each_shape_with_the_same_sentences_as_the_menus() {
    use yolu_app::fx::names;
    for lang in Lang::ALL {
        let mut h = app(1280.0, 1800.0, 32);
        h.state_mut().state.set_language(lang);
        apply(&mut h, Action::LoadDemoModel);
        fx(
            &mut h,
            FxOp::AddGenerator {
                target: FilterTarget::Content,
                kind: Kind::ShapeGradient,
            },
        );
        let shape = procedural(&h).volume.shape;
        let row = h
            .get_by_label(&format!(
                "{}: {}",
                lang.pick("形", "Shape"),
                names::shape_name(lang, shape)
            ))
            .rect();
        hover_and_wait(&mut h, row.center());
        // ツールチップは 3 つの形を「名前: メニューと同じ説明」で並べる（形の名前の言い回しも同じ画面の名前と揃う）
        let tip = names::shape_tooltip(lang);
        assert_eq!(tip.lines().count(), names::SHAPES.len(), "{lang:?}");
        for s in names::SHAPES {
            let line = format!(
                "{}: {}",
                names::shape_name(lang, s),
                names::shape_hint(lang, s)
            );
            assert!(tip.lines().any(|l| l == line), "{lang:?}: {line}");
        }
        assert!(
            h.query_all_by_label_contains(names::shape_hint(
                lang,
                yolu_core::generator::Shape::Plane
            ))
            .next()
            .is_some(),
            "{lang:?}: ツールチップが出ていない"
        );
        assert_eq!(has_japanese(&tip), lang == Lang::Ja, "{lang:?}: {tip}");
    }
}

#[test]
fn snapshot_procedural_panels_in_both_languages() {
    let mut results = SnapshotResults::new();
    for lang in Lang::ALL {
        for (kind, name) in [(Kind::Noise, "noise"), (Kind::Grunge, "grunge")] {
            let mut h = app(1280.0, 1800.0, 32);
            h.state_mut().state.set_language(lang);
            fx(
                &mut h,
                FxOp::AddGenerator {
                    target: FilterTarget::Content,
                    kind,
                },
            );
            if kind == Kind::Grunge {
                while !yolu_app::panels::grunge_picker::ready() {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                h.run();
            }
            h.snapshot(format!(
                "fx_procedural_{name}_{}",
                if lang == Lang::Ja { "ja" } else { "en" }
            ));
            results.extend_harness(&mut h);
        }
    }
    results.unwrap();
}

#[test]
fn procedural_fallback_mark_explains_uv_in_both_languages() {
    for lang in Lang::ALL {
        for target in [FilterTarget::Content, FilterTarget::Mask] {
            let mut h = app(1280.0, 1800.0, 32);
            h.state_mut().state.set_language(lang);
            let layer = h.state().state.selected_layer.unwrap();
            if target == FilterTarget::Mask {
                apply(&mut h, Action::M2(Edit::AddMask(layer)));
            }
            fx(
                &mut h,
                FxOp::AddGenerator {
                    target,
                    kind: Kind::Noise,
                },
            );
            let (_, effect, _) = h.state().state.fx.filter(&h.state().state.doc).unwrap();
            let label = yolu_app::fx::names::effect_label(
                lang,
                effect,
                target,
                h.state().state.m2.paint_channel,
                |c| yolu_app::m2::channel_name(lang, &h.state().state.doc, c),
            );
            let row = h.get_by_label(&label).rect();
            move_to(&h, pos2(row.right() - 74.0, row.center().y));
            for _ in 0..90 {
                h.step();
            }
            let reason = lang.pick("UV の空間", "evaluated in UV space");
            assert!(shown_texts(&h).iter().any(|(t, r)| t.contains(reason)
                && r.top() > row.top()
                && r.top() < row.top() + 90.0));
        }
    }
}
