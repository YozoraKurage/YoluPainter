//! テクスチャセットのパネル（Substance Painter の Texture Set List の並び）: 行は目・名前・状態のアイコン・解像度。押すと今のセットを
//! 替え、ダブルクリックで名前を変え、右クリックでメニュー。マテリアルの名前や付いていない状態を書く帯は置かない（名前は行に出ている。
//! 付いていない状態は行のアイコン、理由と詳しいマテリアルはツールチップ）。
//! 下の帯に、足す・消す（`newproject`）・メッシュマップをベイク（炎のアイコン。ベイクのウィンドウを開く）・プロジェクトの構成のボタン。
//! ベイクのボタンは、今のセットにまだ焼いたメッシュマップが無いあいだ、アイコンの角に印を付ける（理由はツールチップ）。
//!
//! 状態のアイコン: 鍵 = 読むだけ（core で扱えない中身がある）、切れた鎖（薄い）= 今のモデルのマテリアルに付いていない、
//! 同期 = Unity に見せている、注意 = Unity 側に Color の流し込み先が無い（描いても Unity には見えない）・3D ビューのメモリの予算で
//! 絵を見せていない（両方あるときも、Unity に見せているときも、印は注意 1 つで、ツールチップが全部を言う）。画面にはアイコンだけを
//! 出し、説明はツールチップに置く。

use egui::{pos2, vec2, Color32, Rect, Sense, Ui, WidgetInfo, WidgetType};

use crate::engine::LayerSetId;
use crate::m2::UiOp;
use crate::state::{Action, AppState, OpenPopup, PopupKind};
use crate::ui::menu::{context_anchor, PopupState};
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};

pub const ROW_HEIGHT: f32 = 28.0;
/// 足す・消す・プロジェクトの構成のボタンの行の高さ。
pub const TOOLBAR_HEIGHT: f32 = 28.0;
/// 下の帯にベイクのボタンも並べられる幅（足す・消す・ベイク・設定の 4 つ、26 px ずつと間）。
const BAKE_BUTTON_MIN_BAR_WIDTH: f32 = 128.0;

/// セットの見え方: アイコンと色、ツールチップの説明（状態を文字では出さない）。
#[derive(Clone, Debug, PartialEq)]
pub struct SetLook {
    pub icon: &'static str,
    pub color: Color32,
    pub tooltip: String,
}

fn look(icon: &'static str, color: Color32, tooltip: String) -> Option<SetLook> {
    Some(SetLook {
        icon,
        color,
        tooltip,
    })
}

/// セットの見え方（無ければ普通）。
pub fn set_state(app: &AppState, index: usize) -> Option<SetLook> {
    let set = app.session.sets.get(index)?;
    if let Some(reason) = &set.read_only {
        return look(
            "lock",
            t::WARNING,
            format!("{}: {reason}", app.lang.t("panels-read-only-tip")),
        );
    }
    let model = app.session.model.as_ref()?;
    if set.bound.is_empty() {
        // 薄い切れた鎖。理由はツールチップ（マテリアルに付いていないセットと、今のモデルに無いマテリアルのセット）
        let named = set
            .materials
            .iter()
            .any(|m| matches!(m, crate::sets::MaterialRef::Material { .. }));
        let tooltip = if named {
            app.lang.t("panels-material-not-in-this-model-tip")
        } else {
            app.lang.t("panels-not-assigned-to-a-material-tip")
        };
        return look("link_off", t::TEXT_DIM, tooltip.into());
    }
    if !set.visible {
        return look(
            "visibility_off",
            t::TEXT_DIM,
            app.lang.t("panels-hidden-in-the-3d-view-tip").into(),
        );
    }
    if set
        .bound
        .iter()
        .any(|m| app.view3d.unpainted.contains(&(*m as i32)))
    {
        return look(
            "warning",
            t::WARNING,
            app.lang.t("panels-not-shown-in-the-3d-tip").into(),
        );
    }
    let _ = model;
    None
}

/// ベイクのボタンの見え方: 今のセットがまだ焼かれていないときの印と、ツールチップ（名前の行、あれば理由の行）。
#[derive(Clone, Debug, PartialEq)]
pub struct BakeEntrance {
    pub marked: bool,
    pub tooltip: String,
}

/// 今のセットのベイクのボタンの見え方。印はそのセットに焼いたメッシュマップが 1 枚も無いあいだ（ほかのセットが焼けていても付く）。
/// 理由は、焼いている最中ならそれ、そうでなければ「まだ焼いていない」。ウィンドウはどちらでも開ける（押せるのは描いていないときだけ）。
pub fn bake_entrance(app: &AppState) -> BakeEntrance {
    let lang = app.lang;
    let name = lang.t("panels-bake-mesh-maps");
    let marked = app.session.sets.current().mesh_maps.is_empty();
    let reason = if app.bake.is_baking() {
        Some(lang.t("panels-baking"))
    } else if marked {
        Some(lang.t("panels-this-texture-set-has-not"))
    } else {
        None
    };
    BakeEntrance {
        marked,
        tooltip: match reason {
            Some(reason) => format!("{name}\n{reason}"),
            None => name.to_owned(),
        },
    }
}

/// 一覧の行 1 つ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ListRow {
    /// テクスチャセット（並びの番号）。
    Texture(usize),
    /// テクスチャセット `texture` の、`index` 番目のレイヤーセット（そのテクスチャセットにレイヤーセットが 2 つ以上あるときだけ）。
    LayerSet { texture: usize, index: usize },
}

fn list_rows(app: &AppState) -> Vec<ListRow> {
    let mut rows = Vec::new();
    for texture in 0..app.session.sets.len() {
        rows.push(ListRow::Texture(texture));
        let n = app.set_doc(texture).layer_sets().len();
        if n >= 2 {
            rows.extend((0..n).map(|index| ListRow::LayerSet { texture, index }));
        }
    }
    rows
}

/// そのテクスチャセットの行が「選んでいる行」か（今のセットで、レイヤーセットの行が無いとき。レイヤーセットの行があるときは、そちらを選ぶ）。
fn texture_row_selected(app: &AppState, index: usize) -> bool {
    index == app.session.sets.current_index() && app.set_doc(index).layer_sets().len() < 2
}

pub fn show(ui: &mut Ui, app: &mut AppState) {
    let r = ui.max_rect();
    ui.advance_cursor_after_rect(r);
    let ctx = ui.ctx().clone();
    let toolbar = Rect::from_min_max(
        pos2(r.left(), (r.bottom() - TOOLBAR_HEIGHT).max(r.top())),
        r.max,
    );
    let list = Rect::from_min_max(r.min, pos2(r.right(), toolbar.top()));
    w::fill(ui.painter(), list, t::CONTROL_BG);
    // 行: テクスチャセットの行（レイヤーセットが 2 つ以上あるセットは、その下にレイヤーセットの行を字下げで並べる）
    let rows = list_rows(app);
    let content = rows.len() as f32 * ROW_HEIGHT;
    let bar = Scroll::begin(ui, list, content, &mut app.ui.set_scroll);
    let row_width = list.width() - bar.reserved();
    let free = !app.is_stroking();
    for (k, kind) in rows.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(
                list.left(),
                list.top() + k as f32 * ROW_HEIGHT - app.ui.set_scroll,
            ),
            vec2(row_width, ROW_HEIGHT),
        );
        if row.bottom() < list.top() || row.top() > list.bottom() {
            continue;
        }
        match *kind {
            ListRow::Texture(index) => set_row(ui, app, &ctx, list, row, index, free),
            ListRow::LayerSet { texture, index } => {
                layer_set_row(ui, app, &ctx, list, row, (texture, index, k), free)
            }
        }
    }
    // 並べ替えのドラッグの落とす先（行の上に重ねる線）
    if let Some(y) = ctx.data(|d| d.get_temp::<f32>(drop_line_id())) {
        let painter = ui.painter_at(list);
        painter.rect_filled(
            Rect::from_min_size(pos2(list.left() + 4.0, y - 1.0), vec2(row_width - 8.0, 2.0)),
            0.0,
            t::ACCENT,
        );
    }
    bar.end(ui, "texture_sets.scroll", &mut app.ui.set_scroll);

    // 足す・消す・プロジェクトの構成（今のセットのマテリアルや見え方を文字の行で繰り返さない。状態は行の印とツールチップ）
    toolbar_buttons(ui, app, toolbar);
}

/// 一覧の下のボタンの行: 空のセットを足す・今のセットを消す（確かめる）、右にメッシュマップをベイク（ウィンドウを開く）・プロジェクト設定を開く。
fn toolbar_buttons(ui: &mut Ui, app: &mut AppState, bar: Rect) {
    let p = ui.painter().clone();
    w::fill(&p, bar, t::PANEL_HEADER);
    w::hline(&p, bar.left(), bar.right(), bar.top(), t::BORDER);
    let lang = app.lang;
    let free = !app.is_stroking();
    let button =
        |x: f32| Rect::from_min_size(pos2(x, bar.top() + 2.0), vec2(26.0, bar.height() - 4.0));
    let mut action = None;
    let mut action_bake = false;
    if w::icon_button(
        ui,
        button(bar.left() + 4.0),
        "set.add",
        "add",
        lang.t("panels-add-an-empty-texture-set-tip"),
        false,
        free && app.session.sets.len() < crate::newproject::MAX_SETS,
        16.0,
    )
    .clicked()
    {
        action = Some(crate::newproject::NpAction::AddSet);
    }
    let only = app.session.sets.len() <= 1;
    if w::icon_button(
        ui,
        button(bar.left() + 34.0),
        "set.remove",
        "delete",
        if only {
            lang.t("panels-a-project-keeps-at-least-tip")
        } else {
            lang.t("panels-remove-this-texture-set-asked-tip")
        },
        false,
        free && !only,
        16.0,
    )
    .clicked()
    {
        action = Some(crate::newproject::NpAction::RemoveSets(vec![
            app.session.sets.current().uid,
        ]));
    }
    // 足す・消す・ベイク・設定の 4 つが重ならない幅があるときだけ（狭いときのベイクは、セットの右クリックのメニューと、操作 `bake.open` のキー・パイから）
    if bar.width() >= BAKE_BUTTON_MIN_BAR_WIDTH {
        let bake = bake_entrance(app);
        let bake_button = button(bar.right() - 60.0);
        let clicked = w::icon_button(
            ui,
            bake_button,
            "set.bake",
            "local_fire_department",
            &bake.tooltip,
            false,
            free,
            16.0,
        )
        .clicked();
        if bake.marked {
            // 印: アイコンの右上の角の小さな点（ボタンの押す・乗せる・使えないの見た目の上に重ねる）
            ui.painter().circle_filled(
                pos2(bake_button.center().x + 8.0, bake_button.center().y - 8.0),
                3.0,
                t::ACCENT,
            );
        }
        if clicked {
            action_bake = true;
        }
    }
    if w::icon_button(
        ui,
        button(bar.right() - 30.0),
        "set.configure",
        "tune",
        lang.t("panels-project-configuration-tip"),
        false,
        free,
        16.0,
    )
    .clicked()
    {
        action = Some(crate::newproject::NpAction::OpenConfigure);
    }
    if let Some(a) = action {
        app.apply(Action::Project(a));
    }
    if action_bake {
        app.apply(Action::Bake(crate::bake::BakeAction::OpenWindow));
    }
}

/// 行の名前（名前の変更中は入力欄）。複数のマテリアルを持つセットは、名前の右にマテリアルの名前を薄く出す。
fn set_name(
    ui: &mut Ui,
    app: &mut AppState,
    list: Rect,
    name_rect: Rect,
    (uid, name, selected, visible): (u32, &str, bool, bool),
    material_names: &[String],
) {
    let painter = ui.painter_at(list);
    if app.ui.renaming_set == Some(uid) {
        let first = !app.ui.rename_set_started;
        app.ui.rename_set_started = true;
        let out = w::text_field(ui, name_rect, ("set.rename", uid), name, None, first);
        if let Some(next) = out.committed {
            app.apply(Action::RenameSet(uid, next));
        }
        if !first && !out.focused {
            app.ui.renaming_set = None;
        }
    } else {
        let color = if selected {
            Color32::WHITE
        } else if !visible {
            t::TEXT_DIM
        } else {
            t::TEXT
        };
        let shown = w::fit(&painter, name, name_rect.width(), t::LABEL);
        w::text(
            &painter,
            name_rect,
            &shown,
            t::LABEL.with_color(color),
            Align::Left,
        );
        material_names_hint(&painter, app.lang, name_rect, &shown, material_names);
    }
}

/// 複数のマテリアルを持つセットだけ、名前の右にマテリアルの名前を薄く（プロジェクト設定の閉じた形と同じ書き方）。
fn material_names_hint(
    painter: &egui::Painter,
    lang: crate::lang::Lang,
    name_rect: Rect,
    shown: &str,
    material_names: &[String],
) {
    if material_names.len() <= 1 {
        return;
    }
    let name_w = w::text_width(painter, shown, t::LABEL);
    let at = Rect::from_min_max(
        pos2(name_rect.left() + name_w + 10.0, name_rect.top()),
        name_rect.max,
    );
    if at.width() > 8.0 {
        let names: Vec<&str> = material_names.iter().map(String::as_str).collect();
        let text = crate::sets::materials_label(lang, &names, |s| {
            w::text_width(painter, s, t::LABEL_DIM) <= at.width()
        });
        let text = w::fit(painter, &text, at.width(), t::LABEL_DIM);
        w::text(painter, at, &text, t::LABEL_DIM, Align::Left);
    }
}

fn set_row(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    list: Rect,
    row: Rect,
    index: usize,
    free: bool,
) {
    let Some(set) = app.session.sets.get(index) else {
        return;
    };
    let (uid, name, visible) = (set.uid, set.name.clone(), set.visible);
    let material_refs = set.materials.clone();
    let material_names = app.bound_material_names(index);
    let selected = texture_row_selected(app, index);
    let doc = app.set_doc(index);
    let resolution = if doc.width() == doc.height() {
        doc.width().to_string()
    } else {
        format!("{}×{}", doc.width(), doc.height())
    };
    let state = set_state(app, index);
    let hit = row.intersect(list);
    let response = ui.interact(
        hit,
        ui.make_persistent_id(("set.row", uid)),
        if free { Sense::click() } else { Sense::hover() },
    );
    let painter = ui.painter_at(list);
    if selected {
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
    let eye = Rect::from_min_size(
        pos2(row.left() + 4.0, row.top() + 3.0),
        vec2(24.0, row.height() - 6.0),
    );
    let res_w = w::text_width(&painter, &resolution, t::LABEL_DIM) + 4.0;
    let res_rect = Rect::from_min_max(
        pos2(row.right() - 8.0 - res_w, row.top()),
        pos2(row.right() - 8.0, row.bottom()),
    );
    let icon_rect = Rect::from_min_size(
        pos2(res_rect.left() - 22.0, row.top() + 5.0),
        vec2(18.0, row.height() - 10.0),
    );
    let name_rect = Rect::from_min_max(
        pos2(eye.right() + 6.0, row.top() + 4.0),
        pos2(icon_rect.left() - 4.0, row.bottom() - 4.0),
    );

    if response.clicked() {
        app.apply(Action::SelectSet(uid));
        if app.ui.renaming_set != Some(uid) {
            app.ui.renaming_set = None;
        }
    }
    if response.double_clicked()
        && ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| name_rect.contains(p))
    {
        app.apply(Action::StartRenameSet(uid));
    }
    if response.secondary_clicked() {
        if let Some(at) = response.interact_pointer_pos() {
            app.ui.popup = Some(OpenPopup {
                kind: PopupKind::SetContext(uid),
                state: PopupState::new(ctx, context_anchor(at)),
            });
        }
    }

    if w::icon_button(
        ui,
        eye,
        ("set.eye", uid),
        if visible {
            "visibility"
        } else {
            "visibility_off"
        },
        if visible {
            app.lang.t("panels-hide-in-the-3d-view-tip")
        } else {
            app.lang.t("panels-show-2-tip")
        },
        false,
        true,
        16.0,
    )
    .clicked()
    {
        app.apply(Action::ToggleSetVisible(uid));
    }

    let painter = ui.painter_at(list);
    if let Some(l) = &state {
        w::icon(&painter, icon_rect, l.icon, l.color, 15.0);
        ui.interact(
            icon_rect.intersect(list),
            ui.make_persistent_id(("set.state", uid)),
            Sense::hover(),
        )
        .on_hover_text(l.tooltip.as_str());
    }
    w::text(&painter, res_rect, &resolution, t::LABEL_DIM, Align::Right);

    set_name(
        ui,
        app,
        list,
        name_rect,
        (uid, &name, selected, visible),
        &material_names,
    );
    // 行の上のツールチップ: マテリアル（詳しく。状態のアイコンの上ではアイコンの説明が先に出る）
    let detail = material_refs
        .iter()
        .map(|m| crate::sets::material_tooltip_in(m, app.lang))
        .collect::<Vec<_>>()
        .join("\n");
    let response = response.on_hover_text(detail);
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, free, selected, &name));
}

/// レイヤーセットの行の左の字下げ（└ の線の位置）と、名前の始まり。
const SET_INDENT: f32 = 16.0;
const SET_NAME_INDENT: f32 = 32.0;

/// ドラッグで並べ替えている間の落とす先の線の位置（一覧の上端からの画面の y。ドラッグの間だけ）。
fn drop_line_id() -> egui::Id {
    egui::Id::new("texture_sets.layer_set_drop")
}

/// レイヤーセットの行 1 つの事実（描く前に読む。`texture` はテクスチャセットの並びの番号、`index` はそのレイヤーセットの番号、
/// `at` は一覧の中の行の番号）。
struct LayerSetRow {
    uid: u32,
    set: LayerSetId,
    index: usize,
    at: usize,
    count: usize,
    name: String,
    /// どのチャンネルも出力していない。
    no_output: bool,
    /// 今のテクスチャセットの行。
    in_current: bool,
    /// 今のレイヤーセットの行。
    selected: bool,
}

fn layer_set_facts(
    app: &AppState,
    (texture, index, at): (usize, usize, usize),
) -> Option<LayerSetRow> {
    let uid = app.session.sets.get(texture)?.uid;
    let doc = app.set_doc(texture);
    let set = doc.layer_sets().get(index)?.id;
    let in_current = texture == app.session.sets.current_index();
    Some(LayerSetRow {
        uid,
        set,
        index,
        at,
        count: doc.layer_sets().len(),
        name: crate::m2::layersets::display_name(app.lang, doc, index),
        no_output: crate::m2::layersets::outputs(doc, set).is_empty(),
        in_current,
        selected: in_current && app.layer_set() == set,
    })
}

/// レイヤーセットの行（テクスチャセットの行の下に字下げで並ぶ）。押すと、そのテクスチャセット・レイヤーの一覧・描くチャンネルが一度に替わる。
/// どのチャンネルも出力していないレイヤーセットは、行の右に切れた鎖の印。ダブルクリックで名前を変え、右クリックでメニュー。今のテクスチャセットの
/// レイヤーセットは、ドラッグで並べ替えられる。`position` は（テクスチャセットの番号, レイヤーセットの番号, 一覧の中の行の番号）。
fn layer_set_row(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    list: Rect,
    row: Rect,
    position: (usize, usize, usize),
    free: bool,
) {
    let Some(r) = layer_set_facts(app, position) else {
        return;
    };
    let response = ui.interact(
        row.intersect(list),
        ui.make_persistent_id(("layer_set.row", r.uid, r.set.0)),
        if free && r.in_current {
            Sense::click_and_drag()
        } else if free {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let name_rect = paint_layer_set_row(ui, app.lang, list, row, &r, &response);
    layer_set_clicks(ui, app, ctx, &r, &response, name_rect);
    if r.in_current && free {
        layer_set_reorder(ui, app, ctx, list, &r, &response);
    }
    layer_set_name(ui, app, list, name_rect, &r);
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::SelectableLabel, free, r.selected, &r.name)
    });
}

/// 行の地・字下げの線・出力なしの印を描く。名前を置く矩形を返す。
fn paint_layer_set_row(
    ui: &mut Ui,
    lang: crate::lang::Lang,
    list: Rect,
    row: Rect,
    r: &LayerSetRow,
    response: &egui::Response,
) -> Rect {
    let painter = ui.painter_at(list);
    if r.selected {
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
    // 字下げの線（└）
    let line_x = row.left() + SET_INDENT;
    let bottom = if r.index + 1 == r.count {
        row.center().y
    } else {
        row.bottom()
    };
    w::vline(&painter, line_x, row.top(), bottom, t::SEPARATOR);
    w::hline(
        &painter,
        line_x,
        line_x + 10.0,
        row.center().y,
        t::SEPARATOR,
    );
    let mut name_right = row.right() - 8.0;
    if r.no_output {
        // どのチャンネルも出力していない: 名前を薄くして、切れた鎖（理由はツールチップ）
        let mark = Rect::from_min_size(
            pos2(name_right - 18.0, row.top() + 5.0),
            vec2(18.0, row.height() - 10.0),
        );
        w::icon(&painter, mark, "link_off", t::TEXT_DIM, 15.0);
        let tip = lang.t("layersets-no-output-tip");
        let hover = ui.interact(
            mark.intersect(list),
            ui.make_persistent_id(("layer_set.off", r.uid, r.set.0)),
            Sense::hover(),
        );
        hover.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, tip));
        hover.on_hover_text(tip);
        name_right = mark.left() - 4.0;
    }
    Rect::from_min_max(
        pos2(row.left() + SET_NAME_INDENT, row.top() + 4.0),
        pos2(name_right, row.bottom() - 4.0),
    )
}

/// 押す・ダブルクリック・右クリック。
fn layer_set_clicks(
    ui: &Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    r: &LayerSetRow,
    response: &egui::Response,
    name_rect: Rect,
) {
    // 押す: そのテクスチャセット・レイヤーのレイヤーセット・描くチャンネルが一度に替わる
    if response.clicked() {
        app.apply(Action::SelectSet(r.uid));
        app.apply(Action::M2Ui(UiOp::SelectLayerSet(r.set)));
        if app.m2.renaming_layer_set != Some(r.set) {
            app.m2.renaming_layer_set = None;
        }
    }
    if response.double_clicked()
        && ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| name_rect.contains(p))
    {
        app.apply(Action::SelectSet(r.uid));
        app.apply(Action::M2Ui(UiOp::RenameLayerSet(r.set)));
    }
    if response.secondary_clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            app.apply(Action::SelectSet(r.uid));
            app.apply(Action::M2Ui(UiOp::SelectLayerSet(r.set)));
            app.ui.popup = Some(OpenPopup {
                kind: PopupKind::M2(crate::m2_menu::Popup::LayerSetContext(r.set)),
                state: PopupState::new(ctx, context_anchor(pos)),
            });
        }
    }
}

/// ドラッグで並べ替え（今のテクスチャセットの中）。落とす先は、ポインタの下の行の位置。
fn layer_set_reorder(
    ui: &Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    list: Rect,
    r: &LayerSetRow,
    response: &egui::Response,
) {
    let first = r.at - r.index;
    let top = list.top() - app.ui.set_scroll + first as f32 * ROW_HEIGHT;
    let target = |pointer: egui::Pos2| {
        (((pointer.y - top) / ROW_HEIGHT).floor().max(0.0) as usize).min(r.count - 1)
    };
    if response.dragged() {
        if let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) {
            let to = target(pointer);
            let y = if to > r.index {
                top + (to + 1) as f32 * ROW_HEIGHT
            } else {
                top + to as f32 * ROW_HEIGHT
            };
            ctx.data_mut(|d| d.insert_temp(drop_line_id(), y));
        }
    }
    if response.drag_stopped() {
        ctx.data_mut(|d| d.remove::<f32>(drop_line_id()));
        if let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) {
            let to = target(pointer);
            if to != r.index {
                app.apply(Action::M2(crate::m2::Edit::LayerSets(
                    crate::m2::layersets::LayerSetEdit::Move(r.set, to),
                )));
            }
        }
    }
}

/// 名前（名前を変えている間は入力欄）。
fn layer_set_name(ui: &mut Ui, app: &mut AppState, list: Rect, name_rect: Rect, r: &LayerSetRow) {
    if r.selected && app.m2.renaming_layer_set == Some(r.set) {
        let first = !app.m2.rename_layer_set_started;
        app.m2.rename_layer_set_started = true;
        let out = w::text_field(
            ui,
            name_rect,
            ("layer_set.rename", r.uid, r.set.0),
            &r.name,
            None,
            first,
        );
        if let Some(next) = out.committed {
            app.apply(Action::M2(crate::m2::Edit::LayerSets(
                crate::m2::layersets::LayerSetEdit::Rename(r.set, next),
            )));
        }
        if !first && !out.focused {
            app.m2.renaming_layer_set = None;
        }
        return;
    }
    let color = if r.selected {
        Color32::WHITE
    } else if r.no_output {
        t::TEXT_DIM
    } else {
        t::TEXT
    };
    let painter = ui.painter_at(list);
    let shown = w::fit(&painter, &r.name, name_rect.width(), t::LABEL);
    w::text(
        &painter,
        name_rect,
        &shown,
        t::LABEL.with_color(color),
        Align::Left,
    );
}
