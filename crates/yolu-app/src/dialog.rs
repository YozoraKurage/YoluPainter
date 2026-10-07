//! ファイルを選ぶウィンドウと確認のウィンドウ（OS のウィンドウ）を、アプリのメインウィンドウの子として出す口。
//!
//! 親を渡さない OS のウィンドウは、アプリのウィンドウの後ろに回ったり、タスクバーに別の項目として出たりする（Windows）。親を渡すと、OS が
//! メインウィンドウの上に重ね、開いている間はメインウィンドウを操作できなくする。親にするのは起動のときにウィンドウの作成の文脈から預かったメインウィンドウ
//! （`set_owner`。実際のウィンドウだけ）で、預かっていない・ウィンドウがもう無いときは親なしで出す。
//! クラッシュの知らせ（`crash`）は、ウィンドウの無い起動でも出すので、ここを通さない。

/// ファイルを選ぶウィンドウ（親つき）。
pub fn file() -> rfd::FileDialog {
    let dialog = rfd::FileDialog::new();
    #[cfg(windows)]
    if let Some(owner) = owner::get() {
        return dialog.set_parent(&owner);
    }
    dialog
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
