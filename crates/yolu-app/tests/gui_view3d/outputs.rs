//! メッシュマップのベイク・テンプレートの書き出し・PSD の読み書きの画面（egui_kittest）: ウィンドウを開く → 選ぶ → 結果のファイル・文書が変わる、
//! 取消、上書きの確かめ、断る理由、日本語と英語。`headless_` で始まる試験は画面を描かず、Wine でも回る。
use crate::common;

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

/// ウィンドウの中だけを撮って、正解の絵と比べる（ほかのパネルの変更で壊れない）。
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

/// 「メッシュマップをベイク…」を開く（入口はテクスチャセットの帯のボタン。ボタンの押し方は `sets` の試験が見る）。
fn open_bake_window(h: &mut Harness<'_, YoluApp>) {
    h.state_mut()
        .state
        .apply(Action::Bake(yolu_app::bake::BakeAction::OpenWindow));
    h.run();
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

/// ポインタを動かして 2 フレーム回す（入れ子のメニューは、行に乗せると開く）。
fn hover(h: &mut Harness<'_, YoluApp>, at: egui::Pos2) {
    h.event(egui::Event::PointerMoved(at));
    h.step();
    h.step();
}

/// 開いている「ファイル」のメニューで、入れ子の「インポート」を開いて、その中の項目を押す。
fn click_import_item(h: &mut Harness<'_, YoluApp>, import: &str, item: &str) {
    // ポインタが前の位置を持つように 1 度動かしてから、入れ子の行へ乗せる
    hover(h, pos2(640.0, 400.0));
    let row = popup_item(h, import);
    hover(h, row.center());
    let target = popup_item(h, item);
    hover(h, pos2(row.right() - 2.0, row.center().y));
    hover(h, target.center());
    click(h, target.center());
}

#[test]
fn the_menus_hold_import_export_and_bake_and_only_ask_for_files() {
    let mut h = app(1280.0, 800.0, 64);
    let at = menu_title(&h, "ファイル").center();
    click(&mut h, at);
    // インポート（入れ子のメニュー）。開いた形の絵も撮る
    hover(&mut h, pos2(640.0, 400.0));
    let row = popup_item(&h, "インポート");
    hover(&mut h, row.center());
    assert_eq!(
        h.state().state.popup.as_ref().unwrap().state.open_depth(),
        1
    );
    h.snapshot("menu_file_import");
    click_import_item(&mut h, "インポート", "PSD を今のテクスチャセットに…");
    assert_eq!(
        h.state().state.dialog_request,
        Some(DialogRequest::PsdImport(PsdTarget::CurrentSet))
    );
    h.state_mut().state.dialog_request = None;
    // 書き出しは 1 つのウィンドウ（テクスチャセット・出力先・出力テンプレート・パディング）と PSD。テンプレートや PNG の項目は並べない
    let at = menu_title(&h, "ファイル").center();
    click(&mut h, at);
    for gone in [
        "テンプレート: Unity Standard / URP Lit…",
        "テンプレート: HDRP Lit…",
        "テンプレート: lilToon…",
        "チャンネルを PNG…",
        "全チャンネルを画像に…",
        "PSD…",
        "書き出し…",
        "PSD を書き出し…",
        "PSD を新しいテクスチャセットへ…",
        "PSD を今のセットの文書へ…",
    ] {
        assert!(h.query_by_label(gone).is_none(), "{gone}");
    }
    let at = popup_item(&h, "テクスチャを書き出す…").center();
    click(&mut h, at);
    assert!(h.state().state.export.window.open);
    assert_eq!(
        h.state().state.dialog_request,
        None,
        "ファイルのウィンドウは、ウィンドウの「選ぶ…」まで出さない"
    );
    apply(&mut h, Action::Export(ExportAction::CloseWindow));
    // PSD の書き出しは、先に設定のウィンドウ（方式・チャンネル）を開き、ファイルはそのウィンドウの「書き出し…」で選ぶ
    let at = menu_title(&h, "ファイル").center();
    click(&mut h, at);
    let at = popup_item(&h, "PSD を書き出す…").center();
    click(&mut h, at);
    assert!(h.state().state.psd.options_open);
    assert_eq!(h.state().state.dialog_request, None);
    apply(&mut h, Action::Psd(PsdAction::CancelExportOptions));
    // 英語
    h.state_mut().state.lang = Lang::En;
    h.run();
    let at = menu_title(&h, "File").center();
    click(&mut h, at);
    popup_item(&h, "Export Textures…");
    h.snapshot("menu_file_english");
    hover(&mut h, pos2(640.0, 400.0));
    let row = popup_item(&h, "Import");
    hover(&mut h, row.center());
    popup_item(&h, "PSD as a New Texture Set…");
    popup_item(&h, "PSD into the Current Texture Set…");
    h.snapshot("menu_file_import_english");
    let at = popup_item(&h, "Export PSD…").center();
    click(&mut h, at);
    assert!(h.state().state.psd.options_open);
    apply(&mut h, Action::Psd(PsdAction::CancelExportOptions));
    let at = menu_title(&h, "File").center();
    click(&mut h, at);
    let at = popup_item(&h, "Export Textures…").center();
    click(&mut h, at);
    assert!(h.state().state.export.window.open);
    apply(&mut h, Action::Export(ExportAction::CloseWindow));
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
        s.bake.outcome = Some((
            "「テクスチャセット 1」のメッシュマップ 5 枚を焼きました。".into(),
            true,
        ));
        s.bake.window.as_mut().unwrap().page = Page::Map(MeshMapKind::AmbientOcclusion);
    }
    h.run();
    shot(&mut h, "bake", "bake_window_baked");
    // 共通の設定の頁の「最後のベイク」は、焼いた場所と注意の行だけ（時間・レイ・テクセル・三角形の数は出さない）
    h.state_mut().state.bake.window.as_mut().unwrap().page = Page::Common;
    h.run();
    for dev in ["テクセル 焼いた", "三角形 焼いた", "（準備 "] {
        assert!(
            h.query_by_label_contains(dev).is_none(),
            "開発用の数の行「{dev}」を出さない"
        );
    }
    assert!(
        !h.query_all_by_label_contains("レイ ").any(|n| {
            n.accesskit_node()
                .label()
                .and_then(|l| l.strip_prefix("レイ ").map(str::to_owned))
                .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()))
        }),
        "レイの数の行を出さない"
    );
    shot(&mut h, "bake", "bake_window_last_bake");
    h.state_mut().state.bake.window.as_mut().unwrap().page =
        Page::Map(MeshMapKind::AmbientOcclusion);
    h.run();
    // ウィンドウの目で別のマップを見る
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
    // モデルが替わった次の 1 フレームで、ウィンドウは入力を別のスレッドで作り始める（このフレームでは作らず、ウィンドウは描かれる）
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
    // ウィンドウを閉じると、作っている最中のものは手放す
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
    // 左下の知らせがキャンバスの隅に重なるので、読む前に消す
    h.state_mut().state.clear_message();
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
    // ウィンドウの取消（1 フレームだけ進める: 取消の旗は立つが、止まった仕事を受けるのは次のフレーム）
    h.get_by_label("取消").click();
    h.step();
    assert!(h.state().state.bake.progress().unwrap().canceling);
    settle(&mut h);
    assert!(h.state().state.sets.current().mesh_maps.is_empty());
    assert!(h.state().state.message.contains("取り消しました"));

    // ウィンドウを閉じても焼き続け、仕事の札から取り消せる
    h.state_mut().state.bake.park_next = true;
    h.get_by_label("チェックしたマップをベイク").click();
    h.run();
    assert!(h.state().state.bake.is_baking());
    h.get_by_label("閉じる").click();
    h.run();
    assert!(h.state().state.bake.window.is_none());
    assert!(
        h.state().state.bake.is_baking(),
        "ウィンドウを閉じてもベイクは続く"
    );
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
    // Esc: 焼いていなければウィンドウを閉じる（ウィンドウの上にポインタがあるとき）
    move_to(&h, egui::pos2(640.0, 400.0));
    h.run();
    key(&h, Key::Escape, egui::Modifiers::NONE);
    h.run();
    assert!(h.state().state.bake.window.is_none());
}

/// ウィンドウの中の、名前のボタンの矩形（同じ名前の部品と区別するため、ウィンドウの中で探す）。
fn bake_button(h: &Harness<'_, YoluApp>, label: &str) -> egui::Rect {
    let win = yolu_app::windows::window_rect(&h.ctx, "bake").expect("ウィンドウ");
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
                ray_query_why: None,
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
                sets: None,
            }),
        )
    };
    export(&mut h);
    settle(&mut h);
    assert_eq!(
        dir.files(),
        ["Texture_Hair_Albedo.png", "Texture_Skin_Albedo.png"]
    );
    // 結果のウィンドウ: 書いた画像の一覧
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
    // 確認のウィンドウの間は、キーの割り当てが働かない
    key(&h, Key::E, egui::Modifiers::NONE);
    h.run();
    assert_eq!(
        h.state().state.tool,
        yolu_app::state::Tool::Brush,
        "確認のウィンドウの間は E で消しゴムにならない"
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
            sets: None,
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

    // 取り込めない PSD（PSB）: 何も変えず、理由のウィンドウ
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
    assert!(h
        .state()
        .state
        .message
        .contains("PSB files cannot be imported"));
}

/// レイヤー ID を持たない 2 レイヤーの PSD を書く（書き手は ID を必ず書くので、lyid のタグを同じ長さの別のタグに書き換える）。
fn psd_without_layer_ids(path: &Path) {
    psd_without_layer_ids_with(path, false)
}

/// `dissolve` のとき、下のレイヤーの合成モードを取り込めない「ディゾルブ」にする（取り込みでは通常になる＝変わる。確認のウィンドウが出る）。
fn psd_without_layer_ids_with(path: &Path, dissolve: bool) {
    use yolu_io::psd::{self, Document, Layer, Limits};
    let layer = |id: i32, name: &str, rgba: [u8; 4]| Layer {
        id,
        name: name.into(),
        width: 8,
        height: 8,
        pixels_rgba: rgba.repeat(64),
        ..Layer::default()
    };
    let doc = Document {
        width: 8,
        height: 8,
        layers: vec![
            layer(2, "上", [0, 0, 255, 255]),
            layer(1, "下", [255, 0, 0, 255]),
        ],
        composite_rgba: None,
    };
    let mut bytes = psd::write(&doc, &Limits::default()).unwrap();
    let mut at = 0;
    while let Some(i) = bytes[at..].windows(4).position(|w| w == b"lyid") {
        bytes[at + i..at + i + 4].copy_from_slice(b"lnsr");
        at += i + 4;
    }
    if dissolve {
        let i = bytes
            .windows(8)
            .position(|w| w == b"8BIMnorm")
            .expect("レイヤーの記録の合成モード");
        bytes[i + 4..i + 8].copy_from_slice(b"diss");
    }
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn importing_a_psd_that_loses_something_lists_it_first_and_imports_only_after_the_yes() {
    let dir = TempDir::new("psd-import-check");
    let path = dir.0.join("Ids.psd");
    psd_without_layer_ids_with(&path, true);
    let mut h = app(1280.0, 800.0, 32);
    let import = |h: &mut Harness<'_, YoluApp>| {
        apply(
            h,
            Action::Psd(PsdAction::Import {
                path: path.clone(),
                target: PsdTarget::NewSet,
            }),
        );
        settle(h);
    };
    import(&mut h);
    assert!(h.state().state.psd.import_check.is_some());
    assert_eq!(h.state().state.sets.len(), 1, "確かめるまで入れない");
    let texts = window_texts(&h, "psd-import");
    for want in [
        "レイヤー ID の欠落・重複",
        "レイヤーのメタデータ",
        "変わる",
        "無視",
        "下、上",
    ] {
        assert!(texts.iter().any(|t| t == want), "{want}: {texts:?}");
    }
    // PSD の内部のタグ名（4 文字のキー）は画面に出さない
    assert!(
        texts
            .iter()
            .all(|t| !t.contains("lnsr") && !t.contains("lyid")),
        "{texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|t| t.contains("Ids.psd") && t.contains("変わる 1") && t.contains("無視 2")),
        "{texts:?}"
    );
    shot(&mut h, "psd-import", "psd_import_check");
    // やめる: 何も入れない
    h.get_by_label("やめる").click();
    h.run();
    assert!(h.state().state.psd.import_check.is_none());
    assert_eq!(h.state().state.sets.len(), 1);
    // 英語で、もう一度読んで取り込む
    h.state_mut().state.lang = Lang::En;
    import(&mut h);
    let texts = window_texts(&h, "psd-import");
    for want in [
        "Missing or duplicate layer IDs",
        "Layer metadata",
        "Changed",
        "Ignored",
    ] {
        assert!(texts.iter().any(|t| t == want), "{want}: {texts:?}");
    }
    assert!(
        texts
            .iter()
            .all(|t| !t.contains("lnsr") && !t.contains("lyid")),
        "{texts:?}"
    );
    // 日本語が残ってよいのは、利用者の名前（レイヤーの名前）だけ
    assert!(
        texts
            .iter()
            .all(|t| !has_japanese(t) || t == "下, 上" || t == "下"),
        "{texts:?}"
    );
    shot(&mut h, "psd-import", "psd_import_check_english");
    h.get_by_label("Import").click();
    h.run();
    assert!(h.state().state.psd.import_check.is_none());
    assert_eq!(h.state().state.sets.len(), 2);
    assert_eq!(h.state().state.sets.current().name, "Ids");
}

/// ウィンドウの中に描いた文字（描いた順）。
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

/// ウィンドウの中の、名前の部品の中心（同じ名前のドックの部品に取り違えない）。
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
    // 日本語が残ってよいのは、利用者の名前（レイヤーの名前）だけ
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
    // 書き出し…: ウィンドウを閉じて、書き出す先を選ぶウィンドウを頼む。初めのファイル名は 1 つのチャンネルなら末尾にチャンネル
    let name = yolu_app::psd::default_export_name(&h.state().state);
    assert!(name.ends_with("_Roughness.psd"), "{name}");
    let at = in_window(&h, "psd-export", "書き出し…");
    click(&mut h, at);
    assert!(!h.state().state.psd.options_open);
    assert_eq!(
        h.state().state.dialog_request,
        Some(DialogRequest::PsdExport)
    );
    // 英語: ウィンドウの文字に日本語が残らない。やめるで閉じる
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

// ───────── 書き出しのウィンドウ ─────────

/// 書き出しのウィンドウの出力テンプレートのドロップダウンを開いて、出力テンプレートを選ぶ。
fn pick_form(h: &mut Harness<'_, YoluApp>, from: &str, to: &str) {
    let at = in_window(
        h,
        "export",
        &format!(
            "{}: {from}",
            h_label(h, "出力テンプレート", "Output Template")
        ),
    );
    click(h, at);
    let at = popup_item(h, to).center();
    click(h, at);
}

/// 画面の言語に合う名前。
fn h_label<'a>(h: &Harness<'_, YoluApp>, ja: &'a str, en: &'a str) -> &'a str {
    if h.state().state.lang == Lang::En {
        en
    } else {
        ja
    }
}

/// 書き出し先を決めて、書き出しのウィンドウの絵を撮る（書き出す先は毎回違う道にならないよう、決まった道を置く）。
fn shot_form(h: &mut Harness<'_, YoluApp>, name: &str) {
    let form = h.state().state.export_form();
    let chosen = if form.writes_file() {
        PathBuf::from("/out/Texture_Color.png")
    } else {
        PathBuf::from("/out/Textures")
    };
    apply(h, Action::Export(ExportAction::Destination(chosen)));
    shot(h, "export", name);
}

#[test]
fn the_export_window_picks_each_template_and_remembers_it_in_both_languages() {
    let mut h = two_sets_app();
    let at = menu_title(&h, "ファイル").center();
    click(&mut h, at);
    let at = popup_item(&h, "テクスチャを書き出す…").center();
    click(&mut h, at);
    assert!(h.state().state.export.window.open);
    // 初めの出力テンプレートは「今のチャンネル」
    assert_eq!(
        h.state().state.export_form(),
        yolu_app::export::ExportForm::ChannelPng
    );
    let texts = window_texts(&h, "export");
    for want in [
        "テクスチャを書き出す",
        "テクスチャセット",
        "出力先",
        "選ぶ…",
        "出力テンプレート",
        "今のチャンネル",
        "パディング",
        "無限に広げる",
        "書き出すファイル",
        "キャンセル",
        "書き出す",
    ] {
        assert!(texts.iter().any(|t| t == want), "{want}: {texts:?}");
    }
    for text in &texts {
        assert_plain("書き出しのウィンドウ", text);
    }
    // 「形」「余白」「書き出す先」の言葉は画面に出さない
    for old in ["形", "余白", "書き出す先"] {
        assert!(texts.iter().all(|t| !t.contains(old)), "{old}: {texts:?}");
    }
    // 出力テンプレートごとの絵（日本語）
    let forms = [
        ("今のチャンネル", "export_window_channel"),
        ("チャンネルごと", "export_window_channels"),
        ("Unity Standard / URP Lit", "export_window_unity_standard"),
        ("HDRP Lit", "export_window_hdrp"),
        ("lilToon", "export_window_liltoon"),
    ];
    let mut current = forms[0].0;
    shot_form(&mut h, forms[0].1);
    for (label, name) in &forms[1..] {
        pick_form(&mut h, current, label);
        current = label;
        assert_eq!(h.state().state.export_form().name(Lang::Ja), *label);
        shot_form(&mut h, name);
    }
    // 出力テンプレートは設定に覚える（保存する設定にも入る）
    assert_eq!(
        h.state().state.settings().export_form,
        yolu_app::export::ExportForm::LilToon
    );
    // 閉じて開き直しても、同じ出力テンプレート
    apply(&mut h, Action::Export(ExportAction::CloseWindow));
    assert!(
        yolu_app::windows::window_rect(&h.ctx, "export").is_none()
            || !h.state().state.export.window.open
    );
    apply(&mut h, Action::Export(ExportAction::OpenWindow));
    assert_eq!(h.state().state.export_form().name(Lang::Ja), "lilToon");
    // 英語: 文字に日本語が残らず、出力テンプレートごとの絵
    h.state_mut().state.lang = Lang::En;
    h.run();
    let texts = window_texts(&h, "export");
    for want in [
        "Export Textures",
        "Texture Sets",
        "Output Path",
        "Choose…",
        "Output Template",
        "lilToon",
        "Padding",
        "Dilation infinite",
        "Files to Export",
        "Cancel",
        "Export",
    ] {
        assert!(texts.iter().any(|t| t == want), "{want}: {texts:?}");
    }
    assert!(texts.iter().all(|t| !has_japanese(t)), "{texts:?}");
    for text in &texts {
        assert_plain("export window", text);
    }
    for old in ["Type", "Destination"] {
        assert!(texts.iter().all(|t| !t.contains(old)), "{old}: {texts:?}");
    }
    let english = [
        "Current Channel",
        "Per Channel",
        "Unity Standard / URP Lit",
        "HDRP Lit",
        "lilToon",
    ];
    let names = [
        "export_window_channel_english",
        "export_window_channels_english",
        "export_window_unity_standard_english",
        "export_window_hdrp_english",
        "export_window_liltoon_english",
    ];
    let mut current = english[4];
    for (label, name) in english.iter().zip(names) {
        if *label != current {
            pick_form(&mut h, current, label);
            current = label;
        }
        shot_form(&mut h, name);
    }
}

#[test]
fn the_export_window_chooses_the_output_path_and_the_padding_and_closes_with_the_buttons() {
    let mut h = two_sets_app();
    apply(&mut h, Action::Export(ExportAction::OpenWindow));
    // 出力先が決まっていない間（保存していないプロジェクト）は書き出せない
    let button = h.get_by_label("書き出す");
    assert!(
        button.accesskit_node().is_disabled(),
        "先が無い間は押せない"
    );
    // 「選ぶ…」は出力先を選ぶウィンドウ（OS）を頼むだけ
    let at = in_window(&h, "export", "選ぶ…");
    click(&mut h, at);
    assert_eq!(
        h.state().state.dialog_request,
        Some(DialogRequest::ExportDestination)
    );
    h.state_mut().state.dialog_request = None;
    // パディング: 設定の「書き出しのパディング」と同じ値を、ここから替えられる
    assert_eq!(h.state().state.export.padding, -1);
    let at = in_window(&h, "export", "パディング: 無限に広げる");
    click(&mut h, at);
    let at = popup_item(&h, "8 px 広げる").center();
    click(&mut h, at);
    assert_eq!(h.state().state.export.padding, 8);
    assert_eq!(h.state().state.settings().export_padding, 8);
    in_window(&h, "export", "パディング: 8 px 広げる");
    // キャンセル・閉じる・Esc で閉じる
    let at = in_window(&h, "export", "キャンセル");
    click(&mut h, at);
    assert!(!h.state().state.export.window.open);
    apply(&mut h, Action::Export(ExportAction::OpenWindow));
    let at = in_window(&h, "export", "ウィンドウを閉じる");
    click(&mut h, at);
    assert!(!h.state().state.export.window.open);
    apply(&mut h, Action::Export(ExportAction::OpenWindow));
    let rect = yolu_app::windows::window_rect(&h.ctx, "export").unwrap();
    h.event(egui::Event::PointerMoved(rect.center()));
    h.step();
    key(&h, Key::Escape, egui::Modifiers::NONE);
    h.run();
    assert!(!h.state().state.export.window.open, "Esc で閉じる");
}

#[test]
fn the_export_window_writes_what_the_menu_items_wrote_and_asks_before_replacing() {
    let dir = TempDir::new("export-window");
    let mut h = two_sets_app();
    apply(&mut h, Action::Export(ExportAction::OpenWindow));
    pick_form(&mut h, "今のチャンネル", "Unity Standard / URP Lit");
    // 先を選ぶと書き出せる
    apply(
        &mut h,
        Action::Export(ExportAction::Destination(dir.0.clone())),
    );
    let at = in_window(&h, "export", "書き出す");
    click(&mut h, at);
    assert!(h.state().state.export.is_exporting());
    assert!(!h.state().state.export.window.open, "書き始めたら閉じる");
    settle(&mut h);
    assert_eq!(
        dir.files(),
        ["Texture_Hair_Albedo.png", "Texture_Skin_Albedo.png"]
    );
    assert!(h.state().state.export.report.is_some(), "結果のウィンドウ");
    h.get_by_label("閉じる").click();
    h.run();
    // もう一度: もうあるファイルを確かめる。やめるとウィンドウに戻り、置き換えると書いて閉じる
    let before = std::fs::read(dir.0.join("Texture_Skin_Albedo.png")).unwrap();
    {
        let s = &mut h.state_mut().state;
        let layer = s.selected_layer.unwrap();
        s.doc.set_layer_opacity(layer, 0.5, false).unwrap();
    }
    apply(&mut h, Action::Export(ExportAction::OpenWindow));
    let at = in_window(&h, "export", "書き出す");
    click(&mut h, at);
    assert!(h.state().state.export.confirm.is_some());
    assert!(!h.state().state.export.is_exporting());
    let at = in_window(&h, "export-confirm", "やめる");
    click(&mut h, at);
    assert!(h.state().state.export.confirm.is_none());
    assert!(
        h.state().state.export.window.open,
        "やめたらウィンドウに戻る"
    );
    assert_eq!(
        std::fs::read(dir.0.join("Texture_Skin_Albedo.png")).unwrap(),
        before
    );
    let at = in_window(&h, "export", "書き出す");
    click(&mut h, at);
    h.get_by_label("置き換える").click();
    h.run();
    assert!(!h.state().state.export.window.open);
    settle(&mut h);
    assert_ne!(
        std::fs::read(dir.0.join("Texture_Skin_Albedo.png")).unwrap(),
        before,
        "置き換えた"
    );
}

#[test]
fn the_export_window_is_a_command_without_a_default_key_that_the_user_can_assign() {
    use yolu_app::keymap::Scope;
    let mut h = app(1280.0, 800.0, 64);
    // Ctrl+Shift+E は表示レイヤーの結合が使っているので、既定のキーは付けない。操作は一覧にあり、割り当てを選べる
    let command = yolu_app::commands::find("file.export").expect("操作の一覧にある");
    assert_eq!(
        command.runnable(),
        Some(Action::Export(ExportAction::OpenWindow))
    );
    assert_eq!(yolu_app::shortcuts::menu_key("file.export"), None);
    let group: yolu_app::keyconfig::GroupKey = ("file.export", Scope::Everywhere);
    assert!(
        yolu_app::shortcuts::editor::all_groups(&h.state().state).contains(&group),
        "ショートカットの設定の一覧に出る"
    );
    // 空いているキーを割り当てると、そのキーでウィンドウが開く
    let trigger = yolu_app::keyconfig::parse_trigger("Ctrl+Alt+E").unwrap();
    h.state_mut().state.keys.set(group, vec![trigger]);
    h.state_mut().state.keys_changed();
    h.run();
    key(
        &h,
        Key::E,
        egui::Modifiers {
            alt: true,
            ..egui::Modifiers::COMMAND
        },
    );
    h.run();
    assert!(h.state().state.export.window.open);
    // メニューの項目にも、割り当てたキーが出る
    apply(&mut h, Action::Export(ExportAction::CloseWindow));
    let at = menu_title(&h, "ファイル").center();
    click(&mut h, at);
    popup_item(&h, "テクスチャを書き出す…");
    assert!(
        yolu_app::shortcuts::menu_key("file.export").is_some(),
        "割り当てたキーが、メニューの項目の右に出る"
    );
}

/// 置き換えの確認のウィンドウ（モーダル）が前に出ている間、はみ出して見えている下の書き出しのウィンドウは押しを受けない
/// （書き出しのウィンドウもモーダルにすると、確認のウィンドウの外の押しが下のウィンドウへ通り、閉じてしまう）。
#[test]
fn the_export_window_below_the_replace_confirm_does_not_take_clicks() {
    let dir = TempDir::new("below-confirm");
    let mut h = two_sets_app();
    apply(&mut h, Action::Export(ExportAction::OpenWindow));
    pick_form(&mut h, "今のチャンネル", "Unity Standard / URP Lit");
    apply(
        &mut h,
        Action::Export(ExportAction::Destination(dir.0.clone())),
    );
    let at = in_window(&h, "export", "書き出す");
    click(&mut h, at);
    settle(&mut h);
    h.get_by_label("閉じる").click();
    h.run();
    // 2 度目: もうあるファイルの確認が出る。書き出しのウィンドウを横へ動かし、確認のウィンドウからはみ出させる
    apply(&mut h, Action::Export(ExportAction::OpenWindow));
    h.state_mut().state.export.window.offset = egui::vec2(-420.0, 0.0);
    h.run();
    h.step();
    let at = in_window(&h, "export", "書き出す");
    click(&mut h, at);
    assert!(h.state().state.export.confirm.is_some());
    h.run();
    let confirm = yolu_app::windows::window_rect(&h.ctx, "export-confirm").unwrap();
    let target = in_window(&h, "export", "キャンセル");
    assert!(!confirm.contains(target), "確認のウィンドウの外にある");
    click(&mut h, target);
    assert!(
        h.state().state.export.window.open,
        "確認のウィンドウの間は、下のウィンドウの「キャンセル」を押しても閉じない"
    );
    assert!(h.state().state.export.confirm.is_some());
}

/// 置き換えの確認が前にあるときの Esc は、確認だけを閉じる（書き出しのウィンドウは開いたまま、確認をやめたあとのここへ戻れる）。
#[test]
fn escape_closes_only_the_replace_confirm_and_leaves_the_export_window() {
    let dir = TempDir::new("esc-confirm");
    let mut h = two_sets_app();
    apply(&mut h, Action::Export(ExportAction::OpenWindow));
    pick_form(&mut h, "今のチャンネル", "Unity Standard / URP Lit");
    apply(
        &mut h,
        Action::Export(ExportAction::Destination(dir.0.clone())),
    );
    let at = in_window(&h, "export", "書き出す");
    click(&mut h, at);
    settle(&mut h);
    h.get_by_label("閉じる").click();
    h.run();
    apply(&mut h, Action::Export(ExportAction::OpenWindow));
    let at = in_window(&h, "export", "書き出す");
    click(&mut h, at);
    assert!(h.state().state.export.confirm.is_some());
    // ポインタは書き出しのウィンドウの上にある（ウィンドウだけが Esc を受ける位置）
    let rect = yolu_app::windows::window_rect(&h.ctx, "export").unwrap();
    h.event(egui::Event::PointerMoved(rect.center()));
    h.step();
    key(&h, Key::Escape, egui::Modifiers::NONE);
    h.run();
    assert!(h.state().state.export.confirm.is_none(), "確認が閉じる");
    assert!(
        h.state().state.export.window.open,
        "書き出しのウィンドウは開いたまま"
    );
    // 確認が無くなれば、次の Esc で書き出しのウィンドウが閉じる
    key(&h, Key::Escape, egui::Modifiers::NONE);
    h.run();
    assert!(!h.state().state.export.window.open);
}

/// 3 つのセット（Skin・Hair・Accessory。Skin は Roughness も、Hair は Emission も使う）を持つ画面。書くファイルが色空間の違う物にもなる。
fn three_sets_app() -> Harness<'static, YoluApp> {
    let mut h = two_sets_app();
    {
        let s = &mut h.state_mut().state;
        let hair = 1 - s.sets.current_index();
        let layer = s.selected_layer.unwrap();
        s.doc
            .set_channel_enabled(layer, Channel::Roughness, true)
            .unwrap();
        let doc = s.set_doc_mut(hair);
        let hair_layer = doc.layers()[0].id();
        doc.set_channel_enabled(hair_layer, Channel::Emission, true)
            .unwrap();
        let uid = s.add_texture_set().unwrap();
        s.rename_set(uid, "Accessory").unwrap();
    }
    h.run();
    h
}

/// 書き出しのウィンドウの、テクスチャセットのチェック（役割で探す。同じ名前のドックの部品に取り違えない）。
fn set_check<'a>(h: &'a Harness<'_, YoluApp>, name: &'a str) -> egui_kittest::Node<'a> {
    let area = yolu_app::windows::window_rect(&h.ctx, "export").expect("書き出しのウィンドウ");
    h.get_all_by_role_and_label(egui::accesskit::Role::CheckBox, name)
        .find(|n| area.contains_rect(n.rect()))
        .unwrap_or_else(|| panic!("{name}: ウィンドウの中のチェックが無い"))
}

fn export_button_disabled(h: &Harness<'_, YoluApp>) -> bool {
    let area = yolu_app::windows::window_rect(&h.ctx, "export").unwrap();
    let label = h_label(h, "書き出す", "Export");
    h.get_all_by_label(label)
        .find(|n| area.contains_rect(n.rect()))
        .expect("書き出すボタン")
        .accesskit_node()
        .is_disabled()
}

#[test]
fn the_export_window_checks_sets_and_lists_the_files_it_will_write() {
    let dir = TempDir::new("export-sets");
    let mut h = three_sets_app();
    apply(&mut h, Action::Export(ExportAction::OpenWindow));
    pick_form(&mut h, "今のチャンネル", "Unity Standard / URP Lit");
    apply(
        &mut h,
        Action::Export(ExportAction::Destination(dir.0.clone())),
    );
    let uids: Vec<u32> = (0..3)
        .map(|i| h.state().state.sets.get(i).unwrap().uid)
        .collect();
    // 3 つのセットがチェック済みで並び、書くファイルが、名前・色空間の順に出る
    let texts = window_texts(&h, "export");
    for name in ["Skin", "Hair", "Accessory"] {
        assert!(texts.iter().any(|t| t == name), "{name}: {texts:?}");
        assert!(!set_check(&h, name).accesskit_node().is_disabled());
    }
    for file in [
        "Texture_Skin_Albedo.png",
        "Texture_Skin_MetallicSmoothness.png",
        "Texture_Hair_Albedo.png",
        "Texture_Hair_Emission.png",
        "Texture_Accessory_Albedo.png",
    ] {
        assert!(texts.iter().any(|t| t == file), "{file}: {texts:?}");
    }
    assert!(texts.iter().any(|t| t == "sRGB") && texts.iter().any(|t| t == "リニア"));
    assert!(!export_button_disabled(&h));
    // Hair を外すと、その名前のファイルは一覧から消え、ほかは残る
    set_check(&h, "Hair").click();
    h.run();
    assert_eq!(h.state().state.export_checked_uids(), [uids[0], uids[2]]);
    let texts = window_texts(&h, "export");
    assert!(!texts.iter().any(|t| t.contains("_Hair_")), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Texture_Skin_Albedo.png"));
    // 3 つのセットで 1 つ外した形の絵（日英）
    apply(
        &mut h,
        Action::Export(ExportAction::SetChecked {
            uid: uids[2],
            on: false,
        }),
    );
    apply(
        &mut h,
        Action::Export(ExportAction::SetChecked {
            uid: uids[1],
            on: true,
        }),
    );
    apply(
        &mut h,
        Action::Export(ExportAction::Destination(PathBuf::from("/out/Textures"))),
    );
    shot(&mut h, "export", "export_window_sets_unchecked");
    h.state_mut().state.lang = Lang::En;
    h.run();
    shot(&mut h, "export", "export_window_sets_unchecked_english");
    h.state_mut().state.lang = Lang::Ja;
    h.run();
    apply(
        &mut h,
        Action::Export(ExportAction::Destination(dir.0.clone())),
    );
    apply(
        &mut h,
        Action::Export(ExportAction::SetChecked {
            uid: uids[2],
            on: true,
        }),
    );
    // 1 つも入っていないと「書き出す」は押せない（一覧も空）
    assert_eq!(h.state().state.export_checked_uids(), uids);
    for name in ["Skin", "Hair", "Accessory"] {
        set_check(&h, name).click();
        h.run();
    }
    assert!(h.state().state.export_checked_uids().is_empty());
    assert!(export_button_disabled(&h));
    assert!(!window_texts(&h, "export")
        .iter()
        .any(|t| t.ends_with(".png")));
    // 1 つ入れ直して、ボタンから書くと、外したセットは書かない
    set_check(&h, "Skin").click();
    h.run();
    set_check(&h, "Accessory").click();
    h.run();
    assert!(!export_button_disabled(&h));
    let at = in_window(&h, "export", "書き出す");
    click(&mut h, at);
    assert!(h.state().state.export.is_exporting());
    settle(&mut h);
    assert_eq!(
        dir.files(),
        [
            "Texture_Accessory_Albedo.png",
            "Texture_Accessory_MetallicSmoothness.png",
            "Texture_Skin_Albedo.png",
            "Texture_Skin_MetallicSmoothness.png"
        ]
    );
}

#[test]
fn the_export_window_cannot_uncheck_the_one_set_of_the_current_channel_png() {
    let mut h = three_sets_app();
    apply(&mut h, Action::Export(ExportAction::OpenWindow));
    assert_eq!(
        h.state().state.export_form(),
        yolu_app::export::ExportForm::ChannelPng
    );
    // 今のチャンネルの 1 枚は、今のセット（Accessory）だけが並び、触れない
    let texts = window_texts(&h, "export");
    assert!(texts.iter().any(|t| t == "Accessory"), "{texts:?}");
    assert!(
        !texts.iter().any(|t| t == "Skin" || t == "Hair"),
        "{texts:?}"
    );
    let node = set_check(&h, "Accessory");
    assert!(node.accesskit_node().is_disabled());
    node.click();
    h.run();
    assert_eq!(h.state().state.export_checked_uids().len(), 1);
    // ほかのテンプレートでは、同じセットも触れる
    pick_form(&mut h, "今のチャンネル", "チャンネルごと");
    assert!(!set_check(&h, "Accessory").accesskit_node().is_disabled());
    assert!(!set_check(&h, "Skin").accesskit_node().is_disabled());
}

#[test]
fn the_export_window_fits_the_smallest_screen_and_scrolls_a_long_list_of_sets() {
    let mut h = app(960.0, 640.0, 64);
    for i in 0..24 {
        let uid = h.state_mut().state.add_texture_set().unwrap();
        h.state_mut()
            .state
            .rename_set(uid, &format!("Set{i:02}"))
            .unwrap();
    }
    h.run();
    apply(&mut h, Action::Export(ExportAction::OpenWindow));
    pick_form(&mut h, "今のチャンネル", "チャンネルごと");
    let rect = yolu_app::windows::window_rect(&h.ctx, "export").unwrap();
    let screen = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(960.0, 640.0));
    assert!(screen.contains_rect(rect), "{rect:?} は画面からはみ出す");
    // 左の一覧は欄の中に収まる行だけを描き、ボタンは下の帯の中にある
    let button = in_window(&h, "export", "書き出す");
    assert!(rect.contains(button));
    let first = set_check(&h, "Set00").rect();
    assert!(rect.contains_rect(first));
    assert!(
        h.query_all_by_label("Set23").next().is_none(),
        "欄に入らない行は描かない"
    );
    // ホイールで送ると、最後の行に届く
    for _ in 0..6 {
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -80.0),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        });
        move_to(&h, pos2(rect.left() + 100.0, rect.top() + 150.0));
        h.run();
    }
    let last = set_check(&h, "Set23").rect();
    assert!(rect.contains_rect(last), "{last:?} / {rect:?}");
    // 最後のセットも外せる
    set_check(&h, "Set23").click();
    h.run();
    assert!(!h
        .state()
        .state
        .export_checked_uids()
        .contains(&h.state().state.sets.get(24).unwrap().uid));
    // 書くファイルの一覧も、欄の中で送れる（行の数が欄を超えても、はみ出さない）
    assert!(
        h.state_mut().state.export_preview().files.len() > 13,
        "欄の行数を超える一覧"
    );
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
    assert_eq!(
        (again.bake.settings.padding, again.bake.settings.ao_samples),
        (4, 8)
    );
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
    // ウィンドウ（harness）は作らないが、確認とベイクが製品のスレッドで GPU の装置を作る。ウィンドウを持つ試験と同時に作らないよう、先に貸し出しを取る
    // （確認のループと `wait_bake` で製品のスレッドはこの試験の中で終わるので、試験のスレッドが持てば足りる）
    common::gpu_thread::lease();
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
        // CI の画面の試験のジョブは YOLUPAINTER_REQUIRE_GPU を付け、アダプターを取れないときに通った扱いにしない（common::canvas_device と同じ）
        assert!(
            std::env::var_os("YOLUPAINTER_REQUIRE_GPU").is_none(),
            "YOLUPAINTER_REQUIRE_GPU があるのに GPU を使えない: {why}"
        );
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
    s.receive_link_model(&two_quads()).1.unwrap();
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

/// 写しとして取り込んだ PSD（レイヤー ID の欠落に新しい ID を振ったもの）も、.ylp に保存して開き直しても同じ絵・同じレイヤー・同じ ID で戻り、
/// 書き出しは新しい PSD になる（取り込んだ PSD は 1 バイトも変わらない。同じファイルへ書くときは置き換える前に確かめる）。
#[test]
fn headless_a_copy_imported_psd_survives_save_and_reopen_and_never_rewrites_the_original() {
    let dir = TempDir::new("hcopy");
    let psd = dir.0.join("Copy.psd");
    psd_without_layer_ids(&psd);
    let original = std::fs::read(&psd).unwrap();
    let mut s = AppState::new(32, 32);
    s.bake.backend = yolu_app::bake::BakeBackend::Cpu;
    s.apply(Action::Psd(PsdAction::Import {
        path: psd.clone(),
        target: PsdTarget::NewSet,
    }));
    s.wait_psd();
    assert!(
        s.psd.import_check.is_none(),
        "レイヤー ID の欠落は無視（確かめずに入る）"
    );
    assert_eq!(s.sets.len(), 2);
    let composite = |s: &AppState| s.doc.composite(s.doc.bounds()).unwrap();
    let before = composite(&s);
    let layers = |s: &AppState| -> Vec<(String, u128)> {
        s.doc
            .layers()
            .iter()
            .map(|l| (l.name().to_owned(), l.id().0))
            .collect()
    };
    let ids = layers(&s);
    assert!(ids.iter().all(|(_, id)| id >> 96 > 0));
    let project = dir.0.join("copy.ylp");
    s.apply(Action::SaveProjectAs(project.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);

    let mut again = AppState::new(32, 32);
    again.bake.backend = yolu_app::bake::BakeBackend::Cpu;
    again.apply(Action::OpenProject(project));
    let index = again
        .sets
        .iter()
        .position(|x| x.name == "Copy")
        .expect("セットの名前");
    again.switch_set(index).unwrap();
    assert!(composite(&again) == before, "同じ絵");
    assert_eq!(layers(&again), ids, "同じレイヤー・同じ ID");
    // 書き出しは別のファイルへ新しい PSD として。取り込んだ PSD は変わらない
    let out = dir.0.join("out.psd");
    again.apply(Action::Psd(PsdAction::Export(out.clone())));
    again.wait_psd();
    assert!(out.exists(), "{}", again.message);
    assert_eq!(
        std::fs::read(&psd).unwrap(),
        original,
        "取り込んだ PSD は書き換えない"
    );
    // 取り込んだ文書から、取り込んだファイルと同じ場所へ書き出そうとすると、置き換える前に確かめる
    s.apply(Action::Psd(PsdAction::Export(psd.clone())));
    assert!(
        s.psd.confirm.as_ref().is_some_and(|c| c.imported),
        "{}",
        s.message
    );
    assert_eq!(
        std::fs::read(&psd).unwrap(),
        original,
        "確かめるまで書かない"
    );
}
