//! 人工画像でのマテリアル塗りの計測。読み込み・Undo の時間を含めない。
use std::time::Instant;
use yolu_core::{glam::DVec2, material::ChannelPaint, *};
fn main() {
    for degree in [1, 4] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(degree)
            .build()
            .unwrap()
            .install(|| {
                for mode in ["塗りつぶし", "三角形"] {
                    let mut times = Vec::new();
                    let mut allocated = 0;
                    for _ in 0..4 {
                        let mut d = Document::with_tile_size(1024, 1024, 128).unwrap();
                        let l = d.add_layer("人工画像").unwrap();
                        let m: Vec<_> = Channel::ALL
                            .iter()
                            .map(|c| ChannelPaint::new(*c, Rgba8::new(61, 137, 213, 211)))
                            .collect();
                        let start = Instant::now();
                        if mode == "塗りつぶし" {
                            d.fill_material(l, &m, 0.63, None, false).unwrap();
                        } else {
                            let mut f = d.begin_material_triangle_fill(l, &m, 0.63, false).unwrap();
                            f.add(
                                &mut d,
                                &[
                                    [
                                        DVec2::ZERO,
                                        DVec2::new(1024.0, 0.0),
                                        DVec2::new(1024.0, 1024.0),
                                    ],
                                    [
                                        DVec2::ZERO,
                                        DVec2::new(1024.0, 1024.0),
                                        DVec2::new(0.0, 1024.0),
                                    ],
                                ],
                            )
                            .unwrap();
                            f.commit(&mut d).unwrap();
                        }
                        times.push(start.elapsed().as_secs_f64() * 1000.0);
                        allocated = d.allocated_bytes();
                    }
                    println!(
                        "{degree} スレッド {mode} 1024²×6 初回除外平均 {:.3} ms、正本 {} bytes",
                        times[1..].iter().sum::<f64>() / 3.0,
                        allocated
                    );
                }
            });
    }
}
