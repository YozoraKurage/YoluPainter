//! 部品の振る舞いと見た目（Unity 版の PaintGui の部品を写したもの）。部品だけを並べた見本のウィンドウで確かめる。
use crate::common;

use egui::{pos2, vec2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::ui::menu::{self, Entry, PopupOutcome, PopupState};
use yolu_app::ui::widgets::{self as w, NumberFormat, SliderSpec};
use yolu_app::YoluApp;

#[derive(Default)]
struct Gallery {
    ready: bool,
    origin: Pos2,
    size: f32,
    flow: f32,
    opacity: f32,
    released: usize,
    open: bool,
    sub_open: bool,
    reset: usize,
    tab: usize,
    toggle: bool,
    pen: bool,
    popup: Option<PopupState>,
    blend: &'static str,
    text: String,
}

const BLENDS: [&str; 4] = ["通常", "乗算", "スクリーン", "オーバーレイ"];

fn row(g: &Gallery, y: f32, h: f32) -> Rect {
    Rect::from_min_size(pos2(g.origin.x + 8.0, g.origin.y + y), vec2(380.0, h))
}

fn draw(ui: &mut egui::Ui, g: &mut Gallery) {
    if !g.ready {
        // 書体は次のフレームから効く（このフレームで太字の書体を使うと egui が止まる）ので、初めのフレームは準備だけ
        YoluApp::setup(ui.ctx());
        g.ready = true;
        ui.ctx().request_repaint();
        return;
    }
    let area = Rect::from_min_size(ui.max_rect().min, vec2(396.0, 500.0));
    ui.allocate_rect(area, egui::Sense::hover());
    g.origin = area.min;
    let p = ui.painter().clone();
    p.rect_filled(area, 0.0, yolu_app::ui::theme::PANEL_BG);
    w::panel_title(
        &p,
        Rect::from_min_size(area.min, vec2(396.0, 24.0)),
        "パネルの見出し",
        Some("layers"),
    );
    g.tab = w::tab_strip(
        ui,
        Rect::from_min_size(pos2(area.left(), area.top() + 26.0), vec2(396.0, 30.0)),
        "tabs",
        &["ブラシ", "アルファ", "ステンシル", "マテリアル"],
        &["paint_brush", "shapes", "square", "layers"],
        g.tab,
    );
    let out = w::section_header(
        ui,
        Rect::from_min_size(pos2(area.left(), area.top() + 60.0), vec2(396.0, 24.0)),
        "section",
        "ブラシ",
        g.open,
        Some("paint_brush"),
        Some("既定に戻す"),
    );
    g.open = out.open;
    if out.reset {
        g.reset += 1;
    }
    g.sub_open = w::subsection_header(ui, row(g, 90.0, 20.0), "sub", "ジッター", g.sub_open);
    // 2 行のスライダー（右にペンのボタン）
    let r = row(g, 114.0, 34.0);
    let out = w::slider(
        ui,
        r,
        "size",
        g.size,
        &SliderSpec::new("直径", 1.0, 256.0, NumberFormat::int(" px")).inset(28.0),
    );
    if out.changed {
        g.size = out.value;
    }
    if out.released {
        g.released += 1;
    }
    if w::icon_button(
        ui,
        Rect::from_min_size(pos2(r.right() - 24.0, r.bottom() - 20.0), vec2(24.0, 20.0)),
        "pen",
        "stylus",
        "筆圧で直径を変える",
        g.pen,
        true,
        15.0,
    )
    .clicked()
    {
        g.pen = !g.pen;
    }
    let out = w::slider(
        ui,
        row(g, 152.0, 34.0),
        "flow",
        g.flow,
        &SliderSpec::new("流量", 0.0, 100.0, NumberFormat::int("%")).enabled(false),
    );
    if out.changed {
        g.flow = out.value;
    }
    // 1 行のスライダー（0 をまたぐ範囲は 0 から塗る）
    let out = w::slider(
        ui,
        row(g, 192.0, 24.0),
        "opacity",
        g.opacity,
        &SliderSpec::new(
            "角度",
            -180.0,
            180.0,
            NumberFormat {
                decimals: 1,
                trim: true,
                suffix: "°",
            },
        ),
    );
    if out.changed {
        g.opacity = out.value;
    }
    g.toggle = w::toggle(
        ui,
        row(g, 222.0, 22.0),
        "toggle",
        "隣り合う所だけ",
        g.toggle,
        None,
        true,
    );
    let (response, b) = w::dropdown(
        ui,
        row(g, 250.0, 24.0),
        "blend",
        Some("合成"),
        g.blend,
        None,
        true,
        60.0,
    );
    if response.clicked() {
        g.popup = Some(PopupState::new(ui.ctx(), b).with_min_width(b.width()));
    }
    let r = row(g, 282.0, 26.0);
    let cells = w::Rows::split(r, 3, 8.0);
    let _ = w::button(ui, cells[0], "ok", "OK", true, true, None, None);
    let _ = w::button(ui, cells[1], "cancel", "取消", false, true, None, None);
    let _ = w::button(
        ui,
        cells[2],
        "save",
        "保存",
        false,
        false,
        None,
        Some("check"),
    );
    let r = row(g, 316.0, 26.0);
    for (i, (icon, selected, enabled)) in [
        ("add", false, true),
        ("paint_brush", true, true),
        ("delete", false, false),
    ]
    .iter()
    .enumerate()
    {
        let _ = w::icon_button(
            ui,
            Rect::from_min_size(pos2(r.left() + i as f32 * 30.0, r.top()), vec2(26.0, 24.0)),
            ("icon", i),
            icon,
            icon,
            *selected,
            *enabled,
            18.0,
        );
    }
    let _ = w::color_swatch(
        ui,
        Rect::from_min_size(pos2(r.left() + 110.0, r.top()), vec2(36.0, 22.0)),
        "swatch",
        [0.2, 0.5, 0.9, 0.5],
        "色の見本",
        true,
    );
    w::checker(
        ui.painter(),
        Rect::from_min_size(pos2(r.left() + 160.0, r.top()), vec2(48.0, 24.0)),
        4.0,
    );
    if let Some(text) =
        w::text_field(ui, row(g, 350.0, 22.0), "text", &g.text, None, false).committed
    {
        g.text = text;
    }
    if let Some(state) = &mut g.popup {
        let entries: Vec<Entry<&'static str>> = BLENDS
            .iter()
            .map(|b| Entry::item(*b, *b).radio(*b == g.blend))
            .collect();
        match menu::show(
            ui.ctx(),
            egui::Id::new("gallery.popup"),
            state,
            &entries,
            &[],
        ) {
            PopupOutcome::Chosen(b) => {
                g.blend = b;
                g.popup = None;
            }
            PopupOutcome::Close | PopupOutcome::Step(_) => g.popup = None,
            PopupOutcome::Open => {}
        }
    }
}

fn gallery() -> Harness<'static, Gallery> {
    let state = Gallery {
        size: 32.0,
        flow: 40.0,
        opacity: 30.0,
        open: true,
        blend: "通常",
        text: "レイヤー 1".into(),
        ..Default::default()
    };
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(420.0, 520.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .renderer(common::shared_gpu::renderer())
        .build_ui_state(draw, state);
    h.run();
    h
}

fn click(h: &mut Harness<'_, Gallery>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.event(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run();
}

fn key(h: &Harness<'_, Gallery>, key: Key, modifiers: Modifiers) {
    h.event(Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    });
    h.event(Event::Key {
        key,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers,
    });
}

#[test]
fn gallery_snapshot() {
    let mut h = gallery();
    h.snapshot("widgets_gallery");
}

#[test]
fn two_line_slider_clicks_drags_and_types() {
    let mut h = gallery();
    let r = row(h.state(), 114.0, 34.0);
    let track_w = r.width() - 28.0;
    // 押した位置の値（溝の 3/4）
    click(&mut h, pos2(r.left() + track_w * 0.75, r.bottom() - 6.0));
    assert!(
        (h.state().size - (1.0 + 255.0 * 0.75)).abs() < 0.01,
        "{}",
        h.state().size
    );
    assert_eq!(h.state().released, 1);
    // ドラッグで左端へ
    let start = pos2(r.left() + track_w * 0.5, r.bottom() - 6.0);
    h.event(Event::PointerMoved(start));
    h.event(Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    for x in [0.3, 0.1, -0.2] {
        h.event(Event::PointerMoved(pos2(
            r.left() + track_w * x,
            r.bottom() - 6.0,
        )));
        h.step();
    }
    h.event(Event::PointerButton {
        pos: pos2(r.left() - 20.0, r.bottom() - 6.0),
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run();
    assert_eq!(h.state().size, 1.0);
    // 値の箱を押すと数値を打てる（Enter で決める）
    click(&mut h, pos2(r.right() - 10.0, r.top() + 7.0));
    key(&h, Key::A, Modifiers::COMMAND);
    h.event(Event::Text("42".into()));
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().size, 42.0);
    // 範囲の外は切り詰める・Esc はやめる
    click(&mut h, pos2(r.right() - 10.0, r.top() + 7.0));
    key(&h, Key::A, Modifiers::COMMAND);
    h.event(Event::Text("999".into()));
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().size, 42.0);
    click(&mut h, pos2(r.right() - 10.0, r.top() + 7.0));
    key(&h, Key::A, Modifiers::COMMAND);
    h.event(Event::Text("999".into()));
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().size, 256.0);
    // 2 行目の右のペンのボタンはスライダーにならない
    click(&mut h, pos2(r.right() - 12.0, r.bottom() - 8.0));
    assert!(h.state().pen);
    assert_eq!(h.state().size, 256.0);
}

#[test]
fn disabled_slider_does_not_move() {
    let mut h = gallery();
    let r = row(h.state(), 152.0, 34.0);
    click(&mut h, pos2(r.left() + 10.0, r.bottom() - 6.0));
    assert_eq!(h.state().flow, 40.0);
}

#[test]
fn one_line_slider_double_click_types_a_number() {
    let mut h = gallery();
    let r = row(h.state(), 192.0, 24.0);
    click(&mut h, r.center());
    assert!(
        h.state().opacity.abs() < 0.5,
        "中央は 0°: {}",
        h.state().opacity
    );
    for _ in 0..2 {
        h.event(Event::PointerButton {
            pos: r.center(),
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        h.event(Event::PointerButton {
            pos: r.center(),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.run();
    key(&h, Key::A, Modifiers::COMMAND);
    h.event(Event::Text("-12.5".into()));
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().opacity, -12.5);
}

#[test]
fn headers_tabs_toggle_and_reset() {
    let mut h = gallery();
    h.get_by_label("アルファ").click();
    h.run();
    assert_eq!(h.state().tab, 1);
    h.get_by_label("既定に戻す").click();
    h.run();
    assert_eq!(h.state().reset, 1);
    assert!(h.state().open, "既定に戻すのボタンは見出しを畳まない");
    let header = h
        .get_all_by_label("ブラシ")
        .map(|n| n.rect())
        .find(|r| r.top() > 50.0)
        .unwrap(); // タブと同じ名前の見出し
    click(&mut h, header.center());
    assert!(!h.state().open);
    h.get_by_label("ジッター").click();
    h.run();
    assert!(h.state().sub_open);
    h.get_by_label("隣り合う所だけ").click();
    h.run();
    assert!(h.state().toggle);
}

#[test]
fn dropdown_opens_own_menu_and_picks() {
    let mut h = gallery();
    h.get_by_label("合成: 通常").click();
    h.run();
    assert!(h.state().popup.is_some());
    h.snapshot("widgets_dropdown_open");
    // キーで選ぶ（↓ ↓ Enter = 2 つ目の「乗算」。最初の ↓ で先頭の行）
    key(&h, Key::ArrowDown, Modifiers::NONE);
    key(&h, Key::ArrowDown, Modifiers::NONE);
    h.run();
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().blend, "乗算");
    assert!(h.state().popup.is_none());
    // 押して選ぶ
    h.get_by_label("合成: 乗算").click();
    h.run();
    let body = h.state().popup.as_ref().unwrap().rect;
    let item = h
        .get_all_by_label("スクリーン")
        .map(|n| n.rect())
        .find(|r| body.contains(r.center()))
        .unwrap();
    click(&mut h, item.center());
    assert_eq!(h.state().blend, "スクリーン");
    // 外を押すと閉じる（選ばない）
    h.get_by_label("合成: スクリーン").click();
    h.run();
    click(&mut h, pos2(5.0, 515.0));
    assert!(h.state().popup.is_none());
    assert_eq!(h.state().blend, "スクリーン");
}

#[test]
fn text_field_commits_on_enter_and_cancels_on_escape() {
    let mut h = gallery();
    let r = row(h.state(), 350.0, 22.0);
    click(&mut h, r.center());
    key(&h, Key::A, Modifiers::COMMAND);
    h.event(Event::Text("背景".into()));
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().text, "レイヤー 1");
    click(&mut h, r.center());
    key(&h, Key::A, Modifiers::COMMAND);
    h.event(Event::Text("背景".into()));
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert_eq!(h.state().text, "背景");
}

#[test]
fn number_format_matches_unity() {
    assert_eq!(NumberFormat::int("%").format(2.5), "3%"); // 0.5 は 0 から離れる向き（C# と同じ）
    assert_eq!(NumberFormat::int(" px").format(-0.4), "0 px");
    let f = NumberFormat {
        decimals: 2,
        trim: true,
        suffix: "",
    };
    assert_eq!(f.format(1.5), "1.5");
    assert_eq!(f.format(2.0), "2");
    assert_eq!(f.format(0.125), "0.13");
}
