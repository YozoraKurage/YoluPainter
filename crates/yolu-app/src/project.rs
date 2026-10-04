//! .ylp（Unity 版と同じ作業ファイル）の開く・保存・新規。読み書きと検証は yolu-io、ここは画面の状態（テクスチャセット）との受け渡しだけ。
//!
//! - 開く: yolu-io の `SaveTarget::open`（ZIP・manifest・正本を検証し、保存で外からの書き換えを見張る印を取る）で読み、セットごとに
//!   正本（`NativeDocument`）を core の文書へ変える（`to_core`。透明の画素の RGB・文書とレイヤーの ID を保つ）。core で扱えない中身
//!   （ロック・手動の ID の色。`core_issues`）のあるセットは**読むだけ**にして理由を出す（黙って捨てない）。
//!   グループ・マスク・塗りつぶし・調整・クリッピング・チャンネルごとの合成・ユーザーチャンネルと、効果（フィルター・Generator・Anchor・
//!   塗りつぶしの画像・グラデーション・パス）は core が持つので、描けて保存で保たれる。
//!   読むだけのセットは、保存した合成の PNG（`composite/Color.png`）を 1 枚のレイヤーにして見せる（描けない。Live Link でも Unity に
//!   見せる）。
//! - 保存: 形式 7 で書く（開いたのが古い形式なら yolu-io の `upgraded` で上げてから）。開いた後に描いた・変えたセットだけ core の文書を
//!   正本に戻し（`from_core`。Color の合成の PNG も書く）、描いていないセット・読むだけのセット・知らないエントリは開いた時のバイト列の
//!   まま残す。セットの並び・名前・マテリアルの鍵・今のセットは `with_sets`、ファイルが無かったプロジェクトは `create`。書くのは
//!   yolu-io の安全な保存（検証した一時ファイルから 1 回の置き換え。上書きなら前の版は `<名前>-backups~/` に、設定の「退避を残す数」
//!   （既定はすべて）だけ残す。開いた後に外で書き換えられていたら断る）。

use std::path::{Path, PathBuf};

use yolu_core::mesh_maps::MeshMapKind;
use yolu_io::{composite_pngs, NativeDocument, Project, SaveTarget, SetSpec, WriterInfo};

/// 1 枚のメッシュマップの読み込みの上限（予算。壊れた・大きすぎるものは読まずに知らせる）。
const MESH_MAP_LIMIT_BYTES: usize = 512 * 1024 * 1024;

use crate::engine::{Channel, Document, TileCoord};
use crate::lang::Lang;
use crate::sets::TextureSets;
use crate::shelf::ShelfState;
use crate::state::{blank_document_in, AppState, DEFAULT_DOCUMENT_SIZE};

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
    pub fn notes(&self) -> &[yolu_io::Note] {
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
fn to_core(native: &NativeDocument, lang: Lang) -> Result<Document, String> {
    let issues = native.core_issues();
    if !issues.is_empty() {
        return Err(lang.unsupported_features(&issues));
    }
    let doc = native.to_core().map_err(|e| {
        format!("{}: {}", lang.pick("core の文書にできません", "Cannot convert to a core document"), lang.io_error(&e))
    })?;
    // 効果の入力（焼いたメッシュマップ・モデルのルート・画像）はまだ app から文書へ渡していない。効果が効かない（入力のまま通る）文書は、
    // 画面にも保存した合成にも効果が出ないので、編集させず保存済みの合成を見せる（効く効果だけの文書は評価されるので編集できる）
    let inactive = doc.inactive_effect_list();
    if !inactive.is_empty() {
        return Err(lang.inactive_effects(&inactive));
    }
    Ok(doc)
}

/// 保存した合成の PNG から、見せるだけの文書（1 枚のレイヤー）を作る。大きさが正本と違えば使わない。
pub(crate) fn preview_document(png: Option<&[u8]>, width: u32, height: u32, lang: Lang) -> (Document, Option<String>) {
    let blank = || {
        let mut doc = Document::new(width, height).expect("正本の大きさは検証済み");
        let _ = doc.add_layer(lang.pick("保存した合成（読むだけ）", "Saved composite (read-only)"));
        let _ = doc.clear_history();
        doc
    };
    let Some(png) = png else {
        return (blank(), Some(lang.pick("保存した合成の絵なし", "No saved composite").into()));
    };
    let image = match image::load_from_memory_with_format(png, image::ImageFormat::Png) {
        Ok(i) => i.to_rgba8(),
        Err(e) => {
            return (
                blank(),
                Some(lang.pick(format!("保存した合成の絵を読めません（{e}）"), format!("Invalid saved composite ({e})"))),
            )
        }
    };
    if image.width() != width || image.height() != height {
        return (
            blank(),
            Some(lang.pick(format!(
                "保存した合成の絵の大きさ {}×{} が正本の {width}×{height} と違う",
                image.width(),
                image.height()
            ), format!(
                "Saved composite size {}×{} differs from document {width}×{height}",
                image.width(),
                image.height()
            ))),
        );
    }
    let mut doc = Document::new(width, height).expect("正本の大きさは検証済み");
    let layer = doc
        .add_layer(lang.pick("保存した合成（読むだけ）", "Saved composite (read-only)"))
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
            state.message = format!("{}: {}: {}", state.lang.pick("開けません", "Cannot open"), path.display(), state.lang.io_error(&e));
            return;
        }
    };
    let entries = project.migrated_entries();
    let mut parts = Vec::with_capacity(project.sets().len());
    let mut read_only = Vec::new();
    let mut selection_issues = Vec::new();
    for set in project.sets() {
        let native = &set.document;
        let (w, h) = (native.width() as u32, native.height() as u32);
        match to_core(native, state.lang) {
            Ok(mut doc) => {
                // 選択範囲（selection.bin）は文書に戻す（読めなければ選択なしで開き、理由を出す）
                if let Err(e) =
                    crate::selection::io::restore_into(&mut doc, set.selection.as_ref(), state.lang)
                {
                    selection_issues.push(format!("{}: {e}", set.name));
                }
                parts.push((
                    set.id.clone(),
                    set.name.clone(),
                    set.material.clone(),
                    None,
                    doc,
                ))
            }
            Err(reason) => {
                read_only.push(set.name.clone());
                let png = entries
                    .get(&format!("sets/{}/composite/Color.png", set.id))
                    .map(|b| &b[..]);
                let (doc, note) = preview_document(png, w, h, state.lang);
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
    let (mut sets, doc) = TextureSets::from_parts(parts, current);
    // メッシュマップ（セットごとの派生物）。壊れていれば読まずに知らせる（ファイルのエントリはそのまま残る）
    let (mut map_count, mut map_problems) = (0usize, Vec::new());
    for (i, set) in project.sets().iter().enumerate() {
        for kind in MeshMapKind::ALL {
            match project.mesh_map(&set.id, kind, MESH_MAP_LIMIT_BYTES) {
                Ok(Some(map)) => {
                    if let Some(target) = sets.get_mut(i) {
                        target.mesh_maps.load(map);
                        map_count += 1;
                    }
                }
                Ok(None) => {}
                Err(e) => map_problems.push(format!("{} {}: {e}", set.name, kind.name())),
            }
        }
    }
    state.replace_sets(sets, doc);
    state.shelf = ShelfState::from_project(&project).inherit_running_from(&state.shelf);
    state.project_name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| state.lang.pick("名称未設定", "Untitled").into());
    state.modified = false;
    let mut text = state.lang.pick(format!(
        "開きました: {}（形式 {}・テクスチャセット {count}）。",
        path.display(),
        project.info().format
    ), format!(
        "Opened: {} (format {} · {count} texture sets).",
        path.display(),
        project.info().format
    ));
    if !read_only.is_empty() {
        text += &state.lang.pick(format!(
            " 読むだけのセット {}: {}。",
            read_only.len(),
            read_only.join("・")
        ), format!(
            " Read-only texture sets ({}): {}.",
            read_only.len(),
            read_only.join(", ")
        ));
    }
    if !selection_issues.is_empty() {
        text += &format!(" {}", selection_issues.join(" "));
    }
    if let Some(notice) = unreadable_shelf_notice(state) {
        text += &notice;
    }
    if map_count > 0 {
        text += &state.lang.pick(format!(" メッシュマップ {map_count} 枚。"), format!(" {map_count} mesh map(s)."));
    }
    if !map_problems.is_empty() {
        text += &format!(
            " 読めないメッシュマップ {}（ファイルには残っています）: {}。",
            map_problems.len(),
            map_problems.join("、")
        );
    }
    // io の知らせのうち、セットごとの変換の理由は上で言ったので除く
    let notes: Vec<String> = project
        .notes()
        .iter()
        .filter(|n| !matches!(n, yolu_io::Note::SetNotConvertible { .. }))
        .map(|n| state.lang.project_note(n))
        .collect();
    if !notes.is_empty() {
        text += &state.lang.pick(format!(" {}", notes.join(" ")), format!(" {}.", notes.join("; ")));
    }
    state.message = text;
    state.project = Some(ProjectFile {
        path: path.to_path_buf(),
        target,
        original: project,
    });
}

/// 開いたときの知らせのうち、棚を読めなかった分（読めた棚には None。先頭に空白を置いて、知らせの文へ続ける）。
pub fn unreadable_shelf_notice(state: &AppState) -> Option<String> {
    let reason = state.shelf.unavailable.as_ref()?.reason(state.lang);
    Some(state.lang.pick(
        format!(" アセットの棚を読めません（{reason}）。"),
        format!(" The asset shelf cannot be read ({reason})."),
    ))
}

/// 新しいプロジェクト（空の 2048² のセット 1 つ）にする。Live Link のモデルがあれば、そのマテリアルにセットを付ける。
pub fn new_into(state: &mut AppState) {
    let (doc, _) = blank_document_in(DEFAULT_DOCUMENT_SIZE, DEFAULT_DOCUMENT_SIZE, state.lang);
    let sets = TextureSets::first_in(&doc, state.lang);
    state.replace_sets(sets, doc);
    state.shelf = ShelfState::default().inherit_running_from(&state.shelf);
    state.project = None;
    state.project_name = state.lang.pick("名称未設定", "Untitled").into();
    state.modified = false;
    state.message = state.lang.pick("新しいプロジェクトを作りました。", "New project created.").into();
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
        Err(e) => {
            state.message = format!(
                "{}: {}: {e}",
                state.lang.pick("保存できません", "Cannot save"),
                path.display()
            )
        }
    }
}

fn save(state: &mut AppState, path: &Path) -> Result<String, String> {
    if state.is_stroking() {
        return Err(state.lang.pick("描いている間は保存しません", "Cannot save during a stroke").into());
    }
    let base = state.project.as_ref().map(|p| &p.original);
    let mut specs = Vec::with_capacity(state.sets.len());
    let mut written = Vec::new();
    for (i, set) in state.sets.iter().enumerate() {
        let doc = state.set_doc(i);
        let in_base = base.is_some_and(|b| b.sets().iter().any(|s| s.id == set.id));
        let unchanged = set.saved == Some((doc.id(), doc.revision()));
        if set.read_only.is_some() && !in_base {
            return Err(state.lang.pick(format!(
                "読むだけのセット「{}」の元の正本がありません",
                set.name
            ), format!(
                "Original document missing for read-only set “{}”",
                set.name
            )));
        }
        let (document, composites) = if in_base && (set.read_only.is_some() || unchanged) {
            (None, Vec::new())
        } else {
            let native = NativeDocument::from_core(doc)
                .map_err(|e| {
                    let what = state.lang.pick(format!("セット「{}」を正本にできません", set.name), format!("Cannot convert texture set “{}” to a document", set.name));
                    format!("{what}: {}", state.lang.io_error(&e))
                })?;
            let pngs = composite_pngs(doc).map_err(|e| {
                let what = state.lang.pick(format!("セット「{}」の合成の PNG を作れません", set.name), format!("Cannot build the composite PNG of texture set “{}”", set.name));
                format!("{what}: {}", state.lang.io_error(&e))
            })?;
            written.push((i, doc.id(), doc.revision()));
            (Some(native), pngs)
        };
        specs.push(SetSpec {
            id: set.id.clone(),
            name: set.name.clone(),
            material: set.material.clone(),
            document,
            composites,
        });
    }
    let current = state.sets.current().id.clone();
    let project = match base {
        Some(b) => {
            let upgraded;
            let b = if b.info().format < 7 {
                upgraded = b.upgraded(writer()).map_err(|e| state.lang.io_error(&e))?;
                &upgraded
            } else {
                b
            };
            b.with_sets(writer(), &specs, &current)
        }
        None => Project::create(writer(), &specs, &current),
    }
    .map_err(|e| state.lang.io_error(&e))?;
    // 選択範囲（selection.bin）は正本と別のエントリ。読むだけのセットは元のまま、描けるセットは今の選択範囲と違えば書き換える
    let selections: Vec<(&str, Option<&crate::engine::SelectionMask>)> = state
        .sets
        .iter()
        .enumerate()
        .filter(|(_, set)| set.read_only.is_none())
        .map(|(i, set)| (set.id.as_str(), state.set_doc(i).selection()))
        .collect();
    let project = crate::selection::io::write_into(project, &selections, state.lang)?;
    // 焼いてまだ書いていないメッシュマップ（開いた時のものは、ファイルのバイト列のまま残っている）
    let mut project = project;
    let mut maps_written = Vec::new();
    for (i, set) in state.sets.iter().enumerate() {
        let unsaved = set.mesh_maps.unsaved();
        for map in &unsaved {
            project = project.with_mesh_map(&set.id, map).map_err(|e| {
                state.lang.pick(
                    format!("セット「{}」のメッシュマップ: {}", set.name, state.lang.io_error(&e)),
                    format!("Mesh maps of set \"{}\": {}", set.name, state.lang.io_error(&e)),
                )
            })?;
        }
        if !unsaved.is_empty() {
            maps_written.push((i, unsaved));
        }
    }
    // アセットの棚: 変えたときだけ resources を書き直す（変えていなければ開いたファイルのバイト列のまま）
    let project = state.shelf.write_into(project, state.lang)?;
    let overwrite = path.exists();
    let reuse = state
        .project
        .as_ref()
        .is_some_and(|p| same_file(&p.path, path));
    let keep = state.prefs.backups;
    let report = if reuse {
        let file = state.project.as_mut().expect("上で確かめた");
        file.target.save_with(&project, keep).map_err(|e| state.lang.io_error(&e))?
    } else {
        // 別の場所: あれば .ylp として読めるものだけを上書きする（読めないファイルを黙って潰さない）
        let mut target = if overwrite {
            SaveTarget::open(path)
                .map_err(|e| format!("{}: {}", state.lang.pick("上書きする先を .ylp として読めません", "Invalid overwrite target"), state.lang.io_error(&e)))?
                .1
        } else {
            SaveTarget::create(path).map_err(|e| state.lang.io_error(&e))?
        };
        let report = target.save_with(&project, keep).map_err(|e| state.lang.io_error(&e))?;
        state.project = Some(ProjectFile {
            path: path.to_path_buf(),
            target,
            original: project.clone(),
        });
        report
    };
    if let Some(file) = state.project.as_mut() {
        file.original = project;
        file.path = path.to_path_buf();
    }
    for (i, id, revision) in &written {
        if let Some(set) = state.sets.get_mut(*i) {
            set.saved = Some((*id, *revision));
        }
    }
    let map_total: usize = maps_written.iter().map(|(_, m)| m.len()).sum();
    for (i, maps) in &maps_written {
        if let Some(set) = state.sets.get_mut(*i) {
            set.mesh_maps.mark_saved(maps);
        }
    }
    state.modified = false;
    state.shelf.changed = false;
    state.project_name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| state.lang.pick("名称未設定", "Untitled").into());
    let mut text = state.lang.pick(format!(
        "保存しました: {}（形式 7・テクスチャセット {}・書き直した正本 {}）。",
        path.display(),
        state.sets.len(),
        written.len()
    ), format!(
        "Saved: {} (format 7 · {} texture sets · {} updated documents).",
        path.display(),
        state.sets.len(),
        written.len()
    ));
    if map_total > 0 {
        text += &state.lang.pick(
            format!(" メッシュマップ {map_total} 枚を書きました。"),
            format!(" Wrote {map_total} mesh map(s)."),
        );
    }
    // 書き直したセットに入力のまま通る効果があれば、合成の PNG に入っていないことを言う（正本には設定が残る）
    for (i, _, _) in &written {
        let inactive = state.set_doc(*i).inactive_effect_list();
        if !inactive.is_empty() {
            let set = state.sets.get(*i).map_or("", |s| s.name.as_str());
            text += &state.lang.inactive_effects_not_in_composite(set, &inactive);
        }
    }
    text += &backup_text(state.lang, path, &report);
    Ok(text)
}

/// 保存の知らせのうち退避の文。前の版は、退避する設定で上書きしたときだけ残る（退避しない設定・新規の保存では作らない）。
/// 古い退避の整理で消せなかったものは、保存の成功と別に知らせる（保存は済んでいる）。
fn backup_text(lang: Lang, path: &Path, report: &yolu_io::SaveReport) -> String {
    let mut text = String::new();
    if report.backup.is_some() {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        text += &lang.pick(format!(" 前の版は {name}-backups~ に残しました。"), format!(" Previous version: {name}-backups~."));
    }
    if let Some(first) = report.prune_failures.first() {
        let n = report.prune_failures.len();
        let reason = lang.file_error(&first.error);
        text += &lang.pick(
            format!(" 古い退避 {n} 件を消せませんでした（{reason}）。"),
            format!(" Could not delete {n} old backup{} ({reason}).", if n == 1 { "" } else { "s" }),
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backup_text_names_the_kept_version_and_the_old_ones_that_could_not_be_deleted() {
        use yolu_io::{FileStamp, PruneFailure, SaveReport};
        let report = |backup: bool, failures: usize| SaveReport {
            stamp: FileStamp { sha256: String::new(), length: 0, modified: std::time::UNIX_EPOCH },
            backup: backup.then(|| PathBuf::from("a.ylp-backups~/a-20260101T000000000Z.ylp")),
            prune_failures: (0..failures)
                .map(|i| PruneFailure {
                    path: PathBuf::from(format!("old-{i}.ylp")),
                    error: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
                })
                .collect(),
        };
        let path = Path::new("dir/a.ylp");
        // 退避しない設定・新規の保存は、何も言わない
        assert_eq!(backup_text(Lang::Ja, path, &report(false, 0)), "");
        assert_eq!(backup_text(Lang::En, path, &report(false, 0)), "");
        assert_eq!(backup_text(Lang::Ja, path, &report(true, 0)), " 前の版は a.ylp-backups~ に残しました。");
        assert_eq!(backup_text(Lang::En, path, &report(true, 0)), " Previous version: a.ylp-backups~.");
        // 整理で消せなかったものは件数と理由（OS のエラーの種類）。保存の成功の文は消さない
        let one = backup_text(Lang::En, path, &report(true, 1));
        assert_eq!(one, " Previous version: a.ylp-backups~. Could not delete 1 old backup (Access denied).");
        let many = backup_text(Lang::En, path, &report(true, 3));
        assert!(many.ends_with("Could not delete 3 old backups (Access denied)."), "{many}");
        let ja = backup_text(Lang::Ja, path, &report(true, 2));
        assert!(ja.contains("古い退避 2 件を消せませんでした（アクセスが拒否されました）"), "{ja}");
        // 退避しない設定のとき整理は走らないが、理由だけがある報告でも文にする
        assert!(backup_text(Lang::En, path, &report(false, 1)).starts_with(" Could not delete 1 old backup"));
    }

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
        let (doc, note) = preview_document(Some(&png), 3, 2, Lang::Ja);
        assert!(note.is_none());
        let px = |x, y| crate::engine::layer_pixel(&doc.layers()[0], x, y);
        assert_eq!(
            px(1, 1),
            [255, 0, 0, 255],
            "PNG の上の行は文書の上（y = 1）"
        );
        assert_eq!(px(1, 0), [10, 20, 30, 0], "透明の画素の RGB も保つ");
        assert!(!doc.can_undo());
        let (_, note) = preview_document(Some(&png), 4, 2, Lang::Ja);
        assert!(note.unwrap().contains("大きさ"));
        let (doc, note) = preview_document(None, 8, 8, Lang::Ja);
        assert!(note.is_some());
        assert_eq!((doc.width(), doc.layers().len()), (8, 1));
    }
}
