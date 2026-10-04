//! ブラシの取り込みの画面（egui_kittest）: 一覧の下のボタン・取り込んだブラシのタブと行の印・ツールチップの項目の一覧・ファイルのドロップ・
//! 詳細の窓の Krita の格子と模様の選び・日英。試験のファイルは試験の中で組む（外のファイルは持ち込まない）。
mod brush_import_files;
mod common;

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

fn has_japanese(text: &str) -> bool {
    text.chars()
        .any(|c| matches!(c, '\u{3000}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}' | '\u{ff00}'..='\u{ffef}'))
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

/// 名前のマスの「選んでいる」印（アクセシビリティの木の切り替え）。同じ名前の部品（プロパティの欄と詳細の窓の同じ格子）が
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
    assert_eq!(st(&h).brushes.ui.group, Group::Imported);
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
    // 取り込んだブラシを全部消すと、タブも消えて、ほかのグループへ戻る
    for key in st(&h)
        .brushes
        .lib
        .in_group(Group::Imported)
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
    assert_eq!(st(&h).brushes.ui.group, Group::Pen);
    assert!(h.query_by_label("Imported").is_none());
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
    // ABR は窓のどこに落としても取り込む。PNG はほかの用途と区別がつかないので、一覧の外では取り込まない
    drop_files(&mut h, outside, &[&png]);
    assert!(!st(&h).is_brush_importing());
    drop_files(&mut h, outside, &[&abr]);
    assert!(st(&h).is_brush_importing());
    wait_import(&mut h);
    assert_eq!(st(&h).brushes.lib.in_group(Group::Imported).len(), 2);
    // 一覧の上の PNG は取り込もうとする（PNG として読めなければ、その理由を状態の帯に出す）
    drop_files(&mut h, inside, &[&png]);
    assert!(st(&h).is_brush_importing());
    wait_import(&mut h);
    assert!(st(&h).message.contains("tip.png"), "{}", st(&h).message);
    assert_eq!(st(&h).brushes.lib.in_group(Group::Imported).len(), 2);
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
    // 棚やチャンネルのタブを開いている間は、一覧を描かないので、前の位置へ落としても取り込まない
    for tab in [Tab::Assets, Tab::Channels] {
        click_tab(&mut h, tab);
        h.run();
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
    assert_eq!(st(&h).brushes.ui.group, Group::Imported);
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
    // 反転の欄（プロパティの欄のアルファのタブと詳細の窓に 1 つずつ）。全部が同じ有効・無効のときだけ値を返す
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
