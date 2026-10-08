//! 設定のウィンドウ（言語・書き出しの余白・メモリの予算・CPU のスレッド・表示の合成・棚の場所・退避を残す数）: 値の選びが画面の状態・文書の予算に効くこと、
//! 設定のファイルへの保存と起動での復元、壊れた値の理由、ウィンドウの操作と日英。`headless_` で始まる試験は画面を描かず、Wine でも回る。
use crate::common;

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

/// ディスクキャッシュを切った設定（画素の予算が設定の値そのものになる。キャッシュの入った予算は
/// `headless_the_disk_cache_adds_the_disk_limit_to_the_pixel_budget_and_follows_the_settings`）。
fn without_cache() -> Settings {
    Settings {
        disk_cache: false,
        ..Settings::default()
    }
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
    let before = (
        s.doc.undo_budget_bytes(),
        s.doc.source_budget_bytes(),
        s.doc.stroke_budget_bytes(),
    );
    s.sync_budgets();
    assert_eq!(
        (
            s.doc.undo_budget_bytes(),
            s.doc.source_budget_bytes(),
            s.doc.stroke_budget_bytes()
        ),
        before,
        "設定を読んでいない状態は、core の既定のまま"
    );
    // 設定を読むと自動の予算（16 GB: 取り消し 2048・レイヤーのメモリ 8192・1 回の操作 1024）
    s.load_settings(without_cache());
    s.sync_budgets();
    assert_eq!(s.doc.undo_budget_bytes(), 2048 * MIB);
    assert_eq!(s.doc.source_budget_bytes(), 8192 * MIB);
    assert_eq!(s.doc.stroke_budget_bytes(), 1024 * MIB);
    assert_eq!(s.doc.minimum_undo_steps(), 5);
}

#[test]
fn headless_choosing_a_budget_changes_the_current_document_and_new_documents_follow() {
    let mut s = state();
    s.load_settings(without_cache());
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
    assert_eq!(s.doc.undo_budget_bytes(), 2048 * MIB);
    // 新しいプロジェクト（新しい文書）にも、次の同期で入る
    set(&mut s, Pref::Budget(BudgetKind::Source, Budget::Mib(1024)));
    s.apply(Action::NewProject);
    s.sync_budgets();
    assert_eq!(s.doc.source_budget_bytes(), 1024 * MIB, "新しい文書");
    assert_eq!(s.doc.minimum_undo_steps(), 100);
}

/// 文書のレイヤーに、タイルを `count` 枚ぶん（先頭から）不透明に塗って、画素を確保する（1 タイル 128 × 128 × 4 バイト = 64 KiB）。
fn fill_tiles(
    doc: &mut Document,
    layer: LayerId,
    count: u32,
) -> Result<(), yolu_app::engine::CoreError> {
    let ts = doc.tile_size();
    let per_row = doc.width() / ts;
    for i in 0..count {
        doc.set_pixel(
            layer,
            (i % per_row) * ts,
            (i / per_row) * ts,
            Rgba8::new(1, 2, 3, 255),
        )?;
    }
    Ok(())
}

#[test]
fn headless_the_pixel_and_history_budgets_are_the_projects_total_shared_by_all_the_texture_sets() {
    let mut s = AppState::new(2048, 2048);
    s.prefs.ram_mib = 16384;
    let (_, shape) = s.receive_link_model(&two_sets_model());
    shape.expect("3D に読める");
    assert_eq!(s.sets.len(), 2);
    s.load_settings(without_cache());
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
    stroke
        .add_point(&mut s.doc, 20.5, 20.5, 1.0, yolu_app::engine::DVec2::ZERO)
        .unwrap();
    stroke
        .add_point(&mut s.doc, 90.5, 60.5, 1.0, yolu_app::engine::DVec2::ZERO)
        .unwrap();
    s.doc.end_stroke(stroke).unwrap();
    let (used, history) = (s.doc.allocated_bytes(), s.doc.history_bytes());
    assert_eq!(used, 100 * 65536);
    assert!(history > 0, "ストロークで履歴が増える");
    // 切り替えると、新しい今のセットには「設定 − ほかのセットの使用量」が入る
    s.switch_set(second).unwrap();
    assert_eq!(s.doc.source_budget_bytes(), 16 * MIB - used);
    assert_eq!(s.doc.undo_budget_bytes(), 64 * MIB - history);
    assert_eq!(
        s.doc.minimum_undo_steps(),
        9,
        "最小の取り消し段数はセットごとに残る"
    );
    // 2 つ目のセットは、残りまでしか塗れない（断られても何も壊れず、合計が設定を超えない）
    let layer = s.selected_layer.unwrap();
    let refused = fill_tiles(&mut s.doc, layer, 256).unwrap_err();
    assert_eq!(refused, yolu_app::engine::CoreError::SourceBudgetExceeded);
    let total = s.set_doc(first).allocated_bytes() + s.set_doc(second).allocated_bytes();
    assert!(total <= 16 * MIB, "{total}");
    assert!(total > 15 * MIB, "残りまで使える: {total}");
    // 戻ると、1 つ目のセットの予算は 2 つ目の使用量を引いた分（今の画素以上）
    s.switch_set(first).unwrap();
    assert_eq!(
        s.doc.source_budget_bytes(),
        16 * MIB - s.set_doc(second).allocated_bytes()
    );
    assert!(s.doc.source_budget_bytes() >= s.doc.allocated_bytes());
    // 設定を変えると、今のセットだけに次の同期で入る（全体の値から、ほかのセットの使用量を引いて）
    set(&mut s, Pref::Budget(BudgetKind::Source, Budget::Mib(64)));
    assert_eq!(
        s.doc.source_budget_bytes(),
        64 * MIB - s.set_doc(second).allocated_bytes()
    );
    s.switch_set(second).unwrap();
    assert_eq!(
        s.doc.source_budget_bytes(),
        64 * MIB - s.set_doc(first).allocated_bytes()
    );
    // 自動の予算でも同じ（16 GB: 画素 8192 MiB）
    set(&mut s, Pref::Budget(BudgetKind::Source, Budget::Auto));
    assert_eq!(
        s.doc.source_budget_bytes(),
        8192 * MIB - s.set_doc(first).allocated_bytes()
    );
}

#[test]
fn headless_the_largest_budgets_are_shared_between_the_sets_without_overflow() {
    // 自動の最大（物理メモリが十分にあるとき）: レイヤーのメモリ 64 GiB・取り消し 16 GiB・1 回の操作 4 GiB
    let mut s = AppState::new(2048, 2048);
    s.prefs.ram_mib = 1 << 20;
    let (_, shape) = s.receive_link_model(&two_sets_model());
    shape.expect("3D に読める");
    s.load_settings(without_cache());
    s.sync_budgets();
    let (first, second) = (s.sets.current_index(), 1 - s.sets.current_index());
    assert_eq!(s.doc.source_budget_bytes(), 65536 * MIB);
    assert_eq!(s.doc.undo_budget_bytes(), 16384 * MIB);
    assert_eq!(s.doc.stroke_budget_bytes(), 4096 * MIB);
    // 予約ではない: 予算が大きくても、使っていない間の画素は増えない
    assert!(
        s.doc.allocated_bytes() < 64 * MIB,
        "{}",
        s.doc.allocated_bytes()
    );
    // 使った分だけ、ほかのセットの予算から引かれる（全体が最大でも引き算が合う）
    let layer = s.selected_layer.unwrap();
    fill_tiles(&mut s.doc, layer, 100).unwrap();
    let used = s.doc.allocated_bytes();
    s.switch_set(second).unwrap();
    assert_eq!(s.doc.source_budget_bytes(), 65536 * MIB - used);
    assert_eq!(
        s.doc.undo_budget_bytes(),
        16384 * MIB - s.set_doc(first).history_bytes()
    );
    assert_eq!(s.doc.stroke_budget_bytes(), 4096 * MIB);
    // 設定の最大の値を選んでも同じ（範囲の上限）。1 つ上へは指定できない（範囲に丸める）
    set(&mut s, Pref::Budget(BudgetKind::Source, Budget::Mib(65536)));
    assert_eq!(s.prefs.settings.source_budget, Budget::Mib(65536));
    assert_eq!(s.doc.source_budget_bytes(), 65536 * MIB - used);
    set(&mut s, Pref::Budget(BudgetKind::Source, Budget::Mib(65537)));
    assert_eq!(
        s.prefs.settings.source_budget,
        Budget::Mib(65536),
        "上限に丸める"
    );
    // 読み込みの上限（.ylp を開く）も、桁あふれせず予算の値まで広がる
    assert_eq!(s.load_source_bytes(), 65536 * MIB);
}

#[test]
fn headless_a_project_already_over_the_pixel_budget_keeps_its_pixels_and_says_so_once() {
    let mut s = AppState::new(2048, 2048);
    s.prefs.ram_mib = 16384;
    let (_, shape) = s.receive_link_model(&two_sets_model());
    shape.expect("3D に読める");
    s.load_settings(without_cache());
    // 1 つ目のセットが 20 MiB（2048 × 2048 の 1 レイヤーは 16 MiB なので 2 レイヤー）使っているところへ、全体の予算 16 MiB
    let first = s.sets.current_index();
    for _ in 0..2 {
        s.apply(Action::NewLayer);
        let layer = s.selected_layer.unwrap();
        fill_tiles(&mut s.doc, layer, 160).unwrap();
    }
    s.message.clear();
    set(&mut s, Pref::Budget(BudgetKind::Source, Budget::Mib(16)));
    assert_eq!(
        s.doc.source_budget_bytes(),
        s.doc.allocated_bytes(),
        "画素は捨てず、予算を今の量まで広げる"
    );
    assert_eq!(
        s.message,
        "レイヤーのメモリがすでに予算を超えているので、予算を上げるまで追加できません。"
    );
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
    assert_eq!(
        s.message,
        "The layer memory is already over the budget; nothing can be added until it is raised."
    );
}

fn two_sets_model() -> yolu_protocol::Model {
    use yolu_protocol::{MaterialInfo, MaterialKey, MeshData, Model, Submesh, TextureProperty};
    let material = |name: &str| MaterialInfo {
        key: MaterialKey::Material {
            name: name.into(),
            asset: None,
        },
        shader: "Standard".into(),
        textures: vec![TextureProperty {
            name: "_MainTex".into(),
            width: 64,
            height: 64,
        }],
        routes: vec![],
    };
    let mesh = |name: &str, x: f32, material: u32| MeshData {
        key: name.into(),
        name: name.into(),
        skinned: false,
        positions: vec![[x, 0.0, 0.0], [x + 1.0, 0.0, 0.0], [x, 1.0, 0.0]],
        normals: vec![],
        uv0: vec![[0.1, 0.1], [0.4, 0.1], [0.1, 0.4]],
        submeshes: vec![Submesh {
            material,
            indices: vec![0, 2, 1],
        }],
    };
    Model {
        generation: 1,
        name: "二つ".into(),
        materials: vec![material("A"), material("B")],
        meshes: vec![mesh("a", 0.0, 0), mesh("b", 3.0, 1)],
    }
}

#[test]
fn headless_a_budget_is_not_changed_while_drawing_and_a_too_small_pixel_budget_widens_to_the_current_pixels(
) {
    let mut s = AppState::new(4096, 4096);
    s.prefs.ram_mib = 16384;
    s.load_settings(without_cache());
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
            s.doc
                .set_pixel(id, tx * 128, ty * 128, Rgba8::new(1, 2, 3, 255))
                .unwrap();
        }
    }
    assert!(
        s.doc.allocated_bytes() > 16 * MIB,
        "{}",
        s.doc.allocated_bytes()
    );
    s.message.clear();
    set(&mut s, Pref::Budget(BudgetKind::Source, Budget::Mib(16)));
    assert_eq!(
        s.doc.source_budget_bytes(),
        s.doc.allocated_bytes(),
        "予算を今の量まで広げた（足せない）"
    );
    assert_eq!(
        s.message,
        "レイヤーのメモリがすでに予算を超えているので、予算を上げるまで追加できません。"
    );
    // 選んだ値は設定に残る（画素の少ない文書なら効く）。何度も知らせ直さない
    assert_eq!(s.prefs.settings.source_budget, Budget::Mib(16));
    s.message.clear();
    s.sync_budgets();
    assert_eq!(s.message, "");
    s.apply(Action::M2Ui(UiOp::Language(Lang::En)));
    set(&mut s, Pref::Budget(BudgetKind::Source, Budget::Mib(64)));
    assert_eq!(s.doc.source_budget_bytes(), 64 * MIB);
    set(&mut s, Pref::Budget(BudgetKind::Source, Budget::Mib(16)));
    assert_eq!(
        s.message,
        "The layer memory is already over the budget; nothing can be added until it is raised."
    );
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
    set(
        &mut s,
        Pref::LibraryFolder(Some(PathBuf::from("relative/shelf"))),
    );
    assert_eq!(
        s.prefs.settings.library_folder,
        Some(folder),
        "相対パスは受けない"
    );
    assert_eq!(s.message, "ライブラリの場所は絶対パスで指定します。");
    s.apply(Action::M2Ui(UiOp::Language(Lang::En)));
    set(
        &mut s,
        Pref::LibraryFolder(Some(PathBuf::from("relative/shelf"))),
    );
    assert_eq!(s.message, "The library folder must be an absolute path.");
    set(&mut s, Pref::LibraryFolder(None));
    assert_eq!(s.prefs.settings.library_folder, None);
    // 言語は画面の言語が持ち、設定の取り出しで重なる
    assert_eq!(s.settings().lang, Lang::En);
    s.apply(Action::M2Ui(UiOp::Language(Lang::Ja)));
    assert_eq!(s.settings().lang, Lang::Ja);
    // 棚の場所を選ぶウィンドウを頼む
    s.apply(Action::Prefs(PrefsAction::ChooseLibraryFolder));
    assert_eq!(s.dialog_request, Some(DialogRequest::PrefsLibraryFolder));
    // ウィンドウの開け閉め
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
    assert_eq!(ja[0], "自動（2048 MiB）", "{ja:?}");
    assert!(ja.contains(&"0 MiB".to_string()) && ja.contains(&"8192 MiB".to_string()));
    assert_eq!(
        labels(&entries(&s, PrefChoice::Budget(BudgetKind::Stroke)))[0],
        "自動（1024 MiB）"
    );
    assert_eq!(
        labels(&entries(&s, PrefChoice::CpuThreads)),
        ["自動（8）", "1（並列にしない）", "2", "4", "8"]
    );
    assert_eq!(
        labels(&entries(&s, PrefChoice::ExportPadding)),
        [
            "なし",
            "2 テクセル",
            "4 テクセル",
            "8 テクセル",
            "16 テクセル",
            "32 テクセル",
            "64 テクセル",
            "届くかぎり"
        ]
    );
    assert_eq!(
        labels(&entries(&s, PrefChoice::Compositing)),
        ["自動", "GPU", "CPU"]
    );
    assert_eq!(
        labels(&entries(&s, PrefChoice::Language)),
        ["日本語", "English"]
    );
    // ファイルに書いた中途半端な数も、選択肢に出る（選び直せる）
    s.prefs.settings.undo_budget = Budget::Mib(777);
    s.prefs.settings.cpu_threads = Some(3);
    assert_eq!(
        labels(&entries(&s, PrefChoice::Budget(BudgetKind::Undo)))
            .last()
            .unwrap(),
        "777 MiB"
    );
    assert_eq!(
        labels(&entries(&s, PrefChoice::CpuThreads)).last().unwrap(),
        "3"
    );
    // 英語
    s.apply(Action::M2Ui(UiOp::Language(Lang::En)));
    assert_eq!(
        labels(&entries(&s, PrefChoice::Budget(BudgetKind::Undo)))[0],
        "Auto (2048 MiB)"
    );
    assert_eq!(
        labels(&entries(&s, PrefChoice::CpuThreads))[0],
        "Automatic (8)"
    );
    assert_eq!(labels(&entries(&s, PrefChoice::ExportPadding))[0], "Off");
    assert_eq!(
        labels(&entries(&s, PrefChoice::ExportPadding))[7],
        "Fill (all the way)"
    );
    // 1 コア（並列にしない）の機械は 1 が 1 つだけ
    s.prefs.cores = 1;
    s.prefs.settings.cpu_threads = None;
    assert_eq!(
        labels(&entries(&s, PrefChoice::CpuThreads)),
        ["Automatic (1)", "1 (no parallel work)"]
    );
}

// ───────── ウィンドウと設定のファイル ─────────

fn settings_dir(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/prefs-tests")
        .join(std::process::id().to_string())
        .join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 機械によらないキャッシュの置き場所（ウィンドウの絵に出る）と、そこの空きとして見せる量（自動のディスクの上限はその半分の 50 GiB）。
fn fixed_cache_folder() -> PathBuf {
    PathBuf::from(if cfg!(windows) { "C:\\Cache" } else { "/Cache" })
}
const FIXED_CACHE_FREE: u64 = 100 * 1024 * MIB;

fn app_with_settings(path: &Path, size: egui::Vec2) -> Harness<'static, YoluApp> {
    let path = path.to_path_buf();
    let mut h = common::gpu_thread::builder()
        .with_size(size)
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .renderer(common::shared_gpu::renderer())
        .build_eframe(move |cc| {
            YoluApp::for_context_with_settings(&cc.egui_ctx, Some(path), PenInput::detached())
                .with_render_state(cc.wgpu_render_state.as_ref())
        });
    h.state_mut().state.prefs.ram_mib = 16384;
    h.state_mut().state.prefs.cores = 8;
    // 作るときのフレームで測った値があれば置き換える
    let free = &mut h.state_mut().state.prefs.cache_free;
    free.retain(|(f, _)| *f != fixed_cache_folder());
    free.push((fixed_cache_folder(), Some(FIXED_CACHE_FREE)));
    h.run();
    h
}

fn open_settings(h: &mut Harness<'static, YoluApp>) {
    // 設定は編集のメニューの一番下（Unity の Edit ▸ Preferences と同じ）
    let at = menu_title(h, "編集").center();
    click(h, at);
    let item = popup_item(h, "設定…").center();
    click(h, item);
}

/// 同じ名前の要素のうち、いちばん下にあるもの。
fn lowest<'h>(h: &'h Harness<'_, YoluApp>, label: &'h str) -> egui_kittest::Node<'h> {
    h.query_all_by_label(label)
        .max_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .unwrap_or_else(|| panic!("「{label}」が無い"))
}

fn window_rect(h: &Harness<'_, YoluApp>) -> Rect {
    prefs::last_rect(&h.ctx).expect("設定のウィンドウが開いている")
}

/// ウィンドウの中だけを撮って、正解の絵と比べる（ほかのパネルの変更で壊れない）。
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
fn the_settings_window_opens_from_the_edit_menu_and_edits_every_value_into_the_file() {
    let dir = settings_dir("window");
    let path = dir.join("YoluPainter").join("settings.conf");
    let mut h = app_with_settings(&path, vec2(1280.0, 800.0));
    assert!(!h.state().state.prefs.open);
    open_settings(&mut h);
    assert!(h.state().state.prefs.open);
    // 棚の場所は、機械によらない場所にして撮る（既定の場所は設定のフォルダの下で、機械で違う）
    let shelf = PathBuf::from(if cfg!(windows) {
        "C:\\Library"
    } else {
        "/Library"
    });
    h.state_mut()
        .state
        .apply(Action::Prefs(PrefsAction::Set(Pref::LibraryFolder(Some(
            shelf,
        )))));
    // キャッシュの置き場所も（既定は OS の一時フォルダで、機械で違う）
    h.state_mut()
        .state
        .apply(Action::Prefs(PrefsAction::Set(Pref::DiskCacheFolder(
            Some(fixed_cache_folder()),
        ))));
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
    pick(&mut h, "取り消し履歴: 自動（2048 MiB）", "512 MiB");
    assert_eq!(h.state().state.doc.undo_budget_bytes(), 512 * MIB);
    pick(&mut h, "レイヤーのメモリ: 自動（8192 MiB）", "4096 MiB");
    // ディスクキャッシュが入（既定）: 画素の予算は、メモリの上限（レイヤーのメモリ＋取り消し履歴）＋ディスクの上限（空き 100 GiB の半分）
    let with_cache = (4096 + 512) * MIB + FIXED_CACHE_FREE / 2;
    assert_eq!(h.state().state.doc.source_budget_bytes(), with_cache);
    pick(&mut h, "1 回の操作: 自動（1024 MiB）", "256 MiB");
    assert_eq!(h.state().state.doc.stroke_budget_bytes(), 256 * MIB);
    pick(&mut h, "表示の合成: 自動", "CPU");
    assert_eq!(h.state().state.prefs.settings.compositing, Compositing::Cpu);
    // CPU のスレッドは次の起動から効く（ウィンドウに出る）
    pick(&mut h, "CPU のスレッド: 自動（8）", "4");
    assert_eq!(h.state().state.prefs.settings.cpu_threads, Some(4));
    let _ = h.get_by_label("CPU のスレッド: 4 ・ 再起動で反映");
    h.get_by_label("Unity の Live Link を受け付ける").click();
    h.run();
    assert!(!h.state().state.settings().livelink_on_startup);
    shot(&mut h, "prefs_window_changed");
    // 言語（選ぶとウィンドウの文言も替わる）
    pick(&mut h, "言語: 日本語", "English");
    assert_eq!(h.state().state.lang, Lang::En);
    let _ = h.get_by_label("Language: English");
    let _ = h.get_by_label("CPU threads: 4 · applies after restart");
    shot(&mut h, "prefs_window_english");
    // ウィンドウの中に全部収まっている（最後の行の下も）
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
    assert_eq!(s.doc.source_budget_bytes(), with_cache);
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
    assert!(
        h.state().state.prefs.settings.min_undo_steps > 50,
        "{}",
        h.state().state.prefs.settings.min_undo_steps
    );
    assert_eq!(
        h.state().state.doc.minimum_undo_steps(),
        h.state().state.prefs.settings.min_undo_steps as usize
    );
    // 棚の場所: 初めは既定（「既定に戻す」は押せない）。選ぶウィンドウを頼み、選んだ場所が出る。ボタンの名前はキャッシュの場所と同じなので、
    // 下の節（ファイル）の方を押す
    assert_eq!(h.state().state.prefs.settings.library_folder, None);
    lowest(&h, "選ぶ…").click();
    h.run();
    assert_eq!(
        h.state().state.dialog_request,
        Some(DialogRequest::PrefsLibraryFolder)
    );
    let folder = dir.join("MyLibrary");
    h.state_mut()
        .state
        .apply(Action::Prefs(PrefsAction::Set(Pref::LibraryFolder(Some(
            folder.clone(),
        )))));
    h.run();
    assert_eq!(
        h.state().state.prefs.settings.library_folder,
        Some(folder.clone())
    );
    lowest(&h, "既定に戻す").click();
    h.run();
    assert_eq!(h.state().state.prefs.settings.library_folder, None);
    // 設定のファイルに入った棚の場所は、次の起動で読める
    h.state_mut()
        .state
        .apply(Action::Prefs(PrefsAction::Set(Pref::LibraryFolder(Some(
            folder.clone(),
        )))));
    h.run();
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("library_folder="));
    drop(h);
    let h = app_with_settings(&path, vec2(1280.0, 800.0));
    assert_eq!(h.state().state.prefs.settings.library_folder, Some(folder));
}

#[test]
fn broken_values_in_the_file_fall_back_one_by_one_and_say_why_in_the_status_bar() {
    let dir = settings_dir("broken");
    let path = dir.join("YoluPainter").join("settings.conf");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "language=en\nundo_budget_mib=lots\ncpu_threads=0\ncompositing=gpu\nexport_padding=16\n",
    )
    .unwrap();
    let before = std::fs::read_to_string(&path).unwrap();
    let h = app_with_settings(&path, vec2(1280.0, 800.0));
    let s = &h.state().state;
    assert_eq!(s.lang, Lang::En);
    assert_eq!(
        s.prefs.settings.undo_budget,
        Budget::Auto,
        "壊れた項目だけ既定"
    );
    assert_eq!(s.prefs.settings.cpu_threads, None);
    assert_eq!(
        s.prefs.settings.compositing,
        Compositing::Gpu,
        "正しい項目は生かす"
    );
    assert_eq!(s.export.padding, 16);
    assert!(
        s.message.contains("Undo history") && s.message.contains("CPU threads"),
        "{}",
        s.message
    );
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
    h.state_mut()
        .state
        .apply(Action::Prefs(PrefsAction::Set(Pref::Compositing(
            Compositing::Cpu,
        ))));
    h.run();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "language=ja\ncompositing=cpu\n"
    );
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
    // ウィンドウで選ぶと、次のフレームから替わる（GPU・自動も）
    for (choice, want) in [
        (Compositing::Gpu, CanvasBackend::Gpu),
        (Compositing::Cpu, CanvasBackend::Cpu),
        (Compositing::Auto, CanvasBackend::from_env()),
    ] {
        h.state_mut()
            .state
            .apply(Action::Prefs(PrefsAction::Set(Pref::Compositing(choice))));
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
    assert_eq!(s.load_source_bytes(16384), 8192 * MIB, "自動（16 GB）");
    s.source_budget = Budget::Mib(8192);
    assert_eq!(s.load_source_bytes(16384), 8192 * MIB);
    s.source_budget = Budget::Mib(64);
    assert_eq!(
        s.load_source_bytes(16384),
        yolu_app::engine::DEFAULT_SOURCE_BUDGET_BYTES,
        "256 MiB を下回らない"
    );
    // 状態からも同じ値（設定のウィンドウで選ぶと変わる）
    let mut state = state();
    assert_eq!(state.load_source_bytes(), 8192 * MIB);
    set(&mut state, Pref::DiskCache(false));
    set(
        &mut state,
        Pref::Budget(BudgetKind::Source, Budget::Mib(4096)),
    );
    assert_eq!(state.load_source_bytes(), 4096 * MIB);
}

// ───────── ウィンドウの収まり・メニューの位置とキー・3D ビューの視点の中心 ─────────

fn drawn_texts(h: &Harness<'_, YoluApp>) -> Vec<(String, Rect)> {
    use egui::epaint::Shape;
    fn walk(shape: &Shape, clip: Rect, out: &mut Vec<(String, Rect)>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, clip, out)),
            Shape::Text(t) => {
                let r = t.galley.rect.translate(t.pos.to_vec2());
                out.push((t.galley.job.text.clone(), r.intersect(clip)));
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for s in &h.output().shapes {
        // 切り取られた外の文字は画面に出ない（共通のスクロールで隠れた行）
        walk(&s.shape, s.clip_rect, &mut out);
    }
    out
}

fn english(h: &mut Harness<'static, YoluApp>, lang: Lang) {
    h.state_mut().state.lang = lang;
    h.run();
}

/// ウィンドウの高さの見積もり（描く行の数と合わせた表）が、実際に並べた高さと同じ（行を足して数え違えると、最後の行がウィンドウからはみ出す）。
/// GPU とディスクキャッシュの詳しくの開け閉め・外からの操作の入り切（入っている間だけポート番号の行が出る）のどれでも。
#[test]
fn the_window_height_is_exactly_the_rows_it_lays_out_open_or_closed_in_both_languages() {
    let dir = settings_dir("height");
    let path = dir.join("YoluPainter").join("settings.conf");
    let mut h = app_with_settings(&path, vec2(1600.0, 1100.0));
    // 外からの操作を入れても、ほかの試験・開いているアプリと番号が重ならない（0: OS が空いた番号を選ぶ）
    h.state_mut().state.prefs.settings.external_ops_port = 0;
    open_settings(&mut h);
    for lang in Lang::ALL {
        english(&mut h, lang);
        for ops in [false, true] {
            h.state_mut().state.prefs.settings.external_ops = ops;
            // 「ペン」の節は macOS だけに出る。ほかの OS でも、出したときの高さを確かめる
            for pen in [false, true] {
                h.state_mut().state.prefs.tablet_row = pen;
                for (details, cache) in [(false, false), (true, false), (false, true), (true, true)]
                {
                    h.state_mut()
                        .state
                        .apply(Action::Prefs(PrefsAction::GpuDetails(details)));
                    h.state_mut()
                        .state
                        .apply(Action::Prefs(PrefsAction::CacheDetails(cache)));
                    h.run();
                    h.run();
                    let window = window_rect(&h);
                    let drawn = prefs::drawn_content_height(&h.ctx).expect("中身を並べた");
                    let body = window.height() - yolu_app::ui::window::HEADER_HEIGHT;
                    assert!(
                        (body - drawn).abs() < 0.5,
                        "{lang:?} 外からの操作={ops} ペンの節={pen} 詳しく={details} キャッシュの詳しく={cache}: ウィンドウの中身 {body} と、並べた高さ {drawn} が違う"
                    );
                }
            }
        }
    }
    h.state_mut().state.prefs.settings.external_ops = false;
    h.run();
}

/// 「ペン」の節（タブレットの筆圧（試し））は macOS だけに出て、切り替えがペンの受け口の札と設定のファイルに届き、次の起動でも切のまま。
#[test]
fn the_pen_section_has_the_tablet_pressure_toggle_which_reaches_the_pen_input_and_survives_a_restart(
) {
    let dir = settings_dir("tablet");
    let path = dir.join("YoluPainter").join("settings.conf");
    let mut h = app_with_settings(&path, vec2(1280.0, 900.0));
    open_settings(&mut h);
    // 節が出るのは macOS だけ
    assert_eq!(h.state().state.prefs.tablet_row, cfg!(target_os = "macos"));
    // 設定のウィンドウの中に描いた文字だけ（ツールの名前などにも「ペン」がある）
    let shown = |h: &Harness<'_, YoluApp>, text: &str| {
        let window = window_rect(h);
        drawn_texts(h)
            .iter()
            .any(|(t, r)| t == text && window.contains_rect(*r))
    };
    if !cfg!(target_os = "macos") {
        assert!(!shown(&h, "ペン"), "macOS 以外には節が無い");
        assert!(h.query_by_label("タブレットの筆圧（試し）").is_none());
        h.state_mut().state.prefs.tablet_row = true;
        h.run();
        h.run();
    }
    assert!(shown(&h, "ペン"), "節の見出し");
    // 既定は入。札は設定から合わせる
    assert!(h.state().state.settings().tablet_pressure);
    assert!(h.state().pen().tablet_on());
    // ライブラリの場所は、機械によらない場所にして撮る（既定の場所は設定のフォルダの下で、機械で違う）
    let library = PathBuf::from(if cfg!(windows) {
        "C:\\Library"
    } else {
        "/Library"
    });
    h.state_mut()
        .state
        .apply(Action::Prefs(PrefsAction::Set(Pref::LibraryFolder(Some(
            library,
        )))));
    h.run();
    shot(&mut h, "prefs_window_pen");
    // 切る
    h.get_by_label("タブレットの筆圧（試し）").click();
    h.run();
    h.run();
    assert!(!h.state().state.settings().tablet_pressure);
    assert!(!h.state().pen().tablet_on(), "ペンの受け口に届く");
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.lines().any(|l| l == "tablet_pressure=off"),
        "{written}"
    );
    // 英語でも節と名前が出る
    english(&mut h, Lang::En);
    assert!(shown(&h, "Pen"), "節の見出し");
    let _ = h.get_by_label("Tablet pressure (experimental)");
    // 入れ直すと行が消える
    h.get_by_label("Tablet pressure (experimental)").click();
    h.run();
    h.run();
    assert!(h.state().state.settings().tablet_pressure && h.state().pen().tablet_on());
    assert!(!std::fs::read_to_string(&path)
        .unwrap()
        .contains("tablet_pressure"));
    // 切って終わると、次の起動も切（札も切）
    h.get_by_label("Tablet pressure (experimental)").click();
    h.run();
    h.run();
    drop(h);
    let h = app_with_settings(&path, vec2(1280.0, 900.0));
    assert!(!h.state().state.settings().tablet_pressure);
    assert!(
        !h.state().pen().tablet_on(),
        "起動のとき、設定の値が札に入る"
    );
}

/// どの大きさのウィンドウでも、最後の行（UV ワイヤーフレームでなく、いちばん下の「すべて残す」）まで届く: 収まらない低い画面では共通のスクロールで送り、
/// 横にははみ出さない。日英・いちばん小さいウィンドウ（960 × 640）。
#[test]
fn every_row_fits_the_window_or_scrolls_into_view_in_the_smallest_window_in_both_languages() {
    let dir = settings_dir("small");
    let path = dir.join("YoluPainter").join("settings.conf");
    for lang in Lang::ALL {
        let mut h = app_with_settings(&path, vec2(960.0, 640.0));
        // 小さいウィンドウでは編集のメニューも長くてポップアップの中で送るので、ウィンドウはキーで開く
        key(&h, egui::Key::Comma, egui::Modifiers::COMMAND);
        h.run();
        assert!(h.state().state.prefs.open);
        english(&mut h, lang);
        let window = window_rect(&h);
        let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(960.0, 640.0));
        assert!(screen.contains_rect(window), "{lang:?}: {window:?}");
        // 横: どの文字もウィンドウの幅に収まる（見える所だけ）
        for (text, r) in drawn_texts(&h)
            .into_iter()
            .filter(|(_, r)| r.height() > 0.0 && window.contains(r.center()))
        {
            assert!(
                r.left() >= window.left() - 0.5 && r.right() <= window.right() + 0.5,
                "{lang:?}: ウィンドウの横からはみ出す「{text}」{r:?} {window:?}"
            );
        }
        // 縦: 低い画面では中身がウィンドウに収まらないので、つまみがある。ホイールで一番下まで送ると、最後の行がウィンドウの中に入る
        let bar_in = |h: &Harness<'static, YoluApp>| {
            h.query_all_by_role(egui::accesskit::Role::ScrollBar)
                .map(|n| n.rect())
                .find(|r| window.contains_rect(*r))
        };
        let bar = bar_in(&h)
            .unwrap_or_else(|| panic!("{lang:?}: 収まらないのに、ウィンドウの中につまみが無い"));
        assert!(bar.height() > 100.0, "{bar:?}");
        let keep_all = lang.pick("すべて残す", "Keep all");
        h.event(egui::Event::PointerMoved(
            window.center() - vec2(100.0, 0.0),
        ));
        h.step();
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -5000.0),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        });
        h.run();
        h.run();
        let last = h.get_by_label(keep_all).rect();
        assert!(
            window.contains_rect(last),
            "{lang:?}: 一番下の行 {last:?} がウィンドウ {window:?} の外"
        );
        // つまみを掴んで一番上へ戻せる
        let bar = bar_in(&h).expect("ウィンドウのつまみ");
        let grab = egui::pos2(bar.center().x, bar.bottom() - 6.0);
        drag(&mut h, &[grab, egui::pos2(grab.x, bar.top() - 50.0)]);
        h.run();
        let name = lang.pick("言語", "Language");
        assert!(
            drawn_texts(&h)
                .iter()
                .any(|(t, r)| t == name && window.contains(r.center())),
            "{lang:?}: 一番上へ戻すと最初の行が見える"
        );
    }
}

/// 設定は編集のメニューの一番下（区切りの後。Unity の Edit ▸ Preferences と同じ）で、表示のメニューには無い。
#[test]
fn settings_is_the_last_item_of_the_edit_menu_after_a_separator_and_not_in_the_view_menu() {
    use yolu_app::ui::menu::Entry;
    for lang in Lang::ALL {
        let mut s = state();
        s.lang = lang;
        let edit = yolu_app::shell::menu_entries(&s, 1);
        let Some(Entry::Item {
            label,
            action,
            shortcut,
            ..
        }) = edit.last()
        else {
            panic!("{lang:?}: 編集のメニューの最後が項目でない");
        };
        assert_eq!(*action, Action::Prefs(PrefsAction::Open));
        assert_eq!(label, lang.pick("設定…", "Settings…"));
        assert_eq!(shortcut.as_deref(), Some("Ctrl+,"));
        assert!(
            matches!(edit[edit.len() - 2], Entry::Separator),
            "{lang:?}: 設定の前に区切り"
        );
        for index in 0..7 {
            if index == 1 {
                continue;
            }
            let entries = yolu_app::shell::menu_entries(&s, index);
            assert!(
                !yolu_app::ui::menu::leaves(&entries)
                    .iter()
                    .any(|e| matches!(
                        e,
                        Entry::Item {
                            action: Action::Prefs(_),
                            ..
                        }
                    )),
                "{lang:?}: 編集以外のメニュー {index} に設定がある"
            );
        }
    }
}

/// Ctrl+, で設定のウィンドウが開き、キーの一覧にも出る（名前はメニューと同じ）。
#[test]
fn ctrl_comma_opens_the_settings_and_the_shortcut_list_names_it() {
    let dir = settings_dir("key");
    let path = dir.join("YoluPainter").join("settings.conf");
    let mut h = app_with_settings(&path, vec2(1280.0, 800.0));
    assert!(!h.state().state.prefs.open);
    key(&h, egui::Key::Comma, egui::Modifiers::COMMAND);
    h.run();
    assert!(h.state().state.prefs.open, "Ctrl+, で開く");
    // 開いたままもう一度押しても、そのまま（閉じも、二重にもならない）
    key(&h, egui::Key::Comma, egui::Modifiers::COMMAND);
    h.run();
    assert!(h.state().state.prefs.open);
    // キーの一覧（読むだけのウィンドウ）にある
    let binding = yolu_app::shortcuts::bindings()
        .into_iter()
        .find(|b| b.action() == Some(Action::Prefs(PrefsAction::Open)))
        .expect("一覧に設定のキーがある");
    assert_eq!(yolu_app::shortcuts::key_label(&binding), "Ctrl+,");
    for lang in Lang::ALL {
        h.state_mut().state.lang = lang;
        assert_eq!(
            yolu_app::shortcuts::action_label(&h.state().state, &binding.action().unwrap())
                .as_deref(),
            Some(lang.pick("設定…", "Settings…"))
        );
    }
    assert_eq!(
        yolu_app::shortcuts::shortcut_text(&Action::Prefs(PrefsAction::Open)).as_deref(),
        Some("Ctrl+,")
    );
}

/// 3D の視点の中心（回転・ズーム）を設定のウィンドウの「3D ビュー」の節でも選べる。3D ビューの表示の設定の「視点」と同じ値で、設定のファイルに
/// 書かれ、次の起動で戻る。
#[test]
fn the_orbit_and_zoom_centers_are_chosen_in_the_3d_view_section_and_survive_a_restart() {
    use yolu_app::view3d::navigation::{OrbitCenter, ZoomCenter};
    // 選択肢は 4 つと 2 つ。今の値に印
    let mut s = state();
    for (choice, count) in [(PrefChoice::OrbitCenter, 4), (PrefChoice::ZoomCenter, 2)] {
        let e = entries(&s, choice);
        assert_eq!(e.len(), count);
        assert_eq!(labels(&e).len(), count);
    }
    assert_eq!(
        labels(&entries(&s, PrefChoice::OrbitCenter)),
        OrbitCenter::ALL
            .map(|c| c.label(s.lang).to_owned())
            .to_vec()
    );
    set(&mut s, Pref::OrbitCenter(OrbitCenter::TextureSet));
    set(&mut s, Pref::ZoomCenter(ZoomCenter::Pointer));
    assert_eq!(s.prefs.settings.navigation.orbit, OrbitCenter::TextureSet);
    assert_eq!(s.prefs.settings.navigation.zoom, ZoomCenter::Pointer);

    // ウィンドウで選ぶ（節の見出しと値の箱）
    let dir = settings_dir("pivot");
    let path = dir.join("YoluPainter").join("settings.conf");
    let mut h = app_with_settings(&path, vec2(1280.0, 800.0));
    open_settings(&mut h);
    assert!(
        drawn_texts(&h).iter().any(|(t, _)| t == "3D ビュー"),
        "節の見出し"
    );
    let pick = |h: &mut Harness<'static, YoluApp>, label: &str, item: &str| {
        let at = h.get_by_label(label).rect().center();
        click(h, at);
        let at = popup_item(h, item).center();
        click(h, at);
    };
    assert_eq!(
        h.state().state.prefs.settings.navigation.orbit,
        OrbitCenter::View
    );
    pick(&mut h, "回転の中心: 画面の中心", "面の位置（自動深度）");
    assert_eq!(
        h.state().state.prefs.settings.navigation.orbit,
        OrbitCenter::Surface
    );
    pick(&mut h, "回転の中心: 面の位置（自動深度）", "モデルの中心");
    assert_eq!(
        h.state().state.prefs.settings.navigation.orbit,
        OrbitCenter::Model
    );
    pick(&mut h, "ズームの中心: 画面の中心へ", "ポインタの所へ");
    assert_eq!(
        h.state().state.prefs.settings.navigation.zoom,
        ZoomCenter::Pointer
    );
    // 3D ビューの表示の設定と同じ値（見る口が同じ）
    assert_eq!(
        h.state().state.settings().navigation.orbit,
        OrbitCenter::Model
    );
    h.run();
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.lines().any(|l| l == "view3d_orbit=model"),
        "{written}"
    );
    assert!(
        written.lines().any(|l| l == "view3d_zoom=pointer"),
        "{written}"
    );
    drop(h);
    let h = app_with_settings(&path, vec2(1280.0, 800.0));
    assert_eq!(
        h.state().state.prefs.settings.navigation.orbit,
        OrbitCenter::Model
    );
    assert_eq!(
        h.state().state.prefs.settings.navigation.zoom,
        ZoomCenter::Pointer
    );
    // 英語でも節と値の名前が出る
    let mut h = h;
    open_settings_in(&mut h, Lang::En);
    assert!(
        drawn_texts(&h).iter().any(|(t, _)| t == "3D View"),
        "節の見出し"
    );
    let _ = h.get_by_label("Orbit center: Model center");
    let _ = h.get_by_label("Zoom center: Toward pointer");
}

fn open_settings_in(h: &mut Harness<'static, YoluApp>, lang: Lang) {
    h.state_mut().state.lang = lang;
    h.run();
    h.state_mut().state.apply(Action::Prefs(PrefsAction::Open));
    h.run();
}

/// 状態の帯の右端（丸のあたり）だけを撮って、正解の絵と比べる。
fn status_bar_shot(h: &mut Harness<'_, YoluApp>, name: &str) {
    h.event(egui::Event::PointerGone);
    h.step();
    let image = h.render().expect("描画");
    let height = yolu_app::ui::theme::STATUS_BAR_HEIGHT.ceil() as u32;
    let cropped = image::imageops::crop_imm(
        &image,
        image.width() - 120,
        image.height() - height,
        120,
        height,
    )
    .to_image();
    egui_kittest::image_snapshot(&cropped, name);
}

/// 「外からの操作を受ける」: 既定は切。欄を押すと 127.0.0.1 の番号で待ち受けて設定のファイルに書かれ、状態の帯の右端に小さな丸（説明は
/// ツールチップ。つなぐ側に渡す URL つき。日英）が出る。つながると丸の色が替わり、もう一度押すと待ち受けをやめ、丸も消える。
/// 番号の欄に打つと、その番号で待ち直す（範囲の外は断る）。
#[test]
fn the_external_commands_row_turns_listening_on_and_off_and_the_status_bar_shows_a_dot() {
    let dir = settings_dir("ops");
    let path = dir.join("YoluPainter").join("settings.conf");
    let mut h = app_with_settings(&path, vec2(1280.0, 800.0));
    // 試験ごとに空いた番号で受ける（ほかの試験・開いているアプリと重ならない）
    let free = || {
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        l.local_addr().unwrap().port()
    };
    let port = free();
    h.state_mut().state.prefs.settings.external_ops_port = port;
    let waiting = |port: u16| format!("外からの操作を待っています（http://127.0.0.1:{port}/mcp）");
    assert!(
        h.query_by_label(&waiting(port)).is_none(),
        "切のあいだは丸が無い"
    );
    open_settings(&mut h);
    assert!(!h.state().state.prefs.settings.external_ops, "既定は切");
    h.get_by_label("外からの操作を受ける").click();
    h.run();
    assert!(h.state().state.settings().external_ops);
    assert_eq!(
        h.state().state.ops.status,
        yolu_app::mcp_server::OpsStatus::Listening
    );
    // まだ誰もつないでいない: 丸は「待っている」
    status_bar_shot(&mut h, "status_bar_ops_listening");
    let dot = h.get_by_label(&waiting(port)).rect();
    let bar = Rect::from_min_max(
        egui::pos2(0.0, 800.0 - yolu_app::ui::theme::STATUS_BAR_HEIGHT),
        egui::pos2(1280.0, 800.0),
    );
    assert!(
        bar.contains_rect(dot),
        "状態の帯の中にある: {bar:?} {dot:?}"
    );
    assert!(dot.right() > bar.right() - 20.0, "右端にある: {dot:?}");
    h.run();
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.lines().any(|l| l == "external_ops=on"), "{written}");
    assert!(
        written
            .lines()
            .any(|l| l == format!("external_ops_port={port}")),
        "{written}"
    );
    // つながると、丸は「つながっている」になる（色が替わる。ツールチップも）
    let held = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while h.state().state.ops.status != yolu_app::mcp_server::OpsStatus::Connected(1) {
        assert!(
            std::time::Instant::now() < deadline,
            "つながらない: {:?}",
            h.state().state.ops.status
        );
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    h.run();
    let _ = h.get_by_label(&format!(
        "外からの操作を受けています（つながり 1・http://127.0.0.1:{port}/mcp）"
    ));
    status_bar_shot(&mut h, "status_bar_ops_connected");
    // 英語の表示
    english(&mut h, Lang::En);
    let _ = h.get_by_label(&format!(
        "Accepting external commands (1 connected, http://127.0.0.1:{port}/mcp)"
    ));
    let _ = h.get_by_label("Accept external commands");
    english(&mut h, Lang::Ja);
    drop(held);
    // 番号の欄（外からの操作の行のすぐ下の文字の欄）に打つと、その番号で待ち直す
    let toggle = h.get_by_label("外からの操作を受ける").rect();
    let field = h
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .map(|n| n.rect())
        .find(|r| r.top() > toggle.bottom() && r.top() < toggle.bottom() + 40.0)
        .expect("番号の欄");
    let moved = free();
    click(&mut h, field.center());
    key(&h, egui::Key::A, egui::Modifiers::COMMAND);
    h.event(egui::Event::Text(moved.to_string()));
    key(&h, egui::Key::Enter, egui::Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.settings().external_ops_port, moved);
    assert_eq!(h.state().ops().port(), Some(moved));
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err());
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .any(|l| l == format!("external_ops_port={moved}")));
    // 範囲の外は断る（番号は変わらず、理由を知らせる）
    click(&mut h, field.center());
    key(&h, egui::Key::A, egui::Modifiers::COMMAND);
    h.event(egui::Event::Text("80".into()));
    key(&h, egui::Key::Enter, egui::Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.settings().external_ops_port, moved);
    assert_eq!(h.state().state.message, "ポート番号は 1024〜65535 です。");
    // 切る: 待ち受けをやめ、丸も消える
    h.get_by_label("外からの操作を受ける").click();
    h.run();
    assert!(!h.state().state.settings().external_ops);
    assert_eq!(
        h.state().state.ops.status,
        yolu_app::mcp_server::OpsStatus::Off
    );
    assert!(h.query_by_label(&waiting(moved)).is_none());
    assert!(std::net::TcpStream::connect(("127.0.0.1", moved)).is_err());
    h.run();
    assert!(!std::fs::read_to_string(&path)
        .unwrap()
        .contains("external_ops=on"));
}

/// ポート番号の行は、「外からの操作を受ける」を入れている間だけ、そのすぐ下に出る（切ると消え、ウィンドウも 1 行分低くなる）。日英の絵。
#[test]
fn the_port_row_appears_below_external_commands_only_while_it_is_on() {
    let dir = settings_dir("ops-port");
    let path = dir.join("YoluPainter").join("settings.conf");
    let mut h = app_with_settings(&path, vec2(1280.0, 800.0));
    // 棚の場所は、機械によらない場所にして撮る
    let shelf = PathBuf::from(if cfg!(windows) {
        "C:\\Library"
    } else {
        "/Library"
    });
    h.state_mut()
        .state
        .apply(Action::Prefs(PrefsAction::Set(Pref::LibraryFolder(Some(
            shelf,
        )))));
    // 絵には既定の番号を出す。試験のアプリがその番号で外からの要求を受けないよう、先に取っておく（取れなければ、ほかのプログラムが
    // 使っている。どちらでも試験のアプリは待てず、ウィンドウの中身は同じ）
    let _held = std::net::TcpListener::bind(("127.0.0.1", yolu_mcp::DEFAULT_PORT));
    open_settings(&mut h);
    let has_port_row = |h: &Harness<'_, YoluApp>, label: &str| {
        drawn_texts(h).iter().any(|(text, _)| text == label)
    };
    assert!(!has_port_row(&h, "ポート番号"), "切のあいだは出ない");
    let closed = window_rect(&h);
    h.get_by_label("外からの操作を受ける").click();
    h.run();
    assert!(h.state().state.settings().external_ops);
    let toggle = h.get_by_label("外からの操作を受ける").rect();
    let (_, label) = drawn_texts(&h)
        .into_iter()
        .find(|(text, _)| text == "ポート番号")
        .expect("入れるとポート番号の行が出る");
    assert!(
        label.top() > toggle.bottom() && label.top() < toggle.bottom() + 40.0,
        "外からの操作の行のすぐ下: {toggle:?} {label:?}"
    );
    let field = h
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .map(|n| n.rect())
        .find(|r| r.top() > toggle.bottom() && r.top() < toggle.bottom() + 40.0)
        .expect("ポート番号の欄");
    assert!(
        (field.center().y - label.center().y).abs() < 4.0,
        "名前と欄は同じ行"
    );
    let open = window_rect(&h);
    assert!(open.height() > closed.height(), "{closed:?} {open:?}");
    // 待てなかった知らせは状態の帯の丸の試験が見る。ここではウィンドウだけを撮る
    h.state_mut().state.clear_message();
    h.run();
    shot(&mut h, "prefs_window_external_ops");
    english(&mut h, Lang::En);
    assert!(has_port_row(&h, "Port"));
    h.state_mut().state.clear_message();
    h.run();
    shot(&mut h, "prefs_window_external_ops_english");
    english(&mut h, Lang::Ja);
    // 切ると消える
    h.get_by_label("外からの操作を受ける").click();
    h.run();
    assert!(!h.state().state.settings().external_ops);
    assert!(!has_port_row(&h, "ポート番号"), "切ると消える");
    assert!((window_rect(&h).height() - closed.height()).abs() < 0.5);
}

/// ディスクキャッシュが入（既定）なら、今のセットの画素の予算は「メモリの上限（レイヤーのメモリ＋取り消し履歴）＋ディスクの上限 − ほかの
/// セットの画素」。自動のディスクの上限は 64 GiB と置き場所の空きの半分の小さい方（空きは置き場所で初めて測った値）。切ると、キャッシュの無い
/// 予算に戻る。
#[test]
fn headless_the_disk_cache_adds_the_disk_limit_to_the_pixel_budget_and_follows_the_settings() {
    use yolu_app::settings::DiskLimit;
    const GIB: u64 = 1024 * MIB;
    let mut s = state();
    s.prefs
        .cache_free
        .push((std::env::temp_dir(), Some(40 * GIB)));
    s.load_settings(Settings::default());
    s.sync_budgets();
    // 16 GB: レイヤーのメモリ 8 GiB・取り消し 2 GiB、空き 40 GiB の半分 20 GiB
    assert_eq!(s.doc.source_budget_bytes(), 10 * GIB + 20 * GIB);
    assert_eq!(s.load_source_bytes(), 30 * GIB);
    assert_eq!(
        s.doc.undo_budget_bytes(),
        2 * GIB,
        "取り消しの予算は変えない"
    );
    set(&mut s, Pref::DiskCacheLimit(DiskLimit::Gib(8)));
    assert_eq!(s.doc.source_budget_bytes(), 18 * GIB);
    set(&mut s, Pref::DiskCacheLimit(DiskLimit::Gib(99_999)));
    assert_eq!(
        s.prefs.settings.disk_cache_limit,
        DiskLimit::Gib(4096),
        "範囲に丸める"
    );
    set(&mut s, Pref::DiskCacheLimit(DiskLimit::Auto));
    // 空きの多い置き場所では 64 GiB まで
    let roomy = std::env::temp_dir().join("roomy");
    s.prefs.cache_free.push((roomy.clone(), Some(1000 * GIB)));
    set(&mut s, Pref::DiskCacheFolder(Some(roomy.clone())));
    assert_eq!(s.prefs.settings.disk_cache_folder, Some(roomy));
    assert_eq!(s.doc.source_budget_bytes(), 10 * GIB + 64 * GIB);
    // 相対パスは断る（今の場所のまま）
    s.message.clear();
    set(
        &mut s,
        Pref::DiskCacheFolder(Some(PathBuf::from("relative/cache"))),
    );
    assert!(s.prefs.settings.disk_cache_folder.is_some());
    assert_eq!(s.message, "キャッシュの場所は絶対パスで指定します。");
    // 選択肢: 自動（今の場所での量）と GiB の並び
    let names = labels(&entries(&s, PrefChoice::DiskCacheLimit));
    assert_eq!(names[0], "自動（64 GiB）");
    assert_eq!(
        names[1..],
        ["4 GiB", "8 GiB", "16 GiB", "32 GiB", "64 GiB", "128 GiB", "256 GiB", "512 GiB"]
    );
    // 切ると、キャッシュの無い予算
    set(&mut s, Pref::DiskCache(false));
    assert_eq!(s.doc.source_budget_bytes(), 8 * GIB);
    assert_eq!(s.load_source_bytes(), 8 * GIB);
    // 場所を選ぶウィンドウ・既定に戻す
    s.apply(Action::Prefs(PrefsAction::ChooseCacheFolder));
    assert_eq!(s.dialog_request, Some(DialogRequest::PrefsCacheFolder));
    set(&mut s, Pref::DiskCacheFolder(None));
    assert_eq!(s.prefs.settings.disk_cache_folder, None);
}

/// 同じ名前の要素のうち、いちばん上にあるもの。
fn highest<'h>(h: &'h Harness<'_, YoluApp>, label: &'h str) -> egui_kittest::Node<'h> {
    h.query_all_by_label(label)
        .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .unwrap_or_else(|| panic!("「{label}」が無い"))
}

/// メモリの節のディスクキャッシュ: 入切・詳しく（上限・置き場所）の欄で選んだ値が設定のファイルと文書の予算に入る。日英の絵。
#[test]
fn the_disk_cache_rows_switch_the_cache_and_choose_the_limit_and_the_folder_in_both_languages() {
    const GIB: u64 = 1024 * MIB;
    let dir = settings_dir("disk-cache");
    let path = dir.join("YoluPainter").join("settings.conf");
    let mut h = app_with_settings(&path, vec2(1280.0, 1000.0));
    open_settings(&mut h);
    // ウィンドウに出る場所は、機械によらない場所にして撮る
    let shelf = PathBuf::from(if cfg!(windows) {
        "C:\\Library"
    } else {
        "/Library"
    });
    for pref in [
        Pref::LibraryFolder(Some(shelf)),
        Pref::DiskCacheFolder(Some(fixed_cache_folder())),
    ] {
        h.state_mut()
            .state
            .apply(Action::Prefs(PrefsAction::Set(pref)));
    }
    h.run();
    // 詳しく（メモリの節の方。処理の節の GPU のメモリにもある）
    highest(&h, "詳しく").click();
    h.run();
    assert!(h.state().state.prefs.cache_details);
    shot(&mut h, "prefs_disk_cache");
    english(&mut h, Lang::En);
    shot(&mut h, "prefs_disk_cache_english");
    english(&mut h, Lang::Ja);
    // 上限を選ぶ
    let at = h
        .get_by_label("キャッシュの上限: 自動（50 GiB）")
        .rect()
        .center();
    click(&mut h, at);
    let at = popup_item(&h, "16 GiB").center();
    click(&mut h, at);
    assert_eq!(
        h.state().state.prefs.settings.disk_cache_limit,
        yolu_app::settings::DiskLimit::Gib(16)
    );
    // 画素の予算: メモリの上限（8192 + 2048 MiB）＋ディスクの上限
    assert_eq!(
        h.state().state.doc.source_budget_bytes(),
        10 * GIB + 16 * GIB
    );
    // 置き場所を選ぶウィンドウを頼む（棚の場所の「選ぶ…」より上の方）
    highest(&h, "選ぶ…").click();
    h.run();
    assert_eq!(
        h.state().state.dialog_request,
        Some(DialogRequest::PrefsCacheFolder)
    );
    h.state_mut().state.dialog_request = None;
    // 切ると、キャッシュの無い予算
    h.get_by_label("ディスクキャッシュ").click();
    h.run();
    assert!(!h.state().state.prefs.settings.disk_cache);
    assert_eq!(h.state().state.doc.source_budget_bytes(), 8 * GIB);
    h.run();
    let written = std::fs::read_to_string(&path).unwrap();
    for line in ["disk_cache=off", "disk_cache_limit_gib=16"] {
        assert!(written.lines().any(|l| l == line), "{line}\n{written}");
    }
    assert!(written.contains("disk_cache_folder="), "{written}");
    drop(h);
    // 次の起動も同じ設定
    let h = app_with_settings(&path, vec2(1280.0, 1000.0));
    let s = &h.state().state.prefs.settings;
    assert!(!s.disk_cache);
    assert_eq!(s.disk_cache_limit, yolu_app::settings::DiskLimit::Gib(16));
    assert_eq!(s.disk_cache_folder, Some(fixed_cache_folder()));
}
