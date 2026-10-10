//! ブラシの一覧の操作・ツールごとの覚え・保存と読み戻し・見本の描き直しの条件。画面を描かないので Wine でも回る（`headless_`）。
use std::path::PathBuf;

use yolu_app::brushes::sample::{SampleCache, SampleSpec, RENDERS_PER_FRAME};
use yolu_app::brushes::{builtin, store, BrushAction, BrushKey, DropAt, Group, MAX_USER_BRUSHES};
use yolu_app::engine::{Brush, BrushEffect};
use yolu_app::lang::Lang;
use yolu_app::m2::UiOp;
use yolu_app::pen::PenInput;
use yolu_app::state::{Action, AppState, BrushState, Tool};
use yolu_app::toolset::{ToolsetAction, MAX_GROUP_BRUSHES};
use yolu_app::YoluApp;

fn b(id: &'static str) -> BrushKey {
    BrushKey::Builtin(id)
}

fn select(s: &mut AppState, key: BrushKey) {
    s.apply(Action::Brush(BrushAction::Select(key)));
}

/// 試験ごとの設定のフォルダの中の、ブラシのフォルダ（ツールの並びの `tools.json` は、その隣の設定のフォルダの直下に置かれる）。
fn temp_dir(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/brush-list-tests")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join("brushes");
    std::fs::create_dir_all(&dir).unwrap();
    dir
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

/// 利用者のブラシの名前（一覧の並びの順）。
fn user_names(s: &AppState) -> Vec<String> {
    s.brushes
        .lib
        .entries()
        .iter()
        .filter(|e| e.key.is_user())
        .map(|e| e.name.clone())
        .collect()
}

fn group_keys(s: &AppState, group: Group) -> Vec<BrushKey> {
    s.brush_entries_in(group).iter().map(|e| e.key).collect()
}

fn user_id(key: BrushKey) -> u32 {
    match key {
        BrushKey::User(id) => id,
        other => panic!("利用者のブラシではない: {other:?}"),
    }
}

#[test]
fn headless_the_list_starts_on_the_standard_brush_and_changes_nothing() {
    let s = AppState::new(64, 64);
    assert_eq!(s.brushes.lib.current(), b(builtin::STANDARD));
    assert_eq!(s.shown_brush_group(), Some(Group::Pen));
    // 起動直後の設定は、今までの既定のブラシと同じ（標準を選んでも何も変わらない）
    assert_eq!(s.brush, BrushState::default());
    assert_eq!(s.m2.brush, Brush::default());
    assert!(!s.brush_is_modified(b(builtin::STANDARD)));
    // どのグループにも組み込みがある。消しゴムのグループの先頭は消しゴムの標準
    for group in Group::ALL {
        assert!(!group_keys(&s, group).is_empty(), "{group:?}");
    }
    assert_eq!(
        group_keys(&s, Group::Eraser)[0],
        b(builtin::STANDARD_ERASER)
    );
    // 標準を選び直しても同じ（`m2.brush.base` はストロークのたびに画面の値で上書きされるので、比べない）
    let mut again = AppState::new(64, 64);
    select(&mut again, b(builtin::STANDARD));
    assert_eq!(again.brush, s.brush);
    let mut a = again.m2.brush.clone();
    a.base = s.m2.brush.base;
    assert_eq!(a, s.m2.brush);
}

#[test]
fn headless_selecting_a_brush_loads_its_settings_and_follows_the_tool() {
    let mut s = AppState::new(64, 64);
    s.m2.brush.assist.stabilizer = 12.0;
    s.color.set_main([0.2, 0.4, 0.6, 1.0]);
    select(&mut s, b("pencil"));
    assert_eq!((s.brush.radius, s.brush.hardness), (3.0, 0.6));
    assert!(s.m2.brush.texture.is_some());
    assert_eq!(s.tool, Tool::Brush);
    assert_eq!(s.shown_brush_group(), Some(Group::Pen));
    assert_eq!(
        s.m2.brush.assist.stabilizer, 12.0,
        "手ぶれ補正は描き手の設定"
    );
    assert_eq!(s.color.main, [0.2, 0.4, 0.6, 1.0], "描画色は替わらない");
    // 消しゴムのグループのブラシは、ツールも消しゴムにする
    select(&mut s, b("soft-eraser"));
    assert_eq!(s.tool, Tool::Eraser);
    assert_eq!(s.shown_brush_group(), Some(Group::Eraser));
    assert_eq!((s.brush.radius, s.brush.hardness), (24.0, 0.0));
    // 効果のブラシは描くツール
    select(&mut s, b("blur"));
    assert_eq!(s.tool, Tool::Brush);
    assert!(matches!(s.m2.brush.effect, BrushEffect::Blur { .. }));
    assert_eq!(s.shown_brush_group(), Some(Group::Effect));
    // 選択のツールのときに選べば、描くツールになる
    s.apply(Action::SelectTool(Tool::Lasso));
    select(&mut s, b("marker"));
    assert_eq!(s.tool, Tool::Brush);
    // 旧い入口（プリセットの番号）も同じ一覧を通る
    let chalk = yolu_app::m2::presets()
        .iter()
        .position(|p| p.id == "chalk")
        .unwrap();
    s.apply(Action::M2Ui(UiOp::Preset(chalk)));
    assert_eq!(s.brushes.lib.current(), b("chalk"));
    assert_eq!(s.m2.preset, Some(chalk));
    // どの組み込みも、選んでそのまま描き始められる（検証を通る）
    for e in s.brushes.lib.entries().to_vec() {
        select(&mut s, e.key);
        let id = s.selected_layer.unwrap();
        let eraser = s.tool == Tool::Eraser;
        let stroke = s
            .begin_paint_stroke(id, eraser)
            .unwrap_or_else(|err| panic!("{:?}: {err}", e.key));
        s.doc.cancel_stroke(stroke);
    }
}

#[test]
fn headless_each_tool_remembers_its_last_brush() {
    let mut s = AppState::new(64, 64);
    select(&mut s, b("chalk"));
    // E: 消しゴムのグループの、最後に使ったブラシ（初めは標準の消しゴム）
    s.apply(Action::SelectTool(Tool::Eraser));
    assert_eq!(s.brushes.lib.current(), b(builtin::STANDARD_ERASER));
    assert_eq!(s.shown_brush_group(), Some(Group::Eraser));
    select(&mut s, b("hard-eraser"));
    s.brush.radius = 33.0;
    s.apply(Action::SelectTool(Tool::Eraser)); // もう消しゴムなので何も変わらない
    assert_eq!(s.brush.radius, 33.0);
    // B: 描くツールの最後のブラシ（チョーク）
    s.apply(Action::SelectTool(Tool::Brush));
    assert_eq!(s.brushes.lib.current(), b("chalk"));
    assert_eq!(s.brush.radius, 18.0, "チョークの設定");
    s.apply(Action::SelectTool(Tool::Eraser));
    assert_eq!(s.brushes.lib.current(), b("hard-eraser"));
    assert_eq!(s.brush.radius, 33.0, "消しゴムの変えた設定も覚えている");
    assert!(s.brush_is_modified(b("hard-eraser")));
    // 選択のツールへ替えても、ブラシは替わらない
    s.apply(Action::SelectTool(Tool::SelectRect));
    assert_eq!(s.brushes.lib.current(), b("hard-eraser"));
    // 選択のツールから B を押すと、描くツールの最後のブラシへ
    s.apply(Action::SelectTool(Tool::Brush));
    assert_eq!(s.brushes.lib.current(), b("chalk"));
    // 消した利用者のブラシを覚えたままにしない
    s.apply(Action::Brush(BrushAction::Add));
    let user = s.brushes.lib.current();
    assert!(user.is_user());
    s.apply(Action::SelectTool(Tool::Eraser));
    s.apply(Action::SelectTool(Tool::Brush));
    assert_eq!(s.brushes.lib.current(), user);
    s.apply(Action::Brush(BrushAction::Delete(user)));
    s.apply(Action::SelectTool(Tool::Eraser));
    s.apply(Action::SelectTool(Tool::Brush));
    assert!(s.brushes.lib.entry(s.brushes.lib.current()).is_some());
    assert_ne!(s.brushes.lib.current(), user);
}

#[test]
fn headless_edits_stay_with_their_brush_and_can_be_reverted() {
    let mut s = AppState::new(64, 64);
    select(&mut s, b("pencil"));
    assert!(!s.brush_is_modified(b("pencil")));
    s.brush.radius = 40.0;
    s.m2.brush.jitter.size = 0.3;
    assert!(s.brush_is_modified(b("pencil")), "変えたら変更あり");
    assert!(!s.brush_is_modified(b("marker")));
    // 別のブラシへ替えても、変更あり（のまま）。戻ると変えた設定が残っている
    select(&mut s, b("marker"));
    assert!(s.brush_is_modified(b("pencil")));
    assert_eq!(s.brush.radius, 14.0);
    select(&mut s, b("pencil"));
    assert_eq!((s.brush.radius, s.m2.brush.jitter.size), (40.0, 0.3));
    // 元に戻す: 元の設定へ（今のブラシなら、今の設定も）
    s.apply(Action::Brush(BrushAction::Revert(b("pencil"))));
    assert_eq!((s.brush.radius, s.m2.brush.jitter.size), (3.0, 0.0));
    assert!(!s.brush_is_modified(b("pencil")));
    // 今のブラシでないものの変更も戻せる
    s.brush.radius = 9.0;
    select(&mut s, b("marker"));
    assert!(s.brush_is_modified(b("pencil")));
    s.apply(Action::Brush(BrushAction::Revert(b("pencil"))));
    assert!(!s.brush_is_modified(b("pencil")));
    select(&mut s, b("pencil"));
    assert_eq!(s.brush.radius, 3.0);
    // 変えて、元と同じ値へ戻せば、変更ありではない
    s.brush.radius = 5.0;
    assert!(s.brush_is_modified(b("pencil")));
    s.brush.radius = 3.0;
    assert!(!s.brush_is_modified(b("pencil")));
    // 手ぶれ補正・入り抜きは描き手の設定なので、変えても変更ありにならない
    s.m2.brush.assist.stabilizer = 50.0;
    s.m2.brush.assist.taper_in = 10.0;
    assert!(!s.brush_is_modified(b("pencil")));
    // ブラシは文書ではない: どの操作も Undo の段を作らない
    assert!(!s.doc.can_undo());
}

#[test]
fn headless_switching_brushes_is_refused_while_stroking() {
    let mut s = AppState::new(64, 64);
    let id = s.selected_layer.unwrap();
    select(&mut s, b("pencil"));
    let stroke = s.begin_paint_stroke(id, false).unwrap();
    assert!(s.is_stroking());
    let before = (s.brush.clone(), s.m2.brush.clone(), s.tool);
    for action in [
        BrushAction::Select(b("marker")),
        BrushAction::Add,
        BrushAction::Duplicate(b("pencil")),
        BrushAction::Revert(b("pencil")),
    ] {
        s.message.clear();
        s.apply(Action::Brush(action.clone()));
        assert_eq!(s.message, "描いている間はできません。", "{action:?}");
        assert_eq!(s.brushes.lib.current(), b("pencil"));
    }
    // ツールをブラシから消しゴムへ替えるのも断る
    s.message.clear();
    s.apply(Action::SelectTool(Tool::Eraser));
    assert_eq!(s.tool, Tool::Brush);
    assert_eq!(s.message, "描いている間はできません。");
    s.apply(Action::M2Ui(UiOp::Preset(0)));
    assert_eq!(s.brushes.lib.current(), b("pencil"));
    assert_eq!((s.brush.clone(), s.m2.brush.clone(), s.tool), before);
    // 英語でも短い理由
    s.lang = Lang::En;
    s.apply(Action::Brush(BrushAction::Select(b("marker"))));
    assert_eq!(s.message, "Not while drawing.");
    s.doc.cancel_stroke(stroke);
    s.canvas.stroke = None;
    s.apply(Action::SelectTool(Tool::Eraser));
    assert_eq!(s.tool, Tool::Eraser, "終われば替えられる");
}

#[test]
fn headless_add_duplicate_rename_delete_and_reorder_user_brushes() {
    let mut s = AppState::new(64, 64);
    select(&mut s, b("marker"));
    s.brush.radius = 21.0;
    // 追加: 今の設定を新しいブラシに（今のブラシのグループの一番後ろ。それに替わる）
    s.apply(Action::Brush(BrushAction::Add));
    let first = s.brushes.lib.current();
    assert!(first.is_user());
    assert_eq!(user_names(&s), ["ブラシ"]);
    assert_eq!(s.brush.radius, 21.0);
    assert!(!s.brush_is_modified(first), "登録した設定が元");
    assert_eq!(*group_keys(&s, Group::Pen).last().unwrap(), first);
    assert_eq!(s.shown_brush_group(), Some(Group::Pen));
    // 元のマーカーは、変更ありのまま残る
    assert!(s.brush_is_modified(b("marker")));
    // 複製: すぐ後ろへ。名前は「… のコピー」で重ならない
    s.apply(Action::Brush(BrushAction::Duplicate(first)));
    let second = s.brushes.lib.current();
    assert_eq!(user_names(&s), ["ブラシ", "ブラシ のコピー"]);
    s.apply(Action::Brush(BrushAction::Duplicate(first)));
    let third = s.brushes.lib.current();
    assert_eq!(
        s.brushes.lib.entry(third).unwrap().name,
        "ブラシ のコピー 2"
    );
    let pen = group_keys(&s, Group::Pen);
    let at = pen.iter().position(|k| *k == first).unwrap();
    assert_eq!(pen[at + 1], third, "元のすぐ後ろ");
    assert_eq!(pen[at + 2], second);
    // 組み込みも複製できる（利用者のブラシになる）。消しゴムの複製は消しゴムのグループで、ツールも消しゴム
    s.apply(Action::Brush(BrushAction::Duplicate(b("soft-eraser"))));
    let eraser_copy = s.brushes.lib.current();
    assert!(eraser_copy.is_user());
    assert_eq!(
        s.brushes.lib.entry(eraser_copy).unwrap().group,
        Group::Eraser
    );
    assert_eq!(s.tool, Tool::Eraser);
    assert_eq!(s.brush.radius, 24.0);
    assert_eq!(user_names(&s)[3], "ソフト消しゴム のコピー");
    // 名前の変更: 整える（制御文字・前後の空白）。空は変えない
    s.apply(Action::Brush(BrushAction::StartRename(first)));
    assert_eq!(s.brushes.ui.renaming, Some(first));
    s.apply(Action::Brush(BrushAction::Rename(
        first,
        "  線画\n用  ".into(),
    )));
    assert_eq!(s.brushes.ui.renaming, None);
    assert_eq!(s.brushes.lib.entry(first).unwrap().name, "線画 用");
    s.apply(Action::Brush(BrushAction::Rename(first, "   ".into())));
    assert_eq!(
        s.brushes.lib.entry(first).unwrap().name,
        "線画 用",
        "空は変えない"
    );
    // 並べ替え: 同じグループの中だけ
    let before = group_keys(&s, Group::Pen);
    s.apply(Action::Brush(BrushAction::Move {
        key: first,
        at: DropAt::Before(before[0]),
    }));
    assert_eq!(group_keys(&s, Group::Pen)[0], first);
    s.apply(Action::Brush(BrushAction::Move {
        key: first,
        at: DropAt::End,
    }));
    assert_eq!(*group_keys(&s, Group::Pen).last().unwrap(), first);
    let order = s.toolset.set.brushes();
    s.apply(Action::Brush(BrushAction::Move {
        key: first,
        at: DropAt::Before(b("soft-eraser")), // ほかのグループの前へは動かさない
    }));
    assert_eq!(s.toolset.set.brushes(), order);
    s.apply(Action::Brush(BrushAction::Move {
        key: first,
        at: DropAt::Before(first),
    }));
    assert_eq!(s.toolset.set.brushes(), order);
    // 削除: 今のブラシなら同じグループの隣へ移る
    select(&mut s, third);
    let pen = group_keys(&s, Group::Pen);
    let at = pen.iter().position(|k| *k == third).unwrap();
    s.apply(Action::Brush(BrushAction::Delete(third)));
    assert!(!s.toolset.set.contains(third));
    assert!(s.brushes.lib.entry(third).is_some(), "並びから外すだけ");
    assert_eq!(s.brushes.lib.current(), pen[at + 1]);
    assert_eq!(user_names(&s).len(), 4);
    // 一番後ろの今のブラシを消すと、前のブラシへ
    let pen = group_keys(&s, Group::Pen);
    let last = *pen.last().unwrap();
    select(&mut s, last);
    s.apply(Action::Brush(BrushAction::Delete(last)));
    assert_eq!(s.brushes.lib.current(), pen[pen.len() - 2]);
    // どれも文書は変えない
    assert!(!s.doc.can_undo());
}

#[test]
fn headless_built_in_brushes_are_taken_out_of_the_layout_and_become_copies_when_renamed_or_registered(
) {
    let mut s = AppState::new(64, 64);
    let count = s.brushes.lib.entries().len();
    // 削除は並びから外すだけ（組み込みの元は残り、「＋」のウィンドウから戻せる）
    s.apply(Action::Brush(BrushAction::Delete(b("chalk"))));
    assert_eq!(s.message, "ブラシを削除しました: チョーク");
    assert!(!s.toolset.set.contains(b("chalk")));
    assert_eq!(s.brushes.lib.entries().len(), count);
    s.apply(Action::Brush(BrushAction::AddFrom(vec![
        yolu_app::toolset::catalog::CatalogItem::Builtin("chalk"),
    ])));
    assert!(s.toolset.set.contains(b("chalk")), "参照のまま戻る");
    assert_eq!(s.brushes.lib.user_count(), 0, "写しは作らない");
    // 名前の変更は、同じ場所でファイルの写しに替える（組み込みの名前のままなら、何もしない）
    let place = s.toolset.set.find(b("chalk")).unwrap();
    s.apply(Action::Brush(BrushAction::StartRename(b("chalk"))));
    assert_eq!(s.brushes.ui.renaming, Some(b("chalk")));
    s.apply(Action::Brush(BrushAction::Rename(
        b("chalk"),
        "チョーク".into(),
    )));
    assert!(s.toolset.set.contains(b("chalk")));
    s.apply(Action::Brush(BrushAction::Rename(
        b("chalk"),
        "下描き".into(),
    )));
    assert!(!s.toolset.set.contains(b("chalk")));
    let copy = s.toolset.set.slots()[place.slot].groups[place.group].brushes[place.index];
    assert!(copy.is_user());
    assert_eq!(s.brushes.lib.entry(copy).unwrap().name, "下描き");
    assert_eq!(
        s.brushes.lib.entry(copy).unwrap().baseline,
        s.brushes.lib.entry(b("chalk")).unwrap().baseline,
        "組み込みと同じ設定"
    );
    // 登録も、変えたままの設定を元にした写しに替える（組み込みの元は出荷時のまま）
    select(&mut s, b("marker"));
    s.brush.radius = 9.0;
    let marker = s.toolset.set.find(b("marker")).unwrap();
    s.apply(Action::Brush(BrushAction::Register(b("marker"))));
    let registered = s.brushes.lib.current();
    assert!(registered.is_user());
    assert_eq!(
        s.toolset.set.slots()[marker.slot].groups[marker.group].brushes[marker.index],
        registered
    );
    assert!(!s.brush_is_modified(registered));
    assert_eq!(
        s.brushes
            .lib
            .entry(registered)
            .unwrap()
            .baseline
            .base
            .radius,
        9.0
    );
    assert_ne!(
        s.brushes
            .lib
            .entry(b("marker"))
            .unwrap()
            .baseline
            .base
            .radius,
        9.0
    );
    assert_eq!(s.brushes.lib.entry(b("marker")).unwrap().edited, None);
    s.lang = Lang::En;
    s.apply(Action::Brush(BrushAction::Delete(b("ink-pen"))));
    assert_eq!(s.message, "Brush deleted: Ink Pen");
    // 名前は言語に従う
    let chalk = s.brushes.lib.entry(b("chalk")).unwrap();
    assert_eq!(chalk.name_in(Lang::En), "Chalk");
    assert_eq!(chalk.name_in(Lang::Ja), "チョーク");
    for e in s.brushes.lib.entries() {
        for lang in Lang::ALL {
            assert!(!e.name_in(lang).is_empty());
        }
    }
    assert!(!s.doc.can_undo(), "文書は変えない");
}

#[test]
fn headless_registering_makes_the_edit_the_brush_and_revert_goes_back_to_it() {
    let mut s = AppState::new(64, 64);
    select(&mut s, b("pencil"));
    s.apply(Action::Brush(BrushAction::Add));
    let key = s.brushes.lib.current();
    s.brush.radius = 50.0;
    assert!(s.brush_is_modified(key));
    s.apply(Action::Brush(BrushAction::Register(key)));
    assert!(!s.brush_is_modified(key), "登録した設定が新しい元");
    assert_eq!(s.brushes.lib.entry(key).unwrap().baseline.base.radius, 50.0);
    s.brush.radius = 60.0;
    s.apply(Action::Brush(BrushAction::Revert(key)));
    assert_eq!(s.brush.radius, 50.0);
}

#[test]
fn headless_the_number_of_user_brushes_is_capped() {
    let mut s = AppState::new(64, 64);
    add_many(&mut s, MAX_USER_BRUSHES);
    assert_eq!(user_names(&s).len(), MAX_USER_BRUSHES);
    s.message.clear();
    s.apply(Action::Brush(BrushAction::Add));
    assert_eq!(user_names(&s).len(), MAX_USER_BRUSHES);
    assert!(
        s.message.contains(&MAX_USER_BRUSHES.to_string()),
        "{}",
        s.message
    );
    // 名前は全部ちがう
    let mut names = user_names(&s);
    names.sort();
    names.dedup();
    assert_eq!(names.len(), MAX_USER_BRUSHES);
}

// ───────── 保存と読み戻し ─────────

#[test]
fn headless_user_brushes_are_saved_and_come_back_with_their_order_and_registered_edits() {
    let dir = temp_dir("save");
    let mut s = AppState::new(64, 64);
    s.attach_brush_store(dir.clone());
    select(&mut s, b("chalk"));
    s.brush.radius = 27.0;
    s.m2.brush.jitter.scatter = 0.5;
    s.apply(Action::Brush(BrushAction::Add));
    let one = s.brushes.lib.current();
    s.apply(Action::Brush(BrushAction::Rename(one, "粉の線".into())));
    select(&mut s, b("blur"));
    s.apply(Action::Brush(BrushAction::Add));
    let two = s.brushes.lib.current();
    // 並べ替え（組み込みも動かす）
    s.apply(Action::Brush(BrushAction::Move {
        key: b("hard-round"),
        at: DropAt::End,
    }));
    select(&mut s, one);
    s.apply(Action::Brush(BrushAction::Move {
        key: one,
        at: DropAt::End,
    }));
    s.brush.radius = 31.0;
    s.apply(Action::Brush(BrushAction::Register(one)));
    // 変えただけで登録していない設定は、保存しない
    s.brush.radius = 99.0;
    let saved_order = s.toolset.set.brushes();
    // 起動し直し: 同じフォルダを読む
    let mut back = AppState::new(64, 64);
    back.attach_brush_store(dir.clone());
    assert!(back.brushes.problems.is_empty());
    assert_eq!(back.toolset.set.brushes(), saved_order);
    assert_eq!(user_names(&back).len(), 2);
    let loaded = back.brushes.lib.entry(one).unwrap();
    assert_eq!(loaded.name, "粉の線");
    assert_eq!(loaded.baseline.base.radius, 31.0, "登録した設定");
    assert_eq!(loaded.baseline.jitter.scatter, 0.5);
    assert!(loaded.baseline.tip.image.is_some(), "チョークの筆先");
    assert_eq!(
        back.brushes.lib.entry(two).unwrap().baseline.effect,
        BrushEffect::BLUR
    );
    assert!(!back.brush_is_modified(one));
    // 読んだブラシを選べば、その設定になる。次に足す番号は、読んだ番号の続き
    select(&mut back, one);
    assert_eq!(back.brush.radius, 31.0);
    back.apply(Action::Brush(BrushAction::Add));
    assert!(user_id(back.brushes.lib.current()) > user_id(two));
    // 削除は並びから外すだけ（ファイルは残り、起動し直しても並びには戻らない）。並びのファイルも直る
    let files = store::BrushStore::new(dir.clone());
    assert!(files.path_of(user_id(one)).exists());
    back.apply(Action::Brush(BrushAction::Delete(one)));
    assert!(files.path_of(user_id(one)).exists());
    let mut again = AppState::new(64, 64);
    again.attach_brush_store(dir.clone());
    assert!(again.brushes.lib.entry(one).is_some());
    assert!(!again.toolset.set.contains(one));
    assert_eq!(again.toolset.set.brushes(), back.toolset.set.brushes());
    // ファイルの削除は、ファイルを消す
    again.apply(Action::Brush(BrushAction::DeleteFile(one)));
    assert!(!files.path_of(user_id(one)).exists());
    assert!(again.brushes.lib.entry(one).is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_a_pressure_response_is_saved_with_the_brush_and_loads_into_the_live_settings() {
    use yolu_app::engine::PressureResponse;
    use yolu_core::generator::CurvePoint;
    let dir = temp_dir("pressure");
    let mut s = AppState::new(64, 64);
    s.attach_brush_store(dir.clone());
    select(&mut s, b("chalk"));
    // 応えは画面のスライダーと同じ場所（`m2.brush.pressure`）で変える。ドキュメントではないので Undo の段は作らない
    let steps = s.doc.undo_count();
    s.m2.brush.pressure.size = PressureResponse::new(0.25, vec![]).unwrap();
    s.m2.brush.pressure.opacity = PressureResponse::new(
        0.0,
        vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 0.5, y: 0.75 },
            CurvePoint { x: 1.0, y: 1.0 },
        ],
    )
    .unwrap();
    s.m2.brush.controls.pressure_hardness = true;
    s.brush_sync();
    assert!(s.brush_is_modified(b("chalk")), "応えを変えたら変更あり");
    assert_eq!(s.doc.undo_count(), steps);
    s.apply(Action::Brush(BrushAction::Add));
    let key = s.brushes.lib.current();
    // 書いたファイルは版 2。応えを使わないブラシは版 1 のまま
    let files = store::BrushStore::new(dir.clone());
    let text = std::fs::read_to_string(files.path_of(user_id(key))).unwrap();
    assert!(text.starts_with("yolupainter-brush 2\n"), "{text}");
    select(&mut s, b("blur"));
    s.apply(Action::Brush(BrushAction::Add));
    let plain = s.brushes.lib.current();
    let text = std::fs::read_to_string(files.path_of(user_id(plain))).unwrap();
    assert!(text.starts_with("yolupainter-brush 1\n"), "{text}");
    // 起動し直すと、応えが戻り、選べば今の設定に入る。別のブラシへ替えると応えも替わる
    let mut back = AppState::new(64, 64);
    back.attach_brush_store(dir.clone());
    assert!(
        back.brushes.problems.is_empty(),
        "{:?}",
        back.brushes.problems.len()
    );
    select(&mut back, key);
    assert_eq!(back.m2.brush.pressure.size.min(), 0.25);
    assert_eq!(back.m2.brush.pressure.opacity.curve().len(), 3);
    assert!(back.m2.brush.controls.pressure_hardness);
    assert!(!back.brush_is_modified(key));
    select(&mut back, plain);
    assert!(back.m2.brush.pressure.is_identity());
    assert!(!back.m2.brush.controls.pressure_hardness);
    // 元の設定へ戻す: 応えも戻る
    select(&mut back, key);
    back.m2.brush.pressure.size = PressureResponse::new(0.9, vec![]).unwrap();
    back.brush_sync();
    assert!(back.brush_is_modified(key));
    back.apply(Action::Brush(BrushAction::Revert(key)));
    assert_eq!(back.m2.brush.pressure.size.min(), 0.25);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_a_brush_with_a_pressure_response_is_left_alone_by_an_app_that_only_knows_version_one() {
    // 版 1 しか読めない古いアプリは、版 2 のファイルを「新しい形式」として理由つきで読み飛ばす（このアプリの読み手で、まだ無い版 6 を同じ形で確かめる）
    let dir = temp_dir("future-version");
    std::fs::write(
        dir.join("brush-00000001.ylbrush"),
        "yolupainter-brush 6\nname=future\ngroup=pen\npressure.size.min=0.5\n",
    )
    .unwrap();
    let mut s = AppState::new(64, 64);
    s.attach_brush_store(dir.clone());
    assert_eq!(s.brushes.problems.len(), 1);
    assert!(matches!(
        s.brushes.problems[0].reason,
        store::StoreError::NewerVersion(_)
    ));
    assert_eq!(
        std::fs::read_to_string(dir.join("brush-00000001.ylbrush")).unwrap(),
        "yolupainter-brush 6\nname=future\ngroup=pen\npressure.size.min=0.5\n",
        "触らない"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_broken_brush_files_are_skipped_and_the_reason_is_shown_in_both_languages() {
    let dir = temp_dir("broken");
    let mut s = AppState::new(64, 64);
    s.attach_brush_store(dir.clone());
    s.apply(Action::Brush(BrushAction::Add));
    let good = s.brushes.lib.current();
    let folder = dir.clone();
    // 壊れたファイル 2 つと、版の新しいファイル
    std::fs::write(folder.join("brush-00000063.ylbrush"), "ごみ").unwrap();
    std::fs::write(
        folder.join("brush-00000064.ylbrush"),
        "yolupainter-brush 1\nname=x\ngroup=pen\nradius=99999999\n",
    )
    .unwrap();
    std::fs::write(
        folder.join("brush-00000065.ylbrush"),
        "yolupainter-brush 9\nname=future\n",
    )
    .unwrap();
    let newer = std::fs::read(folder.join("brush-00000065.ylbrush")).unwrap();
    for lang in Lang::ALL {
        let mut back = AppState::new_in(64, 64, lang);
        back.attach_brush_store(dir.clone());
        // 読めるものは読む
        assert!(back.brushes.lib.entry(good).is_some());
        assert_eq!(user_names(&back).len(), 1);
        assert_eq!(back.brushes.problems.len(), 3);
        let message = back.brush_problem_message().unwrap();
        assert!(
            message.contains(lang.pick("3 件", "3 brushes")),
            "{message}"
        );
        assert!(message.contains("brush-0000006"), "{message}");
        // 理由は言語に従う（英語に日本語を出さない）
        let reason = back.brushes.problems[0].describe(lang);
        assert_eq!(
            reason.chars().any(|c| matches!(c, '\u{3040}'..='\u{9fff}')),
            lang == Lang::Ja,
            "{reason}"
        );
    }
    // 次に足すブラシの番号は、読めなかったファイルの番号より後（読めたブラシの番号は 1 なので、続きなら 2 になって読めなかった
    // ファイルと混ざる場所ではない。0x63〜0x65 のどれにも当たらず、その続きから）。足してもそのファイルには触らない
    let broken: Vec<(String, Vec<u8>)> = ["63", "64", "65"]
        .iter()
        .map(|n| {
            let name = format!("brush-000000{n}.ylbrush");
            let bytes = std::fs::read(folder.join(&name)).unwrap();
            (name, bytes)
        })
        .collect();
    assert_eq!(broken[2].1, newer);
    let mut back = AppState::new(64, 64);
    back.attach_brush_store(dir.clone());
    back.apply(Action::Brush(BrushAction::Add));
    let next = user_id(back.brushes.lib.current());
    assert_eq!(next, 0x66, "読めなかったファイルの番号の続き");
    assert_ne!(next, user_id(good));
    for (name, bytes) in &broken {
        assert_eq!(
            &std::fs::read(folder.join(name)).unwrap(),
            bytes,
            "{name} はそのまま"
        );
    }
    // 並びのファイルが壊れていても読める（元の並び）
    std::fs::write(folder.join("order.conf"), vec![b'x'; 70_000]).unwrap();
    let mut back = AppState::new(64, 64);
    back.attach_brush_store(dir.clone());
    assert!(back.brushes.problems.iter().any(|p| p.file == "order.conf"));
    assert!(back.brushes.lib.entry(good).is_some());
    std::fs::remove_dir_all(dir).unwrap();
}

/// フォルダの中のファイル名と中身の一覧（変わっていないかを比べる）。
fn snapshot_of(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect();
    files.sort();
    files
}

#[test]
fn headless_a_new_brush_never_takes_the_number_of_a_file_that_was_not_read() {
    let dir = temp_dir("ids");
    let mut s = AppState::new(64, 64);
    s.attach_brush_store(dir.clone());
    s.apply(Action::Brush(BrushAction::Add));
    let good = s.brushes.lib.current();
    assert_eq!(user_id(good), 1);
    // 読めたブラシの番号のすぐ後ろに、新しい版・壊れた・大きすぎる・フォルダの 3 種類を置く
    let newer = b"yolupainter-brush 9\nname=future\n".to_vec();
    std::fs::write(dir.join("brush-00000002.ylbrush"), &newer).unwrap();
    std::fs::write(dir.join("brush-00000003.ylbrush"), "ごみ").unwrap();
    std::fs::write(dir.join("brush-00000004.ylbrush"), vec![b'a'; 70_000]).unwrap();
    std::fs::create_dir(dir.join("brush-00000005.ylbrush")).unwrap();
    let before = snapshot_of(&dir);
    let mut back = AppState::new(64, 64);
    back.attach_brush_store(dir.clone());
    assert_eq!(user_names(&back).len(), 1);
    let mut taken = vec![1, 2, 3, 4, 5];
    for _ in 0..3 {
        back.apply(Action::Brush(BrushAction::Add));
        let id = user_id(back.brushes.lib.current());
        assert!(!taken.contains(&id), "{id} は読めなかったファイルの番号");
        assert!(id > 5, "読めなかったファイルの番号の続き: {id}");
        taken.push(id);
    }
    // 新しく書いたファイル（番号 6 以降）と、書き直す並びのファイルを除いて、前からあったブラシのファイルは 1 バイトも変わらない
    let brush_files = |files: Vec<(String, Vec<u8>)>, only: &[(String, Vec<u8>)]| {
        files
            .into_iter()
            .filter(|(name, _)| name != "order.conf" && only.iter().any(|(o, _)| o == name))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        brush_files(snapshot_of(&dir), &before),
        brush_files(before.clone(), &before)
    );
    assert!(dir.join("brush-00000005.ylbrush").is_dir());
    assert_eq!(
        std::fs::read(dir.join("brush-00000002.ylbrush")).unwrap(),
        newer
    );
    // 起動のあとに別の所（もう 1 つの起動）が置いたファイルの番号も使わない
    let next = taken.last().unwrap() + 1;
    std::fs::write(
        dir.join(format!("brush-{next:08x}.ylbrush")),
        "yolupainter-brush 9\nname=late\n",
    )
    .unwrap();
    back.apply(Action::Brush(BrushAction::Add));
    let id = user_id(back.brushes.lib.current());
    assert_eq!(id, next + 1, "置かれたファイルを飛ばす");
    assert_eq!(
        std::fs::read(dir.join(format!("brush-{next:08x}.ylbrush"))).unwrap(),
        b"yolupainter-brush 9\nname=late\n"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_when_the_brush_numbers_run_out_adding_is_refused_and_nothing_is_overwritten() {
    for readable in [false, true] {
        let dir = temp_dir(if readable {
            "ids-max-ok"
        } else {
            "ids-max-bad"
        });
        let mut s = AppState::new(64, 64);
        s.attach_brush_store(dir.clone());
        s.apply(Action::Brush(BrushAction::Add));
        let top = dir.join(format!("brush-{:08x}.ylbrush", u32::MAX));
        if readable {
            // 読める最大の番号のブラシ
            std::fs::copy(dir.join("brush-00000001.ylbrush"), &top).unwrap();
        } else {
            std::fs::write(&top, "ごみ").unwrap();
        }
        let before = snapshot_of(&dir);
        for lang in Lang::ALL {
            let mut back = AppState::new_in(64, 64, lang);
            back.attach_brush_store(dir.clone());
            let current = back.brushes.lib.current();
            let count = back.brushes.lib.entries().len();
            for _ in 0..2 {
                back.apply(Action::Brush(BrushAction::Add));
                assert_eq!(
                    back.brushes.lib.entries().len(),
                    count,
                    "{readable} {lang:?}: 足さない"
                );
                assert_eq!(back.brushes.lib.current(), current);
                assert!(
                    back.message.contains(lang.pick("使い切", "Out of")),
                    "{}",
                    back.message
                );
            }
            // 複製も同じ（番号を取るので断る）
            back.apply(Action::Brush(BrushAction::Duplicate(good_key(&back))));
            assert_eq!(back.brushes.lib.entries().len(), count);
        }
        assert_eq!(snapshot_of(&dir), before, "どのファイルも変わらない");
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// 一覧の中の利用者のブラシのどれか（読めたもの）。
fn good_key(s: &AppState) -> BrushKey {
    s.brushes
        .lib
        .entries()
        .iter()
        .find(|e| e.key.is_user())
        .map(|e| e.key)
        .unwrap_or(b(builtin::STANDARD))
}

#[test]
fn headless_a_failed_save_is_reported_and_the_brush_stays_in_the_list() {
    let dir = temp_dir("failed");
    // 保存先のフォルダの場所に、同じ名前のファイルがある（フォルダを作れない）
    let blocked = dir.join("brushes");
    std::fs::write(&blocked, "not a folder").unwrap();
    let mut s = AppState::new(64, 64);
    s.attach_brush_store(blocked.clone());
    s.apply(Action::Brush(BrushAction::Add));
    assert!(
        s.message.starts_with("ブラシを保存できません"),
        "{}",
        s.message
    );
    assert_eq!(user_names(&s), ["ブラシ"], "一覧には残る");
    assert_eq!(std::fs::read(&blocked).unwrap(), b"not a folder");
    s.lang = Lang::En;
    s.apply(Action::Brush(BrushAction::Add));
    assert!(
        s.message.starts_with("Cannot save the brush"),
        "{}",
        s.message
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_imported_tip_images_are_saved_with_the_brush_and_come_back() {
    let dir = temp_dir("imported");
    let mut s = AppState::new(64, 64);
    s.attach_brush_store(dir.clone());
    let tip = std::sync::Arc::new(
        yolu_core::BrushTip::new("取り込み", 2, 2, vec![0, 255, 255, 0]).unwrap(),
    );
    s.m2.brush.tip.image = Some(tip.clone());
    s.apply(Action::Brush(BrushAction::Add));
    assert!(
        s.message.starts_with("ブラシを追加しました"),
        "{}",
        s.message
    );
    let key = s.brushes.lib.current();
    assert!(dir.join("brush-00000001.ylbrush").exists());
    // 画像は内容の名前で 1 枚（ブラシのファイルは画像の名前を指すだけ）
    let images: Vec<_> = std::fs::read_dir(dir.join("images")).unwrap().collect();
    assert_eq!(images.len(), 1);
    // 別の起動で読み戻しても、同じ画像（名前・画素）のブラシ
    let mut again = AppState::new(64, 64);
    again.attach_brush_store(dir.clone());
    assert!(
        again.brushes.problems.is_empty(),
        "{:?}",
        again.brushes.problems
    );
    let back = again.brushes.lib.entry(key).expect("読み戻したブラシ");
    assert_eq!(back.baseline.tip.image.as_deref(), Some(&*tip));
    // ファイルを消すと、その画像のファイルも消える
    again.apply(Action::Brush(BrushAction::DeleteFile(key)));
    assert_eq!(std::fs::read_dir(dir.join("images")).unwrap().count(), 0);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_the_app_reads_the_brush_folder_next_to_its_settings() {
    let dir = temp_dir("app");
    let settings = dir.join("settings.conf");
    let ctx = egui::Context::default();
    let mut app =
        YoluApp::for_context_with_settings(&ctx, Some(settings.clone()), PenInput::detached());
    app.state.apply(Action::Brush(BrushAction::Add));
    let key = app.state.brushes.lib.current();
    assert!(dir.join("brushes").is_dir());
    let again = YoluApp::for_context_with_settings(&ctx, Some(settings), PenInput::detached());
    assert!(again.state.brushes.lib.entry(key).is_some());
    // 壊れたファイルは、起動の知らせに出る
    std::fs::write(dir.join("brushes").join("brush-00000009.ylbrush"), "x").unwrap();
    let settings = dir.join("settings.conf");
    let broken = YoluApp::for_context_with_settings(&ctx, Some(settings), PenInput::detached());
    assert!(
        broken.state.message.contains("brush-00000009.ylbrush"),
        "{}",
        broken.state.message
    );
    // 設定が無ければ保存しない
    let mut none = YoluApp::for_context_with_settings(&ctx, None, PenInput::detached());
    none.state.apply(Action::Brush(BrushAction::Add));
    assert!(none.state.brushes.store.is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

// ───────── 見本のストローク ─────────

#[test]
fn headless_samples_are_redrawn_only_for_brushes_that_changed() {
    let mut s = AppState::new(64, 64);
    let mut cache = SampleCache::default();
    let spec = SampleSpec::row(false);
    let keys = group_keys(&s, Group::Pen);
    assert!(
        keys.len() <= 2 * RENDERS_PER_FRAME,
        "試験の前提: 2 フレームで揃う"
    );
    let brush_of =
        |s: &AppState, key: BrushKey| s.brushes.lib.entry(key).unwrap().effective().clone();
    let mut frame = 0;
    let mut pass = |cache: &mut SampleCache, s: &AppState| {
        frame += 1;
        cache.begin_frame(frame);
        keys.iter()
            .filter(|k| cache.request(&brush_of(s, **k), spec).is_some())
            .count()
    };
    assert_eq!(pass(&mut cache, &s), RENDERS_PER_FRAME.min(keys.len()));
    assert_eq!(pass(&mut cache, &s), keys.len());
    let renders = cache.stats.renders;
    assert_eq!(renders as usize, keys.len());
    // 何も変えなければ描き直さない
    assert_eq!(pass(&mut cache, &s), keys.len());
    assert_eq!(cache.stats.renders, renders);
    // 変えたブラシ 1 つだけ描き直す
    select(&mut s, b("marker"));
    s.m2.brush.jitter.size = 0.4;
    s.brush_sync();
    assert_eq!(pass(&mut cache, &s), keys.len());
    assert_eq!(cache.stats.renders, renders + 1);
    // 元に戻せば、前の絵が残っている（描き直さない）
    s.apply(Action::Brush(BrushAction::Revert(b("marker"))));
    assert_eq!(pass(&mut cache, &s), keys.len());
    assert_eq!(cache.stats.renders, renders + 1);
    // 描き手の設定（手ぶれ補正）は見本に出ない。入り抜きは出る
    let mut live = brush_of(&s, b("marker"));
    live.assist.stabilizer = 80.0;
    cache.begin_frame(1000);
    cache.request(&live, spec);
    assert_eq!(cache.stats.renders, renders + 1);
    live.assist.taper_out = 40.0;
    cache.begin_frame(1001);
    cache.request(&live, spec);
    assert_eq!(cache.stats.renders, renders + 2);
}

// ───────── ブラシの行の右クリックのメニュー ─────────

mod row_menu {
    use super::*;
    use yolu_app::toolset::ui::{brush_menu, droppable_groups};
    use yolu_app::toolset::{Lock, Refusal};
    use yolu_app::ui::menu::Entry;

    /// 項目の名前（区切り・見出しは除く）。
    fn labels(entries: &[Entry<Action>]) -> Vec<String> {
        entries
            .iter()
            .filter_map(|e| e.label().map(str::to_owned))
            .collect()
    }

    fn item<'a>(entries: &'a [Entry<Action>], label: &str) -> &'a Entry<Action> {
        entries
            .iter()
            .find(|e| e.label() == Some(label))
            .unwrap_or_else(|| panic!("{label} の項目が無い: {:?}", labels(entries)))
    }

    /// (押せるか, ツールチップ)。
    fn state(entry: &Entry<Action>) -> (bool, Option<String>) {
        match entry {
            Entry::Item {
                enabled, tooltip, ..
            }
            | Entry::Submenu {
                enabled, tooltip, ..
            } => (*enabled, tooltip.clone()),
            _ => panic!("項目ではない"),
        }
    }

    fn group_items(entries: &[Entry<Action>]) -> Vec<Entry<Action>> {
        match item(entries, "グループへ移す") {
            Entry::Submenu { entries, .. } => entries.clone(),
            _ => panic!("入れ子のメニューではない"),
        }
    }

    fn menu_for(s: &mut AppState, key: BrushKey) -> Vec<Entry<Action>> {
        s.brushes.ui.context = Some(key);
        brush_menu(s)
    }

    #[test]
    fn headless_the_menu_starts_with_the_settings_and_lists_every_action_in_both_languages() {
        for lang in Lang::ALL {
            let mut s = AppState::new(64, 64);
            s.lang = lang;
            let entries = menu_for(&mut s, b("pencil"));
            assert_eq!(
                labels(&entries),
                lang.pick(
                    [
                        "ブラシの設定…",
                        "名前を変更",
                        "複製",
                        "グループへ移す",
                        "この設定で登録",
                        "元に戻す",
                        "削除"
                    ],
                    [
                        "Brush Settings…",
                        "Rename",
                        "Duplicate",
                        "Move to Group",
                        "Register These Settings",
                        "Revert",
                        "Delete"
                    ]
                ),
                "{lang:?}: 先頭がブラシの設定"
            );
            // 変更が無ければ、登録と元に戻すだけが押せない。理由はいらない（説明は置かない）
            for entry in &entries {
                let Some(name) = entry.label() else { continue };
                let modified_only = name == lang.pick("この設定で登録", "Register These Settings")
                    || name == lang.pick("元に戻す", "Revert");
                assert_eq!(state(entry), (!modified_only, None), "{lang:?} {name}");
            }
        }
    }

    #[test]
    fn headless_the_first_item_selects_that_brush_and_opens_its_detail_window() {
        let mut s = AppState::new(64, 64);
        assert_eq!(s.brushes.lib.current(), b(builtin::STANDARD));
        assert!(!s.brushes.ui.detail.open);
        let entries = menu_for(&mut s, b("pencil"));
        let Entry::Item { action, .. } = item(&entries, "ブラシの設定…") else {
            panic!()
        };
        assert_eq!(*action, Action::Brush(BrushAction::OpenDetail(b("pencil"))));
        s.apply(action.clone());
        assert_eq!(s.brushes.lib.current(), b("pencil"), "そのブラシに替わる");
        assert!(s.brushes.ui.detail.open, "ウィンドウが開く");
        // 開いたまま別のブラシのメニューから開けば、そのブラシの設定になる（ウィンドウは開いたまま）
        s.apply(Action::Brush(BrushAction::OpenDetail(b("marker"))));
        assert_eq!(s.brushes.lib.current(), b("marker"));
        assert!(s.brushes.ui.detail.open);
        // ブラシの設定は文書ではない
        assert!(!s.doc.can_undo());
    }

    #[test]
    fn headless_opening_the_detail_while_stroking_changes_nothing() {
        let mut s = AppState::new(64, 64);
        select(&mut s, b("pencil"));
        let id = s.selected_layer.unwrap();
        let stroke = s.begin_paint_stroke(id, false).unwrap();
        s.apply(Action::Brush(BrushAction::OpenDetail(b("marker"))));
        assert_eq!(s.brushes.lib.current(), b("pencil"), "替わらない");
        assert!(!s.brushes.ui.detail.open, "開かない");
        s.doc.cancel_stroke(stroke);
        s.canvas.stroke = None;
    }

    #[test]
    fn headless_the_move_list_names_the_groups_of_the_brush_tool_and_marks_its_own() {
        let mut s = AppState::new(64, 64);
        let entries = menu_for(&mut s, b("pencil"));
        // 自分のツールのグループ（別のブラシのツールは、入れ子の下）
        let groups: Vec<Entry<Action>> = group_items(&entries)
            .into_iter()
            .filter(|e| matches!(e, Entry::Item { .. }))
            .collect();
        let slot = s.toolset.set.slot_of(b("pencil")).unwrap();
        let expected: Vec<String> = s
            .toolset
            .set
            .slot(slot)
            .unwrap()
            .groups
            .iter()
            .map(|g| g.name_in(Lang::Ja))
            .collect();
        assert_eq!(
            labels(&groups),
            expected,
            "ブラシのツールのグループが、タブの順に"
        );
        let own = s.toolset.set.group_of(b("pencil")).unwrap();
        for (entry, group) in groups.iter().zip(&s.toolset.set.slot(slot).unwrap().groups) {
            let Entry::Item {
                action,
                enabled,
                check,
                ..
            } = entry
            else {
                panic!()
            };
            if group.id == own {
                assert!(!enabled, "今のグループは押せない");
                assert_eq!(*check, yolu_app::ui::menu::Check::Radio, "印だけ");
            } else {
                assert!(enabled);
                assert_eq!(
                    *action,
                    Action::Brush(BrushAction::Place {
                        key: b("pencil"),
                        group: group.id,
                        at: DropAt::End,
                        copy: false
                    })
                );
            }
        }
        // 選ぶと、そのグループの後ろへ動く（ドラッグで落としたのと同じ操作）
        let target = s.toolset.set.slot(slot).unwrap().groups[1].id;
        let Entry::Item { action, .. } = &groups[1] else {
            panic!()
        };
        s.apply(action.clone());
        assert_eq!(s.toolset.set.group_of(b("pencil")), Some(target));
        assert_eq!(
            s.toolset.set.group(target).unwrap().1.brushes.last(),
            Some(&b("pencil"))
        );
        assert!(!s.doc.can_undo(), "文書は変えない");
    }

    #[test]
    fn headless_other_brush_tools_appear_as_nested_lists() {
        let mut s = AppState::new(64, 64);
        let entries = menu_for(&mut s, b("pencil"));
        let groups = group_items(&entries);
        // 消しゴムのツールのグループは、消しゴムのツールの名前の入れ子の下に
        let nested: Vec<&Entry<Action>> = groups
            .iter()
            .filter(|e| matches!(e, Entry::Submenu { .. }))
            .collect();
        assert_eq!(nested.len(), 1);
        assert_eq!(nested[0].label(), Some("消しゴム"));
        // 消しゴムのブラシからは、ブラシのツールが入れ子になる
        let entries = menu_for(&mut s, b("soft-eraser"));
        let groups = group_items(&entries);
        assert!(groups
            .iter()
            .any(|e| matches!(e, Entry::Submenu { .. }) && e.label() == Some("ブラシ")));
    }

    #[test]
    fn headless_each_blocked_item_says_why_in_the_refusal_sentences() {
        // 描いている間: ブラシを替える・並びを変える項目は全部押せない
        let mut s = AppState::new(64, 64);
        select(&mut s, b("pencil"));
        let id = s.selected_layer.unwrap();
        let stroke = s.begin_paint_stroke(id, false).unwrap();
        for lang in Lang::ALL {
            s.lang = lang;
            let entries = menu_for(&mut s, b("pencil"));
            let why = yolu_app::lang::refusals::during_stroke(lang);
            for name in lang.pick(
                [
                    "ブラシの設定…",
                    "名前を変更",
                    "複製",
                    "グループへ移す",
                    "削除",
                ],
                [
                    "Brush Settings…",
                    "Rename",
                    "Duplicate",
                    "Move to Group",
                    "Delete",
                ],
            ) {
                assert_eq!(
                    state(item(&entries, name)),
                    (false, Some(why.to_owned())),
                    "{lang:?} 描いている間 {name}"
                );
            }
        }
        s.doc.cancel_stroke(stroke);
        s.canvas.stroke = None;

        // 並びのファイルが読めない間: 並び・数・名前を変える項目が押せない（利用者のブラシの名前の変更は、並びを変えないので押せる）
        let mut s = AppState::new(64, 64);
        s.toolset.set.locked = Some(Lock::Newer(9));
        let why = Refusal::Newer(9).describe(Lang::Ja);
        let entries = menu_for(&mut s, b("pencil"));
        for name in ["名前を変更", "複製", "グループへ移す", "削除"] {
            assert_eq!(
                state(item(&entries, name)),
                (false, Some(why.clone())),
                "{name}（組み込みのブラシ）"
            );
        }
        assert_eq!(
            state(item(&entries, "ブラシの設定…")),
            (true, None),
            "ブラシの設定は並びを変えない"
        );
        s.toolset.set.locked = None;
        s.apply(Action::Brush(BrushAction::Duplicate(b("pencil"))));
        let copy = s.brushes.lib.current();
        assert!(copy.is_user());
        s.toolset.set.locked = Some(Lock::Unreadable);
        let entries = menu_for(&mut s, copy);
        assert_eq!(state(item(&entries, "名前を変更")), (true, None));
        assert_eq!(
            state(item(&entries, "削除")),
            (false, Some(Refusal::Unreadable.describe(Lang::Ja)))
        );
    }

    #[test]
    fn headless_a_full_group_is_not_offered_and_says_so() {
        let mut s = AppState::new(64, 64);
        // 今のブラシのグループをいっぱいにする
        let slot = s.toolset.set.active().unwrap();
        let full = s.toolset.set.shown_group(slot).unwrap();
        while s.toolset.set.group(full).unwrap().1.has_room() {
            s.apply(Action::Brush(BrushAction::Add));
        }
        let other = s
            .toolset
            .set
            .slot(slot)
            .unwrap()
            .groups
            .iter()
            .find(|g| g.id != full)
            .map(|g| g.brushes[0])
            .unwrap();
        let entries = menu_for(&mut s, other);
        let groups = group_items(&entries);
        let full_name = s.toolset.set.group(full).unwrap().1.name_in(Lang::Ja);
        assert_eq!(
            state(item(&groups, &full_name)),
            (false, Some(Refusal::TooManyBrushes.describe(Lang::Ja)))
        );
        // ほかのグループは押せる。満杯のグループは、引いているときも落とせるグループに数えない
        assert!(!droppable_groups(&s, other).contains(&full));
        assert!(!droppable_groups(&s, other).is_empty());
    }

    #[test]
    fn headless_the_groups_a_dragged_brush_can_drop_on_exclude_its_own_and_a_locked_layout() {
        let mut s = AppState::new(64, 64);
        let slot = s.toolset.set.slot_of(b("pencil")).unwrap();
        let own = s.toolset.set.group_of(b("pencil")).unwrap();
        let drops = droppable_groups(&s, b("pencil"));
        assert!(
            !drops.contains(&own),
            "今のグループには、ドラッグでは落とせない"
        );
        // ブラシのツールの別のグループと、消しゴムのツールのグループ（ツールの列の上へも落とせる）
        for g in s
            .toolset
            .set
            .slot(slot)
            .unwrap()
            .groups
            .iter()
            .filter(|g| g.id != own)
        {
            assert!(drops.contains(&g.id));
        }
        s.toolset.set.locked = Some(Lock::Newer(2));
        assert!(
            droppable_groups(&s, b("pencil")).is_empty(),
            "並びが読めない間は無い"
        );
    }
}
