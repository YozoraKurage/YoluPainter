//! 重なった UV: 重なりの図（表示と塗りの知らせ）・ベイクの優先（窓の操作・手で島を選ぶ・島のメニュー・焼いた値・古さ・保存と開き直し）・
//! 2D のポリゴン塗りつぶしで重なった島を選び替える・右クリックの島のメニュー。画面を描かない（窓の見た目と日英の絵は
//! `gui_view3d/overlap_uv_ui.rs`）。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{pos2, vec2, Pos2, Rect};
use yolu_app::bake::overlap::PriorityOp;
use yolu_app::bake::{BakeAction, BakeBackend};
use yolu_app::engine::composite_pixel;
use yolu_app::lang::Lang;
use yolu_app::region::tools::{begin_polygon, finish_drag, update_hover, Where};
use yolu_app::state::{Action, AppState, Tool};
use yolu_app::uv_wireframe::overlap::Built;
use yolu_app::view3d::model::ViewModel;
use yolu_core::geometry::{ModelMesh, Submesh, SurfaceRegionKind};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::mesh_maps::{MeshMapKind, MeshMapState, MeshOverlapList, MeshOverlapRule};

/// z = 0 の四角（x0〜x1・y0〜y1）を、UV の正方形（u0, v0, u1, v1）に写したメッシュ（三角形 2 つ）。
fn quad(name: &str, x: [f32; 2], y: [f32; 2], uv: [f32; 4]) -> ModelMesh {
    let p = |x: f32, y: f32| Vec3::new(x, y, 0.0);
    ModelMesh {
        name: name.into(),
        positions: vec![p(x[0], y[0]), p(x[1], y[0]), p(x[0], y[1]), p(x[1], y[1])],
        normals: Vec::new(),
        uvs: vec![
            Vec2::new(uv[0], uv[1]),
            Vec2::new(uv[2], uv[1]),
            Vec2::new(uv[0], uv[3]),
            Vec2::new(uv[2], uv[3]),
        ],
        // 試しのカメラ（yaw・pitch 0）から表に見える巻き
        submeshes: vec![Submesh {
            material: 0,
            indices: vec![0, 2, 1, 2, 3, 1],
        }],
    }
}

const SQUARE: [f32; 4] = [0.25, 0.25, 0.75, 0.75];
/// 重なった所の真ん中（UV）と、重ならない島の中（UV）。
const OVERLAP: (f32, f32) = (0.5, 0.5);
const APART: (f32, f32) = (0.1, 0.1);

/// ミラーの両側（−X の小さな四角 = 三角形 0・1、+X の大きな四角 = 2・3）を同じ UV に重ね、離れた島（4・5）を足したモデル。
fn mirrored() -> ViewModel {
    ViewModel::new(
        "鏡",
        vec![
            quad("左", [-2.0, -1.0], [0.0, 1.0], SQUARE),
            quad("右", [1.0, 3.0], [0.0, 2.0], SQUARE),
            quad("離れ", [5.0, 6.0], [0.0, 1.0], [0.0, 0.0, 0.2, 0.2]),
        ],
        vec![Some("材".into())],
        1,
    )
    .expect("モデル")
}

fn with_model(model: ViewModel) -> (AppState, Rect) {
    let mut s = AppState::new(64, 64);
    s.bake.backend = BakeBackend::Cpu;
    s.view3d.set_model(model);
    s.view3d.material = 0;
    s.view3d.camera.yaw = 0.0;
    s.view3d.camera.pitch = 0.0;
    (
        s,
        Rect::from_min_size(pos2(100.0, 100.0), vec2(600.0, 400.0)),
    )
}

fn at_uv(s: &AppState, rect: Rect, uv: (f32, f32)) -> Pos2 {
    let view = s.view.view(rect, s.doc.width(), s.doc.height());
    view.to_screen(
        uv.0 as f64 * s.doc.width() as f64,
        uv.1 as f64 * s.doc.height() as f64,
    )
}

fn at_model(s: &AppState, rect: Rect, p: Vec3) -> Pos2 {
    let view = s.view3d.camera.view(rect.width(), rect.height());
    let q = view.to_screen(p).expect("カメラの前");
    pos2(rect.left() + q.x, rect.top() + q.y)
}

fn wait_overlap(s: &mut AppState) -> Arc<Built> {
    let start = Instant::now();
    loop {
        s.poll_uv_overlap();
        if !s.uv_overlap.is_counting() {
            if let Some(b) = s.uv_overlap.built() {
                return b.clone();
            }
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "重なりを数え終わらない"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn bake(s: &mut AppState, a: BakeAction) {
    s.apply(Action::Bake(a));
}

/// 島の索引ができるまで待つ（入力も索引も別のスレッドで作る。画面は待たずに毎フレーム求める）。
fn wait_islands(s: &mut AppState) {
    let start = Instant::now();
    while s.overlap_islands(false).is_none() {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "島の索引ができない"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// 右クリックのメニューが島の索引を待っているあいだ、毎フレームの呼びを回して開くのを待つ。
fn settle_menu(s: &mut AppState, ctx: &egui::Context) {
    let start = Instant::now();
    while yolu_app::bake::overlap::menu_pending(s) {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "メニューが開かない"
        );
        yolu_app::bake::overlap::poll_menu(ctx, s);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn priority(s: &mut AppState, op: PriorityOp) {
    bake(s, BakeAction::Priority(op));
}

#[test]
fn headless_the_overlap_map_has_the_shared_texels_and_the_edges_of_both_islands() {
    let (mut s, _) = with_model(mirrored());
    let built = wait_overlap(&mut s);
    assert!(built.map.texel_count() > 0);
    assert_eq!(built.map.triangles(), &[0, 1, 2, 3], "離れた島は関わらない");
    // UV がぴったり重なった 2 つの島の、それぞれの外周（4 辺ずつ）
    assert_eq!(built.edges.len(), 8);
    let (x, y) = ((OVERLAP.0 * 64.0) as u32, (OVERLAP.1 * 64.0) as u32);
    assert!(built.map.contains(x, y));
    assert!(!built
        .map
        .contains((APART.0 * 64.0) as u32, (APART.1 * 64.0) as u32));
    // 同じモデル・セット・大きさでは数え直さない。位置だけ動かしたモデルでも数え直さない
    assert_eq!(s.uv_overlap.counts, 1);
    s.poll_uv_overlap();
    let mut lifted = mirrored();
    let moved: Vec<ModelMesh> = lifted
        .meshes
        .iter()
        .map(|m| ModelMesh {
            positions: m.positions.iter().map(|p| *p + Vec3::Z).collect(),
            ..m.clone()
        })
        .collect();
    lifted = ViewModel::new("鏡", moved, lifted.materials.clone(), 2).unwrap();
    s.view3d.set_model(lifted);
    s.poll_uv_overlap();
    assert_eq!(s.uv_overlap.counts, 1, "UV が同じなら数え直さない");
    // UV の違うモデルは数え直す（重ならないモデルは空の図）
    s.view3d.set_model(
        ViewModel::new(
            "離れ",
            vec![quad("一", [0.0, 1.0], [0.0, 1.0], [0.0, 0.0, 0.4, 0.4])],
            vec![Some("材".into())],
            3,
        )
        .unwrap(),
    );
    let built = wait_overlap(&mut s);
    assert_eq!(s.uv_overlap.counts, 2);
    assert!(built.map.is_empty() && built.edges.is_empty());
}

#[test]
fn headless_painting_overlapped_texels_tells_once_per_set_as_info() {
    for lang in Lang::ALL {
        let (mut s, _) = with_model(mirrored());
        s.lang = lang;
        wait_overlap(&mut s);
        let text = lang.pick(
            "重なった UV は同じテクセルを使うので、片側だけには描けません。",
            "Overlapping UVs share the same texels, so one side cannot be painted alone.",
        );
        // 重ならない所は知らせない
        s.note_overlap_canvas(APART.0 as f64 * 64.0, APART.1 as f64 * 64.0, 1.0);
        assert_ne!(s.message, text);
        // ブラシの半径が届けば知らせる（種類は済んだ知らせ。ログの窓には残らない）
        s.note_overlap_canvas(14.0, 14.0, 4.0);
        assert_eq!(s.message, text);
        let notice = s.current_notice().expect("知らせ");
        assert_eq!(notice.kind, yolu_app::notice::Kind::Info);
        assert_eq!(notice.source, yolu_app::notice::Source::Brush);
        assert!(s.notice_log.is_empty(), "ログの窓には出さない");
        // 同じセットでは 2 度目は出さない（2D でも 3D でも）
        s.message.clear();
        s.note_overlap_canvas(32.0, 32.0, 1.0);
        s.note_overlap_uv(Vec2::new(0.5, 0.5));
        assert!(s.message.is_empty(), "{}", s.message);
    }
    // 3D で描いた面の UV でも知らせる
    let (mut s, _) = with_model(mirrored());
    wait_overlap(&mut s);
    s.note_overlap_uv(Vec2::new(0.5, 0.5));
    assert!(s.message.contains("重なった UV"), "{}", s.message);
}

#[test]
fn headless_the_window_changes_the_sets_priority_with_one_undo_and_the_bake_follows_it() {
    let (mut s, _) = with_model(mirrored());
    s.bake.settings.maps = vec![MeshMapKind::Position];
    s.bake.settings.padding = 0;
    priority(&mut s, PriorityOp::Rule(MeshOverlapRule::PositiveX));
    assert_eq!(s.doc.bake_priority().rule, MeshOverlapRule::PositiveX);
    assert_eq!(s.doc.undo_count(), 1);
    assert!(s.modified);
    priority(&mut s, PriorityOp::SkipOutside(true));
    assert!(s.doc.bake_priority().skip_outside);
    assert_eq!(s.doc.undo_count(), 2);
    bake(&mut s, BakeAction::Start);
    s.wait_bake();
    let map = s
        .sets
        .current()
        .mesh_maps
        .get(MeshMapKind::Position)
        .cloned()
        .expect("焼いた");
    // 重なった所は +X の大きな四角（境界箱 −2〜6 の右の側）の値
    let x = map.raw_value(32, 32, 0).unwrap();
    assert!(x > 65535 / 2, "{x}");
    assert_eq!(
        s.mesh_map_check(0, MeshMapKind::Position).unwrap().state,
        MeshMapState::Current
    );
    // 決め方を変えると古い。取り消すと今に戻る
    priority(&mut s, PriorityOp::Rule(MeshOverlapRule::NegativeX));
    assert_eq!(
        s.mesh_map_check(0, MeshMapKind::Position).unwrap().state,
        MeshMapState::Stale
    );
    s.apply(Action::Undo);
    assert_eq!(s.doc.bake_priority().rule, MeshOverlapRule::PositiveX);
    assert_eq!(
        s.mesh_map_check(0, MeshMapKind::Position).unwrap().state,
        MeshMapState::Current
    );
}

#[test]
fn headless_picking_islands_in_2d_cycles_the_overlapped_ones_and_3d_takes_the_face() {
    let (mut s, rect) = with_model(mirrored());
    bake(&mut s, BakeAction::OpenWindow);
    priority(&mut s, PriorityOp::Pick(Some(MeshOverlapList::Skip)));
    assert!(s.bake.pick.is_some());
    let view = s.view.view(rect, 64, 64);
    let at = at_uv(&s, rect, OVERLAP);
    // 強調は選ぶ島（2 つの三角形）。範囲の道具を選んでいなくても出る（島はモデルの入力を作り終えてから）
    s.tool = Tool::Brush;
    wait_islands(&mut s);
    assert!(yolu_app::bake::overlap::update_hover(
        &mut s,
        Where::Canvas(&view),
        Some(at)
    ));
    assert_eq!(s.region_hover_len(), Some(2));
    let skipped =
        |s: &AppState| -> Vec<usize> { s.doc.bake_priority().skipped().iter().copied().collect() };
    let press = |s: &mut AppState, w: Where, p: Pos2| {
        assert!(yolu_app::bake::overlap::press(s, w, p));
    };
    press(&mut s, Where::Canvas(&view), at);
    assert_eq!(skipped(&s), vec![0], "番号の小さい −X の島");
    assert_eq!(s.doc.undo_count(), 1);
    // 同じ所を続けて押すと、前の選びを取り消して重なったもう一方の島へ
    press(&mut s, Where::Canvas(&view), at);
    assert_eq!(skipped(&s), vec![2]);
    assert_eq!(s.doc.undo_count(), 1, "選び替えは段を増やさない");
    // 強調も選んでいる島
    yolu_app::bake::overlap::update_hover(&mut s, Where::Canvas(&view), Some(at));
    assert_eq!(s.region_hover_len(), Some(2));
    press(&mut s, Where::Canvas(&view), at);
    assert_eq!(skipped(&s), vec![0], "一回りして戻る");
    // 離れた島を足し、もう一度押すと外れる
    let apart = at_uv(&s, rect, APART);
    press(&mut s, Where::Canvas(&view), apart);
    assert_eq!(skipped(&s), vec![0, 4]);
    press(&mut s, Where::Canvas(&view), apart);
    assert_eq!(skipped(&s), vec![0]);
    // 3D は当たった面の島（+X の四角）
    let surface = at_model(&s, rect, Vec3::new(2.0, 1.0, 0.0));
    press(&mut s, Where::Surface(rect), surface);
    assert_eq!(skipped(&s), vec![0, 2], "{}", s.message);
    // 一覧の名前（メッシュの名前と島の何番目か）と、一覧から外す
    let rows = s.overlap_island_rows(MeshOverlapList::Skip);
    assert_eq!(
        rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(),
        ["左 · 島 1", "右 · 島 1"]
    );
    priority(&mut s, PriorityOp::Remove(0));
    assert_eq!(skipped(&s), vec![2]);
    // 結び付けたモデルは今のモデル（ベイクの入力の指紋）
    let binding = s.bake_input().unwrap().topology_hash().to_owned();
    assert_eq!(s.doc.bake_priority().binding(), binding);
    assert!(!s.overlap_islands_foreign());
    // 同じ一覧の追加をもう一度押す・窓を閉じると、選ぶのをやめる（押下は道具に戻る）
    priority(&mut s, PriorityOp::Pick(Some(MeshOverlapList::Skip)));
    assert!(s.bake.pick.is_none());
    priority(&mut s, PriorityOp::Pick(Some(MeshOverlapList::Prefer)));
    bake(&mut s, BakeAction::CloseWindow);
    assert!(s.bake.pick.is_none());
    assert!(!yolu_app::bake::overlap::press(
        &mut s,
        Where::Canvas(&view),
        at
    ));
}

#[test]
fn headless_the_hand_picked_islands_are_not_baked() {
    let (mut s, rect) = with_model(mirrored());
    s.bake.settings.maps = vec![MeshMapKind::Position];
    s.bake.settings.padding = 0;
    bake(&mut s, BakeAction::OpenWindow);
    priority(&mut s, PriorityOp::Pick(Some(MeshOverlapList::Skip)));
    let view = s.view.view(rect, 64, 64);
    let at = at_uv(&s, rect, OVERLAP);
    assert!(yolu_app::bake::overlap::press(
        &mut s,
        Where::Canvas(&view),
        at
    ));
    bake(&mut s, BakeAction::Start);
    s.wait_bake();
    let map = s
        .sets
        .current()
        .mesh_maps
        .get(MeshMapKind::Position)
        .cloned()
        .expect("焼いた");
    // −X の島を焼かないので、重なった所は +X の島で、重なりの印も無い
    assert!(map.raw_value(32, 32, 0).unwrap() > 65535 / 2);
    assert!(map.coverage().iter().all(|c| *c != 2));
}

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> TempDir {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "yolu-app-overlap-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
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

#[test]
fn headless_the_priority_survives_save_and_reopen_as_version_33() {
    let dir = TempDir::new("save");
    let path = dir.0.join("a.ylp");
    let (mut s, _) = with_model(mirrored());
    priority(&mut s, PriorityOp::Rule(MeshOverlapRule::LargerArea));
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let project = yolu_io::Project::open(&path, &yolu_io::Limits::unbounded()).unwrap();
    assert_eq!(
        project.sets()[0].document.version(),
        yolu_io::BAKE_PRIORITY_VERSION
    );
    let mut t = AppState::new(64, 64);
    t.apply(Action::OpenProject(path.clone()));
    assert_eq!(t.doc.bake_priority().rule, MeshOverlapRule::LargerArea);
    assert_eq!(t.doc.undo_count(), 0);
    assert!(!t.modified);
}

/// 一部だけ重なった 2 つの島（P = 三角形 0・1 の UV 0.1〜0.5、Q = 2・3 の UV 0.3〜0.7）。
fn partly() -> ViewModel {
    ViewModel::new(
        "二つ",
        vec![
            quad("P", [0.0, 1.0], [0.0, 1.0], [0.1, 0.1, 0.5, 0.5]),
            quad("Q", [3.0, 4.0], [0.0, 1.0], [0.3, 0.3, 0.7, 0.7]),
        ],
        vec![Some("材".into())],
        1,
    )
    .unwrap()
}

#[test]
fn headless_polygon_fill_in_2d_cycles_the_overlapped_islands_on_the_same_spot() {
    let (mut s, rect) = with_model(partly());
    s.tool = Tool::PolygonFill;
    s.region.kind = SurfaceRegionKind::UvIsland;
    let view = s.view.view(rect, 64, 64);
    let only_p = (0.2, 0.2);
    let only_q = (0.6, 0.6);
    let painted = |s: &AppState, uv: (f32, f32)| {
        composite_pixel(&s.doc, (uv.0 * 64.0) as u32, (uv.1 * 64.0) as u32)[3] > 0
    };
    let click = |s: &mut AppState, uv: (f32, f32)| {
        let at = at_uv(s, rect, uv);
        assert!(begin_polygon(s, Where::Canvas(&view), at));
        assert!(finish_drag(s, false));
    };
    let hover = |s: &mut AppState, uv: (f32, f32)| -> Vec<u32> {
        let at = at_uv(s, rect, uv);
        update_hover(s, Where::Canvas(&view), Some(at));
        s.region
            .hover
            .as_ref()
            .map(|h| h.tris.to_vec())
            .unwrap_or_default()
    };
    let both = (0.4, 0.4);
    // 押す前の強調は番号の小さい島
    assert_eq!(hover(&mut s, both), vec![0, 1]);
    click(&mut s, both);
    assert!(painted(&s, only_p) && !painted(&s, only_q));
    let steps = s.doc.undo_count();
    assert_eq!(hover(&mut s, both), vec![0, 1], "塗った島を強調");
    // 同じ所をもう一度: P の塗りを取り消して Q を塗る（段は増えない）
    click(&mut s, both);
    assert!(!painted(&s, only_p) && painted(&s, only_q));
    assert_eq!(s.doc.undo_count(), steps);
    assert_eq!(hover(&mut s, both), vec![2, 3]);
    // さらにもう一度で P に戻る
    click(&mut s, both);
    assert!(painted(&s, only_p) && !painted(&s, only_q));
    // 別の所を押してから戻ると、選び替えは始めから（前の塗りは残る）
    click(&mut s, only_q);
    assert!(painted(&s, only_p) && painted(&s, only_q));
    click(&mut s, both);
    assert_eq!(s.doc.undo_count(), steps + 2);
    // 重なっていない所は続けて押しても選び替えない（同じ島をまた塗る）
    let before = s.doc.undo_count();
    click(&mut s, only_p);
    click(&mut s, only_p);
    assert!(s.doc.undo_count() >= before);
    assert!(painted(&s, only_p));
}

/// 開いている島のメニュー（島・見取り図からか・3D からか）。
fn island_menu(s: &AppState) -> Option<(usize, bool, bool)> {
    match s.popup.as_ref().map(|p| p.kind) {
        Some(yolu_app::state::PopupKind::BakeIsland {
            island,
            map,
            surface,
            ..
        }) => Some((island, map, surface)),
        _ => None,
    }
}

#[test]
fn headless_the_island_menu_puts_the_island_in_one_list_with_one_undo() {
    use yolu_app::bake::overlap::menu_entries;
    use yolu_app::ui::menu::{Check, Entry};
    let (mut s, _) = with_model(mirrored());
    s.bake_input().unwrap();
    let uid = s.sets.current().uid;
    let items = |s: &AppState, map: bool| -> Vec<(String, Check, bool)> {
        menu_entries(s, uid, 0, map)
            .into_iter()
            .filter_map(|e| match e {
                Entry::Item {
                    label,
                    check,
                    enabled,
                    ..
                } => Some((label, check, enabled)),
                _ => None,
            })
            .collect()
    };
    let none = Check::None;
    assert_eq!(
        items(&s, false),
        [
            ("優先して焼く".to_owned(), none, true),
            ("焼かない".to_owned(), none, true)
        ]
    );
    // 見取り図からは「外す」も（一覧に無い島では選べない）
    assert_eq!(
        items(&s, true),
        [
            ("優先する".to_owned(), none, true),
            ("焼かない".to_owned(), none, true),
            ("外す".to_owned(), none, false)
        ]
    );
    let set = |s: &mut AppState, list: Option<MeshOverlapList>| {
        priority(
            s,
            PriorityOp::Set {
                set: uid,
                island: 0,
                list,
            },
        )
    };
    let lists = |s: &AppState| -> (Vec<usize>, Vec<usize>) {
        let p = s.doc.bake_priority();
        (
            p.preferred().iter().copied().collect(),
            p.skipped().iter().copied().collect(),
        )
    };
    set(&mut s, Some(MeshOverlapList::Prefer));
    assert_eq!(lists(&s), (vec![0], vec![]));
    assert_eq!(s.doc.undo_count(), 1);
    assert_eq!(s.message, "島を「優先する島」に追加しました。");
    assert_eq!(items(&s, true)[0].1, Check::Checked, "今の状態にチェック");
    assert!(items(&s, true)[2].2, "一覧にある島は外せる");
    // もう一方の一覧へ移す・外す（どれも 1 回の Undo）
    set(&mut s, Some(MeshOverlapList::Skip));
    assert_eq!(lists(&s), (vec![], vec![0]));
    set(&mut s, None);
    assert_eq!(lists(&s), (vec![], vec![]));
    assert_eq!(s.message, "島を「焼かない島」から外しました。");
    assert_eq!(s.doc.undo_count(), 3);
    s.doc.undo().unwrap();
    assert_eq!(lists(&s), (vec![], vec![0]));
    // 開いた後にセットを替えた・代表でない番号（開いた後にモデルが替わった）は何もしない
    priority(
        &mut s,
        PriorityOp::Set {
            set: uid + 999,
            island: 2,
            list: Some(MeshOverlapList::Prefer),
        },
    );
    priority(
        &mut s,
        PriorityOp::Set {
            set: uid,
            island: 1,
            list: Some(MeshOverlapList::Prefer),
        },
    );
    assert_eq!(lists(&s), (vec![], vec![0]));
}

#[test]
fn headless_right_clicking_with_the_polygon_fill_opens_the_island_menu_and_cycles_the_overlaps() {
    use yolu_app::bake::overlap::{menu_press, menu_release};
    let (mut s, rect) = with_model(mirrored());
    let ctx = egui::Context::default();
    s.tool = Tool::PolygonFill;
    let view = s.view.view(rect, 64, 64);
    let at = at_uv(&s, rect, OVERLAP);
    let right = |s: &mut AppState, w: Where, from: Pos2, to: Pos2| {
        s.popup = None;
        menu_press(s, w, from);
        menu_release(s, &ctx, w, to);
        settle_menu(s, &ctx);
        island_menu(s)
    };
    // 2D: 番号の小さい側の島、同じ所を続けて右クリックすると重なった次の島（一回りして戻る）
    assert_eq!(
        right(&mut s, Where::Canvas(&view), at, at),
        Some((0, false, false))
    );
    assert_eq!(
        right(&mut s, Where::Canvas(&view), at, at),
        Some((2, false, false))
    );
    assert_eq!(
        right(&mut s, Where::Canvas(&view), at, at),
        Some((0, false, false))
    );
    // 開いている間は、道具によらずその島を強調する（2D の側）
    s.tool = Tool::Brush;
    assert!(yolu_app::bake::overlap::update_hover(
        &mut s,
        Where::Canvas(&view),
        None
    ));
    assert_eq!(s.region_hover_len(), Some(2));
    // 窓を閉じていても、メニューを開いている間は島の入力を手放さない
    s.release_idle_bake_input();
    assert!(matches!(s.overlap_islands(false), Some(Ok(_))));
    // 動かしてから離した・ほかの道具・何も無い所では開かない
    assert_eq!(
        right(&mut s, Where::Canvas(&view), at, at + vec2(10.0, 0.0)),
        None
    );
    s.tool = Tool::Brush;
    assert_eq!(right(&mut s, Where::Canvas(&view), at, at), None);
    s.tool = Tool::PolygonFill;
    let empty = at_uv(&s, rect, (0.9, 0.9));
    assert_eq!(right(&mut s, Where::Canvas(&view), empty, empty), None);
    // 3D: 当たった面の島（+X の四角）
    let surface = at_model(&s, rect, Vec3::new(2.0, 1.0, 0.0));
    assert_eq!(
        right(&mut s, Where::Surface(rect), surface, surface),
        Some((2, false, true))
    );
    // 2D で押して 3D で離した（別の画面）は開かない
    s.popup = None;
    menu_press(&mut s, Where::Canvas(&view), at);
    menu_release(&mut s, &ctx, Where::Surface(rect), at);
    assert_eq!(island_menu(&s), None);
}

#[test]
fn headless_the_right_click_menu_does_not_wait_for_the_island_index_and_lets_go_after() {
    use yolu_app::bake::overlap::{menu_pending, menu_press, menu_release, poll_menu};
    let (mut s, rect) = with_model(mirrored());
    let ctx = egui::Context::default();
    s.tool = Tool::PolygonFill;
    let view = s.view.view(rect, 64, 64);
    let at = at_uv(&s, rect, OVERLAP);
    let w = Where::Canvas(&view);
    // 入力も島の索引も作っていない最初の右クリック: 離したところでは開かず（UI のスレッドで作らない）、できたフレームで開く
    menu_press(&mut s, w, at);
    menu_release(&mut s, &ctx, w, at);
    assert!(menu_pending(&s));
    assert_eq!(island_menu(&s), None);
    settle_menu(&mut s, &ctx);
    assert_eq!(island_menu(&s), Some((0, false, false)));
    // 開いている間は索引を持つ。閉じて 1 フレーム回すと手放す（次の右クリックは作り直す）
    s.release_idle_bake_input();
    assert!(matches!(s.overlap_islands(false), Some(Ok(_))));
    s.popup = None;
    s.release_idle_bake_input();
    assert!(
        s.overlap_islands(false).is_none(),
        "メニューを閉じたあとも索引か入力を持っている"
    );
    // 待っている間に道具を替えたら、開かずにやめる
    s.release_idle_bake_input();
    menu_press(&mut s, w, at);
    menu_release(&mut s, &ctx, w, at);
    assert!(menu_pending(&s));
    s.tool = Tool::Brush;
    poll_menu(&ctx, &mut s);
    assert!(!menu_pending(&s));
    assert_eq!(island_menu(&s), None);
    // 待っている間に新しく押したら、前の頼みは捨てる
    s.tool = Tool::PolygonFill;
    s.release_idle_bake_input();
    menu_press(&mut s, w, at);
    menu_release(&mut s, &ctx, w, at);
    assert!(menu_pending(&s));
    menu_press(&mut s, w, at);
    assert!(!menu_pending(&s));
    // 下に何も無い所は、索引を待たずに終える
    s.popup = None;
    let empty = at_uv(&s, rect, (0.9, 0.9));
    menu_press(&mut s, w, empty);
    menu_release(&mut s, &ctx, w, empty);
    assert!(!menu_pending(&s));
    assert_eq!(island_menu(&s), None);
}
