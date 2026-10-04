//! .ylp（Unity 版と同じ作業ファイル）の開く・保存・新規。読み書きと検証は yolu-io、ここは画面の状態（テクスチャセット）との受け渡しだけ。
//!
//! - 開く: yolu-io の `SaveTarget::open`（ZIP・manifest・正本を検証し、保存で外からの書き換えを見張る印を取る）で読み、セットごとに
//!   正本（`NativeDocument`）を core の文書へ変える（`to_core`。透明の画素の RGB・文書とレイヤーの ID を保つ）。core で扱えない中身
//!   （マスク・フィルター・Generator・別のチャンネル など。`core_issues`）のあるセットは**読むだけ**にして理由を出す（黙って捨てない）。
//!   読むだけのセットは、保存した合成の PNG（`composite/Color.png`）を 1 枚のレイヤーにして見せる（描けない。Live Link でも Unity に
//!   見せる）。
//! - 保存: 形式 7 で書く（開いたのが古い形式なら yolu-io の `upgraded` で上げてから）。開いた後に描いた・変えたセットだけ core の文書を
//!   正本に戻し（`from_core`。Color の合成の PNG も書く）、描いていないセット・読むだけのセット・知らないエントリは開いた時のバイト列の
//!   まま残す。セットの並び・名前・マテリアルの鍵・今のセットは `with_sets`、ファイルが無かったプロジェクトは `create`。書くのは
//!   yolu-io の安全な保存（検証した一時ファイルから 1 回の置き換え。上書きなら前の版は `<名前>-backups~/` に残す。開いた後に外で
//!   書き換えられていたら断る）。

use std::path::{Path, PathBuf};

use yolu_io::{composite_png, NativeDocument, Project, SaveTarget, SetSpec, WriterInfo};

use crate::engine::{Channel, Document, TileCoord};
use crate::sets::TextureSets;
use crate::state::{blank_document, AppState, DEFAULT_DOCUMENT_SIZE};

/// 開いた・保存した .ylp。
pub struct ProjectFile {
    path: PathBuf,
    /// 開いた・保存した時の印（保存で、外から書き換えられていないかを見る）。
    target: SaveTarget,
    /// 開いた・保存した時の中身（次の保存で、描いていないセット・読むだけのセット・知らないエントリをそのまま残す）。
    original: Project,
}

impl std::fmt::Debug for ProjectFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectFile")
            .field("path", &self.path)
            .field("format", &self.original.info().format)
            .field("sets", &self.original.sets().len())
            .finish()
    }
}

impl ProjectFile {
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// ファイルの .ylp の形式（開いた古い形式は、保存するまでそのまま）。
    pub fn format(&self) -> i32 {
        self.original.info().format
    }
    /// 開いた時の yolu-io の知らせ（移行したこと・知らないエントリなど）。
    pub fn notes(&self) -> &[String] {
        self.original.notes()
    }
    /// 開いた・保存した時の中身。
    pub fn project(&self) -> &Project {
        &self.original
    }
}

/// .ylp の ylp.json に書く書き手（Unity の版の欄はスタンドアロンなので "standalone"）。
pub fn writer() -> WriterInfo {
    WriterInfo {
        app: "YoluPainter-rs".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        unity: "standalone".into(),
    }
}

/// 正本を core の文書へ。扱えない中身があれば、その理由（多ければ初めの 3 つと数）。
fn to_core(native: &NativeDocument) -> Result<Document, String> {
    let issues = native.core_issues();
    if !issues.is_empty() {
        let mut text = issues
            .iter()
            .take(3)
            .map(|i| i.trim_start_matches("M1で保持できない項目: ").to_owned())
            .collect::<Vec<_>>()
            .join("、");
        if issues.len() > 3 {
            text += &format!(" ほか {} 件", issues.len() - 3);
        }
        return Err(format!("core で扱えない中身（{text}）"));
    }
    native
        .to_core()
        .map_err(|e| format!("core の文書にできません: {e}"))
}

/// 保存した合成の PNG から、見せるだけの文書（1 枚のレイヤー）を作る。大きさが正本と違えば使わない。
fn preview_document(png: Option<&[u8]>, width: u32, height: u32) -> (Document, Option<String>) {
    let blank = || {
        let mut doc = Document::new(width, height).expect("正本の大きさは検証済み");
        let _ = doc.add_layer("保存した合成（読むだけ）");
        let _ = doc.clear_history();
        doc
    };
    let Some(png) = png else {
        return (blank(), Some("保存した合成の絵なし".into()));
    };
    let image = match image::load_from_memory_with_format(png, image::ImageFormat::Png) {
        Ok(i) => i.to_rgba8(),
        Err(e) => {
            return (
                blank(),
                Some(format!("保存した合成の絵を読めません（{e}）")),
            )
        }
    };
    if image.width() != width || image.height() != height {
        return (
            blank(),
            Some(format!(
                "保存した合成の絵の大きさ {}×{} が正本の {width}×{height} と違う",
                image.width(),
                image.height()
            )),
        );
    }
    let mut doc = Document::new(width, height).expect("正本の大きさは検証済み");
    let layer = doc
        .add_layer("保存した合成（読むだけ）")
        .expect("空の文書にレイヤーを足せる");
    let ts = doc.tile_size();
    let mut tile = vec![0u8; (ts * ts * 4) as usize];
    let raw = image.as_raw();
    for ty in 0..height.div_ceil(ts) {
        for tx in 0..width.div_ceil(ts) {
            tile.fill(0);
            let mut any = false;
            for r in 0..ts {
                let y = ty * ts + r; // 文書の行（下から）
                if y >= height {
                    break;
                }
                let png_row = (height - 1 - y) as usize; // PNG は上の行から
                let x0 = tx * ts;
                let n = ts.min(width - x0) as usize;
                let src = &raw[(png_row * width as usize + x0 as usize) * 4..][..n * 4];
                tile[(r * ts) as usize * 4..][..n * 4].copy_from_slice(src);
                any |= src.iter().any(|&b| b != 0);
            }
            if any {
                let _ = doc.import_tile(layer, Channel::Color, TileCoord::new(tx, ty), &tile);
            }
        }
    }
    let _ = doc.clear_history();
    (doc, None)
}

/// .ylp を開いて今の状態を置き換える。開けなければ何も変えずに理由を出す。
pub fn open_into(state: &mut AppState, path: &Path) {
    let (project, target) = match SaveTarget::open(path) {
        Ok(x) => x,
        Err(e) => {
            state.message = format!("開けません: {}: {e}", path.display());
            return;
        }
    };
    let entries = project.migrated_entries();
    let mut parts = Vec::with_capacity(project.sets().len());
    let mut read_only = Vec::new();
    for set in project.sets() {
        let native = &set.document;
        let (w, h) = (native.width() as u32, native.height() as u32);
        match to_core(native) {
            Ok(doc) => parts.push((
                set.id.clone(),
                set.name.clone(),
                set.material.clone(),
                None,
                doc,
            )),
            Err(reason) => {
                read_only.push(set.name.clone());
                let png = entries
                    .get(&format!("sets/{}/composite/Color.png", set.id))
                    .map(|b| &b[..]);
                let (doc, note) = preview_document(png, w, h);
                let reason = match note {
                    Some(n) => format!("{reason}。{n}"),
                    None => reason,
                };
                parts.push((
                    set.id.clone(),
                    set.name.clone(),
                    set.material.clone(),
                    Some(reason),
                    doc,
                ));
            }
        }
    }
    let current = project
        .sets()
        .iter()
        .position(|s| s.id == project.current_set())
        .unwrap_or(0);
    let count = parts.len();
    let (sets, doc) = TextureSets::from_parts(parts, current);
    state.replace_sets(sets, doc);
    state.project_name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "名称未設定".into());
    state.modified = false;
    let mut text = format!(
        "開きました: {}（形式 {}・テクスチャセット {count}）。",
        path.display(),
        project.info().format
    );
    if !read_only.is_empty() {
        text += &format!(
            " 読むだけのセット {}: {}。",
            read_only.len(),
            read_only.join("・")
        );
    }
    // io の知らせのうち、セットごとの変換の理由は上で言ったので除く
    let notes: Vec<&String> = project
        .notes()
        .iter()
        .filter(|n| !n.contains("coreへ変換できません"))
        .collect();
    if !notes.is_empty() {
        text += &format!(
            " {}",
            notes
                .iter()
                .map(|n| n.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    state.message = text;
    state.project = Some(ProjectFile {
        path: path.to_path_buf(),
        target,
        original: project,
    });
}

/// 新しいプロジェクト（空の 2048² のセット 1 つ）にする。Live Link のモデルがあれば、そのマテリアルにセットを付ける。
pub fn new_into(state: &mut AppState) {
    let (doc, _) = blank_document(DEFAULT_DOCUMENT_SIZE, DEFAULT_DOCUMENT_SIZE);
    let sets = TextureSets::first(&doc);
    state.replace_sets(sets, doc);
    state.project = None;
    state.project_name = "名称未設定".into();
    state.modified = false;
    state.message = "新しいプロジェクトを作りました。".into();
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// 保存する。失敗したら何も変えずに理由を出す（ファイルは yolu-io の安全な保存なので、前の中身のまま）。
pub fn save_from(state: &mut AppState, path: &Path) {
    match save(state, path) {
        Ok(text) => state.message = text,
        Err(e) => state.message = format!("保存できません: {}: {e}", path.display()),
    }
}

fn save(state: &mut AppState, path: &Path) -> Result<String, String> {
    if state.is_stroking() {
        return Err("描いている間は保存しません".into());
    }
    let base = state.project.as_ref().map(|p| &p.original);
    let mut specs = Vec::with_capacity(state.sets.len());
    let mut written = Vec::new();
    for (i, set) in state.sets.iter().enumerate() {
        let doc = state.set_doc(i);
        let in_base = base.is_some_and(|b| b.sets().iter().any(|s| s.id == set.id));
        let unchanged = set.saved == Some((doc.id(), doc.revision()));
        if set.read_only.is_some() && !in_base {
            return Err(format!(
                "読むだけのセット「{}」の元の正本がありません",
                set.name
            ));
        }
        let (document, composite_color) = if in_base && (set.read_only.is_some() || unchanged) {
            (None, None)
        } else {
            let native = NativeDocument::from_core(doc)
                .map_err(|e| format!("セット「{}」を正本にできません: {e}", set.name))?;
            let png = composite_png(doc)
                .map_err(|e| format!("セット「{}」の合成の PNG を作れません: {e}", set.name))?;
            written.push((i, doc.id(), doc.revision()));
            (Some(native), Some(png))
        };
        specs.push(SetSpec {
            id: set.id.clone(),
            name: set.name.clone(),
            material: set.material.clone(),
            document,
            composite_color,
        });
    }
    let current = state.sets.current().id.clone();
    let project = match base {
        Some(b) => {
            let upgraded;
            let b = if b.info().format < 7 {
                upgraded = b.upgraded(writer()).map_err(|e| e.to_string())?;
                &upgraded
            } else {
                b
            };
            b.with_sets(writer(), &specs, &current)
        }
        None => Project::create(writer(), &specs, &current),
    }
    .map_err(|e| e.to_string())?;
    let overwrite = path.exists();
    let reuse = state
        .project
        .as_ref()
        .is_some_and(|p| same_file(&p.path, path));
    let stamp = if reuse {
        let file = state.project.as_mut().expect("上で確かめた");
        file.target.save(&project).map_err(|e| e.to_string())?
    } else {
        // 別の場所: あれば .ylp として読めるものだけを上書きする（読めないファイルを黙って潰さない）
        let mut target = if overwrite {
            SaveTarget::open(path)
                .map_err(|e| format!("上書きする先を .ylp として読めません（{e}）"))?
                .1
        } else {
            SaveTarget::create(path).map_err(|e| e.to_string())?
        };
        let stamp = target.save(&project).map_err(|e| e.to_string())?;
        state.project = Some(ProjectFile {
            path: path.to_path_buf(),
            target,
            original: project.clone(),
        });
        stamp
    };
    let _ = stamp;
    if let Some(file) = state.project.as_mut() {
        file.original = project;
        file.path = path.to_path_buf();
    }
    for (i, id, revision) in &written {
        if let Some(set) = state.sets.get_mut(*i) {
            set.saved = Some((*id, *revision));
        }
    }
    state.modified = false;
    state.project_name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "名称未設定".into());
    let mut text = format!(
        "保存しました: {}（形式 7・テクスチャセット {}・書き直した正本 {}）。",
        path.display(),
        state.sets.len(),
        written.len()
    );
    if overwrite {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        text += &format!(" 前の版は {name}-backups~ に残しました。");
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_flips_the_png_rows_to_bottom_up() {
        // 3 × 2: 上の行が赤、下の行が透明（RGB は残す）
        let mut img = image::RgbaImage::new(3, 2);
        for x in 0..3 {
            img.put_pixel(x, 0, image::Rgba([255, 0, 0, 255]));
            img.put_pixel(x, 1, image::Rgba([10, 20, 30, 0]));
        }
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let (doc, note) = preview_document(Some(&png), 3, 2);
        assert!(note.is_none());
        let px = |x, y| crate::engine::layer_pixel(&doc.layers()[0], x, y);
        assert_eq!(
            px(1, 1),
            [255, 0, 0, 255],
            "PNG の上の行は文書の上（y = 1）"
        );
        assert_eq!(px(1, 0), [10, 20, 30, 0], "透明の画素の RGB も保つ");
        assert!(!doc.can_undo());
        let (_, note) = preview_document(Some(&png), 4, 2);
        assert!(note.unwrap().contains("大きさ"));
        let (doc, note) = preview_document(None, 8, 8);
        assert!(note.is_some());
        assert_eq!((doc.width(), doc.layers().len()), (8, 1));
    }
}
