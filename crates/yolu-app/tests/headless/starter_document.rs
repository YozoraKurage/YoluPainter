//! 新しい文書の初めの形（一番下に白の単色、その上に空のレイヤー。選ぶのは空のレイヤー）と、単色を選んだレイヤーの下に作る置き場
//! （画面は描かない）。起動の文書・ファイルの「新しいプロジェクト」・レイヤーセットの追加・言語を替えた時の名前・保存と開き直し・Color の PNG の書き出し。
use crate::common::drive::Drive;
use crate::common::tmp::test_dir;

use yolu_app::engine::{Channel, ChannelKind, Document, LayerId, LayerKind, Rgba8};
use yolu_app::export::ExportAction;
use yolu_app::lang::Lang;
use yolu_app::m2::layersets::{self, LayerSetEdit};
use yolu_app::m2::{new_channel_info, Edit};
use yolu_app::state::{is_starter_shape, Action, AppState};

const WHITE: Rgba8 = Rgba8::new(255, 255, 255, 255);

/// 文書が初めの形（下から [白の単色、空の「レイヤー 1」]）で、履歴が空であることを細かく確かめる。
fn assert_starter(doc: &Document, lang: Lang, why: &str) {
    let (fill_name, layer_name) = match lang {
        Lang::Ja => ("塗りつぶし 1", "レイヤー 1"),
        Lang::En => ("Fill 1", "Layer 1"),
    };
    assert_eq!(doc.layers().len(), 2, "{why}");
    let fill = &doc.layers()[0];
    assert_eq!(fill.kind(), LayerKind::Fill, "{why}");
    assert_eq!(fill.name(), fill_name, "{why}");
    assert_eq!(
        fill.fill_values().collect::<Vec<_>>(),
        [(Channel::Color, WHITE)],
        "{why}: 値を持つのは Color だけで不透明な白"
    );
    let layer = &doc.layers()[1];
    assert_eq!(layer.kind(), LayerKind::Raster, "{why}");
    assert_eq!(layer.name(), layer_name, "{why}");
    assert!(!yolu_app::engine::layer_has_pixels(layer), "{why}: 空");
    assert!(!doc.can_undo(), "{why}: 履歴は空");
    assert!(is_starter_shape(doc), "{why}");
}

fn top(s: &AppState) -> LayerId {
    s.doc().layers().last().unwrap().id()
}

fn names(s: &AppState) -> Vec<String> {
    s.doc()
        .layers()
        .iter()
        .map(|l| l.name().to_owned())
        .collect()
}

fn ids(s: &AppState) -> Vec<LayerId> {
    s.doc().layers().iter().map(|l| l.id()).collect()
}

#[test]
fn the_startup_document_is_a_white_fill_under_an_empty_layer_in_both_languages() {
    for lang in [Lang::Ja, Lang::En] {
        let s = AppState::starter_in(lang);
        assert_starter(s.doc(), lang, &format!("{lang:?}"));
        assert_eq!(s.selected_layer(), Some(top(&s)), "選ぶのは空のレイヤー");
        assert!(!s.modified() && !s.app().shows_modified());
        assert_eq!(
            (s.doc().width(), s.doc().height()),
            (2048, 2048),
            "既定の大きさ"
        );
        assert_eq!(s.sets().len(), 1);
    }
}

#[test]
fn the_test_foundation_keeps_the_single_layer_form() {
    let s = AppState::new_in(64, 64, Lang::Ja);
    assert_eq!(names(&s), ["レイヤー 1"]);
    assert!(!is_starter_shape(s.doc()));
}

#[test]
fn the_new_project_command_starts_from_the_starter_form_and_does_not_ask_to_save_it() {
    let mut s = AppState::new_in(64, 64, Lang::En);
    let layer = top(&s);
    s.doc_mut()
        .set_pixel(layer, 1, 1, Rgba8::new(9, 9, 9, 255))
        .unwrap();
    s.set_modified(true);
    s.act(Action::NewProject);
    assert_starter(s.doc(), Lang::En, "新しいプロジェクト");
    assert_eq!(s.selected_layer(), Some(top(&s)));
    assert!(
        !s.modified() && !s.app().shows_modified(),
        "初めの形は保存を聞かない"
    );
    assert_eq!(s.sets().len(), 1);
    assert_starter(s.set_doc(0), Lang::En, "セット");
}

#[test]
fn a_new_layer_set_holds_a_white_fill_under_an_empty_layer_in_one_undo() {
    let mut s = AppState::starter_in(Lang::Ja);
    let before = ids(&s);
    s.act(Action::M2(Edit::LayerSets(LayerSetEdit::New)));
    assert_eq!(s.doc().layer_sets().len(), 2);
    let set = s.doc().layer_sets()[1].id;
    let in_set: Vec<_> = s
        .doc()
        .layers()
        .iter()
        .filter(|l| l.layer_set() == set)
        .collect();
    assert_eq!(
        in_set
            .iter()
            .map(|l| (l.kind(), l.name()))
            .collect::<Vec<_>>(),
        [
            (LayerKind::Fill, "塗りつぶし 1"),
            (LayerKind::Raster, "レイヤー 1")
        ],
        "下から白の単色、空のレイヤー"
    );
    assert_eq!(
        in_set[0].fill_values().collect::<Vec<_>>(),
        [(Channel::Color, WHITE)]
    );
    let empty = in_set[1].id();
    assert_eq!(s.selected_layer(), Some(empty), "選ぶのは空のレイヤー");
    let after = ids(&s);
    s.act(Action::Undo);
    assert_eq!(
        s.doc().layer_sets().len(),
        1,
        "1 回の取り消しでレイヤーセットごと無くなる"
    );
    assert_eq!(ids(&s), before);
    s.act(Action::Redo);
    assert_eq!(s.doc().layer_sets().len(), 2);
    assert_eq!(ids(&s), after, "やり直しで同じ形に戻る");
    assert_eq!(top_of_set(&s, set), empty);
}

fn top_of_set(s: &AppState, set: yolu_app::engine::LayerSetId) -> LayerId {
    layersets::top_layer_of(s.doc(), set).unwrap()
}

#[test]
fn the_layer_set_of_a_user_channel_has_no_white_fill() {
    let mut s = AppState::starter_in(Lang::Ja);
    s.act(Action::M2(Edit::AddChannel(new_channel_info(
        "影の強度".into(),
        ChannelKind::Scalar,
    ))));
    let set = s.app().layer_set();
    assert_eq!(s.doc().layer_sets().len(), 2);
    let kinds: Vec<_> = s
        .doc()
        .layers()
        .iter()
        .filter(|l| l.layer_set() == set)
        .map(|l| (l.kind(), l.name().to_owned()))
        .collect();
    assert_eq!(kinds, [(LayerKind::Raster, "レイヤー 1".to_owned())]);
}

#[test]
fn changing_the_language_renames_both_layers_of_an_untouched_starter_but_not_after_drawing() {
    let mut s = AppState::starter_in(Lang::Ja);
    s.app_mut().set_language(Lang::En);
    assert_starter(s.doc(), Lang::En, "言語を替えた");
    s.app_mut().set_language(Lang::Ja);
    assert_starter(s.doc(), Lang::Ja, "戻した");
    // 描いたあとは替えない
    let layer = top(&s);
    s.doc_mut()
        .set_pixel(layer, 2, 2, Rgba8::new(1, 2, 3, 255))
        .unwrap();
    s.app_mut().set_language(Lang::En);
    assert_eq!(names(&s), ["塗りつぶし 1", "レイヤー 1"]);
    // 名前を付けた物も替えない
    let mut t = AppState::starter_in(Lang::Ja);
    let empty = top(&t);
    t.doc_mut().set_layer_name(empty, "肌").unwrap();
    t.app_mut().set_language(Lang::En);
    assert_eq!(names(&t), ["塗りつぶし 1", "肌"]);
    // 1 枚の形も今までどおり
    let mut u = AppState::new_in(8, 8, Lang::Ja);
    u.app_mut().set_language(Lang::En);
    assert_eq!(names(&u), ["Layer 1"]);
}

/// 1 回のストロークで描く（履歴に 1 段入る）。
fn draw_dab(s: &mut AppState, layer: LayerId) {
    let mut stroke = s.begin_paint_stroke(layer, false).unwrap();
    stroke
        .add_point(s.doc_mut(), 4.0, 4.0, 1.0, yolu_app::engine::DVec2::ZERO)
        .unwrap();
    s.doc_mut().end_stroke(stroke).unwrap();
}

/// 描いて Undo した文書（Redo の段がある）は、言語を替えても名前を替えず、履歴を消さない（Redo で描いた物に戻る）。
#[test]
fn changing_the_language_after_undo_keeps_the_redo_step_in_both_forms() {
    for starter in [true, false] {
        let mut s = if starter {
            AppState::starter_in(Lang::Ja)
        } else {
            AppState::new_in(8, 8, Lang::Ja)
        };
        let layer = top(&s);
        draw_dab(&mut s, layer);
        s.act(Action::Undo);
        assert!(s.doc().can_redo() && !s.doc().can_undo(), "{starter}");
        let before = names(&s);
        s.app_mut().set_language(Lang::En);
        assert!(s.doc().can_redo(), "{starter}: 言語を替えても Redo が残る");
        assert_eq!(
            names(&s),
            before,
            "{starter}: Redo のある文書は名前を替えない"
        );
        s.act(Action::Redo);
        assert!(
            yolu_app::engine::layer_has_pixels(s.doc().layer(layer).unwrap()),
            "{starter}: Redo で描いた物に戻る"
        );
    }
}

/// 初めの形の見分けは、履歴が空でも、Redo の段・名前・表示・不透明度・合成・マスク・効果・ロックのどれかが違えば外れる。
#[test]
fn a_starter_with_any_one_thing_changed_is_not_a_starter_even_without_history() {
    use yolu_app::engine::BlendMode;
    use yolu_core::{EffectSettings, FilterSpec, FilterTarget};
    let mut s = AppState::starter_in(Lang::Ja);
    let (fill, layer) = (s.doc().layers()[0].id(), top(&s));
    assert!(is_starter_shape(s.doc()));
    // 履歴を消した後でも、1 つ変えれば初めの形ではない
    let mut check = |why: &str, change: &dyn Fn(&mut Document), undo: &dyn Fn(&mut Document)| {
        change(s.doc_mut());
        let _ = s.doc_mut().clear_history();
        assert!(!is_starter_shape(s.doc()), "{why}");
        undo(s.doc_mut());
        let _ = s.doc_mut().clear_history();
        assert!(is_starter_shape(s.doc()), "{why}: 戻せば初めの形");
    };
    for (id, which) in [(fill, "単色"), (layer, "空のレイヤー")] {
        check(
            &format!("{which}の名前"),
            &|d| d.set_layer_name(id, "肌").unwrap(),
            &|d| {
                let name = if id == fill {
                    "塗りつぶし 1"
                } else {
                    "レイヤー 1"
                };
                d.set_layer_name(id, name).unwrap()
            },
        );
        check(
            &format!("{which}の表示"),
            &|d| d.set_layer_visible(id, false).unwrap(),
            &|d| d.set_layer_visible(id, true).unwrap(),
        );
        check(
            &format!("{which}の不透明度"),
            &|d| d.set_layer_opacity(id, 0.5, false).unwrap(),
            &|d| d.set_layer_opacity(id, 1.0, false).unwrap(),
        );
        check(
            &format!("{which}の合成"),
            &|d| d.set_layer_blend_mode(id, BlendMode::Multiply).unwrap(),
            &|d| d.set_layer_blend_mode(id, BlendMode::Normal).unwrap(),
        );
        check(
            &format!("{which}のロック"),
            &|d| {
                d.set_layer_locks(id, yolu_core::LayerLocks::PIXELS)
                    .unwrap()
            },
            &|d| d.set_layer_locks(id, yolu_core::LayerLocks::NONE).unwrap(),
        );
        check(
            &format!("{which}のマスク"),
            &|d| d.add_black_layer_mask(id).unwrap(),
            &|d| d.remove_layer_mask(id).unwrap(),
        );
        let made = std::cell::Cell::new(None);
        check(
            &format!("{which}の効果"),
            &|d| {
                let spec = FilterSpec::new(EffectSettings::invert()).channels(&[Channel::Color]);
                made.set(Some(d.add_filter(id, FilterTarget::Content, spec).unwrap()));
            },
            &|d| d.remove_filter(id, made.get().unwrap()).unwrap(),
        );
    }
    // 英語の既定の名前でも初めの形
    let e = AppState::starter_in(Lang::En);
    assert!(is_starter_shape(e.doc()));
    // Redo の段が残る初めの形は、初めの形ではない
    let mut r = AppState::starter_in(Lang::Ja);
    let l = top(&r);
    r.doc_mut()
        .set_pixel(l, 1, 1, Rgba8::new(1, 1, 1, 255))
        .unwrap();
    r.act(Action::Undo);
    assert!(!is_starter_shape(r.doc()), "Redo の段がある");
}

/// Color を使っていないレイヤーだけの文書に足すレイヤーセットには、白の単色を入れない（Color は文書から消せないので、分けるのは使うチャンネル）。
#[test]
fn a_layer_set_added_to_a_document_that_does_not_use_color_has_no_white_fill() {
    let mut s = AppState::new_in(16, 16, Lang::Ja);
    let only = top(&s);
    s.doc_mut()
        .set_channel_enabled(only, Channel::Color, false)
        .unwrap();
    s.doc_mut()
        .set_channel_enabled(only, Channel::Roughness, true)
        .unwrap();
    assert!(!yolu_app::newproject::used_channels(s.doc()).contains(&Channel::Color));
    s.act(Action::M2(Edit::LayerSets(LayerSetEdit::New)));
    assert!(
        s.doc().layers().iter().all(|l| l.kind() != LayerKind::Fill),
        "Color を使わない文書のレイヤーセットに白は入らない"
    );
    // Color を使う文書は、今までどおり白が入る
    let mut t = AppState::starter_in(Lang::Ja);
    t.act(Action::M2(Edit::LayerSets(LayerSetEdit::New)));
    assert_eq!(
        t.doc()
            .layers()
            .iter()
            .filter(|l| l.kind() == LayerKind::Fill)
            .count(),
        2
    );
}

/// 下から a・b・c の 3 枚の文書（選んでいるのは a）。
fn three() -> (AppState, [LayerId; 3]) {
    let mut s = AppState::new(64, 64);
    let a = s.selected_layer().unwrap();
    s.act(Action::NewLayer);
    let b = s.selected_layer().unwrap();
    s.act(Action::NewLayer);
    let c = s.selected_layer().unwrap();
    s.set_selected_layer(Some(a));
    (s, [a, b, c])
}

/// 単色を足して、足した単色（選ばれている）と、足す前の並びを返す。
fn new_fill(s: &mut AppState) -> (LayerId, Vec<LayerId>) {
    let before = ids(s);
    s.act(Action::M2(Edit::NewFill));
    let fill = s.selected_layer().expect("作った単色を選ぶ");
    assert_eq!(s.doc().layer(fill).unwrap().kind(), LayerKind::Fill);
    (fill, before)
}

fn undo_restores(s: &mut AppState, before: &[LayerId]) {
    s.act(Action::Undo);
    assert_eq!(ids(s), before, "1 回の取り消しで元の並び");
}

#[test]
fn a_new_fill_goes_right_below_the_selected_layer_in_one_undo() {
    // 真ん中
    let (mut s, [a, b, c]) = three();
    s.set_selected_layer(Some(b));
    let (fill, before) = new_fill(&mut s);
    assert_eq!(ids(&s), [a, fill, b, c]);
    undo_restores(&mut s, &before);
    // 一番下（その下に入る）
    let (mut s, [a, b, c]) = three();
    let (fill, before) = new_fill(&mut s);
    assert_eq!(ids(&s), [fill, a, b, c]);
    undo_restores(&mut s, &before);
    // 一番上
    let (mut s, [a, b, c]) = three();
    s.set_selected_layer(Some(c));
    let (fill, _) = new_fill(&mut s);
    assert_eq!(ids(&s), [a, b, fill, c]);
}

#[test]
fn a_new_fill_with_nothing_selected_goes_to_the_top_as_before() {
    let (mut s, [a, b, c]) = three();
    s.set_selected_layer(None);
    let (fill, _) = new_fill(&mut s);
    assert_eq!(ids(&s), [a, b, c, fill]);
}

#[test]
fn a_new_fill_inside_a_group_stays_in_the_group_below_the_selected_child() {
    let (mut s, [a, b, _]) = three();
    s.set_selected_layer(Some(b));
    s.act(Action::M2(Edit::GroupSelected));
    let group = s.selected_layer().unwrap();
    assert_eq!(s.doc().layer(b).unwrap().parent(), Some(group));
    s.set_selected_layer(Some(b));
    let (fill, before) = new_fill(&mut s);
    assert_eq!(
        s.doc().layer(fill).unwrap().parent(),
        Some(group),
        "同じグループの中"
    );
    let order = ids(&s);
    assert!(
        order.iter().position(|x| *x == fill).unwrap()
            < order.iter().position(|x| *x == b).unwrap(),
        "選んだ子のすぐ下"
    );
    assert_eq!(s.doc().children_of(Some(group)).unwrap(), [fill, b]);
    assert_eq!(s.doc().layer(a).unwrap().parent(), None);
    undo_restores(&mut s, &before);
}

#[test]
fn a_new_fill_with_a_group_selected_goes_below_the_group_among_its_siblings() {
    let (mut s, [a, b, c]) = three();
    s.set_selected_layer(Some(b));
    s.act(Action::M2(Edit::GroupSelected));
    let group = s.selected_layer().unwrap();
    s.set_selected_layer(Some(group));
    let (fill, before) = new_fill(&mut s);
    assert_eq!(s.doc().layer(fill).unwrap().parent(), None);
    let top_level = s.doc().children_of(None).unwrap();
    assert_eq!(top_level, [a, fill, group, c], "グループの下の兄弟");
    assert_eq!(
        s.doc().children_of(Some(group)).unwrap(),
        [b],
        "グループの中身は変わらない"
    );
    undo_restores(&mut s, &before);
}

#[test]
fn a_new_fill_in_the_second_layer_set_stays_in_that_set() {
    let mut s = AppState::starter_in(Lang::Ja);
    s.act(Action::M2(Edit::LayerSets(LayerSetEdit::New)));
    let set = s.doc().layer_sets()[1].id;
    let empty = s.selected_layer().unwrap();
    let (fill, before) = new_fill(&mut s);
    assert_eq!(s.doc().layer(fill).unwrap().layer_set(), set);
    let in_set: Vec<_> = s
        .doc()
        .layers()
        .iter()
        .filter(|l| l.layer_set() == set)
        .map(|l| l.id())
        .collect();
    assert_eq!(in_set.len(), 3);
    assert_eq!(in_set[1], fill, "白の単色と空のレイヤーのあいだ");
    assert_eq!(in_set[2], empty);
    undo_restores(&mut s, &before);
    // そのレイヤーセットの一番下（白の単色）を選んでいても、その下に入る
    let white = s
        .doc()
        .layers()
        .iter()
        .find(|l| l.layer_set() == set && l.kind() == LayerKind::Fill)
        .unwrap()
        .id();
    s.set_selected_layer(Some(white));
    let (fill, _) = new_fill(&mut s);
    let in_set: Vec<_> = s
        .doc()
        .layers()
        .iter()
        .filter(|l| l.layer_set() == set)
        .map(|l| l.id())
        .collect();
    assert_eq!(in_set, [fill, white, empty]);
}

#[test]
fn the_starter_survives_a_save_and_the_color_export_is_opaque_white() {
    let dir = test_dir("starter");
    let path = dir.join("starter.ylp");
    let mut s = AppState::starter_in(Lang::Ja);
    s.act(Action::SaveProjectAs(path.clone()));
    s.wait_save();
    assert!(s.message().starts_with("保存しました"), "{}", s.message());

    let mut t = AppState::new_in(8, 8, Lang::Ja);
    t.act(Action::OpenProject(path));
    assert!(t.message().starts_with("開きました"), "{}", t.message());
    assert_eq!(names(&t), ["塗りつぶし 1", "レイヤー 1"]);
    assert_eq!(
        t.doc().layers()[0].fill_values().collect::<Vec<_>>(),
        [(Channel::Color, WHITE)],
        "白の単色が残る"
    );
    assert_eq!(t.doc().layers()[1].kind(), LayerKind::Raster);

    let png = dir.join("color.png");
    t.act(Action::Export(ExportAction::ChannelTo(png.clone())));
    t.wait_export();
    let image = image::open(&png).expect("書き出した PNG").to_rgba8();
    assert_eq!(image.dimensions(), (2048, 2048));
    assert!(
        image
            .pixels()
            .step_by(997)
            .all(|p| p.0 == [255, 255, 255, 255]),
        "何も描いていなければ不透明な白"
    );
}
