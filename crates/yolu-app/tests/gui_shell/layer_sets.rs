//! レイヤーセットの画面（egui_kittest）: テクスチャセットのパネルの行・レイヤーのパネルの頭・チャンネルの行のドロップダウン・キャンバスの右上の部品。
//! 行を押すと、レイヤーセット・一覧・描くチャンネルが一度に替わる。出力のドロップダウンで最終の結果が替わり、1 回の取り消しで戻る。
//! 2D の表示は今のレイヤーセットの合成。レイヤーセットが 1 つの文書は、頭もドロップダウンも出さない。
use crate::common;
use crate::common::drive::Drive;

use common::*;
use egui::{pos2, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::engine::{Channel, ChannelKind, LayerSetId, Rgba8};
use yolu_app::lang::Lang;
use yolu_app::m2::layersets::{self, LayerSetEdit};
use yolu_app::m2::{new_channel_info, Edit, UiOp};
use yolu_app::m2_menu::Popup;
use yolu_app::state::{Action, PopupKind};
use yolu_app::{Tab, YoluApp};

const RED: Rgba8 = Rgba8::new(220, 20, 30, 255);
const BLUE: Rgba8 = Rgba8::new(10, 40, 230, 255);

fn lang_name(lang: Lang) -> &'static str {
    lang.pick("ja", "en")
}

fn apply(h: &mut Harness<'_, YoluApp>, action: Action) {
    h.act(action);
    h.run();
}

fn undo(h: &mut Harness<'_, YoluApp>) {
    key(h, egui::Key::Z, egui::Modifiers::COMMAND);
    h.run();
}

/// 最初のレイヤーセット（マテリアル）・影のマスク・下書きの 3 つ。影の強度のチャンネルを足して、その出力を影のマスクへ（足したときに作った
/// 影の強度のレイヤーセットは消す）。今のレイヤーセットは最初のレイヤーセット。
struct Sets {
    material: LayerSetId,
    shadow: LayerSetId,
    draft: LayerSetId,
    channel: Channel,
}

fn three_sets(h: &mut Harness<'_, YoluApp>) -> Sets {
    let lang = h.lang();
    let material = h.doc().layer_sets()[0].id;
    apply(
        h,
        Action::M2(Edit::AddChannel(new_channel_info(
            lang.pick("影の強度", "Shadow Strength").into(),
            ChannelKind::Scalar,
        ))),
    );
    let channel = h.m2().paint_channel;
    let shadow = h.app().layer_set();
    apply(
        h,
        Action::M2(Edit::LayerSets(LayerSetEdit::Rename(
            shadow,
            lang.pick("影のマスク", "Shadow Mask").into(),
        ))),
    );
    apply(h, Action::M2(Edit::LayerSets(LayerSetEdit::New)));
    let draft = h.app().layer_set();
    apply(
        h,
        Action::M2(Edit::LayerSets(LayerSetEdit::Rename(
            draft,
            lang.pick("下書き", "Draft").into(),
        ))),
    );
    apply(h, Action::M2Ui(UiOp::SelectLayerSet(material)));
    Sets {
        material,
        shadow,
        draft,
        channel,
    }
}

fn set_row(h: &Harness<'_, YoluApp>, name: &str) -> egui::Rect {
    let panel = common::shot::group(h, Tab::TextureSets);
    rect_of(h, name, |r| panel.contains_rect(r) && r.width() > 60.0)
}

fn right_click(h: &mut Harness<'_, YoluApp>, at: egui::Pos2) {
    press(h, at, egui::PointerButton::Secondary);
    h.step();
    release(h, at, egui::PointerButton::Secondary);
    h.run();
}

/// 範囲 `area` の中の、値が `value` のドロップダウン（上から順）。
fn combos(h: &Harness<'_, YoluApp>, area: Rect, value: &str) -> Vec<Rect> {
    let mut all: Vec<Rect> = h
        .query_all_by_role_and_label(egui::accesskit::Role::ComboBox, value)
        .map(|n| n.rect())
        .filter(|r| area.contains_rect(*r))
        .collect();
    all.sort_by(|a, b| a.top().total_cmp(&b.top()));
    all
}

/// レイヤーのパネルの頭のレイヤーセットのドロップダウン（今のレイヤーセットの名前が値）。
fn layers_head(h: &Harness<'_, YoluApp>) -> Option<Rect> {
    let name = layersets::set_label(h.lang(), h.doc(), h.app().layer_set());
    combos(h, layers_body(h), &name).into_iter().next()
}

fn index_of(h: &Harness<'_, YoluApp>, set: LayerSetId) -> usize {
    h.doc()
        .layer_sets()
        .iter()
        .position(|s| s.id == set)
        .unwrap()
}

#[test]
fn a_document_with_one_layer_set_has_no_layer_set_rows_dropdown_or_channel_dropdowns() {
    let mut h = app(1280.0, 800.0, 128);
    h.run();
    assert!(
        h.query_by_label("レイヤーセット").is_none(),
        "レイヤーのパネルの頭"
    );
    assert!(
        h.query_by_label("出力元のレイヤーセット").is_none(),
        "チャンネルの行"
    );
    // テクスチャセットのパネルは今と同じ 1 行（レイヤーセットの名前の行は無い）
    assert!(h.query_by_label("マテリアル").is_none());
    let sets_before = h.sets().len();
    assert_eq!(sets_before, 1);
}

#[test]
fn pressing_a_layer_set_row_switches_the_set_the_list_and_the_paint_channel_at_once() {
    let mut h = app(1280.0, 800.0, 128);
    let s = three_sets(&mut h);
    // 行の並び: テクスチャセットの行の下に、レイヤーセットを字下げで
    let texture = set_row(&h, "テクスチャセット 1");
    let rows: Vec<Rect> = ["マテリアル", "影のマスク", "下書き"]
        .iter()
        .map(|n| set_row(&h, n))
        .collect();
    assert!(rows[0].top() > texture.bottom() - 1.0);
    assert!(rows[0].bottom() <= rows[1].top() + 1.0 && rows[1].bottom() <= rows[2].top() + 1.0);
    assert!(rows[0].left() > texture.left() || rows[0].width() < texture.width() + 1.0);
    // 影のマスクの行: 一覧・選ぶレイヤー・描くチャンネル（そのレイヤーセットが出力している最初のチャンネル = 影の強度）が一度に替わる
    click(&mut h, rows[1].center());
    assert_eq!(h.app().layer_set(), s.shadow);
    assert_eq!(h.m2().paint_channel, s.channel);
    let layer = h.selected_layer().unwrap();
    assert_eq!(h.doc().layer_set_of(layer).unwrap(), s.shadow);
    // 下書きの行: どのチャンネルも出力していないので、描くチャンネルは今のまま
    click(&mut h, rows[2].center());
    assert_eq!(h.app().layer_set(), s.draft);
    assert_eq!(h.m2().paint_channel, s.channel);
    // マテリアルの行: カラー
    click(&mut h, rows[0].center());
    assert_eq!(h.app().layer_set(), s.material);
    assert_eq!(h.m2().paint_channel, Channel::Color);
    assert!(!h.app().can_redo());
}

#[test]
fn another_texture_sets_layer_set_row_switches_the_texture_set_too() {
    let mut h = app(1280.0, 800.0, 128);
    let first_set_uid = h.sets().current().uid;
    let s = three_sets(&mut h);
    apply(&mut h, Action::M2Ui(UiOp::SelectLayerSet(s.shadow)));
    // 2 つ目のテクスチャセット（レイヤーセット 1 つ: 1 行のまま）に切り替えてから、最初のテクスチャセットの下書きの行を押す
    h.app_mut().add_texture_set().unwrap();
    h.run();
    assert_eq!(h.sets().current_index(), 1);
    assert_eq!(h.doc().layer_sets().len(), 1);
    let at = set_row(&h, "下書き").center();
    click(&mut h, at);
    assert_eq!(h.sets().current_index(), 0);
    assert_eq!(h.sets().current().uid, first_set_uid);
    assert_eq!(h.app().layer_set(), s.draft);
    assert_eq!(
        h.selected_layer().map(|l| h.doc().layer_set_of(l).unwrap()),
        Some(s.draft)
    );
}

#[test]
fn the_layer_set_menu_adds_duplicates_renames_and_removes_and_the_last_one_stays() {
    let mut h = app(1280.0, 800.0, 128);
    let s = three_sets(&mut h);
    let count = |h: &Harness<'_, YoluApp>| h.doc().layer_sets().len();
    // 右クリック: 押した行が今のレイヤーセットになってからメニュー
    let at = set_row(&h, "下書き").center();
    right_click(&mut h, at);
    assert_eq!(h.app().layer_set(), s.draft);
    assert_eq!(
        h.popup().as_ref().map(|p| p.kind),
        Some(PopupKind::M2(Popup::LayerSetContext(s.draft)))
    );
    for item in ["新しいレイヤーセット", "複製", "名前を変更", "削除"] {
        popup_item(&h, item);
    }
    // 複製
    let at = popup_item(&h, "複製").center();
    click(&mut h, at);
    assert_eq!(count(&h), 4);
    let copy = h.app().layer_set();
    assert_eq!(
        h.doc().layer_sets()[index_of(&h, s.draft) + 1].id,
        copy,
        "元のすぐ後"
    );
    assert_eq!(
        layersets::display_name(Lang::Ja, h.doc(), index_of(&h, copy)),
        "下書き 2"
    );
    undo(&mut h);
    assert_eq!(count(&h), 3);
    // 新しいレイヤーセット（空のレイヤー 1 枚つき）
    let at = set_row(&h, "下書き").center();
    right_click(&mut h, at);
    let at = popup_item(&h, "新しいレイヤーセット").center();
    click(&mut h, at);
    assert_eq!(count(&h), 4);
    let new = h.app().layer_set();
    assert_eq!(
        layersets::top_layer_of(h.doc(), new).map(|l| h.doc().layer_set_of(l).unwrap()),
        Some(new)
    );
    // 名前の変更（ダブルクリックと同じ入口）: 入力欄に打って確定
    apply(&mut h, Action::M2Ui(UiOp::RenameLayerSet(new)));
    assert_eq!(h.m2().renaming_layer_set, Some(new));
    key(&h, egui::Key::A, egui::Modifiers::COMMAND);
    h.event(egui::Event::Text("影 B".into()));
    key(&h, egui::Key::Enter, egui::Modifiers::NONE);
    h.run();
    assert_eq!(h.m2().renaming_layer_set, None);
    assert_eq!(
        layersets::display_name(Lang::Ja, h.doc(), index_of(&h, new)),
        "影 B"
    );
    // 削除（今のレイヤーセット。次のレイヤーセットがあれば次、無ければ前）
    let at = set_row(&h, "影 B").center();
    right_click(&mut h, at);
    let at = popup_item(&h, "削除").center();
    click(&mut h, at);
    assert_eq!(count(&h), 3);
    assert_eq!(h.app().layer_set(), s.draft, "一覧の最後だったので前");
    undo(&mut h);
    assert_eq!(count(&h), 4);
}

#[test]
fn the_remove_item_is_disabled_for_the_last_layer_set() {
    let mut h = app(1280.0, 800.0, 128);
    let s = three_sets(&mut h);
    for set in [s.draft, s.shadow] {
        apply(
            &mut h,
            Action::M2(Edit::LayerSets(LayerSetEdit::Remove(set))),
        );
    }
    assert_eq!(h.doc().layer_sets().len(), 1);
    // 1 つになるとレイヤーセットの行は出ない。メニューは項目の並びを直に確かめる
    let entries = yolu_app::m2_menu::entries(h.app(), Popup::LayerSetContext(s.material));
    let delete = entries
        .iter()
        .find_map(|e| match e {
            yolu_app::ui::menu::Entry::Item { label, enabled, .. } if label == "削除" => {
                Some(*enabled)
            }
            _ => None,
        })
        .expect("削除の項目");
    assert!(!delete, "最後の 1 つは消せない");
    apply(
        &mut h,
        Action::M2(Edit::LayerSets(LayerSetEdit::Remove(s.material))),
    );
    assert_eq!(h.doc().layer_sets().len(), 1);
}

#[test]
fn dragging_a_layer_set_row_reorders_the_sets_in_one_undo() {
    let mut h = app(1280.0, 800.0, 128);
    let s = three_sets(&mut h);
    apply(&mut h, Action::M2Ui(UiOp::SelectLayerSet(s.draft)));
    let draft = set_row(&h, "下書き");
    let material = set_row(&h, "マテリアル");
    // 下書きを一番上の行へ
    drag(
        &mut h,
        &[
            draft.center(),
            pos2(draft.center().x, draft.center().y - 20.0),
            pos2(material.center().x, material.top() + 4.0),
        ],
    );
    let order: Vec<LayerSetId> = h.doc().layer_sets().iter().map(|x| x.id).collect();
    assert_eq!(order, [s.draft, s.material, s.shadow]);
    assert_eq!(h.app().layer_set(), s.draft, "ドラッグでは選び直さない");
    undo(&mut h);
    let order: Vec<LayerSetId> = h.doc().layer_sets().iter().map(|x| x.id).collect();
    assert_eq!(order, [s.material, s.shadow, s.draft]);
}

#[test]
fn the_layers_panel_has_a_layer_set_dropdown_and_a_mark_when_the_paint_channel_is_not_in_the_output(
) {
    let mut h = app(1280.0, 800.0, 128);
    let s = three_sets(&mut h);
    let body = layers_body(&h);
    let in_layers = |r: Rect| body.contains_rect(r);
    // マテリアル × カラー: 出力に出ている。印は無い
    assert!(h.get_all_by_label("出力なし").all(|n| !in_layers(n.rect())));
    assert!(layers_head(&h).is_some(), "レイヤーのパネルの頭");
    // 下書き × カラー: 出力に出ていないので、ドロップダウンの右に印
    apply(&mut h, Action::M2Ui(UiOp::SelectLayerSet(s.draft)));
    let dropdown = layers_head(&h).expect("レイヤーのパネルの頭");
    let mark = rect_of(&h, "出力なし", in_layers);
    assert!(mark.left() >= dropdown.right() - 1.0, "ドロップダウンの右");
    // ドロップダウンを押すと、レイヤーセットを選べる（今のレイヤーセットに印）
    let at = layers_head(&h).unwrap().center();
    click(&mut h, at);
    assert_eq!(
        h.popup().as_ref().map(|p| p.kind),
        Some(PopupKind::M2(Popup::LayerSet))
    );
    for name in ["マテリアル", "影のマスク", "下書き"] {
        popup_item(&h, name);
    }
    let at = popup_item(&h, "影のマスク").center();
    click(&mut h, at);
    assert_eq!(h.app().layer_set(), s.shadow);
    assert_eq!(h.m2().paint_channel, s.channel);
    // 一覧は今のレイヤーセットのレイヤーだけ
    let layer = h.selected_layer().unwrap();
    assert_eq!(h.doc().layer_set_of(layer).unwrap(), s.shadow);
    // 新しいレイヤーは今のレイヤーセットに入る
    apply(&mut h, Action::NewLayer);
    let added = h.selected_layer().unwrap();
    assert_eq!(h.doc().layer_set_of(added).unwrap(), s.shadow);
}

#[test]
fn the_layer_menu_moves_layers_to_another_set_and_the_view_stays() {
    let mut h = app(1280.0, 800.0, 128);
    let s = three_sets(&mut h);
    let layer = h.selected_layer().unwrap();
    let name = h.doc().layer(layer).unwrap().name().to_owned();
    open_layer_menu(&mut h, &name);
    // 「レイヤーセットへ移す ▸」は、今のレイヤーセット以外を出す
    let at = popup_item(&h, "レイヤーセットへ移す").center();
    click(&mut h, at);
    popup_item(&h, "影のマスク");
    popup_item(&h, "下書き");
    // 移し先の一覧に、今のレイヤーセットは出ない
    let yolu_app::ui::menu::Entry::Submenu { entries, .. } =
        yolu_app::m2_menu::layer_set_move_submenu(h.app(), vec![layer])
            .expect("レイヤーセットが 2 つ以上")
    else {
        panic!("入れ子のメニュー");
    };
    let labels: Vec<String> = entries
        .iter()
        .filter_map(|e| match e {
            yolu_app::ui::menu::Entry::Item { label, .. } => Some(label.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(labels, ["影のマスク", "下書き"]);
    let at = popup_item(&h, "下書き").center();
    click(&mut h, at);
    assert_eq!(h.doc().layer_set_of(layer).unwrap(), s.draft);
    assert_eq!(h.app().layer_set(), s.material, "画面は移し先へ飛ばない");
    undo(&mut h);
    assert_eq!(h.doc().layer_set_of(layer).unwrap(), s.material);
}

/// チャンネルの行の、出力元のレイヤーセットのドロップダウン（値はレイヤーセットの名前か「なし」。行の並びの順）。
fn channel_dropdown(h: &Harness<'_, YoluApp>, channel: Channel) -> Rect {
    let panel = common::shot::group(h, Tab::Channels);
    let lang = h.lang();
    let mut names: Vec<String> = (0..h.doc().layer_sets().len())
        .map(|i| layersets::display_name(lang, h.doc(), i))
        .collect();
    names.push(lang.pick("なし", "None").to_owned());
    let mut all: Vec<Rect> = names.iter().flat_map(|n| combos(h, panel, n)).collect();
    all.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let index = h
        .doc()
        .channels()
        .iter()
        .position(|c| *c == channel)
        .unwrap();
    all[index]
}

#[test]
fn the_channel_row_dropdown_changes_the_final_result_in_one_undo() {
    let mut h = app(1280.0, 800.0, 128);
    let s = three_sets(&mut h);
    click_tab(&mut h, Tab::Channels);
    h.run();
    let material_layer = layersets::top_layer_of(h.doc(), s.material).unwrap();
    h.doc_mut().set_pixel(material_layer, 10, 10, RED).unwrap();
    h.run();
    let px = |h: &Harness<'_, YoluApp>| h.doc().composite_pixel(Channel::Color, 10, 10).unwrap();
    assert_eq!(px(&h), RED);
    // カラーの行: 出力元をなしにする（ドロップダウンの項目は、レイヤーセットの名前と、区切りの後の「なし」）
    let at = channel_dropdown(&h, Channel::Color).center();
    click(&mut h, at);
    assert_eq!(
        h.popup().as_ref().map(|p| p.kind),
        Some(PopupKind::M2(Popup::LayerSetOutput(Channel::Color)))
    );
    for name in ["マテリアル", "影のマスク", "下書き", "なし"] {
        popup_item(&h, name);
    }
    let steps = h.doc().undo_count();
    let at = popup_item(&h, "なし").center();
    click(&mut h, at);
    assert_eq!(h.doc().channel_output(Channel::Color).unwrap(), None);
    let default = h.doc().channel_info(Channel::Color).unwrap().default;
    assert_eq!(px(&h), default, "最終のカラーは既定値");
    assert_eq!(h.doc().undo_count(), steps + 1, "1 回の取り消し");
    undo(&mut h);
    assert_eq!(
        h.doc().channel_output(Channel::Color).unwrap(),
        Some(s.material)
    );
    assert_eq!(px(&h), RED);
    // 影の強度の行: 影のマスクが選ばれている
    let at = channel_dropdown(&h, s.channel).center();
    click(&mut h, at);
    let at = popup_item(&h, "下書き").center();
    click(&mut h, at);
    assert_eq!(h.doc().channel_output(s.channel).unwrap(), Some(s.draft));
}

/// 2D のキャンバスが描いた表示の画素（画面の点の下）。`canvas_pixel` は最終の結果を読むので、ここでは描いた表示を読む。
fn shown_pixel(h: &mut Harness<'_, YoluApp>, doc_x: f64, doc_y: f64) -> [u8; 4] {
    let r = canvas_rect(h);
    let (w, hh) = (h.doc().width(), h.doc().height());
    let at = h.view().view(r, w, hh).to_screen(doc_x, doc_y);
    let image = h.render().expect("画面を描く");
    image.get_pixel(at.x as u32, at.y as u32).0
}

#[test]
fn the_canvas_shows_the_current_layer_set_while_the_final_result_stays_the_output() {
    let mut h = app(1280.0, 800.0, 128);
    let s = three_sets(&mut h);
    let material_layer = layersets::top_layer_of(h.doc(), s.material).unwrap();
    let draft_layer = layersets::top_layer_of(h.doc(), s.draft).unwrap();
    for y in 50..70 {
        for x in 50..70 {
            h.doc_mut().set_pixel(material_layer, x, y, RED).unwrap();
            h.doc_mut().set_pixel(draft_layer, x, y, BLUE).unwrap();
        }
    }
    h.run();
    let red = [RED.r, RED.g, RED.b, 255];
    let blue = [BLUE.r, BLUE.g, BLUE.b, 255];
    let near = |a: [u8; 4], b: [u8; 4]| (0..3).all(|i| (a[i] as i32 - b[i] as i32).abs() <= 2);
    assert!(
        near(shown_pixel(&mut h, 60.0, 60.0), red),
        "マテリアルの合成"
    );
    // 下書き（どのチャンネルにも出ていない）を見せる: 2D は下書きの合成。3D・書き出しが読む最終のカラーはマテリアルのまま
    apply(&mut h, Action::M2Ui(UiOp::SelectLayerSet(s.draft)));
    h.run();
    assert!(near(shown_pixel(&mut h, 60.0, 60.0), blue), "下書きの合成");
    assert_eq!(
        h.doc().composite_pixel(Channel::Color, 60, 60).unwrap(),
        RED
    );
    // 描くと、描いた所が見えるレイヤーセットは今のレイヤーセット
    apply(&mut h, Action::M2Ui(UiOp::SelectLayerSet(s.material)));
    h.run();
    assert!(near(shown_pixel(&mut h, 60.0, 60.0), red));
    // 見せるレイヤーセットを出力から外す・戻すを繰り返しても、表示は今のレイヤーセットのまま
    apply(
        &mut h,
        Action::M2(Edit::LayerSets(LayerSetEdit::Output {
            channel: Channel::Color,
            set: Some(s.draft),
        })),
    );
    h.run();
    assert!(
        near(shown_pixel(&mut h, 60.0, 60.0), red),
        "出力を替えても、見せている合成は今のレイヤーセット"
    );
    assert_eq!(
        h.doc().composite_pixel(Channel::Color, 60, 60).unwrap(),
        BLUE
    );
}

fn bar_chip(h: &Harness<'_, YoluApp>) -> Rect {
    let canvas = canvas_rect(h);
    let name = h.lang().pick("描くチャンネル", "Paint Channel");
    rect_of(h, name, |r| canvas.contains_rect(r))
}

#[test]
fn the_canvas_bar_opens_three_columns_and_picks_set_layer_set_channel_and_displayed_channel() {
    let mut h = app(1280.0, 800.0, 128);
    let s = three_sets(&mut h);
    let canvas = canvas_rect(&h);
    // 右上に 2 つのボタン: 「目・UV・▽」と、描くチャンネル
    assert!(bar_chip(&h).right() <= canvas.right() && bar_chip(&h).top() >= canvas.top());
    let uv = rect_of(
        &h,
        "UV ワイヤーフレームの色と不透明度",
        |r| canvas.contains_rect(r),
    );
    assert!(uv.right() <= bar_chip(&h).left(), "チャンネルのボタンの左");
    let at = bar_chip(&h).center();
    click(&mut h, at);
    assert!(h.ui().canvas_bar.open);
    // 3 つの列（テクスチャセット・レイヤーセット・チャンネル）の中身
    let pop_rows = |h: &Harness<'_, YoluApp>, name: &str| -> Vec<Rect> {
        h.query_all_by_label(name)
            .map(|n| n.rect())
            .filter(|r| canvas.contains_rect(*r) && r.top() > bar_chip(h).bottom())
            .collect()
    };
    assert_eq!(pop_rows(&h, "影のマスク").len(), 1, "レイヤーセットの列");
    assert_eq!(pop_rows(&h, "ラフネス").len(), 1, "チャンネルの列");
    assert_eq!(
        pop_rows(&h, "テクスチャセット 1").len(),
        1,
        "テクスチャセットの列"
    );
    // レイヤーセットの列: 押すと今のレイヤーセット（描くチャンネルも替わる）。欄は開いたまま
    let at = pop_rows(&h, "影のマスク")[0].center();
    click(&mut h, at);
    assert_eq!(h.app().layer_set(), s.shadow);
    assert_eq!(h.m2().paint_channel, s.channel);
    assert!(h.ui().canvas_bar.open);
    // チャンネルの列: 押すと描くチャンネル。右の目で、見せるチャンネル
    let at = pop_rows(&h, "ラフネス")[0].center();
    click(&mut h, at);
    assert_eq!(h.m2().paint_channel, Channel::Roughness);
    let eye_tip = h
        .lang()
        .f("panels-show-in-the-canvas-tip", &[("name", &"ラフネス")]);
    let eye = h
        .get_all_by_label(&eye_tip)
        .map(|n| n.rect())
        .find(|r| canvas.contains_rect(*r))
        .expect("見せる目");
    click(&mut h, eye.center());
    assert_eq!(h.m2().display_channel, Channel::Roughness);
    // 「＋」: レイヤーセットを追加（今のテクスチャセットに。今のレイヤーセットになる）
    let before = h.doc().layer_sets().len();
    let add = rect_of(&h, "レイヤーセットを追加", |r| {
        canvas.contains_rect(r)
    });
    click(&mut h, add.center());
    assert_eq!(h.doc().layer_sets().len(), before + 1);
    // テクスチャセットの「＋」
    let add = rect_of(&h, "テクスチャセットを追加", |r| {
        canvas.contains_rect(r)
    });
    click(&mut h, add.center());
    assert_eq!(h.sets().len(), 2);
    assert_eq!(h.sets().current_index(), 1);
    // 欄の外を押すと閉じる。その押しでは描き始めず、文書も変えない
    let steps = h.doc().undo_count();
    let outside = pos2(canvas.center().x - 100.0, canvas.bottom() - 60.0);
    click(&mut h, outside);
    assert!(!h.ui().canvas_bar.open);
    assert!(!h.app().is_stroking());
    assert_eq!(h.doc().undo_count(), steps);
    // もう一度押すと、閉じたボタンを押して開き、チャンネルのボタンで閉じる
    let at = bar_chip(&h).center();
    click(&mut h, at);
    assert!(h.ui().canvas_bar.open);
    let at = bar_chip(&h).center();
    click(&mut h, at);
    assert!(!h.ui().canvas_bar.open);
}

#[test]
fn the_canvas_bar_uv_button_toggles_the_wireframe_and_opens_the_color_menu_without_painting() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 800.0, 64);
        h.set_lang(lang);
        h.act(Action::LoadDemoModel);
        h.app_mut().sync_view3d();
        h.run();
        let canvas = canvas_rect(&h);
        let toggle = lang.pick("UV ワイヤーフレーム", "UV Wireframe");
        let arrow = lang.pick(
            "UV ワイヤーフレームの色と不透明度",
            "UV wireframe color and opacity",
        );
        assert!(h.prefs().settings.uv_wireframe);
        let at = rect_of(&h, toggle, |r| canvas.contains_rect(r)).center();
        click(&mut h, at);
        assert!(!h.prefs().settings.uv_wireframe);
        assert!(!h.app().is_stroking());
        assert!(!h.app().can_undo());
        let at = rect_of(&h, toggle, |r| canvas.contains_rect(r)).center();
        click(&mut h, at);
        assert!(h.prefs().settings.uv_wireframe);
        // ▽: 色と不透明度の欄
        let at = rect_of(&h, arrow, |r| canvas.contains_rect(r)).center();
        click(&mut h, at);
        assert!(h.ui().canvas_bar.uv_menu);
        assert!(!h.ui().canvas_bar.open);
        let swatch = rect_of(&h, arrow, |r| {
            r.top() > rect_of(&h, toggle, |r| canvas.contains_rect(r)).bottom()
                && canvas.contains_rect(r)
        });
        assert!(swatch.width() > 20.0, "色の見本");
        // 欄の外を押すと閉じる
        click(
            &mut h,
            pos2(canvas.center().x - 100.0, canvas.bottom() - 60.0),
        );
        assert!(!h.ui().canvas_bar.uv_menu);
        assert!(!h.app().can_undo());
    }
}

#[test]
fn through_check_the_shadow_mask_set_is_drawn_from_the_canvas_bar_and_the_final_result_follows() {
    let mut h = app(1280.0, 800.0, 128);
    let s = three_sets(&mut h);
    // 影の強度の出力を影のマスクへ（チャンネルを足したときに作ったレイヤーセットは、三つのレイヤーセットを作ったとき影のマスクにした）
    assert_eq!(h.doc().channel_output(s.channel).unwrap(), Some(s.shadow));
    let shadow_layer = layersets::top_layer_of(h.doc(), s.shadow).unwrap();
    let final_shadow = |h: &Harness<'_, YoluApp>| {
        h.doc()
            .composite_channel(s.channel, h.doc().bounds())
            .unwrap()
    };
    let before = final_shadow(&h);
    let color_before = h
        .doc()
        .composite_channel(Channel::Color, h.doc().bounds())
        .unwrap();
    // 右上のチャンネルのボタンから、影のマスク → 影の強度を選ぶ
    let at = bar_chip(&h).center();
    click(&mut h, at);
    let canvas = canvas_rect(&h);
    let pick = |h: &mut Harness<'_, YoluApp>, name: &str| {
        let r = h
            .query_all_by_label(name)
            .map(|n| n.rect())
            .find(|r| canvas.contains_rect(*r) && r.top() > bar_chip(h).bottom())
            .unwrap_or_else(|| panic!("{name}"));
        click(h, r.center());
    };
    pick(&mut h, "影のマスク");
    pick(&mut h, "影の強度");
    let at = bar_chip(&h).center();
    click(&mut h, at);
    assert_eq!(h.app().layer_set(), s.shadow);
    assert_eq!(h.m2().paint_channel, s.channel);
    assert_eq!(h.selected_layer(), Some(shadow_layer));
    // 描く: 影のマスクのレイヤーに、影の強度の値が入る
    h.color_mut().set_main([1.0, 1.0, 1.0, 1.0]);
    let c = canvas.center();
    drag(
        &mut h,
        &[
            offset(c, -30.0, 0.0),
            offset(c, 0.0, 0.0),
            offset(c, 30.0, 0.0),
        ],
    );
    let layer = h.doc().layer(shadow_layer).unwrap();
    assert!(
        layer.surface(s.channel).is_some_and(|f| f.tile_count() > 0),
        "描いた"
    );
    // 3D・書き出しが読む影の強度の最終の結果が替わる。カラーは変わらない
    assert_ne!(final_shadow(&h), before);
    assert_eq!(
        h.doc()
            .composite_channel(Channel::Color, h.doc().bounds())
            .unwrap(),
        color_before
    );
    // カラーの出力をなしにすると最終のカラーは既定値。Ctrl+Z 1 回で戻る
    click_tab(&mut h, Tab::Channels);
    h.run();
    let at = channel_dropdown(&h, Channel::Color).center();
    click(&mut h, at);
    let at = popup_item(&h, "なし").center();
    click(&mut h, at);
    let default = h.doc().channel_info(Channel::Color).unwrap().default;
    assert_eq!(
        h.doc().composite_pixel(Channel::Color, 3, 3).unwrap(),
        default
    );
    undo(&mut h);
    assert_eq!(
        h.doc().channel_output(Channel::Color).unwrap(),
        Some(s.material)
    );
    assert_eq!(
        h.doc()
            .composite_channel(Channel::Color, h.doc().bounds())
            .unwrap(),
        color_before
    );
}

// ───────── スクリーンショット ─────────

fn shot_state(lang: Lang) -> (Harness<'static, YoluApp>, Sets) {
    let mut h = app(1280.0, 800.0, 128);
    h.set_lang(lang);
    h.run();
    let s = three_sets(&mut h);
    apply(&mut h, Action::M2Ui(UiOp::SelectLayerSet(s.draft)));
    (h, s)
}

#[test]
fn the_layer_set_panels_look_the_same() {
    for lang in Lang::ALL {
        let (mut h, _) = shot_state(lang);
        let n = lang_name(lang);
        let group = common::shot::group(&h, Tab::TextureSets);
        common::shot::clean_region(&mut h, &format!("layer_sets_texture_panel_{n}"), group);
        let layers = common::shot::group(&h, Tab::Layers);
        common::shot::clean_region(&mut h, &format!("layer_sets_layers_head_{n}"), layers);
        click_tab(&mut h, Tab::Channels);
        h.run();
        let channels = common::shot::group(&h, Tab::Channels);
        common::shot::clean_region(&mut h, &format!("layer_sets_channels_{n}"), channels);
        // 出力元のドロップダウンを開いた形
        let at = channel_dropdown(&h, h.m2().paint_channel).center();
        click(&mut h, at);
        let rect = common::shot::with_popup(&h, channels);
        common::shot::region(&mut h, &format!("layer_sets_channels_pick_{n}"), rect);
    }
}

#[test]
fn the_canvas_bar_looks_the_same() {
    for lang in Lang::ALL {
        let (mut h, _) = shot_state(lang);
        let n = lang_name(lang);
        let canvas = canvas_rect(&h);
        let top_right = Rect::from_min_max(
            pos2(canvas.right() - 520.0, canvas.top()),
            pos2(canvas.right(), canvas.top() + 60.0),
        );
        common::shot::clean_region(&mut h, &format!("canvas_bar_closed_{n}"), top_right);
        let at = bar_chip(&h).center();
        click(&mut h, at);
        let open = Rect::from_min_max(
            pos2(canvas.right() - 540.0, canvas.top()),
            pos2(canvas.right(), canvas.top() + 350.0),
        );
        common::shot::clean_region(&mut h, &format!("canvas_bar_open_{n}"), open);
    }
}
