//! 3D ビューの表示の写しの塗り広げ（UV の外へ。`view3d::paint` の `set_padding`）: アイランドの中は正本と同じバイト・アイランドの外は書き出しと同じ式で
//! 塗り広がる・描いたタイルの周りだけの塗り広げ直しが全体の塗り広げと同じ・モデルの UV が替わると覆いを作り直す・離れて見たときの継ぎ目。
use crate::common;

use common::*;
use egui_kittest::Harness;
use yolu_app::view3d::model::ViewModel;
use yolu_app::view3d::paint::{
    reduce_premultiplied, reduce_srgb_premultiplied, Slot, DISPLAY_PAD_TEXELS,
};
use yolu_app::YoluApp;
use yolu_core::geometry::{cube_sphere, ModelMesh, OrbitCamera, Submesh};
use yolu_core::glam::{DVec2, Vec2, Vec3};
use yolu_core::padding::{self, Reach};
use yolu_core::{
    Channel, Document, HeightEdgeMode, LayerId, NormalSettings, NormalYDirection, Rect, Rgba8,
};

/// 立方体を膨らませた球（面ごとに 3 × 2 の UV アイランド）のアイランドを、アイランドの中心のまわりに `scale` 倍へ縮めたもの（アイランドの外の隙間を広げる）。
fn islands(n: u32, scale: f32) -> ModelMesh {
    let mut mesh = cube_sphere(n, 0.5);
    let per_face = ((n + 1) * (n + 1)) as usize;
    for (i, uv) in mesh.uvs.iter_mut().enumerate() {
        let face = i / per_face;
        let center = yolu_core::glam::Vec2::new(
            (face % 3) as f32 / 3.0 + 1.0 / 6.0,
            (face / 3) as f32 * 0.5 + 0.25,
        );
        *uv = center + (*uv - center) * scale;
    }
    mesh
}

fn view(width: f32, height: f32, doc: u32) -> Harness<'static, YoluApp> {
    view_on(app(width, height, doc))
}

/// 計測用の `view`: 装置をアダプターの上限で作る（OpenGL でも compute を使う道を通す。`shared_gpu::renderer_with_adapter_limits`）。
fn view_for_measure(width: f32, height: f32, doc: u32) -> Harness<'static, YoluApp> {
    view_on(app_with_renderer(
        width,
        height,
        doc,
        common::shared_gpu::renderer_with_adapter_limits(),
    ))
}

fn view_on(mut h: Harness<'static, YoluApp>) -> Harness<'static, YoluApp> {
    h.state_mut()
        .state
        .doc
        .restore_look(yolu_core::look::MaterialLook::default())
        .unwrap();
    click_tab(&mut h, yolu_app::Tab::View3d);
    move_to(&h, egui::pos2(1.0, 1.0));
    h.run();
    h
}

fn set_model(h: &mut Harness<'_, YoluApp>, meshes: Vec<ModelMesh>) {
    let state = &mut h.state_mut().state;
    let revision = state.view3d.next_revision();
    state.view3d.material = 0;
    let model = ViewModel::new("試験", meshes, vec![Some("試験".to_string())], revision).unwrap();
    state.view3d.set_model(model);
    h.run();
}

fn look_at(h: &mut Harness<'_, YoluApp>, distance: f32, yaw: f32, pitch: f32) {
    h.state_mut().state.view3d.camera = OrbitCamera {
        target: Vec3::ZERO,
        yaw,
        pitch,
        distance,
        model_radius: 1.0,
        ..Default::default()
    };
    h.run();
}

/// メッシュの UV の三角形が覆うテクセル（書き出しと同じ `padding::coverage`）。
fn keep_of(mesh: &ModelMesh, size: u32) -> Vec<bool> {
    let s = size as f64;
    let at = |i: u32| {
        let uv = mesh.uvs[i as usize];
        DVec2::new(uv.x as f64 * s, uv.y as f64 * s)
    };
    let triangles = mesh
        .submeshes
        .iter()
        .flat_map(|sub| sub.indices.chunks(3))
        .map(|t| [at(t[0]), at(t[1]), at(t[2])]);
    padding::coverage(size, size, triangles).unwrap()
}

fn first_layer(h: &Harness<'_, YoluApp>) -> LayerId {
    h.state().state.doc.layers()[0].id()
}

/// レイヤーのチャンネルに、`at` が返す画素を丸ごと読み込む（行は下から。タイルごと）。
fn import(doc: &mut Document, layer: LayerId, channel: Channel, at: impl Fn(u32, u32) -> [u8; 4]) {
    let ts = doc.tile_size();
    let coords: Vec<_> = doc.canvas_tiles().collect();
    for c in coords {
        let r = doc.tile_rect(c).unwrap();
        let mut bytes = vec![0u8; (ts * ts * 4) as usize];
        for y in 0..r.height {
            for x in 0..r.width {
                let i = ((y * ts + x) * 4) as usize;
                bytes[i..i + 4].copy_from_slice(&at(r.x + x, r.y + y));
            }
        }
        doc.import_tile(layer, channel, c, &bytes).unwrap();
    }
}

/// アイランドの中（覆うテクセル）だけを塗った文書: Color はアイランドごとに違う色（アイランドの外は透明）、Roughness はアイランドの中だけ 200。
fn paint_inside(h: &mut Harness<'_, YoluApp>, keep: &[bool]) {
    let layer = first_layer(h);
    let doc = &mut h.state_mut().state.doc;
    let size = doc.width();
    import(doc, layer, Channel::Color, |x, y| {
        if keep[(y * size + x) as usize] {
            let island = (x * 3 / size + 3 * (y * 2 / size)) as u8;
            [60 + island * 30, 200 - island * 20, 90, 255]
        } else {
            [0, 0, 0, 0]
        }
    });
    import(doc, layer, Channel::Roughness, |x, y| {
        if keep[(y * size + x) as usize] {
            [200, 200, 200, 255]
        } else {
            [0, 0, 0, 0]
        }
    });
}

/// 表示の写しの塗り広げの幅（文書の画素）: 縮めて持つ絵は 2^縮め 倍の文書の画素で塗り広げてから縮める。段の地図の段数の上限で頭打ち。
fn reach_for(shift: u32) -> u32 {
    (DISPLAY_PAD_TEXELS << shift).min(padding::MAX_RING_REACH)
}

/// 文書の解像度で書き出しと同じ式（`padding::dilate`）で塗り広げた straight RGBA8。
fn dilated(straight: &[u8], doc: &Document, keep: &[bool], shift: u32) -> Vec<u8> {
    padding::dilate(
        straight,
        doc.width(),
        doc.height(),
        keep,
        Reach::Texels(reach_for(shift)),
        u64::MAX,
    )
    .unwrap()
}

/// 書き出しと同じ式で塗り広げた Color の表示の段 0（乗算済みの sRGB）。
fn expected_color(doc: &Document, keep: &[bool]) -> Vec<u8> {
    expected_color_at(doc, keep, 0)
}

/// `expected_color` の、縮め `shift` の絵（文書の解像度で塗り広げてから縮める）。
fn expected_color_at(doc: &Document, keep: &[bool], shift: u32) -> Vec<u8> {
    let straight = doc.composite_channel(Channel::Color, doc.bounds()).unwrap();
    reduce_srgb_premultiplied(&dilated(&straight, doc, keep, shift), doc.bounds(), shift).0
}

/// 書き出しと同じ式で塗り広げた Roughness の表示の段 0（値 × アルファ、1 チャンネル）。
fn expected_scalar(doc: &Document, channel: Channel, keep: &[bool]) -> Vec<u8> {
    expected_scalar_at(doc, channel, keep, 0)
}

/// `expected_scalar` の、縮め `shift` の絵。
fn expected_scalar_at(doc: &Document, channel: Channel, keep: &[bool], shift: u32) -> Vec<u8> {
    let straight = doc.composite_channel(channel, doc.bounds()).unwrap();
    let padded = dilated(&straight, doc, keep, shift);
    // 値 × アルファを箱で平均する（乗算済みの 1 つ目の値と同じ式）
    reduce_premultiplied(&padded, doc.bounds(), shift)
        .0
        .chunks(4)
        .map(|p| p[0])
        .collect()
}

/// 書き出しと同じ式で塗り広げた Normal の出力（OpenGL Y+・不透明）の、縮め `shift` の絵。
fn expected_normal_at(doc: &Document, keep: &[bool], shift: u32) -> Vec<u8> {
    let output = doc.normal_output(u64::MAX).unwrap();
    assert!(
        output.chunks(4).all(|p| p[3] == 255),
        "Normal の出力は不透明"
    );
    // 不透明なので、乗算済みにしても値は同じ
    reduce_premultiplied(&dilated(&output, doc, keep, shift), doc.bounds(), shift).0
}

fn level0(h: &Harness<'_, YoluApp>, slot: Slot) -> Vec<u8> {
    h.state()
        .view3d_read_paint_level(slot, 0)
        .expect("使っているチャンネル")
        .0
}

#[test]
fn the_view_copy_keeps_island_texels_and_fills_outside_like_the_export() {
    let size = 256;
    let mut h = view(900.0, 700.0, size);
    let mesh = islands(8, 0.6);
    let keep = keep_of(&mesh, size);
    paint_inside(&mut h, &keep);
    set_model(&mut h, vec![mesh]);
    look_at(&mut h, 2.5, 30.0, 20.0);
    let color = level0(&h, Slot::Color);
    let doc = &h.state().state.doc;
    // アイランドの中は正本（合成）を塗り広げずに上げたのと同じバイト
    let plain = reduce_srgb_premultiplied(
        &doc.composite_channel(Channel::Color, doc.bounds()).unwrap(),
        doc.bounds(),
        0,
    )
    .0;
    let mut filled = 0;
    for (i, &k) in keep.iter().enumerate() {
        let (a, b) = (&color[i * 4..i * 4 + 4], &plain[i * 4..i * 4 + 4]);
        if k {
            assert_eq!(a, b, "アイランドの中のテクセル {i}");
        } else if b[3] == 0 && a[3] > 0 {
            filled += 1;
        }
    }
    assert!(filled > 0, "アイランドの外が塗り広がっている");
    // 全体が、書き出しと同じ式（`padding::dilate`、同じ段数）で塗り広げたものと同じ
    assert!(color == expected_color(doc, &keep), "Color の段 0");
    assert!(
        level0(&h, Slot::Roughness) == expected_scalar(doc, Channel::Roughness, &keep),
        "Roughness の段 0"
    );
    // 届く幅の外（アイランドから DISPLAY_PAD_TEXELS より遠い所）は元のまま（透明）
    let rings = padding::Rings::new(size, size, &keep, DISPLAY_PAD_TEXELS).unwrap();
    let far = (0..size * size)
        .filter(|&i| rings.ring(i % size, i / size).is_none())
        .collect::<Vec<_>>();
    assert!(!far.is_empty(), "隙間が塗り広げの幅より広い場面");
    assert!(far.iter().all(|&i| color[i as usize * 4 + 3] == 0));
}

/// UV の [lo, hi]² を覆う板 1 枚（三角形 2 つ）。
fn quad_island(lo: f32, hi: f32) -> ModelMesh {
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
            Vec2::new(lo, lo),
            Vec2::new(hi, lo),
            Vec2::new(lo, hi),
            Vec2::new(hi, hi),
        ],
        submeshes: vec![Submesh {
            material: 0,
            indices: vec![0, 2, 1, 2, 3, 1],
        }],
    }
}

#[test]
fn repadding_only_the_painted_tiles_matches_padding_the_whole_image() {
    // 1024² の文書（128² のタイル）に、UV の [0.1, 0.49]² のアイランド 1 つ。タイルは 4 通りになる: アイランドの縁をまたぐ・アイランドの奥（塗り広げる
    // テクセルが届く幅の外）・アイランドの外で塗り広げるテクセルがある・アイランドから遠い
    let size = 1024;
    let mut h = view(900.0, 700.0, size);
    let mesh = quad_island(0.1, 0.49);
    let keep = keep_of(&mesh, size);
    paint_inside(&mut h, &keep);
    set_model(&mut h, vec![mesh]);
    look_at(&mut h, 2.0, 180.0, 0.0);
    let layer = first_layer(&h);
    let rings = padding::Rings::new(size, size, &keep, DISPLAY_PAD_TEXELS).unwrap();
    let ts = h.state().state.doc.tile_size();
    assert_eq!(ts, 128);
    let tile = |tx: u32, ty: u32| Rect::new(tx * ts, ty * ts, ts, ts);
    let grown = |r: Rect| {
        let d = DISPLAY_PAD_TEXELS;
        let (x0, y0) = (r.x.saturating_sub(d), r.y.saturating_sub(d));
        Rect::new(
            x0,
            y0,
            (r.x + r.width + d).min(size) - x0,
            (r.y + r.height + d).min(size) - y0,
        )
    };
    let cases = [
        ("アイランドの縁をまたぐ", tile(0, 1), (true, true)),
        ("アイランドの奥", tile(1, 1), (true, false)),
        ("アイランドの外の塗り広げ", tile(4, 1), (false, true)),
        ("アイランドから遠い", tile(6, 6), (false, false)),
    ];
    for (name, t, (has_keep, near)) in cases {
        assert_eq!(
            rings.kinds_in(t).0,
            has_keep,
            "{name}: アイランドの中を含む"
        );
        let near_filled = if has_keep {
            rings.kinds_in(grown(t)).1
        } else {
            rings.kinds_in(t).1
        };
        assert_eq!(near_filled, near, "{name}: 塗り広げるテクセル");
        let before = h.state().view3d_stats().unwrap();
        {
            let doc = &mut h.state_mut().state.doc;
            let (cx, cy) = (t.x + ts / 2, t.y + ts / 2);
            for y in cy - 3..cy + 3 {
                for x in cx - 3..cx + 3 {
                    doc.set_channel_pixel(
                        layer,
                        Channel::Color,
                        x,
                        y,
                        Rgba8::new(250, 20, 200, 255),
                    )
                    .unwrap();
                    doc.set_channel_pixel(
                        layer,
                        Channel::Roughness,
                        x,
                        y,
                        Rgba8::new(30, 30, 30, 255),
                    )
                    .unwrap();
                }
            }
        }
        h.run();
        let after = h.state().view3d_stats().unwrap();
        assert!(!after.last_rebuilt, "{name}: 描いたタイルだけを上げる");
        assert_eq!(
            after.total_slot_tiles[Slot::Color.index()]
                - before.total_slot_tiles[Slot::Color.index()],
            1,
            "{name}"
        );
        let doc = &h.state().state.doc;
        assert!(
            level0(&h, Slot::Color) == expected_color(doc, &keep),
            "{name}: タイルだけの Color が全体の塗り広げと同じ"
        );
        assert!(
            level0(&h, Slot::Roughness) == expected_scalar(doc, Channel::Roughness, &keep),
            "{name}: タイルだけの Roughness が全体の塗り広げと同じ"
        );
    }
    // 全部を作り直しても同じ
    let partial = (level0(&h, Slot::Color), level0(&h, Slot::Roughness));
    h.state_mut().view3d_invalidate_paint();
    h.run();
    assert!(h.state().view3d_stats().unwrap().last_rebuilt);
    assert!(level0(&h, Slot::Color) == partial.0);
    assert!(level0(&h, Slot::Roughness) == partial.1);
}

/// タイル（`size` のキャンバスの中）の中にアイランドの中のテクセルがあるか・塗り広げるテクセルが届く所にあるか（アプリと同じ見方: アイランドの中を含むなら、
/// タイルを段数だけ広げた所の塗り広げるテクセル、含まないならタイルの中の塗り広げるテクセル）。
fn tile_kinds(rings: &padding::Rings, tile: Rect, size: u32) -> (bool, bool) {
    let d = rings.reach();
    let (x0, y0) = (tile.x.saturating_sub(d), tile.y.saturating_sub(d));
    let grown = Rect::new(
        x0,
        y0,
        (tile.x + tile.width + d).min(size) - x0,
        (tile.y + tile.height + d).min(size) - y0,
    );
    let has_keep = rings.kinds_in(tile).0;
    let near = if has_keep {
        rings.kinds_in(grown).1
    } else {
        rings.kinds_in(tile).1
    };
    (has_keep, near)
}

/// タイルの中を描く（真ん中の 6 × 6 と、タイルの左下の隅から 2 画素の 6 × 6。隅の分は塗り広げの届く先がタイルの外へ出る）。
fn touch(doc: &mut Document, layer: LayerId, tile: Rect, channels: &[(Channel, Rgba8)]) {
    let origins = [
        (tile.x + tile.width / 2 - 3, tile.y + tile.height / 2 - 3),
        (tile.x + 2, tile.y + 2),
    ];
    for (ox, oy) in origins {
        for y in oy..oy + 6 {
            for x in ox..ox + 6 {
                for (channel, v) in channels {
                    doc.set_channel_pixel(layer, *channel, x, y, *v).unwrap();
                }
            }
        }
    }
}

/// (名前, タイルの番号, アイランドの中を含むか, 塗り広げるテクセルが届くか)
type Cases<'a> = &'a [(&'a str, (u32, u32), (bool, bool))];

/// 縮めて持つ今のセット（予算で縮める）: 1 タイルを描いたあとの段 0 が、文書の解像度で塗り広げてから縮めたものと同じ（塗り広げの幅は
/// 文書の画素で `16 << shift`、段の地図の上限 254 で頭打ち）。アイランドは UV の右上に寄せて、キャンバスの端で切れる場合と、塗り広げの幅が縮めの境
/// （2^shift）の倍数でなくなる場合（頭打ちの 254）の合わせ方も通す。
fn check_shrunk_current(size: u32, shift: u32, budget: u64, cases: Cases<'_>) {
    let mut h = view(900.0, 700.0, size);
    let mesh = quad_island(0.55, 1.0);
    let keep = keep_of(&mesh, size);
    paint_inside(&mut h, &keep);
    h.state_mut().view3d_set_paint_budget(budget);
    set_model(&mut h, vec![mesh]);
    look_at(&mut h, 2.0, 180.0, 0.0);
    assert_eq!(
        h.state().view3d_stats().unwrap().paint_level,
        shift,
        "予算で縮めて持つ"
    );
    let small = size >> shift;
    assert_eq!(
        h.state().view3d_read_paint_level(Slot::Color, 0).unwrap().1,
        [small, small]
    );
    let doc = &h.state().state.doc;
    assert!(
        level0(&h, Slot::Color) == expected_color_at(doc, &keep, shift),
        "作った直後の Color"
    );
    let layer = first_layer(&h);
    let rings = padding::Rings::new(size, size, &keep, reach_for(shift)).unwrap();
    let ts = h.state().state.doc.tile_size();
    for (name, (tx, ty), kinds) in cases {
        let tile = Rect::new(tx * ts, ty * ts, ts, ts);
        assert_eq!(tile_kinds(&rings, tile, size), *kinds, "{name}: 場合");
        let before = h.state().view3d_stats().unwrap();
        touch(
            &mut h.state_mut().state.doc,
            layer,
            tile,
            &[
                (Channel::Color, Rgba8::new(250, 20, 200, 255)),
                (Channel::Roughness, Rgba8::new(30, 30, 30, 255)),
            ],
        );
        h.run();
        let after = h.state().view3d_stats().unwrap();
        assert!(!after.last_rebuilt, "{name}: 描いたタイルだけを上げる");
        assert_eq!(
            after.total_slot_tiles[Slot::Color.index()]
                - before.total_slot_tiles[Slot::Color.index()],
            1,
            "{name}"
        );
        let doc = &h.state().state.doc;
        assert!(
            level0(&h, Slot::Color) == expected_color_at(doc, &keep, shift),
            "{name}: タイルだけの Color が、文書の解像度で塗り広げてから縮めたものと同じ"
        );
        assert!(
            level0(&h, Slot::Roughness)
                == expected_scalar_at(doc, Channel::Roughness, &keep, shift),
            "{name}: タイルだけの Roughness が同じ"
        );
    }
    // 全部を作り直しても同じ
    let partial = (level0(&h, Slot::Color), level0(&h, Slot::Roughness));
    h.state_mut().view3d_invalidate_paint();
    h.run();
    assert!(h.state().view3d_stats().unwrap().last_rebuilt);
    assert!(level0(&h, Slot::Color) == partial.0);
    assert!(level0(&h, Slot::Roughness) == partial.1);
}

#[test]
fn repadding_a_tile_of_a_shrunk_picture_matches_padding_the_document_then_shrinking() {
    // 1024² を 1/4 に（256²）: 幅は文書の画素で 64（縮めの境の倍数）
    check_shrunk_current(
        1024,
        2,
        1 << 20,
        &[
            ("アイランドの縁をまたぐ", (4, 4), (true, true)),
            ("アイランドの縁とキャンバスの端", (7, 4), (true, true)),
            ("アイランドの奥", (6, 6), (true, false)),
            ("アイランドの外の塗り広げ", (3, 6), (false, true)),
            ("アイランドから遠い", (0, 0), (false, false)),
        ],
    );
}

#[test]
fn repadding_a_tile_when_the_reach_is_capped_matches_padding_the_document_then_shrinking() {
    // 2048² を 1/16 に（128²）: 幅は 16 << 4 = 256 のはずが段の地図の上限 254 で頭打ちになり、縮めの境（16）の倍数でなくなる。
    // タイルの周りの塗り広げの入力は縮めの境へ広げ直す
    assert_eq!(reach_for(4), 254);
    check_shrunk_current(
        2048,
        4,
        200_000,
        &[
            ("アイランドの縁をまたぐ", (8, 8), (true, true)),
            ("アイランドの縁とキャンバスの端", (15, 8), (true, true)),
            ("アイランドの奥", (12, 12), (true, false)),
            ("アイランドの外の塗り広げ", (6, 12), (false, true)),
            ("アイランドから遠い", (0, 0), (false, false)),
        ],
    );
}

/// 2 枚の板（マテリアル 0・1）の Live Link のモデル。`meshes` の UV をそのまま使い、板が重ならないよう位置をずらす。
fn two_sets(size: u32, meshes: [ModelMesh; 2]) -> Harness<'static, YoluApp> {
    use yolu_protocol::{
        channel, ChannelRoute, MaterialInfo, MaterialKey, MeshData, Model, Submesh as ProtoSubmesh,
        TextureProperty,
    };
    let model = Model {
        generation: 1,
        name: "板".into(),
        materials: (0..2)
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
            .collect(),
        meshes: meshes
            .iter()
            .enumerate()
            .map(|(i, m)| MeshData {
                key: format!("{i}"),
                name: format!("板{i}"),
                skinned: false,
                positions: m
                    .positions
                    .iter()
                    .map(|p| [p.x + (i as f32 - 0.5) * 1.3, p.y, p.z])
                    .collect(),
                normals: vec![],
                uv0: m.uvs.iter().map(|uv| [uv.x, uv.y]).collect(),
                submeshes: vec![ProtoSubmesh {
                    material: i as u32,
                    indices: m.submeshes[0].indices.clone(),
                }],
            })
            .collect(),
    };
    let mut h = app(1100.0, 700.0, size);
    click_tab(&mut h, yolu_app::Tab::View3d);
    move_to(&h, egui::pos2(1.0, 1.0));
    h.run();
    h.state_mut().load_live_link_model(&model).unwrap();
    h.run();
    look_at(&mut h, 4.0, 180.0, 0.0);
    assert_eq!(h.state().state.sets.len(), 2);
    h
}

/// 文書 `set` の最初のレイヤーに、アイランドの中だけを塗った Color と Roughness（`paint_inside` の、今のセット以外の文書向け）。
fn paint_inside_set(h: &mut Harness<'_, YoluApp>, set: usize, keep: &[bool]) -> LayerId {
    let doc = h.state_mut().state.set_doc_mut(set);
    let layer = doc.layers()[0].id();
    let size = doc.width();
    import(doc, layer, Channel::Color, |x, y| {
        if keep[(y * size + x) as usize] {
            let island = (x * 3 / size + 3 * (y * 2 / size)) as u8;
            [60 + island * 30, 200 - island * 20, 90, 255]
        } else {
            [0, 0, 0, 0]
        }
    });
    import(doc, layer, Channel::Roughness, |x, y| {
        if keep[(y * size + x) as usize] {
            [200, 200, 200, 255]
        } else {
            [0, 0, 0, 0]
        }
    });
    layer
}

#[test]
fn repadding_a_tile_of_another_set_held_shrunk_matches_padding_the_document_then_shrinking() {
    // ほかのセットは辺の上限で縮めて持つ（1024² を上限 256 で 1/4 に）。上限の道は今のセットの予算の道と別の計画で縮めを決める
    let size = 1024;
    let shift = 2;
    let mut h = two_sets(size, [quad_island(0.55, 1.0), quad_island(0.55, 1.0)]);
    h.state_mut().view3d_set_other_cap(size >> shift);
    let model = h.state().state.view3d.model.clone().unwrap();
    // セット 1（マテリアル 1）の面の UV から覆いを作る（アプリが見ているモデルの UV）
    let keep = keep_of(&model.meshes[1], size);
    let layer = paint_inside_set(&mut h, 1, &keep);
    h.run();
    let read = |h: &Harness<'_, YoluApp>, slot: Slot| {
        let (texels, dims, level) = h
            .state()
            .view3d_read_other_level(1, slot, 0)
            .expect("ほかのセットの絵を持っている");
        assert_eq!((dims, level), ([size >> shift; 2], shift));
        texels
    };
    let doc = h.state().state.set_doc(1);
    assert!(
        read(&h, Slot::Color) == expected_color_at(doc, &keep, shift),
        "作った直後の Color"
    );
    assert!(
        read(&h, Slot::Roughness) == expected_scalar_at(doc, Channel::Roughness, &keep, shift),
        "作った直後の Roughness"
    );
    let rings = padding::Rings::new(size, size, &keep, reach_for(shift)).unwrap();
    let ts = h.state().state.set_doc(1).tile_size();
    // 同じセットの文書を、何度か 1 タイルずつ変える（そのたびに同期して、絵は文書の解像度で塗り広げてから縮めたものと同じ）
    for (name, (tx, ty), kinds) in [
        ("アイランドの縁をまたぐ", (4, 4), (true, true)),
        ("アイランドの縁とキャンバスの端", (7, 4), (true, true)),
        ("アイランドの外の塗り広げ", (3, 6), (false, true)),
        ("アイランドの奥", (6, 6), (true, false)),
    ] {
        let tile = Rect::new(tx * ts, ty * ts, ts, ts);
        assert_eq!(tile_kinds(&rings, tile, size), kinds, "{name}: 場合");
        touch(
            h.state_mut().state.set_doc_mut(1),
            layer,
            tile,
            &[
                (Channel::Color, Rgba8::new(250, 20, 200, 255)),
                (Channel::Roughness, Rgba8::new(30, 30, 30, 255)),
            ],
        );
        h.run();
        // ほかのセットは、作り終えたら段の地図を手放す（文書の大きさのメモリを、ほかのセットの数に比例して残さない）
        let stats = h.state().view3d_stats().unwrap();
        assert_eq!(stats.other_scratch_bytes, 0, "{name}: {stats:?}");
        let doc = h.state().state.set_doc(1);
        assert!(
            read(&h, Slot::Color) == expected_color_at(doc, &keep, shift),
            "{name}: ほかのセットの Color"
        );
        assert!(
            read(&h, Slot::Roughness) == expected_scalar_at(doc, Channel::Roughness, &keep, shift),
            "{name}: ほかのセットの Roughness"
        );
    }
}

/// Height から Normal を作る設定で、Height のタイルを描いたあとの Normal の段 0 が、Normal の出力を文書の解像度で塗り広げてから縮めた
/// ものと同じ（Sobel のために外へ 1 画素広げた出力から切り出して塗り広げる道）。アイランドは UV の左下に寄せて、キャンバスの端で切れる場合も通す。
fn check_normal_from_height(edges: HeightEdgeMode, shift: u32, budget: Option<u64>) {
    let size = 1024;
    let mut h = view(900.0, 700.0, size);
    let mesh = quad_island(0.0, 0.49);
    let keep = keep_of(&mesh, size);
    let layer = first_layer(&h);
    {
        let doc = &mut h.state_mut().state.doc;
        // アイランドの中だけに、緩い起伏の Height（アイランドの外は透明）
        import(doc, layer, Channel::Height, |x, y| {
            if keep[(y * size + x) as usize] {
                let v = (40 + (x * 3 + y * 5) % 160) as u8;
                [v, v, v, 255]
            } else {
                [0, 0, 0, 0]
            }
        });
        doc.set_normal_settings(
            NormalSettings::new(true, 8.0, edges, NormalYDirection::OpenGL).unwrap(),
            false,
        )
        .unwrap();
        assert!(doc.derives_normal());
    }
    if let Some(budget) = budget {
        h.state_mut().view3d_set_paint_budget(budget);
    }
    set_model(&mut h, vec![mesh]);
    look_at(&mut h, 2.0, 180.0, 0.0);
    assert_eq!(h.state().view3d_stats().unwrap().paint_level, shift);
    let doc = &h.state().state.doc;
    assert!(
        level0(&h, Slot::Normal) == expected_normal_at(doc, &keep, shift),
        "作った直後の Normal"
    );
    let rings = padding::Rings::new(size, size, &keep, reach_for(shift)).unwrap();
    let ts = h.state().state.doc.tile_size();
    let cases: Cases<'_> = &[
        ("アイランドの縁をまたぐ", (3, 3), (true, true)),
        ("アイランドの縁とキャンバスの端", (3, 0), (true, true)),
        ("アイランドの奥", (1, 1), (true, false)),
        ("アイランドの外の塗り広げ", (4, 1), (false, true)),
        ("アイランドから遠い", (6, 6), (false, false)),
    ];
    for (name, (tx, ty), kinds) in cases {
        let tile = Rect::new(tx * ts, ty * ts, ts, ts);
        assert_eq!(tile_kinds(&rings, tile, size), *kinds, "{name}: 場合");
        let before = h.state().view3d_stats().unwrap();
        touch(
            &mut h.state_mut().state.doc,
            layer,
            tile,
            &[(Channel::Height, Rgba8::new(250, 250, 250, 255))],
        );
        h.run();
        let after = h.state().view3d_stats().unwrap();
        if edges == HeightEdgeMode::Clamp {
            assert!(!after.last_rebuilt, "{name}: 描いたタイルだけを上げる");
        }
        assert!(
            after.total_slot_tiles[Slot::Normal.index()]
                > before.total_slot_tiles[Slot::Normal.index()],
            "{name}: Normal を上げた"
        );
        let doc = &h.state().state.doc;
        assert!(
            level0(&h, Slot::Normal) == expected_normal_at(doc, &keep, shift),
            "{name}: タイルだけの Normal が、出力全体を塗り広げてから縮めたものと同じ"
        );
    }
    // 全部を作り直しても同じ
    let partial = level0(&h, Slot::Normal);
    h.state_mut().view3d_invalidate_paint();
    h.run();
    assert!(h.state().view3d_stats().unwrap().last_rebuilt);
    assert!(level0(&h, Slot::Normal) == partial);
}

#[test]
fn repadding_a_tile_of_a_normal_made_from_height_matches_padding_the_whole_output() {
    check_normal_from_height(HeightEdgeMode::Clamp, 0, None);
}

#[test]
fn repadding_a_tile_of_a_shrunk_normal_made_from_height_matches_padding_the_whole_output() {
    // Height（1 チャンネル）と Normal（4 チャンネル）を 1/4 に（256²）縮めて持つ
    check_normal_from_height(HeightEdgeMode::Clamp, 2, Some(1 << 20));
}

#[test]
fn a_normal_made_from_height_that_wraps_around_the_edges_is_padded_like_the_whole_output() {
    // 端が反対側の端を読む設定は全部を 1 つの矩形で作る（部分では足りない）
    check_normal_from_height(HeightEdgeMode::Wrap, 0, None);
}

#[test]
fn a_model_with_other_uvs_rebuilds_the_coverage_and_a_new_pose_does_not() {
    let size = 256;
    let mut h = view(900.0, 700.0, size);
    let a = islands(8, 0.6);
    let keep_a = keep_of(&a, size);
    paint_inside(&mut h, &keep_a);
    set_model(&mut h, vec![a.clone()]);
    look_at(&mut h, 2.5, 30.0, 20.0);
    assert!(level0(&h, Slot::Color) == expected_color(&h.state().state.doc, &keep_a));
    // 同じ UV で位置だけ違う（ポーズを変えたのと同じ: 面の世代が変わる）モデルは、作り直さない
    let mut moved = a.clone();
    for p in &mut moved.positions {
        *p *= 1.1;
    }
    let tiles = h.state().view3d_stats().unwrap().total_tiles;
    set_model(&mut h, vec![moved]);
    let s = h.state().view3d_stats().unwrap();
    assert_eq!(s.total_tiles, tiles, "UV が同じなら上げ直さない: {s:?}");
    // UV が違うモデル: 覆いを作り直し、その覆いで塗り広げる（前のアイランドの外の塗り広げは残らない）
    let b = islands(8, 0.8);
    let keep_b = keep_of(&b, size);
    assert_ne!(keep_a, keep_b);
    set_model(&mut h, vec![b]);
    assert!(h.state().view3d_stats().unwrap().last_rebuilt);
    assert!(level0(&h, Slot::Color) == expected_color(&h.state().state.doc, &keep_b));
    // 塗り広げを切れば、前と同じ（アイランドの外は透明のまま）
    h.state_mut().view3d_set_display_padding(0);
    h.run();
    let doc = &h.state().state.doc;
    let plain = reduce_srgb_premultiplied(
        &doc.composite_channel(Channel::Color, doc.bounds()).unwrap(),
        doc.bounds(),
        0,
    )
    .0;
    assert!(level0(&h, Slot::Color) == plain);
}

/// 差が 8 を超える画素の数と、差の最大。
fn seam_count(inside: &image::RgbaImage, full: &image::RgbaImage) -> (usize, u8) {
    let mut count = 0;
    let mut max = 0u8;
    for (a, b) in inside.pixels().zip(full.pixels()) {
        let d = (0..3).map(|k| a.0[k].abs_diff(b.0[k])).max().unwrap();
        if d > 8 {
            count += 1;
        }
        max = max.max(d);
    }
    (count, max)
}

/// アイランドの中だけを塗った球と、全面を塗った球を、同じカメラで撮る。
fn seam_shots(
    h: &mut Harness<'_, YoluApp>,
    keep: &[bool],
    distance: f32,
) -> (image::RgbaImage, image::RgbaImage) {
    seam_shots_with(
        h,
        keep,
        [0, 0, 0, 0],
        [230, 120, 60, 255],
        (distance, 35.0, 25.0),
    )
}

/// `seam_shots` の、アイランドの外の色（`outside`。透明にせず塗りつぶしの色を置く）・中の色・カメラ（距離・ヨー・ピッチ）を選べる形。全面を塗った絵は、
/// 外にも中の色を置いたもの。
fn seam_shots_with(
    h: &mut Harness<'_, YoluApp>,
    keep: &[bool],
    outside: [u8; 4],
    inside: [u8; 4],
    (distance, yaw, pitch): (f32, f32, f32),
) -> (image::RgbaImage, image::RgbaImage) {
    let layer = first_layer(h);
    let size = h.state().state.doc.width();
    let colored = |x: u32, y: u32, keep: &[bool]| {
        if keep[(y * size + x) as usize] {
            inside
        } else {
            outside
        }
    };
    let shot = |h: &mut Harness<'_, YoluApp>| {
        look_at(h, distance, yaw, pitch);
        let image = h.render().unwrap();
        let rect = h.state().view3d_rect().unwrap();
        image::imageops::crop_imm(
            &image,
            rect.left() as u32,
            rect.top() as u32,
            rect.width() as u32,
            rect.height() as u32,
        )
        .to_image()
    };
    import(
        &mut h.state_mut().state.doc,
        layer,
        Channel::Color,
        |x, y| colored(x, y, keep),
    );
    h.run();
    let inside_shot = shot(h);
    import(
        &mut h.state_mut().state.doc,
        layer,
        Channel::Color,
        |_, _| inside,
    );
    h.run();
    let full = shot(h);
    (inside_shot, full)
}

/// 球の内側（球の輪郭の半径の 0.8 倍の中。縁の寝た面はミップの段が高く、塗り広げの幅では届かないので除く）で、差が 8 を超える
/// 画素の数。球の中心は表示域の中心（カメラは原点を見る）、半径は中心の行の物の画素の幅から。
fn interior_seams(inside: &image::RgbaImage, full: &image::RgbaImage) -> usize {
    let background = [31u8, 33, 38];
    let (w, h) = full.dimensions();
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let row: Vec<u32> = (0..w)
        .filter(|&x| {
            let p = full.get_pixel(x, h / 2).0;
            (0..3).any(|k| p[k].abs_diff(background[k]) > 3)
        })
        .collect();
    let r = (row.last().unwrap() - row.first().unwrap()) as f32 / 2.0;
    full.enumerate_pixels()
        .filter(|&(x, y, _)| ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt() < 0.8 * r)
        .filter(|&(x, y, b)| {
            let a = inside.get_pixel(x, y).0;
            (0..3).any(|k| a[k].abs_diff(b.0[k]) > 8)
        })
        .count()
}

#[test]
fn far_away_the_island_borders_no_longer_bleed_the_transparent_outside() {
    let size = 1024;
    let mut h = view(800.0, 600.0, size);
    let mesh = islands(16, 0.6);
    let keep = keep_of(&mesh, size);
    set_model(&mut h, vec![mesh]);
    // 球が表示域の高さの約 1/4（アイランドの 1 テクセルが画面の 1/4 画素ほど: ミップの段 2〜3）
    let distance = 6.0;
    h.state_mut().view3d_set_display_padding(0);
    h.state_mut().view3d_invalidate_paint();
    let (inside, full) = seam_shots(&mut h, &keep, distance);
    let before = interior_seams(&inside, &full);
    h.state_mut().view3d_set_display_padding(DISPLAY_PAD_TEXELS);
    h.state_mut().view3d_invalidate_paint();
    let (inside, full) = seam_shots(&mut h, &keep, distance);
    let after = interior_seams(&inside, &full);
    if let Some(dir) = std::env::var_os("PADDING_SEAM_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        inside.save(dir.join("far_inside.png")).unwrap();
        full.save(dir.join("far_full.png")).unwrap();
    }
    println!("球の内側で差が 8 を超える画素: 塗り広げなし {before}・あり {after}");
    assert!(
        before > 50,
        "塗り広げないとアイランドの縁がにじむ（{before}）"
    );
    assert_eq!(after, 0, "塗り広げるとアイランドの縁がにじまない");
    // アイランドの中だけを塗った絵（塗り広げあり）
    let layer = first_layer(&h);
    import(
        &mut h.state_mut().state.doc,
        layer,
        Channel::Color,
        |x, y| {
            if keep[(y * size + x) as usize] {
                [230, 120, 60, 255]
            } else {
                [0, 0, 0, 0]
            }
        },
    );
    h.run();
    h.snapshot("view3d_padding_far");
}

/// 平らな立方体（面ごとに 3 × 2 のセルの中に、セルの中心のまわりに `scale` 倍の正方形のアイランド）。面の辺が UV の継ぎ目になる。
fn cube_islands(scale: f32) -> ModelMesh {
    // 面の中心の向き・横・縦（外から見て、横 × 縦 が外向きになる並び。`cube_sphere` と同じ）
    let frames = [
        (Vec3::NEG_Z, Vec3::X, Vec3::Y),
        (Vec3::Z, Vec3::NEG_X, Vec3::Y),
        (Vec3::NEG_X, Vec3::NEG_Z, Vec3::Y),
        (Vec3::X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::X, Vec3::Z),
        (Vec3::NEG_Y, Vec3::X, Vec3::NEG_Z),
    ];
    let (mut positions, mut normals, mut uvs, mut indices) = (vec![], vec![], vec![], vec![]);
    for (face, (center, right, up)) in frames.into_iter().enumerate() {
        let start = positions.len() as u32;
        let cell = Vec2::new(
            (face % 3) as f32 / 3.0 + 1.0 / 6.0,
            (face / 3) as f32 * 0.5 + 0.25,
        );
        for j in 0..2 {
            for i in 0..2 {
                let (s, t) = (i as f32 * 2.0 - 1.0, j as f32 * 2.0 - 1.0);
                positions.push((center + right * s + up * t) * 0.5);
                normals.push(center);
                uvs.push(cell + Vec2::new(s, t) * (scale / 6.0));
            }
        }
        indices.extend_from_slice(&[start, start + 2, start + 1, start + 1, start + 2, start + 3]);
    }
    ModelMesh {
        name: "立方体".into(),
        positions,
        normals,
        uvs,
        submeshes: vec![Submesh {
            material: 0,
            indices,
        }],
    }
}

/// 外側が白（不透明）・内側が黒の文書を、カメラごとに撮ったときの、全面を黒く塗った絵と違う画素の数（アイランドの外の白がミップに混ざった数）。
fn white_outside_seams(
    mesh: ModelMesh,
    size: u32,
    shots: &[(&str, (f32, f32, f32))],
) -> Vec<(String, usize)> {
    let mut h = view(800.0, 600.0, size);
    let keep = keep_of(&mesh, size);
    set_model(&mut h, vec![mesh]);
    shots
        .iter()
        .map(|&(name, camera)| {
            let (inside, full) = seam_shots_with(&mut h, &keep, [255; 4], [0, 0, 0, 255], camera);
            if let Some(dir) = std::env::var_os("PADDING_SEAM_DIR") {
                let dir = std::path::PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                inside
                    .save(dir.join(format!("white_{name}_inside.png")))
                    .unwrap();
                full.save(dir.join(format!("white_{name}_full.png")))
                    .unwrap();
            }
            (name.to_string(), seam_count(&inside, &full).0)
        })
        .collect()
}

const WHITE_OUTSIDE_SHOTS: [(&str, (f32, f32, f32)); 8] = [
    ("near", (3.0, 35.0, 25.0)),
    ("mid", (6.0, 35.0, 25.0)),
    ("far", (12.0, 35.0, 25.0)),
    ("farther", (24.0, 35.0, 25.0)),
    ("oblique", (4.0, 60.0, 70.0)),
    ("oblique_far", (8.0, 60.0, 70.0)),
    ("grazing", (4.0, 15.0, 80.0)),
    ("grazing_far", (8.0, 15.0, 80.0)),
];

#[test]
fn deep_mip_levels_never_mix_in_the_outside_fill_near_far_and_oblique() {
    // 立方体の面の UV アイランドはほぼ隣り合う（隙間は塗り広げの幅の内）。外側は白、内側は黒
    let seams = white_outside_seams(cube_islands(0.95), 1024, &WHITE_OUTSIDE_SHOTS);
    for (name, count) in &seams {
        println!("白い外側が混ざった画素（{name}）: {count}");
    }
    for (name, count) in &seams {
        assert_eq!(*count, 0, "{name}: アイランドの外の白がミップに混ざらない");
    }
}

// ───────── ミップマップは UV の上の画素だけで作る（押し引き） ─────────

type Levels = Vec<(Vec<u8>, [u32; 2])>;

/// 段 0 から最後の段までを読む。
fn all_levels(read: impl Fn(u32) -> Option<(Vec<u8>, [u32; 2])>) -> Levels {
    (0..).map_while(read).collect()
}

fn color_levels(h: &Harness<'_, YoluApp>, slot: Slot) -> Levels {
    all_levels(|l| h.state().view3d_read_paint_level(slot, l))
}

fn weight_levels(h: &Harness<'_, YoluApp>) -> Levels {
    all_levels(|l| h.state().view3d_read_paint_weight_level(l))
}

fn other_levels(h: &Harness<'_, YoluApp>, slot: Slot) -> Levels {
    other_levels_of(h, 1, slot)
}

/// マテリアル `material` のセット（今のセットでない）の絵の段。
fn other_levels_of(h: &Harness<'_, YoluApp>, material: i32, slot: Slot) -> Levels {
    all_levels(|l| {
        h.state()
            .view3d_read_other_level(material, slot, l)
            .map(|(texels, dims, _)| (texels, dims))
    })
}

fn other_weight_levels(h: &Harness<'_, YoluApp>) -> Levels {
    all_levels(|l| h.state().view3d_read_other_weight_level(1, l))
}

/// 外側を透明にせず塗りつぶしの色にした文書: 内側と外側で、Color は黒と白、Roughness は 40 と 250、Normal は (200, 100, 255) と (30, 220, 140)、
/// Emission は (10, 20, 30) と (240, 200, 100)（スカラー・sRGB・リニアの 3 つの形式をすべて通す）。
fn paint_with_outside_fill(doc: &mut Document, layer: LayerId, keep: &[bool]) {
    let size = doc.width();
    let inside = |x: u32, y: u32| keep[(y * size + x) as usize];
    for (channel, inner, outer) in [
        (Channel::Color, [0, 0, 0, 255], [255, 255, 255, 255]),
        (Channel::Roughness, [40, 40, 40, 255], [250, 250, 250, 255]),
        (Channel::Normal, [200, 100, 255, 255], [30, 220, 140, 255]),
        (Channel::Emission, [10, 20, 30, 255], [240, 200, 100, 255]),
    ] {
        import(doc, layer, channel, |x, y| {
            if inside(x, y) {
                inner
            } else {
                outer
            }
        });
    }
}

/// 使っているチャンネルの段（Color・Roughness・Normal・Emission）。
const FILLED_SLOTS: [(Slot, usize); 4] = [
    (Slot::Color, 4),
    (Slot::Roughness, 1),
    (Slot::Normal, 4),
    (Slot::Emission, 4),
];

/// 段 1 以降の全テクセルが、段 0 の真ん中のテクセル（アイランドの中）の値だけ: 外側の値を 1 つも含まない。チャンネルごとに、違うテクセルの数を返す。
fn texels_with_outside_fill(read: impl Fn(Slot) -> Levels) -> Vec<(Slot, usize)> {
    FILLED_SLOTS
        .iter()
        .map(|&(slot, bpt)| {
            let levels = read(slot);
            let [w, h] = levels[0].1;
            let at = ((h / 2 * w + w / 2) as usize) * bpt;
            let want = levels[0].0[at..at + bpt].to_vec();
            let off = levels
                .iter()
                .skip(1)
                .flat_map(|(t, _)| t.chunks(bpt))
                .filter(|t| t.iter().zip(&want).any(|(a, b)| a.abs_diff(*b) > 1))
                .count();
            (slot, off)
        })
        .collect()
}

#[test]
fn every_mip_level_holds_only_the_uv_colors_never_the_outside_fill() {
    // 256² の中央に 0.4 四方のアイランド。塗り広げの幅（16）の外は、段 0 では白のまま。段 1 以降は、どのテクセルも UV の中の色だけ
    let size = 256;
    let mut h = view(900.0, 700.0, size);
    let mesh = quad_island(0.3, 0.7);
    let keep = keep_of(&mesh, size);
    let layer = first_layer(&h);
    paint_with_outside_fill(&mut h.state_mut().state.doc, layer, &keep);
    set_model(&mut h, vec![mesh]);
    look_at(&mut h, 2.0, 180.0, 0.0);
    let color = color_levels(&h, Slot::Color);
    assert_eq!(color.len(), 9, "256² は 9 段");
    assert!(
        color[0].0.chunks(4).any(|t| t == [255, 255, 255, 255]),
        "段 0 の遠い外側は塗り広げの外で白のまま（試験の前提）"
    );
    let off = texels_with_outside_fill(|slot| color_levels(&h, slot));
    println!("段 1 以降で外側の値を含むテクセル: {off:?}");
    assert!(
        off.iter().all(|&(_, n)| n == 0),
        "段 1 以降に外側の値が混ざらない: {off:?}"
    );
}

#[test]
fn every_mip_level_of_a_shrunk_picture_holds_only_the_uv_colors() {
    // 512² を予算で 1/2（256²）に縮めて持つ今のセット（4 チャンネルと重みの絵で 1 テクセル 14 B）
    let size = 512;
    let mut h = view(900.0, 700.0, size);
    let mesh = quad_island(0.3, 0.7);
    let keep = keep_of(&mesh, size);
    let layer = first_layer(&h);
    paint_with_outside_fill(&mut h.state_mut().state.doc, layer, &keep);
    h.state_mut().view3d_set_paint_budget(2 << 20);
    set_model(&mut h, vec![mesh]);
    look_at(&mut h, 2.0, 180.0, 0.0);
    assert_eq!(h.state().view3d_stats().unwrap().paint_level, 1);
    assert_eq!(color_levels(&h, Slot::Color)[0].1, [256, 256]);
    let off = texels_with_outside_fill(|slot| color_levels(&h, slot));
    assert!(
        off.iter().all(|&(_, n)| n == 0),
        "縮めて持つ絵でも段 1 以降に外側の値が混ざらない: {off:?}"
    );
}

#[test]
fn every_mip_level_of_another_set_holds_only_the_uv_colors() {
    // ほかのセット（辺の上限で 1/2 に縮めて持つ）
    let size = 512;
    let mut h = two_sets(size, [quad_island(0.3, 0.7), quad_island(0.3, 0.7)]);
    h.state_mut().view3d_set_other_cap(size / 2);
    let model = h.state().state.view3d.model.clone().unwrap();
    let keep = keep_of(&model.meshes[1], size);
    {
        let doc = h.state_mut().state.set_doc_mut(1);
        let layer = doc.layers()[0].id();
        paint_with_outside_fill(doc, layer, &keep);
    }
    h.run();
    assert_eq!(
        other_levels(&h, Slot::Color)[0].1,
        [256, 256],
        "ほかのセットは縮めて持つ"
    );
    let off = texels_with_outside_fill(|slot| other_levels(&h, slot));
    assert!(
        off.iter().all(|&(_, n)| n == 0),
        "ほかのセットでも段 1 以降に外側の値が混ざらない: {off:?}"
    );
}

/// 離れた 2 枚の板（UV の [0.05, 0.3]² と [0.7, 0.95]² のアイランド）を 1 つのメッシュに。
fn two_quads() -> ModelMesh {
    let (a, b) = (quad_island(0.05, 0.3), quad_island(0.7, 0.95));
    let mut positions = a.positions.clone();
    positions.extend(b.positions.iter().map(|p| *p + Vec3::new(2.0, 0.0, 0.0)));
    let mut uvs = a.uvs.clone();
    uvs.extend(&b.uvs);
    let mut indices = a.submeshes[0].indices.clone();
    indices.extend(b.submeshes[0].indices.iter().map(|i| i + 4));
    ModelMesh {
        name: "2 枚の板".into(),
        positions,
        normals: vec![Vec3::NEG_Z; 8],
        uvs,
        submeshes: vec![Submesh {
            material: 0,
            indices,
        }],
    }
}

#[test]
fn the_pull_fills_texels_without_uv_pixels_from_the_coarser_level() {
    // 64² に離れた 2 つのアイランド（左は赤・右は青）、塗り広げの幅 2。段 1・2 では、読む粗い段（段 2・3）でも 2 つのアイランドは別々のテクセルにいる:
    // 重みが 0 でアイランドのとなりのテクセルは、そのアイランドの色そのもの（読む 4 つの中の重みが 0 でないテクセルだけで決まる）。
    // 2 つのアイランドの間の、どちらからも離れたテクセルは、2 つの色の間の値（赤と青の混ざった色）
    let size = 64;
    let mut h = view(900.0, 700.0, size);
    h.state_mut().view3d_set_display_padding(2);
    let mesh = two_quads();
    let keep = keep_of(&mesh, size);
    let layer = first_layer(&h);
    let red = [250u8, 20, 20, 255];
    let blue = [20u8, 30, 240, 255];
    import(
        &mut h.state_mut().state.doc,
        layer,
        Channel::Color,
        |x, y| {
            if keep[(y * size + x) as usize] {
                if x < size / 2 {
                    red
                } else {
                    blue
                }
            } else {
                [255, 255, 255, 255]
            }
        },
    );
    set_model(&mut h, vec![mesh]);
    look_at(&mut h, 3.0, 180.0, 0.0);
    let (colors, weights) = (color_levels(&h, Slot::Color), weight_levels(&h));
    assert_eq!(colors.len(), weights.len());
    let (mut adjacent, mut between) = (0, 0);
    for level in 1..=2 {
        let ([w, hh], texels, weight) = (colors[level].1, &colors[level].0, &weights[level].0);
        let at = |x: i64, y: i64| {
            (x >= 0 && y >= 0 && x < w as i64 && y < hh as i64)
                .then(|| (y as u32 * w + x as u32) as usize)
        };
        for y in 0..hh as i64 {
            for x in 0..w as i64 {
                let i = at(x, y).unwrap();
                let texel = &texels[i * 4..i * 4 + 4];
                if weight[i] > 0 {
                    assert!(
                        texel == red || texel == blue,
                        "段 {level} の重みのあるテクセルは、アイランドの色そのもの: {texel:?}"
                    );
                    continue;
                }
                let near: Vec<&[u8]> = (-1..=1)
                    .flat_map(|dy| (-1..=1).map(move |dx| (dx, dy)))
                    .filter_map(|(dx, dy)| at(x + dx, y + dy))
                    .filter(|&j| weight[j] > 0)
                    .map(|j| &texels[j * 4..j * 4 + 4])
                    .collect();
                if let Some(first) = near.first() {
                    assert!(
                        near.iter().all(|n| n == first),
                        "試験の前提: となりのアイランドは 1 つ"
                    );
                    assert_eq!(
                        texel, *first,
                        "段 {level} ({x}, {y}): アイランドのとなりの引いたテクセル"
                    );
                    adjacent += 1;
                } else {
                    for k in 0..3 {
                        let (lo, hi) = (red[k].min(blue[k]), red[k].max(blue[k]));
                        assert!(
                            (lo..=hi).contains(&texel[k]),
                            "段 {level} ({x}, {y}): 離れたテクセルは赤と青の間: {texel:?}"
                        );
                    }
                    assert_eq!(texel[3], 255);
                    between += 1;
                }
            }
        }
    }
    println!("となり {adjacent}・離れた {between}");
    assert!(adjacent > 50 && between > 50, "{adjacent} {between}");
}

/// 重みの絵の期待値（試験の中で素朴に数える。アプリの式を使わない）: 段 0 は表示のテクセルの箱（`2^shift` 四方の文書の画素。端は欠ける）の画素が全部、
/// 覆い・塗り広げの中なら 255・そうでなければ 0、段 1 以降は 1 つ上の段の箱（画素 y は `floor((2y + 1) m / (2n))` の箱）の平均の切り上げ。
fn expected_weights(rings: &padding::Rings, size: u32, shift: u32) -> Levels {
    let block = 1u32 << shift;
    let side = size.div_ceil(block);
    let mut level0 = vec![0u8; (side * side) as usize];
    for j in 0..side {
        for i in 0..side {
            let mut whole = true;
            for y in j * block..((j + 1) * block).min(size) {
                for x in i * block..((i + 1) * block).min(size) {
                    whole &= rings.ring(x, y).is_some();
                }
            }
            level0[(j * side + i) as usize] = if whole { 255 } else { 0 };
        }
    }
    let mut out: Levels = vec![(level0, [side, side])];
    loop {
        let (above, [n, _]) = out.last().unwrap().clone();
        if n == 1 {
            break;
        }
        let m = (n / 2).max(1);
        let boxed = |y: u32| (2 * y + 1) * m / (2 * n);
        let mut next = vec![(0u32, 0u32); (m * m) as usize];
        for y in 0..n {
            for x in 0..n {
                let cell = &mut next[(boxed(y) * m + boxed(x)) as usize];
                cell.0 += above[(y * n + x) as usize] as u32;
                cell.1 += 1;
            }
        }
        out.push((
            next.iter()
                .map(|&(sum, count)| sum.div_ceil(count) as u8)
                .collect(),
            [m, m],
        ));
    }
    out
}

/// 重みが 0 でないテクセルと、そのとなり（斜めを含む 8 近傍）のテクセル。描画が読む（双線形が読む）のはこの範囲だけ。
fn observable(weights: &[u8], [w, h]: [u32; 2]) -> Vec<bool> {
    let at = |x: i64, y: i64| {
        (0..w as i64).contains(&x)
            && (0..h as i64).contains(&y)
            && weights[(y as u32 * w + x as u32) as usize] > 0
    };
    (0..h as i64)
        .flat_map(|y| (0..w as i64).map(move |x| (x, y)))
        .map(|(x, y)| (-1..=1).any(|dy| (-1..=1).any(|dx| at(x + dx, y + dy))))
        .collect()
}

#[test]
fn the_weight_picture_counts_the_uv_pixels_of_each_texel_at_every_level() {
    // 縮めない・縮めて持つ（1/2・1/4）。覆い + 塗り広げの中の画素の割合を段ごとに箱で平均した絵が、GPU に上がっている
    for (size, shift, budget) in [
        (256, 0, None),
        (512, 1, Some(2u64 << 20)),
        (1024, 2, Some(2 << 20)),
    ] {
        let mut h = view(900.0, 700.0, size);
        let mesh = quad_island(0.3, 0.7);
        let keep = keep_of(&mesh, size);
        let layer = first_layer(&h);
        paint_with_outside_fill(&mut h.state_mut().state.doc, layer, &keep);
        if let Some(budget) = budget {
            h.state_mut().view3d_set_paint_budget(budget);
        }
        set_model(&mut h, vec![mesh]);
        look_at(&mut h, 2.0, 180.0, 0.0);
        assert_eq!(h.state().view3d_stats().unwrap().paint_level, shift);
        let rings = padding::Rings::new(size, size, &keep, reach_for(shift)).unwrap();
        let want = expected_weights(&rings, size, shift);
        let got = weight_levels(&h);
        assert_eq!(got.len(), want.len(), "{size}²: 色と同じ段の数");
        assert_eq!(got.len(), color_levels(&h, Slot::Color).len());
        for (level, (g, w)) in got.iter().zip(&want).enumerate() {
            assert_eq!(g.1, w.1, "{size}² 段 {level} の大きさ");
            assert!(g.0 == w.0, "{size}²（縮め {shift}）段 {level} の重み");
        }
        // 前提: 段 0 に 0 のテクセルがある（引くテクセルがある）。段 1 以降のどこかに 0 と 255 の間の値がある（箱の一部だけが重みのあるテクセル）。
        // 最後の段は 0 でない（どの段も、引く先の粗い段に重みのあるテクセルがある）
        assert!(got[0].0.contains(&0), "{size}²: 重みが 0 のテクセルがある");
        assert!(got[0].0.contains(&255));
        assert!(got[0].0.iter().all(|&v| v == 0 || v == 255));
        assert!(
            got[1..]
                .iter()
                .any(|(t, _)| t.iter().any(|&v| v > 0 && v < 255)),
            "{size}²"
        );
        assert!(*got.last().unwrap().0.last().unwrap() > 0);
    }
}

/// 2 つの絵の段（段 1 以降）を、重みの絵から出した「描画が読む」テクセルだけで比べて、違うテクセルの数を返す（`bpt` は 1 テクセルのバイト数）。
fn differing_observable(a: &Levels, b: &Levels, weights: &Levels, bpt: usize) -> usize {
    assert_eq!(a.len(), b.len());
    (1..a.len())
        .map(|l| {
            assert_eq!((a[l].1, a[l].0.len()), (b[l].1, b[l].0.len()), "段 {l}");
            let seen = observable(&weights[l].0, weights[l].1);
            seen.iter()
                .enumerate()
                .filter(|&(i, &on)| {
                    on && a[l].0[i * bpt..(i + 1) * bpt] != b[l].0[i * bpt..(i + 1) * bpt]
                })
                .count()
        })
        .sum()
}

/// 1 タイルを描いたあとのミップが、全部を作り直したものと、描画が読むテクセルで同じ（段 1 以降）。描くたびに全部を作り直した絵から始める。
fn check_mips_after_a_stroke_match_a_full_rebuild(
    h: &mut Harness<'_, YoluApp>,
    cases: Cases<'_>,
    size: u32,
    keep: &[bool],
    shift: u32,
    layer: LayerId,
    other_set: bool,
) {
    let read = |h: &Harness<'_, YoluApp>| {
        if other_set {
            (
                other_levels(h, Slot::Color),
                other_levels(h, Slot::Roughness),
                other_weight_levels(h),
            )
        } else {
            (
                color_levels(h, Slot::Color),
                color_levels(h, Slot::Roughness),
                weight_levels(h),
            )
        }
    };
    let rings = padding::Rings::new(size, size, keep, reach_for(shift)).unwrap();
    let ts = if other_set {
        h.state().state.set_doc(1).tile_size()
    } else {
        h.state().state.doc.tile_size()
    };
    let mut touched = 0;
    for (name, (tx, ty), kinds) in cases {
        let tile = Rect::new(tx * ts, ty * ts, ts, ts);
        assert_eq!(tile_kinds(&rings, tile, size), *kinds, "{name}: 場合");
        // 全部を作り直した絵から始める
        h.state_mut().view3d_invalidate_paint();
        h.run();
        h.run();
        touched += 1;
        let channels = [
            (Channel::Color, Rgba8::new(250, 20, 200, 255 - touched * 20)),
            (
                Channel::Roughness,
                Rgba8::new(30 + touched * 10, 30, 30, 255),
            ),
        ];
        let doc = if other_set {
            h.state_mut().state.set_doc_mut(1)
        } else {
            &mut h.state_mut().state.doc
        };
        touch(doc, layer, tile, &channels);
        h.run();
        let (partial_color, partial_roughness, weights) = read(h);
        h.state_mut().view3d_invalidate_paint();
        h.run();
        h.run();
        let (full_color, full_roughness, full_weights) = read(h);
        assert!(
            weights == full_weights,
            "{name}: 重みの絵は作り直しても同じ"
        );
        let differ_color = differing_observable(&partial_color, &full_color, &weights, 4);
        let differ_roughness =
            differing_observable(&partial_roughness, &full_roughness, &weights, 1);
        println!("{name}: 描いたタイルだけの更新と全部の作り直しで違う、描画が読むテクセル: Color {differ_color}・Roughness {differ_roughness}");
        assert_eq!(
            (differ_color, differ_roughness),
            (0, 0),
            "{name}: 描画が読むテクセルは、タイルだけの更新と全部の作り直しで同じ"
        );
        assert!(partial_color[0].0 == full_color[0].0, "{name}: 段 0 も同じ");
    }
}

#[test]
fn mips_after_painting_one_tile_match_a_full_rebuild_where_the_view_reads_them() {
    // 1024² に UV の [0.1, 0.49]² のアイランド 1 つ。タイルの 4 通り（縁をまたぐ・奥・外の塗り広げ・遠い）。ストロークの色は毎回違う
    let size = 1024;
    let mut h = view(900.0, 700.0, size);
    let mesh = quad_island(0.1, 0.49);
    let keep = keep_of(&mesh, size);
    paint_inside(&mut h, &keep);
    set_model(&mut h, vec![mesh]);
    look_at(&mut h, 2.0, 180.0, 0.0);
    let layer = first_layer(&h);
    check_mips_after_a_stroke_match_a_full_rebuild(
        &mut h,
        &[
            ("アイランドの縁をまたぐ", (0, 1), (true, true)),
            ("アイランドの奥", (1, 1), (true, false)),
            ("アイランドの外の塗り広げ", (4, 1), (false, true)),
            ("アイランドから遠い", (6, 6), (false, false)),
        ],
        size,
        &keep,
        0,
        layer,
        false,
    );
}

#[test]
fn mips_of_a_shrunk_picture_after_painting_one_tile_match_a_full_rebuild() {
    // 1024² を 1/4（256²）に縮めて持つ今のセット
    let size = 1024;
    let mut h = view(900.0, 700.0, size);
    let mesh = quad_island(0.55, 1.0);
    let keep = keep_of(&mesh, size);
    paint_inside(&mut h, &keep);
    h.state_mut().view3d_set_paint_budget(1 << 20);
    set_model(&mut h, vec![mesh]);
    look_at(&mut h, 2.0, 180.0, 0.0);
    assert_eq!(h.state().view3d_stats().unwrap().paint_level, 2);
    let layer = first_layer(&h);
    check_mips_after_a_stroke_match_a_full_rebuild(
        &mut h,
        &[
            ("アイランドの縁をまたぐ", (4, 4), (true, true)),
            ("アイランドの縁とキャンバスの端", (7, 4), (true, true)),
            ("アイランドの奥", (6, 6), (true, false)),
            ("アイランドの外の塗り広げ", (3, 6), (false, true)),
        ],
        size,
        &keep,
        2,
        layer,
        false,
    );
}

#[test]
fn mips_of_another_set_after_painting_one_tile_match_a_full_rebuild() {
    // ほかのセット（1024² を辺の上限 256 で 1/4 に縮めて持つ）
    let size = 1024;
    let mut h = two_sets(size, [quad_island(0.55, 1.0), quad_island(0.55, 1.0)]);
    h.state_mut().view3d_set_other_cap(size >> 2);
    let model = h.state().state.view3d.model.clone().unwrap();
    let keep = keep_of(&model.meshes[1], size);
    let layer = paint_inside_set(&mut h, 1, &keep);
    h.run();
    check_mips_after_a_stroke_match_a_full_rebuild(
        &mut h,
        &[
            ("アイランドの縁をまたぐ", (4, 4), (true, true)),
            ("アイランドの外の塗り広げ", (3, 6), (false, true)),
            ("アイランドの奥", (6, 6), (true, false)),
        ],
        size,
        &keep,
        2,
        layer,
        true,
    );
}

#[test]
fn mips_after_painting_a_whole_tile_match_a_full_rebuild_where_the_view_reads_them() {
    // タイル 1 枚を丸ごと白で塗る（粗い段のテクセルが大きく変わる）。重みが 0 のテクセルを粗い段の全部の読みで埋めると、重みが 0 のテクセルが
    // 何段も続く連鎖で、変わった範囲の外のテクセルまで値が変わり、タイルだけの更新と全部の作り直しが描画が読むテクセルでも食い違う
    let size = 1024;
    let mut h = view(900.0, 700.0, size);
    let mesh = quad_island(0.1, 0.49);
    let keep = keep_of(&mesh, size);
    paint_inside(&mut h, &keep);
    set_model(&mut h, vec![mesh]);
    look_at(&mut h, 2.0, 180.0, 0.0);
    let layer = first_layer(&h);
    let ts = h.state().state.doc.tile_size();
    for (name, (tx, ty)) in [("縁", (0u32, 1u32)), ("縁と角", (3, 3)), ("奥", (1, 1))] {
        h.state_mut().view3d_invalidate_paint();
        h.run();
        h.run();
        let tile = Rect::new(tx * ts, ty * ts, ts, ts);
        {
            let doc = &mut h.state_mut().state.doc;
            for y in tile.y..tile.y + ts {
                for x in tile.x..tile.x + ts {
                    for channel in [Channel::Color, Channel::Roughness] {
                        doc.set_channel_pixel(layer, channel, x, y, Rgba8::new(255, 255, 255, 255))
                            .unwrap();
                    }
                }
            }
        }
        h.run();
        let (partial_color, partial_roughness, weights) = (
            color_levels(&h, Slot::Color),
            color_levels(&h, Slot::Roughness),
            weight_levels(&h),
        );
        h.state_mut().view3d_invalidate_paint();
        h.run();
        h.run();
        let (full_color, full_roughness) = (
            color_levels(&h, Slot::Color),
            color_levels(&h, Slot::Roughness),
        );
        let color = differing_observable(&partial_color, &full_color, &weights, 4);
        let roughness = differing_observable(&partial_roughness, &full_roughness, &weights, 1);
        println!("{name}: 描画が読むテクセルで違う Color {color}・Roughness {roughness}");
        assert_eq!((color, roughness), (0, 0), "{name}");
    }
}

/// ミップ込みのバイト数（`channels` は 1 テクセルのバイト数）。
fn mip_total(size: u32, channels: u64) -> u64 {
    let levels = 32 - size.leading_zeros();
    (0..levels)
        .map(|l| ((size >> l).max(1) as u64).pow(2))
        .sum::<u64>()
        * channels
}

#[test]
fn the_weight_picture_counts_against_the_budget_and_goes_away_without_padding() {
    // 256² の Color・Roughness・Normal・Emission（1 テクセル 13 B）に、重みの絵の 1 B を足した分が予算に入る
    let size = 256;
    let mesh = quad_island(0.3, 0.7);
    let keep = keep_of(&mesh, size);
    let prepared = |budget: Option<u64>| {
        let mut h = view(900.0, 700.0, size);
        let layer = first_layer(&h);
        paint_with_outside_fill(&mut h.state_mut().state.doc, layer, &keep);
        if let Some(budget) = budget {
            h.state_mut().view3d_set_paint_budget(budget);
        }
        set_model(&mut h, vec![mesh.clone()]);
        look_at(&mut h, 2.0, 180.0, 0.0);
        h
    };
    let exact = mip_total(size, 14);
    let h = prepared(Some(exact));
    let s = h.state().view3d_stats().unwrap();
    assert_eq!(
        (s.paint_level, s.paint_by_budget, s.paint_bytes),
        (0, false, exact),
        "ちょうど入る予算では縮めない: {s:?}"
    );
    assert_eq!(
        weight_levels(&h)
            .iter()
            .map(|(t, _)| t.len() as u64)
            .sum::<u64>(),
        mip_total(size, 1),
        "重みの絵のバイト数"
    );
    let h = prepared(Some(exact - 1));
    let s = h.state().view3d_stats().unwrap();
    assert_eq!(
        (s.paint_level, s.paint_by_budget, s.paint_bytes),
        (1, true, mip_total(size / 2, 14)),
        "1 バイト足りないと 1 段縮める: {s:?}"
    );
    // 塗り広げない（幅 0）と、重みの絵は持たず、予算の数えも前のまま
    let mut h = prepared(None);
    h.state_mut().view3d_set_display_padding(0);
    h.state_mut().view3d_invalidate_paint();
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!(s.paint_bytes, mip_total(size, 13), "{s:?}");
    assert!(h.state().view3d_read_paint_weight_level(0).is_none());
    let mut h = prepared(Some(mip_total(size, 13)));
    h.state_mut().view3d_set_display_padding(0);
    h.state_mut().view3d_invalidate_paint();
    h.run();
    assert_eq!(h.state().view3d_stats().unwrap().paint_level, 0);
}

/// 計測: 4096² の 6 チャンネル・7 万三角形で、描いている最中の 1 フレームの同期の時間（`last_sync_us`）を、塗り広げなし・ありで
/// 交互に測る。
#[test]
#[ignore = "計測"]
fn measure_painting_frames_with_and_without_display_padding() {
    use std::time::Instant;
    let size = 4096;
    let mut h = view(1400.0, 900.0, size);
    let adapter = h.state().view3d_adapter().unwrap_or_default();
    let mesh = islands(77, 0.9);
    let started = Instant::now();
    let keep = keep_of(&mesh, size);
    println!(
        "GPU: {adapter}、覆い（4096²・71148 三角形）: {:.0} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
    let started = Instant::now();
    let rings = padding::Rings::new(size, size, &keep, DISPLAY_PAD_TEXELS).unwrap();
    println!(
        "段の地図（{DISPLAY_PAD_TEXELS} 段）: {:.0} ms、{} MiB",
        started.elapsed().as_secs_f64() * 1000.0,
        rings.bytes() >> 20
    );
    set_model(&mut h, vec![mesh]);
    look_at(&mut h, 1.6, 20.0, 10.0);
    let layer = first_layer(&h);
    let started = Instant::now();
    paint_inside(&mut h, &keep);
    h.state_mut()
        .state
        .doc
        .add_fill_layer(
            "値",
            &[
                (Channel::Metallic, Rgba8::new(60, 60, 60, 255)),
                (Channel::Emission, Rgba8::new(20, 10, 5, 255)),
                (Channel::Height, Rgba8::new(120, 120, 120, 255)),
            ],
            None,
        )
        .unwrap();
    h.step();
    h.state().view3d_wait_gpu();
    let s = h.state().view3d_stats().unwrap();
    println!(
        "初めての構築: {:.0} ms（同期 {:.0} ms）",
        started.elapsed().as_secs_f64() * 1000.0,
        s.last_sync_us as f64 / 1000.0
    );
    const ROUNDS: usize = 3;
    const FRAMES: u32 = 30;
    let mut counter = 0u32;
    let mut frames = |h: &mut Harness<'_, YoluApp>| {
        let mut sync = Vec::new();
        for _ in 0..FRAMES {
            counter += 1;
            let (x, y) = (200 + counter * 37 % 3500, 300 + counter * 53 % 3400);
            let doc = &mut h.state_mut().state.doc;
            for k in 0..2 {
                for (channel, v) in [
                    (Channel::Color, Rgba8::new(255, 128, 0, 255)),
                    (Channel::Roughness, Rgba8::new(10, 10, 10, 255)),
                ] {
                    doc.set_channel_pixel(layer, channel, x + k * 130, y, v)
                        .unwrap();
                }
            }
            h.step();
            h.state().view3d_wait_gpu();
            sync.push(h.state().view3d_stats().unwrap().last_sync_us as f64 / 1000.0);
        }
        sync.iter().sum::<f64>() / sync.len() as f64
    };
    // 幅ごとに交互に測る（0 は塗り広げない前の仕事）
    let widths = [0, 16, 32];
    let mut results: Vec<Vec<f64>> = vec![Vec::new(); widths.len()];
    for _ in 0..ROUNDS {
        for (i, &texels) in widths.iter().enumerate() {
            h.state_mut().view3d_set_display_padding(texels);
            h.state_mut().view3d_invalidate_paint();
            h.step();
            h.state().view3d_wait_gpu();
            results[i].push(frames(&mut h));
        }
    }
    for (texels, v) in widths.iter().zip(&results) {
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        let min = v.iter().copied().fold(f64::MAX, f64::min);
        let max = v.iter().copied().fold(f64::MIN, f64::max);
        println!(
            "描いている最中の 1 フレームの同期（Color・Roughness の 2 タイルずつ、{FRAMES} フレームの平均、{ROUNDS} 回）: 幅 {texels}: {mean:.2}（{min:.2}〜{max:.2}）ms"
        );
    }
}

/// 試しの立方体に、塗りつぶしレイヤー（白）の形のグラデーション（グラデーションデカール。球）を置き、メッシュマップを焼いた状態の 3D ビュー（文書は
/// `size`²）と、ギズモのつまみ（X の面）のあるポインタの位置。
fn shape_gradient_scene(size: u32) -> (Harness<'static, YoluApp>, egui::Pos2) {
    shape_gradient_scene_on(view(1400.0, 900.0, size))
}

/// `shape_gradient_scene` の、作った画面（`view` か `view_for_measure`）に置く版。
fn shape_gradient_scene_on(
    mut h: Harness<'static, YoluApp>,
) -> (Harness<'static, YoluApp>, egui::Pos2) {
    use yolu_app::bake::{BakeAction, BakeBackend};
    use yolu_app::fillfx::{gizmo, FillOp};
    use yolu_app::m2::Edit;
    use yolu_app::state::Action;
    use yolu_app::view3d::shape_gizmo::Handle;
    use yolu_core::generator::{Kind, MapState, Settings, Shape};
    use yolu_core::mesh_maps::MeshMapKind;
    use yolu_core::{EffectSettings, FilterSpec, FilterTarget, MapInput};
    {
        let s = &mut h.state_mut().state;
        s.bake.backend = BakeBackend::Cpu;
        s.apply(Action::LoadDemoModel);
        s.bake.settings.maps = vec![MeshMapKind::WorldNormal, MeshMapKind::Position];
        s.bake.settings.padding = 4;
        s.apply(Action::Bake(BakeAction::Start));
        s.wait_bake();
        let mut inputs = s.doc.effect_inputs().clone();
        for kind in [MeshMapKind::Position, MeshMapKind::WorldNormal] {
            let map = s
                .sets
                .current()
                .mesh_maps
                .get(kind)
                .expect("焼いたマップ")
                .clone();
            inputs = inputs
                .with_map(MapInput::from_baked(&map, MapState::Current).expect("マップ"))
                .expect("入力");
        }
        s.doc.set_effect_inputs(inputs).expect("入力を置く");
        s.apply(Action::M2(Edit::NewFill));
        let layer = s.selected_layer.expect("足したレイヤー");
        s.doc
            .set_fill_value(
                layer,
                Channel::Color,
                Some(Rgba8::new(255, 255, 255, 255)),
                false,
            )
            .unwrap();
        let mut settings = Settings::new(Kind::ShapeGradient);
        settings.volume.shape = Shape::Sphere;
        settings.volume.center = [-0.5, 0.5, 0.5];
        settings.volume.size = [0.9; 3];
        let filter = s
            .doc
            .add_filter(
                layer,
                FilterTarget::Content,
                FilterSpec::new(EffectSettings::generator(settings)).channels(&[Channel::Color]),
            )
            .unwrap();
        s.view3d.camera.yaw = -40.0;
        s.view3d.camera.pitch = 15.0;
        s.apply(Action::Fill(FillOp::EditFilter(Some((layer, filter)))));
    }
    h.run();
    h.step();
    h.state().view3d_wait_gpu();
    let rect = h.state().view3d_rect().unwrap();
    let from = gizmo::handle_point(&h.state().state, rect, Handle::SizeXPos).expect("つまみ");
    (h, from)
}

/// ポインタでつまみを押して、毎フレーム `wobble(frame)` だけ横へずらす。`frame` ごとに `each` を呼ぶ（step のあと）。押したまま返す。
fn drag_shape_handle(
    h: &mut Harness<'_, YoluApp>,
    from: egui::Pos2,
    frames: usize,
    mut each: impl FnMut(&mut Harness<'_, YoluApp>, usize),
) {
    press(h, from, egui::PointerButton::Primary);
    h.step();
    assert!(
        h.state().state.fillfx.drag.is_some(),
        "つまみのドラッグが始まった"
    );
    for frame in 0..frames {
        // 最初のフレームから動かす（動かさないと文書が変わらない）
        let wobble = 30.0 * ((frame + 1) as f32 * 0.35).sin();
        move_to(h, from + egui::vec2(wobble, 0.0));
        h.step();
        each(h, frame);
    }
}

#[test]
fn while_a_gizmo_drag_shows_coarse_tiles_the_mips_are_plain_and_the_release_builds_them_with_weights(
) {
    // ドラッグの間の絵は粗い仮の絵なので、ミップは重みを使わない今までの作り（全部のテクセルの箱の平均）。離して正確に上げ直したときに、
    // 重みつき（UV の上の画素だけ）で作り直し、全部を作り直したものと、描画が読むテクセルで同じになる
    let (mut h, from) = shape_gradient_scene(1024);
    let start = h.state().view3d_stats().unwrap();
    assert!(start.paint_weighted_mip_builds >= 1, "初めの構築は重みつき");
    assert_eq!(start.paint_coarse_mip_builds, 0);
    let mut previous = start;
    drag_shape_handle(&mut h, from, 8, |h, frame| {
        let s = h.state().view3d_stats().unwrap();
        assert!(
            s.paint_coarse_tiles > 0,
            "フレーム {frame}: 粗い絵を見せている"
        );
        assert_eq!(
            s.paint_weighted_mip_builds, start.paint_weighted_mip_builds,
            "フレーム {frame}: ドラッグの間は重みつきの道を通らない"
        );
        assert!(
            s.paint_coarse_mip_builds > previous.paint_coarse_mip_builds,
            "フレーム {frame}: 毎フレーム今までの作りで作る"
        );
        previous = s;
    });
    release(&h, from, egui::PointerButton::Primary);
    h.step();
    h.run();
    let after = h.state().view3d_stats().unwrap();
    assert_eq!(after.paint_coarse_tiles, 0, "正確に上げ直した");
    assert!(
        after.paint_weighted_mip_builds > start.paint_weighted_mip_builds,
        "離したら重みつきで作り直す"
    );
    assert_eq!(
        after.paint_coarse_mip_builds, previous.paint_coarse_mip_builds,
        "離したあとは今までの作りで作らない"
    );
    // 離したあとの段 1 以降は、全部を作り直したものと、描画が読むテクセルで同じ
    let weights = weight_levels(&h);
    let released = color_levels(&h, Slot::Color);
    h.state_mut().view3d_invalidate_paint();
    h.run();
    h.run();
    let full = color_levels(&h, Slot::Color);
    assert!(weights == weight_levels(&h));
    let differ = differing_observable(&released, &full, &weights, 4);
    println!("離したあとと全部の作り直しで違う、描画が読むテクセル: {differ}");
    assert_eq!(differ, 0);
    assert!(released[0].0 == full[0].0, "段 0 も同じ");
}

/// アイランドの中が黒・外が白のレイヤー（`keep` がアイランドの中）の上に、アイランドの外（右）のタイル（x 640〜767、y 256〜383）の中だけを薄く塗ったレイヤーを
/// 足し、そのレイヤーにぼかしを付ける。ぼかしの半径のドラッグで粗く上げるのは、そのタイルと隣のタイル（x 512〜895）だけで、塗り広げの届く所にアイランド
/// （`quad_island(0.1, 0.47)` なら x 481 まで、塗り広げて 497 まで）は無い。ぼかしを付けたレイヤーと、ぼかしの番号を返す。
fn add_blur_scene(
    state: &mut yolu_app::state::AppState,
    keep: &[bool],
) -> (LayerId, yolu_core::FilterId) {
    use yolu_core::{EffectSettings, FilterSpec, FilterTarget};
    let size = state.doc.width();
    let base = state.doc.layers()[0].id();
    import(&mut state.doc, base, Channel::Color, |x, y| {
        if keep[(y * size + x) as usize] {
            [0, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        }
    });
    state.apply(yolu_app::state::Action::NewLayer);
    let layer = state.selected_layer.expect("足したレイヤー");
    assert_ne!(layer, base);
    for y in 300..340 {
        for x in 700..740 {
            state
                .doc
                .set_channel_pixel(layer, Channel::Color, x, y, Rgba8::new(128, 128, 128, 255))
                .unwrap();
        }
    }
    let filter = state
        .doc
        .add_filter(
            layer,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(3)).channels(&[Channel::Color]),
        )
        .unwrap();
    (layer, filter)
}

#[test]
fn a_partial_coarse_drag_leaves_no_plain_mip_texels_after_the_release() {
    // `add_blur_scene` のぼかしの半径のドラッグで粗く上げるタイルの外の 1 テクセルまで、粗い絵のための全部のテクセルの箱の平均は段が上がると書き換え、その中に
    // アイランドの縁（外側の白を含む）が入る。離したあとの重みつきの作り直しが、その範囲も作り直して、外側の白が残らず、全部を作り直したものと、描画が読む
    // テクセルで同じになる
    use yolu_core::EffectSettings;
    let size = 1024;
    let mut h = view(900.0, 700.0, size);
    let mesh = quad_island(0.1, 0.47);
    let keep = keep_of(&mesh, size);
    let (layer, filter) = add_blur_scene(&mut h.state_mut().state, &keep);
    set_model(&mut h, vec![mesh]);
    look_at(&mut h, 2.0, 180.0, 0.0);
    let start = h.state().view3d_stats().unwrap();
    for radius in 4..12 {
        h.state_mut()
            .state
            .doc
            .set_filter_settings(layer, filter, EffectSettings::blur(radius), true)
            .unwrap();
        h.step();
        let s = h.state().view3d_stats().unwrap();
        assert!(
            s.paint_coarse_tiles > 0 && s.paint_coarse_tiles < 16,
            "ぼかしの影響の及ぶタイルだけ粗く上げる: {}",
            s.paint_coarse_tiles
        );
    }
    let mid = h.state().view3d_stats().unwrap();
    assert!(mid.paint_coarse_mip_builds > start.paint_coarse_mip_builds);
    assert_eq!(
        mid.paint_weighted_mip_builds,
        start.paint_weighted_mip_builds
    );
    h.state_mut().state.doc.end_coalescing();
    h.run();
    let after = h.state().view3d_stats().unwrap();
    assert_eq!(after.paint_coarse_tiles, 0);
    assert!(after.paint_weighted_mip_builds > mid.paint_weighted_mip_builds);
    let weights = weight_levels(&h);
    let released = color_levels(&h, Slot::Color);
    h.state_mut().view3d_invalidate_paint();
    h.run();
    h.run();
    let full = color_levels(&h, Slot::Color);
    let differ = differing_observable(&released, &full, &weights, 4);
    println!("離したあとと全部の作り直しで違う、描画が読むテクセル: {differ}");
    assert_eq!(differ, 0);
}

#[test]
fn switching_sets_during_a_coarse_drag_rebuilds_the_other_picture_instead_of_copying_it() {
    // 粗い絵を見せている間（重みを使わない作りで書き換えた段が残っている間）に今のセットが替わると、前のセットの絵は、段をコピーして縮めると、コピーした
    // 段 0 に外側の色が混ざった範囲が残る（離したあとの上げ直しは粗いタイルの範囲しか直さない）。コピーせず、文書から縮めて作り直す。
    // 対照: ドラッグしていないときは、今までどおりコピーで縮める
    use yolu_core::EffectSettings;
    let size = 1024;
    let mut h = two_sets(size, [quad_island(0.1, 0.47), quad_island(0.1, 0.47)]);
    h.state_mut().view3d_set_other_cap(size / 4);
    let model = h.state().state.view3d.model.clone().unwrap();
    let keep = keep_of(&model.meshes[0], size);
    let (layer, filter) = add_blur_scene(&mut h.state_mut().state, &keep);
    h.run();
    let switch = |h: &mut Harness<'_, YoluApp>, set: usize| {
        h.state_mut().state.switch_set(set).unwrap();
        h.run();
        h.run();
    };
    let demotions = |h: &Harness<'_, YoluApp>| h.state().view3d_stats().unwrap().other_demotions;
    let before = demotions(&h);
    switch(&mut h, 1);
    assert_eq!(
        demotions(&h),
        before + 1,
        "対照: ドラッグしていないときはコピーで縮める"
    );
    switch(&mut h, 0);
    let before = demotions(&h);
    for radius in 4..12 {
        h.state_mut()
            .state
            .doc
            .set_filter_settings(layer, filter, EffectSettings::blur(radius), true)
            .unwrap();
        h.step();
        assert!(h.state().view3d_stats().unwrap().paint_coarse_tiles > 0);
    }
    switch(&mut h, 1);
    assert_eq!(
        demotions(&h),
        before,
        "粗い絵を見せている間に替えたセットの絵は、コピーで縮めない"
    );
    assert_eq!(h.state().view3d_held_materials(), vec![0], "作り直して持つ");
    // 作り直した絵は、全部を作り直したものと同じ
    let rebuilt: Vec<_> = [Slot::Color, Slot::Roughness]
        .into_iter()
        .map(|slot| other_levels_of(&h, 0, slot))
        .collect();
    assert_eq!(rebuilt[0][0].1, [size / 4; 2]);
    h.state_mut().view3d_invalidate_paint();
    h.run();
    h.run();
    for (slot, got) in [Slot::Color, Slot::Roughness].into_iter().zip(&rebuilt) {
        assert!(
            &other_levels_of(&h, 0, slot) == got,
            "{slot:?}: 全部を作り直した絵と同じ"
        );
    }
}

/// 計測: ギズモのつまみのドラッグ（`shape_gradient_scene`）の 1 フレームの時間と、離した直後の 1 フレーム。文書の大きさは環境変数 `DRAG_SIZE`（既定 2048）、
/// フレーム数は `DRAG_FRAMES`（既定 44）。本物の GPU で測るときは、Vulkan を D3D12 の上で動かす dzn を選び（`VK_ICD_FILENAMES`・`LD_LIBRARY_PATH`・`WGPU_BACKEND=vulkan`）、
/// `WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER=1` も付ける。
/// `cargo test -p yolu-app --test gui_view3d measure_dragging_the_shape_gradient_gizmo -- --ignored --nocapture`
#[test]
#[ignore = "計測"]
fn measure_dragging_the_shape_gradient_gizmo_in_3d() {
    use std::time::Instant;
    let size: u32 = std::env::var("DRAG_SIZE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2048);
    let started = Instant::now();
    let (mut h, from) = shape_gradient_scene_on(view_for_measure(1400.0, 900.0, size));
    println!(
        "GPU: {}、文書 {size}²（ベイクと準備 {:.1} 秒）",
        h.state().view3d_adapter().unwrap_or_default(),
        started.elapsed().as_secs_f64()
    );
    let frames: usize = std::env::var("DRAG_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(44);
    // [塗った絵の同期（CPU）, prepare, step（GPU の実行を含む）, GPU の完了まで, 上げたタイル]
    let mut rows: Vec<[f64; 5]> = Vec::new();
    let mut coarse_seen = 0;
    let frame_time = |h: &mut Harness<'_, YoluApp>,
                      f: &mut dyn FnMut(&mut Harness<'_, YoluApp>)| {
        let t = Instant::now();
        f(h);
        let step_ms = t.elapsed().as_secs_f64() * 1000.0;
        h.state().view3d_wait_gpu();
        let total_ms = t.elapsed().as_secs_f64() * 1000.0;
        let st = h.state().view3d_stats().unwrap();
        (
            [
                st.last_sync_us as f64 / 1000.0,
                st.last_prepare_us as f64 / 1000.0,
                step_ms,
                total_ms,
                st.last_tiles as f64,
            ],
            st.paint_coarse_tiles,
        )
    };
    press(&h, from, egui::PointerButton::Primary);
    h.step();
    assert!(h.state().state.fillfx.drag.is_some());
    for frame in 0..frames {
        let wobble = 30.0 * ((frame + 1) as f32 * 0.35).sin();
        move_to(&h, from + egui::vec2(wobble, 0.0));
        let (row, coarse) = frame_time(&mut h, &mut |h| h.step());
        if frame >= 4 {
            coarse_seen += usize::from(coarse > 0);
            rows.push(row);
        }
    }
    let mean = |k: usize| rows.iter().map(|r| r[k]).sum::<f64>() / rows.len() as f64;
    println!(
        "ドラッグ 1 フレーム（{} フレーム、粗い絵を見せたフレーム {coarse_seen}）: 平均 上げたタイル {:.1}・塗った絵の同期 {:.2} ms・prepare {:.2} ms・step {:.1} ms・GPU の完了まで {:.1} ms（step のあとに待った {:.2} ms）",
        rows.len(),
        mean(4),
        mean(0),
        mean(1),
        mean(2),
        mean(3),
        mean(3) - mean(2)
    );
    release(&h, from, egui::PointerButton::Primary);
    let (row, coarse) = frame_time(&mut h, &mut |h| h.step());
    println!(
        "離した直後の 1 フレーム（粗い絵のタイル {coarse}）: 上げたタイル {:.0}・塗った絵の同期 {:.2} ms・prepare {:.2} ms・step {:.1} ms・GPU の完了まで {:.1} ms",
        row[4], row[0], row[1], row[2], row[3]
    );
}

/// 計測で動かす操作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragOp {
    /// 形のグラデーションの Generator のつまみ（ポインタで 3D ビューのギズモを動かす）。
    Gizmo,
    /// デカールの投影の箱の位置。
    Projection,
    /// レイヤーの不透明度。
    Opacity,
    /// 調整レイヤー（色相）の値。
    Adjust,
    /// ぼかしの半径。
    Blur,
    /// 点のグラデーションの点。
    Points,
}

impl DragOp {
    const ALL: [DragOp; 6] = [
        DragOp::Gizmo,
        DragOp::Projection,
        DragOp::Opacity,
        DragOp::Adjust,
        DragOp::Blur,
        DragOp::Points,
    ];

    fn name(self) -> &'static str {
        match self {
            DragOp::Gizmo => "gizmo",
            DragOp::Projection => "projection",
            DragOp::Opacity => "opacity",
            DragOp::Adjust => "adjust",
            DragOp::Blur => "blur",
            DragOp::Points => "points",
        }
    }
}

/// 計測の文書のレイヤー（`drag_scene`）。下から: 模様を読み込んだラスター（不透明度を動かす）、同じ模様にぼかしを掛けたラスター、
/// 色相の調整、デカールの画像の塗りつぶし、点のグラデーションの塗りつぶし、形のグラデーションの Generator を掛けた白の塗りつぶし。
struct DragScene {
    base: LayerId,
    blurred: LayerId,
    blur: yolu_core::FilterId,
    adjust: LayerId,
    decal: LayerId,
    points: LayerId,
    gradient: LayerId,
    filter: yolu_core::FilterId,
}

impl DragScene {
    /// 操作の対象のレイヤー（と土台の模様）だけを見せる。
    fn layers_for(&self, op: DragOp) -> Vec<LayerId> {
        let own = match op {
            DragOp::Gizmo => self.gradient,
            DragOp::Projection => self.decal,
            DragOp::Opacity => self.base,
            DragOp::Adjust => self.adjust,
            DragOp::Blur => self.blurred,
            DragOp::Points => self.points,
        };
        vec![self.base, own]
    }

    fn all(&self) -> [LayerId; 6] {
        [
            self.base,
            self.blurred,
            self.adjust,
            self.decal,
            self.points,
            self.gradient,
        ]
    }
}

/// `shape_gradient_scene` の文書に、操作ごとのレイヤーを足す（模様は文書の全面。画像は 1024² の sRGB）。
fn drag_scene(size: u32) -> (Harness<'static, YoluApp>, DragScene) {
    use yolu_core::fill_image::ProjectionMode;
    use yolu_core::fill_points::{GradientPoint, PointGradient, PointSpace};
    use yolu_core::{AdjustmentSettings, EffectSettings, FilterSpec, FilterTarget};
    let (mut h, _) = shape_gradient_scene_on(view_for_measure(1400.0, 900.0, size));
    let gradient = h
        .state()
        .state
        .selected_layer
        .expect("形のグラデーションのレイヤー");
    let (filter, _) = h
        .state()
        .state
        .fillfx
        .edit_filter
        .map(|(l, f)| (f, l))
        .expect("Generator");
    let pattern = |x: u32, y: u32| -> [u8; 4] {
        let c = ((x / 37 + y / 53) % 7) as u8;
        [40 + c * 30, 200 - c * 20, 60 + ((x ^ y) & 63) as u8, 255]
    };
    let base = first_layer(&h);
    let scene = {
        let s = &mut h.state_mut().state;
        import(&mut s.doc, base, Channel::Color, pattern);
        let blurred = s.doc.add_layer("ぼかし").unwrap();
        import(&mut s.doc, blurred, Channel::Color, pattern);
        let blur = s
            .doc
            .add_filter(
                blurred,
                FilterTarget::Content,
                FilterSpec::new(EffectSettings::blur(6)).channels(&[Channel::Color]),
            )
            .unwrap();
        let adjust = s
            .doc
            .add_adjustment_layer(
                "色相",
                AdjustmentSettings::hue_saturation(20.0, 0.0, 0.0).unwrap(),
                Some(&[Channel::Color]),
                Some(blurred),
            )
            .unwrap();
        // デカール: 1024² の模様の画像を、カメラから見て正面に貼る
        let image_id = yolu_core::ImageId(0xD8A6_0000_0000_0000_0000_0000_0000_0001);
        let n = 1024u32;
        let pixels: Vec<u8> = (0..n * n)
            .flat_map(|i| {
                let (x, y) = (i % n, i / n);
                let on = ((x / 64) + (y / 64)) % 2 == 0;
                if on {
                    [230, 80, 40, 255]
                } else {
                    [40, 90, 220, 255]
                }
            })
            .collect();
        let image =
            yolu_core::ImageInput::new(n, n, pixels, yolu_core::ImageColorSpace::Srgb).unwrap();
        let inputs = s.doc.effect_inputs().clone().with_image(image_id, image);
        s.doc.set_effect_inputs(inputs).unwrap();
        let decal = s
            .doc
            .add_fill_layer(
                "デカール",
                &[(Channel::Color, Rgba8::new(255, 255, 255, 255))],
                Some(adjust),
            )
            .unwrap();
        s.doc
            .set_fill_image(decal, Channel::Color, Some(image_id))
            .unwrap();
        let mut projection = *s.doc.layer(decal).unwrap().projection();
        projection.mode = ProjectionMode::Decal;
        projection.placement = s
            .fitted_placement_for(ProjectionMode::Decal, Some((n, n)))
            .expect("モデルがある");
        s.doc.set_fill_projection(decal, projection, false).unwrap();
        // 点のグラデーション: モデルの外形の中の 3 点
        let bounds = s.model_bounds().expect("モデルがある");
        let at = |k: f32| {
            let p = bounds.center + bounds.extents * Vec3::new(k, -k * 0.5, 0.3);
            [p.x as f64, p.y as f64, p.z as f64]
        };
        let points_layer = s
            .doc
            .add_fill_layer(
                "点",
                &[(Channel::Color, Rgba8::new(255, 255, 255, 255))],
                Some(decal),
            )
            .unwrap();
        s.doc
            .set_fill_points(
                points_layer,
                Channel::Color,
                Some(PointGradient {
                    space: PointSpace::Model,
                    spread: 0.3,
                    points: vec![
                        GradientPoint {
                            position: at(-0.6),
                            color: Rgba8::new(250, 40, 40, 255),
                        },
                        GradientPoint {
                            position: at(0.0),
                            color: Rgba8::new(40, 250, 40, 255),
                        },
                        GradientPoint {
                            position: at(0.6),
                            color: Rgba8::new(40, 40, 250, 255),
                        },
                    ],
                }),
                false,
            )
            .unwrap();
        // 形のグラデーションのレイヤーをいちばん上へ（ギズモの対象は選んだレイヤー）
        let top = s.doc.layers().len() - 1;
        let index = s
            .doc
            .layers()
            .iter()
            .position(|l| l.id() == gradient)
            .unwrap();
        if index != top {
            s.doc.move_layer(gradient, top).unwrap();
        }
        DragScene {
            base,
            blurred,
            blur,
            adjust,
            decal,
            points: points_layer,
            gradient,
            filter,
        }
    };
    h.run();
    (h, scene)
}

/// ドックの並び: 3D ビューだけ（3D ビューを前に）か、キャンバスと 3D ビューを左右に並べる。左はレイヤー。
/// プロパティのタブは置かない: プロパティはポインタを押していないフレームで変更のまとめを終える（スライダーを離した扱い）。この計測は
/// 文書の口で値を変え、ポインタを押さないので、プロパティがあると、それより後に描くビュー（並べたときの 3D ビュー）がまとめの終わった文書を見る。
fn set_drag_layout(h: &mut Harness<'static, YoluApp>, both: bool) {
    use egui_dock::{DockState, NodeIndex};
    use yolu_app::Tab;
    let mut dock = if both {
        DockState::new(vec![Tab::Canvas])
    } else {
        DockState::new(vec![Tab::Canvas, Tab::View3d])
    };
    let surface = dock.main_surface_mut();
    let [center, _] = surface.split_left(NodeIndex::root(), 0.18, vec![Tab::Layers]);
    if both {
        surface.split_right(center, 0.5, vec![Tab::View3d]);
    }
    h.state_mut().dock = dock;
    h.run();
    if !both {
        click_tab(h, Tab::View3d);
    }
    h.run();
}

/// 計測のための一時の内訳（コミットしない細工が入っているときだけ値を返す）。
fn drag_breakdown() -> Vec<(&'static str, f64)> {
    Vec::new()
}

/// 計測: 3D ビューで見ているときの、ドラッグの 1 フレーム（文書の変更・step・GPU の完了まで）を、操作ごと・並びごとに（3D だけ／キャンバスと
/// 並べる）。離した直後の 1 フレームも。文書の大きさ `DRAG_SIZE`（既定 2048）、平均を取るフレーム数 `DRAG_FRAMES`（既定 16）、回数
/// `DRAG_ROUNDS`（既定 2。操作と並びを交互に回して、ほかの負荷のぶれを見る）、操作 `DRAG_OPS`（gizmo,projection,opacity,adjust,blur,points の
/// どれか。カンマ区切り）、並び `DRAG_LAYOUTS`（3d,both）。2D のキャンバスの合成は `DRAG_CANVAS`（auto・gpu・cpu。既定は実際のアプリの既定と
/// 同じ自動: ソフトウェアの GPU では CPU、そうでなければ GPU の常駐の合成）。2D が GPU の常駐の合成のときは、そのフレームの 2D の上げ（タイルの数と
/// MiB）も出す。GPU は wgpu の環境変数（`WGPU_BACKEND` など）で選ぶ。
/// `cargo test -p yolu-app --test gui_view3d measure_drag_frames_by_operation -- --ignored --nocapture`
#[test]
#[ignore = "計測"]
fn measure_drag_frames_by_operation() {
    use std::time::Instant;
    use yolu_app::fillfx::gizmo;
    use yolu_app::view3d::shape_gizmo::Handle;
    use yolu_core::EffectSettings;
    let var = |name: &str| std::env::var(name).ok();
    let size: u32 = var("DRAG_SIZE")
        .and_then(|v| v.parse().ok())
        .unwrap_or(2048);
    let frames: usize = var("DRAG_FRAMES")
        .and_then(|v| v.parse().ok())
        .unwrap_or(16);
    let rounds: usize = var("DRAG_ROUNDS").and_then(|v| v.parse().ok()).unwrap_or(2);
    let ops: Vec<DragOp> = match var("DRAG_OPS") {
        Some(list) => DragOp::ALL
            .into_iter()
            .filter(|op| list.split(',').any(|n| n.trim() == op.name()))
            .collect(),
        None => DragOp::ALL.to_vec(),
    };
    let layouts: Vec<bool> = match var("DRAG_LAYOUTS") {
        Some(list) => list
            .split(',')
            .filter_map(|n| match n.trim() {
                "3d" => Some(false),
                "both" => Some(true),
                _ => None,
            })
            .collect(),
        None => vec![false, true],
    };
    let started = Instant::now();
    let (mut h, scene) = drag_scene(size);
    h.state_mut()
        .set_canvas_backend(yolu_app::canvas::gpu::CanvasBackend::parse(
            var("DRAG_CANVAS").as_deref(),
        ));
    println!(
        "GPU: {}、文書 {size}²（ベイクと準備 {:.1} 秒）、{frames} フレームの平均",
        h.state().view3d_adapter().unwrap_or_default(),
        started.elapsed().as_secs_f64()
    );
    const WARM: usize = 3;
    let mut run = 0usize;
    for round in 0..rounds {
        for &both in &layouts {
            set_drag_layout(&mut h, both);
            for &op in &ops {
                // 対象のレイヤーだけを見せて、落ち着くまで回す
                {
                    let s = &mut h.state_mut().state;
                    let shown = scene.layers_for(op);
                    for id in scene.all() {
                        s.doc.set_layer_visible(id, shown.contains(&id)).unwrap();
                    }
                    s.selected_layer = Some(if op == DragOp::Gizmo {
                        scene.gradient
                    } else {
                        scene.base
                    });
                    s.fillfx.edit_filter =
                        (op == DragOp::Gizmo).then_some((scene.gradient, scene.filter));
                    // 前の回の評価のキャッシュを使わない（同じ値の繰り返しで、粗い絵の道を通らなくなる）
                    s.doc.release_effect_cache();
                }
                run += 1;
                h.run();
                h.step();
                h.state().view3d_wait_gpu();
                let canvas = if both {
                    // CPU なら理由も（GPU を試して落ちたのか、方針か）
                    match h.state().display().fallback() {
                        Some(why) => format!("Cpu({why:?})"),
                        None => format!("{:?}", h.state().display().shown()),
                    }
                } else {
                    "-".into()
                };
                let rect = h.state().view3d_rect().expect("3D ビュー");
                let from = gizmo::handle_point(&h.state().state, rect, Handle::SizeXPos);
                if op == DragOp::Gizmo {
                    let from = from.expect("つまみ");
                    press(&h, from, egui::PointerButton::Primary);
                    h.step();
                    assert!(h.state().state.fillfx.drag.is_some(), "つまみを掴んだ");
                }
                let originals = {
                    let doc = &h.state().state.doc;
                    (
                        *doc.layer(scene.decal).unwrap().projection(),
                        doc.layer(scene.points)
                            .unwrap()
                            .fill_points(Channel::Color)
                            .unwrap()
                            .clone(),
                    )
                };
                let mut rows: Vec<Vec<f64>> = Vec::new();
                let mut coarse_frames = 0;
                // step のあとに変更のまとめが切れていたフレーム（操作がまとめのままのはずなのに、どこかが終えた）
                let mut ended = 0;
                for frame in 0..frames + WARM {
                    let wobble = ((frame + 1) as f64 * 0.35 + run as f64 * 0.7).sin();
                    let t = Instant::now();
                    {
                        let doc = &mut h.state_mut().state.doc;
                        match op {
                            DragOp::Gizmo => {
                                move_to(&h, from.unwrap() + egui::vec2(30.0 * wobble as f32, 0.0))
                            }
                            DragOp::Projection => {
                                let mut p = originals.0;
                                p.placement.center[0] += 0.05 * wobble;
                                doc.set_fill_projection(scene.decal, p, true).unwrap();
                            }
                            DragOp::Opacity => doc
                                .set_layer_opacity(scene.base, 0.6 + 0.3 * wobble, true)
                                .unwrap(),
                            DragOp::Adjust => doc
                                .set_adjustment(
                                    scene.adjust,
                                    yolu_core::AdjustmentSettings::hue_saturation(
                                        60.0 * wobble,
                                        0.0,
                                        0.0,
                                    )
                                    .unwrap(),
                                    true,
                                )
                                .unwrap(),
                            DragOp::Blur => doc
                                .set_filter_settings(
                                    scene.blurred,
                                    scene.blur,
                                    EffectSettings::blur(4 + ((frame + run) % 8) as u32),
                                    true,
                                )
                                .unwrap(),
                            DragOp::Points => {
                                let mut g = originals.1.clone();
                                g.points[1].position[0] += 0.1 * wobble;
                                doc.set_fill_points(scene.points, Channel::Color, Some(g), true)
                                    .unwrap();
                            }
                        }
                    }
                    let edit_ms = t.elapsed().as_secs_f64() * 1000.0;
                    h.step();
                    let step_ms = t.elapsed().as_secs_f64() * 1000.0;
                    h.state().view3d_wait_gpu();
                    let total_ms = t.elapsed().as_secs_f64() * 1000.0;
                    let st = h.state().view3d_stats().unwrap();
                    let breakdown = drag_breakdown();
                    // 2D の GPU の常駐の合成の、このフレームの上げ（GPU でなければ 0）
                    let canvas_upload =
                        match (h.state().display().shown(), h.state().display().gpu().last) {
                            (yolu_app::canvas::gpu::Shown::Gpu, Some(u)) => (
                                u.uploaded_tiles as f64,
                                u.uploaded_bytes as f64 / (1u64 << 20) as f64,
                            ),
                            _ => (0.0, 0.0),
                        };
                    if frame < WARM {
                        continue;
                    }
                    coarse_frames += usize::from(st.paint_coarse_tiles > 0);
                    ended += usize::from(!h.state().state.doc.is_coalescing());
                    let mut row = vec![
                        edit_ms,
                        step_ms - edit_ms,
                        total_ms - step_ms,
                        total_ms,
                        st.last_sync_us as f64 / 1000.0,
                        st.last_prepare_us as f64 / 1000.0,
                        st.last_tiles as f64,
                        canvas_upload.0,
                        canvas_upload.1,
                    ];
                    row.extend(breakdown.iter().map(|(_, v)| *v));
                    rows.push(row);
                }
                let mean = |k: usize| rows.iter().map(|r| r[k]).sum::<f64>() / rows.len() as f64;
                let names: Vec<&str> = drag_breakdown_names();
                let extra: String = names
                    .iter()
                    .enumerate()
                    .map(|(i, n)| format!(" {n} {:.2}", mean(9 + i)))
                    .collect();
                println!(
                    "[{round}] {:<10} {:<4} 2D={canvas:<4} 粗 {coarse_frames}/{} 切れ {ended}: 合計 {:.1} ms（変更 {:.2}・step {:.1}・GPU 待ち {:.1}）同期 {:.1} prepare {:.1} タイル {:.1} 2D の上げ {:.0} タイル・{:.1} MiB{extra}",
                    op.name(),
                    if both { "both" } else { "3d" },
                    rows.len(),
                    mean(3),
                    mean(0),
                    mean(1),
                    mean(2),
                    mean(4),
                    mean(5),
                    mean(6),
                    mean(7),
                    mean(8),
                );
                // 離した直後の 1 フレーム
                let _ = drag_breakdown();
                let t = Instant::now();
                if op == DragOp::Gizmo {
                    release(&h, from.unwrap(), egui::PointerButton::Primary);
                } else {
                    h.state_mut().state.doc.end_coalescing();
                }
                h.step();
                let step_ms = t.elapsed().as_secs_f64() * 1000.0;
                h.state().view3d_wait_gpu();
                let total_ms = t.elapsed().as_secs_f64() * 1000.0;
                let st = h.state().view3d_stats().unwrap();
                let breakdown = drag_breakdown();
                let extra: String = breakdown
                    .iter()
                    .map(|(n, v)| format!(" {n} {v:.2}"))
                    .collect();
                println!(
                    "[{round}] {:<10} {:<4} 2D={canvas:<4} 離した直後: 合計 {total_ms:.1} ms（step {step_ms:.1}）同期 {:.1} prepare {:.1} タイル {}{extra}",
                    op.name(),
                    if both { "both" } else { "3d" },
                    st.last_sync_us as f64 / 1000.0,
                    st.last_prepare_us as f64 / 1000.0,
                    st.last_tiles,
                );
                assert_eq!(st.paint_coarse_tiles, 0, "離したら正確に上げ直す");
                h.run();
            }
        }
    }
}

/// `drag_breakdown` の項目の名前（細工が無ければ空）。
fn drag_breakdown_names() -> Vec<&'static str> {
    drag_breakdown().into_iter().map(|(n, _)| n).collect()
}

/// 計測: この GPU での上げ方ごとの時間（2048² の RGBA8 = 16 MiB を、`write_texture` の 1 回・128² のタイルごと、`write_buffer` のタイルごと
/// （作ってある・毎回作る入れ物へ）・1 回で大きな入れ物へ上げて GPU の中でタイルへ写す）。数は「呼びの CPU の時間 / GPU の完了まで」。
/// GPU の選び方は `measure_dragging_the_shape_gradient_gizmo_in_3d` と同じ環境変数。
#[test]
#[ignore = "計測"]
fn measure_upload_calls_on_this_gpu() {
    use eframe::egui_wgpu::wgpu;
    use std::time::Instant;
    let rs = egui_kittest::wgpu::create_render_state(
        egui_kittest::wgpu::default_wgpu_setup(),
        common::render_options(),
    );
    let info = rs.adapter.get_info();
    println!(
        "GPU: {} ({:?}, {:?})",
        info.name, info.backend, info.device_type
    );
    let (device, queue) = (&rs.device, &rs.queue);
    let (size, ts) = (2048u32, 128u32);
    let tiles = (size / ts) * (size / ts);
    let tile_bytes = (ts * ts * 4) as u64;
    let data: Vec<u8> = (0..size * size * 4)
        .map(|i| ((i * 31) >> 3) as u8)
        .collect();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let buffer = |bytes: u64| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        })
    };
    let kept: Vec<wgpu::Buffer> = (0..tiles).map(|_| buffer(tile_bytes)).collect();
    let big = buffer(tile_bytes * tiles as u64);
    let tile_data = |k: u32| {
        let (tx, ty) = (k % (size / ts), k / (size / ts));
        let mut out = Vec::with_capacity(tile_bytes as usize);
        for y in 0..ts {
            let at = (((ty * ts + y) * size + tx * ts) * 4) as usize;
            out.extend_from_slice(&data[at..at + (ts * 4) as usize]);
        }
        out
    };
    let packed: Vec<Vec<u8>> = (0..tiles).map(tile_data).collect();
    let flat: Vec<u8> = packed.concat();
    let finish = |t: Instant| {
        let cpu = t.elapsed().as_secs_f64() * 1000.0;
        queue.submit(std::iter::empty());
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        (cpu, t.elapsed().as_secs_f64() * 1000.0)
    };
    let layout = |w: u32, h: u32| wgpu::TexelCopyBufferLayout {
        offset: 0,
        bytes_per_row: Some(w * 4),
        rows_per_image: Some(h),
    };
    // 写しの帯（`StagingBelt`）: 対応付けたままの入れ物を回して使う（初めの回だけ作る）
    let mut belt = wgpu::util::StagingBelt::new(device.clone(), tile_bytes * tiles as u64);
    for round in 0..4 {
        let t = Instant::now();
        let mut encoder = device.create_command_encoder(&Default::default());
        for k in 0..tiles {
            belt.write_buffer(
                &mut encoder,
                &kept[k as usize],
                0,
                std::num::NonZeroU64::new(tile_bytes).unwrap(),
            )
            .copy_from_slice(&packed[k as usize]);
        }
        belt.finish();
        queue.submit(Some(encoder.finish()));
        let belt_tiles = finish(t);
        belt.recall();
        let t = Instant::now();
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let slice = belt.allocate(
                std::num::NonZeroU64::new(data.len() as u64).unwrap(),
                std::num::NonZeroU64::new(256).unwrap(),
            );
            slice
                .get_mapped_range_mut()
                .expect("対応付けた")
                .copy_from_slice(&data);
            encoder.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer: slice.buffer(),
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: slice.offset(),
                        bytes_per_row: Some(size * 4),
                        rows_per_image: Some(size),
                    },
                },
                texture.as_image_copy(),
                texture.size(),
            );
        }
        belt.finish();
        queue.submit(Some(encoder.finish()));
        let belt_texture = finish(t);
        belt.recall();
        println!(
            "[{round}] 写しの帯: タイルごと {tiles} 回 {:.1}/{:.1} ms・テクスチャへ 1 回 {:.1}/{:.1} ms",
            belt_tiles.0, belt_tiles.1, belt_texture.0, belt_texture.1
        );
    }
    for round in 0..3 {
        let t = Instant::now();
        queue.write_texture(
            texture.as_image_copy(),
            &data,
            layout(size, size),
            texture.size(),
        );
        let whole = finish(t);
        let t = Instant::now();
        for k in 0..tiles {
            let (tx, ty) = (k % (size / ts), k / (size / ts));
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: tx * ts,
                        y: ty * ts,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &packed[k as usize],
                layout(ts, ts),
                wgpu::Extent3d {
                    width: ts,
                    height: ts,
                    depth_or_array_layers: 1,
                },
            );
        }
        let per_tile_texture = finish(t);
        let t = Instant::now();
        for k in 0..tiles {
            queue.write_buffer(&kept[k as usize], 0, &packed[k as usize]);
        }
        let per_tile_buffer = finish(t);
        let t = Instant::now();
        let fresh: Vec<wgpu::Buffer> = (0..tiles)
            .map(|k| {
                let b = buffer(tile_bytes);
                queue.write_buffer(&b, 0, &packed[k as usize]);
                b
            })
            .collect();
        let fresh_buffers = finish(t);
        drop(fresh);
        let t = Instant::now();
        queue.write_buffer(&big, 0, &flat);
        let mut encoder = device.create_command_encoder(&Default::default());
        for k in 0..tiles {
            encoder.copy_buffer_to_buffer(
                &big,
                k as u64 * tile_bytes,
                &kept[k as usize],
                0,
                tile_bytes,
            );
        }
        queue.submit(Some(encoder.finish()));
        let one_buffer = finish(t);
        let t = Instant::now();
        queue.submit(std::iter::empty());
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let empty = t.elapsed().as_secs_f64() * 1000.0;
        println!(
            "[{round}] 16 MiB: write_texture 1 回 {:.1}/{:.1} ms・タイルごと {tiles} 回 {:.1}/{:.1} ms、write_buffer タイルごと（作ってある）{:.1}/{:.1} ms・\
             （毎回作る）{:.1}/{:.1} ms・1 回で大きな入れ物へ＋GPU の中で写す {:.1}/{:.1} ms、空の submit と待ち {empty:.2} ms",
            whole.0, whole.1, per_tile_texture.0, per_tile_texture.1, per_tile_buffer.0, per_tile_buffer.1,
            fresh_buffers.0, fresh_buffers.1, one_buffer.0, one_buffer.1
        );
    }
}

/// 計測: 塗り広げの幅ごとの、離れて見たときの継ぎ目のにじみ（アイランドの中だけを塗った球と、全面を塗った球の差）。
#[test]
#[ignore = "計測"]
fn measure_seams_by_padding_width_and_distance() {
    let size = 2048;
    let mut h = view(800.0, 600.0, size);
    let mesh = islands(32, 0.6);
    let keep = keep_of(&mesh, size);
    set_model(&mut h, vec![mesh]);
    println!("| 幅 | 距離 | 差が 8 を超える画素 | そのうち球の内側 | 最大 |");
    println!("|---:|---:|---:|---:|---:|");
    for texels in [0, 2, 4, 8, 16, 32, 64] {
        h.state_mut().view3d_set_display_padding(texels);
        for distance in [3.0, 6.0, 12.0, 24.0] {
            h.state_mut().view3d_invalidate_paint();
            let (inside, full) = seam_shots(&mut h, &keep, distance);
            let (count, max) = seam_count(&inside, &full);
            let interior = interior_seams(&inside, &full);
            println!("| {texels} | {distance} | {count} | {interior} | {max} |");
            // 絵を見たいときは PADDING_SEAM_DIR に書く（差 × 4 の絵も）
            if let Some(dir) = std::env::var_os("PADDING_SEAM_DIR") {
                let dir = std::path::PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                inside
                    .save(dir.join(format!("inside_{texels}_{distance}.png")))
                    .unwrap();
                let mut d = image::RgbaImage::new(inside.width(), inside.height());
                for ((a, b), o) in inside.pixels().zip(full.pixels()).zip(d.pixels_mut()) {
                    for k in 0..3 {
                        o.0[k] = (a.0[k].abs_diff(b.0[k]) as u32 * 4).min(255) as u8;
                    }
                    o.0[3] = 255;
                }
                d.save(dir.join(format!("diff_{texels}_{distance}.png")))
                    .unwrap();
            }
        }
    }
}

/// 計測: ほかのセット（4096²・7 万三角形、辺の上限 1024 で 1/4 に縮めて持つ）の文書が 1 タイル変わったときの同期の時間
/// （`last_sync_us`。今のセットの同期と、ほかのセットの同期の全部）を、塗り広げなし・ありで交互に測る。
#[test]
#[ignore = "計測"]
fn measure_syncing_another_set_with_and_without_display_padding() {
    use std::time::Instant;
    let size = 4096;
    let shift = 2;
    let mut h = two_sets(size, [islands(77, 0.9), islands(77, 0.9)]);
    let adapter = h.state().view3d_adapter().unwrap_or_default();
    h.state_mut().view3d_set_other_cap(size >> shift);
    let model = h.state().state.view3d.model.clone().unwrap();
    let started = Instant::now();
    let keep = keep_of(&model.meshes[1], size);
    println!(
        "GPU: {adapter}、覆い（{size}²・{} 三角形）: {:.0} ms",
        model.meshes[1].submeshes[0].indices.len() / 3,
        started.elapsed().as_secs_f64() * 1000.0
    );
    let started = Instant::now();
    let rings = padding::Rings::new(size, size, &keep, reach_for(shift)).unwrap();
    println!(
        "段の地図（{} 段）: {:.0} ms、{} MiB",
        reach_for(shift),
        started.elapsed().as_secs_f64() * 1000.0,
        rings.bytes() >> 20
    );
    let started = Instant::now();
    let mut pixels: Vec<u8> = (0..size * size * 4)
        .map(|i| (i.wrapping_mul(2654435761) >> 24) as u8)
        .collect();
    println!(
        "（試験の画素の用意: {:.0} ms）",
        started.elapsed().as_secs_f64() * 1000.0
    );
    let started = Instant::now();
    let bounds = Rect::new(0, 0, size, size);
    rings.dilate_region(&mut pixels, bounds, bounds).unwrap();
    println!(
        "全面の塗り広げ（`dilate_region`、{} 段）: {:.0} ms",
        reach_for(shift),
        started.elapsed().as_secs_f64() * 1000.0
    );
    drop(pixels);
    let layer = paint_inside_set(&mut h, 1, &keep);
    h.step();
    h.state().view3d_wait_gpu();
    const ROUNDS: usize = 3;
    const CHANGES: u32 = 10;
    let mut counter = 0u32;
    let mut results: Vec<Vec<f64>> = vec![Vec::new(); 2];
    let mut first_build = [(0.0f64, 0.0f64); 2];
    for round in 0..ROUNDS {
        for (i, texels) in [0, DISPLAY_PAD_TEXELS].into_iter().enumerate() {
            h.state_mut().view3d_set_display_padding(texels);
            h.state_mut().view3d_invalidate_paint();
            // 今のセットだけを作るフレームと、そのあとにほかのセットだけを作るフレームを分けて測る
            h.state_mut().view3d_set_show_other_sets(false);
            h.step();
            h.state().view3d_wait_gpu();
            let current_build = h.state().view3d_stats().unwrap().last_sync_us as f64 / 1000.0;
            h.state_mut().view3d_set_show_other_sets(true);
            h.step();
            h.state().view3d_wait_gpu();
            let s = h.state().view3d_stats().unwrap();
            assert_eq!(s.other_sets, 1, "{s:?}");
            if round == 0 {
                first_build[i] = (current_build, s.last_sync_us as f64 / 1000.0);
            }
            let mut sync = Vec::new();
            for _ in 0..CHANGES {
                counter += 1;
                let (tx, ty) = (4 + counter % 12, 4 + counter * 5 % 12);
                let ts = h.state().state.set_doc(1).tile_size();
                touch(
                    h.state_mut().state.set_doc_mut(1),
                    layer,
                    Rect::new(tx * ts, ty * ts, ts, ts),
                    // 毎回違う値にする（同じ値を描いても文書は変わらず、同期が起きない）
                    &[
                        (Channel::Color, Rgba8::new(counter as u8, 128, 0, 255)),
                        (
                            Channel::Roughness,
                            Rgba8::new((counter * 3) as u8, 10, 10, 255),
                        ),
                    ],
                );
                h.step();
                h.state().view3d_wait_gpu();
                sync.push(h.state().view3d_stats().unwrap().last_sync_us as f64 / 1000.0);
            }
            println!("  幅 {texels}: {sync:.1?}");
            results[i].push(sync.iter().sum::<f64>() / sync.len() as f64);
        }
    }
    for (texels, (current, other)) in [0, DISPLAY_PAD_TEXELS].iter().zip(&first_build) {
        println!(
            "初めて作るフレームの同期（幅 {texels}）: 今のセット（空の文書）{current:.0} ms・ほかのセット {other:.0} ms"
        );
    }
    for (texels, v) in [0, DISPLAY_PAD_TEXELS].iter().zip(&results) {
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        let min = v.iter().copied().fold(f64::MAX, f64::min);
        let max = v.iter().copied().fold(f64::MIN, f64::max);
        println!(
            "ほかのセットの文書が 1 タイル変わった 1 フレームの同期（{CHANGES} 回の平均、{ROUNDS} 回）: 幅 {texels}: {mean:.1}（{min:.1}〜{max:.1}）ms"
        );
    }
}
