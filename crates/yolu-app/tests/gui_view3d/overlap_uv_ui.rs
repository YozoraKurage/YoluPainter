//! 重なった UV の画面: ベイクのウィンドウの「重なった UV」の欄（決め方・0〜1 の外のアイランド・手で選んだアイランドの一覧・UV の見取り図とアイランドのメニュー）、
//! ポリゴン塗りつぶしの右クリックのアイランドのメニュー（2D・3D）、2D のキャンバスの重なりの色とアイランドの縁、表示のメニューの入切、設定の色の並び、
//! 描いたときの知らせ。日本語と英語の絵。操作の中身（焼いた値・保存・選び替え）は `headless/overlap_uv.rs`。
use crate::common;

use common::*;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, SnapshotResults};
use yolu_app::bake::overlap::PriorityOp;
use yolu_app::bake::window::Page;
use yolu_app::bake::BakeAction;
use yolu_app::lang::Lang;
use yolu_app::region::tools::Where;
use yolu_app::state::{Action, Tool};
use yolu_app::view3d::model::ViewModel;
use yolu_app::YoluApp;
use yolu_core::geometry::{ModelMesh, Submesh};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::mesh_maps::{MeshOverlapList, MeshOverlapRule};

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
        submeshes: vec![Submesh {
            material: 0,
            indices: vec![0, 2, 1, 2, 3, 1],
        }],
    }
}

/// ミラーの両側を同じ UV に重ね（左右の四角）、離れたアイランドを足したモデル。
fn mirrored() -> ViewModel {
    let square = [0.25, 0.25, 0.75, 0.75];
    ViewModel::new(
        "鏡",
        vec![
            quad("左", [-2.0, -1.0], [0.0, 1.0], square),
            quad("右", [1.0, 3.0], [0.0, 2.0], square),
            quad("離れ", [5.0, 6.0], [0.0, 1.0], [0.0, 0.0, 0.2, 0.2]),
        ],
        vec![Some("材".into())],
        1,
    )
    .expect("モデル")
}

fn apply(h: &mut Harness<'_, YoluApp>, action: Action) {
    h.state_mut().state.apply(action);
    h.run();
}

fn with_model(lang: Lang) -> Harness<'static, YoluApp> {
    let mut h = app(1280.0, 860.0, 64);
    h.state_mut().state.set_language(lang);
    {
        let s = &mut h.state_mut().state;
        s.view3d.set_model(mirrored());
    }
    h.run();
    h
}

/// 重なりを数え終えるまでフレームを回す。
fn counted(h: &mut Harness<'_, YoluApp>) {
    let start = std::time::Instant::now();
    loop {
        h.step();
        let s = &h.state().state;
        if !s.uv_overlap.is_counting() && s.uv_overlap.built().is_some() {
            break;
        }
        assert!(start.elapsed().as_secs() < 60, "重なりを数え終わらない");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    h.run();
}

/// ウィンドウの中だけを撮る。
fn shot(h: &mut Harness<'_, YoluApp>, window: &str, name: &str) {
    let rect = yolu_app::windows::window_rect(&h.ctx, window)
        .unwrap_or_else(|| panic!("{window} を描いていない"));
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
fn the_overlap_page_of_the_bake_window_sets_the_current_sets_priority() {
    for lang in Lang::ALL {
        let mut h = with_model(lang);
        apply(&mut h, Action::Bake(BakeAction::OpenWindow));
        h.state_mut().state.bake.window.as_mut().unwrap().page = Page::Overlap;
        h.run();
        // 決め方のボタン（押すと文書が変わる。1 回の Undo）
        h.get_by_role_and_label(Role::Button, "+X").click();
        h.run();
        assert_eq!(
            h.state().state.doc.bake_priority().rule,
            MeshOverlapRule::PositiveX
        );
        assert_eq!(h.state().state.doc.undo_count(), 1);
        let outside = lang.pick(
            "0〜1 の外のアイランドを焼かない",
            "Skip islands outside 0–1",
        );
        h.get_by_role_and_label(Role::CheckBox, outside).click();
        h.run();
        assert!(h.state().state.doc.bake_priority().skip_outside);
        // 手で選ぶ: 「追加」で選び始め、2D・3D で押したアイランドが一覧に入る
        let add = lang.pick(
            "2D か 3D で押したアイランドを「焼かないアイランド」に追加する（入っているアイランドを押すと外す・Esc でやめる）",
            "Add the island you click in 2D or 3D to \"Islands Not Baked\" (clicking one already listed removes it; Esc stops)",
        );
        h.get_by_label(add).click();
        h.run();
        assert!(
            h.state().state.bake.pick.is_some(),
            "{} {:?}",
            h.state().state.message,
            h.state().state.bake.window.as_ref().map(|w| w.page)
        );
        let rect = canvas_rect(&h);
        {
            let s = &mut h.state_mut().state;
            s.bake_input().unwrap();
            let view = s.view.view(rect, 64, 64);
            let at = view.to_screen(32.0, 32.0);
            assert!(yolu_app::bake::overlap::press(s, Where::Canvas(&view), at));
            let apart = view.to_screen(6.0, 6.0);
            s.apply(Action::Bake(BakeAction::Priority(PriorityOp::Pick(Some(
                MeshOverlapList::Prefer,
            )))));
            assert!(yolu_app::bake::overlap::press(
                s,
                Where::Canvas(&view),
                apart
            ));
            s.apply(Action::Bake(BakeAction::Priority(PriorityOp::Pick(None))));
        }
        h.run();
        let rows = h
            .state_mut()
            .state
            .overlap_island_rows(MeshOverlapList::Skip);
        assert_eq!(rows.len(), 1);
        shot(
            &mut h,
            "bake",
            &format!("bake_overlap_page_{}", lang.pick("ja", "en")),
        );
        // 一覧から外す（並びは「優先するアイランド」・「焼かないアイランド」の順。焼かないアイランドの行のボタン）
        h.query_all_by_label(lang.pick("一覧から外す", "Remove from the list"))
            .last()
            .expect("外すボタン")
            .click();
        h.run();
        assert!(h.state().state.doc.bake_priority().skipped().is_empty());
        assert_eq!(h.state().state.doc.bake_priority().preferred().len(), 1);
        // Esc は選ぶのをやめるだけ（ウィンドウは閉じない）
        apply(
            &mut h,
            Action::Bake(BakeAction::Priority(PriorityOp::Pick(Some(
                MeshOverlapList::Skip,
            )))),
        );
        key(&h, egui::Key::Escape, egui::Modifiers::NONE);
        h.run();
        assert!(h.state().state.bake.pick.is_none());
        assert!(h.state().state.bake.window.is_some());
    }
}

#[test]
fn the_canvas_shows_overlapped_texels_and_the_view_menu_turns_them_off() {
    let mut results = SnapshotResults::new();
    for lang in Lang::ALL {
        let mut h = with_model(lang);
        counted(&mut h);
        assert!(h.state().state.prefs.settings.uv_overlap, "既定は入");
        if lang == Lang::Ja {
            h.event(egui::Event::PointerGone);
            h.step();
            h.snapshot("uv_overlap_canvas");
            results.extend_harness(&mut h);
        }
        // 表示のメニューの入切
        let at = menu_title(&h, lang.pick("表示", "View")).center();
        click(&mut h, at);
        let item = popup_item(&h, lang.pick("重なった UV", "Overlapping UVs"));
        h.event(egui::Event::PointerMoved(item.center()));
        h.step();
        h.snapshot(format!("uv_overlap_menu_{}", lang.pick("ja", "en")));
        results.extend_harness(&mut h);
        click(&mut h, item.center());
        assert!(!h.state().state.prefs.settings.uv_overlap);
        apply(&mut h, Action::ToggleUvOverlap);
        assert!(h.state().state.prefs.settings.uv_overlap);
    }
}

#[test]
fn painting_overlapped_texels_in_the_canvas_tells_once() {
    let mut h = with_model(Lang::Ja);
    counted(&mut h);
    h.state_mut().state.tool = Tool::Brush;
    h.run();
    let rect = canvas_rect(&h);
    let view = {
        let s = &h.state().state;
        s.view.view(rect, 64, 64)
    };
    let a = view.to_screen(30.0, 30.0);
    let b = view.to_screen(34.0, 34.0);
    drag(&mut h, &[a, b]);
    let text = "重なった UV は同じテクセルを使うので、片側だけには描けません。";
    assert_eq!(h.state().state.message, text);
    h.state_mut().state.message.clear();
    drag(&mut h, &[a, b]);
    assert_ne!(h.state().state.message, text, "セットごとに 1 度だけ");
}

#[test]
fn the_settings_row_has_the_overlap_color_after_the_wireframe_color() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 860.0, 64);
        h.state_mut().state.set_language(lang);
        apply(
            &mut h,
            Action::Prefs(yolu_app::prefs::PrefsAction::OpenAt(
                yolu_app::prefs::Category::View3d,
            )),
        );
        let wire = h
            .get_by_role_and_label(
                Role::ColorWell,
                lang.pick(
                    "UV ワイヤーフレームの色と不透明度",
                    "UV wireframe color and opacity",
                ),
            )
            .rect();
        let over = h
            .get_by_role_and_label(
                Role::ColorWell,
                lang.pick(
                    "重なった UV の色と不透明度",
                    "Overlapping UV color and opacity",
                ),
            )
            .rect();
        assert!(over.left() > wire.right(), "ワイヤーフレームの色の後ろ");
        assert!((over.center().y - wire.center().y).abs() < 1.0, "同じ行");
        h.get_by_role_and_label(
            Role::ColorWell,
            lang.pick(
                "重なった UV の色と不透明度",
                "Overlapping UV color and opacity",
            ),
        )
        .click();
        h.run();
        // 設定のウィンドウの全体には機械ごとの値（メモリの自動の上限・スレッドの数・設定のフォルダ）が出るので、色の行と色のウィンドウだけを撮る
        let prefs = yolu_app::prefs::last_rect(&h.ctx).expect("設定のウィンドウ");
        let row = egui::Rect::from_min_max(
            // 左の区分の列（幅 210）は撮らない
            egui::pos2(prefs.left() + 210.0, wire.top() - 4.0),
            egui::pos2(over.right() + 4.0, wire.bottom() + 4.0),
        );
        shot_row_and_color_window(
            &mut h,
            row,
            &format!("uv_overlap_color_{}", lang.pick("ja", "en")),
        );
        assert_eq!(
            h.state().state.prefs.settings.uv_overlap_color,
            yolu_app::uv_wireframe::DEFAULT_OVERLAP_COLOR
        );
    }
}

/// 行の矩形と、開いている色のウィンドウを、上下に並べた 1 枚の絵にして正解と比べる。
fn shot_row_and_color_window(h: &mut Harness<'_, YoluApp>, row: egui::Rect, name: &str) {
    let window = yolu_app::panels::color_window::rect(&h.ctx).expect("色のウィンドウが開いている");
    let image = h.render().expect("描画");
    let crop = |r: egui::Rect| {
        image::imageops::crop_imm(
            &image,
            r.left().floor() as u32,
            r.top().floor() as u32,
            r.width().ceil() as u32,
            r.height().ceil() as u32,
        )
        .to_image()
    };
    let (a, b) = (crop(row), crop(window));
    let mut out = image::RgbaImage::new(a.width().max(b.width()), a.height() + b.height());
    image::imageops::overlay(&mut out, &a, 0, 0);
    image::imageops::overlay(&mut out, &b, 0, a.height() as i64);
    egui_kittest::image_snapshot(&out, name);
}

/// ウィンドウと開いているポップアップを合わせた所だけを撮る。
fn shot_with_popup(h: &mut Harness<'_, YoluApp>, window: &str, name: &str) {
    let mut rect = yolu_app::windows::window_rect(&h.ctx, window)
        .unwrap_or_else(|| panic!("{window} を描いていない"));
    if let Some(p) = &h.state().state.popup {
        rect = rect.union(p.state.rect);
    }
    let image = h.render().expect("描画");
    let rect = rect.intersect(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(image.width() as f32, image.height() as f32),
    ));
    let cropped = image::imageops::crop_imm(
        &image,
        rect.left().floor() as u32,
        rect.top().floor() as u32,
        rect.width().floor() as u32,
        rect.height().floor() as u32,
    )
    .to_image();
    egui_kittest::image_snapshot(&cropped, name);
}

/// 開いているアイランドのメニュー（セット・アイランド・見取り図からか・3D からか）。
/// 開いたメニューの項目の矩形。メニューを開いた状態にしたフレームで、まだ描いていなければ描くまで回す（30 フレームまで）。
/// 出なければ、メニューの状態を添えて落とす。
fn menu_item(h: &mut Harness<'_, YoluApp>, label: &str) -> egui::Rect {
    for _ in 0..30 {
        if h.query_by_label(label).is_some() {
            return popup_item(h, label);
        }
        h.step();
    }
    let popup = h
        .state()
        .state
        .popup
        .as_ref()
        .map(|p| format!("{:?} {:?}", p.kind, p.state.rect));
    panic!("{label} が描かれない（メニュー: {popup:?}）");
}

fn island_menu(h: &Harness<'_, YoluApp>) -> Option<(usize, bool, bool)> {
    match h.state().state.popup.as_ref().map(|p| p.kind) {
        Some(yolu_app::state::PopupKind::BakeIsland {
            island,
            map,
            surface,
            ..
        }) => Some((island, map, surface)),
        _ => None,
    }
}

/// ウィンドウの「重なった UV」の項目を出し、モデルの入力（アイランド）を作り終えるまで回す。
fn overlap_page(lang: Lang) -> Harness<'static, YoluApp> {
    let mut h = with_model(lang);
    counted(&mut h);
    apply(&mut h, Action::Bake(BakeAction::OpenWindow));
    h.state_mut().state.bake.window.as_mut().unwrap().page = Page::Overlap;
    h.state_mut().state.bake_input().unwrap();
    h.run();
    h
}

/// 見取り図の上の、UV の点（拡大 1。0〜1 の正方形のまわりに 10 点の余白）。
fn map_point(h: &Harness<'_, YoluApp>, u: f32, v: f32) -> egui::Pos2 {
    let r = h
        .state()
        .state
        .bake
        .window
        .as_ref()
        .unwrap()
        .map
        .rect
        .shrink(10.0);
    egui::pos2(r.left() + u * r.width(), r.bottom() - v * r.height())
}

#[test]
fn the_uv_map_of_the_overlap_page_opens_the_island_menu_and_cycles_the_overlapped_islands() {
    for lang in Lang::ALL {
        let mut h = overlap_page(lang);
        let r = h.state().state.bake.window.as_ref().unwrap().map.rect;
        assert!(
            r.width() >= 160.0 && (r.width() - r.height()).abs() < 0.5,
            "{r:?}"
        );
        // 重ならないアイランドを優先に、片側を焼かないにしておく（絵に優先の縁と斜線が出る）
        let apart = map_point(&h, 0.1, 0.1);
        click(&mut h, apart);
        assert_eq!(island_menu(&h), Some((4, true, false)), "離れたアイランド");
        let item = popup_item(&h, lang.pick("優先する", "Prefer")).center();
        click(&mut h, item);
        assert!(h.state().state.doc.bake_priority().preferred().contains(&4));
        assert_eq!(h.state().state.doc.undo_count(), 1, "1 回の Undo");
        // 重なった所: 押すと番号の小さい側のアイランド、続けて押すと次のアイランド
        let at = map_point(&h, 0.5, 0.4);
        click(&mut h, at);
        assert_eq!(island_menu(&h), Some((0, true, false)));
        click(&mut h, at);
        assert_eq!(
            island_menu(&h),
            Some((2, true, false)),
            "続けて押すと次のアイランド"
        );
        // メニューを開いているアイランドは、キャンバスでも強調する（同じアイランドの UV の輪郭）
        assert!(h.state().state.region_hover_len().is_some());
        let skip = popup_item(&h, lang.pick("焼かない", "Skip"));
        h.event(egui::Event::PointerMoved(skip.center()));
        h.step();
        shot_with_popup(
            &mut h,
            "bake",
            &format!("bake_overlap_map_menu_{}", lang.pick("ja", "en")),
        );
        click(&mut h, skip.center());
        assert!(h.state().state.doc.bake_priority().skipped().contains(&2));
        // もう一度押すと最初のアイランドへ戻る。「外す」は一覧に無いアイランドでは選べない
        click(&mut h, at);
        assert_eq!(island_menu(&h), Some((0, true, false)));
        let entries = yolu_app::shell::popup_entries(
            &h.state().state,
            h.state().state.popup.as_ref().unwrap().kind,
        );
        let remove = entries
            .iter()
            .find(|e| e.label() == Some(lang.pick("外す", "Remove")))
            .expect("外す");
        assert!(matches!(
            remove,
            yolu_app::ui::menu::Entry::Item { enabled: false, .. }
        ));
        key(&h, egui::Key::Escape, egui::Modifiers::NONE);
        h.run();
        // 焼かないアイランドを外す（続けて押した次のアイランド）
        click(&mut h, at);
        assert_eq!(island_menu(&h), Some((2, true, false)));
        let item = popup_item(&h, lang.pick("外す", "Remove")).center();
        click(&mut h, item);
        assert!(h.state().state.doc.bake_priority().skipped().is_empty());
        // ポインタを置いたアイランドは、ウィンドウの外でも強調するアイランド
        move_to(&h, apart);
        h.run();
        assert_eq!(h.state().state.bake.map_hover, Some(4));
        // ホイールは見取り図の拡大（ポインタの下の UV は動かない）。右の欄は送らない
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 3.0),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        });
        h.run();
        let zoom = h.state().state.bake.window.as_ref().unwrap().map.zoom();
        assert!(zoom > 1.5, "{zoom}");
        assert_eq!(
            h.state().state.bake.map_hover,
            Some(4),
            "ポインタの下の UV は動かない"
        );
        // ウィンドウを閉じると、強調するアイランドも無くなる
        apply(&mut h, Action::Bake(BakeAction::CloseWindow));
        assert_eq!(h.state().state.bake.map_hover, None);
    }
}

#[test]
fn the_uv_map_shows_the_preferred_and_skipped_islands() {
    for lang in Lang::ALL {
        let mut h = overlap_page(lang);
        {
            let s = &mut h.state_mut().state;
            let uid = s.sets.current().uid;
            s.apply(Action::Bake(BakeAction::Priority(PriorityOp::Set {
                set: uid,
                island: 4,
                list: Some(MeshOverlapList::Prefer),
            })));
            s.apply(Action::Bake(BakeAction::Priority(PriorityOp::Set {
                set: uid,
                island: 0,
                list: Some(MeshOverlapList::Skip),
            })));
        }
        h.run();
        // 重なった所にポインタを置く（重なった片側のアイランドの強調）
        let at = map_point(&h, 0.5, 0.4);
        move_to(&h, at);
        h.run();
        assert_eq!(h.state().state.bake.map_hover, Some(0));
        let rect = yolu_app::windows::window_rect(&h.ctx, "bake").unwrap();
        let image = h.render().expect("描画");
        let cropped = image::imageops::crop_imm(
            &image,
            rect.left().floor() as u32,
            rect.top().floor() as u32,
            rect.width().ceil() as u32,
            rect.height().ceil() as u32,
        )
        .to_image();
        egui_kittest::image_snapshot(
            &cropped,
            format!("bake_overlap_map_{}", lang.pick("ja", "en")),
        );
    }
}

#[test]
fn right_clicking_an_island_with_the_polygon_fill_opens_its_bake_menu() {
    let mut results = SnapshotResults::new();
    for lang in Lang::ALL {
        let mut h = with_model(lang);
        counted(&mut h);
        h.state_mut().state.tool = Tool::PolygonFill;
        h.state_mut().state.bake_input().unwrap();
        h.run();
        let rect = canvas_rect(&h);
        let view = h.state().state.view.view(rect, 64, 64);
        let at = view.to_screen(32.0, 26.0);
        let right = |h: &mut Harness<'_, YoluApp>, p: egui::Pos2| {
            press(h, p, egui::PointerButton::Secondary);
            h.step();
            release(h, p, egui::PointerButton::Secondary);
            h.run();
        };
        right(&mut h, at);
        assert_eq!(island_menu(&h), Some((0, false, false)));
        let prefer = menu_item(&mut h, lang.pick("優先して焼く", "Prefer in Bake"));
        h.event(egui::Event::PointerMoved(prefer.center()));
        h.step();
        h.snapshot(format!(
            "polygon_fill_island_menu_{}",
            lang.pick("ja", "en")
        ));
        // 開いているメニューの外で、同じ所をもう一度右クリックすると次のアイランド（重なった片側）
        right(&mut h, at);
        assert_eq!(island_menu(&h), Some((2, false, false)), "次のアイランド");
        let item = menu_item(&mut h, lang.pick("優先して焼く", "Prefer in Bake")).center();
        click(&mut h, item);
        assert!(h.state().state.doc.bake_priority().preferred().contains(&2));
        assert_eq!(h.state().state.doc.undo_count(), 1);
        // チェックのある項目を選ぶと外す
        right(&mut h, at);
        right(&mut h, at);
        assert_eq!(island_menu(&h), Some((2, false, false)));
        let item = menu_item(&mut h, lang.pick("優先して焼く", "Prefer in Bake")).center();
        click(&mut h, item);
        assert!(h.state().state.doc.bake_priority().preferred().is_empty());
        // 何も無い所では開かない
        let empty = view.to_screen(60.0, 60.0);
        right(&mut h, empty);
        assert_eq!(island_menu(&h), None);
        // ほかのツールでは開かない（右ボタンはスポイトになり、透明の所を取ろうとした知らせが出る。あとの絵に写らないよう、前の知らせへ戻す）
        let (message, toast) = {
            let s = &h.state().state;
            (s.message.clone(), s.toast.clone())
        };
        h.state_mut().state.tool = Tool::Brush;
        h.run();
        right(&mut h, at);
        assert_eq!(island_menu(&h), None);
        {
            let s = &mut h.state_mut().state;
            s.message = message;
            s.toast = toast;
        }
        // 3D: 動かさずに離した右クリックは当たった面のアイランド（+X の四角）のメニュー、3D の面を強調する。右ドラッグは回すだけ
        h.state_mut().state.tool = Tool::PolygonFill;
        {
            let s = &mut h.state_mut().state;
            s.view3d.camera.yaw = 0.0;
            s.view3d.camera.pitch = 0.0;
        }
        click_tab(&mut h, yolu_app::Tab::View3d);
        h.run();
        let rect = h.state().view3d_rect().expect("3D のタブを描いた");
        let at = {
            let v = h
                .state()
                .state
                .view3d
                .camera
                .view(rect.width(), rect.height());
            let q = v
                .to_screen(yolu_core::glam::Vec3::new(2.0, 1.0, 0.0))
                .expect("カメラの前");
            egui::pos2(rect.left() + q.x, rect.top() + q.y)
        };
        right(&mut h, at);
        assert_eq!(
            island_menu(&h),
            Some((2, false, true)),
            "{}",
            h.state().state.message
        );
        let hover = h.state().state.region.hover.as_ref().expect("強調");
        assert!(hover.on_surface && hover.tris.len() == 2);
        let prefer = menu_item(&mut h, lang.pick("優先して焼く", "Prefer in Bake"));
        h.event(egui::Event::PointerMoved(prefer.center()));
        h.step();
        h.snapshot(format!(
            "polygon_fill_island_menu_3d_{}",
            lang.pick("ja", "en")
        ));
        key(&h, egui::Key::Escape, egui::Modifiers::NONE);
        h.run();
        assert_eq!(island_menu(&h), None);
        let yaw = h.state().state.view3d.camera.yaw;
        press(&h, at, egui::PointerButton::Secondary);
        h.step();
        move_to(&h, at + egui::vec2(60.0, 0.0));
        h.step();
        release(
            &h,
            at + egui::vec2(60.0, 0.0),
            egui::PointerButton::Secondary,
        );
        h.run();
        assert_eq!(island_menu(&h), None, "回しただけ");
        assert_ne!(h.state().state.view3d.camera.yaw, yaw);
        results.extend_harness(&mut h);
    }
}
