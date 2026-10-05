//! Live Link の版の確かめ（スタンドアロンの側）: つないだまま、版か機能の印がずれていれば、入口の印を警告の色にし、ツールチップに両方の版・
//! どちらを上げればよいか・使えない機能の名前を出す。プロトコルの版が重ならなければ断り、その文は日英で「どちらを何版以上に」の形。
//! 欄の無い古い相手とも今までどおりつながる。画面の文字（状態の帯・窓）には出さず、警告の理由はツールチップだけ。
use crate::common;
use crate::common::wait;

use std::io::Write;
use std::time::{Duration, Instant};

use common::*;
use egui::{Color32, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use wait::WATCHDOG;
use yolu_app::lang::Lang;
use yolu_app::livelink::{self, LinkIndicator, LinkStatus};
use yolu_app::state::Action;
use yolu_app::ui::theme as t;
use yolu_app::{shell, YoluApp};
use yolu_protocol::link::{self, connect_and_greet, connect_and_greet_as};
use yolu_protocol::{
    encode_message, feature, AppVersion, Connection, FrameReader, Hello, HelloAuth, Identity,
    LinkKey, Message, Received, RejectCode, VersionInfo, PROTOCOL_VERSION,
};

const V: fn(u16, u16, u16) -> AppVersion = AppVersion::new;

fn unique_name(tag: &str) -> String {
    crate::common::names::unique_name("ylver", tag)
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

/// 待ち受けを始めて名前を返す。
fn listen(h: &mut Harness<'_, YoluApp>, tag: &str) -> String {
    let name = unique_name(tag);
    h.state_mut().link_mut().set_name(&name).unwrap();
    h.state_mut().state.apply(Action::ToggleLiveLink);
    h.run();
    assert_eq!(h.state().state.link.status, LinkStatus::Listening);
    name
}

/// Unity の役: 名乗り（版・求める版・機能の印）を選んでつなぐ。来たものは読み捨てる。
struct Unity {
    _conn: Connection,
}

impl Unity {
    fn connect(name: &str, identity: &Identity) -> Unity {
        let (conn, mut reader, _) = connect_and_greet_as(name, identity).unwrap();
        let reply = conn.clone();
        std::thread::spawn(move || {
            while let Ok(Received::Idle | Received::Message(_)) = reader.next(&reply) {}
        });
        Unity { _conn: conn }
    }
}

fn unity(version: Option<AppVersion>, min_peer: AppVersion, features: u64) -> Identity {
    Identity::unity("YoluPainter 0.3.0 (Unity 2022.3.22f1)")
        .with_version(version)
        .with_min_peer(min_peer)
        .with_features(features)
}

fn connected(h: &mut Harness<'_, YoluApp>) {
    step_until(h, "つながる", |a| {
        matches!(a.state.link.status, LinkStatus::Connected { .. })
    });
    h.run();
}

fn icon_rect(h: &Harness<'_, YoluApp>) -> Rect {
    let tip = h.state().state.link.tooltip(h.state().state.lang);
    h.get_by_label(&tip).rect()
}

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

fn own_version() -> AppVersion {
    livelink::identity()
        .app_version
        .expect("Cargo の版は読める")
}

#[test]
fn a_unity_that_meets_the_versions_shows_no_warning() {
    let mut h = app(1280.0, 800.0, 256);
    let name = listen(&mut h, "clean");
    // 機能の印もこのスタンドアロンと同じ（印のずれも警告になる。印を立てる機能が増えても、この試験は変わらない）
    let _u = Unity::connect(
        &name,
        &unity(Some(V(0, 3, 0)), V(0, 0, 0), livelink::FEATURES),
    );
    connected(&mut h);
    let link = h.state().state.link.clone();
    assert_eq!(link.indicator(), LinkIndicator::Connected);
    assert!(link.skew().is_none());
    // ツールチップは状態の 1 行だけ
    for lang in Lang::ALL {
        assert_eq!(link.tooltip(lang).lines().count(), 1, "{lang:?}");
    }
    let rect = icon_rect(&h);
    assert!(pixels_near(&mut h, rect, t::OK) > 8);
    // 名乗った版と決まった版が見える（両側から同じ共通部分）
    let info = link.link.as_ref().unwrap();
    assert_eq!(info.peer.app_version(), Some(V(0, 3, 0)));
    assert_eq!(info.own.app_version, Some(own_version()));
    assert_eq!(link.common_features(), livelink::FEATURES);
}

#[test]
fn an_old_unity_without_the_version_fields_connects_and_the_mark_turns_to_the_warning_color() {
    let mut h = app(1280.0, 800.0, 256);
    let name = listen(&mut h, "old");
    // 版・機能の印の欄を送らない古いブリッジ（connect_and_greet は名乗りの文字列だけ）
    let (conn, mut reader, welcome) = connect_and_greet(&name, "古い Unity").unwrap();
    let reply = conn.clone();
    std::thread::spawn(move || {
        while let Ok(Received::Idle | Received::Message(_)) = reader.next(&reply) {}
    });
    connected(&mut h);
    // つながる（欄を読めない古い相手にも、返事は読める）
    assert_eq!(welcome.version, PROTOCOL_VERSION);
    let own = own_version();
    let s = &h.state().state;
    assert_eq!(s.link.indicator(), LinkIndicator::Skewed);
    assert_eq!(
        shell::link_indicator_color(LinkIndicator::Skewed),
        t::WARNING
    );
    let rect = icon_rect(&h);
    assert!(
        pixels_near(&mut h, rect, t::WARNING) > 8,
        "入口の印が警告の色"
    );
    for lang in Lang::ALL {
        let tip = h.state().state.link.tooltip(lang);
        // 状態の 1 行に続けて、両方の版（相手は不明）とどちらを上げるか
        assert!(tip.lines().count() >= 3, "{lang:?}: {tip}");
        assert!(tip.contains(&own.to_string()), "{lang:?}: {tip}");
        assert!(
            tip.contains(lang.pick("不明", "unknown")),
            "{lang:?}: {tip}"
        );
        assert!(
            tip.contains(lang.pick(
                "Unity のパッケージを更新する必要があります",
                "The Unity package must be updated"
            )),
            "{lang:?}: {tip}"
        );
    }
    // 状態の帯には出さない（直前の操作の結果の message だけ）
    assert_eq!(
        shell::status_text(&h.state().state),
        h.state().state.message
    );
    assert!(!h.state().state.message.contains("更新"));
    // つないだままで、描く・モデルを受けるなどの通常の流れは変わらない（版のずれはつなぐのを妨げない）
    assert!(matches!(
        h.state().state.link.status,
        LinkStatus::Connected { .. }
    ));
    // 切れると警告も消える
    conn.send(&Message::Bye).unwrap();
    step_until(&mut h, "切れた", |a| {
        a.state.link.status == LinkStatus::Listening
    });
    assert_eq!(h.state().state.link.indicator(), LinkIndicator::Waiting);
    assert!(h.state().state.link.skew().is_none());
}

#[test]
fn a_unity_that_asks_for_a_newer_standalone_gets_the_version_to_update_to() {
    let mut h = app(1280.0, 800.0, 256);
    let name = listen(&mut h, "newer");
    // Unity のパッケージが、スタンドアロンに求める版（この版より新しい）
    let wanted = V(own_version().major + 1, 2, 0);
    let _u = Unity::connect(
        &name,
        &unity(Some(V(0, 9, 0)), wanted, livelink::FEATURES),
    );
    connected(&mut h);
    let link = &h.state().state.link;
    assert_eq!(link.indicator(), LinkIndicator::Skewed);
    let skew = link.skew().unwrap();
    assert_eq!(skew.update_self, Some(wanted));
    assert!(skew.own_should_update() && !skew.peer_should_update());
    for lang in Lang::ALL {
        let tip = link.tooltip(lang);
        assert!(
            tip.contains(&lang.pick(
                format!("スタンドアロンを {wanted} 以上に上げる必要があります"),
                format!("The standalone must be {wanted} or newer")
            )),
            "{lang:?}: {tip}"
        );
        // 両方の版
        assert!(
            tip.contains(&own_version().to_string()) && tip.contains("0.9.0"),
            "{lang:?}: {tip}"
        );
        assert!(
            !tip.contains(lang.pick("Unity のパッケージを", "The Unity package")),
            "{lang:?}: {tip}"
        );
    }
}

/// 機能の印の名前（画面の文と同じ。名前を知らない印は「新しい機能」にまとめる）。
fn names_of(lang: Lang, mask: u64) -> String {
    let table = [
        (feature::MATERIAL_VALUES, "マテリアルの値", "Material values"),
        (feature::ASSETS, "アセット", "Assets"),
        (feature::PROJECT_TRANSFER, "プロジェクトの転送", "Project transfer"),
        (feature::ANIMATION, "アニメーション", "Animation"),
        (feature::ORIGINAL_TEXTURES, "元のテクスチャ", "Original textures"),
        (feature::MATERIAL_REQUEST, "マテリアルの頼み", "Material requests"),
    ];
    let mut names: Vec<&str> = table
        .iter()
        .filter(|(bit, ..)| mask & bit != 0)
        .map(|(_, ja, en)| lang.pick(*ja, *en))
        .collect();
    if mask & !feature::KNOWN != 0 {
        names.push(lang.pick("新しい機能", "Newer features"));
    }
    names.join(lang.pick("・", ", "))
}

#[test]
fn features_only_one_side_has_are_named_and_the_common_part_is_what_can_be_used() {
    let mut h = app(1280.0, 800.0, 256);
    let name = listen(&mut h, "marks");
    // Unity は、このスタンドアロンが出していない名前のある印（全部出しているなら無し）と、名前を知らない印を足して出す。
    // このスタンドアロンの印は定数から取り、期待する集合も定数から計算する（印を立てる機能が増えても、この試験は変わらない）
    let unknown = 1u64 << 50;
    let extra = (feature::KNOWN & !livelink::FEATURES) | unknown;
    let _u = Unity::connect(
        &name,
        &unity(Some(V(0, 3, 0)), V(0, 0, 0), livelink::FEATURES | extra),
    );
    connected(&mut h);
    let link = &h.state().state.link;
    assert_eq!(link.indicator(), LinkIndicator::Skewed);
    let skew = link.skew().unwrap();
    assert_eq!(skew.missing_here, extra);
    assert_eq!(skew.missing_on_peer, 0);
    assert_eq!(link.common_features(), livelink::FEATURES);
    for lang in Lang::ALL {
        let tip = link.tooltip(lang);
        assert!(
            tip.contains(&format!(
                "{}: {}",
                lang.pick("使えない機能", "Unavailable"),
                names_of(lang, extra)
            )),
            "{lang:?}: {tip}"
        );
        assert!(
            tip.contains(lang.pick(
                "スタンドアロンを更新する必要があります",
                "The standalone must be updated"
            )),
            "{lang:?}: {tip}"
        );
    }
}

#[test]
fn features_this_standalone_has_and_the_unity_lacks_are_named() {
    let mut h = app(1280.0, 800.0, 256);
    let name = listen(&mut h, "lacks");
    // Unity は印を 1 つも出さない（このスタンドアロンの印は定数から取る。印が無ければ、ずれなし）
    let _u = Unity::connect(&name, &unity(Some(V(0, 3, 0)), V(0, 0, 0), 0));
    connected(&mut h);
    let link = &h.state().state.link;
    assert_eq!(link.common_features(), 0);
    assert_eq!(
        link.skew().map_or(0, |s| s.missing_on_peer),
        livelink::FEATURES
    );
    if livelink::FEATURES == 0 {
        assert_eq!(link.indicator(), LinkIndicator::Connected);
        return;
    }
    assert_eq!(link.indicator(), LinkIndicator::Skewed);
    for lang in Lang::ALL {
        let tip = link.tooltip(lang);
        assert!(
            tip.contains(&format!(
                "{}: {}",
                lang.pick("使えない機能", "Unavailable"),
                names_of(lang, livelink::FEATURES)
            )),
            "{lang:?}: {tip}"
        );
        assert!(
            tip.contains(lang.pick(
                "Unity のパッケージを更新する必要があります",
                "The Unity package must be updated"
            )),
            "{lang:?}: {tip}"
        );
    }
}

/// 鍵を知っていて、版の範囲だけが合わない生の挨拶を送り、断りを読む。
fn refused_greeting(
    name: &str,
    min: u16,
    max: u16,
    versions: Option<VersionInfo>,
) -> yolu_protocol::Reject {
    let key = LinkKey::load(name).unwrap();
    let stream = link::connect(name).unwrap();
    let mut s = &stream;
    let nonce = yolu_protocol::auth::random_bytes().unwrap();
    s.write_all(&encode_message(&Message::Hello(Hello {
        min_version: min,
        max_version: max,
        agent: "試験の Unity".into(),
        features: 0,
        auth: Some(HelloAuth {
            nonce,
            proof: key.hello_proof(&nonce),
        }),
        versions,
    })))
    .unwrap();
    let mut frames = FrameReader::new();
    match frames
        .read_frame(&mut s)
        .unwrap()
        .unwrap()
        .decode()
        .unwrap()
    {
        Message::Reject(r) => r,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_protocol_range_that_does_not_overlap_is_refused_and_the_tooltip_says_which_to_update() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 800.0, 256);
        h.state_mut().state.lang = lang;
        let name = listen(&mut h, "refuse");
        // Unity が新しすぎる（スタンドアロンを、Unity の求める版以上に）
        let wanted = V(7, 1, 0);
        let reject = refused_greeting(
            &name,
            PROTOCOL_VERSION + 1,
            PROTOCOL_VERSION + 2,
            Some(VersionInfo {
                app: V(9, 0, 0),
                min_peer: wanted,
            }),
        );
        assert_eq!(reject.code, RejectCode::VersionMismatch);
        step_until(&mut h, "版の不一致", |a| {
            a.state.link.mismatch.is_some()
        });
        let link = &h.state().state.link;
        assert_eq!(link.status, LinkStatus::Listening, "待ち受けは続ける");
        assert_eq!(link.indicator(), LinkIndicator::Mismatch);
        let tip = link.tooltip(lang);
        let range = |u: (u16, u16)| {
            lang.pick(
                format!("Unity 側 {}〜{}、スタンドアロン 1〜1", u.0, u.1),
                format!("Unity {}–{}, standalone 1–1", u.0, u.1),
            )
        };
        assert!(
            tip.contains(&range((PROTOCOL_VERSION + 1, PROTOCOL_VERSION + 2))),
            "{lang:?}: {tip}"
        );
        assert!(
            tip.contains(&lang.pick(
                format!("スタンドアロンを {wanted} 以上に上げる必要があります"),
                format!("The standalone must be {wanted} or newer")
            )),
            "{lang:?}: {tip}"
        );
        // 古すぎる Unity（版を名乗らない）: Unity のパッケージを更新
        let reject = refused_greeting(&name, 0, 0, None);
        assert_eq!(reject.code, RejectCode::VersionMismatch);
        step_until(&mut h, "もう 1 度", |a| {
            a.state
                .link
                .refusal
                .is_some_and(|r| r.update == yolu_protocol::Product::Unity)
        });
        let tip = h.state().state.link.tooltip(lang);
        assert!(
            tip.contains(lang.pick(
                "Unity のパッケージを更新する必要があります",
                "The Unity package must be updated"
            )),
            "{lang:?}: {tip}"
        );
        // 状態の帯には文を出さない: 知らせは短い名前だけで、どちらを上げるかはツールチップ
        assert!(
            !h.state().state.message.contains("更新")
                && !h.state().state.message.contains("must be"),
            "{}",
            h.state().state.message
        );
        // 版の合うブリッジがつながると、断りの表示は消える
        let _u = Unity::connect(
            &name,
            &unity(Some(V(0, 3, 0)), V(0, 0, 0), livelink::FEATURES),
        );
        connected(&mut h);
        assert!(h.state().state.link.mismatch.is_none() && h.state().state.link.refusal.is_none());
    }
}
