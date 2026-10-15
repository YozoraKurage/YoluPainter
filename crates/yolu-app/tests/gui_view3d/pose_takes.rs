//! ポーズの欄の「テイク」の節の試験（egui_kittest。描画は wgpu のソフトの描画）: テイクのあるモデルだけに節が出る・テイクを選ぶ箱と
//! ポップアップ・フレームのつまみ・「ポーズにする」・日英（英語に日本語が残らない・文字が切れない・「…」に詰められない）。試験のモデルは
//! コードで組んだ ASCII の FBX（`fbx_ascii::arm_takes_scene`。ユーザーの FBX は使わない）。
use crate::common;
use crate::common::drive::Drive;

use common::livelink::fbx_ascii;
use common::*;
use egui::{epaint::Shape, pos2, vec2, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::lang::Lang;
use yolu_app::pen::PenInput;
use yolu_app::state::{Action, AppState};
use yolu_app::ui::widgets;
use yolu_app::view3d::pose::{self, takes, PoseAction};
use yolu_app::{Tab, YoluApp};
use yolu_model::ModelLimits;

type H = Harness<'static, YoluApp>;

fn app_in(width: f32, height: f32, lang: Lang) -> H {
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(width, height))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .renderer(common::shared_gpu::renderer())
        .build_eframe(move |cc| {
            with_render_state_cpu_canvas(
                YoluApp::for_context(
                    &cc.egui_ctx,
                    AppState::new_in(256, 256, lang),
                    PenInput::detached(),
                ),
                cc.wgpu_render_state.as_ref(),
            )
        });
    // 中央は 1 つの組（`common::app` と同じ並び）
    h.state_mut().dock = common::tabbed_center_dock(width);
    h.run();
    h
}

/// FBX を書いて、裏で読んで 3D ビューに入れる（ポーズの欄のタブも出る。プロジェクトのモデルとして）。
fn load(h: &mut H, scene: &fbx_ascii::Scene, file: &str) -> std::path::PathBuf {
    let dir = common::tmp::test_dir("pose-takes");
    let path = dir.join(file);
    std::fs::write(&path, scene.to_ascii()).unwrap();
    let job = pose::prepare_fbx(h.view3d_mut(), &path, ModelLimits::default());
    let start = std::time::Instant::now();
    let prepared = loop {
        if let Some(r) = job.poll() {
            break r.expect("読める");
        }
        assert!(start.elapsed().as_secs() < 120, "読み込みが終わらない");
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    pose::install_prepared(h.view3d_mut(), prepared);
    // プロジェクトのモデルのファイル（ポーズと欄の選びは、そのモデルのものだけ .ylp に残る）
    h.np_mut().model_file = Some(path.clone());
    h.run();
    path
}

/// ポーズのタブをドックから外して、左の上に浮いたウィンドウにする。
/// ポーズのタブを外すときに渡す矩形（egui_dock は外したウィンドウの大きさをこの 0.8 倍にする）。
fn float_rect() -> Rect {
    Rect::from_min_size(pos2(8.0, 40.0), vec2(400.0, 760.0))
}

/// `float_pose_tab` で浮かせたポーズのウィンドウの範囲（余白つき）。
fn pose_window() -> Rect {
    let r = float_rect();
    Rect::from_min_size(r.min, r.size() * 0.8).expand(6.0)
}

fn float_pose_tab(h: &mut H) {
    {
        let dock = &mut h.state_mut().dock;
        let path = dock.find_tab(&Tab::Pose).expect("ポーズのタブ");
        dock.detach_tab(path, float_rect());
    }
    h.run();
}

fn session(h: &H) -> &pose::PoseSession {
    h.view3d().pose.session.as_ref().unwrap()
}

fn drawn(shape: &Shape, out: &mut Vec<String>) {
    match shape {
        Shape::Vec(shapes) => shapes.iter().for_each(|s| drawn(s, out)),
        Shape::Text(text) => out.push(text.galley.job.text.clone()),
        _ => {}
    }
}

fn texts(h: &H) -> Vec<String> {
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        drawn(&shape.shape, &mut out);
    }
    out
}

/// 描いた文字が、描く先のクリップの中に収まっている（切れていない）。
fn clipped_texts(h: &H) -> Vec<String> {
    fn walk(shape: &Shape, clip: Rect, out: &mut Vec<String>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, clip, out)),
            Shape::Text(text) => {
                let bounds = Rect::from_min_size(text.pos, text.galley.size());
                let visible = clip.y_range().contains(bounds.center().y);
                if visible
                    && (bounds.left() < clip.left() - 1.0 || bounds.right() > clip.right() + 1.0)
                {
                    out.push(format!(
                        "{}: {bounds:?} clip={clip:?}",
                        text.galley.job.text
                    ));
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        walk(&shape.shape, shape.clip_rect, &mut out);
    }
    out
}

/// テイクの節の見え方（ボーンの節を閉じ、テイクの節の下にプリセットの節が続く。Raise のフレーム 40 を当てた 3D ビュー）。日英。
#[test]
fn pose_takes_snapshots() {
    for lang in Lang::ALL {
        let mut h = app_in(1280.0, 860.0, lang);
        load(&mut h, &fbx_ascii::arm_takes_scene(), "腕.fbx");
        float_pose_tab(&mut h);
        for section in ["pose.bones", "pose.hide"] {
            h.ui_mut().sections.insert(section, false);
        }
        h.state_mut()
            .apply(Action::Pose(PoseAction::ChooseTake(0, 1)));
        takes::set_frame(h.app_mut(), 40);
        h.state_mut().apply(Action::Pose(PoseAction::ApplyTake));
        takes::wait(h.app_mut());
        click_tab(&mut h, Tab::View3d);
        h.run();
        // 浮かせたポーズのタブと、フレーム 40 を当てた 3D ビュー。英語の絵は撮らない（節の並びは文字の長さで変わらない）
        let shown = pose_window().union(crate::view3d_area(&h));
        if lang == Lang::Ja {
            crate::common::shot::region(&mut h, "pose_tab_takes", shown);
            // テイクを選ぶポップアップ
            h.get_by_label("Raise").click();
            h.run();
            let menu = crate::with_popup(&h, shown);
            crate::common::shot::region(&mut h, "pose_tab_takes_menu", menu);
        }
    }
}

/// テイクの無いモデル（試しの人形・テイクの無い FBX）では、節ごと出さない。
#[test]
fn the_takes_section_is_shown_only_for_a_model_with_takes() {
    for lang in Lang::ALL {
        let title = match lang {
            Lang::Ja => "テイク",
            Lang::En => "Takes",
        };
        let mut h = app_in(1280.0, 860.0, lang);
        h.state_mut().apply(Action::Pose(PoseAction::LoadFigure));
        h.run();
        float_pose_tab(&mut h);
        assert!(!texts(&h).iter().any(|t| t == title), "{lang:?} 人形");
        load(&mut h, &fbx_ascii::arm_scene(), "テイクなし.fbx");
        h.run();
        assert!(session(&h).takes.is_empty());
        assert!(!texts(&h).iter().any(|t| t == title), "{lang:?} テイクなし");
        load(&mut h, &fbx_ascii::arm_takes_scene(), "腕.fbx");
        h.run();
        assert!(texts(&h).iter().any(|t| t == title), "{lang:?} テイクあり");
    }
}

/// 箱からテイクを選び、フレームを打って、「ポーズにする」で当てる（取り消しの 1 段。取り消すと前のポーズ）。
#[test]
fn a_take_is_chosen_from_the_box_and_applied_with_the_button() {
    let mut h = app_in(1280.0, 860.0, Lang::Ja);
    load(&mut h, &fbx_ascii::arm_takes_scene(), "腕.fbx");
    float_pose_tab(&mut h);
    h.ui_mut().sections.insert("pose.bones", false);
    h.run();
    // 箱は最初のテイク
    assert_eq!(session(&h).takes.chosen(), Some((0, 0)));
    h.get_by_label("Wave").click();
    h.run();
    let item = popup_item(&h, "Raise");
    click(&mut h, item.center());
    h.run();
    assert_eq!(session(&h).takes.chosen(), Some((0, 1)));
    assert_eq!(session(&h).takes.frame(), 15, "範囲（15〜60）へ寄せる");
    assert!(h.modified(), "選びを変えたら変更あり");
    takes::set_frame(h.app_mut(), 50);
    h.run();
    let before = session(&h).pose().clone();
    let steps = session(&h).undo_len();
    h.get_by_label("ポーズにする").click();
    h.run();
    takes::wait(h.app_mut());
    h.run();
    let s = session(&h);
    assert_ne!(s.pose(), &before);
    assert_eq!(s.undo_len(), steps + 1);
    let upper = s
        .rig
        .bones()
        .iter()
        .position(|b| b.name == "Upper")
        .unwrap();
    assert!((s.pose().locals[upper].translation.y - 1.2).abs() < 1e-5);
    assert!(
        h.message().contains("「Raise」のフレーム 50"),
        "{}",
        h.message()
    );
    h.get_by_label("取り消し").click();
    h.run();
    assert_eq!(session(&h).pose(), &before);
}

/// 日英で、テイクの節の文字が切れない・「…」に詰められない。英語に日本語が残らない（テイク・ボーンの名前などファイルの中身は除く）。
#[test]
fn the_takes_section_fits_in_both_languages_and_english_has_no_japanese() {
    widgets::record_truncations(true);
    for (width, height) in [(1600.0, 960.0), (1280.0, 800.0)] {
        for lang in Lang::ALL {
            let mut h = app_in(width, height, lang);
            load(&mut h, &fbx_ascii::arm_takes_scene(), "腕.fbx");
            click_tab(&mut h, Tab::Pose);
            h.ui_mut().sections.insert("pose.bones", false);
            h.run();
            let clipped = clipped_texts(&h);
            assert!(
                clipped.is_empty(),
                "{lang:?} {width}x{height}: {clipped:#?}"
            );
            widgets::take_truncations();
            h.step();
            let truncated = widgets::take_truncations();
            assert!(
                truncated.is_empty(),
                "{lang:?} {width}x{height}: 「…」に詰められた文字 {truncated:#?}"
            );
            if lang == Lang::En {
                let left: Vec<String> = texts(&h)
                    .into_iter()
                    .map(|t| t.replace("腕", ""))
                    .filter(|t| has_japanese(t))
                    .collect();
                assert!(left.is_empty(), "{width}x{height}: {left:?}");
                assert!(h.query_by_label("Apply as Pose").is_some());
            }
        }
    }
    widgets::record_truncations(false);
}
