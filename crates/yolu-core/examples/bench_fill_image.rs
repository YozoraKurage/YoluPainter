//! 4096² の投影。入力・ミップの準備を除外、RGBA8 出力の確保を含む。予熱 1 回。
#[path = "../tests/support/fill_cases.rs"]
mod cases;
use cases::*;
use yolu_core::fill_image::*;
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let runs = args.get(1).map_or(3, |s| s.parse::<usize>().unwrap());
    let threads = args.get(2).map_or(4, |s| s.parse::<usize>().unwrap());
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap();
    pool.install(|| {
        let fixture = Fixture::new(4096, 4096);
        let d = picture(31, 23, false);
        let sd = picture(13, 17, true);
        let chain =
            ImageMipChain::build(&d, 31, 23, Conversion::None, false, u64::MAX, None).unwrap();
        let shape =
            ImageMipChain::build(&sd, 13, 17, Conversion::None, false, u64::MAX, None).unwrap();
        for mode in 0..6 {
            let s = fixture.sampler(mode, 1, &chain, &shape);
            std::hint::black_box(s.render(0, 0, 4096, 4096, u64::MAX, None).unwrap());
            for r in 0..runs {
                let start = std::time::Instant::now();
                let out = s.render(0, 0, 4096, 4096, u64::MAX, None).unwrap();
                let ms = start.elapsed().as_secs_f64() * 1000.;
                println!(
                    "{:?},{r},{ms:.3},{}",
                    ProjectionMode::try_from(mode).unwrap(),
                    out[out.len() / 2]
                );
                std::hint::black_box(out);
            }
        }
    });
}
