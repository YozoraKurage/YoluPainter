//! 取り込んだブラシに付く「表せない」の一覧（`Unrepresented`）と、ブラシの出どころ（`Source`）。
//!
//! 元のアプリの設定のうち core のブラシで表せないもの、近似したもの、読めなかったものは、捨てずに 1 つずつ型で返す
//! （黙って丸い筆先にしない）。文は種類と値から画面が言語ごとに作る（`Display` は日本語、`english()` は英語）。

use std::fmt;

use super::error::{Fault, PatternRefusal};

/// ブラシの出どころ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    GimpGbr,
    GimpGih,
    /// 版は `1.0` か `1.5`。
    GimpVbr {
        version: String,
    },
    PhotoshopAbr {
        version: i16,
        kind: AbrKind,
    },
    PhotoshopPattern,
    PngTip,
    /// CLIP STUDIO PAINT のサブツール（`.sut`）。
    ClipStudioSut,
    /// 同梱の Krita 4 の既定の筆先。
    BundledKrita4,
}

/// ABR の中のどの記録からできたブラシか。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbrKind {
    /// 版 1・2 の計算で描くブラシ。
    Computed,
    /// 版 1・2 の画像の筆先。
    Sampled,
    /// 版 6 以降で、どのプリセットも使っていない筆先（既定の設定で取り込む）。
    Tip,
    /// 版 6 以降のプリセット（筆先と設定）。
    Preset,
}

impl Source {
    /// 出どころの表示名（固有名詞と版だけで、言語によらない）。
    pub fn label(&self) -> String {
        match self {
            Self::GimpGbr => "GIMP GBR".into(),
            Self::GimpGih => "GIMP GIH".into(),
            Self::GimpVbr { version } => format!("GIMP VBR {version}"),
            Self::PhotoshopAbr { version, kind } => match kind {
                AbrKind::Computed => format!("Photoshop ABR v{version} computed"),
                AbrKind::Sampled => format!("Photoshop ABR v{version} sampled"),
                AbrKind::Tip => format!("Photoshop ABR v{version} tip"),
                AbrKind::Preset => format!("Photoshop ABR v{version}"),
            },
            Self::PhotoshopPattern => "Photoshop pattern".into(),
            Self::PngTip => "PNG tip".into(),
            Self::ClipStudioSut => "CLIP STUDIO SUT".into(),
            Self::BundledKrita4 => "Krita 4".into(),
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label())
    }
}

/// ペンの操作（ABR の「コントロール」）の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlKind {
    Fade,
    PenPressure,
    PenTilt,
    StylusWheel,
    Rotation,
    InitialDirection,
    Direction,
    Unknown(i32),
}

impl ControlKind {
    pub(crate) fn from_code(code: i32) -> ControlKind {
        match code {
            1 => Self::Fade,
            2 => Self::PenPressure,
            3 => Self::PenTilt,
            4 => Self::StylusWheel,
            5 => Self::Rotation,
            6 => Self::InitialDirection,
            7 => Self::Direction,
            other => Self::Unknown(other),
        }
    }
    fn texts(self) -> (String, String) {
        match self {
            Self::Fade => ("フェード".into(), "fade".into()),
            Self::PenPressure => ("筆圧".into(), "pen pressure".into()),
            Self::PenTilt => ("ペンの傾き".into(), "pen tilt".into()),
            Self::StylusWheel => ("スタイラスホイール".into(), "stylus wheel".into()),
            Self::Rotation => ("回転".into(), "rotation".into()),
            Self::InitialDirection => ("初めの方向".into(), "initial direction".into()),
            Self::Direction => ("方向".into(), "direction".into()),
            Self::Unknown(n) => (format!("不明（{n}）"), format!("unknown ({n})")),
        }
    }
}

/// ゆらぎ・コントロールを持つ設定の名前。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Size,
    Angle,
    Roundness,
    Opacity,
    Flow,
    Scatter,
    DualScatter,
}

impl Setting {
    fn texts(self) -> (&'static str, &'static str) {
        match self {
            Self::Size => ("大きさ", "Size"),
            Self::Angle => ("角度", "Angle"),
            Self::Roundness => ("真円率", "Roundness"),
            Self::Opacity => ("不透明度", "Opacity"),
            Self::Flow => ("流量", "Flow"),
            Self::Scatter => ("散布", "Scatter"),
            Self::DualScatter => ("デュアルブラシの散布", "Dual brush scatter"),
        }
    }
}

/// GIMP のパラメトリックブラシの形。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VbrShape {
    Circle,
    Square,
    Diamond,
}

impl VbrShape {
    fn texts(self) -> (&'static str, &'static str) {
        match self {
            Self::Circle => ("円", "circle"),
            Self::Square => ("正方形", "square"),
            Self::Diamond => ("ひし形", "diamond"),
        }
    }
}

/// 模様（質感）の読み替えの注記。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternNote {
    /// 宣言の大きさとデータの大きさが違い、データの方を使った。
    SizeDiffers {
        stated: (i64, i64),
        data: (i64, i64),
    },
    /// 16 bit を 8 bit に落とした（上位バイト）。
    Reduced16Bit,
    /// 透明度・マスクのチャンネルを使わなかった。
    ExtraChannelsIgnored(usize),
    /// RGB かインデックスをグレーにした（Rec. 601 の重み）。
    GreyConversion { indexed: bool },
}

impl PatternNote {
    fn texts(&self) -> (String, String) {
        match self {
            Self::SizeDiffers { stated, data } => (
                format!("模様の宣言の大きさ（{}×{}）がデータ（{}×{}）と違うので、データの方を使った", stated.0, stated.1, data.0, data.1),
                format!(
                    "The pattern's stated size ({}x{}) differs from its data ({}x{}); the data is used.",
                    stated.0, stated.1, data.0, data.1
                ),
            ),
            Self::Reduced16Bit => ("16 bit の模様を 8 bit に落とした".into(), "The 16-bit pattern is reduced to 8 bits.".into()),
            Self::ExtraChannelsIgnored(n) => (
                format!("模様の透明度・マスクのチャンネル（{n}）は使っていない"),
                format!("The pattern's transparency / mask channels are ignored ({n})."),
            ),
            Self::GreyConversion { indexed } => (
                format!(
                    "{}の模様を Rec. 601 の重みでグレーにした（Photoshop の変換とは少し違うことがある）",
                    if *indexed { "インデックスカラー" } else { "RGB" }
                ),
                format!(
                    "The {} pattern was converted to grey with Rec. 601 weights; Photoshop's own conversion may differ slightly.",
                    if *indexed { "indexed" } else { "RGB" }
                ),
            ),
        }
    }
}

/// 質感の読み替えの注記（ABR の「テクスチャ」）。
#[derive(Clone, Debug, PartialEq)]
pub enum TextureNote {
    /// 模様がファイルに無い（Photoshop は自分の模様の一覧から取る）。質感は付けない。
    PatternMissing { name: String },
    /// 模様はあるが使えない。質感は付けない。
    PatternRefused {
        name: String,
        reason: PatternRefusal,
    },
    /// 拡大の割合が 5〜6400% の外で、範囲に収めた。
    ScaleClamped { percent: f64, used_percent: f64 },
    /// 知らない合わせ方（乗算にした）。
    Mode(String),
    /// 「描点ごとにテクスチャを適用」は無く、ストロークに 1 回だけ当てる。
    EachTip,
    /// 質感の深さのゆらぎ・コントロールは無い。
    DepthDynamics,
    /// 明るさの調整は無い。
    Brightness,
    /// コントラストの調整は無い。
    Contrast,
}

/// デュアルブラシの読み替えの注記。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DualNote {
    /// 2 つ目の筆先の記述が無く、デュアルブラシは使わない。
    MissingTip,
    /// 2 つ目の筆先がファイルに無く、デュアルブラシは使わない。
    TipNotInFile,
    /// 知らない筆先の種類（丸い筆先にした）。
    UnknownTipKind(String),
    /// 知らない合わせ方（乗算にした）。
    Mode(String),
    /// 1 軸だけの散布は無く、両方の軸へ散る。
    ScatterOneAxis,
    CountJitter,
    Flip,
}

/// CLIP STUDIO の「影響元設定」の対象（`.sut` の効果の列の名前から分かる）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SutTarget {
    Size,
    Opacity,
    Flow,
    /// 太さ（真円率）。
    Thickness,
    /// 紙質濃度（質感の濃さ）。
    TextureDensity,
    /// 絵の具量。
    MixPaint,
    /// 絵の具濃度。
    MixDensity,
    /// 色の変化の色相・彩度・明度・サブ描画色混合率（応えの最小値は、色相・彩度・明度だけ −1〜1）。
    ColorHue,
    ColorSaturation,
    ColorValue,
    ColorSubMix,
    /// 散布効果の粒子サイズ・粒子密度。
    SpraySize,
    SprayDensity,
}

impl SutTarget {
    fn texts(self) -> (&'static str, &'static str) {
        match self {
            Self::Size => ("大きさ", "size"),
            Self::Opacity => ("不透明度", "opacity"),
            Self::Flow => ("流量", "flow"),
            Self::Thickness => ("太さ", "thickness"),
            Self::TextureDensity => ("質感の濃さ", "texture density"),
            Self::MixPaint => ("絵の具の量", "paint amount"),
            Self::MixDensity => ("絵の具の濃さ", "paint density"),
            Self::ColorHue => ("色相の変化", "hue change"),
            Self::ColorSaturation => ("彩度の変化", "saturation change"),
            Self::ColorValue => ("明度の変化", "value change"),
            Self::ColorSubMix => ("サブ描画色混合率", "secondary color mix"),
            Self::SpraySize => ("粒子サイズ", "particle size"),
            Self::SprayDensity => ("粒子密度", "particle density"),
        }
    }

    /// 写す先の影響元。
    pub fn influence_mut(
        self,
        influences: &mut yolu_core::Influences,
    ) -> &mut yolu_core::Influence {
        use yolu_core::brush::{ColorItem, Item, SprayItem};
        let item = match self {
            Self::Size => Item::Size,
            Self::Opacity => Item::Opacity,
            Self::Flow => Item::Flow,
            Self::Thickness => Item::Roundness,
            Self::TextureDensity => Item::TextureDensity,
            Self::MixPaint => Item::MixPaint,
            Self::MixDensity => Item::MixDensity,
            Self::ColorHue => return influences.color_mut(ColorItem::Hue),
            Self::ColorSaturation => return influences.color_mut(ColorItem::Saturation),
            Self::ColorValue => return influences.color_mut(ColorItem::Value),
            Self::ColorSubMix => return influences.color_mut(ColorItem::SubMix),
            Self::SpraySize => return influences.spray_mut(SprayItem::Size),
            Self::SprayDensity => return influences.spray_mut(SprayItem::Density),
        };
        influences.get_mut(item)
    }

    /// 応えの最小値が −1〜1 か（色相・彩度・明度の変化）。ほかは 0〜1。
    pub fn signed(self) -> bool {
        matches!(
            self,
            Self::ColorHue | Self::ColorSaturation | Self::ColorValue
        )
    }

    /// 色の変化の項目か（速さ・ランダムの曲線の知らせは、曲線の付く BLOB のときだけ出す）。
    pub fn is_color(self) -> bool {
        matches!(
            self,
            Self::ColorHue | Self::ColorSaturation | Self::ColorValue | Self::ColorSubMix
        )
    }
}

/// 入り抜きの影響先の番号の名前（日本語・英語。画面の名前）。デュアル側（50000 を足した番号）は「サブ: 」を付ける。
fn taper_target_name(id: u32) -> (String, String) {
    let (dual, base) = if id >= 50000 {
        (true, id - 50000)
    } else {
        (false, id)
    };
    let (ja, en) = match base {
        112 => ("紙質濃度", "texture density"),
        1001 => ("ブラシサイズ", "brush size"),
        1021 => ("ブラシ濃度", "brush density"),
        1041 => ("間隔", "spacing"),
        1051 => ("厚さ", "thickness"),
        1082 => ("絵の具量", "paint amount"),
        1084 => ("絵の具濃度", "paint density"),
        1100 => ("サブ描画色混合率", "sub color mixing"),
        1104 => ("色相", "hue"),
        1106 => ("彩度", "saturation"),
        1108 => ("明度", "brightness"),
        1132 => ("粒子サイズ", "particle size"),
        1134 => ("粒子密度", "particle density"),
        _ => return (format!("番号 {id}"), format!("number {id}")),
    };
    if dual {
        (format!("デュアルブラシの{ja}"), format!("dual brush {en}"))
    } else {
        (ja.into(), en.into())
    }
}

fn join_ja(names: impl Iterator<Item = String>) -> String {
    names.collect::<Vec<_>>().join("・")
}

/// CLIP STUDIO の「影響元設定」の入力（注記で、どの入力の曲線かを言う）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SutInput {
    Pressure,
    Tilt,
    Speed,
    Random,
}

impl SutInput {
    fn texts(self) -> (&'static str, &'static str) {
        match self {
            Self::Pressure => ("筆圧", "pen pressure"),
            Self::Tilt => ("ペンの傾き", "pen tilt"),
            Self::Speed => ("速さ", "speed"),
            Self::Random => ("ランダム", "random"),
        }
    }
}

/// CLIP STUDIO の `.sut` の設定のうち、このアプリの設定へ写せたもの（近似を含む。近似した中身は [`SutNote`] に別に載る）。
/// 大きさ・不透明度・流量・硬さ・間隔・真円率は、どのブラシにも写すので載せない。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SutMapped {
    /// 筆先の画像（1 枚以上）。
    TipImage,
    /// 質感。
    Texture,
    /// 項目への筆圧の影響（最小値と曲線）。
    Pressure,
    /// 項目へのペンの傾きの影響（最小値と曲線）。
    Tilt,
    /// 入り抜き（大きさへ。長さは画素）。
    StartEnd,
    /// 手ぶれ補正。
    Stabilizer,
    /// 筆先の向きの角度。
    TipAngle,
    /// 筆先の角度のランダム。
    AngleRandom,
    /// 筆先の角度の影響元（ペンの向き・ペン軸の回転・線の方向）。
    AngleSource,
    /// 厚さの適用方向（水平）。
    ThicknessDirection,
    /// 最低 1 ピクセル。
    MinOnePixel,
    /// 色の混ぜ。
    ColorMixing,
    /// アンチエイリアス。
    AntiAliasing,
    /// 項目への速さの影響（最小値。曲線は直線）。
    Speed,
    /// 項目へのランダムの影響（最小値。曲線は直線）。
    Random,
    /// 複数の筆先の繰り返し方法。
    TipOrder,
    /// 筆先の左右・上下の反転（なし・反転・ランダム・折り返し時に反転）。
    TipFlip,
    /// ブラシ先端同士の比較暗の合成。
    TipDarken,
    /// 質感の回転。
    TextureRotation,
    /// 質感の明るさ・コントラスト（取り込みのとき画像へ焼き込む。元に戻せない）。
    TextureTone,
    /// 色の変化（先端の色相・彩度・明度・サブ描画色混合率・ストロークごとの変化・変化対象）。
    ColorChange,
    /// 散布効果（粒子サイズ・連動・粒子密度・散布偏向・粒子の向き）。
    Spray,
}

impl SutMapped {
    /// 項目の名前（日本語・英語。短い名詞句）。
    pub fn texts(self) -> (&'static str, &'static str) {
        match self {
            Self::TipImage => ("筆先の画像", "tip image"),
            Self::Texture => ("質感", "texture"),
            Self::Pressure => ("筆圧", "pen pressure"),
            Self::Tilt => ("傾き", "pen tilt"),
            Self::StartEnd => ("入り抜き", "start and end"),
            Self::Stabilizer => ("手ぶれ補正", "stabilization"),
            Self::TipAngle => ("筆先の角度", "tip angle"),
            Self::AngleRandom => ("角度のランダム", "angle jitter"),
            Self::AngleSource => ("角度の影響元", "angle source"),
            Self::ThicknessDirection => ("厚さの向き", "thickness direction"),
            Self::MinOnePixel => ("最低 1 ピクセル", "at least 1 pixel"),
            Self::ColorMixing => ("色の混ぜ", "color mixing"),
            Self::AntiAliasing => ("アンチエイリアス", "anti-aliasing"),
            Self::Speed => ("速さ", "speed"),
            Self::Random => ("ランダム", "random"),
            Self::TipOrder => ("筆先の繰り返し方法", "tip repeat method"),
            Self::TipFlip => ("筆先の反転", "tip flip"),
            Self::TipDarken => ("先端同士の比較暗", "darken between tips"),
            Self::TextureRotation => ("質感の回転", "texture rotation"),
            Self::TextureTone => (
                "質感の明るさ・コントラスト",
                "texture brightness and contrast",
            ),
            Self::ColorChange => ("色の変化", "color change"),
            Self::Spray => ("散布効果", "spraying effect"),
        }
    }
}

/// 色の変化の、近い動き・当たらない設定の知らせの種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorNote {
    /// 変化対象がサブ描画色だけ。このアプリはいつもメイン描画色で描くので、色相・彩度・明度の変化は当たらない。
    TargetSub,
    /// 影響元に傾き・速度を使う（動きを測っていないので、筆圧と同じ式で写した近い動き）。
    MotionInfluence,
    /// 先端とストロークごとの色相の変化を両方使う（足し算で当てる。足し算に上限があるかは確かめていない）。
    HueSum,
}

/// 散布効果の、近い動きの知らせの種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SprayNote {
    /// 粒子密度から描点ごとの粒子の数への換算（確かめていない。近い動き）。
    Density,
    /// 散布偏向の式（中心への寄り方を確かめていない。近い動き）。
    Bias,
    /// 粒子の向きの影響元のうち、ランダムでないビットが立っている（ビットの割り当てが分からないので、向きは足さない）。
    Direction,
}

/// CLIP STUDIO の `.sut` の設定のうち、読めなかった・表せない・近似したもの。
#[derive(Clone, Debug, PartialEq)]
pub enum SutNote {
    /// 筆先・質感の画像は素材に入っているプレビュー（PNG）から取った。元の大きさの画像は CLIP STUDIO 独自の入れ物（`.layer`）の
    /// 中にあり、読まない。解像度はプレビューまで。
    PreviewImage,
    /// 筆先・質感の画像が CLIP STUDIO 独自の入れ物（`.layer`）にだけ入っていて、読めなかった（使える PNG が無い）。
    ProprietaryImage,
    /// 筆先にできない素材があった（特定できない・画像を取り出せない・筆先の数の上限を超えた）ので、その筆先は使わない。1 枚も
    /// 使えなければ丸い筆先にした。
    TipMissing,
    /// 筆先の素材を名前で決められず、素材の並びで当てた（別の画像かもしれない）。
    TipGuessed,
    /// 複数の筆先を使う順序（繰り返し方法）を読めなかった（列が無い・知らない値）ので、ランダムにした。
    TipOrder,
    /// 繰り返し方法を写したが、折り返し・ランダムに一巡は近い動き（端の筆先の扱い・並べ替えの中身が、公式の説明で決まらない）。
    TipOrderApprox,
    /// 筆先の反転の値が知らない値だったので、反転しない。
    TipFlip,
    /// 質感の素材を特定できなかった（または画像を取り出せなかった）ので、質感なし。
    TextureMissing,
    /// 質感の素材を名前で決められず、素材の並びで当てた（別の画像かもしれない）。
    TextureGuessed,
    /// 質感の明るさ・コントラストを、一般的な式で画像へ焼き込んだ（CLIP STUDIO の式は公開の説明に無く、同じ結果になるか確かめていない）。
    TextureToneApprox,
    /// 質感の回転・明るさ・コントラストは、取り込みのとき写す（[`SutMapped::TextureRotation`]・[`SutMapped::TextureTone`]）ので、今の取り込みは
    /// この 3 つを出さない。前のバージョンが書いたブラシのファイルの印を読むために残す。
    TextureRotation,
    TextureBrightness,
    TextureContrast,
    /// 質感の合わせ方のうち、同じものが無い（通常・輪郭・高さ・知らない値）ので、乗算にした。
    TextureMode,
    /// 「描点ごとに質感」は無く、ストロークに 1 回だけ当てる。
    TextureEachTip,
    /// 筆先の向きの影響元のうち、写せなかったもの（強さを読めないランダム・知らない旗）。
    Direction,
    /// 厚さの適用方向（水平）は、筆先の画像では写せない（画像も回ってしまう）。
    ThicknessDirection,
    /// 下地混色の方式（にじみ・色重ね）は、エンジンの「混ぜる」と同じ振る舞いにならないので写さなかった。値は列の番号（1 にじみ・2 色重ね）。
    ColorMixingMethod(f64),
    /// 後補正（描いたあとに線を整える）。手ぶれ補正とは別の機能で、エンジンに無い。
    PostCorrection,
    /// ベクターレイヤー向けの設定（ベクター消去・ベクター吸着）。
    VectorTools,
    /// ブラシサイズの単位の列が 0（px）でない。値は px として読んだ。
    LengthUnit,
    /// 間隔の種類（`BrushAutoIntervalType`）が 0〜3 の整数でなく、写さなかった（既定の間隔のまま）。
    IntervalType(f64),
    /// 色の混ぜ（CLIP STUDIO の「色の混ぜ」: 絵の具量・絵の具濃度・色延び）。値は元の設定のまま（0〜100）。列は `BrushMixColor`
    /// （絵の具量: RGB の混ぜの割合）・`BrushMixAlpha`（絵の具濃度: 透明成分の混ぜの割合）・`BrushMixColorExtension`（色延び）。
    ColorMixing {
        paint: f64,
        density: f64,
        stretch: f64,
    },
    /// 散布効果を写せなかった（粒子サイズ・粒子密度の値が無い・粒子サイズの単位が画素でない）。
    Spray,
    DualBrush,
    /// 入り抜き（指定方法がパーセント・フェード、長さの単位が画素でない、影響先の列が読めない、読めない長さ）。写さなかった。
    StartEnd,
    /// 入り抜きの影響先のうち、エンジンに同じ項目が無く、写さなかったもの（影響先の番号。間隔・色相・彩度・明度・サブ描画色混合率・粒子・
    /// デュアル側・番号の分からないもの）。写せる項目があれば、そちらだけを写している。
    StartEndTargets(Vec<u32>),
    /// 手ぶれ補正のうち、写せなかったもの（速度による手ブレ補正）。
    Stabilizer,
    /// 色の変化の値があるが、切り替え（ブラシ先端色の変化・ストロークごとのランダム変化）が切で、使っていない。
    ColorChange,
    /// 色の変化の、近い動き・当たらない設定の知らせ。
    ColorApprox(ColorNote),
    /// 散布効果の、近い動きの知らせ。
    SprayApprox(SprayNote),
    /// 合成モード。
    BlendMode,
    /// 影響元設定の入力（速さ・ランダム）の曲線は読めず、直線にした。
    InfluenceCurve {
        target: SutTarget,
        input: SutInput,
    },
    /// 影響元設定の中身を読めなかった（この版の形が分からない）ので、外した。
    InfluenceUnreadable(SutTarget),
    /// 筆圧か傾きの曲線の点が 16 を超え、16 点に取り直した。
    CurveSimplified(SutTarget, SutInput),
    /// 傾きの曲線を決められず（影響元の頭の枠が読めない・曲線と合わない）、直立で 1・寝かせきって 0 の直線にした。
    TiltCurve(SutTarget),
    /// 入り抜きの長さだけを写した。速さに応じた長さ・割合（`BrushInOutBySpeed`・`BrushInRatio`・`BrushOutRatio`）は読まない。
    StartEndDetail,
    /// 手ぶれ補正の強さ（段階）を、糸の長さ（画素）へ 1 対 1 で写した（換算は推定）。
    StabilizerStrength,
    /// ブラシの設定の表（`Variant`）か、行の番号の列（`VariantID`）が無く、設定は既定のまま。
    SettingsMissing,
    /// 画像を取り出せなかった素材の数（ファイル全体）。
    MaterialsUnreadable(usize),
    /// ブラシが多く、上限を超えた分は読まなかった（ファイル全体。読まなかったブラシの数）。
    BrushesCapped(usize),
    /// 素材が多く、上限を超えた分は読まなかった（ファイル全体）。
    MaterialsCapped,
    /// アンチエイリアスの値（`AntiAlias`）が 0〜3 の整数でなく、写さなかった（なし のまま）。
    AntiAliasing(f64),
}

impl SutNote {
    /// 筆先と質感の素材・繰り返し方法・反転・明るさとコントラストの知らせの文。
    fn tip_and_texture_texts(&self) -> (String, String) {
        match self {
            Self::TipMissing => (
                "筆先にできない素材がある（特定できない・画像を取り出せない・筆先の数の上限を超えた）ので、その筆先は使わず、1 枚も無ければ丸い筆先にした".into(),
                "Some tip materials could not be used (not identified, image not extracted, or over the limit on tips); they are left out, and a round tip is used if none remain.".into(),
            ),
            Self::TipGuessed => (
                "筆先の画像は、素材の名前では決められず、素材の並びで当てた（別の画像かもしれない）".into(),
                "The tip image could not be identified by name and was matched by the order of the materials; it may be a different image.".into(),
            ),
            Self::TipOrderApprox => (
                "筆先の繰り返し方法（折り返し・ランダムに一巡）は近い動きで写した".into(),
                "The tip repeat method (Reverse, One random cycle) is carried over with a close behavior.".into(),
            ),
            Self::TextureToneApprox => (
                "質感の明るさ・コントラストは一般的な式で画像へ焼き込んだ（近い動き。元に戻せない）".into(),
                "Texture brightness and contrast are baked into the image with a common formula (a close behavior; it cannot be undone).".into(),
            ),
            Self::TipFlip => (
                "筆先の反転が知らない値なので、反転しない".into(),
                "The tip flip setting has an unknown value; the tip is not flipped.".into(),
            ),
            Self::TipOrder => (
                "複数の筆先の繰り返し方法を読めなかった（列が無い・知らない値）ので、ランダムにした".into(),
                "The repeat method for several tips could not be read (missing or unknown value); random order is used.".into(),
            ),
            Self::TextureMissing => (
                "質感の素材を特定できなかった（または画像を取り出せなかった）ので、質感なし".into(),
                "The texture material could not be identified (or its image extracted); the brush has no texture.".into(),
            ),
            Self::TextureGuessed => (
                "質感の画像は、素材の名前では決められず、素材の並びで当てた（別の画像かもしれない）".into(),
                "The texture image could not be identified by name and was matched by the order of the materials; it may be a different image.".into(),
            ),
            _ => unreachable!("筆先と質感の知らせだけがここへ来る"),
        }
    }

    /// 色の変化の知らせの文。
    fn color_texts(&self) -> (String, String) {
        match self {
            Self::ColorChange => (
                "色の変化の値があるが、ブラシ先端色の変化かストロークごとのランダム変化が切のため、使っていない".into(),
                "Color change values are set, but the tip color change or the per-stroke random change is off; they are not used.".into(),
            ),
            Self::ColorApprox(ColorNote::TargetSub) => (
                "色の変化の変化対象がサブ描画色だけ。このアプリはメイン描画色で描くので、色相・彩度・明度は変わらない".into(),
                "The color change target is the secondary color only. This app paints with the main color, so hue, saturation and value do not change.".into(),
            ),
            Self::ColorApprox(ColorNote::MotionInfluence) => (
                "色の変化の影響元の傾き・速度は、動きを測っておらず、筆圧と同じ式で写した（近い動き）".into(),
                "The tilt and speed influences of the color change were not measured and use the same formula as pressure (a close behavior).".into(),
            ),
            Self::ColorApprox(ColorNote::HueSum) => (
                "先端とストロークごとの色相の変化は足し算で当てた（足し算に上限があるかは確かめていない。近い動き）".into(),
                "The tip and per-stroke hue changes are added together (whether the sum has a limit was not verified; a close behavior).".into(),
            ),
            _ => unreachable!("色の変化の知らせだけがここへ来る"),
        }
    }

    /// 散布効果の知らせの文。
    fn spray_texts(&self) -> (String, String) {
        match self {
            Self::Spray => (
                "散布効果は、粒子サイズ・粒子密度の値が無いか、粒子密度が 1 未満か、粒子サイズの単位が画素でないため、写していない".into(),
                "The spraying effect is not carried over: the particle size or density is missing, the density is below 1, or the particle size unit is not pixels.".into(),
            ),
            Self::SprayApprox(SprayNote::Density) => (
                "散布効果の粒子密度は、描点ごとの粒子の数へ近い換算で写した（近い動き）".into(),
                "The particle density of the spraying effect is converted to a number of particles per dab with a close estimate (a close behavior).".into(),
            ),
            Self::SprayApprox(SprayNote::Bias) => (
                "散布効果の散布偏向は、中心への寄り方を近い式で写した（近い動き）".into(),
                "The spray deviation is carried over with a close formula for how particles gather to the center (a close behavior).".into(),
            ),
            Self::SprayApprox(SprayNote::Direction) => (
                "散布効果の粒子の向きの影響元は、知らない旗か複数の向きがあるため、向きは足していない".into(),
                "The particle direction source of the spraying effect has an unknown flag or several directions; no direction is added.".into(),
            ),
            _ => unreachable!("散布効果の知らせだけがここへ来る"),
        }
    }

    fn texts(&self) -> (String, String) {
        match self {
            Self::PreviewImage => (
                "筆先・質感の画像は素材のプレビューから取った（元の大きさの画像は独自の入れ物の中にあり、読まない。解像度はプレビューまで）".into(),
                "Tip and texture images come from the material's preview; the full-size image is in a proprietary container and is not read, so resolution is limited to the preview.".into(),
            ),
            Self::ProprietaryImage => (
                "筆先・質感の画像が CLIP STUDIO 独自の形式でしか入っていないため、読めない（設定だけを取り込んだ）".into(),
                "The tip and texture images are stored only in CLIP STUDIO's own format and cannot be read; only the settings were imported.".into(),
            ),
            Self::TipMissing
            | Self::TipGuessed
            | Self::TipOrderApprox
            | Self::TextureToneApprox
            | Self::TipFlip
            | Self::TipOrder
            | Self::TextureMissing
            | Self::TextureGuessed => self.tip_and_texture_texts(),
            Self::TextureRotation => ("質感の回転は未対応".into(), "Texture rotation is not supported.".into()),
            Self::TextureBrightness => ("質感の明るさは未対応".into(), "Texture brightness is not supported.".into()),
            Self::TextureContrast => ("質感のコントラストは未対応".into(), "Texture contrast is not supported.".into()),
            Self::TextureMode => (
                "質感の合わせ方（通常・輪郭・高さ）は未対応で、乗算にした".into(),
                "The texture blend mode (Normal, Outline, Height) is not supported; Multiply is used.".into(),
            ),
            Self::TextureEachTip => (
                "質感: 描点ごとの適用は未対応で、ストロークに 1 回だけ当てる".into(),
                "Texture per dab is not supported; the texture is applied once per stroke.".into(),
            ),
            Self::Direction => (
                "筆先の向きの影響元のうち、読めないもの（ランダムの強さ・知らない設定）は写していない".into(),
                "Some tip direction influences could not be read (random strength, unknown settings) and were not carried over.".into(),
            ),
            Self::ThicknessDirection => (
                "厚さの適用方向（水平）は筆先の画像では未対応で、垂直のまま".into(),
                "The horizontal thickness direction is not supported for an image tip; vertical is used.".into(),
            ),
            Self::ColorMixingMethod(kind) => {
                let (ja, en) = match *kind as i64 {
                    1 => ("にじみ", "smear"),
                    2 => ("色重ね", "layering"),
                    _ => ("知らない方式", "unknown method"),
                };
                (
                    format!("下地混色の方式（{ja}）は未対応で、色の混ぜは写していない"),
                    format!("The color mixing method ({en}) is not supported; color mixing is not carried over."),
                )
            }
            Self::PostCorrection => (
                "後補正（描いたあとに線を整える）は未対応".into(),
                "Post correction (smoothing the line after drawing) is not supported.".into(),
            ),
            Self::VectorTools => (
                "ベクターレイヤー向けの設定（ベクター消去・ベクター吸着）は未対応".into(),
                "Settings for vector layers (vector eraser, vector magnet) are not supported.".into(),
            ),
            Self::LengthUnit => (
                "ブラシサイズの単位が px でない設定は、px として読んだ".into(),
                "A brush size unit other than pixels was read as pixels.".into(),
            ),
            Self::IntervalType(v) => (
                format!("間隔の種類の値 {v} は読めないので、間隔は既定のままにした"),
                format!("The interval type {v} is not understood; the default spacing is kept."),
            ),
            Self::ColorMixing { paint, density, stretch } => (
                format!("色の混ぜは未対応（絵の具量 {paint}・絵の具濃度 {density}・色延び {stretch}）"),
                format!("Color mixing is not supported (amount of paint {paint}, density of paint {density}, color stretch {stretch})."),
            ),
            Self::Spray | Self::SprayApprox(_) => self.spray_texts(),
            Self::DualBrush => ("デュアルブラシは未対応".into(), "Dual brush is not supported.".into()),
            Self::StartEnd => (
                "入り抜き（パーセント指定・フェード・画素以外の長さの単位・読めない影響先）は未対応".into(),
                "Start and end taper is not supported (percentage and fade modes, length units other than pixels, unreadable targets).".into(),
            ),
            Self::StartEndTargets(ids) => (
                format!("入り抜きの影響先のうち、{}は未対応で、写していない", join_ja(ids.iter().map(|&id| taper_target_name(id).0))),
                format!("These start and end taper targets are not supported and were not carried over: {}.", ids.iter().map(|&id| taper_target_name(id).1).collect::<Vec<_>>().join(", ")),
            ),
            Self::Stabilizer => (
                "手ぶれ補正の速度による調整は未対応".into(),
                "Stabilization adjustment by speed is not supported.".into(),
            ),
            Self::ColorChange | Self::ColorApprox(_) => self.color_texts(),
            Self::BlendMode => ("合成モードは未対応".into(), "The blend mode is not supported.".into()),
            Self::InfluenceCurve { target, input } => {
                let (tja, ten) = target.texts();
                let (ija, ien) = input.texts();
                (
                    format!("{tja}の影響元「{ija}」の曲線は読めず、直線にした（最小値は写した）"),
                    format!("The {ien} curve for {ten} could not be read; a straight line is used (the minimum is kept)."),
                )
            }
            Self::InfluenceUnreadable(target) => {
                let (tja, ten) = target.texts();
                (
                    format!("{tja}の影響元設定を読めなかった（この版の形が分からない）ので、外した"),
                    format!("The influence settings for {ten} could not be read (unknown layout in this version); they are left off."),
                )
            }
            Self::CurveSimplified(target, input) => {
                let (tja, ten) = target.texts();
                let (ija, ien) = input.texts();
                (
                    format!("{tja}の{ija}の曲線は 16 点に取り直した（点が 16 を超える）"),
                    format!("The {ien} curve for {ten} was resampled to 16 points (it had more)."),
                )
            }
            Self::TiltCurve(target) => {
                let (tja, ten) = target.texts();
                (
                    format!("{tja}の傾きの曲線を決められず、直立で 1・寝かせきって 0 の直線にした"),
                    format!("The pen tilt curve for {ten} could not be determined; a straight line from 1 (upright) to 0 (flat) is used."),
                )
            }
            Self::StartEndDetail => (
                "入り抜きは長さ・影響先・最小値を写した（速さに応じた長さ・割合は読まない）".into(),
                "Start and end taper keeps the lengths, targets and minimums (speed-dependent lengths and ratios are not read).".into(),
            ),
            Self::StabilizerStrength => (
                "手ぶれ補正の強さは、段階 1 を糸の長さ 1 画素として写した（換算は推定）".into(),
                "Stabilization strength maps one level to one pixel of string length (the conversion is an estimate).".into(),
            ),
            Self::SettingsMissing => (
                "ブラシの設定の表（Variant）か行の番号の列（VariantID）が無く、設定は既定のまま".into(),
                "The settings table (Variant) or its row-number column (VariantID) is missing; settings are left at their defaults.".into(),
            ),
            Self::MaterialsUnreadable(count) => (
                format!("画像を取り出せなかった素材が {count} 個ある"),
                format!("{count} materials had no image that could be extracted."),
            ),
            Self::BrushesCapped(count) => (
                format!("ブラシが多く、上限を超えた {count} 個は読まなかった"),
                format!("{count} brushes beyond the limit were not read."),
            ),
            Self::MaterialsCapped => (
                "素材が多く、上限を超えた分は読まなかった".into(),
                "There are more materials than the limit; the rest were not read.".into(),
            ),
            Self::AntiAliasing(v) => (
                format!("アンチエイリアスの値 {v} は読めないので、なしにした"),
                format!("The anti-aliasing value {v} is not understood; none is used."),
            ),
        }
    }
}

/// 元のファイルの設定のうち、core のブラシで表せない・近似した・読めなかったもの。
#[derive(Clone, Debug, PartialEq)]
pub enum Unrepresented {
    // GIMP
    /// 色つきの筆はアルファだけを筆先にした（描画色で塗る）。
    ColorTipAsMask,
    /// セルの選び方（筆圧・角度・速さ・傾き）は無く、ランダムにした。
    HoseSelection {
        mode: String,
    },
    /// 多次元のホースを 1 列のセルにした。
    HoseDimensions {
        dim: String,
    },
    /// 宣言したセル数よりファイルのセルが少なく、あるだけで使った。
    HoseShort {
        declared: u32,
        present: u32,
    },
    /// ブラシの後ろにデータが残っていた（古い .gpb の色つき模様は未対応）。
    GbrTrailingData {
        bytes: usize,
    },
    /// 円以外・スパイクの形を筆先の画像に描いた（縁が GIMP と少し違う）。
    VbrShapeRendered {
        shape: VbrShape,
        spikes: u32,
    },

    // Photoshop
    /// 16 bit の筆先を 8 bit に落とした。
    Tip16Bit,
    /// プリセットの節を読めず、筆先だけを既定の設定で取り込んだ。
    PresetsUnreadable(Fault),
    /// 模様の節を読めず、模様を使うブラシは質感なしで取り込んだ。
    PatternsUnreadable(Fault),
    /// 知らない節を読み飛ばした（4 文字のキー。制御文字は `?`）。ファイルごとに 1 種類 1 回。
    SectionSkipped(String),
    /// 知らない節の種類が多く、上に並べた分のほかにも読み飛ばした節の数（種類で数えず、節の数）。
    MoreSectionsSkipped(usize),
    /// 知らない筆先の種類（丸い筆先にした）。
    UnknownTipKind(String),
    /// 最小の直径は無い（％）。
    MinimumDiameter(f64),
    /// この設定のこのコントロールは無く、外した。
    Control {
        setting: Setting,
        control: ControlKind,
    },
    /// 描画色/背景色のコントロールは無く、ランダムに混ぜるだけ。
    ForegroundBackgroundControl(ControlKind),
    /// フェードの長さが範囲外で、外した。
    FadeRange {
        setting: Setting,
        steps: f64,
    },
    /// 1 軸だけの散布は無く、両方の軸へ散る。
    ScatterOneAxis,
    /// 数のゆらぎは無い。
    CountJitter,
    /// ノイズは無い。
    Noise,
    /// ウェットエッジは無い。
    WetEdges,
    /// 混合ブラシのウェット・混合のゆらぎ（`wetnessControl`・`mixControl`）は無く、外した。混合ブラシの基本の値（ウェット・負荷・混合）は
    /// ブラシの記述に入らない（ツールのオプション側）ので、写せない。キー名は実物の ABR では確かめていない（合成のデータの試験だけ）。
    MixerBrush,
    /// 質感の読み替え。
    Texture(TextureNote),
    /// 質感の模様の読み替え。
    TexturePattern(PatternNote),
    /// デュアルブラシの読み替え。
    Dual(DualNote),

    // .pat
    /// 模様を使えず読み飛ばした。
    PatternSkipped {
        name: String,
        reason: PatternRefusal,
    },
    /// 模様の読み替え。
    Pattern(PatternNote),

    // CLIP STUDIO .sut
    ClipStudio(SutNote),
}

impl Unrepresented {
    fn texts(&self) -> (String, String) {
        use Unrepresented::*;
        match self {
            ColorTipAsMask => (
                "色つきの筆先はアルファのマスクとして取り込んだ（このツールは選んだ色で塗るので、元の色は残らない）".into(),
                "A colour brush is imported as its alpha mask; its own colours are not kept (this engine paints with the chosen colour).".into(),
            ),
            HoseSelection { mode } => (
                format!("セルの選び方 '{mode}' は無く、ランダムにした"),
                format!("Cell selection '{mode}' (GIMP chooses cells by {mode}) is approximated by random selection."),
            ),
            HoseDimensions { dim } => (
                format!("{dim} 次元のホースを 1 列のセルにした"),
                format!("A {dim}-dimensional hose is flattened into one list of cells."),
            ),
            HoseShort { declared, present } => (
                format!("ホースは {declared} 個のセルを宣言しているがファイルには {present} 個しかなく、ある {present} 個を使う"),
                format!("The hose declares {declared} cells but the file holds only {present}; the {present} present are used."),
            ),
            GbrTrailingData { bytes } => (
                format!("ブラシの後ろのデータ（{bytes} バイト）は使っていない（GIMP の古い .gpb の色つき模様は未対応）"),
                format!("Data after the brush was ignored ({bytes} bytes; GIMP's old .gpb colour pattern is not supported)."),
            ),
            VbrShapeRendered { shape, spikes } => {
                let (ja, en) = shape.texts();
                (
                    format!(
                        "{ja}{}の形を筆先の画像に描いた（縁が GIMP と少し違うことがある）",
                        if *spikes > 2 { format!("（スパイク {spikes}）") } else { String::new() }
                    ),
                    format!(
                        "The {en}{} shape is rendered into a tip image; edges may differ slightly from GIMP.",
                        if *spikes > 2 { format!(" with {spikes} spikes") } else { String::new() }
                    ),
                )
            }
            Tip16Bit => ("16 bit の筆先を 8 bit に落とした".into(), "The 16-bit tip is reduced to 8 bits.".into()),
            PresetsUnreadable(fault) => (
                format!("ブラシの設定を読めなかった（{}）。筆先だけを既定の設定で取り込んだ", fault.texts().0),
                format!(
                    "Brush settings could not be read ({}); only the tips were imported with default settings.",
                    fault.texts().1.trim_end_matches('.')
                ),
            ),
            PatternsUnreadable(fault) => (
                format!("ブラシの質感の模様を読めなかった（{}）。模様を使うブラシは質感なしで取り込んだ", fault.texts().0),
                format!(
                    "Brush textures (patterns) could not be read ({}); brushes that use one are imported without it.",
                    fault.texts().1.trim_end_matches('.')
                ),
            ),
            SectionSkipped(key) => (
                format!("節 '{key}' は未対応で読み飛ばした"),
                format!("Section '{key}' is not supported and was skipped."),
            ),
            MoreSectionsSkipped(count) => (
                format!("ほかにも未対応の節 {count} 個を読み飛ばした"),
                format!("{count} more unsupported sections were skipped."),
            ),
            UnknownTipKind(kind) => (
                format!("知らない筆先の種類 '{kind}' は丸い筆先にした"),
                format!("Unknown tip kind '{kind}'; a round tip is used."),
            ),
            MinimumDiameter(p) => (format!("最小の直径（{p}%）は未対応"), format!("Minimum diameter ({p}%) is not supported.")),
            Control { setting, control } => {
                let (sja, sen) = setting.texts();
                let (cja, cen) = control.texts();
                (
                    format!("{sja}の{cja}のコントロールは未対応で、外した"),
                    format!("{sen} {cen} control is not supported; it is left off."),
                )
            }
            ForegroundBackgroundControl(control) => {
                let (cja, cen) = control.texts();
                (
                    format!("描画色/背景色の{cja}のコントロールは未対応で、ランダムに混ぜるだけにした"),
                    format!("Foreground/background {cen} control is not supported; the colour is mixed at random only."),
                )
            }
            FadeRange { setting, steps } => {
                let (sja, sen) = setting.texts();
                let max = yolu_core::brush::MAX_FADE;
                (
                    format!("{sja}の {steps} 描点のフェードは 1〜{max} の外で、外した"),
                    format!("{sen} fade over {steps} steps is outside 1..{max}; it is left off."),
                )
            }
            ScatterOneAxis => (
                "1 軸だけの散布は未対応で、両方の軸へ散る".into(),
                "Scatter along one axis only is not supported; dabs scatter along both axes.".into(),
            ),
            CountJitter => ("数のゆらぎは未対応".into(), "Count jitter is not supported.".into()),
            Noise => ("ノイズは未対応".into(), "Noise is not supported.".into()),
            WetEdges => ("ウェットエッジは未対応".into(), "Wet edges is not supported.".into()),
            MixerBrush => (
                "混合ブラシのウェット・混合のゆらぎは未対応".into(),
                "Mixer brush wetness and mix jitter are not supported.".into(),
            ),
            Texture(note) => match note {
                TextureNote::PatternMissing { name } => (
                    format!("質感: 模様 '{name}' がファイルに無い（Photoshop は自分の模様の一覧から取る）ので、質感なし"),
                    format!("Texture: the pattern '{name}' is not in the file (Photoshop takes it from its pattern library); the brush has no texture."),
                ),
                TextureNote::PatternRefused { name, reason } => (
                    format!("質感: 模様 '{name}' は使えない（{}）ので、質感なし", reason.texts().0),
                    format!("Texture: the pattern '{name}' could not be used ({}); the brush has no texture.", reason.texts().1),
                ),
                TextureNote::ScaleClamped { percent, used_percent } => (
                    format!("質感の拡大 {percent}% は 5〜6400% の外で、{used_percent}% にした"),
                    format!("Texture scale {percent}% is outside 5..6400%; {used_percent}% is used."),
                ),
                TextureNote::Mode(mode) => (
                    format!("質感の合わせ方 '{mode}' は未対応で、乗算にした"),
                    format!("Texture mode '{mode}' is not supported; Multiply is used."),
                ),
                TextureNote::EachTip => (
                    "質感: 描点ごとの適用は未対応で、ストロークに 1 回だけ当てる".into(),
                    "Texture each tip is not supported; the texture is applied once per stroke.".into(),
                ),
                TextureNote::DepthDynamics => (
                    "質感の深さのゆらぎとコントロールは未対応".into(),
                    "Texture depth jitter and control are not supported.".into(),
                ),
                TextureNote::Brightness => ("質感の明るさは未対応".into(), "Texture brightness is not supported.".into()),
                TextureNote::Contrast => ("質感のコントラストは未対応".into(), "Texture contrast is not supported.".into()),
            },
            TexturePattern(note) => {
                let (ja, en) = note.texts();
                (format!("質感: {ja}"), format!("Texture: {en}"))
            }
            Dual(note) => match note {
                DualNote::MissingTip => (
                    "デュアルブラシ: 2 つ目の筆先の記述が無いので使わない".into(),
                    "Dual brush: the second tip is missing; the dual brush is not used.".into(),
                ),
                DualNote::TipNotInFile => (
                    "デュアルブラシ: 2 つ目の筆先がファイルに無いので使わない".into(),
                    "Dual brush: its tip is not in the file; the dual brush is not used.".into(),
                ),
                DualNote::UnknownTipKind(kind) => (
                    format!("デュアルブラシ: 知らない筆先の種類 '{kind}' は丸い筆先にした"),
                    format!("Dual brush: unknown tip kind '{kind}'; a round tip is used."),
                ),
                DualNote::Mode(mode) => (
                    format!("デュアルブラシの合わせ方 '{mode}' は未対応で、乗算にした"),
                    format!("Dual brush mode '{mode}' is not supported; Multiply is used."),
                ),
                DualNote::ScatterOneAxis => (
                    "デュアルブラシの 1 軸だけの散布は未対応で、両方の軸へ散る".into(),
                    "Dual brush scatter along one axis only is not supported; dabs scatter along both axes.".into(),
                ),
                DualNote::CountJitter => ("デュアルブラシの数のゆらぎは未対応".into(), "Dual brush count jitter is not supported.".into()),
                DualNote::Flip => ("デュアルブラシの反転は未対応".into(), "Dual brush flipping is not supported.".into()),
            },
            PatternSkipped { name, reason } => (
                format!("模様 '{name}' は{}ので読み飛ばした", reason.texts().0),
                format!("Pattern '{name}' was skipped: {}.", reason.texts().1),
            ),
            Pattern(note) => note.texts(),
            ClipStudio(note) => note.texts(),
        }
    }

    /// 英語の文。
    pub fn english(&self) -> String {
        self.texts().1
    }
}

impl fmt::Display for Unrepresented {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.texts().0)
    }
}
