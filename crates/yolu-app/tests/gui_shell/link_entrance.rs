//! Live Link の入口（メニューバーの右端、プロジェクトの名前の左の Unity の印）: 切断は灰・待機中は薄い色・接続は緑・版の不一致は
//! 警告の色。押すと小さな窓（状態・つながっている Unity・受け取ったモデルの名前と、待つ／切る）。モデルが届いたら 3D ビューを前に出す。
//! 文は名前と状態だけで、案内は書かない。
use crate::common;
use crate::common::wait;

use std::time::{Duration, Instant};

use common::*;
use egui::{Color32, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use wait::WATCHDOG;
use yolu_app::lang::Lang;
use yolu_app::livelink::{LinkIndicator, LinkStatus};
use yolu_app::state::{Action, PopupKind};
use yolu_app::ui::theme as t;
use yolu_app::{shell, Tab, YoluApp};
use yolu_protocol::link::connect_and_greet_as;
use yolu_protocol::{
    channel, AppVersion, ChannelRoute, Connection, Identity, MaterialInfo, MaterialKey, MeshData,
    Message, Model, Received, Submesh, TextureProperty,
};

fn unique_name(tag: &str) -> String {
    crate::common::names::unique_name("ylent", tag)
}

fn step_until(h: &mut Harness<'_, YoluApp>, what: &str, mut cond: impl FnMut(&YoluApp) -> bool) {
    let deadline = Instant::now() + WATCHDOG;
    loop {
        h.step();
        if cond(h.state()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what} を待ったが来ない: {:?}",
            h.state().state.link.status
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Unity の役（挨拶の名乗りを選べる）。
struct FakeUnity {
    conn: Connection,
}

impl FakeUnity {
    fn connect(name: &str, agent: &str) -> FakeUnity {
        // 版を名乗る Unity（名乗らない古いブリッジだと、入口の印は版のずれの警告の色になる。link_version.rs）
        // 機能の印もスタンドアロンと同じにする（印のずれも警告になる）
        let identity = Identity::unity(agent)
            .with_version(Some(AppVersion::new(0, 3, 0)))
            .with_features(yolu_app::livelink::FEATURES);
        let (conn, mut reader, _) = connect_and_greet_as(name, &identity).unwrap();
        let reply = conn.clone();
        // 来たものは読み捨てる（Unity の役は返事を見ない）。つながりが終われば止まる
        std::thread::spawn(move || {
            while let Ok(Received::Idle | Received::Message(_)) = reader.next(&reply) {}
        });
        FakeUnity { conn }
    }

    fn send(&self, m: Message) {
        self.conn.send(&m).unwrap();
    }
}

fn model(generation: u32, name: &str) -> Model {
    Model {
        generation,
        name: name.into(),
        materials: vec![MaterialInfo {
            key: MaterialKey::Material {
                name: "Skin".into(),
                asset: None,
            },
            shader: "Standard".into(),
            textures: vec![TextureProperty {
                name: "_MainTex".into(),
                width: 64,
                height: 64,
            }],
            routes: vec![ChannelRoute {
                channel: channel::COLOR,
                property: "_MainTex".into(),
            }],
        }],
        meshes: vec![MeshData {
            key: "0".into(),
            name: "Quad".into(),
            skinned: false,
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]],
            normals: vec![],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
            submeshes: vec![Submesh {
                material: 0,
                indices: vec![0, 2, 1, 1, 2, 3],
            }],
        }],
    }
}

fn popup_kind(h: &Harness<'_, YoluApp>) -> Option<PopupKind> {
    h.state().state.popup.as_ref().map(|p| p.kind)
}

/// 入口の印の矩形（名前はツールチップの文）。
fn icon_rect(h: &Harness<'_, YoluApp>) -> Rect {
    let tip = h.state().state.link.tooltip(h.state().state.lang);
    h.get_by_label(&tip).rect()
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

#[test]
fn the_mark_changes_color_with_the_state_and_sits_left_of_the_project_name() {
    let mut h = app(1280.0, 800.0, 256);
    // 切断は灰
    assert_eq!(h.state().state.link.indicator(), LinkIndicator::Off);
    let rect = icon_rect(&h);
    assert!(
        rect.top() < t::MENU_BAR_HEIGHT && rect.right() < 1280.0 - 60.0,
        "メニューバーの右の方: {rect:?}"
    );
    assert!(pixels_near(&mut h, rect, t::TEXT_DISABLED) > 8);
    // 待機中は薄い色
    let name = unique_name("mark");
    h.state_mut().link_mut().set_name(&name).unwrap();
    h.state_mut().state.apply(Action::ToggleLiveLink);
    h.run();
    assert_eq!(h.state().state.link.status, LinkStatus::Listening);
    assert_eq!(h.state().state.link.indicator(), LinkIndicator::Waiting);
    let rect = icon_rect(&h);
    assert!(pixels_near(&mut h, rect, t::ACCENT_DIM) > 8);
    // つながると緑
    let _unity = FakeUnity::connect(
        &name,
        "YoluPainter 0.3.0 (Unity 2022.3.22f1) (yolu-bridge abi 1)",
    );
    step_until(&mut h, "つながる", |a| {
        matches!(a.state.link.status, LinkStatus::Connected { .. })
    });
    h.run();
    assert_eq!(h.state().state.link.indicator(), LinkIndicator::Connected);
    let rect = icon_rect(&h);
    assert!(pixels_near(&mut h, rect, t::OK) > 8);
    assert_eq!(
        shell::link_indicator_color(LinkIndicator::Mismatch),
        t::WARNING
    );
    assert_eq!(shell::link_indicator_color(LinkIndicator::Failed), t::ERROR);
    // 状態の帯には Live Link の文字を出さない（直前の操作の結果の message だけ）
    assert_eq!(
        shell::status_text(&h.state().state),
        h.state().state.message
    );
}

#[test]
fn pressing_the_mark_opens_a_small_window_that_waits_and_stops() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 800.0, 256);
        h.state_mut().state.lang = lang;
        let name = unique_name("window");
        h.state_mut().link_mut().set_name(&name).unwrap();
        h.run();
        let at = icon_rect(&h).center();
        click(&mut h, at);
        assert_eq!(popup_kind(&h), Some(PopupKind::LiveLink), "{lang:?}");
        // 状態の名前。つながっていないので Unity とモデルの行は無い
        h.get_by_label(lang.pick("待つ", "Wait"));
        h.get_by_label(lang.pick("切る", "Stop"));
        assert!(h.query_by_label_contains("Unity 2022").is_none());
        // 切断中は「切る」を選べず、「待つ」で待ち受けを始める
        let at = popup_item(&h, lang.pick("待つ", "Wait")).center();
        click(&mut h, at);
        assert_eq!(
            h.state().state.link.status,
            LinkStatus::Listening,
            "{lang:?}"
        );
        assert_eq!(popup_kind(&h), None);
        // もう一度押すと窓が開き、押した印をもう一度押すと閉じる
        let at = icon_rect(&h).center();
        click(&mut h, at);
        assert_eq!(popup_kind(&h), Some(PopupKind::LiveLink));
        let at = icon_rect(&h).center();
        click(&mut h, at);
        assert_eq!(popup_kind(&h), None);
        // 「切る」で待ち受けをやめる
        let at = icon_rect(&h).center();
        click(&mut h, at);
        let at = popup_item(&h, lang.pick("切る", "Stop")).center();
        click(&mut h, at);
        assert_eq!(h.state().state.link.status, LinkStatus::Off, "{lang:?}");
        // Esc でも閉じる
        let at = icon_rect(&h).center();
        click(&mut h, at);
        key(&h, egui::Key::Escape, egui::Modifiers::NONE);
        h.run();
        assert_eq!(popup_kind(&h), None);
    }
}

#[test]
fn the_window_names_the_unity_and_the_model_and_the_3d_view_comes_forward_once() {
    let mut h = app(1280.0, 800.0, 256);
    let name = unique_name("model");
    h.state_mut().link_mut().set_name(&name).unwrap();
    h.state_mut().state.apply(Action::ToggleLiveLink);
    h.run();
    assert!(h.state().view3d_rect().is_none(), "初めはキャンバスが前");
    let unity = FakeUnity::connect(
        &name,
        "YoluPainter 0.3.0 (Unity 2022.3.22f1) (yolu-bridge abi 1)",
    );
    step_until(&mut h, "つながる", |a| {
        matches!(a.state.link.status, LinkStatus::Connected { .. })
    });
    // つないだだけでは 3D ビューを前に出さない（モデルが届いたとき）
    h.run();
    assert!(h.state().view3d_rect().is_none());
    unity.send(Message::Model(model(1, "試しの四角")));
    step_until(&mut h, "モデル", |a| a.state.model.is_some());
    h.run();
    assert!(
        h.state().view3d_rect().is_some(),
        "モデルが届いたら 3D ビューに出す"
    );
    // 窓: 状態・Unity の名前（版の名前だけ）・モデルの名前
    let at = icon_rect(&h).center();
    click(&mut h, at);
    assert_eq!(popup_kind(&h), Some(PopupKind::LiveLink));
    h.get_by_label("Unity 2022.3.22f1");
    h.get_by_label("モデル: 試しの四角");
    // 接続中は「待つ」を選べず「切る」を選べる
    let at = popup_item(&h, "切る").center();
    click(&mut h, at);
    assert_eq!(h.state().state.link.status, LinkStatus::Off);
    // 切ったあとも 3D の形は残るが、記録は出どころと切れる（窓にモデルの名前は出ない）
    let at = icon_rect(&h).center();
    click(&mut h, at);
    assert!(h.query_by_label("モデル: 試しの四角").is_none());
    key(&h, egui::Key::Escape, egui::Modifiers::NONE);
    h.run();

    // 同じつながりで送り直しても、キャンバスへ戻した画面を 3D に取り上げない
    h.state_mut().state.apply(Action::ToggleLiveLink);
    h.run();
    let unity = FakeUnity::connect(&name, "試験の Unity");
    step_until(&mut h, "つながる", |a| {
        matches!(a.state.link.status, LinkStatus::Connected { .. })
    });
    unity.send(Message::Model(model(2, "二つ目")));
    step_until(&mut h, "モデル", |a| {
        a.state.model.as_ref().is_some_and(|m| m.name == "二つ目")
    });
    h.run();
    click_tab(&mut h, Tab::Canvas);
    assert!(h.state().view3d_rect().is_none());
    unity.send(Message::Model(model(3, "二つ目")));
    step_until(&mut h, "モデル", |a| {
        a.state.model.as_ref().is_some_and(|m| m.generation == 3)
    });
    h.run();
    assert!(
        h.state().view3d_rect().is_none(),
        "送り直しでは 3D ビューを前に出し直さない"
    );
    // つないだ相手の名乗りが版の形でなければ、名乗りをそのまま出す
    let at = icon_rect(&h).center();
    click(&mut h, at);
    h.get_by_label("試験の Unity");
}

/// プロジェクトの名前が長くても、印は見えている名前の左に付き、メニューの見出しに重ならない。名前は後ろを詰め、全体はツールチップに出す。
/// 窓の最小の幅（960）と、それより狭い窓の両方で、日本語（幅広の書体）の長い名前と、保存していない印（•）つきを確かめる。
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
            // 見えている名前は印の右。印と窓の右の間に、名前の画素（明るい字）がある
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
