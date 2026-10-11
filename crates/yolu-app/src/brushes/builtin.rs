//! 組み込みのブラシ（消せない元）。core の組み込み 13 個（`m2::presets()`）に、各ツールの「標準」と効果のブラシ 3 つと、
//! 厚塗りの筆 3 つ（下の色を拾って混ぜる。`oil`・`gouache`・`mixer`）を足し、
//! グループ（ペン・筆・エアブラシ・消しゴム・効果・特殊）に分けたもの。ここに無い id の組み込みは作らない（保存した並びの
//! 読み戻しは、ここに載っている id だけを組み込みと見る）。縁の硬い丸い筆先（ペン・筆・消しゴムのグループで硬さ 0.5 以上）は、
//! 縁のアンチエイリアスを 中 にする（[`hard_edge`]。core の組み込みの値は なし のまま）。

use std::sync::OnceLock;

use super::{canonical, Group};
use crate::engine::{
    AntiAlias, Brush, BrushEffect, BrushSettings, ColorMix, DVec2, MixMode, PressureResponse,
};
use crate::lang::Lang;
use crate::m2;

/// 組み込みの 1 つ。`brush` は正規の形（`canonical`）。
pub struct Builtin {
    pub id: &'static str,
    pub group: Group,
    pub brush: Brush,
}

/// 今の画面の既定の設定（描くツールの「標準」。起動直後の設定と同じで、選んでも何も変わらない）。
pub const STANDARD: &str = "standard";
/// 消しゴムの「標準」（設定は描くツールの標準と同じ。消しゴムのグループの先頭）。
pub const STANDARD_ERASER: &str = "standard-eraser";

/// 縁の硬い丸い筆先の描くブラシか（ペン・筆・消しゴムのグループで、画像の筆先でなく硬さ 0.5 以上）。縁が画素の段になりやすいので、
/// 組み込みの既定のアンチエイリアスを 中 にする。柔らかい筆先は帯がぼかしの幅より細いので、どの段でも同じ画素になる（なし のまま）。
pub(crate) fn hard_edge(group: Group, brush: &Brush) -> bool {
    matches!(group, Group::Pen | Group::Brush | Group::Eraser)
        && brush.tip.image.is_none()
        && brush.tip.images.is_empty()
        && brush.effect.is_paint()
        && brush.base.hardness >= 0.5
}

/// core の組み込みの id が属するグループ。
fn preset_group(id: &str) -> Group {
    match id {
        "hard-round" | "ink-pen" | "pencil" | "marker" => Group::Pen,
        "dry-brush" | "watercolor" | "chalk" | "charcoal" => Group::Brush,
        "soft-round" | "airbrush" => Group::Airbrush,
        "soft-eraser" | "hard-eraser" => Group::Eraser,
        _ => Group::Special,
    }
}

/// 組み込みの全部（グループごとに画面の並びで）。初めて呼んだときに 1 回だけ作る。
pub fn all() -> &'static [Builtin] {
    static ALL: OnceLock<Vec<Builtin>> = OnceLock::new();
    ALL.get_or_init(|| {
        let presets = m2::presets();
        let preset = |id: &str| {
            presets
                .iter()
                .find(|p| p.id == id)
                .map(|p| canonical(&p.brush))
                .expect("core の組み込みのブラシ")
        };
        let standard = canonical(&Brush::default());
        let effect = |radius: f64, hardness: f64, flow: f64, effect: BrushEffect| {
            canonical(&Brush {
                effect,
                ..Brush::from(BrushSettings {
                    radius,
                    hardness,
                    flow,
                    spacing: 0.1,
                    ..BrushSettings::default()
                })
            })
        };
        // 厚塗りの筆（色の混ぜ。筆のグループ）
        let thick = |radius: f64, hardness: f64, spacing: f64, flow: f64, mix: ColorMix| {
            canonical(&Brush {
                mix,
                ..Brush::from(BrushSettings {
                    radius,
                    hardness,
                    spacing,
                    flow,
                    // 置いた絵の具は筆圧で薄くならない（量・濃さのほうを筆圧に従わせる）
                    pressure_opacity: false,
                    ..BrushSettings::default()
                })
            })
        };
        let mut oil = thick(
            20.0,
            0.7,
            0.05,
            1.0,
            ColorMix {
                mode: MixMode::Mix,
                paint: 0.65,
                density: 1.0,
                stretch: 0.6,
                pressure_paint: true,
                response_paint: PressureResponse::new(0.3, Vec::new())
                    .expect("最小値だけの筆圧の応え"),
                ..ColorMix::default()
            },
        );
        oil.tip.image = yolu_core::builtin_tip("bristles");
        oil.tip.follow_direction = true;
        let oil = canonical(&oil);
        let gouache = thick(
            24.0,
            0.85,
            0.1,
            0.9,
            ColorMix {
                mode: MixMode::Mix,
                paint: 0.85,
                density: 0.95,
                stretch: 0.25,
                ..ColorMix::default()
            },
        );
        let mixer = thick(
            26.0,
            0.4,
            0.06,
            0.7,
            ColorMix {
                mode: MixMode::Smear,
                paint: 0.2,
                density: 0.85,
                stretch: 0.7,
                pressure_density: true,
                response_density: PressureResponse::new(0.25, Vec::new())
                    .expect("最小値だけの筆圧の応え"),
                ..ColorMix::default()
            },
        );
        let mut v = Vec::new();
        let mut add = |id: &'static str, group: Group, mut brush: Brush| {
            if hard_edge(group, &brush) {
                brush.base.anti_alias = AntiAlias::Medium;
            }
            v.push(Builtin { id, group, brush })
        };
        add(STANDARD, Group::Pen, standard.clone());
        for id in ["hard-round", "ink-pen", "pencil", "marker"] {
            add(id, preset_group(id), preset(id));
        }
        for id in ["dry-brush", "watercolor", "chalk", "charcoal"] {
            add(id, preset_group(id), preset(id));
        }
        add("oil", Group::Brush, oil);
        add("gouache", Group::Brush, gouache);
        add("mixer", Group::Brush, mixer);
        for id in ["soft-round", "airbrush"] {
            add(id, preset_group(id), preset(id));
        }
        add(STANDARD_ERASER, Group::Eraser, standard);
        for id in ["soft-eraser", "hard-eraser"] {
            add(id, preset_group(id), preset(id));
        }
        add(
            "blur",
            Group::Effect,
            effect(20.0, 0.5, 0.6, BrushEffect::BLUR),
        );
        add(
            "smudge",
            Group::Effect,
            effect(16.0, 0.5, 1.0, BrushEffect::SMUDGE),
        );
        add(
            "clone",
            Group::Effect,
            effect(
                20.0,
                0.7,
                1.0,
                BrushEffect::Clone {
                    offset: DVec2::new(64.0, 0.0),
                },
            ),
        );
        add("splatter", preset_group("splatter"), preset("splatter"));
        v
    })
}

/// id の組み込み。
pub fn find(id: &str) -> Option<&'static Builtin> {
    all().iter().find(|b| b.id == id)
}

/// 組み込みの表示名。
pub fn name(lang: Lang, id: &str) -> String {
    let own = match id {
        STANDARD | STANDARD_ERASER => Some(lang.pick("標準", "Standard")),
        "blur" => Some(lang.pick("ぼかし", "Blur")),
        "smudge" => Some(lang.pick("指先", "Smudge")),
        "clone" => Some(lang.pick("クローン", "Clone")),
        "oil" => Some(lang.pick("油彩", "Oil paint")),
        "gouache" => Some(lang.pick("ガッシュ", "Gouache")),
        "mixer" => Some(lang.pick("混色", "Mixer")),
        _ => None,
    };
    match own {
        Some(name) => name.to_owned(),
        None => m2::presets()
            .iter()
            .find(|p| p.id == id)
            .map(|p| m2::preset_label(lang, p).0.to_owned())
            .unwrap_or_else(|| id.to_owned()),
    }
}
