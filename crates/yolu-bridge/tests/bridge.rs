//! C の口をそのまま呼ぶ試験（C# と同じ使い方）。相手は同じプロセスの中の自己診断のスタンドアロン（本物のソケットと共有メモリ）。

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use yolu_bridge::*;

fn unique_name(tag: &str) -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    format!(
        "ylb-test-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

fn wait_for(h: u64, what: &str, mut f: impl FnMut() -> bool) {
    // 本物のソケットとバックグラウンド受信には外部のスケジューリングがある。上限はハング検出用。
    let deadline = Instant::now() + Duration::from_secs(120);
    while !f() {
        let status = ylb_status(h);
        if !matches!(status, 0 | 1) && f() {
            return;
        }
        assert!(
            Instant::now() < deadline && matches!(status, 0 | 1),
            "{what} を待ったが来ない: 接続状態={status}, 受信番号={}, セット数={}, 知らせ={:?}",
            ylb_serial(h),
            ylb_set_count(h),
            events(h)
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn events(h: u64) -> Vec<(i32, String)> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 1024];
    loop {
        let mut e = YlbEvent::default();
        let r = unsafe { ylb_next_event(h, &mut e, buf.as_mut_ptr(), buf.len() as i32) };
        if r != 1 {
            return out;
        }
        out.push((
            e.kind,
            String::from_utf8(buf[..e.text_len as usize].to_vec()).unwrap(),
        ));
    }
}

fn connect(name: &str) -> u64 {
    let agent = "試験";
    let h = unsafe {
        ylb_connect(
            name.as_ptr(),
            name.len() as i32,
            agent.as_ptr(),
            agent.len() as i32,
        )
    };
    assert_ne!(h, 0);
    h
}

/// 四角 1 つ（2 つの三角形）を 2 つのマテリアルで描くモデルを送る。
fn send_quad_model(h: u64) -> i32 {
    unsafe {
        let name = "四角";
        assert_eq!(ylb_model_begin(h, name.as_ptr(), name.len() as i32), 0);
        for (i, m) in ["Body", "Hair"].iter().enumerate() {
            let guid = "0123456789abcdef0123456789abcdef";
            let shader = "Standard";
            let idx = ylb_model_material(
                h,
                0,
                m.as_ptr(),
                m.len() as i32,
                guid.as_ptr(),
                32,
                i as i64 + 1,
                shader.as_ptr(),
                shader.len() as i32,
            );
            assert_eq!(idx, i as i32);
            let prop = "_MainTex";
            assert_eq!(
                ylb_model_material_texture(h, idx, prop.as_ptr(), prop.len() as i32, 1024, 1024),
                0
            );
            assert_eq!(
                ylb_model_material_route(h, idx, 0, prop.as_ptr(), prop.len() as i32),
                0
            );
        }
        let pos: [f32; 12] = [0., 0., 0., 1., 0., 0., 0., 1., 0., 1., 1., 0.];
        let nrm: [f32; 12] = [0., 0., -1., 0., 0., -1., 0., 0., -1., 0., 0., -1.];
        let uv: [f32; 8] = [0., 0., 1., 0., 0., 1., 1., 1.];
        let (key, mname) = ("0", "Quad");
        let mesh = ylb_model_mesh(
            h,
            key.as_ptr(),
            1,
            mname.as_ptr(),
            4,
            0,
            pos.as_ptr(),
            nrm.as_ptr(),
            uv.as_ptr(),
            4,
        );
        assert_eq!(mesh, 0);
        let a: [i32; 3] = [0, 2, 1];
        let b: [i32; 3] = [1, 2, 3];
        assert_eq!(ylb_model_submesh(h, mesh, 0, a.as_ptr(), 3), 0);
        assert_eq!(ylb_model_submesh(h, mesh, 1, b.as_ptr(), 3), 0);
        // 範囲外の添字・3 の倍数でない数・無いマテリアルは断る
        let bad: [i32; 3] = [0, 1, 4];
        assert_eq!(
            ylb_model_submesh(h, mesh, 0, bad.as_ptr(), 3),
            YLB_E_ARGUMENT
        );
        assert_eq!(
            ylb_model_submesh(h, mesh, 0, bad.as_ptr(), 2),
            YLB_E_ARGUMENT
        );
        assert_eq!(ylb_model_submesh(h, mesh, 5, a.as_ptr(), 3), YLB_E_ARGUMENT);
        let generation = ylb_model_send(h);
        assert!(generation >= 1);
        generation
    }
}

fn sets(h: u64) -> Vec<YlbSetInfo> {
    (0..ylb_set_count(h).max(0))
        .map(|i| {
            let mut info = YlbSetInfo::default();
            assert_eq!(unsafe { ylb_set_info(h, i, &mut info) }, 0);
            info
        })
        .collect()
}

#[test]
fn the_version_is_asked_first() {
    assert_eq!(ylb_abi_version(), 1);
    assert_eq!(ylb_protocol_versions(), (1 << 16) | 1);
}

#[test]
fn a_model_comes_back_as_patterned_texture_sets_and_single_tiles_update() {
    let name = unique_name("full");
    let server = unsafe { ylb_test_server_start(name.as_ptr(), name.len() as i32, 512, 128) };
    assert_ne!(server, 0);
    let h = connect(&name);
    wait_for(h, "つながり", || ylb_status(h) == 1);
    let before_model = ylb_serial(h);
    let generation = send_quad_model(h);
    wait_for(h, "2 つのテクスチャセット", || sets(h).len() == 2);
    // TextureSet だけでも全タイルが dirty になる。各セットの初回 TilesChanged も
    // 受信してからコピーしないと、遅れて来た初回通知が1タイルの更新に混ざる。
    wait_for(
        h,
        "2セットの追加と初回タイル通知（計4件）",
        || ylb_serial(h) >= before_model + 4,
    );
    assert_eq!(ylb_serial(h), before_model + 4, "余分なエラー通知がない");
    let all = sets(h);
    let mut images = Vec::new();
    for (i, s) in all.iter().enumerate() {
        assert_eq!(
            (s.generation, s.material, s.width, s.height, s.tile_size),
            (generation as u32, i as u32, 512, 512, 128)
        );
        assert_eq!(s.channel_mask, 1, "Color だけ");
        // 知らせた時の中身は全部が汚れている
        wait_for(h, "汚れたタイル", || {
            ylb_channel_dirty(h, s.set, 0) == 16
        });
        let mut image = vec![0u8; 512 * 512 * 4];
        let mut r = YlbCopyResult::default();
        let n = unsafe {
            ylb_copy_dirty(
                h,
                s.set,
                0,
                image.as_mut_ptr(),
                image.len() as u64,
                std::ptr::null_mut(),
                0,
                0,
                std::ptr::null_mut(),
                &mut r,
            )
        };
        assert_eq!((n, r.tiles, r.remaining, r.torn), (16, 16, 0, 0));
        assert_eq!((r.x_min, r.y_min, r.x_max, r.y_max), (0, 0, 512, 512));
        for (tx, ty) in [(0u32, 0u32), (1, 0), (3, 3), (2, 1)] {
            let p = ((ty * 128 + 5) as usize * 512 + (tx * 128 + 7) as usize) * 4;
            let expected = ylb_test_server_pattern(s.material, tx, ty).to_le_bytes();
            assert_eq!(
                image[p..p + 4],
                expected,
                "セット {} のタイル ({tx}, {ty})",
                s.material
            );
        }
        images.push(image);
    }
    let mut buf = [0u8; 64];
    let len = unsafe { ylb_set_name(h, all[1].set, buf.as_mut_ptr(), 64) };
    assert_eq!(std::str::from_utf8(&buf[..len as usize]).unwrap(), "Hair");

    // 1 タイルだけ塗ると、そのタイルだけが帯で届く
    assert_eq!(
        ylb_test_server_paint(server, 1, 0, 2, 3, 3, 4, 0xff00_ff00),
        1
    );
    wait_for(h, "1 タイルの知らせ", || {
        ylb_channel_dirty(h, all[1].set, 0) == 1
    });
    let mut strip = vec![0u8; 4 * 128 * 128 * 4];
    let mut coords = [0u32; 8];
    let mut r = YlbCopyResult::default();
    let n = unsafe {
        ylb_copy_dirty(
            h,
            all[1].set,
            0,
            images[1].as_mut_ptr(),
            images[1].len() as u64,
            strip.as_mut_ptr(),
            strip.len() as u64,
            4,
            coords.as_mut_ptr(),
            &mut r,
        )
    };
    assert_eq!(n, 1);
    assert_eq!(&coords[..2], &[2, 3]);
    assert_eq!((r.x_min, r.y_min, r.x_max, r.y_max), (256, 384, 384, 512));
    assert!(r.stamp_us > 0 && r.received_us >= r.stamp_us);
    assert_eq!(strip[0..4], [0, 255, 0, 255]);
    let p = ((3 * 128 + 1) * 512 + 2 * 128 + 1) * 4;
    assert_eq!(images[1][p..p + 4], [0, 255, 0, 255]);
    // 隣のタイルは変わらない
    let q = ((3 * 128 + 1) * 512 + 128 + 1) * 4;
    assert_eq!(
        images[1][q..q + 4],
        ylb_test_server_pattern(1, 1, 3).to_le_bytes()
    );

    // ポーズ（頂点の数が違うものは断る）
    let pos: [f32; 12] = [0.5, 0.25, 0.125, 1., 0., 0., 0., 1., 0., 1., 1., 0.];
    assert_eq!(ylb_pose_begin(h), 0);
    assert_eq!(
        unsafe { ylb_pose_mesh(h, 0, pos.as_ptr(), std::ptr::null(), 3) },
        YLB_E_ARGUMENT
    );
    assert_eq!(
        unsafe { ylb_pose_mesh(h, 1, pos.as_ptr(), std::ptr::null(), 4) },
        YLB_E_ARGUMENT
    );
    assert_eq!(
        unsafe { ylb_pose_mesh(h, 0, pos.as_ptr(), std::ptr::null(), 4) },
        0
    );
    assert_eq!(ylb_pose_send(h), 1);
    wait_for(h, "ポーズ", || {
        let mut st = YlbTestServerStats::default();
        unsafe { ylb_test_server_stats(server, &mut st) };
        st.poses == 1
    });
    let mut st = YlbTestServerStats::default();
    unsafe { ylb_test_server_stats(server, &mut st) };
    assert_eq!(
        (st.models, st.materials, st.meshes, st.vertices),
        (1, 2, 1, 4)
    );
    assert_eq!(
        (st.last_pose_x, st.last_pose_y, st.last_pose_z),
        (0.5, 0.25, 0.125)
    );

    let ev = events(h);
    assert_eq!(ev.iter().filter(|e| e.0 == 5).count(), 2, "{ev:?}");
    assert!(ev.iter().any(|e| e.0 == 1));

    // モデルを閉じるとセットが消える
    assert_eq!(ylb_model_close(h), 0);
    wait_for(h, "セットが消える", || sets(h).is_empty());

    assert_eq!(ylb_disconnect(h), 0);
    assert_eq!(ylb_status(h), YLB_E_HANDLE);
    assert_eq!(ylb_disconnect(h), YLB_E_HANDLE);
    assert_eq!(ylb_test_server_stop(server), 0);
}

#[test]
fn a_missing_standalone_reports_failure() {
    let name = unique_name("none");
    let h = connect(&name);
    wait_for(h, "失敗", || ylb_status(h) == 3);
    let ev = events(h);
    assert!(ev.iter().any(|e| e.0 == 3), "{ev:?}");
    // つながっていないので送れない
    let n = "x";
    unsafe { assert_eq!(ylb_model_begin(h, n.as_ptr(), 1), 0) };
    assert_eq!(ylb_model_send(h), YLB_E_STATE);
    assert_eq!(ylb_disconnect(h), 0);
}

#[test]
fn bad_names_and_unknown_handles_are_refused() {
    let bad = "名前/パス";
    assert_eq!(
        unsafe { ylb_connect(bad.as_ptr(), bad.len() as i32, bad.as_ptr(), 0) },
        0
    );
    assert_eq!(
        unsafe { ylb_connect(std::ptr::null(), 3, std::ptr::null(), 0) },
        0
    );
    assert_eq!(ylb_status(987654321), YLB_E_HANDLE);
    assert_eq!(ylb_set_count(987654321), YLB_E_HANDLE);
    assert_eq!(ylb_pose_begin(987654321), YLB_E_HANDLE);
}

#[test]
fn connect_returns_before_the_peer_sends_its_welcome() {
    use interprocess::local_socket::traits::Listener as _;
    use yolu_protocol::link;
    let name = unique_name("pending");
    let listener = link::listen(&name).unwrap();
    // 返るまでは accept も Welcome も実行しない。同期の挨拶待ちなら
    // 挨拶の時間切れで失敗し、Connecting のまま返るという条件を満たせない。
    let h = connect(&name);
    assert_eq!(ylb_status(h), 0, "相手の挨拶より先に返る");
    let stream = listener.accept().unwrap();
    let (conn, _reader, _) = link::accept(stream, "試験", 1).unwrap();
    wait_for(h, "保留していた挨拶の完了", || {
        ylb_status(h) == 1
    });
    assert_eq!(ylb_disconnect(h), 0);
    drop(conn);
}
