//! 通過のグループにクリッピングされたレイヤーが 1 枚でもあれば、そのレイヤーが見えるかによらずグループは分離する（Photoshop と同じ）。
//! 文書は、基のラスター（左 赤・右 青）の上に通過のグループ（下から緑の帯のラスターと反転の調整）を置き、グループの上にレイヤーをクリッピングする形。
//! 外側（緑の帯の外）の画素は、分離しなければ反転し、分離すれば基のまま。

use yolu_core::{AdjustmentSettings, BlendMode, Channel, Document, LayerId, Rgba8};

const RED: Rgba8 = Rgba8::new(200, 0, 0, 255);
const BLUE: Rgba8 = Rgba8::new(0, 0, 255, 255);
const GREEN: Rgba8 = Rgba8::new(0, 255, 0, 255);
const CYAN: Rgba8 = Rgba8::new(0, 255, 255, 255);
const W: u32 = 8;
const H: u32 = 2;

/// グループの上にクリッピングするレイヤーの状態。
#[derive(Clone, Copy, Debug)]
enum Clip {
    /// クリッピングされたレイヤーが無い。
    None,
    /// 非表示のラスター。
    Hidden,
    /// 表示で透明なラスター。
    Transparent,
    /// 色のチャンネルを切ったラスター。
    ChannelOff,
    /// 不透明度 0 のラスター。
    OpacityZero,
    /// 色の値の無い塗りつぶし（そのチャンネルに中身が無い）。
    NoContent,
    /// 見える画素のあるラスター。
    Visible,
}

const ALL: [Clip; 7] = [
    Clip::None,
    Clip::Hidden,
    Clip::Transparent,
    Clip::ChannelOff,
    Clip::OpacityZero,
    Clip::NoContent,
    Clip::Visible,
];

fn fill_rect(d: &mut Document, id: LayerId, c: Rgba8, x0: u32, x1: u32) {
    for y in 0..H {
        for x in x0..x1 {
            d.set_pixel(id, x, y, c).unwrap();
        }
    }
}

/// 基・通過のグループ・クリッピングされたレイヤーの文書。グループの id と、クリッピングされたレイヤーの id（None なら無い）を返す。
fn scene(clip: Clip, group_mode: BlendMode) -> (Document, LayerId, Option<LayerId>) {
    let mut d = Document::with_tile_size(W, H, 4).unwrap();
    let base = d.add_layer("基").unwrap();
    fill_rect(&mut d, base, RED, 0, 4);
    fill_rect(&mut d, base, BLUE, 4, W);
    let band = d.add_layer("緑").unwrap();
    fill_rect(&mut d, band, GREEN, 2, 6);
    let invert = d
        .add_adjustment_layer("反転", AdjustmentSettings::invert(), None, None)
        .unwrap();
    let g = d.group_layers(&[band, invert], "グループ").unwrap();
    d.set_layer_blend_mode(g, group_mode).unwrap();
    let above = match clip {
        Clip::None => None,
        Clip::NoContent => {
            let f = d
                .add_fill_layer("粗さだけ", &[(Channel::Roughness, RED)], None)
                .unwrap();
            Some(f)
        }
        _ => Some(d.add_layer("クリッピング").unwrap()),
    };
    if let Some(c) = above {
        match clip {
            Clip::Visible => fill_rect(&mut d, c, CYAN, 3, 5),
            Clip::Hidden => {
                fill_rect(&mut d, c, CYAN, 3, 5);
                d.set_layer_visible(c, false).unwrap();
            }
            Clip::Transparent => {}
            Clip::ChannelOff => {
                fill_rect(&mut d, c, CYAN, 3, 5);
                d.set_channel_enabled(c, Channel::Color, false).unwrap();
            }
            Clip::OpacityZero => {
                fill_rect(&mut d, c, CYAN, 3, 5);
                d.set_layer_opacity(c, 0.0, false).unwrap();
            }
            Clip::NoContent | Clip::None => {}
        }
        d.set_layer_clipping(c, true).unwrap();
    }
    d.clear_history().unwrap();
    (d, g, above)
}

fn px(bytes: &[u8], x: u32, y: u32) -> Rgba8 {
    let i = ((y * W + x) * 4) as usize;
    Rgba8::from_slice(&bytes[i..i + 4])
}

/// タイルの経路と画素ごとの参照が同じ値で、外側・中・クリッピングの画素を返す。
fn composite(d: &Document) -> Vec<u8> {
    let all = d.composite_channel(Channel::Color, d.bounds()).unwrap();
    for y in 0..H {
        for x in 0..W {
            assert_eq!(
                px(&all, x, y),
                d.composite_pixel(Channel::Color, x, y).unwrap(),
                "タイルの経路と参照の画素が違う ({x},{y})"
            );
        }
    }
    all
}

const INVERTED_RED: Rgba8 = Rgba8::new(55, 255, 255, 255);
const INVERTED_BLUE: Rgba8 = Rgba8::new(255, 255, 0, 255);
/// 緑の帯は不透明なので、反転しても分離しても同じ値（反転した緑）。
const INVERTED_GREEN: Rgba8 = Rgba8::new(255, 0, 255, 255);

#[test]
fn a_pass_through_group_with_any_clipped_layer_is_isolated_whether_or_not_it_shows() {
    for clip in ALL {
        let (d, _, _) = scene(clip, BlendMode::PassThrough);
        let out = composite(&d);
        let isolated = !matches!(clip, Clip::None);
        let (red, blue) = if isolated {
            (RED, BLUE)
        } else {
            (INVERTED_RED, INVERTED_BLUE)
        };
        assert_eq!(px(&out, 0, 0), red, "{clip:?}: 帯の外（赤）");
        assert_eq!(px(&out, 7, 1), blue, "{clip:?}: 帯の外（青）");
        assert_eq!(
            px(&out, 2, 0),
            INVERTED_GREEN,
            "{clip:?}: 帯の中は、分離しても反転する"
        );
        // 見える画素のクリッピングだけが、グループの上へ重なる（帯の中 3..5）
        let over_band = if matches!(clip, Clip::Visible) {
            CYAN
        } else {
            INVERTED_GREEN
        };
        assert_eq!(px(&out, 3, 0), over_band, "{clip:?}: クリッピングの画素");
        assert_eq!(px(&out, 5, 0), INVERTED_GREEN, "{clip:?}: 帯の右の端");
    }
}

#[test]
fn an_isolated_group_without_clipping_and_a_pass_through_group_with_a_visible_clip_do_not_change() {
    // 通過でないグループ: クリッピングの有無によらず元から分離（外側は反転しない）
    for clip in [Clip::None, Clip::Hidden, Clip::Visible] {
        let (d, _, _) = scene(clip, BlendMode::Normal);
        let out = composite(&d);
        assert_eq!(px(&out, 0, 0), RED, "{clip:?}");
        assert_eq!(px(&out, 7, 1), BLUE, "{clip:?}");
    }
    // クリッピングの無い通過のグループは今までどおり反転する
    let (d, _, _) = scene(Clip::None, BlendMode::PassThrough);
    assert_eq!(px(&composite(&d), 0, 0), INVERTED_RED);
}

#[test]
fn removing_or_unclipping_the_hidden_layer_puts_the_group_back_to_pass_through() {
    let (mut d, _, above) = scene(Clip::Hidden, BlendMode::PassThrough);
    let c = above.unwrap();
    assert_eq!(px(&composite(&d), 0, 0), RED);
    d.set_layer_clipping(c, false).unwrap();
    assert_eq!(px(&composite(&d), 0, 0), INVERTED_RED);
    d.undo().unwrap();
    assert_eq!(px(&composite(&d), 0, 0), RED);
    d.remove_layer(c).unwrap();
    assert_eq!(px(&composite(&d), 0, 0), INVERTED_RED);
    d.undo().unwrap();
    assert_eq!(px(&composite(&d), 0, 0), RED);
}

#[test]
fn a_clipped_layer_in_another_layer_set_does_not_count() {
    // 評価しているレイヤーセットに入らないクリッピングのレイヤーは数えない（そのレイヤーセットの合成では、通過のまま）
    let (mut d, _, above) = scene(Clip::Hidden, BlendMode::PassThrough);
    let c = above.unwrap();
    let first = d.layer_sets()[0].id;
    let other = d.add_layer_set("別").unwrap();
    d.move_layers_to_set(&[c], other).unwrap();
    d.clear_history().unwrap();
    let set_first = d
        .composite_set_channel(first, Channel::Color, d.bounds())
        .unwrap();
    assert_eq!(px(&set_first, 0, 0), INVERTED_RED, "最初のレイヤーセット");
    assert_eq!(px(&set_first, 7, 1), INVERTED_BLUE);
    // 最終の結果（最初のレイヤーセットが出力）も同じ
    assert_eq!(px(&composite(&d), 0, 0), INVERTED_RED);
    // 別のレイヤーセットには下地が無い（クリッピングの印は一番下で効かない）ので、何も描かない
    let set_other = d
        .composite_set_channel(other, Channel::Color, d.bounds())
        .unwrap();
    assert_eq!(px(&set_other, 0, 0), Rgba8::TRANSPARENT);
    // 同じレイヤーセットに戻せば分離する
    d.move_layers_to_set(&[c], first).unwrap();
    assert_eq!(px(&composite(&d), 0, 0), RED);
}

/// 色のチャンネルを切った塗りつぶしがグループにクリッピングされ、その上の空のラスターを下へ結合する。結合の結果（色の面を持つラスター）は
/// 組に入るが、結合の前の塗りつぶしは色が切れていた。どちらもグループを分離するので、見た目は変わらず、許容差 0 で結合できる。
#[test]
fn merging_into_a_clipped_fill_with_the_color_channel_off_does_not_change_the_look() {
    let (mut d, _, _) = scene(Clip::None, BlendMode::PassThrough);
    let tint = d
        .add_fill_layer("色", &[(Channel::Color, CYAN)], None)
        .unwrap();
    d.set_channel_enabled(tint, Channel::Color, false).unwrap();
    d.set_layer_clipping(tint, true).unwrap();
    let top = d.add_layer("空").unwrap();
    d.clear_history().unwrap();
    let before = composite(&d);
    assert_eq!(px(&before, 0, 0), RED, "色を切った塗りつぶしでも分離する");
    let report = d.merge_down(top, 0).unwrap();
    assert_eq!(report.changed_pixels, 0, "{report:?}");
    assert_eq!(composite(&d), before);
}
