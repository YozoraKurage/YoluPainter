use yolu_core::material::{ChannelPaint, GradientSettings};
use yolu_core::*;
fn patterned() -> (Document, LayerId) {
    let mut d = Document::with_tile_size(9, 7, 4).unwrap();
    let id = d.add_layer("レイヤー").unwrap();
    for y in 0..7 {
        for x in 0..9 {
            d.set_pixel(
                id,
                x,
                y,
                Rgba8::new(
                    (x * 23) as u8,
                    (y * 31) as u8,
                    71,
                    if x % 3 == 0 { 0 } else { 128 },
                ),
            )
            .unwrap();
        }
    }
    d.clear_history().unwrap();
    (d, id)
}
fn pixels(d: &Document, id: LayerId) -> Vec<Rgba8> {
    (0..d.height())
        .flat_map(|y| {
            (0..d.width()).map(move |x| d.layer(id).unwrap().pixel(Channel::Color, x, y).unwrap())
        })
        .collect()
}
fn hard() -> BrushSettings {
    BrushSettings {
        radius: 80.,
        hardness: 1.,
        color: Rgba8::new(255, 0, 11, 128),
        pressure_opacity: false,
        pressure_size: false,
        ..Default::default()
    }
}
#[test]
fn transparency_keeps_alpha_and_hidden_rgb_and_undo() {
    let (mut d, id) = patterned();
    let before = pixels(&d, id);
    d.set_layer_locks(id, LayerLocks::TRANSPARENCY).unwrap();
    d.clear_history().unwrap();
    let mut s = d.begin_stroke(id, &hard()).unwrap();
    s.add_point(&mut d, 4., 3., 1., glam::DVec2::ZERO).unwrap();
    d.end_stroke(s).unwrap();
    for (a, b) in pixels(&d, id).iter().zip(&before) {
        assert_eq!(a.a, b.a);
        if b.a == 0 {
            assert_eq!(a, b);
        } else {
            assert_ne!(a, b);
        }
    }
    assert_eq!(d.undo_count(), 1);
    d.undo().unwrap();
    assert_eq!(pixels(&d, id), before);
    d.redo().unwrap();
    assert_ne!(pixels(&d, id), before);
}
#[test]
fn transparent_only_brush_needs_no_budget_or_history() {
    let mut d = Document::new(8, 8).unwrap();
    let id = d.add_layer("空").unwrap();
    d.set_layer_locks(id, LayerLocks::TRANSPARENCY).unwrap();
    d.clear_history().unwrap();
    d.set_stroke_budget_bytes(0).unwrap();
    let mut s = d.begin_stroke(id, &hard()).unwrap();
    s.add_point(&mut d, 3., 3., 1., glam::DVec2::ZERO).unwrap();
    assert!(!d.end_stroke(s).unwrap().changed);
    assert_eq!(d.undo_count(), 0);
}
#[test]
fn inherited_lock_names_holder_and_does_not_create_channel() {
    let (mut d, id) = patterned();
    let g = d.group_layers(&[id], "親").unwrap();
    d.set_layer_locks(g, LayerLocks::PIXELS).unwrap();
    let rev = d.revision();
    assert!(
        matches!(d.begin_stroke_in(id,Channel::Emission,&hard()),Err(CoreError::LayerLocked { holder, .. }) if holder==g)
    );
    assert!(d.layer(id).unwrap().surface(Channel::Emission).is_none());
    assert_eq!(d.revision(), rev);
}
#[test]
fn all_lock_keeps_name_visibility_order_copy_delete_available() {
    let (mut d, id) = patterned();
    d.set_layer_locks(id, LayerLocks::ALL).unwrap();
    assert!(d.set_layer_opacity(id, 0.2, false).is_err());
    assert!(d.add_layer_mask(id).is_err());
    assert!(d.set_channel_enabled(id, Channel::Normal, true).is_err());
    d.set_layer_name(id, "名前").unwrap();
    d.set_layer_visible(id, false).unwrap();
    let copy = d.duplicate_layer(id, None).unwrap();
    d.remove_layer(copy).unwrap();
}
#[test]
fn image_lock_allows_mask_and_integer_move_but_not_scale() {
    let (mut d, id) = patterned();
    d.add_layer_mask(id).unwrap();
    d.set_layer_locks(id, LayerLocks::PIXELS).unwrap();
    let mut s = d.begin_mask_stroke(id, &hard()).unwrap();
    s.apply_pixel(&mut d, 1, 1, 1., 1.).unwrap();
    d.end_stroke(s).unwrap();
    assert!(d
        .transform_layer(
            id,
            Affine2D::translation(1., 0.),
            Resampling::Bilinear,
            true
        )
        .unwrap());
    assert!(matches!(
        d.transform_layer(
            id,
            Affine2D::translation(0.5, 0.),
            Resampling::Bilinear,
            true
        ),
        Err(CoreError::LayerLocked { .. })
    ));
}
#[test]
fn erase_and_position_refusals_are_atomic() {
    let (mut d, id) = patterned();
    d.set_layer_locks(id, LayerLocks::TRANSPARENCY | LayerLocks::POSITION)
        .unwrap();
    let rev = d.revision();
    let b = pixels(&d, id);
    assert!(d
        .begin_stroke(
            id,
            &BrushSettings {
                erase: true,
                ..hard()
            }
        )
        .is_err());
    assert!(d
        .transform_layer(id, Affine2D::translation(1., 0.), Resampling::Nearest, true)
        .is_err());
    assert_eq!(d.revision(), rev);
    assert_eq!(pixels(&d, id), b);
    assert!(LayerLocks::from_bits(16).is_err());
}
#[test]
fn multiple_locks_undo_once_and_unknown_ids_preserve_redo() {
    let (mut d, a) = patterned();
    let b = d.add_layer("b").unwrap();
    d.clear_history().unwrap();
    d.change_layer_locks(&[a, b, a], LayerLocks::POSITION, true)
        .unwrap();
    assert_eq!(d.undo_count(), 1);
    d.undo().unwrap();
    assert!(d
        .change_layer_locks(&[a, LayerId(0)], LayerLocks::ALL, true)
        .is_err());
    assert_eq!(d.layer(a).unwrap().locks(), LayerLocks::NONE);
    assert_eq!(d.redo_count(), 1);
}
#[test]
fn duplicate_and_remove_groups_once() {
    let (mut d, a) = patterned();
    let g = d.group_layers(&[a], "g").unwrap();
    let b = d.add_layer("b").unwrap();
    d.clear_history().unwrap();
    assert_eq!(d.topmost_of(&[a, g, b, a]).unwrap(), vec![g, b]);
    let copies = d.duplicate_layers(&[g, b, a]).unwrap();
    assert_eq!(copies.len(), 2);
    assert_eq!(d.undo_count(), 1);
    assert_eq!(d.layers().len(), 6);
    d.undo().unwrap();
    assert_eq!(d.layers().len(), 3);
    d.remove_layers(&[g, b, a]).unwrap();
    assert!(d.layers().is_empty());
    d.undo().unwrap();
    assert_eq!(d.layers().len(), 3);
}
#[test]
fn duplicate_budget_failure_preserves_everything() {
    let (mut d, a) = patterned();
    let b = d.duplicate_layer(a, None).unwrap();
    d.clear_history().unwrap();
    let bytes = d.allocated_bytes();
    d.set_source_budget_bytes(bytes + bytes / 2).unwrap();
    let rev = d.revision();
    assert_eq!(
        d.duplicate_layers(&[a, b]),
        Err(CoreError::SourceBudgetExceeded)
    );
    assert_eq!(d.layers().len(), 2);
    assert_eq!(d.revision(), rev);
    assert_eq!(d.undo_count(), 0);
}
#[test]
fn move_multiple_preserves_order_and_rejects_cycles() {
    let (mut d, a) = patterned();
    let b = d.add_layer("b").unwrap();
    let c = d.add_layer("c").unwrap();
    let g = d.add_group("g", None).unwrap();
    d.clear_history().unwrap();
    d.move_layers(&[c, a], Some(g), 0).unwrap();
    assert_eq!(
        d.layers().iter().map(Layer::id).collect::<Vec<_>>(),
        vec![b, a, c, g]
    );
    assert!(d.move_layers(&[g], Some(g), 0).is_err());
    d.undo().unwrap();
    assert_eq!(d.layers()[0].id(), a);
}
#[test]
fn step_multiple_does_not_leapfrog_selected_neighbour() {
    let mut d = Document::new(1, 1).unwrap();
    let a = d.add_layer("a").unwrap();
    let b = d.add_layer("b").unwrap();
    let c = d.add_layer("c").unwrap();
    d.clear_history().unwrap();
    assert!(!d.step_layers(&[b, c], true).unwrap());
    assert!(d.step_layers(&[a, b], true).unwrap());
    assert_eq!(
        d.layers().iter().map(Layer::id).collect::<Vec<_>>(),
        vec![c, a, b]
    );
    assert_eq!(d.undo_count(), 1);
}
#[test]
fn integer_move_copies_hidden_rgb_and_mask_and_undo() {
    let (mut d, id) = patterned();
    d.add_layer_mask(id).unwrap();
    d.set_mask_pixel(id, 2, 2, 99).unwrap();
    let before = pixels(&d, id);
    d.clear_history().unwrap();
    d.transform_layer(
        id,
        Affine2D::translation(1., -1.),
        Resampling::Bilinear,
        true,
    )
    .unwrap();
    let p = pixels(&d, id);
    for y in 0..7 {
        for x in 0..9 {
            assert_eq!(
                p[y * 9 + x],
                if x == 0 || y == 6 {
                    Rgba8::TRANSPARENT
                } else {
                    before[(y + 1) * 9 + x - 1]
                }
            );
        }
    }
    assert_eq!(
        d.layer(id)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .pixel(3, 1)
            .unwrap()
            .a,
        99
    );
    d.undo().unwrap();
    assert_eq!(pixels(&d, id), before);
    d.redo().unwrap();
    assert_eq!(pixels(&d, id), p);
}
#[test]
fn transform_budgets_cancel_and_invalid_matrix_leave_history_and_serial() {
    let (mut d, id) = patterned();
    let b = pixels(&d, id);
    let serial = d.change_serial();
    let rev = d.revision();
    d.set_stroke_budget_bytes(0).unwrap();
    assert_eq!(
        d.transform_layer(id, Affine2D::translation(1., 0.), Resampling::Nearest, true),
        Err(CoreError::StrokeBudgetExceeded)
    );
    d.set_stroke_budget_bytes(1 << 20).unwrap();
    assert_eq!(
        d.transform_layers_cancellable(
            &[id],
            Affine2D::translation(1., 0.),
            Resampling::Nearest,
            &mut || true
        ),
        Err(CoreError::Cancelled)
    );
    assert!(d
        .transform_layer(
            id,
            Affine2D {
                a: 0.,
                ..Affine2D::IDENTITY
            },
            Resampling::Nearest,
            true
        )
        .is_err());
    assert_eq!(pixels(&d, id), b);
    assert_eq!(d.change_serial(), serial);
    assert_eq!(d.revision(), rev);
    assert_eq!(d.undo_count(), 0);
}
#[test]
fn later_layer_lock_refuses_entire_transform() {
    let (mut d, a) = patterned();
    let b = d.duplicate_layer(a, None).unwrap();
    d.set_layer_locks(b, LayerLocks::POSITION).unwrap();
    let old = pixels(&d, a);
    let rev = d.revision();
    assert!(d
        .transform_layers(&[a, b], Affine2D::translation(1., 0.), Resampling::Nearest)
        .is_err());
    assert_eq!(pixels(&d, a), old);
    assert_eq!(d.revision(), rev);
}
#[test]
fn resize_nearest_repeats_and_undo_restores_size_and_locks() {
    let (mut d, id) = patterned();
    let before = pixels(&d, id);
    d.set_layer_locks(id, LayerLocks::ALL).unwrap();
    d.clear_history().unwrap();
    d.resize_image(18, 14, CanvasResampling::Nearest).unwrap();
    for y in 0..14 {
        for x in 0..18 {
            assert_eq!(
                d.layer(id).unwrap().pixel(Channel::Color, x, y).unwrap(),
                before[(y / 2 * 9 + x / 2) as usize]
            );
        }
    }
    assert_eq!(d.layer(id).unwrap().locks(), LayerLocks::ALL);
    assert_eq!(d.undo_count(), 1);
    d.undo().unwrap();
    assert_eq!((d.width(), d.height()), (9, 7));
    assert_eq!(pixels(&d, id), before);
    d.redo().unwrap();
    assert_eq!(d.width(), 18);
}
#[test]
fn area_premultiplies_and_preserves_transparent_colour() {
    let mut d = Document::with_tile_size(2, 1, 1).unwrap();
    let id = d.add_layer("a").unwrap();
    d.set_pixel(id, 0, 0, Rgba8::new(240, 10, 20, 255)).unwrap();
    d.set_pixel(id, 1, 0, Rgba8::new(0, 0, 250, 0)).unwrap();
    d.resize_image(1, 1, CanvasResampling::Area).unwrap();
    assert_eq!(pixels(&d, id), vec![Rgba8::new(240, 10, 20, 128)]);
    d.undo().unwrap();
    d.set_pixel(id, 0, 0, Rgba8::new(100, 20, 30, 0)).unwrap();
    d.set_pixel(id, 1, 0, Rgba8::new(200, 40, 50, 0)).unwrap();
    d.resize_image(1, 1, CanvasResampling::Area).unwrap();
    assert_eq!(pixels(&d, id), vec![Rgba8::new(150, 30, 40, 0)]);
}
#[test]
fn canvas_resize_offsets_without_interpolation() {
    let (mut d, id) = patterned();
    let before = pixels(&d, id);
    d.resize_canvas(12, 10, (2, 1)).unwrap();
    assert_eq!(
        d.layer(id).unwrap().pixel(Channel::Color, 3, 2).unwrap(),
        before[10]
    );
    assert_eq!(pixels(&d, id)[0], Rgba8::TRANSPARENT);
    d.undo().unwrap();
    assert_eq!(pixels(&d, id), before);
}
#[test]
fn resize_budget_and_cancel_do_not_change_document() {
    let (mut d, id) = patterned();
    let old = pixels(&d, id);
    d.set_source_budget_bytes(d.allocated_bytes()).unwrap();
    assert_eq!(
        d.resize_image(18, 14, CanvasResampling::Bilinear),
        Err(CoreError::SourceBudgetExceeded)
    );
    d.set_source_budget_bytes(1 << 20).unwrap();
    assert_eq!(
        d.resize_image_cancellable(18, 14, CanvasResampling::Area, &mut || true),
        Err(CoreError::Cancelled)
    );
    assert_eq!(pixels(&d, id), old);
    assert_eq!(d.undo_count(), 0);
}
/// 大きさを変えた段を、やり直しに残さず捨てて元へ戻せる（複数の文書の大きさを順に変える途中で 1 つが断られたとき、先の文書を戻す）。
/// 前からある履歴は残る（捨てるのは直前の段だけ）。
#[test]
fn discarding_the_last_step_restores_the_size_and_leaves_no_redo() {
    let (mut d, id) = patterned();
    let before = pixels(&d, id);
    // 前の履歴（1 段）は残る
    d.set_layer_name(id, "名前").unwrap();
    let kept = d.history_bytes();
    d.resize_image(18, 14, CanvasResampling::Bilinear).unwrap();
    assert_eq!(d.undo_count(), 2);
    assert_eq!(d.discard_last_step(), Ok(true));
    assert_eq!((d.width(), d.height()), (9, 7));
    assert_eq!(pixels(&d, id), before, "大きさと画素が元のまま");
    assert_eq!(d.undo_count(), 1, "前の段は残る");
    assert!(!d.can_redo(), "やり直しには残らない");
    assert_eq!(d.history_bytes(), kept, "捨てた段の費用は引いてある");
    d.undo().unwrap();
    assert_eq!(
        d.discard_last_step(),
        Ok(false),
        "戻す段が無ければ何もしない"
    );
}
/// 大きさの変更を「準備」と「入れる」に分けて使える: 準備は文書も履歴（Undo・Redo）も変えず、予算で断られても何も残らない。入れると
/// `resize_image` と同じ結果の 1 段になる。準備のあとに文書が変わった・別の文書へ入れるのは、何も変えずに断る。
#[test]
fn a_prepared_resize_changes_nothing_until_committed_and_then_is_one_step() {
    let (mut d, id) = patterned();
    let (mut reference, reference_id) = patterned();
    reference
        .resize_image(18, 14, CanvasResampling::Bilinear)
        .unwrap();
    // Undo が 1 段・Redo が 1 段ある文書
    d.set_layer_name(id, "一").unwrap();
    d.set_layer_name(id, "二").unwrap();
    d.undo().unwrap();
    let (undo, redo, revision, bytes) = (
        d.undo_count(),
        d.redo_count(),
        d.revision(),
        d.history_bytes(),
    );
    let before = pixels(&d, id);
    // 同じ大きさなら準備は要らない
    assert!(d
        .prepare_resize_image(9, 7, CanvasResampling::Area, &mut || false)
        .unwrap()
        .is_none());
    let prepared = d
        .prepare_resize_image(18, 14, CanvasResampling::Bilinear, &mut || false)
        .unwrap()
        .expect("大きさが違う");
    assert_eq!((d.width(), d.height()), (9, 7));
    assert_eq!(pixels(&d, id), before);
    assert_eq!(
        (
            d.undo_count(),
            d.redo_count(),
            d.revision(),
            d.history_bytes()
        ),
        (undo, redo, revision, bytes),
        "準備は文書も履歴も変えない"
    );
    let report = d.commit_prepared_resize(prepared).unwrap();
    assert_eq!((d.width(), d.height()), (18, 14));
    assert_eq!(pixels(&d, id), pixels(&reference, reference_id));
    assert_eq!(d.undo_count(), undo + 1, "1 回の Undo の段");
    assert_eq!(d.redo_count(), 0, "新しい操作なので Redo は消える");
    assert_eq!(report, yolu_core::ResizeReport::default());
    d.undo().unwrap();
    assert_eq!(pixels(&d, id), before);
    // 予算で断られる準備は、何も残さない
    d.set_source_budget_bytes(d.allocated_bytes()).unwrap();
    let (undo, redo, revision) = (d.undo_count(), d.redo_count(), d.revision());
    assert!(matches!(
        d.prepare_resize_image(18, 14, CanvasResampling::Bilinear, &mut || false),
        Err(CoreError::SourceBudgetExceeded)
    ));
    assert_eq!(
        (d.undo_count(), d.redo_count(), d.revision()),
        (undo, redo, revision)
    );
    d.set_source_budget_bytes(1 << 20).unwrap();
    // 準備のあとに文書が変わった: 断る
    let stale = d
        .prepare_resize_image(18, 14, CanvasResampling::Bilinear, &mut || false)
        .unwrap()
        .unwrap();
    d.set_layer_name(id, "三").unwrap();
    assert!(matches!(
        d.commit_prepared_resize(stale),
        Err(CoreError::InvalidArgument(_))
    ));
    assert_eq!((d.width(), d.height()), (9, 7));
    // 別の文書へは入れない
    let (mut other, other_id) = patterned();
    let foreign = d
        .prepare_resize_image(18, 14, CanvasResampling::Bilinear, &mut || false)
        .unwrap()
        .unwrap();
    assert!(matches!(
        other.commit_prepared_resize(foreign),
        Err(CoreError::InvalidArgument(_))
    ));
    assert_eq!((other.width(), other.height()), (9, 7));
    assert_eq!(other.undo_count(), 0);
    let _ = other_id;
}
#[test]
fn clipped_merge_preserves_base_attributes_and_locks() {
    let (mut d, base) = patterned();
    let top = d
        .add_fill_layer(
            "上",
            &[(Channel::Color, Rgba8::new(200, 20, 80, 180))],
            None,
        )
        .unwrap();
    d.set_layer_clipping(top, true).unwrap();
    d.set_layer_opacity(base, 0.4, false).unwrap();
    d.set_layer_locks(base, LayerLocks::TRANSPARENCY).unwrap();
    let old = d.composite(d.bounds()).unwrap();
    d.clear_history().unwrap();
    let r = d.merge_down(top, 0).unwrap();
    assert!(r.exact());
    assert_eq!(r.method, MergeMethod::IntoClippingBase);
    assert_eq!(
        d.layer(r.result_id).unwrap().locks(),
        LayerLocks::TRANSPARENCY
    );
    assert_eq!(d.composite(d.bounds()).unwrap(), old);
    assert_eq!(d.undo_count(), 1);
    d.undo().unwrap();
    assert!(d.layer(base).is_some());
    d.redo().unwrap();
    assert_eq!(d.composite(d.bounds()).unwrap(), old);
}
#[test]
fn visible_merge_keeps_hidden_children_and_undo() {
    let (mut d, a) = patterned();
    let hidden = d.duplicate_layer(a, None).unwrap();
    d.set_layer_visible(hidden, false).unwrap();
    let g = d.group_layers(&[a, hidden], "g").unwrap();
    let before = d.composite(d.bounds()).unwrap();
    let r = d.merge_visible("結合", 2).unwrap();
    assert!(d.layer(hidden).is_some());
    assert!(d.layer(g).is_some());
    assert!(d.layer(a).is_none());
    assert!(r.exact());
    assert_eq!(d.composite(d.bounds()).unwrap(), before);
    d.undo().unwrap();
    assert_eq!(d.layers().len(), 3);
}
#[test]
fn merge_refusals_and_budget_leave_document_untouched() {
    let (mut d, a) = patterned();
    assert_eq!(
        d.merge_down_refusal(a).unwrap(),
        Some(MergeRefusal::NoLayerBelow)
    );
    let b = d.duplicate_layer(a, None).unwrap();
    d.clear_history().unwrap();
    d.set_stroke_budget_bytes(0).unwrap();
    let old = d.composite(d.bounds()).unwrap();
    let rev = d.revision();
    assert_eq!(d.merge_down(b, 255), Err(CoreError::StrokeBudgetExceeded));
    assert_eq!(d.composite(d.bounds()).unwrap(), old);
    assert_eq!(d.revision(), rev);
    assert_eq!(d.undo_count(), 0);
}
#[test]
fn merge_reports_and_rejects_visible_change() {
    let mut d = Document::new(1, 1).unwrap();
    let bg = d.add_layer("背景").unwrap();
    d.set_pixel(bg, 0, 0, Rgba8::new(200, 120, 90, 255))
        .unwrap();
    let a = d.add_layer("下").unwrap();
    d.set_pixel(a, 0, 0, Rgba8::new(10, 30, 180, 255)).unwrap();
    d.set_layer_blend_mode(a, BlendMode::Multiply).unwrap();
    let b = d.add_layer("上").unwrap();
    d.set_pixel(b, 0, 0, Rgba8::new(250, 200, 90, 255)).unwrap();
    let old = d.composite(d.bounds()).unwrap();
    let rev = d.revision();
    assert!(matches!(
        d.merge_down(b, 0),
        Err(CoreError::MergeAppearance(_))
    ));
    assert_eq!(d.revision(), rev);
    assert_eq!(d.composite(d.bounds()).unwrap(), old);
    assert!(d.merge_down(b, 255).unwrap().max_visible_difference > 0);
}
#[test]
fn redo_budget_refusal_preserves_history() {
    let (mut d, id) = patterned();
    d.resize_image(18, 14, CanvasResampling::Nearest).unwrap();
    d.undo().unwrap();
    let old = pixels(&d, id);
    d.set_source_budget_bytes(d.allocated_bytes()).unwrap();
    assert_eq!(d.redo(), Err(CoreError::SourceBudgetExceeded));
    assert_eq!(d.redo_count(), 1);
    assert_eq!(pixels(&d, id), old);
}

#[test]
fn fill_value_transparency_lock_refuses_alpha_change_but_allows_colour() {
    let mut d = Document::new(4, 4).unwrap();
    let id = d
        .add_fill_layer("値", &[(Channel::Color, Rgba8::new(10, 20, 30, 128))], None)
        .unwrap();
    d.set_layer_locks(id, LayerLocks::TRANSPARENCY).unwrap();
    assert!(d
        .set_fill_value(id, Channel::Color, Some(Rgba8::new(1, 2, 3, 255)), false)
        .is_err());
    d.set_fill_value(id, Channel::Color, Some(Rgba8::new(1, 2, 3, 128)), false)
        .unwrap();
    assert!(d.set_fill_value(id, Channel::Color, None, false).is_err());
}

#[test]
fn selected_transform_moves_pixels_and_selection_once_with_undo() {
    let (mut d, id) = patterned();
    let other = d.duplicate_layer(id, None).unwrap();
    let selection = SelectionMask::rectangle(&d, 1, 1, 4, 4);
    d.set_selection(Some(selection.clone())).unwrap();
    d.clear_history().unwrap();
    let before = pixels(&d, id);
    d.transform_layers(
        &[id, other],
        Affine2D::translation(2., 0.),
        Resampling::Bilinear,
    )
    .unwrap();
    assert_eq!(d.undo_count(), 1);
    assert_eq!(d.selection().unwrap().amount(5, 2), 255);
    assert_eq!(d.selection().unwrap().amount(1, 2), 0);
    assert_eq!(
        d.layer(id).unwrap().pixel(Channel::Color, 1, 2).unwrap(),
        Rgba8::TRANSPARENT
    );
    d.undo().unwrap();
    assert_eq!(d.selection(), Some(&selection));
    assert_eq!(pixels(&d, id), before);
    d.redo().unwrap();
    assert_eq!(d.selection().unwrap().amount(5, 2), 255);
}
#[test]
fn partial_move_is_refused_by_transparency_and_image_locks() {
    let (mut d, id) = patterned();
    d.set_selection(Some(SelectionMask::rectangle(&d, 0, 0, 4, 4)))
        .unwrap();
    for locks in [LayerLocks::TRANSPARENCY, LayerLocks::PIXELS] {
        d.set_layer_locks(id, locks).unwrap();
        let before = pixels(&d, id);
        let revision = d.revision();
        assert!(matches!(
            d.transform_layer(id, Affine2D::translation(1., 0.), Resampling::Nearest, true),
            Err(CoreError::LayerLocked { .. })
        ));
        assert_eq!(pixels(&d, id), before);
        assert_eq!(d.revision(), revision);
    }
}
#[test]
fn selected_fill_obeys_locks_and_refuses_before_any_mutation() {
    let (mut d, id) = patterned();
    d.set_layer_locks(id, LayerLocks::TRANSPARENCY).unwrap();
    d.set_selection(Some(SelectionMask::rectangle(&d, 1, 1, 4, 4)))
        .unwrap();
    let before = pixels(&d, id);
    d.fill(
        id,
        Channel::Color,
        Rgba8::new(255, 12, 30, 123),
        0.6,
        None,
        false,
    )
    .unwrap();
    for y in 0..7 {
        for x in 0..9 {
            let p = d.layer(id).unwrap().pixel(Channel::Color, x, y).unwrap();
            let old = before[(y * 9 + x) as usize];
            assert_eq!(p.a, old.a);
            if !(1..4).contains(&x) || !(1..4).contains(&y) || old.a == 0 {
                assert_eq!(p, old);
            }
        }
    }
    assert!(d
        .fill(id, Channel::Color, Rgba8::new(0, 0, 0, 255), 1., None, true)
        .is_err());
    d.set_layer_locks(id, LayerLocks::PIXELS).unwrap();
    let rev = d.revision();
    assert!(d
        .fill(
            id,
            Channel::Color,
            Rgba8::new(0, 0, 0, 255),
            1.,
            None,
            false
        )
        .is_err());
    assert_eq!(d.revision(), rev);
}
#[test]
fn resize_selection_and_history_dimensions_stay_together() {
    let (mut d, id) = patterned();
    let s = SelectionMask::rectangle(&d, 1, 1, 4, 4);
    d.set_selection(Some(s.clone())).unwrap();
    d.clear_history().unwrap();
    d.resize_image(18, 14, CanvasResampling::Nearest).unwrap();
    assert_eq!(d.selection().unwrap().width(), 18);
    assert_eq!(d.selection().unwrap().amount(2, 2), 255);
    d.undo().unwrap();
    assert_eq!(d.selection(), Some(&s));
    d.redo().unwrap();
    assert_eq!(d.selection().unwrap().width(), 18);
    d.resize_canvas(20, 16, (1, 1)).unwrap();
    assert_eq!(d.selection().unwrap().amount(3, 3), 255);
    d.undo().unwrap();
    assert_eq!(d.width(), 18);
    assert!(d.layer(id).is_some());
}
#[test]
fn cancelled_transform_after_prepared_tiles_leaves_selection_and_redo() {
    let (mut d, id) = patterned();
    let sel = SelectionMask::rectangle(&d, 0, 0, 9, 7);
    d.set_selection(Some(sel.clone())).unwrap();
    d.set_layer_name(id, "変更").unwrap();
    d.undo().unwrap();
    let old = pixels(&d, id);
    let rev = d.revision();
    let serial = d.change_serial();
    let mut calls = 0;
    assert_eq!(
        d.transform_layers_cancellable(
            &[id],
            Affine2D::translation(1., 0.),
            Resampling::Bilinear,
            &mut || {
                calls += 1;
                calls > 1
            }
        ),
        Err(CoreError::Cancelled)
    );
    assert_eq!(pixels(&d, id), old);
    assert_eq!(d.selection(), Some(&sel));
    assert_eq!(d.revision(), rev);
    assert_eq!(d.change_serial(), serial);
    assert_eq!(d.redo_count(), 1);
}

// ───────── ロックによる結合の拒否・結合の型と組の拒否 ─────────

/// 断った操作の前後で変わってはいけない、文書の見える状態（変更番号・記録の番号・履歴・レイヤーの並びと画素・合成）。
#[derive(Debug, PartialEq)]
struct Fingerprint {
    revision: u64,
    change_serial: u64,
    undo: usize,
    redo: usize,
    layers: Vec<LayerId>,
    pixels: Vec<Vec<Rgba8>>,
    composite: Vec<u8>,
}
fn fingerprint(d: &Document) -> Fingerprint {
    let layers: Vec<_> = d.layers().iter().map(Layer::id).collect();
    Fingerprint {
        revision: d.revision(),
        change_serial: d.change_serial(),
        undo: d.undo_count(),
        redo: d.redo_count(),
        pixels: layers.iter().map(|&id| pixels(d, id)).collect(),
        layers,
        composite: d.composite(d.bounds()).unwrap(),
    }
}
fn locked(layer: LayerId, holder: LayerId, lock: LayerLocks) -> CoreError {
    CoreError::LayerLocked {
        layer,
        holder,
        lock,
    }
}
/// 下のレイヤー a の上に写しの b。取り消した操作を 1 つ残してあるので、Redo が残ることも確かめられる。
fn two_layers() -> (Document, LayerId, LayerId) {
    let (mut d, a) = patterned();
    let b = d.duplicate_layer(a, None).unwrap();
    d.set_layer_name(b, "上").unwrap();
    d.undo().unwrap();
    assert_eq!(d.redo_count(), 1);
    (d, a, b)
}

#[test]
fn merge_down_refuses_locks_of_either_layer_and_changes_nothing() {
    // (ロックを付けるのは下のレイヤーか, ロック, 断られたときに名指すロック。None は結合できる)
    let cases = [
        (false, LayerLocks::PIXELS, Some(LayerLocks::PIXELS)),
        (false, LayerLocks::ALL, Some(LayerLocks::ALL)),
        (true, LayerLocks::PIXELS, Some(LayerLocks::PIXELS)),
        (true, LayerLocks::ALL, Some(LayerLocks::ALL)),
        (
            true,
            LayerLocks::TRANSPARENCY,
            Some(LayerLocks::TRANSPARENCY),
        ),
        (false, LayerLocks::TRANSPARENCY, None),
        (false, LayerLocks::POSITION, None),
        (true, LayerLocks::POSITION, None),
    ];
    for (on_lower, locks, expected) in cases {
        let (mut d, a, b) = two_layers();
        let holder = if on_lower { a } else { b };
        d.set_layer_locks(holder, locks).unwrap();
        let before = fingerprint(&d);
        match expected {
            Some(lock) => {
                assert_eq!(
                    d.merge_down(b, 255),
                    Err(locked(holder, holder, lock)),
                    "下={on_lower} {locks:?}"
                );
                assert_eq!(fingerprint(&d), before, "下={on_lower} {locks:?}");
                // 断ったのはロック: 外せば同じ結合ができる
                d.set_layer_locks(holder, LayerLocks::NONE).unwrap();
                assert!(d.merge_down(b, 255).is_ok());
            }
            None => {
                assert!(d.merge_down(b, 255).is_ok(), "下={on_lower} {locks:?}");
                assert_eq!(d.layers().len(), 1);
            }
        }
    }
}
#[test]
fn merge_down_inherits_group_locks_and_names_the_holder() {
    for (locks, layer_is_lower, lock) in [
        (LayerLocks::ALL, false, LayerLocks::ALL),
        (LayerLocks::PIXELS, false, LayerLocks::PIXELS),
        // 透明部分のロックは下のレイヤーへの結合だけを断る（クリッピングの下地への結合は断らない）
        (LayerLocks::TRANSPARENCY, true, LayerLocks::TRANSPARENCY),
    ] {
        let (mut d, a, b) = two_layers();
        let g = d.group_layers(&[a, b], "親").unwrap();
        d.set_layer_locks(g, locks).unwrap();
        let before = fingerprint(&d);
        let layer = if layer_is_lower { a } else { b };
        assert_eq!(d.merge_down(b, 255), Err(locked(layer, g, lock)));
        assert_eq!(fingerprint(&d), before);
    }
}
#[test]
fn merge_visible_refuses_a_locked_contributor_but_not_a_hidden_one() {
    let (mut d, a, b) = two_layers();
    d.set_layer_locks(b, LayerLocks::PIXELS).unwrap();
    let before = fingerprint(&d);
    assert_eq!(
        d.merge_visible("結合", 255),
        Err(locked(b, b, LayerLocks::PIXELS))
    );
    assert_eq!(fingerprint(&d), before);
    // 隠したレイヤーは結合に入らないので、そのロックは数えない（隠したレイヤーは残る）
    d.set_layer_visible(b, false).unwrap();
    d.merge_visible("結合", 255).unwrap();
    assert!(d.layer(b).is_some() && d.layer(a).is_none());
    // 親のすべてのロックは、どの子も断る。名指すのは並びの最初（下）のレイヤー
    let (mut d, a, b) = two_layers();
    let g = d.group_layers(&[a, b], "親").unwrap();
    d.set_layer_locks(g, LayerLocks::ALL).unwrap();
    let before = fingerprint(&d);
    assert_eq!(
        d.merge_visible("結合", 255),
        Err(locked(a, g, LayerLocks::ALL))
    );
    assert_eq!(fingerprint(&d), before);
    // 何も見えていなければ結合しない
    let (mut d, a, b) = two_layers();
    d.set_layer_visible(a, false).unwrap();
    d.set_layer_visible(b, false).unwrap();
    let before = fingerprint(&d);
    assert_eq!(
        d.merge_visible("結合", 255),
        Err(CoreError::MergeRefused(MergeRefusal::NothingVisible))
    );
    assert_eq!(fingerprint(&d), before);
}
#[test]
fn merge_layers_and_group_refuse_own_and_inherited_locks_and_change_nothing() {
    // merge_layers: 選んだレイヤーの画像・すべて、親グループのすべて
    for (lock_on, locks, layer, holder_is_group, lock) in [
        ("a", LayerLocks::PIXELS, "a", false, LayerLocks::PIXELS),
        ("b", LayerLocks::ALL, "b", false, LayerLocks::ALL),
        ("g", LayerLocks::ALL, "a", true, LayerLocks::ALL),
        ("g", LayerLocks::PIXELS, "a", true, LayerLocks::PIXELS),
    ] {
        let (mut d, a, b) = two_layers();
        let g = d.group_layers(&[a, b], "親").unwrap();
        let pick = |name: &str| match name {
            "a" => a,
            "b" => b,
            _ => g,
        };
        d.set_layer_locks(pick(lock_on), locks).unwrap();
        let before = fingerprint(&d);
        let holder = if holder_is_group { g } else { pick(layer) };
        assert_eq!(
            d.merge_layers(&[a, b], 255),
            Err(locked(pick(layer), holder, lock)),
            "{lock_on} {locks:?}"
        );
        assert_eq!(fingerprint(&d), before);
        // merge_group も同じレイヤーを同じロックで断る
        assert_eq!(
            d.merge_group(g, 255),
            Err(locked(pick(layer), holder, lock)),
            "{lock_on} {locks:?}"
        );
        assert_eq!(fingerprint(&d), before);
    }
    // 透明部分・位置のロックは結合を断らない。グループのロックは結果のレイヤーへ引き継ぐ
    let (mut d, a, b) = two_layers();
    let g = d.group_layers(&[a, b], "親").unwrap();
    d.set_layer_locks(g, LayerLocks::TRANSPARENCY | LayerLocks::POSITION)
        .unwrap();
    let report = d.merge_group(g, 255).unwrap();
    assert_eq!(
        d.layer(report.result_id).unwrap().locks(),
        LayerLocks::TRANSPARENCY | LayerLocks::POSITION
    );
    d.undo().unwrap();
    d.set_layer_locks(g, LayerLocks::NONE).unwrap();
    d.set_layer_locks(a, LayerLocks::TRANSPARENCY).unwrap();
    d.merge_layers(&[a, b], 255).unwrap();
}
#[test]
fn merge_refuses_wrong_kinds_pairs_and_unknown_layers_without_changes() {
    let (mut d, a, b) = two_layers();
    let empty = d.add_group("空", None).unwrap();
    let g = d.group_layers(&[b], "子の親").unwrap();
    let other = d.add_layer("別").unwrap();
    d.set_layer_visible(other, false).unwrap();
    let before = fingerprint(&d);
    let refused = |r| Err(CoreError::MergeRefused(r));
    assert_eq!(d.merge_group(a, 255), refused(MergeRefusal::NotGroup));
    assert_eq!(d.merge_group(empty, 255), refused(MergeRefusal::EmptyGroup));
    assert_eq!(
        d.merge_group(LayerId(0), 255),
        Err(CoreError::LayerNotFound)
    );
    // 2 レイヤーに満たない（重複と、選んだグループに含まれる子は 1 つに数える）
    for ids in [vec![a], vec![a, a], vec![], vec![g, b]] {
        assert!(
            matches!(
                d.merge_layers(&ids, 255),
                Err(CoreError::InvalidArgument(_))
            ),
            "{ids:?}"
        );
    }
    // 親が違う（a は最上位、b はグループの中）
    assert_eq!(
        d.merge_layers(&[a, b], 255),
        refused(MergeRefusal::DifferentGroups)
    );
    // 隠したレイヤー
    assert_eq!(
        d.merge_layers(&[a, other], 255),
        refused(MergeRefusal::HiddenLayer)
    );
    assert_eq!(
        d.merge_layers(&[a, LayerId(0)], 255),
        Err(CoreError::LayerNotFound)
    );
    // merge_down の拒否: グループ・下が無い・下がグループ（隠したレイヤーより先に断る）
    assert_eq!(d.merge_down(g, 255), refused(MergeRefusal::IsGroup));
    assert_eq!(d.merge_down(a, 255), refused(MergeRefusal::NoLayerBelow));
    assert_eq!(
        d.merge_down(other, 255),
        refused(MergeRefusal::LayerBelowIsGroup)
    );
    assert_eq!(fingerprint(&d), before);

    let (mut d, a, b) = two_layers();
    d.set_layer_visible(b, false).unwrap();
    assert_eq!(d.merge_down(b, 255), refused(MergeRefusal::HiddenLayer));
    d.set_layer_visible(b, true).unwrap();
    d.set_layer_visible(a, false).unwrap();
    assert_eq!(d.merge_down(b, 255), refused(MergeRefusal::HiddenLayer));

    let (mut d, a, b) = two_layers();
    d.group_layers(&[a], "下のグループ").unwrap();
    assert_eq!(
        d.merge_down(b, 255),
        refused(MergeRefusal::LayerBelowIsGroup)
    );
    let (mut d, a, b) = two_layers();
    d.add_adjustment_layer(
        "調整",
        AdjustmentSettings::levels(0.1, 0.9, 1.4, 0.0, 1.0).unwrap(),
        None,
        Some(a),
    )
    .unwrap();
    assert_eq!(
        d.merge_down(b, 255),
        refused(MergeRefusal::LayerBelowIsAdjustment)
    );
}
#[test]
fn merge_refusals_read_as_short_japanese_states() {
    for r in [
        MergeRefusal::NoLayerBelow,
        MergeRefusal::LayerBelowIsGroup,
        MergeRefusal::LayerBelowIsAdjustment,
        MergeRefusal::HiddenLayer,
        MergeRefusal::IsGroup,
        MergeRefusal::NotGroup,
        MergeRefusal::EmptyGroup,
        MergeRefusal::NothingVisible,
        MergeRefusal::DifferentGroups,
        MergeRefusal::TooManyRulers,
    ] {
        let text = CoreError::MergeRefused(r).to_string();
        assert!(
            !text.chars().any(|c| c.is_ascii_alphabetic()),
            "識別子が見える: {text}"
        );
        assert!(text.chars().count() <= 20, "長い: {text}");
    }
    assert_eq!(
        CoreError::MergeRefused(MergeRefusal::NoLayerBelow).to_string(),
        "結合できない: 下にレイヤーが無い"
    );
}

// ───────── 読み込み用のロック・変形の範囲と予算・型 ─────────

#[test]
fn set_locks_for_load_clears_history_and_refuses_during_a_stroke_and_unknown_layers() {
    let (mut d, a) = patterned();
    let b = d.add_layer("b").unwrap();
    d.clear_history().unwrap();
    d.set_layer_name(a, "名前").unwrap();
    d.set_layer_name(a, "別").unwrap();
    d.undo().unwrap();
    assert_eq!((d.undo_count(), d.redo_count()), (1, 1));
    let revision = d.revision();
    d.set_locks_for_load(a, LayerLocks::ALL | LayerLocks::POSITION)
        .unwrap();
    assert_eq!(
        d.layer(a).unwrap().locks(),
        LayerLocks::ALL | LayerLocks::POSITION
    );
    assert_eq!((d.undo_count(), d.redo_count()), (0, 0));
    assert!(d.revision() > revision);
    // 検査なしで置ける（ロック済みのレイヤーにも。読み込みの途中の編集を断らないため）
    d.set_locks_for_load(a, LayerLocks::NONE).unwrap();
    assert_eq!(
        d.set_locks_for_load(LayerId(0), LayerLocks::NONE),
        Err(CoreError::LayerNotFound)
    );
    // 進行中のストロークがあれば断る（ロックは変わらない）
    let stroke = d.begin_stroke(b, &hard()).unwrap();
    assert_eq!(
        d.set_locks_for_load(b, LayerLocks::PIXELS),
        Err(CoreError::StrokeActive)
    );
    assert_eq!(d.layer(b).unwrap().locks(), LayerLocks::NONE);
    d.cancel_stroke(stroke);
    // 保存した値は型で受ける: 既知の 4 ビットの組はそのまま往復し、未知のビットは断る
    for bits in 0..16u8 {
        assert_eq!(LayerLocks::from_bits(bits).unwrap().bits(), bits);
    }
    for bits in [16u8, 32, 64, 128, 255] {
        assert!(LayerLocks::from_bits(bits).is_err(), "{bits}");
    }
}

#[test]
fn transform_bounds_covers_alpha_pixels_masks_regions_and_the_document_selection() {
    let (mut d, id) = patterned();
    // 左右の端の透明な列（x が 3 の倍数は透明）を含まない
    assert_eq!(
        d.transform_bounds(id, None),
        Ok(Some(Rect::new(1, 0, 8, 7)))
    );
    let region = SelectionMask::rectangle(&d, 3, 2, 6, 5);
    assert_eq!(
        d.transform_bounds(id, Some(&region)),
        Ok(Some(Rect::new(4, 2, 2, 3)))
    );
    // 範囲を渡さなければ文書の選択範囲。両方あれば交差
    d.set_selection(Some(region.clone())).unwrap();
    assert_eq!(
        d.transform_bounds(id, None),
        Ok(Some(Rect::new(4, 2, 2, 3)))
    );
    let wide = SelectionMask::rectangle(&d, 0, 0, 5, 4);
    assert_eq!(
        d.transform_bounds(id, Some(&wide)),
        Ok(Some(Rect::new(4, 2, 1, 2)))
    );
    // 範囲の外だけにしか画素が無ければ無し
    let far = SelectionMask::rectangle(&d, 0, 5, 1, 7);
    assert_eq!(d.transform_bounds(id, Some(&far)), Ok(None));
    d.clear_selection().unwrap();
    // マスクの隠す量も数える（0 は数えない）。画素の無いレイヤー・アルファ 0 だけのレイヤーは無し
    let only_mask = d.add_layer("マスクだけ").unwrap();
    d.add_layer_mask(only_mask).unwrap();
    assert_eq!(d.transform_bounds(only_mask, None), Ok(None));
    d.set_mask_pixel(only_mask, 2, 3, 77).unwrap();
    d.set_mask_pixel(only_mask, 6, 4, 0).unwrap();
    assert_eq!(
        d.transform_bounds(only_mask, None),
        Ok(Some(Rect::new(2, 3, 1, 1)))
    );
    let clear = d.add_layer("透明だけ").unwrap();
    d.set_pixel(clear, 5, 5, Rgba8::new(10, 20, 30, 0)).unwrap();
    assert_eq!(d.transform_bounds(clear, None), Ok(None));
    assert_eq!(
        d.transform_bounds(LayerId(0), None),
        Err(CoreError::LayerNotFound)
    );
}

#[test]
fn transform_budget_counts_every_layer_together() {
    let (mut d, a) = patterned();
    let b = d.duplicate_layer(a, None).unwrap();
    d.clear_history().unwrap();
    let shift = Affine2D::translation(1., 0.);
    // 1 レイヤーが通る一番小さい予算（見積りはレイヤーの元のタイル + タイルごとの 64 バイト）
    let one = (0..4096)
        .find(|&budget| {
            d.set_stroke_budget_bytes(budget).unwrap();
            let ok = d
                .transform_layer(a, shift, Resampling::Nearest, true)
                .is_ok();
            if ok {
                d.undo().unwrap();
            }
            ok
        })
        .expect("通る予算がある");
    d.clear_history().unwrap();
    d.set_stroke_budget_bytes(one).unwrap();
    let before = fingerprint(&d);
    // 2 レイヤーは、レイヤーごとには通っても合計で超えるので断る。1 レイヤーも変わらない
    assert_eq!(
        d.transform_layers(&[a, b], shift, Resampling::Nearest),
        Err(CoreError::StrokeBudgetExceeded)
    );
    assert_eq!(fingerprint(&d), before);
    d.set_stroke_budget_bytes(2 * one).unwrap();
    assert_eq!(
        d.transform_layers(&[a, b], shift, Resampling::Nearest),
        Ok(true)
    );
    assert_eq!(d.undo_count(), 1);
}

#[test]
fn transform_refuses_non_raster_layers_and_inherits_group_locks() {
    let (mut d, a) = patterned();
    let fill = d
        .add_fill_layer("塗り", &[(Channel::Color, Rgba8::new(1, 2, 3, 255))], None)
        .unwrap();
    let empty = d.add_group("空", None).unwrap();
    d.clear_history().unwrap();
    let shift = Affine2D::translation(1., 0.);
    let before = fingerprint(&d);
    // ラスター以外は 1 レイヤーを指しても、グループや塗りつぶしだけを選んでも動かせない
    assert!(matches!(
        d.transform_layer(fill, shift, Resampling::Nearest, true),
        Err(CoreError::Unsupported(_))
    ));
    assert!(matches!(
        d.transform_layer(empty, shift, Resampling::Nearest, true),
        Err(CoreError::Unsupported(_))
    ));
    for ids in [vec![fill], vec![empty], vec![fill, empty]] {
        assert!(
            matches!(
                d.transform_layers(&ids, shift, Resampling::Nearest),
                Err(CoreError::Unsupported(_))
            ),
            "{ids:?}"
        );
    }
    assert_eq!(fingerprint(&d), before);
    // 親グループの位置ロックは、子の変形も、グループを選んだ変形も断る
    let g = d.group_layers(&[a], "親").unwrap();
    d.set_layer_locks(g, LayerLocks::POSITION).unwrap();
    let before = fingerprint(&d);
    let refused = Err(locked(a, g, LayerLocks::POSITION));
    assert_eq!(
        d.transform_layer(a, shift, Resampling::Nearest, true),
        refused
    );
    assert_eq!(
        d.transform_layers(&[g], shift, Resampling::Nearest),
        refused
    );
    assert_eq!(fingerprint(&d), before);
    // 親の画像ロックは整数画素の移動だけ許す
    d.set_layer_locks(g, LayerLocks::PIXELS).unwrap();
    assert_eq!(
        d.transform_layers(&[g], shift, Resampling::Nearest),
        Ok(true)
    );
    assert_eq!(
        d.transform_layer(
            a,
            Affine2D::translation(0.5, 0.),
            Resampling::Bilinear,
            true
        ),
        Err(locked(a, g, LayerLocks::PIXELS))
    );
}

#[test]
fn resize_cancelled_at_every_check_leaves_the_document_untouched() {
    // 1 スレッドだとタイル 4 枚ごとの確認になり、確認の回数が並列度によらず決まる
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    pool.install(|| {
        let (mut d, id) = patterned();
        d.add_layer_mask(id).unwrap();
        d.set_mask_pixel(id, 2, 2, 99).unwrap();
        d.set_selection(Some(SelectionMask::rectangle(&d, 1, 1, 6, 5)))
            .unwrap();
        d.clear_history().unwrap();
        d.set_layer_name(id, "名前").unwrap();
        d.undo().unwrap();
        let before = fingerprint(&d);
        let selection = d.selection().cloned();
        let mask = d
            .layer(id)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .to_canvas_bytes();
        let mut checks_to_finish = 0;
        for stop_at in 1.. {
            let mut calls = 0;
            let result =
                d.resize_image_cancellable(36, 28, CanvasResampling::Bilinear, &mut || {
                    calls += 1;
                    calls == stop_at
                });
            if result.is_ok() {
                checks_to_finish = stop_at;
                break;
            }
            // N 回目の確認で取り消すと、寸法・画素・マスク・選択範囲・履歴（Redo も）は元のまま
            assert_eq!(result, Err(CoreError::Cancelled), "{stop_at} 回目");
            assert_eq!((d.width(), d.height()), (9, 7), "{stop_at} 回目");
            assert_eq!(fingerprint(&d), before, "{stop_at} 回目");
            assert_eq!(d.selection().cloned(), selection, "{stop_at} 回目");
            assert_eq!(
                d.layer(id)
                    .unwrap()
                    .mask()
                    .unwrap()
                    .surface()
                    .to_canvas_bytes(),
                mask,
                "{stop_at} 回目"
            );
        }
        // 確認はタイルのまとまりごと: 途中で取り消せる回数が十分にある
        assert!(checks_to_finish > 10, "{checks_to_finish}");
        assert_eq!((d.width(), d.height()), (36, 28));
        assert_eq!(d.undo_count(), 1);
        assert_eq!(d.redo_count(), 0);
    });
}

// ───────── サイズ変更の履歴・変化の記録・書き込み口のロック ─────────

#[test]
fn resize_beyond_the_history_budget_keeps_its_own_undo_and_says_so() {
    let (mut d, id) = patterned();
    let before = pixels(&d, id);
    d.set_layer_name(id, "一").unwrap();
    d.set_layer_opacity(id, 0.5, false).unwrap();
    // 小さい段 2 つは入り、サイズ変更（前後の格納量）は入らない予算
    d.set_undo_budget_bytes(300).unwrap();
    assert_eq!(d.undo_count(), 2);
    // 予算に入らない: 古い 2 段は落とすが、このサイズ変更の 1 段は残して知らせる
    let report = d.resize_image(36, 28, CanvasResampling::Nearest).unwrap();
    assert!(report.history_over_budget);
    assert!(report.notes.iter().any(|n| n == "履歴の予算を超えた"));
    assert!(d.history_bytes() > d.undo_budget_bytes());
    assert_eq!(d.undo_count(), 1);
    assert_eq!(d.history_trimmed().0, 1);
    d.undo().unwrap();
    assert_eq!((d.width(), d.height()), (9, 7));
    assert_eq!(pixels(&d, id), before);
    d.redo().unwrap();
    assert_eq!((d.width(), d.height()), (36, 28));
    // 次の編集の整理では普通の段として落ち得る（予算は守る）。画像は変わったまま
    d.set_layer_name(id, "二").unwrap();
    assert_eq!(d.undo_count(), 1);
    assert!(d.history_bytes() <= d.undo_budget_bytes());
    d.undo().unwrap();
    assert_eq!((d.width(), d.height()), (36, 28));
}
#[test]
fn resize_history_respects_minimum_steps_and_a_roomy_budget_and_refusals_keep_history() {
    // 予算が十分なら古い段も残り、知らせない
    let (mut d, id) = patterned();
    d.set_layer_name(id, "一").unwrap();
    d.set_layer_opacity(id, 0.5, false).unwrap();
    let report = d.resize_image(18, 14, CanvasResampling::Nearest).unwrap();
    assert!(!report.history_over_budget && report.notes.is_empty());
    assert_eq!(d.undo_count(), 3);
    // 守る段数が多ければ、予算を超えても古い段を落とさない
    let (mut d, id) = patterned();
    d.set_layer_name(id, "一").unwrap();
    d.set_layer_opacity(id, 0.5, false).unwrap();
    d.set_minimum_undo_steps(3).unwrap();
    d.set_undo_budget_bytes(1).unwrap();
    assert!(d.resize_canvas(20, 16, (1, 1)).unwrap().history_over_budget);
    assert_eq!(d.undo_count(), 3);
    // 断る・取り消すときは、履歴を落とさない（小さい予算でも）
    let (mut d, id) = patterned();
    d.set_layer_name(id, "一").unwrap();
    d.set_undo_budget_bytes(2000).unwrap();
    d.set_source_budget_bytes(d.allocated_bytes()).unwrap();
    assert_eq!(
        d.resize_image(18, 14, CanvasResampling::Bilinear),
        Err(CoreError::SourceBudgetExceeded)
    );
    assert_eq!(d.undo_count(), 1);
    assert_eq!(d.history_trimmed(), (0, 0));
    let _ = id;
}

/// 3 レイヤー（それぞれ別のタイルに画素が 1 つ）の 64×64、タイルは 8。
fn three_tiles() -> (Document, [LayerId; 3]) {
    let mut d = Document::with_tile_size(64, 64, 8).unwrap();
    let ids = [(1, 1), (20, 20), (50, 50)].map(|(x, y)| {
        let id = d.add_layer("レイヤー").unwrap();
        d.set_pixel(id, x, y, Rgba8::new(200, 100, 50, 255))
            .unwrap();
        id
    });
    d.clear_history().unwrap();
    (d, ids)
}
fn changed(d: &Document, since: u64) -> Vec<(u32, u32)> {
    let mut v: Vec<_> = d
        .changed_tiles(Channel::Color, since)
        .unwrap()
        .iter()
        .map(|c| (c.x, c.y))
        .collect();
    v.sort();
    v
}
#[test]
fn multi_layer_operations_mark_only_the_layers_they_change() {
    // ロックだけの変更は合成を変えないので、何も印を付けない（Undo・Redo も）
    let (mut d, [a, b, c]) = three_tiles();
    let since = d.change_serial();
    d.change_layer_locks(&[a, b, c], LayerLocks::POSITION, true)
        .unwrap();
    assert_eq!(d.change_serial(), since);
    assert!(changed(&d, since).is_empty());
    d.undo().unwrap();
    d.redo().unwrap();
    assert_eq!(d.change_serial(), since);

    // レイヤーの削除・複製・表示・一段移動・結合・変形は、動いたレイヤーのタイルだけ（元に戻したときも）
    let (mut d, [_, b, _]) = three_tiles();
    let since = d.change_serial();
    d.remove_layers(&[b]).unwrap();
    assert_eq!(changed(&d, since), [(2, 2)]);
    let since = d.change_serial();
    d.undo().unwrap();
    assert_eq!(changed(&d, since), [(2, 2)]);

    let (mut d, [_, _, c]) = three_tiles();
    let since = d.change_serial();
    d.duplicate_layers(&[c]).unwrap();
    assert_eq!(changed(&d, since), [(6, 6)]);

    let (mut d, [a, b, _]) = three_tiles();
    let since = d.change_serial();
    d.set_layers_visibility(&[a, b], false).unwrap();
    assert_eq!(changed(&d, since), [(0, 0), (2, 2)]);

    let (mut d, [a, _, _]) = three_tiles();
    let since = d.change_serial();
    assert!(d.step_layers(&[a], true).unwrap());
    assert_eq!(changed(&d, since), [(0, 0)]);

    let (mut d, [_, b, _]) = three_tiles();
    let since = d.change_serial();
    d.merge_down(b, 255).unwrap();
    assert_eq!(changed(&d, since), [(0, 0), (2, 2)]);
    let since = d.change_serial();
    d.undo().unwrap();
    assert_eq!(changed(&d, since), [(0, 0), (2, 2)]);

    let (mut d, [_, _, c]) = three_tiles();
    let since = d.change_serial();
    d.transform_layers(&[c], Affine2D::translation(8., 0.), Resampling::Nearest)
        .unwrap();
    assert_eq!(changed(&d, since), [(6, 6), (7, 6)]);

    // 全部のレイヤーを外しても（文書にレイヤーが無くなっても）印を付けられる
    let (mut d, ids) = three_tiles();
    let since = d.change_serial();
    d.remove_layers(&ids).unwrap();
    assert_eq!(changed(&d, since), [(0, 0), (2, 2), (6, 6)]);
    let since = d.change_serial();
    d.undo().unwrap();
    assert_eq!(changed(&d, since), [(0, 0), (2, 2), (6, 6)]);

    // サイズ変更は寸法が変わるので、どのレイヤーも変わり得る
    let (mut d, _) = three_tiles();
    let since = d.change_serial();
    d.resize_canvas(70, 70, (0, 0)).unwrap();
    let all = changed(&d, since);
    for tile in [(0, 0), (2, 2), (6, 6)] {
        assert!(all.contains(&tile), "{tile:?}");
    }
}

// ───────── 画素・マスクを書く入口とロック ─────────

/// 断った書き込みの前後で変わってはいけない状態に、全チャンネルの有効・面の有無・画素とマスクを足したもの（断った書き込みが、
/// 無効のチャンネルを有効にしたり面を作ったりして残さないことも見る）。
#[derive(Debug, PartialEq)]
struct DeepFingerprint {
    base: Fingerprint,
    channels: Vec<Vec<(bool, Option<Vec<u8>>)>>,
    masks: Vec<Option<Vec<u8>>>,
    active: bool,
}
fn deep_fingerprint(d: &Document) -> DeepFingerprint {
    DeepFingerprint {
        base: fingerprint(d),
        channels: d
            .layers()
            .iter()
            .map(|l| {
                Channel::ALL
                    .iter()
                    .map(|&c| {
                        (
                            l.is_channel_enabled(c),
                            l.surface(c).map(|s| s.to_canvas_bytes()),
                        )
                    })
                    .collect()
            })
            .collect(),
        masks: d
            .layers()
            .iter()
            .map(|l| l.mask().map(|m| m.surface().to_canvas_bytes()))
            .collect(),
        active: d.has_active_stroke() || d.active_stroke_stats().is_some(),
    }
}
/// ロックの検査の試験のレイヤー: 画素のある Color に、マスク。ほかのチャンネルは無効で面が無い（マテリアルで塗ると有効にする）。
fn locked_target() -> (Document, LayerId) {
    let (mut d, id) = patterned();
    d.add_layer_mask(id).unwrap();
    d.clear_history().unwrap();
    (d, id)
}
fn solid() -> Rgba8 {
    Rgba8::new(10, 20, 30, 255)
}
fn two_channels() -> Vec<ChannelPaint> {
    vec![
        ChannelPaint::new(Channel::Color, solid()),
        ChannelPaint::new(Channel::Emission, Rgba8::new(200, 100, 50, 255)),
    ]
}
fn ramp() -> GradientSettings {
    GradientSettings {
        start: glam::DVec2::new(0., 0.),
        end: glam::DVec2::new(8., 6.),
        from: Rgba8::new(255, 0, 0, 255),
        to: Rgba8::new(0, 0, 255, 90),
        ..Default::default()
    }
}
fn erasing() -> BrushSettings {
    BrushSettings {
        erase: true,
        ..hard()
    }
}
fn triangles() -> [material_triangles::PixelTriangle; 1] {
    [[
        glam::DVec2::new(0., 0.),
        glam::DVec2::new(9., 0.),
        glam::DVec2::new(0., 7.),
    ]]
}
type WriteEntry = Box<dyn Fn(&mut Document, LayerId) -> Result<(), CoreError>>;
/// 書き込みの入口 1 つ。refused_by_pixels は画像のロックで断るか（マスクの入口は断らない）、erases は消す書き込みか
/// （透明部分のロックで断る）。
struct Entry {
    name: &'static str,
    run: WriteEntry,
    refused_by_pixels: bool,
    erases: bool,
}
fn entry(
    name: &'static str,
    refused_by_pixels: bool,
    erases: bool,
    run: impl Fn(&mut Document, LayerId) -> Result<(), CoreError> + 'static,
) -> Entry {
    Entry {
        name,
        run: Box::new(run),
        refused_by_pixels,
        erases,
    }
}
fn stroke_entry(d: &mut Document, started: Result<Stroke, CoreError>) -> Result<(), CoreError> {
    started.map(|s| d.cancel_stroke(s))
}
fn fill_entry(d: &mut Document, started: Result<TriangleFill, CoreError>) -> Result<(), CoreError> {
    started.map(|f| f.cancel(d))
}
/// 画素・マスクを書く入口の全部。新しい入口を足したら、ここへ足して、ロックの検査（`Document::pixel_write_guard`、マスクは
/// `refuse_lock`）を通すこと。
fn write_entries() -> Vec<Entry> {
    vec![
        entry("begin_stroke", true, false, |d, id| {
            let s = d.begin_stroke(id, &hard());
            stroke_entry(d, s)
        }),
        entry("begin_stroke（消す）", true, true, |d, id| {
            let s = d.begin_stroke(id, &erasing());
            stroke_entry(d, s)
        }),
        entry("begin_stroke_in", true, false, |d, id| {
            let s = d.begin_stroke_in(id, Channel::Emission, &hard());
            stroke_entry(d, s)
        }),
        entry("begin_brush_stroke", true, false, |d, id| {
            let s = d.begin_brush_stroke(id, &Brush::from(hard()));
            stroke_entry(d, s)
        }),
        entry("begin_material_stroke", true, false, |d, id| {
            let s = d.begin_material_stroke(id, &two_channels(), &hard());
            stroke_entry(d, s)
        }),
        entry("begin_material_stroke（消す）", true, true, |d, id| {
            let s = d.begin_material_stroke(id, &two_channels(), &erasing());
            stroke_entry(d, s)
        }),
        entry("begin_material_brush_stroke", true, false, |d, id| {
            let s = d.begin_material_brush_stroke(id, &two_channels(), &Brush::from(hard()));
            stroke_entry(d, s)
        }),
        entry("begin_triangle_fill", true, false, |d, id| {
            let f = d.begin_triangle_fill(id, Channel::Color, solid(), 1., false);
            fill_entry(d, f)
        }),
        entry("begin_triangle_fill（消す）", true, true, |d, id| {
            let f = d.begin_triangle_fill(id, Channel::Color, solid(), 1., true);
            fill_entry(d, f)
        }),
        entry("begin_material_triangle_fill", true, false, |d, id| {
            let f = d.begin_material_triangle_fill(id, &two_channels(), 1., false);
            fill_entry(d, f)
        }),
        entry(
            "begin_material_triangle_fill（消す）",
            true,
            true,
            |d, id| {
                let f = d.begin_material_triangle_fill(id, &two_channels(), 1., true);
                fill_entry(d, f)
            },
        ),
        entry("fill", true, false, |d, id| {
            d.fill(id, Channel::Color, solid(), 1., None, false)
                .map(|_| ())
        }),
        entry("fill（消す）", true, true, |d, id| {
            d.fill(id, Channel::Color, solid(), 1., None, true)
                .map(|_| ())
        }),
        entry("fill_material", true, false, |d, id| {
            d.fill_material(id, &two_channels(), 1., None, false)
                .map(|_| ())
        }),
        entry("fill_material（消す）", true, true, |d, id| {
            d.fill_material(id, &two_channels(), 1., None, true)
                .map(|_| ())
        }),
        entry("gradient", true, false, |d, id| {
            d.gradient(id, Channel::Color, &ramp(), None, false)
                .map(|_| ())
        }),
        entry("gradient（消す）", true, true, |d, id| {
            d.gradient(id, Channel::Color, &ramp(), None, true)
                .map(|_| ())
        }),
        entry("gradient_material", true, false, |d, id| {
            d.gradient_material(id, &two_channels(), None, &ramp(), None, false)
                .map(|_| ())
        }),
        entry("gradient_material（消す）", true, true, |d, id| {
            d.gradient_material(id, &two_channels(), None, &ramp(), None, true)
                .map(|_| ())
        }),
        entry(
            "gradient_material（終点つき）",
            true,
            false,
            |d, id| {
                let to = [
                    ChannelPaint::new(Channel::Emission, Rgba8::new(1, 2, 3, 255)),
                    ChannelPaint::new(Channel::Color, Rgba8::new(4, 5, 6, 255)),
                ];
                d.gradient_material(id, &two_channels(), Some(&to), &ramp(), None, false)
                    .map(|_| ())
            },
        ),
        // ここから下はマスクへの書き込み: 画像・透明部分のロックでは通り、すべてのロックでだけ断る
        entry("begin_mask_stroke", false, false, |d, id| {
            let s = d.begin_mask_stroke(id, &hard());
            stroke_entry(d, s)
        }),
        entry("begin_brush_mask_stroke", false, false, |d, id| {
            let s = d.begin_brush_mask_stroke(id, &Brush::from(hard()));
            stroke_entry(d, s)
        }),
        entry("begin_mask_triangle_fill", false, false, |d, id| {
            let f = d.begin_mask_triangle_fill(id, 1., false);
            fill_entry(d, f)
        }),
        entry("fill_mask", false, false, |d, id| {
            d.fill_mask(id, 1., None, false).map(|_| ())
        }),
        entry("gradient_mask", false, false, |d, id| {
            d.gradient_mask(id, &ramp(), None, false).map(|_| ())
        }),
        entry("apply_smart_mask", false, false, |d, id| {
            let m = d.capture_smart_mask(id, "マスク")?;
            d.apply_smart_mask(&m, id, None).map(|_| ())
        }),
    ]
}
/// ロックの種類ごとの、入口が断るか（断るなら名指すロック）。
fn expected_refusal(entry: &Entry, lock: LayerLocks) -> Option<LayerLocks> {
    if lock == LayerLocks::ALL {
        Some(LayerLocks::ALL)
    } else if lock == LayerLocks::PIXELS && entry.refused_by_pixels {
        Some(LayerLocks::PIXELS)
    } else if lock == LayerLocks::TRANSPARENCY && entry.erases {
        Some(LayerLocks::TRANSPARENCY)
    } else {
        None
    }
}
#[test]
fn every_pixel_write_entry_goes_through_the_layer_locks() {
    let entries = write_entries();
    // 入口の名前が重ならない（表の取り違えを防ぐ）
    let mut names: Vec<_> = entries.iter().map(|e| e.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), entries.len());
    let (mut refused, mut passed) = (0, 0);
    for lock in [
        LayerLocks::NONE,
        LayerLocks::TRANSPARENCY,
        LayerLocks::PIXELS,
        LayerLocks::POSITION,
        LayerLocks::ALL,
    ] {
        for e in &entries {
            // レイヤー自身のロックと、親のグループのロック（持ち主はグループ）
            for grouped in [false, true] {
                let (mut d, id) = locked_target();
                let holder = if grouped {
                    d.group_layers(&[id], "親").unwrap()
                } else {
                    id
                };
                d.set_layer_locks(holder, lock).unwrap();
                d.clear_history().unwrap();
                let before = deep_fingerprint(&d);
                let result = (e.run)(&mut d, id);
                let context = format!("{} / {lock:?} / grouped={grouped}", e.name);
                match expected_refusal(e, lock) {
                    Some(named) => {
                        assert_eq!(
                            result,
                            Err(CoreError::LayerLocked {
                                layer: id,
                                holder,
                                lock: named
                            }),
                            "{context}"
                        );
                        // 断ったあとは何も変わらない（変更番号・履歴・全チャンネルの有効と画素・マスク・進行中のストローク）
                        assert_eq!(deep_fingerprint(&d), before, "{context}");
                        // 断ったのはロックだと、外せば同じ入口が通ることで確かめる
                        d.set_layer_locks(holder, LayerLocks::NONE).unwrap();
                        assert!((e.run)(&mut d, id).is_ok(), "{context}");
                        refused += 1;
                    }
                    None => {
                        assert_eq!(result, Ok(()), "{context}");
                        assert!(!d.has_active_stroke(), "{context}");
                        passed += 1;
                    }
                }
            }
        }
    }
    // 事例の数は規則から数えた値で固定する（規則が崩れて数がずれたら落ちる）。26 入口 × ロック 5 種 × レイヤー/親グループ 2 = 260 事例のうち、
    // 断るのは すべて 26 入口 × 2 = 52、画像 画素の 20 入口 × 2 = 40、透明部分 消す 8 入口 × 2 = 16 の計 108、通るのは残りの 152
    assert_eq!(entries.len(), 26);
    assert_eq!((refused, passed), (108, 152));
}
#[test]
fn refused_material_writes_leave_no_channel_enabled_and_no_surface() {
    // 無効のチャンネル（Emission）を有効にして面を作るのはマテリアルの入口だけ。断るのは、それより前
    for lock in [LayerLocks::PIXELS, LayerLocks::ALL] {
        for e in write_entries().iter().filter(|e| e.refused_by_pixels) {
            let (mut d, id) = locked_target();
            d.set_layer_locks(id, lock).unwrap();
            d.clear_history().unwrap();
            let revision = d.revision();
            assert!((e.run)(&mut d, id).is_err(), "{}", e.name);
            let layer = d.layer(id).unwrap();
            assert!(!layer.is_channel_enabled(Channel::Emission), "{}", e.name);
            assert!(layer.surface(Channel::Emission).is_none(), "{}", e.name);
            assert_eq!(d.revision(), revision, "{}", e.name);
            assert_eq!(d.undo_count(), 0, "{}", e.name);
            // 断ったあとにマテリアルの状態が残らない: ロックを外せばすぐ次のストロークが始まる
            d.set_layer_locks(id, LayerLocks::NONE).unwrap();
            let s = d
                .begin_material_stroke(id, &two_channels(), &hard())
                .unwrap();
            d.cancel_stroke(s);
            assert!(!d.has_active_stroke(), "{}", e.name);
        }
    }
}

/// 3 チャンネルに画素のあるレイヤー（透明な画素と、透明なのに RGB が残る画素を含む）。
fn three_channels() -> (Document, LayerId, Vec<ChannelPaint>) {
    let mut d = Document::with_tile_size(9, 7, 4).unwrap();
    let id = d.add_layer("レイヤー").unwrap();
    let material = vec![
        ChannelPaint::new(Channel::Color, Rgba8::new(240, 20, 30, 220)),
        ChannelPaint::new(Channel::Emission, Rgba8::new(10, 220, 40, 200)),
        ChannelPaint::new(Channel::Roughness, Rgba8::new(90, 90, 90, 255)),
    ];
    for (c, m) in material.iter().enumerate() {
        for y in 0..7 {
            for x in 0..9 {
                let a = match (x + y + c as u32) % 4 {
                    0 => 0,
                    1 => 255,
                    _ => 70 + (x * 13 + y * 7) as u8,
                };
                d.set_channel_pixel(
                    id,
                    m.channel,
                    x,
                    y,
                    Rgba8::new((x * 23) as u8, (y * 31 + c as u32 * 40) as u8, 71, a),
                )
                .unwrap();
            }
        }
    }
    d.clear_history().unwrap();
    (d, id, material)
}
fn channel_pixels(d: &Document, id: LayerId, channel: Channel) -> Vec<Rgba8> {
    (0..d.height())
        .flat_map(|y| {
            (0..d.width()).map(move |x| d.layer(id).unwrap().pixel(channel, x, y).unwrap())
        })
        .collect()
}
/// 透明部分のロック（レイヤーか親のグループ）の下で、マテリアルの塗り 4 種と単チャンネルの 2 種（グラデーション・三角形の塗り）が、
/// 塗った全チャンネルのアルファを変えず、透明画素の RGB も保つ。色は実際に変わる（空振りでない）。Undo で元の画素へ、Redo で同じ結果へ。
#[test]
fn transparency_lock_keeps_alpha_and_hidden_rgb_in_every_channel_of_every_material_write() {
    type Write = Box<dyn Fn(&mut Document, LayerId, &[ChannelPaint]) -> Result<(), CoreError>>;
    let writes: Vec<(&str, Write, Vec<Channel>)> = vec![
        (
            "begin_material_stroke",
            Box::new(|d, id, m| {
                let mut s = d.begin_material_stroke(id, m, &hard())?;
                s.add_point(d, 4., 3., 1., glam::DVec2::ZERO)?;
                d.end_stroke(s).map(|_| ())
            }),
            vec![Channel::Color, Channel::Emission, Channel::Roughness],
        ),
        (
            "fill_material",
            Box::new(|d, id, m| d.fill_material(id, m, 0.8, None, false).map(|_| ())),
            vec![Channel::Color, Channel::Emission, Channel::Roughness],
        ),
        (
            "gradient_material",
            Box::new(|d, id, m| {
                d.gradient_material(id, m, None, &ramp(), None, false)
                    .map(|_| ())
            }),
            vec![Channel::Color, Channel::Emission, Channel::Roughness],
        ),
        (
            "gradient_material（終点つき）",
            Box::new(|d, id, m| {
                let to: Vec<_> = m
                    .iter()
                    .map(|p| ChannelPaint::new(p.channel, Rgba8::new(5, 6, 250, 255)))
                    .collect();
                d.gradient_material(id, m, Some(&to), &ramp(), None, false)
                    .map(|_| ())
            }),
            vec![Channel::Color, Channel::Emission, Channel::Roughness],
        ),
        (
            "begin_material_triangle_fill",
            Box::new(|d, id, m| {
                let mut f = d.begin_material_triangle_fill(id, m, 0.9, false)?;
                f.add(d, &triangles())?;
                f.commit(d).map(|_| ())
            }),
            vec![Channel::Color, Channel::Emission, Channel::Roughness],
        ),
        (
            "gradient",
            Box::new(|d, id, _| {
                d.gradient(id, Channel::Color, &ramp(), None, false)
                    .map(|_| ())
            }),
            vec![Channel::Color],
        ),
        (
            "begin_triangle_fill",
            Box::new(|d, id, _| {
                let mut f = d.begin_triangle_fill(id, Channel::Color, solid(), 0.9, false)?;
                f.add(d, &triangles())?;
                f.commit(d).map(|_| ())
            }),
            vec![Channel::Color],
        ),
    ];
    for (name, write, channels) in &writes {
        for grouped in [false, true] {
            let (mut d, id, material) = three_channels();
            let holder = if grouped {
                d.group_layers(&[id], "親").unwrap()
            } else {
                id
            };
            d.set_layer_locks(holder, LayerLocks::TRANSPARENCY).unwrap();
            d.clear_history().unwrap();
            let before: Vec<_> = Channel::ALL
                .iter()
                .map(|&c| {
                    d.layer(id)
                        .unwrap()
                        .surface(c)
                        .map(|_| channel_pixels(&d, id, c))
                })
                .collect();
            write(&mut d, id, &material).unwrap();
            assert_eq!(d.undo_count(), 1, "{name}: 1 回の Undo");
            let after: Vec<_> = Channel::ALL
                .iter()
                .map(|&c| {
                    d.layer(id)
                        .unwrap()
                        .surface(c)
                        .map(|_| channel_pixels(&d, id, c))
                })
                .collect();
            for (i, &c) in Channel::ALL.iter().enumerate() {
                let (Some(b), Some(a)) = (&before[i], &after[i]) else {
                    continue;
                };
                if !channels.contains(&c) {
                    assert_eq!(a, b, "{name}: 塗らないチャンネル {c:?}");
                    continue;
                }
                let mut changed = 0;
                for (p, q) in a.iter().zip(b) {
                    assert_eq!(p.a, q.a, "{name}: {c:?} のアルファ");
                    if q.a == 0 {
                        assert_eq!(p, q, "{name}: {c:?} の透明画素の RGB");
                    } else if p != q {
                        changed += 1;
                    }
                }
                assert!(changed > 0, "{name}: {c:?} の色が変わっていない");
            }
            d.undo().unwrap();
            for (i, &c) in Channel::ALL.iter().enumerate() {
                if let Some(b) = &before[i] {
                    assert_eq!(&channel_pixels(&d, id, c), b, "{name}: Undo {c:?}");
                }
            }
            d.redo().unwrap();
            for (i, &c) in Channel::ALL.iter().enumerate() {
                if let Some(a) = &after[i] {
                    assert_eq!(&channel_pixels(&d, id, c), a, "{name}: Redo {c:?}");
                }
            }
        }
    }
}
/// マテリアルの組の画素は、各値を単独で塗ったもの（`begin_stroke_in`・`fill`）とバイト一致する（透明部分のロックの下でも）。
#[test]
fn locked_material_writes_equal_painting_each_channel_alone() {
    let (mut together, id, material) = three_channels();
    together
        .set_layer_locks(id, LayerLocks::TRANSPARENCY)
        .unwrap();
    let (mut alone, id2, _) = three_channels();
    alone
        .set_layer_locks(id2, LayerLocks::TRANSPARENCY)
        .unwrap();
    // ストローク
    let mut s = together
        .begin_material_stroke(id, &material, &hard())
        .unwrap();
    s.add_point(&mut together, 4., 3., 1., glam::DVec2::ZERO)
        .unwrap();
    together.end_stroke(s).unwrap();
    for m in &material {
        let mut settings = hard();
        settings.color = m.value;
        let mut s = alone.begin_stroke_in(id2, m.channel, &settings).unwrap();
        s.add_point(&mut alone, 4., 3., 1., glam::DVec2::ZERO)
            .unwrap();
        alone.end_stroke(s).unwrap();
    }
    for m in &material {
        assert_eq!(
            channel_pixels(&together, id, m.channel),
            channel_pixels(&alone, id2, m.channel),
            "ストローク {:?}",
            m.channel
        );
    }
    // 範囲の塗り
    together
        .fill_material(id, &material, 0.7, None, false)
        .unwrap();
    for m in &material {
        alone
            .fill(id2, m.channel, m.value, 0.7, None, false)
            .unwrap();
    }
    for m in &material {
        assert_eq!(
            channel_pixels(&together, id, m.channel),
            channel_pixels(&alone, id2, m.channel),
            "範囲の塗り {:?}",
            m.channel
        );
    }
}
/// 三角形の塗りを取り消すと、透明部分のロックの下でも画素・履歴・無効にしていたチャンネルが元へ戻る。
#[test]
fn cancelled_locked_triangle_fill_restores_pixels_history_and_channels() {
    let (mut d, id) = locked_target();
    d.set_layer_locks(id, LayerLocks::TRANSPARENCY).unwrap();
    d.clear_history().unwrap();
    let before = deep_fingerprint(&d);
    let mut f = d
        .begin_material_triangle_fill(id, &two_channels(), 1., false)
        .unwrap();
    assert!(f.add(&mut d, &triangles()).unwrap());
    assert!(d.layer(id).unwrap().is_channel_enabled(Channel::Emission));
    f.cancel(&mut d);
    let after = deep_fingerprint(&d);
    // 取り消しは変更番号と変化の記録を進める。それ以外（履歴・全チャンネルの有効と画素・マスク）は元のまま
    assert_eq!(after.base.undo, before.base.undo);
    assert_eq!(after.base.redo, before.base.redo);
    assert_eq!(after.channels, before.channels);
    assert_eq!(after.masks, before.masks);
    assert!(!after.active);
}

// ───────── 型の拒否とロックの拒否の順・マスクの有無・予算の拒否・手動の ID 色 ─────────

/// 塗りつぶし・調整・グループのレイヤー（全体のマスク付き）。kind: 0 塗りつぶし・1 調整・2 グループ。
fn non_raster_target(kind: u32) -> (Document, LayerId) {
    let mut d = Document::with_tile_size(9, 7, 4).unwrap();
    let id = match kind {
        0 => d.add_fill_layer(
            "n",
            &[
                (Channel::Color, solid()),
                (Channel::Emission, Rgba8::new(200, 100, 50, 255)),
            ],
            None,
        ),
        1 => d.add_adjustment_layer("n", AdjustmentSettings::invert(), None, None),
        _ => d.add_group("n", None),
    }
    .unwrap();
    d.add_layer_mask(id).unwrap();
    d.clear_history().unwrap();
    (d, id)
}
fn is_type_refusal(r: &Result<(), CoreError>) -> bool {
    matches!(r, Err(CoreError::Unsupported(_)))
}
/// 塗りつぶし・調整・グループのレイヤーへの書き込みは、型で断るのとロックで断るのとで順が決まっている（C# と同じ。tests/reference/material_golden.rs の
/// locknr が全バイトで照らす）。画素の入口は型が先で、ロックの名指しは出ない。ただし単チャンネルのストローク（begin_stroke の 4 入口）は
/// 調整・グループではロックが先（C# の BeginStroke は面を取る GetChannel が型で断るのをロックの検査のあとに置く）で、塗りつぶしだけ型が先。
/// マスクへの書き込みはどの種類のレイヤーにも通り、すべてのロックでだけ断る。断ったあとは何も変わらない。
#[test]
fn non_raster_layers_refuse_by_type_before_the_lock_except_begin_stroke_on_adjustments_and_groups()
{
    let lock_first = [
        "begin_stroke",
        "begin_stroke（消す）",
        "begin_stroke_in",
        "begin_brush_stroke",
    ];
    let (mut by_type, mut by_lock, mut written) = (0, 0, 0);
    for lock in [
        LayerLocks::NONE,
        LayerLocks::TRANSPARENCY,
        LayerLocks::PIXELS,
        LayerLocks::POSITION,
        LayerLocks::ALL,
    ] {
        for grouped in [false, true] {
            for kind in 0..3 {
                for e in &write_entries() {
                    let (mut d, id) = non_raster_target(kind);
                    let holder = if grouped {
                        d.group_layers(&[id], "親").unwrap()
                    } else {
                        id
                    };
                    d.set_layer_locks(holder, lock).unwrap();
                    d.clear_history().unwrap();
                    let before = deep_fingerprint(&d);
                    let result = (e.run)(&mut d, id);
                    let context =
                        format!("{} / {lock:?} / grouped={grouped} / kind={kind}", e.name);
                    let named = expected_refusal(e, lock);
                    let masks = !e.refused_by_pixels;
                    if masks || (lock_first.contains(&e.name) && kind != 0) {
                        match named {
                            Some(named) => {
                                assert_eq!(result, Err(locked(id, holder, named)), "{context}");
                                by_lock += 1;
                            }
                            None if masks => {
                                assert_eq!(result, Ok(()), "{context}");
                                written += 1;
                            }
                            None => {
                                assert!(is_type_refusal(&result), "{context}: {result:?}");
                                by_type += 1;
                            }
                        }
                    } else {
                        assert!(is_type_refusal(&result), "{context}: {result:?}");
                        by_type += 1;
                    }
                    if result.is_err() {
                        assert_eq!(deep_fingerprint(&d), before, "{context}");
                    }
                    assert!(!d.has_active_stroke(), "{context}");
                }
            }
        }
    }
    // 26 入口 × ロック 5 種 × レイヤー/親グループ 2 × レイヤーの種類 3 = 780 事例。ロックで断る 72（マスクの 6 入口 × すべて 2 × 3 種 = 36 と、
    // 調整・グループの begin_stroke の 4 入口 × すべて・画像 2 種 × 2 × 2 種 = 32 に、消す 1 入口 × 透明部分 × 2 × 2 種 = 4）、
    // 通る 144（マスクの 6 入口 × 通る 4 種 × 2 × 3 種）、残りは型で断る
    assert_eq!((by_lock, written, by_type), (72, 144, 780 - 72 - 144));
}
/// マスクの無いレイヤーは、マスクへ書く入口もマスクを変える入口も、ロックに関わらず「マスクが無い」で断る（C# は RequireMask がロックの検査より先。
/// すべてのロックは、マスクの有無を確かめたあとにだけ効く）。断ったあとは何も変わらない。スマートマスクの適用は、マスクの無いレイヤーへ新しく付けるので対象外。
#[test]
fn mask_entries_refuse_a_layer_without_a_mask_before_the_lock() {
    let no_mask = Err(CoreError::Unsupported("レイヤーにマスクが無い"));
    type Edit = Box<dyn Fn(&mut Document, LayerId) -> Result<(), CoreError>>;
    let mut edits: Vec<(&str, Edit)> = Vec::new();
    for e in write_entries()
        .into_iter()
        .filter(|e| !e.refused_by_pixels && e.name != "apply_smart_mask")
    {
        edits.push((e.name, e.run));
    }
    edits.push((
        "remove_layer_mask",
        Box::new(|d, id| d.remove_layer_mask(id)),
    ));
    edits.push((
        "set_layer_mask_enabled",
        Box::new(|d, id| d.set_layer_mask_enabled(id, false)),
    ));
    edits.push((
        "set_layer_mask_inverted",
        Box::new(|d, id| d.set_layer_mask_inverted(id, true)),
    ));
    edits.push((
        "set_layer_mask_density",
        Box::new(|d, id| d.set_layer_mask_density(id, 0.5, false)),
    ));
    assert_eq!(edits.len(), 5 + 4);
    let mut refused = 0;
    for lock in [
        LayerLocks::NONE,
        LayerLocks::TRANSPARENCY,
        LayerLocks::PIXELS,
        LayerLocks::POSITION,
        LayerLocks::ALL,
    ] {
        for grouped in [false, true] {
            for (name, edit) in &edits {
                let (mut d, id) = patterned();
                let holder = if grouped {
                    d.group_layers(&[id], "親").unwrap()
                } else {
                    id
                };
                d.set_layer_locks(holder, lock).unwrap();
                d.clear_history().unwrap();
                let before = deep_fingerprint(&d);
                let result = edit(&mut d, id);
                assert_eq!(result, no_mask, "{name} / {lock:?} / grouped={grouped}");
                assert_eq!(deep_fingerprint(&d), before, "{name} / {lock:?}");
                refused += 1;
            }
        }
    }
    assert_eq!(refused, 9 * 5 * 2);
    // マスクを足すほうは逆で、もうマスクのあるレイヤーはロックに関わらず「もうマスクを持っている」で断る（C# の AddLayerMask）
    for lock in [LayerLocks::NONE, LayerLocks::ALL] {
        let (mut d, id) = locked_target();
        d.set_layer_locks(id, lock).unwrap();
        d.clear_history().unwrap();
        assert_eq!(
            d.add_layer_mask(id),
            Err(CoreError::Unsupported("レイヤーはもうマスクを持っている")),
            "{lock:?}"
        );
    }
    // マスクがあれば、すべてのロックは今までどおり効く
    let (mut d, id) = locked_target();
    d.set_layer_locks(id, LayerLocks::ALL).unwrap();
    d.clear_history().unwrap();
    for result in [
        d.remove_layer_mask(id),
        d.set_layer_mask_enabled(id, false),
        d.set_layer_mask_inverted(id, true),
        d.set_layer_mask_density(id, 0.5, false),
    ] {
        assert_eq!(result, Err(locked(id, id, LayerLocks::ALL)));
    }
}

/// 透明部分のロックを立てた、チャンネルに画素のあるレイヤーへの、予算を超えうるマテリアルの書き込み（ストロークは点を足して確定、
/// 範囲の塗りは 1 回、三角形の塗りは三角形を足して確定）。
fn budgeted_writes() -> Vec<(&'static str, WriteEntry)> {
    vec![
        (
            "begin_material_stroke",
            Box::new(|d, id| {
                let mut s = d.begin_material_stroke(id, &two_channels(), &hard())?;
                s.add_point(d, 4., 3., 1., glam::DVec2::ZERO)?;
                d.end_stroke(s).map(|_| ())
            }),
        ),
        (
            "fill_material",
            Box::new(|d, id| {
                d.fill_material(id, &two_channels(), 1., None, false)
                    .map(|_| ())
            }),
        ),
        (
            "gradient_material",
            Box::new(|d, id| {
                d.gradient_material(id, &two_channels(), None, &ramp(), None, false)
                    .map(|_| ())
            }),
        ),
        (
            "begin_material_triangle_fill",
            Box::new(|d, id| {
                let mut f = d.begin_material_triangle_fill(id, &two_channels(), 1., false)?;
                f.add(d, &triangles())?;
                f.commit(d).map(|_| ())
            }),
        ),
    ]
}
/// 断ったあとも変わらないもの（変更番号と変化の記録は、取り消しで進むので除く）: 履歴・レイヤー・全チャンネルの有効と画素・マスク・進行中のストローク。
fn assert_restored(after: &DeepFingerprint, before: &DeepFingerprint, context: &str) {
    assert_eq!(after.base.undo, before.base.undo, "{context}: undo");
    assert_eq!(after.base.redo, before.base.redo, "{context}: redo");
    assert_eq!(after.base.layers, before.base.layers, "{context}: layers");
    assert_eq!(after.base.pixels, before.base.pixels, "{context}: pixels");
    assert_eq!(after.channels, before.channels, "{context}: channels");
    assert_eq!(after.masks, before.masks, "{context}: masks");
    assert!(!after.active, "{context}: active");
}
/// 透明部分のロックの下でも、ストロークの予算で断られたら元へ戻る（画素・無効だったチャンネルの有効と面・履歴）。予算を 0 から増やし、
/// 断られる間は元のまま、通ったら 1 回の Undo になる。ロックの下では透明な画素のタイルを写さない（Emission は空なので写しは Color の分だけ）
/// ので、同じ書き込みのロック無しより少ない予算で通る（三角形の塗りだけは、触れたタイルを対象ごとに丸ごと写すので同じ）。巻き戻しの数え方の
/// C# との差（チャンネルごとの覆い）は material_golden の rollback が持つ。ロックの下の量を C# と照らす行は無い（予算の拒否は Rust の試験だけ）。
#[test]
fn locked_writes_refused_by_the_stroke_budget_leave_everything_as_it_was() {
    for (name, write) in budgeted_writes() {
        for grouped in [false, true] {
            let mut first_ok = [None, None]; // [透明部分のロック有り, 無し]
            for (i, lock) in [LayerLocks::TRANSPARENCY, LayerLocks::NONE]
                .into_iter()
                .enumerate()
            {
                let mut refused = 0;
                for budget in 0..8192 {
                    let (mut d, id) = locked_target();
                    let holder = if grouped {
                        d.group_layers(&[id], "親").unwrap()
                    } else {
                        id
                    };
                    d.set_layer_locks(holder, lock).unwrap();
                    d.clear_history().unwrap();
                    d.set_stroke_budget_bytes(budget).unwrap();
                    let before = deep_fingerprint(&d);
                    let context =
                        format!("{name} / {lock:?} / grouped={grouped} / budget={budget}");
                    match write(&mut d, id) {
                        Err(CoreError::StrokeBudgetExceeded) => {
                            assert_restored(&deep_fingerprint(&d), &before, &context);
                            // 断ったあとにマテリアルの状態が残らない: 予算を戻せば次のストロークが始まる
                            d.set_stroke_budget_bytes(64 << 20).unwrap();
                            let s = d
                                .begin_material_stroke(id, &two_channels(), &hard())
                                .unwrap();
                            d.cancel_stroke(s);
                            refused += 1;
                        }
                        Ok(()) => {
                            assert_eq!(d.undo_count(), 1, "{context}");
                            first_ok[i] = Some(budget);
                            break;
                        }
                        Err(e) => panic!("{context}: {e:?}"),
                    }
                }
                assert!(refused > 0, "{name} / {lock:?}: 断られた事例が無い");
            }
            let (locked_need, free_need) = (first_ok[0].unwrap(), first_ok[1].unwrap());
            if name == "begin_material_triangle_fill" {
                // 三角形の塗りは、触れたタイルを対象ごとに丸ごと写す（C# と同じ。透明かどうかを見ない）ので、ロックの有無で同じ
                assert_eq!(locked_need, free_need, "{name} / grouped={grouped}");
            } else {
                assert!(
                    locked_need < free_need,
                    "{name} / grouped={grouped}: 透明部分のロックの下は写しが少ない {locked_need} {free_need}"
                );
            }
        }
    }
}
/// 透明部分のロックの下では新しいタイルを作らない（透明な画素は変わらない）ので、画素の予算が足りなくても通る。ロックを外せば同じ予算で
/// 断られ（SourceBudgetExceeded）、元へ戻る。
#[test]
fn transparency_lock_writes_need_no_pixel_budget_and_the_unlocked_refusal_restores() {
    for (name, write) in budgeted_writes() {
        // ロック有り: 予算が今の量ちょうどでも通り、確保量は増えない
        let (mut d, id) = locked_target();
        d.set_layer_locks(id, LayerLocks::TRANSPARENCY).unwrap();
        d.clear_history().unwrap();
        let used = d.allocated_bytes();
        d.set_source_budget_bytes(used).unwrap();
        write(&mut d, id).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(d.undo_count(), 1, "{name}");
        assert_eq!(d.allocated_bytes(), used, "{name}");
        // ロック無し: 同じ予算では Emission の新しいタイルを作れず断る。元のまま
        let (mut d, id) = locked_target();
        d.clear_history().unwrap();
        d.set_source_budget_bytes(d.allocated_bytes()).unwrap();
        let before = deep_fingerprint(&d);
        let result = write(&mut d, id);
        assert_eq!(result, Err(CoreError::SourceBudgetExceeded), "{name}");
        assert_restored(&deep_fingerprint(&d), &before, name);
        assert_eq!(d.allocated_bytes(), used, "{name}");
    }
}

/// 手動の ID 色（塊の番号と色の結び付け）は画素でもレイヤーでもないので、レイヤーの操作・変形・サイズ変更では変わらない（準備用の文書へ写さず・交換しない）。
/// 実行の前後も、Undo・Redo のあとも同じ。C# の Resampled は別の文書を返すので ID 色を持ち越さないが、Rust は同じ文書の中で交換するので値が
/// そのまま残る（意図した違い）。
#[test]
fn manual_id_colours_survive_layer_operations_transforms_and_resizes() {
    let colours = || {
        yolu_core::mesh_maps::IdColorAssignments::new(
            "ab".repeat(32),
            std::collections::BTreeMap::from([(0usize, 0xff0000u32), (3, 0x00ff80)]),
        )
        .unwrap()
    };
    type Op = Box<dyn Fn(&mut Document, &[LayerId]) -> Result<(), CoreError>>;
    let ops: Vec<(&str, Op)> = vec![
        (
            "resize_image",
            Box::new(|d, _| {
                d.resize_image(18, 14, CanvasResampling::Nearest)
                    .map(|_| ())
            }),
        ),
        (
            "resize_canvas",
            Box::new(|d, _| d.resize_canvas(12, 9, (1, 1)).map(|_| ())),
        ),
        (
            "transform_layer",
            Box::new(|d, l| {
                d.transform_layer(
                    l[0],
                    Affine2D::translation(1., 0.),
                    Resampling::Nearest,
                    true,
                )
                .map(|_| ())
            }),
        ),
        (
            "transform_layers",
            Box::new(|d, l| {
                d.transform_layers(&l[..2], Affine2D::translation(1., 0.), Resampling::Nearest)
                    .map(|_| ())
            }),
        ),
        (
            "merge_down",
            Box::new(|d, l| d.merge_down(l[1], 255).map(|_| ())),
        ),
        (
            "merge_layers",
            Box::new(|d, l| d.merge_layers(&l[..2], 255).map(|_| ())),
        ),
        (
            "merge_visible",
            Box::new(|d, _| d.merge_visible("結合", 255).map(|_| ())),
        ),
        ("remove_layers", Box::new(|d, l| d.remove_layers(&l[..2]))),
        (
            "duplicate_layers",
            Box::new(|d, l| d.duplicate_layers(&l[..2]).map(|_| ())),
        ),
        (
            "move_layers",
            Box::new(|d, l| d.move_layers(&l[..1], None, 2)),
        ),
        (
            "change_layer_locks",
            Box::new(|d, l| d.change_layer_locks(&l[..2], LayerLocks::PIXELS, true)),
        ),
    ];
    for (name, op) in &ops {
        let (mut d, a) = patterned();
        let b = d.duplicate_layer(a, None).unwrap();
        let c = d.duplicate_layer(a, None).unwrap();
        d.set_id_colors(colours(), false).unwrap();
        d.clear_history().unwrap();
        let same = |d: &Document, when: &str| {
            assert_eq!(
                d.id_colors().binding(),
                colours().binding(),
                "{name} {when}"
            );
            assert_eq!(d.id_colors().colors(), colours().colors(), "{name} {when}");
        };
        op(&mut d, &[a, b, c]).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(
            d.undo_count(),
            1,
            "{name}: 1 回の Undo（何もしない操作では確かめにならない）"
        );
        same(&d, "実行後");
        d.undo().unwrap();
        same(&d, "Undo 後");
        d.redo().unwrap();
        same(&d, "Redo 後");
    }
}
