#![allow(dead_code)]
use yolu_core::{generator::*, Rgba8};
pub const KEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub fn ramp() -> Ramp {
    Ramp::new(
        vec![
            ColorStop {
                position: 0.1,
                color: Rgba8::new(231, 19, 47, 0),
                midpoint: 0.17,
            },
            ColorStop {
                position: 0.57,
                color: Rgba8::new(11, 207, 59, 255),
                midpoint: 0.81,
            },
            ColorStop {
                position: 0.94,
                color: Rgba8::new(29, 43, 249, 64),
                midpoint: 0.5,
            },
        ],
        vec![
            OpacityStop {
                position: 0.,
                opacity: 0.9,
                midpoint: 0.24,
            },
            OpacityStop {
                position: 0.63,
                opacity: 0.,
                midpoint: 0.73,
            },
            OpacityStop {
                position: 1.,
                opacity: 0.7,
                midpoint: 0.5,
            },
        ],
        Some(vec![
            CurvePoint { x: 0., y: 0.1 },
            CurvePoint { x: 0.23, y: 0.9 },
            CurvePoint { x: 0.61, y: 0.2 },
            CurvePoint { x: 1., y: 1. },
        ]),
    )
    .unwrap()
}
pub const KINDS: [Kind; 8] = [
    Kind::EdgeWear,
    Kind::Dirt,
    Kind::PositionGradient,
    Kind::Thickness,
    Kind::Direction,
    Kind::ShapeGradient,
    Kind::IdColor,
    Kind::Anchor,
];
pub fn settings(kind: Kind, v: usize) -> Settings {
    let mut s = Settings::new(kind);
    s.low = 0.07;
    s.high = 0.89;
    s.softness = 0.37;
    s.invert = v % 2 == 1;
    s.noise_amount = if v.is_multiple_of(3) { 0. } else { 0.73 };
    s.noise_scale = 0.071;
    s.noise_seed = if v.is_multiple_of(2) { i32::MIN } else { 19381 };
    s.noise_space = if v % 3 == 1 {
        NoiseSpace::Uv
    } else {
        NoiseSpace::Model
    };
    s.blend = [
        Blend::Multiply,
        Blend::Replace,
        Blend::Screen,
        Blend::Max,
        Blend::Min,
        Blend::Add,
        Blend::Subtract,
    ][v % 7];
    if kind == Kind::Dirt {
        s.balance = [0., 0.3, 1.][v % 3];
    }
    if kind == Kind::PositionGradient {
        s.axis = v % 3;
    }
    if kind == Kind::Direction {
        s.direction = [0.31, -0.71, 0.19];
        s.use_bent_normal = v % 2 == 1;
    }
    if kind == Kind::ShapeGradient {
        s.volume = Volume {
            shape: [Shape::Box, Shape::Sphere, Shape::Plane][v % 3],
            center: [0.2, -0.1, 0.4],
            rotation: [17., -31., 43.],
            size: [2.3, 3.1, 4.7],
            falloff: if v.is_multiple_of(2) { 0. } else { 0.63 },
        };
        if v >= 3 {
            s.ramp = Some(ramp());
        }
    }
    if kind == Kind::IdColor {
        s.id_colors = vec![id(1, 1), id(3, 4), 0xabc123];
        s.id_tolerance = if v.is_multiple_of(2) { 8 } else { 43 };
    }
    s
}
fn raw(x: u32, y: u32, c: usize, k: MapKind) -> u16 {
    ((u64::from(x) * 1193 + u64::from(y) * 3571 + c as u64 * 13451 + k as u64 * 7919) % 65536)
        as u16
}
fn id(x: u32, y: u32) -> u32 {
    let b = |c| ((raw(x, y, c, MapKind::Id) as u32) * 255 + 32767) / 65535;
    b(0) << 16 | b(1) << 8 | b(2)
}
pub struct Maps {
    pub data: Vec<(MapKind, Vec<u16>, Vec<u8>)>,
    pub w: u32,
    pub h: u32,
}
impl Maps {
    pub fn new(s: &Settings, w: u32, h: u32) -> Self {
        let data = s
            .used_maps()
            .into_iter()
            .map(|k| {
                let mut data = Vec::with_capacity(w as usize * h as usize * k.channels());
                let mut cover = Vec::with_capacity(w as usize * h as usize);
                for y in 0..h {
                    for x in 0..w {
                        for c in 0..k.channels() {
                            data.push(raw(x, y, c, k));
                        }
                        cover.push(if (x + 3 * y + k as u32).is_multiple_of(17) {
                            0
                        } else {
                            1 + (x % 2) as u8
                        });
                    }
                }
                (k, data, cover)
            })
            .collect();
        Self { data, w, h }
    }
    pub fn maps(&self) -> Vec<Map<'_>> {
        self.data
            .iter()
            .map(|(k, d, c)| Map {
                kind: *k,
                width: self.w,
                height: self.h,
                data: d,
                coverage: c,
                bounds_min: [-1., -2., -3.],
                bounds_max: [2., 3., 1.],
                condition_key: KEY,
                state: MapState::Current,
            })
            .collect()
    }
}
pub fn frame() -> ModelFrame {
    ModelFrame::new([0.13, -0.27, 0.41], [0.17, -0.31, 0.23, 0.89]).unwrap()
}
pub fn pixels(w: u32, h: u32, target: Target) -> Vec<u8> {
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        for x in 0..w {
            let mut p = [
                ((x * 17 + y * 31 + 3) % 256) as u8,
                ((x * 7 + y * 13 + 71) % 256) as u8,
                ((x * 43 + y * 5 + 191) % 256) as u8,
                if (x + y) % 7 == 0 {
                    0
                } else if (x + y) % 5 == 0 {
                    255
                } else {
                    ((x * 19 + y * 23 + 41) % 256) as u8
                },
            ];
            if target == Target::Scalar {
                p[1] = p[0];
                p[2] = p[0];
            }
            if target == Target::Mask {
                p[0] = 0;
                p[1] = 0;
                p[2] = 0;
            }
            out.extend_from_slice(&p);
        }
    }
    out
}
/// 試験用の設定・マップ・Anchor の値を束ねた `BoundGenerator` を作って `f` に渡す。
pub fn with_bound<R>(
    kind: Kind,
    v: usize,
    w: u32,
    h: u32,
    f: impl FnOnce(&BoundGenerator<'_>) -> R,
) -> R {
    let s = settings(kind, v);
    let owned = Maps::new(&s, w, h);
    let maps = owned.maps();
    let anchor_bytes = pixels(w, h, Target::Color);
    let image = Image::new(&anchor_bytes, w, h).unwrap();
    let layer = anchor::LayerSample {
        source: &image,
        read: [
            anchor::Read::Color,
            anchor::Read::Scalar,
            anchor::Read::Coverage,
        ][v % 3],
    };
    let mask = anchor::MaskSample::new(anchor::Mask {
        source: &image,
        enabled: !v.is_multiple_of(4),
        inverted: v % 2 == 1,
        density: 0.63,
    })
    .unwrap();
    let a: &dyn anchor::ValueSource = if v >= 6 { &mask } else { &layer };
    f(&BoundGenerator::bind(&s, &maps, Some(frame()), (w, h), Ok(a)).unwrap())
}
pub fn run(kind: Kind, v: usize, target: Target, w: u32, h: u32) -> Vec<u8> {
    let bytes = pixels(w, h, target);
    let source = Image::new(&bytes, w, h).unwrap();
    with_bound(kind, v, w, h, |bound| {
        evaluate(
            &source,
            bound,
            yolu_core::Rect::new(0, 0, w, h),
            target,
            if v.is_multiple_of(2) { 1. } else { 0.43 },
            &Options::default(),
        )
        .unwrap()
        .pixels
    })
}
pub fn anchor_scene(v: usize) -> Vec<u8> {
    anchor_scene_region(v, yolu_core::Rect::new(0, 0, 41, 29))
}
/// 41×29 の部分スタックの `region` だけの評価（全面は `anchor_scene`）。
pub fn anchor_scene_region(v: usize, region: yolu_core::Rect) -> Vec<u8> {
    use anchor::*;
    use yolu_core::{BlendMode, ChannelKind};
    let bytes = pixels(41, 29, Target::Color);
    let image = Image::new(&bytes, 41, 29).unwrap();
    let mut layers = vec![
        Layer::new(Content::Pixels(&image)),
        Layer::new(Content::Group),
        Layer::new(Content::Fill(Rgba8::new(200, 60, 20, 173))),
        Layer::new(Content::Fill(Rgba8::new(20, 180, 230, 121))),
        Layer::new(Content::Fill(Rgba8::new(245, 7, 191, 231))),
    ];
    if v >= 24 {
        layers[2].content = Content::Adjustment(yolu_core::AdjustmentSettings::invert());
    }
    layers[1].blend = if v.is_multiple_of(2) {
        BlendMode::PassThrough
    } else {
        BlendMode::Normal
    };
    layers[1].opacity = 0.23;
    layers[1].visible = !v.is_multiple_of(7);
    layers[1].clipping = v.is_multiple_of(4);
    layers[2].parent = Some(1);
    layers[3].parent = Some(1);
    layers[2].opacity = 0.67;
    layers[2].blend = BlendMode::Multiply;
    layers[2].clipping = v.is_multiple_of(5);
    layers[2].visible = !v.is_multiple_of(6);
    layers[3].clipping = v.is_multiple_of(3);
    layers[4].clipping = true;
    layers[2].mask = Some(Mask {
        source: &image,
        enabled: true,
        inverted: v % 2 == 1,
        density: 0.63,
    });
    if v >= 12 {
        let mut outer = Layer::new(Content::Group);
        outer.blend = if v.is_multiple_of(3) {
            BlendMode::Normal
        } else {
            BlendMode::PassThrough
        };
        outer.opacity = 0.31;
        outer.visible = false;
        layers.push(outer);
        layers[1].parent = Some(5);
        layers[4].parent = Some(5);
    }
    let host = [2, 3, 1, 4][v % 4];
    Plan::new(&layers, host, (41, 29), ChannelKind::Color)
        .unwrap()
        .evaluate(region, &Options::default())
        .unwrap()
}
