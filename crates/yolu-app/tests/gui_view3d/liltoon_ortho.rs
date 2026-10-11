//! 正投影の 3D ビューの描き方（lilToon が `lilIsPerspective()` で分ける 3 か所と、背景の向き）: マットキャップは「透視」の設定によらず
//! カメラの手前への向きで読む・輪郭線の Z バイアスは前の向きへ押す（視線に沿うので画面の縁は動かない）・背景は前の向き 1 つ。
//! 透視の結果は前と同じ（透視の側は比べの元として同じ場面を描く）。
use crate::common::{app, click_tab, move_to};
use egui::Pos2;
use egui_kittest::Harness;
use yolu_app::state::Action;
use yolu_app::view3d::display::{EnvKind, Op};
use yolu_app::view3d::model::ViewModel;
use yolu_app::YoluApp;
use yolu_core::geometry::{
    cube_sphere, visible_height, ModelMesh, OrbitCamera, Projection, Submesh,
};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::look::{LookKind, LookValue, MaterialLook, TextureSource};
use yolu_core::{Channel, ImageColorSpace, ImageInput, Rgba8};

type H = Harness<'static, YoluApp>;

fn view() -> H {
    // （3D の表示域の幅が前の既定の並びと同じになるよう、右の列を広げた分だけウィンドウも広げる）
    let mut h = app(1207.0, 640.0, 64);
    click_tab(&mut h, yolu_app::Tab::View3d);
    move_to(&h, egui::pos2(1.0, 1.0));
    h.run();
    h
}

/// カメラに向いた板（外向きの法線は −Z）。
fn quad(size: f32) -> ModelMesh {
    let h = size * 0.5;
    ModelMesh {
        name: "板".into(),
        positions: vec![
            Vec3::new(-h, -h, 0.0),
            Vec3::new(h, -h, 0.0),
            Vec3::new(-h, h, 0.0),
            Vec3::new(h, h, 0.0),
        ],
        normals: vec![Vec3::NEG_Z; 4],
        uvs: vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 1.0),
        ],
        submeshes: vec![Submesh {
            material: 0,
            indices: vec![0, 2, 1, 2, 3, 1],
        }],
    }
}

fn set_model(h: &mut H, meshes: Vec<ModelMesh>) {
    let state = &mut h.state_mut().state;
    let revision = state.view3d.next_revision();
    state.view3d.material = 0;
    let model = ViewModel::new("試験", meshes, vec![Some("試験".to_string())], revision).unwrap();
    model.tangents();
    state.view3d.set_model(model);
    h.run();
}

/// −Z の側から見るカメラ（正投影なら、同じ距離の透視と注視点の所で同じ大きさ）。
fn look_at(h: &mut H, distance: f32, ortho: bool) {
    h.state_mut().state.view3d.camera = OrbitCamera {
        target: Vec3::ZERO,
        yaw: 0.0,
        pitch: 0.0,
        distance,
        model_radius: 1.0,
        projection: if ortho {
            Projection::Orthographic {
                height: visible_height(distance),
            }
        } else {
            Projection::Perspective
        },
    };
    h.run();
}

fn op(h: &mut H, op: Op) {
    h.state_mut().apply(Action::View3d(op));
    h.run();
}

fn fill_color(h: &mut H, color: [u8; 4]) {
    h.state_mut()
        .state
        .doc
        .add_fill_layer(
            "色",
            &[(
                Channel::Color,
                Rgba8::new(color[0], color[1], color[2], color[3]),
            )],
            None,
        )
        .unwrap();
    h.run();
}

fn lil() -> MaterialLook {
    let mut look = MaterialLook {
        kind: LookKind::LilToon,
        ..MaterialLook::default()
    };
    look.textures
        .insert("_MainTex".into(), TextureSource::Channel(Channel::Color));
    look
}

fn set_look(h: &mut H, look: MaterialLook) {
    h.state_mut().state.doc.set_look(look, false).unwrap();
    h.run();
}

/// 世界の点の、ウィンドウの画面の点。
fn screen(h: &H, world: Vec3) -> Pos2 {
    let rect = h.state().view3d_rect().unwrap();
    let c = h.state().state.view3d.camera;
    let s = c
        .view(rect.width(), rect.height())
        .to_screen(world)
        .unwrap();
    let p = rect.min + egui::vec2(s.x, s.y);
    assert!(
        rect.shrink(4.0).contains(p),
        "{world} は表示域の外: {p:?} {rect:?}"
    );
    p
}

fn px(image: &image::RgbaImage, p: Pos2) -> [u8; 3] {
    let c = image.get_pixel(p.x.round() as u32, p.y.round() as u32).0;
    [c[0], c[1], c[2]]
}

fn far(a: [u8; 3], b: [u8; 3]) -> u8 {
    (0..3).map(|i| a[i].abs_diff(b[i])).max().unwrap()
}

/// マットキャップだけの色（光・メインカラーの影響なし）。画像は横に赤、縦に緑が増える 16 × 16。
fn matcap_look(h: &mut H, perspective: bool) {
    let id = yolu_core::ImageId(0xA11C_A900_0000_0000_0000_0000_0000_0007);
    let mut pixels = Vec::new();
    for y in 0..16u8 {
        for x in 0..16u8 {
            pixels.extend_from_slice(&[x * 16, y * 16, 128, 255]);
        }
    }
    let doc = &mut h.state_mut().state.doc;
    let image = ImageInput::new(16, 16, pixels, ImageColorSpace::Srgb).unwrap();
    let inputs = doc.effect_inputs().clone().with_image(id, image);
    doc.set_effect_inputs(inputs).unwrap();
    let mut look = lil();
    for (name, v) in [
        ("_UseMatCap", 1.0),
        ("_MatCapBlend", 1.0),
        ("_MatCapBlendMode", 0.0),
        ("_MatCapEnableLighting", 0.0),
        ("_MatCapMainStrength", 0.0),
        ("_MatCapPerspective", if perspective { 1.0 } else { 0.0 }),
    ] {
        look.properties.insert(name.into(), LookValue::Float(v));
    }
    look.properties
        .insert("_MatCapColor".into(), LookValue::Color([1.0; 4]));
    look.textures
        .insert("_MatCapTex".into(), TextureSource::Image(id));
    set_look(h, look);
}

#[test]
fn the_matcap_reads_by_the_camera_front_in_orthographic_whatever_its_perspective_switch() {
    let mut h = view();
    set_model(&mut h, vec![quad(1.6)]);
    op(&mut h, Op::Env(EnvKind::None));
    fill_color(&mut h, [200, 120, 60, 255]);
    let sides = |h: &mut H| {
        let image = h.render().expect("描ける");
        (
            px(&image, screen(h, Vec3::new(-0.3, 0.0, 0.0))),
            px(&image, screen(h, Vec3::new(0.3, 0.0, 0.0))),
        )
    };
    // 透視で「透視」が入: 板の上で視線が変わるので、左右で読む所が違う（比べの元）
    look_at(&mut h, 1.5, false);
    matcap_look(&mut h, true);
    let (l, r) = sides(&mut h);
    assert!(far(l, r) > 20, "{l:?} {r:?}");
    // 正投影: 「透視」が入でも切でも、カメラの手前への向きで読む（平らな面は一様、両方で同じ）
    look_at(&mut h, 1.5, true);
    let (l1, r1) = sides(&mut h);
    assert!(far(l1, r1) <= 1, "{l1:?} {r1:?}");
    matcap_look(&mut h, false);
    let (l0, r0) = sides(&mut h);
    assert_eq!((l0, r0), (l1, r1));
    // 透視で「透視」が切なら、正投影と同じ読み方
    look_at(&mut h, 1.5, false);
    let (lp, rp) = sides(&mut h);
    assert!(far(lp, rp) <= 1 && far(lp, l1) <= 1, "{lp:?} {rp:?} {l1:?}");
    // 絵: 球のマットキャップ（正投影）
    set_model(&mut h, vec![cube_sphere(24, 0.5)]);
    look_at(&mut h, 3.0, true);
    matcap_look(&mut h, true);
    h.event(egui::Event::PointerGone);
    h.run();
    h.snapshot("view3d_liltoon_matcap_orthographic");
}

/// 輪郭線（赤・光なし）の右の縁: 中心の行で、右の端から内へ探した最初の赤の画素の x。
fn outline_edge(h: &mut H) -> u32 {
    let image = h.render().expect("描ける");
    let rect = h.state().view3d_rect().unwrap();
    let row = rect.center().y.round() as u32;
    (rect.center().x as u32..rect.right() as u32 - 1)
        .rev()
        .find(|x| {
            let p = image.get_pixel(*x, row).0;
            p[0] > 150 && p[1] < 80 && p[2] < 80
        })
        .expect("輪郭線")
}

fn outline_look(h: &mut H, z_bias: f32) {
    let mut look = lil();
    look.shader = "Hidden/lilToonOutline".into();
    look.properties.insert(
        "_OutlineColor".into(),
        LookValue::Color([1.0, 0.0, 0.0, 1.0]),
    );
    for (name, v) in [
        ("_OutlineWidth", 2.0),
        ("_OutlineFixWidth", 0.0),
        ("_OutlineEnableLighting", 0.0),
        ("_OutlineZBias", z_bias),
    ] {
        look.properties.insert(name.into(), LookValue::Float(v));
    }
    set_look(h, look);
}

#[test]
fn the_outline_z_bias_pushes_toward_the_camera_front_in_orthographic() {
    let mut h = view();
    set_model(&mut h, vec![cube_sphere(32, 0.5)]);
    op(&mut h, Op::LightYaw(180.0));
    op(&mut h, Op::Env(EnvKind::None));
    fill_color(&mut h, [255, 255, 255, 255]);
    // Z バイアスは視線に沿って奥へ押す（透視はカメラの位置からの向き、正投影は前の向き）ので、どちらでも画面の縁は動かない
    // （正投影でもカメラの位置からの向きに押すと、縁は外へずれる: 縁の横のずれ 0.52 / 奥行き 3 × 0.4 ≈ 0.07 = 20 画素ほど）
    for ortho in [false, true] {
        look_at(&mut h, 3.0, ortho);
        outline_look(&mut h, 0.0);
        let flat = outline_edge(&mut h);
        outline_look(&mut h, 0.4);
        let pushed = outline_edge(&mut h);
        assert!(
            pushed.abs_diff(flat) <= 1,
            "正投影 {ortho}: {flat} {pushed}"
        );
    }
    let pushed = outline_edge(&mut h);
    // 殻の縁は、半径 0.5 + 太さ 0.02 を見える高さで写した所
    let rect = h.state().view3d_rect().unwrap();
    let expected = rect.center().x + 0.52 * rect.height() / visible_height(3.0);
    assert!(
        (pushed as f32 - expected).abs() <= 2.0,
        "{pushed} {expected}"
    );
    h.event(egui::Event::PointerGone);
    h.run();
    h.snapshot("view3d_liltoon_outline_orthographic");
}

#[test]
fn the_background_looks_along_the_view_in_orthographic() {
    let mut h = view();
    set_model(&mut h, vec![quad(0.2)]);
    op(&mut h, Op::Env(EnvKind::Sky));
    op(&mut h, Op::EnvBackground(true));
    op(&mut h, Op::EnvBlur(0.0));
    let rect = h.state().view3d_rect().unwrap();
    let corners = [
        rect.min + egui::vec2(10.0, 10.0),
        rect.max - egui::vec2(10.0, 10.0),
        egui::pos2(rect.min.x + 10.0, rect.max.y - 10.0),
    ];
    // 透視: 背景は画素ごとの視線の向き（上と下で空の色が違う）
    h.state_mut().state.view3d.camera = OrbitCamera {
        pitch: 20.0,
        ..OrbitCamera::default()
    };
    h.run();
    let image = h.render().expect("描ける");
    let persp: Vec<[u8; 3]> = corners.iter().map(|p| px(&image, *p)).collect();
    assert!(far(persp[0], persp[1]) > 3, "{persp:?}");
    // 正投影: 視線はどこも前の向きなので、背景は 1 つの色（引いて広く見ても。近い面と遠い面の間の点からの向きでは、隅ほど傾く）
    h.state_mut().state.view3d.camera.projection = Projection::Orthographic { height: 40.0 };
    h.run();
    let image = h.render().expect("描ける");
    let ortho: Vec<[u8; 3]> = corners.iter().map(|p| px(&image, *p)).collect();
    assert!(ortho.windows(2).all(|w| far(w[0], w[1]) <= 1), "{ortho:?}");
    h.event(egui::Event::PointerGone);
    h.run();
    h.snapshot("view3d_background_orthographic");
}
