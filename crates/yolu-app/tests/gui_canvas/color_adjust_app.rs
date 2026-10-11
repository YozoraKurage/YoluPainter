//! 色調補正の 6 種のアプリの中でのつなぎ: 調整レイヤーとフィルターの段として足し、選ぶと欄が出て、操作が 1 回の Undo になり、
//! 使えないチャンネルは理由つきで断る。欄そのものの操作・日英・スナップショットは `color_adjust.rs`。
use crate::common;

use common::*;
use egui::{pos2, Rect};
use egui_kittest::Harness;
use yolu_app::fx::{FilterKind, FxOp};
use yolu_app::lang::Lang;
use yolu_app::m2::{AdjustmentKind, Edit};
use yolu_app::m2_menu::Popup;
use yolu_app::state::Action;
use yolu_app::ui::menu::Entry;
use yolu_app::YoluApp;
use yolu_core::{
    AdjustmentType, Channel, ChannelKind, ColorAdjust, EffectSettings, FilterTarget, LayerId,
    ToneChannel,
};

const SIX: [AdjustmentKind; 6] = [
    AdjustmentKind::GradientMap,
    AdjustmentKind::ToneCurve,
    AdjustmentKind::ColorBalance,
    AdjustmentKind::BrightnessContrast,
    AdjustmentKind::Threshold,
    AdjustmentKind::Posterize,
];

fn apply(h: &mut Harness<'_, YoluApp>, action: Action) {
    h.state_mut().state.apply(action);
    h.run();
}

fn new_adjustment(h: &mut Harness<'_, YoluApp>, kind: AdjustmentKind) -> LayerId {
    apply(h, Action::M2(Edit::NewAdjustment(kind)));
    // 狭い欄は、欄の中身の高さが分かった次のフレームで並びが変わる（スクロールバーの分）。そのあとで部品の位置を読む
    h.step();
    h.step();
    h.state()
        .state
        .selected_layer
        .expect("足したレイヤーを選ぶ")
}

fn value_of(h: &Harness<'_, YoluApp>, id: LayerId) -> ColorAdjust {
    h.state()
        .state
        .doc
        .layer(id)
        .unwrap()
        .adjustment()
        .and_then(|a| a.color_adjust())
        .expect("色調補正のレイヤー")
}

fn undo_count(h: &Harness<'_, YoluApp>) -> usize {
    h.state().state.doc.undo_count()
}

/// 右の列（プロパティの欄）の中で、スライダーの溝の `fraction` の位置を押す。
fn click_panel_slider(h: &mut Harness<'_, YoluApp>, label: &str, fraction: f32) {
    let r = rect_of(h, label, |r| r.left() > rx());
    let width = (r.width()).max(120.0);
    let y = r.center().y + 8.0;
    click(h, pos2(r.left() + width * fraction, y));
}

#[test]
fn every_kind_is_a_new_adjustment_layer_acting_on_the_channels_it_can() {
    for kind in SIX {
        let mut h = app(1280.0, 1400.0, 64);
        let before = undo_count(&h);
        let id = new_adjustment(&mut h, kind);
        assert_eq!(undo_count(&h), before + 1, "{kind:?}: 1 回の Undo");
        let layer = h.state().state.doc.layer(id).unwrap();
        let settings = layer.adjustment().unwrap();
        assert_eq!(AdjustmentKind::of(settings), kind);
        assert_eq!(settings.kind(), kind.adjustment_type());
        let enabled = layer.enabled_channels();
        assert!(enabled.contains(&Channel::Color), "{kind:?}");
        assert!(
            !enabled.contains(&Channel::Normal),
            "{kind:?}: 法線には効かない"
        );
        let scalar = enabled.contains(&Channel::Roughness);
        assert_eq!(
            scalar,
            settings.applies_to(ChannelKind::Scalar),
            "{kind:?}: 色だけの種類はスカラーに効かない"
        );
        // 元に戻せる
        apply(&mut h, Action::Undo);
        assert!(h.state().state.doc.layer(id).is_none(), "{kind:?}");
        assert_eq!(undo_count(&h), before);
    }
}

#[test]
fn the_new_adjustment_menus_list_every_kind_in_both_languages() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 1400.0, 64);
        h.state_mut().state.set_language(lang);
        h.run();
        let entries = yolu_app::m2_menu::entries(&h.state().state, Popup::NewAdjustment);
        let labels: Vec<String> = entries
            .iter()
            .filter_map(|e| match e {
                Entry::Item { label, .. } => Some(label.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(labels.len(), AdjustmentKind::ALL.len());
        for kind in AdjustmentKind::ALL {
            assert!(
                labels.iter().any(|l| l == kind.name(lang)),
                "{kind:?} {labels:?}"
            );
        }
        if lang == Lang::En {
            assert!(labels.iter().all(|l| !has_japanese(l)), "{labels:?}");
        }
    }
}

#[test]
fn a_threshold_slider_drag_in_the_panel_is_one_undo_step_and_undo_restores_it() {
    let mut h = app(1280.0, 1400.0, 64);
    let id = new_adjustment(&mut h, AdjustmentKind::Threshold);
    let first = value_of(&h, id);
    let steps = undo_count(&h);
    let r = rect_of(&h, "しきい値", |r| r.left() > rx());
    let y = r.center().y + 8.0;
    drag(
        &mut h,
        &[
            pos2(r.left() + 30.0, y),
            pos2(r.left() + 60.0, y),
            pos2(r.left() + 90.0, y),
        ],
    );
    assert_ne!(value_of(&h, id), first, "つまみでしきい値が変わる");
    assert_eq!(undo_count(&h), steps + 1, "ドラッグは 1 回の Undo");
    apply(&mut h, Action::Undo);
    assert_eq!(value_of(&h, id), first);
}

#[test]
fn brightness_contrast_posterize_and_color_balance_edits_are_undoable() {
    // 高いウィンドウ（カラーバランスの欄の下の行まで、ドックの内側に収める）
    let mut h = app(1280.0, 1300.0, 64);
    let id = new_adjustment(&mut h, AdjustmentKind::BrightnessContrast);
    let steps = undo_count(&h);
    click_panel_slider(&mut h, "明るさ", 0.9);
    click_panel_slider(&mut h, "コントラスト", 0.8);
    let ColorAdjust::BrightnessContrast(v) = value_of(&h, id) else {
        panic!()
    };
    assert!(v.brightness() > 100.0 && v.contrast() > 50.0, "{v:?}");
    assert_eq!(undo_count(&h), steps + 2, "別々の操作は別の Undo");
    apply(&mut h, Action::Undo);
    apply(&mut h, Action::Undo);
    let ColorAdjust::BrightnessContrast(v) = value_of(&h, id) else {
        panic!()
    };
    assert_eq!((v.brightness(), v.contrast()), (0.0, 0.0));

    let id = new_adjustment(&mut h, AdjustmentKind::Posterize);
    click_panel_slider(&mut h, "階調", 0.0);
    let ColorAdjust::Posterize(v) = value_of(&h, id) else {
        panic!()
    };
    assert_eq!(v.levels(), 2);

    let id = new_adjustment(&mut h, AdjustmentKind::ColorBalance);
    let steps = undo_count(&h);
    click_label_in_panel(&mut h, "シャドウ");
    click_panel_slider(&mut h, "マゼンタ — グリーン", 1.0);
    scroll_panel_to(&mut h, "輝度を保つ");
    click_check_in_panel(&mut h, "輝度を保つ");
    let ColorAdjust::ColorBalance(v) = value_of(&h, id) else {
        panic!()
    };
    assert_eq!(v.values(yolu_core::BalanceRange::Shadows)[1], 100.0);
    assert!(!v.preserve_luminosity());
    assert_eq!(undo_count(&h), steps + 2, "スライダーと切り替えは別の Undo");
}

/// 右の列を、その名前の部品が見える所（ウィンドウの中ほど）まで送る（欄は縦に長く、ウィンドウの外にはみ出す）。
fn scroll_panel_to(h: &mut Harness<'_, YoluApp>, label: &str) {
    let r = rect_of(h, label, |r| r.left() > rx());
    let scroll = h.state().state.m2.props_scroll + (r.top() - props_mid(h));
    h.state_mut().state.m2.props_scroll = scroll.max(0.0);
    h.run();
}

fn click_label_in_panel(h: &mut Harness<'_, YoluApp>, label: &str) {
    let r = rect_of(h, label, |r| r.left() > rx());
    click(h, r.center());
}

fn click_check_in_panel(h: &mut Harness<'_, YoluApp>, label: &str) {
    let r = rect_of(h, label, |r| r.left() > rx());
    click(h, pos2(r.left() + 8.0, r.center().y));
}

#[test]
fn the_gradient_map_panel_applies_a_preset_flips_and_undoes() {
    let mut h = app(1280.0, 2000.0, 64);
    let id = new_adjustment(&mut h, AdjustmentKind::GradientMap);
    let first = value_of(&h, id);
    let steps = undo_count(&h);
    // グラデーションセット: 組を替えて見本を押す
    click_label_in_panel(&mut h, "色味");
    click_label_in_panel(&mut h, "セピア");
    assert_ne!(value_of(&h, id), first);
    click_check_in_panel(&mut h, "逆向き");
    let ColorAdjust::GradientMap(g) = value_of(&h, id) else {
        panic!()
    };
    assert!(g.reverse());
    assert_eq!(undo_count(&h), steps + 2);
    apply(&mut h, Action::Undo);
    apply(&mut h, Action::Undo);
    assert_eq!(value_of(&h, id), first);
    // 見た目: 黒→白のランプの逆向きは階調の反転になる
    apply(&mut h, Action::Redo);
    apply(&mut h, Action::Redo);
}

#[test]
fn the_tone_curve_panel_edits_a_curve_with_one_undo_and_the_histogram_follows_what_lies_below() {
    let mut h = app(1280.0, 1400.0, 64);
    // 下に絵のある調整レイヤー（分布が出る）
    apply(&mut h, Action::NewLayer);
    let paint = h.state().state.selected_layer.unwrap();
    for y in 0..64 {
        for x in 0..64 {
            h.state_mut()
                .state
                .doc
                .set_pixel(
                    paint,
                    x,
                    y,
                    yolu_core::Rgba8::new((x * 4) as u8, 90, (y * 4) as u8, 255),
                )
                .unwrap();
        }
    }
    let id = new_adjustment(&mut h, AdjustmentKind::ToneCurve);
    let steps = undo_count(&h);
    let strip = rect_of(&h, "RGB", |r| r.left() > rx());
    let blue = rect_of(&h, "B", |r| r.left() > rx());
    let editor = Rect::from_min_max(
        pos2(strip.left(), strip.bottom() + 4.0),
        pos2(blue.right(), strip.bottom() + 4.0 + 116.0),
    );
    let inner = editor.shrink(6.0);
    let at = |x: f32, y: f32| {
        pos2(
            inner.left() + inner.width() * x,
            inner.bottom() - inner.height() * y,
        )
    };
    drag(&mut h, &[at(0.5, 0.5), at(0.5, 0.6), at(0.5, 0.75)]);
    let ColorAdjust::ToneCurve(v) = value_of(&h, id) else {
        panic!()
    };
    assert_eq!(v.curve(ToneChannel::Composite).points().len(), 3);
    assert_eq!(undo_count(&h), steps + 1, "曲線の編集は離したとき 1 回");
    apply(&mut h, Action::Undo);
    let ColorAdjust::ToneCurve(v) = value_of(&h, id) else {
        panic!()
    };
    assert!(v.is_identity());
    // レイヤーの下の分布は、文書の合成の下半分（調整レイヤーより下）から求め、文書が変わるまで同じ
    let doc = &h.state().state.doc;
    let hist = yolu_app::panels::color_adjust::Histogram::below(doc, id, Channel::Color).unwrap();
    assert!(
        hist.bins(ToneChannel::Red)
            .iter()
            .filter(|v| **v > 0.0)
            .count()
            > 20
    );
    assert_eq!(
        hist.bins(ToneChannel::Green)
            .iter()
            .filter(|v| **v > 0.0)
            .count(),
        1,
        "緑は一定の 90"
    );
}

fn fill_layer(h: &mut Harness<'_, YoluApp>) -> LayerId {
    apply(h, Action::M2(Edit::NewFill));
    h.state().state.selected_layer.unwrap()
}

const FILTER_SIX: [FilterKind; 6] = [
    FilterKind::GradientMap,
    FilterKind::ToneCurve,
    FilterKind::ColorBalance,
    FilterKind::BrightnessContrast,
    FilterKind::Threshold,
    FilterKind::Posterize,
];

#[test]
fn filter_stages_of_the_six_kinds_are_added_selected_edited_and_undone() {
    for kind in FILTER_SIX {
        let mut h = app(1280.0, 1500.0, 64);
        let layer = fill_layer(&mut h);
        let steps = undo_count(&h);
        apply(
            &mut h,
            Action::Fx(FxOp::AddFilter {
                target: FilterTarget::Content,
                kind,
            }),
        );
        assert_eq!(undo_count(&h), steps + 1, "{kind:?}");
        let stages = h
            .state()
            .state
            .doc
            .filters_of(layer, FilterTarget::Content)
            .unwrap();
        assert_eq!(stages.len(), 1);
        let id = stages[0].id();
        let first = stages[0].settings().clone();
        let adjust = first.color_adjust().expect("色調補正の段");
        assert_eq!(
            adjust.kind(),
            match kind {
                FilterKind::GradientMap => AdjustmentType::GradientMap,
                FilterKind::ToneCurve => AdjustmentType::ToneCurve,
                FilterKind::ColorBalance => AdjustmentType::ColorBalance,
                FilterKind::BrightnessContrast => AdjustmentType::BrightnessContrast,
                FilterKind::Threshold => AdjustmentType::Threshold,
                _ => AdjustmentType::Posterize,
            }
        );
        // 段を選ぶと欄が出る（調整レイヤーと同じ欄）
        apply(&mut h, Action::Fx(FxOp::SelectFilter { layer, id }));
        let probe = match kind {
            FilterKind::GradientMap => "逆向き",
            FilterKind::ToneCurve => "RGB",
            FilterKind::ColorBalance => "輝度を保つ",
            FilterKind::BrightnessContrast => "明るさ",
            FilterKind::Threshold => "しきい値",
            _ => "階調",
        };
        rect_of(&h, probe, |r| r.left() > rx());
        // 値を 1 つ替える（段の設定が変わり、元に戻せる）
        match kind {
            FilterKind::GradientMap => {
                click_label_in_panel(&mut h, "色味");
                click_label_in_panel(&mut h, "セピア");
            }
            FilterKind::ToneCurve => click_label_in_panel(&mut h, "S 字"),
            FilterKind::ColorBalance => click_panel_slider(&mut h, "シアン — レッド", 1.0),
            FilterKind::BrightnessContrast => click_panel_slider(&mut h, "明るさ", 1.0),
            FilterKind::Threshold => click_panel_slider(&mut h, "しきい値", 0.1),
            _ => click_panel_slider(&mut h, "階調", 0.0),
        }
        let now = h
            .state()
            .state
            .doc
            .find_filter(id)
            .unwrap()
            .1
            .settings()
            .clone();
        assert_ne!(now, first, "{kind:?}: 値が替わる");
        assert!(matches!(now, EffectSettings::Filter(_)));
        assert_eq!(
            undo_count(&h),
            steps + 2,
            "{kind:?}: 1 回の操作は 1 回の Undo"
        );
        apply(&mut h, Action::Undo);
        let restored = h
            .state()
            .state
            .doc
            .find_filter(id)
            .unwrap()
            .1
            .settings()
            .clone();
        assert_eq!(restored, first, "{kind:?}");
    }
}

#[test]
fn the_add_filter_menu_names_the_six_and_refuses_them_where_they_cannot_apply() {
    let mut h = app(1280.0, 1400.0, 64);
    let _ = fill_layer(&mut h);
    // (ラベル, 押せるか, 理由)。理由はラベルに続けず、ツールチップに置く
    let label_of = |h: &Harness<'_, YoluApp>, kind: FilterKind| -> (String, bool, Option<String>) {
        let lang = h.state().state.lang;
        let entries =
            yolu_app::fx::menu::add_filter_entries(&h.state().state, FilterTarget::Content);
        yolu_app::ui::menu::leaves(&entries)
            .into_iter()
            .find_map(|e| match e {
                Entry::Item {
                    label,
                    enabled,
                    tooltip,
                    ..
                } if label.starts_with(kind.name(lang)) => {
                    Some((label.clone(), *enabled, tooltip.clone()))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{kind:?}"))
    };
    // Color: 6 つとも足せる
    h.state_mut().state.m2.paint_channel = Channel::Color;
    for kind in FILTER_SIX {
        assert!(label_of(&h, kind).1, "{kind:?}");
    }
    // スカラー（Height）: 色だけの 2 つは理由つきで断り、ほかは足せる
    h.state_mut().state.m2.paint_channel = Channel::Height;
    for kind in FILTER_SIX {
        let (label, enabled, reason) = label_of(&h, kind);
        let colour_only = matches!(kind, FilterKind::GradientMap | FilterKind::ColorBalance);
        assert_eq!(enabled, !colour_only, "{kind:?} {label}");
        assert_eq!(reason.is_some(), colour_only, "{label}");
        assert!(!label.contains(" — "), "ラベルに理由を続けない: {label}");
    }
    // 法線: 6 つとも断る
    h.state_mut().state.m2.paint_channel = Channel::Normal;
    for kind in FILTER_SIX {
        let (label, enabled, reason) = label_of(&h, kind);
        assert!(
            !enabled && reason.is_some() && !label.contains(" — "),
            "{kind:?} {label}"
        );
    }
    // 英語の画面でも、理由は一般の文（「Unsupported value or operation」）に落ちず、日本語も混ざらない
    h.state_mut().state.set_language(Lang::En);
    h.run();
    for kind in FILTER_SIX {
        let (label, _, reason) = label_of(&h, kind);
        let reason = reason.expect(&label);
        assert!(
            !has_japanese(&label) && !has_japanese(&reason),
            "{kind:?} {label} {reason}"
        );
        assert!(
            !reason.contains("Unsupported value or operation"),
            "{kind:?} 法線: {label} {reason}"
        );
    }
    h.state_mut().state.m2.paint_channel = Channel::Height;
    for kind in [FilterKind::GradientMap, FilterKind::ColorBalance] {
        let (label, enabled, reason) = label_of(&h, kind);
        assert!(!enabled, "{label}");
        assert!(
            reason
                .as_deref()
                .is_some_and(|r| r.contains("apply only to color channels")),
            "{kind:?} {label} {reason:?}"
        );
    }
}

#[test]
fn the_panels_for_layers_and_stages_have_no_instruction_text_and_the_english_ones_no_japanese() {
    use egui::epaint::Shape;
    fn texts(h: &Harness<'_, YoluApp>) -> Vec<String> {
        fn walk(shape: &Shape, out: &mut Vec<String>) {
            match shape {
                Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
                Shape::Text(text) => out.push(text.galley.job.text.clone()),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for s in &h.output().shapes {
            walk(&s.shape, &mut out);
        }
        out
    }
    for lang in Lang::ALL {
        for kind in SIX {
            let mut h = app(1280.0, 1400.0, 64);
            h.state_mut().state.set_language(lang);
            h.run();
            let _ = new_adjustment(&mut h, kind);
            for text in texts(&h) {
                assert!(
                    !text.contains('。') && !text.ends_with('.'),
                    "{lang:?}/{kind:?}: {text:?}"
                );
                if lang == Lang::En {
                    assert!(!has_japanese(&text), "{lang:?}/{kind:?}: {text:?}");
                }
            }
        }
    }
}

#[test]
fn saving_a_smart_material_with_a_colour_adjustment_is_refused_with_a_short_reason_in_both_languages(
) {
    use yolu_io::smart::SmartFile;
    use yolu_io::WriterInfo;
    let mut doc = yolu_core::Document::with_tile_size(32, 32, 16).unwrap();
    let base = doc.add_layer("下地").unwrap();
    let adj = doc
        .add_adjustment_layer("調整", AdjustmentKind::Threshold.settings(), None, None)
        .unwrap();
    let material = doc.capture_smart_material(&[base, adj], "素材").unwrap();
    let writer = WriterInfo {
        app: "YoluPainter-rs".into(),
        version: "0.0.1".into(),
        unity: "none".into(),
    };
    let err = SmartFile::from_core(&material, &writer).unwrap_err();
    assert_eq!(
        yolu_app::lang::shelf_io_error(Lang::Ja, &err),
        "色調補正を使っています"
    );
    let en = yolu_app::lang::shelf_io_error(Lang::En, &err);
    assert_eq!(en, "It uses colour adjustments");
    assert!(!has_japanese(&en));
}

#[test]
fn the_mixing_mode_and_the_mixing_curve_are_one_undo_each_and_come_back_after_saving() {
    use yolu_core::generator::{LuminanceCorrection, MixMode};
    let mut h = app(1280.0, 1500.0, 64);
    let id = new_adjustment(&mut h, AdjustmentKind::GradientMap);
    let first = value_of(&h, id);
    let steps = undo_count(&h);
    let ramp_of = |h: &Harness<'_, YoluApp>| match value_of(h, id) {
        ColorAdjust::GradientMap(g) => g.ramp().clone(),
        other => panic!("{other:?}"),
    };
    // 混色モード（1 回の操作 = 1 回の取り消し）
    click_label_in_panel(&mut h, "知覚的");
    assert_eq!(ramp_of(&h).mix_mode(), MixMode::Perceptual);
    assert_eq!(undo_count(&h), steps + 1);
    click_label_in_panel(&mut h, "最大");
    assert_eq!(ramp_of(&h).luminance_correction(), LuminanceCorrection::Max);
    assert_eq!(undo_count(&h), steps + 2);
    // 混合率曲線（最初の分岐点の区間）
    scroll_panel_to(&mut h, "混合率曲線");
    click_check_in_panel(&mut h, "混合率曲線");
    assert!(ramp_of(&h).segment_curve(0).is_some());
    assert_eq!(undo_count(&h), steps + 3);
    let mixed = value_of(&h, id);
    // 保存して開き直すと、混色も混合率曲線も戻る（正本の版 25）
    let dir = std::env::temp_dir().join(format!("yolu-gradmap-save-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("mixing.ylp");
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(path.clone()));
    assert!(
        h.state().state.message.starts_with("保存しました"),
        "{}",
        h.state().state.message
    );
    let mut opened = yolu_app::state::AppState::new(8, 8);
    opened.apply(Action::OpenProject(path));
    assert!(
        opened.message.starts_with("開きました"),
        "{}",
        opened.message
    );
    let again = opened
        .doc
        .layers()
        .iter()
        .find_map(|l| l.adjustment().and_then(|a| a.color_adjust()))
        .expect("調整レイヤー");
    assert_eq!(again, mixed);
    let _ = std::fs::remove_dir_all(dir);
    // 1 つずつ Undo で戻り、Redo で進む
    for _ in 0..3 {
        apply(&mut h, Action::Undo);
    }
    assert_eq!(value_of(&h, id), first);
    assert_eq!(undo_count(&h), steps);
    for _ in 0..3 {
        apply(&mut h, Action::Redo);
    }
    assert_eq!(value_of(&h, id), mixed);
}

#[test]
fn dragging_in_the_colour_picker_is_one_undo_step_and_escape_goes_back_to_the_first_colour() {
    let mut h = app(1280.0, 1500.0, 64);
    let id = new_adjustment(&mut h, AdjustmentKind::GradientMap);
    let first = value_of(&h, id);
    let steps = undo_count(&h);
    // 色の見本を押して色の選びを開き、四角の中で何度か動かす（1 回のドラッグ）
    scroll_panel_to(&mut h, "分岐点の色");
    let swatch = rect_of(&h, "分岐点の色", |r| r.left() > rx());
    click(&mut h, swatch.center());
    let target = yolu_app::panels::ramp_rows::stop_target(("adjustment", id.0), 0);
    assert!(yolu_app::panels::color_window::is_target(&h.ctx, target));
    let window = yolu_app::panels::color_window::rect(&h.ctx).expect("色のウィンドウが開く");
    let wheel = yolu_app::panels::color_window::wheel_of(window);
    let sq = yolu_app::panels::color::wheel_square(wheel);
    drag(
        &mut h,
        &[
            pos2(sq.left() + 8.0, sq.top() + 8.0),
            pos2(sq.center().x, sq.center().y),
            pos2(sq.right() - 8.0, sq.bottom() - 8.0),
        ],
    );
    assert_ne!(value_of(&h, id), first, "その場で色が変わる");
    assert_eq!(
        undo_count(&h),
        steps + 1,
        "ドラッグは離すまで 1 回の取り消し"
    );
    // Esc で最初の色へ戻る（戻す変更も 1 回の取り消しとして積む。Undo で選んだ色へ、もう 1 回で最初の色へ）
    h.key_press(egui::Key::Escape);
    h.run();
    assert_eq!(value_of(&h, id), first);
    assert_eq!(undo_count(&h), steps + 2);
    apply(&mut h, Action::Undo);
    assert_ne!(value_of(&h, id), first);
    apply(&mut h, Action::Undo);
    assert_eq!(value_of(&h, id), first);
    assert_eq!(undo_count(&h), steps);
}
