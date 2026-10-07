//! 同梱のスマートマテリアル。塗りつぶしレイヤー（チャンネルごとの値）と、そのマスクのノイズ・グランジの Generator だけで組む
//! （外の画像を使わない。画素のレイヤーも持たない）ので、許諾の問題が無く、どの大きさのテクスチャにも置ける。置くと文書へレイヤーの組として写り、
//! その後は普通のレイヤーと同じように直せる。
//!
//! ここにはマップ（焼き）が要る Generator（エッジの摩耗・汚れ）を入れない。入れた文書は、焼く前に開くと入力が足りない効果として
//! 読むだけになるので、同梱の素材はモデルが無くても・焼く前でも同じ見た目で、いつも編集できる。位置のマップが使えるときは、
//! ノイズ・グランジは位置から 3D で評価され（UV アイランドの継ぎ目で模様がずれない）、使えないときは UV に落ちる。
//!
//! 絵の良し悪しは人が決める: ここの値（色・粗さ・模様の大きさ）は出発点で、サムネイルを見て直す。
use crate::generator::{Blend, GrungePreset, Kind, NoiseBasis, Settings};
use crate::smart::SmartMaterial;
use crate::{Channel, CoreError, Document, EffectSettings, FilterSpec, FilterTarget, Rgba8};

/// 素材の文書の 1 辺（画素）。レイヤーに画素が無い（値とマスクの Generator だけ）ので、置く先の大きさに依らない。
pub const SIZE: u32 = 128;

/// 同梱の素材 1 つ。
#[derive(Clone, Copy)]
pub struct Entry {
    /// 変わらない識別子（棚の項目の ID の後ろ）。
    pub id: &'static str,
    pub ja: &'static str,
    pub en: &'static str,
    parts: fn() -> Vec<Part>,
}
impl Entry {
    pub fn name(&self, japanese: bool) -> &'static str {
        if japanese {
            self.ja
        } else {
            self.en
        }
    }
    /// 素材を組む。`name` は置いたときのグループの名前、`japanese` はレイヤーの名前の言語。
    pub fn build(&self, name: &str, japanese: bool) -> Result<SmartMaterial, CoreError> {
        let mut doc = Document::with_tile_size(SIZE, SIZE, 64)?;
        let mut ids = Vec::new();
        for part in (self.parts)() {
            let value = |v: u8| Rgba8::new(v, v, v, 255);
            let layer = doc.add_fill_layer(
                if japanese { part.ja } else { part.en },
                &[
                    (
                        Channel::Color,
                        Rgba8::new(part.color[0], part.color[1], part.color[2], 255),
                    ),
                    (Channel::Roughness, value(part.rough)),
                    (Channel::Metallic, value(part.metal)),
                    (Channel::Height, value(part.height)),
                ],
                None,
            )?;
            if part.opacity < 1. {
                doc.set_layer_opacity(layer, part.opacity, false)?;
            }
            if !part.mask.is_empty() {
                doc.add_layer_mask(layer)?;
                for settings in part.mask {
                    doc.add_filter(
                        layer,
                        FilterTarget::Mask,
                        FilterSpec::new(EffectSettings::generator(settings)),
                    )?;
                }
            }
            ids.push(layer);
        }
        doc.capture_smart_material(&ids, name)
    }
}

/// 同梱の素材の一覧（棚の「組み込み」に並べる順）。
pub fn entries() -> &'static [Entry] {
    &ENTRIES
}

struct Part {
    ja: &'static str,
    en: &'static str,
    color: [u8; 3],
    rough: u8,
    metal: u8,
    height: u8,
    opacity: f64,
    /// マスクの Generator の段（下から）。空なら全面に出る土台のレイヤー。
    mask: Vec<Settings>,
}
impl Part {
    fn new(
        names: (&'static str, &'static str),
        color: [u8; 3],
        rough: u8,
        metal: u8,
        height: u8,
    ) -> Self {
        Self {
            ja: names.0,
            en: names.1,
            color,
            rough,
            metal,
            height,
            opacity: 1.,
            mask: Vec::new(),
        }
    }
    fn opacity(mut self, o: f64) -> Self {
        self.opacity = o;
        self
    }
    fn mask(mut self, stage: Settings) -> Self {
        self.mask.push(stage);
        self
    }
}

/// マスクの最初の段（見える量を置き換える）のグランジ。
fn grunge(preset: GrungePreset, scale: f64, seed: i32) -> Settings {
    let mut s = Settings::grunge(preset);
    s.procedural.scale = scale;
    s.procedural.seed = seed;
    s.blend = Blend::Replace;
    s
}
/// マスクの最初の段のノイズ（fBm）。
fn noise(basis: NoiseBasis, scale: f64, seed: i32, octaves: u32) -> Settings {
    let mut s = Settings::new(Kind::Noise);
    s.procedural.basis = basis;
    s.procedural.scale = scale;
    s.procedural.seed = seed;
    s.procedural.octaves = octaves;
    s.blend = Blend::Replace;
    s
}
trait Shape {
    fn levels(self, low: f64, high: f64) -> Self;
    fn inverted(self) -> Self;
}
impl Shape for Settings {
    fn levels(mut self, low: f64, high: f64) -> Self {
        self.low = low;
        self.high = high;
        self
    }
    fn inverted(mut self) -> Self {
        self.invert = true;
        self
    }
}

static ENTRIES: [Entry; 14] = [
    Entry {
        id: "rusty-iron",
        ja: "錆びた鉄",
        en: "Rusty Iron",
        parts: rusty_iron,
    },
    Entry {
        id: "chipped-paint-metal",
        ja: "塗装の剥げ",
        en: "Worn Paint",
        parts: chipped_paint_metal,
    },
    Entry {
        id: "dusty-plastic",
        ja: "ほこりのプラ",
        en: "Dusty Plastic",
        parts: dusty_plastic,
    },
    Entry {
        id: "worn-leather",
        ja: "使い込んだ革",
        en: "Worn Leather",
        parts: worn_leather,
    },
    Entry {
        id: "woven-cloth",
        ja: "織りの布",
        en: "Woven Cloth",
        parts: woven_cloth,
    },
    Entry {
        id: "ceramic",
        ja: "陶器",
        en: "Ceramic",
        parts: ceramic,
    },
    Entry {
        id: "wood",
        ja: "木",
        en: "Wood",
        parts: wood,
    },
    Entry {
        id: "gold-trim",
        ja: "金の装飾",
        en: "Gold Trim",
        parts: gold_trim,
    },
    Entry {
        id: "rubber",
        ja: "ゴム",
        en: "Rubber",
        parts: rubber,
    },
    Entry {
        id: "dirty-cloth",
        ja: "汚れた布",
        en: "Dirty Cloth",
        parts: dirty_cloth,
    },
    Entry {
        id: "concrete",
        ja: "コンクリート",
        en: "Concrete",
        parts: concrete,
    },
    Entry {
        id: "scratched-steel",
        ja: "傷だらけの鋼",
        en: "Scored Steel",
        parts: scratched_steel,
    },
    Entry {
        id: "mossy-stone",
        ja: "苔むした石",
        en: "Mossy Stone",
        parts: mossy_stone,
    },
    Entry {
        id: "mud-splatter",
        ja: "泥はね",
        en: "Mud Splatter",
        parts: mud_splatter,
    },
];

use GrungePreset as P;
use NoiseBasis as B;

fn rusty_iron() -> Vec<Part> {
    vec![
        Part::new(("地金", "Iron"), [88, 90, 96], 150, 255, 140),
        Part::new(("錆", "Rust"), [118, 58, 26], 220, 0, 100).mask(grunge(P::Rust, 0.22, 11)),
        Part::new(("錆の粒", "Rust pits"), [78, 34, 14], 235, 0, 80).mask(grunge(
            P::Splatter,
            0.12,
            3,
        )),
        Part::new(("傷", "Scratches"), [160, 160, 166], 100, 255, 150)
            .opacity(0.8)
            .mask(grunge(P::Scratches, 0.2, 7)),
    ]
}
fn chipped_paint_metal() -> Vec<Part> {
    vec![
        Part::new(("地金", "Bare metal"), [132, 134, 140], 90, 255, 120),
        Part::new(("塗装", "Paint"), [170, 36, 32], 115, 0, 150),
        Part::new(("剥げ", "Chips"), [136, 138, 144], 85, 255, 118)
            .mask(grunge(P::Peeling, 0.3, 4).levels(0.575, 0.59)),
        Part::new(("傷", "Scratches"), [200, 200, 205], 70, 255, 140)
            .opacity(0.7)
            .mask(grunge(P::Scratches, 0.2, 9)),
    ]
}
fn dusty_plastic() -> Vec<Part> {
    vec![
        Part::new(("プラスチック", "Plastic"), [205, 208, 212], 105, 0, 128),
        Part::new(("ほこり", "Dust"), [168, 162, 150], 245, 0, 132).mask(grunge(P::Dust, 0.1, 2)),
        Part::new(("指紋", "Fingerprints"), [190, 192, 196], 60, 0, 128)
            .opacity(0.5)
            .mask(grunge(P::Fingerprints, 0.12, 5)),
    ]
}
fn worn_leather() -> Vec<Part> {
    vec![
        Part::new(("革", "Leather"), [92, 52, 30], 170, 0, 128),
        Part::new(("しぼ", "Grain"), [70, 38, 22], 190, 0, 95)
            .mask(grunge(P::Pebbles, 0.04, 3).inverted()),
        Part::new(("擦れ", "Scuffs"), [150, 98, 60], 110, 0, 135)
            .mask(grunge(P::Stain, 0.3, 6).levels(0.45, 0.85)),
        Part::new(("ひび", "Cracks"), [50, 28, 16], 200, 0, 85)
            .opacity(0.6)
            .mask(grunge(P::Cracks, 0.2, 8)),
    ]
}
fn woven_cloth() -> Vec<Part> {
    vec![
        Part::new(("布地", "Fabric"), [46, 66, 110], 230, 0, 128),
        Part::new(("織り目", "Weave"), [30, 44, 80], 245, 0, 100)
            .mask(grunge(P::Weave, 0.1, 1).inverted()),
        Part::new(("毛羽", "Fuzz"), [120, 138, 178], 250, 0, 130)
            .opacity(0.35)
            .mask(grunge(P::Dust, 0.05, 4).levels(0.5, 1.0)),
    ]
}
fn ceramic() -> Vec<Part> {
    vec![
        Part::new(("釉薬", "Glaze"), [238, 236, 228], 35, 0, 128),
        Part::new(("ゆず肌", "Orange peel"), [225, 223, 214], 60, 0, 118)
            .opacity(0.6)
            .mask(noise(B::Value, 0.02, 2, 3).levels(0.35, 0.65)),
        Part::new(("貫入", "Crazing"), [120, 112, 100], 90, 0, 100)
            .opacity(0.35)
            .mask(grunge(P::Cracks, 0.12, 3)),
        Part::new(("汚れ", "Stains"), [190, 180, 160], 120, 0, 125)
            .opacity(0.35)
            .mask(grunge(P::Stain, 0.3, 5).levels(0.6, 0.9)),
    ]
}
fn wood() -> Vec<Part> {
    vec![
        Part::new(("木地", "Wood"), [170, 120, 70], 180, 0, 128),
        Part::new(("年輪", "Rings"), [110, 70, 36], 200, 0, 112).mask(grunge(P::WoodGrain, 0.2, 1)),
        Part::new(("濃淡", "Tone"), [90, 56, 30], 210, 0, 105)
            .opacity(0.5)
            .mask(noise(B::Perlin, 0.4, 7, 4).levels(0.55, 0.8)),
        Part::new(("ニスの擦れ", "Varnish wear"), [200, 150, 95], 110, 0, 130)
            .opacity(0.6)
            .mask(grunge(P::Scratches, 0.2, 9)),
    ]
}
fn gold_trim() -> Vec<Part> {
    vec![
        Part::new(("金", "Gold"), [255, 195, 86], 70, 255, 128),
        Part::new(
            ("細かい傷", "Fine scratches"),
            [230, 175, 70],
            110,
            255,
            128,
        )
        .mask(grunge(P::Scratches, 0.25, 5)),
        Part::new(("くすみ", "Tarnish"), [150, 110, 48], 150, 255, 120)
            .opacity(0.5)
            .mask(grunge(P::Stain, 0.25, 3).levels(0.55, 0.9)),
    ]
}
fn rubber() -> Vec<Part> {
    vec![
        Part::new(("ゴム", "Rubber"), [30, 30, 32], 215, 0, 128),
        Part::new(("きめ", "Texture"), [22, 22, 24], 235, 0, 112)
            .opacity(0.6)
            .mask(grunge(P::Pebbles, 0.02, 4).levels(0.0, 0.7)),
        Part::new(("白化", "Bloom"), [70, 70, 72], 250, 0, 130)
            .opacity(0.5)
            .mask(grunge(P::Dust, 0.1, 6).levels(0.35, 0.8)),
    ]
}
fn dirty_cloth() -> Vec<Part> {
    vec![
        Part::new(("布地", "Fabric"), [200, 196, 184], 235, 0, 128),
        Part::new(("織り目", "Weave"), [150, 146, 134], 250, 0, 105)
            .mask(grunge(P::Weave, 0.1, 1).inverted()),
        Part::new(("染み", "Stains"), [110, 84, 50], 245, 0, 125)
            .opacity(0.8)
            .mask(grunge(P::Stain, 0.3, 8).levels(0.4, 0.8)),
        Part::new(("泥はね", "Mud"), [70, 52, 32], 250, 0, 135).mask(grunge(P::Splatter, 0.12, 2)),
    ]
}
fn concrete() -> Vec<Part> {
    vec![
        Part::new(("コンクリート", "Concrete"), [140, 140, 136], 215, 0, 128),
        Part::new(("むら", "Mottling"), [110, 110, 106], 230, 0, 118)
            .opacity(0.6)
            .mask(noise(B::Perlin, 0.25, 3, 5).levels(0.4, 0.75)),
        Part::new(("気泡", "Air pits"), [90, 90, 88], 240, 0, 70)
            .opacity(0.8)
            .mask(grunge(P::Splatter, 0.08, 5)),
        Part::new(("ひび", "Cracks"), [60, 60, 58], 245, 0, 60)
            .opacity(0.45)
            .mask(grunge(P::Cracks, 0.3, 9)),
    ]
}
fn scratched_steel() -> Vec<Part> {
    vec![
        Part::new(("鋼", "Steel"), [150, 152, 158], 80, 255, 128),
        Part::new(
            ("細かい傷", "Fine scratches"),
            [200, 200, 206],
            130,
            255,
            128,
        )
        .mask(grunge(P::Scratches, 0.2, 3)),
        Part::new(("深い傷", "Deep scratches"), [110, 112, 118], 160, 255, 100)
            .mask(grunge(P::Scratches, 0.12, 6).levels(0.5, 1.0)),
        Part::new(("くもり", "Haze"), [120, 120, 126], 170, 255, 128)
            .opacity(0.35)
            .mask(grunge(P::Stain, 0.3, 4)),
    ]
}
fn mossy_stone() -> Vec<Part> {
    vec![
        Part::new(("石", "Stone"), [118, 116, 110], 220, 0, 128),
        Part::new(("石の粒", "Grain"), [90, 88, 84], 235, 0, 100)
            .opacity(0.6)
            .mask(noise(B::Worley, 0.06, 2, 2).levels(0.3, 0.7)),
        Part::new(("苔", "Moss"), [62, 92, 40], 245, 0, 150)
            .mask(noise(B::Perlin, 0.25, 5, 5).levels(0.5, 0.85)),
        Part::new(("ひび", "Cracks"), [50, 48, 46], 240, 0, 80)
            .opacity(0.6)
            .mask(grunge(P::Cracks, 0.15, 7)),
    ]
}
fn mud_splatter() -> Vec<Part> {
    vec![
        Part::new(("泥", "Mud"), [88, 64, 40], 250, 0, 150).mask(grunge(P::Splatter, 0.12, 4)),
        Part::new(("乾いた泥", "Dried mud"), [130, 104, 74], 255, 0, 160)
            .opacity(0.8)
            .mask(grunge(P::Stain, 0.3, 2).levels(0.55, 0.9)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_builds_in_both_languages_with_only_values_and_masks() {
        let mut ids = std::collections::HashSet::new();
        for e in entries() {
            assert!(ids.insert(e.id), "{}", e.id);
            for ja in [true, false] {
                let m = e.build(e.name(ja), ja).unwrap();
                assert_eq!(m.name(), e.name(ja));
                assert!(m.layers().len() >= 2, "{}", e.id);
                assert_eq!((m.width(), m.height()), (SIZE, SIZE));
                // 画素のレイヤーを持たない（値とマスクの Generator だけ）
                assert_eq!(m.pixel_bytes(), 0, "{}: 画素が入っている", e.id);
            }
        }
        assert_eq!(ids.len(), 14);
    }
}
