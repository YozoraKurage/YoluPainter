//! 日英の対象パネル・通知・失敗時の保存契約。
mod common;
use egui::{epaint::Shape, vec2, Rect};
use egui_kittest::{kittest::Queryable, Harness, SnapshotResults};
use common::*;
use yolu_app::{
    engine::BlendMode,
    lang::Lang,
    m2::{AdjustmentKind, BrushOp, Edit, EffectKind, UiOp},
    panels::{assets, color, texture_sets},
    pen::PenInput,
    state::{Action, AppState},
    ui::widgets,
    view3d::pose::PoseAction,
    Tab, YoluApp,
};

/// ひらがな・カタカナ・漢字・全角の記号があるか（英語の画面に日本語が残っていないかの確かめ）。
fn has_japanese(text: &str) -> bool {
    text.chars().any(|c| matches!(c, '\u{3000}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}' | '\u{ff00}'..='\u{ffef}'))
}

fn text_shapes(shape: &Shape, clip: Rect, labels: &mut Vec<String>) {
    match shape {
        Shape::Vec(shapes) => for shape in shapes { text_shapes(shape, clip, labels); },
        Shape::Text(text) => {
            let value = text.galley.job.text.clone();
            // アイコンも含め、描く文字がクリップ領域の中に収まることを確認する。
            let bounds = Rect::from_min_size(text.pos, text.galley.size());
            assert!(clip.expand(1.0).contains_rect(bounds), "{value}: {bounds:?}, clip={clip:?}");
            labels.push(value);
        }
        _ => {}
    }
}

#[test]
fn panels_draw_in_both_languages_without_clipped_text() {
    let mut snapshots = SnapshotResults::new();
    for lang in Lang::ALL {
        for panel in 0..3 {
            let mut state = AppState::new(64, 64);
            state.lang = lang;
            state.sets.get_mut(0).unwrap().name = "Sample".into();
            let mut ready = false;
            let mut textures = color::ColorTextures::default();
            let mut h = Harness::builder().with_size(vec2(300.0, 360.0))
                .with_render_options(common::render_options()).wgpu()
                .build_ui_state(move |ui, state| {
                    if !ready {
                        YoluApp::setup(ui.ctx());
                        ready = true;
                        ui.ctx().request_repaint();
                        return;
                    }
                    match panel {
                        0 => color::show(ui, state, &mut textures),
                        1 => texture_sets::show(ui, state),
                        _ => assets::show(ui, state),
                    }
                }, state);
            h.run();
            let mut labels = Vec::new();
            for shape in &h.output().shapes { text_shapes(&shape.shape, shape.clip_rect, &mut labels); }
            let expected = match panel {
                0 => lang.pick("メイン  #", "Foreground  #"),
                1 => "Sample",
                _ => lang.pick("なし", "Empty"),
            };
            assert!(labels.iter().any(|s| s.contains(expected)), "{labels:?}");
            if panel == 0 {
                assert!(h.query_by_label(lang.pick("色相", "Hue")).is_some());
            }
            h.snapshot(format!("i18n_{}_{}", lang.pick("ja", "en"), panel));
            snapshots.extend_harness(&mut h);
        }
    }
}

#[test]
fn project_notices_and_conflicts_follow_the_language_without_changing_data() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/i18n-project-tests").join(std::process::id().to_string());
    std::fs::create_dir_all(&root).unwrap();
    for lang in Lang::ALL {
        let path = root.join(lang.pick("ja.ylp", "en.ylp"));
        let mut state = AppState::new(32, 32);
        state.lang = lang;
        state.apply(Action::SaveProjectAs(path.clone()));
        assert!(state.message.starts_with(lang.pick("保存しました", "Saved")), "{}", state.message);
        state.apply(Action::OpenProject(path.clone()));
        assert!(state.message.starts_with(lang.pick("開きました", "Opened")), "{}", state.message);
        state.apply(Action::SaveProjectAs(path.clone()));
        assert!(state.message.contains(lang.pick("前の版", "Previous version")));
        let before = state.doc.id();
        state.apply(Action::OpenProject(root.join("missing.ylp")));
        assert!(state.message.contains(lang.pick("ファイルまたはフォルダーがありません", "File or folder not found")));
        assert_eq!(state.doc.id(), before);
        std::fs::write(&path, b"external edit").unwrap();
        state.apply(Action::SaveProjectAs(path.clone()));
        assert!(state.message.contains(lang.pick("保存先が外部で変更されています", "Save target or backup changed")), "{}", state.message);
        assert_eq!(std::fs::read(&path).unwrap(), b"external edit");
        state.apply(Action::NewProject);
        assert_eq!(state.message, lang.pick("新しいプロジェクトを作りました。", "New project created."));
    }
    std::fs::remove_dir_all(root).unwrap();
}

/// 開く・保存するの失敗は、壊れたファイル・予算超過・まだ書けない中身を言い分ける（どれも同じ文にしない）。日本語は診断を保つ。
#[test]
fn project_failures_are_told_apart_by_kind_in_both_languages() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/i18n-failure-tests").join(std::process::id().to_string());
    std::fs::create_dir_all(&root).unwrap();
    // 予算を超える大きさのファイル（中身は読まずに断る。疎なファイルなので場所を取らない）
    let big = root.join("big.ylp");
    std::fs::File::create(&big).unwrap().set_len((yolu_io::MAX_TOTAL_BYTES + 3 * 1024 * 1024) as u64).unwrap();
    let broken = root.join("broken.ylp");
    std::fs::write(&broken, b"not an archive").unwrap();
    let mut seen: Vec<Vec<String>> = Vec::new();
    for lang in Lang::ALL {
        let mut state = AppState::new_in(32, 32, lang);
        let before = state.doc.id();
        let mut messages = Vec::new();
        for path in [&broken, &big] {
            state.apply(Action::OpenProject(path.clone()));
            assert!(state.message.starts_with(lang.pick("開けません", "Cannot open")), "{}", state.message);
            assert_eq!(state.doc.id(), before);
            messages.push(state.message.clone());
        }
        // 手動の ID の色はまだ .ylp に書けない。保存を断り、ファイルは作らない
        let colors = yolu_core::mesh_maps::IdColorAssignments::new("0".repeat(64), std::collections::BTreeMap::from([(0usize, 0xff0000u32)])).unwrap();
        state.doc.set_id_colors(colors).unwrap();
        let target = root.join(lang.pick("ja.ylp", "en.ylp"));
        state.apply(Action::SaveProjectAs(target.clone()));
        assert!(state.message.starts_with(lang.pick("保存できません", "Cannot save")), "{}", state.message);
        assert!(!target.exists());
        messages.push(state.message.clone());
        // 3 つとも別の文。日本語は診断（どの予算か・どの中身か）を残し、英語は日本語を出さない
        for (i, a) in messages.iter().enumerate() {
            assert_eq!(has_japanese(a), lang == Lang::Ja, "{a}");
            for b in &messages[i + 1..] {
                assert_ne!(a, b);
            }
        }
        match lang {
            Lang::Ja => {
                assert!(messages[0].contains("mimetype"), "{}", messages[0]);
                assert!(messages[1].contains("予算"), "{}", messages[1]);
                assert!(messages[2].contains("ID の色"), "{}", messages[2]);
            }
            Lang::En => {
                assert!(messages[0].contains("Invalid or unsupported"), "{}", messages[0]);
                assert!(messages[1].contains("limit exceeded"), "{}", messages[1]);
                assert!(messages[2].contains("Manual ID colors"), "{}", messages[2]);
            }
        }
        seen.push(messages);
    }
    assert_ne!(seen[0], seen[1]);
    std::fs::remove_dir_all(root).unwrap();
}

/// 古い形式を開いたときの io の知らせは、件数ではなく内容を日英それぞれで読める。
#[test]
fn opening_an_old_format_reports_what_io_noted_in_both_languages() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../yolu-io/tests/fixtures");
    for lang in Lang::ALL {
        let mut state = AppState::new_in(32, 32, lang);
        state.apply(Action::OpenProject(fixtures.join("format3.ylp")));
        let message = state.message.clone();
        assert!(message.starts_with(lang.pick("開きました", "Opened")), "{message}");
        match lang {
            Lang::Ja => assert!(message.contains("マテリアル参照を形式7へメモリ上で移行しました"), "{message}"),
            Lang::En => {
                assert!(message.contains("Migrated format 3 material references to format 7 in memory"), "{message}");
                assert!(!has_japanese(&message), "{message}");
                assert!(!message.contains("notices"), "{message}");
            }
        }
    }
}

#[test]
fn link_status_and_stop_notice_follow_the_language() {
    use yolu_app::livelink::{LinkStatus, LinkView, LiveLink};
    for lang in Lang::ALL {
        let mut state = AppState::new(32, 32);
        state.lang = lang;
        let mut link = LiveLink::new();
        link.stop(&mut state);
        assert_eq!(state.message, lang.pick("Live Link の待ち受けをやめました。", "Live Link stopped."));
        for status in [LinkStatus::Off, LinkStatus::Listening, LinkStatus::Connected { agent: "Unity".into(), version: 1, session: 1 }, LinkStatus::Failed("test".into())] {
            let view = LinkView { status, ..Default::default() };
            let text = view.summary_in(lang);
            assert_eq!(has_japanese(&text), lang == Lang::Ja, "{text}");
        }
    }
}

// ───────── 英語の画面に日本語が残っていない ─────────

/// 英語で作った窓の全体（最初のレイヤー・テクスチャセット・プロジェクトの名前も英語）。
fn english_app() -> Harness<'static, YoluApp> {
    english_app_sized(1280.0, 800.0, Lang::En)
}

fn english_app_sized(width: f32, height: f32, lang: Lang) -> Harness<'static, YoluApp> {
    let mut h = Harness::builder()
        .with_size(vec2(width, height))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .with_render_options(render_options())
        .wgpu()
        .build_eframe(move |cc| {
            YoluApp::for_context(&cc.egui_ctx, AppState::new_in(64, 64, lang), PenInput::detached())
                .with_render_state(cc.wgpu_render_state.as_ref())
        });
    h.run();
    h
}

fn drawn_texts(shape: &Shape, out: &mut Vec<String>) {
    match shape {
        Shape::Vec(shapes) => shapes.iter().for_each(|s| drawn_texts(s, out)),
        Shape::Text(text) => out.push(text.galley.job.text.clone()),
        _ => {}
    }
}

/// いま描いた文字に日本語があれば、その文字（`data` は試しのモデル・試しの人形の名前など、言語に関わらない中身）。
fn japanese_left(h: &Harness<'_, YoluApp>, data: &[String]) -> Vec<String> {
    let mut texts = Vec::new();
    for shape in &h.output().shapes {
        drawn_texts(&shape.shape, &mut texts);
    }
    texts
        .into_iter()
        .map(|mut t| {
            for d in data {
                t = t.replace(d.as_str(), "");
            }
            t
        })
        .filter(|t| has_japanese(t))
        .collect()
}

fn all_texts(h: &Harness<'_, YoluApp>) -> Vec<String> {
    let mut texts = Vec::new();
    for shape in &h.output().shapes {
        drawn_texts(&shape.shape, &mut texts);
    }
    texts
}

/// 描いた文字が、描く先のクリップの中に収まっている（切れていない）。
fn clipped_texts(h: &Harness<'_, YoluApp>) -> Vec<String> {
    fn walk(shape: &Shape, clip: Rect, out: &mut Vec<String>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, clip, out)),
            Shape::Text(text) => {
                let bounds = Rect::from_min_size(text.pos, text.galley.size());
                // 見えている行（スクロールで外に出ている行は除く）が、横にはみ出して切れていない
                let visible = clip.y_range().contains(bounds.center().y);
                if visible && (bounds.left() < clip.left() - 1.0 || bounds.right() > clip.right() + 1.0) {
                    out.push(format!("{}: {bounds:?} clip={clip:?}", text.galley.job.text));
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

fn assert_english(h: &Harness<'_, YoluApp>, what: &str, data: &[String]) {
    let left = japanese_left(h, data);
    assert!(left.is_empty(), "{what}: {left:?}");
    let clipped = clipped_texts(h);
    assert!(clipped.is_empty(), "{what}: {clipped:#?}");
}

#[test]
fn english_docks_menus_and_layer_kinds_have_no_japanese() {
    let mut h = english_app();
    assert_english(&h, "default", &[]);
    // 見張りが働いていること: 同じ画面を日本語にすると日本語が見つかり、英語の文字が描かれている
    assert!(all_texts(&h).iter().any(|t| t.contains("Layers")));
    h.state_mut().state.lang = Lang::Ja;
    h.run();
    assert!(!japanese_left(&h, &[]).is_empty());
    h.state_mut().state.lang = Lang::En;
    h.run();
    for tab in [Tab::Assets, Tab::Color, Tab::Channels, Tab::TextureSets, Tab::Layers, Tab::Properties, Tab::View3d, Tab::Canvas] {
        click_tab(&mut h, tab);
        assert_english(&h, tab.title_in(Lang::En), &[]);
    }
    // 層の種類・マスク・合成モード・ブラシの種類ごとのプロパティ
    let apply = |h: &mut Harness<'_, YoluApp>, action: Action| {
        h.state_mut().state.apply(action);
        h.run();
    };
    click_tab(&mut h, Tab::Properties);
    for edit in [Edit::NewGroup, Edit::NewFill, Edit::NewAdjustment(AdjustmentKind::ALL[0]), Edit::NewAdjustment(AdjustmentKind::ALL[1]), Edit::NewAdjustment(AdjustmentKind::ALL[2])] {
        apply(&mut h, Action::M2(edit));
        assert_english(&h, "layer kind", &[]);
    }
    let id = h.state().state.selected_layer.unwrap();
    apply(&mut h, Action::M2(Edit::AddMask(id)));
    apply(&mut h, Action::M2Ui(UiOp::EditMask(true)));
    assert_english(&h, "mask", &[]);
    apply(&mut h, Action::M2Ui(UiOp::EditMask(false)));
    apply(&mut h, Action::SetBlend(id, BlendMode::Multiply));
    for kind in EffectKind::ALL {
        apply(&mut h, Action::M2Ui(UiOp::Brush(BrushOp::Effect(kind))));
        assert_english(&h, kind.name(Lang::En), &[]);
    }
    apply(&mut h, Action::M2Ui(UiOp::Brush(BrushOp::Effect(EffectKind::Paint))));
    apply(&mut h, Action::M2Ui(UiOp::Brush(BrushOp::DualEnabled(true))));
    assert_english(&h, "dual brush", &[]);
    // メニュー（開いているあいだは項目も描く）
    for (i, title) in ["File", "Edit", "Layer", "View", "Help"].into_iter().enumerate() {
        let at = menu_title(&h, title).center();
        if i == 0 {
            click(&mut h, at);
        } else {
            // 開いているあいだはホバーで切り替わる
            move_to(&h, at);
            h.run();
        }
        assert!(h.state().state.popup.is_some(), "{title}");
        // 言語の選択肢は、その言語の自分の名前
        assert_english(&h, title, &[Lang::Ja.name().to_owned()]);
    }
}

/// 日本語（既定）で起動してから English に替えても、既定の名前（プロジェクト・最初のテクスチャセット・最初のレイヤー）は
/// 英語になる。利用者が付けた名前・編集した文書は変えない。
#[test]
fn switching_language_at_runtime_renames_the_defaults_but_not_the_users_names() {
    let mut h = english_app_sized(1280.0, 800.0, Lang::Ja);
    let names = |h: &Harness<'_, YoluApp>| {
        let s = &h.state().state;
        (s.project_name.clone(), s.sets.iter().next().unwrap().name.clone(), s.doc.layers()[0].name().to_owned())
    };
    assert_eq!(names(&h), ("名称未設定".into(), "テクスチャセット 1".into(), "レイヤー 1".into()));
    h.state_mut().state.apply(Action::M2Ui(UiOp::Language(Lang::En)));
    h.run();
    assert_eq!(names(&h), ("Untitled".into(), "Texture Set 1".into(), "Layer 1".into()));
    // 付け直しは編集ではない（変更の印も Undo の履歴も付かない）
    assert!(!h.state().state.modified && !h.state().state.doc.can_undo());
    assert!(all_texts(&h).iter().any(|t| t.contains("Untitled")));
    for tab in [Tab::TextureSets, Tab::Layers] {
        click_tab(&mut h, tab);
        assert_english(&h, tab.title_in(Lang::En), &[]);
    }
    // 戻すと日本語の既定の名前に戻る
    h.state_mut().state.apply(Action::M2Ui(UiOp::Language(Lang::Ja)));
    h.run();
    assert_eq!(names(&h), ("名称未設定".into(), "テクスチャセット 1".into(), "レイヤー 1".into()));

    // 利用者が付けた名前は、言語を替えても変えない
    let uid = h.state().state.sets.iter().next().unwrap().uid;
    h.state_mut().state.rename_set(uid, "Mine").unwrap();
    h.state_mut().state.project_name = "Work".into();
    h.state_mut().state.apply(Action::M2Ui(UiOp::Language(Lang::En)));
    h.run();
    assert_eq!(names(&h), ("Work".into(), "Mine".into(), "Layer 1".into()));
    // 編集した文書の層は、既定の名前のままでも変えない（Undo の履歴とずれる）
    h.state_mut().state.apply(Action::M2(Edit::NewGroup));
    h.state_mut().state.apply(Action::M2Ui(UiOp::Language(Lang::Ja)));
    h.run();
    assert_eq!(h.state().state.doc.layers()[0].name(), "Layer 1");
    assert_eq!(h.state().state.project_name, "Work");
}

// ───────── 言語の選択の保存と復元（アプリの配線） ─────────

/// 設定のファイルを使うアプリ（`settings` は設定のファイルの場所）。
fn app_with_settings(settings: &std::path::Path) -> Harness<'static, YoluApp> {
    let settings = settings.to_path_buf();
    let mut h = Harness::builder()
        .with_size(vec2(1280.0, 800.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .with_render_options(render_options())
        .wgpu()
        .build_eframe(move |cc| {
            YoluApp::for_context_with_settings(&cc.egui_ctx, Some(settings), PenInput::detached())
                .with_render_state(cc.wgpu_render_state.as_ref())
        });
    h.run();
    h
}

fn settings_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/i18n-settings-tests").join(std::process::id().to_string()).join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn language_choice_is_written_when_changed_and_restored_at_startup() {
    let dir = settings_dir("restore");
    let path = dir.join("YoluPainter").join("settings.conf");
    // 設定が無い初回は日本語。選ぶと、そのフレームのうちに書く
    let mut h = app_with_settings(&path);
    assert_eq!(h.state().state.lang, Lang::Ja);
    assert!(!path.exists());
    assert_eq!(h.state().state.message, "");
    h.state_mut().state.apply(Action::M2Ui(UiOp::Language(Lang::En)));
    h.run();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=en\n");
    drop(h);
    // 次の起動は、書いた言語で始まる（最初の名前も英語）
    let h = app_with_settings(&path);
    assert_eq!(h.state().state.lang, Lang::En);
    assert_eq!(h.state().state.project_name, "Untitled");
    assert_eq!(h.state().state.message, "");
    assert_eq!(h.state().state.sets.iter().next().unwrap().name, "Texture Set 1");
    drop(h);
    // 選び直すと書き直す。同じ選択では書き直さない（外から書き換えた中身をそのまま）
    let mut h = app_with_settings(&path);
    h.state_mut().state.apply(Action::M2Ui(UiOp::Language(Lang::Ja)));
    h.run();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=ja\n");
    std::fs::write(&path, "language=en\n").unwrap();
    h.run();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=en\n", "選択が変わらなければ書かない");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unwritable_settings_report_once_and_do_not_break_the_app() {
    let dir = settings_dir("unwritable");
    let path = dir.join("settings.conf");
    // 書く途中のファイルが残っていて書けない（settings.rs の試験と同じ手）
    std::fs::write(path.with_extension(format!("{}.pending", std::process::id())), "busy").unwrap();
    let mut h = app_with_settings(&path);
    h.state_mut().state.apply(Action::M2Ui(UiOp::Language(Lang::En)));
    h.run();
    assert_eq!(h.state().state.lang, Lang::En, "書けなくても言語は替わる");
    assert_eq!(h.state().state.message, "Cannot save language setting.");
    assert!(!path.exists());
    // 同じ選択で毎フレーム書き直さない（知らせを消したら、出し直さない）
    h.state_mut().state.message.clear();
    h.run();
    h.run();
    assert_eq!(h.state().state.message, "");
    // 窓の操作はそのまま動く
    h.state_mut().state.apply(Action::NewProject);
    assert_eq!(h.state().state.message, "New project created.");
    // 書けるようになれば、次に選んだときに書く
    std::fs::remove_file(path.with_extension(format!("{}.pending", std::process::id()))).unwrap();
    h.state_mut().state.apply(Action::M2Ui(UiOp::Language(Lang::Ja)));
    h.run();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=ja\n");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn broken_settings_fall_back_to_japanese_and_are_repaired_by_choosing() {
    let dir = settings_dir("broken");
    let path = dir.join("settings.conf");
    std::fs::write(&path, "language=unknown").unwrap();
    let mut h = app_with_settings(&path);
    assert_eq!(h.state().state.lang, Lang::Ja);
    assert_eq!(h.state().state.message, "言語の設定を読めません。");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=unknown", "選ぶまで壊れたファイルには触らない");
    h.state_mut().state.apply(Action::M2Ui(UiOp::Language(Lang::En)));
    h.run();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=en\n");
    drop(h);
    let h = app_with_settings(&path);
    assert_eq!(h.state().state.lang, Lang::En);
    assert_eq!(h.state().state.message, "");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn english_3d_view_and_pose_panel_have_no_japanese() {
    let mut h = english_app();
    click_tab(&mut h, Tab::View3d);
    assert_english(&h, "no model", &[]);
    assert!(all_texts(&h).iter().any(|t| t == "No model"));
    h.state_mut().state.apply(Action::LoadDemoModel);
    h.run();
    let demo = h.state().state.view3d.model.as_ref().map(|m| vec![m.name.clone()]).unwrap_or_default();
    assert_english(&h, "test cube", &demo);
    assert!(h.state().state.message.is_ascii(), "{}", h.state().state.message);
    h.state_mut().state.apply(Action::Pose(PoseAction::LoadFigure));
    h.run();
    h.run();
    // 試しの人形の骨・メッシュ・マテリアルの名前は中身（言語に関わらない）
    let mut data = Vec::new();
    if let Some(s) = h.state().state.view3d.pose.session.as_ref() {
        data.push(s.rig.name().to_owned());
        data.extend(s.rig.bones().iter().map(|b| b.name.clone()));
        data.extend(s.rig.materials().iter().cloned());
        for m in s.rig.meshes() {
            data.push(m.mesh.name.clone());
            data.extend(m.blend_shapes.iter().map(|b| b.name.clone()));
        }
    }
    data.sort_by_key(|d| std::cmp::Reverse(d.len()));
    assert_english(&h, "pose panel", &data);
    assert!(all_texts(&h).iter().any(|t| t == "Bones"), "{:?}", all_texts(&h));
    assert!(!h.state().state.message.is_empty());
    assert_english_message(&h, &data);
}

fn assert_english_message(h: &Harness<'_, YoluApp>, data: &[String]) {
    let mut m = h.state().state.message.clone();
    for d in data {
        m = m.replace(d.as_str(), "");
    }
    assert!(!has_japanese(&m), "{m}");
}

#[test]
fn english_texture_set_states_have_no_japanese() {
    use yolu_protocol::{channel, ChannelRoute, MaterialInfo, MaterialKey, MeshData, Model, Submesh, TextureProperty};
    let material = |name: Option<&str>, color_route: bool| MaterialInfo {
        key: match name {
            Some(n) => MaterialKey::Material { name: n.into(), asset: None },
            None => MaterialKey::Unassigned,
        },
        shader: "Standard".into(),
        textures: vec![TextureProperty { name: "_MainTex".into(), width: 64, height: 64 }],
        routes: if color_route {
            vec![ChannelRoute { channel: channel::COLOR, property: "_MainTex".into() }]
        } else {
            vec![]
        },
    };
    let materials = vec![material(Some("Skin"), true), material(Some("Hair"), false), material(None, true)];
    let n = materials.len() as u32;
    let model = Model {
        generation: 1,
        name: "Sample".into(),
        materials,
        meshes: vec![MeshData {
            key: "0".into(),
            name: "Body".into(),
            skinned: false,
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            submeshes: (0..n).map(|m| Submesh { material: m, indices: vec![0, 1, 2] }).collect(),
        }],
    };
    let mut h = english_app();
    h.state_mut().load_live_link_model(&model).unwrap();
    h.run();
    click_tab(&mut h, Tab::TextureSets);
    let uids: Vec<u32> = h.state().state.sets.iter().map(|s| s.uid).collect();
    assert_eq!(uids.len(), 3);
    h.state_mut().state.apply(Action::ToggleSetVisible(uids[0]));
    h.run();
    for (i, uid) in uids.iter().enumerate() {
        h.state_mut().state.apply(Action::SelectSet(*uid));
        h.run();
        assert_english(&h, &format!("set {i}"), &[]);
    }
    let texts = all_texts(&h);
    assert!(texts.iter().any(|t| t.contains("Skin")), "{texts:?}");
    // 読むだけのセットは理由を英語で（開くときに言語で作る理由）
    h.state_mut().state.sets.get_mut(0).unwrap().read_only = Some("Unsupported document features (1)".into());
    h.state_mut().state.apply(Action::SelectSet(uids[0]));
    h.run();
    assert_english(&h, "read-only set", &[]);
}

/// 窓の中の代表の状態を順に出して、そのつど `visit` に見せる（初めの画面・全部のタブ・試しのモデル・ポーズのパネル・メニュー）。
fn walk_states(lang: Lang, width: f32, height: f32, mut visit: impl FnMut(&mut Harness<'static, YoluApp>, &str)) {
    let mut h = english_app_sized(width, height, lang);
    visit(&mut h, "default");
    for tab in [Tab::Assets, Tab::Color, Tab::Channels, Tab::TextureSets, Tab::Layers, Tab::Properties, Tab::View3d, Tab::Canvas] {
        click_tab(&mut h, tab);
        visit(&mut h, tab.title_in(lang));
    }
    h.state_mut().state.apply(Action::LoadDemoModel);
    h.run();
    visit(&mut h, "test cube");
    h.state_mut().state.apply(Action::Pose(PoseAction::LoadFigure));
    h.run();
    h.run();
    visit(&mut h, "pose panel");
    for (i, title) in lang.pick(["ファイル", "編集", "レイヤー", "表示", "ヘルプ"], ["File", "Edit", "Layer", "View", "Help"]).into_iter().enumerate() {
        let at = menu_title(&h, title).center();
        if i == 0 {
            click(&mut h, at);
        } else {
            move_to(&h, at);
            h.run();
        }
        visit(&mut h, title);
    }
}

/// 固定の文字（部品が幅に合わせて「…」に詰める `w::fit` の文字）が詰められた記録を、落ち着いた 1 フレームだけから集める。
/// `w::fit` は描く文字を必ず幅に収めるので、描いた文字だけを見ても詰められたことは分からない。詰めた記録（元の文字）を見る。
struct Truncations;
impl Truncations {
    fn start() -> Self {
        widgets::record_truncations(true);
        Truncations
    }
    /// 今の状態の、詰められた文字（起動直後の寸法が決まる前のフレームは含めない）。
    fn settled(&self, h: &mut Harness<'_, YoluApp>) -> Vec<String> {
        widgets::take_truncations();
        h.step();
        widgets::take_truncations()
    }
}
impl Drop for Truncations {
    fn drop(&mut self) {
        widgets::record_truncations(false);
    }
}

#[test]
fn fixed_text_is_not_truncated_at_ordinary_window_sizes_in_both_languages() {
    let truncations = Truncations::start();
    // 既定の窓（main.rs の with_inner_size）と、ふつうのノート PC の大きさ
    for (width, height) in [(1600.0, 960.0), (1280.0, 800.0)] {
        for lang in Lang::ALL {
            walk_states(lang, width, height, |h, what| {
                let clipped = clipped_texts(h);
                assert!(clipped.is_empty(), "{lang:?} {width}x{height} {what}: {clipped:#?}");
                let truncated = truncations.settled(h);
                assert!(truncated.is_empty(), "{lang:?} {width}x{height} {what}: 「…」に詰められた文字 {truncated:#?}");
            });
        }
    }
}

/// 窓の最小の大きさ（main.rs の with_min_inner_size）。ドックが狭く、日英どちらでも詰まる箱の値がある（チャンネルの名前・
/// プリセットなど。言語に依らない配置の積み残し）。`KNOWN` が詰まる文字の全部で、増えれば落ち、直せば一覧から消す。
#[test]
fn fixed_text_truncation_at_the_minimum_window_size_is_exactly_the_known_set() {
    // チャンネルの名前（チャンネルのパネルの行）、プリセット・効果・合成モードの箱の値、テクスチャセットの名前と説明
    const KNOWN_JA: [&str; 11] = [
        "エミッション", "カスタム", "カラー", "テクスチャセット 1", "ノーマル", "ハイト", "ペイント", "まだマテリアルに付いていない（スロット 0）", "メタリック", "ラフネス", "手ぶれ補正と入り抜き",
    ];
    const KNOWN_EN: [&str; 8] = ["Color", "Custom", "Emission", "Height", "Metallic", "Normal", "Paint", "Roughness"];
    let truncations = Truncations::start();
    for lang in Lang::ALL {
        let mut seen = std::collections::BTreeSet::new();
        walk_states(lang, 960.0, 640.0, |h, what| {
            // 描く文字は横に切れない（詰めたあとの文字は必ず収まる）
            let clipped = clipped_texts(h);
            assert!(clipped.is_empty(), "{lang:?} {what}: {clipped:#?}");
            seen.extend(truncations.settled(h));
        });
        let known: &[&str] = lang.pick(&KNOWN_JA, &KNOWN_EN);
        let seen: Vec<&str> = seen.iter().map(String::as_str).collect();
        let mut known = known.to_vec();
        known.sort();
        assert_eq!(seen, known, "{lang:?}: 詰められた文字が一覧と違う。新しく詰まったなら配置を直す。直したなら一覧から消す");
    }
}
