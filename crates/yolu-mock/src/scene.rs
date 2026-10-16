//! ダミーの状態（`AppState`）と、走らせるあいだだけあるライブラリの置き場。

use std::path::{Path, PathBuf};

use yolu_app::engine::LayerId;
use yolu_app::lang::Lang;
use yolu_app::m2::Edit;
use yolu_app::prefs::{Pref, PrefsAction};
use yolu_app::state::{Action, AppState};

/// ダミーの文書の一辺。
const DOCUMENT_SIZE: u32 = 1024;

/// ダミーの状態。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scene {
    /// `AppState::new_in` のまま。
    Empty,
    /// 空の文書に、ラスターのレイヤー 3 つ・グループ 1 つ・マスク 1 つを足した物。
    Layers,
}

/// ダミーの状態を作る。文書は `Action`（公開の操作）だけで追加する。ライブラリの場所は `library` のフォルダに当てる。
pub fn scene_state(scene: Scene, lang: Lang, library: &Path) -> AppState {
    let mut state = AppState::new_in(DOCUMENT_SIZE, DOCUMENT_SIZE, lang);
    state.apply(Action::Prefs(PrefsAction::Set(Pref::LibraryFolder(Some(
        library.to_path_buf(),
    )))));
    if scene == Scene::Layers {
        // 最初のレイヤーの上に 1 つ足し、そのレイヤーをグループに入れる。グループの上にもう 1 つ足して、マスクを付ける
        state.apply(Action::NewLayer);
        state.apply(Action::M2(Edit::GroupSelected));
        state.apply(Action::NewLayer);
        let top: Option<LayerId> = state.session.selected_layer;
        if let Some(id) = top {
            state.apply(Action::M2(Edit::AddMask(id)));
            // グループにも番号が振られるので、ラスターの名前は 1・2・3 と続けて見せる
            let name = format!("{} 3", lang.pick("レイヤー", "Layer"));
            state.apply(Action::RenameLayer(id, name));
        }
    }
    state
}

/// 走らせるあいだだけある、空のフォルダ（ライブラリの置き場）。消せなくても落ちない。
pub struct TempLibrary(pub PathBuf);

impl TempLibrary {
    pub fn create() -> Result<TempLibrary, String> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir =
            std::env::temp_dir().join(format!("yolu-mock-library-{}-{nanos}", std::process::id()));
        std::fs::create_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(TempLibrary(dir))
    }
}

impl Drop for TempLibrary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
