//! 自前のメニュー（Unity 版の `PaintMenu`）。メニューバーの見出し・ドロップダウン・右クリックのメニューが同じポップアップを使う。
//! 開いているあいだは画面全体に入力の受け皿を置き、外を押したら閉じるだけで下の部品（キャンバスなど）へは渡さない。
//! メニューバーから開いたものは、別の見出しへポインタが移ればクリックを待たずに切り替わる（呼ぶ側が `BarOutcome::hovered` を見る）。
//! 項目は行動の値（`A`）を持ち、選ばれたら返す（閉じてから実行するのは呼ぶ側）。`Entry::Submenu` は右に開く入れ子のメニュー（段数の決まりは無い）。
//! 押せない項目の理由はラベルに続けず `Entry::tooltip` に置く（乗せると出る）。

use egui::{pos2, vec2, Color32, Id, Order, Pos2, Rect, Sense, Ui, Vec2, WidgetInfo, WidgetType};

use super::scroll::Scroll;
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
    /// 印の場所にアイコン（モードのドロップダウン）。`on` は今選んでいる行（行を青くする）。
    Icon {
        name: &'static str,
        on: bool,
    },
}

#[derive(Clone, Debug)]
pub enum Entry<A> {
    Item {
        label: String,
        shortcut: Option<String>,
        enabled: bool,
        check: Check,
        action: A,
        /// 乗せると出る説明（押せない項目の理由はここへ。ラベルには続けない）。
        tooltip: Option<String>,
    },
    /// 右に開く入れ子のメニュー（項目の右に ▸）。乗せるか押すと開き、外へ出ると閉じる。入れ子は何段でもよい。
    Submenu {
        label: String,
        entries: Vec<Entry<A>>,
        enabled: bool,
        tooltip: Option<String>,
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
            tooltip: None,
        }
    }
    /// 入れ子のメニューの項目（`entries` が右に開く）。
    pub fn submenu(label: impl Into<String>, entries: Vec<Entry<A>>) -> Entry<A> {
        Entry::Submenu {
            label: label.into(),
            entries,
            enabled: true,
            tooltip: None,
        }
    }
    /// 操作（`commands` の ID）のキーの文字を添える（キーの表の主の行から作る。割り当てが無ければ何も添えない）。
    pub fn command_key(mut self, command: &str) -> Self {
        if let Entry::Item { shortcut, .. } = &mut self {
            *shortcut = crate::shortcuts::menu_key(command);
        }
        self
    }
    pub fn enabled(mut self, on: bool) -> Self {
        match &mut self {
            Entry::Item { enabled, .. } | Entry::Submenu { enabled, .. } => *enabled = on,
            _ => {}
        }
        self
    }
    /// 乗せると出る説明。押せない項目にも出る（理由を言う場所）。
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        match &mut self {
            Entry::Item { tooltip: slot, .. } | Entry::Submenu { tooltip: slot, .. } => {
                *slot = Some(tooltip.into())
            }
            _ => {}
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
    /// 印の場所にアイコンを出す（`on` なら今選んでいる行として青くする）。
    pub fn icon(mut self, name: &'static str, on: bool) -> Self {
        if let Entry::Item { check, .. } = &mut self {
            *check = Check::Icon { name, on };
        }
        self
    }
    fn selectable(&self) -> bool {
        matches!(
            self,
            Entry::Item { enabled: true, .. } | Entry::Submenu { enabled: true, .. }
        )
    }
    fn height(&self) -> f32 {
        if matches!(self, Entry::Separator) {
            SEPARATOR_HEIGHT
        } else {
            ROW_HEIGHT
        }
    }
    /// 項目の名前（見出し・区切りは `None`）。
    pub fn label(&self) -> Option<&str> {
        match self {
            Entry::Item { label, .. } | Entry::Submenu { label, .. } => Some(label),
            Entry::Heading(label) => Some(label),
            Entry::Separator => None,
        }
    }
}

/// 入れ子の中も含めた、押せる項目（`Entry::Item`）の全部（深さ優先。入れ子のメニューの項目そのものは含まない）。
pub fn leaves<A>(entries: &[Entry<A>]) -> Vec<&Entry<A>> {
    let mut out = Vec::new();
    for entry in entries {
        match entry {
            Entry::Item { .. } => out.push(entry),
            Entry::Submenu { entries, .. } => out.extend(leaves(entries)),
            _ => {}
        }
    }
    out
}

/// 1 段（親のメニューか、開いている入れ子のメニュー 1 つ）の中の状態。
#[derive(Clone, Copy, Debug, PartialEq, Default)]
struct Level {
    selected: Option<usize>,
    scroll: f32,
}

/// 開いている入れ子のメニュー（親の段の中のどの行から開いたか・その段の状態・最後に描いた本体の矩形）。
#[derive(Clone, Debug, PartialEq)]
struct Sub {
    row: usize,
    level: Level,
    body: Rect,
}

/// 開いているポップアップの状態（どこに開いたか・選んでいる行・スクロール・開いている入れ子のメニュー）。
#[derive(Clone, Debug, PartialEq)]
pub struct PopupState {
    /// 開いた元の矩形（見出し・箱。右クリックはポインタの点）。
    pub anchor: Rect,
    pub selected: Option<usize>,
    pub scroll: f32,
    /// 箱の幅より狭くしない（ドロップダウン）。
    pub min_width: f32,
    /// 最後に描いた本体の矩形（重なりの判定用。開いている入れ子のメニューも含めた外接）。
    pub rect: Rect,
    opened_frame: u64,
    /// 開いている入れ子のメニュー（根に近いほうから。`subs[0]` は根の段の行から開いたもの）。
    subs: Vec<Sub>,
    /// キーを受ける段（0 が根、n は `subs[n - 1]`）。
    focus: usize,
    /// 開いたウィンドウ（メインウィンドウか、外へ出したウィンドウの viewport）。そのウィンドウのパスだけが描く。
    pub viewport: egui::ViewportId,
}

impl PopupState {
    /// `ctx` の今のウィンドウ（パスを回している viewport）に開く。
    pub fn new(ctx: &egui::Context, anchor: Rect) -> PopupState {
        PopupState {
            anchor,
            selected: None,
            scroll: 0.0,
            min_width: 0.0,
            rect: Rect::NOTHING,
            opened_frame: ctx.cumulative_frame_nr(),
            subs: Vec::new(),
            focus: 0,
            viewport: ctx.viewport_id(),
        }
    }
    pub fn with_min_width(mut self, width: f32) -> Self {
        self.min_width = width;
        self
    }
    /// 開いている入れ子のメニューの段数（根だけなら 0）。
    pub fn open_depth(&self) -> usize {
        self.subs.len()
    }
    /// 開いている入れ子のメニューの、最後に描いた本体の矩形（根に近いほうから）。
    pub fn sub_rects(&self) -> Vec<Rect> {
        self.subs.iter().map(|s| s.body).collect()
    }
    /// 開いている入れ子のメニューの、親の中の行の番号（根に近いほうから）。
    pub fn open_rows(&self) -> Vec<usize> {
        self.subs.iter().map(|s| s.row).collect()
    }

    fn level(&self, depth: usize) -> Level {
        match depth.checked_sub(1) {
            None => Level {
                selected: self.selected,
                scroll: self.scroll,
            },
            Some(i) => self.subs[i].level,
        }
    }
    fn set_level(&mut self, depth: usize, level: Level) {
        match depth.checked_sub(1) {
            None => {
                self.selected = level.selected;
                self.scroll = level.scroll;
            }
            Some(i) => self.subs[i].level = level,
        }
    }
    /// `depth` の段の行 `row` から入れ子のメニューを開く（もう開いていればそのまま。別の行で開いていたものは、その先ごと閉じる）。
    fn open_sub(&mut self, depth: usize, row: usize) {
        if self.subs.get(depth).is_some_and(|s| s.row == row) {
            return;
        }
        self.subs.truncate(depth);
        self.subs.push(Sub {
            row,
            level: Level::default(),
            body: Rect::NOTHING,
        });
    }
}

pub enum PopupOutcome<A> {
    Open,
    Chosen(A),
    Close,
    /// メニューバーの隣の見出しへ（←/→）。
    Step(i32),
}

/// 入れ子のメニューを親の本体に重ねる幅（親の右の端と入れ子の左の端の間に隙間を作らない。ポインタが横切るとき閉じないように）。
const SUB_OVERLAP: f32 = 2.0;

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
            Entry::Submenu { label: l, .. } => {
                label = label.max(w::text_width(painter, l, t::LABEL));
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

/// 入れ子のメニューが親のどちら側に開いたか。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Side {
    #[default]
    Right,
    Left,
}

/// 入れ子のメニューを置く位置（影の余白を含む左上）と、親のどちら側に置いたか。親の本体 `parent` の横に、開いた行 `row` へ先頭の項目の高さをそろえて置く。
/// `prefer` の側（根は右、入れ子の中は親が開いた側）に入ればそこへ、入らなければ反対側へ。親が左へ折り返したのに子が右へ戻ると、子が祖父の上に重なるため。縦は画面の中に収める。
pub fn place_sub(parent: Rect, row: Rect, size: Vec2, screen: Rect, prefer: Side) -> (Rect, Side) {
    let size = vec2(size.x.min(screen.width()), size.y.min(screen.height()));
    let body_width = size.x - MARGIN * 2.0;
    let right_left = parent.right() - SUB_OVERLAP;
    let left_left = parent.left() + SUB_OVERLAP - body_width;
    let fits_right = right_left + body_width <= screen.right();
    let fits_left = left_left >= screen.left();
    let side = match prefer {
        Side::Right if fits_right => Side::Right,
        Side::Right => Side::Left,
        Side::Left if fits_left || !fits_right => Side::Left,
        Side::Left => Side::Right,
    };
    let body_left = match side {
        Side::Right => right_left,
        Side::Left => left_left.max(screen.left()),
    };
    let y = (row.top() - PADDING - MARGIN)
        .min(screen.bottom() - size.y)
        .max(screen.top());
    (Rect::from_min_size(pos2(body_left - MARGIN, y), size), side)
}

/// `path`（各段の行の番号）をたどった先の入れ子のメニューの項目（無い・入れ子でなければ `None`）。
fn entries_at<'a, A>(root: &'a [Entry<A>], path: &[usize]) -> Option<&'a [Entry<A>]> {
    let mut current = root;
    for &row in path {
        match current.get(row) {
            Some(Entry::Submenu { entries, .. }) => current = entries,
            _ => return None,
        }
    }
    Some(current)
}

/// 1 段を描いた結果。
struct LevelOut<A> {
    chosen: Option<A>,
    /// ポインタが動いて乗った行（行の番号・入れ子を開く行か）。
    hovered: Option<(usize, bool)>,
    /// 押された入れ子の行。
    clicked_sub: Option<usize>,
    /// 全部の行の矩形（スクロールで見えない行も含む。入れ子の置き場所の基準）。
    rows: Vec<Rect>,
}

/// 1 段（親のメニューか、入れ子のメニュー 1 つ）を描く。`rect` は影の余白を含む置き場所、`open_row` はこの段から入れ子が開いている行（強調する）。
#[allow(clippy::too_many_arguments)]
fn draw_level<A: Clone>(
    ctx: &egui::Context,
    base: Id,
    rect: Rect,
    entries: &[Entry<A>],
    level: &mut Level,
    open_row: Option<usize>,
) -> LevelOut<A> {
    let body = rect.shrink(MARGIN);
    let content = entries.iter().map(Entry::height).sum::<f32>() + PADDING * 2.0;
    let mut out = LevelOut {
        chosen: None,
        hovered: None,
        clicked_sub: None,
        rows: Vec::new(),
    };
    egui::Area::new(base)
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
            let bar = Scroll::begin(ui, body, content, &mut level.scroll);
            let clip = body.shrink2(vec2(0.0, 1.0));
            let p = p.with_clip_rect(clip);
            let mut y = body.top() + PADDING - level.scroll;
            for (i, entry) in entries.iter().enumerate() {
                let row = Rect::from_min_size(
                    pos2(body.left() + 4.0, y),
                    vec2(body.width() - 8.0 - bar.reserved(), entry.height()),
                );
                y += row.height();
                out.rows.push(row);
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
                        ui.interact(row.intersect(clip), base.with(("row", i)), Sense::hover())
                            .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, label));
                    }
                    Entry::Submenu {
                        label,
                        enabled,
                        tooltip,
                        ..
                    } => {
                        let response = ui.interact(
                            row.intersect(clip),
                            base.with(("row", i)),
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
                            level.selected = Some(i);
                            out.hovered = Some((i, true));
                        }
                        if !*enabled
                            && response.hovered()
                            && ui.input(|inp| inp.pointer.delta() != Vec2::ZERO)
                        {
                            out.hovered = Some((i, false));
                        }
                        if *enabled && response.clicked() {
                            out.clicked_sub = Some(i);
                        }
                        let color = if *enabled { t::TEXT } else { t::TEXT_DISABLED };
                        if (level.selected == Some(i) && *enabled) || open_row == Some(i) {
                            w::rounded(&p, row, t::CONTROL_HOVER, 4.0);
                        }
                        let label_rect = Rect::from_min_size(
                            pos2(row.left() + 28.0, row.top()),
                            vec2((row.width() - 48.0).max(0.0), row.height()),
                        );
                        w::text(
                            &p,
                            label_rect,
                            label,
                            t::LABEL.with_color(color),
                            Align::Left,
                        );
                        w::icon(
                            &p,
                            Rect::from_min_size(
                                pos2(row.right() - 22.0, row.top()),
                                vec2(18.0, row.height()),
                            ),
                            "chevron_right",
                            if *enabled {
                                t::TEXT_DIM
                            } else {
                                t::TEXT_DISABLED
                            },
                            14.0,
                        );
                        response.widget_info(|| {
                            WidgetInfo::labeled(WidgetType::Button, *enabled, label)
                        });
                        if let Some(tip) = tooltip.as_deref().filter(|s| !s.is_empty()) {
                            response.on_hover_text(tip);
                        }
                    }
                    Entry::Item {
                        label,
                        shortcut,
                        enabled,
                        check,
                        action,
                        tooltip,
                    } => {
                        // フォーカスを取らない押し方（取ると矢印キーと Enter をフォーカスの移動に食べられる）
                        let response = ui.interact(
                            row.intersect(clip),
                            base.with(("row", i)),
                            if *enabled {
                                Sense::CLICK
                            } else {
                                Sense::hover()
                            },
                        );
                        let moved = ui.input(|inp| {
                            inp.pointer.delta() != Vec2::ZERO || inp.pointer.any_pressed()
                        });
                        if *enabled && response.hovered() && moved {
                            level.selected = Some(i);
                        }
                        if response.hovered() && moved {
                            out.hovered = Some((i, false));
                        }
                        if *enabled && response.clicked() {
                            out.chosen = Some(action.clone());
                        }
                        let color = if *enabled { t::TEXT } else { t::TEXT_DISABLED };
                        if matches!(check, Check::Icon { on: true, .. }) {
                            w::rounded(&p, row, t::ACCENT_DIM, 4.0);
                        } else if level.selected == Some(i) && *enabled {
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
                            Check::Icon { name, .. } => w::icon(&p, mark, name, color, 16.0),
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
                        let selected = matches!(
                            check,
                            Check::Checked | Check::Radio | Check::Icon { on: true, .. }
                        );
                        response.widget_info(|| {
                            WidgetInfo::selected(WidgetType::Button, *enabled, selected, label)
                        });
                        if let Some(tip) = tooltip.as_deref().filter(|s| !s.is_empty()) {
                            response.on_hover_text(tip);
                        }
                    }
                }
            }
            bar.end(ui, base.with("scroll"), &mut level.scroll);
        });
    out
}

/// 選べる最初の行（無ければ `None`）。
fn first_selectable<A>(entries: &[Entry<A>]) -> Option<usize> {
    entries.iter().position(Entry::selectable)
}

/// ポップアップを描いて、選ばれた行動などを返す。keep は押しても閉じない矩形（メニューバー。そちらの切り替えは呼ぶ側）。
/// 入れ子のメニュー（`Entry::Submenu`）は、乗せるか押すか → キー、で右に開き（画面の端なら左へ）、別の行へ移るか外へ出ると閉じる。
/// キーは一番奥の（フォーカスのある）段が受ける: ↑↓ は行、→ は入れ子を開く（開く行でなければメニューバーの隣へ）、← と Esc は入れ子を 1 段閉じる
/// （開いていなければ ← はメニューバーの隣へ・Esc は全部閉じる）。
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
    // 項目が変わって、開いている入れ子が無くなった・押せなくなったら、そこから先を閉じる
    let mut valid = 0;
    {
        let mut path = Vec::new();
        for sub in &state.subs {
            let ok = entries_at(entries, &path)
                .and_then(|list| list.get(sub.row))
                .is_some_and(|e| matches!(e, Entry::Submenu { enabled: true, .. }));
            if !ok {
                break;
            }
            path.push(sub.row);
            valid += 1;
        }
    }
    state.subs.truncate(valid);
    state.focus = state.focus.min(state.subs.len());

    let painter = ctx.layer_painter(egui::LayerId::new(Order::Tooltip, id));
    let size = measure(&painter, entries, state.min_width);
    let root_rect = place(state.anchor, size, screen);
    let root_body = root_rect.shrink(MARGIN);
    let mut outcome = PopupOutcome::Open;

    // キー（フォーカスのある段が受ける）
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
    {
        let focus = state.focus;
        let path: Vec<usize> = state.subs[..focus].iter().map(|s| s.row).collect();
        let list = entries_at(entries, &path).unwrap_or(&[]);
        let body_height = if focus == 0 {
            root_body.height()
        } else {
            state.subs[focus - 1].body.height()
        };
        if down || up {
            let n = list.len();
            let dir: isize = if down { 1 } else { -1 };
            let mut level = state.level(focus);
            let start =
                level
                    .selected
                    .map(|s| s as isize)
                    .unwrap_or(if down { -1 } else { n as isize });
            for step in 1..=n as isize {
                let i = (start + dir * step).rem_euclid(n as isize) as usize;
                if list[i].selectable() {
                    if level.selected != Some(i) {
                        // 選んでいる行が替わったら、その段から開いていた入れ子は閉じる
                        state.subs.truncate(focus);
                    }
                    level.selected = Some(i);
                    // 選んだ行が見えるように
                    let top: f32 = list[..i].iter().map(Entry::height).sum::<f32>() + PADDING;
                    if top - level.scroll < 0.0 {
                        level.scroll = top - PADDING;
                    } else if body_height > 0.0 && top + ROW_HEIGHT - level.scroll > body_height {
                        level.scroll = top + ROW_HEIGHT + PADDING - body_height;
                    }
                    break;
                }
            }
            state.set_level(focus, level);
        }
        let selected_entry = state.level(focus).selected.and_then(|s| list.get(s));
        let opens = matches!(selected_entry, Some(Entry::Submenu { enabled: true, .. }));
        if escape {
            if state.subs.is_empty() {
                outcome = PopupOutcome::Close;
            } else {
                state.subs.pop();
                state.focus = state.focus.min(state.subs.len());
            }
        } else if left {
            if state.subs.is_empty() {
                outcome = PopupOutcome::Step(-1);
            } else {
                state.subs.pop();
                state.focus = state.focus.min(state.subs.len());
            }
        } else if right || enter {
            if opens {
                // 入れ子を開いて、先頭の行を選ぶ
                let row = state.level(focus).selected.unwrap_or(0);
                state.open_sub(focus, row);
                let mut next = path;
                next.push(row);
                state.subs[focus].level.selected =
                    entries_at(entries, &next).and_then(first_selectable);
                state.focus = focus + 1;
            } else if right {
                if state.subs.is_empty() {
                    outcome = PopupOutcome::Step(1);
                }
            } else if let Some(Entry::Item {
                enabled: true,
                action,
                ..
            }) = selected_entry
            {
                outcome = PopupOutcome::Chosen(action.clone());
            }
        }
    }

    // 段を根から順に描く（開いた入れ子は、このフレームのうちに続けて描く）
    let (moved, pointer) = ctx.input(|i| {
        (
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::PointerMoved(_))),
            i.pointer.hover_pos(),
        )
    });
    let mut bodies = vec![root_body];
    let mut depth = 0;
    let mut rect = root_rect;
    // 親が開いた側（折り返した入れ子の子は、まず同じ側へ。根の子は右から）
    let mut side = Side::Right;
    let mut list = entries;
    loop {
        let base = if depth == 0 {
            id
        } else {
            id.with(("sub", depth))
        };
        let mut level = state.level(depth);
        let open_row = state.subs.get(depth).map(|s| s.row);
        let drawn = draw_level(ctx, base, rect, list, &mut level, open_row);
        state.set_level(depth, level);
        if let Some(action) = drawn.chosen {
            if matches!(outcome, PopupOutcome::Open) {
                outcome = PopupOutcome::Chosen(action);
            }
        }
        if let Some((row, opens)) = drawn.hovered {
            // 乗った行が入れ子ならそれを開き、そうでなければこの段から開いていた入れ子を閉じる
            state.focus = depth;
            if opens {
                state.open_sub(depth, row);
            } else {
                state.subs.truncate(depth);
            }
        }
        if let Some(row) = drawn.clicked_sub {
            state.open_sub(depth, row);
        }
        if depth >= state.subs.len() {
            break;
        }
        let parent_body = rect.shrink(MARGIN);
        let sub_row = state.subs[depth].row;
        let Some(open) = drawn.rows.get(sub_row).copied() else {
            state.subs.truncate(depth);
            break;
        };
        let mut path: Vec<usize> = state.subs[..depth].iter().map(|s| s.row).collect();
        path.push(sub_row);
        let Some(next) = entries_at(entries, &path) else {
            state.subs.truncate(depth);
            break;
        };
        let size = measure(&painter, next, 0.0);
        (rect, side) = place_sub(parent_body, open, size, screen, side);
        state.subs[depth].body = rect.shrink(MARGIN);
        bodies.push(rect.shrink(MARGIN));
        list = next;
        depth += 1;
    }
    // ポインタが動いて、開いているメニューのどれの上でもなくなったら、入れ子は全部閉じる
    if moved && !state.subs.is_empty() {
        if let Some(at) = pointer {
            if !bodies.iter().any(|b| b.contains(at)) {
                state.subs.clear();
                state.focus = 0;
            }
        }
    }
    state.focus = state.focus.min(state.subs.len());
    state.rect = bodies
        .iter()
        .skip(1)
        .fold(bodies[0], |all, b| all.union(*b));

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
            if !bodies.iter().any(|b| b.contains(at)) && !keep.iter().any(|k| k.contains(at)) {
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
            p.circle_filled(
                pos2(item.right() - 6.0, item.center().y - 6.0),
                3.0,
                t::ACCENT,
            );
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
