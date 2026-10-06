//! ファイルを選ぶ窓と確かめの窓（OS の窓）を、アプリの主の窓の子として出す口。
//!
//! 親を渡さない OS の窓は、アプリの窓の後ろに回ったり、タスクバーに別の項目として出たりする（Windows）。親を渡すと、OS が
//! 主の窓の上に重ね、開いている間は主の窓を操作できなくする。親にするのは起動のときに窓の作成の文脈から預かった主の窓
//! （`set_owner`。実際の窓だけ）で、預かっていない・窓がもう無いときは親なしで出す。
//! クラッシュの知らせ（`crash`）は、窓の無い起動でも出すので、ここを通さない。

/// ファイルを選ぶ窓（親つき）。
pub fn file() -> rfd::FileDialog {
    let dialog = rfd::FileDialog::new();
    #[cfg(windows)]
    if let Some(owner) = owner::get() {
        return dialog.set_parent(&owner);
    }
    dialog
}

/// 確かめの窓（親つき）。
pub fn message() -> rfd::MessageDialog {
    let dialog = rfd::MessageDialog::new();
    #[cfg(windows)]
    if let Some(owner) = owner::get() {
        return dialog.set_parent(&owner);
    }
    dialog
}

/// 主の窓を預かる（実際の窓の作成のとき 1 回。Windows 以外は何もしない）。
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
        DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
    };
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::IsWindow;

    /// 預かった主の窓（HWND の値。0 は預かっていない）。
    static OWNER: AtomicIsize = AtomicIsize::new(0);

    pub fn set(cc: &eframe::CreationContext<'_>) {
        if let Ok(handle) = cc.window_handle() {
            if let RawWindowHandle::Win32(raw) = handle.as_raw() {
                OWNER.store(raw.hwnd.get(), Ordering::Relaxed);
            }
        }
    }

    /// 預かった窓が今もあれば、`rfd` に親として渡せる形で返す。
    pub fn get() -> Option<Owner> {
        let hwnd = NonZeroIsize::new(OWNER.load(Ordering::Relaxed))?;
        // SAFETY: 窓のハンドルが今も窓を指しているかを見るだけ。
        unsafe { IsWindow(Some(HWND(hwnd.get() as *mut _))) }.as_bool().then_some(Owner(hwnd))
    }

    /// 主の窓（`rfd` の `set_parent` が窓のハンドルと表示のハンドルを求める）。
    pub struct Owner(NonZeroIsize);

    impl HasWindowHandle for Owner {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            // SAFETY: 主の窓はアプリが動いている間ずっとあり、`get` が今も窓であることを確かめている。
            Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(Win32WindowHandle::new(self.0))) })
        }
    }

    impl HasDisplayHandle for Owner {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            Ok(DisplayHandle::windows())
        }
    }
}
