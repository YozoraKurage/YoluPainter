//! 塗りつぶしのチャンネルの源にした、このテクスチャセットのベイクしたマップ: UV の既定の置き方は画素を直に（大きさが違えば 16 bit のまま
//! 双線形で）読む、ほかの投影は同じ画素の画像と同じ置き方で読む、覆いの無い所・焼いていない時は値、焼き直すと追従して古いキャッシュを
//! 使わない、古い・無いの理由、画像との入れ替えと 1 回の Undo、複製・スマートマテリアル・結合で写る、読み込みの決まりを断る。
use yolu_core::fill_image::{Projection, ProjectionMode, Wrap};
use yolu_core::generator::{Inactive, MapKind, MapState};
use yolu_core::smart::SmartPlacement;
use yolu_core::{
    Channel, CoreError, Document, EffectInputs, ImageColorSpace, ImageId, ImageInput,
    InactiveReason, InactiveTarget, LayerId, MapInput, ModelFrame, Rect, Rgba8,
};

const W: u32 = 32;
const H: u32 = 16;
const GRAY: Rgba8 = Rgba8::new(90, 90, 90, 255);
const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const IMAGE: ImageId = ImageId(0x1234_5678_9abc_4def_8123_4567_89ab_cdef);

/// 16 bit → 8 bit（`v / 257` を丸める）。
fn byte(v: u16) -> u8 {
    ((u32::from(v) + 128) / 257) as u8
}

/// 1 成分のマップ（画素ごとに違う値。`hole` の画素は覆いが無い）。
fn scalar_map(kind: MapKind, w: u32, h: u32, salt: u32, hole: Option<(u32, u32)>) -> MapInput {
    let mut data = Vec::new();
    let mut coverage = Vec::new();
    for y in 0..h {
        for x in 0..w {
            data.push(((x * 1931 + y * 7919 + salt * 4099) % 65536) as u16);
            coverage.push(u8::from(hole != Some((x, y))));
        }
    }
    MapInput::new(
        kind,
        w,
        h,
        data,
        coverage,
        [0.0; 3],
        [1.0; 3],
        KEY,
        MapState::Current,
    )
    .unwrap()
}

/// 3 成分のマップ（World Normal の形の値）。
fn color_map(kind: MapKind, w: u32, h: u32) -> MapInput {
    let mut data = Vec::new();
    for y in 0..h {
        for x in 0..w {
            data.extend([
                ((x * 2053) % 65536) as u16,
                ((y * 4111) % 65536) as u16,
                ((x * y * 997 + 12345) % 65536) as u16,
            ]);
        }
    }
    MapInput::new(
        kind,
        w,
        h,
        data,
        vec![1; (w * h) as usize],
        [0.0; 3],
        [1.0; 3],
        KEY,
        MapState::Current,
    )
    .unwrap()
}

/// 位置（モデルの x・y が UV に沿う平らな板）と法線（+z）のマップ。投影を置くための入力。
fn placement_maps(inputs: EffectInputs) -> EffectInputs {
    let mut position = Vec::new();
    let mut normal = Vec::new();
    for y in 0..H {
        for x in 0..W {
            position.extend([
                ((x as f64 + 0.5) / W as f64 * 65535.0).round() as u16,
                ((y as f64 + 0.5) / H as f64 * 65535.0).round() as u16,
                32768,
            ]);
            normal.extend([32768, 32768, 65535]);
        }
    }
    let n = (W * H) as usize;
    inputs
        .with_map(
            MapInput::new(
                MapKind::Position,
                W,
                H,
                position,
                vec![1; n],
                [0.0; 3],
                [1.0; 3],
                KEY,
                MapState::Current,
            )
            .unwrap(),
        )
        .unwrap()
        .with_map(
            MapInput::new(
                MapKind::WorldNormal,
                W,
                H,
                normal,
                vec![1; n],
                [0.0; 3],
                [1.0; 3],
                KEY,
                MapState::Current,
            )
            .unwrap(),
        )
        .unwrap()
        .with_frame(Some(
            ModelFrame::new([0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap(),
        ))
}

fn fill_doc(inputs: EffectInputs) -> (Document, LayerId) {
    let mut doc = Document::with_tile_size(W, H, 8).unwrap();
    let fill = doc
        .add_fill_layer(
            "塗り",
            &[
                (Channel::Color, GRAY),
                (Channel::Roughness, Rgba8::new(40, 40, 40, 255)),
            ],
            None,
        )
        .unwrap();
    doc.set_effect_inputs(inputs).unwrap();
    (doc, fill)
}

fn whole(doc: &Document, c: Channel) -> Vec<u8> {
    doc.composite_channel(c, Rect::new(0, 0, W, H)).unwrap()
}

fn px(doc: &Document, c: Channel, x: u32, y: u32) -> Rgba8 {
    doc.composite_pixel(c, x, y).unwrap()
}

/// 8 bit にしたマップの画素（覆いの無い画素は透明）。同じ画素の画像の源と比べる。
fn as_image(m: &MapInput) -> ImageInput {
    let n = m.kind.channels();
    let mut pixels = Vec::new();
    for i in 0..(m.width * m.height) as usize {
        if m.coverage[i] == 0 {
            pixels.extend([0, 0, 0, 0]);
            continue;
        }
        let v = |k: usize| byte(m.data[i * n + k.min(n - 1)]);
        pixels.extend([v(0), v(1), v(2), 255]);
    }
    ImageInput::new(m.width, m.height, pixels, ImageColorSpace::Unspecified).unwrap()
}

fn fill_maps(doc: &Document, layer: LayerId) -> Vec<(Channel, MapKind)> {
    doc.layer(layer).unwrap().fill_maps().collect()
}

#[test]
fn a_same_size_map_is_read_pixel_by_pixel_and_uncovered_pixels_show_the_value() {
    let ao = scalar_map(MapKind::AmbientOcclusion, W, H, 1, Some((3, 2)));
    let (mut doc, fill) = fill_doc(EffectInputs::new().with_map(ao.clone()).unwrap());
    doc.set_fill_map(fill, Channel::Color, Some(MapKind::AmbientOcclusion))
        .unwrap();
    assert_eq!(
        fill_maps(&doc, fill),
        vec![(Channel::Color, MapKind::AmbientOcclusion)]
    );
    for y in 0..H {
        for x in 0..W {
            let expect = if (x, y) == (3, 2) {
                GRAY
            } else {
                let v = byte(ao.data[(y * W + x) as usize]);
                Rgba8::new(v, v, v, 255)
            };
            assert_eq!(px(&doc, Channel::Color, x, y), expect, "({x},{y})");
        }
    }
    assert!(doc.inactive_effect_list().is_empty());
}

#[test]
fn a_three_component_map_is_rgb_and_a_scalar_channel_reads_its_luminance() {
    let normal = color_map(MapKind::TangentNormal, W, H);
    let (mut doc, fill) = fill_doc(EffectInputs::new().with_map(normal.clone()).unwrap());
    doc.set_fill_map(fill, Channel::Color, Some(MapKind::TangentNormal))
        .unwrap();
    doc.set_fill_map(fill, Channel::Roughness, Some(MapKind::TangentNormal))
        .unwrap();
    for (x, y) in [(0, 0), (5, 3), (31, 15), (17, 9)] {
        let i = (y * W + x) as usize * 3;
        let [r, g, b] = [0, 1, 2].map(|k| byte(normal.data[i + k]));
        // 値はデータのまま（カラーのチャンネルでも sRGB に直さない）
        assert_eq!(
            px(&doc, Channel::Color, x, y),
            Rgba8::new(r, g, b, 255),
            "({x},{y})"
        );
        let l =
            ((2126 * u32::from(r) + 7152 * u32::from(g) + 722 * u32::from(b) + 5000) / 10000) as u8;
        assert_eq!(
            px(&doc, Channel::Roughness, x, y),
            Rgba8::new(l, l, l, 255),
            "({x},{y})"
        );
    }
}

#[test]
fn a_map_of_another_size_is_sampled_bilinearly_in_sixteen_bits() {
    // 文書の半分の大きさ: 文書の画素の中心をマップの画素へ写し、16 bit のまま双線形に引いてから丸める
    let (mw, mh) = (W / 2, H / 2);
    let height = scalar_map(MapKind::Height, mw, mh, 7, None);
    let (mut doc, fill) = fill_doc(EffectInputs::new().with_map(height.clone()).unwrap());
    doc.set_fill_map(fill, Channel::Roughness, Some(MapKind::Height))
        .unwrap();
    let at = |x: i64, y: i64| {
        let x = x.clamp(0, i64::from(mw) - 1) as u32;
        let y = y.clamp(0, i64::from(mh) - 1) as u32;
        f64::from(height.data[(y * mw + x) as usize])
    };
    for y in 0..H {
        for x in 0..W {
            let fx = (f64::from(x) + 0.5) * 0.5 - 0.5;
            let fy = (f64::from(y) + 0.5) * 0.5 - 0.5;
            let (x0, y0) = (fx.floor(), fy.floor());
            let (tx, ty) = (fx - x0, fy - y0);
            let (x0, y0) = (x0 as i64, y0 as i64);
            let v = at(x0, y0) * (1.0 - tx) * (1.0 - ty)
                + at(x0 + 1, y0) * tx * (1.0 - ty)
                + at(x0, y0 + 1) * (1.0 - tx) * ty
                + at(x0 + 1, y0 + 1) * tx * ty;
            let b = (v / 257.0).round() as u8;
            assert_eq!(
                px(&doc, Channel::Roughness, x, y),
                Rgba8::new(b, b, b, 255),
                "({x},{y})"
            );
        }
    }
}

/// UV の置き方を変えた・UV 以外の投影は、同じ画素の画像を同じ投影で置いたときと同じ画素になる。
#[test]
fn other_projections_place_the_map_like_an_image_of_the_same_pixels() {
    let mut tiled = Projection {
        tiles: [2.0, 3.0],
        rotation: 30.0,
        ..Projection::default()
    };
    tiled.wrap = Wrap::Repeat;
    let triplanar = Projection {
        mode: ProjectionMode::Triplanar,
        ..Projection::default()
    };
    let planar = Projection {
        mode: ProjectionMode::Planar,
        wrap: Wrap::Clamp,
        ..Projection::default()
    };
    let curvature = scalar_map(MapKind::Curvature, 24, 20, 3, None);
    let inputs = placement_maps(
        EffectInputs::new()
            .with_map(curvature.clone())
            .unwrap()
            .with_image(IMAGE, as_image(&curvature)),
    );
    for projection in [tiled, triplanar, planar] {
        let (mut by_map, a) = fill_doc(inputs.clone());
        by_map.set_fill_projection(a, projection, false).unwrap();
        by_map
            .set_fill_map(a, Channel::Color, Some(MapKind::Curvature))
            .unwrap();
        let (mut by_image, b) = fill_doc(inputs.clone());
        by_image.set_fill_projection(b, projection, false).unwrap();
        by_image
            .set_fill_image(b, Channel::Color, Some(IMAGE))
            .unwrap();
        assert_eq!(
            whole(&by_map, Channel::Color),
            whole(&by_image, Channel::Color),
            "{:?}",
            projection.mode
        );
        assert!(
            whole(&by_map, Channel::Color)
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| *p != GRAY.to_array()),
            "{:?} で値のまま",
            projection.mode
        );
        assert!(by_map.inactive_effect_list().is_empty());
    }
}

#[test]
fn where_a_projected_map_does_not_reach_the_channel_shows_its_value() {
    // 繰り返さない置き方で外に出た画素・覆いの無いマップの画素は、透明でなく値
    let shifted = Projection {
        offset: [0.5, 0.0],
        wrap: Wrap::None,
        ..Projection::default()
    };
    let thickness = scalar_map(MapKind::Thickness, W, H, 5, Some((20, 4)));
    let (mut doc, fill) = fill_doc(EffectInputs::new().with_map(thickness).unwrap());
    doc.set_fill_projection(fill, shifted, false).unwrap();
    doc.set_fill_map(fill, Channel::Color, Some(MapKind::Thickness))
        .unwrap();
    let pixels = whole(&doc, Channel::Color);
    assert!(pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    assert!(pixels
        .as_chunks::<4>()
        .0
        .iter()
        .any(|p| *p == GRAY.to_array()));
    assert!(pixels
        .as_chunks::<4>()
        .0
        .iter()
        .any(|p| *p != GRAY.to_array()));
}

#[test]
fn rebaking_follows_the_new_map_and_never_reuses_the_old_result() {
    let planar = Projection {
        mode: ProjectionMode::Planar,
        ..Projection::default()
    };
    for projection in [Projection::default(), planar] {
        let first = scalar_map(MapKind::AmbientOcclusion, W, H, 1, None);
        let (mut doc, fill) =
            fill_doc(placement_maps(EffectInputs::new().with_map(first).unwrap()));
        doc.set_fill_projection(fill, projection, false).unwrap();
        doc.set_fill_map(fill, Channel::Color, Some(MapKind::AmbientOcclusion))
            .unwrap();
        let before = whole(&doc, Channel::Color);
        let built = doc.effect_counters().mip_chains_built;
        // 同じ種類のマップを焼き直す（値が替わる）
        let second = scalar_map(MapKind::AmbientOcclusion, W, H, 2, None);
        doc.set_effect_inputs(placement_maps(
            EffectInputs::new().with_map(second.clone()).unwrap(),
        ))
        .unwrap();
        let after = whole(&doc, Channel::Color);
        assert_ne!(before, after, "{:?}", projection.mode);
        // 新しい文書に新しいマップを渡した結果と同じ（前のマップの結果・ミップマップを使っていない）
        let (mut fresh, f) = fill_doc(placement_maps(
            EffectInputs::new().with_map(second).unwrap(),
        ));
        fresh.set_fill_projection(f, projection, false).unwrap();
        fresh
            .set_fill_map(f, Channel::Color, Some(MapKind::AmbientOcclusion))
            .unwrap();
        assert_eq!(
            after,
            whole(&fresh, Channel::Color),
            "{:?}",
            projection.mode
        );
        if projection.mode == ProjectionMode::Uv {
            // 既定の UV の置き方はミップマップを作らない
            assert_eq!(doc.effect_counters().mip_chains_built, 0);
        } else {
            assert_eq!(doc.effect_counters().mip_chains_built, built + 1);
            // ほかの入力（モデルのルート）だけが替わり、同じ画素のマップを渡し直したときは、ミップマップを作り直さない
            let again = scalar_map(MapKind::AmbientOcclusion, W, H, 2, None);
            let inputs = placement_maps(EffectInputs::new().with_map(again).unwrap()).with_frame(
                Some(ModelFrame::new([0.0, 0.0, 0.01], [0.0, 0.0, 0.0, 1.0]).unwrap()),
            );
            doc.set_effect_inputs(inputs).unwrap();
            let _ = whole(&doc, Channel::Color);
            assert_eq!(doc.effect_counters().mip_chains_built, built + 1);
        }
    }
}

#[test]
fn a_missing_map_shows_the_value_and_a_stale_map_is_used_as_it_is_both_with_a_reason() {
    let (mut doc, fill) = fill_doc(EffectInputs::new());
    doc.set_fill_map(fill, Channel::Color, Some(MapKind::Curvature))
        .unwrap();
    assert!(whole(&doc, Channel::Color)
        .as_chunks::<4>()
        .0
        .iter()
        .all(|p| *p == GRAY.to_array()));
    let list = doc.inactive_effect_list();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].target, InactiveTarget::FillMap(Channel::Color));
    assert_eq!(
        list[0].reason,
        InactiveReason::Generator(Inactive::MissingMap(MapKind::Curvature))
    );
    // 古い・照合できないマップは値をそのまま使い、理由も言う
    for (state, why) in [
        (MapState::Stale, Inactive::StaleMap(MapKind::Curvature)),
        (
            MapState::Unverified,
            Inactive::UnverifiedMap(MapKind::Curvature),
        ),
    ] {
        let mut map = scalar_map(MapKind::Curvature, W, H, 9, None);
        map.state = state;
        let v = byte(map.data[0]);
        doc.set_effect_inputs(EffectInputs::new().with_map(map).unwrap())
            .unwrap();
        assert_eq!(px(&doc, Channel::Color, 0, 0), Rgba8::new(v, v, v, 255));
        let list = doc.inactive_effect_list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].target, InactiveTarget::FillMap(Channel::Color));
        assert_eq!(list[0].reason, InactiveReason::Generator(why));
    }
    // 無効にしたチャンネルは言わない
    doc.set_channel_enabled(fill, Channel::Color, false)
        .unwrap();
    assert!(doc.inactive_effect_list().is_empty());
}

#[test]
fn a_projection_that_cannot_be_placed_says_why_and_shows_the_value() {
    let (mut doc, fill) = fill_doc(
        EffectInputs::new()
            .with_map(scalar_map(MapKind::Height, W, H, 1, None))
            .unwrap(),
    );
    let triplanar = Projection {
        mode: ProjectionMode::Triplanar,
        ..Projection::default()
    };
    doc.set_fill_projection(fill, triplanar, false).unwrap();
    doc.set_fill_map(fill, Channel::Color, Some(MapKind::Height))
        .unwrap();
    assert!(whole(&doc, Channel::Color)
        .as_chunks::<4>()
        .0
        .iter()
        .all(|p| *p == GRAY.to_array()));
    let list = doc.inactive_effect_list();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].target, InactiveTarget::FillMap(Channel::Color));
    assert_eq!(
        list[0].reason,
        InactiveReason::Generator(Inactive::MissingMap(MapKind::Position))
    );
}

#[test]
fn image_and_map_replace_each_other_on_a_channel_and_each_change_is_one_undo() {
    let ao = scalar_map(MapKind::AmbientOcclusion, W, H, 4, None);
    let inputs = EffectInputs::new()
        .with_map(ao.clone())
        .unwrap()
        .with_image(IMAGE, as_image(&ao));
    let mut doc = Document::with_tile_size(W, H, 8).unwrap();
    // 値の無いチャンネルに置くと既定の値を持って有効になる
    let fill = doc
        .add_fill_layer("塗り", &[(Channel::Roughness, GRAY)], None)
        .unwrap();
    doc.set_effect_inputs(inputs).unwrap();
    doc.set_fill_map(fill, Channel::Color, Some(MapKind::AmbientOcclusion))
        .unwrap();
    let layer = doc.layer(fill).unwrap();
    assert!(layer.fill_value(Channel::Color).is_some());
    assert!(layer.is_channel_enabled(Channel::Color));
    // 画像を置くとマップは外れ、Undo で戻る
    doc.set_fill_image(fill, Channel::Color, Some(IMAGE))
        .unwrap();
    let layer = doc.layer(fill).unwrap();
    assert_eq!(layer.fill_image(Channel::Color), Some(IMAGE));
    assert_eq!(layer.fill_map(Channel::Color), None);
    doc.undo().unwrap();
    let layer = doc.layer(fill).unwrap();
    assert_eq!(layer.fill_image(Channel::Color), None);
    assert_eq!(
        layer.fill_map(Channel::Color),
        Some(MapKind::AmbientOcclusion)
    );
    // マップを置くと画像は外れ、Undo で画像に戻る
    doc.redo().unwrap();
    doc.set_fill_map(fill, Channel::Color, Some(MapKind::Height))
        .unwrap();
    let layer = doc.layer(fill).unwrap();
    assert_eq!(layer.fill_image(Channel::Color), None);
    assert_eq!(layer.fill_map(Channel::Color), Some(MapKind::Height));
    doc.undo().unwrap();
    assert_eq!(
        doc.layer(fill).unwrap().fill_image(Channel::Color),
        Some(IMAGE)
    );
    assert_eq!(doc.layer(fill).unwrap().fill_map(Channel::Color), None);
    // 外すと値は残る
    doc.set_fill_map(fill, Channel::Roughness, Some(MapKind::AmbientOcclusion))
        .unwrap();
    doc.set_fill_map(fill, Channel::Roughness, None).unwrap();
    let layer = doc.layer(fill).unwrap();
    assert_eq!(layer.fill_map(Channel::Roughness), None);
    assert_eq!(layer.fill_value(Channel::Roughness), Some(GRAY));
    // 値を消すとマップも外れ、1 回の Undo で両方戻る
    doc.set_fill_map(fill, Channel::Roughness, Some(MapKind::AmbientOcclusion))
        .unwrap();
    doc.set_fill_value(fill, Channel::Roughness, None, false)
        .unwrap();
    assert_eq!(doc.layer(fill).unwrap().fill_map(Channel::Roughness), None);
    doc.undo().unwrap();
    let layer = doc.layer(fill).unwrap();
    assert_eq!(
        layer.fill_map(Channel::Roughness),
        Some(MapKind::AmbientOcclusion)
    );
    assert_eq!(layer.fill_value(Channel::Roughness), Some(GRAY));
}

#[test]
fn a_gradient_or_point_gradient_replaces_the_map_and_maps_need_a_fill_layer() {
    let (mut doc, fill) = fill_doc(EffectInputs::new());
    doc.set_fill_map(fill, Channel::Color, Some(MapKind::Curvature))
        .unwrap();
    let points = yolu_core::fill_points::PointGradient {
        space: yolu_core::fill_points::PointSpace::Uv,
        spread: 0.0,
        points: vec![yolu_core::fill_points::GradientPoint {
            position: [0.5, 0.5, 0.0],
            color: Rgba8::new(1, 2, 3, 255),
        }],
    };
    doc.set_fill_points(fill, Channel::Color, Some(points), false)
        .unwrap();
    assert_eq!(doc.layer(fill).unwrap().fill_map(Channel::Color), None);
    doc.undo().unwrap();
    assert_eq!(
        doc.layer(fill).unwrap().fill_map(Channel::Color),
        Some(MapKind::Curvature)
    );
    let raster = doc.add_layer("ラスター").unwrap();
    assert!(matches!(
        doc.set_fill_map(raster, Channel::Color, Some(MapKind::Curvature)),
        Err(CoreError::Unsupported(_))
    ));
}

#[test]
fn duplicating_placing_a_smart_material_and_merging_carry_the_map_source() {
    let ao = scalar_map(MapKind::AmbientOcclusion, W, H, 6, Some((1, 1)));
    let (mut doc, fill) = fill_doc(EffectInputs::new().with_map(ao).unwrap());
    doc.set_fill_map(fill, Channel::Color, Some(MapKind::AmbientOcclusion))
        .unwrap();
    // 複製
    let copy = doc.duplicate_layer(fill, None).unwrap();
    assert_eq!(fill_maps(&doc, copy), fill_maps(&doc, fill));
    doc.undo().unwrap();
    // スマートマテリアル（置いた先のテクスチャセットのマップを読む）
    let material = doc.capture_smart_material(&[fill], "素材").unwrap();
    let other = scalar_map(MapKind::AmbientOcclusion, W, H, 8, None);
    let mut target = Document::with_tile_size(W, H, 8).unwrap();
    target
        .set_effect_inputs(EffectInputs::new().with_map(other.clone()).unwrap())
        .unwrap();
    let placed = target
        .place_smart_material(&material, &SmartPlacement::default())
        .unwrap();
    assert_eq!(
        fill_maps(&target, placed.layer_id),
        vec![(Channel::Color, MapKind::AmbientOcclusion)]
    );
    let v = byte(other.data[(3 * W + 5) as usize]);
    assert_eq!(px(&target, Channel::Color, 5, 3), Rgba8::new(v, v, v, 255));
    // 下のラスターレイヤーへ結合すると、評価した画素が残る
    let expect = whole(&doc, Channel::Color);
    let base = doc.add_layer_above("下", None).unwrap();
    doc.move_layer(base, 0).unwrap();
    doc.merge_down(fill, 0).unwrap();
    assert_eq!(whole(&doc, Channel::Color), expect);
}

#[test]
fn loading_refuses_maps_that_break_the_rules() {
    let (mut doc, fill) = fill_doc(EffectInputs::new());
    // 値の無いチャンネル・同じチャンネルの 2 つ目
    for maps in [
        vec![(Channel::Height, MapKind::Height)],
        vec![
            (Channel::Color, MapKind::Height),
            (Channel::Color, MapKind::Curvature),
        ],
    ] {
        assert!(matches!(
            doc.set_fill_maps_for_load(fill, &maps),
            Err(CoreError::InvalidArgument(_))
        ));
    }
    // 画像のあるチャンネル
    doc.set_fill_images_for_load(fill, &[(Channel::Color, IMAGE)], Projection::default())
        .unwrap();
    assert!(doc
        .set_fill_maps_for_load(fill, &[(Channel::Color, MapKind::Height)])
        .is_err());
    doc.set_fill_maps_for_load(fill, &[(Channel::Roughness, MapKind::Height)])
        .unwrap();
    assert_eq!(
        fill_maps(&doc, fill),
        vec![(Channel::Roughness, MapKind::Height)]
    );
    // マップのあるチャンネルに画像は置けない（読み込み）
    assert!(doc
        .set_fill_images_for_load(fill, &[(Channel::Roughness, IMAGE)], Projection::default())
        .is_err());
}

/// 投影したマップのミップマップは、文書の全部のブロックで 1 つを使う（ブロックごとに作り直さない）。
#[test]
fn a_projected_map_builds_its_mip_chain_once_for_all_blocks() {
    let n = 512;
    let mut doc = Document::with_tile_size(n, n, 64).unwrap();
    doc.set_filter_block_pixels(128).unwrap();
    let fill = doc
        .add_fill_layer("塗り", &[(Channel::Color, GRAY)], None)
        .unwrap();
    doc.set_effect_inputs(
        EffectInputs::new()
            .with_map(scalar_map(MapKind::AmbientOcclusion, 64, 64, 1, None))
            .unwrap(),
    )
    .unwrap();
    let tiled = Projection {
        tiles: [3.0, 3.0],
        ..Projection::default()
    };
    doc.set_fill_projection(fill, tiled, false).unwrap();
    doc.set_fill_map(fill, Channel::Color, Some(MapKind::AmbientOcclusion))
        .unwrap();
    doc.composite_channel(Channel::Color, Rect::new(0, 0, n, n))
        .unwrap();
    assert_eq!(doc.effect_counters().mip_chains_built, 1);
}
