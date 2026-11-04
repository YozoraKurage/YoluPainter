//! レイヤーセットの組（扱うチャンネル）の画面の操作（画面は描かない）: テクスチャセットのパネルの「＋」のメニュー・行の右クリックの「チャンネル ▸」での
//! 入切・描くチャンネルを組の外へ替えたときにレイヤーセットが移る・画像のドロップの置き先・lilToon から足すチャンネルはチャンネルごとのレイヤーセット。
use crate::common::drive::Drive;

use yolu_app::engine::{Channel, ChannelKind, HistoryKind, LayerSetId, Rgba8};
use yolu_app::look::LookOp;
use yolu_app::m2::layersets::{self, LayerSetEdit};
use yolu_app::m2::{new_channel_info, Edit, UiOp};
use yolu_app::m2_menu::Popup;
use yolu_app::state::{Action, AppState};
use yolu_app::ui::menu::{Check, Entry};
use yolu_core::look::{PlaneSource, TextureSource};

/// 項目の（名前・印・押せるか）。入れ子のメニューは（名前・印なし・押せるか）。
fn rows(entries: &[Entry<Action>]) -> Vec<(String, bool, bool)> {
    entries
        .iter()
        .filter_map(|e| match e {
            Entry::Item {
                label,
                check,
                enabled,
                ..
            } => Some((label.clone(), *check == Check::Checked, *enabled)),
            Entry::Submenu { label, enabled, .. } => Some((label.clone(), false, *enabled)),
            _ => None,
        })
        .collect()
}

/// 名前の項目の操作。
fn action_of(entries: &[Entry<Action>], name: &str) -> Action {
    entries
        .iter()
        .find_map(|e| match e {
            Entry::Item { label, action, .. } if label == name => Some(action.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("項目 {name}"))
}

/// レイヤーセットの行の右クリックの「チャンネル ▸」の中身。
fn channel_submenu(s: &AppState, set: LayerSetId) -> Vec<Entry<Action>> {
    yolu_app::m2_menu::entries(s, Popup::LayerSetContext(set))
        .into_iter()
        .find_map(|e| match e {
            Entry::Submenu { label, entries, .. } if label == "チャンネル" => Some(entries),
            _ => None,
        })
        .expect("チャンネル ▸")
}

fn first_set(s: &AppState) -> LayerSetId {
    s.doc().layer_sets()[0].id
}

/// ユーザーチャンネル「影の強度」を足す（そのチャンネルの名前のレイヤーセットができる）。
fn add_shadow(s: &mut AppState) -> (Channel, LayerSetId) {
    s.act(Action::M2(Edit::AddChannel(new_channel_info(
        "影の強度".into(),
        ChannelKind::Scalar,
    ))));
    (s.m2().paint_channel, s.app().layer_set())
}

fn toggle(s: &mut AppState, set: LayerSetId, channel: Channel) {
    s.act(Action::M2(Edit::LayerSets(LayerSetEdit::ToggleChannel {
        set,
        channel,
    })));
}

#[test]
fn the_add_menu_offers_a_texture_set_and_a_layer_set_and_disables_each_at_its_limit() {
    let mut s = AppState::new(64, 64);
    let entries = yolu_app::m2_menu::entries(&s, Popup::AddSet);
    assert_eq!(
        rows(&entries),
        [
            ("テクスチャセットを追加".to_owned(), false, true),
            ("レイヤーセットを追加".to_owned(), false, true),
        ]
    );
    // レイヤーセットを追加: 今のテクスチャセットに、標準のチャンネルの組で（空のレイヤー 1 枚つき。1 回の取り消し）
    let steps = s.doc().undo_count();
    s.act(action_of(&entries, "レイヤーセットを追加"));
    assert_eq!(s.doc().layer_sets().len(), 2);
    assert_eq!(s.doc().undo_count(), steps + 1);
    let added = s.app().layer_set();
    assert_eq!(
        s.doc().layer_set_channels(added).unwrap(),
        s.doc().standard_channel_bits()
    );
    // テクスチャセットを追加
    s.act(action_of(&entries, "テクスチャセットを追加"));
    assert_eq!(s.sets().len(), 2);
    // 上限: 項目は押せない形
    while s.sets().len() < yolu_app::newproject::MAX_SETS {
        s.app_mut().add_texture_set().unwrap();
    }
    while s.doc().layer_sets().len() < yolu_core::MAX_LAYER_SETS {
        s.doc_mut().add_layer_set("x").unwrap();
    }
    let entries = yolu_app::m2_menu::entries(&s, Popup::AddSet);
    assert_eq!(
        rows(&entries)
            .into_iter()
            .map(|(_, _, on)| on)
            .collect::<Vec<_>>(),
        [false, false]
    );
}

#[test]
fn the_channels_submenu_toggles_one_channel_of_the_set_in_one_undo_and_says_when_the_output_goes() {
    let mut s = AppState::new(64, 64);
    let base = first_set(&s);
    let (shadow_channel, shadow) = add_shadow(&mut s);
    s.act(Action::M2Ui(UiOp::SelectLayerSet(base)));
    // 文書のチャンネルごとの項目。組にあれば印
    let menu = channel_submenu(&s, base);
    let names: Vec<(String, bool)> = rows(&menu).into_iter().map(|(n, c, _)| (n, c)).collect();
    assert_eq!(names.len(), s.doc().channels().len());
    assert!(names.contains(&("カラー".to_owned(), true)));
    assert!(names.contains(&("影の強度".to_owned(), false)));
    // メタリックを外す: 出力の割り当てがこのレイヤーセットだったので外れ（最終の結果は既定値）、知らせる
    assert_eq!(
        s.doc().channel_output(Channel::Metallic).unwrap(),
        Some(base)
    );
    let bits = s.doc().layer_set_channels(base).unwrap();
    let steps = s.doc().undo_count();
    s.message_mut().clear();
    s.act(action_of(&menu, "メタリック"));
    assert_eq!(
        s.doc().layer_set_channels(base).unwrap(),
        bits & !Channel::Metallic.bit()
    );
    assert_eq!(s.doc().undo_count(), steps + 1, "1 回の取り消し");
    assert_eq!(s.doc().history().last(), Some(HistoryKind::LayerSets));
    assert_eq!(s.doc().channel_output(Channel::Metallic).unwrap(), None);
    assert_eq!(
        s.message(),
        "「マテリアル」が「メタリック」を扱わなくなったので、「メタリック」の出力元の割り当てを外しました。"
    );
    s.act(Action::Undo);
    assert_eq!(s.doc().layer_set_channels(base).unwrap(), bits);
    assert_eq!(
        s.doc().channel_output(Channel::Metallic).unwrap(),
        Some(base)
    );
    // 影の強度を入れる: 出力は影の強度のレイヤーセットのまま（知らせない）
    s.message_mut().clear();
    toggle(&mut s, base, shadow_channel);
    assert_eq!(
        s.doc().layer_set_channels(base).unwrap(),
        bits | shadow_channel.bit()
    );
    assert_eq!(
        s.doc().channel_output(shadow_channel).unwrap(),
        Some(shadow)
    );
    assert!(s.message().is_empty(), "{}", s.message());
    assert!(
        rows(&channel_submenu(&s, base)).contains(&("影の強度".to_owned(), true, true)),
        "印が付く"
    );
    // 今の描くチャンネル（カラー）を今のレイヤーセットから外すと、描く・見せるチャンネルは組に残ったチャンネルへ
    assert_eq!(s.m2().paint_channel, Channel::Color);
    toggle(&mut s, base, Channel::Color);
    let left = s.doc().layer_set_channels(base).unwrap();
    assert_ne!(s.m2().paint_channel, Channel::Color);
    assert_ne!(left & s.m2().paint_channel.bit(), 0);
    assert_ne!(left & s.m2().display_channel.bit(), 0);
}

#[test]
fn painting_a_channel_outside_the_set_moves_to_the_set_that_outputs_it_then_to_the_first_that_has_it(
) {
    let mut s = AppState::new(64, 64);
    let base = first_set(&s);
    let (shadow_channel, shadow) = add_shadow(&mut s);
    s.act(Action::M2(Edit::LayerSets(LayerSetEdit::New)));
    let draft = s.app().layer_set();
    // 出力の割り当て先へ（描く先のレイヤーもそのレイヤーセットのレイヤー）
    s.act(Action::M2Ui(UiOp::SelectLayerSet(base)));
    s.act(Action::M2Ui(UiOp::PaintChannel(shadow_channel)));
    assert_eq!(s.app().layer_set(), shadow);
    assert_eq!(s.m2().paint_channel, shadow_channel);
    let layer = s.selected_layer().expect("描く先のレイヤー");
    assert_eq!(s.doc().layer_set_of(layer).unwrap(), shadow);
    // 組の中のチャンネルへ替えるだけなら移らない
    s.act(Action::M2Ui(UiOp::SelectLayerSet(base)));
    s.act(Action::M2Ui(UiOp::PaintChannel(Channel::Roughness)));
    assert_eq!(s.app().layer_set(), base);
    assert_eq!(s.m2().paint_channel, Channel::Roughness);
    // 割り当てが無ければ、組に持つ最初のレイヤーセットへ（影の強度のレイヤーセットから外し、下書きに入れる）
    toggle(&mut s, shadow, shadow_channel);
    toggle(&mut s, draft, shadow_channel);
    assert_eq!(s.doc().channel_output(shadow_channel).unwrap(), None);
    s.act(Action::M2Ui(UiOp::SelectLayerSet(base)));
    s.act(Action::M2Ui(UiOp::PaintChannel(shadow_channel)));
    assert_eq!(s.app().layer_set(), draft);
    assert_eq!(s.m2().paint_channel, shadow_channel);
    // どのレイヤーセットも持たなければ替えずに知らせる
    toggle(&mut s, draft, shadow_channel);
    s.act(Action::M2Ui(UiOp::SelectLayerSet(base)));
    let paint = s.m2().paint_channel;
    s.message_mut().clear();
    s.act(Action::M2Ui(UiOp::PaintChannel(shadow_channel)));
    assert_eq!(s.app().layer_set(), base);
    assert_eq!(s.m2().paint_channel, paint);
    assert_eq!(
        s.message(),
        "「影の強度」を描くチャンネルにできません（どのレイヤーセットもそのチャンネルを扱わない）。"
    );
    // 見るチャンネルは組の外でもよい（移らない）
    s.act(Action::M2Ui(UiOp::DisplayChannel(shadow_channel)));
    assert_eq!(s.m2().display_channel, shadow_channel);
    assert_eq!(s.app().layer_set(), base);
}

/// lilToon の口から足したチャンネル: 同じ名前のレイヤーセット（組はそのチャンネルだけ）から出力し、最初のレイヤーはそのチャンネルを有効にする。
fn assert_own_set(s: &AppState, channel: Channel) -> LayerSetId {
    let doc = s.doc();
    let name = doc.channel_info(channel).unwrap().name.clone();
    let set = doc
        .channel_output(channel)
        .unwrap()
        .expect("出力の割り当て");
    assert_eq!(layersets::set_label(s.lang(), doc, set), name);
    assert_eq!(
        doc.layer_set_channels(set).unwrap(),
        channel.bit(),
        "{name}"
    );
    let layer = layersets::top_layer_of(doc, set).expect("最初のレイヤー");
    assert!(doc.layer(layer).unwrap().is_channel_enabled(channel));
    assert_ne!(set, doc.layer_sets()[0].id);
    set
}

#[test]
fn a_liltoon_slot_painted_first_gets_its_own_layer_set_and_one_undo_removes_both() {
    let mut s = AppState::new(64, 64);
    let base = first_set(&s);
    let sets = s.doc().layer_sets().len();
    let steps = s.doc().undo_count();
    s.act(Action::Look(LookOp::PaintSlot("_RimShadeMask")));
    let channel = s.m2().paint_channel;
    assert!(!channel.is_standard());
    let set = assert_own_set(&s, channel);
    assert_eq!(s.doc().layer_sets().len(), sets + 1);
    assert_eq!(s.doc().undo_count(), steps + 1, "1 回の取り消し");
    // 描く先はそのレイヤーセット
    assert_eq!(s.app().layer_set(), set);
    // メインの組には入らない
    assert_eq!(s.doc().layer_set_channels(base).unwrap() & channel.bit(), 0);
    // もう割り当てのあるスロット: 何も作らない
    s.act(Action::Look(LookOp::PaintSlot("_RimShadeMask")));
    assert_eq!(s.doc().layer_sets().len(), sets + 1);
    s.act(Action::Undo);
    assert_eq!(s.doc().layer_sets().len(), sets);
    assert!(s.doc().channel_info(channel).is_none());
    assert_eq!(s.app().layer_set(), base);
}

#[test]
fn a_new_channel_for_a_slot_and_for_a_packed_component_each_get_their_own_layer_set() {
    let mut s = AppState::new(64, 64);
    s.act(Action::Look(LookOp::NewChannel {
        slot: "_ShadowStrengthMask",
        plane: None,
    }));
    let whole = s.m2().paint_channel;
    assert_own_set(&s, whole);
    // 成分ごとの詰め合わせの G を新しいチャンネルに
    let planes = [
        PlaneSource::Channel {
            channel: whole,
            component: 0,
        },
        PlaneSource::Zero,
        PlaneSource::One,
        PlaneSource::Zero,
    ];
    s.act(Action::Look(LookOp::Texture {
        slot: "_ShadowStrengthMask",
        source: Some(TextureSource::Packed(planes)),
    }));
    let sets = s.doc().layer_sets().len();
    let steps = s.doc().undo_count();
    s.act(Action::Look(LookOp::NewChannel {
        slot: "_ShadowStrengthMask",
        plane: Some(1),
    }));
    let TextureSource::Packed(now) = s.doc().look().textures["_ShadowStrengthMask"] else {
        panic!("詰め合わせのまま");
    };
    let PlaneSource::Channel { channel: g, .. } = now[1] else {
        panic!("{now:?}");
    };
    let set = assert_own_set(&s, g);
    assert_eq!(s.app().layer_set(), set);
    assert_eq!(s.doc().layer_sets().len(), sets + 1);
    assert_eq!(s.doc().undo_count(), steps + 1);
    // 前からあるチャンネル（R）は動かさない
    assert_eq!(
        s.doc()
            .layer_set_channels(s.doc().channel_output(whole).unwrap().unwrap())
            .unwrap(),
        whole.bit()
    );
    s.act(Action::Undo);
    assert_eq!(s.doc().layer_sets().len(), sets);
    assert!(s.doc().channel_info(g).is_none());
}

#[test]
fn the_liltoon_template_makes_one_layer_set_per_channel_and_one_undo_removes_them_all() {
    let mut s = AppState::new(64, 64);
    let sets = s.doc().layer_sets().len();
    let channels = s.doc().channels().len();
    let steps = s.doc().undo_count();
    s.act(Action::Look(LookOp::Template));
    let made: Vec<Channel> = s
        .doc()
        .channels()
        .into_iter()
        .filter(|c| !c.is_standard())
        .collect();
    assert!(made.len() >= 2, "{made:?}");
    assert_eq!(s.doc().channels().len(), channels + made.len());
    assert_eq!(s.doc().layer_sets().len(), sets + made.len());
    for c in &made {
        assert_own_set(&s, *c);
    }
    assert_eq!(s.doc().undo_count(), steps + 1);
    s.act(Action::Undo);
    assert_eq!(s.doc().layer_sets().len(), sets);
    assert_eq!(s.doc().channels().len(), channels);
}

#[test]
fn a_fill_layer_in_the_main_set_does_not_change_a_liltoon_masks_final_result() {
    let mut s = AppState::new(64, 64);
    let base = first_set(&s);
    s.act(Action::Look(LookOp::PaintSlot("_RimShadeMask")));
    let mask = s.m2().paint_channel;
    let before = s.doc().composite_pixel(mask, 5, 5).unwrap();
    // メインに、マスクのチャンネルにも黒の値を持つ塗りつぶしレイヤー
    let above = layersets::top_layer_of(s.doc(), base);
    let black = Rgba8::new(0, 0, 0, 255);
    let fill = s
        .doc_mut()
        .add_fill_layer("塗り", &[(Channel::Color, black), (mask, black)], above)
        .unwrap();
    assert_eq!(s.doc().layer_set_of(fill).unwrap(), base);
    assert_eq!(s.doc().composite_pixel(mask, 5, 5).unwrap(), before);
    assert_ne!(before, black);
    assert_eq!(
        s.doc().composite_pixel(Channel::Color, 5, 5).unwrap(),
        black
    );
}

#[test]
fn at_the_layer_set_limit_a_liltoon_channel_goes_in_without_its_own_set_and_says_so() {
    let mut s = AppState::new(64, 64);
    while s.doc().layer_sets().len() < yolu_core::MAX_LAYER_SETS {
        s.doc_mut().add_layer_set("x").unwrap();
    }
    s.message_mut().clear();
    s.act(Action::Look(LookOp::PaintSlot("_RimShadeMask")));
    let channel = s
        .doc()
        .channels()
        .into_iter()
        .find(|c| !c.is_standard())
        .expect("チャンネルは足す");
    assert_eq!(s.doc().layer_sets().len(), yolu_core::MAX_LAYER_SETS);
    assert_eq!(
        s.doc().channel_output(channel).unwrap(),
        Some(first_set(&s)),
        "前と同じく最初のレイヤーセットから"
    );
    assert_eq!(
        s.message(),
        "レイヤーセットが上限（256）なので、チャンネルを専用のレイヤーセットに分けずに追加しました。"
    );
}

#[test]
fn emptying_a_layer_sets_channels_is_allowed_and_says_so() {
    let mut s = AppState::new(64, 64);
    let base = first_set(&s);
    s.message_mut().clear();
    // 1 つ残っているうちは知らせない
    let channels = s.doc().channels();
    let (last, others) = channels.split_last().unwrap();
    for c in others {
        toggle(&mut s, base, *c);
    }
    assert_eq!(s.doc().layer_set_channels(base).unwrap(), last.bit());
    assert!(
        !s.message().contains("チャンネルがなくなりました"),
        "{}",
        s.message()
    );
    // 最後の 1 つを外して空にする: 断らず、注意の知らせを 1 つ出す（下書きのレイヤーセットのため）
    s.message_mut().clear();
    toggle(&mut s, base, *last);
    assert_eq!(s.doc().layer_set_channels(base).unwrap(), 0);
    assert!(
        s.message().contains("が扱うチャンネルがなくなりました"),
        "{}",
        s.message()
    );
    // 元に戻せる
    s.act(Action::Undo);
    assert_ne!(s.doc().layer_set_channels(base).unwrap(), 0);
}
