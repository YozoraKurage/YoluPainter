//! 画面の外枠の決まり（ユーザーの方針）: 状態の帯の左には知らせの文を出さない（直前の操作の結果と理由は、小さな知らせとして短く出て消える）。
//! 状態の帯の右端には、ユーザーの頼みで版とビルド・使っているメモリだけを出す。文書の大きさ・上げたタイル・合成の方式・Live Link の様子は出さない。
//! 2D と 3D のビューに見出しの帯は置かず、その高さをビューに返す。帯にあった操作は、ビューの右上の隅に重ねた小さなアイコン（文字なし。名前は
//! ツールチップ）へ。開発用の数は、画面に出さずに状態（AppState）から試験で読む。
use crate::common;

use common::*;
use egui::epaint::Shape;
use egui::{Key, Modifiers, Rect};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::lang::Lang;
use yolu_app::state::Action;
use yolu_app::ui::theme as t;
use yolu_app::{shell, toast, Tab, YoluApp};

/// 描いた文字を全部集める。
fn texts(shape: &Shape, out: &mut Vec<String>) {
    match shape {
        Shape::Vec(shapes) => shapes.iter().for_each(|s| texts(s, out)),
        Shape::Text(text) => out.push(text.galley.job.text.clone()),
        _ => {}
    }
}

fn screen_texts(h: &Harness<'_, YoluApp>) -> Vec<String> {
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        texts(&shape.shape, &mut out);
    }
    out
}

/// 開発用の数・言葉（メモリの量・上げたタイル・合成の方式・三角形の数・ビューの見出しの帯）。
const DEVELOPER: [&str; 13] = [
    "MiB",
    "KiB",
    "上げたタイル",
    "Uploaded tiles",
    "CPU で合成",
    "CPU compositing",
    "三角形",
    "triangles",
    "試しの立方体",
    "Test Cube",
    "2D ·",
    "3D ·",
    "テクスチャセット 1 ·",
];

fn assert_no_developer_text(h: &Harness<'_, YoluApp>, what: &str) {
    let texts = screen_texts(h);
    for text in &texts {
        for word in DEVELOPER {
            assert!(
                !text.contains(word),
                "{what}: 画面に開発用の言葉「{word}」: {text}\n全部: {texts:#?}"
            );
        }
    }
}

/// 描いた文字（`Shape::Text`）のうち、`text` と同じ文の矩形（画面の点）。描いていなければ None。
fn text_rect(h: &Harness<'_, YoluApp>, text: &str) -> Option<Rect> {
    fn find(shape: &Shape, text: &str) -> Option<Rect> {
        match shape {
            Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, text)),
            Shape::Text(t) if t.galley.job.text == text => {
                Some(t.galley.rect.translate(t.pos.to_vec2()))
            }
            _ => None,
        }
    }
    h.output().shapes.iter().find_map(|s| find(&s.shape, text))
}

/// 状態の帯（窓の下の端）の画素が、背景と上の縁の線だけか（左の部分。右端の版とメモリを避けて、幅の左の 2/3）。
fn assert_status_bar_left_is_empty(h: &mut Harness<'_, YoluApp>, what: &str) {
    let image = h.render().expect("描画");
    let (w, hh) = (image.width(), image.height());
    let bar_top = hh - t::STATUS_BAR_HEIGHT as u32;
    let background = *image.get_pixel(w / 2, hh - 4);
    for y in bar_top + 2..hh {
        for x in 0..w * 2 / 3 {
            assert_eq!(
                *image.get_pixel(x, y),
                background,
                "{what}: 状態の帯の左の ({x}, {y}) に何かが描かれている"
            );
        }
    }
}

/// 状態の帯の左には、知らせの文を常に出さない（開発用の数も）。直前の操作の結果と理由（message）は、小さな知らせとして状態の帯の上に
/// 短く出て、数秒で消える。message の中身と、`shell::status_text`・`state.message` を読む口は今のまま。
#[test]
fn the_status_bar_never_shows_the_message_and_it_shows_as_a_small_toast_that_goes_away() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 800.0, 512);
        h.state_mut().state.lang = lang;
        h.state_mut().apply(Action::LoadDemoModel);
        h.state_mut().state.message = String::new();
        h.run();
        // 描く・3D を出す、のあとでも、状態の帯は空（message が空なら知らせも無い）
        let c = canvas_rect(&h);
        drag(
            &mut h,
            &[
                offset(c.center(), -40.0, 0.0),
                offset(c.center(), 40.0, 10.0),
            ],
        );
        h.state_mut().state.message = String::new();
        h.run();
        assert_eq!(shell::status_text(&h.state().state), "");
        assert_no_developer_text(&h, &format!("{lang:?}"));
        assert_status_bar_left_is_empty(&mut h, &format!("{lang:?} 知らせなし"));
        let bar_top = 800.0 - t::STATUS_BAR_HEIGHT;

        // message があっても、状態の帯には出さない。小さな知らせが、状態の帯のすぐ上に出る
        h.state_mut().state.message = "試験の知らせ".into();
        h.run();
        assert_eq!(
            shell::status_text(&h.state().state),
            "試験の知らせ",
            "message の読み口は今のまま"
        );
        assert_status_bar_left_is_empty(&mut h, &format!("{lang:?} 知らせあり"));
        let toast = text_rect(&h, "試験の知らせ").expect("知らせが出ている");
        assert!(
            toast.bottom() <= bar_top,
            "{lang:?}: 知らせ {toast:?} は状態の帯 {bar_top} の上"
        );
        assert!(
            bar_top - toast.bottom() < 40.0,
            "状態の帯のすぐ上: {toast:?}"
        );

        // 数秒で消える。message は残る（試験が読む）
        for _ in 0..(((toast::INFO_SECONDS + 1.0) * 60.0) as usize) {
            h.step();
        }
        assert!(
            text_rect(&h, "試験の知らせ").is_none(),
            "{lang:?}: 普通の知らせは数秒で消える"
        );
        assert_eq!(h.state().state.message, "試験の知らせ");

        // 画面に出さなくなった数は、状態から読める
        let s = h.state();
        assert!(s.display().stats.total_tiles > 0);
        assert!(s.state.doc.allocated_bytes() > 0);
        assert_eq!(s.state.doc.width(), 512);
    }
}

/// エラー（断り・失敗）の知らせは、普通の知らせより長く出て、押せば消える。長さは種類で決まる（文の言い回しでは決めない）。
#[test]
fn an_error_toast_stays_longer_and_pressing_it_dismisses_it() {
    use yolu_app::notice::Source;
    let mut h = app(1280.0, 800.0, 256);
    h.state_mut()
        .state
        .fail(Source::Open, "開けません: 試験のファイル");
    h.run();
    assert!(text_rect(&h, "開けません: 試験のファイル").is_some());
    // 普通の知らせが消える秒数のあとも、エラーは残る
    for _ in 0..(((toast::INFO_SECONDS + 2.0) * 60.0) as usize) {
        h.step();
    }
    assert!(
        text_rect(&h, "開けません: 試験のファイル").is_some(),
        "エラーは長く出る"
    );
    // 押すと消える（文は message に残る）
    let at = h.get_by_label("開けません: 試験のファイル").rect().center();
    click(&mut h, at);
    h.run();
    assert!(
        text_rect(&h, "開けません: 試験のファイル").is_none(),
        "押したら消える"
    );
    assert_eq!(h.state().state.message, "開けません: 試験のファイル");
    // 書き直されない（操作が文を書かない）あいだは、出し直さない。別の文になれば出る
    h.run();
    assert!(text_rect(&h, "開けません: 試験のファイル").is_none());
    h.state_mut().state.info(Source::Save, "保存しました。");
    h.run();
    assert!(text_rect(&h, "保存しました。").is_some());
    // エラーでも、時間が来れば消える（ポインタを乗せていると延びるので、離しておく）
    h.event(egui::Event::PointerGone);
    h.state_mut()
        .state
        .fail(Source::Settings, "Cannot save the settings.");
    h.run();
    for _ in 0..(((toast::LONG_SECONDS + 1.0) * 60.0) as usize) {
        h.step();
    }
    assert!(text_rect(&h, "Cannot save the settings.").is_none());
}

/// 同じ文を続けて書く操作（開けないファイルをもう一度開く）は、押して消したあとも、時間切れのあとも、そのたびに知らせが出る。
/// 旧い状態の帯は最後の文を出しっぱなしにしていたので、「出したのに画面に何も無い」退行を、ここで固定する。
#[test]
fn the_same_sentence_written_by_a_repeated_operation_shows_again() {
    let missing =
        std::env::temp_dir().join(format!("yolu-chrome-missing-{}.ylp", std::process::id()));
    let mut h = app(1280.0, 800.0, 256);
    h.event(egui::Event::PointerGone);
    h.state_mut().apply(Action::OpenProject(missing.clone()));
    let text = h.state().state.message.clone();
    assert!(text.contains("を開けません（"), "{text}");
    h.run();
    assert!(text_rect(&h, &text).is_some(), "最初の 1 回");

    // 押して消したあとに、同じ文がもう一度書かれる
    let at = h.get_by_label(&text).rect().center();
    click(&mut h, at);
    h.run();
    assert!(text_rect(&h, &text).is_none(), "押したら消える");
    h.event(egui::Event::PointerGone);
    h.state_mut().apply(Action::OpenProject(missing.clone()));
    assert_eq!(h.state().state.message, text, "同じ文");
    h.run();
    assert!(
        text_rect(&h, &text).is_some(),
        "押して消したあとの同じ文も、出る"
    );

    // 時間切れのあとに、同じ文がもう一度書かれる（message は書き換わらず残ったまま）
    for _ in 0..(((toast::LONG_SECONDS + 1.0) * 60.0) as usize) {
        h.step();
    }
    assert!(text_rect(&h, &text).is_none(), "時間切れで消える");
    assert_eq!(h.state().state.message, text, "最後の文は読める");
    h.state_mut().apply(Action::OpenProject(missing));
    h.run();
    assert!(
        text_rect(&h, &text).is_some(),
        "時間切れのあとの同じ文も、出る"
    );
}

/// フレームの中で（`apply` の外から）直接書かれた文も、同じ文が続けば、そのたびに出る。ここでは、描いている間に落とした .ylp の断り
/// （落とした先を開く経路が `message` へ直接書く）。
#[test]
fn a_sentence_written_inside_a_frame_shows_again_when_it_repeats() {
    #[derive(Debug)]
    struct Dropped(std::path::PathBuf);
    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            Err("none".into())
        }
    }
    let mut h = app(1280.0, 800.0, 256);
    h.event(egui::Event::PointerGone);
    let layer = h.state().state.doc.layers().last().expect("レイヤー").id();
    let stroke = h
        .state_mut()
        .state
        .doc
        .begin_stroke(layer, &yolu_app::engine::BrushSettings::default())
        .expect("ストローク");
    let drop = |h: &mut Harness<'_, YoluApp>| {
        h.input_mut()
            .dropped_files
            .push(std::sync::Arc::new(Dropped(std::path::PathBuf::from(
                "dropped.ylp",
            ))));
        h.run();
    };
    drop(&mut h);
    let text = h.state().state.message.clone();
    assert_eq!(
        text,
        yolu_app::lang::refusals::during_stroke(Lang::Ja),
        "描いている間は開かない（断りの文は 1 つ）"
    );
    assert!(text_rect(&h, &text).is_some(), "最初の 1 回");
    let at = h.get_by_label(&text).rect().center();
    click(&mut h, at);
    h.run();
    assert!(text_rect(&h, &text).is_none(), "押したら消える");
    h.event(egui::Event::PointerGone);
    drop(&mut h);
    assert!(
        text_rect(&h, &text).is_some(),
        "フレームの中で同じ文が書かれたら、押して消したあとでも出る"
    );
    h.state_mut().state.doc.cancel_stroke(stroke);
}

/// 状態の帯の右端に、版とビルド・使っているメモリ（ユーザーの頼み）。短い数で、内訳はツールチップ。日英。左は空のまま。
#[test]
fn the_status_bar_right_end_shows_the_build_and_the_memory_with_a_breakdown_tooltip() {
    use yolu_app::usage::Usage;
    for lang in Lang::ALL {
        let mut h = app(1280.0, 800.0, 256);
        h.state_mut().state.lang = lang;
        h.state_mut().state.message = String::new();
        h.run();
        // 値が無い（測っていない）うちは何も出さない
        assert!(text_rect(&h, "0.1.0 · abc1234").is_none());
        h.state_mut().state.usage = Usage {
            build: Some("0.1.0 · abc1234".into()),
            process: Some(812 * 1024 * 1024),
            gpu: Some(1536 * 1024 * 1024),
            layers: 120 * 1024 * 1024,
            history: 64 * 1024 * 1024,
            sampled_at: Some(1.0e9),
        };
        h.run();
        let bar = Rect::from_min_max(
            egui::pos2(0.0, 800.0 - t::STATUS_BAR_HEIGHT),
            egui::pos2(1280.0, 800.0),
        );
        let build = text_rect(&h, "0.1.0 · abc1234").expect("版とビルド");
        let memory = text_rect(&h, "812 MB").expect("使っているメモリ");
        let gpu = text_rect(&h, "GPU 1.5 GB").expect("GPU のメモリ（分かるとき）");
        for r in [build, memory, gpu] {
            assert!(
                bar.contains_rect(r),
                "{lang:?}: 右端の文字 {r:?} は状態の帯 {bar:?} の中"
            );
            assert!(r.left() > 1280.0 / 2.0, "右端に寄っている: {r:?}");
        }
        assert!(
            build.right() > 1280.0 - 20.0,
            "いちばん右が版とビルド: {build:?}"
        );
        assert!(
            memory.right() < gpu.left() && gpu.right() < build.left(),
            "並び: メモリ・GPU・版: {memory:?} {gpu:?} {build:?}"
        );
        // 数は短い（1 項目 16 文字以内）。開発用の言葉（MiB・タイル・三角形…）は無い
        assert!(screen_texts(&h)
            .iter()
            .all(|t| !t.contains("MiB") && !t.contains("KiB")));
        assert_status_bar_left_is_empty(&mut h, &format!("{lang:?} 値あり"));
        // ツールチップに内訳（アプリ全体・レイヤーのメモリ・取り消しの履歴・GPU）
        let at = memory.center();
        hover_and_wait(&mut h, at);
        let tip = h
            .query_all_by_label_contains(lang.pick("レイヤーのメモリ", "Layer memory"))
            .next()
            .unwrap_or_else(|| panic!("{lang:?}: 内訳のツールチップが出ない"));
        // ツールチップの文字は、部品の値として出る
        let label = tip.accesskit_node().value().unwrap_or_default().to_string();
        for want in [
            format!(
                "{}: 812 MB",
                lang.pick("アプリ全体（実メモリ）", "App (resident)")
            ),
            format!("{}: 120 MB", lang.pick("レイヤーのメモリ", "Layer memory")),
            format!("{}: 64 MB", lang.pick("取り消しの履歴", "Undo history")),
            "GPU: 1.5 GB".to_owned(),
        ] {
            assert!(
                label.contains(&want),
                "{lang:?}: {want} が内訳に無い: {label}"
            );
        }
        // 版とビルドのツールチップ
        hover_and_wait(&mut h, build.center());
        let _ = h.get_by_label(lang.pick("版とビルド", "Version and build"));
    }
}

/// 実際の窓の外では測らない（試験の画像が揺れないように）。測るときは間隔を空け、このプロセスの量は測れる（Linux・Windows）。
#[test]
fn the_memory_is_measured_only_at_the_interval_and_the_process_size_is_available() {
    let mut s = yolu_app::state::AppState::new(64, 64);
    assert_eq!(
        s.usage,
        yolu_app::usage::Usage::default(),
        "試験の窓は測らない"
    );
    assert!(s.refresh_usage(100.0, || Some(5)));
    assert_eq!(s.usage.gpu, Some(5));
    assert_eq!(s.usage.sampled_at, Some(100.0));
    #[cfg(any(target_os = "linux", windows))]
    assert!(
        s.usage.process.is_some_and(|b| b > 1024 * 1024),
        "{:?}",
        s.usage.process
    );
    assert!(s.usage.layers > 0 || s.usage.history == 0);
    // 間隔の中では測り直さない（GPU の量を測る呼び出しも、呼ばない）
    assert!(!s.refresh_usage(100.5, || panic!("間隔の中では測らない")));
    assert!(s.refresh_usage(100.0 + yolu_app::usage::INTERVAL + 0.1, || None));
    assert_eq!(s.usage.gpu, None);
    // 版とビルドは測り直しても消えない
    s.usage.build = Some("x".into());
    assert!(s.refresh_usage(200.0, || None));
    assert_eq!(s.usage.build.as_deref(), Some("x"));
}

#[test]
fn the_views_have_no_header_band_and_get_its_height_back() {
    let mut h = app(1280.0, 800.0, 512);
    h.state_mut().apply(Action::LoadDemoModel);
    h.state_mut().state.message = String::new(); // 読んだという知らせは状態の帯に出る（見出しの帯の名前とは別）
    h.run();
    // キャンバス: 表示域はタブの帯のすぐ下から始まる（見出しの帯の分だけ下がらない）
    let strip = h.state().tab_rects[&Tab::Canvas];
    let canvas = canvas_rect(&h);
    assert!(
        canvas.top() - strip.bottom() <= 2.0,
        "キャンバスの上 {} とタブの帯の下 {}",
        canvas.top(),
        strip.bottom()
    );
    assert_no_developer_text(&h, "キャンバス");
    // 3D ビュー
    click_tab(&mut h, Tab::View3d);
    let strip = h.state().tab_rects[&Tab::View3d];
    let view = h.state().view3d_rect().expect("3D ビューが前");
    assert!(
        view.top() - strip.bottom() <= 2.0,
        "3D の上 {} とタブの帯の下 {}",
        view.top(),
        strip.bottom()
    );
    assert_no_developer_text(&h, "3D ビュー");
}

#[test]
fn the_3d_corner_icon_sits_at_the_top_right_and_is_not_a_paint_surface() {
    let mut h = app(1280.0, 800.0, 512);
    h.state_mut().apply(Action::LoadDemoModel);
    click_tab(&mut h, Tab::View3d);
    h.run();
    let view = h.state().view3d_rect().expect("3D ビュー");
    let icon = h.get_by_label("モデル全体が見える位置へ戻す").rect();
    assert!(
        view.expand(1.0).contains_rect(icon)
            && view.right() - icon.right() < 24.0
            && icon.top() - view.top() < 24.0,
        "{icon:?} は {view:?} の右上の隅"
    );
    // アイコンを押しても描き始めない（下のビューの入力へ通さない）
    let before = h.state().state.doc.can_undo();
    let at = icon.center();
    press(&h, at, egui::PointerButton::Primary);
    h.step();
    assert!(
        !h.state().state.is_stroking(),
        "隅のアイコンの上で描き始めた"
    );
    release(&h, at, egui::PointerButton::Primary);
    h.run();
    assert_eq!(h.state().state.doc.can_undo(), before, "文書は変わらない");
}

#[test]
fn the_2d_corner_icons_appear_only_when_the_state_needs_them() {
    let mut h = app(1280.0, 800.0, 512);
    // 何も変えていなければ、隅にアイコンは無い
    assert!(h
        .query_by_label("表示を回しています（-15°）。押すと回転を戻します（Shift+R）")
        .is_none());
    key(&h, Key::Minus, Modifiers::NONE);
    h.run();
    let canvas = canvas_rect(&h);
    let label = "表示を回しています（-15°）。押すと回転を戻します（Shift+R）";
    let icon = h.get_by_label(label).rect();
    assert!(
        canvas.expand(1.0).contains_rect(icon)
            && canvas.right() - icon.right() < 24.0
            && icon.top() - canvas.top() < 24.0,
        "{icon:?} は {canvas:?} の右上の隅"
    );
    // 押しても描き始めず、回転が戻る
    let at = icon.center();
    press(&h, at, egui::PointerButton::Primary);
    h.step();
    assert!(!h.state().state.is_stroking());
    release(&h, at, egui::PointerButton::Primary);
    h.run();
    assert_eq!(h.state().state.view.angle, 0.0);
    assert!(!h.state().state.doc.can_undo(), "アイコンの上では描かない");
    assert!(
        h.query_by_label(label).is_none(),
        "戻したら隅のアイコンも消える"
    );
}

/// ユーザーが名指しで「余計な文章やめろ」と言った文言（2026-10-04）。どの状態・言語・タブでも、画面に出ない。
/// 上げたタイル（状態の帯）、3D・試験・三角形 / 2D・テクスチャセット・カラー 100%（見出しの帯）、まだマテリアルに付いていない（テクスチャセットの
/// 下の帯）、メイン #000000 / サブ #FFFFFF（カラーの欄の左下の文字）。
const NAMED: [&str; 14] = [
    "上げたタイル",
    "Uploaded tiles",
    "三角形",
    "triangles",
    "2D ·",
    "3D ·",
    "付いていない",
    "未割り当て",
    "Unassigned",
    "メイン  #",
    "サブ  #",
    "Main  #",
    "Sub  #",
    "カラー 100%",
];

#[test]
fn the_texts_the_user_named_never_appear_on_screen() {
    for lang in Lang::ALL {
        for model in [false, true] {
            for tab in [Tab::Canvas, Tab::View3d] {
                let mut h = app(1280.0, 800.0, 512);
                h.state_mut().state.lang = lang;
                if model {
                    h.state_mut().apply(Action::LoadDemoModel);
                }
                click_tab(&mut h, tab);
                // 描いたあとと、知らせを消したあと
                if tab == Tab::Canvas {
                    let c = canvas_rect(&h);
                    drag(
                        &mut h,
                        &[
                            offset(c.center(), -30.0, 0.0),
                            offset(c.center(), 30.0, 5.0),
                        ],
                    );
                }
                h.state_mut().state.message = String::new();
                h.run();
                let texts = screen_texts(&h);
                for text in &texts {
                    for named in NAMED {
                        assert!(
                            !text.contains(named),
                            "{lang:?} モデル={model} {tab:?}: 画面に名指しされた文言「{named}」: {text}\n全部: {texts:#?}"
                        );
                    }
                }
                // 色の欄の左下は色の四角だけ（メインとサブの値は、四角のツールチップと 16 進の欄）。16 進の欄の「#000000」は 1 つだけ
                assert_eq!(
                    texts.iter().filter(|t| t.starts_with('#')).count(),
                    1,
                    "{texts:?}"
                );
                // テクスチャセットの行は、名前・（状態のアイコン）・解像度だけ。下に帯が無い（行の下の余白は空）
                assert!(texts.iter().any(|t| t == "512"), "解像度は行に出る");
            }
        }
    }
}

/// 窓の下の端（状態の帯と、そのすぐ上の知らせ）だけを撮って、正解の絵と比べる（ほかのパネルの変更で壊れない）。
fn bottom_shot(h: &mut Harness<'_, YoluApp>, name: &str) {
    // 直前に押した所のポインタが絵に残らないように
    h.event(egui::Event::PointerGone);
    h.step();
    let image = h.render().expect("描画");
    let height = 96u32;
    let cropped =
        image::imageops::crop_imm(&image, 0, image.height() - height, image.width(), height)
            .to_image();
    egui_kittest::image_snapshot(&cropped, name);
}

/// 絵: 右端の版・ビルド・メモリ・GPU と、左の知らせ（普通・エラー）。日英。
#[test]
fn the_status_bar_with_the_build_the_memory_and_a_toast_looks_right() {
    use yolu_app::usage::Usage;
    for (lang, suffix) in [(Lang::Ja, ""), (Lang::En, "_english")] {
        let mut h = app(1000.0, 700.0, 128);
        h.state_mut().state.lang = lang;
        h.state_mut().state.usage = Usage {
            build: Some("0.1.0 · a1b2c3d".into()),
            process: Some(812 * 1024 * 1024),
            gpu: Some(1536 * 1024 * 1024),
            layers: 120 * 1024 * 1024,
            history: 64 * 1024 * 1024,
            sampled_at: Some(1.0e9),
        };
        h.state_mut().state.info(
            yolu_app::notice::Source::Project,
            lang.pick("新しいプロジェクトを作りました。", "Created a new project."),
        );
        h.run();
        bottom_shot(&mut h, &format!("status_bar_toast{suffix}"));
        // 失敗の知らせ（左の帯が赤）
        h.state_mut().state.fail(
            yolu_app::notice::Source::Open,
            lang.pick("開けません: 試験のファイル", "Cannot open: sample file"),
        );
        h.run();
        bottom_shot(&mut h, &format!("status_bar_toast_error{suffix}"));
    }
}
