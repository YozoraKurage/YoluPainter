//! 参照レイヤー: 同じ文書のレイヤー・グループ・レイヤーセットを指すと、その合成がそのレイヤーの出力になる（マスク・合成モード・不透明度・
//! フィルターが上に掛かる）、読む元のチャンネルを替えられる、指した先を描くと変わり関係ないレイヤーでは評価し直さない、循環と深さを断る、
//! 指した先を消すと既定値と理由（取り消しで戻る）、結合で画素になる、複製は指す先を写す、ほかのテクスチャセットの入力を UV で読む。
use std::sync::Arc;

use yolu_core::{
    BlendMode, Channel, CoreError, Document, EffectInputs, EffectSettings, ExternalSet, FilterSpec,
    FilterTarget, InactiveReason, InactiveTarget, LayerId, LayerSetId, PointerInput, PointerIssue,
    PointerTarget, Reason, Rect, Rgba8, TileCoord, MAX_POINTER_DEPTH,
};

const W: u32 = 32;
const H: u32 = 32;
const TS: u32 = 8;
const RED: Rgba8 = Rgba8::new(220, 30, 40, 255);
const BLUE: Rgba8 = Rgba8::new(20, 40, 230, 200);
const GREEN: Rgba8 = Rgba8::new(30, 200, 60, 160);
const WHITE: Rgba8 = Rgba8::new(255, 255, 255, 255);
const KEY: ExternalSet = ExternalSet {
    texture_set: 0x1111_2222_3333_4444_5555_6666_7777_8888,
    layer_set: 0x9999_aaaa_bbbb_cccc_dddd_eeee_ffff_0001,
};

fn doc() -> Document {
    Document::with_tile_size(W, H, TS).unwrap()
}

fn all(d: &Document) -> Rect {
    Rect::new(0, 0, d.width(), d.height())
}

/// 12×12 の四角を色で塗る（`at` が左下）。
fn paint(d: &mut Document, id: LayerId, color: Rgba8, at: (u32, u32)) {
    for y in at.1..at.1 + 12 {
        for x in at.0..at.0 + 12 {
            d.set_pixel(id, x, y, color).unwrap();
        }
    }
}

fn painted(d: &mut Document, name: &str, color: Rgba8, at: (u32, u32)) -> LayerId {
    let id = d.add_layer(name).unwrap();
    paint(d, id, color, at);
    id
}

fn output(d: &Document, id: LayerId, c: Channel) -> Vec<u8> {
    d.layer_output(id, c, all(d)).unwrap()
}

fn set_composite(d: &Document, set: LayerSetId, c: Channel) -> Vec<u8> {
    d.composite_set_channel(set, c, all(d)).unwrap()
}

/// 1 色で埋めた画素。
fn filled(v: Rgba8) -> Vec<u8> {
    v.to_array().repeat((W * H) as usize)
}

/// 左下の 6×6 を隠すマスク。
fn mask_corner(d: &mut Document, id: LayerId) {
    d.add_layer_mask(id).unwrap();
    for y in 0..6 {
        for x in 0..6 {
            d.set_mask_pixel(id, x, y, 255).unwrap();
        }
    }
}

fn first_set(d: &Document) -> LayerSetId {
    d.layer_sets()[0].id
}

fn pointer_issues(d: &Document) -> Vec<(Option<Channel>, PointerIssue)> {
    d.inactive_effect_list()
        .into_iter()
        .filter_map(|e| match (e.target, e.reason) {
            (InactiveTarget::Pointer(c), InactiveReason::Pointer(i)) => Some((c, i)),
            _ => None,
        })
        .collect()
}

fn refused(r: Result<impl std::fmt::Debug, CoreError>, why: Reason) {
    match r {
        Err(CoreError::InvalidArgument(r)) if r == why => {}
        other => panic!("{why:?} で断るはず: {other:?}"),
    }
}

#[test]
fn a_pointer_to_a_layer_outputs_that_layer_alone() {
    let mut d = doc();
    painted(&mut d, "下", BLUE, (8, 8));
    let target = painted(&mut d, "指す先", RED, (2, 2));
    let p = d
        .add_pointer_layer("参照", PointerTarget::Layer(target), None)
        .unwrap();
    // 下のレイヤー（青）は含めない: 指したレイヤーだけを透明から重ねた結果
    assert_eq!(
        output(&d, p, Channel::Color),
        output(&d, target, Channel::Color)
    );
    // 指したレイヤー自身の不透明度は掛かる
    d.set_layer_opacity(target, 0.5, false).unwrap();
    let half = output(&d, p, Channel::Color);
    let at = ((3 * W + 3) * 4) as usize;
    assert_eq!(&half[at..at + 3], &[RED.r, RED.g, RED.b]);
    assert_eq!(half[at + 3], 128);
    assert_eq!(&half[0..4], &[0, 0, 0, 0]);
}

#[test]
fn mask_mode_opacity_and_filters_apply_on_top_like_a_layer_with_the_same_pixels() {
    // 参照レイヤーと、指したレイヤーと同じ画素のラスターレイヤーを、同じ設定で同じ所に置いた 2 つの文書は、同じ合成になる
    let build = |pointer: bool| {
        let mut d = doc();
        let shelf = d.add_layer_set("別").unwrap();
        painted(&mut d, "下", BLUE, (8, 8));
        let source = d.add_layer_in_set("元", shelf, None).unwrap();
        paint(&mut d, source, RED, (2, 2));
        let top = if pointer {
            d.add_pointer_layer("参照", PointerTarget::Layer(source), None)
                .unwrap()
        } else {
            painted(&mut d, "同じ画素", RED, (2, 2))
        };
        d.set_layer_blend_mode(top, BlendMode::Multiply).unwrap();
        d.set_layer_opacity(top, 0.6, false).unwrap();
        mask_corner(&mut d, top);
        d.add_filter(
            top,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(2)),
        )
        .unwrap();
        d
    };
    let a = build(true);
    let b = build(false);
    for c in [Channel::Color, Channel::Roughness] {
        assert_eq!(
            set_composite(&a, first_set(&a), c),
            set_composite(&b, first_set(&b), c)
        );
    }
}

#[test]
fn a_pointer_to_a_group_and_to_a_layer_set_reads_their_composite() {
    let mut d = doc();
    let shelf = d.add_layer_set("別").unwrap();
    let a = d.add_layer_in_set("a", shelf, None).unwrap();
    paint(&mut d, a, RED, (2, 2));
    let b = d.add_layer_in_set("b", shelf, Some(a)).unwrap();
    paint(&mut d, b, GREEN, (8, 6));
    d.set_layer_blend_mode(b, BlendMode::Screen).unwrap();
    let group = d.group_layers(&[a, b], "グループ").unwrap();
    d.set_layer_opacity(group, 0.8, false).unwrap();
    let to_set = d
        .add_pointer_layer("セット", PointerTarget::LayerSet(shelf), None)
        .unwrap();
    let to_group = d
        .add_pointer_layer("グループ", PointerTarget::Layer(group), None)
        .unwrap();
    let whole = set_composite(&d, shelf, Channel::Color);
    assert_eq!(output(&d, to_set, Channel::Color), whole);
    // グループはレイヤーセットの唯一の段なので、グループだけの結果もレイヤーセットの合成と同じ
    assert_eq!(output(&d, to_group, Channel::Color), whole);
    // レイヤーセットの組に無いチャンネルは透明（既定値ではない）
    d.set_layer_set_channels(shelf, Channel::Color.bit())
        .unwrap();
    let rough = output(&d, to_set, Channel::Roughness);
    assert!(rough.iter().all(|b| *b == 0));
    assert!(pointer_issues(&d).is_empty());
}

#[test]
fn a_channel_can_read_another_channel_of_the_target_and_undo_restores_it() {
    let mut d = doc();
    let target = painted(&mut d, "指す先", RED, (2, 2));
    let p = d
        .add_pointer_layer("参照", PointerTarget::Layer(target), None)
        .unwrap();
    // 既定は同じチャンネル: 指す先の Roughness は無いので透明
    assert!(output(&d, p, Channel::Roughness).iter().all(|b| *b == 0));
    d.set_pointer_source(p, Channel::Roughness, Some(Channel::Color))
        .unwrap();
    assert_eq!(
        d.layer(p)
            .unwrap()
            .pointer()
            .unwrap()
            .source(Channel::Roughness),
        Channel::Color
    );
    // 値は変換しない（保存したままのバイト）
    assert_eq!(
        output(&d, p, Channel::Roughness),
        output(&d, target, Channel::Color)
    );
    d.undo().unwrap();
    assert!(output(&d, p, Channel::Roughness).iter().all(|b| *b == 0));
    d.redo().unwrap();
    // 読む元のチャンネルが文書に無い指し方は断る
    assert!(matches!(
        d.set_pointer_source(p, Channel::Roughness, Channel::from_index(40)),
        Err(CoreError::ChannelNotFound)
    ));
}

#[test]
fn drawing_the_target_changes_the_result_and_drawing_elsewhere_does_not_reevaluate() {
    let mut d = doc();
    let other = painted(&mut d, "関係ない", BLUE, (16, 16));
    let target = painted(&mut d, "指す先", RED, (2, 2));
    let p = d
        .add_pointer_layer("参照", PointerTarget::Layer(target), None)
        .unwrap();
    d.set_pointer_source(p, Channel::Roughness, Some(Channel::Color))
        .unwrap();
    let before = d.composite(all(&d)).unwrap();
    let _ = d.composite_channel(Channel::Roughness, all(&d)).unwrap();
    let evaluated = d.effect_counters().blocks_evaluated;
    // 関係ないレイヤーに描く: 合成は変わるが、参照レイヤーの評価はやり直さない
    d.set_pixel(other, 30, 30, GREEN).unwrap();
    let _ = d.composite(all(&d)).unwrap();
    let _ = d.composite_channel(Channel::Roughness, all(&d)).unwrap();
    assert_eq!(d.effect_counters().blocks_evaluated, evaluated);
    // 指した先に描く: 評価し直し、結果が変わる。読む元を替えたチャンネルにも、変わったタイルの印が付く
    let serial2 = d.change_serial();
    d.set_pixel(target, 20, 3, GREEN).unwrap();
    let after = d.composite(all(&d)).unwrap();
    assert!(d.effect_counters().blocks_evaluated > evaluated);
    assert_ne!(before, after);
    let at = ((3 * W + 20) * 4) as usize;
    assert_eq!(
        &output(&d, p, Channel::Color)[at..at + 4],
        &GREEN.to_array()
    );
    let changed = d.changed_tiles(Channel::Roughness, serial2).unwrap();
    assert!(changed.contains(&TileCoord::new(20 / TS, 3 / TS)));
}

#[test]
fn cycles_are_refused_directly_through_a_group_and_through_another_pointer() {
    let mut d = doc();
    let base = painted(&mut d, "元", RED, (2, 2));
    // 自分のレイヤーセット・自分自身
    refused(
        d.add_pointer_layer("輪", PointerTarget::LayerSet(first_set(&d)), None),
        Reason::PointerCycle,
    );
    let p = d
        .add_pointer_layer("参照", PointerTarget::Layer(base), None)
        .unwrap();
    refused(
        d.set_pointer_target(p, PointerTarget::Layer(p)),
        Reason::PointerCycle,
    );
    // グループを挟む: 参照レイヤーが入ったグループを指す
    let group = d.group_layers(&[base, p], "グループ").unwrap();
    refused(
        d.set_pointer_target(p, PointerTarget::Layer(group)),
        Reason::PointerCycle,
    );
    // 参照の参照: q → p、p → q は輪
    let q = d
        .add_pointer_layer("参照の参照", PointerTarget::Layer(p), None)
        .unwrap();
    refused(
        d.set_pointer_target(p, PointerTarget::Layer(q)),
        Reason::PointerCycle,
    );
    // 指す先のグループへ参照レイヤーを動かすのも輪
    let r = d
        .add_pointer_layer("外", PointerTarget::Layer(group), None)
        .unwrap();
    refused(d.move_layer_to(r, Some(group), 0), Reason::PointerCycle);
    // 断った後も文書はそのまま（指す先・並び）
    assert_eq!(
        d.layer(p).unwrap().pointer().unwrap().target(),
        PointerTarget::Layer(base)
    );
    assert_eq!(d.layer(r).unwrap().parent(), None);
    assert!(pointer_issues(&d).is_empty());
}

#[test]
fn four_levels_of_pointers_pass_and_a_fifth_is_refused() {
    let mut d = doc();
    let base = painted(&mut d, "元", RED, (2, 2));
    let mut last = base;
    let mut chain = Vec::new();
    for k in 0..MAX_POINTER_DEPTH {
        last = d
            .add_pointer_layer(&format!("{k}"), PointerTarget::Layer(last), None)
            .unwrap();
        chain.push(last);
    }
    assert_eq!(d.pointer_depth(last).unwrap(), Some(MAX_POINTER_DEPTH));
    // 4 段の一番上の出力は、一番下の元と同じ
    assert_eq!(
        output(&d, last, Channel::Color),
        output(&d, base, Channel::Color)
    );
    refused(
        d.add_pointer_layer("5", PointerTarget::Layer(last), None),
        Reason::PointerTooDeep,
    );
    // 下に参照を挟んで深くするのも断る
    let deeper = d
        .add_pointer_layer("横", PointerTarget::Layer(base), None)
        .unwrap();
    refused(
        d.set_pointer_target(chain[0], PointerTarget::Layer(deeper)),
        Reason::PointerTooDeep,
    );
}

#[test]
fn deleting_the_target_shows_the_default_with_a_reason_and_undo_brings_it_back() {
    let mut d = doc();
    let target = painted(&mut d, "指す先", RED, (2, 2));
    let p = d
        .add_pointer_layer("参照", PointerTarget::Layer(target), None)
        .unwrap();
    let reading = output(&d, p, Channel::Color);
    let before = d.composite(all(&d)).unwrap();
    let serial = d.change_serial();
    d.remove_layer(target).unwrap();
    assert_eq!(output(&d, p, Channel::Color), filled(WHITE));
    assert_eq!(
        d.pointer_issue(p).unwrap(),
        Some(PointerIssue::MissingTarget)
    );
    assert_eq!(
        pointer_issues(&d),
        vec![(None, PointerIssue::MissingTarget)]
    );
    // 既定値はキャンバス全体を覆うので、変わったタイルは全部
    let changed = d.changed_tiles(Channel::Color, serial).unwrap();
    assert_eq!(changed.len(), ((W / TS) * (H / TS)) as usize);
    d.undo().unwrap();
    assert_eq!(output(&d, p, Channel::Color), reading);
    assert_eq!(d.composite(all(&d)).unwrap(), before);
    assert!(pointer_issues(&d).is_empty());
    // 消したレイヤーセットを指す参照レイヤーも同じ
    let shelf = d.add_layer_set("別").unwrap();
    let q = d
        .add_pointer_layer("セット", PointerTarget::LayerSet(shelf), None)
        .unwrap();
    d.remove_layer_set(shelf).unwrap();
    assert_eq!(
        d.pointer_issue(q).unwrap(),
        Some(PointerIssue::MissingTarget)
    );
}

#[test]
fn merging_bakes_the_pointer_into_pixels() {
    let mut d = doc();
    let shelf = d.add_layer_set("別").unwrap();
    let source = d.add_layer_in_set("元", shelf, None).unwrap();
    paint(&mut d, source, RED, (2, 2));
    painted(&mut d, "下", BLUE, (8, 8));
    let p = d
        .add_pointer_layer("参照", PointerTarget::Layer(source), None)
        .unwrap();
    d.set_layer_opacity(p, 0.7, false).unwrap();
    let before = set_composite(&d, first_set(&d), Channel::Color);
    let report = d.merge_down(p, Document::MERGE_ROUNDING_TOLERANCE).unwrap();
    let _ = report;
    assert!(d.layers().iter().all(|l| l.pointer().is_none()));
    assert_eq!(set_composite(&d, first_set(&d), Channel::Color), before);
    // 指す先が無い参照レイヤーは焼かない（既定値を焼くと指す先が落ちる）
    d.undo().unwrap();
    d.remove_layer(source).unwrap();
    assert!(matches!(
        d.merge_down(p, Document::MERGE_ROUNDING_TOLERANCE),
        Err(CoreError::InactiveEffect { .. })
    ));
}

#[test]
fn duplicates_keep_the_target_and_copies_inside_a_copied_group_point_at_the_copy() {
    let mut d = doc();
    let base = painted(&mut d, "元", RED, (2, 2));
    let p = d
        .add_pointer_layer("参照", PointerTarget::Layer(base), None)
        .unwrap();
    let copy = d.duplicate_layer(p, None).unwrap();
    assert_eq!(
        d.layer(copy).unwrap().pointer().unwrap().target(),
        PointerTarget::Layer(base)
    );
    let group = d.group_layers(&[base, p], "グループ").unwrap();
    let group_copy = d.duplicate_layer(group, None).unwrap();
    let inner: Vec<LayerId> = d
        .layers()
        .iter()
        .filter(|l| l.parent() == Some(group_copy))
        .map(|l| l.id())
        .collect();
    let copied_pointer = inner
        .iter()
        .find(|id| d.layer(**id).unwrap().pointer().is_some())
        .unwrap();
    let copied_base = inner.iter().find(|id| **id != *copied_pointer).unwrap();
    assert_eq!(
        d.layer(*copied_pointer)
            .unwrap()
            .pointer()
            .unwrap()
            .target(),
        PointerTarget::Layer(*copied_base)
    );
}

#[test]
fn painting_a_pointer_layer_is_refused() {
    let mut d = doc();
    let base = painted(&mut d, "元", RED, (2, 2));
    let p = d
        .add_pointer_layer("参照", PointerTarget::Layer(base), None)
        .unwrap();
    assert!(matches!(
        d.set_pixel(p, 1, 1, RED),
        Err(CoreError::Unsupported(Reason::CannotPaintPointerLayer))
    ));
    assert!(matches!(
        d.set_pointer_target(base, PointerTarget::Layer(p)),
        Err(CoreError::Unsupported(Reason::NotPointerLayer))
    ));
    // 文書に無い指す先は断る
    assert!(matches!(
        d.add_pointer_layer("無い", PointerTarget::Layer(LayerId(7)), None),
        Err(CoreError::LayerNotFound)
    ));
}

/// 4×4 の、画素ごとに違う入力（左下原点）。
fn small_input() -> PointerInput {
    let mut px = Vec::new();
    for y in 0..4u8 {
        for x in 0..4u8 {
            px.extend([x * 60, y * 60, 100, 255]);
        }
    }
    PointerInput::new(4, 4, Arc::from(px)).unwrap()
}

#[test]
fn another_texture_set_is_read_from_the_inputs_stretched_by_uv_and_missing_shows_the_default() {
    let mut d = doc();
    let p = d
        .add_pointer_layer("外", PointerTarget::External(KEY), None)
        .unwrap();
    assert_eq!(d.external_pointers(), vec![(p, KEY)]);
    // 渡されていない: 既定値と、チャンネルごとの理由
    assert_eq!(output(&d, p, Channel::Color), filled(WHITE));
    assert!(pointer_issues(&d)
        .iter()
        .any(|(c, i)| *c == Some(Channel::Color) && *i == PointerIssue::MissingInput));
    // 同じ大きさの入力はそのまま
    let mut same = Vec::new();
    for y in 0..H {
        for x in 0..W {
            same.extend([(x * 7) as u8, (y * 7) as u8, 9, 255]);
        }
    }
    let same = PointerInput::new(W, H, Arc::from(same.clone())).unwrap();
    d.set_effect_inputs(EffectInputs::new().with_pointer(KEY, Channel::Color, same.clone()))
        .unwrap();
    assert_eq!(output(&d, p, Channel::Color), same.pixels.to_vec());
    // 大きさが違う入力は UV で引き伸ばす（画素の中心で双線形。角は端の画素）
    d.set_effect_inputs(EffectInputs::new().with_pointer(KEY, Channel::Color, small_input()))
        .unwrap();
    let out = output(&d, p, Channel::Color);
    assert_eq!(&out[0..4], &[0, 0, 100, 255]);
    let last = ((W * H - 1) * 4) as usize;
    assert_eq!(&out[last..last + 4], &[180, 180, 100, 255]);
    // 文書の画素 (16, 0) の中心は入力の u = 16.5 / 32 * 4 - 0.5 = 1.5625: 60 と 120 の間
    let mid = (16 * 4) as usize;
    assert_eq!(out[mid], (60.0 + 0.5625 * 60.0f64).round() as u8);
    // 読む元のチャンネルの入力だけを読む（ほかのチャンネルは渡されていない）
    d.set_pointer_source(p, Channel::Roughness, Some(Channel::Color))
        .unwrap();
    assert_eq!(output(&d, p, Channel::Roughness), out);
    // 予算に収まらない入力は読まない
    d.set_pointer_input_budget_bytes(16);
    assert_eq!(output(&d, p, Channel::Color), filled(WHITE));
    assert!(pointer_issues(&d)
        .iter()
        .any(|(_, i)| *i == PointerIssue::InputOverBudget));
    // 予算を上げても、外した入力は戻らない（アプリが渡し直す）。それまでは入力が無い
    d.set_pointer_input_budget_bytes(1 << 20);
    assert_eq!(output(&d, p, Channel::Color), filled(WHITE));
    assert!(pointer_issues(&d)
        .iter()
        .any(|(_, i)| *i == PointerIssue::MissingInput));
    d.set_effect_inputs(EffectInputs::new().with_pointer(KEY, Channel::Color, small_input()))
        .unwrap();
    assert_eq!(output(&d, p, Channel::Color), out);
}

#[test]
fn inputs_that_no_pointer_reads_or_that_do_not_fit_are_dropped_and_never_push_out_a_read_input() {
    let other = ExternalSet {
        texture_set: 1,
        layer_set: 1,
    };
    let mut d = doc();
    let p = d
        .add_pointer_layer("外", PointerTarget::External(KEY), None)
        .unwrap();
    assert!(d.external_pointer_reads().contains(&(KEY, Channel::Color)));
    assert!(!d
        .external_pointer_reads()
        .contains(&(other, Channel::Color)));
    // 予算は入力 1 つ分と少し（64 バイト）。読まれない入力は鍵の順で先に来るが、予算を使わない
    d.set_pointer_input_budget_bytes(100);
    d.set_effect_inputs(
        EffectInputs::new()
            .with_pointer(other, Channel::Color, small_input())
            .with_pointer(KEY, Channel::Color, small_input()),
    )
    .unwrap();
    assert_eq!(
        pointer_issues(&d)
            .iter()
            .filter(|(c, i)| *c == Some(Channel::Color) && *i == PointerIssue::MissingInput)
            .count(),
        0
    );
    assert_eq!(
        &output(&d, p, Channel::Color)[0..4],
        &[0, 0, 100, 255],
        "読む入力は、読まない入力に押し出されない"
    );
    // 読まれない入力は持たない（読む参照レイヤーを足しても、渡し直すまで入力は無い）
    let q = d
        .add_pointer_layer("もう 1 つ", PointerTarget::External(other), None)
        .unwrap();
    assert_eq!(output(&d, q, Channel::Color), filled(WHITE));
    d.set_effect_inputs(
        EffectInputs::new()
            .with_pointer(other, Channel::Color, small_input())
            .with_pointer(KEY, Channel::Color, small_input()),
    )
    .unwrap();
    // 予算に収まるのは 1 つ（鍵の順で先の other）。もう 1 つは外れ、理由を言う
    assert_eq!(&output(&d, q, Channel::Color)[0..4], &[0, 0, 100, 255]);
    assert_eq!(output(&d, p, Channel::Color), filled(WHITE));
    assert_eq!(
        d.pointer_issue(p).unwrap(),
        None,
        "レイヤー全体の理由は無く、チャンネルごとに言う"
    );
    assert!(pointer_issues(&d)
        .iter()
        .any(|(c, i)| *c == Some(Channel::Color) && *i == PointerIssue::InputOverBudget));
}

#[test]
fn a_new_external_input_reevaluates_only_the_pointers_that_read_it() {
    let mut d = doc();
    let base = painted(&mut d, "元", RED, (2, 2));
    let inner = d
        .add_pointer_layer("中", PointerTarget::Layer(base), None)
        .unwrap();
    let p = d
        .add_pointer_layer("外", PointerTarget::External(KEY), None)
        .unwrap();
    d.set_effect_inputs(EffectInputs::new().with_pointer(KEY, Channel::Color, small_input()))
        .unwrap();
    let _ = d.composite(all(&d)).unwrap();
    let first = output(&d, inner, Channel::Color);
    let evaluated = d.effect_counters().blocks_evaluated;
    // 同じ入力を渡し直しても何も変わらない
    d.set_effect_inputs(EffectInputs::new().with_pointer(KEY, Channel::Color, small_input()))
        .unwrap();
    let _ = d.composite(all(&d)).unwrap();
    assert_eq!(d.effect_counters().blocks_evaluated, evaluated);
    // 別の画素: 外を読む参照レイヤーだけ評価し直す（ブロックの数 = キャンバスのブロック）
    let mut other = small_input().pixels.to_vec();
    other[0] = 1;
    d.set_effect_inputs(EffectInputs::new().with_pointer(
        KEY,
        Channel::Color,
        PointerInput::new(4, 4, Arc::from(other)).unwrap(),
    ))
    .unwrap();
    let _ = d.composite(all(&d)).unwrap();
    let after = d.effect_counters().blocks_evaluated;
    assert!(after > evaluated);
    let _ = output(&d, inner, Channel::Color);
    assert_eq!(d.effect_counters().blocks_evaluated, after);
    assert_eq!(output(&d, inner, Channel::Color), first);
    let _ = p;
}

#[test]
fn the_reach_of_a_layer_set_counts_levels_and_lists_external_pointers() {
    let mut d = doc();
    let shelf = d.add_layer_set("別").unwrap();
    let a = d.add_layer_in_set("a", shelf, None).unwrap();
    paint(&mut d, a, RED, (2, 2));
    let out = d
        .add_pointer_layer("外", PointerTarget::External(KEY), Some(a))
        .unwrap();
    let mid = d
        .add_pointer_layer("中", PointerTarget::Layer(out), Some(out))
        .unwrap();
    let reach = d.pointer_reach(shelf).unwrap();
    assert_eq!(reach.depth, 2);
    assert_eq!(reach.external.len(), 1);
    assert_eq!(reach.external[0].key, KEY);
    assert_eq!(reach.external[0].levels, 2);
    assert_eq!(reach.external[0].layer, out);
    // 最初のレイヤーセットから別のレイヤーセットを指すと、そこからの連なりは 1 段増える
    let top = d
        .add_pointer_layer("上", PointerTarget::LayerSet(shelf), None)
        .unwrap();
    assert_eq!(d.pointer_depth(top).unwrap(), Some(3));
    let reach = d.pointer_reach(first_set(&d)).unwrap();
    assert_eq!(reach.depth, 3);
    assert_eq!(reach.external[0].levels, 3);
    let _ = mid;
}

#[test]
fn a_stroke_on_the_target_updates_a_pointer_below_it_while_drawing() {
    let mut d = doc();
    let target = painted(&mut d, "指す先", RED, (2, 2));
    d.set_layer_opacity(target, 0.5, false).unwrap();
    let p = d
        .add_pointer_layer("参照", PointerTarget::Layer(target), None)
        .unwrap();
    // 参照レイヤーを、描くレイヤーの下へ（描く間の下の覚えに入る位置）
    d.move_layer(p, 0).unwrap();
    let brush = yolu_core::BrushSettings {
        color: GREEN,
        ..yolu_core::BrushSettings::default()
    };
    let mut stroke = d.begin_stroke(target, &brush).unwrap();
    stroke
        .add_point(&mut d, 20.0, 20.0, 1.0, yolu_core::glam::DVec2::ZERO)
        .unwrap();
    let _ = d.composite(all(&d)).unwrap();
    stroke
        .add_point(&mut d, 28.0, 26.0, 1.0, yolu_core::glam::DVec2::ZERO)
        .unwrap();
    let during = d.composite(all(&d)).unwrap();
    // 描いているレイヤーを下の参照レイヤーが読むので、下の覚えは使わない
    assert_eq!(d.composite_memo_stats().tiles, 0);
    d.set_composite_memo(false);
    assert_eq!(during, d.composite(all(&d)).unwrap());
    d.end_stroke(stroke).unwrap();
}

#[test]
fn a_pointer_below_that_does_not_read_the_drawn_layer_keeps_using_the_memo() {
    let mut d = doc();
    let other = painted(&mut d, "別", BLUE, (2, 2));
    let p = d
        .add_pointer_layer("参照", PointerTarget::Layer(other), None)
        .unwrap();
    let target = painted(&mut d, "描く", RED, (14, 14));
    assert!(
        d.layers().iter().position(|l| l.id() == p)
            < d.layers().iter().position(|l| l.id() == target)
    );
    let brush = yolu_core::BrushSettings {
        color: GREEN,
        ..yolu_core::BrushSettings::default()
    };
    let mut stroke = d.begin_stroke(target, &brush).unwrap();
    stroke
        .add_point(&mut d, 20.0, 20.0, 1.0, yolu_core::glam::DVec2::ZERO)
        .unwrap();
    let _ = d.composite(all(&d)).unwrap();
    stroke
        .add_point(&mut d, 28.0, 26.0, 1.0, yolu_core::glam::DVec2::ZERO)
        .unwrap();
    let during = d.composite(all(&d)).unwrap();
    assert!(
        d.composite_memo_stats().tiles > 0,
        "届かない参照レイヤーでは覚えを使う"
    );
    d.set_composite_memo(false);
    assert_eq!(during, d.composite(all(&d)).unwrap());
    d.end_stroke(stroke).unwrap();
}

#[test]
fn removing_a_channel_drops_its_source_and_undo_brings_it_back() {
    let mut d = doc();
    let base = painted(&mut d, "元", RED, (2, 2));
    let user = d
        .add_channel(yolu_core::ChannelInfo {
            name: "影".into(),
            kind: yolu_core::ChannelKind::Scalar,
            color_space: yolu_core::ColorSpace::Linear,
            default: Rgba8::new(0, 0, 0, 255),
        })
        .unwrap();
    let p = d
        .add_pointer_layer("参照", PointerTarget::Layer(base), None)
        .unwrap();
    d.set_pointer_source(p, user, Some(Channel::Color)).unwrap();
    let read = output(&d, p, user);
    d.remove_channel(user).unwrap();
    assert_eq!(d.layer(p).unwrap().pointer().unwrap().sources().count(), 0);
    d.undo().unwrap();
    assert_eq!(
        d.layer(p).unwrap().pointer().unwrap().source(user),
        Channel::Color
    );
    assert_eq!(output(&d, p, user), read);
}

/// 変わったタイルが全面か（既定値はキャンバス全体を覆う）。
fn whole_changed(d: &Document, c: Channel, serial: u64) -> bool {
    d.changed_tiles(c, serial).unwrap().len() == ((W / TS) * (H / TS)) as usize
}

#[test]
fn deleting_undoing_and_redoing_the_pointed_layer_set_marks_the_pointer_as_changed() {
    for filled in [false, true] {
        let mut d = doc();
        let shelf = d.add_layer_set("別").unwrap();
        if filled {
            let l = d.add_layer_in_set("中身", shelf, None).unwrap();
            paint(&mut d, l, RED, (2, 2));
        }
        let p = d
            .add_pointer_layer("セット", PointerTarget::LayerSet(shelf), None)
            .unwrap();
        // 解析を作っておく（読める状態）
        assert_eq!(d.pointer_issue(p).unwrap(), None);
        let reading = output(&d, p, Channel::Color);
        let serial = d.change_serial();
        d.remove_layer_set(shelf).unwrap();
        assert_eq!(
            d.pointer_issue(p).unwrap(),
            Some(PointerIssue::MissingTarget)
        );
        assert!(whole_changed(&d, Channel::Color, serial), "消す {filled}");
        assert_eq!(output(&d, p, Channel::Color), filled_default());
        let serial = d.change_serial();
        d.undo().unwrap();
        assert_eq!(d.pointer_issue(p).unwrap(), None);
        assert!(
            whole_changed(&d, Channel::Color, serial),
            "取り消す {filled}"
        );
        assert_eq!(output(&d, p, Channel::Color), reading);
        let serial = d.change_serial();
        d.redo().unwrap();
        assert!(
            whole_changed(&d, Channel::Color, serial),
            "やり直す {filled}"
        );
        assert_eq!(output(&d, p, Channel::Color), filled_default());
    }
}

fn filled_default() -> Vec<u8> {
    filled(WHITE)
}

#[test]
fn removing_and_restoring_the_source_channel_marks_the_pointer_channel_as_changed() {
    let mut d = doc();
    let base = painted(&mut d, "元", RED, (2, 2));
    let user = d
        .add_channel(yolu_core::ChannelInfo {
            name: "影".into(),
            kind: yolu_core::ChannelKind::Scalar,
            color_space: yolu_core::ColorSpace::Linear,
            default: Rgba8::new(0, 0, 0, 255),
        })
        .unwrap();
    // 元のレイヤーにユーザーチャンネルの画素を描き、Roughness を参照レイヤーが X から読む
    d.set_channel_pixel(base, user, 4, 4, Rgba8::new(200, 200, 200, 255))
        .unwrap();
    let p = d
        .add_pointer_layer("参照", PointerTarget::Layer(base), None)
        .unwrap();
    d.set_pointer_source(p, Channel::Roughness, Some(user))
        .unwrap();
    let reading = output(&d, p, Channel::Roughness);
    let serial = d.change_serial();
    d.remove_channel(user).unwrap();
    let removed = d.changed_tiles(Channel::Roughness, serial).unwrap();
    assert!(
        !removed.is_empty(),
        "消した: Roughness に変わったタイルが出る"
    );
    assert_ne!(output(&d, p, Channel::Roughness), reading);
    let serial = d.change_serial();
    d.undo().unwrap();
    let restored = d.changed_tiles(Channel::Roughness, serial).unwrap();
    assert!(
        !restored.is_empty(),
        "取り消した: Roughness に変わったタイルが出る"
    );
    assert_eq!(output(&d, p, Channel::Roughness), reading);
}

#[test]
fn a_hidden_target_is_still_read_but_hiding_a_child_of_a_pointed_group_changes_the_result() {
    let mut d = doc();
    let target = painted(&mut d, "指す先", RED, (2, 2));
    let p = d
        .add_pointer_layer("参照", PointerTarget::Layer(target), None)
        .unwrap();
    let reading = output(&d, p, Channel::Color);
    // 指す先自身を隠しても、参照レイヤーの出力は変わらない（隠した元を別の所で使える）
    d.set_layer_visible(target, false).unwrap();
    assert_eq!(output(&d, p, Channel::Color), reading);
    assert_eq!(d.pointer_issue(p).unwrap(), None);
    d.set_layer_visible(target, true).unwrap();
    // グループを指す場合も、グループ自身を隠しても同じ・子を隠すと変わる
    let child = painted(&mut d, "子", BLUE, (10, 10));
    let group = d.group_layers(&[child], "組").unwrap();
    let q = d
        .add_pointer_layer("グループ", PointerTarget::Layer(group), None)
        .unwrap();
    let group_reading = output(&d, q, Channel::Color);
    d.set_layer_visible(group, false).unwrap();
    assert_eq!(output(&d, q, Channel::Color), group_reading);
    d.set_layer_visible(group, true).unwrap();
    d.set_layer_visible(child, false).unwrap();
    assert_ne!(output(&d, q, Channel::Color), group_reading);
}
