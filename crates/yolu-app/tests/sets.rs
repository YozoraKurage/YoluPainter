//! テクスチャセット（マテリアルごとの文書）のパネル・切り替え・読むだけのセット・.ylp を開く（egui_kittest）。
mod common;

use common::*;
use egui::Key;
use egui_kittest::kittest::{NodeT, Queryable};
use yolu_app::engine::{composite_pixel, layer_has_pixels};
use yolu_app::lang::Lang;
use yolu_app::panels::texture_sets::bake_entrance;
use yolu_app::sets::{MaterialRef, TextureSets};
use yolu_app::state::{blank_document, Action, DialogRequest};
use yolu_protocol::{
    channel, ChannelRoute, MaterialInfo, MaterialKey, MeshData, Model, Submesh, TextureProperty,
};

/// マテリアル（name が None ならマテリアルの無いスロット）。size はメインのテクスチャの大きさ。
fn material(name: Option<&str>, size: u32, color_route: bool) -> MaterialInfo {
    MaterialInfo {
        key: match name {
            Some(n) => MaterialKey::Material {
                name: n.into(),
                asset: None,
            },
            None => MaterialKey::Unassigned,
        },
        shader: "Standard".into(),
        textures: vec![TextureProperty {
            name: "_MainTex".into(),
            width: size,
            height: size,
        }],
        routes: if color_route {
            vec![ChannelRoute {
                channel: channel::COLOR,
                property: "_MainTex".into(),
            }]
        } else {
            vec![]
        },
    }
}

/// 三角形 1 つのメッシュに、マテリアルごとのサブメッシュ。
fn model(materials: Vec<MaterialInfo>) -> Model {
    let n = materials.len() as u32;
    Model {
        generation: 1,
        name: "試し".into(),
        materials,
        meshes: vec![MeshData {
            key: "0".into(),
            name: "Body".into(),
            skinned: false,
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            submeshes: (0..n)
                .map(|m| Submesh {
                    material: m,
                    indices: vec![0, 1, 2],
                })
                .collect(),
        }],
    }
}

fn give_model(h: &mut egui_kittest::Harness<'_, yolu_app::YoluApp>, materials: Vec<MaterialInfo>) {
    // Live Link が受けたときと同じ道（記録・結び付け・3D の形）。つながりの外なので Unity には出さない
    h.state_mut()
        .load_live_link_model(&model(materials))
        .unwrap();
    h.run();
}

fn set_names(h: &egui_kittest::Harness<'_, yolu_app::YoluApp>) -> Vec<String> {
    h.state()
        .state
        .sets
        .iter()
        .map(|s| s.name.clone())
        .collect()
}

#[test]
fn the_list_switches_the_canvas_and_the_layers_per_material() {
    let mut h = app(1280.0, 800.0, 256);
    let c = canvas_rect(&h).center();
    // モデルの前に描いたものは、最初のスロットのマテリアルのセットに残る
    drag(&mut h, &[offset(c, -30.0, 0.0), offset(c, 30.0, 0.0)]);
    give_model(
        &mut h,
        vec![
            material(Some("Skin"), 1024, true),
            material(Some("Hair"), 512, true),
            material(None, 0, false),
        ],
    );
    assert_eq!(set_names(&h), ["Skin", "Hair", "Unassigned"]);
    assert_eq!(
        h.state().state.doc.width(),
        256,
        "描いたセットの大きさは変えない"
    );
    assert_eq!(canvas_pixel(&h, c), [0, 0, 0, 255]);

    // Hair を選ぶと、キャンバスとレイヤーはその文書
    h.get_by_label("Hair").click();
    h.run();
    let s = &h.state().state;
    assert_eq!(s.sets.current_index(), 1);
    assert_eq!((s.doc.width(), s.doc.height()), (512, 512));
    assert!(h.state().display().stats.last_rebuilt || h.state().display().stats.total_tiles > 0);
    assert_eq!(canvas_pixel(&h, c), [0, 0, 0, 0], "Hair はまだ透明");
    let c = canvas_rect(&h).center();
    drag(&mut h, &[offset(c, 0.0, -20.0), offset(c, 0.0, 20.0)]);
    assert_eq!(canvas_pixel(&h, c), [0, 0, 0, 255]);
    h.get_by_label("新規レイヤー").click();
    h.run();
    let s = &h.state().state;
    assert_eq!(s.doc.layers().len(), 2);
    assert_eq!(s.set_doc(0).layers().len(), 1, "レイヤーは今のセットに足す");

    // Skin へ戻ると、その絵と選んでいたレイヤー
    h.get_by_label("Skin").click();
    h.run();
    let s = &h.state().state;
    assert_eq!(s.doc.width(), 256);
    assert_eq!(s.doc.layers().len(), 1);
    assert_eq!(canvas_pixel(&h, c), [0, 0, 0, 255]);
    // 取り消しはセットごと（Skin の線だけを戻す）
    h.state_mut().state.apply(Action::Undo);
    h.run();
    let s = &h.state().state;
    assert_eq!(composite_pixel(&s.doc, 128, 128), [0, 0, 0, 0]);
    assert!(
        layer_has_pixels(&s.set_doc(1).layers()[0]),
        "Hair の線は残る"
    );
}

#[test]
fn rename_hide_and_the_header_show_the_set() {
    let mut h = app(1280.0, 800.0, 256);
    give_model(
        &mut h,
        vec![
            material(Some("Skin"), 256, true),
            material(Some("Hair"), 256, true),
        ],
    );
    // ダブルクリックで名前を変える
    let row = h.get_by_label("Hair").rect();
    let at = offset(row.left_center(), 60.0, 0.0);
    for _ in 0..2 {
        press(&h, at, egui::PointerButton::Primary);
        release(&h, at, egui::PointerButton::Primary);
        h.step();
    }
    h.run();
    assert_eq!(
        h.state().state.renaming_set,
        h.state().state.sets.get(1).map(|s| s.uid)
    );
    key(&h, Key::A, egui::Modifiers::COMMAND);
    h.event(egui::Event::Text("髪".into()));
    key(&h, Key::Enter, egui::Modifiers::NONE);
    h.run();
    let hair = h.state().state.sets.get(1).unwrap();
    assert_eq!(hair.name, "髪");
    assert!(!hair.auto_name);
    assert!(h.state().state.modified);
    // 目
    let uid = hair.uid;
    let eyes: Vec<egui::Rect> = h
        .get_all_by_label("隠す（3D ビューと Unity に見せない）")
        .map(|n| n.rect())
        .collect();
    assert_eq!(eyes.len(), 2);
    click(&mut h, eyes[1].center());
    let hair = h.state().state.sets.by_uid(uid).unwrap();
    assert!(!hair.visible);
    h.snapshot("texture_sets_renamed_hidden");
}

#[test]
fn every_state_of_a_set_has_its_icon() {
    let mut h = app(1280.0, 800.0, 256);
    let (body, _) = blank_document(1024, 1024);
    let (face, _) = blank_document(512, 512);
    let (old, _) = blank_document(256, 128);
    let (sets, doc) = TextureSets::from_parts(
        vec![
            (
                "00000000-0000-4000-8000-000000000001".into(),
                "Body".into(),
                MaterialRef::Material {
                    name: "Body".into(),
                    asset: None,
                },
                None,
                body,
            ),
            (
                "00000000-0000-4000-8000-000000000002".into(),
                "Face".into(),
                MaterialRef::Material {
                    name: "Face".into(),
                    asset: None,
                },
                Some("マスクのあるレイヤーがあります".into()),
                face,
            ),
            (
                "00000000-0000-4000-8000-000000000003".into(),
                "Old".into(),
                MaterialRef::Material {
                    name: "Old".into(),
                    asset: None,
                },
                None,
                old,
            ),
        ],
        0,
    );
    h.state_mut().state.replace_sets(sets, doc);
    give_model(
        &mut h,
        vec![
            material(Some("Body"), 1024, true),
            material(Some("Face"), 512, true),
            material(Some("Hair"), 512, false),
        ],
    );
    assert_eq!(set_names(&h), ["Body", "Face", "Old", "Hair"]);
    let s = &h.state().state;
    assert_eq!(
        s.sets.get(2).unwrap().bound,
        None,
        "モデルに無い Old は残る"
    );
    assert_eq!(s.set_for_material(2), Some(3));
    let uid = s.sets.get(0).unwrap().uid;
    h.state_mut().state.apply(Action::ToggleSetVisible(uid));
    h.run();
    let icons = |i| yolu_app::panels::texture_sets::set_state(&h.state().state, i).map(|s| s.icon);
    assert_eq!(icons(0), Some("visibility_off"));
    assert_eq!(icons(1), Some("lock"));
    assert_eq!(icons(2), Some("link_off"));
    assert_eq!(icons(3), Some("warning"));
    h.snapshot("texture_sets_states");
}

#[test]
fn read_only_sets_refuse_painting_and_layer_edits() {
    let mut h = app(1280.0, 800.0, 128);
    let (doc, _) = blank_document(128, 128);
    let (sets, doc) = TextureSets::from_parts(
        vec![(
            "00000000-0000-4000-8000-000000000009".into(),
            "読むだけ".into(),
            MaterialRef::PendingSlot(0),
            Some("フィルターのあるレイヤーがあります".into()),
            doc,
        )],
        0,
    );
    h.state_mut().state.replace_sets(sets, doc);
    h.run();
    let c = canvas_rect(&h).center();
    drag(&mut h, &[offset(c, -20.0, 0.0), offset(c, 20.0, 0.0)]);
    let s = &h.state().state;
    assert_eq!(canvas_pixel(&h, c), [0, 0, 0, 0]);
    assert!(s.message.contains("読むだけ"), "{}", s.message);
    assert!(!s.doc.can_undo());
    for action in [Action::NewLayer, Action::DeleteLayer, Action::Undo] {
        h.state_mut().state.apply(action);
        assert_eq!(h.state().state.doc.layers().len(), 1);
        assert!(h.state().state.message.contains("フィルター"));
    }
    let uid = h.state().state.sets.current().uid;
    assert!(h.state_mut().state.rename_set(uid, "x").is_err());
    h.state_mut().state.apply(Action::StartRenameSet(uid));
    assert_eq!(h.state().state.renaming_set, None);
    // レイヤーのパネルの操作は押せない
    assert!(h
        .get_by_label("新規レイヤー")
        .accesskit_node()
        .is_disabled());
}

// ───────── 下の帯のベイクのボタン ─────────

/// 試しの立方体のセットを、位置のマップ 1 枚だけ CPU で焼いた状態にする（窓の外の、最小のベイク）。
fn bake_current_set(h: &mut egui_kittest::Harness<'_, yolu_app::YoluApp>) {
    let s = &mut h.state_mut().state;
    s.bake.backend = yolu_app::bake::BakeBackend::Cpu;
    s.bake.settings.maps = vec![yolu_core::mesh_maps::MeshMapKind::Position];
    s.bake.settings.padding = 4;
    s.apply(Action::Bake(yolu_app::bake::BakeAction::Start));
    s.wait_bake();
    h.run();
}

#[test]
fn the_bake_button_sits_in_the_texture_set_bar_next_to_the_configuration_and_opens_the_window() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 800.0, 256);
        h.state_mut().state.lang = lang;
        h.run();
        let name = lang.pick("メッシュマップをベイク…", "Bake Mesh Maps…");
        let entrance = bake_entrance(&h.state().state);
        // まだ焼いていないセット: 印と、理由（ツールチップの 2 行目。名前の行の後）
        assert!(entrance.marked, "{lang:?}");
        let lines: Vec<&str> = entrance.tooltip.lines().collect();
        assert_eq!(lines.len(), 2, "{lang:?}: {lines:?}");
        assert_eq!(lines[0], name);
        assert_eq!(has_japanese(lines[1]), lang == Lang::Ja, "{}", lines[1]);
        assert_plain(&format!("{lang:?} ベイクのボタン"), &entrance.tooltip);
        // 帯の中: 足す・消すと同じ行で、プロジェクト設定のすぐ左
        let bake = h.get_by_label(&entrance.tooltip).rect();
        let configure = h
            .get_by_label(lang.pick("プロジェクト設定…", "Project Configuration…"))
            .rect();
        assert_eq!(bake.center().y, configure.center().y, "{lang:?}");
        assert!(bake.right() <= configure.left(), "{bake:?} {configure:?}");
        assert!(configure.left() - bake.right() < 12.0, "{bake:?} {configure:?}");
        // 押すとベイクの窓が開く（メニューの項目と同じ操作）
        assert!(h.state().state.bake.window.is_none());
        click(&mut h, bake.center());
        assert!(h.state().state.bake.window.is_some(), "{lang:?}");
    }
}

/// テクスチャセットのパネルだけを `width` の幅で描く。
fn set_panel(width: f32, lang: Lang) -> egui_kittest::Harness<'static, yolu_app::state::AppState> {
    let mut state = yolu_app::state::AppState::new(64, 64);
    state.lang = lang;
    let mut ready = false;
    let mut h = gpu_thread::builder()
        .with_size(egui::vec2(width, 200.0))
        .with_render_options(render_options())
        .wgpu()
        .build_ui_state(
            move |ui, state| {
                if !ready {
                    yolu_app::YoluApp::setup(ui.ctx());
                    ready = true;
                    ui.ctx().request_repaint();
                    return;
                }
                yolu_app::panels::texture_sets::show(ui, state)
            },
            state,
        );
    h.run();
    h
}

#[test]
fn the_bar_buttons_never_overlap_and_the_bake_button_leaves_when_the_panel_is_too_narrow() {
    /// ベイクのボタンを出す、帯の幅の境（足す・消す・ベイク・設定の 4 つが並ぶ幅）。
    const BAKE_MIN_BAR_WIDTH: f32 = 128.0;
    for lang in Lang::ALL {
        let configure = lang.pick("プロジェクト設定…", "Project Configuration…");
        let add = lang.pick(
            "空のテクスチャセットを足す（今のセットと同じ大きさ・チャンネル）",
            "Add an empty texture set (same size and channels as this one)",
        );
        let remove_label = |h: &egui_kittest::Harness<'_, yolu_app::state::AppState>| {
            h.get_all_by_label_contains(lang.pick("プロジェクトには少なくとも", "A project keeps at least"))
                .next()
                .expect("消すボタン")
                .rect()
        };
        // 帯の幅: 足すボタンは左端から 4、設定のボタンは右端の 4 手前に終わる（パネルの縁の分だけ窓の幅より狭い）
        let bar_width = |h: &egui_kittest::Harness<'_, yolu_app::state::AppState>| {
            h.get_by_label(configure).rect().right() - h.get_by_label(add).rect().left() + 8.0
        };
        let edge = 400.0 - bar_width(&set_panel(400.0, lang));
        assert!(edge >= 0.0, "{lang:?}: 帯が窓より広い");
        for bar in [
            90.0_f32, 110.0, 120.0, 126.0, 127.0, 127.5, 128.0, 128.5, 129.0, 135.0, 160.0, 235.0, 400.0,
        ] {
            let h = set_panel(bar + edge, lang);
            assert!((bar_width(&h) - bar).abs() < 0.01, "{lang:?} 帯 {bar}: {}", bar_width(&h));
            let bake = h
                .query_all_by_label_contains(lang.pick("メッシュマップをベイク…", "Bake Mesh Maps…"))
                .next()
                .map(|n| n.rect());
            // 境は 128 px ちょうど: 127.5 までは出さず、128 からは出す
            assert_eq!(bake.is_some(), bar >= BAKE_MIN_BAR_WIDTH, "{lang:?} 帯 {bar}");
            let mut rects = vec![
                h.get_by_label(add).rect(),
                remove_label(&h),
                h.get_by_label(configure).rect(),
            ];
            rects.extend(bake);
            // 出ているとき（出ていないときも、残りの 3 つが）重ならない（縁が接するのは重なりではない）
            let overlap = |a: &egui::Rect, b: &egui::Rect| {
                a.min.x < b.max.x && b.min.x < a.max.x && a.min.y < b.max.y && b.min.y < a.max.y
            };
            for (i, a) in rects.iter().enumerate() {
                for b in &rects[i + 1..] {
                    assert!(!overlap(a, b), "{lang:?} 帯 {bar}: {a:?} {b:?}");
                }
            }
        }
    }
}

#[test]
fn the_bake_mark_follows_the_current_set_and_goes_once_that_set_is_baked() {
    let mut h = app(1280.0, 800.0, 128);
    h.state_mut().state.apply(Action::LoadDemoModel);
    h.run();
    let marked = |h: &egui_kittest::Harness<'_, yolu_app::YoluApp>| {
        bake_entrance(&h.state().state).marked
    };
    assert!(marked(&h), "焼く前");
    bake_current_set(&mut h);
    let baked = bake_entrance(&h.state().state);
    assert!(!baked.marked, "焼いたセットに印は無い");
    assert_eq!(baked.tooltip, "メッシュマップをベイク…", "理由の行も無い");
    // ボタンは焼いたあとも窓を開く
    let at = h.get_by_label(&baked.tooltip).rect().center();
    click(&mut h, at);
    assert!(h.state().state.bake.window.is_some());
    h.state_mut().state.apply(Action::Bake(yolu_app::bake::BakeAction::CloseWindow));
    // 足した空のセットは、ほかのセットが焼けていても印が付く。戻せば消える
    let first = h.state().state.sets.current().uid;
    h.state_mut()
        .state
        .apply(Action::Project(yolu_app::newproject::NpAction::AddSet));
    let second = h.state().state.sets.iter().last().unwrap().uid;
    assert_ne!(first, second);
    h.state_mut().state.apply(Action::SelectSet(second));
    h.run();
    assert!(marked(&h), "足したセット");
    h.state_mut().state.apply(Action::SelectSet(first));
    h.run();
    assert!(!marked(&h), "焼いたセットへ戻る");
}

/// テクスチャセットの一覧と下の帯（と、その下に出るツールチップ）を切り出して撮る。
fn snapshot_set_bar(h: &mut egui_kittest::Harness<'_, yolu_app::YoluApp>, lang: Lang, name: &str) {
    let add = h
        .get_by_label(lang.pick(
            "空のテクスチャセットを足す（今のセットと同じ大きさ・チャンネル）",
            "Add an empty texture set (same size and channels as this one)",
        ))
        .rect();
    let configure = h
        .get_by_label(lang.pick("プロジェクト設定…", "Project Configuration…"))
        .rect();
    let image = h.render().expect("描画");
    // 左へは、帯の左端より広く（右に寄った帯のボタンのツールチップは左へ伸びる）
    let (left, top) = ((configure.right() - 330.0).max(0.0) as u32, (add.top() - 100.0).max(0.0) as u32);
    let right = ((configure.right() + 8.0) as u32).min(image.width());
    let bottom = ((add.bottom() + 70.0) as u32).min(image.height());
    let cropped =
        image::imageops::crop_imm(&image, left, top, right - left, bottom - top).to_image();
    egui_kittest::image_snapshot(&cropped, name);
}

#[test]
fn snapshot_the_bake_button_marked_with_its_reason_and_unmarked_once_baked() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 800.0, 128);
        h.state_mut().state.lang = lang;
        h.state_mut().state.apply(Action::LoadDemoModel);
        h.run();
        let at = h
            .get_by_label(&bake_entrance(&h.state().state).tooltip)
            .rect()
            .center();
        hover_and_wait(&mut h, at);
        snapshot_set_bar(
            &mut h,
            lang,
            &format!("texture_sets_bake_button_{}", lang.pick("ja", "en")),
        );
    }
    // ポインタを乗せない 2 枚（印の有る・無い）。アイコンと印がポインタに隠れない
    let mut h = app(1280.0, 800.0, 128);
    h.state_mut().state.apply(Action::LoadDemoModel);
    h.run();
    snapshot_set_bar(&mut h, Lang::Ja, "texture_sets_bake_button_marked");
    bake_current_set(&mut h);
    snapshot_set_bar(&mut h, Lang::Ja, "texture_sets_bake_button_baked");
}

#[test]
fn the_bake_button_cannot_be_pressed_while_drawing() {
    let mut h = app(1280.0, 800.0, 128);
    let entrance = bake_entrance(&h.state().state);
    assert!(!h
        .get_by_label(&entrance.tooltip)
        .accesskit_node()
        .is_disabled());
    let layer = h.state().state.selected_layer.unwrap();
    let brush = h.state().state.stroke_settings(false);
    let stroke = h.state_mut().state.doc.begin_stroke(layer, &brush).unwrap();
    h.run();
    assert!(h
        .get_by_label(&entrance.tooltip)
        .accesskit_node()
        .is_disabled());
    h.state_mut().state.doc.cancel_stroke(stroke);
}

#[test]
fn the_fire_icon_is_loaded_and_listed_in_the_licence_tables() {
    use sha2::{Digest, Sha256};
    let ctx = egui::Context::default();
    assert!(yolu_app::ui::icons::Icons::load(&ctx).has("local_fire_department"));
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let png = std::fs::read(root.join("assets/icons/local_fire_department.png")).unwrap();
    let digest: String = Sha256::digest(&png).iter().map(|b| format!("{b:02x}")).collect();
    // 許諾の表: 確認済みの PNG の SHA-256 と、元の名前の対応（JSON として読む。改行や字下げに頼らない）
    let reviewed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("../../tools/licenses-reviewed.json")).unwrap()).unwrap();
    let listed = reviewed["bundled"]["yolu-app"]
        .as_array()
        .expect("bundled の yolu-app の配列")
        .iter()
        .flat_map(|entry| entry["files"].as_array().into_iter().flatten())
        .find(|file| file["path"] == "crates/yolu-app/assets/icons/local_fire_department.png")
        .expect("licenses-reviewed.json に載っていない");
    assert_eq!(listed["sha256"], digest.as_str(), "載っている SHA-256 と PNG が違う");
    let notices = std::fs::read_to_string(root.join("assets/icons/THIRD-PARTY-NOTICES.md")).unwrap();
    assert!(notices.contains("| `local_fire_department` | fluent | `fire` | regular |"));
}

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../yolu-io/tests/fixtures")
        .join(name)
}

/// 試験ごとの一時フォルダ（終わったら消す）。
struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new(tag: &str) -> TempDir {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "yolu-app-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 保存した .ylp（試験用。yolu-io で読む）。
fn read_project(path: &std::path::Path) -> yolu_io::Project {
    yolu_io::Project::read(&std::fs::read(path).unwrap()).unwrap()
}

/// .ylp の 1 つのエントリ（形式 7 の並びへ移した名前で）。
fn zip_entry(path: &std::path::Path, name: &str) -> Vec<u8> {
    read_project(path).migrated_entries()[name].to_vec()
}

fn backups(path: &std::path::Path) -> Vec<std::path::PathBuf> {
    let dir = path.with_file_name(format!(
        "{}-backups~",
        path.file_name().unwrap().to_string_lossy()
    ));
    std::fs::read_dir(dir)
        .map(|d| d.map(|e| e.unwrap().path()).collect())
        .unwrap_or_default()
}

#[test]
fn opening_paints_and_saving_rewrites_only_the_painted_set() {
    let dir = TempDir::new("save");
    let path = dir.0.join("format6.ylp");
    std::fs::copy(fixture("format6.ylp"), &path).unwrap();
    let before = std::fs::read(&path).unwrap();
    let original = read_project(&path);
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut().state.apply(Action::OpenProject(path.clone()));
    h.run();
    assert_eq!(set_names(&h), ["Skin", "Cloth", "Skin 2"]);
    let s = &h.state().state;
    assert_eq!(s.project_name, "format6");
    assert!(!s.modified);
    assert_eq!(
        s.project.as_ref().unwrap().format(),
        6,
        "開いただけでは形式を変えない"
    );
    for (i, set) in s.sets.iter().enumerate() {
        assert!(set.read_only.is_none(), "core が持つ中身だけなので描ける");
        assert_eq!(
            set.material,
            MaterialRef::PendingSlot(i as u16),
            "形式 6 のスロットの番号"
        );
        assert_eq!(s.set_doc(i).width(), 512);
    }
    // 見せている絵は、保存した合成の PNG と同じ画素（PNG は上の行から、文書は下の行から）
    let png = image::load_from_memory(&zip_entry(
        &path,
        &format!("sets/{}/composite/Color.png", s.sets.get(0).unwrap().id),
    ))
    .unwrap()
    .to_rgba8();
    let (x, y) = (0..512 * 512)
        .map(|i| (i % 512, i / 512))
        .find(|&(x, y)| png.get_pixel(x, y)[3] == 255)
        .expect("不透明の画素がある");
    assert_eq!(composite_pixel(&s.doc, x, 511 - y), png.get_pixel(x, y).0);

    // Skin に描いて Ctrl+S
    let c = canvas_rect(&h).center();
    h.state_mut().state.color.set_main([1.0, 0.0, 1.0, 1.0]);
    drag(&mut h, &[offset(c, -20.0, 0.0), offset(c, 20.0, 0.0)]);
    assert!(h.state().state.modified);
    let painted = composite_pixel(&h.state().state.doc, 256, 256);
    assert_eq!(painted, [255, 0, 255, 255]);
    key(&h, Key::S, egui::Modifiers::COMMAND);
    h.run();
    let s = &h.state().state;
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    assert!(s.message.contains("書き直した正本 1"), "{}", s.message);
    assert!(!s.modified);
    let saved = read_project(&path);
    assert_eq!(saved.info().format, 7);
    assert_eq!(
        saved.info().saved_by.as_ref().unwrap().app,
        "YoluPainter-rs"
    );
    assert_eq!(saved.info().created_by, original.info().created_by);
    let doc = saved.sets()[0].document.to_core().unwrap();
    assert_eq!(
        doc.composite_pixel(yolu_app::engine::Channel::Color, 256, 256)
            .unwrap(),
        yolu_app::engine::Rgba8::new(255, 0, 255, 255)
    );
    assert_eq!(doc.id(), s.doc.id(), "文書とレイヤーの ID を保つ");
    for set in &original.sets()[1..] {
        let n = format!("sets/{}/document.utpaint", set.id);
        assert_eq!(
            saved.migrated_entries()[&n],
            original.migrated_entries()[&n],
            "描かなかったセットの正本はバイト列のまま"
        );
    }
    let backup = backups(&path);
    assert_eq!(backup.len(), 1, "前の版を 1 つ残す");
    assert_eq!(std::fs::read(&backup[0]).unwrap(), before);

    // 開き直すと、描いた所がある
    let mut again = app(1280.0, 800.0, 256);
    again
        .state_mut()
        .state
        .apply(Action::OpenProject(path.clone()));
    again.run();
    assert_eq!(
        composite_pixel(&again.state().state.doc, 256, 256),
        [255, 0, 255, 255]
    );
    // 2 回目の保存は描いていなければ正本を書き直さない
    h.state_mut().state.apply(Action::SaveProject);
    assert!(
        h.state().state.message.contains("書き直した正本 0"),
        "{}",
        h.state().state.message
    );
}

#[test]
fn sets_core_cannot_hold_are_read_only_and_kept_byte_for_byte() {
    // format4.ylp の最初のセットの文書を、手動の ID の色を持つ正本（core に無い。ロックは core が持つ）に差し替える。2 つ目の Trim は core が持つ中身だけ
    let dir = TempDir::new("readonly");
    let path = dir.0.join("format4.ylp");
    let base = read_project(&fixture("format4.ylp"));
    let rich =
        yolu_io::NativeDocument::read(&std::fs::read(fixture("native-rich-v21.utpaint")).unwrap())
            .unwrap();
    let first = base.sets()[0].id.clone();
    std::fs::write(
        &path,
        base.with_document(&first, &rich)
            .unwrap()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    let original = read_project(&path);
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut().state.apply(Action::OpenProject(path.clone()));
    h.run();
    let s = &h.state().state;
    assert_eq!(s.sets.len(), 2);
    let reason = s
        .sets
        .get(0)
        .unwrap()
        .read_only
        .as_deref()
        .expect("手動の ID の色があるので読むだけ");
    assert!(reason.contains("core で扱えない中身"), "{reason}");
    assert!(reason.contains("手動"), "{reason}");
    assert!(!reason.contains("ロック"), "ロックは core が持つ: {reason}");
    assert!(
        !reason.contains("フィルター"),
        "フィルターは core が持つ: {reason}"
    );
    assert!(
        s.sets.get(1).unwrap().read_only.is_none(),
        "Trim は core が持つ中身だけ"
    );
    assert!(s.message.contains("読むだけのセット 1"), "{}", s.message);
    // 描けない
    let c = canvas_rect(&h).center();
    let was = canvas_pixel(&h, c);
    drag(&mut h, &[offset(c, -20.0, 5.0), offset(c, 20.0, 5.0)]);
    assert_eq!(canvas_pixel(&h, c), was);
    assert!(h.state().state.message.contains("読むだけ"));
    // 保存しても、正本・ほかのチャンネルの合成・資源はバイト列のまま（形式だけ 7 へ）
    h.state_mut().state.apply(Action::SaveProject);
    assert!(
        h.state().state.message.starts_with("保存しました"),
        "{}",
        h.state().state.message
    );
    let saved = read_project(&path);
    assert_eq!(saved.info().format, 7);
    for (name, bytes) in original.migrated_entries() {
        if name == "project.json" {
            continue; // 形式 7 の material の鍵に書き直す
        }
        assert_eq!(saved.migrated_entries().get(name), Some(bytes), "{name}");
    }
}

/// format4.ylp の 2 つのセットの文書を、効果のある C# の正本 2 つに差し替えた .ylp（評価の入力は app が渡さない）。
fn effects_project(path: &std::path::Path, first: &str, second: &str) {
    let base = read_project(&fixture("format4.ylp"));
    let native = |n: &str| {
        yolu_io::NativeDocument::read(&std::fs::read(fixture(&format!("{n}.utpaint"))).unwrap())
            .unwrap()
    };
    let ids: Vec<String> = base.sets().iter().map(|s| s.id.clone()).collect();
    let project = base
        .with_document(&ids[0], &native(first))
        .unwrap()
        .with_document(&ids[1], &native(second))
        .unwrap();
    std::fs::write(path, project.to_bytes().unwrap()).unwrap();
}

#[test]
fn sets_with_effects_that_cannot_work_yet_are_read_only_and_working_ones_are_editable() {
    // メッシュマップ・画像を使う Generator・画像の塗りつぶしは、入力がそろわないあいだは効かない。入力のいらないフィルターだけの文書は編集できる
    let dir = TempDir::new("effects-readonly");
    let path = dir.0.join("effects.ylp");
    effects_project(&path, "effects-generators", "effects-filters");
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut().state.apply(Action::OpenProject(path.clone()));
    h.run();
    let s = &h.state().state;
    assert_eq!(s.sets.len(), 2);
    let reason = s
        .sets
        .get(0)
        .unwrap()
        .read_only
        .as_deref()
        .expect("入力がそろわないジェネレーターがあるので読むだけ");
    assert!(reason.contains("効果の入力がそろっていない"), "{reason}");
    assert!(reason.contains("マップがありません"), "{reason}");
    assert!(
        !reason.contains("焼いてください"),
        "理由は状態だけで、手順を言わない: {reason}"
    );
    assert!(
        s.sets.get(1).unwrap().read_only.is_none(),
        "フィルターだけの文書は評価されるので編集できる"
    );
    assert!(s.message.contains("読むだけのセット 1"), "{}", s.message);
    // 読むだけのセットは保存しても元の正本のまま、編集できるセットは書き直して効果を合成の PNG に入れる
    let original = read_project(&path);
    h.state_mut().state.apply(Action::SaveProject);
    assert!(
        h.state().state.message.starts_with("保存しました"),
        "{}",
        h.state().state.message
    );
    let saved = read_project(&path);
    let id0 = &original.sets()[0].id;
    for name in ["document.utpaint", "composite/Color.png"] {
        let entry = format!("sets/{id0}/{name}");
        assert_eq!(
            saved.migrated_entries().get(&entry),
            original.migrated_entries().get(&entry),
            "{entry}: 読むだけのセットは元のバイト列のまま"
        );
    }
}

#[test]
fn saving_names_the_effects_that_are_not_in_the_composite_png() {
    // 編集できるセットへ、マップが無いので効かない Generator を足してから保存する: 正本には設定が残り、合成の PNG に入らないことを言う
    let dir = TempDir::new("effects-note");
    let path = dir.0.join("effects.ylp");
    effects_project(&path, "effects-paths", "effects-filters");
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut().state.apply(Action::OpenProject(path.clone()));
    h.run();
    assert!(h.state().state.sets.get(0).unwrap().read_only.is_none());
    {
        use yolu_core::generator::{self, Settings};
        use yolu_core::{EffectSettings, FilterSpec, FilterTarget};
        let doc = h.state_mut().state.set_doc_mut(0);
        let layer = doc.layers()[0].id();
        let mut g = Settings::new(generator::Kind::Thickness);
        g.blend = generator::Blend::Replace;
        doc.add_filter(
            layer,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(g)).channels(&[yolu_core::Channel::Color]),
        )
        .unwrap();
    }
    h.state_mut().state.apply(Action::SaveProject);
    let message = h.state().state.message.clone();
    assert!(message.starts_with("保存しました"), "{message}");
    assert!(
        message.contains("効いていない効果 1 件は合成の PNG に入っていません"),
        "{message}"
    );
    assert!(
        message.contains("Thickness のマップがありません"),
        "{message}"
    );
    // 設定は正本に残る（開き直すと、効かない効果があるので読むだけで、理由に出る）
    let mut again = app(1280.0, 800.0, 256);
    again.state_mut().state.apply(Action::OpenProject(path));
    again.run();
    let reason = again
        .state()
        .state
        .sets
        .get(0)
        .unwrap()
        .read_only
        .clone()
        .expect("効かないジェネレーターが正本に残っている");
    assert!(reason.contains("厚み"), "{reason}");
}

/// C# が書いた M2 の文書（グループ・マスク・チャンネルごとの合成・調整・塗りつぶし）の 2 セットを入れた .ylp。
fn m2_project(path: &std::path::Path) -> Vec<yolu_io::NativeDocument> {
    let natives: Vec<_> = ["m2-groups", "m2-channels"]
        .iter()
        .map(|n| {
            yolu_io::NativeDocument::read(&std::fs::read(fixture(&format!("{n}.utpaint"))).unwrap())
                .unwrap()
        })
        .collect();
    let specs: Vec<_> = natives
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let core = n.to_core().unwrap();
            yolu_io::SetSpec {
                id: format!("00000000-0000-4000-8000-00000000010{i}"),
                name: format!("M2 {i}"),
                material: yolu_io::MaterialRef::Material {
                    name: format!("M2 {i}"),
                    asset: None,
                },
                document: Some(n.clone()),
                composites: yolu_io::composite_pngs(&core).unwrap(),
            }
        })
        .collect();
    let id = specs[0].id.clone();
    let project = yolu_io::Project::create(writer(), &specs, &id).unwrap();
    std::fs::write(path, project.to_bytes().unwrap()).unwrap();
    natives
}
fn writer() -> yolu_io::WriterInfo {
    yolu_io::WriterInfo {
        app: "試験の書き手".into(),
        version: "0.0.1".into(),
        unity: "standalone".into(),
    }
}

#[test]
fn sets_with_groups_masks_and_channel_blends_open_editable_and_save_back_without_losing_them() {
    let dir = TempDir::new("m2");
    let path = dir.0.join("m2.ylp");
    let natives = m2_project(&path);
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut().state.apply(Action::OpenProject(path.clone()));
    h.run();
    let s = &h.state().state;
    assert_eq!(s.sets.len(), 2);
    for set in s.sets.iter() {
        assert!(set.read_only.is_none(), "{:?}", set.read_only);
    }
    // 描かずに保存しても正本は書き直さない（バイト列のまま）
    h.state_mut().state.apply(Action::SaveProject);
    assert!(
        h.state().state.message.contains("書き直した正本 0"),
        "{}",
        h.state().state.message
    );
    // 層の表示を切ると、そのセットだけ正本を書き直す。グループ・マスク・チャンネルごとの合成・調整は残り、もう 1 つのセットはバイト列のまま
    let doc = &h.state().state.doc;
    let target = doc
        .layers()
        .iter()
        .find(|l| l.mask().is_some() || l.is_group())
        .map(|l| l.id())
        .expect("グループかマスクのある層");
    let mut expected = natives[0].to_core().unwrap();
    let visible = expected.layer(target).unwrap().visible();
    expected.set_layer_visible(target, !visible).unwrap();
    h.state_mut().state.apply(Action::ToggleVisible(target));
    h.run();
    assert!(h.state().state.modified);
    h.state_mut().state.apply(Action::SaveProject);
    let message = h.state().state.message.clone();
    assert!(message.starts_with("保存しました"), "{message}");
    assert!(message.contains("書き直した正本 1"), "{message}");
    let saved = read_project(&path);
    assert_eq!(
        saved.sets()[0].document.to_bytes(),
        yolu_io::NativeDocument::from_core(&expected)
            .unwrap()
            .to_bytes()
    );
    assert_eq!(saved.sets()[1].document.to_bytes(), natives[1].to_bytes());
    assert_eq!(saved.sets()[0].document.version(), 21);
    // 書き直したセットの合成は、使っているチャンネルごと（Color のほか Height）に書く。Unity 版が合成の並びからチャンネルを出す
    let first = saved.sets()[0].id.clone();
    let channels = yolu_io::composite_pngs(&expected).unwrap();
    assert!(channels
        .iter()
        .any(|(c, _)| *c == yolu_core::Channel::Height));
    for (channel, png) in channels {
        let entry = format!(
            "sets/{first}/composite/{}.png",
            channel.standard_name().unwrap()
        );
        assert_eq!(
            saved.migrated_entries().get(&entry).map(|b| &b[..]),
            Some(png.as_slice()),
            "{entry}"
        );
    }
    // 1 回 Undo して保存すると、元の正本に戻る
    h.state_mut().state.apply(Action::Undo);
    h.state_mut().state.apply(Action::SaveProject);
    assert_eq!(
        read_project(&path).sets()[0].document.to_bytes(),
        natives[0].to_bytes()
    );
}

#[test]
fn a_new_project_saves_as_and_then_overwrites_with_a_backup() {
    let dir = TempDir::new("new");
    let path = dir.0.join("新しい.ylp");
    let mut h = app(1280.0, 800.0, 256);
    let c = canvas_rect(&h).center();
    drag(&mut h, &[offset(c, -20.0, 0.0), offset(c, 20.0, 0.0)]);
    // 保存（まだファイルが無い）は別名で保存の窓を頼む
    h.state_mut().state.apply(Action::SaveProject);
    assert_eq!(h.state().state.dialog_request, Some(DialogRequest::SaveAs));
    h.state_mut().state.dialog_request = None;
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(path.clone()));
    let s = &h.state().state;
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    assert_eq!(s.project_name, "新しい");
    let saved = read_project(&path);
    assert_eq!(saved.sets().len(), 1);
    let set = &saved.sets()[0];
    assert_eq!(set.name, "テクスチャセット 1");
    assert_eq!(set.material, MaterialRef::PendingSlot(0));
    assert_eq!(set.id, set.document.id(), "新しいセットの ID は文書の ID");
    assert!(backups(&path).is_empty(), "新しく作ったので前の版は無い");
    // 描き足して上書き
    drag(&mut h, &[offset(c, 0.0, -30.0), offset(c, 0.0, 30.0)]);
    key(&h, Key::S, egui::Modifiers::COMMAND);
    h.run();
    assert!(
        h.state().state.message.contains("前の版は"),
        "{}",
        h.state().state.message
    );
    assert_eq!(backups(&path).len(), 1);
    // 新規プロジェクトで空に戻る
    h.state_mut().state.apply(Action::NewProject);
    let s = &h.state().state;
    assert!(s.project.is_none());
    assert_eq!(s.project_name, "名称未設定");
    assert_eq!(s.doc.width(), yolu_app::state::DEFAULT_DOCUMENT_SIZE);
    assert!(!layer_has_pixels(&s.doc.layers()[0]));
}

#[test]
fn saving_never_clobbers_what_it_cannot_read_or_what_changed_outside() {
    let dir = TempDir::new("guard");
    let mut h = app(1280.0, 800.0, 128);
    // .ylp でないファイルへの別名で保存は断る（中身はそのまま）
    let other = dir.0.join("notes.ylp");
    std::fs::write(&other, b"not a project").unwrap();
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(other.clone()));
    assert!(
        h.state().state.message.starts_with("保存できません"),
        "{}",
        h.state().state.message
    );
    assert_eq!(std::fs::read(&other).unwrap(), b"not a project");
    assert!(h.state().state.project.is_none());
    // 開いた後に外で書き換えられたら断る
    let path = dir.0.join("format6.ylp");
    std::fs::copy(fixture("format6.ylp"), &path).unwrap();
    h.state_mut().state.apply(Action::OpenProject(path.clone()));
    let outside = std::fs::read(fixture("format5-shared-materials.ylp")).unwrap();
    std::fs::write(&path, &outside).unwrap();
    h.state_mut().state.apply(Action::SaveProject);
    assert!(
        h.state().state.message.starts_with("保存できません"),
        "{}",
        h.state().state.message
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        outside,
        "外で書いた中身を潰さない"
    );
}

#[test]
fn sets_made_from_a_model_save_their_material_keys() {
    let dir = TempDir::new("keys");
    let path = dir.0.join("model.ylp");
    let mut h = app(1280.0, 800.0, 256);
    let mut skin = material(Some("Skin"), 512, true);
    skin.key = MaterialKey::Material {
        name: "Skin".into(),
        asset: Some(("0123456789abcdef0123456789abcdef".into(), 2100000)),
    };
    give_model(&mut h, vec![skin, material(None, 256, true)]);
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(path.clone()));
    assert!(
        h.state().state.message.starts_with("保存しました"),
        "{}",
        h.state().state.message
    );
    let saved = read_project(&path);
    let names: Vec<&str> = saved.sets().iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Skin", "Unassigned"]);
    assert_eq!(
        saved.sets()[0].material,
        MaterialRef::Material {
            name: "Skin".into(),
            asset: Some(yolu_app::sets::MaterialAsset {
                guid: "0123456789abcdef0123456789abcdef".into(),
                file_id: 2100000
            })
        }
    );
    assert_eq!(saved.sets()[1].material, MaterialRef::Unassigned);
    assert_eq!(saved.sets()[1].document.width(), 256);
    // 開き直してモデルを受けると、同じセットに付く（新しいセットは作らない）
    let mut again = app(1280.0, 800.0, 256);
    again
        .state_mut()
        .state
        .apply(Action::OpenProject(path.clone()));
    let mut skin = material(Some("名前を変えた"), 512, true);
    skin.key = MaterialKey::Material {
        name: "名前を変えた".into(),
        asset: Some(("0123456789abcdef0123456789abcdef".into(), 2100000)),
    };
    give_model(&mut again, vec![material(None, 256, true), skin]);
    let s = &again.state().state;
    assert_eq!(s.sets.len(), 2);
    assert_eq!(s.set_for_material(1), Some(0), "識別子で付く");
    assert_eq!(s.set_for_material(0), Some(1));
}

#[test]
fn a_broken_file_changes_nothing_and_says_why() {
    let dir = TempDir::new("broken");
    let path = dir.0.join("broken.ylp");
    std::fs::write(&path, b"PK\x03\x04 not really").unwrap();
    let mut h = app(1280.0, 800.0, 256);
    let doc = h.state().state.doc.id();
    h.state_mut().state.apply(Action::OpenProject(path.clone()));
    h.run();
    let s = &h.state().state;
    assert_eq!(s.doc.id(), doc);
    assert_eq!(s.sets.len(), 1);
    assert!(s.project.is_none());
    assert!(s.message.starts_with("開けません"), "{}", s.message);
}

#[test]
fn open_from_the_menu_and_the_keys_asks_for_a_file() {
    let mut h = app(1280.0, 800.0, 256);
    key(&h, Key::O, egui::Modifiers::COMMAND);
    h.run();
    assert_eq!(h.state().state.dialog_request, Some(DialogRequest::Open));
    h.state_mut().state.dialog_request = None;
    let at = menu_title(&h, "ファイル").center();
    click(&mut h, at);
    h.snapshot("menu_file");
    let at = popup_item(&h, "開く…").center();
    click(&mut h, at);
    assert_eq!(h.state().state.dialog_request, Some(DialogRequest::Open));
}

/// 画面を使わない保存の往復（Windows 向けに組んで wine でも回す。置き換えの rename と前の版の置き場を通す）。
#[test]
fn headless_open_paint_save_and_reopen() {
    use yolu_app::engine::DVec2;
    use yolu_app::state::AppState;
    let dir = TempDir::new("hsave");
    let path = dir.0.join("format6.ylp");
    std::fs::copy(fixture("format6.ylp"), &path).unwrap();
    let mut s = AppState::new(64, 64);
    s.apply(Action::OpenProject(path.clone()));
    assert_eq!(s.sets.len(), 3);
    s.apply(Action::SelectSet(s.sets.get(1).unwrap().uid));
    s.color.set_main([0.0, 1.0, 0.0, 1.0]);
    let layer = s.selected_layer.unwrap();
    let brush = s.stroke_settings(false);
    let mut stroke = s.doc.begin_stroke(layer, &brush).unwrap();
    stroke
        .add_point(&mut s.doc, 100.0, 100.0, 1.0, DVec2::ZERO)
        .unwrap();
    stroke
        .add_point(&mut s.doc, 140.0, 100.0, 1.0, DVec2::ZERO)
        .unwrap();
    s.doc.end_stroke(stroke).unwrap();
    s.modified = true;
    s.apply(Action::SaveProject);
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    assert_eq!(backups(&path).len(), 1);
    let mut again = AppState::new(64, 64);
    again.apply(Action::OpenProject(path.clone()));
    assert_eq!(again.sets.get(1).map(|x| x.name.as_str()), Some("Cloth"));
    assert_eq!(again.sets.current_index(), 1, "今のセットも保存する");
    assert_eq!(composite_pixel(&again.doc, 120, 100), [0, 255, 0, 255]);
}

// ───────── 退避の保持数・保存先の名前とフォルダー（保存の配線） ─────────

/// 今のレイヤーに短い線を 1 本描いて、変更ありにする（保存のたびに内容が変わる）。
fn paint_a_stroke(s: &mut yolu_app::state::AppState, x: f64) {
    use yolu_app::engine::DVec2;
    let layer = s.selected_layer.unwrap();
    let brush = s.stroke_settings(false);
    let mut stroke = s.doc.begin_stroke(layer, &brush).unwrap();
    stroke.add_point(&mut s.doc, x, 20.0, 1.0, DVec2::ZERO).unwrap();
    stroke.add_point(&mut s.doc, x + 6.0, 20.0, 1.0, DVec2::ZERO).unwrap();
    s.doc.end_stroke(stroke).unwrap();
    s.modified = true;
}

#[test]
fn the_backups_to_keep_setting_decides_how_many_previous_versions_stay() {
    use yolu_app::prefs::PrefsAction;
    use yolu_app::state::AppState;
    use yolu_io::BackupKeep;
    let dir = TempDir::new("keep");
    let path = dir.0.join("keep.ylp");
    let mut s = AppState::new(64, 64);
    assert_eq!(s.prefs.settings.backups, BackupKeep::All, "既定はすべて残す");
    s.apply(Action::Prefs(PrefsAction::SetBackups(BackupKeep::Count(2))));
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    assert!(!s.message.contains("前の版は"), "新しく作ったので前の版は無い");
    for i in 0..4 {
        paint_a_stroke(&mut s, 8.0 + 8.0 * i as f64);
        s.apply(Action::SaveProject);
        assert!(s.message.starts_with("保存しました"), "{}", s.message);
        assert!(s.message.contains("前の版は keep.ylp-backups~ に残しました"), "{}", s.message);
    }
    // 4 回の上書きで、残るのは新しい 2 つだけ。今のファイルの 1 つ前の版が必ず入っている
    assert_eq!(backups(&path).len(), 2);
    let newest = yolu_io::backups(&path).unwrap().remove(0);
    let before_last_save = std::fs::read(&newest).unwrap();
    assert_ne!(before_last_save, std::fs::read(&path).unwrap());
    assert!(yolu_io::Project::read(&before_last_save).is_ok(), "退避は .ylp として読める");
    // 0: 退避しない（知らせにも出さず、すでにある 2 つは消さない）
    s.apply(Action::Prefs(PrefsAction::SetBackups(BackupKeep::Count(0))));
    paint_a_stroke(&mut s, 50.0);
    s.apply(Action::SaveProject);
    assert!(s.message.starts_with("保存しました") && !s.message.contains("前の版は"), "{}", s.message);
    assert_eq!(backups(&path).len(), 2);
    // すべて: 消さずに溜める
    s.apply(Action::Prefs(PrefsAction::SetBackups(BackupKeep::All)));
    for i in 0..2 {
        paint_a_stroke(&mut s, 20.0 + 8.0 * i as f64);
        s.apply(Action::SaveProject);
    }
    assert_eq!(backups(&path).len(), 4);
    // 英語の知らせ
    s.set_language(yolu_app::lang::Lang::En);
    paint_a_stroke(&mut s, 40.0);
    s.apply(Action::SaveProject);
    assert!(s.message.contains("Previous version: keep.ylp-backups~."), "{}", s.message);
}

#[test]
fn saving_as_over_another_file_follows_the_setting_too() {
    use yolu_app::prefs::PrefsAction;
    use yolu_app::state::AppState;
    use yolu_io::BackupKeep;
    let dir = TempDir::new("keep-as");
    let (a, b) = (dir.0.join("a.ylp"), dir.0.join("b.ylp"));
    let mut s = AppState::new(64, 64);
    s.apply(Action::SaveProjectAs(a.clone()));
    paint_a_stroke(&mut s, 10.0);
    s.apply(Action::SaveProjectAs(b.clone()));
    // b は新しい保存先なので退避なし。a に別名で保存し直す（a は上書き）と a の前の版を数に従って残す
    s.apply(Action::Prefs(PrefsAction::SetBackups(BackupKeep::Count(0))));
    paint_a_stroke(&mut s, 20.0);
    s.apply(Action::SaveProjectAs(a.clone()));
    assert!(s.message.starts_with("保存しました") && !s.message.contains("前の版は"), "{}", s.message);
    assert!(backups(&a).is_empty());
    s.apply(Action::Prefs(PrefsAction::SetBackups(BackupKeep::Count(1))));
    paint_a_stroke(&mut s, 30.0);
    s.apply(Action::SaveProjectAs(a.clone()));
    assert!(s.message.contains("前の版は a.ylp-backups~"), "{}", s.message);
    assert_eq!(backups(&a).len(), 1);
}

#[test]
fn a_name_without_ylp_is_refused_and_a_missing_folder_is_created() {
    use yolu_app::state::AppState;
    let dir = TempDir::new("names");
    let mut s = AppState::new(64, 64);
    for bad in ["noext", "pic.png", "doc.ylp.bak", "doc.ylp~"] {
        s.apply(Action::SaveProjectAs(dir.0.join("sub").join(bad)));
        assert!(s.message.starts_with("保存できません") && s.message.contains(".ylp"), "{bad}: {}", s.message);
        assert!(s.project.is_none(), "{bad}");
    }
    assert!(!dir.0.join("sub").exists(), "断った保存はフォルダーも作らない");
    s.set_language(yolu_app::lang::Lang::En);
    s.apply(Action::SaveProjectAs(dir.0.join("noext")));
    assert!(s.message.ends_with("The file name must end with .ylp"), "{}", s.message);
    // フォルダーは、なければ保存のときに作る（大文字の拡張子でも保存できる）
    let nested = dir.0.join("a").join("b").join("Deep.YLP");
    s.apply(Action::SaveProjectAs(nested.clone()));
    assert!(s.message.starts_with("Saved"), "{}", s.message);
    assert!(nested.is_file());
    assert_eq!(s.project_name, "Deep");
    // 開き直せる
    let mut again = AppState::new(64, 64);
    again.apply(Action::OpenProject(nested.clone()));
    assert!(again.project.is_some(), "{}", again.message);
}
