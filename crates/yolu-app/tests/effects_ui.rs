//! 効果の行（層の行の下）と、選んだ効果の欄・メニューの画面（egui_kittest）。行を押す・目・上へ・下へ・消す・右クリック・メニュー、
//! 日英の見た目（説明の文を置かない）、スナップショット。文書の操作そのものは `tests/effects.rs`（画面なし）。
mod common;

use common::*;
use egui::{epaint::Shape, pos2, PointerButton, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::SnapshotResults;
use egui_kittest::Harness;
use yolu_app::fx::{FilterKind, FxOp, Selected};
use yolu_app::lang::Lang;
use yolu_app::m2::{Edit, UiOp};
use yolu_app::state::{Action, PopupKind};
use yolu_app::YoluApp;
use yolu_core::generator::Kind;
use yolu_core::{AnchorPlacement, FilterTarget};

fn apply(h: &mut Harness<'_, YoluApp>, action: Action) {
    h.state_mut().state.apply(action);
    h.run();
}

fn fx(h: &mut Harness<'_, YoluApp>, op: FxOp) {
    apply(h, Action::Fx(op));
}

/// 効果の行を並べた文書: 下地の層（ぼかし・階調の反転・アンカー）と、マスクにマップを読む Generator。
fn effect_document(h: &mut Harness<'_, YoluApp>) {
    apply(h, Action::M2(Edit::NewFill));
    let layer = h.state().state.selected_layer.unwrap();
    fx(h, FxOp::AddAnchor { layer, placement: AnchorPlacement::Layer });
    fx(h, FxOp::AddFilter { target: FilterTarget::Content, kind: FilterKind::Blur });
    fx(h, FxOp::AddFilter { target: FilterTarget::Content, kind: FilterKind::Invert });
    apply(h, Action::M2(Edit::AddMask(layer)));
    fx(h, FxOp::AddGenerator { target: FilterTarget::Mask, kind: Kind::EdgeWear });
}

fn selected(h: &Harness<'_, YoluApp>) -> Option<Selected> {
    h.state().state.fx.selected
}

#[test]
fn snapshot_effect_rows_and_the_generator_that_has_no_maps() {
    let mut results = SnapshotResults::new();
    for lang in Lang::ALL {
        let mut h = app(1280.0, 1000.0, 128);
        h.state_mut().state.set_language(lang);
        effect_document(&mut h);
        h.snapshot(format!("fx_rows_generator_{}", lang.pick("ja", "en")));
        results.extend_harness(&mut h);
    }
}

#[test]
fn snapshot_a_selected_filter_and_a_selected_anchor() {
    let mut h = app(1280.0, 1000.0, 128);
    effect_document(&mut h);
    let layer = h.state().state.selected_layer.unwrap();
    let blur = h.state().state.doc.filters_of(layer, FilterTarget::Content).unwrap()[0].id();
    fx(&mut h, FxOp::SelectFilter { layer, id: blur });
    h.snapshot("fx_rows_blur_selected");
    let anchor = h.state().state.doc.layer(layer).unwrap().anchor().unwrap().id();
    fx(&mut h, FxOp::SelectAnchor(anchor));
    h.snapshot("fx_rows_anchor_selected");
}

#[test]
fn snapshot_the_add_menu() {
    let mut h = app(1280.0, 1000.0, 128);
    effect_document(&mut h);
    fx(&mut h, FxOp::Deselect);
    h.state_mut().state.property_tab = yolu_app::panels::properties::TAB_ICONS.len() - 1; // レイヤーのタブ（最後）
    h.run();
    h.get_by_label("フィルターを足す").click();
    h.run();
    assert!(matches!(
        h.state().state.popup.as_ref().map(|p| p.kind),
        Some(PopupKind::M2(yolu_app::m2_menu::Popup::AddEffect(FilterTarget::Content)))
    ));
    h.snapshot("fx_add_menu");
}

#[test]
fn rows_select_toggle_move_and_remove_from_the_panel() {
    let mut h = app(1280.0, 1000.0, 128);
    apply(&mut h, Action::M2(Edit::NewFill));
    let layer = h.state().state.selected_layer.unwrap();
    fx(&mut h, FxOp::AddFilter { target: FilterTarget::Content, kind: FilterKind::Blur });
    fx(&mut h, FxOp::AddFilter { target: FilterTarget::Content, kind: FilterKind::Invert });
    let ids: Vec<_> = h.state().state.doc.filters_of(layer, FilterTarget::Content).unwrap().iter().map(|e| e.id()).collect();
    // 層の行を押すと、選んでいた効果は外れる
    let layer_name = h.state().state.doc.layer(layer).unwrap().name().to_owned();
    h.get_by_label(&layer_name).click();
    h.run();
    assert_eq!(selected(&h), None);
    // 効果の行を押すと選ぶ（その層のまま）
    let blur_label = "ぼかし（ガウス）  4 px";
    h.get_by_label(blur_label).click();
    h.run();
    assert_eq!(selected(&h), Some(Selected::Filter { layer, id: ids[0] }));
    // 上へ（後から掛かる）: ぼかしが 1 つ上の段になる。1 回の Undo
    let steps = h.state().state.doc.undo_count();
    h.get_by_label("上へ（後から掛かる）").click();
    h.run();
    let order: Vec<_> = h.state().state.doc.filters_of(layer, FilterTarget::Content).unwrap().iter().map(|e| e.id()).collect();
    assert_eq!(order, vec![ids[1], ids[0]]);
    assert_eq!(h.state().state.doc.undo_count(), steps + 1);
    // 目で無効にする
    h.get_all_by_label("フィルターを無効にする").next().unwrap().click();
    h.run();
    assert!(!h.state().state.doc.find_filter(ids[0]).unwrap().1.enabled(), "一番上の行（ぼかし）の目で無効になる");
    // 消す
    h.get_by_label("フィルターを削除").click();
    h.run();
    assert_eq!(h.state().state.doc.filters_of(layer, FilterTarget::Content).unwrap().len(), 1);
    assert_eq!(selected(&h), None, "消した行の選びは外れる");
    // Undo（Ctrl+Z）で戻る
    key(&h, egui::Key::Z, egui::Modifiers::COMMAND);
    h.run();
    assert_eq!(h.state().state.doc.filters_of(layer, FilterTarget::Content).unwrap().len(), 2);
}

#[test]
fn the_context_menu_of_a_row_acts_on_that_row() {
    let mut h = app(1280.0, 1000.0, 128);
    apply(&mut h, Action::M2(Edit::NewFill));
    let layer = h.state().state.selected_layer.unwrap();
    fx(&mut h, FxOp::AddFilter { target: FilterTarget::Content, kind: FilterKind::Blur });
    let id = h.state().state.doc.filters_of(layer, FilterTarget::Content).unwrap()[0].id();
    fx(&mut h, FxOp::Deselect);
    let at = h.get_by_label("ぼかし（ガウス）  4 px").rect().center();
    press(&h, at, PointerButton::Secondary);
    h.step();
    release(&h, at, PointerButton::Secondary);
    h.run();
    assert_eq!(selected(&h), Some(Selected::Filter { layer, id }), "右クリックで選ぶ");
    assert!(matches!(
        h.state().state.popup.as_ref().map(|p| p.kind),
        Some(PopupKind::M2(yolu_app::m2_menu::Popup::EffectContext))
    ));
    // 行の消すボタンと同じ名前なので、メニューの項目（幅のある矩形）を選ぶ
    let item = rect_of(&h, "フィルターを削除", |r| r.width() > 80.0);
    click(&mut h, item.center());
    assert!(h.state().state.doc.filters_of(layer, FilterTarget::Content).unwrap().is_empty());
}

#[test]
fn a_filter_is_added_from_the_properties_button_and_the_filter_menu() {
    let mut h = app(1280.0, 1000.0, 128);
    let layer = h.state().state.selected_layer.unwrap();
    h.state_mut().state.property_tab = yolu_app::panels::properties::TAB_ICONS.len() - 1; // レイヤーのタブ（最後）
    h.run();
    h.get_by_label("フィルターを足す").click();
    h.run();
    let item = popup_item(&h, "シャープ");
    click(&mut h, item.center());
    assert_eq!(h.state().state.doc.filters_of(layer, FilterTarget::Content).unwrap().len(), 1);
    // メニューバーの「フィルター」から
    let title = menu_title(&h, "フィルター");
    click(&mut h, title.center());
    assert!(matches!(h.state().state.popup.as_ref().map(|p| p.kind), Some(PopupKind::MenuBar(4))));
    let item = popup_item(&h, "階調の反転");
    click(&mut h, item.center());
    assert_eq!(h.state().state.doc.filters_of(layer, FilterTarget::Content).unwrap().len(), 2);
}

#[test]
fn dragging_a_slider_in_the_effect_panel_is_one_undo_step() {
    let mut h = app(1280.0, 1000.0, 128);
    let layer = h.state().state.selected_layer.unwrap();
    fx(&mut h, FxOp::AddFilter { target: FilterTarget::Content, kind: FilterKind::Blur });
    let id = h.state().state.doc.filters_of(layer, FilterTarget::Content).unwrap()[0].id();
    let steps = h.state().state.doc.undo_count();
    let slider = rect_of(&h, "半径", |r| r.left() > 1000.0);
    let y = slider.center().y + 8.0;
    drag(&mut h, &[pos2(slider.left() + 30.0, y), pos2(slider.left() + 80.0, y), pos2(slider.left() + 120.0, y)]);
    let radius = match h.state().state.doc.find_filter(id).unwrap().1.settings() {
        yolu_core::EffectSettings::Filter(yolu_core::filter::Settings::GaussianBlur { radius }) => *radius,
        other => panic!("{other:?}"),
    };
    assert_ne!(radius, 4, "つまみで半径が変わる");
    assert_eq!(h.state().state.doc.undo_count(), steps + 1, "ドラッグは 1 回の Undo");
    let _ = UiOp::EditMask(false);
}


/// 描いた文字（アイコンの頭文字は除く）の一覧。
fn shown_texts(h: &Harness<'_, YoluApp>) -> Vec<(String, Rect)> {
    fn walk(shape: &Shape, out: &mut Vec<(String, Rect)>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            Shape::Text(text) => {
                out.push((text.galley.job.text.clone(), Rect::from_min_size(text.pos, text.galley.size())));
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        walk(&shape.shape, &mut out);
    }
    out
}

fn has_japanese(text: &str) -> bool {
    text.chars().any(|c| matches!(c, '\u{3000}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}' | '\u{ff00}'..='\u{ffef}'))
}

#[test]
fn the_effect_screens_have_no_instruction_text_and_the_english_one_no_japanese() {
    for lang in Lang::ALL {
        for select in 0..3 {
            let mut h = app(1280.0, 1000.0, 128);
            h.state_mut().state.set_language(lang);
            effect_document(&mut h);
            let layer = h.state().state.selected_layer.unwrap();
            let stage = h.state().state.doc.filters_of(layer, FilterTarget::Content).unwrap()[0].id();
            match select {
                0 => {}
                1 => fx(&mut h, FxOp::SelectFilter { layer, id: stage }),
                _ => {
                    let anchor = h.state().state.doc.layer(layer).unwrap().anchor().unwrap().id();
                    fx(&mut h, FxOp::SelectAnchor(anchor));
                }
            }
            // 開いたメニューも
            let texts = shown_texts(&h);
            // 層の一覧とプロパティの欄（右の列。状態の帯の知らせは状態なので除く）
            for (text, rect) in texts.iter().filter(|(_, r)| r.left() > 1050.0 && r.top() > 280.0 && r.bottom() < 976.0) {
                assert!(!text.contains('。') && !text.ends_with('.'), "{lang:?}/{select}: 文の形の文字 {text:?}");
                assert!(text.chars().count() <= 40, "{lang:?}/{select}: 長い文字 {text:?} {rect:?}");
            }
            if lang == Lang::En {
                for (text, _) in &texts {
                    assert!(!has_japanese(text), "英語の画面に日本語 {text:?}");
                }
            }
        }
    }
}

#[test]
fn the_add_menu_in_english_has_no_japanese_and_in_japanese_every_name_is_localised() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 1000.0, 128);
        h.state_mut().state.set_language(lang);
        h.state_mut().state.property_tab = yolu_app::panels::properties::TAB_ICONS.len() - 1;
        h.run();
        h.get_by_label(lang.pick("フィルターを足す", "Add Filter")).click();
        h.run();
        for (text, _) in shown_texts(&h) {
            if lang == Lang::En {
                assert!(!has_japanese(&text), "{text:?}");
            }
        }
        let names: Vec<String> = yolu_app::fx::menu::add_entries(&h.state().state, FilterTarget::Content)
            .iter()
            .filter_map(|e| match e {
                yolu_app::ui::menu::Entry::Item { label, .. } => Some(label.clone()),
                _ => None,
            })
            .collect();
        assert!(names.len() >= 15, "{names:?}");
    }
}

#[test]
fn a_generator_row_without_maps_has_a_mark_whose_tooltip_gives_the_reason() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 1000.0, 128);
        h.state_mut().state.set_language(lang);
        effect_document(&mut h);
        let label = format!("{}  {}", lang.pick("エッジの摩耗", "Edge Wear"), lang.pick("乗算", "Multiply"));
        let row = h.get_by_label(&label).rect();
        // 選んでいる行には上へ・下へ・消すの 3 つのボタンがあり、その左に印
        let at = pos2(row.right() - 4.0 - 60.0 - 10.0, row.center().y);
        move_to(&h, at);
        for _ in 0..90 {
            h.step();
        }
        let reason = lang.pick("Curvature のマップがありません", "No Curvature map");
        let texts = shown_texts(&h);
        // 欄の警告の行にも同じ理由が出ているので、ポインタの近く（行のすぐ下）に出た文字を見る
        assert!(
            texts.iter().any(|(t, r)| t.contains(reason) && r.top() > row.top() && r.top() < row.top() + 90.0),
            "{lang:?}: {texts:?}"
        );
    }
}

fn procedural(h: &Harness<'_, YoluApp>) -> yolu_core::generator::Settings {
    let (_, effect, _) = h.state().state.fx.filter(&h.state().state.doc).unwrap();
    match effect.settings() {
        yolu_core::EffectSettings::Generator(g) => *g.clone(),
        _ => panic!("Generator"),
    }
}

#[test]
fn procedural_controls_and_presets_are_one_undo_and_bilingual() {
    use yolu_core::generator::{CellOutput, NoiseBasis, GrungePreset};
    for lang in Lang::ALL {
        let mut h = app(1280.0, 1800.0, 32);
        h.state_mut().state.set_language(lang);
        fx(&mut h, FxOp::AddGenerator { target: FilterTarget::Content, kind: Kind::Noise });
        let steps = h.state().state.doc.undo_count();
        h.get_by_label(lang.pick("振り直す", "Reroll")).click(); h.run();
        assert_ne!(procedural(&h).procedural.seed, 0);
        assert_eq!(h.state().state.doc.undo_count(), steps + 1);
        apply(&mut h, Action::Undo);
        assert_eq!(procedural(&h).procedural.seed, 0);
        let slider = rect_of(&h, lang.pick("模様の大きさ", "Pattern Size"), |r| r.left() > 1000.0);
        let y = slider.center().y + 8.0;
        drag(&mut h, &[pos2(slider.left() + 30.0, y), pos2(slider.left() + 80.0, y), pos2(slider.left() + 120.0, y)]);
        assert_ne!(procedural(&h).procedural.scale, 0.1);
        assert_eq!(h.state().state.doc.undo_count(), steps + 1);
        apply(&mut h, Action::Undo);
        assert_eq!(procedural(&h).procedural.scale, 0.1);
        let basis = rect_of(&h, lang.pick("基底: Perlin", "Basis: Perlin"), |r| r.left() > 1000.0);
        click(&mut h, basis.center());
        let choice = popup_item(&h, "Worley"); click(&mut h, choice.center());
        assert_eq!(procedural(&h).procedural.basis, NoiseBasis::Worley);
        let cell = rect_of(&h, lang.pick("セルの出力: F1", "Cell Output: F1"), |r| r.left() > 1000.0);
        click(&mut h, cell.center());
        let choice = popup_item(&h, "F2−F1"); click(&mut h, choice.center());
        assert_eq!(procedural(&h).procedural.cell_output, CellOutput::F2MinusF1);
        let basis = rect_of(&h, lang.pick("基底: Worley", "Basis: Worley"), |r| r.left() > 1000.0);
        click(&mut h, basis.center());
        let choice = popup_item(&h, "Perlin"); click(&mut h, choice.center());
        assert_eq!(procedural(&h).procedural.cell_output, CellOutput::F1);
        fx(&mut h, FxOp::AddGenerator { target: FilterTarget::Content, kind: Kind::Grunge });
        let steps = h.state().state.doc.undo_count();
        h.get_by_label(lang.pick("布目", "Weave")).click(); h.run();
        assert_eq!(procedural(&h).procedural.preset, GrungePreset::Weave);
        assert_eq!(h.state().state.doc.undo_count(), steps + 1);
        apply(&mut h, Action::Undo);
        assert_eq!(procedural(&h).procedural.preset, GrungePreset::Stain);
        if lang == Lang::En { assert!(!shown_texts(&h).iter().any(|(t, _)| has_japanese(t))); }
    }
}

/// ノイズ・グランジの欄の、見えている操作を全部触る。どれも core に断られず（状態の帯が空のまま）、設定が変わり、1 回の Undo になる。
fn touch_every_procedural_control(h: &mut Harness<'_, YoluApp>, lang: Lang, kind: Kind) {
    use egui::{Event, Modifiers};
    use yolu_core::generator::{CellOutput, FractalMode, NoiseBasis, ProceduralSpace};
    let column = |r: Rect| r.left() > 1000.0;
    // 1 つの操作の後に、断られていない・設定が変わった・1 回の Undo、を確かめる
    fn accepted(h: &Harness<'_, YoluApp>, what: &str, steps: usize, before: &yolu_core::generator::Settings) {
        let state = &h.state().state;
        assert!(state.message.is_empty(), "{what}: {}", state.message);
        assert_ne!(&procedural(h), before, "{what}: 設定が変わらない");
        assert_eq!(state.doc.undo_count(), steps + 1, "{what}: 1 回の Undo");
    }
    let run = |h: &mut Harness<'_, YoluApp>, what: &str, act: &dyn Fn(&mut Harness<'_, YoluApp>)| {
        h.state_mut().state.message.clear();
        let (steps, before) = (h.state().state.doc.undo_count(), procedural(h));
        act(h);
        accepted(h, what, steps, &before);
    };
    let slide_to = |h: &mut Harness<'_, YoluApp>, label: &str, end: f32| {
        let r = rect_of(h, label, column);
        let y = r.center().y + 8.0;
        drag(h, &[pos2(r.left() + 30.0, y), pos2(r.left() + (30.0 + end) / 2.0, y), pos2(r.left() + end, y)]);
    };
    let slide = |h: &mut Harness<'_, YoluApp>, label: &str| slide_to(h, label, 120.0);
    let choose = |h: &mut Harness<'_, YoluApp>, shown: &str, item: &str| {
        let r = rect_of(h, shown, column);
        click(h, r.center());
        let choice = popup_item(h, item);
        click(h, choice.center());
    };
    let t = |ja: &'static str, en: &'static str| lang.pick(ja, en);
    // 共通
    run(h, "空間", &|h| choose(h, &format!("{}: {}", t("空間", "Space"), t("位置", "Position")), "UV"));
    assert_eq!(procedural(h).procedural.space, ProceduralSpace::Uv);
    run(h, "空間（トライプラナー）", &|h| choose(h, &format!("{}: UV", t("空間", "Space")), t("トライプラナー", "Triplanar")));
    run(h, "シード", &|h| {
        let field = h
            .get_all_by_role(egui::accesskit::Role::TextInput)
            .map(|n| n.rect())
            .find(|r| column(*r))
            .expect("右の列の数の入力欄はシードだけ");
        click(h, field.center());
        key(h, egui::Key::A, Modifiers::COMMAND);
        h.step();
        h.event(Event::Text("7".into()));
        h.step();
        key(h, egui::Key::Enter, Modifiers::NONE);
        h.run();
    });
    assert_eq!(procedural(h).procedural.seed, 7);
    run(h, "振り直す", &|h| {
        h.get_by_label(t("振り直す", "Reroll")).click();
        h.run();
    });
    for label in [
        t("模様の大きさ", "Pattern Size"),
        t("にじみ", "Bleed"),
        t("トライプラナーの幅", "Triplanar Width"),
        &format!("{} X", t("回転", "Rotation")),
        &format!("{} Y", t("回転", "Rotation")),
        &format!("{} Z", t("回転", "Rotation")),
    ] {
        run(h, label, &|h| slide(h, label));
    }
    match kind {
        Kind::Noise => {
            run(h, "基底", &|h| choose(h, &format!("{}: Perlin", t("基底", "Basis")), "Worley"));
            assert_eq!(procedural(h).procedural.basis, NoiseBasis::Worley);
            run(h, "セルの出力", &|h| choose(h, &format!("{}: F1", t("セルの出力", "Cell Output")), "F2−F1"));
            assert_eq!(procedural(h).procedural.cell_output, CellOutput::F2MinusF1);
            run(h, "重ね方", &|h| choose(h, &format!("{}: fBm", t("重ね方", "Fractal")), "ridged"));
            assert_eq!(procedural(h).procedural.fractal, FractalMode::Ridged);
            // オクターブは初期値（5）が 1〜8 の真ん中より少し上なので、手前へ動かす
            run(h, "オクターブ", &|h| slide_to(h, t("オクターブ", "Octaves"), 50.0));
            for label in [t("ラクナリティ", "Lacunarity"), t("ゲイン", "Gain")] {
                run(h, label, &|h| slide(h, label));
            }
            // Worley から戻すとセルの出力も既定へ戻り、core に断られない
            run(h, "基底（戻す）", &|h| choose(h, &format!("{}: Worley", t("基底", "Basis")), "Perlin"));
            assert_eq!(procedural(h).procedural.cell_output, CellOutput::F1);
        }
        _ => {
            run(h, "プリセット", &|h| {
                h.get_by_label(t("布目", "Weave")).click();
                h.run();
            });
        }
    }
    // 範囲・反転・合成・強さ（上限を先に動かすので、下限も動く）
    for label in [t("上限", "High"), t("下限", "Low"), t("やわらかさ", "Softness")] {
        run(h, label, &|h| slide(h, label));
    }
    run(h, "反転", &|h| {
        let r = rect_of(h, t("反転", "Invert"), column);
        click(h, pos2(r.left() + 8.0, r.center().y));
    });
    assert!(procedural(h).invert);
    run(h, "合成", &|h| choose(h, &format!("{}: {}", t("合成", "Combine"), t("乗算", "Multiply")), t("加算", "Add")));
    let (steps, strength) = (h.state().state.doc.undo_count(), selected_strength(h));
    slide(h, t("強さ", "Strength"));
    assert_ne!(selected_strength(h), strength);
    assert_eq!(h.state().state.doc.undo_count(), steps + 1);
    assert!(h.state().state.message.is_empty(), "{}", h.state().state.message);
}

fn selected_strength(h: &Harness<'_, YoluApp>) -> f64 {
    let (_, effect, _) = h.state().state.fx.filter(&h.state().state.doc).unwrap();
    effect.strength()
}

#[test]
fn procedural_panels_have_no_breakup_rows_and_every_visible_control_is_accepted() {
    for lang in Lang::ALL {
        for kind in [Kind::Noise, Kind::Grunge] {
            let mut h = app(1280.0, 3200.0, 32);
            h.state_mut().state.set_language(lang);
            fx(&mut h, FxOp::AddGenerator { target: FilterTarget::Content, kind });
            if kind == Kind::Grunge {
                while !yolu_app::panels::grunge_picker::ready() { std::thread::sleep(std::time::Duration::from_millis(10)); }
                h.run();
            }
            // 崩しの組（量・シード・大きさ・置き場）は、重ねるノイズを持たない種類には出さない（core が断るので、同じ名前のシードも 1 つだけ）
            let texts = shown_texts(&h);
            let column: Vec<&String> = texts.iter().filter(|(_, r)| r.left() > 1000.0).map(|(t, _)| t).collect();
            for gone in [lang.pick("崩し", "Breakup"), lang.pick("量", "Amount"), lang.pick("置き場", "Placed")] {
                assert!(!column.iter().any(|t| t.as_str() == gone || t.starts_with(&format!("{gone}: "))), "{lang:?} {kind:?}: {gone}");
            }
            assert!(!column.iter().any(|t| t.as_str() == lang.pick("大きさ", "Size")), "{lang:?} {kind:?}: 崩しの大きさ");
            assert_eq!(column.iter().filter(|t| t.as_str() == lang.pick("シード", "Seed")).count(), 1, "{lang:?} {kind:?}");
            touch_every_procedural_control(&mut h, lang, kind);
        }
    }
}

#[test]
fn shape_generator_edit_button_toggles_the_3d_target() {
    let mut h = app(1280.0, 1800.0, 32);
    apply(&mut h, Action::LoadDemoModel);
    fx(&mut h, FxOp::AddGenerator { target: FilterTarget::Content, kind: Kind::ShapeGradient });
    let Some(Selected::Filter { layer, id }) = selected(&h) else { panic!("選択") };
    let steps = h.state().state.doc.undo_count();
    h.get_by_label("3D ビューで編集").click(); h.run();
    assert_eq!(h.state().state.fillfx.edit_filter, Some((layer, id)));
    assert_eq!(yolu_app::fillfx::gizmo::target(&h.state().state), Some(yolu_app::fillfx::gizmo::Target::Filter(layer, id)));
    h.get_by_label("3D ビューで編集").click(); h.run();
    assert_eq!(h.state().state.fillfx.edit_filter, None);
    assert_eq!(h.state().state.doc.undo_count(), steps);
}

#[test]
fn snapshot_procedural_panels_in_both_languages() {
    let mut results = SnapshotResults::new();
    for lang in Lang::ALL {
        for (kind, name) in [(Kind::Noise, "noise"), (Kind::Grunge, "grunge")] {
            let mut h = app(1280.0, 1800.0, 32);
            h.state_mut().state.set_language(lang);
            fx(&mut h, FxOp::AddGenerator { target: FilterTarget::Content, kind });
            if kind == Kind::Grunge {
                while !yolu_app::panels::grunge_picker::ready() { std::thread::sleep(std::time::Duration::from_millis(10)); }
                h.run();
            }
            h.snapshot(format!("fx_procedural_{name}_{}", if lang == Lang::Ja { "ja" } else { "en" }));
            results.extend_harness(&mut h);
        }
    }
    results.unwrap();
}

#[test]
fn procedural_fallback_mark_explains_uv_in_both_languages() {
    for lang in Lang::ALL {
        for target in [FilterTarget::Content, FilterTarget::Mask] {
            let mut h = app(1280.0, 1800.0, 32);
            h.state_mut().state.set_language(lang);
            let layer = h.state().state.selected_layer.unwrap();
            if target == FilterTarget::Mask { apply(&mut h, Action::M2(Edit::AddMask(layer))); }
            fx(&mut h, FxOp::AddGenerator { target, kind: Kind::Noise });
            let (_, effect, _) = h.state().state.fx.filter(&h.state().state.doc).unwrap();
            let label = yolu_app::fx::names::effect_label(lang, effect, target, h.state().state.m2.paint_channel,
                |c| yolu_app::m2::channel_name(lang, &h.state().state.doc, c));
            let row = h.get_by_label(&label).rect();
            move_to(&h, pos2(row.right() - 74.0, row.center().y));
            for _ in 0..90 { h.step(); }
            let reason = lang.pick("UV の空間", "evaluated in UV space");
            assert!(shown_texts(&h).iter().any(|(t, r)| t.contains(reason) && r.top() > row.top() && r.top() < row.top() + 90.0));
        }
    }
}
