//! メッシュマップのベイク・テンプレートの書き出し・PSD の読み書きの画面（egui_kittest）: 窓を開く → 選ぶ → 結果のファイル・文書が変わる、
//! 取消、上書きの確かめ、断る理由、日本語と英語。`headless_` で始まる試験は画面を描かず、Wine でも回る。
mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use common::*;
use egui::{pos2, vec2, Key};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::bake::window::Page;
use yolu_app::bake::{BakeAction, BakeAdapter, BakeBackend, BakeRun, MeshMapView};
use yolu_app::engine::Channel;
use yolu_app::export::ExportAction;
use yolu_app::lang::Lang;
use yolu_app::psd::{PsdAction, PsdTarget};
use yolu_app::state::{blank_document, Action, AppState, DialogRequest};
use yolu_app::YoluApp;
use yolu_core::mesh_maps::{MeshMapKind, MeshMapState};
use yolu_core::{LayerId, TileCoord};
use yolu_protocol::{MaterialInfo, MaterialKey, MeshData, Model, Submesh};

/// 試験ごとの一時フォルダ（終わったら消す）。
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "yolu-app-outputs-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
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

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 窓の中だけを撮って、正解の絵と比べる（ほかのパネルの変更で壊れない）。
fn shot(h: &mut Harness<'_, YoluApp>, window: &str, name: &str) {
    let rect = yolu_app::windows::window_rect(&h.ctx, window)
        .unwrap_or_else(|| panic!("{window} を描いていない"));
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

/// 別のスレッドの仕事（ベイク・書き出し・PSD）が終わるまで、フレームを回しながら待つ。
fn settle(h: &mut Harness<'_, YoluApp>) {
    let start = Instant::now();
    loop {
        h.step();
        let s = &h.state().state;
        if !(s.bake.is_baking()
            || s.bake.is_checking()
            || s.export.is_exporting()
            || s.psd.is_busy())
        {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "仕事が終わらない（ハング検出上限）"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    h.run();
}

fn apply(h: &mut Harness<'_, YoluApp>, action: Action) {
    h.state_mut().state.apply(action);
    h.run();
}

/// 試しの立方体を読んだ画面。ベイクが速く済むよう設定を小さくする。
fn cube_app(size: u32) -> Harness<'static, YoluApp> {
    let mut h = app(1280.0, 800.0, size);
    apply(&mut h, Action::LoadDemoModel);
    let s = &mut h.state_mut().state;
    s.bake.settings.ao_samples = 8;
    s.bake.settings.thickness_samples = 8;
    s.bake.settings.padding = 4;
    h.run();
    h
}

/// メニューから「メッシュマップをベイク…」を開く。
fn open_bake_window(h: &mut Harness<'_, YoluApp>) {
    let at = menu_title(h, "表示").center();
    click(h, at);
    let at = popup_item(h, "メッシュマップをベイク…").center();
    click(h, at);
    assert!(h.state().state.bake.window.is_some());
}

/// 名前のチェック（同じ名前のボタンと区別するため、役割で探す）。
fn check<'a>(h: &'a Harness<'_, YoluApp>, label: &'a str) -> egui_kittest::Node<'a> {
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, label)
}

/// チェックが入っているか（読み上げの木の toggled）。
fn checked(h: &Harness<'_, YoluApp>, label: &str) -> bool {
    format!("{:?}", check(h, label).accesskit_node().toggled()) == "Some(True)"
}

fn kinds(h: &Harness<'_, YoluApp>) -> Vec<MeshMapKind> {
    h.state()
        .state
        .sets
        .current()
        .mesh_maps
        .iter()
        .map(|m| m.kind())
        .collect()
}

#[test]
fn the_menus_hold_import_export_and_bake_and_only_ask_for_files() {
    let mut h = app(1280.0, 800.0, 64);
    let at = menu_title(&h, "ファイル").center();
    click(&mut h, at);
    // 読み込み
    let at = popup_item(&h, "PSD を今のセットの文書へ…").center();
    click(&mut h, at);
    assert_eq!(
        h.state().state.dialog_request,
        Some(DialogRequest::PsdImport(PsdTarget::CurrentSet))
    );
    h.state_mut().state.dialog_request = None;
    // 書き出し（3 つのテンプレートと PSD）
    for (label, expect) in [
        (
            "テンプレート: Unity Standard / URP Lit…",
            DialogRequest::ExportFolder("unity-standard".into()),
        ),
        (
            "テンプレート: HDRP Lit…",
            DialogRequest::ExportFolder("unity-hdrp".into()),
        ),
        (
            "テンプレート: lilToon…",
            DialogRequest::ExportFolder("liltoon".into()),
        ),
    ] {
        let at = menu_title(&h, "ファイル").center();
        click(&mut h, at);
        let at = popup_item(&h, label).center();
        click(&mut h, at);
        assert_eq!(h.state().state.dialog_request, Some(expect), "{label}");
        h.state_mut().state.dialog_request = None;
    }
    // PSD の書き出しは、先に設定の窓（方式・チャンネル）を開き、ファイルはその窓の「書き出し…」で選ぶ
    let at = menu_title(&h, "ファイル").center();
    click(&mut h, at);
    let at = popup_item(&h, "PSD…").center();
    click(&mut h, at);
    assert!(h.state().state.psd.options_open);
    assert_eq!(h.state().state.dialog_request, None);
    apply(&mut h, Action::Psd(PsdAction::CancelExportOptions));
    // 英語
    h.state_mut().state.lang = Lang::En;
    h.run();
    let at = menu_title(&h, "File").center();
    click(&mut h, at);
    popup_item(&h, "PSD as a New Texture Set…");
    popup_item(&h, "Template: HDRP Lit…");
    let at = popup_item(&h, "PSD…").center();
    click(&mut h, at);
    assert!(h.state().state.psd.options_open);
    apply(&mut h, Action::Psd(PsdAction::CancelExportOptions));
    // 描いている間は選べない
    h.state_mut().state.dialog_request = None;
    let layer = h.state().state.selected_layer.unwrap();
    let brush = h.state().state.stroke_settings(false);
    let stroke = h.state_mut().state.doc.begin_stroke(layer, &brush).unwrap();
    h.run();
    let at = menu_title(&h, "File").center();
    click(&mut h, at);
    assert!(
        h.state().state.popup.is_none(),
        "描いている間はメニューを開かない"
    );
    h.state_mut().state.doc.end_stroke(stroke).unwrap();
}

#[test]
fn the_bake_window_lists_sets_and_maps_and_matches_its_snapshot() {
    let mut h = cube_app(64);
    open_bake_window(&mut h);
    shot(&mut h, "bake", "bake_window");
    // 焼くマップのチェック（既定は先頭の 5 つ）
    for name in ["ワールド法線", "位置", "AO", "曲率", "厚み"] {
        assert!(checked(&h, name), "{name}");
    }
    assert!(!checked(&h, "接空間法線"));
    // 押すと反転して、設定の値が変わる
    check(&h, "曲率").click();
    h.run();
    assert!(!h
        .state()
        .state
        .bake
        .settings
        .maps
        .contains(&MeshMapKind::Curvature));
    check(&h, "ハイト").click();
    h.run();
    assert!(h
        .state()
        .state
        .bake
        .settings
        .maps
        .contains(&MeshMapKind::Height));
    // 焼くセットの欄と、押せるボタン
    assert!(checked(&h, "テクスチャセット 1"));
    h.get_by_label("チェックしたマップをベイク");
    // 閉じる
    h.get_by_label("閉じる").click();
    h.run();
    assert!(h.state().state.bake.window.is_none());
}

#[test]
fn baking_from_the_window_fills_the_set_and_the_canvas_shows_the_overlay() {
    let mut h = cube_app(64);
    open_bake_window(&mut h);
    h.get_by_label("チェックしたマップをベイク").click();
    h.run();
    assert!(h.state().state.bake.is_baking(), "別のスレッドで焼いている");
    settle(&mut h);
    let s = &h.state().state;
    assert_eq!(
        kinds(&h),
        [
            MeshMapKind::WorldNormal,
            MeshMapKind::Position,
            MeshMapKind::AmbientOcclusion,
            MeshMapKind::Curvature,
            MeshMapKind::Thickness
        ]
    );
    assert!(s.message.contains("焼きました"), "{}", s.message);
    assert!(s.modified);
    assert_eq!(s.bake.view, MeshMapView::Coverage);
    // キャンバスに重ねて見える（頁のテクスチャを作った）。右上の隅に名前のアイコン（名前はツールチップ）
    assert_eq!(s.bake.overlay.page_count(), 1);
    h.get_by_label("メッシュマップ: UV の範囲");
    // 撮る絵は毎回同じに（かかった時間と書き出し先は毎回違う）
    {
        let s = &mut h.state_mut().state;
        s.message = "試験".into();
        s.bake.outcome = Some(("テクスチャセット 1: 試験".into(), true));
        s.bake.window.as_mut().unwrap().page = Page::Map(MeshMapKind::AmbientOcclusion);
    }
    h.run();
    shot(&mut h, "bake", "bake_window_baked");
    // 窓の目で別のマップを見る
    h.get_by_label("アンビエントオクルージョンをキャンバスに出す")
        .click();
    h.run();
    assert_eq!(
        h.state().state.bake.view,
        MeshMapView::Kind(MeshMapKind::AmbientOcclusion)
    );
    h.get_by_label("メッシュマップ: アンビエントオクルージョン");
    // 隅のアイコンを押すとやめる
    h.get_by_label("メッシュマップ: アンビエントオクルージョン")
        .click();
    h.run();
    assert_eq!(h.state().state.bake.view, MeshMapView::None);
    assert_eq!(h.state().state.bake.overlay.page_count(), 0);
    // 焼いたのでマップの状態は最新
    for kind in kinds(&h) {
        let check = h.state_mut().state.mesh_map_check(0, kind).unwrap();
        assert_eq!(check.state, MeshMapState::Current, "{kind:?}");
    }
}

#[test]
fn the_bake_window_checks_a_replaced_model_in_another_thread_and_keeps_working() {
    let mut h = cube_app(64);
    open_bake_window(&mut h);
    settle(&mut h);
    h.get_by_label("チェックしたマップをベイク").click();
    h.run();
    settle(&mut h);
    assert_eq!(kinds(&h).len(), 5);
    assert!(!h.state().state.bake.is_checking());
    // モデルが替わった次の 1 フレームで、窓は入力を別のスレッドで作り始める（このフレームでは作らず、窓は描かれる）
    h.state_mut().state.apply(Action::LoadDemoModel);
    h.step();
    assert!(h.state().state.bake.is_checking());
    h.get_by_label("チェックしたマップをベイク");
    settle(&mut h);
    assert!(!h.state().state.bake.is_checking());
    // 同じ形の立方体なので、焼いたマップは今も最新（作り直した入力で照合した）
    for kind in kinds(&h) {
        let check = h.state_mut().state.mesh_map_check(0, kind).unwrap();
        assert_eq!(check.state, MeshMapState::Current, "{kind:?}");
    }
    // 窓を閉じると、作っている最中のものは手放す
    h.state_mut().state.apply(Action::LoadDemoModel);
    h.step();
    assert!(h.state().state.bake.is_checking());
    apply(&mut h, Action::Bake(BakeAction::CloseWindow));
    h.run();
    assert!(!h.state().state.bake.is_checking());
}

/// 画面の描画の画素（キャンバスのこの文書の画素の真ん中）。
fn screen_pixel(h: &mut Harness<'_, YoluApp>, x: u32, y: u32) -> [u8; 4] {
    let rect = canvas_rect(h);
    let doc = &h.state().state.doc;
    let view = h.state().state.view.view(rect, doc.width(), doc.height());
    let at = view.to_screen(x as f64 + 0.5, y as f64 + 0.5);
    h.event(egui::Event::PointerGone);
    h.step();
    let image = h.render().expect("描画");
    image.get_pixel(at.x.round() as u32, at.y.round() as u32).0
}

#[test]
fn the_overlay_paints_the_texel_origin_colors_where_the_texels_are() {
    let mut h = cube_app(64);
    apply(&mut h, Action::Bake(BakeAction::Start));
    settle(&mut h);
    assert_eq!(h.state().state.bake.view, MeshMapView::Coverage);
    let map = h
        .state()
        .state
        .sets
        .current()
        .mesh_maps
        .get(MeshMapKind::Position)
        .unwrap()
        .clone();
    let find = |want: u8| {
        let i = map
            .coverage()
            .iter()
            .position(|c| *c == want)
            .expect("その由来のテクセル");
        ((i % 64) as u32, (i / 64) as u32)
    };
    let near = |a: [u8; 4], b: [u8; 3]| {
        (0..3).all(|i| (a[i] as i32 - b[i] as i32).abs() <= 3) && a[3] == 255
    };
    // 緑 = 覆う、青 = 余白（UV の中は文書の絵が透明でも、重ねた色が見える）
    let (x, y) = find(1);
    assert!(
        near(screen_pixel(&mut h, x, y), [40, 170, 70]),
        "{:?}",
        screen_pixel(&mut h, x, y)
    );
    let (x, y) = find(3);
    assert!(near(screen_pixel(&mut h, x, y), [60, 100, 220]));
    // マップを替えると、そのマップの値（位置は 3 チャンネル。覆うテクセルが緑一色ではなくなる）
    apply(
        &mut h,
        Action::Bake(BakeAction::View(MeshMapView::Kind(MeshMapKind::Position))),
    );
    let (x, y) = find(1);
    let expected = &map.to_rgba8(false)[((y * 64 + x) * 4) as usize..][..4];
    assert_eq!(
        &screen_pixel(&mut h, x, y)[..3],
        &expected[..3],
        "位置のマップの値"
    );
    // やめると、重ね表示は消える（UV の外は文書の透明のまま、重ねた色は無い）
    apply(&mut h, Action::Bake(BakeAction::View(MeshMapView::None)));
    let (x, y) = find(1);
    let p = screen_pixel(&mut h, x, y);
    assert!(
        !near(p, [40, 170, 70]) && !(p[..3] == expected[..3]),
        "{p:?}"
    );
}

#[test]
fn cancel_from_the_window_and_the_job_card_after_closing_it() {
    let mut h = cube_app(64);
    h.state_mut().state.bake.settings.maps = vec![MeshMapKind::AmbientOcclusion];
    open_bake_window(&mut h);
    // 取消が来るまで始めない仕事にする（取消が効いたことを、焼き終わる速さに頼らず確かめる）
    h.state_mut().state.bake.park_next = true;
    h.get_by_label("チェックしたマップをベイク").click();
    h.run();
    assert!(h.state().state.bake.is_baking());
    // 窓の取消（1 フレームだけ進める: 取消の旗は立つが、止まった仕事を受けるのは次のフレーム）
    h.get_by_label("取消").click();
    h.step();
    assert!(h.state().state.bake.progress().unwrap().canceling);
    settle(&mut h);
    assert!(h.state().state.sets.current().mesh_maps.is_empty());
    assert!(h.state().state.message.contains("取り消しました"));

    // 窓を閉じても焼き続け、仕事の札から取り消せる
    h.state_mut().state.bake.park_next = true;
    h.get_by_label("チェックしたマップをベイク").click();
    h.run();
    assert!(h.state().state.bake.is_baking());
    h.get_by_label("閉じる").click();
    h.run();
    assert!(h.state().state.bake.window.is_none());
    assert!(h.state().state.bake.is_baking(), "窓を閉じてもベイクは続く");
    h.get_by_label("取消: メッシュマップをベイク").click();
    h.step();
    assert!(h.state().state.bake.progress().unwrap().canceling);
    settle(&mut h);
    assert!(h.state().state.sets.current().mesh_maps.is_empty());
}

#[test]
fn the_window_moves_by_its_title_bar_stays_on_screen_and_scrolls_many_sets() {
    let mut h = cube_app(64);
    open_bake_window(&mut h);
    let before = yolu_app::windows::window_rect(&h.ctx, "bake").unwrap();
    // 見出しをつかんで左下へ動かす
    let grab = pos2(before.center().x - 100.0, before.top() + 14.0);
    drag(
        &mut h,
        &[
            grab,
            pos2(grab.x - 80.0, grab.y + 40.0),
            pos2(grab.x - 160.0, grab.y + 90.0),
        ],
    );
    let after = yolu_app::windows::window_rect(&h.ctx, "bake").unwrap();
    assert!(
        (after.min - before.min - vec2(-160.0, 90.0)).length() < 2.0,
        "{before:?} → {after:?}"
    );
    // 画面の外へは出さない
    let grab = pos2(after.center().x - 100.0, after.top() + 14.0);
    drag(&mut h, &[grab, pos2(grab.x - 900.0, grab.y - 900.0)]);
    let clamped = yolu_app::windows::window_rect(&h.ctx, "bake").unwrap();
    assert!(clamped.left() >= 0.0 && clamped.top() >= 0.0, "{clamped:?}");
    // セットが多いときは、焼くセットの欄の中でスクロールする（描き続けて壊れない）
    let model = Model {
        generation: 1,
        name: "多".into(),
        materials: (0..9).map(|i| info(&format!("Mat{i}"))).collect(),
        meshes: vec![MeshData {
            key: "0".into(),
            name: "板".into(),
            skinned: false,
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            submeshes: (0..9)
                .map(|m| Submesh {
                    material: m,
                    indices: vec![0, 1, 2],
                })
                .collect(),
        }],
    };
    h.state_mut().load_live_link_model(&model).unwrap();
    h.run();
    assert_eq!(h.state().state.sets.len(), 9);
    check(&h, "Mat0");
    let rect = yolu_app::windows::window_rect(&h.ctx, "bake").unwrap();
    for _ in 0..3 {
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -60.0),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        });
        move_to(&h, pos2(rect.left() + 100.0, rect.top() + 90.0));
        h.run();
    }
    shot(&mut h, "bake", "bake_window_many_sets");
    assert!(rect.width() > 0.0);
}

#[test]
fn the_bake_window_refuses_with_a_short_reason_and_speaks_english() {
    let mut h = app(1280.0, 800.0, 64);
    open_bake_window(&mut h);
    // モデルが無い: ボタンは押せず、理由が出る
    assert_eq!(
        h.state_mut().state.bake_refusal().as_deref(),
        Some("モデルがありません")
    );
    h.get_by_label("チェックしたマップをベイク").click();
    h.run();
    assert!(!h.state().state.bake.is_baking());
    assert!(h.state().state.bake.outcome.is_none());
    shot(&mut h, "bake", "bake_window_no_model");
    // 英語
    h.state_mut().state.lang = Lang::En;
    h.run();
    h.get_by_label("Bake Checked Maps");
    h.get_by_label("Close");
    assert!(checked(&h, "World Normal"));
    assert_eq!(
        h.state_mut().state.bake_refusal().as_deref(),
        Some("No model")
    );
    apply(&mut h, Action::LoadDemoModel);
    shot(&mut h, "bake", "bake_window_english");
    // Esc: 焼いていなければ窓を閉じる（窓の上にポインタがあるとき）
    move_to(&h, egui::pos2(640.0, 400.0));
    h.run();
    key(&h, Key::Escape, egui::Modifiers::NONE);
    h.run();
    assert!(h.state().state.bake.window.is_none());
}

/// 窓の中の、名前のボタンの矩形（同じ名前の部品と区別するため、窓の中で探す）。
fn bake_button(h: &Harness<'_, YoluApp>, label: &str) -> egui::Rect {
    let win = yolu_app::windows::window_rect(&h.ctx, "bake").expect("窓");
    rect_of(h, label, |r| win.contains(r.center()))
}

#[test]
fn the_bake_window_switches_where_to_bake_and_shows_the_adapter_or_why_it_fell_back() {
    let mut h = cube_app(64);
    open_bake_window(&mut h);
    assert_eq!(
        h.state().state.bake.backend,
        BakeBackend::Cpu,
        "試験の台は CPU に固定"
    );
    assert!(
        yolu_app::bake::probe_line(
            Lang::Ja,
            BakeBackend::Cpu,
            &h.state().state.bake.gpu_probe()
        )
        .is_none(),
        "CPU を選んでいれば GPU の確認の行は無い"
    );
    // 自動: GPU の確認の結果は決めておく（別のスレッドで確かめない）
    h.state()
        .state
        .bake
        .fix_gpu_probe(false, Err("使えるハードウェアの GPU がありません".into()));
    let at = bake_button(&h, "自動").center();
    click(&mut h, at);
    assert_eq!(h.state().state.bake.backend, BakeBackend::Auto);
    let line = yolu_app::bake::probe_line(
        Lang::Ja,
        BakeBackend::Auto,
        &h.state().state.bake.gpu_probe(),
    )
    .unwrap();
    assert!(
        line.warn && line.text == "CPU で焼く（GPU を使えません）",
        "{line:?}"
    );
    shot(&mut h, "bake", "bake_window_cpu_fallback");
    // GPU: アダプターの名前・種別・ray query。最後のベイクを行った場所も出る。英語。
    // 確認の結果は、選ぶ前（今は CPU を選んで確かめに行かない間）に決めておく。選んだ後の毎フレームは決めた結果を使う
    let at = bake_button(&h, "CPU").center();
    click(&mut h, at);
    let adapter = BakeAdapter {
        name: "Test Adapter".into(),
        backend: "Vulkan".into(),
        device_type: "DiscreteGpu".into(),
        software: false,
        ray_query: true,
    };
    h.state()
        .state
        .bake
        .fix_gpu_probe(true, Ok(adapter.clone()));
    h.state_mut().state.lang = Lang::En;
    h.run();
    let at = bake_button(&h, "GPU").center();
    click(&mut h, at);
    assert_eq!(h.state().state.bake.backend, BakeBackend::Gpu);
    let run = BakeRun {
        requested: BakeBackend::Gpu,
        gpu: Some((
            adapter,
            yolu_gpu::GpuBakeStats {
                method: yolu_app::bake::GpuBakeMethod::RayQuery,
                dispatches: 4,
                max_dispatch_ms: 1.0,
                max_dispatch_texels: 256,
                bands: 1,
                input_bytes: 0,
                band_bytes: 0,
                ray_query_note: None,
            },
        )),
        fallback_kind: None,
        fallback_reason: None,
    };
    h.state_mut()
        .state
        .sets
        .get_mut(0)
        .unwrap()
        .mesh_maps
        .set_run(run);
    h.run();
    shot(&mut h, "bake", "bake_window_gpu");
    // CPU へ戻すと GPU の確認の行は消える
    let at = bake_button(&h, "CPU").center();
    click(&mut h, at);
    assert_eq!(h.state().state.bake.backend, BakeBackend::Cpu);
}

// ───────── 書き出し ─────────

fn paint_left_half(doc: &mut yolu_core::Document, layer: LayerId, rgba: [u8; 4]) {
    let ts = doc.tile_size();
    let (w, h) = (doc.width(), doc.height());
    for ty in 0..h.div_ceil(ts) {
        for tx in 0..(w / 2 + 1).div_ceil(ts) {
            let mut tile = vec![0u8; (ts * ts * 4) as usize];
            for y in 0..ts.min(h - ty * ts) {
                for x in 0..ts.min(w / 2 + 1 - tx * ts) {
                    tile[((y * ts + x) * 4) as usize..][..4].copy_from_slice(&rgba);
                }
            }
            doc.import_tile(layer, Channel::Color, TileCoord::new(tx, ty), &tile)
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

/// Skin（UV の左半分）と Hair（右半分）の 2 枚の板。
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

/// 2 つのセット（どちらも 64 × 64、左半分を塗ってある）を持つ画面。
fn two_sets_app() -> Harness<'static, YoluApp> {
    let mut h = app(1280.0, 800.0, 64);
    h.state_mut().load_live_link_model(&two_quads()).unwrap();
    let s = &mut h.state_mut().state;
    let layer = s.selected_layer.unwrap();
    paint_left_half(&mut s.doc, layer, [255, 0, 0, 255]);
    let other = 1 - s.sets.current_index();
    let (mut fresh, first) = blank_document(64, 64);
    paint_left_half(&mut fresh, first.unwrap(), [0, 255, 0, 255]);
    *s.set_doc_mut(other) = fresh;
    h.run();
    h
}

#[test]
fn exporting_asks_before_replacing_then_shows_the_short_list() {
    let dir = TempDir::new("export");
    let mut h = two_sets_app();
    let export = |h: &mut Harness<'_, YoluApp>| {
        apply(
            h,
            Action::Export(ExportAction::TemplateTo {
                id: "unity-standard".into(),
                dir: dir.0.clone(),
            }),
        )
    };
    export(&mut h);
    settle(&mut h);
    assert_eq!(
        dir.files(),
        ["Texture_Hair_Albedo.png", "Texture_Skin_Albedo.png"]
    );
    // 結果の窓: 書いた画像の一覧
    assert!(h.state().state.export.report.is_some());
    {
        // 撮る絵は毎回同じに（書き出し先は毎回違う）
        let s = &mut h.state_mut().state;
        s.message = "試験".into();
        s.export.report.as_mut().unwrap().dir = PathBuf::from("/out");
    }
    h.run();
    shot(&mut h, "export-report", "export_report");
    h.get_by_label("閉じる").click();
    h.run();
    assert!(h.state().state.export.report.is_none());

    // もう一度: もうあるファイルを確かめる（まだ何も書かない）
    let before = std::fs::read(dir.0.join("Texture_Skin_Albedo.png")).unwrap();
    {
        let s = &mut h.state_mut().state;
        let layer = s.selected_layer.unwrap();
        s.doc.set_layer_opacity(layer, 0.5, false).unwrap();
    }
    export(&mut h);
    assert!(h.state().state.export.confirm.is_some());
    assert!(!h.state().state.export.is_exporting());
    shot(&mut h, "export-confirm", "export_confirm");
    // 確かめの窓の間は、キーの割り当てが働かない
    key(&h, Key::E, egui::Modifiers::NONE);
    h.run();
    assert_eq!(
        h.state().state.tool,
        yolu_app::state::Tool::Brush,
        "確かめの窓の間は E で消しゴムにならない"
    );
    // やめる
    h.get_by_label("やめる").click();
    h.run();
    assert!(h.state().state.export.confirm.is_none());
    assert_eq!(
        std::fs::read(dir.0.join("Texture_Skin_Albedo.png")).unwrap(),
        before,
        "やめたら元のファイルのまま"
    );
    // 置き換える
    export(&mut h);
    h.get_by_label("置き換える").click();
    h.run();
    settle(&mut h);
    assert_ne!(
        std::fs::read(dir.0.join("Texture_Skin_Albedo.png")).unwrap(),
        before
    );
    let report = h.state().state.export.report.as_ref().unwrap();
    assert!(report.images.iter().all(|i| i.replaced));
    assert!(dir.files().iter().all(|f| f.ends_with(".png")));
    // 閉じるボタンの × でも閉じる
    h.get_by_label("閉じる").click();
    h.run();
    assert!(h.state().state.export.report.is_none());
}

#[test]
fn the_export_job_card_cancels_and_leaves_the_folder_untouched() {
    let dir = TempDir::new("card");
    let mut h = app(1280.0, 800.0, 256);
    {
        let s = &mut h.state_mut().state;
        let layer = s.selected_layer.unwrap();
        paint_left_half(&mut s.doc, layer, [1, 2, 3, 255]);
        s.doc
            .set_channel_enabled(layer, Channel::Emission, true)
            .unwrap();
    }
    // 取消が来るまで始めない仕事にする（札の取消が効いたことを、書き終わる速さに頼らず確かめる）
    h.state_mut().state.export.park_next = true;
    apply(
        &mut h,
        Action::Export(ExportAction::TemplateTo {
            id: "unity-standard".into(),
            dir: dir.0.clone(),
        }),
    );
    assert!(h.state().state.export.is_exporting());
    h.get_by_label("取消: 書き出し").click();
    h.step();
    assert!(h.state().state.export.progress().unwrap().canceling);
    settle(&mut h);
    assert!(h.state().state.message.contains("取り消しました"));
    assert!(dir.files().is_empty(), "{:?}", dir.files());
}

// ───────── PSD ─────────

fn write_psd(path: &Path) -> Vec<u8> {
    let mut s = AppState::new(64, 64);
    s.bake.backend = yolu_app::bake::BakeBackend::Cpu;
    let layer = s.selected_layer.unwrap();
    paint_left_half(&mut s.doc, layer, [255, 0, 0, 255]);
    s.apply(Action::Psd(PsdAction::Export(path.to_path_buf())));
    s.wait_psd();
    std::fs::read(path).unwrap()
}

#[test]
fn importing_a_psd_adds_a_set_and_a_refused_one_shows_its_reasons() {
    let dir = TempDir::new("psd");
    let good = dir.0.join("Body.psd");
    let bytes = write_psd(&good);
    let mut h = app(1280.0, 800.0, 32);
    apply(
        &mut h,
        Action::Psd(PsdAction::Import {
            path: good.clone(),
            target: PsdTarget::NewSet,
        }),
    );
    settle(&mut h);
    let s = &h.state().state;
    assert_eq!(s.sets.len(), 2);
    assert_eq!(s.sets.current().name, "Body");
    assert_eq!((s.doc.width(), s.doc.height()), (64, 64));
    // テクスチャセットのパネルに出る
    h.get_by_label("Body");

    // 原本の保持だけの PSD（PSB）: 何も変えず、理由の窓
    let mut psb = bytes.clone();
    psb[4..6].copy_from_slice(&2u16.to_be_bytes());
    let psb_path = dir.0.join("Big.psd");
    std::fs::write(&psb_path, &psb).unwrap();
    apply(
        &mut h,
        Action::Psd(PsdAction::Import {
            path: psb_path,
            target: PsdTarget::NewSet,
        }),
    );
    settle(&mut h);
    assert_eq!(h.state().state.sets.len(), 2, "何も変えない");
    assert!(h.state().state.psd.report.is_some());
    shot(&mut h, "psd-report", "psd_report_refused");
    h.get_by_label("閉じる").click();
    h.run();
    assert!(h.state().state.psd.report.is_none());
    // 英語
    h.state_mut().state.lang = Lang::En;
    apply(
        &mut h,
        Action::Psd(PsdAction::Import {
            path: dir.0.join("Big.psd"),
            target: PsdTarget::NewSet,
        }),
    );
    settle(&mut h);
    h.get_by_label("Close");
    assert!(h.state().state.message.contains("Preserve only"));
}

/// 窓の中に描いた文字（描いた順）。
fn window_texts(h: &Harness<'_, YoluApp>, window: &str) -> Vec<String> {
    use egui::epaint::Shape;
    fn walk(shape: &Shape, area: egui::Rect, out: &mut Vec<String>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, area, out)),
            Shape::Text(text)
                if area
                    .expand(1.0)
                    .contains_rect(egui::Rect::from_min_size(text.pos, text.galley.size())) =>
            {
                out.push(text.galley.job.text.clone())
            }
            _ => {}
        }
    }
    let area = yolu_app::windows::window_rect(&h.ctx, window)
        .unwrap_or_else(|| panic!("{window} を描いていない"));
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        walk(&shape.shape, area, &mut out);
    }
    out
}

/// 窓の中の、名前の部品の中心（同じ名前のドックの部品に取り違えない）。
fn in_window(h: &Harness<'_, YoluApp>, window: &str, label: &str) -> egui::Pos2 {
    let area = yolu_app::windows::window_rect(&h.ctx, window)
        .unwrap_or_else(|| panic!("{window} を描いていない"));
    rect_of(h, label, |r| area.contains_rect(r)).center()
}

#[test]
fn exporting_a_psd_with_an_inverted_mask_lists_what_it_bakes_and_writes_only_after_the_yes() {
    let dir = TempDir::new("psd-bake");
    let mut h = app(1280.0, 800.0, 64);
    let layer = h.state().state.selected_layer.unwrap();
    apply(&mut h, Action::M2(yolu_app::m2::Edit::AddMask(layer)));
    // マスクそのものは PSD に書ける。PSD に非破壊の反転が無い、反転したマスクは、反転した値の画素に焼く
    apply(
        &mut h,
        Action::M2(yolu_app::m2::Edit::MaskInverted(layer, true)),
    );
    let before = h.state().state.doc.revision();
    apply(
        &mut h,
        Action::Psd(PsdAction::Export(dir.0.join("out.psd"))),
    );
    settle(&mut h);
    assert!(dir.files().is_empty(), "確かめるまで何も書かない");
    assert!(h.state().state.psd.notes_confirm.is_some());
    let texts = window_texts(&h, "psd-bake");
    assert!(texts.iter().any(|t| t == "反転したマスク"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "マスクの画素へ"), "{texts:?}");
    assert!(texts.iter().any(|t| t.contains("焼く 1")), "{texts:?}");
    shot(&mut h, "psd-bake", "psd_export_check");
    // やめる
    h.get_by_label("やめる").click();
    h.run();
    assert!(h.state().state.psd.notes_confirm.is_none());
    assert!(dir.files().is_empty());
    // もう一度、書く
    apply(
        &mut h,
        Action::Psd(PsdAction::Export(dir.0.join("out.psd"))),
    );
    settle(&mut h);
    h.get_by_label("書く").click();
    h.run();
    settle(&mut h);
    assert_eq!(dir.files(), ["out.psd"]);
    assert!(h.state().state.message.contains("書き出しました"));
    assert_eq!(h.state().state.doc.revision(), before, "文書は変わらない");
    assert!(h
        .state()
        .state
        .doc
        .layer(layer)
        .unwrap()
        .mask()
        .unwrap()
        .inverted());
    // 英語
    h.state_mut().state.lang = Lang::En;
    apply(
        &mut h,
        Action::Psd(PsdAction::Export(dir.0.join("again.psd"))),
    );
    settle(&mut h);
    let texts = window_texts(&h, "psd-bake");
    assert!(texts.iter().any(|t| t == "Inverted mask"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "To mask pixels"), "{texts:?}");
    // 日本語が残ってよいのは、利用者の名前（層の名前）だけ
    let layer_name = h.state().state.doc.layer(layer).unwrap().name().to_owned();
    assert!(
        texts.iter().all(|t| !has_japanese(t) || *t == layer_name),
        "{texts:?}"
    );
    shot(&mut h, "psd-bake", "psd_export_check_english");
    h.get_by_label("Cancel").click();
    h.run();
    assert_eq!(dir.files(), ["out.psd"]);
}

#[test]
fn the_psd_check_window_lists_bakes_rounds_and_drops_per_channel() {
    use yolu_core::{
        AdjustmentSettings, AnchorPlacement, EffectSettings, FilterSpec, FilterTarget,
    };
    let dir = TempDir::new("psd-check");
    let mut h = app(1280.0, 800.0, 64);
    {
        let s = &mut h.state_mut().state;
        let base = s.selected_layer.unwrap();
        paint_left_half(&mut s.doc, base, [200, 80, 40, 255]);
        s.doc
            .add_filter(
                base,
                FilterTarget::Content,
                FilterSpec::new(EffectSettings::blur(3)).channels(&[Channel::Color]),
            )
            .unwrap();
        s.doc
            .add_anchor(base, AnchorPlacement::Layer, None, None)
            .unwrap();
        s.doc
            .add_fill_layer(
                "ガラス",
                &[(Channel::Color, yolu_core::Rgba8::new(10, 90, 200, 120))],
                None,
            )
            .unwrap();
        s.doc
            .add_adjustment_layer(
                "明るさ",
                AdjustmentSettings::levels(0.3, 1.0, 1.234, 0.0, 1.0).unwrap(),
                None,
                None,
            )
            .unwrap();
        s.psd.export.channels = vec![Channel::Color, Channel::Roughness];
    }
    apply(
        &mut h,
        Action::Psd(PsdAction::Export(dir.0.join("out.psd"))),
    );
    settle(&mut h);
    let texts = window_texts(&h, "psd-bake");
    for want in [
        "カラー",
        "ラフネス",
        "ガラス",
        "半透明の塗りつぶし",
        "明るさ",
        "アンカー",
        "落とす",
    ] {
        assert!(texts.iter().any(|t| t == want), "{want}: {texts:?}");
    }
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("レベル補正: 入力の黒 76.5→76、ガンマ 1.234→1.23")),
        "{texts:?}"
    );
    assert!(texts.iter().any(|t| t.starts_with("最大差 ")), "{texts:?}");
    shot(&mut h, "psd-bake", "psd_export_check_many");
    h.get_by_label("書く").click();
    h.run();
    settle(&mut h);
    assert_eq!(dir.files(), ["out_Color.psd", "out_Roughness.psd"]);
}

#[test]
fn the_psd_export_window_chooses_the_mode_and_channels_and_asks_for_the_file() {
    let mut h = app(1280.0, 800.0, 64);
    apply(&mut h, Action::Psd(PsdAction::ExportDialog));
    let texts = window_texts(&h, "psd-export");
    for want in [
        "PSD の書き出し",
        "方式",
        "焼き込んで書く",
        "平らに 1 枚",
        "チャンネル",
        "カラー",
        "ラフネス",
        "ノーマル",
        "やめる",
        "書き出し…",
    ] {
        assert!(texts.iter().any(|t| t == want), "{want}: {texts:?}");
    }
    shot(&mut h, "psd-export", "psd_export_options");
    // 方式
    let at = in_window(&h, "psd-export", "平らに 1 枚");
    click(&mut h, at);
    assert_eq!(
        h.state().state.psd.export.mode,
        yolu_io::psd::ExportMode::Flat
    );
    let at = in_window(&h, "psd-export", "焼き込んで書く");
    click(&mut h, at);
    assert_eq!(
        h.state().state.psd.export.mode,
        yolu_io::psd::ExportMode::Bake
    );
    // チャンネル: 足す・外す・最後の 1 つは外せない
    let channels = |h: &Harness<'_, YoluApp>| h.state().state.psd.export.channels.clone();
    assert_eq!(channels(&h), [Channel::Color]);
    let at = in_window(&h, "psd-export", "ラフネス");
    click(&mut h, at);
    assert_eq!(channels(&h), [Channel::Color, Channel::Roughness]);
    let at = in_window(&h, "psd-export", "カラー");
    click(&mut h, at);
    assert_eq!(channels(&h), [Channel::Roughness]);
    let at = in_window(&h, "psd-export", "ラフネス");
    click(&mut h, at);
    assert_eq!(channels(&h), [Channel::Roughness], "最後の 1 つは外せない");
    // 書き出し…: 窓を閉じて、書き出す先を選ぶ窓を頼む。初めのファイル名は 1 つのチャンネルなら末尾にチャンネル
    let name = yolu_app::psd::default_export_name(&h.state().state);
    assert!(name.ends_with("_Roughness.psd"), "{name}");
    let at = in_window(&h, "psd-export", "書き出し…");
    click(&mut h, at);
    assert!(!h.state().state.psd.options_open);
    assert_eq!(
        h.state().state.dialog_request,
        Some(DialogRequest::PsdExport)
    );
    // 英語: 窓の文字に日本語が残らない。やめるで閉じる
    h.state_mut().state.dialog_request = None;
    h.state_mut().state.lang = Lang::En;
    apply(&mut h, Action::Psd(PsdAction::ExportDialog));
    let texts = window_texts(&h, "psd-export");
    for want in [
        "Export PSD",
        "Mode",
        "Bake and write",
        "Flatten to one layer",
        "Channels",
        "Export…",
        "Cancel",
    ] {
        assert!(texts.iter().any(|t| t == want), "{want}: {texts:?}");
    }
    assert!(texts.iter().all(|t| !has_japanese(t)), "{texts:?}");
    shot(&mut h, "psd-export", "psd_export_options_english");
    let at = in_window(&h, "psd-export", "Cancel");
    click(&mut h, at);
    assert!(!h.state().state.psd.options_open);
}

// ───────── 画面なしの保存の往復（Windows 向けに組んで wine でも回す） ─────────

/// 焼いたメッシュマップを .ylp に保存して開き直すと、同じ中身で戻り、今の条件と合えば最新。
#[test]
fn headless_baked_mesh_maps_survive_save_and_reopen() {
    let dir = TempDir::new("hsave");
    let path = dir.0.join("baked.ylp");
    let quick = |s: &mut AppState| {
        s.bake.settings.maps = vec![MeshMapKind::WorldNormal, MeshMapKind::AmbientOcclusion];
        s.bake.settings.ao_samples = 8;
        s.bake.settings.padding = 4;
    };
    let mut s = AppState::new(64, 64);
    s.bake.backend = yolu_app::bake::BakeBackend::Cpu;
    s.apply(Action::LoadDemoModel);
    quick(&mut s);
    s.apply(Action::Bake(BakeAction::Start));
    s.wait_bake();
    assert!(s.modified);
    assert_eq!(s.sets.current().mesh_maps.unsaved().len(), 2);
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.contains("メッシュマップ 2 枚"), "{}", s.message);
    assert!(!s.modified);
    assert!(
        s.sets.current().mesh_maps.unsaved().is_empty(),
        "書いたら保存済み"
    );
    let maps: Vec<_> = s.sets.current().mesh_maps.iter().cloned().collect();

    // 開き直す: 同じ中身（16 bit の正本そのまま）
    let mut again = AppState::new(64, 64);
    again.bake.backend = yolu_app::bake::BakeBackend::Cpu;
    again.apply(Action::OpenProject(path.clone()));
    assert!(
        again.message.contains("メッシュマップ 2 枚"),
        "{}",
        again.message
    );
    let loaded: Vec<_> = again.sets.current().mesh_maps.iter().cloned().collect();
    assert_eq!(loaded.len(), 2);
    for (a, b) in maps.iter().zip(&loaded) {
        assert_eq!(a.provenance(), b.provenance());
        assert!(
            a.data() == b.data() && a.coverage() == b.coverage(),
            "{:?}",
            a.kind()
        );
    }
    assert!(
        again.sets.current().mesh_maps.unsaved().is_empty(),
        "開いたものは保存済み"
    );
    // 開いただけでは古くならない: 焼く設定は保存したマップの条件にそろう（モデルが無いあいだは照合できないので「未確認」）
    assert_eq!((again.bake.settings.padding, again.bake.settings.ao_samples), (4, 8));
    let check = again.mesh_map_check(0, MeshMapKind::WorldNormal).unwrap();
    assert_eq!(check.state, MeshMapState::Unverified);
    // 設定を変えれば、前のマップは古い（照合できなくても、条件の違いは分かる）
    again.bake.settings.padding = 12;
    let stale = again.mesh_map_check(0, MeshMapKind::WorldNormal).unwrap();
    assert_eq!(stale.state, MeshMapState::Stale, "設定が焼いたときと違う");
    quick(&mut again);
    let check = again.mesh_map_check(0, MeshMapKind::WorldNormal).unwrap();
    assert_eq!(check.state, MeshMapState::Unverified);
    // 同じモデル・同じ設定なら最新
    again.apply(Action::LoadDemoModel);
    for kind in [MeshMapKind::WorldNormal, MeshMapKind::AmbientOcclusion] {
        assert_eq!(
            again.mesh_map_check(0, kind).unwrap().state,
            MeshMapState::Current,
            "{kind:?}"
        );
    }

    // もう一度保存しても、焼いていないマップはファイルのバイト列のまま残る
    again.apply(Action::SaveProject);
    assert!(
        again.message.starts_with("保存しました"),
        "{}",
        again.message
    );
    assert!(
        !again.message.contains("メッシュマップ"),
        "書き直さない: {}",
        again.message
    );
    let project = yolu_io::Project::read(&std::fs::read(&path).unwrap()).unwrap();
    let set_id = project.sets()[0].id.clone();
    for kind in [MeshMapKind::WorldNormal, MeshMapKind::AmbientOcclusion] {
        let map = project
            .mesh_map(&set_id, kind, 1 << 30)
            .unwrap()
            .expect("残っている");
        assert_eq!(map.provenance().width, 64);
    }
    // 焼き直して保存すると、同じ種類だけ置き換わる
    again.bake.settings.maps = vec![MeshMapKind::WorldNormal];
    again.bake.settings.padding = 8;
    again.apply(Action::Bake(BakeAction::Start));
    again.wait_bake();
    assert_eq!(again.sets.current().mesh_maps.unsaved().len(), 1);
    again.apply(Action::SaveProject);
    let project = yolu_io::Project::read(&std::fs::read(&path).unwrap()).unwrap();
    let normal = project
        .mesh_map(&set_id, MeshMapKind::WorldNormal, 1 << 30)
        .unwrap()
        .unwrap();
    assert_eq!(normal.provenance().padding, 8);
    let ao = project
        .mesh_map(&set_id, MeshMapKind::AmbientOcclusion, 1 << 30)
        .unwrap()
        .unwrap();
    assert_eq!(ao.provenance().padding, 4, "焼き直していない種類はそのまま");
}

/// GPU で焼いたメッシュマップも、.ylp に保存して開き直すと 16 bit の値・覆い・由来がそのまま戻り、今の条件と合えば最新になる
/// （焼いた場所は保存しない。GPU を使えない環境では省く）。
#[test]
fn headless_gpu_baked_mesh_maps_survive_save_and_reopen() {
    let dir = TempDir::new("hgpu");
    let path = dir.0.join("gpu.ylp");
    let quick = |s: &mut AppState| {
        s.bake.settings.maps = vec![MeshMapKind::WorldNormal, MeshMapKind::AmbientOcclusion];
        s.bake.settings.ao_samples = 8;
        s.bake.settings.padding = 4;
    };
    let mut s = AppState::new(64, 64);
    // 「GPU」はソフトウェアの描画も許す。使えるか確かめる（別のスレッド）
    s.apply(Action::Bake(BakeAction::Backend(BakeBackend::Gpu)));
    let start = Instant::now();
    let probe = loop {
        if let yolu_app::bake::GpuProbe::Done { result, .. } = s.bake.gpu_probe() {
            break result;
        }
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "GPU の確認が終わらない"
        );
        std::thread::sleep(Duration::from_millis(2));
    };
    if let Err(why) = probe {
        eprintln!("GPU を使えない環境なので、GPU で焼いた結果の保存と復元の試験は省く: {why}");
        return;
    }
    s.apply(Action::LoadDemoModel);
    quick(&mut s);
    s.apply(Action::Bake(BakeAction::Start));
    s.wait_bake();
    let run = s
        .sets
        .current()
        .mesh_maps
        .run()
        .cloned()
        .expect("記録がある");
    assert!(run.used_gpu(), "GPU で焼いた: {:?}", run.fallback_reason);
    assert_eq!(s.sets.current().mesh_maps.unsaved().len(), 2);
    let maps: Vec<_> = s.sets.current().mesh_maps.iter().cloned().collect();
    let normal = &maps[0];
    assert!(
        normal.data().iter().any(|v| *v != normal.data()[0]) && normal.coverage().contains(&1),
        "値も覆いも空でない"
    );
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.contains("メッシュマップ 2 枚"), "{}", s.message);
    assert!(s.sets.current().mesh_maps.unsaved().is_empty());

    // 開き直す: GPU で焼いた 16 bit の値・覆い・由来がそのまま（保存で丸めたり、CPU で焼き直したりしない）
    let mut again = AppState::new(64, 64);
    again.bake.backend = BakeBackend::Cpu;
    again.apply(Action::OpenProject(path));
    let loaded: Vec<_> = again.sets.current().mesh_maps.iter().cloned().collect();
    assert_eq!(loaded.len(), 2);
    for (a, b) in maps.iter().zip(&loaded) {
        assert_eq!(a.provenance(), b.provenance());
        assert!(a.data() == b.data(), "{:?} の値", a.kind());
        assert!(a.coverage() == b.coverage(), "{:?} の覆い", a.kind());
    }
    assert!(again.sets.current().mesh_maps.unsaved().is_empty());
    // 焼いた場所は保存していない（開いたものには場所の記録が無い）
    assert!(again.sets.current().mesh_maps.run().is_none());
    // 同じモデル・同じ設定なら、GPU で焼いたものも今の条件に合う（由来は CPU と区別しない）
    quick(&mut again);
    again.apply(Action::LoadDemoModel);
    for kind in [MeshMapKind::WorldNormal, MeshMapKind::AmbientOcclusion] {
        let check = again.mesh_map_check(0, kind).unwrap();
        assert_eq!(
            check.state,
            MeshMapState::Current,
            "{kind:?}: {:?}",
            check.reasons
        );
    }
}

/// 2 つのセットのメッシュマップはセットごとに保存され、新しいプロジェクトの最初の保存でも書かれる。
#[test]
fn headless_each_sets_mesh_maps_are_saved_under_its_own_set() {
    let dir = TempDir::new("hsets");
    let path = dir.0.join("two.ylp");
    let mut s = AppState::new(64, 64);
    s.bake.backend = yolu_app::bake::BakeBackend::Cpu;
    s.receive_link_model(&two_quads(), 0).1.unwrap();
    s.bake.settings.maps = vec![MeshMapKind::Position];
    s.bake.settings.padding = 2;
    s.apply(Action::Bake(BakeAction::Start));
    s.wait_bake();
    assert_eq!(s.sets.len(), 2);
    assert!(s.sets.iter().all(|x| x.mesh_maps.len() == 1));
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.contains("メッシュマップ 2 枚"), "{}", s.message);
    let project = yolu_io::Project::read(&std::fs::read(&path).unwrap()).unwrap();
    let slots: Vec<Vec<i32>> = project
        .sets()
        .iter()
        .map(|set| {
            project
                .mesh_map(&set.id, MeshMapKind::Position, 1 << 30)
                .unwrap()
                .expect("セットごとに保存")
                .provenance()
                .target_slots
                .clone()
        })
        .collect();
    assert_eq!(slots, [vec![0], vec![1]], "セットごとのスロット");
}

/// 読み込んだ PSD のセットは、.ylp に保存して開き直しても同じ絵・同じレイヤーで戻り、そのまま PSD へ書き出せる。
#[test]
fn headless_an_imported_psd_survives_save_and_reopen_and_exports_again() {
    let dir = TempDir::new("hpsd");
    let psd = dir.0.join("Cloth.psd");
    write_psd(&psd);
    let mut s = AppState::new(32, 32);
    s.bake.backend = yolu_app::bake::BakeBackend::Cpu;
    s.apply(Action::Psd(PsdAction::Import {
        path: psd.clone(),
        target: PsdTarget::NewSet,
    }));
    s.wait_psd();
    assert_eq!(s.sets.len(), 2);
    let composite = |s: &AppState| s.doc.composite(s.doc.bounds()).unwrap();
    let before = composite(&s);
    let names: Vec<String> = s.doc.layers().iter().map(|l| l.name().to_owned()).collect();
    let project = dir.0.join("psd.ylp");
    s.apply(Action::SaveProjectAs(project.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);

    let mut again = AppState::new(32, 32);

    again.bake.backend = yolu_app::bake::BakeBackend::Cpu;
    again.apply(Action::OpenProject(project));
    assert_eq!(again.sets.len(), 2);
    let index = again
        .sets
        .iter()
        .position(|x| x.name == "Cloth")
        .expect("セットの名前");
    again.switch_set(index).unwrap();
    assert!(composite(&again) == before, "同じ絵");
    let reopened: Vec<String> = again
        .doc
        .layers()
        .iter()
        .map(|l| l.name().to_owned())
        .collect();
    assert_eq!(reopened, names, "同じレイヤー");
    assert!(again.read_only_reason().is_none(), "編集できる");
    // そのまま PSD へ書き出せる（別のファイルへ）
    let out = dir.0.join("again.psd");
    again.apply(Action::Psd(PsdAction::Export(out.clone())));
    again.wait_psd();
    assert!(out.exists(), "{}", again.message);
}
