#![allow(clippy::chunks_exact_to_as_chunks)]
use crate::golden_update;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use yolu_core::{
    glam::DVec2,
    material::{ChannelPaint, GradientSettings, GradientShape},
    material_triangles::PixelTriangle,
    *,
};
/// 画布の場面（tools/csharp-golden/MaterialGolden.cs の Scenes と同じ）。mat は画布がタイルの整数倍、edge は端が欠けたタイルに
/// 画素と三角形が掛かる。
struct Scene {
    name: &'static str,
    w: u32,
    h: u32,
    triangles: [PixelTriangle; 3],
    points: [(f64, f64, f64, f64); 3],
    ellipse: (f64, f64, f64, f64),
    gradient: (DVec2, DVec2),
}
fn tri(t: [(f64, f64); 3]) -> PixelTriangle {
    t.map(|(x, y)| DVec2::new(x, y))
}
fn scenes() -> [Scene; 2] {
    [
        Scene {
            name: "mat",
            w: 32,
            h: 24,
            triangles: [
                tri([(1.2, 2.3), (30.2, 3.1), (25.4, 21.6)]),
                tri([(1.2, 2.3), (25.4, 21.6), (2.1, 22.2)]),
                tri([(9.7, 0.1), (31.8, 19.7), (4.9, 17.2)]),
            ],
            points: [
                (3.2, 4.3, 0.4, 0.0),
                (16.7, 13.2, 1.0, 0.1),
                (29.1, 20.6, 0.7, 0.2),
            ],
            ellipse: (17.3, 11.7, 12.4, 8.6),
            gradient: (DVec2::new(3.2, 5.7), DVec2::new(27.1, 18.9)),
        },
        Scene {
            name: "edge",
            w: 37,
            h: 29,
            triangles: [
                tri([(2.5, 1.5), (44.0, 3.0), (30.5, 35.5)]),
                tri([(-6.2, 10.0), (14.1, -4.4), (20.3, 31.8)]),
                tri([(30.1, 5.2), (36.9, 24.1), (34.4, 28.7)]),
            ],
            points: [
                (2.2, 3.1, 0.4, 0.0),
                (21.7, 15.2, 1.0, 0.1),
                (35.1, 27.6, 0.7, 0.2),
            ],
            ellipse: (28.3, 19.7, 11.4, 8.6),
            gradient: (DVec2::new(33.5, 25.0), DVec2::new(3.0, 2.5)),
        },
    ]
}
fn selection_of(d: &Document, sc: &Scene) -> SelectionMask {
    let (cx, cy, rx, ry) = sc.ellipse;
    SelectionMask::ellipse(d, cx, cy, rx, ry).unwrap()
}
fn run(sc: &Scene, mode: u32, v: u32) -> Vec<u8> {
    let mut d = Document::with_tile_size(sc.w, sc.h, 8).unwrap();
    let l = d.add_layer("paint").unwrap();
    let material: Vec<_> = Channel::ALL
        .iter()
        .enumerate()
        .map(|(i, c)| ChannelPaint::new(*c, Rgba8::new(31 + i as u8 * 32, 79, 133, 211)))
        .collect();
    for (c, m) in material.iter().enumerate() {
        for y in 0..sc.h {
            for x in 0..sc.w {
                d.set_channel_pixel(
                    l,
                    m.channel,
                    x,
                    y,
                    Rgba8::new(
                        ((x * 17 + c as u32 * 31) % 256) as u8,
                        ((y * 23 + c as u32 * 11) % 256) as u8,
                        ((x * 7 + y * 13) % 256) as u8,
                        if (x + y) % 5 == 0 {
                            0
                        } else {
                            ((x + y) * 19 % 256) as u8
                        },
                    ),
                )
                .unwrap();
            }
        }
    }
    if v % 2 == 1 {
        d.set_selection(Some(selection_of(&d, sc))).unwrap();
    }
    let triangles = sc.triangles;
    let region = SelectionMask::from_triangles(&d, &triangles).unwrap();
    let erase = v >= 2;
    if mode == 0 {
        let mut b = Brush::from(BrushSettings {
            radius: 5.0,
            hardness: 0.4,
            spacing: 0.2,
            opacity: 0.85,
            flow: 0.45,
            erase,
            ..Default::default()
        });
        b.seed = 1234;
        if v % 2 == 1 {
            b.color.hue = 0.4;
            b.color.brightness = 0.2;
            b.jitter.size = 0.3;
            b.assist.curve = true;
            b.assist.taper_out = 4.0;
        }
        let mut s = d.begin_material_brush_stroke(l, &material, &b).unwrap();
        for (x, y, p, t) in sc.points {
            s.add_sample(&mut d, BrushSample::new(x, y, p, t, DVec2::ZERO).unwrap())
                .unwrap();
        }
        d.end_stroke(s).unwrap();
    } else if mode == 1 {
        d.fill_material(l, &material, 0.63, Some(&region), erase)
            .unwrap();
    } else if mode == 2 || mode == 3 {
        let g = GradientSettings {
            start: sc.gradient.0,
            end: sc.gradient.1,
            opacity: 0.73,
            shape: if v.is_multiple_of(2) {
                GradientShape::Linear
            } else {
                GradientShape::Radial
            },
            ..Default::default()
        };
        let ends: Vec<_> = material
            .iter()
            .enumerate()
            .rev()
            .map(|(c, m)| ChannelPaint::new(m.channel, Rgba8::new(213, 47, 13 + c as u8 * 27, 83)))
            .collect();
        d.gradient_material(
            l,
            &material,
            if mode == 3 { Some(&ends) } else { None },
            &g,
            Some(&region),
            erase,
        )
        .unwrap();
    } else if mode == 4 {
        let mut f = d
            .begin_material_triangle_fill(l, &material, 0.63, erase)
            .unwrap();
        f.add(&mut d, &[triangles[1]]).unwrap();
        f.add(&mut d, &[triangles[0], triangles[1]]).unwrap();
        f.add(&mut d, &[triangles[2]]).unwrap();
        f.commit(&mut d).unwrap();
    } else {
        // 画布ぴったりの四角を 2 回に分けて（端が欠けたタイルも画布の中が全部覆われた印になる）
        let (w, h) = (sc.w as f64, sc.h as f64);
        let mut f = d
            .begin_material_triangle_fill(l, &material, 0.63, erase)
            .unwrap();
        f.add(&mut d, &[tri([(0.0, 0.0), (w, 0.0), (w, h)])])
            .unwrap();
        f.add(&mut d, &[tri([(0.0, 0.0), (w, h), (0.0, h)])])
            .unwrap();
        f.commit(&mut d).unwrap();
    }
    material
        .iter()
        .flat_map(|m| {
            d.layer(l)
                .unwrap()
                .surface(m.channel)
                .unwrap()
                .to_canvas_bytes()
        })
        .collect()
}
/// マスクの塗り（MaterialGolden.cs の RunMask）: mode 0 は gradient_mask（範囲は三角形）、1 は begin_mask_triangle_fill。
/// v のビット 0 は文書の選択範囲、ビット 1 は見せる側。
fn run_mask(sc: &Scene, mode: u32, v: u32) -> Vec<u8> {
    let mut d = Document::with_tile_size(sc.w, sc.h, 8).unwrap();
    let l = d.add_layer("paint").unwrap();
    d.add_layer_mask(l).unwrap();
    for y in 0..sc.h {
        for x in 0..sc.w {
            let hide = if (x * 29 + y * 41) % 7 == 0 {
                0
            } else {
                ((x * 11 + y * 17 + 40) % 256) as u8
            };
            d.set_mask_pixel(l, x, y, hide).unwrap();
        }
    }
    if v % 2 == 1 {
        d.set_selection(Some(selection_of(&d, sc))).unwrap();
    }
    let reveal = v >= 2;
    let t = sc.triangles;
    if mode == 0 {
        let g = GradientSettings {
            start: sc.gradient.0,
            end: sc.gradient.1,
            from: Rgba8::new(31, 79, 133, 200),
            to: Rgba8::new(213, 47, 13, 40),
            opacity: 0.73,
            shape: if v.is_multiple_of(2) {
                GradientShape::Linear
            } else {
                GradientShape::Radial
            },
        };
        let region = SelectionMask::from_triangles(&d, &t).unwrap();
        d.gradient_mask(l, &g, Some(&region), reveal).unwrap();
    } else {
        let mut f = d.begin_mask_triangle_fill(l, 0.63, reveal).unwrap();
        f.add(&mut d, &[t[1]]).unwrap();
        f.add(&mut d, &[t[0], t[1]]).unwrap();
        f.add(&mut d, &[t[2]]).unwrap();
        f.commit(&mut d).unwrap();
    }
    d.layer(l)
        .unwrap()
        .mask()
        .unwrap()
        .surface()
        .to_canvas_bytes()
        .chunks_exact(4)
        .map(|p| p[3])
        .collect()
}
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
/// golden/material.txt の「鍵 → ハッシュ」。鍵は最後の語を除いた行頭の語を空白でつないだもの。同じ鍵が 2 回あれば落とす。
/// 出力の SHA-256 を正解と比べる（撮り直しの間は、違えば material.txt の行を書き直す）。
fn check(g: &HashMap<String, String>, key: &str, bytes: &[u8]) {
    let got = sha(bytes);
    if golden_update::updating() && got != g[key] {
        let path = golden_update::tests_dir().join("golden/material.txt");
        golden_update::replace_line(&path, key, &format!("{key} {got}"));
        return;
    }
    assert_eq!(got, g[key], "{key}");
}
fn golden() -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in include_str!("../golden/material.txt")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let (key, hash) = line.rsplit_once(' ').expect("鍵とハッシュ");
        assert!(
            map.insert(key.to_owned(), hash.to_owned()).is_none(),
            "{key}"
        );
    }
    map
}
/// 鍵の接頭辞ごとの件数を確かめる（行頭の判定や正解の欠け・書式変更で 0 件になっても通らないように）。
fn keys_with(golden: &HashMap<String, String>, prefix: &str) -> usize {
    golden
        .keys()
        .filter(|k| k.split(' ').next() == Some(prefix))
        .count()
}
/// 鍵と行の件数が、C# が書く事例の組（場面 × 並列度 × モード × 変種ほか）と過不足なく一致する。
#[test]
fn golden_covers_exactly_the_documented_cases() {
    let g = golden();
    for (prefix, n) in [
        ("mat", 48),
        ("edge", 48),
        ("mask-mat", 16),
        ("mask-edge", 16),
        ("id", 6),
        ("uv", 6),
        ("uvx", 10),
        ("tri", 11),
        ("rollback", 8),
        ("rollback-fill", 4),
        ("lock", 1024),
        ("lockmask", 512),
        ("locknr", 576),
    ] {
        assert_eq!(keys_with(&g, prefix), n, "{prefix}");
    }

    let mut expected = Vec::new();
    for sc in scenes() {
        for degree in [1, 4] {
            for mode in 0..6 {
                for v in 0..4 {
                    expected.push(format!("{} {degree} {mode} {v}", sc.name));
                }
            }
            for mode in 0..2 {
                for v in 0..4 {
                    expected.push(format!("mask-{} {degree} {mode} {v}", sc.name));
                }
            }
        }
    }
    for (prefix, n) in [("id", 6), ("uv", 6), ("uvx", 10), ("tri", 11)] {
        expected.extend((0..n).map(|v| format!("{prefix} {v}")));
    }
    for tile in [8, 128] {
        for existing in [0, 1] {
            for channels in [1, 6] {
                expected.push(format!("rollback {tile} {existing} {channels}"));
            }
        }
    }
    for selected in [0, 1] {
        for channels in [1, 6] {
            expected.push(format!("rollback-fill {selected} {channels}"));
        }
    }
    for degree in [1, 4] {
        for (prefix, kinds) in [("lock", 8), ("lockmask", 4)] {
            for kind in 0..kinds {
                for lock_case in 0..LOCK_CASES {
                    for v in 0..8 {
                        expected.push(format!("{prefix} {degree} {kind} {lock_case} {v}"));
                    }
                }
            }
        }
    }
    for entry in 0..12 {
        for target in 0..3 {
            for lock_case in 0..LOCK_CASES {
                for v in 0..2 {
                    expected.push(format!("locknr {entry} {target} {lock_case} {v}"));
                }
            }
        }
    }
    for key in &expected {
        assert!(g.contains_key(key), "正解に無い事例: {key}");
    }
    assert_eq!(expected.len(), g.len());
}
/// ロックの場面の数（MaterialGolden.cs の LockDoc の lockCase）: 0 なし・1 透明部分・2 画像・3 すべて・4 位置（塗りは通る）、
/// 5〜7 は層を 1 つだけ含むグループに 透明部分・画像・すべて。
const LOCK_CASES: u32 = 8;
const LOCK_BITS: [LayerLocks; 8] = [
    LayerLocks::NONE,
    LayerLocks::TRANSPARENCY,
    LayerLocks::PIXELS,
    LayerLocks::ALL,
    LayerLocks::POSITION,
    LayerLocks::TRANSPARENCY,
    LayerLocks::PIXELS,
    LayerLocks::ALL,
];
/// ロックの場面（edge の画布）。層は 6 チャンネルとも画素あり。透明な画素はチャンネルごとに違う（アルファの式にチャンネル番号が入る）。
/// off は 2 つのチャンネルを無効にしておく。selected は文書の選択範囲（楕円）を立てる。
fn lock_doc(
    lock_case: u32,
    off: bool,
    with_mask: bool,
    selected: bool,
) -> (Document, LayerId, Option<LayerId>, Scene) {
    let sc = scenes().into_iter().nth(1).unwrap();
    let mut d = Document::with_tile_size(sc.w, sc.h, 8).unwrap();
    let l = d.add_layer("paint").unwrap();
    for (c, ch) in Channel::ALL.iter().enumerate() {
        for y in 0..sc.h {
            for x in 0..sc.w {
                let a = if (x + y + c as u32).is_multiple_of(5) {
                    0
                } else {
                    (((x + y) * 19 + c as u32 * 23) % 256) as u8
                };
                let p = Rgba8::new(
                    ((x * 17 + c as u32 * 31) % 256) as u8,
                    ((y * 23 + c as u32 * 11) % 256) as u8,
                    ((x * 7 + y * 13) % 256) as u8,
                    a,
                );
                d.set_channel_pixel(l, *ch, x, y, p).unwrap();
            }
        }
    }
    if with_mask {
        d.add_layer_mask(l).unwrap();
        for y in 0..sc.h {
            for x in 0..sc.w {
                let hide = if (x * 29 + y * 41) % 7 == 0 {
                    0
                } else {
                    ((x * 11 + y * 17 + 40) % 256) as u8
                };
                d.set_mask_pixel(l, x, y, hide).unwrap();
            }
        }
    }
    if selected {
        d.set_selection(Some(selection_of(&d, &sc))).unwrap();
    }
    if off {
        d.set_channel_enabled(l, Channel::ALL[2], false).unwrap();
        d.set_channel_enabled(l, Channel::ALL[4], false).unwrap();
    }
    let group = (lock_case >= 5).then(|| d.group_layers(&[l], "g").unwrap());
    d.set_layer_locks(group.unwrap_or(l), LOCK_BITS[lock_case as usize])
        .unwrap();
    d.clear_history().unwrap();
    (d, l, group, sc)
}
/// 層の全チャンネル（有効か・面があるか・画素）か、マスクの隠す量だけ（MaterialGolden.cs の LockState）。
fn lock_state(out: &mut Vec<u8>, d: &Document, l: LayerId, mask_only: bool) {
    let layer = d.layer(l).unwrap();
    if mask_only {
        out.push(u8::from(layer.mask().is_some()));
        if let Some(m) = layer.mask() {
            out.extend(m.surface().to_canvas_bytes().chunks_exact(4).map(|p| p[3]));
        }
        return;
    }
    for c in Channel::ALL {
        out.push(u8::from(layer.is_channel_enabled(c)));
        let surface = layer.surface(c);
        out.push(u8::from(surface.is_some()));
        if let Some(s) = surface {
            out.extend(s.to_canvas_bytes());
        }
    }
}
/// 書いた結果の先頭: 0 通った・1 ロックで断った（層・持ち主の並びの位置とロック）・2 そのほかの拒否。
fn lock_outcome(out: &mut Vec<u8>, d: &Document, result: Result<(), CoreError>) -> bool {
    match result {
        Ok(()) => {
            out.push(0);
            true
        }
        Err(CoreError::LayerLocked {
            layer,
            holder,
            lock,
        }) => {
            let at = |id: LayerId| d.layers().iter().position(|l| l.id() == id).unwrap() as i32;
            out.push(1);
            out.extend(at(layer).to_le_bytes());
            out.extend(at(holder).to_le_bytes());
            out.extend(i32::from(lock.bits()).to_le_bytes());
            false
        }
        Err(_) => {
            out.push(2);
            false
        }
    }
}
/// 断ったか通ったかのあとの文書（Undo/Redo の可否と、通って履歴ができたときは Undo の後と Redo の後も）。
fn lock_after(out: &mut Vec<u8>, d: &mut Document, l: LayerId, ok: bool, mask_only: bool) {
    let flags = |out: &mut Vec<u8>, d: &Document| {
        out.push(u8::from(d.can_undo()));
        out.push(u8::from(d.can_redo()));
        lock_state(out, d, l, mask_only);
    };
    flags(out, d);
    if ok && d.can_undo() {
        d.undo().unwrap();
        flags(out, d);
        d.redo().unwrap();
        flags(out, d);
    }
}
/// 層へ書く入口（ピクセルのチャンネルへ）。MaterialGolden.cs の RunLock・RunLockKind が呼ぶ書き込みと同じ。
#[derive(Clone, Copy)]
enum PixelWrite {
    Stroke,
    MaterialStroke,
    FillMaterial,
    GradientMaterial,
    GradientMaterialEnds,
    MaterialTriangles,
    Fill,
    Gradient,
    Triangles,
}
/// 層のマスクへ書く入口。
#[derive(Clone, Copy)]
enum MaskWrite {
    Gradient,
    Triangles,
    Fill,
    Stroke,
}
fn lock_material() -> Vec<ChannelPaint> {
    Channel::ALL
        .iter()
        .enumerate()
        .map(|(i, c)| ChannelPaint::new(*c, Rgba8::new(31 + i as u8 * 32, 79, 133, 211)))
        .collect()
}
fn lock_gradient(sc: &Scene) -> GradientSettings {
    GradientSettings {
        start: sc.gradient.0,
        end: sc.gradient.1,
        from: Rgba8::new(31, 79, 133, 200),
        to: Rgba8::new(213, 47, 13, 40),
        opacity: 0.73,
        shape: GradientShape::Linear,
    }
}
fn lock_brush(erase: bool) -> Brush {
    let mut b = Brush::from(BrushSettings {
        color: Rgba8::new(213, 47, 13, 211),
        radius: 5.0,
        hardness: 0.4,
        spacing: 0.2,
        opacity: 0.85,
        flow: 0.45,
        erase,
        ..Default::default()
    });
    b.seed = 1234;
    b
}
/// channel は単チャンネルの入口（Stroke・Fill・Gradient・Triangles）が書くチャンネル。erase は消す書き込みか。
fn write_pixels(
    d: &mut Document,
    l: LayerId,
    sc: &Scene,
    write: PixelWrite,
    channel: Channel,
    erase: bool,
) -> Result<(), CoreError> {
    let material = lock_material();
    let t = sc.triangles;
    let region = SelectionMask::from_triangles(d, &t).unwrap();
    let g = lock_gradient(sc);
    let ends: Vec<_> = material
        .iter()
        .enumerate()
        .rev()
        .map(|(c, m)| ChannelPaint::new(m.channel, Rgba8::new(213, 47, 13 + c as u8 * 27, 83)))
        .collect();
    match write {
        PixelWrite::Stroke => {
            let mut s = d.begin_brush_stroke_in(l, channel, &lock_brush(erase))?;
            for (x, y, p, t) in sc.points {
                s.add_sample(d, BrushSample::new(x, y, p, t, DVec2::ZERO).unwrap())?;
            }
            d.end_stroke(s)?;
        }
        PixelWrite::MaterialStroke => {
            let mut s = d.begin_material_brush_stroke(l, &material, &lock_brush(erase))?;
            for (x, y, p, t) in sc.points {
                s.add_sample(d, BrushSample::new(x, y, p, t, DVec2::ZERO).unwrap())?;
            }
            d.end_stroke(s)?;
        }
        PixelWrite::FillMaterial => {
            d.fill_material(l, &material, 0.63, Some(&region), erase)?;
        }
        PixelWrite::GradientMaterial => {
            d.gradient_material(l, &material, None, &g, Some(&region), erase)?;
        }
        PixelWrite::GradientMaterialEnds => {
            d.gradient_material(l, &material, Some(&ends), &g, Some(&region), erase)?;
        }
        PixelWrite::MaterialTriangles => {
            let mut f = d.begin_material_triangle_fill(l, &material, 0.63, erase)?;
            f.add(d, &[t[1]])?;
            f.add(d, &[t[0], t[1]])?;
            f.add(d, &[t[2]])?;
            f.commit(d)?;
        }
        PixelWrite::Fill => {
            d.fill(
                l,
                channel,
                Rgba8::new(213, 47, 13, 211),
                0.63,
                Some(&region),
                erase,
            )?;
        }
        PixelWrite::Gradient => {
            d.gradient(l, channel, &g, Some(&region), erase)?;
        }
        PixelWrite::Triangles => {
            let mut f =
                d.begin_triangle_fill(l, channel, Rgba8::new(213, 47, 13, 211), 0.63, erase)?;
            f.add(d, &[t[1]])?;
            f.add(d, &[t[0], t[1]])?;
            f.add(d, &[t[2]])?;
            f.commit(d)?;
        }
    }
    Ok(())
}
fn write_mask(
    d: &mut Document,
    l: LayerId,
    sc: &Scene,
    write: MaskWrite,
    reveal: bool,
) -> Result<(), CoreError> {
    let t = sc.triangles;
    let region = SelectionMask::from_triangles(d, &t).unwrap();
    match write {
        MaskWrite::Gradient => {
            d.gradient_mask(l, &lock_gradient(sc), Some(&region), reveal)?;
        }
        MaskWrite::Triangles => {
            let mut f = d.begin_mask_triangle_fill(l, 0.63, reveal)?;
            f.add(d, &[t[1]])?;
            f.add(d, &[t[0], t[1]])?;
            f.add(d, &[t[2]])?;
            f.commit(d)?;
        }
        MaskWrite::Fill => {
            d.fill_mask(l, 0.63, Some(&region), reveal)?;
        }
        MaskWrite::Stroke => {
            let mut s = d.begin_brush_mask_stroke(l, &lock_brush(reveal))?;
            for (x, y, p, t) in sc.points {
                s.add_sample(d, BrushSample::new(x, y, p, t, DVec2::ZERO).unwrap())?;
            }
            d.end_stroke(s)?;
        }
    }
    Ok(())
}
/// ロックを立てた層への書き込み（MaterialGolden.cs の RunLock）: kind 0 マテリアルのストローク・1 範囲の塗り・2 グラデーション・
/// 3 二つのマテリアルのグラデーション・4 マテリアルの三角形の塗り・5 単チャンネルのグラデーション・6 単チャンネルの三角形の塗り・
/// 7 単チャンネルのストローク。v のビット 0 は消す・ビット 1 は 2 つのチャンネルを無効にしておく（単チャンネルの入口はその無効のチャンネルへ書く）・
/// ビット 2 は文書の選択範囲。
fn run_lock(kind: u32, lock_case: u32, v: u32) -> Vec<u8> {
    const KINDS: [PixelWrite; 8] = [
        PixelWrite::MaterialStroke,
        PixelWrite::FillMaterial,
        PixelWrite::GradientMaterial,
        PixelWrite::GradientMaterialEnds,
        PixelWrite::MaterialTriangles,
        PixelWrite::Gradient,
        PixelWrite::Triangles,
        PixelWrite::Stroke,
    ];
    let (erase, off) = (v & 1 != 0, v & 2 != 0);
    let (mut d, l, _, sc) = lock_doc(lock_case, off, false, v & 4 != 0);
    let channel = Channel::ALL[if off { 2 } else { 0 }];
    let result = write_pixels(&mut d, l, &sc, KINDS[kind as usize], channel, erase);
    let mut out = Vec::new();
    let ok = lock_outcome(&mut out, &d, result);
    lock_after(&mut out, &mut d, l, ok, false);
    out
}
/// ロックを立てた層のマスクへの書き込み（MaterialGolden.cs の RunLockMask）: kind 0 グラデーション・1 三角形の塗り・2 範囲の塗り・
/// 3 ブラシのストローク。v のビット 0 は見せる側・ビット 1 は層にマスクが無い・ビット 2 は文書の選択範囲。
fn run_lock_mask(kind: u32, lock_case: u32, v: u32) -> Vec<u8> {
    const KINDS: [MaskWrite; 4] = [
        MaskWrite::Gradient,
        MaskWrite::Triangles,
        MaskWrite::Fill,
        MaskWrite::Stroke,
    ];
    let (mut d, l, _, sc) = lock_doc(lock_case, false, v & 2 == 0, v & 4 != 0);
    let result = write_mask(&mut d, l, &sc, KINDS[kind as usize], v & 1 != 0);
    let mut out = Vec::new();
    let ok = lock_outcome(&mut out, &d, result);
    lock_after(&mut out, &mut d, l, ok, true);
    out
}
/// 塗りつぶし・調整・グループの層の場面（MaterialGolden.cs の LockKindDoc）。層に全体のマスクを付け、ロックの置き方は `lock_doc` と同じ。
/// target: 0 塗りつぶし（Color と Emission の値）・1 調整（反転）・2 グループ。
fn lock_kind_doc(target: u32, lock_case: u32) -> (Document, LayerId, Scene) {
    let sc = scenes().into_iter().nth(1).unwrap();
    let mut d = Document::with_tile_size(sc.w, sc.h, 8).unwrap();
    let l = match target {
        0 => d.add_fill_layer(
            "n",
            &[
                (Channel::Color, Rgba8::new(10, 20, 30, 255)),
                (Channel::Emission, Rgba8::new(40, 50, 60, 255)),
            ],
            None,
        ),
        1 => d.add_adjustment_layer("n", AdjustmentSettings::invert(), None, None),
        _ => d.add_group("n", None),
    }
    .unwrap();
    d.add_layer_mask(l).unwrap();
    for y in 0..sc.h {
        for x in 0..sc.w {
            let hide = if (x * 29 + y * 41) % 7 == 0 {
                0
            } else {
                ((x * 11 + y * 17 + 40) % 256) as u8
            };
            d.set_mask_pixel(l, x, y, hide).unwrap();
        }
    }
    let group = (lock_case >= 5).then(|| d.group_layers(&[l], "g").unwrap());
    d.set_layer_locks(group.unwrap_or(l), LOCK_BITS[lock_case as usize])
        .unwrap();
    d.clear_history().unwrap();
    (d, l, sc)
}
/// 各チャンネルの有効とマスクの隠す量（MaterialGolden.cs の LockKindState）。
fn lock_kind_state(out: &mut Vec<u8>, d: &Document, l: LayerId) {
    let layer = d.layer(l).unwrap();
    for c in Channel::ALL {
        out.push(u8::from(layer.is_channel_enabled(c)));
    }
    out.extend(
        layer
            .mask()
            .unwrap()
            .surface()
            .to_canvas_bytes()
            .chunks_exact(4)
            .map(|p| p[3]),
    );
}
/// 塗りつぶし・調整・グループの層への書き込みの、型の拒否とロックの拒否の順（MaterialGolden.cs の RunLockKind）: entry 0 begin_stroke・
/// 1 begin_material_stroke・2 fill_material・3 gradient_material・4 begin_material_triangle_fill・5 fill・6 gradient・7 begin_triangle_fill
/// （5〜7 は Color）・8 gradient_mask・9 begin_mask_triangle_fill・10 fill_mask・11 begin_mask_stroke。v のビット 0 は消す（マスクでは見せる）。
fn run_lock_kind(entry: u32, target: u32, lock_case: u32, v: u32) -> Vec<u8> {
    const PIXELS: [PixelWrite; 8] = [
        PixelWrite::Stroke,
        PixelWrite::MaterialStroke,
        PixelWrite::FillMaterial,
        PixelWrite::GradientMaterial,
        PixelWrite::MaterialTriangles,
        PixelWrite::Fill,
        PixelWrite::Gradient,
        PixelWrite::Triangles,
    ];
    const MASKS: [MaskWrite; 4] = [
        MaskWrite::Gradient,
        MaskWrite::Triangles,
        MaskWrite::Fill,
        MaskWrite::Stroke,
    ];
    let (mut d, l, sc) = lock_kind_doc(target, lock_case);
    let flag = v & 1 != 0;
    let result = match entry {
        0..=7 => write_pixels(&mut d, l, &sc, PIXELS[entry as usize], Channel::Color, flag),
        _ => write_mask(&mut d, l, &sc, MASKS[entry as usize - 8], flag),
    };
    let mut out = Vec::new();
    let ok = lock_outcome(&mut out, &d, result);
    out.push(u8::from(d.can_undo()));
    out.push(u8::from(d.can_redo()));
    lock_kind_state(&mut out, &d, l);
    if ok && d.can_undo() {
        for step in 0..2 {
            if step == 0 {
                d.undo().unwrap();
            } else {
                d.redo().unwrap();
            }
            out.push(u8::from(d.can_undo()));
            out.push(u8::from(d.can_redo()));
            lock_kind_state(&mut out, &d, l);
        }
    }
    out
}
fn pool(degree: usize) -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new()
        .num_threads(degree)
        .build()
        .unwrap()
}
#[test]
fn csharp_all_bytes_at_both_parallel_degrees() {
    let g = golden();
    let mut checked = 0;
    for sc in scenes() {
        for degree in [1, 4] {
            for mode in 0..6 {
                for v in 0..4 {
                    let bytes = pool(degree).install(|| run(&sc, mode, v));
                    let key = format!("{} {degree} {mode} {v}", sc.name);
                    check(&g, &key, &bytes);
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 96);
}
#[test]
fn csharp_mask_all_bytes_at_both_parallel_degrees() {
    let g = golden();
    let mut checked = 0;
    for sc in scenes() {
        for degree in [1, 4] {
            for mode in 0..2 {
                for v in 0..4 {
                    let bytes = pool(degree).install(|| run_mask(&sc, mode, v));
                    let key = format!("mask-{} {degree} {mode} {v}", sc.name);
                    check(&g, &key, &bytes);
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 32);
}
fn id_map() -> mesh_maps::BakedMeshMap {
    use mesh_maps::*;
    BakedMeshMap::new(
        MeshMapProvenance {
            kind: MeshMapKind::Id,
            engine_version: 1,
            mesh_hash: "synthetic".into(),
            topology_hash: "synthetic".into(),
            uv_channel: 0,
            width: 32,
            height: 24,
            target_slot: -1,
            target_slots: vec![],
            padding: 0,
            antialiasing: 1,
            settings_key: "test".into(),
            space: "SnapshotWorld".into(),
            pose: "StaticSnapshot".into(),
            source: "Self".into(),
            bounds_min: [-1.0, -2.0, -3.0],
            bounds_max: [2.0, 3.0, 1.0],
        },
        (0..32 * 24)
            .flat_map(|i| (0..3).map(move |c| ((i * 1193 + c * 13451) % 65536) as u16))
            .collect(),
        (0..32 * 24)
            .map(|i| if i % 7 == 0 { 0 } else { 1 + (i % 2) as u8 })
            .collect(),
    )
    .unwrap()
}
fn uv_bytes(
    before: &[uv_layout::UvTriangle],
    after: &[uv_layout::UvTriangle],
    res: u32,
) -> Vec<u8> {
    match uv_layout::compare(before, after, res) {
        Ok(r) => {
            let mut bytes = vec![u8::from(r.same)];
            bytes.extend(r.kept.to_le_bytes());
            bytes.extend(r.added.to_le_bytes());
            bytes
        }
        Err(_) => vec![255],
    }
}
#[test]
fn csharp_id_and_uv_all_bytes() {
    let g = golden();
    let mut checked = 0;
    for v in 0..6usize {
        let d = Document::with_tile_size(32, 24, 8).unwrap();
        let m = SelectionMask::from_id_colors(
            &d,
            &id_map(),
            if v == 0 {
                &[]
            } else {
                &[0, 0x0800f7, 0x325678]
            },
            [0, 0, 7, 8, 63, 255][v],
        )
        .unwrap();
        let bytes: Vec<u8> = (0..24)
            .flat_map(|y| (0..32).map(move |x| (x, y)))
            .map(|(x, y)| m.amount(x, y))
            .collect();
        assert_eq!(sha(&bytes), g[&format!("id {v}")], "id {v}");
        let a = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let b = [[0.1, 0.2], [0.8, 0.1], [0.6, 0.7]];
        let after = match v {
            0 => vec![b, a],
            1 => vec![[a[1], a[2], a[0]], b],
            2 => vec![[a[0], a[2], a[1]], b],
            3 => vec![a],
            4 => vec![],
            _ => vec![[[0.2, 0.2], [0.9, 0.2], [0.2, 0.9]]],
        };
        assert_eq!(
            sha(&uv_bytes(&[a, b], &after, 32)),
            g[&format!("uv {v}")],
            "uv {v}"
        );
        checked += 2;
    }
    assert_eq!(checked, 12);
}
/// 解像度の境界（0 と 4097 は断る・1 と 4096 は通る）・NaN と無限大の三角形・はみ出す UV・斜めの辺を共有する向かいの半分。
/// 1 バイトの 255 は解像度の拒否（MaterialGolden.cs の UvExtra と同じ入力）。
#[test]
fn csharp_uv_boundaries_all_bytes() {
    let g = golden();
    let a = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
    let b = [[0.1, 0.2], [0.8, 0.1], [0.6, 0.7]];
    let nan = [[f64::NAN, 0.0], [1.0, 0.0], [0.0, 1.0]];
    let inf = [[0.0, f64::INFINITY], [1.0, 0.0], [0.0, 1.0]];
    let cases: [(Vec<_>, Vec<_>, u32); 10] = [
        (vec![a, b], vec![a, b], 0),
        (vec![a, b], vec![a, b], 4097),
        (vec![a, b], vec![b], 1),
        (vec![a, b], vec![b], 4096),
        (vec![nan], vec![nan], 16),
        (vec![nan, b], vec![b, inf], 16),
        (vec![a], vec![[[-0.5, -0.5], [1.5, -0.5], [0.25, 1.5]]], 16),
        (vec![nan, inf], vec![], 16),
        (vec![a], vec![a, a], 16),
        (vec![a], vec![[[1.0, 1.0], [0.0, 1.0], [1.0, 0.0]]], 4),
    ];
    for (v, (before, after, res)) in cases.iter().enumerate() {
        assert_eq!(
            sha(&uv_bytes(before, after, *res)),
            g[&format!("uvx {v}")],
            "uvx {v}"
        );
    }
}
/// 三角形の和集合の選択範囲（MaterialGolden.cs の TriangleCases）。端が欠けたタイルの画布（37×29・タイル 8）の上で、
/// 画布の外・一部が外・一直線・極小・巨大・継ぎ目を C# と全バイトで照らす。
#[test]
fn csharp_triangle_union_all_bytes() {
    let g = golden();
    for (v, ts) in triangle_cases().iter().enumerate() {
        let d = Document::with_tile_size(37, 29, 8).unwrap();
        let m = SelectionMask::from_triangles(&d, ts).unwrap();
        let bytes: Vec<u8> = (0..29)
            .flat_map(|y| (0..37).map(move |x| (x, y)))
            .map(|(x, y)| m.amount(x, y))
            .collect();
        assert_eq!(sha(&bytes), g[&format!("tri {v}")], "tri {v}");
    }
}
fn triangle_cases() -> Vec<Vec<PixelTriangle>> {
    vec![
        vec![tri([(50.0, 5.0), (60.0, 5.0), (55.0, 20.0)])],
        vec![tri([(5.0, -20.0), (15.0, -20.0), (10.0, -3.0)])],
        vec![tri([(-6.2, 10.0), (14.1, -4.4), (20.3, 31.8)])],
        vec![tri([(1.0, 1.0), (10.0, 10.0), (20.0, 20.0)])],
        vec![tri([(1.0, 1.0), (1.000001, 1.0), (1.0, 1.0000001)])],
        vec![tri([(1.375, 1.375), (1.375001, 1.375), (1.375, 1.375004)])],
        vec![tri([(-1e6, -1e6), (1e6, -1e6), (0.0, 1e6)])],
        vec![tri([(-1e9, -1e9), (1e9, -1e9), (0.0, 1e9)])],
        vec![
            tri([(0.0, 0.0), (37.0, 0.0), (37.0, 29.0)]),
            tri([(0.0, 0.0), (37.0, 29.0), (0.0, 29.0)]),
        ],
        vec![tri([(0.0, 0.0), (37.0, 0.0), (0.0, 29.0)])],
        vec![tri([(8.0, 8.0), (16.0, 8.0), (8.0, 16.0)])],
    ]
}
/// 種類だけを変えた（大きさ・チャンネル数は種類どおりの）焼いたマップ。
fn map_of(kind: mesh_maps::MeshMapKind) -> mesh_maps::BakedMeshMap {
    use mesh_maps::*;
    let texels = 32 * 24;
    BakedMeshMap::new(
        MeshMapProvenance {
            kind,
            engine_version: 1,
            mesh_hash: "synthetic".into(),
            topology_hash: "synthetic".into(),
            uv_channel: 0,
            width: 32,
            height: 24,
            target_slot: -1,
            target_slots: vec![],
            padding: 0,
            antialiasing: 1,
            settings_key: "test".into(),
            space: "SnapshotWorld".into(),
            pose: "StaticSnapshot".into(),
            source: "Self".into(),
            bounds_min: [-1.0, -2.0, -3.0],
            bounds_max: [2.0, 3.0, 1.0],
        },
        vec![40000; texels * kind.channels()],
        vec![1; texels],
    )
    .unwrap()
}
#[test]
fn id_map_boundaries_and_refusals() {
    let map = id_map();
    assert_eq!(id_colors::try_get(&map, -1, 0).unwrap(), None);
    assert_eq!(id_colors::try_get(&map, 0, 0).unwrap(), None);
    assert_eq!(id_colors::try_get(&map, 32, 1).unwrap(), None);
    assert_eq!(id_colors::try_get(&map, 1, 24).unwrap(), None);
    assert_eq!(id_colors::try_get(&map, i64::MIN, i64::MAX).unwrap(), None);
    assert_eq!(id_colors::try_get_at_uv(&map, f64::NAN, 0.5).unwrap(), None);
    assert_eq!(id_colors::try_get_at_uv(&map, 0.5, f64::NAN).unwrap(), None);
    assert_eq!(id_colors::try_get_at_uv(&map, -0.1, 0.5).unwrap(), None);
    assert_eq!(id_colors::try_get_at_uv(&map, 0.5, 1.1).unwrap(), None);
    assert_eq!(
        id_colors::try_get_at_uv(&map, 1.0, 0.0).unwrap(),
        id_colors::try_get(&map, 31, 0).unwrap()
    );
    assert!(id_colors::near(0x0000ff, 0x0800f7, 8));
    assert!(!id_colors::near(0x0000ff, 0x0800f7, 7));
    assert!(SelectionMask::from_id_colors(&Document::new(1, 1).unwrap(), &map, &[], 0).is_err());
    assert!(
        SelectionMask::from_id_colors(&Document::new(32, 24).unwrap(), &map, &[0x1000000], 0)
            .is_err()
    );
}
/// ID マップ以外（ほかの 9 種類）は、色の取得・UV の点の取得・色による選択のどれでも断る。大きさが文書に合っていても、
/// 色が正しくても断る（種類の確認が最初）。
#[test]
fn id_colors_refuse_every_map_that_is_not_the_id_map() {
    let doc = Document::with_tile_size(32, 24, 8).unwrap();
    let mut refused = 0;
    for kind in mesh_maps::MeshMapKind::ALL {
        let map = map_of(kind);
        if kind == mesh_maps::MeshMapKind::Id {
            assert!(id_colors::try_get(&map, 3, 3).is_ok());
            assert!(id_colors::try_get_at_uv(&map, 0.5, 0.5).is_ok());
            assert!(SelectionMask::from_id_colors(&doc, &map, &[0x010203], 8).is_ok());
            continue;
        }
        assert!(id_colors::try_get(&map, 3, 3).is_err(), "{kind:?}");
        assert!(
            id_colors::try_get(&map, -1, -1).is_err(),
            "範囲の外でも種類が先: {kind:?}"
        );
        assert!(
            id_colors::try_get_at_uv(&map, 0.5, 0.5).is_err(),
            "{kind:?}"
        );
        assert!(
            id_colors::try_get_at_uv(&map, f64::NAN, 0.5).is_err(),
            "{kind:?}"
        );
        assert!(
            SelectionMask::from_id_colors(&doc, &map, &[0x010203], 8).is_err(),
            "{kind:?}"
        );
        assert!(
            SelectionMask::from_id_colors(&doc, &map, &[], 0).is_err(),
            "{kind:?}"
        );
        refused += 1;
    }
    assert_eq!(refused, mesh_maps::MeshMapKind::ALL.len() - 1);
}
/// 1 つの点で 1 回塗ったときのストロークの巻き戻しのバイト数と、手を付けたタイルの数（チャンネルの数 × 実際のタイル）。
/// MaterialGolden.cs の Rollback と同じ入力（画布は 6×5 タイル、existing は全チャンネルに画素のある層）。
fn rollback_of(tile: u32, existing: bool, channels: usize) -> (u64, usize) {
    let mut d = Document::with_tile_size(tile * 6, tile * 5, tile).unwrap();
    let l = d.add_layer("paint").unwrap();
    if existing {
        for c in Channel::ALL {
            for y in 0..tile * 5 {
                for x in 0..tile * 6 {
                    let p = Rgba8::new((x * 7) as u8, (y * 5) as u8, (x + y) as u8, 200);
                    d.set_channel_pixel(l, c, x, y, p).unwrap();
                }
            }
        }
    }
    let material: Vec<_> = Channel::ALL
        .iter()
        .map(|c| ChannelPaint::new(*c, Rgba8::new(31, 79, 133, 211)))
        .take(channels)
        .collect();
    let brush = BrushSettings {
        radius: tile as f64 * 0.75,
        ..Default::default()
    };
    let mut s = d.begin_material_stroke(l, &material, &brush).unwrap();
    let c = tile as f64 * 2.5;
    s.add_point(&mut d, c, c, 1.0, DVec2::ZERO).unwrap();
    let stats = d.active_stroke_stats().unwrap();
    d.cancel_stroke(s);
    (stats.rollback_bytes, stats.tiles)
}
/// 巻き戻しの確保量の数え方は C# と違う: C# はストロークの覆い（画素ごとの被覆率）を全チャンネルで 1 枚共有するが、Rust はチャンネルごとの
/// 状態が覆いを持つ。1 チャンネルの量は C# と同じで、n チャンネルは丁度 n 倍（元の写しの有無に依らず）。C# は元が空のタイルでは
/// チャンネルを増やしても増えず、元が埋まったタイルでは写しの分だけ増える。そのため同じ予算で通るタイルの数が減る（早く断る側）。
/// この試験は C# が書いた数（golden の rollback の行）と Rust の数を照らして、その倍率と、既定の予算（64 MiB・タイル 128²）で
/// 通るタイルの数を固定する。
#[test]
fn rollback_accounting_against_csharp_and_the_tiles_a_default_budget_admits() {
    let g = golden();
    let csharp = |tile: u32, existing: u32, channels: u32| -> u64 {
        g[&format!("rollback {tile} {existing} {channels}")]
            .parse()
            .unwrap()
    };
    for tile in [8, 128] {
        for existing in [0, 1] {
            let (one, tiles1) = rollback_of(tile, existing == 1, 1);
            let (six, tiles6) = rollback_of(tile, existing == 1, 6);
            // 同じ入力なら、全チャンネルが同じタイルに手を付ける
            assert_eq!(tiles6, tiles1 * 6, "tile={tile} existing={existing}");
            assert_eq!(one, csharp(tile, existing, 1), "1 チャンネルは C# と同じ");
            assert_eq!(six, 6 * one, "チャンネルごとの覆いなので丁度 6 倍");
            let cs6 = csharp(tile, existing, 6);
            if existing == 0 {
                assert_eq!(cs6, csharp(tile, existing, 1), "C# は元が空なら増えない");
                assert_eq!(six, 6 * cs6);
            } else {
                // 元が埋まったタイルでは、C# も写しの分は増える（タイル 8 で 1.86 倍、128 で 1.71 倍まで Rust が多い）
                assert!(six > cs6 && six < 2 * cs6, "{six} {cs6}");
            }
        }
    }
    // 既定の予算（64 MiB）とタイル 128² で、1 回のストロークが触れて通るタイルの数（空の層 / 全チャンネルに画素のある層）
    let budget = Document::new(1, 1).unwrap().stroke_budget_bytes();
    assert_eq!(budget, 64 << 20);
    for (existing, rust_tiles, csharp_tiles) in [(false, 170, 1023), (true, 85, 146)] {
        let (six, n) = rollback_of(128, existing, 6);
        let touched = (n / 6) as u64;
        assert_eq!(touched, 9);
        assert_eq!(budget / (six / touched), rust_tiles, "existing={existing}");
        let cs = csharp(128, existing as u32, 6);
        assert_eq!(budget / (cs / touched), csharp_tiles, "existing={existing}");
    }
}
/// 累積の三角形の塗りの巻き戻しのバイト数は C# と同じ（写しは対象ごと、サンプルの記録は全対象で 1 つ）。選択範囲の外のタイルも、
/// 写しを取って数える（C# と同じ。選択範囲を付けても量が変わらない）。
#[test]
fn triangle_fill_rollback_bytes_equal_csharp() {
    let g = golden();
    let sc = &scenes()[1];
    for selected in [false, true] {
        for channels in [1, 6] {
            let mut d = Document::with_tile_size(sc.w, sc.h, 8).unwrap();
            let l = d.add_layer("paint").unwrap();
            for (c, ch) in Channel::ALL.iter().enumerate() {
                for y in 0..sc.h {
                    for x in 0..sc.w {
                        let a = if (x + y) % 5 == 0 {
                            0
                        } else {
                            ((x + y) * 19 % 256) as u8
                        };
                        let p = Rgba8::new(
                            ((x * 17 + c as u32 * 31) % 256) as u8,
                            ((y * 23 + c as u32 * 11) % 256) as u8,
                            ((x * 7 + y * 13) % 256) as u8,
                            a,
                        );
                        d.set_channel_pixel(l, *ch, x, y, p).unwrap();
                    }
                }
            }
            if selected {
                d.set_selection(Some(selection_of(&d, sc))).unwrap();
            }
            let material: Vec<_> = Channel::ALL
                .iter()
                .map(|c| ChannelPaint::new(*c, Rgba8::new(31, 79, 133, 211)))
                .take(channels)
                .collect();
            let mut f = d
                .begin_material_triangle_fill(l, &material, 0.63, false)
                .unwrap();
            f.add(&mut d, &sc.triangles).unwrap();
            let bytes = d.active_stroke_stats().unwrap().rollback_bytes;
            f.cancel(&mut d);
            let key = format!("rollback-fill {} {channels}", u8::from(selected));
            assert_eq!(bytes.to_string(), g[&key], "{key}");
        }
    }
}
/// ロックを立てた層への書き込み（マテリアルのストローク・範囲の塗り・グラデーション 2 種・三角形の塗り・単チャンネルのグラデーション・
/// 三角形の塗り・ストローク）が、C# と全バイト一致する: 断った層・持ち主・ロック、断ったあとの文書（Undo/Redo の可否と全チャンネルの有効・画素）、
/// 通ったときの画素と Undo/Redo。透明部分のロックでは全チャンネルがアルファと透明画素の RGB を守る。透明な画素はチャンネルごとに違い、
/// 文書の選択範囲を立てた事例と、無効にしたチャンネルへ書く単チャンネルの事例を含む。
#[test]
fn csharp_locked_layer_writes_all_bytes_at_both_parallel_degrees() {
    let g = golden();
    let mut checked = 0;
    for degree in [1, 4] {
        for kind in 0..8 {
            for lock_case in 0..LOCK_CASES {
                for v in 0..8 {
                    let bytes = pool(degree).install(|| run_lock(kind, lock_case, v));
                    let key = format!("lock {degree} {kind} {lock_case} {v}");
                    check(&g, &key, &bytes);
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 1024);
}
/// ロックを立てた層のマスクへの書き込みも同じ。画像・透明部分のロックでは通り、すべてのロック（層か親のグループ）でだけ断る。
/// マスクの無い層は、どのロックでも「マスクが無い」で断る（ロックの拒否より先）。文書の選択範囲を立てた事例を含む。
#[test]
fn csharp_locked_layer_mask_writes_all_bytes_at_both_parallel_degrees() {
    let g = golden();
    let mut checked = 0;
    for degree in [1, 4] {
        for kind in 0..4 {
            for lock_case in 0..LOCK_CASES {
                for v in 0..8 {
                    let bytes = pool(degree).install(|| run_lock_mask(kind, lock_case, v));
                    let key = format!("lockmask {degree} {kind} {lock_case} {v}");
                    check(&g, &key, &bytes);
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 512);
}
/// 塗りつぶし・調整・グループの層への書き込みで、型の拒否とロックの拒否のどちらが先かも C# と一致する（拒否の層・持ち主・ロック、断ったあとの
/// 各チャンネルの有効とマスク、マスクへの書き込みが通るときの Undo/Redo まで）。拒否だけの事例が主なので並列度は 1 だけ。
#[test]
fn csharp_non_raster_layer_writes_order_type_and_lock_refusals_the_same() {
    let g = golden();
    let mut checked = 0;
    for entry in 0..12 {
        for target in 0..3 {
            for lock_case in 0..LOCK_CASES {
                for v in 0..2 {
                    let bytes = pool(1).install(|| run_lock_kind(entry, target, lock_case, v));
                    let key = format!("locknr {entry} {target} {lock_case} {v}");
                    check(&g, &key, &bytes);
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 576);
}
/// 書いた結果の先頭の 1 バイト（と、ロックで断ったときの層・持ち主・ロック）を読み直したもの。
#[derive(Debug, PartialEq, Clone, Copy)]
enum Outcome {
    Written,
    /// 層・持ち主の並びの位置とロックの番号
    Locked(i32, i32, i32),
    /// ロック以外の理由（型・無効のチャンネル・マスクが無い）で断った
    Other,
}
fn outcome_of(out: &[u8]) -> Outcome {
    let int = |at: usize| i32::from_le_bytes(out[at..at + 4].try_into().unwrap());
    match out[0] {
        0 => Outcome::Written,
        1 => Outcome::Locked(int(1), int(5), int(9)),
        _ => Outcome::Other,
    }
}
/// ロックの規則だけから決めた、書き込みを断るロック（層は 0 番。グループに掛けたロックの持ち主は 1 番）。
/// 画像・すべては断り、消すときだけ透明部分でも断る。位置のロックは塗りを妨げない。マスクはすべてのロックだけで断る（erase は無視）。
fn locking(lock_case: u32, erase: bool, mask: bool) -> Option<Outcome> {
    let lock = match (LOCK_BITS[lock_case as usize], erase) {
        (LayerLocks::ALL, _) => LayerLocks::ALL,
        (LayerLocks::PIXELS, _) if !mask => LayerLocks::PIXELS,
        (LayerLocks::TRANSPARENCY, true) if !mask => LayerLocks::TRANSPARENCY,
        _ => return None,
    };
    Some(Outcome::Locked(
        0,
        i32::from(lock_case >= 5),
        i32::from(lock.bits()),
    ))
}
/// 正解と一致するだけでは、双方が同じ誤りでも通る。どの組合せがどう終わるべきかを、ロックの規則と「どの拒否が先か」の順（C# の検査の順）から
/// 独立に決めて、全事例で確かめる。事例の数は規則から数えた値で固定する（規則が崩れて数がずれたら落ちる）。
#[test]
fn locked_layer_outcomes_follow_the_lock_rules() {
    let mut tally = [0u32; 3]; // 断った（ロック）・断った（そのほか）・通った
    let mut count = |actual: Outcome, expected: Outcome, context: &dyn Fn() -> String| {
        assert_eq!(actual, expected, "{}", context());
        tally[match actual {
            Outcome::Locked(..) => 0,
            Outcome::Other => 1,
            Outcome::Written => 2,
        }] += 1;
    };
    // 画素の書き込み: 8 種 × ロックの場面 8 × 変種 8
    for kind in 0..8 {
        for lock_case in 0..LOCK_CASES {
            for v in 0..8 {
                let (erase, off) = (v & 1 != 0, v & 2 != 0);
                let expected = match kind {
                    // 単チャンネルの範囲の塗りは、無効のチャンネルを先に断る（ロックの前）
                    5 | 6 if off => Outcome::Other,
                    // 単チャンネルのストロークはロックが先で、無効のチャンネルはそのあと
                    7 => locking(lock_case, erase, false).unwrap_or(if off {
                        Outcome::Other
                    } else {
                        Outcome::Written
                    }),
                    _ => locking(lock_case, erase, false).unwrap_or(Outcome::Written),
                };
                let out = run_lock(kind, lock_case, v);
                count(outcome_of(&out), expected, &|| {
                    format!("kind {kind} lock {lock_case} v {v}")
                });
            }
        }
    }
    // マスクの書き込み: 4 種 × ロックの場面 8 × 変種 8（ビット 1 がマスク無し: ロックより先に「マスクが無い」で断る）
    for kind in 0..4 {
        for lock_case in 0..LOCK_CASES {
            for v in 0..8 {
                let expected = if v & 2 != 0 {
                    Outcome::Other
                } else {
                    locking(lock_case, false, true).unwrap_or(Outcome::Written)
                };
                let out = run_lock_mask(kind, lock_case, v);
                count(outcome_of(&out), expected, &|| {
                    format!("mask kind {kind} lock {lock_case} v {v}")
                });
            }
        }
    }
    // 画素 512 事例: ロックで断る 280（マテリアル 5 種は各 40、単チャンネルの塗り 2 種は無効にしない半分の 20 ずつ、ストロークは 40）・
    // ほかの理由で断る 76（無効のチャンネルへの単チャンネル 3 種）・通る 156。マスク 256 事例: ロックで 32（4 種 × すべての 2 場面 × 4 変種）・
    // マスク無しで 128・通る 96。
    assert_eq!(tally, [280 + 32, 76 + 128, 156 + 96]);
}
/// 塗りつぶし・調整・グループの層への書き込み。型で断る入口は、ロックの有無に関わらず型で断る（ロックの名指しは出ない）。ただし単チャンネルの
/// ストローク（begin_stroke）は、調整・グループではロックが先（C# は GetChannel が型で断るのをロックの後に置く）で、塗りつぶしだけ型が先。
/// マスクへの書き込みはどの種類の層にも通り、すべてのロックでだけ断る。
#[test]
fn non_raster_layer_writes_refuse_by_type_before_the_lock_except_the_single_channel_stroke() {
    let mut tally = [0u32; 3];
    for entry in 0..12 {
        for target in 0..3 {
            for lock_case in 0..LOCK_CASES {
                for v in 0..2 {
                    let erase = v & 1 != 0;
                    let expected = if entry >= 8 {
                        locking(lock_case, erase, true).unwrap_or(Outcome::Written)
                    } else if entry == 0 && target != 0 {
                        locking(lock_case, erase, false).unwrap_or(Outcome::Other)
                    } else {
                        Outcome::Other
                    };
                    let out = run_lock_kind(entry, target, lock_case, v);
                    let actual = outcome_of(&out);
                    assert_eq!(
                        actual, expected,
                        "entry {entry} target {target} lock {lock_case} v {v}"
                    );
                    tally[match actual {
                        Outcome::Locked(..) => 0,
                        Outcome::Other => 1,
                        Outcome::Written => 2,
                    }] += 1;
                }
            }
        }
    }
    // ロックで断る 68（begin_stroke の調整・グループ 2 × 10 と、マスクの 4 入口 × 3 種 × すべての 2 場面 × 2 変種 = 48）・
    // 型で断る 364（画素 8 入口 × 3 種 × 16 から 20 を除いた数）・通る 144（マスク 4 入口 × 3 種 × ロックの 6 場面 × 2 変種）
    assert_eq!(tally, [20 + 48, 364, 144]);
}
