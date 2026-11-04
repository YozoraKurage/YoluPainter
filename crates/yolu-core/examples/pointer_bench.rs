//! 参照レイヤーの速さ。4096² の文書で、全面に画素のある下のラスターレイヤーを、参照レイヤー 4 段の連なり（1 段目が下のレイヤー、2〜4 段目が
//! 1 つ下の参照レイヤーを指す）が読む。最初の合成と、一番下（指した先）に 1 画素描いた後の合成と、関係ないレイヤーに 1 画素描いた後の合成を測る。
//! `cargo run --release -p yolu-core --example pointer_bench`（スレッドの数は `EFFECT_THREADS`、既定 4）
use std::time::Instant;
use yolu_core::{Channel, Document, PointerTarget, Rect, Rgba8, TileCoord};

const N: u32 = 4096;
const TS: u32 = 256;

fn main() {
    let threads: usize = std::env::var("EFFECT_THREADS")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(4);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap();
    println!("スレッド {threads}");
    pool.install(run);
}

fn time<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let v = f();
    println!("{label}: {:.1} ms", start.elapsed().as_secs_f64() * 1000.0);
    v
}

fn run() {
    let mut doc = Document::with_tile_size(N, N, TS).unwrap();
    doc.set_source_budget_bytes(1 << 30).unwrap();
    let base = doc.add_layer("下").unwrap();
    let mut tile = vec![0u8; (TS * TS * 4) as usize];
    for ty in 0..N / TS {
        for tx in 0..N / TS {
            for (i, p) in tile.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let (x, y) = (i as u32 % TS + tx * TS, i as u32 / TS + ty * TS);
                p.copy_from_slice(&[(x % 251) as u8, (y % 241) as u8, ((x ^ y) % 239) as u8, 255]);
            }
            doc.import_tile(base, Channel::Color, TileCoord::new(tx, ty), &tile)
                .unwrap();
        }
    }
    let other = doc.add_layer("関係ない").unwrap();
    doc.set_pixel(other, 10, 10, Rgba8::new(1, 2, 3, 255))
        .unwrap();
    let mut last = base;
    for k in 0..4 {
        last = doc
            .add_pointer_layer(&format!("参照 {k}"), PointerTarget::Layer(last), None)
            .unwrap();
    }
    assert_eq!(doc.pointer_depth(last).unwrap(), Some(4));
    let all = Rect::new(0, 0, N, N);
    let blocks = |d: &Document| d.effect_counters().blocks_evaluated;
    time("最初の合成", || {
        doc.composite_channel(Channel::Color, all).unwrap()
    });
    // 描いている間の表示は、変わったタイル（`changed_tiles`）だけを合成し直す
    for round in 0..3 {
        let serial = doc.change_serial();
        let before = blocks(&doc);
        doc.set_pixel(base, 600 + round, 700, Rgba8::new(0, 0, 250, 255))
            .unwrap();
        let tiles = doc.changed_tiles(Channel::Color, serial).unwrap();
        time(
            &format!(
                "指した先に 1 画素描いた後の、変わったタイル {} 枚だけの合成 {round}",
                tiles.len()
            ),
            || {
                for t in &tiles {
                    let r = Rect::new(t.x * TS, t.y * TS, TS, TS);
                    doc.composite_channel(Channel::Color, r).unwrap();
                }
            },
        );
        println!("  評価したブロック {}", blocks(&doc) - before);
        // 次の回のために全体を合成し直しておく
        doc.composite_channel(Channel::Color, all).unwrap();
    }
    // 描いている間は、関係ないレイヤーのタイル 1 枚の変化でも、参照レイヤーの解析（変化の時計が鍵に入る）をやり直す。そのタイルだけの合成の
    // 時間を、描く前（解析はキャッシュ済み）と後で比べる（`FILLERS` はレイヤーの数を増やす空のレイヤーの数）
    let fillers: usize = std::env::var("FILLERS")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(0);
    for k in 0..fillers {
        doc.add_layer(&format!("空 {k}")).unwrap();
    }
    let tile_rect = Rect::new(0, 0, TS, TS);
    for round in 0..3 {
        doc.composite_channel(Channel::Color, tile_rect).unwrap();
        time(
            &format!(
                "描かずにタイル 1 枚だけの合成（レイヤー {} 枚） {round}",
                doc.layers().len()
            ),
            || doc.composite_channel(Channel::Color, tile_rect).unwrap(),
        );
        doc.set_pixel(other, 300 + round, 400, Rgba8::new(9, 9, 9, 255))
            .unwrap();
        time(
            &format!("関係ないレイヤーに 1 画素描いた後の、タイル 1 枚だけの合成 {round}"),
            || doc.composite_channel(Channel::Color, tile_rect).unwrap(),
        );
    }
    for round in 0..3 {
        let before = blocks(&doc);
        doc.set_pixel(base, 100 + round, 200, Rgba8::new(250, 0, 0, 255))
            .unwrap();
        time(
            &format!("指した先に 1 画素描いた後の合成 {round}"),
            || doc.composite_channel(Channel::Color, all).unwrap(),
        );
        println!("  評価したブロック {}", blocks(&doc) - before);
        let before = blocks(&doc);
        doc.set_pixel(other, 300 + round, 400, Rgba8::new(0, 250, 0, 255))
            .unwrap();
        time(
            &format!("関係ないレイヤーに 1 画素描いた後の合成 {round}"),
            || doc.composite_channel(Channel::Color, all).unwrap(),
        );
        println!("  評価したブロック {}", blocks(&doc) - before);
    }
}
