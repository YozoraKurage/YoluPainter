//! 組み込みのグラデーションセット（色味の違う一揃いを数組）。色はこのアプリのために作ったもので、位置は端と真ん中が中心、中点は 0.5、
//! 不透明度はすべて 1（PSD の刻みに乗る）。最初の組「基本」だけは、メインの色とサブの色（描画色）から作る見本を含む。

use crate::engine::Rgba8;
use crate::lang::Lang;
use yolu_core::generator::{ColorStop, OpacityStop, Preset, Ramp};

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

fn item(name: fn(Lang) -> &'static str, ramp: Ramp) -> Builtin {
    Builtin {
        ja: name(Lang::Ja),
        en: name(Lang::En),
        ramp,
    }
}

/// 暗い所を青みの沈んだ色へ、明るい所を暖かい淡い色へ寄せた 3 色（影が黒く潰れず、色が濁らない陰影用）。
fn shade(name: fn(Lang) -> &'static str, shadow: [u8; 3], mid: [u8; 3], light: [u8; 3]) -> Builtin {
    item(name, ramp(&[(0.0, shadow), (0.5, mid), (1.0, light)]))
}

/// f32 の色(straight の RGBA 0〜1)を、アルファを 255 にした 8 ビットの色にする（RGB だけを見る）。
pub(crate) fn opaque_rgba8(c: [f32; 4]) -> Rgba8 {
    Rgba8::from_unit_f32([c[0], c[1], c[2], 1.0])
}

/// 全部の組み込みの組。`main`・`sub` はメインの色とサブの色（straight の RGBA 0〜1）で、「基本」の見本に使う。
pub fn groups(main: [f32; 4], sub: [f32; 4]) -> Vec<BuiltinGroup> {
    let (main, sub) = (opaque_rgba8(main), opaque_rgba8(sub));
    vec![
        BuiltinGroup {
            ja: Lang::Ja.t("rampsets-basic"),
            en: Lang::En.t("rampsets-basic"),
            items: vec![
                item(|l| l.t("rampsets-black-to-white"), Ramp::default()),
                item(
                    |l| l.t("rampsets-white-to-black"),
                    Ramp::preset(Preset::WhiteBlack, main, sub),
                ),
                item(
                    |l| l.t("rampsets-main-to-sub"),
                    Ramp::preset(Preset::ForegroundBackground, main, sub),
                ),
                item(
                    |l| l.t("rampsets-main-to-transparent"),
                    Ramp::preset(Preset::ForegroundTransparent, main, sub),
                ),
                item(
                    |l| l.t("rampsets-warm-to-cool"),
                    Ramp::preset(Preset::WarmCool, main, sub),
                ),
            ],
        },
        BuiltinGroup {
            ja: Lang::Ja.t("rampsets-shading"),
            en: Lang::En.t("rampsets-shading"),
            items: vec![
                shade(
                    |l| l.t("rampsets-shade-red"),
                    [58, 36, 72],
                    [196, 78, 80],
                    [255, 226, 200],
                ),
                shade(
                    |l| l.t("rampsets-shade-yellow"),
                    [70, 56, 62],
                    [214, 170, 72],
                    [255, 246, 208],
                ),
                shade(
                    |l| l.t("rampsets-shade-green"),
                    [34, 52, 64],
                    [84, 150, 96],
                    [236, 250, 208],
                ),
                shade(
                    |l| l.t("rampsets-shade-blue"),
                    [28, 32, 70],
                    [72, 120, 200],
                    [222, 238, 255],
                ),
                shade(
                    |l| l.t("rampsets-shade-purple"),
                    [40, 30, 70],
                    [130, 90, 180],
                    [246, 226, 255],
                ),
                shade(
                    |l| l.t("rampsets-shade-pink"),
                    [66, 36, 70],
                    [226, 120, 150],
                    [255, 236, 236],
                ),
            ],
        },
        BuiltinGroup {
            ja: Lang::Ja.t("rampsets-tones"),
            en: Lang::En.t("rampsets-tones"),
            items: vec![
                shade(
                    |l| l.t("rampsets-sepia"),
                    [36, 22, 10],
                    [160, 110, 70],
                    [255, 235, 200],
                ),
                shade(
                    |l| l.t("rampsets-teal-and-orange"),
                    [16, 40, 84],
                    [128, 120, 124],
                    [255, 190, 110],
                ),
                shade(
                    |l| l.t("rampsets-sunset"),
                    [30, 10, 60],
                    [200, 40, 80],
                    [255, 200, 80],
                ),
                shade(
                    |l| l.t("rampsets-forest"),
                    [4, 20, 12],
                    [40, 120, 70],
                    [190, 255, 190],
                ),
                shade(
                    |l| l.t("rampsets-ice"),
                    [6, 12, 48],
                    [90, 170, 230],
                    [255, 255, 255],
                ),
                shade(
                    |l| l.t("rampsets-blossom"),
                    [62, 34, 60],
                    [232, 142, 170],
                    [255, 244, 240],
                ),
                shade(
                    |l| l.t("rampsets-desert"),
                    [52, 34, 40],
                    [204, 130, 72],
                    [252, 232, 176],
                ),
                shade(
                    |l| l.t("rampsets-sea"),
                    [8, 24, 56],
                    [30, 130, 160],
                    [214, 246, 240],
                ),
            ],
        },
        BuiltinGroup {
            ja: Lang::Ja.t("rampsets-sky-and-light"),
            en: Lang::En.t("rampsets-sky-and-light"),
            items: vec![
                item(
                    |l| l.t("rampsets-morning"),
                    ramp(&[
                        (0.0, [44, 52, 98]),
                        (0.4375, [226, 150, 150]),
                        (1.0, [255, 240, 196]),
                    ]),
                ),
                item(
                    |l| l.t("rampsets-noon"),
                    ramp(&[
                        (0.0, [30, 86, 170]),
                        (0.59375, [118, 190, 240]),
                        (1.0, [250, 252, 255]),
                    ]),
                ),
                item(
                    |l| l.t("rampsets-evening"),
                    ramp(&[
                        (0.0, [40, 30, 84]),
                        (0.34375, [170, 70, 110]),
                        (0.6875, [250, 140, 80]),
                        (1.0, [255, 226, 150]),
                    ]),
                ),
                item(
                    |l| l.t("rampsets-night"),
                    ramp(&[
                        (0.0, [4, 6, 24]),
                        (0.59375, [30, 44, 110]),
                        (1.0, [170, 190, 250]),
                    ]),
                ),
                item(
                    |l| l.t("rampsets-moonlight"),
                    ramp(&[
                        (0.0, [10, 14, 36]),
                        (0.5, [70, 96, 150]),
                        (1.0, [236, 242, 255]),
                    ]),
                ),
                item(
                    |l| l.t("rampsets-dappled-light"),
                    ramp(&[
                        (0.0, [24, 44, 40]),
                        (0.5, [120, 160, 80]),
                        (1.0, [255, 250, 190]),
                    ]),
                ),
            ],
        },
        BuiltinGroup {
            ja: Lang::Ja.t("rampsets-special"),
            en: Lang::En.t("rampsets-special"),
            items: vec![
                item(
                    |l| l.t("rampsets-rainbow"),
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
                    |l| l.t("rampsets-heat"),
                    ramp(&[
                        (0.0, [8, 0, 40]),
                        (0.3125, [150, 20, 90]),
                        (0.59375, [240, 110, 30]),
                        (0.84375, [255, 220, 90]),
                        (1.0, [255, 255, 235]),
                    ]),
                ),
                item(
                    |l| l.t("rampsets-metal"),
                    ramp(&[
                        (0.0, [30, 32, 38]),
                        (0.3125, [150, 156, 166]),
                        (0.5, [60, 64, 72]),
                        (0.75, [210, 214, 220]),
                        (1.0, [255, 255, 255]),
                    ]),
                ),
                item(
                    |l| l.t("rampsets-neon"),
                    ramp(&[
                        (0.0, [10, 0, 40]),
                        (0.5, [210, 40, 200]),
                        (1.0, [60, 240, 255]),
                    ]),
                ),
                item(
                    |l| l.t("rampsets-film"),
                    ramp(&[
                        (0.0, [20, 30, 36]),
                        (0.5, [150, 140, 110]),
                        (1.0, [250, 236, 200]),
                    ]),
                ),
                item(
                    |l| l.t("rampsets-duotone"),
                    ramp(&[(0.0, [24, 28, 90]), (1.0, [255, 210, 120])]),
                ),
            ],
        },
    ]
}
