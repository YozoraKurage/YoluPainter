//! ランプの欄（`panels::ramp_rows`。グラデーションマップと塗りつぶしのグラデーションが共通で使う）: 混色・グラデーションセット・分岐点の編集・
//! ダブルクリックの色の選び・メインとサブに付いていく色・スポイトの色・混合率曲線・値のカーブ・無効・日英・説明を置かないこと・画像。
//! 欄だけを並べた見本の窓で確かめる。アプリの中でのつなぎ（層・段の選び・1 回の Undo）は `color_adjust_app.rs`・`fillfx*.rs`。
use crate::common;

use egui::{pos2, vec2, Key, Pos2, Rect, Ui};
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, SnapshotResults};
use yolu_app::eyedrop::EyedropState;
use yolu_app::lang::Lang;
use yolu_app::panels::color_popup;
use yolu_app::panels::ramp_rows::{self, Change, Features, Params};
use yolu_app::rampsets::{RampSets, MAX_USER};
use yolu_app::ui::ramp::{ops, Selection, STOPS_HEIGHT};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets::Rows;
use yolu_app::YoluApp;
use yolu_core::curve::{Curve, CurvePoint};
use yolu_core::generator::{ColorStop, LuminanceCorrection, MixMode, OpacityStop, Ramp};
use yolu_core::Rgba8;

const WIDTH: f32 = 320.0;
const KEY: (&str, u128) = ("test", 7);
const ROWS_WIDTH: f32 = WIDTH - 2.0 * t::PADDING - t::SECTION_INDENT;

struct Panel {
    ready: bool,
    ramp: Ramp,
    features: Features,
    lang: Lang,
    enabled: bool,
    scalar: bool,
    main: [f32; 4],
    sub: [f32; 4],
    sets: RampSets,
    eyedrop: EyedropState,
    failure: Option<String>,
    changes: Vec<Change>,
    height: f32,
    width: f32,
    /// false の間は欄を描かない（別の層を選んでいる間）。
    shown: bool,
    popup_open: bool,
    popup: Option<Rect>,
    selected: Selection,
}

fn draw(ui: &mut Ui, p: &mut Panel) {
    if !p.ready {
        // 書体は次のフレームから効くので、初めのフレームは準備だけ
        YoluApp::setup(ui.ctx());
        p.ready = true;
        ui.ctx().request_repaint();
        return;
    }
    if !p.shown {
        ui.ctx().request_repaint();
        return;
    }
    let area = Rect::from_min_size(ui.max_rect().min, vec2(p.width, p.height));
    ui.allocate_rect(area, egui::Sense::hover());
    ui.painter().rect_filled(area, 0.0, t::PANEL_BG);
    let mut rows = Rows::new(area, 0.0);
    rows.indent = t::SECTION_INDENT;
    let mut params = Params {
        key: KEY,
        enabled: p.enabled,
        lang: p.lang,
        main: p.main,
        sub: p.sub,
        scalar: p.scalar,
        features: p.features,
        sets: &mut p.sets,
        eyedrop: &mut p.eyedrop,
        failure: &mut p.failure,
    };
    if let Some(change) = ramp_rows::rows(ui, &mut rows, &mut params, &p.ramp) {
        p.ramp = change.ramp.clone();
        p.changes.push(change);
    }
    let id = ramp_rows::popup_id(KEY);
    p.popup_open = color_popup::is_open(ui.ctx(), id);
    p.popup = color_popup::rect(ui.ctx(), id).filter(|_| p.popup_open);
    p.selected = ramp_rows::selected(ui, KEY);
}

const MAP: Features = Features {
    mixing: true,
    value_curve: false,
};
const FILL: Features = Features {
    mixing: false,
    value_curve: true,
};

fn stop(position: f64, rgb: [u8; 3]) -> ColorStop {
    ColorStop {
        position,
        color: Rgba8::new(rgb[0], rgb[1], rgb[2], 255),
        midpoint: 0.5,
    }
}

fn three() -> Ramp {
    Ramp::new(
        vec![
            stop(0.0, [10, 20, 120]),
            stop(0.5, [200, 60, 90]),
            stop(1.0, [250, 230, 120]),
        ],
        vec![
            OpacityStop {
                position: 0.0,
                opacity: 1.0,
                midpoint: 0.5,
            },
            OpacityStop {
                position: 1.0,
                opacity: 1.0,
                midpoint: 0.5,
            },
        ],
        None,
    )
    .unwrap()
}

fn panel_with(ramp: Ramp, features: Features, lang: Lang, height: f32) -> Harness<'static, Panel> {
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(WIDTH, height))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            draw,
            Panel {
                ready: false,
                ramp,
                features,
                lang,
                enabled: true,
                scalar: false,
                main: [0.9, 0.2, 0.1, 1.0],
                sub: [0.1, 0.2, 0.9, 1.0],
                sets: RampSets::default(),
                eyedrop: EyedropState::default(),
                failure: None,
                changes: Vec::new(),
                height,
                width: WIDTH,
                shown: true,
                popup_open: false,
                popup: None,
                selected: Selection::default(),
            },
        );
    h.run();
    h.run();
    h
}

fn panel(ramp: Ramp) -> Harness<'static, Panel> {
    panel_with(ramp, MAP, Lang::Ja, 760.0)
}

fn rect(h: &Harness<'_, Panel>, label: &str) -> Rect {
    h.get_all_by_label(label)
        .next()
        .unwrap_or_else(|| panic!("{label}"))
        .rect()
}

/// 描いた文字の一覧。
fn shown_texts(h: &Harness<'_, Panel>) -> Vec<String> {
    use egui::epaint::Shape;
    fn walk(shape: &Shape, out: &mut Vec<String>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            Shape::Text(text) => out.push(text.galley.job.text.clone()),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        walk(&shape.shape, &mut out);
    }
    out
}

/// その名前が画面にあるか（押せる部品の名前か、描いた文字）。
fn has(h: &Harness<'_, Panel>, label: &str) -> bool {
    h.query_all_by_label(label).next().is_some() || shown_texts(h).iter().any(|t| t == label)
}

fn click_at(h: &mut Harness<'_, Panel>, at: Pos2) {
    h.hover_at(at);
    h.step();
    h.drag_at(at);
    h.step();
    h.drop_at(at);
    h.run();
}

fn double_click_at(h: &mut Harness<'_, Panel>, at: Pos2) {
    // egui は、直前（0.6 秒以内）の別の場所の押しがあると、続けた 2 回を 3 回押しと数える。先に間を空ける
    h.run_steps(40);
    h.hover_at(at);
    h.step();
    for _ in 0..2 {
        h.drag_at(at);
        h.step();
        h.drop_at(at);
        h.step();
    }
    h.run();
}

fn click_label(h: &mut Harness<'_, Panel>, label: &str) {
    let at = rect(h, label).center();
    click_at(h, at);
}

fn click_check(h: &mut Harness<'_, Panel>, label: &str) {
    let at = rect(h, label).left_center() + vec2(8.0, 0.0);
    click_at(h, at);
}

fn drag_through(h: &mut Harness<'_, Panel>, points: &[Pos2]) {
    h.hover_at(points[0]);
    h.step();
    h.drag_at(points[0]);
    h.step();
    for p in &points[1..] {
        h.hover_at(*p);
        h.step();
    }
    h.drop_at(*points.last().unwrap());
    h.run();
}

fn last<'a>(h: &'a Harness<'_, Panel>) -> &'a Change {
    h.state().changes.last().expect("変更が返された")
}

/// 分岐点の編集の部品の場所（「前の分岐点」のボタンの上）。
fn editor(h: &Harness<'_, Panel>) -> Rect {
    let prev = rect(h, "<");
    Rect::from_min_size(
        pos2(prev.left(), prev.top() - 4.0 - STOPS_HEIGHT),
        vec2(ROWS_WIDTH, STOPS_HEIGHT),
    )
}

/// 色の分岐点の行の、位置 p（0〜1）の画面の点。
fn stop_at(h: &Harness<'_, Panel>, p: f64) -> Pos2 {
    let e = editor(h);
    pos2(
        e.left() + 7.0 + (e.width() - 14.0) * p as f32,
        e.top() + 67.0,
    )
}

/// 不透明度の分岐点の行の、位置 p の画面の点。
fn opacity_at(h: &Harness<'_, Panel>, p: f64) -> Pos2 {
    let e = editor(h);
    pos2(
        e.left() + 7.0 + (e.width() - 14.0) * p as f32,
        e.top() + 7.0,
    )
}

fn click_stop(h: &mut Harness<'_, Panel>, p: f64) {
    let at = stop_at(h, p);
    click_at(h, at);
}

fn double_click_stop(h: &mut Harness<'_, Panel>, p: f64) {
    let at = stop_at(h, p);
    double_click_at(h, at);
}

fn click_opacity(h: &mut Harness<'_, Panel>, p: f64) {
    let at = opacity_at(h, p);
    click_at(h, at);
}

fn double_click_opacity(h: &mut Harness<'_, Panel>, p: f64) {
    let at = opacity_at(h, p);
    double_click_at(h, at);
}

fn colour_of(h: &Harness<'_, Panel>, k: usize) -> (u8, u8, u8) {
    let c = h.state().ramp.colors()[k].color;
    (c.r, c.g, c.b)
}

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("yolu-ramp-panel-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ───────── 混色 ─────────

#[test]
fn the_mixing_mode_buttons_change_the_mode_and_the_luminance_correction_follows_the_perceptual_mode(
) {
    let mut h = panel(three());
    assert!(has(&h, "混色モード") && has(&h, "輝度の補正"));
    // 通常のあいだは輝度の補正を選べない（押しても変わらない）
    let n = h.state().changes.len();
    click_label(&mut h, "最大");
    assert_eq!(h.state().changes.len(), n);
    click_label(&mut h, "知覚的");
    assert_eq!(h.state().ramp.mix_mode(), MixMode::Perceptual);
    assert!(last(&h).discrete);
    assert_eq!(
        h.state().ramp.luminance_correction(),
        LuminanceCorrection::High,
        "既定は高"
    );
    click_label(&mut h, "最大");
    assert_eq!(
        h.state().ramp.luminance_correction(),
        LuminanceCorrection::Max
    );
    click_label(&mut h, "なし");
    assert_eq!(
        h.state().ramp.luminance_correction(),
        LuminanceCorrection::None
    );
    // リニアへ替えると、知覚的でない輝度の補正は既定へそろう
    click_label(&mut h, "リニア");
    assert_eq!(h.state().ramp.mix_mode(), MixMode::Linear);
    assert_eq!(
        h.state().ramp.luminance_correction(),
        LuminanceCorrection::default()
    );
    // 分岐点の並びは混色を替えても変わらない
    assert_eq!(h.state().ramp.colors(), three().colors());
    click_label(&mut h, "通常");
    assert_eq!(h.state().ramp, three());
}

#[test]
fn the_fill_gradient_panel_has_no_mixing_rows_but_has_the_value_curve() {
    let mut h = panel_with(three(), FILL, Lang::Ja, 760.0);
    assert!(!has(&h, "混色モード") && !has(&h, "輝度の補正") && !has(&h, "混合率曲線"));
    assert!(has(&h, "値のカーブ"));
    // 値のカーブのプリセット
    click_label(&mut h, "S 字");
    assert_eq!(h.state().ramp.curve().len(), 4);
    assert!(last(&h).discrete);
    click_label(&mut h, "線形");
    assert!(h.state().ramp.value_curve().is_identity());
    // セットを当てると、混色は外れる（持てない欄）
    h.state_mut().ramp = three().with_mixing(MixMode::Linear, LuminanceCorrection::default());
    h.run();
    click_label(&mut h, "色味");
    click_label(&mut h, "セピア");
    assert!(!h.state().ramp.uses_mixing());
}

// ───────── グラデーションセット ─────────

fn builtin(en: &str) -> Ramp {
    yolu_app::rampsets::builtin::groups([0.9, 0.2, 0.1, 1.0], [0.1, 0.2, 0.9, 1.0])
        .into_iter()
        .flat_map(|g| g.items)
        .find(|i| i.en == en)
        .unwrap()
        .ramp
}

#[test]
fn a_set_swatch_applies_its_gradient_and_keeps_the_mixing_of_the_map() {
    let mut h = panel(three().with_mixing(MixMode::Perceptual, LuminanceCorrection::Low));
    click_label(&mut h, "陰影");
    click_label(&mut h, "陰影（青）");
    let applied = h.state().ramp.clone();
    assert_eq!(applied.colors(), builtin("Shade (Blue)").colors());
    assert_eq!(applied.mix_mode(), MixMode::Perceptual);
    assert_eq!(applied.luminance_correction(), LuminanceCorrection::Low);
    assert!(last(&h).discrete);
    // 「基本」のメイン→サブは、今のメインとサブの色で作る
    click_label(&mut h, "基本");
    click_label(&mut h, "メイン→サブ");
    assert_eq!(colour_of(&h, 0), (230, 51, 26));
    assert_eq!(colour_of(&h, 1), (26, 51, 230));
    // 日英
    let mut en = panel_with(three(), MAP, Lang::En, 760.0);
    click_label(&mut en, "Tones");
    click_label(&mut en, "Sepia");
    assert_eq!(en.state().ramp.colors(), builtin("Sepia").colors());
}

#[test]
fn a_set_with_a_value_curve_is_applied_straight_to_the_map_and_keeps_the_curve_in_the_fill_gradient(
) {
    let bent = Curve::new(vec![
        CurvePoint { x: 0.0, y: 0.0 },
        CurvePoint { x: 0.5, y: 0.8 },
        CurvePoint { x: 1.0, y: 1.0 },
    ])
    .unwrap();
    // 塗りつぶしのグラデーションで S 字を付けて足した組（値のカーブを持つ）
    let curved = three().with_value_curve(bent.clone());
    for (features, name, keeps) in [
        (MAP, "グラデーションマップ", false),
        (FILL, "塗りつぶし", true),
    ] {
        let mut h = panel_with(Ramp::default(), features, Lang::Ja, 760.0);
        h.state_mut()
            .sets
            .add("曲線つき", &curved, Lang::Ja)
            .unwrap();
        assert!(!h.state().sets.user()[0].ramp.value_curve().is_identity());
        h.state_mut().sets.show_group(RampSets::user_group());
        h.run();
        click_label(&mut h, "曲線つき");
        let applied = &h.state().ramp;
        assert_eq!(applied.colors(), curved.colors(), "{name}");
        if keeps {
            assert_eq!(
                applied.value_curve(),
                &bent,
                "{name}: 値のカーブの欄がある欄では残す"
            );
        } else {
            assert!(
                applied.value_curve().is_identity(),
                "{name}: 値のカーブを出さない欄には、見えず直せないカーブを入れない"
            );
        }
    }
}

#[test]
fn adding_renaming_and_removing_your_own_gradients_are_kept_in_the_settings_folder() {
    let dir = temp("own");
    let mut h = panel(
        three()
            .with_segment_curve(1, Some(Curve::identity()))
            .unwrap(),
    );
    h.state_mut().sets.attach(dir.clone());
    h.run();
    // 足す: 今のランプが「自分」の組へ入り、選ばれる
    click_label(&mut h, "今のグラデーションを自分の組に追加");
    assert_eq!(h.state().sets.user().len(), 1);
    assert!(h.state().sets.showing_user());
    assert_eq!(h.state().sets.selected, Some(0));
    assert_eq!(h.state().sets.user()[0].name, "グラデーション 1");
    assert_eq!(h.state().sets.user()[0].ramp.colors(), three().colors());
    assert!(h.state().sets.user()[0].ramp.segment_curve(1).is_some());
    assert!(dir.join("user.ylgrad").exists());
    // 別のランプにして、足した見本を押すと戻る
    h.state_mut().ramp = Ramp::default();
    h.run();
    click_label(&mut h, "グラデーション 1");
    assert_eq!(h.state().ramp.colors(), three().colors());
    // 名前を変える（欄が出て、入れて決める）
    click_label(&mut h, "名前を変える");
    h.run();
    let field = h.get_by_role(egui::accesskit::Role::TextInput);
    field.focus();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    h.run();
    h.get_by_role(egui::accesskit::Role::TextInput)
        .type_text("夕空");
    h.run();
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().sets.user()[0].name, "夕空");
    let mut again = RampSets::default();
    again.attach(dir.clone());
    assert_eq!(again.user()[0].name, "夕空", "保存された");
    // 消す
    click_label(&mut h, "自分の組から消す");
    assert!(h.state().sets.user().is_empty());
    assert!(!dir.join("user.ylgrad").exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_user_set_is_limited_and_the_reason_goes_to_the_failure_notice() {
    let dir = temp("limit");
    let mut h = panel(three());
    h.state_mut().sets.attach(dir.clone());
    for _ in 0..MAX_USER {
        h.state_mut()
            .sets
            .add("g", &Ramp::default(), Lang::Ja)
            .unwrap();
    }
    h.run();
    // 上限では足すボタンが押せない（理由はツールチップ）
    let n = h.state().sets.user().len();
    click_label(&mut h, "今のグラデーションを自分の組に追加");
    assert_eq!(h.state().sets.user().len(), n);
    // 保存に失敗する（読めたあとで、フォルダの場所がファイルでふさがれた）: 足さず、理由が状態の帯へ出る
    let blocked = temp("blocked");
    let target = blocked.join("gradients");
    let mut stuck = panel(three());
    stuck.state_mut().sets.attach(target.clone());
    assert!(
        stuck.state().sets.problem().is_none(),
        "無いフォルダは空で読める"
    );
    std::fs::write(&target, "x").unwrap();
    stuck.run();
    click_label(&mut stuck, "今のグラデーションを自分の組に追加");
    assert!(
        stuck.state().sets.user().is_empty(),
        "保存できなければ足さない"
    );
    assert!(
        stuck
            .state()
            .failure
            .as_deref()
            .unwrap_or_default()
            .starts_with("グラデーションを保存できません"),
        "{:?}",
        stuck.state().failure
    );
    // 起動のとき読めなかったファイルがある（上書きしない）: 同じく足さず、理由を出す（再起動で黙って消えない）
    let kept = temp("kept");
    let newer = kept.join("user.ylgrad");
    std::fs::write(&newer, "yolupainter-gradients 9\nfuture=1\n").unwrap();
    let mut unreadable = panel(three());
    unreadable.state_mut().sets.attach(kept.clone());
    assert!(unreadable.state().sets.problem().is_some());
    unreadable.run();
    click_label(&mut unreadable, "今のグラデーションを自分の組に追加");
    assert!(unreadable.state().sets.user().is_empty());
    assert!(
        unreadable
            .state()
            .failure
            .as_deref()
            .is_some_and(|f| f.contains("グラデーションを保存できません")
                && f.contains("新しい形式です（9）")),
        "{:?}",
        unreadable.state().failure
    );
    assert_eq!(
        std::fs::read_to_string(&newer).unwrap(),
        "yolupainter-gradients 9\nfuture=1\n",
        "読めなかったファイルには触らない"
    );
    // 英語の画面では英語で出る
    let mut en = panel_with(three(), MAP, Lang::En, 760.0);
    en.state_mut().sets.attach(kept.clone());
    en.run();
    click_label(&mut en, "Add the current gradient to your set");
    assert!(
        en.state()
            .failure
            .as_deref()
            .is_some_and(|f| f.starts_with("Cannot save the gradients")),
        "{:?}",
        en.state().failure
    );
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(blocked);
    let _ = std::fs::remove_dir_all(kept);
}

#[test]
fn a_built_in_group_cannot_be_renamed_or_removed() {
    let mut h = panel(three());
    click_label(&mut h, "陰影");
    click_label(&mut h, "陰影（赤）");
    assert_eq!(h.state().sets.selected, Some(0));
    let before = h.state().sets.user().len();
    click_label(&mut h, "名前を変える");
    click_label(&mut h, "自分の組から消す");
    assert_eq!(h.state().sets.user().len(), before);
    assert!(
        h.query_all_by_role(egui::accesskit::Role::TextInput)
            .next()
            .is_none(),
        "組み込みの名前の欄は出ない"
    );
}

// ───────── 分岐点の移動・消す・位置 ─────────

#[test]
fn the_arrows_move_between_stops_and_the_buttons_stop_at_the_ends() {
    let mut h = panel(three());
    assert_eq!(
        h.state().selected,
        Selection {
            index: 0,
            alpha: false
        }
    );
    click_label(&mut h, ">");
    assert_eq!(h.state().selected.index, 1);
    click_label(&mut h, ">");
    assert_eq!(h.state().selected.index, 2);
    // 端では次へは進めない
    click_label(&mut h, ">");
    assert_eq!(h.state().selected.index, 2);
    click_label(&mut h, "<");
    assert_eq!(h.state().selected.index, 1);
    click_label(&mut h, "<");
    click_label(&mut h, "<");
    assert_eq!(h.state().selected.index, 0);
    assert!(h.state().changes.is_empty(), "移るだけでは値は変わらない");
    // 不透明度の行を選ぶと、その行の中を移る
    click_opacity(&mut h, 1.0);
    assert_eq!(
        h.state().selected,
        Selection {
            index: 1,
            alpha: true
        }
    );
    click_label(&mut h, "<");
    assert_eq!(
        h.state().selected,
        Selection {
            index: 0,
            alpha: true
        }
    );
}

#[test]
fn removing_a_stop_keeps_two_and_the_position_field_moves_the_selected_stop() {
    let mut h = panel(three());
    click_label(&mut h, ">");
    // 位置: 0〜100% の欄（隣を越えない）
    let before = h.state().ramp.clone();
    let r = rect(&h, "位置");
    click_at(&mut h, pos2(r.left() + r.width() * 0.25, r.bottom() - 6.0));
    let moved = h.state().ramp.colors()[1].position;
    assert!(moved < 0.5 && moved > 0.0, "{moved}");
    assert!(!last(&h).discrete);
    assert_eq!(h.state().ramp.colors()[0], before.colors()[0]);
    // 消す
    click_label(&mut h, "分岐点を消す");
    assert_eq!(h.state().ramp.colors().len(), 2);
    assert!(last(&h).discrete);
    // 2 つは残す（消すボタンは押せない）
    let n = h.state().changes.len();
    click_label(&mut h, "分岐点を消す");
    assert_eq!(h.state().ramp.colors().len(), 2);
    assert_eq!(h.state().changes.len(), n);
}

// ───────── ダブルクリックの色の選び ─────────

#[test]
fn double_clicking_a_stop_opens_the_colour_picker_next_to_it_and_the_wheel_changes_the_colour_in_place(
) {
    let mut h = panel(three());
    assert!(!h.state().popup_open);
    double_click_stop(&mut h, 0.5);
    assert!(h.state().popup_open, "ダブルクリックで色の選びが出る");
    assert_eq!(h.state().selected.index, 1);
    let window = h.state().popup.expect("窓の場所");
    let anchor = stop_at(&h, 0.5);
    assert!(
        (window.left() - anchor.x).abs() < 80.0 && window.top() >= anchor.y - 4.0,
        "分岐点のそば: {window:?} {anchor:?}"
    );
    let before = colour_of(&h, 1);
    // 円の輪をドラッグして色相を変える（その場で色が変わり、離すまで 1 回の取り消しにまとめる）
    let wheel = color_popup::wheel_of(window);
    let radius = wheel.width() * 0.5 * (1.0 - 0.17 * 0.5);
    let top = pos2(wheel.center().x, wheel.center().y - radius);
    let right = pos2(wheel.center().x + radius, wheel.center().y);
    drag_through(
        &mut h,
        &[top, pos2(top.x + radius * 0.7, top.y + radius * 0.3), right],
    );
    let after = colour_of(&h, 1);
    assert_ne!(after, before, "色が変わった");
    assert!(!last(&h).discrete);
    assert!(h.state().popup_open, "ドラッグのあとも開いたまま");
    // 色相は右（0.25）。元の彩度・明度のまま
    assert_eq!(
        h.state().ramp.colors()[0].color,
        Rgba8::new(10, 20, 120, 255)
    );
    // 四角の右上（彩度 1・明度 1）を押すと、その色相の純色
    let sq = yolu_app::panels::color::wheel_square(wheel);
    click_at(&mut h, pos2(sq.right() - 1.0, sq.top() + 1.0));
    let (r, g, b) = colour_of(&h, 1);
    assert!(
        r.max(g).max(b) >= 250 && r.min(g).min(b) <= 5,
        "{r} {g} {b}"
    );
}

#[test]
fn escape_puts_the_colour_back_and_closes_the_picker_and_a_click_outside_keeps_it() {
    let mut h = panel(three());
    double_click_stop(&mut h, 0.5);
    let original = colour_of(&h, 1);
    let window = h.state().popup.unwrap();
    let wheel = color_popup::wheel_of(window);
    let sq = yolu_app::panels::color::wheel_square(wheel);
    click_at(&mut h, sq.center());
    assert_ne!(colour_of(&h, 1), original);
    // Esc: 開いたときの色へ戻して閉じる
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(colour_of(&h, 1), original, "Esc で戻る");
    assert!(!h.state().popup_open);
    // もう 1 度開いて、色を変えてから外を押す: 今の色のまま閉じる
    double_click_stop(&mut h, 0.5);
    let window = h.state().popup.unwrap();
    let sq = yolu_app::panels::color::wheel_square(color_popup::wheel_of(window));
    click_at(&mut h, sq.center());
    let picked = colour_of(&h, 1);
    assert_ne!(picked, original);
    click_at(&mut h, pos2(20.0, 740.0));
    assert!(!h.state().popup_open);
    assert_eq!(colour_of(&h, 1), picked);
}

#[test]
fn the_picker_follows_the_stop_that_is_selected_and_closes_when_another_is_chosen() {
    let mut h = panel(three());
    double_click_stop(&mut h, 0.5);
    assert!(h.state().popup_open);
    // 別の分岐点を選ぶ（矢印）と閉じる。Esc で戻すのは開いた分岐点のものだけなので、別の分岐点の色は触らない
    click_label(&mut h, ">");
    assert!(!h.state().popup_open);
    let before = colour_of(&h, 2);
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(colour_of(&h, 2), before);
    // 色の見本を押しても開く
    let swatch = rect(&h, "分岐点の色（押すと色の選びを開く）");
    click_at(&mut h, swatch.center());
    assert!(h.state().popup_open);
}

#[test]
fn double_clicking_the_opacity_row_does_not_open_the_picker() {
    let mut h = panel(three());
    double_click_opacity(&mut h, 0.0);
    assert!(!h.state().popup_open);
}

// ───────── メイン・サブに付いていく色 ─────────

#[test]
fn a_stop_set_to_main_or_sub_follows_that_colour_until_something_else_changes_it() {
    let mut h = panel(three());
    click_label(&mut h, ">");
    click_label(&mut h, "メイン");
    assert_eq!(colour_of(&h, 1), (230, 51, 26));
    // メインの色を替えると付いていく（ドラッグのようにまとめる変更）
    h.state_mut().main = [0.1, 0.8, 0.3, 1.0];
    h.run();
    assert_eq!(colour_of(&h, 1), (26, 204, 77));
    assert!(!last(&h).discrete);
    // サブの色は関係しない
    let n = h.state().changes.len();
    h.state_mut().sub = [0.5, 0.5, 0.5, 1.0];
    h.run();
    assert_eq!(h.state().changes.len(), n);
    // サブに替える
    click_label(&mut h, "サブ");
    assert_eq!(colour_of(&h, 1), (128, 128, 128));
    h.state_mut().sub = [0.0, 0.0, 1.0, 1.0];
    h.run();
    assert_eq!(colour_of(&h, 1), (0, 0, 255));
    // 指定へ戻すと付いていかない
    click_label(&mut h, "指定");
    h.state_mut().sub = [1.0, 0.0, 0.0, 1.0];
    h.run();
    assert_eq!(colour_of(&h, 1), (0, 0, 255));
}

#[test]
fn several_stops_follow_at_once_and_keep_following_while_another_stop_or_the_opacity_row_is_selected(
) {
    let mut h = panel(three());
    // 分岐点 0 をメイン、分岐点 2 をサブにする（メイン→サブ）
    click_label(&mut h, "メイン");
    click_label(&mut h, ">");
    click_label(&mut h, ">");
    click_label(&mut h, "サブ");
    let middle = colour_of(&h, 1);
    assert_eq!(colour_of(&h, 0), (230, 51, 26));
    assert_eq!(colour_of(&h, 2), (26, 51, 230));
    // 選んでいる分岐点が 2 でも、0 もメインの色に付いていく
    h.state_mut().main = [0.0, 1.0, 0.0, 1.0];
    h.state_mut().sub = [1.0, 1.0, 0.0, 1.0];
    h.run();
    assert_eq!(colour_of(&h, 0), (0, 255, 0));
    assert_eq!(colour_of(&h, 2), (255, 255, 0));
    assert_eq!(colour_of(&h, 1), middle, "付いていかない分岐点は動かない");
    assert!(!last(&h).discrete);
    // 選びを替えても（真ん中・不透明度の行）外れない
    click_label(&mut h, "<");
    h.state_mut().main = [0.0, 0.0, 1.0, 1.0];
    h.run();
    assert_eq!(colour_of(&h, 0), (0, 0, 255));
    click_opacity(&mut h, 1.0);
    h.state_mut().sub = [1.0, 0.0, 1.0, 1.0];
    h.run();
    assert_eq!(colour_of(&h, 2), (255, 0, 255));
    assert_eq!(colour_of(&h, 1), middle);
    // 指定へ戻すと、その分岐点だけやめる（もう一方は付いていく）
    click_stop(&mut h, 1.0);
    click_label(&mut h, "指定");
    h.state_mut().main = [1.0, 1.0, 1.0, 1.0];
    h.state_mut().sub = [0.0, 0.0, 0.0, 1.0];
    h.run();
    assert_eq!(colour_of(&h, 0), (255, 255, 255));
    assert_eq!(
        colour_of(&h, 2),
        (255, 0, 255),
        "指定へ戻した分岐点は付いていかない"
    );
}

#[test]
fn another_edit_to_a_followed_stop_ends_only_that_stops_following_and_a_set_ends_all() {
    let mut h = panel(three());
    click_label(&mut h, "メイン");
    click_label(&mut h, ">");
    click_label(&mut h, "メイン");
    // 分岐点 1 を色の選びで決めると、分岐点 1 だけ付いていくのをやめる
    let swatch = rect(&h, "分岐点の色（押すと色の選びを開く）");
    click_at(&mut h, swatch.center());
    let sq = yolu_app::panels::color::wheel_square(color_popup::wheel_of(h.state().popup.unwrap()));
    click_at(&mut h, sq.center());
    let chosen = colour_of(&h, 1);
    h.state_mut().main = [1.0, 1.0, 0.0, 1.0];
    h.run();
    assert_eq!(colour_of(&h, 1), chosen);
    assert_eq!(colour_of(&h, 0), (255, 255, 0), "分岐点 0 は付いていく");
    // ほかの操作（セットを当てる）で並びが替わると、全部付いていかない
    click_label(&mut h, "色味");
    click_label(&mut h, "セピア");
    h.state_mut().main = [0.2, 0.2, 0.2, 1.0];
    h.run();
    assert_eq!(h.state().ramp.colors(), builtin("Sepia").colors());
    // スポイトの色も、その分岐点だけ外す
    let mut h = panel(three());
    click_label(&mut h, "メイン");
    click_label(&mut h, ">");
    click_label(&mut h, "サブ");
    h.state_mut().eyedrop.ramp_stop_pick = Some([12, 200, 99]);
    h.run();
    h.state_mut().main = [0.0, 0.0, 0.0, 1.0];
    h.state_mut().sub = [1.0, 1.0, 1.0, 1.0];
    h.run();
    assert_eq!(colour_of(&h, 1), (12, 200, 99));
    assert_eq!(colour_of(&h, 0), (0, 0, 0));
}

#[test]
fn adding_or_removing_a_stop_keeps_the_stops_that_follow_and_shifts_their_numbers() {
    let mut h = panel(three());
    click_label(&mut h, "メイン");
    click_label(&mut h, ">");
    click_label(&mut h, ">");
    click_label(&mut h, "サブ");
    assert_eq!(h.state().ramp.colors().len(), 3);
    // 0 と 1 の間に足す: 付いていくのは 0 と（番号が 3 つ目に変わった）元の 2
    click_stop(&mut h, 0.25);
    assert_eq!(h.state().ramp.colors().len(), 4);
    assert_eq!(h.state().selected.index, 1, "足した分岐点が選ばれる");
    h.state_mut().main = [0.0, 1.0, 0.0, 1.0];
    h.state_mut().sub = [0.0, 0.0, 1.0, 1.0];
    h.run();
    assert_eq!(colour_of(&h, 0), (0, 255, 0));
    assert_eq!(colour_of(&h, 3), (0, 0, 255));
    assert_ne!(
        colour_of(&h, 1),
        (0, 255, 0),
        "足した分岐点は付いていかない"
    );
    assert_ne!(colour_of(&h, 1), (0, 0, 255));
    assert_ne!(colour_of(&h, 2), (0, 255, 0));
    assert_ne!(colour_of(&h, 2), (0, 0, 255));
    // 足した分岐点を消す: 残りは付いていく
    click_label(&mut h, "分岐点を消す");
    assert_eq!(h.state().ramp.colors().len(), 3);
    h.state_mut().main = [1.0, 0.0, 0.0, 1.0];
    h.state_mut().sub = [1.0, 1.0, 1.0, 1.0];
    h.run();
    assert_eq!(colour_of(&h, 0), (255, 0, 0));
    assert_eq!(colour_of(&h, 2), (255, 255, 255));
    // 付いていく分岐点を消すと、その印は無くなり、ほかの分岐点の印は残る
    click_label(&mut h, "<");
    click_label(&mut h, "<");
    assert_eq!(h.state().selected.index, 0);
    click_label(&mut h, "分岐点を消す");
    assert_eq!(h.state().ramp.colors().len(), 2);
    h.state_mut().main = [0.0, 0.0, 0.0, 1.0];
    h.state_mut().sub = [0.0, 1.0, 1.0, 1.0];
    h.run();
    assert_eq!(
        colour_of(&h, 1),
        (0, 255, 255),
        "サブに付いていた分岐点は番号が 0 つ減っても付いていく"
    );
    assert_ne!(
        colour_of(&h, 0),
        (0, 0, 0),
        "消した分岐点のメインの印は、隣へ移らない"
    );
}

// ───────── スポイトの色 ─────────

#[test]
fn a_colour_picked_from_the_screen_goes_to_the_selected_colour_stop_only() {
    let mut h = panel(three());
    click_label(&mut h, ">");
    h.state_mut().eyedrop.ramp_stop_pick = Some([12, 200, 99]);
    h.run();
    assert_eq!(colour_of(&h, 1), (12, 200, 99));
    assert!(last(&h).discrete);
    assert_eq!(h.state().eyedrop.ramp_stop_pick, None, "1 回だけ取り出す");
    assert_eq!(colour_of(&h, 0), (10, 20, 120));
    // 不透明度の分岐点を選んでいるときは、何もしない
    click_opacity(&mut h, 1.0);
    let n = h.state().changes.len();
    h.state_mut().eyedrop.ramp_stop_pick = Some([1, 2, 3]);
    h.run();
    assert_eq!(h.state().changes.len(), n);
}

// ───────── 混合率曲線 ─────────

#[test]
fn the_mixing_curve_toggle_starts_from_the_midpoint_and_the_curve_edits_and_goes_away_again() {
    let mut ramp = three();
    let mut colors = ramp.colors().to_vec();
    colors[0].midpoint = 0.3;
    ramp = ops::with_colors(&ramp, colors).unwrap();
    let mut h = panel(ramp);
    assert!(has(&h, "混合率曲線") && has(&h, "中点"));
    click_check(&mut h, "混合率曲線");
    let curve = h
        .state()
        .ramp
        .segment_curve(0)
        .cloned()
        .expect("曲線ができる");
    assert!(
        (curve.value(0.3).unwrap() - 0.5).abs() < 1e-6,
        "中点の混ざり方から始まる"
    );
    assert!(last(&h).discrete);
    assert!(!has(&h, "中点"), "曲線を使う区間は中点を出さない");
    // 曲線の編集: 何も無い所を押して動かすと点が足される（枠は 116）
    let label = rect(&h, "混合率曲線");
    let box_ = Rect::from_min_size(
        pos2(label.left() - 24.0, label.bottom() + 6.0),
        vec2(ROWS_WIDTH, 116.0),
    );
    let inner = box_.shrink(6.0);
    let at = |x: f32, y: f32| {
        pos2(
            inner.left() + inner.width() * x,
            inner.bottom() - inner.height() * y,
        )
    };
    let n = h.state().changes.len();
    drag_through(&mut h, &[at(0.8, 0.9), at(0.8, 0.5), at(0.8, 0.2)]);
    assert!(h.state().changes.len() > n, "曲線の編集が返る");
    let edited = h.state().ramp.segment_curve(0).cloned().unwrap();
    assert!(edited.points().len() > curve.points().len(), "点が足された");
    // 別の区間には広がらない
    assert!(h.state().ramp.segment_curve(1).is_none());
    // 外すと中点の欄が戻り、曲線は無くなる（中点は残っている）
    click_check(&mut h, "混合率曲線");
    assert!(h.state().ramp.segment_curve(0).is_none());
    assert!(has(&h, "中点"));
    assert_eq!(h.state().ramp.colors()[0].midpoint, 0.3);
}

#[test]
fn the_last_stop_has_no_segment_row_and_a_segment_curve_survives_moving_the_stop() {
    let mut h = panel(three());
    click_label(&mut h, ">");
    click_label(&mut h, ">");
    assert!(
        !has(&h, "混合率曲線") && !has(&h, "中点"),
        "最後の分岐点には区間が無い"
    );
    click_label(&mut h, "<");
    click_check(&mut h, "混合率曲線");
    assert!(h.state().ramp.segment_curve(1).is_some());
    // 位置を動かしても残る
    let r = rect(&h, "位置");
    click_at(&mut h, pos2(r.left() + r.width() * 0.3, r.bottom() - 6.0));
    assert!(h.state().ramp.segment_curve(1).is_some());
    // 分岐点を足すと区間が割れて両方に曲線が付く
    let n = h.state().ramp.segment_curves().len();
    click_stop(&mut h, 0.8);
    assert_eq!(h.state().ramp.colors().len(), 4);
    assert_eq!(h.state().ramp.segment_curves().len(), n + 1);
}

// ───────── 値のチャンネル・無効 ─────────

#[test]
fn a_value_channel_shows_a_value_slider_instead_of_the_colour_rows() {
    let mut h = panel_with(three(), FILL, Lang::Ja, 760.0);
    h.state_mut().scalar = true;
    h.run();
    assert!(has(&h, "値") && !has(&h, "メイン") && !has(&h, "指定"));
    // 色の分岐点をダブルクリックしても色の選びは出ない
    double_click_stop(&mut h, 0.5);
    assert!(!h.state().popup_open);
}

#[test]
fn a_disabled_panel_changes_nothing_and_opens_nothing() {
    let mut h = panel(three());
    h.state_mut().enabled = false;
    h.run();
    for y in (40..740).step_by(23) {
        click_at(&mut h, pos2(100.0, y as f32));
        click_at(&mut h, pos2(250.0, y as f32));
    }
    double_click_stop(&mut h, 0.5);
    assert!(h.state().changes.is_empty());
    assert!(!h.state().popup_open);
    assert_eq!(h.state().ramp, three());
}

// ───────── 見た目 ─────────

#[test]
fn the_panel_shows_names_and_values_only_in_both_languages_even_with_everything_open() {
    for lang in Lang::ALL {
        for features in [MAP, FILL] {
            let mut ramp = three().with_mixing(MixMode::Perceptual, LuminanceCorrection::High);
            if features.mixing {
                ramp = ramp.with_segment_curve(0, Some(Curve::identity())).unwrap();
            }
            let mut h = panel_with(ramp, features, lang, 900.0);
            double_click_stop(&mut h, 0.5);
            for text in shown_texts(&h) {
                assert!(text.chars().count() <= 24, "長い文字 {text:?}");
                assert!(!text.contains('。') && !text.ends_with('.'), "{text:?}");
                if lang == Lang::En {
                    assert!(
                        !text.chars().any(|c| matches!(c, '\u{3000}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}' | '\u{ff00}'..='\u{ffef}')),
                        "英語の画面に日本語 {text:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn snapshot_the_panel_with_the_picker_and_the_mixing_curve() {
    let mut results = SnapshotResults::new();
    for (lang, name) in [(Lang::Ja, "ja"), (Lang::En, "en")] {
        let ramp = three()
            .with_mixing(MixMode::Perceptual, LuminanceCorrection::High)
            .with_segment_curve(
                0,
                Some(
                    Curve::new(vec![
                        CurvePoint { x: 0.0, y: 0.0 },
                        CurvePoint { x: 0.4, y: 0.75 },
                        CurvePoint { x: 1.0, y: 1.0 },
                    ])
                    .unwrap(),
                ),
            )
            .unwrap();
        let mut h = panel_with(ramp, MAP, lang, 900.0);
        h.state_mut().sets.show_group(1);
        h.run();
        click_stop(&mut h, 0.0);
        double_click_stop(&mut h, 0.5);
        h.snapshot(format!("ramp_panel_picker_{name}"));
        results.extend_harness(&mut h);
    }
}

/// 欄の幅が細いとき（左右のドックを狭めたとき）は、ボタンと見本が折り返し、どの文字も欄の幅に収まる。
#[test]
fn a_narrow_panel_wraps_its_buttons_and_swatches_and_no_text_leaves_the_panel() {
    use egui::epaint::Shape;
    fn walk(shape: &Shape, out: &mut Vec<(String, Rect)>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            Shape::Text(text) => out.push((
                text.galley.job.text.clone(),
                Rect::from_min_size(text.pos, text.galley.size()),
            )),
            _ => {}
        }
    }
    for lang in Lang::ALL {
        for width in [200.0_f32, 240.0, 300.0] {
            let ramp = three().with_mixing(MixMode::Perceptual, LuminanceCorrection::High);
            let mut h = panel_with(ramp, MAP, lang, 1000.0);
            h.state_mut().width = width;
            h.state_mut().sets.show_group(2);
            h.run();
            let mut texts = Vec::new();
            for shape in &h.output().shapes {
                walk(&shape.shape, &mut texts);
            }
            assert!(!texts.is_empty());
            for (text, bounds) in texts {
                assert!(
                    bounds.right() <= width + 1.0,
                    "{lang:?} 幅 {width}: {text:?} が欄の外へ出る {bounds:?}"
                );
            }
            // 見本は 2 列以上で、幅が足りるだけ並ぶ
            let first = rect(
                &h,
                if lang == Lang::Ja {
                    "セピア"
                } else {
                    "Sepia"
                },
            );
            let second = rect(
                &h,
                if lang == Lang::Ja {
                    "青と橙"
                } else {
                    "Teal and Orange"
                },
            );
            assert!((first.top() - second.top()).abs() < 1.0, "同じ行に並ぶ");
            assert!(first.right() <= second.left(), "重ならない");
        }
    }
}

#[test]
fn a_picker_left_open_while_the_panel_was_not_shown_does_not_come_back() {
    let mut h = panel(three());
    double_click_stop(&mut h, 0.5);
    assert!(h.state().popup_open);
    // 別の層を選んでいる間（この欄が描かれない）
    h.state_mut().shown = false;
    h.run_steps(10);
    h.state_mut().shown = true;
    h.run_steps(3);
    assert!(!h.state().popup_open, "戻ってきても開いたままにしない");
    // 描かれない間に色は変わっていない
    assert_eq!(h.state().ramp, three());
}

#[test]
fn a_stop_that_followed_the_main_colour_does_not_jump_when_the_panel_comes_back() {
    let mut h = panel(three());
    click_label(&mut h, ">");
    click_label(&mut h, "メイン");
    let followed = colour_of(&h, 1);
    h.state_mut().shown = false;
    h.run_steps(10);
    // 欄が描かれない間にメインの色が替わっても、戻ってきたときに文書は変わらない
    h.state_mut().main = [0.0, 0.0, 1.0, 1.0];
    h.state_mut().shown = true;
    h.run_steps(4);
    assert_eq!(colour_of(&h, 1), followed);
}
