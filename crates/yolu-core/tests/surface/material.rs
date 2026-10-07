#![allow(clippy::chunks_exact_to_as_chunks)]
use yolu_core::{glam::DVec2, material::ChannelPaint, *};
fn material() -> Vec<ChannelPaint> {
    Channel::ALL
        .iter()
        .enumerate()
        .map(|(i, c)| ChannelPaint::new(*c, Rgba8::new(31 + i as u8 * 32, 79, 133, 211)))
        .collect()
}
fn feed(s: &mut Stroke, d: &mut Document) {
    for (x, y, p) in [(4.2, 5.6, 0.4), (19.3, 11.2, 1.0), (29.5, 22.1, 0.7)] {
        s.add_point(d, x, y, p, DVec2::ZERO).unwrap();
    }
}
fn bytes(d: &Document, l: LayerId, c: Channel) -> Vec<u8> {
    d.layer(l).unwrap().surface(c).unwrap().to_canvas_bytes()
}
#[test]
fn channels_match_single_strokes_and_undo() {
    for degree in [1, 4] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(degree)
            .build()
            .unwrap()
            .install(|| {
                for complex in [false, true] {
                    let mut d = Document::with_tile_size(40, 32, 8).unwrap();
                    let l = d.add_layer("塗り").unwrap();
                    d.clear_history().unwrap();
                    let mut b = Brush::from(BrushSettings {
                        radius: 7.0,
                        hardness: 0.4,
                        opacity: 0.85,
                        flow: 0.45,
                        ..Default::default()
                    });
                    if complex {
                        b.seed = 1234;
                        b.jitter.size = 0.3;
                        b.color.hue = 0.5;
                        b.assist.curve = true;
                        b.assist.taper_out = 4.0;
                    }
                    let mut s = d.begin_material_brush_stroke(l, &material(), &b).unwrap();
                    feed(&mut s, &mut d);
                    d.end_stroke(s).unwrap();
                    let saved: Vec<_> = Channel::ALL.iter().map(|c| bytes(&d, l, *c)).collect();
                    for m in material() {
                        let mut one = Document::with_tile_size(40, 32, 8).unwrap();
                        let ol = one.add_layer("塗り").unwrap();
                        one.set_channel_enabled(ol, m.channel, true).unwrap();
                        let mut single = b.clone();
                        single.base.color = m.value;
                        let mut s = one.begin_brush_stroke_in(ol, m.channel, &single).unwrap();
                        feed(&mut s, &mut one);
                        one.end_stroke(s).unwrap();
                        assert_eq!(bytes(&d, l, m.channel), bytes(&one, ol, m.channel));
                    }
                    assert_eq!(d.undo_count(), 1);
                    d.undo().unwrap();
                    assert!(d.layer(l).unwrap().surface(Channel::Normal).is_none());
                    assert_eq!(d.allocated_bytes(), 0);
                    d.redo().unwrap();
                    for (c, b) in Channel::ALL.iter().zip(saved) {
                        assert_eq!(bytes(&d, l, *c), b);
                    }
                }
            });
    }
}
#[test]
fn cancel_and_failures_restore_every_channel() {
    for budget in [None, Some(500)] {
        let mut d = Document::with_tile_size(40, 32, 8).unwrap();
        let l = d.add_layer("塗り").unwrap();
        d.clear_history().unwrap();
        if let Some(n) = budget {
            d.set_stroke_budget_bytes(n).unwrap();
        }
        let mut s = d
            .begin_material_stroke(l, &material(), &BrushSettings::default())
            .unwrap();
        let r = s.add_point(&mut d, 10.0, 10.0, 1.0, DVec2::ZERO);
        if budget.is_some() {
            assert_eq!(r, Err(CoreError::StrokeBudgetExceeded));
        } else {
            r.unwrap();
            d.cancel_stroke(s);
        }
        assert_eq!(d.allocated_bytes(), 0);
        assert_eq!(d.undo_count(), 0);
        assert!(d.layer(l).unwrap().surface(Channel::Normal).is_none());
    }
}
/// 1 つの点で塗ったストロークの巻き戻しのバイト数（元が空のタイルへ、既定の半径 16 のダブ）。
fn rollback_for(channels: &[ChannelPaint]) -> u64 {
    let mut d = Document::with_tile_size(40, 32, 8).unwrap();
    let l = d.add_layer("塗り").unwrap();
    let mut s = d
        .begin_material_stroke(l, channels, &BrushSettings::default())
        .unwrap();
    s.add_point(&mut d, 10.0, 10.0, 1.0, DVec2::ZERO).unwrap();
    let bytes = d.active_stroke_stats().unwrap().rollback_bytes;
    d.cancel_stroke(s);
    bytes
}
/// ストロークの予算は全チャンネルの合計で守る（チャンネルごとに別々の予算を持たない）。1 チャンネルのストロークが通る予算でも、
/// 6 チャンネルは合計で超えて断られ、層・チャンネル・履歴は元に戻る。境目は合計と同じ値で通り、1 バイト少なければ断る。
#[test]
fn the_stroke_budget_is_shared_by_all_channels() {
    let one: Vec<_> = material().iter().map(|m| rollback_for(&[*m])).collect();
    let all = rollback_for(&material());
    // 元が空のタイルで、各チャンネルの量は同じ。6 チャンネルはその和（覆いも写しもチャンネルごと）
    assert!(one.iter().all(|b| *b == one[0] && *b > 0));
    assert_eq!(all, one.iter().sum::<u64>());
    // 1 チャンネルだけなら通る予算（どのチャンネルも自分の分は収まる）。全部を合わせると超える
    let budget = one[0] + 1;
    assert!(all > budget);
    for m in material() {
        let mut d = Document::with_tile_size(40, 32, 8).unwrap();
        let l = d.add_layer("塗り").unwrap();
        d.clear_history().unwrap();
        d.set_stroke_budget_bytes(budget).unwrap();
        let mut s = d
            .begin_material_stroke(l, &[m], &BrushSettings::default())
            .unwrap();
        s.add_point(&mut d, 10.0, 10.0, 1.0, DVec2::ZERO).unwrap();
        d.end_stroke(s).unwrap();
        assert_eq!(d.undo_count(), 1);
    }
    let mut d = Document::with_tile_size(40, 32, 8).unwrap();
    let l = d.add_layer("塗り").unwrap();
    d.clear_history().unwrap();
    d.set_stroke_budget_bytes(budget).unwrap();
    let before = snapshot(&d, l);
    let mut s = d
        .begin_material_stroke(l, &material(), &BrushSettings::default())
        .unwrap();
    assert_eq!(
        s.add_point(&mut d, 10.0, 10.0, 1.0, DVec2::ZERO),
        Err(CoreError::StrokeBudgetExceeded)
    );
    assert_eq!(d.allocated_bytes(), 0);
    assert_eq!(d.undo_count(), 0);
    assert!(!d.has_active_stroke());
    assert!(!d.layer(l).unwrap().is_channel_enabled(Channel::Normal));
    assert_eq!(snapshot(&d, l), before);
    // 境目: 合計の量で通り、1 バイト少なければ断る
    for (budget, ok) in [(all, true), (all - 1, false)] {
        let mut d = Document::with_tile_size(40, 32, 8).unwrap();
        let l = d.add_layer("塗り").unwrap();
        d.set_stroke_budget_bytes(budget).unwrap();
        let mut s = d
            .begin_material_stroke(l, &material(), &BrushSettings::default())
            .unwrap();
        let r = s.add_point(&mut d, 10.0, 10.0, 1.0, DVec2::ZERO);
        assert_eq!(r.is_ok(), ok, "{budget}");
        if ok {
            d.cancel_stroke(s);
        }
        assert!(!d.has_active_stroke());
        assert_eq!(d.allocated_bytes(), 0);
    }
}
#[test]
fn rejects_bad_material_and_layer() {
    let mut d = Document::new(8, 8).unwrap();
    let l = d.add_layer("塗り").unwrap();
    assert!(d
        .begin_material_stroke(l, &[], &BrushSettings::default())
        .is_err());
    assert!(d
        .begin_material_stroke(l, &[material()[0]; 2], &BrushSettings::default())
        .is_err());
    let s = d
        .begin_material_stroke(l, &material(), &BrushSettings::default())
        .unwrap();
    assert_eq!(d.undo(), Err(CoreError::StrokeActive));
    d.cancel_stroke(s);
}
#[test]
fn no_op_keeps_redo_and_disabled_channels() {
    let mut d = Document::new(8, 8).unwrap();
    let l = d.add_layer("塗り").unwrap();
    d.set_layer_name(l, "別名").unwrap();
    d.undo().unwrap();
    let n = d.redo_count();
    let s = d
        .begin_material_stroke(l, &material(), &BrushSettings::default())
        .unwrap();
    assert!(!d.end_stroke(s).unwrap().changed);
    assert_eq!(d.redo_count(), n);
    assert!(d.layer(l).unwrap().surface(Channel::Normal).is_none());
}
#[test]
fn uv_layout_compare_counts_overlap_and_keeps_winding() {
    use yolu_core::uv_layout::*;
    let a = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
    // 頂点の回し方と三角形の順は同じ、向き（巻き）は別
    assert!(same_triangles(&[a], &[[a[1], a[2], a[0]]]));
    assert!(!same_triangles(&[a], &[[a[0], a[2], a[1]]]));
    let same = compare(&[a], &[[a[2], a[0], a[1]]], 16).unwrap();
    assert!(same.same && same.kept == 1.0 && same.added == 0.0);
    // 向きだけ違う集まりは別物だが、覆うテクセルは同じ
    let flipped = compare(&[a], &[[a[0], a[2], a[1]]], 16).unwrap();
    assert!(!flipped.same && flipped.kept == 1.0 && flipped.added == 0.0);
    // 前だけが覆う → 何も残らない。新しい側が何も覆わなければ added は 0（0 で割らない）
    let gone = compare(&[a], &[], 16).unwrap();
    assert!(!gone.same && gone.kept == 0.0 && gone.added == 0.0);
    // 前が何も覆わなければ kept は 1。増えた分は全部
    let grown = compare(&[], &[a], 16).unwrap();
    assert!(!grown.same && grown.kept == 1.0 && grown.added == 1.0);
    // 斜めの辺を共有する向かいの半分: 4×4 のテクセルで、前の 13 テクセルのうち 10 が重なる（辺に触れるだけのテクセルも数える）
    let opposite = compare(&[a], &[[[1.0, 1.0], [0.0, 1.0], [1.0, 0.0]]], 4).unwrap();
    assert!(!opposite.same);
    assert_eq!(opposite.kept, 10.0 / 13.0);
    assert_eq!(opposite.added, 3.0 / 13.0);
    // 同じ三角形が 2 つ入った集まりは、1 つの集まりとは別（数が違う）。覆いは同じ
    let twice = compare(&[a], &[a, a], 16).unwrap();
    assert!(!twice.same && twice.kept == 1.0 && twice.added == 0.0);
}
#[test]
fn uv_layout_compare_refuses_resolutions_outside_one_to_4096() {
    use yolu_core::uv_layout::*;
    let a = [[0.0, 0.0], [0.5, 0.0], [0.0, 0.5]];
    let b = [[0.1, 0.1], [0.4, 0.1], [0.1, 0.4]];
    assert_eq!(MAX_RESOLUTION, 4096);
    for res in [0, MAX_RESOLUTION + 1, u32::MAX] {
        assert!(compare(&[a], &[b], res).is_err(), "{res}");
        // 同じ集まりでも解像度を先に断る
        assert!(compare(&[a], &[a], res).is_err(), "{res}");
    }
    for res in [1, MAX_RESOLUTION] {
        assert!(compare(&[a], &[b], res).is_ok(), "{res}");
    }
    // 解像度 1: 1 つのテクセルを両方が覆う
    let one = compare(&[a], &[b], 1).unwrap();
    assert!(!one.same && one.kept == 1.0 && one.added == 0.0);
    assert_eq!(DEFAULT_RESOLUTION, 256);
}
#[test]
fn uv_layout_compare_skips_non_finite_triangles_and_pins_the_huge_quantization() {
    use yolu_core::uv_layout::*;
    let b = [[0.1, 0.2], [0.8, 0.1], [0.6, 0.7]];
    let nan = [[f64::NAN, 0.0], [1.0, 0.0], [0.0, 1.0]];
    let inf = [[0.0, f64::INFINITY], [1.0, 0.0], [0.0, 1.0]];
    let same_place_inf = [[f64::INFINITY, 0.0], [1.0, 0.0], [0.0, 1.0]];
    let same_place_neg_inf = [[f64::NEG_INFINITY, 0.0], [1.0, 0.0], [0.0, 1.0]];
    // 判定（丸め）では、NaN と無限大の座標は同じ値（i64::MIN）になる。同じ場所の座標が非有限どうしなら同じ集まりとして数える
    assert!(same_triangles(&[nan], &[same_place_inf]));
    assert!(same_triangles(&[nan], &[same_place_neg_inf]));
    assert!(!same_triangles(&[nan], &[inf]));
    assert!(!same_triangles(&[nan], &[b]));
    // 被覆では非有限の三角形を飛ばす: 片方にだけあっても残りの三角形の覆いは変わらない
    let r = compare(&[nan, b], &[b, inf], 16).unwrap();
    assert!(!r.same && r.kept == 1.0 && r.added == 0.0);
    // 非有限だけの集まりは何も覆わない
    let none = compare(&[nan, inf], &[b], 16).unwrap();
    assert!(!none.same && none.kept == 1.0 && none.added == 1.0);
    let both_none = compare(&[nan, inf], &[], 16).unwrap();
    assert!(!both_none.same && both_none.kept == 1.0 && both_none.added == 0.0);
    // 丸めた値が i64 に収まらない有限の座標も、NaN と同じ値（i64::MIN）にする。C# の `(long)` の変換は環境で結果が違うので、
    // これは Rust 側の決めの固定で、C# との照合ではない
    let huge = [[1e300, 0.0], [1.0, 0.0], [0.0, 1.0]];
    assert!(same_triangles(&[huge], &[nan]));
    assert!(same_triangles(
        &[huge],
        &[[[-1e300, 0.0], [1.0, 0.0], [0.0, 1.0]]]
    ));
    assert!(!same_triangles(&[huge], &[b]));
}
#[test]
fn manual_id_colors_undo() {
    let mut d = Document::new(8, 8).unwrap();
    let colors =
        mesh_maps::IdColorAssignments::new("a".repeat(64), [(0, 0x123456)].into()).unwrap();
    let key = colors.key();
    d.set_id_colors(colors, false).unwrap();
    assert_eq!(d.id_colors().key(), key);
    d.undo().unwrap();
    assert_eq!(d.id_colors().key(), "");
    d.redo().unwrap();
    assert_eq!(d.id_colors().key(), key);
}
/// 色の窓のドラッグ: まとめた手動 ID 色の変更は 1 回の Undo（戻すと最初の変更の前＝自動、やり直すと最後の色）。まとめを切った後・ほかの
/// 変更の後は別の段。ドラッグを Escape で止めると、その段ごと捨てる。保存用の写しは最後の色を持つ。
#[test]
fn dragged_manual_id_colors_coalesce_into_one_undo_step() {
    let binding = "a".repeat(64);
    let with =
        |rgb: u32| mesh_maps::IdColorAssignments::new(binding.clone(), [(0, rgb)].into()).unwrap();
    let color = |d: &Document| d.id_colors().colors().get(&0).copied();
    let mut d = Document::new(8, 8).unwrap();
    let layer = d.add_layer("a").unwrap();
    let before = d.history_bytes();
    d.set_id_colors(with(0x112233), true).unwrap();
    let first = d.history_bytes() - before;
    for rgb in [0x445566, 0x778899] {
        d.set_id_colors(with(rgb), true).unwrap();
    }
    assert!(d.is_coalescing());
    assert_eq!(d.undo_count(), 2);
    assert_eq!(color(&d), Some(0x778899));
    // まとめた段の費用は最初の変更のまま（ほかのまとめと同じ）
    assert_eq!(d.history_bytes() - before, first);
    // 同じ色は段を積まない
    d.set_id_colors(with(0x778899), true).unwrap();
    assert_eq!(d.undo_count(), 2);
    d.end_coalescing();
    assert!(!d.is_coalescing());
    // 切った後の変更は別の段
    d.set_id_colors(with(0xabcdef), true).unwrap();
    // ほかの変更の後のまとめも別の段
    d.set_layer_opacity(layer, 0.5, false).unwrap();
    d.set_id_colors(with(0x010203), true).unwrap();
    d.end_coalescing();
    assert_eq!(d.undo_count(), 5);
    // 保存用の写しは最後の色
    assert_eq!(color(&d.capture_snapshot().unwrap()), Some(0x010203));
    for expected in [Some(0xabcdef), Some(0xabcdef), Some(0x778899), None] {
        d.undo().unwrap();
        assert_eq!(color(&d), expected);
    }
    assert_eq!(d.id_colors().key(), "");
    for expected in [
        Some(0x778899),
        Some(0xabcdef),
        Some(0xabcdef),
        Some(0x010203),
    ] {
        d.redo().unwrap();
        assert_eq!(color(&d), expected);
    }
    // ドラッグを Escape で止める: まとめた段を戻して捨てる
    let steps = d.undo_count();
    d.set_id_colors(with(0x0f0f0f), true).unwrap();
    d.set_id_colors(with(0xf0f0f0), true).unwrap();
    assert!(d.cancel_coalescing().unwrap());
    assert_eq!((d.undo_count(), color(&d)), (steps, Some(0x010203)));
    // まとめは手動 ID 色の変更どうしだけ（不透明度のドラッグの段に混ざらない）
    d.set_layer_opacity(layer, 0.25, true).unwrap();
    d.set_id_colors(with(0x123456), true).unwrap();
    d.end_coalescing();
    assert_eq!(d.undo_count(), steps + 2);
    d.undo().unwrap();
    assert_eq!(
        (color(&d), d.layer(layer).unwrap().opacity()),
        (Some(0x010203), 0.25)
    );
}
fn quad() -> [material_triangles::PixelTriangle; 2] {
    [
        [
            DVec2::new(0.0, 0.0),
            DVec2::new(8.0, 0.0),
            DVec2::new(8.0, 8.0),
        ],
        [
            DVec2::new(0.0, 0.0),
            DVec2::new(8.0, 8.0),
            DVec2::new(0.0, 8.0),
        ],
    ]
}
#[test]
fn triangle_union_has_no_seams_and_order_does_not_matter() {
    for reversed in [false, true] {
        let mut d = Document::with_tile_size(8, 8, 4).unwrap();
        let l = d.add_layer("塗り").unwrap();
        d.clear_history().unwrap();
        let mut q = quad();
        if reversed {
            q.reverse();
        }
        let mut f = d
            .begin_material_triangle_fill(l, &material(), 0.5, false)
            .unwrap();
        f.add(&mut d, &q[..1]).unwrap();
        assert!(!f.add(&mut d, &q[..1]).unwrap());
        f.add(&mut d, &q[1..]).unwrap();
        assert_eq!(f.triangles_added(&d), Some(3));
        assert_eq!(d.active_stroke_stats().unwrap().tiles, 24);
        f.commit(&mut d).unwrap();
        for m in material() {
            let expected = blend::blend(Rgba8::TRANSPARENT, m.value, 0.5, BlendMode::Normal);
            assert!(bytes(&d, l, m.channel)
                .chunks_exact(4)
                .all(|p| p == expected.to_array()));
        }
        assert_eq!(d.undo_count(), 1);
        d.undo().unwrap();
        assert_eq!(d.allocated_bytes(), 0);
        d.redo().unwrap();
        assert!(d.allocated_bytes() > 0);
    }
}
#[test]
fn triangle_failure_and_cancel_restore_pixels_and_setup() {
    for mode in 0..3 {
        let mut d = Document::with_tile_size(8, 8, 4).unwrap();
        let l = d.add_layer("塗り").unwrap();
        d.clear_history().unwrap();
        if mode == 2 {
            d.set_stroke_budget_bytes(300).unwrap();
        }
        let mut f = d
            .begin_material_triangle_fill(l, &material(), 0.5, false)
            .unwrap();
        if mode == 2 {
            assert_eq!(f.add(&mut d, &quad()), Err(CoreError::StrokeBudgetExceeded));
        } else {
            f.add(&mut d, &quad()[..1]).unwrap();
            if mode == 0 {
                f.cancel(&mut d);
            } else {
                assert!(f.add(&mut d, &[[DVec2::NAN; 3]]).is_err());
            }
        }
        assert!(!d.has_active_stroke());
        assert_eq!(d.allocated_bytes(), 0);
        assert_eq!(d.undo_count(), 0);
        assert!(d.layer(l).unwrap().surface(Channel::Normal).is_none());
    }
}
#[test]
fn region_budgets_and_redo_are_atomic() {
    for source in [false, true] {
        let mut d = Document::with_tile_size(8, 8, 4).unwrap();
        let l = d.add_layer("塗り").unwrap();
        d.clear_history().unwrap();
        if source {
            d.set_source_budget_bytes(20).unwrap();
        } else {
            d.set_stroke_budget_bytes(300).unwrap();
        }
        assert!(d.fill_material(l, &material(), 0.5, None, false).is_err());
        assert_eq!(d.allocated_bytes(), 0);
        assert_eq!(d.undo_count(), 0);
        assert!(d.layer(l).unwrap().surface(Channel::Normal).is_none());
    }
    let mut d = Document::with_tile_size(8, 8, 4).unwrap();
    let l = d.add_layer("塗り").unwrap();
    d.clear_history().unwrap();
    d.fill_material(l, &material(), 0.5, None, false).unwrap();
    d.undo().unwrap();
    d.set_source_budget_bytes(20).unwrap();
    assert_eq!(d.redo(), Err(CoreError::SourceBudgetExceeded));
    assert_eq!(d.allocated_bytes(), 0);
    assert_eq!(d.redo_count(), 1);
    assert!(d.layer(l).unwrap().surface(Channel::Normal).is_none());
}
/// C# の MaterialRegionTests.ARefusedUndoDoesNotRestoreAnyOfTheDenseChannels: 6 チャンネルとも画素が密な層を `fill_material` で
/// 一様に塗ってから予算を縮めると、Undo は全チャンネルの増分を先にまとめて確かめて断る。最初のチャンネルの分（252）だけなら
/// 入る予算でも、どのチャンネルも戻さない。予算が足りれば全チャンネルが元のバイトへ戻る。
#[test]
fn a_refused_region_undo_restores_none_of_the_dense_channels() {
    let mut d = Document::with_tile_size(8, 8, 8).unwrap();
    let l = d.add_layer("塗り").unwrap();
    for c in Channel::ALL {
        d.set_channel_enabled(l, c, true).unwrap();
        for y in 0..8 {
            for x in 0..8 {
                d.set_channel_pixel(l, c, x, y, Rgba8::new(x as u8, y as u8, 11, 255))
                    .unwrap();
            }
        }
    }
    d.clear_history().unwrap();
    let before = snapshot(&d, l);
    assert_eq!(d.allocated_bytes(), 6 * 256);
    let paint: Vec<_> = Channel::ALL
        .iter()
        .map(|c| ChannelPaint::new(*c, Rgba8::new(200, 30, 50, 255)))
        .collect();
    assert!(d.fill_material(l, &paint, 1.0, None, false).unwrap());
    assert_eq!(
        d.allocated_bytes(),
        6 * 4,
        "一様に塗るとタイルは 4 バイトの一様になる"
    );
    let painted = snapshot(&d, l);
    assert_ne!(painted, before);
    let (revision, serial, history) = (d.revision(), d.change_serial(), d.history_bytes());
    // 1 チャンネル分（256 - 4）の増分は入るが、6 チャンネル分は入らない予算
    d.set_source_budget_bytes(d.allocated_bytes() + 256)
        .unwrap();
    for _ in 0..2 {
        assert_eq!(d.undo(), Err(CoreError::SourceBudgetExceeded));
        assert_eq!(snapshot(&d, l), painted, "どのチャンネルも戻らない");
        assert_eq!(
            (d.revision(), d.change_serial(), d.history_bytes()),
            (revision, serial, history)
        );
        assert_eq!((d.undo_count(), d.redo_count()), (1, 0));
    }
    // 5 チャンネル分までの予算でも、残りの 1 チャンネル分が足りないので断る
    d.set_source_budget_bytes(6 * 4 + 5 * 252).unwrap();
    assert_eq!(d.undo(), Err(CoreError::SourceBudgetExceeded));
    assert_eq!(snapshot(&d, l), painted);
    // ちょうど入る予算で通り、全チャンネルが元の画素へ戻る
    d.set_source_budget_bytes(6 * 256).unwrap();
    assert!(d.undo().unwrap());
    assert_eq!(snapshot(&d, l), before);
    assert_eq!((d.undo_count(), d.redo_count()), (0, 1));
    // やり直しは縮むので、予算を絞っても通る
    d.set_source_budget_bytes(6 * 256).unwrap();
    assert!(d.redo().unwrap());
    assert_eq!(snapshot(&d, l), painted);
}
#[test]
fn no_op_region_keeps_redo_and_disabled_channels() {
    let mut d = Document::new(8, 8).unwrap();
    let l = d.add_layer("塗り").unwrap();
    d.set_layer_name(l, "別名").unwrap();
    d.undo().unwrap();
    let n = d.redo_count();
    assert!(!d.fill_material(l, &material(), 0.0, None, false).unwrap());
    assert_eq!(d.redo_count(), n);
    assert!(d.layer(l).unwrap().surface(Channel::Normal).is_none());
}
#[test]
fn region_bad_inputs_refuse_before_enabling() {
    use material::GradientSettings;
    let mut d = Document::new(8, 8).unwrap();
    let l = d.add_layer("塗り").unwrap();
    let region = SelectionMask::empty(4, 4, 4).unwrap();
    assert!(d
        .fill_material(l, &material(), 0.5, Some(&region), false)
        .is_err());
    assert!(d
        .gradient_material(
            l,
            &material(),
            Some(&material()[..1]),
            &GradientSettings::default(),
            None,
            false
        )
        .is_err());
    assert!(d
        .fill_material(l, &material(), f64::NAN, None, false)
        .is_err());
    assert!(d.layer(l).unwrap().surface(Channel::Normal).is_none());
    let group = d.add_group("グループ", None).unwrap();
    assert!(d
        .fill_material(group, &material(), 1.0, None, false)
        .is_err());
    assert!(d
        .begin_material_stroke(group, &material(), &BrushSettings::default())
        .is_err());
}
#[test]
fn mask_triangle_fill_undo_and_cancel() {
    let mut d = Document::with_tile_size(8, 8, 4).unwrap();
    let l = d.add_layer("塗り").unwrap();
    d.add_layer_mask(l).unwrap();
    d.clear_history().unwrap();
    let mut f = d.begin_mask_triangle_fill(l, 0.5, false).unwrap();
    f.add(&mut d, &quad()).unwrap();
    f.commit(&mut d).unwrap();
    assert_eq!(d.undo_count(), 1);
    assert!(d
        .layer(l)
        .unwrap()
        .mask()
        .unwrap()
        .surface()
        .to_canvas_bytes()
        .chunks_exact(4)
        .all(|p| p == [0, 0, 0, 128]));
    d.undo().unwrap();
    assert_eq!(d.allocated_bytes(), 0);
    d.redo().unwrap();
    let before = d.allocated_bytes();
    let mut f = d.begin_mask_triangle_fill(l, 1.0, true).unwrap();
    f.add(&mut d, &quad()).unwrap();
    f.cancel(&mut d);
    assert_eq!(d.allocated_bytes(), before);
}
#[test]
fn manual_id_restore_has_no_history_or_revision() {
    let mut d = Document::new(8, 8).unwrap();
    let revision = d.revision();
    let colors =
        mesh_maps::IdColorAssignments::new("a".repeat(64), [(0, 0x123456)].into()).unwrap();
    d.restore_id_colors(colors.clone()).unwrap();
    assert_eq!(d.revision(), revision);
    assert_eq!(d.undo_count(), 0);
    assert_eq!(d.id_colors().key(), colors.key());
    d.add_layer("塗り").unwrap();
    assert!(d.restore_id_colors(colors).is_err());
}
#[test]
fn mask_gradient_known_answer_and_undo() {
    let mut d = Document::with_tile_size(8, 8, 4).unwrap();
    let l = d.add_layer("塗り").unwrap();
    d.add_layer_mask(l).unwrap();
    d.clear_history().unwrap();
    let g = material::GradientSettings {
        start: DVec2::ZERO,
        end: DVec2::new(8.0, 0.0),
        from: Rgba8::new(31, 79, 133, 255),
        to: Rgba8::TRANSPARENT,
        ..Default::default()
    };
    d.gradient_mask(l, &g, None, false).unwrap();
    assert_eq!(
        d.layer(l)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .pixel(0, 0)
            .unwrap()
            .a,
        239
    );
    assert_eq!(
        d.layer(l)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .pixel(7, 0)
            .unwrap()
            .a,
        16
    );
    assert_eq!(d.undo_count(), 1);
    d.undo().unwrap();
    assert_eq!(d.allocated_bytes(), 0);
}
/// 立方体（6 面が別の UV アイランド・12 三角形・材質 0）と、離れた所の 2 三角形（3D の位置は辺でつながるが UV は離れた別の
/// アイランド・材質 1）。三角形の番号は立方体が 0〜11、あとの 2 つが 12・13。
fn two_part_geometry() -> geometry::SurfaceGeometry {
    use geometry::*;
    use glam::{Vec2, Vec3};
    let apart = ModelMesh {
        name: "離れた面".into(),
        positions: vec![
            Vec3::new(5.0, 0.0, 0.0),
            Vec3::new(6.0, 0.0, 0.0),
            Vec3::new(5.0, 1.0, 0.0),
            Vec3::new(5.0, 1.0, 0.0),
            Vec3::new(6.0, 0.0, 0.0),
            Vec3::new(6.0, 1.0, 0.0),
        ],
        normals: Vec::new(),
        // 12: 左下が (0.25, 0.5)・右が 0.75・上が 1.0。13: 左下の隅の小さい三角形
        uvs: vec![
            Vec2::new(0.25, 0.5),
            Vec2::new(0.75, 0.5),
            Vec2::new(0.25, 1.0),
            Vec2::new(0.0, 0.0),
            Vec2::new(0.5, 0.0),
            Vec2::new(0.0, 0.25),
        ],
        submeshes: vec![Submesh {
            material: 1,
            indices: vec![0, 1, 2, 3, 4, 5],
        }],
    };
    SurfaceGeometry::new(model_triangles(&[demo_cube(), apart]).unwrap(), 1, 1e-5).unwrap()
}
#[test]
fn geometry_regions_have_the_known_sizes_and_members() {
    use geometry::{region, SurfaceRegionKind::*};
    let g = two_part_geometry();
    assert_eq!(g.triangles().len(), 14);
    let all_cube: Vec<u32> = (0..12).collect();
    // (開始, 種類) → 三角形の番号（昇順）
    for (start, kind, expected) in [
        (0, Triangle, vec![0]),
        (7, Triangle, vec![7]),
        (0, UvIsland, vec![0, 1]),
        (5, UvIsland, vec![4, 5]),
        (0, MeshPart, all_cube.clone()),
        (11, MeshPart, all_cube.clone()),
        (0, Material, all_cube.clone()),
        (12, Triangle, vec![12]),
        (12, UvIsland, vec![12]),
        (13, UvIsland, vec![13]),
        (12, MeshPart, vec![12, 13]),
        (13, MeshPart, vec![12, 13]),
        (12, Material, vec![12, 13]),
    ] {
        let mut got = region(&g, start, kind);
        got.sort_unstable();
        assert_eq!(got, expected, "{start} {kind:?}");
    }
    // 範囲外の番号は断る（最後の番号は通る）
    let d = Document::with_tile_size(32, 24, 8).unwrap();
    assert!(material_triangles::surface_triangles(&d, &g, 13, Triangle).is_ok());
    assert!(material_triangles::surface_triangles(&d, &g, 14, Triangle).is_err());
    assert!(SelectionMask::from_surface_region(&d, &g, 14, Triangle).is_err());
    assert!(SelectionMask::from_surface_region(&d, &g, u32::MAX, Triangle).is_err());
}
#[test]
fn surface_regions_map_uv_to_pixels_with_v_counted_from_the_bottom() {
    use geometry::SurfaceRegionKind::*;
    let g = two_part_geometry();
    // 幅と高さが違う画布で、x = u × 幅、y = v × 高さ（v = 0 が下の行）
    let d = Document::with_tile_size(32, 24, 8).unwrap();
    let corners = |t: &[DVec2; 3]| t.map(|p| (p.x, p.y));
    let a = material_triangles::surface_triangles(&d, &g, 12, Triangle).unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(corners(&a[0]), [(8.0, 12.0), (24.0, 12.0), (8.0, 24.0)]);
    let b = material_triangles::surface_triangles(&d, &g, 13, Triangle).unwrap();
    assert_eq!(corners(&b[0]), [(0.0, 0.0), (16.0, 0.0), (0.0, 6.0)]);
    // 同じ形でも、三角形の数は種類どおり（メッシュの塊は 2 つ・立方体の 1 面は 2 つ）
    assert_eq!(
        material_triangles::surface_triangles(&d, &g, 12, MeshPart)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        material_triangles::surface_triangles(&d, &g, 0, Material)
            .unwrap()
            .len(),
        12
    );
    // 選択範囲: 12 は (8,12)-(24,12)-(8,24) の中だけ。上半分にあり、下の行と右上の外は選ばれない
    let m = SelectionMask::from_surface_region(&d, &g, 12, Triangle).unwrap();
    assert_eq!(m, SelectionMask::from_triangles(&d, &a).unwrap());
    assert_eq!(m.amount(10, 13), 255);
    assert_eq!(m.amount(8, 20), 255);
    assert_eq!(m.amount(10, 3), 0);
    assert_eq!(m.amount(2, 13), 0);
    assert_eq!(m.amount(20, 22), 0);
    assert_eq!(m.amount(30, 23), 0);
    // UV アイランドは 12 だけ（13 は別のアイランド）。メッシュの塊は両方で、13 の小さい三角形も入る
    let island = SelectionMask::from_surface_region(&d, &g, 12, UvIsland).unwrap();
    assert_eq!(island, m);
    assert_eq!(island.amount(2, 1), 0);
    let part = SelectionMask::from_surface_region(&d, &g, 12, MeshPart).unwrap();
    assert_eq!(part.amount(10, 13), 255);
    assert_eq!(part.amount(2, 1), 255);
    assert_eq!(part.amount(2, 13), 0);
    // 材質 1 の 2 三角形（立方体の 12 は入らない）。立方体の 1 面は UV の (0.02..0.313, 0.03..0.47) の中だけ
    let material = SelectionMask::from_surface_region(&d, &g, 13, Material).unwrap();
    assert_eq!(material, part);
    let face = SelectionMask::from_surface_region(&d, &g, 0, UvIsland).unwrap();
    assert_eq!(face.amount(5, 5), 255);
    assert_eq!(face.amount(20, 5), 0);
    assert_eq!(face.amount(5, 20), 0);
    assert_eq!(face.amount(0, 15), 0);
    // 面の左下の隅（UV (0.02, 0.03) は画素 (0.64, 0.72)）は 16 サンプルのうち 1 つだけが入る
    assert_eq!(face.amount(0, 0), 16);
}
#[test]
fn manual_id_assignment_is_immutable_and_bound_to_model() {
    let empty = mesh_maps::IdColorAssignments::default();
    let binding = "a".repeat(64);
    let a = empty.with_color(&binding, 3, Some(0x123456)).unwrap();
    assert!(empty.colors().is_empty());
    assert_eq!(a.binding(), binding);
    assert_eq!(a.colors()[&3], 0x123456);
    assert!(a.with_color(&"b".repeat(64), 3, None).is_err());
    assert!(a.with_color(&binding, 2, Some(0x1000000)).is_err());
    let removed = a.with_color(&binding, 3, None).unwrap();
    assert!(removed.colors().is_empty());
    assert_eq!(removed.binding(), "");
    assert_eq!(removed.key(), "");
}
fn ragged() -> Document {
    // 端のタイルが欠ける画布（右の列は 5 画素・上の行は 5 画素だけ画布の中）
    Document::with_tile_size(37, 29, 8).unwrap()
}
fn tri(t: [(f64, f64); 3]) -> material_triangles::PixelTriangle {
    t.map(|(x, y)| DVec2::new(x, y))
}
fn canvas_quad() -> [material_triangles::PixelTriangle; 2] {
    [
        tri([(0.0, 0.0), (37.0, 0.0), (37.0, 29.0)]),
        tri([(0.0, 0.0), (37.0, 29.0), (0.0, 29.0)]),
    ]
}
fn pixel_amounts(m: &SelectionMask) -> Vec<u8> {
    m.to_canvas_bytes()
}
#[test]
fn triangle_union_known_answers_on_a_ragged_canvas() {
    let d = ragged();
    // 画布ぴったりの 2 枚: 継ぎ目も二重もなく画布の全画素が 255。欠けたタイルも画布の中が全部
    let quad = SelectionMask::from_triangles(&d, &canvas_quad()).unwrap();
    assert!(pixel_amounts(&quad).iter().all(|a| *a == 255));
    assert_eq!(quad.tile_coords().len(), 5 * 4);
    // 斜めの辺を共有する 2 つの半分は、画素ごとに量の合計が 255（サンプルは 1 つの三角形にだけ属する。丸めの 1 まで）
    let lower = SelectionMask::from_triangles(&d, &canvas_quad()[..1]).unwrap();
    let upper = SelectionMask::from_triangles(&d, &canvas_quad()[1..]).unwrap();
    for (a, b) in pixel_amounts(&lower).iter().zip(pixel_amounts(&upper)) {
        assert!((*a as i32 + b as i32 - 255).abs() <= 1, "{a} + {b}");
    }
    assert_eq!(lower.amount(36, 0), pixel_amounts(&lower)[36]);
    assert!(lower.amount(36, 0) > 0 && upper.amount(0, 28) > 0);
    // 画布の外・縮退・面積が極小: 何も選ばれない
    for t in [
        tri([(50.0, 5.0), (60.0, 5.0), (55.0, 20.0)]),
        tri([(5.0, -20.0), (15.0, -20.0), (10.0, -3.0)]),
        tri([(-30.0, 5.0), (-2.0, 5.0), (-10.0, 20.0)]),
        tri([(5.0, 40.0), (15.0, 40.0), (10.0, 60.0)]),
        tri([(1.0, 1.0), (10.0, 10.0), (20.0, 20.0)]),
        tri([(1.0, 1.0), (1.000001, 1.0), (1.0, 1.0000001)]),
        tri([(3.0, 3.0), (3.0, 3.0), (3.0, 3.0)]),
    ] {
        let m = SelectionMask::from_triangles(&d, &[t]).unwrap();
        assert!(m.is_empty(), "{t:?}");
        assert_eq!(m.allocated_bytes(), 0);
    }
    // 一部が画布の外: 外は切り、中は普通に塗る。画布の角（左下の外へ延びる三角形）
    let partial =
        SelectionMask::from_triangles(&d, &[tri([(-6.2, 10.0), (14.1, -4.4), (20.3, 31.8)])])
            .unwrap();
    assert_eq!(partial.amount(10, 10), 255);
    assert_eq!(partial.amount(36, 28), 0);
    assert_eq!(partial.amount(0, 0), 0);
    // 座標が巨大でも面積が溢れない大きさまでは、画布に切って画布の全画素を塗る（C# は int に収まらない座標を飛ばす差がある）
    for r in [1e6, 1e9, 1e150] {
        let m = SelectionMask::from_triangles(&d, &[tri([(-r, -r), (r, -r), (0.0, r)])]).unwrap();
        assert!(pixel_amounts(&m).iter().all(|a| *a == 255), "{r}");
    }
    // 面積が溢れる大きさ（1e300 ほか）は何も覆わず、panic も NaN の汚れもない。有限でない座標は断る
    for r in [1e200, 1e300, f64::MAX] {
        let m = SelectionMask::from_triangles(&d, &[tri([(-r, -r), (r, -r), (0.0, r)])]).unwrap();
        assert!(m.is_empty(), "{r}");
    }
    assert!(SelectionMask::from_triangles(
        &d,
        &[tri([(0.0, 0.0), (f64::INFINITY, 1.0), (1.0, 4.0)])]
    )
    .is_err());
}
#[test]
fn triangles_off_canvas_or_degenerate_leave_the_fill_and_setup_untouched() {
    let cases = [
        tri([(50.0, 5.0), (60.0, 5.0), (55.0, 20.0)]),
        tri([(5.0, -20.0), (15.0, -20.0), (10.0, -3.0)]),
        tri([(1.0, 1.0), (10.0, 10.0), (20.0, 20.0)]),
        tri([(1.0, 1.0), (1.000001, 1.0), (1.0, 1.0000001)]),
        tri([(-1e300, -1e300), (1e300, -1e300), (0.0, 1e300)]),
    ];
    for t in cases {
        let mut d = ragged();
        let l = d.add_layer("塗り").unwrap();
        d.clear_history().unwrap();
        let mut f = d
            .begin_material_triangle_fill(l, &material(), 0.5, false)
            .unwrap();
        let revision = d.revision();
        assert_eq!(f.add(&mut d, &[t]), Ok(false), "{t:?}");
        assert_eq!(d.revision(), revision);
        assert_eq!(f.triangles_added(&d), Some(1));
        assert_eq!(f.covered_tile_count(&d), Some(0));
        let stats = d.active_stroke_stats().unwrap();
        assert_eq!(stats.tiles, 0);
        // 続けて画布の中の三角形を足せる（状態が汚れていない）
        assert!(f.add(&mut d, &canvas_quad()).unwrap());
        assert_eq!(f.triangles_added(&d), Some(3));
        f.cancel(&mut d);
        assert_eq!(d.allocated_bytes(), 0);
        assert_eq!(d.undo_count(), 0);
        assert!(!d.has_active_stroke());
        assert!(!d.layer(l).unwrap().is_channel_enabled(Channel::Normal));
        // 何も覆わないまま確定しても、履歴・有効化・画素は変わらない
        let mut f = d
            .begin_material_triangle_fill(l, &material(), 0.5, false)
            .unwrap();
        f.add(&mut d, &[t]).unwrap();
        assert!(!f.commit(&mut d).unwrap().changed);
        assert_eq!(d.undo_count(), 0);
        assert_eq!(d.allocated_bytes(), 0);
        assert!(!d.layer(l).unwrap().is_channel_enabled(Channel::Normal));
    }
}
#[test]
fn fills_on_a_ragged_canvas_paint_exactly_the_canvas() {
    for use_fill in [false, true] {
        let mut d = ragged();
        let l = d.add_layer("塗り").unwrap();
        d.clear_history().unwrap();
        if use_fill {
            let region = SelectionMask::from_triangles(&d, &canvas_quad()).unwrap();
            assert!(d
                .fill_material(l, &material(), 0.5, Some(&region), false)
                .unwrap());
        } else {
            let mut f = d
                .begin_material_triangle_fill(l, &material(), 0.5, false)
                .unwrap();
            f.add(&mut d, &canvas_quad()[..1]).unwrap();
            f.add(&mut d, &canvas_quad()[1..]).unwrap();
            // 全覆いのタイルは覆いの記録を持たず、タイルの数は画布のタイルの数（5×4）
            assert_eq!(f.covered_tile_count(&d), Some(20));
            f.commit(&mut d).unwrap();
        }
        for m in material() {
            let expected = blend::blend(Rgba8::TRANSPARENT, m.value, 0.5, BlendMode::Normal);
            let b = bytes(&d, l, m.channel);
            assert_eq!(b.len(), 37 * 29 * 4);
            assert!(b.chunks_exact(4).all(|p| p == expected.to_array()));
        }
        assert_eq!(d.undo_count(), 1);
        d.undo().unwrap();
        assert_eq!(d.allocated_bytes(), 0);
        assert!(!d.layer(l).unwrap().is_channel_enabled(Channel::Normal));
    }
}
#[test]
fn a_full_cover_of_a_ragged_canvas_keeps_no_sample_memory() {
    let mut d = ragged();
    let l = d.add_layer("塗り").unwrap();
    d.clear_history().unwrap();
    let mut f = d
        .begin_material_triangle_fill(l, &material(), 0.5, false)
        .unwrap();
    f.add(&mut d, &canvas_quad()).unwrap();
    // 端が欠けたタイルも画布の中が全部覆われていれば「全覆い」で、サンプルの記録（新しいタイルごとに 16 + 2 × タイルの画素数 バイト）を持たない。
    // 巻き戻しは 6 チャンネル × 20 タイルの、元が空のタイルの記録（64 バイト）だけ
    assert_eq!(d.active_stroke_stats().unwrap().rollback_bytes, 6 * 20 * 64);
    assert_eq!(d.active_stroke_stats().unwrap().tiles, 6 * 20);
    f.cancel(&mut d);
}
/// 層の状態の写し: 各チャンネルの（有効か・面の画素）・マスクの画素・確保量・Undo の段・進行中のストロークの有無。
type Snapshot = (
    Vec<(Channel, bool, Option<Vec<u8>>)>,
    Option<Vec<u8>>,
    u64,
    usize,
    bool,
);
fn snapshot(d: &Document, l: LayerId) -> Snapshot {
    let layer = d.layer(l).unwrap();
    (
        Channel::ALL
            .iter()
            .map(|c| {
                (
                    *c,
                    layer.is_channel_enabled(*c),
                    layer.surface(*c).map(|s| s.to_canvas_bytes()),
                )
            })
            .collect(),
        layer.mask().map(|m| m.surface().to_canvas_bytes()),
        d.allocated_bytes(),
        d.undo_count(),
        d.has_active_stroke(),
    )
}
/// 描いたことのある無効のチャンネル（面が残る）と、一度も描いていない無効のチャンネルが混ざった層。
fn mixed_layer() -> (Document, LayerId) {
    let mut d = Document::with_tile_size(40, 32, 8).unwrap();
    let l = d.add_layer("塗り").unwrap();
    for (c, x) in [(Channel::Color, 3), (Channel::Roughness, 21)] {
        d.set_channel_enabled(l, c, true).unwrap();
        d.set_channel_pixel(l, c, x, 5, Rgba8::new(10, 20, 30, 200))
            .unwrap();
    }
    // Roughness は面を残したまま無効に戻す。Metallic などは面を持たない
    d.set_channel_enabled(l, Channel::Roughness, false).unwrap();
    d.clear_history().unwrap();
    (d, l)
}
#[test]
fn escape_cancels_a_material_stroke_back_to_the_enabling_and_every_pixel() {
    let (mut d, l) = mixed_layer();
    let before = snapshot(&d, l);
    let of = |c: Channel| before.0.iter().find(|e| e.0 == c).unwrap();
    assert!(!of(Channel::Roughness).1 && of(Channel::Roughness).2.is_some());
    assert!(!of(Channel::Metallic).1 && of(Channel::Metallic).2.is_none());
    // 札を持たない入口（フォーカス喪失・Escape・札を落としたとき）は、何も無ければ false
    assert!(!d.cancel_active_stroke());
    let mut s = d
        .begin_material_stroke(l, &material(), &BrushSettings::default())
        .unwrap();
    feed(&mut s, &mut d);
    assert!(d.has_active_stroke());
    // 途中: 全チャンネルが有効になり、画素が変わっている
    assert!(d.layer(l).unwrap().is_channel_enabled(Channel::Metallic));
    assert_ne!(snapshot(&d, l).0, before.0);
    let revision = d.revision();
    assert!(d.cancel_active_stroke());
    assert!(d.revision() > revision);
    assert_eq!(snapshot(&d, l), before);
    assert!(!d.cancel_active_stroke());
    // 札は無効になり、ストロークを取り残さない（次のストロークを始められ、確定すれば 1 段になる）
    assert_eq!(
        s.add_point(&mut d, 1.0, 1.0, 1.0, DVec2::ZERO),
        Err(CoreError::NoActiveStroke)
    );
    let s = d
        .begin_material_stroke(l, &material(), &BrushSettings::default())
        .unwrap();
    let mut s = s;
    feed(&mut s, &mut d);
    assert!(d.end_stroke(s).unwrap().changed);
    assert_eq!(d.undo_count(), 1);
    d.undo().unwrap();
    assert_eq!(snapshot(&d, l), before);
}
#[test]
fn escape_cancels_a_triangle_fill_back_to_the_enabling_and_every_pixel() {
    let (mut d, l) = mixed_layer();
    let before = snapshot(&d, l);
    let mut f = d
        .begin_material_triangle_fill(l, &material(), 0.5, false)
        .unwrap();
    assert!(f.add(&mut d, &quad()[..1]).unwrap());
    assert!(d.has_active_stroke());
    assert!(d.layer(l).unwrap().is_channel_enabled(Channel::Metallic));
    assert_ne!(snapshot(&d, l).0, before.0);
    assert!(d.cancel_active_stroke());
    assert_eq!(snapshot(&d, l), before);
    assert!(!d.cancel_active_stroke());
    // 札は取り残されない: 足すと断られ、確定しても何も起きない
    assert_eq!(f.triangles_added(&d), None);
    assert_eq!(f.add(&mut d, &quad()), Err(CoreError::NoActiveStroke));
    // 次の塗りを始められる
    let mut f = d
        .begin_material_triangle_fill(l, &material(), 0.5, false)
        .unwrap();
    f.add(&mut d, &quad()).unwrap();
    assert!(f.commit(&mut d).unwrap().changed);
    assert_eq!(d.undo_count(), 1);
    d.undo().unwrap();
    assert_eq!(snapshot(&d, l), before);
}
#[test]
fn escape_cancels_a_mask_triangle_fill_back_to_the_mask_pixels() {
    let mut d = Document::with_tile_size(8, 8, 4).unwrap();
    let l = d.add_layer("塗り").unwrap();
    d.add_layer_mask(l).unwrap();
    d.set_mask_pixel(l, 1, 1, 77).unwrap();
    d.clear_history().unwrap();
    let before = snapshot(&d, l);
    for reveal in [false, true] {
        let mut f = d.begin_mask_triangle_fill(l, 0.5, reveal).unwrap();
        assert!(f.add(&mut d, &quad()).unwrap());
        assert_ne!(snapshot(&d, l).1, before.1);
        assert!(d.cancel_active_stroke());
        assert_eq!(snapshot(&d, l), before);
        assert!(!d.cancel_active_stroke());
    }
}
/// 三角形の塗りの結果（画素・巻き戻しの量・予算で断る所）は、並列の度合いによらない。タイルが多く（まとまりが何度も回る）、
/// 選択範囲があり、画素の予算と巻き戻しの予算のどちらで止めても。
#[test]
fn triangle_fill_does_not_depend_on_the_thread_count() {
    let run = |threads: usize, rollback: Option<u64>, source: Option<u64>| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let mut d = Document::with_tile_size(301, 203, 16).unwrap();
                let l = d.add_layer("塗り").unwrap();
                for y in (0..203).step_by(7) {
                    for x in (0..301).step_by(5) {
                        d.set_channel_pixel(l, Channel::Color, x, y, Rgba8::new(9, 8, 7, 200))
                            .unwrap();
                    }
                }
                d.clear_history().unwrap();
                d.set_selection(Some(
                    SelectionMask::ellipse(&d, 150.0, 100.0, 120.0, 70.0).unwrap(),
                ))
                .unwrap();
                if let Some(b) = rollback {
                    d.set_stroke_budget_bytes(b).unwrap();
                }
                if let Some(extra) = source {
                    // 今ある分に足せる余裕（画素の予算は今の確保量より小さくできない）
                    d.set_source_budget_bytes(d.allocated_bytes() + extra)
                        .unwrap();
                }
                let mut f = d
                    .begin_material_triangle_fill(l, &material(), 0.6, false)
                    .unwrap();
                let tris = [
                    tri([(-5.0, -5.0), (310.0, 20.0), (290.0, 210.0)]),
                    tri([(-5.0, -5.0), (290.0, 210.0), (10.0, 215.0)]),
                    tri([(40.5, 8.5), (270.25, 100.0), (60.0, 190.5)]),
                ];
                let r = f
                    .add(&mut d, &tris[..1])
                    .and_then(|_| f.add(&mut d, &tris[1..]));
                let stats = d.active_stroke_stats().map(|s| (s.tiles, s.rollback_bytes));
                if r.is_ok() {
                    f.commit(&mut d).unwrap();
                }
                (r, stats, snapshot(&d, l))
            })
    };
    for (rollback, source) in [
        (None, None),
        (Some(150_000), None),
        (Some(400_000), None),
        (None, Some(60_000)),
    ] {
        let reference = run(1, rollback, source);
        for threads in [2, 3, 8] {
            assert_eq!(
                run(threads, rollback, source),
                reference,
                "{threads} {rollback:?} {source:?}"
            );
        }
    }
    // どの予算でも断られる設定が、期待する種類で断る（試験が成功だけを比べていないことの確かめ）
    assert_eq!(
        run(1, Some(150_000), None).0,
        Err(CoreError::StrokeBudgetExceeded)
    );
    assert_eq!(
        run(1, None, Some(60_000)).0,
        Err(CoreError::SourceBudgetExceeded)
    );
    assert!(run(1, None, None).0.is_ok());
}

// ───────── 効果のブラシ × 複数チャンネル（C# の BrushEffectTests） ─────────

const EW: usize = 19;
const EH: usize = 13;
const BLUR: BrushEffect = BrushEffect::Blur { radius: 2 };
const SMUDGE: BrushEffect = BrushEffect::Smudge { strength: 0.6 };
const CLONE: BrushEffect = BrushEffect::Clone {
    offset: DVec2::new(-2.25, 0.5),
};
const EFFECTS: [BrushEffect; 3] = [BLUR, SMUDGE, CLONE];
/// C# の BrushEffectTests.Settings: 半径 3・硬さ 0.6・間隔 0.2・不透明度 0.7・流量 0.6・筆圧は不透明度と流量だけ。
fn effect_brush(effect: BrushEffect) -> Brush {
    Brush {
        effect,
        ..Brush::from(BrushSettings {
            radius: 3.0,
            hardness: 0.6,
            spacing: 0.2,
            opacity: 0.7,
            flow: 0.6,
            pressure_size: false,
            pressure_opacity: true,
            pressure_flow: true,
            ..BrushSettings::default()
        })
    }
}
fn at(x: f64, y: f64, time: f64) -> BrushSample {
    BrushSample::new(x, y, 1.0, time, DVec2::ZERO).unwrap()
}
/// C# の試験のストローク: (5, 5) から (12, 7) への 2 点。
fn effect_stroke(s: &mut Stroke, d: &mut Document) -> Result<(), CoreError> {
    s.add_sample(d, at(5.0, 5.0, 0.0))?;
    s.add_sample(d, at(12.0, 7.0, 1.0))
}
/// C# の BrushEffectTests.Make: 19×13 の層の全チャンネルに、チャンネルごとに違う決まった模様（アルファ 0・120・255 が混ざる）。
fn effect_layer(tile: u32) -> (Document, LayerId) {
    let mut d = Document::with_tile_size(EW as u32, EH as u32, tile).unwrap();
    let l = d.add_layer("塗り").unwrap();
    for (i, c) in Channel::ALL.iter().enumerate() {
        d.set_channel_enabled(l, *c, true).unwrap();
        for y in 0..EH {
            for x in 0..EW {
                let a = if (x + y) % 5 == 0 {
                    0
                } else if (x + y) % 3 == 0 {
                    120
                } else {
                    255
                };
                let p = Rgba8::new(
                    ((x * 31 + i * 7) % 256) as u8,
                    ((y * 47 + i) % 256) as u8,
                    ((x * 19 + y * 7) % 256) as u8,
                    a,
                );
                d.set_channel_pixel(l, *c, x as u32, y as u32, p).unwrap();
            }
        }
    }
    d.clear_history().unwrap();
    (d, l)
}
fn red_material(channels: &[Channel]) -> Vec<ChannelPaint> {
    channels
        .iter()
        .map(|c| ChannelPaint::new(*c, Rgba8::new(255, 0, 0, 255)))
        .collect()
}

/// C# の MaterialChannelsComputeFromTheirOwnPixelsAndCancelTogether: 効果のブラシ（ぼかし・指先・クローン）の複数チャンネルのストロークは、
/// 各チャンネルが自分の画素から計算する（1 チャンネルずつ別の文書で描いた結果と同じバイトで、チャンネルどうしは違う）。
/// Undo 1 回で全チャンネルが戻り、Redo で同じ結果に戻り、取消は全チャンネルを元へ戻す。
#[test]
fn effect_brushes_compute_each_channel_from_its_own_pixels_and_undo_and_cancel_together() {
    for effect in EFFECTS {
        let (mut d, l) = effect_layer(16);
        let originals: Vec<_> = Channel::ALL.iter().map(|c| bytes(&d, l, *c)).collect();
        let b = effect_brush(effect);
        let mut s = d
            .begin_material_brush_stroke(l, &red_material(&Channel::ALL), &b)
            .unwrap();
        effect_stroke(&mut s, &mut d).unwrap();
        assert!(d.end_stroke(s).unwrap().changed, "{effect:?}");
        let outputs: Vec<_> = Channel::ALL.iter().map(|c| bytes(&d, l, *c)).collect();
        for (i, c) in Channel::ALL.iter().enumerate() {
            let (mut one, ol) = effect_layer(16);
            let mut s = one.begin_brush_stroke_in(ol, *c, &b).unwrap();
            effect_stroke(&mut s, &mut one).unwrap();
            one.end_stroke(s).unwrap();
            assert_eq!(outputs[i], bytes(&one, ol, *c), "{effect:?} {c:?}");
            assert_ne!(outputs[i], originals[i], "{effect:?} {c:?} は変わる");
            for j in 0..i {
                assert_ne!(outputs[i], outputs[j], "{effect:?} {c:?} は自分の画素から");
            }
        }
        assert_eq!(d.undo_count(), 1, "{effect:?}");
        assert!(d.undo().unwrap());
        for (i, c) in Channel::ALL.iter().enumerate() {
            assert_eq!(bytes(&d, l, *c), originals[i], "{effect:?} {c:?}");
        }
        assert!(d.redo().unwrap());
        for (i, c) in Channel::ALL.iter().enumerate() {
            assert_eq!(bytes(&d, l, *c), outputs[i], "{effect:?} {c:?}");
        }
        let saved = snapshot(&d, l);
        let mut s = d
            .begin_material_brush_stroke(l, &red_material(&Channel::ALL), &b)
            .unwrap();
        effect_stroke(&mut s, &mut d).unwrap();
        assert_ne!(snapshot(&d, l), saved, "{effect:?} 取消の前は描けている");
        d.cancel_stroke(s);
        assert_eq!(snapshot(&d, l), saved, "{effect:?}");
    }
}

/// 効果のブラシのチャンネル数の違いで 1 チャンネルの結果が変わらない（2 チャンネルでも 6 チャンネルでも同じ）。
#[test]
fn an_effect_channels_result_does_not_depend_on_which_other_channels_are_in_the_stroke() {
    for effect in EFFECTS {
        let b = effect_brush(effect);
        let run = |channels: &[Channel]| {
            let (mut d, l) = effect_layer(8);
            let mut s = d
                .begin_material_brush_stroke(l, &red_material(channels), &b)
                .unwrap();
            effect_stroke(&mut s, &mut d).unwrap();
            d.end_stroke(s).unwrap();
            bytes(&d, l, Channel::Roughness)
        };
        let six = run(&Channel::ALL);
        assert_eq!(six, run(&[Channel::Roughness]), "{effect:?}");
        assert_eq!(
            six,
            run(&[Channel::Height, Channel::Roughness]),
            "{effect:?}"
        );
    }
}

/// 無効のチャンネル（描いた面は残る）を含む効果のブラシのストロークの前の状態: Color は有効、Height は無効だが画素が残る。
/// どちらも一様で不透明な 1 タイル（4 バイト）なので、書くとタイルが広がって元画素の予算を使う。
fn growth_layer() -> (Document, LayerId) {
    let mut d = Document::with_tile_size(32, 32, 8).unwrap();
    let l = d.add_layer("塗り").unwrap();
    d.set_channel_enabled(l, Channel::Height, true).unwrap();
    for c in [Channel::Color, Channel::Height] {
        d.import_tile(l, c, TileCoord::new(0, 0), &[255u8; 8 * 8 * 4])
            .unwrap();
    }
    d.set_channel_enabled(l, Channel::Height, false).unwrap();
    d.clear_history().unwrap();
    assert_eq!(d.allocated_bytes(), 8);
    (d, l)
}
fn growth_brush(effect: BrushEffect) -> Brush {
    let effect = match effect {
        BrushEffect::Clone { .. } => BrushEffect::Clone {
            offset: DVec2::new(-4.0, 0.0),
        },
        e => e,
    };
    Brush {
        base: BrushSettings {
            radius: 6.0,
            ..effect_brush(effect).base
        },
        ..effect_brush(effect)
    }
}

/// growth_layer の効果のストロークの 3 つの点（効果によって、1 つ目では何も広げないものもある）。
fn growth_sample(i: usize) -> BrushSample {
    let (x, y, time) = [(5.0, 5.0, 0.0), (12.0, 7.0, 1.0), (9.0, 5.0, 2.0)][i];
    at(x, y, time)
}
/// growth_layer へ `channels` だけのストロークを元画素の予算の余裕なしに与えて、何番目の点で初めて元画素が広がるかと、そこまでに
/// 広がったバイト数（ほかのチャンネルの面は動かさないので、Color だけなら Color の広がり）。
fn first_growth(effect: BrushEffect, channels: &[Channel]) -> (usize, u64) {
    let (mut d, l) = growth_layer();
    let before = d.allocated_bytes();
    let mut s = d
        .begin_material_brush_stroke(l, &red_material(channels), &growth_brush(effect))
        .unwrap();
    for i in 0..3 {
        s.add_sample(&mut d, growth_sample(i)).unwrap();
        let grown = d.allocated_bytes() - before;
        if grown > 0 {
            d.cancel_stroke(s);
            return (i, grown);
        }
    }
    panic!("{effect:?} {channels:?} は元画素を広げない");
}

/// C# の ASourceGrowthRefusalRollsBackPixelsAndChannelsEnabledByTheStroke: 元画素の予算が 1 タイルの広がりも許さないとき、複数
/// チャンネルの効果のストロークは断られ、ストロークが有効にしたチャンネルも書いた画素も元へ戻り、Undo の段を残さない。
/// もう 1 つの余裕は、Color が最初に広がる分（Color だけのストロークを同じ余裕で走らせると、その点まで通る）。Color の分を先に書き
/// 終えてから、Height が余裕の無さで断る。そのときも Color まで戻る。どちらの余裕でも、断られるのは Color が初めて広がる点。
#[test]
fn a_source_growth_refusal_rolls_back_pixels_and_the_channels_the_stroke_enabled() {
    for effect in EFFECTS {
        let (point, color_room) = first_growth(effect, &[Channel::Color]);
        let (both_point, both_grown) = first_growth(effect, &[Channel::Color, Channel::Height]);
        assert_eq!(both_point, point, "{effect:?}");
        assert!(
            both_grown > color_room,
            "{effect:?} Height も広がる（Color の分だけでは足りない）"
        );
        // Color の余裕で Color だけのストロークは、広がる点まで通る（Color の分は断られない）
        {
            let (mut d, l) = growth_layer();
            d.set_source_budget_bytes(d.allocated_bytes() + color_room)
                .unwrap();
            let mut s = d
                .begin_material_brush_stroke(
                    l,
                    &red_material(&[Channel::Color]),
                    &growth_brush(effect),
                )
                .unwrap();
            for i in 0..=point {
                s.add_sample(&mut d, growth_sample(i)).unwrap();
            }
            d.cancel_stroke(s);
        }
        for room in [0, color_room] {
            let (mut d, l) = growth_layer();
            let saved = snapshot(&d, l);
            assert!(!saved.0[3].1, "Height は無効");
            d.set_source_budget_bytes(d.allocated_bytes() + room)
                .unwrap();
            let mut s = d
                .begin_material_brush_stroke(
                    l,
                    &red_material(&[Channel::Color, Channel::Height]),
                    &growth_brush(effect),
                )
                .unwrap();
            assert!(
                d.layer(l).unwrap().is_channel_enabled(Channel::Height),
                "ストロークが Height を有効にする"
            );
            let mut refused = None;
            for i in 0..3 {
                if let Err(e) = s.add_sample(&mut d, growth_sample(i)) {
                    refused = Some((i, e));
                    break;
                }
            }
            assert_eq!(
                refused,
                Some((point, CoreError::SourceBudgetExceeded)),
                "{effect:?} {room}"
            );
            assert!(!d.has_active_stroke());
            assert_eq!(snapshot(&d, l), saved, "{effect:?} {room}");
            // 戻したあとは、予算が足りれば同じ入力が通る
            d.set_source_budget_bytes(1 << 20).unwrap();
            let mut s = d
                .begin_material_brush_stroke(
                    l,
                    &red_material(&[Channel::Color, Channel::Height]),
                    &growth_brush(effect),
                )
                .unwrap();
            for i in 0..3 {
                s.add_sample(&mut d, growth_sample(i)).unwrap();
            }
            assert!(d.end_stroke(s).unwrap().changed, "{effect:?} {room}");
            assert_eq!(d.undo_count(), 1);
        }
    }
}

/// C# の BudgetsAndInvalidInputsCancelAllChannelsWithoutAnUndo（複数チャンネル）: ストロークの予算の拒否も、不正な入力
/// （覆いが範囲外のダブ）も、無効のチャンネルを有効にしたストロークごと取り消して Undo の段を残さない。
#[test]
fn effect_budgets_and_invalid_inputs_cancel_every_channel_without_an_undo() {
    for effect in EFFECTS {
        let (mut d, l) = effect_layer(16);
        d.set_channel_enabled(l, Channel::Height, false).unwrap();
        d.clear_history().unwrap();
        let saved = snapshot(&d, l);
        let channels = red_material(&[Channel::Color, Channel::Height]);
        d.set_stroke_budget_bytes(10).unwrap();
        let mut s = d
            .begin_material_brush_stroke(l, &channels, &effect_brush(effect))
            .unwrap();
        assert!(d.layer(l).unwrap().is_channel_enabled(Channel::Height));
        let r = effect_stroke(&mut s, &mut d);
        assert_eq!(r, Err(CoreError::StrokeBudgetExceeded), "{effect:?}");
        assert!(!d.has_active_stroke());
        assert_eq!(snapshot(&d, l), saved, "{effect:?}");
        assert_eq!(d.undo_count(), 0);
        d.set_stroke_budget_bytes(4 << 20).unwrap();
        let mut s = d
            .begin_material_brush_stroke(l, &channels, &effect_brush(effect))
            .unwrap();
        effect_stroke(&mut s, &mut d).unwrap();
        let bad = [BrushPixel {
            x: 5,
            y: 5,
            coverage: 2.0,
        }];
        assert!(s
            .apply_dab(&mut d, &bad, DVec2::new(5.0, 5.0), 1.0)
            .is_err());
        assert!(!d.has_active_stroke());
        assert_eq!(snapshot(&d, l), saved, "{effect:?}");
        assert_eq!(d.undo_count(), 0);
        assert!(!d.layer(l).unwrap().is_channel_enabled(Channel::Height));
        // 時刻が戻る入力でも同じ（取消して返す）
        let mut s = d
            .begin_material_brush_stroke(l, &channels, &effect_brush(effect))
            .unwrap();
        s.add_sample(&mut d, at(5.0, 5.0, 1.0)).unwrap();
        assert!(s.add_sample(&mut d, at(6.0, 5.0, 0.5)).is_err());
        assert!(!d.has_active_stroke());
        assert_eq!(snapshot(&d, l), saved, "{effect:?}");
    }
}
