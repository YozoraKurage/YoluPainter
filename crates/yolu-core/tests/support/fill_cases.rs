use yolu_core::{fill_image::*, Rgba8};
pub fn picture(w: usize, h: usize, shape: bool) -> Vec<u8> {
    let mut d = vec![0; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            d[i] = (x * 37 + y * 11) as u8;
            d[i + 1] = (x * 13 + y * 43) as u8;
            d[i + 2] = (x * 19 + y * 7 + 61) as u8;
            d[i + 3] = if shape {
                ((x * 17 + y * 23) % 256) as u8
            } else if (x + y) % 7 == 0 {
                0
            } else if (x * 3 + y) % 5 == 0 {
                128
            } else {
                255
            };
        }
    }
    d
}
pub struct Fixture {
    pub w: u32,
    pub h: u32,
    pub pos: Vec<u16>,
    pub nor: Vec<u16>,
    pub pc: Vec<u8>,
    pub nc: Vec<u8>,
}
impl Fixture {
    pub fn new(w: u32, h: u32) -> Self {
        let n = w as usize * h as usize;
        let mut s = Self {
            w,
            h,
            pos: vec![0; n * 3],
            nor: vec![0; n * 3],
            pc: vec![0; n],
            nc: vec![0; n],
        };
        for y in 0..h as usize {
            for x in 0..w as usize {
                let i = y * w as usize + x;
                s.pos[i * 3] = (x * 65535 / (w as usize - 1).max(1)) as u16;
                s.pos[i * 3 + 1] = (y * 65535 / (h as usize - 1).max(1)) as u16;
                s.pos[i * 3 + 2] = ((x * 197 + y * 101 + 13000) % 65536) as u16;
                s.nor[i * 3] = ((x * 311 + y * 17 + 4000) % 65536) as u16;
                s.nor[i * 3 + 1] = ((x * 37 + y * 211 + 21000) % 65536) as u16;
                s.nor[i * 3 + 2] = ((x * 71 + y * 97 + 1000) % 65536) as u16;
                s.pc[i] = u8::from(!(x as i64 > w as i64 - 4 && y as i64 > h as i64 - 4));
                s.nc[i] = if x == 2 && y == 3 { 0 } else { s.pc[i] };
            }
        }
        s
    }
    pub fn sampler<'a>(
        &'a self,
        mode: u8,
        v: usize,
        chain: &'a ImageMipChain<'a>,
        shape: &'a ImageMipChain<'a>,
    ) -> FillSampler<'a> {
        let frame = if v.is_multiple_of(2) {
            ModelFrame::default()
        } else {
            ModelFrame {
                position: [0.13, -0.07, 0.09],
                rotation: [
                    0.,
                    (std::f64::consts::PI / 12.).sin(),
                    0.,
                    (std::f64::consts::PI / 12.).cos(),
                ],
            }
        };
        FillSampler::bind(FillInput {
            projection: projection(mode, v),
            width: self.w,
            height: self.h,
            image: if mode == 5 && v == 7 {
                None
            } else {
                Some(chain)
            },
            shape: if mode == 5 && v >= 6 {
                Some(shape)
            } else {
                None
            },
            fallback: Rgba8::new(17, 81, 203, 149),
            positions: Some(Map {
                width: self.w,
                height: self.h,
                values: &self.pos,
                coverage: &self.pc,
            }),
            normals: Some(Map {
                width: self.w,
                height: self.h,
                values: &self.nor,
                coverage: &self.nc,
            }),
            bounds_min: [-1., -1., -0.5],
            bounds_max: [1., 1., 0.5],
            frame: Some(frame),
            ..FillInput::default()
        })
        .unwrap()
    }
}
pub fn projection(mode: u8, v: usize) -> Projection {
    let mut p = Projection {
        mode: mode.try_into().unwrap(),
        wrap: ((v % 3) as u8).try_into().unwrap(),
        blend_width: match v % 3 {
            0 => 0.,
            1 => 0.3,
            _ => 1.,
        },
        placement: Placement {
            center: [0.1, -0.2, 0.],
            rotation: if v == 0 { [0.; 3] } else { [20., -35., 10.] },
            size: [1.6, 1.8, 0.9],
        },
        ..Projection::default()
    };
    if v != 0 {
        p.tiles = if v == 3 {
            [40., 35.]
        } else if v == 4 {
            [0.25, 0.75]
        } else {
            [2.3, 1.7]
        };
        p.offset = [0.17, -0.23];
        p.rotation = if v == 4 { -90. } else { 27. };
    }
    if mode == 5 {
        p.depth_hardness = if v == 0 { 1. } else { 0.3 };
        p.backface_angle = if v == 0 { 180. } else { 120. };
        p.backface_hardness = if v == 0 { 1. } else { 0.4 };
    }
    p
}
/// デカールの `apply_decal_to_value` 用の人工の値（座標だけで決まる。tools/csharp-golden/FillGolden.cs の Value と同じ式）。
#[allow(dead_code)] // 計測の example はこの支えを読むが、値の経路は使わない
pub fn decal_values(w: usize, h: usize) -> Vec<u8> {
    let mut d = vec![0; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            d[i] = (x * 29 + y * 7 + 3) as u8;
            d[i + 1] = (x * 5 + y * 31 + 90) as u8;
            d[i + 2] = (x * 3 + y * 3 + 200) as u8;
            d[i + 3] = ((x * 11 + y * 13) % 256) as u8;
        }
    }
    d
}
