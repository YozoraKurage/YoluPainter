//! ファイルを選ぶウィンドウと確認のウィンドウ（OS のウィンドウ）を、アプリのメインウィンドウの子として出す口。
//!
//! 親を渡さない OS のウィンドウは、アプリのウィンドウの後ろに回ったり、タスクバーに別の項目として出たりする（Windows）。親を渡すと、OS が
//! メインウィンドウの上に重ね、開いている間はメインウィンドウを操作できなくする。親にするのは起動のときにウィンドウの作成の文脈から預かったメインウィンドウ
//! （`set_owner`。実際のウィンドウだけ）で、預かっていない・ウィンドウがもう無いときは親なしで出す。
//! クラッシュの知らせ（`crash`）は、ウィンドウの無い起動でも出すので、ここを通さない。
//!
//! ファイルを選ぶウィンドウは、入り口の種類（`places::Place`）を渡して作り、始まりの場所は種類ごとに前に使った場所・今の文書の場所・OS の書類の
//! フォルダから決める（`places`）。選んだら `AppState::note_file_chosen`・`note_folder_chosen` で場所を覚える。

pub mod places;

use std::path::{Path, PathBuf};

use self::places::{Place, Rule, Sources};
use crate::state::AppState;

/// 親つきの、始まりの場所の無いファイルを選ぶウィンドウ。
fn parented() -> rfd::FileDialog {
    let dialog = rfd::FileDialog::new();
    #[cfg(windows)]
    if let Some(owner) = owner::get() {
        return dialog.set_parent(&owner);
    }
    dialog
}

/// ファイルを選ぶウィンドウ（親つき。開く・取り込み・書き出し）。始まりの場所は、前に `place` で使った場所 → 今の文書のフォルダ → OS の
/// 書類のフォルダ（`places::Rule::Open`）。
pub fn file(state: &AppState, place: Place) -> rfd::FileDialog {
    started(state, place, Rule::Open)
}

/// ファイルを選ぶウィンドウ（親つき。保存・別名で保存）。始まりの場所は、退避を開いた文書の元のフォルダ → 今の文書のフォルダ → 前に `place` で
/// 使った場所 → OS の書類のフォルダ（`places::Rule::Save`）。
pub fn file_for_save(state: &AppState, place: Place) -> rfd::FileDialog {
    started(state, place, Rule::Save)
}

fn started(state: &AppState, place: Place, rule: Rule) -> rfd::FileDialog {
    let dialog = parented();
    match start_folder_of(state, place, rule) {
        Some(folder) => dialog.set_directory(folder),
        None => dialog,
    }
}

/// `place` のウィンドウを開く始まりの場所（決められなければ None）。
pub fn start_folder_of(state: &AppState, place: Place, rule: Rule) -> Option<PathBuf> {
    start_folder_with(state, place, rule, places::documents_folder().as_deref())
}

/// `start_folder_of` の、OS の書類のフォルダを渡せる形（試験は決まったフォルダを渡す）。
pub fn start_folder_with(
    state: &AppState,
    place: Place,
    rule: Rule,
    documents: Option<&Path>,
) -> Option<PathBuf> {
    let document = state.document_folder();
    places::start_folder(
        rule,
        &Sources {
            preferred: state.save_folder.as_deref(),
            document: document.as_deref(),
            remembered: state.places.get(place),
            documents,
        },
        &|p: &Path| p.is_dir(),
    )
}

impl AppState {
    /// 今の文書の .ylp のあるフォルダ（保存していない文書・ファイルの無いプロジェクトでは None）。
    pub fn document_folder(&self) -> Option<PathBuf> {
        let parent = self
            .project
            .as_ref()
            .filter(|p| p.is_file())
            .and_then(|p| p.path().parent())?;
        std::path::absolute(parent).ok()
    }

    /// 選んだフォルダを、`place` の前に使った場所として覚える。
    pub fn note_folder_chosen(&mut self, place: Place, folder: &Path) {
        self.places.remember(place, folder);
    }

    /// 選んだファイルのあるフォルダを、`place` の前に使った場所として覚える。
    pub fn note_file_chosen(&mut self, place: Place, file: &Path) {
        if let Some(folder) = file.parent().filter(|p| !p.as_os_str().is_empty()) {
            self.places.remember(place, folder);
        }
    }
}

/// 確認のウィンドウ（親つき）。
pub fn message() -> rfd::MessageDialog {
    let dialog = rfd::MessageDialog::new();
    #[cfg(windows)]
    if let Some(owner) = owner::get() {
        return dialog.set_parent(&owner);
    }
    dialog
}

/// メインウィンドウを預かる（実際のウィンドウの作成のとき 1 回。Windows 以外は何もしない）。
pub fn set_owner(cc: &eframe::CreationContext<'_>) {
    #[cfg(windows)]
    owner::set(cc);
    #[cfg(not(windows))]
    let _ = cc;
}

#[cfg(windows)]
mod owner {
    use std::num::NonZeroIsize;
    use std::sync::atomic::{AtomicIsize, Ordering};

    use raw_window_handle::{
        DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle,
        Win32WindowHandle, WindowHandle,
    };
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::IsWindow;

    /// 預かったメインウィンドウ（HWND の値。0 は預かっていない）。
    static OWNER: AtomicIsize = AtomicIsize::new(0);

    pub fn set(cc: &eframe::CreationContext<'_>) {
        if let Ok(handle) = cc.window_handle() {
            if let RawWindowHandle::Win32(raw) = handle.as_raw() {
                OWNER.store(raw.hwnd.get(), Ordering::Relaxed);
            }
        }
    }

    /// 預かったウィンドウが今もあれば、`rfd` に親として渡せる形で返す。
    pub fn get() -> Option<Owner> {
        let hwnd = NonZeroIsize::new(OWNER.load(Ordering::Relaxed))?;
        // SAFETY: ウィンドウのハンドルが今もウィンドウを指しているかを見るだけ。
        unsafe { IsWindow(Some(HWND(hwnd.get() as *mut _))) }
            .as_bool()
            .then_some(Owner(hwnd))
    }

    /// メインウィンドウ（`rfd` の `set_parent` がウィンドウのハンドルと表示のハンドルを求める）。
    pub struct Owner(NonZeroIsize);

    impl HasWindowHandle for Owner {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            // SAFETY: メインウィンドウはアプリが動いている間ずっとあり、`get` が今もウィンドウであることを確かめている。
            Ok(unsafe {
                WindowHandle::borrow_raw(RawWindowHandle::Win32(Win32WindowHandle::new(self.0)))
            })
        }
    }

    impl HasDisplayHandle for Owner {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            Ok(DisplayHandle::windows())
        }
    }
}
