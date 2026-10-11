//! 効果のレイヤー（フィルター・Generator・Anchor）の画面の操作（`Action::Fx`）と、効果の入力のつなぎ（焼いたメッシュマップ・モデル・画像）。
//! 画面なしで `AppState` を叩く: 各操作の結果と Undo 1 回、ロックでの断り、断った理由の日英、入力がそろうまでの理由と焼いた後に効くこと、
//! 焼き直しで読むレイヤーだけが描き直されること、保存復元（Unity 版が書いた効果入りの正本を開いて編集して保存）。

use std::time::Duration;
use yolu_app::bake::{BakeAction, BakeBackend};
use yolu_app::fx::{FilterKind, FxOp, Selected};
use yolu_app::lang::Lang;
use yolu_app::m2::{Edit, UiOp};
use yolu_app::state::{Action, AppState};
use yolu_core::generator::{self, Kind};
use yolu_core::mesh_maps::MeshMapKind;
use yolu_core::{AnchorPlacement, Channel, EffectSettings, FilterTarget, LayerId, LayerLocks};

fn fx(app: &mut AppState, op: FxOp) {
    app.apply(Action::Fx(op));
}

fn add_filter(app: &mut AppState, target: FilterTarget, kind: FilterKind) {
    fx(app, FxOp::AddFilter { target, kind });
}

fn add_generator(app: &mut AppState, target: FilterTarget, kind: Kind) {
    fx(app, FxOp::AddGenerator { target, kind });
}

fn filters(app: &AppState, layer: LayerId, target: FilterTarget) -> usize {
    app.doc.filters_of(layer, target).unwrap().len()
}

/// 全チャンネルの合成（効果込み）。
fn composite(app: &AppState) -> Vec<u8> {
    app.doc.composite(app.doc.bounds()).unwrap()
}

// ───────── 操作と Undo ─────────

#[test]
fn every_filter_kind_is_added_to_the_paint_channel_with_one_undo_step() {
    for kind in FilterKind::ALL {
        let mut s = AppState::new(64, 64);
        let layer = s.selected_layer.unwrap();
        // スカラーとマスクだけの種類は、スカラーのチャンネル（Roughness）を描いているときに足す
        let scalar_only = matches!(
            kind,
            FilterKind::HistogramScan
                | FilterKind::HistogramRange
                | FilterKind::Morphology
                | FilterKind::EdgeDetect
        );
        let paint = if scalar_only {
            Channel::Roughness
        } else {
            Channel::Color
        };
        s.apply(Action::M2Ui(UiOp::PaintChannel(paint)));
        let before = s.doc.undo_count();
        add_filter(&mut s, FilterTarget::Content, kind);
        assert_eq!(
            filters(&s, layer, FilterTarget::Content),
            1,
            "{kind:?}: {}",
            s.message
        );
        assert_eq!(s.doc.undo_count(), before + 1, "{kind:?}: 1 回の Undo");
        let stage = &s.doc.filters_of(layer, FilterTarget::Content).unwrap()[0];
        assert_eq!(
            stage.channels(),
            &[paint],
            "{kind:?}: 描くチャンネルだけに掛かる"
        );
        assert_eq!(stage.settings(), &kind.settings(), "{kind:?}");
        assert_eq!(
            s.fx.selected,
            Some(Selected::Filter {
                layer,
                id: stage.id()
            }),
            "足したものを選ぶ"
        );
        assert!(s.modified);
        s.apply(Action::Undo);
        assert_eq!(
            filters(&s, layer, FilterTarget::Content),
            0,
            "{kind:?}: Undo 1 回で戻る"
        );
        s.apply(Action::Redo);
        assert_eq!(
            filters(&s, layer, FilterTarget::Content),
            1,
            "{kind:?}: Redo"
        );
    }
}

#[test]
fn the_names_core_puts_in_its_texts_are_the_menu_names() {
    // core の `EffectSettings::name()`（大きさを変えたときの知らせに出る）は、0.5.0 の種類も、メニューと同じ日本語の名前
    for kind in FilterKind::ALL {
        if kind.catalog_id().is_some() {
            assert_eq!(kind.settings().name(), kind.name(Lang::Ja), "{kind:?}");
        }
    }
    for kind in [Kind::Pattern, Kind::Light, Kind::MaskBuilder] {
        assert_eq!(
            yolu_core::effects::generator_kind_name(kind),
            yolu_app::fx::names::generator_name(Lang::Ja, kind)
        );
    }
    // スカラーとマスクだけの種類を色のチャンネルへ足す断り（メニューの押せない項目のツールチップ）も、日英ともメニューの名前で挙げる
    let scalar_only = [
        FilterKind::HistogramScan,
        FilterKind::HistogramRange,
        FilterKind::Morphology,
        FilterKind::EdgeDetect,
    ];
    let mut s = AppState::new(32, 32);
    let layer = s.selected_layer.unwrap();
    for kind in scalar_only {
        let error = s
            .doc
            .add_filter(
                layer,
                FilterTarget::Content,
                yolu_core::FilterSpec::new(kind.settings()).channels(&[Channel::Color]),
            )
            .expect_err("色のチャンネルには足せない");
        for lang in Lang::ALL {
            let text = lang.core_error(&error);
            for other in scalar_only {
                assert!(text.contains(other.name(lang)), "{lang:?} {kind:?}: {text}");
            }
        }
    }
}

#[test]
fn filters_go_to_the_current_paint_channel() {
    let mut s = AppState::new(64, 64);
    let layer = s.selected_layer.unwrap();
    s.apply(Action::M2Ui(UiOp::PaintChannel(Channel::Roughness)));
    add_filter(&mut s, FilterTarget::Content, FilterKind::Invert);
    let stage = &s.doc.filters_of(layer, FilterTarget::Content).unwrap()[0];
    assert_eq!(stage.channels(), &[Channel::Roughness]);
    // 法線のチャンネルにはぼかしだけ（核の決め）。断りは理由つきで、何も足さない
    s.apply(Action::M2Ui(UiOp::PaintChannel(Channel::Normal)));
    let steps = s.doc.undo_count();
    add_filter(&mut s, FilterTarget::Content, FilterKind::Invert);
    assert_eq!(s.doc.undo_count(), steps, "断ったら何も変えない");
    assert!(!s.message.is_empty());
}

#[test]
fn mask_stacks_take_filters_and_generators_and_refuse_what_a_mask_cannot_use() {
    let mut s = AppState::new(64, 64);
    let layer = s.selected_layer.unwrap();
    s.apply(Action::M2(Edit::AddMask(layer)));
    add_filter(&mut s, FilterTarget::Mask, FilterKind::Blur);
    assert_eq!(filters(&s, layer, FilterTarget::Mask), 1, "{}", s.message);
    assert_eq!(filters(&s, layer, FilterTarget::Content), 0);
    add_generator(&mut s, FilterTarget::Mask, Kind::Dirt);
    assert_eq!(filters(&s, layer, FilterTarget::Mask), 2, "{}", s.message);
    // 色のノイズはマスクに使えない
    let steps = s.doc.undo_count();
    add_filter(&mut s, FilterTarget::Mask, FilterKind::NoiseColor);
    assert_eq!(s.doc.undo_count(), steps);
    assert_eq!(filters(&s, layer, FilterTarget::Mask), 2);
    // マスクの無いレイヤーのマスクへは足せない
    s.apply(Action::NewLayer);
    let plain = s.selected_layer.unwrap();
    add_filter(&mut s, FilterTarget::Mask, FilterKind::Blur);
    assert_eq!(filters(&s, plain, FilterTarget::Mask), 0);
}

#[test]
fn reorder_enable_and_remove_are_one_undo_step_each() {
    let mut s = AppState::new(64, 64);
    let layer = s.selected_layer.unwrap();
    add_filter(&mut s, FilterTarget::Content, FilterKind::Blur);
    add_filter(&mut s, FilterTarget::Content, FilterKind::Invert);
    let ids: Vec<_> = s
        .doc
        .filters_of(layer, FilterTarget::Content)
        .unwrap()
        .iter()
        .map(|e| e.id())
        .collect();
    let steps = s.doc.undo_count();
    fx(
        &mut s,
        FxOp::Move {
            layer,
            id: ids[0],
            index: 1,
        },
    );
    assert_eq!(s.doc.undo_count(), steps + 1);
    let after: Vec<_> = s
        .doc
        .filters_of(layer, FilterTarget::Content)
        .unwrap()
        .iter()
        .map(|e| e.id())
        .collect();
    assert_eq!(after, vec![ids[1], ids[0]]);
    fx(
        &mut s,
        FxOp::SetEnabled {
            layer,
            id: ids[0],
            enabled: false,
        },
    );
    assert_eq!(s.doc.undo_count(), steps + 2);
    assert!(!s.doc.find_filter(ids[0]).unwrap().1.enabled());
    // 同じ値に直しても履歴は増えない
    fx(
        &mut s,
        FxOp::SetEnabled {
            layer,
            id: ids[0],
            enabled: false,
        },
    );
    assert_eq!(s.doc.undo_count(), steps + 2);
    fx(&mut s, FxOp::Remove { layer, id: ids[0] });
    assert_eq!(s.doc.undo_count(), steps + 3);
    assert_eq!(filters(&s, layer, FilterTarget::Content), 1);
    // 1 回ずつ逆の順に戻る: 削除 → 無効化 → 並べ替え（順序と有効を確かめる）
    let order = |s: &AppState| -> Vec<_> {
        s.doc
            .filters_of(layer, FilterTarget::Content)
            .unwrap()
            .iter()
            .map(|e| (e.id(), e.enabled()))
            .collect()
    };
    assert_eq!(order(&s), vec![(ids[1], true)]);
    s.apply(Action::Undo);
    assert_eq!(
        order(&s),
        vec![(ids[1], true), (ids[0], false)],
        "削除が戻る（無効のまま）"
    );
    s.apply(Action::Undo);
    assert_eq!(
        order(&s),
        vec![(ids[1], true), (ids[0], true)],
        "無効化が戻る（順序はそのまま）"
    );
    s.apply(Action::Undo);
    assert_eq!(
        order(&s),
        vec![(ids[0], true), (ids[1], true)],
        "並べ替えが戻る"
    );
    assert_eq!(s.doc.undo_count(), steps);
    // やり直しも 1 回ずつ
    s.apply(Action::Redo);
    assert_eq!(order(&s), vec![(ids[1], true), (ids[0], true)]);
    s.apply(Action::Redo);
    s.apply(Action::Redo);
    assert_eq!(order(&s), vec![(ids[1], true)]);
}

#[test]
fn a_slider_drag_is_one_undo_step_and_menu_choices_are_separate_steps() {
    let mut s = AppState::new(64, 64);
    let layer = s.selected_layer.unwrap();
    add_filter(&mut s, FilterTarget::Content, FilterKind::Blur);
    let id = s.doc.filters_of(layer, FilterTarget::Content).unwrap()[0].id();
    let steps = s.doc.undo_count();
    for radius in [5, 6, 7, 8, 9] {
        fx(
            &mut s,
            FxOp::SetSettings {
                layer,
                id,
                settings: EffectSettings::blur(radius),
                coalesce: true,
            },
        );
    }
    s.m2_end_drag();
    assert_eq!(s.doc.undo_count(), steps + 1, "ドラッグは 1 回の Undo");
    for strength in [0.9, 0.8, 0.7] {
        fx(
            &mut s,
            FxOp::SetStrength {
                layer,
                id,
                strength,
                coalesce: true,
            },
        );
    }
    s.m2_end_drag();
    assert_eq!(s.doc.undo_count(), steps + 2);
    s.apply(Action::Undo);
    assert_eq!(s.doc.find_filter(id).unwrap().1.strength(), 1.0);
    s.apply(Action::Undo);
    assert_eq!(
        s.doc.find_filter(id).unwrap().1.settings(),
        &EffectSettings::blur(4)
    );
    // 範囲の外の値は断る（何も変えない）
    let steps = s.doc.undo_count();
    fx(
        &mut s,
        FxOp::SetSettings {
            layer,
            id,
            settings: EffectSettings::blur(9999),
            coalesce: false,
        },
    );
    assert_eq!(s.doc.undo_count(), steps);
    assert!(!s.message.is_empty());
}

// ───────── ロックでの断り ─────────

#[test]
fn locks_refuse_effect_edits_with_a_reason_in_both_languages_and_change_nothing() {
    for lang in Lang::ALL {
        let mut s = AppState::new(64, 64);
        s.lang = lang;
        let layer = s.selected_layer.unwrap();
        add_filter(&mut s, FilterTarget::Content, FilterKind::Blur);
        let id = s.doc.filters_of(layer, FilterTarget::Content).unwrap()[0].id();
        s.doc.set_layer_locks(layer, LayerLocks::ALL).unwrap();
        let (steps, revision) = (s.doc.undo_count(), s.doc.revision());
        s.message.clear();
        add_filter(&mut s, FilterTarget::Content, FilterKind::Invert);
        assert_eq!(s.doc.revision(), revision, "{lang:?}: 追加は断る");
        assert_eq!(
            s.message,
            lang.pick(
                "レイヤーの「すべて」がロックされています",
                "The layer has \"All\" locked"
            ),
            "{lang:?}"
        );
        for op in [
            FxOp::SetEnabled {
                layer,
                id,
                enabled: false,
            },
            FxOp::Remove { layer, id },
            FxOp::SetStrength {
                layer,
                id,
                strength: 0.5,
                coalesce: false,
            },
            FxOp::AddAnchor {
                layer,
                placement: AnchorPlacement::Layer,
            },
        ] {
            s.message.clear();
            fx(&mut s, op.clone());
            assert_eq!(s.doc.revision(), revision, "{lang:?}: {op:?}");
            assert!(!s.message.is_empty(), "{lang:?}: {op:?} の理由");
        }
        assert_eq!(s.doc.undo_count(), steps);
        // ロックを外せば同じ操作が通る
        s.doc.set_layer_locks(layer, LayerLocks::NONE).unwrap();
        add_filter(&mut s, FilterTarget::Content, FilterKind::Invert);
        assert_eq!(filters(&s, layer, FilterTarget::Content), 2, "{lang:?}");
    }
}

#[test]
fn a_pixel_lock_does_not_stop_effects_that_leave_the_pixels_alone() {
    let mut s = AppState::new(64, 64);
    let layer = s.selected_layer.unwrap();
    s.doc.set_layer_locks(layer, LayerLocks::PIXELS).unwrap();
    add_filter(&mut s, FilterTarget::Content, FilterKind::Blur);
    assert_eq!(
        filters(&s, layer, FilterTarget::Content),
        1,
        "{}",
        s.message
    );
}

#[test]
fn nothing_changes_while_drawing() {
    let mut s = AppState::new(64, 64);
    let layer = s.selected_layer.unwrap();
    let brush = s.stroke_settings(false);
    let stroke = s.doc.begin_stroke(layer, &brush).unwrap();
    add_filter(&mut s, FilterTarget::Content, FilterKind::Blur);
    assert_eq!(s.message, "描いている間はできません。");
    assert_eq!(filters(&s, layer, FilterTarget::Content), 0);
    // 行を選ぶだけなら描いている間でも通る
    fx(&mut s, FxOp::Deselect);
    s.doc.cancel_stroke(stroke);
}

// ───────── Anchor ─────────

#[test]
fn anchors_are_put_renamed_removed_and_read_by_a_generator() {
    let mut s = AppState::new(64, 64);
    let bottom = s.selected_layer.unwrap();
    s.apply(Action::NewLayer);
    let top = s.selected_layer.unwrap();
    let steps = s.doc.undo_count();
    fx(
        &mut s,
        FxOp::AddAnchor {
            layer: bottom,
            placement: AnchorPlacement::Layer,
        },
    );
    assert_eq!(s.doc.undo_count(), steps + 1, "{}", s.message);
    let anchor = s.doc.layer(bottom).unwrap().anchor().unwrap().clone();
    assert_eq!(
        anchor.name(),
        s.doc.layer(bottom).unwrap().name(),
        "名前の既定はレイヤーの名前"
    );
    assert_eq!(s.fx.selected, Some(Selected::Anchor { id: anchor.id() }));
    // 同じレイヤーにもう 1 つは置けない
    let steps = s.doc.undo_count();
    fx(
        &mut s,
        FxOp::AddAnchor {
            layer: bottom,
            placement: AnchorPlacement::Layer,
        },
    );
    assert_eq!(s.doc.undo_count(), steps);
    // 名前の変更
    fx(
        &mut s,
        FxOp::RenameAnchor {
            id: anchor.id(),
            name: "  下地  ".into(),
        },
    );
    assert_eq!(
        s.doc.layer(bottom).unwrap().anchor().unwrap().name(),
        "下地"
    );
    fx(
        &mut s,
        FxOp::RenameAnchor {
            id: anchor.id(),
            name: "   ".into(),
        },
    );
    assert_eq!(
        s.doc.layer(bottom).unwrap().anchor().unwrap().name(),
        "下地",
        "空の名前は断る"
    );
    // 上のレイヤーの Anchor の Generator は、すぐ下のアンカーを読む
    s.selected_layer = Some(top);
    add_generator(&mut s, FilterTarget::Content, Kind::Anchor);
    let stage = s.doc.filters_of(top, FilterTarget::Content).unwrap()[0].clone();
    let g = stage.settings().generator_settings().unwrap();
    assert_eq!(g.anchor.id, anchor.id().0, "すぐ下のアンカーを読む");
    assert_eq!(
        s.doc.generator_inactive(top, stage.id()).unwrap(),
        None,
        "{}",
        s.message
    );
    s.sync_effects(); // 画面は毎フレーム見る（前の状態を覚える）
                      // アンカーを外すと読む段は入力のまま通し、知らせる。取り消せば戻る
    fx(&mut s, FxOp::RemoveAnchor(anchor.id()));
    assert!(s.doc.layer(bottom).unwrap().anchor().is_none());
    assert!(s.doc.generator_inactive(top, stage.id()).unwrap().is_some());
    s.sync_effects();
    assert!(s.message.contains("入力をそのまま通す"), "{}", s.message);
    s.apply(Action::Undo);
    s.sync_effects();
    assert!(s.doc.layer(bottom).unwrap().anchor().is_some());
    assert_eq!(s.doc.generator_inactive(top, stage.id()).unwrap(), None);
    // レイヤーを並べ替えて下になると読めない（知らせる。編集は断らない）
    s.selected_layer = Some(top);
    s.apply(Action::LayerDown);
    s.sync_effects();
    assert!(s.doc.generator_inactive(top, stage.id()).unwrap().is_some());
    assert!(s.message.contains("入力をそのまま通す"), "{}", s.message);
}

#[test]
fn a_mask_anchor_needs_the_mask_and_an_anchor_generator_needs_an_anchor_below() {
    let mut s = AppState::new(64, 64);
    let layer = s.selected_layer.unwrap();
    fx(
        &mut s,
        FxOp::AddAnchor {
            layer,
            placement: AnchorPlacement::Mask,
        },
    );
    assert!(s.doc.layer(layer).unwrap().mask().is_none());
    s.apply(Action::M2(Edit::AddMask(layer)));
    fx(
        &mut s,
        FxOp::AddAnchor {
            layer,
            placement: AnchorPlacement::Mask,
        },
    );
    let mask_anchor = s
        .doc
        .layer(layer)
        .unwrap()
        .mask()
        .unwrap()
        .anchor()
        .unwrap()
        .clone();
    assert!(
        mask_anchor.name().contains("マスク"),
        "{}",
        mask_anchor.name()
    );
    // 自分のレイヤーの Anchor は読めない: メニュー（ジェネレーターを追加）の項目は押せない。ラベルは名前だけで、理由はツールチップ
    s.lang = Lang::En;
    let entries = yolu_app::fx::menu::add_generator_entries(&s, FilterTarget::Content);
    let anchor_entry = yolu_app::ui::menu::leaves(&entries)
        .into_iter()
        .find_map(|e| match e {
            yolu_app::ui::menu::Entry::Item {
                label,
                enabled,
                tooltip,
                ..
            } if label.starts_with("Anchor") => Some((label.clone(), *enabled, tooltip.clone())),
            _ => None,
        })
        .unwrap();
    assert!(!anchor_entry.1, "{}", anchor_entry.0);
    assert_eq!(anchor_entry.0, "Anchor", "ラベルに理由を続けない");
    assert!(
        anchor_entry
            .2
            .as_deref()
            .is_some_and(|t| t.contains("no anchor below")),
        "{:?}",
        anchor_entry.2
    );
}

// ───────── メニュー ─────────

#[test]
fn the_add_menu_lists_every_kind_and_gives_a_reason_for_the_ones_a_channel_refuses() {
    use yolu_app::ui::menu::{leaves, Entry};
    for lang in Lang::ALL {
        let mut s = AppState::new(64, 64);
        s.lang = lang;
        // 入り口は 2 つ: 「フィルターを追加」はフィルターだけ、「ジェネレーターを追加」はジェネレーターだけを平らに並べる
        // （見出し・区切り・入れ子は置かない）
        let filters = yolu_app::fx::menu::add_filter_entries(&s, FilterTarget::Content);
        let generators = yolu_app::fx::menu::add_generator_entries(&s, FilterTarget::Content);
        for (entries, count) in [
            (&filters, yolu_app::fx::FilterKind::ALL.len()),
            (&generators, yolu_app::fx::names::GENERATOR_KINDS.len()),
        ] {
            assert_eq!(entries.len(), count, "{lang:?}");
            assert!(
                entries.iter().all(|e| matches!(e, Entry::Item { .. })),
                "{lang:?}: 見出し・区切り・入れ子を置かない"
            );
        }
        // 断られる種類（色のチャンネルにスカラーだけのフィルター）は押せない項目で残る
        assert!(filters.iter().all(|e| matches!(
            e,
            Entry::Item {
                action: Action::Fx(FxOp::AddFilter { .. }),
                ..
            } | Entry::Item { enabled: false, .. }
        )));
        assert!(generators.iter().all(|e| matches!(
            e,
            Entry::Item {
                action: Action::Fx(FxOp::AddGenerator { .. }) | Action::Fx(FxOp::Deselect),
                ..
            }
        )));
        let layer = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::AddMask(layer)));
        for target in [FilterTarget::Content, FilterTarget::Mask] {
            let entries = yolu_app::fx::menu::add_generator_entries(&s, target);
            for expected in [Kind::Noise, Kind::Grunge] {
                assert!(leaves(&entries).iter().any(|e| matches!(e,
                    Entry::Item { action: Action::Fx(FxOp::AddGenerator { kind, .. }), enabled: true, .. } if *kind == expected
                )));
            }
        }

        // 法線のチャンネル: ぼかし以外は押せない。理由はラベルに続けず、ツールチップに置く
        s.apply(Action::M2Ui(UiOp::PaintChannel(Channel::Normal)));
        let mut entries = yolu_app::fx::menu::add_filter_entries(&s, FilterTarget::Content);
        entries.extend(yolu_app::fx::menu::add_generator_entries(
            &s,
            FilterTarget::Content,
        ));
        let disabled: Vec<(&String, &Option<String>)> = leaves(&entries)
            .into_iter()
            .filter_map(|e| match e {
                Entry::Item {
                    label,
                    enabled: false,
                    tooltip,
                    ..
                } => Some((label, tooltip)),
                _ => None,
            })
            .collect();
        assert!(disabled.len() >= 6, "{lang:?}: {disabled:?}");
        assert!(
            disabled.iter().all(|(l, _)| !l.contains(" — ")),
            "{lang:?}: ラベルに理由を続けない {disabled:?}"
        );
        assert!(
            disabled
                .iter()
                .all(|(_, t)| t.as_deref().is_some_and(|t| !t.is_empty())),
            "{lang:?}: 理由はツールチップに {disabled:?}"
        );
        if lang == Lang::En {
            assert!(
                disabled.iter().all(|(l, t)| !l
                    .chars()
                    .chain(t.iter().flat_map(|t| t.chars()))
                    .any(|c| matches!(c, '\u{3040}'..='\u{9fff}'))),
                "{disabled:?}"
            );
        }
    }
}

#[test]
fn the_filter_menu_is_in_the_menu_bar_before_view() {
    let titles = yolu_app::shell::menu_titles(Lang::Ja);
    assert_eq!(titles[4], "フィルター");
    assert_eq!(yolu_app::shell::menu_titles(Lang::En)[4], "Filter");
    let s = AppState::new(64, 64);
    let entries = yolu_app::shell::menu_entries(&s, 4);
    assert!(entries.len() > yolu_app::fx::FilterKind::ALL.len());
}

// ───────── 入力のつなぎ ─────────

/// 試しの立方体を読み、速く焼ける設定にした状態（焼く場所は CPU）。
fn cube() -> AppState {
    let mut s = AppState::new(64, 64);
    s.bake.backend = BakeBackend::Cpu;
    s.apply(Action::LoadDemoModel);
    s.bake.settings.maps = vec![
        MeshMapKind::WorldNormal,
        MeshMapKind::Position,
        MeshMapKind::AmbientOcclusion,
        MeshMapKind::Curvature,
    ];
    s.bake.settings.ao_samples = 8;
    s.bake.settings.padding = 4;
    s
}

fn bake(s: &mut AppState) {
    s.apply(Action::Bake(BakeAction::Start));
    s.wait_bake();
    s.sync_effects();
}

/// 黒い塗りつぶしレイヤーにマスクを付け、そのマスクへ Generator を足す。
fn masked_fill(s: &mut AppState, kind: Kind) -> (LayerId, yolu_core::FilterId) {
    s.apply(Action::M2(Edit::NewFill));
    let layer = s.selected_layer.unwrap();
    s.apply(Action::M2(Edit::AddMask(layer)));
    add_generator(s, FilterTarget::Mask, kind);
    let id = s.doc.filters_of(layer, FilterTarget::Mask).unwrap()[0].id();
    (layer, id)
}

#[test]
fn a_generator_says_which_map_is_missing_until_the_maps_are_baked_and_then_works() {
    let mut s = cube();
    let (layer, id) = masked_fill(&mut s, Kind::EdgeWear);
    s.sync_effects();
    // 焼く前: 理由（足りないマップ）が出て、入力のまま通す（日英）
    let why = s
        .doc
        .generator_inactive(layer, id)
        .unwrap()
        .expect("マップが無い");
    assert!(
        s.lang.inactive_reason(&why).contains("Curvature"),
        "{why:?}"
    );
    assert_eq!(Lang::En.inactive_reason(&why), "No Curvature map");
    assert!(
        s.message.contains("効果がありません"),
        "足したときの知らせに理由を添える: {}",
        s.message
    );
    let before = composite(&s);
    assert!(!s.doc.inactive_effect_list().is_empty());
    // 焼いた後: 効く（読むマップがそろい、合成が変わる）
    bake(&mut s);
    assert_eq!(
        s.doc.generator_inactive(layer, id).unwrap(),
        None,
        "{}",
        s.message
    );
    assert!(s.doc.inactive_effect_list().is_empty());
    assert_ne!(composite(&s), before, "焼いたマップで合成が変わる");
}

/// 焼いたマップがそろっているところへ、そのマップを読むジェネレーターを足す。文書へ渡すマップは今の効果が読む分だけなので、足した直後は
/// まだ渡っていないが、その場で渡してから理由を見る: 足した知らせに「効果がありません（…のマップがありません）」が出ない（日英）。
/// 本当に焼いていないマップ（厚み）は、今までどおり理由を言う。
#[test]
fn adding_a_generator_whose_maps_are_baked_does_not_claim_a_map_is_missing() {
    for lang in [Lang::Ja, Lang::En] {
        let claim = lang.pick("効果がありません", "It has no effect");
        // 汚れ・隙間（AO と曲率）・エッジの摩耗（曲率）・位置のグラデーション（位置）・向き（ワールドの法線）
        for kind in [
            Kind::Dirt,
            Kind::EdgeWear,
            Kind::PositionGradient,
            Kind::Direction,
        ] {
            let mut s = cube();
            s.lang = lang;
            bake(&mut s);
            assert!(s.doc.inactive_effect_list().is_empty());
            let (layer, id) = masked_fill(&mut s, kind);
            assert!(
                !s.message.contains(claim),
                "{kind:?} {lang:?}: {}",
                s.message
            );
            assert_eq!(
                s.doc.generator_inactive(layer, id).unwrap(),
                None,
                "{kind:?}: {}",
                s.message
            );
            // 足すだけでは取り消しの段は 1 つ（入力を渡すのは文書の版を上げない）
            s.apply(Action::Undo);
            assert_eq!(filters(&s, layer, FilterTarget::Mask), 0, "{kind:?}");
        }
        // 焼いていないマップ（厚み）は本当に無い
        let mut s = cube();
        s.lang = lang;
        bake(&mut s);
        let (layer, id) = masked_fill(&mut s, Kind::Thickness);
        assert!(s.message.contains(claim), "{lang:?}: {}", s.message);
        assert!(s.doc.generator_inactive(layer, id).unwrap().is_some());
    }
}

#[test]
fn maps_go_stale_with_the_bake_settings_and_the_generator_says_so() {
    let mut s = cube();
    let (layer, id) = masked_fill(&mut s, Kind::EdgeWear);
    bake(&mut s);
    assert_eq!(s.doc.generator_inactive(layer, id).unwrap(), None);
    // ベイクの設定（余白）を変えると、前のマップは古い（黙って使わない）
    s.bake.settings.padding = 8;
    s.sync_effects();
    let why = s.doc.generator_inactive(layer, id).unwrap().expect("古い");
    assert!(
        matches!(
            why,
            yolu_core::InactiveReason::Generator(generator::Inactive::StaleMap(_))
        ),
        "{why:?}"
    );
    // 焼き直すと効く
    bake(&mut s);
    assert_eq!(s.doc.generator_inactive(layer, id).unwrap(), None);
}

#[test]
fn rebaking_redraws_only_the_layers_that_read_the_maps() {
    let mut s = cube();
    // 下: 効果なしの塗りつぶし、真ん中: ぼかしだけ、上: マップを読む Generator
    s.apply(Action::M2(Edit::NewFill));
    s.apply(Action::M2(Edit::NewFill));
    let blur_layer = s.selected_layer.unwrap();
    add_filter(&mut s, FilterTarget::Content, FilterKind::Blur);
    let (reader, _) = masked_fill(&mut s, Kind::EdgeWear);
    let _ = (blur_layer, reader);
    bake(&mut s);
    composite(&s);
    let evaluated = |s: &AppState| s.doc.effect_counters().blocks_evaluated;
    let settled = evaluated(&s);
    composite(&s);
    assert_eq!(evaluated(&s), settled, "入力が同じなら評価し直さない");
    // 条件を変えて焼き直す（余白を変えると、焼いたマップの条件の鍵が変わる）
    s.bake.settings.padding = 8;
    bake(&mut s);
    composite(&s);
    let redrawn = evaluated(&s) - settled;
    assert_eq!(
        redrawn, 1,
        "マップを読むレイヤーの 1 ブロックだけ（ぼかしのレイヤーは評価し直さない）"
    );
}

#[test]
fn the_same_maps_passed_again_evaluate_nothing_again() {
    let mut s = cube();
    masked_fill(&mut s, Kind::Dirt);
    bake(&mut s);
    composite(&s);
    let passed = s.fx.inputs.passed;
    let counted = s.doc.effect_counters().blocks_evaluated;
    for _ in 0..5 {
        s.sync_effects();
    }
    composite(&s);
    assert_eq!(s.fx.inputs.passed, passed, "鍵が変わらなければ渡し直さない");
    assert_eq!(s.doc.effect_counters().blocks_evaluated, counted);
}

#[test]
fn id_colors_are_picked_from_the_id_map_and_the_generator_then_works() {
    let mut s = cube();
    s.bake.settings.maps = vec![MeshMapKind::Id, MeshMapKind::Position];
    let (layer, id) = masked_fill(&mut s, Kind::IdColor);
    bake(&mut s);
    // 色が無いあいだは効かない（文書の中で直せる設定の不備）
    let why = s
        .doc
        .generator_inactive(layer, id)
        .unwrap()
        .expect("ID の色が無い");
    assert!(
        matches!(
            why,
            yolu_core::InactiveReason::Generator(generator::Inactive::NoIdColors)
        ),
        "{why:?}"
    );
    // 選ぶのを始めると、「ID の色で選択」のツールの入力を使う（効果の欄は開いたまま）
    fx(
        &mut s,
        FxOp::PickIdColors {
            layer,
            id,
            on: true,
        },
    );
    assert_eq!(s.tool, yolu_app::state::Tool::IdSelect);
    assert_eq!(s.fx.id_pick, Some((layer, id)));
    assert!(s.fx.selected.is_some(), "効果の欄は閉じない");
    // 焼いた ID マップの、どこかの画素の色
    let map = s
        .sets
        .current()
        .mesh_maps
        .get(MeshMapKind::Id)
        .unwrap()
        .clone();
    let rgb = (0..64)
        .flat_map(|y| (0..64).map(move |x| (x, y)))
        .find_map(|(x, y)| yolu_core::id_colors::try_get(&map, x, y).ok().flatten())
        .expect("ID の色のある画素");
    let steps = s.doc.undo_count();
    assert!(s.pick_id_color(rgb));
    assert_eq!(s.doc.undo_count(), steps + 1, "足すのは 1 回の Undo");
    let colors = |s: &AppState| {
        s.doc
            .find_filter(id)
            .unwrap()
            .1
            .settings()
            .generator_settings()
            .unwrap()
            .id_colors
            .clone()
    };
    assert_eq!(colors(&s), vec![rgb]);
    assert_eq!(
        s.doc.generator_inactive(layer, id).unwrap(),
        None,
        "色が選ばれたので効く"
    );
    // 同じ色をもう一度押しても増えず、もう入っていると知らせる。Ctrl を押していれば外し、入っていない色を外そうとしても知らせる
    assert!(s.pick_id_color(rgb));
    assert_eq!(colors(&s), vec![rgb]);
    assert_eq!(s.doc.undo_count(), steps + 1);
    assert!(s.message.contains("入っています"), "{}", s.message);
    s.region.modifiers.command = true;
    assert!(s.pick_id_color(rgb));
    assert!(colors(&s).is_empty());
    assert!(s.message.contains("外しました"), "{}", s.message);
    assert!(s.pick_id_color(rgb));
    assert!(s.message.contains("入っていません"), "{}", s.message);
    s.lang = Lang::En;
    assert!(s.pick_id_color(rgb));
    assert!(
        s.message.contains("is not in the ID colors"),
        "{}",
        s.message
    );
    s.lang = Lang::Ja;
    s.region.modifiers.command = false;
    s.apply(Action::Undo);
    assert_eq!(colors(&s), vec![rgb]);
    // ツールを替えると選ぶのをやめ、押しても選択のツールとして働く
    s.apply(Action::SelectTool(yolu_app::state::Tool::Brush));
    assert_eq!(s.fx.id_pick, None);
    assert!(!s.pick_id_color(rgb));
}

/// 焼いた ID マップの、色のある画素の座標と色。
fn id_pixel(s: &AppState) -> ((i64, i64), u32) {
    let map = s
        .sets
        .current()
        .mesh_maps
        .get(MeshMapKind::Id)
        .unwrap()
        .clone();
    (0..64)
        .flat_map(|y| (0..64).map(move |x| (x, y)))
        .find_map(|(x, y)| {
            yolu_core::id_colors::try_get(&map, x, y)
                .ok()
                .flatten()
                .map(|rgb| ((x, y), rgb))
        })
        .expect("ID の色のある画素")
}

#[test]
fn id_colors_come_in_through_the_id_select_press_without_making_a_selection() {
    use yolu_app::state::{StrokeSource, Tool};
    let mut s = cube();
    s.bake.settings.maps = vec![MeshMapKind::Id, MeshMapKind::Position];
    let (layer, id) = masked_fill(&mut s, Kind::IdColor);
    bake(&mut s);
    fx(
        &mut s,
        FxOp::PickIdColors {
            layer,
            id,
            on: true,
        },
    );
    assert_eq!(s.tool, Tool::IdSelect);
    let ((x, y), rgb) = id_pixel(&s);
    let colors = |s: &AppState| {
        s.doc
            .find_filter(id)
            .unwrap()
            .1
            .settings()
            .generator_settings()
            .unwrap()
            .id_colors
            .clone()
    };
    // 2D のキャンバスの押下（ID の色で選択の入口）から足す。選択範囲は作らない
    let rect = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(600.0, 400.0));
    let view = s.view.view(rect, s.doc.width(), s.doc.height());
    let at = view.to_screen(x as f64 + 0.5, y as f64 + 0.5);
    let steps = s.doc.undo_count();
    assert!(s.doc.selection().is_none());
    yolu_app::region::tools::canvas_press(&mut s, &view, at, StrokeSource::Mouse);
    assert_eq!(colors(&s), vec![rgb], "{}", s.message);
    assert_eq!(s.doc.undo_count(), steps + 1, "足すのは 1 回の Undo");
    assert!(s.doc.selection().is_none(), "選択範囲は作らない");
    assert_eq!(s.doc.generator_inactive(layer, id).unwrap(), None);
    // もう一度押しても増えない（知らせる）。Ctrl を押していれば外れる
    yolu_app::region::tools::canvas_press(&mut s, &view, at, StrokeSource::Mouse);
    assert_eq!(colors(&s), vec![rgb]);
    assert!(s.message.contains("入っています"), "{}", s.message);
    s.region.modifiers.command = true;
    yolu_app::region::tools::canvas_press(&mut s, &view, at, StrokeSource::Mouse);
    s.region.modifiers.command = false;
    assert!(colors(&s).is_empty(), "{}", s.message);
    assert!(s.doc.selection().is_none());
    // 選ぶのをやめると、同じ押下は選択のツールとして働く（選択範囲ができ、ID の色は変わらない）
    fx(
        &mut s,
        FxOp::PickIdColors {
            layer,
            id,
            on: false,
        },
    );
    yolu_app::region::tools::canvas_press(&mut s, &view, at, StrokeSource::Mouse);
    assert!(s.doc.selection().is_some(), "{}", s.message);
    assert!(colors(&s).is_empty());
}

#[test]
fn picking_id_colors_changes_the_tool_the_same_way_as_choosing_it_and_not_while_drawing() {
    use yolu_app::pathtool::PointRef;
    use yolu_app::state::Tool;
    let mut s = cube();
    s.bake.settings.maps = vec![MeshMapKind::Id, MeshMapKind::Position];
    let (layer, id) = masked_fill(&mut s, Kind::IdColor);
    // パスのツールで点を選び、スライダーの途中の値がある
    s.apply(Action::SelectTool(Tool::Path));
    s.path.selected = Some(PointRef {
        layer,
        path: 1,
        index: 0,
    });
    s.path.pending = Some(("width", 3.0));
    s.select_effect(layer, id);
    fx(
        &mut s,
        FxOp::PickIdColors {
            layer,
            id,
            on: true,
        },
    );
    assert_eq!(s.tool, Tool::IdSelect);
    assert!(
        s.path.selected.is_none() && s.path.pending.is_none() && s.path.drag.is_none(),
        "パスの途中の状態を捨てる"
    );
    assert_eq!(s.fx.id_pick, Some((layer, id)));
    assert_eq!(
        s.fx.selected,
        Some(Selected::Filter { layer, id }),
        "効果の欄は開いたまま"
    );
    // 同じツールのままもう一度押しても同じ
    fx(
        &mut s,
        FxOp::PickIdColors {
            layer,
            id,
            on: true,
        },
    );
    assert_eq!(s.fx.id_pick, Some((layer, id)));
    // 描いている間はツールを替えない（断って、何も変えない）
    fx(
        &mut s,
        FxOp::PickIdColors {
            layer,
            id,
            on: false,
        },
    );
    s.apply(Action::SelectTool(Tool::Brush));
    let base = s.doc.layers()[0].id();
    let brush = s.stroke_settings(false);
    let stroke = s.doc.begin_stroke(base, &brush).unwrap();
    s.message.clear();
    fx(
        &mut s,
        FxOp::PickIdColors {
            layer,
            id,
            on: true,
        },
    );
    assert_eq!(s.message, "描いている間はできません。");
    assert_eq!(s.tool, Tool::Brush);
    assert_eq!(s.fx.id_pick, None);
    // 選ぶのをやめる操作は描いている間でも通る
    fx(
        &mut s,
        FxOp::PickIdColors {
            layer,
            id,
            on: false,
        },
    );
    s.doc.cancel_stroke(stroke);
    fx(
        &mut s,
        FxOp::PickIdColors {
            layer,
            id,
            on: true,
        },
    );
    assert_eq!(s.tool, Tool::IdSelect);
}

// ───────── 保存復元 ─────────

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("yolu-fx-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 効果の数（画素とマスクの段・Anchor）。
fn effect_counts(app: &AppState) -> (usize, usize, usize) {
    let (mut stages, mut mask_stages) = (0, 0);
    for l in app.doc.layers() {
        stages += l.filters().len();
        mask_stages += l.mask().map_or(0, |m| m.filters().len());
    }
    (stages, mask_stages, app.doc.anchors().len())
}

#[test]
fn filters_and_anchors_survive_save_and_reopen_and_can_be_edited_after() {
    let dir = temp_dir("roundtrip");
    let path = dir.join("fx.ylp");
    let mut s = AppState::new(64, 64);
    let base = s.selected_layer.unwrap();
    add_filter(&mut s, FilterTarget::Content, FilterKind::Blur);
    add_filter(&mut s, FilterTarget::Content, FilterKind::Levels);
    fx(
        &mut s,
        FxOp::AddAnchor {
            layer: base,
            placement: AnchorPlacement::Layer,
        },
    );
    s.apply(Action::M2(Edit::AddMask(base)));
    add_filter(&mut s, FilterTarget::Mask, FilterKind::Sharpen);
    let expected = effect_counts(&s);
    assert_eq!(expected, (2, 1, 1));
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let mut again = AppState::new(64, 64);
    again.apply(Action::OpenProject(path.clone()));
    assert!(again.read_only_reason().is_none(), "{}", again.message);
    assert_eq!(effect_counts(&again), expected);
    // 開いた文書で編集し、また保存して開ける
    let layer = again.doc.layers()[0].id();
    again.selected_layer = Some(layer);
    add_filter(&mut again, FilterTarget::Content, FilterKind::Invert);
    assert_eq!(effect_counts(&again).0, 3, "{}", again.message);
    again.apply(Action::SaveProject);
    assert!(
        again.message.starts_with("保存しました"),
        "{}",
        again.message
    );
    let mut third = AppState::new(64, 64);
    third.apply(Action::OpenProject(path));
    assert_eq!(effect_counts(&third), (3, 1, 1), "{}", third.message);
    let _ = std::fs::remove_dir_all(dir);
}

/// 0.5.0 のフィルター（種類 70〜79）を画素とマスクに足し、値を変え、保存して開き直しても同じ設定（取り消しで足す前へ戻る）。
#[test]
fn the_new_filters_survive_save_and_reopen_with_their_values() {
    use yolu_core::filter::{MorphologyMode, Settings as F};
    let dir = temp_dir("filters_v28");
    let path = dir.join("fx.ylp");
    let mut s = AppState::new(64, 64);
    let base = s.selected_layer.unwrap();
    let before = s.doc.undo_count();
    for kind in [
        FilterKind::DirectionalBlur,
        FilterKind::SlopeBlur,
        FilterKind::Warp,
        FilterKind::Median,
        FilterKind::HighPass,
        FilterKind::Glow,
    ] {
        add_filter(&mut s, FilterTarget::Content, kind);
    }
    s.apply(Action::M2(Edit::AddMask(base)));
    for kind in [
        FilterKind::HistogramScan,
        FilterKind::HistogramRange,
        FilterKind::Morphology,
        FilterKind::EdgeDetect,
    ] {
        add_filter(&mut s, FilterTarget::Mask, kind);
    }
    // 1 つの値を変える（目録の欄の値で）
    let morph = s.doc.filters_of(base, FilterTarget::Mask).unwrap()[2].id();
    fx(
        &mut s,
        FxOp::SetSettings {
            layer: base,
            id: morph,
            settings: EffectSettings::Filter(F::Morphology {
                mode: MorphologyMode::Erode,
                radius: 7,
            }),
            coalesce: false,
        },
    );
    let stages = |app: &AppState| -> Vec<EffectSettings> {
        app.doc.layers()[0]
            .filters()
            .iter()
            .chain(app.doc.layers()[0].mask().unwrap().filters().iter())
            .map(|e| e.settings().clone())
            .collect()
    };
    let expected = stages(&s);
    assert_eq!(expected.len(), 10, "{}", s.message);
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let mut again = AppState::new(64, 64);
    again.apply(Action::OpenProject(path.clone()));
    assert!(again.read_only_reason().is_none(), "{}", again.message);
    assert_eq!(stages(&again), expected);
    // 取り消しを重ねると、足す前（マスクを足す前）へ戻る
    while s.doc.undo_count() > before {
        s.apply(Action::Undo);
    }
    assert!(s.doc.layers()[0].filters().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// 0.5.0 の模様（マップを読まない Generator）を足して値を変え、保存して開き直しても同じ設定（開いた文書はマップを待たずに編集できる）。
/// ライト・マスクの組み立ての往復は yolu-io の `filters_v28`（マップを読む種類は、開くとマップがそろうまで読むだけ）。
#[test]
fn the_pattern_generator_survives_save_and_reopen() {
    let dir = temp_dir("generators_v28");
    let path = dir.join("gen.ylp");
    let mut s = AppState::new(64, 64);
    let base = s.selected_layer.unwrap();
    add_generator(&mut s, FilterTarget::Content, Kind::Pattern);
    let pattern = s.doc.filters_of(base, FilterTarget::Content).unwrap()[0].id();
    let mut g = generator::Settings::new(Kind::Pattern);
    g.pattern.shape = generator::PatternShape::Checker;
    g.pattern.scale = 3.0;
    fx(
        &mut s,
        FxOp::SetSettings {
            layer: base,
            id: pattern,
            settings: EffectSettings::generator(g),
            coalesce: false,
        },
    );
    let stages = |app: &AppState| -> Vec<EffectSettings> {
        app.doc.layers()[0]
            .filters()
            .iter()
            .map(|e| e.settings().clone())
            .collect()
    };
    let expected = stages(&s);
    assert_eq!(expected.len(), 1, "{}", s.message);
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let mut again = AppState::new(64, 64);
    again.apply(Action::OpenProject(path));
    assert!(again.read_only_reason().is_none(), "{}", again.message);
    assert_eq!(stages(&again), expected, "{}", again.message);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_set_with_generators_is_read_only_until_its_inputs_arrive_and_then_editable() {
    let dir = temp_dir("waiting");
    let path = dir.join("gen.ylp");
    let mut s = cube();
    let (layer, id) = masked_fill(&mut s, Kind::EdgeWear);
    bake(&mut s);
    assert_eq!(s.doc.generator_inactive(layer, id).unwrap(), None);
    let expected = effect_counts(&s);
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);

    // モデルの無い状態で開く: 焼いたマップは照合できないので、読むだけにして足りない入力を言う
    let mut again = AppState::new(64, 64);
    again.bake.backend = BakeBackend::Cpu;
    again.apply(Action::OpenProject(path.clone()));
    let reason = again
        .read_only_reason()
        .expect("入力がそろわない")
        .to_owned();
    assert!(reason.contains("効果の入力がそろっていない"), "{reason}");
    assert!(reason.contains("Curvature"), "足りない入力を言う: {reason}");
    assert!(again.sets.current().waiting_inputs);
    // 読むだけのあいだは文書を変える操作を断る
    let steps = again.doc.undo_count();
    add_filter(&mut again, FilterTarget::Content, FilterKind::Blur);
    assert_eq!(again.doc.undo_count(), steps);
    assert!(again.message.contains("読むだけ"), "{}", again.message);
    // 保存しても、読むだけのセットの正本は元のバイト列のまま残る
    let before = std::fs::read(&path).unwrap();
    again.apply(Action::SaveProject);
    let after = std::fs::read(&path).unwrap();
    let (a, b) = (
        yolu_io::Project::read(&before).unwrap(),
        yolu_io::Project::read(&after).unwrap(),
    );
    assert_eq!(
        a.sets()[0].document.to_bytes().unwrap(),
        b.sets()[0].document.to_bytes().unwrap(),
        "読むだけのセットの正本は書き換えない"
    );

    // 同じモデルを読むと、マップが照合できて、入力がそろい、同じセットが編集できる
    again.apply(Action::LoadDemoModel);
    again.sync_effect_inputs_with(true); // 画面は毎フレーム見る。試験はモデルの入力を作り終えるまで待つ
    assert!(
        again.read_only_reason().is_none(),
        "{:?} {}",
        again.read_only_reason(),
        again.message
    );
    assert!(!again.sets.current().waiting_inputs);
    assert_eq!(effect_counts(&again), expected);
    assert!(again.doc.inactive_effect_list().is_empty(), "効果が効く");
    let steps = again.doc.undo_count();
    again.selected_layer = Some(again.doc.layers().last().unwrap().id());
    add_filter(&mut again, FilterTarget::Content, FilterKind::Invert);
    assert_eq!(again.doc.undo_count(), steps + 1, "{}", again.message);
    let _ = std::fs::remove_dir_all(dir);
}

/// FBX のモデルで焼いて保存した .ylp を、起動した直後のアプリ（3D ビューは試しの立方体）で開く。開いた瞬間はモデルを別のスレッドで
/// 読み直している最中で、マップを立方体と照合するので読むだけになる。読み終えたら、ベイクのウィンドウを開かなくても、画面の毎フレームの
/// 同期（効果の入力を渡す → 使わない入力を手放す）だけでマップが最新になり、同じセットを編集できる。`priority` は焼く前に変える重なった UV の優先。
fn reopened_set_becomes_editable_once_its_model_is_read(
    name: &str,
    priority: Option<yolu_app::bake::overlap::PriorityOp>,
) {
    use crate::common::fbx::{ascii_fbx, mesh, write_fbx};
    use std::time::{Duration, Instant};
    use yolu_app::newproject::NpAction;
    use yolu_core::mesh_maps::MeshMapState;
    let dir = temp_dir(name);
    let model = write_fbx(
        &dir,
        "m.fbx",
        &ascii_fbx(&["Skin"], &[mesh("Body", &[Some(0), Some(0)])]),
    );
    let mut s = AppState::new(64, 64);
    s.bake.backend = BakeBackend::Cpu;
    s.apply(Action::Project(NpAction::OpenNew));
    s.apply(Action::Project(NpAction::ChooseModel(model)));
    let start = Instant::now();
    while s.np.window.as_ref().is_some_and(|w| w.is_loading()) {
        s.poll_newproject();
        assert!(start.elapsed() < Duration::from_secs(60), "モデルの準備");
        std::thread::sleep(Duration::from_millis(5));
    }
    s.apply(Action::Project(NpAction::Resolution(512)));
    s.apply(Action::Project(NpAction::Submit));
    assert!(s.np.window.is_none(), "{}", s.message);
    // エッジの摩耗（Curvature）と、位置のマップ
    s.bake.settings.maps = vec![MeshMapKind::Position, MeshMapKind::Curvature];
    s.bake.settings.padding = 4;
    if let Some(op) = priority {
        s.apply(Action::Bake(BakeAction::Priority(op)));
    }
    let (layer, id) = masked_fill(&mut s, Kind::EdgeWear);
    bake(&mut s);
    assert_eq!(
        s.doc.generator_inactive(layer, id).unwrap(),
        None,
        "{}",
        s.message
    );
    let saved_priority = s.doc.bake_priority().clone();
    let path = dir.join("p.ylp");
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);

    let mut again = AppState::new(64, 64);
    again.bake.backend = BakeBackend::Cpu;
    again.view3d.load_demo(); // 起動したアプリの 3D ビュー
    again.apply(Action::OpenProject(path));
    assert!(
        again.sets.current().waiting_inputs,
        "開いた瞬間のモデルは立方体: {}",
        again.message
    );
    // 読むだけのあいだも、照合とベイクはそのセットのベイクの優先を読む（保存した合成の文書は既定の値ではない）
    assert_eq!(again.doc.bake_priority(), &saved_priority);
    // モデルの読み直しが終わるまで
    let start = Instant::now();
    while again.np.reopening.is_some() {
        again.poll_newproject();
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "モデルの読み直し"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        again.view3d.full_model().is_some_and(|m| !m.demo),
        "{}",
        again.message
    );
    // 画面の 1 フレームと同じ順に回す（ベイクのウィンドウは開かない）
    let start = Instant::now();
    while again.sets.current().waiting_inputs {
        again.sync_effects();
        again.release_idle_bake_input();
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "モデルを読んだのに入力が届かない: {:?}",
            again.read_only_reason()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(again.bake.window.is_none());
    assert!(again.read_only_reason().is_none(), "{}", again.message);
    for kind in [MeshMapKind::Position, MeshMapKind::Curvature] {
        let check = again.mesh_map_check(0, kind).expect("焼いたマップ");
        assert_eq!(
            check.state,
            MeshMapState::Current,
            "{kind:?}: {}",
            yolu_app::bake::stale_reasons(&check)
        );
    }
    assert!(again.doc.inactive_effect_list().is_empty(), "効果が効く");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_reopened_set_becomes_editable_once_its_model_is_read_without_opening_the_bake_window() {
    reopened_set_becomes_editable_once_its_model_is_read("reopen-model", None);
}

#[test]
fn a_reopened_set_baked_with_an_overlap_priority_becomes_editable_once_its_model_is_read() {
    use yolu_app::bake::overlap::PriorityOp;
    use yolu_core::mesh_maps::MeshOverlapRule;
    reopened_set_becomes_editable_once_its_model_is_read(
        "reopen-priority",
        Some(PriorityOp::Rule(MeshOverlapRule::LargerArea)),
    );
}

/// 腕の FBX（骨 3 本・マテリアル 2 つ・UV つき）で位置のマップを焼いた状態（保存していない）。位置のマップを読むノイズ（位置の空間が既定）を、
/// マスクのジェネレーターに持つこともできる。戻りは（状態、置き場のフォルダー、ノイズの段を持つレイヤーと段の ID）。
fn baked_arm(
    name: &str,
    with_noise: bool,
) -> (
    AppState,
    std::path::PathBuf,
    LayerId,
    Option<yolu_core::FilterId>,
) {
    use crate::common::livelink::Exchange;
    use std::time::{Duration, Instant};
    use yolu_app::newproject::NpAction;
    let ex = Exchange::new(name);
    let model = ex.write_arm("arm.fbx");
    let dir = ex.dir.clone();
    std::mem::forget(ex); // 置き場は試験の最後に自分で消す
    let mut s = AppState::new(64, 64);
    s.bake.backend = BakeBackend::Cpu;
    s.apply(Action::Project(NpAction::OpenNew));
    s.apply(Action::Project(NpAction::ChooseModel(model)));
    let start = Instant::now();
    while s.np.window.as_ref().is_some_and(|w| w.is_loading()) {
        s.poll_newproject();
        assert!(start.elapsed() < Duration::from_secs(60), "モデルの準備");
        std::thread::sleep(Duration::from_millis(5));
    }
    s.apply(Action::Project(NpAction::Resolution(256)));
    s.apply(Action::Project(NpAction::Submit));
    assert!(s.np.window.is_none(), "{}", s.message);
    s.bake.settings.maps = vec![MeshMapKind::Position];
    s.bake.settings.padding = 4;
    let (layer, id) = if with_noise {
        let (layer, id) = masked_fill(&mut s, Kind::Noise);
        (layer, Some(id))
    } else {
        s.apply(Action::M2(Edit::NewFill));
        (s.selected_layer.unwrap(), None)
    };
    bake(&mut s);
    assert!(
        s.sets
            .current()
            .mesh_maps
            .get(MeshMapKind::Position)
            .is_some(),
        "{}",
        s.message
    );
    if let Some(id) = id {
        assert_eq!(
            s.doc.generator_fallback(layer, id).unwrap(),
            None,
            "{}",
            s.message
        );
    }
    (s, dir, layer, id)
}

/// `baked_arm` を保存した .ylp。戻りは（保存先の .ylp、レイヤーと段の ID）。
fn arm_project_with_position_noise(
    name: &str,
    with_noise: bool,
) -> (std::path::PathBuf, LayerId, Option<yolu_core::FilterId>) {
    let (mut s, dir, layer, id) = baked_arm(name, with_noise);
    let path = dir.join("arm.ylp");
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    (path, layer, id)
}

/// 腕の「Lower」の骨を曲げる（形が変わり、モデルは作り直される）。
fn bend_the_arm(s: &mut AppState) {
    use yolu_app::view3d::pose::set_pose;
    use yolu_core::glam::Quat;
    let session = s.view3d.pose.session.as_ref().expect("腕のポーズ");
    let mut pose = session.pose().clone();
    let bone = session
        .rig
        .bones()
        .iter()
        .position(|b| b.name == "Lower")
        .expect("腕の骨");
    pose.locals[bone].rotation = Quat::from_rotation_z(0.8);
    set_pose(&mut s.view3d, pose).unwrap();
}

/// 起動した直後のアプリ（3D ビューは試しの立方体）で .ylp を開き、モデルの読み直しが終わるまで待つ。
fn open_on_the_demo_cube(path: &std::path::Path) -> AppState {
    use std::time::{Duration, Instant};
    let mut again = AppState::new(64, 64);
    again.bake.backend = BakeBackend::Cpu;
    again.view3d.load_demo();
    again.apply(Action::OpenProject(path.to_path_buf()));
    let start = Instant::now();
    while again.np.reopening.is_some() {
        again.poll_newproject();
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "モデルの読み直し"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        again.view3d.full_model().is_some_and(|m| !m.demo),
        "{}",
        again.message
    );
    again
}

/// 画面の 1 フレームと同じ順（効果の入力を渡す → 使わない入力を手放す。ベイクのウィンドウは閉じたまま）で、`done` になるまで回す。
fn frames_until(again: &mut AppState, what: &str, mut done: impl FnMut(&AppState) -> bool) {
    use std::time::{Duration, Instant};
    let start = Instant::now();
    while !done(again) {
        again.sync_effects();
        again.release_idle_bake_input();
        assert!(start.elapsed() < Duration::from_secs(30), "{what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// 位置のマップを読むノイズ（位置のマップが使えなければ UV に落として動き続ける）を持つセットは、開いても読むだけにならない。開いた瞬間のモデルは
/// 試しの立方体なので、焼いたマップは立方体と照合して古く、ノイズは UV に落ちる。モデルを読み終えたら、ベイクのウィンドウを開かなくても、
/// 画面の毎フレームの同期だけで入力ができて照合し直し、ノイズは位置で評価する。済んだあとはポーズを付けても、前の入力のまま照合する
/// （ベイクのウィンドウを開いているあいだだけモデルの変更を追う）ので、焼いたマップを読む効果は止まらない。
#[test]
fn a_reopened_set_that_keeps_running_is_checked_again_once_without_opening_the_bake_window() {
    let (path, layer, id) = arm_project_with_position_noise("reopen-noise", true);
    let id = id.unwrap();
    let dir = path.parent().unwrap().to_path_buf();
    let mut again = open_on_the_demo_cube(&path);
    // 読むだけにならずに開く。照合し直すまでは、立方体と照合して古い（ノイズは UV に落ちる）
    assert!(!again.sets.current().waiting_inputs && again.read_only_reason().is_none());
    let before = again.doc.generator_fallback(layer, id).unwrap();
    assert!(
        matches!(before, Some(generator::Inactive::StaleMap(_))),
        "{before:?}"
    );
    // ウィンドウを開かずに毎フレームの同期だけで、読み終えたモデルで照合し直して最新になる
    frames_until(
        &mut again,
        "モデルを読んだのに照合し直せない",
        |a| a.doc.generator_fallback(layer, id).unwrap().is_none(),
    );
    assert!(again.bake.window.is_none());
    let check = again.mesh_map_check(0, MeshMapKind::Position).unwrap();
    assert_eq!(
        check.state,
        yolu_core::mesh_maps::MeshMapState::Current,
        "{}",
        yolu_app::bake::stale_reasons(&check)
    );
    // 済んだあとはポーズを付けても追わない: 前の入力のまま照合するので、効果は止まらない
    bend_the_arm(&mut again);
    for _ in 0..20 {
        again.sync_effects();
        again.release_idle_bake_input();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        again.doc.generator_fallback(layer, id).unwrap(),
        None,
        "ポーズを付けても位置のマップを読み続ける"
    );
    assert!(again.doc.inactive_effect_list().is_empty());
    // ポーズは本当に形を変えている: 今の形の入力で照合すれば（ベイクのウィンドウが開いているときの動き）、位置のマップは古い
    again.bake_input().expect("今の形の入力");
    again.sync_effects();
    assert!(
        matches!(
            again.doc.generator_fallback(layer, id).unwrap(),
            Some(generator::Inactive::StaleMap(_))
        ),
        "{:?}",
        again.doc.generator_fallback(layer, id)
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// 焼いてからポーズを付け、そのあとで位置を読むジェネレーターを追加する（ベイクのウィンドウは閉じたまま）。ポーズは追わず、手元の入力（焼いたときの
/// 形）のまま照合するので、位置のマップは渡って理由が消える。作りかけの入力を毎フレーム立てては捨てることもしない（そうすると入力が届かず、
/// マップがいつまでも渡らなかった）。
#[test]
fn a_generator_added_after_a_pose_gets_the_baked_map_and_no_input_is_built_every_frame() {
    use yolu_app::fx::FxOp;
    let (mut s, dir, layer, _) = baked_arm("pose-add", false);
    bend_the_arm(&mut s);
    let started = s.bake.input_builds_started();
    s.apply(Action::Fx(FxOp::AddGenerator {
        target: FilterTarget::Content,
        kind: Kind::PositionGradient,
    }));
    let id = s.doc.filters_of(layer, FilterTarget::Content).unwrap()[0].id();
    frames_until(
        &mut s,
        "ポーズのあとに足したジェネレーターにマップが渡らない",
        |a| a.doc.generator_inactive(layer, id).unwrap().is_none(),
    );
    for _ in 0..50 {
        s.sync_effects();
        s.release_idle_bake_input();
    }
    assert_eq!(
        s.bake.input_builds_started(),
        started,
        "ウィンドウを閉じたまま、入力の作りかけを毎フレーム立てている"
    );
    assert!(!s.bake.is_checking());
    assert!(s.doc.inactive_effect_list().is_empty());
    // ポーズは形を変えている: ID の色のツールのように今のモデルを追うあいだは、位置のマップは古い
    s.tool = yolu_app::state::Tool::IdSelect;
    frames_until(
        &mut s,
        "ID の色のツールが今のモデルを追わない",
        |a| a.doc.generator_inactive(layer, id).unwrap().is_some(),
    );
    assert!(s.bake.input_builds_started() > started);
    let _ = std::fs::remove_dir_all(dir);
}

/// 効果が焼いたマップを読まないセットも、開いた直後は入力が立方体のままで、後から焼いたマップを読むジェネレーターを足しても、そのマップが渡らず
/// 効かなかった。モデルを読み終えて照合し直したあとに足せば、すぐ効く（ベイクのウィンドウは閉じたまま）。
#[test]
fn a_generator_added_to_a_reopened_set_reads_the_baked_map_without_the_bake_window() {
    use yolu_app::fx::FxOp;
    let (path, _, _) = arm_project_with_position_noise("reopen-add", false);
    let dir = path.parent().unwrap().to_path_buf();
    let mut again = open_on_the_demo_cube(&path);
    assert!(again.read_only_reason().is_none());
    let layer = again.doc.layers().last().unwrap().id();
    again.selected_layer = Some(layer);
    again.apply(Action::Fx(FxOp::AddGenerator {
        target: FilterTarget::Content,
        kind: Kind::PositionGradient,
    }));
    let id = again.doc.filters_of(layer, FilterTarget::Content).unwrap()[0].id();
    frames_until(
        &mut again,
        "足したジェネレーターにマップが渡らない",
        |a| a.doc.generator_inactive(layer, id).unwrap().is_none(),
    );
    assert!(again.bake.window.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_remembered_selections_come_back_when_a_read_only_set_becomes_editable_and_stay_in_the_file()
{
    use yolu_app::engine::SelectionCombine;
    use yolu_app::selection::saved::SavedOp;
    use yolu_app::selection::{SelAction, SelEdit};
    let rect = |x0, y0, x1, y1| SelEdit::Rect {
        x0,
        y0,
        x1,
        y1,
        mode: SelectionCombine::Replace,
    };
    let on_disk = |path: &std::path::Path| {
        let project = yolu_io::Project::read(&std::fs::read(path).unwrap()).unwrap();
        let id = project.sets()[0].id.clone();
        let read = project.saved_selections(&id).unwrap();
        let names: Vec<String> = read.items.iter().map(|i| i.name.clone()).collect();
        (
            project.info().format,
            names,
            read.skipped.len(),
            project.sets()[0].selection.is_some(),
        )
    };
    let dir = temp_dir("waiting-saved");
    let path = dir.join("gen.ylp");
    let mut s = cube();
    masked_fill(&mut s, Kind::EdgeWear);
    bake(&mut s);
    s.apply(Action::Sel(SelAction::Edit(rect(2, 2, 20, 20))));
    s.apply(Action::Sel(SelAction::Saved(SavedOp::Save("髪".into()))));
    s.apply(Action::Sel(SelAction::Edit(rect(30, 10, 60, 50))));
    s.apply(Action::Sel(SelAction::Saved(SavedOp::Save("服".into()))));
    let current = s.doc.selection().cloned().expect("今の選択範囲");
    let remembered: Vec<_> = s
        .saved_selections()
        .iter()
        .map(|x| (x.name.clone(), x.mask.clone()))
        .collect();
    assert_eq!(remembered.len(), 2);
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    assert_eq!(
        on_disk(&path),
        (8, vec!["髪".to_owned(), "服".to_owned()], 0, true)
    );

    // モデルの無い状態で開く: 読むだけ。保存しても、覚えた選択範囲と今の選択範囲はファイルに残る
    let mut again = AppState::new(64, 64);
    again.bake.backend = BakeBackend::Cpu;
    again.apply(Action::OpenProject(path.clone()));
    assert!(again.read_only_reason().is_some(), "{}", again.message);
    again.modified = true;
    again.apply(Action::SaveProject);
    assert!(
        again.message.starts_with("保存しました"),
        "{}",
        again.message
    );
    assert_eq!(
        on_disk(&path),
        (8, vec!["髪".to_owned(), "服".to_owned()], 0, true),
        "読むだけのセットの保存で消えない"
    );

    // 同じモデルを読むと入力がそろって編集できる: 覚えた選択範囲（名前・並び・中身）と今の選択範囲が戻り、理由の知らせは出ない
    again.apply(Action::LoadDemoModel);
    again.sync_effect_inputs_with(true);
    assert!(
        again.read_only_reason().is_none(),
        "{:?} {}",
        again.read_only_reason(),
        again.message
    );
    let restored: Vec<_> = again
        .saved_selections()
        .iter()
        .map(|x| (x.name.clone(), x.mask.clone()))
        .collect();
    assert_eq!(restored, remembered, "{}", again.message);
    assert_eq!(again.doc.selection(), Some(&current), "{}", again.message);
    assert!(!again.message.contains("読めない"), "{}", again.message);
    assert!(!again.doc.can_undo(), "戻しただけでは取り消しの段にしない");
    // 呼び戻しもできる
    again.apply(Action::Sel(SelAction::Edit(SelEdit::Recall {
        index: 0,
        mode: SelectionCombine::Replace,
    })));
    assert_eq!(again.doc.selection().map(|m| m.amount(5, 5)), Some(255));
    // 編集できるようになったあとの保存でも、エントリは残る（空の並びで置き換えない）。名前を変えれば書き換わる
    again.modified = true;
    again.apply(Action::SaveProject);
    assert!(
        again.message.starts_with("保存しました"),
        "{}",
        again.message
    );
    assert_eq!(on_disk(&path).1, ["髪", "服"], "{}", again.message);
    again.apply(Action::Sel(SelAction::Saved(SavedOp::Rename {
        index: 0,
        name: "前髪".into(),
    })));
    again.apply(Action::SaveProject);
    assert_eq!(
        on_disk(&path),
        (8, vec!["前髪".to_owned(), "服".to_owned()], 0, true)
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_generators_own_settings_problem_does_not_make_a_set_read_only() {
    // アンカーを選んでいない Anchor の Generator は、文書の中で直せる不備なので、編集できる
    let dir = temp_dir("anchorless");
    let path = dir.join("anchorless.ylp");
    let mut s = AppState::new(64, 64);
    let bottom = s.selected_layer.unwrap();
    s.apply(Action::NewLayer);
    let top = s.selected_layer.unwrap();
    add_generator(&mut s, FilterTarget::Content, Kind::Anchor);
    let id = s.doc.filters_of(top, FilterTarget::Content).unwrap()[0].id();
    assert!(s.doc.generator_inactive(top, id).unwrap().is_some());
    let _ = bottom;
    s.apply(Action::SaveProjectAs(path.clone()));
    let mut again = AppState::new(64, 64);
    again.apply(Action::OpenProject(path));
    assert!(
        again.read_only_reason().is_none(),
        "{:?}",
        again.read_only_reason()
    );
    assert_eq!(effect_counts(&again).0, 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_native_document_written_by_the_unity_version_opens_edits_and_saves() {
    // Unity 版の実際の書き手が作った効果入りの正本（フィルター・Anchor）を .ylp に入れて開く
    let fixtures =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../yolu-io/tests/fixtures");
    for name in ["effects-filters", "effects-anchors"] {
        let bytes = std::fs::read(fixtures.join(format!("{name}.utpaint"))).unwrap();
        let native = yolu_io::NativeDocument::read(&bytes).unwrap();
        let core = native.to_core().unwrap();
        let mut s = AppState::new(64, 64);
        let sets = yolu_app::sets::TextureSets::first_in(&core, Lang::Ja);
        s.replace_sets(sets, core);
        let imported = effect_counts(&s);
        assert!(imported.0 + imported.1 + imported.2 > 0, "{name}");
        let dir = temp_dir(name);
        let path = dir.join("unity.ylp");
        s.apply(Action::SaveProjectAs(path.clone()));
        assert!(
            s.message.starts_with("保存しました"),
            "{name}: {}",
            s.message
        );
        let mut again = AppState::new(64, 64);
        again.apply(Action::OpenProject(path.clone()));
        assert!(
            again.read_only_reason().is_none(),
            "{name}: {}",
            again.message
        );
        assert_eq!(effect_counts(&again), imported, "{name}");
        // 編集（段を足す・並べ替える・消す）して保存し、開き直しても効果が残る
        let layer = again.doc.layers().last().unwrap().id();
        again.selected_layer = Some(layer);
        add_filter(&mut again, FilterTarget::Content, FilterKind::Blur);
        let after_add = effect_counts(&again);
        assert_eq!(after_add.0, imported.0 + 1, "{name}: {}", again.message);
        again.apply(Action::SaveProject);
        let mut third = AppState::new(64, 64);
        third.apply(Action::OpenProject(path));
        assert_eq!(
            effect_counts(&third),
            after_add,
            "{name}: {}",
            third.message
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}

// ───────── 棚の画像 ─────────

const IMAGE_ID: &str = "0a0b0c0d-0000-4000-8000-000000000001";

/// 4 × 4 の画像を 1 枚持つ棚。
fn shelf_with_image() -> yolu_app::shelf::ShelfState {
    let mut shelf = yolu_io::shelf::Shelf::new(yolu_app::shelf::SHELF_BUDGET);
    let rgba: Vec<u8> = (0..16u8)
        .flat_map(|i| [i * 16, 255 - i * 16, 128, 255])
        .collect();
    shelf
        .add_image(IMAGE_ID, "image", &rgba, 4, 4, "srgb", Default::default())
        .unwrap();
    yolu_app::shelf::ShelfState::with_shelf(shelf)
}

#[test]
fn shelf_images_become_effect_inputs_when_used_and_follow_the_shelf() {
    let mut s = AppState::new(64, 64);
    s.shelf = shelf_with_image();
    s.apply(Action::M2(Edit::NewFill));
    let layer = s.selected_layer.unwrap();
    let plain = composite(&s);
    // 先に入力にしてから文書が指す（core は入力に無い画像を断る）
    assert!(s
        .doc
        .set_fill_image(layer, Channel::Color, Some(yolu_core::ImageId(1)))
        .is_err());
    let image = s.use_shelf_image(IMAGE_ID).unwrap();
    assert_eq!(image.0, 0x0a0b0c0d_0000_4000_8000_000000000001);
    s.doc
        .set_fill_image(layer, Channel::Color, Some(image))
        .unwrap();
    assert!(
        s.doc.inactive_effect_list().is_empty(),
        "{:?}",
        s.doc.inactive_effect_list()
    );
    let with_image = composite(&s);
    assert_ne!(with_image, plain, "画像が見える");
    // 棚から画像が無くなると、画像は効かない（値を見せる）。理由は画像が無いこと
    s.shelf = yolu_app::shelf::ShelfState::default();
    s.sync_effects();
    let inactive = s.doc.inactive_effect_list();
    assert_eq!(inactive.len(), 1, "{inactive:?}");
    assert!(yolu_app::fx::inputs::is_input_problem(&inactive[0]));
    assert_eq!(composite(&s), plain);
    // 棚に戻すと、文書が指している画像は頼まなくても入力になる
    s.shelf = shelf_with_image();
    s.sync_effects();
    assert!(s.doc.inactive_effect_list().is_empty());
    assert_eq!(composite(&s), with_image);
    // 棚に無い画像は断る
    assert!(s
        .use_shelf_image("ffffffff-0000-4000-8000-000000000002")
        .is_err());
}

#[test]
fn a_fill_image_comes_back_with_the_project_and_a_missing_one_makes_the_set_wait() {
    let dir = temp_dir("images");
    let path = dir.join("img.ylp");
    let mut s = AppState::new(64, 64);
    s.shelf = shelf_with_image();
    s.shelf.changed = true; // 棚を変えたので、保存で resources を書く
    s.apply(Action::M2(Edit::NewFill));
    let layer = s.selected_layer.unwrap();
    let image = s.use_shelf_image(IMAGE_ID).unwrap();
    s.doc
        .set_fill_image(layer, Channel::Color, Some(image))
        .unwrap();
    let shown = composite(&s);
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    // 開くと、画像は棚から入力になり、同じ絵になる（読むだけにならない）
    let mut again = AppState::new(64, 64);
    again.apply(Action::OpenProject(path));
    assert!(again.read_only_reason().is_none(), "{}", again.message);
    assert!(again.doc.inactive_effect_list().is_empty());
    assert_eq!(composite(&again), shown);

    // プロジェクトに無い画像を指す塗りつぶし: 読むだけにして、足りない画像を言う。棚に入れば編集できる
    let missing = dir.join("missing.ylp");
    let mut t = AppState::new(64, 64);
    t.apply(Action::M2(Edit::NewFill));
    let layer = t.selected_layer.unwrap();
    t.doc
        .set_fill_images_for_load(
            layer,
            &[(Channel::Color, image)],
            yolu_core::fill_image::Projection::default(),
        )
        .unwrap();
    t.apply(Action::SaveProjectAs(missing.clone()));
    assert!(t.message.starts_with("保存しました"), "{}", t.message);
    let mut opened = AppState::new(64, 64);
    opened.apply(Action::OpenProject(missing));
    let reason = opened.read_only_reason().expect("画像が無い").to_owned();
    assert!(reason.contains("効果の入力がそろっていない"), "{reason}");
    assert!(reason.contains("プロジェクトに画像が無い"), "{reason}");
    assert!(opened.sets.current().waiting_inputs);
    opened.shelf = shelf_with_image();
    opened.sync_effects();
    assert!(
        opened.read_only_reason().is_none(),
        "{:?}",
        opened.read_only_reason()
    );
    assert!(opened.doc.inactive_effect_list().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

const IMAGE_A: &str = "0a0b0c0d-0000-4000-8000-000000000001";
const IMAGE_B: &str = "0a0b0c0d-0000-4000-8000-000000000002";

/// 指定した（ID・幅・高さ・色の種）の画像を持つ棚。
fn shelf_with(images: &[(&str, u32, u32, u8)]) -> yolu_app::shelf::ShelfState {
    let mut shelf = yolu_io::shelf::Shelf::new(yolu_app::shelf::SHELF_BUDGET);
    for (id, w, h, seed) in images {
        let rgba: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                [
                    (i as u8).wrapping_mul(16).wrapping_add(*seed),
                    255 - seed,
                    128,
                    255,
                ]
            })
            .collect();
        shelf
            .add_image(
                id,
                &format!("image{seed}"),
                &rgba,
                *w,
                *h,
                "srgb",
                Default::default(),
            )
            .unwrap();
    }
    yolu_app::shelf::ShelfState::with_shelf(shelf)
}

/// 塗りつぶしレイヤーを足し、棚の画像（リソースの ID）を指させる（入力に入っているかは問わない）。
fn fill_pointing_at(s: &mut AppState, resource: &str) -> LayerId {
    s.apply(Action::M2(Edit::NewFill));
    let layer = s.selected_layer.unwrap();
    let id = yolu_app::fx::inputs::image_id(resource).unwrap();
    s.doc
        .set_fill_images_for_load(
            layer,
            &[(Channel::Color, id)],
            yolu_core::fill_image::Projection::default(),
        )
        .unwrap();
    layer
}

#[test]
fn decoded_images_share_one_budget_and_a_refusal_is_tried_again_only_when_it_could_change() {
    use yolu_app::fx::inputs::image_id;
    let (id_a, id_b) = (image_id(IMAGE_A).unwrap(), image_id(IMAGE_B).unwrap());
    // 4 × 4 の 2 枚（64 バイトずつ）。1 枚ずつ復号しても、持っている分と合わせて上限（100 バイト）を超える 2 枚目は断る
    let mut s = AppState::new(64, 64);
    s.fx.inputs.image_limit = Some(100);
    s.shelf = shelf_with(&[(IMAGE_A, 4, 4, 1), (IMAGE_B, 4, 4, 2)]);
    let first = fill_pointing_at(&mut s, IMAGE_A);
    fill_pointing_at(&mut s, IMAGE_B);
    s.sync_effects();
    assert_eq!(
        (
            s.fx.inputs.decoded_image_count(),
            s.fx.inputs.decoded_image_bytes()
        ),
        (1, 64)
    );
    let why =
        s.fx.inputs
            .image_error(id_b)
            .expect("2 枚目は断る")
            .to_owned();
    assert!(why.contains("予算"), "{why}");
    assert!(s.fx.inputs.image_error(id_a).is_none());
    // 同じ状態では試し直さず、理由も同じ
    s.sync_effects();
    assert_eq!(s.fx.inputs.decoded_image_count(), 1);
    assert_eq!(s.fx.inputs.image_error(id_b), Some(why.as_str()));
    // 持っている分が減る（1 枚目を指すレイヤーを消す）と、断っていた画像が通る
    s.selected_layer = Some(first);
    s.apply(Action::DeleteLayer);
    s.sync_effects();
    assert!(s.fx.inputs.image_error(id_b).is_none());
    assert_eq!(
        (
            s.fx.inputs.decoded_image_count(),
            s.fx.inputs.decoded_image_bytes()
        ),
        (1, 64),
        "1 枚目は手放した"
    );

    // 同じ ID の別の中身に替わると、試し直して通る
    let mut s = AppState::new(64, 64);
    s.fx.inputs.image_limit = Some(80);
    s.shelf = shelf_with(&[(IMAGE_A, 4, 4, 1), (IMAGE_B, 4, 4, 2)]);
    fill_pointing_at(&mut s, IMAGE_A);
    fill_pointing_at(&mut s, IMAGE_B);
    s.sync_effects();
    assert!(s.fx.inputs.image_error(id_b).is_some());
    s.shelf = shelf_with(&[(IMAGE_A, 4, 4, 1), (IMAGE_B, 1, 1, 3)]);
    s.sync_effects();
    assert!(s.fx.inputs.image_error(id_b).is_none());
    assert_eq!(
        (
            s.fx.inputs.decoded_image_count(),
            s.fx.inputs.decoded_image_bytes()
        ),
        (2, 68)
    );

    // 頼んだだけの画像は、文書が指して、指さなくなると手放す
    let mut s = AppState::new(64, 64);
    s.shelf = shelf_with(&[(IMAGE_A, 4, 4, 1)]);
    let image = s.use_shelf_image(IMAGE_A).unwrap();
    assert_eq!(s.fx.inputs.decoded_image_count(), 1);
    s.apply(Action::M2(Edit::NewFill));
    let layer = s.selected_layer.unwrap();
    s.doc
        .set_fill_image(layer, Channel::Color, Some(image))
        .unwrap();
    s.sync_effects();
    assert_eq!(
        s.fx.inputs.decoded_image_count(),
        1,
        "文書が指しているあいだは持つ"
    );
    s.apply(Action::DeleteLayer);
    s.sync_effects();
    assert_eq!(
        s.fx.inputs.decoded_image_count(),
        0,
        "指さなくなった画像は手放す"
    );
    // 断られた頼みは誰も持たない
    s.fx.inputs.image_limit = Some(10);
    assert!(s.use_shelf_image(IMAGE_A).is_err());
    s.sync_effects();
    assert_eq!(s.fx.inputs.decoded_image_count(), 0);
}

#[test]
fn an_image_that_cannot_be_decoded_says_why_in_the_read_only_reason_and_opens_once_it_can() {
    for lang in Lang::ALL {
        let dir = temp_dir(&format!("undecodable-{}", lang.pick("ja", "en")));
        let path = dir.join("two.ylp");
        let mut s = AppState::new(64, 64);
        s.shelf = shelf_with(&[(IMAGE_A, 4, 4, 1), (IMAGE_B, 4, 4, 2)]);
        s.shelf.changed = true;
        for resource in [IMAGE_A, IMAGE_B] {
            s.apply(Action::M2(Edit::NewFill));
            let layer = s.selected_layer.unwrap();
            let image = s.use_shelf_image(resource).unwrap();
            s.doc
                .set_fill_image(layer, Channel::Color, Some(image))
                .unwrap();
        }
        s.apply(Action::SaveProjectAs(path.clone()));
        assert!(s.message.starts_with("保存しました"), "{}", s.message);
        // 上限が小さくて 2 枚目を復号できない: 「プロジェクトに画像が無い」ではなく、読めない理由を言う
        let mut again = AppState::new(64, 64);
        again.lang = lang;
        again.fx.inputs.image_limit = Some(100);
        again.apply(Action::OpenProject(path));
        let reason = again
            .read_only_reason()
            .expect("2 枚目を読めない")
            .to_owned();
        assert!(
            reason.contains(lang.pick("画像を読めません", "Cannot read the image")),
            "{lang:?}: {reason}"
        );
        assert!(
            reason.contains(lang.pick("予算", "limit")),
            "{lang:?}: {reason}"
        );
        assert!(
            !reason.contains("プロジェクトに画像が無い") && !reason.contains("not in the project"),
            "{reason}"
        );
        assert!(again.sets.current().waiting_inputs);
        // 上限を上げると、読むだけを抜けて編集できる
        again.fx.inputs.image_limit = None;
        again.sync_effects();
        assert!(
            again.read_only_reason().is_none(),
            "{lang:?}: {:?} {}",
            again.read_only_reason(),
            again.message
        );
        assert!(again.doc.inactive_effect_list().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
