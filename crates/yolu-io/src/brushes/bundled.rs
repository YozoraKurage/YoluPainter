//! 同梱の筆先（Krita 4 の既定の筆先、CC0 1.0。データは葉のクレート `yolu-brush-sets` に原本のまま入っている）。
//!
//! 取り込みと同じ読み手（GBR・GIH・PNG）で読み、各筆先にこのツールの既定の設定を付けてブラシにする（Krita のプリセットではない）。
//! 原寸（最大 600 画素ほど）のままでは既定として大きすぎるので、半径を 4〜40 に収め、筆圧で不透明度は変えない。
//! 初めて呼んだときに 1 回だけ全部を読み、後は同じ結果を返す。

use std::path::Path;
use std::sync::OnceLock;

use yolu_brush_sets::{Tip, KRITA4};
use yolu_core::Brush;

use super::{import_bytes, pretty_name, BrushImportError, FileKind, Source, Unrepresented};

/// 同梱の筆先の ID の頭（`bundled:krita4/<ファイル名>`）。
pub const ID_PREFIX: &str = "bundled:";
/// ブラシの分類の名前。
pub const KRITA4_CATEGORY: &str = "Krita";

/// 同梱のブラシ 1 つ（筆先の画像 1 ファイルに 1 つ）。
#[derive(Clone, Debug, PartialEq)]
pub struct BundledBrush {
    /// `bundled:krita4/<ファイル名>`。ブラシが指す筆先の ID として保存できる（同じファイルなら同じ ID）。
    pub id: String,
    /// 常に `Source::BundledKrita4`（元のファイルの形式ではなく、同梱の出どころ）。
    pub source: Source,
    pub category: &'static str,
    pub name: String,
    pub brush: Brush,
    pub unrepresented: Vec<Unrepresented>,
}

/// 同梱の筆先の読み込み結果。
#[derive(Debug)]
pub struct BundledSet {
    pub brushes: Vec<BundledBrush>,
    /// 読めなかったファイル（通常は空。試験で確かめる）。
    pub failures: Vec<(String, BrushImportError)>,
}

/// 同梱の Krita 4 の既定の筆先（ファイル名の順）。
pub fn krita4() -> &'static BundledSet {
    static SET: OnceLock<BundledSet> = OnceLock::new();
    SET.get_or_init(|| {
        let mut set = BundledSet {
            brushes: Vec::new(),
            failures: Vec::new(),
        };
        for tip in KRITA4.iter() {
            match load(tip) {
                Ok(brush) => set.brushes.push(brush),
                Err(e) => set.failures.push((tip.file.to_string(), e)),
            }
        }
        set
    })
}

/// ID から（同梱でない ID は None）。
pub fn find(id: &str) -> Option<&'static BundledBrush> {
    krita4().brushes.iter().find(|b| b.id == id)
}

/// 原本のバイト列（`SHA256SUMS` と照合できる）。
pub fn original_bytes(file: &str) -> Option<&'static [u8]> {
    KRITA4.iter().find(|t| t.file == file).map(|t| t.bytes)
}

fn load(tip: &Tip) -> Result<BundledBrush, BrushImportError> {
    let kind = Path::new(tip.file)
        .extension()
        .and_then(|e| e.to_str())
        .and_then(|e| FileKind::from_extension(e).ok())
        .ok_or(BrushImportError::Unsupported(
            super::UnsupportedFile::Extension(tip.file.chars().take(16).collect()),
        ))?;
    let fallback = pretty_name(tip.file);
    let mut set = import_bytes(kind, tip.bytes, Some(&fallback))?;
    let imported = set.brushes.remove(0);
    let mut brush = imported.brush;
    brush.base.radius = brush.base.radius.clamp(4.0, 40.0);
    brush.base.pressure_opacity = false;
    // 名前が無い筆（Krita は "Layer" と付けることがある）はファイル名から
    let name = if imported.name == "Layer" || imported.name.is_empty() {
        fallback
    } else {
        imported.name
    };
    Ok(BundledBrush {
        id: format!("{ID_PREFIX}krita4/{}", tip.file),
        source: Source::BundledKrita4,
        category: KRITA4_CATEGORY,
        name,
        brush,
        unrepresented: imported.unrepresented,
    })
}
