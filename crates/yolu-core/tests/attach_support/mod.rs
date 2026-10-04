//! 効果の試験の道具: 人工の文書・メッシュマップ・画像・乱数。実データは使わない。
#![allow(dead_code)]
use yolu_core::generator::{MapKind, MapState};
use yolu_core::{
    BrushSettings, Channel, Document, EffectInputs, ImageColorSpace, ImageId, ImageInput, LayerId,
    MapInput, ModelFrame, Rect, Rgba8,
};

pub const W: u32 = 40;
pub const H: u32 = 28;

pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

pub const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn raw(x: u32, y: u32, c: usize, k: usize) -> u16 {
    ((x as u64 * 1193 + y as u64 * 3571 + c as u64 * 13451 + k as u64 * 7919) % 65536) as u16
}

/// 人工のメッシュマップ（Thickness は焼いていない）と画像 2 枚。`seed` を変えると別のマップになる。
pub fn inputs(seed: u32) -> EffectInputs {
    let mut inputs = EffectInputs::new();
    for kind in [
        MapKind::WorldNormal,
        MapKind::Position,
        MapKind::AmbientOcclusion,
        MapKind::Curvature,
        MapKind::Id,
        MapKind::BentNormal,
    ] {
        let channels = kind.channels();
        let k = kind as usize + seed as usize;
        let mut data = Vec::new();
        let mut coverage = Vec::new();
        for y in 0..H {
            for x in 0..W {
                for c in 0..channels {
                    data.push(match kind {
                        MapKind::Position => match c {
                            0 => (65535.0 * (x as f64 + 0.5) / W as f64) as u16,
                            1 => (65535.0 * (y as f64 + 0.5) / H as f64) as u16,
                            _ => (65535.0 * (((x * 3 + y * 5) % 41) as f64 / 40.0)) as u16,
                        },
                        _ => raw(x, y, c, k),
                    });
                }
                coverage.push(if (x + 3 * y + k as u32).is_multiple_of(17) {
                    0
                } else {
                    1 + (x % 2) as u8
                });
            }
        }
        inputs = inputs
            .with_map(
                MapInput::new(
                    kind,
                    W,
                    H,
                    data,
                    coverage,
                    [-1.0, -2.0, -3.0],
                    [2.0, 3.0, 1.0],
                    KEY,
                    MapState::Current,
                )
                .unwrap(),
            )
            .unwrap();
    }
    inputs = inputs.with_frame(Some(
        ModelFrame::new([0.13, -0.27, 0.41], [0.17, -0.31, 0.23, 0.89]).unwrap(),
    ));
    for (n, (w, h, space)) in [
        (8u32, 6u32, ImageColorSpace::Srgb),
        (5, 7, ImageColorSpace::Linear),
    ]
    .into_iter()
    .enumerate()
    {
        let mut pixels = Vec::new();
        for y in 0..h {
            for x in 0..w {
                pixels.extend([
                    (x * 31 + 20 + n as u32) as u8,
                    (y * 40 + 10) as u8,
                    if (x + y) % 2 == 0 { 220 } else { 60 },
                    if (x + 2 * y) % 5 == 0 { 90 } else { 255 },
                ]);
            }
        }
        inputs = inputs.with_image(image(n), ImageInput::new(w, h, pixels, space).unwrap());
    }
    inputs
}

pub fn image(n: usize) -> ImageId {
    ImageId(0x1000 + n as u128)
}

pub fn paint(doc: &mut Document, id: LayerId, channel: Channel, seed: u32) {
    for y in 0..doc.height() {
        for x in 0..doc.width() {
            if (x * 7 + y * 3 + seed).is_multiple_of(5) {
                continue;
            }
            let c = Rgba8::new(
                (x * 13 + y * 5 + seed * 3) as u8,
                (x * 3 + y * 17 + seed) as u8,
                (x + y * 31 + seed * 7) as u8,
                ((x * 29 + y * 11 + seed * 5) % 256) as u8,
            );
            doc.set_channel_pixel(id, channel, x, y, c).unwrap();
        }
    }
}

/// 40×28・タイル 8 の文書: 土台（Color・Height）、中（Height）、上（Color・Height）の 3 つのラスターと、塗りつぶし。
pub fn world() -> (Document, Vec<LayerId>) {
    let mut doc = Document::with_tile_size(W, H, 8).unwrap();
    let base = doc.add_layer("土台").unwrap();
    paint(&mut doc, base, Channel::Color, 1);
    paint(&mut doc, base, Channel::Height, 2);
    let mid = doc.add_layer("中").unwrap();
    paint(&mut doc, mid, Channel::Height, 3);
    let top = doc.add_layer("上").unwrap();
    paint(&mut doc, top, Channel::Color, 4);
    paint(&mut doc, top, Channel::Height, 5);
    let fill = doc
        .add_fill_layer(
            "塗り",
            &[
                (Channel::Color, Rgba8::new(60, 160, 220, 200)),
                (Channel::Height, Rgba8::new(128, 128, 128, 100)),
            ],
            None,
        )
        .unwrap();
    doc.set_effect_inputs(inputs(0)).unwrap();
    // 評価のブロックを小さく（2×2 タイル）して、ブロックの境・半径の窓を使う
    doc.set_filter_block_pixels(16).unwrap();
    (doc, vec![base, mid, top, fill])
}

pub fn stroke(doc: &mut Document, layer: LayerId, channel: Channel, rng: &mut Rng) {
    let brush = BrushSettings {
        color: Rgba8::new(
            rng.below(256) as u8,
            rng.below(256) as u8,
            rng.below(256) as u8,
            255,
        ),
        ..BrushSettings::default()
    };
    let mut s = doc.begin_stroke_in(layer, channel, &brush).unwrap();
    let (mut x, mut y) = (rng.unit() * W as f64, rng.unit() * H as f64);
    for _ in 0..4 {
        s.add_point(doc, x, y, 1.0, yolu_core::glam::DVec2::ZERO)
            .unwrap();
        x = (x + rng.unit() * 12.0 - 6.0).clamp(0.0, W as f64 - 1.0);
        y = (y + rng.unit() * 12.0 - 6.0).clamp(0.0, H as f64 - 1.0);
    }
    doc.end_stroke(s).unwrap();
}

pub fn whole(doc: &Document, c: Channel) -> Vec<u8> {
    doc.composite_channel(c, Rect::new(0, 0, doc.width(), doc.height()))
        .unwrap()
}
