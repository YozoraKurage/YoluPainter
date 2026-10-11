//! 3D ビューのグラデーション・図形・定規（egui_kittest。試しの立方体の上で、3D ビューのドラッグの入力の道で）: 画面で引いた形が見えている面だけに
//! 写ること（隠れた面・裏の面は変わらない）・グラデーションの色はテクセルが写る画面の点で決まること・選択範囲・マスク・マテリアルで塗る・正投影・
//! 図形の塗り（縁の帯）と線（3D のストロークのブラシ・対称）・定規（文書の 3D の定規を作る）・取り消し 1 回・ロックと読むだけのセットと予算の断り・
//! Esc とフォーカスの喪失・絵（日英）。面の上の計算は core の `tests/surface/surface_screen.rs` が見る。
use crate::common::*;
use crate::view3d_brush::cube_view;
use egui::{pos2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::Harness;
use yolu_app::drafting::Figure;
use yolu_app::engine::composite_pixel;
use yolu_app::engine::Tilt;
use yolu_app::gradient::GradientOp;
use yolu_app::lang::Lang;
use yolu_app::pen::PenSample;
use yolu_app::state::StrokeSource;
use yolu_app::state::{Action, Tool};
use yolu_app::view3d::draft::{self, DraftKind, SurfaceDraft};
use yolu_app::view3d::model::ViewModel;
use yolu_app::YoluApp;
use yolu_core::geometry::{pick, ModelMesh, OrbitCamera, Submesh, SurfaceHit};
use yolu_core::glam::Vec2;
use yolu_core::glam::{DVec2, DVec3, Vec3};
use yolu_core::material::{GradientSettings, GradientShape};
use yolu_core::{AntiAlias, Channel, LayerLocks, Rgba8, SelectionMask};

type H = Harness<'static, YoluApp>;

const SIZE: u32 = 256;

/// 試しの立方体の面（`demo_cube` と同じ並び）: 4 つの角（UV の (u, v)・(u, v + 高さ)・(u + 幅, v)・(u + 幅, v + 高さ) の点）。
fn corners(face: usize) -> [Vec3; 4] {
    let h = 0.5;
    let v = Vec3::new;
    [
        [v(-h, -h, -h), v(-h, h, -h), v(h, -h, -h), v(h, h, -h)],
        [v(h, -h, h), v(h, h, h), v(-h, -h, h), v(-h, h, h)],
        [v(-h, -h, h), v(-h, h, h), v(-h, -h, -h), v(-h, h, -h)],
        [v(h, -h, -h), v(h, h, -h), v(h, -h, h), v(h, h, h)],
        [v(-h, h, -h), v(-h, h, h), v(h, h, -h), v(h, h, h)],
        [v(-h, -h, h), v(-h, -h, -h), v(h, -h, h), v(h, -h, -h)],
    ][face]
}

/// 面 face のアイランドの中の、縁から margin テクセルより内のテクセルと、その世界の点。
fn texels(face: usize, margin: f32) -> Vec<((u32, u32), Vec3)> {
    let c = corners(face);
    let u0 = (face % 3) as f32 / 3.0 + 0.02;
    let v0 = (face / 3) as f32 * 0.5 + 0.03;
    let (w, hh) = (1.0 / 3.0 - 0.04, 0.44);
    let s = SIZE as f32;
    let mut out = Vec::new();
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (u, v) = ((x as f32 + 0.5) / s, (y as f32 + 0.5) / s);
            let (a, b) = ((u - u0) / w, (v - v0) / hh);
            let (ma, mb) = (margin / (w * s), margin / (hh * s));
            if a < ma || a > 1.0 - ma || b < mb || b > 1.0 - mb {
                continue;
            }
            out.push(((x, y), c[0] + (c[2] - c[0]) * a + (c[1] - c[0]) * b));
        }
    }
    out
}

/// 面 face がカメラから見えるか（表がカメラを向く）。
fn visible(h: &H, face: usize) -> bool {
    let c = corners(face);
    let center = (c[0] + c[3]) * 0.5;
    let normal = (c[1] - c[0]).cross(c[2] - c[0]).normalize();
    let eye = h.state().state.view3d.camera.position();
    // 立方体の外向きの法線（巻きによらず、中心から外へ）
    let out = if normal.dot(center) > 0.0 {
        normal
    } else {
        -normal
    };
    out.dot(eye - center) > 0.0
}

fn st(h: &H) -> &yolu_app::state::AppState {
    &h.state().state
}

fn alpha(h: &H, x: u32, y: u32) -> u8 {
    composite_pixel(&st(h).doc, x, y)[3]
}

fn snapshot(h: &H) -> Vec<u8> {
    let doc = &st(h).doc;
    doc.composite(doc.bounds()).unwrap()
}

fn message(h: &H) -> String {
    st(h).message.clone()
}

/// 世界の点の画面の点（表示域の外の座標系。egui の点）。
fn screen(h: &H, rect: Rect, p: Vec3) -> Pos2 {
    let view = st(h).view3d.camera.view(rect.width(), rect.height());
    let s = view.to_screen(p).expect("カメラの前");
    pos2(rect.left() + s.x, rect.top() + s.y)
}

/// 表示域の画面の点（左上が原点）。
fn local(rect: Rect, p: Pos2) -> DVec2 {
    DVec2::new((p.x - rect.left()) as f64, (p.y - rect.top()) as f64)
}

/// 表示域の画面の点の下の面の点。
fn hit_at(h: &H, rect: Rect, p: Pos2) -> Option<SurfaceHit> {
    let model = st(h).view3d.model.clone()?;
    let view = st(h).view3d.camera.view(rect.width(), rect.height());
    pick(
        &model.geometry,
        &view,
        yolu_core::glam::Vec2::new(p.x - rect.left(), p.y - rect.top()),
    )
}

/// 立方体の画面の箱。
fn cube_box(h: &H, rect: Rect) -> Rect {
    let mut r = Rect::NOTHING;
    for f in 0..6 {
        for c in corners(f) {
            r.extend_with(screen(h, rect, c));
        }
    }
    r
}

/// 3 点で引く（押す・途中・離す）。
fn pull(h: &mut H, a: Pos2, b: Pos2) {
    drag(h, &[a, a + (b - a) * 0.5, b]);
}

fn tool(h: &mut H, t: Tool) {
    h.state_mut().state.apply(Action::SelectTool(t));
    h.run();
}

/// 隠れた面（見えない面）のアイランドの画素が全部透明か。
fn hidden_untouched(h: &H) -> bool {
    (0..6).filter(|&f| !visible(h, f)).all(|f| {
        texels(f, 0.0)
            .iter()
            .all(|&((x, y), _)| alpha(h, x, y) == 0)
    })
}

fn red(h: &mut H) {
    h.state_mut().state.color.set_main([1.0, 0.0, 0.0, 1.0]);
    h.state_mut().state.brush.opacity = 1.0;
}

#[test]
fn a_linear_gradient_colors_each_visible_texel_by_its_screen_point_and_undoes_once() {
    let (mut h, rect) = cube_view();
    red(&mut h);
    tool(&mut h, Tool::Gradient);
    // ブラシの「隠れた所も塗る」「裏の面も塗る」を入れていても、画面の形は見えている面だけ
    h.state_mut().state.view3d.projection.paint_hidden = true;
    h.state_mut().state.view3d.projection.paint_backfaces = true;
    assert!(visible(&h, 0) && visible(&h, 3) && !visible(&h, 1) && !visible(&h, 2));
    let before = snapshot(&h);
    let cube = cube_box(&h, rect);
    let (a, b) = (
        pos2(cube.left() - 10.0, cube.center().y),
        pos2(cube.right() + 10.0, cube.center().y),
    );
    pull(&mut h, a, b);
    assert_eq!(message(&h), "グラデーションを塗りました");
    assert_eq!(st(&h).doc.undo_count(), 1);
    assert!(!st(&h).is_stroking());
    let g = GradientSettings {
        shape: GradientShape::Linear,
        start: local(rect, a),
        end: local(rect, b),
        from: Rgba8::new(255, 0, 0, 255),
        to: Rgba8::TRANSPARENT,
        opacity: 1.0,
    };
    let mut checked = 0;
    for face in [0, 3] {
        for ((x, y), p) in texels(face, 3.0) {
            let s = local(rect, screen(&h, rect, p));
            let want = g.color_at(s.x, s.y).a as i32;
            let got = alpha(&h, x, y) as i32;
            assert!((got - want).abs() <= 2, "面 {face} ({x}, {y}) {got} {want}");
            checked += 1;
        }
    }
    assert!(checked > 5000, "{checked}");
    assert!(hidden_untouched(&h), "隠れた面・裏の面は変わらない");
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert_eq!(snapshot(&h), before, "1 回の取り消しで戻る");
}

#[test]
fn a_radial_gradient_in_orthographic_starts_full_at_the_center() {
    let (mut h, rect) = cube_view();
    red(&mut h);
    h.state_mut().state.view3d.camera.set_orthographic(true);
    tool(&mut h, Tool::Gradient);
    h.state_mut()
        .state
        .apply(Action::Gradient(GradientOp::Shape(GradientShape::Radial)));
    let center = screen(&h, rect, Vec3::new(0.0, 0.0, -0.5));
    let edge = screen(&h, rect, Vec3::new(0.0, 0.45, -0.5));
    pull(&mut h, center, edge);
    assert_eq!(st(&h).doc.undo_count(), 1, "{}", message(&h));
    let g = GradientSettings {
        shape: GradientShape::Radial,
        start: local(rect, center),
        end: local(rect, edge),
        from: Rgba8::new(255, 0, 0, 255),
        to: Rgba8::TRANSPARENT,
        opacity: 1.0,
    };
    let mut full = 0;
    for face in [0, 3] {
        for ((x, y), p) in texels(face, 3.0) {
            let s = local(rect, screen(&h, rect, p));
            let want = g.color_at(s.x, s.y).a as i32;
            let got = alpha(&h, x, y) as i32;
            assert!((got - want).abs() <= 2, "面 {face} ({x}, {y}) {got} {want}");
            if got > 240 {
                full += 1;
            }
        }
    }
    assert!(full > 0);
    assert!(hidden_untouched(&h));
}

#[test]
fn the_gradient_stays_in_the_selection_and_paints_a_mask_or_the_material_set_in_one_undo() {
    // 選択範囲: 文書の左の 40 列だけ（手前の面のアイランドの一部）
    let (mut h, rect) = cube_view();
    red(&mut h);
    tool(&mut h, Tool::Gradient);
    {
        let doc = &mut h.state_mut().state.doc;
        let m = SelectionMask::rectangle(doc, 0, 0, 40, SIZE as i64);
        doc.set_selection(Some(m)).unwrap();
        doc.clear_history().unwrap();
    }
    let cube = cube_box(&h, rect);
    let (a, b) = (
        pos2(cube.left() - 10.0, cube.center().y),
        pos2(cube.right() + 400.0, cube.center().y),
    );
    pull(&mut h, a, b);
    for ((x, y), _) in texels(0, 3.0) {
        assert_eq!(alpha(&h, x, y) > 0, x < 40, "({x}, {y})");
    }
    assert!(hidden_untouched(&h));
    // マスク: レイヤーの画素は変えず、マスクの見える面だけ
    let (mut h, rect) = cube_view();
    tool(&mut h, Tool::Gradient);
    let layer = st(&h).selected_layer.unwrap();
    h.state_mut().state.doc.add_layer_mask(layer).unwrap();
    h.state_mut().state.doc.clear_history().unwrap();
    h.state_mut().state.set_edit_mask(true);
    let mask = |h: &H| {
        st(h)
            .doc
            .layer(layer)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .to_canvas_bytes()
    };
    let before = mask(&h);
    let cube = cube_box(&h, rect);
    pull(
        &mut h,
        pos2(cube.left() - 10.0, cube.center().y),
        pos2(cube.right() + 400.0, cube.center().y),
    );
    let after = mask(&h);
    assert_ne!(before, after, "{}", message(&h));
    let changed = |x: u32, y: u32| {
        let i = ((y * SIZE + x) * 4) as usize;
        before[i..i + 4] != after[i..i + 4]
    };
    assert!(texels(0, 3.0).iter().any(|&((x, y), _)| changed(x, y)));
    for f in (0..6).filter(|&f| !visible(&h, f)) {
        assert!(texels(f, 0.0).iter().all(|&((x, y), _)| !changed(x, y)));
    }
    assert_eq!(st(&h).doc.undo_count(), 1);
    // マテリアルで塗る: 組の全部のチャンネルを 1 回の取り消しで
    let (mut h, rect) = cube_view();
    red(&mut h);
    tool(&mut h, Tool::Gradient);
    {
        let s = &mut h.state_mut().state;
        s.mat.set_enabled(true, Channel::Color);
        s.mat.set_channel(Channel::Roughness, true);
    }
    let cube = cube_box(&h, rect);
    pull(
        &mut h,
        pos2(cube.left() - 10.0, cube.center().y),
        pos2(cube.right() + 400.0, cube.center().y),
    );
    let layer = st(&h).selected_layer.unwrap();
    for channel in [Channel::Color, Channel::Roughness] {
        let bytes = st(&h)
            .doc
            .layer(layer)
            .unwrap()
            .surface(channel)
            .expect("塗った")
            .to_canvas_bytes();
        let ((x, y), _) = texels(0, 3.0)[0];
        assert!(bytes[((y * SIZE + x) * 4 + 3) as usize] > 0, "{channel:?}");
    }
    assert_eq!(st(&h).doc.undo_count(), 1);
}

/// 手前の面の上の画面の長方形（世界の (−0.3, −0.3)〜(0.3, 0.3) の角を画面へ写した点）。
fn front_rect(h: &H, rect: Rect) -> (Pos2, Pos2) {
    (
        screen(h, rect, Vec3::new(-0.3, -0.3, -0.5)),
        screen(h, rect, Vec3::new(0.3, 0.3, -0.5)),
    )
}

/// 手前の面（0 番）のテクセル (x, y) の中の点（中心からテクセルの単位で (ox, oy) ずれた所）の世界の点。
fn front_sub(x: u32, y: u32, ox: f32, oy: f32) -> Vec3 {
    let c = corners(0);
    let (w, hh) = (1.0 / 3.0 - 0.04, 0.44);
    let s = SIZE as f32;
    let (u0, v0) = (0.02, 0.03);
    let a = ((x as f32 + 0.5 + ox) / s - u0) / w;
    let b = ((y as f32 + 0.5 + oy) / s - v0) / hh;
    c[0] + (c[2] - c[0]) * a + (c[1] - c[0]) * b
}

#[test]
fn a_filled_rectangle_counts_four_by_four_points_like_2d_whatever_the_anti_aliasing() {
    const OFFSETS: [f32; 4] = [-0.375, -0.125, 0.125, 0.375];
    for ortho in [false, true] {
        let mut results = Vec::new();
        for anti_alias in [AntiAlias::None, AntiAlias::Strong] {
            let (mut h, rect) = cube_view();
            red(&mut h);
            h.state_mut().state.view3d.camera.set_orthographic(ortho);
            tool(&mut h, Tool::Shape);
            {
                let s = &mut h.state_mut().state;
                s.drafting.figure = Figure::Rectangle;
                s.drafting.fill = true;
                s.drafting.corner = 0.0;
                s.brush.anti_alias = anti_alias;
            }
            let (a, b) = front_rect(&h, rect);
            pull(&mut h, a, b);
            assert_eq!(st(&h).doc.undo_count(), 1, "{}", message(&h));
            let (lo, hi) = (
                local(rect, a).min(local(rect, b)),
                local(rect, a).max(local(rect, b)),
            );
            let (mut full, mut partial) = (0, 0);
            for ((x, y), _) in texels(0, 1.0) {
                // 期待: テクセルを 4 × 4 の点で見て、画面の長方形の中の点の数（2D の選択範囲の形と同じ丸め）。テクセルの写しを
                // 中心の一次で近似するので、透視では縁で 1 点まで違いうる
                let mut count = 0u32;
                for oy in OFFSETS {
                    for ox in OFFSETS {
                        let s = local(rect, screen(&h, rect, front_sub(x, y, ox, oy)));
                        if s.x >= lo.x && s.x <= hi.x && s.y >= lo.y && s.y <= hi.y {
                            count += 1;
                        }
                    }
                }
                let want = ((count * 255 + 8) / 16) as i32;
                let got = alpha(&h, x, y) as i32;
                assert!((got - want).abs() <= 16, "{ortho} ({x}, {y}) {got} {want}");
                if got == 255 {
                    full += 1;
                } else if got > 0 {
                    partial += 1;
                }
            }
            assert!(full > 100 && partial > 10, "{ortho} {full} {partial}");
            assert!(hidden_untouched(&h));
            results.push(snapshot(&h));
            key(&h, Key::Z, Modifiers::COMMAND);
            h.run();
            assert_eq!(st(&h).doc.undo_count(), 0);
            assert!((0..SIZE).all(|y| (0..SIZE).all(|x| alpha(&h, x, y) == 0)));
        }
        assert!(
            results[0] == results[1],
            "{ortho}: ブラシのアンチエイリアスの段は縁を変えない"
        );
    }
}

/// カメラを向いた四角（z の高さ、x・y が −r〜r、UV は u0〜u1 × 0〜1）。
fn plate(z: f32, r: f32, (u0, u1): (f32, f32)) -> ModelMesh {
    let p = |x: f32, y: f32| Vec3::new(x, y, z);
    ModelMesh {
        name: "板".into(),
        positions: vec![p(-r, -r), p(r, -r), p(-r, r), p(r, r)],
        normals: Vec::new(),
        uvs: vec![
            Vec2::new(u0, 0.0),
            Vec2::new(u1, 0.0),
            Vec2::new(u0, 1.0),
            Vec2::new(u1, 1.0),
        ],
        submeshes: vec![Submesh {
            material: 0,
            indices: vec![0, 2, 1, 2, 3, 1],
        }],
    }
}

/// 奥の大きな板（UV の左半分）と、その手前の小さな板（右半分）。どちらも表がカメラを向き、奥の板の真ん中は手前の板に隠れる。
fn two_plates() -> (H, Rect) {
    let mut h = app(1100.0, 760.0, 64);
    click_tab(&mut h, yolu_app::Tab::View3d);
    h.run();
    {
        let state = &mut h.state_mut().state;
        let revision = state.view3d.next_revision();
        state.view3d.material = 0;
        let model = ViewModel::new(
            "試験",
            vec![plate(0.0, 1.0, (0.0, 0.5)), plate(-0.6, 0.35, (0.5, 1.0))],
            vec![Some("試験".to_string())],
            revision,
        )
        .unwrap();
        state.view3d.set_model(model);
        state.view3d.camera = OrbitCamera {
            target: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            distance: 8.0,
            model_radius: 1.0,
            ..OrbitCamera::default()
        };
        // ブラシの「隠れた所も塗る」「裏の面も塗る」を入れておく（画面の形は、それでも見えている面だけ）
        state.view3d.projection.paint_hidden = true;
        state.view3d.projection.paint_backfaces = true;
        state.color.set_main([1.0, 0.0, 0.0, 1.0]);
        state.brush.opacity = 1.0;
    }
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブを描いた");
    (h, rect)
}

#[test]
fn a_front_facing_surface_hidden_behind_another_is_not_painted_by_the_gradient_or_the_fill() {
    for t in [Tool::Gradient, Tool::Shape] {
        let (mut h, rect) = two_plates();
        tool(&mut h, t);
        {
            let s = &mut h.state_mut().state;
            s.drafting.figure = Figure::Rectangle;
            s.drafting.fill = true;
            s.gradient.end = yolu_app::gradient::End::Sub;
            s.color.sub = [0.0, 0.0, 1.0, 1.0];
        }
        // 奥の板の全部を覆うドラッグ
        let (a, b) = (
            screen(&h, rect, Vec3::new(-1.2, 1.2, 0.0)),
            screen(&h, rect, Vec3::new(1.2, -1.2, 0.0)),
        );
        assert!(rect.contains(a) && rect.contains(b), "{rect:?} {a:?} {b:?}");
        pull(&mut h, a, b);
        assert_eq!(st(&h).doc.undo_count(), 1, "{t:?} {}", message(&h));
        // 手前の板の画面の箱
        let (fa, fb) = (
            local(rect, screen(&h, rect, Vec3::new(-0.35, 0.35, -0.6))),
            local(rect, screen(&h, rect, Vec3::new(0.35, -0.35, -0.6))),
        );
        let (lo, hi) = (fa.min(fb), fa.max(fb));
        let (mut hidden, mut seen) = (0, 0);
        for y in 0..64u32 {
            for x in 0..32u32 {
                // 奥の板のテクセルの中心（UV の左半分 → x・y が −1〜1）
                let p = Vec3::new(
                    -1.0 + 2.0 * (x as f32 + 0.5) / 32.0,
                    -1.0 + 2.0 * (y as f32 + 0.5) / 64.0,
                    0.0,
                );
                let s = local(rect, screen(&h, rect, p));
                let inside =
                    |m: f64| s.x > lo.x + m && s.x < hi.x - m && s.y > lo.y + m && s.y < hi.y - m;
                let outside =
                    s.x < lo.x - 3.0 || s.x > hi.x + 3.0 || s.y < lo.y - 3.0 || s.y > hi.y + 3.0;
                if inside(3.0) {
                    assert_eq!(
                        composite_pixel(&st(&h).doc, x, y)[3],
                        0,
                        "{t:?} 隠れた所 ({x}, {y})"
                    );
                    hidden += 1;
                } else if outside && x > 1 && x < 30 && y > 1 && y < 62 {
                    assert!(
                        composite_pixel(&st(&h).doc, x, y)[3] > 0,
                        "{t:?} 見える所 ({x}, {y})"
                    );
                    seen += 1;
                }
            }
        }
        assert!(hidden > 20 && seen > 200, "{t:?} {hidden} {seen}");
        // 手前の板は塗った
        assert!(composite_pixel(&st(&h).doc, 48, 32)[3] > 0, "{t:?}");
    }
}

#[test]
fn a_lost_release_paints_the_gradient_at_the_last_point_and_drops_shapes_and_rulers() {
    for t in [Tool::Gradient, Tool::Shape, Tool::Ruler] {
        let (mut h, rect) = cube_view();
        red(&mut h);
        tool(&mut h, t);
        h.state_mut().state.drafting.fill = true;
        let (a, b) = front_rect(&h, rect);
        let kind = match t {
            Tool::Gradient => DraftKind::Gradient,
            Tool::Shape => DraftKind::Figure,
            _ => DraftKind::Ruler,
        };
        // ボタンは押していないのに、ドラッグの途中だけが残った（ウィンドウの外で離したなど）
        h.state_mut().state.view3d.input.draft = Some(SurfaceDraft {
            kind,
            source: StrokeSource::Mouse,
            start: local(rect, a),
            current: local(rect, b),
            shift: false,
            alt: false,
            hit: hit_at(&h, rect, a).filter(|_| t == Tool::Ruler),
        });
        assert!(draft::dragging(&h.state().state));
        move_to(&h, b);
        h.run();
        assert!(!draft::dragging(&h.state().state), "{t:?}");
        assert_eq!(crate::common::rulers::total(st(&h)), 0, "{t:?}");
        assert_eq!(
            st(&h).doc.undo_count(),
            usize::from(t == Tool::Gradient),
            "{t:?} {}",
            message(&h)
        );
    }
}

#[test]
fn the_outline_is_a_3d_stroke_with_the_brush_and_the_mirror_but_the_fill_is_not_mirrored() {
    let count = |h: &H, half: std::ops::Range<f32>| {
        texels(0, 1.0)
            .iter()
            .filter(|&&((x, y), p)| half.contains(&p.x) && alpha(h, x, y) > 0)
            .count()
    };
    let center_painted = |h: &H| {
        texels(0, 1.0).iter().any(|&((x, y), p)| {
            (p.x.abs() - 0.25).abs() < 0.05 && p.y.abs() < 0.05 && alpha(h, x, y) > 0
        })
    };
    let mut painted = Vec::new();
    for (radius, mirror) in [(1.5, false), (4.0, false), (4.0, true)] {
        let (mut h, rect) = cube_view();
        red(&mut h);
        tool(&mut h, Tool::Shape);
        {
            let s = &mut h.state_mut().state;
            s.drafting.figure = Figure::Rectangle;
            s.drafting.fill = false;
            s.brush.radius = radius;
            if mirror {
                crate::common::rulers::mirror_3d(s, DVec3::ZERO, DVec3::X);
            }
        }
        let base = st(&h).doc.undo_count();
        // 手前の面の右の半分（x が 0.1〜0.4）の長方形
        let a = screen(&h, rect, Vec3::new(0.1, -0.2, -0.5));
        let b = screen(&h, rect, Vec3::new(0.4, 0.2, -0.5));
        pull(&mut h, a, b);
        assert_eq!(st(&h).doc.undo_count(), base + 1, "{}", message(&h));
        assert!(!center_painted(&h), "輪郭だけ");
        let (left, right) = (count(&h, -0.5..0.0), count(&h, 0.0..0.5));
        assert!(right > 20, "{right}");
        assert_eq!(
            left > 20,
            mirror,
            "3D の対称定規（線対称 2 本）の写し {left}"
        );
        painted.push(right);
        assert!(hidden_untouched(&h));
    }
    assert!(painted[1] > painted[0], "ブラシの大きさが効く: {painted:?}");
    // 塗るは 2D と同じく対称を使わない
    let (mut h, rect) = cube_view();
    red(&mut h);
    tool(&mut h, Tool::Shape);
    {
        let s = &mut h.state_mut().state;
        s.drafting.figure = Figure::Ellipse;
        s.drafting.fill = true;
        crate::common::rulers::mirror_3d(s, DVec3::ZERO, DVec3::X);
    }
    let a = screen(&h, rect, Vec3::new(0.1, -0.2, -0.5));
    let b = screen(&h, rect, Vec3::new(0.4, 0.2, -0.5));
    pull(&mut h, a, b);
    assert!(count(&h, 0.0..0.5) > 20);
    assert_eq!(count(&h, -0.5..0.0), 0);
}

#[test]
fn an_outline_over_the_budget_is_cancelled_whole_and_locks_and_read_only_sets_refuse() {
    // 予算: 線（3D のストローク）も塗るもグラデーションも、文書を変えずに断る
    for (t, fill) in [
        (Tool::Shape, false),
        (Tool::Shape, true),
        (Tool::Gradient, false),
    ] {
        let (mut h, rect) = cube_view();
        red(&mut h);
        tool(&mut h, t);
        h.state_mut().state.drafting.figure = Figure::Rectangle;
        h.state_mut().state.drafting.fill = fill;
        h.state_mut()
            .state
            .doc
            .set_stroke_budget_bytes(4096)
            .unwrap();
        let (a, b) = front_rect(&h, rect);
        pull(&mut h, a, b);
        assert!(!message(&h).is_empty(), "{t:?} {fill}");
        assert_eq!(st(&h).doc.undo_count(), 0, "{t:?} {fill}");
        assert!(!st(&h).is_stroking() && !st(&h).doc.has_active_stroke());
        assert!((0..SIZE).all(|y| (0..SIZE).all(|x| alpha(&h, x, y) == 0)));
    }
    // ロック
    for (t, fill) in [
        (Tool::Shape, false),
        (Tool::Shape, true),
        (Tool::Gradient, false),
    ] {
        let (mut h, rect) = cube_view();
        tool(&mut h, t);
        h.state_mut().state.drafting.fill = fill;
        let layer = st(&h).selected_layer.unwrap();
        h.state_mut()
            .state
            .doc
            .set_layer_locks(layer, LayerLocks::ALL)
            .unwrap();
        h.state_mut().state.doc.clear_history().unwrap();
        let (a, b) = front_rect(&h, rect);
        pull(&mut h, a, b);
        assert!(!message(&h).is_empty(), "{t:?} {fill}");
        assert_eq!(st(&h).doc.undo_count(), 0);
        assert!(!st(&h).doc.has_active_stroke());
    }
    // 読むだけのセット: 押したところで断る（ドラッグを始めない）
    for t in [Tool::Shape, Tool::Gradient] {
        let (mut h, rect) = cube_view();
        tool(&mut h, t);
        h.state_mut().state.sets.get_mut(0).unwrap().read_only = Some("読めない中身".into());
        let (a, b) = front_rect(&h, rect);
        press(&h, a, PointerButton::Primary);
        h.step();
        assert!(
            message(&h).contains("読めない中身"),
            "{t:?} {}",
            message(&h)
        );
        assert!(st(&h).view3d.input.draft.is_none());
        move_to(&h, b);
        release(&h, b, PointerButton::Primary);
        h.run();
        assert_eq!(st(&h).doc.undo_count(), 0);
    }
}

#[test]
fn what_the_3d_tools_paint_and_the_3d_ruler_they_make_survive_save_and_reopen() {
    let dir = crate::common::tmp::test_dir("view3d-draft");
    let path = dir.join("draft.ylp");
    let (mut h, rect) = cube_view();
    red(&mut h);
    tool(&mut h, Tool::Gradient);
    let cube = cube_box(&h, rect);
    pull(
        &mut h,
        pos2(cube.left(), cube.top()),
        pos2(cube.right(), cube.bottom()),
    );
    tool(&mut h, Tool::Shape);
    h.state_mut().state.drafting.figure = Figure::Ellipse;
    h.state_mut().state.drafting.fill = true;
    h.state_mut().state.color.set_main([0.0, 0.0, 1.0, 1.0]);
    let (a, b) = front_rect(&h, rect);
    pull(&mut h, a, b);
    assert_eq!(st(&h).doc.undo_count(), 2, "{}", message(&h));
    // 3D の同心円の定規（モデルの空間の値で、文書が持つ）
    tool(&mut h, Tool::Ruler);
    h.state_mut().state.rulers.kind = yolu_core::RulerKind::Concentric;
    pull(&mut h, a, b);
    assert_eq!(st(&h).doc.undo_count(), 3, "{}", message(&h));
    let layer = st(&h).selected_layer.unwrap();
    let placed = crate::common::rulers::of(st(&h), layer);
    assert_eq!(placed.len(), 1);
    let saved = snapshot(&h);
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(path.clone()));
    assert!(message(&h).starts_with("保存しました"), "{}", message(&h));
    h.state_mut().state.apply(Action::OpenProject(path.clone()));
    h.run();
    assert_eq!(snapshot(&h), saved, "{}", message(&h));
    let layer = st(&h).selected_layer.unwrap();
    assert_eq!(
        crate::common::rulers::of(st(&h), layer),
        placed,
        "3D の定規は文書の値として保存される"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// ペンで a から b へ引く（触れる → 動く → 離す。1 点ごとに 1 フレーム）。
fn pen_pull(h: &mut H, a: Pos2, b: Pos2) {
    pen_pull_ending(h, a, b, false);
}

/// ペンで a から b へ引き、最後の離しが本物（`lost` が false）か、OS に押しを奪われて補った離し（true）か。
fn pen_pull_ending(h: &mut H, a: Pos2, b: Pos2, lost: bool) {
    let sample = |at: Pos2, contact: bool, time_ms: u32| PenSample {
        pos: [at.x, at.y],
        pressure: 0.6,
        tilt: Tilt::default(),
        rotation: None,
        contact,
        eraser: false,
        barrel: false,
        pointer_id: 7,
        time_ms,
    };
    for i in 0..=4u32 {
        let at = a + (b - a) * (i as f32 / 4.0);
        h.state().pen().push(sample(at, true, i * 10));
        h.step();
    }
    if lost {
        h.state().pen().push_lost(sample(b, false, 60));
    } else {
        h.state().pen().push(sample(b, false, 60));
    }
    h.step();
    h.run();
}

/// ペンの押しを OS に奪われて離しが補われたら、離した位置が不明なので、グラデーションは最後の位置で塗り（2D のグラデーションの補った離しと同じ）、図形と定規はやめる
/// （マウスの取りこぼしと同じ）。本物の離しは、図形も確定し、定規も置く。次の押しは新しい押しとして始まる。
#[test]
fn a_pen_press_taken_by_the_os_paints_the_gradient_at_the_last_point_and_drops_shapes_and_rulers() {
    for t in [Tool::Gradient, Tool::Shape, Tool::Ruler] {
        let (mut h, rect) = cube_view();
        red(&mut h);
        tool(&mut h, t);
        h.state_mut().state.drafting.figure = Figure::Rectangle;
        h.state_mut().state.drafting.fill = true;
        let (a, b) = front_rect(&h, rect);
        pen_pull_ending(&mut h, a, b, true);
        assert!(!draft::dragging(&h.state().state), "{t:?}");
        assert!(st(&h).view3d.input.pen_press.is_none(), "{t:?}");
        assert_eq!(
            crate::common::rulers::total(st(&h)),
            0,
            "{t:?}: 補った離しでは定規を置かない"
        );
        assert_eq!(
            st(&h).doc.undo_count(),
            usize::from(t == Tool::Gradient),
            "{t:?}: グラデーションだけ最後の位置で塗る {}",
            message(&h)
        );
        // 次の押しは新しい押しとして始まり、本物の離しで決まる（図形も確定し、定規も置く）
        let before = st(&h).doc.undo_count();
        pen_pull(&mut h, a, b);
        assert!(!draft::dragging(&h.state().state), "{t:?}");
        assert_eq!(
            st(&h).doc.undo_count(),
            before + 1,
            "{t:?}: {}",
            message(&h)
        );
        assert_eq!(
            crate::common::rulers::total(st(&h)),
            usize::from(t == Tool::Ruler),
            "{t:?}"
        );
    }
}

#[test]
fn the_pen_draws_the_same_gradient_shape_and_ruler_as_the_mouse() {
    for t in [Tool::Gradient, Tool::Shape, Tool::Ruler] {
        let result = |pen: bool| {
            let (mut h, rect) = cube_view();
            red(&mut h);
            tool(&mut h, t);
            h.state_mut().state.drafting.figure = Figure::Rectangle;
            h.state_mut().state.drafting.fill = true;
            let (a, b) = front_rect(&h, rect);
            if pen {
                pen_pull(&mut h, a, b);
            } else {
                pull(&mut h, a, b);
            }
            assert!(st(&h).view3d.input.draft.is_none(), "{t:?} {pen}");
            let layer = st(&h).selected_layer.unwrap();
            (
                snapshot(&h),
                st(&h).doc.undo_count(),
                crate::common::rulers::of(st(&h), layer)
                    .into_iter()
                    .map(|mut r| {
                        r.id = yolu_core::RulerId(1);
                        r
                    })
                    .collect::<Vec<_>>(),
            )
        };
        let (pen, mouse) = (result(true), result(false));
        assert!(pen == mouse, "{t:?}");
        assert_eq!(pen.1, 1, "{t:?}");
        assert_eq!(pen.2.len(), usize::from(t == Tool::Ruler), "{t:?}");
    }
}

#[test]
fn escape_focus_loss_and_switching_tools_drop_the_drag_without_painting() {
    for way in 0..3 {
        let (mut h, rect) = cube_view();
        tool(
            &mut h,
            if way == 1 {
                Tool::Gradient
            } else {
                Tool::Shape
            },
        );
        h.state_mut().state.drafting.fill = true;
        h.state_mut().state.drafting.figure = Figure::Rectangle;
        let (a, b) = front_rect(&h, rect);
        press(&h, a, PointerButton::Primary);
        h.step();
        move_to(&h, b);
        h.step();
        assert!(st(&h).view3d.input.draft.is_some(), "{way}");
        match way {
            0 => {
                h.event(Event::Key {
                    key: Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                });
            }
            1 => h.event(Event::WindowFocused(false)),
            _ => h.state_mut().state.apply(Action::SelectTool(Tool::Brush)),
        }
        h.step();
        assert!(st(&h).view3d.input.draft.is_none(), "{way}");
        release(&h, b, PointerButton::Primary);
        h.run();
        assert_eq!(st(&h).doc.undo_count(), 0, "{way}");
        assert!(!st(&h).is_stroking());
    }
}

#[test]
fn snapshot_gradient_shapes_and_ruler_in_3d_in_both_languages() {
    let mut snapshots = egui_kittest::SnapshotResults::new();
    for lang in Lang::ALL {
        let (mut h, rect) = cube_view();
        h.state_mut().state.lang = lang;
        h.state_mut().state.color.set_main([0.95, 0.55, 0.1, 1.0]);
        h.state_mut().state.color.sub = [0.2, 0.35, 0.95, 1.0];
        let cube = cube_box(&h, rect);
        // 2 色の線形のグラデーション
        tool(&mut h, Tool::Gradient);
        h.state_mut().state.apply(Action::Gradient(GradientOp::End(
            yolu_app::gradient::End::Sub,
        )));
        pull(
            &mut h,
            pos2(cube.left(), cube.top()),
            pos2(cube.right(), cube.bottom()),
        );
        // 塗った楕円と、線の長方形
        tool(&mut h, Tool::Shape);
        h.state_mut().state.color.set_main([1.0, 1.0, 1.0, 1.0]);
        {
            let s = &mut h.state_mut().state;
            s.drafting.figure = Figure::Ellipse;
            s.drafting.fill = true;
        }
        let (a, b) = (
            screen(&h, rect, Vec3::new(-0.35, -0.1, -0.5)),
            screen(&h, rect, Vec3::new(-0.05, 0.3, -0.5)),
        );
        pull(&mut h, a, b);
        {
            let s = &mut h.state_mut().state;
            s.drafting.figure = Figure::Rectangle;
            s.drafting.fill = false;
            s.brush.radius = 2.0;
        }
        h.state_mut().state.color.set_main([0.1, 0.1, 0.12, 1.0]);
        let (a, b) = (
            screen(&h, rect, Vec3::new(0.5, -0.3, -0.35)),
            screen(&h, rect, Vec3::new(0.5, 0.3, 0.3)),
        );
        pull(&mut h, a, b);
        // 2 点のパースの 3D の定規（定規のツールで引いて作る）
        tool(&mut h, Tool::Ruler);
        h.state_mut().state.rulers.two_points = true;
        h.state_mut().state.rulers.kind = yolu_core::RulerKind::Perspective;
        let (a, b) = (
            screen(&h, rect, Vec3::new(-0.45, 0.3, -0.5)),
            screen(&h, rect, Vec3::new(0.45, 0.3, -0.5)),
        );
        pull(&mut h, a, b);
        h.state_mut().state.message.clear();
        h.event(Event::PointerGone);
        h.run();
        h.snapshot(lang.pick("view3d_draft_ja", "view3d_draft_en"));
        snapshots.extend_harness(&mut h);
    }
}
