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

/// 元のファイルの設定のうち、core のブラシで表せない・近似した・読めなかったもの。
#[derive(Clone, Debug, PartialEq)]
pub enum Unrepresented {
    // GIMP
    /// 色つきの筆はアルファだけを筆先にした（描画色で塗る）。
    ColorTipAsMask,
    /// セルの選び方（筆圧・角度・速さ・傾き）は無く、ランダムにした。
    HoseSelection { mode: String },
    /// 多次元のホースを 1 列のセルにした。
    HoseDimensions { dim: String },
    /// 宣言したセル数よりファイルのセルが少なく、あるだけで使った。
    HoseShort { declared: u32, present: u32 },
    /// ブラシの後ろにデータが残っていた（古い .gpb の色つき模様は未対応）。
    GbrTrailingData { bytes: usize },
    /// 円以外・スパイクの形を筆先の画像に描いた（縁が GIMP と少し違う）。
    VbrShapeRendered { shape: VbrShape, spikes: u32 },

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
    FadeRange { setting: Setting, steps: f64 },
    /// 1 軸だけの散布は無く、両方の軸へ散る。
    ScatterOneAxis,
    /// 数のゆらぎは無い。
    CountJitter,
    /// ノイズは無い。
    Noise,
    /// ウェットエッジは無い。
    WetEdges,
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
