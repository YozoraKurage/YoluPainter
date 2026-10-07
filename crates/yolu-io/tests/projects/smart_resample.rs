//! 寸法・タイル寸法を変えて置くときの画素を、C# の Core が作った答えと全バイトで照らす
//! （`tools/csharp-golden/SmartGolden.cs`。元が複数タイルで疎・一様・端のタイルが混ざる面、Normal のチャンネル、マスク）。
use yolu_core::{
    smart::{SmartMaterial, SmartPlacement, SmartResampling},
    Channel, Document, Rgba8,
};
fn expected(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/smart")
            .join(name),
    )
    .unwrap()
}
const METHODS: [SmartResampling; 3] = [
    SmartResampling::Nearest,
    SmartResampling::Bilinear,
    SmartResampling::Area,
];
fn sparse_material() -> SmartMaterial {
    let mut d = Document::with_tile_size(9, 7, 2).unwrap();
    let l = d.add_layer("疎").unwrap();
    for y in 0..4 {
        for x in 0..4 {
            d.set_pixel(l, x, y, Rgba8::new(90, 100, 110, 255)).unwrap();
        }
    }
    for y in 2..4 {
        for x in 6..8 {
            d.set_pixel(l, x, y, Rgba8::new(200, 30, 60, 128)).unwrap();
        }
    }
    for y in 4..6u32 {
        for x in 4..6u32 {
            let alpha = if (x + y) % 3 == 0 { 0 } else { 100 + x * 10 };
            let color = Rgba8::new(
                ((x * 40 + y * 3) % 256) as u8,
                ((y * 50 + x * 7) % 256) as u8,
                (((x + y) * 20) % 256) as u8,
                alpha as u8,
            );
            d.set_pixel(l, x, y, color).unwrap();
        }
    }
    d.set_pixel(l, 8, 6, Rgba8::new(10, 20, 30, 255)).unwrap();
    for y in 0..2 {
        d.set_pixel(l, 8, y, Rgba8::new(50, 60, 70, 255)).unwrap();
    }
    d.capture_smart_material(&[l], "疎な素材").unwrap()
}
fn place(material: &SmartMaterial, w: u32, h: u32, tile: u32, how: SmartResampling) -> Vec<u8> {
    let mut target = Document::with_tile_size(w, h, tile).unwrap();
    let placed = target
        .place_smart_material(
            material,
            &SmartPlacement {
                resampling: Some(how),
                ..Default::default()
            },
        )
        .unwrap();
    let layer = target.layer(placed.layer_id).unwrap();
    let channel = if material
        .layers()
        .iter()
        .any(|l| l.surface(Channel::Normal).is_some())
    {
        Channel::Normal
    } else {
        Channel::Color
    };
    layer.surface(channel).unwrap().to_canvas_bytes()
}
#[test]
fn sparse_multi_tile_source_matches_csharp_for_fifteen_cases() {
    let material = sparse_material();
    for (w, h, tile) in [(5, 3, 4), (20, 15, 8), (13, 11, 2), (9, 7, 4), (4, 9, 2)] {
        for (i, how) in METHODS.iter().enumerate() {
            assert_eq!(
                place(&material, w, h, tile, *how),
                expected(&format!("resize-sparse-{w}-{h}-{tile}-{i}.bin")),
                "{w}x{h}/{tile}/{i}"
            );
        }
    }
}
#[test]
fn normal_channel_resampling_renormalises_like_csharp() {
    let mut d = Document::with_tile_size(4, 3, 2).unwrap();
    let l = d.add_layer("法線").unwrap();
    for y in 0..2 {
        for x in 0..2 {
            d.set_channel_pixel(l, Channel::Normal, x, y, Rgba8::new(128, 128, 255, 255))
                .unwrap();
        }
    }
    for (x, y, p) in [
        (2, 0, Rgba8::new(200, 128, 220, 255)),
        (3, 0, Rgba8::new(60, 190, 200, 200)),
        (2, 1, Rgba8::new(128, 60, 230, 0)),
        (3, 1, Rgba8::new(90, 90, 250, 255)),
        (2, 2, Rgba8::new(255, 128, 128, 255)),
    ] {
        d.set_channel_pixel(l, Channel::Normal, x, y, p).unwrap();
    }
    let material = d.capture_smart_material(&[l], "法線の素材").unwrap();
    for (w, h, tile) in [(7, 5, 4), (2, 1, 2), (2, 5, 2), (4, 3, 4)] {
        for (i, how) in METHODS.iter().enumerate() {
            assert_eq!(
                place(&material, w, h, tile, *how),
                expected(&format!("resize-normal-{w}-{h}-{tile}-{i}.bin")),
                "{w}x{h}/{tile}/{i}"
            );
        }
    }
}
#[test]
fn smart_mask_resampling_matches_csharp() {
    let mut d = Document::with_tile_size(5, 4, 2).unwrap();
    let l = d.add_layer("マスクのレイヤー").unwrap();
    d.add_layer_mask(l).unwrap();
    for (x, y, hide) in [(0, 0, 120), (1, 1, 200), (3, 0, 77), (4, 3, 255)] {
        d.set_mask_pixel(l, x, y, hide).unwrap();
    }
    let mask = d.capture_smart_mask(l, "マスクの素材").unwrap();
    for (w, h, tile) in [(11, 9, 4), (3, 2, 2), (5, 4, 4), (8, 3, 8)] {
        for (i, how) in METHODS.iter().enumerate() {
            let mut target = Document::with_tile_size(w, h, tile).unwrap();
            let layer = target.add_layer("先").unwrap();
            target.apply_smart_mask(&mask, layer, Some(*how)).unwrap();
            assert_eq!(
                target
                    .layer(layer)
                    .unwrap()
                    .mask()
                    .unwrap()
                    .surface()
                    .to_canvas_bytes(),
                expected(&format!("resize-mask-{w}-{h}-{tile}-{i}.bin")),
                "{w}x{h}/{tile}/{i}"
            );
        }
    }
}
