//! M2 の画面の状態と文書の操作: レイヤーの種類（グループ・塗りつぶし・調整）・マスク・クリッピング・チャンネルごとの合成・文書のチャンネルの一覧と、
//! 全部入りのブラシ（`Brush`）の設定。計算・検証・履歴は core に任せ、ここは「どの操作をどの順で当てるか」と画面の覚えだけを持つ。
//! 文書を変える操作は `Action::M2(Edit)`（1 つが 1 回の Undo。スライダーのドラッグは core がまとめる）、画面だけの操作は `Action::M2Ui(UiOp)`。

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use yolu_core::LayerLocks;

use crate::engine::{
    AdjustmentSettings, AntiAlias, BlendMode, Brush, BrushEffect, BrushPreset, Channel,
    ChannelBlend, ChannelInfo, ChannelKind, ColorSpace, CoreError, Document, DualBrush,
    DualBrushMode, LayerId, LayerKind, NormalSettings, NormalYDirection, PaperTexture, Rgba8,
    Stroke, TextureMode,
};
use crate::lang::Lang;
use crate::layerops::Xform;
use crate::notice::Source;
use crate::state::AppState;

/// 調整レイヤーの種類（新しく足すときの選択肢）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdjustmentKind {
    Invert,
    Levels,
    HueSaturation,
    GradientMap,
    ToneCurve,
    ColorBalance,
    BrightnessContrast,
    Threshold,
    Posterize,
}

impl AdjustmentKind {
    pub const ALL: [AdjustmentKind; 9] = [
        AdjustmentKind::Invert,
        AdjustmentKind::Levels,
        AdjustmentKind::HueSaturation,
        AdjustmentKind::GradientMap,
        AdjustmentKind::ToneCurve,
        AdjustmentKind::ColorBalance,
        AdjustmentKind::BrightnessContrast,
        AdjustmentKind::Threshold,
        AdjustmentKind::Posterize,
    ];

    /// 既定の値の設定。
    pub fn settings(self) -> AdjustmentSettings {
        match self {
            AdjustmentKind::Invert => AdjustmentSettings::invert(),
            AdjustmentKind::Levels => {
                AdjustmentSettings::levels(0.0, 1.0, 1.0, 0.0, 1.0).expect("レベル補正の既定値")
            }
            AdjustmentKind::HueSaturation => {
                AdjustmentSettings::hue_saturation(0.0, 0.0, 0.0).expect("色相・彩度の既定値")
            }
            other => crate::engine::ColorAdjust::default_for(other.adjustment_type())
                .expect("色調補正の既定値")
                .into_settings(),
        }
    }

    /// 保存形式の種類。
    pub fn adjustment_type(self) -> crate::engine::AdjustmentType {
        use crate::engine::AdjustmentType as T;
        match self {
            AdjustmentKind::Invert => T::Invert,
            AdjustmentKind::Levels => T::Levels,
            AdjustmentKind::HueSaturation => T::HueSaturation,
            AdjustmentKind::GradientMap => T::GradientMap,
            AdjustmentKind::ToneCurve => T::ToneCurve,
            AdjustmentKind::ColorBalance => T::ColorBalance,
            AdjustmentKind::BrightnessContrast => T::BrightnessContrast,
            AdjustmentKind::Threshold => T::Threshold,
            AdjustmentKind::Posterize => T::Posterize,
        }
    }

    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            AdjustmentKind::Invert => lang.pick("階調の反転", "Invert"),
            AdjustmentKind::Levels => lang.pick("レベル補正", "Levels"),
            AdjustmentKind::HueSaturation => lang.pick("色相・彩度", "Hue / Saturation"),
            AdjustmentKind::GradientMap => lang.pick("グラデーションマップ", "Gradient Map"),
            AdjustmentKind::ToneCurve => lang.pick("トーンカーブ", "Tone Curve"),
            AdjustmentKind::ColorBalance => lang.pick("カラーバランス", "Color Balance"),
            AdjustmentKind::BrightnessContrast => {
                lang.pick("明るさ・コントラスト", "Brightness / Contrast")
            }
            AdjustmentKind::Threshold => lang.pick("2 値化", "Threshold"),
            AdjustmentKind::Posterize => lang.pick("ポスタリゼーション", "Posterize"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            AdjustmentKind::Invert => "invert_colors",
            AdjustmentKind::Levels => "contrast",
            AdjustmentKind::HueSaturation => "palette",
            AdjustmentKind::GradientMap => "tools/gradient",
            AdjustmentKind::ToneCurve => "ink_stroke",
            AdjustmentKind::ColorBalance => "tune",
            AdjustmentKind::BrightnessContrast => "light_mode",
            AdjustmentKind::Threshold => "vignette",
            AdjustmentKind::Posterize => "grid_dots",
        }
    }

    pub fn of(settings: &AdjustmentSettings) -> AdjustmentKind {
        use crate::engine::AdjustmentType as T;
        match settings.kind() {
            T::Invert => AdjustmentKind::Invert,
            T::Levels => AdjustmentKind::Levels,
            T::HueSaturation => AdjustmentKind::HueSaturation,
            T::GradientMap => AdjustmentKind::GradientMap,
            T::ToneCurve => AdjustmentKind::ToneCurve,
            T::ColorBalance => AdjustmentKind::ColorBalance,
            T::BrightnessContrast => AdjustmentKind::BrightnessContrast,
            T::Threshold => AdjustmentKind::Threshold,
            T::Posterize => AdjustmentKind::Posterize,
        }
    }
}

/// 文書を変える操作（どれも 1 回の Undo。断られたら何も変えず、理由をステータスバーへ）。
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// 選んでいるレイヤーのすぐ上に足す（選ぶのは足したもの）。
    NewGroup,
    NewFill,
    NewAdjustment(AdjustmentKind),
    /// 選んでいるレイヤーを新しいグループに入れる。
    GroupSelected,
    /// グループをほどく（中身は元の並びのまま外へ）。
    Ungroup(LayerId),
    Duplicate(LayerId),
    /// ドラッグの並べ替え・グループへの出し入れ。position は parent の子の中の位置（0 が一番下）。
    Move {
        id: LayerId,
        parent: Option<LayerId>,
        position: usize,
    },
    Clipping(LayerId, bool),
    /// channel が None ならレイヤーの値、Some ならそのチャンネルだけの値。
    Opacity {
        id: LayerId,
        channel: Option<Channel>,
        value: f64,
    },
    BlendMode {
        id: LayerId,
        channel: Option<Channel>,
        mode: BlendMode,
    },
    /// そのチャンネルにレイヤーの自分の合成を持たせる（今のレイヤーの値から始める）・レイヤーの値に戻す。
    OwnBlend {
        id: LayerId,
        channel: Channel,
        own: bool,
    },
    ChannelEnabled {
        id: LayerId,
        channel: Channel,
        enabled: bool,
    },
    AddMask(LayerId),
    RemoveMask(LayerId),
    MaskEnabled(LayerId, bool),
    MaskInverted(LayerId, bool),
    MaskDensity(LayerId, f64),
    /// 塗りつぶしレイヤーのチャンネルの値（None で外す）。
    FillValue {
        id: LayerId,
        channel: Channel,
        value: Option<Rgba8>,
    },
    Adjust {
        id: LayerId,
        settings: AdjustmentSettings,
    },
    AddChannel(ChannelInfo),
    SetChannel {
        channel: Channel,
        info: ChannelInfo,
    },
    RemoveChannel(Channel),
    /// Ctrl+E: 複数選んでいれば選んだレイヤーを結合・グループならグループを結合・そうでなければ下のレイヤーと結合。
    MergeDown,
    /// Ctrl+Shift+E: 見えているレイヤーを 1 枚にする。
    MergeVisible,
    /// 見た目が変わると確かめた結合を、そのまま行う・やめる。
    ConfirmMerge,
    CancelMerge,
    /// 選んでいるグループをほどく（グループでなければ断る）。
    UngroupSelected,
    /// 選んでいるレイヤー（グループなら中身ごと）の複製・表示の切り替え。
    DuplicateSelected,
    ToggleSelectedVisible,
    /// ドラッグで選んだレイヤーをまとめて動かす（`position` は `parent` の子の中の位置。0 が一番下）。
    MoveLayers {
        ids: Vec<LayerId>,
        parent: Option<LayerId>,
        position: usize,
    },
    /// レイヤーのロックを付ける・外す（`flag` は個別の 1 種。全部をまとめて外すときは 15）。
    Lock {
        ids: Vec<LayerId>,
        flag: LayerLocks,
        on: bool,
    },
    /// 選んでいるレイヤーの移動・90° 回転・反転・数値と手のドラッグの変形。
    Transform(Xform),
    /// 文書の Normal の出力の設定（Height → Normal・強さ・端・ファイルの Y の向き）。レイヤーの合成は変えない。`coalesce` はスライダーの
    /// ドラッグ（離したとき 1 回の Undo にまとめる）。
    NormalSettings {
        settings: NormalSettings,
        coalesce: bool,
    },
}

/// ブラシの選択肢（ポップアップから選んだもの）。スライダーとスイッチは画面が `AppState::m2.brush` を直に変える。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrushOp {
    Effect(EffectKind),
    /// 筆先の画像（None は丸い先端）。
    Tip(Option<&'static str>),
    /// 同梱の Krita の筆先（`brushes::store::krita()` の添字。ホースなら複数の筆先と選び方も入る）。
    KritaTip(usize),
    /// 紙の質感の画像（None は無し）。
    Texture(Option<&'static str>),
    /// 取り込んだ模様（模様から作ったブラシの質感の画像）を紙の質感にする。
    PatternTexture(crate::brushes::BrushKey),
    TextureMode(TextureMode),
    DualTip(Option<&'static str>),
    DualMode(DualBrushMode),
    DualEnabled(bool),
    /// 縁のアンチエイリアス（基本の値 `AppState::brush` に持つ）。
    AntiAlias(AntiAlias),
}

/// 紙の質感の画像を替える（今の質感があれば深さ・スケール・合わせ方は残し、無ければ最後に使った設定で付ける）。
fn put_texture_image(
    brush: &mut Brush,
    prefs: TexturePrefs,
    image: std::sync::Arc<yolu_core::BrushTip>,
) {
    brush.texture = Some(match brush.texture.take() {
        Some(t) => PaperTexture { image, ..t },
        None => PaperTexture {
            image,
            depth: prefs.depth,
            scale: prefs.scale,
            mode: prefs.mode,
        },
    });
}

/// 効果のブラシの種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    Paint,
    Blur,
    Smudge,
    Clone,
}

impl EffectKind {
    pub const ALL: [EffectKind; 4] = [
        EffectKind::Paint,
        EffectKind::Blur,
        EffectKind::Smudge,
        EffectKind::Clone,
    ];
    pub fn of(effect: &BrushEffect) -> EffectKind {
        match effect {
            BrushEffect::Paint => EffectKind::Paint,
            BrushEffect::Blur { .. } => EffectKind::Blur,
            BrushEffect::Smudge { .. } => EffectKind::Smudge,
            BrushEffect::Clone { .. } => EffectKind::Clone,
        }
    }
    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            EffectKind::Paint => lang.pick("ペイント", "Paint"),
            EffectKind::Blur => lang.pick("ぼかし", "Blur"),
            EffectKind::Smudge => lang.pick("指先", "Smudge"),
            EffectKind::Clone => lang.pick("クローン", "Clone"),
        }
    }
}

/// 画面だけの操作（文書は変えない）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UiOp {
    /// 描くチャンネル（表示のチャンネルが描くチャンネルと同じだったときは、表示も一緒に替える）。
    PaintChannel(Channel),
    /// 2D のキャンバスに出すチャンネル。
    DisplayChannel(Channel),
    ToggleCollapsed(LayerId),
    /// ユーザーチャンネルの名前を変え始める。
    RenameChannel(Channel),
    /// マスクに描く・レイヤーに描く。
    EditMask(bool),
    /// 組み込みのブラシ（番号は [`presets`] の並び）。
    Preset(usize),
    Brush(BrushOp),
    Language(Lang),
    /// 移動・変形の補間（バイリニア・ニアレストネイバー）。
    Resampling(yolu_core::Resampling),
}

/// レイヤーの行の 1 つ（一覧の上から。閉じたグループの中身は含まない）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: LayerId,
    pub depth: usize,
    pub is_group: bool,
}

/// ドラッグの落とす先。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropTarget {
    /// 行と行の間（0 は一番上の行の上、行の数は一番下の行の下）。
    Gap(usize),
    /// グループの行の中ほど（その中の一番上）。
    Into(LayerId),
}

/// ドラッグの途中（動かしているレイヤーと、今の落とす先）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerDrag {
    pub id: LayerId,
    pub target: Option<DropTarget>,
}

/// 紙の質感を「なし」にしたあとも覚えておく値（また選んだとき同じ値から始める）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TexturePrefs {
    pub depth: f64,
    pub scale: f64,
    pub mode: TextureMode,
}

impl Default for TexturePrefs {
    fn default() -> Self {
        TexturePrefs {
            depth: 0.5,
            scale: 1.0,
            mode: TextureMode::Multiply,
        }
    }
}

/// M2 の画面の状態。
pub struct M2State {
    /// 全部入りのブラシ。`base`（大きさ・流量など）は `AppState::brush` が持つので使わない（ストロークのたびに上書き）。
    pub brush: Brush,
    /// 選んでいる組み込みのブラシ（[`presets`] の番号）。
    pub preset: Option<usize>,
    pub paint_channel: Channel,
    pub display_channel: Channel,
    /// 選んでいるレイヤーのマスクに描く。
    pub edit_mask: bool,
    pub collapsed: HashSet<LayerId>,
    pub dual_stash: DualBrush,
    pub texture_prefs: TexturePrefs,
    seed_state: u32,
    /// ストロークごとに乱数の種を替える（false なら 0 で、同じストロークは同じ絵になる）。
    pub random_seed: bool,
    /// 名前を変えているユーザーチャンネルと、入力欄がフォーカスを取った後か。
    pub renaming_channel: Option<Channel>,
    pub rename_channel_started: bool,
    pub channel_scroll: f32,
    /// チャンネルのパネルの中身（一覧と Normal の設定）の高さ。前のフレームのもの（スクロールの上限に使う）。
    pub channels_content: f32,
    /// プロパティの欄のスクロールと、前のフレームの中身の高さ（はみ出しの判定）。
    pub props_scroll: f32,
    pub props_content: f32,
    /// マテリアルのパネルのスクロールと、前のフレームの中身の高さ。
    pub material_scroll: f32,
    pub material_content: f32,
}

impl Default for M2State {
    fn default() -> Self {
        M2State {
            brush: Brush::default(),
            preset: None,
            paint_channel: Channel::Color,
            display_channel: Channel::Color,
            edit_mask: false,
            collapsed: HashSet::new(),
            dual_stash: DualBrush::default(),
            texture_prefs: TexturePrefs::default(),
            seed_state: 0x2545_F491,
            random_seed: true,
            renaming_channel: None,
            rename_channel_started: false,
            channel_scroll: 0.0,
            channels_content: 0.0,
            props_scroll: 0.0,
            props_content: 0.0,
            material_scroll: 0.0,
            material_content: 0.0,
        }
    }
}

impl M2State {
    /// 次のストロークの乱数の種（xorshift。同じ状態からは同じ列）。
    fn next_seed(&mut self) -> i32 {
        let mut x = self.seed_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed_state = x;
        (x & 0x7fff_ffff) as i32
    }
}

/// 組み込みのブラシ 13 個（初めて呼んだときに 1 回だけ作る）。
pub fn presets() -> &'static [BrushPreset] {
    static PRESETS: OnceLock<Vec<BrushPreset>> = OnceLock::new();
    PRESETS.get_or_init(yolu_core::builtin_presets)
}

/// 組み込みのブラシの表示名と分類。
pub fn preset_label(lang: Lang, preset: &BrushPreset) -> (&'static str, &'static str) {
    let name = match preset.id {
        "soft-round" => lang.pick("ソフト円", "Soft Round"),
        "hard-round" => lang.pick("ハード円", "Hard Round"),
        "airbrush" => lang.pick("エアブラシ", "Airbrush"),
        "pencil" => lang.pick("鉛筆", "Pencil"),
        "ink-pen" => lang.pick("インクペン", "Ink Pen"),
        "marker" => lang.pick("マーカー", "Marker"),
        "chalk" => lang.pick("チョーク", "Chalk"),
        "charcoal" => lang.pick("木炭", "Charcoal"),
        "dry-brush" => lang.pick("かすれ筆", "Dry Brush"),
        "watercolor" => lang.pick("水彩の縁", "Watercolor Edge"),
        "splatter" => lang.pick("飛沫", "Splatter"),
        "soft-eraser" => lang.pick("ソフト消しゴム", "Soft Eraser"),
        "hard-eraser" => lang.pick("ハード消しゴム", "Hard Eraser"),
        _ => preset.name,
    };
    let category = match preset.category {
        "Basic" => lang.pick("基本", "Basic"),
        "Drawing" => lang.pick("描画", "Drawing"),
        "Dry media" => lang.pick("乾いた画材", "Dry media"),
        "Paint" => lang.pick("絵の具", "Paint"),
        "Effects" => lang.pick("効果", "Effects"),
        "Erasers" => lang.pick("消しゴム", "Erasers"),
        _ => preset.category,
    };
    (name, category)
}

/// 筆先・紙の質感の画像の名前（組み込みの生成画像）。
pub fn tip_label(lang: Lang, id: &str) -> &'static str {
    match id {
        "grain" => lang.pick("粒子", "Grain"),
        "noisy-disc" => lang.pick("ざらついた円", "Noisy Disc"),
        "charcoal" => lang.pick("木炭", "Charcoal"),
        "bristles" => lang.pick("毛先", "Bristles"),
        "dots" => lang.pick("ドット", "Dots"),
        "rim" => lang.pick("縁取り", "Rim"),
        "rounded-square" => lang.pick("角丸の四角", "Rounded Square"),
        _ => "",
    }
}

/// 筆先・質感の画像の表示名（組み込みは言語ごとの名前、取り込んだ画像は画像の名前。名前が無ければ「取り込んだ画像」）。
pub fn tip_display(lang: Lang, tip: &yolu_core::BrushTip) -> String {
    let builtin = tip_label(lang, tip.name());
    if !builtin.is_empty() && yolu_core::builtin_tip(tip.name()).is_some_and(|b| *b == *tip) {
        return builtin.to_owned();
    }
    if tip.name().trim().is_empty() {
        lang.pick("取り込んだ画像", "Imported image").to_owned()
    } else {
        tip.name().to_owned()
    }
}

pub fn texture_mode_label(lang: Lang, mode: TextureMode) -> &'static str {
    match mode {
        TextureMode::Multiply => blend_label(lang, BlendMode::Multiply),
        TextureMode::Subtract => blend_label(lang, BlendMode::Subtract),
        TextureMode::Darken => blend_label(lang, BlendMode::Darken),
        TextureMode::Overlay => blend_label(lang, BlendMode::Overlay),
        TextureMode::ColorDodge => blend_label(lang, BlendMode::ColorDodge),
        TextureMode::ColorBurn => blend_label(lang, BlendMode::ColorBurn),
        TextureMode::LinearBurn => blend_label(lang, BlendMode::LinearBurn),
        TextureMode::HardMix => blend_label(lang, BlendMode::HardMix),
    }
}

/// アンチエイリアスの段の名前。
pub fn anti_alias_label(lang: Lang, level: AntiAlias) -> &'static str {
    match level {
        AntiAlias::None => lang.pick("なし", "None"),
        AntiAlias::Weak => lang.pick("弱", "Weak"),
        AntiAlias::Medium => lang.pick("中", "Medium"),
        AntiAlias::Strong => lang.pick("強", "Strong"),
    }
}

pub fn dual_mode_label(lang: Lang, mode: DualBrushMode) -> &'static str {
    match mode {
        DualBrushMode::Multiply => blend_label(lang, BlendMode::Multiply),
        DualBrushMode::Darken => blend_label(lang, BlendMode::Darken),
        DualBrushMode::Overlay => blend_label(lang, BlendMode::Overlay),
        DualBrushMode::ColorDodge => blend_label(lang, BlendMode::ColorDodge),
        DualBrushMode::ColorBurn => blend_label(lang, BlendMode::ColorBurn),
        DualBrushMode::LinearBurn => blend_label(lang, BlendMode::LinearBurn),
        DualBrushMode::HardMix => blend_label(lang, BlendMode::HardMix),
        DualBrushMode::Subtract => blend_label(lang, BlendMode::Subtract),
    }
}

/// 合成モードの名前（日本語は Unity 版と同じ、英語は Photoshop の呼び名）。
pub fn blend_label(lang: Lang, mode: BlendMode) -> &'static str {
    if lang == Lang::Ja {
        return crate::state::blend_name(mode);
    }
    match mode {
        BlendMode::Normal => "Normal",
        BlendMode::Multiply => "Multiply",
        BlendMode::Screen => "Screen",
        BlendMode::Overlay => "Overlay",
        BlendMode::Darken => "Darken",
        BlendMode::Lighten => "Lighten",
        BlendMode::ColorDodge => "Color Dodge",
        BlendMode::ColorBurn => "Color Burn",
        BlendMode::LinearDodge => "Linear Dodge (Add)",
        BlendMode::LinearBurn => "Linear Burn",
        BlendMode::HardLight => "Hard Light",
        BlendMode::SoftLight => "Soft Light",
        BlendMode::VividLight => "Vivid Light",
        BlendMode::LinearLight => "Linear Light",
        BlendMode::PinLight => "Pin Light",
        BlendMode::HardMix => "Hard Mix",
        BlendMode::Difference => "Difference",
        BlendMode::Exclusion => "Exclusion",
        BlendMode::Subtract => "Subtract",
        BlendMode::Divide => "Divide",
        BlendMode::Hue => "Hue",
        BlendMode::Saturation => "Saturation",
        BlendMode::Color => "Color",
        BlendMode::Luminosity => "Luminosity",
        BlendMode::DarkerColor => "Darker Color",
        BlendMode::LighterColor => "Lighter Color",
        BlendMode::PassThrough => "Pass Through",
    }
}

/// レイヤーに選べる合成モード（PassThrough はグループだけ。グループでは先頭）。
pub fn blend_choices(group: bool) -> Vec<BlendMode> {
    let mut v = Vec::new();
    if group {
        v.push(BlendMode::PassThrough);
    }
    v.extend(BlendMode::LAYER_MODES);
    v
}

/// ノーマルのファイルの Y の向きの名前（言語によらない）。
pub fn direction_name(direction: NormalYDirection) -> &'static str {
    match direction {
        NormalYDirection::OpenGL => "OpenGL (Y+)",
        NormalYDirection::DirectX => "DirectX (Y−)",
    }
}

/// チャンネルの表示名（標準の 6 つは日本語ならカタカナ。ユーザーチャンネルは文書の名前）。
pub fn channel_name(lang: Lang, doc: &Document, channel: Channel) -> String {
    let standard = match channel {
        Channel::Color => Some(lang.pick("カラー", "Color")),
        Channel::Roughness => Some(lang.pick("ラフネス", "Roughness")),
        Channel::Metallic => Some(lang.pick("メタリック", "Metallic")),
        Channel::Height => Some(lang.pick("ハイト", "Height")),
        Channel::Normal => Some(lang.pick("ノーマル", "Normal")),
        Channel::Emission => Some(lang.pick("エミッション", "Emission")),
        _ => None,
    };
    match standard {
        Some(name) => name.to_owned(),
        None => doc
            .channel_info(channel)
            .map(|i| i.name.clone())
            .unwrap_or_else(|| format!("#{}", channel.index())),
    }
}

pub fn channel_icon(channel: Channel) -> &'static str {
    match channel {
        Channel::Color => "palette",
        Channel::Roughness => "blur_on",
        Channel::Metallic => "contrast",
        Channel::Height => "texture",
        Channel::Normal => "3d_rotation",
        Channel::Emission => "light_mode",
        _ => "layers",
    }
}

/// チャンネルの種類の名前（新しいチャンネルの名前の元。画面の種類の欄は形式の名前 [`channel_format`]）。
pub fn kind_name(lang: Lang, kind: ChannelKind) -> &'static str {
    match kind {
        ChannelKind::Color => lang.pick("カラー", "Color"),
        ChannelKind::Scalar => lang.pick("スカラー", "Scalar"),
        ChannelKind::Normal => lang.pick("ノーマル", "Normal"),
    }
}

/// チャンネルの画素の 1 成分のビット数（画素は `Rgba8`）。
const CHANNEL_BITS: u32 = 8;

/// チャンネルの形式の名前（チャンネルの欄と種類のメニューの表記。Substance Painter のテクスチャセットのチャンネルの形式と同じ書き方:
/// sRGB8・RGB8・L8）。成分の名前（スカラーは L、カラー・ノーマルは RGB）に、色空間が sRGB なら頭に s、後ろに 1 成分のビット数。
pub fn channel_format(info: &ChannelInfo) -> String {
    let components = match info.kind {
        ChannelKind::Scalar => "L",
        ChannelKind::Color | ChannelKind::Normal => "RGB",
    };
    let srgb = match info.color_space {
        ColorSpace::Srgb => "s",
        ColorSpace::Linear => "",
    };
    format!("{srgb}{components}{CHANNEL_BITS}")
}

/// 描画色（0〜1）から塗りつぶしレイヤーの値へ（描画色のまま。アルファは 255）。バケツ・ポリゴン塗りつぶしの 1 チャンネルの値
/// （`matpaint::single_value`）と同じ変換で、チャンネルの種類によらない（Unity 版の塗りつぶしレイヤーも `GetBrush().Color`）。
pub fn fill_from_color(c: [f32; 4]) -> Rgba8 {
    crate::matpaint::single_value([c[0], c[1], c[2], 1.0])
}

/// 新しいユーザーチャンネルの情報（種類ごとの既定の色空間と値）。
pub fn new_channel_info(name: String, kind: ChannelKind) -> ChannelInfo {
    let (color_space, default) = match kind {
        ChannelKind::Color => (ColorSpace::Srgb, Rgba8::new(255, 255, 255, 255)),
        ChannelKind::Scalar => (ColorSpace::Linear, Rgba8::new(0, 0, 0, 255)),
        ChannelKind::Normal => (ColorSpace::Linear, Rgba8::new(128, 128, 255, 255)),
    };
    ChannelInfo {
        name,
        kind,
        color_space,
        default,
    }
}

/// レイヤーの種類の名前とアイコン。
pub fn layer_kind_label(lang: Lang, kind: LayerKind) -> &'static str {
    match kind {
        LayerKind::Raster => lang.pick("レイヤー", "Layer"),
        LayerKind::Fill => lang.pick("塗りつぶし", "Fill"),
        LayerKind::Adjustment => lang.pick("調整", "Adjustment"),
        LayerKind::Group => lang.pick("グループ", "Group"),
    }
}

pub fn layer_kind_icon(kind: LayerKind) -> &'static str {
    match kind {
        LayerKind::Raster => "paint_brush",
        LayerKind::Fill => "format_color_fill",
        LayerKind::Adjustment => "tune",
        LayerKind::Group => "folder",
    }
}

// ───────── 一覧の行とドラッグ ─────────

/// 一覧の行（上から。閉じたグループの中身は出さない）。
pub fn visible_rows(doc: &Document, collapsed: &HashSet<LayerId>) -> Vec<Row> {
    let parents: HashMap<LayerId, Option<LayerId>> =
        doc.layers().iter().map(|l| (l.id(), l.parent())).collect();
    let depth = |id: LayerId| {
        let mut d = 0;
        let mut at = parents.get(&id).copied().flatten();
        while let Some(p) = at {
            d += 1;
            at = parents.get(&p).copied().flatten();
            if d > parents.len() {
                break;
            }
        }
        d
    };
    let hidden = |id: LayerId| {
        let mut at = parents.get(&id).copied().flatten();
        let mut hops = 0;
        while let Some(p) = at {
            if collapsed.contains(&p) {
                return true;
            }
            at = parents.get(&p).copied().flatten();
            hops += 1;
            if hops > parents.len() {
                break;
            }
        }
        false
    };
    doc.layers()
        .iter()
        .rev()
        .filter(|l| !hidden(l.id()))
        .map(|l| Row {
            id: l.id(),
            depth: depth(l.id()),
            is_group: l.is_group(),
        })
        .collect()
}

/// `id` が `ancestor` の中（何段下でも）にあるか。
pub fn is_inside(doc: &Document, id: LayerId, ancestor: LayerId) -> bool {
    let mut at = doc.layer(id).and_then(|l| l.parent());
    let mut hops = 0;
    while let Some(p) = at {
        if p == ancestor {
            return true;
        }
        at = doc.layer(p).and_then(|l| l.parent());
        hops += 1;
        if hops > doc.layers().len() {
            break;
        }
    }
    false
}

/// レイヤー（グループなら中身ごと）の数。
pub fn subtree_len(doc: &Document, id: LayerId) -> usize {
    1 + doc
        .layers()
        .iter()
        .filter(|l| is_inside(doc, l.id(), id))
        .count()
}

/// ドラッグで落とした所の移動（動かせない所 — 自分の中・同じ所 — は None）。
pub fn drop_edit(
    doc: &Document,
    rows: &[Row],
    dragged: LayerId,
    target: DropTarget,
) -> Option<Edit> {
    let (parent, position) = match target {
        DropTarget::Into(group) => {
            if group == dragged || is_inside(doc, group, dragged) {
                return None;
            }
            let children = doc.children_of(Some(group)).ok()?;
            (
                Some(group),
                children.iter().filter(|c| **c != dragged).count(),
            )
        }
        DropTarget::Gap(k) => match rows.get(k) {
            None => (None, 0),
            Some(row) => {
                if row.id == dragged || is_inside(doc, row.id, dragged) {
                    return None;
                }
                let parent = doc.layer(row.id)?.parent();
                let siblings = doc.children_of(parent).ok()?;
                let rest: Vec<LayerId> = siblings.into_iter().filter(|c| *c != dragged).collect();
                (parent, rest.iter().position(|c| *c == row.id)? + 1)
            }
        },
    };
    Some(Edit::Move {
        id: dragged,
        parent,
        position,
    })
}

/// 複数のレイヤーのドラッグで落とした所の移動（Unity 版の `DropLayers` と同じ）。選んだレイヤー（とグループの中身）が運ばれるレイヤーで、
/// 落とす先は運ばれないレイヤーから数える: グループの中へなら、そのグループの運ばれない子の数の位置（自分たちの中へは落とせない）、
/// 線の上なら、その線のすぐ下の運ばれない最初のレイヤーの上、無ければ一番下。1 つしか運ぶものが無ければ単独のドラッグと同じ。
pub fn drop_edit_for(
    doc: &Document,
    rows: &[Row],
    dragged: &[LayerId],
    target: DropTarget,
) -> Option<Edit> {
    let members = doc.topmost_of(dragged).ok()?;
    if members.len() <= 1 {
        return members
            .first()
            .and_then(|m| drop_edit(doc, rows, *m, target));
    }
    let carried = |id: LayerId| members.iter().any(|m| *m == id || is_inside(doc, id, *m));
    match target {
        DropTarget::Into(group) => {
            if carried(group) {
                return None;
            }
            let children = doc.children_of(Some(group)).ok()?;
            let position = children.iter().filter(|c| !carried(**c)).count();
            Some(Edit::MoveLayers {
                ids: members.clone(),
                parent: Some(group),
                position,
            })
        }
        DropTarget::Gap(k) => {
            for row in rows.iter().skip(k) {
                if carried(row.id) {
                    continue;
                }
                let parent = doc.layer(row.id)?.parent();
                let rest: Vec<LayerId> = doc
                    .children_of(parent)
                    .ok()?
                    .into_iter()
                    .filter(|c| !carried(*c))
                    .collect();
                let position = rest.iter().position(|c| *c == row.id)? + 1;
                return Some(Edit::MoveLayers {
                    ids: members.clone(),
                    parent,
                    position,
                });
            }
            Some(Edit::MoveLayers {
                ids: members.clone(),
                parent: None,
                position: 0,
            })
        }
    }
}

/// 一覧の中の高さ（行の上端からの距離を行の高さで割ったもの。0 が一番上の行の上端）から落とす先を決める。
pub fn drop_target_at(
    doc: &Document,
    rows: &[Row],
    dragged: LayerId,
    position: f32,
) -> Option<DropTarget> {
    drop_target_for(doc, rows, &[dragged], position)
}

/// `drop_target_at` の、運ぶレイヤーが複数の形（選んだレイヤー）。
pub fn drop_target_for(
    doc: &Document,
    rows: &[Row],
    dragged: &[LayerId],
    position: f32,
) -> Option<DropTarget> {
    if rows.is_empty() {
        return None;
    }
    let n = rows.len();
    let index = position.floor().clamp(0.0, (n - 1) as f32) as usize;
    let within = position - index as f32;
    let row = rows[index];
    let target = if position >= n as f32 {
        DropTarget::Gap(n)
    } else if position < 0.0 {
        DropTarget::Gap(0)
    } else if row.is_group && within > 0.3 && within < 0.7 {
        DropTarget::Into(row.id)
    } else if within < 0.5 {
        DropTarget::Gap(index)
    } else {
        DropTarget::Gap(index + 1)
    };
    drop_edit_for(doc, rows, dragged, target).map(|_| target)
}

// ───────── 操作 ─────────

impl AppState {
    /// 描いている間・読むだけのセットでは何もしない（`Action::apply` が先に断る）。
    pub fn m2_edit(&mut self, edit: Edit) {
        if self.is_stroking() {
            self.refuse(
                Source::Layer,
                crate::lang::refusals::during_stroke(self.lang),
            );
            return;
        }
        let revision = self.doc.revision();
        match self.m2_apply(edit) {
            Ok(()) => {}
            // ロックで断られたときは、どのロックか（と、親のグループのロックか）を言う短い文（ブラシ・バケツと同じ）
            Err(e @ CoreError::LayerLocked { .. }) => self.notify(
                crate::notice::Kind::of_core(&e),
                Source::Layer,
                self.lang.core_error(&e),
            ),
            Err(e) => self.notify(
                crate::notice::Kind::of_core(&e),
                Source::Layer,
                self.lang.core_error(&e),
            ),
        }
        if self.doc.revision() != revision {
            self.modified = true;
        }
        self.ensure_selection();
    }

    /// 選んでいるレイヤーの上に足すときの基準（無ければ一番上）。
    fn above_selected(&self) -> Option<LayerId> {
        self.selected_layer
            .filter(|id| self.doc.layer(*id).is_some())
    }

    pub(crate) fn new_layer_name(&self, kind: LayerKind) -> String {
        let n = self
            .doc
            .layers()
            .iter()
            .filter(|l| l.kind() == kind)
            .count()
            + 1;
        format!("{} {n}", layer_kind_label(self.lang, kind))
    }

    pub(crate) fn select_new(&mut self, id: LayerId) {
        self.selected_layer = Some(id);
        self.set_edit_mask(false);
    }

    /// 描く先をマスクにする・やめる（`edit_mask` を替えるのはここだけ）。マスクに描くあいだは、ツールプロパティの「塗るチャンネル」の区分が
    /// レイヤーマスクの欄に替わる（`panels::tool_props`）。
    /// 選んだ効果の行は、マスクを描き始めるとき閉じ、やめるときはマスクのスタックの行だけ閉じる（一覧に出ない行を選んだままにしない。
    /// 画素の効果の行を選んだ状態は、レイヤーの画素が対象なので残す）。
    pub fn set_edit_mask(&mut self, on: bool) {
        self.m2.edit_mask = on;
        if on {
            self.fx.selected = None; // マスクの欄へ移る（選んだ効果の欄は閉じる）
        } else if self.fx.in_mask(&self.doc) {
            self.fx.selected = None;
        }
    }

    fn m2_apply(&mut self, edit: Edit) -> Result<(), CoreError> {
        let above = self.above_selected();
        match edit {
            Edit::NewGroup => {
                let name = self.new_layer_name(LayerKind::Group);
                let id = self.doc.add_group(&name, above)?;
                self.select_new(id);
            }
            Edit::NewFill => {
                let name = self.new_layer_name(LayerKind::Fill);
                let value = fill_from_color(self.color.main);
                let id =
                    self.doc
                        .add_fill_layer(&name, &[(self.m2.paint_channel, value)], above)?;
                self.select_new(id);
            }
            Edit::NewAdjustment(kind) => {
                let name = kind.name(self.lang).to_owned();
                let id = self
                    .doc
                    .add_adjustment_layer(&name, kind.settings(), None, above)?;
                self.select_new(id);
            }
            Edit::GroupSelected => self.group_selected_layers()?,
            Edit::Ungroup(id) => {
                let first = self.doc.children_of(Some(id))?.last().copied();
                // グループの定規は、グループの中でだけ意味を持つので、グループと一緒に外れる（取り消しで戻る）。黙って捨てずに知らせる
                let rulers = self.doc.layer(id).map_or(0, |l| l.rulers().len());
                self.doc.ungroup(id)?;
                self.m2.collapsed.remove(&id);
                self.selected_layer = first;
                self.set_edit_mask(false);
                if rulers > 0 {
                    self.info(
                        Source::Layer,
                        self.lang.pick(
                            format!("グループの定規 {rulers} 個も一緒に削除しました"),
                            format!("Also deleted the group's {rulers} ruler(s)"),
                        ),
                    );
                }
            }
            Edit::Duplicate(id) => {
                let copy = self.doc.duplicate_layer(id, None)?;
                self.select_new(copy);
            }
            Edit::Move {
                id,
                parent,
                position,
            } => {
                self.doc.move_layer_to(id, parent, position)?;
                // 閉じたグループへ入れたレイヤーが見えなくならないよう、入れた先を開く
                if let Some(p) = parent {
                    self.m2.collapsed.remove(&p);
                }
            }
            Edit::Clipping(id, v) => self.doc.set_layer_clipping(id, v)?,
            Edit::Opacity { id, channel, value } => match channel {
                None => self.doc.set_layer_opacity(id, value, true)?,
                Some(c) => self.doc.set_channel_opacity(id, c, Some(value), true)?,
            },
            Edit::BlendMode { id, channel, mode } => match channel {
                None => self.doc.set_layer_blend_mode(id, mode)?,
                Some(c) => self.doc.set_channel_blend_mode(id, c, Some(mode))?,
            },
            Edit::OwnBlend { id, channel, own } => {
                let layer = self.doc.layer(id).ok_or(CoreError::LayerNotFound)?;
                let blend = if own {
                    ChannelBlend::new(Some(layer.blend_mode()), Some(layer.opacity()))
                } else {
                    ChannelBlend::default()
                };
                self.doc.set_channel_blend(id, channel, blend, false)?;
            }
            Edit::ChannelEnabled {
                id,
                channel,
                enabled,
            } => self.doc.set_channel_enabled(id, channel, enabled)?,
            Edit::AddMask(id) => {
                self.doc.add_layer_mask(id)?;
                self.selected_layer = Some(id);
                self.set_edit_mask(true);
            }
            Edit::RemoveMask(id) => {
                self.doc.remove_layer_mask(id)?;
                if self.selected_layer == Some(id) {
                    self.set_edit_mask(false);
                }
            }
            Edit::MaskEnabled(id, v) => self.doc.set_layer_mask_enabled(id, v)?,
            Edit::MaskInverted(id, v) => self.doc.set_layer_mask_inverted(id, v)?,
            Edit::MaskDensity(id, v) => self.doc.set_layer_mask_density(id, v, true)?,
            Edit::FillValue { id, channel, value } => {
                self.doc.set_fill_value(id, channel, value, true)?
            }
            Edit::Adjust { id, settings } => self.doc.set_adjustment(id, settings, true)?,
            Edit::AddChannel(info) => {
                let channel = self.doc.add_channel(info)?;
                self.m2.paint_channel = channel;
                self.m2.display_channel = channel;
            }
            Edit::SetChannel { channel, info } => self.doc.set_channel_info(channel, info)?,
            Edit::RemoveChannel(channel) => crate::look::remove_channel(&mut self.doc, channel)?,
            Edit::MergeDown => self.merge_down_selected()?,
            Edit::MergeVisible => self.merge_visible_layers()?,
            Edit::ConfirmMerge => self.confirm_merge()?,
            Edit::CancelMerge => self.cancel_merge(),
            Edit::UngroupSelected => {
                let id = self
                    .selected_layer
                    .filter(|id| self.doc.layer(*id).is_some_and(|l| l.is_group()))
                    .ok_or(CoreError::Unsupported("グループではない"))?;
                return self.m2_apply(Edit::Ungroup(id));
            }
            Edit::DuplicateSelected => self.duplicate_selected_layers()?,
            Edit::ToggleSelectedVisible => self.toggle_selected_visibility()?,
            Edit::MoveLayers {
                ids,
                parent,
                position,
            } => self.move_selected_layers(&ids, parent, position)?,
            Edit::Lock { ids, flag, on } => self.change_locks(&ids, flag, on)?,
            Edit::Transform(x) => {
                self.apply_xform(x)?;
            }
            Edit::NormalSettings { settings, coalesce } => {
                let old = self.doc.normal_settings();
                self.doc.set_normal_settings(settings, coalesce)?;
                let lang = self.lang;
                if settings.derive_from_height() != old.derive_from_height() {
                    self.info(
                        Source::Channel,
                        if settings.derive_from_height() {
                            lang.pick("ハイト → ノーマルをオンにしました。", "Height → Normal on.")
                        } else {
                            lang.pick(
                                "ハイト → ノーマルをオフにしました。",
                                "Height → Normal off.",
                            )
                        },
                    );
                } else if settings.file_direction() != old.file_direction() {
                    self.info(
                        Source::Channel,
                        lang.pick(
                            format!(
                                "ノーマルのファイル: {}",
                                direction_name(settings.file_direction())
                            ),
                            format!(
                                "Normal files: {}",
                                direction_name(settings.file_direction())
                            ),
                        ),
                    );
                }
            }
        }
        Ok(())
    }

    /// スライダーのドラッグを終える（まとめていた変更を 1 回の Undo にする）。
    pub fn m2_end_drag(&mut self) {
        // 打っている文字のまとめ（打った分を 1 回の取り消し）は、打ち終わりで終える
        if self.text.editing.is_some() {
            return;
        }
        self.doc.end_coalescing();
    }

    /// スライダーのドラッグを押したまま Esc で止める: まとめていた変更を戻して、その段ごと捨てる（Undo の段を残さない）。
    /// まとめている変更が無ければ何もしない。
    pub fn m2_cancel_drag(&mut self) {
        if self.is_stroking() {
            return;
        }
        let revision = self.doc.revision();
        match self.doc.cancel_coalescing() {
            Ok(true) => {
                crate::automation::record::drag_cancelled(self, revision);
                self.info(
                    Source::Layer,
                    self.lang.pick("取り消しました。", "Cancelled."),
                );
            }
            Ok(false) => {}
            Err(e) => self.notify(
                crate::notice::Kind::of_core(&e),
                Source::Layer,
                self.lang.core_error(&e),
            ),
        }
    }

    /// 画面だけの操作。
    pub fn m2_ui(&mut self, op: UiOp) {
        let stroking = self.is_stroking();
        let refuse = |s: &mut AppState| {
            s.refuse(Source::Layer, crate::lang::refusals::during_stroke(s.lang))
        };
        match op {
            UiOp::PaintChannel(channel) => {
                if stroking {
                    return refuse(self);
                }
                if self.doc.channel_info(channel).is_some() {
                    if self.m2.display_channel == self.m2.paint_channel {
                        self.m2.display_channel = channel;
                    }
                    self.m2.paint_channel = channel;
                }
            }
            UiOp::DisplayChannel(channel) => {
                if self.doc.channel_info(channel).is_some() {
                    self.m2.display_channel = channel;
                }
            }
            UiOp::ToggleCollapsed(id) => {
                if !self.m2.collapsed.remove(&id) {
                    self.m2.collapsed.insert(id);
                    // 選んでいるレイヤーが閉じたグループの中なら、グループを選ぶ（見えない物を選んだままにしない）
                    if let Some(sel) = self.selected_layer {
                        if is_inside(&self.doc, sel, id) {
                            self.selected_layer = Some(id);
                            self.set_edit_mask(false);
                        }
                    }
                }
            }
            UiOp::RenameChannel(channel) => {
                if !channel.is_standard() && self.can_edit() {
                    self.m2.renaming_channel = Some(channel);
                    self.m2.rename_channel_started = false;
                }
            }
            UiOp::EditMask(on) => {
                if stroking {
                    return refuse(self);
                }
                let has_mask = self
                    .selected_layer
                    .and_then(|id| self.doc.layer(id))
                    .is_some_and(|l| l.mask().is_some());
                self.set_edit_mask(on && has_mask); // マスクを描くと決めたら、マスクの設定が見えるように
            }
            UiOp::Preset(index) => {
                // 一覧の組み込みのブラシ（core の組み込みの番号）に替える。描画色・背景色・手ぶれ補正と入り抜きは残る
                if stroking {
                    return refuse(self);
                }
                if let Some(preset) = presets().get(index) {
                    self.brush_action(crate::brushes::BrushAction::Select(
                        crate::brushes::BrushKey::Builtin(preset.id),
                    ));
                }
            }
            UiOp::Brush(op) => {
                if stroking {
                    return refuse(self);
                }
                self.apply_brush_op(op);
            }
            UiOp::Language(lang) => self.set_language(lang),
            UiOp::Resampling(mode) => self.transform.resampling = mode,
        }
    }

    fn apply_brush_op(&mut self, op: BrushOp) {
        if let BrushOp::AntiAlias(level) = op {
            self.brush.anti_alias = level;
            return;
        }
        let b = &mut self.m2.brush;
        match op {
            BrushOp::Effect(kind) => {
                let before = EffectKind::of(&b.effect);
                b.effect = match kind {
                    EffectKind::Paint => BrushEffect::Paint,
                    EffectKind::Blur => match b.effect {
                        BrushEffect::Blur { .. } => b.effect,
                        _ => BrushEffect::BLUR,
                    },
                    EffectKind::Smudge => match b.effect {
                        BrushEffect::Smudge { .. } => b.effect,
                        _ => BrushEffect::SMUDGE,
                    },
                    EffectKind::Clone => match b.effect {
                        BrushEffect::Clone { .. } => b.effect,
                        _ => BrushEffect::Clone {
                            offset: yolu_core::glam::DVec2::new(64.0, 0.0),
                        },
                    },
                };
                // 種類を替えたときだけ、新しい効果をブラシの持つ値として覚え直す（2D の揃える offset が決まっていれば入れ直す）。同じ種類を
                // 選び直しても、元から入れた offset はそのまま（「変えた」に数えない）
                if before != kind {
                    self.clone.brush_loaded(&mut self.m2.brush.effect);
                }
                // 効果のブラシは消しゴムにできない。描くツールへ戻す
                if kind != EffectKind::Paint && self.tool.erases() {
                    self.tool = crate::state::Tool::Brush;
                }
            }
            BrushOp::Tip(id) => {
                b.tip.image = id.and_then(yolu_core::builtin_tip);
                // ホース（複数の筆先）が残っていると、1 枚の画像の選びが効かない
                b.tip.images.clear();
            }
            BrushOp::KritaTip(index) => {
                if let Some(krita) = crate::brushes::store::krita().brushes.get(index) {
                    b.tip.image = krita.brush.tip.image.clone();
                    b.tip.images = krita.brush.tip.images.clone();
                    b.tip.selection = krita.brush.tip.selection;
                }
            }
            BrushOp::PatternTexture(key) => {
                let image = self
                    .brushes
                    .lib
                    .entry(key)
                    .and_then(|e| e.baseline.texture.as_ref())
                    .map(|t| t.image.clone());
                if let Some(image) = image {
                    put_texture_image(b, self.m2.texture_prefs, image);
                }
            }
            BrushOp::Texture(None) => {
                if let Some(t) = &b.texture {
                    self.m2.texture_prefs = TexturePrefs {
                        depth: t.depth,
                        scale: t.scale,
                        mode: t.mode,
                    };
                }
                b.texture = None;
            }
            BrushOp::Texture(Some(id)) => {
                if let Some(image) = yolu_core::builtin_tip(id) {
                    put_texture_image(b, self.m2.texture_prefs, image);
                }
            }
            BrushOp::TextureMode(mode) => {
                if let Some(t) = &mut b.texture {
                    t.mode = mode;
                }
                self.m2.texture_prefs.mode = mode;
            }
            BrushOp::DualEnabled(on) => {
                if on {
                    if b.dual.is_none() {
                        b.dual = Some(self.m2.dual_stash.clone());
                    }
                } else if let Some(d) = b.dual.take() {
                    self.m2.dual_stash = d;
                }
            }
            BrushOp::DualTip(id) => {
                if let Some(d) = &mut b.dual {
                    d.tip = id.and_then(yolu_core::builtin_tip);
                }
            }
            BrushOp::DualMode(mode) => {
                if let Some(d) = &mut b.dual {
                    d.mode = mode;
                }
            }
            // 基本の値なので先に当てた
            BrushOp::AntiAlias(_) => {}
        }
    }

    /// 今のツールとブラシの設定から、ストロークに渡す全部入りのブラシ（大きさ・色などの基本は画面の値、背景色は副の色、
    /// 消しゴムのときは効果を外す）。
    pub fn stroke_brush(&mut self, eraser: bool) -> Brush {
        let mut brush = self.m2.brush.clone();
        brush.base = self.stroke_settings(eraser);
        let sub = self.color.sub.map(crate::state::to_byte);
        brush.color.secondary = Rgba8::new(sub[0], sub[1], sub[2], 255);
        brush.seed = if self.m2.random_seed {
            self.m2.next_seed()
        } else {
            0
        };
        if brush.base.erase {
            brush.effect = BrushEffect::Paint;
        }
        brush
    }

    /// 描き始める（マスクを選んでいればマスク、そうでなければ描くチャンネル）。
    pub fn begin_paint_stroke(&mut self, id: LayerId, eraser: bool) -> Result<Stroke, CoreError> {
        self.begin_paint_stroke_with(id, eraser, None)
    }

    /// [`AppState::begin_paint_stroke`] に、ステンシル（`canvas_stencil`・`surface_stencil` が作ったもの）を足したもの。
    pub fn begin_paint_stroke_with(
        &mut self,
        id: LayerId,
        eraser: bool,
        stencil: Option<std::sync::Arc<yolu_core::BrushStencil>>,
    ) -> Result<Stroke, CoreError> {
        let mut brush = self.stroke_brush(eraser);
        brush.stencil = stencil;
        self.begin_stroke_with(id, &brush)
    }

    /// 渡したブラシで描き始める（マスクを選んでいればマスク、そうでなければ描くチャンネル）。2D のキャンバスは対称を足したブラシで
    /// ここへ来る（`begin_canvas_stroke`）。
    pub fn begin_stroke_with(&mut self, id: LayerId, brush: &Brush) -> Result<Stroke, CoreError> {
        if self.m2.edit_mask {
            return self.doc.begin_brush_mask_stroke(id, brush);
        }
        let mut stroke = if self.paints_material() {
            // マテリアルで塗る: 組の全部のチャンネルを同じダブで 1 回のストロークに（レイヤーで無効のチャンネルは有効にする）
            let channels = self.paint_channels();
            self.doc.begin_material_brush_stroke(id, &channels, brush)?
        } else {
            self.doc
                .begin_brush_stroke_in(id, self.m2.paint_channel, brush)?
        };
        // クローンが、描くレイヤーだけでなく見えているレイヤーの重なり（チャンネルごと）を読む（最初のダブの前に凍結する。マスクには使えない）
        if self.clone.all_layers && matches!(brush.effect, BrushEffect::Clone { .. }) {
            stroke.use_composite_clone_source(&mut self.doc)?;
        }
        // 色の混ぜが「全レイヤーから」拾うブラシも、同じ参照元（見えているレイヤーの重なり）を最初のダブの前に凍結する
        // （色のチャンネルを塗らない間は、混ぜが効かないので凍結しない）
        if brush.mix.wants_composite()
            && brush.effect.is_paint()
            && !brush.base.erase
            && self.paint_channels().iter().any(|p| {
                self.doc
                    .channel_info(p.channel)
                    .is_some_and(|i| yolu_core::brush::carries_color(i.kind))
            })
        {
            stroke.use_composite_clone_source(&mut self.doc)?;
        }
        Ok(stroke)
    }

    /// 選んでいるレイヤー・チャンネルが消えていれば選び直す。
    pub fn ensure_m2_selection(&mut self) {
        if self.doc.channel_info(self.m2.paint_channel).is_none() {
            self.m2.paint_channel = Channel::Color;
        }
        if self.doc.channel_info(self.m2.display_channel).is_none() {
            self.m2.display_channel = Channel::Color;
        }
        let masked = self
            .selected_layer
            .and_then(|id| self.doc.layer(id))
            .is_some_and(|l| l.mask().is_some());
        if !masked {
            self.set_edit_mask(false);
        }
        let alive = |id: &LayerId| self.doc.layer(*id).is_some();
        self.m2.collapsed.retain(alive);
        if let Some(c) = self.m2.renaming_channel {
            if self.doc.channel_info(c).is_none() {
                self.m2.renaming_channel = None;
            }
        }
    }

    /// 描くレイヤーが描ける状態か（描けないときの短い理由）。
    pub fn paint_blocker(&self) -> Option<String> {
        let id = self.selected_layer?;
        let layer = self.doc.layer(id)?;
        let lang = self.lang;
        if self.m2.edit_mask {
            return None;
        }
        if layer.kind() != LayerKind::Raster {
            return Some(lang.with_reason(
                lang.pick("このレイヤーには描けません", "Cannot paint on this layer"),
                layer_kind_label(lang, layer.kind()),
            ));
        }
        let channel = self.m2.paint_channel;
        // マテリアルで塗るなら、無効のチャンネルは core が有効にする
        if !self.mat.enabled
            && layer.surface(channel).is_some()
            && !layer.is_channel_enabled(channel)
        {
            let channel = channel_name(lang, &self.doc, channel);
            return Some(lang.pick(
                format!(
                    "このレイヤーは{}チャンネルが無効です。",
                    lang.quote(&channel)
                ),
                format!("The {channel} channel is off on this layer."),
            ));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Action;

    fn app() -> AppState {
        AppState::new(64, 64)
    }

    fn names(s: &AppState) -> Vec<String> {
        s.doc.layers().iter().map(|l| l.name().to_owned()).collect()
    }

    /// チャンネルの種類の表記は形式の名前だけ（色空間と 1 成分のビット数から作る）。標準の 6 つと新しいチャンネルの既定の作り、
    /// リニアのカラー・sRGB のスカラー（ファイルから読める組み合わせ）も、固定の文字でなく作りから決まる。新しいチャンネルの名前の元は
    /// 種類の名前。
    #[test]
    fn the_channel_kind_reads_as_its_format_built_from_the_color_space() {
        let format = |channel| channel_format(&ChannelInfo::standard(channel).unwrap());
        assert_eq!(format(Channel::Color), "sRGB8");
        assert_eq!(format(Channel::Roughness), "L8");
        assert_eq!(format(Channel::Metallic), "L8");
        assert_eq!(format(Channel::Height), "L8");
        assert_eq!(format(Channel::Normal), "RGB8");
        assert_eq!(format(Channel::Emission), "sRGB8");
        for kind in [ChannelKind::Color, ChannelKind::Scalar, ChannelKind::Normal] {
            let info = new_channel_info("x".into(), kind);
            let standard = match kind {
                ChannelKind::Color => "sRGB8",
                ChannelKind::Scalar => "L8",
                ChannelKind::Normal => "RGB8",
            };
            assert_eq!(channel_format(&info), standard, "{kind:?}");
        }
        let linear_color = ChannelInfo {
            color_space: ColorSpace::Linear,
            ..new_channel_info("x".into(), ChannelKind::Color)
        };
        assert_eq!(channel_format(&linear_color), "RGB8");
        let srgb_scalar = ChannelInfo {
            color_space: ColorSpace::Srgb,
            ..new_channel_info("x".into(), ChannelKind::Scalar)
        };
        assert_eq!(channel_format(&srgb_scalar), "sL8");
        let s = app();
        assert_eq!(
            crate::m2_menu::new_channel_name(&s, ChannelKind::Scalar),
            "スカラー 1"
        );
    }

    #[test]
    fn group_edits_are_one_undo_each() {
        let mut s = app();
        let first = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::NewGroup));
        let group = s.selected_layer.unwrap();
        assert_eq!(s.doc.layers().len(), 2);
        assert!(s.doc.layer(group).unwrap().is_group());
        // レイヤーをグループへ入れる
        s.apply(Action::M2(Edit::Move {
            id: first,
            parent: Some(group),
            position: 0,
        }));
        assert_eq!(s.doc.layer(first).unwrap().parent(), Some(group));
        s.apply(Action::Undo);
        assert_eq!(s.doc.layer(first).unwrap().parent(), None);
        s.apply(Action::Undo);
        assert_eq!(s.doc.layers().len(), 1, "グループを足したのも 1 回で戻る");
        s.apply(Action::Redo);
        assert_eq!(s.doc.layers().len(), 2);
    }

    #[test]
    fn mask_clipping_fill_and_adjustment_edits_undo() {
        let mut s = app();
        let id = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::AddMask(id)));
        assert!(s.doc.layer(id).unwrap().mask().is_some());
        assert!(s.m2.edit_mask, "足したら描く先をマスクにする");
        s.apply(Action::M2(Edit::MaskInverted(id, true)));
        s.apply(Action::M2(Edit::MaskEnabled(id, false)));
        s.apply(Action::M2(Edit::MaskDensity(id, 0.25)));
        s.m2_end_drag();
        let mask = s.doc.layer(id).unwrap().mask().unwrap();
        assert!(mask.inverted() && !mask.enabled() && mask.density() == 0.25);
        s.apply(Action::Undo);
        assert_eq!(s.doc.layer(id).unwrap().mask().unwrap().density(), 1.0);
        s.apply(Action::M2(Edit::RemoveMask(id)));
        assert!(!s.m2.edit_mask && s.doc.layer(id).unwrap().mask().is_none());
        s.apply(Action::Undo);
        assert!(s.doc.layer(id).unwrap().mask().is_some());

        s.apply(Action::M2(Edit::Clipping(id, true)));
        assert!(s.doc.layer(id).unwrap().clipping());
        s.apply(Action::M2(Edit::NewFill));
        let fill = s.selected_layer.unwrap();
        assert_eq!(s.doc.layer(fill).unwrap().kind(), LayerKind::Fill);
        assert!(s
            .doc
            .layer(fill)
            .unwrap()
            .fill_value(Channel::Color)
            .is_some());
        s.apply(Action::M2(Edit::FillValue {
            id: fill,
            channel: Channel::Color,
            value: Some(Rgba8::new(10, 20, 30, 255)),
        }));
        s.m2_end_drag();
        assert_eq!(
            s.doc.layer(fill).unwrap().fill_value(Channel::Color),
            Some(Rgba8::new(10, 20, 30, 255))
        );
        s.apply(Action::M2(Edit::NewAdjustment(AdjustmentKind::Levels)));
        let adj = s.selected_layer.unwrap();
        assert_eq!(s.doc.layer(adj).unwrap().kind(), LayerKind::Adjustment);
        let levels = AdjustmentSettings::levels(0.1, 0.9, 1.5, 0.0, 1.0).unwrap();
        s.apply(Action::M2(Edit::Adjust {
            id: adj,
            settings: levels.clone(),
        }));
        s.m2_end_drag();
        assert_eq!(s.doc.layer(adj).unwrap().adjustment(), Some(&levels));
        // 色以外のチャンネルが有効なレイヤーは、色相・彩度へ替えられない（断って、理由が出る）
        s.message.clear();
        let hue = AdjustmentSettings::hue_saturation(30.0, 0.0, 0.0).unwrap();
        s.apply(Action::M2(Edit::Adjust {
            id: adj,
            settings: hue.clone(),
        }));
        assert_eq!(s.doc.layer(adj).unwrap().adjustment(), Some(&levels));
        assert!(!s.message.is_empty());
        // 色相・彩度は色のチャンネルだけに足したレイヤーなら使える
        s.apply(Action::M2(Edit::NewAdjustment(
            AdjustmentKind::HueSaturation,
        )));
        let hs = s.selected_layer.unwrap();
        assert!(s.doc.layer(hs).unwrap().is_channel_enabled(Channel::Color));
        assert!(!s
            .doc
            .layer(hs)
            .unwrap()
            .is_channel_enabled(Channel::Roughness));
        s.apply(Action::M2(Edit::Adjust {
            id: hs,
            settings: hue.clone(),
        }));
        assert_eq!(s.doc.layer(hs).unwrap().adjustment(), Some(&hue));
    }

    #[test]
    fn refused_edits_change_nothing_and_say_why() {
        let mut s = app();
        let id = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::AddMask(id)));
        let revision = s.doc.revision();
        s.modified = false;
        s.apply(Action::M2(Edit::AddMask(id))); // 2 つ目は断る
        assert_eq!(s.doc.revision(), revision);
        assert!(!s.modified);
        assert!(!s.message.is_empty());
        // 描いている間の編集は断る
        s.message.clear();
        let brush = s.stroke_brush(false);
        let stroke = s.doc.begin_brush_stroke(id, &brush).unwrap();
        s.stroke = Some(stroke);
        s.apply(Action::M2(Edit::NewGroup));
        assert_eq!(s.doc.layers().len(), 1);
        assert_eq!(s.message, "描いている間はできません。");
    }

    #[test]
    fn duplicate_ungroup_and_group_selected() {
        let mut s = app();
        let id = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::GroupSelected));
        let group = s.selected_layer.unwrap();
        assert_eq!(s.doc.layer(id).unwrap().parent(), Some(group));
        s.apply(Action::M2(Edit::Duplicate(group)));
        assert_eq!(s.doc.layers().len(), 4, "グループは中身ごと写す");
        s.apply(Action::Undo);
        assert_eq!(s.doc.layers().len(), 2);
        s.apply(Action::M2(Edit::Ungroup(group)));
        assert_eq!(s.doc.layers().len(), 1);
        assert_eq!(s.doc.layer(id).unwrap().parent(), None);
        assert_eq!(s.selected_layer, Some(id));
    }

    #[test]
    fn ungrouping_a_group_with_rulers_deletes_them_with_it_says_so_and_undo_brings_them_back() {
        let mut s = app();
        s.apply(Action::M2(Edit::GroupSelected));
        let group = s.selected_layer.unwrap();
        let ruler = yolu_core::Ruler::canvas(
            s.doc.new_ruler_id(),
            yolu_core::RulerKind::Line,
            yolu_core::glam::DVec2::new(0.0, 5.0),
            yolu_core::glam::DVec2::new(10.0, 5.0),
        );
        s.doc.set_rulers(group, vec![ruler], false).unwrap();
        s.message.clear();
        s.apply(Action::M2(Edit::Ungroup(group)));
        assert!(s.doc.layer(group).is_none());
        assert!(
            s.message.contains("定規 1 個"),
            "黙って捨てない: {}",
            s.message
        );
        s.apply(Action::Undo);
        assert_eq!(s.doc.layer(group).unwrap().rulers().len(), 1);
        // 定規の無いグループでは、定規の知らせを出さない
        let plain = s.doc.add_group("plain", None).unwrap();
        s.message.clear();
        s.apply(Action::M2(Edit::Ungroup(plain)));
        assert!(!s.message.contains("定規"), "{}", s.message);
    }

    #[test]
    fn rows_hide_collapsed_children_and_drops_resolve() {
        let mut s = app();
        let first = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::GroupSelected));
        let group = s.selected_layer.unwrap();
        s.apply(Action::NewLayer);
        let top = s.selected_layer.unwrap();
        let rows = visible_rows(&s.doc, &s.m2.collapsed);
        assert_eq!(
            rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            [top, group, first]
        );
        assert_eq!(rows[2].depth, 1);
        s.m2_ui(UiOp::ToggleCollapsed(group));
        let rows = visible_rows(&s.doc, &s.m2.collapsed);
        assert_eq!(rows.len(), 2, "閉じたグループの中身は出ない");
        s.m2_ui(UiOp::ToggleCollapsed(group));
        let rows = visible_rows(&s.doc, &s.m2.collapsed);
        // 一番上のレイヤーを、グループの行の中ほどへ → グループの一番上
        let edit = drop_edit(&s.doc, &rows, top, DropTarget::Into(group)).unwrap();
        assert_eq!(
            edit,
            Edit::Move {
                id: top,
                parent: Some(group),
                position: 1
            }
        );
        // 子の行の上へ → グループの中の、その子のすぐ上
        let edit = drop_edit(&s.doc, &rows, top, DropTarget::Gap(2)).unwrap();
        assert_eq!(
            edit,
            Edit::Move {
                id: top,
                parent: Some(group),
                position: 1
            }
        );
        // グループを自分の中へは動かせない
        assert_eq!(
            drop_edit(&s.doc, &rows, group, DropTarget::Into(group)),
            None
        );
        assert_eq!(drop_edit(&s.doc, &rows, group, DropTarget::Gap(2)), None);
        // 一番下の下へ → 一番上の段の一番下
        assert_eq!(
            drop_edit(&s.doc, &rows, top, DropTarget::Gap(3)),
            Some(Edit::Move {
                id: top,
                parent: None,
                position: 0
            })
        );
        s.apply(Action::M2(edit));
        assert_eq!(names(&s).len(), 3);
    }

    #[test]
    fn collapsing_moves_the_selection_to_the_group() {
        let mut s = app();
        let first = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::GroupSelected));
        let group = s.selected_layer.unwrap();
        s.selected_layer = Some(first);
        s.m2_ui(UiOp::ToggleCollapsed(group));
        assert_eq!(s.selected_layer, Some(group));
    }

    #[test]
    fn channels_add_rename_remove_and_selection_follows() {
        let mut s = app();
        let info = new_channel_info("AO".into(), ChannelKind::Scalar);
        s.apply(Action::M2(Edit::AddChannel(info.clone())));
        let ao = s.m2.paint_channel;
        assert!(!ao.is_standard());
        assert_eq!(s.m2.display_channel, ao);
        assert_eq!(channel_name(Lang::Ja, &s.doc, ao), "AO");
        s.apply(Action::M2(Edit::SetChannel {
            channel: ao,
            info: ChannelInfo {
                name: "Cavity".into(),
                ..info
            },
        }));
        assert_eq!(s.doc.channel_info(ao).unwrap().name, "Cavity");
        // 標準のチャンネルは変えられない（断って、理由が出る）
        s.message.clear();
        s.apply(Action::M2(Edit::RemoveChannel(Channel::Color)));
        assert!(!s.message.is_empty());
        s.apply(Action::M2(Edit::RemoveChannel(ao)));
        assert_eq!(
            s.m2.paint_channel,
            Channel::Color,
            "消えたチャンネルは選び直す"
        );
        s.apply(Action::Undo);
        assert!(s.doc.channel_info(ao).is_some());
        // 描くチャンネルを替えると、表示が同じだったときは表示も付いてくる
        s.m2_ui(UiOp::PaintChannel(Channel::Roughness));
        assert_eq!(s.m2.display_channel, Channel::Roughness);
        s.m2_ui(UiOp::DisplayChannel(Channel::Color));
        s.m2_ui(UiOp::PaintChannel(Channel::Metallic));
        assert_eq!(s.m2.display_channel, Channel::Color);
    }

    #[test]
    fn per_channel_blend_follows_the_layer_until_it_gets_its_own() {
        let mut s = app();
        let id = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::BlendMode {
            id,
            channel: None,
            mode: BlendMode::Multiply,
        }));
        s.apply(Action::M2(Edit::OwnBlend {
            id,
            channel: Channel::Roughness,
            own: true,
        }));
        let l = s.doc.layer(id).unwrap();
        assert_eq!(l.blend_mode_in(Channel::Roughness), BlendMode::Multiply);
        s.apply(Action::M2(Edit::BlendMode {
            id,
            channel: Some(Channel::Roughness),
            mode: BlendMode::Screen,
        }));
        s.apply(Action::M2(Edit::Opacity {
            id,
            channel: Some(Channel::Roughness),
            value: 0.5,
        }));
        s.m2_end_drag();
        let l = s.doc.layer(id).unwrap();
        assert_eq!(
            l.blend_mode(),
            BlendMode::Multiply,
            "レイヤーの値は変わらない"
        );
        assert_eq!(l.blend_mode_in(Channel::Roughness), BlendMode::Screen);
        assert_eq!(l.opacity_in(Channel::Roughness), 0.5);
        assert_eq!(l.opacity_in(Channel::Color), 1.0);
        s.apply(Action::M2(Edit::OwnBlend {
            id,
            channel: Channel::Roughness,
            own: false,
        }));
        assert!(s
            .doc
            .layer(id)
            .unwrap()
            .channel_blend(Channel::Roughness)
            .is_empty());
    }

    #[test]
    fn painting_goes_to_the_selected_channel_or_the_mask() {
        let mut s = app();
        let id = s.selected_layer.unwrap();
        s.m2_ui(UiOp::PaintChannel(Channel::Roughness));
        let stroke = s.begin_paint_stroke(id, false).unwrap();
        let mut stroke = stroke;
        stroke
            .add_point(&mut s.doc, 20.0, 20.0, 1.0, Default::default())
            .unwrap();
        s.doc.end_stroke(stroke).unwrap();
        let layer = s.doc.layer(id).unwrap();
        assert!(layer.surface(Channel::Roughness).unwrap().tile_count() > 0);
        assert_eq!(layer.surface(Channel::Color).unwrap().tile_count(), 0);
        // マスク
        s.apply(Action::M2(Edit::AddMask(id)));
        let mut stroke = s.begin_paint_stroke(id, false).unwrap();
        stroke
            .add_point(&mut s.doc, 30.0, 30.0, 1.0, Default::default())
            .unwrap();
        s.doc.end_stroke(stroke).unwrap();
        assert!(
            s.doc
                .layer(id)
                .unwrap()
                .mask()
                .unwrap()
                .surface()
                .tile_count()
                > 0
        );
    }

    #[test]
    fn blocked_painting_names_the_reason() {
        let mut s = app();
        s.apply(Action::M2(Edit::NewGroup));
        assert!(s.paint_blocker().unwrap().contains("グループ"));
        s.lang = Lang::En;
        assert!(s.paint_blocker().unwrap().contains("Group"));
        s.apply(Action::NewLayer);
        let id = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::ChannelEnabled {
            id,
            channel: Channel::Color,
            enabled: false,
        }));
        assert!(s.paint_blocker().is_some());
        assert!(s.begin_paint_stroke(id, false).is_err());
    }

    #[test]
    fn presets_carry_their_tip_and_keep_the_assist() {
        let mut s = app();
        s.m2.brush.assist.stabilizer = 12.0;
        let chalk = presets().iter().position(|p| p.id == "chalk").unwrap();
        s.m2_ui(UiOp::Preset(chalk));
        assert!(s.m2.brush.tip.image.is_some() && s.m2.brush.texture.is_some());
        assert_eq!(s.m2.brush.assist.stabilizer, 12.0, "補正は描き手の設定");
        assert_eq!(s.brush.radius, 18.0);
        let eraser = presets()
            .iter()
            .position(|p| p.id == "soft-eraser")
            .unwrap();
        s.m2_ui(UiOp::Preset(eraser));
        assert_eq!(s.tool, crate::state::Tool::Eraser);
        s.m2_ui(UiOp::Preset(0));
        assert_eq!(s.tool, crate::state::Tool::Brush);
        // どのプリセットも、そのままストロークを始められる（検証を通る）
        for i in 0..presets().len() {
            s.m2_ui(UiOp::Preset(i));
            let id = s.selected_layer.unwrap();
            let eraser = s.tool == crate::state::Tool::Eraser;
            let stroke = s.begin_paint_stroke(id, eraser).unwrap();
            s.doc.cancel_stroke(stroke);
        }
    }

    #[test]
    fn brush_options_round_trip_and_validate() {
        let mut s = app();
        s.m2_ui(UiOp::Brush(BrushOp::Tip(Some("noisy-disc"))));
        assert!(s.m2.brush.tip.image.is_some());
        s.m2_ui(UiOp::Brush(BrushOp::Tip(None)));
        assert!(s.m2.brush.tip.image.is_none());
        s.m2_ui(UiOp::Brush(BrushOp::Texture(Some("grain"))));
        s.m2.brush.texture.as_mut().unwrap().depth = 0.9;
        s.m2_ui(UiOp::Brush(BrushOp::Texture(None)));
        s.m2_ui(UiOp::Brush(BrushOp::Texture(Some("dots"))));
        assert_eq!(
            s.m2.brush.texture.as_ref().unwrap().depth,
            0.9,
            "同じ値から始める"
        );
        s.m2_ui(UiOp::Brush(BrushOp::DualEnabled(true)));
        s.m2_ui(UiOp::Brush(BrushOp::DualMode(DualBrushMode::Subtract)));
        s.m2_ui(UiOp::Brush(BrushOp::DualEnabled(false)));
        s.m2_ui(UiOp::Brush(BrushOp::DualEnabled(true)));
        assert_eq!(
            s.m2.brush.dual.as_ref().unwrap().mode,
            DualBrushMode::Subtract
        );
        for kind in EffectKind::ALL {
            s.m2_ui(UiOp::Brush(BrushOp::Effect(kind)));
            let id = s.selected_layer.unwrap();
            let stroke = s.begin_paint_stroke(id, false).unwrap();
            s.doc.cancel_stroke(stroke);
        }
        // 効果のブラシで消しゴムは使えない: ツールを戻し、消しゴムのストロークは効果を外す
        s.m2_ui(UiOp::Brush(BrushOp::Effect(EffectKind::Blur)));
        s.tool = crate::state::Tool::Eraser;
        let id = s.selected_layer.unwrap();
        let stroke = s.begin_paint_stroke(id, false).unwrap();
        s.doc.cancel_stroke(stroke);
        s.m2_ui(UiOp::Brush(BrushOp::Effect(EffectKind::Smudge)));
        assert_eq!(s.tool, crate::state::Tool::Brush);
    }

    #[test]
    fn seeds_change_per_stroke_unless_fixed() {
        let mut s = app();
        let a = s.stroke_brush(false).seed;
        let b = s.stroke_brush(false).seed;
        assert_ne!(a, b);
        s.m2.random_seed = false;
        assert_eq!(s.stroke_brush(false).seed, 0);
    }

    #[test]
    fn labels_exist_in_both_languages() {
        for lang in Lang::ALL {
            for p in presets() {
                let (name, category) = preset_label(lang, p);
                assert!(!name.is_empty() && !category.is_empty(), "{}", p.id);
            }
            for id in yolu_core::brush::BUILTIN_TIPS {
                assert!(!tip_label(lang, id).is_empty(), "{id}");
            }
            for m in BlendMode::LAYER_MODES {
                assert!(!blend_label(lang, m).is_empty());
            }
        }
        assert_eq!(
            blend_label(Lang::En, BlendMode::PassThrough),
            "Pass Through"
        );
        assert_eq!(blend_label(Lang::Ja, BlendMode::PassThrough), "通過");
        assert_eq!(blend_choices(true)[0], BlendMode::PassThrough);
        assert_eq!(blend_choices(false).len(), 26);
    }
}
