//! 塗りつぶしのベイクしたマップの源の速さ。4096² の文書で、下のラスターレイヤーの上に、AO のマップを源にした塗りつぶし 1 枚を置き、
//! 最初の合成と、関係ないレイヤー（下のラスター）に 1 画素描いた後の合成を測る。UV の既定の置き方（マップを直に読む）と、トライプラナー
//! （マップのミップマップを作って投影する）と、比べるための同じ画素の画像の源。
//! `cargo run --release -p yolu-core --example fill_map_bench`（スレッドの数は `EFFECT_THREADS`、既定 4）
use std::time::Instant;
use yolu_core::fill_image::{Projection, ProjectionMode};
use yolu_core::generator::{MapKind, MapState};
use yolu_core::{
    Channel, Document, EffectInputs, ImageColorSpace, ImageId, ImageInput, MapInput, ModelFrame,
    Rect, Rgba8,
};

const N: u32 = 4096;
const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const IMAGE: ImageId = ImageId(0x1234_5678_9abc_4def_8123_4567_89ab_cdef);

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

fn map(kind: MapKind, f: impl Fn(u32, u32) -> [u16; 3]) -> MapInput {
    let n = (N * N) as usize;
    let mut data = Vec::with_capacity(n * kind.channels());
    for y in 0..N {
        for x in 0..N {
            data.extend_from_slice(&f(x, y)[..kind.channels()]);
        }
    }
    MapInput::new(
        kind,
        N,
        N,
        data,
        vec![1; n],
        [0.0; 3],
        [1.0; 3],
        KEY,
        MapState::Current,
    )
    .unwrap()
}

fn inputs() -> EffectInputs {
    let ao = map(MapKind::AmbientOcclusion, |x, y| {
        [((x * 31 + y * 17) % 65536) as u16, 0, 0]
    });
    let image: Vec<u8> = ao
        .data
        .iter()
        .flat_map(|v| {
            let b = ((u32::from(*v) + 128) / 257) as u8;
            [b, b, b, 255]
        })
        .collect();
    let scale = |v: u32| ((f64::from(v) + 0.5) / f64::from(N) * 65535.0).round() as u16;
    EffectInputs::new()
        .with_map(ao)
        .unwrap()
        .with_map(map(MapKind::Position, |x, y| [scale(x), scale(y), 32768]))
        .unwrap()
        .with_map(map(MapKind::WorldNormal, |_, _| [32768, 32768, 65535]))
        .unwrap()
        .with_frame(Some(
            ModelFrame::new([0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap(),
        ))
        .with_image(
            IMAGE,
            ImageInput::new(N, N, image, ImageColorSpace::Unspecified).unwrap(),
        )
}

fn run() {
    let inputs = inputs();
    let rect = Rect::new(0, 0, N, N);
    for (label, mode, by_map) in [
        ("マップ・UV", ProjectionMode::Uv, true),
        ("画像・UV", ProjectionMode::Uv, false),
        ("マップ・トライプラナー", ProjectionMode::Triplanar, true),
        ("画像・トライプラナー", ProjectionMode::Triplanar, false),
    ] {
        let mut doc = Document::new(N, N).unwrap();
        doc.set_source_budget_bytes(1 << 30).unwrap();
        doc.set_filter_cache_budget_bytes(1 << 30);
        let base = doc.add_layer("下").unwrap();
        doc.set_pixel(base, 0, 0, Rgba8::new(1, 2, 3, 255)).unwrap();
        let fill = doc
            .add_fill_layer(
                "塗り",
                &[(Channel::Color, Rgba8::new(90, 90, 90, 255))],
                None,
            )
            .unwrap();
        doc.set_effect_inputs(inputs.clone()).unwrap();
        doc.set_fill_projection(
            fill,
            Projection {
                mode,
                ..Projection::default()
            },
            false,
        )
        .unwrap();
        if by_map {
            doc.set_fill_map(fill, Channel::Color, Some(MapKind::AmbientOcclusion))
                .unwrap();
        } else {
            doc.set_fill_image(fill, Channel::Color, Some(IMAGE))
                .unwrap();
        }
        let first = time(&format!("{label}: 最初の合成"), || {
            doc.composite_channel(Channel::Color, rect).unwrap()
        });
        let c = doc.effect_counters();
        println!(
            "{label}: ミップマップを作った数 {}・画像の予算に数えたバイト数 {}",
            c.mip_chains_built, c.image_cache_bytes
        );
        doc.set_pixel(base, 100, 100, Rgba8::new(200, 0, 0, 255))
            .unwrap();
        let again = time(
            &format!("{label}: 下のレイヤーに描いた後の合成"),
            || doc.composite_channel(Channel::Color, rect).unwrap(),
        );
        let tile = doc.tile_size();
        time(
            &format!("{label}: 描いたタイルだけの合成"),
            || {
                doc.composite_channel(Channel::Color, Rect::new(0, 0, tile, tile))
                    .unwrap()
            },
        );
        assert_eq!(first.len(), again.len());
        std::hint::black_box((first, again));
    }
}
