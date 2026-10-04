//! C# と同じ合成入力の 4096²。入力作成・プール構築を計測から除き、束縛・返却画像の確保を含む。
#[path = "../tests/generator_support/mod.rs"]
mod support;
use sha2::{Digest, Sha256};
use std::time::Instant;
use yolu_core::{generator::*, Rect};
fn main() {
    let threads = std::env::var("GEN_THREADS")
        .unwrap_or("4".into())
        .parse()
        .unwrap();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap();
    pool.install(|| {
        for (kind, v, name) in support::KINDS
            .into_iter()
            .map(|k| {
                (
                    k,
                    if k == Kind::ShapeGradient { 5 } else { 2 },
                    format!("{k:?}"),
                )
            })
            .chain([
                (Kind::ShapeGradient, 3, "ShapeBoxRamp".into()),
                (Kind::ShapeGradient, 4, "ShapeSphereRamp".into()),
            ])
        {
            let (w, h) = (4096, 4096);
            let s = support::settings(kind, v);
            let owned = support::Maps::new(&s, w, h);
            let maps = owned.maps();
            let input = support::pixels(w, h, Target::Color);
            let image = Image::new(&input, w, h).unwrap();
            let anchor = anchor::LayerSample {
                source: &image,
                read: anchor::Read::Coverage,
            };
            let run = || {
                let b =
                    BoundGenerator::bind(&s, &maps, Some(support::frame()), (w, h), Ok(&anchor))
                        .unwrap();
                evaluate(
                    &image,
                    &b,
                    Rect::new(0, 0, w, h),
                    Target::Color,
                    if v.is_multiple_of(2) { 1. } else { 0.43 },
                    &Options::default(),
                )
                .unwrap()
                .pixels
            };
            drop(run());
            let now = Instant::now();
            let output = run();
            let elapsed = now.elapsed().as_secs_f64() * 1000.;
            println!("{name} {elapsed:.3} {:x}", Sha256::digest(output));
        }
        let r = support::ramp();
        let run = || {
            use rayon::prelude::*;
            let mut output = vec![0; 4096 * 4096 * 4];
            output
                .par_chunks_mut(4096 * 4)
                .enumerate()
                .for_each(|(y, row)| {
                    for (x, p) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                        p.copy_from_slice(
                            &r.evaluate(((x + y * 17) % 65536) as f64 / 65535., false)
                                .unwrap()
                                .to_array(),
                        );
                    }
                });
            output
        };
        drop(run());
        let now = Instant::now();
        let output = run();
        let elapsed = now.elapsed().as_secs_f64() * 1000.;
        println!("Ramp {:.3} {:x}", elapsed, Sha256::digest(output));
    });
}
