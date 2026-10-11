//! ブラシの取り込みの画面（egui_kittest）: 一覧の下のボタン・取り込んだブラシのタブと行の印・ツールチップの項目の一覧・ファイルのドロップ・
//! 詳細のウィンドウの Krita の格子と模様の選び・日英。試験のファイルは試験の中で組む（外のファイルは持ち込まない）。
#[path = "../brush_import_files/mod.rs"]
mod brush_import_files;
use crate::common;
/// 合成の .sut（SQLite）の組み立ては、読み手の試験（yolu-io）と同じものを使う。
#[allow(dead_code)]
#[path = "../../../yolu-io/tests/brush_files/sut.rs"]
mod sut_files;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use brush_import_files::*;
use common::*;
use egui::{pos2, Event, Rect};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::brushes::{BrushAction, Category, Group};
use yolu_app::lang::Lang;
use yolu_app::m2::UiOp;
use yolu_app::state::{Action, AppState, DialogRequest};
use yolu_app::{Tab, YoluApp};

type H = Harness<'static, YoluApp>;

fn st(h: &H) -> &AppState {
    &h.state().state
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/brush-import-ui-tests")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, file: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(file);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn language(h: &mut H, lang: Lang) {
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(lang)));
    h.run();
}

/// 取り込みの仕事が終わるまでフレームを回す（別のスレッドなので待つだけ）。
fn wait_import(h: &mut H) {
    let start = Instant::now();
    while st(h).is_brush_importing() && start.elapsed() < Duration::from_secs(60) {
        h.run();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!st(h).is_brush_importing(), "取り込みが終わらない");
    h.run();
}

fn with_store(h: &mut H, dir: &Path) {
    h.state_mut().state.attach_brush_store(dir.join("brushes"));
    h.run();
}

fn in_panel(r: Rect) -> bool {
    r.left() < 340.0 && r.top() > 80.0 && r.top() < 900.0
}

fn drawn_texts(h: &H) -> Vec<String> {
    fn walk(shape: &egui::epaint::Shape, out: &mut Vec<String>) {
        match shape {
            egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            egui::epaint::Shape::Text(text) => out.push(text.galley.job.text.clone()),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        walk(&shape.shape, &mut out);
    }
    out
}

/// 出ているツールチップ（複数行の文は、文字の部品の値に入る）のうち、その文字を含むものの全文。
fn tooltip_text(h: &H, needle: &str) -> Option<String> {
    h.query_all_by_role(egui::accesskit::Role::Label)
        .filter_map(|n| n.accesskit_node().value())
        .find(|text| text.contains(needle))
}

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

/// ポインタを `at` に置いて、ファイルを落とす。
fn drop_files(h: &mut H, at: egui::Pos2, files: &[&PathBuf]) {
    move_to(h, at);
    h.step();
    for f in files {
        h.input_mut()
            .dropped_files
            .push(Arc::new(Dropped((*f).clone())));
    }
    h.step();
}

/// 名前のマスの「選んでいる」印（アクセシビリティの木の切り替え）。同じ名前の部品（プロパティの欄と詳細のウィンドウの同じ格子）が
/// 全部同じ印のときだけ、その値を返す（食い違えば None）。部品が無くても None。
fn marked(h: &H, label: &str) -> Option<bool> {
    let all: Vec<bool> = h
        .query_all_by_label(label)
        .filter_map(|n| n.accesskit_node().toggled())
        .map(|t| t == egui::accesskit::Toggled::True)
        .collect();
    let first = *all.first()?;
    all.iter().all(|m| *m == first).then_some(first)
}

#[test]
fn the_import_button_asks_for_the_file_window_and_is_off_while_importing() {
    let dir = temp_dir("button");
    let mut h = app(1600.0, 960.0, 128);
    with_store(&mut h, &dir);
    h.get_by_label("ブラシを取り込む（ABR・GBR・GIH・VBR・PNG・PAT）")
        .click();
    h.run();
    assert_eq!(st(&h).dialog_request, Some(DialogRequest::ImportBrushes));
    // 取り込み中は押せない
    h.state_mut().state.dialog_request = None;
    let file = write(&dir, "chalk.gbr", &gbr_gray("Chalk"));
    h.state_mut().state.brushes.import.park_next = true;
    h.state_mut()
        .state
        .apply(Action::Brush(BrushAction::Import(vec![file])));
    h.run();
    assert!(h
        .get_by_label("ブラシを取り込む（ABR・GBR・GIH・VBR・PNG・PAT）")
        .accesskit_node()
        .is_disabled());
    // 進み具合の札に、今のファイルと取消
    assert!(
        drawn_texts(&h).iter().any(|t| t.contains("chalk.gbr")),
        "{:?}",
        drawn_texts(&h)
    );
    h.state_mut()
        .state
        .apply(Action::Brush(BrushAction::ImportCancel));
    wait_import(&mut h);
    assert!(!h
        .get_by_label("ブラシを取り込む（ABR・GBR・GIH・VBR・PNG・PAT）")
        .accesskit_node()
        .is_disabled());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn imported_brushes_get_their_own_tab_and_a_mark_whose_tooltip_lists_what_was_left_out() {
    let dir = temp_dir("mark");
    let mut h = app(1600.0, 960.0, 128);
    with_store(&mut h, &dir);
    // 取り込む前は「取り込み」のタブは無い
    assert!(h.query_by_label("取り込み").is_none());
    let colour = write(&dir, "colour.gbr", &gbr_color("Colour tip"));
    let plain = write(&dir, "plain.gbr", &gbr_gray("Plain tip"));
    h.state_mut()
        .state
        .apply(Action::Brush(BrushAction::Import(vec![colour, plain])));
    wait_import(&mut h);
    // タブが現れ、取り込んだブラシに替わって、そのグループを開いている
    assert_eq!(st(&h).shown_brush_group(), Some(Group::Imported));
    assert!(h.query_by_label("取り込み").is_some());
    assert!(h.query_by_label("Colour tip").is_some());
    assert!(h.query_by_label("Plain tip").is_some());
    // 表せなかった項目のあるブラシの行にだけ印があり、ツールチップは項目の名前の一覧（文ではない）
    let row = |h: &H, name: &str| rect_of(h, name, |r| in_panel(r) && r.width() > 200.0);
    let colour_row = row(&h, "Colour tip");
    move_to(&h, pos2(2.0, 2.0));
    h.run();
    hover_and_wait(
        &mut h,
        pos2(colour_row.left() + 30.0, colour_row.center().y),
    );
    let tip = tooltip_text(&h, "表せなかった項目").expect("ツールチップ");
    assert!(
        tip.contains("Colour tip") && tip.contains("GIMP GBR"),
        "{tip}"
    );
    assert!(tip.contains("表せなかった項目: 色つきの筆先"), "{tip}");
    assert!(!tip.contains('。'), "文ではなく項目の名前: {tip}");
    // 項目の無い行には付かない
    let plain_row = row(&h, "Plain tip");
    move_to(&h, pos2(2.0, 2.0));
    h.run();
    hover_and_wait(&mut h, pos2(plain_row.left() + 30.0, plain_row.center().y));
    let tip = tooltip_text(&h, "GIMP GBR").expect("ツールチップ");
    assert!(!tip.contains("表せなかった"), "{tip}");
    // 英語（状態の帯の知らせは、出した時の言語の文のまま残るので消してから見る）
    language(&mut h, Lang::En);
    h.state_mut().state.message.clear();
    h.run();
    let colour_row = row(&h, "Colour tip");
    move_to(&h, pos2(2.0, 2.0));
    h.run();
    hover_and_wait(
        &mut h,
        pos2(colour_row.left() + 30.0, colour_row.center().y),
    );
    let tip = tooltip_text(&h, "Not represented").expect("ツールチップ");
    assert!(tip.contains("Not represented: Colored tip"), "{tip}");
    let japanese: Vec<String> = drawn_texts(&h)
        .into_iter()
        .filter(|t| has_japanese(t))
        .collect();
    assert!(japanese.is_empty(), "英語の画面に日本語: {japanese:?}");
    // 取り込んだブラシを全部並びから外しても、グループ（タブ）は残る（グループは利用者が消す）。今のブラシはツールのほかのブラシへ
    for key in st(&h)
        .brush_entries_in(Group::Imported)
        .iter()
        .map(|e| e.key)
        .collect::<Vec<_>>()
    {
        h.state_mut()
            .state
            .apply(Action::Brush(BrushAction::Delete(key)));
    }
    h.run();
    h.run();
    assert_eq!(st(&h).shown_brush_group(), Some(Group::Pen));
    assert!(h.query_by_label("Imported").is_some());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_sample_of_a_brush_without_start_end_keeps_the_drawers_setting_while_a_brush_that_has_them_is_selected(
) {
    use sut_files::*;
    use yolu_app::brushes::sample::{key_of, SampleSpec};
    use yolu_app::engine::{Brush, StrokeAssist};
    let dir = temp_dir("row-assist");
    let mut h = app(1600.0, 960.0, 128);
    with_store(&mut h, &dir);
    let drawer = StrokeAssist {
        stabilizer: 7.0,
        taper_in: 3.0,
        taper_out: 0.0,
        curve: false,
    };
    h.state_mut().state.m2.brush.assist = drawer;
    let taper = SutBuilder::new()
        .brush(
            "Taper",
            1,
            &[
                ("BrushSize", real(30.0)),
                ("BrushUseIn", int(1)),
                ("BrushInLength", real(1130.0)),
                ("BrushUseOut", int(1)),
                ("BrushOutLength", real(14.0)),
            ],
        )
        .build();
    let files = vec![
        write(&dir, "plain.gbr", &gbr_gray("Plain tip")),
        write(&dir, "taper.sut", &taper),
    ];
    h.state_mut()
        .state
        .apply(Action::Brush(BrushAction::Import(files)));
    wait_import(&mut h);
    let find = |h: &H, name: &str| {
        st(h)
            .brush_entries_in(Group::Imported)
            .into_iter()
            .find(|e| e.name == name)
            .map(|e| e.key)
            .expect("取り込んだブラシ")
    };
    let (plain, taper) = (find(&h, "Plain tip"), find(&h, "Taper"));
    h.state_mut()
        .state
        .apply(Action::Brush(BrushAction::Select(taper)));
    h.run();
    // 持つブラシを選んでいる間、今の設定にはその値が重なっている
    assert_eq!(st(&h).m2.brush.assist.taper_in, 1130.0);
    assert_eq!(st(&h).brushes.drawer_assist, Some(drawer));
    // 持たないブラシの行の見本は、描き手の設定で描かれる（重なった値で描かれない）
    let effective = st(&h).brushes.lib.entry(plain).unwrap().effective().clone();
    let key_for = |assist: StrokeAssist| {
        key_of(
            &Brush {
                assist,
                ..effective.clone()
            },
            SampleSpec::row(false),
        )
    };
    let (right, wrong) = (key_for(drawer), key_for(st(&h).m2.brush.assist));
    assert_ne!(right, wrong, "二つの見本は別の絵");
    let start = Instant::now();
    while st(&h).brushes.samples.image(right).is_none() && start.elapsed() < Duration::from_secs(30)
    {
        h.run();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        st(&h).brushes.samples.image(right).is_some(),
        "描き手の設定の見本"
    );
    assert!(
        st(&h).brushes.samples.image(wrong).is_none(),
        "持つブラシの値で描いた見本は作られない"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn dropping_brush_files_imports_them_and_a_png_only_when_dropped_on_the_list() {
    let dir = temp_dir("drop");
    let mut h = app(1600.0, 960.0, 128);
    with_store(&mut h, &dir);
    let list = st(&h).brushes.ui.list_rect.expect("一覧を描いている");
    let inside = list.center();
    let outside = pos2(900.0, 500.0);
    let abr = write(&dir, "old.abr", &abr_v1());
    let png = write(&dir, "tip.png", b"not even a png");
    // ABR はウィンドウのどこに落としても取り込む。PNG はほかの用途と区別がつかないので、一覧の外では取り込まない
    drop_files(&mut h, outside, &[&png]);
    assert!(!st(&h).is_brush_importing());
    drop_files(&mut h, outside, &[&abr]);
    assert!(st(&h).is_brush_importing());
    wait_import(&mut h);
    assert_eq!(st(&h).brush_entries_in(Group::Imported).len(), 2);
    // 一覧の上の PNG は取り込もうとする（PNG として読めなければ、その理由を状態の帯に出す）
    drop_files(&mut h, inside, &[&png]);
    assert!(st(&h).is_brush_importing());
    wait_import(&mut h);
    assert!(st(&h).message.contains("tip.png"), "{}", st(&h).message);
    assert_eq!(st(&h).brush_entries_in(Group::Imported).len(), 2);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_krita_grid_in_the_shape_category_filters_by_name_and_picks_a_tip() {
    let mut h = app(1600.0, 960.0, 128);
    h.state_mut().state.brushes.ui.detail.open = true;
    h.state_mut().state.brushes.ui.detail.category = Category::Shape;
    h.run();
    h.run();
    assert!(st(&h).brushes.krita.is_ready());
    // 格子の中の筆先は名前で探せる
    let krita = yolu_app::brushes::store::krita();
    let first = &krita.brushes[0];
    assert!(
        h.query_by_label(first.name.as_str()).is_some(),
        "{}",
        first.name
    );
    // 検索の欄に打つと絞られる（打つ文字が合わない筆先のマスは消える）
    h.state_mut().state.brushes.krita.search = "bristle".into();
    h.run();
    let shown = st(&h).brushes.krita.matches();
    assert!(!shown.is_empty() && shown.len() < 76);
    let hidden = (0..76).find(|i| !shown.contains(i)).unwrap();
    let hidden_name = krita.brushes[hidden].name.clone();
    assert!(
        h.query_by_label(hidden_name.as_str()).is_none(),
        "絞られた筆先 {hidden_name} は出ない"
    );
    // マスを押すと、その筆先になる
    h.state_mut().state.brushes.krita.search.clear();
    h.run();
    let name = krita.brushes[hidden].name.clone();
    h.get_by_label(name.as_str()).click();
    h.run();
    assert_eq!(
        st(&h).m2.brush.tip.image,
        krita.brushes[hidden].brush.tip.image
    );
    assert_eq!(
        yolu_app::brushes::krita::current_index(&st(&h).m2.brush.tip),
        Some(hidden)
    );
    let japanese_free = {
        language(&mut h, Lang::En);
        drawn_texts(&h)
            .into_iter()
            .filter(|t| has_japanese(t))
            .count()
    };
    assert_eq!(japanese_free, 0);
}

#[test]
fn a_png_dropped_where_the_list_was_is_not_taken_while_another_tab_is_open() {
    let dir = temp_dir("drop-other-tab");
    let mut h = app(1600.0, 960.0, 128);
    with_store(&mut h, &dir);
    let inside = st(&h)
        .brushes
        .ui
        .list_rect
        .expect("一覧を描いている")
        .center();
    let png = write(&dir, "tip.png", b"not even a png");
    // ツールプロパティやブラシサイズのタブを一覧の組へ入れて前に出している間は、一覧を描かないので、前の位置へ落としても取り込まない
    for tab in [Tab::ToolProperties, Tab::BrushSize] {
        put_in_group_of(&mut h, tab, Tab::SubTools);
        assert!(st(&h).brushes.ui.list_rect.is_none(), "{tab:?}");
        drop_files(&mut h, inside, &[&png]);
        assert!(!st(&h).is_brush_importing(), "{tab:?}");
        assert!(st(&h).message.is_empty() || !st(&h).message.contains("tip.png"));
    }
    // ブラシのタブへ戻れば、一覧の上の PNG は取り込もうとする
    click_tab(&mut h, Tab::SubTools);
    h.run();
    let inside = st(&h)
        .brushes
        .ui
        .list_rect
        .expect("一覧を描いている")
        .center();
    drop_files(&mut h, inside, &[&png]);
    assert!(st(&h).is_brush_importing());
    wait_import(&mut h);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn only_the_cell_of_the_current_tip_is_marked_in_the_shape_grid() {
    let dir = temp_dir("marks");
    let mut h = app(1600.0, 960.0, 128);
    with_store(&mut h, &dir);
    h.state_mut().state.brushes.ui.detail.open = true;
    h.state_mut().state.brushes.ui.detail.category = Category::Shape;
    h.run();
    h.run();
    let round = "丸（硬さ）";
    // 丸のときは、丸のマスだけ
    assert_eq!(marked(&h, round), Some(true));
    let krita = yolu_app::brushes::store::krita();
    let hose = krita
        .brushes
        .iter()
        .position(|b| b.brush.tip.images.len() > 1)
        .unwrap();
    for index in [0, hose] {
        h.state_mut()
            .state
            .apply(Action::M2Ui(UiOp::Brush(yolu_app::m2::BrushOp::KritaTip(
                index,
            ))));
        h.run();
        h.run();
        assert_eq!(
            marked(&h, round),
            Some(false),
            "Krita の筆先 {index} を選んだとき、丸のマスに印が付く"
        );
        assert_eq!(
            marked(&h, &krita.brushes[index].name),
            Some(true),
            "{index}"
        );
    }
    // 組み込みの筆先を選べば、そのマスだけ（丸は外れる）
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Brush(yolu_app::m2::BrushOp::Tip(Some(
            "grain",
        )))));
    h.run();
    h.run();
    assert_eq!(marked(&h, round), Some(false));
    assert_eq!(marked(&h, &krita.brushes[hose].name), Some(false));
    // 取り込んだ画像のブラシを選んでも、丸にも組み込みにも Krita にも印は付かない
    let file = write(&dir, "chalk.gbr", &gbr_gray("Chalk"));
    h.state_mut()
        .state
        .apply(Action::Brush(BrushAction::Import(vec![file])));
    wait_import(&mut h);
    h.run();
    assert_eq!(st(&h).shown_brush_group(), Some(Group::Imported));
    assert_eq!(marked(&h, round), Some(false));
    assert_eq!(marked(&h, &krita.brushes[0].name), Some(false));
    // 丸へ戻せば丸のマスだけ
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Brush(yolu_app::m2::BrushOp::Tip(None))));
    h.run();
    h.run();
    assert_eq!(marked(&h, round), Some(true));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_flip_fields_are_on_for_a_hose_as_well_as_for_a_single_image() {
    let mut h = app(1600.0, 960.0, 128);
    h.state_mut().state.brushes.ui.detail.open = true;
    h.state_mut().state.brushes.ui.detail.category = Category::Shape;
    h.run();
    h.run();
    // 反転の欄（プロパティの欄のアルファのタブと詳細のウィンドウに 1 つずつ）。全部が同じ有効・無効のときだけ値を返す
    let flip_x = |h: &H| {
        let all: Vec<bool> = h
            .query_all_by_label("左右反転")
            .map(|n| !n.accesskit_node().is_disabled())
            .collect();
        assert!(!all.is_empty(), "反転の欄が無い");
        assert!(
            all.iter().all(|e| *e == all[0]),
            "欄ごとに食い違う: {all:?}"
        );
        all[0]
    };
    // 丸は反転する画像が無いので無効
    assert!(!flip_x(&h));
    let krita = yolu_app::brushes::store::krita();
    let single = krita
        .brushes
        .iter()
        .position(|b| b.brush.tip.image.is_some())
        .unwrap();
    let hose = krita
        .brushes
        .iter()
        .position(|b| b.brush.tip.images.len() > 1)
        .unwrap();
    for (index, what) in [(single, "1 枚の筆先"), (hose, "ホース")] {
        h.state_mut()
            .state
            .apply(Action::M2Ui(UiOp::Brush(yolu_app::m2::BrushOp::KritaTip(
                index,
            ))));
        h.run();
        h.run();
        assert!(flip_x(&h), "{what}で反転の欄が無効");
    }
    // 反転が入ったままホースを選んでも、欄から外せる（押すと反転が外れる）
    h.state_mut().state.m2.brush.tip.flip_x = true;
    h.run();
    h.get_all_by_label("左右反転").next().unwrap().click();
    h.run();
    assert!(!st(&h).m2.brush.tip.flip_x);
}

/// 矩形の中だけを撮って、正解の絵と比べる。
fn shot(h: &mut H, rect: Rect, name: &str) {
    // 直前に押した所のポインタが絵に残らないように
    h.event(Event::PointerGone);
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

/// ブラシのパネルの全体（タブの帯から、下のカラーのパネルの見出しの上まで）。
fn panel_rect(h: &H) -> Rect {
    let tab = h.state().tab_rects[&Tab::SubTools];
    let color = h.state().tab_rects[&Tab::Color];
    Rect::from_min_max(
        pos2(tab.left() - 2.0, tab.top()),
        pos2(tab.left() + 300.0, color.top()),
    )
}

fn imported_panel(lang: Lang, dir: &Path) -> H {
    let mut h = app(1600.0, 900.0, 128);
    language(&mut h, lang);
    with_store(&mut h, dir);
    let files = vec![
        write(dir, "colour_tip.gbr", &gbr_color("Colour tip")),
        write(dir, "chalk_set.gbr", &gbr_gray("Chalk")),
        write(dir, "old_set.abr", &abr_v1()),
    ];
    h.state_mut()
        .state
        .apply(Action::Brush(BrushAction::Import(files)));
    wait_import(&mut h);
    h.run();
    h
}

#[test]
fn snapshot_the_imported_group_with_the_mark_for_what_was_left_out() {
    let dir = temp_dir("snapshot");
    let mut h = imported_panel(Lang::Ja, &dir);
    let rect = panel_rect(&h);
    shot(&mut h, rect, "brushes_panel_imported");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn snapshot_the_imported_group_in_english() {
    let dir = temp_dir("snapshot-en");
    let mut h = imported_panel(Lang::En, &dir);
    let rect = panel_rect(&h);
    shot(&mut h, rect, "brushes_panel_imported_english");
    std::fs::remove_dir_all(dir).unwrap();
}

// ---------------- 「CLIP STUDIO から」のウィンドウ ----------------

/// 試験用の CELSYS の設定のフォルダ: 筆先の画像を持つ .sut・丸い筆先の .sut・読めないファイル。
fn celsys_places(dir: &Path) -> yolu_io::brushes::clipstudio::Places {
    use sut_files::*;
    let root = dir.join("AppData/Roaming/CELSYSUserData/CELSYS/CLIPStudioModule/SubTool/Pen");
    std::fs::create_dir_all(&root).unwrap();
    let tip = png_gray(2, 2, &[0, 255, 255, 0]);
    let stamp = SutBuilder::new()
        .material(Some("tip_a"), material_with_thumbnail(&tip))
        .brush(
            "Stamp",
            1,
            &[
                ("BrushSize", real(40.0)),
                ("BrushUsePatternImage", int(1)),
                (
                    "BrushPatternImageArray",
                    blob(refs(&[["x/tip_a.png", "cat/a", "tip_a"]])),
                ),
            ],
        )
        .build();
    let ink = SutBuilder::new()
        .brush(
            "Ink",
            1,
            &[
                ("BrushSize", real(24.0)),
                ("BrushHardness", int(30)),
                ("BrushUseIn", int(1)),
                ("BrushInLength", real(20.0)),
            ],
        )
        .build();
    std::fs::write(root.join("a_stamp.sut"), stamp).unwrap();
    std::fs::write(root.join("b_ink.sut"), ink).unwrap();
    std::fs::write(root.join("c_broken.sut"), b"not a database at all").unwrap();
    yolu_io::brushes::clipstudio::Places {
        appdata: Some(dir.join("AppData/Roaming")),
        profile: Some(dir.to_path_buf()),
        onedrive: None,
    }
}

fn csp_window(lang: Lang, dir: &Path) -> H {
    let mut h = app(1600.0, 900.0, 128);
    language(&mut h, lang);
    with_store(&mut h, dir);
    h.state_mut().state.brushes.csp.places = Some(celsys_places(dir));
    let button = lang.pick("CLIP STUDIO から取り込む", "Import from CLIP STUDIO");
    h.get_by_label(button).click();
    wait_csp(&mut h);
    h
}

/// 探す・覗く仕事が終わるまでフレームを回す。
fn wait_csp(h: &mut H) {
    let start = Instant::now();
    h.run();
    while st(h).brushes.csp.is_busy() && start.elapsed() < Duration::from_secs(60) {
        h.run();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!st(h).brushes.csp.is_busy(), "探す仕事が終わらない");
    h.run();
}

#[test]
fn the_clip_studio_button_opens_a_window_whose_rows_are_marked_and_imported() {
    let dir = temp_dir("csp-ui");
    let mut h = csp_window(Lang::Ja, &dir);
    assert!(st(&h).brushes.csp.open);
    // 見出し・状態の 1 行・名前（読めたもの）・ファイル名（読めなかったもの）・下の帯のボタン
    for label in [
        "Stamp",
        "Ink",
        "c_broken",
        "フォルダ…",
        "探し直す",
        "すべて選ぶ",
    ] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    assert!(drawn_texts(&h).iter().any(|t| t == "CLIP STUDIO から"));
    assert!(
        drawn_texts(&h).iter().any(|t| t == "サブツール 3 個"),
        "{:?}",
        drawn_texts(&h)
    );
    // 何も選んでいないので、取り込むは押せない
    assert!(h.get_by_label("取り込む").accesskit_node().is_disabled());
    assert_eq!(marked(&h, "Ink"), Some(false));
    h.get_by_label("Ink").click();
    h.run();
    assert_eq!(marked(&h, "Ink"), Some(true));
    assert_eq!(marked(&h, "Stamp"), Some(false));
    h.get_by_label("取り込む（1）").click();
    h.run();
    wait_import(&mut h);
    // ウィンドウが閉じ、選んだものだけが取り込まれている
    assert!(!st(&h).brushes.csp.open);
    assert!(!drawn_texts(&h).iter().any(|t| t == "CLIP STUDIO から"));
    assert!(h.query_by_label("Ink").is_some() && h.query_by_label("Stamp").is_none());
    assert_eq!(st(&h).shown_brush_group(), Some(Group::Imported));
    // 取り込んだブラシの行のツールチップに、写した項目（入り抜き）が並ぶ
    let row = rect_of(&h, "Ink", |r| in_panel(r) && r.width() > 200.0);
    move_to(&h, pos2(2.0, 2.0));
    h.run();
    hover_and_wait(&mut h, pos2(row.left() + 30.0, row.center().y));
    let tip = tooltip_text(&h, "写した項目").expect("ツールチップ");
    assert!(
        tip.contains("入り抜き") && tip.contains("CLIP STUDIO SUT"),
        "{tip}"
    );
    assert!(
        tip.contains("入り抜きの速さ・割合"),
        "近似した中身は表せなかった項目に: {tip}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_clip_studio_window_in_english_and_the_button_is_off_while_importing() {
    let dir = temp_dir("csp-ui-en");
    let mut h = csp_window(Lang::En, &dir);
    for label in [
        "Stamp",
        "Ink",
        "c_broken",
        "Folder…",
        "Rescan",
        "Select All",
    ] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    assert!(drawn_texts(&h).iter().any(|t| t == "From CLIP STUDIO"));
    assert!(
        drawn_texts(&h).iter().any(|t| t == "3 sub tools"),
        "{:?}",
        drawn_texts(&h)
    );
    assert!(
        drawn_texts(&h)
            .iter()
            .all(|t| !has_japanese(t) || t.contains("Ink") || t.contains("Stamp")),
        "英語の画面に日本語が混ざらない: {:?}",
        drawn_texts(&h)
    );
    h.get_by_label("Select All").click();
    h.run();
    assert!(h.query_by_label("Clear").is_some());
    assert!(h.query_by_label("Import (2)").is_some());
    // 閉じる
    h.get_by_label("Close").click();
    h.run();
    assert!(!st(&h).brushes.csp.open);
    // 取り込み中は入口を押せない
    h.state_mut().state.brushes.import.park_next = true;
    h.state_mut()
        .state
        .apply(Action::Brush(BrushAction::Import(vec![dir.join(
            "AppData/Roaming/CELSYSUserData/CELSYS/CLIPStudioModule/SubTool/Pen/b_ink.sut",
        )])));
    h.run();
    assert!(h
        .get_by_label("Import from CLIP STUDIO")
        .accesskit_node()
        .is_disabled());
    h.state_mut()
        .state
        .apply(Action::Brush(BrushAction::ImportCancel));
    wait_import(&mut h);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_clip_studio_window_says_why_nothing_was_found_in_both_languages() {
    for (lang, expect) in [
        (Lang::Ja, "サブツールのフォルダが見つかりません"),
        (Lang::En, "Sub tool folder not found"),
    ] {
        let dir = temp_dir("csp-ui-none");
        let mut h = app(1600.0, 900.0, 128);
        language(&mut h, lang);
        with_store(&mut h, &dir);
        h.state_mut().state.brushes.csp.places = Some(yolu_io::brushes::clipstudio::Places {
            appdata: Some(dir.join("AppData/Roaming")),
            profile: Some(dir.clone()),
            onedrive: None,
        });
        h.get_by_label(lang.pick("CLIP STUDIO から取り込む", "Import from CLIP STUDIO"))
            .click();
        wait_csp(&mut h);
        assert!(
            drawn_texts(&h).iter().any(|t| t == expect),
            "{lang:?}: {:?}",
            drawn_texts(&h)
        );
        // 一覧が無いので、取り込むと全部選ぶは押せない
        let import = lang.pick("取り込む", "Import");
        assert!(h.get_by_label(import).accesskit_node().is_disabled());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// ウィンドウの中だけを撮って、正解の絵と比べる。
fn shot_window(h: &mut H, name: &str) {
    let rect = yolu_app::windows::window_rect(&h.ctx, yolu_app::panels::brush_clipstudio::NAME)
        .expect("ウィンドウが開いている");
    h.event(Event::PointerGone);
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
fn snapshot_the_clip_studio_window() {
    let dir = temp_dir("csp-shot");
    let mut h = csp_window(Lang::Ja, &dir);
    h.get_by_label("Ink").click();
    h.run();
    shot_window(&mut h, "brush_clipstudio_window");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn snapshot_the_clip_studio_window_in_english() {
    let dir = temp_dir("csp-shot-en");
    let mut h = csp_window(Lang::En, &dir);
    h.get_by_label("Ink").click();
    h.run();
    shot_window(&mut h, "brush_clipstudio_window_english");
    std::fs::remove_dir_all(dir).unwrap();
}
