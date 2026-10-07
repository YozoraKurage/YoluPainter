//! PSD を開いたときのキャンバスの表示の合成を、CPU と GPU の常駐の合成で比べる計測（アプリの表示の道と同じ手順）。
//!   cargo run --release -p yolu-app --example canvas_psd_measure -- <PSD のパス> [予算の MiB（既定 512）]
//!
//! 出すもの: 文書の形（大きさ・レイヤーの種類の数・調整の種類・効果の有無）、GPU で合成できるか（できなければ理由）、全部を常駐させるのに要る量、
//! 全面の合成の時間（CPU は評価のキャッシュが空の初回と、あるあとの 2 回。GPU は初回）、レイヤーの操作（不透明度・1 タイルの描き込み）のあとの
//! 表示の更新の時間（CPU は `changed_tiles` を合成し直して乗算済みへ変換して転送、GPU は `ResidentCompositor::update`）、CPU との画素の差。
//! アダプター名・バックエンドを併せて出す。ソフトウェアの GPU（llvmpipe・lavapipe）と実 GPU を混同しないこと。
use eframe::egui_wgpu::wgpu;
use std::time::Instant;
use yolu_core::{AdjustmentType, Channel, Document, LayerId, LayerKind, RowOrder, TileCoord};
use yolu_gpu::{
    resident_requirements, supports, GpuPainter, Options, ResidentCompositor, ResidentOptions,
};
use yolu_io::psd::{self, CopyOptions, CopyOutcome};

const MIB: f64 = 1024.0 * 1024.0;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
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

/// アプリの CPU の表示と同じ手順（`changed_tiles` を合成し直して、乗算済みにして、表示のテクスチャへ転送する）。
struct Cpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    texture: wgpu::Texture,
    serial: u64,
    buffer: Vec<u8>,
}

impl Cpu {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, doc: &Document) -> Cpu {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cpu 表示"),
            size: wgpu::Extent3d {
                width: doc.width(),
                height: doc.height(),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
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
                timeout: Some(std::time::Duration::from_secs(120)),
            })
            .unwrap();
        coords.len()
    }
}

fn describe(doc: &Document) {
    let mut raster = 0;
    let mut fill = 0;
    let mut group = 0;
    let mut adjustment = 0;
    let mut kinds: std::collections::BTreeMap<u8, usize> = Default::default();
    let mut isolated = 0;
    let mut masks = 0;
    let mut clipping = 0;
    let mut effects = 0;
    let mut tiles = 0usize;
    for l in doc.layers() {
        match l.kind() {
            LayerKind::Raster => raster += 1,
            LayerKind::Fill => fill += 1,
            LayerKind::Group => {
                group += 1;
                if l.blend_mode_in(Channel::Color) != yolu_core::BlendMode::PassThrough
                    || l.opacity_in(Channel::Color) != 1.0
                {
                    isolated += 1;
                }
            }
            LayerKind::Adjustment => {
                adjustment += 1;
                if let Some(a) = l.adjustment() {
                    *kinds.entry(a.kind() as u8).or_default() += 1;
                }
            }
        }
        if l.mask().is_some() {
            masks += 1;
        }
        if l.clipping() {
            clipping += 1;
        }
        if l.has_evaluated_output(Channel::Color)
            || l.mask().is_some_and(|m| m.has_active_filters())
        {
            effects += 1;
        }
        if let Some(s) = l.surface(Channel::Color) {
            tiles += s.tile_count();
        }
    }
    println!(
        "キャンバス {}×{}（タイル {}²）、レイヤー {}: ラスター {raster}・塗りつぶし {fill}・グループ {group}（独立 {isolated}）・調整 {adjustment}・マスク {masks}・クリッピング {clipping}・効果のあるレイヤー {effects}、Color の描いたタイル {tiles}",
        doc.width(),
        doc.height(),
        doc.tile_size(),
        doc.layers().len()
    );
    let names: Vec<String> = kinds
        .iter()
        .map(|(k, n)| {
            format!(
                "{:?}×{n}",
                AdjustmentType::from_index(i64::from(*k)).unwrap()
            )
        })
        .collect();
    println!("調整の種類: {}", names.join("・"));
}

/// 描いたタイルの多い順に、(レイヤー, そのタイルの数)。ラスターで Color の面を持つもの。
fn biggest_layers(doc: &Document) -> Vec<(LayerId, usize)> {
    let mut all: Vec<(LayerId, usize)> = doc
        .layers()
        .iter()
        .filter(|l| l.kind() == LayerKind::Raster && l.visible())
        .filter_map(|l| {
            l.surface(Channel::Color)
                .map(|s| (l.id(), s.tile_count()))
                .filter(|(_, n)| *n > 0)
        })
        .collect();
    all.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    all
}

fn max_and_mean(a: &[u8], b: &[u8]) -> (u8, f64, usize) {
    let mut max = 0u8;
    let mut sum = 0u64;
    let mut over = 0usize;
    for (x, y) in a.iter().zip(b) {
        let d = x.abs_diff(*y);
        max = max.max(d);
        sum += u64::from(d);
        if d > 2 {
            over += 1;
        }
    }
    (max, sum as f64 / a.len().max(1) as f64, over)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let path = args.get(1).ok_or("PSD のパスを渡す")?;
    let budget: u64 = args.get(2).map_or(Ok(512), |s| s.parse())?;
    let budget = budget << 20;
    let t = Instant::now();
    let file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::with_capacity(256 * 1024, file);
    let outcome = psd::import_copy(
        &mut reader,
        &CopyOptions {
            source_budget: 4 << 30,
            cancel: None,
        },
    )?;
    let imported = match outcome {
        CopyOutcome::Imported(i) => i,
        CopyOutcome::Refused(why) => return Err(format!("取り込めない: {}", why.message()).into()),
    };
    let doc = imported.document;
    println!("取り込み {:.0} ms", ms(t));
    let mut doc = doc;
    // 環境変数 EXTRA（カンマ区切り）で、開いた文書に機能を足して測る: adjust = 一番上に色相/彩度の調整レイヤーと、描いたタイルの
    // 多いレイヤーへクリッピングしたレベル補正、blur = 描いたタイルの多いレイヤーにぼかし（半径 8）のフィルター
    if let Ok(extra) = std::env::var("EXTRA") {
        let biggest = biggest_layers(&doc).first().map(|(id, _)| *id);
        for item in extra.split(',').map(str::trim) {
            match (item, biggest) {
                ("adjust", Some(target)) => {
                    let top = doc.add_adjustment_layer(
                        "色相",
                        yolu_core::AdjustmentSettings::hue_saturation(40.0, 0.2, 0.0)?,
                        None,
                        None,
                    )?;
                    let _ = top;
                    let clipped = doc.add_adjustment_layer(
                        "レベル",
                        yolu_core::AdjustmentSettings::levels(0.1, 0.9, 1.3, 0.0, 1.0)?,
                        None,
                        Some(target),
                    )?;
                    doc.set_layer_clipping(clipped, true)?;
                }
                ("blur", Some(target)) => {
                    doc.add_filter(
                        target,
                        yolu_core::FilterTarget::Content,
                        yolu_core::FilterSpec::new(yolu_core::EffectSettings::blur(8))
                            .channels(&[Channel::Color]),
                    )?;
                }
                (other, _) => return Err(format!("知らない EXTRA: {other}").into()),
            }
        }
        println!("EXTRA={extra} を足した");
    }
    describe(&doc);

    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
    let info = adapter.get_info();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
    println!(
        "release={}、アダプター={} backend={:?} type={:?}、予算 {} MiB",
        !cfg!(debug_assertions),
        info.name,
        info.backend,
        info.device_type,
        budget >> 20
    );
    if info.device_type == wgpu::DeviceType::Cpu {
        println!("ソフトウェアの GPU の数値。実 GPU や Windows の結果ではない。");
    }

    // CPU の全面の合成（評価のキャッシュが空の初回と、あるあと）
    let mut cpu = Cpu::new(&device, &queue, &doc);
    doc.release_effect_cache();
    let t = Instant::now();
    cpu.update(&doc, true);
    let cpu_cold = ms(t);
    let t = Instant::now();
    cpu.update(&doc, true);
    let cpu_warm = ms(t);
    println!("CPU 全面: 初回 {cpu_cold:.0} ms・評価のキャッシュあり {cpu_warm:.0} ms");

    let supported = supports(&doc, Channel::Color);
    let options = ResidentOptions {
        resident_budget_bytes: budget,
        premultiplied_display: true,
        ..Default::default()
    };
    match &supported {
        Ok(()) => println!("GPU で合成できる文書"),
        Err(why) => println!("GPU で合成できない: {why}"),
    }
    let need = resident_requirements(&doc, Channel::Color, &options, &device.limits());
    match &need {
        Ok(n) => println!(
            "全部を常駐させるのに要る量 {:.0} MiB（予算 {} MiB → {}）",
            n.total_bytes() as f64 / MIB,
            budget >> 20,
            if n.total_bytes() <= budget {
                "収まる"
            } else {
                "超える（アプリは CPU）"
            }
        ),
        Err(e) => println!("要る量を見積もれない: {e}"),
    }
    if supported.is_err() || need.is_err() {
        println!("GPU の計測は省略");
        return Ok(());
    }

    let painter = GpuPainter::from_device(
        info.clone(),
        device.clone(),
        queue.clone(),
        Options::default(),
    )?;
    let mut gpu = ResidentCompositor::with_gpu(painter, options)?;
    doc.release_effect_cache();
    let t = Instant::now();
    let first = match gpu.update(&doc, Channel::Color) {
        Ok(s) => s,
        Err(e) => {
            println!("GPU の初回の更新に失敗: {e}");
            return Ok(());
        }
    };
    let gpu_first = ms(t);
    println!(
        "GPU 全面: 初回 {gpu_first:.0} ms（{} タイル更新・常駐 {} タイル {:.0} MiB・転送 {:.0} MiB）",
        first.updated_tiles,
        first.cached_tiles,
        first.resident_bytes as f64 / MIB,
        first.uploaded_bytes as f64 / MIB,
    );
    let compare = |gpu: &mut ResidentCompositor, doc: &Document, what: &str| {
        let expected = {
            let mut v = doc.composite_channel(Channel::Color, doc.bounds()).unwrap();
            premultiply(&mut v);
            v
        };
        let request = gpu.request_readback(doc.bounds()).unwrap();
        let actual = gpu.finish_readback(request).unwrap();
        let (max, mean, over) = max_and_mean(&expected, &actual);
        println!("{what}: CPU との差 最大 {max}・平均 {mean:.4}・2 を超える成分 {over}");
    };
    compare(&mut gpu, &doc, "初回の表示");

    // レイヤーの操作: 描いたタイルの最も多いレイヤーの不透明度と、1 タイルの描き込み
    let layers = biggest_layers(&doc);
    if layers.is_empty() {
        println!("描いたラスターレイヤーが無いのでレイヤーの操作は省略");
        return Ok(());
    }
    let (target, tiles) = layers[0];
    let median = layers[layers.len() / 2].0;
    let mut stamp = |name: &str, doc: &mut Document, change: &dyn Fn(&mut Document)| {
        change(doc);
        let n_changed = doc
            .changed_tiles(Channel::Color, cpu.serial)
            .map_or(0, |v| v.len());
        let t = Instant::now();
        cpu.update(doc, false);
        let c = ms(t);
        let t = Instant::now();
        let stats = gpu.update(doc, Channel::Color).unwrap();
        let g = ms(t);
        println!(
            "{name}: 変わったタイル {n_changed} → CPU {c:.1} ms / GPU {g:.1} ms（更新 {} タイル・転送 {} タイル）",
            stats.updated_tiles, stats.uploaded_tiles
        );
        compare(&mut gpu, doc, &format!("  {name} のあと"));
    };
    println!("操作するレイヤー: タイルの多いレイヤー（{tiles} タイル）");
    stamp(
        "不透明度 0.5（タイルの多いレイヤー）",
        &mut doc,
        &|d| d.set_layer_opacity(target, 0.5, false).unwrap(),
    );
    stamp("不透明度を戻す", &mut doc, &|d| {
        d.set_layer_opacity(target, 1.0, false).unwrap()
    });
    stamp("非表示（中ほどのレイヤー）", &mut doc, &|d| {
        d.set_layer_visible(median, false).unwrap()
    });
    stamp("表示に戻す", &mut doc, &|d| {
        d.set_layer_visible(median, true).unwrap()
    });
    // 1 タイルの描き込み
    let ts = doc.tile_size();
    let coord = TileCoord::new(doc.width().div_ceil(ts) / 2, doc.height().div_ceil(ts) / 2);
    let mut seed = 0x1234567u32;
    let mut tile = vec![0u8; (ts * ts * 4) as usize];
    for px in tile.as_chunks_mut::<4>().0 {
        for v in &mut px[..3] {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *v = seed as u8;
        }
        px[3] = 255;
    }
    stamp("1 タイルの描き込み", &mut doc, &|d| {
        d.import_tile(target, Channel::Color, coord, &tile).unwrap();
    });
    Ok(())
}
