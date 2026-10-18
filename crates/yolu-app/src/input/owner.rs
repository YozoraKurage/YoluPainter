//! 押しの持ち主: 押した瞬間に、その押しをツール・ビューを動かす操作・スポイト・何もしない のどれにするかを決める（`owner`）。
//! 2D のキャンバスと 3D ビュー、マウスとペンが同じ関数を通る。ビューごとの事情（押しを始めてよいか・ドラッグの操作と離しの操作の表・
//! ツールのボタン・モード）は、ビューが `PressCtx` の欄にして渡す。決めた持ち主は `Press` に控え、離すまで変えない。

use egui::Pos2;

use crate::state::key::PointerButton;
use crate::state::StrokeSource;

/// 押しの持ち主。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// ツールの押し（ブラシ・消しゴム・範囲のツール・選択のツール・図形など。3D は編集・ポーズのモードの物の選び・ギズモも）。
    /// 始められなかった押し（読むだけ・Alt の予約など）も、離すまでここに留まる。
    Tool,
    /// ビューを動かす操作（2D は回す・パン・拡縮、3D は回す・パン・拡縮）と、動かさずに離したときの操作（クローンの元。3D はスポイトも）。
    View,
    /// 2D のスポイトのボタン（既定は右ボタン・ペンのサイドボタン）: 押した所の色を見本にして、離して取る。
    Eyedrop,
    /// 何もしない（押した所がビューの外・別の部品の上・ポップアップの下、ステンシルを動かしている間、描いている・ビューを動かしている最中、
    /// ポリゴン塗りつぶしの 2D のサイドボタン、修飾を押した描くツールのペン）。
    Ignored,
}

/// ビューが持つ、今の押し（押してから離すまで）。ペンは押しごと、マウスはツールの押し（`Owner::Tool`）を控える。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Press {
    pub source: StrokeSource,
    pub owner: Owner,
    /// 押したボタン（ペンはサイドボタンで右ボタン、ペン先で左ボタン）。
    pub button: PointerButton,
    /// 前の点（ビューを動かす量を求める）。
    pub last: Pos2,
}

/// 押しの持ち主を決めるのに使う、ビューごとの事情。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PressCtx {
    /// 押しを始めてはいけない（ポップアップ・ドックの見出しをつかんでいる・押しがほかの部品のもの。3D は設定のパネルを開いている間も）。
    pub blocked: bool,
    /// 押した点で一番上にあるのがこのビュー（`input::on_top`）。
    pub on_top: bool,
    /// ステンシルを動かしている（置き場を動かすキーを押している・ドラッグしている）。
    pub stencil: bool,
    /// 描いている・図形や選択の形を作っている・ビューを動かしている最中。
    pub busy: bool,
    /// このボタンと修飾に、ビューを動かすドラッグの操作がある。
    pub view_op: bool,
    /// このボタンと修飾に、動かさずに離したときの操作がある。
    pub click_op: bool,
    /// スポイトのボタン（2D の `nav::Start::Pick`。3D は離しの操作で持つので使わない）。
    pub eyedrop: bool,
    /// ツールのボタン（左ボタンと、選択のツールの選択の行を割り当てたボタン）。
    pub tool: bool,
    /// ツールの押しを受けるモード（2D は描けるモード。3D は編集・ポーズのモードの物の選び・ギズモも受けるので、いつも真）。
    pub paints: bool,
    /// 描くツールで Ctrl を押したペン（描かない）。
    pub holds_off: bool,
}

/// 押しの持ち主（決める順は 1 つ）:
/// 1. 始めてはいけない・一番上でない・ステンシルを動かしている → 何もしない
/// 2. 描いている・図形や選択の形を作っている・ビューを動かしている最中 → 何もしない
/// 3. ビューを動かす操作か、離しの操作がある → ビュー
/// 4. スポイトのボタン → スポイト
/// 5. ツールのボタンで、ツールの押しを受けるモードで、ペンの Ctrl でない → ツール
/// 6. ほか → 何もしない
pub fn owner(c: &PressCtx) -> Owner {
    if c.blocked || !c.on_top || c.stencil {
        return Owner::Ignored;
    }
    if c.busy {
        return Owner::Ignored;
    }
    if c.view_op || c.click_op {
        return Owner::View;
    }
    if c.eyedrop {
        return Owner::Eyedrop;
    }
    if c.tool && c.paints && !c.holds_off {
        return Owner::Tool;
    }
    Owner::Ignored
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一番上で、押してよく、何もしていない、左ボタンのツールの押し（描けるモード）。
    fn tool_press() -> PressCtx {
        PressCtx {
            on_top: true,
            tool: true,
            paints: true,
            ..PressCtx::default()
        }
    }

    #[test]
    fn step_1_a_blocked_press_off_the_top_or_while_moving_the_stencil_is_ignored() {
        for c in [
            PressCtx {
                blocked: true,
                ..tool_press()
            },
            PressCtx {
                on_top: false,
                ..tool_press()
            },
            PressCtx {
                stencil: true,
                ..tool_press()
            },
            // ビューの操作やスポイトより先
            PressCtx {
                blocked: true,
                view_op: true,
                ..tool_press()
            },
            PressCtx {
                stencil: true,
                eyedrop: true,
                ..tool_press()
            },
        ] {
            assert_eq!(owner(&c), Owner::Ignored, "{c:?}");
        }
    }

    #[test]
    fn step_2_a_press_while_busy_is_ignored_even_for_a_view_operation() {
        for c in [
            PressCtx {
                busy: true,
                ..tool_press()
            },
            PressCtx {
                busy: true,
                view_op: true,
                ..tool_press()
            },
            PressCtx {
                busy: true,
                click_op: true,
                ..tool_press()
            },
            PressCtx {
                busy: true,
                eyedrop: true,
                ..tool_press()
            },
        ] {
            assert_eq!(owner(&c), Owner::Ignored, "{c:?}");
        }
    }

    #[test]
    fn step_3_a_view_operation_or_a_click_operation_goes_to_the_view_before_the_tool() {
        for c in [
            PressCtx {
                view_op: true,
                ..tool_press()
            },
            PressCtx {
                click_op: true,
                ..tool_press()
            },
            PressCtx {
                click_op: true,
                eyedrop: true,
                ..tool_press()
            },
            // 描けないモードでも、ビューは動かせる
            PressCtx {
                view_op: true,
                paints: false,
                ..tool_press()
            },
        ] {
            assert_eq!(owner(&c), Owner::View, "{c:?}");
        }
    }

    #[test]
    fn step_4_the_eyedrop_button_picks_before_the_tool() {
        let c = PressCtx {
            eyedrop: true,
            ..tool_press()
        };
        assert_eq!(owner(&c), Owner::Eyedrop);
        let c = PressCtx {
            eyedrop: true,
            tool: false,
            paints: false,
            ..tool_press()
        };
        assert_eq!(owner(&c), Owner::Eyedrop);
    }

    #[test]
    fn step_5_the_tool_button_in_a_painting_mode_without_the_pen_ctrl_is_the_tool() {
        assert_eq!(owner(&tool_press()), Owner::Tool);
        let held_off = PressCtx {
            holds_off: true,
            ..tool_press()
        };
        assert_eq!(owner(&held_off), Owner::Ignored);
        let not_painting = PressCtx {
            paints: false,
            ..tool_press()
        };
        assert_eq!(owner(&not_painting), Owner::Ignored);
    }

    #[test]
    fn step_6_anything_else_is_ignored() {
        let c = PressCtx {
            tool: false,
            ..tool_press()
        };
        assert_eq!(owner(&c), Owner::Ignored);
        assert_eq!(owner(&PressCtx::default()), Owner::Ignored);
    }

    /// D13: マウスで描いている途中に、ペン先が触れた（ツールのボタン・描けるモード）。
    #[test]
    fn d13_a_pen_tip_while_the_mouse_paints_does_nothing() {
        let c = PressCtx {
            busy: true,
            ..tool_press()
        };
        assert_eq!(owner(&c), Owner::Ignored);
    }

    /// D16: 中ボタンでパンしている途中に、ペン先が触れた。
    #[test]
    fn d16_a_pen_tip_while_the_view_pans_does_nothing() {
        let c = PressCtx {
            busy: true,
            ..tool_press()
        };
        assert_eq!(owner(&c), Owner::Ignored);
        // パンが終われば描く
        assert_eq!(owner(&tool_press()), Owner::Tool);
    }

    /// D17: 選択の行を左以外のボタンに割り当てた押しは、ツールのボタン（2D も 3D も）。
    #[test]
    fn d17_a_selection_row_on_another_button_is_a_tool_press() {
        let c = PressCtx {
            tool: true,
            ..tool_press()
        };
        assert_eq!(owner(&c), Owner::Tool);
        // 割り当てていないボタン（ビューの操作も無い）は何もしない
        let c = PressCtx {
            tool: false,
            ..tool_press()
        };
        assert_eq!(owner(&c), Owner::Ignored);
    }
}
