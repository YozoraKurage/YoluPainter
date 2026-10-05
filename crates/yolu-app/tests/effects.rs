//! 効果の層（フィルター・Generator・Anchor）の画面の操作（`Action::Fx`）と、効果の入力のつなぎ（焼いたメッシュマップ・モデル・画像）。
//! 画面なしで `AppState` を叩く: 各操作の結果と Undo 1 回、ロックでの断り、断った理由の日英、入力がそろうまでの理由と焼いた後に効くこと、
//! 焼き直しで読む層だけが描き直されること、保存復元（Unity 版が書いた効果入りの正本を開いて編集して保存）。

use yolu_app::bake::{BakeAction, BakeBackend};
use yolu_app::fx::{FilterKind, FxOp, Selected};
use yolu_app::lang::Lang;
use yolu_app::m2::{Edit, UiOp};
use yolu_app::state::{Action, AppState};
use yolu_core::generator::{self, Kind};
use yolu_core::mesh_maps::MeshMapKind;
use yolu_core::{
    AnchorPlacement, Channel, EffectSettings, FilterTarget, LayerId, LayerLocks,
};

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
        let before = s.doc.undo_count();
        add_filter(&mut s, FilterTarget::Content, kind);
        assert_eq!(filters(&s, layer, FilterTarget::Content), 1, "{kind:?}: {}", s.message);
        assert_eq!(s.doc.undo_count(), before + 1, "{kind:?}: 1 回の Undo");
        let stage = &s.doc.filters_of(layer, FilterTarget::Content).unwrap()[0];
        assert_eq!(stage.channels(), &[Channel::Color], "{kind:?}: 描くチャンネルだけに掛かる");
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
        assert_eq!(filters(&s, layer, FilterTarget::Content), 0, "{kind:?}: Undo 1 回で戻る");
        s.apply(Action::Redo);
        assert_eq!(filters(&s, layer, FilterTarget::Content), 1, "{kind:?}: Redo");
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
    // マスクの無い層のマスクへは足せない
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
    fx(&mut s, FxOp::Move { layer, id: ids[0], index: 1 });
    assert_eq!(s.doc.undo_count(), steps + 1);
    let after: Vec<_> = s.doc.filters_of(layer, FilterTarget::Content).unwrap().iter().map(|e| e.id()).collect();
    assert_eq!(after, vec![ids[1], ids[0]]);
    fx(&mut s, FxOp::SetEnabled { layer, id: ids[0], enabled: false });
    assert_eq!(s.doc.undo_count(), steps + 2);
    assert!(!s.doc.find_filter(ids[0]).unwrap().1.enabled());
    // 同じ値に直しても履歴は増えない
    fx(&mut s, FxOp::SetEnabled { layer, id: ids[0], enabled: false });
    assert_eq!(s.doc.undo_count(), steps + 2);
    fx(&mut s, FxOp::Remove { layer, id: ids[0] });
    assert_eq!(s.doc.undo_count(), steps + 3);
    assert_eq!(filters(&s, layer, FilterTarget::Content), 1);
    // 1 回ずつ逆の順に戻る: 削除 → 無効化 → 並べ替え（順序と有効を確かめる）
    let order = |s: &AppState| -> Vec<_> {
        s.doc.filters_of(layer, FilterTarget::Content).unwrap().iter().map(|e| (e.id(), e.enabled())).collect()
    };
    assert_eq!(order(&s), vec![(ids[1], true)]);
    s.apply(Action::Undo);
    assert_eq!(order(&s), vec![(ids[1], true), (ids[0], false)], "削除が戻る（無効のまま）");
    s.apply(Action::Undo);
    assert_eq!(order(&s), vec![(ids[1], true), (ids[0], true)], "無効化が戻る（順序はそのまま）");
    s.apply(Action::Undo);
    assert_eq!(order(&s), vec![(ids[0], true), (ids[1], true)], "並べ替えが戻る");
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
        fx(&mut s, FxOp::SetStrength { layer, id, strength, coalesce: true });
    }
    s.m2_end_drag();
    assert_eq!(s.doc.undo_count(), steps + 2);
    s.apply(Action::Undo);
    assert_eq!(s.doc.find_filter(id).unwrap().1.strength(), 1.0);
    s.apply(Action::Undo);
    assert_eq!(s.doc.find_filter(id).unwrap().1.settings(), &EffectSettings::blur(4));
    // 範囲の外の値は断る（何も変えない）
    let steps = s.doc.undo_count();
    fx(&mut s, FxOp::SetSettings { layer, id, settings: EffectSettings::blur(9999), coalesce: false });
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
        assert_eq!(s.message, lang.pick("層または親グループがロックされている", "Layer or parent group is locked"), "{lang:?}");
        for op in [
            FxOp::SetEnabled { layer, id, enabled: false },
            FxOp::Remove { layer, id },
            FxOp::SetStrength { layer, id, strength: 0.5, coalesce: false },
            FxOp::AddAnchor { layer, placement: AnchorPlacement::Layer },
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
    assert_eq!(filters(&s, layer, FilterTarget::Content), 1, "{}", s.message);
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
    fx(&mut s, FxOp::AddAnchor { layer: bottom, placement: AnchorPlacement::Layer });
    assert_eq!(s.doc.undo_count(), steps + 1, "{}", s.message);
    let anchor = s.doc.layer(bottom).unwrap().anchor().unwrap().clone();
    assert_eq!(anchor.name(), s.doc.layer(bottom).unwrap().name(), "名前の既定は層の名前");
    assert_eq!(s.fx.selected, Some(Selected::Anchor { id: anchor.id() }));
    // 同じ層にもう 1 つは置けない
    let steps = s.doc.undo_count();
    fx(&mut s, FxOp::AddAnchor { layer: bottom, placement: AnchorPlacement::Layer });
    assert_eq!(s.doc.undo_count(), steps);
    // 名前の変更
    fx(&mut s, FxOp::RenameAnchor { id: anchor.id(), name: "  下地  ".into() });
    assert_eq!(s.doc.layer(bottom).unwrap().anchor().unwrap().name(), "下地");
    fx(&mut s, FxOp::RenameAnchor { id: anchor.id(), name: "   ".into() });
    assert_eq!(s.doc.layer(bottom).unwrap().anchor().unwrap().name(), "下地", "空の名前は断る");
    // 上の層の Anchor の Generator は、すぐ下のアンカーを読む
    s.selected_layer = Some(top);
    add_generator(&mut s, FilterTarget::Content, Kind::Anchor);
    let stage = s.doc.filters_of(top, FilterTarget::Content).unwrap()[0].clone();
    let g = stage.settings().generator_settings().unwrap();
    assert_eq!(g.anchor.id, anchor.id().0, "すぐ下のアンカーを読む");
    assert_eq!(s.doc.generator_inactive(top, stage.id()).unwrap(), None, "{}", s.message);
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
    // 層を並べ替えて下になると読めない（知らせる。編集は断らない）
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
    fx(&mut s, FxOp::AddAnchor { layer, placement: AnchorPlacement::Mask });
    assert!(s.doc.layer(layer).unwrap().mask().is_none());
    s.apply(Action::M2(Edit::AddMask(layer)));
    fx(&mut s, FxOp::AddAnchor { layer, placement: AnchorPlacement::Mask });
    let mask_anchor = s.doc.layer(layer).unwrap().mask().unwrap().anchor().unwrap().clone();
    assert!(mask_anchor.name().contains("マスク"), "{}", mask_anchor.name());
    // 自分の層の Anchor は読めない: メニュー（ジェネレーター ▸）の項目は押せない。ラベルは名前だけで、理由はツールチップ
    s.lang = Lang::En;
    let entries = yolu_app::fx::menu::add_entries(&s, FilterTarget::Content);
    let anchor_entry = yolu_app::ui::menu::leaves(&entries)
        .into_iter()
        .find_map(|e| match e {
            yolu_app::ui::menu::Entry::Item { label, enabled, tooltip, .. } if label.starts_with("Anchor") => {
                Some((label.clone(), *enabled, tooltip.clone()))
            }
            _ => None,
        })
        .unwrap();
    assert!(!anchor_entry.1, "{}", anchor_entry.0);
    assert_eq!(anchor_entry.0, "Anchor", "ラベルに理由を続けない");
    assert!(anchor_entry.2.as_deref().is_some_and(|t| t.contains("no anchor below")), "{:?}", anchor_entry.2);
}

// ───────── メニュー ─────────

#[test]
fn the_add_menu_lists_every_kind_and_gives_a_reason_for_the_ones_a_channel_refuses() {
    use yolu_app::ui::menu::{leaves, Entry};
    for lang in Lang::ALL {
        let mut s = AppState::new(64, 64);
        s.lang = lang;
        let entries = yolu_app::fx::menu::add_entries(&s, FilterTarget::Content);
        // 並びは フィルター（平ら）→ 区切り → ジェネレーター ▸。見出し（「…の画素」・「Generator」）は置かない
        assert!(!entries.iter().any(|e| matches!(e, Entry::Heading(_))), "{lang:?}: 見出しを置かない");
        let separator = entries.iter().position(|e| matches!(e, Entry::Separator)).expect("区切り");
        assert_eq!(separator, 13, "{lang:?}: フィルターが 13 種、平らに並ぶ");
        assert!(entries[..separator].iter().all(|e| matches!(e, Entry::Item { .. })));
        match &entries[separator + 1..] {
            [Entry::Submenu { label, entries: generators, .. }] => {
                assert_eq!(label, lang.pick("ジェネレーター", "Generators"));
                assert_eq!(generators.len(), 10, "{lang:?}");
            }
            other => panic!("{lang:?}: 区切りの後はジェネレーターの入れ子だけ: {other:?}"),
        }
        let labels: Vec<(String, bool)> = leaves(&entries)
            .into_iter()
            .filter_map(|e| match e {
                Entry::Item { label, enabled, .. } => Some((label.clone(), *enabled)),
                _ => None,
            })
            .collect();
        assert_eq!(labels.len(), 13 + 10, "{lang:?}: {labels:?}");
        let layer = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::AddMask(layer)));
        for target in [FilterTarget::Content, FilterTarget::Mask] {
            let entries = yolu_app::fx::menu::add_entries(&s, target);
            for expected in [Kind::Noise, Kind::Grunge] {
                assert!(leaves(&entries).iter().any(|e| matches!(e,
                    Entry::Item { action: Action::Fx(FxOp::AddGenerator { kind, .. }), enabled: true, .. } if *kind == expected
                )));
            }
        }

        // 法線のチャンネル: ぼかし以外は押せない。理由はラベルに続けず、ツールチップに置く
        s.apply(Action::M2Ui(UiOp::PaintChannel(Channel::Normal)));
        let entries = yolu_app::fx::menu::add_entries(&s, FilterTarget::Content);
        let disabled: Vec<(&String, &Option<String>)> = leaves(&entries)
            .into_iter()
            .filter_map(|e| match e {
                Entry::Item { label, enabled: false, tooltip, .. } => Some((label, tooltip)),
                _ => None,
            })
            .collect();
        assert!(disabled.len() >= 6, "{lang:?}: {disabled:?}");
        assert!(disabled.iter().all(|(l, _)| !l.contains(" — ")), "{lang:?}: ラベルに理由を続けない {disabled:?}");
        assert!(
            disabled.iter().all(|(_, t)| t.as_deref().is_some_and(|t| !t.is_empty())),
            "{lang:?}: 理由はツールチップに {disabled:?}"
        );
        if lang == Lang::En {
            assert!(
                disabled.iter().all(|(l, t)| !l.chars().chain(t.iter().flat_map(|t| t.chars())).any(|c| matches!(c, '\u{3040}'..='\u{9fff}'))),
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
    assert!(entries.len() > 15);
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

/// 黒い塗りつぶしの層にマスクを付け、そのマスクへ Generator を足す。
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
    let why = s.doc.generator_inactive(layer, id).unwrap().expect("マップが無い");
    assert!(s.lang.inactive_reason(&why).contains("Curvature"), "{why:?}");
    assert_eq!(Lang::En.inactive_reason(&why), "No Curvature map");
    assert!(s.message.contains("効果なし"), "足したときの知らせに理由を添える: {}", s.message);
    let before = composite(&s);
    assert!(!s.doc.inactive_effect_list().is_empty());
    // 焼いた後: 効く（読むマップがそろい、合成が変わる）
    bake(&mut s);
    assert_eq!(s.doc.generator_inactive(layer, id).unwrap(), None, "{}", s.message);
    assert!(s.doc.inactive_effect_list().is_empty());
    assert_ne!(composite(&s), before, "焼いたマップで合成が変わる");
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
        matches!(why, yolu_core::InactiveReason::Generator(generator::Inactive::StaleMap(_))),
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
    assert_eq!(redrawn, 1, "マップを読む層の 1 ブロックだけ（ぼかしの層は評価し直さない）");
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
    let why = s.doc.generator_inactive(layer, id).unwrap().expect("ID の色が無い");
    assert!(matches!(why, yolu_core::InactiveReason::Generator(generator::Inactive::NoIdColors)), "{why:?}");
    // 選ぶのを始めると、「ID の色で選択」の道具の入力を使う（効果の欄は開いたまま）
    fx(&mut s, FxOp::PickIdColors { layer, id, on: true });
    assert_eq!(s.tool, yolu_app::state::Tool::IdSelect);
    assert_eq!(s.fx.id_pick, Some((layer, id)));
    assert!(s.fx.selected.is_some(), "効果の欄は閉じない");
    // 焼いた ID マップの、どこかの画素の色
    let map = s.sets.current().mesh_maps.get(MeshMapKind::Id).unwrap().clone();
    let rgb = (0..64)
        .flat_map(|y| (0..64).map(move |x| (x, y)))
        .find_map(|(x, y)| yolu_core::id_colors::try_get(&map, x, y).ok().flatten())
        .expect("ID の色のある画素");
    let steps = s.doc.undo_count();
    assert!(s.pick_id_color(rgb));
    assert_eq!(s.doc.undo_count(), steps + 1, "足すのは 1 回の Undo");
    let colors = |s: &AppState| s.doc.find_filter(id).unwrap().1.settings().generator_settings().unwrap().id_colors.clone();
    assert_eq!(colors(&s), vec![rgb]);
    assert_eq!(s.doc.generator_inactive(layer, id).unwrap(), None, "色が選ばれたので効く");
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
    assert!(s.message.contains("is not in the ID colors"), "{}", s.message);
    s.lang = Lang::Ja;
    s.region.modifiers.command = false;
    s.apply(Action::Undo);
    assert_eq!(colors(&s), vec![rgb]);
    // 道具を替えると選ぶのをやめ、押しても選択の道具として働く
    s.apply(Action::SelectTool(yolu_app::state::Tool::Brush));
    assert_eq!(s.fx.id_pick, None);
    assert!(!s.pick_id_color(rgb));
}

/// 焼いた ID マップの、色のある画素の座標と色。
fn id_pixel(s: &AppState) -> ((i64, i64), u32) {
    let map = s.sets.current().mesh_maps.get(MeshMapKind::Id).unwrap().clone();
    (0..64)
        .flat_map(|y| (0..64).map(move |x| (x, y)))
        .find_map(|(x, y)| yolu_core::id_colors::try_get(&map, x, y).ok().flatten().map(|rgb| ((x, y), rgb)))
        .expect("ID の色のある画素")
}

#[test]
fn id_colors_come_in_through_the_id_select_press_without_making_a_selection() {
    use yolu_app::state::{StrokeSource, Tool};
    let mut s = cube();
    s.bake.settings.maps = vec![MeshMapKind::Id, MeshMapKind::Position];
    let (layer, id) = masked_fill(&mut s, Kind::IdColor);
    bake(&mut s);
    fx(&mut s, FxOp::PickIdColors { layer, id, on: true });
    assert_eq!(s.tool, Tool::IdSelect);
    let ((x, y), rgb) = id_pixel(&s);
    let colors = |s: &AppState| s.doc.find_filter(id).unwrap().1.settings().generator_settings().unwrap().id_colors.clone();
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
    // 選ぶのをやめると、同じ押下は選択の道具として働く（選択範囲ができ、ID の色は変わらない）
    fx(&mut s, FxOp::PickIdColors { layer, id, on: false });
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
    // パスの道具で点を選び、スライダーの途中の値がある
    s.apply(Action::SelectTool(Tool::Path));
    s.path.selected = Some(PointRef { layer, path: 1, index: 0 });
    s.path.pending = Some(("width", 3.0));
    s.select_effect(layer, id);
    fx(&mut s, FxOp::PickIdColors { layer, id, on: true });
    assert_eq!(s.tool, Tool::IdSelect);
    assert!(s.path.selected.is_none() && s.path.pending.is_none() && s.path.drag.is_none(), "パスの途中の状態を捨てる");
    assert_eq!(s.fx.id_pick, Some((layer, id)));
    assert_eq!(s.fx.selected, Some(Selected::Filter { layer, id }), "効果の欄は開いたまま");
    // 同じ道具のままもう一度押しても同じ
    fx(&mut s, FxOp::PickIdColors { layer, id, on: true });
    assert_eq!(s.fx.id_pick, Some((layer, id)));
    // 描いている間は道具を替えない（断って、何も変えない）
    fx(&mut s, FxOp::PickIdColors { layer, id, on: false });
    s.apply(Action::SelectTool(Tool::Brush));
    let base = s.doc.layers()[0].id();
    let brush = s.stroke_settings(false);
    let stroke = s.doc.begin_stroke(base, &brush).unwrap();
    s.message.clear();
    fx(&mut s, FxOp::PickIdColors { layer, id, on: true });
    assert_eq!(s.message, "描いている間はできません。");
    assert_eq!(s.tool, Tool::Brush);
    assert_eq!(s.fx.id_pick, None);
    // 選ぶのをやめる操作は描いている間でも通る
    fx(&mut s, FxOp::PickIdColors { layer, id, on: false });
    s.doc.cancel_stroke(stroke);
    fx(&mut s, FxOp::PickIdColors { layer, id, on: true });
    assert_eq!(s.tool, Tool::IdSelect);
}

// ───────── 保存復元 ─────────

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("yolu-fx-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
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
    fx(&mut s, FxOp::AddAnchor { layer: base, placement: AnchorPlacement::Layer });
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
    assert!(again.message.starts_with("保存しました"), "{}", again.message);
    let mut third = AppState::new(64, 64);
    third.apply(Action::OpenProject(path));
    assert_eq!(effect_counts(&third), (3, 1, 1), "{}", third.message);
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
    let reason = again.read_only_reason().expect("入力がそろわない").to_owned();
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
    let (a, b) = (yolu_io::Project::read(&before).unwrap(), yolu_io::Project::read(&after).unwrap());
    assert_eq!(
        a.sets()[0].document.to_bytes().unwrap(),
        b.sets()[0].document.to_bytes().unwrap(),
        "読むだけのセットの正本は書き換えない"
    );

    // 同じモデルを読むと、マップが照合できて、入力がそろい、同じセットが編集できる
    again.apply(Action::LoadDemoModel);
    again.sync_effect_inputs_with(true); // 画面は毎フレーム見る。試験はモデルの入力を作り終えるまで待つ
    assert!(again.read_only_reason().is_none(), "{:?} {}", again.read_only_reason(), again.message);
    assert!(!again.sets.current().waiting_inputs);
    assert_eq!(effect_counts(&again), expected);
    assert!(again.doc.inactive_effect_list().is_empty(), "効果が効く");
    let steps = again.doc.undo_count();
    again.selected_layer = Some(again.doc.layers().last().unwrap().id());
    add_filter(&mut again, FilterTarget::Content, FilterKind::Invert);
    assert_eq!(again.doc.undo_count(), steps + 1, "{}", again.message);
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
    assert!(again.read_only_reason().is_none(), "{:?}", again.read_only_reason());
    assert_eq!(effect_counts(&again).0, 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_native_document_written_by_the_unity_version_opens_edits_and_saves() {
    // Unity 版の実際の書き手が作った効果入りの正本（フィルター・Anchor）を .ylp に入れて開く
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../yolu-io/tests/fixtures");
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
        assert!(s.message.starts_with("保存しました"), "{name}: {}", s.message);
        let mut again = AppState::new(64, 64);
        again.apply(Action::OpenProject(path.clone()));
        assert!(again.read_only_reason().is_none(), "{name}: {}", again.message);
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
        assert_eq!(effect_counts(&third), after_add, "{name}: {}", third.message);
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
    assert!(s.doc.set_fill_image(layer, Channel::Color, Some(yolu_core::ImageId(1))).is_err());
    let image = s.use_shelf_image(IMAGE_ID).unwrap();
    assert_eq!(image.0, 0x0a0b0c0d_0000_4000_8000_000000000001);
    s.doc.set_fill_image(layer, Channel::Color, Some(image)).unwrap();
    assert!(s.doc.inactive_effect_list().is_empty(), "{:?}", s.doc.inactive_effect_list());
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
    assert!(s.use_shelf_image("ffffffff-0000-4000-8000-000000000002").is_err());
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
    s.doc.set_fill_image(layer, Channel::Color, Some(image)).unwrap();
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
        .set_fill_images_for_load(layer, &[(Channel::Color, image)], yolu_core::fill_image::Projection::default())
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
    assert!(opened.read_only_reason().is_none(), "{:?}", opened.read_only_reason());
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
            .flat_map(|i| [(i as u8).wrapping_mul(16).wrapping_add(*seed), 255 - seed, 128, 255])
            .collect();
        shelf
            .add_image(id, &format!("image{seed}"), &rgba, *w, *h, "srgb", Default::default())
            .unwrap();
    }
    yolu_app::shelf::ShelfState::with_shelf(shelf)
}

/// 塗りつぶしの層を足し、棚の画像（リソースの ID）を指させる（入力に入っているかは問わない）。
fn fill_pointing_at(s: &mut AppState, resource: &str) -> LayerId {
    s.apply(Action::M2(Edit::NewFill));
    let layer = s.selected_layer.unwrap();
    let id = yolu_app::fx::inputs::image_id(resource).unwrap();
    s.doc
        .set_fill_images_for_load(layer, &[(Channel::Color, id)], yolu_core::fill_image::Projection::default())
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
    assert_eq!((s.fx.inputs.decoded_image_count(), s.fx.inputs.decoded_image_bytes()), (1, 64));
    let why = s.fx.inputs.image_error(id_b).expect("2 枚目は断る").to_owned();
    assert!(why.contains("予算"), "{why}");
    assert!(s.fx.inputs.image_error(id_a).is_none());
    // 同じ状態では試し直さず、理由も同じ
    s.sync_effects();
    assert_eq!(s.fx.inputs.decoded_image_count(), 1);
    assert_eq!(s.fx.inputs.image_error(id_b), Some(why.as_str()));
    // 持っている分が減る（1 枚目を指す層を消す）と、断っていた画像が通る
    s.selected_layer = Some(first);
    s.apply(Action::DeleteLayer);
    s.sync_effects();
    assert!(s.fx.inputs.image_error(id_b).is_none());
    assert_eq!((s.fx.inputs.decoded_image_count(), s.fx.inputs.decoded_image_bytes()), (1, 64), "1 枚目は手放した");

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
    assert_eq!((s.fx.inputs.decoded_image_count(), s.fx.inputs.decoded_image_bytes()), (2, 68));

    // 頼んだだけの画像は、文書が指して、指さなくなると手放す
    let mut s = AppState::new(64, 64);
    s.shelf = shelf_with(&[(IMAGE_A, 4, 4, 1)]);
    let image = s.use_shelf_image(IMAGE_A).unwrap();
    assert_eq!(s.fx.inputs.decoded_image_count(), 1);
    s.apply(Action::M2(Edit::NewFill));
    let layer = s.selected_layer.unwrap();
    s.doc.set_fill_image(layer, Channel::Color, Some(image)).unwrap();
    s.sync_effects();
    assert_eq!(s.fx.inputs.decoded_image_count(), 1, "文書が指しているあいだは持つ");
    s.apply(Action::DeleteLayer);
    s.sync_effects();
    assert_eq!(s.fx.inputs.decoded_image_count(), 0, "指さなくなった画像は手放す");
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
            s.doc.set_fill_image(layer, Channel::Color, Some(image)).unwrap();
        }
        s.apply(Action::SaveProjectAs(path.clone()));
        assert!(s.message.starts_with("保存しました"), "{}", s.message);
        // 上限が小さくて 2 枚目を復号できない: 「プロジェクトに画像が無い」ではなく、読めない理由を言う
        let mut again = AppState::new(64, 64);
        again.lang = lang;
        again.fx.inputs.image_limit = Some(100);
        again.apply(Action::OpenProject(path));
        let reason = again.read_only_reason().expect("2 枚目を読めない").to_owned();
        assert!(reason.contains(lang.pick("画像を読めません", "Cannot read the image")), "{lang:?}: {reason}");
        assert!(reason.contains(lang.pick("予算", "limit")), "{lang:?}: {reason}");
        assert!(!reason.contains("プロジェクトに画像が無い") && !reason.contains("not in the project"), "{reason}");
        assert!(again.sets.current().waiting_inputs);
        // 上限を上げると、読むだけを抜けて編集できる
        again.fx.inputs.image_limit = None;
        again.sync_effects();
        assert!(again.read_only_reason().is_none(), "{lang:?}: {:?} {}", again.read_only_reason(), again.message);
        assert!(again.doc.inactive_effect_list().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
