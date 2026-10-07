//! チャンネルのパネル（Substance Painter のテクスチャセットのチャンネルの一覧）: 文書のチャンネル（標準の 6 つとユーザーチャンネル）を
//! 行にし、筆のボタンで描くチャンネル、目で 2D のキャンバスに出すチャンネルを選ぶ。ユーザーチャンネルは名前（ダブルクリック）・
//! 種類（右端の形式の名前「sRGB8」「L8」「RGB8」を押す）を変えられ、下の帯で追加・削除する。どれも文書の操作（1 回の Undo）で、画面だけの選択は `UiOp`。
//! 見た目が lilToon のときは、ユーザーチャンネルを、読むスロットの lilToon の部位（インスペクターの節の並び）ごとのまとまりにして
//! 折りたためる（標準の 6 つは上のまま。どの部位にも入らないものは「そのほか」）。見た目が使っていない部位のまとまりは、開いたことが
//! 無ければたたんでおく。

use egui::{pos2, vec2, Color32, Rect, Sense, Ui, WidgetInfo, WidgetType};

use super::properties::{group_label, open_popup, section, slider_row, toggle_row};
use crate::engine::{Channel, ChannelInfo, NormalSettings};
use crate::look::{liltoon, Section};
use crate::m2::{self, direction_name, Edit, UiOp};
use crate::m2_menu::{edges_name, Popup};
use crate::state::{Action, AppState, OpenPopup, PopupKind};
use crate::ui::menu::{context_anchor, PopupState};
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, NumberFormat, Rows};

pub const ROW_HEIGHT: f32 = 28.0;
pub const TOOLBAR_HEIGHT: f32 = 30.0;
/// 右端の種類の欄の、形式の名前の外の幅（選ぶ箱の左の余白・矢印と、その間の隙間）。
const KIND_PAD: f32 = 32.0;

/// まとまりの見出しの高さ。
const GROUP_HEIGHT: f32 = 24.0;

/// ユーザーチャンネルのまとまり: 読むスロットの lilToon の部位か、どの部位のスロットも読まないもの。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Look(Section),
    Other,
}

impl Part {
    /// 並び（lilToon のインスペクターの節の並び、最後に「そのほか」）。
    fn all() -> impl Iterator<Item = Part> {
        liltoon::SLOT_SECTIONS
            .iter()
            .map(|(section, _)| Part::Look(*section))
            .chain(std::iter::once(Part::Other))
    }

    /// 開閉を覚える名前（画面の状態）。
    fn key(self) -> &'static str {
        match self {
            Part::Look(Section::Main) => "channels.part.main",
            Part::Look(Section::Shadow) => "channels.part.shadow",
            Part::Look(Section::RimShade) => "channels.part.rimshade",
            Part::Look(Section::Emission) => "channels.part.emission",
            Part::Look(Section::Normal) => "channels.part.normal",
            Part::Look(Section::Backlight) => "channels.part.backlight",
            Part::Look(Section::Reflection) => "channels.part.reflection",
            Part::Look(Section::MatCap) => "channels.part.matcap",
            Part::Look(Section::Rim) => "channels.part.rim",
            Part::Look(Section::Glitter) => "channels.part.glitter",
            Part::Look(Section::Outline) => "channels.part.outline",
            Part::Look(_) | Part::Other => "channels.part.other",
        }
    }

    fn label(self, lang: crate::lang::Lang) -> &'static str {
        match self {
            Part::Look(Section::Main) => lang.pick("メインカラー", "Main Color"),
            Part::Look(Section::Shadow) => lang.pick("影", "Shadow"),
            Part::Look(Section::RimShade) => lang.pick("リムシェード", "RimShade"),
            Part::Look(Section::Emission) => lang.pick("発光", "Emission"),
            Part::Look(Section::Normal) => lang.pick("ノーマルマップ", "Normal Map"),
            Part::Look(Section::Backlight) => lang.pick("逆光ライト", "Backlight"),
            Part::Look(Section::Reflection) => lang.pick("光沢", "Reflection"),
            Part::Look(Section::MatCap) => lang.pick("マットキャップ", "MatCap"),
            Part::Look(Section::Rim) => lang.pick("リムライト", "Rim Light"),
            Part::Look(Section::Glitter) => lang.pick("ラメ", "Glitter"),
            Part::Look(Section::Outline) => lang.pick("輪郭線", "Outline"),
            Part::Look(_) | Part::Other => lang.pick("そのほか", "Other"),
        }
    }
}

/// 一覧の 1 段: チャンネルの行か、まとまりの見出し。
#[derive(Clone, Copy, Debug)]
enum Entry {
    Row(Channel),
    Group {
        part: Part,
        open: bool,
        /// 描くチャンネルがこのまとまりにある（たたんでいても分かるように、見出しに印）。
        painting: bool,
    },
}

impl Entry {
    fn height(self) -> f32 {
        match self {
            Entry::Row(_) => ROW_HEIGHT,
            Entry::Group { .. } => GROUP_HEIGHT,
        }
    }
}

/// 一覧の並び: 標準の 6 つ（まとまり無し）。見た目が lilToon なら、ユーザーチャンネルを部位ごとのまとまりにしてインスペクターの並びで
/// （たたんだまとまりの行は出さない。開閉を覚えていなければ、見た目がその部位を使っているときだけ開く）。lilToon でなければ、
/// ユーザーチャンネルをそのまま並べる。
fn entries(app: &AppState, channels: &[Channel]) -> Vec<Entry> {
    let mut out: Vec<Entry> = channels
        .iter()
        .filter(|c| c.is_standard())
        .map(|c| Entry::Row(*c))
        .collect();
    let user: Vec<Channel> = channels
        .iter()
        .filter(|c| !c.is_standard())
        .copied()
        .collect();
    let look = app.doc.drawn_look();
    if look.kind != yolu_core::look::LookKind::LilToon {
        out.extend(user.into_iter().map(Entry::Row));
        return out;
    }
    let part_of =
        |c: Channel| crate::look::channel_section(&app.doc, c).map_or(Part::Other, Part::Look);
    let parts: Vec<(Channel, Part)> = user.iter().map(|c| (*c, part_of(*c))).collect();
    // どの部位のスロットも読まないチャンネルだけなら、まとめない（見出しが「そのほか」1 つだけにならないように）
    if parts.iter().all(|(_, p)| *p == Part::Other) {
        out.extend(user.into_iter().map(Entry::Row));
        return out;
    }
    for part in Part::all() {
        let members: Vec<Channel> = parts
            .iter()
            .filter(|(_, p)| *p == part)
            .map(|(c, _)| *c)
            .collect();
        if members.is_empty() {
            continue;
        }
        let used = match part {
            Part::Look(section) => liltoon::section_in_use(look, section),
            Part::Other => true,
        };
        let open = app.section_open(part.key(), used);
        let painting = members.contains(&app.m2.paint_channel);
        out.push(Entry::Group {
            part,
            open,
            painting,
        });
        if open {
            out.extend(members.into_iter().map(Entry::Row));
        }
    }
    out
}

/// まとまりの見出し（押すと開閉。開閉は画面の状態）。
fn group_header(
    ui: &mut Ui,
    app: &mut AppState,
    list: Rect,
    r: Rect,
    part: Part,
    open: bool,
    painting: bool,
) {
    let painter = ui.painter_at(list);
    w::fill(&painter, r, t::PANEL_BG);
    w::hline(&painter, r.left(), r.right(), r.bottom() - 1.0, t::BORDER);
    if painting && !open {
        w::fill(
            &painter,
            Rect::from_min_size(r.min, vec2(3.0, r.height())),
            t::ACCENT,
        );
    }
    let header = Rect::from_min_max(
        pos2(r.left() + 8.0, r.top() + 2.0),
        pos2(r.right() - 4.0, r.bottom() - 2.0),
    )
    .intersect(list);
    let next = w::subsection_header(
        ui,
        header,
        ("channels.part", part.key()),
        part.label(app.lang),
        open,
    );
    if next != open {
        app.ui.sections.insert(part.key(), next);
    }
}

/// 右端の種類の欄（形式の名前「sRGB8」「L8」「RGB8」）。ユーザーチャンネルの選ぶ箱は、並ぶ箱の文字のいちばん長いものに揃えた幅。
#[derive(Clone, Copy)]
struct KindColumn {
    dropdown: f32,
}

impl KindColumn {
    fn fit(ui: &Ui, app: &AppState, channels: &[Channel]) -> KindColumn {
        let p = ui.painter();
        let dropdown = channels
            .iter()
            .filter(|c| !c.is_standard())
            .filter_map(|c| app.doc.channel_info(*c))
            .map(|i| w::text_width(p, &m2::channel_format(i), t::LABEL))
            .fold(0.0, f32::max)
            + KIND_PAD;
        KindColumn { dropdown }
    }

    /// その行の種類の欄の幅（標準のチャンネルは文字の幅だけ、ユーザーチャンネルは揃えた箱の幅）。
    fn width(self, p: &egui::Painter, channel: Channel, format: &str) -> f32 {
        if channel.is_standard() {
            w::text_width(p, format, t::LABEL_DIM)
        } else {
            self.dropdown
        }
    }
}

fn open(app: &mut AppState, ctx: &egui::Context, kind: PopupKind, anchor: Rect) {
    app.popup = Some(OpenPopup {
        kind,
        state: PopupState::new(ctx, anchor),
    });
}

pub fn show(ui: &mut Ui, app: &mut AppState) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    let ctx = ui.ctx().clone();
    let lang = app.lang;
    let enabled = app.can_edit();
    // 一覧と、その下の Normal の設定は 1 つのスクロールで動く（下の帯の足す・消すは動かない）
    let body = Rect::from_min_max(
        r.min,
        pos2(r.right(), (r.bottom() - TOOLBAR_HEIGHT).max(r.top())),
    );
    let channels = app.doc.channels();
    let entries = entries(app, &channels);
    let list_height: f32 = entries.iter().map(|e| e.height()).sum();
    let bar = Scroll::begin(
        ui,
        body,
        app.m2.channels_content,
        &mut app.m2.channel_scroll,
    );
    let scroll = app.m2.channel_scroll;
    w::fill(&ui.painter_at(body), body, t::PANEL_BG);
    let list = Rect::from_min_size(
        pos2(body.left(), body.top() - scroll),
        vec2(body.width(), list_height),
    );
    w::fill(&ui.painter_at(body), list, t::CONTROL_BG);
    // つまみは行に重ねて出す（浮かぶつまみ。カラーセットの欄と同じ。行の幅を狭めると、名前が切れる）
    let row_width = list.width();
    let kind = KindColumn::fit(ui, app, &channels);
    let mut y = list.top();
    for entry in entries {
        let row = Rect::from_min_size(pos2(list.left(), y), vec2(row_width, entry.height()));
        y += entry.height();
        if row.bottom() < body.top() || row.top() > body.bottom() {
            continue;
        }
        match entry {
            Entry::Row(channel) => channel_row(ui, app, &ctx, body, row, channel, kind),
            Entry::Group {
                part,
                open,
                painting,
            } => group_header(ui, app, body, row, part, open, painting),
        }
    }
    // 一覧の下: Normal の設定
    let outer_clip = ui.clip_rect();
    ui.set_clip_rect(body.intersect(outer_clip));
    let area = Rect::from_min_max(
        pos2(body.left(), list.bottom()),
        pos2(
            body.right() - bar.reserved(),
            body.bottom().max(list.bottom() + 1.0),
        ),
    );
    let mut rows = Rows::new(area, 0.0);
    normal_section(ui, app, &mut rows, &ctx);
    rows.indent = 0.0;
    rows.space(8.0);
    app.m2.channels_content = list_height + rows.used();
    ui.set_clip_rect(outer_clip);
    // スライダーのドラッグを離したら、まとめていた変更を 1 回の Undo にする（ギズモと点のドラッグ中は終えない。properties.rs の同じ所の注記）
    if !ui.input(|i| i.pointer.primary_down()) && !crate::fillfx::dragging(app) {
        app.m2_end_drag();
    }
    bar.end(ui, "channels.scroll", &mut app.m2.channel_scroll);

    // 下: 足す・消す
    let bar = Rect::from_min_size(
        pos2(r.left(), body.bottom()),
        vec2(r.width(), TOOLBAR_HEIGHT),
    );
    w::fill(ui.painter(), bar, t::PANEL_HEADER);
    let button = |x: f32| Rect::from_min_size(pos2(x, bar.top() + 3.0), vec2(26.0, 24.0));
    let add = button(bar.left() + 4.0);
    if w::icon_button(
        ui,
        add,
        "channels.add",
        "add",
        lang.pick("チャンネルを追加", "Add Channel"),
        false,
        enabled,
        18.0,
    )
    .clicked()
    {
        open(app, &ctx, PopupKind::M2(Popup::NewChannel), add);
    }
    let paint = app.m2.paint_channel;
    if w::icon_button(
        ui,
        button(bar.right() - 4.0 - 26.0),
        "channels.delete",
        "delete",
        lang.pick("チャンネルを削除", "Delete Channel"),
        false,
        enabled && !paint.is_standard(),
        17.0,
    )
    .clicked()
    {
        app.apply(Action::M2(Edit::RemoveChannel(paint)));
    }
}

/// Normal の設定の節（文書の Normal の出力: Height から作る・強さ・端・ファイルの Y の向き）。値の変更は文書の操作（1 回の Undo。
/// 強さのスライダーのドラッグは離したとき 1 回にまとまる）。強さと端は Height から作るときだけ効く。
fn normal_section(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, ctx: &egui::Context) {
    let lang = app.lang;
    // 初めは閉じておく（チャンネルの一覧を押しのけない。開いた・閉じたは覚える）
    app.ui.sections.entry("normal").or_insert(false);
    let (open, _) = section(
        ui,
        app,
        rows,
        "normal",
        lang.pick("ノーマル", "Normal"),
        "3d_rotation",
        None,
    );
    if !open {
        return;
    }
    let settings = app.doc.normal_settings();
    let enabled = app.can_edit();
    let derive = settings.derive_from_height();
    let edit = |app: &mut AppState, settings: NormalSettings, coalesce: bool| {
        app.apply(Action::M2(Edit::NormalSettings { settings, coalesce }));
    };
    if let Some(next) = toggle_row(
        ui,
        rows,
        "normal.derive",
        lang.pick("ハイト → ノーマル", "Height → Normal"),
        derive,
        Some(lang.pick(
            "ノーマルの出力（プレビュー・.ylp のテクスチャ・書き出し）で、描いたノーマルのレイヤーの下にハイトのチャンネルから作った法線を足します。ハイトから毎回作り直し、レイヤーには描きません",
            "Adds the normal derived from the Height channel under the painted Normal layers in the Normal output (preview, .ylp texture, exports). It is regenerated from Height, never painted into a layer",
        )),
        enabled,
    ) {
        edit(app, settings.with_derive(next), false);
    }
    if let Some(value) = slider_row(
        ui,
        rows,
        "normal.strength",
        lang.pick("強さ", "Strength"),
        settings.strength() as f32,
        (
            -(NormalSettings::MAX_STRENGTH as f32),
            NormalSettings::MAX_STRENGTH as f32,
        ),
        NumberFormat {
            decimals: 2,
            trim: true,
            suffix: "",
        },
        Some(lang.pick(
            "ハイトの全範囲（0 → 1）で何テクセル分盛り上がるか。マイナスにすると凸が凹になります",
            "Texels of rise for the full height range (0 → 1). Negative turns bumps into dents",
        )),
        enabled && derive,
    ) {
        if let Ok(next) = settings.with_strength(value as f64) {
            edit(app, next, true);
        }
    }
    // 狭い欄でも値を切らないよう、名前を上に置いて箱は幅いっぱいに
    let dropdown =
        |ui: &mut Ui, rows: &mut Rows, id: &str, value: &str, tip: &str, enabled: bool| {
            let r = rows.row(t::ROW_HEIGHT, 4.0);
            let (response, at) = w::dropdown(ui, r, id, None, value, Some(tip), enabled, 0.0);
            response.clicked().then_some(at)
        };
    group_label(ui, rows, lang.pick("端", "Edges"));
    if let Some(at) = dropdown(
        ui,
        rows,
        "normal.edges",
        edges_name(lang, settings.edges()),
        lang.pick(
            "クランプ: キャンバスの端の傾きは端のテクセルで求めます。ラップ: 反対側の端を読みます（タイルするテクスチャ）",
            "Clamp: the slope at the canvas edge uses the edge texel. Wrap: it reads the opposite edge (tiling textures)",
        ),
        enabled && derive,
    ) {
        open_popup(app, ctx, Popup::NormalEdges, at, at.width());
    }
    group_label(ui, rows, lang.pick("ファイルの Y", "File Y"));
    if let Some(at) = dropdown(
        ui,
        rows,
        "normal.direction",
        direction_name(settings.file_direction()),
        lang.pick(
            "画像の書き出し・PNG・PSD で書くノーマルの画像の緑の向き。Unity は OpenGL（Y+）で、.ylp のテクスチャとプレビューは常に OpenGL です",
            "Green direction of Normal images written by the exports. Unity uses OpenGL (Y+); the .ylp texture and the preview always do",
        ),
        enabled,
    ) {
        open_popup(app, ctx, Popup::NormalDirection, at, at.width());
    }
}

fn channel_row(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    list: Rect,
    row: Rect,
    channel: Channel,
    kind: KindColumn,
) {
    let lang = app.lang;
    let name = m2::channel_name(lang, &app.doc, channel);
    let Some(info) = app.doc.channel_info(channel).cloned() else {
        return;
    };
    let painting = app.m2.paint_channel == channel;
    let showing = app.m2.display_channel == channel;
    let user = !channel.is_standard();
    let enabled = app.can_edit();
    let hit = row.intersect(list);
    let response = ui.interact(
        hit,
        ui.make_persistent_id(("channel.row", channel.index())),
        Sense::click(),
    );
    let painter = ui.painter_at(list);
    if painting {
        w::fill(&painter, row, t::ACCENT_SOFT);
        w::fill(
            &painter,
            Rect::from_min_size(row.min, vec2(3.0, row.height())),
            t::ACCENT,
        );
    } else if response.hovered() {
        w::fill(&painter, row, t::CONTROL_HOVER);
    }
    w::hline(
        &painter,
        row.left(),
        row.right(),
        row.bottom() - 1.0,
        t::BORDER,
    );

    let paint_btn = Rect::from_min_size(
        pos2(row.left() + 4.0, row.top() + 2.0),
        vec2(24.0, row.height() - 4.0),
    );
    let eye = Rect::from_min_size(
        pos2(paint_btn.right() + 2.0, row.top() + 2.0),
        vec2(24.0, row.height() - 4.0),
    );
    let icon_rect =
        Rect::from_min_size(pos2(eye.right() + 4.0, row.top()), vec2(18.0, row.height()));
    let format = m2::channel_format(&info);
    let kind_width = kind.width(ui.painter(), channel, &format);
    let kind_rect = Rect::from_min_max(
        pos2(row.right() - kind_width - 4.0, row.top() + 4.0),
        pos2(row.right() - 4.0, row.bottom() - 4.0),
    );
    let name_rect = Rect::from_min_max(
        pos2(icon_rect.right() + 4.0, row.top() + 3.0),
        pos2(kind_rect.left() - 4.0, row.bottom() - 3.0),
    );
    // このチャンネルを読む lilToon のスロット（見た目が lilToon のとき）: チャンネルの印の右上に小さな点、スロットの名前はツールチップ
    let reading = crate::look::slots_reading(&app.doc, channel);

    if response.clicked() {
        app.apply(Action::M2Ui(UiOp::PaintChannel(channel)));
        if app.m2.renaming_channel != Some(channel) {
            app.m2.renaming_channel = None;
        }
    }
    // 直前に別の所を押していると、egui は 2 度目を 3 回押しとして数えることがある（位置は直前の 1 回としか比べない）ので、どちらでも
    if (response.double_clicked() || response.triple_clicked())
        && user
        && ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| name_rect.contains(p))
    {
        app.apply(Action::M2Ui(UiOp::RenameChannel(channel)));
    }
    if response.secondary_clicked() {
        if let Some(at) = response.interact_pointer_pos() {
            open(
                app,
                ctx,
                PopupKind::M2(Popup::ChannelContext(channel)),
                context_anchor(at),
            );
        }
    }

    if w::icon_button(
        ui,
        paint_btn,
        ("channel.paint", channel.index()),
        "paint_brush",
        &lang.pick(
            format!("{name} を描くチャンネルにする"),
            format!("Paint {name}"),
        ),
        painting,
        enabled,
        15.0,
    )
    .clicked()
    {
        app.apply(Action::M2Ui(UiOp::PaintChannel(channel)));
    }
    if w::icon_button(
        ui,
        eye,
        ("channel.eye", channel.index()),
        if showing {
            "visibility"
        } else {
            "visibility_off"
        },
        &lang.pick(
            format!("{name} をキャンバスに出す"),
            format!("Show {name} in the canvas"),
        ),
        showing,
        true,
        16.0,
    )
    .clicked()
    {
        app.apply(Action::M2Ui(UiOp::DisplayChannel(channel)));
    }
    let painter = ui.painter_at(list);
    w::icon(
        &painter,
        icon_rect,
        m2::channel_icon(channel),
        if painting {
            Color32::WHITE
        } else {
            t::TEXT_DIM
        },
        15.0,
    );

    // 名前（ユーザーチャンネルはダブルクリックで変える）
    if app.m2.renaming_channel == Some(channel) {
        let first = !app.m2.rename_channel_started;
        app.m2.rename_channel_started = true;
        let out = w::text_field(
            ui,
            name_rect,
            ("channel.rename", channel.index()),
            &info.name,
            None,
            first,
        );
        if let Some(next) = out.committed {
            let next = next.trim().to_owned();
            if !next.is_empty() && next != info.name {
                app.apply(Action::M2(Edit::SetChannel {
                    channel,
                    info: ChannelInfo {
                        name: next,
                        ..info.clone()
                    },
                }));
            }
        }
        if !first && !out.focused {
            app.m2.renaming_channel = None;
        }
    } else {
        let shown = w::fit(&painter, &name, name_rect.width(), t::LABEL);
        w::text(
            &painter,
            name_rect,
            &shown,
            t::LABEL.with_color(if painting { Color32::WHITE } else { t::TEXT }),
            Align::Left,
        );
    }

    if !reading.is_empty() {
        let painter = ui.painter_at(list);
        let dot = pos2(icon_rect.right() - 1.0, row.top() + 7.0);
        painter.circle_filled(dot, 3.0, t::ACCENT);
        let names: Vec<&str> = reading.iter().map(|s| s.label(lang)).collect();
        let tip = format!("lilToon: {}", names.join(lang.pick("、", ", ")));
        let hit = icon_rect.intersect(list);
        let response = ui.interact(
            hit,
            ui.make_persistent_id(("channel.look", channel.index())),
            Sense::hover(),
        );
        let label = tip.clone();
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &label));
        response.on_hover_text(tip);
    }

    // 種類（形式の名前。ユーザーチャンネルは押すと替えられる）
    if user {
        let (response, b) = w::dropdown(
            ui,
            kind_rect,
            ("channel.kind", channel.index()),
            None,
            &format,
            Some(lang.pick("チャンネルの種類", "Channel type")),
            enabled,
            0.0,
        );
        if response.clicked() {
            open(app, ctx, PopupKind::M2(Popup::ChannelKind(channel)), b);
        }
    } else {
        w::text(&painter, kind_rect, &format, t::LABEL_DIM, Align::Right);
    }
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::SelectableLabel, enabled, painting, &name)
    });
}
