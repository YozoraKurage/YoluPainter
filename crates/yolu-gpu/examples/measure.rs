//! 4096²、4 層の全面と 1 タイル。転送・読み戻し込みの時間を同じ矩形の CPU と比べる。
use std::time::Instant;
use yolu_core::{BlendMode, BrushSettings, Channel, Document, TileCoord};
use yolu_gpu::{GpuPainter, Options};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut gpu = GpuPainter::new(Options::default())?;
    println!("GPU: {:?}", gpu.adapter_info());
    let mut d = Document::new(4096, 4096)?;
    let mut first = None;
    let mut seed = 1234567u32;
    for k in 0..4 {
        let l = d.add_layer("計測")?;
        first.get_or_insert(l);
        let mut tile = vec![0u8; 128 * 128 * 4];
        for px in tile.as_chunks_mut::<4>().0 {
            for v in px.iter_mut() {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                *v = seed as u8;
            }
            px[3] = if k == 0 { 255 } else { 128 };
        }
        for y in 0..32 {
            for x in 0..32 {
                d.import_tile(l, Channel::Color, TileCoord::new(x, y), &tile)?;
            }
        }
        d.set_layer_blend_mode(
            l,
            [
                BlendMode::Normal,
                BlendMode::Multiply,
                BlendMode::Screen,
                BlendMode::Normal,
            ][k],
        )?;
    }
    let all: Vec<_> = (0..32)
        .flat_map(|y| (0..32).map(move |x| TileCoord::new(x, y)))
        .collect();
    let serial = d.change_serial();
    let mut stroke = d.begin_stroke(
        first.unwrap(),
        &BrushSettings {
            radius: 4.0,
            ..Default::default()
        },
    )?;
    stroke.add_point(&mut d, 64.0, 64.0, 1.0, yolu_core::glam::DVec2::ZERO)?;
    d.end_stroke(stroke)?;
    let changed = d.changed_tiles(Channel::Color, serial).unwrap();
    assert_eq!(changed.len(), 1);
    for (name, coords) in [("全面", &all), ("変更分", &changed)] {
        let _ = gpu.composite_tiles(&d, Channel::Color, coords)?;
        let _ = d.composite(if name == "全面" {
            d.bounds()
        } else {
            d.tile_rect(coords[0]).unwrap()
        })?;
        let mut cpu = Vec::new();
        let mut gpu_times = Vec::new();
        let mut maximum = 0;
        for _ in 0..3 {
            let now = Instant::now();
            let expected = d.composite(if name == "全面" {
                d.bounds()
            } else {
                d.tile_rect(coords[0]).unwrap()
            })?;
            cpu.push(now.elapsed().as_secs_f64() * 1000.0);
            let now = Instant::now();
            let result = gpu.composite_tiles(&d, Channel::Color, coords)?;
            gpu_times.push(now.elapsed().as_secs_f64() * 1000.0);
            for tile in result.tiles {
                for y in 0..tile.rect.height as usize {
                    let at = if name == "全面" {
                        ((tile.rect.y as usize + y) * 4096 + tile.rect.x as usize) * 4
                    } else {
                        y * tile.rect.width as usize * 4
                    };
                    let row = &tile.pixels
                        [y * tile.rect.width as usize * 4..(y + 1) * tile.rect.width as usize * 4];
                    for (a, b) in expected[at..at + row.len()].iter().zip(row) {
                        maximum = maximum.max(a.abs_diff(*b));
                    }
                }
            }
        }
        cpu.sort_by(f64::total_cmp);
        gpu_times.sort_by(f64::total_cmp);
        println!("{name}: {} タイル、CPU ms={cpu:?}、GPU ms={gpu_times:?}、中央値比 GPU/CPU={:.3}、最大差={maximum}",coords.len(),gpu_times[1]/cpu[1]);
    }
    Ok(())
}
