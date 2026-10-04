//! 見た目の設定の欄（プロパティの「マテリアル」のタブ）: 種類の切り替え・節の値・テクスチャのスロット・ひな形・Undo（egui_kittest）。
mod common;

use common::*;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::lang::Lang;
use yolu_app::state::Action;
use yolu_app::look::LookOp;
use yolu_app::ui::widgets::{NumberFormat, SliderSpec};
use yolu_app::YoluApp;
use yolu_core::look::{LookKind, LookValue, MaterialLook, PlaneSource, TextureSource};
use yolu_core::{Channel, ChannelInfo, ChannelKind, ColorSpace, Rgba8};

fn harness(lang: Lang) -> Harness<'static, YoluApp> {
    harness_sized(lang, 1600.0)
}

/// 欄の下の方の行まで窓に入る高さで（右の欄は窓の高さで切れる）。
fn harness_sized(lang: Lang, height: f32) -> Harness<'static, YoluApp> {
    let mut h = app(1500.0, height, 64);
    h.state_mut().state.lang = lang;
    // 描く文脈のマテリアルのタブ
    h.state_mut().state.property_tab = 1;
    h.run();
    h
}

fn click_label(h: &mut Harness<'_, YoluApp>, label: &str) {
    let at = h.get_by_label(label).rect().center();
    click(h, at);
}

fn pick(h: &mut Harness<'_, YoluApp>, label: &str) {
    let at = popup_item(h, label).center();
    click(h, at);
}

#[test]
fn switching_to_liltoon_assigns_the_standard_channels_and_undo_takes_it_back() {
    let mut h = harness(Lang::Ja);
    click_label(&mut h, "種類: 標準（PBR）");
    pick(&mut h, "lilToon");
    let look = h.state().state.doc.look().clone();
    assert_eq!(look.kind, LookKind::LilToon);
    assert_eq!(look.textures["_MainTex"], TextureSource::Channel(Channel::Color));
    assert_eq!(look.textures["_BumpMap"], TextureSource::Channel(Channel::Normal));
    assert!(h.state().state.modified);
    h.state_mut().apply(Action::Undo);
    h.run();
    assert!(h.state().state.doc.look().is_default());
}

#[test]
fn the_shadow_section_toggles_and_a_slider_drag_is_one_undo_step() {
    let mut h = harness(Lang::Ja);
    h.state_mut()
        .apply(Action::Look(yolu_app::look::LookOp::Kind(LookKind::LilToon)));
    h.run();
    click_label(&mut h, "影設定");
    click_label(&mut h, "影");
    assert!(yolu_app::look::liltoon::on(h.state().state.doc.look(), "_UseShadow"));
    let steps = h.state().state.doc.undo_count();
    // 「範囲」のスライダー（影の 1 影）をドラッグ
    let r = h
        .get_all_by_label("範囲")
        .map(|n| n.rect())
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .expect("範囲");
    let y = r.bottom() - 4.0;
    drag(
        &mut h,
        &[egui::pos2(r.left() + 10.0, y), egui::pos2(r.left() + 40.0, y), egui::pos2(r.left() + 80.0, y)],
    );
    let after = h.state().state.doc.undo_count();
    assert_eq!(after, steps + 1, "ドラッグは 1 段");
    let border = yolu_app::look::liltoon::number(h.state().state.doc.look(), "_ShadowBorder");
    assert!((border - 0.5).abs() > 1e-3, "{border}");
    h.state_mut().apply(Action::Undo);
    h.run();
    assert_eq!(
        yolu_app::look::liltoon::number(h.state().state.doc.look(), "_ShadowBorder"),
        0.5
    );
}

#[test]
fn the_template_button_makes_channels_in_one_undo() {
    let mut h = harness(Lang::Ja);
    h.state_mut()
        .apply(Action::Look(yolu_app::look::LookOp::Kind(LookKind::LilToon)));
    h.run();
    let steps = h.state().state.doc.undo_count();
    click_label(&mut h, "lilToon のひな形");
    let doc = &h.state().state.doc;
    assert_eq!(doc.undo_count(), steps + 1);
    let users: Vec<Channel> = doc.channels().into_iter().filter(|c| !c.is_standard()).collect();
    assert_eq!(users.len(), 3, "影の強度・影色・AO");
    assert_eq!(
        doc.look().textures["_ShadowStrengthMask"],
        TextureSource::Channel(users[0])
    );
    // 影の節のスロットの行に、割り当てたチャンネルの名前が出る
    click_label(&mut h, "影設定");
    assert!(h.query_by_label("影の強度マスク: 影の強度").is_some());
    // スロットをほかのチャンネルへ
    click_label(&mut h, "影の強度マスク: 影の強度");
    pick(&mut h, "ラフネス");
    assert_eq!(
        h.state().state.doc.look().textures["_ShadowStrengthMask"],
        TextureSource::Channel(Channel::Roughness)
    );
}

fn texts(shape: &egui::epaint::Shape, out: &mut Vec<(egui::Pos2, String)>) {
    match shape {
        egui::epaint::Shape::Vec(shapes) => {
            for s in shapes {
                texts(s, out);
            }
        }
        egui::epaint::Shape::Text(t) => out.push((t.pos, t.galley.job.text.clone())),
        _ => {}
    }
}

#[test]
fn the_panel_draws_in_both_languages_with_names_only() {
    let mut snapshots = egui_kittest::SnapshotResults::new();
    for lang in Lang::ALL {
        let mut h = harness(lang);
        h.state_mut()
            .apply(Action::Look(yolu_app::look::LookOp::Template));
        h.run();
        for (ja, en) in [("基本設定", "Base Setting"), ("影設定", "Shadow"), ("リムライト設定", "Rim Light")] {
            click_label(&mut h, lang.pick(ja, en));
        }
        h.run();
        let header = h.get_by_label(lang.pick("見た目", "Look")).rect();
        let mut all = Vec::new();
        for shape in &h.output().shapes {
            texts(&shape.shape, &mut all);
        }
        let mine: Vec<String> = all
            .into_iter()
            .filter(|(p, _)| p.x >= header.left() - 4.0 && p.x <= header.right() + 4.0 && p.y >= header.top() - 2.0)
            .map(|(_, s)| s)
            .collect();
        assert!(mine.iter().any(|s| s.contains(lang.pick("影設定", "Shadow"))), "{mine:?}");
        for text in &mine {
            assert_plain("見た目の欄", text);
            if lang == Lang::En {
                assert!(!has_japanese(text), "{text}");
            }
        }
        h.snapshot(format!("liltoon_panel_{}", lang.pick("ja", "en")));
        snapshots.extend_harness(&mut h);
    }
}

fn set_value(h: &mut Harness<'_, YoluApp>, name: &'static str, value: f32) {
    h.state_mut().apply(Action::Look(LookOp::Value {
        name,
        value: LookValue::Float(value),
        drag: false,
    }));
}

#[test]
fn a_property_shown_in_two_sections_has_its_own_slider_in_each() {
    // lilToon のインスペクターと同じく、影色への環境光影響度はライティング設定と影設定の両方に出る。両方を開いても欄の ID が重ならない
    // （重なると 1 つの欄として扱われ、片方のドラッグがもう片方に移る）
    let mut h = harness_sized(Lang::Ja, 4400.0);
    h.state_mut().apply(Action::Look(LookOp::Kind(LookKind::LilToon)));
    set_value(&mut h, "_UseShadow", 1.0);
    h.run();
    click_label(&mut h, "ライティング設定");
    click_label(&mut h, "影設定");
    let sliders: Vec<egui::Rect> = h.get_all_by_label("影色への環境光影響度").map(|n| n.rect()).collect();
    assert_eq!(sliders.len(), 2, "{sliders:?}");
    // 下（影設定）の欄の右端を押すと 1 になり、上（ライティング設定）の欄も同じ値を見せる
    fn lower<'a>(h: &'a Harness<'_, YoluApp>) -> egui_kittest::Node<'a> {
        h.get_all_by_label("影色への環境光影響度")
            .max_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
            .expect("2 つある")
    }
    let r = lower(&h).rect();
    assert!(r.bottom() < h.ctx.content_rect().bottom(), "窓に入っている: {r:?}");
    click(&mut h, egui::pos2(r.right() - 2.0, r.bottom() - 4.0));
    let value = yolu_app::look::liltoon::number(h.state().state.doc.look(), "_ShadowEnvStrength");
    assert!(value > 0.95, "右端の近く: {value}");
    h.run();
    let shown: Vec<Option<f64>> = h
        .get_all_by_label("影色への環境光影響度")
        .map(|n| n.accesskit_node().numeric_value())
        .collect();
    assert_eq!(shown.len(), 2);
    for v in shown {
        assert!((v.expect("値") - value as f64).abs() < 1e-6, "両方の欄が同じ値: {v:?} {value}");
    }
}

#[test]
fn a_power_slider_spreads_the_small_values_like_unity() {
    // Unity の PowerSlider: 溝の位置は値の 1/power 乗に比例
    let format = NumberFormat { decimals: 2, trim: false, suffix: "" };
    let spec = SliderSpec::new("x", 0.01, 50.0, format).power(3.0);
    let at = |v: f32| (v.powf(1.0 / 3.0) - 0.01f32.powf(1.0 / 3.0)) / (50.0f32.powf(1.0 / 3.0) - 0.01f32.powf(1.0 / 3.0));
    for v in [0.01, 1.0, 3.5, 10.0, 50.0] {
        assert!((spec.fraction(v) - at(v)).abs() < 1e-5, "{v}");
        assert!((spec.value_at(spec.fraction(v)) - v).abs() < 1e-3 * v.max(1.0), "{v}");
    }
    assert!((spec.fraction(3.5) - 0.376).abs() < 1e-3, "既定の 3.5 は溝の 4 割近く（線形なら 7 %）");
    let linear = SliderSpec::new("y", 0.0, 2.0, format);
    assert_eq!(linear.fraction(0.5), 0.25);
    assert_eq!(linear.value_at(0.25), 0.5);
    // 欄のリムライトの細さ（_RimFresnelPower、0.01〜50、power 3）は、溝の真ん中を押すと約 7.4（線形なら 25）
    let mut h = harness_sized(Lang::Ja, 2400.0);
    h.state_mut().apply(Action::Look(LookOp::Kind(LookKind::LilToon)));
    set_value(&mut h, "_UseRim", 1.0);
    h.run();
    click_label(&mut h, "リムライト設定");
    let r = h.get_by_label("リムライトの細さ").rect();
    assert!(r.bottom() < h.ctx.content_rect().bottom(), "窓に入っている: {r:?}");
    click(&mut h, egui::pos2(r.left() + r.width() * 0.5, r.bottom() - 4.0));
    let value = yolu_app::look::liltoon::number(h.state().state.doc.look(), "_RimFresnelPower");
    assert!((value - 7.41).abs() < 0.4, "{value}");
}

#[test]
fn a_slot_past_sixteen_user_channels_says_it_is_not_drawn() {
    let mut h = harness(Lang::Ja);
    let doc = &mut h.state_mut().state.doc;
    let channels: Vec<Channel> = (0..17)
        .map(|i| {
            doc.add_channel(ChannelInfo {
                name: format!("マスク {i}"),
                kind: ChannelKind::Scalar,
                color_space: ColorSpace::Linear,
                default: Rgba8::new(255, 255, 255, 255),
            })
            .unwrap()
        })
        .collect();
    let mut look = MaterialLook {
        kind: LookKind::LilToon,
        ..MaterialLook::default()
    };
    look.properties.insert("_UseShadow".into(), LookValue::Float(1.0));
    // スロットの並びで先の 4 つが 4 チャンネルずつ（16 個）、5 つ目のスロットが 17 個目
    for (k, slot) in ["_ShadowStrengthMask", "_ShadowBorderMask", "_ShadowBlurMask", "_ShadowColorTex"].iter().enumerate() {
        let planes = [0, 1, 2, 3].map(|j| PlaneSource::Channel { channel: channels[k * 4 + j], component: 0 });
        look.textures.insert((*slot).into(), TextureSource::Packed(planes));
    }
    look.textures.insert("_Shadow2ndColorTex".into(), TextureSource::Channel(channels[16]));
    doc.set_look(look, false).unwrap();
    h.run();
    let doc = &h.state().state.doc;
    assert!(!yolu_app::look::panel::slot_over_layer_limit(doc, "_ShadowColorTex"));
    assert!(yolu_app::look::panel::slot_over_layer_limit(doc, "_Shadow2ndColorTex"));
    click_label(&mut h, "影設定");
    assert!(h.query_by_label("2影色: マスク 16（描かない）").is_some());
    assert_eq!(h.query_all_by_label_contains("描かない").count(), 1, "16 個までのスロットには出ない");
    // 先のスロットの割り当てを外すと、17 個目も描ける（印が消える）
    let mut look = h.state().state.doc.look().clone();
    look.textures.remove("_ShadowBlurMask");
    h.state_mut().state.doc.set_look(look, false).unwrap();
    h.run();
    assert!(h.query_by_label("2影色: マスク 16").is_some());
    assert_eq!(h.query_all_by_label_contains("描かない").count(), 0);
}
