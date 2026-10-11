//! 定規（画面の側）。定規そのものはレイヤー（グループも）が持つ文書の値（core の `Ruler`。`yolu_core::rulers`）で、ここは
//! 「どの定規が今の描く先で効くか」と、定規を作る・動かす・消す画面を持つ。
//!
//! - `active`: 描くレイヤーから見える定規 → 寄せ先と対称の写し（`AppState::rulers_active`）。ストロークの始めに固める。
//! - `ops`: 文書を変える操作（`RulerAction`。1 つが 1 回の取り消し。ドラッグとスライダーはまとめる）。
//! - `canvas`・`draw`: 2D のキャンバスの定規のツール（作る・つまみで動かす）と、定規と対称の線の描画。
//! - `edit3d`・`draw3d`: 3D の定規（モデルの空間）を 3D ビューで作る・画面に写した寄せ先にする・編集のモードで動かす幾何と、3D ビューの上の描画。
//! - `tool`: 定規のツールのオプションバーとツールプロパティ、「定規にスナップ」「特殊定規にスナップ」のボタン。
//! - `props`: プロパティのレイヤーの欄の「定規」の区分。`layer_icon`: レイヤーの一覧の定規のアイコン。
//!
//! スナップ 2 つの入り切りと、これから作る定規の設定（種類・本数・角度の刻みなど）は画面の状態で、文書にも設定にも書かない。

pub mod active;
pub mod canvas;
pub mod draw;
pub mod draw3d;
pub mod edit3d;
pub mod layer_icon;
pub mod ops;
pub mod props;
pub mod tool;

pub use active::{Active, Place, Shown};
pub use ops::RulerAction;

use std::ops::RangeInclusive;

use yolu_core::glam::DVec2;
use yolu_core::{LayerId, Ruler, RulerId, RulerKind};

use crate::state::StrokeSource;

/// 直線定規へ寄せる距離（画面の点。約 7 mm）。この内で描き始めたときだけ寄せる。
pub const SNAP_POINTS: f64 = 26.0;
/// つまみを掴める距離（画面の点）。
pub const HANDLE_POINTS: f32 = 12.0;
/// 角度の刻みの範囲（度）と既定。
pub const STEP_RANGE: RangeInclusive<u32> = 1..=90;
pub const DEFAULT_STEP: u32 = 15;

/// つまみ（選んでいる定規の掴める点）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    /// 点 `a`（種類ごとに、その点だけを動かす・全体を動かす）。
    A,
    /// 点 `b`（向き・半径・もう 1 つの消失点・線の端）。
    B,
    /// 直線定規の 2 点の真ん中（全体を動かす）。
    Mid,
}

/// 掴んだ既存の定規。
#[derive(Clone, Debug, PartialEq)]
pub struct Grab {
    pub owner: LayerId,
    pub original: Ruler,
    pub handle: Handle,
}

/// 2D のキャンバスの定規のドラッグの途中（離すまで文書を変えない）。
#[derive(Clone, Debug, PartialEq)]
pub struct RulerDrag {
    pub source: StrokeSource,
    pub start: DVec2,
    pub current: DVec2,
    pub shift: bool,
    /// 掴んだ既存の定規。None は新しい定規を引いている。
    pub grab: Option<Grab>,
}

/// レイヤーの一覧で、定規のアイコンを持って動かしているところ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IconDrag {
    pub from: LayerId,
    /// ポインタの下の行のレイヤー（`from` の行を除く）。
    pub over: Option<LayerId>,
    /// その行へ移せるか（読むだけでない・上限を超えない）。移せない行は光らせない（離すと、移す操作が理由を知らせて断る）。
    pub can_drop: bool,
}

/// 定規の画面の状態。
#[derive(Clone, Debug)]
pub struct RulerState {
    /// これから作る定規の種類と設定（置いてある定規は替えない）。
    pub kind: RulerKind,
    pub two_points: bool,
    /// 対称定規の線の本数（2〜16）と、線対称（偽は回転対称）。
    pub lines: u8,
    pub line_symmetry: bool,
    /// 最初の線（ほかの種類は a→b）の向きを刻みの倍数に丸める。
    pub angle_step: bool,
    pub step: u32,
    /// 選んでいるレイヤー（グループ）へ作る。偽なら、選んでいるレイヤーのすぐ上に新しいレイヤー「定規」を作ってそこへ。
    pub on_layer: bool,
    /// 「定規にスナップ」（直線定規）と「特殊定規にスナップ」（平行線・同心円・パース・対称）。
    pub snap_ruler: bool,
    pub snap_special: bool,
    /// 選んでいる定規（持ち主のレイヤーと ID）。持ち主が選んでいるレイヤーのときだけ、2D のキャンバスにつまみが出る。
    pub selected: Option<(LayerId, RulerId)>,
    pub drag: Option<RulerDrag>,
    pub icon_drag: Option<IconDrag>,
    /// ポインタが乗っているビュー（前のフレームの結果。どちらにも乗っていなければ None）。「スナップする特殊定規の切り替え」の空間を決める。
    pub pointer_in: Option<Place>,
    /// 最後にツールで押して描き始めた（ブラシ・図形・グラデーション・塗りつぶし・定規などの押しが入った）ビュー。2D のキャンバスと 3D ビューの描く押しの入口が書く。
    pub last_drew: Option<Place>,
}

impl Default for RulerState {
    fn default() -> Self {
        RulerState {
            kind: RulerKind::Line,
            two_points: false,
            lines: 2,
            line_symmetry: true,
            angle_step: false,
            step: DEFAULT_STEP,
            on_layer: true,
            snap_ruler: true,
            snap_special: true,
            selected: None,
            drag: None,
            icon_drag: None,
            pointer_in: None,
            last_drew: None,
        }
    }
}

impl RulerState {
    /// 線の本数を 2〜16 に丸めて置く（線対称は偶数だけ。奇数なら 1 つ上へ）。
    pub fn set_lines(&mut self, lines: u8) {
        self.lines = fit_lines(lines, self.line_symmetry);
    }

    /// 線対称の入り切り（入れるときに奇数なら 1 つ上の偶数へ）。
    pub fn set_line_symmetry(&mut self, on: bool) {
        self.line_symmetry = on;
        self.lines = fit_lines(self.lines, on);
    }

    /// ポインタがビュー `place` に乗っているか（毎フレーム、ビューが描くときに知らせる。乗っていなければ、そのビューに乗っていた印を外す）。
    pub fn note_pointer(&mut self, place: Place, over: bool) {
        if over {
            self.pointer_in = Some(place);
        } else if self.pointer_in == Some(place) {
            self.pointer_in = None;
        }
    }

    pub fn set_step(&mut self, step: u32) {
        self.step = step.clamp(*STEP_RANGE.start(), *STEP_RANGE.end());
    }
}

/// 線の本数を 2〜16 に丸める（線対称は偶数だけ。奇数なら 1 つ上へ。16 の奇数は 16）。
pub fn fit_lines(lines: u8, line_symmetry: bool) -> u8 {
    let lines = lines.clamp(2, 16);
    if line_symmetry && lines % 2 == 1 {
        (lines + 1).min(16)
    } else {
        lines
    }
}
