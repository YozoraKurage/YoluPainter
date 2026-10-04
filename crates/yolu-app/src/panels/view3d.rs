//! 3D ビューのタブ: 自前の wgpu の 3D（`crate::view3d`。モデル・カメラ・面に描く）と、後で差し込む外の窓（埋め込んだ Unity の
//! プレイヤー（UaaL）。Windows では子の窓の HWND をこのタブの矩形に合わせて動かす）への知らせ。
//!
//! 外の窓へ知らせる口: `View3dHost` を渡すと、タブの中身の矩形（物理の画素、ネイティブの窓のクライアント領域の左上が原点）が
//! 決まった・変わったとき（`Placed`）、隠れたとき（別のタブの裏・閉じた。`Hidden`）、別の窓に出したとき（`Placed` の viewport・
//! floating が変わる）、メニューが重なった・外れたとき（`Covered`。子の窓は egui の絵より上に出るので、重なるあいだは隠すなど）に
//! 呼ばれる。入力は Rust の側で受け、タブの中のポインタ・ボタン・ホイールを `Input` で渡す（子の窓には入力を持たせない前提）。

use std::sync::{Arc, Mutex};

use egui::{pos2, vec2, Color32, CursorIcon, Modifiers, Order, Rect, Sense, Ui, ViewportId};

use crate::canvas::HEADER_HEIGHT;
use crate::pen::PenSample;
use crate::state::{Action, AppState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};
use crate::view3d::{gizmo, input, render::View3dRenderer};

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

    /// タブの中身を描き、置き場所と入力を記録する。renderer は wgpu の 3D（無ければ 3D を描けないと出す）。
    pub fn show(
        &mut self,
        ui: &mut Ui,
        app: &mut AppState,
        renderer: Option<&mut View3dRenderer>,
        pen: &[PenSample],
    ) {
        let full = ui.max_rect();
        ui.advance_cursor_after_rect(full);
        let header = Rect::from_min_size(full.min, vec2(full.width(), HEADER_HEIGHT));
        let below = Rect::from_min_max(pos2(full.left(), header.bottom()), full.max);
        // スキンのあるモデルなら左にポーズの欄（無ければ全部が 3D の表示域）
        let (pose_panel, content) = crate::panels::pose::split(app, below);
        let response = ui.interact(content, ui.id().with("view3d"), Sense::click_and_drag());
        let ppp = ui.ctx().pixels_per_point();
        input::handle(ui, app, content, pen);

        let p = ui.painter().clone();
        let drawn = match (app.view3d.model.clone(), renderer) {
            (Some(model), Some(renderer)) => {
                let size = [
                    (content.width() * ppp).round().max(1.0) as u32,
                    (content.height() * ppp).round().max(1.0) as u32,
                ];
                let id = renderer.prepare(
                    &app.doc,
                    &model,
                    app.view3d.material,
                    &app.view3d.camera,
                    size,
                );
                p.image(
                    id,
                    content,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                true
            }
            _ => false,
        };
        if !drawn {
            self.placeholder(ui, app, content);
        } else if app.view3d.pose.mode {
            // ポーズのモード: ギズモ（輪の上は掴む形のポインタ）
            let pointer = ui
                .input(|i| i.pointer.hover_pos())
                .filter(|p| response.contains_pointer() && content.contains(*p));
            gizmo::draw(ui, app, content, pointer);
            if pointer.is_some() {
                ui.ctx().set_cursor_icon(if app.view3d.input.nav.is_some() {
                    CursorIcon::Move
                } else if app.view3d.pose.drag.is_some() {
                    CursorIcon::Grabbing
                } else if app.view3d.pose.hover_axis.is_some() {
                    CursorIcon::Grab
                } else {
                    CursorIcon::Default
                });
            }
        } else if let Some(pointer) = ui.input(|i| i.pointer.hover_pos()) {
            // ブラシのカーソル（回している・パンしているあいだは出さない）
            if response.contains_pointer() && content.contains(pointer) {
                if app.view3d.input.nav.is_some() {
                    ui.ctx().set_cursor_icon(CursorIcon::Move);
                } else if input::draw_cursor(ui, app, content, pointer) {
                    ui.ctx().set_cursor_icon(CursorIcon::None);
                } else {
                    ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
                }
            }
        }
        if let Some(panel) = pose_panel {
            crate::panels::pose::show(ui, app, panel);
        }
        self.header(ui, app, header);

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

    fn header(&self, ui: &mut Ui, app: &mut AppState, bar: Rect) {
        let p = ui.painter().clone();
        w::fill(&p, bar, t::PANEL_HEADER);
        w::hline(&p, bar.left(), bar.right(), bar.bottom() - 1.0, t::BORDER);
        let lang = app.lang;
        let label = match &app.view3d.model {
            Some(m) => lang.pick(
                format!("3D · {} · {} 三角形", m.display_name(lang), m.triangle_count()),
                format!("3D · {} · {} triangles", m.display_name(lang), m.triangle_count()),
            ),
            None => lang.pick("3D · モデルなし", "3D · No model").to_string(),
        };
        w::text(
            &p,
            Rect::from_min_max(
                pos2(bar.left() + 8.0, bar.top()),
                pos2(bar.right() - 32.0, bar.bottom()),
            ),
            &label,
            t::LABEL_DIM,
            Align::Left,
        );
        if app.view3d.model.is_some() {
            let r =
                Rect::from_min_size(pos2(bar.right() - 28.0, bar.top() + 2.0), vec2(24.0, 22.0));
            if w::icon_button(
                ui,
                r,
                "view3d.frame",
                "target",
                lang.pick("モデル全体が見える位置へ戻す", "Fit the whole model in view"),
                false,
                !app.is_stroking(),
                16.0,
            )
            .clicked()
            {
                app.apply(Action::FrameModel);
            }
        }
    }

    /// モデルが無い・3D を描けないときの中身。
    fn placeholder(&self, ui: &mut Ui, app: &mut AppState, content: Rect) {
        let p = ui.painter().clone();
        w::fill(&p, content, t::CANVAS_BG);
        let text = Rect::from_center_size(content.center(), vec2(content.width().min(360.0), 88.0));
        // 名前・状態だけ（使い方の説明は置かない）
        let all_hidden = app.view3d.model.is_none() && app.view3d.full_model().is_some();
        let lang = app.lang;
        let title = if all_hidden {
            lang.pick("すべて隠しています", "All hidden")
        } else if app.view3d.model.is_none() {
            lang.pick("モデルなし", "No model")
        } else {
            lang.pick("3D を描けません（GPU なし）", "Cannot draw 3D (no GPU)")
        };
        w::text(
            &p,
            Rect::from_min_size(text.min, vec2(text.width(), 22.0)),
            title,
            t::LABEL.with_color(t::TEXT_DIM),
            Align::Center,
        );
        if app.view3d.full_model().is_none() {
            let r =
                Rect::from_center_size(pos2(text.center().x, text.top() + 64.0), vec2(160.0, 24.0));
            if w::button(
                ui,
                r,
                "view3d.demo",
                lang.pick("試しの立方体を読む", "Load Test Cube"),
                true,
                true,
                None,
                Some("view_in_ar"),
            )
            .clicked()
            {
                app.apply(Action::LoadDemoModel);
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
