//! テクスチャセットの見た目の設定（3D ビューで、そのセットの面を標準の PBR か lilToon の再現で描くか、と lilToon の値）の画面の側:
//! 値の表（`liltoon`）、操作（`LookOp`。`Action::Look`）、lilToon のひな形（要るユーザーチャンネルを作って割り当てる）、欄（`panel`）、
//! .ylp との受け渡し（`io`）。値は文書（`Document::look`）が持ち、変更は 1 回の Undo（スライダーのドラッグは 1 段にまとめる）。

pub mod export;
pub mod fields;
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
    Uv,
    /// メインカラー / 透過設定（メインカラー・2nd・3rd・アルファマスク）。
    Main,
    Shadow,
    RimShade,
    Emission,
    /// ノーマルマップ設定（1st・2nd・異方性反射）。
    Normal,
    Backlight,
    /// 光沢設定。
    Reflection,
    MatCap,
    Rim,
    Glitter,
    /// 輪郭線設定（節の頭の入切はシェーダーの名前）。
    Outline,
    DistanceFade,
}

impl Section {
    /// 節の値（既定に戻すと設定から外す名前）。
    pub fn props(self) -> impl Iterator<Item = &'static str> {
        liltoon::section_props(self)
    }
}

/// ライティングのプリセット（lilToon の `ApplyLightingPreset` と同じ値。シェーダーの設定の既定は lilToon の既定のまま）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightingPreset {
    Default,
    SemiMonochrome,
}

impl LightingPreset {
    /// 入れる値: lilToon の `ApplyLightingPreset` が入れる 8 つのうち、欄の表にある（再現が描く）5 つ。頂点ライトの強さ・露出の上限・
    /// ディレクショナルライトの強さは描かず欄にも無いので入れない（入れると節の「既定に戻す」で外れず、変えた値の数にだけ加わる）。
    pub fn values(self) -> [(&'static str, f32); 5] {
        let mono = match self {
            LightingPreset::Default => 0.0,
            LightingPreset::SemiMonochrome => 0.5,
        };
        [
            ("_AsUnlit", 0.0),
            ("_LightMinLimit", 0.05),
            ("_LightMaxLimit", 1.0),
            ("_MonochromeLighting", mono),
            ("_ShadowEnvStrength", 0.0),
        ]
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
    /// いくつかの値を 1 回で（光沢のタイプ・デカールのミラーモードのように、lilToon の欄の 1 つの項目が複数のプロパティを決めるもの）。
    Values {
        values: Vec<(&'static str, LookValue)>,
        drag: bool,
    },
    /// 節の値を既定へ（Unity から受けた値があれば、その節は Unity の値で描く）。基本設定の節は描画モード（見出しのすぐ上の行）、
    /// 輪郭線設定の節は節の頭の輪郭線の入切も戻す（どちらもシェーダーの名前の一部）。
    Reset(Section),
    /// ライティングのプリセット（lilToon の「プリセットを適用」）。
    LightingPreset(LightingPreset),
    /// Unity から受けた値に合わせる（欄で変えた値・描画モード・輪郭線・描き方の選びを外す。スロットの割り当ては残す）。
    FollowReceived,
    /// lilToon のひな形（lilToon にして、入にしている機能のマスクのユーザーチャンネルを作って割り当てる）。
    Template,
    /// スロットを描く（割り当てが無ければチャンネルを作って割り当て、描くチャンネルをそれにする）。
    PaintSlot(&'static str),
    /// スロットの読むチャンネルを新しいユーザーチャンネルにする（割り当てがあっても新しく作る）。`plane` が None ならスロットの
    /// 割り当て全体を、Some なら成分ごとの詰め合わせのその成分（0〜3）だけを割り当てる。作ったチャンネルを描くチャンネルにする。
    NewChannel { slot: &'static str, plane: Option<u8> },
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

/// 新しく作るテクスチャセットの見た目の既定: lilToon（シェーダーの既定の値。メインカラー・ノーマルマップ・発光は同じ名前の
/// チャンネルに割り当てる。書き出しの lilToon の画像と同じ）。新しいプロジェクト・新しいテクスチャセット・モデルや Live Link の
/// マテリアルから作るセットが使う。読み込んだ文書（`.ylp` の `look.json` が無い文書を含む）は読み込んだとおり（無ければ標準）で、
/// ここを通らない。
pub fn new_set_look() -> MaterialLook {
    let mut look = MaterialLook {
        kind: LookKind::LilToon,
        ..MaterialLook::default()
    };
    default_textures(&mut look);
    look
}

/// 作ったばかりの文書（履歴なし）に、新しいセットの見た目の既定を入れる（Undo に入らない）。履歴がある文書は変えない。
pub fn apply_new_set_look(doc: &mut Document) {
    if doc.undo_count() == 0 && doc.look().is_default() {
        let _ = doc.restore_look(new_set_look());
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

/// ひな形で作るチャンネル 1 つ（スロットを描く口で作るチャンネルも、このスロットならこの作り）。
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
const CLEAR: Rgba8 = Rgba8::new(0, 0, 0, 0);

const fn mask(toggle: &'static str, slot: &'static str, ja: &'static str, en: &'static str) -> TemplateMask {
    TemplateMask {
        toggle,
        slot,
        ja,
        en,
        kind: ChannelKind::Scalar,
        space: ColorSpace::Linear,
        default: WHITE,
    }
}

const fn layer(toggle: &'static str, slot: &'static str, ja: &'static str, en: &'static str) -> TemplateMask {
    TemplateMask {
        toggle,
        slot,
        ja,
        en,
        kind: ChannelKind::Color,
        space: ColorSpace::Srgb,
        default: CLEAR,
    }
}

/// ひな形が作るチャンネル（インスペクターの節の並び）。色の層（影色・メインカラー 2nd・3rd）は何も描いていない所を透明にして、
/// 描いた所だけが効くようにする（影色のテクスチャは A でメインカラーと混ぜる。2nd・3rd は A で重ねる）。
const TEMPLATE: &[TemplateMask] = &[
    layer("_UseMain2ndTex", "_Main2ndTex", "メインカラー2nd", "Main Color 2nd"),
    layer("_UseMain3rdTex", "_Main3rdTex", "メインカラー3rd", "Main Color 3rd"),
    mask("_UseShadow", "_ShadowStrengthMask", "影の強度", "Shadow Strength"),
    layer("_UseShadow", "_ShadowColorTex", "影色", "Shadow Color"),
    mask("_UseShadow", "_ShadowBorderMask", "AO", "AO"),
    mask("_UseRimShade", "_RimShadeMask", "リムシェードのマスク", "RimShade Mask"),
    mask("_UseEmission", "_EmissionBlendMask", "発光のマスク", "Emission Mask"),
    mask("_UseEmission2nd", "_Emission2ndBlendMask", "発光2nd のマスク", "Emission 2nd Mask"),
    mask("_UseAnisotropy", "_AnisotropyScaleMask", "異方性反射のマスク", "Anisotropy Mask"),
    mask("_UseBacklight", "_BacklightColorTex", "逆光ライトのマスク", "Backlight Mask"),
    mask("_UseReflection", "_ReflectionColorTex", "光沢のマスク", "Reflection Mask"),
    mask("_UseMatCap", "_MatCapBlendMask", "マットキャップのマスク", "MatCap Mask"),
    mask("_UseMatCap2nd", "_MatCap2ndBlendMask", "マットキャップ2nd のマスク", "MatCap 2nd Mask"),
    mask("_UseRim", "_RimColorTex", "リムライトのマスク", "Rim Light Mask"),
    mask("_UseGlitter", "_GlitterColorTex", "ラメのマスク", "Glitter Mask"),
    mask("outline", "_OutlineWidthMask", "輪郭線の太さ", "Outline Width"),
];

/// ひな形の機能の入切（"outline" は輪郭線のシェーダー）。
const TEMPLATE_TOGGLES: &[&str] = &[
    "_UseMain2ndTex",
    "_UseMain3rdTex",
    "_UseShadow",
    "_UseRimShade",
    "_UseEmission",
    "_UseEmission2nd",
    "_UseAnisotropy",
    "_UseBacklight",
    "_UseReflection",
    "_UseMatCap",
    "_UseMatCap2nd",
    "_UseRim",
    "_UseGlitter",
];

/// スロットを描くチャンネルを作るときの作り（名前・種類・色空間・何も描いていない所の値）。ひな形にあるスロットはひな形と同じ、
/// ほかはスロットの読み方から: マスクはスカラー（既定の白・黒）、色は sRGB の色（既定の白・黒。黒のスロットは透明）、法線は平らな法線。
/// マットキャップの絵（プロジェクトの画像）は描くものではないので None。
pub fn paint_channel_spec(slot: &liltoon::Slot, lang: Lang) -> Option<ChannelInfo> {
    if let Some(t) = TEMPLATE.iter().find(|t| t.slot == slot.name) {
        return Some(ChannelInfo {
            name: lang.pick(t.ja, t.en).to_owned(),
            kind: t.kind,
            color_space: t.space,
            default: t.default,
        });
    }
    let name = slot.label(lang).to_owned();
    let white = slot.default != liltoon::SlotDefault::Black;
    match slot.usage {
        liltoon::SlotUse::Mask => Some(ChannelInfo {
            name,
            kind: ChannelKind::Scalar,
            color_space: ColorSpace::Linear,
            default: if white { WHITE } else { Rgba8::new(0, 0, 0, 255) },
        }),
        liltoon::SlotUse::Color => Some(ChannelInfo {
            name,
            kind: ChannelKind::Color,
            color_space: ColorSpace::Srgb,
            default: if white { WHITE } else { CLEAR },
        }),
        liltoon::SlotUse::Normal => Some(ChannelInfo {
            name,
            kind: ChannelKind::Normal,
            color_space: ColorSpace::Linear,
            default: Rgba8::new(128, 128, 255, 255),
        }),
        liltoon::SlotUse::Image => None,
    }
}

/// スロットを描くときに描くチャンネル（今の割り当てのチャンネル。詰め合わせは最初の成分のチャンネル）。割り当てが無いか、
/// 無いチャンネルを指していれば None。
pub fn slot_channel(doc: &Document, slot: &str) -> Option<Channel> {
    let source = doc.drawn_look().textures.get(slot)?;
    source
        .channels()
        .into_iter()
        .find(|c| doc.channel_info(*c).is_some())
}

/// スロットを描く口: 割り当てがあればそのチャンネルを返す（文書を変えない）。無ければ、そのスロットを描くチャンネルを作って
/// 割り当て（ひな形と同じ作り方。1 回の Undo）、そのチャンネルを返す。見た目が lilToon でなければ lilToon にもする（同じ 1 回）。
pub fn paint_slot(doc: &mut Document, slot: &str, lang: Lang) -> Result<Channel, CoreError> {
    if let Some(c) = slot_channel(doc, slot) {
        if doc.drawn_look().kind == LookKind::LilToon {
            return Ok(c);
        }
    }
    let Some(spec) = liltoon::slot(slot).and_then(|s| paint_channel_spec(s, lang)) else {
        return Err(CoreError::InvalidArgument("描けないスロット"));
    };
    doc.batch(|d| {
        let mut look = d.look().clone();
        ensure_liltoon(d, &mut look);
        let channel = match slot_channel(d, slot) {
            Some(c) => c,
            None => {
                let name = free_name(d, &spec.name);
                let c = d.add_channel(ChannelInfo { name, ..spec.clone() })?;
                look.textures.insert(slot.into(), TextureSource::Channel(c));
                c
            }
        };
        d.set_look(look, false)?;
        Ok(channel)
    })
}

/// 描く見た目が lilToon でなければ、設定（`look`）を lilToon にする（スロットが 1 つも無ければ既定の割り当ても足す）。
fn ensure_liltoon(d: &Document, look: &mut MaterialLook) {
    if d.drawn_look().kind != LookKind::LilToon {
        look.kind = LookKind::LilToon;
        look.kind_chosen = d.received_look().is_some();
        if d.drawn_look().textures.is_empty() {
            default_textures(look);
        }
    }
}

/// スロットの読むチャンネルを、新しいユーザーチャンネルにする（1 回の Undo。割り当てがあっても新しく作る。古いチャンネルは、
/// ほかのスロットが読んでいるかもしれないので消さない）。見た目が lilToon でなければ lilToon にもする（同じ 1 回）。
///
/// - `plane` が None: スロット全体を新しいチャンネルの割り当てにする。作りは `paint_channel_spec`（描く口と同じ。マットキャップの絵の
///   スロットは描くものではないので断る）。
/// - `plane` が Some(k): 成分ごとの詰め合わせの k 番目の成分だけを、新しいスカラーのチャンネルにする（名前は「描く口と同じ名前 R/G/B/A」。
///   何も描いていない所は、置き換える成分が 0・1 ならその値、チャンネルならスロットの既定のその成分）。詰め合わせでないスロットは断る。
///
/// 3D ビューが持つユーザーチャンネルの上限（16）は作る側では見ない（超えた分は、欄が「描かない」の印を出す）。作ったチャンネルを返す。
pub fn new_slot_channel(doc: &mut Document, slot: &str, plane: Option<u8>, lang: Lang) -> Result<Channel, CoreError> {
    let Some(info) = liltoon::slot(slot) else {
        return Err(CoreError::InvalidArgument("知らないスロット"));
    };
    let planes = match plane {
        None => None,
        Some(_) => match doc.drawn_look().textures.get(slot) {
            Some(TextureSource::Packed(p)) => Some(*p),
            _ => return Err(CoreError::InvalidArgument("成分ごとの割り当てではない")),
        },
    };
    let spec = match (plane, planes) {
        (None, _) => paint_channel_spec(info, lang).ok_or(CoreError::InvalidArgument("描けないスロット"))?,
        (Some(k), Some(planes)) if k < 4 => {
            // 何も描いていない所は、置き換える成分が定数ならその値（見た目を変えない）、チャンネルならスロットの既定のその成分
            let white = match planes[k as usize] {
                PlaneSource::Zero => false,
                PlaneSource::One => true,
                PlaneSource::Channel { .. } => info.default.rgba()[k as usize] > 0.5,
            };
            let base = paint_channel_spec(info, lang).map_or_else(|| info.label(lang).to_owned(), |spec| spec.name);
            ChannelInfo {
                name: format!("{base} {}", ["R", "G", "B", "A"][k as usize]),
                kind: ChannelKind::Scalar,
                color_space: ColorSpace::Linear,
                default: if white { WHITE } else { Rgba8::new(0, 0, 0, 255) },
            }
        }
        _ => return Err(CoreError::InvalidArgument("成分は 0〜3")),
    };
    doc.batch(|d| {
        let mut look = d.look().clone();
        ensure_liltoon(d, &mut look);
        let name = free_name(d, &spec.name);
        let channel = d.add_channel(ChannelInfo { name, ..spec.clone() })?;
        let source = match (plane, planes) {
            (Some(k), Some(mut p)) => {
                p[k as usize] = PlaneSource::Channel { channel, component: 0 };
                TextureSource::Packed(p)
            }
            _ => TextureSource::Channel(channel),
        };
        look.textures.insert(slot.into(), source);
        d.set_look(look, false)?;
        Ok(channel)
    })
}

/// チャンネルを読んでいる lilToon のスロット（描く見た目の。スロットの並び）。チャンネルの欄の印に使う。
pub fn slots_reading(doc: &Document, channel: Channel) -> Vec<&'static liltoon::Slot> {
    let look = doc.drawn_look();
    if look.kind != LookKind::LilToon {
        return Vec::new();
    }
    liltoon::SLOTS
        .iter()
        .filter(|s| look.textures.get(s.name).is_some_and(|t| t.channels().contains(&channel)))
        .collect()
}

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
        let outline = liltoon::shader_info(&drawn).outline;
        let mut on_now = drawn.clone();
        if !outline && !TEMPLATE_TOGGLES.iter().any(|t| liltoon::on(&drawn, t)) {
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
        if let LookOp::PaintSlot(slot) = op {
            let lang = self.lang;
            let before = self.doc.revision();
            match paint_slot(&mut self.doc, slot, lang) {
                Ok(channel) => {
                    if self.doc.revision() != before {
                        self.modified = true;
                    }
                    self.m2_ui(crate::m2::UiOp::PaintChannel(channel));
                }
                Err(e) => self.message = self.lang.core_error(&e),
            }
            return;
        }
        if let LookOp::NewChannel { slot, plane } = op {
            let lang = self.lang;
            match new_slot_channel(&mut self.doc, slot, plane, lang) {
                Ok(channel) => {
                    self.modified = true;
                    self.m2_ui(crate::m2::UiOp::PaintChannel(channel));
                }
                Err(e) => self.message = self.lang.core_error(&e),
            }
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
            LookOp::Values { values, drag } => {
                for (name, value) in values {
                    look.properties.insert(name.into(), value);
                }
                coalesce = drag;
            }
            LookOp::Reset(section) => {
                for name in section.props() {
                    look.properties.remove(name);
                }
                // シェーダーの名前の 2 つの部分: 描画モードは基本設定の節（その見出しのすぐ上の行）、輪郭線の入切は輪郭線設定の節の頭。
                // 戻した名前が受けた値（無ければ既定）と同じなら、利用者の設定から外す
                if matches!(section, Section::Base | Section::Outline) && !look.shader.is_empty() {
                    let base = self
                        .doc
                        .received_look()
                        .map(|r| liltoon::shader_info(&r.look))
                        .unwrap_or(liltoon::shader_info(&MaterialLook::default()));
                    let now = liltoon::shader_info(&drawn);
                    let (mode, outline) = if section == Section::Base {
                        (base.mode, now.outline)
                    } else {
                        (now.mode, base.outline)
                    };
                    look.shader = if (mode, outline) == (base.mode, base.outline) {
                        String::new()
                    } else {
                        liltoon::shader_name(mode, outline)
                    };
                }
            }
            LookOp::LightingPreset(preset) => {
                for (name, value) in preset.values() {
                    look.properties.insert(name.into(), LookValue::Float(value));
                }
            }
            LookOp::Template | LookOp::PaintSlot(_) | LookOp::NewChannel { .. } => unreachable!("上で扱った"),
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
