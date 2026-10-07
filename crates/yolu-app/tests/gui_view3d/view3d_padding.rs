//! 3D ビューの表示の写しの塗り広げ（UV の外へ。`view3d::paint` の `set_padding`）: 島の中は正本と同じバイト・島の外は書き出しと同じ式で
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

/// 立方体を膨らませた球（面ごとに 3 × 2 の UV の島）の島を、島の中心のまわりに `scale` 倍へ縮めたもの（島の外の隙間を広げる）。
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
    let mut h = app(width, height, doc);
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

/// 層のチャンネルに、`at` が返す画素を丸ごと読み込む（行は下から。タイルごと）。
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

/// 島の中（覆うテクセル）だけを塗った文書: Color は島ごとに違う色（島の外は透明）、Roughness は島の中だけ 200。
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
    // 島の中は正本（合成）を塗り広げずに上げたのと同じバイト
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
            assert_eq!(a, b, "島の中のテクセル {i}");
        } else if b[3] == 0 && a[3] > 0 {
            filled += 1;
        }
    }
    assert!(filled > 0, "島の外が塗り広がっている");
    // 全体が、書き出しと同じ式（`padding::dilate`、同じ段数）で塗り広げたものと同じ
    assert!(color == expected_color(doc, &keep), "Color の段 0");
    assert!(
        level0(&h, Slot::Roughness) == expected_scalar(doc, Channel::Roughness, &keep),
        "Roughness の段 0"
    );
    // 届く幅の外（島から DISPLAY_PAD_TEXELS より遠い所）は元のまま（透明）
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
    // 1024² の文書（128² のタイル）に、UV の [0.1, 0.49]² の島 1 つ。タイルは 4 通りになる: 島の縁をまたぐ・島の奥（塗り広げる
    // テクセルが届く幅の外）・島の外で塗り広げるテクセルがある・島から遠い
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
        ("島の縁をまたぐ", tile(0, 1), (true, true)),
        ("島の奥", tile(1, 1), (true, false)),
        ("島の外の塗り広げ", tile(4, 1), (false, true)),
        ("島から遠い", tile(6, 6), (false, false)),
    ];
    for (name, t, (has_keep, near)) in cases {
        assert_eq!(rings.kinds_in(t).0, has_keep, "{name}: 島の中を含む");
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

/// タイル（`size` の画布の中）の中に島の中のテクセルがあるか・塗り広げるテクセルが届く所にあるか（アプリと同じ見方: 島の中を含むなら、
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

/// (名前, タイルの番号, 島の中を含むか, 塗り広げるテクセルが届くか)
type Cases<'a> = &'a [(&'a str, (u32, u32), (bool, bool))];

/// 縮めて持つ今のセット（予算で縮める）: 1 タイルを描いたあとの段 0 が、文書の解像度で塗り広げてから縮めたものと同じ（塗り広げの幅は
/// 文書の画素で `16 << shift`、段の地図の上限 254 で頭打ち）。島は UV の右上に寄せて、画布の端で切れる場合と、塗り広げの幅が縮めの境
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
            ("島の縁をまたぐ", (4, 4), (true, true)),
            ("島の縁と画布の端", (7, 4), (true, true)),
            ("島の奥", (6, 6), (true, false)),
            ("島の外の塗り広げ", (3, 6), (false, true)),
            ("島から遠い", (0, 0), (false, false)),
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
            ("島の縁をまたぐ", (8, 8), (true, true)),
            ("島の縁と画布の端", (15, 8), (true, true)),
            ("島の奥", (12, 12), (true, false)),
            ("島の外の塗り広げ", (6, 12), (false, true)),
            ("島から遠い", (0, 0), (false, false)),
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

/// 文書 `set` の最初の層に、島の中だけを塗った Color と Roughness（`paint_inside` の、今のセット以外の文書向け）。
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
        ("島の縁をまたぐ", (4, 4), (true, true)),
        ("島の縁と画布の端", (7, 4), (true, true)),
        ("島の外の塗り広げ", (3, 6), (false, true)),
        ("島の奥", (6, 6), (true, false)),
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
/// ものと同じ（Sobel のために外へ 1 画素広げた出力から切り出して塗り広げる道）。島は UV の左下に寄せて、画布の端で切れる場合も通す。
fn check_normal_from_height(edges: HeightEdgeMode, shift: u32, budget: Option<u64>) {
    let size = 1024;
    let mut h = view(900.0, 700.0, size);
    let mesh = quad_island(0.0, 0.49);
    let keep = keep_of(&mesh, size);
    let layer = first_layer(&h);
    {
        let doc = &mut h.state_mut().state.doc;
        // 島の中だけに、緩い起伏の Height（島の外は透明）
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
        ("島の縁をまたぐ", (3, 3), (true, true)),
        ("島の縁と画布の端", (3, 0), (true, true)),
        ("島の奥", (1, 1), (true, false)),
        ("島の外の塗り広げ", (4, 1), (false, true)),
        ("島から遠い", (6, 6), (false, false)),
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
    // UV が違うモデル: 覆いを作り直し、その覆いで塗り広げる（前の島の外の塗り広げは残らない）
    let b = islands(8, 0.8);
    let keep_b = keep_of(&b, size);
    assert_ne!(keep_a, keep_b);
    set_model(&mut h, vec![b]);
    assert!(h.state().view3d_stats().unwrap().last_rebuilt);
    assert!(level0(&h, Slot::Color) == expected_color(&h.state().state.doc, &keep_b));
    // 塗り広げを切れば、前と同じ（島の外は透明のまま）
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

/// 島の中だけを塗った球と、全面を塗った球を、同じカメラで撮る。
fn seam_shots(
    h: &mut Harness<'_, YoluApp>,
    keep: &[bool],
    distance: f32,
) -> (image::RgbaImage, image::RgbaImage) {
    let layer = first_layer(h);
    let size = h.state().state.doc.width();
    let colored = |x: u32, y: u32, keep: &[bool]| {
        if keep[(y * size + x) as usize] {
            [230, 120, 60, 255]
        } else {
            [0, 0, 0, 0]
        }
    };
    let shot = |h: &mut Harness<'_, YoluApp>| {
        look_at(h, distance, 35.0, 25.0);
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
    let inside = shot(h);
    import(
        &mut h.state_mut().state.doc,
        layer,
        Channel::Color,
        |_, _| [230, 120, 60, 255],
    );
    h.run();
    let full = shot(h);
    (inside, full)
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
    // 球が表示域の高さの約 1/4（島の 1 テクセルが画面の 1/4 画素ほど: ミップの段 2〜3）
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
    assert!(before > 50, "塗り広げないと島の縁がにじむ（{before}）");
    assert_eq!(after, 0, "塗り広げると島の縁がにじまない");
    // 島の中だけを塗った絵（塗り広げあり）
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

/// 計測: 塗り広げの幅ごとの、離れて見たときの継ぎ目のにじみ（島の中だけを塗った球と、全面を塗った球の差）。
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
