//! 選びとメニュー（選んでいる値の箱・タブの帯・メニューバー・ポップアップのメニュー）。

use egui::{pos2, vec2, Rect, Ui};
use yolu_app::ui::menu::{self, Entry as MenuEntry, PopupOutcome, PopupState};
use yolu_app::ui::widgets as w;

use super::{remember, rows, store, Entry, Section};
use crate::page::SketchCtx;

pub fn section() -> Section {
    Section {
        title: ("選びとメニュー", "Pickers and menus"),
        entries: vec![
            Entry::new(
                "widgets::dropdown",
                ("選んでいる値を出す箱", "Box showing the chosen value"),
                92.0,
                dropdowns,
            ),
            Entry::new(
                "widgets::tab_strip",
                ("アイコンと名前のタブの帯", "Tab strip with icons and names"),
                84.0,
                tab_strips,
            ),
            Entry::new(
                "menu::menu_bar",
                ("メニューバーの見出し", "Menu bar titles"),
                44.0,
                menu_bar,
            ),
            Entry::new(
                "menu::show・menu::Entry",
                ("ポップアップのメニュー", "Popup menu"),
                318.0,
                popup_menu,
            ),
        ],
    }
}

fn dropdowns(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let mut rows = rows(sample);
    rows.space(8.0);
    let blend = ctx.pick("通常", "Normal");
    let r = rows.row(24.0, 6.0);
    let _ = w::dropdown(ui, r, "plain", None, blend, None, true, 0.0);
    let r = rows.row(24.0, 6.0);
    let _ = w::dropdown(
        ui,
        r,
        "labeled",
        Some(ctx.pick("合成方法", "Blend mode")),
        blend,
        None,
        true,
        0.0,
    );
    let r = rows.row(24.0, 0.0);
    let _ = w::dropdown(
        ui,
        r,
        "off",
        None,
        ctx.pick("押せない", "Disabled"),
        None,
        false,
        0.0,
    );
}

fn tab_strips(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let labels = [
        ctx.pick("レイヤー", "Layer"),
        ctx.pick("マスク", "Mask"),
        ctx.pick("効果", "Effects"),
    ];
    let icons = ["layers", "vignette", "auto_awesome"];
    let active = ctx.values.index("gallery.tab-strip", 0);
    let wide = Rect::from_min_size(
        pos2(sample.left(), sample.top() + 6.0),
        vec2(sample.width(), 30.0),
    );
    let next = w::tab_strip(ui, wide, "wide", &labels, &icons, active);
    ctx.values.set_index("gallery.tab-strip", next);
    // 狭い幅（名前だけ）
    let narrow = Rect::from_min_size(
        pos2(sample.left(), sample.top() + 44.0),
        vec2(sample.width() * 0.62, 30.0),
    );
    let next = w::tab_strip(ui, narrow, "narrow", &labels, &icons, next);
    ctx.values.set_index("gallery.tab-strip", next);
}

fn menu_bar(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let titles = [
        ctx.pick("ファイル", "File"),
        ctx.pick("編集", "Edit"),
        ctx.pick("レイヤー", "Layer"),
        ctx.pick("表示", "View"),
    ];
    let r = Rect::from_min_size(
        pos2(sample.left(), sample.top() + 10.0),
        vec2(sample.width(), 24.0),
    );
    let _ = menu::menu_bar(ui, r, &titles, None);
}

fn popup_menu(ui: &mut Ui, sample: Rect, ctx: &mut SketchCtx) {
    let lang = ctx.lang;
    let button = Rect::from_min_size(
        pos2(sample.left() + 10.0, sample.top() + 10.0),
        vec2(150.0, 24.0),
    );
    let pressed = w::button(
        ui,
        button,
        "open",
        lang.pick("メニューを開く", "Open the menu"),
        false,
        true,
        None,
        Some("arrow_drop_down"),
    )
    .clicked();
    let entries: Vec<MenuEntry<()>> = vec![
        MenuEntry::Heading(lang.pick("合成方法", "Blend mode").to_owned()),
        MenuEntry::item(lang.pick("通常", "Normal"), ()).radio(true),
        MenuEntry::item(lang.pick("乗算", "Multiply"), ()).radio(false),
        MenuEntry::item(lang.pick("スクリーン", "Screen"), ()).radio(false),
        MenuEntry::Separator,
        MenuEntry::item(lang.pick("グリッドを表示", "Show grid"), ()).checked(true),
        MenuEntry::item(lang.pick("フォルダーを開く", "Open folder"), ())
            .icon("folder_open", false),
        MenuEntry::item(lang.pick("押せない項目", "Disabled item"), ()).enabled(false),
        MenuEntry::submenu(
            lang.pick("フィルター", "Filter"),
            vec![
                MenuEntry::item(lang.pick("ぼかし", "Blur"), ()),
                MenuEntry::item(lang.pick("シャープ", "Sharpen"), ()),
            ],
        ),
    ];
    // 撮る間は、最初のフレームで開いた形にする（ウィンドウではボタンで開く）
    let first = !remember(ui, "opened-once", || false);
    store(ui, "opened-once", true);
    let id = ui.id().with("popup");
    let mut state: Option<PopupState> = ui.data(|d| d.get_temp(id));
    if pressed || (ctx.still && first) {
        state = Some(PopupState::new(ui.ctx(), button));
    }
    if let Some(mut open) = state {
        let outcome = menu::show(ui.ctx(), id, &mut open, &entries, &[button]);
        if matches!(outcome, PopupOutcome::Open) || ctx.still {
            ui.data_mut(|d| d.insert_temp(id, open));
        } else {
            ui.data_mut(|d| d.remove::<PopupState>(id));
        }
    }
}
