//! スレッド数によらない結果（C# の CpuParallelismTests のうち、Core の編集の範囲）: 大きなダブのストローク（色・マスク）、
//! 領域の編集（塗りつぶし・グラデーション・マテリアルの塗り・マスクの塗り）が、スレッド数 1・2・3・既定のどれでも同じバイトで、
//! 取消・予算の拒否では画素・確保量・履歴・版が元のまま。合成そのものは document.rs（`compositing_does_not_depend_on_the_thread_count`）、
//! 1 本のストロークは同（`big_dabs_give_the_same_bytes_with_any_thread_count`）、ロック下の塗りと 1・4 スレッドの C# との一致は
//! material_golden.rs が見る。レイヤーの操作のうち、変形（任意の角度・拡大縮小・反転・90° 回転・選択範囲の持ち上げ・マスクと全チャンネル）と
//! 結合のスレッド数は末尾の試験が見る（変形の取消・予算の拒否は元のバイトのまま）。サイズ変更は docops.rs・resize.rs に任せる。
//! 結果は画素のバイトを SHA-256 に、確保量・タイルの数・変更の記録・履歴のバイト数・ストロークの統計を添えて比べる。
//! 束に入れず直下の 1 本: ワーカーの閾値（`yolu_core::brush::set_parallel_dab_pixels`。プロセスで 1 つ）を最初の試験で 1 にして戻さない。束のほかの試験のダブの経路を変える。

#![allow(clippy::chunks_exact_to_as_chunks)]

use sha2::{Digest, Sha256};
use yolu_core::glam::DVec2;
use yolu_core::material::{ChannelPaint, GradientSettings};
use yolu_core::selection::DEFAULT_WORKING_BUDGET_BYTES as BUDGET;
use yolu_core::{
    builtin_tip, Brush, BrushEffect, BrushSample, BrushSettings, Channel, CoreError, Document,
    DualBrush, DualBrushMode, LayerId, PaperTexture, Rgba8, SelectionMask, Surface, TileCoord,
};

/// 0 は rayon の既定の数（全プロセッサ）。
const DEGREES: [usize; 4] = [1, 2, 3, 0];

/// この試験は、直列で描いても、ワーカーで描いても画素が同じことを確かめる。既定のしきい値は画素ごとの時間の見積もりで決まり、速いブラシの
/// 大きなダブは直列で描くので、小さめの箱もワーカーで描かせる。どの試験にも効くよう、一度だけ決めて戻さない（経路で画素は変わらない）。
fn force_workers() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        yolu_core::brush::set_parallel_dab_pixels(1);
    });
}

fn with_degree<T: Send>(degree: usize, run: impl FnOnce() -> T + Send) -> T {
    force_workers();
    if degree == 0 {
        return run();
    }
    rayon::ThreadPoolBuilder::new()
        .num_threads(degree)
        .build()
        .unwrap()
        .install(run)
}
/// どのスレッド数でも、1 スレッドの結果と同じ。
fn same_for_every_degree<T: Send + PartialEq + std::fmt::Debug>(
    what: &str,
    run: impl Fn() -> T + Sync,
) -> T {
    let single = with_degree(1, &run);
    for degree in &DEGREES[1..] {
        assert_eq!(
            with_degree(*degree, &run),
            single,
            "{what}: スレッド {} で変わった",
            if *degree == 0 {
                "既定".to_string()
            } else {
                degree.to_string()
            }
        );
    }
    single
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
/// 面の画素と、疎な構造（中身のあるタイルの数・確保量。一様なタイルと全画素のタイルの違いが出る）。
fn surface_hash(s: &Surface) -> String {
    format!(
        "{}/{}/{}",
        s.tile_count(),
        s.allocated_bytes(),
        hash(&s.to_canvas_bytes())
    )
}
fn color_hash(d: &Document, l: LayerId) -> String {
    surface_hash(d.layer(l).unwrap().surface(Channel::Color).unwrap())
}
fn mask_hash(d: &Document, l: LayerId) -> String {
    surface_hash(d.layer(l).unwrap().mask().unwrap().surface())
}

struct Rnd(u64);
impl Rnd {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn double(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn fill(&mut self, bytes: &mut [u8]) {
        for b in bytes {
            *b = (self.next() >> 24) as u8;
        }
    }
}
/// share の割合のタイルへ乱数の画素を置く（透明でも RGB を持つ画素を含める。キャンバスの外は 0）。
#[allow(clippy::too_many_arguments)]
fn import(
    d: &mut Document,
    l: LayerId,
    channel: Channel,
    tile: u32,
    rnd: &mut Rnd,
    share: f64,
    opaque: bool,
) {
    let (w, h) = (d.width(), d.height());
    let mut b = vec![0u8; (tile * tile * 4) as usize];
    for ty in 0..h.div_ceil(tile) {
        for tx in 0..w.div_ceil(tile) {
            if rnd.double() >= share {
                continue;
            }
            rnd.fill(&mut b);
            for y in 0..tile {
                for x in 0..tile {
                    let o = ((y * tile + x) * 4) as usize;
                    if tx * tile + x >= w || ty * tile + y >= h {
                        b[o..o + 4].fill(0);
                    } else if opaque {
                        b[o + 3] = 255;
                    } else if b[o + 3] < 60 {
                        b[o + 3] = 0;
                    }
                }
            }
            d.import_tile(l, channel, TileCoord::new(tx, ty), &b)
                .unwrap();
        }
    }
}
/// 600×360・タイル 64。不透明な背景と、画素が既にあるレイヤー（一部のタイルは一様、一部は全画素）。selection ならぼかした楕円の選択範囲。
fn canvas(selection: bool) -> (Document, LayerId) {
    let mut d = Document::with_tile_size(600, 360, 64).unwrap();
    let mut rnd = Rnd(0x9E37_79B9_7F4A_7C15);
    let bg = d.add_layer("bg").unwrap();
    import(&mut d, bg, Channel::Color, 64, &mut rnd, 1.0, true);
    let l = d.add_layer("paint").unwrap();
    import(&mut d, l, Channel::Color, 64, &mut rnd, 0.4, false);
    d.import_tile(
        l,
        Channel::Color,
        TileCoord::new(3, 2),
        &[90u8; 64 * 64 * 4],
    )
    .unwrap();
    if selection {
        let mask = SelectionMask::ellipse(&d, 300.0, 180.0, 250.0, 150.0)
            .unwrap()
            .feather(12.0, false, BUDGET)
            .unwrap();
        d.set_selection(Some(mask)).unwrap();
    }
    d.clear_history().unwrap();
    (d, l)
}
/// C# の Path: 正弦に沿う長さ length の線を n 区間（筆圧と傾きも動く）。
fn path(x0: f64, y0: f64, length: f64, n: usize) -> Vec<BrushSample> {
    (0..=n)
        .map(|i| {
            let t = i as f64 / n as f64;
            BrushSample::new(
                x0 + length * t,
                y0 + 40.0 * (t * 7.0).sin(),
                0.4 + 0.6 * (t * 4.0).sin().abs(),
                i as f64 * 0.01,
                DVec2::new(0.3 * (t * 3.0).sin(), 0.2),
            )
            .unwrap()
        })
        .collect()
}
fn big_brushes() -> Vec<(&'static str, Brush)> {
    let ink = Rgba8::new(200, 60, 30, 255);
    let base = |radius, hardness, spacing| BrushSettings {
        radius,
        hardness,
        spacing,
        color: ink,
        ..BrushSettings::default()
    };
    let mut out = vec![
        ("round", Brush::from(base(70.0, 0.6, 0.1))),
        (
            "soft",
            Brush::from(BrushSettings {
                flow: 0.3,
                opacity: 0.8,
                pressure_flow: true,
                ..base(90.0, 0.0, 0.05)
            }),
        ),
    ];
    let mut tip = Brush::from(base(75.0, 0.8, 0.1));
    tip.tip.image = builtin_tip("charcoal");
    tip.tip.angle = 25.0;
    tip.tip.follow_direction = true;
    tip.tip.roundness = 0.7;
    out.push(("tip", tip));
    let mut texture = Brush::from(base(70.0, 0.8, 0.1));
    let mut paper = PaperTexture::new(builtin_tip("grain").unwrap(), 0.8);
    paper.scale = 2.0;
    texture.texture = Some(paper);
    out.push(("texture", texture));
    let mut dynamics = Brush::from(base(70.0, 0.8, 0.1));
    dynamics.seed = 3;
    dynamics.jitter.size = 0.3;
    dynamics.jitter.scatter = 0.2;
    dynamics.jitter.count = 2;
    dynamics.color.hue = 0.2;
    dynamics.color.per_tip = true;
    dynamics.assist.curve = true;
    dynamics.assist.taper_in = 30.0;
    dynamics.assist.taper_out = 40.0;
    dynamics.dual = Some(DualBrush {
        radius: 12.0,
        spacing: 0.3,
        hardness: 0.4,
        scatter: 0.5,
        count: 2,
        mode: DualBrushMode::Overlay,
        ..DualBrush::default()
    });
    out.push(("dynamics", dynamics));
    out.push((
        "erase",
        Brush::from(BrushSettings {
            erase: true,
            opacity: 0.7,
            ..base(80.0, 0.5, 0.1)
        }),
    ));
    out
}

/// C# の LargeDabsGiveTheSameStrokeWithAnyNumberOfThreads: 大きなダブ（丸・柔らかい・筆先の画像・質感・ゆらぎとデュアルと入り抜き・
/// 消しゴム）の Color へのストロークが、選択範囲の有無にかかわらず、どのスレッド数でも同じバイト。統計・変更の記録・取消・やり直しも同じ。
#[test]
fn large_dabs_of_every_kind_give_the_same_stroke_with_any_number_of_threads() {
    for (name, brush) in big_brushes() {
        for selection in [false, true] {
            let run = || {
                let (mut d, l) = canvas(selection);
                let before = color_hash(&d, l);
                let serial = d.change_serial();
                let mut s = d.begin_brush_stroke(l, &brush).unwrap();
                for p in path(40.0, 170.0, 520.0, 60) {
                    s.add_sample(&mut d, p).unwrap();
                }
                let stats = d.active_stroke_stats().unwrap();
                let log = (stats.stamps, stats.tiles, stats.rollback_bytes);
                assert!(d.end_stroke(s).unwrap().changed, "{name}");
                let mut changed = d.changed_tiles(Channel::Color, serial).unwrap();
                changed.sort();
                changed.dedup();
                let after = color_hash(&d, l);
                assert_ne!(after, before, "{name} は描けている");
                assert!(d.undo().unwrap());
                let undone = color_hash(&d, l);
                assert_eq!(undone, before, "{name} の Undo は元のバイト");
                assert!(d.redo().unwrap());
                assert_eq!(color_hash(&d, l), after, "{name} の Redo");
                (log, changed, after, undone, d.history_bytes())
            };
            same_for_every_degree(&format!("{name} {selection}"), run);
        }
    }
}

/// C# の ALargeDabStrokeOnAMaskIsTheSameWithAnyNumberOfThreads: 大きなダブ（半径 85）のマスクへのストロークが、どのスレッド数でも
/// 同じマスクのバイト・合成・統計・変更の記録で、取消・やり直しも同じ。1 スレッドではワーカーで描かず、複数ではワーカーで描く。
#[test]
fn a_large_dab_stroke_on_a_mask_is_the_same_with_any_number_of_threads() {
    let run = || {
        let (mut d, l) = canvas(false);
        d.add_layer_mask(l).unwrap();
        d.clear_history().unwrap();
        let before = mask_hash(&d, l);
        let serial = d.change_serial();
        let brush = BrushSettings {
            radius: 85.0,
            hardness: 0.3,
            spacing: 0.08,
            flow: 0.5,
            ..BrushSettings::default()
        };
        let mut s = d.begin_mask_stroke(l, &brush).unwrap();
        for p in path(30.0, 150.0, 540.0, 50) {
            s.add_sample(&mut d, p).unwrap();
        }
        let stats = d.active_stroke_stats().unwrap();
        assert!(d.end_stroke(s).unwrap().changed);
        let mut changed = d.changed_tiles(Channel::Color, serial).unwrap();
        changed.sort();
        changed.dedup();
        let after = mask_hash(&d, l);
        let composite = hash(&d.composite(d.bounds()).unwrap());
        assert_ne!(after, before);
        assert!(d.undo().unwrap());
        assert_eq!(mask_hash(&d, l), before);
        assert!(d.redo().unwrap());
        assert_eq!(mask_hash(&d, l), after);
        (
            (
                (stats.stamps, stats.tiles, stats.rollback_bytes),
                changed,
                after,
                composite,
                d.history_bytes(),
            ),
            stats.parallel_dabs,
        )
    };
    let one = with_degree(1, run);
    assert_eq!(one.1, 0, "1 スレッドはワーカーを使わない");
    for degree in [2, 3, 0] {
        let other = with_degree(degree, run);
        assert!(other.1 > 0, "スレッド {degree} はワーカーで描く");
        assert_eq!(other.0, one.0, "スレッド {degree}");
    }
}

/// C# の CancellingOrRefusingALargeDabStrokeLeavesTheExactPixels: 大きなダブのストロークを取り消しても、ストロークの予算が途中で
/// （タイルを何枚か持ったあとで）断っても、画素・確保量・履歴が厳密に元のまま。色とマスクの両方、どのスレッド数でも。
#[test]
fn cancelling_or_refusing_a_large_dab_stroke_leaves_the_exact_pixels() {
    for degree in DEGREES {
        for mask in [false, true] {
            let (mut d, l) = canvas(false);
            if mask {
                d.add_layer_mask(l).unwrap();
                d.clear_history().unwrap();
            }
            let surface_hash = |d: &Document| {
                if mask {
                    mask_hash(d, l)
                } else {
                    color_hash(d, l)
                }
            };
            let brush = BrushSettings {
                radius: 90.0,
                spacing: 0.1,
                hardness: 0.5,
                ..BrushSettings::default()
            };
            let begin = |d: &mut Document| {
                if mask {
                    d.begin_mask_stroke(l, &brush).unwrap()
                } else {
                    d.begin_stroke(l, &brush).unwrap()
                }
            };
            let before = surface_hash(&d);
            let allocated = d.allocated_bytes();
            let (during, full) = with_degree(degree, || {
                let mut s = begin(&mut d);
                for p in path(40.0, 170.0, 520.0, 60) {
                    s.add_sample(&mut d, p).unwrap();
                }
                let during = surface_hash(&d);
                let full = d.active_stroke_stats().unwrap().rollback_bytes;
                d.cancel_stroke(s);
                (during, full)
            });
            let what = format!("スレッド {degree} マスク {mask}");
            assert_ne!(during, before, "{what}: 取消の前は描けている");
            assert_eq!(surface_hash(&d), before, "{what}: 取消");
            assert_eq!((d.allocated_bytes(), d.undo_count()), (allocated, 0));
            assert!(!d.has_active_stroke());
            // 途中で止まる予算（全部を描くのに要る量の半分）: 何枚かのタイルを持ったあとで断られ、全部が元に戻る
            d.set_stroke_budget_bytes(full / 2).unwrap();
            let (refused, held) = with_degree(degree, || {
                let mut s = begin(&mut d);
                let mut held = 0;
                for p in path(40.0, 170.0, 520.0, 60) {
                    held = held.max(d.active_stroke_stats().map_or(0, |s| s.tiles));
                    if let Err(e) = s.add_sample(&mut d, p) {
                        return (Some(e), held);
                    }
                }
                (d.end_stroke(s).err(), held)
            });
            assert_eq!(refused, Some(CoreError::StrokeBudgetExceeded), "{what}");
            assert!(held > 0, "{what}: タイルを持ったあとで断られた");
            assert!(!d.has_active_stroke());
            assert_eq!(surface_hash(&d), before, "{what}: 拒否");
            assert_eq!((d.allocated_bytes(), d.undo_count()), (allocated, 0));
            // 予算を戻せば同じ入力が通る（拒否が壊れた状態を残していない）
            d.set_stroke_budget_bytes(64 << 20).unwrap();
            let mut s = begin(&mut d);
            for p in path(40.0, 170.0, 520.0, 60) {
                s.add_sample(&mut d, p).unwrap();
            }
            assert!(d.end_stroke(s).unwrap().changed, "{what}");
            assert_eq!(d.undo_count(), 1);
        }
    }
}

/// C# の RegionEditsAreTheSameWithAnyNumberOfThreads のうち塗りの分: 塗りつぶし（選択範囲なし・ぼかした選択範囲で消す）・放射
/// グラデーション・マスクの塗りが、どのスレッド数でも段ごとに同じバイトで、全部を Undo すると元のバイトへ戻る。
#[test]
fn region_edits_are_the_same_with_any_number_of_threads() {
    let stages = same_for_every_degree("region edits", || {
        let (mut d, l) = canvas(false);
        let original = color_hash(&d, l);
        let mut parts = vec![original.clone()];
        let mut stage = |d: &Document, extra: String| {
            let now = format!("{}|{extra}", color_hash(d, l));
            assert_ne!(parts.last().unwrap(), &now, "段が画素を変えている");
            parts.push(now);
        };
        d.fill(
            l,
            Channel::Color,
            Rgba8::new(10, 200, 30, 180),
            0.6,
            None,
            false,
        )
        .unwrap();
        stage(&d, String::new());
        let ellipse = SelectionMask::ellipse(&d, 250.0, 150.0, 200.0, 120.0)
            .unwrap()
            .feather(6.0, false, BUDGET)
            .unwrap();
        d.set_selection(Some(ellipse)).unwrap();
        d.fill(
            l,
            Channel::Color,
            Rgba8::new(200, 20, 30, 255),
            1.0,
            None,
            true,
        )
        .unwrap();
        stage(&d, String::new());
        let gradient = GradientSettings {
            shape: yolu_core::material::GradientShape::Radial,
            start: DVec2::new(200.0, 100.0),
            end: DVec2::new(500.0, 300.0),
            from: Rgba8::new(255, 0, 0, 255),
            to: Rgba8::new(0, 0, 255, 40),
            opacity: 0.9,
        };
        d.gradient(l, Channel::Color, &gradient, None, false)
            .unwrap();
        stage(&d, String::new());
        d.add_layer_mask(l).unwrap();
        assert!(d.fill_mask(l, 0.7, None, false).unwrap());
        stage(&d, mask_hash(&d, l));
        while d.undo().unwrap() {}
        assert_eq!(color_hash(&d, l), original, "全部を戻すと元のバイト");
        parts.push(color_hash(&d, l));
        parts
    });
    assert_eq!(stages.len(), 6); // 元・塗り 4 段・全部を戻したあと
}

/// レイヤーに全チャンネルを持たせるマテリアルの塗りつぶし・グラデーションも、どのスレッド数でも全チャンネルが同じバイト。
#[test]
fn material_region_edits_are_the_same_with_any_number_of_threads() {
    let material = |seed: u8| -> Vec<ChannelPaint> {
        Channel::ALL
            .iter()
            .enumerate()
            .map(|(i, c)| {
                ChannelPaint::new(
                    *c,
                    Rgba8::new(31 + i as u8 * 32, seed, 133, 160 + i as u8 * 10),
                )
            })
            .collect()
    };
    // 面が無い（Undo で外れた）チャンネルは "none"
    let hashes = |d: &Document, l: LayerId| -> Vec<String> {
        Channel::ALL
            .iter()
            .map(|c| {
                d.layer(l)
                    .unwrap()
                    .surface(*c)
                    .map_or("none".to_string(), surface_hash)
            })
            .collect()
    };
    let stages = same_for_every_degree("material region edits", || {
        let (mut d, _) = canvas(true);
        let m = d.add_layer("material").unwrap();
        d.clear_history().unwrap();
        let mut parts = Vec::new();
        assert!(d.fill_material(m, &material(70), 0.6, None, false).unwrap());
        parts.push(hashes(&d, m));
        let gradient = GradientSettings {
            start: DVec2::new(100.0, 60.0),
            end: DVec2::new(480.0, 300.0),
            from: Rgba8::new(255, 0, 0, 255),
            to: Rgba8::new(0, 0, 255, 90),
            opacity: 0.8,
            ..GradientSettings::default()
        };
        assert!(d
            .gradient_material(
                m,
                &material(200),
                Some(&material(20)),
                &gradient,
                None,
                false
            )
            .unwrap());
        parts.push(hashes(&d, m));
        assert!(d.fill_material(m, &material(5), 1.0, None, true).unwrap());
        parts.push(hashes(&d, m));
        for pair in parts.windows(2) {
            assert_ne!(pair[0], pair[1], "段が画素を変えている");
        }
        assert_eq!(d.undo_count(), 3);
        while d.undo().unwrap() {}
        parts.push(hashes(&d, m));
        parts
    });
    assert!(stages
        .last()
        .unwrap()
        .iter()
        .all(|h| h == "none" || h.starts_with("0/0/")));
}

/// C# の RefusedRegionEditsChangeNothingWithAnyNumberOfThreads のうち塗りの分: ストロークの予算で断られる塗りつぶし・グラデーション・
/// マテリアルの塗りは何も変えず（無効のチャンネルを有効にもしない）、巻き戻しの見積もりは通って書き込みの途中で面の画素の予算が尽きる
/// グラデーションも、画素・確保量・履歴・版・変更の通し番号を元のままにする。どのスレッド数でも。予算を戻せば同じ編集が通る。
#[test]
fn refused_region_edits_change_nothing_with_any_number_of_threads() {
    let gradient = GradientSettings {
        start: DVec2::new(100.0, 60.0),
        end: DVec2::new(480.0, 300.0),
        ..GradientSettings::default()
    };
    let material: Vec<_> = [Channel::Color, Channel::Roughness, Channel::Height]
        .iter()
        .map(|c| ChannelPaint::new(*c, Rgba8::new(10, 20, 30, 255)))
        .collect();
    for degree in DEGREES {
        let (mut d, l) = canvas(false);
        let before = color_hash(&d, l);
        let state = |d: &Document| {
            (
                color_hash(d, l),
                d.allocated_bytes(),
                d.undo_count(),
                d.revision(),
                d.change_serial(),
                d.layer(l).unwrap().is_channel_enabled(Channel::Roughness),
            )
        };
        let saved = state(&d);
        d.set_stroke_budget_bytes(64 * 64 * 4 * 5).unwrap(); // 数枚で尽きる
        let refused = with_degree(degree, || {
            [
                d.fill(
                    l,
                    Channel::Color,
                    Rgba8::new(1, 2, 3, 255),
                    1.0,
                    None,
                    false,
                ),
                d.gradient(l, Channel::Color, &gradient, None, false),
                d.fill_material(l, &material, 1.0, None, false),
            ]
        });
        for r in refused {
            assert_eq!(r, Err(CoreError::StrokeBudgetExceeded), "スレッド {degree}");
            assert_eq!(state(&d), saved, "スレッド {degree}");
        }
        // 巻き戻しの見積もりは通り、書き込みの途中で面の予算が尽きる（グラデーションは新しいタイルを全画素へ広げる）
        d.set_stroke_budget_bytes(64 << 20).unwrap();
        d.set_source_budget_bytes(d.allocated_bytes() + 64 * 64 * 4 * 2)
            .unwrap();
        let grown = with_degree(degree, || {
            d.gradient(l, Channel::Color, &gradient, None, false)
        });
        assert_eq!(
            grown,
            Err(CoreError::SourceBudgetExceeded),
            "スレッド {degree}"
        );
        assert_eq!(state(&d), saved, "スレッド {degree}");
        assert_eq!(color_hash(&d, l), before);
        // 予算を戻せば、同じ編集が通る
        d.set_source_budget_bytes(256 << 20).unwrap();
        with_degree(degree, || {
            assert!(d
                .fill(
                    l,
                    Channel::Color,
                    Rgba8::new(1, 2, 3, 255),
                    1.0,
                    None,
                    false
                )
                .unwrap());
            assert!(d
                .gradient(l, Channel::Color, &gradient, None, false)
                .unwrap());
        });
        assert_eq!(d.undo_count(), 2);
    }
}

/// 大きなダブ（半径 70）の効果のブラシ（ぼかし・指先・クローン）が、全チャンネルのマテリアルのストロークでも、マスクへのストロークでも、
/// どのスレッド数でも同じバイトで、取消・やり直しも同じ。ワーカーの経路（複数のスレッドで、外接の箱が 128² 以上）を通る。
#[test]
fn large_effect_dabs_on_every_channel_and_a_mask_are_the_same_with_any_number_of_threads() {
    for effect in [
        BrushEffect::Blur { radius: 3 },
        BrushEffect::Smudge { strength: 0.6 },
        BrushEffect::Clone {
            offset: DVec2::new(-30.0, 12.5),
        },
    ] {
        let brush = Brush {
            effect,
            ..Brush::from(BrushSettings {
                radius: 70.0,
                hardness: 0.5,
                spacing: 0.1,
                opacity: 0.8,
                flow: 0.6,
                ..BrushSettings::default()
            })
        };
        let run = || {
            let mut d = Document::with_tile_size(300, 200, 32).unwrap();
            let l = d.add_layer("paint").unwrap();
            let mut rnd = Rnd(0x2545_F491_4F6C_DD1D);
            for c in Channel::ALL {
                d.set_channel_enabled(l, c, true).unwrap();
                import(&mut d, l, c, 32, &mut rnd, 0.8, false);
            }
            d.add_layer_mask(l).unwrap();
            let mut tile = vec![0u8; 32 * 32 * 4];
            for ty in 0..200u32.div_ceil(32) {
                for tx in 0..300u32.div_ceil(32) {
                    for (i, p) in tile.chunks_exact_mut(4).enumerate() {
                        let w = rnd.next();
                        p.copy_from_slice(&[0, 0, 0, (w >> 40) as u8]);
                        if (tx * 32 + (i as u32 % 32) >= 300) || (ty * 32 + (i as u32 / 32) >= 200)
                        {
                            p[3] = 0;
                        }
                    }
                    d.import_mask_tile(l, TileCoord::new(tx, ty), &tile)
                        .unwrap();
                }
            }
            d.clear_history().unwrap();
            let all = |d: &Document| -> Vec<String> {
                let mut v: Vec<String> = Channel::ALL
                    .iter()
                    .map(|c| surface_hash(d.layer(l).unwrap().surface(*c).unwrap()))
                    .collect();
                v.push(mask_hash(d, l));
                v
            };
            let before = all(&d);
            let material: Vec<_> = Channel::ALL
                .iter()
                .map(|c| yolu_core::material::ChannelPaint::new(*c, Rgba8::new(255, 0, 0, 255)))
                .collect();
            let mut parallel = 0;
            let mut s = d.begin_material_brush_stroke(l, &material, &brush).unwrap();
            for p in path(30.0, 100.0, 240.0, 20) {
                s.add_sample(&mut d, p).unwrap();
            }
            parallel += d.active_stroke_stats().unwrap().parallel_dabs;
            assert!(d.end_stroke(s).unwrap().changed);
            let painted = all(&d);
            let mut s = d.begin_brush_mask_stroke(l, &brush).unwrap();
            for p in path(30.0, 60.0, 240.0, 20) {
                s.add_sample(&mut d, p).unwrap();
            }
            parallel += d.active_stroke_stats().unwrap().parallel_dabs;
            assert!(d.end_stroke(s).unwrap().changed);
            let masked = all(&d);
            assert_ne!(painted, before);
            assert_ne!(masked, painted);
            assert_eq!(d.undo_count(), 2);
            assert!(d.undo().unwrap());
            assert_eq!(all(&d), painted, "マスクの Undo");
            assert!(d.undo().unwrap());
            assert_eq!(all(&d), before, "全チャンネルの Undo");
            assert!(d.redo().unwrap() && d.redo().unwrap());
            assert_eq!(all(&d), masked, "Redo");
            ((before, painted, masked, d.history_bytes()), parallel)
        };
        let one = with_degree(1, run);
        assert_eq!(one.1, 0, "{effect:?}: 1 スレッドはワーカーを使わない");
        for degree in [2, 3, 0] {
            let other = with_degree(degree, run);
            assert!(
                other.1 > 0,
                "{effect:?}: スレッド {degree} はワーカーで描く"
            );
            assert_eq!(other.0, one.0, "{effect:?}: スレッド {degree}");
        }
    }
}

// ───────── レイヤーの操作（変形・結合）: 画面の移動・変形のツールと結合が頼る ─────────

use yolu_core::{Affine2D, LayerLocks, Resampling};

/// 600×360 の canvas に、Height も持ち、マスクのあるレイヤーを足したもの。レイヤーは下から bg・paint・heights（Color と Height・マスク）。
fn operation_canvas(selection: bool) -> (Document, LayerId, LayerId) {
    let (mut d, paint) = canvas(selection);
    let mut rnd = Rnd(0xD1B5_4A32_D192_ED03);
    let heights = d.add_layer("heights").unwrap();
    import(&mut d, heights, Channel::Color, 64, &mut rnd, 0.5, false);
    import(&mut d, heights, Channel::Height, 64, &mut rnd, 0.5, true);
    d.add_layer_mask(heights).unwrap();
    assert!(d.fill_mask(heights, 0.6, None, false).unwrap());
    d.clear_history().unwrap();
    (d, paint, heights)
}

/// 画素・確保量・選択範囲・履歴・版を 1 つの文字列にして、スレッド数どうしで比べる。
fn operation_fingerprint(d: &Document, ids: &[LayerId]) -> String {
    let mut parts = Vec::new();
    for &id in ids {
        let l = d.layer(id).unwrap();
        for c in [Channel::Color, Channel::Height] {
            parts.push(l.surface(c).map_or("-".to_owned(), surface_hash));
        }
        parts.push(
            l.mask()
                .map_or("-".to_owned(), |m| surface_hash(m.surface())),
        );
    }
    parts.push(format!(
        "sel={}",
        d.selection().map_or("-".to_owned(), |m| {
            let mut bytes = Vec::new();
            for y in 0..d.height() {
                for x in 0..d.width() {
                    bytes.push(m.amount(x, y));
                }
            }
            hash(&bytes)
        })
    ));
    parts.push(format!(
        "alloc={} undo={} rev={}",
        d.allocated_bytes(),
        d.undo_count(),
        d.revision()
    ));
    parts.join("|")
}

fn transforms() -> Vec<(&'static str, Affine2D, Resampling)> {
    let parts = |pivot: (f64, f64), mv: (f64, f64), deg: f64, sc: (f64, f64)| {
        Affine2D::from_parts(pivot, mv, deg, sc).unwrap()
    };
    vec![
        (
            "整数の移動",
            Affine2D::translation(37.0, -21.0),
            Resampling::Bilinear,
        ),
        (
            "小数の移動",
            Affine2D::translation(10.5, 3.25),
            Resampling::Bilinear,
        ),
        (
            "任意の角度",
            parts((300.0, 180.0), (0.0, 0.0), 23.5, (1.0, 1.0)),
            Resampling::Bilinear,
        ),
        (
            "拡大",
            parts((120.0, 90.0), (5.0, -5.0), 0.0, (2.5, 1.75)),
            Resampling::Bilinear,
        ),
        (
            "縮小と回転",
            parts((300.0, 180.0), (0.0, 0.0), -71.0, (0.4, 0.6)),
            Resampling::Bilinear,
        ),
        (
            "左右反転",
            parts((300.0, 180.0), (0.0, 0.0), 0.0, (-1.0, 1.0)),
            Resampling::Bilinear,
        ),
        (
            "90° 回転",
            parts((300.0, 180.0), (0.0, 0.0), 90.0, (1.0, 1.0)),
            Resampling::Bilinear,
        ),
        (
            "ニアレストの回転",
            parts((300.0, 180.0), (0.0, 0.0), 33.0, (1.3, 1.3)),
            Resampling::Nearest,
        ),
        (
            "ニアレストの縮小",
            parts((0.0, 0.0), (0.0, 0.0), 0.0, (0.37, 0.37)),
            Resampling::Nearest,
        ),
    ]
}

/// 変形は、どのスレッド数でも段ごとに同じバイトで（全チャンネル・マスク・選択範囲の持ち上げを含む）、全部を Undo すると元のバイトへ戻る。
#[test]
fn layer_transforms_are_the_same_with_any_number_of_threads() {
    for selection in [false, true] {
        for (name, t, resampling) in transforms() {
            let stages = same_for_every_degree(name, || {
                let (mut d, paint, heights) = operation_canvas(selection);
                let ids = [paint, heights];
                let original = operation_fingerprint(&d, &ids);
                // 2 つのレイヤーをまとめて 1 回で（グループなら中身ごとの経路と同じ transform_layers）
                assert!(d.transform_layers(&ids, t, resampling).unwrap(), "{name}");
                let moved = operation_fingerprint(&d, &ids);
                assert_ne!(moved, original, "{name}: 変形が画素を変えている");
                // 続けて 1 つのレイヤーだけ（マスク込み）を、同じ変形で
                d.transform_layer(heights, t, resampling, true).unwrap();
                let twice = operation_fingerprint(&d, &ids);
                while d.undo().unwrap() {}
                let back = operation_fingerprint(&d, &ids);
                assert_eq!(
                    back.split("|alloc").next(),
                    original.split("|alloc").next(),
                    "{name}: 全部を戻すと元のバイト"
                );
                vec![original, moved, twice, back]
            });
            assert_eq!(stages.len(), 4, "{name}");
        }
    }
}

/// 変形の取消（タイルのまとまりごとに確認）は、どのスレッド数でも何も変えず、予算の拒否も元のバイトのまま。
#[test]
fn cancelled_or_refused_transforms_leave_the_exact_pixels_with_any_number_of_threads() {
    let t = Affine2D::from_parts((300.0, 180.0), (0.0, 0.0), 17.0, (1.2, 1.2)).unwrap();
    for degree in DEGREES {
        with_degree(degree, || {
            let (mut d, paint, heights) = operation_canvas(true);
            let ids = [paint, heights];
            let before = operation_fingerprint(&d, &ids);
            // 最初の確認で取り消す・3 回目の確認で取り消す
            for stop_at in [0usize, 2] {
                let mut calls = 0;
                let result =
                    d.transform_layers_cancellable(&ids, t, Resampling::Bilinear, &mut || {
                        calls += 1;
                        calls > stop_at
                    });
                assert_eq!(result, Err(CoreError::Cancelled), "{degree}: {stop_at}");
                assert_eq!(
                    operation_fingerprint(&d, &ids),
                    before,
                    "{degree}: {stop_at}"
                );
            }
            // 一操作の予算が足りない
            let budget = d.stroke_budget_bytes();
            d.set_stroke_budget_bytes(4096).unwrap();
            assert_eq!(
                d.transform_layers(&ids, t, Resampling::Bilinear),
                Err(CoreError::StrokeBudgetExceeded),
                "{degree}"
            );
            assert_eq!(
                operation_fingerprint(&d, &ids),
                before,
                "{degree}: 予算の拒否"
            );
            d.set_stroke_budget_bytes(budget).unwrap();
            // ロックの拒否
            d.set_layer_locks(heights, LayerLocks::POSITION).unwrap();
            let locked = operation_fingerprint(&d, &ids);
            assert!(matches!(
                d.transform_layers(&ids, t, Resampling::Bilinear),
                Err(CoreError::LayerLocked { .. })
            ));
            assert_eq!(
                operation_fingerprint(&d, &ids),
                locked,
                "{degree}: ロックの拒否"
            );
        });
    }
}

/// 結合の前後の合成を、結合の報告と同じ規則（両方とも透明な画素は数えない）で比べる。返すのは（変わった画素の数、チャンネルごとの最大の差）。
fn color_difference(before: &[u8], after: &[u8]) -> (u64, u8) {
    assert_eq!(before.len(), after.len());
    let (mut changed, mut largest) = (0u64, 0u8);
    for (a, b) in after.chunks_exact(4).zip(before.chunks_exact(4)) {
        if a[3] == 0 && b[3] == 0 {
            continue;
        }
        let delta = (0..4).map(|q| a[q].abs_diff(b[q])).max().expect("RGBA");
        if delta != 0 {
            changed += 1;
            largest = largest.max(delta);
        }
    }
    (changed, largest)
}

/// 結合（下のレイヤー・複数のレイヤー・表示しているレイヤー）は、どのスレッド数でも同じバイトで、Undo で元に戻る。
#[test]
fn layer_merges_are_the_same_with_any_number_of_threads() {
    for kind in ["down", "layers", "visible"] {
        same_for_every_degree(kind, || {
            let (mut d, paint, heights) = operation_canvas(false);
            let ids = [paint, heights];
            let before = operation_fingerprint(&d, &ids);
            let composite = d.composite(d.bounds()).unwrap();
            let report = match kind {
                "down" => d.merge_down(heights, 255).unwrap(),
                "layers" => d.merge_layers(&ids, 255).unwrap(),
                _ => d.merge_visible("merged", 255).unwrap(),
            };
            let merged = report.result_id;
            let after = operation_fingerprint(&d, &[merged]);
            // 結合の前後の Color の合成は、報告と同じ規則で比べて報告と合う（変わった画素の数が同じで、最大の差は報告の内。報告が
            // 正確なら 1 画素も変わらない）。報告そのものがスレッド数で変わらないことは、返す値の比べ合わせが見る
            let (changed, largest) =
                color_difference(&composite, &d.composite(d.bounds()).unwrap());
            assert_eq!(
                changed,
                report
                    .changed_by_channel
                    .get(&Channel::Color)
                    .copied()
                    .unwrap_or(0),
                "{kind}: 報告の変わった画素の数"
            );
            assert!(
                largest <= report.max_difference,
                "{kind}: {largest} / {}",
                report.max_difference
            );
            if report.exact() {
                assert_eq!(changed, 0, "{kind}");
            }
            assert!(d.undo().unwrap());
            assert_eq!(
                operation_fingerprint(&d, &ids).split("|alloc").next(),
                before.split("|alloc").next(),
                "{kind}: 取り消すと元のバイト"
            );
            (after, report.changed_pixels, report.max_difference)
        });
    }
}

/// 3D の面のストロークの投影の塗り（覆いの集め・候補の並べ替え）をワーカーで行う道は、直列の道と同じバイトで、どのスレッド数でも変わらない。
/// 既定では大きなダブだけがワーカーを使うので、下限を 0（いつもワーカー）にして、小さなブラシの線でも通す。
#[test]
fn surface_strokes_are_the_same_whether_the_projection_gathers_on_workers_or_not() {
    use std::sync::Arc;
    use yolu_core::geometry::{
        cube_sphere, model_triangles, set_parallel_projection_candidates, CameraView, Projection,
        SurfaceGeometry, SurfaceStroke, SurfaceStrokeOptions, DEFAULT_WELD_TOLERANCE,
    };
    use yolu_core::glam::{Quat, Vec2, Vec3};
    let geometry = Arc::new(
        SurfaceGeometry::new(
            model_triangles(&[cube_sphere(18, 0.5)]).unwrap(),
            1,
            DEFAULT_WELD_TOLERANCE,
        )
        .unwrap(),
    );
    // 回転なしの正投影のカメラ（三角関数を通らない）
    let view = CameraView {
        position: Vec3::new(0.0, 0.0, -3.0),
        rotation: Quat::IDENTITY,
        forward: Vec3::Z,
        near: 0.1,
        far: 100.0,
        width: 640.0,
        height: 480.0,
        projection: Projection::Orthographic { height: 1.2 },
    };
    let stroke_hash = |threshold: usize, degree: usize| -> String {
        let previous = set_parallel_projection_candidates(threshold);
        let hash = with_degree(degree, || {
            let mut doc = Document::new(256, 256).unwrap();
            let layer = doc.add_layer("a").unwrap();
            let mut brush = Brush::from(BrushSettings {
                radius: 14.0,
                hardness: 0.8,
                color: Rgba8::new(200, 60, 30, 255),
                pressure_size: false,
                pressure_opacity: false,
                ..BrushSettings::default()
            });
            brush.seed = 7;
            let mut stroke = doc.begin_brush_stroke(layer, &brush).unwrap();
            let c = Vec2::new(320.0, 240.0);
            let mut s = SurfaceStroke::begin_with_options(
                &mut doc,
                &mut stroke,
                geometry.clone(),
                view,
                &brush.base,
                Some(0),
                c + Vec2::new(-150.0, -30.0),
                1.0,
                SurfaceStrokeOptions::default(),
            )
            .unwrap();
            for p in [
                Vec2::new(150.0, -60.0),
                Vec2::new(-140.0, 20.0),
                Vec2::new(140.0, 90.0),
            ] {
                s.add(&mut doc, &mut stroke, c + p, 1.0).unwrap();
            }
            s.finish(&mut doc, &mut stroke).unwrap();
            assert!(s.stats.dabs > 50, "{:?}", s.stats);
            doc.end_stroke(stroke).unwrap();
            color_hash(&doc, layer)
        });
        set_parallel_projection_candidates(previous);
        hash
    };
    let sequential = stroke_hash(usize::MAX, 1);
    for degree in DEGREES {
        assert_eq!(
            stroke_hash(0, degree),
            sequential,
            "ワーカーで集めても（スレッド {degree}）同じバイト"
        );
        assert_eq!(
            stroke_hash(usize::MAX, degree),
            sequential,
            "直列で集めても（スレッド {degree}）同じバイト"
        );
    }
}
