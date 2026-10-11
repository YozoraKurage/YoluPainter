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

use crate::notice::Source;
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
    let set = app.sets.get(index)?;
    if let Some(reason) = &set.read_only {
        return look(
            "lock",
            t::WARNING,
            format!("{}: {reason}", app.lang.pick("読むだけ", "Read-only")),
        );
    }
    let model = app.model.as_ref()?;
    let Some(material) = set.bound else {
        // 薄い切れた鎖。理由はツールチップ（マテリアルに付いていないセットと、今のモデルに無いマテリアルのセット）
        let tooltip = match &set.material {
            crate::sets::MaterialRef::Material { .. } => app.lang.pick(
                "今のモデルに無いマテリアル。鍵は残してあり、そのマテリアルのあるモデルでまた付く",
                "Material not in this model. Its reference is kept for models that use it.",
            ),
            _ => app.lang.pick(
                "今のモデルのマテリアルに付いていない",
                "Not assigned to a material of this model",
            ),
        };
        return look("link_off", t::TEXT_DIM, tooltip.into());
    };
    if !set.visible {
        return look(
            "visibility_off",
            t::TEXT_DIM,
            app.lang
                .pick("3D ビューに見せていない", "Hidden in the 3D View")
                .into(),
        );
    }
    if app.view3d.unpainted.contains(&(material as i32)) {
        return look(
            "warning",
            t::WARNING,
            app.lang
                .pick(
                    "3D ビューに絵を見せていない: GPU のメモリの予算が足りない（今のセットから遠いセットから見せない。絵と書き出しはそのまま）",
                    "Not shown in the 3D View: over the GPU memory budget (the sets farthest from the current one are left out; the texture and exports are unchanged)",
                )
                .into(),
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
    let name = lang.pick("メッシュマップをベイク…", "Bake Mesh Maps…");
    let marked = app.sets.current().mesh_maps.is_empty();
    let reason = if app.bake.is_baking() {
        Some(lang.pick("ベイク中", "Baking"))
    } else if marked {
        Some(lang.pick(
            "このテクスチャセットはまだベイクしていません",
            "This texture set has not been baked yet",
        ))
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
    let n = app.sets.len();
    let content = n as f32 * ROW_HEIGHT;
    let bar = Scroll::begin(ui, list, content, &mut app.ui.set_scroll);
    let row_width = list.width() - bar.reserved();
    for index in 0..n {
        let row = Rect::from_min_size(
            pos2(
                list.left(),
                list.top() + index as f32 * ROW_HEIGHT - app.ui.set_scroll,
            ),
            vec2(row_width, ROW_HEIGHT),
        );
        if row.bottom() < list.top() || row.top() > list.bottom() {
            continue;
        }
        set_row(ui, app, &ctx, list, row, index);
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
        lang.pick(
            "空のテクスチャセットを追加（今のセットと同じ大きさ・チャンネル）",
            "Add an empty texture set (same size and channels as this one)",
        ),
        false,
        free && app.sets.len() < crate::newproject::MAX_SETS,
        16.0,
    )
    .clicked()
    {
        action = Some(crate::newproject::NpAction::AddSet);
    }
    let only = app.sets.len() <= 1;
    if w::icon_button(
        ui,
        button(bar.left() + 34.0),
        "set.remove",
        "delete",
        if only {
            lang.pick(
                "プロジェクトには少なくとも 1 つのテクスチャセットが要ります",
                "A project keeps at least one texture set",
            )
        } else {
            lang.pick(
                "今のテクスチャセットを消す（確かめます。その作業は消えます）",
                "Remove this texture set (asked first; its work is lost)",
            )
        },
        false,
        free && !only,
        16.0,
    )
    .clicked()
    {
        action = Some(crate::newproject::NpAction::RemoveSets(vec![
            app.sets.current().uid,
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
        lang.pick("プロジェクト設定…", "Project Configuration…"),
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

fn set_row(
    ui: &mut Ui,
    app: &mut AppState,
    ctx: &egui::Context,
    list: Rect,
    row: Rect,
    index: usize,
) {
    let Some(set) = app.sets.get(index) else {
        return;
    };
    let (uid, name, visible) = (set.uid, set.name.clone(), set.visible);
    let material_ref = set.material.clone();
    let selected = index == app.sets.current_index();
    let free = !app.is_stroking();
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
            app.popup = Some(OpenPopup {
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
            app.lang.pick(
                "隠す（3D ビューと Unity に見せない）",
                "Hide in the 3D View and Unity",
            )
        } else {
            app.lang.pick("見せる", "Show")
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

    if app.ui.renaming_set == Some(uid) {
        let first = !app.ui.rename_set_started;
        app.ui.rename_set_started = true;
        let out = w::text_field(ui, name_rect, ("set.rename", uid), &name, None, first);
        if let Some(next) = out.committed {
            if let Err(e) = app.rename_set(uid, &next) {
                app.refuse(Source::TextureSet, e);
            }
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
        let shown = w::fit(&painter, &name, name_rect.width(), t::LABEL);
        w::text(
            &painter,
            name_rect,
            &shown,
            t::LABEL.with_color(color),
            Align::Left,
        );
    }
    // 行の上のツールチップ: マテリアル（詳しく。状態のアイコンの上ではアイコンの説明が先に出る）
    let detail = crate::sets::material_tooltip_in(&material_ref, app.lang);
    let response = response.on_hover_text(detail);
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, free, selected, &name));
}
