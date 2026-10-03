//! 3D ビューの枠。中身は後で (a) wgpu の自前の 3D か、(b) 埋め込んだ Unity のプレイヤー（UaaL。Windows では子の窓の HWND を
//! このタブの矩形に合わせて動かす）を差し込む。今は空の枠と「3D ビュー（準備中）」の表示だけ。
//!
//! 外の窓へ知らせる口: `View3dHost` を渡すと、タブの中身の矩形（物理の画素、ネイティブの窓のクライアント領域の左上が原点）が
//! 決まった・変わったとき（`Placed`）、隠れたとき（別のタブの裏・閉じた。`Hidden`）、別の窓に出したとき（`Placed` の viewport・
//! floating が変わる）、メニューが重なった・外れたとき（`Covered`。子の窓は egui の絵より上に出るので、重なるあいだは隠すなど）に
//! 呼ばれる。入力は Rust の側で受け、タブの中のポインタ・ボタン・ホイールを `Input` で渡す（子の窓には入力を持たせない前提）。

use std::sync::{Arc, Mutex};

use egui::{pos2, vec2, Modifiers, Order, Rect, Sense, Ui, ViewportId};

use crate::canvas::HEADER_HEIGHT;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};

/// タブの中身の置き場所。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub viewport: ViewportId,
    /// x・y・幅・高さ（物理の画素）。
    pub rect_px: [i32; 4],
    pub pixels_per_point: f32,
    /// ドックから外して浮かせた窓の中か。
    pub floating: bool,
}

/// タブの中の入力（位置は中身の左上からの物理の画素）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View3dInput {
    pub pos: [f32; 2],
    pub primary: bool,
    pub secondary: bool,
    pub middle: bool,
    pub scroll: [f32; 2],
    pub modifiers: Modifiers,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum View3dEvent {
    Placed(Placement),
    Hidden,
    Covered(bool),
    Input(View3dInput),
}

/// 3D ビューの中身を描く外の窓（Unity のプレイヤーなど）が受ける口。
pub trait View3dHost {
    fn on_event(&mut self, event: &View3dEvent);
}

/// 受けた知らせを溜めるだけの口（試験・ログ用）。
#[derive(Clone, Default)]
pub struct RecordingHost {
    pub events: Arc<Mutex<Vec<View3dEvent>>>,
}

impl View3dHost for RecordingHost {
    fn on_event(&mut self, event: &View3dEvent) {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(*event);
    }
}

#[derive(Default)]
pub struct View3dSlot {
    host: Option<Box<dyn View3dHost>>,
    last: Option<Placement>,
    this_frame: Option<(Placement, Rect)>,
    covered: bool,
    last_input: Option<View3dInput>,
}

impl View3dSlot {
    pub fn set_host(&mut self, host: Box<dyn View3dHost>) {
        self.host = Some(host);
        // 新しい口には今の置き場所から知らせ直す
        self.last = None;
        self.covered = false;
    }

    fn emit(&mut self, event: View3dEvent) {
        if let Some(host) = &mut self.host {
            host.on_event(&event);
        }
    }

    pub fn begin_frame(&mut self) {
        self.this_frame = None;
    }

    /// 今の中身の矩形（画面の点。このフレームで描かれていなければ None）。
    pub fn content_rect(&self) -> Option<Rect> {
        self.this_frame.map(|(_, r)| r)
    }

    /// タブの中身を描き、置き場所と入力を記録する。
    pub fn show(&mut self, ui: &mut Ui) {
        let full = ui.max_rect();
        ui.advance_cursor_after_rect(full);
        let header = Rect::from_min_size(full.min, vec2(full.width(), HEADER_HEIGHT));
        let content = Rect::from_min_max(pos2(full.left(), header.bottom()), full.max);
        let p = ui.painter().clone();
        w::fill(&p, header, t::PANEL_HEADER);
        w::hline(
            &p,
            header.left(),
            header.right(),
            header.bottom() - 1.0,
            t::BORDER,
        );
        w::text(
            &p,
            Rect::from_min_max(pos2(header.left() + 8.0, header.top()), header.max),
            "3D · モデルなし",
            t::LABEL_DIM,
            Align::Left,
        );
        w::fill(&p, content, t::CANVAS_BG);
        let text = Rect::from_center_size(content.center(), vec2(content.width().min(360.0), 60.0));
        w::text(
            &p,
            Rect::from_min_size(text.min, vec2(text.width(), 22.0)),
            "3D ビュー（準備中）",
            t::LABEL.with_color(t::TEXT_DIM),
            Align::Center,
        );
        w::text(
            &p,
            Rect::from_min_size(
                pos2(text.left(), text.top() + 24.0),
                vec2(text.width(), 18.0),
            ),
            "Unity のプレイヤーか wgpu の 3D をここに差し込みます。",
            t::LABEL_SMALL,
            Align::Center,
        );

        let ppp = ui.ctx().pixels_per_point();
        let placement = Placement {
            viewport: ui.ctx().viewport_id(),
            rect_px: [
                (content.left() * ppp).round() as i32,
                (content.top() * ppp).round() as i32,
                (content.width() * ppp).round() as i32,
                (content.height() * ppp).round() as i32,
            ],
            pixels_per_point: ppp,
            floating: ui.layer_id().order != Order::Background,
        };
        self.this_frame = Some((placement, content));

        let response = ui.interact(content, ui.id().with("view3d"), Sense::click_and_drag());
        if response.contains_pointer() || response.dragged() {
            let input = ui.input(|i| {
                let at = i.pointer.hover_pos().unwrap_or(content.min) - content.min;
                View3dInput {
                    pos: [at.x * ppp, at.y * ppp],
                    primary: i.pointer.primary_down(),
                    secondary: i.pointer.secondary_down(),
                    middle: i.pointer.middle_down(),
                    scroll: [i.smooth_scroll_delta.x * ppp, i.smooth_scroll_delta.y * ppp],
                    modifiers: i.modifiers,
                }
            });
            if self.last_input != Some(input) {
                self.last_input = Some(input);
                self.emit(View3dEvent::Input(input));
            }
        }
    }

    /// フレームの終わり: 置き場所が変わった・隠れた・メニューが重なったを知らせる。popup はこのフレームのポップアップの矩形。
    pub fn end_frame(&mut self, popup: Option<Rect>) {
        let now = self.this_frame.map(|(p, _)| p);
        if now != self.last {
            match now {
                Some(p) => self.emit(View3dEvent::Placed(p)),
                None => self.emit(View3dEvent::Hidden),
            }
            self.last = now;
        }
        let covered = match (self.this_frame, popup) {
            (Some((_, content)), Some(popup)) => content.intersects(popup),
            _ => false,
        };
        if covered != self.covered {
            self.covered = covered;
            self.emit(View3dEvent::Covered(covered));
        }
    }
}
