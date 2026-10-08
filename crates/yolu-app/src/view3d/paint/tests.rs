use super::*;

#[test]
fn reduce_averages_premultiplied_blocks_and_partial_edges() {
    // 3 × 2 の赤（不透明・半透明・透明）を 2 で縮めると 2 × 1
    let px = |r: u8, a: u8| [r, 0, 0, a];
    let mut s = Vec::new();
    for p in [
        px(255, 255),
        px(255, 128),
        px(9, 0),
        px(255, 255),
        px(255, 128),
        px(9, 0),
    ] {
        s.extend_from_slice(&p);
    }
    let (out, w, h) = reduce_premultiplied(&s, DocRect::new(0, 0, 3, 2), 1);
    assert_eq!((w, h), (2, 1));
    // 左の箱: (255 + 128 + 255 + 128) / 4、右の箱は透明だけ（RGB も 0。乗算済みなので色は消える）
    assert_eq!(&out[0..4], &[192, 0, 0, 192]);
    assert_eq!(&out[4..8], &[0, 0, 0, 0]);
    let (same, w, h) = reduce_premultiplied(&s, DocRect::new(0, 0, 3, 2), 0);
    assert_eq!((w, h), (3, 2));
    assert_eq!(&same[4..8], &[128, 0, 0, 128]);
}

#[test]
fn srgb_texels_keep_opaque_bytes_and_premultiply_and_average_in_linear() {
    // 不透明な画素は元のバイトのまま（8 bit の sRGB → 16 bit のリニア → 8 bit の sRGB が元に戻る。書き出しと同じテクセル）
    let all: Vec<u8> = (0..=255u8).flat_map(|v| [v, 255 - v, v / 2, 255]).collect();
    let (out, _, _) = reduce_srgb_premultiplied(&all, DocRect::new(0, 0, 256, 1), 0);
    assert_eq!(out, all);
    let (out, _, _) = reduce_emission(&all, DocRect::new(0, 0, 256, 1), 0);
    assert_eq!(out, all);
    // 半透明: リニアで掛ける（sRGB 255 × α 0.5 → リニア 0.5 → sRGB 188）。ガンマのまま掛ける Emission は書き出しと同じ 128
    let half = [255u8, 255, 255, 128];
    let (out, _, _) = reduce_srgb_premultiplied(&half, DocRect::new(0, 0, 1, 1), 0);
    assert_eq!(out, [188, 188, 188, 128]);
    let (out, _, _) = reduce_emission(&half, DocRect::new(0, 0, 1, 1), 0);
    assert_eq!(out, [128, 128, 128, 128]);
    // 白と黒（不透明）を縮めると、リニアの平均 0.5 → sRGB 188（ガンマの平均の 128 より明るい。Unity のミップと同じ）
    let bw = [255u8, 255, 255, 255, 0, 0, 0, 255];
    let (out, w, h) = reduce_srgb_premultiplied(&bw, DocRect::new(0, 0, 2, 1), 1);
    assert_eq!((out.as_slice(), w, h), (&[188u8, 188, 188, 255][..], 1, 1));
    let (out, _, _) = reduce_emission(&bw, DocRect::new(0, 0, 2, 1), 1);
    assert_eq!(out, [188, 188, 188, 255]);
}

#[test]
fn scalar_reduce_flattens_value_times_alpha() {
    // 値 128・アルファ 255 と、値 255・アルファ 0（塗っていない所は 0）
    let s = [128u8, 0, 0, 255, 255, 0, 0, 0];
    let map = |p: &[u8]| [(p[0] as u32 * p[3] as u32 + 127) / 255, 0, 0, 0];
    let (out, w, h) = reduce(&s, DocRect::new(0, 0, 2, 1), 0, 1, map);
    assert_eq!((out.as_slice(), w, h), (&[128u8, 0][..], 2, 1));
    let (out, w, h) = reduce(&s, DocRect::new(0, 0, 2, 1), 1, 1, map);
    assert_eq!((out.as_slice(), w, h), (&[64u8][..], 1, 1));
}

#[test]
fn rects_align_to_the_reduction_blocks_and_expand_inside_the_canvas() {
    let bounds = DocRect::new(0, 0, 300, 200);
    // 外へ 1 画素: キャンバスの端で切る
    assert_eq!(
        expand(DocRect::new(256, 0, 44, 200), 1, bounds),
        DocRect::new(255, 0, 45, 200)
    );
    // 縮めの境（4 画素）に合わせて広げる
    assert_eq!(
        align(DocRect::new(255, 1, 3, 3), 2, bounds),
        DocRect::new(252, 0, 8, 4)
    );
    assert_eq!(
        align(DocRect::new(297, 197, 3, 3), 2, bounds),
        DocRect::new(296, 196, 4, 4),
        "キャンバスの端では端で切る"
    );
    assert_eq!(
        align(DocRect::new(5, 6, 7, 8), 0, bounds),
        DocRect::new(5, 6, 7, 8)
    );
}

fn tiles(grid: u32, size: u32, only: impl Fn(u32, u32) -> bool) -> Vec<(DocRect, usize)> {
    let mut v = Vec::new();
    for y in 0..grid {
        for x in 0..grid {
            if only(x, y) {
                v.push((DocRect::new(x * size, y * size, size, size), 1));
            }
        }
    }
    v
}

#[test]
fn adjacent_tiles_become_one_write_and_far_ones_stay_apart() {
    let bounds = DocRect::new(0, 0, 512, 512);
    // 2 × 1 の隣り合うタイル → 1 つ（2 タイル）
    let two = plan_regions(
        tiles(4, 128, |x, y| y == 1 && (x == 1 || x == 2)),
        0,
        bounds,
        false,
    );
    assert_eq!(two, vec![(DocRect::new(128, 128, 256, 128), 2)]);
    // 2 × 2 → 1 つ（4 タイル）
    let four = plan_regions(
        tiles(4, 128, |x, y| (1..=2).contains(&x) && (1..=2).contains(&y)),
        0,
        bounds,
        false,
    );
    assert_eq!(four, vec![(DocRect::new(128, 128, 256, 256), 4)]);
    // 斜めに離れた 2 つは別（合わせると 4 倍の面積）
    let apart = plan_regions(
        tiles(4, 128, |x, y| x == y && x != 1 && x != 2),
        0,
        bounds,
        false,
    );
    assert_eq!(apart.len(), 2);
    assert_eq!(apart.iter().map(|(_, n)| n).sum::<usize>(), 2);
    // 面積が少し増えるだけの L 字は合わせる（3 タイル → 1.33 倍で、1.25 倍を超えるので別）
    let l = plan_regions(
        tiles(4, 128, |x, y| {
            (x, y) != (1, 1) && (1..=2).contains(&x) && (1..=2).contains(&y)
        }),
        0,
        bounds,
        false,
    );
    assert_eq!(l.iter().map(|(_, n)| n).sum::<usize>(), 3);
    assert!(l.len() >= 2, "{l:?}");
}

#[test]
fn a_whole_canvas_is_cut_into_bands_with_the_tile_count_kept() {
    let bounds = DocRect::new(0, 0, 4096, 4096);
    let all = tiles(32, 128, |_, _| true);
    assert_eq!(all.len(), 1024);
    let bands = plan_regions(all.clone(), 0, bounds, false);
    assert_eq!(bands.len(), 4, "1024 行ずつの帯");
    assert_eq!(bands.iter().map(|(_, n)| n).sum::<usize>(), 1024);
    assert!(bands
        .iter()
        .all(|(r, _)| r.width == 4096 && r.height == 1024));
    // 縮めの境（8 画素）に合っている
    let reduced = plan_regions(all.clone(), 3, bounds, false);
    assert!(reduced
        .iter()
        .all(|(r, _)| r.y % 8 == 0 && r.height % 8 == 0));
    // 端が反対側を読む設定は切らない（1 つの矩形のまま）
    let whole = plan_regions(all, 0, bounds, true);
    assert_eq!(whole.len(), 1);
    assert_eq!(whole[0].0, bounds);
}

#[test]
fn shrink_follows_the_texture_limit_and_the_byte_budget() {
    let all = 15; // Color・Emission・Normal が 4、Metallic・Roughness・Height が 1
                  // 4096² の 6 チャンネルは予算（512 MiB）に収まり、縮めない（約 320 MiB）
    assert_eq!((mip_bytes([4096, 4096], all) + (1 << 19)) >> 20, 320);
    assert_eq!(choose_shift([4096, 4096], all, 8192, PAINT_BUDGET_BYTES), 0);
    // 8192² の 6 チャンネルは約 1.28 GiB なので 1 段縮めて 4096²（予算が決める）
    assert!(mip_bytes([8192, 8192], all) > 1 << 30);
    assert_eq!(choose_shift([8192, 8192], all, 8192, PAINT_BUDGET_BYTES), 1);
    // 使うのが Color だけなら 8192² も縮めない（約 341 MiB）
    assert_eq!(choose_shift([8192, 8192], 4, 8192, PAINT_BUDGET_BYTES), 0);
    // 辺の上限が先に効く（GPU の上限 4096 で 8192² は 1 段）。予算が無限でも
    assert_eq!(choose_shift([8192, 8192], 1, 4096, u64::MAX), 1);
    assert_eq!(choose_shift([8192, 4096], 1, 4096, u64::MAX), 1);
    // 奇数の大きさは切り上げて数える
    assert_eq!(choose_shift([4097, 100], 1, 4096, u64::MAX), 1);
    // 縮めたあとのバイト数は必ず予算以下（1 × 1 で止まる場合を除く）
    for budget in [1u64 << 20, 5 << 20, 64 << 20] {
        for per_texel in [1u64, 4, 15] {
            let shift = choose_shift([6000, 5000], per_texel, 8192, budget);
            let reduced = [6000u32.div_ceil(1 << shift), 5000u32.div_ceil(1 << shift)];
            assert!(
                mip_bytes(reduced, per_texel) <= budget,
                "{budget} {per_texel} {shift}"
            );
            if shift > 0 {
                let before = [
                    6000u32.div_ceil(1 << (shift - 1)),
                    5000u32.div_ceil(1 << (shift - 1)),
                ];
                assert!(mip_bytes(before, per_texel) > budget, "最小の段");
            }
        }
    }
    // 予算が 1 テクセルにも足りなくても、止まる（1 × 1）
    assert_eq!(choose_shift([4, 4], 15, 8192, 1), 2);
}

#[test]
fn merging_the_plain_ranges_of_each_level_keeps_the_union() {
    let mut held: Vec<Option<[u32; 4]>> = Vec::new();
    merge_levels(&mut held, &[None, Some([2, 2, 5, 5]), None]);
    assert_eq!(held, vec![None, Some([2, 2, 5, 5]), None]);
    merge_levels(
        &mut held,
        &[
            None,
            Some([4, 0, 9, 3]),
            Some([1, 1, 2, 2]),
            Some([0, 0, 1, 1]),
        ],
    );
    assert_eq!(
        held,
        vec![
            None,
            Some([2, 0, 9, 5]),
            Some([1, 1, 2, 2]),
            Some([0, 0, 1, 1])
        ]
    );
    merge_levels(&mut held, &[]);
    assert_eq!(held.len(), 4);
}

#[test]
fn a_failed_coarse_composite_keeps_the_requested_tiles_for_the_exact_upload() {
    let tile = |x, y| TileCoord { x, y };
    let requested = [tile(0, 0), tile(1, 0)];
    // 上げたタイルがあれば、それ（まとめた矩形の中の全部）を覚える
    assert_eq!(
        held_after_coarse(vec![tile(0, 0), tile(1, 0), tile(2, 0)], &requested),
        vec![tile(0, 0), tile(1, 0), tile(2, 0)]
    );
    // 何も上げなかったら、頼んだタイルを覚える
    assert_eq!(
        held_after_coarse(Vec::new(), &requested),
        requested.to_vec()
    );
}

#[test]
fn the_weight_byte_keeps_the_shrink_of_common_documents() {
    // 塗り広げるセットは、使っているチャンネルの 1 テクセルのバイト数に重みの絵の 1 を足して数える（`bytes_per_texel_planned`）
    let with_weights = |channels: u64| channels + 1;
    // 4096² の 6 チャンネル（15 → 16 B）は収まる（約 341 MiB）。8192² の 6 チャンネルは今までどおり 1 段縮めて 4096²
    assert_eq!(
        choose_shift([4096, 4096], with_weights(15), 8192, PAINT_BUDGET_BYTES),
        0
    );
    assert_eq!(
        choose_shift([8192, 8192], with_weights(15), 8192, PAINT_BUDGET_BYTES),
        1
    );
    // 8192² の Color だけ（4 → 5 B、約 427 MiB）は収まる。Color と Roughness（5 → 6 B）は予算に 2 バイトだけ残して収まる
    assert_eq!(
        choose_shift([8192, 8192], with_weights(4), 8192, PAINT_BUDGET_BYTES),
        0
    );
    assert_eq!(
        choose_shift([8192, 8192], with_weights(5), 8192, PAINT_BUDGET_BYTES),
        0
    );
    assert_eq!(mip_bytes([8192, 8192], 6), PAINT_BUDGET_BYTES - 2);
    // 重みの 1 B で境を越えるのは、8192² で 1 テクセル 6 B（Color と 2 つのスカラーのチャンネル）だったセットだけ（前は予算に 2 バイトの余りで収まっていた）
    assert_eq!(choose_shift([8192, 8192], 6, 8192, PAINT_BUDGET_BYTES), 0);
    assert_eq!(
        choose_shift([8192, 8192], with_weights(6), 8192, PAINT_BUDGET_BYTES),
        1
    );
}

#[test]
fn a_cap_on_the_side_shrinks_other_sets_like_the_texture_limit() {
    // ほかのセットの上限（辺 1024）: 4096² は 1/4、2048² は 1/2、1024² 以下は縮めない。予算には依らない（u64::MAX）
    assert_eq!(choose_shift([4096, 4096], 15, 1024, u64::MAX), 2);
    assert_eq!(choose_shift([2048, 2048], 15, 1024, u64::MAX), 1);
    assert_eq!(choose_shift([1024, 1024], 15, 1024, u64::MAX), 0);
    assert_eq!(choose_shift([1000, 700], 15, 1024, u64::MAX), 0);
    // 細長い絵は長い辺で決まる
    assert_eq!(choose_shift([4096, 512], 4, 1024, u64::MAX), 2);
    // 縮めた後のバイト数（持つかどうかの計画が見積もる値）: 4096² の Color だけは 1024² のミップ込み
    let reduced = [1024u32, 1024];
    assert_eq!(mip_bytes(reduced, 4), 4 * 1_398_101);
}

#[test]
fn boxes_partition_each_level_into_nonempty_groups_of_two_or_three() {
    // 辺 n の段の画素は、辺 m = max(n / 2, 1) の段の箱へ重ならず隙間なく入り、箱の幅は 2〜3（n = 1 は 1）。`box_of` と `box_start` が逆
    for n in 1..=130u32 {
        let m = (n / 2).max(1);
        assert_eq!(box_start(0, n, m), 0);
        assert_eq!(box_start(m, n, m), n, "n={n}");
        for o in 0..m {
            let (a, b) = (box_start(o, n, m), box_start(o + 1, n, m));
            assert!(b > a, "n={n} o={o}: 空の箱");
            if n >= 2 {
                assert!((2..=3).contains(&(b - a)), "n={n} o={o}: 幅 {}", b - a);
            } else {
                assert_eq!(b - a, 1);
            }
            for y in a..b {
                assert_eq!(box_of(y, n, m), o, "n={n} y={y}");
            }
        }
    }
}

#[test]
fn the_pull_reads_a_positive_texel_next_to_any_positive_texel_at_every_size() {
    // 重みが 0 でないテクセル y のとなり x（±1）が引くとき、読む 2 つ（双線形の左右）の中に y の箱（重みが 0 でない）が入る
    for n in 2..=130u32 {
        let m = (n / 2).max(1);
        for y in 0..n {
            let parent = box_of(y, n, m) as i64;
            for x in [y.saturating_sub(1), y, (y + 1).min(n - 1)] {
                // 読み位置（粗い段のテクセルの座標。真ん中が 0.5）。シェーダーの式と同じ
                let f = (x as f32 + 0.5) * m as f32 / n as f32 - 0.5;
                let (a, b) = (f.floor() as i64, f.floor() as i64 + 1);
                let (a, b) = (a.clamp(0, m as i64 - 1), b.clamp(0, m as i64 - 1));
                assert!(
                    parent == a || parent == b,
                    "n={n} y={y} x={x}: 箱 {parent}、読む {a} {b}"
                );
            }
        }
    }
}

#[test]
fn push_and_pull_rects_cover_everything_a_change_can_reach() {
    // 段 n の画素の範囲を押すと、その画素の箱が全部入る。粗い段の範囲を読みうる段 n のテクセルは、全部 `children_rect` の中
    for n in [1u32, 2, 3, 5, 8, 17, 64, 65, 100] {
        let m = (n / 2).max(1);
        for x0 in 0..n {
            for x1 in x0 + 1..=n.min(x0 + 9) {
                let r = parent_rect([x0, 0, x1, 1], (n, 1), (m, 1));
                for y in x0..x1 {
                    assert!(r[0] <= box_of(y, n, m) && box_of(y, n, m) < r[2]);
                }
            }
        }
        for a in 0..m {
            for b in a + 1..=m.min(a + 5) {
                let r = children_rect([a, 0, b, 1], (n, 1), (m, 1));
                for x in 0..n {
                    let f = (x as f32 + 0.5) * m as f32 / n as f32 - 0.5;
                    let taps = [
                        (f.floor() as i64).clamp(0, m as i64 - 1),
                        (f.floor() as i64 + 1).clamp(0, m as i64 - 1),
                    ];
                    if taps.iter().any(|&t| (a as i64..b as i64).contains(&t)) {
                        assert!(r[0] <= x && x < r[2], "n={n} 範囲 {a}..{b} x={x}: {r:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn level_zero_weights_are_whole_boxes_and_the_pyramid_keeps_any_positive_child() {
    // 8 × 8 の文書の中央 4 × 4 が覆い、塗り広げの幅 1（全部で 6 × 6）。縮め 1（4 × 4 の絵）: 箱 2 × 2 が全部中のテクセルだけ 255
    let size = 8u32;
    let keep: Vec<bool> = (0..size * size)
        .map(|i| (2..6).contains(&(i % size)) && (2..6).contains(&(i / size)))
        .collect();
    let rings = Rings::new(size, size, &keep, 1).unwrap();
    let flat = coverage_weights(&rings, 0, [8, 8]);
    assert_eq!(
        flat.iter().filter(|&&w| w == 255).count(),
        36,
        "覆い 16 + 塗り広げ 20"
    );
    assert!(flat.iter().all(|&w| w == 0 || w == 255));
    let boxed = coverage_weights(&rings, 1, [4, 4]);
    // 6 × 6（1..7）の中に 2 × 2 の箱で全部入るのは、箱の x・y が 1..3 の 2 × 2 = 4 個だけ（残りは一部だけ中）
    assert_eq!(boxed.iter().filter(|&&w| w == 255).count(), 4);
    assert!(boxed.iter().all(|&w| w == 0 || w == 255));
    // 端の欠けた箱（5 × 5 の文書を縮め 1 で 3 × 3）は中の画素だけで見る: 右上の箱は 1 画素で、それが中なら 255
    let keep5 = vec![true; 25];
    let rings5 = Rings::new(5, 5, &keep5, 0).unwrap();
    assert!(coverage_weights(&rings5, 1, [3, 3])
        .iter()
        .all(|&w| w == 255));
    // ピラミッド: 子に 1 つでも重みがあれば最低 1、全部 255 なら 255、全部 0 なら 0
    let pyramid = weight_pyramid(
        vec![0, 0, 0, 255, 255, 0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 255],
        [4, 4],
        3,
    );
    assert_eq!(pyramid.len(), 3);
    // 255 を 1 つ含む 2 × 2 は 255 / 4 の切り上げで 64、2 つなら 510 / 4 の切り上げで 128
    assert_eq!(pyramid[1], vec![64, 64, 128, 128]);
    assert_eq!(pyramid[2], vec![96]);
    let tiny = weight_pyramid(vec![1, 0, 0, 0], [2, 2], 2);
    assert_eq!(tiny[1], vec![1], "1 つでも重みがあれば最低 1（切り上げ）");
}

#[test]
fn slots_match_standard_channels_and_defaults_match_the_export() {
    for slot in Slot::ALL {
        assert_eq!(Slot::of(slot.channel()), Some(slot));
    }
    assert_eq!(Slot::of(Channel::from_index(6).unwrap()), None);
    // 使っていないチャンネルの既定: Roughness は Standard の平滑度 0.5、法線は平ら
    assert_eq!(Slot::Roughness.default_texel()[0], 128);
    assert_eq!(&Slot::Normal.default_texel()[..3], &[128, 128, 255]);
    assert_eq!(Slot::Metallic.default_texel()[0], 0);
    assert_eq!(&Slot::Emission.default_texel()[..3], &[0, 0, 0]);
}
