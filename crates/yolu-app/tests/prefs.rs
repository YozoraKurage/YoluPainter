//! 設定の窓（言語・書き出しの余白・メモリの予算・CPU のスレッド・表示の合成・棚の場所・退避を残す数）: 値の選びが画面の状態・文書の予算に効くこと、
//! 設定のファイルへの保存と起動での復元、壊れた値の理由、窓の操作と日英。`headless_` で始まる試験は画面を描かず、Wine でも回る。
mod common;

use std::path::{Path, PathBuf};

use common::*;
use egui::{vec2, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::engine::{Document, LayerId, Rgba8};
use yolu_app::lang::Lang;
use yolu_app::m2::UiOp;
use yolu_app::pen::PenInput;
use yolu_app::prefs::{self, entries, Pref, PrefChoice, PrefsAction};
use yolu_app::settings::{Budget, BudgetKind, Compositing, Settings};
use yolu_app::state::{Action, AppState, DialogRequest};
use yolu_app::YoluApp;

const MIB: u64 = 1024 * 1024;

fn set(s: &mut AppState, pref: Pref) {
    s.apply(Action::Prefs(PrefsAction::Set(pref)));
}

fn state() -> AppState {
    let mut s = AppState::new(64, 64);
    s.prefs.ram_mib = 16384;
    s.prefs.cores = 8;
    s
}

fn labels(entries: &[yolu_app::ui::menu::Entry<Action>]) -> Vec<String> {
    entries
        .iter()
        .filter_map(|e| match e {
            yolu_app::ui::menu::Entry::Item { label, .. } => Some(label.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn headless_an_unmanaged_state_keeps_the_core_budgets_until_settings_are_loaded() {
    let mut s = state();
    let before = (s.doc.undo_budget_bytes(), s.doc.source_budget_bytes(), s.doc.stroke_budget_bytes());
    s.sync_budgets();
    assert_eq!(
        (s.doc.undo_budget_bytes(), s.doc.source_budget_bytes(), s.doc.stroke_budget_bytes()),
        before,
        "設定を読んでいない状態は、core の既定のまま"
    );
    // 設定を読むと自動の予算（16 GB: 取り消し 1024・画素 2048・1 回の操作 512）
    s.load_settings(Settings::default());
    s.sync_budgets();
    assert_eq!(s.doc.undo_budget_bytes(), 1024 * MIB);
    assert_eq!(s.doc.source_budget_bytes(), 2048 * MIB);
    assert_eq!(s.doc.stroke_budget_bytes(), 512 * MIB);
    assert_eq!(s.doc.minimum_undo_steps(), 5);
}

#[test]
fn headless_choosing_a_budget_changes_the_current_document_and_new_documents_follow() {
    let mut s = state();
    s.load_settings(Settings::default());
    s.apply(Action::M2Ui(UiOp::Language(Lang::Ja)));
    set(&mut s, Pref::Budget(BudgetKind::Undo, Budget::Mib(512)));
    set(&mut s, Pref::Budget(BudgetKind::Source, Budget::Mib(4096)));
    set(&mut s, Pref::Budget(BudgetKind::Stroke, Budget::Mib(128)));
    set(&mut s, Pref::MinUndoSteps(9));
    assert_eq!(s.doc.undo_budget_bytes(), 512 * MIB);
    assert_eq!(s.doc.source_budget_bytes(), 4096 * MIB);
    assert_eq!(s.doc.stroke_budget_bytes(), 128 * MIB);
    assert_eq!(s.doc.minimum_undo_steps(), 9);
    assert_eq!(s.prefs.settings.undo_budget, Budget::Mib(512));
    // 範囲の外は範囲に収める
    set(&mut s, Pref::Budget(BudgetKind::Stroke, Budget::Mib(1)));
    assert_eq!(s.prefs.settings.stroke_budget, Budget::Mib(8));
    set(&mut s, Pref::Budget(BudgetKind::Undo, Budget::Mib(999_999)));
    assert_eq!(s.prefs.settings.undo_budget, Budget::Mib(16384));
    set(&mut s, Pref::MinUndoSteps(9999));
    assert_eq!(s.prefs.settings.min_undo_steps, 100);
    // 自動に戻すと、メモリから
    set(&mut s, Pref::Budget(BudgetKind::Undo, Budget::Auto));
    assert_eq!(s.doc.undo_budget_bytes(), 1024 * MIB);
    // 新しいプロジェクト（新しい文書）にも、次の同期で入る
    set(&mut s, Pref::Budget(BudgetKind::Source, Budget::Mib(1024)));
    s.apply(Action::NewProject);
    s.sync_budgets();
    assert_eq!(s.doc.source_budget_bytes(), 1024 * MIB, "新しい文書");
    assert_eq!(s.doc.minimum_undo_steps(), 100);
}

/// 文書の層に、タイルを `count` 枚ぶん（先頭から）不透明に塗って、画素を確保する（1 タイル 128 × 128 × 4 バイト = 64 KiB）。
fn fill_tiles(doc: &mut Document, layer: LayerId, count: u32) -> Result<(), yolu_app::engine::CoreError> {
    let ts = doc.tile_size();
    let per_row = doc.width() / ts;
    for i in 0..count {
        doc.set_pixel(layer, (i % per_row) * ts, (i / per_row) * ts, Rgba8::new(1, 2, 3, 255))?;
    }
    Ok(())
}

#[test]
fn headless_the_pixel_and_history_budgets_are_the_projects_total_shared_by_all_the_texture_sets() {
    let mut s = AppState::new(2048, 2048);
    s.prefs.ram_mib = 16384;
    let (_, shape) = s.receive_link_model(&two_sets_model(), 0);
    shape.expect("3D に読める");
    assert_eq!(s.sets.len(), 2);
    s.load_settings(Settings::default());
    // 全体の予算: 画素 16 MiB（最小）・取り消し履歴 64 MiB・最小の取り消し段数 9
    set(&mut s, Pref::Budget(BudgetKind::Source, Budget::Mib(16)));
    set(&mut s, Pref::Budget(BudgetKind::Undo, Budget::Mib(64)));
    set(&mut s, Pref::MinUndoSteps(9));
    let (first, second) = (s.sets.current_index(), 1 - s.sets.current_index());
    // ほかのセットが何も使っていなければ、今のセットが全体を使える
    assert_eq!(s.doc.source_budget_bytes(), 16 * MIB);
    assert_eq!(s.doc.undo_budget_bytes(), 64 * MIB);
    // 今のセットが画素と履歴を使う（100 タイル = 6.25 MiB）
    let layer = s.selected_layer.unwrap();
    fill_tiles(&mut s.doc, layer, 100).unwrap();
    // ストロークを 1 本（先頭のタイルの中。履歴に 1 段入る。画素を直に書く操作は履歴を消す）
    let mut stroke = s.begin_paint_stroke(layer, false).unwrap();
    stroke.add_point(&mut s.doc, 20.5, 20.5, 1.0, yolu_app::engine::DVec2::ZERO).unwrap();
    stroke.add_point(&mut s.doc, 90.5, 60.5, 1.0, yolu_app::engine::DVec2::ZERO).unwrap();
    s.doc.end_stroke(stroke).unwrap();
    let (used, history) = (s.doc.allocated_bytes(), s.doc.history_bytes());
    assert_eq!(used, 100 * 65536);
    assert!(history > 0, "ストロークで履歴が増える");
    // 切り替えると、新しい今のセットには「設定 − ほかのセットの使用量」が入る
    s.switch_set(second).unwrap();
    assert_eq!(s.doc.source_budget_bytes(), 16 * MIB - used);
    assert_eq!(s.doc.undo_budget_bytes(), 64 * MIB - history);
    assert_eq!(s.doc.minimum_undo_steps(), 9, "最小の取り消し段数はセットごとに残る");
    // 2 つ目のセットは、残りまでしか塗れない（断られても何も壊れず、合計が設定を超えない）
    let layer = s.selected_layer.unwrap();
    let refused = fill_tiles(&mut s.doc, layer, 256).unwrap_err();
    assert_eq!(refused, yolu_app::engine::CoreError::SourceBudgetExceeded);
    let total = s.set_doc(first).allocated_bytes() + s.set_doc(second).allocated_bytes();
    assert!(total <= 16 * MIB, "{total}");
    assert!(total > 15 * MIB, "残りまで使える: {total}");
    // 戻ると、1 つ目のセットの予算は 2 つ目の使用量を引いた分（今の画素以上）
    s.switch_set(first).unwrap();
    assert_eq!(s.doc.source_budget_bytes(), 16 * MIB - s.set_doc(second).allocated_bytes());
    assert!(s.doc.source_budget_bytes() >= s.doc.allocated_bytes());
    // 設定を変えると、今のセットだけに次の同期で入る（全体の値から、ほかのセットの使用量を引いて）
    set(&mut s, Pref::Budget(BudgetKind::Source, Budget::Mib(64)));
    assert_eq!(s.doc.source_budget_bytes(), 64 * MIB - s.set_doc(second).allocated_bytes());
    s.switch_set(second).unwrap();
    assert_eq!(s.doc.source_budget_bytes(), 64 * MIB - s.set_doc(first).allocated_bytes());
    // 自動の予算でも同じ（16 GB: 画素 2048 MiB）
    set(&mut s, Pref::Budget(BudgetKind::Source, Budget::Auto));
    assert_eq!(s.doc.source_budget_bytes(), 2048 * MIB - s.set_doc(first).allocated_bytes());
}

#[test]
fn headless_a_project_already_over_the_pixel_budget_keeps_its_pixels_and_says_so_once() {
    let mut s = AppState::new(2048, 2048);
    s.prefs.ram_mib = 16384;
    let (_, shape) = s.receive_link_model(&two_sets_model(), 0);
    shape.expect("3D に読める");
    s.load_settings(Settings::default());
    // 1 つ目のセットが 20 MiB（2048 × 2048 の 1 層は 16 MiB なので 2 層）使っているところへ、全体の予算 16 MiB
    let first = s.sets.current_index();
    for _ in 0..2 {
        s.apply(Action::NewLayer);
        let layer = s.selected_layer.unwrap();
        fill_tiles(&mut s.doc, layer, 160).unwrap();
    }
    s.message.clear();
    set(&mut s, Pref::Budget(BudgetKind::Source, Budget::Mib(16)));
    assert_eq!(s.doc.source_budget_bytes(), s.doc.allocated_bytes(), "画素は捨てず、予算を今の量まで広げる");
    assert_eq!(s.message, "レイヤーの画素がすでに予算を超えているので、予算を上げるまで足せません。");
    // 同じ状況では知らせ直さない
    s.message.clear();
    s.sync_budgets();
    assert_eq!(s.message, "");
    // 足せない（今の量が予算）
    let layer = s.selected_layer.unwrap();
    assert!(fill_tiles(&mut s.doc, layer, 256).is_err());
    // もう 1 つのセットは、残りが無いので 0（足せない）。知らせは今のセットの画素が超えているときだけ
    s.switch_set(1 - first).unwrap();
    assert_eq!(s.doc.source_budget_bytes(), 0);
    // 予算を上げれば、また足せる
    set(&mut s, Pref::Budget(BudgetKind::Source, Budget::Mib(1024)));
    let layer = s.selected_layer.unwrap();
    assert!(fill_tiles(&mut s.doc, layer, 10).is_ok());
    // 英語
    s.switch_set(first).unwrap();
    s.apply(Action::M2Ui(UiOp::Language(Lang::En)));
    set(&mut s, Pref::Budget(BudgetKind::Source, Budget::Mib(16)));
    assert_eq!(s.message, "The layer pixels are already over the budget; nothing can be added until it is raised.");
}

fn two_sets_model() -> yolu_protocol::Model {
    use yolu_protocol::{MaterialInfo, MaterialKey, MeshData, Model, Submesh, TextureProperty};
    let material = |name: &str| MaterialInfo {
        key: MaterialKey::Material { name: name.into(), asset: None },
        shader: "Standard".into(),
        textures: vec![TextureProperty { name: "_MainTex".into(), width: 64, height: 64 }],
        routes: vec![],
    };
    let mesh = |name: &str, x: f32, material: u32| MeshData {
        key: name.into(),
        name: name.into(),
        skinned: false,
        positions: vec![[x, 0.0, 0.0], [x + 1.0, 0.0, 0.0], [x, 1.0, 0.0]],
        normals: vec![],
        uv0: vec![[0.1, 0.1], [0.4, 0.1], [0.1, 0.4]],
        submeshes: vec![Submesh { material, indices: vec![0, 2, 1] }],
    };
    Model {
        generation: 1,
        name: "二つ".into(),
        materials: vec![material("A"), material("B")],
        meshes: vec![mesh("a", 0.0, 0), mesh("b", 3.0, 1)],
    }
}

#[test]
fn headless_a_budget_is_not_changed_while_drawing_and_a_too_small_pixel_budget_widens_to_the_current_pixels() {
    let mut s = AppState::new(4096, 4096);
    s.prefs.ram_mib = 16384;
    s.load_settings(Settings::default());
    // 描いている間は入れず、終わったフレームで入れる
    let layer = s.selected_layer.unwrap();
    let stroke = s.begin_paint_stroke(layer, false).unwrap();
    set(&mut s, Pref::Budget(BudgetKind::Undo, Budget::Mib(256)));
    assert_ne!(s.doc.undo_budget_bytes(), 256 * MIB);
    s.doc.cancel_stroke(stroke);
    s.sync_budgets();
    assert_eq!(s.doc.undo_budget_bytes(), 256 * MIB);
    // 今の画素（20 MiB 以上）より小さい画素の予算（16 MiB）: 画素は捨てず、予算を今の量まで広げて知らせる
    let id = s.selected_layer.unwrap();
    for ty in 0..32u32 {
        for tx in 0..10u32 {
            s.doc.set_pixel(id, tx * 128, ty * 128, Rgba8::new(1, 2, 3, 255)).unwrap();
        }
    }
    assert!(s.doc.allocated_bytes() > 16 * MIB, "{}", s.doc.allocated_bytes());
    s.message.clear();
    set(&mut s, Pref::Budget(BudgetKind::Source, Budget::Mib(16)));
    assert_eq!(s.doc.source_budget_bytes(), s.doc.allocated_bytes(), "予算を今の量まで広げた（足せない）");
    assert_eq!(s.message, "レイヤーの画素がすでに予算を超えているので、予算を上げるまで足せません。");
    // 選んだ値は設定に残る（画素の少ない文書なら効く）。何度も知らせ直さない
    assert_eq!(s.prefs.settings.source_budget, Budget::Mib(16));
    s.message.clear();
    s.sync_budgets();
    assert_eq!(s.message, "");
    s.apply(Action::M2Ui(UiOp::Language(Lang::En)));
    set(&mut s, Pref::Budget(BudgetKind::Source, Budget::Mib(64)));
    assert_eq!(s.doc.source_budget_bytes(), 64 * MIB);
    set(&mut s, Pref::Budget(BudgetKind::Source, Budget::Mib(16)));
    assert_eq!(s.message, "The layer pixels are already over the budget; nothing can be added until it is raised.");
}

#[test]
fn headless_the_other_values_are_kept_clamped_or_refused() {
    let mut s = state();
    set(&mut s, Pref::ExportPadding(8));
    assert_eq!((s.export.padding, s.prefs.settings.export_padding), (8, 8));
    set(&mut s, Pref::ExportPadding(-1));
    assert_eq!(s.export.padding, -1);
    set(&mut s, Pref::ExportPadding(3));
    assert_eq!(s.export.padding, -1, "選択肢に無い余白は受けない");
    set(&mut s, Pref::CpuThreads(Some(0)));
    assert_eq!(s.prefs.settings.cpu_threads, Some(1));
    set(&mut s, Pref::CpuThreads(Some(5000)));
    assert_eq!(s.prefs.settings.cpu_threads, Some(1024));
    set(&mut s, Pref::CpuThreads(None));
    assert_eq!(s.prefs.settings.cpu_threads, None);
    set(&mut s, Pref::Compositing(Compositing::Gpu));
    assert_eq!(s.prefs.settings.compositing, Compositing::Gpu);
    // 棚の場所: 絶対パスだけ。None で既定
    let folder = std::env::current_dir().unwrap().join("shelf");
    set(&mut s, Pref::LibraryFolder(Some(folder.clone())));
    assert_eq!(s.prefs.settings.library_folder, Some(folder.clone()));
    set(&mut s, Pref::LibraryFolder(Some(PathBuf::from("relative/shelf"))));
    assert_eq!(s.prefs.settings.library_folder, Some(folder), "相対パスは受けない");
    assert_eq!(s.message, "棚の場所は絶対パスで指定します。");
    s.apply(Action::M2Ui(UiOp::Language(Lang::En)));
    set(&mut s, Pref::LibraryFolder(Some(PathBuf::from("relative/shelf"))));
    assert_eq!(s.message, "The library folder must be an absolute path.");
    set(&mut s, Pref::LibraryFolder(None));
    assert_eq!(s.prefs.settings.library_folder, None);
    // 言語は画面の言語が持ち、設定の取り出しで重なる
    assert_eq!(s.settings().lang, Lang::En);
    s.apply(Action::M2Ui(UiOp::Language(Lang::Ja)));
    assert_eq!(s.settings().lang, Lang::Ja);
    // 棚の場所を選ぶ窓を頼む
    s.apply(Action::Prefs(PrefsAction::ChooseLibraryFolder));
    assert_eq!(s.dialog_request, Some(DialogRequest::PrefsLibraryFolder));
    // 窓の開け閉め
    s.apply(Action::Prefs(PrefsAction::Open));
    assert!(s.prefs.open);
    s.apply(Action::Prefs(PrefsAction::Close));
    assert!(!s.prefs.open);
}

#[test]
fn headless_the_choices_mark_the_current_value_and_add_an_odd_one() {
    let mut s = state();
    s.apply(Action::M2Ui(UiOp::Language(Lang::Ja)));
    let ja = labels(&entries(&s, PrefChoice::Budget(BudgetKind::Undo)));
    assert_eq!(ja[0], "自動（1024 MiB）", "{ja:?}");
    assert!(ja.contains(&"0 MiB".to_string()) && ja.contains(&"8192 MiB".to_string()));
    assert_eq!(labels(&entries(&s, PrefChoice::Budget(BudgetKind::Stroke)))[0], "自動（512 MiB）");
    assert_eq!(
        labels(&entries(&s, PrefChoice::CpuThreads)),
        ["自動（8）", "1（並列にしない）", "2", "4", "8"]
    );
    assert_eq!(
        labels(&entries(&s, PrefChoice::ExportPadding)),
        ["なし", "2 テクセル", "4 テクセル", "8 テクセル", "16 テクセル", "32 テクセル", "64 テクセル", "届くかぎり"]
    );
    assert_eq!(labels(&entries(&s, PrefChoice::Compositing)), ["自動", "GPU", "CPU"]);
    assert_eq!(labels(&entries(&s, PrefChoice::Language)), ["日本語", "English"]);
    // ファイルに書いた中途半端な数も、選択肢に出る（選び直せる）
    s.prefs.settings.undo_budget = Budget::Mib(777);
    s.prefs.settings.cpu_threads = Some(3);
    assert_eq!(labels(&entries(&s, PrefChoice::Budget(BudgetKind::Undo))).last().unwrap(), "777 MiB");
    assert_eq!(labels(&entries(&s, PrefChoice::CpuThreads)).last().unwrap(), "3");
    // 英語
    s.apply(Action::M2Ui(UiOp::Language(Lang::En)));
    assert_eq!(labels(&entries(&s, PrefChoice::Budget(BudgetKind::Undo)))[0], "Auto (1024 MiB)");
    assert_eq!(labels(&entries(&s, PrefChoice::CpuThreads))[0], "Automatic (8)");
    assert_eq!(labels(&entries(&s, PrefChoice::ExportPadding))[0], "Off");
    assert_eq!(labels(&entries(&s, PrefChoice::ExportPadding))[7], "Fill (all the way)");
    // 1 コア（並列にしない）の機械は 1 が 1 つだけ
    s.prefs.cores = 1;
    s.prefs.settings.cpu_threads = None;
    assert_eq!(labels(&entries(&s, PrefChoice::CpuThreads)), ["Automatic (1)", "1 (no parallel work)"]);
}

// ───────── 窓と設定のファイル ─────────

fn settings_dir(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/prefs-tests")
        .join(std::process::id().to_string())
        .join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn app_with_settings(path: &Path, size: egui::Vec2) -> Harness<'static, YoluApp> {
    let path = path.to_path_buf();
    let mut h = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .with_render_options(render_options())
        .wgpu()
        .build_eframe(move |cc| {
            YoluApp::for_context_with_settings(&cc.egui_ctx, Some(path), PenInput::detached())
                .with_render_state(cc.wgpu_render_state.as_ref())
        });
    h.state_mut().state.prefs.ram_mib = 16384;
    h.state_mut().state.prefs.cores = 8;
    h.run();
    h
}

fn open_settings(h: &mut Harness<'static, YoluApp>) {
    let at = menu_title(h, "表示").center();
    click(h, at);
    let item = popup_item(h, "設定…").center();
    click(h, item);
}

fn window_rect(h: &Harness<'_, YoluApp>) -> Rect {
    prefs::last_rect(&h.ctx).expect("設定の窓が開いている")
}

/// 窓の中だけを撮って、正解の絵と比べる（ほかのパネルの変更で壊れない）。
fn shot(h: &mut Harness<'_, YoluApp>, name: &str) {
    let rect = window_rect(h);
    // 直前に押した所のポインタが絵に残らないように
    h.event(egui::Event::PointerGone);
    h.step();
    let image = h.render().expect("描画");
    let cropped = image::imageops::crop_imm(
        &image,
        rect.left().floor() as u32,
        rect.top().floor() as u32,
        rect.width().ceil() as u32,
        rect.height().ceil() as u32,
    )
    .to_image();
    egui_kittest::image_snapshot(&cropped, name);
}

#[test]
fn the_settings_window_opens_from_the_view_menu_and_edits_every_value_into_the_file() {
    let dir = settings_dir("window");
    let path = dir.join("YoluPainter").join("settings.conf");
    let mut h = app_with_settings(&path, vec2(1280.0, 800.0));
    assert!(!h.state().state.prefs.open);
    open_settings(&mut h);
    assert!(h.state().state.prefs.open);
    // 棚の場所は、機械によらない場所にして撮る（既定の場所は設定のフォルダの下で、機械で違う）
    let shelf = PathBuf::from(if cfg!(windows) { "C:\\Shelf" } else { "/Shelf" });
    h.state_mut().state.apply(Action::Prefs(PrefsAction::Set(Pref::LibraryFolder(Some(shelf)))));
    h.run();
    let window = window_rect(&h);
    shot(&mut h, "prefs_window");
    // 値の箱（名前: 値）
    let pick = |h: &mut Harness<'static, YoluApp>, label: &str, item: &str| {
        let at = h.get_by_label(label).rect().center();
        click(h, at);
        let at = popup_item(h, item).center();
        click(h, at);
    };
    pick(&mut h, "書き出しの余白: 届くかぎり", "8 テクセル");
    assert_eq!(h.state().state.export.padding, 8);
    pick(&mut h, "取り消し履歴: 自動（1024 MiB）", "512 MiB");
    assert_eq!(h.state().state.doc.undo_budget_bytes(), 512 * MIB);
    pick(&mut h, "レイヤーの画素: 自動（2048 MiB）", "4096 MiB");
    assert_eq!(h.state().state.doc.source_budget_bytes(), 4096 * MIB);
    pick(&mut h, "1 回の操作: 自動（512 MiB）", "256 MiB");
    assert_eq!(h.state().state.doc.stroke_budget_bytes(), 256 * MIB);
    pick(&mut h, "表示の合成: 自動", "CPU");
    assert_eq!(h.state().state.prefs.settings.compositing, Compositing::Cpu);
    // CPU のスレッドは次の起動から効く（窓に出る）
    pick(&mut h, "CPU のスレッド: 自動（8）", "4");
    assert_eq!(h.state().state.prefs.settings.cpu_threads, Some(4));
    let _ = h.get_by_label("CPU のスレッド: 4 ・ 再起動で反映");
    h.get_by_label("起動時に Live Link を待ち受ける").click();
    h.run();
    assert!(!h.state().state.settings().livelink_on_startup);
    shot(&mut h, "prefs_window_changed");
    // 言語（選ぶと窓の文言も替わる）
    pick(&mut h, "言語: 日本語", "English");
    assert_eq!(h.state().state.lang, Lang::En);
    let _ = h.get_by_label("Language: English");
    let _ = h.get_by_label("CPU threads: 4 · applies after restart");
    shot(&mut h, "prefs_window_english");
    // 窓の中に全部収まっている（最後の行の下も）
    let last = h.get_by_label("Keep all").rect();
    assert!(window.contains_rect(last), "{window:?} {last:?}");
    // ファイル: 変えた値だけが書かれている
    h.run();
    let written = std::fs::read_to_string(&path).unwrap();
    for line in [
        "language=en",
        "export_padding=8",
        "undo_budget_mib=512",
        "source_budget_mib=4096",
        "stroke_budget_mib=256",
        "cpu_threads=4",
        "compositing=cpu",
        "livelink_on_startup=off",
    ] {
        assert!(written.lines().any(|l| l == line), "{line}\n{written}");
    }
    assert!(!written.contains("min_undo_steps"), "{written}");
    assert!(written.contains("library_folder="), "{written}");
    // 閉じる
    h.get_by_label("Close").click();
    h.run();
    assert!(!h.state().state.prefs.open);
    drop(h);
    // 次の起動は、書いた設定で始まる（スレッドは起動のときの値として覚える）
    let h = app_with_settings(&path, vec2(1280.0, 800.0));
    let s = &h.state().state;
    assert_eq!(s.lang, Lang::En);
    assert_eq!(s.export.padding, 8);
    assert_eq!(s.prefs.settings.cpu_threads, Some(4));
    assert_eq!(s.prefs.threads_at_start, Some(4));
    assert!(!s.settings().livelink_on_startup);
    assert_eq!(s.prefs.settings.compositing, Compositing::Cpu);
    assert_eq!(s.doc.undo_budget_bytes(), 512 * MIB, "文書にも入っている");
    assert_eq!(s.doc.source_budget_bytes(), 4096 * MIB);
    assert_eq!(s.doc.stroke_budget_bytes(), 256 * MIB);
    assert_eq!(s.message, "");
}

#[test]
fn the_minimum_undo_steps_slider_and_the_library_buttons_work() {
    let dir = settings_dir("slider");
    let path = dir.join("YoluPainter").join("settings.conf");
    let mut h = app_with_settings(&path, vec2(1280.0, 800.0));
    open_settings(&mut h);
    // スライダー（右端 = 100）
    let slider = h.get_by_label("最小の取り消し段数").rect();
    drag(
        &mut h,
        &[
            egui::pos2(slider.left() + slider.width() * 0.5, slider.bottom() - 4.0),
            egui::pos2(slider.right() - 2.0, slider.bottom() - 4.0),
        ],
    );
    assert!(h.state().state.prefs.settings.min_undo_steps > 50, "{}", h.state().state.prefs.settings.min_undo_steps);
    assert_eq!(h.state().state.doc.minimum_undo_steps(), h.state().state.prefs.settings.min_undo_steps as usize);
    // 棚の場所: 初めは既定（「既定に戻す」は押せない）。選ぶ窓を頼み、選んだ場所が出る
    assert_eq!(h.state().state.prefs.settings.library_folder, None);
    h.get_by_label("選ぶ…").click();
    h.run();
    assert_eq!(h.state().state.dialog_request, Some(DialogRequest::PrefsLibraryFolder));
    let folder = dir.join("MyShelf");
    h.state_mut().state.apply(Action::Prefs(PrefsAction::Set(Pref::LibraryFolder(Some(folder.clone())))));
    h.run();
    assert_eq!(h.state().state.prefs.settings.library_folder, Some(folder.clone()));
    h.get_by_label("既定に戻す").click();
    h.run();
    assert_eq!(h.state().state.prefs.settings.library_folder, None);
    // 設定のファイルに入った棚の場所は、次の起動で読める
    h.state_mut().state.apply(Action::Prefs(PrefsAction::Set(Pref::LibraryFolder(Some(folder.clone())))));
    h.run();
    assert!(std::fs::read_to_string(&path).unwrap().contains("library_folder="));
    drop(h);
    let h = app_with_settings(&path, vec2(1280.0, 800.0));
    assert_eq!(h.state().state.prefs.settings.library_folder, Some(folder));
}

#[test]
fn broken_values_in_the_file_fall_back_one_by_one_and_say_why_in_the_status_bar() {
    let dir = settings_dir("broken");
    let path = dir.join("YoluPainter").join("settings.conf");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "language=en\nundo_budget_mib=lots\ncpu_threads=0\ncompositing=gpu\nexport_padding=16\n").unwrap();
    let before = std::fs::read_to_string(&path).unwrap();
    let h = app_with_settings(&path, vec2(1280.0, 800.0));
    let s = &h.state().state;
    assert_eq!(s.lang, Lang::En);
    assert_eq!(s.prefs.settings.undo_budget, Budget::Auto, "壊れた項目だけ既定");
    assert_eq!(s.prefs.settings.cpu_threads, None);
    assert_eq!(s.prefs.settings.compositing, Compositing::Gpu, "正しい項目は生かす");
    assert_eq!(s.export.padding, 16);
    assert!(s.message.contains("Undo history") && s.message.contains("CPU threads"), "{}", s.message);
    assert!(s.message.contains("lots"), "{}", s.message);
    // 読んだだけではファイルを書き換えない
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    drop(h);
    // 読めないファイルは全部既定（日本語）で、理由
    std::fs::write(&path, "junk without equals\n").unwrap();
    let h = app_with_settings(&path, vec2(1280.0, 800.0));
    assert_eq!(h.state().state.lang, Lang::Ja);
    assert_eq!(h.state().state.message, "設定を読めません。");
    // 何か選び直すと、新しい中身で置き換える
    drop(h);
    let mut h = app_with_settings(&path, vec2(1280.0, 800.0));
    h.state_mut().state.apply(Action::Prefs(PrefsAction::Set(Pref::Compositing(Compositing::Cpu))));
    h.run();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=ja\ncompositing=cpu\n");
}

#[test]
fn the_compositing_setting_reaches_the_canvas_display_at_startup_and_when_chosen() {
    use yolu_app::canvas::gpu::CanvasBackend;
    let dir = settings_dir("compositing");
    let path = dir.join("YoluPainter").join("settings.conf");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    // 設定のファイルの CPU は、起動からキャンバスの表示に入る
    std::fs::write(&path, "language=ja\ncompositing=cpu\n").unwrap();
    let mut h = app_with_settings(&path, vec2(1280.0, 800.0));
    assert_eq!(h.state().canvas_backend(), CanvasBackend::Cpu);
    // 窓で選ぶと、次のフレームから替わる（GPU・自動も）
    for (choice, want) in [(Compositing::Gpu, CanvasBackend::Gpu), (Compositing::Cpu, CanvasBackend::Cpu), (Compositing::Auto, CanvasBackend::from_env())] {
        h.state_mut().state.apply(Action::Prefs(PrefsAction::Set(Pref::Compositing(choice))));
        h.run();
        assert_eq!(h.state().canvas_backend(), want, "{choice:?}");
    }
    drop(h);
    // 自動（書いていない）は、試験が決めた方針を上書きしない
    std::fs::remove_file(&path).unwrap();
    let mut h = app_with_settings(&path, vec2(1280.0, 800.0));
    h.state_mut().set_canvas_backend(CanvasBackend::Gpu);
    h.run();
    assert_eq!(h.state().canvas_backend(), CanvasBackend::Gpu);
}

#[test]
fn headless_loading_allows_the_set_budget_but_never_less_than_the_core_default() {
    // 読み込み（.ylp を開く・書き出しが文書を戻す）は、設定の画素の予算まで読める。下げても、今まで読めた大きさは読める
    let mut s = Settings::default();
    assert_eq!(s.load_source_bytes(16384), 2048 * MIB, "自動（16 GB）");
    s.source_budget = Budget::Mib(8192);
    assert_eq!(s.load_source_bytes(16384), 8192 * MIB);
    s.source_budget = Budget::Mib(64);
    assert_eq!(s.load_source_bytes(16384), yolu_app::engine::DEFAULT_SOURCE_BUDGET_BYTES, "256 MiB を下回らない");
    // 状態からも同じ値（設定の窓で選ぶと変わる）
    let mut state = state();
    assert_eq!(state.load_source_bytes(), 2048 * MIB);
    set(&mut state, Pref::Budget(BudgetKind::Source, Budget::Mib(4096)));
    assert_eq!(state.load_source_bytes(), 4096 * MIB);
}
