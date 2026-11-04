//! 計測: ほかのテクスチャ・マップを参照して描く塗りつぶし（トライプラナー・位置のグラデーション）の重さ。合否は持たない（`#[ignore]`）。
//!
//! 文書 N² に、ラスターのレイヤー 10 枚（画素あり。下 5 枚・塗りつぶし・上 5 枚）と、場面ごとの塗りつぶしを置き、
//! 試しの立方体の位置・法線のマップ（文書と同じ大きさ。大きさが違うと塗りつぶしは値を見せる）を CPU で焼いて渡す。
//! 場面: 0 塗りつぶし無し / A トライプラナー 1 枚（Color・Normal・Roughness の 3 チャンネルに 2048² の画像）/ B A を 8 枚（画像 4 種を使い回す）/
//! C マスクに位置のグラデーションの塗りつぶし 1 枚 / D C を 8 枚 / E1 A の投影を UV にした物 / E2 C のマスクを素のラスターにした物（基準）/ E8 E1 を 8 枚。
//!
//! 使い方（本物の GPU は d3d12 の OpenGL。スキル test の「GPU」）:
//! `REFFILL_SIDE=4096 REFFILL_SCENES=0,A,B,C,D,E1,E2 REFFILL_RUNS=5 WGPU_BACKEND=gl LD_LIBRARY_PATH=/opt/mesa-d3d12/lib:/usr/lib/wsl/lib GALLIUM_DRIVER=d3d12 \
//!  cargo test --release -p yolu-app --test gui_view3d ref_fill_measure -- --ignored --nocapture`
//! 出力: `RESULT<TAB>場面<TAB>測る物<TAB>中央値(ms)<TAB>最大(ms)<TAB>回数<TAB>メモ`。
//! 測る物: cold_color / cold_3ch（キャッシュ空からの全面の合成。core の直）/ warm_color（続けてもう一度）/
//! stroke_live_*（ストローク中の 1 フレームぶんの合成。10 点ごと）/ stroke_end_*（確定後の更新。core の直）/
//! app_stroke_*（確定後にアプリのフレームを回して落ち着くまで。2D と 3D の両方）/ setting_core・app_setting（塗りつぶしの設定を 1 つ変えた後）/
//! rotate_3d（3D ビューを回した 1 フレーム）/ switch_*（見せるチャンネルを Color → Normal）。
//! `DETAIL` の行（`REFFILL_PARTS=detail`）: 全面が変わる操作の後のフレームの内訳（3D の prepare・塗った絵の同期の CPU、キャンバスの上げたタイル）。
//! 設定は製品と同じく読む（効果のメモリは自動。`REFFILL_EFFECT_MIB` で値を選ぶ）。`REFFILL_CACHE_MIB` は設定を読まずに予算を直に入れる。
use crate::common;
use crate::common::drive::{Drive, DriveApp};

use common::*;
use egui_kittest::Harness;
use std::time::Instant;
use yolu_app::bake::{BakeAction, BakeBackend};
use yolu_app::fillfx::{inputs, FillOp};
use yolu_app::lang::Lang;
use yolu_app::state::Action;
use yolu_app::YoluApp;
use yolu_core::fill_image::ProjectionMode;
use yolu_core::generator::{self, MapState};
use yolu_core::glam::DVec2;
use yolu_core::mesh_maps::MeshMapKind;
use yolu_core::{
    BrushSettings, Channel, Document, EffectSettings, FilterId, FilterSpec, FilterTarget, LayerId,
    MapInput, Rgba8, TileCoord,
};

type H = Harness<'static, YoluApp>;

const CHANNELS: [Channel; 3] = [Channel::Color, Channel::Normal, Channel::Roughness];
const IMAGE_SIDE: u32 = 2048;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    s[s.len() / 2]
}

fn max(v: &[f64]) -> f64 {
    v.iter().cloned().fold(f64::MIN, f64::max)
}

fn row(scene: &str, metric: &str, v: &[f64], note: &str) {
    println!(
        "RESULT\t{scene}\t{metric}\t{:.1}\t{:.1}\t{}\t{note}",
        median(v),
        max(v),
        v.len()
    );
}

/// 評価の回数の差（診断の数。キャッシュの当たり外れの代わり: 評価したブロックの数とミップを作った数）。
#[derive(Clone, Copy, Default)]
struct Counts {
    blocks: u64,
    mips: u64,
}

fn counts(doc: &Document) -> Counts {
    let c = doc.effect_counters();
    Counts {
        blocks: c.blocks_evaluated,
        mips: c.mip_chains_built,
    }
}

fn delta(doc: &Document, before: Counts) -> String {
    let now = counts(doc);
    format!(
        "blocks+{} mips+{} cache={}MiB images={}MiB",
        now.blocks - before.blocks,
        now.mips - before.mips,
        doc.effect_counters().cache_bytes >> 20,
        doc.effect_counters().image_cache_bytes >> 20
    )
}

/// 乱数（毎回同じ）。
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
}

/// 2048² の模様（格子・斜めの縞・ノイズ。圧縮されない中身）。
fn pattern(seed: u64) -> Vec<u8> {
    let n = IMAGE_SIDE;
    let mut rng = Rng(seed);
    let mut pixels = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let grid = ((x >> 6) + (y >> 6) + seed as u32).is_multiple_of(2);
            let stripe = ((x + y) >> 4) % 2 == 0;
            let noise = (rng.next() & 0x1f) as u8;
            pixels.extend_from_slice(&[
                (if grid { 200u8 } else { 50 }).wrapping_add(noise),
                (if stripe { 170u8 } else { 70 }).wrapping_add(noise),
                ((x * 255 / n) as u8).wrapping_add(noise),
                255,
            ]);
        }
    }
    pixels
}

struct Env {
    h: H,
    side: u32,
    rasters: Vec<LayerId>,
    images: Vec<yolu_core::ImageId>,
    /// ストロークの始点（位置のマップの被覆が最も多い所を通る線。文書の大きさに対する割合）。
    origin: (f64, f64),
}

impl Env {
    fn doc(&mut self) -> &mut Document {
        self.h.doc_mut()
    }
}

/// 試しの立方体のマップを焼き、ラスターのレイヤー 10 枚と画像 4 種を置く。
fn setup(side: u32) -> Env {
    // REFFILL_TABBED=1: 中央を 1 つの組（キャンバスだけ描く）にする。既定は製品と同じ並び（キャンバスと 3D ビューを左右に）
    let mut h = if std::env::var("REFFILL_TABBED").is_ok() {
        app(1470.0, 760.0, side)
    } else {
        app_default(1470.0, 760.0, side)
    };
    h.view3d_mut().load_demo();
    let mut origin = (0.12, 0.22);
    let t = Instant::now();
    {
        let s = h.app_mut();
        s.bake.backend = BakeBackend::Cpu;
        s.bake.settings.maps = vec![MeshMapKind::WorldNormal, MeshMapKind::Position];
        s.bake.settings.padding = 4;
        s.apply(Action::Bake(BakeAction::Start));
        s.wait_bake();
    }
    eprintln!("マップを焼いた {side}²: {:.0} ms", ms(t));
    {
        let s = h.app_mut();
        let mut effect_inputs = s.session.doc.effect_inputs().clone();
        for kind in [MeshMapKind::Position, MeshMapKind::WorldNormal] {
            let map = s
                .sets()
                .current()
                .mesh_maps
                .get(kind)
                .expect("焼いたマップ")
                .clone();
            let input = MapInput::from_baked(&map, MapState::Current).expect("マップ");
            if kind == MeshMapKind::Position {
                let covered = input.coverage.iter().filter(|c| **c > 0).count();
                // ストロークの線は、被覆の多い所を通す（被覆の無い所は塗りつぶしが何も描かず、合成が軽くなって測れない）
                let (w, hh) = (input.width as usize, input.height as usize);
                let mut best = (0.0, origin);
                for gy in 0..14 {
                    for gx in 0..14 {
                        let (ox, oy) = (0.02 + gx as f64 * 0.05, 0.02 + gy as f64 * 0.05);
                        let mut hit = 0;
                        for i in 0..100 {
                            let u = i as f64 / 99.0;
                            let x = ((ox + 0.14 * u) * w as f64) as usize;
                            let y = ((oy + 0.08 * u) * hh as f64) as usize;
                            if x < w && y < hh && input.coverage[y * w + x] > 0 {
                                hit += 1;
                            }
                        }
                        if hit as f64 > best.0 {
                            best = (hit as f64, (ox, oy));
                        }
                    }
                }
                origin = best.1;
                eprintln!("ストロークの始点 {:?}、線上の被覆 {:.0}%", origin, best.0);
                eprintln!(
                    "位置のマップ {}x{}、被覆 {:.1}%",
                    input.width,
                    input.height,
                    covered as f64 * 100.0 / (input.width as f64 * input.height as f64)
                );
            }
            effect_inputs = effect_inputs.with_map(input).expect("入力");
        }
        s.session
            .doc
            .set_effect_inputs(effect_inputs)
            .expect("入力を置く");
        s.session
            .doc
            .set_source_budget_bytes(4 << 30)
            .expect("画素の予算");
    }
    // ラスターのレイヤー 10 枚（最初の 1 枚は文書が持つ）。タイルの 2〜3 割に半透明の画素
    let tile = h.doc().tile_size();
    let mut rasters = vec![h.doc().layers()[0].id()];
    {
        let doc = h.doc_mut();
        for i in 1..10 {
            rasters.push(doc.add_layer(&format!("raster {i}")).unwrap());
        }
        let n = side.div_ceil(tile);
        for (li, id) in rasters.iter().enumerate() {
            let mut bytes = vec![0u8; (tile * tile * 4) as usize];
            for (i, p) in bytes.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let (x, y) = (i as u32 % tile, i as u32 / tile);
                p.copy_from_slice(&[
                    (x + li as u32 * 20) as u8,
                    (y * 2) as u8,
                    (li * 25) as u8,
                    170,
                ]);
            }
            for ty in 0..n {
                for tx in 0..n {
                    if (tx * 7 + ty * 13 + li as u32 * 5).is_multiple_of(5) {
                        doc.import_tile(*id, Channel::Color, TileCoord::new(tx, ty), &bytes)
                            .unwrap();
                    }
                }
            }
        }
    }
    // 画像 4 種をライブラリに（塗りつぶしの画像の ID はライブラリの ID と同じ）
    let mut images = Vec::new();
    for seed in 1..=4u64 {
        let pixels = pattern(seed);
        let rid = h
            .state_mut()
            .state
            .shelf
            .add_image(
                Lang::Ja,
                &format!("pattern{seed}"),
                &pixels,
                IMAGE_SIDE,
                IMAGE_SIDE,
            )
            .expect("ライブラリへ入る");
        images.push(inputs::image_id(&rid).expect("GUID"));
    }
    h.run();
    Env {
        h,
        side,
        rasters,
        images,
        origin,
    }
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    /// 画像を 3 チャンネルに（`uv` なら UV の投影）。
    Image { uv: bool },
    /// マスクに位置のグラデーション。
    Gradient,
    /// マスクが素のラスター（グラデーションの基準）。
    PlainMask,
}

/// 場面の塗りつぶしを足す。`above` の上に積む。足したレイヤーの ID を返す。
fn add_fills(env: &mut Env, kind: Kind, count: usize) -> Vec<(LayerId, Option<FilterId>)> {
    let mut above = env.rasters[4];
    let mut out = Vec::new();
    for k in 0..count {
        let image = env.images[k % env.images.len()];
        let values = [
            (Channel::Color, Rgba8::new(200, 120, 60, 255)),
            (Channel::Normal, Rgba8::new(128, 128, 255, 255)),
            (Channel::Roughness, Rgba8::new(128, 128, 128, 255)),
        ];
        let mut filter = None;
        let doc = env.doc();
        let id = match kind {
            Kind::Image { .. } => doc.add_fill_layer(&format!("fill {k}"), &[], Some(above)),
            _ => doc.add_fill_layer(&format!("fill {k}"), &values, Some(above)),
        }
        .unwrap();
        match kind {
            Kind::Image { uv } => {
                for channel in CHANNELS {
                    env.h.app_mut().apply(Action::Fill(FillOp::Image {
                        layer: id,
                        channel,
                        image: Some(image),
                    }));
                }
                if !uv {
                    env.h
                        .state_mut()
                        .state
                        .apply(Action::Fill(FillOp::ProjectionMode {
                            layer: id,
                            mode: ProjectionMode::Triplanar,
                        }));
                }
            }
            Kind::Gradient | Kind::PlainMask => {
                let doc = env.doc();
                doc.add_layer_mask(id).unwrap();
                if matches!(kind, Kind::Gradient) {
                    let mut g = generator::Settings::new(generator::Kind::PositionGradient);
                    g.axis = k % 3;
                    g.blend = generator::Blend::Replace;
                    g.low = 0.05 * (k % 4) as f64;
                    g.high = 1.0 - 0.05 * ((k + 1) % 4) as f64;
                    filter = Some(
                        doc.add_filter(
                            id,
                            FilterTarget::Mask,
                            FilterSpec::new(EffectSettings::generator(g)),
                        )
                        .unwrap(),
                    );
                }
            }
        }
        above = id;
        out.push((id, filter));
    }
    out
}

/// アプリのフレームを、再描画の要求が止むまで回す。（かかった ms, フレームの数）。3D の GPU の積みが終わるまで待つ。
fn settle(h: &mut H) -> (f64, u64) {
    let t = Instant::now();
    let mut frames = 0;
    loop {
        match h.try_run() {
            Ok(n) => {
                frames += n;
                break;
            }
            Err(_) => {
                frames += 121;
                assert!(frames < 100_000, "落ち着かない");
            }
        }
    }
    h.view3d_wait_gpu();
    (ms(t), frames)
}

/// 見せるチャンネルを替える。
fn show(h: &mut H, channel: Channel) {
    h.app_mut().m2.display_channel = channel;
}

fn all_tiles(doc: &Document) -> Vec<TileCoord> {
    doc.canvas_tiles().collect()
}

fn composite(doc: &Document, channel: Channel, coords: &[TileCoord]) -> f64 {
    let t = Instant::now();
    let tiles = doc.composite_tiles(channel, coords).expect("合成");
    std::hint::black_box(&tiles);
    ms(t)
}

/// 100 点のストローク。`live` なら 10 点ごとに変わったタイルを合成した時間を返す。
fn stroke(
    doc: &mut Document,
    layer: LayerId,
    side: u32,
    origin: (f64, f64),
    k: usize,
    live: bool,
) -> Vec<f64> {
    let brush = BrushSettings {
        radius: 6.0,
        color: Rgba8::new(255, 0, 0, 255),
        ..Default::default()
    };
    let mut s = doc.begin_stroke_in(layer, Channel::Color, &brush).unwrap();
    let w = side as f64;
    let mut frames = Vec::new();
    let mut since = doc.change_serial();
    // 位置のマップの被覆が多い所を斜めに（500 画素ほど）。回ごとに少しずらす
    for i in 0..100 {
        let u = i as f64 / 99.0;
        let x = w * (origin.0 + 0.14 * u) + k as f64 * 3.0;
        let y = w * (origin.1 + 0.08 * u) + k as f64 * 5.0;
        s.add_point(doc, x, y, 1.0, DVec2::ZERO).unwrap();
        if live && i % 10 == 9 {
            if let Some(changed) = doc.changed_tiles(Channel::Color, since) {
                since = doc.change_serial();
                frames.push(composite(doc, Channel::Color, &changed));
            }
        }
    }
    doc.end_stroke(s).unwrap();
    frames
}

/// 測る部分の選び（`REFFILL_PARTS=cold,stroke,appstroke,setting,rotate,switch`。既定は全部）。
fn part(name: &str) -> bool {
    std::env::var("REFFILL_PARTS").map_or(true, |v| v.split(',').any(|p| p == name))
}

fn run_scene(env: &mut Env, name: &str, kind: Option<Kind>, count: usize, runs: usize) {
    let side = env.side;
    let origin = env.origin;
    let fill_specs = kind.map_or_else(Vec::new, |k| add_fills(env, k, count));
    let fills: Vec<LayerId> = fill_specs.iter().map(|f| f.0).collect();
    // 画面を一度落ち着かせて、効かない効果が無いことを確かめる
    // 開いた直後の 1 フレームずつ（重いフレームの内訳: 時間・3D の prepare と塗った絵の同期の CPU・上げたタイルの数）
    let mut detail = Vec::new();
    {
        let mut quiet = 0;
        let t0 = Instant::now();
        while quiet < 3 && detail.len() < 2000 {
            let t = Instant::now();
            env.h.step();
            env.h.view3d_wait_gpu();
            let m = ms(t);
            let st = env.h.view3d_stats().unwrap();
            detail.push((
                m,
                st.last_prepare_us as f64 / 1000.0,
                st.last_sync_us as f64 / 1000.0,
                st.last_tiles,
            ));
            quiet = if m < 30.0 { quiet + 1 } else { 0 };
        }
        let mut top = detail.clone();
        top.sort_by(|a, b| b.0.total_cmp(&a.0));
        top.truncate(3);
        println!(
            "OPEN_DETAIL {name}: 合計 {:.0} ms / {} フレーム。重い 3 フレーム (ms, 3D prepare CPU ms, 塗った絵の同期 CPU ms, 上げたタイル): {:?}",
            ms(t0),
            detail.len(),
            top
        );
    }
    let (open_ms, open_frames) = (detail.iter().map(|d| d.0).sum::<f64>(), detail.len() as u64);
    let inactive = env.doc().inactive_effects();
    println!(
        "SCENE {name}: 塗りつぶし {} 枚、アプリが最初に落ち着くまで {open_ms:.0} ms/{open_frames} フレーム、効かない効果 {inactive:?}",
        fills.len()
    );
    let tiles = all_tiles(env.doc());
    let below = env.rasters[4];
    let above = env.rasters[5];
    let far = env.rasters[9];

    // 1. キャッシュ空からの全面の合成
    if part("cold") {
        let (mut cold_color, mut cold_3ch, mut warm) = (Vec::new(), Vec::new(), Vec::new());
        let mut note = String::new();
        for _ in 0..runs {
            let doc = env.doc();
            doc.release_effect_cache();
            let c0 = counts(doc);
            cold_color.push(composite(doc, Channel::Color, &tiles));
            note = delta(doc, c0);
            warm.push(composite(doc, Channel::Color, &tiles));
        }
        row(name, "cold_color", &cold_color, &note);
        let warm_note = format!("同じ合成をもう一度。{note}");
        row(name, "warm_color", &warm, &warm_note);
        // キャンバスは 64 タイルずつ合成する（`CHUNK_MAX`）。同じ分け方でキャッシュ空から全面を作る時間
        let mut chunked = Vec::new();
        for _ in 0..runs {
            let doc = env.doc();
            doc.release_effect_cache();
            let t = Instant::now();
            for chunk in tiles.chunks(64) {
                composite(doc, Channel::Color, chunk);
            }
            chunked.push(ms(t));
        }
        row(
            name,
            "cold_color_chunks64",
            &chunked,
            "64 タイルずつ（キャンバスと同じ）",
        );
        let mut per_channel = [Vec::new(), Vec::new(), Vec::new()];
        for _ in 0..runs {
            let doc = env.doc();
            doc.release_effect_cache();
            let c0 = counts(doc);
            let mut total = 0.0;
            for (i, ch) in CHANNELS.iter().enumerate() {
                let t = composite(doc, *ch, &tiles);
                per_channel[i].push(t);
                total += t;
            }
            cold_3ch.push(total);
            note = delta(doc, c0);
        }
        row(name, "cold_3ch", &cold_3ch, &note);
        for (i, ch) in CHANNELS.iter().enumerate() {
            row(name, &format!("cold_3ch_{ch:?}"), &per_channel[i], "");
        }
    }
    // 2・3. 関係ないレイヤー・塗りつぶしの上・下に短いストローク。core の直（ストローク中の 1 フレームと、確定後の更新）
    if part("stroke") {
        for (label, layer) in [("far", far), ("above", above), ("below", below)] {
            // 全チャンネルをキャッシュに載せてから
            let doc = env.doc();
            for ch in CHANNELS {
                composite(doc, ch, &tiles);
            }
            let (mut live, mut end, mut ntiles) = (Vec::new(), Vec::new(), 0);
            let mut note = String::new();
            let total0 = counts(env.doc());
            for k in 0..runs {
                let doc = env.doc();
                let c0 = counts(doc);
                let serial = doc.change_serial();
                live.extend(stroke(doc, layer, side, origin, k, true));
                // 確定後（end_stroke が追加で変えたタイル）の更新
                let changed = doc
                    .changed_tiles(Channel::Color, serial)
                    .unwrap_or_default();
                ntiles = changed.len();
                end.push(composite(doc, Channel::Color, &changed));
                note = delta(doc, c0);
            }
            let total = counts(env.doc());
            let slow = live.iter().filter(|t| **t > 30.0).count();
            row(
            name,
            &format!("stroke_live_{label}"),
            &live,
            &format!(
                "10 点ごとの 1 フレーム。全 {runs} 回で評価したブロック +{}、30 ms を超えたフレーム {slow}/{}。{note}",
                total.blocks - total0.blocks,
                live.len()
            ),
        );
            row(
                name,
                &format!("stroke_end_{label}"),
                &end,
                &format!("変わったタイル {ntiles}/{}。{note}", tiles.len()),
            );
        }
    }
    // 2・3'. アプリのフレームを回して落ち着くまで（2D と 3D の両方）
    if part("appstroke") {
        settle(&mut env.h);
        for (label, layer) in [("far", far), ("above", above), ("below", below)] {
            let (mut t, mut frames, mut note) = (Vec::new(), Vec::new(), String::new());
            for k in 0..runs {
                let c0 = counts(env.doc());
                stroke(env.doc(), layer, side, origin, k + 20, false);
                let (m, f) = settle(&mut env.h);
                t.push(m);
                frames.push(f as f64);
                note = delta(env.doc(), c0);
            }
            let s = env.h.view3d_stats();
            row(
                name,
                &format!("app_stroke_{label}"),
                &t,
                &format!(
                    "frames中央値{} 3Dの最後の同期タイル{:?}。{note}",
                    median(&frames),
                    s.map(|s| s.last_tiles)
                ),
            );
        }
    }
    // 4. 塗りつぶしの設定を 1 つ変える（最初の塗りつぶし）
    if part("setting") {
        if let Some(&(first, first_filter)) = fill_specs.first() {
            // キャッシュを載せなおす
            {
                let doc = env.doc();
                for ch in CHANNELS {
                    composite(doc, ch, &tiles);
                }
            }
            let (mut core_t, mut app_t, mut note) = (Vec::new(), Vec::new(), String::new());
            for k in 0..runs {
                let c0 = counts(env.doc());
                change_setting(env.doc(), first, first_filter, kind.unwrap(), k + 1);
                core_t.push(composite(env.doc(), Channel::Color, &tiles));
                note = delta(env.doc(), c0);
            }
            row(
                name,
                "setting_core",
                &core_t,
                &format!("Color だけ。{note}"),
            );
            settle(&mut env.h);
            for k in 0..runs {
                let c0 = counts(env.doc());
                change_setting(env.doc(), first, first_filter, kind.unwrap(), k + 20);
                let (m, _) = settle(&mut env.h);
                app_t.push(m);
                note = delta(env.doc(), c0);
            }
            row(name, "app_setting", &app_t, &note);
        }
    }
    // 5. 3D ビューを回した 1 フレーム
    if part("rotate") {
        {
            settle(&mut env.h);
            let c0 = counts(env.doc());
            let (mut wall, mut prepare, mut sync) = (Vec::new(), Vec::new(), Vec::new());
            for frame in 0..60 {
                env.h.view3d_mut().view.camera.yaw += 3.0 + frame as f32 * 0.01;
                let t = Instant::now();
                env.h.step();
                env.h.view3d_wait_gpu();
                if frame >= 10 {
                    wall.push(ms(t));
                    let s = env.h.view3d_stats().unwrap();
                    prepare.push(s.last_prepare_us as f64 / 1000.0);
                    sync.push(s.last_sync_us as f64 / 1000.0);
                }
            }
            let note = delta(env.doc(), c0);
            row(name, "rotate_3d", &wall, &format!("1 フレーム全体。{note}"));
            row(name, "rotate_3d_prepare_cpu", &prepare, "prepare の CPU");
            row(name, "rotate_3d_sync_cpu", &sync, "塗った絵の同期の CPU");
        }
    }
    // 6. 見せるチャンネルを Color → Normal
    if part("switch") {
        {
            show(&mut env.h, Channel::Color);
            settle(&mut env.h);
            let (mut cold, mut warm_t, mut note) = (Vec::new(), Vec::new(), String::new());
            for _ in 0..runs {
                show(&mut env.h, Channel::Color);
                settle(&mut env.h);
                env.doc().release_effect_cache();
                // Color を載せ直す（Normal は載せない）
                composite(env.doc(), Channel::Color, &tiles);
                let c0 = counts(env.doc());
                show(&mut env.h, Channel::Normal);
                let (m, f) = settle(&mut env.h);
                cold.push(m);
                note = format!(
                    "フレーム {f}（1 フレームあたり {:.0} ms）。{}",
                    m / f as f64,
                    delta(env.doc(), c0)
                );
                // Color に戻して、また Normal（Normal はキャッシュにある）
                show(&mut env.h, Channel::Color);
                settle(&mut env.h);
                let c1 = counts(env.doc());
                show(&mut env.h, Channel::Normal);
                warm_t.push(settle(&mut env.h).0);
                let _ = c1;
            }
            show(&mut env.h, Channel::Color);
            settle(&mut env.h);
            row(name, "switch_cold_normal", &cold, &note);
            row(
                name,
                "switch_warm_normal",
                &warm_t,
                "Normal がキャッシュにある",
            );
        }
    }
    // 4'・6'. 全面が変わる操作の 1 フレームずつの内訳（設定を 1 つ変える・見せるチャンネルを替える）
    if part("detail") {
        settle(&mut env.h);
        let first = fill_specs.first().copied();
        for (label, k) in [("setting", 0), ("switch", 1), ("back", 2)] {
            match (k, first) {
                (0, Some((layer, filter))) => {
                    change_setting(env.doc(), layer, filter, kind.unwrap(), 77)
                }
                (0, None) => continue,
                (1, _) => show(&mut env.h, Channel::Normal),
                _ => show(&mut env.h, Channel::Color),
            }
            frame_detail(&mut env.h, name, label);
        }
        show(&mut env.h, Channel::Color);
        settle(&mut env.h);
    }
    // 後片づけ
    for id in fills.iter().rev() {
        env.doc().remove_layer(*id).unwrap();
    }
    env.doc().release_effect_cache();
    settle(&mut env.h);
}

/// 変えた後のフレームを 1 つずつ回して、フレームの時間・3D の prepare と塗った絵の同期の CPU・キャンバスの上げたタイルを足し合わせる
/// （落ち着くまで: 30 ms 未満のフレームが 3 つ続くまで）。
fn frame_detail(h: &mut H, scene: &str, label: &str) {
    let (mut wall, mut prepare, mut sync, mut tiles3d, mut canvas, mut rebuilt) =
        (0.0, 0.0, 0.0, 0usize, 0usize, 0usize);
    let (mut frames, mut quiet, mut heavy) = (0, 0, Vec::new());
    while quiet < 3 && frames < 2000 {
        let t = Instant::now();
        h.step();
        let step = ms(t);
        h.view3d_wait_gpu();
        let m = ms(t);
        let st = h.view3d_stats().unwrap();
        let up = h.state().display().stats;
        let (p, sy) = (
            st.last_prepare_us as f64 / 1000.0,
            st.last_sync_us as f64 / 1000.0,
        );
        wall += m;
        prepare += p;
        sync += sy;
        tiles3d += st.last_tiles;
        canvas += up.last_tiles;
        rebuilt += up.last_rebuilt as usize;
        if m >= 30.0 {
            heavy.push(format!(
                "{m:.0}(step {step:.0} 3D prepare {p:.0} sync {sy:.0} 3Dタイル {} canvasタイル {}{})",
                st.last_tiles,
                up.last_tiles,
                if up.last_rebuilt { " 作り直し" } else { "" }
            ));
        }
        frames += 1;
        quiet = if m < 30.0 { quiet + 1 } else { 0 };
    }
    println!(
        "DETAIL\t{scene}\t{label}\t合計 {wall:.0} ms / {frames} フレーム\t3D prepare {prepare:.0} ms（うち塗った絵の同期 {sync:.0} ms、3D のタイル {tiles3d}）\tキャンバスの上げたタイル {canvas}（作り直し {rebuilt} 回）\t重いフレーム {heavy:?}"
    );
}

/// 塗りつぶしの設定を 1 つ変える（毎回別の値）。
fn change_setting(
    doc: &mut Document,
    layer: LayerId,
    filter: Option<FilterId>,
    kind: Kind,
    k: usize,
) {
    match kind {
        Kind::Image { .. } => {
            let mut p = *doc.layer(layer).unwrap().projection();
            p.blend_width = 0.1 + 0.01 * (k % 70) as f64;
            doc.set_fill_projection(layer, p, false).unwrap();
        }
        Kind::Gradient => {
            let id = filter.unwrap();
            let mut g = generator::Settings::new(generator::Kind::PositionGradient);
            g.axis = 0;
            g.blend = generator::Blend::Replace;
            g.low = 0.0;
            g.high = 0.5 + 0.005 * (k % 90) as f64;
            doc.set_filter_settings(layer, id, EffectSettings::generator(g), false)
                .unwrap();
        }
        Kind::PlainMask => {
            doc.set_layer_mask_density(layer, 0.5 + 0.004 * (k % 100) as f64, false)
                .unwrap();
        }
    }
}

#[test]
#[ignore = "計測"]
fn measure() {
    let side: u32 = std::env::var("REFFILL_SIDE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4096);
    let runs: usize = std::env::var("REFFILL_RUNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    let scenes = std::env::var("REFFILL_SCENES").unwrap_or("0,A,B,C,D,E1,E2".into());
    let mut env = setup(side);
    if let Some(mib) = std::env::var("REFFILL_CACHE_MIB")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        // 試し: 設定を読まずに（予算を文書へ入れない状態で）、評価のキャッシュと画像のミップマップの予算を直に入れる
        env.doc().set_filter_cache_budget_bytes(mib << 20);
        env.doc().set_fill_image_cache_budget_bytes(mib << 20);
        println!("CACHE_BUDGET_MIB\t{mib}");
    } else {
        // 製品と同じく設定を読む（既定は自動。`REFFILL_EFFECT_MIB` で効果のメモリを選んだ値に）
        let mut settings = yolu_app::settings::Settings::default();
        if let Some(mib) = std::env::var("REFFILL_EFFECT_MIB")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
        {
            settings.effect_cache_budget = yolu_app::settings::Budget::Mib(mib);
        }
        // REFFILL_NO_DISK_CACHE=1: ディスクキャッシュを切る（既定は製品と同じく入）
        if std::env::var("REFFILL_NO_DISK_CACHE").is_ok() {
            settings.disk_cache = false;
        }
        let s = env.h.app_mut();
        s.load_settings(settings);
        s.sync_budgets();
        println!(
            "EFFECT_BUDGET_MIB\t{}",
            env.doc().filter_cache_budget_bytes() >> 20
        );
    }
    println!(
        "ENV\tside={side}\truns={runs}\tGPU={}\trayon={}",
        env.h.state().view3d_adapter().unwrap_or_default(),
        rayon::current_num_threads()
    );
    for name in scenes.split(',') {
        let (kind, count) = match name {
            "0" => (None, 0),
            "A" => (Some(Kind::Image { uv: false }), 1),
            "B" => (Some(Kind::Image { uv: false }), 8),
            "C" => (Some(Kind::Gradient), 1),
            "D" => (Some(Kind::Gradient), 8),
            "E1" => (Some(Kind::Image { uv: true }), 1),
            "E8" => (Some(Kind::Image { uv: true }), 8),
            "E2" => (Some(Kind::PlainMask), 1),
            other => panic!("知らない場面: {other}"),
        };
        run_scene(&mut env, name, kind, count, runs);
    }
}

/// 段ごとの時間（core の直）: 画像のミップの構築、投影ごとの全面の評価（4096² の Color。異方性あり・なし）。ほかの仕事と同じ rayon のプールで回す。
/// `REFFILL_SIDE=4096 cargo test --release -p yolu-app --test gui_view3d ref_fill_measure::stages -- --ignored --nocapture`
#[test]
#[ignore = "計測"]
fn stages() {
    use yolu_core::fill_image::{
        Conversion, FillInput, FillSampler, ImageMipChain, Map, ModelFrame, Placement, Projection,
    };
    let side: u32 = std::env::var("REFFILL_SIDE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4096);
    let runs: usize = std::env::var("REFFILL_RUNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    let mut env = setup(side);
    println!(
        "ENV\tstages side={side}\truns={runs}\trayon={}",
        rayon::current_num_threads()
    );
    let pixels = pattern(1);
    for (label, conversion, luminance) in [
        ("mip_color", Conversion::None, false),
        ("mip_color_linear_to_srgb", Conversion::LinearToSrgb, false),
        ("mip_roughness_luminance", Conversion::None, true),
    ] {
        let times: Vec<f64> = (0..runs)
            .map(|_| {
                let t = Instant::now();
                let chain = ImageMipChain::build(
                    &pixels,
                    IMAGE_SIDE,
                    IMAGE_SIDE,
                    conversion,
                    luminance,
                    u64::MAX,
                    None,
                )
                .unwrap();
                std::hint::black_box(&chain);
                ms(t)
            })
            .collect();
        row("stages", label, &times, "2048² の画像 1 枚");
    }
    let chain = ImageMipChain::build(
        &pixels,
        IMAGE_SIDE,
        IMAGE_SIDE,
        Conversion::None,
        false,
        u64::MAX,
        None,
    )
    .unwrap();
    let baked = |kind: MeshMapKind| {
        let map = env
            .h
            .state()
            .state
            .sets()
            .current()
            .mesh_maps
            .get(kind)
            .expect("焼いたマップ")
            .clone();
        MapInput::from_baked(&map, MapState::Current).expect("マップ")
    };
    let (position, normal) = (
        baked(MeshMapKind::Position),
        baked(MeshMapKind::WorldNormal),
    );
    // モデルのルートは恒等（試しの立方体）
    let fill_frame = Some(ModelFrame::default());
    fn as_map(m: &MapInput) -> Map<'_> {
        Map {
            width: m.width,
            height: m.height,
            values: &m.data,
            coverage: &m.coverage,
        }
    }
    let placement = Placement {
        center: [0.0; 3],
        rotation: [0.0; 3],
        size: [
            position.bounds_max[0] - position.bounds_min[0],
            position.bounds_max[1] - position.bounds_min[1],
            position.bounds_max[2] - position.bounds_min[2],
        ],
    };
    // 場面 A の塗りつぶしと同じ投影（アプリが置き場をモデルに合わせた物）で、全面・256² のブロックごと（並べ方 2 通り）の時間
    {
        let layers = add_fills(&mut env, Kind::Image { uv: false }, 1);
        let projection = *env.doc().layer(layers[0].0).unwrap().projection();
        println!(
            "PROJECTION\t{:?} blend_width={} tiles={:?} placement={:?}",
            projection.mode, projection.blend_width, projection.tiles, projection.placement
        );
        for aniso in [false, true] {
            let sampler = FillSampler::bind(FillInput {
                projection,
                width: side,
                height: side,
                image: Some(&chain),
                positions: Some(as_map(&position)),
                normals: Some(as_map(&normal)),
                bounds_min: position.bounds_min,
                bounds_max: position.bounds_max,
                frame: fill_frame,
                anisotropic: aniso,
                ..Default::default()
            })
            .unwrap();
            assert!(sampler.reason().is_none(), "{:?}", sampler.reason());
            let n = side / 256;
            let blocks: Vec<(u32, u32)> = (0..n)
                .flat_map(|by| (0..n).map(move |bx| (bx * 256, by * 256)))
                .collect();
            let whole: Vec<f64> = (0..runs)
                .map(|_| {
                    let t = Instant::now();
                    std::hint::black_box(sampler.render(0, 0, side, side, u64::MAX, None).unwrap());
                    ms(t)
                })
                .collect();
            let par: Vec<f64> = (0..runs)
                .map(|_| {
                    use rayon::prelude::*;
                    let t = Instant::now();
                    let out: Vec<Vec<u8>> = blocks
                        .par_iter()
                        .map(|&(x, y)| sampler.render(x, y, 256, 256, u64::MAX, None).unwrap())
                        .collect();
                    std::hint::black_box(out);
                    ms(t)
                })
                .collect();
            let seq: Vec<f64> = (0..runs)
                .map(|_| {
                    let t = Instant::now();
                    for &(x, y) in &blocks {
                        std::hint::black_box(
                            sampler.render(x, y, 256, 256, u64::MAX, None).unwrap(),
                        );
                    }
                    ms(t)
                })
                .collect();
            // 文書の評価と同じ道（`filter::evaluate` を読み元 `Source` の 1 画素ずつの読みで、ブロックごとに回す）。
            // 文書は `parallel_blocks` 個ずつのバッチで待ち合わせる。ここでは同じ 8 個のバッチと、全ブロックを一度に並べる場合を測る
            if aniso {
                use rayon::prelude::*;
                use yolu_core::filter::{self, Options, ValueType};
                struct SamplerSource<'a>(&'a FillSampler<'a>, u32);
                impl filter::Source for SamplerSource<'_> {
                    fn dimensions(&self) -> (u32, u32) {
                        (self.1, self.1)
                    }
                    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
                        self.0.pixel(x, y).unwrap_or(Rgba8::TRANSPARENT).to_array()
                    }
                }
                let source = SamplerSource(&sampler, side);
                let one = |&(x, y): &(u32, u32)| {
                    filter::evaluate(
                        &source,
                        ValueType::Color,
                        &[],
                        yolu_core::Rect::new(x, y, 256, 256),
                        &Options::default(),
                    )
                    .unwrap()
                };
                let threads = rayon::current_num_threads();
                let batched: Vec<f64> = (0..runs)
                    .map(|_| {
                        let t = Instant::now();
                        for batch in blocks.chunks(threads) {
                            let out: Vec<Vec<u8>> = batch.par_iter().map(one).collect();
                            std::hint::black_box(out);
                        }
                        ms(t)
                    })
                    .collect();
                let all: Vec<f64> = (0..runs)
                    .map(|_| {
                        let t = Instant::now();
                        let out: Vec<Vec<u8>> = blocks.par_iter().map(one).collect();
                        std::hint::black_box(out);
                        ms(t)
                    })
                    .collect();
                row(
                    "stages",
                    "A_filter_evaluate_batched",
                    &batched,
                    "filter::evaluate をスレッド数ずつのバッチで（文書の評価と同じ並べ方）",
                );
                row(
                    "stages",
                    "A_filter_evaluate_all_parallel",
                    &all,
                    "filter::evaluate を全ブロック一度に並べる",
                );
            }
            let tag = if aniso { "aniso" } else { "iso" };
            row(
                "stages",
                &format!("A_projection_{tag}_whole"),
                &whole,
                "場面 A の投影・全面を 1 回",
            );
            row(
                "stages",
                &format!("A_projection_{tag}_blocks_parallel"),
                &par,
                "256² のブロックを rayon で並べる",
            );
            row(
                "stages",
                &format!("A_projection_{tag}_blocks_sequential"),
                &seq,
                "256² のブロックを 1 つずつ（中は行ごとに並列）",
            );
        }
        env.doc().remove_layer(layers[0].0).unwrap();
    }
    for (label, mode, aniso) in [
        ("render_uv_iso", ProjectionMode::Uv, false),
        ("render_uv_aniso", ProjectionMode::Uv, true),
        ("render_triplanar_iso", ProjectionMode::Triplanar, false),
        ("render_triplanar_aniso", ProjectionMode::Triplanar, true),
        ("render_planar_iso", ProjectionMode::Planar, false),
    ] {
        let sampler = FillSampler::bind(FillInput {
            projection: Projection {
                mode,
                placement,
                ..Default::default()
            },
            width: side,
            height: side,
            image: Some(&chain),
            positions: Some(as_map(&position)),
            normals: Some(as_map(&normal)),
            bounds_min: position.bounds_min,
            bounds_max: position.bounds_max,
            frame: fill_frame,
            anisotropic: aniso,
            ..Default::default()
        })
        .unwrap();
        assert!(
            sampler.reason().is_none(),
            "{label}: {:?}",
            sampler.reason()
        );
        let times: Vec<f64> = (0..runs)
            .map(|_| {
                let t = Instant::now();
                let out = sampler.render(0, 0, side, side, u64::MAX, None).unwrap();
                std::hint::black_box(&out);
                ms(t)
            })
            .collect();
        row("stages", label, &times, &format!("{side}² 全面"));
    }
}

/// 内訳を取る専用（perf で見る）: 場面を置いて、キャッシュ空からの全面の合成を `REFFILL_LOOPS` 回、Color だけ回す。
#[test]
#[ignore = "計測"]
fn profile_cold() {
    let side: u32 = std::env::var("REFFILL_SIDE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4096);
    let loops: usize = std::env::var("REFFILL_LOOPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6);
    let scene = std::env::var("REFFILL_SCENES").unwrap_or("A".into());
    let mut env = setup(side);
    let (kind, count) = match scene.as_str() {
        "A" => (Kind::Image { uv: false }, 1),
        "B" => (Kind::Image { uv: false }, 8),
        "C" => (Kind::Gradient, 1),
        "D" => (Kind::Gradient, 8),
        "E1" => (Kind::Image { uv: true }, 1),
        other => panic!("知らない場面: {other}"),
    };
    add_fills(&mut env, kind, count);
    let tiles = all_tiles(env.doc());
    let mut times = Vec::new();
    for _ in 0..loops {
        env.doc().release_effect_cache();
        times.push(composite(env.doc(), Channel::Color, &tiles));
    }
    row(&scene, "profile_cold_color", &times, "");
}
