//! 3D ビュー（wgpu の自前の描画・カメラの操作・面に描く）の振る舞いと見た目（egui_kittest。描画は wgpu のソフトの描画）。
use crate::common;

use common::*;
use egui::{pos2, vec2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::Harness;
use yolu_app::engine::composite_pixel;
use yolu_app::view3d::model::ViewModel;
use yolu_app::view3d::paint::Slot;
use yolu_app::YoluApp;
use yolu_core::glam::Vec3;

/// 3D のタブを出し、試しの立方体を読み、右（+X）と手前（−Z）の面が見えるカメラにする。
fn cube_view(width: f32, height: f32, doc: u32) -> (Harness<'static, YoluApp>, Rect) {
    let mut h = app(width, height, doc);
    h.state_mut().state.view3d.load_demo();
    h.state_mut().state.view3d.camera.yaw = -40.0;
    h.state_mut().state.view3d.camera.pitch = 15.0;
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブを描いた");
    (h, rect)
}

/// モデルの空間の点が見える画面の点。
fn screen_of(h: &Harness<'_, YoluApp>, rect: Rect, p: Vec3) -> Pos2 {
    let view = h
        .state()
        .state
        .view3d
        .camera
        .view(rect.width(), rect.height());
    let s = view.to_screen(p).expect("カメラの前");
    pos2(rect.left() + s.x, rect.top() + s.y)
}

/// 立方体の UV アイランド（3 × 2）。文書の画素 → (列, 行)。
fn island(x: u32, y: u32, size: u32) -> (u32, u32) {
    (x * 3 / size, y * 2 / size)
}

fn painted_islands(h: &Harness<'_, YoluApp>) -> std::collections::BTreeSet<(u32, u32)> {
    let doc = &h.state().state.doc;
    let size = doc.width();
    let mut out = std::collections::BTreeSet::new();
    for y in 0..size {
        for x in 0..size {
            if composite_pixel(doc, x, y)[3] > 0 {
                out.insert(island(x, y, size));
            }
        }
    }
    out
}

fn pixel(image: &image::RgbaImage, p: Pos2) -> [u8; 4] {
    image.get_pixel(p.x.round() as u32, p.y.round() as u32).0
}

#[test]
fn painting_on_the_cube_crosses_the_seam_and_uploads_only_changed_tiles() {
    let (mut h, rect) = cube_view(1100.0, 760.0, 256); // 2 × 2 タイル
    h.state_mut().state.color.set_main([0.85, 0.15, 0.1, 1.0]);
    // 入力の点を曲線で結ぶブラシ（撮った絵の線は曲線。3D の線もブラシの「曲線」に従う）
    h.state_mut().state.m2.brush.assist.curve = true;
    let stats = h.state().view3d_stats().expect("wgpu の 3D");
    assert!(
        stats.renders >= 1 && stats.total_tiles == 0,
        "空の文書は上げるタイルが無い: {stats:?}"
    );
    let before = stats.total_tiles;
    // 手前の面から右の面へ、縁をまたいで描く
    let from = screen_of(&h, rect, Vec3::new(0.15, 0.1, -0.5));
    let mid = screen_of(&h, rect, Vec3::new(0.5, 0.1, -0.5));
    let to = screen_of(&h, rect, Vec3::new(0.5, 0.05, -0.15));
    let points: Vec<Pos2> = (0..=12)
        .map(|i| {
            let t = i as f32 / 12.0;
            if t < 0.5 {
                from + (mid - from) * (t * 2.0)
            } else {
                mid + (to - mid) * ((t - 0.5) * 2.0)
            }
        })
        .collect();
    drag(&mut h, &points);
    assert_eq!(
        painted_islands(&h),
        [(0, 0), (0, 1)].into_iter().collect(),
        "手前の面（アイランド 0,0）と右の面（アイランド 0,1）。見えない面は塗らない"
    );
    assert!(h.state().state.doc.can_undo());
    assert!(
        h.state().state.message.is_empty(),
        "{}",
        h.state().state.message
    );
    let stats = h.state().view3d_stats().unwrap();
    assert!(
        !stats.last_rebuilt && stats.total_tiles > before,
        "{stats:?}"
    );
    // 描いた所は赤く、描いていない面は灰色、背景は Unity 版の 3D ビューの色
    let image = h.render().expect("描ける");
    let painted = pixel(&image, from);
    assert!(painted[0] > 120 && painted[1] < 80, "描いた所: {painted:?}");
    let plain = pixel(&image, screen_of(&h, rect, Vec3::new(-0.3, -0.3, -0.5)));
    assert!(
        plain[0].abs_diff(plain[1]) < 6 && plain[0] > 40,
        "描いていない面: {plain:?}"
    );
    let corner = pixel(&image, rect.min + vec2(4.0, 4.0));
    assert_eq!(&corner[0..3], &[31, 33, 38], "背景");
    // 上の面（+Y）も見えている（裏の面を描いていない = 表の向きが合っている）
    let top = pixel(&image, screen_of(&h, rect, Vec3::new(0.0, 0.5, 0.0)));
    assert!(top[0] > 40, "上の面: {top:?}");
    h.snapshot("view3d_cube_painted");
    // 1 回の Undo で両方の面が戻る
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert!(painted_islands(&h).is_empty());
}

/// 正投影でも、アプリの入力の道（ドラッグ）から同じように描ける: 手前の面から右の面へ縁をまたいで描くと、その 2 つのアイランドだけに
/// 塗り、1 回の取り消しで戻る。ブラシの円の大きさは奥行きによらない。
#[test]
fn painting_on_the_cube_in_orthographic_crosses_the_seam_and_undoes_in_one_step() {
    let (mut h, rect) = cube_view(1100.0, 760.0, 256);
    h.state_mut().state.view3d.camera.set_orthographic(true);
    h.run();
    h.state_mut().state.color.set_main([0.85, 0.15, 0.1, 1.0]);
    let from = screen_of(&h, rect, Vec3::new(0.15, 0.1, -0.5));
    let mid = screen_of(&h, rect, Vec3::new(0.5, 0.1, -0.5));
    let to = screen_of(&h, rect, Vec3::new(0.5, 0.05, -0.15));
    let points: Vec<Pos2> = (0..=12)
        .map(|i| {
            let t = i as f32 / 12.0;
            if t < 0.5 {
                from + (mid - from) * (t * 2.0)
            } else {
                mid + (to - mid) * ((t - 0.5) * 2.0)
            }
        })
        .collect();
    drag(&mut h, &points);
    assert!(h.state().state.view3d.camera.is_orthographic());
    assert_eq!(
        painted_islands(&h),
        [(0, 0), (0, 1)].into_iter().collect(),
        "手前の面（アイランド 0,0）と右の面（アイランド 0,1）。見えない面は塗らない"
    );
    assert!(
        h.state().state.message.is_empty(),
        "{}",
        h.state().state.message
    );
    let image = h.render().expect("描ける");
    let painted = pixel(&image, from);
    assert!(painted[0] > 120 && painted[1] < 80, "描いた所: {painted:?}");
    // ブラシの円の画面の大きさは、手前の面の点と奥の面の点で同じ
    let view = h
        .state()
        .state
        .view3d
        .camera
        .view(rect.width(), rect.height());
    let near = view.world_radius_to_screen(Vec3::new(0.0, 0.0, -0.5), 0.1);
    let far = view.world_radius_to_screen(Vec3::new(0.0, 0.0, 0.5), 0.1);
    assert!((near - far).abs() < 1e-4, "{near} {far}");
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert!(painted_islands(&h).is_empty());
}

/// `open_3d_over_a_blurred_corner` の文書の一辺。
const BLURRED_CORNER_DOC: u32 = 256;

/// 一部の画素だけを置いたレイヤーにぼかしを掛け、試しの立方体を読んで 3D のタブを開く（`seams` は文書の「UV の継ぎ目をまたぐ」）。
/// 開いたあとの 3D へ上がった絵（Color のアルファ）を、2D の合成と画素ごとに突き合わせ、元の画素のあるタイルの外に出力が上がった数と、
/// 上がったアルファ（行は文書と同じ並び、一辺は文書の大きさ）を返す。
fn open_3d_over_a_blurred_corner(seams: bool, source: &[(u32, u32)]) -> (usize, Vec<u8>) {
    use yolu_core::{Channel, EffectSettings, FilterSpec, FilterTarget, Rgba8};
    let mut h = app(1100.0, 760.0, BLURRED_CORNER_DOC);
    let ts = h.state().state.doc.tile_size();
    let layer = h.state().state.selected_layer.unwrap();
    {
        let doc = &mut h.state_mut().state.doc;
        doc.set_filter_seams(seams).unwrap();
        // 元の画素は 1 つのタイル（0, 0）の中だけ。ぼかしは隣のタイルまで届く
        for &(x, y) in source {
            assert!(x < ts && y < ts, "元の画素は 1 つのタイルの中");
            doc.set_channel_pixel(layer, Channel::Color, x, y, Rgba8::new(240, 30, 60, 255))
                .unwrap();
        }
        doc.add_filter(
            layer,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(ts / 2)).channels(&[Channel::Color]),
        )
        .unwrap();
    }
    let doc = &h.state().state.doc;
    let (w, hh) = (doc.width(), doc.height());
    assert_eq!(
        doc.layer(layer)
            .unwrap()
            .surface(Channel::Color)
            .unwrap()
            .tile_count(),
        1,
        "元の画素があるタイルは 1 つ"
    );
    // モデルを読む前（2D の上）の合成で、ぼかしの出力は隣のタイルにも出ている
    let reached: Vec<_> = doc
        .canvas_tiles()
        .filter(|c| {
            let r = doc.tile_rect(*c).unwrap();
            (r.y..r.y + r.height)
                .any(|y| (r.x..r.x + r.width).any(|x| composite_pixel(doc, x, y)[3] > 0))
        })
        .collect();
    assert!(
        reached.len() > 1 && w > ts && hh > ts,
        "ぼかしの出力は隣のタイルにもある: {reached:?}"
    );
    let all = doc.canvas_tiles().count();
    // 上がった絵を合成と画素ごとに比べるので、UV の外への塗り広げは切る（塗り広げは view3d_padding の試験が見る）
    h.state_mut().view3d_set_display_padding(0);
    h.state_mut().state.view3d.load_demo();
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    let doc = &h.state().state.doc;
    assert_eq!(
        doc.seams_active(),
        seams,
        "モデルを読んだあとの継ぎ目の設定"
    );
    let stats = h.state().view3d_stats().expect("wgpu の 3D");
    assert_eq!(
        stats.total_slot_tiles[Slot::Color.index()],
        all,
        "効果のあるチャンネルは全タイルを合成する（元の画素のあるタイルだけでは足りない）: {stats:?}"
    );
    // 上がった絵は、2D の表示と同じ（元の画素の無いタイルへ広がった分も）。アルファで照らす
    let (bytes, size) = h
        .state()
        .view3d_read_paint_level(Slot::Color, 0)
        .expect("Color を使っている");
    assert_eq!(size, [w, hh], "縮めていない");
    let mut beyond = 0;
    let mut alphas = Vec::with_capacity((w * hh) as usize);
    for y in 0..hh {
        for x in 0..w {
            let alpha = bytes[((y * w + x) * 4 + 3) as usize];
            assert_eq!(alpha, composite_pixel(doc, x, y)[3], "({x}, {y})");
            if alpha > 0 && (x >= ts || y >= ts) {
                beyond += 1;
            }
            alphas.push(alpha);
        }
    }
    (beyond, alphas)
}

/// 立方体の UV（アイランド 3 × 2）で、下の段と上の段のアイランドのすきま（v 0.47〜0.53。アイランドの外）にあたる 4 × 4 画素（タイル（0, 0）の右上の隅）。
fn corner_outside_the_islands() -> Vec<(u32, u32)> {
    (124..128)
        .flat_map(|y| (124..128).map(move |x| (x, y)))
        .collect()
}

/// 立方体のアイランドの中（左から 2 つ目・下の段）の 4 × 4 画素。タイル（0, 0）の右の端で、同じアイランドはタイル（1, 0）へ続く。
fn corner_inside_an_island() -> Vec<(u32, u32)> {
    (60..64)
        .flat_map(|y| (124..128).map(move |x| (x, y)))
        .collect()
}

/// 3D のタブを開いて絵を作り直すとき、効果の出力が元の画素の無いタイルへ広がった分も上げる（2D の表示と同じ見た目）:
/// ぼかしが隣のタイルへ広げた分は、元の画素のあるタイルだけを上げると欠ける。
/// 継ぎ目をまたぐ設定（既定は入。モデルがあるときだけ効く）では、アイランドの外のテクセルは段の入力のままでぼかしが広がらないので、
/// ここでは切って、ぼかしが 2D の上で隣のタイルへ広がる形で確かめる（継ぎ目をまたぐ側は次の試験）。
#[test]
fn opening_the_3d_view_uploads_tiles_that_an_effect_reaches_beyond_the_source_pixels() {
    let (beyond, _) = open_3d_over_a_blurred_corner(false, &corner_outside_the_islands());
    assert!(beyond > 0, "元の画素のあるタイルの外にも出力が上がっている");
}

/// 継ぎ目をまたぐ設定が入のとき、3D のタブを開いた作り直しで上がる絵も、2D の合成と同じ。
/// アイランドの外の画素（すきま）はぼかしが広がらず元のまま、アイランドの中の画素のぼかしは同じアイランドの隣のタイルへ広がる。
#[test]
fn opening_the_3d_view_uploads_the_same_picture_as_the_composite_across_uv_seams() {
    let mut source = corner_outside_the_islands();
    source.extend(corner_inside_an_island());
    let (beyond, alphas) = open_3d_over_a_blurred_corner(true, &source);
    assert!(
        beyond > 0,
        "アイランドの中の画素のぼかしは隣のタイルへ広がる"
    );
    let at = |x: u32, y: u32| alphas[(y * BLURRED_CORNER_DOC + x) as usize];
    for (x, y) in corner_outside_the_islands() {
        assert_eq!(at(x, y), 255, "アイランドの外の画素は元のまま ({x}, {y})");
    }
    // アイランドの外の画素のぼかしは広がらない（2D の上なら、すきまの 24 画素先も染まる）
    assert_eq!(at(100, 127), 0, "アイランドの外のすきまは段の入力のまま");
}

/// 塗りつぶしレイヤー（元の画素が無く、キャンバス全体に出る）も、3D のタブを開いた作り直しでキャンバス全体が上がる。
#[test]
fn opening_the_3d_view_uploads_the_whole_canvas_of_a_fill_layer() {
    use yolu_core::{Channel, Rgba8};
    let mut h = app(1100.0, 760.0, 256);
    h.state_mut()
        .state
        .doc
        .add_fill_layer(
            "塗り",
            &[(Channel::Color, Rgba8::new(10, 200, 30, 255))],
            None,
        )
        .unwrap();
    let tiles = h.state().state.doc.canvas_tiles().count();
    h.state_mut().state.view3d.load_demo();
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    assert_eq!(h.state().view3d_stats().unwrap().total_tiles, tiles);
}

#[test]
fn a_dab_uploads_only_the_touched_tiles() {
    let (mut h, rect) = cube_view(1000.0, 700.0, 1024); // 8 × 8 = 64 タイル
    let before = h.state().view3d_stats().unwrap();
    let at = screen_of(&h, rect, Vec3::new(0.1, 0.1, -0.5));
    click(&mut h, at);
    assert!(h.state().state.doc.can_undo());
    let after = h.state().view3d_stats().unwrap();
    let uploaded = after.total_tiles - before.total_tiles;
    assert!(!after.last_rebuilt);
    assert!(
        (1..=4).contains(&uploaded),
        "変わったタイルだけを上げる: {uploaded}"
    );
    assert!(after.renders > before.renders, "絵が変われば描き直す");
    // 何も変わらなければ描き直さない
    h.run();
    let idle = h.state().view3d_stats().unwrap();
    h.run();
    assert_eq!(h.state().view3d_stats().unwrap().renders, idle.renders);
}

#[test]
fn camera_orbits_pans_and_zooms_like_the_unity_view() {
    let (mut h, rect) = cube_view(1000.0, 700.0, 256);
    let c = rect.center();
    let cam = h.state().state.view3d.camera;
    // 右ドラッグで回す（右へ 100 点で yaw +35°）
    press(&h, c, PointerButton::Secondary);
    h.step();
    move_to(&h, c + vec2(100.0, 20.0));
    h.step();
    release(&h, c + vec2(100.0, 20.0), PointerButton::Secondary);
    h.run();
    let after = h.state().state.view3d.camera;
    assert!(
        (after.yaw - cam.yaw - 35.0).abs() < 1e-3,
        "{} → {}",
        cam.yaw,
        after.yaw
    );
    assert!((after.pitch - cam.pitch - 7.0).abs() < 1e-3);
    assert!(!h.state().state.doc.can_undo(), "回しても描かない");
    // 中ドラッグでパン
    press(&h, c, PointerButton::Middle);
    h.step();
    move_to(&h, c + vec2(-50.0, 0.0));
    h.step();
    release(&h, c + vec2(-50.0, 0.0), PointerButton::Middle);
    h.run();
    assert_ne!(h.state().state.view3d.camera.target, after.target);
    // Alt + 左ドラッグでも回す（描かない）
    let yaw = h.state().state.view3d.camera.yaw;
    h.event(Event::PointerMoved(c));
    h.event(Event::PointerButton {
        pos: c,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::ALT,
    });
    h.step();
    move_to(&h, c + vec2(40.0, 0.0));
    h.step();
    release(&h, c + vec2(40.0, 0.0), PointerButton::Primary);
    h.run();
    assert!((h.state().state.view3d.camera.yaw - yaw - 14.0).abs() < 1e-3);
    assert!(!h.state().state.doc.can_undo());
    // ホイールで寄る（上へ 1 目盛りで約 1/1.2 の距離）
    let distance = h.state().state.view3d.camera.distance;
    move_to(&h, c);
    h.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, 1.0),
        modifiers: Modifiers::NONE,
        phase: egui::TouchPhase::Move,
    });
    h.run();
    let d = h.state().state.view3d.camera.distance;
    assert!(
        (d / distance - (-0.18f32).exp()).abs() < 1e-4,
        "{distance} → {d}"
    );
    // 枠のボタンで全体が見える位置へ戻る
    use egui_kittest::kittest::Queryable;
    h.get_by_label("モデル全体が見える位置へ戻す").click();
    h.run();
    // 全体を表示する位置は、モデルを開いた直後と同じ向き（モデルの前、+Z 側から）
    let camera = h.state().state.view3d.camera;
    assert_eq!(camera.yaw, yolu_core::geometry::DEFAULT_YAW);
    assert_eq!(camera.pitch, yolu_core::geometry::DEFAULT_PITCH);
    assert!(camera.position().z > camera.target.z, "前から見る");
}

#[test]
fn a_model_opens_seen_from_its_front() {
    // モデルは +Z を向く。開いた直後（カメラを触らない）に、中央のレイが当たる面は +Z の面で、
    // ほかのモデルへ替えても・全体を表示しても同じ向きになる
    let mut h = app(1000.0, 700.0, 256);
    h.state_mut().state.view3d.load_demo();
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブを描いた");
    let opened = h.state().state.view3d.camera;
    let model = h
        .state()
        .state
        .view3d
        .model
        .clone()
        .expect("モデルを読んだ");
    assert_eq!(
        opened,
        yolu_core::geometry::OrbitCamera::framing(&model.geometry.bounds())
    );
    let view = opened.view(rect.width(), rect.height());
    let center = yolu_core::glam::Vec2::new(rect.width() * 0.5, rect.height() * 0.5);
    let hit =
        yolu_core::geometry::pick(&model.geometry, &view, center).expect("中央にモデルがある");
    assert!(hit.normal.z > 0.9, "前（+Z）の面が見える: {:?}", hit.normal);
    // 手で回してから全体を表示すると、開いた直後の位置へ戻る
    h.state_mut().state.view3d.camera.yaw = 100.0;
    h.state_mut().state.view3d.frame_model();
    assert_eq!(h.state().state.view3d.camera, opened);
    // モデルを替えても（別の名前）同じ向き
    h.state_mut().state.view3d.camera.yaw = 100.0;
    let revision = h.state_mut().state.view3d.next_revision();
    let other = ViewModel::new(
        "別のモデル",
        vec![yolu_core::geometry::cube_sphere(3, 0.5)],
        vec![Some("別のモデル".to_string())],
        revision,
    )
    .unwrap();
    h.state_mut().state.view3d.set_model(other);
    h.run();
    let replaced = h.state().state.view3d.camera;
    assert_eq!((replaced.yaw, replaced.pitch), (opened.yaw, opened.pitch));
    assert!(replaced.position().z > replaced.target.z, "前から見る");
}

#[test]
fn escape_cancels_and_focus_loss_commits_the_surface_stroke() {
    let (mut h, rect) = cube_view(1000.0, 700.0, 256);
    let a = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
    let b = screen_of(&h, rect, Vec3::new(0.3, 0.0, -0.5));
    press(&h, a, PointerButton::Primary);
    h.step();
    move_to(&h, b);
    h.step();
    assert!(h.state().state.is_stroking());
    // 描いている間はホイールでカメラを動かさない
    let distance = h.state().state.view3d.camera.distance;
    h.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, 3.0),
        modifiers: Modifiers::NONE,
        phase: egui::TouchPhase::Move,
    });
    h.step();
    assert_eq!(h.state().state.view3d.camera.distance, distance);
    key(&h, Key::Escape, Modifiers::NONE);
    h.step();
    assert!(!h.state().state.is_stroking());
    assert!(painted_islands(&h).is_empty(), "Esc は捨てる");
    assert!(!h.state().state.doc.can_undo());
    release(&h, b, PointerButton::Primary);
    h.run();
    // 描いている途中でフォーカスを失うと、そこまでを確定する
    press(&h, a, PointerButton::Primary);
    h.step();
    move_to(&h, b);
    h.step();
    h.event(Event::WindowFocused(false));
    h.step();
    assert!(!h.state().state.is_stroking());
    assert!(h.state().state.doc.can_undo());
    assert_eq!(painted_islands(&h), [(0, 0)].into_iter().collect());
    release(&h, b, PointerButton::Primary);
    h.run();
}

fn two_material_model() -> yolu_protocol::Model {
    use yolu_protocol::{MaterialInfo, MaterialKey, MeshData, Model, Submesh};
    let cube = yolu_core::geometry::demo_cube();
    let indices = &cube.submeshes[0].indices;
    let info = |name: &str| MaterialInfo {
        key: MaterialKey::Material {
            name: name.into(),
            asset: None,
        },
        shader: String::new(),
        textures: Vec::new(),
        routes: Vec::new(),
    };
    Model {
        generation: 1,
        name: "二色の立方体".into(),
        materials: vec![info("肌"), info("服")],
        meshes: vec![MeshData {
            key: "0".into(),
            name: "立方体".into(),
            skinned: false,
            positions: cube.positions.iter().map(|p| p.to_array()).collect(),
            normals: Vec::new(),
            uv0: cube.uvs.iter().map(|u| u.to_array()).collect(),
            // 手前の面（三角形 0・1）は肌、ほかは服
            submeshes: vec![
                Submesh {
                    material: 0,
                    indices: indices[..6].to_vec(),
                },
                Submesh {
                    material: 1,
                    indices: indices[6..].to_vec(),
                },
            ],
        }],
    }
}

/// マテリアルの無いスロットの面に描こうとしたときの知らせの名前も、表示の言語で（日本語の名前を英語の文に埋めない）。
#[test]
fn painting_a_face_without_a_material_names_the_slot_in_the_selected_language() {
    use yolu_app::lang::Lang;
    for lang in Lang::ALL {
        let (mut h, _) = cube_view(1000.0, 700.0, 256);
        h.state_mut().state.lang = lang;
        let mut model = two_material_model();
        model.materials[1].key = yolu_protocol::MaterialKey::Unassigned;
        h.state_mut().load_live_link_model(&model).unwrap();
        h.state_mut().state.view3d.camera.yaw = -40.0;
        h.state_mut().state.view3d.camera.pitch = 15.0;
        h.run();
        let rect = h.state().view3d_rect().unwrap();
        // 右の面はマテリアルの無いスロット。最初のセット（肌）を描いているので、描き始めない
        let side = screen_of(&h, rect, Vec3::new(0.5, 0.0, -0.2));
        click(&mut h, side);
        assert!(!h.state().state.doc.can_undo());
        let message = h.state().state.message.clone();
        match lang {
            Lang::Ja => assert!(message.contains("マテリアルなし"), "{message}"),
            Lang::En => {
                assert!(message.contains("No material"), "{message}");
                assert!(message.chars().all(|c| (c as u32) < 0x3000), "{message}");
            }
        }
    }
}

#[test]
fn live_link_model_paints_only_its_texture_set_and_waits_for_the_stroke() {
    let (mut h, _) = cube_view(1000.0, 700.0, 256);
    h.state_mut()
        .load_live_link_model(&two_material_model())
        .unwrap();
    h.state_mut().state.view3d.camera.yaw = -40.0;
    h.state_mut().state.view3d.camera.pitch = 15.0;
    h.run();
    let rect = h.state().view3d_rect().unwrap();
    // 右の面は服（テクスチャセット 1）。肌を描いているので、描き始めない
    let side = screen_of(&h, rect, Vec3::new(0.5, 0.0, -0.2));
    click(&mut h, side);
    assert!(!h.state().state.doc.can_undo());
    assert!(
        h.state().state.message.contains("服"),
        "{}",
        h.state().state.message
    );
    // 肌の面から服の面へ描いても、肌の面だけ（服の三角形は同じスロットでないので辿らない）
    let front = screen_of(&h, rect, Vec3::new(0.2, 0.0, -0.5));
    press(&h, front, PointerButton::Primary);
    h.step();
    move_to(&h, side);
    h.step();
    // 描いている最中に来たモデルは、ストロークが終わってから入れ替わる
    h.state_mut()
        .load_live_link_model(&two_material_model())
        .unwrap();
    assert!(h.state().state.view3d.has_pending_model());
    // ポーズ（手前の面を 0.25 だけ手前へ）も、待っているモデルに当たる
    let mut model = two_material_model();
    for p in model.meshes[0].positions.iter_mut().take(4) {
        p[2] -= 0.25;
    }
    let pose = yolu_protocol::Pose {
        generation: 1,
        meshes: vec![yolu_protocol::MeshPose {
            mesh: 0,
            positions: model.meshes[0].positions.clone(),
            normals: Vec::new(),
        }],
    };
    h.state_mut().apply_live_link_pose(&pose).unwrap();
    let before = std::sync::Arc::as_ptr(h.state().state.view3d.model.as_ref().unwrap());
    let camera = h.state().state.view3d.camera;
    release(&h, side, PointerButton::Primary);
    h.run();
    assert!(!h.state().state.view3d.has_pending_model());
    let now = h.state().state.view3d.model.clone().unwrap();
    assert_ne!(std::sync::Arc::as_ptr(&now), before);
    assert_eq!(now.geometry.triangles()[0].a.z, -0.75, "ポーズの位置");
    assert_eq!(
        h.state().state.view3d.camera,
        camera,
        "同じモデルのポーズではカメラを動かさない"
    );
    assert_eq!(painted_islands(&h), [(0, 0)].into_iter().collect());
}

fn pen(pos: Pos2, pressure: f32, contact: bool, eraser: bool) -> yolu_app::pen::PenSample {
    yolu_app::pen::PenSample {
        pos: [pos.x, pos.y],
        pressure,
        tilt: yolu_app::engine::Tilt::default(),
        rotation: None,
        contact,
        eraser,
        barrel: false,
        pointer_id: 3,
        time_ms: 0,
    }
}

fn painted_count(h: &Harness<'_, YoluApp>) -> (usize, u8) {
    let doc = &h.state().state.doc;
    let (mut n, mut max) = (0, 0);
    for y in 0..doc.height() {
        for x in 0..doc.width() {
            let a = composite_pixel(doc, x, y)[3];
            if a > 0 {
                n += 1;
                max = max.max(a);
            }
        }
    }
    (n, max)
}

#[test]
fn pen_pressure_scales_the_surface_dab_and_the_eraser_end_erases() {
    let (mut h, rect) = cube_view(1000.0, 700.0, 256);
    let at = screen_of(&h, rect, Vec3::new(0.0, 0.0, -0.5));
    for (pressure, contact) in [(1.0, true), (1.0, false)] {
        h.state().pen().push(pen(at, pressure, contact, false));
    }
    h.run();
    assert!(!h.state().state.is_stroking());
    let (full, full_alpha) = painted_count(&h);
    assert!(full > 50 && full_alpha == 255, "{full} {full_alpha}");
    // 消しゴムの端で同じ所を消す
    for contact in [true, false] {
        h.state().pen().push(pen(at, 1.0, contact, true));
    }
    h.run();
    // 硬さ 0.8 なので、覆いが 1 の内側（面積の 64%）は消え、柔らかい縁は一部だけ消える
    let (left, left_alpha) = painted_count(&h);
    assert!(
        left * 2 < full && left_alpha < 255,
        "消しゴムの端で消える: {left} / {full}、{left_alpha}"
    );
    assert_eq!(h.state().state.doc.undo_count(), 2);
    h.state_mut().state.doc.undo().unwrap();
    h.state_mut().state.doc.undo().unwrap();
    h.run();
    assert_eq!(painted_count(&h).0, 0);
    // 筆圧 0.25: 半径（面の上）も不透明度も 1/4
    for (pressure, contact) in [(0.25, true), (0.25, false)] {
        h.state().pen().push(pen(at, pressure, contact, false));
    }
    h.run();
    let (quarter, alpha) = painted_count(&h);
    assert!(quarter > 0 && quarter * 8 < full, "{quarter} / {full}");
    assert!(alpha <= 65, "不透明度も筆圧で: {alpha}");
}

#[test]
fn a_dab_rim_rounding_below_zero_does_not_cancel_the_stroke() {
    // 2048² の文書・この大きさのウィンドウ・このドラッグでは、ダブの縁のテクセルの覆いが単精度の丸めで −1.2e−7 になる（Unity 版と同じ値）。
    // 0 以下の覆いを apply_pixel に渡すと「値が範囲外」でストロークごと取り消されていた
    let mut h = app(1600.0, 960.0, 2048);
    h.state_mut().state.view3d.load_demo();
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    let mut points = vec![pos2(760.0, 520.0)];
    points.extend([780.0, 800.0, 820.0, 840.0, 860.0, 880.0, 900.0, 920.0].map(|x| pos2(x, 515.0)));
    drag(&mut h, &points);
    assert!(
        h.state().state.message.is_empty(),
        "{}",
        h.state().state.message
    );
    assert!(h.state().state.doc.can_undo());
}

/// 空の 3D ビュー（日英）: 主のボタンは「新規プロジェクト…」で、押すとファイルメニューの新規プロジェクトと同じ要求になる（モデルは読まない）。
/// 試しの立方体はその下の控えめなボタン。ツールチップは名前とキーだけ。
#[test]
fn placeholder_offers_a_new_project_first_and_the_test_cube_below() {
    use egui_kittest::kittest::Queryable;
    use yolu_app::lang::Lang;
    use yolu_app::state::DialogRequest;
    for lang in [Lang::Ja, Lang::En] {
        let mut h = app(1000.0, 700.0, 256);
        h.state_mut().state.lang = lang;
        click_tab(&mut h, yolu_app::Tab::View3d);
        h.run();
        let new_label = lang.pick("新規プロジェクト…", "New Project…");
        let cube_label = lang.pick("試しの立方体を読む", "Load Test Cube");
        let new = h.get_by_label(new_label).rect();
        let cube = h.get_by_label(cube_label).rect();
        let view = h.state().view3d_rect().expect("3D のタブを描いた");
        assert!(
            view.contains_rect(new) && view.contains_rect(cube),
            "{lang:?}"
        );
        assert!(new.bottom() <= cube.top(), "{lang:?}: 新規プロジェクトが上");
        assert!(
            new.width() > cube.width() - 1.0 && new.height() > cube.height(),
            "{lang:?}: 主のボタンの方が大きい"
        );
        // ツールチップは名前とキー（ファイルメニューの新規プロジェクトと同じキー）
        let key = yolu_app::shortcuts::shortcut_text(&yolu_app::state::Action::NewProjectDialog)
            .expect("新規プロジェクトのキー");
        let tip = lang.pick(
            format!("新規プロジェクト（{key}）"),
            format!("New Project ({key})"),
        );
        hover_and_wait(&mut h, new.center());
        assert!(h.query_by_label(&tip).is_some(), "{lang:?}: {tip}");
        move_to(&h, pos2(2.0, 2.0));
        h.run();
        // 押す
        click(&mut h, new.center());
        assert_eq!(
            h.state().state.dialog_request,
            Some(DialogRequest::New),
            "{lang:?}"
        );
        assert!(h.state().state.view3d.model.is_none(), "{lang:?}");
        // 試しの立方体も今どおり読める
        h.state_mut().state.dialog_request = None;
        h.get_by_label(cube_label).click();
        h.run();
        assert!(h.state().state.view3d.model.is_some(), "{lang:?}");
    }
}

/// 保存の間は、メニューの新規プロジェクトと同じく押せず、理由（保存の途中です）が出る。保存が終われば押せる。
#[test]
fn placeholder_new_project_is_unavailable_while_saving() {
    use egui_kittest::kittest::{NodeT, Queryable};
    use yolu_app::lang::Lang;
    use yolu_app::state::DialogRequest;
    for lang in Lang::ALL {
        let mut h = app(1000.0, 700.0, 64);
        h.state_mut().state.lang = lang;
        click_tab(&mut h, yolu_app::Tab::View3d);
        h.run();
        let label = lang.pick("新規プロジェクト…", "New Project…");
        let reason = lang.pick("保存の途中です", "A save is in progress");
        assert!(
            !h.get_by_label(label).accesskit_node().is_disabled(),
            "{lang:?}: 保存していなければ押せる"
        );
        // 保存を 1 つ走らせて止めておく
        let dir = std::env::temp_dir().join(format!(
            "yolu-placeholder-save-{}-{}",
            std::process::id(),
            lang.pick("ja", "en")
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("work.ylp");
        h.state_mut().state.save.background = true;
        let hold = h.state_mut().state.save.hold_next();
        h.state_mut()
            .state
            .apply(yolu_app::state::Action::SaveProjectAs(path));
        h.run();
        assert!(h.state().state.is_saving(), "{lang:?}");
        let node = h.get_by_label(label);
        assert!(
            node.accesskit_node().is_disabled(),
            "{lang:?}: 保存の間は押せない"
        );
        let at = node.rect().center();
        hover_and_wait(&mut h, at);
        assert!(h.query_by_label(reason).is_some(), "{lang:?}: 理由が出る");
        click(&mut h, at);
        assert_ne!(
            h.state().state.dialog_request,
            Some(DialogRequest::New),
            "{lang:?}: 押しても要求は出ない"
        );
        // 保存が終われば押せる
        hold.release();
        for _ in 0..500 {
            if !h.state().state.is_saving() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            h.run();
        }
        assert!(!h.state().state.is_saving(), "{lang:?}");
        move_to(&h, pos2(2.0, 2.0));
        h.run();
        assert!(
            !h.get_by_label(label).accesskit_node().is_disabled(),
            "{lang:?}: 保存が終われば押せる"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 3D ビューの枠が低くても、ボタンは枠の外へ出ない（入りきらないときは試しの立方体を出さず、新規プロジェクトを枠の上端に寄せる）。
#[test]
fn placeholder_buttons_stay_inside_a_low_frame() {
    use egui_kittest::kittest::Queryable;
    for height in [300.0_f32, 360.0, 420.0] {
        let mut h = app(1000.0, height, 64);
        click_tab(&mut h, yolu_app::Tab::View3d);
        h.run();
        let view = h.state().view3d_rect().expect("3D のタブを描いた");
        let new = h.get_by_label("新規プロジェクト…").rect();
        assert!(
            view.contains_rect(new),
            "{height}: {new:?} は {view:?} の外"
        );
        if let Some(cube) = h.query_by_label("試しの立方体を読む") {
            assert!(
                view.contains_rect(cube.rect()),
                "{height}: {:?} は {view:?} の外",
                cube.rect()
            );
        }
    }
}

/// 空の 3D ビューの絵（ボタンの所）。
fn placeholder_snapshot(lang: yolu_app::lang::Lang, name: &str) {
    let mut h = app(1000.0, 700.0, 256);
    h.state_mut().state.lang = lang;
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    move_to(&h, pos2(2.0, 2.0));
    h.run();
    h.snapshot(name);
}

#[test]
fn placeholder_buttons_snapshot_ja() {
    placeholder_snapshot(yolu_app::lang::Lang::Ja, "view3d_placeholder_buttons_ja");
}

#[test]
fn placeholder_buttons_snapshot_en() {
    placeholder_snapshot(yolu_app::lang::Lang::En, "view3d_placeholder_buttons_en");
}

#[test]
fn placeholder_offers_the_demo_cube() {
    let mut h = app(1000.0, 700.0, 256);
    click_tab(&mut h, yolu_app::Tab::View3d);
    use egui_kittest::kittest::Queryable;
    h.get_by_label("試しの立方体を読む").click();
    h.run();
    assert!(h.state().state.view3d.model.is_some());
    assert!(h.state().view3d_stats().unwrap().renders >= 1);
}
