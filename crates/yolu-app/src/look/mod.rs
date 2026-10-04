//! テクスチャセットの見た目の設定（3D ビューで、そのセットの面を標準の PBR か lilToon の再現で描くか、と lilToon の値）の画面の側:
//! 値の表（`liltoon`）、操作（`LookOp`。`Action::Look`）、lilToon のひな形（要るユーザーチャンネルを作って割り当てる）、欄（`panel`）、
//! .ylp との受け渡し（`io`）。値は文書（`Document::look`）が持ち、変更は 1 回の Undo（スライダーのドラッグは 1 段にまとめる）。

pub mod export;
pub mod io;
pub mod liltoon;
pub mod link;
pub mod panel;

use yolu_core::look::{LookKind, LookValue, MaterialLook, PlaneSource, TextureSource};
use yolu_core::{Channel, ChannelInfo, ChannelKind, ColorSpace, CoreError, Document, ImageId, Rgba8};

use crate::lang::Lang;
use crate::state::AppState;
use liltoon::RenderMode;

/// 欄の節（lilToon のインスペクターの並び。既定に戻す単位）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Base,
    Lighting,
    Main,
    Shadow,
    Emission,
    Normal,
    MatCap,
    Rim,
    Outline,
}

impl Section {
    /// 節の値（既定に戻すと設定から外す名前）。
    pub fn props(self) -> &'static [&'static str] {
        match self {
            Section::Base => &["_Cutoff", "_Cull", "_FlipNormal", "_BackfaceForceShadow", "_BackfaceColor", "_Invisible"],
            Section::Lighting => &[
                "_LightMinLimit",
                "_LightMaxLimit",
                "_MonochromeLighting",
                "_ShadowEnvStrength",
                "_AsUnlit",
                "_AAStrength",
                "_LightDirectionOverride",
            ],
            Section::Main => &["_Color", "_MainTexHSVG", "_AlphaMaskMode", "_AlphaMaskScale", "_AlphaMaskValue"],
            Section::Shadow => &[
                "_UseShadow",
                "_ShadowStrength",
                "_ShadowStrengthMaskLOD",
                "_ShadowBorderMaskLOD",
                "_ShadowBlurMaskLOD",
                "_ShadowAOShift",
                "_ShadowAOShift2",
                "_ShadowPostAO",
                "_ShadowColorType",
                "_ShadowColor",
                "_ShadowNormalStrength",
                "_ShadowBorder",
                "_ShadowBlur",
                "_ShadowReceive",
                "_Shadow2ndColor",
                "_Shadow2ndNormalStrength",
                "_Shadow2ndBorder",
                "_Shadow2ndBlur",
                "_Shadow2ndReceive",
                "_Shadow3rdColor",
                "_Shadow3rdNormalStrength",
                "_Shadow3rdBorder",
                "_Shadow3rdBlur",
                "_Shadow3rdReceive",
                "_ShadowBorderColor",
                "_ShadowBorderRange",
                "_ShadowMainStrength",
                "_ShadowMaskType",
                "_ShadowFlatBorder",
                "_ShadowFlatBlur",
            ],
            Section::Emission => &[
                "_UseEmission",
                "_EmissionColor",
                "_EmissionMap_UVMode",
                "_EmissionMainStrength",
                "_EmissionBlend",
                "_EmissionBlendMode",
                "_EmissionFluorescence",
                "_UseEmission2nd",
                "_Emission2ndColor",
                "_Emission2ndMap_UVMode",
                "_Emission2ndMainStrength",
                "_Emission2ndBlend",
                "_Emission2ndBlendMode",
                "_Emission2ndFluorescence",
            ],
            Section::Normal => &["_UseBumpMap", "_BumpScale"],
            Section::MatCap => &[
                "_UseMatCap",
                "_MatCapColor",
                "_MatCapMainStrength",
                "_MatCapZRotCancel",
                "_MatCapPerspective",
                "_MatCapBlend",
                "_MatCapEnableLighting",
                "_MatCapShadowMask",
                "_MatCapBackfaceMask",
                "_MatCapLod",
                "_MatCapBlendMode",
                "_MatCapApplyTransparency",
                "_MatCapNormalStrength",
                "_UseMatCap2nd",
                "_MatCap2ndColor",
                "_MatCap2ndMainStrength",
                "_MatCap2ndZRotCancel",
                "_MatCap2ndPerspective",
                "_MatCap2ndBlend",
                "_MatCap2ndEnableLighting",
                "_MatCap2ndShadowMask",
                "_MatCap2ndBackfaceMask",
                "_MatCap2ndLod",
                "_MatCap2ndBlendMode",
                "_MatCap2ndApplyTransparency",
                "_MatCap2ndNormalStrength",
            ],
            Section::Rim => &[
                "_UseRim",
                "_RimColor",
                "_RimMainStrength",
                "_RimNormalStrength",
                "_RimBorder",
                "_RimBlur",
                "_RimFresnelPower",
                "_RimEnableLighting",
                "_RimShadowMask",
                "_RimBackfaceMask",
                "_RimApplyTransparency",
                "_RimDirStrength",
                "_RimDirRange",
                "_RimIndirRange",
                "_RimIndirColor",
                "_RimIndirBorder",
                "_RimIndirBlur",
                "_RimBlendMode",
            ],
            Section::Outline => &[
                "_OutlineColor",
                "_OutlineTexHSVG",
                "_OutlineLitColor",
                "_OutlineLitApplyTex",
                "_OutlineLitScale",
                "_OutlineLitOffset",
                "_OutlineLitShadowReceive",
                "_OutlineWidth",
                "_OutlineFixWidth",
                "_OutlineDeleteMesh",
                "_OutlineEnableLighting",
                "_OutlineZBias",
            ],
        }
    }
}

/// 見た目の設定の操作（`Action::Look`。どれも文書を変え、1 回の Undo）。
#[derive(Clone, Debug, PartialEq)]
pub enum LookOp {
    /// 標準と lilToon を切り替える（lilToon にしたとき、スロットが 1 つも無ければ、メインカラー・ノーマルマップ・発光を同じ名前の
    /// チャンネルに割り当てる。書き出しの lilToon の画像と同じ）。
    Kind(LookKind),
    /// 描画モード（シェーダーの名前が変わる。Cutoff は lilToon のインスペクターと同じく、カットアウトで 0.5・半透明で 0.001）。
    Mode(RenderMode),
    /// 輪郭線の入切（シェーダーの名前が変わる）。
    Outline(bool),
    /// プロパティの値。`drag` ならスライダーのドラッグとして前の変更とまとめる。
    Value {
        name: &'static str,
        value: LookValue,
        drag: bool,
    },
    /// テクスチャのスロットの入力（None は割り当てを外す）。
    Texture {
        slot: &'static str,
        source: Option<TextureSource>,
    },
    /// 節の値を既定へ（Unity から受けた値があれば、その節は Unity の値で描く）。基本設定の節は、その節の行の描画モードと輪郭線の入切
    /// （シェーダーの名前）も戻す。
    Reset(Section),
    /// Unity から受けた値に合わせる（欄で変えた値・描画モード・輪郭線・描き方の選びを外す。スロットの割り当ては残す）。
    FollowReceived,
    /// lilToon のひな形（lilToon にして、入にしている機能のマスクのユーザーチャンネルを作って割り当てる）。
    Template,
}

impl LookOp {
    pub fn edits_document(&self) -> bool {
        true
    }
}

/// lilToon にしたときの既定の割り当て（書き出しの lilToon のテンプレートの Main・Normal・Emission と同じチャンネル）。
pub fn default_textures(look: &mut MaterialLook) {
    for (slot, channel) in [
        ("_MainTex", Channel::Color),
        ("_BumpMap", Channel::Normal),
        ("_EmissionMap", Channel::Emission),
    ] {
        look.textures
            .entry(slot.into())
            .or_insert(TextureSource::Channel(channel));
    }
}

/// 見た目の設定が指すプロジェクトの画像（マットキャップの絵。効果の入力として復号してもらう）。
pub fn image_ids(doc: &Document) -> Vec<ImageId> {
    let look = doc.drawn_look();
    if look.kind != LookKind::LilToon {
        return Vec::new();
    }
    look.textures
        .values()
        .filter_map(|t| match t {
            TextureSource::Image(id) => Some(*id),
            _ => None,
        })
        .collect()
}

/// チャンネルを消す。見た目の設定がそのチャンネルを読んでいれば、割り当ても外す（1 回の Undo。外さないと、あとで同じ番号に足した
/// 別のチャンネルを黙って読む）。
pub fn remove_channel(doc: &mut Document, channel: Channel) -> Result<(), CoreError> {
    if !doc.look().channels().contains(&channel) {
        return doc.remove_channel(channel);
    }
    doc.batch(|d| {
        let look = d.look().without_channel(channel);
        d.set_look(look, false)?;
        d.remove_channel(channel)
    })
}

/// ひな形で作るチャンネル 1 つ。
struct TemplateMask {
    /// この機能が入のときに作る（"outline" は輪郭線のシェーダー）。
    toggle: &'static str,
    slot: &'static str,
    ja: &'static str,
    en: &'static str,
    kind: ChannelKind,
    space: ColorSpace,
    default: Rgba8,
}

const WHITE: Rgba8 = Rgba8::new(255, 255, 255, 255);

const TEMPLATE: &[TemplateMask] = &[
    TemplateMask {
        toggle: "_UseShadow",
        slot: "_ShadowStrengthMask",
        ja: "影の強度",
        en: "Shadow Strength",
        kind: ChannelKind::Scalar,
        space: ColorSpace::Linear,
        default: WHITE,
    },
    TemplateMask {
        toggle: "_UseShadow",
        slot: "_ShadowColorTex",
        ja: "影色",
        en: "Shadow Color",
        kind: ChannelKind::Color,
        space: ColorSpace::Srgb,
        default: Rgba8::new(0, 0, 0, 0),
    },
    TemplateMask {
        toggle: "_UseShadow",
        slot: "_ShadowBorderMask",
        ja: "AO",
        en: "AO",
        kind: ChannelKind::Scalar,
        space: ColorSpace::Linear,
        default: WHITE,
    },
    TemplateMask {
        toggle: "_UseRim",
        slot: "_RimColorTex",
        ja: "リムライトのマスク",
        en: "Rim Light Mask",
        kind: ChannelKind::Scalar,
        space: ColorSpace::Linear,
        default: WHITE,
    },
    TemplateMask {
        toggle: "_UseMatCap",
        slot: "_MatCapBlendMask",
        ja: "マットキャップのマスク",
        en: "MatCap Mask",
        kind: ChannelKind::Scalar,
        space: ColorSpace::Linear,
        default: WHITE,
    },
    TemplateMask {
        toggle: "_UseMatCap2nd",
        slot: "_MatCap2ndBlendMask",
        ja: "マットキャップ 2nd のマスク",
        en: "MatCap 2nd Mask",
        kind: ChannelKind::Scalar,
        space: ColorSpace::Linear,
        default: WHITE,
    },
    TemplateMask {
        toggle: "_UseEmission",
        slot: "_EmissionBlendMask",
        ja: "発光のマスク",
        en: "Emission Mask",
        kind: ChannelKind::Scalar,
        space: ColorSpace::Linear,
        default: WHITE,
    },
    TemplateMask {
        toggle: "_UseEmission2nd",
        slot: "_Emission2ndBlendMask",
        ja: "発光 2nd のマスク",
        en: "Emission 2nd Mask",
        kind: ChannelKind::Scalar,
        space: ColorSpace::Linear,
        default: WHITE,
    },
    TemplateMask {
        toggle: "outline",
        slot: "_OutlineWidthMask",
        ja: "輪郭線の太さ",
        en: "Outline Width",
        kind: ChannelKind::Scalar,
        space: ColorSpace::Linear,
        default: WHITE,
    },
];

/// 名前が文書のチャンネルと重ならないように（重なれば「 2」「 3」…）。
fn free_name(doc: &Document, base: &str) -> String {
    let taken = |n: &str| {
        doc.channels()
            .iter()
            .any(|c| doc.channel_info(*c).is_some_and(|i| i.name == n))
    };
    if !taken(base) {
        return base.to_owned();
    }
    (2..)
        .map(|i| format!("{base} {i}"))
        .find(|n| !taken(n))
        .expect("どこかで空く")
}

/// lilToon のひな形を当てる（1 回の Undo）。lilToon にして既定の割り当てを足し、入にしている機能（影・リム・マットキャップ・発光・
/// 輪郭線）のマスクのスロットが空なら、ユーザーチャンネルを作って割り当てる。影の SDF のときは、影の強度マスクを左右の 2 つの
/// チャンネル（R・G）の詰め合わせにする。機能が 1 つも入でなければ影を入にする。作ったチャンネルの数を返す。
pub fn apply_template(doc: &mut Document, lang: Lang) -> Result<usize, CoreError> {
    doc.batch(|d| {
        // 入にしている機能は描く見た目（Unity から受けた値を含む）で見て、割り当ては利用者の設定に書く
        let drawn = d.drawn_look().clone();
        let mut look = d.look().clone();
        look.kind = LookKind::LilToon;
        let mut assigned = drawn.clone();
        default_textures(&mut assigned);
        for (slot, source) in &assigned.textures {
            if !drawn.textures.contains_key(slot) {
                look.textures.insert(slot.clone(), *source);
            }
        }
        let toggles = [
            "_UseShadow",
            "_UseRim",
            "_UseMatCap",
            "_UseMatCap2nd",
            "_UseEmission",
            "_UseEmission2nd",
        ];
        let outline = liltoon::shader_info(&drawn).outline;
        let mut on_now = drawn.clone();
        if !outline && !toggles.iter().any(|t| liltoon::on(&drawn, t)) {
            look.properties
                .insert("_UseShadow".into(), LookValue::Float(1.0));
            on_now.properties
                .insert("_UseShadow".into(), LookValue::Float(1.0));
        }
        let mut made = 0usize;
        let mut add = |d: &mut Document, name: &str, kind: ChannelKind, space: ColorSpace, default: Rgba8| {
            let name = free_name(d, name);
            made += 1;
            d.add_channel(ChannelInfo {
                name,
                kind,
                color_space: space,
                default,
            })
        };
        for mask in TEMPLATE {
            let on = if mask.toggle == "outline" {
                outline
            } else {
                liltoon::on(&on_now, mask.toggle)
            };
            let has_image = d
                .received_look()
                .is_some_and(|r| r.images.contains_key(mask.slot));
            if !on || look.textures.contains_key(mask.slot) || drawn.textures.contains_key(mask.slot) || has_image {
                continue;
            }
            if mask.slot == "_ShadowStrengthMask"
                && liltoon::number(&drawn, "_ShadowMaskType").round() == 2.0
            {
                let right = add(
                    d,
                    lang.pick("顔の影（右）", "Face Shadow (Right)"),
                    ChannelKind::Scalar,
                    ColorSpace::Linear,
                    WHITE,
                )?;
                let left = add(
                    d,
                    lang.pick("顔の影（左）", "Face Shadow (Left)"),
                    ChannelKind::Scalar,
                    ColorSpace::Linear,
                    WHITE,
                )?;
                look.textures.insert(
                    mask.slot.into(),
                    TextureSource::Packed([
                        PlaneSource::Channel {
                            channel: right,
                            component: 0,
                        },
                        PlaneSource::Channel {
                            channel: left,
                            component: 0,
                        },
                        PlaneSource::Zero,
                        PlaneSource::One,
                    ]),
                );
                continue;
            }
            let channel = add(d, lang.pick(mask.ja, mask.en), mask.kind, mask.space, mask.default)?;
            look.textures
                .insert(mask.slot.into(), TextureSource::Channel(channel));
        }
        d.set_look(look, false)?;
        Ok(made)
    })
}

impl AppState {
    /// 見た目の設定の操作を当てる（`Action::Look`）。
    pub fn look_apply(&mut self, op: LookOp) {
        if self.is_stroking() {
            self.message = self
                .lang
                .pick("描いている間はできません。", "Not while drawing.")
                .into();
            return;
        }
        if op == LookOp::Template {
            let lang = self.lang;
            match apply_template(&mut self.doc, lang) {
                Ok(made) => {
                    self.modified = true;
                    self.message = lang.pick(
                        format!("lilToon のひな形を当てました（チャンネル {made}）。"),
                        format!("Applied the lilToon template ({made} channels)."),
                    );
                }
                Err(e) => self.message = self.lang.core_error(&e),
            }
            return;
        }
        let mut look = self.doc.look().clone();
        // 描画モード・輪郭線の今の様子は描く見た目（Unity から受けた値を含む）から読み、変えた値は利用者の設定に書く
        let drawn = self.doc.drawn_look().clone();
        let mut coalesce = false;
        match op {
            LookOp::Kind(kind) => {
                look.kind = kind;
                // Unity から受けた値があるときだけ「選んだ」と覚える（受けた値の描き方より欄の選びが勝つ）。無いときの選びは種類だけで
                // 足り、標準に戻した設定は既定のまま（保存しない）
                look.kind_chosen = self.doc.received_look().is_some();
                if kind == LookKind::LilToon && drawn.textures.is_empty() {
                    default_textures(&mut look);
                }
            }
            LookOp::Mode(mode) => {
                let info = liltoon::shader_info(&drawn);
                look.shader = liltoon::shader_name(mode, info.outline);
                let cutoff = match mode {
                    RenderMode::Cutout => Some(0.5),
                    RenderMode::Transparent => Some(0.001),
                    RenderMode::Opaque => None,
                };
                if let Some(c) = cutoff {
                    look.properties
                        .insert("_Cutoff".into(), LookValue::Float(c));
                }
            }
            LookOp::Outline(on) => {
                let info = liltoon::shader_info(&drawn);
                look.shader = liltoon::shader_name(info.mode, on);
            }
            LookOp::FollowReceived => {
                look = MaterialLook {
                    textures: look.textures,
                    ..MaterialLook::default()
                };
            }
            LookOp::Value { name, value, drag } => {
                look.properties.insert(name.into(), value);
                coalesce = drag;
            }
            LookOp::Texture { slot, source } => match source {
                Some(s) => {
                    look.textures.insert(slot.into(), s);
                }
                None => {
                    look.textures.remove(slot);
                }
            },
            LookOp::Reset(section) => {
                for name in section.props() {
                    look.properties.remove(*name);
                }
                // 描画モードと輪郭線の入切は基本設定の節の行（輪郭線設定の節は、輪郭線が入のときに出る値だけ）
                if section == Section::Base {
                    look.shader.clear();
                }
            }
            LookOp::Template => unreachable!("上で扱った"),
        }
        let before = self.doc.revision();
        match self.doc.set_look(look, coalesce) {
            Ok(()) => {
                if self.doc.revision() != before {
                    self.modified = true;
                }
            }
            Err(e) => self.message = self.lang.core_error(&e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_template_makes_masks_for_enabled_features_in_one_undo() {
        let mut doc = Document::new(32, 32).unwrap();
        let mut look = MaterialLook {
            kind: LookKind::LilToon,
            shader: "Hidden/lilToonOutline".into(),
            ..MaterialLook::default()
        };
        look.properties.insert("_UseRim".into(), LookValue::Float(1.0));
        doc.set_look(look, false).unwrap();
        let steps = doc.undo_count();
        let made = apply_template(&mut doc, Lang::Ja).unwrap();
        assert_eq!(made, 2, "リムと輪郭線");
        assert_eq!(doc.undo_count(), steps + 1);
        let look = doc.look();
        assert!(matches!(look.textures["_RimColorTex"], TextureSource::Channel(c) if !c.is_standard()));
        assert!(matches!(look.textures["_OutlineWidthMask"], TextureSource::Channel(c) if !c.is_standard()));
        assert_eq!(look.textures["_MainTex"], TextureSource::Channel(Channel::Color));
        // もう一度当てても、割り当て済みのスロットには作らない
        assert_eq!(apply_template(&mut doc, Lang::Ja).unwrap(), 0);
        doc.undo().unwrap();
        doc.undo().unwrap();
        assert_eq!(doc.channels().len(), Channel::STANDARD_COUNT);
        assert!(doc.look().textures.is_empty());
    }

    #[test]
    fn the_template_turns_on_the_shadow_when_nothing_is_on_and_packs_sdf() {
        let mut doc = Document::new(32, 32).unwrap();
        assert_eq!(apply_template(&mut doc, Lang::En).unwrap(), 3, "影の強度・影色・AO");
        assert!(liltoon::on(doc.look(), "_UseShadow"));
        let mut doc = Document::new(32, 32).unwrap();
        let mut look = MaterialLook {
            kind: LookKind::LilToon,
            ..MaterialLook::default()
        };
        look.properties
            .insert("_UseShadow".into(), LookValue::Float(1.0));
        look.properties
            .insert("_ShadowMaskType".into(), LookValue::Float(2.0));
        doc.set_look(look, false).unwrap();
        apply_template(&mut doc, Lang::En).unwrap();
        let TextureSource::Packed(planes) = doc.look().textures["_ShadowStrengthMask"] else {
            panic!("SDF は詰め合わせ");
        };
        assert!(matches!(planes[0], PlaneSource::Channel { .. }));
        assert!(matches!(planes[1], PlaneSource::Channel { .. }));
        assert_eq!(planes[2], PlaneSource::Zero);
        assert_eq!(planes[3], PlaneSource::One);
    }

    #[test]
    fn removing_a_channel_drops_its_slots_in_the_same_undo() {
        let mut doc = Document::new(32, 32).unwrap();
        apply_template(&mut doc, Lang::Ja).unwrap();
        let mask = match doc.look().textures["_ShadowStrengthMask"] {
            TextureSource::Channel(c) => c,
            _ => unreachable!(),
        };
        let steps = doc.undo_count();
        remove_channel(&mut doc, mask).unwrap();
        assert_eq!(doc.undo_count(), steps + 1);
        assert!(!doc.look().textures.contains_key("_ShadowStrengthMask"));
        doc.undo().unwrap();
        assert!(doc.look().textures.contains_key("_ShadowStrengthMask"));
        assert!(doc.channel_info(mask).is_some());
    }
}
