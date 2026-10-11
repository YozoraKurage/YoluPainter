//! 「メッシュマップをベイク」のウィンドウ（Unity 版の `MeshBakeWindow` と同じ並び: 上に焼くテクスチャセット、左に共通の設定と焼くマップの
//! 一覧、右に選んだ項目の設定、下に進み具合・取消・ベイク・閉じる）。値は `AppState::bake.settings` そのもの（保存したマップの
//! 由来・古さの判定と同じ値）。ウィンドウを閉じてもベイクは続く（仕事の札に進み具合と取消が出る）。
//!
//! 文言は名前・状態・短い理由だけ。マップの意味や設定の説明はツールチップ。高ポリの指定はまだ無い。

use egui::{pos2, vec2, Id, Key, Rect, Ui, UiBuilder, Vec2};
use yolu_core::mesh_maps::{
    MeshBakeNote, MeshBakeReport, MeshMapKind, MeshMapState, MeshOcclusionFalloff, MeshOverlapList,
    MeshOverlapRule,
};

use super::{
    backend_help, backend_label, id_source_label, kind_label, kind_short, needs_reference,
    phase_label, probe_line, run_line, slot_list, stale_reasons, state_label, BakeAction,
    BakeBackend, MeshMapView, PlaceLine, ID_SOURCES,
};
use crate::lang::Lang;
use crate::state::AppState;
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align, NumberFormat, SliderSpec};
use crate::ui::window::{self, Frame, Spec};

/// ウィンドウの大きさ（Unity 版は 780 × 600）。
pub const SIZE: Vec2 = vec2(760.0, 560.0);
const LIST_WIDTH: f32 = 252.0;
const FOOTER_HEIGHT: f32 = 52.0;
const ROW: f32 = 26.0;
const SET_ROW: f32 = 24.0;
/// 焼くテクスチャセットの欄の、見せる行の数の上限（それ以上は欄の中でスクロール）。
const SET_ROWS_VISIBLE: usize = 5;
/// 「重なった UV」の項目の、見取り図の左の一覧の幅と、見取り図の最小の辺。
const MAP_LIST_WIDTH: f32 = 180.0;
const MAP_MIN: f32 = 160.0;

/// 右に出している項目。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Common,
    Map(MeshMapKind),
    /// 重なった UV のテクセルの持ち主の決め方（今のセット）。
    Overlap,
}

/// ウィンドウの状態（右の項目・スクロール・動かした量）。
#[derive(Default)]
pub struct BakeWindow {
    pub page: Page,
    sets_scroll: f32,
    list_scroll: f32,
    page_scroll: f32,
    /// 右の項目の中身の高さ（前のフレームに描いたもの。つまみの長さとずらせる範囲の元）。
    page_content: f32,
    offset: Vec2,
    /// 「重なった UV」の UV の見取り図（拡大・中心・選び替え）。
    pub map: super::uvmap::MapUi,
}

/// ウィンドウの 1 フレームぶんの表示データ（描く前に集める。描く途中で状態を借りないため）。
struct SetRow {
    uid: u32,
    name: String,
    size: (u32, u32),
    slots: Option<Vec<i32>>,
    checked: bool,
}

struct MapRow {
    kind: MeshMapKind,
    /// 焼いてあり、照合できたときの状態。
    state: Option<MeshMapState>,
    /// 焼いてあるが、モデルの入力を別のスレッドで作っている最中で、まだ照合できない。
    checking: bool,
    /// 焼いたマップの大きさとスロット。
    baked: Option<((i32, i32), Vec<i32>)>,
    reasons: String,
}

/// 焼く場所の欄に出すもの（選び・GPU の確認・最後のベイクを行った場所）。
struct Place {
    backend: BakeBackend,
    probe: Option<PlaceLine>,
    last: Option<PlaceLine>,
}

/// 重なった UV の欄に出すもの（今のセットの文書のベイクの優先）。
struct Overlap {
    set: String,
    rule: MeshOverlapRule,
    skip_outside: bool,
    rows: [(MeshOverlapList, Vec<super::overlap::IslandRow>); 2],
    picking: Option<MeshOverlapList>,
    /// 手で選んだアイランドが別のモデルのもの。
    foreign: bool,
    /// 変えられない理由（読むだけのセット・描いている間）。
    locked: Option<String>,
}

/// 焼く場所の選びの並び。
const BACKENDS: [BakeBackend; 3] = [BakeBackend::Auto, BakeBackend::Gpu, BakeBackend::Cpu];

/// ウィンドウを描く（開いていなければ何もしない）。
pub fn show(ctx: &egui::Context, app: &mut AppState) {
    let Some(mut win) = app.bake.window.take() else {
        return;
    };
    let lang = app.lang;
    app.bake.ensure_gpu_probe(app.prefs.settings.bake_ray_query);
    // 表示データ（モデルの入力は別のスレッドで作る。できるまで状態は「確認中」）
    let input = app.bake_input_nowait();
    let checking = input.is_none();
    let input_ref = input
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .map(|a| a.as_ref());
    let sets: Vec<SetRow> = (0..app.sets.len())
        .map(|i| {
            let set = app.sets.get(i).expect("範囲内");
            let doc = app.set_doc(i);
            let slots = app.set_slots(i);
            SetRow {
                uid: set.uid,
                name: set.name.clone(),
                size: (doc.width(), doc.height()),
                checked: slots.is_some() && !app.bake.skipped.contains(&set.uid),
                slots,
            }
        })
        .collect();
    let current = app.sets.current_index();
    let maps: Vec<MapRow> = MeshMapKind::ALL
        .iter()
        .map(|&kind| {
            let baked = app
                .sets
                .get(current)
                .and_then(|s| s.mesh_maps.get(kind))
                .map(|m| {
                    let p = m.provenance();
                    ((p.width, p.height), p.target_slots.clone())
                });
            let check = if checking {
                None
            } else {
                app.mesh_map_check_with(current, kind, input_ref)
            };
            MapRow {
                kind,
                state: check.as_ref().map(|c| c.state),
                checking: checking && baked.is_some(),
                baked,
                reasons: check
                    .filter(|c| c.state == MeshMapState::Stale)
                    .map(|c| stale_reasons(&c))
                    .unwrap_or_default(),
            }
        })
        .collect();
    let refusal = app.bake_refusal_nowait();
    let baking = app.bake.is_baking();
    let progress = app.bake.progress();
    let report = app
        .sets
        .get(current)
        .and_then(|s| s.mesh_maps.report())
        .cloned();
    let showing = app.bake.view;
    let place = Place {
        backend: app.bake.backend,
        probe: probe_line(lang, app.bake.backend, &app.bake.gpu_probe()),
        last: app
            .sets
            .get(current)
            .and_then(|s| s.mesh_maps.run())
            .map(|r| run_line(lang, r)),
    };

    let priority = app.doc.bake_priority().clone();
    let overlap = Overlap {
        set: app.sets.current().name.clone(),
        rule: priority.rule,
        skip_outside: priority.skip_outside,
        rows: [
            (
                MeshOverlapList::Prefer,
                app.overlap_island_rows(MeshOverlapList::Prefer),
            ),
            (
                MeshOverlapList::Skip,
                app.overlap_island_rows(MeshOverlapList::Skip),
            ),
        ],
        picking: super::overlap::picking(app),
        foreign: app.overlap_islands_foreign(),
        locked: if app.is_stroking() {
            Some(crate::lang::refusals::during_stroke(lang).to_owned())
        } else {
            app.read_only_reason()
                .map(|r| crate::lang::refusals::read_only_set(lang, r))
        },
    };

    // 「重なった UV」の UV の見取り図（項目を出しているときだけ作る）
    let map_frame = (win.page == Page::Overlap).then(|| super::uvmap::MapFrame {
        data: app.overlap_map(),
        priority: priority.clone(),
        overlap: crate::uv_wireframe::overlap::texture_for_map(ctx, app),
        menu: match app.popup.as_ref().map(|p| p.kind) {
            Some(crate::state::PopupKind::BakeIsland {
                set,
                island,
                map: true,
                ..
            }) if set == app.sets.current().uid => Some(island),
            _ => None,
        },
    });
    let mut map_out = super::uvmap::MapOutcome::default();

    let mut actions: Vec<BakeAction> = Vec::new();
    let mut close = false;
    let title = lang.pick("メッシュマップをベイク", "Bake Mesh Maps");
    let spec = Spec {
        title,
        icon: Some("texture"),
        size: SIZE,
        modal: false,
        close_label: lang.pick("ウィンドウを閉じる", "Close Window"),
    };
    let id = Id::new("yolu.bake-window");
    let popup_open = app.ui.popup_was_open;
    let mut offset = win.offset;
    let closed = window::show(ctx, id, &spec, &mut offset, false, |ui, frame| {
        // Esc: 焼いている間は取消、そうでなければ閉じる（ウィンドウの上にポインタがあるとき。ポップアップを開いていたら、閉じるのはそちらだけ）
        let esc = !popup_open
            && ui.input(|i| i.key_pressed(Key::Escape))
            && ui
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|p| frame.rect.contains(p));
        // アイランドを手で選んでいる間の Esc は、ポインタの場所によらず選ぶのをやめるだけ
        if overlap.picking.is_some() && ui.input(|i| i.key_pressed(Key::Escape)) {
            actions.push(BakeAction::Priority(super::overlap::PriorityOp::Pick(None)));
        } else if esc {
            if baking {
                actions.push(BakeAction::Cancel);
            } else {
                close = true;
            }
        }
        draw(
            ui,
            frame,
            &mut win,
            lang,
            &sets,
            &maps,
            showing,
            refusal.as_deref(),
            progress.as_ref(),
            app.bake.outcome.clone(),
            report.as_ref(),
            &place,
            &overlap,
            map_frame.as_ref(),
            &mut map_out,
            &mut actions,
            &mut app.bake.settings,
        );
    });
    win.offset = offset;
    let showing_map = win.page == Page::Overlap;
    if !(closed || close) {
        app.bake.window = Some(win);
    }
    // 見取り図・一覧の行で指しているアイランドは、キャンバスと 3D ビューでも強調する。押したアイランドはメニューを開く
    app.bake.map_hover = map_out.hover.filter(|_| showing_map && !(closed || close));
    if let Some((island, at)) = map_out.menu.filter(|_| showing_map) {
        super::overlap::open_menu(app, ctx, island, at, true, false);
    }
    for a in actions {
        app.apply(crate::state::Action::Bake(a));
    }
    if closed || close {
        app.apply(crate::state::Action::Bake(BakeAction::CloseWindow));
    }
}

#[allow(clippy::too_many_arguments)]
fn draw(
    ui: &mut Ui,
    frame: &Frame,
    win: &mut BakeWindow,
    lang: Lang,
    sets: &[SetRow],
    maps: &[MapRow],
    showing: MeshMapView,
    refusal: Option<&str>,
    progress: Option<&super::Progress>,
    outcome: Option<(String, bool)>,
    report: Option<&MeshBakeReport>,
    place: &Place,
    overlap: &Overlap,
    map: Option<&super::uvmap::MapFrame>,
    map_out: &mut super::uvmap::MapOutcome,
    actions: &mut Vec<BakeAction>,
    settings: &mut yolu_core::mesh_maps::MeshBakeSettings,
) {
    let body = frame.body;
    let p = ui.painter().clone();
    // 焼くテクスチャセット
    let mut y = body.top() + 6.0;
    w::text(
        &p,
        Rect::from_min_size(pos2(body.left() + 12.0, y), vec2(body.width() - 24.0, 16.0)),
        lang.pick("焼くテクスチャセット", "Texture Sets to Bake"),
        t::HEADER.with_color(t::TEXT_DIM),
        Align::Left,
    );
    y += 20.0;
    let visible = sets.len().min(SET_ROWS_VISIBLE);
    let area = Rect::from_min_size(
        pos2(body.left(), y),
        vec2(body.width(), visible as f32 * SET_ROW),
    );
    let content = sets.len() as f32 * SET_ROW;
    let sets_bar = Scroll::begin(ui, area, content, &mut win.sets_scroll);
    {
        let mut child = ui.new_child(UiBuilder::new().max_rect(area));
        child.set_clip_rect(area.intersect(ui.clip_rect()));
        let cp = child.painter().clone();
        let checked_count = sets.iter().filter(|s| s.checked).count();
        for (i, s) in sets.iter().enumerate() {
            let row = Rect::from_min_size(
                pos2(
                    area.left() + 12.0,
                    area.top() + i as f32 * SET_ROW - win.sets_scroll,
                ),
                vec2(area.width() - 24.0 - sets_bar.reserved(), SET_ROW - 2.0),
            );
            if row.bottom() < area.top() || row.top() > area.bottom() {
                continue;
            }
            let in_model = s.slots.is_some();
            let last = s.checked && checked_count == 1;
            let tip = if !in_model {
                Some(lang.pick(
                    "このマテリアルは読み込んだモデルにありません",
                    "This material is not in the loaded model",
                ))
            } else if last {
                Some(lang.pick("1 つは残します", "At least one stays checked"))
            } else {
                None
            };
            let name_width = (row.width() * 0.5).min(300.0);
            let next = w::toggle(
                &mut child,
                Rect::from_min_size(row.min, vec2(name_width, row.height())),
                ("bake.set", s.uid),
                &s.name,
                s.checked,
                tip,
                in_model && !last || (in_model && !s.checked),
            );
            if next != s.checked {
                actions.push(BakeAction::Set(s.uid, next));
            }
            let info = format!(
                "{}×{}   {}",
                s.size.0,
                s.size.1,
                match &s.slots {
                    Some(slots) =>
                        format!("{} {}", lang.pick("スロット", "Slot"), slot_list(slots)),
                    None => lang.pick("モデルに無い", "Not in model").to_owned(),
                }
            );
            w::text(
                &cp,
                Rect::from_min_max(pos2(row.left() + name_width + 8.0, row.top()), row.max),
                &info,
                t::LABEL_DIM.with_color(if in_model { t::TEXT_DIM } else { t::WARNING }),
                Align::Left,
            );
        }
    }
    sets_bar.end(ui, "bake.sets.scroll", &mut win.sets_scroll);
    let top = area.bottom() + 6.0;
    // 左の一覧・右の設定・下の帯
    let footer = Rect::from_min_max(pos2(body.left(), body.bottom() - FOOTER_HEIGHT), body.max);
    let list = Rect::from_min_max(
        pos2(body.left(), top),
        pos2(body.left() + LIST_WIDTH, footer.top()),
    );
    let page = Rect::from_min_max(
        pos2(list.right() + 1.0, top),
        pos2(body.right(), footer.top()),
    );
    w::hline(&p, body.left(), body.right(), top - 1.0, t::BORDER);
    w::fill(&p, list, t::MENU_BG);
    w::vline(&p, list.right(), top, footer.top(), t::BORDER);
    draw_list(ui, list, win, lang, maps, showing, actions, settings);
    draw_page(
        ui, page, win, lang, maps, report, place, overlap, map, map_out, actions, settings,
    );
    draw_footer(ui, footer, lang, refusal, progress, outcome, actions);
}

#[allow(clippy::too_many_arguments)]
fn draw_list(
    ui: &mut Ui,
    list: Rect,
    win: &mut BakeWindow,
    lang: Lang,
    maps: &[MapRow],
    showing: MeshMapView,
    actions: &mut Vec<BakeAction>,
    settings: &yolu_core::mesh_maps::MeshBakeSettings,
) {
    let rows = maps.len() + 3; // 共通の設定・UV の範囲・重なった UV・マップ
    let content = rows as f32 * ROW + 8.0;
    let bar = Scroll::begin(ui, list, content, &mut win.list_scroll);
    let mut child = ui.new_child(UiBuilder::new().max_rect(list));
    child.set_clip_rect(list.intersect(ui.clip_rect()));
    let p = child.painter().clone();
    let mut cursor = RowCursor {
        y: list.top() + 4.0 - win.list_scroll,
        x: list.left() + 4.0,
        width: list.width() - 8.0 - bar.reserved(),
    };
    // 共通の設定
    let common = cursor.next();
    let on = win.page == Page::Common;
    page_row(&mut child, common, on);
    w::text(
        &p,
        Rect::from_min_max(pos2(common.left() + 28.0, common.top()), common.max),
        lang.pick("共通の設定", "Common Settings"),
        t::LABEL.with_color(t::TEXT),
        Align::Left,
    );
    w::icon(
        &p,
        Rect::from_min_size(common.min + vec2(6.0, 0.0), vec2(18.0, common.height())),
        "tune",
        t::TEXT_DIM,
        15.0,
    );
    if click_page(&mut child, common, "bake.page.common") {
        win.page = Page::Common;
    }
    // UV の範囲（テクセルの由来）
    let cov = cursor.next();
    {
        let any = maps.iter().any(|m| m.baked.is_some());
        let label = lang.pick("UV の範囲", "UV Coverage");
        w::icon(
            &p,
            Rect::from_min_size(cov.min + vec2(6.0, 0.0), vec2(18.0, cov.height())),
            "grid_dots",
            t::TEXT_DIM,
            15.0,
        );
        w::text(
            &p,
            Rect::from_min_max(pos2(cov.left() + 28.0, cov.top()), cov.max),
            label,
            t::LABEL.with_color(if any { t::TEXT } else { t::TEXT_DISABLED }),
            Align::Left,
        );
        let eye = Rect::from_min_size(
            pos2(cov.right() - 26.0, cov.top() + 1.0),
            vec2(24.0, cov.height() - 2.0),
        );
        let on = showing == MeshMapView::Coverage;
        if w::icon_button(
            &mut child,
            eye,
            "bake.view.coverage",
            if on { "visibility" } else { "visibility_off" },
            &lang.pick(
                format!("{label}をキャンバスに出す"),
                format!("Show {label} in the canvas"),
            ),
            on,
            any,
            16.0,
        )
        .clicked()
        {
            actions.push(BakeAction::View(if on {
                MeshMapView::None
            } else {
                MeshMapView::Coverage
            }));
        }
    }
    // 重なった UV（今のセットの持ち主の決め方）
    let over = cursor.next();
    page_row(&mut child, over, win.page == Page::Overlap);
    w::icon(
        &p,
        Rect::from_min_size(over.min + vec2(6.0, 0.0), vec2(18.0, over.height())),
        "shape_intersect",
        t::TEXT_DIM,
        15.0,
    );
    w::text(
        &p,
        Rect::from_min_max(pos2(over.left() + 28.0, over.top()), over.max),
        lang.pick("重なった UV", "Overlapping UVs"),
        t::LABEL.with_color(t::TEXT),
        Align::Left,
    );
    if click_page(&mut child, over, "bake.page.overlap") {
        win.page = Page::Overlap;
    }
    w::hline(
        &p,
        list.left() + 8.0,
        list.right() - 8.0,
        (cursor.y + 1.0).round(),
        t::SEPARATOR,
    );
    cursor.y += 4.0;
    // 焼くマップ
    for m in maps {
        let row = cursor.next();
        let on = win.page == Page::Map(m.kind);
        if row.bottom() < list.top() || row.top() > list.bottom() {
            continue;
        }
        let id = ("bake.page", m.kind as i32);
        page_row(&mut child, row, on);
        let full = kind_label(lang, m.kind);
        let name = kind_short(lang, m.kind);
        let state = row_state_label(lang, m);
        let state_color = match m.state {
            None if m.checking => t::TEXT_DIM,
            None => t::TEXT_DISABLED,
            Some(MeshMapState::Current) => t::ACCENT,
            Some(MeshMapState::Stale) => t::WARNING,
            Some(MeshMapState::Unverified) => t::TEXT_DIM,
        };
        let eye = Rect::from_min_size(
            pos2(row.right() - 26.0, row.top() + 1.0),
            vec2(24.0, row.height() - 2.0),
        );
        let state_w = 46.0;
        let state_rect = Rect::from_min_size(
            pos2(eye.left() - state_w - 2.0, row.top()),
            vec2(state_w, row.height()),
        );
        w::text(
            &p,
            state_rect,
            state,
            t::LABEL_SMALL.with_color(state_color),
            Align::Right,
        );
        let box_rect =
            Rect::from_min_size(pos2(row.left() + 6.0, row.top()), vec2(18.0, row.height()));
        let name_rect = Rect::from_min_max(
            pos2(box_rect.right() + 6.0, row.top()),
            pos2(state_rect.left() - 4.0, row.bottom()),
        );
        if click_page(
            &mut child,
            Rect::from_min_max(
                pos2(box_rect.right(), row.top()),
                pos2(state_rect.right(), row.bottom()),
            ),
            id,
        ) {
            win.page = Page::Map(m.kind);
        }
        let checked = settings.maps.contains(&m.kind);
        let last = checked && settings.maps.len() == 1;
        let help = if last {
            lang.pick("1 つは残します", "At least one stays checked")
                .to_owned()
        } else {
            format!("{full}\n{}", kind_help(lang, m.kind))
        };
        let next = check_box(
            &mut child,
            box_rect,
            ("bake.map", m.kind as i32),
            name,
            checked,
            &help,
            !last,
        );
        w::text(
            &p,
            name_rect,
            name,
            t::LABEL.with_color(t::TEXT),
            Align::Left,
        );
        child
            .interact(
                name_rect,
                Id::new(("bake.map.tip", m.kind as i32)),
                egui::Sense::hover(),
            )
            .on_hover_text(&help);
        if next != checked {
            actions.push(BakeAction::Map(m.kind, next));
        }
        let showing_this = showing == MeshMapView::Kind(m.kind);
        if w::icon_button(
            &mut child,
            eye,
            ("bake.view", m.kind as i32),
            if showing_this {
                "visibility"
            } else {
                "visibility_off"
            },
            &lang.pick(
                format!("{full}をキャンバスに出す"),
                format!("Show {full} in the canvas"),
            ),
            showing_this,
            m.baked.is_some(),
            16.0,
        )
        .clicked()
        {
            actions.push(BakeAction::View(if showing_this {
                MeshMapView::None
            } else {
                MeshMapView::Kind(m.kind)
            }));
        }
    }
    bar.end(ui, "bake.list.scroll", &mut win.list_scroll);
}

/// 一覧の右端の状態の名前（照合の最中は「確認中」）。
fn row_state_label(lang: Lang, row: &MapRow) -> &'static str {
    if row.checking {
        lang.pick("確認中", "Checking")
    } else {
        state_label(lang, row.state)
    }
}

struct RowCursor {
    y: f32,
    x: f32,
    width: f32,
}

impl RowCursor {
    fn next(&mut self) -> Rect {
        let r = Rect::from_min_size(pos2(self.x, self.y), vec2(self.width, ROW));
        self.y += ROW;
        r
    }
}

/// 四角のチェック（名前は呼ぶ側が描く）。`label` は読み上げ・試験の名前。押すと反転した値を返す。
fn check_box(
    ui: &mut Ui,
    r: Rect,
    id_salt: impl std::hash::Hash,
    label: &str,
    value: bool,
    tooltip: &str,
    enabled: bool,
) -> bool {
    let id = Id::new(("bake.check", id_hash(&id_salt)));
    let response = ui.interact(
        r,
        id,
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let next = if response.clicked() { !value } else { value };
    let b = Rect::from_min_size(pos2(r.left(), r.center().y - 8.0), vec2(16.0, 16.0));
    let p = ui.painter();
    w::rounded(
        p,
        b,
        if next {
            if enabled {
                t::ACCENT
            } else {
                t::CONTROL_HOVER
            }
        } else {
            t::CONTROL_BG
        },
        3.0,
    );
    if next {
        w::icon(p, b, "check", egui::Color32::WHITE, 14.0);
    } else {
        w::outline(
            p,
            b,
            if enabled && response.hovered() {
                t::ACCENT_DIM
            } else {
                t::SEPARATOR
            },
            1.0,
            3.0,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, next, label)
    });
    let _ = response.on_hover_text(tooltip);
    next
}

fn page_row(ui: &mut Ui, row: Rect, on: bool) {
    if on {
        w::rounded(ui.painter(), row, t::ACCENT_SOFT, 4.0);
        w::fill(
            ui.painter(),
            Rect::from_min_size(row.min + vec2(0.0, 3.0), vec2(3.0, row.height() - 6.0)),
            t::ACCENT,
        );
    }
}

/// 行を押したら右の項目を替える（部品の下に敷く押下）。
fn click_page(ui: &mut Ui, r: Rect, id: impl std::hash::Hash) -> bool {
    let response = ui.interact(
        r,
        Id::new(("bake.pick", id_hash(&id))),
        egui::Sense::click(),
    );
    if response.hovered() {
        w::rounded(ui.painter(), r, t::CONTROL_HOVER, 4.0);
    }
    response.clicked()
}

fn id_hash(id: &impl std::hash::Hash) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut h);
    h.finish()
}

/// 右の項目。
#[allow(clippy::too_many_arguments)]
fn draw_page(
    ui: &mut Ui,
    page: Rect,
    win: &mut BakeWindow,
    lang: Lang,
    maps: &[MapRow],
    report: Option<&MeshBakeReport>,
    place: &Place,
    overlap: &Overlap,
    map: Option<&super::uvmap::MapFrame>,
    map_out: &mut super::uvmap::MapOutcome,
    actions: &mut Vec<BakeAction>,
    settings: &mut yolu_core::mesh_maps::MeshBakeSettings,
) {
    // 中身の高さは前のフレームのもの（描き終えたあとに入れる）。見取り図の上のホイールは見取り図の拡大（欄を送らない）
    let bar = if win.page == Page::Overlap && ui.rect_contains_pointer(win.map.rect) {
        Scroll::new(page, win.page_content, &mut win.page_scroll)
    } else {
        Scroll::begin(ui, page, win.page_content, &mut win.page_scroll)
    };
    let mut child = ui.new_child(UiBuilder::new().max_rect(page));
    child.set_clip_rect(page.intersect(ui.clip_rect()));
    let p = child.painter().clone();
    w::fill(&p, page, t::PANEL_BG);
    let mut y = page.top() + 8.0 - win.page_scroll;
    let width = page.width() - 24.0 - bar.reserved();
    let x = page.left() + 12.0;
    let row = |h: f32, gap: f32, y: &mut f32| {
        let r = Rect::from_min_size(pos2(x, *y), vec2(width, h));
        *y += h + gap;
        r
    };
    let slider = |ui: &mut Ui,
                  y: &mut f32,
                  id: &str,
                  label: &str,
                  value: f64,
                  min: f32,
                  max: f32,
                  format: NumberFormat,
                  tip: &str|
     -> Option<f64> {
        let r = row(t::SLIDER_ROW_HEIGHT, 4.0, y);
        let out = w::slider(
            ui,
            r,
            id,
            value as f32,
            &SliderSpec::new(label, min, max, format).tooltip(tip),
        );
        out.changed.then_some(out.value as f64)
    };
    match win.page {
        Page::Common => {
            let r = row(16.0, 4.0, &mut y);
            w::text(
                &p,
                r,
                lang.pick("共通の設定", "Common Settings"),
                t::HEADER.with_color(t::TEXT_DIM),
                Align::Left,
            );
            if let Some(v) = slider(
                &mut child,
                &mut y,
                "bake.padding",
                lang.pick("余白", "Padding"),
                settings.padding as f64,
                0.0,
                64.0,
                NumberFormat::int(" px"),
                lang.pick(
                    "UV の外へ塗り広げるテクセルの数",
                    "Texels dilated beyond the UV islands",
                ),
            ) {
                settings.padding = v.round() as i32;
            }
            if let Some(v) = slider(
                &mut child,
                &mut y,
                "bake.aa",
                lang.pick("アンチエイリアス", "Antialiasing"),
                settings.antialiasing as f64,
                1.0,
                4.0,
                NumberFormat::int("×"),
                lang.pick(
                    "テクセルごとの縦横のサンプル数（2 なら 2 × 2）",
                    "Samples per texel along each axis (2 means 2 × 2)",
                ),
            ) {
                settings.antialiasing = v.round().clamp(1.0, 4.0) as i32;
            }
            y += 6.0;
            let r = row(16.0, 4.0, &mut y);
            w::text(
                &p,
                r,
                lang.pick("焼く場所", "Bake On"),
                t::HEADER.with_color(t::TEXT_DIM),
                Align::Left,
            );
            let r = row(24.0, 4.0, &mut y);
            let parts = w::Rows::split(r, BACKENDS.len() + 1, 4.0);
            w::text(
                &p,
                parts[0],
                lang.pick("場所", "Run on"),
                t::LABEL,
                Align::Left,
            );
            for (i, backend) in BACKENDS.iter().enumerate() {
                if w::button(
                    &mut child,
                    parts[i + 1],
                    ("bake.backend", i),
                    backend_label(lang, *backend),
                    place.backend == *backend,
                    true,
                    Some(backend_help(lang, *backend)),
                    None,
                )
                .clicked()
                {
                    actions.push(BakeAction::Backend(*backend));
                }
            }
            if let Some(line) = &place.probe {
                let r = row(18.0, 2.0, &mut y);
                place_line(&mut child, &p, r, "bake.backend.probe", line);
            }
            // 最後のベイク: 焼いた場所と、記録の注意の行だけ（時間・レイ・テクセル・三角形の数は記録 `MeshBakeReport` に残し、画面には
            // 出さない）。焼いていなければ見出しごと出さない（空の欄に文も見出しも置かない）
            let notes = report
                .map(|rep| report_lines(lang, rep))
                .unwrap_or_default();
            if place.last.is_some() || !notes.is_empty() {
                y += 6.0;
                let r = row(16.0, 4.0, &mut y);
                w::text(
                    &p,
                    r,
                    lang.pick("最後のベイク", "Last Bake"),
                    t::HEADER.with_color(t::TEXT_DIM),
                    Align::Left,
                );
            }
            if let Some(line) = &place.last {
                let r = row(18.0, 2.0, &mut y);
                place_line(&mut child, &p, r, "bake.backend.last", line);
            }
            for (text, warn) in notes {
                let r = row(18.0, 2.0, &mut y);
                let shown = w::fit(&p, &text, r.width(), t::LABEL_DIM);
                w::text(
                    &p,
                    r,
                    &shown,
                    t::LABEL_DIM.with_color(if warn { t::WARNING } else { t::TEXT_DIM }),
                    Align::Left,
                );
                let tip = ui.interact(
                    r,
                    Id::new(("bake.report", text.clone())),
                    egui::Sense::hover(),
                );
                if shown != text {
                    tip.on_hover_text(text);
                }
            }
        }
        Page::Overlap => {
            use super::overlap::{list_name, rule_help, rule_label, PriorityOp};
            let enabled = overlap.locked.is_none();
            let locked = overlap.locked.as_deref();
            let r = row(18.0, 2.0, &mut y);
            w::text(
                &p,
                r,
                lang.pick("重なった UV", "Overlapping UVs"),
                t::HEADER,
                Align::Left,
            );
            let r = row(18.0, 2.0, &mut y);
            w::text(&p, r, &overlap.set, t::LABEL_DIM, Align::Left);
            y += 4.0;
            let r = row(24.0, 4.0, &mut y);
            let parts = w::Rows::split(r, MeshOverlapRule::ALL.len() + 1, 4.0);
            w::text(
                &p,
                parts[0],
                lang.pick("優先", "Priority"),
                t::LABEL,
                Align::Left,
            );
            for (i, rule) in MeshOverlapRule::ALL.iter().enumerate() {
                if w::button(
                    &mut child,
                    parts[i + 1],
                    ("bake.overlap.rule", i),
                    rule_label(lang, *rule),
                    overlap.rule == *rule,
                    enabled,
                    Some(locked.unwrap_or(rule_help(lang, *rule))),
                    None,
                )
                .clicked()
                {
                    actions.push(BakeAction::Priority(PriorityOp::Rule(*rule)));
                }
            }
            let r = row(22.0, 4.0, &mut y);
            let next = w::toggle(
                &mut child,
                r,
                "bake.overlap.outside",
                lang.pick("0〜1 の外のアイランドを焼かない", "Skip islands outside 0–1"),
                overlap.skip_outside,
                Some(locked.unwrap_or(lang.pick(
                    "UV を 0〜1 の外へずらしたアイランドを焼かない（切っているときは、0〜1 の外の UV があるとベイクを断る）",
                    "Islands moved outside the 0–1 UV square are not baked (when off, a UV outside 0–1 stops the bake)",
                ))),
                enabled,
            );
            if next != overlap.skip_outside {
                actions.push(BakeAction::Priority(PriorityOp::SkipOutside(next)));
            }
            // 左に一覧、右に UV の見取り図（ウィンドウの大きさに合わせた正方形）
            y += 6.0;
            let top = y;
            let side = (width - MAP_LIST_WIDTH - 12.0)
                .min(page.bottom() - (top + win.page_scroll) - 8.0)
                .max(MAP_MIN);
            let list_width = (width - side - 12.0).max(0.0);
            let lrow = |h: f32, gap: f32, y: &mut f32| {
                let r = Rect::from_min_size(pos2(x, *y), vec2(list_width, h));
                *y += h + gap;
                r
            };
            let mut list_hover = None;
            for (n, (list, rows)) in overlap.rows.iter().enumerate() {
                if n > 0 {
                    y += 6.0;
                }
                let r = lrow(22.0, 2.0, &mut y);
                let name = list_name(lang, *list);
                w::text(
                    &p,
                    Rect::from_min_max(r.min, pos2(r.right() - 28.0, r.bottom())),
                    name,
                    t::HEADER.with_color(t::TEXT_DIM),
                    Align::Left,
                );
                let on = overlap.picking == Some(*list);
                let add_tip = lang.pick(
                    format!("2D か 3D で押したアイランドを{}に追加する（入っているアイランドを押すと外す・Esc でやめる）", lang.quote(name)),
                    format!("Add the island you click in 2D or 3D to {} (clicking one already listed removes it; Esc stops)", lang.quote(name)),
                );
                if w::icon_button(
                    &mut child,
                    Rect::from_min_size(pos2(r.right() - 24.0, r.top()), vec2(24.0, r.height())),
                    ("bake.overlap.add", *list as u8),
                    "add",
                    locked.unwrap_or(&add_tip),
                    on,
                    enabled,
                    16.0,
                )
                .clicked()
                {
                    actions.push(BakeAction::Priority(PriorityOp::Pick(Some(*list))));
                }
                for island in rows {
                    let r = lrow(20.0, 0.0, &mut y);
                    if child.rect_contains_pointer(r) {
                        list_hover = Some(island.representative);
                    }
                    let shown = w::fit(&p, &island.label, r.width() - 30.0, t::LABEL);
                    w::text(
                        &p,
                        Rect::from_min_max(
                            pos2(r.left() + 6.0, r.top()),
                            pos2(r.right() - 28.0, r.bottom()),
                        ),
                        &shown,
                        t::LABEL,
                        Align::Left,
                    );
                    if shown != island.label {
                        ui.interact(
                            r,
                            Id::new(("bake.overlap.row", *list as u8, island.representative)),
                            egui::Sense::hover(),
                        )
                        .on_hover_text(&island.label);
                    }
                    if w::icon_button(
                        &mut child,
                        Rect::from_min_size(
                            pos2(r.right() - 24.0, r.top()),
                            vec2(24.0, r.height()),
                        ),
                        ("bake.overlap.remove", *list as u8, island.representative),
                        "close",
                        locked.unwrap_or(lang.pick("一覧から外す", "Remove from the list")),
                        false,
                        enabled,
                        14.0,
                    )
                    .clicked()
                    {
                        actions.push(BakeAction::Priority(PriorityOp::Remove(
                            island.representative,
                        )));
                    }
                }
            }
            if overlap.foreign {
                y += 4.0;
                let r = lrow(18.0, 2.0, &mut y);
                let text = lang.with_reason(
                    lang.pick(
                        "手で選んだアイランドは使えません",
                        "The chosen islands cannot be used",
                    ),
                    lang.pick("別のモデルのもの", "they belong to another model"),
                );
                let shown = w::fit(&p, &text, r.width(), t::LABEL_DIM);
                w::text(
                    &p,
                    r,
                    &shown,
                    t::LABEL_DIM.with_color(t::WARNING),
                    Align::Left,
                );
                if shown != text {
                    ui.interact(r, Id::new("bake.overlap.foreign"), egui::Sense::hover())
                        .on_hover_text(text);
                }
            }
            if let Some(map) = map {
                let rect = Rect::from_min_size(pos2(x + width - side, top), Vec2::splat(side));
                let out = super::uvmap::draw(&mut child, rect, &mut win.map, map, list_hover);
                map_out.hover = out.hover.or(list_hover);
                map_out.menu = out.menu;
                y = y.max(rect.bottom() + 4.0);
            }
        }
        Page::Map(kind) => {
            let m = maps.iter().find(|m| m.kind == kind);
            let r = row(18.0, 2.0, &mut y);
            w::text(&p, r, kind_label(lang, kind), t::HEADER, Align::Left);
            // 状態
            let state_text = match m.and_then(|m| m.baked.as_ref().map(|b| (m.state, b))) {
                None => lang.pick("まだ焼いていません", "Not baked yet").to_owned(),
                Some((state, ((w_, h_), slots))) => format!(
                    "{}: {} ({w_}×{h_}, {} {})",
                    lang.pick("焼いたマップ", "Baked map"),
                    m.map_or_else(|| state_label(lang, state), |m| row_state_label(lang, m)),
                    lang.pick("スロット", "slot"),
                    slot_list(slots)
                ),
            };
            let r = row(18.0, 2.0, &mut y);
            w::text(&p, r, &state_text, t::LABEL_DIM, Align::Left);
            if let Some(m) = m.filter(|m| !m.reasons.is_empty()) {
                let text = lang.with_reason(
                    lang.pick("古いので使いません", "Stale and not used"),
                    &m.reasons,
                );
                let r = row(18.0, 2.0, &mut y);
                let shown = w::fit(&p, &text, r.width(), t::LABEL_DIM);
                w::text(
                    &p,
                    r,
                    &shown,
                    t::LABEL_DIM.with_color(t::WARNING),
                    Align::Left,
                );
                let tip = ui.interact(
                    r,
                    Id::new(("bake.stale", kind as i32)),
                    egui::Sense::hover(),
                );
                if shown != text {
                    tip.on_hover_text(text);
                }
            }
            y += 6.0;
            let rays_tip = lang.pick(
                "テクセルごとのレイの数（多いほどなめらかで遅い）",
                "Rays per texel (more is smoother and slower)",
            );
            let distance_tip = lang.pick(
                "最大の距離（モデルの境界箱の対角線に対する割合）",
                "Max distance, relative to the bounding-box diagonal",
            );
            let spread_tip = lang.pick("180° なら半球全体", "180° is the whole hemisphere");
            match kind {
                MeshMapKind::AmbientOcclusion | MeshMapKind::BentNormal => {
                    if let Some(v) = slider(&mut child, &mut y, "bake.ao.rays", lang.pick("レイ", "Rays"), settings.ao_samples as f64, 1.0, 512.0, NumberFormat::int(""), rays_tip) {
                        settings.ao_samples = v.round() as i32;
                    }
                    if let Some(v) = slider(&mut child, &mut y, "bake.ao.distance", lang.pick("距離", "Distance"), settings.ao_max_distance, 0.001, 1.0, NumberFormat { decimals: 3, trim: true, suffix: "" }, distance_tip) {
                        settings.ao_max_distance = v.clamp(0.001, 1.0);
                    }
                    if let Some(v) = slider(&mut child, &mut y, "bake.ao.spread", lang.pick("広がり", "Spread"), settings.ao_spread_degrees, 1.0, 180.0, NumberFormat::int("°"), spread_tip) {
                        settings.ao_spread_degrees = v.round().clamp(1.0, 180.0);
                    }
                    let r = row(22.0, 4.0, &mut y);
                    settings.ao_ignore_backfaces = w::toggle(
                        &mut child,
                        r,
                        "bake.ao.backfaces",
                        lang.pick("裏面を無視", "Ignore back faces"),
                        settings.ao_ignore_backfaces,
                        Some(lang.pick("裏から見た面をレイが通り抜ける", "Rays pass through faces seen from behind")),
                        true,
                    );
                    if kind == MeshMapKind::AmbientOcclusion {
                        let r = row(24.0, 4.0, &mut y);
                        let halves = Rect::from_min_size(r.min, vec2(r.width(), r.height()));
                        let parts = w::Rows::split(halves, 3, 4.0);
                        w::text(&p, parts[0], lang.pick("減衰", "Falloff"), t::LABEL, Align::Left);
                        for (i, (falloff, name, tip)) in [
                            (MeshOcclusionFalloff::None, lang.pick("なし", "None"), lang.pick("距離で変えない", "Distance does not matter")),
                            (MeshOcclusionFalloff::Linear, lang.pick("線形", "Linear"), lang.pick("近い遮蔽物ほど暗い", "Nearer occluders darken more")),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            if w::button(&mut child, parts[i + 1], ("bake.ao.falloff", i), name, settings.ao_falloff == falloff, true, Some(tip), None).clicked() {
                                settings.ao_falloff = falloff;
                            }
                        }
                    }
                }
                MeshMapKind::Thickness => {
                    if let Some(v) = slider(&mut child, &mut y, "bake.thickness.rays", lang.pick("レイ", "Rays"), settings.thickness_samples as f64, 1.0, 512.0, NumberFormat::int(""), rays_tip) {
                        settings.thickness_samples = v.round() as i32;
                    }
                    if let Some(v) = slider(&mut child, &mut y, "bake.thickness.distance", lang.pick("距離", "Distance"), settings.thickness_max_distance, 0.001, 1.0, NumberFormat { decimals: 3, trim: true, suffix: "" }, distance_tip) {
                        settings.thickness_max_distance = v.clamp(0.001, 1.0);
                    }
                    if let Some(v) = slider(&mut child, &mut y, "bake.thickness.spread", lang.pick("広がり", "Spread"), settings.thickness_spread_degrees, 1.0, 180.0, NumberFormat::int("°"), spread_tip) {
                        settings.thickness_spread_degrees = v.round().clamp(1.0, 180.0);
                    }
                }
                MeshMapKind::Curvature => {
                    if let Some(v) = slider(&mut child, &mut y, "bake.curvature.radius", lang.pick("半径", "Radius"), settings.curvature_radius, 0.001, 0.5, NumberFormat { decimals: 3, trim: true, suffix: "" }, lang.pick(
                        "この距離の内側の辺を数える（モデルの境界箱の対角線に対する割合）。大きいほど幅が広くやわらかい",
                        "Edges within this distance count (relative to the bounding-box diagonal). Larger is wider and softer",
                    )) {
                        settings.curvature_radius = v.clamp(0.001, 0.5);
                    }
                }
                MeshMapKind::Id => {
                    let r = row(24.0, 4.0, &mut y);
                    let parts = w::Rows::split(r, ID_SOURCES.len() + 1, 4.0);
                    w::text(&p, parts[0], lang.pick("色の分け方", "Colors from"), t::LABEL, Align::Left);
                    for (i, source) in ID_SOURCES.iter().enumerate() {
                        if w::button(&mut child, parts[i + 1], ("bake.id.source", i), id_source_label(lang, *source), settings.id_source == *source, true, Some(id_source_help(lang, *source)), None).clicked() {
                            settings.id_source = *source;
                        }
                    }
                }
                _ => {}
            }
            if needs_reference(kind) {
                let r = row(18.0, 2.0, &mut y);
                w::text(
                    &p,
                    r,
                    lang.pick(
                        "高ポリが無いので、このマップは一様です",
                        "No high poly, so this map is uniform",
                    ),
                    t::LABEL_DIM.with_color(t::WARNING),
                    Align::Left,
                );
            }
            if let Some(text) = id_status(lang, kind, report) {
                let r = row(18.0, 2.0, &mut y);
                let shown = w::fit(&p, &text, r.width(), t::LABEL_DIM);
                w::text(
                    &p,
                    r,
                    &shown,
                    t::LABEL_DIM.with_color(t::WARNING),
                    Align::Left,
                );
                if shown != text {
                    ui.interact(r, Id::new("bake.id.status"), egui::Sense::hover())
                        .on_hover_text(text);
                }
            }
        }
    }
    win.page_content = y + win.page_scroll - page.top() + 8.0;
    bar.end(ui, "bake.page.scroll", &mut win.page_scroll);
}

fn draw_footer(
    ui: &mut Ui,
    footer: Rect,
    lang: Lang,
    refusal: Option<&str>,
    progress: Option<&super::Progress>,
    outcome: Option<(String, bool)>,
    actions: &mut Vec<BakeAction>,
) {
    let p = ui.painter().clone();
    w::fill(&p, footer, t::PANEL_HEADER);
    w::hline(&p, footer.left(), footer.right(), footer.top(), t::BORDER);
    let bake_label = lang.pick("チェックしたマップをベイク", "Bake Checked Maps");
    let cancel_label = lang.pick("取消", "Cancel");
    let close_label = lang.pick("閉じる", "Close");
    let button_w = |s: &str| w::text_width(&p, s, t::LABEL) + 28.0;
    let by = footer.top() + 12.0;
    let bh = 28.0;
    let mut x = footer.right() - 12.0;
    let close = Rect::from_min_size(
        pos2(x - button_w(close_label), by),
        vec2(button_w(close_label), bh),
    );
    x = close.left() - 8.0;
    let close_tip = progress.map(|_| {
        lang.pick(
            "ベイクは続きます。進み具合は画面の右下に出ます",
            "The bake goes on; its progress shows at the bottom right",
        )
    });
    if w::button(
        ui,
        close,
        "bake.close",
        close_label,
        false,
        true,
        close_tip,
        None,
    )
    .clicked()
    {
        actions.push(BakeAction::CloseWindow);
    }
    let main = Rect::from_min_size(
        pos2(x - button_w(bake_label) - 12.0, by),
        vec2(button_w(bake_label) + 12.0, bh),
    );
    if let Some(prog) = progress {
        if w::button(
            ui,
            main,
            "bake.cancel",
            cancel_label,
            false,
            !prog.canceling,
            Some(lang.pick(
                "ベイクを止めます。前のマップはそのままです",
                "Stop the bake; the previous maps stay as they are",
            )),
            Some("close"),
        )
        .clicked()
        {
            actions.push(BakeAction::Cancel);
        }
        // 進み具合
        let bar = Rect::from_min_max(
            pos2(footer.left() + 12.0, footer.top() + 24.0),
            pos2(main.left() - 12.0, footer.top() + 32.0),
        );
        w::rounded(&p, bar, t::CONTROL_BG, 3.0);
        let fill = Rect::from_min_size(
            bar.min,
            vec2(
                bar.width() * prog.fraction.clamp(0.0, 1.0) as f32,
                bar.height(),
            ),
        );
        w::rounded(&p, fill, t::ACCENT, 3.0);
        let set = if prog.total > 1 {
            format!(
                "{} {}/{}: {} · ",
                lang.pick("セット", "Set"),
                prog.index,
                prog.total,
                prog.set
            )
        } else {
            String::new()
        };
        let phase = if prog.canceling {
            lang.pick("取り消し中", "Canceling").to_owned()
        } else {
            phase_label(lang, &prog.phase)
        };
        let text = format!("{set}{phase}… {}%", (prog.fraction * 100.0) as i32);
        w::text(
            &p,
            Rect::from_min_max(
                pos2(bar.left(), footer.top() + 4.0),
                pos2(bar.right(), bar.top() - 2.0),
            ),
            &text,
            t::LABEL_DIM,
            Align::Left,
        );
    } else {
        if w::button(
            ui,
            main,
            "bake.start",
            bake_label,
            true,
            refusal.is_none(),
            Some(refusal.unwrap_or(lang.pick(
                "読み込んだモデルから焼きます。モデル・マテリアル・テクスチャは変わりません",
                "Bake from the loaded model. The model, its materials and textures are not changed",
            ))),
            Some("texture"),
        )
        .clicked()
        {
            actions.push(BakeAction::Start);
        }
        let (text, ok) = match (&outcome, refusal) {
            (Some((t, ok)), _) => (Some(t.as_str()), *ok),
            (None, Some(r)) => (Some(r), false),
            (None, None) => (None, true),
        };
        if let Some(text) = text {
            let r = Rect::from_min_max(
                pos2(footer.left() + 12.0, footer.top() + 6.0),
                pos2(main.left() - 12.0, footer.bottom() - 6.0),
            );
            let shown = w::fit(&p, text, r.width(), t::LABEL_DIM);
            w::text(
                &p,
                r,
                &shown,
                t::LABEL_DIM.with_color(if ok { t::TEXT_DIM } else { t::WARNING }),
                Align::Left,
            );
            if shown != text {
                ui.interact(r, Id::new("bake.outcome"), egui::Sense::hover())
                    .on_hover_text(text);
            }
        }
    }
}

/// ID マップのページの状態の行。最後のベイクで部品が 1 つだけだったときに限り、ID が 1 色だと知らせる（部品は ID の分け方で決まり、
/// 高ポリの有無とは関係ない。焼いていなければ何も言わない）。
pub fn id_status(lang: Lang, kind: MeshMapKind, report: Option<&MeshBakeReport>) -> Option<String> {
    (kind == MeshMapKind::Id && report.is_some_and(|r| r.id_parts == 1)).then(|| {
        lang.pick(
            "最後のベイクは部品が 1 つなので、ID は 1 色です",
            "The last bake had one part, so the ID is one color",
        )
        .to_owned()
    })
}

/// 焼く場所の一行（長ければ … で切り、全文と詳しい理由はツールチップ）。
fn place_line(ui: &mut Ui, p: &egui::Painter, r: Rect, id: &str, line: &PlaceLine) {
    let shown = w::fit(p, &line.text, r.width(), t::LABEL_DIM);
    w::text(
        p,
        r,
        &shown,
        t::LABEL_DIM.with_color(if line.warn { t::WARNING } else { t::TEXT_DIM }),
        Align::Left,
    );
    let tip = match (&line.detail, shown != line.text) {
        (Some(d), true) => Some(format!("{}\n{d}", line.text)),
        (Some(d), false) => Some(d.clone()),
        (None, true) => Some(line.text.clone()),
        (None, false) => None,
    };
    if let Some(tip) = tip {
        ui.interact(r, Id::new(id), egui::Sense::hover())
            .on_hover_text(tip);
    }
}

/// 最後のベイクの記録の行（文と、注意か）。注意（`MeshBakeNote`）だけ。時間の内訳・レイ・テクセル・三角形の数は開発用の数なので
/// 画面に出さない（記録 `MeshBakeReport` には残り、試験と計測が読む）。
pub fn report_lines(lang: Lang, r: &MeshBakeReport) -> Vec<(String, bool)> {
    r.notes
        .iter()
        .map(|note| (note_text(lang, note), true))
        .collect()
}

/// 記録の注意の文（日本語は core の文、英語は短い文）。
pub fn note_text(lang: Lang, note: &MeshBakeNote) -> String {
    if lang == Lang::Ja {
        // 法線の由来・ID の分け方の名前（方式の名前）は画面に出さない（記録の `MeshBakeNote` には残る）
        return match note {
            MeshBakeNote::ReconstructedNormals(_) => {
                "頂点法線は形から作り直しました。編集した法線は再現しません".into()
            }
            MeshBakeNote::IdParts { parts, .. } => format!("IDの部品 {parts}"),
            _ => note.to_string(),
        };
    }
    match note {
        MeshBakeNote::OverlappingTexels(n) => {
            format!("{n} texels have overlapping UVs (the lower triangle index wins)")
        }
        MeshBakeNote::ZeroUvAreaTriangles(n) => {
            format!("{n} triangles have no UV area (not baked, still occlude)")
        }
        MeshBakeNote::FaceNormals => "No vertex normals (face normals are used)".into(),
        MeshBakeNote::ReconstructedNormals(_) => {
            "Vertex normals were rebuilt from the shape. Edited normals are not reproduced".into()
        }
        MeshBakeNote::TangentFallback(n) => {
            format!("{n} triangles without tangents (made from the UVs)")
        }
        MeshBakeNote::IgnoredEdges(n) => {
            format!("{n} non-manifold or flipped-winding edges (ignored by curvature)")
        }
        MeshBakeNote::MissedSamples {
            missed, projected, ..
        } => format!("{missed}/{projected} samples missed the high poly and used this model"),
        MeshBakeNote::ManualIdColors(n) => format!("Manual ID colors override {n} parts"),
        MeshBakeNote::NoVertexColors => "No vertex colors (the ID is white)".into(),
        MeshBakeNote::IdParts { parts, .. } => format!("{parts} ID parts"),
        MeshBakeNote::NoReference => "No high poly (tangent normal is flat, height is 0.5)".into(),
    }
}

/// マップの意味（ツールチップ）。
pub fn kind_help(lang: Lang, kind: MeshMapKind) -> &'static str {
    match kind {
        MeshMapKind::WorldNormal => lang.pick(
            "モデルの滑らかな（頂点の）法線。モデルの空間のワールドの軸で、n × 0.5 + 0.5 で保存する",
            "The model's smooth (vertex) normals in model space: world axes, stored as n × 0.5 + 0.5",
        ),
        MeshMapKind::Position => lang.pick(
            "位置をモデルの境界箱で 0〜1 にしたもの（軸ごと）",
            "Positions normalized to the model's bounding box (0–1 on each axis)",
        ),
        MeshMapKind::AmbientOcclusion => lang.pick(
            "どれだけ開けているか。1 は何にも遮られず、0 はどの向きにも近くで遮られる",
            "How open each texel is: 1 is unblocked, 0 is blocked close by in every direction",
        ),
        MeshMapKind::Curvature => lang.pick(
            "0.5 は平ら。明るいほど凸（外側の縁）、暗いほど凹（内側の角）",
            "0.5 is flat; brighter is convex (outer edges), darker is concave (inner corners)",
        ),
        MeshMapKind::Thickness => lang.pick(
            "内側へ向けたレイが反対側へ出るまでの距離を最大の距離で割ったもの。0 は薄く、1 は厚い",
            "How far rays travel inward before leaving the other side, over the max distance: 0 is thin, 1 is thick",
        ),
        MeshMapKind::TangentNormal => lang.pick(
            "高ポリの法線をこのモデルの接空間で（OpenGL、Y+）。高ポリに当たらなければ平ら",
            "The high poly's normals in this model's tangent space (OpenGL, Y+). Flat where it is missed",
        ),
        MeshMapKind::Height => lang.pick(
            "投影に沿った、このモデルから高ポリまでの符号付きの距離（外側が明るい）。当たらなければ 0.5",
            "The signed distance from this model to the high poly along the projection (outside is brighter). 0.5 where it is missed",
        ),
        MeshMapKind::Id => lang.pick(
            "部品（スロット・メッシュ・つながった部品・UV アイランド）ごとの単色",
            "A flat color for each part (slot, mesh, connected mesh part or UV island)",
        ),
        MeshMapKind::BentNormal => lang.pick(
            "AO のレイの、遮られなかった向きの平均（ワールド空間）",
            "The average unblocked direction of the AO rays (world space)",
        ),
        MeshMapKind::Opacity => lang.pick(
            "高ポリに当たれば 1、外れれば 0（高ポリが無ければモデルが覆う所は 1）",
            "1 where the high poly is hit, 0 where it is missed (1 where the model covers when there is none)",
        ),
    }
}

fn id_source_help(lang: Lang, source: yolu_core::mesh_maps::MeshIdSource) -> &'static str {
    use yolu_core::mesh_maps::MeshIdSource as S;
    match source {
        S::MeshPart => lang.pick(
            "つながった部品（3D で辺を共有する三角形。UV の継ぎ目をまたぐ）ごとの色",
            "A color per connected piece of the mesh (triangles sharing edges in 3D, across UV seams)",
        ),
        S::UvIsland => lang.pick("UV アイランドごとの色", "A color per UV island"),
        S::Mesh => lang.pick("メッシュ（レンダラー）ごとの色", "A color per mesh (renderer)"),
        S::MaterialSlot => lang.pick(
            "マテリアルのスロット（レンダラーのサブメッシュ）ごとの色",
            "A color per material slot (a renderer's submesh)",
        ),
        S::VertexColor | S::MaterialAsset => "",
    }
}
