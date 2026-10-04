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
    assert_eq!(ylb_abi_version(), 2);
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
