//! 部品の見本帳。部品ごとに、名前（Rust の関数・型の名前）と短い用途の 1 行を添えて、本物の部品で描く。節に分け、1 つの節の中は 2 列に並べる。
//!
//! 追加するとき: 節のファイルの `section()` の `entries` の末尾に `Entry::new(名前, (用途の日本語, 英語), 高さ, 描く関数)` を追加する。
//! 描く関数は `fn(&mut Ui, Rect, &mut SketchCtx)`（`Rect` は見本の枠。部品は絶対座標なので、この中に収める）。名前は見本帳の中で 1 つだけ。
//! 触って開く物（メニュー・ウィンドウ）は、`ctx.still`（撮る間）なら開いた形で、そうでなければボタンで開く。

use egui::{pos2, vec2, Rect, Sense, Ui, UiBuilder};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::{self as w, Align};

use crate::page::SketchCtx;

mod buttons;
mod colors;
mod curves;
mod icons;
mod lists;
mod numbers;
mod panel_parts;
mod pickers;
mod windows;

/// 見本を描く関数。`Rect` は見本の枠。
pub type Sample = fn(&mut Ui, Rect, &mut SketchCtx);

/// 見本 1 つ（名前・用途・見本の高さ・描く関数）。
pub struct Entry {
    pub name: &'static str,
    pub note: (&'static str, &'static str),
    /// 見本の枠の高さ（幅は列の幅の 300 点。`wide` なら全幅）。
    pub height: f32,
    pub wide: bool,
    /// 見本の枠の幅から高さを決める物（幅によって並びの行数が変わる見本）。
    pub height_at: Option<fn(f32) -> f32>,
    pub draw: Sample,
}

impl Entry {
    pub fn new(
        name: &'static str,
        note: (&'static str, &'static str),
        height: f32,
        draw: Sample,
    ) -> Entry {
        Entry {
            name,
            note,
            height,
            wide: false,
            height_at: None,
            draw,
        }
    }

    /// 幅から見本の高さを決める。
    pub fn height_at(mut self, f: fn(f32) -> f32) -> Entry {
        self.height_at = Some(f);
        self
    }

    fn height_for(&self, width: f32) -> f32 {
        self.height_at.map_or(self.height, |f| f(width))
    }

    /// 全幅の見本（アイコンの一覧など）。
    pub fn wide(mut self) -> Entry {
        self.wide = true;
        self
    }
}

pub struct Section {
    pub title: (&'static str, &'static str),
    pub entries: Vec<Entry>,
}

/// 節の並び（見本帳の上から下）。
pub fn sections() -> Vec<Section> {
    vec![
        colors::section(),
        buttons::section(),
        numbers::section(),
        pickers::section(),
        lists::section(),
        windows::section(),
        curves::section(),
        icons::section(),
        panel_parts::section(),
    ]
}

/// 見本の枠の幅（本物のドックの幅 300 点）と、余白・列の間・行の間。
const SAMPLE_WIDTH: f32 = t::DOCK_WIDTH;
const MARGIN: f32 = 10.0;
const COLUMN_GAP: f32 = 12.0;
const ROW_GAP: f32 = 14.0;
/// 見本の上の、名前（1 行）と用途（1 行）の高さ。
const HEAD: f32 = 34.0;
const SECTION_HEIGHT: f32 = 26.0;

pub fn draw(ui: &mut Ui, ctx: &mut SketchCtx) {
    let out = egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| content(ui, ctx));
    // 撮るときの高さを中身に合わせる（スクロールを動かしていない前提）
    ctx.content_bottom = Some(out.inner_rect.top() + out.content_size.y + MARGIN);
}

fn content(ui: &mut Ui, ctx: &mut SketchCtx) {
    let avail = ui.available_width();
    let columns = (((avail - 2.0 * MARGIN + COLUMN_GAP) / (SAMPLE_WIDTH + COLUMN_GAP)).floor()
        as usize)
        .max(1);
    let column_width = SAMPLE_WIDTH.min((avail - 2.0 * MARGIN).max(100.0));
    ui.allocate_space(vec2(avail, MARGIN));
    for section in sections() {
        let (band, _) = ui.allocate_exact_size(vec2(avail, SECTION_HEIGHT), Sense::hover());
        let p = ui.painter();
        w::fill(p, band, t::SECTION_BAND);
        w::hline(p, band.left(), band.right(), band.top(), t::BORDER);
        w::hline(p, band.left(), band.right(), band.bottom() - 1.0, t::BORDER);
        w::text(
            p,
            Rect::from_min_max(pos2(band.left() + MARGIN, band.top()), band.max),
            ctx.pick(section.title.0, section.title.1),
            t::HEADER,
            Align::Left,
        );
        ui.allocate_space(vec2(avail, 10.0));

        let origin = ui.cursor().min;
        let mut heights = vec![0.0_f32; columns];
        for entry in &section.entries {
            let width = if entry.wide {
                avail - 2.0 * MARGIN
            } else {
                column_width
            };
            let height = entry.height_for(width);
            let total = HEAD + height + ROW_GAP;
            let rect = if entry.wide {
                let y = heights.iter().copied().fold(0.0, f32::max);
                heights.fill(y + total);
                Rect::from_min_size(
                    pos2(origin.x + MARGIN, origin.y + y),
                    vec2(width, HEAD + height),
                )
            } else {
                let (column, y) =
                    heights
                        .iter()
                        .copied()
                        .enumerate()
                        .fold(
                            (0, f32::MAX),
                            |best, (i, y)| {
                                if y < best.1 {
                                    (i, y)
                                } else {
                                    best
                                }
                            },
                        );
                heights[column] += total;
                Rect::from_min_size(
                    pos2(
                        origin.x + MARGIN + column as f32 * (column_width + COLUMN_GAP),
                        origin.y + y,
                    ),
                    vec2(width, HEAD + height),
                )
            };
            show_entry(ui, ctx, rect, entry);
        }
        let used = heights.iter().copied().fold(0.0, f32::max);
        ui.allocate_space(vec2(avail, used));
    }
}

fn show_entry(ui: &mut Ui, ctx: &mut SketchCtx, rect: Rect, entry: &Entry) {
    // 見本ごとに別の ID の子（部品の名前が見本の間で重ならない）。親の位置は動かさない
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .id_salt(("gallery", entry.name)),
    );
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    let p = child.painter().clone();
    let name = w::fit(&p, entry.name, rect.width(), t::LABEL_BOLD);
    w::text(
        &p,
        Rect::from_min_size(rect.min, vec2(rect.width(), 16.0)),
        &name,
        t::LABEL_BOLD.with_color(t::ACCENT),
        Align::Left,
    );
    // 用途は 1 行（`notes_fit_one_line` が長さを見る。収まらなければ「…」で詰める）
    let note = w::fit(
        &p,
        ctx.pick(entry.note.0, entry.note.1),
        rect.width(),
        t::LABEL_DIM,
    );
    w::text(
        &p,
        Rect::from_min_size(
            pos2(rect.left(), rect.top() + 17.0),
            vec2(rect.width(), 16.0),
        ),
        &note,
        t::LABEL_DIM,
        Align::Left,
    );
    let sample = Rect::from_min_max(pos2(rect.left(), rect.top() + HEAD), rect.max);
    w::rounded(&p, sample, t::PANEL_BG, 3.0);
    w::outline(&p, sample, t::SEPARATOR, 1.0, 3.0);
    (entry.draw)(&mut child, sample, ctx);
}

/// 見本の中の行（上から積む。左右に余白）。上の余白は呼ぶ側が `space` で空ける。
pub fn rows(sample: Rect) -> w::Rows {
    w::Rows::new(sample, 0.0)
}

/// 部品の状態を egui の一時のデータに覚える（`Values` に入らない型の物。ウィンドウで触ったときに値が動くように）。
pub fn remember<T: Clone + Send + Sync + 'static>(
    ui: &Ui,
    key: &str,
    default: impl FnOnce() -> T,
) -> T {
    let id = ui.id().with(("gallery.remember", key));
    ui.data_mut(|d| d.get_temp_mut_or_insert_with(id, default).clone())
}

pub fn store<T: Clone + Send + Sync + 'static>(ui: &Ui, key: &str, value: T) {
    let id = ui.id().with(("gallery.remember", key));
    ui.data_mut(|d| d.insert_temp(id, value));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_names_are_unique_and_every_section_has_entries() {
        let mut seen = std::collections::HashSet::new();
        for section in sections() {
            assert!(!section.entries.is_empty(), "{}", section.title.0);
            for entry in &section.entries {
                assert!(seen.insert(entry.name), "名前が重なる: {}", entry.name);
                assert!(entry.height > 0.0, "{}", entry.name);
                assert!(!entry.note.0.is_empty() && !entry.note.1.is_empty());
            }
        }
    }

    /// 用途は見本の上の 1 行（幅 300 点）に収める。日本語は 26 字、英語は 52 字まで。
    #[test]
    fn notes_fit_one_line() {
        for section in sections() {
            for entry in &section.entries {
                assert!(entry.note.0.chars().count() <= 26, "{}", entry.name);
                assert!(entry.note.1.chars().count() <= 52, "{}", entry.name);
            }
        }
    }

    #[test]
    fn the_sections_are_the_ones_the_gallery_promises() {
        let titles: Vec<_> = sections().iter().map(|s| s.title.0).collect();
        for want in [
            "色と文字",
            "ボタンと切り替え",
            "数の入力とスライダー",
            "選びとメニュー",
            "一覧とサムネイル",
            "ウィンドウ",
            "曲線とランプ",
            "アイコン",
        ] {
            assert!(titles.contains(&want), "{want}");
        }
    }
}
