//! 効かない欄は注記の行を置かず、無効（灰色）にして理由をツールチップに出す。その 3 か所（対称定規の欄・ブラシの 2D だけの設定・調整レイヤー）が、
//! 無効のとき理由を出し、条件が外れたら有効に戻ることを確かめる。選択範囲を変更する操作は「選択範囲」メニューの項目で、選択範囲が無いあいだ無効になり戻ることを確かめる。無効にしてよい条件を狭く保つ試験も含む:
//! - 対称定規の欄は、読むだけのセットのあいだだけ無効（文書の値なので、ブラシの効果にも描ける先にも左右されない）
//! - ブラシの 2D だけの設定（手ぶれ補正・入り抜き・ゆらぎ・筆先の形・効果のブラシの値）も、描ける先が 3D だけのあいだだけ無効
//! - 調整レイヤーの欄は、描くチャンネルに効かなくても有効のまま（レイヤーの値は効くチャンネルの出力に効くので、直すためにチャンネルを替えさせない）
use crate::common;

use common::*;
use egui::Rect;
use egui_dock::{DockState, NodeIndex};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::engine::{BrushEffect, Channel};
use yolu_app::lang::Lang;
use yolu_app::m2::{AdjustmentKind, Edit, UiOp};
use yolu_app::selection::{SelAction, SelEdit};
use yolu_app::state::{Action, Tool};
use yolu_app::{Tab, YoluApp};

type H = Harness<'static, YoluApp>;

fn apply(h: &mut H, action: Action) {
    h.state_mut().state.apply(action);
    h.run();
}

/// 画面の右の方（ブラシの詳細のウィンドウ・プロパティの欄）にある、この名前の部品（同じ名前がほかにあっても、いちばん右のもの）。
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

/// その部品の上にポインタを置くと、この文が（読むだけのセットの理由が部品の名前になっているほかのボタンのほかに）新しく出るか。
fn tooltip_adds(h: &mut H, label: &str, text: &str) -> bool {
    let at = rect_of_field(h, label).center();
    move_to(h, egui::pos2(2.0, 2.0));
    h.run();
    let before = h.query_all_by_label(text).count();
    hover_and_wait(h, at);
    let after = h.query_all_by_label(text).count();
    move_to(h, egui::pos2(2.0, 2.0));
    h.run();
    after > before
}

// ───────── 対称定規の欄 ─────────

/// プロパティのレイヤーの欄に、選んでいるレイヤーの 2D の対称定規（線対称 6 本）の欄が出ている画面。
fn ruler_app(lang: Lang) -> H {
    // プロパティの欄が縦に収まる高さ
    let mut h = app(1280.0, 2400.0, 256);
    h.state_mut().state.lang = lang;
    {
        let s = &mut h.state_mut().state;
        // 中心をキャンバスの中心からずらしておく（「キャンバスの中心」は中心がずれているときだけ押せる）
        common::rulers::symmetry_2d(s, (64.0, 192.0), (1.0, 0.0), 6, true);
        s.ui.property_tab = 1;
    }
    h.run();
    h
}

fn ruler_labels(lang: Lang) -> [&'static str; 4] {
    lang.pick(
        ["線の本数", "中心 X", "角度", "キャンバスの中心"],
        ["Lines", "Center X", "Angle", "Canvas Center"],
    )
}

fn assert_ruler_fields(h: &H, lang: Lang, disabled: bool, what: &str) {
    for label in ruler_labels(lang) {
        assert_eq!(is_disabled(h, label), disabled, "{lang:?} {what}: {label}");
    }
}

/// 対称定規は文書の値なので、欄はブラシの効果（指先・クローン）にも、描ける先（2D・3D）にも左右されず、読むだけのセットのあいだだけ無効になる。
#[test]
fn the_ruler_fields_are_disabled_only_while_the_set_is_read_only() {
    for lang in Lang::ALL {
        let mut h = ruler_app(lang);
        assert_ruler_fields(&h, lang, false, "ふつう");
        // 指先・クローンでも欄は有効（効かないのはストロークを始めるとき）
        h.state_mut().state.m2.brush.effect = BrushEffect::Smudge { strength: 0.5 };
        h.run();
        assert_ruler_fields(&h, lang, false, "指先");
        h.state_mut().state.m2.brush.effect = BrushEffect::Paint;
        h.state_mut().state.sets.get_mut(0).unwrap().read_only = Some("テスト".into());
        h.run();
        assert_ruler_fields(&h, lang, true, "読むだけのセット");
        // 無効の欄には、ほかの欄と同じく理由のツールチップ（スライダー・トグル・ボタン）
        let reason = yolu_app::lang::refusals::read_only_set(lang, "テスト");
        for label in ruler_labels(lang) {
            assert!(
                tooltip_adds(&mut h, label, &reason),
                "{lang:?}: {label} のツールチップに理由"
            );
        }
        h.state_mut().state.sets.get_mut(0).unwrap().read_only = None;
        h.run();
        assert_ruler_fields(&h, lang, false, "戻した");
        for label in ruler_labels(lang) {
            assert!(
                !tooltip_adds(&mut h, label, &reason),
                "{lang:?}: {label} は戻したら理由を出さない"
            );
        }
    }
}

/// ドックを分けてキャンバスと 3D ビューを並べても、重ねて 3D だけにしても、対称定規の欄は有効のまま（どちらのビューにも効く文書の値）。
#[test]
fn the_ruler_fields_stay_enabled_when_the_canvas_and_the_3d_view_are_side_by_side_or_stacked() {
    for lang in Lang::ALL {
        let mut h = ruler_app(lang);
        h.state_mut().state.view3d.load_demo();
        let mut dock = DockState::new(vec![Tab::Canvas]);
        let surface = dock.main_surface_mut();
        let [_, view3d] = surface.split_right(NodeIndex::root(), 0.4, vec![Tab::View3d]);
        surface.split_right(view3d, 0.5, vec![Tab::Properties]);
        h.state_mut().dock = dock;
        h.run();
        assert!(h.state().view3d_rect().is_some(), "{lang:?}: 3D も出ている");
        assert!(!h.state().state.paints_only_in_3d(), "{lang:?}");
        assert_ruler_fields(&h, lang, false, "並べた");
        // キャンバスを 3D の裏へ回して 3D だけになっても、有効のまま
        let mut stacked = DockState::new(vec![Tab::Canvas, Tab::View3d]);
        stacked
            .main_surface_mut()
            .split_right(NodeIndex::root(), 0.5, vec![Tab::Properties]);
        h.state_mut().dock = stacked;
        h.run();
        click_tab(&mut h, Tab::View3d);
        h.run();
        assert!(h.state().state.paints_only_in_3d(), "{lang:?}");
        assert_ruler_fields(&h, lang, false, "重ねた");
    }
}

// ───────── ブラシの 2D だけの設定 ─────────

/// 左にサブツールとその下のツールプロパティ。`side_by_side` ならキャンバスと 3D ビューを並べ、そうでなければ同じ組に重ねて 3D ビューを前に出す。
fn brush_app(lang: Lang, side_by_side: bool) -> H {
    let mut h = app(1600.0, 1000.0, 128);
    h.state_mut().state.lang = lang;
    h.state_mut().state.view3d.load_demo();
    let mut dock = if side_by_side {
        DockState::new(vec![Tab::Canvas])
    } else {
        DockState::new(vec![Tab::Canvas, Tab::View3d])
    };
    let surface = dock.main_surface_mut();
    let [center, left] = surface.split_left(NodeIndex::root(), 0.25, vec![Tab::SubTools]);
    surface.split_below(left, 0.3, vec![Tab::ToolProperties]);
    if side_by_side {
        surface.split_right(center, 0.5, vec![Tab::View3d]);
    }
    h.state_mut().dock = dock;
    h.run();
    if !side_by_side {
        click_tab(&mut h, Tab::View3d);
        h.run();
    }
    h
}

fn open_brush_detail(h: &mut H, category: yolu_app::brushes::Category) {
    let ui = &mut h.state_mut().state.brushes.ui;
    ui.detail.open = true;
    ui.detail.category = category;
    ui.detail.scroll = 0.0;
    h.run();
}

fn close_brush_detail(h: &mut H) {
    h.state_mut().state.brushes.ui.detail.open = false;
    h.run();
}

/// ブラシの詳細のウィンドウの右側の欄の部品（左のカテゴリの同じ名前と区別する）の、場所と無効か。
fn pane_field(h: &H, label: &str) -> (Rect, bool) {
    let window = yolu_app::ui::window::last_rect(&h.ctx, yolu_app::panels::brush_detail::id())
        .expect("ブラシの詳細のウィンドウを描いている");
    let node = h
        .query_all_by_label(label)
        .find(|n| window.contains(n.rect().center()) && n.rect().left() > window.left() + 168.0)
        .unwrap_or_else(|| panic!("{label} がウィンドウの欄に無い"));
    (node.rect(), node.accesskit_node().is_disabled())
}

/// 欄が `disabled` どおりか。ポインタを置いたツールチップに理由が出るのは、無効のときだけ。
fn assert_brush_field(
    h: &mut H,
    lang: Lang,
    (rect, is_off): (Rect, bool),
    label: &str,
    reason: &str,
    disabled: bool,
) {
    assert_eq!(is_off, disabled, "{lang:?}: {label} の無効");
    move_to(h, egui::pos2(2.0, 2.0));
    h.run();
    hover_and_wait(h, rect.center());
    let shown = h.query_by_label(reason).is_some();
    move_to(h, egui::pos2(2.0, 2.0));
    h.run();
    assert_eq!(shown, disabled, "{lang:?}: {label} のツールチップの理由");
}

/// ツールプロパティ（手ぶれ補正・効果のブラシの値）とブラシの詳細（形状・ストローク・入り抜き・ゆらぎ）の欄が有効で、前の「3D では
/// 効きません」の理由はツールチップにも出ない（3D のビューの面のダブも同じ式で使う）。
fn assert_brush_fields_enabled(h: &mut H, lang: Lang) {
    use yolu_app::brushes::Category;
    let effect_reason = lang.pick("3D では使えません", "Not available in 3D");
    let reason = lang.pick("3D では効きません", "No effect in 3D");
    close_brush_detail(h);
    let stabilizer = lang.pick("手ぶれ補正", "Stabilizer");
    let node = h.get_by_label(stabilizer);
    let found = (node.rect(), node.accesskit_node().is_disabled());
    assert_brush_field(h, lang, found, stabilizer, reason, false);
    // 効果のブラシ（ぼかし）の値
    h.state_mut().state.m2.brush.effect = BrushEffect::BLUR;
    h.run();
    let blur = lang.pick("ぼかしの半径", "Blur radius");
    let node = h.get_by_label(blur);
    let found = (node.rect(), node.accesskit_node().is_disabled());
    assert_brush_field(h, lang, found, blur, effect_reason, false);
    h.state_mut().state.m2.brush.effect = BrushEffect::Paint;
    h.run();
    for (category, labels) in [
        (
            Category::Shape,
            lang.pick(["真円率", "角度"], ["Roundness", "Angle"]),
        ),
        (
            Category::Stroke,
            lang.pick(["手ぶれ補正", "曲線"], ["Stabilizer", "Curve"]),
        ),
        (
            Category::Dynamics,
            lang.pick(["入り", "抜き"], ["Taper in", "Taper out"]),
        ),
        (
            Category::Jitter,
            lang.pick(["サイズ", "散布"], ["Size", "Scatter"]),
        ),
    ] {
        open_brush_detail(h, category);
        for label in labels {
            let found = pane_field(h, label);
            assert_brush_field(h, lang, found, label, reason, false);
        }
    }
}

/// ドックを分けてキャンバスと 3D ビューが同時に出ているあいだも、手ぶれ補正などの欄は有効（理由のツールチップも出ない）。
#[test]
fn the_brush_fields_stay_enabled_when_the_canvas_and_the_3d_view_are_side_by_side() {
    for lang in Lang::ALL {
        let mut h = brush_app(lang, true);
        assert!(h.state().view3d_rect().is_some(), "{lang:?}: 3D も出ている");
        assert!(
            h.state().state.ui.canvas_visible && h.state().state.view3d.paintable_on_screen(),
            "{lang:?}"
        );
        assert!(!h.state().state.paints_only_in_3d(), "{lang:?}");
        assert_brush_fields_enabled(&mut h, lang);
    }
}

/// 3D のタブだけが出ていて、描ける先が 3D の面だけのあいだも、筆先・ストローク・入り抜き・ゆらぎ・効果の値の欄は有効（3D の面のダブも
/// 2D と同じ式で使う）。
#[test]
fn the_brush_fields_stay_enabled_while_only_the_3d_view_can_be_painted() {
    for lang in Lang::ALL {
        let mut h = brush_app(lang, false);
        assert!(h.state().state.paints_only_in_3d(), "{lang:?}");
        assert_brush_fields_enabled(&mut h, lang);
    }
}

// ───────── 対称定規にスナップ（バケツ・自動選択） ─────────

/// バケツと自動選択の切り替えは、対称定規の写しが 2D のキャンバスだけに効くので、描ける先が 3D だけのあいだは押せず、短い理由を出す。
/// キャンバスが出ているあいだ（並べていても）は有効で、押すと入り切りできる。
#[test]
fn the_snap_to_symmetry_ruler_toggle_is_unavailable_only_while_just_the_3d_view_can_be_painted() {
    for lang in Lang::ALL {
        let label = lang.pick("対称定規にスナップ", "Snap to Symmetry Ruler");
        let reason = lang.pick("2D だけ", "2D only");
        for tool in [Tool::Fill, Tool::Wand] {
            let on = |h: &H| match tool {
                Tool::Fill => h.state().state.region.snap_symmetry,
                _ => h.state().state.sel.snap_symmetry,
            };
            // 並べて出している: 有効。押すと切り替わる
            let mut h = brush_app(lang, true);
            apply(&mut h, Action::SelectTool(tool));
            assert!(!h.state().state.paints_only_in_3d(), "{lang:?} {tool:?}");
            assert!(on(&h), "{lang:?} {tool:?}: 既定は入");
            assert!(!is_disabled(&h, label), "{lang:?} {tool:?}");
            assert!(!tooltip_shows(&mut h, label, reason), "{lang:?} {tool:?}");
            let at = rect_of_field(&h, label).center();
            click(&mut h, at);
            assert!(!on(&h), "{lang:?} {tool:?}: 押すと切");
            click(&mut h, at);
            assert!(on(&h), "{lang:?} {tool:?}: もう一度押すと入");
            // 3D だけ: 押せず、押しても変わらない。ポインタを置くと理由
            let mut h = brush_app(lang, false);
            apply(&mut h, Action::SelectTool(tool));
            assert!(h.state().state.paints_only_in_3d(), "{lang:?} {tool:?}");
            assert!(is_disabled(&h, label), "{lang:?} {tool:?}");
            assert!(tooltip_shows(&mut h, label, reason), "{lang:?} {tool:?}");
            let at = rect_of_field(&h, label).center();
            click(&mut h, at);
            assert!(on(&h), "{lang:?} {tool:?}: 3D だけでは押しても変わらない");
        }
    }
}

/// バケツの切り替えは、塗り残し（なぞり塗り）が入のときは効かないので押せず、短い理由を出す。切ると押せる。
#[test]
fn the_bucket_snap_to_symmetry_ruler_is_unavailable_while_paint_unfilled_areas_is_on() {
    for lang in Lang::ALL {
        let label = lang.pick("対称定規にスナップ", "Snap to Symmetry Ruler");
        let reason = lang.pick("塗り残しでは効きません", "Not used with leftover fill");
        let mut h = brush_app(lang, true);
        apply(&mut h, Action::SelectTool(Tool::Fill));
        h.state_mut().state.region.by_color = true;
        h.run();
        assert!(
            !is_disabled(&h, label),
            "{lang:?}: 塗り残しが切のあいだは有効"
        );
        assert!(!tooltip_shows(&mut h, label, reason), "{lang:?}");
        h.state_mut().state.region.color.leftovers = true;
        h.run();
        assert!(
            is_disabled(&h, label),
            "{lang:?}: 塗り残しが入のあいだは押せない"
        );
        assert!(tooltip_shows(&mut h, label, reason), "{lang:?}");
        h.state_mut().state.region.color.leftovers = false;
        h.run();
        assert!(!is_disabled(&h, label), "{lang:?}: 切ると戻る");
    }
}

// ───────── 選択範囲を変更（「選択範囲」メニューの項目） ─────────
// ツールプロパティには置かない。同じ操作は「選択範囲」メニューにあり、選択範囲が無いあいだは項目が灰色になる。

fn modify_labels(lang: Lang) -> [&'static str; 5] {
    lang.pick(
        [
            "拡張…",
            "縮小…",
            "境界線…",
            "境界をぼかす…",
            "境界をくっきり",
        ],
        ["Grow…", "Shrink…", "Border…", "Feather…", "Sharpen Edge"],
    )
}

/// 「選択範囲」メニューを開き、項目が無効かを順に返して閉じる。
fn menu_items_disabled(h: &mut H, lang: Lang) -> Vec<bool> {
    let title = menu_title(h, lang.pick("選択範囲", "Select")).center();
    click(h, title);
    let flags = modify_labels(lang)
        .into_iter()
        .map(|label| {
            let target = popup_item(h, label);
            h.get_all_by_label(label)
                .find(|n| n.rect() == target)
                .unwrap()
                .accesskit_node()
                .is_disabled()
        })
        .collect();
    key(h, egui::Key::Escape, egui::Modifiers::NONE);
    h.run();
    flags
}

#[test]
fn the_selection_modify_menu_items_are_disabled_without_a_selection_and_come_back() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 800.0, 256);
        h.state_mut().state.lang = lang;
        apply(&mut h, Action::SelectTool(Tool::SelectRect));
        assert!(h.state().state.doc.selection().is_none());
        assert_eq!(
            menu_items_disabled(&mut h, lang),
            vec![true; 5],
            "{lang:?}: 選択範囲が無いので無効"
        );
        // ツールプロパティには置かない
        for label in lang.pick(["半径", "端を固定"], ["Radius", "Edge lock"]) {
            assert!(
                h.query_by_label(label).is_none(),
                "{lang:?}: ツールプロパティに {label} は無い"
            );
        }
        // 選択範囲を作れば有効に戻る
        apply(&mut h, Action::Sel(SelAction::Edit(SelEdit::All)));
        assert!(h.state().state.doc.selection().is_some());
        assert_eq!(
            menu_items_disabled(&mut h, lang),
            vec![false; 5],
            "{lang:?}: 選択範囲があるので有効"
        );
        // 解除すればまた無効
        apply(&mut h, Action::Sel(SelAction::Edit(SelEdit::Clear)));
        assert_eq!(
            menu_items_disabled(&mut h, lang),
            vec![true; 5],
            "{lang:?}: 解除したので無効"
        );
    }
}

// ───────── 調整レイヤー ─────────

/// 色相・彩度の調整レイヤーを選んだ文書。描くチャンネルは `paint`。
fn adjustment_app(lang: Lang, paint: Channel) -> H {
    // （調整レイヤーの値が全部入る高さで）
    let mut h = app(1280.0, 1600.0, 256);
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
            // レイヤーの値はカラーのチャンネルの出力に効くので、描くチャンネルが違っても無効にはしない
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
        // 直せる: 色相のスライダーの右の方を押すと、レイヤーの値が変わる
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

// ───────── 画像の筆先・消しゴム・ツールチップは名前とキー ─────────

fn image_tip() -> std::sync::Arc<yolu_core::BrushTip> {
    std::sync::Arc::new(yolu_core::BrushTip::new("試し", 2, 2, vec![0, 255, 255, 0]).unwrap())
}

/// 画像の筆先のとき、硬さは効かないので押せず、理由が出る（ツールプロパティ・ブラシの詳細の形状と筆圧の最小）。丸い筆先なら有効で、理由は出ない。
#[test]
fn hardness_says_it_has_no_effect_with_an_image_tip() {
    use yolu_app::brushes::Category;
    for lang in Lang::ALL {
        let reason = lang.pick("画像の筆先では効きません", "No effect on an image tip");
        let hardness = lang.pick("硬さ", "Hardness");
        let minimum = lang.pick("最小", "Minimum");
        // 丸い筆先: 有効・理由なし
        let mut h = brush_app(lang, true);
        assert!(!is_disabled(&h, hardness), "{lang:?}");
        assert!(!tooltip_shows(&mut h, hardness, reason), "{lang:?}");
        // 画像の筆先: ツールプロパティ
        h.state_mut().state.m2.brush.tip.image = Some(image_tip());
        h.run();
        assert!(is_disabled(&h, hardness), "{lang:?}");
        assert!(tooltip_shows(&mut h, hardness, reason), "{lang:?}");
        // ブラシの詳細の形状
        open_brush_detail(&mut h, Category::Shape);
        let found = pane_field(&h, hardness);
        assert_brush_field(&mut h, lang, found, hardness, reason, true);
        // 筆圧: 硬さの項目（最後）の最小は、画像の筆先が理由
        open_brush_detail(&mut h, Category::Pressure);
        // 硬さの項目は一番下なので、見えるところまで送る
        h.state_mut().state.brushes.ui.detail.scroll = 1000.0;
        h.run();
        let window = yolu_app::ui::window::last_rect(&h.ctx, yolu_app::panels::brush_detail::id())
            .expect("ブラシの詳細のウィンドウ");
        let last = h
            .query_all_by_label(minimum)
            .filter(|n| window.contains(n.rect().center()))
            .max_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
            .expect("最小の欄");
        let found = (last.rect(), last.accesskit_node().is_disabled());
        assert_brush_field(&mut h, lang, found, minimum, reason, true);
    }
}

/// 消しゴムのときのクローンの「全レイヤーから」「揃える」は、ずれの欄と同じく消しゴムが理由。
#[test]
fn the_clone_toggles_say_the_eraser_is_the_reason() {
    use yolu_app::brushes::Category;
    for lang in Lang::ALL {
        let reason = lang.pick("消しゴムでは使えません", "Not available with the eraser");
        let mut h = brush_app(lang, true);
        apply(&mut h, Action::SelectTool(Tool::Eraser));
        h.state_mut().state.m2.brush.effect = BrushEffect::Clone {
            offset: yolu_app::engine::DVec2::new(8.0, 0.0),
        };
        open_brush_detail(&mut h, Category::Effect);
        for label in [
            lang.pick("全レイヤーから", "All layers"),
            lang.pick("揃える", "Aligned"),
        ] {
            let found = pane_field(&h, label);
            assert_brush_field(&mut h, lang, found, label, reason, true);
        }
    }
}

/// 元が無いときの「揃える」の理由には、元を決める入力（組み合わせの表から）が付く。
#[test]
fn the_aligned_reason_names_the_key_that_sets_the_clone_source() {
    use yolu_app::brushes::Category;
    for lang in Lang::ALL {
        let reason = lang.pick(
            "元を決めると使える（Alt+クリック）",
            "Available once a source is set (Alt+Click)",
        );
        let mut h = brush_app(lang, true);
        h.state_mut().state.m2.brush.effect = BrushEffect::Clone {
            offset: yolu_app::engine::DVec2::new(8.0, 0.0),
        };
        open_brush_detail(&mut h, Category::Effect);
        let label = lang.pick("揃える", "Aligned");
        let found = pane_field(&h, label);
        assert_brush_field(&mut h, lang, found, label, reason, true);
    }
}

/// 値の欄のツールチップは名前とキーだけ（動きの説明は出ない）。選択の形のツールの縦横比・中心からには、修飾キーが付く。
#[test]
fn value_tooltips_are_the_name_and_the_key_only() {
    for lang in Lang::ALL {
        let mut h = brush_app(lang, true);
        // ブラシ: 流量・不透明度に説明は出ない
        for (label, old) in [
            (
                lang.pick("流量", "Flow"),
                lang.pick("ダブ 1 つが足す量", "How much each dab adds"),
            ),
            (
                lang.pick("不透明度", "Opacity"),
                lang.pick(
                    "1 本のストロークが覆える上限",
                    "The most one stroke can cover",
                ),
            ),
        ] {
            assert!(!tooltip_shows(&mut h, label, old), "{lang:?}: {label}");
        }
        // 選択の形のツール
        apply(&mut h, Action::SelectTool(Tool::SelectRect));
        for (label, tip) in [
            (
                lang.pick("縦横比を固定", "Fixed ratio"),
                lang.pick("縦横比を固定（Shift）", "Fixed ratio (Shift)"),
            ),
            (
                lang.pick("中心から", "From center"),
                lang.pick("中心から（Alt）", "From center (Alt)"),
            ),
        ] {
            assert!(tooltip_shows(&mut h, label, tip), "{lang:?}: {tip}");
        }
    }
}
