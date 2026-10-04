#![allow(dead_code)]
use yolu_core::filter::{Generated, GeneratorBlend, GeneratorInput, Settings, Stage, ValueType};
pub fn pattern(w: u32, h: u32, ty: ValueType) -> Vec<u8> {
    let mut b = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        for x in 0..w {
            let r = ((x * 17 + y * 31 + 3) % 256) as u8;
            let mut g = ((x * 7 + y * 13 + 71) % 256) as u8;
            let mut blue = ((x * 43 + y * 5 + 191) % 256) as u8;
            let a = if (x + y) % 7 == 0 {
                0
            } else if (x + y) % 5 == 0 {
                255
            } else {
                ((x * 19 + y * 23 + 41) % 256) as u8
            };
            if ty == ValueType::Scalar {
                g = r;
                blue = r;
            }
            b.extend_from_slice(&[r, g, blue, a]);
        }
    }
    b
}
/// C# の実 FilterEngine が解決した Generator の値（FilterGolden.cs の SampleGolden）。段ごとに 1 画素 9 バイト:
/// 種類（0 = 値なし、1 = スカラー f64、2 = ランプ適用後の RGBA）+ 8 バイト。段の番号が slot。
pub struct Samples {
    pub width: u32,
    pub tables: Vec<Vec<u8>>,
}
impl Samples {
    /// `<名前>.s<段>` を読む（Generator でない段は空）。1 つも無ければ None。
    pub fn load(dir: &std::path::Path, name: &str, width: u32) -> Option<Self> {
        let tables: Vec<Vec<u8>> = (0..32)
            .map(|k| std::fs::read(dir.join(format!("{name}.s{k}"))).unwrap_or_default())
            .collect();
        tables
            .iter()
            .any(|t| !t.is_empty())
            .then_some(Self { width, tables })
    }
}
impl GeneratorInput for Samples {
    fn sample(&self, slot: u32, x: u32, y: u32) -> Option<Generated> {
        let t = &self.tables[slot as usize];
        let i = (y as usize * self.width as usize + x as usize) * 9;
        match t[i] {
            0 => None,
            1 => Some(Generated::Scalar(f64::from_le_bytes(
                t[i + 1..i + 9].try_into().unwrap(),
            ))),
            2 => Some(Generated::Mapped([t[i + 1], t[i + 2], t[i + 3], t[i + 4]])),
            k => panic!("値の種類が不正: {k}"),
        }
    }
}
pub fn stages(s: &str) -> Vec<Stage> {
    s.split(';')
        .enumerate()
        .map(|(index, s)| {
            let p: Vec<_> = s.split_whitespace().collect();
            let d = |i: usize| p[i].parse::<f64>().unwrap();
            let u = |i: usize| p[i].parse::<u32>().unwrap();
            let settings = match p[1] {
                "blur" => Settings::GaussianBlur { radius: u(2) },
                "sharpen" => Settings::Sharpen {
                    radius: u(2),
                    amount: d(3),
                    threshold: u(4),
                },
                "noise" => Settings::Noise {
                    amount: d(2),
                    seed: p[3].parse().unwrap(),
                    monochrome: p[4] == "1",
                },
                "levels" => Settings::Levels {
                    input_black: d(2),
                    input_white: d(3),
                    gamma: d(4),
                    output_black: d(5),
                    output_white: d(6),
                },
                "invert" => Settings::Invert,
                "normalize" => Settings::Normalize,
                "generator" => Settings::Generator {
                    slot: index as u32,
                    blend: match p[2] {
                        "multiply" => GeneratorBlend::Multiply,
                        "replace" => GeneratorBlend::Replace,
                        "screen" => GeneratorBlend::Screen,
                        "max" => GeneratorBlend::Max,
                        "min" => GeneratorBlend::Min,
                        "add" => GeneratorBlend::Add,
                        "subtract" => GeneratorBlend::Subtract,
                        _ => panic!("ブレンドが不正"),
                    },
                },
                _ => panic!("種類が不正"),
            };
            Stage {
                settings,
                strength: d(0),
                enabled: true,
            }
        })
        .collect()
}
