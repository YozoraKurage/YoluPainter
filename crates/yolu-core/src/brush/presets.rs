//! 組み込みのブラシ（C# の BuiltInBrushes.Presets）。画面は描画色と大きさを上に重ねて使う。筆先と紙の質感は [`super::builtin_tip`]。

use super::settings::{Brush, Jitter, PaperTexture, TipShape};
use super::{builtin_tip, BrushSettings};

/// 名前の付いたブラシ（始めの設定）。
#[derive(Clone, Debug, PartialEq)]
pub struct BrushPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub brush: Brush,
}

fn tip(id: &str) -> std::sync::Arc<super::BrushTip> {
    builtin_tip(id).expect("組み込みの筆先")
}

/// 組み込みのブラシ 13 個（C# と同じ値・同じ並び）。呼ぶたびに新しい写しを返す。
pub fn builtin_presets() -> Vec<BrushPreset> {
    let base = BrushSettings::default();
    let mut v = Vec::new();
    let mut add = |id, name, category, brush: Brush| {
        v.push(BrushPreset {
            id,
            name,
            category,
            brush,
        })
    };
    let b = |s: BrushSettings| Brush::from(s);
    add(
        "soft-round",
        "Soft Round",
        "Basic",
        b(BrushSettings {
            radius: 24.0,
            hardness: 0.0,
            spacing: 0.1,
            ..base
        }),
    );
    add(
        "hard-round",
        "Hard Round",
        "Basic",
        b(BrushSettings {
            radius: 12.0,
            hardness: 0.95,
            spacing: 0.08,
            ..base
        }),
    );
    add(
        "airbrush",
        "Airbrush",
        "Basic",
        b(BrushSettings {
            radius: 40.0,
            hardness: 0.0,
            flow: 0.08,
            spacing: 0.05,
            pressure_flow: true,
            pressure_opacity: false,
            pressure_size: false,
            ..base
        }),
    );
    add(
        "pencil",
        "Pencil",
        "Drawing",
        Brush {
            texture: Some(PaperTexture::new(tip("grain"), 0.6)),
            ..b(BrushSettings {
                radius: 3.0,
                hardness: 0.6,
                spacing: 0.1,
                pressure_size: false,
                ..base
            })
        },
    );
    add(
        "ink-pen",
        "Ink Pen",
        "Drawing",
        Brush {
            tip: TipShape {
                roundness: 0.6,
                angle: 35.0,
                ..TipShape::default()
            },
            ..b(BrushSettings {
                radius: 6.0,
                hardness: 1.0,
                spacing: 0.05,
                pressure_opacity: false,
                ..base
            })
        },
    );
    add(
        "marker",
        "Marker",
        "Drawing",
        Brush {
            tip: TipShape {
                image: Some(tip("rounded-square")),
                ..TipShape::default()
            },
            ..b(BrushSettings {
                radius: 14.0,
                opacity: 0.7,
                flow: 0.9,
                spacing: 0.05,
                pressure_size: false,
                pressure_opacity: false,
                ..base
            })
        },
    );
    add(
        "chalk",
        "Chalk",
        "Dry media",
        Brush {
            tip: TipShape {
                image: Some(tip("noisy-disc")),
                ..TipShape::default()
            },
            jitter: Jitter {
                size: 0.2,
                angle: 1.0,
                ..Jitter::default()
            },
            texture: Some(PaperTexture {
                scale: 2.0,
                ..PaperTexture::new(tip("grain"), 0.5)
            }),
            ..b(BrushSettings {
                radius: 18.0,
                spacing: 0.2,
                ..base
            })
        },
    );
    add(
        "charcoal",
        "Charcoal",
        "Dry media",
        Brush {
            tip: TipShape {
                image: Some(tip("charcoal")),
                follow_direction: true,
                ..TipShape::default()
            },
            jitter: Jitter {
                angle: 0.1,
                ..Jitter::default()
            },
            texture: Some(PaperTexture::new(tip("grain"), 0.4)),
            ..b(BrushSettings {
                radius: 16.0,
                spacing: 0.1,
                ..base
            })
        },
    );
    add(
        "dry-brush",
        "Dry Brush",
        "Paint",
        Brush {
            tip: TipShape {
                image: Some(tip("bristles")),
                follow_direction: true,
                ..TipShape::default()
            },
            jitter: Jitter {
                opacity: 0.2,
                ..Jitter::default()
            },
            ..b(BrushSettings {
                radius: 20.0,
                spacing: 0.03,
                flow: 0.6,
                pressure_opacity: false,
                ..base
            })
        },
    );
    add(
        "watercolor",
        "Watercolor Edge",
        "Paint",
        Brush {
            tip: TipShape {
                image: Some(tip("rim")),
                ..TipShape::default()
            },
            jitter: Jitter {
                size: 0.1,
                angle: 1.0,
                ..Jitter::default()
            },
            ..b(BrushSettings {
                radius: 30.0,
                opacity: 0.6,
                flow: 0.25,
                spacing: 0.1,
                pressure_opacity: false,
                ..base
            })
        },
    );
    add(
        "splatter",
        "Splatter",
        "Effects",
        Brush {
            tip: TipShape {
                image: Some(tip("dots")),
                ..TipShape::default()
            },
            jitter: Jitter {
                scatter: 1.2,
                count: 3,
                size: 0.6,
                angle: 1.0,
                ..Jitter::default()
            },
            ..b(BrushSettings {
                radius: 20.0,
                spacing: 0.5,
                pressure_size: false,
                ..base
            })
        },
    );
    add(
        "soft-eraser",
        "Soft Eraser",
        "Erasers",
        b(BrushSettings {
            radius: 24.0,
            hardness: 0.0,
            spacing: 0.1,
            erase: true,
            pressure_opacity: false,
            ..base
        }),
    );
    add(
        "hard-eraser",
        "Hard Eraser",
        "Erasers",
        b(BrushSettings {
            radius: 10.0,
            hardness: 0.95,
            spacing: 0.08,
            erase: true,
            pressure_opacity: false,
            ..base
        }),
    );
    v
}
