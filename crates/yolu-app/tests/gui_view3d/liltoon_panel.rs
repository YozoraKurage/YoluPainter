//! 見た目の設定の欄（右のドックの「マテリアル」のパネル）: 種類の切り替え・節の値・テクスチャのスロット・ひな形・Undo（egui_kittest）。
use crate::common;

use common::*;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::lang::Lang;
use yolu_app::look::LookOp;
use yolu_app::state::Action;
use yolu_app::ui::widgets::{NumberFormat, SliderSpec};
use yolu_app::YoluApp;
use yolu_core::look::{LookKind, LookValue, MaterialLook, PlaneSource, TextureSource};
use yolu_core::{Channel, ChannelInfo, ChannelKind, ColorSpace, Rgba8};

fn harness(lang: Lang) -> Harness<'static, YoluApp> {
    harness_sized(lang, 1600.0)
}

/// 欄の下の方の行までウィンドウに入る高さで（右の欄はウィンドウの高さで切れる。マテリアルのパネルはプロパティの組の中なので、列の高さのうち組の取り分の分だけ高くしてある）。
fn harness_sized(lang: Lang, height: f32) -> Harness<'static, YoluApp> {
    let mut h = app(1500.0, height * 1.4, 64);
    h.state_mut().state.lang = lang;
    // 右のドックの「マテリアル」のパネル
    open_material(&mut h);
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
fn a_new_set_starts_as_liltoon_and_switching_the_kind_is_one_undo() {
    let mut h = harness(Lang::Ja);
    // 新しいセットの既定は lilToon（メインカラー・ノーマルマップ・発光を同じ名前のチャンネルに）。履歴には入らない
    let look = h.state().state.doc.look().clone();
    assert_eq!(look.kind, LookKind::LilToon);
    assert_eq!(
        look.textures["_MainTex"],
        TextureSource::Channel(Channel::Color)
    );
    assert_eq!(
        look.textures["_BumpMap"],
        TextureSource::Channel(Channel::Normal)
    );
    assert_eq!(
        look.textures["_EmissionMap"],
        TextureSource::Channel(Channel::Emission)
    );
    assert_eq!(h.state().state.doc.undo_count(), 0);
    assert!(!h.state().state.modified);
    click_label(&mut h, "種類: lilToon");
    pick(&mut h, "標準（PBR）");
    assert_eq!(h.state().state.doc.look().kind, LookKind::Standard);
    assert!(h.state().state.modified);
    h.state_mut().apply(Action::Undo);
    h.run();
    assert_eq!(h.state().state.doc.look(), &look);
}

#[test]
fn switching_a_standard_set_to_liltoon_assigns_the_standard_channels_and_undo_takes_it_back() {
    let mut h = harness(Lang::Ja);
    h.state_mut()
        .state
        .doc
        .restore_look(MaterialLook::default())
        .unwrap();
    h.run();
    click_label(&mut h, "種類: 標準（PBR）");
    pick(&mut h, "lilToon");
    let look = h.state().state.doc.look().clone();
    assert_eq!(look.kind, LookKind::LilToon);
    assert_eq!(
        look.textures["_MainTex"],
        TextureSource::Channel(Channel::Color)
    );
    assert_eq!(
        look.textures["_BumpMap"],
        TextureSource::Channel(Channel::Normal)
    );
    assert!(h.state().state.modified);
    h.state_mut().apply(Action::Undo);
    h.run();
    assert!(h.state().state.doc.look().is_default());
}

#[test]
fn the_shadow_section_toggles_and_a_slider_drag_is_one_undo_step() {
    let mut h = harness(Lang::Ja);
    h.state_mut()
        .apply(Action::Look(yolu_app::look::LookOp::Kind(
            LookKind::LilToon,
        )));
    h.run();
    click_label(&mut h, "影設定");
    click_label(&mut h, "影");
    assert!(yolu_app::look::liltoon::on(
        h.state().state.doc.look(),
        "_UseShadow"
    ));
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
        &[
            egui::pos2(r.left() + 10.0, y),
            egui::pos2(r.left() + 40.0, y),
            egui::pos2(r.left() + 80.0, y),
        ],
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
        .apply(Action::Look(yolu_app::look::LookOp::Kind(
            LookKind::LilToon,
        )));
    h.run();
    let steps = h.state().state.doc.undo_count();
    click_label(&mut h, "lilToon のひな形");
    let doc = &h.state().state.doc;
    assert_eq!(doc.undo_count(), steps + 1);
    let users: Vec<Channel> = doc
        .channels()
        .into_iter()
        .filter(|c| !c.is_standard())
        .collect();
    assert_eq!(users.len(), 3, "影の強度・影色・AO");
    assert_eq!(
        doc.look().textures["_ShadowStrengthMask"],
        TextureSource::Channel(users[0])
    );
    // 影の節のスロットの行に、割り当てたチャンネルの名前が出る
    click_label(&mut h, "影設定");
    assert!(h.query_by_label("マスクと強度: 影の強度").is_some());
    // スロットをほかのチャンネルへ
    click_label(&mut h, "マスクと強度: 影の強度");
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
        for (ja, en) in [
            ("基本設定", "Base Setting"),
            ("影設定", "Shadow"),
            ("リムライト設定", "Rim Light"),
        ] {
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
            .filter(|(p, _)| {
                p.x >= header.left() - 4.0
                    && p.x <= header.right() + 4.0
                    && p.y >= header.top() - 2.0
            })
            .map(|(_, s)| s)
            .collect();
        assert!(
            mine.iter()
                .any(|s| s.contains(lang.pick("影設定", "Shadow"))),
            "{mine:?}"
        );
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
    h.state_mut()
        .apply(Action::Look(LookOp::Kind(LookKind::LilToon)));
    set_value(&mut h, "_UseShadow", 1.0);
    h.run();
    click_label(&mut h, "ライティング・明るさ設定");
    click_label(&mut h, "影設定");
    let sliders: Vec<egui::Rect> = h
        .get_all_by_label("影色への環境光影響度")
        .map(|n| n.rect())
        .collect();
    assert_eq!(sliders.len(), 2, "{sliders:?}");
    // 下（影設定）の欄の右端を押すと 1 になり、上（ライティング設定）の欄も同じ値を見せる
    fn lower<'a>(h: &'a Harness<'_, YoluApp>) -> egui_kittest::Node<'a> {
        h.get_all_by_label("影色への環境光影響度")
            .max_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
            .expect("2 つある")
    }
    let r = lower(&h).rect();
    assert!(
        r.bottom() < h.ctx.content_rect().bottom(),
        "ウィンドウに入っている: {r:?}"
    );
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
        assert!(
            (v.expect("値") - value as f64).abs() < 1e-6,
            "両方の欄が同じ値: {v:?} {value}"
        );
    }
}

#[test]
fn a_power_slider_spreads_the_small_values_like_unity() {
    // Unity の PowerSlider: 溝の位置は値の 1/power 乗に比例
    let format = NumberFormat {
        decimals: 2,
        trim: false,
        suffix: "",
    };
    let spec = SliderSpec::new("x", 0.01, 50.0, format).power(3.0);
    let at = |v: f32| {
        (v.powf(1.0 / 3.0) - 0.01f32.powf(1.0 / 3.0))
            / (50.0f32.powf(1.0 / 3.0) - 0.01f32.powf(1.0 / 3.0))
    };
    for v in [0.01, 1.0, 3.5, 10.0, 50.0] {
        assert!((spec.fraction(v) - at(v)).abs() < 1e-5, "{v}");
        assert!(
            (spec.value_at(spec.fraction(v)) - v).abs() < 1e-3 * v.max(1.0),
            "{v}"
        );
    }
    assert!(
        (spec.fraction(3.5) - 0.376).abs() < 1e-3,
        "既定の 3.5 は溝の 4 割近く（線形なら 7 %）"
    );
    let linear = SliderSpec::new("y", 0.0, 2.0, format);
    assert_eq!(linear.fraction(0.5), 0.25);
    assert_eq!(linear.value_at(0.25), 0.5);
    // 欄のリムライトの細さ（_RimFresnelPower、0.01〜50、power 3）は、溝の真ん中を押すと約 7.4（線形なら 25）
    let mut h = harness_sized(Lang::Ja, 3200.0);
    h.state_mut()
        .apply(Action::Look(LookOp::Kind(LookKind::LilToon)));
    set_value(&mut h, "_UseRim", 1.0);
    h.run();
    click_label(&mut h, "リムライト設定");
    let r = h.get_by_label("リムライトの細さ").rect();
    assert!(
        r.bottom() < h.ctx.content_rect().bottom(),
        "ウィンドウに入っている: {r:?}"
    );
    click(
        &mut h,
        egui::pos2(r.left() + r.width() * 0.5, r.bottom() - 4.0),
    );
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
    look.properties
        .insert("_UseShadow".into(), LookValue::Float(1.0));
    // スロットの並びで先の 4 つが 4 チャンネルずつ（16 個）、5 つ目のスロットが 17 個目
    for (k, slot) in [
        "_ShadowStrengthMask",
        "_ShadowBorderMask",
        "_ShadowBlurMask",
        "_ShadowColorTex",
    ]
    .iter()
    .enumerate()
    {
        let planes = [0, 1, 2, 3].map(|j| PlaneSource::Channel {
            channel: channels[k * 4 + j],
            component: 0,
        });
        look.textures
            .insert((*slot).into(), TextureSource::Packed(planes));
    }
    look.textures.insert(
        "_Shadow2ndColorTex".into(),
        TextureSource::Channel(channels[16]),
    );
    doc.set_look(look, false).unwrap();
    h.run();
    let doc = &h.state().state.doc;
    assert!(!yolu_app::look::panel::slot_over_layer_limit(
        doc,
        "_ShadowColorTex"
    ));
    assert!(yolu_app::look::panel::slot_over_layer_limit(
        doc,
        "_Shadow2ndColorTex"
    ));
    click_label(&mut h, "影設定");
    assert!(h.query_by_label("影色2: マスク 16（描かない）").is_some());
    assert_eq!(
        h.query_all_by_label_contains("描かない").count(),
        1,
        "16 個までのスロットには出ない"
    );
    // 先のスロットの割り当てを外すと、17 個目も描ける（印が消える）
    let mut look = h.state().state.doc.look().clone();
    look.textures.remove("_ShadowBlurMask");
    h.state_mut().state.doc.set_look(look, false).unwrap();
    h.run();
    assert!(h.query_by_label("影色2: マスク 16").is_some());
    assert_eq!(h.query_all_by_label_contains("描かない").count(), 0);
}

/// Live Link で Unity から受けた値（試験が文書へ直に入れる）。影は入、範囲 0.25、影色、マットキャップの絵は届いた・影色の絵は予算超え。
fn received_from_unity() -> yolu_core::look::ReceivedLook {
    let mut look = MaterialLook {
        kind: LookKind::LilToon,
        shader: "Hidden/lilToonOutline".into(),
        ..MaterialLook::default()
    };
    look.properties
        .insert("_UseShadow".into(), LookValue::Float(1.0));
    look.properties
        .insert("_ShadowBorder".into(), LookValue::Float(0.25));
    look.properties.insert(
        "_ShadowColor".into(),
        LookValue::Color([0.4, 0.3, 0.5, 1.0]),
    );
    look.textures
        .insert("_MainTex".into(), TextureSource::Channel(Channel::Color));
    let mut r = yolu_core::look::ReceivedLook {
        look,
        source: "lilToon 2.3.4 · Standard/Opaque+Outline".into(),
        ..Default::default()
    };
    r.images.insert(
        "_ShadowStrengthMask".into(),
        std::sync::Arc::new(yolu_core::look::ReceivedImage {
            width: 2,
            height: 2,
            srgb: false,
            pixels: vec![255; 16].into(),
        }),
    );
    r.missing.insert(
        "_ShadowColorTex".into(),
        yolu_core::look::MissingImage::OverBudget,
    );
    r
}

#[test]
fn values_from_unity_show_their_row_and_the_changed_items_are_marked() {
    let mut snapshots = egui_kittest::SnapshotResults::new();
    for lang in Lang::ALL {
        let mut h = harness(lang);
        h.state_mut()
            .state
            .doc
            .set_received_look(Some(received_from_unity()))
            .unwrap();
        // 欄で 1 項目だけ変える（範囲）
        h.state_mut().apply(Action::Look(LookOp::Value {
            name: "_ShadowBorder",
            value: LookValue::Float(0.6),
            drag: false,
        }));
        h.run();
        click_label(&mut h, lang.pick("影設定", "Shadow"));
        h.run();
        // 種類は Unity の値の lilToon、行の様子は「最後の値」（Live Link のモデルが無い）
        let _ = h.get_by_label(&format!("{}: lilToon", lang.pick("種類", "Kind")));
        let _ = h.get_by_label(lang.pick("Unity の値: 最後の値", "Unity Values: Last received"));
        // 変えた項目には印、変えていない項目には無い
        assert!(h
            .query_by_label(&format!("{} •", lang.pick("範囲", "Border")))
            .is_some());
        assert!(h
            .query_by_label(&format!("{} •", lang.pick("ぼかし", "Blur")))
            .is_none());
        // 受けた絵のスロットと、送られなかった絵のスロット
        let _ = h.get_by_label(&format!(
            "{}: {}",
            lang.pick("マスクと強度", "Mask & Strength"),
            lang.pick("Unity のテクスチャ", "Unity texture")
        ));
        let header = h.get_by_label(lang.pick("見た目", "Look")).rect();
        let mut all = Vec::new();
        for shape in &h.output().shapes {
            texts(&shape.shape, &mut all);
        }
        for (p, text) in &all {
            if p.x >= header.left() - 4.0
                && p.x <= header.right() + 4.0
                && p.y >= header.top() - 2.0
            {
                assert_plain("見た目の欄（Unity の値）", text);
                if lang == Lang::En {
                    assert!(!has_japanese(text), "{text}");
                }
            }
        }
        h.snapshot(format!("liltoon_panel_unity_{}", lang.pick("ja", "en")));
        snapshots.extend_harness(&mut h);
        // Unity に合わせる: 欄で変えた値が外れ、Unity の値で描く（1 回の Undo）
        let steps = h.state().state.doc.undo_count();
        click_label(&mut h, lang.pick("Unity に合わせる", "Match Unity"));
        h.run();
        assert_eq!(h.state().state.doc.undo_count(), steps + 1);
        assert!(h.state().state.doc.look().properties.is_empty());
        assert_eq!(
            yolu_app::look::liltoon::number(h.state().state.doc.drawn_look(), "_ShadowBorder"),
            0.25
        );
    }
}

#[test]
fn a_unity_texture_past_sixteen_says_it_is_not_drawn() {
    let mut h = harness(Lang::Ja);
    let mut r = received_from_unity();
    r.missing.clear();
    r.look
        .properties
        .insert("_UseRim".into(), LookValue::Float(1.0));
    // 流し込み先（_MainTex）のほかの全部のスロットに Unity のテクスチャ（20 枚）: 並びで 17 枚目から描かない
    for slot in &yolu_app::look::liltoon::SLOTS[1..] {
        r.images.insert(
            slot.name.into(),
            std::sync::Arc::new(yolu_core::look::ReceivedImage {
                width: 2,
                height: 2,
                srgb: false,
                pixels: vec![255; 16].into(),
            }),
        );
    }
    h.state_mut().state.doc.set_received_look(Some(r)).unwrap();
    h.run();
    let doc = &h.state().state.doc;
    assert!(!yolu_app::look::panel::slot_over_received_limit(
        doc,
        "_MatCap2ndTex"
    ));
    assert!(yolu_app::look::panel::slot_over_received_limit(
        doc,
        "_RimColorTex"
    ));
    assert!(!yolu_app::look::panel::slot_over_received_limit(
        doc,
        "_ShadowColorTex"
    ));
    click_label(&mut h, "リムライト設定");
    h.run();
    assert!(h
        .query_by_label("色 / マスク: Unity のテクスチャ（描かない）")
        .is_some());
    assert_eq!(
        h.query_all_by_label_contains("描かない").count(),
        1,
        "開いた節の、16 枚を超えたスロットだけ"
    );
    // 欄で前のスロットを割り当てると、その分だけ後ろのスロットが描ける（印が消える）
    let mut look = h.state().state.doc.look().clone();
    for slot in ["_MainColorAdjustMask", "_AlphaMask"] {
        look.textures
            .insert(slot.into(), TextureSource::Channel(Channel::Color));
    }
    h.state_mut().state.doc.set_look(look, false).unwrap();
    h.run();
    assert!(!yolu_app::look::panel::slot_over_received_limit(
        &h.state().state.doc,
        "_RimColorTex"
    ));
    assert!(h
        .query_by_label("色 / マスク: Unity のテクスチャ")
        .is_some());
    assert_eq!(h.query_all_by_label_contains("描かない").count(), 0);
}

#[test]
fn an_emission_texture_unity_left_empty_shows_the_default_texture_like_the_view_draws() {
    for lang in Lang::ALL {
        let mut h = harness_sized(lang, 2400.0);
        // 新しいセットの既定の割り当て（発光 → Emission）は、どのレイヤーも使っていない。Unity の値では発光が入で、発光のテクスチャは空
        let mut r = received_from_unity();
        r.missing.clear();
        r.images.clear();
        r.look
            .properties
            .insert("_UseEmission".into(), LookValue::Float(1.0));
        h.state_mut()
            .state
            .doc
            .set_received_look(Some(r.clone()))
            .unwrap();
        h.run();
        click_label(&mut h, lang.pick("発光設定", "Emission"));
        let row = |h: &Harness<'_, YoluApp>, value: &str| {
            h.query_by_label(&format!(
                "{}: {value}",
                lang.pick("色 / マスク", "Color / Mask")
            ))
            .is_some()
        };
        // 3D ビューは使っていないチャンネル（黒）ではなく Unity と同じ既定の白で読むので、欄もチャンネルの名前でなく既定を見せる
        assert!(row(&h, lang.pick("白", "White")), "{lang:?}");
        // 絵が届けば Unity のテクスチャ
        r.images.insert(
            "_EmissionMap".into(),
            std::sync::Arc::new(yolu_core::look::ReceivedImage {
                width: 2,
                height: 2,
                srgb: true,
                pixels: vec![255; 16].into(),
            }),
        );
        h.state_mut().state.doc.set_received_look(Some(r)).unwrap();
        h.run();
        assert!(
            row(&h, lang.pick("Unity のテクスチャ", "Unity texture")),
            "{lang:?}"
        );
        // Live Link でつないでいない（受けた見た目が無い）セットは今までどおり、割り当てたチャンネルの名前
        h.state_mut().state.doc.set_received_look(None).unwrap();
        h.state_mut().apply(Action::Look(LookOp::Value {
            name: "_UseEmission",
            value: LookValue::Float(1.0),
            drag: false,
        }));
        h.run();
        assert!(!row(&h, lang.pick("白", "White")), "{lang:?}");
        assert!(row(&h, lang.pick("エミッション", "Emission")), "{lang:?}");
    }
}

#[test]
fn the_rendering_mode_and_outline_changed_here_are_marked_and_the_section_resets_follow_unity() {
    for lang in Lang::ALL {
        let mut h = harness_sized(lang, 3200.0);
        // Unity の値: 不透明・輪郭線あり（Hidden/lilToonOutline）
        h.state_mut()
            .state
            .doc
            .set_received_look(Some(received_from_unity()))
            .unwrap();
        h.run();
        // 輪郭線の入切は輪郭線設定の節の頭（基本設定には無い）
        click_label(&mut h, lang.pick("輪郭線設定", "Outline"));
        h.run();
        let (mode, outline) = (
            lang.pick("描画モード", "Rendering Mode"),
            lang.pick("輪郭線", "Outline"),
        );
        let marked = |h: &Harness<'_, YoluApp>, label: &str| {
            h.query_all_by_label_contains(&format!("{label} •")).count() > 0
        };
        assert!(
            !marked(&h, mode) && !marked(&h, outline),
            "変えていなければ印は無い"
        );
        // 欄で描画モードを変える: 描画モードだけに印（輪郭線は Unity と同じ）
        h.state_mut().apply(Action::Look(LookOp::Mode(
            yolu_app::look::liltoon::RenderMode::Cutout,
        )));
        h.run();
        assert!(marked(&h, mode) && !marked(&h, outline));
        // 輪郭線も切る: 両方に印
        h.state_mut().apply(Action::Look(LookOp::Outline(false)));
        h.run();
        assert!(marked(&h, mode) && marked(&h, outline));
        // 基本設定の節を既定に戻すと、描画モードが Unity の値（1 回の Undo）。輪郭線は欄の値のまま
        let steps = h.state().state.doc.undo_count();
        h.state_mut()
            .apply(Action::Look(LookOp::Reset(yolu_app::look::Section::Base)));
        h.run();
        assert_eq!(h.state().state.doc.undo_count(), steps + 1);
        let info = yolu_app::look::liltoon::shader_info(h.state().state.doc.drawn_look());
        assert_eq!(
            (info.mode, info.outline),
            (yolu_app::look::liltoon::RenderMode::Opaque, false)
        );
        assert!(!marked(&h, mode) && marked(&h, outline));
        // 輪郭線設定の節を既定に戻すと、輪郭線も Unity の値。利用者の設定からシェーダーの名前が外れる
        h.state_mut().apply(Action::Look(LookOp::Reset(
            yolu_app::look::Section::Outline,
        )));
        h.run();
        assert!(h.state().state.doc.look().shader.is_empty());
        let info = yolu_app::look::liltoon::shader_info(h.state().state.doc.drawn_look());
        assert_eq!(
            (info.mode, info.outline),
            (yolu_app::look::liltoon::RenderMode::Opaque, true)
        );
        assert!(!marked(&h, mode) && !marked(&h, outline));
        // 受けた値の無いセットでは、既定（不透明・輪郭線なし）に戻る
        h.state_mut().state.doc.set_received_look(None).unwrap();
        h.state_mut()
            .apply(Action::Look(LookOp::Kind(LookKind::LilToon)));
        h.state_mut().apply(Action::Look(LookOp::Mode(
            yolu_app::look::liltoon::RenderMode::Transparent,
        )));
        h.state_mut().apply(Action::Look(LookOp::Outline(true)));
        h.state_mut()
            .apply(Action::Look(LookOp::Reset(yolu_app::look::Section::Base)));
        let info = yolu_app::look::liltoon::shader_info(h.state().state.doc.drawn_look());
        assert_eq!(
            (info.mode, info.outline),
            (yolu_app::look::liltoon::RenderMode::Opaque, true)
        );
        h.state_mut().apply(Action::Look(LookOp::Reset(
            yolu_app::look::Section::Outline,
        )));
        let info = yolu_app::look::liltoon::shader_info(h.state().state.doc.drawn_look());
        assert_eq!(
            (info.mode, info.outline),
            (yolu_app::look::liltoon::RenderMode::Opaque, false)
        );
        assert!(h.state().state.doc.look().shader.is_empty());
    }
}

/// 節の見出し（開け閉めの見出し）の上からの並び。
fn headers_in_order(h: &Harness<'_, YoluApp>, names: &[&str]) -> Vec<f32> {
    names
        .iter()
        .map(|n| {
            h.get_all_by_label(n)
                .map(|node| node.rect().top())
                .min_by(|a, b| a.total_cmp(b))
                .unwrap_or_else(|| panic!("見出し {n} が無い"))
        })
        .collect()
}

#[test]
fn the_sections_follow_the_liltoon_inspector_and_the_outline_has_its_own_section() {
    for lang in Lang::ALL {
        let mut h = harness_sized(lang, 3200.0);
        let names: Vec<&str> = [
            ("描画モード: 不透明", "Rendering Mode: Opaque"),
            ("基本設定", "Base Setting"),
            ("ライティング・明るさ設定", "Lighting"),
            ("UV設定", "UV Setting"),
            ("メインカラー / 透過設定", "Main Color / Alpha"),
            ("影設定", "Shadow"),
            ("リムシェード", "RimShade"),
            ("発光設定", "Emission"),
            ("ノーマルマップ設定", "Normal Map"),
            ("逆光ライト", "Backlight"),
            ("光沢設定", "Reflections"),
            ("マットキャップ設定", "MatCap"),
            ("リムライト設定", "Rim Light"),
            ("ラメ設定", "Glitter"),
            ("輪郭線設定", "Outline"),
            ("距離フェード", "Distance Fade"),
        ]
        .iter()
        .map(|(ja, en)| lang.pick(*ja, *en))
        .collect();
        let tops = headers_in_order(&h, &names);
        for k in 1..tops.len() {
            assert!(
                tops[k] > tops[k - 1],
                "{} が {} より下: {tops:?}",
                names[k],
                names[k - 1]
            );
        }
        // 組の見出し（lilToon の太字の見出し）は、その組の最初の節のすぐ上
        let mut all = Vec::new();
        for shape in &h.output().shapes {
            texts(&shape.shape, &mut all);
        }
        let look = h.get_by_label(lang.pick("見た目", "Look")).rect();
        // 見た目の欄の中の、ある見出しより下のいちばん上のその文言
        let label_below = |text: &str, after: f32| {
            all.iter()
                .filter(|(p, s)| s == text && p.x >= look.left() - 4.0 && p.y > after)
                .map(|(p, _)| p.y)
                .min_by(|a, b| a.total_cmp(b))
                .unwrap_or_else(|| panic!("組の見出し {text} が無い"))
        };
        for (group, first, before) in [
            (lang.pick("色設定", "Color"), 4, 3),
            (
                lang.pick("ノーマルマップ・光沢設定", "Normal Map & Reflection"),
                8,
                7,
            ),
            (lang.pick("拡張設定", "Advanced"), 14, 13),
        ] {
            let y = label_below(group, tops[before]);
            assert!(y > tops[before] && y < tops[first], "{group}: {y} {tops:?}");
        }
        // 基本設定に輪郭線の入切は無い。輪郭線設定の節の頭にある
        click_label(&mut h, lang.pick("基本設定", "Base Setting"));
        let outline_toggle = lang.pick("輪郭線", "Outline");
        let toggles = |h: &Harness<'_, YoluApp>| {
            h.query_all_by_label(outline_toggle)
                .filter(|n| n.accesskit_node().role() == egui::accesskit::Role::CheckBox)
                .count()
        };
        assert_eq!(toggles(&h), 0, "基本設定を開いても輪郭線の入切は出ない");
        click_label(&mut h, lang.pick("輪郭線設定", "Outline"));
        assert_eq!(toggles(&h), 1);
        let header = h
            .get_all_by_label(lang.pick("輪郭線設定", "Outline"))
            .find(|n| n.accesskit_node().role() != egui::accesskit::Role::CheckBox)
            .unwrap()
            .rect();
        let toggle = h
            .get_all_by_label(outline_toggle)
            .find(|n| n.accesskit_node().role() == egui::accesskit::Role::CheckBox)
            .unwrap()
            .rect();
        assert!(
            toggle.top() > header.top() && toggle.top() - header.bottom() < 12.0,
            "節の頭: {header:?} {toggle:?}"
        );
    }
}

#[test]
fn the_paint_button_makes_assigns_and_switches_to_the_slot_channel_in_one_undo() {
    let mut h = harness_sized(Lang::Ja, 2400.0);
    set_value(&mut h, "_UseRimShade", 1.0);
    h.run();
    click_label(&mut h, "リムシェード");
    let channels_before = h.state().state.doc.channels().len();
    let steps = h.state().state.doc.undo_count();
    // 割り当てが無いスロット: ひな形と同じ作りのチャンネルを作って割り当て、描くチャンネルにする（1 回の Undo）
    click_label(
        &mut h,
        "リムシェードのマスクを描く（描くチャンネルを作って割り当てる）",
    );
    let doc = &h.state().state.doc;
    assert_eq!(doc.undo_count(), steps + 1);
    assert_eq!(doc.channels().len(), channels_before + 1);
    let TextureSource::Channel(c) = doc.look().textures["_RimShadeMask"] else {
        panic!("チャンネルを割り当てる");
    };
    let info = doc.channel_info(c).unwrap();
    assert_eq!(
        (info.name.as_str(), info.kind),
        ("リムシェードのマスク", ChannelKind::Scalar)
    );
    assert_eq!(info.default, Rgba8::new(255, 255, 255, 255));
    assert_eq!(h.state().state.m2.paint_channel, c);
    assert!(yolu_app::look::panel::painting_slot(
        &h.state().state,
        "_RimShadeMask"
    ));
    // 今描いているスロットはボタンが押し込まれた見た目（読み上げの選択の印）
    let button = h.get_by_label("リムシェードのマスクを描く");
    assert_eq!(
        button.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    // チャンネルの欄の行に、そのチャンネルを読む lilToon のスロットの印
    click_tab(&mut h, yolu_app::Tab::Channels);
    assert!(h.query_by_label("lilToon: リムシェードのマスク").is_some());
    assert!(
        h.query_by_label("lilToon: メインカラー").is_some(),
        "Color の行にはメインカラー"
    );
    // 割り当てのあるスロット: 描くチャンネルを切り替えるだけ（Undo に入らない）
    h.state_mut()
        .apply(Action::Look(LookOp::PaintSlot("_MainTex")));
    h.run();
    assert_eq!(h.state().state.m2.paint_channel, Channel::Color);
    assert_eq!(h.state().state.doc.undo_count(), steps + 1);
    assert_eq!(h.state().state.doc.channels().len(), channels_before + 1);
    // Undo でチャンネルと割り当てが一緒に消え、描くチャンネルは標準に戻る
    h.state_mut()
        .apply(Action::Look(LookOp::PaintSlot("_RimShadeMask")));
    h.state_mut().apply(Action::Undo);
    h.run();
    let doc = &h.state().state.doc;
    assert_eq!(doc.channels().len(), channels_before);
    assert!(!doc.look().textures.contains_key("_RimShadeMask"));
    assert!(doc.channel_info(h.state().state.m2.paint_channel).is_some());
    // 標準の見た目のセットで描く口を使うと、lilToon にもする（同じ 1 回の Undo）
    h.state_mut()
        .state
        .doc
        .set_look(MaterialLook::default(), false)
        .unwrap();
    let steps = h.state().state.doc.undo_count();
    h.state_mut()
        .apply(Action::Look(LookOp::PaintSlot("_ShadowColorTex")));
    let doc = &h.state().state.doc;
    assert_eq!(doc.undo_count(), steps + 1);
    assert_eq!(doc.look().kind, LookKind::LilToon);
    let TextureSource::Channel(c) = doc.look().textures["_ShadowColorTex"] else {
        panic!("割り当てる");
    };
    let info = doc.channel_info(c).unwrap();
    assert_eq!(
        (info.kind, info.color_space, info.default),
        (ChannelKind::Color, ColorSpace::Srgb, Rgba8::new(0, 0, 0, 0))
    );
}

#[test]
fn the_slot_menu_ends_its_channels_with_a_new_channel_assigned_and_painted_in_one_undo() {
    let mut h = harness_sized(Lang::Ja, 2400.0);
    h.state_mut().apply(Action::Look(LookOp::Template));
    h.run();
    click_label(&mut h, "影設定");
    let users = |h: &Harness<'_, YoluApp>| -> Vec<Channel> {
        h.state()
            .state
            .doc
            .channels()
            .into_iter()
            .filter(|c| !c.is_standard())
            .collect()
    };
    let before = users(&h);
    let old = h.state().state.doc.look().textures["_ShadowStrengthMask"];
    assert_eq!(old, TextureSource::Channel(before[0]));
    let steps = h.state().state.doc.undo_count();
    // 割り当てがあっても、スロットの名前で新しく作って割り当て、描くチャンネルにする（1 回の Undo）
    click_label(&mut h, "マスクと強度: 影の強度");
    pick(&mut h, "新しいチャンネル");
    let doc = &h.state().state.doc;
    assert_eq!(doc.undo_count(), steps + 1);
    let after = users(&h);
    assert_eq!(after.len(), before.len() + 1);
    let made = *after.last().unwrap();
    assert_eq!(
        doc.look().textures["_ShadowStrengthMask"],
        TextureSource::Channel(made)
    );
    let info = doc.channel_info(made).unwrap();
    assert_eq!(
        (info.name.as_str(), info.kind),
        ("影の強度 2", ChannelKind::Scalar),
        "同じ名前があれば番号を足す"
    );
    assert_eq!(info.default, Rgba8::new(255, 255, 255, 255));
    assert_eq!(h.state().state.m2.paint_channel, made);
    assert!(yolu_app::look::panel::painting_slot(
        &h.state().state,
        "_ShadowStrengthMask"
    ));
    assert!(h.state().state.modified);
    // 古いチャンネルは消さない（ほかのスロットが読んでいるかもしれない）。Undo 1 回で、割り当てもチャンネルも元に戻る
    assert!(doc.channel_info(before[0]).is_some());
    h.state_mut().apply(Action::Undo);
    h.run();
    let doc = &h.state().state.doc;
    assert_eq!(users(&h), before);
    assert_eq!(doc.look().textures["_ShadowStrengthMask"], old);
    assert!(
        doc.channel_info(h.state().state.m2.paint_channel).is_some(),
        "描くチャンネルは消えたチャンネルのままにならない"
    );
    // 標準のチャンネルを読むスロットも同じ（メインカラーは sRGB の色のチャンネル）
    h.state_mut().apply(Action::Look(LookOp::NewChannel {
        slot: "_MainTex",
        plane: None,
    }));
    let doc = &h.state().state.doc;
    let TextureSource::Channel(c) = doc.look().textures["_MainTex"] else {
        panic!("チャンネルを割り当てる");
    };
    assert!(!c.is_standard());
    let info = doc.channel_info(c).unwrap();
    assert_eq!(
        (info.kind, info.color_space, info.default),
        (
            ChannelKind::Color,
            ColorSpace::Srgb,
            Rgba8::new(255, 255, 255, 255)
        )
    );
    assert_eq!(h.state().state.m2.paint_channel, c);
    // 英語
    h.state_mut().state.lang = Lang::En;
    h.run();
    click_label(&mut h, "Mask & Strength: 影の強度");
    assert!(h.query_by_label("New Channel").is_some());
}

#[test]
fn a_new_channel_for_a_component_assigns_one_scalar_channel_to_that_plane_in_one_undo() {
    let mut h = harness_sized(Lang::Ja, 2400.0);
    h.state_mut().apply(Action::Look(LookOp::Template));
    h.run();
    let Some(TextureSource::Channel(first)) = h
        .state()
        .state
        .doc
        .look()
        .textures
        .get("_ShadowStrengthMask")
        .copied()
    else {
        panic!("ひな形が割り当てる");
    };
    let planes = [
        PlaneSource::Channel {
            channel: first,
            component: 0,
        },
        PlaneSource::Zero,
        PlaneSource::One,
        PlaneSource::Zero,
    ];
    h.state_mut().apply(Action::Look(LookOp::Texture {
        slot: "_ShadowStrengthMask",
        source: Some(TextureSource::Packed(planes)),
    }));
    h.run();
    click_label(&mut h, "影設定");
    let channels = h.state().state.doc.channels().len();
    let steps = h.state().state.doc.undo_count();
    // 成分ごとの行（G）の選択肢の最後に「新しいチャンネル」
    click_label(&mut h, "G: 0");
    pick(&mut h, "新しいチャンネル");
    let doc = &h.state().state.doc;
    assert_eq!(doc.undo_count(), steps + 1);
    assert_eq!(doc.channels().len(), channels + 1);
    let TextureSource::Packed(now) = doc.look().textures["_ShadowStrengthMask"] else {
        panic!("成分ごとのまま");
    };
    let PlaneSource::Channel {
        channel: made,
        component: 0,
    } = now[1]
    else {
        panic!("G にチャンネルを割り当てる: {now:?}");
    };
    assert_eq!(
        (now[0], now[2], now[3]),
        (planes[0], planes[2], planes[3]),
        "ほかの成分はそのまま"
    );
    let info = doc.channel_info(made).unwrap();
    assert_eq!(
        (info.name.as_str(), info.kind, info.color_space),
        ("影の強度 G", ChannelKind::Scalar, ColorSpace::Linear)
    );
    assert_eq!(
        info.default,
        Rgba8::new(0, 0, 0, 255),
        "何も描いていない所は、置き換えた成分（0）のまま"
    );
    assert_eq!(h.state().state.m2.paint_channel, made);
    h.state_mut().apply(Action::Undo);
    h.run();
    let doc = &h.state().state.doc;
    assert_eq!(doc.channels().len(), channels);
    assert_eq!(
        doc.look().textures["_ShadowStrengthMask"],
        TextureSource::Packed(planes)
    );
    // 1 の成分を置き換えるなら白、チャンネルの成分を置き換えるならスロットの既定（白）から始まる
    for k in [2u8, 0] {
        h.state_mut().apply(Action::Look(LookOp::NewChannel {
            slot: "_ShadowStrengthMask",
            plane: Some(k),
        }));
        let doc = &h.state().state.doc;
        let TextureSource::Packed(now) = doc.look().textures["_ShadowStrengthMask"] else {
            panic!("成分ごとのまま");
        };
        let PlaneSource::Channel { channel, .. } = now[k as usize] else {
            panic!("{now:?}");
        };
        assert_eq!(
            doc.channel_info(channel).unwrap().default,
            Rgba8::new(255, 255, 255, 255),
            "成分 {k}"
        );
        h.state_mut().apply(Action::Undo);
    }
    h.run();
    let doc = &h.state().state.doc;
    assert_eq!(
        doc.look().textures["_ShadowStrengthMask"],
        TextureSource::Packed(planes)
    );
    // 成分ごとでないスロットに成分の新しいチャンネルは作れない（何も変えない）
    let steps = doc.undo_count();
    h.state_mut().apply(Action::Look(LookOp::NewChannel {
        slot: "_MainTex",
        plane: Some(0),
    }));
    assert_eq!(h.state().state.doc.undo_count(), steps);
    assert_eq!(h.state().state.doc.channels().len(), channels);
}

#[test]
fn a_new_channel_is_not_offered_for_a_slot_that_reads_an_image_and_past_sixteen_user_channels_it_is_marked_not_drawn(
) {
    let mut h = harness_sized(Lang::Ja, 2400.0);
    // マットキャップの絵のスロット（プロジェクトの画像を読む）には描く口が無く、新しいチャンネルも無い
    let steps = h.state().state.doc.undo_count();
    h.state_mut().apply(Action::Look(LookOp::NewChannel {
        slot: "_MatCapTex",
        plane: None,
    }));
    assert_eq!(h.state().state.doc.undo_count(), steps, "何も変えない");
    // ユーザーチャンネルが 16 を超えると、3D ビューは配列に入る先の 16 個（スロットの並びの順）だけを描く（今までの印）
    let paintable: Vec<&str> = yolu_app::look::liltoon::SLOTS
        .iter()
        .filter(|s| s.usage != yolu_app::look::liltoon::SlotUse::Image)
        .map(|s| s.name)
        .take(17)
        .collect();
    for slot in &paintable[..16] {
        h.state_mut()
            .apply(Action::Look(LookOp::NewChannel { slot, plane: None }));
    }
    let users = |h: &Harness<'_, YoluApp>| {
        h.state()
            .state
            .doc
            .channels()
            .iter()
            .filter(|c| !c.is_standard())
            .count()
    };
    assert_eq!(users(&h), 16);
    assert!(paintable[..16]
        .iter()
        .all(|s| !yolu_app::look::panel::slot_over_layer_limit(&h.state().state.doc, s)));
    h.state_mut().apply(Action::Look(LookOp::NewChannel {
        slot: paintable[16],
        plane: None,
    }));
    assert_eq!(users(&h), 17, "作れる。上限は表示の側");
    let doc = &h.state().state.doc;
    assert!(
        yolu_app::look::panel::slot_over_layer_limit(doc, paintable[16]),
        "17 個目は描かない印"
    );
    assert!(!yolu_app::look::panel::slot_over_layer_limit(
        doc,
        paintable[0]
    ));
}

#[test]
fn every_section_with_every_feature_on_shows_names_only_in_both_languages() {
    for lang in Lang::ALL {
        let mut h = harness_sized(lang, 2400.0);
        for toggle in [
            "_UseMain2ndTex",
            "_UseMain3rdTex",
            "_UseShadow",
            "_UseRimShade",
            "_UseEmission",
            "_UseEmission2nd",
            "_UseBumpMap",
            "_UseBump2ndMap",
            "_UseAnisotropy",
            "_Anisotropy2Reflection",
            "_UseBacklight",
            "_UseReflection",
            "_ApplyReflection",
            "_UseMatCap",
            "_UseMatCap2nd",
            "_MatCapCustomNormal",
            "_UseRim",
            "_UseGlitter",
            "_Main2ndTexIsDecal",
        ] {
            set_value(&mut h, toggle, 1.0);
        }
        h.state_mut().apply(Action::Look(LookOp::Outline(true)));
        h.state_mut().apply(Action::Look(LookOp::Mode(
            yolu_app::look::liltoon::RenderMode::Transparent,
        )));
        set_value(&mut h, "_AlphaMaskMode", 1.0);
        h.run();
        let sections: Vec<&str> = [
            ("基本設定", "Base Setting"),
            ("ライティング・明るさ設定", "Lighting"),
            ("UV設定", "UV Setting"),
            ("メインカラー / 透過設定", "Main Color / Alpha"),
            ("影設定", "Shadow"),
            ("リムシェード", "RimShade"),
            ("発光設定", "Emission"),
            ("ノーマルマップ設定", "Normal Map"),
            ("逆光ライト", "Backlight"),
            ("光沢設定", "Reflections"),
            ("マットキャップ設定", "MatCap"),
            ("リムライト設定", "Rim Light"),
            ("ラメ設定", "Glitter"),
            ("輪郭線設定", "Outline"),
            ("距離フェード", "Distance Fade"),
        ]
        .iter()
        .map(|(ja, en)| lang.pick(*ja, *en))
        .collect();
        let mut seen = std::collections::BTreeSet::new();
        // 1 つずつ開いて、その節の文言を見る（全部を開くとウィンドウに入らない）
        for name in &sections {
            let header = h
                .get_all_by_label(name)
                .find(|n| n.accesskit_node().role() != egui::accesskit::Role::CheckBox)
                .unwrap()
                .rect();
            click(&mut h, header.center());
            h.run();
            let look = h.get_by_label(lang.pick("見た目", "Look")).rect();
            let mut all = Vec::new();
            for shape in &h.output().shapes {
                texts(&shape.shape, &mut all);
            }
            for (p, text) in all {
                if p.x >= look.left() - 4.0 && p.x <= look.right() + 4.0 && p.y >= look.top() - 2.0
                {
                    assert_plain("見た目の欄（全部の機能）", &text);
                    if lang == Lang::En {
                        assert!(!has_japanese(&text), "{name}: {text}");
                    }
                    seen.insert(text);
                }
            }
            // 閉じて次へ
            let header = h
                .get_all_by_label(name)
                .find(|n| n.accesskit_node().role() != egui::accesskit::Role::CheckBox)
                .unwrap()
                .rect();
            click(&mut h, header.center());
            h.run();
        }
        // 足した機能の名前が欄に出る
        for want in [
            lang.pick("メインカラー2nd", "Main Color 2nd"),
            lang.pick("ミラーモード", "Mirror Mode"),
            lang.pick("ノーマルマップ2nd", "Normal Map 2nd"),
            lang.pick("異方性反射", "Anisotropy"),
            lang.pick("光沢のタイプ", "Specular Mode"),
            lang.pick("指向性", "Directivity"),
            lang.pick("パーティクルサイズ", "Particle Size"),
            lang.pick("ハイライト", "Highlight"),
            lang.pick("開始距離", "Start Distance"),
            lang.pick("カスタムノーマルマップ", "Custom normal map"),
        ] {
            assert!(
                seen.iter().any(|s| s.contains(want)),
                "{want} が無い: {seen:?}"
            );
        }
    }
}

/// 節の見出しを押す（同じ名前の機能の入切があれば、見出しのほう）。
fn toggle_section(h: &mut Harness<'_, YoluApp>, name: &str) {
    let header = h
        .get_all_by_label(name)
        .find(|n| n.accesskit_node().role() != egui::accesskit::Role::CheckBox)
        .unwrap_or_else(|| panic!("見出し {name}"))
        .rect();
    click(h, header.center());
    h.run();
}

/// スライダーが見せている値。
fn shown(h: &Harness<'_, YoluApp>, label: &str) -> f64 {
    h.get_by_label(label)
        .accesskit_node()
        .numeric_value()
        .unwrap_or_else(|| panic!("{label} の値"))
}

fn stored(h: &Harness<'_, YoluApp>, name: &str) -> Option<LookValue> {
    h.state().state.doc.look().get(name)
}

#[test]
fn turning_the_decal_off_clears_its_mirror_and_copy_modes_in_one_undo_like_liltoon() {
    use yolu_app::look::fields;
    let mut h = harness_sized(Lang::Ja, 3200.0);
    set_value(&mut h, "_UseMain2ndTex", 1.0);
    set_value(&mut h, "_Main2ndTexIsDecal", 1.0);
    h.run();
    toggle_section(&mut h, "メインカラー / 透過設定");
    // 位置と大きさは lilToon の欄と同じ換算: 既定の (1, 1, 0, 0) は X・Y 座標 0.5、サイズ 1
    for (label, v) in [
        ("X座標", 0.5),
        ("Y座標", 0.5),
        ("X軸サイズ", 1.0),
        ("Y軸サイズ", 1.0),
    ] {
        assert!(
            (shown(&h, label) - v).abs() < 1e-6,
            "{label}: {}",
            shown(&h, label)
        );
    }
    // 欄の値 → 保存する値 → 欄の値（1 回の Undo）
    let steps = h.state().state.doc.undo_count();
    h.state_mut().apply(Action::Look(fields::decal_st_op(
        "_Main2ndTex_ST",
        [0.3, 0.7, 0.25, 0.4],
        false,
    )));
    h.run();
    assert_eq!(h.state().state.doc.undo_count(), steps + 1);
    let Some(LookValue::Vector(st)) = stored(&h, "_Main2ndTex_ST") else {
        panic!("保存する値");
    };
    for (k, want) in [4.0f32, 2.5, -0.7, -1.25].into_iter().enumerate() {
        assert!((st[k] - want).abs() < 1e-5, "{st:?}");
    }
    for (label, v) in [
        ("X座標", 0.3),
        ("Y座標", 0.7),
        ("X軸サイズ", 0.25),
        ("Y軸サイズ", 0.4),
    ] {
        assert!(
            (shown(&h, label) - v).abs() < 1e-5,
            "{label}: {}",
            shown(&h, label)
        );
    }
    // ミラーモード「右のみ・反転」と複製モード「反転」（それぞれ 1 回の Undo）
    let steps = h.state().state.doc.undo_count();
    click_label(&mut h, "ミラーモード: 通常");
    pick(&mut h, "右のみ・反転");
    click_label(&mut h, "複製モード: 通常");
    pick(&mut h, "反転");
    assert_eq!(h.state().state.doc.undo_count(), steps + 2);
    let on = |h: &Harness<'_, YoluApp>, n: &str| {
        yolu_app::look::liltoon::on(h.state().state.doc.look(), n)
    };
    let flags = [
        "IsLeftOnly",
        "IsRightOnly",
        "ShouldFlipMirror",
        "ShouldCopy",
        "ShouldFlipCopy",
    ];
    let set: Vec<bool> = flags
        .iter()
        .map(|f| on(&h, &format!("_Main2ndTex{f}")))
        .collect();
    assert_eq!(set, [false, true, true, true, true]);
    // 複製モードの X 座標は右半分で見せる（0.3 は 0.7）
    assert!((shown(&h, "X座標") - 0.7).abs() < 1e-5);
    // デカールを切: ミラーと複製のフラグも 0（lilToon の UV4Decal と同じ）、1 回の Undo。欄にミラー・複製の行は出ない
    let steps = h.state().state.doc.undo_count();
    click_label(&mut h, "デカール化");
    assert_eq!(h.state().state.doc.undo_count(), steps + 1, "1 回の Undo");
    assert!(!on(&h, "_Main2ndTexIsDecal"));
    for f in flags {
        assert!(!on(&h, &format!("_Main2ndTex{f}")), "{f}");
    }
    assert_eq!(h.query_all_by_label_contains("ミラーモード").count(), 0);
    // 3D ビューへ渡す値にも、隠す・裏返すフラグは残らない
    let values = yolu_app::view3d::look_gpu::params(
        &h.state().state.doc,
        h.state().state.doc.drawn_look(),
        &[],
        [None, None],
    );
    let at = |i: usize, k: usize| values[i * 4 + k];
    assert_eq!(
        [
            at(65, 0),
            at(65, 1),
            at(65, 2),
            at(65, 3),
            at(66, 0),
            at(66, 1)
        ],
        [0.0; 6]
    );
    // Undo で入とフラグが一緒に戻る
    h.state_mut().apply(Action::Undo);
    h.run();
    assert!(on(&h, "_Main2ndTexIsDecal"));
    let set: Vec<bool> = flags
        .iter()
        .map(|f| on(&h, &format!("_Main2ndTex{f}")))
        .collect();
    assert_eq!(set, [false, true, true, true, true]);
    assert!(h.query_by_label("ミラーモード: 右のみ・反転").is_some());
    // 入にするときはフラグを変えない（切のあとに入にしても、フラグは 0 のまま）
    click_label(&mut h, "デカール化");
    click_label(&mut h, "デカール化");
    assert!(on(&h, "_Main2ndTexIsDecal"));
    assert!(flags.iter().all(|f| !on(&h, &format!("_Main2ndTex{f}"))));
}

#[test]
fn the_specular_mode_glitter_fields_and_lighting_presets_match_liltoon_and_are_one_undo() {
    use yolu_app::look::{fields, liltoon, Section};
    let mut h = harness_sized(Lang::Ja, 4400.0);
    set_value(&mut h, "_UseReflection", 1.0);
    set_value(&mut h, "_UseGlitter", 1.0);
    h.run();
    // 光沢のタイプ: 既定は lilToon と同じくトゥーン。リアルを選ぶと 2 つの値を 1 回の Undo で
    toggle_section(&mut h, "光沢設定");
    let steps = h.state().state.doc.undo_count();
    click_label(&mut h, "光沢のタイプ: トゥーン");
    pick(&mut h, "リアル");
    assert_eq!(h.state().state.doc.undo_count(), steps + 1);
    assert_eq!(stored(&h, "_ApplySpecular"), Some(LookValue::Float(1.0)));
    assert_eq!(stored(&h, "_SpecularToon"), Some(LookValue::Float(0.0)));
    assert!(h.query_by_label("光沢のタイプ: リアル").is_some());
    h.state_mut().apply(Action::Undo);
    h.run();
    assert_eq!(stored(&h, "_ApplySpecular"), None);
    assert!(h.query_by_label("光沢のタイプ: トゥーン").is_some());
    toggle_section(&mut h, "光沢設定");

    // ラメ: 既定の値は lilToon の欄と同じ表示（サイズ 1・パーティクルサイズ 0.4・密度 √(1/50)/1.5・感度 0.25 / 密度）
    toggle_section(&mut h, "ラメ設定");
    let density = (1.0f64 / 50.0).sqrt() / 1.5;
    for (label, v) in [
        ("サイズ X", 1.0),
        ("サイズ Y", 1.0),
        ("パーティクルサイズ", 0.4),
        ("密度", density),
        ("感度", 0.25 / density),
    ] {
        assert!(
            (shown(&h, label) - v).abs() < 1e-4,
            "{label}: {}",
            shown(&h, label)
        );
    }
    // 欄の値 → 保存する 2 つの値（1 回の Undo）→ 欄の値
    let steps = h.state().state.doc.undo_count();
    h.state_mut().apply(Action::Look(fields::glitter_op(
        [2.0, 0.5, 0.3, 0.2, 4.0],
        false,
    )));
    h.run();
    assert_eq!(h.state().state.doc.undo_count(), steps + 1);
    let Some(LookValue::Vector(p)) = stored(&h, "_GlitterParams1") else {
        panic!("保存する値");
    };
    for (k, want) in [128.0f32, 512.0, 0.09, 1.0 / (0.04 * 2.25)]
        .into_iter()
        .enumerate()
    {
        assert!((p[k] - want).abs() < 1e-3 * want.abs().max(1.0), "{p:?}");
    }
    assert!(
        matches!(stored(&h, "_GlitterSensitivity"), Some(LookValue::Float(v)) if (v - 0.8).abs() < 1e-5)
    );
    for (label, v) in [
        ("サイズ X", 2.0),
        ("サイズ Y", 0.5),
        ("パーティクルサイズ", 0.3),
        ("密度", 0.2),
        ("感度", 4.0),
    ] {
        assert!(
            (shown(&h, label) - v).abs() < 1e-3,
            "{label}: {}",
            shown(&h, label)
        );
    }
    toggle_section(&mut h, "ラメ設定");

    // ライティングのプリセット: lilToon の ApplyLightingPreset と同じ値（欄の表にある 5 つ）を 1 回の Undo で
    toggle_section(&mut h, "ライティング・明るさ設定");
    let before: Vec<String> = h
        .state()
        .state
        .doc
        .look()
        .properties
        .keys()
        .cloned()
        .collect();
    let steps = h.state().state.doc.undo_count();
    click_label(&mut h, "半モノクロ");
    assert_eq!(h.state().state.doc.undo_count(), steps + 1);
    let semi = [
        ("_AsUnlit", 0.0),
        ("_LightMinLimit", 0.05),
        ("_LightMaxLimit", 1.0),
        ("_MonochromeLighting", 0.5),
        ("_ShadowEnvStrength", 0.0),
    ];
    for (name, v) in semi {
        assert_eq!(stored(&h, name), Some(LookValue::Float(v)), "{name}");
    }
    let added: Vec<String> = h
        .state()
        .state
        .doc
        .look()
        .properties
        .keys()
        .filter(|k| !before.contains(k))
        .cloned()
        .collect();
    assert_eq!(
        added.len(),
        5,
        "表に無い値（頂点ライトなど）は入れない: {added:?}"
    );
    assert!(added
        .iter()
        .all(|k| liltoon::section_props(Section::Lighting).any(|n| n == k)));
    // 「通常」はほかの欄にもあるので、「半モノクロ」と同じ行のボタン
    let semi_row = h.get_by_label("半モノクロ").rect();
    let default_button = h
        .get_all_by_label("通常")
        .map(|n| n.rect())
        .find(|r| (r.center().y - semi_row.center().y).abs() < 2.0)
        .expect("プリセットの通常");
    click(&mut h, default_button.center());
    assert_eq!(
        stored(&h, "_MonochromeLighting"),
        Some(LookValue::Float(0.0))
    );
    // 節の「既定に戻す」で、プリセットの値は全部外れる
    h.state_mut()
        .apply(Action::Look(LookOp::Reset(Section::Lighting)));
    for (name, _) in semi {
        assert_eq!(stored(&h, name), None, "{name}");
    }
    // Undo は 1 段ずつ（既定に戻す → 通常 → 半モノクロ）
    h.state_mut().apply(Action::Undo);
    h.state_mut().apply(Action::Undo);
    assert_eq!(
        stored(&h, "_MonochromeLighting"),
        Some(LookValue::Float(0.5))
    );
    h.state_mut().apply(Action::Undo);
    assert_eq!(stored(&h, "_MonochromeLighting"), None);
}

#[test]
fn the_alpha_mask_and_outline_highlight_fields_match_liltoon_and_round_trip() {
    use yolu_app::look::fields;
    let mut h = harness_sized(Lang::Ja, 4400.0);
    h.state_mut().apply(Action::Look(LookOp::Mode(
        yolu_app::look::liltoon::RenderMode::Transparent,
    )));
    set_value(&mut h, "_AlphaMaskMode", 1.0);
    h.state_mut().apply(Action::Look(LookOp::Outline(true)));
    h.state_mut().apply(Action::Look(LookOp::Value {
        name: "_OutlineLitColor",
        value: LookValue::Color([1.0, 0.2, 0.0, 1.0]),
        drag: false,
    }));
    h.run();
    // アルファマスク: 既定は反転なし・透明度 0（lilToon の欄の表示）
    toggle_section(&mut h, "メインカラー / 透過設定");
    let toggled = |h: &Harness<'_, YoluApp>| h.get_by_label("Invert").accesskit_node().toggled();
    assert_eq!(toggled(&h), Some(egui::accesskit::Toggled::False));
    assert_eq!(shown(&h, "Transparency"), 0.0);
    // 反転を入: スケール −1・オフセット 1（透明度はそのまま 0）、1 回の Undo
    let steps = h.state().state.doc.undo_count();
    click_label(&mut h, "Invert");
    assert_eq!(h.state().state.doc.undo_count(), steps + 1);
    assert_eq!(stored(&h, "_AlphaMaskScale"), Some(LookValue::Float(-1.0)));
    assert_eq!(stored(&h, "_AlphaMaskValue"), Some(LookValue::Float(1.0)));
    assert_eq!(toggled(&h), Some(egui::accesskit::Toggled::True));
    assert_eq!(shown(&h, "Transparency"), 0.0);
    // 透明度 −0.4 → オフセット 0.6 → 欄は −0.4
    h.state_mut()
        .apply(Action::Look(fields::alpha_mask_op(true, -0.4, false)));
    h.run();
    assert!(
        matches!(stored(&h, "_AlphaMaskValue"), Some(LookValue::Float(v)) if (v - 0.6).abs() < 1e-6)
    );
    assert!((shown(&h, "Transparency") + 0.4).abs() < 1e-6);
    h.state_mut().apply(Action::Undo);
    h.state_mut().apply(Action::Undo);
    h.run();
    assert_eq!(stored(&h, "_AlphaMaskScale"), None);
    assert_eq!(toggled(&h), Some(egui::accesskit::Toggled::False));
    toggle_section(&mut h, "メインカラー / 透過設定");

    // 輪郭線のハイライト: 既定（スケール 10・オフセット −8）は lilToon の欄で Min 0.8・Max 0.9
    toggle_section(&mut h, "輪郭線設定");
    assert!(
        (shown(&h, "Min") - 0.8).abs() < 1e-5,
        "{}",
        shown(&h, "Min")
    );
    assert!(
        (shown(&h, "Max") - 0.9).abs() < 1e-5,
        "{}",
        shown(&h, "Max")
    );
    let steps = h.state().state.doc.undo_count();
    h.state_mut()
        .apply(Action::Look(fields::outline_lit_op(0.2, 0.6, false)));
    h.run();
    assert_eq!(h.state().state.doc.undo_count(), steps + 1);
    assert!(
        matches!(stored(&h, "_OutlineLitScale"), Some(LookValue::Float(v)) if (v - 2.5).abs() < 1e-5)
    );
    assert!(
        matches!(stored(&h, "_OutlineLitOffset"), Some(LookValue::Float(v)) if (v + 0.5).abs() < 1e-5)
    );
    assert!((shown(&h, "Min") - 0.2).abs() < 1e-5);
    assert!((shown(&h, "Max") - 0.6).abs() < 1e-5);
    h.state_mut().apply(Action::Undo);
    h.run();
    assert_eq!(stored(&h, "_OutlineLitScale"), None);
    assert!((shown(&h, "Min") - 0.8).abs() < 1e-5);
}
