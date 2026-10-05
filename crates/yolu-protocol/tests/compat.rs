//! 互いの版と機能の印の取り決め: 欄の無い古い挨拶ともつながる・版のずれの警告・機能の共通部分・印の無い相手へ新しい命令を送らない・
//! 版の範囲が重ならない断りの文（日本語と英語）と、どちらを何版以上に上げるか。

#[path = "support/names.rs"]
mod names;

use std::thread;
use std::time::Duration;

use yolu_protocol::compat::{judge_ranges, refusal_from_reject};
use yolu_protocol::link::{self, accept_as, connect_and_greet_as, negotiate_as};
use yolu_protocol::*;

fn unique_name(tag: &str) -> String {
    names::unique_name("ylp-compat", tag)
}

const V: fn(u16, u16, u16) -> AppVersion = AppVersion::new;
/// 試験用の印（割り当て済みの印とも、新しい相手が足す印とも重ならない上の方のビット）。
const FEATURE_A: u64 = 1 << 40;
const FEATURE_B: u64 = 1 << 41;

fn standalone(version: Option<AppVersion>, min_peer: AppVersion, features: u64) -> Identity {
    Identity::standalone("試験のスタンドアロン")
        .with_version(version)
        .with_min_peer(min_peer)
        .with_features(features)
}

fn unity(version: Option<AppVersion>, min_peer: AppVersion, features: u64) -> Identity {
    Identity::unity("試験の Unity")
        .with_version(version)
        .with_min_peer(min_peer)
        .with_features(features)
}

/// 1 つのつながりを張って、両側の `LinkInfo` と、スタンドアロンの側の `Connection`・Unity の側の `Connection`（と読む側）を返す。
struct Pair {
    standalone_conn: Connection,
    standalone_reader: ConnectionReader,
    unity_conn: Connection,
    unity_reader: ConnectionReader,
    welcome: Welcome,
    hello: Hello,
}

fn connect_pair(tag: &str, server_identity: Identity, client_identity: Identity) -> Pair {
    let name = unique_name(tag);
    let listener = Server::bind(&name, false).unwrap();
    let server = thread::spawn(move || {
        let stream = listener.accept().unwrap();
        let (conn, reader, hello) = accept_as(
            stream,
            &server_identity,
            7,
            &listener.key(),
            link::HANDSHAKE_TIMEOUT,
            &|_| Ok(()),
        )
        .unwrap();
        (conn, reader, hello, listener)
    });
    let (unity_conn, unity_reader, welcome) =
        connect_and_greet_as(&name, &client_identity).unwrap();
    let (standalone_conn, standalone_reader, hello, _listener) = server.join().unwrap();
    // 待ち受けを落とすと鍵のファイルが消えるが、つながりは残る
    Pair {
        standalone_conn,
        standalone_reader,
        unity_conn,
        unity_reader,
        welcome,
        hello,
    }
}

#[test]
fn a_peer_without_the_version_fields_still_connects_and_is_taken_for_an_older_one() {
    // 古い Unity（版の欄・機能の印の無い挨拶。connect_and_greet は名乗りの文字列だけ）と、新しいスタンドアロン
    let name = unique_name("oldunity");
    let listener = Server::bind(&name, false).unwrap();
    let identity = standalone(Some(V(0, 1, 0)), V(0, 3, 0), FEATURE_A);
    let server = thread::spawn(move || {
        let stream = listener.accept().unwrap();
        let (conn, _reader, hello) = accept_as(
            stream,
            &identity,
            1,
            &listener.key(),
            link::HANDSHAKE_TIMEOUT,
            &|_| Ok(()),
        )
        .unwrap();
        (conn.link_info().cloned().unwrap(), hello, listener)
    });
    let (unity_conn, _r, welcome) = link::connect_and_greet(&name, "古い Unity").unwrap();
    let (info, hello, _l) = server.join().unwrap();
    assert_eq!(hello.versions, None);
    assert_eq!(hello.features, 0);
    // 新しいスタンドアロンの返事には欄がある。古い Unity はそれを読まずに使える（後ろの欄は読み飛ばされる）
    assert_eq!(welcome.versions.map(|v| v.app), Some(V(0, 1, 0)));
    // 古い Unity の側の口: 自分は印を出さないので、共通の機能は無い
    assert_eq!(unity_conn.common_features(), 0);
    let skew = info.skew();
    assert_eq!(info.peer.app_version(), None);
    assert_eq!(
        skew.update_peer,
        Some(V(0, 3, 0)),
        "版を名乗らない相手は上げるのを勧める"
    );
    assert_eq!(skew.update_self, None);
    assert_eq!(skew.missing_on_peer, FEATURE_A);
    assert!(skew.is_skewed() && skew.peer_should_update() && !skew.own_should_update());
    assert_eq!(info.common_features(), 0);

    // 新しい Unity と、欄を書かない古いスタンドアロン
    let name = unique_name("oldstandalone");
    let listener = Server::bind(&name, false).unwrap();
    let server = thread::spawn(move || {
        let stream = listener.accept().unwrap();
        let r = link::accept(stream, "古いスタンドアロン", 1, &listener.key());
        (r.map(|(c, _, h)| (c, h)).is_ok(), listener)
    });
    let own = unity(Some(V(0, 3, 0)), V(0, 1, 0), FEATURE_B);
    let (conn, _r, welcome) = connect_and_greet_as(&name, &own).unwrap();
    assert!(server.join().unwrap().0);
    assert_eq!(welcome.versions, None);
    let info = conn.link_info().unwrap();
    assert_eq!(info.peer.app_version(), None);
    assert_eq!(info.skew().update_peer, Some(V(0, 1, 0)));
    assert_eq!(info.common_features(), 0);
}

#[test]
fn the_versions_and_marks_are_exchanged_and_the_common_part_is_the_usable_features() {
    let pair = connect_pair(
        "marks",
        standalone(
            Some(V(0, 1, 4)),
            V(0, 3, 0),
            FEATURE_A | feature::MATERIAL_VALUES,
        ),
        unity(
            Some(V(0, 3, 2)),
            V(0, 1, 0),
            FEATURE_B | feature::MATERIAL_VALUES,
        ),
    );
    let s = pair.standalone_conn.link_info().unwrap();
    let u = pair.unity_conn.link_info().unwrap();
    assert_eq!(pair.hello.features, FEATURE_B | feature::MATERIAL_VALUES);
    assert_eq!(pair.welcome.features, FEATURE_A | feature::MATERIAL_VALUES);
    // 双方が同じ共通部分を見る
    assert_eq!(s.common_features(), feature::MATERIAL_VALUES);
    assert_eq!(u.common_features(), feature::MATERIAL_VALUES);
    assert_eq!(
        pair.standalone_conn.common_features(),
        feature::MATERIAL_VALUES
    );
    assert!(s.has_feature(feature::MATERIAL_VALUES) && u.has_feature(feature::MATERIAL_VALUES));
    assert!(!s.has_feature(FEATURE_A) && !u.has_feature(FEATURE_B) && !u.has_feature(0));
    assert_eq!(
        (s.protocol, u.protocol),
        (PROTOCOL_VERSION, PROTOCOL_VERSION)
    );
    // 相手の版と名乗り
    assert_eq!(s.peer.app_version(), Some(V(0, 3, 2)));
    assert_eq!(u.peer.app_version(), Some(V(0, 1, 4)));
    assert_eq!(s.peer.agent, "試験の Unity");
    assert_eq!(u.peer.agent, "試験のスタンドアロン");
    // 版は求める版を満たすので、ずれは機能の印だけ
    let ss = s.skew();
    assert_eq!((ss.update_peer, ss.update_self), (None, None));
    assert_eq!(
        (ss.missing_on_peer, ss.missing_here),
        (FEATURE_A, FEATURE_B)
    );
    let us = u.skew();
    assert_eq!(
        (us.missing_on_peer, us.missing_here),
        (FEATURE_B, FEATURE_A)
    );
    assert!(ss.is_skewed() && ss.peer_should_update() && ss.own_should_update());
}

#[test]
fn a_matching_pair_has_no_skew_and_a_too_old_side_is_told_which_one_to_update() {
    let clean = connect_pair(
        "clean",
        standalone(Some(V(0, 2, 0)), V(0, 3, 0), 0),
        unity(Some(V(0, 3, 0)), V(0, 2, 0), 0),
    );
    assert!(!clean
        .standalone_conn
        .link_info()
        .unwrap()
        .skew()
        .is_skewed());
    assert!(!clean.unity_conn.link_info().unwrap().skew().is_skewed());

    // 別の製品なので版の番号が違うこと自体はずれではない（求める版に足りているか）
    // Unity のパッケージが、スタンドアロンの求める版より古い
    let old_unity = connect_pair(
        "oldu",
        standalone(Some(V(0, 2, 0)), V(0, 4, 0), 0),
        unity(Some(V(0, 3, 9)), V(0, 2, 0), 0),
    );
    let s = old_unity.standalone_conn.link_info().unwrap().skew();
    assert_eq!(s.update_peer, Some(V(0, 4, 0)));
    assert!(s.peer_should_update() && !s.own_should_update());
    let u = old_unity.unity_conn.link_info().unwrap().skew();
    assert_eq!(
        u.update_self,
        Some(V(0, 4, 0)),
        "Unity から見ると、自分を 0.4.0 以上に"
    );
    assert!(u.own_should_update() && !u.peer_should_update());

    // スタンドアロンが、Unity のパッケージの求める版より古い
    let old_standalone = connect_pair(
        "olds",
        standalone(Some(V(0, 1, 0)), V(0, 3, 0), 0),
        unity(Some(V(0, 3, 0)), V(0, 2, 0), 0),
    );
    let s = old_standalone.standalone_conn.link_info().unwrap().skew();
    assert_eq!(s.update_self, Some(V(0, 2, 0)));
    let u = old_standalone.unity_conn.link_info().unwrap().skew();
    assert_eq!(u.update_peer, Some(V(0, 2, 0)));
}

#[test]
fn a_new_command_is_sent_only_when_the_peer_has_the_mark() {
    let mut pair = connect_pair(
        "gate",
        standalone(Some(V(0, 1, 0)), V(0, 0, 0), FEATURE_A | FEATURE_B),
        unity(Some(V(0, 3, 0)), V(0, 0, 0), FEATURE_B),
    );
    // 印 A は Unity に無い: 送らない（Ok(false)）。誤りにも切断にもならない
    let removed = |set| Message::TextureSetRemoved { set };
    assert!(!pair
        .standalone_conn
        .send_requiring(FEATURE_A, &removed(1))
        .unwrap());
    // 印 B は双方にある: 送る。印の要らない命令（0）はいつも送る
    assert!(pair
        .standalone_conn
        .send_requiring(FEATURE_B, &removed(2))
        .unwrap());
    assert!(pair.standalone_conn.send_requiring(0, &removed(3)).unwrap());
    // 今ある全部の命令は印を要らない（send_gated はそのまま送る）
    assert!(pair.standalone_conn.send_gated(&removed(4)).unwrap());
    assert!(pair
        .unity_conn
        .send_gated(&Message::ModelClosed { generation: 1 })
        .unwrap());
    // 送らなかった 1 は Unity に届かず、2・3・4 だけが順に届く
    let mut got = Vec::new();
    while got.len() < 3 {
        match pair
            .unity_reader
            .next_within(&pair.unity_conn, Duration::from_secs(5))
            .unwrap()
        {
            Received::Message(Message::TextureSetRemoved { set }) => got.push(set),
            Received::Idle => panic!("届かない: {got:?}"),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(got, vec![2, 3, 4]);
    match pair
        .standalone_reader
        .next_within(&pair.standalone_conn, Duration::from_secs(5))
        .unwrap()
    {
        Received::Message(Message::ModelClosed { generation: 1 }) => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(pair.standalone_conn.common_features(), FEATURE_B);
}

/// 試験用の印の要る表（今の命令は印を要らないので、試験が差し込む）: モデルを閉じるのは A、マテリアルは B、ポーズは A と B。
fn test_need(kind: Kind) -> u64 {
    match kind {
        Kind::ModelClosed => FEATURE_A,
        Kind::Materials => FEATURE_B,
        Kind::Pose => FEATURE_A | FEATURE_B,
        _ => 0,
    }
}

#[test]
fn the_gate_compares_the_needed_marks_with_the_common_part() {
    use yolu_protocol::compat::satisfies;
    let unknown = 1u64 << 50;
    // (共通の印, 要る印, 送ってよいか)
    let table = [
        // 印を要らないものは、共通が空でもいつも送ってよい
        (0, 0, true),
        (FEATURE_A, 0, true),
        // 要る印が立っている
        (FEATURE_A, FEATURE_A, true),
        (FEATURE_A | FEATURE_B, FEATURE_A, true),
        (FEATURE_A | unknown, FEATURE_A, true),
        // 要る印が立っていない（共通が空・別の印だけ・名前を知らない印だけ）
        (0, FEATURE_A, false),
        (FEATURE_B, FEATURE_A, false),
        (unknown, FEATURE_A, false),
        // 要る印が複数なら、全部が要る（片方だけでは送らない）
        (FEATURE_A, FEATURE_A | FEATURE_B, false),
        (FEATURE_B, FEATURE_A | FEATURE_B, false),
        (FEATURE_A | FEATURE_B, FEATURE_A | FEATURE_B, true),
        (u64::MAX, FEATURE_A | FEATURE_B, true),
        // 最上位のビットも同じ
        (1 << 63, 1 << 63, true),
        (1 << 62, 1 << 63, false),
    ];
    for (common, need, expected) in table {
        assert_eq!(
            satisfies(common, need),
            expected,
            "common={common:#x} need={need:#x}"
        );
    }
}

#[test]
fn a_message_is_accepted_by_the_marks_its_kind_needs() {
    use yolu_protocol::compat::accepts_with;
    let closed = Message::ModelClosed { generation: 1 };
    let materials = Message::Materials(MaterialsUpdate {
        generation: 1,
        materials: Vec::new(),
    });
    let pose = Message::Pose(Pose {
        generation: 1,
        meshes: Vec::new(),
    });
    let bye = Message::Bye;
    // 共通が A だけ: A を要るものだけ（B を要るもの・A と B を要るものは送らない。印の要らないものは送る）
    assert!(accepts_with(FEATURE_A, &closed, test_need));
    assert!(!accepts_with(FEATURE_A, &materials, test_need));
    assert!(!accepts_with(FEATURE_A, &pose, test_need));
    assert!(accepts_with(FEATURE_A, &bye, test_need));
    // 共通が B だけ: 逆
    assert!(!accepts_with(FEATURE_B, &closed, test_need));
    assert!(accepts_with(FEATURE_B, &materials, test_need));
    assert!(!accepts_with(FEATURE_B, &pose, test_need));
    // 共通が空: 印の要らないものだけ
    assert!(!accepts_with(0, &closed, test_need));
    assert!(accepts_with(0, &bye, test_need));
    // 共通が A と B: 全部
    for m in [&closed, &materials, &pose, &bye] {
        assert!(accepts_with(FEATURE_A | FEATURE_B, m, test_need));
    }
    // 実際の表（Kind::required_feature）では、今の命令は共通が空でも送ってよい
    assert!(yolu_protocol::compat::accepts(0, &closed));
    assert!(yolu_protocol::compat::accepts(0, &pose));
}

#[test]
fn a_kind_that_needs_a_mark_is_not_sent_without_it_through_the_connection() {
    // スタンドアロンは A と B を出し、Unity は A だけ: 共通は A
    let mut pair = connect_pair(
        "gatewith",
        standalone(Some(V(0, 1, 0)), V(0, 0, 0), FEATURE_A | FEATURE_B),
        unity(Some(V(0, 3, 0)), V(0, 0, 0), FEATURE_A),
    );
    assert_eq!(pair.unity_conn.common_features(), FEATURE_A);
    let removed = |set| Message::TextureSetRemoved { set };
    // 種類ごとの表から印を決めて送る: TextureSetRemoved を A 要り・B 要りとして試す
    let need_a = |k: Kind| if k == Kind::TextureSetRemoved { FEATURE_A } else { 0 };
    let need_b = |k: Kind| if k == Kind::TextureSetRemoved { FEATURE_B } else { 0 };
    let need_ab = |k: Kind| {
        if k == Kind::TextureSetRemoved {
            FEATURE_A | FEATURE_B
        } else {
            0
        }
    };
    assert!(pair
        .standalone_conn
        .send_gated_with(&removed(1), need_a)
        .unwrap());
    assert!(!pair
        .standalone_conn
        .send_gated_with(&removed(2), need_b)
        .unwrap());
    assert!(!pair
        .standalone_conn
        .send_gated_with(&removed(3), need_ab)
        .unwrap());
    // 別の種類は、表が 0 を返すのでいつも送る
    assert!(pair
        .standalone_conn
        .send_gated_with(&Message::ModelClosed { generation: 9 }, need_ab)
        .unwrap());
    // 届くのは送った 1 と、モデルを閉じる命令だけ
    match pair
        .unity_reader
        .next_within(&pair.unity_conn, Duration::from_secs(5))
        .unwrap()
    {
        Received::Message(Message::TextureSetRemoved { set: 1 }) => {}
        other => panic!("{other:?}"),
    }
    match pair
        .unity_reader
        .next_within(&pair.unity_conn, Duration::from_secs(5))
        .unwrap()
    {
        Received::Message(Message::ModelClosed { generation: 9 }) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn every_command_of_today_needs_no_mark() {
    use yolu_protocol::compat::accepts;
    let all = [
        Kind::Hello,
        Kind::Bye,
        Kind::Model,
        Kind::Pose,
        Kind::Materials,
        Kind::ModelClosed,
        Kind::Welcome,
        Kind::Reject,
        Kind::TextureSet,
        Kind::TextureSetRemoved,
        Kind::TilesChanged,
        Kind::Error,
    ];
    for k in all {
        assert_eq!(k.required_feature(), 0, "{k:?}");
    }
    assert!(accepts(0, &Message::Bye));
}

#[test]
fn a_protocol_range_that_does_not_overlap_is_refused_with_which_side_to_update() {
    // Unity が新しすぎる（読める版が先）: スタンドアロンを上げる。版は Unity の挨拶が求める版
    let own = Identity::standalone("s").with_min_peer(V(0, 3, 0));
    let future = Hello {
        min_version: PROTOCOL_VERSION + 1,
        max_version: PROTOCOL_VERSION + 2,
        agent: "未来の Unity".into(),
        features: 0,
        auth: None,
        versions: Some(VersionInfo {
            app: V(0, 9, 0),
            min_peer: V(0, 5, 0),
        }),
    };
    let reject = negotiate_as(&own, &future).unwrap_err();
    assert_eq!(reject.code, RejectCode::VersionMismatch);
    assert!(
        reject
            .text
            .contains("スタンドアロンを 0.5.0 以上に上げる必要があります。"),
        "{}",
        reject.text
    );
    assert!(
        reject
            .text
            .contains("The standalone must be 0.5.0 or newer."),
        "{}",
        reject.text
    );
    assert!(reject.text.contains(&format!(
        "Unity 側 {}〜{}",
        PROTOCOL_VERSION + 1,
        PROTOCOL_VERSION + 2
    )));
    // 断りの文は理由の形（「〜を上げる必要があります / must be」）で、指示の文（ください・Update）にしない
    assert!(
        !reject.text.contains("ください") && !reject.text.contains("Update "),
        "{}",
        reject.text
    );
    let detail = reject.detail.expect("断りの詳しい欄");
    assert_eq!(
        (detail.min_version, detail.max_version, detail.min_peer),
        (MIN_PROTOCOL_VERSION, PROTOCOL_VERSION, V(0, 3, 0))
    );
    // 断られた側の言った範囲と求める版も入る（この欄だけで、どちらを上げるかが決まる）
    assert_eq!(
        (
            detail.peer_min_version,
            detail.peer_max_version,
            detail.peer_min_peer
        ),
        (PROTOCOL_VERSION + 1, PROTOCOL_VERSION + 2, V(0, 5, 0))
    );
    let from_detail = refusal_from_reject(Product::Standalone, &reject).unwrap();
    assert_eq!(
        (from_detail.update, from_detail.to),
        (Product::Standalone, Some(V(0, 5, 0)))
    );

    // Unity が古すぎる: Unity のパッケージを、スタンドアロンの求める版以上に
    let old = Hello {
        min_version: 0,
        max_version: 0,
        agent: "古い Unity".into(),
        features: 0,
        auth: None,
        versions: None,
    };
    let reject = negotiate_as(&own, &old).unwrap_err();
    assert!(
        reject
            .text
            .contains("Unity のパッケージを 0.3.0 以上に上げる必要があります。"),
        "{}",
        reject.text
    );
    assert!(
        reject
            .text
            .contains("The Unity package must be 0.3.0 or newer."),
        "{}",
        reject.text
    );

    // 求める版が決まっていなければ、版を添えず更新を促す
    let reject = negotiate_as(&Identity::standalone("s"), &old).unwrap_err();
    assert!(
        reject.text.contains("Unity のパッケージを更新する必要があります。"),
        "{}",
        reject.text
    );
    assert!(
        reject
            .text
            .contains("The Unity package must be updated."),
        "{}",
        reject.text
    );
    // 既定の（版を名乗らない）negotiate は今までどおり断る
    assert_eq!(
        link::negotiate(&old).unwrap_err().code,
        RejectCode::VersionMismatch
    );
    assert_eq!(
        link::negotiate(&Hello {
            min_version: 1,
            max_version: 1,
            ..old.clone()
        }),
        Ok(1)
    );

    // 断りの欄だけで、どちらを何版以上に上げるかを組み立てられる（断られた側も、断った側も同じ答え）
    let newer_standalone = Reject {
        code: RejectCode::VersionMismatch,
        text: "x".into(),
        detail: Some(RejectDetail {
            min_version: PROTOCOL_VERSION + 1,
            max_version: PROTOCOL_VERSION + 1,
            min_peer: V(0, 6, 0),
            peer_min_version: MIN_PROTOCOL_VERSION,
            peer_max_version: PROTOCOL_VERSION,
            peer_min_peer: V(0, 1, 0),
        }),
    };
    // 断ったのは新しいスタンドアロン（読めるのは先の版）: Unity のパッケージを、スタンドアロンの求める版（0.6.0）へ
    let r = refusal_from_reject(Product::Standalone, &newer_standalone).unwrap();
    assert_eq!((r.update, r.to), (Product::Unity, Some(V(0, 6, 0))));
    assert_eq!(
        (r.unity_range, r.standalone_range),
        (
            (MIN_PROTOCOL_VERSION, PROTOCOL_VERSION),
            (PROTOCOL_VERSION + 1, PROTOCOL_VERSION + 1)
        )
    );
    let older_standalone = Reject {
        detail: Some(RejectDetail {
            min_version: 0,
            max_version: 0,
            min_peer: V(0, 0, 0),
            peer_min_version: 1,
            peer_max_version: 1,
            peer_min_peer: V(0, 2, 0),
        }),
        ..newer_standalone.clone()
    };
    // 断ったのは古いスタンドアロン: 断られた Unity の求める版（0.2.0）以上に、スタンドアロンを上げる
    let r = refusal_from_reject(Product::Standalone, &older_standalone).unwrap();
    assert_eq!((r.update, r.to), (Product::Standalone, Some(V(0, 2, 0))));
    assert!(r
        .text()
        .contains("スタンドアロンを 0.2.0 以上に上げる必要があります。"));
    // 古い相手の断り（詳しい欄なし）・版の断りでないものは None
    assert_eq!(
        refusal_from_reject(
            Product::Standalone,
            &Reject::plain(RejectCode::VersionMismatch, "古い")
        ),
        None
    );
    assert_eq!(
        refusal_from_reject(
            Product::Standalone,
            &Reject {
                code: RejectCode::Busy,
                ..newer_standalone
            }
        ),
        None
    );
    let me = Identity::unity("u")
        .with_min_peer(V(0, 1, 0))
        .with_version(Some(V(0, 3, 0)));
    // 重なる範囲は断らない
    assert_eq!(judge_ranges(&me, (1, 1), None), None);
}

#[test]
fn a_refusal_for_the_version_range_reaches_the_bridge_with_its_detail() {
    let name = unique_name("refuse");
    let listener = Server::bind(&name, false).unwrap();
    let key = LinkKey::load(&name).unwrap();
    let server = thread::spawn(move || {
        let stream = listener.accept().unwrap();
        let own = Identity::standalone("s").with_min_peer(V(0, 3, 0));
        let r = accept_as(
            stream,
            &own,
            1,
            &listener.key(),
            link::HANDSHAKE_TIMEOUT,
            &|_| Ok(()),
        );
        (matches!(r, Err(link::LinkError::Rejected(_))), listener)
    });
    // 鍵を知っていて、版の範囲だけが合わない生の挨拶を送り、断りを読む
    use std::io::Write;
    let stream = link::connect(&name).unwrap();
    let mut s = &stream;
    let nonce = yolu_protocol::auth::random_bytes().unwrap();
    s.write_all(&encode_message(&Message::Hello(Hello {
        min_version: PROTOCOL_VERSION + 1,
        max_version: PROTOCOL_VERSION + 1,
        agent: "未来の Unity".into(),
        features: 0,
        auth: Some(HelloAuth {
            nonce,
            proof: key.hello_proof(&nonce),
        }),
        versions: Some(VersionInfo {
            app: V(1, 0, 0),
            min_peer: V(0, 7, 0),
        }),
    })))
    .unwrap();
    let mut frames = FrameReader::new();
    let reply = frames
        .read_frame(&mut s)
        .unwrap()
        .unwrap()
        .decode()
        .unwrap();
    let Message::Reject(r) = reply else {
        panic!("{reply:?}")
    };
    assert_eq!(r.code, RejectCode::VersionMismatch);
    assert_eq!(r.detail.map(|d| d.min_peer), Some(V(0, 3, 0)));
    assert!(server.join().unwrap().0);
    // 版の範囲の断りだけが詳しい欄を持ち、鍵の断りは版の範囲を教えない
    assert_eq!(Reject::plain(RejectCode::Unauthorized, "鍵").detail, None);
}

#[test]
fn the_identity_decides_which_fields_go_on_the_wire() {
    // 版を知らない名乗りは版の欄を送らない。知っていれば自分の版と求める版の両方
    assert_eq!(Identity::unity("u").version_info(), None);
    let id = Identity::unity("u").with_version(Some(V(1, 2, 3)));
    assert_eq!(
        id.version_info(),
        Some(VersionInfo {
            app: V(1, 2, 3),
            min_peer: MIN_STANDALONE
        })
    );
    assert_eq!(Identity::standalone("s").min_peer, MIN_UNITY_PACKAGE);
}
