//! 色調補正の 6 種（グラデーションマップ・トーンカーブ・カラーバランス・明るさ/コントラスト・2 値化・ポスタリゼーション）の欄
//! （`panels::color_adjust`。調整の層と、フィルターの段が同じ欄を使う）。欄だけを並べた見本の窓で、操作・日英・説明を置かないこと・無効・
//! スナップショットを確かめる。アプリの中でのつなぎ（層・段の選び・1 回の Undo）は `color_adjust_app.rs`。
use crate::common;

use common::*;
use egui::{pos2, vec2, Rect, Ui};
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, SnapshotResults};
use yolu_app::eyedrop::EyedropState;
use yolu_app::lang::Lang;
use yolu_app::panels::color_adjust::{self, Change, Histogram, Params};
use yolu_app::rampsets::RampSets;
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::Rows;
use yolu_app::YoluApp;
use yolu_core::curve::{Curve, CurvePoint};
use yolu_core::{
    AdjustmentType, BalanceRange, BrightnessContrast, ColorAdjust, ColorBalance, GradientMap,
    Posterize, Threshold, ToneChannel, ToneCurves,
};

const WIDTH: f32 = 320.0;
const HEIGHT: f32 = 560.0;
/// グラデーションマップの欄は縦に長い（混色・セット・分岐点・色・混合率曲線）。
const TALL: f32 = 600.0;

struct Panel {
    ready: bool,
    value: ColorAdjust,
    lang: Lang,
    enabled: bool,
    histogram: Option<Histogram>,
    /// 返された変更（新しい値と discrete）。
    changes: Vec<Change>,
    height: f32,
    sets: RampSets,
    eyedrop: EyedropState,
    failure: Option<String>,
}

fn draw(ui: &mut Ui, p: &mut Panel) {
    if !p.ready {
        // 書体は次のフレームから効く（このフレームで太字の書体を使うと egui が止まる）ので、初めのフレームは準備だけ
        YoluApp::setup(ui.ctx());
        p.ready = true;
        ui.ctx().request_repaint();
        return;
    }
    let area = Rect::from_min_size(ui.max_rect().min, vec2(WIDTH, p.height));
    ui.allocate_rect(area, egui::Sense::hover());
    ui.painter().rect_filled(area, 0.0, t::PANEL_BG);
    let mut rows = Rows::new(area, 0.0);
    rows.indent = t::SECTION_INDENT;
    let mut params = Params {
        key: ("test", 1),
        enabled: p.enabled,
        why: None,
        paint: [0.9, 0.2, 0.1, 1.0],
        sub: [0.1, 0.2, 0.9, 1.0],
        lang: p.lang,
        histogram: p.histogram.as_ref(),
        sets: &mut p.sets,
        eyedrop: &mut p.eyedrop,
        failure: &mut p.failure,
    };
    if let Some(change) = color_adjust::rows(ui, &mut rows, &mut params, &p.value) {
        p.value = change.value.clone();
        p.changes.push(change);
    }
}

fn panel(value: ColorAdjust, lang: Lang) -> Harness<'static, Panel> {
    let height = if matches!(value, ColorAdjust::GradientMap(_)) {
        TALL
    } else {
        HEIGHT
    };
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(WIDTH, height))
        .build_ui_state(
            draw,
            Panel {
                ready: false,
                value,
                lang,
                enabled: true,
                histogram: None,
                changes: Vec::new(),
                height,
                sets: RampSets::default(),
                eyedrop: EyedropState::default(),
                failure: None,
            },
        );
    h.run();
    h.run();
    h
}

fn default_value(kind: AdjustmentType) -> ColorAdjust {
    ColorAdjust::default_for(kind).unwrap()
}

fn rect(h: &Harness<'_, Panel>, label: &str) -> Rect {
    h.get_all_by_label(label)
        .next()
        .unwrap_or_else(|| panic!("{label}"))
        .rect()
}

/// スライダーの溝の真ん中の `fraction`（0〜1）の位置を押す。
fn click_slider(h: &mut Harness<'_, Panel>, label: &str, fraction: f32) {
    let r = rect(h, label);
    let at = pos2(r.left() + r.width() * fraction, r.bottom() - 6.0);
    click_at(h, at);
}

fn click_at(h: &mut Harness<'_, Panel>, at: egui::Pos2) {
    h.hover_at(at);
    h.step();
    h.drag_at(at);
    h.step();
    h.drop_at(at);
    h.run();
}

/// ラベルの部品の真ん中を押す。
fn click_label(h: &mut Harness<'_, Panel>, label: &str) {
    let at = rect(h, label).center();
    click_at(h, at);
}

/// チェックの行（左端の四角がボタン）を押す。
fn click_check(h: &mut Harness<'_, Panel>, label: &str) {
    let at = rect(h, label).left_center() + vec2(8.0, 0.0);
    click_at(h, at);
}

fn drag_through(h: &mut Harness<'_, Panel>, points: &[egui::Pos2]) {
    h.hover_at(points[0]);
    h.step();
    h.drag_at(points[0]);
    h.step();
    for p in &points[1..] {
        h.hover_at(*p);
        h.step();
    }
    h.drop_at(*points.last().unwrap());
    h.run();
}

fn last<'a>(h: &'a Harness<'_, Panel>) -> &'a Change {
    h.state().changes.last().expect("変更が返された")
}

#[test]
fn threshold_posterize_and_brightness_contrast_sliders_change_their_values_in_their_ranges() {
    let mut h = panel(default_value(AdjustmentType::Threshold), Lang::Ja);
    click_slider(&mut h, "しきい値", 0.0);
    assert_eq!(
        last(&h).value,
        ColorAdjust::Threshold(Threshold::new(1).unwrap())
    );
    assert!(
        !last(&h).discrete,
        "スライダーは離すまで 1 回の取り消しにまとめる"
    );
    click_slider(&mut h, "しきい値", 1.0);
    assert_eq!(
        last(&h).value,
        ColorAdjust::Threshold(Threshold::new(255).unwrap())
    );
    click_slider(&mut h, "しきい値", 0.5);
    let ColorAdjust::Threshold(v) = &last(&h).value else {
        panic!()
    };
    assert!((120..=135).contains(&v.level()), "{}", v.level());

    let mut h = panel(default_value(AdjustmentType::Posterize), Lang::Ja);
    click_slider(&mut h, "階調", 0.0);
    assert_eq!(
        last(&h).value,
        ColorAdjust::Posterize(Posterize::new(2).unwrap())
    );
    click_slider(&mut h, "階調", 1.0);
    assert_eq!(
        last(&h).value,
        ColorAdjust::Posterize(Posterize::new(255).unwrap())
    );

    let mut h = panel(default_value(AdjustmentType::BrightnessContrast), Lang::Ja);
    click_slider(&mut h, "明るさ", 0.0);
    click_slider(&mut h, "コントラスト", 1.0);
    let ColorAdjust::BrightnessContrast(v) = &last(&h).value else {
        panic!()
    };
    assert_eq!((v.brightness(), v.contrast()), (-150.0, 100.0));
    click_slider(&mut h, "明るさ", 1.0);
    click_slider(&mut h, "コントラスト", 0.0);
    let ColorAdjust::BrightnessContrast(v) = &last(&h).value else {
        panic!()
    };
    assert_eq!((v.brightness(), v.contrast()), (150.0, -50.0));
    assert!(h.state().changes.iter().all(|c| !c.discrete));
}

#[test]
fn color_balance_edits_the_chosen_range_and_keeps_the_others() {
    let mut h = panel(default_value(AdjustmentType::ColorBalance), Lang::Ja);
    // 中間（始めの範囲）のシアン — レッドを右端へ
    click_slider(&mut h, "シアン — レッド", 1.0);
    let ColorAdjust::ColorBalance(v) = last(&h).value.clone() else {
        panic!()
    };
    assert_eq!(v.values(BalanceRange::Midtones), [100.0, 0.0, 0.0]);
    assert_eq!(v.values(BalanceRange::Shadows), [0.0; 3]);
    // シャドウに切り替えて、イエロー — ブルーを左端へ。中間は残る
    click_label(&mut h, "シャドウ");
    assert!(
        h.state().changes.iter().all(|c| !c.discrete),
        "範囲の切り替えは値を変えない"
    );
    click_slider(&mut h, "イエロー — ブルー", 0.0);
    let ColorAdjust::ColorBalance(v) = last(&h).value.clone() else {
        panic!()
    };
    assert_eq!(v.values(BalanceRange::Shadows), [0.0, 0.0, -100.0]);
    assert_eq!(v.values(BalanceRange::Midtones), [100.0, 0.0, 0.0]);
    // 輝度を保つは 1 回で決まる変更
    assert!(v.preserve_luminosity());
    click_check(&mut h, "輝度を保つ");
    let ColorAdjust::ColorBalance(v) = last(&h).value.clone() else {
        panic!()
    };
    assert!(!v.preserve_luminosity());
    assert!(last(&h).discrete);
    assert_eq!(v.values(BalanceRange::Shadows), [0.0, 0.0, -100.0]);
}

#[test]
fn tone_curve_edits_one_curve_at_a_time_and_presets_set_the_selected_one() {
    let mut h = panel(default_value(AdjustmentType::ToneCurve), Lang::Ja);
    // 曲線の枠（チャンネルのボタンの行の下）
    let strip = rect(&h, "RGB");
    let blue = rect(&h, "B");
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
    // R に切り替え、中ほどを押して上へ引いて離す（点が 1 つ増える。1 回で決まる変更）
    click_label(&mut h, "R");
    assert!(h.state().changes.is_empty());
    drag_through(&mut h, &[at(0.5, 0.5), at(0.5, 0.7), at(0.5, 0.8)]);
    assert_eq!(h.state().changes.len(), 1);
    assert!(last(&h).discrete);
    let ColorAdjust::ToneCurve(v) = last(&h).value.clone() else {
        panic!()
    };
    assert_eq!(v.curve(ToneChannel::Red).points().len(), 3);
    let p = v.curve(ToneChannel::Red).points()[1];
    assert!(
        (p.x - 0.5).abs() < 0.04 && (p.y - 0.8).abs() < 0.04,
        "{p:?}"
    );
    // 画面で作る曲線の点は PSD と同じ 0〜255 の整数の刻みに乗る（PSD の調整レイヤーへ書ける）
    let on_steps = |c: &yolu_core::curve::Curve| {
        c.points().iter().all(|p| {
            [p.x, p.y]
                .iter()
                .all(|v| ((v * 255.0).round() - v * 255.0).abs() < 1e-9)
        })
    };
    assert!(on_steps(v.curve(ToneChannel::Red)), "{p:?}");
    for other in [
        ToneChannel::Composite,
        ToneChannel::Green,
        ToneChannel::Blue,
    ] {
        assert!(v.curve(other).is_identity(), "{other:?}");
    }
    // B に切り替えて、S 字のプリセット: B だけが替わる
    click_label(&mut h, "B");
    click_label(&mut h, "S 字");
    let ColorAdjust::ToneCurve(w) = last(&h).value.clone() else {
        panic!()
    };
    assert_eq!(w.curve(ToneChannel::Blue).points().len(), 4);
    assert!(
        on_steps(w.curve(ToneChannel::Blue)),
        "プリセットも刻みに乗る"
    );
    assert_eq!(w.curve(ToneChannel::Red), v.curve(ToneChannel::Red));
    assert!(last(&h).discrete);
    // 線形のプリセットは直線に戻す
    click_label(&mut h, "線形");
    let ColorAdjust::ToneCurve(w) = last(&h).value.clone() else {
        panic!()
    };
    assert!(w.curve(ToneChannel::Blue).is_identity());
}

/// 分岐点の編集の部品の場所（「前の分岐点」のボタンの上。部品の高さは 76、行の間は 4）。
fn stops_editor_rect(h: &Harness<'_, Panel>) -> Rect {
    let prev = rect(h, "<");
    Rect::from_min_size(
        pos2(
            prev.left(),
            prev.top() - 4.0 - yolu_app::ui::ramp::STOPS_HEIGHT,
        ),
        vec2(
            WIDTH - 2.0 * t::PADDING - t::SECTION_INDENT,
            yolu_app::ui::ramp::STOPS_HEIGHT,
        ),
    )
}

#[test]
fn gradient_map_presets_reverse_stops_and_the_colour_of_the_selected_stop() {
    let mut h = panel(default_value(AdjustmentType::GradientMap), Lang::Ja);
    let ramp_of = |h: &Harness<'_, Panel>| match &h.state().value {
        ColorAdjust::GradientMap(g) => g.clone(),
        other => panic!("{other:?}"),
    };
    // グラデーションセット: 組を替えて、見本を押す（1 回で決まる変更）
    click_label(&mut h, "色味");
    click_label(&mut h, "セピア");
    let sepia = yolu_app::rampsets::builtin::groups([0.0; 4], [0.0; 4])
        .into_iter()
        .flat_map(|g| g.items)
        .find(|i| i.en == "Sepia")
        .unwrap()
        .ramp;
    assert_eq!(ramp_of(&h).ramp(), &sepia);
    assert!(last(&h).discrete);
    // 逆向き
    assert!(!ramp_of(&h).reverse());
    click_check(&mut h, "逆向き");
    assert!(ramp_of(&h).reverse());
    assert_eq!(ramp_of(&h).ramp(), &sepia, "向きだけが替わる");
    // 分岐点の編集: 色の行の何も無い所を押すと足す
    let editor = stops_editor_rect(&h);
    let bar_left = editor.left() + 7.0;
    let bar_width = editor.width() - 14.0;
    // 色の分岐点の行は枠の下の方（上から 67）。位置 0.25 は 2 つの端の間の何も無い所
    click_at(
        &mut h,
        pos2(bar_left + bar_width * 0.25, editor.top() + 67.0),
    );
    assert_eq!(ramp_of(&h).ramp().colors().len(), 4);
    assert!(last(&h).discrete);
    // 選んだ分岐点を消す
    click_label(&mut h, "分岐点を消す");
    assert_eq!(ramp_of(&h).ramp().colors().len(), 3);
    // 分岐点の色をメインの色にする（1 回で決まる）
    click_label(&mut h, "メイン");
    assert!(last(&h).discrete);
    let g = ramp_of(&h);
    assert!(g
        .ramp()
        .colors()
        .iter()
        .any(|c| (c.color.r, c.color.g, c.color.b) == (230, 51, 26)));
    // サブの色にもでき、指定へ戻すと今の色のまま
    click_label(&mut h, "サブ");
    assert!(ramp_of(&h)
        .ramp()
        .colors()
        .iter()
        .any(|c| (c.color.r, c.color.g, c.color.b) == (26, 51, 230)));
    // 分岐点の位置のスライダー（離すまでまとめる）
    let n = h.state().changes.len();
    let before = ramp_of(&h);
    click_slider(&mut h, "位置", 0.25);
    assert!(h.state().changes.len() > n);
    assert_ne!(ramp_of(&h), before);
    assert!(
        !last(&h).discrete,
        "スライダーは離すまで 1 回の取り消しにまとめる"
    );
}

#[test]
fn a_disabled_panel_changes_nothing() {
    for kind in [
        AdjustmentType::Threshold,
        AdjustmentType::GradientMap,
        AdjustmentType::ToneCurve,
        AdjustmentType::ColorBalance,
    ] {
        let mut h = panel(default_value(kind), Lang::Ja);
        h.state_mut().enabled = false;
        h.run();
        let before = h.state().value.clone();
        for y in (60..400).step_by(37) {
            click_at(&mut h, pos2(100.0, y as f32));
            click_at(&mut h, pos2(250.0, y as f32));
        }
        assert!(h.state().changes.is_empty(), "{kind:?}");
        assert_eq!(h.state().value, before);
    }
}

/// 描いた文字の一覧。
fn shown_texts(h: &Harness<'_, Panel>) -> Vec<String> {
    use egui::epaint::Shape;
    fn walk(shape: &Shape, out: &mut Vec<String>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            Shape::Text(text) => out.push(text.galley.job.text.clone()),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        walk(&shape.shape, &mut out);
    }
    out
}

#[test]
fn the_panels_show_names_and_values_only_in_both_languages() {
    for lang in Lang::ALL {
        for kind in [
            AdjustmentType::GradientMap,
            AdjustmentType::ToneCurve,
            AdjustmentType::ColorBalance,
            AdjustmentType::BrightnessContrast,
            AdjustmentType::Threshold,
            AdjustmentType::Posterize,
        ] {
            let h = panel(default_value(kind), lang);
            let texts = shown_texts(&h);
            assert!(!texts.is_empty(), "{kind:?}");
            for text in &texts {
                assert_plain(&format!("{lang:?}/{kind:?}"), text);
                assert!(!text.contains('。') && !text.ends_with('.'), "{text:?}");
                assert!(text.chars().count() <= 30, "長い文字 {text:?}");
                if lang == Lang::En {
                    assert!(!has_japanese(text), "英語の画面に日本語 {text:?}");
                }
            }
        }
    }
}

fn snapshot_value(value: ColorAdjust, lang: Lang, name: &str, results: &mut SnapshotResults) {
    let mut h = panel(value, lang);
    h.snapshot(name);
    results.extend_harness(&mut h);
}

#[test]
fn snapshot_the_six_panels() {
    let mut results = SnapshotResults::new();
    let rb = Curve::new(vec![
        CurvePoint { x: 0.0, y: 0.0 },
        CurvePoint { x: 0.3, y: 0.15 },
        CurvePoint { x: 0.7, y: 0.85 },
        CurvePoint { x: 1.0, y: 1.0 },
    ])
    .unwrap();
    let tone =
        ColorAdjust::ToneCurve(ToneCurves::identity().with_curve(ToneChannel::Composite, rb));
    let sepia = yolu_app::rampsets::builtin::groups([0.0; 4], [0.0; 4])
        .into_iter()
        .flat_map(|g| g.items)
        .find(|i| i.en == "Sepia")
        .unwrap()
        .ramp;
    let gradient = ColorAdjust::GradientMap(GradientMap::new(sepia, false));
    let balance = ColorAdjust::ColorBalance(
        ColorBalance::new(
            [10.0, 0.0, -20.0],
            [0.0, 35.0, 0.0],
            [-15.0, 0.0, 40.0],
            true,
        )
        .unwrap(),
    );
    let bc = ColorAdjust::BrightnessContrast(BrightnessContrast::new(40.0, 25.0).unwrap());
    snapshot_value(
        gradient.clone(),
        Lang::Ja,
        "color_adjust_gradient_map_ja",
        &mut results,
    );
    snapshot_value(
        gradient,
        Lang::En,
        "color_adjust_gradient_map_en",
        &mut results,
    );
    snapshot_value(
        tone.clone(),
        Lang::Ja,
        "color_adjust_tone_curve_ja",
        &mut results,
    );
    snapshot_value(tone, Lang::En, "color_adjust_tone_curve_en", &mut results);
    snapshot_value(
        balance,
        Lang::Ja,
        "color_adjust_color_balance_ja",
        &mut results,
    );
    snapshot_value(
        bc,
        Lang::En,
        "color_adjust_brightness_contrast_en",
        &mut results,
    );
    snapshot_value(
        ColorAdjust::Threshold(Threshold::new(96).unwrap()),
        Lang::Ja,
        "color_adjust_threshold_ja",
        &mut results,
    );
    snapshot_value(
        ColorAdjust::Posterize(Posterize::new(5).unwrap()),
        Lang::En,
        "color_adjust_posterize_en",
        &mut results,
    );
}
