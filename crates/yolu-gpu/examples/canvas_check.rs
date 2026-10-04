//! キャンバスの表示の合成を、使えるアダプターで CPU の合成と照らす（アプリの文書の機能: マスク・塗りつぶし・チャンネルごとの
//! 合成・クリッピング・通過のグループ・操作の列）。`tests/canvas.rs` と同じ照合を、テストの別スレッドではなくメインスレッドで
//! 動かす（コンテナの実 GPU の Mesa d3d12 は、テストの子スレッドでデバイスを作ると落ちる）。
//!   cargo run --release -p yolu-gpu --example canvas_check
//! 出力されるアダプター名・バックエンドを併せて記録する。straight の差が 2 を超えたら（乗算済みは変換の丸めで 3 を超えたら）失敗で終わる。
use yolu_core::{BlendMode, Channel, ChannelBlend, Document, LayerId, Rect, Rgba8};
use yolu_gpu::{GpuPainter, Options, ResidentCompositor, ResidentOptions};

struct Rng(u32);
impl Rng {
    fn byte(&mut self) -> u8 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as u8
    }
}

fn paint(d: &mut Document, layer: LayerId, rng: &mut Rng, alphas: &[u8]) {
    for y in 0..d.height() {
        for x in 0..d.width() {
            let a = alphas[(x as usize + y as usize / 3) % alphas.len()];
            d.set_pixel(
                layer,
                x,
                y,
                Rgba8::new(rng.byte(), rng.byte(), rng.byte(), a),
            )
            .unwrap();
        }
    }
}

fn paint_mask(d: &mut Document, layer: LayerId, rng: &mut Rng) {
    for y in 0..d.height() {
        for x in 0..d.width() {
            let hide = [0, 255, 128, rng.byte(), 1, 254][(x as usize + y as usize) % 6];
            d.set_mask_pixel(layer, x, y, hide).unwrap();
        }
    }
}

fn check(g: &mut ResidentCompositor, d: &Document, what: &str, premultiplied: bool) -> u8 {
    g.update(d, Channel::Color).unwrap();
    let rect = Rect::new(0, 0, d.width(), d.height());
    let mut expected = d.composite(rect).unwrap();
    if premultiplied {
        for p in expected.as_chunks_mut::<4>().0 {
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
    let request = g.request_readback(rect).unwrap();
    let actual = g.finish_readback(request).unwrap();
    let max = actual
        .iter()
        .zip(&expected)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    println!("  {what}: 最大差 {max}");
    let tolerance = if premultiplied { 3 } else { 2 };
    assert!(max <= tolerance, "{what}: 最大差 {max} > {tolerance}");
    max
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // インスタンスは 1 つだけ作り、デバイスを `from_device` で共有する（インスタンスを作り直すと落ちる環境がある）
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
    let info = adapter.get_info();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
    println!("アダプター: {info:?}");
    let mut worst = 0;
    for premultiplied in [false, true] {
        println!("乗算済みの表示 = {premultiplied}");
        let mut g = ResidentCompositor::with_gpu(
            GpuPainter::from_device(
                info.clone(),
                device.clone(),
                queue.clone(),
                Options::default(),
            )?,
            ResidentOptions {
                premultiplied_display: premultiplied,
                ..Default::default()
            },
        )?;
        let mut d = Document::with_tile_size(97, 61, 16)?;
        let mut rng = Rng(0x9e3779b9);
        let base = d.add_layer("下")?;
        paint(&mut d, base, &mut rng, &[255]);
        let a = d.add_layer("乗算")?;
        paint(&mut d, a, &mut rng, &[255, 200, 90, 255]);
        d.set_layer_blend_mode(a, BlendMode::Multiply)?;
        d.set_layer_opacity(a, 0.8, false)?;
        d.add_layer_mask(a)?;
        paint_mask(&mut d, a, &mut rng);
        worst = worst.max(check(&mut g, &d, "マスク付きの乗算", premultiplied));
        d.set_layer_mask_inverted(a, true)?;
        d.set_layer_mask_density(a, 0.4, false)?;
        worst = worst.max(check(&mut g, &d, "マスクの反転・濃度", premultiplied));
        let clip = d.add_layer("クリップ")?;
        paint(&mut d, clip, &mut rng, &[255, 60]);
        d.set_layer_blend_mode(clip, BlendMode::Screen)?;
        d.set_layer_clipping(clip, true)?;
        worst = worst.max(check(&mut g, &d, "クリッピング", premultiplied));
        let fill = d.add_fill_layer(
            "塗り",
            &[(Channel::Color, Rgba8::new(30, 90, 220, 140))],
            None,
        )?;
        d.set_layer_blend_mode(fill, BlendMode::Overlay)?;
        d.add_layer_mask(fill)?;
        for y in 0..d.height() / 2 {
            for x in 0..d.width() {
                d.set_mask_pixel(fill, x, y, 255)?;
            }
        }
        worst = worst.max(check(&mut g, &d, "塗りつぶし + マスク", premultiplied));
        let g1 = d.add_layer("組 1")?;
        paint(&mut d, g1, &mut rng, &[255, 120]);
        let g2 = d.add_layer("組 2")?;
        paint(&mut d, g2, &mut rng, &[200, 255, 0]);
        d.set_layer_clipping(g2, true)?;
        d.set_channel_blend(
            g2,
            Channel::Color,
            ChannelBlend::new(Some(BlendMode::SoftLight), Some(0.7)),
            false,
        )?;
        let group = d.group_layers(&[g1, g2], "組")?;
        worst = worst.max(check(
            &mut g,
            &d,
            "通過のグループ・チャンネルごとの合成",
            premultiplied,
        ));
        d.set_layer_visible(group, false)?;
        worst = worst.max(check(&mut g, &d, "グループを隠す", premultiplied));
        d.undo()?;
        worst = worst.max(check(&mut g, &d, "取消", premultiplied));
        d.move_layer_to(clip, None, 0)?;
        worst = worst.max(check(&mut g, &d, "層の並べ替え", premultiplied));
        d.remove_layer(a)?;
        worst = worst.max(check(&mut g, &d, "層の削除", premultiplied));
    }
    println!("最大差の最悪値 {worst}");
    Ok(())
}
