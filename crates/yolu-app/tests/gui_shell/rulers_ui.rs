//! 定規の画面（レイヤーの一覧の定規のアイコン・プロパティの「定規」の区分・定規のツールのツールプロパティ）。アイコンは押す・Shift＋押す・持って落とす・右クリック、
//! 区分は行ごとの表示・印・削除と選んだ行の欄。どれも「操作 → 文書の定規が変わる → 取り消しは 1 回」で確かめ、日英の絵も撮る（egui_kittest）。
use crate::common;

use common::*;
use egui::{pos2, Event, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::lang::Lang;
use yolu_app::rulers::RulerAction;
use yolu_app::state::{Action, AppState, Tool};
use yolu_app::YoluApp;
use yolu_core::glam::{DVec2, DVec3};
use yolu_core::{LayerId, RulerKind, RulerScope};

type H = Harness<'static, YoluApp>;

fn st(h: &H) -> &AppState {
    &h.state().state
}

fn name_of(lang: Lang) -> &'static str {
    lang.pick("定規", "Ruler")
}

/// 下のレイヤー `a`（選んでいる）と、その上のレイヤー `b`（名前は A・B）。文書は 64 × 64。
fn two_layers(lang: Lang) -> (H, LayerId, LayerId) {
    let mut h = app(1280.0, 900.0, 64);
    let (a, b) = {
        let s = &mut h.state_mut().state;
        s.lang = lang;
        let a = s.selected_layer.unwrap();
        s.doc.set_layer_name(a, "A").unwrap();
        let b = s.doc.add_layer("B").unwrap();
        s.selected_layer = Some(a);
        s.doc.clear_history().unwrap();
        (a, b)
    };
    h.run();
    (h, a, b)
}

/// レイヤーの一覧の中の、定規のアイコン（レイヤーごとに 1 つ）。上の行から順に。
fn icons(h: &H, lang: Lang) -> Vec<Rect> {
    let body = layers_body(h);
    let mut rects: Vec<Rect> = h
        .query_all_by_label(name_of(lang))
        .map(|n| n.rect())
        .filter(|r| body.contains(r.center()))
        .collect();
    rects.sort_by(|p, q| p.top().total_cmp(&q.top()));
    rects
}

/// レイヤーの行（名前の部品の矩形。行の全体）。
fn row_of(h: &H, name: &str) -> Rect {
    let body = layers_body(h);
    rect_of(h, name, |r| body.contains(r.center()))
}

fn shift_click(h: &mut H, at: Pos2) {
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    h.step();
    click(h, at);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.step();
}

fn right_click(h: &mut H, at: Pos2) {
    press(h, at, PointerButton::Secondary);
    h.step();
    release(h, at, PointerButton::Secondary);
    h.run();
}

fn rulers_of(h: &H, layer: LayerId) -> Vec<yolu_core::Ruler> {
    common::rulers::of(st(h), layer)
}

#[test]
fn only_a_layer_with_rulers_shows_the_icon_right_of_its_thumbnail_and_before_its_name() {
    for lang in Lang::ALL {
        let (mut h, a, _b) = two_layers(lang);
        assert!(
            icons(&h, lang).is_empty(),
            "定規が無ければアイコンも場所も無い"
        );
        common::rulers::symmetry_2d(&mut h.state_mut().state, (32.0, 32.0), (0.0, 1.0), 6, true);
        h.run();
        let found = icons(&h, lang);
        assert_eq!(found.len(), 1, "{lang:?}");
        let icon = found[0];
        let row = row_of(&h, "A");
        assert!(row.contains(icon.center()), "{lang:?}: {icon:?} {row:?}");
        assert_eq!((icon.width(), icon.height()), (20.0, 22.0));
        // 名前（レイヤーの名前の文字）は、アイコンの分だけ右へ（アイコンと重ならない）
        let b_row = row_of(&h, "B");
        assert!(!b_row.contains(icon.center()), "B の行には出ない");
        assert_eq!(rulers_of(&h, a).len(), 1);
    }
}

#[test]
fn a_plain_click_selects_the_row_and_opens_the_ruler_section_of_the_properties() {
    for lang in Lang::ALL {
        let (mut h, a, b) = two_layers(lang);
        common::rulers::symmetry_2d(&mut h.state_mut().state, (32.0, 32.0), (0.0, 1.0), 6, true);
        h.state_mut().state.selected_layer = Some(b);
        h.state_mut().state.ui.sections.insert("rulers", false);
        h.state_mut().state.ui.property_tab = 1;
        h.run();
        let at = icons(&h, lang)[0].center();
        click(&mut h, at);
        assert_eq!(st(&h).selected_layer, Some(a), "{lang:?}: 行を選ぶ");
        assert_eq!(st(&h).ui.sections.get("rulers"), Some(&true), "区分を開く");
        assert_eq!(
            st(&h).doc.undo_count(),
            1,
            "文書は変えない（定規を置いた 1 段だけ）"
        );
        // プロパティに「定規」の区分と、対称 6 の行
        let title = lang.pick("対称 6", "Symmetry 6");
        assert!(h.query_by_label(title).is_some(), "{lang:?}");
    }
}

#[test]
fn shift_click_hides_every_ruler_of_the_layer_and_again_shows_them_each_one_undo() {
    let (mut h, a, _b) = two_layers(Lang::Ja);
    {
        let s = &mut h.state_mut().state;
        common::rulers::line(s, (0.0, 20.0), (64.0, 20.0));
        common::rulers::symmetry_2d(s, (32.0, 32.0), (0.0, 1.0), 6, true);
    }
    h.run();
    let steps = st(&h).doc.undo_count();
    let at = icons(&h, Lang::Ja)[0].center();
    shift_click(&mut h, at);
    assert!(rulers_of(&h, a).iter().all(|r| !r.visible), "全部隠す");
    assert_eq!(st(&h).selected_layer, Some(a));
    assert_eq!(st(&h).doc.undo_count(), steps + 1, "1 回の取り消し");
    // 全部隠れているアイコンは薄い（押せる）。もう一度で全部出す
    let at = icons(&h, Lang::Ja)[0].center();
    shift_click(&mut h, at);
    assert!(rulers_of(&h, a).iter().all(|r| r.visible), "全部出す");
    assert_eq!(st(&h).doc.undo_count(), steps + 2);
    // 1 つだけ出ている状態では、全部隠す
    let mut list = rulers_of(&h, a);
    list[0].visible = false;
    h.state_mut().state.doc.set_rulers(a, list, false).unwrap();
    h.run();
    let at = icons(&h, Lang::Ja)[0].center();
    shift_click(&mut h, at);
    assert!(rulers_of(&h, a).iter().all(|r| !r.visible));
    // 取り消しで 1 回ずつ戻る
    h.state_mut().state.apply(Action::Undo);
    assert_eq!(rulers_of(&h, a).iter().filter(|r| r.visible).count(), 1);
    h.state_mut().state.apply(Action::Undo);
    h.state_mut().state.apply(Action::Undo);
    assert!(
        rulers_of(&h, a).iter().all(|r| !r.visible),
        "最初に隠した状態"
    );
}

#[test]
fn dragging_the_icon_to_another_row_moves_every_ruler_there_in_one_undo_step() {
    for lang in Lang::ALL {
        let (mut h, a, b) = two_layers(lang);
        {
            let s = &mut h.state_mut().state;
            common::rulers::line(s, (0.0, 20.0), (64.0, 20.0));
            common::rulers::symmetry_2d(s, (32.0, 32.0), (0.0, 1.0), 6, true);
        }
        h.run();
        let ids: Vec<_> = rulers_of(&h, a).iter().map(|r| r.id).collect();
        let steps = st(&h).doc.undo_count();
        let from = icons(&h, lang)[0].center();
        let target = row_of(&h, "B").center();
        // 同じ行へ落としても、何も起きない
        drag(&mut h, &[from, offset(from, 40.0, 0.0), from]);
        assert_eq!(st(&h).doc.undo_count(), steps, "{lang:?}");
        assert!(st(&h).rulers.icon_drag.is_none());
        // 持っている間は、落とせる行が分かる
        press(&h, from, PointerButton::Primary);
        h.step();
        for p in [offset(from, 0.0, -20.0), target] {
            move_to(&h, p);
            h.step();
        }
        assert_eq!(
            st(&h).rulers.icon_drag.map(|d| (d.over, d.can_drop)),
            Some((Some(b), true)),
            "{lang:?}: 行の上にいて、落とせる"
        );
        release(&h, target, PointerButton::Primary);
        h.run();
        assert_eq!(rulers_of(&h, a).len(), 0, "{lang:?}");
        assert_eq!(
            rulers_of(&h, b).iter().map(|r| r.id).collect::<Vec<_>>(),
            ids,
            "並びは元のまま、全部移る"
        );
        assert_eq!(st(&h).doc.undo_count(), steps + 1, "1 回の取り消し");
        // アイコンは移った先の行にある
        let found = icons(&h, lang);
        assert_eq!(found.len(), 1);
        assert!(row_of(&h, "B").contains(found[0].center()));
        h.state_mut().state.apply(Action::Undo);
        assert_eq!(rulers_of(&h, a).len(), 2);
        assert_eq!(rulers_of(&h, b).len(), 0);
    }
}

#[test]
fn dropping_where_the_target_would_hold_more_than_64_rulers_is_refused_with_a_reason() {
    let (mut h, a, b) = two_layers(Lang::Ja);
    {
        let s = &mut h.state_mut().state;
        for _ in 0..3 {
            common::rulers::line(s, (0.0, 20.0), (64.0, 20.0));
        }
        s.selected_layer = Some(b);
        let mut list = Vec::new();
        for i in 0..62 {
            let mut r = yolu_core::Ruler::canvas(
                s.doc.new_ruler_id(),
                RulerKind::Line,
                DVec2::new(0.0, i as f64),
                DVec2::new(64.0, i as f64),
            );
            r.visible = false;
            list.push(r);
        }
        s.doc.set_rulers(b, list, false).unwrap();
        s.selected_layer = Some(a);
        s.doc.clear_history().unwrap();
    }
    h.run();
    let a_row = row_of(&h, "A");
    let from = icons(&h, Lang::Ja)
        .into_iter()
        .find(|r| a_row.contains(r.center()))
        .expect("A の行のアイコン")
        .center();
    let target = row_of(&h, "B").center();
    // 持っている間、上限を超える行は光らせない（行の上にはいる）
    press(&h, from, PointerButton::Primary);
    h.step();
    for p in [offset(from, 0.0, -10.0), target] {
        move_to(&h, p);
        h.step();
    }
    assert_eq!(
        st(&h).rulers.icon_drag.map(|d| (d.over, d.can_drop)),
        Some((Some(b), false)),
        "上限を超える行は落とせない印"
    );
    release(&h, target, PointerButton::Primary);
    h.run();
    assert_eq!(rulers_of(&h, a).len(), 3, "断ったので何も動かない");
    assert_eq!(rulers_of(&h, b).len(), 62);
    assert!(!st(&h).doc.can_undo());
    assert!(st(&h).message.contains("64"), "{}", st(&h).message);
}

/// 部品（矩形で選ぶ）が無効か。
fn disabled_at(h: &H, label: &str, at: Rect) -> bool {
    use egui_kittest::kittest::NodeT;
    h.get_all_by_label(label)
        .find(|n| n.rect() == at)
        .unwrap_or_else(|| panic!("{label} が {at:?} に無い"))
        .accesskit_node()
        .is_disabled()
}

#[test]
fn the_delete_buttons_of_the_options_bar_and_the_tool_properties_agree_and_follow_the_selected_ruler_of_this_layer(
) {
    for lang in Lang::ALL {
        let (mut h, a, b) = two_layers(lang);
        h.state_mut().state.apply(Action::SelectTool(Tool::Ruler));
        h.run();
        let label = lang.pick("削除", "Delete");
        let both = |h: &H| -> (bool, bool) {
            let bar = bar_rect(h, label);
            let dock = dock_rect(h, label);
            (disabled_at(h, label, bar), disabled_at(h, label, dock))
        };
        assert_eq!(both(&h), (true, true), "{lang:?}: 消せる定規が無い");
        // 選んでいるレイヤーに定規を置くと、置いた定規が選ばれて、どちらも押せる
        common::rulers::line(&mut h.state_mut().state, (0.0, 20.0), (64.0, 20.0));
        h.run();
        assert_eq!(both(&h), (false, false), "{lang:?}: 選んでいる定規がある");
        // 別のレイヤーを選ぶと、前のレイヤーの定規を指す選びは外れ、どちらも押せない（画面の定規も無い）
        h.state_mut().state.select_layers([b], b);
        h.run();
        assert!(st(&h).rulers.selected.is_none(), "{lang:?}");
        assert_eq!(
            both(&h),
            (true, true),
            "{lang:?}: 別のレイヤーの定規は消せない"
        );
        assert_eq!(rulers_of(&h, a).len(), 1);
        // 戻って選び直せば、また押せる。押すと、そのレイヤーの選んだ定規が消える（1 回の取り消し）
        h.state_mut().state.select_layers([a], a);
        let id = rulers_of(&h, a)[0].id;
        h.state_mut()
            .state
            .apply(Action::Ruler(RulerAction::Select(Some((a, id)))));
        h.run();
        assert_eq!(both(&h), (false, false), "{lang:?}");
        let steps = st(&h).doc.undo_count();
        let at = bar_rect(&h, label).center();
        click(&mut h, at);
        h.run();
        assert!(rulers_of(&h, a).is_empty(), "{lang:?}");
        assert_eq!(st(&h).doc.undo_count(), steps + 1);
        assert_eq!(both(&h), (true, true), "{lang:?}: 消したあと");
    }
}

#[test]
fn a_row_of_a_read_only_set_is_not_lit_and_the_drop_is_refused_with_the_reason() {
    let (mut h, a, b) = two_layers(Lang::Ja);
    common::rulers::line(&mut h.state_mut().state, (0.0, 20.0), (64.0, 20.0));
    h.state_mut().state.doc.clear_history().unwrap();
    h.run();
    // アイコンを持ってから、セットが読むだけになった（持っている間に読み直しなどで変わる）
    let from = icons(&h, Lang::Ja)[0].center();
    let target = row_of(&h, "B").center();
    press(&h, from, PointerButton::Primary);
    h.step();
    for p in [offset(from, 0.0, -10.0), target] {
        move_to(&h, p);
        h.step();
    }
    assert_eq!(
        st(&h).rulers.icon_drag.map(|d| (d.over, d.can_drop)),
        Some((Some(b), true))
    );
    h.state_mut().state.sets.get_mut(0).unwrap().read_only = Some("テスト".into());
    move_to(&h, offset(target, 2.0, 0.0));
    h.step();
    assert_eq!(
        st(&h).rulers.icon_drag.map(|d| (d.over, d.can_drop)),
        Some((Some(b), false)),
        "読むだけになったら光らせない"
    );
    release(&h, target, PointerButton::Primary);
    h.run();
    assert_eq!(rulers_of(&h, a).len(), 1, "断ったので動かない");
    assert!(st(&h).message.contains("読むだけ"), "{}", st(&h).message);
}

#[test]
fn escape_and_losing_the_focus_drop_the_held_icon_without_moving_anything() {
    for way in ["Esc", "フォーカスの喪失"] {
        let (mut h, a, b) = two_layers(Lang::Ja);
        common::rulers::line(&mut h.state_mut().state, (0.0, 20.0), (64.0, 20.0));
        h.state_mut().state.doc.clear_history().unwrap();
        h.run();
        let from = icons(&h, Lang::Ja)[0].center();
        let target = row_of(&h, "B").center();
        press(&h, from, PointerButton::Primary);
        h.step();
        for p in [offset(from, 0.0, -10.0), target] {
            move_to(&h, p);
            h.step();
        }
        assert_eq!(
            st(&h).rulers.icon_drag.map(|d| (d.over, d.can_drop)),
            Some((Some(b), true)),
            "{way}: 持っている"
        );
        if way == "Esc" {
            key(&h, egui::Key::Escape, Modifiers::NONE);
        } else {
            h.event(Event::WindowFocused(false));
        }
        h.step();
        assert!(st(&h).rulers.icon_drag.is_none(), "{way}: 手放す");
        // そのまま離しても、移さない
        release(&h, target, PointerButton::Primary);
        h.run();
        assert_eq!(rulers_of(&h, a).len(), 1, "{way}");
        assert!(rulers_of(&h, b).is_empty(), "{way}");
        assert!(!st(&h).doc.can_undo(), "{way}: 取り消しの段は増えない");
        assert!(
            st(&h).doc.selection().is_none(),
            "{way}: Esc で選択を触らない"
        );
    }
}

#[test]
fn the_icon_menu_toggles_the_visibility_and_the_scope_and_deletes_in_one_undo_each() {
    for lang in Lang::ALL {
        let (mut h, a, _b) = two_layers(lang);
        {
            let s = &mut h.state_mut().state;
            common::rulers::line(s, (0.0, 20.0), (64.0, 20.0));
            common::rulers::symmetry_2d(s, (32.0, 32.0), (0.0, 1.0), 6, true);
        }
        h.run();
        let steps = st(&h).doc.undo_count();
        // 表示の範囲
        let at = icons(&h, lang)[0].center();
        right_click(&mut h, at);
        let item = popup_item(&h, lang.pick("表示の範囲", "Visible In")).center();
        click(&mut h, item);
        let item = popup_item(&h, lang.pick("すべてのレイヤー", "All Layers")).center();
        click(&mut h, item);
        assert!(
            rulers_of(&h, a)
                .iter()
                .all(|r| r.scope == RulerScope::All && r.visible),
            "{lang:?}"
        );
        assert_eq!(st(&h).doc.undo_count(), steps + 1);
        // 表示（チェックを外す）
        let at = icons(&h, lang)[0].center();
        right_click(&mut h, at);
        let item = popup_item(&h, lang.pick("表示", "Show")).center();
        click(&mut h, item);
        assert!(rulers_of(&h, a).iter().all(|r| !r.visible));
        assert_eq!(st(&h).doc.undo_count(), steps + 2);
        // 定規を削除
        let at = icons(&h, lang)[0].center();
        right_click(&mut h, at);
        let item = popup_item(&h, lang.pick("定規を削除", "Delete Rulers")).center();
        click(&mut h, item);
        assert!(rulers_of(&h, a).is_empty());
        assert_eq!(st(&h).doc.undo_count(), steps + 3);
        assert!(icons(&h, lang).is_empty(), "アイコンも消える");
        h.state_mut().state.apply(Action::Undo);
        assert_eq!(rulers_of(&h, a).len(), 2);
    }
}

/// 通しの操作: 対称定規（回転対称 6）をキャンバスのドラッグで置く → 描くと 6 つに写る → アイコンの `Shift` ＋クリックで隠すと写らない → 戻す →
/// 別のグループのレイヤーに描くと「同じグループの中」では写らない → 「すべてのレイヤー」にすると写る → 保存して開き直して同じ → 取り消しで 1 回ずつ戻る。
#[test]
fn a_symmetry_ruler_is_placed_by_dragging_mirrors_strokes_follows_the_layer_and_scope_and_survives_saving(
) {
    let mut h = app(1280.0, 1400.0, 64);
    let (l1, l2) = {
        let s = &mut h.state_mut().state;
        s.brush.radius = 1.5;
        s.brush.hardness = 1.0;
        s.brush.pressure_size = false;
        s.m2.random_seed = false;
        let l1 = s.selected_layer.unwrap();
        s.doc.set_layer_name(l1, "L1").unwrap();
        let l2 = s.doc.add_layer("L2").unwrap();
        s.doc.group_layers(&[l1], "G1").unwrap();
        s.doc.group_layers(&[l2], "G2").unwrap();
        s.selected_layer = Some(l1);
        s.doc.clear_history().unwrap();
        // 回転対称 6 本の対称定規を置く設定（キャンバスのドラッグで置く）
        s.apply(Action::SelectTool(Tool::Ruler));
        s.rulers.kind = RulerKind::Symmetry;
        s.rulers.set_line_symmetry(false);
        s.rulers.set_lines(6);
        (l1, l2)
    };
    h.run();
    let canvas_at = |h: &H, x: f64, y: f64| {
        let s = st(h);
        s.view
            .view(canvas_rect(h), s.doc.width(), s.doc.height())
            .to_screen(x, y)
    };
    let (center, toward) = (canvas_at(&h, 32.0, 32.0), canvas_at(&h, 44.0, 32.0));
    drag(&mut h, &[center, offset(center, 30.0, 0.0), toward]);
    let placed = rulers_of(&h, l1);
    assert_eq!(placed.len(), 1, "{}", st(&h).message);
    assert_eq!((placed[0].lines, placed[0].line_symmetry), (6, false));
    assert_eq!(st(&h).doc.undo_count(), 1, "置くのは 1 回の取り消し");
    // 描く（ブラシ）: 中心から右へ 20 の点が、60° ずつ回した 6 つの所に写る
    h.state_mut().state.apply(Action::SelectTool(Tool::Brush));
    h.run();
    let spots: Vec<(f64, f64)> = (0..6)
        .map(|k| {
            let t = std::f64::consts::TAU * f64::from(k) / 6.0;
            (32.0 + 20.0 * t.cos(), 32.0 + 20.0 * t.sin())
        })
        .collect();
    let painted = |h: &H| -> usize {
        spots
            .iter()
            .filter(|(x, y)| {
                yolu_app::engine::composite_pixel(&st(h).doc, x.round() as u32, y.round() as u32)[3]
                    > 0
            })
            .count()
    };
    let dab = |h: &mut H| {
        let at = canvas_at(h, spots[0].0, spots[0].1);
        drag(h, &[at, offset(at, 0.5, 0.0)]);
    };
    dab(&mut h);
    assert_eq!(painted(&h), 6, "6 つに写る");
    assert_eq!(st(&h).doc.undo_count(), 2, "写しも 1 回の取り消し");
    h.state_mut().state.apply(Action::Undo);
    h.run();
    // アイコンの Shift＋クリックで隠すと写らない（1 回の取り消し）
    let at = icons(&h, Lang::Ja)[0].center();
    shift_click(&mut h, at);
    assert!(rulers_of(&h, l1).iter().all(|r| !r.visible));
    dab(&mut h);
    assert_eq!(painted(&h), 1, "隠した定規は効かない");
    h.state_mut().state.apply(Action::Undo);
    h.run();
    let at = icons(&h, Lang::Ja)[0].center();
    shift_click(&mut h, at);
    dab(&mut h);
    assert_eq!(painted(&h), 6, "戻すと写る");
    h.state_mut().state.apply(Action::Undo);
    h.run();
    // 別のグループのレイヤー L2 を選んで描く: 「同じグループの中」（既定）なので、G1 の定規は見えず写らない
    assert_eq!(rulers_of(&h, l1)[0].scope, RulerScope::Group);
    h.state_mut().state.selected_layer = Some(l2);
    h.run();
    dab(&mut h);
    assert_eq!(painted(&h), 1, "別のグループのレイヤーには写らない");
    h.state_mut().state.apply(Action::Undo);
    h.run();
    // 「すべてのレイヤー」にすると、L2 にも写る（アイコンの右クリックのメニュー。L1 の行のアイコン）
    h.state_mut().state.selected_layer = Some(l1);
    h.run();
    let at = icons(&h, Lang::Ja)[0].center();
    right_click(&mut h, at);
    let item = popup_item(&h, "表示の範囲").center();
    click(&mut h, item);
    let item = popup_item(&h, "すべてのレイヤー").center();
    click(&mut h, item);
    assert_eq!(rulers_of(&h, l1)[0].scope, RulerScope::All);
    h.state_mut().state.selected_layer = Some(l2);
    h.run();
    dab(&mut h);
    assert_eq!(painted(&h), 6, "すべてのレイヤーなら L2 にも写る");
    // 保存して開き直しても、定規も絵も同じ
    let dir = crate::common::tmp::test_dir("rulers-flow");
    let path = dir.join("rulers.ylp");
    let expected = {
        let doc = &st(&h).doc;
        doc.composite(doc.bounds()).unwrap()
    };
    let steps = st(&h).doc.undo_count();
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(path.clone()));
    assert!(
        st(&h).message.starts_with("保存しました"),
        "{}",
        st(&h).message
    );
    let mut again = app(1280.0, 1400.0, 64);
    again
        .state_mut()
        .state
        .apply(Action::OpenProject(path.clone()));
    again.run();
    {
        let s = st(&again);
        assert_eq!(s.doc.composite(s.doc.bounds()).unwrap(), expected);
        let all: Vec<yolu_core::Ruler> = s
            .doc
            .layers()
            .iter()
            .flat_map(|l| l.rulers().to_vec())
            .collect();
        assert_eq!(all.len(), 1, "定規は 1 つ");
        assert_eq!((all[0].lines, all[0].scope), (6, RulerScope::All));
    }
    // 取り消しで 1 回ずつ戻る: 最後の絵 → 範囲の変更 → 表示 → 非表示 → 置いた定規（取り消し済みの絵は数えない）
    assert_eq!(steps, 5);
    h.state_mut().state.apply(Action::Undo);
    h.run();
    assert_eq!(painted(&h), 0, "最後の絵が戻った");
    h.state_mut().state.apply(Action::Undo);
    h.run();
    assert_eq!(
        rulers_of(&h, l1)[0].scope,
        RulerScope::Group,
        "範囲の変更が戻った"
    );
    h.state_mut().state.apply(Action::Undo);
    h.run();
    assert!(
        rulers_of(&h, l1).iter().all(|r| !r.visible),
        "表示が戻った（隠した状態）"
    );
    h.state_mut().state.apply(Action::Undo);
    h.run();
    assert!(
        rulers_of(&h, l1).iter().all(|r| r.visible),
        "非表示が戻った"
    );
    h.state_mut().state.apply(Action::Undo);
    h.run();
    assert!(rulers_of(&h, l1).is_empty(), "置いた定規が戻った");
    let _ = std::fs::remove_dir_all(dir);
}

fn props_of(lang: Lang) -> (H, LayerId) {
    // プロパティが縦に収まる高さ
    let mut h = app(1280.0, 2400.0, 64);
    let layer = {
        let s = &mut h.state_mut().state;
        s.lang = lang;
        s.ui.property_tab = 1;
        s.doc.clear_history().unwrap();
        s.selected_layer.unwrap()
    };
    h.run();
    (h, layer)
}

#[test]
fn the_ruler_section_lists_rows_with_eye_snap_mark_space_and_delete_and_each_edit_is_one_undo() {
    for lang in Lang::ALL {
        let (mut h, layer) = props_of(lang);
        let (line, sym2, sym3) = {
            let s = &mut h.state_mut().state;
            let line = common::rulers::line(s, (0.0, 20.0), (64.0, 20.0));
            let sym2 = common::rulers::symmetry_2d(s, (32.0, 32.0), (0.0, 1.0), 6, true);
            let sym3 = common::rulers::mirror_3d(s, DVec3::ZERO, DVec3::X);
            s.doc.clear_history().unwrap();
            (line, sym2, sym3)
        };
        h.run();
        let ruler = |h: &H, id| {
            rulers_of(h, layer)
                .into_iter()
                .find(|r| r.id == id)
                .unwrap()
        };
        // 行の名前（種類と主な値）と空間
        for title in [
            lang.pick("直線定規", "Straight Ruler"),
            lang.pick("対称 6", "Symmetry 6"),
            lang.pick("対称 2", "Symmetry 2"),
        ] {
            assert!(h.query_by_label(title).is_some(), "{lang:?}: {title}");
        }
        let texts = drawn_texts(&h);
        assert!(
            texts.iter().any(|t| t == "2D") && texts.iter().any(|t| t == "3D"),
            "{lang:?}"
        );
        // 作った物（3D の鏡）が選ばれていて、その欄（軸・中心）が出ている
        assert_eq!(st(&h).rulers.selected, Some((layer, sym3)));
        assert!(h
            .query_by_label(lang.pick("見えない面にも写す", "Paint Hidden Surfaces"))
            .is_some());
        // 目: 隠す・出す（1 回ずつ）
        let hide = lang.pick("非表示にする", "Hide");
        // レイヤーの行の目も同じ名前なので、プロパティの中のもの
        let props = props_body(&h);
        let eyes: Vec<Rect> = h
            .query_all_by_label(hide)
            .map(|n| n.rect())
            .filter(|r| props.contains(r.center()))
            .collect();
        assert_eq!(eyes.len(), 3, "{lang:?}: {eyes:?}");
        let first = eyes
            .iter()
            .min_by(|p, q| p.top().total_cmp(&q.top()))
            .unwrap();
        click(&mut h, first.center());
        assert!(!ruler(&h, line).visible, "{lang:?}");
        assert_eq!(st(&h).doc.undo_count(), 1);
        let show = h
            .get_by_label(lang.pick("表示する", "Show"))
            .rect()
            .center();
        click(&mut h, show);
        assert!(ruler(&h, line).visible);
        assert_eq!(st(&h).doc.undo_count(), 2);
        // 印: 2D の対称定規は作ったときに印が付き、3D の鏡も別に付く（2D と 3D は別に数える）
        assert!(ruler(&h, sym2).snap && ruler(&h, sym3).snap);
        let marks: Vec<Rect> = h
            .query_all_by_label(lang.pick("スナップする特殊定規", "Snapping Special Ruler"))
            .map(|n| n.rect())
            .collect();
        assert_eq!(marks.len(), 2, "特殊定規の行だけ（直線定規には無い）");
        // 印を押すと外れる（1 回）
        let top_mark = marks
            .iter()
            .min_by(|p, q| p.top().total_cmp(&q.top()))
            .unwrap();
        click(&mut h, top_mark.center());
        assert!(!ruler(&h, sym2).snap, "{lang:?}");
        assert_eq!(st(&h).doc.undo_count(), 3);
        // 削除: 行の削除（1 回）。選んでいた物なら選びも外れる
        let trash = lang.pick("定規を削除", "Delete Ruler");
        let last = h
            .query_all_by_label(trash)
            .map(|n| n.rect())
            .max_by(|p, q| p.top().total_cmp(&q.top()))
            .unwrap();
        click(&mut h, last.center());
        assert_eq!(rulers_of(&h, layer).len(), 2, "{lang:?}");
        assert_eq!(st(&h).rulers.selected, None);
        assert_eq!(st(&h).doc.undo_count(), 4);
        // 取り消しは 1 つずつ
        for _ in 0..4 {
            h.state_mut().state.apply(Action::Undo);
        }
        assert_eq!(rulers_of(&h, layer).len(), 3);
    }
}

#[test]
fn a_slider_drag_in_the_selected_ruler_fields_is_one_undo_step_and_scope_buttons_hide_and_show() {
    for lang in Lang::ALL {
        let (mut h, layer) = props_of(lang);
        let id = common::rulers::symmetry_2d(
            &mut h.state_mut().state,
            (32.0, 32.0),
            (0.0, 1.0),
            4,
            true,
        );
        h.state_mut().state.doc.clear_history().unwrap();
        h.run();
        // 線の本数のスライダー: 端から端までなぞっても 1 回の取り消し（線対称は偶数だけ）
        let lines = rect_of(&h, lang.pick("線の本数", "Lines"), |r| r.left() > rx());
        let start = pos2(lines.left() + 4.0, lines.center().y);
        let mid = pos2(lines.center().x, lines.center().y);
        let end = pos2(lines.right() - 4.0, lines.center().y);
        drag(&mut h, &[start, mid, end]);
        let r = rulers_of(&h, layer)
            .into_iter()
            .find(|r| r.id == id)
            .unwrap();
        assert_eq!(r.lines, 16, "{lang:?}");
        assert_eq!(
            st(&h).doc.undo_count(),
            1,
            "スライダー 1 回の操作が 1 回の取り消し"
        );
        h.state_mut().state.apply(Action::Undo);
        assert_eq!(rulers_of(&h, layer)[0].lines, 4);
        // 表示の範囲のボタン: 光っている範囲をもう一度押すと隠れ、隠れているときに押すとその範囲で出る
        let group = lang.pick("すべて", "All");
        // 同じ名前のロックの行（幅が広い）と見分ける
        let all_button = rect_of(&h, group, |r| r.left() > rx() && r.width() < 150.0);
        click(&mut h, all_button.center());
        let r = &rulers_of(&h, layer)[0];
        assert!(r.visible && r.scope == RulerScope::All, "{lang:?}");
        click(&mut h, all_button.center());
        assert!(
            !rulers_of(&h, layer)[0].visible,
            "光っているものをもう一度押すと全部切る"
        );
        click(&mut h, all_button.center());
        let r = &rulers_of(&h, layer)[0];
        assert!(r.visible && r.scope == RulerScope::All);
    }
}

fn shot(h: &mut H, rect: Rect, name: &str) {
    h.event(Event::PointerGone);
    h.step();
    let image = h.render().expect("描画");
    let cropped = image::imageops::crop_imm(
        &image,
        rect.left().floor() as u32,
        rect.top().floor() as u32,
        rect.width().ceil() as u32,
        rect.height().ceil() as u32,
    )
    .to_image();
    egui_kittest::image_snapshot(&cropped, name);
}

#[test]
fn snapshot_the_layer_list_with_ruler_icons_in_both_languages() {
    for lang in Lang::ALL {
        let (mut h, a, _b) = two_layers(lang);
        {
            let s = &mut h.state_mut().state;
            // A: 表示している定規、B: 全部隠した定規（薄いアイコン）、C: 定規の無いレイヤー
            common::rulers::line(s, (0.0, 20.0), (64.0, 20.0));
            common::rulers::symmetry_2d(s, (32.0, 32.0), (0.0, 1.0), 6, true);
            let hidden = s.doc.add_layer("C").unwrap();
            s.selected_layer = Some(hidden);
            common::rulers::line(s, (0.0, 40.0), (64.0, 40.0));
            let list: Vec<_> = s
                .doc
                .layer(hidden)
                .unwrap()
                .rulers()
                .iter()
                .cloned()
                .map(|mut r| {
                    r.visible = false;
                    r
                })
                .collect();
            s.doc.set_rulers(hidden, list, false).unwrap();
            s.selected_layer = Some(a);
        }
        h.run();
        let body = layers_body(&h);
        let rect = Rect::from_min_max(body.min, pos2(body.right(), body.top() + 130.0));
        shot(
            &mut h,
            rect,
            &format!("rulers_layer_list_{}", lang.pick("ja", "en")),
        );
    }
}

#[test]
fn snapshot_the_ruler_section_of_the_properties_for_2d_and_3d_in_both_languages() {
    for lang in Lang::ALL {
        for three_d in [false, true] {
            let (mut h, _layer) = props_of(lang);
            {
                let s = &mut h.state_mut().state;
                common::rulers::line(s, (0.0, 20.0), (64.0, 20.0));
                if three_d {
                    common::rulers::mirror_3d(s, DVec3::new(0.2, 0.0, 0.0), DVec3::X);
                } else {
                    common::rulers::symmetry_2d(s, (20.0, 40.0), (3.0, 4.0), 6, true);
                }
            }
            h.run();
            let body = props_body(&h);
            // 区分の終わり（2D は「キャンバスの中心」、3D は最後の「見えない面にも写す」）の下まで
            let last = match (three_d, lang) {
                (false, Lang::Ja) => "キャンバスの中心",
                (false, Lang::En) => "Canvas Center",
                (true, Lang::Ja) => "見えない面にも写す",
                (true, Lang::En) => "Paint Hidden Surfaces",
            };
            let end = rect_of(&h, last, |r| body.contains(r.center())).bottom() + 8.0;
            let rect = Rect::from_min_max(body.min, pos2(body.right(), end));
            shot(
                &mut h,
                rect,
                &format!(
                    "rulers_props_{}_{}",
                    if three_d { "3d" } else { "2d" },
                    lang.pick("ja", "en")
                ),
            );
        }
    }
}

#[test]
fn snapshot_the_ruler_tool_properties_in_both_languages() {
    for lang in Lang::ALL {
        let mut h = app(1600.0, 1000.0, 64);
        {
            let s = &mut h.state_mut().state;
            s.lang = lang;
            s.apply(Action::SelectTool(Tool::Ruler));
            s.rulers.kind = RulerKind::Symmetry;
            s.rulers.set_lines(6);
            s.rulers.angle_step = true;
        }
        h.run();
        // 撮る枠は、ツールプロパティの組の幅（行いっぱいに広がる「削除」の右）と、区分の終わり（最後の「特殊定規にスナップ」の下）に合わせる
        let tab = h.state().tab_rects[&yolu_app::Tab::ToolProperties];
        let in_column = |r: Rect| r.center().x < 400.0 && r.top() > tab.bottom();
        let delete = rect_of(&h, lang.pick("削除", "Delete"), in_column);
        let last = rect_of(
            &h,
            lang.pick("特殊定規にスナップ", "Snap to Special Ruler"),
            in_column,
        );
        let rect = Rect::from_min_max(
            pos2(tab.left() - 2.0, tab.top()),
            pos2(delete.right() + 8.0, last.bottom() + 8.0),
        );
        shot(
            &mut h,
            rect,
            &format!("rulers_tool_props_{}", lang.pick("ja", "en")),
        );
    }
}

#[test]
fn snapshot_the_canvas_shows_the_effective_ruler_strong_and_the_other_special_ruler_faint() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 800.0, 64);
        {
            let s = &mut h.state_mut().state;
            s.lang = lang;
            s.sel.animate = false;
            common::rulers::add_canvas(
                s,
                RulerKind::Parallel,
                DVec2::new(0.0, 10.0),
                DVec2::new(64.0, 22.0),
            );
            // 後から作った対称定規が、スナップする特殊定規（平行線は印が外れて薄くなる）
            common::rulers::symmetry_2d(s, (32.0, 36.0), (2.0, 1.0), 6, true);
            common::rulers::line(s, (4.0, 56.0), (60.0, 52.0));
            s.apply(Action::SelectTool(Tool::Ruler));
        }
        h.run();
        let rect = canvas_rect(&h);
        shot(
            &mut h,
            rect,
            &format!("rulers_canvas_{}", lang.pick("ja", "en")),
        );
    }
}

/// いま描いた文字の全部。
fn drawn_texts(h: &H) -> Vec<String> {
    fn walk(shape: &egui::epaint::Shape, out: &mut Vec<String>) {
        match shape {
            egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            egui::epaint::Shape::Text(text) => out.push(text.galley.job.text.clone()),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        walk(&shape.shape, &mut out);
    }
    out
}

/// 描いた文字で、描いた範囲（クリップ）から横にはみ出しているもの。
fn clipped_texts(h: &H) -> Vec<String> {
    fn walk(shape: &egui::epaint::Shape, clip: Rect, out: &mut Vec<String>) {
        match shape {
            egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, clip, out)),
            egui::epaint::Shape::Text(text) => {
                let bounds = Rect::from_min_size(text.pos, text.galley.size());
                let visible = clip.y_range().contains(bounds.center().y);
                if visible
                    && (bounds.left() < clip.left() - 1.0 || bounds.right() > clip.right() + 1.0)
                {
                    out.push(format!(
                        "{}: {bounds:?} clip={clip:?}",
                        text.galley.job.text
                    ));
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        walk(&shape.shape, shape.clip_rect, &mut out);
    }
    out
}

#[test]
fn the_ruler_screens_have_no_instruction_text_and_the_english_ones_no_japanese() {
    let words_ja = ["クリックして", "ドラッグして", "押して", "ください"];
    let words_en = ["Click", "Drag", "Press", "Please"];
    for lang in Lang::ALL {
        let (mut h, layer) = props_of(lang);
        {
            let s = &mut h.state_mut().state;
            // 最初のレイヤーの名前は日本語なので、言語に合わせた名前にする（言語に関わらない中身）
            s.doc.set_layer_name(layer, "Base").unwrap();
            common::rulers::line(s, (0.0, 20.0), (64.0, 20.0));
            common::rulers::symmetry_2d(s, (32.0, 32.0), (0.0, 1.0), 6, true);
            common::rulers::mirror_3d(s, DVec3::ZERO, DVec3::X);
            s.apply(Action::SelectTool(Tool::Ruler));
        }
        h.run();
        let clipped = clipped_texts(&h);
        assert!(clipped.is_empty(), "{lang:?}: {clipped:#?}");
        let texts = drawn_texts(&h);
        for text in &texts {
            for w in words_ja.iter().chain(words_en.iter()) {
                assert!(!text.contains(w), "{lang:?}: 説明の文: {text}");
            }
            // 名称未設定とテクスチャセット 1 は、保存していないプロジェクト・最初のセットの名前（言語に関わらない中身）
            if lang == Lang::En && text != "名称未設定" && text != "テクスチャセット 1"
            {
                assert!(!has_japanese(text), "英語の画面に日本語: {text}");
            }
        }
    }
}
