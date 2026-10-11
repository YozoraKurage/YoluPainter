//! 3D の面のストロークの画素の指紋: 同じ線を、いろいろなブラシ・対称・ステンシル・複数チャンネルで引いた結果の SHA-256 の先頭 10 バイトが、
//! ダブの画素を文書へまとめて渡す前（画素ごとに渡していた）・投影の画素を区画ごとに位置順に持つ前の版で測った値と同じであること、
//! スレッドの数（1 と 8）によらず同じであることを確かめる。速くする直しは、塗る式も順も変えないので、指紋は動かない。
//! 塗りを意図して変えたときは、理由を決めてから表を更新する。
//!
//! 正投影で固定したカメラ（回転なし）で引くので、三角関数を通らない（OS の数学関数の違いで最後の桁が動かない）。

use std::sync::Arc;

use sha2::{Digest, Sha256};
use yolu_core::geometry::{
    cube_sphere, model_triangles, CameraView, MirrorPlane, Projection, SurfaceGeometry,
    SurfaceStencil, SurfaceStroke, SurfaceStrokeOptions, SurfaceSymmetrySetup, SymmetryAxis,
    DEFAULT_WELD_TOLERANCE,
};
use yolu_core::glam::{Quat, Vec2, Vec3};
use yolu_core::material::ChannelPaint;
use yolu_core::{
    builtin_tip, AntiAlias, Brush, BrushSettings, BrushStencil, Channel, Document, ImageColorSpace,
    PaperTexture, Rgba8, StencilImage, StencilMapping, StencilMode, StencilTiling,
};

const SIDE: u32 = 256;
const VIEW: (f32, f32) = (640.0, 480.0);

fn geometry() -> Arc<SurfaceGeometry> {
    let triangles = model_triangles(&[cube_sphere(18, 0.5)]).unwrap();
    Arc::new(SurfaceGeometry::new(triangles, 1, DEFAULT_WELD_TOLERANCE).unwrap())
}

/// 球を正面から見る正投影のカメラ（縦に 1.2 の長さ）。
fn camera() -> CameraView {
    CameraView {
        position: Vec3::new(0.0, 0.0, -3.0),
        rotation: Quat::IDENTITY,
        forward: Vec3::Z,
        near: 0.1,
        far: 100.0,
        width: VIEW.0,
        height: VIEW.1,
        projection: Projection::Orthographic { height: 1.2 },
    }
}

fn brush() -> Brush {
    let mut b = Brush::from(BrushSettings {
        radius: 14.0,
        hardness: 0.8,
        spacing: 0.15,
        color: Rgba8::new(200, 60, 30, 255),
        pressure_size: true,
        pressure_opacity: true,
        ..BrushSettings::default()
    });
    b.seed = 7;
    b
}

struct Variant {
    name: &'static str,
    brush: Brush,
    options: SurfaceStrokeOptions,
    material: bool,
    prefill: bool,
}

fn variants() -> Vec<Variant> {
    let plain = |name, brush: Brush| Variant {
        name,
        brush,
        options: SurfaceStrokeOptions::default(),
        material: false,
        prefill: false,
    };
    let mut out = vec![plain("plain", brush())];
    let mut b = brush();
    b.base.hardness = 0.0;
    b.base.flow = 0.3;
    b.base.opacity = 0.8;
    out.push(plain("soft_flow", b));
    let mut b = brush();
    b.base.erase = true;
    out.push(Variant {
        prefill: true,
        ..plain("erase", b)
    });
    let mut b = brush();
    b.base.flow = 0.5;
    b.texture = Some(PaperTexture {
        scale: 2.0,
        ..PaperTexture::new(builtin_tip("grain").unwrap(), 0.6)
    });
    out.push(plain("texture", b));
    let mut b = brush();
    b.base.hardness = 1.0;
    b.base.anti_alias = AntiAlias::Strong;
    out.push(plain("anti_alias", b));
    out.push(Variant {
        material: true,
        ..plain("material", brush())
    });
    let mirror = MirrorPlane::from_model(Vec3::ZERO, Quat::IDENTITY, SymmetryAxis::X, 0.0);
    out.push(Variant {
        options: SurfaceStrokeOptions {
            symmetry: Some(SurfaceSymmetrySetup {
                mirror: Some(mirror),
                radial: None,
                ignore_visibility: false,
            }),
            ..SurfaceStrokeOptions::default()
        },
        ..plain("mirror", brush())
    });
    // ステンシル（左から右へ量が増える画像を、画面の幅いっぱいに）
    let mut rgba = Vec::new();
    for _ in 0..8 {
        for x in 0..64u32 {
            let v = (x * 4) as u8;
            rgba.extend_from_slice(&[v, v, v, 255]);
        }
    }
    let image = StencilImage::new(
        64,
        8,
        rgba,
        ImageColorSpace::Srgb,
        StencilImage::DEFAULT_MIP_BUDGET_BYTES,
    )
    .unwrap();
    let mut b = brush();
    b.stencil = Some(Arc::new(BrushStencil::new(
        Arc::new(image),
        StencilMode::Mask,
        StencilTiling::None,
        false,
        None,
        &[],
    )));
    let scale = 64.0 / VIEW.0 as f64;
    out.push(Variant {
        options: SurfaceStrokeOptions {
            stencil: Some(
                SurfaceStencil::new(
                    StencilMapping::new(scale, 0.0, 0.0, 0.0, 0.0, 4.0).unwrap(),
                    scale,
                )
                .unwrap(),
            ),
            ..SurfaceStrokeOptions::default()
        },
        ..plain("stencil", b)
    });
    out
}

/// 速い動き（1 回の入力が長く、区間のダブが多い）と短い動きを混ぜた線を引いて、全チャンネルの画素の指紋を返す。
fn digest(g: &Arc<SurfaceGeometry>, v: &Variant, threads: usize) -> String {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap();
    pool.install(|| {
        let view = camera();
        let mut doc = Document::new(SIDE, SIDE).unwrap();
        doc.set_stroke_budget_bytes(512 << 20).unwrap();
        let layer = doc.add_layer("a").unwrap();
        if v.prefill {
            for y in 0..SIDE {
                for x in 0..SIDE {
                    let c = Rgba8::new((x * 3) as u8, (y * 5) as u8, ((x ^ y) * 7) as u8, 255);
                    doc.set_pixel(layer, x, y, c).unwrap();
                }
            }
            doc.clear_history().unwrap();
        }
        let materials: Vec<ChannelPaint> = [Channel::Color, Channel::Roughness, Channel::Normal]
            .iter()
            .enumerate()
            .map(|(i, c)| ChannelPaint::new(*c, Rgba8::new(31 + i as u8 * 60, 79, 133, 211)))
            .collect();
        let mut stroke = if v.material {
            doc.begin_material_brush_stroke(layer, &materials, &v.brush)
                .unwrap()
        } else {
            doc.begin_brush_stroke(layer, &v.brush).unwrap()
        };
        let c = Vec2::new(view.width * 0.5, view.height * 0.5);
        let r = view.height * 0.3;
        let mut s = SurfaceStroke::begin_with_options(
            &mut doc,
            &mut stroke,
            g.clone(),
            view,
            &v.brush.base,
            Some(0),
            c + Vec2::new(-r, -r * 0.2),
            0.6,
            v.options,
        )
        .unwrap();
        let path = [
            (c + Vec2::new(r, -r * 0.4), 0.9),
            (c + Vec2::new(r * 0.9, -r * 0.3), 1.0),
            (c + Vec2::new(-r, r * 0.1), 0.7),
            (c + Vec2::new(-r * 0.8, r * 0.3), 0.5),
            (c + Vec2::new(r, r * 0.5), 1.0),
            (c + Vec2::new(r * 0.2, r * 0.9), 0.8),
            (c + Vec2::new(-r * 0.6, r * 0.7), 0.6),
        ];
        for (p, pressure) in path {
            s.add(&mut doc, &mut stroke, p, pressure).unwrap();
        }
        s.finish(&mut doc, &mut stroke).unwrap();
        assert!(s.stats.dabs > 100, "{}: {:?}", v.name, s.stats);
        doc.end_stroke(stroke).unwrap();
        let mut hash = Sha256::new();
        let channels: Vec<Channel> = if v.material {
            materials.iter().map(|m| m.channel).collect()
        } else {
            vec![Channel::Color]
        };
        for channel in channels {
            let surface = doc.layer(layer).unwrap().surface(channel).unwrap();
            hash.update(surface.to_canvas_bytes());
        }
        hash.finalize()
            .iter()
            .take(10)
            .map(|b| format!("{b:02x}"))
            .collect()
    })
}

/// 速くする直しの前の版（画素ごとに文書へ渡す・投影の画素の候補を全部比べて並べる）で測った指紋。
const EXPECTED: [(&str, &str); 8] = [
    ("plain", "a846762e6fa7cfb20199"),
    ("soft_flow", "980309169a095990a702"),
    ("erase", "3d5879501e9cf55a270d"),
    ("texture", "310d2fb71f99b153f4c4"),
    ("anti_alias", "12b60d39c0bb830dd698"),
    ("material", "fda24953542e291090e7"),
    ("mirror", "ef63fb8cc515acee2e98"),
    ("stencil", "81781b345c4e8dab5966"),
];

#[test]
fn strokes_paint_the_same_bytes_as_before_the_speed_ups_whatever_the_threads() {
    let g = geometry();
    let print = std::env::var_os("YOLU_PRINT_DIGESTS").is_some();
    let mut wrong = vec![];
    for v in variants() {
        let one = digest(&g, &v, 1);
        let many = digest(&g, &v, 8);
        if print {
            println!("    (\"{}\", \"{one}\"),", v.name);
        }
        assert_eq!(one, many, "{}: スレッドの数で変わった", v.name);
        let expected = EXPECTED.iter().find(|(n, _)| *n == v.name).unwrap().1;
        if one != expected {
            wrong.push(format!("{}: {one}（期待 {expected}）", v.name));
        }
    }
    assert!(print || wrong.is_empty(), "{wrong:#?}");
}
