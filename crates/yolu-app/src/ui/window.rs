//! 浮いた窓（ベイクの窓・確かめ・結果）。Unity 版の浮いた窓と同じく、見出しの帯（アイコン・名前・閉じる）と本体と下の帯を持つ。
//! 見出しをドラッグして動かせる。モーダルの窓は、画面全体に入力の受け皿を敷いて下の部品へ渡さない。
//! 中身は呼ぶ側が `w::*` の部品で描く（窓の矩形を渡す）。

use egui::{pos2, vec2, Color32, Id, Order, Rect, Sense, Ui, Vec2};

use super::theme as t;
use super::widgets::{self as w, Align};

/// 見出しの帯の高さ。
pub const HEADER_HEIGHT: f32 = 28.0;
/// 窓と画面の端の余白。
const SCREEN_MARGIN: f32 = 12.0;

/// 窓の形。
pub struct Spec<'a> {
    pub title: &'a str,
    pub icon: Option<&'a str>,
    /// 欲しい大きさ（画面に収まらなければ縮める）。
    pub size: Vec2,
    /// 下の部品へ入力を渡さない。
    pub modal: bool,
    /// 閉じるボタンの名前（ツールチップ。試験・読み上げ）。
    pub close_label: &'a str,
}

/// 窓の中身を描くときに渡す矩形。
pub struct Frame {
    /// 窓全体。
    pub rect: Rect,
    /// 見出しの帯の下の本体。
    pub body: Rect,
}

/// 浮いた窓（や、浮いて Esc で閉じる部品）を描いた最後のフレームの番号を覚えておく場所。
fn shown_id() -> Id {
    Id::new("yolu.window.shown")
}

/// 浮いた窓を描いたことを覚える（`show` が呼ぶ。Esc で閉じる浮いた部品が自分で描くときも呼ぶ）。
pub fn note_open(ctx: &egui::Context) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| d.insert_temp(shown_id(), frame));
}

/// 浮いた窓が開いているか（今のフレームか、1 つ前のフレームで描いた）。窓が Esc で閉じたフレームも含むので、窓が使う Esc を、
/// 窓より先に描くキャンバスなどが横取りしない（窓を優先する）ために使う。
pub fn any_open(ctx: &egui::Context) -> bool {
    let now = ctx.cumulative_frame_nr();
    ctx.data(|d| d.get_temp::<u64>(shown_id()))
        .is_some_and(|at| now.saturating_sub(at) <= 1)
}

/// Esc を使い切ったフレームの番号を覚えておく場所。
fn escape_taken_id() -> Id {
    Id::new("yolu.window.escape-taken")
}

/// このフレームの Esc を、もう使ったことを覚える。キャンバスより前に描く部品（や、フレームの頭の処理）が Esc で何かをやめたとき、
/// 同じ Esc でキャンバスが選択範囲まで解除しないために呼ぶ（キャンバスより後に描く部品は、`any_open` か自分の状態で足りる）。
pub fn note_escape_taken(ctx: &egui::Context) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| d.insert_temp(escape_taken_id(), frame));
}

/// このフレームの Esc を、キャンバスより前の部品がもう使ったか。
pub fn escape_taken(ctx: &egui::Context) -> bool {
    let now = ctx.cumulative_frame_nr();
    ctx.data(|d| d.get_temp::<u64>(escape_taken_id())) == Some(now)
}

/// 最後に描いた窓の矩形（画面の点。まだ描いていなければ None）。
pub fn last_rect(ctx: &egui::Context, id: Id) -> Option<Rect> {
    ctx.data(|d| d.get_temp(id.with("rect")))
}

/// 窓を描く。見出しの閉じるボタンが押された・Esc（`esc` が true のとき）なら true を返す。`offset` は見出しのドラッグで動いた量
/// （画面の真ん中からの）。
pub fn show(
    ctx: &egui::Context,
    id: Id,
    spec: &Spec<'_>,
    offset: &mut Vec2,
    esc: bool,
    contents: impl FnOnce(&mut Ui, &Frame),
) -> bool {
    let screen = ctx.content_rect();
    let avail = screen.shrink(SCREEN_MARGIN);
    let size = vec2(
        spec.size.x.min(avail.width()),
        spec.size.y.min(avail.height()),
    );
    let center = screen.center() + *offset;
    let mut rect = Rect::from_center_size(center, size);
    // 画面の中に収める
    let dx = (avail.left() - rect.left()).max(0.0) + (avail.right() - rect.right()).min(0.0);
    let dy = (avail.top() - rect.top()).max(0.0) + (avail.bottom() - rect.bottom()).min(0.0);
    rect = rect.translate(vec2(dx, dy));
    *offset += vec2(dx, dy);

    if spec.modal {
        egui::Area::new(id.with("blocker"))
            .order(Order::Middle)
            .fixed_pos(screen.min)
            .constrain(false)
            .interactable(true)
            .show(ctx, |ui| {
                ui.interact(screen, id.with("blocker-hit"), Sense::click_and_drag());
                ui.painter()
                    .rect_filled(screen, 0.0, Color32::from_black_alpha(90));
            });
    }
    // 最後に描いた窓の矩形（試験が窓の中だけを撮る・位置を知るために読む）
    ctx.data_mut(|d| d.insert_temp(id.with("rect"), rect));
    note_open(ctx);
    let mut closed = esc;
    egui::Area::new(id)
        .order(if spec.modal {
            Order::Foreground
        } else {
            Order::Middle
        })
        .fixed_pos(rect.min)
        .constrain(false)
        .show(ctx, |ui| {
            // 窓の上の押下は下へ通さない
            ui.allocate_exact_size(rect.size(), Sense::click_and_drag());
            let p = ui.painter().clone();
            for i in (1..=6).rev() {
                let f = i as f32;
                p.rect_filled(
                    rect.expand(f).translate(vec2(0.0, 2.0)),
                    6.0 + f,
                    Color32::from_black_alpha(14),
                );
            }
            w::rounded(&p, rect, t::PANEL_BG, 6.0);
            let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER_HEIGHT));
            w::rounded(&p, header, t::PANEL_HEADER, 6.0);
            w::fill(
                &p,
                Rect::from_min_max(pos2(header.left(), header.bottom() - 6.0), header.max),
                t::PANEL_HEADER,
            );
            w::hline(
                &p,
                header.left(),
                header.right(),
                header.bottom() - 1.0,
                t::BORDER,
            );
            w::outline(&p, rect, t::SEPARATOR, 1.0, 6.0);
            let mut x = header.left() + 10.0;
            if let Some(name) = spec.icon {
                w::icon(
                    &p,
                    Rect::from_min_size(pos2(x, header.top()), vec2(20.0, header.height())),
                    name,
                    t::TEXT_DIM,
                    16.0,
                );
                x += 26.0;
            }
            let close = Rect::from_min_size(
                pos2(header.right() - 28.0, header.top() + 2.0),
                vec2(24.0, header.height() - 4.0),
            );
            let title = w::fit(&p, spec.title, close.left() - x - 4.0, t::HEADER);
            w::text(
                &p,
                Rect::from_min_max(
                    pos2(x, header.top()),
                    pos2(close.left() - 4.0, header.bottom()),
                ),
                &title,
                t::HEADER,
                Align::Left,
            );
            // 見出しのドラッグで動かす
            let drag = ui.interact(
                Rect::from_min_max(header.min, pos2(close.left(), header.bottom())),
                id.with("drag"),
                Sense::drag(),
            );
            if drag.dragged() {
                *offset += drag.drag_delta();
            }
            if w::icon_button(
                ui,
                close,
                id.with("close"),
                "close",
                spec.close_label,
                false,
                true,
                15.0,
            )
            .clicked()
            {
                closed = true;
            }
            let body = Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max);
            contents(ui, &Frame { rect, body });
        });
    closed
}
