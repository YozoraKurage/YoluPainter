use super::*;
use crate::fx::FxOp;
use crate::m2::layersets::LayerSetEdit;
use crate::m2::Edit;
use crate::state::Action;
use yolu_core::Rgba8;

const RED: Rgba8 = Rgba8::new(200, 10, 20, 255);
const BLUE: Rgba8 = Rgba8::new(10, 20, 220, 255);

/// 2 つのセット（A・B）。B の最初のレイヤーの (5, 7) に赤を描き、A を今のセットに戻す。返すのは B の uid と、B の最初のレイヤーセットを指す鍵。
fn two_sets() -> (AppState, u32, ExternalSet) {
    let mut s = AppState::new(256, 256);
    let b = s.add_texture_set().unwrap();
    paint(&mut s, 5, 7, RED);
    let key = own_key(&s);
    s.switch_set(0).unwrap();
    (s, b, key)
}

/// ポインターレイヤーの読む元を、色だけにする（ほかのチャンネルを外す。作り直しの数を色の分だけにする）。
fn only_color(s: &mut AppState, id: LayerId) {
    for c in s.session.doc.channels() {
        if c != Channel::Color {
            s.session.doc.set_channel_enabled(id, c, false).unwrap();
        }
    }
}

/// 今のセットの最初のレイヤーセットを指す鍵。
fn own_key(s: &AppState) -> ExternalSet {
    ExternalSet {
        texture_set: set_key(&s.session.sets.current().id).unwrap(),
        layer_set: s.session.doc.layer_sets()[0].id.0,
    }
}

/// 今のセットの一番下のラスターレイヤーに 1 画素描く。
fn paint(s: &mut AppState, x: u32, y: u32, color: Rgba8) {
    let raster = s
        .session
        .doc
        .layers()
        .iter()
        .find(|l| l.kind() == LayerKind::Raster)
        .map(|l| l.id())
        .unwrap();
    s.session.doc.set_pixel(raster, x, y, color).unwrap();
}

/// 今のセットに、指す先なしのポインターレイヤーを作って `target` を指させる。
fn pointer_to(s: &mut AppState, target: PointerTarget) -> LayerId {
    s.apply(Action::Fx(FxOp::Pointer(PointerOp::New(None))));
    let id = s.session.selected_layer.unwrap();
    s.apply(Action::Fx(FxOp::Pointer(PointerOp::Target {
        layer: id,
        target,
    })));
    id
}

fn output(s: &AppState, id: LayerId, x: u32, y: u32) -> Rgba8 {
    s.session
        .doc
        .layer_output_pixel(id, Channel::Color, x, y)
        .unwrap()
}

#[test]
fn the_menu_makes_a_pointer_without_a_target_in_one_undo_step() {
    let mut s = AppState::new(64, 64);
    let layers = s.session.doc.layers().len();
    let undo = s.session.doc.undo_count();
    s.apply(Action::Fx(FxOp::Pointer(PointerOp::New(None))));
    let id = s.session.selected_layer.unwrap();
    assert_eq!(s.session.doc.layer(id).unwrap().kind(), LayerKind::Pointer);
    assert_eq!(s.session.doc.undo_count(), undo + 1, "1 回の取り消し");
    s.sync_effect_inputs();
    assert_eq!(
        s.pointer_layer_issue(id),
        Some(PointerIssue::MissingTarget),
        "指す先が無い"
    );
    assert_eq!(s.pointer_target_label(PointerTarget::External(UNSET)), "");
    s.apply(Action::Undo);
    assert_eq!(s.session.doc.layers().len(), layers);
}

#[test]
fn the_context_menu_makes_a_pointer_to_that_layer_in_one_undo_step() {
    let mut s = AppState::new(64, 64);
    paint(&mut s, 3, 4, BLUE);
    let raster = s.session.selected_layer.unwrap();
    let undo = s.session.doc.undo_count();
    s.apply(Action::Fx(FxOp::Pointer(PointerOp::New(Some(raster)))));
    let id = s.session.selected_layer.unwrap();
    assert_ne!(id, raster);
    assert_eq!(s.session.doc.undo_count(), undo + 1);
    assert_eq!(
        s.session.doc.layer(id).unwrap().pointer().unwrap().target(),
        PointerTarget::Layer(raster)
    );
    assert_eq!(output(&s, id, 3, 4), BLUE);
    assert_eq!(s.pointer_layer_issue(id), None);
}

#[test]
fn the_candidates_list_three_kinds_and_refuse_itself_with_a_reason() {
    let (mut s, _, key) = two_sets();
    let id = pointer_to(&mut s, PointerTarget::External(UNSET));
    s.apply(Action::M2(Edit::GroupSelected));
    let group = s.session.doc.layer(id).unwrap().parent().unwrap();
    let sections = s.pointer_candidates(id);
    assert_eq!(
        sections.len(),
        3,
        "レイヤー・グループ、レイヤーセット、ほかのセット"
    );
    let find = |t: PointerTarget| {
        sections
            .iter()
            .flat_map(|sec| sec.items.iter())
            .find(|c| c.target == t)
            .cloned()
            .unwrap()
    };
    assert_eq!(
        find(PointerTarget::Layer(id)).refused,
        Some(PointerIssue::Cycle)
    );
    let g = find(PointerTarget::Layer(group));
    assert_eq!(g.refused, Some(PointerIssue::Cycle), "自分の入ったグループ");
    assert_eq!(
        find(PointerTarget::Layer(id)).depth,
        1,
        "グループの中は字下げ"
    );
    let own_set = PointerTarget::LayerSet(s.session.doc.layer_sets()[0].id);
    assert_eq!(find(own_set).refused, Some(PointerIssue::Cycle));
    assert_eq!(find(PointerTarget::External(key)).refused, None);
    assert_eq!(sections[2].heading, s.session.sets.get(1).unwrap().name);
}

#[test]
fn a_read_only_set_is_not_offered_as_a_target_until_it_can_be_edited() {
    let (mut s, b, key) = two_sets();
    let id = pointer_to(&mut s, PointerTarget::External(UNSET));
    let bi = s.session.sets.index_of(b).unwrap();
    s.session.sets.get_mut(bi).unwrap().read_only = Some("読むだけ".into());
    let headings = |s: &AppState| -> Vec<String> {
        s.pointer_candidates(id)
            .into_iter()
            .map(|sec| sec.heading)
            .collect()
    };
    let name = s.session.sets.get(bi).unwrap().name.clone();
    assert!(
        !headings(&s).contains(&name),
        "読むだけのセットは候補に出ない"
    );
    assert_eq!(headings(&s).len(), 2);
    // 読む元のチャンネルも、仮の文書のものは並べない（この文書のチャンネル）
    let own: Vec<Channel> = s.session.doc.channels();
    let listed: Vec<Channel> = s
        .pointer_source_channels(PointerTarget::External(key))
        .into_iter()
        .map(|(c, _)| c)
        .collect();
    assert_eq!(listed, own);
    // 編集できるようになったら出る
    s.session.sets.get_mut(bi).unwrap().read_only = None;
    assert!(headings(&s).contains(&name));
    assert_eq!(headings(&s).len(), 3);
}

#[test]
fn a_pointer_reads_the_other_sets_composite_and_follows_it_only_when_it_changes() {
    let (mut s, b, key) = two_sets();
    let c = s.add_texture_set().unwrap();
    s.switch_set(0).unwrap();
    let id = pointer_to(&mut s, PointerTarget::External(key));
    only_color(&mut s, id);
    s.sync_effect_inputs();
    assert_eq!(output(&s, id, 5, 7), RED, "向こうの合成");
    assert_eq!(s.pointer_layer_issue(id), None);
    let built = (
        s.fx.inputs.pointers.built_full,
        s.fx.inputs.pointers.built_tiles,
    );
    // 関係ないセットに描いても作り直さない
    let ci = s.session.sets.index_of(c).unwrap();
    s.switch_set(ci).unwrap();
    paint(&mut s, 1, 1, BLUE);
    s.sync_effect_inputs();
    assert_eq!(
        (
            s.fx.inputs.pointers.built_full,
            s.fx.inputs.pointers.built_tiles
        ),
        built
    );
    // 向こうに描くと、変わったタイルだけ作り直して追従する
    let bi = s.session.sets.index_of(b).unwrap();
    s.switch_set(bi).unwrap();
    paint(&mut s, 40, 50, BLUE);
    s.sync_effect_inputs();
    assert_eq!(
        (
            s.fx.inputs.pointers.built_full,
            s.fx.inputs.pointers.built_tiles
        ),
        (built.0, built.1 + 1),
        "変わったタイルだけ"
    );
    s.switch_set(0).unwrap();
    assert_eq!(output(&s, id, 40, 50), BLUE);
    assert_eq!(output(&s, id, 5, 7), RED);
    // 書き出しが写す文書にも入っている
    let snapshot = s.session.doc.capture_snapshot().unwrap();
    assert_eq!(
        snapshot
            .layer_output_pixel(id, Channel::Color, 40, 50)
            .unwrap(),
        BLUE
    );
}

/// A のポインターレイヤーが B のレイヤーセットを、B のポインターレイヤーが C のレイヤーセットを指す 3 つのセット（C の (5, 7) に赤）。
/// 返すのは A のポインターレイヤーと、B・C の uid。今のセットは A。
fn chain() -> (AppState, LayerId, u32, u32) {
    let mut s = AppState::new(256, 256);
    let b = s.add_texture_set().unwrap();
    let c = s.add_texture_set().unwrap();
    paint(&mut s, 5, 7, RED);
    let key_c = own_key(&s);
    let bi = s.session.sets.index_of(b).unwrap();
    s.switch_set(bi).unwrap();
    let b_ptr = pointer_to(&mut s, PointerTarget::External(key_c));
    only_color(&mut s, b_ptr);
    let key_b = own_key(&s);
    s.switch_set(0).unwrap();
    let a_ptr = pointer_to(&mut s, PointerTarget::External(key_b));
    only_color(&mut s, a_ptr);
    (s, a_ptr, b, c)
}

#[test]
fn a_chain_of_pointers_follows_in_one_sync() {
    let (mut s, a_ptr, _, c) = chain();
    s.sync_effect_inputs();
    assert_eq!(output(&s, a_ptr, 5, 7), RED, "A → B → C");
    // C に描いた次の同期で、B を通って A まで追従する
    let ci = s.session.sets.index_of(c).unwrap();
    s.switch_set(ci).unwrap();
    paint(&mut s, 40, 50, BLUE);
    s.switch_set(0).unwrap();
    s.sync_effect_inputs();
    assert_eq!(output(&s, a_ptr, 40, 50), BLUE);
    assert_eq!(output(&s, a_ptr, 5, 7), RED);
}

#[test]
fn the_other_set_changing_only_a_property_or_the_layer_set_channels_is_followed() {
    let (mut s, a_ptr, b, c) = chain();
    s.sync_effect_inputs();
    assert_eq!(output(&s, a_ptr, 5, 7), RED);
    let ci = s.session.sets.index_of(c).unwrap();
    s.switch_set(ci).unwrap();
    let raster = s
        .session
        .doc
        .layers()
        .iter()
        .find(|l| l.kind() == LayerKind::Raster)
        .map(|l| l.id())
        .unwrap();
    // 不透明度だけ
    s.session.doc.set_layer_opacity(raster, 0.0, false).unwrap();
    s.switch_set(0).unwrap();
    s.sync_effect_inputs();
    assert_ne!(output(&s, a_ptr, 5, 7), RED, "不透明度 0 の合成");
    // 向こうでの取り消し（B の先の C を取り消す）
    s.switch_set(ci).unwrap();
    s.apply(Action::Undo);
    s.switch_set(0).unwrap();
    s.sync_effect_inputs();
    assert_eq!(output(&s, a_ptr, 5, 7), RED, "取り消しに追従");
    // 表示だけ
    s.switch_set(ci).unwrap();
    s.session.doc.set_layer_visible(raster, false).unwrap();
    s.switch_set(0).unwrap();
    s.sync_effect_inputs();
    assert_ne!(output(&s, a_ptr, 5, 7), RED, "隠した合成");
    s.switch_set(ci).unwrap();
    s.apply(Action::Undo);
    s.switch_set(0).unwrap();
    s.sync_effect_inputs();
    assert_eq!(output(&s, a_ptr, 5, 7), RED);
    // レイヤーセットが扱うチャンネルの組だけ（色を外す）
    s.switch_set(ci).unwrap();
    let set = s.session.doc.layer_sets()[0].id;
    let bits = s.session.doc.layer_set_channels(set).unwrap();
    s.session
        .doc
        .set_layer_set_channels(set, bits & !Channel::Color.bit())
        .unwrap();
    s.switch_set(0).unwrap();
    s.sync_effect_inputs();
    assert_ne!(output(&s, a_ptr, 5, 7), RED, "色を扱わないレイヤーセット");
    let _ = b;
}

#[test]
fn the_source_channel_can_be_changed() {
    let (mut s, b, key) = two_sets();
    // B の Roughness に描く
    let bi = s.session.sets.index_of(b).unwrap();
    s.switch_set(bi).unwrap();
    let rough = Channel::Roughness;
    assert!(s.session.doc.channels().contains(&rough));
    let raster = s
        .session
        .doc
        .layers()
        .iter()
        .find(|l| l.kind() == LayerKind::Raster)
        .unwrap()
        .id();
    s.session
        .doc
        .set_channel_enabled(raster, rough, true)
        .unwrap();
    s.session
        .doc
        .set_channel_pixel(raster, rough, 2, 2, Rgba8::new(77, 77, 77, 255))
        .unwrap();
    s.switch_set(0).unwrap();
    let id = pointer_to(&mut s, PointerTarget::External(key));
    let sources = s.pointer_source_channels(PointerTarget::External(key));
    assert!(
        sources.iter().any(|(c, _)| *c == rough),
        "向こうのチャンネル"
    );
    s.apply(Action::Fx(FxOp::Pointer(PointerOp::Source {
        layer: id,
        channel: Channel::Color,
        source: Some(rough),
    })));
    s.sync_effect_inputs();
    assert_eq!(output(&s, id, 2, 2), Rgba8::new(77, 77, 77, 255));
    s.apply(Action::Undo);
    s.sync_effect_inputs();
    assert_eq!(output(&s, id, 5, 7), RED, "取り消すと同じチャンネル");
}

#[test]
fn over_budget_and_missing_targets_are_told() {
    let (mut s, b, key) = two_sets();
    let id = pointer_to(&mut s, PointerTarget::External(key));
    s.fx.inputs.pointers.budget_override = Some(1024);
    s.sync_effect_inputs();
    let list = s.inactive_effects_of(0);
    assert!(
        list.iter()
            .any(|e| e.layer == id
                && e.reason == InactiveReason::Pointer(PointerIssue::InputOverBudget)),
        "{list:?}"
    );
    assert_eq!(s.fx.inputs.pointers.held_bytes(), 0);
    s.fx.inputs.pointers.budget_override = None;
    s.sync_effect_inputs();
    assert_eq!(output(&s, id, 5, 7), RED);
    // 指されたセットを消すと「指す先が無い」
    s.remove_sets(&[b]).unwrap();
    s.sync_effect_inputs();
    assert_eq!(s.pointer_layer_issue(id), Some(PointerIssue::MissingTarget));
    assert_ne!(output(&s, id, 5, 7), RED);
}

#[test]
fn a_waiting_set_that_cannot_be_converted_is_over_the_budget() {
    let (mut s, b, key) = two_sets();
    let id = pointer_to(&mut s, PointerTarget::External(key));
    let bi = s.session.sets.index_of(b).unwrap();
    let set = s.session.sets.get_mut(bi).unwrap();
    set.read_only = Some("待ち".into());
    set.waiting_inputs = true;
    s.sync_effect_inputs();
    let list = s.inactive_effects_of(0);
    assert!(
        list.iter()
            .any(|e| e.layer == id
                && e.reason == InactiveReason::Pointer(PointerIssue::InputOverBudget)),
        "{list:?}"
    );
    // 失敗は、予算が替わるまで試し直さない（毎フレーム 4096² の正本を変換しない）
    assert_eq!(s.fx.inputs.pointers.converted, 1);
    s.sync_effect_inputs();
    s.sync_effect_inputs();
    assert_eq!(s.fx.inputs.pointers.converted, 1, "2 回目からは変換しない");
    let list = s.inactive_effects_of(0);
    assert!(
        list.iter()
            .any(|e| e.layer == id
                && e.reason == InactiveReason::Pointer(PointerIssue::InputOverBudget)),
        "理由は出したまま: {list:?}"
    );
}

/// 入力待ち（プロジェクトにまだ無い画像を指す塗りつぶしレイヤーがある）の読むだけのセット B を、A のポインターレイヤーが指す .ylp を開く。
/// 返すのは、A のポインターレイヤーと、B の鍵。
fn open_with_waiting_target(tag: &str) -> (AppState, LayerId, ExternalSet) {
    let dir =
        std::env::temp_dir().join(format!("yolu-pointer-waiting-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("waiting.ylp");
    let mut source = AppState::new(256, 256);
    source.add_texture_set().unwrap();
    paint(&mut source, 5, 7, RED);
    let key = own_key(&source);
    source.apply(Action::M2(Edit::NewFill));
    let fill = source.session.selected_layer.unwrap();
    source
        .session
        .doc
        .set_fill_images_for_load(
            fill,
            &[(
                Channel::Color,
                crate::fx::inputs::image_id("00000000-0000-4000-8000-000000000001").unwrap(),
            )],
            Default::default(),
        )
        .unwrap();
    source.switch_set(0).unwrap();
    let id = pointer_to(&mut source, PointerTarget::External(key));
    source.apply(Action::SaveProjectAs(path.clone()));
    assert!(!source.session.is_modified(), "{}", source.message);
    let mut s = AppState::new(64, 64);
    crate::project::open_within(&mut s, &path, u64::MAX);
    let _ = std::fs::remove_dir_all(&dir);
    (s, id, key)
}

#[test]
fn a_waiting_set_is_converted_once_while_the_pointer_reads_it() {
    let (mut s, id, key) = open_with_waiting_target("once");
    let bi = s.set_index_of_key(key.texture_set).unwrap();
    assert!(
        s.session.sets.get(bi).unwrap().waiting_inputs,
        "{}",
        s.message
    );
    s.sync_effect_inputs();
    assert_eq!(
        s.pointer_layer_issue(id),
        None,
        "{:?}",
        s.inactive_effects_of(0)
    );
    assert_eq!(output(&s, id, 5, 7), RED);
    let first = s.fx.inputs.pointers.converted;
    assert!(first >= 1);
    s.sync_effect_inputs();
    s.sync_effect_inputs();
    assert_eq!(
        s.fx.inputs.pointers.converted, first,
        "作ってある合成を使い、毎フレーム変換し直さない"
    );
    assert_eq!(output(&s, id, 5, 7), RED);
}

#[test]
fn cycles_across_texture_sets_are_refused_when_pointing() {
    let (mut s, b, key_b) = two_sets();
    let key_a = own_key(&s);
    let a_ptr = pointer_to(&mut s, PointerTarget::External(key_b));
    let bi = s.session.sets.index_of(b).unwrap();
    s.switch_set(bi).unwrap();
    // 候補では押せない
    s.apply(Action::Fx(FxOp::Pointer(PointerOp::New(None))));
    let b_ptr = s.session.selected_layer.unwrap();
    let refused = s
        .pointer_candidates(b_ptr)
        .into_iter()
        .flat_map(|sec| sec.items)
        .find(|c| c.target == PointerTarget::External(key_a))
        .unwrap()
        .refused;
    assert_eq!(refused, Some(PointerIssue::Cycle));
    // 押しても断る（何も変えない）
    let undo = s.session.doc.undo_count();
    s.apply(Action::Fx(FxOp::Pointer(PointerOp::Target {
        layer: b_ptr,
        target: PointerTarget::External(key_a),
    })));
    assert_eq!(s.session.doc.undo_count(), undo);
    assert_eq!(s.session.doc.redo_count(), 0);
    assert_eq!(
        s.session
            .doc
            .layer(b_ptr)
            .unwrap()
            .pointer()
            .unwrap()
            .target(),
        PointerTarget::External(UNSET)
    );
    assert!(s.message.contains("循環"), "{}", s.message);
    let _ = a_ptr;
}

#[test]
fn a_structure_edit_that_makes_a_cycle_across_sets_is_undone() {
    let (mut s, b, key_b) = two_sets();
    // A に 2 つ目のレイヤーセットを足し、そこに B を指すポインターレイヤー
    s.apply(Action::M2(Edit::LayerSets(LayerSetEdit::New)));
    let a_ptr = pointer_to(&mut s, PointerTarget::External(key_b));
    let first = s.session.doc.layer_sets()[0].id;
    let key_a0 = ExternalSet {
        texture_set: set_key(&s.session.sets.current().id).unwrap(),
        layer_set: first.0,
    };
    // B は A の最初のレイヤーセットを指す（まだ循環ではない）
    let bi = s.session.sets.index_of(b).unwrap();
    s.switch_set(bi).unwrap();
    let b_ptr = pointer_to(&mut s, PointerTarget::External(key_a0));
    assert_eq!(
        s.session
            .doc
            .layer(b_ptr)
            .unwrap()
            .pointer()
            .unwrap()
            .target(),
        PointerTarget::External(key_a0)
    );
    // A のポインターレイヤーを最初のレイヤーセットへ移すと循環: 戻して断る
    s.switch_set(0).unwrap();
    let undo = s.session.doc.undo_count();
    s.apply(Action::M2(Edit::LayerSets(LayerSetEdit::MoveLayers {
        ids: vec![a_ptr],
        set: first,
    })));
    assert_ne!(s.session.doc.layer(a_ptr).unwrap().layer_set(), first);
    assert_eq!(s.session.doc.undo_count(), undo);
    assert!(s.message.contains("循環"), "{}", s.message);
}

#[test]
fn depth_across_texture_sets_is_limited() {
    let mut s = AppState::new(32, 32);
    // 6 つのセット: 1 → 2 → … → 6 と順に指す。5 段目（5 → 6）で 4 段を超える
    let mut uids = vec![s.session.sets.current().uid];
    for _ in 0..5 {
        uids.push(s.add_texture_set().unwrap());
    }
    let keys: Vec<ExternalSet> = uids
        .iter()
        .map(|u| {
            let i = s.session.sets.index_of(*u).unwrap();
            ExternalSet {
                texture_set: set_key(&s.session.sets.get(i).unwrap().id).unwrap(),
                layer_set: s.set_doc(i).layer_sets()[0].id.0,
            }
        })
        .collect();
    // 後ろから作る（深い方から）
    let mut made = Vec::new();
    for k in (0..5).rev() {
        let i = s.session.sets.index_of(uids[k]).unwrap();
        s.switch_set(i).unwrap();
        let id = pointer_to(&mut s, PointerTarget::External(keys[k + 1]));
        let target = s.session.doc.layer(id).unwrap().pointer().unwrap().target();
        made.push(target == PointerTarget::External(keys[k + 1]));
    }
    assert_eq!(made, vec![true, true, true, true, false], "{}", s.message);
}

#[test]
fn undo_that_brings_back_a_cycle_across_sets_is_refused() {
    let (mut s, b, key_b) = two_sets();
    let key_a = own_key(&s);
    // B が A を指す → 消す
    let bi = s.session.sets.index_of(b).unwrap();
    s.switch_set(bi).unwrap();
    let b_ptr = pointer_to(&mut s, PointerTarget::External(key_a));
    s.apply(Action::DeleteLayer);
    assert!(s.session.doc.layer(b_ptr).is_none());
    // A が B を指す
    s.switch_set(0).unwrap();
    pointer_to(&mut s, PointerTarget::External(key_b));
    // B で取り消すと循環が戻る: やり直して断る
    s.switch_set(bi).unwrap();
    s.apply(Action::Undo);
    assert!(s.session.doc.layer(b_ptr).is_none(), "{}", s.message);
    assert!(s.message.contains("循環"), "{}", s.message);
}

/// 外からの命令（MCP と同じ道）を、起動中のアプリのホストに当てる。
fn mcp(
    s: &mut AppState,
    command: serde_json::Value,
) -> Result<yolu_ops::reply::Reply, yolu_ops::error::OpError> {
    let command = yolu_ops::parse_command(&command).unwrap();
    yolu_ops::execute(&mut crate::ops_host::AppHost::new(s), &command)
}

#[test]
fn an_mcp_undo_that_brings_back_a_cycle_across_sets_is_refused() {
    let (mut s, b, key_b) = two_sets();
    let key_a = own_key(&s);
    // B が A を指す → 消す。A が B を指す（ここまでは循環でない）
    let bi = s.session.sets.index_of(b).unwrap();
    s.switch_set(bi).unwrap();
    let b_ptr = pointer_to(&mut s, PointerTarget::External(key_a));
    s.apply(Action::DeleteLayer);
    s.switch_set(0).unwrap();
    pointer_to(&mut s, PointerTarget::External(key_b));
    let b_id = s.session.sets.get(bi).unwrap().id.clone();
    let (undo, redo) = {
        let d = s.set_doc(bi);
        (d.undo_count(), d.redo_count())
    };
    // B を MCP で取り消すと循環が戻る: 断って、取り消す前のまま
    let e = mcp(
        &mut s,
        serde_json::json!({"command": "undo", "args": {"set": b_id}}),
    )
    .unwrap_err();
    assert!(e.message.ja.contains("循環"), "{}", e.message.ja);
    let d = s.set_doc(bi);
    assert!(d.layer(b_ptr).is_none());
    assert_eq!((d.undo_count(), d.redo_count()), (undo, redo));
    // 循環にならない取り消しは通る（A 側の作成を取り消す）
    let a_id = s.session.sets.get(0).unwrap().id.clone();
    mcp(
        &mut s,
        serde_json::json!({"command": "undo", "args": {"set": a_id}}),
    )
    .unwrap();
}

#[test]
fn an_mcp_move_that_makes_the_chain_too_deep_is_refused() {
    let mut s = AppState::new(32, 32);
    // 5 つのセット: 1 → 2 → 3 → 4 → 5 と順に指す（4 段で上限ちょうど）
    let mut uids = vec![s.session.sets.current().uid];
    for _ in 0..4 {
        uids.push(s.add_texture_set().unwrap());
    }
    let keys: Vec<ExternalSet> = uids
        .iter()
        .map(|u| {
            let i = s.session.sets.index_of(*u).unwrap();
            ExternalSet {
                texture_set: set_key(&s.session.sets.get(i).unwrap().id).unwrap(),
                layer_set: s.set_doc(i).layer_sets()[0].id.0,
            }
        })
        .collect();
    let mut first_pointer = None;
    for k in (0..4).rev() {
        let i = s.session.sets.index_of(uids[k]).unwrap();
        s.switch_set(i).unwrap();
        let id = pointer_to(&mut s, PointerTarget::External(keys[k + 1]));
        let ok = s.session.doc.layer(id).unwrap().pointer().unwrap().target()
            == PointerTarget::External(keys[k + 1]);
        assert!(ok, "{}", s.message);
        first_pointer = Some(id);
    }
    // 1 つ目のセットに、空のグループを読む物（グループへ入れると、1 つ目の外を指す物を読む物ができて 1 段増える）
    let (p1, a_id) = (
        first_pointer.unwrap(),
        s.session
            .sets
            .get(s.session.sets.index_of(uids[0]).unwrap())
            .unwrap()
            .id
            .clone(),
    );
    let doc = &mut s.session.doc;
    let group = doc.add_group("G", None).unwrap();
    doc.add_pointer_layer("P0", PointerTarget::Layer(group), None)
        .unwrap();
    let undo = doc.undo_count();
    let e = mcp(
        &mut s,
        serde_json::json!({"command": "layer.move", "args": {"set": a_id, "layer": p1.to_string(), "parent": group.to_string()}}),
    )
    .unwrap_err();
    assert!(e.message.ja.contains("段"), "{}", e.message.ja);
    assert_eq!(
        s.session.doc.layer(p1).unwrap().parent(),
        None,
        "動かさない"
    );
    assert_eq!(s.session.doc.undo_count(), undo);
}

#[test]
fn a_recovered_project_with_a_cycle_across_sets_opens_with_the_reason() {
    let dir = std::env::temp_dir().join(format!("yolu-pointer-recovered-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (mut s, b, key_b) = two_sets();
    let a_ptr = pointer_to(&mut s, PointerTarget::External(key_b));
    let key_a = own_key(&s);
    let bi = s.session.sets.index_of(b).unwrap();
    let b_ptr = s
        .set_doc_mut(bi)
        .add_pointer_layer("p", PointerTarget::External(key_a), None)
        .unwrap();
    let cyclic = dir.join("cycle.ylp");
    s.apply(Action::SaveProjectAs(cyclic.clone()));
    // ふつうに開くと断る
    let mut u = AppState::new(16, 16);
    crate::project::open_into(&mut u, &cyclic);
    assert_eq!(u.session.sets.len(), 1, "開かない");
    // 復旧から開くと、断らずに理由つきで開く
    let project = yolu_io::Project::open(&cyclic, &yolu_io::Limits::default()).unwrap();
    crate::project::open_recovered(&mut u, project);
    assert_eq!(u.session.sets.len(), 2, "{}", u.message);
    u.sync_effect_inputs();
    for (key, layer) in [(key_a, a_ptr), (key_b, b_ptr)] {
        let index = u.set_index_of_key(key.texture_set).unwrap();
        let reason = u
            .inactive_effects_of(index)
            .into_iter()
            .find(|e| e.layer == layer && e.target == InactiveTarget::Pointer(None))
            .map(|e| e.reason);
        assert_eq!(reason, Some(InactiveReason::Pointer(PointerIssue::Cycle)));
    }
    assert_eq!(u.fx.inputs.pointers.held_bytes(), 0, "読まない");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saving_and_opening_keeps_the_pointer_and_a_cycle_across_sets_is_not_opened() {
    let dir = std::env::temp_dir().join(format!("yolu-pointer-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (mut s, b, key_b) = two_sets();
    let id = pointer_to(&mut s, PointerTarget::External(key_b));
    s.sync_effect_inputs();
    let path = dir.join("pointer.ylp");
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(!s.session.is_modified(), "{}", s.message);
    let mut t = AppState::new(16, 16);
    crate::project::open_into(&mut t, &path);
    assert_eq!(t.session.sets.len(), 2, "{}", t.message);
    t.sync_effect_inputs();
    assert_eq!(output(&t, id, 5, 7), RED, "開き直しても同じ合成");
    // 指す先を選んでいないポインターレイヤーも保存して開ける（指す先が無い）
    s.apply(Action::Fx(FxOp::Pointer(PointerOp::New(None))));
    let unset = s.session.selected_layer.unwrap();
    let path = dir.join("unset.ylp");
    s.apply(Action::SaveProjectAs(path.clone()));
    let mut t = AppState::new(16, 16);
    crate::project::open_into(&mut t, &path);
    assert!(t.session.doc.layer(unset).is_some(), "{}", t.message);
    t.sync_effect_inputs();
    assert_eq!(
        t.pointer_layer_issue(unset),
        Some(PointerIssue::MissingTarget)
    );

    // 循環（core は外を見ないので、画面を通さずに作る）は開かない
    let key_a = own_key(&s);
    let bi = s.session.sets.index_of(b).unwrap();
    s.set_doc_mut(bi)
        .add_pointer_layer("p", PointerTarget::External(key_a), None)
        .unwrap();
    let cyclic = dir.join("cycle.ylp");
    s.apply(Action::SaveProjectAs(cyclic.clone()));
    let mut u = AppState::new(16, 16);
    let before = u.session.doc.id();
    crate::project::open_into(&mut u, &cyclic);
    assert_eq!(u.session.doc.id(), before, "開かない");
    assert!(u.message.contains("循環"), "{}", u.message);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_cross_check_finds_cycles_and_depth() {
    let mut a = Document::new(16, 16).unwrap();
    let mut b = Document::new(16, 16).unwrap();
    let (ka, kb) = (1u128, 2u128);
    let sa = a.layer_sets()[0].id;
    let sb = b.layer_sets()[0].id;
    let la = a
        .add_pointer_layer(
            "a",
            PointerTarget::External(ExternalSet {
                texture_set: kb,
                layer_set: sb.0,
            }),
            None,
        )
        .unwrap();
    let docs = [SetDoc { key: ka, doc: &a }, SetDoc { key: kb, doc: &b }];
    assert_eq!(cross_check(&docs, None), Cross::default());
    let swap = Swap {
        key: ka,
        layer: la,
        target: PointerTarget::External(ExternalSet {
            texture_set: ka,
            layer_set: sa.0,
        }),
    };
    assert_eq!(
        cross_check(&docs, Some(&swap)).new_issue(&Cross::default()),
        Some(PointerIssue::Cycle),
        "自分のレイヤーセットを外の形で指す"
    );
    b.add_pointer_layer(
        "b",
        PointerTarget::External(ExternalSet {
            texture_set: ka,
            layer_set: sa.0,
        }),
        None,
    )
    .unwrap();
    let docs = [SetDoc { key: ka, doc: &a }, SetDoc { key: kb, doc: &b }];
    let cross = cross_check(&docs, None);
    assert_eq!(
        cross.new_issue(&Cross::default()),
        Some(PointerIssue::Cycle)
    );
    assert_eq!(set_key("00000000-0000-0000-0000-000000000000"), None);
}

/// core は読む側の無い入力を外し、あとから自動では戻さない: 読むチャンネルを切って取り消すと、渡し直して読める。
#[test]
fn an_input_dropped_by_the_core_is_delivered_again() {
    let (mut s, _, key) = two_sets();
    let id = pointer_to(&mut s, PointerTarget::External(key));
    s.sync_effect_inputs();
    assert_eq!(output(&s, id, 5, 7), RED);
    s.apply(Action::M2(Edit::ChannelEnabled {
        id,
        channel: Channel::Color,
        enabled: false,
    }));
    s.sync_effect_inputs();
    s.apply(Action::Undo);
    s.sync_effect_inputs();
    assert_eq!(output(&s, id, 5, 7), RED, "{:?}", s.inactive_effects_of(0));
}

#[test]
fn the_composites_held_for_pointers_count_against_the_effect_memory() {
    // ほかのセットを読む合成は効果のメモリに数え、持っている量を引いた残りを効果のキャッシュに配る
    let (mut s, _, key) = two_sets();
    let settings = s.settings();
    s.load_settings(settings);
    let id = pointer_to(&mut s, PointerTarget::External(key));
    only_color(&mut s, id);
    s.sync_effect_inputs();
    let held = s.fx.inputs.pointers.held_bytes();
    assert!(held > 0, "合成を持っている");
    let effects = s.prefs.settings.budgets(s.prefs.ram_mib).effects;
    assert_eq!(s.pointer_input_budget(), (effects / 4).max(MIN_BUDGET));
    s.sync_budgets();
    let total: u64 = (0..s.session.sets.len())
        .map(|i| s.set_doc(i).filter_cache_budget_bytes())
        .sum();
    assert!(
        total <= effects - held,
        "キャッシュの配り {total} は効果のメモリ {effects} から合成 {held} を引いた量まで"
    );
}
