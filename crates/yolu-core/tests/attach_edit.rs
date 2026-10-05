//! 効果の編集: どれも 1 回の Undo で戻り、断ったら何も変えない。予算・取消・外から渡す入力・複製。
mod attach_support;
use attach_support::*;
use std::sync::atomic::AtomicBool;
use yolu_core::fill_image::{Projection, ProjectionMode};
use yolu_core::generator::{self, anchor::ReadMode, Settings};
use yolu_core::{
    AnchorIssueKind, AnchorPlacement, Channel, CoreError, Document, EffectInputs, EffectSettings,
    FilterId, FilterSpec, FilterTarget, InactiveReason, LayerId, Rect, Rgba8, RowOrder,
};

fn snapshot(doc: &Document) -> (Vec<u8>, Vec<u8>) {
    (whole(doc, Channel::Color), whole(doc, Channel::Height))
}

/// 1 つの編集が 1 回の Undo で、Undo で前の合成に、Redo で後の合成に戻る。
fn one_undo_step(
    doc: &mut Document,
    changes_output: bool,
    what: &str,
    edit: impl FnOnce(&mut Document),
) {
    let count = doc.undo_count();
    let before = snapshot(doc);
    edit(doc);
    assert_eq!(doc.undo_count(), count + 1, "{what}: 1 回の Undo");
    let after = snapshot(doc);
    if changes_output {
        assert_ne!(before, after, "{what}: 合成が変わる");
    }
    assert!(doc.undo().unwrap(), "{what}");
    assert_eq!(snapshot(doc), before, "{what}: Undo で前の合成");
    assert!(doc.redo().unwrap(), "{what}");
    assert_eq!(snapshot(doc), after, "{what}: Redo で後の合成");
}

fn top_of(doc: &Document) -> LayerId {
    doc.layers()[2].id()
}

fn blur(doc: &mut Document, layer: LayerId, radius: u32, channels: &[Channel]) -> FilterId {
    doc.add_filter(
        layer,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(radius)).channels(channels),
    )
    .unwrap()
}

#[test]
fn every_filter_edit_is_one_undo_step() {
    let (mut doc, l) = world();
    let (base, top) = (l[0], l[2]);
    let mut id = None;
    one_undo_step(&mut doc, true, "段を足す", |d| {
        id = Some(blur(d, base, 4, &[Channel::Color]))
    });
    let f = id.unwrap();
    one_undo_step(&mut doc, true, "強さ", |d| {
        d.set_filter_strength(base, f, 0.4, false).unwrap()
    });
    one_undo_step(&mut doc, true, "設定", |d| {
        d.set_filter_settings(base, f, EffectSettings::blur(9), false)
            .unwrap()
    });
    one_undo_step(&mut doc, true, "無効", |d| {
        d.set_filter_enabled(base, f, false).unwrap()
    });
    one_undo_step(&mut doc, true, "有効に戻す", |d| {
        d.set_filter_enabled(base, f, true).unwrap_or(());
    });
    // 適用するチャンネルを替える・並べ替える・外す
    one_undo_step(&mut doc, true, "チャンネル", |d| {
        d.set_filter_channels(base, f, &[Channel::Color, Channel::Height])
            .unwrap()
    });
    let g = blur(&mut doc, base, 2, &[Channel::Height]);
    doc.add_filter(
        base,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::invert()).channels(&[Channel::Height]),
    )
    .unwrap();
    one_undo_step(&mut doc, true, "並べ替え", |d| {
        d.move_filter(base, g, 2).unwrap()
    });
    one_undo_step(&mut doc, true, "段を外す", |d| {
        d.remove_filter(base, f).unwrap()
    });
    // マスクのフィルター
    doc.add_layer_mask(top).unwrap();
    one_undo_step(&mut doc, true, "マスクの段", |d| {
        d.add_filter(
            top,
            FilterTarget::Mask,
            FilterSpec::new(EffectSettings::noise(0.5, 3, true)),
        )
        .unwrap();
    });
    // 画像・投影・グラデーション
    let fill = l[3];
    one_undo_step(&mut doc, true, "画像", |d| {
        d.set_fill_image(fill, Channel::Color, Some(image(0)))
            .unwrap()
    });
    let p = Projection {
        mode: ProjectionMode::Planar,
        tiles: [2.0, 2.0],
        ..Default::default()
    };
    one_undo_step(&mut doc, true, "投影", |d| {
        d.set_fill_projection(fill, p, false).unwrap()
    });
    let mut gradient = Settings::new(generator::Kind::ShapeGradient);
    gradient.ramp = Some(generator::Ramp::default());
    gradient.blend = generator::Blend::Replace;
    one_undo_step(&mut doc, true, "グラデーション", |d| {
        d.set_fill_gradient(fill, Channel::Height, Some(gradient.clone()), false)
            .unwrap()
    });
    // 値を消すと画像も外れ、1 回の Undo で一緒に戻る
    one_undo_step(&mut doc, true, "値を消す", |d| {
        d.set_fill_value(fill, Channel::Color, None, false).unwrap()
    });
    assert!(
        doc.layer(fill)
            .unwrap()
            .fill_image(Channel::Color)
            .is_none(),
        "値と一緒に画像も外れる"
    );
    doc.undo().unwrap();
    assert!(
        doc.layer(fill)
            .unwrap()
            .fill_image(Channel::Color)
            .is_some(),
        "Undo で画像も戻る"
    );
    assert!(doc
        .layer(fill)
        .unwrap()
        .fill_value(Channel::Color)
        .is_some());
    doc.redo().unwrap();
    // Anchor（合成は変えない）
    let mut anchor = None;
    one_undo_step(&mut doc, false, "Anchor を置く", |d| {
        anchor = Some(
            d.add_anchor(base, AnchorPlacement::Layer, Some("土台"), None)
                .unwrap(),
        )
    });
    one_undo_step(&mut doc, false, "Anchor の名前", |d| {
        d.rename_anchor(anchor.unwrap(), "別の名前").unwrap()
    });
    one_undo_step(&mut doc, false, "Anchor を外す", |d| {
        d.remove_anchor(anchor.unwrap()).unwrap()
    });
}

#[test]
fn slider_drags_coalesce_into_one_undo_step_and_can_be_cancelled() {
    let (mut doc, l) = world();
    let f = blur(&mut doc, l[0], 4, &[Channel::Color]);
    let start = doc.undo_count();
    let before = snapshot(&doc);
    for k in 1..=10 {
        doc.set_filter_strength(l[0], f, 1.0 - 0.05 * k as f64, true)
            .unwrap();
        doc.set_filter_settings(l[0], f, EffectSettings::blur(4 + k), true)
            .unwrap();
    }
    // 強さと設定は別の鍵（交互に変えると、そのたびに別の段になる）
    assert!(doc.undo_count() > start + 1);
    doc.end_coalescing();
    let (mut doc, l) = world();
    let f = blur(&mut doc, l[0], 4, &[Channel::Color]);
    let start = doc.undo_count();
    let before_drag = snapshot(&doc);
    for k in 1..=10 {
        doc.set_filter_strength(l[0], f, 1.0 - 0.05 * k as f64, true)
            .unwrap();
    }
    assert_eq!(doc.undo_count(), start + 1, "強さのドラッグは 1 回の Undo");
    doc.end_coalescing();
    doc.set_filter_strength(l[0], f, 0.3, true).unwrap();
    assert_eq!(
        doc.undo_count(),
        start + 2,
        "ドラッグを終えたあとは別の Undo"
    );
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert_eq!(snapshot(&doc), before_drag);
    // Escape で止める
    for k in 1..=5 {
        doc.set_filter_strength(l[0], f, 0.9 - 0.1 * k as f64, true)
            .unwrap();
    }
    assert!(doc.cancel_coalescing().unwrap());
    assert_eq!(snapshot(&doc), before_drag, "取り消したドラッグは戻る");
    let _ = before;
}

fn refused(
    doc: &mut Document,
    what: &str,
    op: impl FnOnce(&mut Document) -> Result<(), CoreError>,
) {
    let (count, snap) = (doc.undo_count(), snapshot(doc));
    let revision = doc.revision();
    assert!(op(doc).is_err(), "{what}: 断る");
    assert_eq!(doc.undo_count(), count, "{what}: 履歴を足さない");
    assert_eq!(doc.revision(), revision, "{what}: 何も変えない");
    assert_eq!(snapshot(doc), snap, "{what}: 合成を変えない");
}

#[test]
fn refusals_change_nothing() {
    let (mut doc, l) = world();
    let (base, top, fill) = (l[0], l[2], l[3]);
    let adj = doc
        .add_adjustment_layer("反転", yolu_core::AdjustmentSettings::invert(), None, None)
        .unwrap();
    let group = doc.group_layers(&[top], "グループ").unwrap();
    // 層の種類・チャンネル
    refused(&mut doc, "調整の層", |d| {
        d.add_filter(
            adj,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(3)).channels(&[Channel::Color]),
        )
        .map(|_| ())
    });
    refused(&mut doc, "グループ", |d| {
        d.add_filter(
            group,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(3)),
        )
        .map(|_| ())
    });
    refused(&mut doc, "色ノイズはスカラーに", |d| {
        d.add_filter(
            base,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::noise(0.3, 1, false)).channels(&[Channel::Height]),
        )
        .map(|_| ())
    });
    refused(&mut doc, "法線にぼかし以外", |d| {
        d.add_filter(
            base,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::invert()).channels(&[Channel::Normal]),
        )
        .map(|_| ())
    });
    refused(&mut doc, "ジェネレーターを法線に", |d| {
        d.add_filter(
            base,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(Settings::new(
                generator::Kind::PositionGradient,
            )))
            .channels(&[Channel::Normal]),
        )
        .map(|_| ())
    });
    refused(&mut doc, "マスクが無い", |d| {
        d.add_filter(
            base,
            FilterTarget::Mask,
            FilterSpec::new(EffectSettings::blur(3)),
        )
        .map(|_| ())
    });
    doc.add_layer_mask(base).unwrap();
    refused(&mut doc, "マスクにチャンネル", |d| {
        d.add_filter(
            base,
            FilterTarget::Mask,
            FilterSpec::new(EffectSettings::blur(3)).channels(&[Channel::Color]),
        )
        .map(|_| ())
    });
    refused(&mut doc, "マスクに色ノイズ", |d| {
        d.add_filter(
            base,
            FilterTarget::Mask,
            FilterSpec::new(EffectSettings::noise(0.3, 1, false)),
        )
        .map(|_| ())
    });
    refused(&mut doc, "チャンネルが空", |d| {
        d.add_filter(
            base,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(3)).channels(&[]),
        )
        .map(|_| ())
    });
    refused(&mut doc, "チャンネルが重なる", |d| {
        d.add_filter(
            base,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(3)).channels(&[Channel::Color, Channel::Color]),
        )
        .map(|_| ())
    });
    // ユーザーチャンネル（効果は標準のチャンネルだけ）
    let user = doc
        .add_channel(yolu_core::ChannelInfo {
            name: "ユーザー".into(),
            kind: yolu_core::ChannelKind::Scalar,
            color_space: yolu_core::ColorSpace::Linear,
            default: Rgba8::new(0, 0, 0, 255),
        })
        .unwrap();
    refused(
        &mut doc,
        "ユーザーチャンネルのフィルター",
        |d| {
            d.add_filter(
                base,
                FilterTarget::Content,
                FilterSpec::new(EffectSettings::blur(3)).channels(&[user]),
            )
            .map(|_| ())
        },
    );
    refused(&mut doc, "ユーザーチャンネルの画像", |d| {
        d.set_fill_image(fill, user, Some(image(0)))
    });
    // 設定の範囲・値
    refused(&mut doc, "半径 0", |d| {
        d.add_filter(
            base,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(0)).channels(&[Channel::Color]),
        )
        .map(|_| ())
    });
    refused(&mut doc, "半径 257", |d| {
        d.add_filter(
            base,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(257)).channels(&[Channel::Color]),
        )
        .map(|_| ())
    });
    refused(&mut doc, "強さ 2", |d| {
        d.add_filter(
            base,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(3))
                .channels(&[Channel::Color])
                .strength(2.0),
        )
        .map(|_| ())
    });
    refused(&mut doc, "強さ NaN", |d| {
        d.add_filter(
            base,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(3))
                .channels(&[Channel::Color])
                .strength(f64::NAN),
        )
        .map(|_| ())
    });
    refused(&mut doc, "レベルの範囲", |d| {
        d.add_filter(
            base,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::levels(0.5, 0.5, 1.0, 0.0, 1.0))
                .channels(&[Channel::Color]),
        )
        .map(|_| ())
    });
    let mut bad = Settings::new(generator::Kind::EdgeWear);
    bad.low = 0.9;
    bad.high = 0.9;
    refused(&mut doc, "ジェネレーターのレベル", |d| {
        d.add_filter(
            base,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(bad)).channels(&[Channel::Color]),
        )
        .map(|_| ())
    });
    refused(&mut doc, "ID が空", |d| {
        d.add_filter(
            base,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(3))
                .channels(&[Channel::Color])
                .with_id(FilterId(0)),
        )
        .map(|_| ())
    });
    refused(&mut doc, "index", |d| {
        d.add_filter(
            base,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(3))
                .channels(&[Channel::Color])
                .at(5),
        )
        .map(|_| ())
    });
    // ID の重なり
    let f = blur(&mut doc, base, 3, &[Channel::Color]);
    refused(&mut doc, "ID の重なり", |d| {
        d.add_filter(
            top,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(3))
                .channels(&[Channel::Color])
                .with_id(f),
        )
        .map(|_| ())
    });
    refused(&mut doc, "その層に無い段", |d| {
        d.remove_filter(top, f)
    });
    let top_id = top_of(&doc);
    let mixed = blur(&mut doc, top_id, 3, &[Channel::Color, Channel::Height]);
    refused(
        &mut doc,
        "段の設定を色ノイズに（スカラーのチャンネルがある）",
        |d| {
            d.set_filter_settings(
                top_of(d),
                mixed,
                EffectSettings::noise(0.2, 1, false),
                false,
            )
        },
    );
    // 到達半径・段の数（256 + 250 = 506 は通り、あと 10 で 512 を超える）
    let wide = doc.add_layer("到達半径").unwrap();
    blur(&mut doc, wide, 256, &[Channel::Color]);
    blur(&mut doc, wide, 250, &[Channel::Color]);
    refused(&mut doc, "到達半径が 512 を超える", |d| {
        d.add_filter(
            wide,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(10)).channels(&[Channel::Color]),
        )
        .map(|_| ())
    });
    let top_layer = doc.add_layer("段の数").unwrap();
    for _ in 0..32 {
        doc.add_filter(
            top_layer,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::invert()).channels(&[Channel::Color]),
        )
        .unwrap();
    }
    refused(&mut doc, "33 段目", |d| {
        d.add_filter(
            top_layer,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::invert()).channels(&[Channel::Color]),
        )
        .map(|_| ())
    });
}

#[test]
fn working_memory_budget_refuses_a_stack_before_anything_changes() {
    let (mut doc, l) = world();
    let f = blur(&mut doc, l[0], 3, &[Channel::Color]);
    let need = {
        let (mut d2, l2) = world();
        let _ = blur(&mut d2, l2[0], 3, &[Channel::Color]);
        d2.set_filter_working_budget_bytes(u64::MAX).unwrap();
        // 今のスタックが要る量より小さい予算は置けない
        let mut small = 1u64;
        while d2.set_filter_working_budget_bytes(small).is_err() {
            small *= 2;
        }
        small
    };
    assert!(need > 1);
    doc.set_filter_working_budget_bytes(need).unwrap();
    assert!(
        doc.set_filter_working_budget_bytes(need / 4).is_err(),
        "今のスタックが要る量より小さくできない"
    );
    let count = doc.undo_count();
    // もっと要る段は断る（ぼかしの半径を大きくするとブロックの作業が増える）
    let result = doc.set_filter_settings(l[0], f, EffectSettings::blur(200), false);
    assert_eq!(result, Err(CoreError::WorkingBudgetExceeded));
    assert_eq!(doc.undo_count(), count);
    assert_eq!(doc.filter_block_pixels(), 16);
    assert!(
        doc.set_filter_block_pixels(4096).is_err(),
        "予算に収まらないブロック"
    );
    assert_eq!(doc.filter_block_pixels(), 16);
}

/// 合成したあとにブロックの大きさを替えても、前の大きさのブロックを返さない（結果はブロックの大きさによらない）。
#[test]
fn changing_the_block_size_after_compositing_gives_the_same_pixels() {
    let (mut doc, l) = world();
    blur(&mut doc, l[0], 6, &[Channel::Color, Channel::Height]);
    blur(&mut doc, l[2], 3, &[Channel::Color]);
    doc.add_filter(
        l[2],
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::normalize()).channels(&[Channel::Height]),
    )
    .unwrap();
    let reference = snapshot(&doc);
    // 評価のブロックが小さいほどブロックの数が多い。大きくする・小さくする・元に戻すを、合成を挟んで繰り返す
    for block in [64, 8, 16, 4096 / 16, 24, 16, 40, 8] {
        doc.set_filter_block_pixels(block).unwrap();
        let after = snapshot(&doc);
        assert!(
            after == reference,
            "ブロック {block}: 合成した後に大きさを替えると結果が変わった"
        );
        // 新しい文書（大きさを決めてから最初に合成する）とも同じ
        let (mut fresh, f) = world();
        blur(&mut fresh, f[0], 6, &[Channel::Color, Channel::Height]);
        blur(&mut fresh, f[2], 3, &[Channel::Color]);
        fresh
            .add_filter(
                f[2],
                FilterTarget::Content,
                FilterSpec::new(EffectSettings::normalize()).channels(&[Channel::Height]),
            )
            .unwrap();
        fresh.set_filter_block_pixels(block).unwrap();
        assert!(
            snapshot(&fresh) == reference,
            "ブロック {block}: 新しい文書"
        );
    }
}

#[test]
fn cancelling_stops_evaluation_without_publishing_pixels() {
    let (mut doc, l) = world();
    blur(&mut doc, l[0], 6, &[Channel::Color]);
    let cancel = AtomicBool::new(true);
    let mut out = vec![7u8; (W * H * 4) as usize];
    let err = doc
        .composite_into_cancellable(
            Channel::Color,
            Rect::new(0, 0, W, H),
            &mut out,
            RowOrder::BottomUp,
            Some(&cancel),
        )
        .unwrap_err();
    assert_eq!(err, CoreError::Cancelled);
    assert!(out.iter().all(|b| *b == 7), "出力は書き換えない");
    // 立てなければ評価できて、取り消しは次の評価に残らない
    let expected = whole(&doc, Channel::Color);
    let ok = AtomicBool::new(false);
    let mut out = vec![0u8; (W * H * 4) as usize];
    doc.composite_into_cancellable(
        Channel::Color,
        Rect::new(0, 0, W, H),
        &mut out,
        RowOrder::BottomUp,
        Some(&ok),
    )
    .unwrap();
    assert_eq!(out, expected);
}

#[test]
fn a_zero_cache_budget_gives_the_same_pixels() {
    let (mut doc, l) = world();
    blur(&mut doc, l[0], 6, &[Channel::Color, Channel::Height]);
    let cached = snapshot(&doc);
    assert!(doc.effect_counters().cache_bytes > 0);
    doc.set_filter_cache_budget_bytes(0);
    assert_eq!(doc.effect_counters().cache_bytes, 0);
    let before = doc.effect_counters().blocks_evaluated;
    assert_eq!(snapshot(&doc), cached);
    assert!(
        doc.effect_counters().blocks_evaluated > before,
        "持たないので毎回評価する"
    );
    assert_eq!(doc.effect_counters().cache_bytes, 0);
}

#[test]
fn only_the_blocks_that_changed_are_evaluated_again() {
    let (mut doc, l) = world();
    let base = l[0];
    blur(&mut doc, base, 2, &[Channel::Color]);
    let _ = whole(&doc, Channel::Color);
    let after_first = doc.effect_counters().blocks_evaluated;
    let _ = whole(&doc, Channel::Color);
    assert_eq!(
        doc.effect_counters().blocks_evaluated,
        after_first,
        "同じものは作り直さない"
    );
    // 1 タイルだけ描く: そのタイルの近く（半径の分）のブロックだけ
    let mut rng = Rng(5);
    let brush = yolu_core::BrushSettings {
        radius: 2.0,
        ..Default::default()
    };
    let mut s = doc.begin_stroke_in(base, Channel::Color, &brush).unwrap();
    s.add_point(&mut doc, 2.0, 2.0, 1.0, yolu_core::glam::DVec2::ZERO)
        .unwrap();
    doc.end_stroke(s).unwrap();
    let _ = rng.next();
    let _ = whole(&doc, Channel::Color);
    let evaluated = doc.effect_counters().blocks_evaluated - after_first;
    assert!(
        (1..=2).contains(&evaluated),
        "変わった所のブロックだけ: {evaluated}"
    );
}

#[test]
fn generators_say_why_they_pass_their_input_through() {
    let (mut doc, l) = world();
    let top = l[2];
    let mut g = Settings::new(generator::Kind::Thickness); // 焼いていない
    g.blend = generator::Blend::Replace;
    let missing = doc
        .add_filter(
            top,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(g)).channels(&[Channel::Color]),
        )
        .unwrap();
    let mut pinned = Settings::new(generator::Kind::EdgeWear);
    pinned
        .pins
        .insert(generator::MapKind::Curvature, "b".repeat(64));
    let pin = doc
        .add_filter(
            top,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(pinned)).channels(&[Channel::Color]),
        )
        .unwrap();
    let ok = doc
        .add_filter(
            top,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(Settings::new(
                generator::Kind::PositionGradient,
            )))
            .channels(&[Channel::Color]),
        )
        .unwrap();
    use generator::Inactive;
    assert_eq!(
        doc.generator_inactive(top, missing).unwrap(),
        Some(InactiveReason::Generator(Inactive::MissingMap(
            generator::MapKind::Thickness
        )))
    );
    assert_eq!(
        doc.generator_inactive(top, pin).unwrap(),
        Some(InactiveReason::Generator(Inactive::PinMismatch(
            generator::MapKind::Curvature
        )))
    );
    assert_eq!(doc.generator_inactive(top, ok).unwrap(), None);
    let notes = doc.inactive_effects();
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(notes.iter().all(|n| n.contains("上")), "{notes:?}");
    // 入力を外すと、マップを読むものは全部通す。渡し直すと使える
    let with = whole(&doc, Channel::Color);
    doc.set_effect_inputs(EffectInputs::new()).unwrap();
    assert_ne!(
        whole(&doc, Channel::Color),
        with,
        "入力を外すと合成が変わる"
    );
    assert_eq!(
        doc.generator_inactive(top, ok).unwrap(),
        Some(InactiveReason::Generator(Inactive::MissingMap(
            generator::MapKind::Position
        )))
    );
    doc.set_effect_inputs(inputs(0)).unwrap();
    assert_eq!(whole(&doc, Channel::Color), with);
    // 古い・照合できないマップは使わない（黒として読まない）
    use generator::MapState;
    let mut stale = EffectInputs::new();
    let m = inputs(0).map(generator::MapKind::Position).unwrap().clone();
    stale = stale
        .with_map(
            yolu_core::MapInput::new(
                m.kind,
                m.width,
                m.height,
                m.data.to_vec(),
                m.coverage.to_vec(),
                m.bounds_min,
                m.bounds_max,
                &m.condition_key,
                MapState::Stale,
            )
            .unwrap(),
        )
        .unwrap();
    doc.set_effect_inputs(stale).unwrap();
    assert_eq!(
        doc.generator_inactive(top, ok).unwrap(),
        Some(InactiveReason::Generator(Inactive::StaleMap(
            generator::MapKind::Position
        )))
    );
    // モデルのルートが分からないと、形のグラデーションは通す
    let mut shape = Settings::new(generator::Kind::ShapeGradient);
    shape.blend = generator::Blend::Replace;
    let s = doc
        .add_filter(
            top,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(shape)).channels(&[Channel::Height]),
        )
        .unwrap();
    doc.set_effect_inputs(inputs(0).with_frame(None)).unwrap();
    assert_eq!(
        doc.generator_inactive(top, s).unwrap(),
        Some(InactiveReason::Generator(Inactive::MissingFrame))
    );
}

#[test]
fn setting_the_same_inputs_again_does_not_evaluate_again() {
    let (mut doc, l) = world();
    doc.add_filter(
        l[2],
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::generator(Settings::new(
            generator::Kind::PositionGradient,
        )))
        .channels(&[Channel::Color]),
    )
    .unwrap();
    let _ = whole(&doc, Channel::Color);
    let blocks = doc.effect_counters().blocks_evaluated;
    let serial = doc.change_serial();
    doc.set_effect_inputs(inputs(0)).unwrap();
    let _ = whole(&doc, Channel::Color);
    assert_eq!(doc.effect_counters().blocks_evaluated, blocks);
    assert_eq!(doc.change_serial(), serial, "同じ入力では何も変わらない");
    doc.set_effect_inputs(inputs(7)).unwrap();
    assert!(
        doc.change_serial() > serial,
        "違う入力を読む層が変わったことになる"
    );
    let _ = whole(&doc, Channel::Color);
    assert!(doc.effect_counters().blocks_evaluated > blocks);
}

#[test]
fn anchors_must_be_below_and_say_what_is_wrong() {
    let (mut doc, l) = world();
    let (base, mid, top) = (l[0], l[1], l[2]);
    let a_base = doc
        .add_anchor(base, AnchorPlacement::Layer, None, None)
        .unwrap();
    let a_top = doc
        .add_anchor(top, AnchorPlacement::Layer, None, None)
        .unwrap();
    assert_eq!(doc.find_anchor(a_base).unwrap().anchor.name(), "土台");
    refused(&mut doc, "同じ層に 2 つ目", |d| {
        d.add_anchor(base, AnchorPlacement::Layer, None, None)
            .map(|_| ())
    });
    refused(&mut doc, "マスクが無い", |d| {
        d.add_anchor(base, AnchorPlacement::Mask, None, None)
            .map(|_| ())
    });
    refused(&mut doc, "名前が空", |d| {
        d.add_anchor(mid, AnchorPlacement::Layer, Some("  "), None)
            .map(|_| ())
    });
    refused(&mut doc, "名前が長い", |d| {
        d.add_anchor(mid, AnchorPlacement::Layer, Some(&"あ".repeat(129)), None)
            .map(|_| ())
    });
    refused(&mut doc, "ID の重なり", |d| {
        d.add_anchor(mid, AnchorPlacement::Layer, None, Some(a_base))
            .map(|_| ())
    });
    let mut g = Settings::new(generator::Kind::Anchor);
    g.blend = generator::Blend::Replace;
    let reader = doc
        .add_filter(
            mid,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(g)).channels(&[Channel::Height]),
        )
        .unwrap();
    assert_eq!(doc.anchor_issues().len(), 1);
    assert_eq!(doc.anchor_issues()[0].kind, AnchorIssueKind::NotChosen);
    refused(&mut doc, "上の層の Anchor", |d| {
        d.set_generator_anchor(
            mid,
            reader,
            Some(a_top),
            Channel::Height,
            ReadMode::Value,
            false,
        )
    });
    refused(&mut doc, "Normal を読む", |d| {
        d.set_generator_anchor(
            mid,
            reader,
            Some(a_base),
            Channel::Normal,
            ReadMode::Value,
            false,
        )
    });
    refused(&mut doc, "消えた Anchor", |d| {
        d.set_generator_anchor(
            mid,
            reader,
            Some(yolu_core::AnchorId(99)),
            Channel::Height,
            ReadMode::Value,
            false,
        )
    });
    let plain = doc
        .add_filter(
            mid,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(2)).channels(&[Channel::Color]),
        )
        .unwrap();
    refused(&mut doc, "Anchor のジェネレーターではない", |d| {
        d.set_generator_anchor(
            mid,
            plain,
            Some(a_base),
            Channel::Height,
            ReadMode::Value,
            false,
        )
    });
    assert_eq!(doc.anchors_readable_from(mid).unwrap().len(), 1);
    assert_eq!(doc.anchors_readable_from(base).unwrap().len(), 0);
    doc.set_generator_anchor(
        mid,
        reader,
        Some(a_base),
        Channel::Height,
        ReadMode::Value,
        false,
    )
    .unwrap();
    assert!(doc.anchor_issues().is_empty());
    // Anchor を外すと、読む段は入力のまま通して理由を出す。Undo で戻る
    doc.remove_anchor(a_base).unwrap();
    assert_eq!(doc.anchor_issues()[0].kind, AnchorIssueKind::Missing);
    doc.undo().unwrap();
    assert!(doc.anchor_issues().is_empty());
    // 層を動かして Anchor が読む層より上になると、断らずに通す（Undo で戻る）
    doc.move_layer(base, 3).unwrap();
    assert_eq!(doc.anchor_issues()[0].kind, AnchorIssueKind::NotBelow);
    doc.undo().unwrap();
    assert!(doc.anchor_issues().is_empty());
}

#[test]
fn duplicating_a_layer_gives_new_ids_and_points_copied_readers_at_the_copy() {
    let (mut doc, l) = world();
    let (base, mid) = (l[0], l[1]);
    let a = doc
        .add_anchor(base, AnchorPlacement::Layer, None, None)
        .unwrap();
    let f = blur(&mut doc, base, 3, &[Channel::Color]);
    let group = doc.group_layers(&[base, mid], "グループ").unwrap();
    let mut g = Settings::new(generator::Kind::Anchor);
    g.blend = generator::Blend::Replace;
    let reader = doc
        .add_filter(
            mid,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(g)).channels(&[Channel::Height]),
        )
        .unwrap();
    doc.set_generator_anchor(
        mid,
        reader,
        Some(a),
        Channel::Height,
        ReadMode::Value,
        false,
    )
    .unwrap();
    let copy = doc.duplicate_layer(group, None).unwrap();
    // グループの中の層の写し
    let copies: Vec<_> = doc
        .layers()
        .iter()
        .filter(|x| x.parent() == Some(copy))
        .map(|x| x.id())
        .collect();
    assert_eq!(copies.len(), 2);
    let ids: Vec<FilterId> = doc
        .layers()
        .iter()
        .flat_map(|x| x.filters().iter().map(|e| e.id()))
        .collect();
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(ids.len(), unique.len(), "段の ID は文書の中で重ならない");
    assert!(ids.len() >= 3 && ids.contains(&f));
    let anchors = doc.anchors();
    assert_eq!(anchors.len(), 2, "写しも自分の Anchor を持つ");
    let copied_anchor = anchors
        .iter()
        .find(|x| x.anchor.id() != a)
        .unwrap()
        .anchor
        .id();
    // 写しの中の読む段は、写しの Anchor を読む（元の段は元のまま）
    let reads = |layer: LayerId| -> u128 {
        doc.layer(layer)
            .unwrap()
            .filters()
            .iter()
            .find_map(|e| e.settings().generator_settings().map(|g| g.anchor.id))
            .unwrap()
    };
    assert_eq!(reads(mid), a.0);
    let copied_mid = *copies
        .iter()
        .find(|c| doc.layer(**c).unwrap().name() == "中")
        .unwrap();
    assert_eq!(reads(copied_mid), copied_anchor.0);
    assert!(doc.anchor_issues().is_empty());
    doc.undo().unwrap();
    assert_eq!(doc.anchors().len(), 1);
    doc.redo().unwrap();
    assert_eq!(doc.anchors().len(), 2);
}

#[test]
fn direct_writes_invalidate_what_was_evaluated() {
    let (mut doc, l) = world();
    blur(&mut doc, l[0], 3, &[Channel::Color]);
    let before = whole(&doc, Channel::Color);
    doc.set_pixel(l[0], 10, 10, Rgba8::new(1, 2, 3, 255))
        .unwrap();
    assert_ne!(whole(&doc, Channel::Color), before);
    let fresh = {
        doc.release_effect_cache();
        whole(&doc, Channel::Color)
    };
    doc.set_pixel(l[0], 11, 10, Rgba8::new(9, 8, 7, 255))
        .unwrap();
    assert_ne!(whole(&doc, Channel::Color), fresh);
}

/// 全域の統計と画像のミップマップは、ブロックを並べて評価する前に 1 回ずつ作る（ブロックごとに同時に作って、作業メモリが予算の
/// 並列数倍に届かない）。ブロックの数が多いほど、昔は重複して作った。
#[test]
fn shared_statistics_and_mip_chains_are_built_once_however_many_blocks_run() {
    let (mut doc, l) = world();
    let fill = l[3];
    // 小さいブロックでブロックの数を増やす（40×28、1 タイルのブロックなら 5×4 = 20）
    doc.set_filter_block_pixels(8).unwrap();
    doc.add_filter(
        l[2],
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::normalize()).channels(&[Channel::Color]),
    )
    .unwrap();
    doc.add_filter(
        l[2],
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::normalize()).channels(&[Channel::Height]),
    )
    .unwrap();
    doc.set_fill_image(fill, Channel::Color, Some(image(0)))
        .unwrap();
    doc.set_fill_image(fill, Channel::Height, Some(image(1)))
        .unwrap();
    for round in 0..3 {
        doc.release_effect_cache();
        let before = doc.effect_counters();
        let color = whole(&doc, Channel::Color);
        let height = whole(&doc, Channel::Height);
        let after = doc.effect_counters();
        assert_eq!(
            after.statistics_computed - before.statistics_computed,
            2,
            "round {round}: 正規化の統計は、チャンネルごと（上の層の Color と Height）に 1 回"
        );
        assert_eq!(
            after.mip_chains_built - before.mip_chains_built,
            2,
            "round {round}: 画像のミップマップは、画像（と変換）ごとに 1 つ"
        );
        assert!(
            after.blocks_evaluated - before.blocks_evaluated > 20,
            "ブロックは何度も評価した（並べて評価した）"
        );
        // 結果は変わらない
        doc.release_effect_cache();
        assert!(whole(&doc, Channel::Color) == color && whole(&doc, Channel::Height) == height);
    }
}

#[test]
fn image_budget_refuses_an_image_and_lowering_it_shows_the_value_with_a_reason() {
    let (mut doc, l) = world();
    let fill = l[3];
    doc.set_fill_image(fill, Channel::Color, Some(image(0)))
        .unwrap();
    assert!(
        doc.inactive_effects().is_empty(),
        "{:?}",
        doc.inactive_effects()
    );
    let with = whole(&doc, Channel::Color);
    doc.set_fill_image_cache_budget_bytes(0);
    let notes = doc.inactive_effects();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_ne!(
        whole(&doc, Channel::Color),
        with,
        "画像が使えないので値を見せる"
    );
    refused(&mut doc, "予算を超える画像", |d| {
        d.set_fill_image(fill, Channel::Roughness, Some(image(1)))
    });
    refused(&mut doc, "プロジェクトに無い画像", |d| {
        d.set_fill_image(fill, Channel::Roughness, Some(yolu_core::ImageId(77)))
    });
    doc.set_fill_image_cache_budget_bytes(256 * 1024 * 1024);
    assert_eq!(whole(&doc, Channel::Color), with);
    refused(&mut doc, "ラスターに画像", |d| {
        d.set_fill_image(l[0], Channel::Color, Some(image(0)))
    });
    let p = Projection {
        tiles: [0.0, 1.0],
        ..Default::default()
    };
    refused(&mut doc, "投影の範囲", |d| {
        d.set_fill_projection(fill, p, false)
    });
    let mut gradient = Settings::new(generator::Kind::ShapeGradient);
    gradient.blend = generator::Blend::Replace;
    refused(&mut doc, "ランプの無いグラデーション", |d| {
        d.set_fill_gradient(fill, Channel::Height, Some(gradient.clone()), false)
    });
    gradient.ramp = Some(generator::Ramp::default());
    refused(&mut doc, "法線のグラデーション", |d| {
        d.set_fill_gradient(fill, Channel::Normal, Some(gradient.clone()), false)
    });
    gradient.blend = generator::Blend::Multiply;
    refused(
        &mut doc,
        "置き換えでないグラデーション",
        |d| d.set_fill_gradient(fill, Channel::Height, Some(gradient.clone()), false),
    );
}

/// 単体のフィルターの評価と、層に付けたフィルターの合成が全バイト一致する。
mod standalone {
    use yolu_core::filter::{self, Image, Options, Settings, Stage, ValueType};
    use yolu_core::{Channel, Document, EffectSettings, FilterSpec, FilterTarget, Rect, Rgba8};

    fn painted(doc: &mut Document, id: yolu_core::LayerId) {
        let (w, h) = (doc.width(), doc.height());
        for y in 0..h {
            for x in 0..w {
                if (x * 7 + y * 3) % 5 == 0 {
                    continue;
                }
                let c = Rgba8::new(
                    (x * 13 + y * 5) as u8,
                    (x * 3 + y * 17) as u8,
                    (x + y * 31) as u8,
                    ((x * 29 + y * 11) % 256) as u8,
                );
                doc.set_pixel(id, x, y, c).unwrap();
            }
        }
    }

    fn canvas(doc: &Document, id: yolu_core::LayerId) -> Vec<u8> {
        doc.layer(id)
            .unwrap()
            .surface(Channel::Color)
            .unwrap()
            .to_canvas_bytes()
    }

    #[test]
    fn blur_on_a_layer_matches_the_standalone_filter() {
        let mut doc = Document::with_tile_size(37, 29, 8).unwrap();
        let id = doc.add_layer("a").unwrap();
        painted(&mut doc, id);
        doc.add_filter(
            id,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(5)).channels(&[Channel::Color]),
        )
        .unwrap();
        let got = doc.composite(doc.bounds()).unwrap();
        let src = canvas(&doc, id);
        let image = Image::new(&src, 37, 29).unwrap();
        let stack = [Stage::new(Settings::GaussianBlur { radius: 5 })];
        let mut expected = filter::evaluate(
            &image,
            ValueType::Color,
            &stack,
            Rect::new(0, 0, 37, 29),
            &Options::default(),
        )
        .unwrap();
        for p in expected.as_chunks_mut::<4>().0 {
            if p[3] == 0 {
                p.fill(0);
            }
        }
        assert_eq!(got, expected);
        // Undo で元の画素
        doc.undo().unwrap();
        let plain = doc.composite(doc.bounds()).unwrap();
        assert_ne!(plain, expected);
        doc.redo().unwrap();
        assert_eq!(doc.composite(doc.bounds()).unwrap(), expected);
    }
}
