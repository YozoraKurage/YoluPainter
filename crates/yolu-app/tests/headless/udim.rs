//! UDIM のテクスチャセット（右クリックのメニューの「UDIM」と「UDIM のタイルを今のモデルに合わせる」）: モデルの UV が 1 つのタイルの中だけ・
//! モデルが無いときは押せず理由がある、入れると 3D のモデルの UV がシートへ写り（違うタイルの頂点は複製）、1 回の Undo で戻る、
//! 表に無いタイル・タイルをまたぐ三角形を知らせる、合わせると足りないタイルが増える、書き出しはタイルごとのファイル（空のセルは書かない）、
//! 一覧の大きさの欄、保存して開き直すと同じシート。
use crate::common::drive::Drive;
use yolu_app::export::ExportAction;
use yolu_app::lang::Lang;
use yolu_app::state::{Action, AppState};
use yolu_app::udim::{Blocker, UdimAction};
use yolu_app::view3d::model::ViewModel;
use yolu_app::view3d::pose::{self, PoseAction};
use yolu_core::geometry::{ModelMesh, Submesh};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::{Channel, Rgba8};

const SIZE: u32 = 32;

/// 立方体の 3 つの面を、UV のタイル 1001・1002・1011 に置いた物（面ごとに頂点は別。1 つの面の 2 つの三角形は 1 つのタイルの中）。
/// `shared` なら、1001 と 1002 の面が UV の境（u = 1）の頂点を共有する。
fn tiled_cube(shared: bool, extra_tile: Option<Vec2>) -> ViewModel {
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    let mut quad = |p: [Vec3; 4], origin: Vec2| {
        let base = positions.len() as u32;
        for (k, (u, v)) in [(0.1, 0.1), (0.9, 0.1), (0.1, 0.9), (0.9, 0.9)]
            .into_iter()
            .enumerate()
        {
            positions.push(p[k]);
            uvs.push(origin + Vec2::new(u, v));
        }
        indices.extend([0, 1, 2, 2, 1, 3].map(|i| base + i));
    };
    let v = Vec3::new;
    quad(
        [v(0., 0., 0.), v(1., 0., 0.), v(0., 1., 0.), v(1., 1., 0.)],
        Vec2::new(0.0, 0.0),
    );
    quad(
        [v(1., 0., 0.), v(1., 0., 1.), v(1., 1., 0.), v(1., 1., 1.)],
        Vec2::new(1.0, 0.0),
    );
    quad(
        [v(0., 1., 0.), v(1., 1., 0.), v(0., 1., 1.), v(1., 1., 1.)],
        Vec2::new(0.0, 1.0),
    );
    if let Some(o) = extra_tile {
        quad(
            [v(0., 0., 1.), v(1., 0., 1.), v(0., 1., 1.), v(1., 1., 1.)],
            o,
        );
    }
    if shared {
        // 1002 の面の左の 2 頂点（u = 1.1）を、1001 の面の右の 2 頂点の位置へ寄せて共有させる（UV は境の u = 1.0）
        uvs[1] = Vec2::new(1.0, 0.1);
        uvs[3] = Vec2::new(1.0, 0.9);
        indices[6..12].copy_from_slice(&[1, 5, 3, 3, 5, 7]);
    }
    let mesh = ModelMesh {
        name: "箱".into(),
        positions,
        normals: Vec::new(),
        uvs,
        submeshes: vec![Submesh {
            material: 0,
            indices,
        }],
    };
    ViewModel::new("箱", vec![mesh], vec![Some("材".into())], 1).unwrap()
}

fn flat_plate() -> ViewModel {
    let mesh = ModelMesh {
        name: "板".into(),
        positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::ONE],
        normals: Vec::new(),
        uvs: vec![
            Vec2::new(0.1, 0.1),
            Vec2::new(0.9, 0.1),
            Vec2::new(0.1, 0.9),
            Vec2::new(0.9, 0.9),
        ],
        submeshes: vec![Submesh {
            material: 0,
            indices: vec![0, 1, 2, 2, 1, 3],
        }],
    };
    ViewModel::new("板", vec![mesh], vec![Some("材".into())], 1).unwrap()
}

fn painted() -> AppState {
    let mut s = AppState::new(SIZE, SIZE);
    let layer = s.selected_layer().unwrap();
    for y in 0..SIZE {
        for x in 0..SIZE {
            s.doc_mut()
                .set_pixel(layer, x, y, Rgba8::new(200, 60, 40, 255))
                .unwrap();
        }
    }
    s.doc_mut().clear_history().unwrap();
    s
}

fn uid(s: &AppState) -> u32 {
    s.sets().current().uid
}

#[test]
fn the_item_needs_a_model_with_two_tiles_or_more() {
    let mut s = painted();
    assert_eq!(s.udim_blocker(0), Some(Blocker::NoModel));
    s.view3d_mut().set_model(flat_plate());
    s.sync_view3d();
    assert_eq!(s.udim_blocker(0), Some(Blocker::OneTile));
    for lang in Lang::JA_EN {
        s.set_lang(lang);
        assert_eq!(
            s.udim_blocker_text(Blocker::OneTile),
            lang.pick(
                "UV が 1 つのタイルの中だけです",
                "The UVs are inside one tile only"
            )
        );
    }
    // 押しても断って何も変えない
    let id = uid(&s);
    s.act(Action::Udim(UdimAction::Toggle(id)));
    assert!(s.doc().udim().is_none());
    assert_eq!(s.doc().width(), SIZE);
}

#[test]
fn turning_on_maps_the_model_uvs_and_one_undo_turns_it_off() {
    let mut s = painted();
    s.view3d_mut().set_model(tiled_cube(true, None));
    s.sync_view3d();
    assert_eq!(s.model_udim_tiles(0), Some(vec![1001, 1002, 1011]));
    assert_eq!(s.udim_blocker(0), None);
    let before = s.view3d().full_model().unwrap().clone();
    let id = uid(&s);
    s.act(Action::Udim(UdimAction::Toggle(id)));
    let layout = s.doc().udim().unwrap().clone();
    assert_eq!(layout.tiles(), &[1001, 1002, 1011]);
    assert_eq!((s.doc().width(), s.doc().height()), (2 * SIZE, 2 * SIZE));
    // 今の画素は 1001 のセル（左下）
    assert_eq!(
        s.doc().composite_pixel(Channel::Color, 3, 3).unwrap(),
        Rgba8::new(200, 60, 40, 255)
    );
    assert_eq!(
        s.doc()
            .composite_pixel(Channel::Color, SIZE + 3, 3)
            .unwrap()
            .a,
        0
    );
    // 3D のモデル: UV がシートの 0〜1 へ写り、共有していた 2 頂点が複製される。三角形の数は同じ
    let mapped = s.view3d().full_model().unwrap().clone();
    assert_eq!(mapped.triangle_count(), before.triangle_count());
    assert_eq!(
        mapped.meshes[0].positions.len(),
        before.meshes[0].positions.len() + 2
    );
    for t in mapped.geometry.triangles() {
        for uv in [t.uv_a, t.uv_b, t.uv_c] {
            assert!(
                (0.0..=1.0).contains(&uv.x) && (0.0..=1.0).contains(&uv.y),
                "{uv:?}"
            );
        }
    }
    // 1002 の面は右下のセル
    let t = &mapped.geometry.triangles()[2];
    assert!(t.uv_a.x >= 0.5 && t.uv_a.y < 0.5, "{:?}", t.uv_a);
    // 1 回の Undo で UDIM でなくなり、モデルも写す前の UV に戻る
    s.act(Action::Undo);
    assert!(s.doc().udim().is_none());
    s.sync_view3d();
    let back = s.view3d().full_model().unwrap();
    assert_eq!(
        back.geometry.triangles()[2].uv_a,
        before.geometry.triangles()[2].uv_a
    );
    s.act(Action::Redo);
    s.sync_view3d();
    assert!(s.doc().udim().is_some());
    // やめるのも 1 回の Undo
    s.act(Action::Udim(UdimAction::Toggle(id)));
    assert!(s.doc().udim().is_none());
    assert_eq!(s.doc().width(), SIZE);
    s.act(Action::Undo);
    assert!(s.doc().udim().is_some());
}

#[test]
fn a_new_tile_in_the_model_is_announced_and_syncing_adds_it() {
    let mut s = painted();
    s.view3d_mut().set_model(tiled_cube(false, None));
    s.sync_view3d();
    let id = uid(&s);
    s.act(Action::Udim(UdimAction::Toggle(id)));
    assert!(!s.udim_needs_sync(0));
    // モデルが替わって 1003 を使う: 描けない三角形を知らせ、合わせる項目が出る
    s.view3d_mut()
        .set_model(tiled_cube(false, Some(Vec2::new(2.0, 0.0))));
    s.set_lang(Lang::Ja);
    s.sync_view3d();
    assert!(
        s.notice_log().entries().any(|e| e
            .notice
            .text
            .contains("UDIM のタイルの表に無いタイルの三角形が 2 個")),
        "{:?}",
        s.notice_log()
            .entries()
            .map(|e| &e.notice.text)
            .collect::<Vec<_>>()
    );
    assert!(s.udim_needs_sync(0));
    s.act(Action::Udim(UdimAction::Sync(id)));
    assert_eq!(s.doc().udim().unwrap().tiles(), &[1001, 1002, 1003, 1011]);
    assert!(!s.udim_needs_sync(0));
    s.act(Action::Undo);
    assert_eq!(s.doc().udim().unwrap().tiles(), &[1001, 1002, 1011]);
}

#[test]
fn the_list_shows_the_tile_size_and_count() {
    let mut s = painted();
    assert_eq!(yolu_app::panels::texture_sets::size_label(s.doc()), "32");
    s.view3d_mut().set_model(tiled_cube(false, None));
    s.sync_view3d();
    let id = uid(&s);
    s.act(Action::Udim(UdimAction::Toggle(id)));
    assert_eq!(yolu_app::panels::texture_sets::size_label(s.doc()), "32 ×3");
}

#[test]
fn exporting_writes_one_file_per_tile_and_no_file_for_empty_cells() {
    let mut s = painted();
    s.view3d_mut().set_model(tiled_cube(false, None));
    s.sync_view3d();
    let id = uid(&s);
    s.act(Action::Udim(UdimAction::Toggle(id)));
    // 1002 のセルに青を塗る
    let layer = s.selected_layer().unwrap();
    for y in 0..SIZE {
        for x in SIZE..2 * SIZE {
            s.doc_mut()
                .set_pixel(layer, x, y, Rgba8::new(20, 40, 220, 255))
                .unwrap();
        }
    }
    let dir = std::env::temp_dir().join(format!("yolu-udim-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    s.act(Action::Export(ExportAction::TemplateTo {
        id: "unity-standard".into(),
        dir: dir.clone(),
        sets: None,
    }));
    s.wait_export();
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert!(!files.is_empty());
    for f in &files {
        let tile = f.rsplit('.').nth(1).unwrap();
        assert!(["1001", "1002", "1011"].contains(&tile), "{f}");
    }
    let base = files
        .iter()
        .find(|f| f.ends_with(".1002.png"))
        .unwrap()
        .replace(".1002.png", "");
    for tile in ["1001", "1002", "1011"] {
        assert!(
            files.contains(&format!("{base}.{tile}.png")),
            "{tile}: {files:?}"
        );
    }
    // 中身はセルの大きさで、1002 は青
    let png = image::open(dir.join(format!("{base}.1002.png")))
        .unwrap()
        .to_rgba8();
    assert_eq!((png.width(), png.height()), (SIZE, SIZE));
    assert_eq!(png.get_pixel(5, 5).0, [20, 40, 220, 255]);
    let png = image::open(dir.join(format!("{base}.1001.png")))
        .unwrap()
        .to_rgba8();
    assert_eq!(png.get_pixel(5, 5).0, [200, 60, 40, 255]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_saved_project_reopens_with_the_same_sheet() {
    let mut s = painted();
    s.view3d_mut().set_model(tiled_cube(false, None));
    s.sync_view3d();
    let id = uid(&s);
    s.act(Action::Udim(UdimAction::Toggle(id)));
    let dir = std::env::temp_dir().join(format!("yolu-udim-save-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("udim.ylp");
    yolu_app::project::save_from(&mut s, &path);
    let mut again = AppState::new(8, 8);
    yolu_app::project::open_into(&mut again, &path);
    assert_eq!(again.doc().udim(), s.doc().udim());
    again.view3d_mut().set_model(tiled_cube(false, None));
    again.sync_view3d();
    let t = &again.view3d().full_model().unwrap().geometry.triangles()[2];
    assert!(t.uv_a.x >= 0.5, "開き直してもモデルの UV はシートへ写る");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 試しの人形を読んで、右上腕の面を隠した状態（ポーズのセッション・隠した面・結び付けがある）。
fn figure_with_hidden_arm() -> AppState {
    let mut s = painted();
    s.act(Action::Pose(PoseAction::LoadFigure));
    assert!(s.view3d().pose.session.is_some(), "{}", s.message());
    let arm = s
        .view3d()
        .pose
        .session
        .as_ref()
        .unwrap()
        .rig
        .bones()
        .iter()
        .position(|b| b.name == "右上腕")
        .unwrap();
    pose::hide::hide_bone(&mut s, arm);
    assert!(pose::hide::hidden_total(&s) > 0, "{}", s.message());
    s
}

/// 3D の毎フレームの仕事（ポーズの読み込みの確認・セットの同期）を回す。
fn frame(s: &mut AppState) {
    pose::poll(s.view3d_mut());
    s.sync_view3d();
}

/// UDIM のセットにしても、ポーズのセッション・隠した面・モデルの結び付けは残る（セッションが持つ形の世代は写す前の形のもので、写した形の
/// 世代と比べて終わらない）。合わせる・UDIM をやめる・Undo の後も、ポーズを変えた後も同じ。
#[test]
fn the_pose_session_hidden_faces_and_binding_survive_udim() {
    let mut s = figure_with_hidden_arm();
    let hidden = pose::hide::hidden_total(&s);
    let bound = s.sets().current().bound_set();
    assert!(!bound.is_empty());
    let check = |s: &mut AppState, what: &str| {
        frame(s);
        assert!(
            s.view3d().pose.session.is_some(),
            "{what}: セッションが残る"
        );
        assert_eq!(pose::hide::hidden_total(s), hidden, "{what}: 隠した面");
        assert_eq!(s.view3d().material, bound, "{what}: 描くマテリアル");
        assert_eq!(s.sets().current().bound_set(), bound, "{what}: 結び付け");
    };
    check(&mut s, "UDIM の前");
    // UDIM にする（人形の UV は 1 つのタイルだけなので、文書を直に UDIM にして写しを当てる）
    s.doc_mut().enable_udim(vec![1001, 1002]).unwrap();
    s.sync_view3d();
    assert!(
        !s.view3d().latest_model().unwrap().unmapped_is_none(),
        "写しが当たっている"
    );
    check(&mut s, "UDIM にした後");
    // タイルを合わせる
    s.doc_mut().sync_udim([1003]).unwrap().unwrap();
    check(&mut s, "合わせた後");
    // ポーズを変える（写す前の形に当てて写し直す）
    let mut p = s.view3d().pose.session.as_ref().unwrap().pose().clone();
    p.locals[0].translation += yolu_core::glam::Vec3::new(0.0, 0.01, 0.0);
    pose::set_pose(s.view3d_mut(), p).unwrap();
    check(&mut s, "ポーズを変えた後");
    // Undo・やめる
    s.doc_mut().undo().unwrap();
    check(&mut s, "合わせるを戻した後");
    s.doc_mut().disable_udim().unwrap();
    check(&mut s, "UDIM をやめた後");
}

fn notices_containing(s: &AppState, text: &str) -> usize {
    s.notice_log()
        .entries()
        .filter(|e| e.notice.text.contains(text))
        .map(|e| e.count as usize)
        .sum()
}

/// 同じ形（ポーズが替わるたびに作り直す形）の写しを何度当てても、またぐ・表に無い三角形の知らせは出直さない。タイルの並びが替わると出直す。
#[test]
fn the_straddling_and_outside_notice_is_not_repeated_for_the_same_model_shape() {
    let mut s = painted();
    s.set_lang(Lang::Ja);
    s.view3d_mut().set_model(tiled_cube(false, None));
    s.sync_view3d();
    let id = uid(&s);
    s.act(Action::Udim(UdimAction::Toggle(id)));
    let text = "UDIM のタイルの表に無いタイルの三角形が 2 個";
    s.view3d_mut()
        .set_model(tiled_cube(false, Some(Vec2::new(2.0, 0.0))));
    s.sync_view3d();
    assert_eq!(notices_containing(&s, text), 1);
    // ポーズを受けるたびの入れ替え（世代だけが新しい同じ形）
    for _ in 0..3 {
        s.view3d_mut()
            .set_model(tiled_cube(false, Some(Vec2::new(2.0, 0.0))));
        s.sync_view3d();
    }
    assert_eq!(notices_containing(&s, text), 1, "同じ形では知らせ直さない");
    // 合わせて 1003 が表に載れば、表に無い三角形は無くなる。UDIM を入れ直して同じ形を読めば、また知らせる
    s.act(Action::Udim(UdimAction::Sync(id)));
    s.sync_view3d();
    s.act(Action::Undo);
    s.sync_view3d();
    assert_eq!(
        notices_containing(&s, text),
        2,
        "並びが替わったので知らせる"
    );
}

/// モデルの UV をシートへ写せないときは、そのセットの 3D の描きを理由つきで断り、理由は 1 度だけ知らせる。
#[test]
fn a_failed_udim_mapping_refuses_3d_painting_with_a_reason() {
    let mut s = painted();
    s.set_lang(Lang::Ja);
    s.view3d_mut().set_model(tiled_cube(true, None));
    s.sync_view3d();
    let id = uid(&s);
    s.act(Action::Udim(UdimAction::Toggle(id)));
    assert!(!s.view3d().material.is_empty());
    // 写しの失敗（形のつくりの不正）を、写す所の結果として入れる
    s.view3d_mut().udim_failure = Some(yolu_app::view3d::model::ViewError::BadMeshIndex);
    s.sync_view3d();
    s.sync_view3d();
    assert!(s.view3d().material.is_empty(), "描く面が無い");
    assert_eq!(
        notices_containing(&s, "UDIM のシートへモデルの UV を写せない"),
        1
    );
    assert!(s
        .region_missing_reason()
        .contains("UDIM のシートへモデルの UV を写せない"));
}
