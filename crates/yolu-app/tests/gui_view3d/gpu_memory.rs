//! 設定の「GPU のメモリ」: 選んだ段・量が、3D の絵・キャンバスの GPU の合成・棚のサムネイルの予算へ配られ、次のフレームから効くこと。
//! 予算を下げると 3D のほかのセットが減り、今のセットが縮めた段になること（上げると戻る）。設定のファイルの往復・古い設定・壊れた値、
//! ウィンドウの行（数は「詳しく」の中だけ）と日英。`headless_` で始まる試験は画面を描かない。
use crate::common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::*;
use egui::{vec2, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::gpu_memory::{self, Adapter, Budgets, GpuMemory, MIB};
use yolu_app::lang::Lang;
use yolu_app::pen::PenInput;
use yolu_app::prefs::{self, Category, Pref, PrefsAction};
use yolu_app::shelf::{ShelfState, PREVIEW_BUDGET, SHELF_BUDGET};
use yolu_app::state::Action;
use yolu_app::YoluApp;
use yolu_io::shelf::Shelf;
use yolu_protocol::{
    channel, ChannelRoute, MaterialInfo, MaterialKey, MeshData, Model, Submesh, TextureProperty,
};

fn set(h: &mut Harness<'_, YoluApp>, choice: GpuMemory) {
    h.state_mut()
        .state
        .apply(Action::Prefs(PrefsAction::Set(Pref::GpuMemory(choice))));
}

/// 今のアダプターは分からないものとして（機械の GPU によらず同じ配りにする）、1 フレーム回す。
fn unknown_adapter(h: &mut Harness<'_, YoluApp>) {
    h.state_mut().state.prefs.gpu = Adapter::default();
    h.run();
}

fn applied(h: &Harness<'_, YoluApp>) -> (Budgets, Option<u64>, u64, u64) {
    let app = h.state();
    (
        app.gpu_budgets_applied(),
        app.view3d_paint_budget(),
        app.canvas_gpu_budget(),
        app.state.shelf.preview_budget,
    )
}

fn expect(choice: GpuMemory) -> Budgets {
    gpu_memory::budgets(choice, &Adapter::default())
}

#[test]
fn the_default_gives_the_three_budgets_the_old_constants() {
    let mut h = app(1100.0, 700.0, 64);
    unknown_adapter(&mut h);
    let (budgets, paint, canvas, shelf) = applied(&h);
    assert_eq!(budgets, Budgets::default());
    assert_eq!(paint, Some(yolu_app::view3d::paint::PAINT_BUDGET_BYTES));
    assert_eq!(canvas, yolu_app::canvas::gpu::RESIDENT_BUDGET);
    assert_eq!(shelf, PREVIEW_BUDGET);
    assert_eq!(h.state().state.prefs.settings.gpu_memory, GpuMemory::Auto);
}

#[test]
fn a_chosen_level_reaches_the_three_budgets_from_the_next_frame_and_auto_goes_back() {
    let mut h = app(1100.0, 700.0, 64);
    unknown_adapter(&mut h);
    for choice in [
        GpuMemory::High,
        GpuMemory::Low,
        GpuMemory::Standard,
        GpuMemory::Mib(2048),
        GpuMemory::Auto,
    ] {
        let before = applied(&h);
        set(&mut h, choice);
        // 選んだ直後（フレームの前）はまだ前の値のまま。次のフレームから効く
        assert_eq!(applied(&h), before, "{choice:?}");
        h.run();
        let want = expect(choice);
        assert_eq!(
            applied(&h),
            (want, Some(want.paint), want.canvas, want.shelf_preview),
            "{choice:?}"
        );
    }
    // 段は高 > 標準 > 低
    let total = |c| gpu_memory::total_bytes(c, &Adapter::default());
    assert!(
        total(GpuMemory::High) > total(GpuMemory::Standard)
            && total(GpuMemory::Standard) > total(GpuMemory::Low)
    );
}

#[test]
fn a_custom_total_is_split_four_four_one_and_kept_inside_the_range() {
    let mut h = app(1100.0, 700.0, 64);
    unknown_adapter(&mut h);
    set(&mut h, GpuMemory::Mib(1800));
    h.run();
    let b = h.state().gpu_budgets_applied();
    assert_eq!(
        (b.paint, b.canvas, b.shelf_preview),
        (800 * MIB, 800 * MIB, 200 * MIB)
    );
    // 範囲の外は範囲へ収める
    set(&mut h, GpuMemory::Mib(1));
    assert_eq!(
        h.state().state.prefs.settings.gpu_memory,
        GpuMemory::Mib(gpu_memory::MIN_TOTAL_MIB)
    );
    set(&mut h, GpuMemory::Mib(u32::MAX));
    assert_eq!(
        h.state().state.prefs.settings.gpu_memory,
        GpuMemory::Mib(gpu_memory::MAX_TOTAL_MIB)
    );
}

#[test]
fn the_adapter_amount_changes_the_budgets_when_it_becomes_known() {
    let mut h = app(1100.0, 700.0, 64);
    unknown_adapter(&mut h);
    let before = h.state().gpu_budgets_applied();
    // 24 GiB の GPU: 標準は量の 1/8（3 GiB）
    h.state_mut().state.prefs.gpu = Adapter {
        memory_mib: Some(24 * 1024),
    };
    h.run();
    let after = h.state().gpu_budgets_applied();
    assert!(
        after.paint > before.paint
            && after.canvas > before.canvas
            && after.shelf_preview > before.shelf_preview
    );
    assert_eq!(
        after,
        gpu_memory::budgets(
            GpuMemory::Auto,
            &Adapter {
                memory_mib: Some(24 * 1024)
            }
        )
    );
}

#[test]
fn a_budget_a_test_set_by_hand_stays_until_the_setting_changes() {
    let mut h = app(1100.0, 700.0, 64);
    unknown_adapter(&mut h);
    h.state_mut().view3d_set_paint_budget(700_000);
    h.state_mut().set_canvas_gpu_budget(5_000_000);
    h.run();
    h.run();
    assert_eq!(
        h.state().view3d_paint_budget(),
        Some(700_000),
        "設定が変わらないうちは上書きしない"
    );
    assert_eq!(h.state().canvas_gpu_budget(), 5_000_000);
    set(&mut h, GpuMemory::Low);
    h.run();
    assert_eq!(
        h.state().view3d_paint_budget(),
        Some(expect(GpuMemory::Low).paint)
    );
}

// ───────── 3D のほかのセットと今のセット ─────────

fn model(n: usize, size: u32) -> Model {
    let materials = (0..n)
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
        .collect();
    let meshes = (0..n)
        .map(|i| {
            let x = (i as f32 - (n as f32 - 1.0) / 2.0) * 1.3;
            MeshData {
                key: format!("{i}"),
                name: format!("板{i}"),
                skinned: false,
                positions: vec![
                    [x - 0.5, -0.5, 0.0],
                    [x + 0.5, -0.5, 0.0],
                    [x - 0.5, 0.5, 0.0],
                    [x + 0.5, 0.5, 0.0],
                ],
                normals: vec![],
                uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
                submeshes: vec![Submesh {
                    material: i as u32,
                    indices: vec![0, 2, 1, 2, 3, 1],
                }],
            }
        })
        .collect();
    Model {
        generation: 1,
        name: "板".into(),
        materials,
        meshes,
    }
}

/// n 枚の板（セット i = マテリアル i、文書は size × size）を 3D ビューに出す。どのセットにも Color の全面の塗りつぶしを足す。
fn scene(n: usize, size: u32) -> Harness<'static, YoluApp> {
    let mut h = app(1100.0, 700.0, size);
    unknown_adapter(&mut h);
    click_tab(&mut h, yolu_app::Tab::View3d);
    move_to(&h, egui::pos2(1.0, 1.0));
    h.run();
    h.state_mut().load_live_link_model(&model(n, size)).unwrap();
    h.run();
    for set in 0..n {
        h.state_mut()
            .state
            .set_doc_mut(set)
            .add_fill_layer(
                "色",
                &[(
                    yolu_core::Channel::Color,
                    yolu_core::Rgba8::new(200, 60, 40, 255),
                )],
                None,
            )
            .unwrap();
    }
    h.run();
    assert_eq!(h.state().state.sets.len(), n);
    h
}

#[test]
fn lowering_the_memory_leaves_out_the_farthest_sets_and_raising_it_brings_them_back() {
    let mut h = scene(3, 256);
    // 標準: 256² の Color だけ（ミップ込み 349,524 B）を、今のセットとほかの 2 枚が全部入る
    let s = h.state().view3d_stats().unwrap();
    assert_eq!((s.other_sets, s.other_skipped), (2, 0), "{s:?}");
    // 設定のウィンドウが選べる下限（256 MiB）より小さい合計は、予算の道を通すために設定へ直に入れる（3D の予算 = 合計の 4/9）
    // 合計 2 MiB の 3D = 932,067 B: 今のセット + ほかの 1 枚（残り 582,543 B で 349,524 B を 1 枚）
    h.state_mut().state.prefs.settings.gpu_memory = GpuMemory::Mib(2);
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!((s.other_sets, s.other_skipped), (1, 1), "{s:?}");
    assert_eq!(
        h.state().view3d_held_materials(),
        vec![1],
        "今のセット 0 から近い順"
    );
    assert_eq!(h.state().state.view3d.unpainted, vec![2]);
    // 合計 1 MiB の 3D = 466,033 B: 今のセットだけ（今のセットが先）
    h.state_mut().state.prefs.settings.gpu_memory = GpuMemory::Mib(1);
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!((s.other_sets, s.other_skipped), (0, 2), "{s:?}");
    assert_eq!(s.paint_level, 0, "今のセットは満量のまま");
    // 戻す: 全部持つ
    set(&mut h, GpuMemory::Auto);
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!((s.other_sets, s.other_skipped), (2, 0), "{s:?}");
    assert!(h.state().state.view3d.unpainted.is_empty());
}

#[test]
fn lowering_the_memory_shrinks_the_current_sets_picture_and_raising_it_restores_the_size() {
    let mut h = scene(1, 512);
    let s = h.state().view3d_stats().unwrap();
    assert_eq!((s.paint_level, s.other_sets), (0, 0), "{s:?}");
    // 512² の Color だけ = 1,398,101 B。合計 2 MiB の 3D（932,067 B）には入らないので、256² に縮める
    h.state_mut().state.prefs.settings.gpu_memory = GpuMemory::Mib(2);
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!(s.paint_level, 1, "{s:?}");
    assert!(s.paint_by_budget, "縮めは予算で決まった");
    // 上げると、元の大きさへ戻る（文書を替えなくてよい）
    set(&mut h, GpuMemory::Auto);
    h.run();
    let s = h.state().view3d_stats().unwrap();
    assert_eq!((s.paint_level, s.paint_by_budget), (0, false), "{s:?}");
}

// ───────── 棚 ─────────

fn fixture_shelf() -> Shelf {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../yolu-io/tests/fixtures/smart/all");
    let mut files: BTreeMap<String, Arc<[u8]>> = BTreeMap::from([(
        "resources.json".into(),
        Arc::from(std::fs::read(root.join("resources.json")).unwrap()),
    )]);
    for entry in std::fs::read_dir(root.join("resources")).unwrap() {
        let entry = entry.unwrap();
        files.insert(
            format!("resources/{}", entry.file_name().to_string_lossy()),
            Arc::from(std::fs::read(entry.path()).unwrap()),
        );
    }
    Shelf::read(&files, SHELF_BUDGET).unwrap()
}

#[test]
fn headless_raising_the_preview_budget_makes_the_thumbnails_the_icons_could_not_have() {
    let ctx = egui::Context::default();
    let mut shelf = ShelfState::with_shelf(fixture_shelf());
    shelf.set_preview_budget(1);
    shelf.inspect_pending(100);
    let smart: Vec<String> = shelf
        .resources()
        .iter()
        .filter(|r| r.kind == "smartMaterial" || r.kind == "smartMask")
        .map(|r| r.id.clone())
        .collect();
    assert!(!smart.is_empty(), "試験の素材が見つからない");
    for id in &smart {
        assert!(
            shelf.texture(&ctx, id).is_none(),
            "予算を超える素材はアイコン: {id}"
        );
    }
    // 上げると、見直してサムネイルを作る
    shelf.set_preview_budget(PREVIEW_BUDGET);
    shelf.inspect_pending(100);
    for id in &smart {
        assert!(
            shelf.texture(&ctx, id).is_some(),
            "上げたあとはサムネイル: {id}"
        );
    }
    // 下げても、作ってあるサムネイルは残す
    shelf.set_preview_budget(1);
    for id in &smart {
        assert!(shelf.texture(&ctx, id).is_some(), "下げても残る: {id}");
    }
    assert_eq!(shelf.preview_budget, 1);
}

#[test]
fn headless_raising_the_preview_budget_reviews_only_what_the_budget_left_out() {
    let mut shelf = ShelfState::with_shelf(fixture_shelf());
    // 画像は大きすぎる（予算とは別の理由）ので、サムネイルを作らない
    shelf.image_thumb_pixels = 1;
    shelf.set_preview_budget(1);
    shelf.inspect_pending(100);
    let ids = |kinds: &[&str]| -> Vec<String> {
        shelf
            .resources()
            .iter()
            .filter(|r| kinds.contains(&r.kind.as_str()))
            .map(|r| r.id.clone())
            .collect()
    };
    let images = ids(&["image"]);
    let smart = ids(&["smartMaterial", "smartMask"]);
    assert!(
        !images.is_empty() && !smart.is_empty(),
        "試験の素材が見つからない"
    );
    for id in images.iter().chain(&smart) {
        assert!(shelf.info(id).is_some(), "見終えている: {id}");
    }
    shelf.set_preview_budget(PREVIEW_BUDGET);
    // 量が足りずにアイコンだった素材だけを見直す（取り除いて、次の inspect が作る）。ほかの理由でサムネイルが無い画像は、読み直さない
    for id in &images {
        assert!(
            shelf.info(id).is_some(),
            "予算と関係なくサムネイルが無い画像は残す: {id}"
        );
    }
    for id in &smart {
        assert!(
            shelf.info(id).is_none(),
            "量が足りなかった素材は見直す: {id}"
        );
    }
    shelf.inspect_pending(100);
    for id in &smart {
        assert!(shelf.info(id).is_some(), "{id}");
    }
}

// ───────── ウィンドウと設定のファイル ─────────

fn settings_dir(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/gpu-memory-tests")
        .join(std::process::id().to_string())
        .join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

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
    // 中央は 1 つの組（`common::app` と同じ並び）
    h.state_mut().dock = common::tabbed_center_dock(size.x);
    h.run();
    h
}

fn window_rect(h: &Harness<'_, YoluApp>) -> Rect {
    prefs::last_rect(&h.ctx).expect("設定のウィンドウが開いている")
}

/// 設定のウィンドウを「表示」の区分（GPU のメモリがある所）で開く。
fn open_settings(h: &mut Harness<'static, YoluApp>) {
    h.state_mut()
        .state
        .apply(Action::Prefs(PrefsAction::OpenAt(Category::Display)));
    h.run();
}

/// ウィンドウの中だけを撮って、正解の絵と比べる。
fn shot(h: &mut Harness<'_, YoluApp>, name: &str) {
    let rect = window_rect(h);
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

fn pick(h: &mut Harness<'static, YoluApp>, label: &str, item: &str) {
    let at = h.get_by_label(label).rect().center();
    click(h, at);
    let at = popup_item(h, item).center();
    click(h, at);
}

#[test]
fn the_window_row_names_the_level_without_numbers_and_the_details_show_the_total() {
    let dir = settings_dir("window");
    let path = dir.join("YoluPainter").join("settings.conf");
    let mut h = app_with_settings(&path, vec2(1280.0, 800.0));
    h.state_mut().state.prefs.gpu = Adapter::default();
    open_settings(&mut h);
    h.run();
    let _ = h.get_by_label("GPU のメモリ: 自動");
    let _ = gpu_details(&h, "詳しく");
    // 段を選ぶ（ウィンドウの行に数は出ない）
    pick(&mut h, "GPU のメモリ: 自動", "高");
    h.run();
    let _ = h.get_by_label("GPU のメモリ: 高");
    assert_eq!(h.state().state.prefs.settings.gpu_memory, GpuMemory::High);
    assert_eq!(h.state().gpu_budgets_applied(), expect(GpuMemory::High));
    // 詳しく: 合計のスライダーが出る（ウィンドウの中）
    gpu_details(&h, "詳しく").click();
    h.run();
    assert!(h.state().state.prefs.gpu_details);
    let open = window_rect(&h);
    assert!(
        open.contains_rect(
            h.get_by_role_and_label(egui::accesskit::Role::Slider, "合計")
                .rect()
        ),
        "{open:?}"
    );
    shot(&mut h, "prefs_gpu_details");
    // スライダーで量を指定すると、段の選びは「指定」になる（数は行に出さない）
    set(&mut h, GpuMemory::Mib(3000));
    h.run();
    let _ = h.get_by_label("GPU のメモリ: 指定");
    assert_eq!(
        h.state().gpu_budgets_applied(),
        expect(GpuMemory::Mib(3000))
    );
    // 選び直すと段へ戻る
    pick(&mut h, "GPU のメモリ: 指定", "標準");
    h.run();
    assert_eq!(
        h.state().state.prefs.settings.gpu_memory,
        GpuMemory::Standard
    );
    // 英語（言語は「一般」の区分）
    h.state_mut()
        .state
        .apply(Action::Prefs(PrefsAction::Choose(Category::General)));
    h.run();
    pick(&mut h, "言語: 日本語", "English");
    h.state_mut()
        .state
        .apply(Action::Prefs(PrefsAction::Choose(Category::Display)));
    h.run();
    let _ = h.get_by_label("GPU memory: Standard");
    let _ = gpu_details(&h, "Details");
    assert!(window_rect(&h).contains_rect(
        h.get_by_role_and_label(egui::accesskit::Role::Slider, "Total")
            .rect()
    ));
    shot(&mut h, "prefs_gpu_details_english");
    // 閉じると、次に開いたときは閉じた形
    gpu_details(&h, "Details").click();
    h.run();
    assert!(!h.state().state.prefs.gpu_details);
    assert!(h
        .query_by_role_and_label(egui::accesskit::Role::Slider, "Total")
        .is_none());
    // どの段・指定の名前にも数が無い
    for lang in Lang::ALL {
        for choice in GpuMemory::LEVELS.into_iter().chain([GpuMemory::Mib(1234)]) {
            assert!(
                !choice.name(lang).chars().any(|c| c.is_ascii_digit()),
                "{lang:?} {choice:?}"
            );
        }
    }
}

#[test]
fn the_choice_is_written_to_the_file_and_the_next_start_applies_it() {
    let dir = settings_dir("file");
    let path = dir.join("YoluPainter").join("settings.conf");
    let mut h = app_with_settings(&path, vec2(1280.0, 800.0));
    open_settings(&mut h);
    pick(&mut h, "GPU のメモリ: 自動", "低");
    h.run();
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.lines().any(|l| l == "gpu_memory=low"), "{written}");
    set(&mut h, GpuMemory::Mib(2048));
    h.run();
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .any(|l| l == "gpu_memory=2048"));
    drop(h);
    // 次の起動は、書いた設定で始まり、3 つの予算にも入っている
    let h = app_with_settings(&path, vec2(1280.0, 800.0));
    assert_eq!(
        h.state().state.prefs.settings.gpu_memory,
        GpuMemory::Mib(2048)
    );
    let adapter = h.state().state.prefs.gpu.clone();
    let want = gpu_memory::budgets(GpuMemory::Mib(2048), &adapter);
    assert_eq!(h.state().gpu_budgets_applied(), want);
    assert_eq!(h.state().view3d_paint_budget(), Some(want.paint));
    assert_eq!(h.state().canvas_gpu_budget(), want.canvas);
    assert_eq!(h.state().state.shelf.preview_budget, want.shelf_preview);
    assert_eq!(h.state().state.message, "");
}

#[test]
fn a_settings_file_without_the_key_starts_automatic_and_a_broken_value_says_why_and_is_repaired() {
    // 古い設定（キーが無い）: 自動で、理由は出ない
    let dir = settings_dir("old");
    let path = dir.join("YoluPainter").join("settings.conf");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "language=ja\nbackups=4\n").unwrap();
    let h = app_with_settings(&path, vec2(1280.0, 800.0));
    assert_eq!(h.state().state.prefs.settings.gpu_memory, GpuMemory::Auto);
    assert_eq!(h.state().state.message, "");
    drop(h);
    // 壊れた値: その項目だけ自動へ戻し、理由を状態の帯に出す。ほかの項目は生かす
    let dir = settings_dir("broken");
    let path = dir.join("YoluPainter").join("settings.conf");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "language=ja\ngpu_memory=たくさん\nbackups=4\n").unwrap();
    let mut h = app_with_settings(&path, vec2(1280.0, 800.0));
    assert_eq!(h.state().state.prefs.settings.gpu_memory, GpuMemory::Auto);
    assert_eq!(
        h.state().state.prefs.settings.backups,
        yolu_io::BackupKeep::Count(4)
    );
    let message = h.state().state.message.clone();
    assert!(
        message.contains("GPU のメモリの設定が正しくありません"),
        "{message}"
    );
    // ファイルは、設定を選び直すまで触らない。選び直すと壊れた行が消える
    assert!(std::fs::read_to_string(&path).unwrap().contains("たくさん"));
    open_settings(&mut h);
    pick(&mut h, "GPU のメモリ: 自動", "標準");
    h.run();
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.lines().any(|l| l == "gpu_memory=standard") && !written.contains("たくさん"),
        "{written}"
    );
}

// ───────── 合計のスライダーをドラッグする ─────────

/// GPU のメモリの「詳しく」（処理の節。メモリの節のディスクキャッシュにも同じ名前があるので、下の方）。
fn gpu_details<'h>(h: &'h Harness<'_, YoluApp>, label: &'h str) -> egui_kittest::Node<'h> {
    h.query_all_by_label(label)
        .max_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .unwrap_or_else(|| panic!("「{label}」が無い"))
}

/// 「詳しく」を開いて、合計のスライダーの溝の上の点（左端 0 から右端 1 の割合）を返す関数を作る。
fn total_slider(h: &mut Harness<'static, YoluApp>, lang_total: &str) -> impl Fn(f32) -> egui::Pos2 {
    gpu_details(h, "詳しく").click();
    h.run();
    let rect = h
        .get_by_role_and_label(egui::accesskit::Role::Slider, lang_total)
        .rect();
    let y = rect.bottom() - 4.0;
    move |fraction| egui::pos2(rect.left() + rect.width() * fraction, y)
}

/// ウィンドウを開いて合計のスライダーを押し続けているあいだ、予算は入らない。離すと、指定した量が 1 度に入る。
#[test]
fn the_budgets_wait_while_the_total_slider_is_held_and_apply_when_it_is_let_go() {
    let dir = settings_dir("hold");
    let path = dir.join("YoluPainter").join("settings.conf");
    let mut h = app_with_settings(&path, vec2(1280.0, 800.0));
    unknown_adapter(&mut h);
    open_settings(&mut h);
    let at = total_slider(&mut h, "合計");
    let primary = egui::PointerButton::Primary;
    press(&h, at(0.1), primary);
    h.step();
    // 押しているあいだは、フレームを重ねても入らない（ウィンドウが開いているので、押している印が保たれる）
    for fraction in [0.1, 0.2, 0.3, 0.2] {
        move_to(&h, at(fraction));
        h.step();
        // 何フレーム回しても入らない（`run` は静まるまで複数フレーム回す）
        h.run();
        assert!(h.state().state.prefs.dragging);
        assert!(matches!(
            h.state().state.prefs.settings.gpu_memory,
            GpuMemory::Mib(_)
        ));
        assert_eq!(
            h.state().gpu_budgets_applied(),
            Budgets::default(),
            "押しているあいだは入れない（キャンバスの合成が毎フレーム資源を手放す）"
        );
    }
    let GpuMemory::Mib(mib) = h.state().state.prefs.settings.gpu_memory else {
        unreachable!()
    };
    release(&h, at(0.2), primary);
    h.step();
    h.run();
    assert!(!h.state().state.prefs.dragging);
    assert_eq!(
        h.state().state.prefs.settings.gpu_memory,
        GpuMemory::Mib(mib)
    );
    assert_eq!(h.state().gpu_budgets_applied(), expect(GpuMemory::Mib(mib)));
    // 離したときの量を、設定のファイルへ 1 回書く
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .any(|l| l == format!("gpu_memory={mib}")));
}

/// Esc で合計のスライダーのドラッグを止めると、押す前の選び（自動・段・指定）に戻る（押し始めの量で置き換えない）。何も書かず、予算も入れない。
#[test]
fn escape_while_dragging_the_total_slider_keeps_the_choice_that_was_there() {
    for (start, line) in [
        (GpuMemory::Auto, None),
        (GpuMemory::High, Some("gpu_memory=high")),
        (GpuMemory::Mib(3000), Some("gpu_memory=3000")),
    ] {
        let dir = settings_dir(&format!("escape-{start:?}").replace(['(', ')'], "-"));
        let path = dir.join("YoluPainter").join("settings.conf");
        let mut h = app_with_settings(&path, vec2(1280.0, 800.0));
        unknown_adapter(&mut h);
        open_settings(&mut h);
        if start != GpuMemory::Auto {
            set(&mut h, start);
            h.run();
        }
        let written = std::fs::read_to_string(&path).ok();
        assert_eq!(
            written
                .as_deref()
                .and_then(|w| w.lines().find(|l| l.starts_with("gpu_memory="))),
            line,
            "{start:?}"
        );
        let applied_before = h.state().gpu_budgets_applied();
        assert_eq!(applied_before, expect(start));
        let at = total_slider(&mut h, "合計");
        let primary = egui::PointerButton::Primary;
        // 押して動かすと、指定の量になる
        press(&h, at(0.1), primary);
        h.step();
        move_to(&h, at(0.6));
        h.step();
        assert!(
            matches!(h.state().state.prefs.settings.gpu_memory, GpuMemory::Mib(_)),
            "{start:?}"
        );
        assert!(h.state().state.prefs.dragging);
        // Esc: 押す前の選びへ戻り、押している印は消える
        key(&h, egui::Key::Escape, egui::Modifiers::NONE);
        h.step();
        assert_eq!(
            h.state().state.prefs.settings.gpu_memory,
            start,
            "押す前の選びに戻る"
        );
        assert!(!h.state().state.prefs.dragging);
        // 続けて動かしても、離しても、選びは変わらない。何も書かず、予算も入れ直さない
        move_to(&h, at(0.9));
        h.step();
        release(&h, at(0.9), primary);
        h.step();
        h.run();
        assert_eq!(
            h.state().state.prefs.settings.gpu_memory,
            start,
            "{start:?}"
        );
        assert_eq!(h.state().gpu_budgets_applied(), applied_before, "{start:?}");
        assert_eq!(
            std::fs::read_to_string(&path).ok(),
            written,
            "{start:?}: 設定のファイルは変わらない"
        );
        let row = format!("GPU のメモリ: {}", start.name(Lang::Ja));
        let _ = h.get_by_label(&row);
    }
}
