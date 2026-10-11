//! 3D ビューの選択範囲の重ね（egui_kittest。描いた絵を読み出して、重ねだけの差を見る）: 縁（白と黒の点線）を見えている面の上にだけ描くこと・選択範囲が無ければ
//! 何も描かないこと・取り消しで消えること・変わったタイルだけを上げること・GPU のメモリの予算（3D の絵の取り分）に入らなければ出さずに知らせること・
//! クイックマスクの赤い重ねと選択ペンの途中の被覆・拡大・縮小しても線が太らないこと・トーンマッピングを通しても白いこと・絵（日英）。
use crate::common::*;
use crate::view3d_brush::{cube_view, press_with, screen_of};
use crate::view3d_select::{
    corners, face_visible, island_texels, message, pen_sample, side_by_side, st, tool,
    wide_plate_view, H,
};
use crate::view3d_select_pen::{brush, on_plate, select_rect, WINDING};
use egui::{pos2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::kittest::Queryable;
use image::RgbaImage;
use yolu_app::lang::Lang;
use yolu_app::selection::{SelAction, SelEdit, SelUiOp};
use yolu_app::state::{Action, Tool};
use yolu_app::view3d::brdf::Curve;
use yolu_app::view3d::display::Op;
use yolu_core::glam::Vec3;
use yolu_core::{SelectionCombine, SelectionMask};

/// 手前の面（−Z）の上の楕円（世界の (−0.35, −0.3)〜(0.45, 0.4)）を、選択範囲にする（画面の上で引く）。
fn ellipse_on_front(h: &mut H, rect: Rect) {
    tool(h, Tool::SelectEllipse);
    let (a, b) = (
        screen_of(h, rect, Vec3::new(-0.35, -0.3, -0.5)),
        screen_of(h, rect, Vec3::new(0.45, 0.4, -0.5)),
    );
    let mid = a + (b - a) * 0.5;
    drag(h, &[a, mid, b]);
    h.run();
}

/// 点線を止めた（絵を同じにする）3D だけの画面。
fn still_view() -> (H, Rect) {
    let (mut h, rect) = cube_view();
    h.state_mut().state.sel.animate = false;
    h.run();
    (h, rect)
}

/// 絵を撮る（ポインタは外へ。ポインタの矢印や、ホバーの見た目が絵に入らないように）。
fn shot(h: &mut H) -> RgbaImage {
    h.event(Event::PointerGone);
    h.run();
    h.render().expect("描画")
}

/// 2 枚の絵の、違う画素の数と、それを囲む箱。
fn differences(a: &RgbaImage, b: &RgbaImage) -> (usize, Option<Rect>) {
    assert_eq!(a.dimensions(), b.dimensions());
    let (mut count, mut bounds) = (0usize, None::<Rect>);
    for (x, y, p) in a.enumerate_pixels() {
        if p != b.get_pixel(x, y) {
            count += 1;
            let at = pos2(x as f32, y as f32);
            bounds = Some(bounds.map_or(Rect::from_min_max(at, at), |r| {
                r.union(Rect::from_min_max(at, at))
            }));
        }
    }
    (count, bounds)
}

fn clear_selection(h: &mut H) {
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Edit(SelEdit::Clear)));
    h.run();
}

#[test]
fn the_edge_is_drawn_on_the_model_only_where_the_selection_is_and_undo_takes_it_away() {
    let (mut h, rect) = still_view();
    let base = shot(&mut h);
    assert_eq!(st(&h).doc.undo_count(), 0);
    let stats = h.state().view3d_stats().unwrap();
    assert_eq!(stats.overlay_bytes, 0, "選択範囲が無いときは何も持たない");
    // 選択範囲を作る: 縁が出る（楕円の画面の箱のまわりだけ）
    ellipse_on_front(&mut h, rect);
    let with = shot(&mut h);
    let (count, bounds) = differences(&base, &with);
    assert!(count > 300, "縁の画素: {count}");
    let (a, b) = (
        screen_of(&h, rect, Vec3::new(-0.35, -0.3, -0.5)),
        screen_of(&h, rect, Vec3::new(0.45, 0.4, -0.5)),
    );
    let oval = Rect::from_two_pos(a, b).expand(4.0);
    // 選択のツールの輪郭・通知などの画面の部品は、3D の外（左のパネル・下の知らせ）にも出るので、3D の表示域の中だけを見る
    let (inside, _) = {
        let crop = |img: &RgbaImage| {
            let (w, h2) = img.dimensions();
            let mut out = RgbaImage::new(w, h2);
            for y in rect.top() as u32..(rect.bottom() as u32).min(h2) {
                for x in rect.left() as u32..(rect.right() as u32).min(w) {
                    out.put_pixel(x, y, *img.get_pixel(x, y));
                }
            }
            out
        };
        differences(&crop(&base), &crop(&with))
    };
    assert!(inside > 300, "{inside}");
    let _ = bounds;
    let crop_bounds = {
        let mut r = Rect::NOTHING;
        for (x, y, p) in base.enumerate_pixels() {
            let at = pos2(x as f32, y as f32);
            if rect.contains(at) && p != with.get_pixel(x, y) {
                r.extend_with(at);
            }
        }
        r
    };
    assert!(
        oval.contains_rect(crop_bounds),
        "縁は選んだ範囲のまわりだけ: {crop_bounds:?} {oval:?}"
    );
    let stats = h.state().view3d_stats().unwrap();
    assert!(
        stats.overlay_bytes > 0 && !stats.overlay_skipped,
        "{stats:?}"
    );
    // 取り消しで消える（絵が前と同じ）。やり直しで戻る
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    let undone = shot(&mut h);
    let in_view = |a: &RgbaImage, b: &RgbaImage| {
        a.enumerate_pixels()
            .filter(|(x, y, p)| {
                rect.contains(pos2(*x as f32, *y as f32)) && *p != b.get_pixel(*x, *y)
            })
            .count()
    };
    assert_eq!(in_view(&base, &undone), 0, "取り消しで縁が消える");
    assert_eq!(h.state().view3d_stats().unwrap().overlay_bytes, 0);
    key(&h, Key::Y, Modifiers::COMMAND);
    h.run();
    let redone = shot(&mut h);
    assert_eq!(in_view(&with, &redone), 0, "やり直しで同じ縁が戻る");
    // 解除でも消える
    clear_selection(&mut h);
    assert_eq!(in_view(&base, &shot(&mut h)), 0);
}

#[test]
fn a_selection_on_faces_the_camera_does_not_see_draws_nothing() {
    let (mut h, _) = still_view();
    let base = shot(&mut h);
    // 見えない面のアイランドだけを選ぶ
    let hidden: Vec<usize> = (0..6).filter(|&f| !face_visible(&h, f)).collect();
    assert!(!hidden.is_empty());
    let mut mask = SelectionMask::none(&st(&h).doc);
    for f in &hidden {
        let texels = island_texels(*f, 6.0);
        let (x0, y0) = texels
            .iter()
            .fold((u32::MAX, u32::MAX), |(a, b), t| (a.min(t.0), b.min(t.1)));
        let (x1, y1) = texels
            .iter()
            .fold((0, 0), |(a, b), t| (a.max(t.0), b.max(t.1)));
        let part =
            SelectionMask::rectangle(&st(&h).doc, x0 as i64, y0 as i64, x1 as i64, y1 as i64);
        mask = mask.combine(&part, SelectionCombine::Add).unwrap();
    }
    h.state_mut().state.doc.set_selection(Some(mask)).unwrap();
    let after = shot(&mut h);
    // 縁は引かれる（GPU に上げてある）が、見えない面には描かれない（面の上の絵は同じ）
    assert!(h.state().view3d_stats().unwrap().overlay_bytes > 0);
    let crop_diff = after
        .enumerate_pixels()
        .filter(|(x, y, p)| {
            let at = pos2(*x as f32, *y as f32);
            h.state().view3d_rect().unwrap().contains(at) && *p != base.get_pixel(*x, *y)
        })
        .count();
    assert_eq!(crop_diff, 0, "見えない面の選択範囲は画面に出ない");
    let _ = corners(0);
}

#[test]
fn an_edge_behind_another_face_is_hidden_by_it_and_shows_where_the_face_is_seen() {
    // 奥の板（UV の左半分）の真ん中は手前の板に隠れる
    let (mut h, rect) = crate::view3d_select::two_plates();
    h.state_mut().state.sel.animate = false;
    let base = shot(&mut h);
    let edit = |h: &mut H, (x0, y0, x1, y1): (i64, i64, i64, i64)| {
        h.state_mut()
            .state
            .apply(Action::Sel(SelAction::Edit(SelEdit::Rect {
                x0,
                y0,
                x1,
                y1,
                mode: SelectionCombine::Replace,
            })));
        h.run();
    };
    let in_view = |a: &RgbaImage, b: &RgbaImage| {
        a.enumerate_pixels()
            .filter(|(x, y, p)| {
                rect.contains(pos2(*x as f32, *y as f32)) && *p != b.get_pixel(*x, *y)
            })
            .count()
    };
    // 隠れた所の中だけの選択範囲: 縁は全部手前の板の後ろ。画面に何も出ない
    edit(&mut h, (14, 26, 18, 38));
    let hidden = shot(&mut h);
    assert!(h.state().view3d_stats().unwrap().overlay_bytes > 0);
    assert_eq!(in_view(&base, &hidden), 0, "手前の板が隠す所の縁は出ない");
    // 見えている所にかかる選択範囲: 縁が出る
    edit(&mut h, (3, 20, 8, 44));
    let seen = shot(&mut h);
    assert!(in_view(&base, &seen) > 50, "見える所の縁は出る");
}

#[test]
fn only_the_tiles_that_changed_are_uploaded_and_the_texture_is_released_with_the_selection() {
    // 文書 512²（タイル 256²）: 4 枚のタイル
    let mut h = app(1100.0, 760.0, 512);
    h.state_mut().state.view3d.load_demo();
    h.state_mut().state.view3d.camera.yaw = -40.0;
    h.state_mut().state.view3d.camera.pitch = 15.0;
    h.state_mut().state.sel.animate = false;
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    let tiles = |h: &H| h.state().view3d_stats().unwrap().overlay_tiles;
    assert_eq!(tiles(&h), 0);
    let rect_edit = |h: &mut H, x0, y0, x1, y1, mode| {
        h.state_mut()
            .state
            .apply(Action::Sel(SelAction::Edit(SelEdit::Rect {
                x0,
                y0,
                x1,
                y1,
                mode,
            })));
        h.run();
    };
    rect_edit(&mut h, 10, 10, 100, 100, SelectionCombine::Replace);
    assert_eq!(tiles(&h), 1, "量のあるタイルだけ（1 枚）");
    let first_bytes = h.state().view3d_stats().unwrap().overlay_bytes;
    assert_eq!(
        first_bytes,
        yolu_app::view3d::selection_overlay::texture_bytes([512, 512])
    );
    // 同じ選択範囲のまま回しても、上げ直さない
    for _ in 0..5 {
        h.step();
    }
    assert_eq!(tiles(&h), 1, "変わらなければ上げない");
    // 反対側のタイルへ追加: 新しいタイルだけを上げる
    rect_edit(&mut h, 260, 260, 380, 380, SelectionCombine::Add);
    assert_eq!(
        tiles(&h),
        2,
        "変わったタイルだけ（最初のタイルは上げ直さない）"
    );
    // 最初のタイルの中を削る: そのタイルだけ
    rect_edit(&mut h, 20, 20, 60, 60, SelectionCombine::Subtract);
    assert_eq!(tiles(&h), 3);
    // 解除すると持たない
    clear_selection(&mut h);
    assert_eq!(h.state().view3d_stats().unwrap().overlay_bytes, 0);
}

/// 2 つのマテリアルの板（左の板がマテリアル 0 で、今のセットの面。右の板はマテリアル 1 で、セットに付いていない）。
fn two_materials() -> (H, Rect) {
    use yolu_app::view3d::model::ViewModel;
    use yolu_core::geometry::{ModelMesh, OrbitCamera, Submesh};
    use yolu_core::glam::Vec2;
    let mut h = app(1100.0, 760.0, 64);
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.state_mut().state.sel.animate = false;
    h.run();
    let plate = |cx: f32, material: i32| {
        let p = |x: f32, y: f32| Vec3::new(cx + x, y, 0.0);
        ModelMesh {
            name: format!("板{material}"),
            positions: vec![p(-0.9, -0.9), p(0.9, -0.9), p(-0.9, 0.9), p(0.9, 0.9)],
            normals: Vec::new(),
            uvs: vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(1.0, 0.0),
                Vec2::new(0.0, 1.0),
                Vec2::new(1.0, 1.0),
            ],
            submeshes: vec![Submesh {
                material,
                indices: vec![0, 2, 1, 2, 3, 1],
            }],
        }
    };
    {
        let state = &mut h.state_mut().state;
        let revision = state.view3d.next_revision();
        let model = ViewModel::new(
            "二枚",
            vec![plate(-1.1, 0), plate(1.1, 1)],
            vec![Some("a".to_string()), Some("b".to_string())],
            revision,
        )
        .unwrap();
        state.view3d.set_model(model);
        state.view3d.material = 0;
        state.view3d.camera = OrbitCamera {
            target: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            distance: 7.0,
            model_radius: 1.0,
            ..OrbitCamera::default()
        };
        state.sync_view3d();
    }
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブを描いた");
    (h, rect)
}

#[test]
fn the_overlay_covers_the_faces_of_the_current_material_only() {
    let (mut h, rect) = two_materials();
    let base = shot(&mut h);
    let left = Rect::from_two_pos(
        screen_of(&h, rect, Vec3::new(-2.0, -0.9, 0.0)),
        screen_of(&h, rect, Vec3::new(-0.2, 0.9, 0.0)),
    );
    let right = Rect::from_two_pos(
        screen_of(&h, rect, Vec3::new(0.2, -0.9, 0.0)),
        screen_of(&h, rect, Vec3::new(2.0, 0.9, 0.0)),
    );
    let changed_in = |a: &RgbaImage, b: &RgbaImage, area: Rect| {
        a.enumerate_pixels()
            .filter(|(x, y, p)| {
                area.contains(pos2(*x as f32, *y as f32)) && *p != b.get_pixel(*x, *y)
            })
            .count()
    };
    let select = |h: &mut H| {
        h.state_mut()
            .state
            .apply(Action::Sel(SelAction::Edit(SelEdit::Rect {
                x0: 10,
                y0: 10,
                x1: 54,
                y1: 54,
                mode: SelectionCombine::Replace,
            })));
        h.run();
    };
    // 今のセット（描くマテリアルは 0）の選択範囲: 左の板だけに縁が出る
    select(&mut h);
    let with = shot(&mut h);
    assert!(changed_in(&base, &with, left) > 100, "左の板に縁");
    assert_eq!(changed_in(&base, &with, right), 0, "右の板には出ない");
}

#[test]
fn many_changed_tiles_uploaded_in_one_go_draw_the_same_as_tile_by_tile() {
    // 512²（タイル 128²）で、全部のタイルにかかる楕円。1 つ目は 1 回で作る（変わったタイルが多いので、まとめて上げる）。2 つ目は、同じ形から小さな長方形を
    // 引いた選択範囲を作り（まとめて上げる）、そのあと長方形を追加し直す（1 タイルだけ。タイルごとに上げる）。どちらも最後は同じ選択範囲で、絵も同じ
    let view = |two_steps: bool| {
        let mut h = app(1100.0, 760.0, 512);
        h.state_mut().state.view3d.load_demo();
        h.state_mut().state.view3d.camera.yaw = -40.0;
        h.state_mut().state.view3d.camera.pitch = 15.0;
        h.state_mut().state.sel.animate = false;
        click_tab(&mut h, yolu_app::Tab::View3d);
        h.run();
        let edit = |h: &mut H, e: SelEdit| {
            h.state_mut().state.apply(Action::Sel(SelAction::Edit(e)));
            h.run();
        };
        let ellipse = SelEdit::Ellipse {
            cx: 256.0,
            cy: 256.0,
            rx: 250.0,
            ry: 240.0,
            mode: SelectionCombine::Replace,
        };
        let rect = |mode| SelEdit::Rect {
            x0: 140,
            y0: 140,
            x1: 200,
            y1: 200,
            mode,
        };
        edit(&mut h, ellipse);
        if two_steps {
            edit(&mut h, rect(SelectionCombine::Subtract));
            let tiles = h.state().view3d_stats().unwrap().overlay_tiles;
            assert!(tiles >= 16, "1 回目は多くのタイルを上げた: {tiles}");
            edit(&mut h, rect(SelectionCombine::Add));
            let more = h.state().view3d_stats().unwrap().overlay_tiles - tiles;
            assert_eq!(more, 1, "追加し直しは 1 タイルだけ");
        }
        let selection = st(&h).doc.selection().cloned();
        (shot(&mut h), selection)
    };
    let (direct, direct_mask) = view(false);
    let (stepwise, stepwise_mask) = view(true);
    assert!(direct_mask.is_some() && stepwise_mask.is_some());
    // 選択範囲は同じ（長方形は楕円の内側の 1 タイルの中なので、追加し直すと楕円と同じ量に戻る）
    assert!(direct_mask == stepwise_mask, "選択範囲は同じ");
    // 3D の表示域の中だけを見る（知らせの文字は違う）
    let n = direct
        .enumerate_pixels()
        .filter(|(x, y, p)| {
            let at = pos2(*x as f32, *y as f32);
            at.x > 280.0
                && at.x < 890.0
                && at.y > 100.0
                && at.y < 740.0
                && *p != stepwise.get_pixel(*x, *y)
        })
        .count();
    assert_eq!(n, 0, "まとめて上げても、タイルごとに上げても、同じ絵");
}

#[test]
fn when_the_selection_does_not_fit_the_picture_budget_it_is_not_shown_and_the_view_says_so() {
    let (mut h, rect) = still_view();
    // 描き先の上限は 3D の絵の予算と同じ量なので、予算を下げるとサンプル数が下がって縁の画素が変わる。描き先の上限は離しておく
    h.state_mut().view3d_set_target_budget(Some(u64::MAX));
    let base = shot(&mut h);
    ellipse_on_front(&mut h, rect);
    let stats = h.state().view3d_stats().unwrap();
    let (paint, overlay) = (stats.paint_bytes, stats.overlay_bytes);
    assert!(paint > 0 && overlay > 0 && !stats.overlay_skipped);
    let shown = shot(&mut h);
    // 予算が、絵の分に重ねの分の半分しか加えない: 重ねは出さず、絵はそのまま
    h.state_mut().view3d_set_paint_budget(paint + overlay / 2);
    h.run();
    let stats = h.state().view3d_stats().unwrap();
    assert!(stats.overlay_skipped, "{stats:?}");
    assert_eq!(stats.overlay_bytes, 0, "持たない");
    assert_eq!(stats.paint_level, 0, "絵は縮めない");
    let hidden = shot(&mut h);
    // 隅のアイコン（予算の知らせ）は絵に入るので、モデルの画面の箱の中だけを見る
    let model = crate::view3d_select::cube_box(&h, rect).expand(6.0);
    let in_view = |a: &RgbaImage, b: &RgbaImage| {
        a.enumerate_pixels()
            .filter(|(x, y, p)| {
                model.contains(pos2(*x as f32, *y as f32)) && *p != b.get_pixel(*x, *y)
            })
            .count()
    };
    assert_eq!(
        in_view(&base, &hidden),
        0,
        "予算に入らなければ今までの動き（縁は出ない）"
    );
    // 予算を戻せば出る
    h.state_mut()
        .view3d_set_paint_budget(paint + overlay + 4096);
    h.run();
    let stats = h.state().view3d_stats().unwrap();
    assert!(
        !stats.overlay_skipped && stats.overlay_bytes == overlay,
        "{stats:?}"
    );
    assert_eq!(in_view(&shown, &shot(&mut h)), 0);
}

#[test]
fn the_overlay_counts_in_the_bytes_the_view_held_at_its_peak() {
    // 同期の途中で GPU に持っていた絵のバイト（予算を超えていないかの記録）に、重ねの分も入る
    let (mut h, rect) = still_view();
    ellipse_on_front(&mut h, rect);
    h.state_mut().state.view3d.camera.yaw += 3.0;
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert!(s.overlay_bytes > 0 && !s.overlay_skipped, "{s:?}");
    assert!(s.peak_bytes >= s.paint_bytes + s.overlay_bytes, "{s:?}");
}

#[test]
fn the_skipped_selection_is_told_by_a_corner_icon_in_both_languages() {
    for lang in Lang::ALL {
        let (mut h, rect) = still_view();
        h.state_mut().state.lang = lang;
        ellipse_on_front(&mut h, rect);
        let stats = h.state().view3d_stats().unwrap();
        h.state_mut()
            .view3d_set_paint_budget(stats.paint_bytes + stats.overlay_bytes / 2);
        h.run();
        assert!(h.state().view3d_stats().unwrap().overlay_skipped);
        let tip = lang.pick(
            "選択範囲を 3D に出していません",
            "Selection not shown in 3D",
        );
        assert!(
            h.query_by_label_contains(tip).is_some(),
            "{lang:?}: 隅のアイコンが予算に入らないことを知らせる"
        );
        // 予算を戻すと、アイコンは消える
        h.state_mut()
            .view3d_set_paint_budget(yolu_app::view3d::paint::PAINT_BUDGET_BYTES);
        h.run();
        assert!(h.query_by_label_contains(tip).is_none(), "{lang:?}");
    }
}

#[test]
fn quick_mask_shows_a_red_overlay_in_3d_and_the_ants_are_gone() {
    let (mut h, rect) = still_view();
    ellipse_on_front(&mut h, rect);
    let ants = shot(&mut h);
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Ui(SelUiOp::QuickMask(Some(true)))));
    h.run();
    let red = shot(&mut h);
    // 赤い重ね: 選択範囲の真ん中の画素が、重ねる前より赤くなる（青・緑が下がり、赤が保たれる）
    let center = screen_of(&h, rect, Vec3::new(0.05, 0.05, -0.5));
    let (before, after) = (
        ants.get_pixel(center.x as u32, center.y as u32),
        red.get_pixel(center.x as u32, center.y as u32),
    );
    assert!(
        after[0] as i32 - after[1] as i32 > before[0] as i32 - before[1] as i32 + 30,
        "真ん中が赤みを帯びる {before:?} {after:?}"
    );
    // 選択範囲の外は変わらない（縁の点線が無くなった分のほかは）
    let outside = screen_of(&h, rect, Vec3::new(0.4, -0.45, -0.5));
    assert_eq!(
        ants.get_pixel(outside.x as u32, outside.y as u32),
        red.get_pixel(outside.x as u32, outside.y as u32)
    );
    // クイックマスクを切ると縁へ戻る
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Ui(SelUiOp::QuickMask(Some(false)))));
    h.run();
    let back = shot(&mut h);
    let n = ants
        .enumerate_pixels()
        .filter(|(x, y, p)| {
            rect.contains(pos2(*x as f32, *y as f32)) && *p != back.get_pixel(*x, *y)
        })
        .count();
    assert_eq!(n, 0, "縁へ戻る");
}

/// 面（−Z）の上の線を、選択ペンで引き始めて、離さずに途中の絵を撮る。
fn pen_stroke_halfway(h: &mut H, rect: Rect, modifiers: Modifiers) -> RgbaImage {
    h.state_mut().state.brush.radius = 12.0;
    h.state_mut().state.brush.hardness = 1.0;
    h.state_mut().state.brush.opacity = 1.0;
    let (a, b) = (
        screen_of(h, rect, Vec3::new(-0.3, 0.0, -0.5)),
        screen_of(h, rect, Vec3::new(0.3, 0.0, -0.5)),
    );
    if modifiers != Modifiers::NONE {
        h.event(Event::ModifiersChanged(modifiers));
    }
    press_with(h, a, modifiers);
    h.step();
    for i in 1..=4 {
        crate::common::move_to(h, a + (b - a) * (i as f32 / 4.0));
        h.step();
    }
    shot(h)
}

#[test]
fn a_selection_pen_stroke_shows_its_cover_while_painting_blue_to_add_and_orange_to_erase() {
    let (mut h, rect) = still_view();
    let base = shot(&mut h);
    tool(&mut h, Tool::SelectPen);
    let adding = pen_stroke_halfway(&mut h, rect, Modifiers::NONE);
    let tinted = |img: &RgbaImage, f: &dyn Fn(&image::Rgba<u8>) -> bool| {
        img.enumerate_pixels()
            .filter(|(x, y, p)| {
                rect.contains(pos2(*x as f32, *y as f32)) && base.get_pixel(*x, *y) != *p && f(p)
            })
            .count()
    };
    let blue = tinted(&adding, &|p| p[2] as i32 - p[0] as i32 > 40);
    assert!(blue > 300, "追加している間は青い被覆: {blue}");
    // 離すと、被覆は消えて選択範囲の縁になる
    release(&h, rect.center(), PointerButton::Primary);
    h.run();
    let after = shot(&mut h);
    let blue_after = tinted(&after, &|p| p[2] as i32 - p[0] as i32 > 40);
    assert!(
        blue_after < blue / 10,
        "離したら青は残らない: {blue_after} {blue}"
    );
    assert!(st(&h).doc.selection().is_some());
    // 選択消し（Ctrl）の途中は橙
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    let erasing = pen_stroke_halfway(&mut h, rect, Modifiers::COMMAND);
    let orange = tinted(&erasing, &|p| p[0] as i32 - p[2] as i32 > 60);
    assert!(orange > 100, "消している間は橙の被覆: {orange}");
    release(&h, rect.center(), PointerButton::Primary);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run();
    let _ = (message(&h), pen_sample(Pos2::ZERO, true, 0));
}

/// 縁を横切る 1 本の画素の列で、重ねで変わった画素の数（左の縁のあたり）。
fn edge_width(
    base: &RgbaImage,
    with: &RgbaImage,
    rect: Rect,
    row: u32,
    from_x: u32,
    to_x: u32,
) -> u32 {
    let mut width = 0;
    for x in from_x..to_x {
        if rect.contains(pos2(x as f32, row as f32))
            && base.get_pixel(x, row) != with.get_pixel(x, row)
        {
            width += 1;
        }
    }
    width
}

#[test]
fn the_edge_keeps_its_thickness_when_the_model_is_zoomed_in_or_out() {
    let mut widths = Vec::new();
    for distance_scale in [0.85f32, 1.0, 1.7] {
        let (mut h, rect) = still_view();
        h.state_mut().state.view3d.camera.distance *= distance_scale;
        h.run();
        let base = shot(&mut h);
        ellipse_on_front(&mut h, rect);
        let with = shot(&mut h);
        assert!(
            st(&h).doc.selection().is_some(),
            "{distance_scale}: {}",
            message(&h)
        );
        // 楕円の真ん中の高さの行で、左の縁のまわり（楕円の左端の点から ±12 画素）の変わった画素
        let left = screen_of(&h, rect, Vec3::new(-0.35, 0.05, -0.5));
        let row = left.y.round() as u32;
        let w = edge_width(
            &base,
            &with,
            rect,
            row,
            (left.x - 12.0).max(0.0) as u32,
            (left.x + 12.0) as u32,
        );
        widths.push(w);
    }
    // 1〜2 点の線（点線の黒と白）: 近くでも遠くでも 1〜5 画素
    assert!(
        widths.iter().all(|&w| (1..=5).contains(&w)),
        "縁の太さ（画素）: {widths:?}"
    );
}

#[test]
fn the_edge_stays_white_through_tone_mapping() {
    for curve in [Curve::Aces, Curve::Neutral] {
        let (mut h, rect) = still_view();
        h.state_mut().state.apply(Action::View3d(Op::Tone(curve)));
        h.run();
        let base = shot(&mut h);
        ellipse_on_front(&mut h, rect);
        let with = shot(&mut h);
        // 縁の白い画素（どの色も 215 以上）がある
        let white = with
            .enumerate_pixels()
            .filter(|(x, y, p)| {
                rect.contains(pos2(*x as f32, *y as f32))
                    && base.get_pixel(*x, *y) != *p
                    && p[0] >= 215
                    && p[1] >= 215
                    && p[2] >= 215
            })
            .count();
        assert!(white > 60, "{curve:?}: 白い縁の画素 {white}");
    }
}

#[test]
fn snapshot_selection_in_3d_only_in_both_languages() {
    let mut snapshots = egui_kittest::SnapshotResults::new();
    for lang in Lang::ALL {
        let (mut h, rect) = still_view();
        h.state_mut().state.lang = lang;
        h.run();
        ellipse_on_front(&mut h, rect);
        // 2 つ目の形を追加（楕円の外の、側面にまたがる長方形）
        tool(&mut h, Tool::SelectRect);
        let (a, b) = (
            screen_of(&h, rect, Vec3::new(0.5, -0.4, -0.3)),
            screen_of(&h, rect, Vec3::new(0.5, 0.1, 0.35)),
        );
        let mid = a + (b - a) * 0.5;
        h.event(Event::ModifiersChanged(Modifiers::SHIFT));
        press_with(&h, a, Modifiers::SHIFT);
        h.step();
        crate::common::move_to(&h, mid);
        h.step();
        crate::common::move_to(&h, b);
        h.step();
        crate::view3d_brush::release_with(&h, b, Modifiers::SHIFT);
        h.event(Event::ModifiersChanged(Modifiers::NONE));
        h.step();
        h.event(Event::PointerGone);
        h.run();
        h.snapshot(lang.pick("view3d_selection_edge_ja", "view3d_selection_edge_en"));
        snapshots.extend_harness(&mut h);
        // クイックマスク（赤い重ね）
        h.state_mut()
            .state
            .apply(Action::Sel(SelAction::Ui(SelUiOp::QuickMask(Some(true)))));
        h.run();
        h.snapshot(lang.pick("view3d_selection_quick_ja", "view3d_selection_quick_en"));
        snapshots.extend_harness(&mut h);
    }
}

/// 計測: 選択範囲の重ねを出す前後の 1 フレームの時間（球。カメラを回して毎フレーム描き直すとき）と、選択範囲を変えたフレーム（テクスチャを上げて、ミップを積む）の時間。
/// `WGPU_BACKEND=gl LD_LIBRARY_PATH=/opt/mesa-d3d12/lib:/usr/lib/wsl/lib GALLIUM_DRIVER=d3d12 cargo test -p yolu-app --test gui_view3d view3d_select_overlay::measure -- --ignored --nocapture`
#[test]
#[ignore = "計測"]
fn measure_frames_with_and_without_the_selection_overlay() {
    use std::time::Instant;
    use yolu_app::view3d::display::{EnvKind, Shading};
    use yolu_app::view3d::model::ViewModel;
    use yolu_core::geometry::{cube_sphere, OrbitCamera};
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    for (size, grid) in [(2048u32, 77u32), (4096, 160)] {
        let mut h = app(1100.0, 760.0, size);
        click_tab(&mut h, yolu_app::Tab::View3d);
        h.state_mut().state.sel.animate = false;
        h.run();
        println!(
            "GPU: {}  文書 {size}²  球の格子 {grid}",
            h.state().view3d_adapter().unwrap_or_default()
        );
        h.state_mut()
            .state
            .apply(Action::View3d(Op::Shading(Shading::Material)));
        h.state_mut()
            .state
            .apply(Action::View3d(Op::Env(EnvKind::Studio)));
        let mesh = cube_sphere(grid, 0.5);
        let triangles: usize = mesh.submeshes.iter().map(|s| s.indices.len() / 3).sum();
        {
            let state = &mut h.state_mut().state;
            let revision = state.view3d.next_revision();
            state.view3d.material = 0;
            let model = ViewModel::new("球", vec![mesh], vec![None], revision).unwrap();
            model.tangents();
            state.view3d.set_model(model);
            state.view3d.camera = OrbitCamera {
                target: Vec3::ZERO,
                yaw: 20.0,
                pitch: 10.0,
                distance: 1.6,
                model_radius: 1.0,
                ..Default::default()
            };
        }
        h.run();
        h.state().view3d_wait_gpu();
        let frames = |h: &mut H| -> (f64, f64) {
            let (mut wall, mut cpu) = (Vec::new(), Vec::new());
            for frame in 0..60 {
                h.state_mut().state.view3d.camera.yaw += 3.0 + frame as f32 * 0.01;
                let started = Instant::now();
                h.step();
                h.state().view3d_wait_gpu();
                wall.push(started.elapsed().as_micros() as f64 / 1000.0);
                cpu.push(h.state().view3d_stats().unwrap().last_prepare_us as f64 / 1000.0);
            }
            (mean(&wall[10..]), mean(&cpu[10..]))
        };
        // 選択範囲: 文書の半分ほどの楕円（量のあるタイルが多い）。選択範囲を作る CPU の時間（`apply`）と、重ねを上げて描くフレームの時間を分けて測る
        let started = Instant::now();
        h.state_mut()
            .state
            .apply(Action::Sel(SelAction::Edit(SelEdit::Ellipse {
                cx: size as f64 * 0.5,
                cy: size as f64 * 0.5,
                rx: size as f64 * 0.45,
                ry: size as f64 * 0.4,
                mode: SelectionCombine::Replace,
            })));
        let apply_ms = started.elapsed().as_micros() as f64 / 1000.0;
        let started = Instant::now();
        h.step();
        h.state().view3d_wait_gpu();
        let change_ms = started.elapsed().as_micros() as f64 / 1000.0;
        println!(
            "  選択を作る CPU {apply_ms:.2} ms、上げて描くフレーム {change_ms:.2} ms（prepare の CPU {:.2} ms）",
            h.state().view3d_stats().unwrap().last_prepare_us as f64 / 1000.0
        );
        let s = h.state().view3d_stats().unwrap();
        assert!(s.overlay_bytes > 0 && !s.overlay_skipped);
        let (paint, overlay) = (s.paint_bytes, s.overlay_bytes);
        // 描き先の上限は離す（予算を替えてもサンプル数が変わらないように）。選択範囲は持ったまま、重ねを出す・出さないを予算で切り替えて、交互に測る
        h.state_mut().view3d_set_target_budget(Some(u64::MAX));
        let (mut off, mut on) = (Vec::new(), Vec::new());
        for _round in 0..6 {
            h.state_mut().view3d_set_paint_budget(paint + overlay / 2);
            h.run();
            assert!(h.state().view3d_stats().unwrap().overlay_skipped);
            off.push(frames(&mut h));
            h.state_mut()
                .view3d_set_paint_budget(paint + overlay + (64 << 20));
            h.run();
            assert!(h.state().view3d_stats().unwrap().overlay_bytes > 0);
            on.push(frames(&mut h));
        }
        let col = |v: &[(f64, f64)], i: usize| -> Vec<f64> {
            v.iter().map(|p| if i == 0 { p.0 } else { p.1 }).collect()
        };
        let spread = |v: &[f64]| {
            (
                v.iter().cloned().fold(f64::MAX, f64::min),
                v.iter().cloned().fold(f64::MIN, f64::max),
            )
        };
        let (off_wall, on_wall) = (col(&off, 0), col(&on, 0));
        println!(
            "三角形 {triangles}: 重ねなし 平均 {:.2} ms（6 回の範囲 {:.2}〜{:.2}） / 縁あり 平均 {:.2} ms（{:.2}〜{:.2}）。選択を作ったフレーム {change_ms:.2} ms（上げたタイル {}・重ね {} MiB）",
            mean(&off_wall),
            spread(&off_wall).0,
            spread(&off_wall).1,
            mean(&on_wall),
            spread(&on_wall).0,
            spread(&on_wall).1,
            s.overlay_tiles,
            s.overlay_bytes >> 20,
        );
        // 解除してから同じ選択範囲を作り直すフレーム（パイプラインは作ってあるので、上げ・ミップ・描きだけ）
        clear_selection(&mut h);
        h.state().view3d_wait_gpu();
        h.state_mut()
            .state
            .apply(Action::Sel(SelAction::Edit(SelEdit::Ellipse {
                cx: size as f64 * 0.5,
                cy: size as f64 * 0.5,
                rx: size as f64 * 0.45,
                ry: size as f64 * 0.4,
                mode: SelectionCombine::Replace,
            })));
        let started = Instant::now();
        h.step();
        h.state().view3d_wait_gpu();
        println!(
            "  作り直すフレーム（2 回目）{:.2} ms（prepare の CPU {:.2} ms）",
            started.elapsed().as_micros() as f64 / 1000.0,
            h.state().view3d_stats().unwrap().last_prepare_us as f64 / 1000.0
        );
        // 1 タイルだけ変えるフレーム（追加）
        let started = Instant::now();
        h.state_mut()
            .state
            .apply(Action::Sel(SelAction::Edit(SelEdit::Rect {
                x0: 5,
                y0: 5,
                x1: 60,
                y1: 60,
                mode: SelectionCombine::Add,
            })));
        h.step();
        h.state().view3d_wait_gpu();
        println!(
            "  選択を 1 タイルだけ追加したフレーム {:.2} ms",
            started.elapsed().as_micros() as f64 / 1000.0
        );
        // 何もしていないフレーム: 選択範囲があってカメラを動かさないとき、3D を描き直さないフレームと、描き直すフレーム（点線を流すと毎回これになる）
        h.run();
        let (mut still, mut redrawn) = (Vec::new(), Vec::new());
        let renders = h.state().view3d_stats().unwrap().renders;
        for _ in 0..60 {
            let started = Instant::now();
            h.step();
            h.state().view3d_wait_gpu();
            still.push(started.elapsed().as_micros() as f64 / 1000.0);
        }
        assert_eq!(h.state().view3d_stats().unwrap().renders, renders);
        for _ in 0..60 {
            h.state_mut().state.view3d.camera.yaw += 0.001;
            let started = Instant::now();
            h.step();
            h.state().view3d_wait_gpu();
            redrawn.push(started.elapsed().as_micros() as f64 / 1000.0);
        }
        println!(
            "  何もしていないフレーム: 描き直さない 平均 {:.2} ms（{:.2}〜{:.2}） / 描き直す 平均 {:.2} ms（{:.2}〜{:.2}）",
            mean(&still[10..]),
            spread(&still[10..]).0,
            spread(&still[10..]).1,
            mean(&redrawn[10..]),
            spread(&redrawn[10..]).0,
            spread(&redrawn[10..]).1,
        );
        let (q_wall, _) = {
            h.state_mut()
                .state
                .apply(Action::Sel(SelAction::Ui(SelUiOp::QuickMask(Some(true)))));
            frames(&mut h)
        };
        println!("  クイックマスク（赤い重ね）{q_wall:.2} ms");
    }
}

/// 2 枚の絵の、`area`（画面の点）の中の違う画素の数。
fn diff_in(a: &image::RgbaImage, b: &image::RgbaImage, area: Rect, ppp: f32) -> usize {
    let (x0, y0) = ((area.min.x * ppp) as u32, (area.min.y * ppp) as u32);
    let (x1, y1) = (
        ((area.max.x * ppp) as u32).min(a.width()),
        ((area.max.y * ppp) as u32).min(a.height()),
    );
    let mut n = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            if a.get_pixel(x, y) != b.get_pixel(x, y) {
                n += 1;
            }
        }
    }
    n
}

/// キャンバスと 3D ビューを並べ、3D で塗っている途中の、2D のキャンバスの重ね（選択ペンの被覆・クイックマスクの赤）が、作り直した物と同じか
/// （違う画素 0）。`sync` を 2D のキャンバスと 3D ビューの両方が毎フレーム呼んでも、2D の重ねが変わったタイルを受け取れること。
#[test]
fn the_2d_overlay_beside_the_3d_view_follows_a_3d_stroke_in_progress() {
    for quick in [false, true] {
        for canvas_first in [true, false] {
            let label = format!("quick={quick} canvas_first={canvas_first}");
            let (mut h, rect) = side_by_side(canvas_first);
            if quick {
                select_rect(&mut h, (2, 2, 30, 60));
                h.state_mut()
                    .state
                    .apply(Action::Sel(SelAction::Ui(SelUiOp::QuickMask(Some(true)))));
                h.run();
                tool(&mut h, Tool::Brush);
            } else {
                tool(&mut h, Tool::SelectPen);
            }
            brush(&mut h, 3.0, false);
            let pts = on_plate(&h, rect, &WINDING);
            press(&h, pts[0], PointerButton::Primary);
            h.step();
            for p in &pts[1..] {
                move_to(&h, *p);
                h.step();
            }
            h.step();
            assert!(st(&h).sel.pen.is_some(), "{label}: 描いている");
            let drawn = h.render().expect("描画");
            {
                let s = &mut h.state_mut().state;
                s.sel.pen_overlay.clear();
                s.sel.quick_overlay.clear();
            }
            h.step();
            let rebuilt = h.render().expect("描画");
            let ppp = h.ctx.pixels_per_point();
            let diff = diff_in(&drawn, &rebuilt, canvas_rect(&h), ppp);
            assert_eq!(diff, 0, "{label}: 2D の重ねが、作り直した物と違う");
            release(&h, *pts.last().unwrap(), PointerButton::Primary);
            h.run();
        }
    }
}

/// 幅いっぱいの長方形を選んで、3D の重ねの様子を読む。
fn select_wide(h: &mut H, width: u32) {
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Edit(SelEdit::Rect {
            x0: 10,
            y0: 10,
            x1: i64::from(width) - 10,
            y1: 50,
            mode: SelectionCombine::Replace,
        })));
    h.event(Event::PointerGone);
    h.run();
}

#[test]
fn a_document_wider_than_the_gpu_texture_limit_shows_no_selection_overlay_and_says_why() {
    let limit = {
        let (h, _) = wide_plate_view(64);
        h.state()
            .view3d_texture_limit()
            .expect("GPU の道がある（3D のレンダラー）")
    };
    for lang in Lang::ALL {
        let what = lang.pick(
            "選択範囲を 3D に出していません",
            "Selection not shown in 3D",
        );
        let size = lang.pick("GPU のテクスチャの上限", "GPU texture limit");
        let budget = lang.pick("GPU のメモリの予算", "GPU memory budget");
        // 上限を超える幅: 重ねのテクスチャを作らず（作ると wgpu の検証の誤りで落ちる）、大きさの理由で知らせる
        let width = limit + 128;
        let (mut h, _) = wide_plate_view(width);
        h.state_mut().state.lang = lang;
        select_wide(&mut h, width);
        let stats = h.state().view3d_stats().unwrap();
        assert!(
            stats.overlay_skipped && stats.overlay_too_large && stats.overlay_bytes == 0,
            "{lang:?}: {stats:?}"
        );
        assert_eq!(stats.overlay_tiles, 0, "{lang:?}");
        assert!(
            h.query_by_label_contains(what).is_some(),
            "{lang:?}: 隅のアイコンで知らせる"
        );
        assert!(
            h.query_by_label_contains(size).is_some()
                && h.query_by_label_contains(budget).is_none(),
            "{lang:?}: 理由は予算でなく、大きさの上限"
        );
        // 3D の絵は動く（落ちない）
        h.state_mut().state.view3d.camera.yaw += 5.0;
        h.run();
        assert!(
            h.state().view3d_stats().unwrap().overlay_skipped,
            "{lang:?}"
        );
        // 上限ちょうどの幅は出す（アイコンも無い）
        let (mut h, _) = wide_plate_view(limit);
        h.state_mut().state.lang = lang;
        select_wide(&mut h, limit);
        let stats = h.state().view3d_stats().unwrap();
        assert!(
            !stats.overlay_skipped && !stats.overlay_too_large && stats.overlay_bytes > 0,
            "{lang:?}: {stats:?}"
        );
        assert!(h.query_by_label_contains(what).is_none(), "{lang:?}");
    }
}

/// これ以上先なら、描き直しを頼んでいないのと同じ（点線を流す頼みは 60 ms）。
const IDLE: std::time::Duration = std::time::Duration::from_millis(500);

/// 最後のフレームが頼んだ、次の描き直しまでの時間（頼んでいなければ長い時間。`run` は遅れのある頼みを数えないので、これで見る）。
fn repaint_delay(h: &H) -> std::time::Duration {
    h.output()
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .expect("ルートのビューポート")
        .repaint_delay
}

#[test]
fn a_selection_that_is_not_moving_does_not_make_the_3d_view_redraw() {
    // 3D の縁の点線は止めたまま（2D の点線は流れるが、3D は流さない）。選択範囲があっても、何もしていない間は 3D を描き直さず、
    // 描き直しも頼まない（頼むと `run` が終わらない）
    let (mut h, rect) = cube_view();
    assert!(h.state().state.sel.animate, "点線を流す設定のまま");
    ellipse_on_front(&mut h, rect);
    let stats = h.state().view3d_stats().unwrap();
    assert!(
        stats.overlay_bytes > 0 && !stats.overlay_skipped,
        "{stats:?}"
    );
    h.run();
    let renders = h.state().view3d_stats().unwrap().renders;
    for _ in 0..20 {
        h.step();
    }
    h.run();
    assert_eq!(
        h.state().view3d_stats().unwrap().renders,
        renders,
        "入力が無ければ描き直さない"
    );
    assert!(
        repaint_delay(&h) >= IDLE,
        "点線を流すための描き直しを頼まない: {:?}",
        repaint_delay(&h)
    );
    // 予算に入らず重ねを出していないときも、描き直しを頼まない
    h.state_mut()
        .view3d_set_paint_budget(stats.paint_bytes + stats.overlay_bytes / 2);
    h.run();
    assert!(h.state().view3d_stats().unwrap().overlay_skipped);
    let renders = h.state().view3d_stats().unwrap().renders;
    for _ in 0..20 {
        h.step();
    }
    h.run();
    assert_eq!(h.state().view3d_stats().unwrap().renders, renders);
    assert!(
        repaint_delay(&h) >= IDLE,
        "重ねを出していなくても、描き直しを頼まない: {:?}",
        repaint_delay(&h)
    );
}
