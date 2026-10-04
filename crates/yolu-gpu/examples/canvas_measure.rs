//! キャンバスの表示の更新の計測（アプリの CPU の表示と GPU の常駐の表示を、同じ装置・同じ変更で比べる）。
//!   cargo run --release -p yolu-gpu --example canvas_measure -- 4096 5
//! 引数は画布の一辺（128 の倍数）と反復数（既定 4096・5）。環境変数 `LAYERS`（既定 4,16,32）で層の数を選ぶ。
//!
//! CPU の道はアプリの `CanvasDisplay` と同じ手順: `changed_tiles` で変わったタイルを知り、タイルごとに `composite_into`
//! （下から上の行）→ 乗算済みへ変換 → 表示のテクスチャのそのタイルの範囲へ `write_texture`（egui-wgpu が `set_partial` で行う
//! 転送）→ 提出して完了を待つ。GPU の道は `ResidentCompositor::update`（乗算済みの表示・予算 512 MiB・提出と完了待ち込み）。
//! どちらも変更の作成（文書への `import_tile` など）は計測の外。表示の画素の一致（許し 2）も毎回確かめる。
//! 出力されるアダプター名・バックエンドを併せて記録する。ソフトウェアの GPU（llvmpipe）と実 GPU を混同しないこと。
use std::time::Instant;
use yolu_core::{BlendMode, Channel, Document, Rect, RowOrder, TileCoord};
use yolu_gpu::{resident_requirements, GpuPainter, Options, ResidentCompositor, ResidentOptions};

const TS: u32 = 128;

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

/// 乗算済み（egui の Color32::from_rgba_unmultiplied と同じ式）。
fn premultiply(pixels: &mut [u8]) {
    for p in pixels.as_chunks_mut::<4>().0 {
        match p[3] {
            0 => *p = [0; 4],
            255 => {}
            a => {
                for c in &mut p[..3] {
                    let q = u16::from(*c) * u16::from(a) + 128;
                    *c = ((q + (q >> 8)) >> 8) as u8;
                }
            }
        }
    }
}

struct Cpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    texture: wgpu::Texture,
    serial: u64,
    buffer: Vec<u8>,
}

impl Cpu {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, size: u32) -> Cpu {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cpu 表示"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Cpu {
            device: device.clone(),
            queue: queue.clone(),
            texture,
            serial: 0,
            buffer: Vec::new(),
        }
    }

    /// `changed_tiles` のタイルを合成して表示のテクスチャへ上げる（初めは全部）。上げたタイルの数。
    fn update(&mut self, doc: &Document, first: bool) -> usize {
        let coords: Vec<TileCoord> = if first {
            doc.canvas_tiles().collect()
        } else {
            doc.changed_tiles(Channel::Color, self.serial).unwrap()
        };
        self.serial = doc.change_serial();
        for coord in &coords {
            let r = doc.tile_rect(*coord).unwrap();
            self.buffer.resize((r.width * r.height * 4) as usize, 0);
            doc.composite_into(Channel::Color, r, &mut self.buffer, RowOrder::BottomUp)
                .unwrap();
            premultiply(&mut self.buffer);
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: r.x,
                        y: r.y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &self.buffer,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(r.width * 4),
                    rows_per_image: Some(r.height),
                },
                wgpu::Extent3d {
                    width: r.width,
                    height: r.height,
                    depth_or_array_layers: 1,
                },
            );
        }
        let submission = self.queue.submit(std::iter::empty::<wgpu::CommandBuffer>());
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(60)),
            })
            .unwrap();
        coords.len()
    }
}

fn tile_pattern(seed: &mut u32, alpha: u8) -> Vec<u8> {
    let mut tile = vec![0; (TS * TS * 4) as usize];
    for px in tile.as_chunks_mut::<4>().0 {
        for v in &mut px[..3] {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 17;
            *seed ^= *seed << 5;
            *v = *seed as u8;
        }
        px[3] = alpha;
    }
    tile
}

/// 文書: 先頭の層は全面で不透明、ほかは dense なら全面・sparse なら 4 タイルだけ（実際の絵の疎さに近い）。
fn build(size: u32, layers: usize, dense: bool) -> (Document, Vec<yolu_core::LayerId>) {
    let mut d = Document::with_tile_size(size, size, TS).unwrap();
    d.set_source_budget_bytes(4 << 30).unwrap();
    let mut seed = 0xabcdefu32;
    let mut ids = Vec::new();
    let n = size / TS;
    for k in 0..layers {
        let id = d.add_layer("計測").unwrap();
        let tile = tile_pattern(&mut seed, if k == 0 { 255 } else { 128 });
        if k == 0 || dense {
            for y in 0..n {
                for x in 0..n {
                    d.import_tile(id, Channel::Color, TileCoord::new(x, y), &tile)
                        .unwrap();
                }
            }
        } else {
            for (x, y) in [(0, 0), (1, 0), (n / 2, n / 2), (n - 1, n - 1)] {
                d.import_tile(id, Channel::Color, TileCoord::new(x, y), &tile)
                    .unwrap();
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
        )
        .unwrap();
        ids.push(id);
    }
    (d, ids)
}

fn max_diff(a: &[u8], b: &[u8]) -> u8 {
    a.iter()
        .zip(b)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let size: u32 = args.get(1).map_or(Ok(4096), |s| s.parse())?;
    let repeats: usize = args.get(2).map_or(Ok(5), |s| s.parse())?;
    if size < TS || !size.is_multiple_of(TS) || repeats == 0 {
        return Err("画布は 128 の倍数、反復は 1 以上".into());
    }
    let layer_counts: Vec<usize> = std::env::var("LAYERS")
        .unwrap_or_else(|_| "4,16,32".into())
        .split(',')
        .map(|s| s.trim().parse())
        .collect::<Result<_, _>>()?;
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
    let info = adapter.get_info();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
    println!(
        "画布={size}²、タイル={TS}²、反復={repeats}、release={}、アダプター={} backend={:?} type={:?}",
        !cfg!(debug_assertions),
        info.name,
        info.backend,
        info.device_type
    );
    if info.device_type == wgpu::DeviceType::Cpu {
        println!("ソフトウェアの GPU の数値。実 GPU や Windows の結果ではない。");
    }
    println!("CPU = changed_tiles → composite_into → 乗算済みへ変換 → write_texture → 提出・完了待ち / GPU = ResidentCompositor::update（乗算済みの表示）");
    for &layers in &layer_counts {
        for dense in [true, false] {
            let (mut d, ids) = build(size, layers, dense);
            let painter = GpuPainter::from_device(
                info.clone(),
                device.clone(),
                queue.clone(),
                Options::default(),
            )?;
            let mut gpu = ResidentCompositor::with_gpu(
                painter,
                ResidentOptions {
                    resident_budget_bytes: 512 << 20,
                    premultiplied_display: true,
                    ..Default::default()
                },
            )?;
            // アプリは、描いたタイルを全部常駐させて予算に収まる文書だけ GPU で合成する（収まらなければ CPU）
            let options = ResidentOptions {
                resident_budget_bytes: 512 << 20,
                premultiplied_display: true,
                ..Default::default()
            };
            let need = resident_requirements(&d, Channel::Color, &options, &device.limits())?;
            println!(
                "\n要る量 {:.0} MiB / 予算 512 MiB → アプリは{}",
                need.total_bytes() as f64 / 1048576.0,
                if need.total_bytes() <= options.resident_budget_bytes {
                    "GPU"
                } else {
                    "CPU（追い出しながら合成すると全面の変更が遅いので使わない）"
                }
            );
            let mut cpu = Cpu::new(&device, &queue, size);
            let t = Instant::now();
            cpu.update(&d, true);
            let cpu_first = t.elapsed().as_secs_f64() * 1000.0;
            let t = Instant::now();
            let first = gpu.update(&d, Channel::Color)?;
            let gpu_first = t.elapsed().as_secs_f64() * 1000.0;
            println!(
                "\n{layers}層 {}: 初めて見せる CPU={cpu_first:.1}ms GPU={gpu_first:.1}ms（GPU は {} タイル常駐・{:.0} MiB）",
                if dense { "全層が全面" } else { "下地だけ全面・ほかは 4 タイル" },
                first.cached_tiles,
                first.resident_bytes as f64 / 1048576.0,
            );
            let n = size / TS;
            let mut seed = 0x1234567u32;
            // 変更の種類: 1 タイル・16 タイル（4×4 の大きなブラシ）・1 層の不透明度（その層の全タイル）
            for (name, kind) in [
                ("1 タイル", 0),
                ("16 タイル(4×4)", 1),
                ("1 層の不透明度", 2),
            ] {
                let mut cpu_ms = Vec::new();
                let mut gpu_ms = Vec::new();
                let mut worst = 0u8;
                let (mut tiles, mut uploaded) = (0, 0);
                for iteration in 0..=repeats {
                    // 変更の作成（計測の外）。描く層は、上の層のうち 1 枚（最後の層）。
                    let target = *ids.last().unwrap();
                    match kind {
                        0 => {
                            let tile = tile_pattern(&mut seed, 200);
                            d.import_tile(
                                target,
                                Channel::Color,
                                TileCoord::new(n / 2, n / 2),
                                &tile,
                            )
                            .unwrap();
                        }
                        1 => {
                            let tile = tile_pattern(&mut seed, 200);
                            for y in 0..4 {
                                for x in 0..4 {
                                    d.import_tile(
                                        target,
                                        Channel::Color,
                                        TileCoord::new(n / 2 + x, n / 2 + y),
                                        &tile,
                                    )
                                    .unwrap();
                                }
                            }
                        }
                        _ => {
                            let v = if iteration % 2 == 0 { 0.9 } else { 0.6 };
                            d.set_layer_opacity(target, v, false).unwrap();
                        }
                    }
                    let t = Instant::now();
                    let c = cpu.update(&d, false);
                    let cm = t.elapsed().as_secs_f64() * 1000.0;
                    let t = Instant::now();
                    let stats = gpu.update(&d, Channel::Color)?;
                    let gm = t.elapsed().as_secs_f64() * 1000.0;
                    tiles = c;
                    uploaded = stats.uploaded_tiles;
                    assert_eq!(stats.updated_tiles, c, "同じタイルを更新する");
                    // 一致の確認（計測の外）: 変えたタイルの範囲を読み戻して CPU の合成（乗算済み）と照らす
                    let rect = Rect::new((n / 2) * TS, (n / 2) * TS, TS, TS);
                    let request = gpu.request_readback(rect)?;
                    let actual = gpu.finish_readback(request)?;
                    let mut expected = d.composite(rect)?;
                    premultiply(&mut expected);
                    worst = worst.max(max_diff(&actual, &expected));
                    if iteration > 0 {
                        cpu_ms.push(cm);
                        gpu_ms.push(gm);
                    }
                }
                assert!(worst <= 2, "表示の差 {worst}");
                let (cm, gm) = (median(&mut cpu_ms), median(&mut gpu_ms));
                println!(
                    "  {name:<16} 更新タイル={tiles:<5} 転送タイル={uploaded:<4} CPU={cm:>9.3}ms GPU={gm:>9.3}ms GPU/CPU={:.2} 最大差={worst}",
                    gm / cm
                );
            }
        }
    }
    Ok(())
}
