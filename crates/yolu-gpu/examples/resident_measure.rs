//! Windows でも同じ計測: cargo run --release -p yolu-gpu --example resident_measure -- 4096 3
//! 引数はキャンバスの一辺と反復数。CPU は合成のみ、GPU は転送・完了待ちを含む表示更新。
use std::time::Instant;
use yolu_core::{BlendMode, Channel, Document, TileCoord};
use yolu_gpu::{ResidentCompositor, ResidentOptions};
fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let size: u32 = args.get(1).map_or(Ok(4096), |s| s.parse())?;
    let repeats: usize = args.get(2).map_or(Ok(3), |s| s.parse())?;
    if size < 128 || !size.is_multiple_of(128) || repeats == 0 {
        return Err("キャンバスは128の倍数、反復は1以上".into());
    }
    println!("キャンバス={size}²、反復={repeats}、release={}、CPUは合成のみ（表示転送なし）、GPUは完了待ち込み",!cfg!(debug_assertions));
    for layer_count in [4, 16] {
        let mut g = ResidentCompositor::new(ResidentOptions {
            resident_budget_bytes: 3u64 << 30,
            readback_budget_bytes: 256 << 20,
            batch_tiles: 16,
            ..Default::default()
        })?;
        println!("{layer_count}レイヤー GPU: {:?}", g.adapter_info());
        if g.adapter_info().device_type == wgpu::DeviceType::Cpu {
            println!("ソフトウェアGPUの数値。実GPUやWindowsの結果ではない。");
        }
        let mut d = Document::new(size, size)?;
        d.set_source_budget_bytes(2u64 << 30)?;
        let mut ids = Vec::new();
        let mut patterns = Vec::new();
        let mut seed = 0xabcdefu32;
        for k in 0..layer_count {
            let id = d.add_layer("計測")?;
            ids.push(id);
            let mut tile = vec![0; 128 * 128 * 4];
            for px in tile.as_chunks_mut::<4>().0 {
                for v in &mut px[..3] {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    *v = seed as u8;
                }
                px[3] = if k == 0 { 255 } else { 128 };
            }
            for y in 0..size / 128 {
                for x in 0..size / 128 {
                    d.import_tile(id, Channel::Color, TileCoord::new(x, y), &tile)?;
                }
            }
            d.set_layer_blend_mode(
                id,
                [
                    BlendMode::Normal,
                    BlendMode::Multiply,
                    BlendMode::Screen,
                    BlendMode::Normal,
                ][k % 4],
            )?;
            patterns.push(tile);
        }
        let warm = g.update(&d, Channel::Color)?;
        println!("初回: {warm:?}");
        let _ = d.composite(d.bounds())?;
        for full in [false, true] {
            for readback in [false, true] {
                let mut cpu = Vec::new();
                let mut gpu = Vec::new();
                let mut worst = 0u8;
                let mut uploaded = 0;
                for iteration in 0..=repeats {
                    // 更新の入力を作る時間は双方の計測外。全面は全レイヤーの全タイル、局所は 1 レイヤーの1タイルを実際に変更。
                    if full {
                        for (id, tile) in ids.iter().zip(&mut patterns) {
                            for px in tile.as_chunks_mut::<4>().0 {
                                px[0] = px[0].wrapping_add(1);
                            }
                            for y in 0..size / 128 {
                                for x in 0..size / 128 {
                                    d.import_tile(*id, Channel::Color, TileCoord::new(x, y), tile)?;
                                }
                            }
                        }
                    } else {
                        patterns[0][0] = patterns[0][0].wrapping_add(1);
                        d.import_tile(ids[0], Channel::Color, TileCoord::new(0, 0), &patterns[0])?;
                    }
                    let rect = if full {
                        d.bounds()
                    } else {
                        d.tile_rect(TileCoord::new(0, 0)).unwrap()
                    };
                    let t = Instant::now();
                    let expected = d.composite(rect)?;
                    let cpu_ms = t.elapsed().as_secs_f64() * 1000.0;
                    let t = Instant::now();
                    let stats = g.update(&d, Channel::Color)?;
                    let actual = if readback {
                        let r = g.request_readback(rect)?;
                        Some(g.finish_readback(r)?)
                    } else {
                        None
                    };
                    let gpu_ms = t.elapsed().as_secs_f64() * 1000.0;
                    assert_eq!(
                        stats.updated_tiles,
                        if full {
                            (size / 128).pow(2) as usize
                        } else {
                            1
                        }
                    );
                    assert_eq!(
                        stats.uploaded_tiles,
                        if full {
                            (size / 128).pow(2) as usize * layer_count
                        } else {
                            1
                        }
                    );
                    assert_eq!(stats.evicted_tiles, 0);
                    uploaded = stats.uploaded_bytes;
                    // 読み戻し無しの経路も計測外で結果を照合。
                    let actual = match actual {
                        Some(a) => a,
                        None => {
                            let r = g.request_readback(rect)?;
                            g.finish_readback(r)?
                        }
                    };
                    assert_eq!(actual.len(), expected.len());
                    worst = worst.max(
                        actual
                            .iter()
                            .zip(&expected)
                            .map(|(a, b)| a.abs_diff(*b))
                            .max()
                            .unwrap_or(0),
                    );
                    if iteration > 0 {
                        cpu.push(cpu_ms);
                        gpu.push(gpu_ms);
                    }
                }
                let cm = median(&mut cpu);
                let gm = median(&mut gpu);
                println!("{layer_count}レイヤー {} 読み戻し={readback}: CPU中央値={cm:.3}ms GPU中央値={gm:.3}ms 比={:.3} 転送={uploaded}B 最大差={worst} CPU={cpu:?} GPU={gpu:?}",if full{"全レイヤー全面変更"}else{"1 レイヤー1タイル変更"},gm/cm);
            }
        }
    }
    Ok(())
}
