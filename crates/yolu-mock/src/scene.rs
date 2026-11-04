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
    /// テクスチャセット 2 つ。今のセットに、もう 1 つのセットのレイヤーセットを指すポインターレイヤー（選んでいる）と、グループの中のレイヤー。
    Pointer,
    /// `Pointer` で、ポインターレイヤーの指す先のドロップダウンを開いた形。
    PointerTarget,
    /// `Pointer` で、ラスターのレイヤーの右クリックのメニューを開いた形。
    PointerContext,
    /// `Pointer` で、レイヤーの一覧の空白の右クリック（追加のメニュー）を開いた形。
    PointerAdd,
}

impl Scene {
    pub fn parse(name: &str) -> Option<Scene> {
        Some(match name {
            "empty" => Scene::Empty,
            "layers" => Scene::Layers,
            "pointer" => Scene::Pointer,
            "pointer-target" => Scene::PointerTarget,
            "pointer-context" => Scene::PointerContext,
            "pointer-add" => Scene::PointerAdd,
            _ => return None,
        })
    }

    fn pointer(self) -> bool {
        matches!(
            self,
            Scene::Pointer | Scene::PointerTarget | Scene::PointerContext | Scene::PointerAdd
        )
    }
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
    if scene.pointer() {
        pointer_scene(&mut state, lang);
    }
    state
}

/// もう 1 つのテクスチャセット（塗りつぶしの色を赤に）を足し、今のセットに戻ってグループとポインターレイヤーを作る。
fn pointer_scene(state: &mut AppState, lang: Lang) {
    use yolu_app::fx::pointers::PointerOp;
    use yolu_app::fx::FxOp;
    use yolu_core::{Channel, ExternalSet, LayerKind, PointerTarget, Rgba8};
    let Ok(_) = state.add_texture_set() else {
        return;
    };
    if let Some(fill) = state
        .session
        .doc
        .layers()
        .iter()
        .find(|l| l.kind() == LayerKind::Fill)
        .map(|l| l.id())
    {
        state.apply(Action::M2(Edit::FillValue {
            id: fill,
            channel: Channel::Color,
            value: Some(Rgba8::new(214, 72, 64, 255)),
        }));
    }
    let key = ExternalSet {
        texture_set: yolu_app::fx::pointers::set_key(&state.session.sets.current().id).unwrap_or(0),
        layer_set: state.session.doc.layer_sets()[0].id.0,
    };
    let _ = state.switch_set(0);
    state.apply(Action::NewLayer);
    state.apply(Action::M2(Edit::GroupSelected));
    let name = format!("{} 1", lang.pick("グループ", "Group"));
    if let Some(group) = state.session.selected_layer {
        state.apply(Action::RenameLayer(group, name));
    }
    state.apply(Action::Fx(FxOp::Pointer(PointerOp::New(None))));
    if let Some(id) = state.session.selected_layer {
        state.apply(Action::Fx(FxOp::Pointer(PointerOp::Target {
            layer: id,
            target: PointerTarget::External(key),
        })));
    }
    state.sync_effect_inputs();
}

/// ポップアップを開いた形のダミーの状態（`ctx` が要るので、アプリを起こした後に呼ぶ）。
pub fn open_popup(scene: Scene, state: &mut AppState, ctx: &egui::Context) {
    use yolu_app::m2_menu::Popup;
    use yolu_app::state::{OpenPopup, PopupKind};
    use yolu_app::ui::menu::PopupState;
    let pointer = state.session.selected_layer;
    let raster = state
        .session
        .doc
        .layers()
        .iter()
        .find(|l| l.kind() == yolu_core::LayerKind::Raster)
        .map(|l| l.id());
    let (kind, at) = match (scene, pointer, raster) {
        (Scene::PointerTarget, Some(id), _) => (PopupKind::M2(Popup::PointerTarget(id)), 150.0),
        (Scene::PointerContext, _, Some(id)) => (PopupKind::LayerContext(id), 120.0),
        (Scene::PointerAdd, _, _) => (PopupKind::M2(Popup::LayerBlank), 120.0),
        _ => return,
    };
    let anchor = egui::Rect::from_min_size(egui::pos2(24.0, at), egui::vec2(300.0, 24.0));
    state.ui.popup = Some(OpenPopup {
        kind,
        state: PopupState::new(ctx, anchor).with_min_width(240.0),
    });
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
