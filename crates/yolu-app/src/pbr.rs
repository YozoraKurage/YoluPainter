//! 画像の名前の後ろの印から、PBR のチャンネルを当てる（名前だけを見る。画像の中身は見ない）。
//!
//! - ファイル名（拡張子を除く）を区切り（`_`・`-`・`.`・空白）で分け、最後の語（か、最後の 2 語をつないだ物。「Base Color」「Normal DX」）を
//!   大小文字を問わずに表と照らす。印の前の残りが「前の名前」で、同じ前の名前の画像を 1 組にする。
//! - ノーマルの向きは、既定が OpenGL（Y+）。名前に DirectX の印（`DX`・`DirectX`・`D3D`、`NormalDX` など）があれば、緑（Y）を反転して揃える。
//! - Glossiness・Gloss は Roughness へ反転して当てる。ORM・ARM（R = AO、G = Roughness、B = Metallic）は成分ごとに分ける（1 成分ずつ作る）。
//! - 印に当たるのは名前だけで、画像の中身が合っているかは見ない。
//! - 文書に標準のチャンネルが無い物（AO・Opacity）は `Role::channel` が None を返し、呼び出し側が普通の画像として扱う。

use std::collections::HashMap;

use yolu_core::Channel;

/// 画像が持つ意味（PBR のチャンネル）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Role {
    Color,
    Normal,
    Roughness,
    Metallic,
    Ao,
    Height,
    Emission,
    Opacity,
}

impl Role {
    /// 文書の標準のチャンネル（無い物は None。AO・Opacity）。
    pub fn channel(self) -> Option<Channel> {
        Some(match self {
            Role::Color => Channel::Color,
            Role::Normal => Channel::Normal,
            Role::Roughness => Channel::Roughness,
            Role::Metallic => Channel::Metallic,
            Role::Height => Channel::Height,
            Role::Emission => Channel::Emission,
            Role::Ao | Role::Opacity => return None,
        })
    }

    /// 色として読む画像か（浮動小数の画像を sRGB へ直し、棚の画像の色空間を sRGB にする）。それ以外はデータで、線形のまま。
    pub fn is_color(self) -> bool {
        matches!(self, Role::Color | Role::Emission)
    }

    /// 分けて作る画像の名前に付ける語。
    pub fn label(self) -> &'static str {
        match self {
            Role::Color => "BaseColor",
            Role::Normal => "Normal",
            Role::Roughness => "Roughness",
            Role::Metallic => "Metallic",
            Role::Ao => "AO",
            Role::Height => "Height",
            Role::Emission => "Emission",
            Role::Opacity => "Opacity",
        }
    }
}

/// 名前の印が示す中身。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// 1 つのチャンネル。`invert` は値を 1 − v に（Glossiness）、`flip_green` は緑を反転（DirectX のノーマル）。
    Single {
        role: Role,
        invert: bool,
        flip_green: bool,
    },
    /// ORM・ARM（R = AO、G = Roughness、B = Metallic）。
    Packed,
}

impl Kind {
    /// 1 枚が持つチャンネルの数（詰め込みは 3 つ）。
    pub fn channels(self) -> usize {
        match self {
            Kind::Single { .. } => 1,
            Kind::Packed => 3,
        }
    }

    /// 読むときの値の扱い（浮動小数の画像をどう 8 ビットにするか）で見る、色の画像か。詰め込みはデータ。
    pub fn is_color(self) -> bool {
        matches!(self, Kind::Single { role, .. } if role.is_color())
    }
}

/// 名前の判別の結果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    /// 印を除いた残りの名前（組の名前。空のこともある）。
    pub base: String,
    pub kind: Kind,
}

const SEPARATORS: [char; 4] = ['_', '-', '.', ' '];

fn is_separator(c: char) -> bool {
    SEPARATORS.contains(&c)
}

/// 区切りで分けた語（始まりのバイト位置と語）。
fn tokens(stem: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in stem.char_indices() {
        match (is_separator(c), start) {
            (false, None) => start = Some(i),
            (true, Some(s)) => {
                out.push((s, &stem[s..i]));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, &stem[s..]));
    }
    out
}

fn single(role: Role) -> Kind {
    Kind::Single {
        role,
        invert: false,
        flip_green: false,
    }
}

fn normal(flip_green: bool) -> Kind {
    Kind::Single {
        role: Role::Normal,
        invert: false,
        flip_green,
    }
}

/// 小文字の 1 語（か、つないだ 2 語）から印の中身。
fn lookup(word: &str) -> Option<Kind> {
    Some(match word {
        "basecolor" | "basecolour" | "albedo" | "diffuse" | "color" | "colour" | "col" => {
            single(Role::Color)
        }
        "normal" | "nrm" | "nor" | "normalgl" | "normalopengl" | "nrmgl" | "norgl" => normal(false),
        "normaldx" | "normaldirectx" | "nrmdx" | "nordx" => normal(true),
        "roughness" | "rough" | "rgh" => single(Role::Roughness),
        "glossiness" | "gloss" | "smoothness" => Kind::Single {
            role: Role::Roughness,
            invert: true,
            flip_green: false,
        },
        "metallic" | "metalness" | "metal" | "mtl" => single(Role::Metallic),
        "ao" | "ambientocclusion" | "occlusion" => single(Role::Ao),
        "height" | "displacement" | "disp" | "bump" => single(Role::Height),
        "emission" | "emissive" | "emit" => single(Role::Emission),
        "opacity" | "alpha" | "mask" => single(Role::Opacity),
        "orm" | "arm" | "occlusionroughnessmetallic" | "ambientocclusionroughnessmetallic" => {
            Kind::Packed
        }
        _ => return None,
    })
}

/// ノーマルの向きの印の語。`Some(true)` が DirectX（緑を反転）、`Some(false)` が OpenGL。
fn direction(word: &str) -> Option<bool> {
    match word.to_ascii_lowercase().as_str() {
        "dx" | "directx" | "d3d" => Some(true),
        "gl" | "opengl" => Some(false),
        _ => None,
    }
}

/// 名前（拡張子を除く）の末尾の印を見て、中身と前の名前を返す。印が無ければ None。
pub fn classify(stem: &str) -> Option<Mark> {
    let words = tokens(stem);
    let last = words.len().checked_sub(1)?;
    let joined = last
        .checked_sub(1)
        .and_then(|prev| lookup(&format!("{}{}", words[prev].1, words[last].1).to_lowercase()))
        .map(|kind| (last - 1, kind));
    // ノーマルの印の後ろに向きの印が付いた名前（「Normal_D3D」「Nrm_DirectX」）は、2 語をつながずに向きとして読む
    let directed = last.checked_sub(1).and_then(|prev| {
        let dx = direction(words[last].1)?;
        match lookup(&words[prev].1.to_lowercase())? {
            Kind::Single {
                role: Role::Normal,
                flip_green,
                ..
            } => Some((prev, normal(dx || flip_green))),
            _ => None,
        }
    });
    let (at, mut kind) = joined
        .or(directed)
        .or_else(|| lookup(&words[last].1.to_lowercase()).map(|kind| (last, kind)))?;
    let mut base = stem[..words[at].0].to_owned();
    // ノーマルの向きの印が印の前にもあれば（「Wood_DX_Normal」）、前の名前から外して向きにする
    if matches!(
        kind,
        Kind::Single {
            role: Role::Normal,
            ..
        }
    ) {
        let mut flip = matches!(
            kind,
            Kind::Single {
                flip_green: true,
                ..
            }
        );
        let mut cut = Vec::new();
        for (start, word) in &words[..at] {
            if let Some(dx) = direction(word) {
                flip |= dx;
                cut.push((*start, start + word.len()));
            }
        }
        for (from, to) in cut.into_iter().rev() {
            // 前の区切りも一緒に取る（先頭なら後ろの区切りを取る）
            let before = base[..from]
                .chars()
                .next_back()
                .filter(|c| is_separator(*c));
            let (from, to) = match before {
                Some(c) => (from - c.len_utf8(), to),
                None => (
                    from,
                    to + base[to..]
                        .chars()
                        .next()
                        .filter(|c| is_separator(*c))
                        .map_or(0, char::len_utf8),
                ),
            };
            base.replace_range(from..to, "");
        }
        kind = normal(flip);
    }
    let base = base.trim_matches(is_separator).to_owned();
    Some(Mark { base, kind })
}

/// 組の見分けの鍵（大小文字と区切りの違いを問わない）。
fn key_of(base: &str) -> String {
    base.chars()
        .map(|c| {
            if is_separator(c) {
                '_'
            } else {
                c.to_lowercase().next().unwrap_or(c)
            }
        })
        .collect()
}

/// 組の 1 枚。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    /// 落とした画像の並びでの番号。
    pub index: usize,
    pub kind: Kind,
}

/// 同じ前の名前の組（チャンネルが 2 つ以上ある物だけ）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    /// 組の名前（最初の画像の前の名前。空なら空文字）。
    pub name: String,
    pub members: Vec<Member>,
}

/// 判別の結果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub groups: Vec<Group>,
    /// 組にならなかった画像の番号（印が無い・組が 1 つのチャンネルだけ）。並びは落とした順。
    pub plain: Vec<usize>,
}

/// 落とした画像の名前（拡張子を除く）の並びを、組と普通の画像に分ける。組の並びは最初に出た順。
pub fn plan(stems: &[&str]) -> Plan {
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Group> = HashMap::new();
    let mut plain = Vec::new();
    for (index, stem) in stems.iter().enumerate() {
        let Some(mark) = classify(stem) else {
            plain.push(index);
            continue;
        };
        let key = key_of(&mark.base);
        let group = groups.entry(key.clone()).or_insert_with(|| {
            order.push(key);
            Group {
                name: mark.base.clone(),
                members: Vec::new(),
            }
        });
        group.members.push(Member {
            index,
            kind: mark.kind,
        });
    }
    let mut kept = Vec::new();
    for key in order {
        let group = groups.remove(&key).expect("順に入れた鍵");
        if group
            .members
            .iter()
            .map(|m| m.kind.channels())
            .sum::<usize>()
            >= 2
        {
            kept.push(group);
        } else {
            plain.extend(group.members.iter().map(|m| m.index));
        }
    }
    plain.sort_unstable();
    Plan {
        groups: kept,
        plain,
    }
}

// ── 画素の変換（straight RGBA8。アルファは触らない）──

/// 値を反転する（Glossiness → Roughness。R・G・B を 255 − v に）。
pub fn invert(rgba: &mut [u8]) {
    for px in rgba.as_chunks_mut::<4>().0.iter_mut() {
        px[0] = 255 - px[0];
        px[1] = 255 - px[1];
        px[2] = 255 - px[2];
    }
}

/// 緑（Y）を反転する（DirectX のノーマル → OpenGL のノーマル）。
pub fn flip_green(rgba: &mut [u8]) {
    for px in rgba.as_chunks_mut::<4>().0.iter_mut() {
        px[1] = 255 - px[1];
    }
}

/// ORM・ARM が持つチャンネル（R = AO、G = Roughness、B = Metallic の順）。
pub const PACKED: [Role; 3] = [Role::Ao, Role::Roughness, Role::Metallic];

/// ORM・ARM の成分 1 つ（`PACKED` の番号）を、灰色の画像（不透明）にする。3 枚を同時に持たないように、1 成分ずつ作る。
pub fn packed_part(rgba: &[u8], part: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.as_chunks::<4>().0.iter() {
        out.extend_from_slice(&[px[part], px[part], px[part], 255]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role_of(stem: &str) -> Option<(String, Role, bool, bool)> {
        classify(stem).and_then(|m| match m.kind {
            Kind::Single {
                role,
                invert,
                flip_green,
            } => Some((m.base, role, invert, flip_green)),
            Kind::Packed => None,
        })
    }

    #[test]
    fn every_mark_in_the_table_is_found_in_any_case_and_with_any_separator() {
        let table: [(&[&str], Role); 8] = [
            (
                &["BaseColor", "Albedo", "Diffuse", "Color", "Col"],
                Role::Color,
            ),
            (&["Normal", "Nrm", "Nor", "NormalGL"], Role::Normal),
            (&["Roughness", "Rough", "Rgh"], Role::Roughness),
            (&["Metallic", "Metalness", "Metal", "Mtl"], Role::Metallic),
            (&["AO", "AmbientOcclusion", "Occlusion"], Role::Ao),
            (&["Height", "Displacement", "Disp", "Bump"], Role::Height),
            (&["Emission", "Emissive", "Emit"], Role::Emission),
            (&["Opacity", "Alpha", "Mask"], Role::Opacity),
        ];
        for (marks, want) in table {
            for mark in marks {
                for sep in ["_", "-", ".", " "] {
                    for word in [mark.to_string(), mark.to_lowercase(), mark.to_uppercase()] {
                        let stem = format!("Wood{sep}{word}");
                        let (base, role, invert, flip) =
                            role_of(&stem).unwrap_or_else(|| panic!("{stem} は印が見つかる"));
                        assert_eq!((base.as_str(), role), ("Wood", want), "{stem}");
                        assert!(!invert && !flip, "{stem}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_name_without_a_mark_is_not_matched() {
        for stem in [
            "wood",
            "tip",
            "sky",
            "Wood_Colorful",
            "Watercolor",
            "",
            "_",
            "Normals_01",
        ] {
            assert_eq!(classify(stem), None, "{stem:?}");
        }
    }

    #[test]
    fn the_whole_name_can_be_the_mark_and_two_words_are_joined() {
        assert_eq!(role_of("Normal").unwrap().0, "");
        assert_eq!(role_of("Base Color").unwrap().0, "");
        assert_eq!(role_of("Rock_Base_Color").unwrap().0, "Rock");
        assert_eq!(role_of("Rock_Ambient_Occlusion").unwrap().1, Role::Ao);
        // 前の名前の語は変えない（大小文字・区切りもそのまま）
        assert_eq!(role_of("Brick.Wall-01_Albedo").unwrap().0, "Brick.Wall-01");
    }

    #[test]
    fn glossiness_is_matched_as_an_inverted_roughness() {
        for stem in ["Wood_Glossiness", "Wood_gloss", "Wood_SMOOTHNESS"] {
            assert_eq!(
                role_of(stem),
                Some(("Wood".into(), Role::Roughness, true, false)),
                "{stem}"
            );
        }
    }

    #[test]
    fn a_directx_mark_flips_the_green_of_a_normal_and_is_removed_from_the_group_name() {
        for stem in [
            "Wood_NormalDX",
            "Wood_Normal_DX",
            "Wood_normal-dx",
            "Wood_DX_Normal",
            "Wood_DirectX_Normal",
            "Wood_Normal_DirectX",
            "Wood nor dx",
            "Wood_D3D_Nrm",
            "Wood_Normal_D3D",
            "Wood_Nrm_DirectX",
            "Wood-Nrm-D3D",
            "Wood_Nor_DX",
        ] {
            assert_eq!(
                role_of(stem),
                Some(("Wood".into(), Role::Normal, false, true)),
                "{stem}"
            );
        }
        for stem in [
            "Wood_NormalGL",
            "Wood_Normal_GL",
            "Wood_OpenGL_Normal",
            "Wood_Normal",
            "Wood_Nor_GL",
            "Wood_Nrm_OpenGL",
        ] {
            assert_eq!(
                role_of(stem),
                Some(("Wood".into(), Role::Normal, false, false)),
                "{stem}"
            );
        }
        // 向きの語が先頭にあっても、前の名前に残らない
        assert_eq!(role_of("DX_Normal").unwrap().0, "");
        // ノーマルでない印の前の DX は印でない（前の名前の一部）
        assert_eq!(role_of("Wood_DX_Color").unwrap().0, "Wood_DX");
        // 向きの語だけが付いた、ノーマルでない名前は印でない
        assert_eq!(classify("Wood_Color_DX"), None);
        assert_eq!(classify("Wood_DX"), None);
    }

    #[test]
    fn packed_marks_are_matched_and_split_into_ao_roughness_and_metallic() {
        for stem in ["Wood_ORM", "Wood_arm", "Wood_OcclusionRoughnessMetallic"] {
            let mark = classify(stem).unwrap_or_else(|| panic!("{stem}"));
            assert_eq!(
                (mark.base.as_str(), mark.kind),
                ("Wood", Kind::Packed),
                "{stem}"
            );
        }
        let rgba = [10, 20, 30, 7, 200, 100, 50, 255];
        assert_eq!(PACKED, [Role::Ao, Role::Roughness, Role::Metallic]);
        assert_eq!(
            packed_part(&rgba, 0),
            vec![10, 10, 10, 255, 200, 200, 200, 255]
        );
        assert_eq!(
            packed_part(&rgba, 1),
            vec![20, 20, 20, 255, 100, 100, 100, 255]
        );
        assert_eq!(
            packed_part(&rgba, 2),
            vec![30, 30, 30, 255, 50, 50, 50, 255]
        );
    }

    #[test]
    fn pixel_conversions_keep_the_alpha() {
        let mut px = [10, 20, 30, 77];
        invert(&mut px);
        assert_eq!(px, [245, 235, 225, 77]);
        let mut px = [10, 20, 30, 77];
        flip_green(&mut px);
        assert_eq!(px, [10, 235, 30, 77]);
    }

    #[test]
    fn images_with_the_same_front_name_become_a_group_and_the_rest_stay_plain() {
        let plan = plan(&[
            "Wood_BaseColor",
            "tip",
            "wood-normal",
            "Wood Roughness",
            "Rock_Albedo",
            "Stone_ORM",
            "Stone_Color",
        ]);
        assert_eq!(
            plan.plain,
            [1, 4],
            "印の無い画像と、1 つのチャンネルだけの組"
        );
        assert_eq!(plan.groups.len(), 2);
        let wood = &plan.groups[0];
        assert_eq!(wood.name, "Wood");
        assert_eq!(
            wood.members.iter().map(|m| m.index).collect::<Vec<_>>(),
            [0, 2, 3]
        );
        let stone = &plan.groups[1];
        assert_eq!(stone.name, "Stone");
        assert_eq!(stone.members.len(), 2);
    }

    #[test]
    fn one_packed_image_alone_is_a_group_of_three_channels() {
        let plan = plan(&["Wood_ORM", "other"]);
        assert_eq!(plan.groups.len(), 1);
        assert_eq!(plan.plain, [1]);
    }
}
