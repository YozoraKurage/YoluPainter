//! ツールの並び（ツールの列・ブラシのグループ・グループの中のブラシ）の操作と `tools.json` の保存・読み戻し・壊れたファイル・新しい版・
//! 今までの並びからの移し・最初の並びに戻す。画面を描かないので Wine でも回る（`headless_`）。
use std::path::{Path, PathBuf};

use yolu_app::brushes::{builtin, BrushAction, BrushKey, DropAt, Group};
use yolu_app::lang::Lang;
use yolu_app::state::{Action, AppState, Tool};
use yolu_app::toolset::catalog::{self, CatalogItem, Kind};
use yolu_app::toolset::{file, GroupId, SlotId, ToolsetAction, MAX_GROUPS, MAX_TOOLS};

fn b(id: &'static str) -> BrushKey {
    BrushKey::Builtin(id)
}

/// 試験ごとの設定のフォルダ（ブラシは `brushes/`、ツールの並びは直下の `tools.json`）。
fn settings_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tool-layout-tests")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("brushes")).unwrap();
    dir
}

fn state(dir: &Path) -> AppState {
    let mut s = AppState::new(64, 64);
    s.attach_brush_store(dir.join("brushes"));
    s
}

fn tools(s: &mut AppState, action: ToolsetAction) {
    s.apply(Action::Tools(action));
}

fn brush_op(s: &mut AppState, action: BrushAction) {
    s.apply(Action::Brush(action));
}

fn slot_of(s: &AppState, tool: Tool) -> SlotId {
    s.toolset.set.first_of(tool).unwrap()
}

fn groups(s: &AppState, slot: SlotId) -> Vec<GroupId> {
    s.toolset
        .set
        .slot(slot)
        .unwrap()
        .groups
        .iter()
        .map(|g| g.id)
        .collect()
}

fn keys(s: &AppState, group: GroupId) -> Vec<BrushKey> {
    s.toolset.set.group(group).unwrap().1.brushes.clone()
}

fn strip(s: &AppState) -> Vec<String> {
    s.toolset
        .set
        .slots()
        .iter()
        .map(|slot| slot.name_in(Lang::Ja))
        .collect()
}

#[test]
fn headless_the_initial_layout_is_the_current_tool_strip_and_brush_groups() {
    let s = AppState::new(64, 64);
    let tools: Vec<Tool> = s.toolset.set.slots().iter().map(|x| x.tool).collect();
    assert_eq!(tools, Tool::ALL);
    assert_eq!(s.toolset.set.active(), Some(slot_of(&s, Tool::Brush)));
    assert_eq!(s.shown_brush_group(), Some(Group::Pen));
    let eraser = slot_of(&s, Tool::Eraser);
    assert_eq!(
        keys(&s, groups(&s, eraser)[0])[0],
        b(builtin::STANDARD_ERASER)
    );
    // 消しゴムのツールに替えると、消しゴムのツールの最後（はじめは先頭）のブラシ
    let mut s = s;
    s.apply(Action::SelectTool(Tool::Eraser));
    assert_eq!(s.brushes.lib.current(), b(builtin::STANDARD_ERASER));
    assert_eq!(s.toolset.set.active(), Some(eraser));
}

#[test]
fn headless_a_new_brush_tool_holds_its_own_groups_and_remembers_its_brush() {
    let mut s = AppState::new(64, 64);
    let brush = slot_of(&s, Tool::Brush);
    tools(
        &mut s,
        ToolsetAction::Add {
            tool: Tool::Brush,
            after: Some(brush),
        },
    );
    let ink = s.toolset.set.active().unwrap();
    assert_ne!(ink, brush);
    assert_eq!(
        s.toolset.set.slot_index(ink),
        Some(1),
        "右クリックしたツールの後ろ"
    );
    assert_eq!(strip(&s)[1], "ブラシ 2");
    assert_eq!(s.tool, Tool::Brush);
    let group = groups(&s, ink)[0];
    assert!(keys(&s, group).is_empty());
    // ブラシの無いツールに替えても、今のブラシのまま描ける
    assert_eq!(s.brushes.lib.current(), b(builtin::STANDARD));
    // 「＋」の窓から組み込みを置く: 並びにある物は写しのファイルになる
    brush_op(
        &mut s,
        BrushAction::AddFrom(vec![
            CatalogItem::Builtin("chalk"),
            CatalogItem::Builtin("marker"),
        ]),
    );
    let placed = keys(&s, group);
    assert_eq!(placed.len(), 2);
    assert!(placed.iter().all(|k| k.is_user()), "{placed:?}");
    assert_eq!(s.brushes.lib.current(), placed[0], "置いた最初のブラシ");
    assert_eq!(s.brushes.lib.entry(placed[0]).unwrap().name, "チョーク");
    assert!(s.toolset.set.contains(b("chalk")), "元はそのまま");
    assert!(s.message.contains("2 個追加"), "{}", s.message);
    // ツールを行き来すると、それぞれ最後のブラシへ戻る
    tools(&mut s, ToolsetAction::Select(brush));
    assert_eq!(s.brushes.lib.current(), b(builtin::STANDARD));
    tools(&mut s, ToolsetAction::Select(ink));
    assert_eq!(s.brushes.lib.current(), placed[0]);
    // キーの B は、列の最初のブラシのツール（今がブラシのツールなら、そのまま）
    {
        let slot = slot_of(&s, Tool::Fill);
        tools(&mut s, ToolsetAction::Select(slot));
    }
    s.apply(Action::SelectTool(Tool::Brush));
    assert_eq!(s.toolset.set.active(), Some(brush));
    tools(&mut s, ToolsetAction::Select(ink));
    s.apply(Action::SelectTool(Tool::Brush));
    assert_eq!(s.toolset.set.active(), Some(ink));
    assert!(!s.doc.can_undo(), "文書は変えない");
}

#[test]
fn headless_tools_are_renamed_given_icons_separators_moved_and_removed() {
    let mut s = AppState::new(64, 64);
    let fill = slot_of(&s, Tool::Fill);
    let brush = slot_of(&s, Tool::Brush);
    tools(&mut s, ToolsetAction::StartRename(fill));
    tools(&mut s, ToolsetAction::Rename(fill, "  塗り  ".into()));
    assert_eq!(s.toolset.set.slot(fill).unwrap().name_in(Lang::En), "塗り");
    tools(&mut s, ToolsetAction::Rename(fill, "".into()));
    assert_eq!(
        s.toolset.set.slot(fill).unwrap().name_in(Lang::En),
        "Fill",
        "空ならツールの名前へ戻す"
    );
    tools(&mut s, ToolsetAction::SetIcon(fill, "palette"));
    assert_eq!(s.toolset.set.slot(fill).unwrap().icon().normal, "palette");
    tools(&mut s, ToolsetAction::ToggleGap(fill));
    assert!(s.toolset.set.slot(fill).unwrap().gap);
    tools(
        &mut s,
        ToolsetAction::Move {
            slot: fill,
            before: Some(brush),
        },
    );
    assert_eq!(s.toolset.set.slot_index(fill), Some(0));
    // 外す。外したツールもキーとメニューからは使え、「ツールを追加」で戻せる
    tools(&mut s, ToolsetAction::Remove(fill));
    assert!(s.toolset.set.first_of(Tool::Fill).is_none());
    assert_eq!(s.message, "ツールを削除しました: バケツ");
    s.apply(Action::SelectTool(Tool::Fill));
    assert_eq!(s.tool, Tool::Fill);
    assert_eq!(s.toolset.set.active(), None);
    tools(
        &mut s,
        ToolsetAction::Add {
            tool: Tool::Fill,
            after: None,
        },
    );
    assert_eq!(s.toolset.set.slots().last().unwrap().tool, Tool::Fill);
    // ブラシのツール・消しゴムのツールは 1 つ残す
    tools(&mut s, ToolsetAction::Remove(brush));
    assert!(s.toolset.set.slot(brush).is_some());
    assert!(s.message.contains("1 つ残します"), "{}", s.message);
    s.lang = Lang::En;
    {
        let slot = slot_of(&s, Tool::Eraser);
        tools(&mut s, ToolsetAction::Remove(slot));
    }
    assert!(s.message.contains("one eraser tool stays"), "{}", s.message);
}

#[test]
fn headless_removing_the_current_tool_moves_to_the_same_kind() {
    let mut s = AppState::new(64, 64);
    tools(
        &mut s,
        ToolsetAction::Add {
            tool: Tool::Eraser,
            after: None,
        },
    );
    let second = s.toolset.set.active().unwrap();
    assert_eq!(s.tool, Tool::Eraser);
    tools(&mut s, ToolsetAction::Remove(second));
    assert_eq!(s.toolset.set.active(), Some(slot_of(&s, Tool::Eraser)));
    assert_eq!(s.tool, Tool::Eraser);
}

#[test]
fn headless_groups_are_added_renamed_duplicated_moved_and_removed_without_deleting_files() {
    let dir = settings_dir("groups");
    let mut s = state(&dir);
    let brush = slot_of(&s, Tool::Brush);
    let eraser = slot_of(&s, Tool::Eraser);
    tools(&mut s, ToolsetAction::AddGroup(brush));
    let added = *groups(&s, brush).last().unwrap();
    assert_eq!(
        s.toolset.set.shown_group(brush),
        Some(added),
        "足したグループを出す"
    );
    tools(&mut s, ToolsetAction::RenameGroup(added, "インク".into()));
    assert_eq!(
        s.toolset.set.group(added).unwrap().1.name_in(Lang::En),
        "インク"
    );
    // ブラシを動かす（別のグループへ）・写す（Ctrl。元は動かない）
    brush_op(
        &mut s,
        BrushAction::Place {
            key: b("ink-pen"),
            group: added,
            at: DropAt::End,
            copy: false,
        },
    );
    assert_eq!(keys(&s, added), [b("ink-pen")]);
    brush_op(
        &mut s,
        BrushAction::Place {
            key: b("chalk"),
            group: added,
            at: DropAt::Before(b("ink-pen")),
            copy: true,
        },
    );
    let copy = keys(&s, added)[0];
    assert!(copy.is_user());
    assert_eq!(s.brushes.lib.entry(copy).unwrap().name, "チョーク のコピー");
    assert!(s.toolset.set.contains(b("chalk")), "元は動かない");
    // 写しを変えても、元は変わらない
    brush_op(&mut s, BrushAction::Select(copy));
    s.brush.radius = 40.0;
    brush_op(&mut s, BrushAction::Register(copy));
    assert_ne!(
        s.brushes
            .lib
            .entry(b("chalk"))
            .unwrap()
            .baseline
            .base
            .radius,
        40.0
    );
    // グループの複製は、中のブラシの写しも作る
    let users = s.brushes.lib.user_count();
    tools(&mut s, ToolsetAction::DuplicateGroup(added));
    let dup = groups(&s, brush)[groups(&s, brush).iter().position(|g| *g == added).unwrap() + 1];
    assert_eq!(keys(&s, dup).len(), 2);
    assert!(keys(&s, dup).iter().all(|k| k.is_user() && *k != copy));
    assert_eq!(s.brushes.lib.user_count(), users + 2);
    assert_eq!(
        s.toolset.set.group(dup).unwrap().1.name_in(Lang::Ja),
        "インク のコピー"
    );
    // 別のツールへ動かす（消しゴムのツールの中のブラシは消す）
    tools(
        &mut s,
        ToolsetAction::MoveGroup {
            group: dup,
            to: eraser,
            before: None,
        },
    );
    assert_eq!(s.toolset.set.slot_of_group(dup), Some(eraser));
    let moved = keys(&s, dup)[0];
    brush_op(&mut s, BrushAction::Select(moved));
    assert_eq!(s.tool, Tool::Eraser, "ブラシのあるツールへ替わる");
    // グループの削除はファイルを消さない（並びから外すだけ）
    let files_before = std::fs::read_dir(dir.join("brushes")).unwrap().count();
    tools(&mut s, ToolsetAction::RemoveGroup(dup));
    assert!(s.toolset.set.group(dup).is_none());
    assert!(s.brushes.lib.entry(moved).is_some());
    assert!(!s.toolset.set.contains(moved));
    assert_eq!(
        std::fs::read_dir(dir.join("brushes")).unwrap().count(),
        files_before
    );
    assert_ne!(
        s.brushes.lib.current(),
        moved,
        "今のブラシは今のツールのブラシへ"
    );
    // 外したファイルは「＋」の窓の自分のブラシに出て、並びにない物はそのまま戻る（写しを作らない）
    let mine = catalog::rows(&s, Kind::Mine, "");
    let row = mine
        .iter()
        .find(|r| {
            r.item
                == CatalogItem::User(match moved {
                    BrushKey::User(id) => id,
                    _ => unreachable!(),
                })
        })
        .expect("外したファイル");
    assert!(!row.placed);
    tools(&mut s, ToolsetAction::ShowGroup(added));
    tools(&mut s, ToolsetAction::Select(brush));
    let count = s.brushes.lib.user_count();
    brush_op(&mut s, BrushAction::AddFrom(vec![row.item]));
    assert_eq!(s.brushes.lib.user_count(), count);
    assert!(s.toolset.set.contains(moved));
    // 保存して読み戻しても同じ並び
    let back = state(&dir);
    assert_eq!(back.toolset.set, s.toolset.set);
    assert!(back.toolset.problem.is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_the_catalog_lists_by_kind_and_search_and_copies_what_is_placed() {
    let mut s = AppState::new(64, 64);
    let all = catalog::rows(&s, Kind::Builtin, "");
    assert_eq!(all.len(), builtin::all().len());
    assert!(all.iter().all(|r| r.placed));
    let found = catalog::rows(&s, Kind::Builtin, "鉛");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].item, CatalogItem::Builtin("pencil"));
    s.lang = Lang::En;
    assert_eq!(catalog::rows(&s, Kind::Builtin, "PENCIL").len(), 1);
    // 同梱の Krita は読み込んだあとに出る。置くと写しのファイル（出どころつき）
    let _ = yolu_app::brushes::store::krita();
    let krita = catalog::rows(&s, Kind::Krita, "");
    assert!(!krita.is_empty());
    brush_op(&mut s, BrushAction::AddFrom(vec![krita[0].item]));
    let key = s.brushes.lib.current();
    assert!(key.is_user());
    let entry = s.brushes.lib.entry(key).unwrap();
    assert_eq!(entry.name, krita[0].name);
    assert_eq!(entry.import.as_ref().unwrap().source, "Krita 4");
    assert_eq!(catalog::kind_of(entry), Kind::Mine);
    assert!(catalog::rows(&s, Kind::Mine, "").iter().any(|r| r.item
        == CatalogItem::User(match key {
            BrushKey::User(id) => id,
            _ => unreachable!(),
        })
        && r.placed));
}

#[test]
fn headless_the_layout_refuses_changes_while_importing() {
    let mut s = AppState::new(64, 64);
    let brush = slot_of(&s, Tool::Brush);
    let before = s.toolset.set.clone();
    s.brushes.import.park_next = true;
    let dir = settings_dir("busy");
    let gbr = dir.join("a.gbr");
    std::fs::write(&gbr, b"not a brush").unwrap();
    s.apply(Action::Brush(BrushAction::Import(vec![gbr])));
    assert!(s.is_brush_importing());
    for action in [
        ToolsetAction::AddGroup(brush),
        ToolsetAction::Add {
            tool: Tool::Brush,
            after: None,
        },
        ToolsetAction::Remove(slot_of(&s, Tool::Fill)),
        ToolsetAction::Reset,
    ] {
        s.message.clear();
        tools(&mut s, action.clone());
        assert!(
            s.message.contains("取り込み中"),
            "{action:?}: {}",
            s.message
        );
    }
    assert_eq!(s.toolset.set, before);
    brush_op(&mut s, BrushAction::ImportCancel);
    while s.is_brush_importing() {
        s.poll_brush_import();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_tools_json_is_written_and_read_back_and_the_old_order_is_kept_on_first_start() {
    let dir = settings_dir("first");
    // 今までの版が書いたブラシと並び（order.conf と group=）
    std::fs::write(
        dir.join("brushes/brush-00000001.ylbrush"),
        "yolupainter-brush 1\nname=線画\ngroup=pen\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("brushes/brush-00000002.ylbrush"),
        "yolupainter-brush 1\nname=消し\ngroup=eraser\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("brushes/order.conf"),
        "u:1\nb:standard\nb:pencil\n",
    )
    .unwrap();
    let mut s = state(&dir);
    assert!(s.toolset.problem.is_none());
    let brush = slot_of(&s, Tool::Brush);
    let pen = groups(&s, brush)[0];
    assert_eq!(
        &keys(&s, pen)[..3],
        [BrushKey::User(1), b("standard"), b("pencil")]
    );
    let eraser = slot_of(&s, Tool::Eraser);
    assert_eq!(
        *keys(&s, groups(&s, eraser)[0]).last().unwrap(),
        BrushKey::User(2)
    );
    assert!(!dir.join("tools.json").exists(), "変えるまで書かない");
    // 変えると tools.json を書く。order.conf はもう書かない（消さない）
    tools(&mut s, ToolsetAction::AddGroup(brush));
    let text = std::fs::read_to_string(dir.join("tools.json")).unwrap();
    assert!(text.contains("\"format\": \"yolupainter-tools\""), "{text}");
    assert!(text.contains("\"user_brushes_through\": 2"), "{text}");
    assert_eq!(
        std::fs::read_to_string(dir.join("brushes/order.conf")).unwrap(),
        "u:1\nb:standard\nb:pencil\n"
    );
    let back = state(&dir);
    assert_eq!(back.toolset.set, s.toolset.set);
    // 並びを書いたあとに古い版が足したファイル（番号が大きい）は、元のグループの最後へ
    std::fs::write(
        dir.join("brushes/brush-00000003.ylbrush"),
        "yolupainter-brush 1\nname=あとから\ngroup=brush\n",
    )
    .unwrap();
    let later = state(&dir);
    let brush_group = groups(&later, slot_of(&later, Tool::Brush))[1];
    assert_eq!(
        *keys(&later, brush_group).last().unwrap(),
        BrushKey::User(3)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_a_broken_tools_json_is_set_aside_and_the_initial_layout_is_used() {
    for (lang, expect) in [
        (Lang::Ja, "最初の並びにしました（JSON ではありません）"),
        (Lang::En, "using the initial layout (not JSON)"),
    ] {
        let dir = settings_dir("broken");
        std::fs::write(dir.join("tools.json"), "{ not json").unwrap();
        std::fs::write(dir.join("tools.broken.json"), "older").unwrap();
        let mut s = AppState::new(64, 64);
        s.lang = lang;
        s.attach_brush_store(dir.join("brushes"));
        let message = s.toolset_problem_message().expect("知らせ");
        assert!(message.contains(expect), "{message}");
        assert!(message.contains("tools.broken-2.json"), "{message}");
        assert!(!dir.join("tools.json").exists());
        assert_eq!(
            std::fs::read_to_string(dir.join("tools.broken-2.json")).unwrap(),
            "{ not json",
            "元のファイルは消さずに退避"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("tools.broken.json")).unwrap(),
            "older",
            "前に退避した物も残す"
        );
        assert_eq!(s.toolset.set, AppState::new(64, 64).toolset.set);
        // 変えられる（新しい tools.json を書く）
        {
            let slot = slot_of(&s, Tool::Brush);
            tools(&mut s, ToolsetAction::AddGroup(slot));
        }
        assert!(dir.join("tools.json").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn headless_a_newer_tools_json_is_left_alone_and_changes_are_refused() {
    let dir = settings_dir("newer");
    let text = file::encode(&AppState::new(64, 64).toolset.set, 0)
        .replace("\"version\": 1", "\"version\": 9");
    std::fs::write(dir.join("tools.json"), &text).unwrap();
    let mut s = state(&dir);
    let message = s.toolset_problem_message().unwrap();
    assert!(message.contains("新しい版"), "{message}");
    let brush = slot_of(&s, Tool::Brush);
    s.message.clear();
    tools(&mut s, ToolsetAction::AddGroup(brush));
    assert!(s.message.contains("新しい版"), "{}", s.message);
    brush_action_refused(&mut s, BrushAction::Delete(b("pencil")));
    brush_action_refused(&mut s, BrushAction::Add);
    tools(&mut s, ToolsetAction::Reset);
    assert_eq!(
        std::fs::read_to_string(dir.join("tools.json")).unwrap(),
        text
    );
    // ツールとブラシは今までどおり使える
    s.apply(Action::SelectTool(Tool::Eraser));
    assert_eq!(s.tool, Tool::Eraser);
    std::fs::remove_dir_all(dir).unwrap();
}

fn brush_action_refused(s: &mut AppState, action: BrushAction) {
    let before = s.toolset.set.clone();
    s.message.clear();
    brush_op(s, action.clone());
    assert!(s.message.contains("新しい版"), "{action:?}: {}", s.message);
    assert_eq!(s.toolset.set, before);
}

#[test]
fn headless_a_new_built_in_tool_is_added_at_the_end_of_a_saved_strip() {
    let dir = settings_dir("new-tool");
    let mut s = state(&dir);
    {
        let slot = slot_of(&s, Tool::Ruler);
        tools(&mut s, ToolsetAction::Remove(slot));
    }
    // 新しい版で足されたツールの代わりに、ゆがみのツールを「書いたときに知らなかった」ツールにする（列にも known にも無い）
    {
        let slot = slot_of(&s, Tool::Liquify);
        tools(&mut s, ToolsetAction::Remove(slot));
    }
    let text = std::fs::read_to_string(dir.join("tools.json")).unwrap();
    assert!(text.contains("    \"liquify\",\n"), "{text}");
    std::fs::write(
        dir.join("tools.json"),
        text.replace("    \"liquify\",\n", ""),
    )
    .unwrap();
    let back = state(&dir);
    assert!(back.toolset.problem.is_none(), "{:?}", back.toolset.problem);
    assert_eq!(back.toolset.set.slots().last().unwrap().tool, Tool::Liquify);
    assert!(
        back.toolset.set.first_of(Tool::Ruler).is_none(),
        "外したツールは戻らない"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_resetting_puts_every_brush_file_back_in_its_original_group() {
    let dir = settings_dir("reset");
    let mut s = state(&dir);
    let brush = slot_of(&s, Tool::Brush);
    select_builtin_and_add(&mut s, "chalk");
    let mine = s.brushes.lib.current();
    tools(
        &mut s,
        ToolsetAction::Add {
            tool: Tool::Eraser,
            after: None,
        },
    );
    tools(&mut s, ToolsetAction::Rename(brush, "えんぴつ".into()));
    brush_op(&mut s, BrushAction::Delete(mine));
    brush_op(&mut s, BrushAction::Delete(b("marker")));
    {
        let slot = slot_of(&s, Tool::Lasso);
        tools(&mut s, ToolsetAction::Remove(slot));
    }
    tools(&mut s, ToolsetAction::Reset);
    let tools_now: Vec<Tool> = s.toolset.set.slots().iter().map(|x| x.tool).collect();
    assert_eq!(tools_now, Tool::ALL);
    assert_eq!(strip(&s)[0], "ブラシ");
    assert!(s.toolset.set.contains(b("marker")));
    let place = s
        .toolset
        .set
        .find(mine)
        .expect("ファイルは消さずに元のグループへ");
    assert_eq!(
        s.toolset.set.slots()[place.slot].groups[place.group].builtin,
        Some(Group::Brush)
    );
    assert!(s.message.contains("最初の並び"), "{}", s.message);
    let back = state(&dir);
    assert_eq!(back.toolset.set, s.toolset.set);
    std::fs::remove_dir_all(dir).unwrap();
}

fn select_builtin_and_add(s: &mut AppState, id: &'static str) {
    brush_op(s, BrushAction::Select(b(id)));
    brush_op(s, BrushAction::Add);
}

#[test]
fn headless_limits_of_the_strip_and_the_groups_are_kept() {
    let mut s = AppState::new(64, 64);
    while s.toolset.set.slots().len() < MAX_TOOLS {
        tools(
            &mut s,
            ToolsetAction::Add {
                tool: Tool::Brush,
                after: None,
            },
        );
    }
    s.message.clear();
    tools(
        &mut s,
        ToolsetAction::Add {
            tool: Tool::Brush,
            after: None,
        },
    );
    assert_eq!(s.toolset.set.slots().len(), MAX_TOOLS);
    assert!(s.message.contains(&MAX_TOOLS.to_string()), "{}", s.message);
    let brush = slot_of(&s, Tool::Brush);
    while groups(&s, brush).len() < MAX_GROUPS {
        tools(&mut s, ToolsetAction::AddGroup(brush));
    }
    s.message.clear();
    tools(&mut s, ToolsetAction::AddGroup(brush));
    assert_eq!(groups(&s, brush).len(), MAX_GROUPS);
    assert!(s.message.contains(&MAX_GROUPS.to_string()), "{}", s.message);
}

/// 試験用の利用者のブラシのファイル（番号 `id`・元のグループ `group`）。
fn user_file(dir: &Path, id: u32, group: &str) {
    std::fs::write(
        dir.join(format!("brushes/brush-{id:08x}.ylbrush")),
        format!("yolupainter-brush 1\nname=ブラシ {id}\ngroup={group}\n"),
    )
    .unwrap();
}

fn imported_sizes(s: &AppState) -> Vec<usize> {
    s.toolset
        .set
        .slot(slot_of(s, Tool::Brush))
        .unwrap()
        .groups
        .iter()
        .filter(|g| g.builtin == Some(Group::Imported))
        .map(|g| g.file_len())
        .collect()
}

#[test]
fn headless_more_brush_files_than_one_group_holds_continue_in_the_next_group_and_the_layout_saves()
{
    let dir = settings_dir("overflow");
    let count = yolu_app::toolset::MAX_GROUP_BRUSHES + 1;
    for id in 1..=count as u32 {
        user_file(&dir, id, "imported");
    }
    // tools.json が無い最初の起動（今までの並びからの移し）: 512 個で次の取り込みのグループへ
    let mut s = state(&dir);
    assert_eq!(
        imported_sizes(&s),
        [yolu_app::toolset::MAX_GROUP_BRUSHES, 1]
    );
    assert_eq!(
        s.toolset.set.brushes().len(),
        builtin::all().len() + count,
        "どのファイルも並びにある"
    );
    // 並びを変えて保存できる（保存の読み戻しが同じになる）
    let brush = slot_of(&s, Tool::Brush);
    s.message.clear();
    tools(&mut s, ToolsetAction::AddGroup(brush));
    assert!(!s.message.contains("保存できません"), "{}", s.message);
    assert!(dir.join("tools.json").exists(), "並びを保存した");
    let back = state(&dir);
    assert!(back.toolset.problem.is_none(), "{:?}", back.toolset.problem);
    assert_eq!(back.toolset.set, s.toolset.set);
    assert_eq!(
        imported_sizes(&back),
        [yolu_app::toolset::MAX_GROUP_BRUSHES, 1]
    );
    // 最初の並びに戻しても同じ形で、保存できる
    s.message.clear();
    tools(&mut s, ToolsetAction::Reset);
    assert!(!s.message.contains("保存できません"), "{}", s.message);
    assert_eq!(
        imported_sizes(&s),
        [yolu_app::toolset::MAX_GROUP_BRUSHES, 1]
    );
    let back = state(&dir);
    assert!(back.toolset.problem.is_none(), "{:?}", back.toolset.problem);
    assert_eq!(back.toolset.set, s.toolset.set);
    assert_eq!(
        back.toolset.set.brushes().len(),
        builtin::all().len() + count
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_a_brush_file_that_cannot_be_read_keeps_its_place_in_tools_json_until_it_can_be_read() {
    let dir = settings_dir("unread");
    user_file(&dir, 1, "pen");
    user_file(&dir, 2, "brush");
    user_file(&dir, 3, "brush");
    let mut s = state(&dir);
    let brush = slot_of(&s, Tool::Brush);
    tools(&mut s, ToolsetAction::AddGroup(brush));
    let before = s.toolset.set.find(BrushKey::User(2)).expect("並びにある");
    assert_eq!(before.group, 1, "筆のグループ");
    // 2 が読めなくなり、3 はフォルダから無くなった（利用者が消した）
    std::fs::write(
        dir.join("brushes/brush-00000002.ylbrush"),
        "yolupainter-brush 99\n",
    )
    .unwrap();
    std::fs::remove_file(dir.join("brushes/brush-00000003.ylbrush")).unwrap();
    // 並びのファイルには、この版が知らない組み込みのブラシの札もある
    let text = std::fs::read_to_string(dir.join("tools.json")).unwrap();
    std::fs::write(
        dir.join("tools.json"),
        text.replacen("\"b:standard\"", "\"b:standard\", \"b:from-the-future\"", 1),
    )
    .unwrap();
    let mut s = state(&dir);
    assert!(s.toolset.problem.is_none(), "{:?}", s.toolset.problem);
    assert!(
        !s.toolset.set.contains(BrushKey::User(2)),
        "画面には出さない"
    );
    assert_eq!(s.brushes.problems.len(), 1, "読めなかったファイルの知らせ");
    // 並びを変えて書き直しても、読めなかった札と知らない札は元の場所に残り、フォルダに無い札は消える
    let brush = slot_of(&s, Tool::Brush);
    tools(&mut s, ToolsetAction::AddGroup(brush));
    tools(&mut s, ToolsetAction::Rename(brush, "線画".into()));
    let text = std::fs::read_to_string(dir.join("tools.json")).unwrap();
    assert!(text.contains("\"u:2\""), "{text}");
    assert!(text.contains("\"b:standard\",\n            \"b:from-the-future\""));
    assert!(
        !text.contains("\"u:3\""),
        "フォルダに無い物は覚えない\n{text}"
    );
    // 読めるようになった起動では、同じグループの同じ場所に出る
    user_file(&dir, 2, "pen");
    let back = state(&dir);
    assert!(back.toolset.problem.is_none(), "{:?}", back.toolset.problem);
    assert!(back.brushes.problems.is_empty());
    let after = back.toolset.set.find(BrushKey::User(2)).expect("元の場所");
    assert_eq!((after.slot, after.group), (before.slot, before.group));
    assert_eq!(after.index, before.index);
    assert!(
        !back.toolset.set.contains(BrushKey::User(3)),
        "消えたファイルは戻らない"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
