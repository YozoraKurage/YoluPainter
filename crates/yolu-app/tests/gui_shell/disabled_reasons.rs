//! 効かない欄は注記の行を置かず、無効（灰色）にして理由をツールチップに出す。その 3 か所（対称の欄・選択範囲を変更・調整レイヤー）が、
//! 無効のとき理由を出し、条件が外れたら有効に戻ることを確かめる。無効にしてよい条件を狭く保つ試験も含む:
//! - 対称は、描ける先が 3D だけのあいだと、指先・クローンのあいだだけ（ドックを分けてキャンバスも出ているあいだは 2D に描けるので有効）
//! - 調整レイヤーの欄は、描くチャンネルに効かなくても有効のまま（層の値は効くチャンネルの出力に効くので、直すためにチャンネルを替えさせない）
use crate::common;

use common::*;
use egui::Rect;
use egui_dock::{DockState, NodeIndex};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::engine::{BrushEffect, Channel, DVec2, SymmetryMode};
use yolu_app::lang::Lang;
use yolu_app::m2::{AdjustmentKind, Edit, UiOp};
use yolu_app::selection::{SelAction, SelEdit, SymOp};
use yolu_app::state::{Action, Tool};
use yolu_app::{Tab, YoluApp};

type H = Harness<'static, YoluApp>;

fn apply(h: &mut H, action: Action) {
    h.state_mut().state.apply(action);
    h.run();
}

/// 画面の右の方（ブラシの詳細の窓・プロパティの欄）にある、この名前の部品（同じ名前がほかにあっても、いちばん右のもの）。
fn rect_of_field(h: &H, label: &str) -> Rect {
    h.get_all_by_label(label)
        .map(|n| n.rect())
        .max_by(|a, b| a.left().total_cmp(&b.left()))
        .unwrap_or_else(|| panic!("{label} が画面に無い"))
}

fn is_disabled(h: &H, label: &str) -> bool {
    let target = rect_of_field(h, label);
    h.get_all_by_label(label)
        .find(|n| n.rect() == target)
        .unwrap()
        .accesskit_node()
        .is_disabled()
}

/// ポインタをその部品の上に置いて、ツールチップに（文字が）出るか。
fn tooltip_shows(h: &mut H, label: &str, tooltip: &str) -> bool {
    let at = rect_of_field(h, label).center();
    // 別の所から動かして、確かにその部品に入ったことにする
    move_to(h, egui::pos2(2.0, 2.0));
    h.run();
    hover_and_wait(h, at);
    let shown = h.query_by_label(tooltip).is_some();
    move_to(h, egui::pos2(2.0, 2.0));
    h.run();
    shown
}

// ───────── 対称 ─────────

fn symmetry_app(lang: Lang) -> H {
    // プロパティの欄が縦に収まる高さ
    let mut h = app(1280.0, 2400.0, 256);
    h.state_mut().state.lang = lang;
    h.run();
    apply(
        &mut h,
        Action::Sel(SelAction::Symmetry(SymOp::Mode(SymmetryMode::Radial))),
    );
    // 中心を動かしておく（「キャンバスの中心」は中心がずれているときだけ押せる）
    apply(
        &mut h,
        Action::Sel(SelAction::Symmetry(SymOp::Center(0.25, 0.75))),
    );
    // 対称の欄は、ブラシの詳細の窓の「対称」のカテゴリ
    let ui = &mut h.state_mut().state.brushes.ui;
    ui.detail.open = true;
    ui.detail.category = yolu_app::brushes::Category::Symmetry;
    ui.detail.scroll = 0.0;
    h.run();
    h
}

fn symmetry_labels(lang: Lang) -> [&'static str; 4] {
    lang.pick(
        ["中心 X", "写しの数", "キャンバスの中心", "軸を表示"],
        ["Center X", "Copies", "Canvas center", "Show axes"],
    )
}

fn assert_symmetry_fields(h: &H, lang: Lang, disabled: bool, what: &str) {
    for label in symmetry_labels(lang) {
        assert_eq!(is_disabled(h, label), disabled, "{lang:?} {what}: {label}");
    }
}

#[test]
fn the_symmetry_fields_are_disabled_with_a_reason_for_smudge_and_clone_and_come_back() {
    for lang in Lang::ALL {
        let mut h = symmetry_app(lang);
        assert_symmetry_fields(&h, lang, false, "ペイント");
        let reason = lang.pick("指先・クローンでは使えません", "Not with smudge or clone");
        for effect in [
            BrushEffect::Smudge { strength: 0.5 },
            BrushEffect::Clone {
                offset: DVec2::new(10.0, 0.0),
            },
        ] {
            h.state_mut().state.m2.brush.effect = effect;
            h.run();
            assert_symmetry_fields(&h, lang, true, "指先・クローン");
            // 理由は、無効にした部品のツールチップ（画面に注記の行は無い）
            for label in symmetry_labels(lang) {
                assert!(
                    tooltip_shows(&mut h, label, reason),
                    "{lang:?}: {label} のツールチップに理由"
                );
            }
            assert!(
                h.query_by_label(reason).is_none(),
                "{lang:?}: 注記の行は出さない"
            );
            // モードのボタンは押せる（対称を切る・替えるのは、指先・クローンのあいだもできる）
            assert!(!is_disabled(&h, lang.pick("放射状", "Radial")));
            // 効果をペイントに戻せば有効に戻り、理由は出なくなる
            h.state_mut().state.m2.brush.effect = BrushEffect::Paint;
            h.run();
            assert_symmetry_fields(&h, lang, false, "ペイントに戻した");
            assert!(
                !tooltip_shows(&mut h, symmetry_labels(lang)[0], reason),
                "{lang:?}: 理由は消える"
            );
        }
    }
}

/// 3D のタブだけが出ていて、描ける先が 3D の面だけのあいだは無効。キャンバスのタブへ戻せば有効に戻る。
#[test]
fn the_symmetry_fields_are_disabled_while_only_the_3d_view_can_be_painted() {
    for lang in Lang::ALL {
        let mut h = symmetry_app(lang);
        h.state_mut().state.view3d.load_demo();
        click_tab(&mut h, Tab::View3d);
        h.run();
        assert!(h.state().state.paints_only_in_3d(), "{lang:?}");
        assert_symmetry_fields(&h, lang, true, "3D だけ");
        let reason = lang.pick("3D では効きません", "No effect in 3D");
        for label in symmetry_labels(lang) {
            assert!(
                tooltip_shows(&mut h, label, reason),
                "{lang:?}: {label} のツールチップに理由"
            );
        }
        assert!(
            h.query_by_label(reason).is_none(),
            "{lang:?}: 注記の行は出さない"
        );
        click_tab(&mut h, Tab::Canvas);
        h.run();
        assert!(!h.state().state.paints_only_in_3d());
        assert_symmetry_fields(&h, lang, false, "キャンバスへ戻した");
    }
}

/// ドックを分けてキャンバスと 3D ビューが同時に出ているあいだは、2D にも描けるので、対称の欄は有効のまま（前は注記だけで操作できた）。
#[test]
fn the_symmetry_fields_stay_enabled_when_the_canvas_and_the_3d_view_are_side_by_side() {
    for lang in Lang::ALL {
        let mut h = symmetry_app(lang);
        h.state_mut().state.view3d.load_demo();
        let mut dock = DockState::new(vec![Tab::Canvas]);
        let surface = dock.main_surface_mut();
        let [_, view3d] = surface.split_right(NodeIndex::root(), 0.4, vec![Tab::View3d]);
        surface.split_right(view3d, 0.5, vec![Tab::Properties]);
        h.state_mut().dock = dock;
        h.run();
        assert!(h.state().view3d_rect().is_some(), "{lang:?}: 3D も出ている");
        assert!(
            h.state().state.ui.canvas_visible && h.state().state.view3d.paintable_on_screen(),
            "{lang:?}"
        );
        assert!(!h.state().state.paints_only_in_3d(), "{lang:?}");
        assert_symmetry_fields(&h, lang, false, "並べた");
        // 2D の対称の設定を変えられる
        let before = h.state().state.sel.symmetry.count;
        apply(
            &mut h,
            Action::Sel(SelAction::Symmetry(SymOp::Count(before + 1))),
        );
        assert_eq!(h.state().state.sel.symmetry.count, before + 1);
        // キャンバスを 3D の裏へ回すと、3D だけになって無効
        let mut stacked = DockState::new(vec![Tab::Canvas, Tab::View3d]);
        stacked
            .main_surface_mut()
            .split_right(NodeIndex::root(), 0.5, vec![Tab::Properties]);
        h.state_mut().dock = stacked;
        h.run();
        click_tab(&mut h, Tab::View3d);
        h.run();
        assert!(h.state().state.paints_only_in_3d(), "{lang:?}");
        assert_symmetry_fields(&h, lang, true, "重ねた");
    }
}

// ───────── 選択範囲を変更 ─────────

fn modify_labels(lang: Lang) -> [&'static str; 7] {
    lang.pick(
        [
            "半径",
            "端を固定",
            "拡張",
            "縮小",
            "境界線",
            "境界をぼかす",
            "境界をくっきり",
        ],
        [
            "Radius",
            "Edge lock",
            "Grow",
            "Shrink",
            "Border",
            "Feather",
            "Sharpen Edge",
        ],
    )
}

#[test]
fn the_selection_modify_fields_are_disabled_without_a_selection_with_the_reason_and_come_back() {
    for lang in Lang::ALL {
        // 選択の道具の設定の欄が上に付いたので、変更のボタンが全部見える高さにする
        let mut h = app(1280.0, 1000.0, 256);
        h.state_mut().state.lang = lang;
        apply(&mut h, Action::SelectTool(Tool::SelectRect));
        assert!(h.state().state.doc.selection().is_none());
        let reason = lang.pick("選択範囲なし", "No selection");
        for label in modify_labels(lang) {
            assert!(
                is_disabled(&h, label),
                "{lang:?}: 選択範囲が無いので {label} は無効"
            );
            assert!(
                tooltip_shows(&mut h, label, reason),
                "{lang:?}: {label} のツールチップに理由"
            );
        }
        assert!(
            h.query_by_label(reason).is_none(),
            "{lang:?}: 注記の行は出さない"
        );
        // 選択範囲を作れば有効に戻り、理由は消える
        apply(&mut h, Action::Sel(SelAction::Edit(SelEdit::All)));
        assert!(h.state().state.doc.selection().is_some());
        for label in modify_labels(lang) {
            assert!(
                !is_disabled(&h, label),
                "{lang:?}: 選択範囲があるので {label} は有効"
            );
            assert!(
                !tooltip_shows(&mut h, label, reason),
                "{lang:?}: {label} の理由は消える"
            );
        }
        // 解除すればまた無効
        apply(&mut h, Action::Sel(SelAction::Edit(SelEdit::Clear)));
        for label in modify_labels(lang) {
            assert!(
                is_disabled(&h, label),
                "{lang:?}: 解除したので {label} は無効"
            );
        }
    }
}

// ───────── 調整レイヤー ─────────

/// 色相・彩度の調整レイヤーを選んだ文書。描くチャンネルは `paint`。
fn adjustment_app(lang: Lang, paint: Channel) -> H {
    let mut h = app(1280.0, 1000.0, 256);
    h.state_mut().state.lang = lang;
    apply(
        &mut h,
        Action::M2(Edit::NewAdjustment(AdjustmentKind::HueSaturation)),
    );
    apply(&mut h, Action::M2Ui(UiOp::PaintChannel(paint)));
    h
}

fn adjustment_labels(lang: Lang) -> [&'static str; 3] {
    lang.pick(["色相", "彩度", "明度"], ["Hue", "Saturation", "Lightness"])
}

#[test]
fn the_hue_saturation_fields_say_why_they_do_not_reach_the_paint_channel_but_stay_editable() {
    for lang in Lang::ALL {
        let mut h = adjustment_app(lang, Channel::Roughness);
        let name = yolu_app::m2::channel_name(lang, &h.state().state.doc, Channel::Roughness);
        let reason = lang.pick(
            format!("{name} には効きません"),
            format!("No effect on {name}"),
        );
        for label in adjustment_labels(lang) {
            // 層の値はカラーのチャンネルの出力に効くので、描くチャンネルが違っても無効にはしない
            assert!(!is_disabled(&h, label), "{lang:?}: {label}");
            assert!(
                tooltip_shows(&mut h, label, &reason),
                "{lang:?}: {label} のツールチップに理由"
            );
        }
        assert!(
            h.query_by_label(&reason).is_none(),
            "{lang:?}: 注記の行は出さない"
        );
        // 直せる: 色相のスライダーの右の方を押すと、層の値が変わる
        let id = h.state().state.selected_layer.unwrap();
        let before = h
            .state()
            .state
            .doc
            .layer(id)
            .unwrap()
            .adjustment()
            .cloned()
            .unwrap();
        let row = rect_of_field(&h, lang.pick("色相", "Hue"));
        click(
            &mut h,
            egui::pos2(row.left() + row.width() * 0.8, row.bottom() - 3.0),
        );
        let after = h
            .state()
            .state
            .doc
            .layer(id)
            .unwrap()
            .adjustment()
            .cloned()
            .unwrap();
        assert_ne!(
            after.hue(),
            before.hue(),
            "{lang:?}: 描くチャンネルが違っても直せる"
        );
        // カラーのチャンネルに戻せば、理由は出ない
        apply(&mut h, Action::M2Ui(UiOp::PaintChannel(Channel::Color)));
        for label in adjustment_labels(lang) {
            assert!(!is_disabled(&h, label), "{lang:?}: {label}");
            assert!(
                !tooltip_shows(&mut h, label, &reason),
                "{lang:?}: {label} の理由は消える"
            );
        }
    }
}
