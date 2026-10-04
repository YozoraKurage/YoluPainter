//! チャンネル: チャンネルごとの合成（C# の ChannelBlendTests）、Normal の合成と出力・Height → Normal（C# の NormalChannelTests、
//! 期待値は C# の試験と同じ）、チャンネルの有効の Undo、ユーザーチャンネル（Rust 版だけ）。

use std::collections::HashMap;

use yolu_core::normal::{self, DEFAULT_WORKING_BUDGET_BYTES as BUDGET};
use yolu_core::{
    AdjustmentSettings, BlendMode, BrushSettings, Channel, ChannelBlend, ChannelInfo, ChannelKind,
    ColorSpace, CoreError, Document, HeightEdgeMode, LayerId, NormalSettings, NormalYDirection,
    Rgba8, TileCoord,
};

const FLAT: Rgba8 = Rgba8::new(128, 128, 255, 255);
const TILT_X: Rgba8 = Rgba8::new(255, 128, 128, 255);
const TILT45: Rgba8 = Rgba8::new(218, 128, 218, 255);

/// 層を足し、チャンネルを有効にして value で埋める（全部 0 の画素は書かない）。
fn fill_layer(
    d: &mut Document,
    name: &str,
    ch: Channel,
    value: impl Fn(u32, u32) -> Rgba8,
) -> LayerId {
    let id = d.add_layer(name).unwrap();
    d.set_channel_enabled(id, ch, true).unwrap();
    for y in 0..d.height() {
        for x in 0..d.width() {
            let v = value(x, y);
            if v != Rgba8::TRANSPARENT {
                d.set_channel_pixel(id, ch, x, y, v).unwrap();
            }
        }
    }
    id
}
fn px(rgba: &[u8], width: u32, x: u32, y: u32) -> Rgba8 {
    Rgba8::from_slice(&rgba[((y * width + x) * 4) as usize..])
}
fn derive(strength: f64, edges: HeightEdgeMode, dir: NormalYDirection) -> NormalSettings {
    NormalSettings::new(true, strength, edges, dir).unwrap()
}

// ───────── チャンネルごとの合成（C# ChannelBlendTests） ─────────

const W: u32 = 40;
const H: u32 = 32;
const PAINTED: [Channel; 4] = [
    Channel::Color,
    Channel::Roughness,
    Channel::Height,
    Channel::Normal,
];
fn v(i: i64) -> u8 {
    (i.rem_euclid(16) * 17) as u8
}
fn opaque(x: u32, y: u32, c: Channel) -> Rgba8 {
    let (x, y, c) = (x as i64, y as i64, c.index() as i64);
    Rgba8::new(v(x / 3 + c), v(y / 2), v((x + y) / 4), 255)
}
fn partial(x: u32, y: u32, c: Channel) -> Rgba8 {
    let (x, y, c) = (x as i64, y as i64, c.index() as i64);
    if (x + 2 * y) % 7 == 0 {
        return Rgba8::TRANSPARENT;
    }
    Rgba8::new(
        v(15 - x / 3),
        v(x + y + c),
        v(y / 3),
        if x < 20 { 220 } else { 130 },
    )
}
fn spots(x: u32, y: u32, c: Channel) -> Rgba8 {
    if (x / 5 + y / 5).is_multiple_of(2) {
        Rgba8::new(v(y as i64 + c.index() as i64 * 3), v(x as i64), 200, 180)
    } else {
        Rgba8::TRANSPARENT
    }
}
fn paint_all(d: &mut Document, id: LayerId, pixel: &dyn Fn(u32, u32, Channel) -> Rgba8) {
    for c in PAINTED {
        d.set_channel_enabled(id, c, true).unwrap();
        for y in 0..H {
            for x in 0..W {
                let p = pixel(x, y, c);
                if p.a != 0 {
                    d.set_channel_pixel(id, c, x, y, p).unwrap();
                }
            }
        }
    }
}

/// 下地・通過のグループ（Screen の a、b）・グループにクリッピングした層・レベル補正・マスク付きの上の層。with なら層の値に
/// チャンネルごとの設定を足す。flatten なら設定を付けず、そのチャンネルでの値を層の値にする（比べる相手）。
fn build(with: bool, flatten: Option<Channel>) -> (Document, HashMap<&'static str, LayerId>) {
    let mut d = Document::with_tile_size(W, H, 16).unwrap();
    let back = d.add_layer("back").unwrap();
    paint_all(&mut d, back, &opaque);
    let a = d.add_layer("a").unwrap();
    paint_all(&mut d, a, &partial);
    let b = d.add_layer("b").unwrap();
    paint_all(&mut d, b, &spots);
    let g = d.group_layers(&[a, b], "group").unwrap();
    let clip = d.add_layer("clip").unwrap();
    paint_all(&mut d, clip, &|x, y, c| spots(y, x, c));
    let adjust = d
        .add_adjustment_layer(
            "levels",
            AdjustmentSettings::levels(0.1, 0.9, 1.4, 0.0, 1.0).unwrap(),
            None,
            None,
        )
        .unwrap();
    let top = d.add_layer("top").unwrap();
    paint_all(&mut d, top, &|x, y, c| partial(y, x, c));
    d.add_layer_mask(top).unwrap();
    for x in 0..W {
        d.set_mask_pixel(top, x, x % H, 200).unwrap();
    }
    d.set_layer_clipping(clip, true).unwrap();
    let plain = [
        (a, BlendMode::Screen, 0.9),
        (b, BlendMode::Normal, 0.8),
        (g, BlendMode::PassThrough, 1.0),
        (clip, BlendMode::Normal, 1.0),
        (adjust, BlendMode::Normal, 0.7),
        (top, BlendMode::Normal, 1.0),
    ];
    let own: Vec<(LayerId, Channel, ChannelBlend)> = vec![
        (
            g,
            Channel::Roughness,
            ChannelBlend::new(Some(BlendMode::Screen), Some(0.7)),
        ),
        (g, Channel::Height, ChannelBlend::new(None, Some(0.5))),
        (
            a,
            Channel::Color,
            ChannelBlend::new(Some(BlendMode::Multiply), None),
        ),
        (
            a,
            Channel::Normal,
            ChannelBlend::new(Some(BlendMode::Overlay), None),
        ),
        (clip, Channel::Roughness, ChannelBlend::new(None, Some(0.4))),
        (
            adjust,
            Channel::Height,
            ChannelBlend::new(Some(BlendMode::Overlay), Some(0.6)),
        ),
        (
            top,
            Channel::Normal,
            ChannelBlend::new(Some(BlendMode::Normal), Some(0.5)),
        ),
        (
            top,
            Channel::Roughness,
            ChannelBlend::new(Some(BlendMode::Darken), Some(1.0)),
        ),
    ];
    for (id, mut mode, mut opacity) in plain {
        if let Some(f) = flatten {
            if let Some((_, _, b)) = own.iter().find(|(l, c, _)| *l == id && *c == f) {
                mode = b.mode.unwrap_or(mode);
                opacity = b.opacity.unwrap_or(opacity);
            }
        }
        d.set_layer_blend_mode(id, mode).unwrap();
        d.set_layer_opacity(id, opacity, false).unwrap();
    }
    if with && flatten.is_none() {
        for (id, c, b) in &own {
            d.set_channel_blend(*id, *c, *b, false).unwrap();
        }
    }
    d.clear_history().unwrap();
    let names = HashMap::from([
        ("a", a),
        ("b", b),
        ("group", g),
        ("clip", clip),
        ("levels", adjust),
        ("top", top),
    ]);
    (d, names)
}

#[test]
fn each_channel_composites_with_its_own_setting_and_the_others_with_the_layers() {
    let (with, _) = build(true, None);
    let (without, _) = build(false, None);
    for c in PAINTED {
        let expected = build(false, Some(c))
            .0
            .composite_channel(c, with.bounds())
            .unwrap();
        assert_eq!(
            with.composite_channel(c, with.bounds()).unwrap(),
            expected,
            "{c:?}: そのチャンネルの値を層の値にした文書と同じバイト"
        );
        for y in (0..H).step_by(3) {
            for x in (0..W).step_by(5) {
                assert_eq!(
                    with.composite_pixel(c, x, y).unwrap(),
                    px(&expected, W, x, y)
                );
            }
        }
    }
    assert_eq!(
        with.composite_channel(Channel::Metallic, with.bounds())
            .unwrap(),
        without
            .composite_channel(Channel::Metallic, with.bounds())
            .unwrap()
    );
    assert_ne!(
        with.composite_channel(Channel::Roughness, with.bounds())
            .unwrap(),
        without
            .composite_channel(Channel::Roughness, with.bounds())
            .unwrap()
    );
}

#[test]
fn in_the_normal_channel_overlay_adds_detail_while_color_stays_normal() {
    let mut d = Document::with_tile_size(16, 16, 16).unwrap();
    let back = d.add_layer("back").unwrap();
    let top = d.add_layer("top").unwrap();
    let n1 = normal::encode(0.4, 0.0, 1.0, 255);
    let n2 = normal::encode(0.0, 0.3, 1.0, 255);
    for y in 0..16 {
        for x in 0..16 {
            d.set_channel_pixel(back, Channel::Normal, x, y, n1)
                .unwrap();
            d.set_pixel(back, x, y, Rgba8::new(10, 20, 30, 255))
                .unwrap();
            d.set_channel_pixel(top, Channel::Normal, x, y, n2).unwrap();
            d.set_pixel(top, x, y, Rgba8::new(200, 100, 50, 255))
                .unwrap();
        }
    }
    d.set_channel_blend_mode(top, Channel::Normal, Some(BlendMode::Overlay))
        .unwrap();
    assert_eq!(
        d.composite_pixel(Channel::Color, 3, 3).unwrap(),
        Rgba8::new(200, 100, 50, 255)
    );
    let n = d.composite_pixel(Channel::Normal, 3, 3).unwrap();
    assert_eq!(n, normal::blend(n1, n2, 1.0, BlendMode::Overlay));
    assert_ne!(n, n2);
}

#[test]
fn channel_blend_changes_are_one_undo_step_and_clearing_follows_the_layer() {
    let (mut d, ids) = build(false, None);
    let top = ids["top"];
    d.set_channel_blend_mode(top, Channel::Roughness, Some(BlendMode::Multiply))
        .unwrap();
    assert_eq!(d.undo_count(), 1);
    let l = d.layer(top).unwrap();
    assert_eq!(l.blend_mode_in(Channel::Roughness), BlendMode::Multiply);
    assert_eq!(l.opacity_in(Channel::Roughness), 1.0, "不透明度は層に従う");
    assert_eq!(l.blend_mode(), BlendMode::Normal);
    for o in [0.9, 0.7, 0.5] {
        d.set_channel_opacity(top, Channel::Roughness, Some(o), true)
            .unwrap();
    }
    assert_eq!(d.undo_count(), 2, "ドラッグは 1 段");
    d.end_coalescing();
    d.set_layer_opacity(top, 0.3, false).unwrap();
    let l = d.layer(top).unwrap();
    assert_eq!(l.opacity_in(Channel::Roughness), 0.5);
    assert_eq!(l.opacity_in(Channel::Color), 0.3);
    let set = d.composite_channel(Channel::Roughness, d.bounds()).unwrap();
    d.set_channel_blend(top, Channel::Roughness, ChannelBlend::default(), false)
        .unwrap();
    assert_eq!(
        d.layer(top).unwrap().channel_blends().count(),
        0,
        "空の設定は層に従う"
    );
    d.undo().unwrap();
    assert_eq!(
        d.composite_channel(Channel::Roughness, d.bounds()).unwrap(),
        set
    );
    d.undo().unwrap();
    d.undo().unwrap();
    assert_eq!(
        d.layer(top).unwrap().channel_blend(Channel::Roughness),
        ChannelBlend::new(Some(BlendMode::Multiply), None)
    );
    d.undo().unwrap();
    assert_eq!(d.layer(top).unwrap().channel_blends().count(), 0);
    assert_eq!(
        d.composite_channel(Channel::Roughness, d.bounds()).unwrap(),
        build(false, None)
            .0
            .composite_channel(Channel::Roughness, d.bounds())
            .unwrap()
    );
    d.redo().unwrap();
    d.redo().unwrap();
    assert_eq!(
        d.layer(top).unwrap().channel_blend(Channel::Roughness),
        ChannelBlend::new(Some(BlendMode::Multiply), Some(0.5))
    );
    let steps = d.undo_count();
    d.set_channel_opacity(top, Channel::Roughness, Some(0.5), false)
        .unwrap();
    assert_eq!(d.undo_count(), steps, "同じ値は記録しない");
    // 断る値は何も変えない
    assert!(d
        .set_channel_blend_mode(top, Channel::Color, Some(BlendMode::PassThrough))
        .is_err());
    assert!(d
        .set_channel_opacity(top, Channel::Color, Some(1.5), false)
        .is_err());
    assert!(d
        .set_channel_opacity(top, Channel::Color, Some(f64::NAN), false)
        .is_err());
    assert_eq!(
        d.set_channel_opacity(top, Channel::from_index(42).unwrap(), Some(0.5), false),
        Err(CoreError::ChannelNotFound)
    );
    assert_eq!(d.undo_count(), steps);
    let g = ids["group"];
    d.set_channel_blend_mode(g, Channel::Color, Some(BlendMode::Normal))
        .unwrap();
    assert_eq!(
        d.layer(g).unwrap().blend_mode_in(Channel::Color),
        BlendMode::Normal
    );
    d.set_channel_blend_mode(g, Channel::Color, Some(BlendMode::PassThrough))
        .unwrap();
}

#[test]
fn only_that_channels_tiles_are_reported_changed() {
    let (mut d, ids) = build(false, None);
    let since = d.change_serial();
    d.set_channel_opacity(ids["a"], Channel::Roughness, Some(0.2), false)
        .unwrap();
    assert!(!d
        .changed_tiles(Channel::Roughness, since)
        .unwrap()
        .is_empty());
    // ほかのチャンネルは、どの層の設定でもそうであるようにクリッピングの層だけ
    let clip = d.layer(ids["clip"]).unwrap();
    for c in [Channel::Color, Channel::Height, Channel::Normal] {
        let clipped = clip.surface(c).map(|s| s.tile_coords()).unwrap_or_default();
        for t in d.changed_tiles(c, since).unwrap() {
            assert!(clipped.contains(&t), "{c:?} {t:?}");
        }
    }
}

// ───────── チャンネルの有効 ─────────

#[test]
fn enabling_a_channel_makes_its_surface_and_undo_takes_it_away() {
    let mut d = Document::with_tile_size(16, 16, 8).unwrap();
    let l = d.add_layer("L").unwrap();
    d.clear_history().unwrap();
    d.set_channel_enabled(l, Channel::Height, true).unwrap();
    assert!(d.layer(l).unwrap().surface(Channel::Height).is_some());
    let brush = BrushSettings {
        radius: 3.0,
        color: Rgba8::new(200, 0, 0, 255),
        ..BrushSettings::default()
    };
    let mut s = d.begin_stroke_in(l, Channel::Height, &brush).unwrap();
    s.add_point(&mut d, 4.0, 4.0, 1.0, Default::default())
        .unwrap();
    d.end_stroke(s).unwrap();
    let painted = d
        .layer(l)
        .unwrap()
        .surface(Channel::Height)
        .unwrap()
        .to_canvas_bytes();
    d.undo().unwrap();
    d.undo().unwrap();
    assert!(
        d.layer(l).unwrap().surface(Channel::Height).is_none(),
        "空の面は外す"
    );
    assert!(!d.layer(l).unwrap().is_channel_enabled(Channel::Height));
    d.redo().unwrap();
    d.redo().unwrap();
    assert_eq!(
        d.layer(l)
            .unwrap()
            .surface(Channel::Height)
            .unwrap()
            .to_canvas_bytes(),
        painted,
        "作り直した面へストロークが戻る"
    );
    // 無効にしても画素は保ち、合成からは消える
    d.set_channel_enabled(l, Channel::Height, false).unwrap();
    assert_eq!(
        d.composite_pixel(Channel::Height, 4, 4).unwrap(),
        Rgba8::TRANSPARENT
    );
    assert!(
        d.layer(l)
            .unwrap()
            .surface(Channel::Height)
            .unwrap()
            .tile_count()
            > 0
    );
    assert!(
        d.begin_stroke_in(l, Channel::Height, &brush).is_err(),
        "無効には描けない"
    );
    d.undo().unwrap();
    assert_ne!(
        d.composite_pixel(Channel::Height, 4, 4).unwrap(),
        Rgba8::TRANSPARENT
    );
}

// ───────── Normal（C# NormalChannelTests） ─────────

#[test]
fn partial_normal_coverage_is_a_renormalized_average_not_a_byte_lerp() {
    let mut d = Document::with_tile_size(8, 8, 8).unwrap();
    fill_layer(&mut d, "Below", Channel::Normal, |_, _| FLAT);
    let above = fill_layer(&mut d, "Above", Channel::Normal, |_, _| TILT_X);
    let below = d.layers()[0].id();
    d.set_pixel(below, 1, 1, FLAT).unwrap();
    d.set_pixel(above, 1, 1, TILT_X).unwrap();
    d.set_layer_opacity(above, 0.5, false).unwrap();
    assert_eq!(
        d.composite_pixel(Channel::Normal, 1, 1).unwrap(),
        Rgba8::new(218, 128, 218, 255)
    );
    let all = d.composite_channel(Channel::Normal, d.bounds()).unwrap();
    assert_eq!(&all[4 * 9..4 * 10], &[218, 128, 218, 255], "タイルの経路も");
    assert_eq!(
        d.composite_pixel(Channel::Color, 1, 1).unwrap(),
        Rgba8::new(192, 128, 192, 255),
        "同じ層の Color は色の式"
    );
    let mut single = Document::with_tile_size(8, 8, 8).unwrap();
    fill_layer(&mut single, "One", Channel::Normal, |_, _| {
        Rgba8::new(200, 128, 230, 255)
    });
    assert_eq!(
        single.composite_pixel(Channel::Normal, 0, 0).unwrap(),
        Rgba8::new(201, 128, 232, 255),
        "1 枚でも正規化する"
    );
    let mut t = Document::with_tile_size(8, 8, 8).unwrap();
    let l = fill_layer(&mut t, "L", Channel::Normal, |x, _| {
        if x == 3 {
            Rgba8::new(41, 53, 67, 0)
        } else {
            FLAT
        }
    });
    assert_eq!(
        t.layer(l).unwrap().pixel(Channel::Normal, 3, 0).unwrap(),
        Rgba8::new(41, 53, 67, 0)
    );
    assert_eq!(t.composite_pixel(Channel::Normal, 3, 0).unwrap().a, 0);
}

#[test]
fn output_flattens_onto_a_flat_normal_and_is_opaque() {
    let mut d = Document::with_tile_size(8, 8, 8).unwrap();
    let out = d.normal_output(BUDGET).unwrap();
    assert!(
        out.chunks(4).all(|p| p == [128, 128, 255, 255]),
        "塗っていない所は平らで不透明"
    );
    fill_layer(&mut d, "Half", Channel::Normal, |x, _| {
        if x == 2 {
            Rgba8::new(255, 128, 128, 128)
        } else {
            Rgba8::TRANSPARENT
        }
    });
    let out = d.normal_output(BUDGET).unwrap();
    assert_eq!(px(&out, 8, 2, 5), Rgba8::new(218, 128, 217, 255));
    assert_eq!(px(&out, 8, 3, 5), FLAT);
    assert!(!d.derives_normal());
}

#[test]
fn a_height_ramp_derives_the_expected_slope_and_y_direction() {
    use HeightEdgeMode::*;
    use NormalYDirection::*;
    let mut d = Document::with_tile_size(16, 16, 8).unwrap();
    fill_layer(&mut d, "Ramp", Channel::Height, |x, _| {
        let v = (2 * x) as u8;
        Rgba8::new(v, v, v, 255)
    });
    d.set_normal_settings(derive(127.5, Clamp, OpenGL), false)
        .unwrap();
    assert!(d.derives_normal());
    let out = d.normal_output(BUDGET).unwrap();
    assert_eq!(px(&out, 16, 5, 9), Rgba8::new(37, 128, 218, 255));
    assert_eq!(
        px(&out, 16, 0, 9),
        Rgba8::new(70, 128, 242, 255),
        "端のテクセルを繰り返す"
    );
    d.set_normal_settings(derive(127.5, Wrap, OpenGL), false)
        .unwrap();
    let out = d.normal_output(BUDGET).unwrap();
    assert_eq!(px(&out, 16, 5, 9), Rgba8::new(37, 128, 218, 255));
    assert_eq!(
        px(&out, 16, 0, 9),
        Rgba8::new(254, 128, 146, 255),
        "回り込み"
    );
    assert_eq!(
        d.derive_normal_from_height(Channel::Height, &d.normal_settings(), BUDGET)
            .unwrap(),
        out,
        "Normal の層が無ければ出力は作った法線"
    );
    let mut up = Document::with_tile_size(16, 16, 8).unwrap();
    fill_layer(&mut up, "RampY", Channel::Height, |_, y| {
        Rgba8::new((2 * y) as u8, 0, 0, 255)
    });
    up.set_normal_settings(derive(127.5, Clamp, OpenGL), false)
        .unwrap();
    assert_eq!(
        px(&up.normal_output(BUDGET).unwrap(), 16, 7, 7),
        Rgba8::new(128, 37, 218, 255)
    );
    assert_eq!(
        px(&up.normal_file_output(BUDGET).unwrap(), 16, 7, 7),
        Rgba8::new(128, 37, 218, 255)
    );
    up.set_normal_settings(derive(127.5, Clamp, DirectX), false)
        .unwrap();
    assert_eq!(
        px(&up.normal_file_output(BUDGET).unwrap(), 16, 7, 7),
        Rgba8::new(128, 218, 218, 255),
        "DirectX のファイルは緑を反転"
    );
    assert_eq!(
        px(&up.normal_output(BUDGET).unwrap(), 16, 7, 7),
        Rgba8::new(128, 37, 218, 255)
    );
    let mut alpha = Document::with_tile_size(16, 16, 8).unwrap();
    fill_layer(&mut alpha, "A", Channel::Height, |x, _| {
        Rgba8::new(255, 0, 0, (2 * x) as u8)
    });
    alpha
        .set_normal_settings(derive(127.5, Clamp, OpenGL), false)
        .unwrap();
    assert_eq!(
        px(&alpha.normal_output(BUDGET).unwrap(), 16, 5, 9),
        Rgba8::new(37, 128, 218, 255),
        "高さは R × A"
    );
}

#[test]
fn the_derived_normal_sits_under_the_painted_normal() {
    use HeightEdgeMode::*;
    use NormalYDirection::*;
    let mut d = Document::with_tile_size(16, 16, 8).unwrap();
    fill_layer(&mut d, "Ramp", Channel::Height, |x, _| {
        Rgba8::new((2 * x) as u8, 0, 0, 255)
    });
    let painted = fill_layer(&mut d, "Painted", Channel::Normal, |_, y| {
        if y >= 8 {
            TILT45
        } else {
            Rgba8::TRANSPARENT
        }
    });
    let out = d.normal_output(BUDGET).unwrap();
    assert_eq!(px(&out, 16, 5, 12), TILT45);
    assert_eq!(px(&out, 16, 5, 3), FLAT);
    d.set_normal_settings(derive(127.5, Clamp, OpenGL), false)
        .unwrap();
    let out = d.normal_output(BUDGET).unwrap();
    assert_eq!(px(&out, 16, 5, 3), Rgba8::new(37, 128, 218, 255));
    assert_eq!(
        px(&out, 16, 5, 12),
        FLAT,
        "−45° の土台に +45° の細部で打ち消す（RNM）"
    );
    let settings = derive(9.25, Wrap, OpenGL);
    d.set_normal_settings(settings, false).unwrap();
    let n = d.composite_channel(Channel::Normal, d.bounds()).unwrap();
    let h = d.composite_channel(Channel::Height, d.bounds()).unwrap();
    assert_eq!(
        d.normal_output(BUDGET).unwrap(),
        normal::output_from_composites(&n, Some(&h), 16, 16, &settings).unwrap()
    );
    d.set_layer_visible(painted, false).unwrap();
    assert_eq!(
        d.normal_output(BUDGET).unwrap(),
        d.derive_normal_from_height(Channel::Height, &settings, BUDGET)
            .unwrap()
    );
}

#[test]
fn odd_sizes_and_many_bands_agree_with_the_full_frame_reference() {
    let mut d = Document::with_tile_size(41, 35, 8).unwrap();
    let mut s = 7u64;
    let mut next = move || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 33) as u8
    };
    let hv: Vec<Rgba8> = (0..41 * 35)
        .map(|_| Rgba8::new(next(), 0, 0, next()))
        .collect();
    let nv: Vec<Rgba8> = (0..41 * 35)
        .map(|_| Rgba8::new(next(), next(), 128 + next() / 2, next()))
        .collect();
    fill_layer(&mut d, "H", Channel::Height, |x, y| {
        hv[(y * 41 + x) as usize]
    });
    fill_layer(&mut d, "N", Channel::Normal, |x, y| {
        nv[(y * 41 + x) as usize]
    });
    for edges in [HeightEdgeMode::Clamp, HeightEdgeMode::Wrap] {
        for strength in [0.0, 3.5, -40.0] {
            let settings = derive(strength, edges, NormalYDirection::OpenGL);
            d.set_normal_settings(settings, false).unwrap();
            let n = d.composite_channel(Channel::Normal, d.bounds()).unwrap();
            let h = d.composite_channel(Channel::Height, d.bounds()).unwrap();
            assert_eq!(
                d.normal_output(BUDGET).unwrap(),
                normal::output_from_composites(&n, Some(&h), 41, 35, &settings).unwrap(),
                "{edges:?} {strength}"
            );
        }
    }
}

#[test]
fn height_to_normal_refuses_other_channels_budgets_and_invalid_settings() {
    let mut d = Document::with_tile_size(64, 64, 16).unwrap();
    fill_layer(&mut d, "R", Channel::Roughness, |x, _| {
        Rgba8::new(x as u8, 0, 0, 255)
    });
    let s4 = derive(4.0, HeightEdgeMode::Clamp, NormalYDirection::OpenGL);
    for c in Channel::ALL {
        if c != Channel::Height {
            assert!(matches!(
                d.derive_normal_from_height(c, &s4, BUDGET),
                Err(CoreError::Unsupported(_))
            ));
        }
    }
    assert_eq!(
        d.derive_normal_from_height(Channel::from_index(42).unwrap(), &s4, BUDGET),
        Err(CoreError::ChannelNotFound)
    );
    assert_eq!(d.normal_working_bytes(), 4 * 64 * 64 + 4 * 64 * 16);
    d.set_normal_settings(s4, false).unwrap();
    assert_eq!(
        d.normal_working_bytes(),
        4 * 64 * 64 + 4 * 64 * 16 + 12 * 64 * 18,
        "高さの帯と上下の余白"
    );
    assert_eq!(
        d.normal_output(d.normal_working_bytes() - 1),
        Err(CoreError::WorkingBudgetExceeded)
    );
    assert_eq!(
        d.normal_file_output(1024),
        Err(CoreError::WorkingBudgetExceeded)
    );
    assert_eq!(
        d.derive_normal_from_height(Channel::Height, &s4, 1024),
        Err(CoreError::WorkingBudgetExceeded)
    );
    assert_eq!(
        d.normal_output(d.normal_working_bytes()).unwrap().len(),
        64 * 64 * 4,
        "ちょうどの予算なら足りる"
    );
    assert!(normal::output_from_composites(&[0; 16], None, 2, 2, &s4).is_err());
}

#[test]
fn normal_settings_are_one_undo_step_and_slider_drags_coalesce() {
    let mut d = Document::with_tile_size(16, 16, 8).unwrap();
    let layer = fill_layer(&mut d, "N", Channel::Normal, |_, _| FLAT);
    d.clear_history().unwrap();
    let (serial, revision) = (d.change_serial(), d.revision());
    let on = NormalSettings::DEFAULT.with_derive(true);
    d.set_normal_settings(on, false).unwrap();
    assert_eq!(d.undo_count(), 1);
    assert!(d.revision() > revision);
    assert!(
        d.changed_tiles(Channel::Normal, serial).unwrap().is_empty(),
        "層の合成は変えない（出力だけ）"
    );
    d.set_normal_settings(on, false).unwrap();
    assert_eq!(d.undo_count(), 1, "同じ値は段を足さない");
    for i in 5..=9 {
        let s = d.normal_settings().with_strength(i as f64).unwrap();
        d.set_normal_settings(s, true).unwrap();
    }
    assert_eq!(d.undo_count(), 2);
    assert_eq!(d.normal_settings().strength(), 9.0);
    d.end_coalescing();
    let s = d
        .normal_settings()
        .with_file_direction(NormalYDirection::DirectX)
        .with_edges(HeightEdgeMode::Wrap);
    d.set_normal_settings(s, false).unwrap();
    d.undo().unwrap();
    assert_eq!(
        d.normal_settings(),
        NormalSettings::new(true, 9.0, HeightEdgeMode::Clamp, NormalYDirection::OpenGL).unwrap()
    );
    d.undo().unwrap();
    assert_eq!(d.normal_settings().strength(), 4.0, "ドラッグの前へ");
    d.undo().unwrap();
    assert_eq!(d.normal_settings(), NormalSettings::DEFAULT);
    d.redo().unwrap();
    d.redo().unwrap();
    d.redo().unwrap();
    assert_eq!(
        d.normal_settings(),
        NormalSettings::new(true, 9.0, HeightEdgeMode::Wrap, NormalYDirection::DirectX).unwrap()
    );
    let brush = BrushSettings {
        color: TILT_X,
        ..BrushSettings::default()
    };
    let s = d.begin_stroke_in(layer, Channel::Normal, &brush).unwrap();
    assert_eq!(
        d.set_normal_settings(NormalSettings::DEFAULT, false),
        Err(CoreError::StrokeActive)
    );
    d.cancel_stroke(s);
    assert_eq!(d.normal_settings().edges(), HeightEdgeMode::Wrap);
}

// ───────── ユーザーチャンネル ─────────

fn user(name: &str, kind: ChannelKind) -> ChannelInfo {
    ChannelInfo {
        name: name.to_string(),
        kind,
        color_space: if kind == ChannelKind::Color {
            ColorSpace::Srgb
        } else {
            ColorSpace::Linear
        },
        default: Rgba8::new(0, 0, 0, 255),
    }
}

#[test]
fn the_standard_channels_carry_unitys_numbers_and_kinds() {
    let d = Document::new(8, 8).unwrap();
    assert_eq!(d.channels(), Channel::ALL.to_vec());
    for (i, c) in Channel::ALL.iter().enumerate() {
        assert_eq!(c.index(), i);
        let info = d.channel_info(*c).unwrap();
        assert_eq!(info.name, format!("{c:?}"));
    }
    assert_eq!(
        d.channel_info(Channel::Normal).unwrap().kind,
        ChannelKind::Normal
    );
    assert_eq!(
        d.channel_info(Channel::Emission).unwrap().kind,
        ChannelKind::Color
    );
    assert_eq!(
        d.channel_info(Channel::Emission).unwrap().color_space,
        ColorSpace::Srgb
    );
    assert_eq!(
        d.channel_info(Channel::Height).unwrap().kind,
        ChannelKind::Scalar
    );
    assert_eq!(
        d.channel_info(Channel::Normal).unwrap().default,
        Rgba8::new(128, 128, 255, 255)
    );
    assert_eq!(
        Channel::from_standard_name("Metallic"),
        Some(Channel::Metallic)
    );
    assert_eq!(format!("{:?}", Channel::from_index(9).unwrap()), "User(9)");
}

/// 同じ中身・同じ設定の層を、標準のチャンネルとユーザーチャンネルの両方に描いた文書。
fn twin(user_kind: ChannelKind, standard: Channel) -> (Document, Channel) {
    let mut d = Document::with_tile_size(37, 29, 8).unwrap();
    let u = d.add_channel(user("Mask A", user_kind)).unwrap();
    let mut s = 99u64;
    let mut next = move || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 33) as u32
    };
    let mut ids = Vec::new();
    for l in 0..4 {
        let id = d.add_layer(&format!("L{l}")).unwrap();
        for _ in 0..300 {
            let r = next();
            let c = Rgba8::new(
                r as u8,
                (r >> 8) as u8,
                (r >> 16) as u8,
                (next() >> 3) as u8,
            );
            let (x, y) = (next() % 37, next() % 29);
            d.set_channel_pixel(id, standard, x, y, c).unwrap();
            d.set_channel_pixel(id, u, x, y, c).unwrap();
        }
        d.set_layer_blend_mode(
            id,
            [
                BlendMode::Normal,
                BlendMode::Overlay,
                BlendMode::Screen,
                BlendMode::Hue,
            ][l],
        )
        .unwrap();
        d.set_layer_opacity(id, 0.5 + 0.1 * l as f64, false)
            .unwrap();
        ids.push(id);
    }
    d.set_layer_clipping(ids[2], true).unwrap();
    d.add_layer_mask(ids[1]).unwrap();
    for i in 0..100 {
        d.set_mask_pixel(ids[1], (i * 7) % 37, (i * 3) % 29, (i * 41 % 256) as u8)
            .unwrap();
    }
    let g = d.group_layers(&[ids[1], ids[2]], "G").unwrap();
    d.set_layer_opacity(g, 0.7, false).unwrap();
    d.add_adjustment_layer(
        "hsl",
        AdjustmentSettings::hue_saturation(40.0, 0.3, 0.1).unwrap(),
        None,
        None,
    )
    .unwrap();
    d.add_adjustment_layer(
        "lv",
        AdjustmentSettings::levels(0.1, 0.9, 1.5, 0.0, 1.0).unwrap(),
        None,
        None,
    )
    .unwrap();
    (d, u)
}

#[test]
fn a_user_channel_composites_like_the_standard_channel_of_its_kind() {
    for (kind, standard) in [
        (ChannelKind::Color, Channel::Color),
        (ChannelKind::Color, Channel::Emission),
        (ChannelKind::Scalar, Channel::Roughness),
        (ChannelKind::Normal, Channel::Normal),
    ] {
        let (d, u) = twin(kind, standard);
        assert_eq!(
            d.composite_channel(u, d.bounds()).unwrap(),
            d.composite_channel(standard, d.bounds()).unwrap(),
            "{kind:?} {standard:?}"
        );
        assert_eq!(
            d.composite_pixel(u, 5, 7).unwrap(),
            d.composite_pixel(standard, 5, 7).unwrap()
        );
    }
    // 色相/彩度は色のユーザーチャンネルにだけ効く
    let (d, u) = twin(ChannelKind::Scalar, Channel::Roughness);
    let hsl = d.layers().iter().find(|l| l.name() == "hsl").unwrap();
    assert!(!hsl.is_channel_enabled(u));
    assert!(d
        .layers()
        .iter()
        .find(|l| l.name() == "lv")
        .unwrap()
        .is_channel_enabled(u));
}

#[test]
fn user_channels_can_be_added_changed_and_removed_with_undo() {
    let (mut d, u) = twin(ChannelKind::Color, Channel::Color);
    d.clear_history().unwrap();
    assert_eq!(u.index(), 6);
    assert!(
        d.add_channel(user("Mask A", ChannelKind::Scalar)).is_err(),
        "名前は重ならない"
    );
    assert!(d.add_channel(user("", ChannelKind::Scalar)).is_err());
    assert!(
        d.remove_channel(Channel::Roughness).is_err(),
        "標準は消せない"
    );
    assert!(d
        .set_channel_info(Channel::Color, user("X", ChannelKind::Color))
        .is_err());
    // 色相/彩度の層が有効なまま種類を変えるのは断る（その調整が使えない種類になる）
    let hsl = d.layers().iter().find(|l| l.name() == "hsl").unwrap().id();
    assert!(d
        .set_channel_info(u, user("Mask A", ChannelKind::Normal))
        .is_err());
    d.set_channel_enabled(hsl, u, false).unwrap();
    d.clear_history().unwrap();
    let before = d.composite_channel(u, d.bounds()).unwrap();
    let bytes = d.allocated_bytes();
    // 種類を変えると合成の式が変わる（色 → 法線）
    let since = d.change_serial();
    d.set_channel_info(u, user("Mask A", ChannelKind::Normal))
        .unwrap();
    assert_eq!(d.composite_channel(u, d.bounds()).unwrap(), {
        let (n, nu) = twin(ChannelKind::Normal, Channel::Color);
        n.composite_channel(nu, n.bounds()).unwrap()
    });
    assert!(!d.changed_tiles(u, since).unwrap().is_empty());
    d.undo().unwrap();
    assert_eq!(d.composite_channel(u, d.bounds()).unwrap(), before);
    // 消すと層の中身も消え、1 回の Undo で戻る
    d.remove_channel(u).unwrap();
    assert_eq!(
        d.composite_channel(u, d.bounds()),
        Err(CoreError::ChannelNotFound)
    );
    assert!(d
        .layers()
        .iter()
        .all(|l| l.surface(u).is_none() && !l.is_channel_enabled(u)));
    assert!(d.allocated_bytes() < bytes);
    let l0 = d.layers()[0].id();
    assert_eq!(
        d.set_channel_pixel(l0, u, 0, 0, Rgba8::new(1, 2, 3, 4)),
        Err(CoreError::ChannelNotFound)
    );
    d.undo().unwrap();
    assert_eq!(d.composite_channel(u, d.bounds()).unwrap(), before);
    assert_eq!(d.allocated_bytes(), bytes);
    // 画素の予算が足りなければ戻すのを断る（何も変えない）
    d.redo().unwrap();
    d.set_source_budget_bytes(d.allocated_bytes()).unwrap();
    assert_eq!(d.undo(), Err(CoreError::SourceBudgetExceeded));
    assert!(d.channel_info(u).is_none());
    // 64 まで
    let mut e = Document::new(8, 8).unwrap();
    for i in 0..(Channel::MAX - Channel::STANDARD_COUNT) {
        e.add_channel(user(&format!("U{i}"), ChannelKind::Scalar))
            .unwrap();
    }
    assert!(e.add_channel(user("over", ChannelKind::Scalar)).is_err());
    assert_eq!(e.channels().len(), Channel::MAX);
    // 消した番号はまた使える
    e.remove_channel(Channel::from_index(10).unwrap()).unwrap();
    assert_eq!(
        e.add_channel(user("again", ChannelKind::Color)).unwrap(),
        Channel::from_index(10).unwrap()
    );
}

#[test]
fn fill_and_strokes_reach_user_channels() {
    let mut d = Document::with_tile_size(16, 16, 8).unwrap();
    let u = d.add_channel(user("Sheen", ChannelKind::Scalar)).unwrap();
    let f = d
        .add_fill_layer("F", &[(u, Rgba8::new(90, 90, 90, 255))], None)
        .unwrap();
    assert_eq!(
        d.composite_pixel(u, 15, 15).unwrap(),
        Rgba8::new(90, 90, 90, 255)
    );
    let l = d.add_layer("P").unwrap();
    let brush = BrushSettings {
        radius: 2.0,
        hardness: 1.0,
        color: Rgba8::new(255, 255, 255, 255),
        pressure_size: false,
        pressure_opacity: false,
        ..BrushSettings::default()
    };
    let since = d.change_serial();
    let mut s = d.begin_stroke_in(l, u, &brush).unwrap();
    s.add_point(&mut d, 4.5, 4.5, 1.0, Default::default())
        .unwrap();
    d.end_stroke(s).unwrap();
    assert_eq!(
        d.composite_pixel(u, 4, 4).unwrap(),
        Rgba8::new(255, 255, 255, 255)
    );
    assert_eq!(
        d.changed_tiles(u, since).unwrap(),
        vec![TileCoord::new(0, 0)]
    );
    assert!(d.changed_tiles(Channel::Color, since).unwrap().is_empty());
    d.undo().unwrap();
    assert_eq!(
        d.composite_pixel(u, 4, 4).unwrap(),
        Rgba8::new(90, 90, 90, 255)
    );
    let _ = f;
}

#[test]
fn loaders_can_put_user_channels_back_at_their_saved_numbers() {
    let mut d = Document::new(8, 8).unwrap();
    let nine = Channel::from_index(9).unwrap();
    d.insert_channel_for_load(nine, user("Saved", ChannelKind::Scalar))
        .unwrap();
    assert_eq!(d.channels().last(), Some(&nine));
    assert!(d
        .insert_channel_for_load(nine, user("Other", ChannelKind::Scalar))
        .is_err());
    assert!(d
        .insert_channel_for_load(Channel::Height, user("H", ChannelKind::Scalar))
        .is_err());
    assert!(d
        .insert_channel_for_load(
            Channel::from_index(10).unwrap(),
            user("Saved", ChannelKind::Color)
        )
        .is_err());
    // 足すときは空いている一番小さい番号から
    assert_eq!(
        d.add_channel(user("New", ChannelKind::Color)).unwrap(),
        Channel::from_index(6).unwrap()
    );
}

#[test]
fn a_coalesced_fill_drag_redoes_to_the_enabled_state_it_ended_in() {
    // 値を置いて（有効になる）から消す 2 回をまとめても、Redo の後の有効は最後と同じ
    let mut d = Document::with_tile_size(8, 8, 8).unwrap();
    let f = d.add_fill_layer("F", &[], None).unwrap();
    d.clear_history().unwrap();
    d.set_fill_value(f, Channel::Height, Some(Rgba8::new(9, 9, 9, 255)), true)
        .unwrap();
    d.set_fill_value(f, Channel::Height, None, true).unwrap();
    assert_eq!(d.undo_count(), 1);
    assert!(d.layer(f).unwrap().is_channel_enabled(Channel::Height));
    d.undo().unwrap();
    assert!(!d.layer(f).unwrap().is_channel_enabled(Channel::Height));
    d.redo().unwrap();
    assert!(d.layer(f).unwrap().is_channel_enabled(Channel::Height));
    assert_eq!(d.layer(f).unwrap().fill_value(Channel::Height), None);
}

#[test]
fn a_refused_direct_write_leaves_no_new_surface() {
    let mut d = Document::with_tile_size(8, 8, 8).unwrap();
    let l = d.add_layer("L").unwrap();
    d.set_pixel(l, 0, 0, Rgba8::new(1, 2, 3, 255)).unwrap();
    assert!(d
        .set_channel_pixel(l, Channel::Height, 99, 0, Rgba8::new(1, 1, 1, 255))
        .is_err());
    assert!(d.layer(l).unwrap().surface(Channel::Height).is_none());
    assert!(!d.layer(l).unwrap().is_channel_enabled(Channel::Height));
    assert!(d
        .import_tile(l, Channel::Roughness, TileCoord::new(0, 0), &[0; 4])
        .is_err());
    assert!(d.layer(l).unwrap().surface(Channel::Roughness).is_none());
}
