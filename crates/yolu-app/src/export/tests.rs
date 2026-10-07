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
    s.bake.backend = crate::bake::BakeBackend::Cpu;
    let (_, shape) = s.receive_link_model(&two_quads());
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
    assert!(s.message.contains("塗り広げていません"), "{}", s.message);
    let skin = load_png(&dir.0.join("Texture_Skin_Albedo.png"));
    assert_eq!(px(&skin, 60, 30)[3], 0);
    // 英語
    s.lang = crate::lang::Lang::En;
    assert!(note_text(s.lang, &Note::NoModel).starts_with("Not padded"));
}

#[test]
fn a_single_set_has_no_set_name_in_the_file_names() {
    let dir = Dir::new("single");
    let mut s = AppState::new(64, 64);
    s.bake.backend = crate::bake::BakeBackend::Cpu;
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
    s.bake.backend = crate::bake::BakeBackend::Cpu;
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
    s.bake.backend = crate::bake::BakeBackend::Cpu;
    // 自動の予算は物理メモリから決まる（8 GB で 512 MiB。1/16）。機械によらないように 8 GB とする
    s.prefs.ram_mib = 8192;
    assert_eq!(s.export_working_bytes(), 512 * 1024 * 1024);
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

/// 設定の「1 回の操作」の予算が、書き出しの作業メモリになる（固定の値ではない）: 小さくすると大きな画像の塗り広げを断り、上げれば書ける。
#[test]
fn the_one_operation_budget_in_the_settings_is_the_exports_working_memory() {
    use crate::prefs::{Pref, PrefsAction};
    use crate::settings::{Budget, BudgetKind};
    let dir = Dir::new("budget-setting");
    // 2048 × 2048 の塗り広げは 1 画素に 12 バイト（およそ 48 MiB）要る
    let mut s = AppState::new(2048, 2048);
    s.bake.backend = crate::bake::BakeBackend::Cpu;
    s.prefs.ram_mib = 16384;
    s.apply(Action::LoadDemoModel);
    s.modified = false;
    s.apply(Action::Prefs(PrefsAction::Set(Pref::Budget(
        BudgetKind::Stroke,
        Budget::Mib(8),
    ))));
    assert_eq!(s.export_working_bytes(), 8 * 1024 * 1024);
    export(&mut s, "unity-standard", &dir.0);
    s.wait_export();
    assert!(
        s.message.contains("書き出せません") && s.message.contains("予算"),
        "{}",
        s.message
    );
    assert!(dir.files().is_empty(), "何も書かない: {:?}", dir.files());
    // 設定を上げれば、同じ書き出しが通る
    s.apply(Action::Prefs(PrefsAction::Set(Pref::Budget(
        BudgetKind::Stroke,
        Budget::Mib(512),
    ))));
    assert_eq!(s.export_working_bytes(), 512 * 1024 * 1024);
    export(&mut s, "unity-standard", &dir.0);
    s.wait_export();
    assert_eq!(dir.files().len(), 1, "{} {:?}", s.message, dir.files());
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
    s.bake.backend = crate::bake::BakeBackend::Cpu;
    let (_, shape) = s.receive_link_model(&corner_model(0.0));
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
    s.bake.backend = crate::bake::BakeBackend::Cpu;
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

/// 書き出した画像にも、画面と同じ効果が入る（正本は効果の入力を持たないので、写した文書へ入力を渡し直す）。読むマップが無くて効いていない効果は、
/// 黙って入力のまま書かず、書き出した画像に入っていないことを注意に出す。
#[test]
fn an_exported_image_has_the_generators_the_screen_shows_and_an_inactive_one_is_named_in_the_notes()
{
    use crate::fx::FxOp;
    use yolu_core::generator::Kind;
    use yolu_core::FilterTarget;

    let mut s = AppState::new(64, 64);
    s.bake.backend = crate::bake::BakeBackend::Cpu;
    s.apply(Action::LoadDemoModel);
    s.bake.settings.maps = vec![
        MeshMapKind::WorldNormal,
        MeshMapKind::Position,
        MeshMapKind::AmbientOcclusion,
        MeshMapKind::Curvature,
    ];
    s.bake.settings.ao_samples = 8;
    s.bake.settings.padding = 4;
    s.export.padding = 0; // 塗り広げの覆い（モデルの UV）を使わない
                          // 黒の塗りつぶしの層のマスクへ、焼いた曲率から値を作る Generator（見える所だけを残す）
    s.apply(Action::M2(crate::m2::Edit::NewFill));
    let layer = s.selected_layer.unwrap();
    s.apply(Action::M2(crate::m2::Edit::AddMask(layer)));
    s.apply(Action::Fx(FxOp::AddGenerator {
        target: FilterTarget::Mask,
        kind: Kind::EdgeWear,
    }));

    // 焼く前: 効かないので、書いた画像は全面が見え、効かない効果を注意に出す
    let before = Dir::new("generator-before");
    export(&mut s, "unity-standard", &before.0);
    s.wait_export();
    let report = s.export.report.as_ref().expect("書き出した");
    assert!(
        report
            .notes
            .iter()
            .any(|n| matches!(n, Note::InactiveEffects(_, effects) if effects.len() == 1)),
        "{:?}",
        report.notes
    );
    assert!(
        s.message
            .contains("効いていない効果 1 件は書き出しに入っていません"),
        "{}",
        s.message
    );
    let png = load_png(&before.0.join("Texture_Albedo.png"));
    assert!(
        (0..64).all(|y| (0..64).all(|x| px(&png, x, y)[3] == 255)),
        "入力のまま通る"
    );

    // 焼いた後: 書いた画像の透明（マスクが隠した所）が、画面の合成と同じ
    s.apply(Action::Bake(BakeAction::Start));
    s.wait_bake();
    s.sync_effects();
    assert!(s.doc.inactive_effect_list().is_empty());
    let screen = s.doc.composite(s.doc.bounds()).unwrap();
    let hidden = screen
        .iter()
        .skip(3)
        .step_by(4)
        .filter(|a| **a != 255)
        .count();
    assert!(hidden > 0, "焼いたマップのジェネレーターが見える所を絞る");
    let after = Dir::new("generator-after");
    export(&mut s, "unity-standard", &after.0);
    s.wait_export();
    assert!(
        !s.export
            .report
            .as_ref()
            .unwrap()
            .notes
            .iter()
            .any(|n| matches!(n, Note::InactiveEffects(..))),
        "効く効果は注意に出さない"
    );
    let png = load_png(&after.0.join("Texture_Albedo.png"));
    for y in 0..64u32 {
        for x in 0..64u32 {
            let expected = screen[((y * 64 + x) * 4 + 3) as usize];
            assert_eq!(px(&png, x, y)[3], expected, "({x}, {y}): 画面の合成と同じ");
        }
    }
}

// ───────── チャンネルの画像（描くチャンネルの PNG・全チャンネル） ─────────

fn export_channels(s: &mut AppState, dir: &Path) {
    s.apply(Action::Export(ExportAction::ChannelsTo(dir.to_path_buf())));
}

fn export_channel(s: &mut AppState, path: &Path) {
    s.apply(Action::Export(ExportAction::ChannelTo(path.to_path_buf())));
}

/// PNG を読み戻した、行は下から上の RGBA8（`load_png` と同じ向き）。
fn png_bytes(path: &Path) -> Vec<u8> {
    load_png(path).2
}

#[test]
fn the_menu_entries_only_ask_for_the_file_or_the_folder() {
    let mut s = two_sets();
    s.apply(Action::Export(ExportAction::ChannelDialog));
    assert_eq!(s.dialog_request, Some(DialogRequest::ExportChannel));
    s.dialog_request = None;
    s.apply(Action::Export(ExportAction::ChannelsDialog));
    assert_eq!(s.dialog_request, Some(DialogRequest::ExportChannelsFolder));
    // 描いている間は頼まない
    s.dialog_request = None;
    let layer = s.selected_layer.unwrap();
    let stroke = s.begin_paint_stroke(layer, false).unwrap();
    s.apply(Action::Export(ExportAction::ChannelDialog));
    s.apply(Action::Export(ExportAction::ChannelsDialog));
    assert_eq!(s.dialog_request, None);
    assert!(s.message.contains("描いている間"), "{}", s.message);
    s.doc.cancel_stroke(stroke);
}

#[test]
fn the_dialog_suggests_the_same_name_the_folder_export_would_write() {
    let mut s = AppState::new(64, 64);
    assert_eq!(default_channel_file_name(&s), "Texture_Color.png");
    s.apply(Action::M2Ui(crate::m2::UiOp::PaintChannel(
        Channel::Roughness,
    )));
    assert_eq!(default_channel_file_name(&s), "Texture_Roughness.png");
    // セットが複数ならセット名が入る
    let s = two_sets();
    let name = default_channel_file_name(&s);
    assert!(
        name == "Texture_Skin_Color.png" || name == "Texture_Hair_Color.png",
        "{name}"
    );
    assert!(name.contains(&s.sets.current().name), "{name}");
}

#[test]
fn a_channel_png_is_the_same_bytes_as_the_template_and_the_composite() {
    let dir = Dir::new("png-color");
    let mut s = AppState::new(64, 64);
    s.export.padding = 0;
    let layer = s.selected_layer.unwrap();
    paint_left_half(&mut s.doc, layer, Channel::Color, [10, 20, 30, 255]);
    paint_left_half(&mut s.doc, layer, Channel::Roughness, [70, 70, 70, 255]);
    let path = dir.0.join("color.png");
    export_channel(&mut s, &path);
    assert!(s.export.is_exporting(), "別のスレッドで書く");
    s.wait_export();
    assert_eq!(dir.files(), ["color.png"], "一時ファイルは残らない");
    // 値はテンプレートの Albedo（BaseColor）と同じ。ファイルのバイトまで同じ
    let template = Dir::new("png-color-template");
    export(&mut s, "unity-standard", &template.0);
    s.wait_export();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        std::fs::read(template.0.join("Texture_Albedo.png")).unwrap()
    );
    assert_eq!(png_bytes(&path), s.doc.composite(s.doc.bounds()).unwrap());
    // 1 枚の書き出しは結果の窓を出さず、状態の帯に書いた場所を出す
    assert!(s.export.report.is_some(), "テンプレートの結果");
    s.export.report = None;
    // 描くチャンネルを替えると、そのチャンネルの合成そのまま（詰めない・色を掛けない）
    s.apply(Action::M2Ui(crate::m2::UiOp::PaintChannel(
        Channel::Roughness,
    )));
    let rough = dir.0.join("rough.png");
    export_channel(&mut s, &rough);
    s.wait_export();
    assert!(s.export.report.is_none());
    assert!(
        s.message.contains("書き出しました") && s.message.contains("rough.png"),
        "{}",
        s.message
    );
    assert_eq!(
        png_bytes(&rough),
        s.doc
            .composite_channel(Channel::Roughness, s.doc.bounds())
            .unwrap()
    );
    // 文書は変わらない
    assert!(!s.doc.can_undo() && !s.modified);
    // 英語の知らせ
    s.lang = crate::lang::Lang::En;
    export_channel(&mut s, &dir.0.join("again.png"));
    s.wait_export();
    assert!(s.message.starts_with("Exported "), "{}", s.message);
}

#[test]
fn a_normal_png_follows_the_file_direction_and_a_new_png_replaces_a_chosen_one() {
    let dir = Dir::new("png-normal");
    let mut s = AppState::new(64, 64);
    s.export.padding = 0;
    let layer = s.selected_layer.unwrap();
    paint_left_half(&mut s.doc, layer, Channel::Normal, [200, 90, 220, 255]);
    s.apply(Action::M2Ui(crate::m2::UiOp::PaintChannel(Channel::Normal)));
    let opengl = dir.0.join("opengl.png");
    export_channel(&mut s, &opengl);
    s.wait_export();
    let template = Dir::new("png-normal-template");
    export(&mut s, "unity-hdrp", &template.0);
    s.wait_export();
    assert_eq!(
        std::fs::read(&opengl).unwrap(),
        std::fs::read(template.0.join("Texture_Normal.png")).unwrap(),
        "OpenGL はテンプレートの Normal と同じバイト"
    );
    // DirectX: 緑だけが 255 − G
    let settings = s
        .doc
        .normal_settings()
        .with_file_direction(yolu_core::NormalYDirection::DirectX);
    s.apply(Action::M2(crate::m2::Edit::NormalSettings {
        settings,
        coalesce: false,
    }));
    let directx = dir.0.join("directx.png");
    export_channel(&mut s, &directx);
    s.wait_export();
    let (a, b) = (png_bytes(&opengl), png_bytes(&directx));
    assert_ne!(a, b);
    for (x, y) in a.chunks(4).zip(b.chunks(4)) {
        assert_eq!([x[0], 255 - x[1], x[2], x[3]], [y[0], y[1], y[2], y[3]]);
    }
    assert_eq!(
        b,
        s.doc.normal_file_output(s.export_working_bytes()).unwrap()
    );
    // 選ぶ窓が置き換えを確かめているので、もうあるファイルは確かめずに置き換える
    export_channel(&mut s, &opengl);
    assert!(s.export.confirm.is_none());
    s.wait_export();
    assert_eq!(png_bytes(&opengl), b, "同じ名前に新しい DirectX の画像");
    assert!(
        dir.files().iter().all(|f| f.ends_with(".png")),
        "{:?}",
        dir.files()
    );
}

#[test]
fn the_dialogs_name_without_an_extension_gets_png_and_is_confirmed_when_that_file_exists() {
    let dir = Dir::new("png-named");
    // 拡張子が無い名前だけが、足した名前を確かめる道を通る（付いている名前は、選ぶ窓が確かめた）
    let bare = dir.0.join("foo");
    assert_eq!(
        channel_action(bare.clone()),
        ExportAction::ChannelNamed(dir.0.join("foo.png"))
    );
    assert_eq!(
        channel_action(dir.0.join("foo.png")),
        ExportAction::ChannelTo(dir.0.join("foo.png"))
    );
    assert_eq!(
        channel_action(dir.0.join("foo.PNG")),
        ExportAction::ChannelTo(dir.0.join("foo.PNG"))
    );
    let mut s = AppState::new(64, 64);
    s.export.padding = 0;
    let layer = s.selected_layer.unwrap();
    paint_left_half(&mut s.doc, layer, Channel::Color, [10, 20, 30, 255]);
    // 足した名前のファイルがまだ無ければ、そのまま書く
    s.apply(Action::Export(channel_action(bare.clone())));
    assert!(s.export.confirm.is_none());
    s.wait_export();
    assert_eq!(dir.files(), ["foo.png"], "{}", s.message);
    let written = std::fs::read(dir.0.join("foo.png")).unwrap();
    // 足した名前がもうあれば、確かめの窓を出して何も書かない（無断で置き換えない）
    std::fs::write(dir.0.join("foo.png"), b"mine").unwrap();
    s.apply(Action::Export(channel_action(bare.clone())));
    assert!(!s.export.is_exporting());
    let confirm = s.export.confirm.clone().expect("確かめの窓");
    assert_eq!(confirm.what, What::ChannelFile(dir.0.join("foo.png")));
    assert_eq!(
        (confirm.existing.clone(), confirm.total),
        (vec!["foo.png".to_string()], 1)
    );
    assert_eq!(s.message, "もうあるファイル 1 個を置き換えるか確かめます。");
    assert_eq!(std::fs::read(dir.0.join("foo.png")).unwrap(), b"mine");
    // やめれば、そのまま
    s.apply(Action::Export(ExportAction::CancelConfirm));
    assert!(s.export.confirm.is_none() && !s.export.is_exporting());
    assert_eq!(std::fs::read(dir.0.join("foo.png")).unwrap(), b"mine");
    // 「置き換える」で、新しい画像（結果の窓は出さない）
    s.apply(Action::Export(channel_action(bare.clone())));
    s.apply(Action::Export(ExportAction::ConfirmReplace));
    assert!(s.export.confirm.is_none());
    s.wait_export();
    assert_eq!(std::fs::read(dir.0.join("foo.png")).unwrap(), written);
    assert!(s.export.report.is_none());
    assert_eq!(dir.files(), ["foo.png"], "一時ファイルは残らない");
    // 英語の知らせ
    std::fs::write(dir.0.join("foo.png"), b"mine").unwrap();
    s.lang = crate::lang::Lang::En;
    s.apply(Action::Export(channel_action(bare.clone())));
    assert_eq!(s.message, "Confirm replacing 1 existing file.");
    // 描いている間・書き出し中は、確かめの窓も出さない
    s.apply(Action::Export(ExportAction::CancelConfirm));
    let stroke = s.begin_paint_stroke(layer, false).unwrap();
    s.apply(Action::Export(channel_action(bare.clone())));
    assert!(s.export.confirm.is_none());
    s.doc.cancel_stroke(stroke);
    assert_eq!(std::fs::read(dir.0.join("foo.png")).unwrap(), b"mine");
}

#[test]
fn a_channel_png_pads_outside_the_uvs_like_the_templates_do() {
    let dir = Dir::new("png-pad");
    let mut s = two_sets();
    let path = dir.0.join("skin.png");
    export_channel(&mut s, &path);
    s.wait_export();
    let img = load_png(&path);
    assert_eq!(px(&img, 10, 30), [255, 0, 0, 255]);
    assert_eq!(
        px(&img, 60, 30),
        [255, 0, 0, 255],
        "UV の外は境目の色で塗り広げる"
    );
    // 塗り広げない設定なら空のまま
    s.export.padding = 0;
    let plain = dir.0.join("plain.png");
    export_channel(&mut s, &plain);
    s.wait_export();
    assert_eq!(px(&load_png(&plain), 60, 30)[3], 0);
    // モデルが無ければ塗り広げず、状態の帯に理由
    s.close_link_model(1);
    s.export.padding = -1;
    export_channel(&mut s, &dir.0.join("nomodel.png"));
    s.wait_export();
    assert!(s.message.contains("塗り広げていません"), "{}", s.message);
}

#[test]
fn a_read_only_set_and_a_missing_file_name_are_refused_with_a_reason() {
    let dir = Dir::new("png-refuse");
    let mut s = two_sets();
    let index = s.sets.current_index();
    s.sets.get_mut(index).unwrap().read_only = Some("フィルターのあるレイヤーがあります".into());
    export_channel(&mut s, &dir.0.join("x.png"));
    assert!(!s.export.is_exporting());
    assert_eq!(
        s.message,
        "このテクスチャセットは読むだけです（フィルターのあるレイヤーがあります）。"
    );
    assert!(dir.files().is_empty());
    s.lang = crate::lang::Lang::En;
    export_channel(&mut s, &dir.0.join("x.png"));
    assert!(
        s.message.starts_with("This texture set is read-only"),
        "{}",
        s.message
    );
    // ファイル名の無い道
    let mut s = AppState::new(64, 64);
    s.apply(Action::Export(ExportAction::ChannelTo(PathBuf::from("/"))));
    assert!(!s.export.is_exporting());
    assert!(s.message.contains("ファイル"), "{}", s.message);
}

#[test]
fn all_channels_are_written_per_set_with_english_names_and_only_the_used_ones() {
    let dir = Dir::new("all");
    let mut s = two_sets();
    s.export.padding = 0;
    let layer = s.selected_layer.unwrap();
    paint_left_half(&mut s.doc, layer, Channel::Roughness, [60, 60, 60, 255]);
    paint_left_half(&mut s.doc, layer, Channel::Emission, [5, 6, 7, 128]);
    let current = s.sets.current().name.clone();
    export_channels(&mut s, &dir.0);
    assert!(s.export.is_exporting());
    s.wait_export();
    // 今のセットは Color・Roughness・Emission、もう 1 つは Color だけ（Metallic・Height・Normal は使っていない）
    let other = if current == "Skin" { "Hair" } else { "Skin" };
    let mut want = vec![
        format!("Texture_{current}_Color.png"),
        format!("Texture_{current}_Roughness.png"),
        format!("Texture_{current}_Emission.png"),
        format!("Texture_{other}_Color.png"),
    ];
    want.sort();
    assert_eq!(dir.files(), want);
    let rough = png_bytes(&dir.0.join(format!("Texture_{current}_Roughness.png")));
    assert_eq!(
        rough,
        s.doc
            .composite_channel(Channel::Roughness, s.doc.bounds())
            .unwrap()
    );
    let emission = load_png(&dir.0.join(format!("Texture_{current}_Emission.png")));
    assert_eq!(
        px(&emission, 10, 30),
        [5, 6, 7, 128],
        "テンプレートの Emission と違い、アルファのまま"
    );
    // 結果の窓: 種類（sRGB・リニア）
    let report = s.export.report.as_ref().unwrap();
    assert_eq!(report.images.len(), 4);
    for image in &report.images {
        let srgb = image.suffix == "Color" || image.suffix == "Emission";
        assert_eq!(image.srgb, srgb, "{}", image.file_name);
        assert!(!image.normal_map, "{}", image.file_name);
    }
    assert!(!s.modified, "文書は書き出しで変わらない");
}

#[test]
fn all_channels_write_the_derived_normal_user_channels_and_skip_unused_ones() {
    let dir = Dir::new("all-extra");
    let mut s = AppState::new(64, 64);
    s.export.padding = 0;
    let layer = s.selected_layer.unwrap();
    // Height だけ使い、Height → Normal を有効にすると、Normal の画像も出る（塗った Normal の層が無くても）
    paint_left_half(&mut s.doc, layer, Channel::Height, [200, 200, 200, 255]);
    s.doc
        .set_channel_enabled(layer, Channel::Color, false)
        .unwrap();
    let settings = yolu_core::NormalSettings::DEFAULT.with_derive(true);
    s.apply(Action::M2(crate::m2::Edit::NormalSettings {
        settings,
        coalesce: false,
    }));
    // ユーザーチャンネル（名前が使えない文字を含む）
    let info = yolu_core::ChannelInfo {
        name: "AO/Mask".into(),
        kind: yolu_core::ChannelKind::Scalar,
        color_space: yolu_core::ColorSpace::Linear,
        default: yolu_core::Rgba8::new(255, 255, 255, 255),
    };
    let user = s.doc.add_channel(info).unwrap();
    paint_left_half(&mut s.doc, layer, user, [9, 9, 9, 255]);
    export_channels(&mut s, &dir.0);
    s.wait_export();
    assert_eq!(
        dir.files(),
        [
            "Texture_AO_Mask.png",
            "Texture_Height.png",
            "Texture_Normal.png"
        ]
    );
    assert_eq!(
        png_bytes(&dir.0.join("Texture_Normal.png")),
        s.doc.normal_file_output(s.export_working_bytes()).unwrap()
    );
    let report = s.export.report.as_ref().unwrap();
    assert!(report
        .images
        .iter()
        .any(|i| i.normal_map && i.suffix == "Normal"));
    // 何も使っていない文書は書くものが無い
    let empty = Dir::new("all-empty");
    let mut s = AppState::new(64, 64);
    let layer = s.selected_layer.unwrap();
    s.doc
        .set_channel_enabled(layer, Channel::Color, false)
        .unwrap();
    export_channels(&mut s, &empty.0);
    assert!(!s.export.is_exporting());
    assert!(
        s.message.contains("書き出すものがありません"),
        "{}",
        s.message
    );
    s.lang = crate::lang::Lang::En;
    export_channels(&mut s, &empty.0);
    assert!(s.message.starts_with("Nothing to export"), "{}", s.message);
}

#[test]
fn all_channels_ask_before_replacing_and_names_that_clash_write_nothing() {
    let dir = Dir::new("all-replace");
    let mut s = two_sets();
    export_channels(&mut s, &dir.0);
    s.wait_export();
    let name = format!("Texture_{}_Color.png", s.sets.current().name);
    let original = std::fs::read(dir.0.join(&name)).unwrap();
    // 絵を変えてもう一度: 確かめる（まだ何も書かない）
    let layer = s.selected_layer.unwrap();
    paint_left_half(&mut s.doc, layer, Channel::Color, [0, 0, 255, 255]);
    export_channels(&mut s, &dir.0);
    assert!(!s.export.is_exporting());
    let confirm = s.export.confirm.clone().unwrap();
    assert_eq!(confirm.what, What::Channels);
    assert_eq!(confirm.total, 2);
    assert_eq!(confirm.existing.len(), 2);
    s.apply(Action::Export(ExportAction::CancelConfirm));
    assert_eq!(
        std::fs::read(dir.0.join(&name)).unwrap(),
        original,
        "やめたら元のまま"
    );
    // 置き換える: 確かめの窓の「置き換える」が、同じ全チャンネルをもう一度計画する
    export_channels(&mut s, &dir.0);
    s.apply(Action::Export(ExportAction::ConfirmReplace));
    assert!(s.export.is_exporting());
    s.wait_export();
    assert_ne!(std::fs::read(dir.0.join(&name)).unwrap(), original);
    assert!(s
        .export
        .report
        .as_ref()
        .unwrap()
        .images
        .iter()
        .all(|i| i.replaced));
    // セット名が同じファイルになる
    let clash = Dir::new("all-clash");
    let (a, b) = (s.sets.get(0).unwrap().uid, s.sets.get(1).unwrap().uid);
    s.rename_set(a, "a/b").unwrap();
    s.rename_set(b, "a_b").unwrap();
    export_channels(&mut s, &clash.0);
    assert!(!s.export.is_exporting());
    assert!(s.message.contains("同じファイル"), "{}", s.message);
    assert!(clash.files().is_empty());
}

#[test]
fn all_channels_skip_a_read_only_set_and_say_so_and_cancel_leaves_nothing() {
    let dir = Dir::new("all-readonly");
    let mut s = two_sets();
    let other = 1 - s.sets.current_index();
    s.sets.get_mut(other).unwrap().read_only = Some("フィルターのあるレイヤーがあります".into());
    let name = s.sets.get(other).unwrap().name.clone();
    export_channels(&mut s, &dir.0);
    s.wait_export();
    assert_eq!(dir.files().len(), 1, "{:?}", dir.files());
    let report = s.export.report.as_ref().unwrap();
    assert!(
        report.notes.contains(&Note::ReadOnly(name)),
        "{:?}",
        report.notes
    );
    // 取消: 何も書かず、一時ファイルも残さない
    let cancel = Dir::new("all-cancel");
    s.export.park_next = true;
    export_channels(&mut s, &cancel.0);
    assert!(s.export.is_exporting());
    s.apply(Action::Export(ExportAction::Cancel));
    s.wait_export();
    assert!(cancel.files().is_empty(), "{:?}", cancel.files());
}

/// 正本にすると 512 MiB を超える文書（一様なタイルの層は core では小さいが、正本では全画素を書く）も書き出せる: 書き出しは文書の写し
/// （タイルを共有）から作り、正本を経ない。
#[test]
fn a_document_whose_saved_form_exceeds_512_mib_is_exported() {
    let dir = Dir::new("png-big");
    let mut s = AppState::new(1024, 1024);
    s.export.padding = 0;
    let ts = s.doc.tile_size();
    let n = 1024 / ts;
    let layers = (520u64 << 20).div_ceil(u64::from(n * n) * u64::from(ts * ts) * 4) as u32;
    for i in 0..layers {
        let id = s.doc.add_layer(&format!("平ら {i}")).unwrap();
        let flat = [i as u8, 255 - i as u8, 7, 255].repeat((ts * ts) as usize);
        for ty in 0..n {
            for tx in 0..n {
                s.doc
                    .import_tile(id, Channel::Color, TileCoord::new(tx, ty), &flat)
                    .unwrap();
            }
        }
    }
    s.doc.clear_history().unwrap();
    // 正本の画素の値だけで 512 MiB を超える（作って確かめると 512 MiB を確保するので、数で見る）。core の画素は小さい
    assert!(u64::from(layers * n * n) * u64::from(ts * ts * 4) > yolu_io::MAX_ONE_ENTRY);
    assert!(s.doc.allocated_bytes() < 1 << 20);
    let path = dir.0.join("big.png");
    export_channel(&mut s, &path);
    s.wait_export();
    assert!(s.message.contains("書き出しました"), "{}", s.message);
    let top = layers - 1;
    let image = load_png(&path);
    assert_eq!((image.0, image.1), (1024, 1024));
    assert_eq!(px(&image, 500, 500), [top as u8, 255 - top as u8, 7, 255]);
}
