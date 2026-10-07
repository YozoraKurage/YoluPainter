//! テキストレイヤーの命令（`layer.add` の種類 text・`layer.set` の `text`）。テキストレイヤーの画素はフォントから描くので、文書を変える前に
//! フォントの中身を探しておき（[`font_for`]）、文書の命令へ渡す。
//!
//! - 同梱のフォントは、ホストが持つときだけ描ける（起動中のアプリ。画面なしのホストは持たず、理由を添えて断る）。
//! - OS に入っているフォントは名前（PostScript 名かファミリー名）で選べる（画面なしのホストでも）。
//! - レイヤーのフォントは、覚えた道 → PostScript 名 → ファミリー名と太さ → 道の順で探す（`yolu_io::fonts::find`）。中身の SHA-256 が
//!   違うフォントしか無ければ、見つけたフォントで描き直し、返事の `notes` に「フォントが違います」を残す（画面の「フォントが違います」と同じ。
//!   黙って別のフォントにしない）。どこにも無ければ断る。
//! - フォントのファイルは文書に入れない（道・束の番号・SHA-256・名前だけ）。

use std::path::Path;
use std::sync::Arc;

use yolu_core::text::{self, TextAlign, TextFont, TextSettings};
use yolu_core::{Document, Layer};
use yolu_io::fonts::{self, Lookup};

use crate::command::{Command, NewLayerKind, TextAlignName, TextSpec};
use crate::error::{ErrorCode, OpError};
use crate::host::OpHost;
use crate::refs::resolve_layer;
use crate::reply::{Reply, TextInfo};
use crate::value::{format_color, parse_color};

/// テキストレイヤーを描くフォント（文書の値と中身）。
#[derive(Clone)]
pub struct FontData {
    pub font: TextFont,
    pub bytes: Arc<[u8]>,
    /// レイヤーが覚えたフォントを探して、名前・道では見つかったが中身の SHA-256 が違った（描き直すと見つけたフォントの値になる）。
    pub different: bool,
}

/// 命令がテキストレイヤーを描くなら、そのフォントを探す（描かない命令は None）。文書は変えない。
pub fn font_for(host: &mut dyn OpHost, command: &Command) -> Result<Option<FontData>, OpError> {
    let spec = match command {
        Command::LayerAdd(a) if a.kind == NewLayerKind::Text => a.text.as_ref(),
        Command::LayerSet(a) => a.text.as_ref(),
        _ => None,
    };
    let Some(spec) = spec else {
        return Ok(None);
    };
    if spec.font.is_some() && spec.font_file.is_some() {
        return Err(OpError::invalid_request(
            "font と font_file は一緒に使えません",
            "`font` and `font_file` cannot be used together",
        ));
    }
    if spec.font_index.is_some() && spec.font_file.is_none() {
        return Err(OpError::invalid_request(
            "font_index は font_file と一緒に使います",
            "`font_index` goes with `font_file`",
        ));
    }
    if let Some(path) = &spec.font_file {
        let resolved = host.policy().resolve(path)?;
        let bytes = read_font_file(&resolved)?;
        let index = spec.font_index.unwrap_or(0);
        text::check_font(&bytes, index)?;
        let font = text::file_font(&resolved.to_string_lossy(), index, &bytes);
        return Ok(Some(FontData {
            font,
            bytes,
            different: false,
        }));
    }
    if let Some(name) = &spec.font {
        if text::BUNDLED_FONTS.contains(&name.as_str()) {
            return bundled(host, name).map(Some);
        }
        return installed(host, name).map(Some);
    }
    let Command::LayerSet(args) = command else {
        return bundled(host, text::DEFAULT_FONT).map(Some);
    };
    // フォントを変えない: レイヤーの今のフォント
    let mut current = None;
    host.read_set(args.set.as_deref(), &mut |view| {
        let doc = view.editable_doc()?;
        let layer = text_layer(doc, &args.layer)?;
        current = layer.text().map(|t| t.font.clone());
        Ok(Reply::Layer(crate::doc_ops::layer_info(doc, layer)))
    })?;
    let Some(font) = current else {
        return Err(not_text_layer());
    };
    if let TextFont::Bundled(name) = &font {
        return bundled(host, name).map(Some);
    }
    let found = match fonts::find_at_path(&font) {
        Some(same) => same,
        None => {
            let system = host.system_fonts()?;
            fonts::find(&font, &system)
        }
    };
    match found {
        Lookup::Same { font, bytes } => Ok(Some(FontData {
            font,
            bytes,
            different: false,
        })),
        Lookup::Different { font, bytes } => Ok(Some(FontData {
            font,
            bytes,
            different: true,
        })),
        Lookup::Missing => Err(OpError::new(
            ErrorCode::NotFound,
            format!("フォントが見つかりません（{}）", font_name(&font)),
            format!("Font not found ({})", font_name(&font)),
        )),
    }
}

/// エラーの文のフォントの名前（PostScript 名・ファミリー名・道の順で、あるもの）。
pub(crate) fn font_name(font: &TextFont) -> String {
    match font {
        TextFont::Bundled(name) => name.clone(),
        TextFont::File { path, names, .. } => [&names.postscript, &names.family, path]
            .into_iter()
            .find(|s| !s.is_empty())
            .cloned()
            .unwrap_or_default(),
    }
}

/// OS に入っているフォント（PostScript 名かファミリー名）。
fn installed(host: &mut dyn OpHost, name: &str) -> Result<FontData, OpError> {
    let system = host.system_fonts()?;
    let face = system.by_name(name).ok_or_else(|| {
        OpError::new(
            ErrorCode::NotFound,
            format!(
                "フォント「{name}」が見つかりません（同梱のフォントは {}、ほかは OS に入っているフォントの PostScript 名かファミリー名）",
                text::BUNDLED_FONTS.join("・")
            ),
            format!(
                "Font \"{name}\" not found (bundled fonts are {}; others are installed fonts by PostScript name or family name)",
                text::BUNDLED_FONTS.join(", ")
            ),
        )
    })?;
    let (font, bytes) = fonts::load_face(face).ok_or_else(|| {
        OpError::new(
            ErrorCode::Io,
            format!("フォントのファイルを読めません（{}）", face.path.display()),
            format!("Cannot read the font file ({})", face.path.display()),
        )
    })?;
    Ok(FontData {
        font,
        bytes,
        different: false,
    })
}

/// 同梱のフォント（ホストが持つときだけ）。
fn bundled(host: &mut dyn OpHost, name: &str) -> Result<FontData, OpError> {
    if !text::BUNDLED_FONTS.contains(&name) {
        return Err(OpError::new(
            ErrorCode::NotFound,
            format!(
                "同梱のフォント「{name}」はありません（{}）",
                text::BUNDLED_FONTS.join("・")
            ),
            format!(
                "No bundled font \"{name}\" ({})",
                text::BUNDLED_FONTS.join(", ")
            ),
        ));
    }
    let bytes = host.bundled_font(name).ok_or_else(|| {
        OpError::new(
            ErrorCode::Unsupported,
            format!("同梱のフォント「{name}」は起動中のアプリだけが描けます（ここでは font_file でフォントのファイルを指せます）"),
            format!("The bundled font \"{name}\" can be drawn only by the running app (here, point font_file at a font file)"),
        )
    })?;
    Ok(FontData {
        font: TextFont::Bundled(name.to_owned()),
        bytes,
        different: false,
    })
}

fn read_font_file(path: &Path) -> Result<Arc<[u8]>, OpError> {
    let io = |e: std::io::Error| {
        OpError::new(
            ErrorCode::Io,
            format!("フォントのファイルを読めません（{}）: {e}", path.display()),
            format!("Cannot read the font file ({}): {e}", path.display()),
        )
    };
    let len = std::fs::metadata(path).map_err(io)?.len();
    if len > fonts::MAX_FONT_FILE_BYTES {
        return Err(OpError::new(
            ErrorCode::Budget,
            format!("フォントのファイルが大きすぎます（{}）", path.display()),
            format!("The font file is too large ({})", path.display()),
        ));
    }
    Ok(Arc::from(std::fs::read(path).map_err(io)?))
}

fn not_text_layer() -> OpError {
    OpError::new(
        ErrorCode::Unsupported,
        "テキストレイヤーだけが text を持ちます",
        "Only text layers have `text`",
    )
}

fn text_layer<'d>(doc: &'d Document, name: &str) -> Result<&'d Layer, OpError> {
    let id = resolve_layer(doc, name)?;
    doc.layer(id)
        .filter(|l| l.text().is_some())
        .ok_or_else(not_text_layer)
}

/// 命令の値を当てた文字の値（`base` は層の今の値か、新しい層の既定）。範囲の検査は文書が描くときにする。
pub(crate) fn patched(
    base: &TextSettings,
    spec: &TextSpec,
    font: &FontData,
) -> Result<TextSettings, OpError> {
    let mut t = base.clone();
    t.font = font.font.clone();
    if let Some(v) = &spec.content {
        t.text = v.clone();
    }
    if let Some(v) = spec.size {
        t.size = v;
    }
    if let Some(v) = &spec.color {
        t.color = parse_color(v).ok_or_else(|| {
            OpError::invalid_value(
                format!("色は #rrggbb か #rrggbbaa です（{v}）"),
                format!("A color is #rrggbb or #rrggbbaa ({v})"),
            )
        })?;
    }
    if let Some(v) = spec.line_height {
        t.line_height = v;
    }
    if let Some(v) = spec.letter_spacing {
        t.letter_spacing = v;
    }
    if let Some(v) = spec.align {
        t.align = match v {
            TextAlignName::Left => TextAlign::Left,
            TextAlignName::Center => TextAlign::Center,
            TextAlignName::Right => TextAlign::Right,
        };
    }
    if let Some(v) = spec.x {
        t.x = v;
    }
    if let Some(v) = spec.y {
        t.y = v;
    }
    if let Some(v) = spec.rotation {
        t.rotation = v;
    }
    if let Some(v) = spec.wrap_width {
        t.wrap_width = v;
    }
    t.validate()?;
    Ok(t)
}

/// 層の文字の値（読む命令の返事）。
pub fn text_info(t: &TextSettings) -> TextInfo {
    let (font, font_file, font_index, font_family, font_postscript) = match &t.font {
        TextFont::Bundled(name) => (Some(name.clone()), None, None, None, None),
        TextFont::File {
            path, index, names, ..
        } => (
            None,
            Some(path.clone()),
            Some(*index),
            Some(names.family.clone()).filter(|s| !s.is_empty()),
            Some(names.postscript.clone()).filter(|s| !s.is_empty()),
        ),
    };
    TextInfo {
        content: t.text.clone(),
        font,
        font_file,
        font_index,
        font_family,
        font_postscript,
        size: t.size,
        color: format_color(t.color),
        line_height: t.line_height,
        letter_spacing: t.letter_spacing,
        align: match t.align {
            TextAlign::Left => TextAlignName::Left,
            TextAlign::Center => TextAlignName::Center,
            TextAlign::Right => TextAlignName::Right,
        },
        x: t.x,
        y: t.y,
        rotation: t.rotation,
        wrap_width: t.wrap_width,
    }
}
