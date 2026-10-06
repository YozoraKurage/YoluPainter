use yolu_core::*;
fn scene(tile: u32) -> (Document, LayerId) {
    let mut d = Document::with_tile_size(16, 16, tile).unwrap();
    let id = d.add_layer("試験").unwrap();
    for y in 0..16 {
        for x in 0..16 {
            d.set_pixel(
                id,
                x,
                y,
                Rgba8::new(
                    (x * 13) as u8,
                    (y * 15) as u8,
                    91,
                    if (x + y) % 3 == 0 { 0 } else { 170 },
                ),
            )
            .unwrap();
        }
    }
    d.clear_history().unwrap();
    (d, id)
}
fn bytes(d: &Document, id: LayerId) -> Vec<Rgba8> {
    (0..16)
        .flat_map(|y| {
            (0..16).map(move |x| d.layer(id).unwrap().pixel(Channel::Color, x, y).unwrap())
        })
        .collect()
}
fn translation() -> Warp {
    Warp::Projective(
        Homography::from_quads(
            [(0., 0.), (16., 0.), (16., 16.), (0., 16.)],
            [(1., 0.), (17., 0.), (17., 16.), (1., 16.)],
        )
        .unwrap(),
    )
}
fn deform() -> Warp {
    let mut m = WarpMesh::new([0., 0., 16., 16.], 4, 4).unwrap();
    m.points[12] = (9., 9.);
    Warp::Mesh(m)
}
fn run(d: &mut Document, id: LayerId, w: &Warp) -> Result<bool, CoreError> {
    d.warp_layers_cancellable(&[id], w, &mut || false)
}
#[test]
fn identity_is_byte_exact_including_hidden_rgb_and_has_no_history() {
    for w in [
        Warp::Projective(Homography::IDENTITY),
        Warp::Mesh(WarpMesh::new([0., 0., 16., 16.], 4, 4).unwrap()),
    ] {
        let (mut d, id) = scene(4);
        let b = bytes(&d, id);
        let rev = d.revision();
        assert!(!run(&mut d, id, &w).unwrap());
        assert_eq!(bytes(&d, id), b);
        assert_eq!(d.revision(), rev);
        assert_eq!(d.undo_count(), 0);
    }
}
#[test]
fn projective_maps_known_corners_and_inverse() {
    let a = [(0., 0.), (16., 0.), (16., 16.), (0., 16.)];
    let b = [(2., 1.), (14., 3.), (18., 14.), (-1., 19.)];
    let h = Homography::from_quads(a, b).unwrap();
    let inv = h.inverse().unwrap();
    for (p, q) in a.into_iter().zip(b) {
        let at = h.apply(p.0, p.1);
        assert!((at.0 - q.0).abs() < 1e-9 && (at.1 - q.1).abs() < 1e-9);
        let back = inv.apply(at.0, at.1);
        assert!((back.0 - p.0).abs() < 1e-9 && (back.1 - p.1).abs() < 1e-9);
    }
}
#[test]
fn integer_projective_preserves_hidden_rgb_and_undo_redo() {
    let (mut d, id) = scene(4);
    let b = bytes(&d, id);
    run(&mut d, id, &translation()).unwrap();
    let after = bytes(&d, id);
    for y in 0..16 {
        for x in 1..16 {
            assert_eq!(after[y * 16 + x], b[y * 16 + x - 1]);
        }
    }
    assert_eq!(d.undo_count(), 1);
    d.undo().unwrap();
    assert_eq!(bytes(&d, id), b);
    d.redo().unwrap();
    assert_eq!(bytes(&d, id), after);
}
#[test]
fn non_affine_is_deterministic_across_tiles_and_threads() {
    for warp in [
        deform(),
        Warp::Projective(
            Homography::from_quads(
                [(0., 0.), (16., 0.), (16., 16.), (0., 16.)],
                [(1., 0.), (15., 2.), (16., 16.), (0., 14.)],
            )
            .unwrap(),
        ),
        Warp::Liquify(vec![dab(LiquifyMode::Clockwise)]),
    ] {
        let mut expected = None;
        for tile in [2, 4, 8] {
            for threads in [1, 3] {
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .unwrap();
                let (mut d, id) = scene(tile);
                pool.install(|| run(&mut d, id, &warp).unwrap());
                let got = bytes(&d, id);
                if let Some(e) = &expected {
                    assert_eq!(&got, e);
                } else {
                    expected = Some(got);
                }
            }
        }
    }
}
#[test]
fn budgets_cancellation_locks_and_types_are_atomic() {
    let (mut d, id) = scene(4);
    let b = bytes(&d, id);
    let serial = d.change_serial();
    let rev = d.revision();
    d.set_stroke_budget_bytes(0).unwrap();
    assert_eq!(
        run(&mut d, id, &deform()),
        Err(CoreError::StrokeBudgetExceeded)
    );
    d.set_stroke_budget_bytes(1 << 20).unwrap();
    let mut calls = 0;
    assert_eq!(
        d.warp_layers_cancellable(&[id], &deform(), &mut || {
            calls += 1;
            calls > 2
        }),
        Err(CoreError::Cancelled)
    );
    assert_eq!(bytes(&d, id), b);
    assert_eq!(d.change_serial(), serial);
    assert_eq!(d.revision(), rev);
    assert_eq!(d.undo_count(), 0);
    for lock in [LayerLocks::POSITION, LayerLocks::PIXELS, LayerLocks::ALL] {
        d.set_layer_locks(id, lock).unwrap();
        d.clear_history().unwrap();
        assert!(matches!(
            run(&mut d, id, &deform()),
            Err(CoreError::LayerLocked { .. })
        ));
        assert_eq!(bytes(&d, id), b);
        assert_eq!(d.undo_count(), 0);
    }
    let group = d.add_group("グループ", None).unwrap();
    assert!(run(&mut d, group, &translation()).is_err());
}
#[test]
fn invalid_mesh_and_quad_do_not_touch_document() {
    assert!(Homography::from_quads(
        [(0., 0.), (1., 0.), (1., 1.), (0., 1.)],
        [(0., 0.), (1., 1.), (1., 0.), (0., 1.)]
    )
    .is_err());
    let (mut d, id) = scene(4);
    let b = bytes(&d, id);
    let mut m = WarpMesh::new([0., 0., 16., 16.], 4, 4).unwrap();
    m.points[12] = (40., 40.);
    assert!(run(&mut d, id, &Warp::Mesh(m)).is_err());
    assert_eq!(bytes(&d, id), b);
    assert_eq!(d.undo_count(), 0);
}
#[test]
fn selection_lifts_only_selected_pixels_and_moves_with_projection() {
    let (mut d, id) = scene(4);
    d.set_selection(Some(SelectionMask::rectangle(&d, 4, 4, 8, 8)))
        .unwrap();
    d.clear_history().unwrap();
    let b = bytes(&d, id);
    run(&mut d, id, &translation()).unwrap();
    let after = bytes(&d, id);
    for y in 0..16 {
        for x in 0..16 {
            if !(4..9).contains(&x) || !(4..8).contains(&y) {
                assert_eq!(after[y * 16 + x], b[y * 16 + x]);
            }
        }
    }
    assert_eq!(d.selection().unwrap().amount(4, 5), 0);
    assert_eq!(d.selection().unwrap().amount(8, 5), 255);
    d.undo().unwrap();
    assert_eq!(bytes(&d, id), b);
    assert_eq!(d.selection().unwrap().amount(4, 5), 255);
}
fn dab(mode: LiquifyMode) -> LiquifyDab {
    LiquifyDab {
        center: (8., 8.),
        delta: (3., 1.),
        radius: 7.,
        strength: 0.8,
        mode,
    }
}
#[test]
fn liquify_modes_and_restore_keep_one_undo_per_stroke() {
    for mode in [
        LiquifyMode::Push,
        LiquifyMode::Clockwise,
        LiquifyMode::CounterClockwise,
        LiquifyMode::Pinch,
        LiquifyMode::Expand,
    ] {
        let (mut d, id) = scene(4);
        let b = bytes(&d, id);
        assert!(run(&mut d, id, &Warp::Liquify(vec![dab(mode)])).unwrap());
        assert_ne!(bytes(&d, id), b);
        assert_eq!(d.undo_count(), 1);
        d.undo().unwrap();
        assert_eq!(bytes(&d, id), b);
    }
    let (mut d, id) = scene(4);
    let snapshot = d.capture_snapshot().unwrap();
    let b = bytes(&d, id);
    let mut dabs = vec![dab(LiquifyMode::Push)];
    d.liquify_from_snapshot_cancellable(&[id], &dabs, &snapshot, &mut || false)
        .unwrap();
    let warped = bytes(&d, id);
    let mut restore = dab(LiquifyMode::Restore);
    restore.radius = 1e12;
    restore.strength = 1.;
    dabs.push(restore);
    d.liquify_from_snapshot_cancellable(&[id], &dabs, &snapshot, &mut || false)
        .unwrap();
    assert_eq!(bytes(&d, id), b);
    assert_eq!(d.undo_count(), 2);
    d.undo().unwrap();
    assert_eq!(bytes(&d, id), warped);
}
#[test]
fn liquify_does_not_change_pixels_outside_selection() {
    let (mut d, id) = scene(4);
    d.set_selection(Some(SelectionMask::rectangle(&d, 4, 4, 8, 8)))
        .unwrap();
    d.clear_history().unwrap();
    let b = bytes(&d, id);
    run(&mut d, id, &Warp::Liquify(vec![dab(LiquifyMode::Push)])).unwrap();
    let a = bytes(&d, id);
    for y in 0..16 {
        for x in 0..16 {
            if !(4..8).contains(&x) || !(4..8).contains(&y) {
                assert_eq!(a[y * 16 + x], b[y * 16 + x]);
            }
        }
    }
    assert_eq!(d.selection().unwrap().amount(4, 5), 255);
}

#[test]
fn affine_subset_matches_existing_premultiplied_resampler() {
    let (mut d, id) = scene(4);
    let mut reference = d.capture_snapshot().unwrap();
    let a = [(0., 0.), (16., 0.), (16., 16.), (0., 16.)];
    let t = Affine2D::translation(0.25, -0.4);
    let b = a.map(|p| t.apply(p.0, p.1));
    run(
        &mut d,
        id,
        &Warp::Projective(Homography::from_quads(a, b).unwrap()),
    )
    .unwrap();
    reference
        .transform_layer(id, t, Resampling::Bilinear, true)
        .unwrap();
    assert_eq!(bytes(&d, id), bytes(&reference, id));
}
#[test]
fn source_budget_and_later_layer_lock_leave_all_targets_unchanged() {
    let mut d = Document::with_tile_size(16, 16, 4).unwrap();
    let id = d.add_layer("試験").unwrap();
    d.set_pixel(id, 3, 3, Rgba8::new(40, 80, 120, 255)).unwrap();
    d.clear_history().unwrap();
    let before = bytes(&d, id);
    d.set_source_budget_bytes(d.allocated_bytes()).unwrap();
    assert!(run(
        &mut d,
        id,
        &Warp::Projective(
            Homography::from_quads(
                [(0., 0.), (16., 0.), (16., 16.), (0., 16.)],
                [(0.5, 0.), (16.5, 0.), (16.5, 16.), (0.5, 16.)]
            )
            .unwrap()
        )
    )
    .is_err());
    assert_eq!(bytes(&d, id), before);
    assert_eq!(d.undo_count(), 0);
    d.set_source_budget_bytes(1 << 20).unwrap();
    let second = d.duplicate_layer(id, None).unwrap();
    d.set_layer_locks(second, LayerLocks::PIXELS).unwrap();
    d.clear_history().unwrap();
    assert!(d
        .warp_layers_cancellable(&[id, second], &translation(), &mut || false)
        .is_err());
    assert_eq!(bytes(&d, id), before);
    assert_eq!(bytes(&d, second), before);
    assert_eq!(d.undo_count(), 0);
}
#[test]
fn channels_and_mask_share_one_history_entry() {
    let (mut d, id) = scene(4);
    d.set_channel_pixel(id, Channel::Emission, 2, 2, Rgba8::new(40, 80, 120, 255))
        .unwrap();
    d.add_layer_mask(id).unwrap();
    d.set_mask_pixel(id, 2, 2, 230).unwrap();
    d.clear_history().unwrap();
    run(&mut d, id, &translation()).unwrap();
    assert_eq!(
        d.layer(id).unwrap().pixel(Channel::Emission, 3, 2).unwrap(),
        Rgba8::new(40, 80, 120, 255)
    );
    assert_eq!(d.undo_count(), 1);
    d.undo().unwrap();
    assert_eq!(
        d.layer(id).unwrap().pixel(Channel::Emission, 2, 2).unwrap(),
        Rgba8::new(40, 80, 120, 255)
    );
}

#[test]
fn overflowing_mesh_geometry_is_rejected_instead_of_erasing_pixels() {
    let (mut d, id) = scene(4);
    let before = bytes(&d, id);
    let mut m = WarpMesh::new([0., 0., 16., 16.], 1, 1).unwrap();
    m.points = vec![
        (-1e300, -1e300),
        (1e300, -1e300),
        (-1e300, 1e300),
        (1e300, 1e300),
    ];
    assert!(run(&mut d, id, &Warp::Mesh(m)).is_err());
    assert_eq!(bytes(&d, id), before);
    assert_eq!(d.undo_count(), 0);
}

#[test]
fn review_mesh_preserves_rgba_outside_both_grids_and_undo() {
    for selected in [false, true] {
        let mut d = Document::with_tile_size(16, 16, 4).unwrap();
        let id = d.add_layer("試験").unwrap();
        for y in 0..16 {
            for x in 0..16 {
                d.set_pixel(
                    id,
                    x,
                    y,
                    Rgba8::new(
                        (x * 11 + 1) as u8,
                        (y * 13 + 2) as u8,
                        93,
                        if (4..8).contains(&x) && (4..8).contains(&y) {
                            255
                        } else {
                            0
                        },
                    ),
                )
                .unwrap();
            }
        }
        if selected {
            d.set_selection(Some(SelectionMask::rectangle(&d, 0, 0, 16, 16)))
                .unwrap();
        }
        d.clear_history().unwrap();
        let before = bytes(&d, id);
        let b = d.transform_bounds(id, None).unwrap().unwrap();
        assert_eq!((b.x, b.y, b.width, b.height), (4, 4, 4, 4));
        let mut mesh = WarpMesh::new([4., 4., 4., 4.], 2, 2).unwrap();
        for p in &mut mesh.points {
            p.0 += 4.;
        }
        assert!(run(&mut d, id, &Warp::Mesh(mesh)).unwrap());
        let after = bytes(&d, id);
        for y in 0..16 {
            for x in 0..16 {
                if !(4..12).contains(&x) || !(4..8).contains(&y) {
                    assert_eq!(after[y * 16 + x], before[y * 16 + x], "格子外 ({x},{y})");
                }
            }
        }
        assert_eq!(
            after[6 * 16 + 6],
            Rgba8::TRANSPARENT,
            "元の格子から移した画素は残さない"
        );
        assert_eq!(
            after[6 * 16 + 10],
            before[6 * 16 + 6],
            "移動先は元の透明RGBを上書きする"
        );
        if selected {
            assert_eq!(d.selection().unwrap().amount(1, 1), 255);
        }
        assert_eq!(d.undo_count(), 1);
        d.undo().unwrap();
        assert_eq!(bytes(&d, id), before);
        d.redo().unwrap();
        assert_eq!(bytes(&d, id), after);
    }
}
