//! テンプレートの書き出しの試験（画面なし）: 名前・複数のセット・パディング・AO・上書きの確かめ・取消・読むだけのセット。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use yolu_core::mesh_maps::MeshMapKind;
use yolu_core::{Channel, Document, LayerId, TileCoord};
use yolu_protocol::{MaterialInfo, MaterialKey, MeshData, Model, Submesh};

use super::*;
use crate::bake::BakeAction;
use crate::state::Action;

/// 試験用の一時フォルダ（終わると消す）。
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "yolu-app-export-{name}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Dir(p)
    }

    fn files(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(&self.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 文書の左半分とその境目の 1 列（x <= width / 2）を不透明の色で塗る（UV が左半分のセットの絵。パディングの覆いは UV に触れる
/// テクセルまで含むので、境目の列も塗る）。
fn paint_left_half(doc: &mut Document, layer: LayerId, channel: Channel, rgba: [u8; 4]) {
    let ts = doc.tile_size();
    let (w, h) = (doc.width(), doc.height());
    doc.set_channel_enabled(layer, channel, true).unwrap();
    for ty in 0..h.div_ceil(ts) {
        for tx in 0..(w / 2 + 1).div_ceil(ts) {
            let mut tile = vec![0u8; (ts * ts * 4) as usize];
            for y in 0..ts.min(h - ty * ts) {
                for x in 0..ts.min(w / 2 + 1 - tx * ts) {
                    tile[((y * ts + x) * 4) as usize..][..4].copy_from_slice(&rgba);
                }
            }
            doc.import_tile(layer, channel, TileCoord::new(tx, ty), &tile)
                .unwrap();
        }
    }
}

fn info(name: &str) -> MaterialInfo {
    MaterialInfo {
        key: MaterialKey::Material {
            name: name.into(),
            asset: None,
        },
        shader: String::new(),
        textures: vec![],
        routes: vec![],
    }
}

/// 2 枚の板（Skin は UV の左半分、Hair は右半分。どちらも文書の左半分・右半分を覆う）。
fn two_quads() -> Model {
    let quad = |x: f32, u0: f32, material: u32| MeshData {
        key: format!("{x}"),
        name: format!("板{x}"),
        skinned: false,
        positions: vec![
            [x, 0.0, 0.0],
            [x + 1.0, 0.0, 0.0],
            [x, 1.0, 0.0],
            [x + 1.0, 1.0, 0.0],
        ],
        normals: vec![],
        uv0: vec![[u0, 0.0], [u0 + 0.5, 0.0], [u0, 1.0], [u0 + 0.5, 1.0]],
        submeshes: vec![Submesh {
            material,
            indices: vec![0, 1, 2, 2, 1, 3],
        }],
    };
    Model {
        generation: 1,
        name: "二枚".into(),
        materials: vec![info("Skin"), info("Hair")],
        meshes: vec![quad(0.0, 0.0, 0), quad(2.0, 0.5, 1)],
    }
}

/// 2 つのセット（Skin・Hair）を持つ 64 × 64 の状態。Skin の左半分は赤、Hair の左半分は緑で塗ってある。
fn two_sets() -> AppState {
    let mut s = AppState::new(64, 64);
    let (_, shape) = s.receive_link_model(&two_quads(), 0);
    assert_eq!(shape, Ok(()));
    assert_eq!(s.sets.len(), 2);
    let layer = s.selected_layer.unwrap();
    paint_left_half(&mut s.doc, layer, Channel::Color, [255, 0, 0, 255]);
    // 作られたセットの文書は 256 以上なので、同じ 64 × 64 の文書に替える
    let other = 1 - s.sets.current_index();
    let (mut fresh, first) = crate::state::blank_document(64, 64);
    paint_left_half(&mut fresh, first.unwrap(), Channel::Color, [0, 255, 0, 255]);
    *s.set_doc_mut(other) = fresh;
    s
}

fn export(s: &mut AppState, id: &str, dir: &Path) {
    s.apply(Action::Export(ExportAction::TemplateTo {
        id: id.into(),
        dir: dir.to_path_buf(),
    }));
}

/// PNG を文書の向き（下の行が先）の RGBA8 にする。
fn load_png(path: &Path) -> (u32, u32, Vec<u8>) {
    let image = image::open(path).unwrap().to_rgba8();
    let (w, h) = image.dimensions();
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in (0..h).rev() {
        for x in 0..w {
            out.extend_from_slice(&image.get_pixel(x, y).0);
        }
    }
    (w, h, out)
}

fn px(img: &(u32, u32, Vec<u8>), x: u32, y: u32) -> [u8; 4] {
    let i = ((y * img.0 + x) * 4) as usize;
    [img.2[i], img.2[i + 1], img.2[i + 2], img.2[i + 3]]
}

#[test]
fn the_menu_only_asks_for_the_folder_and_unknown_templates_are_refused() {
    let mut s = two_sets();
    s.apply(Action::Export(ExportAction::Template("unity-hdrp".into())));
    assert_eq!(
        s.dialog_request,
        Some(DialogRequest::ExportFolder("unity-hdrp".into()))
    );
    s.dialog_request = None;
    s.apply(Action::Export(ExportAction::Template("nothing".into())));
    assert_eq!(s.dialog_request, None);
    assert!(s.message.contains("テンプレート"), "{}", s.message);
}

#[test]
fn writes_every_set_with_the_unity_names_and_pads_outside_the_uvs() {
    let dir = Dir::new("pad");
    let mut s = two_sets();
    export(&mut s, "unity-standard", &dir.0);
    assert!(s.export.is_exporting(), "別のスレッドで書く");
    assert!(s.export.confirm.is_none());
    s.wait_export();
    // セットが複数なので _<セット名>。読むものが無い画像（法線・ハイト・AO など）は書かない
    assert_eq!(
        dir.files(),
        ["Texture_Hair_Albedo.png", "Texture_Skin_Albedo.png"]
    );
    let skin = load_png(&dir.0.join("Texture_Skin_Albedo.png"));
    assert_eq!((skin.0, skin.1), (64, 64));
    assert_eq!(px(&skin, 10, 30), [255, 0, 0, 255], "UV の中の絵");
    // UV の外（右半分）は、境目の色で塗り広げる（既定は届くかぎり全部）
    assert_eq!(px(&skin, 60, 30), [255, 0, 0, 255], "パディング");
    let hair = load_png(&dir.0.join("Texture_Hair_Albedo.png"));
    assert_eq!(px(&hair, 10, 30), [0, 255, 0, 255]);
    // 結果の一覧
    let report = s.export.report.as_ref().unwrap();
    assert_eq!(report.images.len(), 2);
    assert!(report
        .images
        .iter()
        .all(|i| i.srgb && !i.normal_map && !i.replaced));
    assert!(s.message.contains("書き出しました"), "{}", s.message);
    // 文書は変わらない
    assert!(!s.doc.can_undo());
}

#[test]
fn padding_off_leaves_the_outside_of_the_uvs_empty_and_no_model_says_so() {
    let dir = Dir::new("nopad");
    let mut s = two_sets();
    s.export.padding = 0;
    export(&mut s, "unity-standard", &dir.0);
    s.wait_export();
    let skin = load_png(&dir.0.join("Texture_Skin_Albedo.png"));
    assert_eq!(px(&skin, 60, 30)[3], 0, "塗り広げない");
    assert!(s.export.report.as_ref().unwrap().notes.is_empty());

    // モデルが無い（閉じた）: パディングは掛けず、短く知らせる
    let dir = Dir::new("nomodel");
    let mut s = two_sets();
    s.close_link_model(1);
    s.export.padding = -1;
    export(&mut s, "unity-standard", &dir.0);
    s.wait_export();
    let report = s.export.report.as_ref().unwrap();
    assert_eq!(report.notes, [Note::NoModel]);
    assert!(s.message.contains("塗り広げなし"), "{}", s.message);
    let skin = load_png(&dir.0.join("Texture_Skin_Albedo.png"));
    assert_eq!(px(&skin, 60, 30)[3], 0);
    // 英語
    s.lang = crate::lang::Lang::En;
    assert!(note_text(s.lang, &Note::NoModel).starts_with("No padding"));
}

#[test]
fn a_single_set_has_no_set_name_in_the_file_names() {
    let dir = Dir::new("single");
    let mut s = AppState::new(64, 64);
    let layer = s.selected_layer.unwrap();
    paint_left_half(&mut s.doc, layer, Channel::Color, [10, 20, 30, 255]);
    // 金属度を塗ると MetallicSmoothness も書く
    paint_left_half(&mut s.doc, layer, Channel::Metallic, [255, 255, 255, 255]);
    export(&mut s, "unity-standard", &dir.0);
    s.wait_export();
    assert_eq!(
        dir.files(),
        ["Texture_Albedo.png", "Texture_MetallicSmoothness.png"]
    );
    let img = load_png(&dir.0.join("Texture_MetallicSmoothness.png"));
    assert_eq!(px(&img, 10, 30)[0], 255, "R は Metallic");
    assert_eq!(
        px(&img, 10, 30)[3],
        128,
        "A は Smoothness（Roughness を塗っていないので既定の 0.5）"
    );
    // HDRP は同じ文書から BaseColor と MaskMap
    let hdrp = Dir::new("hdrp");
    export(&mut s, "unity-hdrp", &hdrp.0);
    s.wait_export();
    assert_eq!(
        hdrp.files(),
        ["Texture_BaseColor.png", "Texture_MaskMap.png"]
    );
    // 開いたプロジェクトが無ければ Texture
    assert_eq!(stem(&s), "Texture");
}

#[test]
fn names_that_become_the_same_file_write_nothing() {
    let dir = Dir::new("clash");
    let mut s = two_sets();
    let (a, b) = (s.sets.get(0).unwrap().uid, s.sets.get(1).unwrap().uid);
    s.rename_set(a, "a/b").unwrap();
    s.rename_set(b, "a_b").unwrap();
    export(&mut s, "unity-standard", &dir.0);
    assert!(!s.export.is_exporting());
    assert!(s.message.contains("同じファイル"), "{}", s.message);
    assert!(dir.files().is_empty());
}

#[test]
fn existing_files_are_asked_about_first_and_cancel_keeps_them() {
    let dir = Dir::new("replace");
    let mut s = two_sets();
    export(&mut s, "unity-standard", &dir.0);
    s.wait_export();
    let original = std::fs::read(dir.0.join("Texture_Skin_Albedo.png")).unwrap();
    // 絵を変えてもう一度: もうあるファイルを確かめる（まだ何も書かない）
    let layer = s.selected_layer.unwrap();
    paint_left_half(&mut s.doc, layer, Channel::Color, [0, 0, 255, 255]);
    export(&mut s, "unity-standard", &dir.0);
    assert!(!s.export.is_exporting());
    let confirm = s.export.confirm.clone().unwrap();
    assert_eq!(confirm.total, 2);
    assert_eq!(
        confirm.existing,
        ["Texture_Skin_Albedo.png", "Texture_Hair_Albedo.png"],
        "書く順（セットの並びの順）"
    );
    s.apply(Action::Export(ExportAction::CancelConfirm));
    assert!(s.export.confirm.is_none());
    assert_eq!(
        std::fs::read(dir.0.join("Texture_Skin_Albedo.png")).unwrap(),
        original,
        "やめたら元のファイルのまま"
    );
    // 置き換える
    export(&mut s, "unity-standard", &dir.0);
    s.apply(Action::Export(ExportAction::ConfirmReplace));
    assert!(s.export.is_exporting());
    s.wait_export();
    let img = load_png(&dir.0.join("Texture_Skin_Albedo.png"));
    assert_eq!(s.sets.current().name, "Skin");
    assert_eq!(px(&img, 10, 30), [0, 0, 255, 255], "新しい絵に置き換わった");
    assert!(s
        .export
        .report
        .as_ref()
        .unwrap()
        .images
        .iter()
        .all(|i| i.replaced));
    // 一時ファイルは残らない
    assert!(
        dir.files().iter().all(|f| f.ends_with(".png")),
        "{:?}",
        dir.files()
    );
}

#[test]
fn cancel_writes_nothing_and_leaves_no_temp_files() {
    let dir = Dir::new("cancel");
    let mut s = AppState::new(256, 256);
    let layer = s.selected_layer.unwrap();
    paint_left_half(&mut s.doc, layer, Channel::Color, [1, 2, 3, 255]);
    paint_left_half(&mut s.doc, layer, Channel::Emission, [9, 9, 9, 255]);
    paint_left_half(&mut s.doc, layer, Channel::Metallic, [255, 255, 255, 255]);
    // 取消が来るまで始めない仕事にする（取消が効いたことを、書き終わる速さに頼らず確かめる）
    s.export.park_next = true;
    export(&mut s, "unity-standard", &dir.0);
    assert!(s.export.is_exporting());
    let progress = s.export.progress().unwrap();
    assert_eq!(progress.total, 3);
    s.apply(Action::Export(ExportAction::Cancel));
    assert!(s.export.progress().unwrap().canceling);
    s.wait_export();
    assert!(s.message.contains("取り消しました"), "{}", s.message);
    assert!(dir.files().is_empty(), "{:?}", dir.files());
    assert!(s.export.report.is_none());
    // 取り消したあとは、また書き出せる
    export(&mut s, "unity-standard", &dir.0);
    s.wait_export();
    assert_eq!(dir.files().len(), 3, "{:?}", dir.files());
}

#[test]
fn an_export_the_working_budget_cannot_hold_is_refused_with_the_reason_and_writes_nothing() {
    let dir = Dir::new("budget");
    // 画像そのもの（4 バイト × 画素）は 8192 × 8192 でも予算（512 MiB）に収まるが、塗り広げは 1 画素に 12 バイト要る。
    // 7000 × 7000 は 588 MB で、予算を超える（塗り広げの前に断る。文書は空なので文書の側の確保は小さい）
    let mut s = AppState::new(7000, 7000);
    s.apply(Action::LoadDemoModel);
    s.modified = false;
    export(&mut s, "unity-standard", &dir.0);
    assert!(s.export.is_exporting());
    s.wait_export();
    assert!(s.message.contains("書き出せません"), "{}", s.message);
    assert!(s.message.contains("予算"), "{}", s.message);
    assert!(dir.files().is_empty(), "何も書かない: {:?}", dir.files());
    assert!(s.export.report.is_none());
    assert!(!s.export.is_exporting());
    // 塗り広げなしなら、同じ文書が予算に収まる（画像は 196 MB）。断る理由は塗り広げの予算だった
    s.export.padding = 0;
    s.lang = crate::lang::Lang::En;
    export(&mut s, "unity-standard", &dir.0);
    s.wait_export();
    assert!(s.message.starts_with("Exported"), "{}", s.message);
    assert_eq!(dir.files().len(), 1, "{:?}", dir.files());
}

/// 床と壁（同じマテリアル。壁の上の辺を `lean` だけ倒せる）。
fn corner_model(lean: f32) -> Model {
    let plate = |positions: [[f32; 3]; 4], u0: f32| MeshData {
        key: format!("{u0}"),
        name: format!("板{u0}"),
        skinned: false,
        positions: positions.to_vec(),
        normals: vec![],
        uv0: vec![[u0, 0.0], [u0 + 0.5, 0.0], [u0, 1.0], [u0 + 0.5, 1.0]],
        submeshes: vec![Submesh {
            material: 0,
            indices: vec![0, 1, 2, 2, 1, 3],
        }],
    };
    Model {
        generation: 1,
        name: "角".into(),
        materials: vec![info("Skin")],
        meshes: vec![
            plate(
                [
                    [0.0, 0.0, 0.0],
                    [1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0],
                    [1.0, 0.0, 1.0],
                ],
                0.0,
            ),
            plate(
                [
                    [0.0, 0.0, 0.0],
                    [1.0, 0.0, 0.0],
                    [0.0, 1.0, lean],
                    [1.0, 1.0, lean],
                ],
                0.5,
            ),
        ],
    }
}

#[test]
fn the_baked_ao_fills_the_occlusion_image_and_a_stale_one_is_left_out_with_a_note() {
    let dir = Dir::new("ao");
    let mut s = AppState::new(64, 64);
    let (_, shape) = s.receive_link_model(&corner_model(0.0), 0);
    assert_eq!(shape, Ok(()));
    let layer = s.selected_layer.unwrap();
    paint_left_half(&mut s.doc, layer, Channel::Color, [200, 100, 50, 255]);
    s.bake.settings.maps = vec![MeshMapKind::AmbientOcclusion];
    s.bake.settings.ao_samples = 32;
    s.bake.settings.padding = 4;
    s.apply(Action::Bake(BakeAction::Start));
    s.wait_bake();
    assert!(!s.sets.current().mesh_maps.is_empty());
    export(&mut s, "unity-standard", &dir.0);
    s.wait_export();
    assert_eq!(dir.files(), ["Texture_Albedo.png", "Texture_Occlusion.png"]);
    let ao = load_png(&dir.0.join("Texture_Occlusion.png"));
    assert!(
        ao.2.as_chunks::<4>().0.iter().any(|p| p[0] < 255),
        "凹む角に遮蔽が出る"
    );
    assert!(ao
        .2
        .as_chunks::<4>()
        .0
        .iter()
        .all(|p| p[0] == p[1] && p[1] == p[2] && p[3] == 255));
    assert!(s.export.report.as_ref().unwrap().notes.is_empty());

    // ポーズで形が変わると古い: AO の画像は書かず、知らせる
    let dir = Dir::new("ao-stale");
    s.receive_link_pose(&yolu_protocol::Pose {
        generation: 1,
        meshes: vec![yolu_protocol::MeshPose {
            mesh: 1,
            positions: corner_model(0.4).meshes[1].positions.clone(),
            normals: vec![],
        }],
    })
    .unwrap();
    export(&mut s, "unity-standard", &dir.0);
    s.wait_export();
    assert_eq!(dir.files(), ["Texture_Albedo.png"]);
    let notes = &s.export.report.as_ref().unwrap().notes;
    assert!(
        notes
            .iter()
            .any(|n| matches!(n, Note::StaleOcclusion(set, _) if set == "Skin")),
        "{notes:?}"
    );
}

#[test]
fn a_read_only_set_is_not_exported_and_nothing_to_write_is_said() {
    let dir = Dir::new("readonly");
    let mut s = two_sets();
    s.sets.get_mut(1).unwrap().read_only = Some("試験".into());
    let skip = s.sets.get(1).unwrap().name.clone();
    export(&mut s, "unity-standard", &dir.0);
    s.wait_export();
    assert_eq!(dir.files().len(), 1, "{:?}", dir.files());
    assert_eq!(
        s.export.report.as_ref().unwrap().notes,
        [Note::ReadOnly(skip)]
    );
    // どのレイヤーも読むチャンネルを使っていなければ、書くものが無い
    let dir = Dir::new("nothing");
    let mut s = AppState::new(64, 64);
    let layer = s.selected_layer.unwrap();
    s.doc
        .set_channel_enabled(layer, Channel::Color, false)
        .unwrap();
    export(&mut s, "unity-hdrp", &dir.0);
    assert!(!s.export.is_exporting());
    assert!(
        s.message.contains("書き出すものがありません"),
        "{}",
        s.message
    );
    assert!(dir.files().is_empty());
}

#[test]
fn it_does_not_start_while_drawing_or_exporting() {
    let dir = Dir::new("busy");
    let mut s = two_sets();
    let layer = s.selected_layer.unwrap();
    let brush = s.stroke_settings(false);
    let stroke = s.doc.begin_stroke(layer, &brush).unwrap();
    export(&mut s, "unity-standard", &dir.0);
    assert!(!s.export.is_exporting());
    assert_eq!(s.message, "描いている間はできません。");
    s.apply(Action::Export(ExportAction::Template("unity-hdrp".into())));
    assert_eq!(s.dialog_request, None);
    s.doc.end_stroke(stroke).unwrap();
    export(&mut s, "unity-standard", &dir.0);
    assert!(s.export.is_exporting());
    export(&mut s, "unity-hdrp", &dir.0);
    assert!(s.message.contains("書き出し中"), "{}", s.message);
    s.wait_export();
    assert_eq!(dir.files().len(), 2, "{:?}", dir.files());
}

#[test]
fn the_set_materials_uv_triangles_scale_to_the_document() {
    let model = crate::view3d::model::ViewModel::from_live_link(&two_quads(), 1).unwrap();
    let tris = uv_triangles(&model, 1, 64, 32);
    assert_eq!(tris.len(), 2, "Hair の板の 2 つの三角形");
    assert!(tris.iter().flatten().all(|p| p.x >= 32.0 && p.y <= 32.0));
    assert!(uv_triangles(&model, 5, 64, 32).is_empty());
}
