//! Win32 のメッセージからスポイト入力への変換。OS 呼び出しだけを差し替える。
use super::PickInput;

pub(super) trait Pointer {
    fn cursor_position(&self) -> Option<[i32; 2]>;
    fn client_to_screen(&self, point: [i32; 2]) -> Option<[i32; 2]>;
}

pub(super) fn decode(
    message: u32,
    key: usize,
    position: isize,
    pointer: &impl Pointer,
) -> Option<PickInput> {
    // WM_LBUTTONDOWN、WM_KEYDOWN、WM_SYSKEYDOWN（Windows SDK のメッセージ値）。
    match message {
        0x0201 => {
            // GET_X_LPARAM / GET_Y_LPARAM と同じ符号拡張。画面の負座標も保持する。
            let point = [
                position as u16 as i16 as i32,
                (position >> 16) as u16 as i16 as i32,
            ];
            Some(
                pointer
                    .client_to_screen(point)
                    .map_or(PickInput::Cancel, PickInput::Click),
            )
        }
        0x0100 | 0x0104 if key == 0x1b => Some(PickInput::Cancel),
        _ => None,
    }
}
