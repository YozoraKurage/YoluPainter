//! .ylp に保存できる上限（セット 64・辺 8192）と Live Link・PSD の取り込み、キーボードでの画面全体の拡大縮小、画面の文。
//! 取り込めても保存だけが止まる文書・セットを作らない（保存の途中で断られると、復旧の書き置きも同じ理由で止まる）。
//! 層の数・入れ子の上限は yolu-io（`psd_import.rs`・`nesting_native.rs`）と yolu-core（`nesting.rs`）が確かめる。

mod common;

use common::fbx::temp_dir;
use egui::{Key, Modifiers};
use yolu_app::lang::Lang;
use yolu_app::newproject::MAX_SETS;
use yolu_app::psd::{PsdAction, PsdTarget};
use yolu_app::state::{Action, AppState};
use yolu_app::ui::menu::Entry;
use yolu_app::YoluApp;
use yolu_protocol::{MaterialInfo, MaterialKey, MeshData, Model, Submesh};

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

fn model_with(materials: usize) -> Model {
    Model {
        generation: 1,
        name: "多".into(),
        materials: (0..materials).map(|i| info(&format!("Mat{i}"))).collect(),
        meshes: vec![MeshData {
            key: "0".into(),
            name: "板".into(),
            skinned: false,
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            submeshes: (0..materials as u32)
                .map(|m| Submesh {
                    material: m,
                    indices: vec![0, 1, 2],
                })
                .collect(),
        }],
    }
}

/// 64×64 の PSD を書いて、そのバイト列を返す（アプリの書き出し）。
fn write_psd(path: &std::path::Path) -> Vec<u8> {
    let mut s = AppState::new(64, 64);
    s.bake.backend = yolu_app::bake::BakeBackend::Cpu;
    s.apply(Action::Psd(PsdAction::Export(path.to_path_buf())));
    s.wait_psd();
    std::fs::read(path).unwrap()
}

fn report_text(s: &AppState) -> String {
    let r = s.psd.report.as_ref().expect("理由の窓");
    let mut text = r.summary.clone();
    for line in &r.lines {
        text.push('\n');
        text.push_str(&line.text);
    }
    text
}

fn import(s: &mut AppState, path: &std::path::Path, target: PsdTarget) {
    s.apply(Action::Psd(PsdAction::Import {
        path: path.to_path_buf(),
        target,
    }));
    s.wait_psd();
}

// ───────── PSD の取り込み ─────────

#[test]
fn a_psd_wider_than_the_ylp_limit_is_refused_with_its_reason_and_nothing_changes() {
    let dir = temp_dir("limits-edge");
    let good = dir.join("ok.psd");
    let bytes = write_psd(&good);
    // ヘッダーの幅（高さの次の 4 バイト）を 8193 にする。8192 ちょうどは断らない
    let mut wide = bytes.clone();
    wide[18..22].copy_from_slice(&8193u32.to_be_bytes());
    let path = dir.join("Wide.psd");
    std::fs::write(&path, &wide).unwrap();

    let mut s = AppState::new(64, 64);
    for target in [PsdTarget::NewSet, PsdTarget::CurrentSet] {
        import(&mut s, &path, target);
        assert_eq!(s.sets.len(), 1, "何も変えない");
        assert!(!s.modified);
        assert_eq!((s.doc.width(), s.doc.height()), (64, 64));
        let text = report_text(&s);
        assert!(text.contains("8193") && text.contains("8192"), "{text}");
        assert!(!text.contains("予算"), "予算を上げても取り込めない理由: {text}");
        s.psd.report = None;
    }
    s.lang = Lang::En;
    import(&mut s, &path, PsdTarget::NewSet);
    let text = report_text(&s);
    assert!(text.contains("limit 8192") && !text.chars().any(|c| c > '\u{2000}'), "{text}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_new_set_psd_is_refused_when_the_project_already_has_the_most_sets() {
    let dir = temp_dir("limits-sets");
    let good = dir.join("Body.psd");
    write_psd(&good);
    let mut s = AppState::new(64, 64);
    for _ in 1..MAX_SETS {
        s.add_texture_set().unwrap();
    }
    assert_eq!(s.sets.len(), MAX_SETS);
    import(&mut s, &good, PsdTarget::NewSet);
    assert_eq!(s.sets.len(), MAX_SETS, "65 個目は作らない");
    assert!(!s.psd.is_busy(), "読み始める前に断る");
    assert!(s.message.contains("64"), "{}", s.message);
    s.lang = Lang::En;
    import(&mut s, &good, PsdTarget::NewSet);
    assert!(s.message.contains("at most 64"), "{}", s.message);
    // 今のセットを替える取り込みは、セットの数に関わらない
    s.lang = Lang::Ja;
    import(&mut s, &good, PsdTarget::CurrentSet);
    assert_eq!(s.sets.len(), MAX_SETS);
    assert!(s.message.contains("読み込みました"), "{}", s.message);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_psd_read_while_the_sets_filled_up_is_not_installed() {
    let dir = temp_dir("limits-sets-race");
    let good = dir.join("Body.psd");
    write_psd(&good);
    let mut s = AppState::new(64, 64);
    for _ in 2..MAX_SETS {
        s.add_texture_set().unwrap();
    }
    assert_eq!(s.sets.len(), MAX_SETS - 1);
    // 読み始めたときはまだ 1 つ足せる。結果を受ける前に、ほかの手でセットが上限まで増える
    s.apply(Action::Psd(PsdAction::Import {
        path: good.clone(),
        target: PsdTarget::NewSet,
    }));
    assert!(s.psd.is_busy());
    s.add_texture_set().unwrap();
    s.wait_psd();
    assert_eq!(s.sets.len(), MAX_SETS, "入れない");
    assert!(s.message.contains("64"), "{}", s.message);
    let _ = std::fs::remove_dir_all(dir);
}

// ───────── Live Link ─────────

#[test]
fn a_live_link_model_with_more_materials_than_a_project_holds_gets_sets_up_to_the_limit() {
    let mut s = AppState::new(64, 64);
    let (report, _) = s.receive_link_model(&model_with(MAX_SETS + 1), 1);
    assert_eq!(s.sets.len(), MAX_SETS, "65 個目のマテリアルにはセットを作らない");
    // 最初のセットがマテリアルに付き、残りの 63 を作る。足りない 1 つを数で知らせる
    assert_eq!(report.created.len(), MAX_SETS - 1);
    assert_eq!(report.skipped, 1);
    let ja = report.limit_text(Lang::Ja).expect("知らせる");
    assert!(ja.contains("64") && ja.contains("1 個"), "{ja}");
    let en = report.limit_text(Lang::En).expect("知らせる");
    assert!(en.contains("(64)") && en.is_ascii(), "{en}");
    // 保存できる（.ylp の上限に収まる）
    let dir = temp_dir("limits-link");
    let path = dir.join("p.ylp");
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let saved = yolu_io::SaveTarget::open(&path).unwrap().0;
    assert_eq!(saved.sets().len(), MAX_SETS);
    let _ = std::fs::remove_dir_all(dir);
    // 上限に収まるモデルは今までどおり、知らせない
    let mut t = AppState::new(64, 64);
    let (report, _) = t.receive_link_model(&model_with(5), 1);
    assert_eq!((t.sets.len(), report.skipped), (5, 0));
    assert!(report.limit_text(Lang::Ja).is_none());
}

#[test]
fn the_set_limit_of_the_app_is_the_one_the_ylp_reader_enforces() {
    assert_eq!(MAX_SETS, yolu_io::MAX_PROJECT_SETS);
    assert_eq!(MAX_SETS, 64);
}

// ───────── キーボードでの画面全体の拡大縮小 ─────────

fn zoom_after_keys(setup: bool) -> f32 {
    let mut h = common::gpu_thread::builder().build_ui(move |ui| {
        if setup {
            YoluApp::setup(ui.ctx());
        }
        ui.label("x");
    });
    h.run();
    for key in [Key::Minus, Key::Minus, Key::Equals, Key::Plus, Key::Num0, Key::Minus] {
        h.key_press_modifiers(Modifiers::COMMAND, key);
        h.run();
    }
    h.ctx.zoom_factor()
}

#[test]
fn ctrl_minus_and_plus_do_not_zoom_the_whole_interface() {
    // 切っていない（egui の既定）窓では、Ctrl+- で画面全体が縮む（この試験が効くことの確かめ）
    assert!(zoom_after_keys(false) < 1.0, "既定では拡大率が変わる");
    // アプリの文脈では、画面全体の拡大率は動かない（キャンバスの拡大縮小はアプリのキーが受ける）
    assert_eq!(zoom_after_keys(true), 1.0);
}

// ───────── 画面の文 ─────────

#[test]
fn about_names_the_product_and_the_version_and_nothing_else() {
    for lang in Lang::ALL {
        let mut s = AppState::new_in(64, 64, lang);
        s.apply(Action::About);
        // 正式版は製品名と版だけ。試験版（0.4.0-rc.1 のような版）には、版のあとに試験版の印が付く
        let version = env!("CARGO_PKG_VERSION");
        let beta = yolu_update::Version::parse(version)
            .is_ok_and(|version| yolu_update::is_beta_version(&version));
        let mark = match (beta, lang) {
            (false, _) => "",
            (true, Lang::Ja) => "（試験版）",
            (true, Lang::En) => " (beta)",
        };
        assert_eq!(s.message, format!("YoluPainter {version}{mark}"));
    }
}

#[test]
fn the_save_notice_names_the_file_and_the_writer_is_the_product_name() {
    let dir = temp_dir("limits-save");
    let path = dir.join("作品.ylp");
    for (lang, text) in [(Lang::Ja, "保存しました: 作品.ylp。"), (Lang::En, "Saved: 作品.ylp.")] {
        let mut s = AppState::new_in(64, 64, lang);
        s.apply(Action::SaveProjectAs(path.clone()));
        assert_eq!(s.message, text, "形式・セットの数・書き直した数は出さない");
        assert_eq!(s.rewritten_sets, 1, "書き直した数は試験が見る");
        let _ = std::fs::remove_file(&path);
    }
    let mut s = AppState::new(64, 64);
    s.apply(Action::SaveProjectAs(path.clone()));
    let saved = yolu_io::SaveTarget::open(&path).unwrap().0;
    let by = saved.info().saved_by.clone().unwrap();
    assert_eq!((by.app.as_str(), by.unity.as_str()), ("YoluPainter", "standalone"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_view_menu_has_no_test_cube_or_test_figure() {
    use yolu_app::view3d::pose::PoseAction;
    for lang in Lang::ALL {
        let s = AppState::new_in(64, 64, lang);
        let entries = yolu_app::shell::menu_entries(&s, 5);
        assert!(entries.len() > 8, "表示のメニューを引けている");
        for e in &entries {
            if let Entry::Item { action, label, .. } = e {
                assert!(
                    !matches!(action, Action::LoadDemoModel | Action::Pose(PoseAction::LoadFigure)),
                    "{label}"
                );
                assert!(!label.contains("試し") && !label.contains("Test"), "{label}");
            }
        }
    }
}
