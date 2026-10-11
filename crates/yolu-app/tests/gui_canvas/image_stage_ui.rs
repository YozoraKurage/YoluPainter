//! 画像の Generator の段の欄（egui_kittest）: 画像の箱（一覧から選ぶ・アセットの画像を落とす・外す）、成分・投影の種類の一覧、
//! 3D ビューのハンドル、日英の見た目（説明の文を置かない・英語の画面に日本語を残さない・切れた文字が無い）とスナップショット。
//! 位置と法線のマップは、試しの立方体を CPU で焼いて文書の効果の入力へ足す（`fillfx_gui` と同じ）。
use crate::common;

use common::*;
use egui::{vec2, Pos2, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, SnapshotResults};
use yolu_app::bake::{BakeAction, BakeBackend};
use yolu_app::fillfx::{gizmo, inputs};
use yolu_app::fx::{FxOp, Selected};
use yolu_app::lang::Lang;
use yolu_app::state::{Action, AppState};
use yolu_app::{Tab, YoluApp};
use yolu_core::fill_image::{Placement, ProjectionMode};
use yolu_core::generator::{ImageComponent, Kind, MapState, Settings};
use yolu_core::mesh_maps::MeshMapKind;
use yolu_core::{Channel, FilterTarget, ImageId, MapInput};

fn quad() -> Vec<u8> {
    vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ]
}

fn st<'a>(h: &'a Harness<'_, YoluApp>) -> &'a AppState {
    &h.state().state
}

fn apply(h: &mut Harness<'_, YoluApp>, a: Action) {
    h.state_mut().state.apply(a);
    h.run();
}

/// 試しの立方体を焼いて（位置・法線）文書の入力へ足し、3D のタブと棚（アセット）のタブを前へ出したウィンドウ。
fn window(height: f32, lang: Lang) -> Harness<'static, YoluApp> {
    let mut h = app(1280.0, height, 64);
    {
        let s = &mut h.state_mut().state;
        s.set_language(lang);
        s.bake.backend = BakeBackend::Cpu;
        s.apply(Action::LoadDemoModel);
        s.bake.settings.maps = vec![MeshMapKind::WorldNormal, MeshMapKind::Position];
        s.bake.settings.padding = 4;
        s.apply(Action::Bake(BakeAction::Start));
        s.wait_bake();
        let mut inputs = s.doc.effect_inputs().clone();
        for kind in [MeshMapKind::Position, MeshMapKind::WorldNormal] {
            let map = s.sets.current().mesh_maps.get(kind).unwrap().clone();
            inputs = inputs
                .with_map(MapInput::from_baked(&map, MapState::Current).unwrap())
                .unwrap();
        }
        s.doc.set_effect_inputs(inputs).unwrap();
    }
    click_tab(&mut h, Tab::View3d);
    click_tab(&mut h, Tab::Assets);
    // 白の塗りつぶしレイヤー（段の画像がそのまま見える）
    apply(&mut h, Action::M2(yolu_app::m2::Edit::NewFill));
    h
}

fn shelf_image(h: &mut Harness<'_, YoluApp>, name: &str) -> ImageId {
    let lang = st(h).lang;
    let rid = h
        .state_mut()
        .state
        .shelf
        .add_image(lang, name, &quad(), 2, 2)
        .unwrap();
    h.run();
    inputs::image_id(&rid).unwrap()
}

/// 選んでいる段の Generator の設定。
fn stage(h: &Harness<'_, YoluApp>) -> Settings {
    let (_, effect, _) = st(h).fx.filter(&st(h).doc).unwrap();
    effect.settings().generator_settings().unwrap().clone()
}

fn add_image_stage(h: &mut Harness<'_, YoluApp>, target: FilterTarget) {
    if target == FilterTarget::Mask {
        let layer = st(h).selected_layer.unwrap();
        apply(h, Action::M2(yolu_app::m2::Edit::AddMask(layer)));
    }
    apply(
        h,
        Action::Fx(FxOp::AddGenerator {
            target,
            kind: Kind::Image,
        }),
    );
}

/// 右の列（プロパティ）の中の部品。
fn right(r: Rect) -> bool {
    r.left() > rx()
}

/// 右の列の画像の箱（名前の付いた、高さの低い横長の部品。段の見出しも「画像」なので、いちばん下のもの）。
fn image_box(h: &Harness<'_, YoluApp>, label: &str) -> Rect {
    h.get_all_by_label(label)
        .map(|n| n.rect())
        .filter(|r| right(*r) && r.height() < 40.0 && r.width() > 100.0)
        .max_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap_or_else(|| panic!("画像の箱: {label}"))
}

/// 右上の組（アセット）の格子の中の素材。
fn card(h: &Harness<'_, YoluApp>, name: &str) -> Rect {
    let body = top_right_body(h);
    rect_of(h, name, |r| body.contains(r.center()))
}

#[test]
fn the_image_is_picked_from_the_list_or_dropped_and_each_change_is_one_undo() {
    let mut h = window(1200.0, Lang::Ja);
    let image = shelf_image(&mut h, "石の模様");
    add_image_stage(&mut h, FilterTarget::Content);
    let Some(Selected::Filter { layer, id }) = st(&h).fx.selected else {
        panic!("段を選ぶ")
    };
    assert_eq!(stage(&h).image.image, 0, "足した直後は画像を選んでいない");
    assert_eq!(
        st(&h).doc.generator_inactive(layer, id).unwrap(),
        Some(yolu_core::InactiveReason::Generator(
            yolu_core::generator::Inactive::NoImage
        ))
    );
    // 箱を押すと一覧
    let steps = st(&h).doc.undo_count();
    let boxed = image_box(&h, "画像");
    click(&mut h, boxed.center());
    assert!(st(&h).popup.is_some(), "画像の一覧が開く");
    let item = popup_item(&h, "石の模様  (2 × 2)");
    click(&mut h, item.center());
    assert_eq!(stage(&h).image.image, image.0, "{}", st(&h).message);
    assert_eq!(st(&h).doc.undo_count(), steps + 1);
    assert_eq!(st(&h).doc.generator_inactive(layer, id).unwrap(), None);
    assert!(st(&h).message.contains("石の模様"), "{}", st(&h).message);
    apply(&mut h, Action::Undo);
    assert_eq!(stage(&h).image.image, 0);
    // アセットの画像をドラッグして箱へ落とす
    let target = image_box(&h, "画像");
    let from = card(&h, "石の模様").center();
    drag(
        &mut h,
        &[
            from,
            from + vec2(30.0, 10.0),
            target.center() + vec2(0.0, -30.0),
            target.center(),
        ],
    );
    assert_eq!(stage(&h).image.image, image.0, "{}", st(&h).message);
    assert_eq!(st(&h).doc.undo_count(), steps + 1);
    assert!(!egui::DragAndDrop::has_any_payload(&h.ctx));
    // 箱の名前は画像の名前になり、外すボタンで外れる（入力のまま通す）
    let named = image_box(&h, "石の模様");
    click(&mut h, Pos2::new(named.right() + 14.0, named.center().y));
    assert_eq!(stage(&h).image.image, 0);
    assert_eq!(st(&h).doc.undo_count(), steps + 2);
    // 取り消すと戻り、もう 1 回で落とす前、もう 1 回で段が無くなる
    apply(&mut h, Action::Undo);
    assert_eq!(stage(&h).image.image, image.0);
    apply(&mut h, Action::Undo);
    assert_eq!(stage(&h).image.image, 0);
    apply(&mut h, Action::Undo);
    assert!(st(&h)
        .doc
        .filters_of(layer, FilterTarget::Content)
        .unwrap()
        .is_empty());
}

#[test]
fn the_projection_list_fits_the_box_to_the_model_and_the_handles_edit_it_in_3d() {
    let mut h = window(1600.0, Lang::Ja);
    let image = shelf_image(&mut h, "tile");
    add_image_stage(&mut h, FilterTarget::Mask);
    let Some(Selected::Filter { layer, id }) = st(&h).fx.selected else {
        panic!("段を選ぶ")
    };
    apply(
        &mut h,
        Action::Fx(FxOp::SetImage {
            layer,
            id,
            image: Some(image),
        }),
    );
    // UV のあいだは置き場の欄もハンドルも無い
    assert!(h.query_all_by_label("3D ビューのハンドル").next().is_none());
    // 投影の種類をトライプラナーへ: 初めのままの置き場はモデルの外形に合わせる（1 回の Undo）
    let steps = st(&h).doc.undo_count();
    let mode = rect_of(&h, "投影: UV", right);
    click(&mut h, mode.center());
    let item = popup_item(&h, "トライプラナー");
    click(&mut h, item.center());
    let g = stage(&h);
    assert_eq!(g.image.projection.mode, ProjectionMode::Triplanar);
    assert_ne!(g.image.projection.placement, Placement::default());
    assert_eq!(st(&h).doc.undo_count(), steps + 1);
    // デカールは一覧に無い
    let mode = rect_of(&h, "投影: トライプラナー", right);
    click(&mut h, mode.center());
    let body = st(&h).popup.as_ref().expect("一覧が開く").state.rect;
    assert!(popup_item(&h, "円柱").is_positive());
    assert!(!h
        .query_all_by_label("デカール")
        .any(|n| body.contains_rect(n.rect())));
    h.state_mut().state.popup = None;
    h.run();
    // 成分（マスクの段なので出る）
    let component = rect_of(&h, "成分: 輝度", right);
    click(&mut h, component.center());
    let item = popup_item(&h, "アルファ");
    click(&mut h, item.center());
    assert_eq!(stage(&h).image.component, ImageComponent::Alpha);
    assert_eq!(st(&h).doc.undo_count(), steps + 2);
    // 3D ビューのハンドルを出すと、ギズモはこの段の置き場
    h.get_by_label("3D ビューのハンドル").click();
    h.run();
    assert_eq!(st(&h).fillfx.edit_filter, Some((layer, id)));
    assert_eq!(
        gizmo::target(st(&h)),
        Some(gizmo::Target::Filter(layer, id))
    );
    h.get_by_label("3D ビューのハンドル").click();
    h.run();
    assert_eq!(st(&h).fillfx.edit_filter, None);
    assert_eq!(
        st(&h).doc.undo_count(),
        steps + 2,
        "ハンドルの出し入れは履歴に入らない"
    );
}

#[test]
fn a_colour_stage_has_no_component_or_range_and_a_mask_stage_has_both() {
    for (target, shown) in [(FilterTarget::Content, false), (FilterTarget::Mask, true)] {
        let mut h = window(1600.0, Lang::Ja);
        add_image_stage(&mut h, target);
        for label in ["成分: 輝度", "下限", "上限", "やわらかさ"] {
            assert_eq!(
                h.query_all_by_label_contains(label)
                    .any(|n| right(n.rect())),
                shown,
                "{target:?}: {label}"
            );
        }
        // 崩し（重ねるノイズ）とピンは、どちらにも無い
        for label in ["崩し", "このベイクだけを読む"] {
            assert!(
                !h.query_all_by_label_contains(label)
                    .any(|n| right(n.rect())),
                "{target:?}: {label}"
            );
        }
        // 色の段でも、スカラーのチャンネルを足すと成分が出る
        if target == FilterTarget::Content {
            let Some(Selected::Filter { layer, id }) = st(&h).fx.selected else {
                panic!("段を選ぶ")
            };
            apply(
                &mut h,
                Action::Fx(FxOp::SetChannels {
                    layer,
                    id,
                    channels: vec![Channel::Color, Channel::Roughness],
                }),
            );
            assert!(h.query_all_by_label("成分: 輝度").any(|n| right(n.rect())));
        }
    }
}

/// 右の列に描いた文字（位置つき）。
fn right_texts(h: &Harness<'_, YoluApp>) -> Vec<String> {
    fn walk(shape: &egui::epaint::Shape, out: &mut Vec<(Pos2, String)>) {
        match shape {
            egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            egui::epaint::Shape::Text(t) => out.push((t.pos, t.galley.text().to_owned())),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        let mut texts = Vec::new();
        walk(&shape.shape, &mut texts);
        for (at, text) in texts {
            if at.x > 1050.0
                && shape
                    .clip_rect
                    .intersects(Rect::from_min_size(at, vec2(1.0, 1.0)))
            {
                out.push(text);
            }
        }
    }
    out
}

#[test]
fn snapshot_the_image_stage_panel_in_both_languages() {
    let mut results = SnapshotResults::new();
    for lang in Lang::ALL {
        for (target, name) in [
            (FilterTarget::Content, "color"),
            (FilterTarget::Mask, "mask"),
        ] {
            let mut h = window(1600.0, lang);
            let image = shelf_image(&mut h, "tile");
            add_image_stage(&mut h, target);
            let Some(Selected::Filter { layer, id }) = st(&h).fx.selected else {
                panic!("段を選ぶ")
            };
            apply(
                &mut h,
                Action::Fx(FxOp::SetImage {
                    layer,
                    id,
                    image: Some(image),
                }),
            );
            if target == FilterTarget::Mask {
                let mut g = stage(&h);
                g.image.projection.mode = ProjectionMode::Triplanar;
                g.image.projection.placement = st(&h)
                    .fitted_placement_for(ProjectionMode::Triplanar, None)
                    .unwrap();
                apply(
                    &mut h,
                    Action::Fx(FxOp::SetSettings {
                        layer,
                        id,
                        settings: yolu_core::EffectSettings::generator(g),
                        coalesce: false,
                    }),
                );
            }
            h.state_mut().state.message.clear();
            h.run();
            let texts = right_texts(&h);
            assert!(
                texts.len() > 15,
                "{lang:?} {name}: 欄の文字を集められていない（{}）",
                texts.len()
            );
            for text in &texts {
                assert_plain(&format!("{lang:?} {name}"), text);
                if lang == Lang::En {
                    assert!(!has_japanese(text), "{name}: 英語の画面に日本語: {text}");
                }
            }
            // 棚の素材の絵は別のスレッドで作る。できるまで待ってから撮る
            h.state_mut().state.shelf.wait_inspections();
            h.run();
            h.snapshot(format!("fx_image_stage_{name}_{}", lang.pick("ja", "en")));
            // マスクの段は欄が長いので、下の半分も
            if target == FilterTarget::Mask {
                h.state_mut().state.m2.props_scroll = 600.0;
                h.run();
                for text in right_texts(&h) {
                    assert_plain(&format!("{lang:?} {name} 下"), &text);
                    assert!(lang == Lang::Ja || !has_japanese(&text), "{text}");
                }
                h.snapshot(format!(
                    "fx_image_stage_{name}_lower_{}",
                    lang.pick("ja", "en")
                ));
            }
            results.extend_harness(&mut h);
        }
    }
    results.unwrap();
}
