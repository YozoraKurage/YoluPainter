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

/// 汚れたタイルを全部写し、新しい知らせが少しのあいだ来なくなるまで繰り返す（セットを知らせた直後の、全タイルの TilesChanged が遅れて着いても、
/// 後の「塗った 1 タイルだけが汚れる」の確かめに混ざらないように）。
fn settle(h: u64, set: u32, image: &mut [u8]) {
    let mut quiet = Instant::now();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(Instant::now() < deadline, "知らせが落ち着かない");
        if ylb_channel_dirty(h, set, 0) > 0 {
            let mut r = YlbCopyResult::default();
            let n = unsafe {
                ylb_copy_dirty(
                    h,
                    set,
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
            assert!(n >= 0);
            quiet = Instant::now();
        } else if quiet.elapsed() > Duration::from_millis(250) {
            return;
        }
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

/// セットの一覧。数えてから 1 つずつ読む間に受信側がモデルを入れ替えると、読む番号が範囲外（YLB_E_ARGUMENT）になる。
/// 一覧が替わった読みは捨てて、揃って読めるまで数え直す。試験の補助だけがやり直す: Unity 側の SyncSets（Editor/LiveLink/LiveLinkDisplay.cs）は
/// 読めなかった番号を飛ばして続け、読めなかったセットを「見えた」に入れないまま、見えなかったセットを解放する（同じ競合が製品側にある）。
fn sets(h: u64) -> Vec<YlbSetInfo> {
    for _ in 0..100 {
        let mut all = Vec::new();
        let mut torn = false;
        for i in 0..ylb_set_count(h).max(0) {
            let mut info = YlbSetInfo::default();
            match unsafe { ylb_set_info(h, i, &mut info) } {
                0 => all.push(info),
                YLB_E_ARGUMENT => {
                    torn = true;
                    break;
                }
                other => panic!("ylb_set_info({i}) が {other}"),
            }
        }
        if !torn {
            return all;
        }
        std::thread::yield_now();
    }
    panic!("セットの一覧が読む間に替わり続ける");
}

#[test]
fn the_version_is_asked_first() {
    assert_eq!(ylb_abi_version(), 5);
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
        settle(h, s.set, &mut image);
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

    // 帯だけで受ける（全体の CPU の写しは持たない）。作り直し・失ったときの全面の写し直しも頼める
    assert_eq!(ylb_channel_mark_all_dirty(h, all[0].set, 0), 16);
    assert_eq!(ylb_channel_dirty(h, all[0].set, 0), 16);
    let mut strip = vec![0u8; 8 * 128 * 128 * 4];
    let mut coords = [0u32; 16];
    let mut got = Vec::new();
    for _ in 0..2 {
        let mut r = YlbCopyResult::default();
        let n = unsafe {
            ylb_copy_dirty(
                h,
                all[0].set,
                0,
                std::ptr::null_mut(),
                0,
                strip.as_mut_ptr(),
                strip.len() as u64,
                8,
                coords.as_mut_ptr(),
                &mut r,
            )
        };
        assert_eq!(n, 8, "帯の数まで写して止める");
        for i in 0..n as usize {
            let (x, y) = (coords[i * 2], coords[i * 2 + 1]);
            let expected = ylb_test_server_pattern(all[0].material, x, y).to_le_bytes();
            assert_eq!(strip[i * 128 * 4..i * 128 * 4 + 4], expected, "帯の {i} 番");
            got.push((x, y));
        }
    }
    got.sort();
    assert_eq!(got.len(), 16);
    assert_eq!(
        got.windows(2).filter(|w| w[0] == w[1]).count(),
        0,
        "重ならず全部"
    );
    assert_eq!(ylb_channel_dirty(h, all[0].set, 0), 0);
    // 写し先が無い・帯だけで image も無い、は断る
    assert_eq!(
        unsafe {
            ylb_copy_dirty(
                h,
                all[0].set,
                0,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        },
        YLB_E_ARGUMENT
    );

    // マテリアルの更新（数が違うものは断る。シェーダーと流し込み先を変えて送る）
    assert_eq!(ylb_materials_send(h), YLB_E_STATE, "組み立てる前は送れない");
    assert_eq!(ylb_materials_begin(h), 0);
    unsafe {
        let shader = "Custom/Other";
        for (i, m) in ["Body", "Hair"].iter().enumerate() {
            let idx = ylb_materials_material(
                h,
                0,
                m.as_ptr(),
                m.len() as i32,
                std::ptr::null(),
                0,
                0,
                shader.as_ptr(),
                shader.len() as i32,
            );
            assert_eq!(idx, i as i32);
            let prop = "_BaseMap";
            assert_eq!(
                ylb_materials_texture(h, idx, prop.as_ptr(), prop.len() as i32, 64, 64),
                0
            );
            assert_eq!(
                ylb_materials_route(h, idx, 0, prop.as_ptr(), prop.len() as i32),
                0
            );
            assert_eq!(
                ylb_materials_route(h, idx, 0, prop.as_ptr(), prop.len() as i32),
                YLB_E_ARGUMENT,
                "同じチャンネルは 2 度足せない"
            );
            assert_eq!(
                ylb_materials_texture(h, 9, prop.as_ptr(), prop.len() as i32, 1, 1),
                YLB_E_ARGUMENT,
                "無いマテリアル"
            );
        }
        // 3 つ目を足すと、送ったモデルの数（2）と違うので断る
        let extra = "Extra";
        assert_eq!(
            ylb_materials_material(
                h,
                0,
                extra.as_ptr(),
                5,
                std::ptr::null(),
                0,
                0,
                std::ptr::null(),
                0
            ),
            2
        );
    }
    assert_eq!(ylb_materials_send(h), YLB_E_ARGUMENT);
    // 断られたら組み立ては捨てられている（もう一度 begin から）
    assert_eq!(ylb_materials_send(h), YLB_E_STATE);
    assert_eq!(ylb_materials_begin(h), 0);
    unsafe {
        let shader = "Custom/Other";
        for (i, m) in ["Body", "Hair"].iter().enumerate() {
            let idx = ylb_materials_material(
                h,
                0,
                m.as_ptr(),
                m.len() as i32,
                std::ptr::null(),
                0,
                0,
                shader.as_ptr(),
                shader.len() as i32,
            );
            assert_eq!(idx, i as i32);
            let prop = "_BaseMap";
            assert_eq!(
                ylb_materials_route(h, idx, 0, prop.as_ptr(), prop.len() as i32),
                0
            );
        }
    }
    assert_eq!(ylb_materials_send(h), 2);
    wait_for(h, "マテリアルの更新", || {
        let mut st = YlbTestServerStats::default();
        unsafe { ylb_test_server_stats(server, &mut st) };
        st.materials_updates == 1
    });
    let mut st = YlbTestServerStats::default();
    unsafe { ylb_test_server_stats(server, &mut st) };
    assert_eq!(
        (
            st.last_materials_count,
            st.last_materials_routes,
            st.last_materials_shader_len
        ),
        (2, 2, "Custom/Other".len() as u32)
    );
    assert_eq!(st.refused, 0);

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

/// 幅・高さがタイルの倍数でないテクスチャ（200² をタイル 64 で。右端・上端のタイルは 8 画素ぶんだけが画像の中）。帯には端のタイルも
/// ts × ts の全体が並び（C# が画像の中の幅・高さだけを置く）、全体の写しには画像の外へはみ出さずに入る。
#[test]
fn edge_tiles_of_a_texture_that_is_not_a_multiple_of_the_tile_arrive_in_a_strip_and_in_the_image() {
    let name = unique_name("edge");
    let server = unsafe { ylb_test_server_start(name.as_ptr(), name.len() as i32, 200, 64) };
    assert_ne!(server, 0);
    let h = connect(&name);
    wait_for(h, "つながり", || ylb_status(h) == 1);
    send_quad_model(h);
    wait_for(h, "2 つのテクスチャセット", || sets(h).len() == 2);
    let s = sets(h)[0];
    assert_eq!((s.width, s.height, s.tile_size), (200, 200, 64));
    wait_for(h, "汚れたタイル", || {
        ylb_channel_dirty(h, s.set, 0) == 16
    });
    let (mut strip, mut coords) = (vec![0u8; 4 * 64 * 64 * 4], [0u32; 8]);
    let mut seen = Vec::new();
    let mut bbox = [u32::MAX, u32::MAX, 0, 0];
    for _ in 0..4 {
        let mut r = YlbCopyResult::default();
        let n = unsafe {
            ylb_copy_dirty(
                h,
                s.set,
                0,
                std::ptr::null_mut(),
                0,
                strip.as_mut_ptr(),
                strip.len() as u64,
                4,
                coords.as_mut_ptr(),
                &mut r,
            )
        };
        assert_eq!(n, 4);
        bbox = [
            bbox[0].min(r.x_min),
            bbox[1].min(r.y_min),
            bbox[2].max(r.x_max),
            bbox[3].max(r.y_max),
        ];
        for i in 0..4usize {
            let (x, y) = (coords[i * 2], coords[i * 2 + 1]);
            let expected = ylb_test_server_pattern(s.material, x, y).to_le_bytes();
            // 帯の i 番目のタイルの 4 隅（端のタイルも、画像の外の所まで ts × ts の全体が同じ色で並ぶ）
            for (px, py) in [(0usize, 0usize), (63, 0), (0, 63), (63, 63)] {
                let p = (py * 4 * 64 + i * 64 + px) * 4;
                assert_eq!(
                    strip[p..p + 4],
                    expected,
                    "タイル ({x}, {y}) の ({px}, {py})"
                );
            }
            seen.push((x, y));
        }
    }
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 16, "全部のタイルが 1 度ずつ");
    assert_eq!(bbox, [0, 0, 200, 200], "写した範囲は画像の中だけ");
    assert_eq!(ylb_channel_dirty(h, s.set, 0), 0);

    // 端のタイルだけを塗ると、帯にそのタイルだけが来る（右端・上端・角）
    for (tx, ty) in [(3u32, 1u32), (1, 3), (3, 3)] {
        assert_eq!(
            ylb_test_server_paint(server, s.material, 0, tx, ty, tx + 1, ty + 1, 0xff10_20f0),
            1
        );
        wait_for(h, "端のタイルの知らせ", || {
            ylb_channel_dirty(h, s.set, 0) == 1
        });
        let mut r = YlbCopyResult::default();
        let n = unsafe {
            ylb_copy_dirty(
                h,
                s.set,
                0,
                std::ptr::null_mut(),
                0,
                strip.as_mut_ptr(),
                strip.len() as u64,
                4,
                coords.as_mut_ptr(),
                &mut r,
            )
        };
        assert_eq!((n, coords[0], coords[1]), (1, tx, ty));
        assert_eq!(
            (r.x_min, r.y_min, r.x_max, r.y_max),
            (
                tx * 64,
                ty * 64,
                (tx * 64 + 64).min(200),
                (ty * 64 + 64).min(200)
            )
        );
        for (px, py) in [(0usize, 0usize), (63, 0), (0, 63), (63, 63)] {
            let p = (py * 4 * 64 + px) * 4;
            assert_eq!(strip[p..p + 4], [0xf0, 0x20, 0x10, 0xff]);
        }
    }

    // 全体の写し（200 × 200）: 端のタイルは画像の中の所だけが入り、隣のタイルの画素を壊さない
    assert_eq!(ylb_channel_mark_all_dirty(h, s.set, 0), 16);
    let mut image = vec![0u8; 200 * 200 * 4];
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
    assert_eq!(n, 16);
    assert_eq!((r.x_min, r.y_min, r.x_max, r.y_max), (0, 0, 200, 200));
    let at = |x: usize, y: usize| -> [u8; 4] {
        let p = (y * 200 + x) * 4;
        image[p..p + 4].try_into().unwrap()
    };
    // 塗ったタイル（3, 1）・（1, 3）・（3, 3）は塗った色、ほかは模様
    let painted = [0xf0, 0x20, 0x10, 0xff];
    let expected = |tx: u32, ty: u32| {
        if [(3, 1), (1, 3), (3, 3)].contains(&(tx, ty)) {
            painted
        } else {
            ylb_test_server_pattern(s.material, tx, ty).to_le_bytes()
        }
    };
    // タイルの境目の両側、右端（x = 192..200）と上端（y = 192..200）、角
    for (x, y) in [
        (191usize, 5usize),
        (192, 5),
        (199, 5),
        (199, 69),
        (192, 63),
        (192, 64),
        (5, 191),
        (5, 192),
        (5, 199),
        (69, 199),
        (199, 199),
        (191, 191),
        (192, 191),
        (191, 192),
    ] {
        let (tx, ty) = ((x / 64) as u32, (y / 64) as u32);
        assert_eq!(
            at(x, y),
            expected(tx, ty),
            "画素 ({x}, {y}) はタイル ({tx}, {ty})"
        );
    }
    assert_eq!(ylb_disconnect(h), 0);
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
    let name = unique_name("pending");
    let server = yolu_protocol::Server::bind(&name, false).unwrap();
    // 返るまでは accept も Welcome も実行しない。同期の挨拶待ちなら
    // 挨拶の時間切れで失敗し、Connecting のまま返るという条件を満たせない。
    let h = connect(&name);
    assert_eq!(ylb_status(h), 0, "相手の挨拶より先に返る");
    let stream = server.accept().unwrap();
    let (conn, _reader, _) =
        yolu_protocol::link::accept(stream, "試験", 1, &server.key()).unwrap();
    wait_for(h, "保留していた挨拶の完了", || {
        ylb_status(h) == 1
    });
    assert_eq!(ylb_disconnect(h), 0);
    drop(conn);
}

#[test]
fn a_bridge_with_a_key_the_standalone_does_not_know_is_rejected_with_the_reason() {
    // 鍵のファイルが別の待ち受けの（古い）ものになっている状態
    let name = unique_name("stale");
    let server = yolu_protocol::Server::bind(&name, false).unwrap();
    yolu_protocol::LinkKey::generate()
        .unwrap()
        .write_file(&yolu_protocol::auth::key_path(&name).unwrap())
        .unwrap();
    let key = server.key();
    let handle = std::thread::spawn(move || {
        let stream = server.accept().unwrap();
        yolu_protocol::link::accept(stream, "試験のスタンドアロン", 1, &key).map(|_| ())
    });
    let h = connect(&name);
    wait_for(h, "失敗", || ylb_status(h) == 3);
    let ev = events(h);
    let rejected = ev
        .iter()
        .find(|e| e.0 == 2)
        .unwrap_or_else(|| panic!("断りの知らせが来ない: {ev:?}"));
    assert!(rejected.1.contains("鍵"), "理由に鍵が入る: {}", rejected.1);
    assert!(matches!(
        handle.join().unwrap(),
        Err(yolu_protocol::LinkError::Rejected(r)) if r.code == yolu_protocol::RejectCode::Unauthorized
    ));
    assert_eq!(ylb_disconnect(h), 0);
}

// ───────── 互いの版と機能の印 ─────────

fn pack(v: Option<yolu_protocol::AppVersion>) -> u64 {
    yolu_protocol::compat::pack_version(v)
}

fn v(major: u16, minor: u16, patch: u16) -> Option<yolu_protocol::AppVersion> {
    Some(yolu_protocol::AppVersion::new(major, minor, patch))
}

/// 版を名乗って（ylb_connect_with）、設定した名乗りの自己診断のスタンドアロンにつなぎ、つながった後の様子を返す。
fn report_against(
    tag: &str,
    own_version: &str,
    server_version: Option<yolu_protocol::AppVersion>,
    server_min_peer: Option<yolu_protocol::AppVersion>,
    server_features: u64,
) -> (u64, YlbLinkReport) {
    let name = unique_name(tag);
    let server = unsafe { ylb_test_server_start(name.as_ptr(), name.len() as i32, 256, 128) };
    assert_ne!(server, 0);
    assert_eq!(
        ylb_test_server_configure(server, pack(server_version), pack(server_min_peer), server_features),
        0
    );
    let agent = "試験";
    let h = unsafe {
        ylb_connect_with(
            name.as_ptr(),
            name.len() as i32,
            agent.as_ptr(),
            agent.len() as i32,
            own_version.as_ptr(),
            own_version.len() as i32,
        )
    };
    assert_ne!(h, 0);
    wait_for(h, "つながり", || ylb_status(h) == 1);
    let mut report: YlbLinkReport = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { ylb_link_report(h, &mut report) }, 1);
    (h, report)
}

#[test]
fn the_peer_version_and_the_marks_are_asked_through_the_c_functions() {
    use yolu_protocol::feature;
    let none = yolu_protocol::AppVersion::NONE_PACKED;
    // 版が合っていて印も同じ（スタンドアロンの役はこのブリッジと同じ印を出す。印を立てる機能が増えても変わらない）: ずれなし
    let (h, r) = report_against("clean", "0.3.0", v(0, 1, 0), v(0, 3, 0), BRIDGE_FEATURES);
    assert_eq!(r.state, 1);
    assert_eq!(r.protocol, 1);
    assert_eq!(r.peer_version, pack(v(0, 1, 0)));
    assert_eq!(ylb_peer_app_version(h), pack(v(0, 1, 0)));
    assert_eq!((r.update_peer, r.update_self), (none, none));
    assert_eq!(r.peer_min_peer, pack(v(0, 3, 0)));
    assert_eq!(
        (r.missing_on_peer, r.missing_here, r.common_features),
        (0, 0, BRIDGE_FEATURES)
    );
    assert_eq!(ylb_common_features(h), BRIDGE_FEATURES);
    assert_eq!(ylb_disconnect(h), 0);

    // スタンドアロンが求める Unity の版（0.4.0）に、名乗った 0.3.0 は足りない: 自分を上げる
    let (h, r) = report_against("oldunity", "0.3.0", v(0, 1, 0), v(0, 4, 0), 0);
    assert_eq!(r.update_self, pack(v(0, 4, 0)));
    assert_eq!(r.update_peer, none);
    assert_eq!(ylb_disconnect(h), 0);

    // 版を名乗らなかったブリッジ（ylb_connect）は、スタンドアロンから見て古い。ここでは名乗りを読めない文字列で試す
    let (h, r) = report_against("nover", "未定", v(0, 1, 0), v(0, 4, 0), 0);
    assert_eq!(r.state, 1, "名乗らなくてもつながる");
    assert_eq!(ylb_disconnect(h), 0);

    // 版を名乗らない古いスタンドアロン: 相手の版は不明で、相手を上げるのを勧める（版の指定なしは 0）
    let (h, r) = report_against("oldstandalone", "0.3.0", None, None, 0);
    assert_eq!(r.peer_version, none);
    assert_eq!(ylb_peer_app_version(h), none);
    assert_eq!(r.update_peer, 0);
    assert_eq!(r.update_self, none);
    assert_eq!(ylb_disconnect(h), 0);

    // 機能の印: 共通はこのブリッジも出す印（マテリアルの値）だけで、ブリッジに無い印（アニメーション）は自分を上げれば使える機能として出る
    let (h, r) = report_against("marks", "0.3.0", v(0, 1, 0), v(0, 0, 0), feature::MATERIAL_VALUES | feature::ANIMATION);
    assert_eq!(r.peer_features, feature::MATERIAL_VALUES | feature::ANIMATION);
    assert_eq!(r.own_features, BRIDGE_FEATURES);
    assert_eq!(r.common_features, BRIDGE_FEATURES & r.peer_features);
    assert_eq!(r.missing_here, r.peer_features & !BRIDGE_FEATURES);
    assert_eq!(r.missing_on_peer, BRIDGE_FEATURES & !r.peer_features);
    assert_eq!(ylb_common_features(h), r.common_features);
    assert_eq!(ylb_disconnect(h), 0);

    // 不明なつながり
    assert_eq!(ylb_common_features(987654321), 0);
    assert_eq!(ylb_peer_app_version(987654321), none);
    let mut report: YlbLinkReport = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { ylb_link_report(987654321, &mut report) }, YLB_E_HANDLE);
}

#[test]
// 試験のスタンドアロンは、Windows では止めても受けた接続を閉じない（名前付きパイプの読みの待ちを外から切れないので、待たずに手放す）。
// 相手の終わりを作れないので Windows では回さない。終わったつながりが版と機能を答えない仕組み（Session の fail・close）は OS に依らず、Linux で確かめる。
#[cfg_attr(windows, ignore = "試験のスタンドアロンが Windows では接続を閉じず、相手の終わりを作れない")]
fn the_answers_about_the_peer_end_with_the_link() {
    // 版も機能もずれたスタンドアロンにつなぎ、そのスタンドアロンが終わる（自己診断のサーバーを止める）
    let name = unique_name("ends");
    let server = unsafe { ylb_test_server_start(name.as_ptr(), name.len() as i32, 256, 128) };
    assert_ne!(server, 0);
    let unknown = 1u64 << 50;
    assert_eq!(
        ylb_test_server_configure(server, pack(v(0, 1, 0)), pack(v(9, 0, 0)), BRIDGE_FEATURES | unknown),
        0
    );
    let agent = "試験";
    let version = "0.3.0";
    let h = unsafe {
        ylb_connect_with(
            name.as_ptr(),
            name.len() as i32,
            agent.as_ptr(),
            agent.len() as i32,
            version.as_ptr(),
            version.len() as i32,
        )
    };
    assert_ne!(h, 0);
    wait_for(h, "つながり", || ylb_status(h) == 1);
    let mut report: YlbLinkReport = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { ylb_link_report(h, &mut report) }, 1);
    assert_eq!(report.update_self, pack(v(9, 0, 0)), "つながっている間は、ずれが見える");
    assert_eq!(report.missing_here, unknown);
    assert_eq!(ylb_peer_app_version(h), pack(v(0, 1, 0)));
    assert_eq!(ylb_common_features(h), BRIDGE_FEATURES);

    assert_eq!(ylb_test_server_stop(server), 0);
    let deadline = Instant::now() + Duration::from_secs(60);
    while ylb_status(h) == 1 {
        assert!(Instant::now() < deadline, "スタンドアロンが終わったのに、つながったまま");
        std::thread::sleep(Duration::from_millis(10));
    }
    // 終わったつながりの版・使える機能は答えない（閉じたあとも版のずれの印を残さない）
    assert_eq!(unsafe { ylb_link_report(h, &mut report) }, 0);
    assert_eq!(report.state, 0);
    assert_eq!(
        (report.update_self, report.update_peer, report.missing_here, report.common_features),
        (yolu_protocol::AppVersion::NONE_PACKED, yolu_protocol::AppVersion::NONE_PACKED, 0, 0)
    );
    assert_eq!(ylb_peer_app_version(h), yolu_protocol::AppVersion::NONE_PACKED);
    assert_eq!(ylb_common_features(h), 0);
    assert_eq!(ylb_disconnect(h), 0);
}

#[test]
fn a_refusal_for_the_protocol_range_comes_with_which_side_to_update() {
    use yolu_protocol::frame::FrameReader;
    use yolu_protocol::{encode_message, Message, RejectCode, RejectDetail};
    let name = unique_name("refused");
    let server = yolu_protocol::Server::bind(&name, false).unwrap();
    let handle = std::thread::spawn(move || {
        use std::io::Write;
        let stream = server.accept().unwrap();
        let mut s = &stream;
        let mut frames = FrameReader::new();
        let hello = frames.read_frame(&mut s).unwrap().unwrap().decode().unwrap();
        assert!(matches!(hello, Message::Hello(_)));
        // 新しいスタンドアロン（読めるのは版 2 から。Unity のパッケージを 0.6.0 以上に求める）の断り
        let reject = yolu_protocol::Reject {
            code: RejectCode::VersionMismatch,
            text: "断り".into(),
            detail: Some(RejectDetail {
                min_version: 2,
                max_version: 3,
                min_peer: yolu_protocol::AppVersion::new(0, 6, 0),
                peer_min_version: 1,
                peer_max_version: 1,
                peer_min_peer: yolu_protocol::AppVersion::new(0, 1, 0),
            }),
        };
        s.write_all(&encode_message(&Message::Reject(reject))).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        server
    });
    let agent = "試験";
    let version = "0.3.0";
    let h = unsafe {
        ylb_connect_with(
            name.as_ptr(),
            name.len() as i32,
            agent.as_ptr(),
            agent.len() as i32,
            version.as_ptr(),
            version.len() as i32,
        )
    };
    wait_for(h, "失敗", || ylb_status(h) == 3);
    let mut report: YlbLinkReport = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { ylb_link_report(h, &mut report) }, 2);
    assert_eq!(report.refused_update, 1, "Unity のパッケージを上げる");
    assert_eq!(report.refused_to, pack(v(0, 6, 0)));
    assert_eq!((report.unity_min_protocol, report.unity_max_protocol), (1, 1));
    assert_eq!((report.standalone_min_protocol, report.standalone_max_protocol), (2, 3));
    let ev = events(h);
    assert!(ev.iter().any(|e| e.0 == 2), "断りの知らせが来る: {ev:?}");
    assert_eq!(ylb_disconnect(h), 0);
    drop(handle.join().unwrap());
}

#[test]
fn the_test_server_with_another_protocol_range_refuses_and_the_report_names_the_side_to_update() {
    let name = unique_name("range");
    let server = unsafe { ylb_test_server_start(name.as_ptr(), name.len() as i32, 256, 128) };
    assert_ne!(server, 0);
    // スタンドアロンの役が、先の版（2〜3）だけを読み、Unity のパッケージを 0.6.0 以上に求める
    assert_eq!(ylb_test_server_set_protocol(server, 2, 3), 0);
    assert_eq!(ylb_test_server_set_protocol(server, 3, 2), YLB_E_ARGUMENT);
    assert_eq!(
        ylb_test_server_configure(server, pack(v(0, 9, 0)), pack(v(0, 6, 0)), 0),
        0
    );
    let agent = "試験";
    let version = "0.3.0";
    let h = unsafe {
        ylb_connect_with(
            name.as_ptr(),
            name.len() as i32,
            agent.as_ptr(),
            agent.len() as i32,
            version.as_ptr(),
            version.len() as i32,
        )
    };
    wait_for(h, "失敗", || ylb_status(h) == 3);
    let mut report: YlbLinkReport = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { ylb_link_report(h, &mut report) }, 2);
    assert_eq!(report.refused_update, 1, "Unity のパッケージを上げる");
    assert_eq!(report.refused_to, pack(v(0, 6, 0)));
    assert_eq!((report.unity_min_protocol, report.unity_max_protocol), (1, 1));
    assert_eq!((report.standalone_min_protocol, report.standalone_max_protocol), (2, 3));
    assert_eq!(ylb_disconnect(h), 0);
    // 古い範囲（0〜0）だけを読むスタンドアロンの役なら、スタンドアロンを上げる（ブリッジの求める版へ。今は要求がないので版の指定なし）
    assert_eq!(ylb_test_server_set_protocol(server, 0, 0), 0);
    let h = unsafe {
        ylb_connect_with(
            name.as_ptr(),
            name.len() as i32,
            agent.as_ptr(),
            agent.len() as i32,
            version.as_ptr(),
            version.len() as i32,
        )
    };
    wait_for(h, "失敗", || ylb_status(h) == 3);
    assert_eq!(unsafe { ylb_link_report(h, &mut report) }, 2);
    assert_eq!(report.refused_update, 2, "スタンドアロンを上げる");
    assert_eq!(report.refused_to, yolu_protocol::AppVersion::NONE_PACKED);
    assert_eq!(ylb_disconnect(h), 0);
    assert_eq!(ylb_test_server_stop(server), 0);
}

// ───────── マテリアルの値（機能の印 MATERIAL_VALUES） ─────────

fn stats(server: u64) -> YlbTestServerStats {
    let mut s = YlbTestServerStats::default();
    assert_eq!(unsafe { ylb_test_server_stats(server, &mut s) }, 0);
    s
}

fn put_values(h: u64, material: i32) -> i32 {
    unsafe {
        let (shader, source) = ("Hidden/lilToonOutline", "lilToon 2.3.4 · Standard/Opaque+Outline");
        let r = ylb_values_begin(
            h,
            material,
            1,
            shader.as_ptr(),
            shader.len() as i32,
            source.as_ptr(),
            source.len() as i32,
        );
        if r != 0 {
            return r;
        }
        let n = |s: &str| (s.as_ptr(), s.len() as i32);
        let (p, l) = n("_ShadowBorder");
        assert_eq!(ylb_values_float(h, p, l, 0.25), 0);
        // 同じ名前は置き換える
        assert_eq!(ylb_values_float(h, p, l, 0.3), 0);
        let (p, l) = n("_UseShadow");
        assert_eq!(ylb_values_int(h, p, l, 1), 0);
        let (p, l) = n("_ShadowColor");
        assert_eq!(ylb_values_color(h, p, l, 0.5, 0.25, 0.75, 1.0), 0);
        let (p, l) = n("_MainTex_ST");
        assert_eq!(ylb_values_vector(h, p, l, 2.0, 1.0, 0.5, 0.0), 0);
        // 有限でない値・空の名前・制御文字の名前は断る
        assert_eq!(ylb_values_float(h, p, l, f32::NAN), YLB_E_ARGUMENT);
        assert_eq!(ylb_values_color(h, p, l, 0.0, f32::INFINITY, 0.0, 1.0), YLB_E_ARGUMENT);
        let (p0, _) = n("x");
        assert_eq!(ylb_values_float(h, p0, 0, 1.0), YLB_E_ARGUMENT);
        let (pc, lc) = n("_A\n");
        assert_eq!(ylb_values_float(h, pc, lc, 1.0), YLB_E_ARGUMENT);
        let (p, l) = n("_EMISSION");
        assert_eq!(ylb_values_keyword(h, p, l), 0);
        assert_eq!(ylb_values_keyword(h, p, l), 0, "重ねて足しても 1 つ");
        let (p, l) = n("A B");
        assert_eq!(ylb_values_keyword(h, p, l), YLB_E_ARGUMENT);
        let (p, l) = n("_MatCapTex");
        assert_eq!(ylb_values_slot(h, p, l, 1, 4, 2), 0);
        assert_eq!(ylb_values_slot(h, p, l, 1, 4, 2), YLB_E_ARGUMENT, "同じスロットは 1 つ");
        let (p, l) = n("_ShadowColorTex");
        assert_eq!(ylb_values_slot(h, p, l, 3, 4096, 4096), 0);
        assert_eq!(ylb_values_slot(h, p, l, 9, 1, 1), YLB_E_ARGUMENT);
        ylb_values_send(h)
    }
}

fn send_texture(h: u64, material: i32, pixels: &[u8], width: u32, height: u32) -> i32 {
    let slot = "_MatCapTex";
    unsafe {
        ylb_texture_send(
            h,
            material,
            slot.as_ptr(),
            slot.len() as i32,
            width,
            height,
            1,
            pixels.as_ptr(),
            pixels.len() as i32,
        )
    }
}

#[test]
fn material_values_reach_a_standalone_with_the_mark() {
    use yolu_protocol::feature;
    assert_ne!(BRIDGE_FEATURES & feature::MATERIAL_VALUES, 0, "ブリッジは値の印を出す");
    let name = unique_name("values");
    let server = unsafe { ylb_test_server_start(name.as_ptr(), name.len() as i32, 256, 128) };
    assert_ne!(server, 0);
    assert_eq!(
        ylb_test_server_configure(server, pack(v(0, 1, 0)), pack(None), feature::MATERIAL_VALUES),
        0
    );
    let h = connect(&name);
    wait_for(h, "つながり", || ylb_status(h) == 1);
    assert_ne!(ylb_common_features(h) & feature::MATERIAL_VALUES, 0);
    // モデルを送る前は組み立てを始めない
    assert_eq!(put_values(h, 0), YLB_E_STATE);
    send_quad_model(h);
    // 無いマテリアルの番号は断る
    assert_eq!(put_values(h, 2), YLB_E_ARGUMENT);
    assert_eq!(put_values(h, 1), 1);
    wait_for(h, "値", || stats(server).values == 1);
    let st = stats(server);
    assert_eq!(
        (
            st.last_values_material,
            st.last_values_kind,
            st.last_values_properties,
            st.last_values_keywords,
            st.last_values_slots,
        ),
        (1, 1, 4, 1, 2)
    );
    assert_eq!(st.last_values_shader_len, "Hidden/lilToonOutline".len() as u32);
    let mut out = [0f32; 4];
    let read = |name: &str, out: &mut [f32; 4]| unsafe {
        ylb_test_server_value(server, 1, name.as_ptr(), name.len() as i32, out.as_mut_ptr())
    };
    assert_eq!(read("_ShadowBorder", &mut out), 0);
    assert_eq!(out[0], 0.3);
    assert_eq!(read("_UseShadow", &mut out), 1);
    assert_eq!(out[0], 1.0);
    assert_eq!(read("_ShadowColor", &mut out), 2);
    assert_eq!(out, [0.5, 0.25, 0.75, 1.0]);
    assert_eq!(read("_MainTex_ST", &mut out), 3);
    assert_eq!(read("_Nothing", &mut out), YLB_E_ARGUMENT);
    let slot = |name: &str, keyword: i32| unsafe {
        ylb_test_server_slot(server, 1, name.as_ptr(), name.len() as i32, keyword)
    };
    assert_eq!(slot("_MatCapTex", 0), 1);
    assert_eq!(slot("_ShadowColorTex", 0), 3);
    assert_eq!(slot("_EMISSION", 1), 1);
    assert_eq!(slot("_OTHER", 1), 0);

    // 絵: 幅 × 高さ × 4 と合わない長さ・上限を超える辺・無いマテリアルは断る
    let mut pixels = vec![0u8; 4 * 2 * 4];
    for (i, p) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        *p = [i as u8 * 10, 20, 30, 255];
    }
    assert_eq!(send_texture(h, 1, &pixels[..31], 4, 2), YLB_E_ARGUMENT);
    // 辺 0・辺の上限（2048）を超える絵は、長さが幅 × 高さ × 4 と合っていても断る
    assert_eq!(send_texture(h, 1, &[], 4, 0), YLB_E_ARGUMENT);
    assert_eq!(send_texture(h, 1, &[], 0, 2), YLB_E_ARGUMENT);
    let edge = yolu_protocol::MAX_SLOT_TEXTURE_SIZE;
    let over = vec![0u8; (edge as usize + 1) * 4];
    assert_eq!(send_texture(h, 1, &over, edge + 1, 1), YLB_E_ARGUMENT);
    assert_eq!(send_texture(h, 1, &over, 1, edge + 1), YLB_E_ARGUMENT);
    assert_eq!(send_texture(h, 5, &pixels, 4, 2), YLB_E_ARGUMENT);
    // 上限ちょうどは送る（同じスロットの絵は、あとの 4 × 2 で置き換わる）
    assert_eq!(send_texture(h, 1, &over[..edge as usize * 4], edge, 1), 1);
    assert_eq!(send_texture(h, 1, &pixels, 4, 2), 1);
    wait_for(h, "絵", || stats(server).textures == 2);
    let mut t = YlbTestServerTexture::default();
    let s = "_MatCapTex";
    assert_eq!(
        unsafe { ylb_test_server_texture(server, 1, s.as_ptr(), s.len() as i32, &mut t) },
        0
    );
    // 真ん中は (2, 1) の画素（行は下から、1 行 4 画素）: 番号 6
    assert_eq!((t.width, t.height, t.srgb), (4, 2, 1));
    assert_eq!(t.center.to_le_bytes(), [60, 20, 30, 255]);
    assert_eq!(stats(server).refused, 0);
    assert_eq!(ylb_disconnect(h), 0);
    assert_eq!(ylb_test_server_stop(server), 0);
}

#[test]
fn material_values_are_not_sent_to_a_standalone_without_the_mark() {
    // 印を出さない（値を知らない）スタンドアロン: 組み立ては受け付けるが、送らずに 0 を返す。モデル・ポーズは今までどおり
    let name = unique_name("novalues");
    let server = unsafe { ylb_test_server_start(name.as_ptr(), name.len() as i32, 256, 128) };
    assert_ne!(server, 0);
    assert_eq!(ylb_test_server_configure(server, pack(v(0, 1, 0)), pack(None), 0), 0);
    let h = connect(&name);
    wait_for(h, "つながり", || ylb_status(h) == 1);
    assert_eq!(ylb_common_features(h), 0);
    send_quad_model(h);
    wait_for(h, "モデル", || stats(server).models == 1);
    assert_eq!(put_values(h, 0), 0);
    let pixels = vec![255u8; 16];
    assert_eq!(send_texture(h, 0, &pixels, 2, 2), 0);
    // 後から送った印の要らない命令が届くまで待ち、その前に値・絵が届いていないことを見る
    assert_eq!(ylb_model_close(h), 0);
    wait_for(h, "モデルを閉じる知らせ", || stats(server).models_closed == 1);
    let st = stats(server);
    assert_eq!((st.values, st.textures, st.unknown), (0, 0, 0));
    assert_eq!(ylb_disconnect(h), 0);
    assert_eq!(ylb_test_server_stop(server), 0);
}

// ───────── 元の絵（機能の印 ORIGINAL_TEXTURES） ─────────

#[allow(clippy::too_many_arguments)]
fn send_original(
    h: u64,
    material: i32,
    slot: &str,
    state: i32,
    read: i32,
    flags: i32,
    (width, height): (u32, u32),
    srgb: i32,
    pixels: &[u8],
) -> i32 {
    unsafe {
        ylb_original_send(
            h,
            material,
            slot.as_ptr(),
            slot.len() as i32,
            state,
            read,
            flags,
            width,
            height,
            srgb,
            pixels.as_ptr(),
            pixels.len() as i32,
        )
    }
}

fn original_of(server: u64, material: u32, slot: &str) -> Option<YlbTestServerOriginal> {
    let mut o = YlbTestServerOriginal::default();
    (unsafe { ylb_test_server_original(server, material, slot.as_ptr(), slot.len() as i32, &mut o) } == 0)
        .then_some(o)
}

/// 4 × 2 の絵（画素 (x, y) は [x * 40 + 10, y * 100 + 5, 7, 255]）。
fn small_original() -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in 0..2u8 {
        for x in 0..4u8 {
            pixels.extend_from_slice(&[x * 40 + 10, y * 100 + 5, 7, 255]);
        }
    }
    pixels
}

#[test]
fn originals_reach_a_standalone_with_the_mark_and_its_sets_wait_until_they_are_all_there() {
    use yolu_protocol::{feature, MAX_ORIGINAL_SIZE};
    assert_ne!(BRIDGE_FEATURES & feature::ORIGINAL_TEXTURES, 0, "ブリッジは元の絵の印を出す");
    let name = unique_name("originals");
    let server = unsafe { ylb_test_server_start(name.as_ptr(), name.len() as i32, 256, 128) };
    assert_ne!(server, 0);
    assert_eq!(
        ylb_test_server_configure(
            server,
            pack(v(0, 1, 0)),
            pack(None),
            feature::MATERIAL_VALUES | feature::ORIGINAL_TEXTURES
        ),
        0
    );
    let h = connect(&name);
    wait_for(h, "つながり", || ylb_status(h) == 1);
    assert_ne!(ylb_common_features(h) & feature::ORIGINAL_TEXTURES, 0);
    let px = small_original();
    // モデルを送る前は送らない
    assert_eq!(send_original(h, 0, "_MainTex", 0, 0, 0, (4, 2), 1, &px), YLB_E_STATE);
    send_quad_model(h);
    wait_for(h, "モデル", || stats(server).models == 1);
    // 2 つのマテリアルの Color の流し込み先に絵が入っている: 元の絵が揃うまで、どのセットも出さない
    assert_eq!(stats(server).held_sets, 2);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(ylb_set_count(h), 0, "揃うまでセットを出さない");

    // 引数の確かめ: 画素の長さが合わない・辺 0・上限を超える辺・知らない様子と読み方・絵の付かない様子に画素・無いマテリアル・空の名前
    assert_eq!(send_original(h, 0, "_MainTex", 0, 0, 0, (4, 2), 1, &px[..31]), YLB_E_ARGUMENT);
    assert_eq!(send_original(h, 0, "_MainTex", 0, 0, 0, (4, 0), 1, &[]), YLB_E_ARGUMENT);
    assert_eq!(send_original(h, 0, "_MainTex", 0, 0, 0, (0, 2), 1, &[]), YLB_E_ARGUMENT);
    let over = vec![0u8; (MAX_ORIGINAL_SIZE as usize + 1) * 4];
    assert_eq!(
        send_original(h, 0, "_MainTex", 0, 0, 0, (MAX_ORIGINAL_SIZE + 1, 1), 1, &over),
        YLB_E_ARGUMENT
    );
    assert_eq!(send_original(h, 0, "_MainTex", 4, 0, 0, (4, 2), 1, &[]), YLB_E_ARGUMENT);
    assert_eq!(send_original(h, 0, "_MainTex", -1, 0, 0, (4, 2), 1, &[]), YLB_E_ARGUMENT);
    assert_eq!(send_original(h, 0, "_MainTex", 0, 3, 0, (4, 2), 1, &px), YLB_E_ARGUMENT);
    assert_eq!(send_original(h, 0, "_MainTex", 2, 0, 0, (16384, 16384), 1, &px), YLB_E_ARGUMENT);
    assert_eq!(send_original(h, 2, "_MainTex", 0, 0, 0, (4, 2), 1, &px), YLB_E_ARGUMENT);
    assert_eq!(send_original(h, -1, "_MainTex", 0, 0, 0, (4, 2), 1, &px), YLB_E_ARGUMENT);
    assert_eq!(send_original(h, 0, "", 0, 0, 0, (4, 2), 1, &px), YLB_E_ARGUMENT);
    assert_eq!(send_original(h, 0, "_A\n", 0, 0, 0, (4, 2), 1, &px), YLB_E_ARGUMENT);
    assert_eq!(stats(server).originals, 0, "断った絵は届かない");

    // マテリアル 0 の元の絵（原本のファイルから）: そのセットだけが出る。マテリアル 1 は待たせたまま
    assert_eq!(send_original(h, 0, "_MainTex", 0, 0, 0, (4, 2), 1, &px), 1);
    wait_for(h, "マテリアル 0 のセット", || sets(h).len() == 1);
    let st = stats(server);
    assert_eq!((st.originals, st.held_sets), (1, 1));
    // 辺が上限を超えるので送らない、と知らせる（画素なし）: マテリアル 1 のセットは試しの模様で出る
    assert_eq!(
        send_original(h, 1, "_MainTex", 2, 2, 1, (16384, 16384), 0, &[]),
        1
    );
    wait_for(h, "マテリアル 1 のセット", || sets(h).len() == 2);
    assert_eq!(stats(server).held_sets, 0);

    // 出たセットの Color は元の絵（最近傍で 256 に合わせた）
    let all = sets(h);
    let zero = all.iter().find(|s| s.material == 0).unwrap();
    wait_for(h, "汚れたタイル", || ylb_channel_dirty(h, zero.set, 0) == 4);
    let mut image = vec![0u8; 256 * 256 * 4];
    let mut r = YlbCopyResult::default();
    let n = unsafe {
        ylb_copy_dirty(
            h,
            zero.set,
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
    assert_eq!(n, 4);
    let at = |x: usize, y: usize| ((y * 256 + x) * 4, (y * 256 + x) * 4 + 4);
    for (x, y, expected) in [(5usize, 7usize, [10u8, 5, 7, 255]), (200, 200, [130, 105, 7, 255]), (70, 130, [50, 105, 7, 255])] {
        let (a, b) = at(x, y);
        assert_eq!(image[a..b], expected, "({x}, {y})");
    }

    // 受けた様子: 読み方・圧縮・sRGB・大きさ・画素
    let first = original_of(server, 0, "_MainTex").unwrap();
    assert_eq!((first.state, first.read, first.compressed, first.srgb), (0, 0, 0, 1));
    assert_eq!((first.width, first.height), (4, 2));
    assert_eq!(first.corner.to_le_bytes(), [10, 5, 7, 255]);
    assert_eq!(first.center.to_le_bytes(), [90, 105, 7, 255]);
    let declined = original_of(server, 1, "_MainTex").unwrap();
    assert_eq!((declined.state, declined.read, declined.compressed, declined.srgb), (2, 2, 1, 0));
    assert_eq!((declined.width, declined.height, declined.center), (16384, 16384, 0));
    assert!(original_of(server, 0, "_Other").is_none());
    // 同じスロットはあとの絵で置き換わる（GPU を通して・圧縮・リニア）
    assert_eq!(send_original(h, 0, "_MainTex", 0, 2, 1, (4, 2), 0, &px), 1);
    wait_for(h, "置き換え", || stats(server).originals == 3);
    let again = original_of(server, 0, "_MainTex").unwrap();
    assert_eq!((again.read, again.compressed, again.srgb), (2, 1, 0));
    wait_for(h, "積んだ命令が出ていく", || ylb_pending_bytes(h) == 0);
    assert_eq!(stats(server).refused, 0);
    assert_eq!(ylb_pending_bytes(0), 0, "知らないつながりは 0");
    assert_eq!(ylb_disconnect(h), 0);
    assert_eq!(ylb_test_server_stop(server), 0);
}

#[test]
fn originals_are_not_sent_to_a_standalone_without_the_mark_and_nothing_waits() {
    use yolu_protocol::feature;
    // 値の印だけを出す（元の絵を知らない）スタンドアロン: 送らずに 0 を返し、セットは待たせずに出る
    let name = unique_name("nooriginals");
    let server = unsafe { ylb_test_server_start(name.as_ptr(), name.len() as i32, 256, 128) };
    assert_ne!(server, 0);
    assert_eq!(
        ylb_test_server_configure(server, pack(v(0, 1, 0)), pack(None), feature::MATERIAL_VALUES),
        0
    );
    let h = connect(&name);
    wait_for(h, "つながり", || ylb_status(h) == 1);
    assert_eq!(ylb_common_features(h) & feature::ORIGINAL_TEXTURES, 0);
    send_quad_model(h);
    wait_for(h, "2 つのセット", || sets(h).len() == 2);
    let px = small_original();
    assert_eq!(send_original(h, 0, "_MainTex", 0, 0, 0, (4, 2), 1, &px), 0);
    assert_eq!(send_original(h, 1, "_MainTex", 2, 2, 0, (16384, 16384), 1, &[]), 0);
    // 後から送った印の要らない命令が届くまで待ち、その前に元の絵が届いていないことを見る
    assert_eq!(ylb_model_close(h), 0);
    wait_for(h, "モデルを閉じる知らせ", || stats(server).models_closed == 1);
    let st = stats(server);
    assert_eq!((st.originals, st.held_sets, st.unknown), (0, 0, 0));
    assert_eq!(ylb_disconnect(h), 0);
    assert_eq!(ylb_test_server_stop(server), 0);
}
