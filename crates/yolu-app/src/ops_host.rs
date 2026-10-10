//! 起動中のアプリのホスト: 外からの操作（`yolu_ops` の命令）を、今開いているプロジェクトに当てる（`OpHost` を `AppState` に実装）。
//!
//! - 文書の命令は `yolu_ops::doc_ops` の 1 か所の関数を、セットの文書（今のセットは `AppState::doc`、ほかはしまってある文書）にそのまま当てる。
//!   1 つの命令は `Document::batch` の 1 回なので、画面の取り消しの並びの 1 段になる（画面の取り消し・やり直しでそのまま戻せる）。
//!   当てる前に取り消しのまとめ（`end_coalescing`）を切り、直前のスライダー操作などと 1 段に混ざらないようにする。
//! - 描いている最中（ストロークと移動・変形などのドラッグ）・保存の途中は、編集・取り消し・保存を理由つきで断る。読むだけのセットは断る
//!   （理由は画面に出している文）。読む命令は、保存の途中でも答える。見本と書き出しは合成を使うので、描いている最中は断る。
//! - `doc.open` は、起動中のアプリでは文書を開き替えない（今開いている同じファイルなら、今の文書を返す）。
//! - 保存は画面の保存と同じ裏の仕組み（`project::save_for_ops`）で始め、返事は保存が終わってから（`mcp_server` が結果を受けて返す）。`OpHost::save` は
//!   同期の形なので、始めたことを `AppHost::take_started_save` に残して、返事を持たない印の誤りを返す。呼び手はその印を見て、返事を待たせる。
//! - 見本・書き出しは、画面なしのホストと同じ関数を、セットの文書（画面で効果の入力を渡している文書）に当てる。塗り広げ（UV の外）と焼いた AO は、
//!   画面なしと同じく使わない。
//! - ファイルの道: 相対パスは、開いているプロジェクトのフォルダ（まだファイルが無ければアプリの今のフォルダ）からで、`..` で外へ出るものは断る。
//!   呼び手は絶対パスで渡すのがよい。

use std::path::{Path, PathBuf};

use serde_json::json;
use yolu_core::Document;
use yolu_ops::command::PreviewArgs;
use yolu_ops::doc_ops::{inactive_texts, SetFacts};
use yolu_ops::error::{ErrorCode, OpError};
use yolu_ops::host::{read_only_error, ExportJob, OpHost, SaveJob, SetView};
use yolu_ops::reply::{DocInfo, Reply, Saved, SetState, SetSummary, WriterInfo};
use yolu_ops::{PathPolicy, Text};

use crate::state::AppState;

/// 裏で始めた保存（返事は、保存が終わってから）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartedSave {
    /// 書く先。
    pub path: PathBuf,
    /// 開いていたファイルが、保存で上げた前の形式の番号（上げなかったら無い）。
    pub upgraded_from: Option<i32>,
    /// もうあったファイルを置き換える保存か。
    pub replaced: bool,
}

/// 開いているプロジェクトに命令を当てるホスト（1 回の命令ごとに作る）。
pub struct AppHost<'a> {
    state: &'a mut AppState,
    policy: PathPolicy,
    started: Option<StartedSave>,
}

/// 保存を始めたときに `OpHost::save` が返す誤りの印（返事は保存が終わってから。呼び手は `take_started_save` を先に見る）。
fn deferred() -> OpError {
    OpError::new(
        ErrorCode::Internal,
        "保存を始めました（返事は保存が終わってから）",
        "The save was started (the reply follows when it finishes)",
    )
}

fn busy(ja: &str, en: &str) -> OpError {
    OpError::new(ErrorCode::Busy, ja, en)
}

/// 同じ読むだけの理由を、両方の言語の文にする（画面に出している文が 1 つの言語なので、同じ文を両方に入れる）。
fn reason_text(reason: &str) -> Text {
    Text::new(reason, reason)
}

impl<'a> AppHost<'a> {
    pub fn new(state: &'a mut AppState) -> AppHost<'a> {
        // 離した後の残りを塗っている 3D のストローク（確定待ち）は、外からの操作を受ける前に確定する（「描いている最中」で断らない）
        crate::view3d::input::settle(state);
        let base = state
            .project
            .as_ref()
            .filter(|p| p.is_file())
            .and_then(|p| p.path().parent().map(Path::to_path_buf))
            .filter(|d| !d.as_os_str().is_empty())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(std::env::temp_dir);
        let policy = PathPolicy::new(&base)
            .or_else(|_| PathPolicy::new(std::env::temp_dir()))
            .expect("一時フォルダは絶対パスにできる");
        AppHost {
            state,
            policy,
            started: None,
        }
    }

    /// 裏で始めた保存（あれば。取ると空になる）。
    pub fn take_started_save(&mut self) -> Option<StartedSave> {
        self.started.take()
    }

    /// 描いている最中か、保存の途中なら、断る理由。
    fn busy(&self) -> Option<OpError> {
        if self.state.is_stroking() {
            return Some(busy("描いている最中です", "A stroke is in progress"));
        }
        if self.state.is_saving() {
            return Some(busy("保存の途中です", "A save is in progress"));
        }
        None
    }

    fn drawing(&self) -> Option<OpError> {
        self.state
            .is_stroking()
            .then(|| busy("描いている最中です", "A stroke is in progress"))
    }

    /// 開いている .ylp のファイル（まだファイルが無い文書は None）。
    fn open_file(&self) -> Option<PathBuf> {
        self.state
            .project
            .as_ref()
            .filter(|p| p.is_file())
            .map(|p| p.path().to_path_buf())
    }

    fn resolve(&self, set: Option<&str>) -> Result<usize, OpError> {
        let list: Vec<(&str, &str)> = self
            .state
            .sets
            .iter()
            .map(|s| (s.id.as_str(), s.name.as_str()))
            .collect();
        yolu_ops::refs::resolve_set(&list, set, &self.state.sets.current().id)
    }

    /// セットが、開いた・保存したあとに編集されたか（まだファイルに無いセットは、いつも編集済み）。
    fn unsaved(&self, index: usize) -> bool {
        let doc = self.state.set_doc(index);
        self.state
            .sets
            .get(index)
            .is_some_and(|s| s.saved != Some((doc.id(), doc.revision())))
    }

    /// セットの大きさとレイヤーの数。読むだけのセットは、画面に出している見せるだけの文書でなく、ファイルの正本の値。
    fn size_of(&self, index: usize) -> (u32, u32, u32) {
        let doc = self.state.set_doc(index);
        let set = &self.state.sets.get(index).expect("範囲内");
        if set.read_only.is_some() {
            let native = self
                .state
                .project
                .as_ref()
                .and_then(|p| p.project().sets().iter().find(|s| s.id == set.id));
            if let Some(native) = native {
                return (
                    native.document.width().max(0) as u32,
                    native.document.height().max(0) as u32,
                    native.document.layer_count() as u32,
                );
            }
        }
        (doc.width(), doc.height(), doc.layers().len() as u32)
    }

    fn stem(&self) -> String {
        self.open_file()
            .and_then(|f| f.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| self.state.project_name.clone())
    }

    fn summary(&self, index: usize) -> SetSummary {
        let set = self.state.sets.get(index).expect("範囲内");
        let (width, height, layers) = self.size_of(index);
        let reason = set.read_only.as_deref().map(reason_text);
        SetSummary {
            id: set.id.clone(),
            name: set.name.clone(),
            width,
            height,
            layer_count: layers,
            state: if reason.is_some() {
                SetState::ReadOnly
            } else {
                SetState::Editable
            },
            reason,
            unsaved: self.unsaved(index),
        }
    }

    /// 保存の返事（保存が終わった後の状態から）。
    pub fn saved_reply(
        state: &AppState,
        started: &StartedSave,
        facts: crate::project::SavedFacts,
    ) -> Reply {
        let project = state.project.as_ref();
        let mut notes = Vec::new();
        for id in &facts.sets_written {
            let Some(index) = state.sets.iter().position(|s| s.id == *id) else {
                continue;
            };
            let name = state.sets.get(index).map_or("", |s| s.name.as_str());
            let inactive = inactive_texts(state.set_doc(index));
            if !inactive.is_empty() {
                notes.push(Text::new(
                    format!("テクスチャセット「{name}」の効いていない効果は、保存した合成の PNG に入っていません（設定は残っています）"),
                    format!("Inactive effects of texture set \"{name}\" are not in the saved composite PNG (their settings are kept)"),
                ));
                notes.extend(inactive);
            }
            let version = project
                .and_then(|p| p.project().sets().iter().find(|s| s.id == *id))
                .map_or(0, |s| s.document.version());
            notes.extend(yolu_ops::newer_version_note(name, version));
        }
        // 読めないため保存に入れなかったセット（保存したことが無いセット。ファイルには入っていない）
        for name in &facts.left_out {
            notes.push(Text::new(
                format!("テクスチャセット「{name}」は読めないため、保存に入れていません"),
                format!("Texture set \"{name}\" could not be read and was left out of the save"),
            ));
        }
        Reply::Saved(Saved {
            path: started.path.display().to_string(),
            written: true,
            format: project.map_or(7, |p| p.format()),
            upgraded_from: started.upgraded_from,
            sets_written: facts.sets_written,
            backup: facts.backup.map(|b| b.display().to_string()),
            notes,
        })
    }
}

impl OpHost for AppHost<'_> {
    /// 同梱のフォント（画面の書体と同じファイル）。起動中のアプリだけが同梱のフォントの文字を描ける。
    fn bundled_font(&self, name: &str) -> Option<std::sync::Arc<[u8]>> {
        crate::textlayer::bundled_font(name)
    }
    /// OS のフォントの一覧は、アプリが別のスレッドでなめたもの（`AppState::text_fonts_wanted`）。まだできていなければ、なめ始めて `Busy` で断る
    /// （画面のスレッドでフォルダ全体をなめて止めない。呼び手は少し待って同じ命令をもう一度送る）。
    fn system_fonts(&mut self) -> Result<std::sync::Arc<yolu_io::fonts::SystemFonts>, OpError> {
        self.state.text_fonts_wanted();
        self.state
            .text
            .fonts
            .list
            .clone()
            .ok_or_else(|| busy("フォントを探しています", "Searching for the fonts"))
    }
    fn policy(&self) -> &PathPolicy {
        &self.policy
    }

    /// `$selected`: 今のテクスチャセットで選んでいるレイヤー（ほかのセットには選んでいるレイヤーが無い）。
    fn selected_layer(&mut self, set: Option<&str>) -> Result<String, OpError> {
        let index = self.resolve(set)?;
        if index != self.state.sets.current_index() {
            return Err(yolu_ops::refs::no_selection(Some((
                "選んでいるレイヤーは今のテクスチャセットにだけあります",
                "only the current texture set has a selected layer",
            ))));
        }
        self.state
            .selected_layer
            .filter(|id| self.state.doc.layer(*id).is_some())
            .map(|id| id.to_string())
            .ok_or_else(|| yolu_ops::refs::no_selection(None))
    }

    fn doc_info(&mut self) -> Result<DocInfo, OpError> {
        let project = self.state.project.as_ref().filter(|p| p.is_file());
        let info = project.map(|p| p.project().info());
        let sets: Vec<SetSummary> = (0..self.state.sets.len())
            .map(|i| self.summary(i))
            .collect();
        Ok(DocInfo {
            path: project
                .map(|p| p.path().display().to_string())
                .unwrap_or_default(),
            // まだファイルが無い文書は、保存で書く形式
            format: project.map_or(7, |p| p.format()),
            saved_by: info.and_then(|i| i.saved_by.as_ref()).map(|w| WriterInfo {
                app: w.app.clone(),
                version: w.version.clone(),
                platform: w.unity.clone(),
            }),
            unsaved: self.state.shows_modified()
                || self.state.shelf.changed
                || sets.iter().any(|s| s.unsaved),
            sets,
            current_set: self.state.sets.current().id.clone(),
            notes: Vec::new(),
        })
    }

    fn open(&mut self, path: &Path, _confirm: bool) -> Result<DocInfo, OpError> {
        match self.open_file() {
            Some(file) if crate::project::same_file(&file, path) => self.doc_info(),
            _ => Err(OpError::new(
                ErrorCode::Unsupported,
                "起動中のアプリはプロジェクトを開き替えません（アプリで開いているプロジェクトを使います）",
                "The running app does not switch projects; it works on the project it has open",
            )
            .with_data(json!({"path": path.display().to_string()}))),
        }
    }

    fn read_set(
        &mut self,
        set: Option<&str>,
        f: &mut dyn FnMut(&SetView<'_>) -> Result<Reply, OpError>,
    ) -> Result<Reply, OpError> {
        let index = self.resolve(set)?;
        let (width, height, layers) = self.size_of(index);
        let stem = self.stem();
        let unsaved = self.unsaved(index);
        let source_budget = self.state.load_source_bytes();
        let set_count = self.state.sets.len();
        let ts = self.state.sets.get(index).expect("範囲内");
        let read_only = ts.read_only.as_deref().map(reason_text);
        let view = SetView {
            id: &ts.id,
            name: &ts.name,
            doc: read_only.is_none().then(|| self.state.set_doc(index)),
            read_only: read_only.as_ref(),
            size: (width, height),
            layer_count: layers,
            unsaved,
            stem: &stem,
            set_count,
            source_budget,
        };
        f(&view)
    }

    fn write_set(
        &mut self,
        set: Option<&str>,
        f: &mut dyn FnMut(SetFacts<'_>, &mut Document) -> Result<Reply, OpError>,
    ) -> Result<Reply, OpError> {
        let index = self.resolve(set)?;
        if let Some(e) = self.busy() {
            return Err(e);
        }
        let (id, name, read_only) = {
            let ts = self.state.sets.get(index).expect("範囲内");
            (ts.id.clone(), ts.name.clone(), ts.read_only.clone())
        };
        if let Some(reason) = read_only {
            return Err(read_only_error(&name, Some(&reason_text(&reason))));
        }
        let state = &mut *self.state;
        let current = index == state.sets.current_index();
        let doc = state.set_doc_mut(index);
        // 直前の画面の操作（スライダーなど）の段に混ざらないよう、取り消しのまとめを切る
        doc.end_coalescing();
        let before = (doc.id(), doc.revision());
        let result = f(
            SetFacts {
                id: &id,
                name: &name,
                unsaved: true,
            },
            doc,
        );
        if (doc.id(), doc.revision()) != before {
            state.modified = true;
            if current {
                // 消したレイヤーを選んだままにしない（画面の取り消しと同じ後始末）
                state.ensure_selection();
            }
        }
        result
    }

    fn save(&mut self, job: &SaveJob) -> Result<Reply, OpError> {
        if let Some(e) = self.busy() {
            return Err(e);
        }
        if self.state.distribute.is_busy() || self.state.distribute.is_open() {
            return Err(busy(
                "配布用に保存の途中です",
                "Saving for distribution is in progress",
            ));
        }
        let open_file = self.open_file();
        let (destination, confirm) = match job {
            SaveJob::InPlace { confirm } => (None, *confirm),
            SaveJob::As { path, confirm } => (Some(path.clone()), *confirm),
        };
        let target = match (destination, &open_file) {
            (Some(path), _) => path,
            (None, Some(file)) => file.clone(),
            (None, None) => return Err(OpError::no_file_to_save()),
        };
        if !target
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("ylp"))
        {
            return Err(OpError::ylp_name_required(&target.display().to_string()));
        }
        let same_file = open_file
            .as_ref()
            .is_some_and(|f| crate::project::same_file(f, &target));
        let exists = std::fs::symlink_metadata(&target).is_ok();
        if exists && !confirm {
            return Err(OpError::replace_confirm_required(&[target
                .display()
                .to_string()]));
        }
        let upgraded_from = self
            .state
            .project
            .as_ref()
            .map(|p| p.format())
            .filter(|f| *f < 7);
        // 何も編集していなければ書かない（開いているファイルへの上書きだけ）
        if same_file && !self.state.shows_modified() && !self.state.shelf.changed {
            return Ok(Reply::Saved(Saved {
                path: target.display().to_string(),
                written: false,
                format: self.state.project.as_ref().map_or(7, |p| p.format()),
                upgraded_from: None,
                sets_written: Vec::new(),
                backup: None,
                notes: Vec::new(),
            }));
        }
        if let Err(text) = crate::project::save_for_ops(self.state, &target) {
            return Err(OpError::new(ErrorCode::Refused, text.clone(), text)
                .with_data(json!({"path": target.display().to_string()})));
        }
        self.started = Some(StartedSave {
            path: target,
            upgraded_from,
            replaced: exists,
        });
        Err(deferred())
    }

    fn export(&mut self, job: &ExportJob<'_>) -> Result<Reply, OpError> {
        if let Some(e) = self.drawing() {
            return Err(e);
        }
        let policy = self.policy.clone();
        self.read_set(job.set(), &mut |view| {
            yolu_ops::export::run(view, &policy, job)
        })
    }

    fn preview(&mut self, args: &PreviewArgs) -> Result<Reply, OpError> {
        if let Some(e) = self.drawing() {
            return Err(e);
        }
        self.read_set(args.set.as_deref(), &mut |view| {
            yolu_ops::preview::render(view, args)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::Worker;
    use serde_json::json;
    use std::sync::Arc;
    use yolu_core::text::TextFont;
    use yolu_io::fonts::SystemFonts;

    /// OS のフォントを名前で選ぶ命令は、アプリが別のスレッドでなめた一覧を使う。なめている間は、画面のスレッドでフォルダ全体をなめずに
    /// `Busy` で断り（文書は変えない）、できたら当たる。
    #[test]
    fn installed_fonts_come_from_the_apps_list_and_a_search_in_progress_is_refused_for_now() {
        let dir = std::env::temp_dir().join(format!("yolu-ops-host-fonts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts");
        std::fs::copy(source.join("BIZUDPGothic-Bold.ttf"), dir.join("b.ttf")).unwrap();
        let list = Arc::new(SystemFonts::from_dirs(&[&dir]));
        let add = yolu_ops::parse_command(&json!({"command": "layer.add", "args": {
            "kind": "text", "name": "Label",
            "text": {"content": "x", "font": "BIZUDPGothic-Bold"}}}))
        .unwrap();
        let mut state = AppState::new(64, 64);
        let (searching, _hold) = Worker::parked();
        state.text.fonts.worker = Some(searching);
        let layers = state.doc.layers().len();
        let undo = state.doc.undo_count();
        let refused = yolu_ops::execute(&mut AppHost::new(&mut state), &add).unwrap_err();
        assert_eq!(refused.code, ErrorCode::Busy);
        assert!(
            refused.message.ja.contains("フォントを探しています"),
            "{}",
            refused.message.ja
        );
        assert!(
            refused.message.en.contains("Searching"),
            "{}",
            refused.message.en
        );
        assert_eq!(state.doc.layers().len(), layers, "断った命令は何も変えない");
        assert_eq!(state.doc.undo_count(), undo);
        // 一覧ができた: その一覧（OS の一覧ではなく、アプリが持っている物）から選ぶ
        state.text.fonts.worker = None;
        state.text.fonts.list = Some(list);
        yolu_ops::execute(&mut AppHost::new(&mut state), &add).expect("一覧から選べる");
        let layer = state
            .doc
            .layers()
            .iter()
            .find(|l| l.name() == "Label")
            .expect("追加したレイヤー");
        let Some(TextFont::File { path, .. }) = layer.text().map(|t| t.font.clone()) else {
            panic!("ファイルのフォント")
        };
        assert!(Path::new(&path).starts_with(&dir), "{path}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
