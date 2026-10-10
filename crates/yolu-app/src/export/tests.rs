//! テンプレートの書き出しの試験（画面なし）: 名前・複数のセット・パディング・AO・上書きの確かめ・取消・読むだけのセット。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use yolu_core::mesh_maps::MeshMapKind;
use yolu_core::{Channel, Document, LayerId, TileCoord};
use yolu_protocol::{MaterialInfo, MaterialKey, MeshData, Model, Submesh};

use super::*;
use crate::bake::BakeAction;
use crate::state::{Action, DialogRequest};

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
        sets: None,
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
fn an_unknown_template_is_refused_and_writes_nothing() {
    let dir = Dir::new("unknown");
    let mut s = two_sets();
    export(&mut s, "nothing", &dir.0);
    assert!(!s.export.is_exporting());
    assert!(s.message.contains("テンプレート"), "{}", s.message);
    assert!(dir.files().is_empty());
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
    // 書く前の一覧にも、今の条件で焼いた AO の画像が入る
    form(&mut s, ExportForm::UnityStandard);
    assert_eq!(
        preview_names(&mut s),
        ["Texture_Albedo.png", "Texture_Occlusion.png"]
    );
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
    assert_eq!(
        preview_names(&mut s),
        ["Texture_Albedo.png"],
        "古い AO の画像は一覧にも出さない"
    );
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
    s.apply(Action::Export(ExportAction::ChooseDestination));
    assert_eq!(s.dialog_request, None);
    s.apply(Action::Export(ExportAction::OpenWindow));
    assert!(!s.export.window.open, "描いている間は開かない");
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
                          // 黒の塗りつぶしレイヤーのマスクへ、焼いた曲率から値を作る Generator（見える所だけを残す）
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
    s.apply(Action::Export(ExportAction::ChannelsTo {
        dir: dir.to_path_buf(),
        sets: None,
    }));
}

fn export_channel(s: &mut AppState, path: &Path) {
    s.apply(Action::Export(ExportAction::ChannelTo(path.to_path_buf())));
}

/// PNG を読み戻した、行は下から上の RGBA8（`load_png` と同じ向き）。
fn png_bytes(path: &Path) -> Vec<u8> {
    load_png(path).2
}

#[test]
fn the_window_asks_for_one_destination_whatever_the_form() {
    let mut s = two_sets();
    s.apply(Action::Export(ExportAction::ChooseDestination));
    assert_eq!(s.dialog_request, Some(DialogRequest::ExportDestination));
    // 描いている間は頼まない
    s.dialog_request = None;
    let layer = s.selected_layer.unwrap();
    let stroke = s.begin_paint_stroke(layer, false).unwrap();
    s.apply(Action::Export(ExportAction::ChooseDestination));
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
    // 1 枚の書き出しは結果のウィンドウを出さず、状態の帯に書いた場所を出す
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
    // 選ぶウィンドウが置き換えを確かめているので、もうあるファイルは確かめずに置き換える
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

/// 書き出しのウィンドウを開いて形を選び、書き出す先を選ぶウィンドウが返した体で決め、「書き出す」を押す。
fn run_window(s: &mut AppState, form: ExportForm, chosen: &Path) {
    s.apply(Action::Export(ExportAction::OpenWindow));
    s.apply(Action::Export(ExportAction::SetForm(form)));
    s.apply(Action::Export(ExportAction::Destination(
        chosen.to_path_buf(),
    )));
    s.apply(Action::Export(ExportAction::Run));
}

#[test]
fn the_dialogs_name_without_an_extension_gets_png_and_is_confirmed_when_that_file_exists() {
    let dir = Dir::new("png-named");
    let bare = dir.0.join("foo");
    let mut s = AppState::new(64, 64);
    s.export.padding = 0;
    let layer = s.selected_layer.unwrap();
    paint_left_half(&mut s.doc, layer, Channel::Color, [10, 20, 30, 255]);
    // 拡張子が無い名前には `.png` を足し、付いている名前（大文字の拡張子も）はそのまま
    s.apply(Action::Export(ExportAction::Destination(bare.clone())));
    assert_eq!(s.export_file(), Some(dir.0.join("foo.png")));
    s.apply(Action::Export(ExportAction::Destination(
        dir.0.join("foo.PNG"),
    )));
    assert_eq!(s.export_file(), Some(dir.0.join("foo.PNG")));
    // 足した名前のファイルがまだ無ければ、そのまま書く
    run_window(&mut s, ExportForm::ChannelPng, &bare);
    assert!(s.export.confirm.is_none());
    s.wait_export();
    assert_eq!(dir.files(), ["foo.png"], "{}", s.message);
    let written = std::fs::read(dir.0.join("foo.png")).unwrap();
    // 足した名前がもうあれば、確認のウィンドウを出して何も書かない（無断で置き換えない）。書き出しのウィンドウは開いたまま
    std::fs::write(dir.0.join("foo.png"), b"mine").unwrap();
    run_window(&mut s, ExportForm::ChannelPng, &bare);
    assert!(!s.export.is_exporting());
    assert!(s.export.window.open, "確かめている間は開いたまま");
    let confirm = s.export.confirm.clone().expect("確認のウィンドウ");
    assert_eq!(confirm.what, What::ChannelFile(dir.0.join("foo.png")));
    assert_eq!(
        (confirm.existing.clone(), confirm.total),
        (vec!["foo.png".to_string()], 1)
    );
    assert_eq!(s.message, "もうあるファイル 1 個を置き換えるか確かめます。");
    assert_eq!(std::fs::read(dir.0.join("foo.png")).unwrap(), b"mine");
    // やめれば、そのまま（ウィンドウに戻る）
    s.apply(Action::Export(ExportAction::CancelConfirm));
    assert!(s.export.confirm.is_none() && !s.export.is_exporting());
    assert!(s.export.window.open);
    assert_eq!(std::fs::read(dir.0.join("foo.png")).unwrap(), b"mine");
    // 「置き換える」で、新しい画像（結果のウィンドウは出さない）。書き始めたらウィンドウは閉じる
    s.apply(Action::Export(ExportAction::Run));
    s.apply(Action::Export(ExportAction::ConfirmReplace));
    assert!(s.export.confirm.is_none());
    assert!(!s.export.window.open, "書き始めたら閉じる");
    s.wait_export();
    assert_eq!(std::fs::read(dir.0.join("foo.png")).unwrap(), written);
    assert!(s.export.report.is_none());
    assert_eq!(dir.files(), ["foo.png"], "一時ファイルは残らない");
    // 選ぶウィンドウが確かめた名前（拡張子つき）は、重ねて確かめずに置き換える
    std::fs::write(dir.0.join("foo.png"), b"mine").unwrap();
    run_window(&mut s, ExportForm::ChannelPng, &dir.0.join("foo.png"));
    assert!(s.export.confirm.is_none());
    s.wait_export();
    assert_eq!(std::fs::read(dir.0.join("foo.png")).unwrap(), written);
    // 英語の知らせ
    std::fs::write(dir.0.join("foo.png"), b"mine").unwrap();
    s.lang = crate::lang::Lang::En;
    run_window(&mut s, ExportForm::ChannelPng, &bare);
    assert_eq!(s.message, "Confirm replacing 1 existing file.");
    // 描いている間・書き出し中は、確認のウィンドウも出さない
    s.apply(Action::Export(ExportAction::CancelConfirm));
    let stroke = s.begin_paint_stroke(layer, false).unwrap();
    s.apply(Action::Export(ExportAction::Run));
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
    // 結果のウィンドウ: 種類（sRGB・リニア）
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
    // Height だけ使い、Height → Normal を有効にすると、Normal の画像も出る（塗った Normal のレイヤーが無くても）
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
    assert_eq!(confirm.what, What::Channels { sets: None });
    assert_eq!(confirm.total, 2);
    assert_eq!(confirm.existing.len(), 2);
    s.apply(Action::Export(ExportAction::CancelConfirm));
    assert_eq!(
        std::fs::read(dir.0.join(&name)).unwrap(),
        original,
        "やめたら元のまま"
    );
    // 置き換える: 確認のウィンドウの「置き換える」が、同じ全チャンネルをもう一度計画する
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

/// 正本にすると 512 MiB を超える文書（一様なタイルのレイヤーは core では小さいが、正本では全画素を書く）も書き出せる: 書き出しは文書の写し
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

// ───────── 書き出しのウィンドウ（形・書き出す先・「書き出す」） ─────────

/// 名前を付けて保存した状態にする（既定の書き出す先とファイル名の元になる）。
fn saved_as(s: &mut AppState, path: &Path) {
    s.apply(Action::SaveProjectAs(path.to_path_buf()));
    s.wait_save();
    assert!(s.project.is_some(), "{}", s.message);
}

fn choose(s: &mut AppState, path: &Path) {
    s.apply(Action::Export(ExportAction::Destination(
        path.to_path_buf(),
    )));
}

fn form(s: &mut AppState, form: ExportForm) {
    s.apply(Action::Export(ExportAction::SetForm(form)));
}

/// 書くファイルの一覧（ウィンドウが書く前に出す物）の名前。並べ替えて返す。
fn preview_names(s: &mut AppState) -> Vec<String> {
    let preview = s.export_preview();
    assert_eq!(preview.problem, None);
    let mut names: Vec<String> = preview.files.into_iter().map(|f| f.name).collect();
    names.sort();
    names
}

fn set_uids(s: &AppState) -> Vec<u32> {
    (0..s.sets.len())
        .map(|i| s.sets.get(i).unwrap().uid)
        .collect()
}

fn check(s: &mut AppState, uid: u32, on: bool) {
    s.apply(Action::Export(ExportAction::SetChecked { uid, on }));
}

#[test]
fn the_forms_have_stable_keys_names_and_the_templates_they_stand_for() {
    assert_eq!(ExportForm::default(), ExportForm::ChannelPng);
    let keys: Vec<&str> = ExportForm::ALL.iter().map(|f| f.key()).collect();
    assert_eq!(
        keys,
        [
            "channel",
            "channels",
            "unity-standard",
            "unity-hdrp",
            "liltoon"
        ],
        "設定のファイルに書く名前は変えない"
    );
    for f in ExportForm::ALL {
        assert_eq!(ExportForm::from_key(f.key()), Some(f));
        assert_eq!(f.writes_file(), f == ExportForm::ChannelPng, "{f:?}");
    }
    assert_eq!(ExportForm::from_key("png"), None);
    assert_eq!(ExportForm::from_key(""), None);
    // テンプレートの形は、組み込みのテンプレートの ID と名前に一致する（並びも）
    let templates = yolu_core::export::ExportTemplate::built_in();
    let forms: Vec<ExportForm> = ExportForm::ALL
        .into_iter()
        .filter(|f| f.template_id().is_some())
        .collect();
    assert_eq!(forms.len(), templates.len());
    for (f, template) in forms.iter().zip(&templates) {
        assert_eq!(f.template_id(), Some(template.id.as_str()));
        for lang in [crate::lang::Lang::Ja, crate::lang::Lang::En] {
            assert_eq!(f.name(lang), template.name, "{f:?}");
        }
    }
    assert_eq!(ExportForm::ChannelPng.template_id(), None);
    assert_eq!(ExportForm::AllChannels.template_id(), None);
    // 名前は日本語と英語がある
    assert_eq!(
        ExportForm::ChannelPng.name(crate::lang::Lang::Ja),
        "今のチャンネル"
    );
    assert_eq!(
        ExportForm::ChannelPng.name(crate::lang::Lang::En),
        "Current Channel"
    );
    assert_eq!(
        ExportForm::AllChannels.name(crate::lang::Lang::Ja),
        "チャンネルごと"
    );
    assert_eq!(
        ExportForm::AllChannels.name(crate::lang::Lang::En),
        "Per Channel"
    );
}

#[test]
fn the_form_is_remembered_in_the_settings_and_not_in_the_document() {
    let mut s = AppState::new(64, 64);
    assert_eq!(s.export_form(), ExportForm::ChannelPng);
    form(&mut s, ExportForm::LilToon);
    assert_eq!(s.export_form(), ExportForm::LilToon);
    assert_eq!(s.settings().export_form, ExportForm::LilToon);
    // 読み込んだ設定の形が、開いたときの形になる
    let mut other = AppState::new(64, 64);
    other.load_settings(s.settings());
    assert_eq!(other.export_form(), ExportForm::LilToon);
    // 文書を替えても形は変わらない
    other.np_project_replaced();
    assert_eq!(other.export_form(), ExportForm::LilToon);
}

#[test]
fn the_destination_starts_at_the_project_folder_and_follows_the_form() {
    let dir = Dir::new("window-default");
    let mut s = AppState::new(64, 64);
    // 保存していないプロジェクトには、既定の先が無い
    assert_eq!(s.export_destination(), None);
    s.apply(Action::Export(ExportAction::Run));
    assert!(!s.export.is_exporting());
    assert_eq!(s.message, "出力先がありません。");
    saved_as(&mut s, &dir.0.join("Hero.ylp"));
    // PNG は、プロジェクトのフォルダの既定のファイル名
    assert_eq!(s.export_destination(), Some(dir.0.join("Hero_Color.png")));
    s.apply(Action::M2Ui(crate::m2::UiOp::PaintChannel(
        Channel::Roughness,
    )));
    assert_eq!(
        s.export_destination(),
        Some(dir.0.join("Hero_Roughness.png"))
    );
    // ほかの形はフォルダ
    for f in [
        ExportForm::AllChannels,
        ExportForm::UnityStandard,
        ExportForm::Hdrp,
        ExportForm::LilToon,
    ] {
        form(&mut s, f);
        assert_eq!(s.export_destination(), Some(dir.0.clone()), "{f:?}");
    }
}

#[test]
fn a_chosen_folder_is_kept_for_every_form_until_the_project_is_replaced() {
    let dir = Dir::new("window-folder");
    let out = dir.0.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let mut s = AppState::new(64, 64);
    saved_as(&mut s, &dir.0.join("Hero.ylp"));
    s.apply(Action::Export(ExportAction::OpenWindow));
    form(&mut s, ExportForm::AllChannels);
    choose(&mut s, &out);
    assert_eq!(s.export_destination(), Some(out.clone()));
    form(&mut s, ExportForm::Hdrp);
    assert_eq!(
        s.export_destination(),
        Some(out.clone()),
        "フォルダの形で共有"
    );
    form(&mut s, ExportForm::ChannelPng);
    assert_eq!(
        s.export_destination(),
        Some(out.join("Hero_Color.png")),
        "PNG はそのフォルダの既定の名前"
    );
    // 閉じて開き直しても覚えている
    s.apply(Action::Export(ExportAction::CloseWindow));
    s.apply(Action::Export(ExportAction::OpenWindow));
    assert_eq!(s.export_folder(), Some(out.clone()));
    // プロジェクトを替えたら忘れる（既定の、プロジェクトのフォルダに戻る）
    s.np_project_replaced();
    s.apply(Action::Export(ExportAction::OpenWindow));
    assert_eq!(s.export_folder(), Some(dir.0.clone()));
}

#[test]
fn what_the_chooser_returns_is_remembered_as_where_the_next_one_starts() {
    use crate::dialog::places::{Place, Rule};
    let dir = Dir::new("window-places");
    let (out, files, documents) = (
        dir.0.join("out"),
        dir.0.join("files"),
        dir.0.join("documents"),
    );
    for d in [&out, &files, &documents] {
        std::fs::create_dir_all(d).unwrap();
    }
    let start = |s: &AppState, place| {
        crate::dialog::start_folder_with(s, place, Rule::Open, Some(&documents))
    };
    // 保存していない文書は、書き出す先が決まらないので、選ぶウィンドウの既定（前に使った場所 → 文書のフォルダ → 書類）に任せる
    let mut s = AppState::new(64, 64);
    assert_eq!(s.export_destination(), None);
    assert_eq!(start(&s, Place::ImageExport), Some(documents.clone()));
    // フォルダの形: 選んだフォルダそのものを覚え、書き出す先にも渡る
    form(&mut s, ExportForm::Hdrp);
    window::dialog_returned(&mut s, out.clone());
    assert_eq!(s.export_destination(), Some(out.clone()));
    assert_eq!(start(&s, Place::ImageExport), Some(out.clone()));
    // PNG のファイル: その置き場を覚える（ファイルそのものではない）
    form(&mut s, ExportForm::ChannelPng);
    window::dialog_returned(&mut s, files.join("tex.png"));
    assert_eq!(s.export_destination(), Some(files.join("tex.png")));
    assert_eq!(start(&s, Place::ImageExport), Some(files.clone()));
    // ほかの種類の始まりには移らない
    assert_eq!(start(&s, Place::Model), Some(documents.clone()));
    assert_eq!(start(&s, Place::PsdExport), Some(documents.clone()));
}

#[test]
fn a_chosen_png_keeps_a_custom_name_but_follows_the_channel_with_the_default_one() {
    let dir = Dir::new("window-file");
    let mut s = AppState::new(64, 64);
    choose(&mut s, &dir.0.join("Texture_Color.png"));
    assert_eq!(s.export_file(), Some(dir.0.join("Texture_Color.png")));
    s.apply(Action::M2Ui(crate::m2::UiOp::PaintChannel(
        Channel::Roughness,
    )));
    assert_eq!(
        s.export_file(),
        Some(dir.0.join("Texture_Roughness.png")),
        "既定の名前を選んだなら、名前はチャンネルを追いかける"
    );
    choose(&mut s, &dir.0.join("mine.png"));
    s.apply(Action::M2Ui(crate::m2::UiOp::PaintChannel(Channel::Color)));
    assert_eq!(
        s.export_file(),
        Some(dir.0.join("mine.png")),
        "自分で付けた名前は変わらない"
    );
    // ファイルの先を選ぶと、フォルダの形の既定の置き場にもなる
    form(&mut s, ExportForm::AllChannels);
    assert_eq!(s.export_folder(), Some(dir.0.clone()));
}

#[test]
fn running_each_form_writes_the_same_files_as_the_direct_actions() {
    let direct = Dir::new("window-direct");
    let window = Dir::new("window-window");
    let mut s = two_sets();
    s.export.padding = 0;
    // 描くチャンネルの PNG
    export_channel(&mut s, &direct.0.join("one.png"));
    s.wait_export();
    run_window(&mut s, ExportForm::ChannelPng, &window.0.join("one.png"));
    assert!(s.export.is_exporting());
    assert!(!s.export.window.open, "書き始めたら閉じる");
    s.wait_export();
    assert_eq!(
        png_bytes(&direct.0.join("one.png")),
        png_bytes(&window.0.join("one.png"))
    );
    assert!(
        s.export.report.is_none(),
        "1 枚の PNG は結果のウィンドウを出さない"
    );
    // 全チャンネル
    let all_direct = Dir::new("window-all-direct");
    let all_window = Dir::new("window-all-window");
    export_channels(&mut s, &all_direct.0);
    s.wait_export();
    run_window(&mut s, ExportForm::AllChannels, &all_window.0);
    s.wait_export();
    assert!(!all_window.files().is_empty());
    assert_eq!(all_window.files(), all_direct.files());
    for f in all_window.files() {
        assert_eq!(
            std::fs::read(all_window.0.join(&f)).unwrap(),
            std::fs::read(all_direct.0.join(&f)).unwrap(),
            "{f}"
        );
    }
    assert!(
        s.export.report.is_some(),
        "フォルダへの書き出しは結果のウィンドウ"
    );
    s.apply(Action::Export(ExportAction::DismissReport));
    // 3 つのテンプレート
    for f in [
        ExportForm::UnityStandard,
        ExportForm::Hdrp,
        ExportForm::LilToon,
    ] {
        let id = f.template_id().unwrap();
        let by_action = Dir::new("window-t-direct");
        let by_window = Dir::new("window-t-window");
        export(&mut s, id, &by_action.0);
        s.wait_export();
        run_window(&mut s, f, &by_window.0);
        s.wait_export();
        assert!(!by_window.files().is_empty(), "{f:?}");
        assert_eq!(by_window.files(), by_action.files(), "{f:?}");
        for name in by_window.files() {
            assert_eq!(
                std::fs::read(by_window.0.join(&name)).unwrap(),
                std::fs::read(by_action.0.join(&name)).unwrap(),
                "{f:?} {name}"
            );
        }
        s.apply(Action::Export(ExportAction::DismissReport));
    }
}

#[test]
fn the_window_asks_before_replacing_a_folder_export_and_cancel_leaves_the_files() {
    let dir = Dir::new("window-replace");
    let mut s = two_sets();
    run_window(&mut s, ExportForm::UnityStandard, &dir.0);
    s.wait_export();
    let before = std::fs::read(dir.0.join("Texture_Skin_Albedo.png")).unwrap();
    let layer = s.selected_layer.unwrap();
    paint_left_half(&mut s.doc, layer, Channel::Color, [0, 0, 255, 255]);
    run_window(&mut s, ExportForm::UnityStandard, &dir.0);
    assert!(!s.export.is_exporting());
    assert!(s.export.confirm.is_some());
    assert!(s.export.window.open, "確かめている間は開いたまま");
    s.apply(Action::Export(ExportAction::CancelConfirm));
    assert!(s.export.window.open, "やめたらウィンドウに戻る");
    assert_eq!(
        std::fs::read(dir.0.join("Texture_Skin_Albedo.png")).unwrap(),
        before
    );
    s.apply(Action::Export(ExportAction::Run));
    s.apply(Action::Export(ExportAction::ConfirmReplace));
    assert!(s.export.is_exporting());
    assert!(!s.export.window.open);
    s.wait_export();
    assert_ne!(
        std::fs::read(dir.0.join("Texture_Skin_Albedo.png")).unwrap(),
        before,
        "置き換えた"
    );
}

#[test]
fn a_run_from_the_window_can_be_canceled_and_a_refusal_keeps_the_window_open() {
    let dir = Dir::new("window-cancel");
    let mut s = AppState::new(256, 256);
    s.bake.backend = crate::bake::BakeBackend::Cpu;
    let layer = s.selected_layer.unwrap();
    paint_left_half(&mut s.doc, layer, Channel::Color, [1, 2, 3, 255]);
    s.export.park_next = true;
    run_window(&mut s, ExportForm::UnityStandard, &dir.0);
    assert!(s.export.is_exporting());
    // 書き出し中は、もう 1 つは始められない（ウィンドウは開き直せるが、書けない）
    s.apply(Action::Export(ExportAction::OpenWindow));
    s.apply(Action::Export(ExportAction::Run));
    assert!(s.message.contains("書き出し中"), "{}", s.message);
    assert!(s.export.window.open, "断られたら閉じない");
    s.apply(Action::Export(ExportAction::Cancel));
    s.wait_export();
    assert!(s.message.contains("取り消しました"), "{}", s.message);
    assert!(dir.files().is_empty(), "{:?}", dir.files());
    // 描いている間は断り、ウィンドウは開いたまま
    let stroke = s.begin_paint_stroke(layer, false).unwrap();
    s.apply(Action::Export(ExportAction::Run));
    assert!(!s.export.is_exporting());
    assert!(s.message.contains("描いている間"), "{}", s.message);
    assert!(s.export.window.open);
    s.doc.cancel_stroke(stroke);
    // 断る理由は今のまま: モデルが無いので塗り広げなかった、と書き出したあとの知らせに出る
    s.export.padding = -1;
    s.apply(Action::Export(ExportAction::Run));
    s.wait_export();
    assert!(s.message.contains("塗り広げていません"), "{}", s.message);
}

#[test]
fn the_form_and_padding_choices_are_the_menu_entries_of_the_popups() {
    use crate::ui::menu::Entry;
    let mut s = AppState::new(64, 64);
    form(&mut s, ExportForm::Hdrp);
    for lang in [crate::lang::Lang::Ja, crate::lang::Lang::En] {
        s.lang = lang;
        let entries = window::form_entries(&s);
        let labels: Vec<String> = entries
            .iter()
            .map(|e| match e {
                Entry::Item { label, .. } => label.clone(),
                _ => unreachable!(),
            })
            .collect();
        let expected: Vec<String> = ExportForm::ALL
            .iter()
            .map(|f| f.name(lang).to_owned())
            .collect();
        assert_eq!(labels, expected);
        let selected: Vec<bool> = entries
            .iter()
            .map(|e| {
                matches!(
                    e,
                    Entry::Item {
                        check: crate::ui::menu::Check::Radio,
                        ..
                    }
                )
            })
            .collect();
        assert_eq!(
            selected,
            [false, false, false, true, false],
            "選んでいる形に印"
        );
    }
    // 余白は設定と同じ値（ここから替えると設定も替わる）
    s.apply(Action::Prefs(crate::prefs::PrefsAction::Set(
        crate::prefs::Pref::ExportPadding(8),
    )));
    assert_eq!(s.export.padding, 8);
    assert_eq!(s.settings().export_padding, 8);
}

#[test]
fn a_destination_chosen_in_the_previous_project_is_not_used_after_the_project_is_replaced() {
    let dir = Dir::new("window-replaced");
    let out = dir.0.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let mut s = AppState::new(64, 64);
    saved_as(&mut s, &dir.0.join("Hero.ylp"));
    s.apply(Action::Export(ExportAction::OpenWindow));
    // 選ぶウィンドウが置き換えを確かめた PNG（自分で付けた名前）と、フォルダの形のフォルダ
    let mine = out.join("mine.png");
    std::fs::write(&mine, b"mine").unwrap();
    choose(&mut s, &mine);
    assert_eq!(s.export_file(), Some(mine.clone()));
    form(&mut s, ExportForm::AllChannels);
    choose(&mut s, &out);
    assert_eq!(s.export_folder(), Some(out.clone()));
    form(&mut s, ExportForm::ChannelPng);
    // ウィンドウを開いたまま、別のプロジェクトを開く（`OpenWindow` を通らない）: 前の先は使わず、今のプロジェクトの既定に戻る
    crate::project::open_into(&mut s, &dir.0.join("Hero.ylp"));
    assert!(s.export.window.open);
    assert_eq!(s.export_folder(), Some(dir.0.clone()));
    assert_eq!(s.export_file(), Some(dir.0.join("Hero_Color.png")));
    // 書き出しても、前のプロジェクトで選んだファイルには触れない（確かめも済んだことにしない）
    std::fs::write(dir.0.join("Hero_Color.png"), b"theirs").unwrap();
    s.apply(Action::Export(ExportAction::Run));
    assert!(!s.export.is_exporting());
    assert_eq!(
        s.export.confirm.as_ref().map(|c| c.what.clone()),
        Some(What::ChannelFile(dir.0.join("Hero_Color.png"))),
        "もうあるファイルは確かめる"
    );
    assert_eq!(std::fs::read(&mine).unwrap(), b"mine");
    assert_eq!(
        std::fs::read(dir.0.join("Hero_Color.png")).unwrap(),
        b"theirs"
    );
    s.apply(Action::Export(ExportAction::CancelConfirm));
    // 新しいプロジェクト（保存していない）にすると、先が決まらず、書かない
    crate::project::new_into(&mut s);
    assert!(s.export.window.open);
    assert_eq!(s.export_destination(), None);
    s.apply(Action::Export(ExportAction::Run));
    assert!(!s.export.is_exporting() && s.export.confirm.is_none());
    assert_eq!(s.message, "出力先がありません。");
    assert_eq!(std::fs::read(&mine).unwrap(), b"mine");
    // 選び直せば、その先を今のプロジェクトで使える
    form(&mut s, ExportForm::AllChannels);
    choose(&mut s, &out);
    assert_eq!(s.export_folder(), Some(out));
}

#[test]
fn the_choosers_confirmation_covers_one_export_and_the_second_one_asks() {
    let dir = Dir::new("window-once");
    let mut s = AppState::new(64, 64);
    s.export.padding = 0;
    let layer = s.selected_layer.unwrap();
    paint_left_half(&mut s.doc, layer, Channel::Color, [10, 20, 30, 255]);
    let path = dir.0.join("foo.png");
    std::fs::write(&path, b"mine").unwrap();
    // 選ぶウィンドウが置き換えを確かめたファイル: 1 回目は確かめずに置き換える
    run_window(&mut s, ExportForm::ChannelPng, &path);
    assert!(s.export.confirm.is_none() && s.export.is_exporting());
    s.wait_export();
    let written = std::fs::read(&path).unwrap();
    assert_ne!(written, b"mine");
    // 選び直さずに、もう 1 度（もうあるファイル）: 確かめる。書き換えない
    std::fs::write(&path, b"mine").unwrap();
    s.apply(Action::Export(ExportAction::OpenWindow));
    s.apply(Action::Export(ExportAction::Run));
    assert!(!s.export.is_exporting());
    assert_eq!(
        s.export.confirm.as_ref().map(|c| c.what.clone()),
        Some(What::ChannelFile(path.clone()))
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"mine");
    s.apply(Action::Export(ExportAction::CancelConfirm));
    // 選び直すと、また 1 回だけ確かめずに置き換える
    run_window(&mut s, ExportForm::ChannelPng, &path);
    assert!(s.export.confirm.is_none() && s.export.is_exporting());
    s.wait_export();
    assert_eq!(std::fs::read(&path).unwrap(), written);
    // 書き出しが断られた（描いている間）ときも、確かめは使い切る
    std::fs::write(&path, b"mine").unwrap();
    s.apply(Action::Export(ExportAction::OpenWindow));
    s.apply(Action::Export(ExportAction::Destination(path.clone())));
    let stroke = s.begin_paint_stroke(layer, false).unwrap();
    s.apply(Action::Export(ExportAction::Run));
    s.doc.cancel_stroke(stroke);
    s.apply(Action::Export(ExportAction::Run));
    assert!(
        s.export.confirm.is_some(),
        "断られたあとも、確かめずには書かない"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"mine");
}

// ───────── 書き出すテクスチャセットのチェックと、書くファイルの一覧 ─────────

/// Skin に Roughness、Hair に Emission も描いてある 2 つのセット（書くファイルを増やして、色空間の違うファイルも出す）。
fn two_sets_with_more_channels() -> AppState {
    let mut s = two_sets();
    let other = 1 - s.sets.current_index();
    let layer = s.selected_layer.unwrap();
    paint_left_half(&mut s.doc, layer, Channel::Roughness, [90, 90, 90, 255]);
    let doc = s.set_doc_mut(other);
    let hair_layer = doc.layers()[0].id();
    paint_left_half(doc, hair_layer, Channel::Emission, [0, 0, 255, 255]);
    s
}

#[test]
fn a_set_taken_out_of_the_list_is_not_written_by_a_template_or_per_channel() {
    for (form_kind, tag) in [
        (ExportForm::UnityStandard, "template"),
        (ExportForm::Hdrp, "hdrp"),
        (ExportForm::LilToon, "liltoon"),
        (ExportForm::AllChannels, "channels"),
    ] {
        let dir = Dir::new(&format!("pick-{tag}"));
        let mut s = two_sets_with_more_channels();
        s.export.padding = 0;
        let uids = set_uids(&s);
        let names: Vec<String> = (0..2)
            .map(|i| s.sets.get(i).unwrap().name.clone())
            .collect();
        s.apply(Action::Export(ExportAction::OpenWindow));
        form(&mut s, form_kind);
        choose(&mut s, &dir.0);
        assert_eq!(s.export_checked_uids(), uids, "{tag}: はじめは全部入り");
        // 両方入れて書いた名前と、1 つ外した名前
        let all = {
            let all_dir = Dir::new(&format!("pick-{tag}-all"));
            choose(&mut s, &all_dir.0);
            s.apply(Action::Export(ExportAction::Run));
            s.wait_export();
            all_dir.files()
        };
        assert!(
            all.iter().any(|f| f.contains(&format!("_{}_", names[1]))),
            "{tag}: {all:?}"
        );
        s.apply(Action::Export(ExportAction::OpenWindow));
        choose(&mut s, &dir.0);
        check(&mut s, uids[1], false);
        assert_eq!(s.export_checked_uids(), vec![uids[0]]);
        s.apply(Action::Export(ExportAction::Run));
        s.wait_export();
        let written = dir.files();
        assert!(!written.is_empty(), "{tag}");
        assert!(
            written
                .iter()
                .all(|f| f.contains(&format!("_{}_", names[0]))),
            "{tag}: 外したセットは書かない {written:?}"
        );
        let expected: Vec<String> = all
            .iter()
            .filter(|f| f.contains(&format!("_{}_", names[0])))
            .cloned()
            .collect();
        assert_eq!(
            written, expected,
            "{tag}: 外さなかったセットの名前は変わらない"
        );
    }
}

#[test]
fn the_template_and_channels_actions_take_the_sets_to_write_and_none_means_all() {
    let dir = Dir::new("pick-actions");
    let mut s = two_sets();
    s.export.padding = 0;
    let uids = set_uids(&s);
    let first = s.sets.get(0).unwrap().name.clone();
    s.apply(Action::Export(ExportAction::TemplateTo {
        id: "unity-standard".into(),
        dir: dir.0.clone(),
        sets: Some(vec![uids[0], 9999]),
    }));
    s.wait_export();
    assert_eq!(dir.files(), [format!("Texture_{first}_Albedo.png")]);
    let dir = Dir::new("pick-actions-channels");
    s.apply(Action::Export(ExportAction::ChannelsTo {
        dir: dir.0.clone(),
        sets: Some(vec![uids[0]]),
    }));
    s.wait_export();
    assert_eq!(dir.files(), [format!("Texture_{first}_Color.png")]);
    let dir = Dir::new("pick-actions-none");
    export(&mut s, "unity-standard", &dir.0);
    s.wait_export();
    assert_eq!(dir.files().len(), 2, "None は全部: {:?}", dir.files());
    // 外していても、選びの中に読むだけのセットがあれば、これまでどおり書かず知らせる
    s.sets.get_mut(1).unwrap().read_only = Some("試験".into());
    let dir = Dir::new("pick-actions-readonly");
    s.apply(Action::Export(ExportAction::ChannelsTo {
        dir: dir.0.clone(),
        sets: Some(uids.clone()),
    }));
    s.wait_export();
    assert_eq!(dir.files().len(), 1);
    assert_eq!(
        s.export.report.as_ref().unwrap().notes,
        [Note::ReadOnly(s.sets.get(1).unwrap().name.clone())]
    );
}

#[test]
fn nothing_checked_cannot_be_exported_and_writes_nothing() {
    let dir = Dir::new("pick-none");
    let mut s = two_sets();
    let uids = set_uids(&s);
    s.apply(Action::Export(ExportAction::OpenWindow));
    form(&mut s, ExportForm::LilToon);
    choose(&mut s, &dir.0);
    for uid in &uids {
        check(&mut s, *uid, false);
    }
    assert!(s.export_checked_uids().is_empty());
    assert_eq!(s.export_preview(), crate::export::list::Preview::default());
    s.apply(Action::Export(ExportAction::Run));
    assert!(!s.export.is_exporting());
    assert_eq!(s.message, "書き出すテクスチャセットがありません。");
    assert!(s.export.window.open, "押せないままウィンドウは開いている");
    assert!(dir.files().is_empty());
    // 1 つ入れ直せば書ける
    check(&mut s, uids[0], true);
    s.apply(Action::Export(ExportAction::Run));
    s.wait_export();
    assert!(!dir.files().is_empty());
}

#[test]
fn the_current_channel_png_lists_only_the_current_set_and_it_cannot_be_unchecked() {
    let dir = Dir::new("pick-current");
    let mut s = two_sets();
    s.export.padding = 0;
    let current = s.sets.current().uid;
    let name = s.sets.current().name.clone();
    s.apply(Action::Export(ExportAction::OpenWindow));
    assert_eq!(s.export_form(), ExportForm::ChannelPng);
    let rows = s.export_set_rows();
    assert_eq!(
        rows,
        [crate::export::list::SetRow {
            uid: current,
            name,
            checked: true,
            enabled: false,
            read_only: false,
        }]
    );
    check(&mut s, current, false);
    assert!(s.export_set_rows()[0].checked, "選べない");
    assert_eq!(s.export_checked_uids(), vec![current]);
    // 別のセットに替えれば、その 1 つだけ
    let other = 1 - s.sets.current_index();
    s.apply(Action::SelectSet(s.sets.get(other).unwrap().uid));
    assert_eq!(s.export_set_rows().len(), 1);
    assert_eq!(s.export_set_rows()[0].uid, s.sets.current().uid);
    // 別の出力テンプレートでは全部が並び、外した印が効く（プロジェクトの間だけ覚えている）
    form(&mut s, ExportForm::AllChannels);
    assert_eq!(s.export_set_rows().len(), 2);
    assert!(
        !s.export_set_rows()
            .iter()
            .find(|r| r.uid == current)
            .unwrap()
            .checked
    );
    // 書き出す
    form(&mut s, ExportForm::ChannelPng);
    choose(&mut s, &dir.0.join("one.png"));
    s.apply(Action::Export(ExportAction::Run));
    s.wait_export();
    assert_eq!(dir.files(), ["one.png"]);
}

#[test]
fn the_set_list_follows_added_removed_and_renamed_sets_and_a_read_only_set_stays_unchecked() {
    let mut s = two_sets();
    s.apply(Action::Export(ExportAction::OpenWindow));
    form(&mut s, ExportForm::UnityStandard);
    let uids = set_uids(&s);
    let rows = s.export_set_rows();
    assert_eq!(rows.iter().map(|r| r.uid).collect::<Vec<_>>(), uids);
    assert!(rows.iter().all(|r| r.checked && r.enabled && !r.read_only));
    // 名前の変更に追いかける。外した印は uid に付くので、名前が変わっても外れたまま
    check(&mut s, uids[1], false);
    s.rename_set(uids[1], "Body").unwrap();
    let rows = s.export_set_rows();
    assert_eq!((rows[1].name.as_str(), rows[1].checked), ("Body", false));
    assert!(rows[0].checked);
    // セットを足すと、チェックが入った行が増える
    let added = s.add_texture_set().unwrap();
    let rows = s.export_set_rows();
    assert_eq!(rows.len(), 3);
    assert_eq!((rows[2].uid, rows[2].checked), (added, true));
    assert_eq!(s.export_checked_uids(), vec![uids[0], added]);
    // セットを消すと、一覧からも、書き出す物からも無くなる（外していたセットでも）
    s.remove_sets(&[uids[1]]).unwrap();
    let rows = s.export_set_rows();
    assert_eq!(
        rows.iter().map(|r| r.uid).collect::<Vec<_>>(),
        vec![uids[0], added]
    );
    // 読むだけのセットは、外れたまま触れない
    s.sets.get_mut(0).unwrap().read_only = Some("試験".into());
    let rows = s.export_set_rows();
    assert_eq!(
        (rows[0].checked, rows[0].enabled, rows[0].read_only),
        (false, false, true)
    );
    assert_eq!(s.export_checked_uids(), vec![added]);
}

#[test]
fn the_checks_come_back_when_the_project_is_replaced() {
    let mut s = two_sets();
    s.apply(Action::Export(ExportAction::OpenWindow));
    form(&mut s, ExportForm::Hdrp);
    let uids = set_uids(&s);
    check(&mut s, uids[0], false);
    assert_eq!(s.export_checked_uids(), vec![uids[1]]);
    // ウィンドウを開いたままプロジェクトを替えても、前のプロジェクトのチェックは使わない
    s.np_project_replaced();
    let uids = set_uids(&s);
    assert!(s.export_set_rows().iter().all(|r| r.checked));
    assert_eq!(s.export_checked_uids(), uids);
    // 開き直しても全部入り
    s.apply(Action::Export(ExportAction::CloseWindow));
    s.apply(Action::Export(ExportAction::OpenWindow));
    assert_eq!(s.export_checked_uids(), uids);
    // チェックは設定に入らない（設定は替わらない）
    let before = s.settings().clone();
    check(&mut s, uids[0], false);
    assert_eq!(s.settings().clone(), before);
}

#[test]
fn the_listed_files_are_the_files_written_for_every_template_and_with_a_set_unchecked() {
    for with_unchecked in [false, true] {
        for form_kind in ExportForm::ALL {
            let tag = format!("list-{}-{with_unchecked}", form_kind.key());
            let dir = Dir::new(&tag);
            let mut s = two_sets_with_more_channels();
            s.export.padding = 0;
            let uids = set_uids(&s);
            s.apply(Action::Export(ExportAction::OpenWindow));
            form(&mut s, form_kind);
            if form_kind.writes_file() {
                choose(&mut s, &dir.0.join("picked.png"));
            } else {
                choose(&mut s, &dir.0);
            }
            if with_unchecked {
                check(&mut s, uids[0], false);
            }
            let names = preview_names(&mut s);
            assert!(!names.is_empty(), "{tag}");
            s.apply(Action::Export(ExportAction::Run));
            s.wait_export();
            assert_eq!(
                names,
                dir.files(),
                "{tag}: 書く前の一覧と書いたファイルが同じ"
            );
        }
    }
}

#[test]
fn the_listed_files_show_the_color_space_and_mark_the_files_that_already_exist() {
    let dir = Dir::new("list-space");
    let mut s = two_sets_with_more_channels();
    s.apply(Action::Export(ExportAction::OpenWindow));
    form(&mut s, ExportForm::UnityStandard);
    choose(&mut s, &dir.0);
    let first = s.sets.get(0).unwrap().name.clone();
    let space = |s: &mut AppState, suffix: &str| {
        s.export_preview()
            .files
            .into_iter()
            .find(|f| f.name.ends_with(&format!("_{suffix}.png")))
            .map(|f| f.srgb)
    };
    assert_eq!(space(&mut s, "Albedo"), Some(true));
    assert_eq!(space(&mut s, "Emission"), Some(true));
    assert_eq!(space(&mut s, "MetallicSmoothness"), Some(false));
    assert!(s.export_preview().files.iter().all(|f| !f.exists));
    // もうあるファイルには印が付き、ほかには付かない
    std::fs::write(dir.0.join(format!("Texture_{first}_Albedo.png")), b"old").unwrap();
    // 外から足されたファイルは、調べ直し（開いたとき・書き終えたとき・約 1 秒ごと）で一覧に出る
    s.export.window.exists.invalidate();
    let marked: Vec<String> = s
        .export_preview()
        .files
        .into_iter()
        .filter(|f| f.exists)
        .map(|f| f.name)
        .collect();
    assert_eq!(marked, [format!("Texture_{first}_Albedo.png")]);
    // 色空間は PNG の 1 枚の一覧にも出る（Roughness は リニア）
    form(&mut s, ExportForm::ChannelPng);
    s.apply(Action::M2Ui(crate::m2::UiOp::PaintChannel(
        Channel::Roughness,
    )));
    let rows = s.export_preview().files;
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].srgb);
    assert!(rows[0].name.ends_with("_Roughness.png"), "{}", rows[0].name);
}

#[test]
fn a_problem_that_stops_the_export_is_shown_in_place_of_the_list() {
    // 書くものが無い
    let dir = Dir::new("list-problem");
    let mut s = AppState::new(64, 64);
    let layer = s.selected_layer.unwrap();
    s.doc
        .set_channel_enabled(layer, Channel::Color, false)
        .unwrap();
    s.apply(Action::Export(ExportAction::OpenWindow));
    form(&mut s, ExportForm::Hdrp);
    choose(&mut s, &dir.0);
    let preview = s.export_preview();
    assert!(preview.files.is_empty());
    assert!(
        preview
            .problem
            .as_deref()
            .is_some_and(|p| p.contains("書き出すものがありません")),
        "{preview:?}"
    );
    // 読むだけのセットは、1 枚の PNG にできない
    let mut s = two_sets();
    s.sets.get_mut(s.sets.current_index()).unwrap().read_only = Some("試験".into());
    s.apply(Action::Export(ExportAction::OpenWindow));
    let preview = s.export_preview();
    assert!(
        preview.files.is_empty() && preview.problem.is_some(),
        "{preview:?}"
    );
}

#[test]
fn the_padding_choices_are_named_in_both_languages() {
    use crate::lang::Lang;
    let names = |lang: Lang| -> Vec<String> {
        crate::settings::EXPORT_PADDINGS
            .iter()
            .map(|p| crate::prefs::padding_name(lang, *p))
            .collect()
    };
    assert_eq!(
        names(Lang::Ja),
        [
            "なし",
            "2 px 広げる",
            "4 px 広げる",
            "8 px 広げる",
            "16 px 広げる",
            "32 px 広げる",
            "64 px 広げる",
            "無限に広げる"
        ]
    );
    assert_eq!(
        names(Lang::En),
        [
            "No padding",
            "Dilation 2 px",
            "Dilation 4 px",
            "Dilation 8 px",
            "Dilation 16 px",
            "Dilation 32 px",
            "Dilation 64 px",
            "Dilation infinite"
        ]
    );
    assert_eq!(
        crate::settings::setting_name(Lang::Ja, "export_padding"),
        "書き出しのパディング"
    );
    assert_eq!(
        crate::settings::setting_name(Lang::En, "export_padding"),
        "Export padding"
    );
}

#[test]
fn the_listed_files_are_looked_up_on_disk_only_when_what_they_depend_on_changes() {
    let dir = Dir::new("list-lookups");
    let other = Dir::new("list-lookups-other");
    let mut s = two_sets_with_more_channels();
    s.export.padding = 0;
    let uids = set_uids(&s);
    s.apply(Action::Export(ExportAction::OpenWindow));
    form(&mut s, ExportForm::UnityStandard);
    choose(&mut s, &dir.0);
    let checks = |s: &AppState| s.export.window.exists_checks();
    let start = checks(&s);
    // 描くたび（毎フレーム）にディスクを調べない
    for _ in 0..30 {
        let _ = s.export_preview();
    }
    assert_eq!(checks(&s) - start, 1);
    // 出力先が替わると調べ直す
    choose(&mut s, &other.0);
    let _ = s.export_preview();
    let _ = s.export_preview();
    assert_eq!(checks(&s) - start, 2);
    // 書くファイルの並びが替わる（チェックを外す）と調べ直す
    check(&mut s, uids[1], false);
    let _ = s.export_preview();
    let _ = s.export_preview();
    assert_eq!(checks(&s) - start, 3);
    // ウィンドウを開き直すと調べ直す
    s.apply(Action::Export(ExportAction::CloseWindow));
    s.apply(Action::Export(ExportAction::OpenWindow));
    let _ = s.export_preview();
    assert_eq!(checks(&s) - start, 4);
    // 書き終えると調べ直し、書いたファイルに印が付く
    assert!(s.export_preview().files.iter().all(|f| !f.exists));
    s.apply(Action::Export(ExportAction::Run));
    s.wait_export();
    let after = s.export_preview();
    assert_eq!(checks(&s) - start, 5);
    assert!(!after.files.is_empty() && after.files.iter().all(|f| f.exists));
    // 同じ入力を続けて描いても、もう調べない
    for _ in 0..10 {
        let _ = s.export_preview();
    }
    assert_eq!(checks(&s) - start, 5);
}

#[test]
fn a_current_channel_png_of_a_read_only_set_refuses_with_the_read_only_reason() {
    let dir = Dir::new("read-only-png");
    let mut s = two_sets();
    s.sets.get_mut(s.sets.current_index()).unwrap().read_only = Some("試験".into());
    s.apply(Action::Export(ExportAction::OpenWindow));
    choose(&mut s, &dir.0.join("one.png"));
    let why = crate::lang::refusals::read_only_set(s.lang, "試験");
    assert_eq!(s.export_no_set_reason(), why);
    s.apply(Action::Export(ExportAction::Run));
    assert_eq!(s.message, why);
    assert!(!s.export.is_exporting() && dir.files().is_empty());
    // ほかの出力テンプレートでは、読むだけの理由ではなく、書くセットが無い断り（全部外したとき）
    form(&mut s, ExportForm::Hdrp);
    for uid in set_uids(&s) {
        check(&mut s, uid, false);
    }
    assert_eq!(s.export_no_set_reason(), no_sets_message(s.lang));
}

#[test]
fn a_direct_export_with_no_set_to_write_is_refused_as_having_no_set() {
    let dir = Dir::new("no-sets");
    let mut s = two_sets();
    let message = no_sets_message(s.lang);
    // 選びが空・無い uid だけ（テンプレートでもチャンネルごとでも）
    for sets in [Some(vec![]), Some(vec![9999])] {
        s.apply(Action::Export(ExportAction::TemplateTo {
            id: "unity-standard".into(),
            dir: dir.0.clone(),
            sets: sets.clone(),
        }));
        assert_eq!(s.message, message);
        s.apply(Action::Export(ExportAction::ChannelsTo {
            dir: dir.0.clone(),
            sets,
        }));
        assert_eq!(s.message, message);
        assert!(!s.export.is_exporting() && dir.files().is_empty());
    }
    // 全部読むだけのときも同じ
    for i in 0..s.sets.len() {
        s.sets.get_mut(i).unwrap().read_only = Some("試験".into());
    }
    export(&mut s, "unity-standard", &dir.0);
    assert_eq!(s.message, message);
    assert!(!s.export.is_exporting() && dir.files().is_empty());
}
