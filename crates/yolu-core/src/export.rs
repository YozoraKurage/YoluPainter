//! 書き出しのテンプレート（Substance Painter の Export の Output Template）: どの画像に、どのチャンネルをどう詰めるか。
//!
//! 画面にも保存先にも依らない計算だけを持つ。Unity 版の `ExportTemplates`（`Runtime/Core/ExportTemplates.cs`）と同じ意味・同じバイトで、
//! 画像（straight RGBA8、行は下から上、文書の大きさ）を [`build`] で作り、書くかどうかを [`should_write`] で決める。ファイルへ書くのは
//! `yolu-io` の `export`、UV の外への塗り広げは [`crate::padding`]。
//!
//! 詰める値は「R × A」（塗っていないテクセルは 0）で、lilToon の割り当てや 3D ビューのマテリアル表示と同じ。使っていないチャンネルの値は
//! [`default_value`]（Smoothness は Unity の Standard の既定と同じ 0.5、AO と 1 は 255、ほかは 0）。AO は焼いたもの（メッシュマップ）を
//! 1 テクセル 1 バイトで受け取る。無ければ 1（遮蔽なし）。

use std::fmt;

use rayon::prelude::*;

use crate::{Channel, CoreError, Document};

/// 書き出す画像 1 枚の中身の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExportImageKind {
    /// Color の合成そのまま（straight RGBA。アルファは透明度）。
    BaseColor,
    /// Emission の合成を、アルファを掛けた RGB と不透明に（シェーダーは RGB だけを読むので、柔らかい縁が明るく残らないように）。
    Emission,
    /// Unity 向けの法線（[`Document::normal_output`]: OpenGL の向き、不透明、塗っていない所は平ら、Height → Normal 込み）。
    Normal,
    /// R・G・B・A をそれぞれ [`ExportScalar`] から詰める（Metallic と Smoothness、MaskMap など）。
    Packed,
}

/// 詰める 1 つの値。値のチャンネルは「R × A」（塗っていないテクセルは 0）。並びは Unity 版の `ExportScalar` と同じ。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExportScalar {
    Zero,
    One,
    Metallic,
    Roughness,
    /// 1 − Roughness。
    Smoothness,
    Height,
    /// 焼いた AO（メッシュマップ）。無いテクセルと、AO を渡されないときは 1（遮蔽なし）。
    Occlusion,
}

impl ExportScalar {
    /// 値を読むチャンネル（0・1・AO は読まない）。Roughness と Smoothness は同じ Roughness から。
    pub fn source_channel(self) -> Option<Channel> {
        match self {
            ExportScalar::Metallic => Some(Channel::Metallic),
            ExportScalar::Roughness | ExportScalar::Smoothness => Some(Channel::Roughness),
            ExportScalar::Height => Some(Channel::Height),
            ExportScalar::Zero | ExportScalar::One | ExportScalar::Occlusion => None,
        }
    }
}

/// 使っていない値のチャンネルの既定（Smoothness は Unity の Standard の既定と同じ 0.5、1 と AO は 255、ほかは 0）。
pub fn default_value(scalar: ExportScalar) -> u8 {
    match scalar {
        ExportScalar::Smoothness => 128,
        ExportScalar::One | ExportScalar::Occlusion => 255,
        _ => 0,
    }
}

/// テンプレートの画像 1 枚。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportImage {
    suffix: String,
    kind: ExportImageKind,
    planes: [ExportScalar; 4],
}

impl ExportImage {
    /// 詰めない画像（BaseColor・Emission・Normal）。
    pub fn of(suffix: impl Into<String>, kind: ExportImageKind) -> ExportImage {
        ExportImage {
            suffix: suffix.into(),
            kind,
            planes: [
                ExportScalar::Zero,
                ExportScalar::Zero,
                ExportScalar::Zero,
                ExportScalar::One,
            ],
        }
    }

    /// R・G・B・A に値を詰める画像。
    pub fn pack(
        suffix: impl Into<String>,
        r: ExportScalar,
        g: ExportScalar,
        b: ExportScalar,
        a: ExportScalar,
    ) -> ExportImage {
        ExportImage {
            suffix: suffix.into(),
            kind: ExportImageKind::Packed,
            planes: [r, g, b, a],
        }
    }

    /// ファイル名の末尾（`<名前>_<接尾辞>.png`）。
    pub fn suffix(&self) -> &str {
        &self.suffix
    }

    pub fn kind(&self) -> ExportImageKind {
        self.kind
    }

    /// R・G・B・A に詰める値（詰める画像だけ。ほかは空）。
    pub fn scalars(&self) -> &[ExportScalar] {
        if self.kind == ExportImageKind::Packed {
            &self.planes
        } else {
            &[]
        }
    }

    /// Unity の取り込み: sRGB の色か（そうでなければリニア）。
    pub fn srgb(&self) -> bool {
        matches!(
            self.kind,
            ExportImageKind::BaseColor | ExportImageKind::Emission
        )
    }

    /// Unity の取り込み: ノーマルマップとして取り込むか。
    pub fn normal_map(&self) -> bool {
        self.kind == ExportImageKind::Normal
    }

    /// Unity の取り込み: アルファを透明度として扱うか（BaseColor だけ）。
    pub fn alpha_is_transparency(&self) -> bool {
        self.kind == ExportImageKind::BaseColor
    }
}

/// 書き出しのテンプレート: ID・名前・画像の並び。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportTemplate {
    pub id: String,
    pub name: String,
    pub images: Vec<ExportImage>,
}

impl ExportTemplate {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        images: Vec<ExportImage>,
    ) -> ExportTemplate {
        ExportTemplate {
            id: id.into(),
            name: name.into(),
            images,
        }
    }

    /// Unity の Built-in Standard（Metallic）と URP Lit: どちらも `_MetallicGlossMap` の R が Metallic、A が Smoothness、
    /// `_OcclusionMap` と `_ParallaxMap` は G を読む（灰色で全部に入れる）。
    pub fn unity_standard() -> ExportTemplate {
        use ExportScalar::*;
        ExportTemplate::new(
            "unity-standard",
            "Unity Standard / URP Lit",
            vec![
                ExportImage::of("Albedo", ExportImageKind::BaseColor),
                ExportImage::pack(
                    "MetallicSmoothness",
                    Metallic,
                    Metallic,
                    Metallic,
                    Smoothness,
                ),
                ExportImage::of("Normal", ExportImageKind::Normal),
                ExportImage::pack("Height", Height, Height, Height, One),
                ExportImage::pack("Occlusion", Occlusion, Occlusion, Occlusion, One),
                ExportImage::of("Emission", ExportImageKind::Emission),
            ],
        )
    }

    /// HDRP Lit: `_MaskMap` は R Metallic・G AO・B ディテールマスク（全部 1）・A Smoothness。`_HeightMap` は R を読む。
    pub fn unity_hdrp() -> ExportTemplate {
        use ExportScalar::*;
        ExportTemplate::new(
            "unity-hdrp",
            "HDRP Lit",
            vec![
                ExportImage::of("BaseColor", ExportImageKind::BaseColor),
                ExportImage::pack("MaskMap", Metallic, Occlusion, One, Smoothness),
                ExportImage::of("Normal", ExportImageKind::Normal),
                ExportImage::pack("Height", Height, Height, Height, One),
                ExportImage::of("Emission", ExportImageKind::Emission),
            ],
        )
    }

    /// lilToon: 割り当て（Unity 版の `LilToonAssignment`）と同じ値の画像（平滑度は 1 − Roughness、Metallic は灰色）。
    pub fn lil_toon() -> ExportTemplate {
        use ExportScalar::*;
        ExportTemplate::new(
            "liltoon",
            "lilToon",
            vec![
                ExportImage::of("Main", ExportImageKind::BaseColor),
                ExportImage::of("Normal", ExportImageKind::Normal),
                ExportImage::pack("Smoothness", Smoothness, Smoothness, Smoothness, One),
                ExportImage::pack("Metallic", Metallic, Metallic, Metallic, One),
                ExportImage::of("Emission", ExportImageKind::Emission),
            ],
        )
    }

    /// 組み込みの全部（Unity 版の `ExportTemplate.BuiltIn` と同じ並び）。
    pub fn built_in() -> Vec<ExportTemplate> {
        vec![
            ExportTemplate::unity_standard(),
            ExportTemplate::unity_hdrp(),
            ExportTemplate::lil_toon(),
        ]
    }

    /// 組み込みを ID から。
    pub fn built_in_by_id(id: &str) -> Option<ExportTemplate> {
        ExportTemplate::built_in().into_iter().find(|t| t.id == id)
    }

    /// 接尾辞で画像を探す。
    pub fn image(&self, suffix: &str) -> Option<&ExportImage> {
        self.images.iter().find(|i| i.suffix == suffix)
    }
}

/// 書き出しの計算（[`build`]・[`crate::padding`]）が断る理由。断った計算は何も変えない。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportError {
    /// 引数が範囲外・大きさが合わない など。中身は何の値か。
    InvalidArgument(&'static str),
    /// 作業のメモリ（確保の前に見積もった量）が予算を超える。
    WorkingBudgetExceeded { needed: u64, allowed: u64 },
    /// 呼び手が取り消した。
    Cancelled,
    /// 文書の合成などの失敗。
    Core(CoreError),
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExportError::InvalidArgument(what) => write!(f, "値が範囲外: {what}"),
            ExportError::WorkingBudgetExceeded { needed, allowed } => write!(
                f,
                "作業のメモリが予算を超える（必要 {needed} バイト、上限 {allowed} バイト）"
            ),
            ExportError::Cancelled => write!(f, "取り消した"),
            ExportError::Core(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ExportError::Core(e) => Some(e),
            _ => None,
        }
    }
}

impl From<CoreError> for ExportError {
    fn from(e: CoreError) -> Self {
        ExportError::Core(e)
    }
}

/// どれかのレイヤーが使っているチャンネル（Height → Normal が有効で Height を使っていれば Normal も）。
pub fn uses(document: &Document, channel: Channel) -> bool {
    document
        .layers()
        .iter()
        .any(|l| l.is_channel_enabled(channel))
        || (channel == Channel::Normal && document.derives_normal())
}

/// 画像を書き出すか: その画像が読むものが 1 つでもあるとき（チャンネルを使っている・AO がある）。0 と 1 だけの画像や、使っていない
/// チャンネルだけの画像は書かない（既定の値で埋めた画像を置かない）。
pub fn should_write(document: &Document, image: &ExportImage, has_occlusion: bool) -> bool {
    match image.kind {
        ExportImageKind::BaseColor => uses(document, Channel::Color),
        ExportImageKind::Emission => uses(document, Channel::Emission),
        ExportImageKind::Normal => uses(document, Channel::Normal),
        ExportImageKind::Packed => image.scalars().iter().any(|&s| {
            if s == ExportScalar::Occlusion {
                has_occlusion
            } else {
                s.source_channel().is_some_and(|c| uses(document, c))
            }
        }),
    }
}

/// メッシュマップの AO の値（0〜1）を書き出しの 1 バイト（0〜255）へ。四捨五入は偶数丸め（Unity 版の `Mathf.RoundToInt` と同じ）で、
/// 範囲の外と NaN は端へ（NaN は 255 = 遮蔽なし）。
pub fn occlusion_byte(value: f32) -> u8 {
    if value.is_nan() {
        return 255;
    }
    (value * 255.0).round_ties_even().clamp(0.0, 255.0) as u8
}

/// [`build`] が確保する作業のバイト数（出力と、同時に持つ合成の画像。法線は [`Document::normal_working_bytes`]）。
pub fn working_bytes(document: &Document, image: &ExportImage) -> u64 {
    let n = document.width() as u64 * document.height() as u64;
    match image.kind {
        ExportImageKind::BaseColor | ExportImageKind::Emission => 4 * n,
        ExportImageKind::Normal => document.normal_working_bytes(),
        ExportImageKind::Packed => {
            let composes = image
                .scalars()
                .iter()
                .filter_map(|s| s.source_channel())
                .any(|c| uses(document, c));
            if composes {
                8 * n
            } else {
                4 * n
            }
        }
    }
}

/// 画像（straight RGBA8、行は下から上、文書の大きさ）を作る。`occlusion` は焼いた AO（テクセルごとに 0〜255、無いテクセルは 255）か
/// None。使っていないチャンネルの値は [`default_value`]。作業のバイト数（[`working_bytes`]）が `max_working_bytes` を超えるなら、
/// 確保の前に断る。画像を書くかどうかは [`should_write`]（作るだけなら、読むものが無い画像も既定の値で作れる）。
pub fn build(
    document: &Document,
    image: &ExportImage,
    occlusion: Option<&[u8]>,
    max_working_bytes: u64,
) -> Result<Vec<u8>, ExportError> {
    let n = document.width() as usize * document.height() as usize;
    if occlusion.is_some_and(|o| o.len() != n) {
        return Err(ExportError::InvalidArgument(
            "AO は 1 テクセルに 1 値（幅 × 高さ）",
        ));
    }
    let needed = working_bytes(document, image);
    if needed > max_working_bytes {
        return Err(ExportError::WorkingBudgetExceeded {
            needed,
            allowed: max_working_bytes,
        });
    }
    match image.kind {
        ExportImageKind::BaseColor => {
            Ok(document.composite_channel(Channel::Color, document.bounds())?)
        }
        ExportImageKind::Normal => Ok(document.normal_output(max_working_bytes)?),
        ExportImageKind::Emission => {
            let mut emission = document.composite_channel(Channel::Emission, document.bounds())?;
            emission
                .par_chunks_mut(4 * PIXELS_PER_TASK)
                .for_each(|chunk| {
                    for px in chunk.chunks_exact_mut(4) {
                        let a = px[3] as u32;
                        px[0] = ((px[0] as u32 * a + 127) / 255) as u8;
                        px[1] = ((px[1] as u32 * a + 127) / 255) as u8;
                        px[2] = ((px[2] as u32 * a + 127) / 255) as u8;
                        px[3] = 255;
                    }
                });
            Ok(emission)
        }
        ExportImageKind::Packed => build_packed(document, image, occlusion, n),
    }
}

/// [`channel_image`] が確保する作業のバイト数（出力。法線は [`Document::normal_working_bytes`]）。
pub fn channel_working_bytes(document: &Document, channel: Channel) -> u64 {
    if channel == Channel::Normal {
        document.normal_working_bytes()
    } else {
        4 * document.width() as u64 * document.height() as u64
    }
}

/// 1 つのチャンネルを、ほかのツールへ渡すファイルの画像（straight RGBA8、行は下から上、文書の大きさ。C# の `YlpContent.FileImage`）にする。
/// Normal は文書の設定のファイルの Y の向き（[`Document::normal_file_output`]。DirectX なら緑を反転）、ほかはチャンネルの合成そのまま。
/// テンプレートの画像（[`build`]）と違い、詰めたり色を掛けたりしない（Emission も合成のまま）ので、そのチャンネルだけを読み戻せる。
/// 作業のバイト数（[`channel_working_bytes`]）が `max_working_bytes` を超えるなら、確保の前に断る。
pub fn channel_image(
    document: &Document,
    channel: Channel,
    max_working_bytes: u64,
) -> Result<Vec<u8>, ExportError> {
    let needed = channel_working_bytes(document, channel);
    if needed > max_working_bytes {
        return Err(ExportError::WorkingBudgetExceeded {
            needed,
            allowed: max_working_bytes,
        });
    }
    if channel == Channel::Normal {
        Ok(document.normal_file_output(max_working_bytes)?)
    } else {
        Ok(document.composite_channel(channel, document.bounds())?)
    }
}

/// 並列の 1 仕事あたりの画素数（結果には効かない）。
const PIXELS_PER_TASK: usize = 1 << 14;

/// 詰める画像: 使うチャンネルの合成を 1 つずつ取り、それを読む面へ値を詰める（合成は同時に 1 つだけ持つ）。
fn build_packed(
    document: &Document,
    image: &ExportImage,
    occlusion: Option<&[u8]>,
    n: usize,
) -> Result<Vec<u8>, ExportError> {
    let mut output = vec![0u8; n * 4];
    let mut channels: Vec<Channel> = Vec::new();
    for (plane, &scalar) in image.planes.iter().enumerate() {
        if let (ExportScalar::Occlusion, Some(values)) = (scalar, occlusion) {
            fill_plane(&mut output, plane, |i| values[i]);
            continue;
        }
        match scalar.source_channel() {
            Some(c) if uses(document, c) => {
                if !channels.contains(&c) {
                    channels.push(c);
                }
            }
            _ => {
                let value = default_value(scalar);
                fill_plane(&mut output, plane, |_| value);
            }
        }
    }
    for channel in channels {
        let source = document.composite_channel(channel, document.bounds())?;
        let planes: Vec<(usize, bool)> = image
            .planes
            .iter()
            .enumerate()
            .filter(|(_, s)| s.source_channel() == Some(channel))
            .map(|(p, s)| (p, *s == ExportScalar::Smoothness))
            .collect();
        output
            .par_chunks_mut(4 * PIXELS_PER_TASK)
            .zip(source.par_chunks(4 * PIXELS_PER_TASK))
            .for_each(|(out, src)| {
                for (o, s) in out.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                    let v = ((s[0] as u32 * s[3] as u32 + 127) / 255) as u8;
                    for &(plane, invert) in &planes {
                        o[plane] = if invert { 255 - v } else { v };
                    }
                }
            });
    }
    Ok(output)
}

fn fill_plane(output: &mut [u8], plane: usize, value: impl Fn(usize) -> u8 + Sync) {
    output
        .par_chunks_mut(4 * PIXELS_PER_TASK)
        .enumerate()
        .for_each(|(chunk, out)| {
            for (k, o) in out.chunks_exact_mut(4).enumerate() {
                o[plane] = value(chunk * PIXELS_PER_TASK + k);
            }
        });
}
