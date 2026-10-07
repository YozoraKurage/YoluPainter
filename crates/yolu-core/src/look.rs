//! テクスチャセット（モデルのマテリアル 1 つ）の「見た目の設定」: 3D ビューがそのセットの面をどう描くか（標準の PBR か、lilToon の再現か）と、
//! lilToon のときのマテリアルの値。
//!
//! - 値は Unity のマテリアルと同じ形で持つ: シェーダーの名前（lilToon の描画モードと輪郭線は、Unity ではシェーダーの名前で決まる）、
//!   プロパティの名前（`_ShadowColor` など）ごとの型つきの値、テクスチャのスロット（`_MainTex` など）ごとの入力、キーワード。
//!   知らない名前・知らないスロットも落とさずに持つ（Live Link で受けたマテリアルの値をそのまま持つため。描くかどうかは読み手が決める）。
//! - 値の既定（プロパティが無いときの値）はここでは持たない。描く側（アプリの lilToon の表）がシェーダーの既定を知っている。
//! - テクスチャのスロットの入力は、テクスチャセットのチャンネル（標準とユーザーチャンネル）か、成分ごとの詰め合わせ（R・G・B・A に
//!   チャンネルの成分か 0・1）か、プロジェクトの画像リソース（マットキャップの絵など）。割り当てていないスロットは、シェーダーの
//!   既定のテクスチャ（白・黒・平らな法線）で描く。
//!
//! 文書（[`crate::Document`]）が持ち、変更は 1 回の Undo（[`crate::Document::set_look`]）。画素ではないので、文書の正本
//! （`document.utpaint`）には入らない: `.ylp` へは呼び手が別のエントリとして書き、読み込み直後に [`crate::Document::restore_look`] で戻す
//! （手動の ID 色・選択範囲と同じ流儀）。
//!
//! 外から受けた見た目（[`ReceivedLook`]。Live Link で Unity の本物のマテリアルから受けた値と、描いていないスロットの絵）は、利用者の
//! 設定とは別に文書が持つ（[`crate::Document::set_received_look`]。Undo にも版にも入らない。受け取りは利用者の操作ではないので）。
//! 描く見た目（[`crate::Document::drawn_look`]）は、受けた値の上に利用者の設定を重ねたもの（[`MaterialLook::over`]）: 利用者が欄で
//! 変えた項目だけが受けた値に勝つ。

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::effects::ImageId;
use crate::{Channel, CoreError};

/// プロパティの数の上限（lilToon 2.3 のプロパティは約 600。知らないシェーダーの値も持てるように余裕をとる）。
pub const MAX_PROPERTIES: usize = 2048;
/// テクスチャのスロットの数の上限。
pub const MAX_TEXTURES: usize = 256;
/// キーワードの数の上限。
pub const MAX_KEYWORDS: usize = 256;
/// プロパティ・スロット・キーワードの名前の長さの上限（UTF-16 の数）。
pub const MAX_NAME: usize = 128;
/// シェーダーの名前の長さの上限（UTF-16 の数）。
pub const MAX_SHADER_NAME: usize = 256;
/// lilToon の既定のシェーダーの名前（不透明・輪郭線なし）。
pub const LILTOON_SHADER: &str = "lilToon";

/// 3D ビューの描き方の種類。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LookKind {
    /// 標準（金属の流儀の PBR。Unity の Standard と同じ BRDF）。
    #[default]
    Standard,
    /// lilToon の再現（基本の範囲）。
    LilToon,
}

impl LookKind {
    /// 保存の名前（`.ylp` の `look.json`）。
    pub fn key(self) -> &'static str {
        match self {
            LookKind::Standard => "standard",
            LookKind::LilToon => "lilToon",
        }
    }

    pub fn from_key(key: &str) -> Option<LookKind> {
        match key {
            "standard" => Some(LookKind::Standard),
            "lilToon" => Some(LookKind::LilToon),
            _ => None,
        }
    }
}

/// マテリアルのプロパティの値（Unity の型ごと）。色は Unity と同じくガンマの空間（sRGB）で持ち、描く側がリニアへ直す。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LookValue {
    /// Float・Range（Unity の古い Int も浮動小数で持つ）。
    Float(f32),
    /// Integer。
    Int(i32),
    /// Color（RGBA。HDR の色は 1 を超えてよい）。
    Color([f32; 4]),
    /// Vector（`_MainTex_ST` のようなテクスチャのタイリングとオフセットも）。
    Vector([f32; 4]),
}

impl LookValue {
    /// 数として読む（Float・Int。色・ベクトルは x）。
    pub fn as_f32(&self) -> f32 {
        match *self {
            LookValue::Float(v) => v,
            LookValue::Int(v) => v as f32,
            LookValue::Color(v) | LookValue::Vector(v) => v[0],
        }
    }

    /// 4 つの値として読む（数は x に置き、ほかは 0）。
    pub fn as_vec4(&self) -> [f32; 4] {
        match *self {
            LookValue::Float(v) => [v, 0.0, 0.0, 0.0],
            LookValue::Int(v) => [v as f32, 0.0, 0.0, 0.0],
            LookValue::Color(v) | LookValue::Vector(v) => v,
        }
    }

    pub fn is_finite(&self) -> bool {
        match self {
            LookValue::Float(v) => v.is_finite(),
            LookValue::Int(_) => true,
            LookValue::Color(v) | LookValue::Vector(v) => v.iter().all(|x| x.is_finite()),
        }
    }
}

/// 詰め合わせの 1 成分の元。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlaneSource {
    Zero,
    One,
    /// チャンネルの成分（0 R・1 G・2 B・3 A。スカラーのチャンネルは 0 が値）。
    Channel {
        channel: Channel,
        component: u8,
    },
}

/// テクスチャのスロットの入力。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextureSource {
    /// チャンネルの全部（色のチャンネルは RGBA、スカラーのチャンネルは値を RGB に・A は 1）。
    Channel(Channel),
    /// R・G・B・A をそれぞれの元から（影の強さのマスクの SDF のように、成分ごとに意味があるスロット）。
    Packed([PlaneSource; 4]),
    /// プロジェクトの画像リソース（マットキャップの絵など、描くものではない絵）。
    Image(ImageId),
}

impl TextureSource {
    /// 読むチャンネル（重ならない。並びは出てきた順）。
    pub fn channels(&self) -> Vec<Channel> {
        let mut out = Vec::new();
        match self {
            TextureSource::Channel(c) => out.push(*c),
            TextureSource::Packed(planes) => {
                for p in planes {
                    if let PlaneSource::Channel { channel, .. } = p {
                        if !out.contains(channel) {
                            out.push(*channel);
                        }
                    }
                }
            }
            TextureSource::Image(_) => {}
        }
        out
    }
}

/// テクスチャセットの見た目の設定。既定は標準（PBR）で、値もスロットも空。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MaterialLook {
    pub kind: LookKind,
    /// 利用者が描き方（`kind`）を欄で選んだ。受けた見た目（[`ReceivedLook`]）があるとき、選んでいなければ受けた描き方で描く
    /// （受けた見た目が無いときは使わない）。
    pub kind_chosen: bool,
    /// Unity のシェーダーの名前（lilToon のときの描画モード・輪郭線。空は [`LILTOON_SHADER`] と同じ）。標準でも持ったままにする
    /// （標準へ切り替えて戻したとき、元の lilToon の設定に戻る）。
    pub shader: String,
    /// プロパティの名前 → 値（名前の順）。
    pub properties: BTreeMap<String, LookValue>,
    /// テクスチャのスロットの名前 → 入力（名前の順）。
    pub textures: BTreeMap<String, TextureSource>,
    /// シェーダーのキーワード（受けたものを持つだけ。並びは受けた順）。
    pub keywords: Vec<String>,
    /// Unity のシェーダーのアセットの GUID（受けたものを持つだけ。空は不明）。
    pub shader_guid: String,
    /// シェーダーの版（lilToon の版など。受けたものを持つだけ。空は不明）。
    pub shader_version: String,
    /// 描画の順（Unity のマテリアルの renderQueue。None はシェーダーの既定）。
    pub render_queue: Option<i32>,
}

impl MaterialLook {
    /// 既定（標準・値なし）か。既定の設定は保存しない。
    pub fn is_default(&self) -> bool {
        *self == MaterialLook::default()
    }

    /// シェーダーの名前（空なら既定の lilToon）。
    pub fn shader_name(&self) -> &str {
        if self.shader.is_empty() {
            LILTOON_SHADER
        } else {
            &self.shader
        }
    }

    /// プロパティの値（無ければ None。描く側が既定を当てる）。
    pub fn get(&self, name: &str) -> Option<LookValue> {
        self.properties.get(name).copied()
    }

    /// プロパティを数として読む（無ければ `default`）。
    pub fn float(&self, name: &str, default: f32) -> f32 {
        self.get(name).map_or(default, |v| v.as_f32())
    }

    /// プロパティを 4 つの値として読む（無ければ `default`）。
    pub fn vec4(&self, name: &str, default: [f32; 4]) -> [f32; 4] {
        self.get(name).map_or(default, |v| v.as_vec4())
    }

    /// 読むチャンネル（全部のスロットの。重ならない。番号の順）。
    pub fn channels(&self) -> Vec<Channel> {
        let mut out: Vec<Channel> = self.textures.values().flat_map(|t| t.channels()).collect();
        out.sort_by_key(|c| c.index());
        out.dedup();
        out
    }

    /// このチャンネルを読むスロットを外した設定（チャンネルを消すとき、割り当てを残さない。詰め合わせはその成分だけ既定の 1 に）。
    pub fn without_channel(&self, channel: Channel) -> MaterialLook {
        let mut look = self.clone();
        look.textures
            .retain(|_, t| *t != TextureSource::Channel(channel));
        for t in look.textures.values_mut() {
            if let TextureSource::Packed(planes) = t {
                for p in planes.iter_mut() {
                    if matches!(p, PlaneSource::Channel { channel: c, .. } if *c == channel) {
                        *p = PlaneSource::One;
                    }
                }
            }
        }
        look
    }

    /// 受けた見た目の値（`base`）の上に、この設定（利用者の設定）を重ねた、描く見た目。利用者が持つ項目が勝つ: プロパティ・
    /// テクスチャのスロットは名前ごと、シェーダーは空でなければ、キーワードは 1 つでもあれば全部。描き方は、利用者が選んでいれば
    /// （`kind_chosen`）その描き方、選んでいなければ lilToon にした利用者の設定か受けた描き方。
    pub fn over(&self, base: &MaterialLook) -> MaterialLook {
        let mut out = base.clone();
        out.kind = if self.kind_chosen || self.kind == LookKind::LilToon {
            self.kind
        } else {
            base.kind
        };
        out.kind_chosen = self.kind_chosen;
        if !self.shader.is_empty() {
            out.shader = self.shader.clone();
        }
        for (k, v) in &self.properties {
            out.properties.insert(k.clone(), *v);
        }
        for (k, v) in &self.textures {
            out.textures.insert(k.clone(), *v);
        }
        if !self.keywords.is_empty() {
            out.keywords = self.keywords.clone();
        }
        if !self.shader_guid.is_empty() {
            out.shader_guid = self.shader_guid.clone();
        }
        if !self.shader_version.is_empty() {
            out.shader_version = self.shader_version.clone();
        }
        if self.render_queue.is_some() {
            out.render_queue = self.render_queue;
        }
        out
    }

    /// 形の検査（数・名前の長さ・制御文字・有限の値・成分の番号）。断るときは何の値か。
    pub fn validate(&self) -> Result<(), CoreError> {
        let name_ok = |s: &str, max: usize| {
            let n = s.encode_utf16().count();
            n >= 1 && n <= max && !s.chars().any(|c| c.is_control())
        };
        if !self.shader.is_empty() && !name_ok(&self.shader, MAX_SHADER_NAME) {
            return Err(CoreError::InvalidArgument("見た目のシェーダーの名前"));
        }
        if [&self.shader_guid, &self.shader_version]
            .iter()
            .any(|s| !s.is_empty() && !name_ok(s, MAX_NAME))
        {
            return Err(CoreError::InvalidArgument("見た目のシェーダーの身元"));
        }
        if self.properties.len() > MAX_PROPERTIES {
            return Err(CoreError::InvalidArgument("見た目のプロパティの数"));
        }
        if self.textures.len() > MAX_TEXTURES {
            return Err(CoreError::InvalidArgument("見た目のテクスチャの数"));
        }
        if self.keywords.len() > MAX_KEYWORDS {
            return Err(CoreError::InvalidArgument("見た目のキーワードの数"));
        }
        for (name, value) in &self.properties {
            if !name_ok(name, MAX_NAME) {
                return Err(CoreError::InvalidArgument("見た目のプロパティの名前"));
            }
            if !value.is_finite() {
                return Err(CoreError::InvalidArgument("見た目のプロパティの値"));
            }
        }
        for (name, source) in &self.textures {
            if !name_ok(name, MAX_NAME) {
                return Err(CoreError::InvalidArgument("見た目のテクスチャの名前"));
            }
            match source {
                TextureSource::Packed(planes) => {
                    if planes.iter().any(
                        |p| matches!(p, PlaneSource::Channel { component, .. } if *component > 3),
                    ) {
                        return Err(CoreError::InvalidArgument("見た目の詰め合わせの成分"));
                    }
                }
                TextureSource::Image(id) if id.0 == 0 => {
                    return Err(CoreError::InvalidArgument("見た目の画像の ID"));
                }
                _ => {}
            }
        }
        let mut seen = std::collections::HashSet::new();
        for k in &self.keywords {
            if !name_ok(k, MAX_NAME) || k.contains(' ') || !seen.insert(k.as_str()) {
                return Err(CoreError::InvalidArgument("見た目のキーワード"));
            }
        }
        Ok(())
    }

    /// 履歴の大きさの見積り（段の名目のバイト数）。
    pub(crate) fn history_cost(&self) -> u64 {
        let names: usize = self
            .properties
            .keys()
            .chain(self.textures.keys())
            .chain(self.keywords.iter())
            .map(|s| s.len() + 32)
            .sum();
        (256 + names + self.shader.len() + self.shader_guid.len() + self.shader_version.len())
            as u64
    }
}

/// 文書が持つ見た目の設定（共有。Undo の段も同じものを指す）。
pub(crate) type SharedLook = Arc<MaterialLook>;

/// 受けた絵の辺の上限（Live Link の送り手は 2048 まで縮めて送る。余裕をとる）。
pub const MAX_RECEIVED_IMAGE_SIZE: u32 = 4096;

/// 受けた絵 1 枚（描いていないスロットの、外のマテリアルのテクスチャ）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceivedImage {
    pub width: u32,
    pub height: u32,
    /// sRGB として読む（RGB をリニアへ直してから使う）。偽はリニアのまま。
    pub srgb: bool,
    /// RGBA8（straight）、行は下から。幅 × 高さ × 4 バイト。
    pub pixels: Arc<[u8]>,
}

/// 受けた見た目のスロットの、絵が無い理由（絵があれば無い）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MissingImage {
    /// 絵の予算（受けた絵の合計）を超えたので持たなかった。
    OverBudget,
    /// 絵のファイルを読めなかった。
    Unreadable,
    /// 送り手の中にしか無い絵（ファイルが無い生成物など）。
    NotAFile,
}

impl MissingImage {
    /// 保存の名前。
    pub fn key(self) -> &'static str {
        match self {
            MissingImage::OverBudget => "overBudget",
            MissingImage::Unreadable => "unreadable",
            MissingImage::NotAFile => "notAFile",
        }
    }

    /// 保存の名前から。0.4 までの `pending`（届いていない）は、読めないとして読む（絵のファイルを読み直すまで描けない）。
    pub fn from_key(key: &str) -> Option<MissingImage> {
        match key {
            "overBudget" => Some(MissingImage::OverBudget),
            "unreadable" | "pending" => Some(MissingImage::Unreadable),
            "notAFile" => Some(MissingImage::NotAFile),
            _ => None,
        }
    }
}

/// 外から受けた見た目（Live Link で Unity の本物のマテリアルから）。文書が持つが、Undo にも版にも入らない。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReceivedLook {
    /// 値（描き方・シェーダー・プロパティ・キーワード。`textures` は送り手が描いた絵を見せるスロットのチャンネル）。
    pub look: MaterialLook,
    /// 何の対応と確かめた値か（人に見せるだけ。例: "lilToon 2.3.4 · Standard/Opaque"）。
    pub source: String,
    /// 描いていないスロットの絵（スロットの名前 → 絵）。
    pub images: BTreeMap<String, Arc<ReceivedImage>>,
    /// 絵の無いスロットの、無い理由。
    pub missing: BTreeMap<String, MissingImage>,
}

impl ReceivedLook {
    /// 形の検査（値は [`MaterialLook::validate`]、絵は大きさとバイトの数、名前の長さと制御文字）。
    pub fn validate(&self) -> Result<(), CoreError> {
        self.look.validate()?;
        let name_ok = |s: &str| {
            let n = s.encode_utf16().count();
            (1..=MAX_NAME).contains(&n) && !s.chars().any(|c| c.is_control())
        };
        if self.source.encode_utf16().count() > MAX_SHADER_NAME
            || self.source.chars().any(|c| c.is_control())
        {
            return Err(CoreError::InvalidArgument("受けた見た目の出どころ"));
        }
        if self.images.len() + self.missing.len() > MAX_TEXTURES {
            return Err(CoreError::InvalidArgument("受けた見た目の絵の数"));
        }
        for (name, image) in &self.images {
            let size_ok = (1..=MAX_RECEIVED_IMAGE_SIZE).contains(&image.width)
                && (1..=MAX_RECEIVED_IMAGE_SIZE).contains(&image.height);
            if !name_ok(name)
                || !size_ok
                || image.pixels.len() as u64 != image.width as u64 * image.height as u64 * 4
            {
                return Err(CoreError::InvalidArgument("受けた見た目の絵"));
            }
        }
        if self.missing.keys().any(|n| !name_ok(n)) {
            return Err(CoreError::InvalidArgument("受けた見た目のスロットの名前"));
        }
        Ok(())
    }

    /// 絵のバイトの合計。
    pub fn image_bytes(&self) -> u64 {
        self.images.values().map(|i| i.pixels.len() as u64).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(i: usize) -> Channel {
        Channel::from_index(i).unwrap()
    }

    #[test]
    fn default_is_standard_and_empty() {
        let look = MaterialLook::default();
        assert!(look.is_default());
        assert_eq!(look.kind, LookKind::Standard);
        assert_eq!(look.shader_name(), LILTOON_SHADER);
        assert!(look.validate().is_ok());
        assert_eq!(
            LookKind::from_key(LookKind::LilToon.key()),
            Some(LookKind::LilToon)
        );
        assert_eq!(LookKind::from_key("lilToonFur"), None);
    }

    #[test]
    fn values_read_with_defaults() {
        let mut look = MaterialLook::default();
        look.properties
            .insert("_ShadowBorder".into(), LookValue::Float(0.25));
        look.properties.insert(
            "_ShadowColor".into(),
            LookValue::Color([0.1, 0.2, 0.3, 1.0]),
        );
        look.properties
            .insert("_UseShadow".into(), LookValue::Int(1));
        assert_eq!(look.float("_ShadowBorder", 0.5), 0.25);
        assert_eq!(look.float("_ShadowBlur", 0.1), 0.1);
        assert_eq!(look.float("_UseShadow", 0.0), 1.0);
        assert_eq!(look.vec4("_ShadowColor", [1.0; 4]), [0.1, 0.2, 0.3, 1.0]);
        assert_eq!(look.vec4("_Missing", [1.0; 4]), [1.0; 4]);
    }

    #[test]
    fn channels_and_removing_a_channel() {
        let mut look = MaterialLook::default();
        look.textures
            .insert("_MainTex".into(), TextureSource::Channel(Channel::Color));
        look.textures.insert(
            "_ShadowStrengthMask".into(),
            TextureSource::Channel(user(7)),
        );
        look.textures.insert(
            "_ShadowBorderMask".into(),
            TextureSource::Packed([
                PlaneSource::Channel {
                    channel: user(6),
                    component: 0,
                },
                PlaneSource::Channel {
                    channel: user(7),
                    component: 0,
                },
                PlaneSource::One,
                PlaneSource::Zero,
            ]),
        );
        assert_eq!(look.channels(), vec![Channel::Color, user(6), user(7)]);
        let without = look.without_channel(user(7));
        assert!(!without.textures.contains_key("_ShadowStrengthMask"));
        assert_eq!(
            without.textures["_ShadowBorderMask"],
            TextureSource::Packed([
                PlaneSource::Channel {
                    channel: user(6),
                    component: 0
                },
                PlaneSource::One,
                PlaneSource::One,
                PlaneSource::Zero,
            ])
        );
        assert_eq!(without.channels(), vec![Channel::Color, user(6)]);
    }

    #[test]
    fn validation_refuses_bad_names_values_and_counts() {
        let ok = |f: &dyn Fn(&mut MaterialLook)| {
            let mut look = MaterialLook::default();
            f(&mut look);
            look.validate()
        };
        assert!(ok(&|l| {
            l.properties.insert("_A".into(), LookValue::Float(1.0));
        })
        .is_ok());
        assert!(ok(&|l| {
            l.properties.insert(String::new(), LookValue::Float(1.0));
        })
        .is_err());
        assert!(ok(&|l| {
            l.properties.insert("a\nb".into(), LookValue::Float(1.0));
        })
        .is_err());
        assert!(ok(&|l| {
            l.properties
                .insert("x".repeat(MAX_NAME + 1), LookValue::Float(1.0));
        })
        .is_err());
        assert!(ok(&|l| {
            l.properties.insert("_A".into(), LookValue::Float(f32::NAN));
        })
        .is_err());
        assert!(ok(&|l| {
            l.properties.insert(
                "_A".into(),
                LookValue::Color([0.0, f32::INFINITY, 0.0, 1.0]),
            );
        })
        .is_err());
        assert!(ok(&|l| {
            for i in 0..=MAX_PROPERTIES {
                l.properties.insert(format!("_P{i}"), LookValue::Int(0));
            }
        })
        .is_err());
        assert!(ok(&|l| {
            l.textures.insert(
                "_T".into(),
                TextureSource::Packed([
                    PlaneSource::Channel {
                        channel: Channel::Color,
                        component: 4,
                    },
                    PlaneSource::Zero,
                    PlaneSource::Zero,
                    PlaneSource::Zero,
                ]),
            );
        })
        .is_err());
        assert!(ok(&|l| {
            l.textures
                .insert("_T".into(), TextureSource::Image(ImageId(0)));
        })
        .is_err());
        assert!(ok(&|l| l.keywords = vec!["A".into(), "A".into()]).is_err());
        assert!(ok(&|l| l.keywords = vec!["A B".into()]).is_err());
        assert!(ok(&|l| l.shader = "Hidden/lilToonCutout".into()).is_ok());
        assert!(ok(&|l| l.shader = "\u{7}".into()).is_err());
    }
}
