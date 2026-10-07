//! Live Link の入口（メニューバーの右端、プロジェクトの名前の左の Unity の印）: 受け付けていないは灰・受付中は薄い色・相手の文書は緑・
//! 合わない物があれば警告の色・フォルダを使えなければ赤。押すと小さなウィンドウ（「Live Link: 状態」・「Unity: 名前」と、受け付ける／受け付けない）。
//! 頼みを開いたら 3D ビューを前に出す（送り直しでは出し直さない）。文は名前と状態だけで、案内は書かない。
use crate::common;
use crate::common::wait;

use std::time::{Duration, Instant};

use common::livelink::{arm_request, Exchange};
use common::*;
use egui::{Color32, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use wait::WATCHDOG;
use yolu_app::lang::Lang;
use yolu_app::livelink::{LinkIndicator, LinkStatus};
use yolu_app::state::PopupKind;
use yolu_app::ui::theme as t;
use yolu_app::{shell, Tab, YoluApp};

fn step_until(h: &mut Harness<'_, YoluApp>, what: &str, mut cond: impl FnMut(&YoluApp) -> bool) {
    let deadline = Instant::now() + WATCHDOG;
    loop {
        h.step();
        if cond(h.state()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what} を待ったが来ない: {:?} {}",
            h.state().state.link.status,
            h.state().state.message
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// 受け渡しのフォルダを試しのフォルダにして、受け付けを始める。
fn accept(h: &mut Harness<'_, YoluApp>, ex: &Exchange) {
    h.state_mut()
        .link_mut()
        .set_folder(ex.root.clone())
        .unwrap();
    h.state_mut().state.prefs.settings.livelink_on_startup = true;
    step_until(h, "受付", |a| {
        a.state.link.status == LinkStatus::Accepting
    });
}

/// 腕の頼みを置いて、開くまで待つ。
fn open_arm(h: &mut Harness<'_, YoluApp>, ex: &Exchange, id: &str, key: &str) {
    let fbx = ex.dir.join("arm.fbx");
    if !fbx.exists() {
        ex.write_arm("arm.fbx");
    }
    let png = ex.dir.join("skin.png");
    if !png.exists() {
        ex.write_png("skin.png", 32, [180, 90, 60, 255]);
    }
    ex.put(&arm_request(id, key, &fbx, &png, &ex.dir.join("export")));
    step_until(h, "返事", |_| !ex.folder().replies().unwrap().is_empty());
    for r in ex.take_replies() {
        assert_eq!(r.kind, yolu_protocol::files::ReplyKind::Opened);
    }
}

fn popup_kind(h: &Harness<'_, YoluApp>) -> Option<PopupKind> {
    h.state().state.popup.as_ref().map(|p| p.kind)
}

/// 入口の印の矩形（ボタンで、名前はツールチップの文。ウィンドウの見出し「Live Link: 状態」と同じ文になることがあるので、役でも絞る）。
fn icon_rect(h: &Harness<'_, YoluApp>) -> Rect {
    let tip = h.state().state.link.tooltip(h.state().state.lang);
    h.get_by_role_and_label(egui::accesskit::Role::Button, &tip)
        .rect()
}

/// 印の矩形の中の、この色（に近い）画素の数。
fn pixels_near(h: &mut Harness<'_, YoluApp>, rect: Rect, color: Color32) -> usize {
    let image = h.render().expect("描画");
    let mut count = 0;
    for y in rect.top() as u32..rect.bottom() as u32 {
        for x in rect.left() as u32..rect.right() as u32 {
            let p = image.get_pixel(x, y).0;
            let d = (p[0] as i32 - color.r() as i32).abs()
                + (p[1] as i32 - color.g() as i32).abs()
                + (p[2] as i32 - color.b() as i32).abs();
            if d <= 24 {
                count += 1;
            }
        }
    }
    count
}

/// 開いているウィンドウ（ポップアップ）と入口の印のあたりを撮る。
fn shot_popup(h: &mut Harness<'_, YoluApp>, name: &str) {
    let body = h
        .state()
        .state
        .popup
        .as_ref()
        .expect("ウィンドウ")
        .state
        .rect;
    let icon = icon_rect(h);
    let area = body.union(icon).expand(6.0);
    h.event(egui::Event::PointerGone);
    h.step();
    let image = h.render().expect("描画");
    let cropped = image::imageops::crop_imm(
        &image,
        area.left().max(0.0) as u32,
        area.top().max(0.0) as u32,
        area.width() as u32,
        area.height() as u32,
    )
    .to_image();
    egui_kittest::image_snapshot(&cropped, name);
}

const KEY: &str = "GlobalObjectId_V1-2-0123-4567-0";

#[test]
fn the_mark_changes_color_with_the_state_and_sits_left_of_the_project_name() {
    let mut h = app(1280.0, 800.0, 256);
    // ウィンドウを作っただけでは受け付けない（利用者のフォルダに触らない）。受けないは灰
    assert_eq!(h.state().state.link.indicator(), LinkIndicator::Off);
    let rect = icon_rect(&h);
    assert!(
        rect.top() < t::MENU_BAR_HEIGHT && rect.right() < 1280.0 - 60.0,
        "メニューバーの右の方: {rect:?}"
    );
    assert!(pixels_near(&mut h, rect, t::TEXT_DISABLED) > 8);
    // 受付中は薄い色
    let ex = Exchange::new("mark");
    accept(&mut h, &ex);
    h.run();
    assert_eq!(h.state().state.link.indicator(), LinkIndicator::Waiting);
    let rect = icon_rect(&h);
    assert!(pixels_near(&mut h, rect, t::ACCENT_DIM) > 8);
    // 相手の文書は緑（Unity が送れなかった物があれば警告の色）
    open_arm(&mut h, &ex, "r1", KEY);
    h.run();
    assert_eq!(h.state().state.link.indicator(), LinkIndicator::Problems);
    let rect = icon_rect(&h);
    assert!(pixels_near(&mut h, rect, t::WARNING) > 8);
    assert!(h
        .state()
        .state
        .link
        .tooltip(Lang::Ja)
        .contains("Accessory: FBX のメッシュではありません"));
    assert_eq!(shell::link_indicator_color(LinkIndicator::Linked), t::OK);
    assert_eq!(shell::link_indicator_color(LinkIndicator::Failed), t::ERROR);
    // 状態の帯には Live Link の文字を出さない（直前の操作の結果の message だけ）
    assert_eq!(
        shell::status_text(&h.state().state),
        h.state().state.message
    );
}

#[test]
fn pressing_the_mark_opens_a_small_window_that_accepts_and_stops() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 800.0, 256);
        h.state_mut().state.lang = lang;
        let ex = Exchange::new("window");
        h.state_mut()
            .link_mut()
            .set_folder(ex.root.clone())
            .unwrap();
        h.state_mut().state.prefs.settings.livelink_on_startup = false;
        h.run();
        let at = icon_rect(&h).center();
        click(&mut h, at);
        assert_eq!(popup_kind(&h), Some(PopupKind::LiveLink), "{lang:?}");
        h.get_by_label(lang.pick("受け付ける", "Accept"));
        h.get_by_label(lang.pick("受け付けない", "Don't accept"));
        // 受け付けていないときは「受け付ける」で受け付けを始める（設定も入る）
        let at = popup_item(&h, lang.pick("受け付ける", "Accept")).center();
        click(&mut h, at);
        step_until(&mut h, "受付", |a| {
            a.state.link.status == LinkStatus::Accepting
        });
        assert!(h.state().state.prefs.settings.livelink_on_startup);
        assert_eq!(popup_kind(&h), None);
        assert!(ex.root.join("presence.json").is_file());
        // もう一度押すとウィンドウが開き、押した印をもう一度押すと閉じる
        let at = icon_rect(&h).center();
        click(&mut h, at);
        assert_eq!(popup_kind(&h), Some(PopupKind::LiveLink));
        let at = icon_rect(&h).center();
        click(&mut h, at);
        assert_eq!(popup_kind(&h), None);
        // 「受け付けない」でやめる（起きている印も消す）
        let at = icon_rect(&h).center();
        click(&mut h, at);
        let at = popup_item(&h, lang.pick("受け付けない", "Don't accept")).center();
        click(&mut h, at);
        assert_eq!(h.state().state.link.status, LinkStatus::Off, "{lang:?}");
        assert!(!ex.root.join("presence.json").exists());
        // Esc でも閉じる
        let at = icon_rect(&h).center();
        click(&mut h, at);
        key(&h, egui::Key::Escape, egui::Modifiers::NONE);
        h.run();
        assert_eq!(popup_kind(&h), None);
    }
}

#[test]
fn the_window_names_the_target_and_the_3d_view_comes_forward_once() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 800.0, 256);
        h.state_mut().state.lang = lang;
        let ex = Exchange::new("target");
        accept(&mut h, &ex);
        h.run();
        assert!(h.state().view3d_rect().is_none(), "初めはキャンバスが前");
        let at = icon_rect(&h).center();
        click(&mut h, at);
        shot_popup(
            &mut h,
            &format!("livelink_popup_waiting_{}", lang.pick("ja", "en")),
        );
        key(&h, egui::Key::Escape, egui::Modifiers::NONE);
        h.run();
        open_arm(&mut h, &ex, "r1", KEY);
        h.run();
        assert!(
            h.state().view3d_rect().is_some(),
            "開いたら 3D ビューに出す"
        );
        // ウィンドウ: 「Live Link: 受付中」と、開いている Unity のオブジェクトの名前
        let at = icon_rect(&h).center();
        click(&mut h, at);
        h.get_by_label(&format!("Live Link: {}", lang.pick("受付中", "Accepting")));
        h.get_by_label("Unity: Arm");
        shot_popup(
            &mut h,
            &format!("livelink_popup_target_{}", lang.pick("ja", "en")),
        );
        key(&h, egui::Key::Escape, egui::Modifiers::NONE);
        h.run();
        // 送り直しでは、キャンバスへ戻した画面を 3D に取り上げない
        click_tab(&mut h, Tab::Canvas);
        assert!(h.state().view3d_rect().is_none());
        open_arm(&mut h, &ex, "r2", KEY);
        h.run();
        assert!(
            h.state().view3d_rect().is_none(),
            "送り直しでは 3D ビューを前に出し直さない"
        );
    }
}

/// プロジェクトの名前が長くても、印は見えている名前の左に付き、メニューの見出しに重ならない。名前は後ろを詰め、全体はツールチップに出す。
/// ウィンドウの最小の幅（960）と、それより狭いウィンドウの両方で、日本語（幅広の書体）の長い名前と、保存していない印（•）つきを確かめる。
#[test]
fn a_long_project_name_is_cut_short_and_the_mark_stays_clear_of_the_menu_titles() {
    let long = "とても長いプロジェクトの名前をつけたときの見え方を確かめるための名前その二十三十四十五十六十七十八十九百";
    for lang in Lang::ALL {
        let last_title = *shell::menu_titles(lang).last().unwrap();
        for width in [960.0, 700.0] {
            let mut h = app(width, 800.0, 256);
            h.state_mut().state.lang = lang;
            h.state_mut().state.project_name = long.to_owned();
            h.state_mut().state.modified = true;
            h.run();
            let icon = icon_rect(&h);
            let menu_end = menu_title(&h, last_title).right();
            assert!(
                icon.left() >= menu_end,
                "{lang:?} {width}: 印 {icon:?} がメニューの見出し（右端 {menu_end}）に重なる"
            );
            assert!(
                icon.right() < width - 8.0,
                "{lang:?} {width}: 印 {icon:?} が名前の左にある"
            );
            // 見えている名前は印の右。印とウィンドウの右の間に、名前の画素（明るい字）がある
            let image = h.render().expect("描画");
            let mut lit = 0;
            for y in 2..(t::MENU_BAR_HEIGHT as u32 - 2) {
                for x in icon.right() as u32 + 2..width as u32 - 6 {
                    let p = image.get_pixel(x, y).0;
                    if p[0] as u32 + p[1] as u32 + p[2] as u32 > 330 {
                        lit += 1;
                    }
                }
            }
            assert!(
                lit > 40,
                "{lang:?} {width}: 名前の字が印の右に見えない（{lit}）"
            );
            // 詰めたので、全体の名前はツールチップに出る
            hover_and_wait(&mut h, egui::pos2(width - 20.0, t::MENU_BAR_HEIGHT / 2.0));
            assert!(
                h.query_by_label(long).is_some(),
                "{lang:?} {width}: 詰めた名前のツールチップが出ない"
            );
        }
    }
}

/// 短い名前は詰めず、ツールチップも出さない（印の位置は名前の幅で決まる）。
#[test]
fn a_short_project_name_is_shown_whole_without_a_tooltip() {
    let mut h = app(960.0, 800.0, 256);
    h.state_mut().state.project_name = "短い名前".to_owned();
    h.run();
    hover_and_wait(&mut h, egui::pos2(960.0 - 20.0, t::MENU_BAR_HEIGHT / 2.0));
    assert!(h.query_by_label("短い名前").is_none());
    let icon = icon_rect(&h);
    assert!(
        icon.right() < 960.0 - 8.0 - 40.0,
        "短い名前の分だけ右に寄る: {icon:?}"
    );
}
