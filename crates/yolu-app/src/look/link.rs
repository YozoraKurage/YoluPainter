//! Live Link で受けたマテリアルの値（Unity の本物の lilToon のマテリアル）を、テクスチャセットの「受けた見た目」にする。
//!
//! 利用者の設定と、Unity から来た値のどちらで描くかの決まり:
//! - Unity から来た値は、そのマテリアルのテクスチャセットの文書の「受けた見た目」（`Document::set_received_look`）として持つ。Undo にも
//!   版にも入らない（受け取りは利用者の操作ではなく、Undo で戻すと Unity の本物と食い違う）。
//! - 描く見た目は、受けた値の上に利用者の設定を重ねたもの（`MaterialLook::over`）。欄で変えた項目（利用者の設定にある項目）だけが勝ち、
//!   ほかは Unity の値で描く。描き方（標準・lilToon）は、欄で選んでいなければ Unity の値（lilToon）。
//! - lilToon でないマテリアル（[`is_liltoon`]。名前だけでは決めない）は、受けた見た目を外す（値は描かない）。
//! - 描いていないスロットの絵は、頼みの絵のファイルから読んで受けた見た目の中に持ち（保存しない。開き直すとファイルから読み直す）、
//!   全部のマテリアルの合計を [`MAX_RECEIVED_IMAGE_BYTES`] までにする。スタンドアロンが描く絵で見せるスロット（[`SHOWN`]。Color の
//!   `_MainTex`）は、絵ではなくそのチャンネルで描く。

use std::collections::BTreeMap;
use std::sync::Arc;

use yolu_core::look::{
    LookKind, LookValue, MaterialLook, MissingImage, ReceivedImage, ReceivedLook, TextureSource,
    MAX_KEYWORDS, MAX_NAME, MAX_PROPERTIES, MAX_SHADER_NAME, MAX_TEXTURES,
};
use yolu_core::Channel;
use yolu_protocol::files::Material;

/// 受けた絵の画素のバイトの合計の上限（全部のマテリアル。超える絵は持たず、スロットは「予算を超える」）。
pub const MAX_RECEIVED_IMAGE_BYTES: u64 = 256 << 20;

/// 受けた絵の辺の上限（大きな絵は縮めて持つ）。
pub const MAX_RECEIVED_SIDE: u32 = 2048;

/// スタンドアロンが描く絵で見せるスロット（チャンネルとプロパティ）。ほかのスロットは Unity の絵（ファイル）で見せる。
pub const SHOWN: &[(Channel, &str)] = &[(Channel::Color, "_MainTex")];

/// lilToon のマテリアルが持つ版の値（lilToon のシェーダーが宣言する隠しのプロパティ）。
pub const LILTOON_VERSION_PROPERTY: &str = "_lilToonVersion";

/// lilToon の UPM パッケージの名前。
pub const LILTOON_PACKAGE: &str = "jp.lilxyzw.liltoon";

/// lilToon のマテリアルか。シェーダーの名前だけでは決めない（似た名前のシェーダーを lilToon として描かない）: 値に
/// [`LILTOON_VERSION_PROPERTY`] がある（lilToon のシェーダーが宣言する）か、シェーダーが lilToon のパッケージ（[`LILTOON_PACKAGE`]）の物のとき。
pub fn is_liltoon(material: &Material) -> bool {
    let v = &material.values;
    v.floats.contains_key(LILTOON_VERSION_PROPERTY)
        || v.ints.contains_key(LILTOON_VERSION_PROPERTY)
        || material.shader.package == LILTOON_PACKAGE
}

/// 名前が見た目の設定に入る形か（UTF-16 で 1〜`max` 文字、制御文字なし）。
fn name_fits(s: &str, max: usize) -> bool {
    let n = s.encode_utf16().count();
    n >= 1 && n <= max && !s.chars().any(char::is_control)
}

/// 人に見せる文を見た目の設定に入る形にする（制御文字を除き、`max` 文字まで）。
fn clean_text(s: &str, max: usize) -> String {
    let mut out = String::new();
    for c in s.chars().filter(|c| !c.is_control()) {
        if out.encode_utf16().count() + c.len_utf16() > max {
            break;
        }
        out.push(c);
    }
    out
}

/// 受けたマテリアルから、受けた見た目を作る。`images` は絵のファイルから読んだスロットの絵、`missing` は絵の無いスロットの理由。
/// lilToon でなければ None（受けた見た目を外す）。見た目の設定に入らない名前（長すぎる・制御文字）は落とす。テクスチャの拡大と
/// ずらし（`scale`・`offset`）は、値に `<スロット>_ST` が無いときだけ値にする。
pub fn received_look(
    material: &Material,
    images: &BTreeMap<String, Arc<ReceivedImage>>,
    missing: &BTreeMap<String, MissingImage>,
) -> Option<ReceivedLook> {
    let shader = &material.shader;
    if !is_liltoon(material) {
        return None;
    }
    let fits = |s: &str| name_fits(s, MAX_NAME);
    let mut look = MaterialLook {
        kind: LookKind::LilToon,
        shader: if name_fits(&shader.name, MAX_SHADER_NAME) {
            shader.name.clone()
        } else {
            String::new()
        },
        shader_guid: if fits(&shader.guid) {
            shader.guid.clone()
        } else {
            String::new()
        },
        shader_version: if fits(&shader.version) {
            shader.version.clone()
        } else {
            String::new()
        },
        render_queue: shader.render_queue,
        ..MaterialLook::default()
    };
    let v = &material.values;
    let entries = v
        .floats
        .iter()
        .map(|(k, x)| (k, LookValue::Float(*x)))
        .chain(v.ints.iter().map(|(k, x)| (k, LookValue::Int(*x))))
        .chain(v.colors.iter().map(|(k, x)| (k, LookValue::Color(*x))))
        .chain(v.vectors.iter().map(|(k, x)| (k, LookValue::Vector(*x))));
    for (name, value) in entries {
        if look.properties.len() < MAX_PROPERTIES && fits(name) && value.is_finite() {
            look.properties.insert(name.clone(), value);
        }
    }
    for t in &material.textures {
        let st = format!("{}_ST", t.property);
        if look.properties.len() < MAX_PROPERTIES && fits(&st) && !look.properties.contains_key(&st)
        {
            let value = LookValue::Vector([t.scale[0], t.scale[1], t.offset[0], t.offset[1]]);
            if value.is_finite() {
                look.properties.insert(st, value);
            }
        }
    }
    for k in &shader.keywords {
        if look.keywords.len() < MAX_KEYWORDS
            && fits(k)
            && !k.contains(' ')
            && !look.keywords.contains(k)
        {
            look.keywords.push(k.clone());
        }
    }
    for (channel, property) in SHOWN {
        look.textures
            .insert((*property).to_owned(), TextureSource::Channel(*channel));
    }
    let source = clean_text(
        format!("{} {}", shader.name, shader.version).trim(),
        MAX_SHADER_NAME,
    );
    let mut out = ReceivedLook {
        look,
        source,
        ..ReceivedLook::default()
    };
    for t in &material.textures {
        let slot = &t.property;
        if out.look.textures.contains_key(slot)
            || !fits(slot)
            || out.images.len() + out.missing.len() >= MAX_TEXTURES
        {
            continue;
        }
        if let Some(image) = images.get(slot) {
            out.images.insert(slot.clone(), image.clone());
        } else if let Some(why) = missing.get(slot) {
            out.missing.insert(slot.clone(), *why);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_protocol::files::{Shader, TextureRef, Values};

    fn material(shader: &str) -> Material {
        let mut m = plain(shader);
        m.values.ints.insert(LILTOON_VERSION_PROPERTY.into(), 45);
        m
    }

    fn plain(shader: &str) -> Material {
        Material {
            key: "guid:0123456789abcdef0123456789abcdef/fileid:1".into(),
            name: "Body".into(),
            shader: Shader {
                name: shader.into(),
                guid: "fedcba9876543210fedcba9876543210".into(),
                package: String::new(),
                version: "2.3.4".into(),
                keywords: vec!["_A".into(), "_A".into(), "has space".into()],
                render_queue: Some(2450),
            },
            values: Values {
                floats: [
                    ("_ShadowBorder".into(), 0.3),
                    ("x".repeat(MAX_NAME + 1), 1.0),
                ]
                .into_iter()
                .collect(),
                ints: [("_UseShadow".into(), 1)].into_iter().collect(),
                colors: [("_Color".into(), [1.0, 0.5, 0.5, 1.0])]
                    .into_iter()
                    .collect(),
                vectors: [("_MatCapTex_ST".into(), [2.0, 2.0, 0.0, 0.0])]
                    .into_iter()
                    .collect(),
            },
            textures: ["_MainTex", "_MatCapTex", "_ShadowColorTex", "_Main2ndTex"]
                .into_iter()
                .map(|p| TextureRef {
                    property: p.into(),
                    path: Some(format!("/x/{p}.png")),
                    guid: String::new(),
                    srgb: true,
                    normal_map: false,
                    scale: [3.0, 3.0],
                    offset: [0.5, 0.0],
                })
                .collect(),
        }
    }

    #[test]
    fn values_become_a_received_liltoon_look() {
        let image = Arc::new(ReceivedImage {
            width: 1,
            height: 1,
            srgb: true,
            pixels: vec![1, 2, 3, 4].into(),
        });
        let images = [("_MatCapTex".to_owned(), image.clone())]
            .into_iter()
            .collect();
        let missing = [
            ("_ShadowColorTex".to_owned(), MissingImage::OverBudget),
            ("_MainTex".to_owned(), MissingImage::Unreadable),
        ]
        .into_iter()
        .collect();
        let r = received_look(&material("Hidden/lilToonCutout"), &images, &missing).unwrap();
        assert_eq!(r.look.kind, LookKind::LilToon);
        assert_eq!(r.look.shader, "Hidden/lilToonCutout");
        assert_eq!(r.look.shader_version, "2.3.4");
        assert_eq!(r.look.render_queue, Some(2450));
        assert_eq!(r.source, "Hidden/lilToonCutout 2.3.4");
        assert_eq!(r.look.get("_ShadowBorder"), Some(LookValue::Float(0.3)));
        assert_eq!(r.look.get("_UseShadow"), Some(LookValue::Int(1)));
        assert_eq!(
            r.look.properties.len(),
            5 + 3,
            "長すぎる名前は落とし、_ST を足す"
        );
        assert_eq!(
            r.look.get("_MatCapTex_ST"),
            Some(LookValue::Vector([2.0, 2.0, 0.0, 0.0])),
            "値にある _ST が勝つ"
        );
        assert_eq!(
            r.look.get("_ShadowColorTex_ST"),
            Some(LookValue::Vector([3.0, 3.0, 0.5, 0.0]))
        );
        assert_eq!(r.look.keywords, ["_A"]);
        assert_eq!(
            r.look.textures["_MainTex"],
            TextureSource::Channel(Channel::Color),
            "Color はスタンドアロンの絵で見せる"
        );
        assert_eq!(r.images["_MatCapTex"], image);
        assert_eq!(r.missing["_ShadowColorTex"], MissingImage::OverBudget);
        assert!(!r.missing.contains_key("_MainTex"));
        assert!(
            !r.missing.contains_key("_Main2ndTex"),
            "理由の無いスロットは既定で描く"
        );
        r.validate().unwrap();
    }

    #[test]
    fn a_material_that_is_not_liltoon_has_no_received_look() {
        let look = |m: &Material| received_look(m, &BTreeMap::new(), &BTreeMap::new());
        assert!(is_liltoon(&material("Hidden/lilToonCutout")));
        // 名前が似ているだけのシェーダーは lilToon にしない
        let lookalike = plain("Custom/lilToonLike");
        assert!(!is_liltoon(&lookalike));
        assert!(look(&lookalike).is_none());
        assert!(look(&plain("Standard")).is_none());
        // 版の値が浮動小数でも、lilToon のパッケージのシェーダーでも lilToon
        let mut float = plain("lilToon");
        float
            .values
            .floats
            .insert(LILTOON_VERSION_PROPERTY.into(), 45.0);
        assert!(is_liltoon(&float));
        let mut packaged = plain("lilToon");
        packaged.shader.package = LILTOON_PACKAGE.into();
        assert!(is_liltoon(&packaged));
        assert!(look(&packaged).is_some());
    }
}
