//! 設定のウィンドウの区分と探す欄、メニューの置き場（ファイル・表示・ヘルプ）。メニューから外した操作が、今までどおり動いて、設定のウィンドウから行けること。
//! 区分ごとの絵と、メニューの絵は日英（ウィンドウの中だけ・メニューの中だけを撮る）。`headless_` で始まる試験は画面を描かず、Wine でも回る。
use crate::common;

use std::path::{Path, PathBuf};

use common::*;
use egui::{vec2, Rect};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use yolu_app::lang::Lang;
use yolu_app::pen::PenInput;
use yolu_app::prefs::{self, Category, Item, PrefsAction};
use yolu_app::shell;
use yolu_app::state::{Action, AppState};
use yolu_app::ui::menu::Entry;
use yolu_app::YoluApp;

type H = Harness<'static, YoluApp>;

fn labels(entries: &[Entry<Action>]) -> Vec<String> {
    yolu_app::ui::menu::leaves(entries)
        .iter()
        .filter_map(|e| e.label().map(str::to_owned))
        .collect()
}

fn has(entries: &[Entry<Action>], action: &Action) -> bool {
    yolu_app::ui::menu::leaves(entries)
        .iter()
        .any(|e| matches!(e, Entry::Item { action: a, .. } if a == action))
}

// ───────── メニューの置き場 ─────────

/// ファイルには、「3D ビューに FBX を開く…」と、押せない見出し（読み込み・書き出し）が無い（モデルは、新規プロジェクト・プロジェクト設定・ドロップ・ポーズの欄で開く）。
/// 表示には、言語・筆圧の調整・FBX・ベイクが無い。ヘルプには、ショートカット・起動時の確かめ・試験版が無い（日英）。
#[test]
fn headless_the_menus_have_the_items_in_the_new_places_in_both_languages() {
    use yolu_app::pen::window::PressureAction;
    use yolu_app::update::UpdateAction;
    use yolu_app::view3d::pose::PoseAction;
    for lang in Lang::ALL {
        let mut app = AppState::new(64, 64);
        app.lang = lang;
        let file = shell::menu_entries(&app, 0);
        let names = labels(&file);
        let at = |name: &str| names.iter().position(|n| n == name).unwrap_or(usize::MAX);
        assert_eq!(
            at(lang.pick("復旧…", "Recovery…")),
            at(lang.pick("開く…", "Open…")) + 1,
            "{lang:?}: {names:?}"
        );
        // FBX を開く項目はメニューに無い（操作は `Action` として残る。下の試験）
        assert!(!has(&file, &Action::Pose(PoseAction::OpenFbx)), "{lang:?}");
        let fbx = lang.pick("3D ビューに FBX を開く…", "Open an FBX in the 3D View…");
        assert_eq!(at(fbx), usize::MAX, "{lang:?}: {names:?}");
        // 押せない見出しは置かず、区切り線で分ける（Live Link の状態の見出しは別のメニュー）
        assert!(
            !file.iter().any(|e| matches!(e, Entry::Heading(_))),
            "{lang:?}: ファイルに見出し {names:?}"
        );
        // 「読み込み」「書き出し」の見出しは無い。インポートは入れ子のメニューで、中に PSD の 2 つ
        for gone in [
            lang.pick("読み込み", "Import"),
            lang.pick("書き出し", "Export"),
        ] {
            assert_eq!(at(gone), usize::MAX, "{lang:?}: {gone}");
        }
        let import = file
            .iter()
            .position(|e| matches!(e, Entry::Submenu { label, .. } if label == lang.pick("インポート", "Import")))
            .unwrap_or_else(|| panic!("{lang:?}: インポートの入れ子のメニュー {names:?}"));
        let Entry::Submenu { entries, .. } = &file[import] else {
            unreachable!()
        };
        assert_eq!(
            labels(entries),
            [
                lang.pick(
                    "PSD を新しいテクスチャセットに…",
                    "PSD as a New Texture Set…"
                ),
                lang.pick(
                    "PSD を今のテクスチャセットに…",
                    "PSD into the Current Texture Set…"
                )
            ],
            "{lang:?}"
        );
        assert!(has(
            entries,
            &Action::Psd(yolu_app::psd::PsdAction::ImportDialog(
                yolu_app::psd::PsdTarget::NewSet
            ))
        ));
        assert!(has(
            entries,
            &Action::Psd(yolu_app::psd::PsdAction::ImportDialog(
                yolu_app::psd::PsdTarget::CurrentSet
            ))
        ));
        // 「今のセットの文書」の言い方は画面に無い
        assert!(
            !names
                .iter()
                .any(|n| n.contains("セットの文書") || n.contains("Set's Document")),
            "{lang:?}: {names:?}"
        );
        // 並び: 配布用に保存… | インポート ▸・テクスチャを書き出す…・PSD を書き出す… | プロジェクト設定… | Live Link | 終了
        let export = at(lang.pick("テクスチャを書き出す…", "Export Textures…"));
        assert_eq!(
            at(lang.pick("PSD を書き出す…", "Export PSD…")),
            export + 1,
            "{lang:?}: {names:?}"
        );
        // （入れ子の中の PSD の 2 つを挟んで、配布用に保存…の 3 つ先）
        assert_eq!(
            export,
            at(lang.pick("配布用に保存…", "Save for Distribution…")) + 3,
            "{lang:?}: {names:?}"
        );
        let project = at(lang.pick("プロジェクト設定…", "Project Configuration…"));
        assert_eq!(project, export + 2, "{lang:?}: 書き出しのすぐ後 {names:?}");
        assert_eq!(
            at("Live Link"),
            project + 1,
            "{lang:?}: Live Link のすぐ上 {names:?}"
        );
        assert_eq!(
            at(lang.pick("終了", "Quit")),
            at("Live Link") + 1,
            "{lang:?}: {names:?}"
        );
        // インポートの前、プロジェクト設定の前後は区切り線（書き出しの 2 つはインポートに続く）
        assert!(matches!(file[import - 1], Entry::Separator), "{lang:?}");
        assert_eq!(
            file[import + 1].label(),
            Some(lang.pick("テクスチャを書き出す…", "Export Textures…")),
            "{lang:?}"
        );
        let project_at = file
            .iter()
            .position(|e| {
                e.label() == Some(lang.pick("プロジェクト設定…", "Project Configuration…"))
            })
            .expect("項目がある");
        assert!(matches!(file[project_at - 1], Entry::Separator), "{lang:?}");
        assert!(matches!(file[project_at + 1], Entry::Separator), "{lang:?}");
        let view = shell::menu_entries(&app, 5);
        for gone in [
            Action::Pose(PoseAction::OpenFbx),
            Action::Pressure(PressureAction::Open),
            Action::Bake(yolu_app::bake::BakeAction::OpenWindow),
            Action::M2Ui(yolu_app::m2::UiOp::Language(Lang::Ja)),
            Action::M2Ui(yolu_app::m2::UiOp::Language(Lang::En)),
        ] {
            assert!(!has(&view, &gone), "{lang:?}: 表示に {gone:?}");
        }
        let view_names = labels(&view);
        for gone in [
            "言語",
            "Language",
            "日本語",
            "English",
            "筆圧の調整…",
            "Pen Pressure…",
            "メッシュマップをベイク…",
            "Bake Mesh Maps…",
        ] {
            assert!(
                !view_names.iter().any(|n| n == gone)
                    && !view
                        .iter()
                        .any(|e| matches!(e, Entry::Heading(h) if h == gone)),
                "{lang:?}: 表示に「{gone}」 {view_names:?}"
            );
        }
        // 表示の最後は区切りで終わらない
        assert!(!matches!(view.last(), Some(Entry::Separator)), "{lang:?}");
        // ヘルプ: 公開鍵の有無によらず、ショートカットは無い
        let help = shell::menu_entries(&app, shell::HELP_MENU);
        assert!(!has(&help, &Action::ShowShortcuts), "{lang:?}");
        assert_eq!(
            labels(&help),
            [
                lang.pick("ログのフォルダを開く", "Open Log Folder"),
                lang.pick("YoluPainter について", "About YoluPainter")
            ],
            "{lang:?}"
        );
        for action in [
            UpdateAction::SetCheckOnStartup(true),
            UpdateAction::SetCheckOnStartup(false),
            UpdateAction::SetBeta(true),
            UpdateAction::SetBeta(false),
        ] {
            assert!(!has(&help, &Action::Update(action)), "{lang:?}");
        }
    }
}

/// メニューから外した操作は、今までどおり `Action` として動く: 言語は選べて、ショートカットと筆圧の調整は設定のウィンドウをその区分で開く。
/// キーの表・パイ・MCP の操作の名前は、外した操作を指していない（指していれば、そのまま効く）。
#[test]
fn headless_the_actions_taken_out_of_the_menus_still_run_and_open_their_categories() {
    use yolu_app::pen::window::PressureAction;
    use yolu_app::state::DialogRequest;
    use yolu_app::view3d::pose::PoseAction;
    let mut app = AppState::new(64, 64);
    // 言語
    app.apply(Action::M2Ui(yolu_app::m2::UiOp::Language(Lang::En)));
    assert_eq!(app.lang, Lang::En);
    app.apply(Action::M2Ui(yolu_app::m2::UiOp::Language(Lang::Ja)));
    assert_eq!(app.lang, Lang::Ja);
    // ショートカット・筆圧の調整
    assert!(!app.prefs.open);
    app.apply(Action::ShowShortcuts);
    assert!(app.prefs.shows(Category::Shortcuts));
    app.apply(Action::Prefs(PrefsAction::Close));
    app.apply(Action::Pressure(PressureAction::Open));
    assert!(app.prefs.open);
    assert_eq!(app.prefs.category, Category::Pen);
    // FBX を開く
    app.apply(Action::Pose(PoseAction::OpenFbx));
    assert_eq!(app.dialog_request, Some(DialogRequest::OpenModel));
    // 外した物の名前はキーの表にも無い（キーの表の操作は `commands` の ID。ショートカット・言語・筆圧の調整・FBX の ID は持たない。
    // ベイクだけは、メニューから外した代わりに `bake.open` を持つ（下の `headless_bake_can_be_opened_…`）
    for command in yolu_app::commands::all() {
        let id = command.id;
        assert!(
            !["shortcut", "language", "pressure", "fbx"]
                .iter()
                .any(|word| id.contains(word)),
            "{id}"
        );
    }
}

// ───────── ベイクの入口 ─────────

/// 「メッシュマップをベイク…」は、既定のキーの無い操作 `bake.open`（キー・パイから呼べる）で、セットの右クリックのメニューにもある。
/// テクスチャセットの帯のボタンは、帯が狭いと隠れるので、表示のメニューから外した入口の代わりになる。
#[test]
fn headless_bake_can_be_opened_by_a_command_without_a_default_key_and_from_the_set_menu() {
    use yolu_app::bake::BakeAction;
    use yolu_app::keymap::Scope;
    use yolu_app::pie::PieItem;
    use yolu_app::state::PopupKind;
    for lang in Lang::ALL {
        let mut app = AppState::new(64, 64);
        app.lang = lang;
        // 操作: 名前・実行する Action・既定のキーが無い・キーの設定の表に載る（キーを割り当てられる）
        let command = yolu_app::commands::find("bake.open").expect("操作 bake.open");
        assert_eq!(
            command.label(&app).as_deref(),
            Some(lang.pick("メッシュマップをベイク…", "Bake Mesh Maps…"))
        );
        assert_eq!(
            command.runnable(),
            Some(Action::Bake(BakeAction::OpenWindow))
        );
        assert!(
            yolu_app::keymap::bindings()
                .iter()
                .all(|b| b.command != "bake.open"),
            "既定のキーは無い"
        );
        assert!(yolu_app::shortcuts::editor::all_groups(&app)
            .contains(&("bake.open", Scope::Everywhere)));
        // パイから: 項目として実行できる
        assert!(app.bake.window.is_none());
        yolu_app::pie::run(&mut app, &PieItem::Command("bake.open".into()));
        assert!(
            app.bake.window.is_some(),
            "パイの項目でベイクのウィンドウが開く"
        );
        // セットの右クリックのメニューに入口がある
        let uid = app.sets.current().uid;
        let entries = shell::popup_entries(&app, PopupKind::SetContext(uid));
        assert!(
            has(&entries, &Action::Bake(BakeAction::OpenWindow)),
            "{lang:?}"
        );
        assert!(labels(&entries)
            .iter()
            .any(|l| l == lang.pick("メッシュマップをベイク…", "Bake Mesh Maps…")));
    }
}

/// テクスチャセットの行を右クリックして「メッシュマップをベイク…」を選ぶと、ベイクのウィンドウが開く。操作に割り当てたキーでも開く。
#[test]
fn the_set_context_menu_and_an_assigned_key_open_the_bake_window() {
    use yolu_app::keymap::{Scope, Trigger};
    let dir = settings_dir("bake-entrance");
    let mut h = app_with_settings(&dir, vec2(1280.0, 800.0));
    h.state_mut()
        .state
        .apply(Action::Project(yolu_app::newproject::NpAction::AddSet));
    h.run();
    let row = h.get_by_label("テクスチャセット 1").rect().center();
    press(&h, row, egui::PointerButton::Secondary);
    h.step();
    release(&h, row, egui::PointerButton::Secondary);
    h.run();
    let item = popup_item(&h, "メッシュマップをベイク…").center();
    click(&mut h, item);
    assert!(
        h.state().state.bake.window.is_some(),
        "右クリックのメニューから開く"
    );
    // 閉じて、キーを割り当てると、そのキーで開く
    h.state_mut().state.bake.window = None;
    h.state_mut().state.keys.set(
        ("bake.open", Scope::Everywhere),
        vec![Trigger::Key {
            modifiers: egui::Modifiers::COMMAND | egui::Modifiers::ALT,
            key: egui::Key::B,
        }],
    );
    h.state_mut().state.keys_changed();
    h.run();
    assert!(!h.state().state.keys.has_conflicts());
    key(
        &h,
        egui::Key::B,
        egui::Modifiers::COMMAND | egui::Modifiers::ALT,
    );
    h.run();
    assert!(
        h.state().state.bake.window.is_some(),
        "割り当てたキーで開く"
    );
}

// ───────── 設定のウィンドウ ─────────

fn settings_dir(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/settings-window-tests")
        .join(std::process::id().to_string())
        .join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn app_with_settings(dir: &Path, size: egui::Vec2) -> H {
    let path = dir.join("YoluPainter").join("settings.conf");
    let mut h = common::gpu_thread::builder()
        .with_size(size)
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .renderer(common::shared_gpu::renderer())
        .build_eframe(move |cc| {
            YoluApp::for_context_with_settings(&cc.egui_ctx, Some(path), PenInput::detached())
                .with_render_state(cc.wgpu_render_state.as_ref())
        });
    h.state_mut().state.prefs.ram_mib = 16384;
    h.state_mut().state.prefs.cores = 8;
    // PC によらない値にする（絵に出る置き場所と空き）
    h.state_mut().state.prefs.settings.library_folder = Some(PathBuf::from(if cfg!(windows) {
        "C:\\Library"
    } else {
        "/Library"
    }));
    h.state_mut().state.prefs.settings.disk_cache_folder = Some(PathBuf::from(if cfg!(windows) {
        "C:\\Cache"
    } else {
        "/Cache"
    }));
    let folder = h.state().state.prefs.settings.disk_cache_folder();
    h.state_mut()
        .state
        .prefs
        .cache_free
        .push((folder, Some(100 * 1024 * 1024 * 1024)));
    h.run();
    h
}

fn window(h: &H) -> Rect {
    prefs::last_rect(&h.ctx).expect("設定のウィンドウが開いている")
}

/// 左の区分の矩形（ウィンドウの左の列にある、その名前のもの）。
fn side(h: &H, label: &str) -> Rect {
    let w = window(h);
    rect_of(h, label, |r| {
        w.contains_rect(r) && r.left() < w.left() + 220.0
    })
}

fn click_side(h: &mut H, category: Category) {
    let label = category.label(h.state().state.lang);
    let at = side(h, label).center();
    click(h, at);
}

fn type_in_search(h: &mut H, text: &str) {
    let w = window(h);
    // 上の探す欄（ウィンドウの右の欄の頭にある文字の欄）
    let field = h
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .map(|n| n.rect())
        .find(|r| r.top() < w.top() + 80.0 && w.contains_rect(*r))
        .expect("探す欄");
    click(h, field.center());
    key(h, egui::Key::A, egui::Modifiers::COMMAND);
    h.event(egui::Event::Text(text.to_owned()));
    h.run();
}

/// 設定のウィンドウが描いた文字（ウィンドウの地を描いたあとの文字だけ。後ろのパネルの文字は入れない）。
fn window_texts(h: &H) -> Vec<(String, Rect)> {
    use egui::epaint::Shape;
    let window = window(h);
    let shapes = &h.output().shapes;
    let start = shapes
        .iter()
        .rposition(|s| matches!(&s.shape, Shape::Rect(r) if r.rect == window))
        .expect("ウィンドウの地を描いた");
    let mut out = Vec::new();
    fn walk(shape: &Shape, clip: Rect, out: &mut Vec<(String, Rect)>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, clip, out)),
            Shape::Text(t) => {
                let r = t.galley.rect.translate(t.pos.to_vec2());
                out.push((t.galley.job.text.clone(), r.intersect(clip)));
            }
            _ => {}
        }
    }
    for s in &shapes[start..] {
        walk(&s.shape, s.clip_rect, &mut out);
    }
    out
}

fn open(h: &mut H) {
    h.state_mut().state.apply(Action::Prefs(PrefsAction::Open));
    h.run();
}

fn lit(h: &H, label: &str) -> bool {
    // 光っている区分は、読み上げで「選ばれた」印が付く
    let w = window(h);
    h.query_all_by_label(label)
        .find(|n| w.contains_rect(n.rect()) && n.rect().left() < w.left() + 220.0)
        .is_some_and(|n| format!("{:?}", n.accesskit_node().toggled()) == "Some(True)")
}

/// 左の区分は 10 個（更新は公開鍵を組み込んだビルドだけ）。押すと右の欄が替わり、前に開いた区分は閉じても次の開き方で覚えている。
#[test]
fn the_sidebar_switches_the_categories_and_the_window_remembers_the_last_one() {
    let dir = settings_dir("sidebar");
    let mut h = app_with_settings(&dir, vec2(1280.0, 800.0));
    open(&mut h);
    assert_eq!(h.state().state.prefs.category, Category::General);
    let w = window(&h);
    // 区分の名前は左の列に並ぶ（更新は、公開鍵の無いこのビルドでは出ない）
    let mut last_top = f32::MIN;
    for category in Category::ALL
        .into_iter()
        .filter(|c| *c != Category::Updates)
    {
        let r = side(&h, category.label(Lang::Ja));
        assert!(r.top() > last_top, "{category:?} は上から順");
        last_top = r.top();
    }
    assert!(h
        .query_all_by_label("更新")
        .all(|n| !w.contains_rect(n.rect())));
    // 押すと右の欄が替わる（その区分の行が出て、前の区分の行は消える）
    click_side(&mut h, Category::Memory);
    assert_eq!(h.state().state.prefs.category, Category::Memory);
    let _ = h.get_by_label("取り消し履歴: 自動（2048 MiB）");
    assert!(h.query_by_label("言語: 日本語").is_none());
    click_side(&mut h, Category::General);
    let _ = h.get_by_label("言語: 日本語");
    assert!(h.query_by_label("取り消し履歴: 自動（2048 MiB）").is_none());
    // 閉じて開くと、前に開いた区分
    click_side(&mut h, Category::View3d);
    h.state_mut().state.apply(Action::Prefs(PrefsAction::Close));
    h.run();
    assert!(prefs::last_rect(&h.ctx).is_none() || !h.state().state.prefs.open);
    open(&mut h);
    assert_eq!(h.state().state.prefs.category, Category::View3d);
    let _ = h.get_by_label("回転の中心: 画面の中心");
    // 編集 → 設定… と Ctrl+, も、前に開いた区分で開く
    h.state_mut().state.apply(Action::Prefs(PrefsAction::Close));
    h.run();
    key(&h, egui::Key::Comma, egui::Modifiers::COMMAND);
    h.run();
    assert!(h.state().state.prefs.shows(Category::View3d));
}

/// 区分を移すと、右の欄はその区分の先頭から始まる（区分ごとにスクロールする）。
#[test]
fn headless_choosing_another_category_clears_the_search_and_starts_from_the_top() {
    let mut app = AppState::new(64, 64);
    app.prefs.search = "メモリ".into();
    app.apply(Action::Prefs(PrefsAction::Choose(Category::Files)));
    assert_eq!(app.prefs.category, Category::Files);
    assert!(app.prefs.search.is_empty(), "区分を選ぶと探す欄は空");
    app.apply(Action::Prefs(PrefsAction::OpenAt(Category::Pen)));
    assert!(app.prefs.shows(Category::Pen));
}

/// このビルドに無い区分（公開鍵の無いビルドの「更新」）を開くよう頼まれたら、「一般」を開く。
#[test]
fn headless_a_category_the_build_does_not_have_opens_general_instead() {
    let mut app = AppState::new(64, 64);
    assert!(!app.update.enabled());
    app.apply(Action::Prefs(PrefsAction::OpenAt(Category::Updates)));
    assert!(app.prefs.open);
    assert_eq!(app.prefs.category, Category::General);
    app.apply(Action::Prefs(PrefsAction::Choose(Category::Updates)));
    assert_eq!(app.prefs.category, Category::General);
}

// ───────── 探す欄 ─────────

/// どの項目も、1 つの区分に属し、名前を持つ。名前（日英）は区分の中で重ならず、探す欄の対象に全部入る。
#[test]
fn headless_every_item_belongs_to_one_category_with_a_name_in_both_languages() {
    let mut app = AppState::new(64, 64);
    // ペンの行は OS ごと。ここでは全部出す
    app.prefs.tablet_row = true;
    app.prefs.pen_input_row = true;
    for lang in Lang::ALL {
        app.lang = lang;
        let mut seen: Vec<Item> = Vec::new();
        for category in Category::ALL {
            for item in category.items(&app) {
                assert_eq!(item.category(), category, "{item:?}");
                assert!(!seen.contains(&item), "{item:?} が 2 つの区分に");
                seen.push(item);
                let names = item.names(lang);
                assert!(
                    !names.is_empty() && names.iter().all(|n| !n.is_empty()),
                    "{item:?}"
                );
                if lang == Lang::En {
                    assert!(
                        names.iter().all(|n| !has_japanese(n)),
                        "{item:?}: {names:?}"
                    );
                }
            }
            // ショートカットは表（項目は無い）。更新は公開鍵の無いビルドでは空
            if matches!(category, Category::Shortcuts | Category::Updates) {
                assert!(category.items(&app).is_empty());
            } else {
                assert!(!category.items(&app).is_empty(), "{category:?}");
            }
        }
    }
}

#[test]
fn headless_the_search_matches_item_names_and_category_names_in_the_current_language() {
    let mut app = AppState::new(64, 64);
    let groups = |app: &AppState, q: &str| -> Vec<(Category, Vec<Item>)> { prefs::search(app, q) };
    // 空・空白は何も当てない
    assert!(groups(&app, "").is_empty());
    assert!(groups(&app, "  ").is_empty());
    // 項目の名前: 日本語
    let found = groups(&app, "垂直");
    assert_eq!(found, vec![(Category::Display, vec![Item::Vsync])]);
    // 区分の名前に当たれば、その区分の項目は全部
    let found = groups(&app, "メモリ");
    let categories: Vec<Category> = found.iter().map(|(c, _)| *c).collect();
    assert_eq!(
        categories,
        vec![Category::Display, Category::Memory],
        "GPU のメモリ（表示）と、メモリの区分"
    );
    assert_eq!(found[0].1, vec![Item::GpuMemory]);
    assert_eq!(found[1].1, Category::Memory.items(&app));
    // ショートカットは区分の名前だけ（項目は無い）
    assert_eq!(
        groups(&app, "ショートカット"),
        vec![(Category::Shortcuts, Vec::new())]
    );
    // 英語に替えると、英語の名前で当たり、日本語では当たらない（大文字小文字は区別しない）
    app.lang = Lang::En;
    assert_eq!(
        groups(&app, "vsync"),
        vec![(Category::Display, vec![Item::Vsync])]
    );
    assert!(groups(&app, "垂直").is_empty());
    assert_eq!(
        groups(&app, "KEYBOARD"),
        vec![(Category::Shortcuts, Vec::new())]
    );
    // 筆圧の調整は、ペンの区分の名前に当たったときも、名前に当たったときも出る
    app.lang = Lang::Ja;
    let found = groups(&app, "筆圧");
    assert!(found
        .iter()
        .any(|(c, items)| *c == Category::Pen && items.contains(&Item::PressureAdjust)));
    let found = groups(&app, "ペン");
    assert!(found
        .iter()
        .any(|(c, items)| *c == Category::Pen && items.contains(&Item::PressureAdjust)));
    // 更新は、公開鍵の無いビルドでは当たらない
    assert!(groups(&app, "試験版").is_empty());
    assert!(groups(&app, "更新").is_empty());
    // 行の下の名前（ポート番号・キャッシュの上限と場所・合計・下限と上限）にも当たる
    for (query, want) in [
        (
            "ポート番号",
            vec![(Category::LiveLink, vec![Item::ExternalOps])],
        ),
        (
            "キャッシュの上限",
            vec![(Category::Memory, vec![Item::DiskCache])],
        ),
        (
            "キャッシュの場所",
            vec![(Category::Memory, vec![Item::DiskCache])],
        ),
        ("合計", vec![(Category::Display, vec![Item::GpuMemory])]),
        ("下限", vec![(Category::Pen, vec![Item::PressureAdjust])]),
        (
            "上限",
            vec![
                (Category::Pen, vec![Item::PressureAdjust]),
                (Category::Memory, vec![Item::DiskCache]),
            ],
        ),
    ] {
        assert_eq!(groups(&app, query), want, "{query}");
    }
    app.lang = Lang::En;
    for (query, want) in [
        // "Export padding" にも "port" がある
        (
            "port",
            vec![
                (Category::Files, vec![Item::ExportPadding]),
                (Category::LiveLink, vec![Item::ExternalOps]),
            ],
        ),
        (
            "cache limit",
            vec![(Category::Memory, vec![Item::DiskCache])],
        ),
        (
            "cache folder",
            vec![(Category::Memory, vec![Item::DiskCache])],
        ),
        ("total", vec![(Category::Display, vec![Item::GpuMemory])]),
        ("high", vec![(Category::Pen, vec![Item::PressureAdjust])]),
    ] {
        assert_eq!(groups(&app, query), want, "{query}");
    }
    // ディスクキャッシュは「メモリ」の区分にある（ファイルの区分には無い）
    assert!(Category::Memory.items(&app).contains(&Item::DiskCache));
    assert!(!Category::Files.items(&app).contains(&Item::DiskCache));
}

/// どの `Item` も、どこかの区分の `items()` に入っている（`Item::ALL` に全部があり、番号が合う）。更新の区分は、公開鍵のあるビルドだけに出る。
#[test]
fn headless_every_item_is_listed_in_some_category() {
    let mut app = AppState::new(64, 64);
    // ペンの行は OS ごと。ここでは全部出す。更新の区分は、公開鍵のあるビルドの形にする
    app.prefs.tablet_row = true;
    app.prefs.pen_input_row = true;
    let _rig = crate::update::rig(&mut app, "0.1.0", yolu_app::update::Mode::Installer);
    let listed: Vec<Item> = Category::ALL
        .into_iter()
        .flat_map(|c| c.items(&app))
        .collect();
    for (i, item) in Item::ALL.into_iter().enumerate() {
        assert_eq!(
            item.rank(),
            i,
            "{item:?} の番号が `Item::ALL` の中の位置と合わない"
        );
        assert!(
            listed.contains(&item),
            "{item:?} が、どの区分の items() にも無い"
        );
    }
    assert_eq!(
        listed.len(),
        Item::ALL.len(),
        "区分の項目と `Item::ALL` の数が合わない"
    );
}

/// 探す欄に打つと、全部の区分から名前の合う項目だけが区分ごとに右へ並び、左では合う区分が光る（合わない区分は薄くなる）。区分を押すと探す欄は
/// 空になってその区分が開く。Esc は、まず探す欄の文字を消し、次にウィンドウを閉じる。
#[test]
fn typing_in_the_search_field_lists_the_matching_items_and_lights_the_categories() {
    let dir = settings_dir("search");
    let mut h = app_with_settings(&dir, vec2(1280.0, 800.0));
    open(&mut h);
    type_in_search(&mut h, "メモリ");
    assert_eq!(h.state().state.prefs.search, "メモリ");
    // 右に並ぶのは、GPU のメモリ（表示）と、メモリの区分の項目。ほかの区分の行は出ない
    let _ = h.get_by_label("GPU のメモリ: 自動");
    let _ = h.get_by_label("取り消し履歴: 自動（2048 MiB）");
    let _ = h.get_by_label("最小の取り消し段数");
    assert!(h.query_by_label("言語: 日本語").is_none());
    assert!(h.query_by_label("CPU のスレッド: 自動（8）").is_none());
    // 左: 表示とメモリが光り、ほかは光らない（今の区分の「一般」も、探している間は選ばれた印にしない）
    assert!(lit(&h, "表示") && lit(&h, "メモリ"));
    for dim in ["一般", "ペン", "処理", "3D ビュー", "ファイル"] {
        assert!(!lit(&h, dim), "{dim}");
    }
    // 結果の見出し（区分の名前）が右に出る
    let w = window(&h);
    let headings: Vec<String> = h
        .query_all_by_label("メモリ")
        .filter(|n| n.rect().left() > w.left() + 220.0)
        .map(|n| n.rect().top().to_string())
        .collect();
    assert!(headings.is_empty() || headings.len() == 1);
    // 当たらない文字は、何も並べない（「なし」の文字も出さない）
    type_in_search(&mut h, "zzzz");
    assert!(h.query_by_label("GPU のメモリ: 自動").is_none());
    assert!(!h
        .query_all_by_label("なし")
        .any(|n| w.contains_rect(n.rect())));
    for category in Category::ALL
        .into_iter()
        .filter(|c| *c != Category::Updates)
    {
        assert!(!lit(&h, category.label(Lang::Ja)), "{category:?}");
    }
    // 英語の名前で探す（言語を替えると、今の言語の文字で当たる）
    h.state_mut().state.lang = Lang::En;
    h.run();
    type_in_search(&mut h, "vsync");
    let _ = h.get_by_label("VSync");
    assert!(lit(&h, "Display") && !lit(&h, "Memory"));
    // 項目は、探した結果の中でも使える（値を替えると設定に入る）
    h.get_by_label("VSync").click();
    h.run();
    assert!(h.state().state.settings().vsync);
    // 区分を押す: 探す欄は空になり、その区分が開く
    click_side(&mut h, Category::Files);
    assert!(h.state().state.prefs.search.is_empty());
    assert_eq!(h.state().state.prefs.category, Category::Files);
    let _ = h.get_by_label("Export padding: Dilation infinite");
    // Esc: 探す欄の文字を消す → もう一度で閉じる
    type_in_search(&mut h, "pen");
    assert!(!h.state().state.prefs.search.is_empty());
    let inside = window(&h).center();
    move_to(&h, inside);
    h.run();
    h.ctx
        .memory_mut(|m| m.surrender_focus(egui::Id::new("yolu.prefs.search")));
    key(&h, egui::Key::Escape, egui::Modifiers::NONE);
    h.run();
    assert!(
        h.state().state.prefs.search.is_empty(),
        "最初の Esc は探す欄を空にする"
    );
    assert!(h.state().state.prefs.open);
    key(&h, egui::Key::Escape, egui::Modifiers::NONE);
    h.run();
    assert!(!h.state().state.prefs.open, "次の Esc で閉じる");
}

/// 探した結果にショートカットの区分の名前があるとき、その行を押すとショートカットの区分が開く。
#[test]
fn a_search_hit_on_the_shortcut_category_opens_it_from_the_row() {
    let dir = settings_dir("search-shortcuts");
    let mut h = app_with_settings(&dir, vec2(1280.0, 800.0));
    open(&mut h);
    type_in_search(&mut h, "ショートカット");
    assert!(lit(&h, "ショートカット"));
    let w = window(&h);
    let row = rect_of(&h, "ショートカット", |r| {
        w.contains_rect(r) && r.left() > w.left() + 220.0 && r.top() > w.top() + 60.0
    });
    click(&mut h, row.center());
    assert!(h.state().state.prefs.shows(Category::Shortcuts));
    assert!(h.state().state.prefs.search.is_empty());
}

// ───────── メニューから外した物が設定のウィンドウにある ─────────

/// 言語（一般）・ショートカット（ショートカット）・筆圧の調整（ペン）・更新（公開鍵のあるビルド）は、設定のウィンドウの区分から行ける。
/// 言語は選ぶと画面が替わり、ショートカットの区分には今までの表と下の帯、ペンの区分には筆圧の調整（曲線・下限・上限・描く枠・ボタン）がある。
#[test]
fn everything_taken_out_of_the_menus_is_in_the_settings_window() {
    let dir = settings_dir("reach");
    let mut h = app_with_settings(&dir, vec2(1280.0, 800.0));
    open(&mut h);
    // 言語
    let at = h.get_by_label("言語: 日本語").rect().center();
    click(&mut h, at);
    let at = popup_item(&h, "English").center();
    click(&mut h, at);
    assert_eq!(h.state().state.lang, Lang::En);
    let _ = h.get_by_label("Language: English");
    // ショートカット: 左の区分を押すと、下にショートカットの区分が出て、右に表と下の帯
    click_side(&mut h, Category::Shortcuts);
    assert!(h.state().state.prefs.shows(Category::Shortcuts));
    for label in [
        "Everywhere & View",
        "Paint",
        "Edit",
        "Pose",
        "During an Operation",
        "Pie Menus",
    ] {
        let _ = side(&h, label);
    }
    for label in ["Reset to Default", "Export…", "Import…", "Find by Key"] {
        let _ = h.get_by_label(label);
    }
    assert!(
        h.state().state.shortcuts.search_rect.is_some(),
        "表の探す欄"
    );
    // ペン: 筆圧の調整
    click_side(&mut h, Category::Pen);
    assert!(h.state().state.pressure.open());
    for label in ["Auto", "Clear", "Revert", "Default", "Low", "High"] {
        let _ = h.get_by_label(label);
    }
    assert!(!h.state().state.shortcuts.capturing());
    // ショートカットの区分に戻ると、表はまた使える
    click_side(&mut h, Category::Shortcuts);
    assert!(
        !h.state().state.pressure.open(),
        "ペンを離れると描く枠は無い"
    );
}

// ───────── Esc ─────────

fn esc(h: &mut H) {
    key(h, egui::Key::Escape, egui::Modifiers::NONE);
    h.run();
}

/// ポップアップ（言語など）を開いて Esc: ポップアップだけが閉じて、設定のウィンドウは閉じない（次の Esc で閉じる）。
/// `YoluApp::popups` がこの描き方より先に Esc でポップアップを閉じるので、「このフレームの初めに開いていたか」（`popup_was_open`）で見る。
#[test]
fn escape_closes_only_the_popup_when_one_of_the_window_is_open() {
    let dir = settings_dir("esc-popup");
    let mut h = app_with_settings(&dir, vec2(1280.0, 800.0));
    open(&mut h);
    let at = h.get_by_label("言語: 日本語").rect().center();
    click(&mut h, at);
    assert!(h.state().state.popup.is_some(), "言語の選びが開く");
    esc(&mut h);
    assert!(
        h.state().state.popup.is_none(),
        "Esc でポップアップが閉じる"
    );
    assert!(
        h.state().state.prefs.open,
        "ポップアップを閉じた Esc で、設定のウィンドウは閉じない"
    );
    esc(&mut h);
    assert!(!h.state().state.prefs.open, "次の Esc で閉じる");
}

/// ポート番号の欄（探す欄以外の入力欄）に打っている途中の Esc: 欄が入力をやめるだけで、ウィンドウは閉じない。値は変わらない。
/// egui は Esc を受けたフレームの初めに欄のフォーカスを外すので、前のフレームの終わりのフォーカスで見る。
#[test]
fn escape_while_typing_in_the_port_field_only_leaves_the_field() {
    let dir = settings_dir("esc-port");
    let mut h = app_with_settings(&dir, vec2(1280.0, 800.0));
    // OS が空いた番号を選ぶ（ほかの試験・開いているアプリと番号が重ならない）
    h.state_mut().state.prefs.settings.external_ops_port = 0;
    h.state_mut().state.prefs.settings.external_ops = true;
    h.state_mut()
        .state
        .apply(Action::Prefs(PrefsAction::OpenAt(Category::LiveLink)));
    h.run();
    let toggle = h.get_by_label("外からの操作を受ける").rect();
    let field = h
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .map(|n| n.rect())
        .find(|r| r.top() > toggle.bottom() && r.top() < toggle.bottom() + 40.0)
        .expect("ポート番号の欄");
    click(&mut h, field.center());
    h.event(egui::Event::Text("2".into()));
    h.run();
    assert!(h.ctx.memory(|m| m.focused()).is_some(), "欄に打っている");
    esc(&mut h);
    assert!(
        h.state().state.prefs.open,
        "打っている途中の Esc では閉じない"
    );
    assert!(
        h.ctx.memory(|m| m.focused()).is_none(),
        "欄はフォーカスを手放した"
    );
    assert_eq!(h.state().state.prefs.settings.external_ops_port, 0);
    esc(&mut h);
    assert!(!h.state().state.prefs.open, "次の Esc で閉じる");
}

/// ショートカットの表の探す欄に打っている途中の Esc: 欄が入力をやめるだけで、ウィンドウは閉じない。
#[test]
fn escape_while_typing_in_the_shortcut_table_search_only_leaves_the_field() {
    let dir = settings_dir("esc-table");
    let mut h = app_with_settings(&dir, vec2(1280.0, 800.0));
    h.state_mut()
        .state
        .apply(Action::Prefs(PrefsAction::OpenAt(Category::Shortcuts)));
    h.run();
    let field = h.state().state.shortcuts.search_rect.expect("表の探す欄");
    click(&mut h, field.center());
    h.event(egui::Event::Text("abc".into()));
    h.run();
    assert_eq!(h.state().state.shortcuts.search, "abc");
    esc(&mut h);
    assert!(h.state().state.prefs.open, "表の探す欄の Esc では閉じない");
    esc(&mut h);
    assert!(!h.state().state.prefs.open, "次の Esc で閉じる");
}

/// 入力欄でない部品（スイッチ）を押したあとの Esc は、すぐ閉じる（押した部品のフォーカスを、打っている印と間違えない）。
#[test]
fn escape_right_after_pressing_a_toggle_closes_the_window() {
    let dir = settings_dir("esc-toggle");
    let mut h = app_with_settings(&dir, vec2(1280.0, 800.0));
    h.state_mut()
        .state
        .apply(Action::Prefs(PrefsAction::OpenAt(Category::LiveLink)));
    h.run();
    h.get_by_label("Unity の Live Link を受け付ける").click();
    h.run();
    let inside = window(&h).center();
    move_to(&h, inside);
    h.run();
    esc(&mut h);
    assert!(!h.state().state.prefs.open);
}

/// 色のウィンドウ（UV ワイヤーフレームの色）を開いている間の Esc は、色のウィンドウだけを閉じる。設定のウィンドウは閉じない。
#[test]
fn escape_closes_the_color_window_of_the_settings_and_not_the_settings() {
    let dir = settings_dir("esc-color");
    let mut h = app_with_settings(&dir, vec2(1280.0, 800.0));
    h.state_mut()
        .state
        .apply(Action::Prefs(PrefsAction::OpenAt(Category::View3d)));
    h.run();
    h.get_by_role_and_label(
        egui::accesskit::Role::ColorWell,
        "UV ワイヤーフレームの色と不透明度",
    )
    .click();
    h.run();
    assert!(yolu_app::panels::color_window::is_open(&h.ctx));
    let inside = window(&h).center();
    move_to(&h, inside);
    h.run();
    esc(&mut h);
    assert!(
        !yolu_app::panels::color_window::is_open(&h.ctx),
        "色のウィンドウが閉じる"
    );
    assert!(h.state().state.prefs.open, "設定のウィンドウは閉じない");
    esc(&mut h);
    assert!(!h.state().state.prefs.open, "次の Esc で閉じる");
}

// ───────── 閉じる・開く ─────────

/// 閉じると探す欄は空になり、次に開くと前に開いた区分から始まる（探していた間も、区分そのものは変わらない）。
#[test]
fn headless_closing_empties_the_search_and_reopening_starts_from_the_last_category() {
    let mut app = AppState::new(64, 64);
    app.apply(Action::Prefs(PrefsAction::OpenAt(Category::Memory)));
    app.prefs.search = "垂直".into();
    assert!(
        !app.prefs.shows(Category::Memory),
        "探している間は、区分を見ていない"
    );
    app.apply(Action::Prefs(PrefsAction::Close));
    assert!(app.prefs.search.is_empty());
    app.apply(Action::Prefs(PrefsAction::Open));
    assert!(app.prefs.shows(Category::Memory), "前に開いた区分から");
}

/// ぶつかるキーが残ったままショートカットの区分を離れて、別の区分から閉じても、保存していないことを知らせる（区分を離れただけでは知らせない）。
#[test]
fn closing_from_another_category_still_tells_that_the_conflicting_keys_are_not_saved() {
    use yolu_app::keymap::{Scope, Trigger};
    let dir = settings_dir("unsaved-keys");
    let mut h = app_with_settings(&dir, vec2(1280.0, 800.0));
    h.state_mut()
        .state
        .apply(Action::Prefs(PrefsAction::OpenAt(Category::Shortcuts)));
    h.run();
    // 消しゴムのキーをブラシの B にする（ぶつかる）
    h.state_mut().state.keys.set(
        ("tool.eraser", Scope::Paint),
        vec![Trigger::Key {
            modifiers: egui::Modifiers::NONE,
            key: egui::Key::B,
        }],
    );
    h.state_mut().state.keys_changed();
    h.run();
    assert!(h.state().state.keys.has_conflicts());
    h.state_mut().state.clear_message();
    click_side(&mut h, Category::General);
    assert!(
        h.state().state.message.is_empty(),
        "区分を離れただけでは知らせない: {}",
        h.state().state.message
    );
    h.state_mut().state.apply(Action::Prefs(PrefsAction::Close));
    h.run();
    assert_eq!(
        h.state().state.message,
        "ぶつかるキーがあるので、キーの設定を保存していません"
    );
}

// ───────── 左の区分の列のスクロール ─────────

/// 低い画面で左の区分（ショートカットを開くと 16 行）が列に収まらないとき、共通のスクロールで送って、下の区分も選べる。
#[test]
fn the_sidebar_scrolls_when_the_categories_do_not_fit_and_the_lower_ones_can_be_chosen() {
    let dir = settings_dir("side-scroll");
    let mut h = app_with_settings(&dir, vec2(960.0, 400.0));
    h.state_mut()
        .state
        .apply(Action::Prefs(PrefsAction::OpenAt(Category::Shortcuts)));
    h.run();
    h.run();
    let w = window(&h);
    // ウィンドウの右の縁のつまみとは別の、左の列のつまみがある
    let bar = h
        .query_all_by_role(egui::accesskit::Role::ScrollBar)
        .map(|n| n.rect())
        .find(|r| {
            w.contains_rect(*r) && r.right() < w.left() + 215.0 && r.left() > w.left() + 190.0
        })
        .expect("左の区分の列のつまみ");
    assert!(bar.height() > 60.0, "{bar:?}");
    // 下の区分は、送る前はウィンドウの外（見えない）で、ホイールで送ると見えて押せる
    let target = Category::LiveLink;
    let before = h
        .query_all_by_label(target.label(Lang::Ja))
        .map(|n| n.rect())
        .find(|r| r.left() < w.left() + 220.0);
    assert!(
        before.is_none_or(|r| !w.contains_rect(r)),
        "送る前は下にはみ出す: {before:?} {w:?}"
    );
    move_to(&h, egui::pos2(w.left() + 60.0, w.top() + 200.0));
    h.step();
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -5000.0),
        modifiers: egui::Modifiers::NONE,
        phase: egui::TouchPhase::Move,
    });
    h.run();
    h.run();
    let r = side(&h, target.label(Lang::Ja));
    assert!(w.contains_rect(r), "送ると見える: {r:?} {w:?}");
    click(&mut h, r.center());
    assert!(h.state().state.prefs.shows(target), "押すと開く");
    // 開いた区分の見出しの名前が、区分の列のはみ出しを描かない（ウィンドウの外へは描かない）
    for (text, r) in window_texts(&h)
        .into_iter()
        .filter(|(_, r)| r.height() > 0.0)
    {
        assert!(
            w.expand(1.0).contains_rect(r),
            "ウィンドウの外に描く「{text}」{r:?} {w:?}"
        );
    }
}

// ───────── 絵（日英） ─────────

/// ウィンドウの中だけを撮って、正解の絵と比べる（ほかのパネルの変更で壊れない）。
fn shot(h: &mut H, name: &str) {
    let rect = window(h);
    // 入力欄の点滅する印が絵に残らないように
    if let Some(focused) = h.ctx.memory(|m| m.focused()) {
        h.ctx.memory_mut(|m| m.surrender_focus(focused));
    }
    h.event(egui::Event::PointerGone);
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

/// 開いているメニューの中だけ（四隅と影を除く内側）を撮る。
fn shot_menu(h: &mut H, name: &str) {
    h.event(egui::Event::PointerGone);
    h.step();
    let popup = h
        .state()
        .state
        .popup
        .as_ref()
        .expect("popup open")
        .state
        .rect
        .shrink(6.0);
    let image = h.render().expect("描画");
    let cropped = image::imageops::crop_imm(
        &image,
        popup.left().floor() as u32,
        popup.top().floor() as u32,
        popup.width().ceil() as u32,
        popup.height().ceil() as u32,
    )
    .to_image();
    egui_kittest::image_snapshot(&cropped, name);
}

#[test]
fn snapshot_the_categories_of_the_settings_window_in_both_languages() {
    let dir = settings_dir("shots");
    let mut h = app_with_settings(&dir, vec2(1280.0, 800.0));
    // Windows の行と更新を出した形で撮る（ほかの OS・公開鍵の無いビルドでも同じ絵）
    h.state_mut().state.prefs.pen_input_row = true;
    h.state_mut().state.prefs.tablet_row = false;
    let _rig = crate::update::rig(
        &mut h.state_mut().state,
        "0.1.0",
        yolu_app::update::Mode::Installer,
    );
    for (lang, suffix) in [(Lang::Ja, "ja"), (Lang::En, "en")] {
        h.state_mut().state.lang = lang;
        for (category, name) in [
            (Category::General, "general"),
            (Category::Pen, "pen"),
            (Category::Shortcuts, "shortcuts"),
            (Category::Display, "display"),
            (Category::Files, "files"),
            (Category::Updates, "updates"),
        ] {
            h.state_mut()
                .state
                .apply(Action::Prefs(PrefsAction::OpenAt(category)));
            h.state_mut().state.clear_message();
            h.run();
            shot(&mut h, &format!("settings_{name}_{suffix}"));
        }
        // 探した結果（メモリ: 表示の GPU のメモリと、メモリの区分）
        h.state_mut()
            .state
            .apply(Action::Prefs(PrefsAction::OpenAt(Category::General)));
        h.run();
        type_in_search(&mut h, lang.pick("メモリ", "memory"));
        h.state_mut().state.clear_message();
        h.run();
        shot(&mut h, &format!("settings_search_{suffix}"));
        h.state_mut().state.prefs.search.clear();
        h.run();
    }
}

#[test]
fn snapshot_the_file_view_and_help_menus_in_both_languages() {
    let dir = settings_dir("menus");
    let mut h = app_with_settings(&dir, vec2(1280.0, 800.0));
    let _rig = crate::update::rig(
        &mut h.state_mut().state,
        "0.1.0",
        yolu_app::update::Mode::Installer,
    );
    for (lang, suffix) in [(Lang::Ja, "ja"), (Lang::En, "en")] {
        h.state_mut().state.lang = lang;
        h.run();
        for (title, name) in [
            (lang.pick("ファイル", "File"), "file"),
            (lang.pick("表示", "View"), "view"),
            (lang.pick("ヘルプ", "Help"), "help"),
        ] {
            let at = menu_title(&h, title).center();
            click(&mut h, at);
            shot_menu(&mut h, &format!("menu_{name}_{suffix}"));
            key(&h, egui::Key::Escape, egui::Modifiers::NONE);
            h.run();
        }
    }
}

/// どの区分も、行（名前の列と値の列）が、上の「設定を探す」と同じ右端まで伸びる（値の列が伸びる。区分ごとの固定の幅は無い）。日英。
#[test]
fn every_category_runs_to_the_right_edge_of_the_search_field_in_both_languages() {
    let dir = settings_dir("right-edge");
    let mut h = app_with_settings(&dir, vec2(1280.0, 800.0));
    let _rig = crate::update::rig(
        &mut h.state_mut().state,
        "0.1.0",
        yolu_app::update::Mode::Installer,
    );
    open(&mut h);
    for lang in Lang::ALL {
        h.state_mut().state.lang = lang;
        h.run();
        let available: Vec<Category> = Category::ALL
            .into_iter()
            // ショートカットは表（下の試験）。更新は入り切りのチェックだけで、右端まで伸びる箱が無い
            .filter(|c| !matches!(c, Category::Shortcuts | Category::Updates))
            .filter(|c| c.available(&h.state().state))
            .collect();
        for category in available {
            click_side(&mut h, category);
            // 外からの操作を入れると、ポート番号の入力欄の行が出る（Live Link と外からの操作の区分は、これが右端まで伸びる唯一の箱）
            h.state_mut().state.prefs.settings.external_ops = category == Category::LiveLink;
            h.state_mut().state.prefs.settings.external_ops_port = 0;
            h.run();
            h.run();
            let w = window(&h);
            // 上の探す欄の右端（ウィンドウの右の端から 16）
            let edge = w.right() - 16.0;
            // 描いた箱（値の箱・ボタン・スライダーなど。幅 60 以上）の右端。後ろのドックの部品は読み上げの木には残るので、描いた絵から見る
            let rights = painted_box_rights(&h, w);
            let widest = rights.iter().copied().fold(f32::MIN, f32::max);
            assert!(
                (widest - edge).abs() < 1.5,
                "{lang:?} {category:?}: 右端 {widest} は探す欄の右端 {edge} と合わない"
            );
        }
    }
}

/// 設定のウィンドウが描いた角丸の箱（幅 60 以上。ウィンドウの地・左の区分・上の探す欄を除く）の右端。
fn painted_box_rights(h: &H, window: Rect) -> Vec<f32> {
    use egui::epaint::Shape;
    let shapes = &h.output().shapes;
    let start = shapes
        .iter()
        .rposition(|s| matches!(&s.shape, Shape::Rect(r) if r.rect == window))
        .expect("ウィンドウの地を描いた");
    let mut out = Vec::new();
    fn walk(shape: &Shape, window: Rect, out: &mut Vec<f32>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, window, out)),
            Shape::Rect(r)
                if r.rect.width() >= 60.0
                    && r.rect.left() > window.left() + 230.0
                    && r.rect.top() > window.top() + 60.0
                    && window.contains_rect(r.rect) =>
            {
                out.push(r.rect.right())
            }
            _ => {}
        }
    }
    for s in &shapes[start..] {
        walk(&s.shape, window, &mut out);
    }
    out
}
