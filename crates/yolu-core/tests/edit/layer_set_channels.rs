//! レイヤーセットの扱うチャンネルの組: 組に無いチャンネルは合成に入らない、出力の割り当ては組に持つレイヤーセットへだけ、組を替える・チャンネルを
//! 足す・消す・複製の 1 回の取り消し、読み込みの決まり、前の形の文書の合成が変わらないこと。

use yolu_core::Reason;
use yolu_core::{
    Channel, ChannelInfo, ChannelKind, ColorSpace, CoreError, Document, LayerId, LayerSetId,
    LayerSetInfo, Rgba8,
};

const RED: Rgba8 = Rgba8::new(220, 20, 30, 255);
const GRAY: Rgba8 = Rgba8::new(90, 90, 90, 255);
const WHITE: Rgba8 = Rgba8::new(255, 255, 255, 255);

fn mask_info(name: &str) -> ChannelInfo {
    ChannelInfo {
        name: name.into(),
        kind: ChannelKind::Scalar,
        color_space: ColorSpace::Linear,
        default: Rgba8::new(0, 0, 0, 255),
    }
}

fn all(d: &Document, c: Channel) -> Vec<u8> {
    d.composite_channel(c, d.bounds()).unwrap()
}

fn set_all(d: &Document, set: LayerSetId, c: Channel) -> Vec<u8> {
    d.composite_set_channel(set, c, d.bounds()).unwrap()
}

fn uniform(c: Rgba8, n: usize) -> Vec<u8> {
    c.to_array().repeat(n)
}

fn paint(d: &mut Document, id: LayerId, channel: Channel, c: Rgba8, x0: u32, x1: u32) {
    for y in 0..d.height() {
        for x in x0..x1 {
            d.set_channel_pixel(id, channel, x, y, c).unwrap();
        }
    }
}

/// メイン（最初のレイヤーセット）に全部のチャンネルの塗りつぶしレイヤー、マスクのチャンネルはマスクのレイヤーセットだけが扱い、出力もそこから。
fn main_and_mask() -> (Document, LayerSetId, LayerSetId, Channel, LayerId) {
    let mut d = Document::with_tile_size(24, 16, 8).unwrap();
    let main = d.layer_sets()[0].id;
    let mask_set = d.add_layer_set_with("マスク", 0).unwrap();
    let mask = d
        .add_channel_in_set(mask_info("影のマスク"), mask_set)
        .unwrap();
    let fill = d
        .add_fill_layer("塗り", &[(Channel::Color, RED), (mask, GRAY)], None)
        .unwrap();
    let m = d.add_layer_in_set_on("m", mask_set, None, mask).unwrap();
    paint(&mut d, m, mask, WHITE, 0, 8);
    d.clear_history().unwrap();
    (d, main, mask_set, mask, fill)
}

#[test]
fn a_channel_outside_the_set_does_not_enter_its_composite() {
    let (d, main, mask_set, mask, fill) = main_and_mask();
    let n = 24 * 16;
    assert_eq!(
        d.layer_set_channels(main).unwrap(),
        d.standard_channel_bits()
    );
    assert_eq!(d.layer_set_channels(mask_set).unwrap(), mask.bit());
    assert_eq!(d.channel_output(mask).unwrap(), Some(mask_set));
    // メインの塗りつぶしはマスクのチャンネルの値を持つが、メインの組に無いので、メインの合成にもレイヤーの入切にも出ない
    assert_eq!(set_all(&d, main, mask), uniform(Rgba8::TRANSPARENT, n));
    assert!(!d.contributes(fill, mask).unwrap());
    assert!(d.contributes(fill, Channel::Color).unwrap());
    // 最終のマスクはマスクのレイヤーセットの合成だけ（メインの塗りつぶしの灰色が入らない）
    let result = all(&d, mask);
    assert_eq!(result, set_all(&d, mask_set, mask));
    assert!(result
        .as_chunks::<4>()
        .0
        .iter()
        .all(|p| *p != GRAY.to_array()));
    // マスクのレイヤーセットは Color を扱わないので、Color の合成は空
    assert_eq!(
        set_all(&d, mask_set, Channel::Color),
        uniform(Rgba8::TRANSPARENT, n)
    );
    assert_eq!(all(&d, Channel::Color), uniform(RED, n));
    // 塗りつぶしの面は残っている（組に戻せば出る）
    let mut d = d;
    d.set_layer_set_channels(main, d.standard_channel_bits() | mask.bit())
        .unwrap();
    assert_eq!(set_all(&d, main, mask), uniform(GRAY, n));
    assert!(d.contributes(fill, mask).unwrap());
}

#[test]
fn the_output_goes_only_to_a_set_that_has_the_channel() {
    let (mut d, main, mask_set, mask, _) = main_and_mask();
    let before = all(&d, mask);
    assert_eq!(
        d.set_channel_output(mask, Some(main)),
        Err(CoreError::InvalidArgument(Reason::ChannelNotInLayerSet))
    );
    assert_eq!(
        d.set_channel_output(Channel::Color, Some(mask_set)),
        Err(CoreError::InvalidArgument(Reason::ChannelNotInLayerSet))
    );
    assert_eq!(d.channel_output(mask).unwrap(), Some(mask_set));
    assert_eq!(d.undo_count(), 0, "断ったら何も変えない");
    assert_eq!(all(&d, mask), before);
    // 割り当てを外す（None）のは、組によらずできる
    d.set_channel_output(mask, None).unwrap();
    assert_eq!(d.channel_output(mask).unwrap(), None);
}

#[test]
fn dropping_a_channel_from_the_set_clears_its_output_and_one_undo_restores_both() {
    let (mut d, main, mask_set, mask, _) = main_and_mask();
    let n = 24 * 16;
    let before = all(&d, mask);
    let color = all(&d, Channel::Color);
    let serial = d.change_serial();
    d.set_layer_set_channels(mask_set, 0).unwrap();
    assert_eq!(d.layer_set_channels(mask_set).unwrap(), 0);
    // ほかのレイヤーセットへは移さず、割り当てを外す（最終の結果は既定値）
    assert_eq!(d.channel_output(mask).unwrap(), None);
    assert_eq!(all(&d, mask), uniform(mask_info("x").default, n));
    assert_eq!(
        d.changed_tiles(mask, serial).unwrap().len(),
        d.canvas_tiles().count()
    );
    assert_eq!(d.undo_count(), 1);
    // メインの Color は変わらない
    assert_eq!(all(&d, Channel::Color), color);
    assert_eq!(d.channel_output(Channel::Color).unwrap(), Some(main));
    d.undo().unwrap();
    assert_eq!(d.layer_set_channels(mask_set).unwrap(), mask.bit());
    assert_eq!(d.channel_output(mask).unwrap(), Some(mask_set));
    assert_eq!(all(&d, mask), before);
    d.redo().unwrap();
    assert_eq!(d.channel_output(mask).unwrap(), None);
}

#[test]
fn changing_the_channels_of_a_set_marks_the_tiles_of_its_layers() {
    let (mut d, main, _, mask, _) = main_and_mask();
    // メインの組にマスクを入れると、メインのマスクの合成（2D の表示が読む）は塗りつぶしの分だけ変わる
    let serial = d.change_serial();
    d.set_layer_set_channels(main, d.standard_channel_bits() | mask.bit())
        .unwrap();
    assert_eq!(
        d.changed_tiles(mask, serial).unwrap().len(),
        d.canvas_tiles().count()
    );
    assert!(d.changed_tiles(Channel::Color, serial).unwrap().is_empty());
}

#[test]
fn channels_outside_the_document_are_refused() {
    let (mut d, main, _, _, _) = main_and_mask();
    let missing = Channel::from_index(40).unwrap();
    assert_eq!(
        d.add_layer_set_with("x", missing.bit()),
        Err(CoreError::InvalidArgument(Reason::LayerSetChannels))
    );
    assert_eq!(
        d.set_layer_set_channels(main, missing.bit()),
        Err(CoreError::InvalidArgument(Reason::LayerSetChannels))
    );
    assert_eq!(d.layer_sets().len(), 2);
    assert_eq!(d.undo_count(), 0);
}

#[test]
fn removing_a_channel_takes_it_out_of_every_set_and_undo_puts_it_back() {
    let (mut d, main, mask_set, mask, _) = main_and_mask();
    d.set_layer_set_channels(main, d.standard_channel_bits() | mask.bit())
        .unwrap();
    d.clear_history().unwrap();
    let before = all(&d, mask);
    d.remove_channel(mask).unwrap();
    assert_eq!(d.undo_count(), 1);
    assert_eq!(
        d.layer_set_channels(main).unwrap(),
        d.standard_channel_bits()
    );
    assert_eq!(d.layer_set_channels(mask_set).unwrap(), 0);
    d.undo().unwrap();
    assert_eq!(
        d.layer_set_channels(main).unwrap(),
        d.standard_channel_bits() | mask.bit()
    );
    assert_eq!(d.layer_set_channels(mask_set).unwrap(), mask.bit());
    assert_eq!(d.channel_output(mask).unwrap(), Some(mask_set));
    assert_eq!(all(&d, mask), before);
}

#[test]
fn a_duplicate_keeps_the_channels() {
    let (mut d, _, mask_set, mask, _) = main_and_mask();
    let copy = d.duplicate_layer_set(mask_set, None).unwrap();
    assert_eq!(d.layer_set_channels(copy).unwrap(), mask.bit());
    assert_eq!(set_all(&d, copy, mask), set_all(&d, mask_set, mask));
    assert_eq!(
        set_all(&d, copy, Channel::Color),
        set_all(&d, mask_set, Channel::Color)
    );
}

#[test]
fn add_channel_in_set_is_one_undo() {
    let mut d = Document::with_tile_size(24, 16, 8).unwrap();
    let main = d.layer_sets()[0].id;
    let set = d.add_layer_set_with("s", 0).unwrap();
    d.clear_history().unwrap();
    let c = d.add_channel_in_set(mask_info("m"), set).unwrap();
    assert_eq!(d.layer_set_channels(set).unwrap(), c.bit());
    assert_eq!(
        d.layer_set_channels(main).unwrap(),
        d.standard_channel_bits()
    );
    assert_eq!(d.channel_output(c).unwrap(), Some(set));
    assert_eq!(d.undo_count(), 1);
    assert_eq!(d.history().last(), Some(yolu_core::HistoryKind::Channel));
    d.undo().unwrap();
    assert!(d.channel_info(c).is_none());
    assert_eq!(d.layer_set_channels(set).unwrap(), 0);
    d.redo().unwrap();
    assert_eq!(d.layer_set_channels(set).unwrap(), c.bit());
    assert_eq!(d.channel_output(c).unwrap(), Some(set));
}

#[test]
fn add_channel_goes_into_the_set_of_its_output_as_before() {
    let mut d = Document::with_tile_size(24, 16, 8).unwrap();
    let main = d.layer_sets()[0].id;
    let c = d.add_channel(mask_info("m")).unwrap();
    // 新しい文書では最初のレイヤーセットから出て、その組に入る（前と同じ結果）
    assert_eq!(d.channel_output(c).unwrap(), Some(main));
    assert_eq!(d.layer_set_channels(main).unwrap(), d.channel_bits());
    assert!(d.has_default_layer_sets());
    let fill = d.add_fill_layer("f", &[(c, GRAY)], None).unwrap();
    assert!(d.contributes(fill, c).unwrap());
    assert_eq!(all(&d, c), uniform(GRAY, 24 * 16));
    assert_eq!(d.undo_count(), 2);
    d.undo().unwrap();
    d.undo().unwrap();
    assert_eq!(
        d.layer_set_channels(main).unwrap(),
        d.standard_channel_bits()
    );
    assert!(d.has_default_layer_sets());
}

#[test]
fn loading_checks_the_channels_against_the_routes() {
    let mut d = Document::with_tile_size(24, 16, 8).unwrap();
    let c = d.add_channel(mask_info("m")).unwrap();
    let a = d.add_layer("a").unwrap();
    d.clear_history().unwrap();
    let _ = a;
    let (s1, s2) = (LayerSetId(11), LayerSetId(22));
    let std = d.standard_channel_bits();
    let set = |id, channels| LayerSetInfo {
        id,
        name: String::new(),
        channels,
    };
    // 割り当て先が組に持たない・文書に無いチャンネルのビット
    for (sets, route) in [
        (vec![set(s1, std), set(s2, 0)], vec![(c, Some(s2))]),
        (
            vec![set(s1, std | c.bit()), set(s2, 0)],
            vec![(Channel::Color, Some(s2))],
        ),
        (vec![set(s1, std | c.bit() | (1 << 40)), set(s2, 0)], vec![]),
    ] {
        assert!(d.set_layer_sets_for_load(sets, &route, &[s1]).is_err());
        assert_eq!(d.layer_sets().len(), 1, "断ったら何も変えない");
    }
    d.set_layer_sets_for_load(
        vec![set(s1, std), set(s2, c.bit())],
        &[(c, Some(s2))],
        &[s1],
    )
    .unwrap();
    assert_eq!(d.layer_set_channels(s1).unwrap(), std);
    assert_eq!(d.layer_set_channels(s2).unwrap(), c.bit());
    assert_eq!(d.channel_output(c).unwrap(), Some(s2));
}

#[test]
fn the_old_form_composites_the_same() {
    // 組の欄の無い前の形: 全部のレイヤーセットが全部のチャンネルを扱う
    let (d, main, mask_set, mask, _) = main_and_mask();
    let mut old = d.capture_snapshot().unwrap();
    let channels = old.channel_bits();
    let sets: Vec<LayerSetInfo> = old
        .layer_sets()
        .iter()
        .map(|s| LayerSetInfo {
            channels,
            ..s.clone()
        })
        .collect();
    let members: Vec<LayerSetId> = old.layers().iter().map(|l| l.layer_set()).collect();
    let route: Vec<_> = old
        .channels()
        .into_iter()
        .map(|c| (c, old.channel_output(c).unwrap()))
        .collect();
    old.set_layer_sets_for_load(sets, &route, &members).unwrap();
    // 出力の割り当ての合成は、組を全部にしても同じ（割り当て先はそのチャンネルを持っていた）
    for c in old.channels() {
        assert_eq!(all(&old, c), all(&d, c), "{c:?}");
    }
    // 組を全部にしたメインは、マスクのチャンネルで塗りつぶしを合成する（前のバージョンの振る舞い）
    assert_eq!(set_all(&old, main, mask), uniform(GRAY, 24 * 16));
    assert_eq!(set_all(&old, mask_set, mask), set_all(&d, mask_set, mask));
}

#[test]
fn merging_visible_layers_notes_content_of_channels_outside_the_set() {
    // 塗りつぶし（画素の面なし）が組の外のチャンネルに値を持つ: 結合は断らず、除いたことを notes の 32 で知らせる。取り消しで戻る
    let (mut d, main, _, mask, fill) = main_and_mask();
    let report = d.merge_visible_in(main, "結合", 2).unwrap();
    assert_ne!(report.notes & 32, 0);
    assert!(d.layer(fill).is_none());
    assert!(d.undo().unwrap());
    assert!(d.layer(fill).is_some());
    assert!(d.contributes(fill, Channel::Color).unwrap());
    assert!(!d.contributes(fill, mask).unwrap());

    // 画素のラスターのレイヤーが組の外のチャンネルに画素を持つ
    let (mut d, main, _, mask, _) = main_and_mask();
    d.remove_layer(d.layers()[0].id()).unwrap();
    let r = d.add_layer_in_set("r", main, None).unwrap();
    paint(&mut d, r, Channel::Color, RED, 0, 8);
    paint(&mut d, r, mask, WHITE, 0, 8);
    let report = d.merge_visible_in(main, "結合", 2).unwrap();
    assert_ne!(report.notes & 32, 0);

    // 組の中のチャンネルだけなら立たない
    let (mut d, main, _, _, _) = main_and_mask();
    d.remove_layer(d.layers()[0].id()).unwrap();
    let r = d.add_layer_in_set("r", main, None).unwrap();
    paint(&mut d, r, Channel::Color, RED, 0, 8);
    let report = d.merge_visible_in(main, "結合", 2).unwrap();
    assert_eq!(report.notes & 32, 0);

    // 組を広げれば（そのチャンネルを結合する）立たない
    let (mut d, main, _, mask, _) = main_and_mask();
    d.set_layer_set_channels(main, d.standard_channel_bits() | mask.bit())
        .unwrap();
    let report = d.merge_visible_in(main, "結合", 2).unwrap();
    assert_eq!(report.notes & 32, 0);
}
