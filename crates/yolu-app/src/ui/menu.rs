//! 自前のメニュー（Unity 版の `PaintMenu`）。メニューバーの見出し・ドロップダウン・右クリックのメニューが同じポップアップを使う。
//! 開いているあいだは画面全体に入力の受け皿を置き、外を押したら閉じるだけで下の部品（キャンバスなど）へは渡さない。
//! メニューバーから開いたものは、別の見出しへポインタが移ればクリックを待たずに切り替わる（呼ぶ側が `BarOutcome::hovered` を見る）。
//! 項目は行動の値（`A`）を持ち、選ばれたら返す（閉じてから実行するのは呼ぶ側）。

use egui::{pos2, vec2, Color32, Id, Order, Pos2, Rect, Sense, Ui, Vec2, WidgetInfo, WidgetType};

use super::theme as t;
use super::widgets::{self as w, Align};

pub const MARGIN: f32 = 8.0;
pub const PADDING: f32 = 6.0;
pub const ROW_HEIGHT: f32 = 28.0;
pub const SEPARATOR_HEIGHT: f32 = 9.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    None,
    Checked,
    Radio,
}

#[derive(Clone, Debug)]
pub enum Entry<A> {
    Item {
        label: String,
        shortcut: Option<String>,
        enabled: bool,
        check: Check,
        action: A,
    },
    Separator,
    Heading(String),
}

impl<A> Entry<A> {
    pub fn item(label: impl Into<String>, action: A) -> Entry<A> {
        Entry::Item {
            label: label.into(),
            shortcut: None,
            enabled: true,
            check: Check::None,
            action,
        }
    }
    pub fn shortcut(mut self, keys: &str) -> Self {
        if let Entry::Item { shortcut, .. } = &mut self {
            *shortcut = Some(keys.to_owned());
        }
        self
    }
    pub fn enabled(mut self, on: bool) -> Self {
        if let Entry::Item { enabled, .. } = &mut self {
            *enabled = on;
        }
        self
    }
    pub fn checked(mut self, on: bool) -> Self {
        if let Entry::Item { check, .. } = &mut self {
            *check = if on { Check::Checked } else { Check::None };
        }
        self
    }
    pub fn radio(mut self, on: bool) -> Self {
        if let Entry::Item { check, .. } = &mut self {
            *check = if on { Check::Radio } else { Check::None };
        }
        self
    }
    fn selectable(&self) -> bool {
        matches!(self, Entry::Item { enabled: true, .. })
    }
    fn height(&self) -> f32 {
        if matches!(self, Entry::Separator) {
            SEPARATOR_HEIGHT
        } else {
            ROW_HEIGHT
        }
    }
}

/// 開いているポップアップの状態（どこに開いたか・選んでいる行・スクロール）。
#[derive(Clone, Debug, PartialEq)]
pub struct PopupState {
    /// 開いた元の矩形（見出し・箱。右クリックはポインタの点）。
    pub anchor: Rect,
    pub selected: Option<usize>,
    pub scroll: f32,
    /// 箱の幅より狭くしない（ドロップダウン）。
    pub min_width: f32,
    /// 最後に描いた本体の矩形（重なりの判定用）。
    pub rect: Rect,
    opened_frame: u64,
}

impl PopupState {
    pub fn new(ctx: &egui::Context, anchor: Rect) -> PopupState {
        PopupState {
            anchor,
            selected: None,
            scroll: 0.0,
            min_width: 0.0,
            rect: Rect::NOTHING,
            opened_frame: ctx.cumulative_frame_nr(),
        }
    }
    pub fn with_min_width(mut self, width: f32) -> Self {
        self.min_width = width;
        self
    }
}

pub enum PopupOutcome<A> {
    Open,
    Chosen(A),
    Close,
    /// メニューバーの隣の見出しへ（←/→）。
    Step(i32),
}

/// 項目の並びの大きさ（影の余白を含む）。
pub fn measure<A>(painter: &egui::Painter, entries: &[Entry<A>], min_width: f32) -> Vec2 {
    let (mut label, mut keys) = (0.0f32, 0.0f32);
    for e in entries {
        match e {
            Entry::Item {
                label: l, shortcut, ..
            } => {
                label = label.max(w::text_width(painter, l, t::LABEL));
                keys = keys.max(
                    shortcut
                        .as_deref()
                        .map(|s| w::text_width(painter, s, t::LABEL_SMALL))
                        .unwrap_or(0.0),
                );
            }
            Entry::Heading(l) => label = label.max(w::text_width(painter, l, t::HEADER)),
            Entry::Separator => {}
        }
    }
    let width = (label + if keys > 0.0 { keys + 28.0 } else { 0.0 } + 64.0 + MARGIN * 2.0)
        .max(180.0)
        .max(min_width + MARGIN * 2.0);
    let height = entries.iter().map(Entry::height).sum::<f32>() + (MARGIN + PADDING) * 2.0;
    vec2(width, height)
}

/// 置く位置（影の余白を含む左上）。下に入らなければ上に開き、画面の中に収める。
pub fn place(anchor: Rect, size: Vec2, screen: Rect) -> Rect {
    let size = vec2(size.x.min(screen.width()), size.y.min(screen.height()));
    let x = anchor.left() - MARGIN;
    let mut y = anchor.bottom() - MARGIN;
    if y + size.y > screen.bottom() && anchor.top() - size.y + MARGIN >= screen.top() {
        y = anchor.top() - size.y + MARGIN;
    }
    let x = x.clamp(screen.left(), (screen.right() - size.x).max(screen.left()));
    let y = y.clamp(screen.top(), (screen.bottom() - size.y).max(screen.top()));
    Rect::from_min_size(pos2(x, y), size)
}

/// ポップアップを描いて、選ばれた行動などを返す。keep は押しても閉じない矩形（メニューバー。そちらの切り替えは呼ぶ側）。
pub fn show<A: Clone>(
    ctx: &egui::Context,
    id: Id,
    state: &mut PopupState,
    entries: &[Entry<A>],
    keep: &[Rect],
) -> PopupOutcome<A> {
    let screen = ctx.content_rect();
    // 開いているあいだはキーをメニューが受ける（ほかの部品のフォーカスを外す。矢印キーと Enter を取られないように）
    if let Some(focused) = ctx.memory(|m| m.focused()) {
        ctx.memory_mut(|m| m.surrender_focus(focused));
    }
    // 入力の受け皿（外を押しても下の部品へ渡さない）
    egui::Area::new(id.with("blocker"))
        .order(Order::Foreground)
        .fixed_pos(screen.min)
        .constrain(false)
        .interactable(true)
        .show(ctx, |ui| {
            ui.interact(screen, id.with("blocker-hit"), Sense::click_and_drag());
        });
    let painter = ctx.layer_painter(egui::LayerId::new(Order::Tooltip, id));
    let size = measure(&painter, entries, state.min_width);
    let rect = place(state.anchor, size, screen);
    let body = rect.shrink(MARGIN);
    state.rect = body;
    let content = entries.iter().map(Entry::height).sum::<f32>() + PADDING * 2.0;
    let max_scroll = (content - body.height()).max(0.0);
    let mut outcome = PopupOutcome::Open;

    // キー
    let (down, up, enter, escape, left, right) = ctx.input(|i| {
        (
            i.key_pressed(egui::Key::ArrowDown),
            i.key_pressed(egui::Key::ArrowUp),
            i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space),
            i.key_pressed(egui::Key::Escape),
            i.key_pressed(egui::Key::ArrowLeft),
            i.key_pressed(egui::Key::ArrowRight),
        )
    });
    if down || up {
        let n = entries.len();
        let dir: isize = if down { 1 } else { -1 };
        let start =
            state
                .selected
                .map(|s| s as isize)
                .unwrap_or(if down { -1 } else { n as isize });
        for step in 1..=n as isize {
            let i = (start + dir * step).rem_euclid(n as isize) as usize;
            if entries[i].selectable() {
                state.selected = Some(i);
                // 選んだ行が見えるように
                let top: f32 = entries[..i].iter().map(Entry::height).sum::<f32>() + PADDING;
                if top - state.scroll < 0.0 {
                    state.scroll = top - PADDING;
                } else if top + ROW_HEIGHT - state.scroll > body.height() {
                    state.scroll = top + ROW_HEIGHT + PADDING - body.height();
                }
                break;
            }
        }
    }
    if escape {
        outcome = PopupOutcome::Close;
    } else if left {
        outcome = PopupOutcome::Step(-1);
    } else if right {
        outcome = PopupOutcome::Step(1);
    } else if enter {
        if let Some(Entry::Item {
            enabled: true,
            action,
            ..
        }) = state.selected.and_then(|s| entries.get(s))
        {
            outcome = PopupOutcome::Chosen(action.clone());
        }
    }

    egui::Area::new(id)
        .order(Order::Tooltip)
        .fixed_pos(rect.min)
        .constrain(false)
        .show(ctx, |ui| {
            let (_, _) = ui.allocate_exact_size(rect.size(), Sense::hover());
            let p = ui.painter().clone();
            for i in (1..=5).rev() {
                let f = i as f32;
                let shadow = Rect::from_min_max(
                    pos2(rect.left() + MARGIN - f, rect.top() + MARGIN - f + 2.0),
                    pos2(rect.right() - MARGIN + f, rect.bottom() - MARGIN + f + 2.0),
                );
                p.rect_filled(shadow, 8.0 + f, Color32::from_black_alpha(14));
            }
            w::rounded(&p, body, t::PANEL_BG, 8.0);
            w::outline(&p, body, t::SEPARATOR, 1.0, 8.0);
            if ui.rect_contains_pointer(body) {
                let wheel = ui.input(|i| i.smooth_scroll_delta.y);
                if wheel != 0.0 {
                    state.scroll = (state.scroll - wheel).clamp(0.0, max_scroll);
                }
            }
            state.scroll = state.scroll.clamp(0.0, max_scroll);
            let clip = body.shrink2(vec2(0.0, 1.0));
            let p = p.with_clip_rect(clip);
            let mut y = body.top() + PADDING - state.scroll;
            for (i, entry) in entries.iter().enumerate() {
                let row = Rect::from_min_size(
                    pos2(body.left() + 4.0, y),
                    vec2(body.width() - 8.0, entry.height()),
                );
                y += row.height();
                if row.bottom() < clip.top() || row.top() > clip.bottom() {
                    continue;
                }
                match entry {
                    Entry::Separator => w::hline(
                        &p,
                        row.left() + 8.0,
                        row.right() - 8.0,
                        row.center().y.round(),
                        t::SEPARATOR,
                    ),
                    Entry::Heading(label) => {
                        w::text(
                            &p,
                            Rect::from_min_max(pos2(row.left() + 28.0, row.top()), row.max),
                            label,
                            t::HEADER.with_color(t::TEXT_DIM),
                            Align::Left,
                        );
                        // 試験と読み上げのため、見出しの行にも名前を付ける
                        ui.interact(row.intersect(clip), id.with(("row", i)), Sense::hover())
                            .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, label));
                    }
                    Entry::Item {
                        label,
                        shortcut,
                        enabled,
                        check,
                        action,
                    } => {
                        // フォーカスを取らない押し方（取ると矢印キーと Enter をフォーカスの移動に食べられる）
                        let response = ui.interact(
                            row.intersect(clip),
                            id.with(("row", i)),
                            if *enabled {
                                Sense::CLICK
                            } else {
                                Sense::hover()
                            },
                        );
                        if *enabled
                            && response.hovered()
                            && ui.input(|inp| {
                                inp.pointer.delta() != Vec2::ZERO || inp.pointer.any_pressed()
                            })
                        {
                            state.selected = Some(i);
                        }
                        if *enabled && response.clicked() {
                            outcome = PopupOutcome::Chosen(action.clone());
                        }
                        let color = if *enabled { t::TEXT } else { t::TEXT_DISABLED };
                        if state.selected == Some(i) && *enabled {
                            w::rounded(&p, row, t::CONTROL_HOVER, 4.0);
                        }
                        let mark = Rect::from_min_size(
                            pos2(row.left() + 4.0, row.top()),
                            vec2(20.0, row.height()),
                        );
                        match check {
                            Check::Checked => w::icon(&p, mark, "check", color, 14.0),
                            Check::Radio => {
                                p.circle_filled(mark.center(), 3.5, color);
                            }
                            Check::None => {}
                        }
                        let key_width = shortcut
                            .as_deref()
                            .map(|s| w::text_width(&p, s, t::LABEL_SMALL))
                            .unwrap_or(0.0);
                        let label_rect = Rect::from_min_size(
                            pos2(row.left() + 28.0, row.top()),
                            vec2(
                                (row.width()
                                    - 48.0
                                    - if key_width > 0.0 {
                                        key_width + 24.0
                                    } else {
                                        0.0
                                    })
                                .max(0.0),
                                row.height(),
                            ),
                        );
                        w::text(
                            &p,
                            label_rect,
                            label,
                            t::LABEL.with_color(color),
                            Align::Left,
                        );
                        if let Some(keys) = shortcut {
                            let kr = Rect::from_min_size(
                                pos2(row.right() - 18.0 - key_width, row.top()),
                                vec2(key_width, row.height()),
                            );
                            w::text(
                                &p,
                                kr,
                                keys,
                                t::LABEL_SMALL.with_color(if *enabled {
                                    t::TEXT_DIM
                                } else {
                                    t::TEXT_DISABLED
                                }),
                                Align::Left,
                            );
                        }
                        let selected = matches!(check, Check::Checked | Check::Radio);
                        response.widget_info(|| {
                            WidgetInfo::selected(WidgetType::Button, *enabled, selected, label)
                        });
                    }
                }
            }
            if max_scroll > 0.0 {
                let bar_h = body.height() * body.height() / content;
                let bar_y = body.top() + (body.height() - bar_h) * (state.scroll / max_scroll);
                w::rounded(
                    &ui.painter().clone(),
                    Rect::from_min_size(pos2(body.right() - 6.0, bar_y), vec2(4.0, bar_h)),
                    t::CONTROL_ACTIVE,
                    2.0,
                );
            }
        });

    // 外を押したら閉じる（開いた押下そのものは数えない）
    if matches!(outcome, PopupOutcome::Open) && ctx.cumulative_frame_nr() != state.opened_frame {
        let press = ctx.input(|i| {
            if i.pointer.any_pressed() {
                i.pointer.press_origin()
            } else {
                None
            }
        });
        if let Some(at) = press {
            if !body.contains(at) && !keep.iter().any(|k| k.contains(at)) {
                outcome = PopupOutcome::Close;
            }
        }
    }
    outcome
}

/// メニューバーの結果: 見出しの矩形、押された見出し、ポインタが動いて乗った見出し。
pub struct BarOutcome {
    pub rects: Vec<Rect>,
    pub pressed: Option<usize>,
    pub hovered: Option<usize>,
}

/// メニューバー: 見出しを並べる。押下とホバーは生の入力で見る（開いているメニューの受け皿が上にあっても切り替えられるように）。
pub fn menu_bar(ui: &mut Ui, r: Rect, titles: &[&str], open: Option<usize>) -> BarOutcome {
    menu_bar_marked(ui, r, titles, open, None)
}

/// `menu_bar` に、`marked` の見出しの右上へ小さな印（青い点。新しい版があるときのヘルプなど）を付けたもの。
pub fn menu_bar_marked(
    ui: &mut Ui,
    r: Rect,
    titles: &[&str],
    open: Option<usize>,
    marked: Option<usize>,
) -> BarOutcome {
    let p = ui.painter().clone();
    w::fill(&p, r, t::MENU_BG);
    w::hline(&p, r.left(), r.right(), r.bottom() - 1.0, t::BORDER);
    // ホバーの切り替えはポインタが動いたフレームだけ（キーで隣へ移ったあと、止まっているポインタに戻されないように）
    let (pointer, moved, pressed_at) = ui.input(|i| {
        (
            i.pointer.hover_pos(),
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::PointerMoved(_))),
            if i.pointer.primary_pressed() {
                i.pointer.press_origin()
            } else {
                None
            },
        )
    });
    let mut x = r.left() + 6.0;
    let mut out = BarOutcome {
        rects: Vec::new(),
        pressed: None,
        hovered: None,
    };
    for (i, title) in titles.iter().enumerate() {
        let width = w::text_width(&p, title, t::LABEL) + 18.0;
        let item = Rect::from_min_size(pos2(x, r.top() + 2.0), vec2(width, r.height() - 4.0));
        let response = ui.interact(item, ui.make_persistent_id(("menubar", i)), Sense::CLICK);
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, *title));
        let hover = pointer.is_some_and(|at| item.contains(at));
        if hover && moved {
            out.hovered = Some(i);
        }
        if pressed_at.is_some_and(|at| item.contains(at)) {
            out.pressed = Some(i);
        }
        if hover || open == Some(i) {
            w::rounded(&p, item, t::CONTROL_HOVER, 3.0);
        }
        w::text(&p, item, title, t::LABEL, Align::Center);
        if marked == Some(i) {
            p.circle_filled(pos2(item.right() - 6.0, item.center().y - 6.0), 3.0, t::ACCENT);
        }
        out.rects.push(item);
        x += width;
    }
    out
}

/// ポインタの位置に開く右クリックのメニューの元の矩形。
pub fn context_anchor(at: Pos2) -> Rect {
    Rect::from_min_size(at, Vec2::ZERO)
}
