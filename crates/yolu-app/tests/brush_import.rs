//! ブラシのファイルの取り込み（ABR・GBR・PAT）・表せなかった項目・読めないファイルの理由・保存と読み戻し・取消・模様と Krita の筆先の選び・
//! 裏のスレッドの見本。画面を描かないので Wine でも回る（`headless_`）。試験のファイルは試験の中で組む（外のファイルは持ち込まない）。
mod brush_import_files;
/// 合成の .sut（SQLite）の組み立ては、読み手の試験（yolu-io）と同じものを使う。
#[allow(dead_code)]
#[path = "../../yolu-io/tests/brush_files/sut.rs"]
mod sut_files;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use brush_import_files::*;
use yolu_app::brushes::krita::KritaTips;
use yolu_app::brushes::sample::{SampleCache, SampleSpec, MAX_IN_FLIGHT};
use yolu_app::brushes::{store, BrushAction, BrushKey, Entry, Gap, Group, MAX_USER_BRUSHES};
use yolu_app::engine::Brush;
use yolu_app::lang::Lang;
use yolu_app::m2::{BrushOp, UiOp};
use yolu_app::m2_menu::{self, Popup};
use yolu_app::state::{Action, AppState, Tool};

fn temp_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/brush-import-tests")
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

/// 保存先を付けた状態。
fn state(dir: &Path) -> AppState {
    let mut s = AppState::new(64, 64);
    s.attach_brush_store(dir.join("brushes"));
    s
}

/// 別のスレッドの仕事が終わるまで受ける。
fn finish(s: &mut AppState) {
    let start = Instant::now();
    while s.is_brush_importing() && start.elapsed() < Duration::from_secs(60) {
        s.poll_brush_import();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!s.is_brush_importing(), "取り込みが終わらない");
}

fn import(s: &mut AppState, paths: &[PathBuf]) {
    s.apply(Action::Brush(BrushAction::Import(paths.to_vec())));
    finish(s);
}

fn imported(s: &AppState) -> Vec<Entry> {
    s.brushes
        .lib
        .in_group(Group::Imported)
        .into_iter()
        .cloned()
        .collect()
}

fn brush_files(dir: &Path) -> usize {
    std::fs::read_dir(dir.join("brushes"))
        .map(|d| {
            d.flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with(".ylbrush"))
                .count()
        })
        .unwrap_or(0)
}

fn image_files(dir: &Path) -> usize {
    std::fs::read_dir(dir.join("brushes/images"))
        .map(|d| d.count())
        .unwrap_or(0)
}

#[test]
fn headless_a_gbr_is_imported_in_the_background_saved_and_comes_back() {
    let dir = temp_dir("gbr");
    let file = write(&dir, "chalk_set.gbr", &gbr_gray("Chalk 筆"));
    let mut s = state(&dir);
    s.apply(Action::Brush(BrushAction::Import(vec![file])));
    // 読むのは別のスレッド。始めた直後は仕事が走っていて、状態の帯に今のファイルが出る
    assert!(s.is_brush_importing());
    assert!(s.message.contains("chalk_set.gbr"), "{}", s.message);
    assert!(imported(&s).is_empty(), "届くまで一覧には足さない");
    finish(&mut s);
    let list = imported(&s);
    assert_eq!(list.len(), 1);
    let e = &list[0];
    assert_eq!((e.name.as_str(), e.group), ("Chalk 筆", Group::Imported));
    // 取り込んだブラシに替わり、「取り込み」のタブが開く
    assert_eq!(s.brushes.lib.current(), e.key);
    assert_eq!(s.brushes.ui.group, Group::Imported);
    assert!(
        s.message.starts_with("ブラシを 1 個取り込みました"),
        "{}",
        s.message
    );
    // 筆先は 3×2 で、行は下から
    let tip = e.baseline.tip.image.as_ref().expect("画像の筆先");
    assert_eq!(
        (tip.width(), tip.height(), tip.at(0, 1), tip.at(2, 0)),
        (3, 2, 255, 128)
    );
    assert_eq!(
        s.m2.brush.tip.image.as_deref(),
        Some(&**tip),
        "今の設定にも入る"
    );
    let meta = e.import.as_ref().expect("出どころ");
    assert_eq!(meta.source, "GIMP GBR");
    assert!(meta.gaps.is_empty() && !meta.pattern);
    // ブラシのファイル 1 つと画像 1 枚。別の起動で、同じブラシ（筆先・出どころ・並び）が読み戻る
    assert_eq!((brush_files(&dir), image_files(&dir)), (1, 1));
    let back = state(&dir);
    assert!(
        back.brushes.problems.is_empty(),
        "{:?}",
        back.brushes.problems
    );
    let again = imported(&back);
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].baseline, e.baseline);
    assert_eq!(again[0].import, e.import);
    assert_eq!(again[0].name, "Chalk 筆");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_what_could_not_be_represented_is_kept_as_a_list_of_item_names() {
    let dir = temp_dir("gaps");
    let file = write(&dir, "colour.gbr", &gbr_color("Colour tip"));
    let mut s = state(&dir);
    import(&mut s, &[file]);
    let e = &imported(&s)[0];
    assert_eq!(e.import.as_ref().unwrap().gaps, [Gap::ColorTip]);
    // 項目の名前は言語ごと（文ではなく名詞句）
    assert_eq!(Gap::ColorTip.name(Lang::Ja), "色つきの筆先");
    assert_eq!(Gap::ColorTip.name(Lang::En), "Colored tip");
    // 保存して読み戻しても同じ一覧
    let back = state(&dir);
    assert_eq!(imported(&back)[0].import, e.import);
    // 項目の無いブラシは印なし。複製は出どころと項目を引き継ぐ（模様の印は引き継がない）
    s.apply(Action::Brush(BrushAction::Duplicate(e.key)));
    let copy = s
        .brushes
        .lib
        .entry(s.brushes.lib.current())
        .unwrap()
        .clone();
    assert_eq!(copy.import.as_ref().unwrap().gaps, [Gap::ColorTip]);
    assert_eq!(copy.group, Group::Imported);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_an_abr_gives_several_brushes_and_a_second_import_gets_new_names() {
    let dir = temp_dir("abr");
    let file = write(&dir, "old_set.abr", &abr_v1());
    let mut s = state(&dir);
    import(&mut s, std::slice::from_ref(&file));
    let names: Vec<String> = imported(&s).iter().map(|e| e.name.clone()).collect();
    assert_eq!(names, ["Old set 1", "Old set 2"]);
    // 計算で描くブラシ（画像なし）と画像のブラシ。画像は 1 枚だけ置く
    assert!(imported(&s)[0].baseline.tip.image.is_none());
    assert!(imported(&s)[1].baseline.tip.image.is_some());
    assert_eq!((brush_files(&dir), image_files(&dir)), (2, 1));
    // 同じファイルをもう一度: 名前が重ならず、画像は同じ内容なので増えない
    import(&mut s, &[file]);
    let names: Vec<String> = imported(&s).iter().map(|e| e.name.clone()).collect();
    assert_eq!(
        names,
        ["Old set 1", "Old set 2", "Old set 1 2", "Old set 2 2"]
    );
    assert_eq!((brush_files(&dir), image_files(&dir)), (4, 1));
    assert!(
        s.message.starts_with("ブラシを 2 個取り込みました"),
        "{}",
        s.message
    );
    // 読み戻した並びは取り込んだ順
    let back = state(&dir);
    let names: Vec<String> = imported(&back).iter().map(|e| e.name.clone()).collect();
    assert_eq!(
        names,
        ["Old set 1", "Old set 2", "Old set 1 2", "Old set 2 2"]
    );
    // 1 つ消すと、そのブラシのファイルだけが消え、同じ画像を使うほかのブラシが残るあいだ画像は残る
    let key = imported(&s)[1].key;
    s.apply(Action::Brush(BrushAction::Delete(key)));
    assert_eq!((brush_files(&dir), image_files(&dir)), (3, 1));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_a_sut_is_imported_with_its_tip_pressure_and_the_item_names_it_could_not_represent() {
    use sut_files::*;
    let dir = temp_dir("sut");
    let tip = png_gray(2, 2, &[0, 255, 255, 0]);
    let file = SutBuilder::new()
        .material(Some("tip_a"), material_with_thumbnail(&tip))
        .brush(
            "Soft ink",
            1,
            &[
                ("BrushSize", real(40.0)),
                ("BrushUsePatternImage", int(1)),
                (
                    "BrushPatternImageArray",
                    blob(refs(&[["x/tip_a.png", "cat/a", "tip_a"]])),
                ),
                (
                    "BrushSizeEffector",
                    blob(effector(44, 0x10 | 0x20, 20, &[&[(0.0, 0.0), (1.0, 1.0)]])),
                ),
                ("BrushUseSpray", int(1)),
            ],
        )
        .build();
    let path = write(&dir, "soft_ink.sut", &file);
    let mut s = state(&dir);
    import(&mut s, std::slice::from_ref(&path));
    let list = imported(&s);
    assert_eq!(list.len(), 1);
    let e = &list[0];
    assert_eq!((e.name.as_str(), e.group), ("Soft ink", Group::Imported));
    assert!(
        s.message.starts_with("ブラシを 1 個取り込みました"),
        "{}",
        s.message
    );
    // 筆先の画像（暗い画素が塗る。行は下から）と、筆圧の最小値・半径
    let tip = e.baseline.tip.image.as_ref().expect("画像の筆先");
    assert_eq!(tip.alpha(), [0, 255, 255, 0]);
    assert_eq!(e.baseline.base.radius, 20.0);
    assert!(e.baseline.base.pressure_size);
    assert!(
        (e.baseline.pressure.size.min() - 0.2).abs() < 1e-6,
        "保存の精度（f32）に丸まる"
    );
    // 表せなかった項目は名前の一覧（並びは項目の順）。傾き・吹き付け・プレビューの解像度
    let meta = e.import.as_ref().expect("出どころ");
    assert_eq!(meta.source, "CLIP STUDIO SUT");
    assert_eq!(meta.gaps, [Gap::Controls, Gap::ImageResolution, Gap::Spray]);
    let names = |lang| -> Vec<&'static str> { meta.gaps.iter().map(|g| g.name(lang)).collect() };
    assert_eq!(
        names(Lang::Ja),
        ["コントロール", "画像の解像度", "吹き付け"]
    );
    assert_eq!(names(Lang::En), ["Controls", "Image resolution", "Spray"]);
    // 保存して読み戻しても同じ（筆先の画像は置き場に 1 枚）
    assert_eq!((brush_files(&dir), image_files(&dir)), (1, 1));
    let back = state(&dir);
    assert!(
        back.brushes.problems.is_empty(),
        "{:?}",
        back.brushes.problems
    );
    assert_eq!(imported(&back)[0].baseline, e.baseline);
    assert_eq!(imported(&back)[0].import, e.import);
    // 同じファイルをもう一度: 名前は重ならず、筆先の画像は同じ内容なので増えない
    import(&mut s, &[path]);
    let names: Vec<String> = imported(&s).iter().map(|e| e.name.clone()).collect();
    assert_eq!(names, ["Soft ink", "Soft ink 2"]);
    assert_eq!((brush_files(&dir), image_files(&dir)), (2, 1));
    // SQLite でないファイルは理由を両方の言語で出して、何も足さない
    let broken = write(&dir, "broken.sut", b"not a database at all");
    for (lang, expect) in [
        (Lang::Ja, "SQLite のデータベース）として読めません"),
        (Lang::En, "Not a readable CLIP STUDIO brush"),
    ] {
        s.lang = lang;
        s.message.clear();
        import(&mut s, std::slice::from_ref(&broken));
        assert!(s.message.contains(expect), "{lang:?}: {}", s.message);
    }
    assert_eq!(imported(&s).len(), 2);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_files_that_cannot_be_read_say_why_in_both_languages_and_add_nothing() {
    let dir = temp_dir("refused");
    let garbage = write(&dir, "broken.gbr", b"this is not a brush at all, just text");
    let kpp = write(&dir, "preset.kpp", b"x");
    let text = write(&dir, "notes.txt", b"x");
    let missing = dir.join("missing.abr");
    let big = dir.join("huge.abr");
    // 疎なファイル（ディスクは使わない）。上限を超える大きさは、読む前に断る
    std::fs::File::create(&big)
        .unwrap()
        .set_len(yolu_io::brushes::MAX_FILE_BYTES + 1)
        .unwrap();
    let mut s = state(&dir);
    for (path, ja, en) in [
        (
            &garbage,
            "取り込めません: broken.gbr — ",
            "Cannot import: broken.gbr — ",
        ),
        (
            &kpp,
            "Krita のブラシプリセット（.kpp）は未対応です",
            "Krita brush presets (.kpp) are not supported",
        ),
        (&text, "'txt'", "'txt'"),
        (&big, "ファイルが大きすぎます", "The file is too large"),
        (
            &missing,
            "ファイルまたはフォルダーがありません",
            "File or folder not found",
        ),
    ] {
        for (lang, expect) in [(Lang::Ja, ja), (Lang::En, en)] {
            s.lang = lang;
            s.message.clear();
            import(&mut s, std::slice::from_ref(path));
            assert!(
                s.message.contains(expect),
                "{lang:?} {path:?}: {}",
                s.message
            );
            assert!(
                !s.message.starts_with("ブラシを") && !s.message.starts_with("Imported"),
                "{}",
                s.message
            );
        }
    }
    assert!(imported(&s).is_empty());
    assert_eq!((brush_files(&dir), image_files(&dir)), (0, 0));
    // 読めたファイルと読めないファイルが混ざれば、読めたものは取り込み、読めなかったファイルを知らせる
    s.lang = Lang::Ja;
    let good = write(&dir, "good.gbr", &gbr_gray("Good"));
    import(&mut s, &[garbage.clone(), good]);
    assert_eq!(imported(&s).len(), 1);
    assert!(
        s.message.contains("broken.gbr は読めません"),
        "{}",
        s.message
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_cancelling_stops_the_import_and_leaves_no_files() {
    let dir = temp_dir("cancel");
    let file = write(&dir, "chalk.gbr", &gbr_gray("Chalk"));
    let mut s = state(&dir);
    // 仕事を取消が来るまで止めておく（取消が効いたことを、仕事の速さに頼らず確かめる）
    s.brushes.import.park_next = true;
    s.apply(Action::Brush(BrushAction::Import(vec![file.clone()])));
    assert!(s.is_brush_importing());
    // 取り込み中の二重の頼みは断る
    s.apply(Action::Brush(BrushAction::Import(vec![file])));
    assert!(s.message.contains("取り込み中"), "{}", s.message);
    s.apply(Action::Brush(BrushAction::ImportCancel));
    assert!(s.message.contains("取り消"), "{}", s.message);
    assert_eq!(s.brushes.import.progress().map(|p| p.canceling), Some(true));
    finish(&mut s);
    assert!(imported(&s).is_empty());
    assert_eq!((brush_files(&dir), image_files(&dir)), (0, 0));
    assert!(s.message.contains("やめ"), "{}", s.message);
    // 取り消したあとも、また取り込める
    let file = write(&dir, "again.gbr", &gbr_gray("Again"));
    import(&mut s, &[file]);
    assert_eq!(imported(&s).len(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

/// ファイルが置かれた数を、仕事がまだ走っている間に数える（止めた仕事の途中を見るため）。
fn wait_for_brush_files(s: &mut AppState, dir: &Path, count: usize) {
    let start = Instant::now();
    while brush_files(dir) < count && start.elapsed() < Duration::from_secs(60) {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(brush_files(dir), count, "置くのが終わらない");
    assert!(s.is_brush_importing(), "止めた仕事はまだ走っている");
}

#[test]
fn headless_cancelling_in_the_middle_of_a_file_keeps_the_brushes_already_placed() {
    let dir = temp_dir("cancel-mid-file");
    // 1 つのファイルに 2 つのブラシ（1 つ目は画像なし・2 つ目は画像）。1 つ目を置いたところで止めて取り消す
    let file = write(&dir, "old_set.abr", &abr_v1());
    let mut s = state(&dir);
    s.brushes.import.park_after_brushes = Some(1);
    s.apply(Action::Brush(BrushAction::Import(vec![file])));
    wait_for_brush_files(&mut s, &dir, 1);
    s.apply(Action::Brush(BrushAction::ImportCancel));
    finish(&mut s);
    // 置いた 1 つ目だけが一覧にもファイルにもあり、2 つ目の画像は置かれていない
    let list = imported(&s);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "Old set 1");
    assert_eq!((brush_files(&dir), image_files(&dir)), (1, 0));
    assert!(
        s.message
            .contains("取り込みをやめました（1 個は取り込み済み）"),
        "{}",
        s.message
    );
    // 読み戻しても同じ（途中で止めても、一覧に出たブラシは全部ファイルから読める）
    let back = state(&dir);
    assert!(
        back.brushes.problems.is_empty(),
        "{:?}",
        back.brushes.problems
    );
    assert_eq!(imported(&back).len(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_cancelling_between_files_keeps_the_files_already_imported_and_skips_the_rest() {
    let dir = temp_dir("cancel-between");
    let first = write(&dir, "first.gbr", &gbr_gray("First"));
    let second = write(&dir, "second.gbr", &gbr_gray("Second"));
    let mut s = state(&dir);
    s.brushes.import.park_after_brushes = Some(1);
    s.apply(Action::Brush(BrushAction::Import(vec![first, second])));
    wait_for_brush_files(&mut s, &dir, 1);
    s.apply(Action::Brush(BrushAction::ImportCancel));
    finish(&mut s);
    let names: Vec<String> = imported(&s).iter().map(|e| e.name.clone()).collect();
    assert_eq!(names, ["First"]);
    assert_eq!((brush_files(&dir), image_files(&dir)), (1, 1));
    // 取り込んだ分に替わっていて、止めたことと取り込み済みの数を知らせる
    assert_eq!(s.brushes.lib.current(), imported(&s)[0].key);
    assert!(
        s.message
            .contains("取り込みをやめました（1 個は取り込み済み）"),
        "{}",
        s.message
    );
    // 止める仕掛けは次の仕事には残らない
    let third = write(&dir, "third.gbr", &gbr_gray("Third"));
    import(&mut s, &[third]);
    assert_eq!(imported(&s).len(), 2);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_a_failed_save_keeps_what_was_placed_stops_the_rest_and_says_so_in_both_languages() {
    for (lang, expect) in [(Lang::Ja, "保存できません"), (Lang::En, "Cannot save")] {
        let dir = temp_dir("save-fails");
        let file = write(&dir, "old_set.abr", &abr_v1());
        let later = write(&dir, "later.gbr", &gbr_gray("Later"));
        let mut s = state(&dir);
        s.lang = lang;
        // 画像を置く場所が普通のファイルで塞がれている（権限に頼らないので Windows でも同じ）。
        // 1 つ目（画像なし）は置け、2 つ目（画像）は置けない
        std::fs::create_dir_all(dir.join("brushes")).unwrap();
        std::fs::write(dir.join("brushes/images"), b"in the way").unwrap();
        import(&mut s, &[file, later]);
        let list = imported(&s);
        assert_eq!(list.len(), 1, "{lang:?}: 置けた分だけ一覧に出る");
        assert_eq!(list[0].name, "Old set 1");
        assert_eq!(brush_files(&dir), 1);
        assert!(s.message.contains(expect), "{lang:?}: {}", s.message);
        // 置けなかった理由で止まり、あとのファイルは読まない
        assert!(imported(&s).iter().all(|e| e.name != "Later"));
        // 一覧に出たブラシは、起動し直してもファイルから読める
        std::fs::remove_file(dir.join("brushes/images")).unwrap();
        let back = state(&dir);
        assert!(
            back.brushes.problems.is_empty(),
            "{:?}",
            back.brushes.problems
        );
        assert_eq!(imported(&back).len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn headless_finishing_an_import_does_not_take_the_tool_from_a_selection_in_progress() {
    let dir = temp_dir("keeps-tool");
    let mut s = state(&dir);
    // 多角形の選択の途中。取り込みが終わっても、道具も途中の形もそのまま。ブラシだけが取り込んだものに替わる
    s.tool = Tool::Polygon;
    s.sel.polygon.push((3.0, 4.0));
    s.sel.polygon.push((10.0, 4.0));
    let before = s.brushes.lib.current();
    let file = write(&dir, "chalk.gbr", &gbr_gray("Chalk"));
    import(&mut s, &[file]);
    assert_eq!(s.tool, Tool::Polygon);
    assert_eq!(s.sel.polygon, vec![(3.0, 4.0), (10.0, 4.0)]);
    let key = imported(&s)[0].key;
    assert_ne!(before, key);
    assert_eq!(s.brushes.lib.current(), key);
    assert_eq!(s.brushes.ui.group, Group::Imported);
    assert!(s.m2.brush.tip.image.is_some(), "今の設定も取り込んだ筆先");
    // バケツなどほかの道具でも同じ
    s.tool = Tool::Fill;
    let file = write(&dir, "second.gbr", &gbr_gray("Second"));
    import(&mut s, &[file]);
    assert_eq!(s.tool, Tool::Fill);
    assert_eq!(s.brushes.lib.current(), imported(&s)[1].key);
    // ブラシ・消しゴムのときは従来どおり、ブラシに合わせて道具もブラシへ
    s.tool = Tool::Eraser;
    let file = write(&dir, "third.gbr", &gbr_gray("Third"));
    import(&mut s, &[file]);
    assert_eq!(s.tool, Tool::Brush);
    assert_eq!(s.brushes.lib.current(), imported(&s)[2].key);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_while_importing_add_duplicate_delete_and_register_are_refused_and_the_shared_image_stays(
) {
    let dir = temp_dir("busy-refuse");
    let file = write(&dir, "old_set.abr", &abr_v1());
    let mut s = state(&dir);
    import(&mut s, std::slice::from_ref(&file));
    assert_eq!((brush_files(&dir), image_files(&dir)), (2, 1));
    let old = imported(&s)[1].key;
    let users = s.brushes.lib.user_count();
    // 同じ ABR を取り込み直している最中（同じ画像を置く）。前に取り込んだ、その画像を使うブラシを消そうとしても断る
    s.brushes.import.park_next = true;
    s.apply(Action::Brush(BrushAction::Import(vec![file])));
    assert!(s.is_brush_importing());
    for action in [
        BrushAction::Delete(old),
        BrushAction::Add,
        BrushAction::Duplicate(old),
        BrushAction::Register(old),
    ] {
        s.message.clear();
        s.apply(Action::Brush(action.clone()));
        assert!(
            s.message.contains("取り込み中"),
            "{action:?}: {}",
            s.message
        );
        assert_eq!(s.brushes.lib.user_count(), users, "{action:?}");
        assert!(s.brushes.lib.entry(old).is_some(), "{action:?}");
    }
    assert_eq!((brush_files(&dir), image_files(&dir)), (2, 1));
    // 英語でも同じ
    s.lang = Lang::En;
    s.apply(Action::Brush(BrushAction::Delete(old)));
    assert!(s.message.contains("Importing brushes"), "{}", s.message);
    // 取消のあと（仕事が無くなれば）、また消せる
    s.apply(Action::Brush(BrushAction::ImportCancel));
    finish(&mut s);
    s.apply(Action::Brush(BrushAction::Delete(old)));
    assert!(s.brushes.lib.entry(old).is_none());
    assert_eq!(brush_files(&dir), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_a_brush_order_that_cannot_be_saved_is_reported_after_the_import() {
    for (lang, expect) in [
        (Lang::Ja, "ブラシの並びを保存できません"),
        (Lang::En, "Cannot save the brush order"),
    ] {
        let dir = temp_dir("order-fails");
        let file = write(&dir, "chalk.gbr", &gbr_gray("Chalk"));
        let mut s = state(&dir);
        s.lang = lang;
        // 並びのファイルの場所をフォルダで塞ぐ（置換できない）。ブラシのファイルは置ける
        std::fs::create_dir_all(dir.join("brushes/order.conf")).unwrap();
        import(&mut s, &[file]);
        assert_eq!(imported(&s).len(), 1);
        assert_eq!(brush_files(&dir), 1);
        assert!(s.message.contains(expect), "{lang:?}: {}", s.message);
        // 取り込んだことの知らせも残っている
        assert!(
            s.message.contains("取り込みました") || s.message.contains("Imported"),
            "{}",
            s.message
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn headless_the_number_of_brushes_stops_the_import_and_says_so() {
    let dir = temp_dir("cap");
    let file = write(&dir, "old.abr", &abr_v1());
    let mut s = AppState::new(64, 64);
    for _ in 0..MAX_USER_BRUSHES - 1 {
        s.apply(Action::Brush(BrushAction::Add));
    }
    import(&mut s, std::slice::from_ref(&file));
    assert_eq!(imported(&s).len(), 1, "あと 1 個だけ入る");
    assert!(
        s.message.contains(&MAX_USER_BRUSHES.to_string()),
        "{}",
        s.message
    );
    // 満杯なら始めない
    s.message.clear();
    s.apply(Action::Brush(BrushAction::Import(vec![file])));
    assert!(!s.is_brush_importing());
    assert!(
        s.message.contains(&MAX_USER_BRUSHES.to_string()),
        "{}",
        s.message
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_a_pat_adds_its_patterns_to_the_texture_choices_and_they_survive_a_restart() {
    let dir = temp_dir("pat");
    let file = write(&dir, "papers.pat", &pat_file(&["Paper A", "Paper B"]));
    let mut s = state(&dir);
    // 取り込む前は、組み込みの質感だけ
    let labels = |s: &AppState| -> Vec<String> {
        m2_menu::entries(s, Popup::Texture)
            .into_iter()
            .filter_map(|e| match e {
                yolu_app::ui::menu::Entry::Item { label, .. } => Some(label),
                _ => None,
            })
            .collect()
    };
    let builtin = labels(&s);
    assert_eq!(builtin.len(), 1 + yolu_core::brush::BUILTIN_TIPS.len());
    import(&mut s, &[file]);
    assert_eq!(imported(&s).len(), 2);
    assert!(imported(&s)
        .iter()
        .all(|e| e.import.as_ref().unwrap().pattern));
    let with = labels(&s);
    assert_eq!(&with[..builtin.len()], &builtin[..]);
    assert_eq!(&with[builtin.len()..], ["Paper A", "Paper B"]);
    // 選ぶと、その模様が紙の質感になる（今の質感の深さ・スケール・合わせ方は残る）
    let (a, b) = (imported(&s)[0].clone(), imported(&s)[1].clone());
    let pattern_b = b.baseline.texture.as_ref().unwrap().image.clone();
    s.m2.brush.texture.as_mut().unwrap().depth = 0.4;
    s.apply(Action::M2Ui(UiOp::Brush(BrushOp::PatternTexture(b.key))));
    let texture = s.m2.brush.texture.as_ref().unwrap();
    assert_eq!(*texture.image, *pattern_b);
    assert_eq!(texture.depth, 0.4);
    // いま選んでいる模様に印が付く
    let marked = m2_menu::entries(&s, Popup::Texture)
        .into_iter()
        .filter_map(|e| match e {
            yolu_app::ui::menu::Entry::Item {
                label,
                check: yolu_app::ui::menu::Check::Radio,
                ..
            } => Some(label),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(marked, ["Paper B"]);
    // 組み込みの質感へ替えたら、模様の印は外れる
    s.apply(Action::M2Ui(UiOp::Brush(BrushOp::Texture(Some("grain")))));
    assert!(m2_menu::entries(&s, Popup::Texture)
        .iter()
        .all(|e| !matches!(
            e,
            yolu_app::ui::menu::Entry::Item { label, check, .. }
                if label.starts_with("Paper") && *check == yolu_app::ui::menu::Check::Radio
        )));
    // 保存して読み戻しても、模様の一覧は同じ
    let back = state(&dir);
    assert_eq!(
        m2_menu::patterns(&back)
            .into_iter()
            .map(|(_, name, _)| name)
            .collect::<Vec<_>>(),
        ["Paper A", "Paper B"]
    );
    // 模様のブラシを消すと、その模様は選びから消える
    s.apply(Action::Brush(BrushAction::Delete(a.key)));
    assert_eq!(&labels(&s)[builtin.len()..], ["Paper B"]);
    assert_eq!(image_files(&dir), 1, "A の画像のファイルも消える");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_krita_tips_can_be_searched_by_name_and_picked_and_picking_a_builtin_clears_a_hose() {
    let dir = temp_dir("krita");
    let mut s = state(&dir);
    let ctx = egui::Context::default();
    // 既定（試験）は、初めて出すときにその場で読む
    s.brushes.krita.poll(&ctx);
    assert!(s.brushes.krita.is_ready());
    let all = s.brushes.krita.matches();
    assert_eq!(all.len(), store::krita().brushes.len());
    assert_eq!(all.len(), 76);
    // 名前（大文字小文字を問わない）で絞る
    s.brushes.krita.search = "BRISTLE".into();
    let found = s.brushes.krita.matches();
    assert!(!found.is_empty() && found.len() < all.len());
    for i in &found {
        let name = s.brushes.krita.name(*i).unwrap().to_lowercase();
        let id = store::krita().brushes[*i].id.to_lowercase();
        assert!(
            name.contains("bristle") || id.contains("bristle"),
            "{name} {id}"
        );
    }
    s.brushes.krita.search = "no such tip anywhere".into();
    assert!(s.brushes.krita.matches().is_empty());
    s.brushes.krita.search.clear();
    // 1 枚の筆先とホース
    let krita = store::krita();
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
    s.apply(Action::M2Ui(UiOp::Brush(BrushOp::KritaTip(single))));
    assert_eq!(s.m2.brush.tip.image, krita.brushes[single].brush.tip.image);
    assert!(s.m2.brush.tip.images.is_empty());
    assert_eq!(
        yolu_app::brushes::krita::current_index(&s.m2.brush.tip),
        Some(single)
    );
    s.apply(Action::M2Ui(UiOp::Brush(BrushOp::KritaTip(hose))));
    assert!(s.m2.brush.tip.image.is_none());
    assert_eq!(s.m2.brush.tip.images, krita.brushes[hose].brush.tip.images);
    assert_eq!(
        s.m2.brush.tip.selection,
        krita.brushes[hose].brush.tip.selection
    );
    assert_eq!(
        yolu_app::brushes::krita::current_index(&s.m2.brush.tip),
        Some(hose)
    );
    // 組み込みの画像や丸を選べば、ホースは外れる（残ると 1 枚の選びが効かない）
    s.apply(Action::M2Ui(UiOp::Brush(BrushOp::Tip(Some("grain")))));
    assert!(s.m2.brush.tip.images.is_empty());
    assert_eq!(s.m2.brush.tip.image, yolu_core::builtin_tip("grain"));
    // Krita の筆先のブラシは、画像のファイルを置かずに ID で保存し、同じ筆先として読み戻る
    s.apply(Action::M2Ui(UiOp::Brush(BrushOp::KritaTip(hose))));
    s.apply(Action::Brush(BrushAction::Add));
    let key = s.brushes.lib.current();
    let saved = std::fs::read_to_string(s.brushes.store.as_ref().unwrap().path_of(match key {
        BrushKey::User(id) => id,
        _ => panic!("利用者のブラシ"),
    }))
    .unwrap();
    assert!(saved.contains(&krita.brushes[hose].id), "{saved}");
    assert_eq!(image_files(&dir), 0);
    let back = state(&dir);
    assert!(
        back.brushes.problems.is_empty(),
        "{:?}",
        back.brushes.problems
    );
    assert_eq!(
        back.brushes.lib.entry(key).unwrap().baseline.tip.images,
        krita.brushes[hose].brush.tip.images
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_krita_tips_load_in_the_background_and_samples_are_drawn_off_the_screen_thread() {
    let ctx = egui::Context::default();
    // Krita の格子: 始めた直後はまだ（別のスレッド）。できたら全部そろう
    let mut tips = KritaTips::default();
    tips.load_in_background();
    assert!(!tips.poll(&ctx), "始めた直後は読み込み中");
    tips.wait_ready(&ctx);
    assert!(tips.is_ready());
    assert_eq!(tips.matches().len(), 76);
    assert!(tips.texture(&ctx, 0).is_some());
    // 見本: 頼んでも画面のスレッドでは描かず、できた絵が次のフレームで届く。描いている最中の札は重ねて頼まない
    let mut cache = SampleCache::default();
    cache.render_in_background(&ctx);
    let (brush, spec) = (Brush::default(), SampleSpec::row(false));
    cache.begin_frame(1);
    assert!(cache.request(&brush, spec).is_none());
    assert!(cache.request(&brush, spec).is_none());
    assert_eq!(cache.in_flight(), 1);
    assert_eq!(cache.stats.renders, 1);
    assert!(
        !cache.needs_next_frame(),
        "描き終わりは裏のスレッドが知らせる"
    );
    let start = Instant::now();
    let mut frame = 2;
    let key = loop {
        cache.begin_frame(frame);
        frame += 1;
        if let Some(key) = cache.request(&brush, spec) {
            break key;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "見本ができない");
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(cache.image(key).unwrap().width, spec.width);
    assert_eq!(cache.in_flight(), 0);
    // 同時に頼む数には上限がある。あふれた分は次のフレームで頼み直す
    cache.begin_frame(frame);
    let many: Vec<Brush> = (0..MAX_IN_FLIGHT + 4)
        .map(|i| {
            let mut b = Brush::default();
            b.base.radius = 3.0 + i as f64;
            b
        })
        .collect();
    for b in &many {
        assert!(cache.request(b, spec).is_none());
    }
    assert_eq!(cache.in_flight(), MAX_IN_FLIGHT);
    assert!(cache.needs_next_frame());
    let start = Instant::now();
    loop {
        frame += 1;
        cache.begin_frame(frame);
        let ready = many
            .iter()
            .filter(|b| cache.request(b, spec).is_some())
            .count();
        if ready == many.len() {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "見本が全部はそろわない"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
