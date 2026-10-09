//! 新規プロジェクトのウィンドウ（Ctrl+N）とプロジェクトの構成（ファイル ▸ プロジェクト設定）: ウィンドウの操作（egui_kittest）、作ったプロジェクトの中身
//! （セットの数・大きさ・チャンネル・法線の形式・マテリアルの鍵）、構成の変更（追加・削除・名前・大きさ・モデルの差し替えと読み直し）、
//! 保存と開き直し（モデルのファイルの参照）、取消、日英。
use crate::common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::fbx::{ascii_fbx, mesh, temp_dir, write_fbx};
use common::{app, click, has_japanese, key, menu_title, popup_item, rect_of};
use egui::{Key, Modifiers, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::engine::{
    CanvasResampling, Channel, Document, NormalYDirection, Rgba8, SelectionMask,
};
use yolu_app::lang::Lang;
use yolu_app::newproject::{DraftOp, NpAction, Prep, Template};
use yolu_app::sets::MaterialRef;
use yolu_app::state::{Action, AppState, DialogRequest};
use yolu_app::YoluApp;

type S = AppState;

fn np(s: &mut S, a: NpAction) {
    s.apply(Action::Project(a));
}

/// ウィンドウのモデルの読み込みが終わるまで待つ（試験が止まらないよう、上限つき）。
fn wait_model(s: &mut S) {
    let start = Instant::now();
    loop {
        s.poll_newproject();
        match s.np.window.as_ref() {
            Some(w) if w.is_loading() => {}
            _ => return,
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "モデルの準備が終わらない"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// .ylp を開いたあとの、モデルの読み込みが終わるまで待つ。
fn wait_reopen(s: &mut S) {
    let start = Instant::now();
    while s.np.reopening.is_some() {
        s.poll_newproject();
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "モデルの読み込みが終わらない"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// 3 つのマテリアル（Skin・Hair・Eye）と 3 つのメッシュ（Body と Face は Skin を共有、Hair、Eye）の FBX。
fn character(dir: &Path, name: &str) -> PathBuf {
    let text = ascii_fbx(
        &["Skin", "Hair", "Eye"],
        &[
            mesh("Body", &[Some(0), Some(0)]),
            mesh("Face", &[Some(0)]),
            mesh("HairMesh", &[Some(1), Some(1)]),
            mesh("EyeMesh", &[Some(2)]),
        ],
    );
    write_fbx(dir, name, &text)
}

fn set_names(s: &S) -> Vec<String> {
    s.sets.iter().map(|x| x.name.clone()).collect()
}

fn size_of(s: &S, index: usize) -> (u32, u32) {
    let d = s.set_doc(index);
    (d.width(), d.height())
}

fn enabled_channels(doc: &Document) -> Vec<Channel> {
    let mut v: Vec<Channel> = doc
        .layers()
        .iter()
        .flat_map(|l| l.enabled_channels())
        .collect();
    v.sort();
    v.dedup();
    v
}

/// ウィンドウを開き、モデルを選んで準備を終える。
fn open_new_with(s: &mut S, path: &Path) {
    np(s, NpAction::OpenNew);
    np(s, NpAction::ChooseModel(path.to_path_buf()));
    wait_model(s);
    assert!(
        matches!(s.np.window.as_ref().unwrap().prep, Prep::Ready { .. }),
        "モデルを準備できなかった: {:?}",
        s.np.window.as_ref().map(|w| w.error.clone())
    );
}

#[test]
fn a_project_without_a_model_follows_the_template_resolution_and_normal_format() {
    let mut s = S::new(64, 64);
    s.modified = true;
    np(&mut s, NpAction::OpenNew);
    np(&mut s, NpAction::Template(Template::ColorOnly));
    np(&mut s, NpAction::Resolution(1024));
    np(&mut s, NpAction::Normal(NormalYDirection::DirectX));
    np(&mut s, NpAction::Submit);
    assert!(s.np.window.is_none(), "作ったらウィンドウは閉じる");
    assert_eq!(s.sets.len(), 1);
    assert_eq!(size_of(&s, 0), (1024, 1024));
    assert_eq!(enabled_channels(&s.doc), [Channel::Color]);
    assert_eq!(
        s.doc.normal_settings().file_direction(),
        NormalYDirection::DirectX
    );
    assert_eq!(
        s.sets.current().material,
        MaterialRef::PendingSlot(0),
        "モデルが無ければ仮のスロット 0"
    );
    assert!(s.sets.current().bound.is_none());
    assert!(
        s.project.is_none() && !s.modified,
        "保存していない新しいプロジェクト"
    );
    assert_eq!(s.project_name, "名称未設定");
    assert!(
        !s.doc.can_undo(),
        "最初のレイヤーを足したことは取り消せない"
    );
    assert_eq!(s.doc.layers().len(), 1);
    assert!(s.model.is_none() && s.np.model_file.is_none());
    assert!(s.message.contains("モデルなし"), "{}", s.message);
}

#[test]
fn the_templates_enable_the_same_channels_as_the_unity_window() {
    for (template, expected) in [
        (
            Template::Pbr,
            vec![
                Channel::Color,
                Channel::Roughness,
                Channel::Metallic,
                Channel::Height,
                Channel::Normal,
                Channel::Emission,
            ],
        ),
        (
            Template::LilToon,
            vec![Channel::Color, Channel::Normal, Channel::Emission],
        ),
        (Template::ColorOnly, vec![Channel::Color]),
    ] {
        let mut s = S::new(64, 64);
        np(&mut s, NpAction::OpenNew);
        np(&mut s, NpAction::Template(template));
        np(&mut s, NpAction::Resolution(512));
        np(&mut s, NpAction::Submit);
        assert_eq!(enabled_channels(&s.doc), expected, "{template:?}");
        assert_eq!(template.channels(), expected.as_slice());
    }
}

#[test]
fn a_model_gives_one_set_per_chosen_material_and_meshes_sharing_a_material_share_a_set() {
    let dir = temp_dir("create");
    let path = character(&dir, "character.fbx");
    let mut s = S::new(64, 64);
    open_new_with(&mut s, &path);
    {
        let win = s.np.window.as_ref().unwrap();
        let groups = win.groups(&s);
        assert_eq!(
            groups.len(),
            3,
            "マテリアルは 3 つ（Body と Face は同じ Skin）"
        );
        assert_eq!(groups[0].name, "Skin");
        assert_eq!(
            groups[0].meshes,
            ["Body", "Face"],
            "同じマテリアルを使うメッシュの名前を添える"
        );
        assert_eq!(groups[1].meshes, ["HairMesh"]);
    }
    // Eye を外す
    np(&mut s, NpAction::Material(2, false));
    np(&mut s, NpAction::Resolution(512));
    np(&mut s, NpAction::Normal(NormalYDirection::DirectX));
    np(&mut s, NpAction::Submit);
    assert!(s.np.window.is_none());
    assert_eq!(set_names(&s), ["Skin", "Hair"]);
    for i in 0..2 {
        assert_eq!(size_of(&s, i), (512, 512));
        assert_eq!(
            s.set_doc(i).normal_settings().file_direction(),
            NormalYDirection::DirectX
        );
        assert_eq!(enabled_channels(s.set_doc(i)).len(), 6, "PBR");
    }
    assert_eq!(
        s.sets.get(0).unwrap().material,
        MaterialRef::Material {
            name: "Skin".into(),
            asset: None
        }
    );
    assert_eq!(s.sets.get(0).unwrap().bound, Some(0));
    assert_eq!(s.sets.get(1).unwrap().bound, Some(1));
    assert!(
        s.sets.iter().all(|x| !x.saved.is_some()),
        "ファイルに無いセット"
    );
    assert!(s.view3d.model.is_some(), "モデルは 3D ビューに入る");
    assert!(s
        .model
        .as_ref()
        .is_some_and(|m| matches!(m.source, yolu_app::model::ModelSource::Rig { .. })));
    assert_eq!(s.np.model_file.as_deref(), Some(path.as_path()));
    assert_eq!(s.sets.current_index(), 0);
    // 選ばなかったマテリアルに、あとからセットを作らない
    s.poll_newproject();
    assert_eq!(s.sets.len(), 2);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn at_least_one_material_stays_checked_and_the_choice_is_forgotten_with_the_model() {
    let dir = temp_dir("checks");
    let path = character(&dir, "c.fbx");
    let mut s = S::new(64, 64);
    open_new_with(&mut s, &path);
    np(&mut s, NpAction::Material(0, false));
    np(&mut s, NpAction::Material(1, false));
    assert_eq!(s.np.window.as_ref().unwrap().materials, Some(vec![2]));
    np(&mut s, NpAction::Material(2, false)); // 最後の 1 つは外せない
    assert_eq!(s.np.window.as_ref().unwrap().materials, Some(vec![2]));
    np(&mut s, NpAction::Material(0, true));
    np(&mut s, NpAction::Material(1, true));
    assert_eq!(
        s.np.window.as_ref().unwrap().materials,
        None,
        "全部に戻せば既定"
    );
    np(&mut s, NpAction::Material(0, false));
    // モデルを選び直すと、選びは全部に戻る
    np(&mut s, NpAction::ChooseModel(path.clone()));
    wait_model(&mut s);
    assert_eq!(s.np.window.as_ref().unwrap().materials, None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn preparing_can_be_canceled_and_tried_again_and_a_broken_model_says_why() {
    let dir = temp_dir("prepare");
    let path = character(&dir, "c.fbx");
    let mut s = S::new(64, 64);
    np(&mut s, NpAction::OpenNew);
    np(&mut s, NpAction::ChooseModel(path.clone()));
    assert!(s.np.window.as_ref().unwrap().is_loading());
    // 準備が済む前に決められない
    np(&mut s, NpAction::Submit);
    assert!(s.np.window.is_some(), "準備ができていなければ作らない");
    assert!(s.np.window.as_ref().unwrap().error.is_some());
    np(&mut s, NpAction::CancelPrepare);
    assert!(matches!(
        s.np.window.as_ref().unwrap().prep,
        Prep::Canceled { .. }
    ));
    s.poll_newproject();
    assert!(
        matches!(s.np.window.as_ref().unwrap().prep, Prep::Canceled { .. }),
        "取り消したあとに結果が来ても入らない"
    );
    np(&mut s, NpAction::PrepareAgain);
    assert!(s.np.window.as_ref().unwrap().is_loading());
    wait_model(&mut s);
    assert!(matches!(
        s.np.window.as_ref().unwrap().prep,
        Prep::Ready { .. }
    ));
    // 壊れたファイルは理由を持つ（日英）
    let broken = write_fbx(&dir, "broken.fbx", "これは FBX ではない");
    np(&mut s, NpAction::ChooseModel(broken));
    wait_model(&mut s);
    match &s.np.window.as_ref().unwrap().prep {
        Prep::Failed { error, .. } => {
            assert!(!Lang::Ja.view_error(error).is_empty());
            assert!(!Lang::En
                .view_error(error)
                .chars()
                .any(|c| ('\u{3040}'..='\u{9fff}').contains(&c)));
        }
        other => panic!("読めないはず: {}", matches!(other, Prep::Ready { .. })),
    }
    np(&mut s, NpAction::Submit);
    assert!(s.np.window.is_some(), "読めなかったモデルでは作らない");
    // モデルを外せば作れる
    np(&mut s, NpAction::ClearModel);
    np(&mut s, NpAction::Submit);
    assert!(s.np.window.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn closing_the_window_stops_the_preparation_and_changes_nothing() {
    let dir = temp_dir("close");
    let path = character(&dir, "c.fbx");
    let mut s = S::new(64, 64);
    let id = s.doc.id();
    open_new_with(&mut s, &path);
    np(&mut s, NpAction::Close);
    assert!(s.np.window.is_none());
    assert_eq!(s.doc.id(), id, "閉じたら何も変えない");
    assert!(s.model.is_none() && s.view3d.model.is_none());
    // 読んでいる途中で閉じても、あとから何も入らない
    np(&mut s, NpAction::OpenNew);
    np(&mut s, NpAction::ChooseModel(path));
    np(&mut s, NpAction::Close);
    for _ in 0..50 {
        s.poll_newproject();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(s.model.is_none() && s.view3d.model.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn nothing_changes_while_drawing_and_the_window_refuses_to_open() {
    let mut s = S::new(64, 64);
    let layer = s.selected_layer.unwrap();
    let brush = s.stroke_settings(false);
    s.stroke = Some(s.doc.begin_stroke(layer, &brush).unwrap());
    np(&mut s, NpAction::OpenNew);
    assert!(s.np.window.is_none());
    np(&mut s, NpAction::OpenConfigure);
    assert!(s.np.window.is_none());
    assert!(s.message.contains("描いている"), "{}", s.message);
    let stroke = s.stroke.take().unwrap();
    s.doc.cancel_stroke(stroke);
    np(&mut s, NpAction::OpenConfigure);
    assert!(s.np.window.is_some());
}

#[test]
fn bakes_mesh_maps_after_creating_when_asked() {
    let dir = temp_dir("bake");
    let path = character(&dir, "c.fbx");
    let mut s = S::new(64, 64);
    s.bake.backend = yolu_app::bake::BakeBackend::Cpu;
    open_new_with(&mut s, &path);
    np(&mut s, NpAction::Resolution(512));
    np(&mut s, NpAction::BakeAfter(true));
    np(&mut s, NpAction::Submit);
    assert!(s.np.window.is_none());
    assert!(
        s.bake.is_baking(),
        "作ったあとにベイクを始める: {}",
        s.message
    );
    let start = Instant::now();
    while s.bake.is_baking() {
        s.poll_bake();
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "ベイクが終わらない"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        s.sets.iter().any(|x| !x.mesh_maps.is_empty()),
        "焼いたメッシュマップがセットに付く: {}",
        s.message
    );
    // モデルが無ければベイクの設定は効かない（ウィンドウの部品も押せない）
    let mut t = S::new(64, 64);
    np(&mut t, NpAction::OpenNew);
    np(&mut t, NpAction::BakeAfter(true));
    np(&mut t, NpAction::Submit);
    assert!(!t.bake.is_baking());
    let _ = std::fs::remove_dir_all(dir);
}

// ───────── 構成 ─────────

fn project_from(path: &Path, size: u32) -> S {
    let mut s = S::new(64, 64);
    open_new_with(&mut s, path);
    np(&mut s, NpAction::Resolution(size));
    np(&mut s, NpAction::Submit);
    assert!(s.np.window.is_none());
    s
}

fn paint_dot(s: &mut S, index: usize, at: (u32, u32), color: Rgba8) {
    let doc = s.set_doc_mut(index);
    let layer = doc.layers().last().unwrap().id();
    doc.set_pixel(layer, at.0, at.1, color).unwrap();
}

/// 一番上のレイヤーの全面を 1 回のストロークで塗る（どのタイルにも画素が入る）。
fn fill_all(s: &mut S, index: usize, color: Rgba8) {
    let doc = s.set_doc_mut(index);
    let layer = doc.layers().last().unwrap().id();
    let brush = yolu_app::engine::BrushSettings {
        radius: 1000.0,
        hardness: 1.0,
        color,
        pressure_opacity: false,
        pressure_size: false,
        ..Default::default()
    };
    let (cx, cy) = (doc.width() as f64 / 2.0, doc.height() as f64 / 2.0);
    let mut stroke = doc.begin_stroke(layer, &brush).unwrap();
    stroke
        .add_point(doc, cx, cy, 1.0, Default::default())
        .unwrap();
    doc.end_stroke(stroke).unwrap();
}

fn pixel_of(s: &S, index: usize, at: (u32, u32)) -> Rgba8 {
    let doc = s.set_doc(index);
    let layer = doc.layers().last().unwrap().id();
    doc.layer(layer)
        .unwrap()
        .pixel(Channel::Color, at.0, at.1)
        .unwrap()
}

#[test]
fn configuration_opens_with_the_sets_of_the_project() {
    let dir = temp_dir("configure-open");
    let path = character(&dir, "c.fbx");
    let mut s = project_from(&path, 512);
    np(&mut s, NpAction::OpenConfigure);
    let win = s.np.window.as_ref().unwrap();
    assert!(win.configure);
    assert_eq!(win.drafts.len(), 3);
    assert_eq!(win.drafts[0].name, "Skin");
    assert_eq!(win.drafts[0].size, (512, 512));
    assert_eq!(win.drafts[0].material, Some(0));
    assert_eq!(win.drafts[1].material, Some(1));
    assert!(!win.drafts[0].resizes());
    assert_eq!(win.normal, NormalYDirection::OpenGL);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn renaming_changing_the_material_and_the_normal_format_apply_without_asking() {
    let dir = temp_dir("configure-rename");
    let path = character(&dir, "c.fbx");
    let mut s = project_from(&path, 512);
    paint_dot(&mut s, 1, (10, 10), Rgba8::new(200, 10, 10, 255));
    let uids: Vec<u32> = s.sets.iter().map(|x| x.uid).collect();
    np(&mut s, NpAction::OpenConfigure);
    np(&mut s, NpAction::Draft(0, DraftOp::Name("  肌  ".into())));
    // Skin のセットを Eye のマテリアルへ、Eye のセットは Skin へ入れ替える（重ならない順に）
    np(&mut s, NpAction::Draft(2, DraftOp::Material(None)));
    np(&mut s, NpAction::Draft(0, DraftOp::Material(Some(2))));
    np(&mut s, NpAction::Draft(2, DraftOp::Material(Some(0))));
    np(&mut s, NpAction::Normal(NormalYDirection::DirectX));
    np(&mut s, NpAction::Submit);
    assert!(
        s.np.window.is_none(),
        "確かめなしで適用する: {:?}",
        s.message
    );
    assert_eq!(set_names(&s), ["肌", "Hair", "Eye"], "前後の空白は除く");
    assert_eq!(
        s.sets.iter().map(|x| x.uid).collect::<Vec<_>>(),
        uids,
        "同じセット（文書）のまま"
    );
    assert_eq!(s.sets.get(0).unwrap().bound, Some(2));
    assert_eq!(s.sets.get(2).unwrap().bound, Some(0));
    assert_eq!(
        s.sets.get(0).unwrap().material,
        MaterialRef::Material {
            name: "Eye".into(),
            asset: None
        }
    );
    assert_eq!(
        pixel_of(&s, 1, (10, 10)),
        Rgba8::new(200, 10, 10, 255),
        "画素は変わらない"
    );
    for i in 0..3 {
        assert_eq!(
            s.set_doc(i).normal_settings().file_direction(),
            NormalYDirection::DirectX
        );
    }
    assert!(s.modified);
    assert!(s.message.contains("構成"), "{}", s.message);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn names_and_materials_that_collide_are_refused_with_a_reason_and_nothing_changes() {
    let dir = temp_dir("configure-refuse");
    let path = character(&dir, "c.fbx");
    let mut s = project_from(&path, 512);
    np(&mut s, NpAction::OpenConfigure);
    let refused = |s: &mut S, why: &str| {
        np(s, NpAction::Submit);
        let win = s.np.window.as_ref().expect("断ったらウィンドウは残る");
        assert!(win.confirm.is_none());
        let error = win.error.clone().expect("理由が出る");
        assert!(!error.is_empty(), "{why}");
        assert_eq!(set_names(s), ["Skin", "Hair", "Eye"], "{why}: 何も変えない");
    };
    np(&mut s, NpAction::Draft(1, DraftOp::Name("skin".into())));
    refused(&mut s, "大文字小文字を区別しない同じ名前");
    np(&mut s, NpAction::Draft(1, DraftOp::Name("   ".into())));
    refused(&mut s, "空の名前");
    np(&mut s, NpAction::Draft(1, DraftOp::Name("a\nb".into())));
    refused(&mut s, "制御文字");
    np(&mut s, NpAction::Draft(1, DraftOp::Name("x".repeat(257))));
    refused(&mut s, "長すぎる名前");
    np(&mut s, NpAction::Draft(1, DraftOp::Name("Hair".into())));
    // 同じマテリアルを描くのは選べない（メニューも断る）
    np(&mut s, NpAction::Draft(1, DraftOp::Material(Some(0))));
    assert_eq!(s.np.window.as_ref().unwrap().drafts[1].material, Some(1));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn resizing_resamples_the_set_clears_its_history_and_asks_first() {
    let dir = temp_dir("configure-resize");
    let path = character(&dir, "c.fbx");
    let mut s = project_from(&path, 512);
    paint_dot(&mut s, 1, (10, 10), Rgba8::new(0, 200, 0, 255));
    let first = s.set_doc(1).layers()[0].id();
    s.set_doc_mut(1).set_layer_name(first, "名前").unwrap();
    assert!(s.set_doc(1).can_undo());
    np(&mut s, NpAction::OpenConfigure);
    np(&mut s, NpAction::Draft(1, DraftOp::Size(1024, 1024)));
    np(
        &mut s,
        NpAction::Resampling(Some(CanvasResampling::Nearest)),
    );
    np(&mut s, NpAction::Submit);
    // 確かめが先に出る（まだ変えない）
    let win = s.np.window.as_ref().expect("確かめの間はウィンドウが残る");
    let plan = win.confirm.as_ref().expect("大きさの変更は確かめる");
    assert!(plan.needs_confirm());
    assert_eq!(plan.rows.len(), 1);
    assert_eq!(plan.rows[0].left, "Hair");
    assert!(
        plan.rows[0].middle.contains("512") && plan.rows[0].middle.contains("1024"),
        "{:?}",
        plan.rows[0]
    );
    assert_eq!(size_of(&s, 1), (512, 512));
    // やめる
    np(&mut s, NpAction::ConfirmCancel);
    assert!(s.np.window.as_ref().unwrap().confirm.is_none());
    assert_eq!(size_of(&s, 1), (512, 512));
    // 適用
    np(&mut s, NpAction::Submit);
    np(&mut s, NpAction::ConfirmApply);
    assert!(s.np.window.is_none());
    assert_eq!(size_of(&s, 1), (1024, 1024));
    assert_eq!(size_of(&s, 0), (512, 512), "ほかのセットは変えない");
    assert!(
        !s.set_doc(1).can_undo() && !s.set_doc(1).can_redo(),
        "大きさを変えたセットの履歴は消える"
    );
    // 最近傍で 2 倍: 画素 (10, 10) は (20..22, 20..22) になる
    for (x, y) in [(20, 20), (21, 21)] {
        assert_eq!(pixel_of(&s, 1, (x, y)), Rgba8::new(0, 200, 0, 255));
    }
    assert_eq!(pixel_of(&s, 1, (22, 22)), Rgba8::new(0, 0, 0, 0));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_shrink_that_removes_remembered_selections_says_so_in_the_result_in_both_languages() {
    for lang in Lang::ALL {
        let mut s = S::new_in(64, 64, lang);
        np(&mut s, NpAction::OpenNew);
        np(&mut s, NpAction::Template(Template::ColorOnly));
        np(&mut s, NpAction::Resolution(1024));
        np(&mut s, NpAction::Submit);
        // 偶数の列の 1 画素幅の線は、2 分の 1 の最近傍で拾われず消える。大きな選択範囲は残る
        let doc = s.set_doc_mut(0);
        let thin = SelectionMask::rectangle(doc, 10, 10, 11, 200);
        doc.set_selection(Some(thin)).unwrap();
        doc.save_selection("thin").unwrap();
        let wide = SelectionMask::rectangle(doc, 100, 100, 400, 400);
        doc.set_selection(Some(wide)).unwrap();
        doc.save_selection("wide").unwrap();
        let set_name = s.sets.get(0).unwrap().name.clone();
        np(&mut s, NpAction::OpenConfigure);
        np(&mut s, NpAction::Draft(0, DraftOp::Size(512, 512)));
        np(
            &mut s,
            NpAction::Resampling(Some(CanvasResampling::Nearest)),
        );
        np(&mut s, NpAction::Submit);
        np(&mut s, NpAction::ConfirmApply);
        assert!(
            s.np.window.is_none(),
            "{:?}",
            s.np.window.as_ref().map(|w| w.error.clone())
        );
        assert_eq!(size_of(&s, 0), (512, 512));
        let kept: Vec<&str> = s
            .set_doc(0)
            .saved_selections()
            .iter()
            .map(|x| x.name.as_str())
            .collect();
        assert_eq!(kept, ["wide"], "{lang:?}");
        assert!(
            !s.set_doc(0).can_undo(),
            "履歴は消える（取り消しでは戻らない）"
        );
        // 戻せないので、外れたことと数を、結果の文で言う
        let want = lang.pick(
            format!("縮小で消えた覚えた選択範囲があります（{set_name} 1 件）。"),
            format!("Some remembered selections were lost to the shrink ({set_name} 1)."),
        );
        assert!(s.message.contains(&want), "{lang:?}: {}", s.message);
        assert_eq!(
            s.message.contains("縮小"),
            lang == Lang::Ja,
            "{lang:?}: {}",
            s.message
        );
    }
    // 縮小でも全部残るときは、何も言わない
    let mut s = S::new(64, 64);
    np(&mut s, NpAction::OpenNew);
    np(&mut s, NpAction::Template(Template::ColorOnly));
    np(&mut s, NpAction::Resolution(1024));
    np(&mut s, NpAction::Submit);
    let doc = s.set_doc_mut(0);
    let wide = SelectionMask::rectangle(doc, 100, 100, 400, 400);
    doc.set_selection(Some(wide)).unwrap();
    doc.save_selection("wide").unwrap();
    np(&mut s, NpAction::OpenConfigure);
    np(&mut s, NpAction::Draft(0, DraftOp::Size(512, 512)));
    np(&mut s, NpAction::Submit);
    np(&mut s, NpAction::ConfirmApply);
    assert_eq!(s.set_doc(0).saved_selections().len(), 1);
    assert!(!s.message.contains("消えた覚えた"), "{}", s.message);
}

#[test]
fn the_automatic_resampling_shrinks_by_area_and_enlarges_bilinearly() {
    let mut s = S::new(64, 64);
    np(&mut s, NpAction::OpenNew);
    np(&mut s, NpAction::Resolution(1024));
    np(&mut s, NpAction::Submit);
    paint_dot(&mut s, 0, (0, 0), Rgba8::new(255, 0, 0, 255));
    np(&mut s, NpAction::OpenConfigure);
    np(&mut s, NpAction::Draft(0, DraftOp::Size(512, 512)));
    np(&mut s, NpAction::Submit);
    let plan = s.np.window.as_ref().unwrap().confirm.clone().unwrap();
    assert_eq!(plan.rows[0].right, "面積平均");
    np(&mut s, NpAction::ConfirmApply);
    // 4 画素の面積平均: 1 画素の赤は 1/4 の濃さ
    let px = pixel_of(&s, 0, (0, 0));
    assert_eq!(px.r, 255);
    assert!((60..=68).contains(&px.a), "{px:?}");
    np(&mut s, NpAction::OpenConfigure);
    np(&mut s, NpAction::Draft(0, DraftOp::Size(1024, 1024)));
    np(&mut s, NpAction::Submit);
    assert_eq!(
        s.np.window.as_ref().unwrap().confirm.clone().unwrap().rows[0].right,
        "バイリニア"
    );
}

#[test]
fn a_resize_refused_halfway_changes_no_set_and_keeps_every_history() {
    let dir = temp_dir("configure-rollback");
    let path = character(&dir, "c.fbx");
    let mut s = project_from(&path, 1024);
    paint_dot(&mut s, 0, (3, 3), Rgba8::new(10, 20, 30, 255));
    fill_all(&mut s, 1, Rgba8::new(40, 50, 60, 255));
    // 1 つ目（Skin）に、Undo の段と Redo の段を持たせ、履歴の予算をちょうどにする（大きさを変えて積むと、古い段が予算から落ちる）
    let layer = s.set_doc(0).layers()[0].id();
    s.set_doc_mut(0).set_layer_name(layer, "一").unwrap();
    s.set_doc_mut(0).set_layer_name(layer, "二").unwrap();
    s.set_doc_mut(0).undo().unwrap();
    let budget = s.set_doc(0).history_bytes();
    s.set_doc_mut(0).set_undo_budget_bytes(budget).unwrap();
    // 2 つ目（Hair）は広げるが、画素の予算が足りず断られる。1 つ目（Skin）は縮める（先に入るはずだった）
    let tight = s.set_doc(1).allocated_bytes();
    s.set_doc_mut(1).set_source_budget_bytes(tight).unwrap();
    let state = |s: &S, i: usize| {
        let d = s.set_doc(i);
        (
            d.undo_count(),
            d.redo_count(),
            d.history_bytes(),
            d.revision(),
        )
    };
    let before = (state(&s, 0), state(&s, 1));
    assert!(before.0 .0 >= 1 && before.0 .1 == 1, "{before:?}");
    np(&mut s, NpAction::OpenConfigure);
    np(&mut s, NpAction::Draft(0, DraftOp::Size(512, 512)));
    np(&mut s, NpAction::Draft(1, DraftOp::Size(2048, 2048)));
    np(&mut s, NpAction::Submit);
    np(&mut s, NpAction::ConfirmApply);
    let win = s.np.window.as_ref().expect("断られたらウィンドウは残る");
    let error = win.error.clone().expect("理由");
    assert!(error.contains("「Hair」"), "{error}");
    assert_eq!(
        size_of(&s, 0),
        (1024, 1024),
        "先に縮めるはずのセットも元の大きさ"
    );
    assert_eq!(size_of(&s, 1), (1024, 1024));
    assert_eq!(pixel_of(&s, 0, (3, 3)), Rgba8::new(10, 20, 30, 255));
    assert_eq!(pixel_of(&s, 1, (4, 4)), Rgba8::new(40, 50, 60, 255));
    assert_eq!(
        (state(&s, 0), state(&s, 1)),
        before,
        "どちらのセットも文書も履歴（Undo・Redo・予算の中の段）も何も変わらない"
    );
    // 断られた理由を直せば（Hair を縮める）、そのまま適用できる
    np(&mut s, NpAction::Draft(1, DraftOp::Size(512, 512)));
    np(&mut s, NpAction::Submit);
    np(&mut s, NpAction::ConfirmApply);
    assert!(
        s.np.window.is_none(),
        "{:?}",
        s.np.window
            .as_ref()
            .map(|w| (w.error.clone(), w.confirm.is_some()))
    );
    assert_eq!(size_of(&s, 0), (512, 512));
    assert_eq!(size_of(&s, 1), (512, 512));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn removing_sets_asks_first_loses_their_work_and_keeps_one() {
    let dir = temp_dir("configure-remove");
    let path = character(&dir, "c.fbx");
    let mut s = project_from(&path, 512);
    paint_dot(&mut s, 1, (10, 10), Rgba8::new(1, 2, 3, 255));
    let keep: Vec<u32> = [0, 2].iter().map(|i| s.sets.get(*i).unwrap().uid).collect();
    np(&mut s, NpAction::OpenConfigure);
    np(&mut s, NpAction::Draft(1, DraftOp::Remove));
    np(&mut s, NpAction::Submit);
    let plan =
        s.np.window
            .as_ref()
            .unwrap()
            .confirm
            .clone()
            .expect("消すときは確かめる");
    assert_eq!(
        (
            plan.rows.len(),
            plan.rows[0].left.as_str(),
            plan.rows[0].right.as_str()
        ),
        (1, "Hair", "消す")
    );
    assert_eq!(s.sets.len(), 3, "確かめの前は消さない");
    np(&mut s, NpAction::ConfirmApply);
    assert_eq!(set_names(&s), ["Skin", "Eye"]);
    assert_eq!(s.sets.iter().map(|x| x.uid).collect::<Vec<_>>(), keep);
    // 最後の 1 つは消せない（ウィンドウでも消す操作でも）
    np(&mut s, NpAction::OpenConfigure);
    np(&mut s, NpAction::Draft(0, DraftOp::Remove));
    np(&mut s, NpAction::Draft(0, DraftOp::Remove));
    assert_eq!(s.np.window.as_ref().unwrap().drafts.len(), 1);
    np(&mut s, NpAction::Close);
    let only = vec![s.sets.get(0).unwrap().uid, s.sets.get(1).unwrap().uid];
    np(&mut s, NpAction::RemoveSets(only));
    assert!(
        s.np.remove_confirm.is_none(),
        "全部は消せない: {}",
        s.message
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn removing_the_current_set_switches_to_the_next_one() {
    let dir = temp_dir("configure-remove-current");
    let path = character(&dir, "c.fbx");
    let mut s = project_from(&path, 512);
    s.switch_set(1).unwrap();
    let hair = s.sets.current().uid;
    let eye = s.sets.get(2).unwrap().uid;
    np(&mut s, NpAction::RemoveSets(vec![hair]));
    assert_eq!(s.np.remove_confirm, Some(vec![hair]));
    np(&mut s, NpAction::ConfirmRemove);
    assert_eq!(set_names(&s), ["Skin", "Eye"]);
    assert_eq!(s.sets.current().uid, eye, "並びの次が今のセットになる");
    assert_eq!(s.sets.current_index(), 1);
    assert_eq!(s.view3d.material, s.sets.current().bound.unwrap() as i32);
    // 最後のセットを消すと、前のセットが今のセット
    let skin = s.sets.get(0).unwrap().uid;
    np(&mut s, NpAction::RemoveSets(vec![eye]));
    np(&mut s, NpAction::ConfirmRemove);
    assert_eq!(s.sets.current().uid, skin);
    // やめる
    np(&mut s, NpAction::AddSet);
    let added = s.sets.current().uid;
    np(&mut s, NpAction::RemoveSets(vec![added]));
    np(&mut s, NpAction::CancelRemove);
    assert_eq!(s.sets.len(), 2);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn adding_a_set_uses_the_open_sets_size_channels_and_normal_format() {
    let mut s = S::new(64, 64);
    np(&mut s, NpAction::OpenNew);
    np(&mut s, NpAction::Template(Template::LilToon));
    np(&mut s, NpAction::Resolution(1024));
    np(&mut s, NpAction::Normal(NormalYDirection::DirectX));
    np(&mut s, NpAction::Submit);
    np(&mut s, NpAction::AddSet);
    assert_eq!(s.sets.len(), 2);
    assert_eq!(s.sets.current_index(), 1, "足したセットが今のセット");
    assert_eq!(set_names(&s), ["テクスチャセット 1", "テクスチャセット 2"]);
    assert_eq!(size_of(&s, 1), (1024, 1024));
    assert_eq!(
        enabled_channels(&s.doc),
        [Channel::Color, Channel::Normal, Channel::Emission]
    );
    assert_eq!(
        s.doc.normal_settings().file_direction(),
        NormalYDirection::DirectX
    );
    assert!(!s.doc.can_undo() && s.doc.layers().len() == 1);
    assert_eq!(
        s.sets.current().material,
        MaterialRef::PendingSlot(1),
        "空いている仮のスロット"
    );
    assert!(s.sets.current().bound.is_none());
    assert!(s.modified);
}

#[test]
fn adding_a_set_in_the_window_for_a_material_without_one() {
    let dir = temp_dir("configure-add");
    let path = character(&dir, "c.fbx");
    let mut s = S::new(64, 64);
    open_new_with(&mut s, &path);
    np(&mut s, NpAction::Material(2, false));
    np(&mut s, NpAction::Resolution(512));
    np(&mut s, NpAction::Submit);
    assert_eq!(set_names(&s), ["Skin", "Hair"]);
    np(&mut s, NpAction::OpenConfigure);
    {
        let win = s.np.window.as_ref().unwrap();
        assert_eq!(
            yolu_app::newproject::unused_groups(&s, win),
            1,
            "Eye にセットが無い"
        );
    }
    np(&mut s, NpAction::AddDraft);
    {
        let win = s.np.window.as_ref().unwrap();
        let d = win.drafts.last().unwrap();
        assert_eq!(
            (d.uid, d.name.as_str(), d.material, d.size),
            (None, "Eye", Some(2), (512, 512))
        );
    }
    // もう 1 つ足すとマテリアルの無いセット
    np(&mut s, NpAction::AddDraft);
    {
        let d = s.np.window.as_ref().unwrap().drafts.last().unwrap().clone();
        assert_eq!((d.material, d.name.as_str()), (None, "テクスチャセット 4"));
        assert!(matches!(d.key, MaterialRef::PendingSlot(_)));
    }
    np(&mut s, NpAction::Draft(3, DraftOp::Size(1024, 1024)));
    np(&mut s, NpAction::Submit);
    assert!(
        s.np.window.is_none(),
        "足すだけなら確かめない: {}",
        s.message
    );
    assert_eq!(set_names(&s), ["Skin", "Hair", "Eye", "テクスチャセット 4"]);
    assert_eq!(s.sets.get(2).unwrap().bound, Some(2));
    assert_eq!(s.sets.get(3).unwrap().bound, None);
    assert_eq!(size_of(&s, 3), (1024, 1024));
    assert_eq!(size_of(&s, 2), (512, 512));
    assert_eq!(
        enabled_channels(s.set_doc(2)).len(),
        6,
        "開いているセットと同じチャンネル"
    );
    let _ = std::fs::remove_dir_all(dir);
}

// ───────── モデルの差し替えと読み直し ─────────

/// Hair・Cape・skin（小文字）の 3 つのマテリアル。Eye は無い。
fn cape_model(dir: &Path, name: &str) -> PathBuf {
    let text = ascii_fbx(
        &["Hair", "Cape", "skin"],
        &[
            mesh("A", &[Some(0)]),
            mesh("B", &[Some(1)]),
            mesh("C", &[Some(2), Some(2)]),
        ],
    );
    write_fbx(dir, name, &text)
}

fn configure_with(s: &mut S, path: &Path) {
    np(s, NpAction::OpenConfigure);
    np(s, NpAction::ChooseModel(path.to_path_buf()));
    wait_model(s);
    assert!(
        matches!(s.np.window.as_ref().unwrap().prep, Prep::Ready { .. }),
        "替えるモデルを準備できなかった"
    );
}

#[test]
fn changing_the_model_moves_the_sets_to_the_new_materials_and_keeps_their_pixels() {
    let dir = temp_dir("swap");
    let old = character(&dir, "old.fbx");
    let new = cape_model(&dir, "new.fbx");
    let mut s = project_from(&old, 512);
    fill_all(&mut s, 1, Rgba8::new(9, 8, 7, 255));
    let uids: Vec<u32> = s.sets.iter().map(|x| x.uid).collect();
    let hair_id = s.set_doc(1).id();
    configure_with(&mut s, &new);
    {
        let win = s.np.window.as_ref().unwrap();
        let groups = win.groups(&s);
        assert_eq!(
            groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
            ["Hair", "Cape", "skin"]
        );
        // 鍵で照合し直す: 大文字小文字を区別せず Skin → skin、名前が同じ Hair → Hair、Eye はモデルに無い
        assert_eq!(
            win.drafts.iter().map(|d| d.material).collect::<Vec<_>>(),
            [Some(2), Some(0), None]
        );
        assert_eq!(
            yolu_app::newproject::unused_groups(&s, win),
            1,
            "Cape にセットが無い"
        );
    }
    np(&mut s, NpAction::Submit);
    let plan =
        s.np.window
            .as_ref()
            .unwrap()
            .confirm
            .clone()
            .expect("モデルの差し替えは確かめる");
    assert_eq!(plan.rows[0].left, "old → new");
    assert_eq!(plan.rows[0].right, "差し替え");
    let rows: Vec<(&str, &str, bool)> = plan.rows[1..]
        .iter()
        .map(|r| (r.left.as_str(), r.middle.as_str(), r.warning))
        .collect();
    assert_eq!(
        rows,
        [
            ("Skin", "→ skin", false),
            ("Hair", "→ Hair", false),
            ("Eye", "→ モデルに無い", true)
        ]
    );
    // 確かめるあいだは何も変えない
    assert_eq!(s.model.as_ref().unwrap().name, "old");
    np(&mut s, NpAction::ConfirmApply);
    assert!(s.np.window.is_none());
    assert_eq!(s.model.as_ref().unwrap().name, "new");
    assert_eq!(s.np.model_file.as_deref(), Some(new.as_path()));
    assert_eq!(
        s.sets.iter().map(|x| x.uid).collect::<Vec<_>>(),
        uids,
        "セットは同じまま（セットを増やさない）"
    );
    assert_eq!(s.sets.get(0).unwrap().bound, Some(2));
    assert_eq!(s.sets.get(1).unwrap().bound, Some(0));
    assert_eq!(s.sets.get(2).unwrap().bound, None, "モデルに無いセット");
    assert_eq!(
        s.sets.get(0).unwrap().material,
        MaterialRef::Material {
            name: "skin".into(),
            asset: None
        },
        "合ったセットの鍵は新しいマテリアルの鍵に書き換える"
    );
    assert_eq!(
        s.sets.get(2).unwrap().material,
        MaterialRef::Material {
            name: "Eye".into(),
            asset: None
        },
        "合わないセットの鍵は変えない"
    );
    assert_eq!(s.set_doc(1).id(), hair_id, "画素（文書）はそのまま");
    assert_eq!(pixel_of(&s, 1, (300, 300)), Rgba8::new(9, 8, 7, 255));
    assert_eq!(size_of(&s, 1), (512, 512));
    assert!(
        s.message.contains("new") && s.message.contains("モデルに無いセット 1"),
        "{}",
        s.message
    );
    // 3D は今のセット（Skin → skin）のマテリアル
    assert_eq!(s.view3d.material, 2);
    // もう一度開いて、セットの無いマテリアルを足す
    np(&mut s, NpAction::OpenConfigure);
    np(&mut s, NpAction::AddUnused);
    np(&mut s, NpAction::Submit);
    assert!(s.np.window.is_none());
    assert_eq!(set_names(&s), ["Skin", "Hair", "Eye", "Cape"]);
    assert_eq!(s.sets.get(3).unwrap().bound, Some(1));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_cancelled_model_change_leaves_the_project_as_it_was() {
    let dir = temp_dir("swap-cancel");
    let old = character(&dir, "old.fbx");
    let new = cape_model(&dir, "new.fbx");
    let mut s = project_from(&old, 512);
    configure_with(&mut s, &new);
    np(&mut s, NpAction::Submit);
    np(&mut s, NpAction::ConfirmCancel);
    assert!(s.np.window.as_ref().unwrap().confirm.is_none());
    assert_eq!(s.model.as_ref().unwrap().name, "old");
    assert_eq!(s.sets.get(0).unwrap().bound, Some(0));
    // 「替えない」で下書きを今のモデルに戻す
    np(&mut s, NpAction::ClearModel);
    {
        let win = s.np.window.as_ref().unwrap();
        assert!(win.model.is_none());
        assert_eq!(
            win.drafts.iter().map(|d| d.material).collect::<Vec<_>>(),
            [Some(0), Some(1), Some(2)]
        );
    }
    np(&mut s, NpAction::Submit);
    assert!(s.np.window.is_none(), "替えないなら確かめない");
    assert_eq!(s.model.as_ref().unwrap().name, "old");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reloading_the_model_after_it_was_exported_again_keeps_the_sets_on_their_materials() {
    let dir = temp_dir("reload");
    let path = character(&dir, "c.fbx");
    let mut s = project_from(&path, 512);
    // FBX を書き出し直した（Hair が Hair2 になり、メッシュが増えた）
    let again = ascii_fbx(
        &["Skin", "Hair2", "Eye"],
        &[
            mesh("Body", &[Some(0)]),
            mesh("Wig", &[Some(1)]),
            mesh("EyeMesh", &[Some(2)]),
            mesh("Extra", &[Some(1)]),
        ],
    );
    write_fbx(&dir, "c.fbx", &again);
    np(&mut s, NpAction::OpenConfigure);
    np(&mut s, NpAction::Reload);
    assert!(s.np.window.as_ref().unwrap().reload && s.np.window.as_ref().unwrap().is_loading());
    wait_model(&mut s);
    np(&mut s, NpAction::Submit);
    let plan =
        s.np.window
            .as_ref()
            .unwrap()
            .confirm
            .clone()
            .expect("読み直しも確かめる");
    assert_eq!(plan.rows[0].right, "読み直す");
    np(&mut s, NpAction::ConfirmApply);
    assert!(s.np.window.is_none());
    assert_eq!(
        s.sets.get(0).unwrap().bound,
        Some(0),
        "Skin は同じマテリアルのまま"
    );
    assert_eq!(
        s.sets.get(1).unwrap().bound,
        None,
        "名前が変わった Hair は新しいモデルに無い"
    );
    assert_eq!(s.sets.get(2).unwrap().bound, Some(2));
    assert_eq!(
        s.model.as_ref().unwrap().triangles,
        8,
        "読み直したモデル（4 つの板 = 三角形 8 つ）が入っている"
    );
    // 読み直すファイルが無いプロジェクトでは、読み直せない
    let mut t = S::new(64, 64);
    np(&mut t, NpAction::OpenConfigure);
    np(&mut t, NpAction::Reload);
    assert!(!t.np.window.as_ref().unwrap().reload && !t.np.window.as_ref().unwrap().is_loading());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn pressing_reload_again_takes_the_reload_back() {
    let dir = temp_dir("reload-toggle");
    let path = character(&dir, "c.fbx");
    let mut s = project_from(&path, 512);
    np(&mut s, NpAction::OpenConfigure);
    np(&mut s, NpAction::Reload);
    assert!(s.np.window.as_ref().unwrap().reload);
    np(&mut s, NpAction::Reload);
    let win = s.np.window.as_ref().unwrap();
    assert!(!win.reload && win.model.is_none() && !win.is_loading());
    np(&mut s, NpAction::Submit);
    assert!(s.np.window.is_none(), "読み直さないなら確かめない");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_key_two_sets_would_share_is_made_unique_so_the_project_still_saves() {
    // Skin のメッシュと、マテリアルの無いメッシュ
    let dir = temp_dir("dedupe");
    let text = ascii_fbx(
        &["Skin"],
        &[mesh("Body", &[Some(0)]), mesh("Bare", &[None])],
    );
    let path = write_fbx(&dir, "c.fbx", &text);
    let mut s = project_from(&path, 512);
    assert_eq!(set_names(&s), ["Skin", "Unassigned"]);
    assert_eq!(s.sets.get(1).unwrap().material, MaterialRef::Unassigned);
    np(&mut s, NpAction::OpenConfigure);
    // Unassigned のセットをモデルに無いことにして（鍵は Unassigned のまま）、同じ Unassigned のマテリアルに新しいセットを足す
    np(&mut s, NpAction::Draft(1, DraftOp::Material(None)));
    np(&mut s, NpAction::AddDraft);
    {
        let d = s.np.window.as_ref().unwrap().drafts.last().unwrap().clone();
        assert_eq!(
            (d.name.as_str(), d.key.clone()),
            ("Unassigned 2", MaterialRef::Unassigned)
        );
    }
    np(&mut s, NpAction::Submit);
    assert!(
        s.np.window.is_none(),
        "{:?}",
        s.np.window.as_ref().map(|w| w.error.clone())
    );
    // 同じ鍵を持つのは 1 つだけ（付いたほう）。外れたほうは名前の鍵になる
    assert_eq!(s.sets.get(2).unwrap().material, MaterialRef::Unassigned);
    assert_eq!(s.sets.get(2).unwrap().bound, Some(1));
    assert_eq!(
        s.sets.get(1).unwrap().material,
        MaterialRef::Material {
            name: "Unassigned".into(),
            asset: None
        }
    );
    // 保存できて、開き直すと同じに付く
    let ylp = dir.join("p.ylp");
    s.apply(Action::SaveProjectAs(ylp.clone()));
    assert!(s.message.contains("保存しました"), "{}", s.message);
    let mut t = S::new(64, 64);
    t.apply(Action::OpenProject(ylp));
    wait_reopen(&mut t);
    assert_eq!(
        t.sets.iter().map(|x| x.bound).collect::<Vec<_>>(),
        [Some(0), None, Some(1)]
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// 3D のパスレイヤーを足す。指紋は今のモデルのもの（`fingerprint` があればそれ。別のモデルのパスの代わり）。
fn add_surface_path_layer(s: &mut S, index: usize, fingerprint: Option<&str>) {
    use yolu_core::paths::{
        fingerprint as print_of, render_surface, Options, PathBrush, SurfacePath,
    };
    let geometry = s.view3d.model.as_ref().unwrap().geometry.clone();
    let doc = s.set_doc_mut(index);
    let mut path = SurfacePath {
        style: Default::default(),
        id: 1 + doc.layers().len() as u128,
        channel: Channel::Color,
        brush: PathBrush::default(),
        points: vec![],
        model_fingerprint: print_of(&geometry),
        material: None,
    };
    let options = Options {
        width: doc.width(),
        height: doc.height(),
        tile_size: doc.tile_size(),
        ..Options::default()
    };
    let rendered = render_surface(&path, &geometry, &options).unwrap().channels;
    if let Some(f) = fingerprint {
        path.model_fingerprint = f.into();
    }
    doc.add_path_layer("パス", yolu_core::LayerPath::Surface(path), rendered, None)
        .unwrap();
}

#[test]
fn a_model_change_names_the_3d_path_layers_that_will_not_follow_it() {
    let dir = temp_dir("paths");
    let old = character(&dir, "old.fbx");
    let new = cape_model(&dir, "new.fbx");
    let mut s = project_from(&old, 512);
    add_surface_path_layer(&mut s, 0, None);
    add_surface_path_layer(&mut s, 1, Some("別のモデルの指紋"));
    // 別のモデルへ: どちらのレイヤーもパスのままでは新しいモデルに結び付かない（画素は残る）
    configure_with(&mut s, &new);
    np(&mut s, NpAction::Submit);
    let rows = s.np.window.as_ref().unwrap().confirm.clone().unwrap().rows;
    let paths = rows
        .iter()
        .find(|r| r.left == "3D のパスレイヤー")
        .expect("パスの行");
    assert_eq!((paths.middle.as_str(), paths.warning), ("2", true));
    np(&mut s, NpAction::ConfirmApply);
    // 画素もパスも消さない
    for i in [0, 1] {
        assert!(
            s.set_doc(i).layers().iter().any(|l| l.path().is_some()),
            "パスレイヤーはそのまま"
        );
    }
    // 同じ形のモデルを読み直すと、現在のモデルの指紋のレイヤーは結び付いたまま（行に数えない）
    let mut t = project_from(&old, 512);
    add_surface_path_layer(&mut t, 0, None);
    add_surface_path_layer(&mut t, 1, Some("別のモデルの指紋"));
    np(&mut t, NpAction::OpenConfigure);
    np(&mut t, NpAction::Reload);
    wait_model(&mut t);
    np(&mut t, NpAction::Submit);
    let rows = t.np.window.as_ref().unwrap().confirm.clone().unwrap().rows;
    let paths = rows
        .iter()
        .find(|r| r.left == "3D のパスレイヤー")
        .expect("パスの行");
    assert_eq!(
        paths.middle, "1",
        "同じ形のモデルなら、その指紋のレイヤーは数えない"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_model_and_the_sizes_change_together_in_one_apply() {
    let dir = temp_dir("swap-resize");
    let old = character(&dir, "old.fbx");
    let new = cape_model(&dir, "new.fbx");
    let mut s = project_from(&old, 512);
    configure_with(&mut s, &new);
    np(&mut s, NpAction::Draft(0, DraftOp::Size(1024, 1024)));
    np(&mut s, NpAction::Submit);
    let plan = s.np.window.as_ref().unwrap().confirm.clone().unwrap();
    assert_eq!(plan.rows.len(), 1 + 1 + 3, "モデル・大きさ・セット 3 つ");
    np(&mut s, NpAction::ConfirmApply);
    assert_eq!(size_of(&s, 0), (1024, 1024));
    assert_eq!(s.model.as_ref().unwrap().name, "new");
    let _ = std::fs::remove_dir_all(dir);
}

/// Unity のシーンから来るモデル（Skin・Hair の 2 マテリアル）。
fn link_model() -> yolu_protocol::Model {
    use yolu_protocol::{MaterialInfo, MaterialKey, MeshData, Model, Submesh};
    Model {
        generation: 1,
        name: "シーン".into(),
        materials: ["Skin", "Hair"]
            .iter()
            .map(|n| MaterialInfo {
                key: MaterialKey::Material {
                    name: (*n).into(),
                    asset: None,
                },
                shader: "Standard".into(),
                textures: vec![],
                routes: vec![],
            })
            .collect(),
        meshes: vec![MeshData {
            key: "0".into(),
            name: "Body".into(),
            skinned: false,
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            submeshes: (0..2)
                .map(|m| Submesh {
                    material: m,
                    indices: vec![0, 1, 2],
                })
                .collect(),
        }],
    }
}

#[test]
fn a_live_link_model_is_never_replaced_from_the_window() {
    let link = link_model();
    let dir = temp_dir("link");
    let path = character(&dir, "c.fbx");
    let mut s = S::new(64, 64);
    let (report, shape) = s.receive_link_model(&link);
    assert!(shape.is_ok());
    assert_eq!(report.created, ["Hair"]);
    np(&mut s, NpAction::OpenConfigure);
    // モデルの行は Unity のシーンのもの: 選んでも読まない
    np(&mut s, NpAction::ChooseModel(path.clone()));
    assert!(
        s.np.window.as_ref().unwrap().model.is_none()
            && !s.np.window.as_ref().unwrap().is_loading()
    );
    np(&mut s, NpAction::Draft(0, DraftOp::Name("肌".into())));
    np(&mut s, NpAction::Submit);
    assert!(s.np.window.is_none());
    assert_eq!(set_names(&s), ["肌", "Hair"]);
    assert!(
        s.model.as_ref().unwrap().is_link(),
        "Live Link のモデルのまま"
    );
    assert!(s.np.model_file.is_none());
    // FBX を開くときは、シーンのモデルを替えずに新しいプロジェクトのウィンドウ
    np(&mut s, NpAction::OpenModel(path));
    let win = s.np.window.as_ref().unwrap();
    assert!(!win.configure && win.is_loading());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_new_project_with_a_live_link_model_makes_a_set_for_each_chosen_material() {
    let mut s = S::new(64, 64);
    let (_, shape) = s.receive_link_model(&link_model());
    assert!(shape.is_ok());
    // Ctrl+N のウィンドウは、Live Link のモデルのマテリアルの組を出し、前と同じく全部を初めから選ぶ
    np(&mut s, NpAction::OpenNew);
    {
        let win = s.np.window.as_ref().unwrap();
        assert!(win.model.is_none());
        let names: Vec<String> = win.groups(&s).iter().map(|g| g.name.clone()).collect();
        assert_eq!(names, ["Skin", "Hair"]);
        assert_eq!(win.chosen(2), [0, 1]);
    }
    np(&mut s, NpAction::Resolution(512));
    np(&mut s, NpAction::Submit);
    assert!(s.np.window.is_none(), "{:?}", s.message);
    assert_eq!(set_names(&s), ["Skin", "Hair"]);
    assert_eq!(size_of(&s, 0), (512, 512));
    assert_eq!(size_of(&s, 1), (512, 512));
    assert_eq!(s.sets.get(0).unwrap().bound, Some(0));
    assert_eq!(s.sets.get(1).unwrap().bound, Some(1));
    assert!(
        s.model.as_ref().unwrap().is_link(),
        "Live Link のモデルのまま"
    );
    assert!(s.message.contains("シーン"), "{}", s.message);
    // 選び直せば、選んだマテリアルだけ
    np(&mut s, NpAction::OpenNew);
    np(&mut s, NpAction::Material(0, false));
    np(&mut s, NpAction::Submit);
    assert_eq!(set_names(&s), ["Hair"]);
    assert_eq!(s.sets.get(0).unwrap().bound, Some(1));
    // モデルを選べば、その FBX のマテリアル（Live Link のモデルは見せない）
    let dir = temp_dir("link-new");
    let path = character(&dir, "c.fbx");
    np(&mut s, NpAction::OpenNew);
    np(&mut s, NpAction::ChooseModel(path));
    wait_model(&mut s);
    let names: Vec<String> = {
        let win = s.np.window.as_ref().unwrap();
        win.groups(&s).iter().map(|g| g.name.clone()).collect()
    };
    assert_eq!(names, ["Skin", "Hair", "Eye"]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_model_dropped_while_the_list_is_up_drops_the_list_and_a_stale_list_is_never_applied() {
    let dir = temp_dir("confirm-drop");
    let old = character(&dir, "old.fbx");
    let new = cape_model(&dir, "new.fbx");
    let again = cape_model(&dir, "again.fbx");
    let mut s = project_from(&old, 512);
    configure_with(&mut s, &new);
    np(&mut s, NpAction::Submit);
    assert!(s.np.window.as_ref().unwrap().confirm.is_some());
    // 一覧が出ている間に、別の FBX を落とす（ファイルを選ぶボタンは止まるが、落とした分はウィンドウに届く）
    np(&mut s, NpAction::OpenModel(again.clone()));
    {
        let win = s.np.window.as_ref().unwrap();
        assert!(win.confirm.is_none(), "古い一覧は消える");
        assert_eq!(win.model.as_deref(), Some(again.as_path()));
        assert!(win.is_loading());
    }
    wait_model(&mut s);
    // 読み終えたあとに「適用」を押しても、見せていない一覧のまま適用しない
    np(&mut s, NpAction::ConfirmApply);
    assert!(s.np.window.is_some(), "適用されない");
    assert_eq!(s.model.as_ref().unwrap().name, "old");
    // 出し直した一覧を見て決める
    np(&mut s, NpAction::Submit);
    let plan =
        s.np.window
            .as_ref()
            .unwrap()
            .confirm
            .clone()
            .expect("出し直す");
    assert_eq!(plan.rows[0].left, "old → again");
    np(&mut s, NpAction::ConfirmApply);
    assert!(s.np.window.is_none(), "{:?}", s.message);
    assert_eq!(s.model.as_ref().unwrap().name, "again");
    // 取り消し・外す・読み直すでも、一覧は消える
    for step in [NpAction::ClearModel, NpAction::Reload] {
        let mut t = project_from(&old, 512);
        configure_with(&mut t, &new);
        np(&mut t, NpAction::Submit);
        assert!(t.np.window.as_ref().unwrap().confirm.is_some());
        np(&mut t, step.clone());
        assert!(
            t.np.window.as_ref().unwrap().confirm.is_none(),
            "{step:?} で一覧が残った"
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_list_that_no_longer_matches_the_window_is_shown_again_instead_of_applied() {
    let dir = temp_dir("confirm-stale");
    let old = character(&dir, "old.fbx");
    let mut s = project_from(&old, 512);
    np(&mut s, NpAction::OpenConfigure);
    np(&mut s, NpAction::Draft(0, DraftOp::Size(1024, 1024)));
    np(&mut s, NpAction::Submit);
    let shown = s.np.window.as_ref().unwrap().confirm.clone().unwrap();
    // 確かめの間に、一覧に無い大きさの変更が入った
    np(&mut s, NpAction::Draft(1, DraftOp::Size(1024, 1024)));
    np(&mut s, NpAction::ConfirmApply);
    assert!(s.np.window.is_some(), "見せていない変更は適用しない");
    assert_eq!(size_of(&s, 0), (512, 512));
    assert_eq!(size_of(&s, 1), (512, 512));
    let now =
        s.np.window
            .as_ref()
            .unwrap()
            .confirm
            .clone()
            .expect("今の計画の一覧を出し直す");
    assert_ne!(now, shown);
    assert_eq!(now.rows.len(), 2);
    np(&mut s, NpAction::ConfirmApply);
    assert!(s.np.window.is_none());
    assert_eq!(size_of(&s, 0), (1024, 1024));
    assert_eq!(size_of(&s, 1), (1024, 1024));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_model_added_to_a_project_without_one_is_listed_by_its_name_alone() {
    let dir = temp_dir("first-model");
    let new = cape_model(&dir, "new.fbx");
    for lang in Lang::ALL {
        let mut s = S::new_in(64, 64, lang);
        configure_with(&mut s, &new);
        np(&mut s, NpAction::Submit);
        let plan = s.np.window.as_ref().unwrap().confirm.clone().unwrap();
        assert_eq!(
            plan.rows[0].left, "new",
            "前のモデルの名前の代わりの文字を出さない"
        );
        assert_eq!(plan.rows[0].right, lang.pick("追加", "Add"));
        for row in &plan.rows {
            for text in [&row.left, &row.middle, &row.right] {
                assert!(text != "なし" && text != "None", "{row:?}");
            }
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_configuration_window_closes_when_the_sets_change_underneath_it() {
    use yolu_protocol::{MaterialInfo, MaterialKey, MeshData, Model, Submesh};
    let mut s = S::new(64, 64);
    np(&mut s, NpAction::OpenConfigure);
    s.poll_newproject();
    assert!(s.np.window.is_some(), "何も変わらなければ開いたまま");
    // Live Link のモデルが来て、マテリアルごとのセットができた
    let model = Model {
        generation: 1,
        name: "シーン".into(),
        materials: ["A", "B"]
            .iter()
            .map(|n| MaterialInfo {
                key: MaterialKey::Material {
                    name: (*n).into(),
                    asset: None,
                },
                shader: String::new(),
                textures: vec![],
                routes: vec![],
            })
            .collect(),
        meshes: vec![MeshData {
            key: "0".into(),
            name: "M".into(),
            skinned: false,
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            submeshes: (0..2)
                .map(|m| Submesh {
                    material: m,
                    indices: vec![0, 1, 2],
                })
                .collect(),
        }],
    };
    let _ = s.receive_link_model(&model);
    s.poll_newproject();
    assert!(s.np.window.is_none());
    assert!(
        s.message.contains("プロジェクトが変わった"),
        "{}",
        s.message
    );
    // 新規プロジェクトのウィンドウは、ウィンドウの外の変更に影響されない
    np(&mut s, NpAction::OpenNew);
    let _ = s.receive_link_model(&Model {
        generation: 2,
        ..model
    });
    s.poll_newproject();
    assert!(s.np.window.is_some());
}

#[test]
fn opening_an_fbx_picks_the_window_that_fits_the_project() {
    let dir = temp_dir("route");
    let path = character(&dir, "c.fbx");
    // 何も触っていない: 新規プロジェクトのウィンドウに、そのモデル
    let mut s = S::new(64, 64);
    assert!(s.is_pristine());
    np(&mut s, NpAction::OpenModel(path.clone()));
    {
        let win = s.np.window.as_ref().unwrap();
        assert!(!win.configure && win.is_loading() && win.model.as_deref() == Some(path.as_path()));
    }
    // ウィンドウが開いていれば、そのウィンドウのモデルを替える
    let other = cape_model(&dir, "other.fbx");
    np(&mut s, NpAction::OpenModel(other.clone()));
    assert_eq!(
        s.np.window.as_ref().unwrap().model.as_deref(),
        Some(other.as_path())
    );
    wait_model(&mut s);
    np(&mut s, NpAction::Close);
    // 描いたあと: 構成のウィンドウで、そのモデルに替える下書き
    paint_dot(&mut s, 0, (1, 1), Rgba8::new(1, 1, 1, 255));
    assert!(!s.is_pristine());
    np(&mut s, NpAction::OpenModel(path.clone()));
    {
        let win = s.np.window.as_ref().unwrap();
        assert!(win.configure && win.is_loading() && win.model.as_deref() == Some(path.as_path()));
    }
    wait_model(&mut s);
    // モデルの無かったセット（仮のスロット 0）は、最初のスロットのマテリアルに付く（描いた画素はそのまま）
    {
        let win = s.np.window.as_ref().unwrap();
        assert_eq!(win.drafts.len(), 1);
        assert_eq!(win.drafts[0].material, Some(0));
        assert_eq!(
            yolu_app::newproject::unused_groups(&s, win),
            2,
            "Hair と Eye にセットが無い"
        );
    }
    np(&mut s, NpAction::Submit);
    np(&mut s, NpAction::ConfirmApply);
    assert_eq!(
        s.sets.get(0).unwrap().material,
        MaterialRef::Material {
            name: "Skin".into(),
            asset: None
        }
    );
    assert_eq!(s.sets.get(0).unwrap().bound, Some(0));
    assert_eq!(pixel_of(&s, 0, (1, 1)), Rgba8::new(1, 1, 1, 255));
    // 保存したファイルがあるプロジェクトも触ったことになる
    let mut t = S::new(64, 64);
    let ylp = dir.join("p.ylp");
    t.apply(Action::SaveProjectAs(ylp));
    assert!(!t.is_pristine());
    let _ = std::fs::remove_dir_all(dir);
}

// ───────── 保存と開き直し ─────────

fn read_file(path: &Path) -> yolu_io::Project {
    yolu_io::SaveTarget::open(path).unwrap().0
}

#[test]
fn a_saved_project_brings_the_pose_of_its_model_back_when_the_model_is_read_again() {
    use yolu_core::glam::Quat;
    let dir = temp_dir("reopen-pose");
    let model = character(&dir.join("models"), "c.fbx");
    let ylp = dir.join("p.ylp");
    let mut s = project_from(&model, 256);
    // モデルの骨を 1 本回す（読んだ FBX の骨のうち、最初のもの）
    let posed = {
        let session = s
            .view3d
            .pose
            .session
            .as_ref()
            .expect("モデルのポーズのセッション");
        assert!(!session.rig.bones().is_empty());
        let mut p = session.pose().clone();
        p.locals[0].rotation = Quat::from_rotation_y(0.7);
        p
    };
    yolu_app::view3d::pose::set_pose(&mut s.view3d, posed.clone()).unwrap();
    yolu_app::view3d::pose::sync_modified(&mut s);
    assert!(s.modified, "ポーズを変えると保存が要る");
    s.apply(Action::SaveProjectAs(ylp.clone()));
    assert!(s.message.contains("保存しました"), "{}", s.message);
    assert!(read_file(&ylp).pose().unwrap().is_some());
    assert_eq!(read_file(&ylp).info().format, 7, "ポーズは形式を上げない");
    // 開き直すと、モデルを読み終えたところでポーズが戻る（取り消しの段にも変更の印にもならない）
    let mut t = S::new(64, 64);
    t.apply(Action::OpenProject(ylp.clone()));
    assert!(t.view3d.pose.session.is_none(), "読み終えるまでは無い");
    wait_reopen(&mut t);
    let session = t.view3d.pose.session.as_ref().unwrap();
    assert!(session.is_posed(), "{}", t.message);
    assert!(
        session.pose().locals[0]
            .rotation
            .dot(posed.locals[0].rotation)
            .abs()
            > 0.999_999
    );
    assert!(!session.can_undo());
    assert!(t.message.contains("ポーズを戻しました"), "{}", t.message);
    yolu_app::view3d::pose::sync_modified(&mut t);
    assert!(!t.modified, "{}", t.message);
    // 別のモデル（骨の名前が違う）を指すように参照を替えたファイルでは、合わない項目を飛ばして理由を残し、ポーズは変えない
    let other = write_fbx(
        &dir.join("other"),
        "o.fbx",
        &ascii_fbx(
            &["Cloth"],
            &[mesh("Wing", &[Some(0)]), mesh("Tail", &[Some(0)])],
        ),
    );
    let swapped = read_file(&ylp)
        .with_view_model(Some(&yolu_app::newproject::relative_model_path(
            &other, &ylp,
        )))
        .unwrap();
    std::fs::write(&ylp, swapped.to_bytes().unwrap()).unwrap();
    let mut u = S::new(64, 64);
    u.apply(Action::OpenProject(ylp.clone()));
    wait_reopen(&mut u);
    let session = u.view3d.pose.session.as_ref().unwrap();
    assert!(
        !session.is_posed(),
        "合わないポーズは当てない: {}",
        u.message
    );
    assert!(!session.preset_notes.is_empty(), "飛ばした項目の理由が残る");
    assert!(
        u.message.contains("合わない") || u.message.contains("合うボーンがありません"),
        "{}",
        u.message
    );
}

/// プロジェクトの構成のウィンドウで、モデルを選び直す（`reload` なら読み直す）。確かめが出たら適用する。
fn configure_model(s: &mut S, choose: Option<&Path>) {
    np(s, NpAction::OpenConfigure);
    match choose {
        Some(path) => np(s, NpAction::ChooseModel(path.to_path_buf())),
        None => np(s, NpAction::Reload),
    }
    wait_model(s);
    np(s, NpAction::Submit);
    if s.np.window.as_ref().is_some_and(|w| w.confirm.is_some()) {
        np(s, NpAction::ConfirmApply);
    }
    assert!(
        s.np.window.is_none(),
        "{:?}",
        s.np.window.as_ref().map(|w| w.error.clone())
    );
}

#[test]
fn the_pose_in_the_file_comes_back_when_the_model_is_chosen_again_in_the_configuration_and_is_not_lost_on_save(
) {
    use yolu_core::glam::Quat;
    let dir = temp_dir("configure-pose");
    let model = character(&dir.join("models"), "c.fbx");
    let ylp = dir.join("p.ylp");
    let mut s = project_from(&model, 256);
    let posed = {
        let session = s
            .view3d
            .pose
            .session
            .as_ref()
            .expect("モデルのポーズのセッション");
        let mut p = session.pose().clone();
        p.locals[0].rotation = Quat::from_rotation_y(0.7);
        p
    };
    yolu_app::view3d::pose::set_pose(&mut s.view3d, posed.clone()).unwrap();
    s.apply(Action::SaveProjectAs(ylp.clone()));
    assert!(s.message.contains("保存しました"), "{}", s.message);
    let stored = read_file(&ylp).pose().unwrap().expect("保存したポーズ");
    let rotated = |t: &S| {
        let session = t.view3d.pose.session.as_ref().expect("ポーズのセッション");
        session.is_posed()
            && session.pose().locals[0]
                .rotation
                .dot(posed.locals[0].rotation)
                .abs()
                > 0.999_999
    };
    // モデルのファイルを動かしてから開く: 見つからず、ポーズのセッションは無い
    let moved = dir.join("moved");
    std::fs::rename(dir.join("models"), &moved).unwrap();
    let mut t = S::new(64, 64);
    t.apply(Action::OpenProject(ylp.clone()));
    wait_reopen(&mut t);
    assert!(t.message.contains("が見つかりません"), "{}", t.message);
    assert!(t.view3d.pose.session.is_none());
    // 構成のウィンドウで動かした先のモデルを選び直すと、ファイルのポーズが戻り、そのことが結果の文に出る
    configure_model(&mut t, Some(&moved.join("c.fbx")));
    assert!(rotated(&t), "{}", t.message);
    assert!(t.message.contains("ポーズを戻しました"), "{}", t.message);
    // そのまま保存しても pose.json は消えず、同じ中身
    t.apply(Action::SaveProject);
    assert!(t.message.contains("保存しました"), "{}", t.message);
    assert_eq!(
        read_file(&ylp).pose().unwrap().as_ref(),
        Some(&stored),
        "ファイルのポーズが残る"
    );
    // 読み直しでも戻る（ポーズのセッションは新しい休みの形から始まる）
    let mut u = S::new(64, 64);
    u.apply(Action::OpenProject(ylp.clone()));
    wait_reopen(&mut u);
    assert!(rotated(&u), "{}", u.message);
    configure_model(&mut u, None);
    assert!(rotated(&u), "読み直したモデルにも戻す: {}", u.message);
    assert!(u.message.contains("ポーズを戻しました"), "{}", u.message);
    u.apply(Action::SaveProject);
    assert_eq!(read_file(&ylp).pose().unwrap().as_ref(), Some(&stored));
    // ファイルにポーズが無いプロジェクトでは、何も言わない
    let plain = dir.join("plain.ylp");
    let mut v = project_from(&moved.join("c.fbx"), 256);
    v.apply(Action::SaveProjectAs(plain.clone()));
    let mut w = S::new(64, 64);
    w.apply(Action::OpenProject(plain));
    wait_reopen(&mut w);
    configure_model(&mut w, None);
    assert!(!w.message.contains("ポーズ"), "{}", w.message);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_saved_project_remembers_the_model_file_and_reads_it_again_when_opened() {
    let dir = temp_dir("reopen");
    let model = character(&dir.join("models"), "c.fbx");
    let ylp = dir.join("art").join("p.ylp");
    std::fs::create_dir_all(ylp.parent().unwrap()).unwrap();
    let mut s = project_from(&model, 512);
    paint_dot(&mut s, 1, (7, 7), Rgba8::new(5, 6, 7, 255));
    s.apply(Action::SaveProjectAs(ylp.clone()));
    assert!(s.message.contains("保存しました"), "{}", s.message);
    let saved = read_file(&ylp);
    assert_eq!(
        saved.view_model().unwrap().as_deref(),
        Some("../models/c.fbx"),
        "プロジェクトからの相対"
    );
    assert_eq!(saved.info().format, 7, "形式は変えない");
    // 開き直す（別の状態で）
    let mut t = S::new(64, 64);
    t.apply(Action::OpenProject(ylp.clone()));
    assert!(t.message.contains("開きました"), "{}", t.message);
    assert!(t.np.reopening.is_some(), "モデルは別のスレッドで読む");
    assert!(t.model.is_none(), "読み終えるまでは結び付けない");
    assert_eq!(set_names(&t), ["Skin", "Hair", "Eye"]);
    wait_reopen(&mut t);
    assert_eq!(t.model.as_ref().unwrap().name, "c");
    assert_eq!(
        set_names(&t),
        ["Skin", "Hair", "Eye"],
        "モデルのマテリアルにセットを増やさない"
    );
    assert_eq!(
        t.sets.iter().map(|x| x.bound).collect::<Vec<_>>(),
        [Some(0), Some(1), Some(2)]
    );
    assert_eq!(
        t.np.model_file.as_ref().map(|p| p.canonicalize().unwrap()),
        Some(model.canonicalize().unwrap())
    );
    assert!(t.view3d.model.is_some());
    assert!(t.message.contains("モデル: c"), "{}", t.message);
    assert_eq!(
        pixel_of(&t, 1, (7, 7)),
        Rgba8::new(5, 6, 7, 255),
        "画素は正本のまま"
    );
    // 保存し直しても参照は残る
    t.apply(Action::SaveProject);
    assert_eq!(
        read_file(&ylp).view_model().unwrap().as_deref(),
        Some("../models/c.fbx")
    );
    // 参照の無いプロジェクトを開くと、前のモデルは引きずらない
    let plain = dir.join("plain.ylp");
    let mut u = S::new(64, 64);
    u.apply(Action::SaveProjectAs(plain.clone()));
    t.apply(Action::OpenProject(plain));
    assert!(t.model.is_none() && t.np.model_file.is_none() && t.view3d.pose.session.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_missing_model_is_told_and_its_reference_survives_a_save() {
    let dir = temp_dir("missing");
    let model = character(&dir, "c.fbx");
    let ylp = dir.join("p.ylp");
    let mut s = project_from(&model, 512);
    s.apply(Action::SaveProjectAs(ylp.clone()));
    std::fs::remove_file(&model).unwrap();
    let mut t = S::new(64, 64);
    t.apply(Action::OpenProject(ylp.clone()));
    // ファイルがあるかの確かめも別のスレッド（画面のスレッドは、遅い共有でも固まらないよう、モデルのファイルに触らない）
    assert!(t.np.reopening.is_some() && t.model.is_none());
    wait_reopen(&mut t);
    // 注意はログの行（1 行に切る）に残る。開いた知らせの後ろに続けず、理由だけの単独の知らせにする（長い開いた知らせの後ろで切れない）
    assert_eq!(t.message, "モデル「c.fbx」が見つかりません。");
    let logged = t.notice_log.entries().last().expect("ログに残る");
    assert_eq!(logged.notice.text, t.message);
    assert_eq!(logged.notice.kind, yolu_app::notice::Kind::Warning);
    assert!(t.np.reopening.is_none() && t.model.is_none());
    assert!(t.np.model_file.is_some(), "参照は残す");
    // 別の場所へ保存しても、参照は（その場所からの相対で）残る
    let other = dir.join("sub").join("q.ylp");
    std::fs::create_dir_all(other.parent().unwrap()).unwrap();
    t.apply(Action::SaveProjectAs(other.clone()));
    assert_eq!(
        read_file(&other).view_model().unwrap().as_deref(),
        Some("../c.fbx")
    );
    // 見つかるようになれば、構成の「読み直す」で読める
    character(&dir, "c.fbx");
    np(&mut t, NpAction::OpenConfigure);
    np(&mut t, NpAction::Reload);
    wait_model(&mut t);
    assert!(matches!(
        t.np.window.as_ref().unwrap().prep,
        Prep::Ready { .. }
    ));
    // 英語
    let mut e = S::new_in(64, 64, Lang::En);
    e.apply(Action::OpenProject(ylp));
    // 開いたときは見つかったので読む。消してから開き直す
    wait_reopen(&mut e);
    std::fs::remove_file(dir.join("c.fbx")).unwrap();
    let mut f = S::new_in(64, 64, Lang::En);
    f.apply(Action::OpenProject(dir.join("p.ylp")));
    wait_reopen(&mut f);
    assert!(
        f.message.contains("The model \"c.fbx\" was not found."),
        "{}",
        f.message
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// 前の版の .ylp を退避のフォルダー（`<名前>.ylp-backups~`）に作る: モデルは `models/` に置き、保存を 2 度（2 度目は絵を 1 点足す）。
/// 戻りは（フォルダー、モデルのファイル、元の .ylp、退避）。
fn project_with_a_backup(tag: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let dir = temp_dir(tag);
    let model = character(&dir.join("work").join("models"), "c.fbx");
    let ylp = dir.join("work").join("p.ylp");
    let mut s = project_from(&model, 512);
    s.apply(Action::SaveProjectAs(ylp.clone()));
    paint_dot(&mut s, 0, (3, 3), Rgba8::new(10, 20, 30, 255));
    s.apply(Action::SaveProject);
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let kept = yolu_io::backups(&ylp).unwrap();
    assert_eq!(kept.len(), 1, "前の版が退避に残る");
    (dir, model, ylp, kept[0].clone())
}

/// 退避のフォルダーの中の .ylp を開く: view.json のモデルの道は元の .ylp から相対に書いてあるので、退避の中から開いても同じモデルを読み、
/// 立方体に落ちない。別の場所へ保存した .ylp も、その場所から同じモデルを辿る。本体から開くのは今までどおり。
#[test]
fn a_backup_opens_with_the_model_of_the_file_it_backs_up_and_saved_elsewhere_still_finds_it() {
    let (dir, model, ylp, backup) = project_with_a_backup("backup-model");
    assert_eq!(
        read_file(&backup).view_model().unwrap().as_deref(),
        Some("models/c.fbx")
    );
    // 本体から開く
    let mut main = S::new(64, 64);
    main.apply(Action::OpenProject(ylp.clone()));
    wait_reopen(&mut main);
    assert!(main.model.is_some(), "{}", main.message);
    // 退避から開く: モデルを読み、警告は出ない
    let mut t = S::new(64, 64);
    t.apply(Action::OpenProject(backup.clone()));
    wait_reopen(&mut t);
    assert!(t.model.is_some(), "{}", t.message);
    assert!(t.view3d.full_model().is_some_and(|m| !m.demo));
    assert!(!t.message.contains("見つかりません"), "{}", t.message);
    assert_eq!(
        t.np.model_file.as_deref().and_then(Path::file_name),
        model.file_name()
    );
    assert!(t.np.model_file.as_deref().is_some_and(Path::is_file));
    assert!(t.notice_log.is_empty(), "警告も失敗も無い");
    // 別の場所へ保存: その場所からの相対で書き直し、開き直してもモデルを読む
    let other = dir.join("restored").join("q.ylp");
    std::fs::create_dir_all(other.parent().unwrap()).unwrap();
    t.apply(Action::SaveProjectAs(other.clone()));
    assert!(t.message.starts_with("保存しました"), "{}", t.message);
    assert_eq!(
        read_file(&other).view_model().unwrap().as_deref(),
        Some("../work/models/c.fbx")
    );
    let mut u = S::new(64, 64);
    u.apply(Action::OpenProject(other));
    wait_reopen(&mut u);
    assert!(
        u.view3d.full_model().is_some_and(|m| !m.demo),
        "{}",
        u.message
    );
    assert!(u.notice_log.is_empty(), "{}", u.message);
    // 英語でも同じ
    let mut e = S::new_in(64, 64, Lang::En);
    e.apply(Action::OpenProject(backup));
    wait_reopen(&mut e);
    assert!(
        e.view3d.full_model().is_some_and(|m| !m.demo),
        "{}",
        e.message
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// 退避（前の版）を開いた文書は保存先を持たない（新しい文書と同じ）。「保存」は名前を付けて保存になり、始まりの場所は元の .ylp のフォルダー、
/// 名前の候補は元の .ylp の名前。退避のファイルは変わらず、退避のフォルダーの中へは何も書かない。元の名前へ保存すれば、元が置き換わり、
/// 前の元は今の保存の決まりで退避に回る。
#[test]
fn a_backup_is_opened_without_a_destination_and_save_asks_where_without_touching_it() {
    let (dir, _model, ylp, backup) = project_with_a_backup("backup-nodest");
    let backup_before = std::fs::read(&backup).unwrap();
    let origin_before = std::fs::read(&ylp).unwrap();
    let kept_before = yolu_io::backups(&ylp).unwrap().len();
    let backup_folder = backup.parent().unwrap().to_path_buf();
    let names_before = std::fs::read_dir(&backup_folder).unwrap().count();
    for lang in [Lang::Ja, Lang::En] {
        let mut t = S::new_in(64, 64, lang);
        t.apply(Action::OpenProject(backup.clone()));
        wait_reopen(&mut t);
        assert!(t.project.as_ref().is_some_and(|p| !p.is_file()));
        assert_eq!(t.project_name, "p", "題名は元の .ylp の名前");
        assert!(!t.modified);
        assert_eq!(t.save_folder.as_deref(), ylp.parent());
        assert!(
            t.message.contains(lang.pick("（退避）", "(backup)")),
            "{}",
            t.message
        );
        // 保存 → 名前を付けて保存。退避は変わらず、退避のフォルダーに何も増えない
        t.apply(Action::SaveProject);
        assert_eq!(t.dialog_request, Some(DialogRequest::SaveAs));
        assert_eq!(std::fs::read(&backup).unwrap(), backup_before);
        assert_eq!(
            std::fs::read_dir(&backup_folder).unwrap().count(),
            names_before
        );
        t.dialog_request = None;
    }
    // 元の名前へ保存する（ウィンドウが確かめて返した道）: 元が置き換わり、前の元が退避に回る
    let mut t = S::new(64, 64);
    t.apply(Action::OpenProject(backup.clone()));
    wait_reopen(&mut t);
    t.apply(Action::SaveProjectAs(ylp.clone()));
    assert!(t.message.starts_with("保存しました"), "{}", t.message);
    let kept = yolu_io::backups(&ylp).unwrap();
    assert_eq!(kept.len(), kept_before + 1);
    assert!(
        kept.iter()
            .any(|b| std::fs::read(b).unwrap() == origin_before),
        "前の元が退避に回る"
    );
    assert_ne!(std::fs::read(&ylp).unwrap(), origin_before);
    assert_eq!(
        std::fs::read(&backup).unwrap(),
        backup_before,
        "開いた退避は変わらない"
    );
    // 保存したら、行き先のある文書になる
    assert!(t.project.as_ref().is_some_and(|p| p.is_file()));
    assert_eq!(t.save_folder, None);
    assert_eq!(
        read_file(&ylp).view_model().unwrap().as_deref(),
        Some("models/c.fbx")
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// 前の版のアプリが退避の場所を基準に書き直したパス（退避から開いて保存した退避）は、元のファイルの場所から解いた 1 か所だけを探すので、
/// 見つからない（退避の場所から解いた先に同じ名前のファイルがあっても、別のモデルを黙って拾わない）。参照は残り、モデルを選び直せば開ける。
#[test]
fn a_model_path_written_from_the_backup_folder_is_searched_only_from_the_original_folder() {
    let (dir, model, ylp, backup) = project_with_a_backup("backup-legacy");
    // 退避の場所から見た相対のパスに書き直した退避。元の場所から辿ると dir/models/c.fbx で、そこには無い
    // （退避の場所から辿ると dir/work/models/c.fbx で、そこにはファイルがある）
    let legacy = read_file(&backup)
        .with_view_model(Some("../models/c.fbx"))
        .unwrap();
    let path = backup.with_file_name("p-20260102T000000000Z.ylp");
    std::fs::write(&path, legacy.to_bytes().unwrap()).unwrap();
    assert!(!dir.join("models").exists() && model.is_file());
    let mut t = S::new(64, 64);
    t.apply(Action::OpenProject(path));
    wait_reopen(&mut t);
    assert!(t.model.is_none() && t.view3d.full_model().is_none_or(|m| m.demo));
    assert_eq!(t.message, "モデル「c.fbx」が見つかりません。");
    let logged = t.notice_log.entries().last().expect("ログに残る");
    assert_eq!(logged.notice.text, t.message);
    assert_eq!(
        t.np.model_file.as_deref().and_then(Path::file_name),
        model.file_name(),
        "参照は残す"
    );
    assert!(
        t.np.model_file
            .as_deref()
            .is_some_and(|p| p.starts_with(&dir) && !p.exists()),
        "参照は元のファイルの場所から解いた道（ファイルは無い）: {:?}",
        t.np.model_file
    );
    let _ = (ylp, std::fs::remove_dir_all(dir));
}

/// 前の版のアプリが作った、退避の中の退避（退避を開いて上書き保存した結果）も、元のファイルまでたどって同じモデルを読む。
#[test]
fn a_backup_of_a_backup_opens_with_the_model_of_the_original_file() {
    let (dir, _model, _ylp, backup) = project_with_a_backup("backup-nested");
    let nested_folder = backup.with_file_name(format!(
        "{}-backups~",
        backup.file_name().unwrap().to_string_lossy()
    ));
    std::fs::create_dir_all(&nested_folder).unwrap();
    let stem = backup.file_stem().unwrap().to_string_lossy().into_owned();
    let nested = nested_folder.join(format!("{stem}-20260102T000000000Z.ylp"));
    std::fs::copy(&backup, &nested).unwrap();
    assert_eq!(
        yolu_io::backup_origin(&nested).as_deref(),
        Some(backup.as_path())
    );
    let mut t = S::new(64, 64);
    t.apply(Action::OpenProject(nested));
    wait_reopen(&mut t);
    assert!(
        t.view3d.full_model().is_some_and(|m| !m.demo),
        "{}",
        t.message
    );
    assert!(t.notice_log.is_empty(), "{}", t.message);
    let _ = std::fs::remove_dir_all(dir);
}

/// 退避の中の .ylp のモデルが元の場所にも退避の場所にも無いときは、本体と同じく理由だけの警告を出し、参照は残す（保存で失わない）。
#[test]
fn a_backup_with_a_missing_model_says_so_and_keeps_the_reference() {
    let (dir, model, _ylp, backup) = project_with_a_backup("backup-missing");
    std::fs::remove_file(&model).unwrap();
    let mut t = S::new(64, 64);
    t.apply(Action::OpenProject(backup));
    wait_reopen(&mut t);
    assert!(t.model.is_none());
    assert_eq!(t.message, "モデル「c.fbx」が見つかりません。");
    let logged = t.notice_log.entries().last().expect("ログに残る");
    assert_eq!(logged.notice.text, t.message);
    // 参照は元のファイルの場所を指したまま、別の場所へ保存してもその場所からの相対で残る
    let other = dir.join("restored").join("q.ylp");
    std::fs::create_dir_all(other.parent().unwrap()).unwrap();
    t.apply(Action::SaveProjectAs(other.clone()));
    assert_eq!(
        read_file(&other).view_model().unwrap().as_deref(),
        Some("../work/models/c.fbx")
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// 保存した .ylp の view.json のモデルのパスを `stored` に書き換えた .ylp（別の名前で書く）。
fn project_with_stored_model(dir: &Path, stored: &str) -> PathBuf {
    let model = character(dir, "c.fbx");
    let ylp = dir.join("p.ylp");
    let mut s = project_from(&model, 512);
    s.apply(Action::SaveProjectAs(ylp.clone()));
    let edited = read_file(&ylp).with_view_model(Some(stored)).unwrap();
    let out = dir.join("edited.ylp");
    std::fs::write(&out, edited.to_bytes().unwrap()).unwrap();
    out
}

#[test]
fn a_model_on_the_network_is_kept_as_a_reference_and_never_touched_when_the_project_opens() {
    // 細工した .ylp のモデルのパスが UNC でも、開くだけで外のホストへ触らない（Windows の認証を送らない・応答しない共有で固まらない）。
    // 参照だけ残し、読み込みの仕事も（ファイルの確かめも）始めない
    let dir = temp_dir("unc-open");
    let stored = "//nas.invalid/share/models/c.fbx";
    let ylp = project_with_stored_model(&dir, stored);
    for (lang, text) in [
        (
            Lang::Ja,
            "ネットワーク上のモデル「c.fbx」は自動では読みません。",
        ),
        (
            Lang::En,
            "The model \"c.fbx\" on the network is not read automatically.",
        ),
    ] {
        let mut t = S::new_in(64, 64, lang);
        t.apply(Action::OpenProject(ylp.clone()));
        assert!(t.np.reopening.is_none(), "確かめも読み込みも始めない");
        assert!(t.model.is_none() && t.view3d.pose.session.is_none());
        assert_eq!(
            t.np.model_file.as_deref(),
            Some(Path::new(stored)),
            "参照は残す"
        );
        assert!(t.message.contains(text), "{}", t.message);
        assert!(
            t.message.starts_with(lang.pick("開きました", "Opened")),
            "{}",
            t.message
        );
        assert_eq!(t.sets.len(), 3, "セットは開く");
    }
    // 書き直しても参照は消えない（保存し直すと、同じ文字列が残る。UNC を相対に直さない。どの OS でも）
    {
        let mut t = S::new(64, 64);
        t.apply(Action::OpenProject(ylp));
        t.apply(Action::SaveProjectAs(dir.join("again.ylp")));
        assert_eq!(
            read_file(&dir.join("again.ylp"))
                .view_model()
                .unwrap()
                .as_deref(),
            Some(stored)
        );
    }
    // バックスラッシュで書かれた UNC も、'/' 区切りの同じ参照として残る（OS によらず、別の相対のファイル名にならない）
    {
        let back = project_with_stored_model(&dir, "\\\\nas.invalid\\share\\models\\c.fbx");
        let mut t = S::new(64, 64);
        t.apply(Action::OpenProject(back));
        t.apply(Action::SaveProjectAs(dir.join("again2.ylp")));
        assert_eq!(
            read_file(&dir.join("again2.ylp"))
                .view_model()
                .unwrap()
                .as_deref(),
            Some(stored)
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_model_reference_on_the_network_is_not_touched_by_a_new_project_window_either() {
    // 開くときに参照だけ残したネットワークのパスを、そのあと「新規プロジェクト」を選んだだけで確かめたり読み始めたりしない。
    // Unix では先頭が `//` のパスが同じ場所のローカルのファイルを指すので、ファイルがあっても読み始めないことまで確かめられる
    let dir = temp_dir("unc-new");
    let local = character(&dir, "c.fbx");
    let mut unc = vec![String::from("//nas.invalid/share/models/c.fbx")];
    if cfg!(unix) {
        unc.push(format!("/{}", local.display()));
    }
    for stored in unc {
        let ylp = project_with_stored_model(&dir, &stored);
        let mut t = S::new(64, 64);
        t.apply(Action::OpenProject(ylp));
        assert_eq!(
            t.np.model_file.as_deref(),
            Some(Path::new(&stored)),
            "参照は残る"
        );
        assert!(t.np.reopening.is_none(), "{stored}");
        np(&mut t, NpAction::OpenNew);
        let win = t.np.window.as_ref().expect("ウィンドウは開く");
        assert!(
            !win.is_loading() && win.model_path().is_none() && matches!(win.prep, Prep::Idle),
            "{stored}: ネットワークの参照を初めのモデルにして読み始めた"
        );
        assert_eq!(
            t.np.model_file.as_deref(),
            Some(Path::new(&stored)),
            "参照は消えない"
        );
    }
    // 対照: 同じファイルをローカルの絶対のパスで参照していれば、新規のウィンドウの初めのモデルとして読む
    let ylp = project_with_stored_model(&dir, &local.to_string_lossy());
    let mut t = S::new(64, 64);
    t.apply(Action::OpenProject(ylp));
    wait_reopen(&mut t);
    np(&mut t, NpAction::OpenNew);
    assert!(
        t.np.window.as_ref().unwrap().model_path().is_some(),
        "ローカルのモデルは初めのモデルになる"
    );
    wait_model(&mut t);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_relative_or_local_model_reference_is_still_read_when_the_project_opens() {
    let dir = temp_dir("rel-open");
    let ylp = project_with_stored_model(&dir, "c.fbx");
    let mut t = S::new(64, 64);
    t.apply(Action::OpenProject(ylp));
    assert!(t.np.reopening.is_some(), "相対のパスは確かめて読む");
    wait_reopen(&mut t);
    assert_eq!(t.model.as_ref().unwrap().name, "c");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_broken_model_file_is_told_when_the_project_opens_and_the_sets_stay() {
    let dir = temp_dir("broken-reopen");
    let model = character(&dir, "c.fbx");
    let ylp = dir.join("p.ylp");
    let mut s = project_from(&model, 512);
    s.apply(Action::SaveProjectAs(ylp.clone()));
    std::fs::write(&model, "壊れた FBX").unwrap();
    let mut t = S::new(64, 64);
    t.apply(Action::OpenProject(ylp));
    wait_reopen(&mut t);
    // 失敗はログの行に残るので、開いた知らせに続けず、理由だけの単独の知らせ
    assert!(
        t.message.starts_with("モデル「c.fbx」を読み込めません（"),
        "{}",
        t.message
    );
    let logged = t.notice_log.entries().last().expect("ログに残る");
    assert_eq!(logged.notice.text, t.message);
    assert_eq!(logged.notice.kind, yolu_app::notice::Kind::Error);
    assert!(t.model.is_none());
    assert_eq!(t.sets.len(), 3);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn opening_another_project_while_the_model_is_loading_drops_the_old_result() {
    let dir = temp_dir("reopen-race");
    let model = character(&dir, "c.fbx");
    let ylp = dir.join("p.ylp");
    let mut s = project_from(&model, 512);
    s.apply(Action::SaveProjectAs(ylp.clone()));
    let plain = dir.join("plain.ylp");
    let mut u = S::new(64, 64);
    u.apply(Action::SaveProjectAs(plain.clone()));
    let mut t = S::new(64, 64);
    t.apply(Action::OpenProject(ylp));
    assert!(t.np.reopening.is_some());
    t.apply(Action::OpenProject(plain));
    assert!(
        t.np.reopening.is_none(),
        "前のプロジェクトのモデルの読み込みは止める"
    );
    for _ in 0..50 {
        t.poll_newproject();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(t.model.is_none() && t.view3d.model.is_none());
    // 読み込みの取消（仕事の札）
    let mut v = S::new(64, 64);
    v.apply(Action::OpenProject(dir.join("p.ylp")));
    np(&mut v, NpAction::CancelReopen);
    assert!(v.np.reopening.is_none() && v.message.contains("取り消し"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn sets_removed_in_the_configuration_are_gone_from_the_saved_file() {
    let dir = temp_dir("saved-removal");
    let model = character(&dir, "c.fbx");
    let ylp = dir.join("p.ylp");
    let mut s = project_from(&model, 512);
    s.apply(Action::SaveProjectAs(ylp.clone()));
    let before = read_file(&ylp);
    assert_eq!(before.sets().len(), 3);
    let removed_id = before.sets()[1].id.clone();
    np(&mut s, NpAction::OpenConfigure);
    np(&mut s, NpAction::Draft(1, DraftOp::Remove));
    np(&mut s, NpAction::Draft(0, DraftOp::Name("肌".into())));
    np(&mut s, NpAction::Submit);
    np(&mut s, NpAction::ConfirmApply);
    assert!(s.modified);
    s.apply(Action::SaveProject);
    assert!(
        s.message.contains("保存しました") && !s.modified,
        "{}",
        s.message
    );
    let after = read_file(&ylp);
    assert_eq!(
        after
            .sets()
            .iter()
            .map(|x| x.name.as_str())
            .collect::<Vec<_>>(),
        ["肌", "Eye"]
    );
    assert!(
        !after
            .migrated_entries()
            .keys()
            .any(|n| n.contains(&removed_id)),
        "消したセットの中身はファイルに残らない"
    );
    assert!(after.unknown_entries().is_empty());
    // 前の版は退避に残る（無断で消さない）
    assert!(
        dir.join("p.ylp-backups~").exists() || dir.join("p-backups~").exists(),
        "{:?}",
        std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect::<Vec<_>>()
    );
    // 開き直すと 2 つ
    let mut t = S::new(64, 64);
    t.apply(Action::OpenProject(ylp));
    assert_eq!(set_names(&t), ["肌", "Eye"]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_set_added_to_a_saved_project_is_written_and_a_resized_one_too() {
    let dir = temp_dir("saved-add");
    let model = character(&dir, "c.fbx");
    let ylp = dir.join("p.ylp");
    let mut s = project_from(&model, 512);
    s.apply(Action::SaveProjectAs(ylp.clone()));
    np(&mut s, NpAction::AddSet);
    np(&mut s, NpAction::OpenConfigure);
    np(&mut s, NpAction::Draft(0, DraftOp::Size(1024, 1024)));
    np(&mut s, NpAction::Submit);
    np(&mut s, NpAction::ConfirmApply);
    s.apply(Action::SaveProject);
    let mut t = S::new(64, 64);
    t.apply(Action::OpenProject(ylp));
    assert_eq!(t.sets.len(), 4);
    assert_eq!(size_of(&t, 0), (1024, 1024));
    assert_eq!(size_of(&t, 1), (512, 512));
    // 足したセットの鍵: モデルのスロット（4 つのメッシュ × 1 つのサブメッシュ）より後の、空いている仮のスロット
    assert_eq!(t.sets.get(3).unwrap().material, MaterialRef::PendingSlot(4));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_view_json_written_by_unity_is_left_alone_when_the_model_does_not_change() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../yolu-io/tests/fixtures/format6.ylp");
    let original = yolu_io::Project::read(&std::fs::read(&fixture).unwrap()).unwrap();
    let dir = temp_dir("unity-view");
    let mut s = S::new(64, 64);
    s.apply(Action::OpenProject(fixture));
    assert!(
        s.np.model_file.is_none(),
        "Unity 版のモデルの GUID はスタンドアロンでは読まない"
    );
    let out = dir.join("saved.ylp");
    s.apply(Action::SaveProjectAs(out.clone()));
    let saved = read_file(&out);
    assert_eq!(
        saved.migrated_entries()["view.json"],
        original.migrated_entries()["view.json"],
        "view.json は 1 バイトも変えない"
    );
    assert_eq!(saved.view_model().unwrap(), None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_project_has_at_most_64_sets_and_a_model_with_more_materials_gives_the_first_64() {
    // 空のセットを足していく
    let mut s = S::new(64, 64);
    np(&mut s, NpAction::OpenNew);
    np(&mut s, NpAction::Resolution(512));
    np(&mut s, NpAction::Submit);
    for _ in 0..63 {
        np(&mut s, NpAction::AddSet);
    }
    assert_eq!(s.sets.len(), 64);
    s.message.clear();
    np(&mut s, NpAction::AddSet);
    assert_eq!(s.sets.len(), 64);
    assert!(s.message.contains("64"), "{}", s.message);
    np(&mut s, NpAction::OpenConfigure);
    np(&mut s, NpAction::AddDraft);
    let win = s.np.window.as_ref().unwrap();
    assert_eq!(win.drafts.len(), 64, "上限を超えて足さない");
    assert!(
        win.error.as_deref().is_some_and(|e| e.contains("64")),
        "{:?}",
        win.error
    );
    // 70 のマテリアルを持つモデル
    let dir = temp_dir("limit");
    let names: Vec<String> = (0..70).map(|i| format!("M{i:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let quads: Vec<Option<usize>> = (0..70).map(Some).collect();
    let path = write_fbx(&dir, "many.fbx", &ascii_fbx(&refs, &[mesh("Many", &quads)]));
    let mut t = S::new(64, 64);
    open_new_with(&mut t, &path);
    assert_eq!(t.np.window.as_ref().unwrap().groups(&t).len(), 70);
    // ウィンドウは上限までしか選ばせない: 初めは先頭の 64、65 個目は理由つきで断る
    {
        let win = t.np.window.as_ref().unwrap();
        assert_eq!(win.chosen(70), (0..64).collect::<Vec<_>>());
        assert_eq!(win.over_limit(70), 6);
    }
    np(&mut t, NpAction::Material(64, true));
    {
        let win = t.np.window.as_ref().unwrap();
        assert!(
            win.error.as_deref().is_some_and(|e| e.contains("64")),
            "{:?}",
            win.error
        );
        assert_eq!(win.materials, None, "選びは変わらない");
    }
    // 1 つ外せば別の 1 つを選べる。選びを初めの形へ戻せば「選び直していない」に戻る
    np(&mut t, NpAction::Material(0, false));
    np(&mut t, NpAction::Material(64, true));
    {
        let win = t.np.window.as_ref().unwrap();
        assert!(win.error.is_none(), "{:?}", win.error);
        assert_eq!(win.chosen(70), (1..65).collect::<Vec<_>>());
    }
    np(&mut t, NpAction::Material(64, false));
    np(&mut t, NpAction::Material(0, true));
    assert_eq!(t.np.window.as_ref().unwrap().materials, None);
    np(&mut t, NpAction::Resolution(512));
    np(&mut t, NpAction::Submit);
    assert_eq!(t.sets.len(), 64, "上限までの最初の 64 マテリアル");
    assert_eq!(t.sets.get(63).unwrap().name, "M63");
    assert_eq!(t.sets.get(63).unwrap().bound, Some(63));
    assert!(
        t.message.contains("上限") && t.message.contains('6'),
        "セットにならなかった数を言う: {}",
        t.message
    );
    // 構成の「セットの無いマテリアルを足す」も、上限で止まるとき理由を言う
    np(&mut t, NpAction::OpenConfigure);
    {
        let win = t.np.window.as_ref().unwrap();
        assert_eq!(win.drafts.len(), 64);
        assert!(win.error.is_none());
    }
    np(&mut t, NpAction::AddUnused);
    {
        let win = t.np.window.as_ref().unwrap();
        assert_eq!(win.drafts.len(), 64, "上限を超えて足さない");
        assert!(
            win.error.as_deref().is_some_and(|e| e.contains("64")),
            "{:?}",
            win.error
        );
    }
    np(&mut t, NpAction::Close);
    // 英語でも
    let mut e = S::new_in(64, 64, Lang::En);
    open_new_with(&mut e, &path);
    np(&mut e, NpAction::Resolution(512));
    np(&mut e, NpAction::Submit);
    assert_eq!(e.sets.len(), 64);
    assert!(
        e.message.contains("limit") && !has_jp(&e.message),
        "{}",
        e.message
    );
    // 保存して開き直せる（上限の 64 つ）
    let ylp = dir.join("many.ylp");
    t.apply(Action::SaveProjectAs(ylp.clone()));
    assert!(t.message.contains("保存しました"), "{}", t.message);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_normal_format_change_is_one_undo_step_in_each_set() {
    let mut s = S::new(64, 64);
    np(&mut s, NpAction::OpenNew);
    np(&mut s, NpAction::Resolution(512));
    np(&mut s, NpAction::Submit);
    np(&mut s, NpAction::AddSet);
    np(&mut s, NpAction::OpenConfigure);
    np(&mut s, NpAction::Normal(NormalYDirection::DirectX));
    np(&mut s, NpAction::Submit);
    for i in 0..2 {
        let doc = s.set_doc_mut(i);
        assert_eq!(
            doc.normal_settings().file_direction(),
            NormalYDirection::DirectX
        );
        assert_eq!(doc.undo_count(), 1, "セットごとに 1 段");
        doc.undo().unwrap();
        assert_eq!(
            doc.normal_settings().file_direction(),
            NormalYDirection::OpenGL
        );
    }
}

#[test]
fn applying_without_touching_the_normal_format_leaves_sets_with_another_format_alone() {
    let mut s = S::new(64, 64);
    np(&mut s, NpAction::OpenNew);
    np(&mut s, NpAction::Resolution(512));
    np(&mut s, NpAction::Submit);
    np(&mut s, NpAction::AddSet);
    // 2 つ目だけ DirectX（セットごとに持てる）。Undo の段は 0 のまま始める
    {
        let doc = s.set_doc_mut(1);
        let settings = doc.normal_settings();
        doc.set_normal_settings(
            settings.with_file_direction(NormalYDirection::DirectX),
            false,
        )
        .unwrap();
        doc.clear_history().unwrap();
    }
    let format = |s: &S, i: usize| s.set_doc(i).normal_settings().file_direction();
    assert_eq!(
        (format(&s, 0), format(&s, 1)),
        (NormalYDirection::OpenGL, NormalYDirection::DirectX)
    );
    // 名前だけ変えて適用: どちらのセットの形式も、Undo の段も変わらない（ウィンドウは今のセット = 1 つ目の形式で開く）
    s.switch_set(0).unwrap();
    np(&mut s, NpAction::OpenConfigure);
    assert_eq!(
        s.np.window.as_ref().unwrap().normal,
        NormalYDirection::OpenGL
    );
    np(&mut s, NpAction::Draft(0, DraftOp::Name("名前".into())));
    np(&mut s, NpAction::Submit);
    assert!(s.np.window.is_none(), "{:?}", s.message);
    assert_eq!(set_names(&s)[0], "名前");
    assert_eq!(
        (format(&s, 0), format(&s, 1)),
        (NormalYDirection::OpenGL, NormalYDirection::DirectX),
        "触っていない形式は書き換えない"
    );
    assert_eq!(
        (s.set_doc(0).undo_count(), s.set_doc(1).undo_count()),
        (0, 0),
        "Undo の段も足さない"
    );
    // 利用者が形式を変えたときだけ、全部のセットへ当てる（セットごとに 1 段）
    np(&mut s, NpAction::OpenConfigure);
    np(&mut s, NpAction::Normal(NormalYDirection::DirectX));
    np(&mut s, NpAction::Submit);
    assert_eq!(
        (format(&s, 0), format(&s, 1)),
        (NormalYDirection::DirectX, NormalYDirection::DirectX)
    );
    assert_eq!(
        (s.set_doc(0).undo_count(), s.set_doc(1).undo_count()),
        (1, 0),
        "もう DirectX だったセットは変えない"
    );
    // 開いたときの値へ戻して適用するのは、変えていないのと同じ
    np(&mut s, NpAction::OpenConfigure);
    np(&mut s, NpAction::Normal(NormalYDirection::OpenGL));
    np(&mut s, NpAction::Normal(NormalYDirection::DirectX));
    np(&mut s, NpAction::Draft(1, DraftOp::Name("二".into())));
    np(&mut s, NpAction::Submit);
    assert_eq!(s.set_doc(0).undo_count(), 1);
}

#[test]
fn a_missing_or_foreign_file_is_told_with_a_reason_in_both_languages() {
    let dir = temp_dir("foreign");
    let png = dir.join("picture.png");
    std::fs::write(&png, [0x89, b'P', b'N', b'G', 13, 10, 26, 10]).unwrap();
    for lang in Lang::ALL {
        let mut s = S::new_in(64, 64, lang);
        np(&mut s, NpAction::OpenNew);
        for path in [dir.join("nothing.fbx"), png.clone()] {
            np(&mut s, NpAction::ChooseModel(path));
            wait_model(&mut s);
            match &s.np.window.as_ref().unwrap().prep {
                Prep::Failed { error, .. } => {
                    let why = lang.view_error(error);
                    assert!(!why.is_empty());
                    assert_eq!(has_jp(&why), lang == Lang::Ja, "{why}");
                }
                _ => panic!("読めないはず"),
            }
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

fn has_jp(text: &str) -> bool {
    text.chars()
        .any(|c| matches!(c, '\u{3000}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}' | '\u{ff00}'..='\u{ffef}'))
}

// ───────── ウィンドウの操作（egui_kittest） ─────────

type H = Harness<'static, YoluApp>;

fn st(h: &H) -> &S {
    &h.state().state
}

fn run_np(h: &mut H, a: NpAction) {
    h.state_mut().state.apply(Action::Project(a));
    h.run();
}

/// ウィンドウの中だけを撮って、正解の絵と比べる（ほかのパネルの変更で壊れない）。
fn shot(h: &mut H, name: &str) {
    let rect = yolu_app::newproject::window::last_rect(&h.ctx).expect("ウィンドウを描いていない");
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

/// 開いているドロップダウンの中の項目の矩形。
fn dd_item(h: &H, label: &str) -> Rect {
    let body = st(h)
        .np
        .window
        .as_ref()
        .and_then(|w| w.dropdown.as_ref())
        .expect("ドロップダウンが開いている")
        .1
        .rect;
    rect_of(h, label, |r| body.contains(r.center()))
}

/// 描いた文字（ウィンドウの中だけ）。クリップの外へはみ出す文字があれば落とす。
fn window_texts(h: &H) -> Vec<String> {
    use egui::epaint::Shape;
    fn walk(shape: &Shape, clip: Rect, out: &mut Vec<String>) {
        match shape {
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, out)),
            Shape::Text(t) => {
                let bounds = Rect::from_min_size(t.pos, t.galley.size());
                assert!(
                    clip.expand(1.0).contains_rect(bounds),
                    "文字がクリップからはみ出す: {:?} {bounds:?} clip={clip:?}",
                    t.galley.job.text
                );
                out.push(t.galley.job.text.clone());
            }
            _ => {}
        }
    }
    let rect = yolu_app::newproject::window::last_rect(&h.ctx).expect("ウィンドウを描いていない");
    let mut out = Vec::new();
    for s in &h.output().shapes {
        if rect.intersects(s.clip_rect) {
            let mut texts = Vec::new();
            walk(&s.shape, s.clip_rect, &mut texts);
            out.extend(texts);
        }
    }
    out
}

fn gui() -> H {
    app(1280.0, 860.0, 256)
}

/// モデルの準備が終わるまでフレームを回す。
fn settle_model(h: &mut H) {
    let start = Instant::now();
    loop {
        h.step();
        match st(h).np.window.as_ref() {
            Some(w) if w.is_loading() => {}
            _ => break,
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "モデルの準備が終わらない"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    h.run();
}

#[test]
fn ctrl_n_asks_then_the_window_creates_with_the_chosen_template_and_resolution() {
    let mut h = gui();
    key(&h, Key::N, Modifiers::COMMAND);
    h.run();
    assert_eq!(
        st(&h).dialog_request,
        Some(DialogRequest::New),
        "Ctrl+N は保存していない変更を先に聞く（ウィンドウは聞いたあとに開く）"
    );
    h.state_mut().state.dialog_request = None;
    run_np(&mut h, NpAction::OpenNew);
    let win = yolu_app::newproject::window::last_rect(&h.ctx).expect("ウィンドウ");
    assert!(win.width() > 400.0);
    for label in [
        "作成",
        "キャンセル",
        "PBR（すべてのチャンネル）",
        "2048 × 2048",
        "OpenGL（Y+、Unity）",
    ] {
        h.get_by_label(label);
    }
    // ドロップダウンで選ぶ
    h.get_by_label("2048 × 2048").click();
    h.run();
    let at = dd_item(&h, "1024 × 1024").center();
    click(&mut h, at);
    assert_eq!(st(&h).np.window.as_ref().unwrap().resolution, 1024);
    assert!(
        st(&h).np.window.as_ref().unwrap().dropdown.is_none(),
        "選んだら閉じる"
    );
    h.get_by_label("PBR（すべてのチャンネル）").click();
    h.run();
    let at = dd_item(&h, "カラーのみ").center();
    click(&mut h, at);
    h.get_by_label("OpenGL（Y+、Unity）").click();
    h.run();
    let at = dd_item(&h, "DirectX（Y−）").center();
    click(&mut h, at);
    // モデルを決めていなければ、ベイクは選べない
    assert!(!st(&h).np.window.as_ref().unwrap().bake);
    h.get_by_label("作成").click();
    h.run();
    assert!(st(&h).np.window.is_none());
    assert_eq!(size_of(st(&h), 0), (1024, 1024));
    assert_eq!(enabled_channels(&st(&h).doc), [Channel::Color]);
    assert_eq!(
        st(&h).doc.normal_settings().file_direction(),
        NormalYDirection::DirectX
    );
}

#[test]
fn escape_closes_the_window_and_enter_creates() {
    let mut h = gui();
    let id = st(&h).doc.id();
    run_np(&mut h, NpAction::OpenNew);
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(st(&h).np.window.is_none());
    assert_eq!(st(&h).doc.id(), id);
    // ウィンドウの外のキーは効かない（モーダル）
    run_np(&mut h, NpAction::OpenNew);
    key(&h, Key::B, Modifiers::NONE);
    key(&h, Key::E, Modifiers::NONE);
    h.run();
    assert_eq!(st(&h).tool, yolu_app::state::Tool::Brush);
    key(&h, Key::Enter, Modifiers::NONE);
    h.run();
    assert!(st(&h).np.window.is_none());
    assert_ne!(st(&h).doc.id(), id, "Enter で作る");
    // ドロップダウンを開いているときの Esc は、ドロップダウンだけを閉じる
    run_np(&mut h, NpAction::OpenNew);
    h.get_by_label("2048 × 2048").click();
    h.run();
    assert!(st(&h).np.window.as_ref().unwrap().dropdown.is_some());
    key(&h, Key::Escape, Modifiers::NONE);
    h.run();
    assert!(st(&h).np.window.is_some() && st(&h).np.window.as_ref().unwrap().dropdown.is_none());
    h.get_by_label("キャンセル").click();
    h.run();
    assert!(st(&h).np.window.is_none());
}

#[test]
fn the_window_lists_the_materials_with_their_meshes_and_creates_the_checked_ones() {
    let dir = temp_dir("gui-materials");
    let path = character(&dir, "c.fbx");
    let mut h = gui();
    run_np(&mut h, NpAction::OpenNew);
    run_np(&mut h, NpAction::ChooseModel(path));
    settle_model(&mut h);
    assert!(matches!(
        st(&h).np.window.as_ref().unwrap().prep,
        Prep::Ready { .. }
    ));
    let texts = window_texts(&h);
    for expected in [
        "c.fbx",
        "Skin",
        "Hair",
        "Eye",
        "Body, Face",
        "HairMesh",
        "EyeMesh",
    ] {
        assert!(texts.iter().any(|t| t == expected), "{expected}: {texts:?}");
    }
    shot(&mut h, "newproject_window");
    // チェックを外す（Eye）→ 作る
    h.get_by_label("Eye").click();
    h.run();
    assert_eq!(
        st(&h).np.window.as_ref().unwrap().materials,
        Some(vec![0, 1])
    );
    // 最後の 1 つは外せない
    h.get_by_label("Hair").click();
    h.run();
    assert_eq!(st(&h).np.window.as_ref().unwrap().materials, Some(vec![0]));
    h.get_by_label("Skin").click();
    h.run();
    assert_eq!(
        st(&h).np.window.as_ref().unwrap().materials,
        Some(vec![0]),
        "最後の 1 つは外れない"
    );
    h.get_by_label("Hair").click();
    h.run();
    h.get_by_label("作成").click();
    h.run();
    assert_eq!(set_names(st(&h)), ["Skin", "Hair"]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_window_shows_preparing_canceled_and_failed_states_with_their_buttons() {
    let dir = temp_dir("gui-prepare");
    let path = character(&dir, "c.fbx");
    let mut h = gui();
    run_np(&mut h, NpAction::OpenNew);
    // 読んでいる間（終わらない読み込みを置く）: 作れない・取り消せる
    let (job, hold) = yolu_app::view3d::pose::PrepareJob::parked();
    {
        let win = h.state_mut().state.np.window.as_mut().unwrap();
        win.model = Some(path.clone());
        win.prep = Prep::Loading {
            path: path.clone(),
            job,
        };
    }
    h.run();
    assert!(window_texts(&h).iter().any(|t| t == "モデルを準備中"));
    h.get_by_label("作成").click(); // 準備が済むまで押せない
    h.run();
    assert!(
        st(&h).np.window.as_ref().unwrap().is_loading()
            && st(&h).np.window.as_ref().unwrap().error.is_none()
    );
    h.get_by_label("準備を取り消す").click();
    h.run();
    assert!(matches!(
        st(&h).np.window.as_ref().unwrap().prep,
        Prep::Canceled { .. }
    ));
    assert!(window_texts(&h).iter().any(|t| t == "準備を取り消しました"));
    h.get_by_label("もう一度準備する").click();
    settle_model(&mut h);
    assert!(matches!(
        st(&h).np.window.as_ref().unwrap().prep,
        Prep::Ready { .. }
    ));
    // 失敗: 短い理由と「もう一度」
    let (job, _hold2) = yolu_app::view3d::pose::PrepareJob::parked();
    {
        let win = h.state_mut().state.np.window.as_mut().unwrap();
        win.prep = Prep::Loading {
            path: path.clone(),
            job,
        };
    }
    let _ = hold; // 最初の保持はここで手放してよい
    let broken = write_fbx(&dir, "broken.fbx", "FBX ではない");
    run_np(&mut h, NpAction::ChooseModel(broken));
    settle_model(&mut h);
    assert!(matches!(
        st(&h).np.window.as_ref().unwrap().prep,
        Prep::Failed { .. }
    ));
    h.get_by_label("もう一度準備する");
    h.get_by_label("作成").click();
    h.run();
    assert!(st(&h).np.window.is_some(), "読めなかったモデルでは作れない");
    let _ = std::fs::remove_dir_all(dir);
}

/// 準備の行: 割合がまだ分からない間はラベルだけ（往復する帯）、分かったら % を添えて帯を埋める。取り消しのボタンは常に出る。
#[test]
fn the_preparing_row_adds_the_percentage_once_it_is_known_in_both_languages() {
    let dir = temp_dir("gui-prepare-progress");
    let path = character(&dir, "c.fbx");
    for lang in Lang::ALL {
        let mut h = gui();
        h.state_mut().state.set_language(lang);
        run_np(&mut h, NpAction::OpenNew);
        let (job, _hold) = yolu_app::view3d::pose::PrepareJob::parked();
        {
            let win = h.state_mut().state.np.window.as_mut().unwrap();
            win.model = Some(path.clone());
            win.prep = Prep::Loading {
                path: path.clone(),
                job,
            };
        }
        let (label, cancel) = match lang {
            Lang::Ja => ("モデルを準備中", "準備を取り消す"),
            Lang::En => ("Preparing the model", "Cancel preparation"),
        };
        h.run();
        assert!(
            window_texts(&h).iter().any(|t| t == label),
            "{lang:?}: {:?}",
            window_texts(&h)
        );
        assert!(h.query_by_label(cancel).is_some(), "{lang:?}");
        match &st(&h).np.window.as_ref().unwrap().prep {
            Prep::Loading { job, .. } => job.report_progress(0.37),
            _ => unreachable!(),
        }
        h.run();
        assert!(
            window_texts(&h)
                .iter()
                .any(|t| *t == format!("{label} 37%")),
            "{lang:?}: {:?}",
            window_texts(&h)
        );
        assert!(h.query_by_label(cancel).is_some(), "{lang:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_empty_model_box_says_nothing_and_a_model_called_none_is_shown_as_a_model() {
    let dir = temp_dir("gui-no-model");
    let named_none = character(&dir, "None");
    for lang in Lang::ALL {
        let mut h = gui();
        h.state_mut().state.set_language(lang);
        for configure in [false, true] {
            run_np(
                &mut h,
                if configure {
                    NpAction::OpenConfigure
                } else {
                    NpAction::OpenNew
                },
            );
            let texts = window_texts(&h);
            assert!(
                texts.iter().all(|t| t != "なし" && t != "None"),
                "{lang:?} 選んでいない欄に文字を出さない: {texts:?}"
            );
            run_np(&mut h, NpAction::Close);
        }
        // 「None」という名前のモデルは、選んでいないのではなくモデルとして出す
        run_np(&mut h, NpAction::OpenNew);
        run_np(&mut h, NpAction::ChooseModel(named_none.clone()));
        settle_model(&mut h);
        assert_eq!(
            st(&h).np.window.as_ref().unwrap().model.as_deref(),
            Some(named_none.as_path())
        );
        let texts = window_texts(&h);
        assert!(texts.iter().any(|t| t == "None"), "{lang:?}: {texts:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_window_follows_the_language_without_clipped_or_japanese_text() {
    let dir = temp_dir("gui-english");
    let path = character(&dir, "c.fbx");
    for lang in Lang::ALL {
        let mut h = gui();
        h.state_mut().state.set_language(lang);
        run_np(&mut h, NpAction::OpenNew);
        run_np(&mut h, NpAction::ChooseModel(path.clone()));
        settle_model(&mut h);
        let texts = window_texts(&h);
        for expected in [
            lang.pick("テンプレート", "Template"),
            lang.pick("メッシュ", "Mesh"),
            lang.pick("解像度", "Resolution"),
            lang.pick("ノーマルマップの形式", "Normal map format"),
            lang.pick(
                "作成後にメッシュマップをベイクする",
                "Bake mesh maps after creating",
            ),
            lang.pick("作成", "Create"),
            lang.pick("キャンセル", "Cancel"),
        ] {
            assert!(
                texts.iter().any(|t| t == expected),
                "{lang:?} {expected}: {texts:?}"
            );
        }
        if lang == Lang::En {
            for t in &texts {
                assert!(!has_japanese(t), "英語の画面に日本語: {t}");
            }
            shot(&mut h, "newproject_window_english");
        }
        // 構成
        run_np(&mut h, NpAction::Close);
        run_np(&mut h, NpAction::OpenConfigure);
        let texts = window_texts(&h);
        for expected in [
            lang.pick("補間方法", "Resampling"),
            lang.pick("適用", "Apply"),
            lang.pick("テクスチャセットを追加", "Add Texture Set"),
        ] {
            assert!(
                texts.iter().any(|t| t == expected),
                "{lang:?} {expected}: {texts:?}"
            );
        }
        if lang == Lang::En {
            assert!(texts.iter().all(|t| !has_japanese(t)), "{texts:?}");
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_windows_fixed_text_fits_without_being_cut_in_both_languages() {
    let dir = temp_dir("gui-fit");
    let old = character(&dir, "old.fbx");
    let new = cape_model(&dir, "new.fbx");
    for lang in Lang::ALL {
        let mut s = project_from(&old, 512);
        s.set_language(lang);
        let mut h = gui();
        *h.state_mut() = YoluApp::for_context(&h.ctx, s, yolu_app::pen::PenInput::detached());
        yolu_app::ui::widgets::record_truncations(true);
        // 新規: モデルを読み終えた・読めなかった・取り消した
        run_np(&mut h, NpAction::OpenNew);
        run_np(&mut h, NpAction::ChooseModel(new.clone()));
        settle_model(&mut h);
        run_np(&mut h, NpAction::ChooseModel(dir.join("nothing.fbx")));
        settle_model(&mut h);
        run_np(&mut h, NpAction::CancelPrepare);
        run_np(&mut h, NpAction::Close);
        // 構成: モデルを替えた（モデルに無い・セットの無いマテリアルの印）・大きさを変える・確かめ・足す
        run_np(&mut h, NpAction::OpenConfigure);
        run_np(&mut h, NpAction::ChooseModel(new.clone()));
        settle_model(&mut h);
        run_np(&mut h, NpAction::Draft(0, DraftOp::Size(1024, 1024)));
        run_np(&mut h, NpAction::AddDraft);
        run_np(&mut h, NpAction::Submit);
        assert!(st(&h).np.window.as_ref().unwrap().confirm.is_some());
        let _ = window_texts(&h);
        run_np(&mut h, NpAction::ConfirmCancel);
        run_np(&mut h, NpAction::Draft(1, DraftOp::Name("Skin".into())));
        run_np(&mut h, NpAction::Submit);
        assert!(st(&h).np.window.as_ref().unwrap().error.is_some());
        let _ = window_texts(&h);
        let cut = yolu_app::ui::widgets::take_truncations();
        yolu_app::ui::widgets::record_truncations(false);
        // 詰めてよいのは、読めなかった理由（OS の文を含む。全文はツールチップ）だけ
        let cut: Vec<&String> = cut.iter().filter(|t| !t.contains("os error")).collect();
        assert!(cut.is_empty(), "{lang:?}: 詰めた文字: {cut:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_file_menu_opens_the_configuration_and_a_row_edits_removes_and_applies() {
    let dir = temp_dir("gui-configure");
    let path = character(&dir, "c.fbx");
    let mut s = project_from(&path, 512);
    paint_dot(&mut s, 0, (3, 3), Rgba8::new(1, 2, 3, 255));
    let mut h = gui();
    *h.state_mut() = YoluApp::for_context(&h.ctx, s, yolu_app::pen::PenInput::detached());
    h.run();
    let title = menu_title(&h, "ファイル").center();
    click(&mut h, title);
    let item = popup_item(&h, "プロジェクト設定…").center();
    click(&mut h, item);
    assert!(st(&h).np.window.as_ref().is_some_and(|w| w.configure));
    let texts = window_texts(&h);
    assert!(
        texts.iter().any(|t| t == "Skin") || st(&h).np.window.as_ref().unwrap().drafts.len() == 3
    );
    shot(&mut h, "newproject_configure");
    // 大きさのドロップダウン（Hair の行）を替える
    let sizes: Vec<_> = h.get_all_by_label("512").map(|n| n.rect()).collect();
    assert!(sizes.len() >= 3, "セットごとの大きさ: {sizes:?}");
    click(&mut h, sizes[1].center());
    assert!(st(&h).np.window.as_ref().unwrap().dropdown.is_some());
    let at = dd_item(&h, "1024 × 1024").center();
    click(&mut h, at);
    assert_eq!(
        st(&h).np.window.as_ref().unwrap().drafts[1].size,
        (1024, 1024)
    );
    assert!(st(&h).np.window.as_ref().unwrap().drafts[1].resizes());
    // 3 つ目（Eye）を消す
    let removes: Vec<_> = h
        .get_all_by_label(
            "このテクスチャセットを消す（適用するときにもう一度確かめます。その作業は消えます）",
        )
        .map(|n| n.rect())
        .collect();
    assert_eq!(removes.len(), 3);
    click(&mut h, removes[2].center());
    assert_eq!(st(&h).np.window.as_ref().unwrap().drafts.len(), 2);
    // 適用 → 確かめの一覧
    h.get_by_label("適用").click();
    h.run();
    let plan = st(&h)
        .np
        .window
        .as_ref()
        .unwrap()
        .confirm
        .clone()
        .expect("確かめる");
    assert_eq!(plan.rows.len(), 2, "消す 1 つと大きさ 1 つ");
    shot(&mut h, "newproject_confirm");
    // 確かめの「やめる」→ ウィンドウは残り、何も変わっていない
    h.get_by_label("やめる").click();
    h.run();
    assert!(st(&h).np.window.as_ref().unwrap().confirm.is_none());
    assert_eq!(st(&h).sets.len(), 3);
    // もう一度: 確かめの「適用」（ウィンドウの「適用」は確かめの下で押せない）
    h.get_by_label("適用").click();
    h.run();
    let apply: Vec<_> = h.get_all_by_label("適用").map(|n| n.rect()).collect();
    assert!(apply.len() >= 2, "確かめの「適用」がある: {apply:?}");
    click(&mut h, apply.last().unwrap().center());
    assert!(st(&h).np.window.is_none());
    assert_eq!(set_names(st(&h)), ["Skin", "Hair"]);
    assert_eq!(size_of(st(&h), 1), (1024, 1024));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_texture_set_panel_adds_removes_and_opens_the_configuration() {
    let mut h = gui();
    h.get_by_label("空のテクスチャセットを追加（今のセットと同じ大きさ・チャンネル）")
        .click();
    h.run();
    assert_eq!(st(&h).sets.len(), 2);
    assert_eq!(st(&h).sets.current_index(), 1);
    // 今のセットを消す: 確認のウィンドウが先に出る
    h.get_by_label("今のテクスチャセットを消す（確かめます。その作業は消えます）")
        .click();
    h.run();
    assert_eq!(st(&h).np.remove_confirm.as_ref().map(Vec::len), Some(1));
    assert_eq!(st(&h).sets.len(), 2, "確かめるまでは消さない");
    h.get_by_label("やめる").click();
    h.run();
    assert!(st(&h).np.remove_confirm.is_none());
    h.get_by_label("今のテクスチャセットを消す（確かめます。その作業は消えます）")
        .click();
    h.run();
    h.get_by_label("消す").click();
    h.run();
    assert_eq!(st(&h).sets.len(), 1);
    assert!(st(&h).modified);
    // 最後の 1 つは消せない（ボタンは押せない）
    h.get_by_label("プロジェクトには少なくとも 1 つのテクスチャセットが要ります")
        .click();
    h.run();
    assert!(st(&h).np.remove_confirm.is_none());
    // 設定のボタン
    h.get_by_label("プロジェクト設定…").click();
    h.run();
    assert!(st(&h).np.window.as_ref().is_some_and(|w| w.configure));
}

#[test]
fn the_context_menu_removes_a_set_after_asking() {
    let mut h = gui();
    h.state_mut().state.apply(Action::Project(NpAction::AddSet));
    h.run();
    let first = h.get_by_label("テクスチャセット 1").rect().center();
    common::press(&h, first, egui::PointerButton::Secondary);
    h.step();
    common::release(&h, first, egui::PointerButton::Secondary);
    h.run();
    let at = popup_item(&h, "消す…").center();
    click(&mut h, at);
    assert_eq!(st(&h).np.remove_confirm.as_ref().map(Vec::len), Some(1));
    assert_eq!(st(&h).sets.len(), 2);
}

/// ウィンドウに落としたファイル（パスだけ持つ）。
#[derive(Debug)]
struct Dropped(PathBuf);
impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

#[test]
fn an_fbx_dropped_on_the_window_opens_the_new_project_window_or_replaces_the_windows_model() {
    let dir = temp_dir("gui-drop");
    let path = character(&dir, "c.fbx");
    let other = cape_model(&dir, "other.fbx");
    let mut h = gui();
    h.input_mut()
        .dropped_files
        .push(std::sync::Arc::new(Dropped(path.clone())));
    h.run();
    {
        let win = st(&h)
            .np
            .window
            .as_ref()
            .expect("落とした FBX で新規プロジェクトのウィンドウ");
        assert!(!win.configure && win.model.as_deref() == Some(path.as_path()));
    }
    settle_model(&mut h);
    // ウィンドウが開いているあいだに別の FBX を落とすと、ウィンドウのモデルが替わる（ウィンドウは 1 つのまま）
    h.input_mut()
        .dropped_files
        .push(std::sync::Arc::new(Dropped(other.clone())));
    h.run();
    settle_model(&mut h);
    let win = st(&h).np.window.as_ref().unwrap();
    assert_eq!(win.model.as_deref(), Some(other.as_path()));
    assert_eq!(
        win.groups(st(&h))
            .iter()
            .map(|g| g.name.as_str())
            .collect::<Vec<_>>(),
        ["Hair", "Cape", "skin"]
    );
    assert!(
        st(&h).model.is_none(),
        "決めるまでは 3D ビューにも記録にも入れない"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_demo_models_are_not_files_so_the_projects_model_reference_goes() {
    let dir = temp_dir("demo");
    let path = character(&dir, "c.fbx");
    let mut s = project_from(&path, 512);
    assert!(s.np.model_file.is_some());
    s.apply(Action::LoadDemoModel);
    assert!(
        s.np.model_file.is_none(),
        "試しの立方体にすると参照は外れる"
    );
    let mut t = project_from(&path, 512);
    t.apply(Action::Pose(yolu_app::view3d::pose::PoseAction::LoadFigure));
    assert!(t.np.model_file.is_none(), "試しの人形も");
    // 外れたあとに保存すると、ファイルにもモデルの参照は残らない
    let ylp = dir.join("p.ylp");
    t.apply(Action::SaveProjectAs(ylp.clone()));
    assert_eq!(read_file(&ylp).view_model().unwrap(), None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn opening_or_creating_another_project_closes_the_window() {
    let dir = temp_dir("replaced");
    let ylp = dir.join("p.ylp");
    let mut other = S::new(64, 64);
    other.apply(Action::SaveProjectAs(ylp.clone()));
    let mut s = S::new(64, 64);
    np(&mut s, NpAction::OpenConfigure);
    s.apply(Action::OpenProject(ylp));
    assert!(s.np.window.is_none());
    np(&mut s, NpAction::OpenNew);
    s.apply(Action::NewProject);
    assert!(s.np.window.is_none());
    // 開けなかったときは、ウィンドウを閉じず何も変えない
    np(&mut s, NpAction::OpenNew);
    s.apply(Action::OpenProject(dir.join("missing.ylp")));
    assert!(s.np.window.is_some(), "開けなかったらウィンドウはそのまま");
    let _ = std::fs::remove_dir_all(dir);
}
