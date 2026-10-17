//! マルチ 3D ビュー（案 B）: 3D ビューを別のタブ（「3D ビュー 2」…）として増やし、今のドックの枠にそのまま並べる。
//! 各ビューの左上に表示の種類のドロップダウン、右上に光と環境・全体が見える位置へ、軸の印。タブの見出しにも表示の種類が出る
//! （重ねて隠れているタブ・別ウィンドウのタブを見分けるため）。ビューの追加は見出しの帯の右端の「+」。タブはドラッグで並べ替えられ、
//! 右クリックの「別ウィンドウで開く」で別のウィンドウ（別のディスプレイ）へ出せる。

use std::collections::HashMap;

use egui::{pos2, vec2, Color32, Id, Pos2, Rect, Ui};
use egui_dock::{DockState, NodeIndex, NodePath, SurfaceIndex, TabIndex, TabPath, TabViewer};
use yolu_app::app::{dock_area, dock_style, Tab};
use yolu_app::titlebar;
use yolu_app::ui::menu::{self, Entry as MenuEntry};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::{self as w, CornerIcon};

use super::multiview_common::{
    self as c, Cam, Shade, BACK_ANGLE, CORNER_IDS, PERSPECTIVE, PERSPECTIVE_B,
};
use crate::page::SketchCtx;

/// ドックの入れ物のタブの種類（3D ビュー n 枚目の目印。名前も中身もここで差し替える）。
const SLOTS: [Tab; 4] = [
    Tab::View3d,
    Tab::TextureSets,
    Tab::Properties,
    Tab::Channels,
];

fn slot(tab: Tab) -> usize {
    SLOTS.iter().position(|t| *t == tab).unwrap_or(0)
}

fn start(i: usize) -> (Shade, Cam) {
    match i {
        0 => (Shade::Material, PERSPECTIVE),
        1 => (Shade::Channel(1), PERSPECTIVE_B),
        2 => (Shade::MeshMap(2), BACK_ANGLE),
        _ => (Shade::Neutral, PERSPECTIVE),
    }
}

struct Viewer<'a> {
    ctx: &'a mut SketchCtx,
    /// フォーカスのある枠の、選んでいるタブ。
    focused: Option<Tab>,
    /// 撮る間に表示のメニューを開いて出すタブ（番号）。
    open_menu: Option<usize>,
    /// タブの見出しの矩形（右クリックのメニューを、そのタブの下に開くため）。
    tab_rects: HashMap<Tab, Rect>,
    /// このフレームに右クリックされたタブと、押した位置。
    context: Option<(Tab, Pos2)>,
}

impl<'a> Viewer<'a> {
    fn new(ctx: &'a mut SketchCtx, focused: Option<Tab>, open_menu: Option<usize>) -> Self {
        Viewer {
            ctx,
            focused,
            open_menu,
            tab_rects: HashMap::new(),
            context: None,
        }
    }

    fn shade(&self, i: usize) -> Shade {
        Shade::from_code(
            self.ctx
                .values
                .index(&format!("mv.b.shade.{i}"), start(i).0.code()),
        )
    }
}

impl TabViewer for Viewer<'_> {
    type Tab = Tab;

    fn id(&mut self, tab: &mut Tab) -> Id {
        Id::new(("mv.b.tab", slot(*tab)))
    }

    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        let i = slot(*tab);
        let name = if i == 0 {
            self.ctx.pick("3D ビュー", "3D View").to_owned()
        } else {
            format!("{} {}", self.ctx.pick("3D ビュー", "3D View"), i + 1)
        };
        egui::RichText::new(format!("{name} · {}", self.shade(i).short(self.ctx)))
            .font(t::HEADER.font())
            .into()
    }

    fn is_closeable(&self, tab: &Tab) -> bool {
        slot(*tab) != 0
    }

    fn scroll_bars(&self, _tab: &Tab) -> [bool; 2] {
        [false, false]
    }

    fn on_tab_button(&mut self, tab: &mut Tab, response: &egui::Response) {
        self.tab_rects.insert(*tab, response.rect);
        if response.secondary_clicked() {
            let at = response
                .interact_pointer_pos()
                .unwrap_or(response.rect.left_bottom());
            self.context = Some((*tab, at));
        }
        // 本物の枠と同じく、フォーカスのある枠の選んでいるタブの上に青い線を引く
        if self.focused == Some(*tab) {
            let r = response.rect;
            let p = response
                .ctx
                .layer_painter(response.layer_id)
                .with_clip_rect(r);
            p.rect_filled(
                Rect::from_min_size(r.min, vec2(r.width(), 2.0)),
                0.0,
                t::ACCENT,
            );
        }
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Tab) {
        let i = slot(*tab);
        let r = ui.max_rect();
        ui.advance_cursor_after_rect(r);
        let shade = self.shade(i);
        let (_, cam) = start(i);
        // 今の 3D ビューの隅のアイコン（光と環境・全体が見える位置へ。表示の種類は左上のドロップダウンへ移した）と軸の印
        let items = [
            CornerIcon::new("settings", "light_mode", c::tip_light(self.ctx)),
            CornerIcon::new("frame", "target", c::tip_frame(self.ctx)),
        ];
        c::draw_view(
            ui,
            r,
            cam,
            shade,
            Some(w::corner_bottom(r, items.len())),
            c::gizmo_scale(r),
        );
        if self.focused == Some(*tab) {
            c::focus_border(ui, r);
        }
        let _ = w::corner_icons(ui, CORNER_IDS[i], r, &items);
        if let Some(chosen) = c::shading_dropdown(
            ui,
            self.ctx,
            r,
            ("mv.b", i),
            shade,
            self.open_menu == Some(i),
        ) {
            self.ctx
                .values
                .set_index(&format!("mv.b.shade.{i}"), chosen.code());
        }
    }
}

thread_local! {
    /// 並べたドックの今の状態（ウィンドウで触って動かせるように、フレームをまたいで持つ）。名前ごと。
    static DOCKS: std::cell::RefCell<HashMap<&'static str, DockState<Tab>>> = std::cell::RefCell::new(HashMap::new());
}

/// 左右 2 つの枠: 左は 3D ビュー（マテリアル、フォーカス）、右は 3D ビュー 2（ラフネス）と重ねた 3D ビュー 3（AO）。
fn build_two_panes() -> DockState<Tab> {
    let mut dock = DockState::new(vec![SLOTS[0]]);
    let surface = dock.main_surface_mut();
    let [left, right] = surface.split_right(NodeIndex::root(), 0.5, vec![SLOTS[1], SLOTS[2]]);
    let _ = dock.set_active_tab(TabPath::new(SurfaceIndex::main(), right, TabIndex(0)));
    dock.set_focused_node_and_surface(NodePath {
        surface: SurfaceIndex::main(),
        node: left,
    });
    dock
}

/// 別ウィンドウへ出す前の本体: 3D ビュー（マテリアル）と 3D ビュー 3（AO）が 1 つの枠に重なっている。
fn build_main_window() -> DockState<Tab> {
    DockState::new(vec![SLOTS[0], SLOTS[2]])
}

/// 別ウィンドウへ出した 3D ビュー 2。
fn build_detached() -> DockState<Tab> {
    DockState::new(vec![SLOTS[1]])
}

/// ドック 1 つを `rect` に描く。`detached` なら、別ウィンドウと同じに、タブの行の右端に「ドックに戻す」の閉じるボタンを置く
/// （本物の別ウィンドウと同じ部品）。右クリックされたタブがあれば返す。
fn show_dock(
    ui: &mut Ui,
    ctx: &mut SketchCtx,
    rect: Rect,
    key: &'static str,
    build: fn() -> DockState<Tab>,
    open_menu: Option<usize>,
    detached: bool,
) -> (Option<(Tab, Pos2)>, HashMap<Tab, Rect>, bool) {
    let mut dock = DOCKS
        .with(|d| d.borrow_mut().remove(key))
        .unwrap_or_else(build);
    let focused = dock.find_active_focused().map(|(_, tab)| *tab);
    let mut viewer = Viewer::new(ctx, focused, open_menu);
    let mut style = dock_style(ui.style());
    let row = Rect::from_min_size(rect.min, vec2(rect.width(), style.tab_bar.height));
    if detached {
        style.tab_bar.inner_margin.right = titlebar::BUTTON_WIDTH as i8;
    }
    let mut closed = false;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        dock_area(&mut dock, Id::new(("mv.b.dock", key)), style)
            .show_add_buttons(!detached)
            .show_close_buttons(!detached)
            .show_inside(ui, &mut viewer);
        if detached {
            closed = titlebar::close_button(
                ui,
                titlebar::close_rect(row),
                Id::new(("mv.b.return", key)),
                viewer.ctx.pick("ドックに戻す", "Return to Dock"),
            );
        }
    });
    let out = (
        viewer.context.take(),
        std::mem::take(&mut viewer.tab_rects),
        closed,
    );
    DOCKS.with(|d| d.borrow_mut().insert(key, dock));
    out
}

fn draw_with(ui: &mut Ui, ctx: &mut SketchCtx, open_menu: Option<usize>) {
    let area = ui.max_rect();
    ui.advance_cursor_after_rect(area);
    // 本物のドックの 1 つのタブの中に、さらにドックを入れる形は取れない（見出しが二重になる）ので、画面全体を覆ってドックを描く
    // （本物では、3D ビューの枠がそのままドックの一部になる）
    c::cover(ui, "b", |ui, screen| {
        let _ = show_dock(ui, ctx, screen, "two", build_two_panes, open_menu, false);
    });
}

/// 左右の 2 つの枠。左はマテリアル（フォーカス）、右は「3D ビュー 2（ラフネス）」と、重ねた「3D ビュー 3（AO）」。
pub fn draw(ui: &mut Ui, ctx: &mut SketchCtx) {
    draw_with(ui, ctx, None);
}

/// 右の枠のビューの左上のドロップダウンから、表示の種類のメニューを開いた形。
pub fn draw_menu(ui: &mut Ui, ctx: &mut SketchCtx) {
    draw_with(ui, ctx, Some(1));
}

/// 3D ビュー 2 を別のウィンドウ（別のディスプレイ）へ出した形。左のディスプレイに本体のウィンドウ（3D ビューと重ねた 3D ビュー 3）、
/// 右のディスプレイに 3D ビュー 2 だけの小さなウィンドウ。本体のタブの右クリックの「別ウィンドウで開く」を開いてある。
pub fn draw_window(ui: &mut Ui, ctx: &mut SketchCtx) {
    let area = ui.max_rect();
    ui.advance_cursor_after_rect(area);
    c::cover(ui, "b-window", |ui, screen| {
        // 2 つのディスプレイ（間の黒い帯は、ディスプレイの縁）
        let split = (screen.width() * 0.56).round();
        let left = Rect::from_min_max(
            screen.min,
            pos2(screen.left() + split - 5.0, screen.bottom()),
        );
        let right = Rect::from_min_max(pos2(screen.left() + split + 5.0, screen.top()), screen.max);
        w::fill(ui.painter(), screen, Color32::BLACK);
        w::fill(ui.painter(), left, t::WINDOW_BG);
        w::fill(ui.painter(), right, Color32::from_rgb(22, 24, 30));

        // 本体のウィンドウ（左のディスプレイいっぱい）
        let (context, rects, _) =
            show_dock(ui, ctx, left, "main-window", build_main_window, None, false);

        // 別のウィンドウ（右のディスプレイの中ほど。枠線と影）
        let win = Rect::from_center_size(
            right.center(),
            vec2(right.width() * 0.72, right.height() * 0.66),
        );
        w::rounded(
            ui.painter(),
            win.translate(vec2(0.0, 6.0)).expand(6.0),
            Color32::from_black_alpha(90),
            8.0,
        );
        w::fill(ui.painter(), win, t::WINDOW_BG);
        let _ = show_dock(ui, ctx, win, "detached-window", build_detached, None, true);
        w::outline(ui.painter(), win.expand(0.5), t::SEPARATOR, 1.0, 0.0);

        // 本体のタブ（3D ビュー 3）の右クリックのメニュー
        let tab3 = rects.get(&SLOTS[2]).copied();
        let request = context
            .filter(|(tab, _)| *tab == SLOTS[2])
            .map(|(_, at)| menu::context_anchor(at));
        let anchor = tab3.map(|r| menu::context_anchor(pos2(r.center().x, r.bottom())));
        let entries = vec![MenuEntry::item(
            ctx.pick("別ウィンドウで開く", "Open in New Window"),
            (),
        )];
        let _ = c::popup(
            ui,
            ctx,
            Id::new("mv.b.tab-menu"),
            request,
            anchor,
            &entries,
            &[],
        );
    });
}
