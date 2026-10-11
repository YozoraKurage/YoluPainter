//! 3D ビューの選択範囲のツール（長方形選択・楕円形選択・なげなわ・多角形選択・自動選択。egui_kittest。3D ビューのドラッグの入力の道で）:
//! 画面で引いた形が、見えている面のテクセルだけの選択範囲になること（隠れた面・裏の面は選ばない）・正面から見た板で 2D の同じ形の選択範囲と
//! 同じ量になること（透視・正投影）・新規/追加/削除/共通・アンチエイリアス・縦横比の固定・中心から・角の丸め・自動選択（2D と同じ結果）・
//! Esc とフォーカスの喪失とツールの切り替えと取りこぼした離しと補った離しと確定待ち・ツールのキーの一時の切り替え・取り消し 1 回と保存・
//! 読むだけのセットと予算の断り・絵（日英）。面の上の計算は core の `tests/surface/surface_screen.rs` が見る。
use crate::common::*;
use crate::view3d_brush::{cube_view, press_with, release_with, screen_of};
use egui::{pos2, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::Harness;
use yolu_app::engine::Tilt;
use yolu_app::lang::Lang;
use yolu_app::pen::PenSample;
use yolu_app::selection::{SelAction, SelEdit, SelUiOp};
use yolu_app::state::{Action, StrokeSource, Tool};
use yolu_app::toolkeys::ToolKeyMode;
use yolu_app::view3d::model::ViewModel;
use yolu_app::view3d::select::{self, Drag};
use yolu_app::{Tab, YoluApp};
use yolu_core::geometry::{ModelMesh, OrbitCamera, Submesh};
use yolu_core::glam::{DVec2, Vec2, Vec3};
use yolu_core::{SelectionCombine, SelectionMask};

pub(crate) type H = Harness<'static, YoluApp>;

/// 板の文書の大きさ。
pub(crate) const N: u32 = 64;
/// 立方体の文書の大きさ（`cube_view`）。
const SIZE: u32 = 256;

pub(crate) fn st(h: &H) -> &yolu_app::state::AppState {
    &h.state().state
}

pub(crate) fn message(h: &H) -> String {
    st(h).message.clone()
}

pub(crate) fn tool(h: &mut H, t: Tool) {
    h.state_mut().state.apply(Action::SelectTool(t));
    h.run();
}

// ───────── 場面 ─────────

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

pub(crate) fn model_view(meshes: Vec<ModelMesh>, ortho: bool, distance: f32) -> (H, Rect) {
    let mut h = app(1100.0, 760.0, N);
    click_tab(&mut h, Tab::View3d);
    h.run();
    {
        let state = &mut h.state_mut().state;
        let revision = state.view3d.next_revision();
        state.view3d.material = 0;
        let model =
            ViewModel::new("試験", meshes, vec![Some("試験".to_string())], revision).unwrap();
        state.view3d.set_model(model);
        state.view3d.camera = OrbitCamera {
            target: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            distance,
            model_radius: 1.0,
            ..OrbitCamera::default()
        };
        state.view3d.camera.set_orthographic(ortho);
    }
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブを描いた");
    (h, rect)
}

/// 文書の全部を覆う、カメラを正面から見た板（透視か正投影）。テクセルの画面の位置は、どちらでも一様な拡大と平行移動。板は表示域に収まる
/// （1 テクセルは画面の 8 点ほど）。
pub(crate) fn plate_view(ortho: bool) -> (H, Rect) {
    model_view(vec![plate(0.0, 1.0, (0.0, 1.0))], ortho, 4.5)
}

/// `plate_view` の文書を、幅 `width`・高さ `N` の文書へ入れ替えた画面（板は文書の全部を覆う。3D の選択範囲の重ねが GPU のテクスチャの
/// 辺の上限を超えるかを見るのに使う）。
pub(crate) fn wide_plate_view(width: u32) -> (H, Rect) {
    let (mut h, _) = plate_view(false);
    {
        let state = &mut h.state_mut().state;
        state.doc = yolu_core::Document::new(width, N).unwrap();
        let revision = state.view3d.next_revision();
        let model = ViewModel::new(
            "試験",
            vec![plate(0.0, 1.0, (0.0, 1.0))],
            vec![Some("試験".to_string())],
            revision,
        )
        .unwrap();
        state.view3d.set_model(model);
    }
    h.run();
    let rect = h.state().view3d_rect().expect("3D のタブを描いた");
    (h, rect)
}

/// 板の上の、文書の点（テクセルの単位、左下が原点）の画面の点。
pub(crate) fn at(h: &H, rect: Rect, c: (f64, f64)) -> Pos2 {
    let s0 = screen_of(h, rect, Vec3::new(-1.0, -1.0, 0.0));
    let s1 = screen_of(h, rect, Vec3::new(1.0, 1.0, 0.0));
    pos2(
        s0.x + (s1.x - s0.x) * (c.0 / f64::from(N)) as f32,
        s0.y + (s1.y - s0.y) * (c.1 / f64::from(N)) as f32,
    )
}

// ───────── 入力 ─────────

/// 押して、途中を通って、離す（押しと離しに別の修飾）。
fn pull_with(h: &mut H, a: Pos2, b: Pos2, press: Modifiers, release: Modifiers) {
    if press != Modifiers::NONE {
        h.event(Event::ModifiersChanged(press));
    }
    press_with(h, a, press);
    h.step();
    move_to(h, a + (b - a) * 0.5);
    h.step();
    move_to(h, b);
    h.step();
    if release != press {
        h.event(Event::ModifiersChanged(release));
    }
    release_with(h, b, release);
    h.step();
    if release != Modifiers::NONE {
        h.event(Event::ModifiersChanged(Modifiers::NONE));
    }
    h.run();
}

fn pull(h: &mut H, a: Pos2, b: Pos2) {
    pull_with(h, a, b, Modifiers::NONE, Modifiers::NONE);
}

/// 板の上の文書の点から文書の点へ引く。
fn pull_c(h: &mut H, rect: Rect, a: (f64, f64), b: (f64, f64)) {
    let (a, b) = (at(h, rect, a), at(h, rect, b));
    pull(h, a, b);
}

/// 板の上の文書の点を押して離す。
fn click_c(h: &mut H, rect: Rect, c: (f64, f64)) {
    let p = at(h, rect, c);
    click(h, p);
}

/// `pull_c` の、押しと離しの修飾つき。
fn pull_c_with(
    h: &mut H,
    rect: Rect,
    a: (f64, f64),
    b: (f64, f64),
    press: Modifiers,
    release: Modifiers,
) {
    let (a, b) = (at(h, rect, a), at(h, rect, b));
    pull_with(h, a, b, press, release);
}

/// 点を 1 つずつ押して離す（多角形）。
fn clicks(h: &mut H, points: &[Pos2]) {
    for p in points {
        click(h, *p);
    }
}

fn enter(h: &mut H) {
    key(h, Key::Enter, Modifiers::NONE);
    h.run();
}

fn escape(h: &mut H) {
    key(h, Key::Escape, Modifiers::NONE);
    h.step();
}

// ───────── 見る ─────────

pub(crate) fn selection(h: &H) -> Option<SelectionMask> {
    st(h).doc.selection().cloned()
}

pub(crate) fn amount(h: &H, x: u32, y: u32) -> u8 {
    st(h).doc.selection().map_or(0, |m| m.amount(x, y))
}

/// 板の選択範囲が `want`（文書の全部の画素）と同じか。違う画素の数と、最初の数個を知らせる。
fn assert_same(h: &H, want: &SelectionMask, what: &str) {
    let got = selection(h).unwrap_or_else(|| panic!("{what}: 選択範囲がない: {}", message(h)));
    let mut diff = Vec::new();
    for y in 0..N {
        for x in 0..N {
            if got.amount(x, y) != want.amount(x, y) {
                diff.push((x, y, got.amount(x, y), want.amount(x, y)));
            }
        }
    }
    assert!(
        diff.is_empty(),
        "{what}: 違う画素 {}（x, y, 3D, 2D）{:?}",
        diff.len(),
        &diff[..diff.len().min(6)]
    );
}

pub(crate) fn selected_count(h: &H, size: u32) -> usize {
    (0..size)
        .flat_map(|y| (0..size).map(move |x| (x, y)))
        .filter(|&(x, y)| amount(h, x, y) > 0)
        .count()
}

fn partial_count(h: &H) -> usize {
    (0..N)
        .flat_map(|y| (0..N).map(move |x| (x, y)))
        .filter(|&(x, y)| matches!(amount(h, x, y), 1..=254))
        .count()
}

fn d(c: (f64, f64)) -> DVec2 {
    DVec2::new(c.0, c.1)
}

// ───────── 正面から見た板: 2D の同じ形と同じ量 ─────────

/// 板の上の形（文書の点）。長方形は整数の角（縁は 0 と 255 だけ）、楕円・多角形・なげなわは小数。
const RECT: ((f64, f64), (f64, f64)) = ((10.0, 14.0), (41.0, 50.0));
const ELLIPSE: ((f64, f64), (f64, f64)) = ((9.3, 12.2), (47.9, 51.4));
const OUTLINE: [(f64, f64); 6] = [
    (14.2, 10.7),
    (49.6, 17.3),
    (53.1, 44.9),
    (31.4, 33.2),
    (20.8, 52.6),
    (8.9, 38.1),
];

#[test]
fn the_four_shapes_seen_straight_on_select_the_same_amounts_as_the_2d_shapes_in_perspective_and_orthographic(
) {
    for ortho in [false, true] {
        let label = if ortho { "正投影" } else { "透視" };
        // 長方形（縁は 0 か 255。2D の長方形と同じ）
        let (mut h, rect) = plate_view(ortho);
        tool(&mut h, Tool::SelectRect);
        pull_c(&mut h, rect, RECT.0, RECT.1);
        assert_eq!(st(&h).doc.undo_count(), 1, "{label} {}", message(&h));
        let want = SelectionMask::rectangle(&st(&h).doc, 10, 14, 41, 50);
        assert_same(&h, &want, &format!("{label} 長方形"));
        assert_eq!(partial_count(&h), 0, "{label}: 長方形の縁は 0 か 255");
        // 楕円（縁は滑らか）
        let (mut h, rect) = plate_view(ortho);
        tool(&mut h, Tool::SelectEllipse);
        pull_c(&mut h, rect, ELLIPSE.0, ELLIPSE.1);
        let (a, b) = (d(ELLIPSE.0), d(ELLIPSE.1));
        let want = SelectionMask::ellipse(
            &st(&h).doc,
            (a.x + b.x) / 2.0,
            (a.y + b.y) / 2.0,
            (b.x - a.x) / 2.0,
            (b.y - a.y) / 2.0,
        )
        .unwrap();
        assert_same(&h, &want, &format!("{label} 楕円"));
        assert!(partial_count(&h) > 30, "{label}: 縁が滑らか");
        // なげなわ（押した点・途中の点・離した点を結んだ多角形）
        let (mut h, rect) = plate_view(ortho);
        tool(&mut h, Tool::Lasso);
        let mut path: Vec<Pos2> = OUTLINE.iter().map(|&c| at(&h, rect, c)).collect();
        path.push(path[0]);
        drag(&mut h, &path);
        let points: Vec<DVec2> = OUTLINE
            .iter()
            .map(|&c| d(c))
            .chain([d(OUTLINE[0])])
            .collect();
        let want = SelectionMask::polygon(&st(&h).doc, &points).unwrap();
        assert_same(&h, &want, &format!("{label} なげなわ"));
        assert!(partial_count(&h) > 30, "{label}: 縁が滑らか");
        // 多角形（クリックで点を打ち、始めの点の近くを押して閉じる）
        let (mut h, rect) = plate_view(ortho);
        tool(&mut h, Tool::Polygon);
        let ps: Vec<Pos2> = OUTLINE.iter().map(|&c| at(&h, rect, c)).collect();
        clicks(&mut h, &ps);
        assert_eq!(st(&h).sel.view3d.polygon.len(), OUTLINE.len(), "{label}");
        assert!(selection(&h).is_none(), "閉じるまでは選ばない");
        click(&mut h, ps[0] + egui::vec2(3.0, 2.0));
        assert!(st(&h).sel.view3d.polygon.is_empty(), "{label}: 閉じた");
        let points: Vec<DVec2> = OUTLINE.iter().map(|&c| d(c)).collect();
        let want = SelectionMask::polygon(&st(&h).doc, &points).unwrap();
        assert_same(&h, &want, &format!("{label} 多角形"));
        assert_eq!(st(&h).doc.undo_count(), 1, "{label}: 取り消し 1 回");
    }
}

#[test]
fn a_rectangle_with_fractional_corners_counts_four_by_four_points_like_the_2d_polygon_of_its_corners(
) {
    // 小数の角の長方形は、2D の長方形（角を画素へ丸める）でなく、4 つの角の多角形（1 テクセルを 4 × 4 の点で見る）を、縁の滑らかさの設定のまま
    // 半分以上の点が入るかで 0 か 255 にした量
    let (a, b) = ((10.3, 14.6), (40.7, 49.2));
    for ortho in [false, true] {
        let (mut h, rect) = plate_view(ortho);
        tool(&mut h, Tool::SelectRect);
        pull_c(&mut h, rect, a, b);
        let corners = [d(a), DVec2::new(b.0, a.1), d(b), DVec2::new(a.0, b.1)];
        let want = SelectionMask::polygon(&st(&h).doc, &corners)
            .unwrap()
            .sharpen();
        assert_same(&h, &want, &format!("ortho={ortho}"));
        assert_eq!(partial_count(&h), 0);
    }
}

#[test]
fn turning_anti_alias_off_makes_every_edge_all_or_nothing_like_the_2d_shapes() {
    let (mut h, rect) = plate_view(false);
    h.state_mut().state.sel.antialias = false;
    for (t, n) in [
        (Tool::SelectEllipse, 0),
        (Tool::Lasso, 1),
        (Tool::Polygon, 2),
    ] {
        tool(&mut h, t);
        match n {
            0 => pull_c(&mut h, rect, ELLIPSE.0, ELLIPSE.1),
            1 => {
                let mut path: Vec<Pos2> = OUTLINE.iter().map(|&c| at(&h, rect, c)).collect();
                path.push(path[0]);
                drag(&mut h, &path);
            }
            _ => {
                let ps: Vec<Pos2> = OUTLINE.iter().map(|&c| at(&h, rect, c)).collect();
                clicks(&mut h, &ps);
                enter(&mut h);
            }
        }
        assert!(selection(&h).is_some(), "{t:?} {}", message(&h));
        assert_eq!(partial_count(&h), 0, "{t:?}: 縁は 0 か 255 だけ");
        assert!(selected_count(&h, N) > 200, "{t:?}");
        // 2D と同じ: 縁を滑らかにした量を半分以上で 0 / 255 にしたもの
        let want = match n {
            0 => {
                let (a, b) = (d(ELLIPSE.0), d(ELLIPSE.1));
                SelectionMask::ellipse(
                    &st(&h).doc,
                    (a.x + b.x) / 2.0,
                    (a.y + b.y) / 2.0,
                    (b.x - a.x) / 2.0,
                    (b.y - a.y) / 2.0,
                )
                .unwrap()
                .sharpen()
            }
            1 => {
                let points: Vec<DVec2> = OUTLINE
                    .iter()
                    .map(|&c| d(c))
                    .chain([d(OUTLINE[0])])
                    .collect();
                SelectionMask::polygon(&st(&h).doc, &points)
                    .unwrap()
                    .sharpen()
            }
            _ => {
                let points: Vec<DVec2> = OUTLINE.iter().map(|&c| d(c)).collect();
                SelectionMask::polygon(&st(&h).doc, &points)
                    .unwrap()
                    .sharpen()
            }
        };
        assert_same(&h, &want, &format!("{t:?}"));
    }
}

// ───────── 見えている面だけ ─────────

/// 立方体の面（`demo_cube` と同じ並び）の 4 つの角。
pub(crate) fn corners(face: usize) -> [Vec3; 4] {
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

/// 面 face のアイランドの中の、縁から margin テクセルより内のテクセル。
pub(crate) fn island_texels(face: usize, margin: f32) -> Vec<(u32, u32)> {
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
            out.push((x, y));
        }
    }
    out
}

/// 面 face がカメラから見えるか（表がカメラを向く）。
pub(crate) fn face_visible(h: &H, face: usize) -> bool {
    let c = corners(face);
    let center = (c[0] + c[3]) * 0.5;
    let normal = (c[1] - c[0]).cross(c[2] - c[0]).normalize();
    let eye = st(h).view3d.camera.position();
    let out = if normal.dot(center) > 0.0 {
        normal
    } else {
        -normal
    };
    out.dot(eye - center) > 0.0
}

pub(crate) fn cube_box(h: &H, rect: Rect) -> Rect {
    let mut r = Rect::NOTHING;
    for f in 0..6 {
        for c in corners(f) {
            r.extend_with(screen_of(h, rect, c));
        }
    }
    r
}

#[test]
fn every_shape_selects_only_the_faces_the_camera_sees_in_perspective_and_orthographic() {
    for ortho in [false, true] {
        for t in [
            Tool::SelectRect,
            Tool::SelectEllipse,
            Tool::Lasso,
            Tool::Polygon,
        ] {
            let (mut h, rect) = cube_view();
            // 立方体を小さく見せる（表示域の隅のアイコンに、囲む多角形の点が重ならないように）
            h.state_mut().state.view3d.camera.distance *= 1.5;
            h.state_mut().state.view3d.camera.set_orthographic(ortho);
            h.run();
            // ブラシの「隠れた所も塗る」「裏の面も塗る」を入れていても、選択範囲は見えている面だけ。見える面の量は、面の向きの弱め（カメラにすれすれの
            // 面を薄くする）を切って 255 を見る
            {
                let projection = &mut h.state_mut().state.view3d.projection;
                projection.paint_hidden = true;
                projection.paint_backfaces = true;
                projection.angle_falloff = false;
            }
            tool(&mut h, t);
            let cube = cube_box(&h, rect);
            // 立方体より少し大きい箱（多角形は押す点が表示域の中に要る）
            let big = cube.expand(12.0);
            assert!(rect.contains_rect(big), "{rect:?} {big:?}");
            match t {
                Tool::SelectRect => pull(&mut h, big.left_top(), big.right_bottom()),
                Tool::SelectEllipse => {
                    // 立方体の箱が入る楕円（箱を √2 倍より大きくして外接させる）。中心から広げるので、押すのは箱の真ん中
                    h.state_mut().state.sel.from_center = true;
                    let c = cube.center();
                    pull(&mut h, c, c + cube.size() * 0.5 * 1.5);
                }
                Tool::Lasso => drag(
                    &mut h,
                    &[
                        big.left_top(),
                        big.right_top(),
                        big.right_bottom(),
                        big.left_bottom(),
                        big.left_top() + egui::vec2(0.0, 2.0),
                    ],
                ),
                _ => {
                    clicks(
                        &mut h,
                        &[
                            big.left_top(),
                            big.right_top(),
                            big.right_bottom(),
                            big.left_bottom(),
                        ],
                    );
                    assert_eq!(st(&h).sel.view3d.polygon.len(), 4, "押した点が全部入った");
                    enter(&mut h);
                }
            }
            let label = format!("{t:?} ortho={ortho} {}", message(&h));
            assert!(selection(&h).is_some(), "{label}");
            assert_eq!(st(&h).doc.undo_count(), 1, "{label}");
            let (mut seen_texels, mut hidden_texels) = (0, 0);
            for f in 0..6 {
                let visible = face_visible(&h, f);
                for (x, y) in island_texels(f, 3.0) {
                    if visible {
                        assert_eq!(amount(&h, x, y), 255, "{label}: 面 {f} ({x}, {y})");
                        seen_texels += 1;
                    } else {
                        hidden_texels += 1;
                    }
                }
                if !visible {
                    for (x, y) in island_texels(f, 0.0) {
                        assert_eq!(amount(&h, x, y), 0, "{label}: 見えない面 {f} ({x}, {y})");
                    }
                }
            }
            assert!(seen_texels > 10_000 && hidden_texels > 10_000, "{label}");
        }
    }
}

/// 奥の大きな板（UV の左半分）と、その手前の小さな板（右半分）。奥の板の真ん中は手前の板に隠れる。
pub(crate) fn two_plates() -> (H, Rect) {
    model_view(
        vec![plate(0.0, 1.0, (0.0, 0.5)), plate(-0.6, 0.35, (0.5, 1.0))],
        false,
        8.0,
    )
}

#[test]
fn a_front_facing_surface_hidden_behind_another_is_not_selected() {
    for t in [Tool::SelectRect, Tool::Lasso] {
        let (mut h, rect) = two_plates();
        // ブラシの「隠れた所も塗る」を入れておく（選択範囲は、それでも見えている面だけ）
        h.state_mut().state.view3d.projection.paint_hidden = true;
        h.state_mut().state.view3d.projection.paint_backfaces = true;
        tool(&mut h, t);
        let (a, b) = (
            screen_of(&h, rect, Vec3::new(-1.2, 1.2, 0.0)),
            screen_of(&h, rect, Vec3::new(1.2, -1.2, 0.0)),
        );
        assert!(rect.contains(a) && rect.contains(b));
        if t == Tool::Lasso {
            drag(
                &mut h,
                &[
                    a,
                    pos2(b.x, a.y),
                    b,
                    pos2(a.x, b.y),
                    a + egui::vec2(0.0, 2.0),
                ],
            );
        } else {
            pull(&mut h, a, b);
        }
        assert_eq!(st(&h).doc.undo_count(), 1, "{t:?} {}", message(&h));
        // 手前の板の画面の箱
        let (fa, fb) = (
            screen_of(&h, rect, Vec3::new(-0.35, 0.35, -0.6)),
            screen_of(&h, rect, Vec3::new(0.35, -0.35, -0.6)),
        );
        let front = Rect::from_two_pos(fa, fb);
        let (mut hidden, mut seen) = (0, 0);
        for y in 0..N {
            for x in 0..N / 2 {
                // 奥の板のテクセルの中心（UV の左半分 → x・y が −1〜1）
                let p = Vec3::new(
                    -1.0 + 2.0 * (x as f32 + 0.5) / (N / 2) as f32,
                    -1.0 + 2.0 * (y as f32 + 0.5) / N as f32,
                    0.0,
                );
                let s = screen_of(&h, rect, p);
                if front.shrink(3.0).contains(s) {
                    assert_eq!(amount(&h, x, y), 0, "{t:?} 隠れた所 ({x}, {y})");
                    hidden += 1;
                } else if !front.expand(3.0).contains(s)
                    && x > 1
                    && x < N / 2 - 2
                    && y > 1
                    && y < N - 2
                {
                    assert_eq!(amount(&h, x, y), 255, "{t:?} 見える所 ({x}, {y})");
                    seen += 1;
                }
            }
        }
        assert!(hidden > 20 && seen > 200, "{t:?} {hidden} {seen}");
        // 手前の板は選んだ
        assert_eq!(amount(&h, N * 3 / 4, N / 2), 255, "{t:?}");
    }
}

// ───────── 新規・追加・削除・共通 ─────────

#[test]
fn new_add_subtract_and_intersect_work_by_the_option_and_by_shift_and_ctrl_as_in_2d() {
    let (a1, a2) = ((8.0, 8.0), (40.0, 40.0));
    let (b1, b2) = ((24.0, 24.0), (56.0, 56.0));
    let rect_mask = |h: &H, (x0, y0): (f64, f64), (x1, y1): (f64, f64)| {
        SelectionMask::rectangle(&st(h).doc, x0 as i64, y0 as i64, x1 as i64, y1 as i64)
    };
    let (mut h, rect) = plate_view(false);
    tool(&mut h, Tool::SelectRect);
    let pa = (at(&h, rect, a1), at(&h, rect, a2));
    let pb = (at(&h, rect, b1), at(&h, rect, b2));
    let first = rect_mask(&h, a1, a2);
    let second = rect_mask(&h, b1, b2);
    for (label, mode, press, option) in [
        ("新規", SelectionCombine::Replace, Modifiers::NONE, None),
        (
            "Shift で追加",
            SelectionCombine::Add,
            Modifiers::SHIFT,
            None,
        ),
        (
            "Ctrl で削除",
            SelectionCombine::Subtract,
            Modifiers::COMMAND,
            None,
        ),
        (
            "Shift+Ctrl で共通",
            SelectionCombine::Intersect,
            Modifiers::SHIFT | Modifiers::COMMAND,
            None,
        ),
        (
            "オプションの追加",
            SelectionCombine::Add,
            Modifiers::NONE,
            Some(SelectionCombine::Add),
        ),
        (
            "オプションの削除",
            SelectionCombine::Subtract,
            Modifiers::NONE,
            Some(SelectionCombine::Subtract),
        ),
        (
            "オプションの共通",
            SelectionCombine::Intersect,
            Modifiers::NONE,
            Some(SelectionCombine::Intersect),
        ),
    ] {
        // 最初の長方形を置く
        h.state_mut()
            .state
            .apply(Action::Sel(SelAction::Ui(SelUiOp::Combine(
                SelectionCombine::Replace,
            ))));
        pull(&mut h, pa.0, pa.1);
        assert_same(&h, &first, label);
        let steps = st(&h).doc.undo_count();
        if let Some(option) = option {
            h.state_mut()
                .state
                .apply(Action::Sel(SelAction::Ui(SelUiOp::Combine(option))));
        }
        pull_with(&mut h, pb.0, pb.1, press, Modifiers::NONE);
        let want = first.combine(&second, mode).unwrap();
        assert_same(&h, &want, label);
        assert_eq!(st(&h).doc.undo_count(), steps + 1, "{label}: 取り消し 1 回");
        // 取り消しで最初の長方形へ戻る
        key(&h, Key::Z, Modifiers::COMMAND);
        h.run();
        assert_same(&h, &first, &format!("{label}: 取り消し"));
        // 解除して次へ
        h.state_mut()
            .state
            .apply(Action::Sel(SelAction::Edit(SelEdit::Clear)));
        h.state_mut().state.doc.clear_history().unwrap();
    }
}

#[test]
fn a_click_without_dragging_deselects_only_in_new_mode() {
    let (mut h, rect) = plate_view(false);
    tool(&mut h, Tool::SelectRect);
    pull_c(&mut h, rect, (8.0, 8.0), (40.0, 40.0));
    let kept = selection(&h).unwrap();
    // 追加のオプションや Shift では、動かさないクリックで何も変えない
    click_c(&mut h, rect, (50.0, 50.0));
    assert!(
        selection(&h).is_none(),
        "新規のクリックは解除: {}",
        message(&h)
    );
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert!(selection(&h) == Some(kept.clone()));
    press_with(&h, at(&h, rect, (50.0, 50.0)), Modifiers::SHIFT);
    h.step();
    release_with(&h, at(&h, rect, (50.0, 50.0)), Modifiers::SHIFT);
    h.run();
    assert!(selection(&h) == Some(kept), "Shift のクリックは何もしない");
}

// ───────── 設定: 縦横比・中心から・角の丸め ─────────

#[test]
fn fixed_ratio_from_center_and_the_corner_radius_shape_the_selection_as_in_2d() {
    // 縦横比を固定（設定）: 長い方に合わせた正方形
    let (mut h, rect) = plate_view(false);
    tool(&mut h, Tool::SelectRect);
    h.state_mut().state.sel.fixed_ratio = true;
    pull_c(&mut h, rect, (10.0, 10.0), (50.0, 30.0));
    let want = SelectionMask::rectangle(&st(&h).doc, 10, 10, 50, 50);
    assert_same(&h, &want, "縦横比を固定");
    // 設定が切れていても、押し始めたあとの Shift で固定する
    let (mut h, rect) = plate_view(false);
    tool(&mut h, Tool::SelectEllipse);
    pull_c_with(
        &mut h,
        rect,
        (8.0, 8.0),
        (48.0, 28.0),
        Modifiers::NONE,
        Modifiers::SHIFT,
    );
    let want = SelectionMask::ellipse(&st(&h).doc, 28.0, 28.0, 20.0, 20.0).unwrap();
    assert_same(&h, &want, "押し始めたあとの Shift");
    // 押し始めに押していた Shift は、追加の合図（縦横比は固定しない）
    let (mut h, rect) = plate_view(false);
    tool(&mut h, Tool::SelectRect);
    pull_c_with(
        &mut h,
        rect,
        (10.0, 10.0),
        (50.0, 30.0),
        Modifiers::SHIFT,
        Modifiers::SHIFT,
    );
    let want = SelectionMask::rectangle(&st(&h).doc, 10, 10, 50, 30);
    assert_same(&h, &want, "押し始めの Shift は追加");
    // 押し始めたあとの Alt: 押した点が中心（設定が切れていても）
    let (mut h, rect) = plate_view(false);
    tool(&mut h, Tool::SelectRect);
    pull_c_with(
        &mut h,
        rect,
        (32.0, 32.0),
        (50.0, 42.0),
        Modifiers::NONE,
        Modifiers::ALT,
    );
    let want = SelectionMask::rectangle(&st(&h).doc, 14, 22, 50, 42);
    assert_same(&h, &want, "押し始めたあとの Alt");
    // 中心から（設定）: 押した点が中心
    let (mut h, rect) = plate_view(false);
    tool(&mut h, Tool::SelectRect);
    h.state_mut().state.sel.from_center = true;
    pull_c(&mut h, rect, (32.0, 32.0), (50.0, 42.0));
    let want = SelectionMask::rectangle(&st(&h).doc, 14, 22, 50, 42);
    assert_same(&h, &want, "中心から");
    // 角の丸め: 角のテクセルは外れ、縁は滑らか（アンチエイリアスを切ると 0 か 255 だけ）
    for anti_alias in [true, false] {
        let (mut h, rect) = plate_view(false);
        tool(&mut h, Tool::SelectRect);
        {
            let s = &mut h.state_mut().state.sel;
            s.corner_radius = 100; // 画面の点。板は 1 テクセルが約 9 点なので、10 テクセル強
            s.antialias = anti_alias;
        }
        pull_c(&mut h, rect, (10.0, 10.0), (50.0, 50.0));
        assert_eq!(amount(&h, 10, 10), 0, "{anti_alias}: 丸めた角の外");
        assert_eq!(amount(&h, 30, 30), 255);
        assert_eq!(amount(&h, 30, 10), 255, "{anti_alias}: 直線の縁");
        assert_eq!(partial_count(&h) > 0, anti_alias, "{anti_alias}");
    }
}

// ───────── 自動選択 ─────────

/// 板の文書の、長方形 (x0, y0)-(x1, y1) を描画色で塗って、選択範囲を解除し、履歴を空にする。
pub(crate) fn paint_rect(h: &mut H, (x0, y0, x1, y1): (i64, i64, i64, i64)) {
    let s = &mut h.state_mut().state;
    s.color.set_main([1.0, 0.0, 0.0, 1.0]);
    s.brush.opacity = 1.0;
    let mask = SelectionMask::rectangle(&s.doc, x0, y0, x1, y1);
    s.doc.set_selection(Some(mask)).unwrap();
    s.apply(Action::Sel(SelAction::Edit(SelEdit::Fill)));
    s.apply(Action::Sel(SelAction::Edit(SelEdit::Clear)));
    s.doc.clear_history().unwrap();
}

#[test]
fn the_wand_runs_the_2d_magic_wand_from_the_texel_under_the_click() {
    for ortho in [false, true] {
        let (mut h, rect) = plate_view(ortho);
        paint_rect(&mut h, (10, 12, 30, 40));
        tool(&mut h, Tool::Wand);
        h.state_mut().state.sel.tolerance = 0;
        // 塗った所の上: 塗った長方形だけ
        click_c(&mut h, rect, (20.5, 25.5));
        assert_eq!(st(&h).doc.undo_count(), 1, "{}", message(&h));
        let want = SelectionMask::rectangle(&st(&h).doc, 10, 12, 30, 40);
        assert_same(&h, &want, &format!("ortho={ortho} 塗った所"));
        // 2D と同じ呼び出しの結果
        let s = st(&h);
        let direct = SelectionMask::magic_wand(
            &s.doc,
            s.selected_layer,
            s.m2.paint_channel,
            20,
            25,
            0,
            true,
            yolu_app::engine::DEFAULT_WORKING_BUDGET_BYTES,
        )
        .unwrap();
        assert_same(&h, &direct, "2D の自動選択と同じ");
        // 取り消しで元へ。塗っていない所: それ以外の全部
        key(&h, Key::Z, Modifiers::COMMAND);
        h.run();
        assert!(selection(&h).is_none());
        click_c(&mut h, rect, (50.5, 55.5));
        let blank = SelectionMask::rectangle(&st(&h).doc, 10, 12, 30, 40).invert();
        assert_same(&h, &blank, &format!("ortho={ortho} 塗っていない所"));
    }
}

#[test]
fn the_wand_follows_tolerance_contiguous_and_the_combine_modifiers_and_ignores_clicks_off_the_model(
) {
    let (mut h, rect) = plate_view(false);
    paint_rect(&mut h, (10, 12, 30, 40));
    paint_rect(&mut h, (40, 12, 56, 40));
    tool(&mut h, Tool::Wand);
    h.state_mut().state.sel.tolerance = 0;
    // 隣接: 押した長方形だけ。隣接を切ると、離れた同じ色の長方形も
    click_c(&mut h, rect, (20.5, 25.5));
    assert_eq!(selected_count(&h, N), 20 * 28);
    h.state_mut().state.sel.contiguous = false;
    click_c(&mut h, rect, (20.5, 25.5));
    assert_eq!(selected_count(&h, N), 20 * 28 + 16 * 28, "隣接しない");
    h.state_mut().state.sel.contiguous = true;
    // Shift で追加（もう一方の長方形を選択範囲へ追加）
    click_c(&mut h, rect, (20.5, 25.5));
    press_with(&h, at(&h, rect, (48.5, 25.5)), Modifiers::SHIFT);
    h.step();
    release_with(&h, at(&h, rect, (48.5, 25.5)), Modifiers::SHIFT);
    h.run();
    assert_eq!(selected_count(&h, N), 20 * 28 + 16 * 28, "追加");
    // Ctrl で削除
    press_with(&h, at(&h, rect, (20.5, 25.5)), Modifiers::COMMAND);
    h.step();
    release_with(&h, at(&h, rect, (20.5, 25.5)), Modifiers::COMMAND);
    h.run();
    assert_eq!(selected_count(&h, N), 16 * 28, "削除");
    // モデルの外（板の外の背景）: 何も変えない
    let before = selection(&h);
    let steps = st(&h).doc.undo_count();
    let off = rect.left_top() + egui::vec2(4.0, 4.0);
    click(&mut h, off);
    assert!(selection(&h) == before, "{}", message(&h));
    assert_eq!(st(&h).doc.undo_count(), steps);
    assert!(!message(&h).is_empty(), "理由を知らせる");
}

/// 自動選択の対称定規の写しは 2D のキャンバスだけ。3D ビューでは、対称定規があっても押した面のテクセルだけから選ぶ。
#[test]
fn the_wand_in_3d_ignores_the_symmetry_ruler() {
    let (mut h, rect) = plate_view(false);
    paint_rect(&mut h, (10, 12, 30, 40));
    paint_rect(&mut h, (34, 12, 54, 40)); // 縦の軸 x = 32 に対する鏡の側
    rulers::vertical(&mut h.state_mut().state, 32.0);
    h.run();
    assert!(st(&h).canvas_symmetry().enabled());
    tool(&mut h, Tool::Wand);
    h.state_mut().state.sel.tolerance = 0;
    let steps = st(&h).doc.undo_count();
    click_c(&mut h, rect, (20.5, 25.5));
    assert_eq!(st(&h).doc.undo_count(), steps + 1, "{}", message(&h));
    assert_eq!(
        selected_count(&h, N),
        20 * 28,
        "鏡の側の長方形は選ばない: {}",
        message(&h)
    );
}

// ───────── 継ぎ目のにじみ・面の向き ─────────

#[test]
fn the_seam_bleed_of_the_3d_group_reaches_past_the_island_edge_and_zero_does_not() {
    // UV の左半分だけの板: アイランドの右の縁は x = 32
    for (bleed, reaches) in [(0u32, false), (2, true)] {
        let (mut h, rect) = model_view(vec![plate(0.0, 1.0, (0.0, 0.5))], false, 4.5);
        h.state_mut().state.view3d.projection.seam_bleed = bleed;
        tool(&mut h, Tool::SelectRect);
        let (a, b) = (
            screen_of(&h, rect, Vec3::new(-1.1, 1.1, 0.0)),
            screen_of(&h, rect, Vec3::new(1.1, -1.1, 0.0)),
        );
        pull(&mut h, a, b);
        assert_eq!(amount(&h, 31, 30), 255, "bleed {bleed}: アイランドの中");
        assert_eq!(
            amount(&h, 32, 30) > 0,
            reaches,
            "bleed {bleed}: 縁の外 1 画素"
        );
        assert_eq!(amount(&h, 40, 30), 0, "bleed {bleed}: 遠くは選ばない");
    }
}

#[test]
fn a_face_seen_edge_on_is_faded_by_the_angle_setting_and_the_setting_off_selects_it_fully() {
    // 板を横へ 82° 回して見る（面の向きの弱めは 80° から）
    let amounts = |falloff: bool| {
        let (mut h, rect) = plate_view(false);
        h.state_mut().state.view3d.camera.yaw = 82.0;
        h.state_mut().state.view3d.projection.angle_falloff = falloff;
        h.run();
        tool(&mut h, Tool::SelectRect);
        // 3D ビューの全体を囲む（幅は並びで変わるので、rect の内側 4 pt から決める）
        let inner = rect.shrink(4.0);
        pull(&mut h, inner.min, inner.max);
        let m = selection(&h);
        let total: u32 = m.map_or(0, |m| {
            (0..N)
                .flat_map(|y| (0..N).map(move |x| (x, y)))
                .map(|(x, y)| u32::from(m.amount(x, y)))
                .sum()
        });
        total
    };
    // 弱めを切ると画素は 0 か 255（長方形の縁）。入れると、すれすれの面の量は薄くなる
    let (off, on) = (amounts(false), amounts(true));
    assert!(off > 0, "切ると選べる");
    assert!(on < off, "弱めると量が減る: {on} {off}");
}

// ───────── 取りやめ・取り残し ─────────

#[test]
fn escape_focus_loss_and_switching_tools_drop_the_drag_and_the_polygon_points_without_selecting() {
    for t in [
        Tool::SelectRect,
        Tool::SelectEllipse,
        Tool::Lasso,
        Tool::Polygon,
    ] {
        for way in 0..3 {
            let (mut h, rect) = plate_view(false);
            tool(&mut h, t);
            let (a, b) = (at(&h, rect, (10.0, 10.0)), at(&h, rect, (50.0, 50.0)));
            let c = at(&h, rect, (50.0, 12.0));
            let live = |h: &H| select::dragging(st(h)) || !st(h).sel.view3d.polygon.is_empty();
            if t == Tool::Polygon {
                clicks(&mut h, &[a, c, b]);
            } else {
                press(&h, a, PointerButton::Primary);
                h.step();
                move_to(&h, b);
                h.step();
            }
            assert!(live(&h), "{t:?} {way}: 途中がある");
            match way {
                0 => escape(&mut h),
                1 => {
                    h.event(Event::WindowFocused(false));
                    h.step();
                }
                _ => h.state_mut().state.apply(Action::SelectTool(Tool::Brush)),
            }
            h.step();
            assert!(!live(&h), "{t:?} {way}: 捨てた");
            release(&h, b, PointerButton::Primary);
            h.run();
            assert!(selection(&h).is_none(), "{t:?} {way}: 何も選ばない");
            assert_eq!(st(&h).doc.undo_count(), 0, "{t:?} {way}");
            assert!(!st(&h).is_stroking());
        }
    }
}

#[test]
fn a_mouse_release_that_never_arrived_drops_the_drag_without_selecting() {
    for t in [Tool::SelectRect, Tool::SelectEllipse, Tool::Lasso] {
        let (mut h, rect) = plate_view(false);
        tool(&mut h, t);
        let (a, b) = (at(&h, rect, (10.0, 10.0)), at(&h, rect, (50.0, 50.0)));
        // ボタンは押していないのに、ドラッグの途中だけが残った（ウィンドウの外で離したなど）
        h.state_mut().state.sel.view3d.drag = Some(Drag {
            tool: t,
            source: StrokeSource::Mouse,
            start: DVec2::new((a.x - rect.left()) as f64, (a.y - rect.top()) as f64),
            current: DVec2::new((b.x - rect.left()) as f64, (b.y - rect.top()) as f64),
            lasso: Vec::new(),
            moved: 100.0,
            pressed_with: Modifiers::NONE,
            now: Modifiers::NONE,
        });
        assert!(select::dragging(st(&h)));
        move_to(&h, b);
        h.run();
        assert!(!select::dragging(st(&h)), "{t:?}");
        assert!(
            selection(&h).is_none(),
            "{t:?}: 離した位置が分からないので選ばない"
        );
        assert_eq!(st(&h).doc.undo_count(), 0, "{t:?}");
    }
}

pub(crate) fn pen_sample(at: Pos2, contact: bool, time_ms: u32) -> PenSample {
    PenSample {
        pos: [at.x, at.y],
        pressure: 0.6,
        tilt: Tilt::default(),
        rotation: None,
        contact,
        eraser: false,
        barrel: false,
        pointer_id: 7,
        time_ms,
    }
}

/// ペンで a から b へ引き、最後の離しが本物（`lost` が false）か、OS に押しを奪われて補った離し（true）か。
pub(crate) fn pen_pull(h: &mut H, a: Pos2, b: Pos2, lost: bool) {
    for i in 0..=4u32 {
        let p = a + (b - a) * (i as f32 / 4.0);
        h.state().pen().push(pen_sample(p, true, i * 10));
        h.step();
    }
    if lost {
        h.state().pen().push_lost(pen_sample(b, false, 60));
    } else {
        h.state().pen().push(pen_sample(b, false, 60));
    }
    h.step();
    h.run();
}

#[test]
fn the_pen_selects_like_the_mouse_and_a_pen_press_taken_by_the_os_selects_nothing() {
    for t in [Tool::SelectRect, Tool::SelectEllipse] {
        let result = |pen: bool| {
            let (mut h, rect) = plate_view(false);
            tool(&mut h, t);
            let (a, b) = (at(&h, rect, (10.0, 14.0)), at(&h, rect, (41.0, 50.0)));
            if pen {
                pen_pull(&mut h, a, b, false);
            } else {
                pull(&mut h, a, b);
            }
            assert!(!select::dragging(st(&h)), "{t:?} {pen}");
            (selection(&h), st(&h).doc.undo_count())
        };
        let (pen, mouse) = (result(true), result(false));
        assert!(pen.0.is_some() && pen == mouse, "{t:?}: ペンもマウスと同じ");
        // 補った離し: 離した位置が不明なので、何も選ばない。次の押しは新しい押しとして始まる
        let (mut h, rect) = plate_view(false);
        tool(&mut h, t);
        let (a, b) = (at(&h, rect, (10.0, 14.0)), at(&h, rect, (41.0, 50.0)));
        pen_pull(&mut h, a, b, true);
        assert!(!select::dragging(st(&h)), "{t:?}");
        assert!(st(&h).view3d.input.pen_press.is_none(), "{t:?}");
        assert!(selection(&h).is_none(), "{t:?}: 補った離しでは選ばない");
        assert_eq!(st(&h).doc.undo_count(), 0, "{t:?}");
        pen_pull(&mut h, a, b, false);
        assert!(
            selection(&h).is_some(),
            "{t:?}: 次の押しは本物の離しで決まる"
        );
        assert_eq!(st(&h).doc.undo_count(), 1, "{t:?}");
    }
}

#[test]
fn a_press_during_the_leftover_of_a_released_brush_stroke_finishes_it_before_the_selection_starts()
{
    let (mut h, rect) = cube_view();
    {
        let s = &mut h.state_mut().state;
        s.brush.radius = 3.0;
        s.brush.spacing = 0.2;
        s.view3d.input.paint_budget = Some(std::time::Duration::ZERO);
        s.color.set_main([1.0, 0.0, 0.0, 1.0]);
    }
    let (a, b) = (
        screen_of(&h, rect, Vec3::new(-0.4, -0.35, -0.5)),
        screen_of(&h, rect, Vec3::new(0.4, 0.35, -0.5)),
    );
    press_with(&h, a, Modifiers::NONE);
    h.step();
    h.input_mut().events.push(Event::PointerMoved(b));
    h.step();
    release_with(&h, b, Modifiers::NONE);
    h.step();
    assert!(
        st(&h).view3d.input.released && st(&h).is_stroking(),
        "確定待ち"
    );
    assert_eq!(st(&h).doc.undo_count(), 0);
    // 描き残しがある間に、選択のツールで押す（ツールは直に替えて、押しが先に確定させる）
    h.state_mut().state.tool = Tool::SelectRect;
    let cube = cube_box(&h, rect);
    pull(
        &mut h,
        cube.left_top() - egui::vec2(10.0, 10.0),
        cube.right_bottom() + egui::vec2(10.0, 10.0),
    );
    assert!(!st(&h).is_stroking(), "{}", message(&h));
    assert_eq!(
        st(&h).doc.undo_count(),
        2,
        "線の確定と選択範囲、それぞれ 1 回: {}",
        message(&h)
    );
    assert!(selection(&h).is_some());
}

// ───────── 多角形の点 ─────────

#[test]
fn polygon_points_close_by_enter_double_click_or_the_first_point_and_backspace_and_undo_remove_the_last(
) {
    let (mut h, rect) = plate_view(false);
    tool(&mut h, Tool::Polygon);
    let ps: Vec<Pos2> = [(10.0, 10.0), (50.0, 12.0), (30.0, 52.0)]
        .iter()
        .map(|&c| at(&h, rect, c))
        .collect();
    clicks(&mut h, &ps);
    assert_eq!(st(&h).sel.view3d.polygon.len(), 3);
    // Backspace で最後の点を消す
    key(&h, Key::Backspace, Modifiers::NONE);
    h.run();
    assert_eq!(st(&h).sel.view3d.polygon.len(), 2);
    // 取り消し（Ctrl+Z）は、多角形の途中なら文書でなく最後の点
    assert!(st(&h).sel_has_polygon_point());
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert_eq!(st(&h).sel.view3d.polygon.len(), 1);
    assert_eq!(st(&h).doc.undo_count(), 0);
    // 3 点に満たないまま Enter: 断って捨てる
    click(&mut h, ps[1]);
    enter(&mut h);
    assert!(st(&h).sel.view3d.polygon.is_empty());
    assert!(selection(&h).is_none());
    assert!(message(&h).contains("点が足りません"), "{}", message(&h));
    // Enter で閉じる
    clicks(&mut h, &ps);
    enter(&mut h);
    assert!(selection(&h).is_some());
    assert_eq!(st(&h).doc.undo_count(), 1);
    let want = SelectionMask::polygon(
        &st(&h).doc,
        &[d((10.0, 10.0)), d((50.0, 12.0)), d((30.0, 52.0))],
    )
    .unwrap();
    assert_same(&h, &want, "Enter で閉じた");
    // ダブルクリックで閉じる（最後の点を 2 度押す）
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Edit(SelEdit::Clear)));
    clicks(&mut h, &ps);
    click(&mut h, ps[2]);
    assert!(
        st(&h).sel.view3d.polygon.is_empty(),
        "ダブルクリックで閉じた"
    );
    assert_same(&h, &want, "ダブルクリックで閉じた");
}

#[test]
fn polygon_points_stay_on_the_screen_while_the_view_orbits_and_starting_in_3d_drops_the_2d_points()
{
    let (mut h, rect) = plate_view(false);
    tool(&mut h, Tool::Polygon);
    let p = at(&h, rect, (20.0, 20.0));
    click(&mut h, p);
    let before = st(&h).sel.view3d.polygon.clone();
    // 右ドラッグで回す
    press(&h, rect.center(), PointerButton::Secondary);
    h.step();
    for i in 1..=5 {
        move_to(&h, rect.center() + egui::vec2(8.0 * i as f32, 0.0));
        h.step();
    }
    release(
        &h,
        rect.center() + egui::vec2(40.0, 0.0),
        PointerButton::Secondary,
    );
    h.run();
    assert!(st(&h).view3d.camera.yaw != 0.0, "回った");
    assert_eq!(st(&h).sel.view3d.polygon, before, "画面の同じ所に残る");
    // 2D のキャンバスで打っていた点は、3D で打ち始めると捨てる
    h.state_mut().state.sel.polygon.push((5.0, 5.0));
    click(&mut h, p + egui::vec2(60.0, 0.0));
    assert!(st(&h).sel.polygon.is_empty());
}

// ───────── ツールのキーの一時の切り替え ─────────

#[test]
fn releasing_the_hold_key_in_the_middle_of_a_3d_selection_drag_finishes_the_drag_first() {
    for (command, key_, shift, tool_) in [
        ("tool.select_rectangle", Key::M, false, Tool::SelectRect),
        ("tool.select_ellipse", Key::M, true, Tool::SelectEllipse),
        ("tool.lasso", Key::L, false, Tool::Lasso),
    ] {
        let (mut h, rect) = plate_view(false);
        h.state_mut()
            .state
            .apply(Action::ToolKeyMode(command, ToolKeyMode::Hold));
        h.run();
        let m = if shift {
            Modifiers::SHIFT
        } else {
            Modifiers::NONE
        };
        h.event(Event::ModifiersChanged(m));
        h.event(Event::Key {
            key: key_,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: m,
        });
        h.step();
        h.step();
        assert_eq!(st(&h).tool, tool_);
        h.event(Event::ModifiersChanged(Modifiers::NONE));
        h.step();
        let (a, b) = (at(&h, rect, (10.0, 14.0)), at(&h, rect, (41.0, 50.0)));
        let corner = pos2(b.x, a.y);
        press(&h, a, PointerButton::Primary);
        h.step();
        move_to(&h, corner);
        h.step();
        assert!(
            select::dragging(st(&h)),
            "{tool_:?}: 3D のドラッグが始まった"
        );
        h.event(Event::Key {
            key: key_,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        for _ in 0..20 {
            h.step();
        }
        assert_eq!(st(&h).tool, tool_, "{tool_:?}: ドラッグの途中は戻さない");
        assert!(select::dragging(st(&h)), "{tool_:?}");
        move_to(&h, b);
        h.step();
        release(&h, b, PointerButton::Primary);
        h.step();
        h.run();
        assert!(!select::dragging(st(&h)), "{tool_:?}");
        assert!(selection(&h).is_some(), "{tool_:?}: {}", message(&h));
        assert_eq!(st(&h).doc.undo_count(), 1, "{tool_:?}");
        assert_eq!(st(&h).tool, Tool::Brush, "{tool_:?}: 選び終えたら戻る");
    }
}

#[test]
fn a_pen_press_taken_by_the_os_during_a_3d_hold_selection_drag_lets_the_tool_return() {
    let (mut h, rect) = plate_view(false);
    h.state_mut().state.apply(Action::ToolKeyMode(
        "tool.select_rectangle",
        ToolKeyMode::Hold,
    ));
    h.run();
    h.event(Event::Key {
        key: Key::M,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.step();
    assert_eq!(st(&h).tool, Tool::SelectRect);
    let (a, b) = (at(&h, rect, (10.0, 14.0)), at(&h, rect, (41.0, 50.0)));
    for p in [a, a + (b - a) * 0.5, b] {
        h.state().pen().push(pen_sample(p, true, 0));
        h.step();
    }
    assert!(select::dragging(st(&h)));
    h.event(Event::Key {
        key: Key::M,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    for _ in 0..20 {
        h.step();
    }
    assert_eq!(st(&h).tool, Tool::SelectRect, "ドラッグの途中は戻さない");
    h.state().pen().push_lost(pen_sample(b, false, 50));
    h.step();
    h.run();
    assert!(!select::dragging(st(&h)));
    assert!(selection(&h).is_none(), "補った離しでは選ばない");
    assert_eq!(st(&h).tool, Tool::Brush, "奪われたあとで戻る");
}

// ───────── 取り消し・保存・断り ─────────

#[test]
fn one_undo_removes_the_selection_and_it_survives_save_and_reopen_like_a_2d_selection() {
    let (mut h, rect) = plate_view(false);
    tool(&mut h, Tool::SelectEllipse);
    pull_c(&mut h, rect, ELLIPSE.0, ELLIPSE.1);
    assert_eq!(st(&h).doc.undo_count(), 1);
    let selected = selection(&h).unwrap();
    // 取り消し 1 回で選択なしへ、やり直しで戻る
    key(&h, Key::Z, Modifiers::COMMAND);
    h.run();
    assert!(selection(&h).is_none());
    key(&h, Key::Y, Modifiers::COMMAND);
    h.run();
    assert!(selection(&h) == Some(selected.clone()));
    // 保存して開き直すと、2D で作った選択範囲と同じく戻る
    let dir = std::env::temp_dir().join(format!("yolu-3dsel-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("p.ylp");
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(path.clone()));
    assert!(message(&h).starts_with("保存しました"), "{}", message(&h));
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Edit(SelEdit::Clear)));
    assert!(selection(&h).is_none());
    h.state_mut().state.apply(Action::OpenProject(path.clone()));
    h.run();
    assert!(
        selection(&h) == Some(selected),
        "開き直すと選択範囲が戻る: {}",
        message(&h)
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_read_only_set_refuses_at_the_press_and_a_small_budget_leaves_the_selection_as_it_was() {
    // 読むだけのセット: 押したところで断る（ドラッグを始めない）
    for t in [
        Tool::SelectRect,
        Tool::SelectEllipse,
        Tool::Lasso,
        Tool::Polygon,
        Tool::Wand,
    ] {
        let (mut h, rect) = plate_view(false);
        tool(&mut h, t);
        h.state_mut().state.sets.get_mut(0).unwrap().read_only = Some("読めない中身".into());
        let (a, b) = (at(&h, rect, (10.0, 14.0)), at(&h, rect, (41.0, 50.0)));
        press(&h, a, PointerButton::Primary);
        h.step();
        assert!(
            message(&h).contains("読めない中身"),
            "{t:?} {}",
            message(&h)
        );
        assert!(!select::dragging(st(&h)));
        move_to(&h, b);
        release(&h, b, PointerButton::Primary);
        h.run();
        assert!(selection(&h).is_none(), "{t:?}");
        assert!(st(&h).sel.view3d.polygon.is_empty(), "{t:?}");
        assert_eq!(st(&h).doc.undo_count(), 0, "{t:?}");
    }
    // 予算が小さい: 選択範囲を変えずに理由を知らせる
    for t in [Tool::SelectRect, Tool::SelectEllipse, Tool::Lasso] {
        let (mut h, rect) = plate_view(false);
        tool(&mut h, t);
        let (a, b) = (at(&h, rect, (10.0, 14.0)), at(&h, rect, (41.0, 50.0)));
        pull_c(&mut h, rect, (5.0, 5.0), (8.0, 8.0));
        let before = selection(&h);
        let steps = st(&h).doc.undo_count();
        h.state_mut()
            .state
            .doc
            .set_stroke_budget_bytes(4096)
            .unwrap();
        if t == Tool::Lasso {
            drag(&mut h, &[a, pos2(b.x, a.y), b, pos2(a.x, b.y)]);
        } else {
            pull(&mut h, a, b);
        }
        assert!(!message(&h).is_empty(), "{t:?}");
        assert!(selection(&h) == before, "{t:?}");
        assert_eq!(st(&h).doc.undo_count(), steps, "{t:?}");
    }
}

#[test]
fn the_tools_that_work_in_3d_no_longer_say_they_are_2d_only_and_the_others_still_do() {
    for lang in Lang::ALL {
        for (t, works) in [
            (Tool::SelectRect, true),
            (Tool::SelectEllipse, true),
            (Tool::Lasso, true),
            (Tool::Polygon, true),
            (Tool::Wand, true),
            (Tool::SelectPen, true),
            (Tool::Move, false),
            (Tool::Liquify, false),
            (Tool::Text, false),
        ] {
            let (mut h, rect) = plate_view(false);
            h.state_mut().state.lang = lang;
            tool(&mut h, t);
            h.state_mut().state.message.clear();
            let p = at(&h, rect, (20.0, 20.0));
            press(&h, p, PointerButton::Primary);
            h.step();
            release(&h, p, PointerButton::Primary);
            h.run();
            let text = message(&h);
            let refused = text.contains("2D のキャンバスで使います")
                || text.contains("works on the 2D canvas");
            assert_eq!(refused, !works, "{lang:?} {t:?}: {text}");
        }
    }
}

// ───────── キャンバスと 3D ビューを並べた画面 ─────────

/// キャンバスと 3D ビューを並べる（`canvas_first` ならキャンバスが左で、3D ビューは後に描かれる。そうでなければ 3D ビューが左）。
pub(crate) fn side_by_side(canvas_first: bool) -> (H, Rect) {
    use egui_dock::{DockState, NodeIndex};
    let (mut h, _) = model_view(vec![plate(0.0, 1.0, (0.0, 1.0))], false, 8.0);
    let mut dock = DockState::new(vec![if canvas_first {
        Tab::Canvas
    } else {
        Tab::View3d
    }]);
    dock.main_surface_mut().split_right(
        NodeIndex::root(),
        0.5,
        vec![if canvas_first {
            Tab::View3d
        } else {
            Tab::Canvas
        }],
    );
    h.state_mut().dock = dock;
    h.state_mut().state.sel.animate = false;
    h.run();
    let rect = h.state().view3d_rect().expect("3D も出ている");
    assert!(st(&h).ui.canvas_visible, "キャンバスも出ている");
    (h, rect)
}

#[test]
fn backspace_and_undo_remove_one_polygon_point_each_when_the_canvas_is_beside_the_3d_view() {
    for canvas_first in [true, false] {
        let (mut h, rect) = side_by_side(canvas_first);
        tool(&mut h, Tool::Polygon);
        let ps: Vec<Pos2> = [(10.0, 10.0), (50.0, 12.0), (30.0, 52.0), (12.0, 40.0)]
            .iter()
            .map(|&c| at(&h, rect, c))
            .collect();
        clicks(&mut h, &ps);
        assert_eq!(st(&h).sel.view3d.polygon.len(), 4, "{canvas_first}");
        key(&h, Key::Backspace, Modifiers::NONE);
        h.run();
        assert_eq!(
            st(&h).sel.view3d.polygon.len(),
            3,
            "{canvas_first}: Backspace 1 回で点が 1 つだけ消える"
        );
        key(&h, Key::Z, Modifiers::COMMAND);
        h.run();
        assert_eq!(
            st(&h).sel.view3d.polygon.len(),
            2,
            "{canvas_first}: Ctrl+Z も 1 つだけ"
        );
        assert_eq!(st(&h).doc.undo_count(), 0, "{canvas_first}");
    }
}

#[test]
fn backspace_removes_one_point_of_a_2d_polygon_too_beside_the_3d_view() {
    for canvas_first in [true, false] {
        let (mut h, _) = side_by_side(canvas_first);
        tool(&mut h, Tool::Polygon);
        let canvas = canvas_rect(&h);
        for dx in [-60.0, 0.0, 60.0] {
            click(&mut h, offset(center(canvas), dx, dx.abs() * 0.5));
        }
        assert_eq!(st(&h).sel.polygon.len(), 3, "{canvas_first}");
        key(&h, Key::Backspace, Modifiers::NONE);
        h.run();
        assert_eq!(st(&h).sel.polygon.len(), 2, "{canvas_first}");
        assert!(st(&h).sel.view3d.polygon.is_empty());
    }
}

#[test]
fn escape_in_the_middle_of_a_3d_polygon_keeps_the_selection_that_was_there_in_both_arrangements() {
    for canvas_first in [true, false] {
        let (mut h, rect) = side_by_side(canvas_first);
        // 前からある選択範囲
        tool(&mut h, Tool::SelectEllipse);
        pull_c(&mut h, rect, ELLIPSE.0, ELLIPSE.1);
        let kept = selection(&h);
        assert!(kept.is_some(), "{canvas_first}");
        // 多角形の途中
        tool(&mut h, Tool::Polygon);
        let ps: Vec<Pos2> = [(10.0, 10.0), (50.0, 12.0)]
            .iter()
            .map(|&c| at(&h, rect, c))
            .collect();
        clicks(&mut h, &ps);
        assert_eq!(st(&h).sel.view3d.polygon.len(), 2, "{canvas_first}");
        escape(&mut h);
        h.run();
        assert!(
            st(&h).sel.view3d.polygon.is_empty(),
            "{canvas_first}: 点を捨てた"
        );
        assert!(
            selection(&h) == kept,
            "{canvas_first}: 同じ Esc で前からある選択範囲まで解除しない"
        );
        // 次の Esc は、やめるものが無いので選択範囲を解除する（キャンバスが出ているときの 2D と同じ）
        escape(&mut h);
        h.run();
        assert!(selection(&h).is_none(), "{canvas_first}: 次の Esc で解除");
    }
}

#[test]
fn the_polygon_double_click_uses_the_same_clock_for_the_mouse_and_the_pen() {
    // ペンの点の時刻は機器の時計（ここでは 0 から数えたミリ秒）で、アプリの時計とは別。ダブルクリックは、マウスもペンも同じフレームの時計で見る
    let (mut h, rect) = plate_view(false);
    tool(&mut h, Tool::Polygon);
    let ps: Vec<Pos2> = [(10.0, 10.0), (50.0, 12.0), (30.0, 52.0)]
        .iter()
        .map(|&c| at(&h, rect, c))
        .collect();
    clicks(&mut h, &ps);
    // 1 秒あとに、マウスで打った点と同じ所をペンで押す: ダブルクリックではなく、点が 1 つ増える（機器の時計の 10 ms がアプリの時計の何秒かと比べられない）
    for _ in 0..60 {
        h.step();
    }
    h.state().pen().push(pen_sample(ps[2], true, 10));
    h.step();
    h.state().pen().push(pen_sample(ps[2], false, 30));
    h.step();
    assert_eq!(st(&h).sel.view3d.polygon.len(), 4, "閉じない");
    assert!(selection(&h).is_none());
    // すぐあとに同じ所をペンでもう一度押せば、ダブルクリックで閉じる
    h.state().pen().push(pen_sample(ps[2], true, 80));
    h.step();
    h.state().pen().push(pen_sample(ps[2], false, 100));
    h.step();
    h.run();
    assert!(
        st(&h).sel.view3d.polygon.is_empty(),
        "ダブルクリックで閉じた"
    );
    assert!(selection(&h).is_some());
}

#[test]
fn an_open_amount_dialog_blocks_the_3d_polygon_keys_like_it_does_on_the_canvas() {
    let (mut h, rect) = plate_view(false);
    tool(&mut h, Tool::SelectRect);
    pull_c(&mut h, rect, (8.0, 8.0), (40.0, 40.0));
    tool(&mut h, Tool::Polygon);
    let ps: Vec<Pos2> = [(10.0, 10.0), (50.0, 12.0), (30.0, 52.0)]
        .iter()
        .map(|&c| at(&h, rect, c))
        .collect();
    clicks(&mut h, &ps);
    h.state_mut()
        .state
        .apply(Action::Sel(SelAction::Ui(SelUiOp::OpenAmount(
            yolu_app::selection::ModifyKind::Grow,
        ))));
    h.run();
    assert!(st(&h).sel.dialog.is_some(), "量を聞くウィンドウが開いた");
    // ウィンドウが開いている間の Enter・Backspace は、多角形に使わない（ウィンドウのもの）
    key(&h, Key::Backspace, Modifiers::NONE);
    h.run();
    assert_eq!(st(&h).sel.view3d.polygon.len(), 3, "点を消さない");
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert_eq!(st(&h).sel.view3d.polygon.len(), 3, "閉じない");
}

// ───────── 自動選択の範囲はテクスチャで決まる ─────────

#[test]
fn the_wand_region_follows_the_texture_not_the_surface_so_it_takes_hidden_faces_and_crosses_seams()
{
    let (mut h, rect) = two_plates();
    // 奥の板（UV の左半分）の全部を赤に塗る。手前の板（右半分）は透明のまま
    paint_rect(&mut h, (0, 0, 32, 64));
    tool(&mut h, Tool::Wand);
    h.state_mut().state.sel.tolerance = 0;
    // 奥の板の見える所（左の端のほう）を押す
    let p = screen_of(&h, rect, Vec3::new(-0.8, 0.0, 0.0));
    click(&mut h, p);
    assert_eq!(st(&h).doc.undo_count(), 1, "{}", message(&h));
    // 手前の板に隠れた所も選ぶ
    let (hx, hy) = (16u32, 32u32);
    assert_eq!(amount(&h, hx, hy), 255, "隠れた所も選ぶ");
    // 範囲は、テクスチャの上で色が近くつながる所（赤く塗った左半分）
    let want = SelectionMask::rectangle(&st(&h).doc, 0, 0, 32, 64);
    assert_same(&h, &want, "赤の範囲");
    // 塗っていないテクスチャでは、押した板から、UV の継ぎ目の向こう（もう一方の板の画素）まで、テクスチャの全部がつながる
    let (mut h, rect) = two_plates();
    tool(&mut h, Tool::Wand);
    h.state_mut().state.sel.tolerance = 0;
    let p = screen_of(&h, rect, Vec3::new(-0.8, 0.0, 0.0));
    click(&mut h, p);
    assert_eq!(selected_count(&h, N), (N * N) as usize, "{}", message(&h));
}

// ───────── 絵（日英） ─────────

#[test]
fn snapshot_selection_in_3d_in_both_languages() {
    use egui_dock::{DockState, NodeIndex};
    let mut snapshots = egui_kittest::SnapshotResults::new();
    for lang in Lang::ALL {
        let (mut h, rect) = cube_view();
        h.state_mut().state.lang = lang;
        // キャンバスと 3D ビューを並べる（選択範囲の縁はキャンバスに出る）
        let mut dock = DockState::new(vec![Tab::Canvas]);
        dock.main_surface_mut()
            .split_right(NodeIndex::root(), 0.5, vec![Tab::View3d]);
        h.state_mut().dock = dock;
        h.state_mut().state.sel.animate = false;
        h.run();
        let rect = h.state().view3d_rect().unwrap_or(rect);
        tool(&mut h, Tool::SelectEllipse);
        let (a, b) = (
            screen_of(&h, rect, Vec3::new(-0.35, -0.3, -0.5)),
            screen_of(&h, rect, Vec3::new(0.45, 0.4, -0.5)),
        );
        // 引いている途中
        press(&h, a, PointerButton::Primary);
        h.step();
        move_to(&h, a + (b - a) * 0.6);
        h.step();
        move_to(&h, b);
        h.step();
        h.state_mut().state.message.clear();
        h.run_steps(2);
        h.snapshot(lang.pick("view3d_select_drag_ja", "view3d_select_drag_en"));
        snapshots.extend_harness(&mut h);
        // 離した結果
        release(&h, b, PointerButton::Primary);
        h.step();
        h.event(Event::PointerGone);
        h.run();
        h.snapshot(lang.pick("view3d_select_result_ja", "view3d_select_result_en"));
        snapshots.extend_harness(&mut h);
    }
}
