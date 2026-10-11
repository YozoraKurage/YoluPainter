//! レイヤーの操作の画面（結合・複数選択・ロック・移動と変形のツール・90° 回転と反転）。どれも「操作 → 文書が変わる（1 回の Undo）→ 断られたら何も変わらず
//! 短い理由が日英で出る」を見る。`headless_` で始まる試験は画面を描かず、Wine でも回る。
use crate::common;

use common::*;
use egui::{pos2, Event, Key, Modifiers, PointerButton};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::engine::{BlendMode, Channel, Document, LayerId, LayerKind, Rgba8};
use yolu_app::lang::Lang;
use yolu_app::layerops::Xform;
use yolu_app::m2::{self, DropTarget, Edit};
use yolu_app::state::{Action, AppState, Tool};
use yolu_app::YoluApp;
use yolu_core::{LayerLocks, Surface};

const RED: Rgba8 = Rgba8::new(255, 0, 0, 255);
const GREEN: Rgba8 = Rgba8::new(0, 255, 0, 255);
const BLUE: Rgba8 = Rgba8::new(0, 0, 255, 255);

fn state(size: u32) -> AppState {
    AppState::new(size, size)
}

fn fill_rect(doc: &mut Document, id: LayerId, x0: u32, y0: u32, x1: u32, y1: u32, color: Rgba8) {
    for y in y0..y1 {
        for x in x0..x1 {
            doc.set_channel_pixel(id, Channel::Color, x, y, color)
                .unwrap();
        }
    }
}

/// 今の文書を履歴の起点にする（setup の塗りを取り消せる段にしない）。
fn settle(s: &mut AppState) {
    s.doc.clear_history().unwrap();
}

/// 新しいレイヤーを足して、そこに矩形を塗る（足したレイヤーを選ぶ）。
fn add_painted(s: &mut AppState, rect: (u32, u32, u32, u32), color: Rgba8) -> LayerId {
    s.apply(Action::NewLayer);
    let id = s.selected_layer.unwrap();
    fill_rect(&mut s.doc, id, rect.0, rect.1, rect.2, rect.3, color);
    id
}

fn pixel(s: &AppState, id: LayerId, x: u32, y: u32) -> [u8; 4] {
    s.doc
        .layer(id)
        .unwrap()
        .surface(Channel::Color)
        .and_then(|surface| surface.pixel(x, y).ok())
        .map(|p| p.to_array())
        .unwrap_or([0; 4])
}

fn color_bytes(s: &AppState, id: LayerId) -> Vec<u8> {
    s.doc
        .layer(id)
        .unwrap()
        .surface(Channel::Color)
        .map(Surface::to_canvas_bytes)
        .unwrap_or_default()
}

fn edit(s: &mut AppState, e: Edit) {
    s.apply(Action::M2(e));
}

fn undo(s: &mut AppState) {
    s.apply(Action::Undo);
}

fn ids(s: &AppState) -> Vec<LayerId> {
    s.doc.layers().iter().map(|l| l.id()).collect()
}

fn layer_rows(s: &AppState) -> Vec<m2::Row> {
    m2::visible_rows(&s.doc, &s.m2.collapsed)
}

fn rows(s: &AppState) -> Vec<LayerId> {
    layer_rows(s).iter().map(|r| r.id).collect()
}

/// 下から 3 つのレイヤー（赤・緑・青の矩形。重なる）。返すのは下から。
fn three(s: &mut AppState) -> [LayerId; 3] {
    let a = s.selected_layer.unwrap();
    fill_rect(&mut s.doc, a, 2, 2, 12, 12, RED);
    let b = add_painted(s, (6, 6, 16, 16), GREEN);
    let c = add_painted(s, (10, 10, 20, 20), BLUE);
    settle(s);
    [a, b, c]
}

// ───────── 複数選択 ─────────

#[test]
fn headless_ctrl_and_shift_select_layers_and_the_active_layer_stays_one() {
    let mut s = state(32);
    let [a, b, c] = three(&mut s);
    let d = add_painted(&mut s, (0, 0, 1, 1), RED);
    let all = rows(&s); // 上から d c b a
    assert_eq!(all, vec![d, c, b, a]);
    s.select_single_layer(a);
    assert_eq!(s.selected_layers(), vec![a]);
    // Ctrl: 足して、足したレイヤーが描く先
    s.toggle_layer_selected(c);
    assert_eq!(s.selected_layers(), vec![a, c]);
    assert_eq!(s.selected_layer, Some(c));
    // Ctrl: 選んでいるレイヤーを外す。描く先を外したら、残りのいちばん上
    s.toggle_layer_selected(c);
    assert_eq!(s.selected_layers(), vec![a]);
    assert_eq!(s.selected_layer, Some(a));
    // 最後の 1 つは外せない
    s.toggle_layer_selected(a);
    assert_eq!(s.selected_layers(), vec![a]);
    // Shift: 起点（最後に押したレイヤー）から押したレイヤーまでの行
    s.select_single_layer(a);
    s.select_layer_range(c, false, &all);
    assert_eq!(s.selected_layers(), vec![a, b, c]);
    assert_eq!(s.selected_layer, Some(c));
    // 起点は動かない: 別のレイヤーまで押し直すと範囲が替わる
    s.select_layer_range(b, false, &all);
    assert_eq!(s.selected_layers(), vec![a, b]);
    // Ctrl + Shift: 今の選択に範囲を足す
    s.select_single_layer(d);
    s.select_layer_range(c, true, &all);
    assert_eq!(s.selected_layers(), vec![c, d]);
    s.toggle_layer_selected(a);
    s.select_layer_range(b, true, &all);
    assert_eq!(s.selected_layers(), vec![a, b, c, d]);
    // 修飾キー無しのクリックは 1 つだけ
    s.select_single_layer(b);
    assert_eq!(s.selected_layers(), vec![b]);
}

#[test]
fn headless_a_multi_selection_collapses_when_the_active_layer_changes_elsewhere_and_forgets_dead_layers(
) {
    let mut s = state(32);
    let [a, b, c] = three(&mut s);
    s.select_layers([a, b, c], c);
    assert_eq!(s.selected_layers(), vec![a, b, c]);
    // ほかの所（新しいレイヤー）が描く先を替えると、集合はその 1 つに戻る
    s.apply(Action::NewLayer);
    let d = s.selected_layer.unwrap();
    assert_eq!(s.selected_layers(), vec![d]);
    // 描く先が同じなら、消えたレイヤーは集合から外れる
    s.select_layers([a, b, d], d);
    s.doc.remove_layer(b).unwrap();
    assert_eq!(s.selected_layers(), vec![a, d]);
    s.doc.remove_layer(a).unwrap();
    assert_eq!(s.selected_layers(), vec![d]);
    assert!(!s.has_multiple_layers_selected());
}

// ───────── 結合 ─────────

#[test]
fn headless_merge_down_makes_one_layer_with_the_same_look_and_one_undo() {
    let mut s = state(32);
    let [a, b, _c] = three(&mut s);
    s.select_single_layer(b);
    let look: Vec<[u8; 4]> = (0..24)
        .flat_map(|y| (0..24).map(move |x| (x, y)))
        .map(|(x, y)| yolu_app::engine::composite_pixel(&s.doc, x, y))
        .collect();
    let revision = s.doc.revision();
    edit(&mut s, Edit::MergeDown);
    assert_ne!(s.doc.revision(), revision);
    assert_eq!(s.doc.layers().len(), 2);
    // 下のレイヤーに重なって 1 枚になり、結果のレイヤーを選ぶ。見た目は変わらない
    let merged = s.selected_layer.unwrap();
    assert!(!ids(&s).contains(&b));
    assert!(s.doc.layers().iter().any(|l| l.id() == merged));
    let after: Vec<[u8; 4]> = (0..24)
        .flat_map(|y| (0..24).map(move |x| (x, y)))
        .map(|(x, y)| yolu_app::engine::composite_pixel(&s.doc, x, y))
        .collect();
    assert_eq!(after, look);
    assert_eq!(
        s.message,
        format!("結合しました: {}", s.doc.layer(merged).unwrap().name())
    );
    assert!(s.modified);
    // 1 回の Undo で、2 つのレイヤーが元のとおりに戻る
    undo(&mut s);
    assert_eq!(s.doc.layers().len(), 3);
    assert_eq!(pixel(&s, a, 3, 3), RED.to_array());
    assert_eq!(pixel(&s, b, 7, 7), GREEN.to_array());
    assert!(!s.doc.can_undo());
}

#[test]
fn headless_merge_refusals_say_why_in_both_languages_and_change_nothing() {
    for lang in Lang::ALL {
        let mut s = AppState::new_in(32, 32, lang);
        let [a, b, _c] = three(&mut s);
        // 一番下には、下のレイヤーが無い
        s.select_single_layer(a);
        let revision = s.doc.revision();
        edit(&mut s, Edit::MergeDown);
        assert_eq!(s.doc.revision(), revision);
        assert_eq!(s.doc.layers().len(), 3);
        assert_eq!(
            s.message,
            lang.pick(
                "結合できません（下にレイヤーが無い）。",
                "Cannot merge (No layer below)."
            ),
        );
        // 隠したレイヤーは結合しない
        s.apply(Action::ToggleVisible(b));
        s.select_single_layer(b);
        let revision = s.doc.revision();
        edit(&mut s, Edit::MergeDown);
        assert_eq!(s.doc.revision(), revision);
        assert_eq!(
            s.message,
            lang.pick(
                "結合できません（非表示のレイヤーがある）。",
                "Cannot merge (A layer is hidden)."
            ),
        );
        // グループは、中が空なら結合しない
        s.apply(Action::ToggleVisible(b));
        edit(&mut s, Edit::NewGroup);
        edit(&mut s, Edit::MergeDown);
        assert_eq!(
            s.message,
            lang.pick(
                "結合できません（グループが空）。",
                "Cannot merge (The group is empty)."
            ),
            "{lang:?}"
        );
        // 表示中のレイヤーが無い
        let all = ids(&s);
        for id in &all {
            if s.doc.layer(*id).unwrap().visible() {
                s.apply(Action::ToggleVisible(*id));
            }
        }
        edit(&mut s, Edit::MergeVisible);
        assert_eq!(
            s.message,
            lang.pick(
                "結合できません（表示中のレイヤーが無い）。",
                "Cannot merge (No visible layers)."
            ),
            "{lang:?}"
        );
    }
}

#[test]
fn headless_merging_a_selection_of_layers_or_a_group_or_the_visible_layers_is_one_undo_each() {
    // 複数選択（隣り合うレイヤー）を結合。間に別のレイヤーをはさむ選択は見た目が変わるので、結合する前に確かめる
    let mut s = state(32);
    let [a, b, c] = three(&mut s);
    s.select_layers([a, c], c);
    edit(&mut s, Edit::MergeDown);
    assert!(
        s.layer_ops.merge_confirm.is_some(),
        "間にレイヤーがあると見た目が変わる"
    );
    assert_eq!(s.doc.layers().len(), 3);
    edit(&mut s, Edit::CancelMerge);
    s.select_layers([b, c], c);
    edit(&mut s, Edit::MergeDown);
    assert_eq!(s.doc.layers().len(), 2, "{}", s.message);
    assert_eq!(s.selected_layers().len(), 1);
    undo(&mut s);
    assert_eq!(s.doc.layers().len(), 3);
    assert!(!s.doc.can_undo());
    // グループを選んで結合（中身ごと 1 枚）
    let mut s = state(32);
    let [a, b, _c] = three(&mut s);
    s.select_layers([a, b], b);
    edit(&mut s, Edit::GroupSelected);
    let group = s.selected_layer.unwrap();
    assert!(s.doc.layer(group).unwrap().is_group());
    assert_eq!(s.doc.layers().len(), 4);
    edit(&mut s, Edit::MergeDown);
    assert_eq!(s.doc.layers().len(), 2, "{}", s.message);
    assert!(s
        .doc
        .layers()
        .iter()
        .all(|l| !l.is_group() && l.kind() == LayerKind::Raster));
    undo(&mut s);
    assert_eq!(s.doc.layers().len(), 4);
    // 表示を結合（隠したレイヤーは残す）
    let mut s = state(32);
    let [a, b, c] = three(&mut s);
    s.apply(Action::ToggleVisible(b));
    settle(&mut s);
    edit(&mut s, Edit::MergeVisible);
    assert_eq!(s.doc.layers().len(), 2, "{}", s.message);
    assert!(ids(&s).contains(&b), "隠したレイヤーは残る");
    assert!(!ids(&s).contains(&a) && !ids(&s).contains(&c));
    undo(&mut s);
    assert_eq!(s.doc.layers().len(), 3);
}

/// 見た目が丸めの許容差を超えて変わる結合は、何も変えずに確かめを出す。「結合する」でそのまま結合し（1 回の Undo）、「やめる」で何も変えない。
#[test]
fn headless_a_merge_that_changes_the_look_asks_first() {
    for lang in Lang::ALL {
        let mut s = AppState::new_in(32, 32, lang);
        let a = s.selected_layer.unwrap();
        fill_rect(&mut s.doc, a, 0, 0, 32, 32, GREEN);
        let b = add_painted(&mut s, (4, 4, 20, 20), RED);
        s.doc.set_layer_blend_mode(b, BlendMode::Multiply).unwrap();
        let c = add_painted(&mut s, (8, 8, 24, 24), BLUE);
        settle(&mut s);
        s.select_single_layer(c);
        let revision = s.doc.revision();
        edit(&mut s, Edit::MergeDown);
        let confirm = s.layer_ops.merge_confirm.clone().expect("確かめが出る");
        assert!(!confirm.channels.is_empty() && confirm.channels.iter().all(|(_, n)| *n > 0));
        assert_eq!(s.doc.revision(), revision, "確かめの間は何も変えない");
        assert_eq!(s.doc.layers().len(), 3);
        // やめる
        edit(&mut s, Edit::CancelMerge);
        assert!(s.layer_ops.merge_confirm.is_none());
        assert_eq!(s.doc.revision(), revision);
        assert_eq!(
            s.message,
            lang.pick("結合をやめました。", "Merge cancelled.")
        );
        // もう一度、今度は結合する
        edit(&mut s, Edit::MergeDown);
        assert!(s.layer_ops.merge_confirm.is_some());
        edit(&mut s, Edit::ConfirmMerge);
        assert!(s.layer_ops.merge_confirm.is_none());
        assert_eq!(s.doc.layers().len(), 2, "{}", s.message);
        // 状態の文は結合したレイヤーの名前だけ（見た目が変わったのを承知した結合でも、画素数・差の数は載せない）
        let merged = s
            .doc
            .layer(s.selected_layer.unwrap())
            .unwrap()
            .name()
            .to_owned();
        assert_eq!(
            s.message,
            lang.pick(
                format!("結合しました: {merged}"),
                format!("Merged into {merged}")
            )
        );
        undo(&mut s);
        assert_eq!(s.doc.layers().len(), 3);
        assert!(!s.doc.can_undo());
    }
}

// ───────── 選んだレイヤーへの操作 ─────────

#[test]
fn headless_delete_group_duplicate_visibility_and_step_act_on_every_selected_layer_in_one_undo() {
    let mut s = state(32);
    let [a, b, c] = three(&mut s);
    let d = add_painted(&mut s, (0, 0, 2, 2), RED);
    settle(&mut s);
    // 複製（複製を選ぶ）
    s.select_layers([b, c], c);
    edit(&mut s, Edit::DuplicateSelected);
    assert_eq!(s.doc.layers().len(), 6);
    assert_eq!(s.selected_layers().len(), 2);
    undo(&mut s);
    assert_eq!(s.doc.layers().len(), 4);
    // 表示の切り替え: どれか見えていれば全部隠し、全部隠れていれば全部見せる
    s.select_layers([a, c], c);
    edit(&mut s, Edit::ToggleSelectedVisible);
    assert!(!s.doc.layer(a).unwrap().visible() && !s.doc.layer(c).unwrap().visible());
    assert!(s.doc.layer(b).unwrap().visible());
    edit(&mut s, Edit::ToggleSelectedVisible);
    assert!(s.doc.layer(a).unwrap().visible() && s.doc.layer(c).unwrap().visible());
    undo(&mut s);
    undo(&mut s);
    assert!(!s.doc.can_undo());
    // 1 段上へ（選んだレイヤーどうしは追い越さない）
    s.select_layers([a, b], b);
    s.apply(Action::LayerUp);
    assert_eq!(ids(&s), vec![c, a, b, d]);
    undo(&mut s);
    // グループにまとめる
    s.select_layers([a, b], b);
    edit(&mut s, Edit::GroupSelected);
    let group = s.selected_layer.unwrap();
    assert_eq!(s.doc.layer(a).unwrap().parent(), Some(group));
    assert_eq!(s.doc.layer(b).unwrap().parent(), Some(group));
    undo(&mut s);
    // 削除（まとめて 1 回の Undo。選ぶのは消した塊のすぐ下）
    s.select_layers([b, c], c);
    s.apply(Action::DeleteLayer);
    assert_eq!(ids(&s), vec![a, d]);
    assert_eq!(s.selected_layer, Some(a));
    undo(&mut s);
    assert_eq!(s.doc.layers().len(), 4);
    assert_eq!(pixel(&s, c, 11, 11), BLUE.to_array());
    // 全部は消さない
    s.select_layers([a, b, c, d], d);
    let revision = s.doc.revision();
    s.apply(Action::DeleteLayer);
    assert_eq!(s.doc.revision(), revision);
    assert_eq!(s.doc.layers().len(), 4);
    assert_eq!(s.message, "最後のレイヤーは消せません。");
}

#[test]
fn headless_a_new_group_next_to_a_layer_in_the_deepest_group_is_refused_with_a_short_reason() {
    for lang in [Lang::Ja, Lang::En] {
        let mut s = state(32);
        s.lang = lang;
        let mut top = s.selected_layer.unwrap();
        let leaf = top;
        for i in 0..yolu_core::MAX_GROUP_DEPTH {
            top = s.doc.group_layers(&[top], &format!("g{i}")).unwrap();
        }
        settle(&mut s);
        s.select_layers([leaf], leaf);
        let (layers, revision) = (s.doc.layers().len(), s.doc.revision());
        edit(&mut s, Edit::NewGroup);
        assert_eq!(
            (s.doc.layers().len(), s.doc.revision(), s.doc.undo_count()),
            (layers, revision, 0),
            "{lang:?}: 断ったら何も変えない"
        );
        s.doc.validate_structure().unwrap();
        assert!(
            s.message
                .contains(lang.pick("入れ子が深すぎる", "nesting is too deep")),
            "{lang:?}: {}",
            s.message
        );
        // 一番外側のグループの隣へなら足せる（鎖は増えない）
        s.select_layers([top], top);
        edit(&mut s, Edit::NewGroup);
        assert_eq!(s.doc.layers().len(), layers + 1, "{lang:?}: {}", s.message);
    }
}

#[test]
fn headless_dragging_a_selection_of_layers_moves_them_together_around_the_layers_that_stay() {
    let mut s = state(32);
    let [a, b, c] = three(&mut s);
    let d = add_painted(&mut s, (0, 0, 2, 2), RED);
    settle(&mut s);
    edit(&mut s, Edit::NewGroup);
    let group = s.selected_layer.unwrap();
    settle(&mut s);
    // 上から g d c b a。a と c を選んで、b と c の間 → 運ばれない最初のレイヤー（b）の上へ
    let list = layer_rows(&s);
    assert_eq!(
        list.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![group, d, c, b, a]
    );
    s.select_layers([a, c], c);
    // 線 3（b の行のすぐ上）。運ばれるのは a と c で、運ばれない最初の行は b
    let moved = m2::drop_edit_for(&s.doc, &list, &[a, c], DropTarget::Gap(3)).unwrap();
    edit(&mut s, moved);
    assert_eq!(ids(&s), vec![b, a, c, d, group]);
    undo(&mut s);
    assert_eq!(ids(&s), vec![a, b, c, d, group]);
    // グループの中へ（運ばれない子の数の位置）
    let into = m2::drop_edit_for(&s.doc, &list, &[a, c], DropTarget::Into(group)).unwrap();
    edit(&mut s, into);
    assert_eq!(s.doc.layer(a).unwrap().parent(), Some(group));
    assert_eq!(s.doc.layer(c).unwrap().parent(), Some(group));
    assert_eq!(s.doc.layer(b).unwrap().parent(), None);
    undo(&mut s);
    // 選んだグループの中へは落とせない・一番下へ
    s.select_layers([group, a], a);
    assert!(m2::drop_edit_for(&s.doc, &list, &[group, a], DropTarget::Into(group)).is_none());
    let bottom = m2::drop_edit_for(&s.doc, &list, &[a, c], DropTarget::Gap(list.len())).unwrap();
    edit(&mut s, bottom);
    assert_eq!(ids(&s)[..2], [a, c], "一番下へ（選んだ順を保つ）");
    // ドラッグの落とす先の判定も、運ぶレイヤーの全部で決める
    s.select_layers([a, c], c);
    let position = 2.5;
    let list = layer_rows(&s);
    assert!(m2::drop_target_for(&s.doc, &list, &[a, c], position).is_some());
}

// ───────── ロック ─────────

#[test]
fn headless_locks_toggle_on_every_selected_layer_in_one_undo_and_a_group_lock_reaches_its_contents()
{
    let mut s = state(32);
    let [a, b, c] = three(&mut s);
    s.select_layers([a, b], b);
    let both = s.selected_layers();
    edit(
        &mut s,
        Edit::Lock {
            ids: both,
            flag: LayerLocks::PIXELS,
            on: true,
        },
    );
    assert_eq!(s.doc.layer(a).unwrap().locks(), LayerLocks::PIXELS);
    assert_eq!(s.doc.layer(b).unwrap().locks(), LayerLocks::PIXELS);
    assert_eq!(s.doc.layer(c).unwrap().locks(), LayerLocks::NONE);
    assert!(s.message.contains("画素"), "{}", s.message);
    assert!(s.modified);
    undo(&mut s);
    assert_eq!(s.doc.layer(a).unwrap().locks(), LayerLocks::NONE);
    assert_eq!(s.doc.layer(b).unwrap().locks(), LayerLocks::NONE);
    assert!(!s.doc.can_undo(), "ロックの付け外しは 1 回の Undo");
    // 外す: 個別のロックだけ外し、ほかは残る
    edit(
        &mut s,
        Edit::Lock {
            ids: vec![a],
            flag: LayerLocks::POSITION | LayerLocks::PIXELS,
            on: true,
        },
    );
    edit(
        &mut s,
        Edit::Lock {
            ids: vec![a],
            flag: LayerLocks::PIXELS,
            on: false,
        },
    );
    assert_eq!(s.doc.layer(a).unwrap().locks(), LayerLocks::POSITION);
    // グループのロックは中身に効く（中身は自分のロックを持たない）
    s.select_layers([a, b], b);
    edit(&mut s, Edit::GroupSelected);
    let group = s.selected_layer.unwrap();
    edit(
        &mut s,
        Edit::Lock {
            ids: vec![group],
            flag: LayerLocks::ALL,
            on: true,
        },
    );
    assert_eq!(s.doc.layer(b).unwrap().locks(), LayerLocks::NONE);
    assert!(s
        .doc
        .effective_locks(b)
        .unwrap()
        .contains(LayerLocks::PIXELS | LayerLocks::ALL));
}

#[test]
fn headless_a_locked_layer_refuses_with_the_existing_short_reason_and_nothing_changes() {
    for lang in Lang::ALL {
        let mut s = AppState::new_in(32, 32, lang);
        let [a, b, c] = three(&mut s);
        s.doc.set_layer_locks(b, LayerLocks::POSITION).unwrap();
        s.select_single_layer(b);
        let before = color_bytes(&s, b);
        let revision = s.doc.revision();
        for x in [
            Xform::Move { dx: 1, dy: 0 },
            Xform::Flip { horizontal: true },
            Xform::Rotate90 { clockwise: true },
        ] {
            edit(&mut s, Edit::Transform(x));
            assert_eq!(s.doc.revision(), revision, "{x:?}");
            assert_eq!(color_bytes(&s, b), before, "{x:?}");
            assert_eq!(
                s.message,
                lang.pick(
                    "レイヤーの「位置」がロックされています",
                    "The layer has \"Position\" locked"
                ),
                "{x:?}"
            );
        }
        // 画素のロック: 整数画素の移動はできるが、回転・反転は画素を変えるので断る
        s.doc.set_layer_locks(b, LayerLocks::PIXELS).unwrap();
        s.message.clear();
        edit(&mut s, Edit::Transform(Xform::Move { dx: 2, dy: 0 }));
        assert!(
            s.message.contains(lang.pick("移動", "Moved")),
            "{}",
            s.message
        );
        assert_eq!(pixel(&s, b, 8, 8), GREEN.to_array());
        undo(&mut s);
        let revision = s.doc.revision();
        edit(&mut s, Edit::Transform(Xform::Flip { horizontal: true }));
        assert_eq!(s.doc.revision(), revision);
        // 結合も、画素のロックがあれば断る（どちらのレイヤーでも）
        s.select_single_layer(c);
        s.doc.set_layer_locks(b, LayerLocks::PIXELS).unwrap();
        edit(&mut s, Edit::MergeDown);
        assert_eq!(s.doc.layers().len(), 3);
        assert_eq!(s.doc.revision(), revision);
        let _ = a;
    }
}

/// グループのロックが効いているレイヤーは、親のグループのロックだと言い（持ち主はグループ）、レイヤーの自分のロックならそのレイヤーだと言う。
#[test]
fn headless_a_group_lock_is_named_as_the_parents_in_both_languages() {
    for lang in Lang::ALL {
        let mut s = AppState::new_in(32, 32, lang);
        let [a, b, _c] = three(&mut s);
        s.select_layers([a, b], b);
        edit(&mut s, Edit::GroupSelected);
        let group = s.selected_layer.unwrap();
        s.doc.set_layer_locks(group, LayerLocks::ALL).unwrap();
        settle(&mut s);
        s.select_single_layer(b);
        let revision = s.doc.revision();
        edit(&mut s, Edit::Transform(Xform::Move { dx: 1, dy: 0 }));
        assert_eq!(s.doc.revision(), revision);
        assert_eq!(
            s.message,
            lang.pick(
                "親グループの「すべて」がロックされています",
                "A parent group has \"All\" locked"
            )
        );
        // 結合も同じ言い方で断る
        edit(&mut s, Edit::MergeDown);
        assert_eq!(s.doc.revision(), revision);
        assert!(
            s.message
                .contains(lang.pick("親グループの", "A parent group has")),
            "{}",
            s.message
        );
        assert_eq!(s.message.is_ascii(), lang == Lang::En);
    }
}

/// 断りの文の名前は、錠の印・プロパティ・付け外しの状態の文と同じ表（`layerops::lock_name`）から出る。透明部分・画素・位置・すべてのそれぞれが、
/// 日英で決まった言い方になる（以前は断りだけ別の表で「ピクセル」「everything」と言っていた）。
#[test]
fn headless_every_kind_of_lock_is_named_the_same_way_in_a_refusal_and_in_the_lock_marks() {
    use yolu_app::layerops::{lock_name, LOCK_FLAGS};
    // (ロックの種類、日本語の名前、英語の名前)
    let cases: [(LayerLocks, &str, &str); 4] = [
        (LayerLocks::TRANSPARENCY, "透明部分", "Transparent pixels"),
        (LayerLocks::PIXELS, "画素", "Image pixels"),
        (LayerLocks::POSITION, "位置", "Position"),
        (LayerLocks::ALL, "すべて", "All"),
    ];
    for lang in Lang::ALL {
        for (flag, ja, en) in cases {
            let name = lang.pick(ja, en);
            assert_eq!(lock_name(lang, flag), name);
            let mut s = AppState::new_in(32, 32, lang);
            let [_a, b, c] = three(&mut s);
            s.doc.set_layer_locks(b, flag).unwrap();
            settle(&mut s);
            let revision = s.doc.revision();
            // 断られる操作はロックごとに違う（透明部分は下のレイヤーへの結合、画素は反転、位置・すべては移動）
            if flag == LayerLocks::TRANSPARENCY {
                s.select_single_layer(c);
                edit(&mut s, Edit::MergeDown);
            } else {
                s.select_single_layer(b);
                let x = if flag == LayerLocks::PIXELS {
                    Xform::Flip { horizontal: true }
                } else {
                    Xform::Move { dx: 1, dy: 0 }
                };
                edit(&mut s, Edit::Transform(x));
            }
            assert_eq!(s.doc.revision(), revision, "{flag:?}");
            assert_eq!(
                s.message,
                lang.pick(
                    format!("レイヤーの「{name}」がロックされています"),
                    format!("The layer has \"{name}\" locked"),
                ),
                "{flag:?} {lang:?}"
            );
            // 断りの文を作る口（ブラシ・バケツ・選択の編集も通る）も同じ名前
            let error = yolu_core::CoreError::LayerLocked {
                layer: b,
                holder: b,
                lock: flag,
            };
            assert!(lang.core_error(&error).contains(name), "{flag:?}");
        }
        // 表の順（透明部分・画素・位置・すべて）と、個別の複数は「、」・「, 」でつなぐ
        assert_eq!(
            LOCK_FLAGS.map(|f| lock_name(lang, f)),
            cases.map(|(_, ja, en)| lang.pick(ja, en))
        );
        let both = yolu_core::CoreError::LayerLocked {
            layer: LayerId(1),
            holder: LayerId(1),
            lock: LayerLocks::PIXELS | LayerLocks::POSITION,
        };
        assert!(
            lang.core_error(&both)
                .contains(lang.pick("「画素、位置」", "\"Image pixels, Position\"")),
            "{}",
            lang.core_error(&both)
        );
    }
}

// ───────── 変形 ─────────

#[test]
fn headless_move_flip_and_rotate_copy_pixels_exactly_with_one_undo_each() {
    let mut s = state(32);
    let id = s.selected_layer.unwrap();
    // 非対称の形（左が太い L 字）
    fill_rect(&mut s.doc, id, 4, 4, 8, 14, RED);
    fill_rect(&mut s.doc, id, 8, 4, 14, 6, BLUE);
    settle(&mut s);
    let original = color_bytes(&s, id);
    // 移動: 画素がそのまま (3, -2) だけ動く
    edit(&mut s, Edit::Transform(Xform::Move { dx: 3, dy: -2 }));
    assert_eq!(pixel(&s, id, 7, 2), RED.to_array());
    assert_eq!(pixel(&s, id, 4, 4), [0; 4]);
    undo(&mut s);
    assert_eq!(color_bytes(&s, id), original);
    assert!(!s.doc.can_undo());
    // 左右反転・上下反転は、動かすものの範囲の中で鏡像（範囲 x 4..14, y 4..14）
    edit(&mut s, Edit::Transform(Xform::Flip { horizontal: true }));
    assert_eq!(pixel(&s, id, 13, 13), RED.to_array());
    assert_eq!(pixel(&s, id, 4, 5), BLUE.to_array());
    edit(&mut s, Edit::Transform(Xform::Flip { horizontal: true }));
    assert_eq!(color_bytes(&s, id), original, "2 回で元のバイト");
    edit(&mut s, Edit::Transform(Xform::Flip { horizontal: false }));
    assert_eq!(pixel(&s, id, 4, 4), RED.to_array());
    assert_eq!(pixel(&s, id, 13, 13), BLUE.to_array());
    undo(&mut s);
    undo(&mut s);
    undo(&mut s);
    assert!(!s.doc.can_undo());
    // 90° 回転: 4 回で元のバイト。時計回りと反時計回りは逆
    for clockwise in [true, false] {
        for _ in 0..4 {
            edit(&mut s, Edit::Transform(Xform::Rotate90 { clockwise }));
        }
        assert_eq!(color_bytes(&s, id), original, "{clockwise}");
        for _ in 0..4 {
            undo(&mut s);
        }
        assert_eq!(color_bytes(&s, id), original);
    }
    edit(&mut s, Edit::Transform(Xform::Rotate90 { clockwise: true }));
    edit(
        &mut s,
        Edit::Transform(Xform::Rotate90 { clockwise: false }),
    );
    assert_eq!(
        color_bytes(&s, id),
        original,
        "時計回りと反時計回りは打ち消す"
    );
    // 時計回り: 左の縦棒（赤）は上の横棒に、下の横棒（青）は左の縦棒になる（範囲 4..14 の中心 9 のまわりの格子の写し (x, y) → (y, 17 - x)）
    undo(&mut s);
    undo(&mut s);
    edit(&mut s, Edit::Transform(Xform::Rotate90 { clockwise: true }));
    assert_eq!(pixel(&s, id, 5, 13), RED.to_array(), "左の赤い棒が上へ");
    assert_eq!(pixel(&s, id, 13, 13), RED.to_array());
    assert_eq!(pixel(&s, id, 5, 4), BLUE.to_array(), "下の青い棒が左へ");
    assert_eq!(pixel(&s, id, 13, 4), [0; 4]);
}

#[test]
fn headless_a_transform_moves_a_selection_with_its_pixels_and_every_selected_layer_and_a_groups_contents(
) {
    let mut s = state(32);
    let [a, b, c] = three(&mut s);
    // 選択範囲があれば、その中の画素と選択範囲が動く
    let mask = yolu_core::SelectionMask::rectangle(&s.doc, 2, 2, 7, 7);
    s.doc.set_selection(Some(mask)).unwrap();
    settle(&mut s);
    s.select_single_layer(a);
    assert_eq!(
        s.transform_bounds(),
        Some((2, 2, 7, 7)),
        "選択範囲の中の画素の外接"
    );
    edit(&mut s, Edit::Transform(Xform::Move { dx: 10, dy: 0 }));
    assert_eq!(pixel(&s, a, 12, 3), RED.to_array());
    assert_eq!(pixel(&s, a, 3, 3), [0; 4], "持ち上げた所は空く");
    assert_eq!(pixel(&s, a, 9, 9), RED.to_array(), "選択の外は動かない");
    let moved = s.doc.selection().unwrap();
    assert_eq!(moved.amount(12, 3), 255);
    assert_eq!(moved.amount(3, 3), 0);
    undo(&mut s);
    assert_eq!(pixel(&s, a, 3, 3), RED.to_array());
    assert_eq!(s.doc.selection().unwrap().amount(3, 3), 255);
    s.doc.set_selection(None).unwrap();
    settle(&mut s);
    // 複数選んでいれば、その全部を 1 回で
    s.select_layers([a, c], c);
    edit(&mut s, Edit::Transform(Xform::Move { dx: 0, dy: 3 }));
    assert_eq!(pixel(&s, a, 3, 6), RED.to_array());
    assert_eq!(pixel(&s, c, 11, 14), BLUE.to_array());
    assert_eq!(
        pixel(&s, b, 7, 7),
        GREEN.to_array(),
        "選んでいないレイヤーは動かない"
    );
    undo(&mut s);
    assert!(!s.doc.can_undo());
    // グループを選べば中身のラスターレイヤーが動く（塗りつぶしは動かさない）
    s.select_layers([a, b], b);
    edit(&mut s, Edit::GroupSelected);
    let group = s.selected_layer.unwrap();
    settle(&mut s);
    assert_eq!(s.transform_targets(), vec![a, b]);
    edit(&mut s, Edit::Transform(Xform::Move { dx: 1, dy: 0 }));
    assert_eq!(pixel(&s, a, 4, 3), RED.to_array());
    assert_eq!(pixel(&s, b, 8, 7), GREEN.to_array());
    assert_eq!(pixel(&s, c, 11, 11), BLUE.to_array());
    let _ = group;
    // 塗りつぶしだけを選んでいれば、動かすレイヤーが無い
    edit(&mut s, Edit::NewFill);
    let revision = s.doc.revision();
    edit(&mut s, Edit::Transform(Xform::Move { dx: 1, dy: 0 }));
    assert_eq!(s.doc.revision(), revision);
    assert_eq!(s.message, "動かす画素のあるレイヤーがありません。");
}

#[test]
fn headless_transform_messages_for_nothing_to_move_a_numeric_transform_and_the_budget_are_in_both_languages(
) {
    for lang in Lang::ALL {
        let mut s = AppState::new_in(32, 32, lang);
        // 画素の無いレイヤー
        let revision = s.doc.revision();
        edit(&mut s, Edit::Transform(Xform::Flip { horizontal: true }));
        assert_eq!(s.doc.revision(), revision);
        assert_eq!(
            s.message,
            lang.pick("動かす画素がありません。", "No pixels to move.")
        );
        let id = s.selected_layer.unwrap();
        fill_rect(&mut s.doc, id, 8, 8, 16, 16, RED);
        settle(&mut s);
        // 数値の変形: 200% で中心（12, 12）から広がる。0% は断る
        edit(
            &mut s,
            Edit::Transform(Xform::Numeric {
                dx: 0.0,
                dy: 0.0,
                degrees: 0.0,
                sx: 2.0,
                sy: 2.0,
            }),
        );
        assert_eq!(pixel(&s, id, 6, 6), RED.to_array());
        assert_eq!(pixel(&s, id, 17, 17), RED.to_array());
        assert_eq!(pixel(&s, id, 2, 2), [0; 4]);
        undo(&mut s);
        let revision = s.doc.revision();
        edit(
            &mut s,
            Edit::Transform(Xform::Numeric {
                dx: 0.0,
                dy: 0.0,
                degrees: 0.0,
                sx: 0.0,
                sy: 1.0,
            }),
        );
        assert_eq!(s.doc.revision(), revision);
        assert_eq!(
            s.message,
            lang.pick("拡大率は 0 にできません。", "Scale must not be 0%.")
        );
        // 一操作の予算が足りないとき（何も変えず、短い理由）
        s.doc.set_stroke_budget_bytes(64).unwrap();
        let before = color_bytes(&s, id);
        edit(&mut s, Edit::Transform(Xform::Move { dx: 1, dy: 1 }));
        assert_eq!(color_bytes(&s, id), before);
        assert_eq!(s.doc.revision(), revision);
        assert_eq!(
            s.message,
            lang.pick(
                "1 回の操作のメモリの予算を超えます（取り消しました）",
                "Over the memory budget of one operation (cancelled)"
            )
        );
        assert_eq!(s.message.is_ascii(), lang == Lang::En);
    }
}

/// 同じ変形を、同じ文書へ 1 スレッドと既定のスレッド数で当てたバイトは同じ（画面から使う経路）。
#[test]
fn headless_the_transform_is_the_same_bytes_with_one_thread_and_many() {
    let run = |threads: usize| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let mut s = state(256);
                let id = s.selected_layer.unwrap();
                for y in 0..256u32 {
                    for x in 0..256u32 {
                        if (x * 7 + y * 13) % 5 != 0 {
                            let v = (x * 31 + y * 17) as u8;
                            s.doc
                                .set_channel_pixel(
                                    id,
                                    Channel::Color,
                                    x,
                                    y,
                                    Rgba8::new(v, v.wrapping_mul(3), v.wrapping_add(90), v | 1),
                                )
                                .unwrap();
                        }
                    }
                }
                settle(&mut s);
                edit(&mut s, Edit::Transform(Xform::Rotate90 { clockwise: true }));
                edit(&mut s, Edit::Transform(Xform::Flip { horizontal: true }));
                edit(
                    &mut s,
                    Edit::Transform(Xform::Numeric {
                        dx: 3.0,
                        dy: -2.0,
                        degrees: 21.0,
                        sx: 1.4,
                        sy: 0.8,
                    }),
                );
                color_bytes(&s, id)
            })
    };
    let one = run(1);
    assert_eq!(one, run(3));
    assert_eq!(one, run(rayon::current_num_threads()));
}

// ───────── 移動・変形のツール（キャンバス） ─────────

fn with_layer_painted() -> (Harness<'static, YoluApp>, LayerId) {
    let mut h = app(1280.0, 800.0, 128);
    let id = h.state().state.selected_layer.unwrap();
    {
        let s = &mut h.state_mut().state;
        fill_rect(&mut s.doc, id, 30, 30, 70, 70, RED);
        settle(s);
        s.apply(Action::SelectTool(Tool::Move));
    }
    h.run();
    (h, id)
}

/// キャンバスの座標の点（画素の座標）の画面の位置。
fn screen_of(h: &Harness<'_, YoluApp>, x: f64, y: f64) -> egui::Pos2 {
    let app = h.state();
    let rect = canvas_rect(h);
    app.state
        .view
        .view(rect, app.state.doc.width(), app.state.doc.height())
        .to_screen(x, y)
}

#[test]
fn the_move_tool_drags_the_layer_by_whole_pixels_and_one_undo_puts_it_back() {
    let (mut h, id) = with_layer_painted();
    assert_eq!(h.state().state.tool, Tool::Move);
    let before = color_bytes(&h.state().state, id);
    let from = screen_of(&h, 50.0, 50.0);
    let to = screen_of(&h, 60.0, 44.0);
    drag(&mut h, &[from, offset(from, 3.0, 0.0), to]);
    let s = &h.state().state;
    assert_eq!(pixel(s, id, 40, 36), RED.to_array());
    assert_eq!(pixel(s, id, 30, 30), [0; 4]);
    assert!(s.modified);
    assert!(s.transform.drag.is_none());
    undo(&mut h.state_mut().state);
    assert_eq!(color_bytes(&h.state().state, id), before);
    assert!(!h.state().state.doc.can_undo(), "1 回の Undo で戻る");
}

#[test]
fn escape_and_losing_focus_and_switching_tools_drop_a_drag_without_changing_anything_and_enter_commits(
) {
    let (mut h, id) = with_layer_painted();
    let before = color_bytes(&h.state().state, id);
    let revision = h.state().state.doc.revision();
    let from = screen_of(&h, 50.0, 50.0);
    let start = |h: &mut Harness<'_, YoluApp>| {
        press(h, from, PointerButton::Primary);
        h.step();
        move_to(h, offset(from, 30.0, 10.0));
        h.step();
        assert!(h.state().state.transform.drag.is_some());
    };
    // Esc
    start(&mut h);
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    assert!(h.state().state.transform.drag.is_none());
    release(&h, offset(from, 30.0, 10.0), PointerButton::Primary);
    h.run();
    assert_eq!(h.state().state.doc.revision(), revision);
    assert_eq!(color_bytes(&h.state().state, id), before);
    assert_eq!(h.state().state.message, "変形をやめました。");
    // ウィンドウがフォーカスを失った
    start(&mut h);
    h.event(Event::WindowFocused(false));
    h.step();
    assert!(h.state().state.transform.drag.is_none());
    release(&h, from, PointerButton::Primary);
    h.run();
    assert_eq!(h.state().state.doc.revision(), revision);
    // ほかのツールへ替えた
    start(&mut h);
    h.state_mut().state.apply(Action::SelectTool(Tool::Brush));
    h.run();
    assert!(h.state().state.transform.drag.is_none());
    release(&h, from, PointerButton::Primary);
    h.run();
    assert_eq!(h.state().state.doc.revision(), revision);
    assert_eq!(color_bytes(&h.state().state, id), before);
    h.state_mut().state.apply(Action::SelectTool(Tool::Move));
    h.run();
    // Enter: ドラッグの途中でその位置で確定し、あとで離しても二重には動かない
    start(&mut h);
    key(&h, Key::Enter, Modifiers::NONE);
    h.step();
    assert!(h.state().state.transform.drag.is_none());
    let moved = color_bytes(&h.state().state, id);
    assert_ne!(moved, before);
    release(&h, offset(from, 60.0, 10.0), PointerButton::Primary);
    h.run();
    assert_eq!(color_bytes(&h.state().state, id), moved);
    undo(&mut h.state_mut().state);
    assert_eq!(color_bytes(&h.state().state, id), before);
}

#[test]
fn arrow_keys_move_one_pixel_and_shift_arrows_ten_following_the_screen_direction() {
    let (mut h, id) = with_layer_painted();
    key(&h, Key::ArrowRight, Modifiers::NONE);
    h.run();
    assert_eq!(pixel(&h.state().state, id, 31, 40), RED.to_array());
    assert_eq!(pixel(&h.state().state, id, 30, 40), [0; 4]);
    key(&h, Key::ArrowUp, Modifiers::SHIFT);
    h.run();
    // 上向き（キャンバスの y が増える）へ 10 画素
    assert_eq!(pixel(&h.state().state, id, 31, 79), RED.to_array());
    assert_eq!(pixel(&h.state().state, id, 31, 30), [0; 4]);
    // 1 つずつ別の Undo
    undo(&mut h.state_mut().state);
    undo(&mut h.state_mut().state);
    assert_eq!(pixel(&h.state().state, id, 30, 30), RED.to_array());
    // 表示を 90°（時計回り）に回すと、画面の右はキャンバスの上向き（y が増える）になる
    h.state_mut().state.view.set_angle(90.0);
    h.run();
    key(&h, Key::ArrowRight, Modifiers::NONE);
    h.run();
    assert_eq!(pixel(&h.state().state, id, 30, 70), RED.to_array());
    assert_eq!(pixel(&h.state().state, id, 30, 30), [0; 4]);
    // Ctrl + 矢印・ほかのツールでは動かさない（文字を打っている間・3D ビューが前のときは下の別の試験）
    let revision = h.state().state.doc.revision();
    key(&h, Key::ArrowLeft, Modifiers::COMMAND);
    h.run();
    assert_eq!(h.state().state.doc.revision(), revision);
    h.state_mut().state.apply(Action::SelectTool(Tool::Brush));
    h.run();
    key(&h, Key::ArrowLeft, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.doc.revision(), revision);
}

/// 矢印キーはキャンバスが見えているときだけ動かす: 3D ビューのタブが前にあるあいだ、見えていない 2D のレイヤーは動かない（タブを戻せば動く）。
#[test]
fn arrow_keys_do_not_move_the_layer_while_the_3d_view_tab_is_in_front() {
    let (mut h, id) = with_layer_painted();
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    h.run();
    assert_eq!(h.state().state.tool, Tool::Move);
    let revision = h.state().state.doc.revision();
    let before = color_bytes(&h.state().state, id);
    for modifiers in [Modifiers::NONE, Modifiers::SHIFT] {
        for k in [Key::ArrowRight, Key::ArrowUp] {
            key(&h, k, modifiers);
            h.run();
        }
    }
    let s = &h.state().state;
    assert_eq!(s.doc.revision(), revision, "見えていないレイヤーが動いた");
    assert_eq!(color_bytes(s, id), before);
    assert!(!s.modified);
    assert!(!s.doc.can_undo());
    // キャンバスのタブを前へ戻すと、また矢印で動く
    click_tab(&mut h, yolu_app::Tab::Canvas);
    h.run();
    h.run();
    key(&h, Key::ArrowRight, Modifiers::NONE);
    h.run();
    let s = &h.state().state;
    assert_eq!(pixel(s, id, 31, 40), RED.to_array());
    assert_eq!(pixel(s, id, 30, 40), [0; 4]);
    assert_eq!(s.doc.undo_count(), 1);
}

/// 文字を打っている間（レイヤーの名前の変更中）の矢印キーは、文字の入力のもので、レイヤーを動かさない。
#[test]
fn arrow_keys_while_typing_a_layer_name_edit_the_text_and_never_move_the_layer() {
    let (mut h, id) = with_layer_painted();
    let row = rect_of(&h, "レイヤー 1", |_| true);
    let at = pos2(row.left() + 70.0, row.center().y);
    for _ in 0..2 {
        press(&h, at, PointerButton::Primary);
        release(&h, at, PointerButton::Primary);
        h.step();
    }
    h.run();
    assert!(h.state().state.ui.renaming.is_some(), "名前の入力が始まる");
    assert_eq!(h.state().state.tool, Tool::Move);
    let revision = h.state().state.doc.revision();
    let before = color_bytes(&h.state().state, id);
    for modifiers in [Modifiers::NONE, Modifiers::SHIFT] {
        for k in [
            Key::ArrowRight,
            Key::ArrowLeft,
            Key::ArrowUp,
            Key::ArrowDown,
        ] {
            key(&h, k, modifiers);
            h.run();
        }
    }
    let s = &h.state().state;
    assert!(s.ui.renaming.is_some(), "入力は続いている");
    assert_eq!(
        s.doc.revision(),
        revision,
        "文字を打っている間にレイヤーが動いた"
    );
    assert_eq!(color_bytes(s, id), before);
    assert!(!s.modified);
    // 入力を終えると、同じキーで動く
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(h.state().state.ui.renaming.is_none());
    key(&h, Key::ArrowRight, Modifiers::NONE);
    h.run();
    assert_eq!(pixel(&h.state().state, id, 31, 40), RED.to_array());
}

#[test]
fn the_move_tool_scales_by_a_corner_and_rotates_outside_a_corner_and_the_handles_follow_the_bounds()
{
    let (mut h, id) = with_layer_painted();
    let bounds = h.state_mut().state.transform_bounds_cached().unwrap();
    assert_eq!(bounds, (30, 30, 70, 70));
    // 右上の角を掴んで、反対の角（左下）を動かさずに 2 倍
    let corner = screen_of(&h, 70.0, 70.0);
    let target = screen_of(&h, 110.0, 110.0);
    drag(&mut h, &[corner, offset(corner, 4.0, -4.0), target]);
    let s = &h.state().state;
    assert_eq!(pixel(s, id, 40, 40), RED.to_array());
    assert_eq!(pixel(s, id, 100, 100), RED.to_array());
    assert_eq!(pixel(s, id, 114, 114), [0; 4]);
    // 縁の画素は補間で半透明になるので、外接は 1 画素のずれを許す。動かさない角（左下）は 30 のまま
    let b = s.transform_bounds().unwrap();
    assert!((b.0 - 30).abs() <= 1 && (b.1 - 30).abs() <= 1, "{b:?}");
    assert!((b.2 - 110).abs() <= 1 && (b.3 - 110).abs() <= 1, "{b:?}");
    undo(&mut h.state_mut().state);
    h.run();
    // 辺の中点は片方向だけ（右の辺を右へ 20 → 横が 1.5 倍、縦はそのまま）
    let edge = screen_of(&h, 70.0, 50.0);
    let edge_to = screen_of(&h, 90.0, 50.0);
    drag(&mut h, &[edge, offset(edge, 5.0, 0.0), edge_to]);
    let b = h.state().state.transform_bounds().unwrap();
    assert!((b.2 - 90).abs() <= 1 && (b.3 - 70).abs() <= 1, "{b:?}");
    assert!((b.0 - 30).abs() <= 1 && (b.1 - 30).abs() <= 1, "{b:?}");
    undo(&mut h.state_mut().state);
    h.run();
    // 角の少し外を掴むと回転（Shift で 15° 刻み。正方形を 45° 回せば菱形になり、範囲が広がる）
    let b = rotate_by_dragging(&mut h, (70.0, 70.0), 45.0);
    assert!(h.state().state.doc.can_undo(), "回した");
    assert!(
        (56..=60).contains(&(b.2 - b.0)) && (56..=60).contains(&(b.3 - b.1)),
        "{b:?}"
    );
    assert_eq!(pixel(&h.state().state, id, 50, 50), RED.to_array());
    assert_eq!(
        pixel(&h.state().state, id, 31, 31),
        [0; 4],
        "角は菱形から外れる"
    );
    undo(&mut h.state_mut().state);
    assert!(!h.state().state.doc.can_undo());
}

/// 角の外側を掴んで、画面の中心のまわりを反時計回りに `degrees` 回して離す（Shift を押したまま）。回した後の範囲を返す。
fn rotate_by_dragging(
    h: &mut Harness<'_, YoluApp>,
    corner: (f64, f64),
    degrees: f64,
) -> (i64, i64, i64, i64) {
    let b = h.state_mut().state.transform_bounds_cached().unwrap();
    let center = screen_of(h, (b.0 + b.2) as f64 / 2.0, (b.1 + b.3) as f64 / 2.0);
    let grab = screen_of(h, corner.0, corner.1);
    let start = offset(grab, 10.0, -10.0);
    let r = start - center;
    let (sin, cos) = degrees.to_radians().sin_cos();
    let (sin, cos) = (sin as f32, cos as f32);
    let end = center + egui::vec2(r.x * cos + r.y * sin, -r.x * sin + r.y * cos);
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    h.step();
    press(h, start, PointerButton::Primary);
    h.step();
    move_to(h, offset(start, 1.0, -1.0));
    h.step();
    move_to(h, end);
    h.step();
    assert!(matches!(
        h.state().state.transform.drag.as_ref().map(|d| d.mode),
        Some(yolu_app::transform::Mode::Rotate)
    ));
    h.event(Event::PointerButton {
        pos: end,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::SHIFT,
    });
    h.step();
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run();
    h.state().state.transform_bounds().unwrap()
}

/// 長方形を Shift で 90° 回すと、画素がそのまま（補間なしで）縦長の長方形になる。
#[test]
fn the_move_tool_turns_a_rectangle_by_exactly_ninety_degrees_with_shift() {
    let mut h = app(1280.0, 800.0, 128);
    let id = h.state().state.selected_layer.unwrap();
    {
        let s = &mut h.state_mut().state;
        fill_rect(&mut s.doc, id, 30, 40, 70, 60, RED);
        settle(s);
        s.apply(Action::SelectTool(Tool::Move));
    }
    h.run();
    let b = rotate_by_dragging(&mut h, (70.0, 60.0), 90.0);
    assert_eq!(
        b,
        (40, 30, 60, 70),
        "横 40・縦 20 が、中心（50, 50）のまわりで縦長に"
    );
    let s = &h.state().state;
    assert_eq!(pixel(s, id, 50, 31), RED.to_array());
    assert_eq!(pixel(s, id, 41, 69), RED.to_array());
    assert_eq!(pixel(s, id, 35, 50), [0; 4]);
    // 境の画素まで 0 か 255 のまま（補間でにじんでいない）
    let bytes = color_bytes(s, id);
    assert!(bytes.chunks(4).all(|p| p[3] == 0 || p[3] == 255));
}

#[test]
fn a_small_layer_has_no_handles_and_any_press_moves_it_and_nothing_to_move_says_so() {
    let mut h = app(1280.0, 800.0, 128);
    let id = h.state().state.selected_layer.unwrap();
    {
        let s = &mut h.state_mut().state;
        fill_rect(&mut s.doc, id, 60, 60, 62, 62, RED); // 2×2 画素: 画面で 36 点より小さい
        settle(s);
        s.apply(Action::SelectTool(Tool::Move));
    }
    h.run();
    let b = h.state_mut().state.transform_bounds_cached().unwrap();
    let rect = canvas_rect(&h);
    let view = h.state().state.view.view(
        rect,
        h.state().state.doc.width(),
        h.state().state.doc.height(),
    );
    assert!(!yolu_app::transform::handles_usable(&view, b));
    // 角の上でも、ハンドルではなく移動
    let corner = screen_of(&h, 62.0, 62.0);
    assert_eq!(
        yolu_app::transform::hit(&view, b, corner),
        yolu_app::transform::Mode::Move
    );
    // 画素の無いレイヤーでは、短い理由を言って始めない
    let empty = {
        let s = &mut h.state_mut().state;
        s.apply(Action::NewLayer);
        s.selected_layer.unwrap()
    };
    h.run();
    let at = canvas_rect(&h).center();
    press(&h, at, PointerButton::Primary);
    h.step();
    assert!(h.state().state.transform.drag.is_none());
    assert_eq!(h.state().state.message, "動かす画素がありません。");
    release(&h, at, PointerButton::Primary);
    h.run();
    let _ = empty;
}

#[test]
fn the_move_tool_never_paints_and_pressing_outside_the_pixels_still_moves_the_layer() {
    let (mut h, id) = with_layer_painted();
    // 範囲の外（画素の無い所）を押して動かしても、描かずにレイヤーを動かす（Unity 版と同じ。ハンドルでも範囲の中でもなければ移動）
    let at = screen_of(&h, 100.0, 100.0);
    let to = screen_of(&h, 105.0, 98.0);
    drag(&mut h, &[at, offset(at, 2.0, 0.0), to]);
    let s = &h.state().state;
    assert!(s.canvas.stroke.is_none());
    assert_eq!(pixel(s, id, 35, 28), RED.to_array(), "5 右・2 下へ動いた");
    assert_eq!(pixel(s, id, 30, 30), [0; 4]);
    // マウスだけで動かしたので、ペンの押している印は付かない（ペンの経路は下の `a_pen_...` の試験）
    assert!(s.transform.pen_down.is_none());
}

// ───────── 移動・変形のツール（ペン） ─────────

fn pen_at(pos: egui::Pos2, contact: bool, pointer_id: u32) -> yolu_app::pen::PenSample {
    yolu_app::pen::PenSample {
        pos: [pos.x, pos.y],
        pressure: 0.5,
        tilt: yolu_app::engine::Tilt::default(),
        rotation: None,
        contact,
        eraser: false,
        barrel: false,
        pointer_id,
        time_ms: 0,
    }
}

/// ペンの 1 点を渡して 1 フレーム回す。
fn pen(h: &mut Harness<'_, YoluApp>, pos: egui::Pos2, contact: bool, pointer_id: u32) {
    h.state().pen().push(pen_at(pos, contact, pointer_id));
    h.step();
}

fn non_transparent(s: &AppState, id: LayerId) -> usize {
    let bytes = color_bytes(s, id);
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] != 0)
        .count()
}

#[test]
fn a_pen_touch_drag_and_lift_move_the_layer_like_the_mouse_with_one_undo_and_never_paint() {
    let (mut h, id) = with_layer_painted();
    let before = color_bytes(&h.state().state, id);
    let painted = non_transparent(&h.state().state, id);
    let revision = h.state().state.doc.revision();
    let from = screen_of(&h, 50.0, 50.0);
    let to = screen_of(&h, 60.0, 44.0);
    pen(&mut h, from, true, 3);
    {
        let s = &h.state().state;
        let drag = s.transform.drag.as_ref().expect("触れたらドラッグが始まる");
        assert_eq!(drag.source, yolu_app::state::StrokeSource::Pen(3));
        assert_eq!(drag.mode, yolu_app::transform::Mode::Move);
        assert_eq!(s.transform.pen_down, Some(3));
        // 範囲の中を触れても描かない（ストロークは始まらず、ドラッグの間は編集を断る印は立つ）
        assert!(s.canvas.stroke.is_none());
        assert!(s.is_stroking());
    }
    pen(&mut h, offset(from, 3.0, 0.0), true, 3);
    pen(&mut h, to, true, 3);
    {
        // ドラッグの間は文書を変えない。外枠だけが動く
        let s = &h.state().state;
        assert_eq!(s.doc.revision(), revision);
        assert_eq!(s.transform.drag.as_ref().unwrap().delta(), (10, -6));
        assert_eq!(color_bytes(s, id), before);
    }
    pen(&mut h, to, false, 3);
    h.run();
    let s = &h.state().state;
    assert_eq!(pixel(s, id, 40, 36), RED.to_array());
    assert_eq!(pixel(s, id, 30, 30), [0; 4]);
    assert_eq!(
        non_transparent(s, id),
        painted,
        "描いていない。動かしただけ"
    );
    assert!(s.canvas.stroke.is_none());
    assert!(s.transform.drag.is_none() && s.transform.pen_down.is_none());
    assert!(!s.is_stroking());
    assert!(s.modified);
    assert_eq!(s.doc.undo_count(), 1, "離したところで 1 回の Undo");
    undo(&mut h.state_mut().state);
    assert_eq!(color_bytes(&h.state().state, id), before);
    assert!(!h.state().state.doc.can_undo());
    // 動かさずに触れて離しただけなら、何も当てない
    let revision = h.state().state.doc.revision();
    pen(&mut h, from, true, 3);
    pen(&mut h, from, false, 3);
    h.run();
    let s = &h.state().state;
    assert_eq!(s.doc.revision(), revision);
    assert!(s.transform.drag.is_none() && s.transform.pen_down.is_none());
}

#[test]
fn escape_losing_focus_and_switching_tools_during_a_pen_drag_change_nothing_and_leave_no_stroke() {
    let (mut h, id) = with_layer_painted();
    let before = color_bytes(&h.state().state, id);
    let revision = h.state().state.doc.revision();
    let from = screen_of(&h, 50.0, 50.0);
    let far = screen_of(&h, 65.0, 55.0);
    let start = |h: &mut Harness<'_, YoluApp>| {
        pen(h, from, true, 3);
        pen(h, offset(from, 20.0, 5.0), true, 3);
        assert!(h.state().state.transform.drag.is_some());
        assert_eq!(h.state().state.transform.pen_down, Some(3));
    };
    let settled = |h: &Harness<'_, YoluApp>| {
        let s = &h.state().state;
        assert_eq!(s.doc.revision(), revision);
        assert_eq!(color_bytes(s, id), before);
        assert!(s.transform.drag.is_none());
        assert!(s.canvas.stroke.is_none());
        assert!(!s.is_stroking(), "ペンを持っていても、編集を断り続けない");
        assert_eq!(s.doc.undo_count(), 0);
    };
    // Esc: ドラッグをやめる。ペンの番号は離すまで残り、残りの動きは新しいドラッグにならない
    start(&mut h);
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    settled(&h);
    assert_eq!(h.state().state.transform.pen_down, Some(3));
    pen(&mut h, far, true, 3);
    assert!(h.state().state.transform.drag.is_none());
    pen(&mut h, far, false, 3);
    h.run();
    settled(&h);
    assert!(
        h.state().state.transform.pen_down.is_none(),
        "離したら外れる"
    );
    // ウィンドウがフォーカスを失った: 離したのを受け取れないので、ドラッグも押している印も捨てる
    start(&mut h);
    h.event(Event::WindowFocused(false));
    h.step();
    settled(&h);
    assert!(h.state().state.transform.pen_down.is_none());
    pen(&mut h, far, false, 3);
    h.run();
    settled(&h);
    // ほかのツールへ替えた: 同じく何も変えずにやめる（ペンを離しても何も起きない）
    start(&mut h);
    h.state_mut()
        .state
        .apply(Action::SelectTool(Tool::SelectRect));
    h.run();
    settled(&h);
    assert!(h.state().state.transform.pen_down.is_none());
    pen(&mut h, far, false, 3);
    h.run();
    settled(&h);
    // 何も残っていないので、続けてペンで動かせる（1 回の Undo）
    h.state_mut().state.apply(Action::SelectTool(Tool::Move));
    h.run();
    let to = screen_of(&h, 55.0, 50.0);
    pen(&mut h, from, true, 3);
    pen(&mut h, to, true, 3);
    pen(&mut h, to, false, 3);
    h.run();
    let s = &h.state().state;
    assert_eq!(pixel(s, id, 35, 30), RED.to_array());
    assert_eq!(s.doc.undo_count(), 1);
    assert!(!s.is_stroking());
}

#[test]
fn a_second_pen_never_joins_the_pen_that_is_dragging_the_layer() {
    let (mut h, id) = with_layer_painted();
    let before = color_bytes(&h.state().state, id);
    let revision = h.state().state.doc.revision();
    let from = screen_of(&h, 50.0, 50.0);
    let moved = screen_of(&h, 58.0, 50.0);
    let other = screen_of(&h, 90.0, 90.0);
    pen(&mut h, from, true, 3);
    pen(&mut h, moved, true, 3);
    // 別のペン（番号 4）が触れる・動く・離す: ドラッグの位置も確定も動かさない
    pen(&mut h, other, true, 4);
    pen(&mut h, offset(other, 40.0, 40.0), true, 4);
    {
        let s = &h.state().state;
        let drag = s.transform.drag.as_ref().expect("番号 3 のドラッグは続く");
        assert_eq!(drag.source, yolu_app::state::StrokeSource::Pen(3));
        assert_eq!(drag.delta(), (8, 0), "番号 4 の位置は混ざらない");
        assert_eq!(s.transform.pen_down, Some(3));
    }
    pen(&mut h, offset(other, 40.0, 40.0), false, 4);
    {
        let s = &h.state().state;
        assert!(s.transform.drag.is_some(), "番号 4 が離れても確定しない");
        assert_eq!(s.transform.pen_down, Some(3));
        assert_eq!(s.doc.revision(), revision);
        assert_eq!(color_bytes(s, id), before);
    }
    // 番号 3 が離れたところで、番号 3 の位置のまま確定する
    pen(&mut h, moved, false, 3);
    h.run();
    let s = &h.state().state;
    assert_eq!(pixel(s, id, 38, 30), RED.to_array());
    assert_eq!(pixel(s, id, 30, 30), [0; 4]);
    assert_eq!(s.doc.undo_count(), 1);
    assert!(s.transform.drag.is_none() && s.transform.pen_down.is_none());
}

#[test]
fn switching_to_the_move_tool_during_a_pen_brush_stroke_still_ends_the_stroke_when_the_pen_lifts() {
    use yolu_app::state::StrokeSource;
    let mut h = app(1280.0, 800.0, 128);
    h.state_mut().state.apply(Action::SelectTool(Tool::Brush));
    h.run();
    let a = screen_of(&h, 20.0, 20.0);
    pen(&mut h, a, true, 3);
    pen(&mut h, offset(a, 30.0, 10.0), true, 3);
    assert_eq!(h.state().state.canvas.stroke, Some(StrokeSource::Pen(3)));
    // ペンを付けたまま V（移動・変形）。描いている間もツールは替えられる
    key(&h, Key::V, Modifiers::NONE);
    h.step();
    assert_eq!(h.state().state.tool, Tool::Move);
    // 動かしている間はストロークの続き（移動のドラッグは始まらない）
    pen(&mut h, offset(a, 60.0, 20.0), true, 3);
    let s = &h.state().state;
    assert_eq!(s.canvas.stroke, Some(StrokeSource::Pen(3)));
    assert!(s.transform.drag.is_none() && s.transform.pen_down.is_none());
    // 離すとストロークが確定する（取り残さない）。描いた分は 1 回の Undo
    pen(&mut h, offset(a, 60.0, 20.0), false, 3);
    h.run();
    let s = &h.state().state;
    assert!(s.canvas.stroke.is_none(), "ペンのストロークが残った");
    assert!(!s.is_stroking());
    assert_eq!(s.doc.undo_count(), 1);
    assert!(s.transform.drag.is_none() && s.transform.pen_down.is_none());
}

// ───────── 画面（レイヤーの欄・メニュー・キー） ─────────

fn row_center(h: &Harness<'_, YoluApp>, name: &str) -> egui::Pos2 {
    h.get_by_label(name).rect().center()
}

fn named_layers(h: &mut Harness<'_, YoluApp>, n: usize) -> Vec<LayerId> {
    for _ in 1..n {
        h.state_mut().state.apply(Action::NewLayer);
    }
    h.run();
    h.state()
        .state
        .doc
        .layers()
        .iter()
        .map(|l| l.id())
        .collect()
}

#[test]
fn clicking_rows_with_ctrl_and_shift_selects_several_layers() {
    let mut h = app(1280.0, 1400.0, 64);
    let layers = named_layers(&mut h, 4); // 下から 1 2 3 4
                                          // 今は一番上の「レイヤー 2」…名前は 2 つ目からの番号（最初が「レイヤー 1」、足すたびにレイヤーの数 + 1）
    let name = |h: &Harness<'_, YoluApp>, id: LayerId| {
        h.state().state.doc.layer(id).unwrap().name().to_owned()
    };
    let names: Vec<String> = layers.iter().map(|id| name(&h, *id)).collect();
    h.get_by_label(&names[0]).click();
    h.run();
    assert_eq!(h.state().state.selected_layers(), vec![layers[0]]);
    h.get_by_label(&names[2])
        .click_modifiers(Modifiers::COMMAND);
    h.run();
    assert_eq!(
        h.state().state.selected_layers(),
        vec![layers[0], layers[2]]
    );
    assert_eq!(h.state().state.selected_layer, Some(layers[2]));
    // Shift: 起点（Ctrl で押したレイヤー）から押したレイヤーまで
    h.get_by_label(&names[3]).click_modifiers(Modifiers::SHIFT);
    h.run();
    assert_eq!(
        h.state().state.selected_layers(),
        vec![layers[2], layers[3]]
    );
    h.get_by_label(&names[0]).click();
    h.run();
    assert_eq!(h.state().state.selected_layers(), vec![layers[0]]);
    h.get_by_label(&names[3]).click_modifiers(Modifiers::SHIFT);
    h.run();
    assert_eq!(h.state().state.selected_layers(), layers);
    // 選んだ行を押してそのままドラッグすれば選んだ全部を運ぶ（選択は壊れない）
    let from = row_center(&h, &names[1]);
    let to = offset(from, 0.0, 120.0);
    press(&h, from, PointerButton::Primary);
    h.step();
    move_to(&h, to);
    h.step();
    assert_eq!(h.state().state.selected_layers(), layers);
    assert!(h.state().state.ui.layer_drag.is_some());
    release(&h, to, PointerButton::Primary);
    h.run();
    assert_eq!(h.state().state.doc.layers().len(), 4);
}

#[test]
fn the_row_shows_a_lock_mark_and_clicking_it_unlocks_the_layers_own_locks() {
    let mut h = app(1280.0, 1400.0, 64);
    let layers = named_layers(&mut h, 2);
    let (low, top) = (layers[0], layers[1]);
    let group = {
        let s = &mut h.state_mut().state;
        s.apply(Action::M2(Edit::Lock {
            ids: vec![low],
            flag: LayerLocks::PIXELS | LayerLocks::TRANSPARENCY,
            on: true,
        }));
        s.select_single_layer(top);
        s.apply(Action::M2(Edit::GroupSelected));
        let group = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::Lock {
            ids: vec![group],
            flag: LayerLocks::POSITION,
            on: true,
        }));
        s.doc.clear_history().unwrap();
        group
    };
    h.run();
    // 自分のロックはロックの名前だけ、グループから効いているだけのレイヤーは、外せない薄い印
    let own = "ロック: 透明部分、画素";
    let inherited = "ロック: 位置（グループから）";
    let group_own = "ロック: 位置";
    assert!(h.query_by_label(own).is_some());
    assert!(h.query_by_label(inherited).is_some());
    assert!(h.query_by_label(group_own).is_some());
    // 英語
    h.state_mut().state.lang = Lang::En;
    h.run();
    assert!(h
        .query_by_label("Locked: Transparent pixels, Image pixels")
        .is_some());
    assert!(h
        .query_by_label("Locked: Position (from its group)")
        .is_some());
    h.state_mut().state.lang = Lang::Ja;
    h.run();
    // 外せない印を押しても何も変わらない
    let revision = h.state().state.doc.revision();
    h.get_by_label(inherited).click();
    h.run();
    assert_eq!(h.state().state.doc.revision(), revision);
    // 押すと自分のロックを全部外す（1 回の Undo）
    h.get_by_label(own).click();
    h.run();
    assert_eq!(
        h.state().state.doc.layer(low).unwrap().locks(),
        LayerLocks::NONE
    );
    assert_eq!(
        h.state().state.doc.layer(group).unwrap().locks(),
        LayerLocks::POSITION
    );
    assert!(h.query_by_label(own).is_none());
    undo(&mut h.state_mut().state);
    assert_eq!(
        h.state().state.doc.layer(low).unwrap().locks(),
        LayerLocks::PIXELS | LayerLocks::TRANSPARENCY
    );
    assert!(!h.state().state.doc.can_undo());
}

#[test]
fn ctrl_e_and_ctrl_shift_e_and_ctrl_j_and_ctrl_g_run_the_layer_operations() {
    let mut h = app(1280.0, 800.0, 64);
    let layers = named_layers(&mut h, 3);
    {
        let s = &mut h.state_mut().state;
        for (i, id) in layers.iter().enumerate() {
            fill_rect(&mut s.doc, *id, i as u32 * 4, 0, i as u32 * 4 + 3, 3, RED);
        }
        settle(s);
    }
    key(&h, Key::E, Modifiers::COMMAND);
    h.run();
    assert_eq!(
        h.state().state.doc.layers().len(),
        2,
        "{}",
        h.state().state.message
    );
    undo(&mut h.state_mut().state);
    h.run();
    key(&h, Key::E, Modifiers::COMMAND | Modifiers::SHIFT);
    h.run();
    assert_eq!(
        h.state().state.doc.layers().len(),
        1,
        "{}",
        h.state().state.message
    );
    undo(&mut h.state_mut().state);
    h.run();
    key(&h, Key::J, Modifiers::COMMAND);
    h.run();
    assert_eq!(h.state().state.doc.layers().len(), 4);
    undo(&mut h.state_mut().state);
    h.run();
    key(&h, Key::G, Modifiers::COMMAND);
    h.run();
    assert_eq!(h.state().state.doc.layers().len(), 4);
    assert!(h
        .state()
        .state
        .doc
        .layer(h.state().state.selected_layer.unwrap())
        .unwrap()
        .is_group());
    // Ctrl+Shift+G はグループをほどく（グループでないときは何も変えず断る）
    key(&h, Key::G, Modifiers::COMMAND | Modifiers::SHIFT);
    h.run();
    assert_eq!(h.state().state.doc.layers().len(), 3);
    assert!(h.state().state.doc.layers().iter().all(|l| !l.is_group()));
    let revision = h.state().state.doc.revision();
    key(&h, Key::G, Modifiers::COMMAND | Modifiers::SHIFT);
    h.run();
    assert_eq!(h.state().state.doc.revision(), revision);
    assert_eq!(
        h.state().state.doc.layers().len(),
        3,
        "グループでなければ、まとめもしない"
    );
    assert_eq!(h.state().state.message, "グループではない。");
    // V は移動・変形のツール
    undo(&mut h.state_mut().state);
    undo(&mut h.state_mut().state);
    h.run();
    key(&h, Key::V, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.tool, Tool::Move);
    // 描いている間は動かさない
    h.state_mut().state.apply(Action::SelectTool(Tool::Brush));
}

#[test]
fn the_layer_menu_offers_merge_and_lock_and_no_transform_and_the_merge_confirmation_window_works() {
    for lang in Lang::ALL {
        let mut s = AppState::new_in(64, 64, lang);
        let [_a, _b, c] = three(&mut s);
        let items = yolu_app::shell::menu_entries(&s, 2);
        let label = |e: &yolu_app::ui::menu::Entry<Action>| match e {
            yolu_app::ui::menu::Entry::Item { label, .. } => Some(label.clone()),
            _ => None,
        };
        let labels: Vec<String> = items.iter().filter_map(label).collect();
        for want in [
            lang.pick("下のレイヤーと結合", "Merge Down"),
            lang.pick("表示レイヤーを結合", "Merge Visible"),
            // 見出しが無いので、ロックの項目は何をするかを言い切る名前
            lang.pick("透明部分をロック", "Lock Transparent Pixels"),
            lang.pick("画素をロック", "Lock Image Pixels"),
            lang.pick("位置をロック", "Lock Position"),
            lang.pick("すべてをロック", "Lock All"),
        ] {
            assert!(
                labels.iter().any(|l| l == want),
                "{lang:?}: {want}: {labels:?}"
            );
        }
        // 左右反転・上下反転・90° 回転は「編集」のメニューにあり、レイヤーのメニューには無い
        for gone in [
            lang.pick("左右反転", "Flip Horizontal"),
            lang.pick("上下反転", "Flip Vertical"),
            lang.pick("時計回りに 90° 回転", "Rotate 90° Clockwise"),
            lang.pick("反時計回りに 90° 回転", "Rotate 90° Counter-clockwise"),
            lang.pick("変形", "Transform"),
            lang.pick("ロック", "Lock"),
        ] {
            assert!(
                !labels.iter().any(|l| l == gone),
                "{lang:?}: レイヤーのメニューに {gone}: {labels:?}"
            );
        }
        let edit_menu = yolu_app::shell::menu_entries(&s, 1);
        let edit_labels: Vec<String> = edit_menu.iter().filter_map(label).collect();
        for want in [
            lang.pick("左右反転", "Flip Horizontal"),
            lang.pick("上下反転", "Flip Vertical"),
            lang.pick("時計回りに 90° 回転", "Rotate 90° Clockwise"),
            lang.pick("反時計回りに 90° 回転", "Rotate 90° Counter-clockwise"),
        ] {
            assert!(
                edit_labels.iter().any(|l| l == want),
                "{lang:?}: 編集のメニューに {want}: {edit_labels:?}"
            );
        }
        // メニューの項目名は「〜をロック」でも、プロパティなどの種類の名前（`lock_name`）は変えない
        assert_eq!(
            yolu_app::layerops::lock_name(lang, LayerLocks::TRANSPARENCY),
            lang.pick("透明部分", "Transparent pixels")
        );
        assert_eq!(
            labels.iter().any(|l| has_japanese(l)),
            lang == Lang::Ja,
            "{labels:?}"
        );
        // 複数選ぶと、結合は「レイヤーを結合」になる
        s.select_layers([_a, c], c);
        let items = yolu_app::shell::menu_entries(&s, 2);
        assert!(items
            .iter()
            .filter_map(label)
            .any(|l| l == lang.pick("レイヤーを結合", "Merge Layers")));
        // ロックの項目は持っているロックにチェック、押すと全部の選んだレイヤーへ
        let both = s.selected_layers();
        edit(
            &mut s,
            Edit::Lock {
                ids: both,
                flag: LayerLocks::POSITION,
                on: true,
            },
        );
        let items = yolu_app::shell::menu_entries(&s, 2);
        let position = items
            .iter()
            .find(|e| label(e).as_deref() == Some(lang.pick("位置をロック", "Lock Position")))
            .unwrap();
        match position {
            yolu_app::ui::menu::Entry::Item { check, action, .. } => {
                assert_ne!(format!("{check:?}"), "None");
                match action {
                    Action::M2(Edit::Lock { ids, flag, on }) => {
                        assert_eq!(ids.len(), 2);
                        assert_eq!(*flag, LayerLocks::POSITION);
                        assert!(!on, "持っているので押すと外す");
                    }
                    other => panic!("{other:?}"),
                }
            }
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn the_merge_confirmation_window_asks_and_merges_or_cancels_from_the_buttons() {
    let mut h = app(1280.0, 800.0, 64);
    {
        let s = &mut h.state_mut().state;
        let a = s.selected_layer.unwrap();
        fill_rect(&mut s.doc, a, 0, 0, 64, 64, GREEN);
        let b = add_painted(s, (10, 10, 40, 40), RED);
        s.doc.set_layer_blend_mode(b, BlendMode::Multiply).unwrap();
        add_painted(s, (20, 20, 50, 50), BLUE);
        settle(s);
        s.apply(Action::M2(Edit::MergeDown));
        assert!(s.layer_ops.merge_confirm.is_some());
    }
    h.run();
    let window = yolu_app::windows::window_rect(&h.ctx, "merge-confirm");
    assert!(window.is_some(), "確認のウィンドウが出る");
    // ウィンドウが開いているあいだは、キーの割り当てを止める（Ctrl+Z で文書を動かさない）
    assert!(yolu_app::windows::modal_open(&h.state().state));
    h.get_by_label("やめる").click();
    h.run();
    assert!(h.state().state.layer_ops.merge_confirm.is_none());
    assert_eq!(h.state().state.doc.layers().len(), 3);
    h.state_mut().state.apply(Action::M2(Edit::MergeDown));
    h.run();
    h.get_by_label("結合する").click();
    h.run();
    assert!(h.state().state.layer_ops.merge_confirm.is_none());
    assert_eq!(h.state().state.doc.layers().len(), 2);
    undo(&mut h.state_mut().state);
    assert_eq!(h.state().state.doc.layers().len(), 3);
    let _ = pos2(0.0, 0.0);
}

// ───────── .ylp への保存と開き直し ─────────

/// ロックのある文書は、.ylp へ保存でき（以前は理由つきで断っていた）、開き直しても自分のロックとグループのロックが同じ。
#[test]
fn headless_a_project_with_layer_locks_saves_and_opens_with_the_same_locks() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/layerops-project-tests")
        .join(std::process::id().to_string());
    std::fs::create_dir_all(&root).unwrap();
    for lang in Lang::ALL {
        let path = root.join(lang.pick("ja.ylp", "en.ylp"));
        let mut s = AppState::new_in(32, 32, lang);
        let [a, b, c] = three(&mut s);
        s.select_layers([b, c], c);
        edit(&mut s, Edit::GroupSelected);
        let group = s.selected_layer.unwrap();
        // ロックを付ける前の文書を先に書いておく（あとでロックを全部外して書き直したものと比べる）
        let plain_path = root.join(lang.pick("ja-plain.ylp", "en-plain.ylp"));
        s.apply(Action::SaveProjectAs(plain_path.clone()));
        assert!(
            s.message.starts_with(lang.pick("保存しました", "Saved")),
            "{}",
            s.message
        );
        s.doc.set_layer_locks(a, LayerLocks::PIXELS).unwrap();
        edit(
            &mut s,
            Edit::Lock {
                ids: vec![group],
                flag: LayerLocks::POSITION | LayerLocks::TRANSPARENCY,
                on: true,
            },
        );
        s.apply(Action::SaveProjectAs(path.clone()));
        assert!(
            s.message.starts_with(lang.pick("保存しました", "Saved")),
            "{}",
            s.message
        );
        let mut opened = AppState::new_in(32, 32, lang);
        opened.apply(Action::OpenProject(path.clone()));
        assert!(
            opened
                .message
                .starts_with(lang.pick("開きました", "Opened")),
            "{}",
            opened.message
        );
        assert_eq!(opened.doc.layers().len(), s.doc.layers().len());
        for l in s.doc.layers() {
            let again = opened.doc.layer(l.id()).unwrap();
            assert_eq!(again.locks(), l.locks(), "{}", l.name());
            assert_eq!(
                opened.doc.effective_locks(l.id()).unwrap(),
                s.doc.effective_locks(l.id()).unwrap(),
                "{}",
                l.name()
            );
        }
        // 開き直した文書でも、ロックは操作を断る
        opened.select_single_layer(a);
        let revision = opened.doc.revision();
        edit(
            &mut opened,
            Edit::Transform(Xform::Flip { horizontal: true }),
        );
        assert_eq!(opened.doc.revision(), revision, "{lang:?}");
        // ロックを全部外して保存し直すと、ロックを付ける前に書いた文書とバイトまで同じ（ロックの印・値がファイルに残らない）
        let all = LayerLocks::from_bits(15).unwrap();
        edit(
            &mut opened,
            Edit::Lock {
                ids: vec![a, group],
                flag: all,
                on: false,
            },
        );
        for l in opened.doc.layers() {
            assert_eq!(l.locks(), LayerLocks::NONE, "{}", l.name());
        }
        let unlocked_path = root.join(lang.pick("ja-unlocked.ylp", "en-unlocked.ylp"));
        opened.apply(Action::SaveProjectAs(unlocked_path.clone()));
        assert!(
            opened
                .message
                .starts_with(lang.pick("保存しました", "Saved")),
            "{}",
            opened.message
        );
        assert_eq!(
            std::fs::read(&unlocked_path).unwrap(),
            std::fs::read(&plain_path).unwrap(),
            "{lang:?}"
        );
        assert_ne!(
            std::fs::read(&path).unwrap(),
            std::fs::read(&plain_path).unwrap(),
            "ロックを付けた版は書き分けられる"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

// ───────── 見た目（スナップショット） ─────────

/// 4 つのレイヤー（下から 1 から 4）。2 と 3 を選び、1 は自分のロック、4 は上のグループのロックが効く。
fn showcase(lang: Lang) -> Harness<'static, YoluApp> {
    let mut h = app(1280.0, 800.0, 128);
    h.state_mut().state.set_language(lang);
    let layers = named_layers(&mut h, 4);
    {
        let s = &mut h.state_mut().state;
        for (i, id) in layers.iter().enumerate() {
            let i = i as u32;
            fill_rect(
                &mut s.doc,
                *id,
                10 + i * 18,
                20 + i * 12,
                50 + i * 18,
                60 + i * 12,
                [RED, GREEN, BLUE, RED][i as usize],
            );
        }
        s.apply(Action::M2(Edit::Lock {
            ids: vec![layers[0]],
            flag: LayerLocks::PIXELS | LayerLocks::TRANSPARENCY,
            on: true,
        }));
        s.select_single_layer(layers[3]);
        s.apply(Action::M2(Edit::GroupSelected));
        let group = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::Lock {
            ids: vec![group],
            flag: LayerLocks::POSITION,
            on: true,
        }));
        s.select_layers([layers[1], layers[2]], layers[2]);
        settle(s);
    }
    h.run();
    h
}

fn drawn_texts(shape: &egui::epaint::Shape, out: &mut Vec<String>) {
    match shape {
        egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|s| drawn_texts(s, out)),
        egui::epaint::Shape::Text(text) => out.push(text.galley.job.text.clone()),
        _ => {}
    }
}

/// いま描いた文字のうち、日本語を含むもの（英語の画面に日本語が残っていないかの見張り。言語の選択肢の名前は除く）。
fn japanese_drawn(h: &Harness<'_, YoluApp>) -> Vec<String> {
    let mut texts = Vec::new();
    for shape in &h.output().shapes {
        drawn_texts(&shape.shape, &mut texts);
    }
    texts
        .into_iter()
        .filter(|t| has_japanese(t) && !t.contains("日本語"))
        .collect()
}

#[test]
fn snapshot_multi_selection_and_lock_marks_in_both_languages() {
    let mut results = egui_kittest::SnapshotResults::new();
    for lang in Lang::ALL {
        let mut h = showcase(lang);
        h.snapshot(format!("layerops_layers_{}", lang.pick("ja", "en")));
        // 見張りが働いていること: 日本語の画面には日本語がある。英語の画面には残らない
        assert_eq!(
            japanese_drawn(&h).is_empty(),
            lang == Lang::En,
            "{:?}",
            japanese_drawn(&h)
        );
        results.extend_harness(&mut h);
    }
}

#[test]
fn snapshot_the_move_tool_with_its_handles_and_properties_in_both_languages() {
    let mut results = egui_kittest::SnapshotResults::new();
    for lang in Lang::ALL {
        let mut h = showcase(lang);
        h.state_mut().state.apply(Action::SelectTool(Tool::Move));
        h.run();
        h.snapshot(format!("layerops_move_tool_{}", lang.pick("ja", "en")));
        assert_eq!(
            japanese_drawn(&h).is_empty(),
            lang == Lang::En,
            "{:?}",
            japanese_drawn(&h)
        );
        results.extend_harness(&mut h);
    }
}

#[test]
fn snapshot_the_layer_menu_and_the_merge_confirmation() {
    let mut results = egui_kittest::SnapshotResults::new();
    let mut h = showcase(Lang::Ja);
    // 選んだレイヤーの行を右クリックしたメニュー
    let at = row_center(&h, "レイヤー 3");
    press(&h, at, PointerButton::Secondary);
    h.step();
    release(&h, at, PointerButton::Secondary);
    h.run();
    assert!(h.state().state.popup.is_some());
    h.snapshot("layerops_layer_menu");
    results.extend_harness(&mut h);
    // 英語のメニュー（開いている項目に日本語が残らない）
    let mut h = showcase(Lang::En);
    let at = row_center(&h, "Layer 3");
    press(&h, at, PointerButton::Secondary);
    h.step();
    release(&h, at, PointerButton::Secondary);
    h.run();
    assert!(h.state().state.popup.is_some());
    assert!(japanese_drawn(&h).is_empty(), "{:?}", japanese_drawn(&h));
    // 見た目が変わる結合の確かめ
    let mut h = app(1280.0, 800.0, 64);
    {
        let s = &mut h.state_mut().state;
        let a = s.selected_layer.unwrap();
        fill_rect(&mut s.doc, a, 0, 0, 64, 64, GREEN);
        let b = add_painted(s, (10, 10, 40, 40), RED);
        s.doc.set_layer_blend_mode(b, BlendMode::Multiply).unwrap();
        add_painted(s, (20, 20, 50, 50), BLUE);
        settle(s);
        s.apply(Action::M2(Edit::MergeDown));
    }
    h.run();
    h.snapshot("layerops_merge_confirm");
    results.extend_harness(&mut h);
}

// ───────── キャンバスの表示が編集に追いつく（CPU と GPU の道の両方） ─────────

/// ウィンドウの絵（描いた画面）の、キャンバスの点（画素の中心）の色。
fn displayed(h: &mut Harness<'_, YoluApp>, x: f64, y: f64) -> [u8; 4] {
    let image = h.render().expect("描ける");
    let rect = canvas_rect(h);
    let doc = &h.state().state.doc;
    let view = h.state().state.view.view(rect, doc.width(), doc.height());
    let p = view.to_screen(x + 0.5, y + 0.5);
    image.get_pixel(p.x.round() as u32, p.y.round() as u32).0
}

fn is_red(p: [u8; 4]) -> bool {
    p[0] > 200 && p[1] < 60 && p[2] < 60
}

/// 移動・反転・90° 回転・結合・取り消しのあと、ウィンドウに描いたキャンバスの絵が文書の中身に追いつく（変わったタイルの記録が、空いた所と
/// 埋まった所の両方を指している。CPU の表示と GPU の表示のどちらでも）。
#[test]
fn the_displayed_canvas_follows_moves_flips_merges_and_undo_on_both_canvas_paths() {
    use yolu_app::canvas::gpu::CanvasBackend;
    for policy in [CanvasBackend::Cpu, CanvasBackend::Gpu] {
        let mut h = app(1280.0, 800.0, 128);
        h.state_mut().set_canvas_backend(policy);
        h.run();
        let id = h.state().state.selected_layer.unwrap();
        {
            let s = &mut h.state_mut().state;
            fill_rect(&mut s.doc, id, 10, 10, 40, 40, RED);
            settle(s);
            s.apply(Action::SelectTool(Tool::Move));
        }
        h.run();
        assert!(is_red(displayed(&mut h, 20.0, 20.0)), "{policy:?}");
        assert!(!is_red(displayed(&mut h, 80.0, 20.0)), "{policy:?}");
        // ドラッグで (60, 0) 動かす
        let from = screen_of(&h, 25.0, 25.0);
        let to = screen_of(&h, 85.0, 25.0);
        drag(&mut h, &[from, offset(from, 4.0, 0.0), to]);
        assert!(
            !is_red(displayed(&mut h, 20.0, 20.0)),
            "{policy:?}: 空いた所"
        );
        assert!(
            is_red(displayed(&mut h, 80.0, 20.0)),
            "{policy:?}: 埋まった所"
        );
        // 取り消す
        undo(&mut h.state_mut().state);
        h.run();
        assert!(
            is_red(displayed(&mut h, 20.0, 20.0)),
            "{policy:?}: 取り消し"
        );
        assert!(
            !is_red(displayed(&mut h, 80.0, 20.0)),
            "{policy:?}: 取り消し"
        );
        // 左右反転（範囲 10..40 の中の鏡像なので位置は同じ）と、90° 回転した後の形（正方形なので同じ）。上下反転は同じ所のまま
        h.state_mut()
            .state
            .apply(Action::M2(Edit::Transform(Xform::Flip {
                horizontal: true,
            })));
        h.run();
        assert!(is_red(displayed(&mut h, 20.0, 20.0)), "{policy:?}: 反転");
        // 結合: 上のレイヤー（緑）を下のレイヤーへ。結合した絵がウィンドウに出る
        {
            let s = &mut h.state_mut().state;
            let top = add_painted(s, (30, 30, 60, 60), GREEN);
            let _ = top;
            settle(s);
        }
        h.run();
        let before_merge = displayed(&mut h, 50.0, 50.0);
        assert!(!is_red(before_merge) && before_merge[1] > 200, "{policy:?}");
        h.state_mut().state.apply(Action::M2(Edit::MergeDown));
        h.run();
        assert_eq!(h.state().state.doc.layers().len(), 1, "{policy:?}");
        assert_eq!(
            displayed(&mut h, 50.0, 50.0),
            before_merge,
            "{policy:?}: 結合しても同じ絵"
        );
        assert!(is_red(displayed(&mut h, 20.0, 20.0)), "{policy:?}");
        undo(&mut h.state_mut().state);
        h.run();
        assert_eq!(h.state().state.doc.layers().len(), 2, "{policy:?}");
        assert_eq!(displayed(&mut h, 50.0, 50.0), before_merge, "{policy:?}");
    }
}
