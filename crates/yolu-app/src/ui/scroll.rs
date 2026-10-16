//! 縦のスクロールの共通の部品。このアプリの自前の欄（ブラシの一覧・レイヤー・アセット・設定のウィンドウなど）は、行を自分で並べて
//! 位置をずらすので、ホイールで送る・つまみを掴んで動かす・溝を押して移る、までをここ 1 か所に持つ。振る舞いと寸法は、カラーセットの
//! 欄が使う egui の `ScrollArea`（`ScrollStyle::thin`）と同じ: つまみの太さは普段 2 点で、ポインタを乗せる・掴むと 10 点に広がり、
//! 掴める幅はいつも 10 点（右の縁）。つまみを押せばその位置から動かし、溝を押せばつまみがそこへ移って（中心がポインタ）、そのまま
//! 動かせる。ペンは winit が WM_POINTER を egui の Touch とポインタ（左ボタン）に変える（触れていない間のホバーも、ポインタを動かす）ので、
//! マウスと同じ道を通る（`pen` の Windows Ink の差し込みは筆圧を読むだけで、egui の入力はそのまま通す。これはソースを読んだ経路で、
//! 試験はポインタと Touch の入力を手で組み立てて渡している。実機の液タブでの確認は別）。
//!
//! 使い方（呼ぶ側が持つのは、ずらす量 `offset` だけ）:
//! 1. 行を並べる前に [`Scroll::begin`]（ホイールを受けて範囲に収める。ホイールを別に決める欄は [`Scroll::new`]）
//! 2. `offset` の分だけずらして行を描く。つまみが乗る幅は [`Scroll::reserved`]（あふれるときだけ 10 点）
//! 3. 行を描いたあとに [`Scroll::end`]（つまみの操作と、右ボタンを押したまま動かす操作を受けて `offset` を動かし、つまみを重ねて描く。
//!    egui の `ScrollArea` と同じく、動かした分は次のフレームの行に出る。開いているメニューの中は [`Scroll::end_menu`]）

use egui::{lerp, pos2, remap, remap_clamp, Id, Rect, Sense, Ui};

use super::right_drag;
use super::theme as t;

/// 掴める幅（右の縁。egui の `ScrollStyle::thin` の `bar_width`）。広がったつまみの太さでもある。
pub const BAR_WIDTH: f32 = 10.0;
/// つまみの普段の太さ（egui の `floating_width`）。
pub const THIN_WIDTH: f32 = 2.0;
/// つまみの最小の長さ（egui の `handle_min_length`）。
pub const HANDLE_MIN: f32 = 12.0;

/// 普段のつまみ・乗せたつまみ・掴んだつまみの色。
const HANDLE: egui::Color32 = t::TEXT_DISABLED;
const HANDLE_HOVER: egui::Color32 = t::TEXT_DIM;
const HANDLE_ACTIVE: egui::Color32 = t::TEXT;

/// 1 つのスクロールの寸法（`view` の中に、高さ `content` の中身を `offset` だけずらして見せる）。
#[derive(Clone, Copy, Debug)]
pub struct Scroll {
    /// 見える範囲（つまみは、この右の縁の内側）。
    pub view: Rect,
    /// 中身の高さ。
    pub content: f32,
    /// ずらせる最大の量（中身が収まるなら 0）。
    pub max: f32,
}

/// 掴んでいる位置を覚える（つまみの上なら、つまみの上端からの距離。溝なら、つまみの中心が来るずれ）。
fn grab_id(id: Id) -> Id {
    id.with("grab")
}

impl Scroll {
    /// ホイールは受けず、寸法だけ。`offset` を範囲に収める。
    pub fn new(view: Rect, content: f32, offset: &mut f32) -> Scroll {
        let max = (content - view.height()).max(0.0);
        *offset = if offset.is_finite() {
            offset.clamp(0.0, max)
        } else {
            0.0
        };
        Scroll { view, content, max }
    }

    /// ホイール（ポインタが `view` の上にあるとき）で `offset` を動かし、範囲に収める。
    pub fn begin(ui: &Ui, view: Rect, content: f32, offset: &mut f32) -> Scroll {
        if ui.rect_contains_pointer(view) {
            *offset -= ui.input(|i| i.smooth_scroll_delta.y);
        }
        Scroll::new(view, content, offset)
    }

    /// 中身があふれて、つまみを出すか。
    pub fn needed(&self) -> bool {
        self.max > 0.0
    }

    /// 行の幅から引く、つまみの分（あふれるときだけ。つまみを行に重ねる欄は引かない）。
    pub fn reserved(&self) -> f32 {
        if self.needed() {
            BAR_WIDTH
        } else {
            0.0
        }
    }

    /// 掴める縦の帯（`view` の右の縁）。
    pub fn grab_rect(&self) -> Rect {
        Rect::from_min_max(
            pos2(self.view.right() - BAR_WIDTH, self.view.top()),
            self.view.max,
        )
    }

    /// 今の `offset` のつまみ（掴める帯の中の、縦の範囲。幅は帯いっぱい）。
    pub fn handle_rect(&self, offset: f32) -> Rect {
        let track = self.view;
        let length = (track.height() * track.height() / self.content.max(1.0))
            .max(HANDLE_MIN)
            .min(track.height());
        let top = remap_clamp(
            offset,
            0.0..=self.max.max(f32::EPSILON),
            track.top()..=(track.bottom() - length),
        );
        let grab = self.grab_rect();
        Rect::from_min_max(pos2(grab.left(), top), pos2(grab.right(), top + length))
    }

    /// つまみの操作と、右ボタンを押したまま動かす操作（[`super::right_drag`]。中身がポインタについて来る向き）を受けて `offset` を
    /// 動かし、つまみを重ねて描く（行を描いたあとに呼ぶ）。`id` は欄ごとに別の値。掴んでいる間だけ true を返す。
    pub fn end(
        &self,
        ui: &mut Ui,
        id: impl std::hash::Hash + std::fmt::Debug,
        offset: &mut f32,
    ) -> bool {
        self.finish(ui, Id::new(id), offset, true)
    }

    /// [`Scroll::end`] から、右ボタンを押したまま動かす操作を除いた物（開いているメニューの中の一覧）。
    pub fn end_menu(
        &self,
        ui: &mut Ui,
        id: impl std::hash::Hash + std::fmt::Debug,
        offset: &mut f32,
    ) -> bool {
        self.finish(ui, Id::new(id), offset, false)
    }

    fn finish(&self, ui: &mut Ui, id: Id, offset: &mut f32, pan: bool) -> bool {
        if !self.needed() {
            ui.data_mut(|d| d.remove::<f32>(grab_id(id)));
            return false;
        }
        let grab = self.grab_rect();
        let response = ui.interact(grab, id, Sense::click_and_drag());
        response.widget_info(|| egui::WidgetInfo::new(egui::WidgetType::ScrollBar));
        // つまみと溝は左ボタン（ペンの接触・指も egui では左ボタン）だけで掴む。右ボタンは一覧の右ドラッグ
        let by_left = {
            super::widgets::pressed_by_primary(&response);
            super::widgets::press_started_by_primary(&response)
        };
        let dragging = by_left && (response.dragged() || response.is_pointer_button_down_on());
        let handle = self.handle_rect(*offset);
        if let Some(pointer) = response.interact_pointer_pos().filter(|_| by_left) {
            // 押した位置: つまみの上ならそのままつかみ、溝ならつまみの中心がポインタに来る所へ移して掴む
            let start = ui
                .data(|d| d.get_temp::<f32>(grab_id(id)))
                .unwrap_or_else(|| {
                    if handle.contains(pointer) {
                        pointer.y - handle.top()
                    } else {
                        let travel_end = self.view.bottom() - handle.height();
                        let top =
                            (pointer.y - handle.height() / 2.0).clamp(self.view.top(), travel_end);
                        pointer.y - top
                    }
                });
            ui.data_mut(|d| d.insert_temp(grab_id(id), start));
            let top = pointer.y - start;
            let travel = self.view.top()..=(self.view.bottom() - handle.height());
            *offset = if travel.start() >= travel.end() {
                0.0
            } else {
                remap(top, travel, 0.0..=self.max)
            };
            *offset = offset.clamp(0.0, self.max);
        } else {
            ui.data_mut(|d| d.remove::<f32>(grab_id(id)));
        }
        // 右ボタンを押したまま動かす: 毎フレームのポインタの縦の動きの分、逆向きにずらし、範囲に収める
        // （ポインタを上へ 50 動かすと `offset` が 50 増え、下の行が見えてくる）。名乗るのは見えている範囲だけ
        if pan {
            if let Some(drag) = right_drag::drag(ui, id, self.view.intersect(ui.clip_rect())) {
                *offset = (*offset - drag.delta.y).clamp(0.0, self.max);
                ui.ctx().request_repaint();
            }
        }
        // 見た目: 乗せる・掴むと広がる（egui の `ScrollArea` の thin と同じ）
        let near = response.hovered() || dragging;
        let grow = ui.ctx().animate_bool_responsive(id.with("grow"), near);
        let width = lerp(THIN_WIDTH..=BAR_WIDTH, grow);
        let handle = self.handle_rect(*offset);
        let over_handle = response.hovered()
            && ui.input(|i| i.pointer.latest_pos().is_some_and(|p| handle.contains(p)));
        let color = if dragging {
            HANDLE_ACTIVE
        } else if over_handle {
            HANDLE_HOVER
        } else {
            HANDLE
        };
        let painter = ui
            .painter()
            .with_clip_rect(self.view.intersect(ui.clip_rect()));
        let shown = |r: Rect| {
            Rect::from_min_max(
                pos2(grab.right() - width, r.top()),
                pos2(grab.right(), r.bottom()),
            )
        };
        if grow > 0.0 {
            // 溝は、広がっているあいだだけ（普段は行の地のまま）
            painter.rect_filled(
                shown(Rect::from_min_max(grab.min, grab.max)),
                2.0,
                t::CONTROL_BG.gamma_multiply(0.85 * grow),
            );
        }
        painter.rect_filled(shown(handle), (width / 2.0).min(3.0), color);
        if dragging {
            ui.ctx().request_repaint();
        }
        dragging
    }
}

/// egui の `ScrollArea` の一覧に、右ボタンを押したまま動かす操作を効かせる（`show` の戻り値を渡す）。持ち主の決め方と動きの式は
/// [`Scroll::end`] と同じ。ずらした量は次のフレームから効く（`ScrollArea` の記憶を書き換える）。
pub fn pan_area<R>(ui: &Ui, out: &egui::scroll_area::ScrollAreaOutput<R>) {
    let max = out.content_size.y - out.inner_rect.height();
    if max <= 0.0 {
        return;
    }
    if let Some(drag) = right_drag::drag(ui, out.id, out.inner_rect.intersect(ui.clip_rect())) {
        let mut state = out.state;
        let next = (state.offset.y - drag.delta.y).clamp(0.0, max);
        if next != state.offset.y {
            state.offset.y = next;
            state.store(ui.ctx(), out.id);
        }
        ui.ctx().request_repaint();
    }
}
