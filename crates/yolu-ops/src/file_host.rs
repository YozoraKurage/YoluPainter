//! 画面なしのホスト。.ylp を開いて、core の文書で命令を当て、保存は yolu-io の安全な保存（検証した一時ファイルから 1 回の置き換え・
//! 開いたあとの外からの書き換えの検出・前の版は退避）で書く。起動中のアプリを必要としない。
//!
//! - **開く**: `SaveTarget::open_within`（ファイルを流して全エントリを確かめる。上限は既定の予算）。セットの文書は、命令が触ったときに 1 つずつ
//!   core の文書にする（触らないセットはメモリに読まない）。core で扱えない中身があるセットは**読むだけ**にして理由を言い、編集は断る
//!   （黙って捨てない）。
//! - **取り消し**はセッションの中だけ（開き直すと無い）。ディスクのファイルに触るのは `save`・`save_as` と書き出しだけ。
//! - **保存**: 編集したセットだけ core の文書の写しを正本の元にして作り直し（Color などの合成の PNG も）、編集していないセット・知らない
//!   エントリ・選択範囲・見た目などは開いたファイルのバイト列のまま写す。古い形式のファイルは形式 7 へ上げて保存する（`upgraded_from` に出る）。
//!   前の版は退避のフォルダへ残し、削除はしない。
//! - **効かない効果**: マップ・モデルを要る Generator などは、画面なしでは入力が無く、入力のまま通す。設定は文書に残るが、合成の PNG・
//!   書き出し・見本には入らない（返事の `notes` / `inactive_effects` に出る）。プロジェクトの画像（塗りつぶしの画像）は渡す。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;
use yolu_core::{Document, EffectInputs, ImageId, ImageInput};
use yolu_io::{
    composite_pngs, BackupKeep, DocumentSource, Limits, Project, SaveTarget, SetSpec, WriterInfo,
};

use crate::doc_ops::{inactive_texts, SetFacts};
use crate::error::{ErrorCode, OpError};
use crate::host::{read_only_error, OpHost, SaveJob, SetView};
use crate::path::PathPolicy;
use crate::reply::*;
use crate::text::Text;

/// .ylp に書く書き手（アプリと同じ名前・版・"standalone"）。
pub fn writer() -> WriterInfo {
    WriterInfo {
        app: "YoluPainter".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        unity: "standalone".into(),
    }
}

/// ホストの設定。
#[derive(Clone, Debug)]
pub struct FileHostConfig {
    /// .ylp を開く上限（既定の予算）。
    pub limits: Limits,
    /// セット 1 つを core の文書にするときの、レイヤーの画素に許すバイト数。
    pub source_budget: u64,
    /// 上書き保存で前の版（退避）をいくつ残すか。既定はすべて残す（消さない）。
    pub backups: BackupKeep,
}

impl Default for FileHostConfig {
    fn default() -> Self {
        FileHostConfig {
            limits: Limits::default(),
            source_budget: yolu_core::DEFAULT_SOURCE_BUDGET_BYTES,
            backups: BackupKeep::All,
        }
    }
}

/// 画面なしのホスト。
pub struct FileHost {
    policy: PathPolicy,
    config: FileHostConfig,
    open: Option<Opened>,
}

struct Opened {
    path: PathBuf,
    stem: String,
    target: SaveTarget,
    project: Project,
    sets: Vec<Slot>,
    current: String,
    /// プロジェクトの画像（塗りつぶしの画像が指すものを渡すために、1 度だけ復号して覚える。復号できなければ空）。
    images: Option<Vec<(ImageId, ImageInput)>>,
}

struct Slot {
    id: String,
    name: String,
    state: State,
}

enum State {
    /// まだ core の文書にしていない。
    Unloaded,
    /// core の文書。`saved` は開いた・保存したときの（文書の ID, 変更番号）。
    Loaded {
        doc: Box<Document>,
        saved: (u128, u64),
    },
    /// core で扱えない中身があるので、読むだけ。
    ReadOnly(Text),
}

impl Slot {
    fn unsaved(&self) -> bool {
        match &self.state {
            State::Loaded { doc, saved } => (doc.id(), doc.revision()) != *saved,
            _ => false,
        }
    }
}

impl FileHost {
    /// 文書を開いていないホスト（`doc.open` で開く）。
    pub fn new(policy: PathPolicy) -> Self {
        Self::with_config(policy, FileHostConfig::default())
    }
    pub fn with_config(policy: PathPolicy, config: FileHostConfig) -> Self {
        FileHost {
            policy,
            config,
            open: None,
        }
    }
    /// .ylp を開いたホスト（作業のフォルダは今のフォルダ）。
    pub fn open_file(path: impl AsRef<Path>) -> Result<Self, OpError> {
        let policy = PathPolicy::current_dir().map_err(|e| OpError::from_io(&e.into()))?;
        let mut host = Self::new(policy);
        host.open(path.as_ref(), true)?;
        Ok(host)
    }
    /// 開いている .ylp の場所。
    pub fn path(&self) -> Option<&Path> {
        self.open.as_ref().map(|o| o.path.as_path())
    }
    /// 保存していない変更があるか。
    pub fn has_unsaved_changes(&self) -> bool {
        self.open
            .as_ref()
            .is_some_and(|o| o.sets.iter().any(Slot::unsaved))
    }
    /// セットの文書を、変えずに読む（試験・呼び手の確認用）。読むだけのセットは断る。
    pub fn with_document<R>(
        &mut self,
        set: Option<&str>,
        f: impl FnOnce(&Document) -> R,
    ) -> Result<R, OpError> {
        let budget = self.config.source_budget;
        let o = self.opened_mut()?;
        let index = o.resolve_set(set)?;
        o.ensure_loaded(index, budget)?;
        match &o.sets[index].state {
            State::Loaded { doc, .. } => Ok(f(doc)),
            State::ReadOnly(reason) => Err(read_only_error(&o.sets[index].name, Some(reason))),
            State::Unloaded => Err(internal("セットを読み込めていません")),
        }
    }

    fn opened_mut(&mut self) -> Result<&mut Opened, OpError> {
        self.open.as_mut().ok_or_else(OpError::no_document)
    }
}

/// 2 つのパスが同じファイルを指すか。書き方の違い（`..`・大文字小文字だけの違い・リンク・ハードリンク）に依らない。
/// どちらかがまだ無いときは、親のフォルダを実体にしたものと名前で比べる（`..` や上のフォルダのリンクは畳める。
/// 無いファイルの名前の大文字小文字の違いは、実体が無いので同じとは言えない）。
fn is_same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    fn identity(p: &Path) -> Option<PathBuf> {
        if let Ok(real) = std::fs::canonicalize(p) {
            return Some(real);
        }
        let name = p.file_name()?;
        Some(std::fs::canonicalize(p.parent()?).ok()?.join(name))
    }
    if let (Some(x), Some(y)) = (identity(a), identity(b)) {
        if x == y {
            return true;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(x), Ok(y)) = (std::fs::metadata(a), std::fs::metadata(b)) {
            return x.dev() == y.dev() && x.ino() == y.ino();
        }
    }
    false
}

fn internal(why: &str) -> OpError {
    OpError::new(ErrorCode::Internal, why, "An internal error occurred")
}

impl Opened {
    fn from_project(path: &Path, project: Project, target: SaveTarget) -> Opened {
        let sets = project
            .sets()
            .iter()
            .map(|set| {
                let issues = set.document.core_issues();
                let state = if issues.is_empty() {
                    State::Unloaded
                } else {
                    State::ReadOnly(unsupported_text(&issues))
                };
                Slot {
                    id: set.id.clone(),
                    name: set.name.clone(),
                    state,
                }
            })
            .collect();
        Opened {
            path: path.to_path_buf(),
            stem: path.file_stem().map_or_else(
                || "Texture".to_owned(),
                |s| s.to_string_lossy().into_owned(),
            ),
            target,
            current: project.current_set().to_owned(),
            project,
            sets,
            images: None,
        }
    }

    /// セットを ID か名前で引く（省略は今のセット）。
    fn resolve_set(&self, set: Option<&str>) -> Result<usize, OpError> {
        let sets: Vec<(&str, &str)> = self
            .sets
            .iter()
            .map(|s| (s.id.as_str(), s.name.as_str()))
            .collect();
        crate::refs::resolve_set(&sets, set, &self.current)
    }

    /// セットを core の文書にする（まだなら）。予算を超える・読めないときは断り、状態は変えない。
    fn ensure_loaded(&mut self, index: usize, budget: u64) -> Result<(), OpError> {
        if !matches!(self.sets[index].state, State::Unloaded) {
            return Ok(());
        }
        let id = self.sets[index].id.clone();
        let set = self
            .project
            .sets()
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| internal("セットがファイルにありません"))?;
        // 途中の panic は、断りの理由にする（画面なしでも、読む途中で止めない）
        let loaded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            set.document.to_core_within(Some(budget))
        }))
        .map_err(|_| {
            OpError::new(
                ErrorCode::Internal,
                "プロジェクトを読む途中で止まりました",
                "Reading the document stopped unexpectedly",
            )
        })?;
        let mut doc = loaded.map_err(|e| OpError::from_io(&e))?;
        self.give_images(&mut doc);
        let saved = (doc.id(), doc.revision());
        self.sets[index].state = State::Loaded {
            doc: Box::new(doc),
            saved,
        };
        Ok(())
    }

    /// 塗りつぶしの画像と画像の段が指すプロジェクトの画像を、効果の入力として文書へ渡す。モデルはここには無いので、位置を読む効果は入力のまま通す。
    fn give_images(&mut self, doc: &mut Document) {
        let wanted: HashSet<ImageId> = doc.layers().iter().flat_map(|l| l.image_ids()).collect();
        if wanted.is_empty() {
            return;
        }
        let images = self
            .images
            .get_or_insert_with(|| self.project.image_inputs().unwrap_or_default());
        let mut inputs = EffectInputs::new().with_frame(None);
        for (id, image) in images.iter() {
            if wanted.contains(id) {
                inputs = inputs.with_image(*id, image.clone());
            }
        }
        // 渡せなくても読む・編集はできる（その効果が入力のまま通ると、知らせに出る）
        let _ = doc.set_effect_inputs(inputs);
    }
}

/// セットを書いた文書の版が、0.4.x までの Unity 版の開ける版より新しいときの知らせ（スタンドアロンだけの効果・調整を使うセットは新しい版で
/// 保存される。その Unity 版は理由を言って開くのを断り、中身は消えない）。起動中のアプリのホストも、保存の返事に同じ文を使う。
pub fn newer_version_note(set_name: &str, version: i32) -> Option<Text> {
    (version > yolu_io::UNITY_NATIVE_VERSION).then(|| {
        Text::new(
            format!("テクスチャセット「{set_name}」はスタンドアロン版だけの機能を使うため、0.4.x までの Unity 版が開けない版（{version}）で保存しました（その Unity 版は理由を言って開くのを断ります。中身は消えません）"),
            format!("Texture set \"{set_name}\" uses features only the standalone application has, so it was saved in document version {version}, which the Unity version up to 0.4.x cannot open (it refuses the file with a reason; nothing is lost)"),
        )
    })
}

fn unsupported_text(issues: &[String]) -> Text {
    let shown = issues
        .iter()
        .take(3)
        .cloned()
        .collect::<Vec<_>>()
        .join("、");
    let more = if issues.len() > 3 {
        format!(" ほか {} 件", issues.len() - 3)
    } else {
        String::new()
    };
    Text::new(
        format!("編集できない中身があります: {shown}{more}"),
        format!(
            "It holds content that cannot be edited yet ({} item(s))",
            issues.len()
        ),
    )
}

fn note_text(note: &yolu_io::Note) -> Text {
    use yolu_io::Note as N;
    let en = match note {
        N::ViewSlotUnreadable(_) => {
            "The material slot in view.json could not be read; slot 0 is used".to_owned()
        }
        N::Migrated { format } => format!("Format {format} was migrated to format 7 in memory"),
        N::MaterialRefsMigrated { format } => format!(
            "The material references of format {format} were migrated to format 7 in memory"
        ),
        N::UnknownEntryKept(name) => format!("An unknown entry is kept as it is: {name}"),
        N::SetNotConvertible { set, .. } => {
            format!("Texture set \"{set}\" cannot be converted for editing")
        }
        N::SmartResourceKept(name) => format!("The smart resource \"{name}\" is kept as a file"),
        N::BrushKept => "Brush settings and images are kept as they are".to_owned(),
    };
    Text::new(note.to_string(), en)
}

fn summary(project: &Project, slot: &Slot) -> SetSummary {
    let (width, height, layers) =
        project
            .sets()
            .iter()
            .find(|s| s.id == slot.id)
            .map_or((0, 0, 0), |s| {
                (
                    s.document.width(),
                    s.document.height(),
                    s.document.layer_count(),
                )
            });
    SetSummary {
        id: slot.id.clone(),
        name: slot.name.clone(),
        width: width.max(0) as u32,
        height: height.max(0) as u32,
        layer_count: layers as u32,
        state: if matches!(slot.state, State::ReadOnly(_)) {
            SetState::ReadOnly
        } else {
            SetState::Editable
        },
        reason: match &slot.state {
            State::ReadOnly(reason) => Some(reason.clone()),
            _ => None,
        },
        unsaved: slot.unsaved(),
    }
}

fn doc_info_of(o: &Opened) -> DocInfo {
    let info = o.project.info();
    DocInfo {
        path: o.path.display().to_string(),
        format: info.format,
        saved_by: info.saved_by.as_ref().map(|w| crate::reply::WriterInfo {
            app: w.app.clone(),
            version: w.version.clone(),
            platform: w.unity.clone(),
        }),
        sets: o.sets.iter().map(|s| summary(&o.project, s)).collect(),
        current_set: o.current.clone(),
        unsaved: o.sets.iter().any(Slot::unsaved),
        notes: o.project.notes().iter().map(note_text).collect(),
    }
}

fn confirm_unsaved(sets: &[String]) -> OpError {
    OpError::confirm_required(
        "保存していない変更があります。開き直すと失います。confirm: true を付けてください",
        "There are unsaved changes that opening another document would discard; pass confirm: true",
        Some(json!({"reason": "unsaved_changes", "sets": sets})),
    )
}

impl OpHost for FileHost {
    fn policy(&self) -> &PathPolicy {
        &self.policy
    }

    /// .ylp には選んでいたレイヤーが入っていないので、`$selected` はいつも断る（理由を言う）。
    fn selected_layer(&mut self, set: Option<&str>) -> Result<String, OpError> {
        self.opened_mut()?.resolve_set(set)?;
        Err(crate::refs::no_selection(Some((
            ".ylp には選んでいたレイヤーが入っていません。名前か ID で指してください",
            "a .ylp file does not store the selected layer; use a name or an id",
        ))))
    }

    fn doc_info(&mut self) -> Result<DocInfo, OpError> {
        Ok(doc_info_of(self.opened_mut()?))
    }

    fn open(&mut self, path: &Path, confirm: bool) -> Result<DocInfo, OpError> {
        if let Some(o) = &self.open {
            let unsaved: Vec<String> = o
                .sets
                .iter()
                .filter(|s| s.unsaved())
                .map(|s| s.name.clone())
                .collect();
            if !unsaved.is_empty() && !confirm {
                return Err(confirm_unsaved(&unsaved));
            }
        }
        // 絶対パスで覚える（あとの保存先との同じファイルの判定が、書き方に依らないように）
        let path = std::path::absolute(path).map_err(|e| OpError::from_io(&e.into()))?;
        let (project, target) = SaveTarget::open_within(&path, &self.config.limits)
            .map_err(|e| OpError::from_io(&e))?;
        let opened = Opened::from_project(&path, project, target);
        let info = doc_info_of(&opened);
        self.open = Some(opened);
        Ok(info)
    }

    fn read_set(
        &mut self,
        set: Option<&str>,
        f: &mut dyn FnMut(&SetView<'_>) -> Result<Reply, OpError>,
    ) -> Result<Reply, OpError> {
        let budget = self.config.source_budget;
        let o = self.opened_mut()?;
        let index = o.resolve_set(set)?;
        o.ensure_loaded(index, budget)?;
        let slot = &o.sets[index];
        let (width, height, layers) =
            o.project
                .sets()
                .iter()
                .find(|s| s.id == slot.id)
                .map_or((0, 0, 0), |s| {
                    (
                        s.document.width(),
                        s.document.height(),
                        s.document.layer_count(),
                    )
                });
        let (doc, read_only) = match &slot.state {
            State::Loaded { doc, .. } => (Some(&**doc), None),
            State::ReadOnly(reason) => (None, Some(reason)),
            State::Unloaded => return Err(internal("セットを読み込めていません")),
        };
        let view = SetView {
            id: &slot.id,
            name: &slot.name,
            doc,
            read_only,
            size: (width.max(0) as u32, height.max(0) as u32),
            layer_count: layers as u32,
            unsaved: slot.unsaved(),
            stem: &o.stem,
            set_count: o.sets.len(),
            source_budget: budget,
        };
        f(&view)
    }

    fn write_set(
        &mut self,
        set: Option<&str>,
        f: &mut dyn FnMut(SetFacts<'_>, &mut Document) -> Result<Reply, OpError>,
    ) -> Result<Reply, OpError> {
        let budget = self.config.source_budget;
        let o = self.opened_mut()?;
        let index = o.resolve_set(set)?;
        if let State::ReadOnly(reason) = &o.sets[index].state {
            return Err(read_only_error(&o.sets[index].name, Some(reason)));
        }
        o.ensure_loaded(index, budget)?;
        let slot = &mut o.sets[index];
        let facts = SetFacts {
            id: &slot.id,
            name: &slot.name,
            unsaved: true,
        };
        match &mut slot.state {
            State::Loaded { doc, .. } => f(facts, doc),
            _ => Err(internal("セットを読み込めていません")),
        }
    }

    fn save(&mut self, job: &SaveJob) -> Result<Reply, OpError> {
        let keep = self.config.backups;
        let limits = self.config.limits;
        let o = self.opened_mut()?;
        let (destination, confirm) = match job {
            SaveJob::InPlace { confirm } => (None, *confirm),
            SaveJob::As { path, confirm } => (Some(path.as_path()), *confirm),
        };
        let requested = destination.unwrap_or(&o.path).to_path_buf();
        let is_ylp = requested
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("ylp"));
        if !is_ylp {
            return Err(OpError::ylp_name_required(&requested.display().to_string()));
        }
        // 開いたファイルと同じ物かは、書き方（`..`・大文字小文字・リンク）に依らず見る。同じなら開いたときの場所と印で書く
        let same_file = destination.is_none_or(|p| is_same_file(p, &o.path));
        let target_path = if same_file { o.path.clone() } else { requested };
        let exists = std::fs::symlink_metadata(&target_path).is_ok();
        if exists && !confirm {
            return Err(OpError::replace_confirm_required(&[target_path
                .display()
                .to_string()]));
        }
        let dirty: Vec<usize> = (0..o.sets.len()).filter(|i| o.sets[*i].unsaved()).collect();
        if same_file && dirty.is_empty() {
            return Ok(Reply::Saved(Saved {
                path: o.path.display().to_string(),
                written: false,
                format: o.project.info().format,
                upgraded_from: None,
                sets_written: Vec::new(),
                backup: None,
                notes: Vec::new(),
            }));
        }
        // 保存先の印: 同じファイルなら、開いたときの印（外からの書き換えを見つける）。別のファイルなら、もうあるものを開いた印（有効な .ylp だけ置き換える）
        let mut target = if same_file {
            o.target.clone()
        } else if exists {
            SaveTarget::open_within(&target_path, &limits)
                .map(|(_, t)| t)
                .map_err(|e| OpError::from_io(&e))?
        } else {
            SaveTarget::create(&target_path).map_err(|e| OpError::from_io(&e))?
        };
        // 組み立て（ディスクに触る前に、全部の検査を通す）
        let writer = writer();
        let upgraded_from = (o.project.info().format < 7).then_some(o.project.info().format);
        let upgraded;
        let base: &Project = match upgraded_from {
            Some(_) => {
                upgraded = o
                    .project
                    .upgraded(writer.clone())
                    .map_err(|e| OpError::from_io(&e))?;
                &upgraded
            }
            None => &o.project,
        };
        let mut specs = Vec::with_capacity(o.sets.len());
        let mut notes = Vec::new();
        let mut written_ids = Vec::new();
        for (i, slot) in o.sets.iter().enumerate() {
            let material = base
                .sets()
                .iter()
                .find(|s| s.id == slot.id)
                .map(|s| s.material.clone())
                .ok_or_else(|| internal("セットがファイルにありません"))?;
            let (document, composites) = match (&slot.state, dirty.contains(&i)) {
                (State::Loaded { doc, .. }, true) => {
                    let snapshot =
                        Arc::new(doc.capture_snapshot().map_err(|e| OpError::from_core(&e))?);
                    let source = DocumentSource::from_core(snapshot.clone())
                        .map_err(|e| OpError::from_io(&e))?;
                    let pngs = composite_pngs(&snapshot).map_err(|e| OpError::from_io(&e))?;
                    let inactive = inactive_texts(doc);
                    if !inactive.is_empty() {
                        notes.push(Text::new(
                            format!("テクスチャセット「{}」の効いていない効果は、保存した合成の PNG に入っていません（設定は残っています）", slot.name),
                            format!("Inactive effects of texture set \"{}\" are not in the saved composite PNG (their settings are kept)", slot.name),
                        ));
                        notes.extend(inactive);
                    }
                    written_ids.push(slot.id.clone());
                    (Some(source), pngs)
                }
                _ => (None, Vec::new()),
            };
            specs.push(SetSpec {
                id: slot.id.clone(),
                name: slot.name.clone(),
                material,
                document,
                composites,
            });
        }
        let project = base
            .with_sets(writer, &specs, &o.current)
            .map_err(|e| OpError::from_io(&e))?;
        let report = target
            .save_with(&project, keep)
            .map_err(|e| OpError::from_io(&e))?;
        // 確定した: 開いているプロジェクトを書いたファイルへ向け、編集したセットは保存済みにする
        o.project = report.project.unwrap_or(project);
        o.target = target;
        o.stem = target_path
            .file_stem()
            .map_or_else(|| o.stem.clone(), |s| s.to_string_lossy().into_owned());
        o.path = target_path;
        for slot in &mut o.sets {
            if let State::Loaded { doc, saved } = &mut slot.state {
                *saved = (doc.id(), doc.revision());
            }
        }
        // 0.4.x までの Unity 版が開けない版で書いたセットを知らせる（スタンドアロンだけの効果・調整を使うと、新しい版で保存される）
        for id in &written_ids {
            let version = o
                .project
                .sets()
                .iter()
                .find(|s| s.id == *id)
                .map_or(0, |s| s.document.version());
            let name = o
                .sets
                .iter()
                .find(|s| s.id == *id)
                .map_or("", |s| s.name.as_str());
            notes.extend(newer_version_note(name, version));
        }
        Ok(Reply::Saved(Saved {
            path: o.path.display().to_string(),
            written: true,
            format: o.project.info().format,
            upgraded_from,
            sets_written: written_ids,
            backup: report.backup.map(|b| b.display().to_string()),
            notes,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_io::MaterialRef;

    fn two_set_project(dir: &Path) -> Opened {
        let spec = |id: &str, name: &str| {
            let doc = Document::new(8, 8).unwrap();
            SetSpec {
                id: id.into(),
                name: name.into(),
                material: MaterialRef::Material {
                    name: name.into(),
                    asset: None,
                },
                composites: composite_pngs(&doc).unwrap(),
                document: Some(DocumentSource::from_core(Arc::new(doc)).unwrap()),
            }
        };
        let (a, b) = (
            "11111111-1111-4111-8111-111111111111",
            "22222222-2222-4222-8222-222222222222",
        );
        let project = Project::create(writer(), &[spec(a, "First"), spec(b, "Second")], a).unwrap();
        let path = dir.join("x.ylp");
        let target = SaveTarget::create(&path).unwrap();
        Opened::from_project(&path, project, target)
    }

    /// 保存の返事の知らせは、.ylp を開く Unity のパッケージ（0.4.x まで。0.5.0 からは .ylp を開かない）が開けない版のセットだけに付き、
    /// 日英とも読み手を「0.4.x までの Unity 版」と言う（古い 0.2.0 や「Rust 版」とは言わない）。
    #[test]
    fn the_newer_version_note_names_the_unity_versions_that_cannot_open_the_file() {
        assert!(newer_version_note("Body", yolu_io::UNITY_NATIVE_VERSION).is_none());
        let note = newer_version_note("Body", yolu_io::UNITY_NATIVE_VERSION + 1).unwrap();
        for text in [&note.ja, &note.en] {
            assert!(text.contains("Body") && text.contains("0.4.x"), "{text}");
            assert!(
                !text.contains("0.2.0") && !text.contains("Rust") && !text.contains("this editor"),
                "{text}"
            );
        }
        assert!(note.ja.contains("Unity 版") && note.en.contains("Unity version"));
    }

    /// yolu-io は名前の重なるセットのファイルを開かせないので、名前で引いて重なることは今は無い。それでも、重なったら候補を
    /// レイヤー・チャンネルと同じ鍵（`candidates`）で返す。
    #[test]
    fn ambiguous_set_names_list_their_candidates_under_the_same_key_as_layers() {
        let dir = std::env::temp_dir();
        let mut opened = two_set_project(&dir);
        assert_eq!(opened.resolve_set(Some("Second")).unwrap(), 1);
        opened.sets[1].name = "First".into();
        let e = opened.resolve_set(Some("First")).unwrap_err();
        assert_eq!(e.code, ErrorCode::Ambiguous);
        let data = e.data.unwrap();
        assert!(data.get("sets").is_none());
        assert_eq!(
            data["candidates"],
            json!([
                {"id": "11111111-1111-4111-8111-111111111111", "name": "First"},
                {"id": "22222222-2222-4222-8222-222222222222", "name": "First"},
            ])
        );
        // ID なら引ける
        assert_eq!(
            opened
                .resolve_set(Some("22222222-2222-4222-8222-222222222222"))
                .unwrap(),
            1
        );
    }

    #[test]
    fn the_same_file_is_recognized_however_the_path_is_written() {
        let dir = std::env::temp_dir().join(format!("yolu-ops-samefile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let file = dir.join("a.ylp");
        std::fs::write(&file, b"x").unwrap();
        std::fs::write(dir.join("b.ylp"), b"x").unwrap();
        assert!(is_same_file(&file, &file));
        assert!(is_same_file(
            &dir.join("sub").join("..").join("a.ylp"),
            &file
        ));
        assert!(is_same_file(&file, &dir.join(".").join("a.ylp")));
        assert!(
            !is_same_file(&dir.join("b.ylp"), &file),
            "中身が同じでも別のファイル"
        );
        assert!(!is_same_file(&dir.join("c.ylp"), &file), "まだ無いファイル");
        // まだ無いファイルどうしも、上のフォルダの書き方の違いは畳む
        assert!(is_same_file(
            &dir.join("sub").join("..").join("gone.ylp"),
            &dir.join("gone.ylp")
        ));
        assert!(!is_same_file(
            &dir.join("sub").join("gone.ylp"),
            &dir.join("gone.ylp")
        ));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&dir, dir.join("dirlink")).unwrap();
            std::os::unix::fs::symlink(&file, dir.join("filelink.ylp")).unwrap();
            std::fs::hard_link(&file, dir.join("hard.ylp")).unwrap();
            assert!(is_same_file(&dir.join("dirlink").join("a.ylp"), &file));
            assert!(is_same_file(&dir.join("filelink.ylp"), &file));
            assert!(is_same_file(&dir.join("hard.ylp"), &file));
        }
        #[cfg(windows)]
        assert!(is_same_file(
            &PathBuf::from(file.to_string_lossy().to_uppercase()),
            &file
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
