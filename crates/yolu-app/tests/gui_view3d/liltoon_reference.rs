//! lilToon の再現を Unity の lilToon と並べる材料（手で回す。`LILTOON_REF_DIR=<フォルダ> cargo test -p yolu-app --test gui_view3d liltoon_reference::
//! -- --ignored --nocapture`）。
//!
//! 場面（合成の素材だけ: 球・板・試しの人形）ごとに、3D ビューの絵（`ours_<名前>.png`）と、Unity で同じ場面を組むための記述
//! （`scenes.txt`）・メッシュ（`<名前>.mesh.txt`）・テクスチャ（`<名前>_<スロット>.rgba`、行は下から）を書く。Unity の側は
//! `tools/liltoon-reference.cs`（常駐の Unity の `unity-do.sh run` で回す）が `unity_<名前>.png` を書き、`compare` が差を数える。
//!
//! 撮った Unity の絵は `tests/liltoon_unity/` に置き、`the_view_stays_within_the_measured_difference_from_unity_liltoon`（普段の試験）が
//! 場面ごとの差を上限（`BOUNDS`）と照らす（影・リム・マットキャップ・発光・ノーマルマップ・SDF・AO・輪郭線・カットアウト・半透明・裏面・
//! 色調補正と、リムシェード・逆光・光沢・環境光の反射・2nd/3rd・デカールのミラーと複製・ラメ・異方性・距離フェードの回帰）。
//!
//! 環境光の反射の場面（`uniform_env`）は、3D ビューの環境を「なし」（一様な環境光）にし、Unity ではマテリアルのキューブマップの差し替え
//! （`_ReflectionCubeOverride`）で同じ一様な色を映す（シーンの反射プローブの絵を揃えずに、反射の式だけを比べる）。
use crate::common;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use common::*;
use egui_kittest::Harness;
use yolu_app::state::Action;
use yolu_app::view3d::display::{Display, EnvKind, Op};
use yolu_app::view3d::environment::{self, SkyColors, Source};
use yolu_app::view3d::model::ViewModel;
use yolu_app::YoluApp;
use yolu_core::geometry::{cube_sphere, ModelMesh, OrbitCamera, Submesh};
use yolu_core::glam::{Quat, Vec2, Vec3};
use yolu_core::look::{LookKind, LookValue, MaterialLook, TextureSource};
use yolu_core::skin::{demo_figure, FigureDetail};
use yolu_core::{
    Channel, ChannelInfo, ChannelKind, ColorSpace, Document, ImageColorSpace, ImageInput, Rgba8,
};

const WIDTH: f32 = 1000.0;
const HEIGHT: f32 = 760.0;

fn out_dir() -> Option<PathBuf> {
    std::env::var_os("LILTOON_REF_DIR").map(PathBuf::from)
}

/// 場面の光（来る向きの yaw・pitch、度）。
const LIGHT: (f32, f32) = (-140.0, 35.0);

struct Scene {
    name: &'static str,
    meshes: Vec<ModelMesh>,
    camera: OrbitCamera,
    /// 文書を作る（レイヤー・チャンネル）。
    paint: fn(&mut Document),
    look: fn(&Document) -> MaterialLook,
    /// Unity に渡すテクスチャ（スロット名 → 中身）。
    textures: fn(&Document) -> Vec<(String, Texture)>,
}

struct Texture {
    width: u32,
    height: u32,
    /// RGBA8、行は下から。
    rgba: Vec<u8>,
    srgb: bool,
}

/// 環境を一様にする場面（環境光の反射の比べと Standard。Standard は Unity の場面の既定の反射を同じ一様な色にする）。
fn uniform_env(name: &str) -> bool {
    name == "reflection_env" || name == "light_standard"
}

/// 一様な環境の色（3D ビューの環境「なし」の環境光: 環境光の色 × 0.4 をリニアへ。場面では環境光の色を白にする）。
fn uniform_env_color() -> f32 {
    yolu_app::view3d::brdf::srgb_to_linear(0.4)
}

fn sphere() -> Vec<ModelMesh> {
    vec![cube_sphere(32, 0.5)]
}

fn camera(distance: f32, target: Vec3) -> OrbitCamera {
    OrbitCamera {
        target,
        yaw: 0.0,
        pitch: 0.0,
        distance,
        model_radius: 1.0,
        ..Default::default()
    }
}

fn fill(doc: &mut Document, c: [u8; 4]) {
    doc.add_fill_layer(
        "色",
        &[(Channel::Color, Rgba8::new(c[0], c[1], c[2], c[3]))],
        None,
    )
    .unwrap();
}

fn lil(shader: &str) -> MaterialLook {
    let mut look = MaterialLook {
        kind: LookKind::LilToon,
        shader: shader.into(),
        ..MaterialLook::default()
    };
    look.textures
        .insert("_MainTex".into(), TextureSource::Channel(Channel::Color));
    look
}

fn set(look: &mut MaterialLook, name: &str, v: f32) {
    look.properties.insert(name.into(), LookValue::Float(v));
}

fn color(look: &mut MaterialLook, name: &str, c: [f32; 4]) {
    look.properties.insert(name.into(), LookValue::Color(c));
}

fn main_texture(doc: &Document) -> Vec<(String, Texture)> {
    vec![(
        "_MainTex".into(),
        Texture {
            width: doc.width(),
            height: doc.height(),
            rgba: doc.composite_channel(Channel::Color, doc.bounds()).unwrap(),
            srgb: true,
        },
    )]
}

/// ユーザーチャンネルを、何も描いていない所の既定に重ねた値で（lilToon の読み方。リニア）。
fn user_texture(doc: &Document, channel: Channel) -> Texture {
    let info = doc.channel_info(channel).unwrap().clone();
    let raw = doc.composite_channel(channel, doc.bounds()).unwrap();
    let d = info.default.to_array();
    let mut rgba = Vec::with_capacity(raw.len());
    for p in raw.as_chunks::<4>().0 {
        let a = p[3] as u32;
        for k in 0..4 {
            let premult = if k == 3 {
                a
            } else {
                (p[k] as u32 * a + 127) / 255
            };
            let v = premult + (d[k] as u32 * (255 - a) + 127) / 255;
            rgba.push(v.min(255) as u8);
        }
    }
    // スカラーは値を RGB に・A は 1
    if info.kind == ChannelKind::Scalar {
        for p in rgba.as_chunks_mut::<4>().0 {
            p[1] = p[0];
            p[2] = p[0];
            p[3] = 255;
        }
    }
    Texture {
        width: doc.width(),
        height: doc.height(),
        rgba,
        srgb: info.color_space == ColorSpace::Srgb,
    }
}

/// 合成のマットキャップの絵（球の正面に寄った明るい点と、縁の暗さ。自作の絵）。行は下から。
fn matcap_image() -> (u32, Vec<u8>) {
    let n = 128u32;
    let mut out = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let u = (x as f32 + 0.5) / n as f32 * 2.0 - 1.0;
            let v = (y as f32 + 0.5) / n as f32 * 2.0 - 1.0;
            let r2 = (u * u + v * v).min(1.0);
            let z = (1.0 - r2).sqrt();
            let spot = (-((u + 0.35).powi(2) + (v - 0.4).powi(2)) * 18.0).exp();
            let base = 0.15 + 0.6 * z;
            let c = [base * 0.8 + spot, base * 0.9 + spot, base + spot * 0.8];
            for k in c {
                out.push((k.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            out.push(255);
        }
    }
    (n, out)
}

fn stripes_channel(
    doc: &mut Document,
    name: &str,
    kind: ChannelKind,
    default: Rgba8,
    value: Rgba8,
) -> Channel {
    let channel = doc
        .add_channel(ChannelInfo {
            name: name.into(),
            kind,
            color_space: ColorSpace::Linear,
            default,
        })
        .unwrap();
    let layer = doc.add_layer(name).unwrap();
    doc.set_channel_enabled(layer, channel, true).unwrap();
    for y in 0..doc.height() {
        for x in 0..doc.width() {
            if (x / 16 + y / 16) % 2 == 0 {
                doc.set_channel_pixel(layer, channel, x, y, value).unwrap();
            }
        }
    }
    channel
}

/// 2 つのスカラーのチャンネルに左右の傾き（u と 1 − u）を塗る（SDF の R・G）。
fn gradient_channels(doc: &mut Document) -> (Channel, Channel) {
    let mut make = |name: &str, flip: bool| {
        let c = doc
            .add_channel(ChannelInfo {
                name: name.into(),
                kind: ChannelKind::Scalar,
                color_space: ColorSpace::Linear,
                default: Rgba8::new(255, 255, 255, 255),
            })
            .unwrap();
        let layer = doc.add_layer(name).unwrap();
        doc.set_channel_enabled(layer, c, true).unwrap();
        for y in 0..doc.height() {
            for x in 0..doc.width() {
                let mut v = (x * 255 / (doc.width() - 1)) as u8;
                if flip {
                    v = 255 - v;
                }
                doc.set_channel_pixel(layer, c, x, y, Rgba8::new(v, v, v, 255))
                    .unwrap();
            }
        }
        c
    };
    (make("顔の影 R", false), make("顔の影 G", true))
}

fn user_channels(doc: &Document) -> Vec<Channel> {
    doc.channels()
        .into_iter()
        .filter(|c| !c.is_standard())
        .collect()
}

/// Unity のノーマルマップの取り込み（DXT5nm: A に X、G に Y、R は 1）と同じ並びに詰めた法線の出力。
fn normal_texture(doc: &Document) -> Texture {
    let out = doc.normal_output(u64::MAX).unwrap();
    let mut rgba = Vec::with_capacity(out.len());
    for p in out.as_chunks::<4>().0 {
        rgba.extend_from_slice(&[255, p[1], 0, p[0]]);
    }
    Texture {
        width: doc.width(),
        height: doc.height(),
        rgba,
        srgb: false,
    }
}

/// Emission の書き出し（値 × アルファ、不透明。sRGB）。
fn emission_texture(doc: &Document) -> Texture {
    let raw = doc
        .composite_channel(Channel::Emission, doc.bounds())
        .unwrap();
    let mut rgba = Vec::with_capacity(raw.len());
    for p in raw.as_chunks::<4>().0 {
        let a = p[3] as u32;
        for v in &p[..3] {
            rgba.push(((*v as u32 * a + 127) / 255) as u8);
        }
        rgba.push(255);
    }
    Texture {
        width: doc.width(),
        height: doc.height(),
        rgba,
        srgb: true,
    }
}

fn stripes_standard(doc: &mut Document, channel: Channel, value: Rgba8) {
    let layer = doc.add_layer("縞").unwrap();
    doc.set_channel_enabled(layer, channel, true).unwrap();
    for y in 0..doc.height() {
        for x in 0..doc.width() {
            if (x / 16 + y / 16) % 2 == 0 {
                doc.set_channel_pixel(layer, channel, x, y, value).unwrap();
            }
        }
    }
}

/// 色のユーザーチャンネル（sRGB、何も描いていない所は透明）に、真ん中の円と縞の絵を描く（メインカラー 2nd のデカール・輪郭線の色）。
fn pattern_color_channel(doc: &mut Document, name: &str) -> Channel {
    let channel = doc
        .add_channel(ChannelInfo {
            name: name.into(),
            kind: ChannelKind::Color,
            color_space: ColorSpace::Srgb,
            default: Rgba8::new(0, 0, 0, 0),
        })
        .unwrap();
    let layer = doc.add_layer(name).unwrap();
    doc.set_channel_enabled(layer, channel, true).unwrap();
    let (w, h) = (doc.width() as f32, doc.height() as f32);
    for y in 0..doc.height() {
        for x in 0..doc.width() {
            let (u, v) = ((x as f32 + 0.5) / w, (y as f32 + 0.5) / h);
            let r = ((u - 0.5).powi(2) + (v - 0.5).powi(2)).sqrt();
            if r < 0.35 {
                let c = if (x / 12) % 2 == 0 {
                    Rgba8::new(240, 60, 90, 255)
                } else {
                    Rgba8::new(60, 200, 240, 220)
                };
                doc.set_channel_pixel(layer, channel, x, y, c).unwrap();
            }
        }
    }
    channel
}

/// 色のユーザーチャンネル（sRGB、何も描いていない所は透明）に、左右で色の違う円を描く（デカールの反転が見える絵）。
fn split_color_channel(doc: &mut Document, name: &str) -> Channel {
    let channel = doc
        .add_channel(ChannelInfo {
            name: name.into(),
            kind: ChannelKind::Color,
            color_space: ColorSpace::Srgb,
            default: Rgba8::new(0, 0, 0, 0),
        })
        .unwrap();
    let layer = doc.add_layer(name).unwrap();
    doc.set_channel_enabled(layer, channel, true).unwrap();
    let (w, h) = (doc.width() as f32, doc.height() as f32);
    for y in 0..doc.height() {
        for x in 0..doc.width() {
            let (u, v) = ((x as f32 + 0.5) / w, (y as f32 + 0.5) / h);
            let r = ((u - 0.5).powi(2) + (v - 0.5).powi(2)).sqrt();
            if r < 0.4 {
                let c = if u < 0.5 {
                    Rgba8::new(240, 60, 90, 255)
                } else if v < 0.5 {
                    Rgba8::new(60, 200, 240, 255)
                } else {
                    Rgba8::new(250, 220, 60, 255)
                };
                doc.set_channel_pixel(layer, channel, x, y, c).unwrap();
            }
        }
    }
    channel
}

fn stripes_color_standard(doc: &mut Document) {
    let layer = doc.add_layer("縞の色").unwrap();
    for y in 0..doc.height() {
        for x in 0..doc.width() {
            let c = if (x / 16 + y / 16) % 2 == 0 {
                Rgba8::new(230, 120, 70, 255)
            } else {
                Rgba8::new(80, 150, 220, 255)
            };
            doc.set_channel_pixel(layer, Channel::Color, x, y, c)
                .unwrap();
        }
    }
}

fn vector(look: &mut MaterialLook, name: &str, v: [f32; 4]) {
    look.properties.insert(name.into(), LookValue::Vector(v));
}

/// 場面のユーザーチャンネル（作った順）の i 番目を、スロットの名前で Unity に渡す。
fn with_user(d: &Document, slots: &[(&str, usize)]) -> Vec<(String, Texture)> {
    let mut t = main_texture(d);
    let c = user_channels(d);
    for (slot, i) in slots {
        t.push(((*slot).into(), user_texture(d, c[*i])));
    }
    t
}

fn scenes() -> Vec<Scene> {
    vec![
        Scene {
            name: "shadow",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| fill(d, [230, 200, 190, 255]),
            look: |_| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                l
            },
            textures: main_texture,
        },
        Scene {
            name: "shadow_3rd",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| fill(d, [200, 210, 240, 255]),
            look: |_| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_ShadowBorder", 0.6);
                set(&mut l, "_ShadowBlur", 0.3);
                color(&mut l, "_Shadow3rdColor", [0.2, 0.15, 0.4, 1.0]);
                set(&mut l, "_ShadowMainStrength", 0.4);
                set(&mut l, "_LightMinLimit", 0.2);
                set(&mut l, "_MonochromeLighting", 0.5);
                l
            },
            textures: main_texture,
        },
        Scene {
            name: "shadow_mask",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| {
                fill(d, [230, 200, 190, 255]);
                stripes_channel(
                    d,
                    "影の強さ",
                    ChannelKind::Scalar,
                    Rgba8::new(255, 255, 255, 255),
                    Rgba8::new(40, 40, 40, 255),
                );
            },
            look: |d| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                let mask = d.channels().into_iter().find(|c| !c.is_standard()).unwrap();
                l.textures
                    .insert("_ShadowStrengthMask".into(), TextureSource::Channel(mask));
                l
            },
            textures: |d| {
                let mut t = main_texture(d);
                let mask = d.channels().into_iter().find(|c| !c.is_standard()).unwrap();
                t.push(("_ShadowStrengthMask".into(), user_texture(d, mask)));
                t
            },
        },
        Scene {
            name: "rim",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| fill(d, [120, 140, 200, 255]),
            look: |_| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_UseRim", 1.0);
                color(&mut l, "_RimColor", [1.0, 0.85, 0.6, 1.0]);
                set(&mut l, "_RimDirStrength", 0.5);
                color(&mut l, "_RimIndirColor", [0.4, 0.6, 1.0, 1.0]);
                l
            },
            textures: main_texture,
        },
        Scene {
            name: "matcap",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| fill(d, [190, 190, 200, 255]),
            look: |_| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_UseMatCap", 1.0);
                l.textures.insert(
                    "_MatCapTex".into(),
                    TextureSource::Image(yolu_core::ImageId(
                        0xA11C_A900_0000_0000_0000_0000_0000_0001,
                    )),
                );
                set(&mut l, "_MatCapBlend", 0.8);
                l
            },
            textures: |d| {
                let mut t = main_texture(d);
                let (n, rgba) = matcap_image();
                t.push((
                    "_MatCapTex".into(),
                    Texture {
                        width: n,
                        height: n,
                        rgba,
                        srgb: true,
                    },
                ));
                t
            },
        },
        Scene {
            name: "outline",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| fill(d, [230, 200, 190, 255]),
            look: |_| {
                let mut l = lil("Hidden/lilToonOutline");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_OutlineWidth", 1.0);
                l
            },
            textures: main_texture,
        },
        Scene {
            name: "emission",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| {
                fill(d, [90, 90, 110, 255]);
                stripes_channel(
                    d,
                    "発光のマスク",
                    ChannelKind::Scalar,
                    Rgba8::new(0, 0, 0, 255),
                    Rgba8::new(255, 255, 255, 255),
                );
            },
            look: |d| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_UseEmission", 1.0);
                color(&mut l, "_EmissionColor", [1.0, 0.5, 0.2, 1.0]);
                let mask = d.channels().into_iter().find(|c| !c.is_standard()).unwrap();
                l.textures
                    .insert("_EmissionBlendMask".into(), TextureSource::Channel(mask));
                l
            },
            textures: |d| {
                let mut t = main_texture(d);
                let mask = d.channels().into_iter().find(|c| !c.is_standard()).unwrap();
                t.push(("_EmissionBlendMask".into(), user_texture(d, mask)));
                t
            },
        },
        Scene {
            name: "figure",
            meshes: figure(),
            camera: OrbitCamera {
                target: Vec3::new(0.0, 0.85, 0.0),
                yaw: 20.0,
                pitch: 5.0,
                distance: 4.2,
                model_radius: 1.0,
                ..Default::default()
            },
            paint: |d| fill(d, [235, 205, 190, 255]),
            look: |_| {
                let mut l = lil("Hidden/lilToonOutline");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_UseRim", 1.0);
                set(&mut l, "_OutlineWidth", 0.3);
                l
            },
            textures: main_texture,
        },
        Scene {
            name: "cutout",
            meshes: vec![quad_with_alpha_uv()],
            camera: camera(2.5, Vec3::ZERO),
            paint: |d| {
                // アルファが左から右へ 0 → 1 の色（カットアウトの Cutoff 0.5 で左半分を捨てる）
                let layer = d.add_layer("色").unwrap();
                for y in 0..d.height() {
                    for x in 0..d.width() {
                        let a = (x * 255 / (d.width() - 1)) as u8;
                        d.set_channel_pixel(
                            layer,
                            Channel::Color,
                            x,
                            y,
                            Rgba8::new(200, 120, 60, a),
                        )
                        .unwrap();
                    }
                }
            },
            look: |_| {
                let mut l = lil("Hidden/lilToonCutout");
                set(&mut l, "_UseShadow", 1.0);
                l
            },
            textures: main_texture,
        },
        Scene {
            name: "transparent",
            meshes: vec![quad_with_alpha_uv()],
            camera: camera(2.5, Vec3::ZERO),
            paint: |d| {
                let layer = d.add_layer("色").unwrap();
                for y in 0..d.height() {
                    for x in 0..d.width() {
                        let a = (x * 255 / (d.width() - 1)) as u8;
                        d.set_channel_pixel(
                            layer,
                            Channel::Color,
                            x,
                            y,
                            Rgba8::new(80, 160, 230, a),
                        )
                        .unwrap();
                    }
                }
            },
            look: |_| {
                let mut l = lil("Hidden/lilToonTransparent");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_Cutoff", 0.001);
                l
            },
            textures: main_texture,
        },
        Scene {
            name: "backface",
            meshes: vec![quad_with_alpha_uv()],
            camera: OrbitCamera {
                target: Vec3::ZERO,
                yaw: 160.0,
                pitch: 10.0,
                distance: 2.5,
                model_radius: 1.0,
                ..Default::default()
            },
            paint: |d| fill(d, [230, 200, 190, 255]),
            look: |_| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_Cull", 0.0);
                color(&mut l, "_BackfaceColor", [0.2, 0.4, 1.0, 0.6]);
                set(&mut l, "_BackfaceForceShadow", 0.5);
                l
            },
            textures: main_texture,
        },
        Scene {
            name: "normal",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| {
                fill(d, [210, 210, 220, 255]);
                stripes_standard(d, Channel::Normal, Rgba8::new(210, 128, 215, 255));
            },
            look: |_| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_UseBumpMap", 1.0);
                set(&mut l, "_BumpScale", 1.5);
                l.textures
                    .insert("_BumpMap".into(), TextureSource::Channel(Channel::Normal));
                l
            },
            textures: |d| {
                let mut t = main_texture(d);
                t.push(("_BumpMap".into(), normal_texture(d)));
                t
            },
        },
        Scene {
            name: "tone",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| fill(d, [200, 120, 60, 255]),
            look: |_| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                l.properties.insert(
                    "_MainTexHSVG".into(),
                    LookValue::Vector([0.15, 1.4, 0.85, 1.3]),
                );
                l
            },
            textures: main_texture,
        },
        Scene {
            name: "sdf",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| {
                fill(d, [230, 200, 190, 255]);
                gradient_channels(d);
            },
            look: |d| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_ShadowMaskType", 2.0);
                let c = user_channels(d);
                l.textures.insert(
                    "_ShadowStrengthMask".into(),
                    TextureSource::Packed([
                        yolu_core::look::PlaneSource::Channel {
                            channel: c[0],
                            component: 0,
                        },
                        yolu_core::look::PlaneSource::Channel {
                            channel: c[1],
                            component: 0,
                        },
                        yolu_core::look::PlaneSource::Zero,
                        yolu_core::look::PlaneSource::One,
                    ]),
                );
                l
            },
            textures: |d| {
                let mut t = main_texture(d);
                let c = user_channels(d);
                let r = user_texture(d, c[0]);
                let g = user_texture(d, c[1]);
                let mut rgba = Vec::with_capacity(r.rgba.len());
                for (a, b) in r
                    .rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(g.rgba.as_chunks::<4>().0)
                {
                    rgba.extend_from_slice(&[a[0], b[0], 0, 255]);
                }
                t.push((
                    "_ShadowStrengthMask".into(),
                    Texture {
                        width: d.width(),
                        height: d.height(),
                        rgba,
                        srgb: false,
                    },
                ));
                t
            },
        },
        Scene {
            name: "ao_flat",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| {
                fill(d, [230, 200, 190, 255]);
                stripes_channel(
                    d,
                    "AO",
                    ChannelKind::Scalar,
                    Rgba8::new(255, 255, 255, 255),
                    Rgba8::new(80, 80, 80, 255),
                );
            },
            look: |d| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_ShadowMaskType", 1.0);
                set(&mut l, "_ShadowStrength", 0.8);
                set(&mut l, "_ShadowPostAO", 1.0);
                l.properties.insert(
                    "_ShadowAOShift".into(),
                    LookValue::Vector([1.2, -0.1, 1.0, 0.0]),
                );
                let c = user_channels(d);
                l.textures
                    .insert("_ShadowBorderMask".into(), TextureSource::Channel(c[0]));
                l
            },
            textures: |d| {
                let mut t = main_texture(d);
                let c = user_channels(d);
                t.push(("_ShadowBorderMask".into(), user_texture(d, c[0])));
                t
            },
        },
        Scene {
            name: "emission_rim",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| {
                fill(d, [90, 90, 110, 255]);
                stripes_standard(d, Channel::Emission, Rgba8::new(255, 200, 80, 255));
            },
            look: |_| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_UseEmission", 1.0);
                set(&mut l, "_EmissionMap_UVMode", 4.0);
                l.textures.insert(
                    "_EmissionMap".into(),
                    TextureSource::Channel(Channel::Emission),
                );
                set(&mut l, "_UseEmission2nd", 1.0);
                color(&mut l, "_Emission2ndColor", [0.2, 0.4, 1.0, 1.0]);
                set(&mut l, "_Emission2ndBlendMode", 0.0);
                set(&mut l, "_Emission2ndBlend", 0.4);
                set(&mut l, "_Emission2ndFluorescence", 0.5);
                l
            },
            textures: |d| {
                let mut t = main_texture(d);
                t.push(("_EmissionMap".into(), emission_texture(d)));
                t
            },
        },
        Scene {
            name: "matcap2",
            meshes: sphere(),
            camera: OrbitCamera {
                target: Vec3::ZERO,
                yaw: 30.0,
                pitch: 20.0,
                distance: 3.0,
                model_radius: 1.0,
                ..Default::default()
            },
            paint: |d| {
                fill(d, [190, 190, 200, 255]);
                stripes_channel(
                    d,
                    "マットキャップのマスク",
                    ChannelKind::Scalar,
                    Rgba8::new(255, 255, 255, 255),
                    Rgba8::new(0, 0, 0, 255),
                );
            },
            look: |d| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_UseMatCap2nd", 1.0);
                l.textures.insert(
                    "_MatCap2ndTex".into(),
                    TextureSource::Image(yolu_core::ImageId(
                        0xA11C_A900_0000_0000_0000_0000_0000_0001,
                    )),
                );
                set(&mut l, "_MatCap2ndBlendMode", 3.0);
                set(&mut l, "_MatCap2ndZRotCancel", 0.0);
                set(&mut l, "_MatCap2ndShadowMask", 0.7);
                set(&mut l, "_MatCap2ndMainStrength", 0.3);
                let c = user_channels(d);
                l.textures
                    .insert("_MatCap2ndBlendMask".into(), TextureSource::Channel(c[0]));
                l
            },
            textures: |d| {
                let mut t = main_texture(d);
                let (n, rgba) = matcap_image();
                t.push((
                    "_MatCap2ndTex".into(),
                    Texture {
                        width: n,
                        height: n,
                        rgba,
                        srgb: true,
                    },
                ));
                let c = user_channels(d);
                t.push(("_MatCap2ndBlendMask".into(), user_texture(d, c[0])));
                t
            },
        },
        Scene {
            name: "outline_mask",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| {
                fill(d, [230, 200, 190, 255]);
                stripes_channel(
                    d,
                    "輪郭線の太さ",
                    ChannelKind::Scalar,
                    Rgba8::new(255, 255, 255, 255),
                    Rgba8::new(0, 0, 0, 255),
                );
            },
            look: |d| {
                let mut l = lil("Hidden/lilToonOutline");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_OutlineWidth", 2.0);
                set(&mut l, "_OutlineFixWidth", 0.0);
                color(&mut l, "_OutlineColor", [0.9, 0.2, 0.3, 1.0]);
                l.properties.insert(
                    "_OutlineTexHSVG".into(),
                    LookValue::Vector([0.0, 1.0, 1.0, 1.0]),
                );
                let c = user_channels(d);
                l.textures
                    .insert("_OutlineWidthMask".into(), TextureSource::Channel(c[0]));
                l
            },
            textures: |d| {
                let mut t = main_texture(d);
                let c = user_channels(d);
                t.push(("_OutlineWidthMask".into(), user_texture(d, c[0])));
                t
            },
        },
        Scene {
            name: "rim_shade",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| {
                fill(d, [230, 200, 190, 255]);
                stripes_channel(
                    d,
                    "リムシェードのマスク",
                    ChannelKind::Scalar,
                    Rgba8::new(255, 255, 255, 255),
                    Rgba8::new(70, 70, 70, 255),
                );
            },
            look: |d| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_UseRimShade", 1.0);
                color(&mut l, "_RimShadeColor", [0.3, 0.25, 0.55, 1.0]);
                set(&mut l, "_RimShadeBorder", 0.4);
                set(&mut l, "_RimShadeBlur", 0.6);
                set(&mut l, "_RimShadeFresnelPower", 2.0);
                l.textures.insert(
                    "_RimShadeMask".into(),
                    TextureSource::Channel(user_channels(d)[0]),
                );
                l
            },
            textures: |d| with_user(d, &[("_RimShadeMask", 0)]),
        },
        Scene {
            name: "backlight",
            meshes: sphere(),
            // 光の向こう側から（光は yaw −140°・pitch 35° から来る。そのほぼ反対から見る）
            camera: OrbitCamera {
                target: Vec3::ZERO,
                yaw: -125.0,
                pitch: -22.0,
                distance: 3.0,
                model_radius: 1.0,
                ..Default::default()
            },
            paint: |d| fill(d, [220, 200, 200, 255]),
            look: |_| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_UseBacklight", 1.0);
                color(&mut l, "_BacklightColor", [1.0, 0.55, 0.3, 1.0]);
                set(&mut l, "_BacklightBorder", 0.3);
                set(&mut l, "_BacklightBlur", 0.15);
                set(&mut l, "_BacklightDirectivity", 2.0);
                set(&mut l, "_BacklightMainStrength", 0.3);
                l
            },
            textures: main_texture,
        },
        Scene {
            name: "specular_toon",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| {
                fill(d, [150, 160, 210, 255]);
                stripes_channel(
                    d,
                    "滑らかさ",
                    ChannelKind::Scalar,
                    Rgba8::new(255, 255, 255, 255),
                    Rgba8::new(150, 150, 150, 255),
                );
            },
            look: |d| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_UseReflection", 1.0);
                set(&mut l, "_Smoothness", 0.85);
                set(&mut l, "_Metallic", 0.2);
                set(&mut l, "_SpecularBorder", 0.6);
                set(&mut l, "_SpecularBlur", 0.1);
                l.textures.insert(
                    "_SmoothnessTex".into(),
                    TextureSource::Channel(user_channels(d)[0]),
                );
                l
            },
            textures: |d| with_user(d, &[("_SmoothnessTex", 0)]),
        },
        Scene {
            name: "specular_real",
            meshes: sphere(),
            camera: OrbitCamera {
                target: Vec3::ZERO,
                yaw: -30.0,
                pitch: 15.0,
                distance: 3.0,
                model_radius: 1.0,
                ..Default::default()
            },
            paint: |d| {
                fill(d, [200, 170, 120, 255]);
                stripes_channel(
                    d,
                    "金属度",
                    ChannelKind::Scalar,
                    Rgba8::new(255, 255, 255, 255),
                    Rgba8::new(60, 60, 60, 255),
                );
            },
            look: |d| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_UseReflection", 1.0);
                set(&mut l, "_SpecularToon", 0.0);
                set(&mut l, "_Smoothness", 0.75);
                set(&mut l, "_Metallic", 0.6);
                color(&mut l, "_ReflectionColor", [1.0, 0.85, 0.7, 1.0]);
                l.textures.insert(
                    "_MetallicGlossMap".into(),
                    TextureSource::Channel(user_channels(d)[0]),
                );
                l
            },
            textures: |d| with_user(d, &[("_MetallicGlossMap", 0)]),
        },
        Scene {
            name: "main2nd_decal",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| {
                fill(d, [220, 210, 200, 255]);
                pattern_color_channel(d, "デカール");
            },
            look: |d| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_UseMain2ndTex", 1.0);
                set(&mut l, "_Main2ndTexIsDecal", 1.0);
                // 正面の面（UV の中心 (1/6, 1/4)）に大きさ 0.2（lilToon の欄の換算: scale = 1 / 大きさ、offset = −位置 × scale + 0.5）
                vector(&mut l, "_Main2ndTex_ST", [5.0, 5.0, -1.0 / 3.0, -0.75]);
                set(&mut l, "_Main2ndTexAngle", 0.5);
                set(&mut l, "_Main2ndEnableLighting", 0.7);
                l.textures.insert(
                    "_Main2ndTex".into(),
                    TextureSource::Channel(user_channels(d)[0]),
                );
                l
            },
            textures: |d| with_user(d, &[("_Main2ndTex", 0)]),
        },
        Scene {
            name: "main3rd_matcap",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| {
                fill(d, [200, 190, 220, 255]);
                stripes_channel(
                    d,
                    "3rd のマスク",
                    ChannelKind::Scalar,
                    Rgba8::new(255, 255, 255, 255),
                    Rgba8::new(0, 0, 0, 255),
                );
            },
            look: |d| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_UseMain3rdTex", 1.0);
                set(&mut l, "_Main3rdTex_UVMode", 4.0);
                color(&mut l, "_Color3rd", [0.4, 0.8, 1.0, 0.7]);
                set(&mut l, "_Main3rdTexBlendMode", 1.0);
                set(&mut l, "_Main3rdEnableLighting", 0.3);
                l.textures.insert(
                    "_Main3rdBlendMask".into(),
                    TextureSource::Channel(user_channels(d)[0]),
                );
                l
            },
            textures: |d| with_user(d, &[("_Main3rdBlendMask", 0)]),
        },
        Scene {
            name: "normal2nd",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| {
                fill(d, [210, 210, 220, 255]);
                stripes_standard(d, Channel::Normal, Rgba8::new(60, 128, 215, 255));
                stripes_channel(
                    d,
                    "2nd のマスク",
                    ChannelKind::Scalar,
                    Rgba8::new(255, 255, 255, 255),
                    Rgba8::new(128, 128, 128, 255),
                );
            },
            look: |d| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_UseBump2ndMap", 1.0);
                set(&mut l, "_Bump2ndScale", 1.2);
                vector(&mut l, "_Bump2ndMap_ST", [2.0, 1.0, 0.0, 0.0]);
                l.textures.insert(
                    "_Bump2ndMap".into(),
                    TextureSource::Channel(Channel::Normal),
                );
                l.textures.insert(
                    "_Bump2ndScaleMask".into(),
                    TextureSource::Channel(user_channels(d)[0]),
                );
                l
            },
            textures: |d| {
                let mut t = with_user(d, &[("_Bump2ndScaleMask", 0)]);
                t.push(("_Bump2ndMap".into(), normal_texture(d)));
                t
            },
        },
        Scene {
            name: "glitter",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| fill(d, [70, 60, 100, 255]),
            look: |_| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_UseGlitter", 1.0);
                color(&mut l, "_GlitterColor", [1.0, 0.9, 0.6, 1.0]);
                vector(&mut l, "_GlitterParams1", [64.0, 64.0, 0.16, 50.0]);
                vector(&mut l, "_GlitterParams2", [0.0, 0.0, 0.0, 0.0]);
                l
            },
            textures: main_texture,
        },
        Scene {
            name: "anisotropy",
            meshes: sphere(),
            camera: OrbitCamera {
                target: Vec3::ZERO,
                yaw: -30.0,
                pitch: 15.0,
                distance: 3.0,
                model_radius: 1.0,
                ..Default::default()
            },
            paint: |d| fill(d, [120, 90, 70, 255]),
            look: |_| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_UseReflection", 1.0);
                set(&mut l, "_Smoothness", 0.8);
                set(&mut l, "_UseAnisotropy", 1.0);
                set(&mut l, "_Anisotropy2Reflection", 1.0);
                set(&mut l, "_AnisotropyScale", 0.6);
                set(&mut l, "_SpecularToon", 0.0);
                set(&mut l, "_Anisotropy2ndSpecularStrength", 0.5);
                set(&mut l, "_Anisotropy2ndShift", 0.5);
                l
            },
            textures: main_texture,
        },
        Scene {
            name: "distance_fade",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| fill(d, [230, 200, 190, 255]),
            look: |_| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                vector(&mut l, "_DistanceFade", [2.55, 2.85, 0.8, 0.0]);
                color(&mut l, "_DistanceFadeColor", [0.1, 0.1, 0.25, 1.0]);
                color(&mut l, "_DistanceFadeRimColor", [1.0, 0.5, 0.5, 0.6]);
                l
            },
            textures: main_texture,
        },
        Scene {
            name: "outline_tex",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| {
                fill(d, [230, 200, 190, 255]);
                pattern_color_channel(d, "輪郭線の色");
            },
            look: |d| {
                let mut l = lil("Hidden/lilToonOutline");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_OutlineWidth", 1.5);
                color(&mut l, "_OutlineColor", [1.0, 1.0, 1.0, 1.0]);
                vector(&mut l, "_OutlineTexHSVG", [0.1, 1.2, 0.9, 1.0]);
                color(&mut l, "_OutlineLitColor", [1.0, 0.9, 0.3, 1.0]);
                set(&mut l, "_OutlineLitScale", 4.0);
                set(&mut l, "_OutlineLitOffset", -2.0);
                l.textures.insert(
                    "_OutlineTex".into(),
                    TextureSource::Channel(user_channels(d)[0]),
                );
                l
            },
            textures: |d| with_user(d, &[("_OutlineTex", 0)]),
        },
        Scene {
            name: "uv",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: stripes_color_standard,
            look: |_| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                vector(&mut l, "_MainTex_ST", [2.0, 1.0, 0.1, 0.0]);
                vector(&mut l, "_MainTex_ScrollRotate", [0.0, 0.0, 0.6, 0.0]);
                l
            },
            textures: main_texture,
        },
        Scene {
            name: "reflection_env",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| fill(d, [200, 130, 90, 255]),
            look: |_| {
                // 環境光の反射だけ（光沢の項は切）。環境は一様（`uniform_env`）
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_UseReflection", 1.0);
                set(&mut l, "_ApplySpecular", 0.0);
                set(&mut l, "_ApplyReflection", 1.0);
                set(&mut l, "_Smoothness", 0.7);
                set(&mut l, "_Metallic", 0.8);
                color(&mut l, "_ReflectionColor", [0.9, 0.95, 1.0, 1.0]);
                l
            },
            textures: main_texture,
        },
        Scene {
            name: "decal_mirror",
            meshes: vec![mirrored_quads()],
            camera: camera(5.0, Vec3::ZERO),
            paint: |d| {
                fill(d, [220, 210, 200, 255]);
                split_color_channel(d, "デカール");
            },
            look: |d| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                let decal = TextureSource::Channel(user_channels(d)[0]);
                // 2nd: 位置 (0.7, 0.5)・大きさ 0.35、ミラーモード「右のみ・反転」（右手の面で裏返し、左手の面には出さない）
                set(&mut l, "_UseMain2ndTex", 1.0);
                set(&mut l, "_Main2ndTexIsDecal", 1.0);
                vector(&mut l, "_Main2ndTex_ST", decal_st(0.7, 0.5, 0.35));
                set(&mut l, "_Main2ndTexIsRightOnly", 1.0);
                set(&mut l, "_Main2ndTexShouldFlipMirror", 1.0);
                l.textures.insert("_Main2ndTex".into(), decal);
                // 3rd: 位置 (0.8, 0.25)・大きさ 0.25、複製モード「反転」（u 0.2 にも写し、写したほうを裏返す）
                set(&mut l, "_UseMain3rdTex", 1.0);
                set(&mut l, "_Main3rdTexIsDecal", 1.0);
                vector(&mut l, "_Main3rdTex_ST", decal_st(0.8, 0.25, 0.25));
                set(&mut l, "_Main3rdTexShouldCopy", 1.0);
                set(&mut l, "_Main3rdTexShouldFlipCopy", 1.0);
                l.textures.insert("_Main3rdTex".into(), decal);
                l
            },
            textures: |d| with_user(d, &[("_Main2ndTex", 0), ("_Main3rdTex", 0)]),
        },
        Scene {
            name: "distance_fade_object",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| fill(d, [230, 200, 190, 255]),
            look: |_| {
                // 座標のモード: カメラから物の原点までの距離（3）で、面の全部が同じだけ消える
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_DistanceFadeMode", 1.0);
                vector(&mut l, "_DistanceFade", [2.5, 3.5, 0.8, 0.0]);
                color(&mut l, "_DistanceFadeColor", [0.1, 0.1, 0.25, 1.0]);
                color(&mut l, "_DistanceFadeRimColor", [1.0, 0.5, 0.5, 0.6]);
                l
            },
            textures: main_texture,
        },
        Scene {
            name: "matcap_normal",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| {
                fill(d, [190, 190, 200, 255]);
                stripes_standard(d, Channel::Normal, Rgba8::new(60, 128, 215, 255));
            },
            look: |_| {
                // マットキャップのカスタムノーマル（メインのノーマルマップは切。マットキャップだけが縞の法線で読む）
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_UseMatCap", 1.0);
                l.textures.insert(
                    "_MatCapTex".into(),
                    TextureSource::Image(yolu_core::ImageId(
                        0xA11C_A900_0000_0000_0000_0000_0000_0001,
                    )),
                );
                set(&mut l, "_MatCapBlend", 0.8);
                set(&mut l, "_MatCapCustomNormal", 1.0);
                set(&mut l, "_MatCapBumpScale", 1.5);
                l.textures.insert(
                    "_MatCapBumpMap".into(),
                    TextureSource::Channel(Channel::Normal),
                );
                l
            },
            textures: |d| {
                let mut t = main_texture(d);
                let (n, rgba) = matcap_image();
                t.push((
                    "_MatCapTex".into(),
                    Texture {
                        width: n,
                        height: n,
                        rgba,
                        srgb: true,
                    },
                ));
                t.push(("_MatCapBumpMap".into(), normal_texture(d)));
                t
            },
        },
        // 1 を超える色（lilToon の欄では HDR で選べる色。Unity の [HDR] ではない）のマットキャップ: リニアの色空間の Unity は
        // GammaToLinearSpace（1 以上は pow(x, 2.2)）で直す。暗いマットキャップで、飽和しない所の明るさを比べる
        Scene {
            name: "matcap_hdr",
            meshes: sphere(),
            camera: camera(3.0, Vec3::ZERO),
            paint: |d| fill(d, [110, 80, 70, 255]),
            look: |_| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                set(&mut l, "_UseMatCap", 1.0);
                l.textures
                    .insert("_MatCapTex".into(), TextureSource::Image(BAND_MATCAP));
                set(&mut l, "_MatCapBlend", 1.0);
                color(&mut l, "_MatCapColor", [2.119, 1.895, 1.789, 1.0]);
                l
            },
            textures: band_matcap_textures,
        },
        // 髪のマテリアルの値（Live Link で受けた実物の値。逆光ライトの色は 16.948 の HDR・指向性 10、マットキャップの色も 1 を
        // 超える）を、光の向き 3 つ（前・横・後ろ。`scene_light`）で
        bright_backlight_scene("bright_backlight_front"),
        bright_backlight_scene("bright_backlight_side"),
        bright_backlight_scene("bright_backlight_back"),
        // 光の強さ（`scene_intensity`）: 3D ビューの強さ 1 と、Unity のディレクショナルライト（白・強さ 1）。3D ビューの既定の光の向きで、
        // 白いテクスチャの lilToon（既定の値・影あり）と Standard（一様な環境）を
        Scene {
            name: "light_default",
            meshes: sphere(),
            camera: light_camera(),
            paint: |d| fill(d, [255, 255, 255, 255]),
            look: |_| lil("lilToon"),
            textures: main_texture,
        },
        Scene {
            name: "light_shadow",
            meshes: sphere(),
            camera: light_camera(),
            paint: |d| fill(d, [255, 255, 255, 255]),
            look: |_| {
                let mut l = lil("lilToon");
                set(&mut l, "_UseShadow", 1.0);
                l
            },
            textures: main_texture,
        },
        Scene {
            name: "light_standard",
            meshes: sphere(),
            camera: light_camera(),
            paint: |d| fill(d, [255, 255, 255, 255]),
            look: |_| MaterialLook::default(),
            textures: main_texture,
        },
    ]
}

/// 光の強さの場面のカメラ: 光の来る向きから横へ 60° 回った所から（光の真正面の点と、光の届かない側の両方が見える）。
fn light_camera() -> OrbitCamera {
    OrbitCamera {
        target: Vec3::ZERO,
        yaw: -143.0,
        pitch: 0.0,
        distance: 3.0,
        model_radius: 1.0,
        ..Default::default()
    }
}

/// 場面の光の強さ（3D ビューと Unity のディレクショナルライトで同じ値）。光の強さの場面のほかは 0.769（前に撮った Unity の絵の強さ）。
fn scene_intensity(name: &str) -> f32 {
    if name.starts_with("light_") {
        1.0
    } else {
        0.769
    }
}

/// 髪の場面のマットキャップの絵（自作の画像の番号）。
const BAND_MATCAP: yolu_core::ImageId =
    yolu_core::ImageId(0xA11C_A900_0000_0000_0000_0000_0000_0002);

/// 髪の場面（光の向きは名前で `scene_light` が決める）。
fn bright_backlight_scene(name: &'static str) -> Scene {
    Scene {
        name,
        meshes: sphere(),
        camera: camera(3.0, Vec3::ZERO),
        paint: |d| fill(d, [110, 80, 70, 255]),
        look: |_| {
            let mut l = lil("lilToon");
            set(&mut l, "_UseShadow", 1.0);
            set(&mut l, "_UseBacklight", 1.0);
            color(&mut l, "_BacklightColor", [16.948, 16.948, 16.948, 1.0]);
            set(&mut l, "_BacklightMainStrength", 0.5);
            set(&mut l, "_BacklightNormalStrength", 0.7);
            set(&mut l, "_BacklightBorder", 0.6);
            set(&mut l, "_BacklightBlur", 0.2);
            set(&mut l, "_BacklightDirectivity", 10.0);
            set(&mut l, "_BacklightViewStrength", 1.0);
            set(&mut l, "_BacklightReceiveShadow", 1.0);
            set(&mut l, "_BacklightBackfaceMask", 1.0);
            set(&mut l, "_UseRim", 1.0);
            color(&mut l, "_RimColor", [0.749, 0.749, 0.749, 1.0]);
            set(&mut l, "_UseMatCap", 1.0);
            l.textures
                .insert("_MatCapTex".into(), TextureSource::Image(BAND_MATCAP));
            set(&mut l, "_MatCapBlend", 1.0);
            color(&mut l, "_MatCapColor", [2.119, 1.895, 1.789, 1.0]);
            set(&mut l, "_UseReflection", 0.0);
            l
        },
        textures: band_matcap_textures,
    }
}

fn band_matcap_textures(d: &Document) -> Vec<(String, Texture)> {
    let mut t = main_texture(d);
    let (n, rgba) = band_matcap_image();
    t.push((
        "_MatCapTex".into(),
        Texture {
            width: n,
            height: n,
            rgba,
            srgb: true,
        },
    ));
    t
}

/// 髪のマットキャップの絵（暗い地に、上寄りの横の明るい帯。天使の輪の形。自作の絵）。行は下から。
fn band_matcap_image() -> (u32, Vec<u8>) {
    let n = 128u32;
    let mut out = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let u = (x as f32 + 0.5) / n as f32 * 2.0 - 1.0;
            let v = (y as f32 + 0.5) / n as f32 * 2.0 - 1.0;
            let band = (-((v - 0.35) / 0.12).powi(2)).exp() * (1.0 - u * u).max(0.0);
            let c = [0.02 + 0.4 * band, 0.02 + 0.38 * band, 0.03 + 0.36 * band];
            for k in c {
                out.push((k.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            out.push(255);
        }
    }
    (n, out)
}

/// 場面の光（来る向きの yaw・pitch、度）。髪の場面はカメラ（−Z から +Z を見る）に対して前・横・後ろ。光の強さの場面は 3D ビューの既定の向き。
fn scene_light(name: &str) -> (f32, f32) {
    match name {
        "bright_backlight_front" => (-150.0, 30.0),
        "bright_backlight_side" => (-90.0, 20.0),
        "bright_backlight_back" => (15.0, 25.0),
        n if n.starts_with("light_") => {
            let d = Display::default();
            (d.light_yaw, d.light_pitch)
        }
        _ => LIGHT,
    }
}

/// 場面がマットキャップに使う画像（文書の効果の入力の画像として渡す。棚の画像と同じ道）。
fn scene_image(name: &str) -> Option<(yolu_core::ImageId, (u32, Vec<u8>))> {
    if name.starts_with("bright_backlight") || name == "matcap_hdr" {
        Some((BAND_MATCAP, band_matcap_image()))
    } else if name.starts_with("matcap") {
        Some((
            yolu_core::ImageId(0xA11C_A900_0000_0000_0000_0000_0000_0001),
            matcap_image(),
        ))
    } else {
        None
    }
}

/// デカールのタイリング・オフセット（lilToon の欄の換算: 位置 (x, y)・大きさ `size` から）。
fn decal_st(x: f32, y: f32, size: f32) -> [f32; 4] {
    yolu_app::look::fields::decal_st([x, y, size, size])
}

/// 試しの人形（休みの形。マテリアルは 1 つにまとめる）。
fn figure() -> Vec<ModelMesh> {
    let rig = demo_figure(FigureDetail {
        sides: 24,
        ring_spacing: 0.02,
        head: 12,
    });
    rig.meshes()
        .iter()
        .map(|m| {
            let mut mesh = m.mesh.clone();
            let mut indices = Vec::new();
            for s in &mesh.submeshes {
                indices.extend_from_slice(&s.indices);
            }
            mesh.submeshes = vec![Submesh {
                material: 0,
                indices,
            }];
            mesh
        })
        .collect()
}

/// 左右の 2 枚の板（UV を左右対称に共有する: 左の板は u が外 0 → 内 1、右の板は内 1 → 外 0 の鏡。接線の向きの右手・左手が 2 枚で逆）。
fn mirrored_quads() -> ModelMesh {
    let h = 0.5;
    ModelMesh {
        name: "鏡の板".into(),
        positions: vec![
            Vec3::new(-1.0, -h, 0.0),
            Vec3::new(0.0, -h, 0.0),
            Vec3::new(-1.0, h, 0.0),
            Vec3::new(0.0, h, 0.0),
            Vec3::new(0.0, -h, 0.0),
            Vec3::new(1.0, -h, 0.0),
            Vec3::new(0.0, h, 0.0),
            Vec3::new(1.0, h, 0.0),
        ],
        normals: vec![Vec3::NEG_Z; 8],
        uvs: vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ],
        submeshes: vec![Submesh {
            material: 0,
            indices: vec![0, 2, 1, 2, 3, 1, 4, 6, 5, 6, 7, 5],
        }],
    }
}

fn quad_with_alpha_uv() -> ModelMesh {
    let h = 0.5;
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

fn harness() -> Harness<'static, YoluApp> {
    let mut h = app(WIDTH, HEIGHT, 256);
    click_tab(&mut h, yolu_app::Tab::View3d);
    move_to(&h, egui::pos2(1.0, 1.0));
    h.run();
    h
}

fn write_mesh(path: &Path, meshes: &[ModelMesh]) {
    let mut s = String::new();
    for m in meshes {
        let normals = m.vertex_normals();
        writeln!(s, "mesh {}", m.positions.len()).unwrap();
        for (i, p) in m.positions.iter().enumerate() {
            let n = normals[i];
            let uv = m.uvs.get(i).copied().unwrap_or_default();
            writeln!(
                s,
                "v {} {} {} {} {} {} {} {}",
                p.x, p.y, p.z, n.x, n.y, n.z, uv.x, uv.y
            )
            .unwrap();
        }
        for sub in &m.submeshes {
            writeln!(s, "sub {}", sub.indices.len()).unwrap();
            for t in sub.indices.chunks(3) {
                writeln!(s, "f {} {} {}", t[0], t[1], t[2]).unwrap();
            }
        }
    }
    std::fs::write(path, s).unwrap();
}

/// Unity の Quaternion.Euler(pitch, yaw, 0) を四元数の成分で。
fn camera_rotation(c: &OrbitCamera) -> Quat {
    c.rotation()
}

/// 場面を 3D ビューで描き、3D の表示域の絵を返す（光・環境・文書・見た目を組んだウィンドウも）。`view` を渡すと、3D の表示域がその大きさに
/// なるようにウィンドウの大きさを合わせる（Unity の絵と同じ投影にする。ウィンドウの中の欄の幅が変わっても同じ場面になるように）。
fn render(
    scene: &Scene,
    view: Option<(u32, u32)>,
) -> (Harness<'static, YoluApp>, image::RgbaImage) {
    render_as(scene, view, &|doc| {
        let look = (scene.look)(doc);
        doc.set_look(look, false).unwrap();
    })
}

/// `render` の、文書の見た目の当て方を選べる形（`setup` が描いた後の文書に見た目を当てる。Live Link の値で描く比べ）。
fn render_as(
    scene: &Scene,
    view: Option<(u32, u32)>,
    setup: &dyn Fn(&mut Document),
) -> (Harness<'static, YoluApp>, image::RgbaImage) {
    let mut h = harness();
    if let Some((w, hgt)) = view {
        // （ウィンドウを 1 点広げても 3D の表示域は 1 点より少ししか広がらないので、合うまで何度か繰り返す）
        for _ in 0..12 {
            let rect = h.state().view3d_rect().expect("3D のタブ");
            let (dw, dh) = (
                w as f32 - rect.width().round(),
                hgt as f32 - rect.height().round(),
            );
            if dw == 0.0 && dh == 0.0 {
                break;
            }
            let size = h.ctx.content_rect().size() + egui::vec2(dw, dh);
            h.set_size(size);
            h.run();
        }
        let rect = h.state().view3d_rect().expect("3D のタブ");
        assert_eq!(
            (rect.width().round(), rect.height().round()),
            (w as f32, hgt as f32),
            "12 回の調整で、3D の表示域が指定の大きさ（{w}×{hgt}）に届かない"
        );
    }
    {
        let state = &mut h.state_mut().state;
        let revision = state.view3d.next_revision();
        state.view3d.material = 0;
        let model = ViewModel::new(
            "場面",
            scene.meshes.clone(),
            vec![Some("場面".to_string())],
            revision,
        )
        .unwrap();
        model.tangents();
        state.view3d.set_model(model);
        state.view3d.camera = scene.camera;
    }
    h.run();
    let light = scene_light(scene.name);
    h.state_mut().apply(Action::View3d(Op::LightYaw(light.0)));
    h.state_mut().apply(Action::View3d(Op::LightPitch(light.1)));
    h.state_mut()
        .apply(Action::View3d(Op::LightIntensity(scene_intensity(
            scene.name,
        ))));
    // Unity の参照の絵は多サンプルなし。縁の比べ（片方だけの画素・平均）が多サンプルでずれないよう、この比べは 1× で描く
    h.state_mut().apply(Action::View3d(Op::Antialias(1)));
    if uniform_env(scene.name) {
        h.state_mut().state.view3d.display.ambient = [1.0; 3];
        h.state_mut().apply(Action::View3d(Op::Env(EnvKind::None)));
    } else {
        h.state_mut().apply(Action::View3d(Op::Env(EnvKind::Sky)));
    }
    {
        let doc = &mut h.state_mut().state.doc;
        (scene.paint)(doc);
        setup(doc);
    }
    // マットキャップの絵は文書の効果の入力の画像として渡す（棚の画像と同じ道）
    if let Some((id, (n, rgba))) = scene_image(scene.name) {
        let image = ImageInput::new(n, n, rgba, ImageColorSpace::Srgb).unwrap();
        let doc = &mut h.state_mut().state.doc;
        let inputs = doc.effect_inputs().clone().with_image(id, image);
        doc.set_effect_inputs(inputs).unwrap();
    }
    h.run();
    h.run();
    let image = h.render().expect("描ける");
    let rect = h.state().view3d_rect().expect("3D のタブ");
    let (x0, y0) = (rect.left().round() as u32, rect.top().round() as u32);
    let (w, hgt) = (rect.width().round() as u32, rect.height().round() as u32);
    let crop = image::imageops::crop_imm(&image, x0, y0, w, hgt).to_image();
    (h, crop)
}

#[test]
#[ignore = "手で回す（LILTOON_REF_DIR に書く）"]
fn export_and_render() {
    let Some(dir) = out_dir() else {
        eprintln!("LILTOON_REF_DIR が無いので書かない");
        return;
    };
    std::fs::create_dir_all(&dir).unwrap();
    let sky = environment::bake(&Source::Sky(SkyColors::default()));
    let mut desc = String::new();
    // `LILTOON_REF_ONLY`（場面の名前の並び）で絞れる（Unity の OpenGL はシェーダーのテクスチャの数に上限があり、場面の全部が使う
    // テクスチャの機能を一度に入れられない）
    let only: Option<Vec<String>> = std::env::var("LILTOON_REF_ONLY")
        .ok()
        .map(|v| v.split(',').map(str::to_owned).collect());
    for scene in scenes() {
        if only
            .as_ref()
            .is_some_and(|o| !o.iter().any(|n| n == scene.name))
        {
            continue;
        }
        let (h, crop) = render(&scene, None);
        let (w, hgt) = crop.dimensions();
        crop.save(dir.join(format!("ours_{}.png", scene.name)))
            .unwrap();
        // Unity の場面の記述
        let display = h.state().state.view3d.display;
        let to_light = display.light_direction();
        let cam = scene.camera;
        let q = camera_rotation(&cam);
        let pos = cam.position();
        writeln!(desc, "scene {}", scene.name).unwrap();
        writeln!(desc, "size {w} {hgt}").unwrap();
        writeln!(
            desc,
            "camera {} {} {} {} {} {} {} 30",
            pos.x, pos.y, pos.z, q.x, q.y, q.z, q.w
        )
        .unwrap();
        writeln!(
            desc,
            "light {} {} {} {}",
            to_light.x, to_light.y, to_light.z, display.light_intensity
        )
        .unwrap();
        let mut coefficients = sky.sh;
        if uniform_env(scene.name) {
            coefficients = [Vec3::ZERO; 9];
            coefficients[0] = Vec3::splat(uniform_env_color());
        }
        let sh: Vec<String> = coefficients
            .iter()
            .flat_map(|c| [c.x, c.y, c.z])
            .map(|v| v.to_string())
            .collect();
        writeln!(desc, "sh {}", sh.join(" ")).unwrap();
        writeln!(desc, "background 0.12 0.13 0.15").unwrap();
        let mesh_file = format!("{}.mesh.txt", scene.name);
        write_mesh(&dir.join(&mesh_file), &scene.meshes);
        writeln!(desc, "mesh {mesh_file}").unwrap();
        let doc = &h.state().state.doc;
        let look = doc.look().clone();
        let standard = look.kind == LookKind::Standard;
        if standard {
            // 3D ビューの標準は Unity の Standard（色は塗った絵だけ、金属 0、滑らかさは使っていない Roughness の既定 128 の 1 − 値）
            writeln!(desc, "shader Standard").unwrap();
            writeln!(desc, "color _Color 1 1 1 1").unwrap();
            writeln!(desc, "float _Metallic 0").unwrap();
            writeln!(desc, "float _Glossiness {}", 1.0 - 128.0 / 255.0).unwrap();
        } else {
            writeln!(desc, "shader {}", look.shader_name()).unwrap();
        }
        for (name, value) in &look.properties {
            match value {
                LookValue::Float(v) => writeln!(desc, "float {name} {v}").unwrap(),
                LookValue::Int(v) => writeln!(desc, "float {name} {v}").unwrap(),
                LookValue::Color(c) => {
                    writeln!(desc, "color {name} {} {} {} {}", c[0], c[1], c[2], c[3]).unwrap()
                }
                LookValue::Vector(c) => {
                    writeln!(desc, "vector {name} {} {} {} {}", c[0], c[1], c[2], c[3]).unwrap()
                }
            }
        }
        for (slot, tex) in (scene.textures)(doc) {
            let file = format!("{}_{slot}.rgba", scene.name);
            std::fs::write(dir.join(&file), &tex.rgba).unwrap();
            writeln!(
                desc,
                "texture {slot} {file} {} {} {}",
                tex.width,
                tex.height,
                u8::from(tex.srgb)
            )
            .unwrap();
        }
        // テクスチャを割り当てずに既定のテクスチャで読む機能（異方性反射の接線のマップ: Unity の既定の bump で、接線が斜めになる）
        if yolu_app::look::liltoon::on(&look, "_UseAnisotropy") {
            writeln!(desc, "feature AnisotropyTangentMap").unwrap();
        }
        // 一様な環境: Unity はマテリアルのキューブマップの差し替えで、3D ビューの環境「なし」と同じ色を映す
        if uniform_env(scene.name) && standard {
            // Standard は場面の既定の反射（反射プローブを使わない物が映す空）を、同じ一様な色のキューブマップにする
            let c = uniform_env_color();
            writeln!(desc, "reflection {c} {c} {c}").unwrap();
        } else if uniform_env(scene.name) {
            let c = uniform_env_color();
            writeln!(desc, "cube _ReflectionCubeTex {c} {c} {c}").unwrap();
            writeln!(desc, "float _ReflectionCubeOverride 1").unwrap();
            writeln!(desc, "color _ReflectionCubeColor 1 1 1 1").unwrap();
            writeln!(desc, "float _ReflectionCubeEnableLighting 0").unwrap();
            writeln!(desc, "feature ReflectionCubeTex").unwrap();
        }
        writeln!(desc, "end").unwrap();
    }
    std::fs::write(dir.join("scenes.txt"), desc).unwrap();
    eprintln!("書いた: {}", dir.display());
}

/// 2 枚の絵の差（両方で物の画素だけ。背景の色から 3 より離れた画素を物とみなす）。skip（絵の中の画素の矩形）の中は比べない
/// （3D の絵の上に重ねた軸の印）。
fn diff(
    ours: &image::RgbaImage,
    unity: &image::RgbaImage,
    background: [u8; 3],
    skip: Option<egui::Rect>,
) -> Option<(f64, f64, u8, usize, usize)> {
    if ours.dimensions() != unity.dimensions() {
        return None;
    }
    let is_bg = |p: [u8; 4]| (0..3).all(|k| p[k].abs_diff(background[k]) <= 3);
    let mut diffs: Vec<u8> = Vec::new();
    let mut only_one = 0usize;
    for ((x, y, a), b) in ours.enumerate_pixels().zip(unity.pixels()) {
        if skip.is_some_and(|r| r.contains(egui::pos2(x as f32, y as f32))) {
            continue;
        }
        let (ba, bb) = (is_bg(a.0), is_bg(b.0));
        if ba && bb {
            continue;
        }
        if ba != bb {
            only_one += 1;
            continue;
        }
        let d = (0..3).map(|k| a.0[k].abs_diff(b.0[k])).max().unwrap();
        diffs.push(d);
    }
    if diffs.is_empty() {
        return None;
    }
    let mean = diffs.iter().map(|d| *d as f64).sum::<f64>() / diffs.len() as f64;
    let mut sorted = diffs.clone();
    sorted.sort_unstable();
    let p95 = sorted[(sorted.len() * 95 / 100).min(sorted.len() - 1)] as f64;
    Some((mean, p95, *sorted.last().unwrap(), diffs.len(), only_one))
}

#[test]
#[ignore = "手で回す（LILTOON_REF_DIR の ours_*.png と unity_*.png を比べる）"]
fn compare() {
    let Some(dir) = out_dir() else {
        return;
    };
    let background = [31u8, 33, 38]; // (0.12, 0.13, 0.15) × 255
    println!("| 場面 | 画素の差の平均 | 95 % | 最大 | 比べた画素 | 片方だけの画素 |");
    println!("|---|---:|---:|---:|---:|---:|");
    for scene in scenes() {
        let ours = image::open(dir.join(format!("ours_{}.png", scene.name)));
        let unity = image::open(dir.join(format!("unity_{}.png", scene.name)));
        let (Ok(ours), Ok(unity)) = (ours, unity) else {
            println!("| {} | （絵が無い） | | | | |", scene.name);
            continue;
        };
        let (ours, unity) = (ours.to_rgba8(), unity.to_rgba8());
        match diff(&ours, &unity, background, None) {
            Some((mean, p95, max, n, only)) => println!(
                "| {} | {mean:.2} | {p95:.0} | {max} | {n} | {only} |",
                scene.name
            ),
            None => println!("| {} | （大きさが違う） | | | | |", scene.name),
        }
        // 差の絵（差 × 4）
        if ours.dimensions() == unity.dimensions() {
            let mut d = image::RgbaImage::new(ours.width(), ours.height());
            for ((a, b), o) in ours.pixels().zip(unity.pixels()).zip(d.pixels_mut()) {
                for k in 0..3 {
                    o.0[k] = (a.0[k].abs_diff(b.0[k]) as u32 * 4).min(255) as u8;
                }
                o.0[3] = 255;
            }
            d.save(dir.join(format!("diff_{}.png", scene.name)))
                .unwrap();
        }
    }
}

/// Live Link が送る値（`tools/liltoon-reference.cs` が Unity のパッケージで読んで書いた `link_<名前>.txt` と絵）から、スタンドアロンが
/// 頼みを受けたときと同じ受けた見た目（`look::link::received_look`）を作る。lilToon と判定されなければ None。
fn link_received(dir: &Path, name: &str) -> Option<yolu_core::look::ReceivedLook> {
    use yolu_protocol::files::{Material, TextureRef};
    let text = std::fs::read_to_string(dir.join(format!("link_{name}.txt"))).ok()?;
    let mut material = Material {
        key: String::new(),
        name: name.to_owned(),
        shader: Default::default(),
        values: Default::default(),
        textures: Vec::new(),
    };
    let mut images = std::collections::BTreeMap::new();
    let mut missing = std::collections::BTreeMap::new();
    let slot = |property: &str| TextureRef {
        property: property.to_owned(),
        path: None,
        guid: String::new(),
        srgb: true,
        normal_map: false,
        scale: [1.0, 1.0],
        offset: [0.0, 0.0],
    };
    for line in text.lines() {
        let p: Vec<&str> = line.split(' ').collect();
        let f = |i: usize| p[i].parse::<f32>().unwrap();
        match p[0] {
            "none" => return None,
            "shader" => {
                material.shader.name = line["shader ".len()..].to_owned();
                // 値を書くのは Unity の側で lilToon と確かめたマテリアルだけ（確かめられなければ none）
                material.shader.package = yolu_app::look::link::LILTOON_PACKAGE.to_owned();
            }
            "prop" => {
                let v = [f(3), f(4), f(5), f(6)];
                let name = p[2].to_owned();
                let values = &mut material.values;
                match p[1] {
                    "0" => {
                        values.floats.insert(name, v[0]);
                    }
                    "1" => {
                        values.ints.insert(name, v[0] as i32);
                    }
                    "2" => {
                        values.colors.insert(name, v);
                    }
                    _ => {
                        values.vectors.insert(name, v);
                    }
                }
            }
            "keyword" => material.shader.keywords.push(p[1].to_owned()),
            "slot" if p.len() == 3 => {
                material.textures.push(slot(p[1]));
                if p[2] != "empty" {
                    missing.insert(p[1].to_owned(), yolu_core::look::MissingImage::Unreadable);
                }
            }
            "slot" => {
                let (w, h) = (p[2].parse().unwrap(), p[3].parse().unwrap());
                material.textures.push(slot(p[1]));
                let pixels = std::fs::read(dir.join(p[5])).unwrap();
                images.insert(
                    p[1].to_owned(),
                    std::sync::Arc::new(yolu_core::look::ReceivedImage {
                        width: w,
                        height: h,
                        srgb: p[4] == "1",
                        pixels: pixels.into(),
                    }),
                );
            }
            _ => {}
        }
    }
    yolu_app::look::link::received_look(&material, &images, &missing)
}

#[test]
#[ignore = "手で回す（LILTOON_REF_DIR の link_*.txt の値だけで描き、unity_*.png と比べる）"]
fn compare_through_live_link() {
    let Some(dir) = out_dir() else {
        return;
    };
    let background = [31u8, 33, 38];
    println!("| 場面 | 画素の差の平均 | 95 % | 最大 | 比べた画素 | 片方だけの画素 | 受けた絵 |");
    println!("|---|---:|---:|---:|---:|---:|---:|");
    for scene in scenes() {
        let Ok(unity) = image::open(dir.join(format!("unity_{}.png", scene.name))) else {
            println!("| {} | （Unity の絵が無い） | | | | | |", scene.name);
            continue;
        };
        let unity = unity.to_rgba8();
        let Some(received) = link_received(&dir, scene.name) else {
            println!("| {} | （lilToon と判定されない） | | | | | |", scene.name);
            continue;
        };
        let images = received.images.len();
        // 利用者の設定は既定のまま（標準）、受けた見た目だけで描く（Live Link でつないだ直後と同じ）
        let (h, ours) = render_as(&scene, Some(unity.dimensions()), &|doc| {
            doc.set_received_look(Some(received.clone())).unwrap();
        });
        ours.save(dir.join(format!("link_{}.png", scene.name)))
            .unwrap();
        match diff(&ours, &unity, background, axes_in_view(&h)) {
            Some((mean, p95, max, n, only)) => println!(
                "| {} | {mean:.2} | {p95:.0} | {max} | {n} | {only} | {images} |",
                scene.name
            ),
            None => println!(
                "| {} | （大きさが違う: {:?} と Unity の {:?}） | | | | | |",
                scene.name,
                ours.dimensions(),
                unity.dimensions()
            ),
        }
    }
}

/// Unity の lilToon の絵（`tests/liltoon_unity/unity_<名前>.png`）。合成の素材（球・板・試しの人形・縞の絵）だけの場面を、
/// Unity 2022.3.22f1・lilToon 2.3.4（Linux のエディタの OpenGL、コンテナの実 GPU）で `tools/liltoon-reference.cs` が撮ったもの
/// （2026-10-05。アルファは捨てて RGB で持つ）。場面を変えたら撮り直し、差の上限（`BOUNDS`）も測り直す。
fn unity_image(name: &str) -> image::RgbaImage {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/liltoon_unity")
        .join(format!("unity_{name}.png"));
    image::open(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .to_rgba8()
}

/// 軸の印の、3D の表示域の絵（`render` の切り抜き）の中の矩形。
fn axes_in_view(h: &Harness<'_, YoluApp>) -> Option<egui::Rect> {
    let rect = h.state().view3d_rect()?;
    let min = egui::pos2(rect.left().round(), rect.top().round());
    view3d_axes_rect(h).map(|r| r.translate(-min.to_vec2()))
}

/// 場面ごとの差の上限: 平均（実 GPU）・平均（ソフトの描画）・95 % の値（0〜255）と、片方だけに物が写っている画素の数。
/// - 平均（実 GPU）: コンテナの実 GPU（wgpu の GL を Mesa d3d12 へ）で測った値に 0.3 ほどの余裕を足したもの（ソフトの描画より大きくしない）。
///   Windows の D3D12・macOS の Metal では測っていない。`transparent` は実 GPU ではガンマで重ねる道（GL）しか測れていないので、リニアで
///   重ねる道の上限はソフトの描画と同じ。
/// - 平均（ソフトの描画）: llvmpipe（Vulkan と GL）だけに許す上限。塗った絵（Color・Emission）とマットキャップの sRGB の画像は sRGB の
///   形式で GPU がリニアに直して読み、llvmpipe のその直し方は近似で、一様な色の所でも Unity（実 GPU）と 1 ずれる（平均が 1 近くまで上がる
///   場面がある。実 GPU では同じ場面の平均が rim 0.06・backface 0.00・anisotropy 0.03・emission 0.31）。llvmpipe の Vulkan と GL で測った値に余裕を足したもの。
/// - 95 %・片方だけ: 両方で同じ上限（llvmpipe の Vulkan・GL と実 GPU の GL で測った値に余裕を足したもの）。
///
/// `transparent` は、描き先の sRGB の見え方を作れない機材（GL）ではガンマの値のまま重ねるので別の上限（`TRANSPARENT_GAMMA`）。
const BOUNDS: &[(&str, f64, f64, f64, usize)] = &[
    ("shadow", 0.4, 0.5, 2.0, 2_600),
    ("shadow_3rd", 0.4, 0.6, 2.0, 2_600),
    ("shadow_mask", 0.4, 0.6, 2.0, 2_600),
    ("rim", 0.4, 1.3, 2.0, 2_600),
    ("matcap", 0.5, 0.8, 2.0, 2_600),
    ("outline", 0.4, 0.6, 2.0, 2_600),
    ("emission", 0.7, 1.4, 2.0, 2_600),
    ("figure", 0.4, 1.3, 2.0, 2_600),
    ("cutout", 0.9, 1.4, 2.0, 2_600),
    ("transparent", 1.4, 1.4, 2.0, 2_600),
    ("backface", 0.3, 1.3, 1.0, 2_600),
    ("normal", 0.7, 0.8, 2.0, 2_600),
    ("tone", 0.4, 0.5, 1.0, 2_600),
    ("sdf", 0.8, 1.0, 2.0, 2_600),
    ("ao_flat", 0.4, 0.6, 2.0, 2_600),
    ("emission_rim", 4.0, 4.5, 30.0, 2_600),
    ("matcap2", 0.6, 1.1, 2.0, 2_600),
    ("outline_mask", 0.4, 0.5, 2.0, 2_600),
    ("rim_shade", 0.6, 0.8, 2.0, 2_600),
    ("backlight", 0.4, 0.7, 2.0, 2_600),
    ("specular_toon", 0.4, 0.4, 2.0, 2_600),
    ("specular_real", 0.5, 0.6, 2.0, 2_600),
    ("main2nd_decal", 0.6, 0.9, 2.0, 2_600),
    ("main3rd_matcap", 0.5, 1.1, 2.0, 2_600),
    ("normal2nd", 0.5, 0.5, 2.0, 2_600),
    ("glitter", 0.4, 0.6, 2.0, 2_600),
    ("anisotropy", 0.4, 1.3, 2.0, 2_600),
    ("distance_fade", 0.5, 0.8, 2.0, 2_600),
    ("outline_tex", 0.5, 0.5, 2.0, 2_600),
    ("uv", 1.1, 1.2, 7.0, 2_600),
    ("reflection_env", 0.5, 1.0, 2.0, 2_600),
    ("decal_mirror", 0.5, 0.6, 2.0, 2_600),
    ("distance_fade_object", 0.4, 1.1, 2.0, 2_600),
    ("matcap_normal", 0.5, 0.7, 2.0, 2_600),
    ("matcap_hdr", 0.4, 1.1, 2.0, 2_600),
    ("bright_backlight_front", 0.5, 1.1, 2.0, 2_600),
    ("bright_backlight_side", 0.5, 1.1, 2.0, 2_600),
    ("bright_backlight_back", 0.5, 0.9, 2.0, 2_600),
    ("light_default", 0.3, 0.3, 1.0, 2_600),
    ("light_shadow", 0.4, 0.4, 1.0, 2_600),
    ("light_standard", 0.4, 0.4, 1.0, 2_600),
];
/// GL（ガンマの値のまま重ねる）の `transparent` の上限。llvmpipe と実 GPU の GL で同じ値（平均 28.16〜28.19・95 % 42・片方だけ 5,623）。
/// 片方だけの画素は、Rust 版のビューの隅のボタン（2,207）と、アルファが 0 に近い薄い縁の帯（llvmpipe の GL で 3,416 = 7 列 × 488 行）: ガンマで重ねると
/// リニアで重ねる Unity より薄い縁が暗く、背景から 3 以内（背景とみなす）になる。余裕は 1 列（488）ほど。
const TRANSPARENT_GAMMA: (f64, f64, usize) = (30.0, 45.0, 6_100);

#[test]
fn the_view_stays_within_the_measured_difference_from_unity_liltoon() {
    let background = [31u8, 33, 38]; // (0.12, 0.13, 0.15) × 255
    let mut failures = Vec::new();
    let scenes = scenes();
    assert_eq!(scenes.len(), BOUNDS.len());
    for scene in &scenes {
        let unity = unity_image(scene.name);
        let (h, ours) = render(scene, Some(unity.dimensions()));
        let linear = h.state().view3d_stats().unwrap().linear_transparent;
        // ソフトの描画（llvmpipe）か（GPU の名前の最後の「(API, 種類)」の種類が Cpu）
        let adapter = h.state().view3d_adapter().expect("wgpu の 3D");
        let software = adapter.ends_with("Cpu)");
        if scene.name == scenes[0].name {
            println!("{adapter}");
        }
        let (mean, p95, max, n, only) = diff(&ours, &unity, background, axes_in_view(&h))
            .unwrap_or_else(|| {
                panic!(
                    "{}: 大きさが違う（{:?} と Unity の {:?}）",
                    scene.name,
                    ours.dimensions(),
                    unity.dimensions()
                )
            });
        let (_, gpu_mean, software_mean, bp, bo) = *BOUNDS
            .iter()
            .find(|b| b.0 == scene.name)
            .expect("上限がある");
        let bm = if software { software_mean } else { gpu_mean };
        let (bm, bp, bo) = if scene.name == "transparent" && !linear {
            TRANSPARENT_GAMMA
        } else {
            (bm, bp, bo)
        };
        println!(
            "| {} | {mean:.2} | {p95:.0} | {max} | {n} | {only} |",
            scene.name
        );
        if mean > bm || p95 > bp || only > bo {
            failures.push(format!(
                "{}: 平均 {mean:.2}（上限 {bm}）・95 % {p95}（上限 {bp}）・片方だけ {only}（上限 {bo}）",
                scene.name
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "Unity の lilToon との差が上限を超えた: {failures:#?}"
    );
}

/// 光の強さの場面の見る点（3D の表示域の画素。光の真正面の側・光と影の間・光の届かない側）と、Unity の絵との許す差（0〜255）。
/// 測った差は lavapipe（Vulkan）・llvmpipe の GL・コンテナの実 GPU の GL（Mesa d3d12）の 3 つとも 0（2026-10-07）。上限 2 は、測っていない
/// 機材（Windows の D3D12・macOS の Metal）の補間の違いへの余裕。
const LIGHT_PROBES: [(u32, u32); 4] = [(380, 230), (300, 200), (200, 300), (150, 420)];
const LIGHT_PROBE_TOLERANCE: u8 = 2;

/// 3D ビューの光の強さ 1 が、Unity のディレクショナルライト（白・強さ 1）と同じ明るさ: 白いテクスチャの lilToon（既定の値）は明るい側が
/// 白（FF）になり、影ありの lilToon の明るい側・影の側と、Standard の光の正面から影の側までの見る点が Unity の絵と合う。
#[test]
fn light_of_strength_one_matches_a_unity_directional_light() {
    let scenes = scenes();
    for name in ["light_default", "light_shadow", "light_standard"] {
        let scene = scenes.iter().find(|s| s.name == name).expect("場面");
        let unity = unity_image(name);
        let (_h, ours) = render(scene, Some(unity.dimensions()));
        let at = |img: &image::RgbaImage, (x, y): (u32, u32)| -> [u8; 3] {
            let p = img.get_pixel(x, y).0;
            [p[0], p[1], p[2]]
        };
        for probe in LIGHT_PROBES {
            let (a, b) = (at(&ours, probe), at(&unity, probe));
            println!("{name} {probe:?}: 3D ビュー {a:?}・Unity {b:?}");
            assert!(
                (0..3).all(|k| a[k].abs_diff(b[k]) <= LIGHT_PROBE_TOLERANCE),
                "{name} の {probe:?}: 3D ビュー {a:?}・Unity {b:?}（許す差 {LIGHT_PROBE_TOLERANCE}）"
            );
        }
        // 光の真正面の側は、白いテクスチャがそのまま白
        if name != "light_standard" {
            assert_eq!(at(&ours, LIGHT_PROBES[0]), [255; 3], "{name} の明るい側");
        }
    }
}
