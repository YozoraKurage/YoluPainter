//! UDIM のシート: UDIM にする・やめる・タイルを合わせる・セルごとの拡大縮小（1 回の Undo）、近くの画素を読むフィルターがセルをまたがない、
//! UV の投影・UV の空間のノイズがタイルの中の UV（各セルで同じ大きさ）、UV の外への塗り広げがセルの外へ出ない、シートの大きさの断り。
use yolu_core::fill_image::{Projection, ProjectionMode};
use yolu_core::filter::Settings;
use yolu_core::padding::{dilate_in_cells, Reach, Rings};
use yolu_core::udim::{CellGrid, UdimLayout};
use yolu_core::{
    CanvasResampling, Channel, CoreError, Document, EffectInputs, EffectSettings, FilterSpec,
    FilterTarget, ImageColorSpace, ImageId, ImageInput, Reason, Rect, Rgba8,
};

const S: u32 = 16;
const RED: Rgba8 = Rgba8::new(220, 20, 20, 255);
const BLUE: Rgba8 = Rgba8::new(20, 20, 220, 255);
const IMAGE: ImageId = ImageId(0x5a5a_0000_1111_4222_8333_4444_5555_6666);

fn color_at(doc: &Document, x: u32, y: u32) -> Rgba8 {
    doc.composite_pixel(Channel::Color, x, y).unwrap()
}

/// 1 つのセル（S × S）の文書に、左半分が赤・右半分が青のレイヤー。
fn painted() -> (Document, yolu_core::LayerId) {
    let mut doc = Document::with_tile_size(S, S, 8).unwrap();
    let id = doc.add_layer("a").unwrap();
    for y in 0..S {
        for x in 0..S {
            doc.set_pixel(id, x, y, if x < S / 2 { RED } else { BLUE })
                .unwrap();
        }
    }
    (doc, id)
}

#[test]
fn enabling_moves_the_pixels_into_the_1001_cell_and_one_undo_restores() {
    let (mut doc, id) = painted();
    let before = doc.composite(doc.bounds()).unwrap();
    doc.enable_udim(vec![1002, 1001, 1011]).unwrap();
    let layout = doc.udim().unwrap().clone();
    assert_eq!(layout.tiles(), &[1001, 1002, 1011]);
    assert_eq!((doc.width(), doc.height()), (2 * S, 2 * S));
    assert_eq!(color_at(&doc, 0, 0), RED, "1001 は左下のセル");
    assert_eq!(color_at(&doc, S - 1, 0), BLUE);
    assert_eq!(color_at(&doc, S, 0).a, 0, "ほかのセルは空");
    assert_eq!(doc.uv_cell(), (S, S));
    assert_eq!(
        doc.udim_cells(),
        Some(CellGrid {
            width: S,
            height: S
        })
    );
    doc.undo().unwrap();
    assert!(doc.udim().is_none());
    assert_eq!(doc.composite(doc.bounds()).unwrap(), before);
    doc.redo().unwrap();
    assert_eq!(doc.udim().unwrap().as_ref(), layout.as_ref());
    // やめると 1001 のセルだけが残る（1 回の Undo で戻る）
    doc.set_pixel(id, S + 1, 1, RED).unwrap();
    doc.disable_udim().unwrap();
    assert!(doc.udim().is_none());
    assert_eq!(doc.composite(doc.bounds()).unwrap(), before);
    doc.undo().unwrap();
    assert_eq!(
        color_at(&doc, S + 1, 1),
        RED,
        "ほかのセルの中身も Undo で戻る"
    );
}

#[test]
fn without_1001_the_smallest_tile_takes_the_pixels() {
    let (mut doc, _) = painted();
    doc.enable_udim(vec![1003, 1002]).unwrap();
    assert_eq!(doc.udim().unwrap().tiles(), &[1002, 1003]);
    assert_eq!(color_at(&doc, 0, 0), RED);
    assert_eq!(color_at(&doc, S, 0).a, 0);
}

#[test]
fn syncing_adds_tiles_and_moves_cells_to_the_new_places() {
    let (mut doc, id) = painted();
    doc.enable_udim(vec![1001, 1002]).unwrap();
    // 1002 のセル（右）に印
    doc.set_pixel(id, S + 3, 2, RED).unwrap();
    assert!(
        doc.sync_udim([1001, 1002]).unwrap().is_none(),
        "足すタイルが無い"
    );
    doc.sync_udim([1011, 1012]).unwrap().unwrap();
    let l = doc.udim().unwrap().clone();
    assert_eq!(l.tiles(), &[1001, 1002, 1011, 1012]);
    assert_eq!((l.columns(), l.rows()), (2, 2));
    // 2 × 2 でも 1002 は右下のまま
    assert_eq!(color_at(&doc, S + 3, 2), RED);
    // 3 つ目を足して 3 × 2 になると、1011 は 3 つ目のセル（右下）へ動く
    doc.set_pixel(id, 1, S + 1, BLUE).unwrap();
    doc.sync_udim([1003]).unwrap().unwrap();
    let l = doc.udim().unwrap().clone();
    assert_eq!(l.tiles(), &[1001, 1002, 1003, 1011, 1012]);
    assert_eq!((l.columns(), l.rows()), (3, 2));
    let r = l.cell_rect(l.cell_of_tile(1011).unwrap());
    assert_eq!(
        color_at(&doc, r.x + 1, r.y + 1),
        BLUE,
        "1011 の中身は新しいセルへ"
    );
    doc.undo().unwrap();
    assert_eq!(doc.udim().unwrap().tiles(), &[1001, 1002, 1011, 1012]);
    assert_eq!(color_at(&doc, 1, S + 1), BLUE);
}

#[test]
fn a_sheet_over_the_edge_limit_is_refused_with_a_reason() {
    let mut doc = Document::with_tile_size(4096, 4096, 128).unwrap();
    doc.add_layer("a").unwrap();
    let err = doc
        .enable_udim(vec![1001, 1002, 1003, 1004, 1005])
        .unwrap_err();
    assert_eq!(err, CoreError::InvalidArgument(Reason::UdimSheetTooLarge));
    assert!(doc.udim().is_none());
    // 予算: 1 枚の面がシートの画素に足りないと断る
    let mut small = Document::with_tile_size(64, 64, 8).unwrap();
    small.set_source_budget_bytes(64 * 64 * 4 + 16).unwrap();
    assert_eq!(
        small.enable_udim(vec![1001, 1002]).unwrap_err(),
        CoreError::InvalidArgument(Reason::UdimOverBudget)
    );
}

#[test]
fn resizing_scales_each_cell_without_mixing_neighbours() {
    let (mut doc, id) = painted();
    // 全部を赤にして、1002 のセルは青
    for y in 0..S {
        for x in 0..S {
            doc.set_pixel(id, x, y, RED).unwrap();
        }
    }
    doc.enable_udim(vec![1001, 1002]).unwrap();
    for y in 0..S {
        for x in S..2 * S {
            doc.set_pixel(id, x, y, BLUE).unwrap();
        }
    }
    doc.resize_image(4 * S, 2 * S, CanvasResampling::Bilinear)
        .unwrap();
    let l = doc.udim().unwrap().clone();
    assert_eq!(l.cell_size(), (2 * S, 2 * S));
    // 境の両側は混ざらない
    assert_eq!(color_at(&doc, 2 * S - 1, 5), RED);
    assert_eq!(color_at(&doc, 2 * S, 5), BLUE);
    // 列の数で割り切れない大きさは断る
    assert!(doc
        .resize_image(4 * S + 1, 2 * S, CanvasResampling::Bilinear)
        .is_err());
    // キャンバスだけの変更は断る
    assert!(doc.resize_canvas(4 * S, 4 * S, (0, 0)).is_err());
    doc.undo().unwrap();
    assert_eq!(doc.udim().unwrap().cell_size(), (S, S));
}

#[test]
fn blur_does_not_read_across_cells() {
    let mut doc = Document::with_tile_size(2 * S, S, 8).unwrap();
    doc.set_udim_for_load(Some(UdimLayout::packed(vec![1001, 1002], S, S).unwrap()))
        .unwrap();
    let id = doc.add_layer("a").unwrap();
    for y in 0..S {
        for x in 0..2 * S {
            doc.set_pixel(id, x, y, if x < S { RED } else { BLUE })
                .unwrap();
        }
    }
    doc.add_filter(
        id,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(4)).channels(&[Channel::Color]),
    )
    .unwrap();
    // セルの境の両側は元の色のまま（隣のセルの色が入らない）
    assert_eq!(color_at(&doc, S - 1, 5), RED);
    assert_eq!(color_at(&doc, S, 5), BLUE);
    // UDIM でなければ境で混ざる（同じ画素で比べる）
    let mut plain = Document::with_tile_size(2 * S, S, 8).unwrap();
    let p = plain.add_layer("a").unwrap();
    for y in 0..S {
        for x in 0..2 * S {
            plain
                .set_pixel(p, x, y, if x < S { RED } else { BLUE })
                .unwrap();
        }
    }
    plain
        .add_filter(
            p,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(4)).channels(&[Channel::Color]),
        )
        .unwrap();
    assert_ne!(
        plain.composite_pixel(Channel::Color, S - 1, 5).unwrap(),
        RED
    );
}

#[test]
fn the_filter_engine_splits_blocks_at_cells() {
    // 直の評価: セルの格子つきの結果は、各セルを別の画像として評価した結果と同じ
    let (w, h) = (24u32, 10u32);
    let mut src = Vec::new();
    for y in 0..h {
        for x in 0..w {
            src.extend([(x * 10) as u8, (y * 20) as u8, ((x + y) * 7) as u8, 255]);
        }
    }
    let image = yolu_core::filter::Image::new(&src, w, h).unwrap();
    let stack = [yolu_core::filter::Stage::new(Settings::GaussianBlur {
        radius: 3,
    })];
    let options = yolu_core::filter::Options {
        block_size: 7,
        cells: Some(CellGrid {
            width: 12,
            height: 10,
        }),
        ..Default::default()
    };
    let got = yolu_core::filter::evaluate(
        &image,
        yolu_core::filter::ValueType::Color,
        &stack,
        Rect::new(0, 0, w, h),
        &options,
    )
    .unwrap();
    for cell in 0..2u32 {
        let part: Vec<u8> = (0..h)
            .flat_map(|y| {
                let o = ((y * w + cell * 12) * 4) as usize;
                src[o..o + 48].to_vec()
            })
            .collect();
        let one = yolu_core::filter::Image::new(&part, 12, 10).unwrap();
        let want = yolu_core::filter::evaluate(
            &one,
            yolu_core::filter::ValueType::Color,
            &stack,
            Rect::new(0, 0, 12, 10),
            &Default::default(),
        )
        .unwrap();
        for y in 0..h {
            let g = ((y * w + cell * 12) * 4) as usize;
            let k = (y * 12 * 4) as usize;
            assert_eq!(&got[g..g + 48], &want[k..k + 48], "セル {cell} の行 {y}");
        }
    }
}

fn checker(w: u32, h: u32) -> ImageInput {
    let mut px = Vec::new();
    for y in 0..h {
        for x in 0..w {
            px.extend(if (x + y) % 2 == 0 {
                [250, 250, 250, 255]
            } else {
                [10, 10, 10, 255]
            });
        }
    }
    ImageInput::new(w, h, px, ImageColorSpace::Srgb).unwrap()
}

#[test]
fn uv_projected_images_repeat_in_every_cell_at_the_same_size() {
    let make = |udim: bool| {
        let (w, h) = if udim { (2 * S, 2 * S) } else { (S, S) };
        let mut doc = Document::with_tile_size(w, h, 8).unwrap();
        if udim {
            doc.set_udim_for_load(Some(
                UdimLayout::packed(vec![1001, 1002, 1011], S, S).unwrap(),
            ))
            .unwrap();
        }
        doc.set_effect_inputs(EffectInputs::default().with_image(IMAGE, checker(4, 4)))
            .unwrap();
        let fill = doc
            .add_fill_layer("塗り", &[(Channel::Color, RED)], None)
            .unwrap();
        doc.set_fill_image(fill, Channel::Color, Some(IMAGE))
            .unwrap();
        let p = Projection {
            mode: ProjectionMode::Uv,
            ..Projection::default()
        };
        doc.set_fill_projection(fill, p, false).unwrap();
        doc
    };
    let plain = make(false);
    let sheet = make(true);
    let want = plain.composite(plain.bounds()).unwrap();
    for cell in [0u32, 1, 2] {
        let (ox, oy) = ((cell % 2) * S, (cell / 2) * S);
        for y in 0..S {
            for x in 0..S {
                let i = ((y * S + x) * 4) as usize;
                assert_eq!(
                    sheet
                        .composite_pixel(Channel::Color, ox + x, oy + y)
                        .unwrap(),
                    Rgba8::from_slice(&want[i..i + 4]),
                    "セル {cell} の ({x}, {y})"
                );
            }
        }
    }
}

#[test]
fn uv_noise_is_the_same_in_every_cell() {
    let make = |udim: bool| {
        let (w, h) = if udim { (2 * S, S) } else { (S, S) };
        let mut doc = Document::with_tile_size(w, h, 8).unwrap();
        if udim {
            doc.set_udim_for_load(Some(UdimLayout::packed(vec![1001, 1002], S, S).unwrap()))
                .unwrap();
        }
        let fill = doc
            .add_fill_layer(
                "塗り",
                &[(Channel::Color, Rgba8::new(200, 200, 200, 255))],
                None,
            )
            .unwrap();
        let mut g = yolu_core::generator::Settings::new(yolu_core::generator::Kind::Noise);
        g.blend = yolu_core::generator::Blend::Replace;
        g.procedural.space = yolu_core::generator::ProceduralSpace::Uv;
        doc.add_filter(
            fill,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(g)).channels(&[Channel::Color]),
        )
        .unwrap();
        doc
    };
    let plain = make(false);
    let sheet = make(true);
    for y in 0..S {
        for x in 0..S {
            let want = plain.composite_pixel(Channel::Color, x, y).unwrap();
            assert_eq!(sheet.composite_pixel(Channel::Color, x, y).unwrap(), want);
            assert_eq!(
                sheet.composite_pixel(Channel::Color, S + x, y).unwrap(),
                want
            );
        }
    }
}

#[test]
fn dilation_stays_inside_the_cell() {
    // 左のセルの右端の列だけ覆い（赤）、右のセルは覆いなし
    let (w, h) = (8u32, 4u32);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let mut keep = vec![false; (w * h) as usize];
    for y in 0..h {
        let i = (y * w + 3) as usize;
        keep[i] = true;
        rgba[i * 4..i * 4 + 4].copy_from_slice(&[255, 0, 0, 255]);
    }
    let cells = Some(CellGrid {
        width: 4,
        height: 4,
    });
    let out = dilate_in_cells(&rgba, w, h, &keep, Reach::Fill, u64::MAX, None, cells).unwrap();
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let a = out[i + 3];
            if x < 4 {
                assert_eq!(a, 255, "左のセルは埋まる ({x}, {y})");
            } else {
                assert_eq!(a, 0, "右のセルへは広げない ({x}, {y})");
            }
        }
    }
    let rings = Rings::new_in_cells(w, h, &keep, 4, cells).unwrap();
    assert_eq!(rings.ring(4, 0), None, "段の地図もセルの外へ届かない");
    assert_eq!(rings.ring(2, 0), Some(1));
}

/// シート（2S × S）の大きさの位置・法線のマップ（人工。X はシートを横切って変わる）。
fn sheet_inputs() -> EffectInputs {
    use yolu_core::generator::{MapKind, MapState};
    use yolu_core::{MapInput, ModelFrame};
    let (w, h) = (2 * S, S);
    let key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let mut inputs = EffectInputs::new();
    for kind in [MapKind::Position, MapKind::WorldNormal] {
        let mut data = Vec::new();
        let mut coverage = Vec::new();
        for y in 0..h {
            for x in 0..w {
                for c in 0..kind.channels() {
                    data.push(match kind {
                        MapKind::Position => match c {
                            0 => (65535.0 * (x as f64 + 0.5) / w as f64) as u16,
                            1 => (65535.0 * (y as f64 + 0.5) / h as f64) as u16,
                            _ => (65535.0 * (((x * 3 + y * 5) % 41) as f64 / 40.0)) as u16,
                        },
                        _ => ((x * 1193 + y * 3571 + c as u32 * 13451) % 65536) as u16,
                    });
                }
                coverage.push(1);
            }
        }
        inputs = inputs
            .with_map(
                MapInput::new(
                    kind,
                    w,
                    h,
                    data,
                    coverage,
                    [-1.0, -2.0, -3.0],
                    [2.0, 3.0, 1.0],
                    key,
                    MapState::Current,
                )
                .unwrap(),
            )
            .unwrap();
    }
    inputs.with_frame(Some(
        ModelFrame::new([0.13, -0.27, 0.41], [0.17, -0.31, 0.23, 0.89]).unwrap(),
    ))
}

#[test]
fn position_and_triplanar_noise_read_the_baked_maps_in_a_udim_set() {
    use yolu_core::generator::{Blend, GrungePreset, Inactive, Kind, ProceduralSpace, Settings};
    let kinds = [
        (Settings::new(Kind::Noise), ProceduralSpace::Position),
        (Settings::new(Kind::Noise), ProceduralSpace::Triplanar),
        (
            Settings::grunge(GrungePreset::Cracks),
            ProceduralSpace::Position,
        ),
        (
            Settings::grunge(GrungePreset::Rust),
            ProceduralSpace::Triplanar,
        ),
    ];
    for (mut g, space) in kinds {
        g.blend = Blend::Replace;
        g.procedural.space = space;
        let make = |udim: bool| {
            let mut doc = Document::with_tile_size(2 * S, S, 8).unwrap();
            if udim {
                doc.set_udim_for_load(Some(UdimLayout::packed(vec![1001, 1002], S, S).unwrap()))
                    .unwrap();
            }
            doc.set_effect_inputs(sheet_inputs()).unwrap();
            let fill = doc
                .add_fill_layer(
                    "塗り",
                    &[(Channel::Color, Rgba8::new(200, 200, 200, 255))],
                    None,
                )
                .unwrap();
            let id = doc
                .add_filter(
                    fill,
                    FilterTarget::Content,
                    FilterSpec::new(EffectSettings::generator(g.clone()))
                        .channels(&[Channel::Color]),
                )
                .unwrap();
            (doc, fill, id)
        };
        let (plain, pl, pid) = make(false);
        let (sheet, sl, sid) = make(true);
        // UV に落ちていない（MapSize の道に入らない）
        assert_eq!(plain.generator_fallback(pl, pid).unwrap(), None);
        let fallback = sheet.generator_fallback(sl, sid).unwrap();
        assert!(
            !matches!(fallback, Some(Inactive::MapSize(_))) && fallback.is_none(),
            "{space:?} {:?}: {fallback:?}",
            g.kind
        );
        // 同じ 3D の点は、UDIM の有り無しで同じ値
        for y in 0..S {
            for x in 0..2 * S {
                assert_eq!(
                    sheet.composite_pixel(Channel::Color, x, y).unwrap(),
                    plain.composite_pixel(Channel::Color, x, y).unwrap(),
                    "{space:?} {:?} ({x}, {y})",
                    g.kind
                );
            }
        }
    }
}

const FONT: &[u8] = include_bytes!("../../../yolu-app/assets/fonts/BIZUDPGothic-Regular.ttf");

/// 3 つのタイル [1001, 1002, 1011]（2 × 2 の格子。1011 は左上のセル）のシート。返すのは 1011 のセルの左下の画素の位置。
fn three_tiles() -> (Document, f64, f64) {
    let mut doc = Document::with_tile_size(S, S, 8).unwrap();
    doc.enable_udim(vec![1001, 1002, 1011]).unwrap();
    let old = doc.udim().unwrap().clone();
    let r = old.cell_rect(old.cell_of_tile(1011).unwrap());
    (doc, f64::from(r.x), f64::from(r.y))
}

/// 1003 を足して並びを替える（1 回の Undo）。1011 が新しいセルへ動いた差（画素）を返す。
fn add_1003(doc: &mut Document, ox: f64, oy: f64) -> (f64, f64) {
    let steps = doc.undo_count();
    doc.sync_udim([1003]).unwrap().unwrap();
    assert_eq!(doc.undo_count(), steps + 1, "1 回の Undo");
    let new = doc.udim().unwrap().clone();
    assert_eq!(new.tiles(), &[1001, 1002, 1003, 1011]);
    let moved = new.cell_rect(new.cell_of_tile(1011).unwrap());
    let d = (f64::from(moved.x) - ox, f64::from(moved.y) - oy);
    assert_ne!(d, (0.0, 0.0), "1011 は別のセルへ動く");
    d
}

fn canvas_path(ox: f64, oy: f64, id: u128) -> yolu_core::paths::CanvasPath {
    use yolu_core::paths::{CanvasPoint, PathBrush};
    yolu_core::paths::CanvasPath {
        style: Default::default(),
        id,
        channel: Channel::Color,
        brush: PathBrush(
            yolu_core::BrushSettings {
                radius: 1.5,
                color: RED,
                ..Default::default()
            },
            yolu_core::PressureSwitches::default(),
        ),
        points: vec![
            CanvasPoint::new(ox + 3.5, oy + 4.25, 1.0).unwrap(),
            CanvasPoint::new(ox + 11.5, oy + 9.5, 1.0).unwrap(),
        ],
        material: None,
    }
}

fn path_of(doc: &Document, layer: yolu_core::LayerId) -> yolu_core::paths::CanvasPath {
    match doc.layer(layer).unwrap().path() {
        Some(yolu_core::LayerPath::Canvas(p)) => p.clone(),
        other => panic!("{other:?}"),
    }
}

/// 合わせて並びが変わると、2D のパスは中心の入るセルのタイルと一緒に新しいセルへ動き（描き直した画素も）、ほかのタイルのパスは動かない。
/// 1 回の Undo で戻る。
#[test]
fn syncing_moves_a_2d_path_with_its_tile_and_redraws_it() {
    use yolu_core::paths::render_canvas;
    let (mut doc, ox, oy) = three_tiles();
    let layer = doc.add_layer("パス").unwrap();
    let path = canvas_path(ox, oy, 0x77);
    doc.set_canvas_path(layer, path.clone()).unwrap();
    // 1001 のセルのパス（動かない）
    let still_layer = doc.add_layer("動かないパス").unwrap();
    let mut still = canvas_path(0.0, 0.0, 0x78);
    still.points = path
        .points
        .iter()
        .map(|p| yolu_core::paths::CanvasPoint::new(p.x - ox, p.y - oy, 1.0).unwrap())
        .collect();
    doc.set_canvas_path(still_layer, still.clone()).unwrap();
    let (dx, dy) = add_1003(&mut doc, ox, oy);
    let moved = path_of(&doc, layer);
    for (a, b) in moved.points.iter().zip(&path.points) {
        assert_eq!((a.x, a.y), (b.x + dx, b.y + dy));
    }
    // 画素: 動かした点から描き直したもので、全部 1011 の新しいセルの中
    let options = yolu_core::paths::Options {
        width: doc.width(),
        height: doc.height(),
        tile_size: 8,
        ..Default::default()
    };
    let drawn = render_canvas(&moved, &options).unwrap();
    let surface = doc.layer(layer).unwrap().surface(Channel::Color).unwrap();
    assert_eq!(
        surface.to_canvas_bytes(),
        drawn.channels[0].1.to_canvas_bytes()
    );
    let new = doc.udim().unwrap().clone();
    let cell = new.cell_rect(new.cell_of_tile(1011).unwrap());
    let mut painted = 0;
    for y in 0..doc.height() {
        for x in 0..doc.width() {
            if surface.pixel(x, y).unwrap().a > 0 {
                painted += 1;
                assert!(
                    (cell.x..cell.x + cell.width).contains(&x)
                        && (cell.y..cell.y + cell.height).contains(&y),
                    "({x}, {y}) は 1011 の新しいセルの外"
                );
            }
        }
    }
    assert!(painted > 0);
    assert_eq!(path_of(&doc, still_layer).points, still.points);
    doc.undo().unwrap();
    assert_eq!(doc.udim().unwrap().tiles(), &[1001, 1002, 1011]);
    assert_eq!(path_of(&doc, layer), path);
}

/// テキストは、基準の点が同じだけ動き、画素は新しい値から描いたものと同じ（次に文を直しても元の位置へ跳ばない）。Undo で戻る。
#[test]
fn syncing_moves_a_text_layer_with_its_tile() {
    use yolu_core::text::{self, TextFont, TextSettings};
    let (mut doc, ox, oy) = three_tiles();
    // 1 行目の上端がセルの上の縁に近い
    let mut t = TextSettings::new(
        "A",
        TextFont::Bundled("biz-udpgothic".into()),
        ox + 3.0,
        oy + S as f64 - 1.0,
    );
    t.size = 8.0;
    t.color = BLUE;
    let layer = doc
        .add_text_layer("文字", t.clone(), FONT, None, false)
        .unwrap();
    let (dx, dy) = add_1003(&mut doc, ox, oy);
    let moved = doc.layer(layer).unwrap().text().unwrap().clone();
    assert_eq!((moved.x, moved.y), (t.x + dx, t.y + dy));
    let rendered = text::render(&moved, FONT, 0, doc.width(), doc.height(), 8, u64::MAX)
        .unwrap()
        .canvas_bytes()
        .unwrap();
    let surface = doc.layer(layer).unwrap().surface(Channel::Color).unwrap();
    assert_eq!(surface.canvas_bytes().unwrap(), rendered);
    doc.undo().unwrap();
    assert_eq!(doc.layer(layer).unwrap().text(), Some(&t));
}

/// 2D の定規は、2 点の真ん中の入るセルのタイルと一緒に動く。Undo で戻る。
#[test]
fn syncing_moves_2d_rulers_with_their_tile() {
    use yolu_core::glam::DVec2;
    use yolu_core::rulers::{Ruler, RulerKind, RulerPlace};
    let (mut doc, ox, oy) = three_tiles();
    let layer = doc.add_layer("定規").unwrap();
    let id = doc.new_ruler_id();
    let ruler = Ruler::canvas(
        id,
        RulerKind::Line,
        DVec2::new(ox + 2.0, oy + 3.0),
        DVec2::new(ox + 12.0, oy + 9.0),
    );
    doc.set_rulers(layer, vec![ruler.clone()], false).unwrap();
    let (dx, dy) = add_1003(&mut doc, ox, oy);
    let RulerPlace::Canvas { a, b } = doc.layer(layer).unwrap().rulers()[0].place else {
        panic!("2D の定規");
    };
    assert_eq!(
        (a, b),
        (
            DVec2::new(ox + 2.0 + dx, oy + 3.0 + dy),
            DVec2::new(ox + 12.0 + dx, oy + 9.0 + dy)
        )
    );
    doc.undo().unwrap();
    assert_eq!(doc.layer(layer).unwrap().rulers()[0], ruler);
}
