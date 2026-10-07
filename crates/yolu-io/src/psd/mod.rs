//! Unity 版と同じ境界を持つ PSD v1 RGB8 と、PSB の原本保持。
//! DTO の並びは上から下、画素も上の行から。原本の編集には必ず `write_edited` を使う。
mod bake;
mod binary;
mod bridge;
mod composite;
mod descriptor;
mod import;
mod read;
mod verified;
mod write;
use crate::{check, Result};
pub use bake::{
    check_exportable, export_blockers, export_core, plan_export, ExportControl, ExportMode,
    ExportNote, ExportOptions, ExportPlan, Exported, FillSources, GradientExpansion, NoteAction,
    RoundedParameter, RoundedValue,
};
pub use bridge::{Blocker, Refusal};
pub use import::{
    import_copy, verify_stream, CopyImport, CopyOptions, CopyOutcome, CopyRefusal, ImportAction,
    ImportDetail, ImportFeature, ImportNote, Unchecked, Verified, ADJUSTMENT_TAG_KEYS,
};
pub use read::{read, read_cancellable, read_stream};
pub use verified::{
    check_written, stage_verified, stage_with, write_verified, Commit, Staged, WriteError,
};
pub use write::{
    write, write_edited, write_with, Checksum, Compression, ExportError, Overrun, Recount, Written,
};

/// PSD を読み、層を重ねた 1 枚にする（幅・高さ・straight RGBA8、上の行から）。層を持たない（読めない・原本を残すだけの）PSD は断る。
/// 重ね方は取り込みの見本と同じ（`composite`）。取消の旗は読み込みと行ごとに見る。
pub fn read_flattened(
    bytes: &[u8],
    limits: &Limits,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<(u32, u32, Vec<u8>)> {
    let read = read_cancellable(bytes, limits, cancel)?;
    let Some(document) = read.document() else {
        let why = read
            .diagnostics()
            .iter()
            .find(|d| !d.is_informational())
            .map_or_else(|| "読めない PSD です".to_owned(), |d| d.message.clone());
        return Err(crate::Error::InvalidData(why));
    };
    let rgba = composite::composite_cancellable(document, cancel)?;
    Ok((document.width, document.height, rgba))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompatibilityMode {
    EditableRaster,
    PreserveOnly,
    Rejected,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub offset: usize,
    pub length: usize,
}
impl Diagnostic {
    pub fn is_informational(&self) -> bool {
        matches!(
            self.code.as_str(),
            "NotCarriedIntoExport" | "CompositeDiffers"
        )
    }
}
#[derive(Clone, Debug)]
pub struct ReadResult {
    mode: CompatibilityMode,
    document: Option<Document>,
    original: Option<Vec<u8>>,
    diagnostics: Vec<Diagnostic>,
}
impl ReadResult {
    pub fn mode(&self) -> CompatibilityMode {
        self.mode
    }
    pub fn document(&self) -> Option<&Document> {
        self.document.as_ref()
    }
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
    pub fn original_bytes(&self) -> Option<&[u8]> {
        self.original.as_deref()
    }
    pub fn copy_original_bytes(&self) -> Option<Vec<u8>> {
        self.original.clone()
    }
}
#[derive(Clone, Debug)]
pub struct Limits {
    pub max_source_bytes: usize,
    pub max_output_bytes: usize,
    pub max_dimension: u32,
    pub max_canvas_pixels: u64,
    pub max_layers: usize,
    pub max_decoded_bytes: u64,
    pub max_metadata_bytes: usize,
    pub max_name_code_units: usize,
    pub max_diagnostics: usize,
    pub max_group_depth: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_source_bytes: 128 * 1024 * 1024,
            max_output_bytes: 128 * 1024 * 1024,
            max_dimension: 8192,
            max_canvas_pixels: 4096 * 4096,
            max_layers: 256,
            max_decoded_bytes: 256 * 1024 * 1024,
            max_metadata_bytes: 4 * 1024 * 1024,
            max_name_code_units: 4096,
            max_diagnostics: 128,
            max_group_depth: 32,
        }
    }
}
impl Limits {
    /// 書き出しの上限。設定の「レイヤーのメモリ」の予算 `budget`（バイト）から決める（取り込みの `CopyOptions` と同じ考え方）:
    /// 層の記録の数は予算 1 MiB につき 1 件（256〜32767 件。グループの区切りも数える）、キャンバス・層 1 枚の画素は予算以内（辺は PSD の上限 30000）、
    /// ファイルは PSD の上限 2 GiB。全層の画素の合計には上限が無い（流して書くので、メモリには層 1 枚ぶんしか持たない）。メモリに全層を組む書き出し
    /// （Normal の焼き込み・平らの 1 枚）の合計だけは、書き出しの側が予算で止める。
    pub fn for_export(budget: u64) -> Self {
        Self {
            max_source_bytes: i32::MAX as usize,
            max_output_bytes: i32::MAX as usize,
            max_dimension: write::MAX_SIDE,
            max_canvas_pixels: (budget / 4).max(1),
            max_layers: ((budget / (1024 * 1024)) as usize).clamp(256, 32767),
            max_decoded_bytes: u64::MAX,
            max_metadata_bytes: usize::try_from(budget).unwrap_or(usize::MAX),
            max_name_code_units: 4096,
            max_diagnostics: 128,
            // 文書の入れ子の上限（core の編集・読み込みが守る）と同じ。これより深い文書は作れない
            max_group_depth: yolu_core::MAX_GROUP_DEPTH,
        }
    }
    fn validate(&self) -> Result<()> {
        check(
            self.max_source_bytes >= 26
                && self.max_source_bytes <= i32::MAX as usize
                && self.max_output_bytes >= 26
                && self.max_output_bytes <= i32::MAX as usize
                && (1..=30000).contains(&self.max_dimension)
                && self.max_canvas_pixels > 0
                && (1..=32767).contains(&self.max_layers)
                && self.max_decoded_bytes >= 4
                && self.max_name_code_units > 0
                && self.max_diagnostics > 0
                && self.max_group_depth <= 1000,
            "PSD の予算設定が不正です",
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    pub width: u32,
    pub height: u32,
    pub layers: Vec<Layer>,
    pub composite_rgba: Option<Vec<u8>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    pub id: i32,
    pub name: String,
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub opacity: u8,
    pub visible: bool,
    pub blend_mode: BlendMode,
    pub clipping: bool,
    pub mask: Option<Mask>,
    pub pixels_rgba: Vec<u8>,
    pub kind: LayerKind,
    /// PSD の lspf と同じビット（0:透明、1:画素、2:位置、31:全体）。
    pub locks: u32,
}
impl Default for Layer {
    fn default() -> Self {
        Self {
            id: 0,
            name: "Layer".into(),
            left: 0,
            top: 0,
            width: 0,
            height: 0,
            opacity: 255,
            visible: true,
            blend_mode: BlendMode::Normal,
            clipping: false,
            mask: None,
            pixels_rgba: Vec::new(),
            kind: LayerKind::Raster,
            locks: 0,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayerKind {
    Raster,
    Group {
        children: Vec<Layer>,
        divider_id: i32,
    },
    Adjustment(Adjustment),
    SolidColor([u8; 3]),
}
/// PSD の刻みの整数で持つ。刻みの間の値へ勝手に丸めない。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Adjustment {
    Invert,
    Levels {
        input_black: u16,
        input_white: u16,
        output_black: u16,
        output_white: u16,
        gamma: u16,
    },
    HueSaturation {
        hue: i16,
        saturation: i16,
        lightness: i16,
    },
    /// グラデーションマップ（`grdm`）。色の分岐点と不透明度の分岐点は位置の昇順で、それぞれ 2〜32 個。
    GradientMap {
        reverse: bool,
        colors: Vec<GradientColorStop>,
        opacities: Vec<GradientOpacityStop>,
    },
    /// トーンカーブ（`curv`）。合成（RGB 全体）と R・G・B の曲線で、点は [入力, 出力]（0〜255 の整数、入力は昇順、2〜19 点）。
    ToneCurve {
        composite: Vec<[u8; 2]>,
        red: Vec<[u8; 2]>,
        green: Vec<[u8; 2]>,
        blue: Vec<[u8; 2]>,
    },
    /// カラーバランス（`blnc`）。範囲ごとの [シアン/レッド, マゼンタ/グリーン, イエロー/ブルー]（−100〜100）と輝度を保つ。
    ColorBalance {
        shadows: [i16; 3],
        midtones: [i16; 3],
        highlights: [i16; 3],
        preserve_luminosity: bool,
    },
    /// 明るさ・コントラスト（`brit`。旧式の記録）。明るさ −150〜150・コントラスト −100〜100。
    BrightnessContrast {
        brightness: i16,
        contrast: i16,
    },
    /// 2 値化（`thrs`）。しきい値 1〜255。
    Threshold {
        level: u16,
    },
    /// ポスタリゼーション（`post`）。階調 2〜255。
    Posterize {
        levels: u16,
    },
}
/// グラデーションマップの色の分岐点（位置は 0〜4096、中点は % で 0〜100、色は 8 bit）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GradientColorStop {
    pub location: u16,
    pub midpoint: u8,
    pub rgb: [u8; 3],
}
/// グラデーションマップの不透明度の分岐点（位置は 0〜4096、中点は % で 0〜100、不透明度は 0〜255）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GradientOpacityStop {
    pub location: u16,
    pub midpoint: u8,
    pub opacity: u8,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mask {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub default_color: u8,
    pub enabled: bool,
    pub density: u8,
    pub pixels: Vec<u8>,
}
impl Mask {
    fn at(&self, x: i64, y: i64) -> u8 {
        let x = x - i64::from(self.left);
        let y = y - i64::from(self.top);
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            self.default_color
        } else {
            self.pixels[(y * i64::from(self.width) + x) as usize]
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    LinearDodge,
    LinearBurn,
    HardLight,
    SoftLight,
    VividLight,
    LinearLight,
    PinLight,
    HardMix,
    Difference,
    Exclusion,
    Subtract,
    Divide,
    Hue,
    Saturation,
    Color,
    Luminosity,
    DarkerColor,
    LighterColor,
    PassThrough,
}
impl BlendMode {
    pub const ALL: [Self; 27] = [
        Self::Normal,
        Self::Multiply,
        Self::Screen,
        Self::Overlay,
        Self::Darken,
        Self::Lighten,
        Self::ColorDodge,
        Self::ColorBurn,
        Self::LinearDodge,
        Self::LinearBurn,
        Self::HardLight,
        Self::SoftLight,
        Self::VividLight,
        Self::LinearLight,
        Self::PinLight,
        Self::HardMix,
        Self::Difference,
        Self::Exclusion,
        Self::Subtract,
        Self::Divide,
        Self::Hue,
        Self::Saturation,
        Self::Color,
        Self::Luminosity,
        Self::DarkerColor,
        Self::LighterColor,
        Self::PassThrough,
    ];
    pub fn key(self) -> [u8; 4] {
        [
            *b"norm", *b"mul ", *b"scrn", *b"over", *b"dark", *b"lite", *b"div ", *b"idiv",
            *b"lddg", *b"lbrn", *b"hLit", *b"sLit", *b"vLit", *b"lLit", *b"pLit", *b"hMix",
            *b"diff", *b"smud", *b"fsub", *b"fdiv", *b"hue ", *b"sat ", *b"colr", *b"lum ",
            *b"dkCl", *b"lgCl", *b"pass",
        ][self as usize]
    }
    pub fn from_key(key: [u8; 4]) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.key() == key)
    }
}
