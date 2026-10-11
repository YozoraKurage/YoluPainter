//! テキストの値の欄: 左のドックのツールプロパティ（テキストツール）・オプションバー・レイヤーのプロパティの「テキスト」の節。フォント（同梱・
//! インストール済み・ファイルから）・サイズ・色（色のウィンドウ）・行間・字間・揃え・折り返しの幅。当て先は打っている文字・選んでいるテキストレイヤー・
//! 次に作る文字の既定（`Target`）。テキストレイヤーのスライダーは離したとき 1 回で描き直す（動かしている間は値だけ）。色のウィンドウのドラッグは
//! 1 回の取り消しにまとめる。画面には名前と値だけを出し、説明はツールチップ。

use egui::{pos2, vec2, Rect, Ui};
use yolu_core::text::{TextAlign, TextFont, TextSettings};

use super::{font_label, ColorSource, Field, FontStatus, Pending, Target, TextAction};
use crate::lang::Lang;
use crate::m2_menu::Popup;
use crate::panels::color_window::{self, Pick};
use crate::panels::properties::{
    choice_buttons, group_label, open_popup, section, status_row, ChoiceButton,
};
use crate::state::{Action, AppState};
use crate::ui::menu::Entry;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows, SliderSpec};

const SIZE: &str = "text.size";
const LINE_HEIGHT: &str = "text.line_height";
const LETTER_SPACING: &str = "text.letter_spacing";
const WRAP: &str = "text.wrap";

fn set(app: &mut AppState, field: Field) {
    app.apply(Action::Text(TextAction::Set(field)));
}

/// 動かしている間の値があればそれ（同じ値のスライダーはどの欄も同じ値を見せる）、無ければ今の値。
fn shown(app: &AppState, key: &'static str, value: f64) -> f64 {
    let target = app.text_target();
    app.text
        .pending
        .filter(|p| p.key == key && p.target == target)
        .map_or(value, |p| p.value)
}

/// スライダー 1 つ。テキストレイヤー（`deferred`）は離したとき、ほかはすぐ、新しい値を返す。
/// 同じ `key` のスライダーが同じフレームに 2 つ以上出ているとき（テキストツールでテキストレイヤーを選ぶと、サイズはオプションバー・
/// ツールプロパティ・レイヤーのプロパティに出る）、動かしている値を当てる・捨てるのは動かしている欄（`owner`）だけ。
fn slider(
    ui: &mut Ui,
    app: &mut AppState,
    at: Rect,
    key: &'static str,
    spec: SliderSpec,
    value: f64,
    deferred: bool,
) -> Option<f64> {
    let current = if deferred {
        shown(app, key, value)
    } else {
        value
    };
    // `w::slider` が部品の id にする値と同じ（欄ごとに違う）
    let owner = ui.make_persistent_id(key);
    let target = app.text_target();
    let out = w::slider(ui, at, key, current as f32, &spec);
    if !deferred {
        return out.changed.then_some(out.value as f64);
    }
    let mine = app.text.pending.filter(|p| p.owner == owner);
    // 動かし始めたレイヤーのままで、同じ値のものだけ当てる
    let valid = |p: Pending| p.key == key && p.target == target;
    if out.released {
        // 離した（数値を打って決めたときも）
        if mine.is_some() {
            app.text.pending = None;
        }
        return if out.changed {
            Some(out.value as f64)
        } else {
            mine.filter(|p| valid(*p)).map(|p| p.value)
        };
    }
    if out.active {
        if out.changed {
            app.text.pending = Some(Pending {
                key,
                owner,
                target,
                value: out.value as f64,
            });
        }
        return None;
    }
    if out.changed {
        // Esc でドラッグを止めて押し始めの値へ戻した: 動かした値は捨てる
        if mine.is_some() {
            app.text.pending = None;
        }
        return None;
    }
    // 動かしていない。ドラッグを離したのに `released` が立たなかった（部品の外・右の空けた所で離した）ときは、動かした値を当てる
    let p = mine?;
    app.text.pending = None;
    valid(p).then_some(p.value)
}

pub fn align_name(lang: Lang, align: TextAlign) -> &'static str {
    match align {
        TextAlign::Left => lang.pick("左", "Left"),
        TextAlign::Center => lang.pick("中央", "Center"),
        TextAlign::Right => lang.pick("右", "Right"),
    }
}

/// 今のフォントが、OS の一覧のこのフォント（道と番号か、PostScript 名）か。
fn is_face(font: &TextFont, face: &yolu_io::fonts::SystemFace) -> bool {
    match font {
        TextFont::File {
            path, index, names, ..
        } => {
            (std::path::Path::new(path) == face.path && *index == face.index)
                || (!names.postscript.is_empty()
                    && names.postscript == face.description.names.postscript)
        }
        TextFont::Bundled(_) => false,
    }
}

/// フォントの選び（押すとポップアップ）: 同梱の 2 つ・インストール済みのフォント（ファミリーごと、スタイルは入れ子）・ファイルから。
pub fn font_entries(app: &AppState) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let current = app.text_value().font;
    let list = app.text.fonts.list.as_deref();
    let mut v: Vec<Entry<Action>> = yolu_core::text::BUNDLED_FONTS
        .iter()
        .map(|name| {
            let font = TextFont::Bundled((*name).into());
            Entry::item(
                font_label(lang, &font, list),
                Action::Text(TextAction::Set(Field::Font(font.clone()))),
            )
            .radio(current == font)
        })
        .collect();
    // 一覧に無いファイルのフォント（「ファイルから…」で選んだ・一覧をなめている間）は、今のフォントとして見せる
    if matches!(current, TextFont::File { .. })
        && list.is_none_or(|l| !l.faces().iter().any(|f| is_face(&current, f)))
    {
        v.push(
            Entry::item(
                font_label(lang, &current, list),
                Action::Text(TextAction::Set(Field::Font(current.clone()))),
            )
            .radio(true),
        );
    }
    v.push(Entry::Separator);
    let installed = lang.pick("インストール済みのフォント", "Installed Fonts");
    match list {
        None => v.push(
            Entry::submenu(installed, Vec::new())
                .enabled(false)
                .tooltip(lang.pick("読み込んでいます", "Loading")),
        ),
        Some(l) if l.is_empty() => v.push(
            Entry::submenu(installed, Vec::new())
                .enabled(false)
                .tooltip(lang.pick("見つかりません", "None found")),
        ),
        Some(l) => {
            let mut families: Vec<(String, Entry<Action>)> = l
                .families()
                .iter()
                .map(|family| {
                    let name = match (lang, &family.name_ja) {
                        (Lang::Ja, Some(ja)) => ja.clone(),
                        _ => family.name.clone(),
                    };
                    let item = |i: usize, label: String| {
                        let face = &l.faces()[i];
                        Entry::item(
                            label,
                            Action::Text(TextAction::SystemFont {
                                path: face.path.clone(),
                                index: face.index,
                            }),
                        )
                        .radio(is_face(&current, face))
                    };
                    let entry = match family.faces.as_slice() {
                        [only] => item(*only, name.clone()),
                        faces => Entry::submenu(
                            name.clone(),
                            faces
                                .iter()
                                .map(|&i| item(i, l.faces()[i].style()))
                                .collect(),
                        ),
                    };
                    (name, entry)
                })
                .collect();
            families.sort_by_key(|(name, _)| name.to_lowercase());
            v.push(Entry::submenu(
                installed,
                families.into_iter().map(|(_, e)| e).collect(),
            ));
        }
    }
    v.push(Entry::Separator);
    v.push(Entry::item(
        lang.pick("ファイルから…", "From File…"),
        Action::Text(TextAction::PickFontFile),
    ));
    v
}

/// フォントの選びを開く（OS のフォントの一覧をなめ始める）。
fn open_fonts(app: &mut AppState, ctx: &egui::Context, anchor: Rect) {
    app.text_fonts_wanted();
    open_popup(app, ctx, Popup::TextFont, anchor, 200.0);
}

/// 値を変えられない理由（フォントが無い・探している）と、違うフォントか。
fn font_state(app: &mut AppState) -> (Option<String>, bool) {
    let lang = app.lang;
    match app.text_target() {
        Target::Layer(id) => match app.text_font_status(id) {
            FontStatus::Ready => (None, false),
            FontStatus::Different => (None, true),
            FontStatus::Missing => (
                Some(
                    lang.pick("フォントが見つかりません", "Font not found")
                        .to_owned(),
                ),
                false,
            ),
            FontStatus::Searching => (
                Some(
                    lang.pick("フォントを探しています", "Searching for the font")
                        .to_owned(),
                ),
                false,
            ),
        },
        _ => (None, false),
    }
}

/// フォントの選びの行。狭い欄（ツールプロパティ）は `choice_row` と同じ 1 行の形で、名前と箱の間を `choice_row` より 4 点詰め、
/// 最小のウィンドウ（960 × 640）の既定のドックの幅でも、既定のフォントの名前を「…」で詰めずに出す（`choice_row` の間では 1.2 点足りない）。
/// レイヤーのプロパティは名前を上の行に置き、箱を行の幅いっぱいにする（名前の列の 92 点を箱に回し、頭の同じフォントの名前の後ろまで見せる）。
/// 箱に収まらない名前は「…」で詰め、ツールチップの頭に名前の全部を出す。
fn font_row(
    ui: &mut Ui,
    rows: &mut Rows,
    label: &str,
    value: &str,
    tooltip: &str,
    enabled: bool,
) -> Option<Rect> {
    let (r, label_width, inline) = if rows.compact {
        let r = rows.row(t::ROW_HEIGHT, 4.0);
        let width = (w::text_width(ui.painter(), label, t::LABEL) + 6.0).min(r.width() * 0.6);
        (r, width, Some(label))
    } else {
        // 名前の行（2 行のスライダーの名前と同じ位置・高さ）
        let head = rows.row(w::TWO_LINE_LABEL_HEIGHT, 2.0);
        w::text(
            ui.painter(),
            Rect::from_min_max(pos2(head.left() + 1.0, head.top()), head.max),
            label,
            t::LABEL.with_color(w::label_color(ui, "text.font.label", enabled)),
            w::Align::Left,
        );
        (rows.row(t::ROW_HEIGHT, 4.0), 0.0, None)
    };
    // 値の箱の中の文字の幅（`w::dropdown` の左の余白と右の印を除いた幅）
    let room = r.width() - label_width - 26.0;
    let full;
    let tooltip = if w::text_width(ui.painter(), value, t::LABEL) > room {
        full = format!("{value}\n{tooltip}");
        full.as_str()
    } else {
        tooltip
    };
    let (response, b) = w::dropdown(
        ui,
        r,
        "text.font",
        inline,
        value,
        Some(tooltip),
        enabled,
        label_width,
    );
    response.clicked().then_some(b)
}

/// テキストの値の欄の行（ツールプロパティとレイヤーのプロパティで同じ。色だけ違う: `tool_props` ならツールの設定として、色の元（描画色・ツールの色）の
/// 選びと、その色を見せる。レイヤーのプロパティはそのレイヤーの色を見せ、変えるとそのレイヤーだけが変わる）。
pub fn fields(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, enabled: bool, tool_props: bool) {
    let lang = app.lang;
    let target = app.text_target();
    let deferred = matches!(target, Target::Layer(_));
    // フォントが見つからないテキストレイヤー: 値は見せるが変えられない（理由は短い状態）。違うフォントは状態だけ
    let (problem, different) = font_state(app);
    let enabled = enabled && problem.is_none();
    if let Some(reason) = &problem {
        status_row(ui, rows, reason);
    } else if different {
        status_row(ui, rows, lang.pick("フォントが違います", "Font differs"));
    }
    let value: TextSettings = app.text_value();
    let ctx = ui.ctx().clone();
    let label = font_label(lang, &value.font, app.text.fonts.list.as_deref());
    if let Some(anchor) = font_row(
        ui,
        rows,
        lang.pick("フォント", "Font"),
        &label,
        lang.pick(
            "同梱のフォント・インストール済みのフォント・フォントのファイル（.ttf・.otf・.ttc）。フォントは .ylp に入らず、道と名前と中身の印だけを覚える",
            "A bundled font, an installed font or a font file (.ttf, .otf, .ttc). The font is not stored in the .ylp; only its path, names and a fingerprint are kept",
        ),
        enabled,
    ) {
        open_fonts(app, &ctx, anchor);
    }
    let at = rows.slider_row();
    let spec = SliderSpec::new(
        lang.pick("サイズ", "Size"),
        1.0,
        512.0,
        NumberFormat::int(" px"),
    )
    .power(2.0)
    .tooltip(lang.pick("1 文字の高さ（画素）", "Height of one em (pixels)"))
    .enabled(enabled);
    if let Some(v) = slider(ui, app, at, SIZE, spec, value.size, deferred) {
        set(app, Field::Size(v.round().max(1.0)));
    }
    // 色（押すと色のウィンドウ。ウィンドウのドラッグ 1 回を 1 回の取り消しにまとめる）
    if tool_props {
        color_source_rows(ui, app, rows);
    }
    let row = rows.row(t::ROW_HEIGHT, 3.0);
    let label = Rect::from_min_size(row.min, vec2(80.0, row.height()));
    let name = lang.pick("色", "Color");
    // 描いている間は、描き始める前の見た目のまま（`w::label_color`）
    let color = w::label_color(ui, "text.color.label", enabled);
    w::text(
        ui.painter(),
        label,
        name,
        t::LABEL.with_color(color),
        w::Align::Left,
    );
    let swatch = Rect::from_min_max(
        pos2(row.left() + 84.0, row.top() + 1.0),
        pos2(row.right(), row.bottom() - 1.0),
    );
    // ツールプロパティの色: 描画色なら描画色そのもの（変えるのは描画色。選んでいるテキストへは `text_follow_paint_color` が当てる）、
    // ツールの色ならそのツールの色。レイヤーのプロパティはそのレイヤーの色
    let source = tool_props.then_some(app.text.color_source);
    let c = match source {
        Some(ColorSource::PaintColor) => crate::matpaint::single_value(app.color.main),
        Some(ColorSource::ToolColor) => app.text.defaults.color,
        None => value.color,
    };
    let window_target = match (source, target) {
        (Some(ColorSource::PaintColor), _) => egui::Id::new("text.color.paint"),
        (Some(ColorSource::ToolColor), _) => egui::Id::new("text.color.tool"),
        (None, Target::Layer(id)) => egui::Id::new(("text.color", id.0)),
        (None, Target::Editing) => egui::Id::new(("text.color.editing", app.doc.id())),
        (None, Target::Defaults) => egui::Id::new("text.color.defaults"),
    };
    let tip = match source {
        Some(ColorSource::PaintColor) => lang.pick("描画色", "Paint Color"),
        Some(ColorSource::ToolColor) => lang.pick("ツールの色", "Tool Color"),
        None => lang.pick("文字の色と不透明度", "Text color and opacity"),
    };
    if let Some(u) = color_window::field(
        ui,
        swatch,
        window_target,
        name,
        Pick {
            rgb: [c.r, c.g, c.b],
            alpha: Some(c.a),
        },
        tip,
        enabled,
    ) {
        let [r, g, b] = u.pick.rgb;
        let next = yolu_core::Rgba8::new(r, g, b, u.pick.alpha.unwrap_or(c.a));
        if next != c {
            match source {
                Some(ColorSource::PaintColor) => app.color.set_main(u.pick.floats()),
                Some(ColorSource::ToolColor) => app.apply(Action::Text(TextAction::ToolColor {
                    color: next,
                    dragging: u.dragging,
                })),
                None => app.apply(Action::Text(if u.dragging {
                    TextAction::Drag(Field::Color(next))
                } else {
                    TextAction::Set(Field::Color(next))
                })),
            }
        }
        if u.done {
            app.m2_end_drag();
        }
    }
    let at = rows.slider_row();
    let spec = SliderSpec::new(
        lang.pick("行間", "Line Spacing"),
        0.5,
        3.0,
        NumberFormat {
            decimals: 2,
            trim: true,
            suffix: "",
        },
    )
    .tooltip(lang.pick(
        "行の送り（サイズに掛ける）",
        "Line advance (times the size)",
    ))
    .enabled(enabled);
    if let Some(v) = slider(ui, app, at, LINE_HEIGHT, spec, value.line_height, deferred) {
        set(app, Field::LineHeight(v));
    }
    let at = rows.slider_row();
    let spec = SliderSpec::new(
        lang.pick("字間", "Tracking"),
        -0.5,
        1.0,
        NumberFormat {
            decimals: 2,
            trim: true,
            suffix: "",
        },
    )
    .tooltip(lang.pick(
        "字ごとに加える送り（サイズに掛ける。負は詰める）",
        "Extra advance per character (times the size; negative tightens)",
    ))
    .enabled(enabled);
    if let Some(v) = slider(
        ui,
        app,
        at,
        LETTER_SPACING,
        spec,
        value.letter_spacing,
        deferred,
    ) {
        set(app, Field::LetterSpacing(v));
    }
    // 揃え
    let names = [TextAlign::Left, TextAlign::Center, TextAlign::Right];
    let items: Vec<ChoiceButton> = names
        .iter()
        .map(|a| ChoiceButton {
            id: match a {
                TextAlign::Left => "text.align.left",
                TextAlign::Center => "text.align.center",
                TextAlign::Right => "text.align.right",
            },
            label: align_name(lang, *a),
            selected: value.align == *a,
            enabled,
            tooltip: Some(lang.pick("行の揃え", "Line alignment")),
        })
        .collect();
    if let Some(i) = choice_buttons(ui, rows, &items) {
        set(app, Field::Align(names[i]));
    }
    let at = rows.slider_row();
    let width = app.doc.width().max(1) as f32;
    let spec = SliderSpec::new(
        lang.pick("折り返しの幅", "Wrap Width"),
        0.0,
        width,
        NumberFormat::int(" px"),
    )
    .tooltip(lang.pick(
        "この幅で行を折り返す（0 は折り返さない）",
        "Lines wrap at this width (0 does not wrap)",
    ))
    .enabled(enabled);
    if let Some(v) = slider(ui, app, at, WRAP, spec, value.wrap_width, deferred) {
        set(app, Field::WrapWidth(v.round()));
    }
}

/// 「テキストの色」の見出しと、色の元（描画色・ツールの色）の選び。
fn color_source_rows(ui: &mut Ui, app: &mut AppState, rows: &mut Rows) {
    let lang = app.lang;
    group_label(ui, rows, lang.pick("テキストの色", "Text Color"));
    let sources = [ColorSource::PaintColor, ColorSource::ToolColor];
    let items: Vec<ChoiceButton> = sources
        .iter()
        .map(|source| ChoiceButton {
            id: match source {
                ColorSource::PaintColor => "text.color.source.paint",
                ColorSource::ToolColor => "text.color.source.tool",
            },
            label: match source {
                ColorSource::PaintColor => lang.pick("描画色", "Paint Color"),
                ColorSource::ToolColor => lang.pick("ツールの色", "Tool Color"),
            },
            selected: app.text.color_source == *source,
            // 色の元は、ツールの設定（文書を変えない）なので、描いている間も替えられる
            enabled: true,
            tooltip: Some(lang.pick("テキストの色", "Text Color")),
        })
        .collect();
    if let Some(i) = choice_buttons(ui, rows, &items) {
        app.apply(Action::Text(TextAction::ColorSource(sources[i])));
    }
}

/// ツールプロパティ（左のドック）: テキストの値の欄。
pub fn props(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, _ctx: &egui::Context) {
    let enabled = app.can_edit();
    fields(ui, app, rows, enabled, true);
}

/// レイヤーのプロパティの「テキスト」の節（テキストレイヤーを選んでいるとき）。
pub fn layer_section(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, enabled: bool) {
    let lang = app.lang;
    let (open, _) = section(
        ui,
        app,
        rows,
        "text",
        lang.pick("テキスト", "Text"),
        "tools/text",
        None,
    );
    if open {
        fields(ui, app, rows, enabled, false);
    }
}

/// オプションバー: フォント・サイズ。
pub fn options(ui: &mut Ui, app: &mut AppState, r: Rect, x: f32) {
    let lang = app.lang;
    let x = x + 4.0;
    let (y, h) = (r.top() + 6.0, r.height() - 12.0);
    let value = app.text_value();
    let deferred = matches!(app.text_target(), Target::Layer(_));
    let enabled = app.can_edit() && font_state(app).0.is_none();
    let label = font_label(lang, &value.font, app.text.fonts.list.as_deref());
    let width = (w::text_width(ui.painter(), &label, t::LABEL) + 34.0).clamp(120.0, 240.0);
    let at = Rect::from_min_size(pos2(x, y), vec2(width, h));
    let (response, anchor) = w::dropdown(
        ui,
        at,
        "text.options.font",
        None,
        &label,
        Some(lang.pick("フォント", "Font")),
        enabled,
        0.0,
    );
    if response.clicked() {
        let ctx = ui.ctx().clone();
        open_fonts(app, &ctx, anchor);
    }
    let at = Rect::from_min_size(pos2(x + width + 10.0, y), vec2(150.0, h));
    let spec = SliderSpec::new(
        lang.pick("サイズ", "Size"),
        1.0,
        512.0,
        NumberFormat::int(" px"),
    )
    .power(2.0)
    .enabled(enabled);
    if let Some(v) = slider(ui, app, at, SIZE, spec, value.size, deferred) {
        set(app, Field::Size(v.round().max(1.0)));
    }
}
