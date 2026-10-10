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
use crate::view3d::render::MeshMapSource;
use crate::view3d::selection_overlay::Skip;
use crate::view3d::{gizmo, input, render::View3dRenderer};

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

/// 隅のアイコンの結果。
#[derive(Default)]
struct Corner {
    /// 設定のアイコンの矩形（設定のパネルは、その外を押すと閉じる。このアイコンは外に数えない）。
    settings: Option<Rect>,
    /// アイコンの列の下の端（軸の印をその下に置く）。
    bottom: f32,
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
        let content = full;
        app.view3d.view_rect = Some(content);
        app.view3d.viewport = Some(ui.ctx().viewport_id());
        crate::objects::transform::settle(app);
        let response = ui.interact(content, ui.id().with("view3d"), Sense::click_and_drag());
        app.rulers
            .note_pointer(crate::rulers::Place::View3d, response.contains_pointer());
        let ppp = ui.ctx().pixels_per_point();
        input::handle(
            ui,
            app,
            content,
            pen,
            crate::gesture::foreign_press(ui.ctx(), &response),
        );

        let p = ui.painter().clone();
        // 焼いたメッシュマップだけを見せているのに、今のセットにそのマップが無ければ（セットを替えた・焼き直して消えた）マテリアルへ戻す
        if let Shading::MeshMap(kind) = app.view3d.display.shading {
            if app.sets.current().mesh_maps.get(kind).is_none() {
                app.apply(Action::View3d(Op::Shading(Shading::Material)));
            }
        }
        // 3D の絵が元より縮んでいるとき（縮めた段、予算で決まったか）。メッシュマップの表示ではそのマップの縮め
        let mut reduced: Option<(u32, Option<bool>)> = None;
        // 選択範囲の重ねが 3D の絵の予算に入らずに出せていないか
        let mut overlay_skip: Option<Skip> = None;
        let drawn = match (app.view3d.model.clone(), renderer) {
            (Some(model), Some(renderer)) => {
                // 選択範囲の重ね（縁・クイックマスクの赤・選択ペンの被覆）
                renderer.set_overlay(crate::view3d::selection_overlay::input(app, ui.ctx()));
                let size = [
                    (content.width() * ppp).round().max(1.0) as u32,
                    (content.height() * ppp).round().max(1.0) as u32,
                ];
                let set = app.sets.current();
                let map = match app.view3d.display.shading {
                    Shading::MeshMap(kind) => set.mesh_maps.get(kind).map(|map| MeshMapSource {
                        key: ((set.uid as u64) << 40)
                            ^ ((kind as i32 as u64) << 32)
                            ^ set.mesh_maps.revision(),
                        map,
                    }),
                    _ => None,
                };
                let others = crate::view3d::other_sets::collect(app);
                let id = renderer.prepare_sets(
                    &app.doc,
                    &model,
                    app.view3d.material,
                    &app.view3d.camera,
                    size,
                    &app.view3d.display,
                    map.as_ref(),
                    &others,
                );
                drop(others);
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
                    .input
                    .note_sync(std::time::Duration::from_micros(stats.last_sync_us));
                app.view3d.display.drawn_samples = stats.samples;
                reduced = match app.view3d.display.shading {
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
        // ステンシル（3D の絵の上に、画面に貼り付いた半透明の画像。編集・ポーズのモードでは描かないので出さない）
        let mode = app.mode;
        if drawn && mode.paints() {
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
        // 塗りつぶしレイヤーの置き場・形のギズモと、棚の画像のデカールの落とし先（3D の絵の上）
        let mut gizmo_cursor = None;
        if drawn && mode != EditorMode::Pose {
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
            // 棚の画像を落としてデカールを置く（編集のモードでは、置いたデカールを選ぶ）
            crate::fillfx::decal_drop(ui, app, content);
            gizmo_cursor = pointer.and_then(|_| crate::fillfx::gizmo::cursor(app));
        }
        if !drawn {
            self.placeholder(ui, app, content);
        } else if let Some(press) = app.view3d.input.eyedrop {
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
        } else if let Some(icon) = gizmo_cursor {
            // 形のギズモのハンドルの上（ブラシの円は出さない）
            ui.ctx().set_cursor_icon(icon);
        } else if mode == EditorMode::Edit {
            // 編集のモード: 描かないので、ブラシの円もツールの印も出さない
            if ui
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|p| response.contains_pointer() && content.contains(p))
            {
                ui.ctx().set_cursor_icon(if app.view3d.input.nav.is_some() {
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
                ui.ctx().set_cursor_icon(if app.view3d.input.nav.is_some() {
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
                ui.ctx().set_cursor_icon(if app.view3d.input.nav.is_some() {
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
                ui.ctx().set_cursor_icon(if app.view3d.input.nav.is_some() {
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
                if app.view3d.input.nav.is_some() {
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
                // 2D の対称の写しのカーソルは、今のセットの UV の格子で引く（無ければここで作る）
                if app.canvas_symmetry().enabled() {
                    let _ = app.region_grid();
                }
                if let Some(zoom) = app.view3d.input.zoom {
                    ui.ctx()
                        .set_cursor_icon(crate::canvas::zoom_cursor(zoom.out));
                } else if app.view3d.input.nav.is_some() {
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
        }
        let corner = self.corner(
            ui,
            app,
            content,
            reduced.filter(|_| drawn),
            overlay_skip.filter(|_| drawn),
        );
        if app.view3d.display.settings_open {
            settings_panel(ui, app, content, corner.settings);
        } else if drawn {
            // 軸の印（アイコンの下。設定のパネルと重なるので、パネルが開いている間は出さない）
            crate::view3d::axis_gizmo::show(ui, app, content, corner.bottom);
        }

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

    /// 表示域の右上の隅に重ねる小さなアイコン（見出しの帯は置かない。文字なし、名前と理由はツールチップ）: 3D の絵が元より縮んでいる印
    /// （押せない。`reduced` は縮めた段と、メモリの予算で決まったか）、3D の表示の切り替え（押すとメニュー）、光と環境の設定、モデル全体が
    /// 見える位置へ戻す。返すのは設定のアイコンの矩形（設定のパネルを下に置く）。
    fn corner(
        &self,
        ui: &mut Ui,
        app: &mut AppState,
        view: Rect,
        reduced: Option<(u32, Option<bool>)>,
        overlay_skip: Option<Skip>,
    ) -> Corner {
        if app.view3d.model.is_none() {
            return Corner::default();
        }
        let lang = app.lang;
        let shading = display::shading_label(app, app.view3d.display.shading);
        let open_shading = matches!(
            app.popup.as_ref().map(|p| p.kind),
            Some(PopupKind::View3dShading)
        );
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
                "shading",
                "texture",
                format!("{}: {shading}", lang.pick("3D の表示", "3D shading")),
            )
            .selected(open_shading),
        );
        items.push(
            w::CornerIcon::new(
                "settings",
                "light_mode",
                lang.pick(
                    "光・環境・トーンマッピング",
                    "Light, environment, tone mapping",
                ),
            )
            .selected(app.view3d.display.settings_open),
        );
        items.push(
            w::CornerIcon::new(
                "frame",
                "target",
                lang.pick(
                    "モデル全体が見える位置へ戻す",
                    "Fit the whole model in view",
                ),
            )
            .enabled(!app.is_stroking()),
        );
        let out = w::corner_icons(ui, "view3d", view, &items);
        if let Some(i) = out.clicked {
            match items[i].id {
                "shading" => {
                    let ctx = ui.ctx().clone();
                    let b = out.rects[i];
                    // 押したアイコンの下に開く
                    app.popup = Some(OpenPopup {
                        kind: PopupKind::View3dShading,
                        state: PopupState::new(
                            &ctx,
                            Rect::from_min_size(
                                pos2(b.left(), b.bottom() + 2.0),
                                vec2(b.width(), 0.0),
                            ),
                        ),
                    });
                }
                "settings" => app.apply(Action::View3d(Op::ToggleSettings)),
                "frame" => app.apply(Action::FrameModel),
                _ => {}
            }
        }
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
            Some(lang.pick("すべて隠しています", "All hidden"))
        } else if app.view3d.model.is_none() {
            None
        } else {
            Some(lang.pick("3D を描けません（GPU なし）", "Cannot draw 3D (no GPU)"))
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
                    lang.pick("新規プロジェクト", "New Project"),
                    &Action::NewProjectDialog,
                )
            };
            let r = Rect::from_center_size(pos2(x, top + NEW_H / 2.0), vec2(width(180.0), NEW_H));
            if w::button(
                ui,
                r,
                "view3d.new_project",
                lang.pick("新規プロジェクト…", "New Project…"),
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
                    lang.pick("試しの立方体を読む", "Load Test Cube"),
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
        Skip::Budget => lang.pick(
            "GPU のメモリの予算（3D の絵の取り分）に入りません",
            "it does not fit in the GPU memory budget for 3D pictures",
        ),
        Skip::Size => lang.pick(
            "キャンバスの幅か高さが GPU のテクスチャの上限を超えています",
            "the canvas is wider or taller than the GPU texture limit",
        ),
    };
    lang.with_reason(
        lang.pick(
            "選択範囲を 3D に出していません",
            "Selection not shown in 3D",
        ),
        reason,
    )
}

/// 3D の絵が元の大きさより縮んでいることの印のツールチップ（隅の警告のアイコン）: 縮めた段と短い理由。`by_budget` は縮めがメモリの予算で
/// 決まったか（None は分からない: メッシュマップ）。
fn reduced_tooltip(lang: crate::lang::Lang, level: u32, by_budget: Option<bool>) -> String {
    let text = lang.pick(
        format!("縮小表示 1/{}", 1u64 << level),
        format!("Reduced 1/{}", 1u64 << level),
    );
    let reason = match by_budget {
        Some(true) => lang.pick(
            "GPU のメモリの予算を超えるので、3D には縮めて見せています（絵と書き出しは元の大きさのまま）",
            "Shown smaller in 3D to stay within the GPU memory budget (the texture and exports keep their size)",
        ),
        Some(false) => lang.pick(
            "GPU のテクスチャの大きさの上限を超えるので、3D には縮めて見せています（絵と書き出しは元の大きさのまま）",
            "Shown smaller in 3D because it exceeds the GPU texture size limit (the texture and exports keep their size)",
        ),
        None => lang.pick(
            "GPU のメモリの予算か大きさの上限を超えるので、縮めて見せています",
            "Shown smaller to stay within the GPU memory budget or size limit",
        ),
    };
    format!("{text}: {reason}")
}

const PANEL_WIDTH: f32 = 252.0;
const PANEL_HEIGHT: f32 = 464.0;
/// 設定のパネルの「画質」の面の高さ（内容が少ないので低い）。
const QUALITY_HEIGHT: f32 = 204.0;

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
    let height = if app.view3d.display.tab == SettingsTab::Quality {
        QUALITY_HEIGHT
    } else {
        PANEL_HEIGHT
    };
    let panel = Rect::from_min_size(pos, vec2(width, height));
    // Esc で閉じる浮いた部品として覚える（閉じたフレームも開いている扱い。キャンバスの Esc が先に選択を外さない）
    crate::ui::window::note_open(&ctx);
    egui::Area::new(egui::Id::new("view3d.settings.area"))
        .order(Order::Foreground)
        .fixed_pos(pos)
        .show(&ctx, |ui| {
            // 下の 3D へ押しが通らないように、パネルの全体を受ける（部品はその上に描く）
            ui.allocate_rect(panel, Sense::click_and_drag());
            let p = ui.painter().clone();
            w::rounded(&p, panel, t::PANEL_BG, 4.0);
            w::outline(&p, panel, t::BORDER, 1.0, 4.0);
            let d = app.view3d.display;
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
                heading(
                    &mut rows,
                    &p,
                    lang.pick("アンチエイリアス", "Anti-aliasing"),
                );
                let r = rows.row(24.0, 4.0);
                let supported = d.supported_sample_counts();
                let shown = d.shown_samples();
                for (n, cell) in display::SAMPLE_CHOICES.iter().zip(Rows::split(r, 4, 4.0)) {
                    let usable = supported.contains(n);
                    let label = if *n == 1 {
                        lang.pick("切", "Off").to_owned()
                    } else {
                        format!("{n}×")
                    };
                    let tip = if usable {
                        format!("{} {label}", lang.pick("アンチエイリアス", "Anti-aliasing"))
                    } else {
                        lang.pick("この GPU は対応していません", "Not supported on this GPU")
                            .to_owned()
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
                        &lang.pick(
                            format!("描画は {}×（メモリの上限）", d.drawn_samples),
                            format!("Drawing at {}× (memory limit)", d.drawn_samples),
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
                    lang.pick("ブルーム", "Bloom"),
                    d.post.bloom,
                    None,
                    true,
                );
                if bloom != d.post.bloom {
                    app.apply(Action::View3d(Op::Bloom(bloom)));
                }
                let mut dragging = false;
                let off = (!d.post.bloom).then(|| lang.pick("ブルームが切です", "Bloom is off"));
                for (id, label, value, max, op) in [
                    (
                        "bloom_strength",
                        lang.pick("強さ", "Strength"),
                        d.post.bloom_strength,
                        display::BLOOM_STRENGTH_MAX,
                        Op::BloomStrength as fn(f32) -> Op,
                    ),
                    (
                        "bloom_threshold",
                        lang.pick("しきい値", "Threshold"),
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
                app.view3d.display.post_dragging = dragging;
                return;
            }

            // 環境
            heading(&mut rows, &p, lang.pick("環境", "Environment"));
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
                lang.pick("明るさ", "Intensity"),
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
                lang.pick("回転", "Rotation"),
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
                lang.pick("背景に映す", "Show as background"),
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
                lang.pick("ぼかし", "Blur"),
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
            heading(&mut rows, &p, lang.pick("ライト", "Light"));
            slider(
                ui,
                app,
                &mut rows,
                "light_intensity",
                lang.pick("強さ", "Strength"),
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
                lang.pick("方位", "Azimuth"),
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
                lang.pick("高さ", "Elevation"),
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
                lang.pick("影", "Shadows"),
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
                lang.pick("やわらかさ", "Softness"),
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
            heading(&mut rows, &p, lang.pick("トーンマッピング", "Tone Mapping"));
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
                lang.pick("露出", "Exposure"),
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
                lang.pick("既定に戻す", "Reset"),
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
    if (outside || escape) && app.popup.is_none() {
        app.apply(Action::View3d(Op::CloseSettings));
    }
}
