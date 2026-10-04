#![allow(clippy::chunks_exact_to_as_chunks)]
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
fn golden() -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in include_str!("golden/material.txt")
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
    for key in &expected {
        assert!(g.contains_key(key), "正解に無い事例: {key}");
    }
    assert_eq!(expected.len(), g.len());
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
                    assert_eq!(sha(&bytes), g[&key], "{key}");
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
                    assert_eq!(sha(&bytes), g[&key], "{key}");
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
