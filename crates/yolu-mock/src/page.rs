//! 下書きと見本帳を出す殻。本物のドックと同じ見出し（タブ 1 つ）と枠で囲み、本物のパネルと同じ見た目にする。

use std::collections::HashMap;
use std::path::Path;

use egui::{Id, Rect};
use egui_dock::{DockState, TabViewer};
use yolu_app::app::{dock_area, dock_style, Tab, YoluApp};
use yolu_app::lang::Lang;
use yolu_app::state::AppState;
use yolu_app::ui::theme as t;

use crate::args::Options;
use crate::scene::scene_state;
use crate::shoot::Fit;

/// 下書き・見本帳の描く関数。`ui` の `max_rect` がパネルの中身の矩形（本物のパネルと同じ）。
pub type Draw = fn(&mut egui::Ui, &mut SketchCtx);

/// ウィンドウで触ったときに動く、名前で引ける小さな値の置き場（スライダー・切り替え・選び・文字）。
/// まだ無い名前は、読むときの既定の値で始まる。
#[derive(Default)]
pub struct Values {
    map: HashMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq)]
enum Value {
    Bool(bool),
    Float(f32),
    Index(usize),
    Text(String),
}

impl Values {
    pub fn bool(&self, name: &str, default: bool) -> bool {
        match self.map.get(name) {
            Some(Value::Bool(v)) => *v,
            _ => default,
        }
    }
    pub fn set_bool(&mut self, name: &str, value: bool) {
        self.map.insert(name.to_owned(), Value::Bool(value));
    }
    pub fn float(&self, name: &str, default: f32) -> f32 {
        match self.map.get(name) {
            Some(Value::Float(v)) => *v,
            _ => default,
        }
    }
    pub fn set_float(&mut self, name: &str, value: f32) {
        self.map.insert(name.to_owned(), Value::Float(value));
    }
    /// 選びの番号。
    pub fn index(&self, name: &str, default: usize) -> usize {
        match self.map.get(name) {
            Some(Value::Index(v)) => *v,
            _ => default,
        }
    }
    pub fn set_index(&mut self, name: &str, value: usize) {
        self.map.insert(name.to_owned(), Value::Index(value));
    }
    pub fn text(&self, name: &str, default: &str) -> String {
        match self.map.get(name) {
            Some(Value::Text(v)) => v.clone(),
            _ => default.to_owned(),
        }
    }
    pub fn set_text(&mut self, name: &str, value: impl Into<String>) {
        self.map.insert(name.to_owned(), Value::Text(value.into()));
    }
}

/// 描く関数に渡す物。
pub struct SketchCtx {
    /// ダミーの状態（部品が状態を読むときのため。下書きは書き換えない。今ある下書きと見本帳は読まない）。
    #[allow(dead_code)]
    pub app: AppState,
    pub lang: Lang,
    pub values: Values,
    /// 撮る間（`--png`）だけ true。触って開く物（メニュー・ウィンドウ）を、開いた形で出す。
    pub still: bool,
    /// 見本帳が描いた中身の下の端（画面の点）。撮るときの高さを中身に合わせるために、見本帳が書く。
    pub content_bottom: Option<f32>,
}

impl SketchCtx {
    pub fn new(options: &Options, library: &Path, still: bool) -> SketchCtx {
        SketchCtx {
            app: scene_state(options.scene, options.lang, library),
            lang: options.lang,
            values: Values::default(),
            still,
            content_bottom: None,
        }
    }

    /// 言語ごとの文字（下書きの文は日本語と英語を直に書く）。
    pub fn pick(&self, ja: &'static str, en: &'static str) -> &'static str {
        self.lang.pick(ja, en)
    }
}

pub struct SketchApp {
    dock: DockState<Tab>,
    ctx: SketchCtx,
    title: String,
    draw: Draw,
}

impl SketchApp {
    /// 文脈に配色・フォント・アイコンを入れて作る。タブの見出しは `title`。
    pub fn new(egui_ctx: &egui::Context, title: String, draw: Draw, ctx: SketchCtx) -> SketchApp {
        YoluApp::setup(egui_ctx);
        SketchApp {
            // タブの種類は枠のための入れ物（見出しの名前も中身も、ここで差し替える）
            dock: DockState::new(vec![Tab::Layers]),
            ctx,
            title,
            draw,
        }
    }
}

impl Fit for SketchApp {
    fn content_bottom(&self) -> Option<f32> {
        self.ctx.content_bottom
    }
}

/// 本物のドックの中の 1 つのタブ（`app::Tabs` と同じく、選んだタブの上に青い線を引く）。
struct Viewer<'a> {
    ctx: &'a mut SketchCtx,
    title: &'a str,
    draw: Draw,
    tab_rect: Option<Rect>,
}

impl TabViewer for Viewer<'_> {
    type Tab = Tab;

    fn id(&mut self, tab: &mut Tab) -> Id {
        Id::new(("yolu-tab", *tab))
    }

    fn title(&mut self, _tab: &mut Tab) -> egui::WidgetText {
        egui::RichText::new(self.title)
            .font(t::HEADER.font())
            .into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _tab: &mut Tab) {
        if let Some(r) = self.tab_rect {
            let mut p = ui.painter().clone();
            p.set_clip_rect(r);
            p.rect_filled(
                Rect::from_min_size(r.min, egui::vec2(r.width(), 2.0)),
                0.0,
                t::ACCENT,
            );
        }
        (self.draw)(ui, self.ctx);
    }

    fn on_tab_button(&mut self, _tab: &mut Tab, response: &egui::Response) {
        self.tab_rect = Some(response.rect);
    }
}

impl eframe::App for SketchApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(t::WINDOW_BG))
            .show(ui, |ui| {
                let style = dock_style(ui.style());
                let mut viewer = Viewer {
                    ctx: &mut self.ctx,
                    title: &self.title,
                    draw: self.draw,
                    tab_rect: None,
                };
                dock_area(&mut self.dock, Id::new("yolu.mock.dock"), style)
                    .show_inside(ui, &mut viewer);
            });
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(t::WINDOW_BG).to_array()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_start_at_the_default_and_keep_what_was_set() {
        let mut values = Values::default();
        assert!(values.bool("a", true));
        assert_eq!(values.float("b", 3.0), 3.0);
        assert_eq!(values.index("c", 2), 2);
        assert_eq!(values.text("d", "x"), "x");
        values.set_bool("a", false);
        values.set_float("b", 7.5);
        values.set_index("c", 0);
        values.set_text("d", "y");
        assert!(!values.bool("a", true));
        assert_eq!(values.float("b", 3.0), 7.5);
        assert_eq!(values.index("c", 2), 0);
        assert_eq!(values.text("d", "x"), "y");
        // 名前が同じでも、型の違う読みは既定に戻る（別の値を壊さない）
        assert_eq!(values.float("a", 1.0), 1.0);
    }
}
