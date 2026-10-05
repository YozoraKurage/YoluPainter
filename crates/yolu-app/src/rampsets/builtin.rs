//! 組み込みのグラデーションセット（色味の違う一揃いを数組）。色はこのアプリのために作ったもので、位置は端と真ん中が中心、中点は 0.5、
//! 不透明度はすべて 1（PSD の刻みに乗る）。最初の組「基本」だけは、メインの色とサブの色（描画色）から作る見本を含む。

use yolu_core::generator::{ColorStop, OpacityStop, Preset, Ramp};
use yolu_core::Rgba8;

/// 組み込みの 1 つ（名前は日本語・英語）。
pub struct Builtin {
    pub ja: &'static str,
    pub en: &'static str,
    pub ramp: Ramp,
}

/// 組み込みの組。
pub struct BuiltinGroup {
    pub ja: &'static str,
    pub en: &'static str,
    pub items: Vec<Builtin>,
}

/// 位置と色の列からランプを作る（中点 0.5・不透明度 1）。
fn ramp(stops: &[(f64, [u8; 3])]) -> Ramp {
    let colors = stops
        .iter()
        .map(|&(position, c)| ColorStop {
            position,
            color: Rgba8::new(c[0], c[1], c[2], 255),
            midpoint: 0.5,
        })
        .collect();
    let opacity = |position| OpacityStop {
        position,
        opacity: 1.0,
        midpoint: 0.5,
    };
    Ramp::new(colors, vec![opacity(0.0), opacity(1.0)], None).expect("組み込みのランプ")
}

fn item(ja: &'static str, en: &'static str, ramp: Ramp) -> Builtin {
    Builtin { ja, en, ramp }
}

/// 暗い所を青みの沈んだ色へ、明るい所を暖かい淡い色へ寄せた 3 色（影が黒く潰れず、色が濁らない陰影用）。
fn shade(
    ja: &'static str,
    en: &'static str,
    shadow: [u8; 3],
    mid: [u8; 3],
    light: [u8; 3],
) -> Builtin {
    item(ja, en, ramp(&[(0.0, shadow), (0.5, mid), (1.0, light)]))
}

fn rgb8(c: [f32; 4]) -> Rgba8 {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgba8::new(byte(c[0]), byte(c[1]), byte(c[2]), 255)
}

/// 全部の組み込みの組。`main`・`sub` はメインの色とサブの色（straight の RGBA 0〜1）で、「基本」の見本に使う。
pub fn groups(main: [f32; 4], sub: [f32; 4]) -> Vec<BuiltinGroup> {
    let (main, sub) = (rgb8(main), rgb8(sub));
    vec![
        BuiltinGroup {
            ja: "基本",
            en: "Basic",
            items: vec![
                item("黒→白", "Black to White", Ramp::default()),
                item(
                    "白→黒",
                    "White to Black",
                    Ramp::preset(Preset::WhiteBlack, main, sub),
                ),
                item(
                    "メイン→サブ",
                    "Main to Sub",
                    Ramp::preset(Preset::ForegroundBackground, main, sub),
                ),
                item(
                    "メイン→透明",
                    "Main to Transparent",
                    Ramp::preset(Preset::ForegroundTransparent, main, sub),
                ),
                item(
                    "暖→寒",
                    "Warm to Cool",
                    Ramp::preset(Preset::WarmCool, main, sub),
                ),
            ],
        },
        BuiltinGroup {
            ja: "陰影",
            en: "Shading",
            items: vec![
                shade(
                    "陰影（赤）",
                    "Shade (Red)",
                    [58, 36, 72],
                    [196, 78, 80],
                    [255, 226, 200],
                ),
                shade(
                    "陰影（黄）",
                    "Shade (Yellow)",
                    [70, 56, 62],
                    [214, 170, 72],
                    [255, 246, 208],
                ),
                shade(
                    "陰影（緑）",
                    "Shade (Green)",
                    [34, 52, 64],
                    [84, 150, 96],
                    [236, 250, 208],
                ),
                shade(
                    "陰影（青）",
                    "Shade (Blue)",
                    [28, 32, 70],
                    [72, 120, 200],
                    [222, 238, 255],
                ),
                shade(
                    "陰影（紫）",
                    "Shade (Purple)",
                    [40, 30, 70],
                    [130, 90, 180],
                    [246, 226, 255],
                ),
                shade(
                    "陰影（桃）",
                    "Shade (Pink)",
                    [66, 36, 70],
                    [226, 120, 150],
                    [255, 236, 236],
                ),
            ],
        },
        BuiltinGroup {
            ja: "色味",
            en: "Tones",
            items: vec![
                shade(
                    "セピア",
                    "Sepia",
                    [36, 22, 10],
                    [160, 110, 70],
                    [255, 235, 200],
                ),
                shade(
                    "青と橙",
                    "Teal and Orange",
                    [16, 40, 84],
                    [128, 120, 124],
                    [255, 190, 110],
                ),
                shade(
                    "夕焼け",
                    "Sunset",
                    [30, 10, 60],
                    [200, 40, 80],
                    [255, 200, 80],
                ),
                shade(
                    "深緑",
                    "Forest",
                    [4, 20, 12],
                    [40, 120, 70],
                    [190, 255, 190],
                ),
                shade("氷", "Ice", [6, 12, 48], [90, 170, 230], [255, 255, 255]),
                shade(
                    "桜",
                    "Blossom",
                    [62, 34, 60],
                    [232, 142, 170],
                    [255, 244, 240],
                ),
                shade(
                    "砂漠",
                    "Desert",
                    [52, 34, 40],
                    [204, 130, 72],
                    [252, 232, 176],
                ),
                shade("海", "Sea", [8, 24, 56], [30, 130, 160], [214, 246, 240]),
            ],
        },
        BuiltinGroup {
            ja: "空と光",
            en: "Sky and Light",
            items: vec![
                item(
                    "朝",
                    "Morning",
                    ramp(&[
                        (0.0, [44, 52, 98]),
                        (0.4375, [226, 150, 150]),
                        (1.0, [255, 240, 196]),
                    ]),
                ),
                item(
                    "昼",
                    "Noon",
                    ramp(&[
                        (0.0, [30, 86, 170]),
                        (0.59375, [118, 190, 240]),
                        (1.0, [250, 252, 255]),
                    ]),
                ),
                item(
                    "夕",
                    "Evening",
                    ramp(&[
                        (0.0, [40, 30, 84]),
                        (0.34375, [170, 70, 110]),
                        (0.6875, [250, 140, 80]),
                        (1.0, [255, 226, 150]),
                    ]),
                ),
                item(
                    "夜",
                    "Night",
                    ramp(&[
                        (0.0, [4, 6, 24]),
                        (0.59375, [30, 44, 110]),
                        (1.0, [170, 190, 250]),
                    ]),
                ),
                item(
                    "月明かり",
                    "Moonlight",
                    ramp(&[
                        (0.0, [10, 14, 36]),
                        (0.5, [70, 96, 150]),
                        (1.0, [236, 242, 255]),
                    ]),
                ),
                item(
                    "木漏れ日",
                    "Dappled Light",
                    ramp(&[
                        (0.0, [24, 44, 40]),
                        (0.5, [120, 160, 80]),
                        (1.0, [255, 250, 190]),
                    ]),
                ),
            ],
        },
        BuiltinGroup {
            ja: "特殊",
            en: "Special",
            items: vec![
                item(
                    "虹",
                    "Rainbow",
                    ramp(&[
                        (0.0, [200, 40, 70]),
                        (0.1875, [240, 150, 40]),
                        (0.40625, [240, 224, 70]),
                        (0.59375, [70, 190, 110]),
                        (0.8125, [60, 120, 220]),
                        (1.0, [140, 80, 200]),
                    ]),
                ),
                item(
                    "熱",
                    "Heat",
                    ramp(&[
                        (0.0, [8, 0, 40]),
                        (0.3125, [150, 20, 90]),
                        (0.59375, [240, 110, 30]),
                        (0.84375, [255, 220, 90]),
                        (1.0, [255, 255, 235]),
                    ]),
                ),
                item(
                    "金属",
                    "Metal",
                    ramp(&[
                        (0.0, [30, 32, 38]),
                        (0.3125, [150, 156, 166]),
                        (0.5, [60, 64, 72]),
                        (0.75, [210, 214, 220]),
                        (1.0, [255, 255, 255]),
                    ]),
                ),
                item(
                    "ネオン",
                    "Neon",
                    ramp(&[
                        (0.0, [10, 0, 40]),
                        (0.5, [210, 40, 200]),
                        (1.0, [60, 240, 255]),
                    ]),
                ),
                item(
                    "フィルム",
                    "Film",
                    ramp(&[
                        (0.0, [20, 30, 36]),
                        (0.5, [150, 140, 110]),
                        (1.0, [250, 236, 200]),
                    ]),
                ),
                item(
                    "二色分け",
                    "Duotone",
                    ramp(&[(0.0, [24, 28, 90]), (1.0, [255, 210, 120])]),
                ),
            ],
        },
    ]
}
