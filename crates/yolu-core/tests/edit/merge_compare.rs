//! 結合の報告（前と後の比べ）が、全画素を 1 つずつ比べた参照と同じ数で、スレッドの数（比べの束の分け方）によらないことの試験。
//! 比べはタイルの束ごとにワーカーへ分けて合成と比べを続けて行うので、束の境目をまたぐ大きさ（16² のタイル 150 枚。束はスレッドあたり 32 枚）にする。

use yolu_core::{BlendMode, Channel, CoreError, Document, LayerId, LayerMergeReport, Rgba8};

const W: u32 = 240;
const H: u32 = 160;

fn rng(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = *seed;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// 全面を塗ったレイヤーを 4 枚（下から: 不透明・半透明の乗算・半透明のスクリーン・半透明の通常）。上の 3 枚は合成のモードと不透明度で
/// 結合の丸めの差が出る。真ん中の 2 枚はグループにまとめる。返すのはレイヤー（下から）とグループ。
fn layered(seed: u64) -> (Document, Vec<LayerId>, LayerId) {
    let mut s = seed;
    let mut d = Document::with_tile_size(W, H, 16).unwrap();
    let modes = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Normal,
    ];
    let mut ids = Vec::new();
    for (i, mode) in modes.into_iter().enumerate() {
        let id = d.add_layer(&format!("l{i}")).unwrap();
        for y in 0..H {
            for x in 0..W {
                let r = rng(&mut s);
                let a = if i == 0 {
                    255
                } else {
                    1 + (r >> 24) as u8 % 255
                };
                d.set_pixel(
                    id,
                    x,
                    y,
                    Rgba8::new(r as u8, (r >> 8) as u8, (r >> 16) as u8, a),
                )
                .unwrap();
            }
        }
        d.set_layer_blend_mode(id, mode).unwrap();
        if i > 0 {
            d.set_layer_opacity(id, 0.3 + 0.2 * i as f64, false)
                .unwrap();
        }
        ids.push(id);
    }
    let group = d.group_layers(&ids[1..3], "g").unwrap();
    d.clear_history().unwrap();
    (d, ids, group)
}

/// 全画素を 1 つずつ比べた参照の報告の数（結合と同じ規則: 両方とも透明な画素は数えない・見える差はアルファを掛けた RGB とアルファ）。
fn reference(after: &[u8], before: &[u8]) -> (u64, u64, u8, u8) {
    let (mut compared, mut changed, mut largest, mut visible) = (0, 0, 0u8, 0u8);
    for (a, b) in after
        .as_chunks::<4>()
        .0
        .iter()
        .zip(before.as_chunks::<4>().0)
    {
        compared += 1;
        if a[3] == 0 && b[3] == 0 {
            continue;
        }
        let delta = (0..4).map(|q| a[q].abs_diff(b[q])).max().unwrap();
        if delta == 0 {
            continue;
        }
        changed += 1;
        largest = largest.max(delta);
        let mut v = a[3].abs_diff(b[3]);
        for q in 0..3 {
            let e = ((a[q] as i32 * a[3] as i32 - b[q] as i32 * b[3] as i32).abs() + 127) / 255;
            v = v.max(e as u8);
        }
        visible = visible.max(v);
    }
    (compared, changed, largest, visible)
}

fn with_threads<T: Send>(threads: usize, run: impl FnOnce() -> T + Send) -> T {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap()
        .install(run)
}

fn merge(kind: &str, seed: u64) -> (LayerMergeReport, (u64, u64, u8, u8)) {
    let (mut d, ids, group) = layered(seed);
    let before = d.composite(d.bounds()).unwrap();
    let report = match kind {
        "down" => d.merge_down(ids[2], 255),
        "layers" => d.merge_layers(&[ids[0], ids[3]], 255),
        "group" => d.merge_group(group, 255),
        _ => d.merge_visible("merged", 255),
    }
    .unwrap();
    let after = d.composite(d.bounds()).unwrap();
    // 表示に寄与するレイヤーの結合は、結合前の合成でなく、結果のレイヤーの画素と比べる
    let before = if kind == "visible" {
        d.layer(report.result_id)
            .unwrap()
            .surface(Channel::Color)
            .unwrap()
            .canvas_bytes()
            .unwrap()
    } else {
        before
    };
    let expected = reference(&after, &before);
    (report, expected)
}

#[test]
fn merge_reports_match_a_pixel_by_pixel_comparison_with_any_thread_count() {
    let mut changed_somewhere = false;
    for kind in ["down", "layers", "group", "visible"] {
        for seed in 0..3 {
            let mut reports = Vec::new();
            for threads in [1, 4, 8] {
                let (report, expected) = with_threads(threads, || merge(kind, seed));
                let (compared, changed, largest, visible) = expected;
                assert_eq!(report.compared_pixels, compared, "{kind} {seed} {threads}");
                assert_eq!(report.changed_pixels, changed, "{kind} {seed} {threads}");
                assert_eq!(report.max_difference, largest, "{kind} {seed} {threads}");
                assert_eq!(
                    report.max_visible_difference, visible,
                    "{kind} {seed} {threads}"
                );
                let by_channel: Vec<(Channel, u64)> = report
                    .changed_by_channel
                    .iter()
                    .map(|(c, n)| (*c, *n))
                    .collect();
                if changed == 0 {
                    assert!(by_channel.is_empty(), "{kind} {seed} {threads}");
                } else {
                    assert_eq!(by_channel, [(Channel::Color, changed)], "{kind} {seed}");
                }
                changed_somewhere |= changed > 0;
                reports.push(format!("{report:?}").replace(&format!("{:?}", report.result_id), ""));
            }
            assert!(
                reports.windows(2).all(|w| w[0] == w[1]),
                "{kind} {seed}: {reports:?}"
            );
        }
    }
    assert!(changed_somewhere, "丸めの差が出る結合を含む");
}

/// 許容差を超える差は、全部のタイルを比べたうえで断り、文書は変えない（比べを束に分けても同じ判定）。
#[test]
fn a_merge_beyond_the_tolerance_is_refused_with_the_same_report() {
    for threads in [1, 4] {
        with_threads(threads, || {
            let (mut d, ids, _) = layered(7);
            let allowed = d.merge_down(ids[2], 255).unwrap();
            assert!(allowed.max_visible_difference > 0);
            assert!(d.undo().unwrap());
            let before = d.composite(d.bounds()).unwrap();
            let revision = d.revision();
            match d.merge_down(ids[2], allowed.max_visible_difference - 1) {
                Err(CoreError::MergeAppearance(r)) => {
                    assert_eq!(r.compared_pixels, allowed.compared_pixels);
                    assert_eq!(r.changed_pixels, allowed.changed_pixels);
                    assert_eq!(r.max_visible_difference, allowed.max_visible_difference);
                }
                other => panic!("{other:?}"),
            }
            assert_eq!(d.revision(), revision);
            assert_eq!(d.composite(d.bounds()).unwrap(), before);
        });
    }
}
