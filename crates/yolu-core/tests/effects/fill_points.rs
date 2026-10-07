//! 塗りつぶしの点のグラデーション: 点 1 つは単色、2 点の間は滑らか、モデルの空間は UV の継ぎ目をまたいで続く、UV の空間はマップが
//! 要らない、上限と範囲の外は断る、位置のマップが無いモデルの空間は値を見せて理由を言う、1 回の Undo で戻る。
use yolu_core::fill_points::{GradientPoint, PointGradient, PointSpace, MAX_POINTS};
use yolu_core::generator::{Inactive, MapKind, MapState};
use yolu_core::{
    Channel, CoreError, Document, EffectInputs, InactiveReason, InactiveTarget, LayerId, MapInput,
    ModelFrame, Rect, Rgba8,
};

const W: u32 = 64;
const H: u32 = 32;
const RED: Rgba8 = Rgba8::new(255, 0, 0, 255);
const BLUE: Rgba8 = Rgba8::new(0, 0, 255, 255);
const GRAY: Rgba8 = Rgba8::new(90, 90, 90, 255);

/// 継ぎ目のある位置のマップ: UV の左半分はモデルの x 0〜0.5 を左から右へ、右半分は x 1〜0.5 を左から右へ（右端の列と左半分の
/// 右端の列が 3D で隣り合う）。y はモデルの y 0〜1、z は 0。
fn seam_inputs() -> EffectInputs {
    let mut data = Vec::new();
    let mut coverage = Vec::new();
    for y in 0..H {
        for x in 0..W {
            let mx = if x < W / 2 {
                (x as f64 + 0.5) / W as f64
            } else {
                1.0 - (x as f64 - W as f64 / 2.0 + 0.5) / W as f64
            };
            let my = (y as f64 + 0.5) / H as f64;
            data.extend([
                (mx * 65535.0).round() as u16,
                (my * 65535.0).round() as u16,
                0,
            ]);
            coverage.push(1);
        }
    }
    EffectInputs::new()
        .with_map(
            MapInput::new(
                MapKind::Position,
                W,
                H,
                data,
                coverage,
                [0.0; 3],
                [1.0, 1.0, 0.0],
                &"a".repeat(64),
                MapState::Current,
            )
            .unwrap(),
        )
        .unwrap()
        .with_frame(Some(
            ModelFrame::new([0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap(),
        ))
}

fn fill_doc() -> (Document, LayerId) {
    let mut doc = Document::with_tile_size(W, H, 16).unwrap();
    let fill = doc
        .add_fill_layer("塗り", &[(Channel::Color, GRAY)], None)
        .unwrap();
    doc.set_effect_inputs(seam_inputs()).unwrap();
    (doc, fill)
}

fn gradient(space: PointSpace, points: &[([f64; 3], Rgba8)]) -> PointGradient {
    PointGradient {
        space,
        spread: 0.0,
        points: points
            .iter()
            .map(|(position, color)| GradientPoint {
                position: *position,
                color: *color,
            })
            .collect(),
    }
}

fn px(doc: &Document, x: u32, y: u32) -> Rgba8 {
    doc.composite_pixel(Channel::Color, x, y).unwrap()
}

fn whole(doc: &Document) -> Vec<u8> {
    doc.composite_channel(Channel::Color, Rect::new(0, 0, W, H))
        .unwrap()
}

#[test]
fn one_point_paints_one_colour_in_either_space() {
    for space in [PointSpace::Model, PointSpace::Uv] {
        let (mut doc, fill) = fill_doc();
        let at = if space == PointSpace::Uv {
            [0.3, 0.6, 0.0]
        } else {
            [0.2, 0.4, 0.0]
        };
        let colour = Rgba8::new(10, 200, 30, 180);
        doc.set_fill_points(
            fill,
            Channel::Color,
            Some(gradient(space, &[(at, colour)])),
            false,
        )
        .unwrap();
        for y in [0, 13, 31] {
            for x in [0, 20, 40, 63] {
                assert_eq!(px(&doc, x, y), colour, "{space:?} ({x},{y})");
            }
        }
    }
}

#[test]
fn two_points_blend_smoothly_between_them_in_uv_space() {
    let (mut doc, fill) = fill_doc();
    let g = gradient(
        PointSpace::Uv,
        &[([0.0, 0.5, 0.0], RED), ([1.0, 0.5, 0.0], BLUE)],
    );
    doc.set_fill_points(fill, Channel::Color, Some(g), false)
        .unwrap();
    let row: Vec<Rgba8> = (0..W).map(|x| px(&doc, x, H / 2)).collect();
    assert!(row[0].r > 240 && row[0].b < 15, "{:?}", row[0]);
    assert!(row[63].b > 240 && row[63].r < 15, "{:?}", row[63]);
    // 左から右へ、赤は減り青は増えるだけ（段が無い）
    for w in row.windows(2) {
        assert!(
            w[1].r <= w[0].r && w[1].b >= w[0].b,
            "{:?} → {:?}",
            w[0],
            w[1]
        );
        assert!(
            w[0].r.abs_diff(w[1].r) < 40,
            "跳ばない: {:?} → {:?}",
            w[0],
            w[1]
        );
    }
    // 真ん中は半々
    let mid = px(&doc, 31, H / 2);
    assert!(mid.r.abs_diff(mid.b) < 12, "{mid:?}");
    // 広がりを上げると、点の上でもほかの点の色が混ざる（丸みが増える）
    let mut wide = doc
        .layer(fill)
        .unwrap()
        .fill_points(Channel::Color)
        .unwrap()
        .clone();
    wide.spread = 0.8;
    doc.set_fill_points(fill, Channel::Color, Some(wide), false)
        .unwrap();
    let edge = px(&doc, 0, H / 2);
    assert!(edge.r < 220 && edge.b > 30, "{edge:?}");
}

#[test]
fn model_space_continues_across_a_uv_seam_where_uv_space_does_not() {
    let points = [([0.0, 0.5, 0.0], RED), ([1.0, 0.5, 0.0], BLUE)];
    let (mut doc, fill) = fill_doc();
    doc.set_fill_points(
        fill,
        Channel::Color,
        Some(gradient(PointSpace::Model, &points)),
        false,
    )
    .unwrap();
    // UV の左半分の右端の列と、右半分の右端の列は 3D で隣り合う（モデルの x ≈ 0.492 と 0.508）: その差は、UV の上で隣り合う列
    // （3D でも同じだけ離れている）の差と同じくらい
    let a = px(&doc, W / 2 - 1, 10);
    let b = px(&doc, W - 1, 10);
    let neighbour = px(&doc, W / 2 - 2, 10);
    let step = neighbour.r.abs_diff(a.r).max(neighbour.b.abs_diff(a.b));
    assert!(
        a.r.abs_diff(b.r) <= step + 2 && a.b.abs_diff(b.b) <= step + 2,
        "継ぎ目をまたいで続く: {a:?} {b:?}（隣の列の差 {step}）"
    );
    // UV の上で隣り合う列（左半分の右端と右半分の左端）は 3D で離れていて、色も離れる
    let c = px(&doc, W / 2, 10);
    assert!(c.b > a.b + 60, "{a:?} {c:?}");
    // UV の空間では、継ぎ目の両側（UV で離れた列）は違う色
    doc.set_fill_points(
        fill,
        Channel::Color,
        Some(gradient(
            PointSpace::Uv,
            &[([0.0, 0.5, 0.0], RED), ([1.0, 0.5, 0.0], BLUE)],
        )),
        false,
    )
    .unwrap();
    let a = px(&doc, W / 2 - 1, 10);
    let b = px(&doc, W - 1, 10);
    assert!(b.b > a.b + 60, "UV の空間は UV の距離: {a:?} {b:?}");
}

#[test]
fn model_space_without_a_position_map_shows_the_value_and_says_why() {
    let (mut doc, fill) = fill_doc();
    doc.set_effect_inputs(EffectInputs::new()).unwrap();
    doc.set_fill_points(
        fill,
        Channel::Color,
        Some(gradient(PointSpace::Model, &[([0.5; 3], RED)])),
        false,
    )
    .unwrap();
    assert_eq!(px(&doc, 5, 5), GRAY, "値を見せる");
    let reason = InactiveReason::Generator(Inactive::MissingMap(MapKind::Position));
    assert_eq!(
        doc.fill_points_inactive(fill, Channel::Color).unwrap(),
        Some(reason.clone())
    );
    let list = doc.inactive_effect_list();
    assert!(
        list.iter().any(|e| e.layer == fill
            && e.target == InactiveTarget::FillPoints(Channel::Color)
            && e.reason == reason),
        "{list:?}"
    );
    // UV の空間はマップが要らない
    doc.set_fill_points(
        fill,
        Channel::Color,
        Some(gradient(PointSpace::Uv, &[([0.5, 0.5, 0.0], RED)])),
        false,
    )
    .unwrap();
    assert_eq!(px(&doc, 5, 5), RED);
    assert!(doc.inactive_effect_list().is_empty());
}

#[test]
fn limits_and_ranges_are_refused_without_changes() {
    let (mut doc, fill) = fill_doc();
    let before = whole(&doc);
    let steps = doc.undo_count();
    let many: Vec<([f64; 3], Rgba8)> = (0..=MAX_POINTS)
        .map(|i| ([i as f64 / 100.0, 0.5, 0.0], RED))
        .collect();
    let bad = [
        gradient(PointSpace::Uv, &many),
        gradient(PointSpace::Uv, &[]),
        PointGradient {
            spread: 1.5,
            ..gradient(PointSpace::Uv, &[([0.5, 0.5, 0.0], RED)])
        },
        gradient(PointSpace::Model, &[([f64::NAN, 0.0, 0.0], RED)]),
        gradient(PointSpace::Model, &[([2e6, 0.0, 0.0], RED)]),
        gradient(PointSpace::Uv, &[([0.5, 0.5, 0.3], RED)]),
    ];
    for g in bad {
        assert!(matches!(
            doc.set_fill_points(fill, Channel::Color, Some(g.clone()), false),
            Err(CoreError::InvalidArgument(_))
        ));
    }
    // 上限ちょうどは置ける
    doc.set_fill_points(
        fill,
        Channel::Color,
        Some(gradient(PointSpace::Uv, &many[..MAX_POINTS])),
        false,
    )
    .unwrap();
    doc.undo().unwrap();
    // 法線のチャンネルには置けない
    let normal = doc
        .add_fill_layer(
            "法線",
            &[(Channel::Normal, Rgba8::new(128, 128, 255, 255))],
            None,
        )
        .unwrap();
    assert!(matches!(
        doc.set_fill_points(
            normal,
            Channel::Normal,
            Some(gradient(PointSpace::Uv, &[([0.5, 0.5, 0.0], RED)])),
            false
        ),
        Err(CoreError::Unsupported(_))
    ));
    doc.undo().unwrap();
    assert_eq!(doc.undo_count(), steps);
    assert_eq!(whole(&doc), before);
}

#[test]
fn placing_dragging_and_removing_points_undo_in_one_step_each() {
    let (mut doc, fill) = fill_doc();
    let before = whole(&doc);
    let steps = doc.undo_count();
    let mut g = gradient(PointSpace::Uv, &[([0.2, 0.5, 0.0], RED)]);
    doc.set_fill_points(fill, Channel::Color, Some(g.clone()), false)
        .unwrap();
    assert_eq!(doc.undo_count(), steps + 1);
    // 点のドラッグ（まとめる）は 1 回の Undo
    g.points.push(GradientPoint {
        position: [0.8, 0.5, 0.0],
        color: BLUE,
    });
    doc.set_fill_points(fill, Channel::Color, Some(g.clone()), false)
        .unwrap();
    let placed = whole(&doc);
    for k in 1..=5 {
        g.points[1].position[0] = 0.8 - k as f64 * 0.05;
        doc.set_fill_points(fill, Channel::Color, Some(g.clone()), true)
            .unwrap();
    }
    doc.end_coalescing();
    assert_eq!(doc.undo_count(), steps + 3);
    assert_ne!(whole(&doc), placed);
    doc.undo().unwrap();
    assert_eq!(whole(&doc), placed);
    // 外すと値に戻る
    doc.set_fill_points(fill, Channel::Color, None, false)
        .unwrap();
    assert_eq!(whole(&doc), before);
    assert!(doc
        .layer(fill)
        .unwrap()
        .fill_points(Channel::Color)
        .is_none());
    doc.undo().unwrap();
    assert_eq!(whole(&doc), placed);
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert_eq!(whole(&doc), before);
    assert_eq!(doc.undo_count(), steps);
}

#[test]
fn points_replace_the_shape_gradient_of_the_channel_in_one_undo() {
    let (mut doc, fill) = fill_doc();
    let mut shape = yolu_core::generator::Settings::new(yolu_core::generator::Kind::ShapeGradient);
    shape.ramp = Some(yolu_core::generator::Ramp::default());
    shape.blend = yolu_core::generator::Blend::Replace;
    doc.set_fill_gradient(fill, Channel::Color, Some(shape.clone()), false)
        .unwrap();
    let with_shape = whole(&doc);
    doc.set_fill_points(
        fill,
        Channel::Color,
        Some(gradient(PointSpace::Uv, &[([0.5, 0.5, 0.0], RED)])),
        false,
    )
    .unwrap();
    let layer = doc.layer(fill).unwrap();
    assert!(layer.fill_gradient(Channel::Color).is_none());
    assert!(layer.fill_points(Channel::Color).is_some());
    doc.undo().unwrap();
    let layer = doc.layer(fill).unwrap();
    assert_eq!(layer.fill_gradient(Channel::Color), Some(&shape));
    assert!(layer.fill_points(Channel::Color).is_none());
    assert_eq!(whole(&doc), with_shape);
}
