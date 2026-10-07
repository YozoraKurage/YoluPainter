//! 効果のつなぎ目の試験の道具: 効果をひと通り持つ文書（`rig`）と 2D のパス。人工データだけ。
#![allow(dead_code)]
use super::attach_support::*;
use yolu_core::generator::{self, anchor::ReadMode, Settings};
use yolu_core::paths::{render_canvas, CanvasPath, CanvasPoint, Options, PathBrush};
use yolu_core::{
    AnchorPlacement, BrushSettings, Channel, Document, EffectSettings, FilterId, FilterSpec,
    FilterTarget, LayerId, Rgba8,
};

pub fn path_points(shift: f64) -> CanvasPath {
    CanvasPath {
        style: Default::default(),
        id: 0x77,
        channel: Channel::Color,
        brush: PathBrush(BrushSettings {
            radius: 3.0,
            spacing: 0.17,
            color: Rgba8::new(201, 37, 89, 219),
            ..BrushSettings::default()
        }),
        points: vec![
            CanvasPoint::new(4.5 + shift, 7.25, 0.3).unwrap(),
            CanvasPoint::new(29.5, 21.5 - shift, 0.9).unwrap(),
            CanvasPoint::new(37.25, 5.75, 0.55).unwrap(),
        ],
        material: None,
    }
}

pub fn rendered(path: &CanvasPath) -> Vec<(Channel, yolu_core::Surface)> {
    render_canvas(
        path,
        &Options {
            width: W,
            height: H,
            tile_size: 8,
            ..Options::default()
        },
    )
    .unwrap()
    .channels
}

/// 効果をひと通り持つ文書: 土台に Anchor とぼかし、中に Anchor を読む Generator、上にマスクとマスクのぼかし、塗りつぶしに
/// 画像・グラデーション、パスの層。
pub struct Rig {
    pub doc: Document,
    pub base: LayerId,
    pub mid: LayerId,
    pub top: LayerId,
    pub fill: LayerId,
    /// 画像もデカールも無い塗りつぶし。
    pub plain_fill: LayerId,
    pub path_layer: LayerId,
    pub blur: FilterId,
    pub mask_blur: FilterId,
    pub reader: FilterId,
    pub anchor: yolu_core::AnchorId,
}

pub fn rig() -> Rig {
    let (mut doc, l) = world();
    let (base, mid, top, fill) = (l[0], l[1], l[2], l[3]);
    let blur = doc
        .add_filter(
            base,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(3)).channels(&[Channel::Color]),
        )
        .unwrap();
    let anchor = doc
        .add_anchor(base, AnchorPlacement::Layer, Some("土台"), None)
        .unwrap();
    let mut g = Settings::new(generator::Kind::Anchor);
    g.blend = generator::Blend::Replace;
    let reader = doc
        .add_filter(
            mid,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(g)).channels(&[Channel::Height]),
        )
        .unwrap();
    doc.set_generator_anchor(
        mid,
        reader,
        Some(anchor),
        Channel::Height,
        ReadMode::Value,
        false,
    )
    .unwrap();
    doc.add_layer_mask(top).unwrap();
    let mask_blur = doc
        .add_filter(
            top,
            FilterTarget::Mask,
            FilterSpec::new(EffectSettings::blur(2)),
        )
        .unwrap();
    doc.set_fill_image(fill, Channel::Color, Some(image(0)))
        .unwrap();
    let plain_fill = doc
        .add_fill_layer(
            "素の塗り",
            &[(Channel::Color, Rgba8::new(5, 6, 7, 255))],
            None,
        )
        .unwrap();
    let path_layer = doc.add_layer("パス").unwrap();
    let path = path_points(0.0);
    doc.set_canvas_path(path_layer, path).unwrap();
    doc.clear_history().unwrap();
    Rig {
        doc,
        base,
        mid,
        top,
        fill,
        plain_fill,
        path_layer,
        blur,
        mask_blur,
        reader,
        anchor,
    }
}
