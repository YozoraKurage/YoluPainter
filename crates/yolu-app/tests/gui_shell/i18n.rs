//! 日英の対象パネル・通知・失敗時の保存契約。
use crate::common;
use common::*;
use egui::{epaint::Shape, vec2, Rect};
use egui_kittest::{kittest::Queryable, Harness, SnapshotResults};
use yolu_app::{
    brushes::{BrushAction, Category, Group},
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

fn text_shapes(shape: &Shape, clip: Rect, labels: &mut Vec<String>) {
    match shape {
        Shape::Vec(shapes) => {
            for shape in shapes {
                text_shapes(shape, clip, labels);
            }
        }
        Shape::Text(text) => {
            let value = text.galley.job.text.clone();
            // アイコンも含め、描く文字がクリップ領域の中に収まることを確認する。
            let bounds = Rect::from_min_size(text.pos, text.galley.size());
            assert!(
                clip.expand(1.0).contains_rect(bounds),
                "{value}: {bounds:?}, clip={clip:?}"
            );
            labels.push(value);
        }
        _ => {}
    }
}

#[test]
fn panels_draw_in_both_languages_without_clipped_text() {
    gpu_thread::run(panels_draw_in_both_languages_without_clipped_text_gpu);
}

fn panels_draw_in_both_languages_without_clipped_text_gpu() {
    let mut snapshots = SnapshotResults::new();
    for lang in Lang::ALL {
        for panel in 0..3 {
            let mut state = AppState::new(64, 64);
            state.lang = lang;
            if panel == 1 {
                // モデルを入れて最初のセットを隠す（行の右に状態のアイコンが付き、その理由がツールチップに言語ごとに出る）
                state.receive_link_model(&three_material_model()).1.unwrap();
                let uid = state.sets.get(0).unwrap().uid;
                state.apply(Action::ToggleSetVisible(uid));
            }
            state.sets.get_mut(0).unwrap().name = "Sample".into();
            // 同梱の素材は行がパネルの下からはみ出して（スクロールで見る）、途中で切れた文字になるので、別の試験で背の高いウィンドウに出す
            state.shelf.show_builtin = false;
            let look = texture_sets::set_state(&state, 0);
            let mut ready = false;
            let mut textures = color::ColorTextures::default();
            let mut h = common::gpu_thread::builder()
                .with_size(vec2(300.0, 360.0))
                .renderer(common::shared_gpu::renderer())
                .build_ui_state(
                    move |ui, state| {
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
                    },
                    state,
                );
            h.run();
            // 棚の素材の絵は別のスレッドで作る。できるまで待って描く
            h.state_mut().shelf.wait_inspections();
            h.run();
            let mut labels = Vec::new();
            for shape in &h.output().shapes {
                text_shapes(&shape.shape, shape.clip_rect, &mut labels);
            }
            let expected = match panel {
                // メインとサブの色の文字は置かない（色の四角のツールチップ）。16 進の欄の文字
                0 => "#000000",
                1 => "Sample",
                // 空の棚は、空の状態の文字を置かない（上の操作の名前だけ）
                _ => lang.pick("レイヤーを保存", "Save Layer"),
            };
            assert!(labels.iter().any(|s| s.contains(expected)), "{labels:?}");
            if panel == 0 {
                // 既定は色相の円と中の四角
                assert!(h
                    .query_by_label(lang.pick("色相の円", "Hue wheel"))
                    .is_some());
            }
            h.snapshot(format!("i18n_{}_{}", lang.pick("ja", "en"), panel));
            snapshots.extend_harness(&mut h);
            if panel == 1 {
                // セットの名前は言語に依らないので、言語の確認は状態のアイコン: 隠したセットの理由が、その言語の文で、行のアイコンの上に出る
                let tooltip = lang.pick("3D ビューに見せていない", "Hidden in the 3D View");
                let look = look.expect("隠したセットには状態のアイコンが付く");
                assert_eq!(
                    (look.icon, look.tooltip.as_str()),
                    ("visibility_off", tooltip),
                    "{lang:?}"
                );
                if lang == Lang::En {
                    assert!(!has_japanese(tooltip));
                }
                // ポインタをアイコン（最初の行の右寄り）の上に置いて、ツールチップが出るまで待つ
                h.event(egui::Event::PointerMoved(egui::pos2(
                    250.0,
                    8.0 + texture_sets::ROW_HEIGHT / 2.0,
                )));
                for _ in 0..30 {
                    h.step();
                }
                assert!(
                    h.query_by_label(tooltip).is_some(),
                    "{lang:?}: 状態のアイコンの上に「{tooltip}」が出ない"
                );
            }
        }
    }
}

#[test]
fn project_notices_and_conflicts_follow_the_language_without_changing_data() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/i18n-project-tests")
        .join(std::process::id().to_string());
    std::fs::create_dir_all(&root).unwrap();
    for lang in Lang::ALL {
        let path = root.join(lang.pick("ja.ylp", "en.ylp"));
        let mut state = AppState::new(32, 32);
        state.lang = lang;
        state.apply(Action::SaveProjectAs(path.clone()));
        assert!(
            state
                .message
                .starts_with(lang.pick("保存しました", "Saved")),
            "{}",
            state.message
        );
        state.apply(Action::OpenProject(path.clone()));
        assert!(
            state.message.starts_with(lang.pick("開きました", "Opened")),
            "{}",
            state.message
        );
        state.apply(Action::SaveProjectAs(path.clone()));
        assert!(state
            .message
            .contains(lang.pick("前の版", "Previous version")));
        let before = state.doc.id();
        state.apply(Action::OpenProject(root.join("missing.ylp")));
        assert!(state.message.contains(lang.pick(
            "ファイルまたはフォルダーがありません",
            "File or folder not found"
        )));
        assert_eq!(state.doc.id(), before);
        std::fs::write(&path, b"external edit").unwrap();
        state.apply(Action::SaveProjectAs(path.clone()));
        assert!(
            state.message.contains(lang.pick(
                "保存先が外部で変更されています",
                "Save target or backup changed"
            )),
            "{}",
            state.message
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"external edit");
        state.apply(Action::NewProject);
        assert_eq!(
            state.message,
            lang.pick("新しいプロジェクトを作りました。", "New project created.")
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

/// 正本の展開後の大きさを 512 MiB を超えると宣言した .ylp（中央ディレクトリとローカルヘッダーの欄だけを書き換える）。
fn declared_too_large() -> Vec<u8> {
    let le16 = |b: &[u8], at: usize| u16::from_le_bytes([b[at], b[at + 1]]) as usize;
    let le32 = |b: &[u8], at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as usize;
    let files = std::collections::BTreeMap::from([(
        "document.utpaint".to_owned(),
        "x".repeat(4096).into_bytes(),
    )]);
    let mut b = yolu_io::Archive::from_entries(files)
        .unwrap()
        .to_bytes()
        .unwrap();
    let end = b.len() - 22;
    let mut at = le32(&b, end + 16);
    while &b[at + 46..at + 46 + le16(&b, at + 28)] != b"document.utpaint" {
        at += 46 + le16(&b, at + 28) + le16(&b, at + 30) + le16(&b, at + 32);
    }
    let local = le32(&b, at + 42);
    let size = (600u32 << 20).to_le_bytes();
    b[at + 24..at + 28].copy_from_slice(&size);
    b[local + 22..local + 26].copy_from_slice(&size);
    b
}

/// 開く・保存するの失敗は、壊れたファイル・予算超過・まだ書けない中身を言い分ける（どれも同じ文にしない）。日本語は診断を保つ。
#[test]
fn project_failures_are_told_apart_by_kind_in_both_languages() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/i18n-failure-tests")
        .join(std::process::id().to_string());
    std::fs::create_dir_all(&root).unwrap();
    // 1 エントリの上限（512 MiB）を超える大きさを宣言したファイル（展開する前に、宣言で断る。中身は小さい）
    let big = root.join("big.ylp");
    std::fs::write(&big, declared_too_large()).unwrap();
    let broken = root.join("broken.ylp");
    std::fs::write(&broken, b"not an archive").unwrap();
    let mut seen: Vec<Vec<String>> = Vec::new();
    for lang in Lang::ALL {
        let mut state = AppState::new_in(32, 32, lang);
        let before = state.doc.id();
        let mut messages = Vec::new();
        for path in [&broken, &big] {
            state.apply(Action::OpenProject(path.clone()));
            assert!(
                state
                    .message
                    .contains(lang.pick("を開けません", "Cannot open")),
                "{}",
                state.message
            );
            assert_eq!(state.doc.id(), before);
            messages.push(state.message.clone());
        }
        // 塗りつぶしのグラデーションのランプの混色はまだ .ylp に書けない（画面からは作れず、core の口だけで作れる）。保存を断り、ファイルは作らない
        let fill = state
            .doc
            .add_fill_layer(
                "塗り",
                &[(
                    yolu_core::Channel::Color,
                    yolu_core::Rgba8::new(1, 2, 3, 255),
                )],
                None,
            )
            .unwrap();
        let mut gradient =
            yolu_core::generator::Settings::new(yolu_core::generator::Kind::ShapeGradient);
        gradient.blend = yolu_core::generator::Blend::Replace;
        gradient.ramp = Some(
            yolu_core::generator::Ramp::new(
                vec![
                    yolu_core::generator::ColorStop {
                        position: 0.0,
                        color: yolu_core::Rgba8::new(0, 0, 0, 255),
                        midpoint: 0.5,
                    },
                    yolu_core::generator::ColorStop {
                        position: 1.0,
                        color: yolu_core::Rgba8::new(255, 255, 255, 255),
                        midpoint: 0.5,
                    },
                ],
                vec![
                    yolu_core::generator::OpacityStop {
                        position: 0.0,
                        opacity: 1.0,
                        midpoint: 0.5,
                    },
                    yolu_core::generator::OpacityStop {
                        position: 1.0,
                        opacity: 1.0,
                        midpoint: 0.5,
                    },
                ],
                None,
            )
            .unwrap()
            .with_mixing(
                yolu_core::generator::MixMode::Perceptual,
                yolu_core::generator::LuminanceCorrection::Low,
            ),
        );
        state
            .doc
            .set_fill_gradient(fill, yolu_core::Channel::Color, Some(gradient), false)
            .unwrap();
        let target = root.join(lang.pick("ja.ylp", "en.ylp"));
        state.apply(Action::SaveProjectAs(target.clone()));
        assert!(
            state
                .message
                .contains(lang.pick("保存できません", "Cannot save")),
            "{}",
            state.message
        );
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
                assert!(messages[2].contains("混色"), "{}", messages[2]);
            }
            Lang::En => {
                assert!(
                    messages[0].contains("Invalid or unsupported"),
                    "{}",
                    messages[0]
                );
                assert!(messages[1].contains("limit exceeded"), "{}", messages[1]);
                assert!(messages[2].contains("Color mixing"), "{}", messages[2]);
            }
        }
        seen.push(messages);
    }
    assert_ne!(seen[0], seen[1]);
    std::fs::remove_dir_all(root).unwrap();
}

/// 設定の予算（「レイヤーのメモリ」から決まる上限）で断った理由は、どの予算かを日英それぞれで言う。開発用の数（MiB）・内部の識別子
/// （セットの ID）・上限の導き方は出さない。.ylp（大きな形 `YLP-4`）と復旧の世代の両方。
#[test]
fn a_budget_refusal_names_the_budget_in_both_languages() {
    use yolu_io::{
        GenerationStore, Limits, NativeDocument, Package, Project, SetSpec, Thresholds, WriterInfo,
    };
    let (doc, _) = yolu_app::state::blank_document(64, 64);
    let id = yolu_app::sets::guid_string(doc.id());
    let native = NativeDocument::from_core(&doc).unwrap();
    let small = Thresholds {
        classic_total_bytes: 64,
        ..Thresholds::REAL
    };
    let project = small.scoped(|| {
        Project::create(
            WriterInfo {
                app: "試験".into(),
                version: "0".into(),
                unity: "standalone".into(),
            },
            &[SetSpec {
                id: id.clone(),
                name: "セット".into(),
                material: yolu_io::MaterialRef::PendingSlot(0),
                document: Some(native.clone().into()),
                composites: vec![],
            }],
            &id,
        )
        .unwrap()
    });
    let bytes = small.scoped(|| project.to_bytes().unwrap());
    let document = native.to_bytes().len() as u64;
    let per_document = Limits {
        document_bytes: document - 1,
        other_bytes: u64::MAX / 2,
    };
    let whole = Limits {
        document_bytes: document,
        other_bytes: 0,
    };
    let refusals = [
        Package::read_bytes(&bytes, &per_document).unwrap_err(),
        Package::read_bytes(&bytes, &whole).unwrap_err(),
    ];
    let ja: Vec<String> = refusals.iter().map(|e| Lang::Ja.io_error(e)).collect();
    let en: Vec<String> = refusals.iter().map(|e| Lang::En.io_error(e)).collect();
    for text in ja.iter().chain(&en) {
        assert!(!text.contains("MiB") && !text.contains(&id), "{text}");
    }
    assert!(
        ja.iter().all(|t| t.contains("「レイヤーのメモリ」の予算")),
        "{ja:?}"
    );
    assert!(
        en.iter()
            .all(|t| t.contains("Layer memory budget") && !has_japanese(t)),
        "{en:?}"
    );
    assert_ne!(en[0], en[1], "セットごとの正本と全体を言い分ける");
    // 復旧の世代も同じ数え方・同じ言い方
    let root = std::env::temp_dir().join(format!("yolu-i18n-budget-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    crate::common::tmp::clean_up_after_test(&root);
    let store = GenerationStore::new(&root);
    store
        .commit(
            project.original_archive().entries(),
            &yolu_io::CommitOptions {
                expected: None,
                keep: None,
                share: true,
            },
        )
        .unwrap();
    let Err(refused) = store.clone().with_limits(per_document).load() else {
        panic!("予算で断らない")
    };
    let (ja, en) = (
        Lang::Ja.store_error(&refused),
        Lang::En.store_error(&refused),
    );
    assert!(
        ja.contains("「レイヤーのメモリ」の予算") && !ja.contains("MiB"),
        "{ja}"
    );
    assert!(en.contains("Layer memory budget"), "{en}");
    let _ = std::fs::remove_dir_all(&root);
}

/// 予算の呼び方は、設定の欄・設定のキーの名前・使用量の内訳・断りの文のどこでも「レイヤーのメモリ」（Layer memory）で、古い「レイヤーの画素」
/// （Layer pixels）は画面のどこにも残らない。設定のキー（source_budget_mib）は変えない。
#[test]
fn the_layer_memory_budget_has_one_name_in_both_languages() {
    use yolu_app::settings::{setting_name, BudgetKind};
    assert_eq!(
        BudgetKind::Source.key(),
        "source_budget_mib",
        "設定ファイルのキーは変えない"
    );
    assert_eq!(
        setting_name(Lang::Ja, "source_budget_mib"),
        "レイヤーのメモリ"
    );
    assert_eq!(setting_name(Lang::En, "source_budget_mib"), "Layer memory");
    for lang in Lang::ALL {
        for key in ["undo_budget_mib", "source_budget_mib", "stroke_budget_mib"] {
            let name = setting_name(lang, key);
            assert!(
                !name.contains("画素") && !name.contains("pixels"),
                "{key}: {name}"
            );
        }
    }
    // 断りの文（.ylp・復旧の世代・PSD の取り込みと書き出し）も同じ名前
    for (ja, en) in [
        (
            yolu_io::OVER_LAYER_PIXELS_DOCUMENT,
            Lang::En.io_error(&yolu_io::Error::Budget(
                yolu_io::OVER_LAYER_PIXELS_DOCUMENT.into(),
            )),
        ),
        (
            yolu_io::OVER_LAYER_PIXELS_TOTAL,
            Lang::En.io_error(&yolu_io::Error::Budget(
                yolu_io::OVER_LAYER_PIXELS_TOTAL.into(),
            )),
        ),
    ] {
        assert!(
            ja.contains("「レイヤーのメモリ」") && !ja.contains("画素"),
            "{ja}"
        );
        assert!(
            en.contains("Layer memory budget") && !en.contains("pixels"),
            "{en}"
        );
    }
}

/// 古い形式を開いたときの io の知らせは、件数ではなく内容を日英それぞれで読める。
#[test]
fn opening_an_old_format_reports_what_io_noted_in_both_languages() {
    let fixtures =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../yolu-io/tests/fixtures");
    for lang in Lang::ALL {
        let mut state = AppState::new_in(32, 32, lang);
        state.apply(Action::OpenProject(fixtures.join("format3.ylp")));
        let message = state.message.clone();
        assert!(
            message.starts_with(lang.pick("開きました", "Opened")),
            "{message}"
        );
        match lang {
            Lang::Ja => assert!(
                message.contains("マテリアル参照を形式7へメモリ上で移行しました"),
                "{message}"
            ),
            Lang::En => {
                assert!(
                    message.contains("Migrated format 3 material references to format 7 in memory"),
                    "{message}"
                );
                assert!(!has_japanese(&message), "{message}");
                assert!(!message.contains("notices"), "{message}");
            }
        }
    }
}

#[test]
fn link_status_and_reasons_follow_the_language() {
    use yolu_app::livelink::{reason_text, LinkStatus, LinkView};
    use yolu_protocol::files::{Problem, Reason};
    for lang in Lang::ALL {
        for status in [
            LinkStatus::Off,
            LinkStatus::Accepting,
            LinkStatus::Failed(lang.pick("理由", "reason").into()),
        ] {
            let view = LinkView {
                status,
                target: Some("Avatar".into()),
                problems: vec![Problem::new("Accessory", Reason::BoneNotFound)],
                ..Default::default()
            };
            let text = view.tooltip(lang);
            assert_eq!(has_japanese(&text), lang == Lang::Ja, "{text}");
            let label = view.state_label(lang);
            assert_eq!(has_japanese(label), lang == Lang::Ja, "{label}");
        }
        for reason in Reason::ALL {
            let text = reason_text(lang, reason.as_str());
            assert_eq!(has_japanese(&text), lang == Lang::Ja, "{text}");
        }
    }
}

// ───────── 英語の画面に日本語が残っていない ─────────

/// 英語で作ったウィンドウの全体（最初のレイヤー・テクスチャセット・プロジェクトの名前も英語）。
fn english_app() -> Harness<'static, YoluApp> {
    english_app_sized(1280.0, 800.0, Lang::En)
}

fn english_app_sized(width: f32, height: f32, lang: Lang) -> Harness<'static, YoluApp> {
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
                    AppState::new_in(64, 64, lang),
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

/// 矩形の中に収まっている文字だけ（ウィンドウの中身。順は描いた順）。
fn texts_inside(h: &Harness<'_, YoluApp>, area: Rect) -> Vec<String> {
    fn walk(shape: &Shape, area: Rect, out: &mut Vec<String>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, area, out)),
            Shape::Text(text)
                if area
                    .expand(1.0)
                    .contains_rect(Rect::from_min_size(text.pos, text.galley.size())) =>
            {
                out.push(text.galley.job.text.clone());
            }
            _ => {}
        }
    }
    let mut texts = Vec::new();
    for shape in &h.output().shapes {
        walk(&shape.shape, area, &mut texts);
    }
    texts
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

fn assert_english(h: &Harness<'_, YoluApp>, what: &str, data: &[String]) {
    let left = japanese_left(h, data);
    assert!(left.is_empty(), "{what}: {left:?}");
    let clipped = clipped_texts(h);
    assert!(clipped.is_empty(), "{what}: {clipped:#?}");
}

#[test]
fn english_docks_menus_and_layer_kinds_have_no_japanese() {
    gpu_thread::run(english_docks_menus_and_layer_kinds_have_no_japanese_gpu);
}

fn english_docks_menus_and_layer_kinds_have_no_japanese_gpu() {
    let mut h = english_app();
    assert_english(&h, "default", &[]);
    // 見張りが働いていること: 同じ画面を日本語にすると日本語が見つかり、英語の文字が描かれている
    assert!(all_texts(&h).iter().any(|t| t.contains("Layers")));
    h.state_mut().state.lang = Lang::Ja;
    h.run();
    assert!(!japanese_left(&h, &[]).is_empty());
    h.state_mut().state.lang = Lang::En;
    h.run();
    for tab in [
        Tab::SubTools,
        Tab::Assets,
        Tab::Color,
        Tab::Channels,
        Tab::TextureSets,
        Tab::Layers,
        Tab::Properties,
        Tab::Material,
        Tab::View3d,
        Tab::Canvas,
    ] {
        click_tab(&mut h, tab);
        assert_english(&h, tab.title_in(Lang::En), &[]);
    }
    // ツールプロパティの「塗るチャンネル」（開いて、全部のチャンネルを組へ）
    open_paint_channels(&mut h);
    for channel in yolu_app::matpaint::CHANNELS {
        h.state_mut()
            .state
            .apply(Action::Mat(yolu_app::matpaint::MatAction::Enabled(true)));
        h.state_mut()
            .state
            .apply(Action::Mat(yolu_app::matpaint::MatAction::Channel(
                channel, true,
            )));
    }
    h.run();
    assert_english(&h, "paint channels", &[]);
    // レイヤーの種類・マスク・合成モード・ブラシの種類ごとのプロパティ
    let apply = |h: &mut Harness<'_, YoluApp>, action: Action| {
        h.state_mut().state.apply(action);
        h.run();
    };
    click_tab(&mut h, Tab::Properties);
    for edit in [
        Edit::NewGroup,
        Edit::NewFill,
        Edit::NewAdjustment(AdjustmentKind::ALL[0]),
        Edit::NewAdjustment(AdjustmentKind::ALL[1]),
        Edit::NewAdjustment(AdjustmentKind::ALL[2]),
    ] {
        apply(&mut h, Action::M2(edit));
        assert_english(&h, "layer kind", &[]);
    }
    let id = h.state().state.selected_layer.unwrap();
    apply(&mut h, Action::M2(Edit::AddMask(id)));
    apply(&mut h, Action::M2Ui(UiOp::EditMask(true)));
    assert_english(&h, "mask", &[]);
    apply(&mut h, Action::M2Ui(UiOp::EditMask(false)));
    apply(&mut h, Action::SetBlend(id, BlendMode::Multiply));
    // ブラシの詳細のウィンドウ: 全カテゴリ・効果の種類・デュアルブラシ
    h.state_mut().state.brushes.ui.detail.open = true;
    for category in Category::ALL {
        h.state_mut().state.brushes.ui.detail.category = category;
        h.run();
        assert_english(&h, category.name(Lang::En), &[]);
    }
    h.state_mut().state.brushes.ui.detail.category = Category::Effect;
    for kind in EffectKind::ALL {
        apply(&mut h, Action::M2Ui(UiOp::Brush(BrushOp::Effect(kind))));
        assert_english(&h, kind.name(Lang::En), &[]);
    }
    apply(
        &mut h,
        Action::M2Ui(UiOp::Brush(BrushOp::Effect(EffectKind::Paint))),
    );
    apply(
        &mut h,
        Action::M2Ui(UiOp::Brush(BrushOp::DualEnabled(true))),
    );
    h.state_mut().state.brushes.ui.detail.category = Category::Dual;
    h.run();
    assert_english(&h, "dual brush", &[]);
    h.state_mut().state.brushes.ui.detail.open = false;
    h.run();
    // 一覧: 全グループ（組み込みの名前・利用者のブラシ）
    click_tab(&mut h, Tab::SubTools);
    apply(&mut h, Action::Brush(BrushAction::Add));
    for group in Group::ALL {
        h.state_mut().state.show_brush_group(group);
        h.run();
        assert_english(&h, group.name(Lang::En), &[]);
    }
    // メニュー（開いているあいだは項目も描く）
    for (i, title) in ["File", "Edit", "Layer", "View", "Help"]
        .into_iter()
        .enumerate()
    {
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
    gpu_thread::run(switching_language_at_runtime_renames_the_defaults_but_not_the_users_names_gpu);
}

fn switching_language_at_runtime_renames_the_defaults_but_not_the_users_names_gpu() {
    let mut h = english_app_sized(1280.0, 800.0, Lang::Ja);
    let names = |h: &Harness<'_, YoluApp>| {
        let s = &h.state().state;
        (
            s.project_name.clone(),
            s.sets.iter().next().unwrap().name.clone(),
            s.doc.layers()[0].name().to_owned(),
        )
    };
    assert_eq!(
        names(&h),
        (
            "名称未設定".into(),
            "テクスチャセット 1".into(),
            "レイヤー 1".into()
        )
    );
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(Lang::En)));
    h.run();
    assert_eq!(
        names(&h),
        ("Untitled".into(), "Texture Set 1".into(), "Layer 1".into())
    );
    // 付け直しは編集ではない（変更の印も Undo の履歴も付かない）
    assert!(!h.state().state.modified && !h.state().state.doc.can_undo());
    assert!(all_texts(&h).iter().any(|t| t.contains("Untitled")));
    for tab in [Tab::TextureSets, Tab::Layers] {
        click_tab(&mut h, tab);
        assert_english(&h, tab.title_in(Lang::En), &[]);
    }
    // 戻すと日本語の既定の名前に戻る
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(Lang::Ja)));
    h.run();
    assert_eq!(
        names(&h),
        (
            "名称未設定".into(),
            "テクスチャセット 1".into(),
            "レイヤー 1".into()
        )
    );

    // 利用者が付けた名前は、言語を替えても変えない
    let uid = h.state().state.sets.iter().next().unwrap().uid;
    h.state_mut().state.rename_set(uid, "Mine").unwrap();
    h.state_mut().state.project_name = "Work".into();
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(Lang::En)));
    h.run();
    assert_eq!(names(&h), ("Work".into(), "Mine".into(), "Layer 1".into()));
    // 編集した文書のレイヤーは、既定の名前のままでも変えない（Undo の履歴とずれる）
    h.state_mut().state.apply(Action::M2(Edit::NewGroup));
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(Lang::Ja)));
    h.run();
    assert_eq!(h.state().state.doc.layers()[0].name(), "Layer 1");
    assert_eq!(h.state().state.project_name, "Work");
}

// ───────── 言語の選択の保存と復元（アプリの配線） ─────────

/// 設定のファイルを使うアプリ（`settings` は設定のファイルの場所。設定に言語が無いときは既定の日本語）。
fn app_with_settings(settings: &std::path::Path) -> Harness<'static, YoluApp> {
    app_with_system_lang(settings, Lang::default())
}

/// `app_with_settings` に、OS の言語（設定に言語が無いときの言語）を渡すもの。
fn app_with_system_lang(settings: &std::path::Path, system: Lang) -> Harness<'static, YoluApp> {
    let settings = settings.to_path_buf();
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(1280.0, 800.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .renderer(common::shared_gpu::renderer())
        .build_eframe(move |cc| {
            with_render_state_cpu_canvas(
                YoluApp::for_context_with_system_lang(
                    &cc.egui_ctx,
                    Some(settings),
                    PenInput::detached(),
                    system,
                ),
                cc.wgpu_render_state.as_ref(),
            )
        });
    // 中央は 1 つの組（`common::app` と同じ並び）
    h.state_mut().dock = common::tabbed_center_dock(1280.0);
    h.run();
    h
}

fn settings_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/i18n-settings-tests")
        .join(std::process::id().to_string())
        .join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn language_choice_is_written_when_changed_and_restored_at_startup() {
    gpu_thread::run(language_choice_is_written_when_changed_and_restored_at_startup_gpu);
}

fn language_choice_is_written_when_changed_and_restored_at_startup_gpu() {
    let dir = settings_dir("restore");
    let path = dir.join("YoluPainter").join("settings.conf");
    // 設定が無い初回は日本語。選ぶと、そのフレームのうちに書く
    let mut h = app_with_settings(&path);
    assert_eq!(h.state().state.lang, Lang::Ja);
    assert!(!path.exists());
    assert_eq!(h.state().state.message, "");
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(Lang::En)));
    h.run();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=en\n");
    drop(h);
    // 次の起動は、書いた言語で始まる（最初の名前も英語）
    let h = app_with_settings(&path);
    assert_eq!(h.state().state.lang, Lang::En);
    assert_eq!(h.state().state.project_name, "Untitled");
    assert_eq!(h.state().state.message, "");
    assert_eq!(
        h.state().state.sets.iter().next().unwrap().name,
        "Texture Set 1"
    );
    drop(h);
    // 選び直すと書き直す。同じ選択では書き直さない（外から書き換えた中身をそのまま）
    let mut h = app_with_settings(&path);
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(Lang::Ja)));
    h.run();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=ja\n");
    std::fs::write(&path, "language=en\n").unwrap();
    h.run();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "language=en\n",
        "選択が変わらなければ書かない"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unwritable_settings_report_once_and_do_not_break_the_app() {
    gpu_thread::run(unwritable_settings_report_once_and_do_not_break_the_app_gpu);
}

fn unwritable_settings_report_once_and_do_not_break_the_app_gpu() {
    let dir = settings_dir("unwritable");
    let path = dir.join("settings.conf");
    let mut h = app_with_settings(&path);
    // 置き換える前に失敗して書けない（このスレッドの置換を失敗させる。settings.rs の試験と同じ手）
    yolu_io::atomic::failing(|| {
        h.state_mut()
            .state
            .apply(Action::M2Ui(UiOp::Language(Lang::En)));
        h.run();
        assert_eq!(h.state().state.lang, Lang::En, "書けなくても言語は替わる");
        assert_eq!(h.state().state.message, "Cannot save the settings.");
        assert!(!path.exists());
        // 同じ選択で毎フレーム書き直さない（知らせを消したら、出し直さない）
        h.state_mut().state.message.clear();
        h.run();
        h.run();
        assert_eq!(h.state().state.message, "");
        // ウィンドウの操作はそのまま動く
        h.state_mut().state.apply(Action::NewProject);
        assert_eq!(h.state().state.message, "New project created.");
    });
    // 書けるようになれば、次に選んだときに書く
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(Lang::Ja)));
    h.run();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=ja\n");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn broken_settings_fall_back_to_japanese_and_are_repaired_by_choosing() {
    gpu_thread::run(broken_settings_fall_back_to_japanese_and_are_repaired_by_choosing_gpu);
}

fn broken_settings_fall_back_to_japanese_and_are_repaired_by_choosing_gpu() {
    let dir = settings_dir("broken");
    let path = dir.join("settings.conf");
    std::fs::write(&path, "language=unknown").unwrap();
    let mut h = app_with_settings(&path);
    assert_eq!(h.state().state.lang, Lang::Ja);
    assert_eq!(h.state().state.message, "言語の設定を読めません。");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "language=unknown",
        "選ぶまで壊れたファイルには触らない"
    );
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(Lang::En)));
    h.run();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=en\n");
    drop(h);
    let h = app_with_settings(&path);
    assert_eq!(h.state().state.lang, Lang::En);
    assert_eq!(h.state().state.message, "");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_first_start_follows_the_system_language_and_a_saved_language_wins() {
    gpu_thread::run(the_first_start_follows_the_system_language_and_a_saved_language_wins_gpu);
}

fn the_first_start_follows_the_system_language_and_a_saved_language_wins_gpu() {
    let dir = settings_dir("system-language");
    let path = dir.join("YoluPainter").join("settings.conf");
    let write = |text: &str| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
    };
    let menu = |lang: Lang| lang.pick("ファイル", "File");
    // 初めての起動（設定のファイルが無い）は OS の言語で始まる。最初の名前も画面の文もその言語で、読むだけではファイルを作らない
    for system in Lang::ALL {
        let h = app_with_system_lang(&path, system);
        assert_eq!(h.state().state.lang, system);
        assert_eq!(
            h.state().state.sets.iter().next().unwrap().name,
            yolu_app::sets::first_set_name(system)
        );
        assert_eq!(h.state().state.message, "");
        h.get_by_label(menu(system));
        assert!(h
            .query_by_label(menu(system.pick(Lang::En, Lang::Ja)))
            .is_none());
        assert!(!path.exists());
    }
    for system in Lang::ALL {
        // 保存した言語が先（OS の言語と同じでも違っても）
        for saved in Lang::ALL {
            write(&format!("language={}\n", saved.pick("ja", "en")));
            let h = app_with_system_lang(&path, system);
            assert_eq!(h.state().state.lang, saved, "OS {system:?}・保存 {saved:?}");
            assert_eq!(h.state().state.message, "");
            h.get_by_label(menu(saved));
        }
        // 言語の行が無い設定: ほかの設定は読み、言語は OS の言語
        write("export_padding=8\n");
        let h = app_with_system_lang(&path, system);
        assert_eq!(h.state().state.lang, system);
        assert_eq!(h.state().state.settings().export_padding, 8);
        assert_eq!(h.state().state.message, "");
        // 言語の値が壊れた設定: 知らせは OS の言語で出し、選ぶまでファイルには触らない
        write("language=unknown");
        let h = app_with_system_lang(&path, system);
        assert_eq!(h.state().state.lang, system);
        assert_eq!(
            h.state().state.message,
            system.pick(
                "言語の設定を読めません。",
                "Cannot read the language setting."
            )
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=unknown");
        // 読めない設定（`キー=値` でない行）: 全部既定で、言語は OS の言語
        write("garbage\n");
        let h = app_with_system_lang(&path, system);
        assert_eq!(h.state().state.lang, system);
        assert_eq!(
            h.state().state.message,
            system.pick("設定を読めません。", "Cannot read the settings.")
        );
    }
    // OS の言語で始めたあとに言語を選ぶと、選んだ言語を書き、次の起動は OS の言語によらずそれで始まる
    let _ = std::fs::remove_file(&path);
    let mut h = app_with_system_lang(&path, Lang::En);
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(Lang::Ja)));
    h.run();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=ja\n");
    drop(h);
    let h = app_with_system_lang(&path, Lang::En);
    assert_eq!(h.state().state.lang, Lang::Ja);
    drop(h);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn backups_to_keep_is_written_when_changed_and_restored_at_startup() {
    gpu_thread::run(backups_to_keep_is_written_when_changed_and_restored_at_startup_gpu);
}

fn backups_to_keep_is_written_when_changed_and_restored_at_startup_gpu() {
    use yolu_app::prefs::PrefsAction;
    use yolu_io::BackupKeep;
    let dir = settings_dir("backups");
    let path = dir.join("settings.conf");
    let mut h = app_with_settings(&path);
    assert_eq!(
        h.state().state.prefs.settings.backups,
        BackupKeep::All,
        "設定が無い初回はすべて残す"
    );
    h.state_mut()
        .state
        .apply(Action::Prefs(PrefsAction::SetBackups(BackupKeep::Count(5))));
    h.run();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "language=ja\nbackups=5\n"
    );
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(Lang::En)));
    h.run();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "language=en\nbackups=5\n",
        "言語を替えても退避の数は残る"
    );
    drop(h);
    // 次の起動は、書いた数で始まる（知らせは無い）
    let mut h = app_with_settings(&path);
    assert_eq!(h.state().state.prefs.settings.backups, BackupKeep::Count(5));
    assert_eq!(h.state().state.lang, Lang::En);
    assert_eq!(h.state().state.message, "");
    // 0 も書いて戻る。「すべて」に戻すと行が消える。同じ選択では書き直さない
    h.state_mut()
        .state
        .apply(Action::Prefs(PrefsAction::SetBackups(BackupKeep::Count(0))));
    h.run();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "language=en\nbackups=0\n"
    );
    h.state_mut()
        .state
        .apply(Action::Prefs(PrefsAction::SetBackups(BackupKeep::All)));
    h.run();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=en\n");
    std::fs::write(&path, "language=en\nbackups=9\n").unwrap();
    h.run();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "language=en\nbackups=9\n",
        "選択が変わらなければ書かない"
    );
    drop(h);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_broken_backups_value_falls_back_to_keeping_all_with_a_reason_and_keeps_the_language() {
    gpu_thread::run(
        a_broken_backups_value_falls_back_to_keeping_all_with_a_reason_and_keeps_the_language_gpu,
    );
}

fn a_broken_backups_value_falls_back_to_keeping_all_with_a_reason_and_keeps_the_language_gpu() {
    use yolu_app::prefs::PrefsAction;
    use yolu_io::BackupKeep;
    let dir = settings_dir("backups-broken");
    let path = dir.join("settings.conf");
    for (bad, shown) in [
        ("5000", "5000"),
        ("-1", "-1"),
        ("many", "many"),
        ("2.5", "2.5"),
    ] {
        let text = format!("language=en\nbackups={bad}\n");
        std::fs::write(&path, &text).unwrap();
        let mut h = app_with_settings(&path);
        assert_eq!(h.state().state.lang, Lang::En, "言語は読めたまま");
        assert_eq!(
            h.state().state.prefs.settings.backups,
            BackupKeep::All,
            "{bad}"
        );
        assert_eq!(
            h.state().state.message,
            format!("Invalid Backups to Keep setting ({shown}); keeping all.")
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            text,
            "選ぶまで壊れたファイルには触らない"
        );
        h.state_mut()
            .state
            .apply(Action::Prefs(PrefsAction::SetBackups(BackupKeep::Count(3))));
        h.run();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "language=en\nbackups=3\n",
            "選び直すと直る"
        );
    }
    // 日本語は日本語で理由を言う
    std::fs::write(&path, "backups=1001\n").unwrap();
    let h = app_with_settings(&path);
    assert_eq!(
        h.state().state.message,
        "退避を残す数の設定が正しくありません（1001）。すべて残します。"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_settings_window_opens_from_the_edit_menu_and_changes_the_backups_in_both_languages() {
    gpu_thread::run(
        the_settings_window_opens_from_the_edit_menu_and_changes_the_backups_in_both_languages_gpu,
    );
}

fn the_settings_window_opens_from_the_edit_menu_and_changes_the_backups_in_both_languages_gpu() {
    use egui_kittest::kittest::NodeT;
    use yolu_app::prefs::{self, Category, PrefsAction};
    use yolu_io::BackupKeep;
    // 退避の欄は「ファイル」の区分にある。1280 × 800 に最後の行まで収まる
    for lang in Lang::ALL {
        let mut h = english_app_sized(1280.0, 800.0, lang);
        let (view, item) = (lang.pick("編集", "Edit"), lang.pick("設定…", "Settings…"));
        let (title, label, keep_all) = (
            lang.pick("設定", "Settings"),
            lang.pick("退避を残す数", "Backups to keep"),
            lang.pick("すべて残す", "Keep all"),
        );
        assert!(prefs::last_rect(&h.ctx).is_none());
        let at = menu_title(&h, view).center();
        click(&mut h, at);
        let at = popup_item(&h, item).center();
        click(&mut h, at);
        assert!(h.state().state.prefs.open, "{lang:?}");
        h.run();
        // ウィンドウは画面の中にある
        let rect = prefs::last_rect(&h.ctx).expect("ウィンドウを描いた");
        assert!(
            Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 800.0)).contains_rect(rect),
            "{rect:?}"
        );
        // 初めは「一般」の区分で、言語の名前が切れずに出る
        let shown = texts_inside(&h, rect);
        for want in [lang.pick("日本語", "English"), lang.pick("一般", "General")] {
            assert!(
                shown.iter().any(|t| t == want),
                "{lang:?}: {want} {shown:?}"
            );
        }
        if lang == Lang::En {
            assert_english(&h, "settings window", &[]);
        }
        h.state_mut()
            .state
            .apply(Action::Prefs(PrefsAction::Choose(Category::Files)));
        h.run();
        // 退避の欄の文字は、欄の名前・チェックの名前・値だけ（説明文・注記・開発用の数を置かない。閉じるは絵とツールチップ）。
        // 値は、画面に出している数（すべて残す間は、最後に選んだ数）。ウィンドウには、ほかの設定の欄もある
        let only = |h: &Harness<'_, YoluApp>, value: &str, what: &str| {
            let shown = texts_inside(h, rect);
            for want in [title, label, keep_all, value] {
                assert!(
                    shown.iter().any(|t| t == want),
                    "{lang:?} {what}: {want} {shown:?}"
                );
            }
            let near: Vec<&String> = shown
                .iter()
                .filter(|t| {
                    t.contains("退避")
                        || t.contains("ackup")
                        || t.contains("すべて残す")
                        || t.contains("Keep all")
                })
                .collect();
            assert_eq!(near.len(), 2, "{lang:?} {what}: {near:?}");
        };
        only(&h, "10", "開いた直後");
        // 「すべて残す」は入っている。外すと最後に選んだ数（まだ無ければ 10）になり、入れ直すとすべてに戻る
        let checked = |h: &Harness<'_, YoluApp>| {
            format!(
                "{:?}",
                h.get_by_role_and_label(egui::accesskit::Role::CheckBox, keep_all)
                    .accesskit_node()
                    .toggled()
            ) == "Some(True)"
        };
        assert!(checked(&h));
        h.get_by_role_and_label(egui::accesskit::Role::CheckBox, keep_all)
            .click();
        h.run();
        assert_eq!(
            h.state().state.prefs.settings.backups,
            BackupKeep::Count(10),
            "{lang:?}"
        );
        assert!(!checked(&h));
        only(&h, "10", "外した直後");
        h.state_mut()
            .state
            .apply(Action::Prefs(prefs::PrefsAction::SetBackups(
                BackupKeep::Count(0),
            )));
        h.run();
        assert!(!checked(&h));
        only(&h, "0", "0 を選んだ");
        h.get_by_role_and_label(egui::accesskit::Role::CheckBox, keep_all)
            .click();
        h.run();
        assert_eq!(h.state().state.prefs.settings.backups, BackupKeep::All);
        only(&h, "0", "すべて残す（最後に選んだ数のまま）");
        h.get_by_role_and_label(egui::accesskit::Role::CheckBox, keep_all)
            .click();
        h.run();
        assert_eq!(
            h.state().state.prefs.settings.backups,
            BackupKeep::Count(0),
            "{lang:?}: 最後に選んだ数に戻る"
        );
        // 閉じるボタンで閉じる。ウィンドウの文字は画面から消える
        h.get_by_label(lang.pick("閉じる", "Close")).click();
        h.run();
        assert!(!h.state().state.prefs.open);
        let texts = all_texts(&h);
        assert!(
            !texts.iter().any(|t| t == label || t == keep_all),
            "{lang:?}: {texts:?}"
        );
    }
}

/// 退避の数のスライダーをドラッグして選ぶ: 丸め・上限と 0 の端・ドラッグの間は書かず離すと 1 回だけ書く・Esc で取り消すと
/// 押す前の数が残り何も書かない。
#[test]
fn dragging_the_backups_slider_writes_the_settings_once_on_release_and_escape_keeps_the_old_count()
{
    gpu_thread::run(dragging_the_backups_slider_writes_the_settings_once_on_release_and_escape_keeps_the_old_count_gpu);
}

fn dragging_the_backups_slider_writes_the_settings_once_on_release_and_escape_keeps_the_old_count_gpu(
) {
    use yolu_app::prefs::{Category, PrefsAction};
    use yolu_io::BackupKeep;
    let dir = settings_dir("backups-drag");
    let path = dir.join("settings.conf");
    std::fs::write(&path, "language=en\nbackups=10\n").unwrap();
    let mut h = app_with_settings(&path);
    assert_eq!(
        h.state().state.prefs.settings.backups,
        BackupKeep::Count(10)
    );
    h.state_mut()
        .state
        .apply(Action::Prefs(PrefsAction::OpenAt(Category::Files)));
    h.run();
    let slider = h
        .get_by_role_and_label(egui::accesskit::Role::Slider, "Backups to keep")
        .rect();
    let y = slider.bottom() - 4.0;
    let at = |fraction: f32| egui::pos2(slider.left() + slider.width() * fraction, y);
    let backups = |h: &Harness<'_, YoluApp>| h.state().state.prefs.settings.backups;
    // 書いたかどうかは、ファイルを見張り用の中身に替えておき、書き換えられたかで見る
    let watch = || std::fs::write(&path, "watch\n").unwrap();
    let untouched = || {
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "watch\n",
            "書いてはいけない"
        )
    };
    let primary = egui::PointerButton::Primary;
    let shows = |h: &Harness<'_, YoluApp>, value: &str| {
        let rect = yolu_app::prefs::last_rect(&h.ctx).unwrap();
        assert!(
            texts_inside(h, rect).iter().any(|t| t == value),
            "{value}: {:?}",
            texts_inside(h, rect)
        );
    };

    // ドラッグの間は値が動くが、設定のファイルへは書かない
    watch();
    press(&h, at(0.25), primary);
    h.step();
    assert_eq!(backups(&h), BackupKeep::Count(250));
    assert!(h.state().state.prefs.dragging);
    untouched();
    move_to(&h, at(0.75));
    h.step();
    assert_eq!(backups(&h), BackupKeep::Count(750));
    untouched();
    // 上限と 0 の端: 溝の外へ出しても、1000 と 0 で止まる
    move_to(&h, egui::pos2(slider.right() + 80.0, y));
    h.step();
    assert_eq!(backups(&h), BackupKeep::Count(1000));
    shows(&h, "1000");
    untouched();
    move_to(&h, egui::pos2(slider.left() - 80.0, y));
    h.step();
    assert_eq!(backups(&h), BackupKeep::Count(0));
    shows(&h, "0");
    untouched();
    move_to(&h, at(0.75));
    h.step();
    assert_eq!(backups(&h), BackupKeep::Count(750));
    shows(&h, "750");
    untouched();
    // 離すと、そのときの数を 1 回だけ書く（そのあとのフレームでは書き直さない）
    release(&h, at(0.75), primary);
    h.step();
    h.run();
    assert!(!h.state().state.prefs.dragging);
    assert_eq!(backups(&h), BackupKeep::Count(750));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "language=en\nbackups=750\n"
    );
    watch();
    h.step();
    h.step();
    untouched();

    // 小数の位置は四捨五入（333.3 → 333）
    drag(&mut h, &[at(0.3333)]);
    assert_eq!(backups(&h), BackupKeep::Count(333));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "language=en\nbackups=333\n"
    );

    // Esc で止めると、押す前の数に戻り、続きのドラッグも、離したときも書かない
    watch();
    press(&h, at(0.5), primary);
    h.step();
    move_to(&h, at(0.9));
    h.step();
    assert_eq!(backups(&h), BackupKeep::Count(900));
    key(&h, egui::Key::Escape, egui::Modifiers::NONE);
    h.step();
    assert_eq!(backups(&h), BackupKeep::Count(333), "押す前の数に戻る");
    assert!(!h.state().state.prefs.dragging);
    shows(&h, "333");
    untouched();
    move_to(&h, at(0.2));
    h.step();
    assert_eq!(
        backups(&h),
        BackupKeep::Count(333),
        "止めたあとのドラッグは受けない"
    );
    release(&h, at(0.2), primary);
    h.step();
    h.run();
    assert_eq!(backups(&h), BackupKeep::Count(333));
    untouched();
    shows(&h, "333");
    std::fs::remove_dir_all(dir).unwrap();
}

/// 前の起動で選んだ数が、「すべて残す」を外したときに戻る数になる（起動時に読んだ数を、最後に選んだ数として覚える）。
#[test]
fn the_saved_count_is_what_keep_all_returns_to_after_a_restart() {
    gpu_thread::run(the_saved_count_is_what_keep_all_returns_to_after_a_restart_gpu);
}

fn the_saved_count_is_what_keep_all_returns_to_after_a_restart_gpu() {
    use yolu_app::prefs::{Category, PrefsAction};
    use yolu_io::BackupKeep;
    let dir = settings_dir("backups-remembered");
    let path = dir.join("settings.conf");
    // 「ペン」の節（macOS のタブレットの筆圧と、Windows のペンの入力）を出した形でも同じ
    for (saved, (tablet, pen_input)) in [
        (5u32, (false, false)),
        (0, (false, false)),
        (1000, (false, false)),
        (5, (true, false)),
        (5, (false, true)),
    ] {
        std::fs::write(&path, format!("language=en\nbackups={saved}\n")).unwrap();
        let mut h = app_with_settings(&path);
        assert_eq!(
            h.state().state.prefs.settings.backups,
            BackupKeep::Count(saved)
        );
        h.state_mut().state.prefs.tablet_row = tablet;
        h.state_mut().state.prefs.pen_input_row = pen_input;
        h.state_mut()
            .state
            .apply(Action::Prefs(PrefsAction::OpenAt(Category::Files)));
        h.run();
        h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Keep all")
            .click();
        h.run();
        assert_eq!(h.state().state.prefs.settings.backups, BackupKeep::All);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "language=en\n");
        // 外すと、既定の 10 ではなく保存してあった数に戻る
        h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Keep all")
            .click();
        h.run();
        assert_eq!(
            h.state().state.prefs.settings.backups,
            BackupKeep::Count(saved)
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("language=en\nbackups={saved}\n")
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn english_3d_view_and_pose_panel_have_no_japanese() {
    gpu_thread::run(english_3d_view_and_pose_panel_have_no_japanese_gpu);
}

fn english_3d_view_and_pose_panel_have_no_japanese_gpu() {
    let mut h = english_app();
    click_tab(&mut h, Tab::View3d);
    assert_english(&h, "no model", &[]);
    // モデルが無いだけのときは、空の状態の文字（「No model」）を置かない
    assert!(!all_texts(&h).iter().any(|t| t == "No model"));
    h.state_mut().state.apply(Action::LoadDemoModel);
    h.run();
    let demo = h
        .state()
        .state
        .view3d
        .model
        .as_ref()
        .map(|m| vec![m.name.clone()])
        .unwrap_or_default();
    assert_english(&h, "test cube", &demo);
    assert!(
        h.state().state.message.is_ascii(),
        "{}",
        h.state().state.message
    );
    h.state_mut()
        .state
        .apply(Action::Pose(PoseAction::LoadFigure));
    h.run();
    click_tab(&mut h, Tab::Pose);
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
    assert!(
        all_texts(&h).iter().any(|t| t == "Bones"),
        "{:?}",
        all_texts(&h)
    );
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

/// マテリアル 3 つ（流し込み先のあるもの・無いもの・割り当てなし）の Live Link のモデル。
fn three_material_model() -> yolu_protocol::Model {
    use yolu_protocol::{
        channel, ChannelRoute, MaterialInfo, MaterialKey, MeshData, Model, Submesh, TextureProperty,
    };
    let material = |name: Option<&str>, color_route: bool| MaterialInfo {
        key: match name {
            Some(n) => MaterialKey::Material {
                name: n.into(),
                asset: None,
            },
            None => MaterialKey::Unassigned,
        },
        shader: "Standard".into(),
        textures: vec![TextureProperty {
            name: "_MainTex".into(),
            width: 64,
            height: 64,
        }],
        routes: if color_route {
            vec![ChannelRoute {
                channel: channel::COLOR,
                property: "_MainTex".into(),
            }]
        } else {
            vec![]
        },
    };
    let materials = vec![
        material(Some("Skin"), true),
        material(Some("Hair"), false),
        material(None, true),
    ];
    let n = materials.len() as u32;
    Model {
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
            submeshes: (0..n)
                .map(|m| Submesh {
                    material: m,
                    indices: vec![0, 1, 2],
                })
                .collect(),
        }],
    }
}

#[test]
fn english_texture_set_states_have_no_japanese() {
    gpu_thread::run(english_texture_set_states_have_no_japanese_gpu);
}

fn english_texture_set_states_have_no_japanese_gpu() {
    let model = three_material_model();
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
    h.state_mut().state.sets.get_mut(0).unwrap().read_only =
        Some("Unsupported project features (1)".into());
    h.state_mut().state.apply(Action::SelectSet(uids[0]));
    h.run();
    assert_english(&h, "read-only set", &[]);
}

/// ウィンドウの中の代表の状態を順に出して、そのつど `visit` に見せる（初めの画面・全部のタブ・試しのモデル・ポーズのパネル・メニュー）。
fn walk_states(
    lang: Lang,
    width: f32,
    height: f32,
    mut visit: impl FnMut(&mut Harness<'static, YoluApp>, &str),
) {
    let mut h = english_app_sized(width, height, lang);
    visit(&mut h, "default");
    for tab in [
        Tab::SubTools,
        Tab::Assets,
        Tab::Color,
        Tab::Channels,
        Tab::TextureSets,
        Tab::Layers,
        Tab::Properties,
        Tab::Material,
        Tab::View3d,
        Tab::Canvas,
    ] {
        click_tab(&mut h, tab);
        visit(&mut h, tab.title_in(lang));
    }
    // ツールプロパティの「塗るチャンネル」を開いた画面
    open_paint_channels(&mut h);
    visit(&mut h, "paint channels");
    // ブラシの一覧（全グループ）と詳細のウィンドウ（全カテゴリ）
    click_tab(&mut h, Tab::SubTools);
    for group in Group::ALL {
        h.state_mut().state.show_brush_group(group);
        h.run();
        visit(&mut h, group.name(lang));
    }
    h.state_mut().state.brushes.ui.detail.open = true;
    for category in Category::ALL {
        h.state_mut().state.brushes.ui.detail.category = category;
        h.state_mut().state.brushes.ui.detail.scroll = 0.0;
        h.run();
        visit(&mut h, category.name(lang));
    }
    h.state_mut().state.brushes.ui.detail.open = false;
    h.state_mut().state.show_brush_group(Group::Pen);
    h.run();
    h.state_mut().state.apply(Action::LoadDemoModel);
    h.run();
    visit(&mut h, "test cube");
    h.state_mut()
        .state
        .apply(Action::Pose(PoseAction::LoadFigure));
    h.run();
    h.run();
    visit(&mut h, "pose panel");
    for (i, title) in lang
        .pick(
            ["ファイル", "編集", "レイヤー", "表示", "ヘルプ"],
            ["File", "Edit", "Layer", "View", "Help"],
        )
        .into_iter()
        .enumerate()
    {
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

/// 同梱のスマートマテリアルの名前は、棚の格子の 1 枚に収まる（「…」に詰められず、パネルからはみ出さない）。日英の両方。
#[test]
fn bundled_card_names_fit_in_both_languages() {
    gpu_thread::run(bundled_card_names_fit_in_both_languages_gpu);
}

fn bundled_card_names_fit_in_both_languages_gpu() {
    let truncations = Truncations::start();
    for lang in Lang::ALL {
        let mut state = AppState::new(64, 64);
        state.lang = lang;
        let mut ready = false;
        let mut h = common::gpu_thread::builder()
            .with_size(vec2(300.0, 1100.0))
            .renderer(common::shared_gpu::renderer())
            .build_ui_state(
                move |ui, state| {
                    if !ready {
                        YoluApp::setup(ui.ctx());
                        ready = true;
                        ui.ctx().request_repaint();
                        return;
                    }
                    assets::show(ui, state)
                },
                state,
            );
        h.run();
        // 棚の素材の絵は別のスレッドで作る。できるまで待って描く
        h.state_mut().shelf.wait_inspections();
        h.run();
        widgets::take_truncations();
        h.step();
        let truncated = widgets::take_truncations();
        assert!(
            truncated.is_empty(),
            "{lang:?}: 「…」に詰められた名前 {truncated:#?}"
        );
        let mut labels = Vec::new();
        for shape in &h.output().shapes {
            text_shapes(&shape.shape, shape.clip_rect, &mut labels);
        }
        for entry in yolu_core::smart_library::entries() {
            let name = entry.name(lang == Lang::Ja);
            assert!(
                labels.iter().any(|l| l == name),
                "{lang:?} {name}: {labels:?}"
            );
        }
    }
    drop(truncations);
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
    gpu_thread::run(fixed_text_is_not_truncated_at_ordinary_window_sizes_in_both_languages_gpu);
}

fn fixed_text_is_not_truncated_at_ordinary_window_sizes_in_both_languages_gpu() {
    let truncations = Truncations::start();
    // 既定のウィンドウ（main.rs の with_inner_size）と、ふつうのノート PC の大きさ
    for (width, height) in [(1600.0, 960.0), (1280.0, 800.0)] {
        for lang in Lang::ALL {
            walk_states(lang, width, height, |h, what| {
                let clipped = clipped_texts(h);
                assert!(
                    clipped.is_empty(),
                    "{lang:?} {width}x{height} {what}: {clipped:#?}"
                );
                let truncated = truncations.settled(h);
                assert!(
                    truncated.is_empty(),
                    "{lang:?} {width}x{height} {what}: 「…」に詰められた文字 {truncated:#?}"
                );
            });
        }
    }
}

/// ウィンドウの最小の大きさ（main.rs の with_min_inner_size）。ドックが狭く、日英どちらでも詰まる箱の値がある（チャンネルの名前・
/// プリセットなど。言語に依らない配置の積み残し）。`KNOWN` が詰まる文字の全部で、増えれば落ち、直せば一覧から消す。
#[test]
fn fixed_text_truncation_at_the_minimum_window_size_is_exactly_the_known_set() {
    gpu_thread::run(fixed_text_truncation_at_the_minimum_window_size_is_exactly_the_known_set_gpu);
}

fn fixed_text_truncation_at_the_minimum_window_size_is_exactly_the_known_set_gpu() {
    // チャンネルの名前（チャンネルのパネルの行。種類の欄は形式の名前だけなので、長い名前だけが詰まる）、プリセット・効果・合成モードの
    // 箱の値、テクスチャセットの名前。（レイヤーの不透明度は、パネルが狭いと合成モードの下の行へ積んで名前を詰めない。ここには入らない）
    const KNOWN_JA: [&str; 0] = [];
    // 英語のマテリアルのパネルは、最小のウィンドウ（右の列が約 231 点）で「Rendering Mode」の名前と値の「Opaque」が 1 行に収まらない。
    // 名前は行の 6 割まで広げて値の箱を残す（`look/panel.rs` の `choice`）が、それでも入らないので、名前も値も「…」で詰まる。
    const KNOWN_EN: [&str; 2] = ["Opaque", "Rendering Mode"];
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

#[test]
fn live_link_accepting_can_be_toggled_in_both_languages() {
    use yolu_app::prefs::{Category, PrefsAction};
    for lang in Lang::ALL {
        let mut h = english_app_sized(1280.0, 800.0, lang);
        h.state_mut()
            .state
            .apply(Action::Prefs(PrefsAction::OpenAt(Category::LiveLink)));
        h.run();
        let label = lang.pick(
            "Unity の Live Link を受け付ける",
            "Accept Live Link from Unity",
        );
        assert!(h.state().state.settings().livelink_on_startup);
        h.get_by_role_and_label(egui::accesskit::Role::CheckBox, label)
            .click();
        h.run();
        assert!(!h.state().state.settings().livelink_on_startup);
        h.get_by_role_and_label(egui::accesskit::Role::CheckBox, label)
            .click();
        h.run();
        assert!(h.state().state.settings().livelink_on_startup);
    }
}
