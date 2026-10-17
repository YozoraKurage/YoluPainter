//! レイヤーセットの状態と操作（画面は描かない）: 今のレイヤーセットは選んでいるレイヤーに付いてくる・レイヤーセットを選ぶと一覧と描くチャンネルが一度に替わる・
//! 新しいレイヤーは今のレイヤーセットに入る・出力の割り当ては 1 回の取り消し・新しいユーザーチャンネルは自分のレイヤーセットを持つ・
//! 最後の 1 つは消せない・今のレイヤーセットは保存しない。
use crate::common::drive::Drive;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use yolu_app::engine::{Channel, ChannelKind, HistoryKind, LayerId, LayerSetId, Rgba8};
use yolu_app::lang::Lang;
use yolu_app::layermenu::Op;
use yolu_app::m2::layersets::{self, LayerSetEdit};
use yolu_app::m2::{self, new_channel_info, Edit, UiOp};
use yolu_app::shelf::{is_builtin, ItemKind, PlaceTarget, ShelfOp};
use yolu_app::state::{Action, AppState};
use yolu_app::textlayer::TextAction;
use yolu_core::generator::Shape;

const RED: Rgba8 = Rgba8::new(220, 20, 30, 255);
const BLUE: Rgba8 = Rgba8::new(10, 40, 230, 255);

/// 最初のレイヤーセットと、その最初のレイヤー。
fn first(s: &AppState) -> (LayerSetId, LayerId) {
    let set = s.doc().layer_sets()[0].id;
    (
        set,
        layersets::top_layer_of(s.doc(), set).expect("最初のレイヤー"),
    )
}

/// ユーザーチャンネル「影の強度」を足す（出力は、そのチャンネルの名前のレイヤーセットへ。今のレイヤーセットもそこへ移る）。
fn add_shadow(s: &mut AppState) -> (Channel, LayerSetId) {
    s.act(Action::M2(Edit::AddChannel(new_channel_info(
        "影の強度".into(),
        ChannelKind::Scalar,
    ))));
    (s.m2().paint_channel, s.app().layer_set())
}

fn set_names(s: &AppState) -> Vec<String> {
    (0..s.doc().layer_sets().len())
        .map(|i| layersets::display_name(Lang::Ja, s.doc(), i))
        .collect()
}

fn row_ids(s: &AppState) -> Vec<LayerId> {
    m2::visible_rows_in(s.doc(), &s.m2().collapsed, s.app().layer_set())
        .iter()
        .map(|r| r.id)
        .collect()
}

#[test]
fn a_new_user_channel_gets_a_layer_set_of_its_name_with_one_empty_layer_in_one_undo() {
    let mut s = AppState::new(64, 64);
    let (base, base_layer) = first(&s);
    assert_eq!(
        set_names(&s),
        ["マテリアル"],
        "名前の空の最初のレイヤーセットは「マテリアル」"
    );
    let (channel, set) = add_shadow(&mut s);
    assert_ne!(set, base);
    assert!(!channel.is_standard());
    assert_eq!(set_names(&s), ["マテリアル", "影の強度"]);
    assert_eq!(
        s.doc().channel_output(channel).unwrap(),
        Some(set),
        "出力はそのレイヤーセットへ"
    );
    assert_eq!(s.doc().channel_output(Channel::Color).unwrap(), Some(base));
    // 描く先: そのレイヤーセットの空のレイヤー。描くチャンネルも見せるチャンネルもそのチャンネル
    let layer = s.selected_layer().expect("選んだレイヤー");
    assert_eq!(s.doc().layer_set_of(layer).unwrap(), set);
    assert_eq!(s.app().layer_set(), set);
    assert_eq!(s.m2().display_channel, channel);
    assert_eq!(s.doc().layers().len(), 2);
    assert_eq!(s.doc().history().last(), Some(HistoryKind::Channel));
    // 1 回の取り消しで、チャンネルもレイヤーセットも戻る（選ぶレイヤーは最初のレイヤーセットへ）
    s.act(Action::Undo);
    assert_eq!(s.doc().layer_sets().len(), 1);
    assert!(s.doc().channel_info(channel).is_none());
    assert_eq!(s.doc().layers().len(), 1);
    assert_eq!(s.app().layer_set(), base);
    assert_eq!(s.selected_layer(), Some(base_layer));
    s.act(Action::Redo);
    assert_eq!(set_names(&s), ["マテリアル", "影の強度"]);
    assert_eq!(s.doc().channel_output(channel).unwrap(), Some(set));
}

#[test]
fn choosing_a_layer_set_switches_the_list_the_layer_and_the_paint_channel_at_once() {
    let mut s = AppState::new(64, 64);
    let (base, base_layer) = first(&s);
    let (shadow_channel, shadow) = add_shadow(&mut s);
    let shadow_layer = s.selected_layer().unwrap();
    assert_eq!(row_ids(&s), [shadow_layer]);
    // 最初のレイヤーセットへ: 一覧・選ぶレイヤー・描くチャンネル（そのレイヤーセットが出力している最初のチャンネル = カラー）
    s.act(Action::M2Ui(UiOp::SelectLayerSet(base)));
    assert_eq!(s.app().layer_set(), base);
    assert_eq!(row_ids(&s), [base_layer]);
    assert_eq!(s.selected_layer(), Some(base_layer));
    assert_eq!(s.m2().paint_channel, Channel::Color);
    assert_eq!(
        s.m2().display_channel,
        Channel::Color,
        "見せるチャンネルが描くチャンネルに付いていたときは一緒に替わる"
    );
    // 影のレイヤーセットへ戻す: 描くチャンネルは影の強度
    s.act(Action::M2Ui(UiOp::SelectLayerSet(shadow)));
    assert_eq!(s.selected_layer(), Some(shadow_layer));
    assert_eq!(s.m2().paint_channel, shadow_channel);
    // どのチャンネルも出力していないレイヤーセットでは、描くチャンネルは今のまま
    s.act(Action::M2(Edit::LayerSets(LayerSetEdit::New)));
    let draft = s.app().layer_set();
    assert!(layersets::outputs(s.doc(), draft).is_empty());
    assert_eq!(s.m2().paint_channel, shadow_channel);
    // 描いている間は替えない
    s.act(Action::M2Ui(UiOp::SelectLayerSet(base)));
    assert_eq!(s.app().layer_set(), base);
}

#[test]
fn new_layers_go_into_the_current_set_and_selecting_a_layer_elsewhere_follows_it() {
    let mut s = AppState::new(64, 64);
    let (base, base_layer) = first(&s);
    let (_, shadow) = add_shadow(&mut s);
    s.act(Action::NewLayer);
    let added = s.selected_layer().unwrap();
    assert_eq!(
        s.doc().layer_set_of(added).unwrap(),
        shadow,
        "今のレイヤーセットに入る"
    );
    assert_eq!(row_ids(&s).len(), 2);
    assert_eq!(s.doc().history().last(), Some(HistoryKind::AddLayer));
    // 別のレイヤーセットのレイヤーを選ぶ操作が来たら、そのレイヤーセットが今のレイヤーセット
    s.set_selected_layer(Some(base_layer));
    assert_eq!(s.app().layer_set(), base);
    assert_eq!(row_ids(&s), [base_layer]);
    // グループ・塗りつぶしも、選んでいるレイヤーのレイヤーセットに入る
    s.act(Action::M2(Edit::NewGroup));
    let group = s.selected_layer().unwrap();
    assert_eq!(s.doc().layer_set_of(group).unwrap(), base);
}

#[test]
fn an_empty_current_set_takes_new_layers_in_one_undo_each() {
    let mut s = AppState::new(64, 64);
    let (_, shadow) = add_shadow(&mut s);
    // 影のレイヤーセットのレイヤーを全部消す（最初のレイヤーセットにレイヤーがあるので消せる）: 選ぶレイヤーは無く、今のレイヤーセットは影のまま
    s.act(Action::DeleteLayer);
    assert!(layersets::top_layer_of(s.doc(), shadow).is_none());
    assert_eq!(s.selected_layer(), None);
    assert_eq!(s.app().layer_set(), shadow);
    assert!(row_ids(&s).is_empty());
    let steps = s.doc().undo_count();
    s.act(Action::NewLayer);
    let layer = s.selected_layer().unwrap();
    assert_eq!(
        s.doc().layer_set_of(layer).unwrap(),
        shadow,
        "空のレイヤーセットへ足す"
    );
    assert_eq!(s.doc().undo_count(), steps + 1);
    // グループ・塗りつぶし・調整は、足してから移すが、1 回の取り消し
    s.act(Action::DeleteLayer);
    for edit in [
        Edit::NewGroup,
        Edit::NewFill,
        Edit::NewAdjustment(m2::AdjustmentKind::Invert),
    ] {
        s.act(Action::M2(edit));
        let added = s.selected_layer().unwrap();
        assert_eq!(s.doc().layer_set_of(added).unwrap(), shadow);
        let steps = s.doc().undo_count();
        s.act(Action::Undo);
        assert_eq!(s.doc().undo_count(), steps - 1);
        assert!(s.doc().layer(added).is_none(), "1 回の取り消しで消える");
        assert_eq!(
            s.app().layer_set(),
            shadow,
            "取り消しても今のレイヤーセットは替わらない"
        );
    }
}

#[test]
fn the_output_assignment_changes_the_final_result_and_one_undo_restores_it() {
    let mut s = AppState::new(64, 64);
    let (base, base_layer) = first(&s);
    let (_, shadow) = add_shadow(&mut s);
    s.doc_mut().set_pixel(base_layer, 3, 3, RED).unwrap();
    let before = s
        .doc()
        .composite_channel(Channel::Color, s.doc().bounds())
        .unwrap();
    let px = |s: &AppState| s.doc().composite_pixel(Channel::Color, 3, 3).unwrap();
    assert_eq!(px(&s), RED);
    // カラーの出力を空の影のレイヤーセットへ: 最終のカラーは透明、3D・書き出しが読む合成が替わる。2D は今のレイヤーセットの合成のまま
    let steps = s.doc().undo_count();
    s.act(Action::M2(Edit::LayerSets(LayerSetEdit::Output {
        channel: Channel::Color,
        set: Some(shadow),
    })));
    assert_eq!(s.doc().undo_count(), steps + 1, "1 回の取り消し");
    assert_eq!(s.doc().history().last(), Some(HistoryKind::LayerSets));
    assert_eq!(px(&s), Rgba8::TRANSPARENT);
    assert_eq!(
        s.doc()
            .composite_set_pixel(base, Channel::Color, 3, 3)
            .unwrap(),
        RED,
        "元のレイヤーセットの合成はそのまま"
    );
    // 「なし」: そのチャンネルの既定値
    s.act(Action::M2(Edit::LayerSets(LayerSetEdit::Output {
        channel: Channel::Color,
        set: None,
    })));
    let default = s.doc().channel_info(Channel::Color).unwrap().default;
    assert_eq!(px(&s), default);
    s.act(Action::Undo);
    assert_eq!(px(&s), Rgba8::TRANSPARENT);
    s.act(Action::Undo);
    assert_eq!(px(&s), RED);
    assert_eq!(
        s.doc()
            .composite_channel(Channel::Color, s.doc().bounds())
            .unwrap(),
        before
    );
}

#[test]
fn the_last_layer_set_cannot_be_removed_and_removing_the_current_one_moves_to_a_neighbour() {
    let mut s = AppState::new(64, 64);
    let (base, base_layer) = first(&s);
    let remove = |s: &mut AppState, set| {
        s.act(Action::M2(Edit::LayerSets(LayerSetEdit::Remove(set))));
    };
    // 最後の 1 つ: 断る（理由つき）。何も変えない
    let revision = s.doc().revision();
    remove(&mut s, base);
    assert_eq!(s.doc().layer_sets().len(), 1);
    assert_eq!(s.doc().revision(), revision);
    assert_eq!(s.message(), "最後のレイヤーセットは削除できません。");
    // 2 つ目を作って消す: 今のレイヤーセットは残った 1 つ。割り当ては「なし」になり、1 回の取り消しで戻る
    let (channel, shadow) = add_shadow(&mut s);
    remove(&mut s, shadow);
    assert_eq!(s.doc().layer_sets().len(), 1);
    assert_eq!(s.app().layer_set(), base);
    assert_eq!(s.selected_layer(), Some(base_layer));
    assert_eq!(s.doc().channel_output(channel).unwrap(), None);
    s.act(Action::Undo);
    assert_eq!(s.doc().layer_sets().len(), 2);
    assert_eq!(s.doc().channel_output(channel).unwrap(), Some(shadow));
    // 3 つのとき、真ん中の今のレイヤーセットを消すと、一覧の次のレイヤーセット
    s.act(Action::M2(Edit::LayerSets(LayerSetEdit::New)));
    let third = s.app().layer_set();
    s.act(Action::M2Ui(UiOp::SelectLayerSet(shadow)));
    remove(&mut s, shadow);
    assert_eq!(s.app().layer_set(), third);
    assert_eq!(set_names(&s).len(), 2);
}

#[test]
fn renaming_duplicating_and_reordering_use_unique_names_and_one_undo_each() {
    let mut s = AppState::new(64, 64);
    let (base, _) = first(&s);
    let rename = |s: &mut AppState, set, name: &str| {
        s.act(Action::M2(Edit::LayerSets(LayerSetEdit::Rename(
            set,
            name.into(),
        ))));
    };
    s.act(Action::M2(Edit::LayerSets(LayerSetEdit::Duplicate(base))));
    let copy = s.app().layer_set();
    assert_ne!(copy, base);
    assert_eq!(
        set_names(&s),
        ["マテリアル", "マテリアル 2"],
        "名前は重ならない"
    );
    assert_eq!(s.doc().history().last(), Some(HistoryKind::LayerSets));
    // 名前の変更: 空・ほかのレイヤーセットと同じ（大文字小文字を区別しない）は断る
    rename(&mut s, copy, "  ");
    assert_eq!(set_names(&s)[1], "マテリアル 2");
    rename(&mut s, copy, "マテリアル");
    assert_eq!(set_names(&s)[1], "マテリアル 2");
    assert!(s.message().contains("すでにあります"), "{}", s.message());
    rename(&mut s, copy, " 影のマスク ");
    assert_eq!(set_names(&s)[1], "影のマスク");
    // 並べ替え
    s.act(Action::M2(Edit::LayerSets(LayerSetEdit::Move(copy, 0))));
    assert_eq!(set_names(&s), ["影のマスク", "マテリアル"]);
    s.act(Action::Undo);
    assert_eq!(set_names(&s), ["マテリアル", "影のマスク"]);
    s.act(Action::Undo);
    assert_eq!(set_names(&s)[1], "マテリアル 2");
}

#[test]
fn moving_layers_to_another_set_keeps_the_view_on_the_current_set() {
    let mut s = AppState::new(64, 64);
    let (base, base_layer) = first(&s);
    s.act(Action::NewLayer);
    let second = s.selected_layer().unwrap();
    let (_, shadow) = add_shadow(&mut s);
    s.act(Action::M2Ui(UiOp::SelectLayerSet(base)));
    s.act(Action::M2(Edit::LayerSets(LayerSetEdit::MoveLayers {
        ids: vec![second],
        set: shadow,
    })));
    assert_eq!(s.doc().layer_set_of(second).unwrap(), shadow);
    assert_eq!(s.doc().history().last(), Some(HistoryKind::LayerSets));
    // 選んでいたレイヤーが移ったので、今のレイヤーセットに残ったレイヤーを選び直し、画面は移し先へ飛ばない
    assert_eq!(s.app().layer_set(), base);
    assert_eq!(s.selected_layer(), Some(base_layer));
    s.act(Action::Undo);
    assert_eq!(s.doc().layer_set_of(second).unwrap(), base);
}

#[test]
fn visible_layers_merge_only_inside_the_current_set() {
    let mut s = AppState::new(64, 64);
    let (base, base_layer) = first(&s);
    s.doc_mut().set_pixel(base_layer, 1, 1, RED).unwrap();
    s.act(Action::NewLayer);
    let (_, shadow) = add_shadow(&mut s);
    let shadow_layer = s.selected_layer().unwrap();
    s.doc_mut().set_pixel(shadow_layer, 2, 2, BLUE).unwrap();
    s.act(Action::NewLayer);
    s.act(Action::M2Ui(UiOp::SelectLayerSet(base)));
    s.act(Action::M2(Edit::MergeVisible));
    let count = |s: &AppState, set| {
        s.doc()
            .layers()
            .iter()
            .filter(|l| l.layer_set() == set)
            .count()
    };
    assert_eq!(count(&s, base), 1, "今のレイヤーセットだけ 1 枚になる");
    assert_eq!(count(&s, shadow), 2, "ほかのレイヤーセットは変わらない");
    assert_eq!(s.app().layer_set(), base);
}

#[test]
fn each_texture_set_remembers_its_own_layer_set() {
    let mut s = AppState::new(64, 64);
    let (base, _) = first(&s);
    let (_, shadow) = add_shadow(&mut s);
    assert_eq!(s.app().layer_set(), shadow);
    s.app_mut().add_texture_set().unwrap();
    assert_eq!(s.sets().current_index(), 1);
    assert_eq!(
        s.app().layer_set(),
        s.doc().layer_sets()[0].id,
        "新しいテクスチャセットは最初のレイヤーセット"
    );
    s.app_mut().switch_set(0).unwrap();
    assert_eq!(
        s.app().layer_set(),
        shadow,
        "戻ると、前に選んでいたレイヤーセット"
    );
    s.act(Action::M2Ui(UiOp::SelectLayerSet(base)));
    s.app_mut().switch_set(1).unwrap();
    s.app_mut().switch_set(0).unwrap();
    assert_eq!(s.app().layer_set(), base);
}

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> TempDir {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "yolu-app-layersets-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn layer_sets_survive_a_save_and_the_current_one_is_not_saved() {
    let dir = TempDir::new();
    let path = dir.0.join("sets.ylp");
    let mut s = AppState::new_in(64, 64, Lang::Ja);
    let (base, base_layer) = first(&s);
    s.doc_mut().set_pixel(base_layer, 5, 5, RED).unwrap();
    let (channel, shadow) = add_shadow(&mut s);
    let shadow_layer = s.selected_layer().unwrap();
    s.doc_mut().set_pixel(shadow_layer, 6, 6, BLUE).unwrap();
    assert_eq!(s.app().layer_set(), shadow);
    let route_before = s.doc().channel_output(channel).unwrap();
    s.act(Action::SaveProjectAs(path.clone()));
    s.wait_save();
    assert!(s.message().starts_with("保存しました"), "{}", s.message());

    let mut t = AppState::new_in(64, 64, Lang::Ja);
    t.act(Action::OpenProject(path));
    assert!(t.message().starts_with("開きました"), "{}", t.message());
    assert_eq!(set_names(&t), ["マテリアル", "影の強度"]);
    assert_eq!(t.doc().channel_output(channel).unwrap(), route_before);
    assert_eq!(
        t.doc()
            .composite_set_pixel(shadow, Channel::Color, 6, 6)
            .unwrap(),
        BLUE
    );
    // 今のレイヤーセットは保存しない: 開いたときは最初のレイヤーセット（その一番上のレイヤー）
    let opened_first = t.doc().layer_sets()[0].id;
    assert_eq!(opened_first, base);
    assert_eq!(t.app().layer_set(), base);
    assert_eq!(t.selected_layer(), Some(base_layer));
    assert_eq!(
        t.doc()
            .composite_channel(Channel::Color, t.doc().bounds())
            .unwrap(),
        s.doc()
            .composite_channel(Channel::Color, s.doc().bounds())
            .unwrap(),
        "最終の結果は保存の前と同じ"
    );
}

#[test]
fn the_eyedropper_and_the_color_region_read_the_current_sets_composite() {
    use egui::{pos2, vec2, Rect};
    use yolu_app::state::Tool;
    let mut s = AppState::new(64, 64);
    let (base, base_layer) = first(&s);
    s.act(Action::M2(Edit::LayerSets(LayerSetEdit::New)));
    let draft = s.app().layer_set();
    let draft_layer = s.selected_layer().unwrap();
    s.doc_mut().set_pixel(base_layer, 10, 10, RED).unwrap();
    s.doc_mut().set_pixel(draft_layer, 10, 10, BLUE).unwrap();
    s.set_tool(Tool::Eyedropper);
    assert!(s.eyedrop().all_layers, "初めは全レイヤーの合成から取る");
    let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(600.0, 400.0));
    let pick = |s: &mut AppState| {
        let view = s.view().view(rect, 64, 64);
        let at = view.to_screen(10.5, 10.5);
        assert!(
            yolu_app::eyedrop::pick_canvas(s, &view, at),
            "{}",
            s.message()
        );
        let b = |v: f32| (v * 255.0).round() as u8;
        [
            b(s.color().main[0]),
            b(s.color().main[1]),
            b(s.color().main[2]),
        ]
    };
    // 2D のキャンバスが見せている、今のレイヤーセットの合成から取る（下書きは出力に出ていないが、見えている表示）
    assert_eq!(s.app().layer_set(), draft);
    assert_eq!(pick(&mut s), [BLUE.r, BLUE.g, BLUE.b]);
    s.act(Action::M2Ui(UiOp::SelectLayerSet(base)));
    assert_eq!(pick(&mut s), [RED.r, RED.g, RED.b]);
    // 最終の結果（3D・書き出しが読む）はカラーの出力のレイヤーセットのまま
    assert_eq!(
        s.doc().composite_pixel(Channel::Color, 10, 10).unwrap(),
        RED
    );
    s.act(Action::M2Ui(UiOp::SelectLayerSet(draft)));
    assert_eq!(
        s.doc().composite_pixel(Channel::Color, 10, 10).unwrap(),
        RED
    );
}

#[test]
fn moving_a_layer_up_or_down_counts_its_position_inside_its_own_set() {
    let mut s = AppState::new(64, 64);
    let (base, a1) = first(&s);
    let (_, shadow) = add_shadow(&mut s);
    // 並びの途中に、ほかのレイヤーセットのレイヤーが挟まる形: 下から a1・影のレイヤー・a2
    let a2 = s.doc_mut().add_layer_in_set("a2", base, None).unwrap();
    s.act(Action::M2Ui(UiOp::SelectLayerSet(base)));
    s.set_selected_layer(Some(a2));
    let flat: Vec<LayerId> = s.doc().layers().iter().map(|l| l.id()).collect();
    assert_eq!(flat[0], a1);
    assert_eq!(flat[2], a2);
    assert_eq!(s.doc().layer_set_of(flat[1]).unwrap(), shadow);
    let in_base = |s: &AppState| s.doc().children_in_set(base, None).unwrap();
    assert_eq!(in_base(&s), [a1, a2]);
    // 下へ: 同じレイヤーセットの a1 の下
    s.act(Action::LayerDown);
    assert_eq!(
        in_base(&s),
        [a2, a1],
        "下のレイヤーは同じレイヤーセットの a1"
    );
    s.act(Action::LayerUp);
    assert_eq!(in_base(&s), [a1, a2]);
    // 一番上で上へ・一番下で下へ: 何もしない（取り消しの段を作らない）
    let steps = s.doc().undo_count();
    s.act(Action::LayerUp);
    assert_eq!(s.doc().undo_count(), steps);
    s.set_selected_layer(Some(a1));
    s.act(Action::LayerDown);
    assert_eq!(s.doc().undo_count(), steps);
    assert_eq!(in_base(&s), [a1, a2]);
}

/// 影のレイヤーセットを空にする（最初のレイヤーセットにレイヤーがあるので消せる）。選ぶレイヤーは無く、今のレイヤーセットは影のまま。
fn empty_shadow(s: &mut AppState) -> LayerSetId {
    let (_, shadow) = add_shadow(s);
    s.act(Action::DeleteLayer);
    assert!(layersets::top_layer_of(s.doc(), shadow).is_none());
    assert_eq!(s.selected_layer(), None);
    assert_eq!(s.app().layer_set(), shadow);
    shadow
}

#[test]
fn a_shelf_material_goes_into_the_current_set_at_the_top_row_and_when_the_set_is_empty() {
    let mut s = AppState::new(64, 64);
    s.shelf_mut().use_language(Lang::Ja);
    let id = s
        .shelf()
        .visible()
        .iter()
        .find(|r| is_builtin(&r.id) && ItemKind::of(&r.kind) == Some(ItemKind::SmartMaterial))
        .map(|r| r.id.clone())
        .expect("同梱のスマートマテリアル");
    let (base, base_layer) = first(&s);
    let (_, shadow) = add_shadow(&mut s);
    let in_set = |s: &AppState, set| {
        s.doc()
            .layers()
            .iter()
            .filter(|l| l.layer_set() == set)
            .count()
    };
    // 2 つ目のレイヤーセットの一番上の段のレイヤーを選んで置く: そのレイヤーセットに入り、最初のレイヤーセットは変わらない
    let selected = s.selected_layer().unwrap();
    let (base_before, shadow_before) = (in_set(&s, base), in_set(&s, shadow));
    s.act(Action::Shelf(ShelfOp::Place {
        id: id.clone(),
        target: PlaceTarget::Selected,
    }));
    let placed = s.selected_layer().unwrap();
    assert_ne!(placed, selected, "{}", s.message());
    assert_eq!(s.doc().layer_set_of(placed).unwrap(), shadow);
    assert_eq!(
        s.app().layer_set(),
        shadow,
        "今のレイヤーセットは替わらない"
    );
    assert_eq!(in_set(&s, base), base_before);
    assert!(in_set(&s, shadow) > shadow_before);
    assert_eq!(s.doc().layers()[0].id(), base_layer);
    // 一覧の落とした所（一番上の段）でも同じ
    let before = in_set(&s, shadow);
    s.act(Action::Shelf(ShelfOp::Place {
        id: id.clone(),
        target: PlaceTarget::At {
            parent: None,
            position: Some(0),
        },
    }));
    assert_eq!(
        s.doc().layer_set_of(s.selected_layer().unwrap()).unwrap(),
        shadow
    );
    assert!(in_set(&s, shadow) > before);
    assert_eq!(in_set(&s, base), base_before);
    // 空のレイヤーセットへも、1 回の取り消しで
    let mut e = AppState::new(64, 64);
    e.shelf_mut().use_language(Lang::Ja);
    let empty = empty_shadow(&mut e);
    let base_count = in_set(&e, e.doc().layer_sets()[0].id);
    let steps = e.doc().undo_count();
    e.act(Action::Shelf(ShelfOp::Place {
        id,
        target: PlaceTarget::Selected,
    }));
    let placed = e
        .selected_layer()
        .unwrap_or_else(|| panic!("{}", e.message()));
    assert_eq!(e.doc().layer_set_of(placed).unwrap(), empty);
    assert_eq!(e.app().layer_set(), empty);
    assert_eq!(in_set(&e, e.doc().layer_sets()[0].id), base_count);
    assert_eq!(e.doc().undo_count(), steps + 1);
}

#[test]
fn text_path_and_fill_layers_added_to_an_empty_set_go_into_that_set() {
    let mut s = AppState::new(64, 64);
    let shadow = empty_shadow(&mut s);
    let base = s.doc().layer_sets()[0].id;
    let base_count = s
        .doc()
        .layers()
        .iter()
        .filter(|l| l.layer_set() == base)
        .count();
    let check = |s: &mut AppState, what: &str| {
        let added = s
            .selected_layer()
            .unwrap_or_else(|| panic!("{what}: {}", s.message()));
        assert_eq!(s.doc().layer_set_of(added).unwrap(), shadow, "{what}");
        assert_eq!(
            s.app().layer_set(),
            shadow,
            "{what}: 今のレイヤーセットは替わらない"
        );
        assert_eq!(
            s.doc()
                .layers()
                .iter()
                .filter(|l| l.layer_set() == base)
                .count(),
            base_count,
            "{what}"
        );
        // 1 回の取り消しで消え、また空のレイヤーセットに戻る
        s.act(Action::Undo);
        assert!(s.doc().layer(added).is_none(), "{what}");
        assert_eq!(s.app().layer_set(), shadow, "{what}");
        assert_eq!(s.selected_layer(), None, "{what}");
    };
    s.act(Action::Text(TextAction::Begin { x: 4.0, y: 20.0 }));
    s.app_mut().text_typed("Hi".into());
    s.act(Action::Text(TextAction::Commit));
    check(&mut s, "テキストレイヤー");
    // パスは標準のチャンネルにだけ描ける
    s.act(Action::M2Ui(UiOp::PaintChannel(Channel::Color)));
    s.act(Action::LayerMenu(Op::PathLayer));
    check(&mut s, "パスのレイヤー");
    s.act(Action::LayerMenu(Op::FillGradient(Shape::Box)));
    check(&mut s, "グラデーションの塗りつぶし");
    // 打って全部消すと、レイヤーは残らず、今のレイヤーセットはそのまま
    let (layers, steps) = (s.doc().layers().len(), s.doc().undo_count());
    s.act(Action::Text(TextAction::Begin { x: 4.0, y: 20.0 }));
    s.app_mut().text_typed("x".into());
    s.app_mut().text_typed(String::new());
    s.act(Action::Text(TextAction::Commit));
    assert_eq!(s.doc().layers().len(), layers);
    assert_eq!(s.doc().undo_count(), steps);
    assert_eq!(s.app().layer_set(), shadow);
    assert_eq!(s.selected_layer(), None);
}
