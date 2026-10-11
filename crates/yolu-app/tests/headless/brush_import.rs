//! ブラシのファイルの取り込み（ABR・GBR・PAT）・表せなかった項目・読めないファイルの理由・保存と読み戻し・取消・模様と Krita の筆先の選び・
//! 裏のスレッドの見本。画面を描かないので Wine でも回る（`headless_`）。試験のファイルは試験の中で組む（外のファイルは持ち込まない）。
#[path = "../brush_import_files/mod.rs"]
mod brush_import_files;
/// 合成の .sut（SQLite）の組み立ては、読み手の試験（yolu-io）と同じものを使う。
#[allow(dead_code)]
#[path = "../../../yolu-io/tests/brush_files/sut.rs"]
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
use yolu_app::toolset::{ToolsetAction, MAX_GROUP_BRUSHES};

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
    s.brush_entries_in(Group::Imported)
        .into_iter()
        .cloned()
        .collect()
}

/// 今の設定の新しいブラシを `n` 個足す（1 つのグループは 512 個までなので、いっぱいになったら今のツールにグループを足す）。
fn add_many(s: &mut AppState, n: usize) {
    for _ in 0..n {
        let slot = s.toolset.set.active().unwrap();
        let full = s
            .toolset
            .set
            .shown_group(slot)
            .and_then(|g| s.toolset.set.group(g))
            .is_some_and(|(_, g)| g.brushes.len() >= MAX_GROUP_BRUSHES);
        if full {
            s.apply(Action::Tools(ToolsetAction::AddGroup(slot)));
        }
        s.apply(Action::Brush(BrushAction::Add));
    }
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
    assert_eq!(s.shown_brush_group(), Some(Group::Imported));
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
fn headless_imported_brushes_go_to_the_current_brush_tool_never_into_an_eraser_tool() {
    let dir = temp_dir("import-tool");
    let file = write(&dir, "chalk_set.gbr", &gbr_gray("Chalk"));
    let mut s = state(&dir);
    let slot = |s: &AppState, tool: Tool| s.toolset.set.first_of(tool).unwrap();
    let group_of = |s: &AppState, key: BrushKey| {
        let id = s.toolset.set.group_of(key).expect("並びにある");
        s.toolset
            .set
            .group(id)
            .map(|(slot, g)| (slot.id, g.clone()))
            .unwrap()
    };
    // 取り込みのタブを消しゴムのツールへ動かしておく
    import(&mut s, std::slice::from_ref(&file));
    let first = s.brushes.lib.current();
    let (_, tab) = group_of(&s, first);
    assert_eq!(tab.builtin, Some(Group::Imported));
    let eraser = slot(&s, Tool::Eraser);
    s.apply(Action::Tools(ToolsetAction::MoveGroup {
        group: tab.id,
        to: eraser,
        before: None,
    }));
    assert_eq!(s.toolset.set.slot_of(first), Some(eraser));
    // ブラシのツールで取り込むと、ブラシのツールの最後に取り込みのグループができる
    let brush = slot(&s, Tool::Brush);
    s.apply(Action::Tools(ToolsetAction::Select(brush)));
    import(&mut s, std::slice::from_ref(&file));
    let second = s.brushes.lib.current();
    assert_ne!(second, first);
    let (second_slot, second_tab) = group_of(&s, second);
    assert_eq!(second_slot, brush, "消しゴムのツールの中には置かない");
    assert_eq!(second_tab.builtin, Some(Group::Imported));
    assert_eq!(
        s.toolset.set.slot(brush).unwrap().groups.last().unwrap().id,
        second_tab.id,
        "ツールの最後"
    );
    assert_eq!(
        s.toolset.set.slot_of(first),
        Some(eraser),
        "前に取り込んだ物は動かさない"
    );
    // 2 つ目のブラシのツールで取り込むと、そのツールの中に作る。元のツールの取り込みのグループには入らない
    s.apply(Action::Tools(ToolsetAction::Add {
        tool: Tool::Brush,
        after: None,
    }));
    let other = s.toolset.set.slots().last().unwrap().id;
    s.apply(Action::Tools(ToolsetAction::Select(other)));
    import(&mut s, std::slice::from_ref(&file));
    let third = s.brushes.lib.current();
    let (third_slot, third_tab) = group_of(&s, third);
    assert_eq!(third_slot, other);
    assert_eq!(third_tab.builtin, Some(Group::Imported));
    assert_eq!(
        group_of(&s, second).1.brushes,
        [second],
        "元のツールのグループのまま"
    );
    // 元のブラシのツールへ戻して取り込むと、そのツールの取り込みのグループの後ろへ入る
    s.apply(Action::Tools(ToolsetAction::Select(brush)));
    import(&mut s, &[file]);
    let fourth = s.brushes.lib.current();
    let (fourth_slot, fourth_tab) = group_of(&s, fourth);
    assert_eq!((fourth_slot, fourth_tab.id), (brush, second_tab.id));
    assert_eq!(fourth_tab.brushes, [second, fourth]);
    // 並びは保存され、読み戻せる
    let back = state(&dir);
    assert!(back.toolset.problem.is_none(), "{:?}", back.toolset.problem);
    assert_eq!(back.toolset.set, s.toolset.set);
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
    // 並びから外しても、ファイルは残る（「＋」のウィンドウから戻せる）
    let key = imported(&s)[1].key;
    s.apply(Action::Brush(BrushAction::Delete(key)));
    assert!(!s.toolset.set.contains(key));
    assert_eq!((brush_files(&dir), image_files(&dir)), (4, 1));
    // ファイルを 1 つ消すと、そのブラシのファイルだけが消え、同じ画像を使うほかのブラシが残るあいだ画像は残る
    s.apply(Action::Brush(BrushAction::DeleteFile(key)));
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
            "「broken.gbr」を取り込めません（",
            "Cannot import \"broken.gbr\" (",
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
        s.message.contains("「broken.gbr」は読めません（"),
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
    // 多角形の選択の途中。取り込みが終わっても、ツールも途中の形もそのまま。ブラシだけが取り込んだものに替わる
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
    assert_eq!(s.shown_brush_group(), Some(Group::Imported));
    assert!(s.m2.brush.tip.image.is_some(), "今の設定も取り込んだ筆先");
    // バケツなどほかのツールでも同じ
    s.tool = Tool::Fill;
    let file = write(&dir, "second.gbr", &gbr_gray("Second"));
    import(&mut s, &[file]);
    assert_eq!(s.tool, Tool::Fill);
    assert_eq!(s.brushes.lib.current(), imported(&s)[1].key);
    // ブラシ・消しゴムのときは従来どおり、ブラシに合わせてツールもブラシへ
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
        BrushAction::DeleteFile(old),
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
    // 取消のあと（仕事が無くなれば）、また消せる（並びから外す・ファイルを消す）
    s.apply(Action::Brush(BrushAction::ImportCancel));
    finish(&mut s);
    s.apply(Action::Brush(BrushAction::Delete(old)));
    assert!(!s.toolset.set.contains(old));
    s.apply(Action::Brush(BrushAction::DeleteFile(old)));
    assert!(s.brushes.lib.entry(old).is_none());
    assert_eq!(brush_files(&dir), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_a_brush_order_that_cannot_be_saved_is_reported_after_the_import() {
    for (lang, expect) in [
        (Lang::Ja, "ツールの並びを保存できません"),
        (Lang::En, "Cannot save the tool layout"),
    ] {
        let dir = temp_dir("order-fails");
        let file = write(&dir, "chalk.gbr", &gbr_gray("Chalk"));
        let mut s = state(&dir);
        s.lang = lang;
        // ツールの並びのファイルの場所をフォルダで塞ぐ（置換できない）。ブラシのファイルは置ける
        std::fs::create_dir_all(dir.join("tools.json")).unwrap();
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
    add_many(&mut s, MAX_USER_BRUSHES - 1);
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
    // 模様のブラシのファイルを消すと、その模様は選びから消える
    s.apply(Action::Brush(BrushAction::DeleteFile(a.key)));
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

// ---------------- 入り抜き・手ぶれ補正を持つブラシ ----------------

/// 入り抜き・手ぶれ補正を持つ .sut（入り `taper_in`・抜き `taper_out`・手ぶれ補正の段階 `level`）。
fn taper_sut(name: &str, taper_in: f64, taper_out: f64, level: i64) -> Vec<u8> {
    use sut_files::*;
    SutBuilder::new()
        .brush(
            name,
            1,
            &[
                ("BrushSize", real(30.0)),
                ("BrushUseIn", int(1)),
                ("BrushInLength", real(taper_in)),
                ("BrushUseOut", int(1)),
                ("BrushOutLength", real(taper_out)),
                ("BrushUseRevision", int(i64::from(level > 0))),
                ("BrushRevision", int(level)),
            ],
        )
        .build()
}

#[test]
fn headless_a_brush_that_carries_start_end_and_stabilization_applies_them_only_while_it_is_selected(
) {
    use yolu_app::engine::StrokeAssist;
    let dir = temp_dir("assist");
    let first = write(&dir, "first.sut", &taper_sut("First", 20.0, 30.0, 8));
    let second = write(&dir, "second.sut", &taper_sut("Second", 50.0, 0.0, 0));
    let mut s = state(&dir);
    // 描き手の設定（ブラシを替えても残る手ぶれ補正）
    s.m2.brush.assist = StrokeAssist {
        stabilizer: 7.0,
        taper_in: 0.0,
        taper_out: 0.0,
        curve: true,
    };
    let drawer = s.m2.brush.assist;
    import(&mut s, &[first, second]);
    let list = imported(&s);
    assert_eq!(list.len(), 2);
    // ブラシの設定には入らず、ブラシの外に持つ（正規の形は手ぶれ補正・入り抜きを持たない）
    assert_eq!(list[0].baseline.assist, StrokeAssist::default());
    let carried = |e: &Entry| e.assist.expect("入り抜きを持つ");
    assert_eq!(
        (
            carried(&list[0]).taper_in,
            carried(&list[0]).taper_out,
            carried(&list[0]).stabilizer
        ),
        (20.0, 30.0, 8.0)
    );
    assert_eq!(carried(&list[1]).taper_in, 50.0);
    // 取り込み終わりに替わった最初のブラシ: その値が今の設定に重なる（曲線の切り替えは描き手のまま）
    assert_eq!(s.brushes.lib.current(), list[0].key);
    assert_eq!(
        s.m2.brush.assist,
        StrokeAssist {
            stabilizer: 8.0,
            taper_in: 20.0,
            taper_out: 30.0,
            curve: true
        }
    );
    // ブラシの設定と比べる印は変わらない（持つ値は「変えた」ことにならない）
    assert!(!s.brush_is_modified(list[0].key));
    // 持つブラシから持つブラシへ: 値が替わり、描き手の設定は覚えたまま
    s.apply(Action::Brush(BrushAction::Select(list[1].key)));
    assert_eq!(
        (s.m2.brush.assist.taper_in, s.m2.brush.assist.stabilizer),
        (50.0, 0.0)
    );
    assert_eq!(s.brushes.drawer_assist, Some(drawer));
    // 持たないブラシへ替えると、描き手の設定へ戻る
    s.apply(Action::Brush(BrushAction::Select(BrushKey::Builtin(
        yolu_app::brushes::builtin::STANDARD,
    ))));
    assert_eq!(s.m2.brush.assist, drawer);
    assert_eq!(s.brushes.drawer_assist, None);
    // 持つブラシで値を変えても、持たないブラシへ替えれば描き手の設定のまま（描き手の設定を上書きしない）
    s.apply(Action::Brush(BrushAction::Select(list[0].key)));
    assert!(!s.brush_is_modified(list[0].key));
    s.m2.brush.assist.taper_in = 99.0;
    // 持つ値から変えたら「変えた」になる（描き手の設定だけのブラシは、変えても「変えた」にならない）
    assert!(s.brush_is_modified(list[0].key));
    // 元に戻す: そのブラシが持つ値へ
    s.apply(Action::Brush(BrushAction::Revert(list[0].key)));
    assert_eq!(s.m2.brush.assist.taper_in, 20.0);
    assert!(!s.brush_is_modified(list[0].key));
    // 登録: 今の値をそのブラシの持つ値にして保存する（曲線の切り替えを変えても「変えた」にならない）
    s.m2.brush.assist.curve = false;
    assert!(!s.brush_is_modified(list[0].key));
    s.m2.brush.assist.curve = true;
    s.m2.brush.assist.taper_out = 31.0;
    s.apply(Action::Brush(BrushAction::Register(list[0].key)));
    assert!(!s.brush_is_modified(list[0].key));
    assert_eq!(
        s.brushes
            .lib
            .entry(list[0].key)
            .unwrap()
            .assist
            .map(|a| a.taper_out),
        Some(31.0)
    );
    assert_eq!(
        state(&dir)
            .brushes
            .lib
            .entry(list[0].key)
            .unwrap()
            .assist
            .map(|a| a.taper_out),
        Some(31.0)
    );
    s.m2.brush.assist.taper_out = 30.0;
    s.apply(Action::Brush(BrushAction::Register(list[0].key)));
    assert_eq!(
        s.brushes
            .lib
            .entry(list[0].key)
            .unwrap()
            .assist
            .map(|a| a.taper_out),
        Some(30.0)
    );
    s.m2.brush.assist.taper_in = 99.0;
    // 複製は持つ値を引き継ぐ（今のブラシからの複製は今の値）
    s.apply(Action::Brush(BrushAction::Duplicate(list[0].key)));
    let copy = s
        .brushes
        .lib
        .entry(s.brushes.lib.current())
        .unwrap()
        .clone();
    assert_eq!(copy.assist.map(|a| a.taper_in), Some(99.0));
    // 今の設定からの追加も、持つブラシの上なら今の値を持つ
    s.m2.brush.assist.taper_out = 5.0;
    s.apply(Action::Brush(BrushAction::Add));
    let added = s
        .brushes
        .lib
        .entry(s.brushes.lib.current())
        .unwrap()
        .clone();
    assert_eq!(
        added.assist.map(|a| (a.taper_in, a.taper_out)),
        Some((99.0, 5.0))
    );
    // 持たないブラシからの追加は持たない
    s.apply(Action::Brush(BrushAction::Select(BrushKey::Builtin(
        yolu_app::brushes::builtin::STANDARD,
    ))));
    s.apply(Action::Brush(BrushAction::Add));
    let plain = s
        .brushes
        .lib
        .entry(s.brushes.lib.current())
        .unwrap()
        .clone();
    assert_eq!(plain.assist, None);
    assert_eq!(s.m2.brush.assist, drawer, "描き手の設定は変わらない");
    // 保存して読み戻しても、持つ値は同じ（持たないブラシは持たない）
    let back = state(&dir);
    assert!(
        back.brushes.problems.is_empty(),
        "{:?}",
        back.brushes.problems
    );
    let key = |e: &Entry| e.key;
    let find = |s: &AppState, k: BrushKey| s.brushes.lib.entry(k).unwrap().assist;
    assert_eq!(find(&back, key(&list[0])), list[0].assist);
    assert_eq!(find(&back, key(&list[1])), list[1].assist);
    assert_eq!(find(&back, key(&copy)), copy.assist);
    assert_eq!(find(&back, key(&added)), added.assist);
    assert_eq!(find(&back, key(&plain)), None);
    // 持つブラシのファイルだけが版 4（持たないブラシのファイルは今までの版のまま）
    let versions: Vec<String> = std::fs::read_dir(dir.join("brushes"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".ylbrush"))
        .map(|e| {
            std::fs::read_to_string(e.path())
                .unwrap()
                .lines()
                .next()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(
        versions
            .iter()
            .filter(|v| *v == "yolupainter-brush 4")
            .count(),
        4
    );
    // 持たないブラシ（今の既定の「標準」。縁のアンチエイリアスが 中）は版 5 で、入り抜き・手ぶれ補正の項目は書かない
    assert_eq!(
        versions
            .iter()
            .filter(|v| *v == "yolupainter-brush 5")
            .count(),
        1
    );
    let fifth: Vec<String> = std::fs::read_dir(dir.join("brushes"))
        .unwrap()
        .flatten()
        .map(|e| std::fs::read_to_string(e.path()).unwrap_or_default())
        .filter(|t| t.starts_with("yolupainter-brush 5\n"))
        .collect();
    assert_eq!(fifth.len(), 1);
    assert!(
        fifth[0].contains("\nanti_alias=medium\n") && !fifth[0].contains("assist."),
        "{}",
        fifth[0]
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_the_sample_of_a_brush_without_start_end_is_drawn_with_the_drawers_setting_while_another_is_selected(
) {
    use yolu_app::engine::StrokeAssist;
    let dir = temp_dir("assist-sample");
    let first = write(&dir, "first.sut", &taper_sut("First", 1130.0, 14.0, 6));
    let second = write(&dir, "second.sut", &taper_sut("Second", 50.0, 0.0, 0));
    let mut s = state(&dir);
    let drawer = StrokeAssist {
        stabilizer: 7.0,
        taper_in: 3.0,
        taper_out: 0.0,
        curve: false,
    };
    s.m2.brush.assist = drawer;
    import(&mut s, &[first, second]);
    let list = imported(&s);
    let plain = BrushKey::Builtin(yolu_app::brushes::builtin::STANDARD);
    // 取り込み終わりに替わった持つブラシを選んでいる間: 今の設定には持つ値が重なっている
    assert_eq!(s.brushes.lib.current(), list[0].key);
    assert_eq!(s.m2.brush.assist.taper_in, 1130.0);
    let row =
        |s: &AppState, key: BrushKey| s.brush_row_assist(s.brushes.lib.entry(key).unwrap().assist);
    // 持たないブラシの見本は、重なった値ではなく描き手の設定
    assert_eq!(row(&s, plain), drawer);
    // 持つブラシの見本は、それぞれの持つ値（曲線の切り替えだけは今の値）
    assert_eq!(row(&s, list[1].key).taper_in, 50.0);
    assert_eq!(row(&s, list[0].key).taper_in, 1130.0);
    // 持つブラシから持つブラシへ替えても、持たないブラシの見本は描き手の設定のまま
    s.apply(Action::Brush(BrushAction::Select(list[1].key)));
    assert_eq!(row(&s, plain), drawer);
    assert_eq!(row(&s, list[0].key).taper_in, 1130.0);
    // 曲線の切り替えは今の値に従う（どの行も同じ）
    s.m2.brush.assist.curve = true;
    assert!(row(&s, plain).curve && row(&s, list[0].key).curve);
    assert_eq!(row(&s, plain).taper_in, 3.0);
    // 持たないブラシを選べば、今の設定が描き手の設定そのもの
    s.apply(Action::Brush(BrushAction::Select(plain)));
    assert_eq!(
        row(&s, plain),
        StrokeAssist {
            curve: true,
            ..drawer
        }
    );
    assert_eq!(row(&s, list[0].key).taper_in, 1130.0);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_what_a_sut_carried_over_is_listed_apart_from_what_was_left_out() {
    use sut_files::*;
    use yolu_io::brushes::SutMapped;
    let dir = temp_dir("carried");
    let fall = [(0.0, 1.0), (1.0, 0.0)];
    let file = SutBuilder::new()
        .brush(
            "Mixed",
            1,
            &[
                ("BrushSize", real(30.0)),
                ("BrushUseIn", int(1)),
                ("BrushInLength", real(10.0)),
                ("BrushRotation", real(15.0)),
                (
                    "BrushSizeEffector",
                    blob(effector_slots(
                        0x30,
                        0,
                        Some(&[(0.0, 0.0), (1.0, 1.0)]),
                        Some(&fall),
                    )),
                ),
                ("BrushUseSpray", int(1)),
            ],
        )
        .build();
    let path = write(&dir, "mixed.sut", &file);
    let mut s = state(&dir);
    import(&mut s, &[path]);
    let e = &imported(&s)[0];
    let carried = e.import.as_ref().unwrap().mapped.clone();
    assert_eq!(
        carried,
        [
            SutMapped::Pressure,
            SutMapped::Tilt,
            SutMapped::StartEnd,
            SutMapped::TipAngle
        ]
    );
    // ツールチップには、取り込んだときに写した項目と表せなかった項目がそのまま並ぶ
    let tip = yolu_app::brushes::gaps::row_tooltip(Lang::En, "Mixed", false, e.import.as_ref());
    assert!(
        tip.contains("Carried over: pen pressure, pen tilt, start and end, tip angle"),
        "{tip}"
    );
    assert!(tip.contains("Not represented: "), "{tip}");
    let tip_ja = yolu_app::brushes::gaps::row_tooltip(Lang::Ja, "Mixed", true, e.import.as_ref());
    assert!(
        tip_ja.starts_with("Mixed（変更あり）\n")
            && tip_ja.contains("写した項目: 筆圧、傾き、入り抜き、筆先の角度"),
        "{tip_ja}"
    );
    // 近似したものと表せなかったものは、写した項目とは別に「表せなかった項目」に出る
    let gaps = &e.import.as_ref().unwrap().gaps;
    assert!(gaps.contains(&Gap::StartEndDetail) && gaps.contains(&Gap::Spray));
    assert!(
        !gaps.contains(&Gap::StartEnd),
        "写せた入り抜きを未対応とは言わない"
    );
    // 保存して読み戻しても、同じ項目が出る
    let back = state(&dir);
    let again = &imported(&back)[0];
    assert_eq!(again.import.as_ref().unwrap().mapped, carried);
    assert_eq!(again.import, e.import);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_the_carried_over_list_is_what_the_file_gave_not_what_the_brush_has_now() {
    use yolu_io::brushes::SutMapped;
    let dir = temp_dir("carried-kept");
    // 入り抜きと手ぶれ補正だけを持つ .sut（筆圧・傾きの影響元は無い）
    let path = write(&dir, "taper.sut", &taper_sut("Taper", 20.0, 30.0, 8));
    let mut s = state(&dir);
    import(&mut s, &[path]);
    let key = imported(&s)[0].key;
    let from_file = vec![SutMapped::StartEnd, SutMapped::Stabilizer];
    assert_eq!(imported(&s)[0].import.as_ref().unwrap().mapped, from_file);
    // 取り込んだあとに利用者が傾き・筆圧・筆先の角度を入れて、ブラシとして登録する
    s.apply(Action::Brush(BrushAction::Select(key)));
    s.m2.brush.controls.tilt_size = true;
    s.brush.pressure_flow = true;
    s.m2.brush.tip.angle = 30.0;
    s.apply(Action::Brush(BrushAction::Register(key)));
    let e = s.brushes.lib.entry(key).unwrap().clone();
    assert!(
        e.baseline.controls.tilt_size
            && e.baseline.base.pressure_flow
            && e.baseline.tip.angle == 30.0
    );
    // ファイルから写していない項目は、写した項目に並ばない（ブラシの今の設定から導かない）
    assert_eq!(e.import.as_ref().unwrap().mapped, from_file);
    let tip = yolu_app::brushes::gaps::row_tooltip(Lang::En, "Taper", false, e.import.as_ref());
    assert!(
        tip.contains("Carried over: start and end, stabilization"),
        "{tip}"
    );
    assert!(
        !tip.contains("pen tilt") && !tip.contains("pen pressure") && !tip.contains("tip angle"),
        "{tip}"
    );
    // 保存して読み戻しても同じ。複製は引き継ぐ
    let back = state(&dir);
    assert_eq!(back.brushes.lib.entry(key).unwrap().import, e.import);
    s.apply(Action::Brush(BrushAction::Duplicate(key)));
    let copy = s
        .brushes
        .lib
        .entry(s.brushes.lib.current())
        .unwrap()
        .clone();
    assert_eq!(copy.import.as_ref().unwrap().mapped, from_file);
    // 今の設定からの追加は、取り込んだブラシではないので何も持たない
    s.apply(Action::Brush(BrushAction::Add));
    let added = s
        .brushes
        .lib
        .entry(s.brushes.lib.current())
        .unwrap()
        .clone();
    assert_eq!(added.import, None);
    // 取り込みの印の無い行は、名前だけ
    assert_eq!(
        yolu_app::brushes::gaps::row_tooltip(Lang::En, "Plain", false, None),
        "Plain"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

// ---------------- 「CLIP STUDIO から」 ----------------

use yolu_app::brushes::clipstudio::RowState;
use yolu_io::brushes::clipstudio::{Missing, Places};

/// 探す・覗く仕事が終わるまで受ける。
fn finish_csp(s: &mut AppState) {
    let start = Instant::now();
    while s.brushes.csp.is_busy() && start.elapsed() < Duration::from_secs(60) {
        s.poll_brush_csp();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!s.brushes.csp.is_busy(), "探す仕事が終わらない");
}

/// 試験用の CELSYS の設定のフォルダ（`AppData/Roaming/CELSYSUserData/CELSYS` の下に .sut 2 つと、読めない 1 つ）。
fn celsys_home(dir: &Path) -> Places {
    use sut_files::*;
    let root = dir.join("AppData/Roaming/CELSYSUserData/CELSYS/CLIPStudioModule/SubTool");
    std::fs::create_dir_all(root.join("Pen")).unwrap();
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
    std::fs::write(root.join("Pen/a_stamp.sut"), stamp).unwrap();
    std::fs::write(root.join("Pen/b_ink.sut"), taper_sut("Ink", 20.0, 20.0, 0)).unwrap();
    std::fs::write(root.join("Pen/c_broken.sut"), b"not a database at all").unwrap();
    Places {
        appdata: Some(dir.join("AppData/Roaming")),
        profile: Some(dir.to_path_buf()),
        onedrive: None,
    }
}

/// フォルダの下の全部の項目（相対パス・大きさ・更新時刻）。読んだだけで変わらないことを確かめる。
fn tree(dir: &Path) -> Vec<(String, u64, Option<std::time::SystemTime>)> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, u64, Option<std::time::SystemTime>)>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let meta = entry.metadata().unwrap();
            out.push((
                entry
                    .path()
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                meta.len(),
                meta.modified().ok(),
            ));
            if meta.is_dir() {
                walk(base, &entry.path(), out);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn headless_the_clip_studio_window_lists_what_it_finds_and_imports_only_the_selected() {
    let dir = temp_dir("csp-list");
    let places = celsys_home(&dir);
    let before = tree(&dir.join("AppData"));
    let mut s = state(&dir);
    s.brushes.csp.places = Some(places);
    s.apply(Action::Brush(BrushAction::ClipStudioOpen));
    assert!(s.brushes.csp.open && s.brushes.csp.is_busy());
    assert!(s.brushes.csp.listing.is_none(), "探している間は一覧が無い");
    finish_csp(&mut s);
    let listing = s.brushes.csp.listing.as_ref().expect("一覧");
    assert_eq!(listing.missing, None);
    let shown: Vec<&str> = listing.rows.iter().map(|r| r.shown.as_str()).collect();
    assert_eq!(
        shown,
        [
            "CLIPStudioModule/SubTool/Pen/a_stamp.sut",
            "CLIPStudioModule/SubTool/Pen/b_ink.sut",
            "CLIPStudioModule/SubTool/Pen/c_broken.sut"
        ],
        "探し始めのフォルダからの相対（パスの順）"
    );
    // 名前と筆先の見本（画像の筆先は縦横比を保って枠に収める）。読めないファイルは読めなかった理由つき・選べない
    let names: Vec<Option<String>> = listing
        .rows
        .iter()
        .map(|r| match &r.state {
            RowState::Ready(p) => Some(p.names[0].clone()),
            _ => None,
        })
        .collect();
    assert_eq!(names, [Some("Stamp".into()), Some("Ink".into()), None]);
    assert!(matches!(listing.rows[2].state, RowState::Failed(_)));
    match &listing.rows[0].state {
        RowState::Ready(p) => {
            let side = p.preview.side as usize;
            assert_eq!(p.preview.alpha.len(), side * side);
            assert_eq!(
                p.preview.alpha[side / 4 * side + side / 4],
                255,
                "左上の暗い画素"
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        (listing.ready(), listing.selected()),
        (2, 0),
        "初めは何も選んでいない"
    );
    // 何も選ばずに取り込んでも何もしない・ウィンドウは開いたまま
    s.apply(Action::Brush(BrushAction::ClipStudioImport));
    assert!(s.brushes.csp.open && !s.is_brush_importing());
    // 読めなかった行は選べない。全部選ぶも読めた行だけ
    s.apply(Action::Brush(BrushAction::ClipStudioToggle(2)));
    assert_eq!(s.brushes.csp.listing.as_ref().unwrap().selected(), 0);
    s.apply(Action::Brush(BrushAction::ClipStudioSelectAll(true)));
    let l = s.brushes.csp.listing.as_ref().unwrap();
    assert_eq!((l.selected(), l.rows[2].selected), (2, false));
    s.apply(Action::Brush(BrushAction::ClipStudioSelectAll(false)));
    // 選んだ行だけを取り込む（いつもの .sut の取り込みの道）。ウィンドウは閉じる
    s.apply(Action::Brush(BrushAction::ClipStudioToggle(1)));
    s.apply(Action::Brush(BrushAction::ClipStudioImport));
    assert!(!s.brushes.csp.open && s.brushes.csp.listing.is_none());
    finish(&mut s);
    let list = imported(&s);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "Ink");
    assert_eq!(
        list[0].assist.map(|a| (a.taper_in, a.taper_out)),
        Some((20.0, 20.0))
    );
    assert!(
        s.message.starts_with("ブラシを 1 個取り込みました"),
        "{}",
        s.message
    );
    // CLIP STUDIO のフォルダは、探す・覗く・取り込むのどれでも変わらない（更新時刻・大きさ・項目）
    assert_eq!(tree(&dir.join("AppData")), before);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_when_nothing_is_found_the_window_says_why_and_a_folder_can_be_chosen_by_hand() {
    let dir = temp_dir("csp-missing");
    let mut s = state(&dir);
    // 既定の場所が無い
    s.brushes.csp.places = Some(Places {
        appdata: Some(dir.join("AppData/Roaming")),
        profile: Some(dir.clone()),
        onedrive: None,
    });
    s.apply(Action::Brush(BrushAction::ClipStudioOpen));
    finish_csp(&mut s);
    let l = s.brushes.csp.listing.as_ref().unwrap();
    assert_eq!((l.rows.len(), l.missing), (0, Some(Missing::NoFolder)));
    assert!(s.brushes.csp.open, "ウィンドウは開いたまま");
    // 場所はあるが .sut が無い
    std::fs::create_dir_all(dir.join("AppData/Roaming/CELSYSUserData/CELSYS")).unwrap();
    s.apply(Action::Brush(BrushAction::ClipStudioRescan));
    finish_csp(&mut s);
    assert_eq!(
        s.brushes.csp.listing.as_ref().unwrap().missing,
        Some(Missing::NoFiles)
    );
    // 手で選んだフォルダ（CLIP STUDIO の外でもよい）の下を同じ探し方で一覧にする。探し直しも、手で選んだフォルダを探す
    let other = temp_dir("csp-picked");
    let picked = celsys_home(&other)
        .appdata
        .unwrap()
        .join("CELSYSUserData/CELSYS");
    s.apply(Action::Brush(BrushAction::ClipStudioFolder(picked.clone())));
    finish_csp(&mut s);
    assert_eq!(s.brushes.csp.folder.as_deref(), Some(picked.as_path()));
    assert_eq!(s.brushes.csp.listing.as_ref().unwrap().rows.len(), 3);
    s.apply(Action::Brush(BrushAction::ClipStudioRescan));
    finish_csp(&mut s);
    assert_eq!(s.brushes.csp.listing.as_ref().unwrap().rows.len(), 3);
    // 手で選んだフォルダが消えていたら、無いと言う
    std::fs::remove_dir_all(&other).unwrap();
    s.apply(Action::Brush(BrushAction::ClipStudioRescan));
    finish_csp(&mut s);
    assert_eq!(
        s.brushes.csp.listing.as_ref().unwrap().missing,
        Some(Missing::NoFolder)
    );
    // ウィンドウを開き直すと、既定の場所へ戻る
    s.apply(Action::Brush(BrushAction::ClipStudioClose));
    s.apply(Action::Brush(BrushAction::ClipStudioOpen));
    assert_eq!(s.brushes.csp.folder, None);
    finish_csp(&mut s);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_closing_the_window_or_searching_again_cancels_the_running_search() {
    let dir = temp_dir("csp-cancel");
    let mut s = state(&dir);
    s.brushes.csp.places = Some(celsys_home(&dir));
    // 仕事を止めておいて、閉じる・別の探しで取り消せることを、仕事の速さに頼らず確かめる
    s.brushes.csp.park_next = true;
    s.apply(Action::Brush(BrushAction::ClipStudioOpen));
    assert!(s.brushes.csp.is_busy());
    s.apply(Action::Brush(BrushAction::ClipStudioClose));
    assert!(!s.brushes.csp.is_busy() && !s.brushes.csp.open && s.brushes.csp.listing.is_none());
    s.brushes.csp.park_next = true;
    s.apply(Action::Brush(BrushAction::ClipStudioOpen));
    s.apply(Action::Brush(BrushAction::ClipStudioRescan));
    finish_csp(&mut s);
    assert_eq!(
        s.brushes.csp.listing.as_ref().unwrap().rows.len(),
        3,
        "新しい探しの結果だけが残る"
    );
    // 取り込みが走っている間は、取り込めない
    s.apply(Action::Brush(BrushAction::ClipStudioSelectAll(true)));
    s.brushes.import.park_next = true;
    s.apply(Action::Brush(BrushAction::Import(vec![dir.join(
        "AppData/Roaming/CELSYSUserData/CELSYS/CLIPStudioModule/SubTool/Pen/b_ink.sut",
    )])));
    s.apply(Action::Brush(BrushAction::ClipStudioImport));
    assert!(s.brushes.csp.open, "取り込み中は閉じず、何も足さない");
    s.apply(Action::Brush(BrushAction::ImportCancel));
    finish(&mut s);
    std::fs::remove_dir_all(dir).unwrap();
}
