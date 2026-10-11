//! lilToon の再現（3D ビューのマテリアル表示）: 光・影・輪郭線・カットアウト・半透明・ユーザーチャンネルのマスクを、lilToon の式を
//! CPU で書いた参照と照らす（egui_kittest。描画は wgpu）。
use crate::common;

use common::*;
use egui::Pos2;
use egui_kittest::Harness;
use yolu_app::state::Action;
use yolu_app::view3d::brdf;
use yolu_app::view3d::display::{Display, EnvKind, Op};
use yolu_app::view3d::model::ViewModel;
use yolu_app::YoluApp;
use yolu_core::geometry::{cube_sphere, ModelMesh, OrbitCamera, Submesh};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::look::{
    LookKind, LookValue, MaterialLook, PlaneSource, ReceivedImage, ReceivedLook, TextureSource,
};
use yolu_core::{
    Channel, ChannelInfo, ChannelKind, ColorSpace, ImageColorSpace, ImageInput, Rgba8,
};

fn view(width: f32, height: f32, doc: u32) -> Harness<'static, YoluApp> {
    let mut h = app(width, height, doc);
    click_tab(&mut h, yolu_app::Tab::View3d);
    move_to(&h, egui::pos2(1.0, 1.0));
    h.run();
    h
}

/// 新しいセットの既定（lilToon）でなく、標準の見た目から始める（標準と lilToon を比べる試験・割り当ての無いスロットを見る試験）。
/// 開いた古い .ylp（look.json が無い）と同じ始まり。
fn start_standard(h: &mut Harness<'_, YoluApp>, set: usize) {
    h.state_mut()
        .state
        .set_doc_mut(set)
        .restore_look(MaterialLook::default())
        .expect("作ったばかりの文書");
    h.run();
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

fn set_model(h: &mut Harness<'_, YoluApp>, meshes: Vec<ModelMesh>) {
    let state = &mut h.state_mut().state;
    let revision = state.view3d.next_revision();
    state.view3d.material = 0;
    let model = ViewModel::new("試験", meshes, vec![Some("試験".to_string())], revision).unwrap();
    model.tangents();
    state.view3d.set_model(model);
    h.run();
}

fn look_at(h: &mut Harness<'_, YoluApp>, distance: f32) {
    h.state_mut().state.view3d.camera = OrbitCamera {
        target: Vec3::ZERO,
        yaw: 0.0,
        pitch: 0.0,
        distance,
        model_radius: 1.0,
        ..Default::default()
    };
    h.run();
}

fn op(h: &mut Harness<'_, YoluApp>, op: Op) {
    h.state_mut().apply(Action::View3d(op));
    h.run();
}

/// 光の来る向きを決め、環境なし（一様な環境光）にする。
fn light(h: &mut Harness<'_, YoluApp>, yaw: f32, pitch: f32) {
    op(h, Op::LightYaw(yaw));
    op(h, Op::LightPitch(pitch));
    op(h, Op::Env(EnvKind::None));
}

fn fill_color(h: &mut Harness<'_, YoluApp>, color: [u8; 4]) {
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

fn set_look(h: &mut Harness<'_, YoluApp>, look: MaterialLook) {
    h.state_mut().state.doc.set_look(look, false).unwrap();
    h.run();
}

fn middle(h: &Harness<'_, YoluApp>) -> Pos2 {
    h.state().view3d_rect().expect("3D のタブを描いた").center()
}

fn px(image: &image::RgbaImage, p: Pos2) -> [u8; 3] {
    let c = image.get_pixel(p.x.round() as u32, p.y.round() as u32).0;
    [c[0], c[1], c[2]]
}

fn assert_close(actual: [u8; 3], expected: [u8; 3], tol: u8, what: &str) {
    for k in 0..3 {
        assert!(
            actual[k].abs_diff(expected[k]) <= tol,
            "{what}: {actual:?} と期待 {expected:?}（許す差 {tol}）"
        );
    }
}

// ───────── CPU の参照（環境なし・一様な環境光、平らな面、テクスチャはメインカラーだけ） ─────────

fn lin(c: [f32; 3]) -> Vec3 {
    Vec3::new(
        brdf::srgb_to_linear(c[0]),
        brdf::srgb_to_linear(c[1]),
        brdf::srgb_to_linear(c[2]),
    )
}

fn to_bytes(c: Vec3) -> [u8; 3] {
    let g = brdf::linear_to_srgb3(c);
    [0, 1, 2].map(|k| (g[k].clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// 3D ビューの光（表示の既定の強さ・色）から、lilToon の光の向きと明るさ（OpenLit の ComputeLights と lilToon の補正）。
fn lil_light(to_light: Vec3) -> (Vec3, Vec3) {
    let main = Display::default().direct_light()[0];
    let flat = brdf::srgb_to_linear(0.2);
    let lum = main * (0.039_681_915 + 0.458_021_8 + 0.006_096_539_6); // OpenLitLuminance の係数
    let dir = (to_light * lum + Vec3::new(0.001, 0.002, 0.001)).normalize();
    let color = Vec3::splat((flat + main).clamp(0.05, 1.0));
    (dir, color)
}

fn tooning(value: f32, border: f32, blur: f32, range: f32) -> f32 {
    let bmin = (border - blur * 0.5 - range).clamp(0.0, 1.0);
    let bmax = (border + blur * 0.5).clamp(0.0, 1.0);
    ((value - bmin) / (bmax - bmin).clamp(0.0, 1.0)).clamp(0.0, 1.0)
}

/// lilGetShading（影の既定の値、マスク・影色テクスチャなし、平らな面: fwidth は 0）。
fn shaded(albedo: Vec3, n: Vec3, to_light: Vec3) -> Vec3 {
    let (l, lc) = lil_light(to_light);
    let ln = (l.dot(n) * 0.5 + 0.5).clamp(0.0, 1.0);
    let x = tooning(ln, 0.5, 0.1, 0.0);
    let y = tooning(ln, 0.15, 0.1, 0.0);
    let w = tooning(ln, 0.5, 0.1, 0.08);
    let c1 = lin([0.82, 0.76, 0.85]);
    let c2 = lin([0.68, 0.66, 0.79]);
    let mut indirect = albedo * c1;
    let y2 = 1.0 - y;
    indirect = indirect.lerp(albedo * c2, y2);
    let direct = albedo * lc;
    indirect *= lc;
    indirect = indirect.min(direct);
    let border = lin([1.0, 0.1, 0.0]);
    indirect += (direct - indirect) * (border * w);
    let col = indirect.lerp(direct, x);
    col.min(albedo)
}

#[test]
fn a_lit_toon_surface_is_albedo_times_the_clamped_light() {
    let mut h = view(900.0, 640.0, 64);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, 2.5);
    light(&mut h, 180.0, 0.0);
    let color = [200u8, 120, 60, 255];
    fill_color(&mut h, color);
    set_look(&mut h, lil());
    let image = h.render().expect("描ける");
    let albedo = lin([
        color[0] as f32 / 255.0,
        color[1] as f32 / 255.0,
        color[2] as f32 / 255.0,
    ]);
    let (_, lc) = lil_light(Vec3::NEG_Z);
    assert_close(px(&image, middle(&h)), to_bytes(albedo * lc), 2, "影なし");
    // 明るさの上限・下限・モノクロ化・Unlit 化
    let mut look = lil();
    look.properties
        .insert("_AsUnlit".into(), LookValue::Float(1.0));
    set_look(&mut h, look);
    let image = h.render().expect("描ける");
    assert_close(px(&image, middle(&h)), to_bytes(albedo), 2, "Unlit 化");
    let mut look = lil();
    look.properties
        .insert("_LightMaxLimit".into(), LookValue::Float(0.3));
    set_look(&mut h, look);
    let image = h.render().expect("描ける");
    assert_close(
        px(&image, middle(&h)),
        to_bytes(albedo * 0.3),
        2,
        "明るさの上限",
    );
    // 標準へ戻すと PBR（lilToon の絵と違う）
    let mut back = lil();
    back.kind = LookKind::Standard;
    set_look(&mut h, back);
    let image = h.render().expect("描ける");
    assert_ne!(px(&image, middle(&h)), to_bytes(albedo * 0.3));
}

#[test]
fn the_shadow_follows_lilgetshading_from_lit_to_dark() {
    let mut h = view(900.0, 640.0, 64);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, 2.5);
    let color = [230u8, 200, 180, 255];
    fill_color(&mut h, color);
    let mut look = lil();
    look.properties
        .insert("_UseShadow".into(), LookValue::Float(1.0));
    set_look(&mut h, look);
    let albedo = lin([
        color[0] as f32 / 255.0,
        color[1] as f32 / 255.0,
        color[2] as f32 / 255.0,
    ]);
    // 光を横から後ろへ回す（板の法線は −Z）: 明るい側・境界・1 影・2 影
    for (yaw, pitch) in [
        (180.0f32, 0.0f32),
        (180.0, 70.0),
        (180.0, 88.0),
        (90.0, 0.0),
        (60.0, 0.0),
        (0.0, 0.0),
    ] {
        light(&mut h, yaw, pitch);
        let to_light = h.state().state.view3d.display.light_direction();
        let image = h.render().expect("描ける");
        let expected = to_bytes(shaded(albedo, Vec3::NEG_Z, to_light));
        assert_close(
            px(&image, middle(&h)),
            expected,
            3,
            &format!("光 {yaw}° {pitch}°"),
        );
    }
}

#[test]
fn a_user_channel_mask_turns_the_shadow_off_where_it_is_black() {
    let mut h = view(900.0, 640.0, 64);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, 2.5);
    light(&mut h, 0.0, 0.0); // 後ろから: 全部が影
    let color = [230u8, 200, 180, 255];
    fill_color(&mut h, color);
    let doc = &mut h.state_mut().state.doc;
    let mask = doc
        .add_channel(ChannelInfo {
            name: "影の強さ".into(),
            kind: ChannelKind::Scalar,
            color_space: ColorSpace::Linear,
            default: Rgba8::new(255, 255, 255, 255),
        })
        .unwrap();
    // 左半分に 0 を塗る（何も描いていない右半分は既定の 1）
    let layer = doc.add_layer("マスク").unwrap();
    doc.set_channel_enabled(layer, mask, true).unwrap();
    for y in 0..64 {
        for x in 0..32 {
            doc.set_channel_pixel(layer, mask, x, y, Rgba8::new(0, 0, 0, 255))
                .unwrap();
        }
    }
    let mut look = lil();
    look.properties
        .insert("_UseShadow".into(), LookValue::Float(1.0));
    look.textures
        .insert("_ShadowStrengthMask".into(), TextureSource::Channel(mask));
    set_look(&mut h, look);
    let image = h.render().expect("描ける");
    let rect = h.state().view3d_rect().unwrap();
    let c = rect.center();
    // 板は画面の真ん中に、幅は高さのおよそ 0.4 倍
    let half = rect.height() * 0.12;
    let albedo = lin([
        color[0] as f32 / 255.0,
        color[1] as f32 / 255.0,
        color[2] as f32 / 255.0,
    ]);
    let (_, lc) = lil_light(Vec3::Z);
    // 左（UV の u が小さい側が画面の左）: 強度 0 = 影なしで光の色
    assert_close(
        px(&image, egui::pos2(c.x - half, c.y)),
        to_bytes(albedo * lc),
        3,
        "マスク 0",
    );
    assert_close(
        px(&image, egui::pos2(c.x + half, c.y)),
        to_bytes(shaded(
            albedo,
            Vec3::NEG_Z,
            h.state().state.view3d.display.light_direction(),
        )),
        3,
        "マスク 1",
    );
    // 詰め合わせで同じ成分を読んでも同じ
    let mut look = lil();
    look.properties
        .insert("_UseShadow".into(), LookValue::Float(1.0));
    look.textures.insert(
        "_ShadowStrengthMask".into(),
        TextureSource::Packed([
            PlaneSource::Channel {
                channel: mask,
                component: 0,
            },
            PlaneSource::One,
            PlaneSource::One,
            PlaneSource::One,
        ]),
    );
    set_look(&mut h, look);
    let again = h.render().expect("描ける");
    assert_eq!(
        px(&again, egui::pos2(c.x - half, c.y)),
        px(&image, egui::pos2(c.x - half, c.y))
    );
}

#[test]
fn a_user_channel_mask_keeps_its_value_in_the_smaller_mips() {
    // 遠くから見ると、マスクは縮めた段（ミップ）から読む。配列の段を作り損ねると、塗った所も「何も描いていない」（既定）に見える
    let mut h = view(900.0, 640.0, 512);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, 10.0);
    light(&mut h, 0.0, 0.0); // 後ろから: 全部が影
    let color = [230u8, 200, 180, 255];
    fill_color(&mut h, color);
    let doc = &mut h.state_mut().state.doc;
    let mut masks = Vec::new();
    for name in ["影の強さ", "影の強さ 2"] {
        masks.push(
            doc.add_channel(ChannelInfo {
                name: name.into(),
                kind: ChannelKind::Scalar,
                color_space: ColorSpace::Linear,
                default: Rgba8::new(255, 255, 255, 255),
            })
            .unwrap(),
        );
    }
    // 2 つ目のレイヤー（配列のレイヤー 1）の左半分に 0 を塗る
    let layer = doc.add_layer("マスク").unwrap();
    doc.set_channel_enabled(layer, masks[1], true).unwrap();
    for y in 0..512 {
        for x in 0..256 {
            doc.set_channel_pixel(layer, masks[1], x, y, Rgba8::new(0, 0, 0, 255))
                .unwrap();
        }
    }
    let mut look = lil();
    look.properties
        .insert("_UseShadow".into(), LookValue::Float(1.0));
    look.textures
        .insert("_ShadowBlurMask".into(), TextureSource::Channel(masks[0]));
    look.textures.insert(
        "_ShadowStrengthMask".into(),
        TextureSource::Channel(masks[1]),
    );
    set_look(&mut h, look);
    let image = h.render().expect("描ける");
    let rect = h.state().view3d_rect().unwrap();
    let c = rect.center();
    // 板の幅は高さのおよそ 0.1 倍（512 の文書が 60 画素ほど: 3 段ほど縮めた段を読む）
    let quarter = rect.height() * 0.025;
    let albedo = lin([
        color[0] as f32 / 255.0,
        color[1] as f32 / 255.0,
        color[2] as f32 / 255.0,
    ]);
    let (_, lc) = lil_light(Vec3::Z);
    assert_close(
        px(&image, egui::pos2(c.x - quarter, c.y)),
        to_bytes(albedo * lc),
        3,
        "遠くのマスク 0",
    );
    assert_close(
        px(&image, egui::pos2(c.x + quarter, c.y)),
        to_bytes(shaded(
            albedo,
            Vec3::NEG_Z,
            h.state().state.view3d.display.light_direction(),
        )),
        3,
        "遠くのマスク 1",
    );
}

#[test]
fn a_packed_slot_reads_color_and_emission_as_their_painted_values() {
    // 詰め合わせ（成分ごと）で Color・Emission を読むスロットは、書き出しの「lilToon の詰め方」と同じくチャンネルの値のまま読む。
    // この 2 つは sRGB の形式のテクスチャで GPU がリニアにして読むので、ガンマの値へ戻さないと 128 が 0.216（= 55 を塗った
    // スカラー）になる。同じ 128 を塗ったスカラーのユーザーチャンネルを読んだときと同じ見た目
    let mut h = view(900.0, 640.0, 64);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, 2.5);
    light(&mut h, 0.0, 0.0); // 後ろから: 全部が影（影の強さのマスクが見た目を決める）
    let doc = &mut h.state_mut().state.doc;
    doc.add_fill_layer(
        "色",
        &[
            (Channel::Color, Rgba8::new(128, 200, 180, 255)),
            (Channel::Emission, Rgba8::new(128, 0, 0, 255)),
        ],
        None,
    )
    .unwrap();
    let mask = doc
        .add_channel(ChannelInfo {
            name: "影の強さ".into(),
            kind: ChannelKind::Scalar,
            color_space: ColorSpace::Linear,
            default: Rgba8::new(255, 255, 255, 255),
        })
        .unwrap();
    h.run();
    let at = middle(&h);
    let shadow_with = |h: &mut Harness<'_, YoluApp>, strength: TextureSource| {
        let mut look = lil();
        look.properties
            .insert("_UseShadow".into(), LookValue::Float(1.0));
        look.textures.insert("_ShadowStrengthMask".into(), strength);
        set_look(h, look);
        px(&h.render().expect("描ける"), at)
    };
    let packed = |channel: Channel| {
        TextureSource::Packed([
            PlaneSource::Channel {
                channel,
                component: 0,
            },
            PlaneSource::One,
            PlaneSource::One,
            PlaneSource::One,
        ])
    };
    let from_color = shadow_with(&mut h, packed(Channel::Color));
    let from_emission = shadow_with(&mut h, packed(Channel::Emission));
    let mut user = Vec::new();
    for v in [128u8, 55] {
        let doc = &mut h.state_mut().state.doc;
        doc.add_fill_layer("マスク", &[(mask, Rgba8::new(v, v, v, 255))], None)
            .unwrap();
        user.push(shadow_with(&mut h, TextureSource::Channel(mask)));
    }
    assert!(
        (0..3).any(|k| user[0][k].abs_diff(user[1][k]) >= 4),
        "マスク 128 と 55 で見た目が違う（試験が値の違いを見分けられる）: {user:?}"
    );
    assert_close(
        from_color,
        user[0],
        1,
        "詰め合わせで読んだ Color の R（128）",
    );
    assert_close(
        from_emission,
        user[0],
        1,
        "詰め合わせで読んだ Emission の R（128）",
    );
}

#[test]
fn a_matcap_image_is_decoded_only_when_it_is_srgb() {
    // マットキャップの画像は、色空間が sRGB（と未指定）なら sRGB の形式で持ち GPU がリニアにして読み、リニア（データ）なら値のまま
    // 読む（リニアの形式）。一様な 128 の画像を、マットキャップだけの色（ライト・メインカラーの影響なし、通常の合成・強さ 1）で見る:
    // sRGB ならリニア 0.216 → 画面の 128、リニアなら 0.502 → 画面の 188
    let mut h = view(900.0, 640.0, 64);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, 2.5);
    light(&mut h, 180.0, 0.0);
    fill_color(&mut h, [200, 120, 60, 255]);
    let id = yolu_core::ImageId(0xA11C_A900_0000_0000_0000_0000_0000_0002);
    let mut look = lil();
    for (name, v) in [
        ("_UseMatCap", 1.0),
        ("_MatCapBlend", 1.0),
        ("_MatCapBlendMode", 0.0),
        ("_MatCapEnableLighting", 0.0),
        ("_MatCapMainStrength", 0.0),
    ] {
        look.properties.insert(name.into(), LookValue::Float(v));
    }
    look.properties
        .insert("_MatCapColor".into(), LookValue::Color([1.0; 4]));
    look.textures
        .insert("_MatCapTex".into(), TextureSource::Image(id));
    set_look(&mut h, look);
    let at = middle(&h);
    for (space, expected) in [
        (ImageColorSpace::Srgb, 128u8),
        (ImageColorSpace::Linear, 188),
        (ImageColorSpace::Unspecified, 128),
    ] {
        let pixels = [128u8, 128, 128, 255].repeat(16);
        let image = ImageInput::new(4, 4, pixels, space).unwrap();
        let doc = &mut h.state_mut().state.doc;
        let inputs = doc.effect_inputs().clone().with_image(id, image);
        doc.set_effect_inputs(inputs).unwrap();
        h.run();
        let c = px(&h.render().expect("描ける"), at);
        assert_close(c, [expected; 3], 2, &format!("{space:?} の画像"));
    }
}

#[test]
fn cutout_discards_and_transparent_blends_over_the_background() {
    let mut h = view(900.0, 640.0, 64);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, 2.5);
    light(&mut h, 180.0, 0.0);
    fill_color(&mut h, [200, 120, 60, 64]);
    let at = middle(&h);
    let background = {
        let image = h.render().expect("描ける");
        let r = h.state().view3d_rect().unwrap();
        px(&image, egui::pos2(r.left() + 5.0, r.top() + 5.0))
    };
    // カットアウト（Cutoff 0.5 > α 0.25）: 捨てて背景
    let mut look = lil();
    look.shader = "Hidden/lilToonCutout".into();
    set_look(&mut h, look.clone());
    let image = h.render().expect("描ける");
    assert_close(px(&image, at), background, 1, "カットアウト");
    // Cutoff を下げると残る
    look.properties
        .insert("_Cutoff".into(), LookValue::Float(0.1));
    set_look(&mut h, look);
    let image = h.render().expect("描ける");
    assert!(px(&image, at) != background);
    // 半透明: 色 × α が背景に重なる（Unity と同じくリニアの空間で、乗算済み。描き先の sRGB の見え方を作れない機材（GL）はガンマの値で重ねる）
    let mut look = lil();
    look.shader = "Hidden/lilToonTransparent".into();
    look.properties
        .insert("_Cutoff".into(), LookValue::Float(0.001));
    set_look(&mut h, look);
    let image = h.render().expect("描ける");
    let linear = h.state().view3d_stats().unwrap().linear_transparent;
    let albedo = lin([200.0 / 255.0, 120.0 / 255.0, 60.0 / 255.0]);
    let (_, lc) = lil_light(Vec3::NEG_Z);
    let a = 64.0 / 255.0;
    let src = albedo * lc;
    let expected = [0, 1, 2].map(|k| {
        let mixed = if linear {
            let bg = brdf::srgb_to_linear(background[k] as f32 / 255.0);
            brdf::linear_to_srgb(src[k] * a + bg * (1.0 - a))
        } else {
            brdf::linear_to_srgb(src[k]) * a + background[k] as f32 / 255.0 * (1.0 - a)
        };
        (mixed.clamp(0.0, 1.0) * 255.0).round() as u8
    });
    assert_close(
        px(&image, at),
        expected,
        3,
        if linear {
            "半透明（リニア）"
        } else {
            "半透明（ガンマ）"
        },
    );
}

#[test]
fn the_alpha_mask_replaces_multiplies_adds_or_subtracts_the_alpha_like_liltoon() {
    // lilToon の OVERRIDE_ALPHAMASK: マスク = saturate(R × _AlphaMaskScale + _AlphaMaskValue)、モード 1 置き換え・2 乗算・3 加算・4 減算
    let mut h = view(900.0, 640.0, 64);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, 2.5);
    light(&mut h, 180.0, 0.0);
    let color = [200u8, 120, 60, 128];
    fill_color(&mut h, color);
    let rect = h.state().view3d_rect().unwrap();
    let background = px(
        &h.render().expect("描ける"),
        egui::pos2(rect.left() + 5.0, rect.top() + 5.0),
    );
    // アルファマスクのユーザーチャンネル: 左半分 0.2、右半分 0.8
    let doc = &mut h.state_mut().state.doc;
    let mask = doc
        .add_channel(ChannelInfo {
            name: "アルファマスク".into(),
            kind: ChannelKind::Scalar,
            color_space: ColorSpace::Linear,
            default: Rgba8::new(0, 0, 0, 255),
        })
        .unwrap();
    let layer = doc.add_layer("マスク").unwrap();
    doc.set_channel_enabled(layer, mask, true).unwrap();
    for y in 0..64 {
        for x in 0..64 {
            let v = if x < 32 { 51 } else { 204 };
            doc.set_channel_pixel(layer, mask, x, y, Rgba8::new(v, v, v, 255))
                .unwrap();
        }
    }
    let albedo = lin([
        color[0] as f32 / 255.0,
        color[1] as f32 / 255.0,
        color[2] as f32 / 255.0,
    ]);
    let (_, lc) = lil_light(Vec3::NEG_Z);
    let src = albedo * lc;
    let base = color[3] as f32 / 255.0;
    let (c, q) = (rect.center(), rect.height() * 0.12);
    for (mode, scale, offset) in [
        (1.0f32, 1.0f32, 0.0f32),
        (2.0, 1.0, 0.0),
        (3.0, 1.0, 0.0),
        (4.0, 1.0, 0.0),
        (1.0, -1.0, 1.0),
    ] {
        let mut look = lil();
        look.shader = "Hidden/lilToonTransparent".into();
        look.properties
            .insert("_Cutoff".into(), LookValue::Float(0.001));
        look.properties
            .insert("_AlphaMaskMode".into(), LookValue::Float(mode));
        look.properties
            .insert("_AlphaMaskScale".into(), LookValue::Float(scale));
        look.properties
            .insert("_AlphaMaskValue".into(), LookValue::Float(offset));
        look.textures
            .insert("_AlphaMask".into(), TextureSource::Channel(mask));
        set_look(&mut h, look);
        let image = h.render().expect("描ける");
        let linear = h.state().view3d_stats().unwrap().linear_transparent;
        for (x, m) in [(c.x - q, 0.2f32), (c.x + q, 0.8f32)] {
            let m = (m * scale + offset).clamp(0.0, 1.0);
            let a = match mode as i32 {
                1 => m,
                2 => base * m,
                3 => (base + m).min(1.0),
                _ => (base - m).max(0.0),
            };
            let expected = [0, 1, 2].map(|k| {
                let mixed = if linear {
                    let bg = brdf::srgb_to_linear(background[k] as f32 / 255.0);
                    brdf::linear_to_srgb(src[k] * a + bg * (1.0 - a))
                } else {
                    brdf::linear_to_srgb(src[k]) * a + background[k] as f32 / 255.0 * (1.0 - a)
                };
                (mixed.clamp(0.0, 1.0) * 255.0).round() as u8
            });
            assert_close(
                px(&image, egui::pos2(x, c.y)),
                expected,
                3,
                &format!("モード {mode}・スケール {scale}・オフセット {offset}・α {a:.3}"),
            );
        }
    }
}

#[test]
fn the_outline_draws_a_shell_outside_the_silhouette() {
    let mut h = view(900.0, 640.0, 64);
    set_model(&mut h, vec![cube_sphere(24, 0.5)]);
    look_at(&mut h, 3.0);
    light(&mut h, 180.0, 0.0);
    fill_color(&mut h, [255, 255, 255, 255]);
    let rect = h.state().view3d_rect().unwrap();
    let c = rect.center();
    // 球の右の縁の少し外を探す: 輪郭線なしでは背景
    let image = h.render().expect("描ける");
    let row = c.y.round() as u32;
    let edge = (c.x as u32..rect.right() as u32)
        .find(|x| {
            let p = image.get_pixel(*x, row).0;
            p[0] < 40 && p[1] < 40
        })
        .expect("球の縁");
    let outside = egui::pos2(edge as f32 + 2.0, c.y);
    let before = px(&image, outside);
    let mut look = lil();
    look.shader = "Hidden/lilToonOutline".into();
    look.properties.insert(
        "_OutlineColor".into(),
        LookValue::Color([1.0, 0.0, 0.0, 1.0]),
    );
    look.properties
        .insert("_OutlineWidth".into(), LookValue::Float(1.0));
    look.properties
        .insert("_OutlineFixWidth".into(), LookValue::Float(0.0));
    look.properties
        .insert("_OutlineEnableLighting".into(), LookValue::Float(0.0));
    set_look(&mut h, look);
    let image = h.render().expect("描ける");
    let after = px(&image, outside);
    assert_ne!(after, before);
    assert_close(after, [255, 0, 0], 2, "輪郭線の色（光を反映しない）");
    // 輪郭線の中（面の上）は面の色のまま
    assert!(px(&image, c)[1] > 100);
}

#[test]
fn undo_takes_the_look_back_and_the_view_follows() {
    let mut h = view(900.0, 640.0, 64);
    start_standard(&mut h, 0);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, 2.5);
    light(&mut h, 180.0, 0.0);
    fill_color(&mut h, [200, 120, 60, 255]);
    let standard = px(&h.render().expect("描ける"), middle(&h));
    set_look(&mut h, lil());
    let toon = px(&h.render().expect("描ける"), middle(&h));
    assert_ne!(standard, toon);
    h.state_mut().apply(Action::Undo);
    h.run();
    assert_eq!(px(&h.render().expect("描ける"), middle(&h)), standard);
    h.state_mut().apply(Action::Redo);
    h.run();
    assert_eq!(px(&h.render().expect("描ける"), middle(&h)), toon);
}

/// ソフトの描画（アダプタが CPU）は使う機能ごとにパイプラインを作り、同じ組み合わせへ戻るときは作り直さない。組み合わせを次々に変えても、
/// 持つ数は上限（48）で止まる。実機は全部入りの 1 本なので、機能を入切しても作らない。
#[test]
fn the_pipelines_per_feature_set_are_reused_and_kept_under_the_cap() {
    let mut h = view(480.0, 360.0, 64);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, 2.5);
    set_look(&mut h, lil());
    let software = h
        .state()
        .view3d_adapter()
        .unwrap_or_default()
        .contains("Cpu");
    let stats = |h: &Harness<'_, YoluApp>| h.state().view3d_stats().expect("3D ビュー");
    let with = |on: &[&str]| {
        let mut look = lil();
        for name in on {
            look.properties
                .insert((*name).into(), LookValue::Float(1.0));
        }
        look
    };
    let before = stats(&h).lil_pipeline_builds;
    set_look(&mut h, with(&["_UseShadow"]));
    let made = stats(&h).lil_pipeline_builds;
    if software {
        assert!(made > before, "影を入れた組み合わせを作る");
    } else {
        assert_eq!(made, before, "実機は全部入りの 1 本");
    }
    set_look(&mut h, lil());
    set_look(&mut h, with(&["_UseShadow"]));
    assert_eq!(
        stats(&h).lil_pipeline_builds,
        made,
        "同じ組み合わせへ戻るときは作り直さない"
    );
    let toggles = [
        "_UseShadow",
        "_UseRimShade",
        "_UseBacklight",
        "_UseReflection",
        "_UseRim",
        "_UseGlitter",
    ];
    for bits in 0..(1u32 << toggles.len()) {
        let on: Vec<&str> = toggles
            .iter()
            .enumerate()
            .filter(|(i, _)| bits >> i & 1 == 1)
            .map(|(_, name)| *name)
            .collect();
        set_look(&mut h, with(&on));
        assert!(
            stats(&h).lil_pipelines <= 48,
            "持つ数: {}",
            stats(&h).lil_pipelines
        );
    }
    if software {
        assert!(
            stats(&h).lil_pipeline_builds >= made + 60,
            "64 通りを作った"
        );
    } else {
        assert_eq!(stats(&h).lil_pipelines, 1);
    }
}

// ───────── ユーザーチャンネルの配列と 3D ビューの予算・ほかのセットの lilToon ─────────

/// 文書に影の強度のマスク（スカラーのユーザーチャンネル。左半分に 0）を足し、lilToon（影を入）でそれを読む見た目を返す。
/// `extra` 個のユーザーチャンネルも、影のぼかしのマスクなどに割り当てる（配列のレイヤーを増やす）。
fn masked_shadow(doc: &mut yolu_core::Document, extra: usize) -> MaterialLook {
    let size = doc.width();
    let mask = doc
        .add_channel(ChannelInfo {
            name: "影の強さ".into(),
            kind: ChannelKind::Scalar,
            color_space: ColorSpace::Linear,
            default: Rgba8::new(255, 255, 255, 255),
        })
        .unwrap();
    let layer = doc.add_layer("マスク").unwrap();
    doc.set_channel_enabled(layer, mask, true).unwrap();
    for y in 0..size {
        for x in 0..size / 2 {
            doc.set_channel_pixel(layer, mask, x, y, Rgba8::new(0, 0, 0, 255))
                .unwrap();
        }
    }
    let mut look = lil();
    look.properties
        .insert("_UseShadow".into(), LookValue::Float(1.0));
    look.textures
        .insert("_ShadowStrengthMask".into(), TextureSource::Channel(mask));
    for (i, slot) in ["_ShadowBlurMask", "_ShadowBorderMask"]
        .iter()
        .take(extra)
        .enumerate()
    {
        let c = doc
            .add_channel(ChannelInfo {
                name: format!("ほか {i}"),
                kind: ChannelKind::Scalar,
                color_space: ColorSpace::Linear,
                default: Rgba8::new(255, 255, 255, 255),
            })
            .unwrap();
        look.textures
            .insert((*slot).into(), TextureSource::Channel(c));
    }
    look
}

/// 標準のチャンネルが Color だけの絵の 1 テクセルのバイト数: Color の 4 B と、塗り広げるときに持つ重みの絵の 1 B。
const COLOR_AND_WEIGHT: u64 = 5;

/// ミップ込みのバイト数（`view3d::paint::mip_bytes` と同じ数え方）。
fn mip_bytes(side: u32, per_texel: u64) -> u64 {
    let mut total = 0u64;
    let mut s = side;
    loop {
        total += s as u64 * s as u64;
        if s == 1 {
            break;
        }
        s = (s / 2).max(1);
    }
    total * per_texel
}

#[test]
fn the_user_channel_array_is_in_the_view_budget_and_shrinks_to_fit() {
    let mut h = view(900.0, 640.0, 256);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, 2.5);
    light(&mut h, 0.0, 0.0); // 後ろから: 全部が影
    let color = [230u8, 200, 180, 255];
    fill_color(&mut h, color);
    let look = masked_shadow(&mut h.state_mut().state.doc, 1);
    set_look(&mut h, look);
    let s = h.state().view3d_stats().unwrap();
    assert_eq!(
        s.paint_bytes,
        mip_bytes(256, COLOR_AND_WEIGHT),
        "標準のチャンネルは Color だけ"
    );
    assert_eq!(
        (s.user_level, s.user_bytes),
        (0, mip_bytes(256, 8)),
        "2 レイヤーの配列を文書の大きさで: {s:?}"
    );
    assert!(
        s.peak_bytes >= s.paint_bytes + s.user_bytes,
        "持っている絵の数に入る: {s:?}"
    );
    let full = h.render().expect("描ける");
    // 全体の予算を、標準のチャンネルの絵と 128² の配列が入る分にする: 配列だけが縮む（標準のチャンネルが先）
    let budget = mip_bytes(256, COLOR_AND_WEIGHT) + mip_bytes(128, 8);
    h.state_mut().view3d_set_paint_budget(budget);
    h.run();
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!(s.paint_level, 0);
    assert_eq!(
        (s.user_level, s.user_bytes),
        (1, mip_bytes(128, 8)),
        "{s:?}"
    );
    assert!(s.peak_bytes <= budget, "{s:?}");
    // 縮めてもマスクは効く（左は影なし、右は影）
    let image = h.render().expect("描ける");
    let rect = h.state().view3d_rect().unwrap();
    let (c, q) = (rect.center(), rect.height() * 0.12);
    for x in [c.x - q, c.x + q] {
        assert_close(
            px(&image, egui::pos2(x, c.y)),
            px(&full, egui::pos2(x, c.y)),
            2,
            "縮めた配列",
        );
    }
    assert!(px(&image, egui::pos2(c.x - q, c.y))[0] > px(&image, egui::pos2(c.x + q, c.y))[0] + 10);
    // 標準のチャンネルの絵で予算が尽きても、配列は 1 × 1 まで縮めて持つ（標準のチャンネルの絵と同じ決まり）
    h.state_mut()
        .view3d_set_paint_budget(mip_bytes(256, COLOR_AND_WEIGHT));
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!(
        (s.paint_level, s.user_level, s.user_bytes),
        (0, 8, 2 * 4),
        "{s:?}"
    );
}

/// 横に並べた 2 枚の板（マテリアル 0・1。セット i = マテリアル i、文書は size × size）を、マテリアルの表示で見る。
fn two_sets(size: u32) -> Harness<'static, YoluApp> {
    use yolu_protocol::{
        channel, ChannelRoute, MaterialInfo, MaterialKey, MeshData, Model, Submesh as Sub,
        TextureProperty,
    };
    // （3D の表示域の幅が前の既定の並びと同じになるよう、右の列を広げた分だけウィンドウも広げる）
    let mut h = view(1340.0, 640.0, size);
    let materials = (0..2)
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
    let meshes = (0..2)
        .map(|i| {
            let x = if i == 0 { -0.65 } else { 0.65 };
            MeshData {
                key: format!("{i}"),
                name: format!("板{i}"),
                skinned: false,
                positions: vec![
                    [x - 0.5, -0.5, 0.0],
                    [x + 0.5, -0.5, 0.0],
                    [x - 0.5, 0.5, 0.0],
                    [x + 0.5, 0.5, 0.0],
                ],
                normals: vec![],
                uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
                submeshes: vec![Sub {
                    material: i,
                    indices: vec![0, 2, 1, 2, 3, 1],
                }],
            }
        })
        .collect();
    h.state_mut()
        .load_live_link_model(&Model {
            generation: 1,
            name: "板".into(),
            materials,
            meshes,
        })
        .unwrap();
    h.run();
    look_at(&mut h, 3.2);
    assert_eq!(h.state().state.sets.len(), 2);
    h
}

/// セットの文書の一辺（正方形）。
fn side_of(h: &Harness<'_, YoluApp>, set: usize) -> u32 {
    let doc = h.state().state.set_doc(set);
    assert_eq!(doc.width(), doc.height());
    doc.width()
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

fn fill_set(h: &mut Harness<'_, YoluApp>, set: usize, color: [u8; 4]) {
    h.state_mut()
        .state
        .set_doc_mut(set)
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

/// 板 1 の左と右の 4 分の 1 の所の画素（左はマスク 0）。
fn set1_halves(h: &mut Harness<'_, YoluApp>) -> ([u8; 3], [u8; 3]) {
    let image = h.render().expect("描ける");
    (
        px(&image, screen_of(h, Vec3::new(0.65 - 0.25, 0.0, 0.0))),
        px(&image, screen_of(h, Vec3::new(0.65 + 0.25, 0.0, 0.0))),
    )
}

#[test]
fn another_sets_liltoon_look_is_drawn_and_follows_its_changes() {
    let mut h = two_sets(64);
    light(&mut h, 0.0, 0.0); // 後ろから: 全部が影
    for i in 0..2 {
        fill_set(&mut h, i, [230, 200, 180, 255]);
    }
    let look = masked_shadow(h.state_mut().state.set_doc_mut(1), 0);
    h.state_mut()
        .state
        .set_doc_mut(1)
        .set_look(look.clone(), false)
        .unwrap();
    // セット 1 を今のセットにして描いた絵を基準に
    h.state_mut().state.switch_set(1).unwrap();
    h.run();
    let (lit, dark) = set1_halves(&mut h);
    assert!(
        lit[0] > dark[0] + 10,
        "マスク 0 の所は影なし: {lit:?} {dark:?}"
    );
    // 今のセットを 0 に替えても、セット 1 の面は lilToon（ユーザーチャンネルのマスクも）のまま
    h.state_mut().state.switch_set(0).unwrap();
    h.run();
    assert_eq!(h.state().view3d_held_materials(), vec![1]);
    let (l, d) = set1_halves(&mut h);
    assert_close(l, lit, 3, "ほかのセットの影なしの所");
    assert_close(d, dark, 3, "ほかのセットの影の所");
    let s = h.state().view3d_stats().unwrap();
    let side = side_of(&h, 1);
    assert_eq!(
        s.other_bytes,
        mip_bytes(side, COLOR_AND_WEIGHT) + mip_bytes(side, 8),
        "ほかのセットの配列も数える: {s:?}"
    );
    assert_eq!(s.user_bytes, 0, "今のセット（標準）は配列を持たない");
    // ほかのセットのままで見た目を変えると、3D ビューが追う（影の強度 0 で右も影なし）
    let mut weaker = look.clone();
    weaker
        .properties
        .insert("_ShadowStrength".into(), LookValue::Float(0.0));
    h.state_mut()
        .state
        .set_doc_mut(1)
        .set_look(weaker, false)
        .unwrap();
    h.run();
    let (l, d) = set1_halves(&mut h);
    assert_close(d, l, 2, "影の強度 0");
    assert_close(l, lit, 3, "影なしの所は同じ");
    // 標準へ戻すと、ほかのセットの配列を手放す
    let mut standard = look;
    standard.kind = LookKind::Standard;
    h.state_mut()
        .state
        .set_doc_mut(1)
        .set_look(standard, false)
        .unwrap();
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!(s.other_bytes, mip_bytes(side, COLOR_AND_WEIGHT), "{s:?}");
}

#[test]
fn another_set_is_held_only_when_its_picture_and_user_channels_both_fit() {
    let mut h = two_sets(64);
    light(&mut h, 0.0, 0.0);
    for i in 0..2 {
        fill_set(&mut h, i, [230, 200, 180, 255]);
    }
    let look = masked_shadow(h.state_mut().state.set_doc_mut(1), 0);
    h.state_mut()
        .state
        .set_doc_mut(1)
        .set_look(look, false)
        .unwrap();
    h.run();
    // 今のセット 0（Color だけ）、ほかのセット 1 は Color と 2 レイヤーの配列
    let (side0, side1) = (side_of(&h, 0), side_of(&h, 1));
    let need = mip_bytes(side0, COLOR_AND_WEIGHT)
        + mip_bytes(side1, COLOR_AND_WEIGHT)
        + mip_bytes(side1, 8);
    h.state_mut().view3d_set_paint_budget(need - 1);
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!(
        (s.other_sets, s.other_skipped),
        (0, 1),
        "標準のチャンネルの絵だけなら入るが、配列が入らない: {s:?}"
    );
    assert_eq!(h.state().state.view3d.unpainted, vec![1]);
    // 同期の途中の最大は直前のフレームの終わりの分から数えるので、予算を下げた次のフレームで見る
    h.step();
    let s = h.state().view3d_stats().unwrap();
    assert!(s.peak_bytes < need, "{s:?}");
    h.state_mut().view3d_set_paint_budget(need);
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!((s.other_sets, s.other_skipped), (1, 0), "{s:?}");
    assert_eq!(
        s.other_bytes,
        mip_bytes(side1, COLOR_AND_WEIGHT) + mip_bytes(side1, 8)
    );
    assert!(s.peak_bytes <= need, "{s:?}");
}

// ───────── Live Link で Unity から受けた絵 ─────────

/// 一色の受けた絵（`side` × `side`、straight の RGBA8）。
fn solid(side: u32, rgba: [u8; 4], srgb: bool) -> std::sync::Arc<ReceivedImage> {
    std::sync::Arc::new(ReceivedImage {
        width: side,
        height: side,
        srgb,
        pixels: rgba.repeat((side * side) as usize).into(),
    })
}

/// Unity から受けた見た目（`look`）と絵。
fn received_with(
    look: MaterialLook,
    images: &[(&str, std::sync::Arc<ReceivedImage>)],
) -> ReceivedLook {
    let mut r = ReceivedLook {
        look,
        ..ReceivedLook::default()
    };
    for (slot, image) in images {
        r.images.insert((*slot).into(), image.clone());
    }
    r
}

fn set_received(h: &mut Harness<'_, YoluApp>, set: usize, r: Option<ReceivedLook>) {
    h.state_mut()
        .state
        .set_doc_mut(set)
        .set_received_look(r)
        .unwrap();
    h.run();
}

fn lil_received() -> MaterialLook {
    MaterialLook {
        kind: LookKind::LilToon,
        ..MaterialLook::default()
    }
}

#[test]
fn a_texture_received_from_unity_draws_its_slot_and_a_channel_assigned_here_wins() {
    let mut h = view(900.0, 640.0, 64);
    start_standard(&mut h, 0);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, 2.5);
    light(&mut h, 180.0, 0.0);
    let (_, lc) = lil_light(Vec3::NEG_Z);
    let albedo = |c: [u8; 4]| {
        lin([
            c[0] as f32 / 255.0,
            c[1] as f32 / 255.0,
            c[2] as f32 / 255.0,
        ])
    };
    // メインカラーを Unity から受けた絵で描く（流し込み先でない。sRGB の絵はリニアへ直して読む）
    let orange = [200u8, 120, 60, 255];
    set_received(
        &mut h,
        0,
        Some(received_with(
            lil_received(),
            &[("_MainTex", solid(4, orange, true))],
        )),
    );
    let image = h.render().expect("描ける");
    assert_close(
        px(&image, middle(&h)),
        to_bytes(albedo(orange) * lc),
        2,
        "受けた絵",
    );
    let s = h.state().view3d_stats().unwrap();
    assert_eq!(
        (s.received_bytes, s.received_size),
        (mip_bytes(4, 8), [4, 4]),
        "2 レイヤー（GL）で持つ: {s:?}"
    );
    // 欄で Color を割り当てると、チャンネルが勝つ（受けた絵の配列は手放す）
    let green = [60u8, 180, 90, 255];
    fill_color(&mut h, green);
    set_look(&mut h, lil());
    let image = h.render().expect("描ける");
    assert_close(
        px(&image, middle(&h)),
        to_bytes(albedo(green) * lc),
        2,
        "割り当てたチャンネル",
    );
    assert_eq!(h.state().view3d_stats().unwrap().received_bytes, 0);
    // 割り当てを外すと、また受けた絵
    let mut unassigned = lil();
    unassigned.textures.clear();
    set_look(&mut h, unassigned);
    let image = h.render().expect("描ける");
    assert_close(
        px(&image, middle(&h)),
        to_bytes(albedo(orange) * lc),
        2,
        "割り当てを外した",
    );
}

#[test]
fn an_emission_texture_unity_left_empty_glows_from_the_default_white_not_the_unused_channel() {
    let mut h = view(900.0, 640.0, 64);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, 2.5);
    light(&mut h, 180.0, 0.0);
    fill_color(&mut h, [0, 0, 0, 255]);
    // 新しいセットの既定の割り当て（発光 → Emission）。どのレイヤーも Emission を使っていない。発光の色はオレンジ
    let mut look = lil();
    yolu_app::look::default_textures(&mut look);
    look.properties
        .insert("_UseEmission".into(), LookValue::Float(1.0));
    look.properties.insert(
        "_EmissionColor".into(),
        LookValue::Color([1.0, 0.5, 0.2, 1.0]),
    );
    set_look(&mut h, look);
    // Live Link でつないでいない: 使っていないチャンネルの値（黒）を読むので光らない
    let unconnected = px(&h.render().expect("描ける"), middle(&h));
    assert!(unconnected[0] < 40, "{unconnected:?}");
    // Unity が発光のテクスチャを空と知らせた（絵も理由も来ない）: Unity は空を既定の白で読むので、発光の色で光る
    let mut received_look = lil_received();
    received_look
        .textures
        .insert("_MainTex".into(), TextureSource::Channel(Channel::Color));
    set_received(&mut h, 0, Some(received_with(received_look.clone(), &[])));
    let empty = px(&h.render().expect("描ける"), middle(&h));
    assert!(
        empty[0] > 200 && empty[0] > empty[1] && empty[1] > empty[2],
        "発光の色（1, 0.5, 0.2）で光る: {empty:?}"
    );
    // 読めない絵（理由つき）の間も同じ
    let mut pending = received_with(received_look, &[]);
    pending.missing.insert(
        "_EmissionMap".into(),
        yolu_core::look::MissingImage::Unreadable,
    );
    set_received(&mut h, 0, Some(pending));
    assert_close(
        px(&h.render().expect("描ける"), middle(&h)),
        empty,
        1,
        "届くまで",
    );
    // 絵が届けば、その絵（黒）が勝つ
    set_received(
        &mut h,
        0,
        Some(received_with(
            lil_received(),
            &[("_EmissionMap", solid(4, [0, 0, 0, 255], true))],
        )),
    );
    let black = px(&h.render().expect("描ける"), middle(&h));
    assert!(black[0] < 40, "{black:?}");
}

#[test]
fn a_received_normal_map_reads_x_from_alpha_times_red_and_keeps_green_where_alpha_is_zero() {
    let mut h = view(900.0, 640.0, 64);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, 2.5);
    light(&mut h, 180.0, 88.0); // ほぼ真上から: 平らな面は影の境界、法線の傾きがよく見える
    fill_color(&mut h, [230, 200, 180, 255]);
    let mut look = lil();
    look.properties
        .insert("_UseShadow".into(), LookValue::Float(1.0));
    look.properties
        .insert("_UseBumpMap".into(), LookValue::Float(1.0));
    set_look(&mut h, look);
    let draw = |h: &mut Harness<'_, YoluApp>, normal: [u8; 4]| {
        set_received(
            h,
            0,
            Some(received_with(
                lil_received(),
                &[("_BumpMap", solid(4, normal, false))],
            )),
        );
        px(&h.render().expect("描ける"), middle(h))
    };
    // RGB の絵（A は 1、X は R）と、DXT5nm（R は 1、X は A）の同じ法線は同じ絵
    let rgb = draw(&mut h, [51, 200, 255, 255]);
    let nm = draw(&mut h, [255, 200, 0, 51]);
    assert_close(nm, rgb, 1, "DXT5nm の X = A × R");
    // A が 0（X = 0）でも G（Y）を読む（乗算済みにすると Y が 0 に落ちる）
    let rgb0 = draw(&mut h, [0, 200, 255, 255]);
    let nm0 = draw(&mut h, [255, 200, 0, 0]);
    assert_close(nm0, rgb0, 1, "A が 0 の DXT5nm の Y");
    let lost = draw(&mut h, [0, 0, 255, 255]);
    assert!(
        (0..3).any(|k| lost[k].abs_diff(rgb0[k]) > 10),
        "Y が違えば絵も違う（試験が Y を見ている）: {lost:?} {rgb0:?}"
    );
    set_received(&mut h, 0, None);
    let flat = px(&h.render().expect("描ける"), middle(&h));
    assert!(
        (0..3).any(|k| flat[k].abs_diff(rgb[k]) > 10),
        "法線マップが効いている: {flat:?} {rgb:?}"
    );
}

#[test]
fn a_unity_texture_past_the_layer_limit_draws_the_slot_default() {
    use yolu_app::look::liltoon::{SlotDefault, SLOTS};
    let mut h = view(900.0, 640.0, 64);
    set_model(&mut h, vec![quad(1.0)]);
    look_at(&mut h, 2.5);
    light(&mut h, 180.0, 0.0);
    // 利用者の設定は値だけ（スロットは割り当てない: メインカラーも受けた絵）
    let mut look = lil();
    look.textures.clear();
    look.properties
        .insert("_UseMatCap2nd".into(), LookValue::Float(1.0));
    look.properties
        .insert("_MatCap2ndBlendMode".into(), LookValue::Float(0.0));
    set_look(&mut h, look);
    // メインカラーと、その後ろのスロットに既定と同じ値の絵（描く絵は変わらない）を `fillers` 枚。マットキャップ 2nd（並びの 17 番目）に赤
    let neutral = |d: SlotDefault| d.rgba().map(|v| (v * 255.0).round() as u8);
    let draw = |h: &mut Harness<'_, YoluApp>, fillers: usize, red: bool| {
        let mut images: Vec<(&str, std::sync::Arc<ReceivedImage>)> =
            vec![("_MainTex", solid(2, [200, 120, 60, 255], true))];
        images.extend(
            SLOTS[1..16]
                .iter()
                .take(fillers)
                .map(|s| (s.name, solid(2, neutral(s.default), false))),
        );
        if red {
            images.push(("_MatCap2ndTex", solid(2, [255, 0, 0, 255], true)));
        }
        set_received(h, 0, Some(received_with(lil_received(), &images)));
        px(&h.render().expect("描ける"), middle(h))
    };
    let white = draw(&mut h, 15, false);
    // 前に 15 枚: 16 枚目なので受けた絵で描く
    let red_drawn = draw(&mut h, 14, true);
    assert!(
        red_drawn[1] + 30 < white[1],
        "16 枚目までは受けた絵で描く: {red_drawn:?} {white:?}"
    );
    assert!(!yolu_app::look::panel::slot_over_received_limit(
        &h.state().state.doc,
        "_MatCap2ndTex"
    ));
    // 前に 16 枚あると、17 枚目は持たず既定（白）で描く
    let dropped = draw(&mut h, 15, true);
    assert_close(dropped, white, 1, "17 枚目は既定");
    assert!(yolu_app::look::panel::slot_over_received_limit(
        &h.state().state.doc,
        "_MatCap2ndTex"
    ));
    let s = h.state().view3d_stats().unwrap();
    assert_eq!(
        s.received_bytes,
        mip_bytes(2, 4 * 16),
        "レイヤーは 16 まで: {s:?}"
    );
}

#[test]
fn received_textures_are_in_the_view_budget_for_the_current_and_the_other_sets() {
    let mut h = two_sets(64);
    light(&mut h, 180.0, 0.0);
    let images = [
        ("_ShadowColorTex", solid(32, [40, 20, 60, 255], true)),
        ("_MatCapTex", solid(32, [255, 0, 0, 255], true)),
    ];
    for i in 0..2 {
        fill_set(&mut h, i, [230, 200, 180, 255]);
        let mut look = lil_received();
        look.properties
            .insert("_UseShadow".into(), LookValue::Float(1.0));
        look.properties
            .insert("_UseMatCap".into(), LookValue::Float(1.0));
        look.properties
            .insert("_MatCapBlendMode".into(), LookValue::Float(0.0));
        look.textures
            .insert("_MainTex".into(), TextureSource::Channel(Channel::Color));
        set_received(&mut h, i, Some(received_with(look, &images)));
    }
    let (side0, side1) = (side_of(&h, 0), side_of(&h, 1));
    let received = mip_bytes(32, 8);
    let s = h.state().view3d_stats().unwrap();
    assert_eq!(
        (s.received_bytes, s.received_size),
        (received, [32, 32]),
        "{s:?}"
    );
    assert_eq!(
        s.other_bytes,
        mip_bytes(side1, COLOR_AND_WEIGHT) + received,
        "ほかのセットの受けた絵も数える: {s:?}"
    );
    // ほかのセットは、標準のチャンネルの絵と受けた絵の配列の両方が入るときだけ持つ
    let need = mip_bytes(side0, COLOR_AND_WEIGHT)
        + received
        + mip_bytes(side1, COLOR_AND_WEIGHT)
        + received;
    h.state_mut().view3d_set_paint_budget(need - 1);
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!((s.other_sets, s.other_skipped), (0, 1), "{s:?}");
    assert_eq!(s.received_bytes, received, "今のセットが先: {s:?}");
    h.step();
    let s = h.state().view3d_stats().unwrap();
    assert!(s.peak_bytes < need, "{s:?}");
    h.state_mut().view3d_set_paint_budget(need);
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!((s.other_sets, s.other_skipped), (1, 0), "{s:?}");
    assert!(s.peak_bytes <= need, "{s:?}");
    // 今のセットの受けた絵の配列は、標準のチャンネルの絵の残りに収まるまで縮める（ほかのセットは持たない）
    let budget = mip_bytes(side0, COLOR_AND_WEIGHT) + mip_bytes(16, 8);
    h.state_mut().view3d_set_paint_budget(budget);
    h.run();
    h.step();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!(
        (s.received_bytes, s.received_size),
        (mip_bytes(16, 8), [16, 16]),
        "{s:?}"
    );
    assert_eq!(s.other_sets, 0, "{s:?}");
    assert!(s.peak_bytes <= budget, "{s:?}");
    // 縮めても受けた絵で描く（マットキャップの赤）
    let image = h.render().expect("描ける");
    let c = px(&image, screen_of(&h, Vec3::new(-0.65, 0.0, 0.0)));
    assert!(c[0] > c[1] + 30, "{c:?}");
}

#[test]
fn the_liltoon_values_are_built_only_when_the_look_or_document_changes() {
    let mut h = two_sets(64);
    start_standard(&mut h, 0);
    start_standard(&mut h, 1);
    light(&mut h, 0.0, 0.0);
    for i in 0..2 {
        fill_set(&mut h, i, [230, 200, 180, 255]);
    }
    let builds = |h: &Harness<'_, YoluApp>| h.state().view3d_stats().unwrap().look_params_builds;
    // 標準の見た目のセットだけなら、値を作らない（カメラを回しても）。数は作った回数の合計なので、標準にした後からの増えを見る
    let base = builds(&h);
    for yaw in [0.1f32, 0.2, 0.3] {
        h.state_mut().state.view3d.camera.yaw = yaw;
        h.run();
    }
    assert_eq!(builds(&h), base);
    // lilToon にすると作る。カメラを回すだけのフレームでは作り直さない
    let look = masked_shadow(h.state_mut().state.set_doc_mut(1), 0);
    h.state_mut()
        .state
        .set_doc_mut(1)
        .set_look(look.clone(), false)
        .unwrap();
    h.run();
    let after = builds(&h);
    assert!(after > base);
    let renders = h.state().view3d_stats().unwrap().renders;
    for yaw in [0.4f32, 0.5, 0.6] {
        h.state_mut().state.view3d.camera.yaw = yaw;
        h.run();
    }
    assert!(
        h.state().view3d_stats().unwrap().renders > renders,
        "描いてはいる"
    );
    assert_eq!(builds(&h), after);
    // 値を変えると作り直す
    let mut changed = look;
    changed
        .properties
        .insert("_ShadowBorder".into(), LookValue::Float(0.3));
    h.state_mut()
        .state
        .set_doc_mut(1)
        .set_look(changed, false)
        .unwrap();
    h.run();
    assert!(builds(&h) > after);
}

/// 計測: 標準（マテリアル (PBR)）と lilToon の 1 フレームの時間。球（4,800・71,148・307,200 三角形）に 2048² の文書（Color・Roughness・Normal の
/// 塗りつぶし）を貼り、カメラを回すだけ（絵は同じ）の 1 フレームを回して GPU の完了まで待った時間（全体）と `prepare` の CPU の時間を取る。
/// 見た目は 3 つ: 標準、lilToon（メインカラーと影）、lilToon（影・ノーマルマップ・マットキャップ 2 つ・リム・発光 2 つ・輪郭線と、ひな形が作る
/// マスクのユーザーチャンネル 9 枚）。同じ実行の中で見た目を交互に 3 回ずつ（1 回は 20 フレーム）測る。環境はスタジオ、影なし。
/// `cargo test -p yolu-app --test gui_view3d liltoon::measure -- --ignored --nocapture`
#[test]
#[ignore = "計測"]
fn measure_frames_standard_and_liltoon() {
    use std::time::Instant;
    use yolu_app::look::LookOp;
    use yolu_app::view3d::display::Shading;

    let mut h = view(1280.0, 800.0, 2048);
    println!("GPU: {}", h.state().view3d_adapter().unwrap_or_default());
    op(&mut h, Op::Shading(Shading::Material));
    op(&mut h, Op::Env(EnvKind::Studio));
    h.state_mut()
        .state
        .doc
        .add_fill_layer(
            "値",
            &[
                (Channel::Color, Rgba8::new(200, 120, 60, 255)),
                (Channel::Roughness, Rgba8::new(120, 120, 120, 255)),
                (Channel::Normal, Rgba8::new(140, 120, 240, 255)),
            ],
            None,
        )
        .unwrap();
    // 3 つの見た目を作る（文書の Undo とは別に、そのまま差し替える）
    let standard = h.state().state.doc.look().clone();
    let simple = {
        let mut look = lil();
        look.properties
            .insert("_UseShadow".into(), LookValue::Float(1.0));
        look
    };
    let full = {
        let state = &mut h.state_mut().state;
        state.apply(Action::Look(LookOp::Kind(LookKind::LilToon)));
        for toggle in [
            "_UseShadow",
            "_UseBumpMap",
            "_UseMatCap",
            "_UseMatCap2nd",
            "_UseRim",
            "_UseEmission",
            "_UseEmission2nd",
        ] {
            state.apply(Action::Look(LookOp::Value {
                name: toggle,
                value: LookValue::Float(1.0),
                drag: false,
            }));
        }
        state.apply(Action::Look(LookOp::Outline(true)));
        state.apply(Action::Look(LookOp::Template));
        // マスクのチャンネルにも絵を置く（GPU にレイヤーを持たせる）
        let users: Vec<Channel> = state
            .doc
            .channels()
            .into_iter()
            .filter(|c| !c.is_standard())
            .collect();
        assert_eq!(users.len(), 9, "ひな形のマスク");
        let fills: Vec<(Channel, Rgba8)> = users
            .iter()
            .map(|c| (*c, Rgba8::new(180, 180, 180, 255)))
            .collect();
        state.doc.add_fill_layer("マスク", &fills, None).unwrap();
        state.doc.look().clone()
    };
    let looks = [
        ("標準", standard),
        ("lilToon（影）", simple),
        ("lilToon（全部・マスク 9）", full),
    ];
    // 測る格子は `LIL_MEASURE_GRIDS`（例 `20,77`）で絞れる
    let grids: Vec<u32> = std::env::var("LIL_MEASURE_GRIDS")
        .ok()
        .map(|g| g.split(',').filter_map(|v| v.trim().parse().ok()).collect())
        .unwrap_or_else(|| vec![20, 77, 160]);
    for grid in grids {
        let mesh = cube_sphere(grid, 0.5);
        let triangles: usize = mesh.submeshes.iter().map(|s| s.indices.len() / 3).sum();
        set_model(&mut h, vec![mesh]);
        h.state_mut().state.view3d.camera = OrbitCamera {
            target: Vec3::ZERO,
            yaw: 20.0,
            pitch: 10.0,
            distance: 1.6,
            model_radius: 1.0,
            ..Default::default()
        };
        let mut sums = vec![(Vec::new(), Vec::new(), Vec::new()); looks.len()];
        // 見た目を切り替えたあと落ち着くまで（1 回目はソフトの描画ならパイプラインを作る。2 回目からは作ったものを使う）
        let mut switches = vec![Vec::new(); looks.len()];
        for _round in 0..3 {
            for (k, (_, look)) in looks.iter().enumerate() {
                let started = Instant::now();
                h.state_mut()
                    .state
                    .doc
                    .set_look(look.clone(), false)
                    .unwrap();
                h.state_mut().state.sync_view3d();
                h.run();
                h.state().view3d_wait_gpu();
                switches[k].push(started.elapsed().as_micros() as f64 / 1000.0);
                let (mut wall, mut cpu) = (Vec::new(), Vec::new());
                let ticks_before = process_cpu_ms();
                for frame in 0..24 {
                    h.state_mut().state.view3d.camera.yaw += 3.0 + frame as f32 * 0.01;
                    let started = Instant::now();
                    h.step();
                    h.state().view3d_wait_gpu();
                    wall.push(started.elapsed().as_micros() as f64 / 1000.0);
                    cpu.push(h.state().view3d_stats().unwrap().last_prepare_us as f64 / 1000.0);
                }
                let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
                sums[k].0.push(mean(&wall[4..]));
                sums[k].1.push(mean(&cpu[4..]));
                sums[k].2.push((process_cpu_ms() - ticks_before) / 24.0);
            }
        }
        for (k, (name, _)) in looks.iter().enumerate() {
            let (wall, cpu, process) = &sums[k];
            let fmt = |v: &[f64]| {
                let mean = v.iter().sum::<f64>() / v.len() as f64;
                let lo = v.iter().copied().fold(f64::MAX, f64::min);
                let hi = v.iter().copied().fold(f64::MIN, f64::max);
                format!("{mean:.2}（{lo:.2}〜{hi:.2}）")
            };
            println!(
                "三角形 {triangles:>6}・{name}: 全体 {} ms、prepare の CPU {} ms、プロセスの CPU 時間 {} ms、切り替え 1 回目 {:.0} ms・2 回目から {} ms",
                fmt(wall),
                fmt(cpu),
                fmt(process),
                switches[k][0],
                fmt(&switches[k][1..])
            );
        }
    }
}

/// プロセスの CPU 時間（ユーザーとシステムの合計、ms。Linux だけ。ほかは 0）。llvmpipe の描画は CPU の仕事なので、ほかの仕事で
/// 混んだ機械でも、全体の時間より揺れの小さい比べになる。
fn process_cpu_ms() -> f64 {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
        // 2 つ目の欄（名前）は括弧で囲まれ、空白を含みうる
        let rest = stat.rsplit_once(')').map_or("", |(_, r)| r);
        let fields: Vec<&str> = rest.split_whitespace().collect();
        let ticks = |i: usize| {
            fields
                .get(i)
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.0)
        };
        // utime・stime は 3 つ目の欄の後の 12・13 番目（0 始まりで 11・12）。1 秒は 100 ティック
        (ticks(11) + ticks(12)) * 10.0
    }
    #[cfg(not(target_os = "linux"))]
    {
        0.0
    }
}
