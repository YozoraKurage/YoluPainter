//! 外へ出したウィンドウ（`detach`）を描く: ウィンドウごとに immediate の viewport を回し、その中で、そのウィンドウのドックをメインウィンドウと同じタブの中身で描く。
//!
//! immediate の viewport はメインウィンドウのパスの中で描くので、`AppState` を同じフレームのまま借りられる（deferred の viewport は閉包が
//! `Send + Sync + 'static` で、状態を共有の錠に包み直す必要がある）。代わりに、どちらのウィンドウに入力が来ても、メインウィンドウと別ウィンドウを全部描き直す。
//!
//! 別ウィンドウのパス（OS の別のウィンドウ。`ViewportClass::Immediate`）は、メインウィンドウと同じ入力の道を通す: キーの割り当て（同じ表。キーはフォーカスの
//! あるウィンドウだけに来る）・落としたファイル・スライダーの Esc・ペンの点（Windows はそのウィンドウに繋いだ Windows Ink）。egui が viewport をウィンドウに出せない
//! とき（試験のウィンドウ。`EmbeddedWindow`）はメインウィンドウの中の egui のウィンドウに描き、入力はメインウィンドウのパスが受ける。
//!
//! タブの出し入れは、パスの中では起きたことを控えるだけ（`DockEvent`）で、全部のウィンドウを描いた後に当てる（ウィンドウをまたいで動かすので）。
//!
//! 自前の枠（Windows。メインウィンドウと同じ `custom_frame`）では OS の枠を外し、タブの並ぶ行の何も無い所を帯の代わりにする（引くと
//! `StartDrag`（ペンが触れた引きは、そのウィンドウのペンの受け口がウィンドウを動かす）、ダブルクリックで最大化と元に戻す）。行の右端に閉じる（OS の閉じると同じく、出す前の組へ戻す）、縁で大きさを変える
//! （`titlebar`）。行の右端の閉じるの分は、どの組のタブの行も右を空ける（egui_dock の見た目はドック全体で 1 つ）。

use std::collections::HashMap;

use egui::{
    CursorIcon, Frame, Id, Pos2, Rect, ResizeDirection, Ui, Vec2, ViewportClass, ViewportId,
    ViewportInfo,
};

use super::gpu_lost::FramePoint;
use super::{dock_area, dock_style, Tab, Tabs, YoluApp};
use crate::detach::{self, place, DockOp, Float, OsWindow, Place, Resolved};
use crate::layout::FloatRecord;
use crate::state::{OpenPopup, PopupKind};
use crate::titlebar;
use crate::ui::menu::{self, PopupOutcome, PopupState};
use crate::ui::theme as t;

/// パスの中で起きた、タブの出し入れ（全部のウィンドウを描いた後に当てる）。
#[derive(Debug)]
pub(super) enum DockEvent {
    /// タブの見出しを、そのウィンドウの外で離した。`px` は離した点（仮想スクリーンの画素）、`size` は新しいウィンドウにするときの内側の大きさ（点）、
    /// `ppp` は離したウィンドウの拡大率。
    Released {
        tab: Tab,
        from: ViewportId,
        px: [f32; 2],
        size: [f32; 2],
        ppp: f32,
    },
    /// egui_dock が浮いたウィンドウの面を作った（別ウィンドウへ替える）。`origin` は描いたウィンドウの内側の左上（点）、`mates` は描く前のタブの組。
    Floats {
        floats: Vec<Float>,
        origin: Option<Rect>,
        ppp: f32,
        mates: HashMap<Tab, Vec<Tab>>,
    },
    /// OS の閉じるボタン。
    Closed(u64),
}

/// 1 つのウィンドウのドックを描いた結果。
pub(super) struct DockPass {
    pub grabbed: bool,
    pub released: Option<Tab>,
    pub context: Option<(Tab, Pos2)>,
}

/// ウィンドウ 1 つの見える様子（メインウィンドウのパスが受けた、全部の viewport の情報から）。
fn info_of(ctx: &egui::Context, id: ViewportId) -> Option<ViewportInfo> {
    ctx.input(|i| i.raw.viewports.get(&id).cloned())
}

/// メインウィンドウの内側の矩形（点）。分からなければ（試験のウィンドウ）画面の矩形。
fn root_inner(ctx: &egui::Context) -> Option<Rect> {
    info_of(ctx, ViewportId::ROOT)
        .and_then(|i| i.inner_rect)
        .or_else(|| Some(ctx.input_for(ViewportId::ROOT, |i| i.content_rect())))
}

impl YoluApp {
    /// 別ウィンドウを全部描く（メインウィンドウのドックの後で）。タブの見出しをつかんでいるウィンドウがあれば true。
    pub(super) fn show_detached(
        &mut self,
        ctx: &egui::Context,
        events: &mut Vec<DockEvent>,
    ) -> bool {
        if self.detached.windows.is_empty() {
            return false;
        }
        // 別ウィンドウはここでその場で描かれる（`show_viewport_immediate`）。ここまでのメインウィンドウの描画（3D の提出など）で
        // 失ったときは、別ウィンドウを失ったデバイスで描く前に終える
        self.watch_point(ctx, FramePoint::BeforeDetached);
        let root = ctx.input(|i| i.viewport().clone());
        let root_ppp = ctx.pixels_per_point();
        let monitors = crate::windowpos::monitors();
        let lang = self.state.lang;
        let mut windows = std::mem::take(&mut self.detached.windows);
        let mut grabbed = false;
        for win in &mut windows {
            if win.resolved.is_none() {
                let (record, resolved) = resolve(win.place, &root, root_ppp, &monitors);
                win.record = Some(record);
                win.resolved = Some(resolved);
            }
            let Some(resolved) = win.resolved.as_ref() else {
                continue;
            };
            let mut builder = egui::ViewportBuilder::default()
                .with_title(win.title(lang))
                .with_inner_size(resolved.size)
                .with_min_inner_size(detach::MIN_SIZE)
                .with_icon(detach::app_icon())
                .with_taskbar(false)
                .with_decorations(!self.custom_frame);
            if let Some(position) = resolved.position {
                builder = builder.with_position(position);
            }
            let id = win.viewport_id();
            let serial = win.serial;
            if ctx.embed_viewports() {
                // egui がウィンドウを OS のウィンドウに出せない（試験のウィンドウ）: メインウィンドウの中の egui のウィンドウに、記録の位置と大きさで描く（egui_dock の浮いたウィンドウと
                // 同じ枠。動かすのは記録を変えたときだけ）
                let origin = root_inner(ctx).map_or(Pos2::ZERO, |r| r.min);
                let record = win.record.unwrap_or(FloatRecord {
                    position: resolved.position.unwrap_or([120.0, 120.0]),
                    size: resolved.size,
                    pixels_per_point: root_ppp,
                });
                let at = Pos2::new(record.position[0], record.position[1]) - origin.to_vec2();
                egui::Window::new("")
                    .id(Id::new(id))
                    .title_bar(false)
                    .frame(Frame::window(&ctx.global_style()))
                    .current_pos(at)
                    .fixed_size(record.size)
                    .collapsible(false)
                    .show(ctx, |ui| {
                        grabbed |=
                            self.detached_pass(ui, ViewportClass::EmbeddedWindow, win, events);
                    });
            } else {
                grabbed |= ctx.show_viewport_immediate(id, builder, |ui, class| {
                    self.detached_pass(ui, class, win, events)
                });
            }
            // このウィンドウの描画の中で失ったときも、次のウィンドウを描く前に終える
            self.watch_point(ctx, FramePoint::AfterDetached(serial));
        }
        // （パスの中ではウィンドウを足さない。足したのは当てる側だけ）
        windows.append(&mut self.detached.windows);
        self.detached.windows = windows;
        // 落としたファイルの行き先の矩形は、メインウィンドウで描いた物に戻す（別ウィンドウの分はウィンドウごとに控えた）
        self.state.brushes.ui.list_rect = self.root_drops.brush_list;
        self.state.library.grid_rect = self.root_drops.library_grid;
        grabbed
    }

    /// 別ウィンドウ 1 つのパス。タブの見出しをつかんでいれば true。
    fn detached_pass(
        &mut self,
        ui: &mut Ui,
        class: ViewportClass,
        win: &mut OsWindow,
        events: &mut Vec<DockEvent>,
    ) -> bool {
        let ctx = ui.ctx().clone();
        // OS の別のウィンドウか（試験のウィンドウはメインウィンドウの中の egui のウィンドウで、入力はメインウィンドウが受ける）
        let own = matches!(class, ViewportClass::Immediate | ViewportClass::Deferred);
        let id = ctx.viewport_id();
        let custom = self.custom_frame;
        let frame_ids = [titlebar::drag_id(win.serial), close_id(win.serial)];
        let mut edge: Option<ResizeDirection> = None;
        let mut pen = Vec::new();
        if own {
            let info = ctx.input(|i| i.viewport().clone());
            if info.close_requested() {
                events.push(DockEvent::Closed(win.serial));
            }
            settle_and_record(&ctx, win, &info);
            #[cfg(windows)]
            self.attach_native(&ctx, win, &info);
            #[cfg(target_os = "macos")]
            self.attach_native_mac(&ctx, win, &info);
            // WinTab の入切（設定「ペンの入力」）は、メインウィンドウと同じ札を、このウィンドウの文脈にも合わせる
            win.pen.sync_wintab();
            let (drained, lost) = win.pen.drain_with_lost();
            pen = drained;
            self.state.pen_lost = lost;
            // 次のフレームの初めで、入力のあるフレームとして数える（別ウィンドウの egui の事象は、主のフレームの初めには見えない。`pacing`）
            if !pen.is_empty() || ctx.input(|i| !i.events.is_empty()) {
                self.pacing.note_detached_input();
            }
            self.state.pressure_observe(ctx.pixels_per_point(), &pen);
            for sample in &mut pen {
                sample.pressure = self.state.adjust_pressure(sample.pressure);
            }
            // 自前の枠: 縁を押したら大きさを変える（描いている最中・ペンが触れている最中は受けない。メインウィンドウと同じ）
            if custom {
                let busy = self.state.is_stroking() || pen.iter().any(|s| s.contact);
                edge = titlebar::edges_with(&ctx, busy, &[], &frame_ids);
            }
            // メインウィンドウと同じ表のキー（キーはフォーカスのあるウィンドウにだけ来る）。フォーカスのあるウィンドウのパスだけが見る: クリップボードのキーの
            // 「前のフレームの修飾」はウィンドウをまたいで 1 つなので、フォーカスの無いウィンドウのパスが（押していない）修飾で上書きすると、
            // Shift を押してから C を押した Ctrl+Shift+C（修飾の変化が無いまま `Copy` だけ来る）が普通のコピーになる
            if info.focused == Some(true) {
                crate::shell::handle_shortcuts(&ctx, &mut self.state);
                crate::stencil::update_keys(&ctx, &mut self.state);
            }
            // 落としたファイル（ブラシの一覧・ライブラリの格子は、このウィンドウで描いたときだけ）
            self.state.brushes.ui.list_rect = win.drops.brush_list;
            self.open_dropped(&ctx);
            crate::panels::assets::import_dropped(&ctx, &mut self.state, win.drops.library_grid);
            if ctx.input(|i| i.key_pressed(egui::Key::Escape) && i.pointer.primary_down()) {
                self.state.m2_cancel_drag();
            }
            // ほかのウィンドウで開いたポップアップは、このウィンドウを押したら閉じる
            close_foreign_popup(&ctx, &mut self.state);
        }
        self.state.brushes.ui.list_rect = None;
        self.state.library.grid_rect = None;
        let mates = detach::leaf_mates(&win.dock);
        let lang = self.state.lang;
        let mut tabs = Tabs {
            app: &mut self.state,
            display: &mut self.display,
            thumbs: &mut self.thumbs,
            colors: &mut self.colors,
            view3d: &mut self.view3d,
            renderer3d: &mut self.renderer3d,
            pen: &pen,
            tab_rects: HashMap::new(),
            grabbed: false,
            released: None,
            context: None,
            windows_allowed: win.dock.main_surface().num_tabs() > 1,
        };
        let serial = win.serial;
        let (area, drag, close) = egui::CentralPanel::default()
            .frame(Frame::NONE.fill(t::WINDOW_BG))
            .show(ui, |ui| {
                let mut style = dock_style(ui.style());
                let full = ui.max_rect();
                let row =
                    Rect::from_min_size(full.min, egui::vec2(full.width(), style.tab_bar.height));
                // 自前の枠: タブの行の何も無い所（タブ・境目より先に作り、その下に敷く）と、右端の閉じるの分の空き
                let drag = custom.then(|| {
                    style.tab_bar.inner_margin.right = titlebar::BUTTON_WIDTH as i8;
                    let zone = Rect::from_min_max(
                        row.min,
                        egui::pos2(row.right() - titlebar::BUTTON_WIDTH, row.bottom()),
                    );
                    titlebar::drag_zone_with(ui, zone, frame_ids[0])
                });
                let hline = style.tab_bar.hline_color;
                dock_area(
                    &mut win.dock,
                    Id::new(("yolu.dock.detached", serial)),
                    style,
                )
                .show_inside(ui, &mut tabs);
                let mut close = false;
                if custom {
                    close_gap_lines(ui, &win.dock, hline);
                    close = titlebar::close_button(
                        ui,
                        titlebar::close_rect(row),
                        frame_ids[1],
                        lang.pick("ドックに戻す", "Return to Dock"),
                    );
                }
                (full, drag, close)
            })
            .inner;
        if close {
            events.push(DockEvent::Closed(win.serial));
        }
        if let (Some(drag), true) = (drag, own) {
            let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            titlebar::send_drag_commands(
                &ctx,
                titlebar::drag_commands(&drag, &[], maximized),
                &win.pen,
            );
        }
        let pass = DockPass {
            grabbed: tabs.grabbed,
            released: tabs.released,
            context: tabs.context,
        };
        win.tab_rects = tabs.tab_rects;
        win.drops = detach::DropRects {
            brush_list: self.state.brushes.ui.list_rect.take(),
            library_grid: self.state.library.grid_rect.take(),
        };
        // 自分のウィンドウならウィンドウの全体、egui のウィンドウ（試験）ならドックの矩形の外で離したタブ
        let inside = if own { ctx.content_rect() } else { area };
        tab_events(
            &ctx,
            &mut self.state,
            &win.dock,
            win.viewport_id(),
            inside,
            &pass,
            events,
        );
        let floats = detach::take_floats(&mut win.dock, |i| {
            ctx.memory(|m| m.area_rect(detach::float_area_id(i)))
        });
        if !floats.is_empty() {
            events.push(floats_event(&ctx, floats, mates));
        }
        if own {
            self.popup_in(&ctx, id);
            refuse_foreign_drop(&ctx);
        }
        // 縁の上のポインタの形（中の部品が決めた形を上書きする）
        if let Some(direction) = edge {
            titlebar::edge_cursor(&ctx, direction);
        }
        // eframe はこのあと、このウィンドウを描く。このウィンドウの中（3D の提出など）で失ったときは、失ったデバイスで描く前に終える
        self.watch_point(&ctx, FramePoint::DetachedPassEnd(win.serial));
        pass.grabbed
    }

    /// 別ウィンドウ・メインウィンドウのタブを前にする（別ウィンドウなら、そのウィンドウの中で）。
    pub(super) fn bring_forward(&mut self, tab: Tab) {
        if let Some(path) = self.dock.find_tab(&tab) {
            let _ = self.dock.set_active_tab(path);
        } else if let Some(w) = self.detached.window_of(tab) {
            detach::activate(&mut self.detached.windows[w].dock, tab);
        }
    }

    /// スキンのあるモデルを読んでいて、ポーズのタブがどこにも無ければ足す: レイヤーが別ウィンドウにあればその組へ（前へは出さない）、
    /// 無ければメインウィンドウ（`panels::pose::ensure_tab`）。
    pub(super) fn ensure_pose_tab(&mut self) {
        if self.state.view3d.pose.session.is_none() || self.detached.contains(Tab::Pose) {
            return;
        }
        if self.dock.find_tab(&Tab::Pose).is_none() {
            if let Some(w) = self.detached.window_of(Tab::Layers) {
                let dock = &mut self.detached.windows[w].dock;
                if let Some((node, _)) = dock.find_main_surface_tab(&Tab::Layers) {
                    if let Ok(leaf) = dock.leaf_mut(egui_dock::NodePath {
                        surface: egui_dock::SurfaceIndex::main(),
                        node,
                    }) {
                        leaf.tabs.push(Tab::Pose);
                        return;
                    }
                }
            }
        }
        crate::panels::pose::ensure_tab(&self.state, &mut self.dock);
    }
}

/// 別ウィンドウの、行の右端の閉じるの名前。
fn close_id(serial: u64) -> Id {
    Id::new(("yolu.detached.close", serial))
}

/// 自前の枠で右を空けたタブの行の、空けた所の下の線（egui_dock は空けた所の手前で線を止める）。
fn close_gap_lines(ui: &Ui, dock: &egui_dock::DockState<Tab>, color: egui::Color32) {
    let px = ui.ctx().pixels_per_point().recip();
    let height = t::PANEL_HEADER_HEIGHT;
    for node in dock.main_surface().iter() {
        if let egui_dock::Node::Leaf(leaf) = node {
            let r = leaf.rect;
            if !r.is_positive() || leaf.tabs.is_empty() {
                continue;
            }
            ui.painter().hline(
                (r.right() - titlebar::BUTTON_WIDTH)..=r.right(),
                r.top() + height - px,
                (px, color),
            );
        }
    }
}

/// 1 つのウィンドウのドックを描いた後: 外で離したタブを控え、右クリックならそのウィンドウにメニューを開く。
pub(super) fn tab_events(
    ctx: &egui::Context,
    state: &mut crate::state::AppState,
    dock: &egui_dock::DockState<Tab>,
    from: ViewportId,
    inside: Rect,
    pass: &DockPass,
    events: &mut Vec<DockEvent>,
) {
    if let Some(tab) = pass.released {
        let at = ctx.input(|i| i.pointer.latest_pos().or(i.pointer.interact_pos()));
        if let Some(at) = at.filter(|p| !inside.contains(*p)) {
            let info = ctx.input(|i| i.viewport().clone());
            let ppp = ctx.pixels_per_point();
            let origin = if ctx.viewport_id() == ViewportId::ROOT {
                info.inner_rect.or_else(|| Some(ctx.content_rect()))
            } else {
                info.inner_rect
            };
            events.push(DockEvent::Released {
                tab,
                from,
                px: place::screen_px(origin, ppp, at),
                size: detach::new_window_size(detach::leaf_rect(dock, tab)),
                ppp,
            });
        }
    }
    if let Some((tab, at)) = pass.context {
        if !state.is_stroking() {
            state.popup = Some(OpenPopup {
                kind: PopupKind::DockTab(tab),
                state: PopupState::new(ctx, Rect::from_min_size(at, Vec2::ZERO)),
            });
        }
    }
}

impl YoluApp {
    /// 別ウィンドウで開いたポップアップを、そのウィンドウで描く（ほかのウィンドウのポップアップは描かない）。
    fn popup_in(&mut self, ctx: &egui::Context, id: ViewportId) {
        let Some(mut open) = self.state.popup.take() else {
            return;
        };
        if open.state.viewport != id {
            self.state.popup = Some(open);
            return;
        }
        if open.kind == PopupKind::Pie {
            if crate::pie::show(ctx, &mut self.state, &mut open.state) {
                self.state.popup.get_or_insert(open);
            }
            return;
        }
        if open.kind == PopupKind::Transform {
            if crate::objects::transform::show(ctx, &mut self.state, &mut open.state) {
                self.state.popup.get_or_insert(open);
            }
            return;
        }
        let entries = crate::shell::popup_entries(&self.state, open.kind);
        match menu::show(ctx, Id::new("yolu.popup"), &mut open.state, &entries, &[]) {
            PopupOutcome::Open | PopupOutcome::Step(_) => self.state.popup = Some(open),
            PopupOutcome::Close => {}
            PopupOutcome::Chosen(action) => self.state.apply(action),
        }
    }

    /// ポップアップが開いているウィンドウがもう無ければ閉じる（別ウィンドウを閉じた・戻した）。
    pub(super) fn drop_orphan_popup(&mut self) {
        let alive = |id: ViewportId| {
            id == ViewportId::ROOT || self.detached.index_of_viewport(id).is_some()
        };
        if self
            .state
            .popup
            .as_ref()
            .is_some_and(|p| !alive(p.state.viewport))
        {
            self.state.popup = None;
        }
    }

    /// 控えたタブの出し入れを当てる（全部のウィンドウを描いた後。メインウィンドウのパスの中）。
    pub(super) fn apply_dock_events(&mut self, ctx: &egui::Context, events: Vec<DockEvent>) {
        for event in events {
            match event {
                DockEvent::Closed(serial) => self.detached.close(&mut self.dock, serial),
                DockEvent::Released {
                    tab,
                    from,
                    px,
                    size,
                    ppp,
                } => self.drop_tab(ctx, tab, from, px, size, ppp),
                DockEvent::Floats {
                    floats,
                    origin,
                    ppp,
                    mates,
                } => {
                    for float in floats {
                        let tabs: Vec<Tab> = float.dock.iter_all_tabs().map(|(_, t)| *t).collect();
                        let home = tabs
                            .first()
                            .and_then(|t| mates.get(t))
                            .map(|m| m.iter().copied().filter(|t| !tabs.contains(t)).collect())
                            .unwrap_or_default();
                        let px = place::screen_px(origin, ppp, float.rect.min);
                        let record = place::record_at(px, float.rect.size().into(), ppp);
                        self.detached.open(float.dock, home, Place::Record(record));
                    }
                }
            }
        }
        // タブを全部出した別ウィンドウ（egui_dock の浮いたウィンドウへ出したあとの元のウィンドウなど）は残さない
        self.detached.drop_empty();
        self.drop_orphan_popup();
    }

    /// タブをウィンドウの外で離した: 別の別ウィンドウの上ならそのウィンドウの組へ、メインウィンドウの上ならその点の下の組へ、どのウィンドウの上でもなければ新しい別ウィンドウへ。
    fn drop_tab(
        &mut self,
        ctx: &egui::Context,
        tab: Tab,
        from: ViewportId,
        px: [f32; 2],
        size: [f32; 2],
        ppp: f32,
    ) {
        let target = self.detached.windows.iter().find_map(|w| {
            let id = w.viewport_id();
            if id == from {
                return None;
            }
            let info = info_of(ctx, id)?;
            let wppp = info.native_pixels_per_point.unwrap_or(ppp);
            place::contains_px(info.inner_rect, wppp, px)
                .then(|| (w.serial, place::viewport_point(info.inner_rect, wppp, px)))
        });
        if let Some((serial, at)) = target {
            self.detached
                .move_into(&mut self.dock, tab, serial, Some(at));
            return;
        }
        let root_ppp = ctx.pixels_per_point();
        let root = root_inner(ctx);
        if from != ViewportId::ROOT && place::contains_px(root, root_ppp, px) {
            let at = place::viewport_point(root, root_ppp, px);
            self.detached.return_tab(&mut self.dock, tab, Some(at));
            return;
        }
        let outer = [
            px[0] - detach::GRAB_OFFSET[0] * ppp,
            px[1] - detach::GRAB_OFFSET[1] * ppp,
        ];
        self.detached.detach(
            &mut self.dock,
            tab,
            Place::Record(place::record_at(outer, size, ppp)),
        );
    }

    /// メニュー・右クリックのドックの操作を当てる（ポップアップの後。メインウィンドウのパスの中）。
    pub(super) fn apply_dock_ops(&mut self, ctx: &egui::Context) {
        let ops = std::mem::take(&mut self.state.ui.dock_ops);
        for op in ops {
            match op {
                DockOp::Detach(tab) => {
                    let place = self.place_beside(ctx, tab);
                    self.detached.detach(&mut self.dock, tab, place);
                }
                DockOp::Return(tab) => {
                    self.detached.return_tab(&mut self.dock, tab, None);
                }
                DockOp::Hide(tab) => {
                    self.detached.hide(&mut self.dock, tab);
                }
                DockOp::Show(tab) => {
                    if let Some(id) = self.detached.show(&mut self.dock, tab) {
                        ctx.send_viewport_cmd_to(id, egui::ViewportCommand::Focus);
                    }
                }
            }
        }
        self.drop_orphan_popup();
    }

    /// 「別ウィンドウで開く」の置き場所: タブがいる組の左上から少しずらした所に、その組の大きさで。
    fn place_beside(&self, ctx: &egui::Context, tab: Tab) -> Place {
        let ppp = ctx.pixels_per_point();
        let (leaf, origin, wppp) = match self.detached.window_of(tab) {
            Some(w) => {
                let window = &self.detached.windows[w];
                let info = info_of(ctx, window.viewport_id());
                (
                    detach::leaf_rect(&window.dock, tab),
                    info.as_ref().and_then(|i| i.inner_rect),
                    info.and_then(|i| i.native_pixels_per_point).unwrap_or(ppp),
                )
            }
            None => (detach::leaf_rect(&self.dock, tab), root_inner(ctx), ppp),
        };
        let size = detach::new_window_size(leaf);
        let corner = leaf
            .filter(|r| r.is_finite())
            .map_or(Pos2::new(120.0, 120.0), |r| r.min + Vec2::splat(24.0));
        let px = place::screen_px(origin, wppp, corner);
        Place::Record(place::record_at(px, size, wppp))
    }

    /// Windows: 別ウィンドウの OS のウィンドウを見つけたら、メインウィンドウを持ち主にし、最大化を自動で隠すタスクバーに合わせ、ペンの入力を繋ぐ（見つかるまで、数十フレーム探す）。
    #[cfg(windows)]
    fn attach_native(&mut self, ctx: &egui::Context, win: &mut OsWindow, info: &ViewportInfo) {
        const TRIES: u32 = 120;
        if win.hwnd.is_some() || win.attach_tries >= TRIES {
            return;
        }
        win.attach_tries += 1;
        let (Some(inner), Some(ppp)) = (info.inner_rect, info.native_pixels_per_point) else {
            return;
        };
        let client = crate::windowpos::PxRect::from_origin_size(
            (inner.min.x * ppp).round() as i32,
            (inner.min.y * ppp).round() as i32,
            (inner.width() * ppp).round() as i32,
            (inner.height() * ppp).round() as i32,
        );
        let mut skip: Vec<isize> = self
            .detached
            .windows
            .iter()
            .filter_map(|w| w.hwnd)
            .collect();
        skip.extend(self.main_hwnd);
        if let Some(hwnd) = detach::native::find_window(client, &skip, &win.title(self.state.lang))
        {
            if let Some(owner) = self.main_hwnd {
                detach::native::set_owner(hwnd, owner);
            }
            crate::windowpos::install_hwnd(hwnd);
            win.pen = crate::pen::PenInput::attach_hwnd(hwnd, ctx, &self.pen);
            win.hwnd = Some(hwnd);
        }
    }

    /// macOS: 別ウィンドウの NSWindow を、そのビューポートに今付いている題名（`info.title`）で見つけたら、タブレットの入力を繋ぐ。フォーカスがあるときはキーウィンドウを
    /// 使う。見つかるまで、数十フレーム探す（諦めたあとも、題名が変わったら 0 から探し直す）。題名が重なって決まらないときは繋がない
    /// （取り違えると、ほかのウィンドウのペンの点を受けてしまう）。
    #[cfg(target_os = "macos")]
    fn attach_native_mac(&mut self, ctx: &egui::Context, win: &mut OsWindow, info: &ViewportInfo) {
        const TRIES: u32 = 120;
        if win.pen.is_window_hooked() {
            return;
        }
        let title = info.title.clone().unwrap_or_default();
        if win.attach_title != title {
            win.attach_title = title.clone();
            win.attach_tries = 0;
        }
        if title.is_empty() || win.attach_tries >= TRIES {
            return;
        }
        win.attach_tries += 1;
        let focused = info.focused == Some(true);
        if let Some(pen) = crate::pen::PenInput::attach_titled(&title, focused, ctx, &self.pen) {
            win.pen = pen;
        }
    }

    /// 見えている別ウィンドウがあるか（メインウィンドウが最小化・隠れていても、eframe は別ウィンドウのためにメインウィンドウのパスを回す）。
    pub(super) fn any_detached_visible(&self, ctx: &egui::Context) -> bool {
        self.detached
            .windows
            .iter()
            .any(|w| info_of(ctx, w.viewport_id()).is_some_and(|i| i.visible().unwrap_or(true)))
    }

    /// メインウィンドウか別ウィンドウのどれかで、ポインタのボタン（マウス・ペン）が押されているか。
    pub(super) fn any_pointer_down(&self, ctx: &egui::Context) -> bool {
        ctx.input(|i| i.pointer.any_down())
            || self
                .detached
                .windows
                .iter()
                .any(|w| ctx.input_for(w.viewport_id(), |i| i.pointer.any_down()))
    }

    /// 別ウィンドウのどれかにフォーカスがあるか。
    pub(super) fn detached_focused(&self, ctx: &egui::Context) -> bool {
        self.detached
            .windows
            .iter()
            .any(|w| info_of(ctx, w.viewport_id()).is_some_and(|i| i.focused == Some(true)))
    }
}

/// egui_dock の浮いたウィンドウ（このパスのウィンドウの点）を、別ウィンドウへ替える頼みにする。
pub(super) fn floats_event(
    ctx: &egui::Context,
    floats: Vec<Float>,
    mates: HashMap<Tab, Vec<Tab>>,
) -> DockEvent {
    let info = ctx.input(|i| i.viewport().clone());
    let origin = if ctx.viewport_id() == ViewportId::ROOT {
        info.inner_rect.or_else(|| Some(ctx.content_rect()))
    } else {
        info.inner_rect
    };
    DockEvent::Floats {
        floats,
        origin,
        ppp: ctx.pixels_per_point(),
        mates,
    }
}

/// 最初の置き場所から、作るときに渡す値（と、記録の初めの値）を決める。
pub(super) fn resolve(
    place: Place,
    root: &ViewportInfo,
    root_ppp: f32,
    monitors: &[crate::windowpos::Monitor],
) -> (FloatRecord, Resolved) {
    let inner = root.inner_rect.unwrap_or(Rect::ZERO);
    let record = match place {
        Place::Record(r) => r,
        Place::OverMain { offset, size } => FloatRecord {
            position: [inner.min.x + offset[0], inner.min.y + offset[1]],
            size,
            pixels_per_point: root_ppp,
        },
        Place::Center { size } => {
            let c = inner.center();
            FloatRecord {
                position: [c.x - size[0] / 2.0, c.y - size[1] / 2.0],
                size,
                pixels_per_point: root_ppp,
            }
        }
    };
    let main = root.outer_rect.map(|r| {
        crate::windowpos::PxRect::from_origin_size(
            (r.min.x * root_ppp).round() as i32,
            (r.min.y * root_ppp).round() as i32,
            (r.width() * root_ppp).round() as i32,
            (r.height() * root_ppp).round() as i32,
        )
    });
    let resolved = match place::plan(&record, monitors, main) {
        Some(p) => Resolved {
            position: Some([p.rect.left as f32 / p.scale, p.rect.top as f32 / p.scale]),
            size: [
                p.rect.width() as f32 / p.scale,
                p.rect.height() as f32 / p.scale,
            ],
            settle: Some(crate::windowpos::Settle::new(p)),
        },
        None => Resolved {
            position: Some(record.position),
            size: record.size,
            settle: None,
        },
    };
    (record, resolved)
}

/// 置き場所を合わせている間は頼みを送り、合った後は今の外枠の位置と内側の大きさを記録する（最小化中は記録しない）。
fn settle_and_record(ctx: &egui::Context, win: &mut OsWindow, info: &ViewportInfo) {
    let minimized = info.minimized == Some(true);
    if let Some(resolved) = win.resolved.as_mut() {
        if let Some(settle) = resolved.settle.as_mut() {
            let actual = if minimized {
                None
            } else {
                crate::windowpos::Actual::from_viewport(info)
            };
            let step = settle.step(actual);
            for command in step.commands {
                ctx.send_viewport_cmd(command);
            }
            if step.settling {
                ctx.request_repaint();
                return;
            }
            resolved.settle = None;
        }
    }
    if minimized {
        return;
    }
    if let (Some(outer), Some(inner), Some(ppp)) = (
        info.outer_rect,
        info.inner_rect,
        info.native_pixels_per_point,
    ) {
        win.record = Some(FloatRecord {
            position: [outer.min.x, outer.min.y],
            size: [inner.width(), inner.height()],
            pixels_per_point: ppp,
        });
    }
}

/// ほかのウィンドウで開いたポップアップは、このウィンドウを押したら閉じる（`ctx` のウィンドウのパスで）。
pub(super) fn close_foreign_popup(ctx: &egui::Context, state: &mut crate::state::AppState) {
    let here = ctx.viewport_id();
    if state
        .popup
        .as_ref()
        .is_some_and(|p| p.state.viewport != here)
        && ctx.input(|i| i.pointer.any_pressed())
    {
        state.popup = None;
    }
}

/// アセットの品などを引いたまま、引き始めたウィンドウの外へ出たら、ポインタを「落とせない」にする（ウィンドウをまたいで落とす道は無い）。
pub(super) fn refuse_foreign_drop(ctx: &egui::Context) {
    if !egui::DragAndDrop::has_any_payload(ctx) {
        return;
    }
    let outside = ctx
        .input(|i| i.pointer.latest_pos())
        .is_some_and(|p| !ctx.content_rect().contains(p));
    if outside {
        ctx.set_cursor_icon(CursorIcon::NotAllowed);
    }
}
