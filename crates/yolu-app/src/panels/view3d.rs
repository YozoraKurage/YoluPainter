//! 3D ビューのタブ: 自前の wgpu の 3D（`crate::view3d`。モデル・カメラ・面に描く）と、後で差し込む別ウィンドウ（埋め込んだ Unity の
//! プレイヤー（UaaL）。Windows では子ウィンドウの HWND をこのタブの矩形に合わせて動かす）への知らせ。
//!
//! 別ウィンドウへ知らせる口: `View3dHost` を渡すと、タブの中身の矩形（物理の画素、ネイティブのウィンドウのクライアント領域の左上が原点）が
//! 決まった・変わったとき（`Placed`）、隠れたとき（別のタブの裏・閉じた。`Hidden`）、別のウィンドウに出したとき（`Placed` の viewport・
//! floating が変わる）、メニューが重なった・外れたとき（`Covered`。子ウィンドウは egui の絵より上に出るので、重なるあいだは隠すなど）に
//! 呼ばれる。入力は Rust の側で受け、タブの中のポインタ・ボタン・ホイールを `Input` で渡す（子ウィンドウには入力を持たせない前提）。

use std::sync::{Arc, Mutex};

use egui::{pos2, vec2, Color32, CursorIcon, Modifiers, Order, Rect, Sense, Ui, ViewportId};

use crate::mode::EditorMode;
use crate::pen::PenSample;
use crate::state::{Action, AppState, OpenPopup, PopupKind};
use crate::tools::input::Surface;
use crate::ui::menu::PopupState;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, NumberFormat, Rows, SliderSpec};
use crate::view3d::brdf::Curve;
use crate::view3d::display::{self, EnvKind, Op, SettingsTab, Shading};
use crate::view3d::selection_overlay::Skip;
use crate::view3d::{gizmo, input, render::View3dRenderer, views, ViewId};

/// タブの中身の置き場所。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub viewport: ViewportId,
    /// x・y・幅・高さ（物理の画素）。
    pub rect_px: [i32; 4],
    pub pixels_per_point: f32,
    /// ドックから外して浮かせたウィンドウの中か。
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

/// 3D ビューの中身を描く別ウィンドウ（Unity のプレイヤーなど）が受ける口。
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

/// 今描いているビューの表示域と押しの受け方。
#[derive(Clone, Copy)]
struct Area<'a> {
    content: Rect,
    response: &'a egui::Response,
    /// 3D の画像を描けたか
    drawn: bool,
    /// 今のビュー（入力を受ける）か
    live: bool,
}

/// 3D の画像を描いた結果。
struct Rendered {
    drawn: bool,
    /// 3D の画像が元より縮んでいるとき（縮めた段、予算で決まったか）。メッシュマップの表示ではそのマップの縮め
    reduced: Option<(u32, Option<bool>)>,
    /// 選択範囲の重ねが 3D の画像の予算に入らずに出せていないか
    overlay_skip: Option<Skip>,
}

/// 焼いたメッシュマップだけを見せているのに、どのセットにもそのマップが無ければ（焼き直して消えた・プロジェクトを替えた）、
/// ユーザーチャンネルだけを見せているのに、今のセットの文書にそのチャンネルが無ければ（セットを替えた・消した）マテリアルへ戻す。
fn restore_missing_shading(app: &mut AppState) {
    let missing = match app.view3d.view.display.shading {
        Shading::MeshMap(kind) => !views::mesh_map_available(app, kind),
        Shading::Channel(c) => !c.is_standard() && app.session.doc.channel_info(c).is_none(),
        _ => false,
    };
    if missing {
        app.apply(Action::View3d(Op::Shading(Shading::Material)));
    }
    // 3D の画像が元より縮んでいるとき（縮めた段、予算で決まったか）。メッシュマップの表示ではそのマップの縮め
}

/// 今のビューの 3D の画像を描いて、表示域に貼る。
fn draw_image(
    ui: &mut Ui,
    app: &mut AppState,
    renderer: Option<&mut View3dRenderer>,
    content: Rect,
) -> Rendered {
    let view = app.view3d.active_view();
    let ppp = ui.ctx().pixels_per_point();
    let p = ui.painter().clone();
    let mut reduced: Option<(u32, Option<bool>)> = None;
    let mut overlay_skip: Option<Skip> = None;
    let drawn = match (app.view3d.model.clone(), renderer) {
        (Some(model), Some(renderer)) => {
            // 選択範囲の重ね（縁・クイックマスクの赤・選択ペンの被覆）
            renderer.set_overlay(crate::view3d::selection_overlay::input(app, ui.ctx()));
            let size = [
                (content.width() * ppp).round().max(1.0) as u32,
                (content.height() * ppp).round().max(1.0) as u32,
            ];
            let maps = match app.view3d.view.display.shading {
                Shading::MeshMap(kind) => views::mesh_map_sources(app, kind),
                _ => Vec::new(),
            };
            let others = crate::view3d::other_sets::collect(app);
            renderer.select_view(view.0);
            renderer.set_shape_source(app.view3d.frame_models().1);
            let id = renderer.prepare_sets(
                &app.session.doc,
                &model,
                &app.view3d.material,
                &app.view3d.view.camera,
                size,
                &app.view3d.view.display,
                &maps,
                &others,
            );
            drop(others);
            drop(maps);
            crate::view3d::other_sets::finish(app, renderer.unpainted_materials());
            if renderer.wants_repaint() {
                // 法線マップの接線を作っている最中: 出来たら描き直す
                ui.ctx().request_repaint();
            }
            let stats = renderer.stats;
            overlay_skip = stats.overlay_skipped.then_some(if stats.overlay_too_large {
                Skip::Size
            } else {
                Skip::Budget
            });
            // このフレームに塗ったダブの分の、表示の同期の時間（次のフレームから、枠のうち塗りに使う割合と、溜まった仕事の見込みに使う）
            app.view3d
                .view
                .input
                .note_sync(std::time::Duration::from_micros(stats.last_sync_us));
            app.view3d.view.display.drawn_samples = stats.samples;
            reduced = match app.view3d.view.display.shading {
                Shading::MeshMap(_) => (stats.map_level > 0).then_some((stats.map_level, None)),
                _ => (stats.paint_level > 0)
                    .then_some((stats.paint_level, Some(stats.paint_by_budget))),
            };
            p.image(
                id,
                content,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            true
        }
        _ => {
            crate::view3d::other_sets::finish(app, &[]);
            false
        }
    };
    Rendered {
        drawn,
        reduced,
        overlay_skip,
    }
}

/// 3D の画像の上の、ツールの重ね（ステンシル・定規・クローンの元・ドラッグの途中の形・ギズモ・デカールの落とし先）。形のギズモのハンドルの上
/// なら、そのポインタの形を返す。
fn tool_overlays(ui: &mut Ui, app: &mut AppState, area: &Area) -> Option<CursorIcon> {
    let Area {
        content,
        response,
        drawn,
        live,
    } = *area;
    // ステンシル（3D の画像の上に、画面に貼り付いた半透明の画像。編集・ポーズのモードでは描かないので出さない）
    let mode = app.mode;
    if drawn && mode.paints() && !live {
        // 今のビューでないビュー: 3D の定規（対称の面と軸）とクローンの元だけ（面の上の物。画面に貼り付いたステンシル・引いている途中の形は今のビューだけ）
        input::draw_overlays(ui, app, content);
    }
    if drawn && mode.paints() && live {
        crate::stencil::draw_overlay(&ui.painter_at(content), &mut app.stencil, content);
        // 3D の定規（対称の面と軸を含む）・クローンの元（ステンシルの上、ブラシのカーソルの下）
        input::draw_overlays(ui, app, content);
        // グラデーション・図形・定規のドラッグの途中の形
        crate::view3d::draft::paint_overlay(&ui.painter_at(content), app, content);
        // 選択のツールで引いている形と、打っている多角形の点（ゴムの線はポインタまで）
        let hover = ui
            .input(|i| i.pointer.hover_pos())
            .filter(|p| response.contains_pointer() && content.contains(*p));
        crate::view3d::select::paint_overlay(&ui.painter_at(content), app, content, hover);
    }
    // 塗りつぶしレイヤーの置き場・形のギズモと、ライブラリの画像のデカールの落とし先（3D の画像の上）
    let mut gizmo_cursor = None;
    if drawn && mode == EditorMode::Hide && live {
        // ハイドのモード: ポインタの下の UV アイランド（引き出し・シェルフの上は外す）と、選んだ・指したアイランド・隠した面の輪郭
        let pointer = ui
            .input(|i| i.pointer.hover_pos())
            .filter(|p| response.contains_pointer() && content.contains(*p))
            .filter(|p| crate::input::on_top(ui, content, *p));
        crate::view3d::hide::input::hover(app, content, pointer);
        crate::view3d::hide::overlay::draw(&ui.painter_at(content), app, content);
    }
    if drawn && mode == EditorMode::Hide {
        crate::view3d::hide::overlay::frame(&ui.painter_at(content), content);
    }
    if drawn {
        // ハイドとポーズのモード: ボーンの丸（どのビューにも出す）
        let pointer = ui
            .input(|i| i.pointer.hover_pos())
            .filter(|p| live && response.contains_pointer() && content.contains(*p));
        crate::view3d::bone_marks::draw(ui, app, content, pointer);
    }
    if drawn && live && mode != EditorMode::Pose && mode != EditorMode::Hide {
        let pointer = ui
            .input(|i| i.pointer.hover_pos())
            .filter(|p| response.contains_pointer() && content.contains(*p));
        crate::fillfx::gizmo::draw(ui, app, content, pointer);
        if mode == EditorMode::Edit {
            // 編集のモード: 選べる物の点の印と、選んだ物の枠・線
            crate::objects::draw(ui, app, content, pointer);
        } else {
            crate::fillfx::points::draw(ui, app, content, pointer);
        }
        // ライブラリの画像を落としてデカールを置く（編集のモードでは、置いたデカールを選ぶ）
        crate::fillfx::decal_drop(ui, app, content);
        gizmo_cursor = pointer.and_then(|_| crate::fillfx::gizmo::cursor(app));
    }
    gizmo_cursor
}

/// ブラシのカーソル（回している・パンしているあいだは出さない。ポインタが表示域の上にあるとき）。
fn brush_cursor(ui: &mut Ui, app: &mut AppState, content: Rect, pointer: egui::Pos2) {
    // 2D の対称の写しのカーソルは、今のセットの UV の格子で引く（無ければここで作る）
    if app.canvas_symmetry().enabled() {
        let _ = app.region_grid();
    }
    if let Some(zoom) = app.view3d.view.input.zoom {
        ui.ctx()
            .set_cursor_icon(crate::canvas::zoom_cursor(zoom.out));
    } else if app.view3d.view.input.nav.is_some() {
        ui.ctx().set_cursor_icon(CursorIcon::Move);
    } else if let Some(out) = zoom_chord_held(ui) {
        // Ctrl+Space を押している: 虫めがね（Alt も押していれば縮小）
        ui.ctx().set_cursor_icon(crate::canvas::zoom_cursor(out));
    } else if let Some(icon) = crate::stencil::cursor_icon(&app.stencil) {
        ui.ctx().set_cursor_icon(icon);
    } else if input::draw_cursor(ui, app, content, pointer) {
        ui.ctx().set_cursor_icon(CursorIcon::None);
    } else {
        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
    }
}

/// 隅のアイコンの結果。
#[derive(Default)]
struct Corner {
    /// 設定のアイコンの矩形（設定のパネルは、その外を押すと閉じる。このアイコンは外に数えない）。
    settings: Option<Rect>,
    /// アイコンの列の下の端（軸の印をその下に置く）。
    bottom: f32,
}

/// 3D ビューのタブの中身の置き場所（ビューごとに 1 つ）。別ウィンドウへ知らせる口（`View3dHost`）は最初のビューだけが持つ。
#[derive(Default)]
pub struct View3dSlots {
    slots: std::collections::BTreeMap<ViewId, View3dSlot>,
}

impl View3dSlots {
    /// 最初のビューの置き場所。
    pub fn first_mut(&mut self) -> &mut View3dSlot {
        self.slot_mut(ViewId::FIRST)
    }

    /// このビューの置き場所（無ければ作る）。
    pub fn slot_mut(&mut self, id: ViewId) -> &mut View3dSlot {
        self.slots.entry(id).or_default()
    }

    pub fn set_host(&mut self, host: Box<dyn View3dHost>) {
        self.first_mut().set_host(host);
    }

    pub fn begin_frame(&mut self) {
        for slot in self.slots.values_mut() {
            slot.begin_frame();
        }
    }

    /// このビューの今の中身の矩形（画面の点。このフレームで描かれていなければ None）。
    pub fn content_rect(&self, id: ViewId) -> Option<Rect> {
        self.slots.get(&id).and_then(View3dSlot::content_rect)
    }

    /// どれかのビューをこのフレームで描いたか。
    pub fn any_drawn(&self) -> bool {
        self.slots.values().any(|s| s.content_rect().is_some())
    }

    /// フレームの終わり（`View3dSlot::end_frame`）。ポップアップの重なりは、口を持つ最初のビューだけが知らせる。
    pub fn end_frame(&mut self, popup: Option<Rect>) {
        for (id, slot) in &mut self.slots {
            slot.end_frame(popup.filter(|_| *id == ViewId::FIRST));
        }
    }

    /// 無くなったビューの置き場所を捨てる（最初のビューは残す）。
    pub fn retain(&mut self, keep: impl Fn(ViewId) -> bool) {
        self.slots.retain(|id, _| *id == ViewId::FIRST || keep(*id));
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

    /// ビュー `view` のタブの中身を描き、置き場所と入力を記録する。renderer は wgpu の 3D（無ければ 3D を描けないと出す）。
    /// ポインタが乗っていて今のビューを替えてよければ、このビューを今のビューにする（`views::may_activate`）。今のビューは入力を受け、
    /// 今のビューでないビューは、一時的に今のビューの場所へ入れ替えて描くだけ（入力・ブラシのカーソル・ツールの重ねは出さない）。
    pub fn show(
        &mut self,
        ui: &mut Ui,
        app: &mut AppState,
        renderer: Option<&mut View3dRenderer>,
        pen: &[PenSample],
        view: ViewId,
    ) {
        let full = ui.max_rect();
        ui.advance_cursor_after_rect(full);
        if views::may_activate(app, ui.ctx(), view, full) {
            self.show_view(ui, app, renderer, pen, full, true);
        } else {
            views::with_view(app, view, |app| {
                self.show_view(ui, app, renderer, &[], full, false)
            });
        }
    }

    /// 今のビューに入っているビューを描く。`live` なら入力を受ける（今のビュー）。
    fn show_view(
        &mut self,
        ui: &mut Ui,
        app: &mut AppState,
        renderer: Option<&mut View3dRenderer>,
        pen: &[PenSample],
        content: Rect,
        live: bool,
    ) {
        app.view3d.view.view_rect = Some(content);
        let sense = if live {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        };
        let response = ui.interact(content, ui.id().with("view3d"), sense);
        if live {
            app.ui.view3d_viewport = Some(ui.ctx().viewport_id());
            crate::objects::transform::settle(app);
            app.rulers
                .note_pointer(crate::rulers::Place::View3d, response.contains_pointer());
            input::handle(
                ui,
                app,
                content,
                pen,
                crate::gesture::foreign_press(ui.ctx(), &response),
            );
        }

        restore_missing_shading(app);
        let Rendered {
            drawn,
            reduced,
            overlay_skip,
        } = draw_image(ui, app, renderer, content);
        let area = Area {
            content,
            response: &response,
            drawn,
            live,
        };
        let gizmo_cursor = tool_overlays(ui, app, &area);
        self.show_cursor(ui, app, &area, gizmo_cursor);
        // 描いている間かは、角のアイコンと引き出しで 1 回だけ求める（このフレームの入力を当てた後）
        let free = !app.is_stroking();
        let corner = self.corner(
            ui,
            app,
            content,
            reduced.filter(|_| drawn),
            overlay_skip.filter(|_| drawn),
            live,
            free,
        );
        // ハイドのモードの右の引き出しと、左下のシェルフ（どのビューにも。押せるのは今のビューのもの）。設定のパネルが開いている間は、
        // 同じ右側に重なるので、引き出し（つまみも）を描かない。UI を隠している間（Tab）は、設定のパネル・軸の印・引き出し・シェルフを出さない
        let hide_ui = app.ui.hide_ui;
        let settings = !hide_ui && app.view3d.view.display.settings_open && live;
        let drawer_open = drawn && app.mode == EditorMode::Hide && app.view3d.view.hide.drawer_open;
        if drawn && !hide_ui {
            if !settings {
                crate::panels::hide_drawer::show(ui, app, content, corner.bottom + 4.0, live, free);
            }
            crate::view3d::hide::shelf::show(ui, app, content, live);
        }
        if hide_ui {
            // UI を隠している間は、設定のパネルも軸の印も出さない
        } else if settings {
            settings_panel(ui, app, content, corner.settings);
        } else if drawn && !drawer_open {
            // 軸の印（アイコンの下。設定のパネル・ハイドの引き出しと重なるので、開いている間は出さない）
            crate::view3d::axis_gizmo::show(ui, app, content, corner.bottom, live);
        }
        // ビューが 2 つ以上あるとき、今のビュー（キーの視点の操作が効くビュー）の縁に細い線
        if live && drawn && app.view3d.view_count() > 1 {
            w::outline(
                &ui.painter_at(content),
                content.shrink(0.75),
                t::ACCENT,
                1.5,
                0.0,
            );
        }
        self.publish(ui, content, &response);
    }

    /// 置き場所を記録し、ポインタがビューの上にあるときの入力を別ウィンドウへ知らせる。
    fn publish(&mut self, ui: &mut Ui, content: Rect, response: &egui::Response) {
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

    /// ポインタのカーソルとツールの印（回している・ツールごとの形。3D の画像が無いときは置き場の文字）。
    fn show_cursor(
        &self,
        ui: &mut Ui,
        app: &mut AppState,
        area: &Area,
        gizmo_cursor: Option<CursorIcon>,
    ) {
        let Area {
            content,
            response,
            drawn,
            live,
        } = *area;
        let mode = app.mode;
        if !drawn {
            self.placeholder(ui, app, content);
        } else if !live {
            // 今のビューでないビューには、カーソル・ギズモを出さない（ポインタが乗れば今のビューになる）
        } else if let Some(press) = app.view3d.view.input.eyedrop {
            // 右ボタンを押して、まだ動かしていない: 押したときの見本の輪とスポイトの絵（動かせば回すだけ。OS の矢印は隠す）
            crate::eyedrop_mark::paint(
                &ui.painter_at(content),
                press.at,
                crate::eyedrop::current_swatch(app),
                press.sample,
            );
            ui.ctx().set_cursor_icon(CursorIcon::None);
        } else if mode == EditorMode::Pose {
            // ポーズのモード: ギズモ（輪の上は掴む形のポインタ。G/R/S の途中は出さない）
            let pointer = ui
                .input(|i| i.pointer.hover_pos())
                .filter(|p| response.contains_pointer() && content.contains(*p));
            if !crate::objects::transforming(app) {
                gizmo::draw(ui, app, content, pointer);
            }
            if pointer.is_some() {
                ui.ctx()
                    .set_cursor_icon(if app.view3d.view.input.nav.is_some() {
                        CursorIcon::Move
                    } else if app.view3d.pose.drag.is_some() {
                        CursorIcon::Grabbing
                    } else if app.view3d.pose.hover_axis.is_some() {
                        CursorIcon::Grab
                    } else {
                        CursorIcon::Default
                    });
            }
        } else if let Some(icon) = gizmo_cursor {
            // 形のギズモのハンドルの上（ブラシの円は出さない）
            ui.ctx().set_cursor_icon(icon);
        } else if matches!(mode, EditorMode::Edit | EditorMode::Hide) {
            // 編集・ハイドのモード: 描かないので、ブラシの円もツールの印も出さない（ハイドでは形のギズモも出ない）
            if ui
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|p| response.contains_pointer() && content.contains(p))
            {
                ui.ctx()
                    .set_cursor_icon(if app.view3d.view.input.nav.is_some() {
                        CursorIcon::Move
                    } else {
                        CursorIcon::Default
                    });
            }
        } else if app.tool.def().surface == Surface::Path {
            // パスのツール: 選んでいるレイヤーのパスの線と点を重ねる（ブラシの円は出さない）
            let pointer = ui
                .input(|i| i.pointer.hover_pos())
                .filter(|p| response.contains_pointer() && content.contains(*p));
            crate::pathtool::surface::paint_overlay(&ui.painter_at(content), app, content, pointer);
            if let Some(p) = pointer {
                ui.ctx()
                    .set_cursor_icon(if app.view3d.view.input.nav.is_some() {
                        CursorIcon::Move
                    } else {
                        crate::pathtool::surface::cursor_icon(app, content, p)
                    });
            }
        } else if app.tool.def().surface == Surface::Region {
            // 範囲のツール: ポインタの下の範囲の面を薄い色で重ねる（ブラシの円は出さない）
            let pointer = ui
                .input(|i| i.pointer.hover_pos())
                .filter(|p| response.contains_pointer() && content.contains(*p));
            let painter = ui.painter_at(content);
            crate::region::overlay::paint_surface(&painter, app, content, pointer);
            if pointer.is_some() {
                ui.ctx()
                    .set_cursor_icon(if app.view3d.view.input.nav.is_some() {
                        CursorIcon::Move
                    } else {
                        CursorIcon::Crosshair
                    });
            }
        } else if app.tool.def().surface == Surface::Screen {
            // グラデーション・図形・定規: 画面の上で引くので十字（ブラシの円は出さない）
            if ui
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|p| response.contains_pointer() && content.contains(p))
            {
                ui.ctx()
                    .set_cursor_icon(if app.view3d.view.input.nav.is_some() {
                        CursorIcon::Move
                    } else {
                        CursorIcon::Crosshair
                    });
            }
        } else if app.tool.def().surface == Surface::Pick {
            // スポイト: ブラシの円は出さない。回しているあいだは回すポインタ、ほかは見本の輪とスポイトの絵（OS の矢印は隠す）
            let pointer = ui
                .input(|i| i.pointer.hover_pos())
                .filter(|p| response.contains_pointer() && content.contains(*p));
            if let Some(p) = pointer {
                if app.view3d.view.input.nav.is_some() {
                    ui.ctx().set_cursor_icon(CursorIcon::Move);
                } else {
                    let sample = crate::eyedrop::sample_surface(app, content, p);
                    crate::eyedrop_mark::paint(
                        &ui.painter_at(content),
                        p,
                        crate::eyedrop::current_swatch(app),
                        sample,
                    );
                    ui.ctx().set_cursor_icon(CursorIcon::None);
                }
            }
        } else if let Some(pointer) = ui.input(|i| i.pointer.hover_pos()) {
            // ブラシのカーソル（回している・パンしているあいだは出さない）
            if response.contains_pointer() && content.contains(pointer) {
                brush_cursor(ui, app, content, pointer);
            }
        }
    }

    /// 表示域の右上の隅に重ねる小さなアイコン（見出しの帯は置かない。文字なし、名前と理由はツールチップ）: 3D の絵が元より縮んでいる印
    /// （押せない。`reduced` は縮めた段と、メモリの予算で決まったか）、光と環境の設定、モデル全体が見える位置へ戻す。左上の隅には表示の種類の
    /// ドロップダウン（押すと、その下に表示の種類のメニュー）。返すのは設定のアイコンの矩形（設定のパネルを下に置く）。
    #[allow(clippy::too_many_arguments)]
    fn corner(
        &self,
        ui: &mut Ui,
        app: &mut AppState,
        view: Rect,
        reduced: Option<(u32, Option<bool>)>,
        overlay_skip: Option<Skip>,
        live: bool,
        free: bool,
    ) -> Corner {
        if app.view3d.model.is_none() || app.ui.hide_ui {
            return Corner::default();
        }
        let lang = app.lang;
        let id = app.view3d.active_view();
        let mut items = Vec::new();
        if let Some((level, by_budget)) = reduced {
            items.push(
                w::CornerIcon::new(
                    "reduced",
                    "warning",
                    reduced_tooltip(lang, level, by_budget),
                )
                .indicator()
                .color(t::WARNING),
            );
        }
        if let Some(skip) = overlay_skip {
            items.push(
                w::CornerIcon::new(
                    "selection-hidden",
                    "warning",
                    overlay_skipped_tooltip(lang, skip),
                )
                .indicator()
                .color(t::WARNING),
            );
        }
        items.push(
            w::CornerIcon::new(
                "settings",
                "light_mode",
                lang.t("panels-light-environment-tone-mapping-tip"),
            )
            .selected(app.view3d.view.display.settings_open),
        );
        items.push(
            w::CornerIcon::new(
                "frame",
                "target",
                lang.t("panels-fit-the-whole-model-in-tip"),
            )
            .enabled(free),
        );
        let out = w::corner_icons(ui, ("view3d", id.0), view, &items);
        // 今のビューでないビューの押しは受けない（ポインタが乗れば今のビューになる。替えられない間 = 描いている途中などは、そのまま）
        if let Some(i) = out.clicked.filter(|_| live) {
            match items[i].id {
                "settings" => app.apply(Action::View3d(Op::ToggleSettings)),
                "frame" => app.apply(Action::FrameModel),
                _ => {}
            }
        }
        shading_dropdown(ui, app, view, out.rects.first().map(|r| r.left()), live);
        Corner {
            settings: items
                .iter()
                .position(|item| item.id == "settings")
                .and_then(|i| out.rects.get(i).copied()),
            bottom: w::corner_bottom(view, items.len()),
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
        // モデルが無いだけのときは、空の状態の文字（「モデルなし」）を置かず、読むボタンだけにする
        let title = if all_hidden {
            Some(lang.t("panels-all-hidden"))
        } else if app.view3d.model.is_none() {
            None
        } else {
            Some(lang.t("panels-cannot-draw-3d-no-gpu"))
        };
        if let Some(title) = title {
            w::text(
                &p,
                Rect::from_min_size(text.min, vec2(text.width(), 22.0)),
                title,
                t::LABEL.with_color(t::TEXT_DIM),
                Align::Center,
            );
        }
        if app.view3d.full_model().is_none() {
            // モデルを開くのは新規プロジェクトを作ること。主の入り口は新規プロジェクトで、試しの立方体はその下に控えめに置く。
            // 2 つの縦並びを枠の真ん中に置き、枠が低くて入りきらないときは試しの立方体を出さない（主のボタンは枠の上端に寄せて切らさない）
            const NEW_H: f32 = 28.0;
            const CUBE_H: f32 = 22.0;
            const GAP: f32 = 8.0;
            const MARGIN: f32 = 4.0;
            let block = NEW_H + GAP + CUBE_H;
            let top = (content.center().y - block / 2.0).max(content.top() + MARGIN);
            let x = content.center().x;
            let width = |wanted: f32| wanted.min((content.width() - 2.0 * MARGIN).max(0.0));
            // メニューの新規プロジェクトと同じに、描いている間・保存の間は押せない（理由は保存の間だけ。描いている間はツールチップを見ない）
            let saving = app.is_saving();
            let idle = !app.is_stroking() && !saving;
            let tip = if saving {
                crate::lang::refusals::saving(lang).to_owned()
            } else {
                crate::shortcuts::tip_with_key(
                    lang,
                    lang.t("panels-new-project-tip"),
                    &Action::NewProjectDialog,
                )
            };
            let r = Rect::from_center_size(pos2(x, top + NEW_H / 2.0), vec2(width(180.0), NEW_H));
            if w::button(
                ui,
                r,
                "view3d.new_project",
                lang.t("panels-new-project"),
                true,
                idle,
                Some(&tip),
                Some("add"),
            )
            .clicked()
            {
                app.apply(Action::NewProjectDialog);
            }
            if top + block <= content.bottom() - MARGIN {
                let r = Rect::from_center_size(
                    pos2(x, top + NEW_H + GAP + CUBE_H / 2.0),
                    vec2(width(160.0), CUBE_H),
                );
                if w::button(
                    ui,
                    r,
                    "view3d.demo",
                    lang.t("panels-load-test-cube"),
                    false,
                    true,
                    None,
                    None,
                )
                .clicked()
                {
                    app.apply(Action::LoadDemoModel);
                }
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

/// 表示の種類のドロップダウンの幅の上限・下限（右上の隅のアイコンの列と軸の印にかからない幅）。
const SHADING_DROPDOWN_MAX: f32 = 250.0;
const SHADING_DROPDOWN_MIN: f32 = 110.0;

/// 表示域の左上の隅の、今のビューの表示の種類のドロップダウン（押すと、その下に表示の種類のメニュー。メニューはこのビューへ当てる）。
/// 別のレイヤー（`egui::Area`）に置くので、下のビューの入力（描く・回す）は、ドロップダウンの上では始まらない。`icons_left` は右上の
/// 隅のアイコンの列の左の端（そこまでに収める）。
fn shading_dropdown(
    ui: &mut Ui,
    app: &mut AppState,
    view: Rect,
    icons_left: Option<f32>,
    live: bool,
) {
    let id = app.view3d.active_view();
    let lang = app.lang;
    let right = icons_left.unwrap_or(view.right()) - w::CORNER_MARGIN;
    let width = (right - view.left() - w::CORNER_MARGIN)
        .min(SHADING_DROPDOWN_MAX)
        .max(SHADING_DROPDOWN_MIN.min(view.width() - 2.0 * w::CORNER_MARGIN));
    if width <= 0.0 {
        return;
    }
    let r = Rect::from_min_size(
        pos2(
            view.left() + w::CORNER_MARGIN,
            view.top() + w::CORNER_MARGIN,
        ),
        vec2(width, w::CORNER_ICON_SIZE),
    );
    let shading = display::shading_label(app, app.view3d.view.display.shading);
    let open = matches!(
        app.ui.popup.as_ref().map(|p| p.kind),
        Some(PopupKind::View3dShading)
    ) && app.view3d.shading_menu == id;
    let mut clicked = None;
    egui::Area::new(egui::Id::new(("yolu.view3d.shading", id.0)))
        .order(Order::Middle)
        .fixed_pos(r.min)
        .constrain(false)
        .show(ui.ctx(), |ui| {
            ui.allocate_exact_size(r.size(), Sense::hover());
            let (response, b) = w::dropdown(
                ui,
                r,
                "view3d.shading",
                None,
                &shading,
                Some(lang.t("panels-3d-shading-tip")),
                true,
                0.0,
            );
            if response.clicked() && !open {
                clicked = Some(b);
            }
        });
    if let Some(b) = clicked.filter(|_| live) {
        // 押した箱の下に開く
        let ctx = ui.ctx().clone();
        app.view3d.shading_menu = id;
        app.ui.popup = Some(OpenPopup {
            kind: PopupKind::View3dShading,
            state: PopupState::new(
                &ctx,
                Rect::from_min_size(pos2(b.left(), b.bottom() + 2.0), vec2(b.width(), 0.0)),
            ),
        });
    }
}

/// Ctrl+Space を押している（キーを打っているときを除く）なら、縮小（Alt も押している）かどうか。
fn zoom_chord_held(ui: &Ui) -> Option<bool> {
    if ui.ctx().egui_wants_keyboard_input() {
        return None;
    }
    ui.input(|i| {
        crate::gesture::zoom_chord(&i.modifiers, crate::keymap::hold_down(i, "view.pan_hold"))
            .then_some(i.modifiers.alt)
    })
}

/// 選択範囲の重ねを 3D に出していない印のツールチップ（何が（なぜ）の 1 文。理由は予算か、文書の大きさか）。
fn overlay_skipped_tooltip(lang: crate::lang::Lang, skip: Skip) -> String {
    let reason = match skip {
        Skip::Budget => lang.t("panels-it-does-not-fit-in-tip"),
        Skip::Size => lang.t("panels-the-canvas-is-wider-or-tip"),
    };
    lang.with_reason(lang.t("panels-selection-not-shown-in-3d-tip"), reason)
}

/// 3D の絵が元の大きさより縮んでいることの印のツールチップ（隅の警告のアイコン）: 縮めた段と短い理由。`by_budget` は縮めがメモリの予算で
/// 決まったか（None は分からない: メッシュマップ）。
fn reduced_tooltip(lang: crate::lang::Lang, level: u32, by_budget: Option<bool>) -> String {
    let text = lang.f("panels-reduced-1-tip", &[("level", &(1u64 << level))]);
    let reason = match by_budget {
        Some(true) => lang.t("panels-shown-smaller-in-3d-to-tip"),
        Some(false) => lang.t("panels-shown-smaller-in-3d-because-tip"),
        None => lang.t("panels-shown-smaller-to-stay-within-tip"),
    };
    format!("{text}: {reason}")
}

const PANEL_WIDTH: f32 = 252.0;
const PANEL_HEIGHT: f32 = 464.0;
/// 設定のパネルの「画質」の面の高さ（内容が少ないので低い）。
const QUALITY_HEIGHT: f32 = 204.0;

/// 設定のパネルの浮いた部品の鍵（ビューごとに別。パネルは今のビューにだけ出す）。
fn settings_area_id(app: &AppState) -> egui::Id {
    egui::Id::new(("view3d.settings.area", app.view3d.active_view().0))
}

/// 光・環境・トーンマッピングの設定（見出しの設定のボタンの下に浮かせる小さなパネル）。外を押すか Esc で閉じる。
fn settings_panel(ui: &mut Ui, app: &mut AppState, content: Rect, button: Option<Rect>) {
    let ctx = ui.ctx().clone();
    let lang = app.lang;
    let width = PANEL_WIDTH.min((content.width() - 8.0).max(120.0));
    // 隅のアイコンの列の下に置く（アイコンを隠さない）
    let pos = pos2(
        (content.right() - width - 6.0).max(content.left() + 4.0),
        w::corner_bottom(content, 3) + 4.0,
    );
    let height = if app.view3d.view.display.tab == SettingsTab::Quality {
        QUALITY_HEIGHT
    } else {
        PANEL_HEIGHT
    };
    let panel = Rect::from_min_size(pos, vec2(width, height));
    // Esc で閉じる浮いた部品として覚える（閉じたフレームも開いている扱い。キャンバスの Esc が先に選択を外さない）
    crate::ui::window::note_open(&ctx);
    egui::Area::new(settings_area_id(app))
        .order(Order::Foreground)
        .fixed_pos(pos)
        .show(&ctx, |ui| {
            // 下の 3D へ押しが通らないように、パネルの全体を受ける（部品はその上に描く）
            ui.allocate_rect(panel, Sense::click_and_drag());
            let p = ui.painter().clone();
            w::rounded(&p, panel, t::PANEL_BG, 4.0);
            w::outline(&p, panel, t::BORDER, 1.0, 4.0);
            let d = app.view3d.view.display;
            let mut rows = Rows::new(panel, 8.0);
            let tab = display::settings_tabs(ui, app, &mut rows, content);
            if tab == SettingsTab::Navigation {
                return;
            }
            let heading = |rows: &mut Rows, p: &egui::Painter, title: &str| {
                let r = rows.row(18.0, 4.0);
                w::text(
                    p,
                    r,
                    title,
                    t::LABEL_BOLD.with_color(t::TEXT_DIM),
                    Align::Left,
                );
            };
            let slider = |ui: &mut Ui,
                          app: &mut AppState,
                          rows: &mut Rows,
                          id: &str,
                          label: &str,
                          value: f32,
                          min: f32,
                          max: f32,
                          decimals: u8,
                          suffix: &str,
                          enabled: bool,
                          op: fn(f32) -> Op| {
                let r = rows.row(22.0, 4.0);
                let spec = SliderSpec::new(
                    label,
                    min,
                    max,
                    NumberFormat {
                        decimals,
                        trim: true,
                        suffix,
                    },
                )
                .enabled(enabled);
                let out = w::slider(ui, r, ("view3d.set", id), value, &spec);
                if out.changed {
                    app.apply(Action::View3d(op(out.value)));
                }
            };
            if tab == SettingsTab::Quality {
                // アンチエイリアス: 機材が使える数だけ押せる（使えない数は理由をツールチップに）
                heading(&mut rows, &p, lang.t("panels-anti-aliasing"));
                let r = rows.row(24.0, 4.0);
                let supported = d.supported_sample_counts();
                let shown = d.shown_samples();
                for (n, cell) in display::SAMPLE_CHOICES.iter().zip(Rows::split(r, 4, 4.0)) {
                    let usable = supported.contains(n);
                    let label = if *n == 1 {
                        lang.t("panels-off").to_owned()
                    } else {
                        format!("{n}×")
                    };
                    let tip = if usable {
                        format!("{} {label}", lang.t("panels-anti-aliasing-tip"))
                    } else {
                        lang.t("panels-not-supported-on-this-gpu-tip").to_owned()
                    };
                    if w::button(
                        ui,
                        cell,
                        ("view3d.set.aa", *n),
                        &label,
                        shown == *n,
                        usable,
                        Some(&tip),
                        None,
                    )
                    .clicked()
                    {
                        app.apply(Action::View3d(Op::Antialias(*n)));
                    }
                }
                // 選んだ数で描けていないとき（描き先のメモリの上限）は、描いている数を短く
                let r = rows.row(18.0, 4.0);
                if d.drawn_samples != 0 && d.drawn_samples < shown {
                    w::text(
                        ui.painter(),
                        r,
                        &lang.f(
                            "panels-drawing-at-memory-limit",
                            &[("drawn_samples", &d.drawn_samples)],
                        ),
                        t::LABEL_DIM.with_color(t::WARNING),
                        Align::Left,
                    );
                }
                rows.space(4.0);
                // ブルーム
                let r = rows.row(22.0, 4.0);
                let bloom = w::toggle(
                    ui,
                    r,
                    "view3d.set.bloom",
                    lang.t("panels-bloom"),
                    d.post.bloom,
                    None,
                    true,
                );
                if bloom != d.post.bloom {
                    app.apply(Action::View3d(Op::Bloom(bloom)));
                }
                let mut dragging = false;
                let off = (!d.post.bloom).then(|| lang.t("panels-bloom-is-off"));
                for (id, label, value, max, op) in [
                    (
                        "bloom_strength",
                        lang.t("panels-strength"),
                        d.post.bloom_strength,
                        display::BLOOM_STRENGTH_MAX,
                        Op::BloomStrength as fn(f32) -> Op,
                    ),
                    (
                        "bloom_threshold",
                        lang.t("panels-threshold"),
                        d.post.bloom_threshold,
                        display::BLOOM_THRESHOLD_MAX,
                        Op::BloomThreshold as fn(f32) -> Op,
                    ),
                ] {
                    let r = rows.row(22.0, 4.0);
                    let spec = SliderSpec::new(
                        label,
                        0.0,
                        max,
                        NumberFormat {
                            decimals: 2,
                            trim: true,
                            suffix: "",
                        },
                    )
                    .tooltip_reason(off)
                    .enabled(d.post.bloom);
                    let out = w::slider(ui, r, ("view3d.set", id), value, &spec);
                    if out.changed {
                        app.apply(Action::View3d(op(out.value)));
                    }
                    dragging |= out.active;
                }
                // ドラッグしているあいだは設定のファイルへ書かない（離したときの値を 1 回書く）
                app.view3d.view.display.post_dragging = dragging;
                return;
            }

            // 環境
            heading(&mut rows, &p, lang.t("panels-environment"));
            let r = rows.row(24.0, 4.0);
            for (i, (kind, cell)) in EnvKind::ALL.iter().zip(Rows::split(r, 3, 4.0)).enumerate() {
                if w::button(
                    ui,
                    cell,
                    ("view3d.set.env", i),
                    kind.label(lang),
                    d.env == *kind,
                    true,
                    None,
                    None,
                )
                .clicked()
                {
                    app.apply(Action::View3d(Op::Env(*kind)));
                }
            }
            let env_on = d.env != EnvKind::None;
            slider(
                ui,
                app,
                &mut rows,
                "env_intensity",
                lang.t("panels-intensity"),
                d.env_intensity,
                0.0,
                4.0,
                2,
                "",
                env_on,
                Op::EnvIntensity,
            );
            slider(
                ui,
                app,
                &mut rows,
                "env_rotation",
                lang.t("panels-rotation"),
                d.env_rotation,
                -180.0,
                180.0,
                0,
                "°",
                env_on,
                Op::EnvRotation,
            );
            let r = rows.row(22.0, 4.0);
            let background = w::toggle(
                ui,
                r,
                "view3d.set.background",
                lang.t("panels-show-as-background"),
                d.env_background,
                None,
                env_on,
            );
            if background != d.env_background {
                app.apply(Action::View3d(Op::EnvBackground(background)));
            }
            slider(
                ui,
                app,
                &mut rows,
                "env_blur",
                lang.t("panels-blur"),
                d.env_blur * 100.0,
                0.0,
                100.0,
                0,
                "%",
                env_on && d.env_background,
                |v| Op::EnvBlur(v / 100.0),
            );
            rows.space(4.0);

            // ライト
            heading(&mut rows, &p, lang.t("panels-light"));
            slider(
                ui,
                app,
                &mut rows,
                "light_intensity",
                lang.t("panels-strength"),
                d.light_intensity,
                0.0,
                4.0,
                2,
                "",
                true,
                Op::LightIntensity,
            );
            slider(
                ui,
                app,
                &mut rows,
                "light_yaw",
                lang.t("panels-azimuth"),
                d.light_yaw,
                -180.0,
                180.0,
                0,
                "°",
                true,
                Op::LightYaw,
            );
            slider(
                ui,
                app,
                &mut rows,
                "light_pitch",
                lang.t("panels-elevation"),
                d.light_pitch,
                -89.0,
                89.0,
                0,
                "°",
                true,
                Op::LightPitch,
            );
            let r = rows.row(22.0, 4.0);
            let shadows = w::toggle(
                ui,
                r,
                "view3d.set.shadows",
                lang.t("panels-shadows-2"),
                d.shadows,
                None,
                true,
            );
            if shadows != d.shadows {
                app.apply(Action::View3d(Op::Shadows(shadows)));
            }
            slider(
                ui,
                app,
                &mut rows,
                "shadow_softness",
                lang.t("panels-softness"),
                d.shadow_softness * 100.0,
                0.0,
                100.0,
                0,
                "%",
                d.shadows,
                |v| Op::ShadowSoftness(v / 100.0),
            );
            rows.space(4.0);

            // トーンマッピング
            heading(&mut rows, &p, lang.t("panels-tone-mapping"));
            let r = rows.row(24.0, 4.0);
            for (i, (curve, cell)) in [Curve::None, Curve::Neutral, Curve::Aces]
                .iter()
                .zip(Rows::split(r, 3, 4.0))
                .enumerate()
            {
                if w::button(
                    ui,
                    cell,
                    ("view3d.set.tone", i),
                    display::tone_label(lang, *curve),
                    d.tone_map == *curve,
                    true,
                    None,
                    None,
                )
                .clicked()
                {
                    app.apply(Action::View3d(Op::Tone(*curve)));
                }
            }
            slider(
                ui,
                app,
                &mut rows,
                "exposure",
                lang.t("panels-exposure"),
                d.exposure,
                -6.0,
                6.0,
                1,
                " EV",
                true,
                Op::Exposure,
            );
            rows.space(4.0);
            let r = rows.row(24.0, 0.0);
            if w::button(
                ui,
                r,
                "view3d.set.reset",
                lang.t("panels-reset"),
                false,
                true,
                None,
                Some("restart_alt"),
            )
            .clicked()
            {
                app.apply(Action::View3d(Op::ResetLighting));
            }
        });
    // 外を押す・Esc で閉じる（設定のボタンを押したときは、ボタンの側が開閉する）
    let pressed = ctx.input(|i| i.pointer.any_pressed());
    let at = ctx.input(|i| i.pointer.interact_pos());
    let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let outside =
        pressed && at.is_some_and(|a| !panel.contains(a) && !button.is_some_and(|b| b.contains(a)));
    if (outside || escape) && app.ui.popup.is_none() {
        app.apply(Action::View3d(Op::CloseSettings));
    }
}
