//! 3D ビューは全部のテクスチャセットの絵を見せる（今のセットを替えても、ほかのセットの面から絵が消えない）。
//! 今のセットは文書の大きさのまま、ほかのセットは縮めた段で持ち、そのセットの文書が変わったときだけ上げ直す。メモリの予算が足りなければ、
//! 今のセットから遠いセットから持たない（その面は絵の無い描き方で、セットの一覧が印とツールチップで言う）。
//! 描きは egui_kittest（wgpu のソフトの描画）。絵の読みは光なしの「チャンネルだけ」の表示で値そのまま。
use crate::common;

use common::*;
use egui::Pos2;
use egui_kittest::Harness;
use yolu_app::state::Action;
use yolu_app::view3d::display::{EnvKind, Op, Shading};
use yolu_app::view3d::paint::Slot;
use yolu_app::YoluApp;
use yolu_core::geometry::OrbitCamera;
use yolu_core::glam::Vec3;
use yolu_core::{Channel, Rgba8};
use yolu_protocol::{
    channel, ChannelRoute, MaterialInfo, MaterialKey, MeshData, Model, Submesh, TextureProperty,
};

const COLORS: [[u8; 3]; 4] = [[220, 40, 40], [40, 200, 60], [50, 70, 230], [230, 210, 40]];

/// n 枚の板（左から右へ、マテリアル 0..n。カメラの方を向く。一辺 1、間 0.3）。マテリアルのテクスチャは size × size。
fn model(n: usize, size: u32) -> Model {
    model_with(n, size, n, &(0..n).collect::<Vec<_>>(), 1)
}

/// n 枚の板を `cols` 列の格子に置いたモデル。マテリアルの並びは `order`（`order[j]` = マテリアル j の名前の番号。板 i は名前 M{i} の
/// マテリアルの面）。世代は `generation`（替えたモデルを新しいモデルとして受けさせる）。
fn model_with(n: usize, size: u32, cols: usize, order: &[usize], generation: u32) -> Model {
    let materials = order
        .iter()
        .map(|i| MaterialInfo {
            key: MaterialKey::Material {
                name: format!("M{i}"),
                asset: None,
            },
            shader: "Standard".into(),
            textures: vec![TextureProperty {
                name: "_MainTex".into(),
                width: size,
                height: size,
            }],
            routes: vec![ChannelRoute {
                channel: channel::COLOR,
                property: "_MainTex".into(),
            }],
        })
        .collect();
    let meshes = (0..n)
        .map(|i| {
            let (x, y) = quad_pos(i, n, cols);
            MeshData {
                key: format!("{i}"),
                name: format!("板{i}"),
                skinned: false,
                positions: vec![
                    [x - 0.5, y - 0.5, 0.0],
                    [x + 0.5, y - 0.5, 0.0],
                    [x - 0.5, y + 0.5, 0.0],
                    [x + 0.5, y + 0.5, 0.0],
                ],
                normals: vec![],
                uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
                submeshes: vec![Submesh {
                    material: order.iter().position(|name| *name == i).unwrap() as u32,
                    indices: vec![0, 2, 1, 2, 3, 1],
                }],
            }
        })
        .collect();
    Model {
        generation,
        name: "板".into(),
        materials,
        meshes,
    }
}

/// 板 i の中心（n 枚を cols 列の格子に置いたとき）。
fn quad_pos(i: usize, n: usize, cols: usize) -> (f32, f32) {
    let (col, row) = (i % cols, i / cols);
    let rows = n.div_ceil(cols);
    (
        (col as f32 - (cols as f32 - 1.0) / 2.0) * 1.3,
        ((rows as f32 - 1.0) / 2.0 - row as f32) * 1.3,
    )
}

fn quad_x(i: usize, n: usize) -> f32 {
    quad_pos(i, n, n).0
}

/// n 枚の板を 3D ビューに出し、色のチャンネルだけを光なしで見る。セットは板の順（セット i = マテリアル i）。文書は size × size。
fn scene(n: usize, size: u32) -> Harness<'static, YoluApp> {
    scene_grid(n, n, size)
}

/// `scene` の、板を `cols` 列の格子に置く形（セットが多いときに、1 枚が小さくなりすぎないように）。
fn scene_grid(n: usize, cols: usize, size: u32) -> Harness<'static, YoluApp> {
    // （3D の表示域の幅が前の既定の並びと同じになるよう、右の列を広げた分だけウィンドウも広げる）
    let mut h = app(1474.0, 700.0, size);
    click_tab(&mut h, yolu_app::Tab::View3d);
    move_to(&h, egui::pos2(1.0, 1.0));
    h.run();
    h.state_mut()
        .load_live_link_model(&model_with(n, size, cols, &(0..n).collect::<Vec<_>>(), 1))
        .unwrap();
    h.run();
    h.state_mut().state.view3d.camera = OrbitCamera {
        target: Vec3::ZERO,
        yaw: 0.0,
        pitch: 0.0,
        // 1 列の横並びは板の数に合わせて引く。格子は縦にも並ぶので、高さに収まる距離に
        distance: if cols == n {
            2.0 + 1.3 * cols as f32 * 1.3
        } else {
            6.0 + 2.2 * cols as f32
        },
        model_radius: 1.0,
        ..Default::default()
    };
    h.state_mut()
        .apply(Action::View3d(Op::Shading(Shading::Channel(
            Channel::Color,
        ))));
    h.run();
    let s = &h.state().state;
    assert_eq!(s.sets.len(), n);
    for i in 0..n {
        assert_eq!(s.sets.get(i).unwrap().bound, Some(i as u32), "セット {i}");
    }
    h
}

fn op(h: &mut Harness<'_, YoluApp>, op: Op) {
    h.state_mut().apply(Action::View3d(op));
    h.run();
}

/// セット i の文書に、全面の Color の塗りつぶしレイヤーを足す（今のセットでなくてもよい）。
fn paint(h: &mut Harness<'_, YoluApp>, set: usize, rgb: [u8; 3]) -> yolu_core::LayerId {
    let layer = h
        .state_mut()
        .state
        .set_doc_mut(set)
        .add_fill_layer(
            "色",
            &[(Channel::Color, Rgba8::new(rgb[0], rgb[1], rgb[2], 255))],
            None,
        )
        .unwrap();
    h.run();
    layer
}

fn switch(h: &mut Harness<'_, YoluApp>, set: usize) {
    h.state_mut().state.switch_set(set).unwrap();
    h.run();
}

/// 世界の点が見えている画素。
fn screen_of(h: &Harness<'_, YoluApp>, world: Vec3) -> Pos2 {
    let area = h.state().view3d_rect().unwrap();
    let view = h
        .state()
        .state
        .view3d
        .camera
        .view(area.width(), area.height());
    let p = view.to_screen(world).expect("カメラの前");
    area.min + egui::vec2(p.x, p.y)
}

fn px(image: &image::RgbaImage, p: Pos2) -> [u8; 3] {
    let c = image.get_pixel(p.x.round() as u32, p.y.round() as u32).0;
    [c[0], c[1], c[2]]
}

fn luma(c: [u8; 3]) -> f32 {
    0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32
}

fn assert_close(actual: [u8; 3], expected: [u8; 3], tol: u8, what: &str) {
    for k in 0..3 {
        assert!(
            actual[k].abs_diff(expected[k]) <= tol,
            "{what}: {actual:?} と期待 {expected:?}（許す差 {tol}）"
        );
    }
}

/// 板 i の中心の画素（n 枚の板のうち）。
fn face(h: &mut Harness<'_, YoluApp>, i: usize, n: usize) -> [u8; 3] {
    face_grid(h, i, n, n)
}

/// 板 i の中心の画素（n 枚を cols 列の格子に置いたとき）。
fn face_grid(h: &mut Harness<'_, YoluApp>, i: usize, n: usize, cols: usize) -> [u8; 3] {
    let (x, y) = quad_pos(i, n, cols);
    let at = screen_of(h, Vec3::new(x, y, 0.0));
    px(&h.render().expect("描ける"), at)
}

/// 絵の無い面（市松）の明るさの範囲に入っているか（絵を貼らない面は、暗い灰色の市松）。
fn is_checker(c: [u8; 3]) -> bool {
    c[0] == c[1] && c[1] == c[2] && (55..=95).contains(&c[0])
}

/// 3D の表示域の中の、板が無い所（背景）の画素。
fn backdrop(h: &mut Harness<'_, YoluApp>) -> [u8; 3] {
    let area = h.state().view3d_rect().unwrap();
    let at = area.min + egui::vec2(8.0, 8.0);
    px(&h.render().expect("描ける"), at)
}

#[test]
fn every_set_shows_in_3d_and_switching_the_current_set_keeps_the_others() {
    let mut h = scene(3, 256);
    for (i, c) in COLORS.iter().take(3).enumerate() {
        paint(&mut h, i, *c);
    }
    // 今のセットをどう替えても、3 枚とも自分の絵のまま（替えるたびに前のセットの面から絵が消えていた）
    for current in [0, 1, 2, 0, 2, 1, 0] {
        switch(&mut h, current);
        for (i, want) in COLORS.iter().take(3).enumerate() {
            assert_close(
                face(&mut h, i, 3),
                *want,
                2,
                &format!("今のセット {current} のとき、板 {i}"),
            );
        }
        let s = h.state().view3d_stats().unwrap();
        assert_eq!(s.other_sets, 2, "今のセット {current}: {s:?}");
        assert_eq!((s.other_skipped, s.other_pending), (0, 0));
    }
    assert_eq!(h.state().view3d_held_materials().len(), 2);
}

#[test]
fn switching_sets_neither_uploads_the_model_again_nor_draws_while_nothing_changes() {
    let mut h = scene(3, 256);
    for (i, c) in COLORS.iter().take(3).enumerate() {
        paint(&mut h, i, *c);
    }
    let before = h.state().view3d_stats().unwrap();
    switch(&mut h, 1);
    switch(&mut h, 2);
    let after = h.state().view3d_stats().unwrap();
    assert_eq!(
        after.mesh_uploads, before.mesh_uploads,
        "頂点はマテリアル順に 1 度だけ。セットを替えても上げ直さない"
    );
    // 何も変わらないフレームは描かない・上げない
    let idle = h.state().view3d_stats().unwrap();
    h.run();
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!(s.renders, idle.renders);
    assert_eq!(s.last_tiles, 0);
    assert_eq!(s.other_bytes, idle.other_bytes);
}

#[test]
fn another_sets_picture_follows_its_document_while_it_is_not_current() {
    let mut h = scene(3, 256);
    paint(&mut h, 0, COLORS[0]);
    let layer = paint(&mut h, 1, COLORS[1]);
    paint(&mut h, 2, COLORS[2]);
    assert_close(face(&mut h, 1, 3), COLORS[1], 2, "初め");
    // ほかのセットの文書のレイヤーの値を替える（今のセットは 0 のまま）
    h.state_mut()
        .state
        .set_doc_mut(1)
        .set_fill_value(
            layer,
            Channel::Color,
            Some(Rgba8::new(COLORS[3][0], COLORS[3][1], COLORS[3][2], 255)),
            false,
        )
        .unwrap();
    h.run();
    assert_close(face(&mut h, 1, 3), COLORS[3], 2, "値を替えた");
    assert_eq!(h.state().state.sets.current_index(), 0);
    // レイヤーを足す（全面を覆う）
    paint(&mut h, 1, COLORS[2]);
    assert_close(face(&mut h, 1, 3), COLORS[2], 2, "レイヤーを足した");
    // 今のセットの面と、もう一方のセットの面は触っていない
    assert_close(face(&mut h, 0, 3), COLORS[0], 2, "今のセット");
    assert_close(face(&mut h, 2, 3), COLORS[2], 2, "もう一方");
}

#[test]
fn a_hidden_set_is_neither_shown_nor_held_and_comes_back_when_shown() {
    let mut h = scene(3, 256);
    for (i, c) in COLORS.iter().take(3).enumerate() {
        paint(&mut h, i, *c);
    }
    assert_eq!(h.state().view3d_held_materials().len(), 2);
    let uid = h.state().state.sets.get(2).unwrap().uid;
    h.state_mut().state.apply(Action::ToggleSetVisible(uid));
    h.run();
    // 隠したセットの面は見えず、その絵は持たない
    let at = screen_of(&h, Vec3::new(quad_x(2, 3), 0.0, 0.0));
    let hidden = px(&h.render().unwrap(), at);
    assert_eq!(hidden, backdrop(&mut h), "隠した面の所は背景");
    assert_eq!(h.state().view3d_held_materials(), vec![1]);
    assert_close(face(&mut h, 1, 3), COLORS[1], 2, "ほかは見えたまま");
    // 目を開くと、戻る（文書は変わっていないので、そのまま持ち直す）
    h.state_mut().state.apply(Action::ToggleSetVisible(uid));
    h.run();
    assert_close(face(&mut h, 2, 3), COLORS[2], 2, "戻った");
    assert_eq!(h.state().view3d_held_materials().len(), 2);
    // 今のセットを隠しても、ほかのセットの絵はそのまま
    let uid0 = h.state().state.sets.get(0).unwrap().uid;
    h.state_mut().state.apply(Action::ToggleSetVisible(uid0));
    h.run();
    assert_close(face(&mut h, 1, 3), COLORS[1], 2, "今のセットを隠した");
    assert_close(face(&mut h, 2, 3), COLORS[2], 2, "今のセットを隠した");
}

#[test]
fn a_set_over_the_memory_budget_is_left_out_farthest_first_and_the_list_says_why() {
    let mut h = scene(3, 256);
    for (i, c) in COLORS.iter().take(3).enumerate() {
        paint(&mut h, i, *c);
    }
    // 256² の Color だけ（ミップ込み 349,524 B + 重みの絵 87,381 B = 436,905 B）を 2 枚入れる予算（今のセット + ほかの 1 枚）
    h.state_mut().view3d_set_paint_budget(900_000);
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!((s.other_sets, s.other_skipped), (1, 1), "{s:?}");
    // 今のセット 0 から近い 1 を持ち、遠い 2 は持たない: 絵の無い面（市松）
    assert_close(face(&mut h, 0, 3), COLORS[0], 2, "今のセット");
    assert_close(face(&mut h, 1, 3), COLORS[1], 2, "近いセット");
    assert!(is_checker(face(&mut h, 2, 3)), "遠いセットは絵の無い描き方");
    assert_eq!(h.state().state.view3d.unpainted, vec![2]);
    // 一覧の行: 3 つ目だけ印（警告）とツールチップ。文は名前・状態・短い理由だけ
    let look = |h: &Harness<'_, YoluApp>, i| {
        yolu_app::panels::texture_sets::set_state(&h.state().state, i)
    };
    assert_eq!(look(&h, 0), None);
    assert_eq!(look(&h, 1), None);
    let mark = look(&h, 2).expect("印が出る");
    assert_eq!(mark.icon, "warning");
    assert!(mark.tooltip.contains("GPU"), "{}", mark.tooltip);
    assert_plain("絵を見せていないセットの印", &mark.tooltip);

    // 今のセットを替える: 2 に替えると、近い 1 を持ち、遠い 0 を持たない
    switch(&mut h, 2);
    assert_close(face(&mut h, 2, 3), COLORS[2], 2, "今のセット 2");
    assert_close(face(&mut h, 1, 3), COLORS[1], 2, "近いセット 1");
    assert!(is_checker(face(&mut h, 0, 3)), "遠いセット 0");
    assert_eq!(h.state().state.view3d.unpainted, vec![0]);
    assert_eq!(h.state().view3d_held_materials(), vec![1]);

    // 予算を戻すと、全部持つ。印も消える
    h.state_mut().view3d_set_paint_budget(512 << 20);
    h.run();
    assert_close(face(&mut h, 0, 3), COLORS[0], 2, "予算を戻した");
    assert!(h.state().state.view3d.unpainted.is_empty());
    assert_eq!(look(&h, 0), None);
    let s = h.state().view3d_stats().unwrap();
    assert_eq!((s.other_sets, s.other_skipped), (2, 0));
}

#[test]
fn new_pictures_for_the_other_sets_are_built_over_frames_and_the_window_keeps_asking_for_them() {
    // （3D の表示域の幅が前の既定の並びと同じになるよう、右の列を広げた分だけウィンドウも広げる）
    let mut h = app(1474.0, 700.0, 256);
    click_tab(&mut h, yolu_app::Tab::View3d);
    move_to(&h, egui::pos2(1.0, 1.0));
    h.run();
    // 1 フレームに作り始めるのは 1 つ（時間の枠を 0 に）
    h.state_mut()
        .view3d_set_other_build_budget(std::time::Duration::ZERO);
    h.state_mut().load_live_link_model(&model(4, 256)).unwrap();
    h.state_mut()
        .apply(Action::View3d(Op::Shading(Shading::Channel(
            Channel::Color,
        ))));
    h.state_mut().state.view3d.camera = OrbitCamera {
        target: Vec3::ZERO,
        yaw: 0.0,
        pitch: 0.0,
        distance: 9.0,
        model_radius: 1.0,
        ..Default::default()
    };
    for (i, c) in COLORS.iter().enumerate() {
        h.state_mut()
            .state
            .set_doc_mut(i)
            .add_fill_layer(
                "色",
                &[(Channel::Color, Rgba8::new(c[0], c[1], c[2], 255))],
                None,
            )
            .unwrap();
    }
    let mut built = Vec::new();
    for _ in 0..40 {
        h.step();
        let s = h.state().view3d_stats().unwrap();
        if built.last() != Some(&s.other_sets) {
            built.push(s.other_sets);
            // 待たせているセットがあるあいだ、ウィンドウは次のフレームを求める
            assert_eq!(
                h.state().view3d_wants_repaint(),
                s.other_pending > 0,
                "{s:?}"
            );
        }
        if s.other_sets == 3 && s.other_pending == 0 {
            break;
        }
    }
    assert!(
        built.windows(2).all(|w| w[1] == w[0] + 1),
        "1 つずつ増える: {built:?}"
    );
    assert_eq!(built.last(), Some(&3));
    h.run();
    assert!(!h.state().view3d_wants_repaint());
    for (i, want) in COLORS.iter().enumerate() {
        assert_close(face(&mut h, i, 4), *want, 2, &format!("板 {i}"));
    }
}

#[test]
fn the_current_sets_bytes_come_off_the_budget_before_the_others() {
    let mut h = scene(3, 256);
    for (i, c) in COLORS.iter().take(3).enumerate() {
        paint(&mut h, i, *c);
    }
    // 今のセットの絵（256² の Color だけ。ミップ込み 349,524 B + 重みの絵 87,381 B）が予算を食い切ったら、ほかのセットは 1 枚も持たない（今のセットが先）
    h.state_mut().view3d_set_paint_budget(436_905);
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!((s.other_sets, s.other_skipped), (0, 2), "{s:?}");
    assert_close(face(&mut h, 0, 3), COLORS[0], 2, "今のセットは見える");
    assert!(is_checker(face(&mut h, 1, 3)));
    assert!(is_checker(face(&mut h, 2, 3)));
    assert_eq!(h.state().state.view3d.unpainted, vec![1, 2]);
}

#[test]
fn other_sets_are_held_shrunk_to_the_cap_and_the_current_set_keeps_its_size() {
    let mut h = scene(2, 512);
    h.state_mut().view3d_set_other_cap(128);
    for (i, c) in COLORS.iter().take(2).enumerate() {
        paint(&mut h, i, *c);
    }
    let s = h.state().view3d_stats().unwrap();
    assert_eq!(s.paint_level, 0, "今のセットは文書の大きさのまま");
    assert_eq!(s.other_level, 2, "ほかのセットは 512 → 128（1/4）: {s:?}");
    assert_close(face(&mut h, 1, 2), COLORS[1], 2, "縮めて持っても色は同じ");
    let (texels, size, level) = h
        .state()
        .view3d_read_other_level(1, Slot::Color, 0)
        .expect("持っている");
    assert_eq!((size, level), ([128, 128], 2));
    assert_eq!(texels.len(), 128 * 128 * 4);
    // 同じセットが今のセットになると、文書の大きさ（512）で作り直す
    switch(&mut h, 1);
    assert_eq!(h.state().view3d_stats().unwrap().paint_level, 0);
    assert_eq!(
        h.state().view3d_read_paint_level(Slot::Color, 0).unwrap().1,
        [512, 512]
    );
    assert_eq!(
        h.state()
            .view3d_read_other_level(0, Slot::Color, 0)
            .unwrap()
            .2,
        2,
        "替わった前のセットは縮めて持つ"
    );
}

#[test]
fn raising_the_cap_builds_the_other_sets_again_at_the_larger_size() {
    let mut h = scene(2, 512);
    h.state_mut().view3d_set_other_cap(128);
    for (i, c) in COLORS.iter().take(2).enumerate() {
        paint(&mut h, i, *c);
    }
    let (_, size, level) = h
        .state()
        .view3d_read_other_level(1, Slot::Color, 0)
        .unwrap();
    assert_eq!((size, level), ([128, 128], 2));
    // 上限を上げると、縮めていた絵を大きさを戻して作り直す（文書は変わっていなくても）
    h.state_mut().view3d_set_other_cap(1024);
    h.run();
    let (texels, size, level) = h
        .state()
        .view3d_read_other_level(1, Slot::Color, 0)
        .unwrap();
    assert_eq!((size, level), ([512, 512], 0));
    assert_eq!(texels.len(), 512 * 512 * 4);
    assert_close(face(&mut h, 1, 2), COLORS[1], 2, "戻した絵");
    assert_eq!(h.state().view3d_stats().unwrap().other_level, 0);
}

/// 最初のレイヤー（絵を描けるレイヤー）に、模様（画素ごとに違う色）を全面に置く。
fn speckle(h: &mut Harness<'_, YoluApp>, set: usize, size: u32) {
    let doc = h.state_mut().state.set_doc_mut(set);
    let layer = doc.layers()[0].id();
    for y in 0..size {
        for x in 0..size {
            let k = x.wrapping_mul(2654435761) ^ y.wrapping_mul(40503) ^ (x * y);
            let v = (k >> 3) as u8;
            doc.set_channel_pixel(
                layer,
                Channel::Color,
                x,
                y,
                Rgba8::new(v, 255 - v, (k >> 11) as u8, 255),
            )
            .unwrap();
        }
    }
    h.run();
}

#[test]
fn a_demoted_picture_equals_one_built_from_the_document_at_the_cap() {
    // 今のセットだった絵は、替わるとき、ミップの段をコピーして縮める（文書を合成し直さない）。文書から縮めて作った絵と同じ
    let mut h = scene(2, 512);
    h.state_mut().view3d_set_other_cap(128);
    speckle(&mut h, 0, 512);
    paint(&mut h, 1, COLORS[1]);
    let before = h.state().view3d_stats().unwrap().other_demotions;
    switch(&mut h, 1);
    let (demoted, size, level) = h
        .state()
        .view3d_read_other_level(0, Slot::Color, 0)
        .unwrap();
    assert_eq!((size, level), ([128, 128], 2));
    assert_eq!(
        h.state().view3d_stats().unwrap().other_demotions,
        before + 1,
        "文書を合成し直さず、ミップのコピーで縮めた"
    );
    // 捨てて、文書から作り直す
    h.state_mut().view3d_invalidate_paint();
    h.run();
    let (built, size2, level2) = h
        .state()
        .view3d_read_other_level(0, Slot::Color, 0)
        .unwrap();
    assert_eq!((size2, level2), ([128, 128], 2));
    // 2 段の箱の平均（GPU の段ごとの丸め）と、1 度の 4 × 4 の平均の丸めの差だけ
    assert_levels_match(&demoted, &built, "縮めた絵と文書から作った絵");
    // 縮めた後に、そのセットの文書が変わっても、変わったタイルだけ足して合う: 文書の (4..8, 4..8) の 4 × 4 を白にすると、縮めた絵の
    // 1 画素（左から 1、上から 1 行目）が白になり、ほかの画素は文書から作り直した絵と合う
    let layer = h.state().state.set_doc(0).layers()[0].id();
    for y in 4..8 {
        for x in 4..8 {
            h.state_mut()
                .state
                .set_doc_mut(0)
                .set_channel_pixel(layer, Channel::Color, x, y, Rgba8::new(255, 255, 255, 255))
                .unwrap();
        }
    }
    h.run();
    let (after, _, _) = h
        .state()
        .view3d_read_other_level(0, Slot::Color, 0)
        .unwrap();
    let at = (128 + 1) * 4;
    assert!(
        built[at..at + 3].iter().all(|c| *c < 235),
        "文書を変える前は白でない: {:?}",
        &built[at..at + 4]
    );
    assert!(
        after[at..at + 4].iter().all(|c| *c >= 254),
        "変えた 4 × 4 の縮めた画素が白: {:?}",
        &after[at..at + 4]
    );
    h.state_mut().view3d_invalidate_paint();
    h.run();
    let (rebuilt, _, _) = h
        .state()
        .view3d_read_other_level(0, Slot::Color, 0)
        .unwrap();
    assert_levels_match(&after, &rebuilt, "変えたあとの縮めた絵と文書から作った絵");
}

/// 2 つの絵（同じ段・同じ大きさ）が、段ごとの丸めの差の範囲で合う（最大 3、平均 1 未満）。
fn assert_levels_match(a: &[u8], b: &[u8], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}: 大きさ");
    let (worst, mean) = level_difference(a, b);
    assert!(
        worst <= 3 && mean < 1.0,
        "{what}: 最大 {worst}、平均 {mean}"
    );
}

fn level_difference(a: &[u8], b: &[u8]) -> (u8, f64) {
    let worst = a.iter().zip(b).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
    let mean = a
        .iter()
        .zip(b)
        .map(|(a, b)| a.abs_diff(*b) as f64)
        .sum::<f64>()
        / a.len() as f64;
    (worst, mean)
}

#[test]
fn edits_made_just_before_the_set_changes_still_reach_the_demoted_picture() {
    // 前のセットの文書が、最後に同期してから変わっていて（描いたあと 1 フレームも描かずに替えた）、その場で縮める（コピー）と変わった分を足す
    // （足す書き込みがコピーより先に走って、古い絵に上書きされないこと）
    let mut h = scene(2, 512);
    h.state_mut().view3d_set_other_cap(128);
    paint(&mut h, 0, COLORS[0]);
    paint(&mut h, 1, COLORS[1]);
    assert_close(face(&mut h, 0, 2), COLORS[0], 2, "初め");
    h.state_mut()
        .state
        .set_doc_mut(0)
        .add_fill_layer(
            "あと",
            &[(
                Channel::Color,
                Rgba8::new(COLORS[2][0], COLORS[2][1], COLORS[2][2], 255),
            )],
            None,
        )
        .unwrap();
    h.state_mut().state.switch_set(1).unwrap();
    h.run();
    assert_close(face(&mut h, 0, 2), COLORS[2], 2, "替える前の変更が見える");
    assert_close(face(&mut h, 1, 2), COLORS[1], 2, "今のセット");
}

#[test]
fn a_picture_whose_mip_sizes_do_not_match_the_reduced_size_is_rebuilt_from_the_document() {
    // 510² を 1/4 にすると 128²（切り上げ）だが、ミップの 2 段目は 127²。大きさが合わないので、コピーでなく文書から作り直す
    let mut h = scene(2, 510);
    h.state_mut().view3d_set_other_cap(128);
    assert_eq!(h.state().state.doc.width(), 510);
    paint(&mut h, 0, COLORS[0]);
    paint(&mut h, 1, COLORS[1]);
    let before = h.state().view3d_stats().unwrap().other_demotions;
    switch(&mut h, 1);
    let (texels, size, level) = h
        .state()
        .view3d_read_other_level(0, Slot::Color, 0)
        .unwrap();
    assert_eq!((size, level), ([128, 128], 2));
    assert_eq!(texels.len(), 128 * 128 * 4);
    assert_eq!(h.state().view3d_stats().unwrap().other_demotions, before);
    assert_close(face(&mut h, 0, 2), COLORS[0], 2, "作り直しても同じ絵");
    assert_close(face(&mut h, 1, 2), COLORS[1], 2, "今のセット");
}

#[test]
fn every_channel_of_another_set_looks_the_same_as_when_it_is_the_current_set() {
    // 値が一様なので、縮めても同じ。6 チャンネルのどれも、ほかのセットのときと今のセットのときで同じに見える。ほかのセットの絵は上限 64 で
    // 縮めて持つ（256 → 64。切り替えのときはミップのコピー、作り始めるときは文書から縮める）。値は絵の無い面（市松の灰色）と見分けがつくものにする
    let mut h = scene(2, 256);
    h.state_mut().view3d_set_other_cap(64);
    let values: [(Channel, [u8; 4]); 5] = [
        (Channel::Color, [200, 120, 60, 255]),
        (Channel::Metallic, [150, 150, 150, 255]),
        (Channel::Roughness, [200, 200, 200, 255]),
        (Channel::Emission, [30, 60, 90, 255]),
        (Channel::Height, [140, 140, 140, 255]),
    ];
    for set in 0..2 {
        let values: Vec<(Channel, Rgba8)> = values
            .iter()
            .map(|(c, v)| (*c, Rgba8::new(v[0], v[1], v[2], v[3])))
            .collect();
        h.state_mut()
            .state
            .set_doc_mut(set)
            .add_fill_layer("値", &values, None)
            .unwrap();
    }
    // Normal は Height からも作る（Normal の出力）
    for set in 0..2 {
        h.state_mut()
            .state
            .set_doc_mut(set)
            .add_fill_layer(
                "法線",
                &[(Channel::Normal, Rgba8::new(190, 150, 230, 255))],
                None,
            )
            .unwrap();
    }
    h.run();
    // 「チャンネルだけ」の表示は値そのまま（Normal は向きをそろえるので数だけ動く）
    for (channel, expected, tolerance) in [
        (Channel::Color, [200, 120, 60], 1),
        (Channel::Metallic, [150, 150, 150], 1),
        (Channel::Roughness, [200, 200, 200], 1),
        (Channel::Normal, [190, 150, 230], 6),
        (Channel::Emission, [30, 60, 90], 1),
        (Channel::Height, [140, 140, 140], 1),
    ] {
        op(&mut h, Op::Shading(Shading::Channel(channel)));
        // セット 1 が今のセットのときと、ほかのセットのときの板 1
        switch(&mut h, 1);
        let as_current = face(&mut h, 1, 2);
        switch(&mut h, 0);
        let as_other = face(&mut h, 1, 2);
        assert_close(as_other, as_current, 1, &format!("{channel:?}"));
        assert!(
            !is_checker(as_other) && !is_checker(as_current),
            "{channel:?} は絵が出ている（絵の無い面でない）: {as_other:?} {as_current:?}"
        );
        assert_close(as_other, expected, tolerance, &format!("{channel:?} の値"));
    }
}

#[test]
fn a_normal_map_of_another_set_tilts_only_that_set() {
    let mut h = scene(3, 256);
    // 標準（PBR）の法線マップを見る試験: 新しいセットの既定（lilToon。ノーマルマップは切）でなく標準を明示する
    for set in 0..3 {
        h.state_mut()
            .state
            .set_doc_mut(set)
            .restore_look(yolu_core::look::MaterialLook::default())
            .unwrap();
    }
    op(&mut h, Op::Env(EnvKind::None));
    op(&mut h, Op::Shading(Shading::Material));
    // 灰色・粗い・金属でない 3 枚。真ん中のセットだけ右へ傾けた法線
    for set in 0..3 {
        let mut values = vec![
            (Channel::Color, Rgba8::new(200, 200, 200, 255)),
            (Channel::Metallic, Rgba8::new(0, 0, 0, 255)),
            (Channel::Roughness, Rgba8::new(255, 255, 255, 255)),
        ];
        if set == 1 {
            values.push((Channel::Normal, Rgba8::new(217, 128, 217, 255)));
        }
        h.state_mut()
            .state
            .set_doc_mut(set)
            .add_fill_layer("面", &values, None)
            .unwrap();
    }
    h.run();
    op(&mut h, Op::LightYaw(90.0));
    op(&mut h, Op::LightPitch(0.0));
    // 接線は別のスレッドが作る。出来るまで回す
    for _ in 0..400 {
        h.step();
        if h.state().view3d_stats().unwrap().tangents_exact {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let lit = |h: &mut Harness<'_, YoluApp>, i| luma(face(h, i, 3));
    // 今のセットが 0（法線マップの無いセット）。1 はほかのセットとして傾いて明るく、0 と 2 は傾かない
    let (a0, a1, a2) = (lit(&mut h, 0), lit(&mut h, 1), lit(&mut h, 2));
    assert!(
        a1 > a0 + 25.0,
        "右からの光で、傾けた板 1 が明るい: {a0} {a1} {a2}"
    );
    assert!((a0 - a2).abs() < 3.0, "法線マップの無い板は同じ: {a0} {a2}");
    // 1 が今のセットになっても同じ
    switch(&mut h, 1);
    let (b0, b1, b2) = (lit(&mut h, 0), lit(&mut h, 1), lit(&mut h, 2));
    assert!(
        (b1 - a1).abs() < 3.0,
        "今のセットでもほかのセットでも同じ: {a1} {b1}"
    );
    assert!((b0 - a0).abs() < 3.0 && (b2 - a2).abs() < 3.0, "{b0} {b2}");
}

fn synthetic_map(
    kind: yolu_core::mesh_maps::MeshMapKind,
    f: impl Fn(u32, u32) -> f32,
) -> yolu_core::mesh_maps::BakedMeshMap {
    use yolu_core::mesh_maps::{BakedMeshMap, MeshMapProvenance};
    let size = 16u32;
    let provenance = MeshMapProvenance {
        kind,
        engine_version: 1,
        mesh_hash: String::new(),
        topology_hash: String::new(),
        uv_channel: 0,
        width: size as i32,
        height: size as i32,
        target_slot: 0,
        target_slots: vec![0],
        padding: 0,
        antialiasing: 1,
        settings_key: String::new(),
        space: String::new(),
        pose: String::new(),
        source: String::new(),
        bounds_min: [0.0; 3],
        bounds_max: [1.0; 3],
    };
    let data = (0..size)
        .flat_map(|y| (0..size).map(move |x| (x, y)))
        .map(|(x, y)| (f(x, y) * 65535.0).round() as u16)
        .collect();
    BakedMeshMap::new(provenance, data, vec![1; (size * size) as usize]).unwrap()
}

#[test]
fn a_baked_mesh_map_shows_only_on_the_current_sets_faces() {
    use yolu_core::mesh_maps::MeshMapKind;
    let mut h = scene(2, 256);
    for (i, c) in COLORS.iter().take(2).enumerate() {
        paint(&mut h, i, *c);
    }
    {
        let state = &mut h.state_mut().state;
        let index = state.sets.current_index();
        state
            .sets
            .get_mut(index)
            .unwrap()
            .mesh_maps
            .put(vec![synthetic_map(
                MeshMapKind::AmbientOcclusion,
                |_, _| 0.5,
            )]);
    }
    h.run();
    op(
        &mut h,
        Op::Shading(Shading::MeshMap(MeshMapKind::AmbientOcclusion)),
    );
    // 今のセット（0）の面は 0.5 の灰色、ほかのセットの面は絵の無い描き方（今のセットのマップを貼らない）
    assert_close(face(&mut h, 0, 2), [128, 128, 128], 3, "今のセットのマップ");
    assert!(
        is_checker(face(&mut h, 1, 2)),
        "ほかのセットの面にマップを貼らない"
    );
}

/// 最初のレイヤーの 1 チャンネルに、画素ごとに違う値の模様を全面に置く。アルファも画素ごとに違い、透明（RGB は残っていても見えない）と
/// 半透明を含む。
fn speckle_channel(h: &mut Harness<'_, YoluApp>, set: usize, size: u32, channel: Channel) {
    let doc = h.state_mut().state.set_doc_mut(set);
    let layer = doc.layers()[0].id();
    for y in 0..size {
        for x in 0..size {
            let k = x.wrapping_mul(2654435761) ^ y.wrapping_mul(40503) ^ (x * y);
            let v = (k >> 3) as u8;
            let a = [0u8, 90, 200, 255][(k >> 17) as usize & 3];
            doc.set_channel_pixel(
                layer,
                channel,
                x,
                y,
                Rgba8::new(v, 255 - v, (k >> 11) as u8, a),
            )
            .unwrap();
        }
    }
    h.run();
}

/// セット 0 の文書を `configure` で整え、今のセットを 1 に替える（前の絵をミップのコピーで縮める）。縮めた絵（`slots` の各チャンネル）が、
/// 文書から上限の大きさで作り直した絵と合う（辺 256 を上限 64 へ、2 段）。絵が模様のままで、どれも平らな絵でないことも確かめる。
fn assert_demoted_equals_built(configure: impl Fn(&mut Harness<'_, YoluApp>), slots: &[Slot]) {
    let mut h = scene(2, 256);
    h.state_mut().view3d_set_other_cap(64);
    configure(&mut h);
    paint(&mut h, 1, COLORS[1]);
    let before = h.state().view3d_stats().unwrap().other_demotions;
    switch(&mut h, 1);
    assert_eq!(
        h.state().view3d_stats().unwrap().other_demotions,
        before + 1,
        "文書を合成し直さず、ミップのコピーで縮めた"
    );
    let read = |h: &Harness<'_, YoluApp>, slot: Slot| {
        let (texels, size, level) = h
            .state()
            .view3d_read_other_level(0, slot, 0)
            .unwrap_or_else(|| panic!("{slot:?} の絵がある"));
        assert_eq!((size, level), ([64, 64], 2), "{slot:?}");
        texels
    };
    let demoted: Vec<Vec<u8>> = slots.iter().map(|slot| read(&h, *slot)).collect();
    h.state_mut().view3d_invalidate_paint();
    h.run();
    for (slot, demoted) in slots.iter().zip(&demoted) {
        let built = read(&h, *slot);
        let (min, max) = built
            .iter()
            .fold((255u8, 0u8), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
        assert!(max - min > 100, "{slot:?} は模様がある: {min}..{max}");
        let (worst, mean) = level_difference(demoted, &built);
        assert!(
            worst <= 3 && mean < 1.0,
            "{slot:?}: 縮めた絵と文書から作った絵の差 最大 {worst}、平均 {mean}"
        );
    }
}

#[test]
fn a_demoted_picture_equals_one_built_from_the_document_for_translucent_color_and_scalar_channels()
{
    // 半透明・透明を含む Color と Emission（乗算済みで縮める）、スカラー（値 × アルファで縮める。Metallic・Roughness・Height）
    assert_demoted_equals_built(
        |h| {
            for channel in [
                Channel::Color,
                Channel::Emission,
                Channel::Metallic,
                Channel::Roughness,
                Channel::Height,
            ] {
                speckle_channel(h, 0, 256, channel);
            }
        },
        &[
            Slot::Color,
            Slot::Emission,
            Slot::Metallic,
            Slot::Roughness,
            Slot::Height,
        ],
    );
}

#[test]
fn a_demoted_normal_picture_equals_one_built_from_the_document_for_derived_and_painted_normals() {
    use yolu_core::{HeightEdgeMode, NormalSettings};
    // Height から作る Normal（端は Clamp）と、その上に描いた Normal
    assert_demoted_equals_built(
        |h| {
            speckle_channel(h, 0, 256, Channel::Height);
            speckle_channel(h, 0, 256, Channel::Normal);
            h.state_mut()
                .state
                .set_doc_mut(0)
                .set_normal_settings(NormalSettings::DEFAULT.with_derive(true), false)
                .unwrap();
            h.run();
        },
        &[Slot::Normal, Slot::Height],
    );
    // 端が反対側の端を読む Height → Normal（Wrap。文書全体を 1 つの矩形で作る道）
    assert_demoted_equals_built(
        |h| {
            speckle_channel(h, 0, 256, Channel::Height);
            h.state_mut()
                .state
                .set_doc_mut(0)
                .set_normal_settings(
                    NormalSettings::DEFAULT
                        .with_derive(true)
                        .with_edges(HeightEdgeMode::Wrap),
                    false,
                )
                .unwrap();
            h.run();
        },
        &[Slot::Normal, Slot::Height],
    );
}

#[test]
fn the_other_sets_do_not_keep_their_composition_work_buffers() {
    use yolu_core::{HeightEdgeMode, NormalSettings};
    // Height から作る Wrap の Normal は、文書全体を作業用のバッファに取る（512² で約 2 MiB）。ほかのセットが作り終えたあとも抱えると、
    // セットの数に比例して CPU のメモリが残る
    let mut h = scene(4, 512);
    for set in 0..4 {
        let doc = h.state_mut().state.set_doc_mut(set);
        doc.add_fill_layer(
            "値",
            &[
                (Channel::Color, Rgba8::new(200, 60, 30, 255)),
                (Channel::Height, Rgba8::new(120, 120, 120, 255)),
            ],
            None,
        )
        .unwrap();
        doc.set_normal_settings(
            NormalSettings::DEFAULT
                .with_derive(true)
                .with_edges(HeightEdgeMode::Wrap),
            false,
        )
        .unwrap();
    }
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!((s.other_sets, s.other_pending), (3, 0), "{s:?}");
    for material in 1..4 {
        assert!(
            h.state()
                .view3d_read_other_level(material, Slot::Normal, 0)
                .is_some(),
            "セット {material} の Normal を作った"
        );
    }
    assert_eq!(s.other_scratch_bytes, 0, "作り終えたら手放す: {s:?}");
    // 切り替え（前の今のセットがほかのセットへ回る）・文書の変更・作り直しのあとも
    for to in [1, 2, 3, 0] {
        switch(&mut h, to);
        let s = h.state().view3d_stats().unwrap();
        assert_eq!(s.other_scratch_bytes, 0, "今のセット {to}: {s:?}");
    }
    let layer = h.state().state.set_doc(2).layers()[0].id();
    h.state_mut()
        .state
        .set_doc_mut(2)
        .set_channel_pixel(layer, Channel::Height, 9, 9, Rgba8::new(255, 255, 255, 255))
        .unwrap();
    h.run();
    h.state_mut().view3d_invalidate_paint();
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!(s.other_sets, 3);
    assert_eq!(s.other_scratch_bytes, 0, "作り直したあとも: {s:?}");
}

#[test]
fn switching_the_current_set_never_holds_the_old_and_the_new_full_pictures_together() {
    // 512² の Color だけ（ミップ込み 1,398,100 B + 重みの絵 349,525 B = 1,747,625 B）が今のセット、128² に縮めたほかのセットが 87,380 B + 21,845 B =
    // 109,225 B。予算 1.9 MB は今のセット 1 枚と、ほかの 1 枚だけが入る。替わるフレームの途中で、前の絵（満量）と新しい絵（満量）が重なると、
    // 予算の 2 倍近くになっていた
    const BUDGET: u64 = 1_900_000;
    let mut h = scene(3, 512);
    h.state_mut().view3d_set_other_cap(128);
    for (i, c) in COLORS.iter().take(3).enumerate() {
        paint(&mut h, i, *c);
    }
    h.state_mut().view3d_set_paint_budget(BUDGET);
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!((s.other_sets, s.other_skipped), (1, 1), "{s:?}");
    for to in [1, 2, 0, 2, 1, 0] {
        switch(&mut h, to);
        let s = h.state().view3d_stats().unwrap();
        assert!(
            s.peak_bytes > 1_747_625 && s.peak_bytes <= BUDGET,
            "今のセット {to} へ替えたフレームの途中の最大: {} B（予算 {BUDGET} B）{s:?}",
            s.peak_bytes
        );
        assert_close(face(&mut h, to, 3), COLORS[to], 2, "今のセット");
    }
}

#[test]
fn a_picture_that_cannot_be_copied_down_is_rebuilt_at_once_instead_of_staying_full_size() {
    // 510² は 1/4 にすると 128²（切り上げ）だが、ミップの 2 段目は 127² で、コピーでは縮められない。満量のまま次のフレームへ残さず、
    // その場で文書から縮めて作り直す（新しい今のセットの絵と重ならない）。セット 0 の文書だけが 510²（ほかのセットは 512² で、コピーで縮まる）
    const BUDGET: u64 = 2_000_000;
    let mut h = scene(2, 510);
    h.state_mut().view3d_set_other_cap(128);
    paint(&mut h, 0, COLORS[0]);
    paint(&mut h, 1, COLORS[1]);
    h.state_mut().view3d_set_paint_budget(BUDGET);
    h.run();
    let before = h.state().view3d_stats().unwrap().other_demotions;
    let mut copied = 0;
    for (to, by_copy) in [(1, 0), (0, 1), (1, 0)] {
        switch(&mut h, to);
        copied += by_copy;
        let s = h.state().view3d_stats().unwrap();
        assert_eq!(
            s.other_demotions,
            before + copied,
            "今のセット {to}: 510² の絵だけがコピーでは縮まない"
        );
        assert_eq!(
            (s.other_sets, s.other_level, s.other_pending),
            (1, 2, 0),
            "{s:?}"
        );
        assert!(
            s.peak_bytes > 1_625_000 && s.peak_bytes <= BUDGET,
            "今のセット {to}: 途中の最大 {} B: {s:?}",
            s.peak_bytes
        );
        assert_close(face(&mut h, 0, 2), COLORS[0], 2, "板 0");
        assert_close(face(&mut h, 1, 2), COLORS[1], 2, "板 1");
    }
}

#[test]
fn switching_the_current_set_keeps_a_picture_the_budget_shrank_instead_of_building_it_again() {
    // セット 0 は Color + Normal + Emission と重みの絵（13 B/テクセル。512² で 4,543,825 B）で、予算 3,750,000 B には入らず 256²（1,135,953 B）へ縮む。
    // そのあと Normal と Emission のレイヤーを消すと Color だけと重みの絵（512² で 1,747,625 B）になるが、縮めは上げるだけなので 256² のまま。
    // 今のセットを 1 に替えるとき、前の絵は縮めたまま持ち越す。ほかのセットへ回すときの `u64::MAX` の入れ直しを「予算を上げた」と
    // 数えると、替えるたびに 512² へ作り直す（新しい絵を作る前に前の絵を縮める、が逆になる）。予算を上げたときだけ戻る
    // （`lowering_the_memory_shrinks_the_current_sets_picture_and_raising_it_restores_the_size`）
    const BUDGET: u64 = 3_750_000;
    let mut h = scene(2, 512);
    paint(&mut h, 0, COLORS[0]);
    paint(&mut h, 1, COLORS[1]);
    let extra = h
        .state_mut()
        .state
        .set_doc_mut(0)
        .add_fill_layer(
            "余分",
            &[
                (Channel::Normal, Rgba8::new(128, 128, 255, 255)),
                (Channel::Emission, Rgba8::new(10, 20, 30, 255)),
            ],
            None,
        )
        .unwrap();
    h.run();
    h.state_mut().view3d_set_paint_budget(BUDGET);
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!((s.paint_level, s.paint_by_budget), (1, true), "{s:?}");
    assert_eq!((s.other_sets, s.other_skipped), (1, 0), "{s:?}");
    h.state_mut()
        .state
        .set_doc_mut(0)
        .remove_layer(extra)
        .unwrap();
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!(s.paint_level, 1, "縮めは上げるだけ: {s:?}");
    // 替える: 前の絵（セット 0）は縮めたまま。作り直さない
    switch(&mut h, 1);
    let s = h.state().view3d_stats().unwrap();
    assert_eq!(
        (s.paint_level, s.other_sets, s.other_skipped),
        (0, 1, 0),
        "{s:?}"
    );
    assert_eq!(
        (s.other_level, s.other_bytes),
        (1, 436_905),
        "縮めたまま持ち越す: {s:?}"
    );
    let (_, size, level) = h
        .state()
        .view3d_read_other_level(0, Slot::Color, 0)
        .unwrap();
    assert_eq!((size, level), ([256, 256], 1));
    assert_close(face(&mut h, 0, 2), COLORS[0], 2, "縮めたままの絵");
    // 前の絵が今のセットへ戻ると、文書の大きさで作る（下げるのは文書が替わるとき）
    switch(&mut h, 0);
    let s = h.state().view3d_stats().unwrap();
    assert_eq!((s.paint_level, s.paint_by_budget), (0, false), "{s:?}");
}

#[test]
fn a_material_with_every_face_hidden_by_the_pose_is_not_held_and_comes_back_with_its_faces() {
    use std::sync::Arc;
    use yolu_app::view3d::pose::hide::FaceMask;
    let mut h = scene(3, 256);
    for (i, c) in COLORS.iter().take(3).enumerate() {
        paint(&mut h, i, *c);
    }
    assert_eq!(h.state().view3d_held_materials(), vec![1, 2]);
    let bytes = h.state().view3d_stats().unwrap().other_bytes;
    // 三角形は板ごとに 2 つ（0 1 | 2 3 | 4 5）。板 2 の両方を隠す（ポーズの「面を隠す」）
    let mask = |hidden: [bool; 6]| Some(Arc::new(FaceMask::new(hidden.to_vec())));
    h.state_mut()
        .state
        .view3d
        .set_face_mask(mask([false, false, false, false, true, true]));
    h.run();
    // 面の無いマテリアルのセットは持たない・絵を見せられない扱い（予算の警告）にもしない
    assert_eq!(h.state().view3d_held_materials(), vec![1]);
    let s = h.state().view3d_stats().unwrap();
    assert_eq!((s.other_sets, s.other_skipped), (1, 0), "{s:?}");
    assert!(s.other_bytes < bytes);
    assert!(h.state().state.view3d.unpainted.is_empty());
    assert_eq!(
        yolu_app::panels::texture_sets::set_state(&h.state().state, 2),
        None
    );
    assert_eq!(face(&mut h, 2, 3), backdrop(&mut h), "隠した面は見えない");
    assert_close(face(&mut h, 1, 3), COLORS[1], 2, "ほかの面はそのまま");
    // 面を戻すと、文書から作り直して絵が出る
    h.state_mut().state.view3d.set_face_mask(None);
    h.run();
    assert_eq!(h.state().view3d_held_materials(), vec![1, 2]);
    assert_close(face(&mut h, 2, 3), COLORS[2], 2, "戻した面");
    // 面の一部だけを隠すなら、マテリアルは残り、見える面にはそのセットの絵が出る（板 2 の右上の三角形だけ隠す）
    h.state_mut()
        .state
        .view3d
        .set_face_mask(mask([false, false, false, false, false, true]));
    h.run();
    assert_eq!(h.state().view3d_held_materials(), vec![1, 2]);
    let (cx, cy) = (quad_x(2, 3), 0.0);
    let at = |h: &mut Harness<'_, YoluApp>, dx: f32, dy: f32| {
        let p = screen_of(h, Vec3::new(cx + dx, cy + dy, 0.0));
        px(&h.render().unwrap(), p)
    };
    assert_close(at(&mut h, -0.25, -0.25), COLORS[2], 2, "見える三角形");
    assert_eq!(at(&mut h, 0.25, 0.25), backdrop(&mut h), "隠した三角形");
}

#[test]
fn a_set_follows_its_material_when_the_model_is_replaced_and_the_materials_change_order() {
    let mut h = scene(3, 256);
    for (i, c) in COLORS.iter().take(3).enumerate() {
        paint(&mut h, i, *c);
    }
    assert_eq!(h.state().view3d_held_materials(), vec![1, 2]);
    // マテリアルの並びが替わったモデル（スロット 0 = M2、1 = M0、2 = M1）。セットは名前でマテリアルに付き直る
    h.state_mut()
        .load_live_link_model(&model_with(3, 256, 3, &[2, 0, 1], 2))
        .unwrap();
    h.run();
    let bound: Vec<_> = (0..3)
        .map(|i| h.state().state.sets.get(i).unwrap().bound)
        .collect();
    assert_eq!(bound, vec![Some(1), Some(2), Some(0)]);
    // ほかのセット（M1・M2）の絵は付き直したマテリアル（2・0）の面に出る。持ち物の記録も付き直した先
    assert_eq!(h.state().view3d_held_materials(), vec![2, 0]);
    for (i, want) in COLORS.iter().take(3).enumerate() {
        assert_close(face(&mut h, i, 3), *want, 2, &format!("板 {i}"));
    }
    // 今のセットを替えても
    switch(&mut h, 2);
    for (i, want) in COLORS.iter().take(3).enumerate() {
        assert_close(
            face(&mut h, i, 3),
            *want,
            2,
            &format!("セット 2 のとき、板 {i}"),
        );
    }
    assert!(h.state().state.view3d.unpainted.is_empty());
}

#[test]
fn sixty_four_sets_each_show_their_own_picture() {
    // セットの数の上限（64）。1 つが今のセット、63 がほかのセット
    const N: usize = 64;
    let colors: Vec<[u8; 3]> = (0..N)
        .map(|i| {
            [
                (i * 53 % 200) as u8 + 30,
                (i * 97 % 200) as u8 + 30,
                (i * 29 % 200) as u8 + 30,
            ]
        })
        .collect();
    for (i, c) in colors.iter().enumerate() {
        assert!(colors[..i].iter().all(|d| d != c), "色 {i} は他と違う");
    }
    let mut h = scene_grid(N, 8, 64);
    for (set, c) in colors.iter().enumerate() {
        h.state_mut()
            .state
            .set_doc_mut(set)
            .add_fill_layer(
                "色",
                &[(Channel::Color, Rgba8::new(c[0], c[1], c[2], 255))],
                None,
            )
            .unwrap();
    }
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!(
        (s.other_sets, s.other_skipped, s.other_pending),
        (N - 1, 0, 0),
        "{s:?}"
    );
    for current in [0, N - 1, 27] {
        switch(&mut h, current);
        for (i, want) in colors.iter().enumerate() {
            assert_close(
                face_grid(&mut h, i, N, 8),
                *want,
                2,
                &format!("今のセット {current}、板 {i}"),
            );
        }
        assert_eq!(h.state().view3d_held_materials().len(), N - 1);
    }
}

#[test]
fn the_list_mark_for_a_set_left_out_of_the_budget_says_why_in_both_languages() {
    use yolu_app::lang::Lang;
    use yolu_app::panels::texture_sets::set_state;
    let mut h = scene(3, 256);
    for (i, c) in COLORS.iter().take(3).enumerate() {
        paint(&mut h, i, *c);
    }
    h.state_mut().view3d_set_paint_budget(900_000);
    h.run();
    assert_eq!(h.state().state.view3d.unpainted, vec![2]);
    let look = |h: &Harness<'_, YoluApp>, i| set_state(&h.state().state, i);
    let left_out = look(&h, 2).expect("印が出る");
    assert_eq!(left_out.icon, "warning");
    assert!(left_out.tooltip.contains("GPU"), "{}", left_out.tooltip);
    left_out
        .tooltip
        .lines()
        .for_each(|line| assert_plain("予算の印", line));
    assert!(look(&h, 1).is_none(), "予算に収まるセットに印は無い");
    h.state_mut().state.lang = Lang::En;
    let en = look(&h, 2).unwrap();
    assert!(en.tooltip.contains("GPU memory budget"), "{}", en.tooltip);
    assert!(!has_japanese(&en.tooltip), "{}", en.tooltip);
    h.state_mut().state.lang = Lang::Ja;
    // 予算を戻すと、印が消える
    h.state_mut().view3d_set_paint_budget(512 << 20);
    h.run();
    assert!(look(&h, 2).is_none());
}

#[test]
fn a_budget_mark_does_not_outlive_the_model_or_the_current_set_while_the_3d_view_is_not_drawn() {
    use yolu_app::panels::texture_sets::set_state;
    let mut h = scene(3, 256);
    for (i, c) in COLORS.iter().take(3).enumerate() {
        paint(&mut h, i, *c);
    }
    h.state_mut().view3d_set_paint_budget(900_000);
    h.run();
    assert_eq!(h.state().state.view3d.unpainted, vec![2]);
    assert!(set_state(&h.state().state, 2).is_some());

    // キャンバスのタブが前に出ている間は 3D が描かれず、印の元は更新されない。モデルを替えると前のモデルのマテリアルの番号の印は消える
    // （今のセット 0 のマテリアルは 0 のまま、マテリアルの並びが替わって、番号 2 はセット 1 のマテリアルになる）
    click_tab(&mut h, yolu_app::Tab::Canvas);
    h.state_mut()
        .load_live_link_model(&model_with(3, 256, 3, &[0, 2, 1], 2))
        .unwrap();
    h.run();
    assert!(h.state().state.view3d.unpainted.is_empty());
    for i in 0..3 {
        assert_eq!(
            set_state(&h.state().state, i),
            None,
            "セット {i} に前の印が残らない"
        );
    }
    // 3D が前に出ると、新しいモデルのマテリアルで計算し直す（今のセット 0 から遠いセット 2 のマテリアルは 1）
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    assert_eq!(h.state().state.view3d.unpainted, vec![1]);
    assert!(set_state(&h.state().state, 2).is_some());
    assert_eq!(set_state(&h.state().state, 0), None);

    // 3D が描かれていないあいだに今のセットを替えても、前の印を残さない（今のセット 2 に、見せていない印が付かない）
    click_tab(&mut h, yolu_app::Tab::Canvas);
    h.state_mut().state.switch_set(2).unwrap();
    h.run();
    assert!(h.state().state.view3d.unpainted.is_empty());
    assert_eq!(set_state(&h.state().state, 2), None);
}

#[test]
fn snapshot_sets_in_3d_with_the_farthest_set_left_out_of_the_budget() {
    let mut snapshots = egui_kittest::SnapshotResults::new();
    for lang in yolu_app::lang::Lang::ALL {
        let mut h = scene(3, 256);
        h.state_mut().state.lang = lang;
        for (i, c) in COLORS.iter().take(3).enumerate() {
            paint(&mut h, i, *c);
        }
        h.state_mut().view3d_set_paint_budget(900_000);
        h.run();
        h.snapshot(lang.pick("view3d_sets_over_budget_ja", "view3d_sets_over_budget_en"));
        snapshots.extend_harness(&mut h);
    }
}

/// 計測: 三角形の数 × 絵を持つセットの数での、1 フレームの時間（カメラを回すだけ・絵が変わらないとき）。マテリアルごとの描き
/// （この実装）の、1 回の描き（同じ三角形を 1 つのセットで描く）に対する増え。
/// `cargo test -p yolu-app --test gui_view3d view3d_sets::measure -- --ignored --nocapture`
#[test]
#[ignore = "計測"]
fn measure_frames_by_triangles_and_set_count() {
    use std::time::Instant;
    use yolu_app::view3d::model::ViewModel;
    use yolu_core::geometry::{cube_sphere, ModelMesh, Submesh as CoreSubmesh};

    fn sphere_with_materials(grid: u32, n: usize) -> ModelMesh {
        let mut mesh = cube_sphere(grid, 0.5);
        let indices = std::mem::take(&mut mesh.submeshes[0].indices);
        let triangles = indices.len() / 3;
        mesh.submeshes = (0..n)
            .map(|m| {
                let (a, b) = (triangles * m / n, triangles * (m + 1) / n);
                CoreSubmesh {
                    material: m as i32,
                    indices: indices[a * 3..b * 3].to_vec(),
                }
            })
            .collect();
        mesh
    }

    let mut h = scene(1, 1024);
    println!("GPU: {}", h.state().view3d_adapter().unwrap_or_default());
    op(&mut h, Op::Shading(Shading::Material));
    op(&mut h, Op::Env(EnvKind::Studio));
    for grid in [20u32, 77, 160] {
        for n in [1usize, 4, 16, 64] {
            // n 個のマテリアルに球を分け、n 個のセットを作って全部に絵を置く
            let mesh = sphere_with_materials(grid, n);
            let triangles: usize = mesh.submeshes.iter().map(|s| s.indices.len() / 3).sum();
            {
                let state = &mut h.state_mut().state;
                // セットを n 個にする（足りなければ足す）
                while state.sets.len() < n {
                    state.add_texture_set().unwrap();
                }
            }
            for i in 0..n {
                let state = &mut h.state_mut().state;
                state.sets.get_mut(i).unwrap().bound = Some(i as u32);
                state.sets.get_mut(i).unwrap().visible = true;
            }
            let revision = h.state_mut().state.view3d.next_revision();
            let model = ViewModel::new("球", vec![mesh], vec![None; n], revision).unwrap();
            model.tangents();
            h.state_mut().state.view3d.set_model(model);
            // 今のセットを、材質 0 に付いたセット 0 にする（この人形は Live Link のモデルでないので、描くマテリアルは常に 0。今のセットが別だと、
            // 材質 0 の面に別のセットの絵が貼られ、本当のセットの面が 1 つ絵の無い描き方になる）
            h.state_mut().state.switch_set(0).unwrap();
            h.state_mut().state.view3d.material = 0;
            h.state_mut().state.view3d.camera = OrbitCamera {
                target: Vec3::ZERO,
                yaw: 20.0,
                pitch: 10.0,
                distance: 1.6,
                model_radius: 1.0,
                ..Default::default()
            };
            for set in 0..n {
                let rgb = COLORS[set % 4];
                h.state_mut()
                    .state
                    .set_doc_mut(set)
                    .add_fill_layer(
                        "色",
                        &[
                            (Channel::Color, Rgba8::new(rgb[0], rgb[1], rgb[2], 255)),
                            (Channel::Roughness, Rgba8::new(120, 120, 120, 255)),
                        ],
                        None,
                    )
                    .unwrap();
            }
            h.state_mut().state.sync_view3d();
            h.run();
            h.state().view3d_wait_gpu();
            // カメラを回すだけ（絵は同じ）の 1 フレーム。GPU の完了まで待つ
            let mut wall = Vec::new();
            let mut cpu = Vec::new();
            for frame in 0..48 {
                h.state_mut().state.view3d.camera.yaw += 3.0 + frame as f32 * 0.01;
                let started = Instant::now();
                h.step();
                h.state().view3d_wait_gpu();
                wall.push(started.elapsed().as_micros() as f64 / 1000.0);
                cpu.push(h.state().view3d_stats().unwrap().last_prepare_us as f64 / 1000.0);
            }
            let s = h.state().view3d_stats().unwrap();
            let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
            println!(
                "三角形 {triangles:>6}・セット {n:>2}（絵を持つ {}・持てず {}・待ち {}）: 全体 {:.2} ms、prepare の CPU {:.2} ms",
                1 + s.other_sets,
                s.other_skipped,
                s.other_pending,
                mean(&wall[8..]),
                mean(&cpu[8..]),
            );
        }
    }
}

/// 計測: セットを替えたフレームの時間（4096² の 3 セット、Color・Roughness・Emission・Normal の 4 チャンネル）。今のセットの作り直し
/// （これは従来も要った）に、前のセットをミップのコピーで縮めてほかのセットへ回す分が足される。ほかのセットを見せない形
/// （`view3d_set_show_other_sets(false)`: 今のセットの絵だけを同期する。変更前の実装と同じ仕事）と、見せる形（普段）を、同じ実行の中で
/// 同じ切り替えの順で測り、1 フレームの時間・同期の時間・同期の途中で GPU に持っていた絵の最大のバイト数を比べる。初めてほかのセットを
/// 作るフレームも測る。
/// `cargo test -p yolu-app --test gui_view3d view3d_sets::measure_switching -- --ignored --nocapture`
#[test]
#[ignore = "計測"]
fn measure_switching_sets() {
    use std::time::Instant;
    let mut h = scene(3, 4096);
    println!("GPU: {}", h.state().view3d_adapter().unwrap_or_default());
    h.state_mut()
        .apply(Action::View3d(Op::Shading(Shading::Material)));
    for (set, rgb) in COLORS.iter().take(3).enumerate() {
        let doc = h.state_mut().state.set_doc_mut(set);
        doc.add_fill_layer(
            "値",
            &[
                (Channel::Color, Rgba8::new(rgb[0], rgb[1], rgb[2], 255)),
                (Channel::Roughness, Rgba8::new(120, 120, 120, 255)),
                (Channel::Emission, Rgba8::new(10, 10, 10, 255)),
                (Channel::Normal, Rgba8::new(128, 128, 255, 255)),
            ],
            None,
        )
        .unwrap();
        // 絵のあるタイルも少し（キャンバスの全面に効くレイヤーだけでなく、合成の手間が要るように）
        let layer = doc.layers()[0].id();
        for k in 0..64u32 {
            doc.set_channel_pixel(
                layer,
                Channel::Color,
                100 + k * 55,
                200 + k * 31,
                Rgba8::new(255, 255, 255, 255),
            )
            .unwrap();
        }
    }
    // 初めてのフレーム: 今のセット（0）を作り、ほかのセットを 1 フレームに 1 つずつ作る
    for frame in 0..4 {
        let started = Instant::now();
        h.step();
        h.state().view3d_wait_gpu();
        let s = h.state().view3d_stats().unwrap();
        println!(
            "初め {frame} フレーム目: {:.1} ms（同期 {:.1} ms）、ほかのセット {} 持つ・待ち {}",
            started.elapsed().as_secs_f64() * 1000.0,
            s.last_sync_us as f64 / 1000.0,
            s.other_sets,
            s.other_pending
        );
    }
    h.run();
    for round in 0..2 {
        for show in [false, true] {
            h.state_mut().view3d_set_show_other_sets(show);
            // 絵を捨てて作り直し、ほかのセットの顔ぶれをそろえてから測る
            h.state_mut().view3d_invalidate_paint();
            h.run();
            let (mut walls, mut syncs, mut peak) = (Vec::new(), Vec::new(), 0u64);
            let mut held = 0;
            for to in [1usize, 2, 0, 1, 0, 2, 1, 2, 0] {
                h.state_mut().state.switch_set(to).unwrap();
                let started = Instant::now();
                h.step();
                h.state().view3d_wait_gpu();
                let s = h.state().view3d_stats().unwrap();
                walls.push(started.elapsed().as_secs_f64() * 1000.0);
                syncs.push(s.last_sync_us as f64 / 1000.0);
                peak = peak.max(s.peak_bytes);
                held = s.other_sets;
                h.run();
            }
            let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
            let min = |v: &[f64]| v.iter().cloned().fold(f64::MAX, f64::min);
            let max = |v: &[f64]| v.iter().cloned().fold(0.0, f64::max);
            println!(
                "{round} 回目・ほかのセットを{}: 替えたフレーム {:.0} ms（{:.0}〜{:.0}）・同期 {:.0} ms・ほかのセット {held} 持つ・同期の途中の最大 {:.0} MiB",
                if show { "見せる" } else { "見せない" },
                mean(&walls),
                min(&walls),
                max(&walls),
                mean(&syncs),
                peak as f64 / (1 << 20) as f64,
            );
        }
    }
}
