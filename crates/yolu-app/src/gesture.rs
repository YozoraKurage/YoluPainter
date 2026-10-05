//! ビュー（2D のキャンバス・3D ビュー）への押しの振り分けの、マウスとペンで共通の部分。
//!
//! - 押しの持ち主: 押した所の部品（ドックのタブの見出し・分け目・隅のアイコンなど）が egui の押しを受けているとき、その押しは
//!   ビューのものではない。押し始めを決めたら、離すまで変えない（見出しをつかんだままペンをキャンバスへ動かしても、そこから描き始めない）。
//! - Ctrl+Space の拡縮（Photoshop・CLIP STUDIO と同じ）: 押しながら左右にドラッグすると拡大・縮小、動かさずに離すと拡大、Alt も押していれば縮小。

use egui::{Modifiers, Pos2, Response};

/// 動かさずに離したとみなす、押してから離すまでに動いてよい距離（画面の点）。
pub const CLICK_MOVE: f32 = 4.0;

/// Ctrl（Mac の Command も）を押しているか。
pub fn ctrl(m: &Modifiers) -> bool {
    m.ctrl || m.command
}

/// Ctrl+Space の拡縮の組み合わせか（`space` は Space を押しているか）。
pub fn zoom_chord(m: &Modifiers, space: bool) -> bool {
    space && ctrl(m)
}

/// この押しを egui が別の部品（ドックの見出し・分け目・ビューの上に重ねたアイコンなど）の押しとして受けているか。`response` は
/// ビューの入力の応答。egui が何の押しも受けていなければ（ペンの代わりのポインタが無いときなど）偽で、ビューのものとして扱う。
pub fn foreign_press(ctx: &egui::Context, response: &Response) -> bool {
    // 窓の縁の押し（大きさを変える）は、縁がビューの端に重なっていても、ビューのものではない
    crate::titlebar::edge_press_held(ctx)
        || (ctx.egui_is_using_pointer() && !response.is_pointer_button_down_on())
}

/// Ctrl+Space のドラッグ（押した点・前の位置・動いた距離）。押した点は 2D の拡縮の中心。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomDrag {
    pub anchor: Pos2,
    pub last: Pos2,
    /// 押してから動いた距離の合計（クリックかドラッグかを分ける）。
    pub moved: f32,
    /// Alt を押して押した（クリックなら縮小）。
    pub out: bool,
}

impl ZoomDrag {
    pub fn new(anchor: Pos2, out: bool) -> ZoomDrag {
        ZoomDrag {
            anchor,
            last: anchor,
            moved: 0.0,
            out,
        }
    }

    /// 位置が動いた。横に動いた量（右が正）を返す。
    pub fn moved_to(&mut self, pos: Pos2) -> f32 {
        let d = pos - self.last;
        self.last = pos;
        self.moved += d.length();
        d.x
    }

    /// 動かさずに離した（クリック）か。
    pub fn is_click(&self) -> bool {
        self.moved < CLICK_MOVE
    }
}

/// 2D の拡縮: ドラッグ 1 点あたりの対数の拡大率（100 点で約 2.2 倍）。
pub const ZOOM_PER_POINT: f32 = 0.008;
/// クリック 1 回の拡大・縮小の倍率。
pub const CLICK_ZOOM: f32 = 2.0;
/// 3D の拡縮: ドラッグの点数を、ホイールの目盛りに直す割り（40 点で 1 目盛り）。
pub const POINTS_PER_NOTCH: f32 = 40.0;
/// 3D のクリック 1 回の寄る・引く（ホイールの目盛り）。
pub const CLICK_NOTCHES: f32 = 3.0;

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    #[test]
    fn ctrl_means_ctrl_or_command() {
        assert!(ctrl(&Modifiers::CTRL));
        assert!(ctrl(&Modifiers::COMMAND));
        assert!(!ctrl(&Modifiers::SHIFT));
        assert!(zoom_chord(&Modifiers::CTRL, true));
        assert!(!zoom_chord(&Modifiers::CTRL, false), "Space が要る");
        assert!(!zoom_chord(&Modifiers::NONE, true), "Ctrl が要る");
    }

    #[test]
    fn a_zoom_drag_tells_a_click_from_a_drag() {
        let mut z = ZoomDrag::new(pos2(10.0, 10.0), false);
        assert!(z.is_click());
        assert_eq!(z.moved_to(pos2(12.0, 10.0)), 2.0);
        assert!(z.is_click(), "4 点より少ない動きはクリック");
        assert_eq!(z.moved_to(pos2(20.0, 10.0)), 8.0);
        assert!(!z.is_click());
        // 戻ってきても、動いた距離は減らない
        z.moved_to(pos2(10.0, 10.0));
        assert!(!z.is_click());
    }
}
