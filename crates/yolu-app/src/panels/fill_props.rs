//! 塗りつぶしのレイヤーのプロパティ（Unity 版の FillImages・Decals・FillGradients・GradientRamp・ShapeGradient の欄）: 描くチャンネルの画像、層の投影
//! （UV・トライプラナー・平面・球・円柱・デカール、タイル・オフセット・回転、トライプラナーのぼかし、デカールの間引き、モデルの上の置き場）、描くチャンネルの
//! グラデーションデカール（形・置き場・減衰・色と不透明度の分岐点・値のカーブ）。値は `Action::Fill` を通る（1 回の Undo。数値とスライダーの
//! ドラッグは離したところで区切る）。画面には名前と値と短い理由だけを出し、使い方の説明はツールチップ。

use egui::{pos2, vec2, DragAndDrop, Rect, Sense, Ui, WidgetInfo, WidgetType};
use yolu_core::fill_image::{Projection, ProjectionMode, Wrap};
use yolu_core::generator::{Preset, Ramp, Settings, Shape};
use yolu_core::{Channel, ChannelKind, ImageId, InactiveEffect, InactiveTarget, LayerId, Rgba8};

use super::properties::{percent_row, section, slider_row, toggle_row};
use crate::fillfx::{inputs, FillOp};
use crate::fx::names::{shape_tooltip, SHAPES};
use crate::lang::Lang;
use crate::m2::{self};
use crate::m2_menu::Popup;
use crate::shelf::{ItemKind, ShelfDrag};
use crate::state::{Action, AppState, OpenPopup, PopupKind};
use crate::ui::menu::{Entry, PopupState};
use crate::ui::numfield::{number_field, NumSpec};
use crate::ui::ramp::ops as ramp_ops;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows};
use crate::view3d::shape_gizmo::{wrap_degrees, Mode, AXIS_X, AXIS_Y, AXIS_Z};

const LABEL_W: f32 = 64.0;
/// X・Y・Z の 3 つの欄の行の名前の幅（3 つの欄に値が収まるように、2 つの欄の行より狭く）。
const VEC3_LABEL_W: f32 = 50.0;

fn fill(app: &mut AppState, op: FillOp) {
    app.apply(Action::Fill(op));
}

/// ポップアップを開く。
fn open(app: &mut AppState, ctx: &egui::Context, popup: Popup, anchor: Rect) {
    app.popup = Some(OpenPopup {
        kind: PopupKind::M2(popup),
        state: PopupState::new(ctx, anchor).with_min_width(anchor.width()),
    });
}

/// 効かない理由の行（警告の印と短い理由。入りきらなければ切り、全文はツールチップ）。
fn warn_row(ui: &mut Ui, rows: &mut Rows, text: &str) {
    let r = rows.row(t::ROW_HEIGHT, 2.0);
    let p = ui.painter();
    w::icon(
        p,
        Rect::from_min_size(r.min, vec2(18.0, r.height())),
        "warning",
        t::WARNING,
        14.0,
    );
    let area = Rect::from_min_max(pos2(r.left() + 20.0, r.top()), r.max);
    let shown = w::fit(p, text, area.width(), t::LABEL_DIM);
    w::text(
        p,
        area,
        &shown,
        t::LABEL_DIM.with_color(t::WARNING),
        w::Align::Left,
    );
    ui.interact(r, ui.id().with(("fill.warn", text)), Sense::hover())
        .on_hover_text(text);
}

fn problem(
    problems: &[InactiveEffect],
    lang: Lang,
    pick: impl Fn(&InactiveTarget) -> bool,
) -> Option<String> {
    problems
        .iter()
        .find(|e| pick(&e.target))
        .map(|e| lang.inactive_reason(&e.reason))
}

/// X・Y・Z の 3 つの数値の欄の 1 行（軸の色の下線）。変えたら新しい値の 3 つを返す。
#[allow(clippy::too_many_arguments)]
fn vec3_row(
    ui: &mut Ui,
    rows: &mut Rows,
    id: &str,
    label: &str,
    values: [f64; 3],
    spec: &NumSpec,
    tooltip: &str,
    enabled: bool,
) -> Option<[f64; 3]> {
    let row = rows.row(t::ROW_HEIGHT, 3.0);
    w::text(
        ui.painter(),
        Rect::from_min_size(row.min, vec2(VEC3_LABEL_W, row.height())),
        label,
        t::LABEL,
        w::Align::Left,
    );
    let cells = Rows::split(
        Rect::from_min_max(pos2(row.left() + VEC3_LABEL_W, row.top()), row.max),
        3,
        3.0,
    );
    let mut next = values;
    let mut changed = false;
    for (k, cell) in cells.iter().enumerate() {
        let out = number_field(
            ui,
            *cell,
            (id, k),
            "", // 軸の見分けは下線の色（X 赤・Y 緑・Z 青）。文字を置くと値が入りきらない
            values[k],
            spec,
            Some(&format!("{}: {tooltip}", ["X", "Y", "Z"][k])),
            Some([AXIS_X, AXIS_Y, AXIS_Z][k]),
            enabled,
        );
        if out.changed {
            next[k] = out.value;
            changed = true;
        }
    }
    changed.then_some(next)
}

/// U・V の 2 つの数値の欄の 1 行。
#[allow(clippy::too_many_arguments)]
fn vec2_row(
    ui: &mut Ui,
    rows: &mut Rows,
    id: &str,
    label: &str,
    values: [f64; 2],
    spec: &NumSpec,
    tooltip_pair: [&str; 2],
    enabled: bool,
) -> Option<[f64; 2]> {
    let row = rows.row(t::ROW_HEIGHT, 3.0);
    w::text(
        ui.painter(),
        Rect::from_min_size(row.min, vec2(LABEL_W, row.height())),
        label,
        t::LABEL,
        w::Align::Left,
    );
    let cells = Rows::split(
        Rect::from_min_max(pos2(row.left() + LABEL_W, row.top()), row.max),
        2,
        3.0,
    );
    let mut next = values;
    let mut changed = false;
    for (k, cell) in cells.iter().enumerate() {
        let out = number_field(
            ui,
            *cell,
            (id, k),
            ["U", "V"][k],
            values[k],
            spec,
            Some(tooltip_pair[k]),
            None,
            enabled,
        );
        if out.changed {
            next[k] = out.value;
            changed = true;
        }
    }
    changed.then_some(next)
}

// ───────── 全体 ─────────

/// 塗りつぶしの層の画像・投影・グラデーションの欄（値の欄のあとに並べる）。
pub fn sections(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    id: LayerId,
    enabled: bool,
    lang: Lang,
) {
    let channel = app.m2.paint_channel;
    let problems: Vec<InactiveEffect> = app
        .doc
        .inactive_effect_list()
        .into_iter()
        .filter(|e| e.layer == id)
        .collect();
    if channel.is_standard() {
        image_section(ui, app, rows, id, channel, &problems, enabled, lang);
    }
    projection_section(ui, app, rows, id, &problems, enabled, lang);
    if channel.is_standard() && channel != Channel::Normal {
        gradient_section(ui, app, rows, id, channel, &problems, enabled, lang);
    }
}

// ───────── 画像 ─────────

#[allow(clippy::too_many_arguments)]
fn image_section(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    id: LayerId,
    channel: Channel,
    problems: &[InactiveEffect],
    enabled: bool,
    lang: Lang,
) {
    let name = m2::channel_name(lang, &app.doc, channel);
    let (open_section, _) = section(
        ui,
        app,
        rows,
        "fill-image",
        &format!("{} · {name}", lang.pick("画像", "Image")),
        "texture",
        None,
    );
    if !open_section {
        return;
    }
    let ctx_for_popup = ui.ctx().clone();
    let image = app.doc.layer(id).and_then(|l| l.fill_image(channel));
    let row = rows.row(26.0, 4.0);
    let clear_w = if image.is_some() { 28.0 } else { 0.0 };
    let box_rect = Rect::from_min_max(row.min, pos2(row.right() - clear_w, row.bottom()));
    image_box(ui, app, box_rect, id, channel, image, enabled, lang);
    if image.is_some() {
        let b = Rect::from_min_size(
            pos2(row.right() - 26.0, row.top()),
            vec2(26.0, row.height()),
        );
        if w::icon_button(
            ui,
            b,
            "fill.image.clear",
            "close",
            lang.pick(
                "画像を外す（値に戻る）",
                "Remove the image (the channel shows its value)",
            ),
            false,
            enabled,
            15.0,
        )
        .clicked()
        {
            fill(
                app,
                FillOp::Image {
                    layer: id,
                    channel,
                    image: None,
                },
            );
        }
        // 読み方（棚の画像の色空間。この画像を読む全部の層に効く）
        if let Some(space) = image
            .and_then(|i| app.shelf.get(&inputs::resource_id(i)))
            .map(|r| inputs::space_of(r).1)
        {
            let r = rows.row(t::ROW_HEIGHT, 4.0);
            let (response, anchor) = w::dropdown(
                ui,
                r,
                "fill.image.space",
                Some(lang.pick("読み方", "Read as")),
                space_name(lang, space),
                Some(lang.pick(
                    "画像の値が何か（棚の画像の色空間。この画像を読む全部の層に効き、取り消しには入らない）。データのチャンネルは常に値のまま、色のチャンネルではリニアの画像を sRGB に直して読む",
                    "What the image's values are (the shelf image's colour space, for every layer that reads it; not an undo step). Data channels always use the values as stored; in a colour channel a linear image is encoded to sRGB",
                )),
                enabled,
                LABEL_W + 22.0,
            );
            if response.clicked() {
                if let Some(image) = image {
                    open(app, &ctx_for_popup, Popup::ImageSpace(image), anchor);
                }
            }
        }
        if let Some(why) = problem(problems, lang, |t| *t == InactiveTarget::FillImage(channel)) {
            warn_row(ui, rows, &why);
        }
    }
}

/// 画像の箱: サムネイルと名前（無ければ印だけ）。押すと棚の画像の一覧、棚の素材のドラッグを受ける。
#[allow(clippy::too_many_arguments)]
fn image_box(
    ui: &mut Ui,
    app: &mut AppState,
    r: Rect,
    layer: LayerId,
    channel: Channel,
    image: Option<ImageId>,
    enabled: bool,
    lang: Lang,
) {
    let ctx = ui.ctx().clone();
    let id = ui.make_persistent_id(("fill.image", channel.index()));
    let response = ui.interact(
        r,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let dragged: Option<std::sync::Arc<ShelfDrag>> =
        DragAndDrop::payload::<ShelfDrag>(&ctx).filter(|d| d.kind == ItemKind::Image);
    let dropping = enabled && dragged.is_some() && ui.rect_contains_pointer(r);
    let rid = image.map(inputs::resource_id);
    let resource = rid
        .as_deref()
        .and_then(|rid| app.shelf.get(rid))
        .map(|res| (res.id.clone(), res.name.clone()));
    // サムネイルは別のスレッドで作る（大きな画像でも画面を止めない）。できるまでは種類のアイコン
    if let Some((rid, _)) = &resource {
        let cache = app.library.cache().cloned();
        app.shelf
            .request_inspections(std::slice::from_ref(rid), cache.as_ref());
    }
    let thumb_tex = resource
        .as_ref()
        .and_then(|(rid, _)| app.shelf.texture(&ctx, rid));
    let p = ui.painter();
    w::rounded(
        p,
        r,
        if dropping {
            t::ACCENT_DIM
        } else if enabled && response.hovered() {
            t::CONTROL_HOVER
        } else {
            t::CONTROL_BG
        },
        3.0,
    );
    w::outline(p, r, if dropping { t::ACCENT } else { t::BORDER }, 1.0, 3.0);
    let thumb = Rect::from_min_size(
        pos2(r.left() + 3.0, r.top() + 3.0),
        vec2(r.height() - 6.0, r.height() - 6.0),
    );
    w::checker(p, thumb, 3.0);
    match thumb_tex {
        Some(tex) => {
            p.image(
                tex.id(),
                thumb,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        None => w::icon(
            p,
            thumb,
            if image.is_some() && resource.is_none() {
                "link_off"
            } else {
                "texture"
            },
            t::TEXT_DIM,
            13.0,
        ),
    }
    w::outline(p, thumb, t::BORDER, 1.0, 0.0);
    let label = match (&resource, image) {
        (Some((_, name)), _) => name.clone(),
        (None, Some(_)) => lang.pick("見つからない画像", "Missing image").into(),
        _ => String::new(),
    };
    let area = Rect::from_min_max(
        pos2(thumb.right() + 6.0, r.top()),
        pos2(r.right() - 22.0, r.bottom()),
    );
    let shown = w::fit(p, &label, area.width(), t::LABEL);
    w::text(
        p,
        area,
        &shown,
        t::LABEL.with_color(if enabled { t::TEXT } else { t::TEXT_DISABLED }),
        w::Align::Left,
    );
    w::icon(
        p,
        Rect::from_min_size(pos2(r.right() - 22.0, r.top()), vec2(20.0, r.height())),
        "arrow_drop_down",
        t::TEXT_DIM,
        16.0,
    );
    let tip = lang.pick(
        "画像（押すとこのプロジェクトの画像の一覧。アセットのパネルの画像をドラッグして落とせる）",
        "Image (click for this project's images; drop one from the Assets panel)",
    );
    let info = if label.is_empty() {
        lang.pick("画像", "Image").to_owned()
    } else {
        label.clone()
    };
    response.widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, enabled, &info));
    let response = response.on_hover_text(tip);
    if enabled && response.clicked() {
        open(app, &ctx, Popup::FillImage(layer, channel), r);
    }
    if dropping && ui.input(|i| i.pointer.any_released()) {
        if let Some(drag) = dragged {
            if let Some(image) = inputs::image_id(&drag.id) {
                fill(
                    app,
                    FillOp::Image {
                        layer,
                        channel,
                        image: Some(image),
                    },
                );
            }
        }
        DragAndDrop::clear_payload(&ctx);
    }
}

/// 棚の画像の読み方（色空間）の名前。
pub fn space_name(lang: Lang, space: yolu_core::ImageColorSpace) -> &'static str {
    match space {
        yolu_core::ImageColorSpace::Srgb => lang.pick("色（sRGB）", "Color (sRGB)"),
        yolu_core::ImageColorSpace::Linear => lang.pick("データ（リニア）", "Data (linear)"),
        yolu_core::ImageColorSpace::Unspecified => {
            lang.pick("不明（そのまま）", "Unknown (as stored)")
        }
    }
}

fn space_entries(app: &AppState, image: ImageId) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let current = app
        .shelf
        .get(&inputs::resource_id(image))
        .map(|r| inputs::space_of(r).1);
    [
        yolu_core::ImageColorSpace::Srgb,
        yolu_core::ImageColorSpace::Linear,
        yolu_core::ImageColorSpace::Unspecified,
    ]
    .into_iter()
    .map(|space| {
        Entry::item(
            space_name(lang, space),
            Action::Fill(FillOp::ImageColorSpace { image, space }),
        )
        .radio(current == Some(space))
        .enabled(!app.is_stroking() && app.shelf.unavailable.is_none())
    })
    .collect()
}

/// 画像の一覧のポップアップ（棚の画像・ファイルから取り込む・外す）。
fn image_entries(app: &AppState, layer: LayerId, channel: Channel) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let free = !app.is_stroking();
    let current = app.doc.layer(layer).and_then(|l| l.fill_image(channel));
    let mut v = Vec::new();
    for r in app.shelf.resources().iter().filter(|r| r.kind == "image") {
        let Some(image) = inputs::image_id(&r.id) else {
            continue;
        };
        let (w, h) = (
            r.metadata["width"].as_u64().unwrap_or(0),
            r.metadata["height"].as_u64().unwrap_or(0),
        );
        v.push(
            Entry::item(
                format!("{}  ({w} × {h})", r.name),
                Action::Fill(FillOp::Image {
                    layer,
                    channel,
                    image: Some(image),
                }),
            )
            .radio(current == Some(image))
            .enabled(free),
        );
    }
    if !v.is_empty() {
        v.push(Entry::Separator);
    }
    v.push(
        Entry::item(
            lang.pick("ファイルから取り込む…", "Import from File…"),
            Action::Fill(FillOp::ImportImageDialog),
        )
        .enabled(free && app.shelf.unavailable.is_none()),
    );
    if current.is_some() {
        v.push(
            Entry::item(
                lang.pick("画像を外す", "Remove the Image"),
                Action::Fill(FillOp::Image {
                    layer,
                    channel,
                    image: None,
                }),
            )
            .enabled(free),
        );
    }
    v
}

// ───────── 投影 ─────────

const MODES: [ProjectionMode; 6] = [
    ProjectionMode::Uv,
    ProjectionMode::Triplanar,
    ProjectionMode::Planar,
    ProjectionMode::Spherical,
    ProjectionMode::Cylindrical,
    ProjectionMode::Decal,
];

pub fn projection_name(lang: Lang, mode: ProjectionMode) -> &'static str {
    match mode {
        ProjectionMode::Uv => lang.pick("UV", "UV"),
        ProjectionMode::Triplanar => lang.pick("トライプラナー", "Tri-planar"),
        ProjectionMode::Planar => lang.pick("平面", "Planar"),
        ProjectionMode::Spherical => lang.pick("球", "Spherical"),
        ProjectionMode::Cylindrical => lang.pick("円柱", "Cylindrical"),
        ProjectionMode::Decal => lang.pick("デカール", "Decal"),
    }
}

pub fn wrap_name(lang: Lang, wrap: Wrap) -> &'static str {
    match wrap {
        Wrap::Repeat => lang.pick("繰り返す", "Repeat"),
        Wrap::Clamp => lang.pick("端を伸ばす", "Clamp to edge"),
        Wrap::None => lang.pick("透明", "Transparent"),
    }
}

const WRAPS: [Wrap; 3] = [Wrap::Repeat, Wrap::Clamp, Wrap::None];

/// 置き場（箱）の欄の名前。
fn size_tooltip(lang: Lang, mode: ProjectionMode) -> &'static str {
    match mode {
        ProjectionMode::Planar => lang.pick(
            "平面: 画像は箱の X と Y（−Z の側から見て）に広がる。Z は使わない",
            "Planar: the image spans the box's X and Y (seen from its −Z side); Z is not used",
        ),
        ProjectionMode::Decal => lang.pick(
            "デカール: 画像は箱の X と Y に広がる。Z は面の中へ届く深さ",
            "Decal: the image spans the box's X and Y; Z is how deep it reaches into the surface",
        ),
        ProjectionMode::Cylindrical => lang.pick(
            "円柱: 画像は箱の Y 軸のまわりを 1 周し、Y の大きさに広がる。X と Z は使わない",
            "Cylindrical: the image goes once around the box's Y axis and spans its Y size; X and Z are not used",
        ),
        _ => lang.pick(
            "トライプラナー: 各面の画像は箱のほかの 2 つの大きさに広がる",
            "Tri-planar: each face's image spans the box's two other sizes",
        ),
    }
}

fn projection_section(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    id: LayerId,
    problems: &[InactiveEffect],
    enabled: bool,
    lang: Lang,
) {
    let ctx = ui.ctx().clone();
    let (open_section, _) = section(
        ui,
        app,
        rows,
        "fill-projection",
        lang.pick("投影", "Projection"),
        "view_in_ar",
        None,
    );
    if !open_section {
        return;
    }
    let Some(p) = app.doc.layer(id).map(|l| *l.projection()) else {
        return;
    };
    let mut next = p;
    // 種類
    let r = rows.row(t::ROW_HEIGHT, 4.0);
    let (response, anchor) = w::dropdown(
        ui,
        r,
        "projection.mode",
        Some(lang.pick("投影", "Projection")),
        projection_name(lang, p.mode),
        Some(lang.pick(
            "UV: UV の正方形に画像を敷く。トライプラナー: 箱の 3 つの軸から投影して面の向きで混ぜる（UV の継ぎ目が出ない）。平面: 箱の正面から。球・円柱: 箱の中心のまわりに。デカール: 箱の正面から、箱の中だけに、画像のアルファで切り抜く",
            "UV: the image on the UV square. Tri-planar: three projections along the box's axes, mixed where the surface turns (no UV seams). Planar: through the box's front face. Spherical and cylindrical: around the box's centre. Decal: through the box's front face, only inside the box, cut out by the image's alpha",
        )),
        enabled,
        LABEL_W + 22.0,
    );
    if response.clicked() {
        open(app, &ctx, Popup::ProjectionMode(id), anchor);
    }
    // 外側
    let r = rows.row(t::ROW_HEIGHT, 4.0);
    let (response, anchor) = w::dropdown(
        ui,
        r,
        "projection.wrap",
        Some(lang.pick("外側", "Outside")),
        wrap_name(lang, p.wrap),
        Some(lang.pick(
            "画像の外を、繰り返す・端の画素を伸ばす・透明にする",
            "Repeat the image, continue its edge pixels, or leave it transparent outside the image",
        )),
        enabled,
        LABEL_W + 22.0,
    );
    if response.clicked() {
        open(app, &ctx, Popup::ProjectionWrap(id), anchor);
    }
    // タイル・オフセット・回転
    if let Some(v) = vec2_row(
        ui,
        rows,
        "projection.tiles",
        lang.pick("タイル", "Tiling"),
        p.tiles,
        &NumSpec::new(1e-3, 1e4, 0.01, 3),
        [
            lang.pick(
                "画像を U 方向に繰り返す回数",
                "How many times the image repeats across the projected square (U)",
            ),
            lang.pick(
                "画像を V 方向に繰り返す回数",
                "How many times the image repeats across the projected square (V)",
            ),
        ],
        enabled,
    ) {
        next.tiles = v;
    }
    if let Some(v) = vec2_row(
        ui,
        rows,
        "projection.offset",
        lang.pick("オフセット", "Offset"),
        p.offset,
        &NumSpec::new(-1e4, 1e4, 0.005, 3),
        [
            lang.pick(
                "画像をずらす（画像の幅の単位）",
                "Shifts the image (in image widths)",
            ),
            lang.pick(
                "画像をずらす（画像の高さの単位）",
                "Shifts the image (in image heights)",
            ),
        ],
        enabled,
    ) {
        next.offset = v;
    }
    if let Some(v) = slider_row(
        ui,
        rows,
        "projection.rotation",
        lang.pick("回転", "Rotation"),
        p.rotation as f32,
        (-180.0, 180.0),
        NumberFormat {
            decimals: 1,
            trim: true,
            suffix: "°",
        },
        Some(lang.pick(
            "投影の正方形の中心のまわりに、画像を反時計回りに回す",
            "Turns the image counter-clockwise about the projected square's centre",
        )),
        enabled,
    ) {
        next.rotation = f64::from(v);
    }
    if p.mode == ProjectionMode::Triplanar {
        if let Some(v) = percent_row(
            ui,
            rows,
            "projection.blend",
            lang.pick("混ぜ幅", "Blend"),
            p.blend_width,
            (0.0, 1.0),
            Some(lang.pick(
                "面が箱の軸の間で向きを変える所で: 0%: いちばん向いている軸へ切り替え、100%: 法線に比例して混ぜる",
                "Where the surface turns between the box's axes: 0%: a hard switch to the axis it faces most; 100%: mixed in proportion to the normal",
            )),
            enabled,
        ) {
            next.blend_width = v;
        }
    }
    if p.mode == ProjectionMode::Decal {
        if let Some(v) = percent_row(
            ui,
            rows,
            "decal.depth",
            lang.pick("奥行きの縁", "Depth edge"),
            1.0 - p.depth_hardness,
            (0.0, 1.0),
            Some(lang.pick(
                "箱の手前・奥の面へ向かって、どれだけやわらかく消えるか（0%: ぱっと切る）",
                "How softly the decal fades towards the box's front and back faces (0%: a hard cut)",
            )),
            enabled,
        ) {
            next.depth_hardness = 1.0 - v;
        }
        if let Some(v) = slider_row(
            ui,
            rows,
            "decal.angle",
            lang.pick("裏向き", "Back faces"),
            p.backface_angle as f32,
            (0.0, 180.0),
            NumberFormat::int("°"),
            Some(lang.pick(
                "画像を見る側からこの角度より傾いた面は隠す（180°: 隠さない）",
                "Faces turned more than this from the side the image is seen from are hidden (180°: none)",
            )),
            enabled,
        ) {
            next.backface_angle = f64::from(v);
        }
        if let Some(v) = percent_row(
            ui,
            rows,
            "decal.angle.edge",
            lang.pick("面の縁", "Face edge"),
            1.0 - p.backface_hardness,
            (0.0, 1.0),
            Some(lang.pick(
                "その角度へ向かって、どれだけやわらかく消えるか（0%: ぱっと切る）",
                "How softly faces fade out towards that angle (0%: a hard cut)",
            )),
            enabled,
        ) {
            next.backface_hardness = 1.0 - v;
        }
    }
    if next != p {
        fill(
            app,
            FillOp::Projection {
                layer: id,
                projection: Box::new(next),
                coalesce: true,
            },
        );
    }
    // デカールの出ていない理由と、置き場
    if p.mode == ProjectionMode::Decal {
        if let Some(why) = problem(problems, lang, |t| *t == InactiveTarget::Decal) {
            warn_row(ui, rows, &why);
        }
    }
    if p.mode != ProjectionMode::Uv {
        placement_rows(ui, app, rows, id, &p, enabled, lang);
    } else {
        // 画像のチャンネルが投影できない理由（UV は画像がなければ何も出ない）
        for e in problems {
            if let InactiveTarget::FillImage(c) = e.target {
                let name = m2::channel_name(lang, &app.doc, c);
                warn_row(
                    ui,
                    rows,
                    &format!("{name}: {}", lang.inactive_reason(&e.reason)),
                );
                break;
            }
        }
    }
}

/// 投影の置き場: 3D ビューのハンドル（出す・隠す・移動・回転・モデルに合わせる）と、中心・回転・大きさ（モデルのルートの空間）。
fn placement_rows(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    id: LayerId,
    p: &Projection,
    enabled: bool,
    lang: Lang,
) {
    let editing =
        crate::fillfx::gizmo::target(app) == Some(crate::fillfx::gizmo::Target::Projection(id));
    let has_model = app.view3d.model.is_some();
    handle_buttons(
        ui,
        app,
        rows,
        "projection",
        editing,
        enabled,
        has_model,
        lang,
        |app| {
            fill(app, FillOp::ToggleHandles);
        },
        |app| {
            fill(app, FillOp::FitPlacement { layer: id });
        },
    );
    let v = p.placement;
    let mut next = v;
    if let Some(c) = vec3_row(
        ui,
        rows,
        "projection.center",
        lang.pick("中心", "Center"),
        v.center,
        &NumSpec::new(-1e6, 1e6, 0.01, 3),
        lang.pick(
            "箱の中心（モデルのルートから。シーンの単位）",
            "The box's centre, from the model root (scene units)",
        ),
        enabled,
    ) {
        next.center = c;
    }
    if let Some(r) = vec3_row(
        ui,
        rows,
        "projection.rotation3d",
        lang.pick("回転", "Rotation"),
        v.rotation,
        &NumSpec::new(-360.0, 360.0, 1.0, 1),
        lang.pick(
            "度のオイラー角（Z → X → Y の順に回す）",
            "Euler angles in degrees (turned about Z, then X, then Y)",
        ),
        enabled,
    ) {
        next.rotation = r.map(wrap_degrees);
    }
    if p.mode != ProjectionMode::Spherical {
        if let Some(s) = vec3_row(
            ui,
            rows,
            "projection.size",
            lang.pick("大きさ", "Size"),
            v.size,
            &NumSpec::new(1e-6, 1e6, 0.01, 3),
            size_tooltip(lang, p.mode),
            enabled,
        ) {
            next.size = s;
        }
    }
    if next != v {
        let mut projection = *p;
        projection.placement = next;
        fill(
            app,
            FillOp::Projection {
                layer: id,
                projection: Box::new(projection),
                coalesce: true,
            },
        );
    }
}

/// 3D ビューのハンドルの行: 出す・隠す（Q）、移動・回転の組、モデルに合わせる。
#[allow(clippy::too_many_arguments)]
fn handle_buttons(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    key: &str,
    editing: bool,
    enabled: bool,
    has_model: bool,
    lang: Lang,
    toggle: impl FnOnce(&mut AppState),
    fit: impl FnOnce(&mut AppState),
) {
    let row = rows.row(24.0, 4.0);
    let side = 26.0;
    let main = Rect::from_min_max(row.min, pos2(row.right() - side - 4.0, row.bottom()));
    let fit_rect = Rect::from_min_size(
        pos2(row.right() - side, row.top()),
        vec2(side, row.height()),
    );
    if w::button(
        ui,
        main,
        (key, "handles"),
        lang.pick("3D ビューのハンドル", "Handles in 3D View"),
        editing,
        enabled && has_model,
        Some(lang.pick(
            "3D ビューに箱とハンドルを出す・隠す（Q）",
            "Show or hide the box and its handles in the 3D view (Q)",
        )),
        Some("view_in_ar"),
    )
    .clicked()
    {
        toggle(app);
    }
    if w::icon_button(
        ui,
        fit_rect,
        (key, "fit"),
        "target",
        lang.pick("モデルに合わせる（外形）", "Fit to the model (its bounds)"),
        false,
        enabled && has_model,
        16.0,
    )
    .clicked()
    {
        fit(app);
    }
    let row = rows.row(24.0, 4.0);
    let cols = Rows::split(row, 2, 4.0);
    for (k, (mode, ja, en, tip_ja, tip_en)) in [
        (
            Mode::Move,
            "移動",
            "Move",
            "ハンドル: 移動（モデルの軸に沿った矢印、中心の四角はビューの面の中で）",
            "Handles: move (arrows along the model's axes, the square in the view's plane)",
        ),
        (
            Mode::Rotate,
            "回転",
            "Rotate",
            "ハンドル: 回転（モデルの軸のまわりの輪。Ctrl で 15° ずつ）",
            "Handles: rotate (rings about the model's axes; Ctrl for 15° steps)",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        if w::button(
            ui,
            cols[k],
            (key, "mode", k),
            lang.pick(ja, en),
            app.fillfx.gizmo_mode == mode,
            enabled && editing,
            Some(lang.pick(tip_ja, tip_en)),
            None,
        )
        .clicked()
        {
            fill(app, FillOp::GizmoMode(mode));
        }
    }
}

// ───────── グラデーションデカール ─────────

/// 形の名前。新規塗りつぶしレイヤーのメニュー・効果の欄と同じもの（`fx::names`）。
pub fn shape_name(lang: Lang, shape: Shape) -> &'static str {
    crate::fx::names::shape_name(lang, shape)
}

fn set_gradient(
    app: &mut AppState,
    layer: LayerId,
    channel: Channel,
    gradient: Settings,
    coalesce: bool,
) {
    fill(
        app,
        FillOp::Gradient {
            layer,
            channel,
            gradient: Some(Box::new(gradient)),
            coalesce,
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn gradient_section(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    id: LayerId,
    channel: Channel,
    problems: &[InactiveEffect],
    enabled: bool,
    lang: Lang,
) {
    let ctx = ui.ctx().clone();
    let (open_section, _) = section(
        ui,
        app,
        rows,
        "fill-gradient",
        lang.pick("グラデーションデカール", "Gradient Decal"),
        "palette",
        None,
    );
    if !open_section {
        return;
    }
    let Some(g) = app
        .doc
        .layer(id)
        .and_then(|l| l.fill_gradient(channel))
        .cloned()
    else {
        let r = rows.row(24.0, 4.0);
        if w::button(
            ui,
            r,
            "fill.gradient.add",
            lang.pick("グラデーションを追加", "Add Gradient"),
            false,
            enabled,
            Some(lang.pick(
                "3D ビューの箱・球・平面の範囲で、値を塗り分ける（モデルの上の位置で決まる）",
                "Paints a value over a box, sphere or plane in the 3D view (decided by the position on the model)",
            )),
            Some("add"),
        )
        .clicked()
        {
            fill(app, FillOp::AddGradient { layer: id, channel });
        }
        return;
    };
    // 取り除く
    let r = rows.row(24.0, 4.0);
    if w::button(
        ui,
        r,
        "fill.gradient.remove",
        lang.pick("グラデーションを外す", "Remove Gradient"),
        false,
        enabled,
        None,
        Some("delete"),
    )
    .clicked()
    {
        fill(
            app,
            FillOp::Gradient {
                layer: id,
                channel,
                gradient: None,
                coalesce: false,
            },
        );
        return;
    }
    let mut next = g.clone();
    // 形（説明は新規塗りつぶしレイヤーのメニューの項目と同じ文）
    let shape_tip = shape_tooltip(lang);
    let r = rows.row(t::ROW_HEIGHT, 4.0);
    let (response, anchor) = w::dropdown(
        ui,
        r,
        "fill.gradient.shape",
        Some(lang.pick("形", "Shape")),
        shape_name(lang, g.volume.shape),
        Some(shape_tip.as_str()),
        enabled,
        LABEL_W + 22.0,
    );
    if response.clicked() {
        open(app, &ctx, Popup::GradientShape(id, channel), anchor);
    }
    // 3D ビューで編集
    let editing = app.fillfx.edit_gradient == Some((id, channel))
        && crate::fillfx::gizmo::target(app)
            == Some(crate::fillfx::gizmo::Target::Gradient(id, channel));
    let has_model = app.view3d.model.is_some();
    let row = rows.row(24.0, 4.0);
    if w::button(
        ui,
        row,
        "fill.gradient.edit",
        lang.pick("3D ビューで編集", "Edit in 3D View"),
        editing,
        enabled && has_model,
        Some(lang.pick(
            "3D ビューに形とハンドルを出す",
            "Show the shape in the 3D view with handles",
        )),
        Some("view_in_ar"),
    )
    .clicked()
    {
        fill(
            app,
            FillOp::EditGradient(if editing { None } else { Some((id, channel)) }),
        );
    }
    let row = rows.row(24.0, 4.0);
    let cols = Rows::split(row, 2, 4.0);
    for (k, (mode, ja, en)) in [
        (Mode::Move, "移動", "Move"),
        (Mode::Rotate, "回転", "Rotate"),
    ]
    .into_iter()
    .enumerate()
    {
        if w::button(
            ui,
            cols[k],
            ("fill.gradient.mode", k),
            lang.pick(ja, en),
            app.fillfx.gizmo_mode == mode,
            enabled && editing,
            None,
            None,
        )
        .clicked()
        {
            fill(app, FillOp::GizmoMode(mode));
        }
    }
    // 置き場
    let mut v = g.volume;
    if let Some(c) = vec3_row(
        ui,
        rows,
        "fill.gradient.center",
        lang.pick("中心", "Center"),
        v.center,
        &NumSpec::new(-1e6, 1e6, 0.01, 3),
        lang.pick(
            "形の中心（モデルのルートから。シーンの単位）",
            "The shape's centre, from the model root (scene units)",
        ),
        enabled,
    ) {
        v.center = c;
    }
    if let Some(a) = vec3_row(
        ui,
        rows,
        "fill.gradient.rotation",
        lang.pick("回転", "Rotation"),
        v.rotation,
        &NumSpec::new(-360.0, 360.0, 1.0, 1),
        lang.pick(
            "度のオイラー角（Z → X → Y の順に回す）",
            "Euler angles in degrees (turned about Z, then X, then Y)",
        ),
        enabled,
    ) {
        v.rotation = a.map(wrap_degrees);
    }
    match v.shape {
        Shape::Box => {
            if let Some(s) = vec3_row(
                ui,
                rows,
                "fill.gradient.size",
                lang.pick("大きさ", "Size"),
                v.size,
                &NumSpec::new(1e-6, 1e6, 0.01, 3),
                lang.pick(
                    "箱の幅（それぞれの軸に沿って。シーンの単位）",
                    "The box's full width along each of its own axes (scene units)",
                ),
                enabled,
            ) {
                v.size = s;
            }
        }
        Shape::Sphere => {
            let row = rows.row(t::ROW_HEIGHT, 3.0);
            let out = number_field(
                ui,
                row,
                "fill.gradient.radius",
                lang.pick("半径", "Radius"),
                v.size[0] / 2.0,
                &NumSpec::new(0.5e-6, 0.5e6, 0.01, 3),
                Some(lang.pick(
                    "球の半径（シーンの単位）",
                    "The sphere's radius (scene units)",
                )),
                None,
                enabled,
            );
            if out.changed {
                v.size[0] = out.value * 2.0;
            }
        }
        Shape::Plane => {
            let row = rows.row(t::ROW_HEIGHT, 3.0);
            let out = number_field(
                ui,
                row,
                "fill.gradient.width",
                lang.pick("幅", "Width"),
                v.size[1],
                &NumSpec::new(1e-6, 1e6, 0.01, 3),
                Some(lang.pick(
                    "平面の後ろの 0 から前の 1 までにかかる距離（シーンの単位）。平面は自分の +Y を向く",
                    "How far the value takes to go from 0 behind the plane to 1 in front of it (scene units). The plane faces its own +Y",
                )),
                None,
                enabled,
            );
            if out.changed {
                v.size[1] = out.value;
            }
        }
    }
    if v.shape != Shape::Plane {
        if let Some(f) = percent_row(
            ui,
            rows,
            "fill.gradient.falloff",
            lang.pick("減衰", "Falloff"),
            v.falloff,
            (0.0, 1.0),
            Some(lang.pick(
                "境の内側がどれだけ 1 から 0 へ消えるか（0%: ぱっと切る、100%: 真ん中まで）",
                "How much of the inside fades from 1 to 0 at the boundary (0%: a hard edge; 100%: the fade reaches the middle)",
            )),
            enabled,
        ) {
            v.falloff = f;
        }
    }
    next.volume = v;
    if let Some(on) = toggle_row(
        ui,
        rows,
        "fill.gradient.invert",
        lang.pick("反転", "Invert"),
        g.invert,
        None,
        enabled,
    ) {
        next.invert = on;
    }
    // 色と値の階調
    let scalar = app
        .doc
        .channel_info(channel)
        .is_some_and(|c| c.kind == ChannelKind::Scalar);
    // ランプの変更が 1 回で決まるもの（編集の確定・色・消す）なら独立の 1 回の取り消し、スライダーのドラッグなら離すまで 1 回にまとめる
    let mut discrete = false;
    if let Some(ramp) = g.ramp.clone() {
        if let Some((new_ramp, once)) =
            ramp_rows(ui, app, rows, id, channel, &ramp, scalar, enabled, lang)
        {
            next.ramp = Some(new_ramp);
            discrete = once;
        }
    }
    if next != g {
        let coalesce = !discrete;
        set_gradient(app, id, channel, next, coalesce);
    }
    if let Some(why) = problem(problems, lang, |t| {
        *t == InactiveTarget::FillGradient(channel)
    }) {
        warn_row(ui, rows, &why);
    }
}

/// ランプの欄（Unity 版の `GradientRampRows` に、グラデーションセット・分岐点の色の選び・メインとサブに付いていく色を足したもの）。欄の中身は
/// グラデーションマップと共通（`ramp_rows`）で、ここは値のカーブを持つ（混色は Unity 版と共有の並びに持てないので出さない）。変更が決まったときだけ
/// 新しいランプを返す。
#[allow(clippy::too_many_arguments)]
fn ramp_rows(
    ui: &mut Ui,
    app: &mut AppState,
    rows: &mut Rows,
    id: LayerId,
    channel: Channel,
    ramp: &Ramp,
    scalar: bool,
    enabled: bool,
    lang: Lang,
) -> Option<(Ramp, bool)> {
    // 層・チャンネルごとに、分岐点の選びなどを別に覚える
    let key = (
        "fill-gradient",
        id.0 ^ (channel.index() as u128).wrapping_mul(0x9E37_79B9_7F4A_7C15),
    );
    let mut failure = None;
    let mut params = super::ramp_rows::Params {
        key,
        enabled,
        lang,
        main: app.color.main,
        sub: app.color.sub,
        scalar,
        features: super::ramp_rows::Features {
            mixing: false,
            value_curve: true,
        },
        sets: &mut app.ramp_sets,
        eyedrop: &mut app.eyedrop,
        failure: &mut failure,
    };
    let change = super::ramp_rows::rows(ui, rows, &mut params, ramp).map(|c| (c.ramp, c.discrete));
    if let Some(text) = failure {
        app.fail(crate::notice::Source::Gradient, text);
    }
    change
}

// ───────── ポップアップ ─────────

fn mode_entries(app: &AppState, layer: LayerId) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let current = app.doc.layer(layer).map(|l| l.projection().mode);
    let free = !app.is_stroking();
    MODES
        .iter()
        .map(|m| {
            Entry::item(
                projection_name(lang, *m),
                Action::Fill(FillOp::ProjectionMode { layer, mode: *m }),
            )
            .radio(current == Some(*m))
            .enabled(free)
        })
        .collect()
}

fn wrap_entries(app: &AppState, layer: LayerId) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let Some(p) = app.doc.layer(layer).map(|l| *l.projection()) else {
        return Vec::new();
    };
    WRAPS
        .iter()
        .map(|wrap| {
            Entry::item(
                wrap_name(lang, *wrap),
                Action::Fill(FillOp::Projection {
                    layer,
                    projection: Box::new(Projection { wrap: *wrap, ..p }),
                    coalesce: false,
                }),
            )
            .radio(p.wrap == *wrap)
            .enabled(!app.is_stroking())
        })
        .collect()
}

fn shape_entries(app: &AppState, layer: LayerId, channel: Channel) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let Some(g) = app
        .doc
        .layer(layer)
        .and_then(|l| l.fill_gradient(channel))
        .cloned()
    else {
        return Vec::new();
    };
    SHAPES
        .iter()
        .map(|shape| {
            let mut next = g.clone();
            next.volume.shape = *shape;
            Entry::item(
                shape_name(lang, *shape),
                Action::Fill(FillOp::Gradient {
                    layer,
                    channel,
                    gradient: Some(Box::new(next)),
                    coalesce: false,
                }),
            )
            .radio(g.volume.shape == *shape)
            .enabled(!app.is_stroking())
        })
        .collect()
}

/// 階調のプリセットの名前。
pub fn preset_name(lang: Lang, preset: Preset) -> &'static str {
    match preset {
        Preset::BlackWhite => lang.pick("黒から白", "Black to white"),
        Preset::WhiteBlack => lang.pick("白から黒", "White to black"),
        Preset::ForegroundBackground => lang.pick("メインからサブ", "Foreground to background"),
        Preset::ForegroundTransparent => lang.pick("メインから透明", "Foreground to transparent"),
        Preset::WarmCool => lang.pick("暖色から寒色", "Warm to cool"),
    }
}

const PRESETS: [Preset; 5] = [
    Preset::BlackWhite,
    Preset::WhiteBlack,
    Preset::ForegroundBackground,
    Preset::ForegroundTransparent,
    Preset::WarmCool,
];

fn ramp_preset_entries(app: &AppState, layer: LayerId, channel: Channel) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let Some(g) = app
        .doc
        .layer(layer)
        .and_then(|l| l.fill_gradient(channel))
        .cloned()
    else {
        return Vec::new();
    };
    let rgb = |c: crate::state::Rgba| {
        Rgba8::new(w::to_byte(c[0]), w::to_byte(c[1]), w::to_byte(c[2]), 255)
    };
    PRESETS
        .iter()
        .map(|preset| {
            // 階調だけを置き換える（値のカーブはプリセットの既定へ戻る。C# の `GradientRamp.Preset` と同じ）
            let mut next = g.clone();
            next.ramp = Some(Ramp::preset(
                *preset,
                rgb(app.color.main),
                rgb(app.color.sub),
            ));
            Entry::item(
                preset_name(lang, *preset),
                Action::Fill(FillOp::Gradient {
                    layer,
                    channel,
                    gradient: Some(Box::new(next)),
                    coalesce: false,
                }),
            )
            .enabled(!app.is_stroking())
        })
        .collect()
}

fn curve_preset_entries(app: &AppState, layer: LayerId, channel: Channel) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let Some(g) = app
        .doc
        .layer(layer)
        .and_then(|l| l.fill_gradient(channel))
        .cloned()
    else {
        return Vec::new();
    };
    let Some(ramp) = g.ramp.clone() else {
        return Vec::new();
    };
    let names = [
        lang.pick("線形", "Linear"),
        lang.pick("やわらかい", "Soft"),
        lang.pick("かたい", "Hard"),
        lang.pick("S 字", "S-curve"),
    ];
    (0..ramp_ops::CURVE_PRESETS.len())
        .filter_map(|k| {
            let next_ramp = ramp_ops::curve_preset(&ramp, k)?;
            let mut next = g.clone();
            next.ramp = Some(next_ramp);
            Some(
                Entry::item(
                    names[k],
                    Action::Fill(FillOp::Gradient {
                        layer,
                        channel,
                        gradient: Some(Box::new(next)),
                        coalesce: false,
                    }),
                )
                .enabled(!app.is_stroking()),
            )
        })
        .collect()
}

/// 塗りつぶしのポップアップの項目（`Popup::FillImage` など）。
pub fn entries(app: &AppState, popup: Popup) -> Vec<Entry<Action>> {
    match popup {
        Popup::FillImage(layer, channel) => image_entries(app, layer, channel),
        Popup::ImageSpace(image) => space_entries(app, image),
        Popup::ProjectionMode(layer) => mode_entries(app, layer),
        Popup::ProjectionWrap(layer) => wrap_entries(app, layer),
        Popup::GradientShape(layer, channel) => shape_entries(app, layer, channel),
        Popup::RampPresets(layer, channel) => ramp_preset_entries(app, layer, channel),
        Popup::CurvePresets(layer, channel) => curve_preset_entries(app, layer, channel),
        _ => Vec::new(),
    }
}
