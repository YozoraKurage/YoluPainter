//! 画像の段（Generator の種類 70）: 塗りつぶしレイヤーの画像と同じ投影で画像を読み、色のチャンネルでは画素の色、マスク・スカラーでは選んだ成分を
//! 出す。画像が無い・読めない・投影のマップが無いときは入力のまま通して理由を言い、1 回の Undo で戻る。
#![allow(clippy::chunks_exact_to_as_chunks)]
use crate::attach_support;
use attach_support::*;
use yolu_core::fill_image::{
    Conversion, FillInput, FillSampler, ImageMipChain, Projection, ProjectionMode, Wrap,
};
use yolu_core::generator::{
    anchor, Blend, BoundGenerator, Generated, ImageComponent, Inactive, Kind, MapKind, Settings,
};
use yolu_core::{
    Channel, Document, EffectInputs, EffectSettings, FilterSpec, FilterTarget, ImageColorSpace,
    ImageInput, InactiveReason, LayerId, Rgba8,
};

const IW: u32 = 8;
const IH: u32 = 6;

/// 8×6 の画像（成分がそれぞれ違い、アルファも 0・90・255 がある）。
fn pixels() -> Vec<u8> {
    let mut p = Vec::new();
    for y in 0..IH {
        for x in 0..IW {
            p.extend([
                (x * 31 + 7) as u8,
                (y * 40 + 13) as u8,
                ((x * 7 + y * 11) * 5 % 256) as u8,
                match (x + y) % 5 {
                    0 => 90,
                    1 => 0,
                    _ => 255,
                },
            ]);
        }
    }
    p
}

fn image_settings(image: u128, component: ImageComponent, invert: bool) -> Settings {
    let mut g = Settings::new(Kind::Image);
    g.image.image = image;
    g.image.component = component;
    g.invert = invert;
    g.blend = Blend::Replace;
    g
}

/// 文書と同じ大きさの画像を UV で読むと、画素の中心が画像の画素の中心に重なり、値は画像の画素そのもの。成分・反転・色の対象。
#[test]
fn uv_reads_the_texels_and_the_chosen_component() {
    let data = pixels();
    let chain =
        ImageMipChain::build(&data, IW, IH, Conversion::None, false, 1 << 20, None).unwrap();
    let sampler = FillSampler::bind(FillInput {
        width: IW,
        height: IH,
        image: Some(&chain),
        ..FillInput::default()
    })
    .unwrap();
    let mut checked = 0;
    for component in ImageComponent::ALL {
        for invert in [false, true] {
            let g = image_settings(1, component, invert);
            let b = BoundGenerator::bind(&g, &[], None, (IW, IH), Err(anchor::Issue::NotChosen))
                .unwrap();
            // 画像を渡すまでは「画像が無い」で入力のまま
            assert_eq!(b.inactive(), Some(&Inactive::MissingImage));
            assert_eq!(b.sample(0, 0, true), None);
            let b = b.with_image(&sampler);
            assert_eq!(b.inactive(), None);
            for y in 0..IH {
                let mut row = vec![None; IW as usize];
                b.sample_row(0, y, true, &mut row);
                let mut colors = vec![None; IW as usize];
                b.sample_row(0, y, false, &mut colors);
                for x in 0..IW {
                    let i = ((y * IW + x) * 4) as usize;
                    let p = &data[i..i + 4];
                    let u = |v: u8| f64::from(v) / 255.0;
                    let base = match component {
                        ImageComponent::Red => u(p[0]),
                        ImageComponent::Green => u(p[1]),
                        ImageComponent::Blue => u(p[2]),
                        ImageComponent::Alpha => u(p[3]),
                        ImageComponent::Luminance => {
                            0.2126 * u(p[0]) + 0.7152 * u(p[1]) + 0.0722 * u(p[2])
                        }
                    };
                    let want = if invert { 1.0 - base } else { base };
                    let got = b.sample(x, y, true);
                    assert_eq!(
                        got,
                        Some(Generated::Scalar(want)),
                        "{component:?} 反転 {invert} ({x}, {y})"
                    );
                    assert_eq!(row[x as usize], got, "行の評価 ({x}, {y})");
                    let rgb = if invert {
                        [255 - p[0], 255 - p[1], 255 - p[2], p[3]]
                    } else {
                        [p[0], p[1], p[2], p[3]]
                    };
                    assert_eq!(b.sample(x, y, false), Some(Generated::Mapped(rgb)));
                    assert_eq!(colors[x as usize], Some(Generated::Mapped(rgb)));
                    checked += 1;
                }
            }
            // 範囲の外は値なし
            assert_eq!(b.sample(IW, 0, true), None);
            assert_eq!(b.sample(0, IH, false), None);
        }
    }
    assert_eq!(checked, 5 * 2 * (IW * IH) as usize);
}

/// 画像を選んでいない段は、ほかの理由より先に「選んでいない」。
#[test]
fn an_unchosen_image_is_named_first() {
    let g = image_settings(0, ImageComponent::Red, false);
    let b = BoundGenerator::bind(&g, &[], None, (IW, IH), Err(anchor::Issue::NotChosen)).unwrap();
    assert_eq!(b.inactive(), Some(&Inactive::NoImage));
    // 位置を読む投影でマップもルートも無くても
    let mut g = g.clone();
    g.image.projection.mode = ProjectionMode::Triplanar;
    let b = BoundGenerator::bind(&g, &[], None, (IW, IH), Err(anchor::Issue::NotChosen)).unwrap();
    assert_eq!(b.inactive(), Some(&Inactive::NoImage));
    // 選んでいれば、マップが先
    g.image.image = 5;
    let b = BoundGenerator::bind(&g, &[], None, (IW, IH), Err(anchor::Issue::NotChosen)).unwrap();
    assert_eq!(b.inactive(), Some(&Inactive::MissingMap(MapKind::Position)));
}

/// 段の設定の検査: デカールの投影・範囲外の投影・重ねるノイズ・ランプは断り、ほかの種類は画像の設定を持てない。
#[test]
fn the_settings_check_refuses_what_the_stage_cannot_do() {
    let ok = image_settings(1, ImageComponent::Alpha, false);
    assert!(ok.validate().is_ok());
    let mut decal = ok.clone();
    decal.image.projection.mode = ProjectionMode::Decal;
    assert!(decal.validate().is_err());
    let mut tiles = ok.clone();
    tiles.image.projection.tiles = [0.0, 1.0];
    assert!(tiles.validate().is_err());
    let mut noise = ok.clone();
    noise.noise_amount = 0.3;
    assert!(noise.validate().is_err());
    let mut ramp = ok.clone();
    ramp.ramp = Some(yolu_core::generator::Ramp::default());
    assert!(ramp.validate().is_err());
    let mut other = Settings::new(Kind::EdgeWear);
    other.image.image = 1;
    assert!(other.validate().is_err());
    assert_eq!(Kind::from_index(70), Some(Kind::Image));
    assert!(Kind::Image.is_rust_only() && !Kind::Image.is_procedural());
    // 投影で読むマップが決まる（UV は読まない）。ピンは持たない
    let mut g = ok.clone();
    assert!(g.used_maps().is_empty() && g.candidate_maps().is_empty());
    g.image.projection.mode = ProjectionMode::Triplanar;
    assert_eq!(g.used_maps(), [MapKind::Position, MapKind::WorldNormal]);
    g.image.projection.mode = ProjectionMode::Spherical;
    assert_eq!(g.used_maps(), [MapKind::Position]);
}

/// 塗りつぶしレイヤー（白）の上へ、同じ画像・同じ投影の塗りつぶしの画像を置いたレイヤーと、白いレイヤーに画像の段（置き換え）を置いたレイヤーは、
/// 全部の投影（UV・トライプラナー・平面・球・円柱）と外側（繰り返す・透明）・タイル・回転で同じ合成になる（色の画像とリニアの画像）。
/// 比べるのは色のチャンネル。マスク・スカラーの輝度は補間の後に求めるので、塗りつぶしレイヤーのスカラー（元の画素で丸めてから補間）とは比べない。
#[test]
fn every_projection_matches_a_fill_layer_with_the_same_image() {
    let white = Rgba8::new(255, 255, 255, 255);
    let mut compared = 0;
    for mode in [
        ProjectionMode::Uv,
        ProjectionMode::Triplanar,
        ProjectionMode::Planar,
        ProjectionMode::Spherical,
        ProjectionMode::Cylindrical,
    ] {
        for wrap in [Wrap::Repeat, Wrap::None] {
            for n in [0, 1] {
                let mut projection = Projection {
                    mode,
                    wrap,
                    tiles: [1.7, 0.8],
                    offset: [0.1, -0.25],
                    rotation: 23.0,
                    ..Projection::default()
                };
                projection.placement.center = [0.2, 0.1, -0.3];
                projection.placement.rotation = [10.0, -20.0, 35.0];
                projection.placement.size = [1.6, 2.2, 1.3];
                let make = |stage: bool| {
                    let mut doc = Document::with_tile_size(W, H, 8).unwrap();
                    doc.set_effect_inputs(inputs(0)).unwrap();
                    let fill = doc
                        .add_fill_layer("塗り", &[(Channel::Color, white)], None)
                        .unwrap();
                    if stage {
                        let mut g = image_settings(image(n).0, ImageComponent::Red, false);
                        g.image.projection = projection;
                        doc.add_filter(
                            fill,
                            FilterTarget::Content,
                            FilterSpec::new(EffectSettings::generator(g))
                                .channels(&[Channel::Color]),
                        )
                        .unwrap();
                    } else {
                        doc.set_fill_image(fill, Channel::Color, Some(image(n)))
                            .unwrap();
                        doc.set_fill_projection(fill, projection, false).unwrap();
                    }
                    whole(&doc, Channel::Color)
                };
                let reference = make(false);
                assert_eq!(
                    make(true),
                    reference,
                    "{mode:?} {wrap:?} 画像 {n}: 塗りつぶしの画像と違う"
                );
                // 画像が効いている（白のままではない）
                assert!(
                    reference.chunks_exact(4).any(|p| p != [255, 255, 255, 255]),
                    "{mode:?} {wrap:?}"
                );
                compared += 1;
            }
        }
    }
    assert_eq!(compared, 20);
}

fn opaque_layer(doc: &mut Document, channel: Channel) -> LayerId {
    let layer = doc.add_layer("下").unwrap();
    doc.set_channel_enabled(layer, channel, true).unwrap();
    for y in 0..doc.height() {
        for x in 0..doc.width() {
            doc.set_channel_pixel(layer, channel, x, y, Rgba8::new(200, 200, 200, 255))
                .unwrap();
        }
    }
    layer
}

fn image_inputs() -> EffectInputs {
    EffectInputs::new().with_image(
        image(0),
        ImageInput::new(IW, IH, pixels(), ImageColorSpace::Srgb).unwrap(),
    )
}

/// スカラーのチャンネルとマスク: 選んだ成分の値（UV で文書と同じ大きさの画像なら画素そのもの）。マスクではアルファの成分が見える度合い。
#[test]
fn scalar_channels_and_masks_take_the_component() {
    let data = pixels();
    // スカラー: 置き換えで、灰色は成分の値（R・G・B が同じ）
    let mut doc = Document::with_tile_size(IW, IH, 8).unwrap();
    doc.set_effect_inputs(image_inputs()).unwrap();
    let layer = opaque_layer(&mut doc, Channel::Roughness);
    doc.add_filter(
        layer,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::generator(image_settings(
            image(0).0,
            ImageComponent::Green,
            true,
        )))
        .channels(&[Channel::Roughness]),
    )
    .unwrap();
    let out = whole(&doc, Channel::Roughness);
    for (p, src) in out.chunks_exact(4).zip(data.chunks_exact(4)) {
        assert_eq!(p[..3], [255 - src[1]; 3], "{src:?}");
    }
    // マスク: アルファの成分を置き換えで → レイヤーの見える度合いは画像のアルファ
    let mut doc = Document::with_tile_size(IW, IH, 8).unwrap();
    doc.set_effect_inputs(image_inputs()).unwrap();
    let layer = opaque_layer(&mut doc, Channel::Color);
    doc.add_layer_mask(layer).unwrap();
    doc.add_filter(
        layer,
        FilterTarget::Mask,
        FilterSpec::new(EffectSettings::generator(image_settings(
            image(0).0,
            ImageComponent::Alpha,
            false,
        ))),
    )
    .unwrap();
    let out = whole(&doc, Channel::Color);
    for (p, src) in out.chunks_exact(4).zip(data.chunks_exact(4)) {
        assert_eq!(p[3], src[3], "{src:?}");
    }
}

/// 画像を選んでいない・入力に無い・投影のマップが無いときは入力のまま通し、理由を言う。入力を渡すと効き、Undo 1 回で段が消える。
#[test]
fn a_missing_image_passes_the_input_and_says_why() {
    let mut doc = Document::with_tile_size(IW, IH, 8).unwrap();
    let layer = opaque_layer(&mut doc, Channel::Color);
    let before = whole(&doc, Channel::Color);
    let count = doc.undo_count();
    let id = doc
        .add_filter(
            layer,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(image_settings(
                0,
                ImageComponent::Red,
                false,
            )))
            .channels(&[Channel::Color]),
        )
        .unwrap();
    assert_eq!(doc.undo_count(), count + 1);
    assert_eq!(whole(&doc, Channel::Color), before);
    assert_eq!(
        doc.generator_inactive(layer, id).unwrap(),
        Some(InactiveReason::Generator(Inactive::NoImage))
    );
    // 選んだが入力に無い
    doc.set_filter_settings(
        layer,
        id,
        EffectSettings::generator(image_settings(image(0).0, ImageComponent::Red, false)),
        false,
    )
    .unwrap();
    assert_eq!(whole(&doc, Channel::Color), before);
    assert_eq!(
        doc.generator_inactive(layer, id).unwrap(),
        Some(InactiveReason::Generator(Inactive::MissingImage))
    );
    let listed = doc.inactive_effect_list();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].to_string().contains("画像"), "{}", listed[0]);
    assert_eq!(
        doc.layer(layer)
            .unwrap()
            .generator_images()
            .collect::<Vec<_>>(),
        [image(0)]
    );
    // 入力を渡すと効く
    doc.set_effect_inputs(image_inputs()).unwrap();
    assert_eq!(doc.generator_inactive(layer, id).unwrap(), None);
    let applied = whole(&doc, Channel::Color);
    assert_ne!(applied, before);
    // トライプラナーにすると、位置のマップが無いので入力のまま
    let mut g = image_settings(image(0).0, ImageComponent::Red, false);
    g.image.projection.mode = ProjectionMode::Triplanar;
    doc.set_filter_settings(layer, id, EffectSettings::generator(g), false)
        .unwrap();
    assert_eq!(
        doc.generator_inactive(layer, id).unwrap(),
        Some(InactiveReason::Generator(Inactive::MissingMap(
            MapKind::Position
        )))
    );
    assert_eq!(whole(&doc, Channel::Color), before);
    // 取り消すと UV に戻って効き、もう 2 回で選ぶ前・段の無い文書
    assert!(doc.undo().unwrap());
    assert_eq!(whole(&doc, Channel::Color), applied);
    assert!(doc.undo().unwrap());
    assert!(doc.undo().unwrap());
    assert!(doc.layer(layer).unwrap().filters().is_empty());
    assert_eq!(whole(&doc, Channel::Color), before);
}
