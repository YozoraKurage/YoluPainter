//! 外へ出したウィンドウ（immediate の viewport）を、試験の中で本物の別のパスとして回す。
//!
//! kittest は根の viewport だけを回し、既定では子の viewport を根の中の egui のウィンドウに埋める。ここでは `Context::set_immediate_viewport_renderer`
//! に「子の viewport を、試験が決めた入力で `run_ui` する」口を入れ、埋め込みを切る。子ウィンドウの入力（キー・ポインタ・閉じる頼み）と、
//! ウィンドウの位置（内側の矩形）は試験が `Driver` に入れる。子ウィンドウの絵は撮らない（根の絵だけ）。子のパスが出したテクスチャの差分は、根の描画器へ
//! 先に渡す（子のパスが先に取った差分を、根の描画が使えるように）。
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use egui::{Event, Rect, TexturesDelta, ViewportId, ViewportInfo};
use egui_kittest::TestRenderer;

/// 子ウィンドウ 1 つの、試験が決める様子。
#[derive(Clone, Debug, Default)]
pub struct Child {
    /// 内側の矩形（仮想スクリーンの点。メインウィンドウと同じ拡大率）。
    pub inner: Option<Rect>,
    pub focused: bool,
    /// 次のパスで渡す入力。
    pub events: Vec<Event>,
    /// 次のパスで閉じる頼みを渡す。
    pub close: bool,
    /// 最大化中か。
    pub maximized: bool,
}

#[derive(Default)]
struct State {
    children: HashMap<ViewportId, Child>,
    /// 回った子のパス（古い順）。
    ran: Vec<ViewportId>,
    /// 子のパスのテクスチャの差分（根の描画器が先に当てる）。
    deltas: Vec<TexturesDelta>,
}

/// 子ウィンドウの入力と、回ったパスの記録。
#[derive(Clone, Default)]
pub struct Driver(Arc<Mutex<State>>);

impl Driver {
    /// 試験のスレッドの文脈に、子の viewport を別のパスとして回す口を入れる（埋め込みを切る）。
    pub fn install(&self, ctx: &egui::Context) {
        ctx.set_embed_viewports(false);
        let driver = self.clone();
        egui::Context::set_immediate_viewport_renderer(move |ctx, mut viewport| {
            let id = viewport.ids.this;
            let input = driver.input_for(id);
            let mut out = ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui));
            let mut state = driver.0.lock().unwrap();
            state.ran.push(id);
            state.deltas.push(std::mem::take(&mut out.textures_delta));
        });
    }

    /// 子ウィンドウの様子を変える。
    pub fn child(&self, id: ViewportId, change: impl FnOnce(&mut Child)) {
        let mut state = self.0.lock().unwrap();
        change(state.children.entry(id).or_default());
    }

    /// 回った子のパスを取り出す。
    pub fn take_ran(&self) -> Vec<ViewportId> {
        std::mem::take(&mut self.0.lock().unwrap().ran)
    }

    /// 根の入力に、子ウィンドウの情報を入れる形（根のパスがウィンドウの位置を読む）。
    pub fn infos(&self) -> Vec<(ViewportId, ViewportInfo)> {
        let state = self.0.lock().unwrap();
        state
            .children
            .iter()
            .map(|(id, c)| (*id, info(c)))
            .collect()
    }

    fn input_for(&self, id: ViewportId) -> egui::RawInput {
        let mut state = self.0.lock().unwrap();
        let child = state.children.entry(id).or_default();
        let size = child.inner.map_or(egui::vec2(400.0, 300.0), |r| r.size());
        let mut info = info(child);
        if std::mem::take(&mut child.close) {
            info.events.push(egui::ViewportEvent::Close);
        }
        let events = std::mem::take(&mut child.events);
        let mut viewports = egui::ViewportIdMap::default();
        viewports.insert(id, info);
        egui::RawInput {
            viewport_id: id,
            viewports,
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
            focused: child.focused,
            events,
            ..Default::default()
        }
    }

    /// 根の描画器（子のパスの差分を先に当てる）。
    pub fn renderer<R: TestRenderer>(&self, inner: R) -> Forwarding<R> {
        Forwarding {
            inner,
            driver: self.clone(),
        }
    }
}

fn info(child: &Child) -> ViewportInfo {
    ViewportInfo {
        native_pixels_per_point: Some(1.0),
        inner_rect: child.inner,
        outer_rect: child.inner,
        minimized: Some(false),
        occluded: Some(false),
        focused: Some(child.focused),
        maximized: Some(child.maximized),
        ..Default::default()
    }
}

/// 子のパスのテクスチャの差分を、根の差分より先に当てる描画器。
pub struct Forwarding<R> {
    inner: R,
    driver: Driver,
}

impl<R: TestRenderer> TestRenderer for Forwarding<R> {
    fn setup_eframe(&self, cc: &mut eframe::CreationContext<'_>, frame: &mut eframe::Frame) {
        self.inner.setup_eframe(cc, frame);
    }

    fn handle_delta(&mut self, delta: &mut TexturesDelta) {
        let pending = std::mem::take(&mut self.driver.0.lock().unwrap().deltas);
        for mut d in pending {
            self.inner.handle_delta(&mut d);
        }
        self.inner.handle_delta(delta);
    }

    fn render(
        &mut self,
        ctx: &egui::Context,
        output: &egui::FullOutput,
    ) -> Result<image::RgbaImage, String> {
        self.inner.render(ctx, output)
    }
}
