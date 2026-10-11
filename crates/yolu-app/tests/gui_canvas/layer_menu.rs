//! レイヤーと効果を足すメニューの組み直しと、メインとサブの色の 2 枚（ツールの帯の一番下）の試験。
//! - 「フィルター」のメニューは、フィルターを平らに並べ、区切りのあと「ジェネレーター ▸」。見出し（「…の画素」・「Generator」）は置かず、
//!   押せない理由はラベルに続けずツールチップへ。
//! - 「レイヤー」のメニューは、メニューバーと右クリックと一覧の空白で同じ関数。新規塗りつぶし ▸・新規調整 ▸ の入れ子、効果、グループ、属性。
//! - 画像・デカールの塗りつぶしは、棚の画像（またはファイル）を選んで 1 回の Undo で作る。選ばずに閉じたら何も作らない。
//! - 2 枚の色は、ツールの帯の下の端に付き、最小のウィンドウでもアイコンと重ならない。
use crate::common;

use std::path::PathBuf;

use common::{app, assert_plain, click, menu_title, popup_item};
use egui::{pos2, vec2, Event, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::engine::{Channel, LayerKind, Rgba8};
use yolu_app::fillfx::inputs;
use yolu_app::lang::Lang;
use yolu_app::layermenu::Op;
use yolu_app::m2::{AdjustmentKind, Edit};
use yolu_app::state::{Action, AppState, DialogRequest, PopupKind, Tool};
use yolu_app::ui::menu::{leaves, Entry};
use yolu_app::{shell, YoluApp};
use yolu_core::fill_image::{Placement, ProjectionMode, Wrap};
use yolu_core::generator::Shape;
use yolu_core::{FilterTarget, ImageId};

const RED: Rgba8 = Rgba8::new(255, 0, 0, 255);
const GREEN: Rgba8 = Rgba8::new(0, 255, 0, 255);
const BLUE: Rgba8 = Rgba8::new(0, 0, 255, 255);
const WHITE: Rgba8 = Rgba8::new(255, 255, 255, 255);

fn quad_image() -> Vec<u8> {
    [RED, GREEN, BLUE, WHITE]
        .iter()
        .flat_map(|c| [c.r, c.g, c.b, c.a])
        .collect()
}

/// 画像を棚へ入れる（出どころなし）。棚の ID と、文書の画像の ID。
fn shelf_image(s: &mut AppState, name: &str) -> (String, ImageId) {
    let rid = s
        .shelf
        .add_image(Lang::Ja, name, &quad_image(), 2, 2)
        .expect("棚へ入る");
    let id = inputs::image_id(&rid).expect("GUID");
    (rid, id)
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yolu-layer-menu-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 並びの形（項目の名前・押せるか・キー・入れ子の中身）を文字にする。2 つのメニューが同じ並びかを比べるのに使う。
fn shape(entries: &[Entry<Action>]) -> Vec<String> {
    entries
        .iter()
        .map(|e| match e {
            Entry::Item {
                label,
                enabled,
                shortcut,
                ..
            } => format!(
                "item:{label}:{enabled}:{}",
                shortcut.clone().unwrap_or_default()
            ),
            Entry::Submenu {
                label,
                entries,
                enabled,
                ..
            } => format!("sub:{label}:{enabled}[{}]", shape(entries).join("|")),
            Entry::Separator => "---".to_owned(),
            Entry::Heading(label) => format!("head:{label}"),
        })
        .collect()
}

/// 項目の名前（区切りは `None`）。
fn names(entries: &[Entry<Action>]) -> Vec<Option<String>> {
    entries
        .iter()
        .map(|e| e.label().map(str::to_owned))
        .collect()
}

fn submenu<'a>(entries: &'a [Entry<Action>], label: &str) -> &'a [Entry<Action>] {
    entries
        .iter()
        .find_map(|e| match e {
            Entry::Submenu {
                label: l, entries, ..
            } if l == label => Some(entries.as_slice()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("入れ子 {label} が無い: {:?}", names(entries)))
}

fn has_japanese(text: &str) -> bool {
    text.chars().any(|c| matches!(c, '\u{3040}'..='\u{9fff}'))
}

// ───────── 効果のメニュー ─────────

#[test]
fn the_filter_menu_has_only_filters_and_anchors_without_a_heading() {
    let filter_count = yolu_app::fx::FilterKind::ALL.len();
    for lang in Lang::ALL {
        for edit_mask in [false, true] {
            let mut s = AppState::new(64, 64);
            s.lang = lang;
            let layer = s.selected_layer.unwrap();
            if edit_mask {
                s.apply(Action::M2(Edit::AddMask(layer)));
                assert!(s.m2.edit_mask, "マスクを足すと描く先がマスクになる");
            }
            let entries = shell::menu_entries(&s, 4);
            // 見出し（足す先の名前・Generator）は置かない
            assert!(
                !entries.iter().any(|e| matches!(e, Entry::Heading(_))),
                "{lang:?} {edit_mask}: {:?}",
                names(&entries)
            );
            for text in names(&entries).into_iter().flatten() {
                assert!(
                    !text.contains("の画素") && !text.contains("pixels") && text != "Generator",
                    "{lang:?}: {text}"
                );
            }
            // 並び: フィルター（平ら）→ 区切り → アンカーの項目。ジェネレーターは入れない（入り口を分ける）
            let labels = names(&entries);
            assert_eq!(labels[filter_count], None, "{lang:?}");
            assert!(
                labels[filter_count + 1]
                    .as_deref()
                    .is_some_and(|l| l.contains(lang.pick("アンカー", "Anchor"))),
                "{lang:?}: {labels:?}"
            );
            // 断られる項目は押せない項目（理由はツールチップ）で残る
            assert!(entries[..filter_count].iter().all(|e| matches!(
                e,
                Entry::Item {
                    action: Action::Fx(yolu_app::fx::FxOp::AddFilter { .. }),
                    ..
                } | Entry::Item { enabled: false, .. }
            )));
            assert!(
                !leaves(&entries).iter().any(|e| matches!(
                    e,
                    Entry::Item {
                        action: Action::Fx(yolu_app::fx::FxOp::AddGenerator { .. }),
                        ..
                    }
                )),
                "{lang:?}: フィルターのメニューにジェネレーターを置かない"
            );
            assert!(!entries.iter().any(|e| matches!(e, Entry::Submenu { .. })));
            // 足す先は、今の編集の状態のまま（マスクを描いていればマスク、そうでなければレイヤーの画素）
            let target = if edit_mask {
                FilterTarget::Mask
            } else {
                FilterTarget::Content
            };
            assert_eq!(yolu_app::fx::menu::target(&s), target);
            assert!(leaves(&entries).iter().any(|e| matches!(
                e,
                Entry::Item { action: Action::Fx(yolu_app::fx::FxOp::AddFilter { target: t, .. }), .. } if *t == target
            )));
            if lang == Lang::En {
                for text in labels.into_iter().flatten() {
                    assert!(!has_japanese(&text), "{text}");
                }
            }
        }
    }
}

#[test]
fn an_anchor_that_cannot_be_read_says_why_in_a_tooltip_and_not_after_the_name() {
    for lang in Lang::ALL {
        let mut s = AppState::new(64, 64);
        s.lang = lang;
        let generators = yolu_app::fx::menu::add_generator_entries(&s, FilterTarget::Content);
        let anchor = generators
            .iter()
            .find_map(|e| match e {
                Entry::Item {
                    label,
                    enabled,
                    tooltip,
                    ..
                } if label.contains(lang.pick("アンカー", "Anchor")) => {
                    Some((label.clone(), *enabled, tooltip.clone()))
                }
                _ => None,
            })
            .expect("アンカーの項目");
        assert!(!anchor.1, "アンカーが無いので押せない");
        assert!(
            !anchor.0.contains('—'),
            "ラベルに理由を続けない: {}",
            anchor.0
        );
        assert_eq!(anchor.0, lang.pick("アンカー", "Anchor"));
        let why = anchor.2.expect("理由はツールチップ");
        assert_eq!(
            why,
            lang.pick(
                "このレイヤーより下にアンカーが無い",
                "no anchor below this layer"
            )
        );
    }
}

#[test]
fn the_filter_and_generator_buttons_open_separate_flat_lists() {
    for lang in Lang::ALL {
        let mut s = AppState::new(64, 64);
        s.lang = lang;
        let filters = yolu_app::m2_menu::entries(
            &s,
            yolu_app::m2_menu::Popup::AddFilter(FilterTarget::Content),
        );
        let generators = yolu_app::m2_menu::entries(
            &s,
            yolu_app::m2_menu::Popup::AddGenerator(FilterTarget::Content),
        );
        let bar = shell::menu_entries(&s, 4);
        // メニューバーの「フィルター」は、フィルターのボタンのポップアップに、区切りとアンカーの項目が続くだけ
        assert_eq!(shape(&filters), shape(&bar[..filters.len()]));
        for popup in [&filters, &generators] {
            assert!(!popup.iter().any(|e| matches!(
                e,
                Entry::Heading(_) | Entry::Separator | Entry::Submenu { .. }
            )));
        }
        assert_eq!(filters.len(), yolu_app::fx::FilterKind::ALL.len());
        assert_eq!(generators.len(), yolu_app::fx::names::GENERATOR_KINDS.len());
    }
}

#[test]
fn the_mask_menu_adds_to_the_mask_and_has_the_mask_switches() {
    for lang in Lang::ALL {
        let mut s = AppState::new(64, 64);
        s.lang = lang;
        let layer = s.selected_layer.unwrap();
        assert!(
            shell::popup_entries(
                &s,
                PopupKind::M2(yolu_app::m2_menu::Popup::MaskContext(layer))
            )
            .is_empty(),
            "マスクが無ければ空"
        );
        s.apply(Action::M2(Edit::AddMask(layer)));
        // レイヤーの画素を対象にしていても、マスクのメニューの項目はマスクへ足す
        s.apply(Action::M2Ui(yolu_app::m2::UiOp::EditMask(false)));
        let v = shell::popup_entries(
            &s,
            PopupKind::M2(yolu_app::m2_menu::Popup::MaskContext(layer)),
        );
        let ja_en = |ja: &str, en: &str| Some(lang.pick(ja, en).to_owned());
        assert_eq!(
            names(&v),
            [
                ja_en("フィルターを追加", "Add Filter"),
                ja_en("ジェネレーターを追加", "Add Generator"),
                None,
                ja_en("反転", "Invert"),
                ja_en("有効", "Enabled"),
                None,
                ja_en("マスクを削除", "Delete Mask"),
            ]
        );
        let filters = submenu(&v, lang.pick("フィルターを追加", "Add Filter"));
        assert!(filters.iter().all(|e| matches!(
            e,
            Entry::Item {
                action: Action::Fx(yolu_app::fx::FxOp::AddFilter {
                    target: FilterTarget::Mask,
                    ..
                }),
                ..
            } | Entry::Item { enabled: false, .. }
        )));
        let generators = submenu(&v, lang.pick("ジェネレーターを追加", "Add Generator"));
        assert!(generators.iter().any(|e| matches!(
            e,
            Entry::Item {
                action: Action::Fx(yolu_app::fx::FxOp::AddGenerator {
                    target: FilterTarget::Mask,
                    ..
                }),
                ..
            }
        )));
        // 反転・有効・削除は、プロパティのマスクの欄と同じ操作
        // 入れ子の中（フィルターの「階調の反転」= Invert など）ではなく、マスクのメニューの直下の項目
        let action = |label: &str| {
            v.iter()
                .find_map(|e| match e {
                    Entry::Item {
                        label: l, action, ..
                    } if l == label => Some(action.clone()),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(
            action(lang.pick("反転", "Invert")),
            Action::M2(Edit::MaskInverted(layer, true))
        );
        assert_eq!(
            action(lang.pick("有効", "Enabled")),
            Action::M2(Edit::MaskEnabled(layer, false))
        );
        assert_eq!(
            action(lang.pick("マスクを削除", "Delete Mask")),
            Action::M2(Edit::RemoveMask(layer))
        );
        if lang == Lang::En {
            for text in names(&v).into_iter().flatten() {
                assert!(!has_japanese(&text), "{text}");
            }
        }
    }
}

// ───────── レイヤーのメニュー ─────────

#[test]
fn the_layer_menu_is_one_list_for_the_menu_bar_and_the_right_click() {
    for lang in Lang::ALL {
        let mut s = AppState::new(64, 64);
        s.lang = lang;
        let id = s.selected_layer.unwrap();
        let bar = shell::menu_entries(&s, 2);
        let context = shell::popup_entries(&s, PopupKind::LayerContext(id));
        assert_eq!(shape(&bar), shape(&context), "{lang:?}: 同じ並び");
        // 選んだレイヤーが無いとき: メニューバーと一覧の空白の右クリックが同じ並び（足す項目とグループだけ）
        s.selected_layer = None;
        let bar = shell::menu_entries(&s, 2);
        let blank = shell::popup_entries(&s, PopupKind::M2(yolu_app::m2_menu::Popup::LayerBlank));
        assert_eq!(shape(&bar), shape(&blank), "{lang:?}");
        let labels: Vec<_> = names(&bar);
        assert_eq!(
            labels,
            [
                Some(lang.pick("新規レイヤー", "New Layer").to_owned()),
                Some(
                    lang.pick("新規塗りつぶしレイヤー", "New Fill Layer")
                        .to_owned()
                ),
                Some(
                    lang.pick("新規調整レイヤー", "New Adjustment Layer")
                        .to_owned()
                ),
                Some(lang.pick("新規パスレイヤー", "New Path Layer").to_owned()),
                None,
                Some(lang.pick("新規グループ", "New Group").to_owned()),
            ]
        );
    }
}

#[test]
fn the_new_path_layer_entry_is_in_the_menu_bar_and_the_right_click_but_not_in_the_layers_toolbar() {
    for lang in Lang::ALL {
        let label = lang.pick("新規パスレイヤー", "New Path Layer");
        // 並びの項目: 押せる・描いている最中は押せない・キーは持たない
        let mut s = AppState::new(64, 64);
        s.lang = lang;
        let id = s.selected_layer.unwrap();
        for entries in [
            shell::menu_entries(&s, 2),
            shell::popup_entries(&s, PopupKind::LayerContext(id)),
        ] {
            let item = entries
                .iter()
                .find(|e| e.label() == Some(label))
                .unwrap_or_else(|| panic!("{lang:?}: {:?}", names(&entries)));
            assert!(
                matches!(item, Entry::Item { enabled: true, shortcut: None, action, .. }
                    if *action == Action::LayerMenu(Op::PathLayer)),
                "{lang:?}"
            );
        }
        let settings = s.stroke_settings(false);
        let stroke = s.doc.begin_stroke(id, &settings).unwrap();
        let bar = shell::menu_entries(&s, 2);
        assert!(
            bar.iter().any(
                |e| e.label() == Some(label) && matches!(e, Entry::Item { enabled: false, .. })
            ),
            "{lang:?}: 描いている最中は押せない"
        );
        s.doc.cancel_stroke(stroke);
        // 画面: レイヤーの一覧の下の帯のボタンには無く、メニューバーの「レイヤー」には出る。押すと作ってパスのツールへ替える
        let mut h = app(1280.0, 800.0, 64);
        h.state_mut().state.lang = lang;
        h.run();
        assert!(
            h.query_by_label(label).is_none(),
            "{lang:?}: 帯には置かない"
        );
        let base = h.state().state.selected_layer.unwrap();
        let at = menu_title(&h, lang.pick("レイヤー", "Layer")).center();
        click(&mut h, at);
        let item = popup_item(&h, label);
        click(&mut h, item.center());
        assert!(h.state().state.popup.is_none(), "選んだら閉じる");
        let made = h.state().state.selected_layer.unwrap();
        assert_ne!(made, base);
        assert!(h.state().state.doc.layer(made).unwrap().has_paths());
        assert_eq!(h.state().state.tool, Tool::Path);
        assert_eq!(h.state().state.doc.layers().len(), 2);
        h.state_mut().state.apply(Action::Undo);
        assert_eq!(
            h.state().state.doc.layers().len(),
            1,
            "1 回の取り消しで戻る"
        );
    }
}

#[test]
fn the_layer_menu_goes_add_then_effects_then_groups_then_the_rest() {
    for lang in Lang::ALL {
        let mut s = AppState::new(64, 64);
        s.lang = lang;
        let v = shell::menu_entries(&s, 2);
        let labels = names(&v);
        let ja_en = |ja: &str, en: &str| Some(lang.pick(ja, en).to_owned());
        let head = [
            ja_en("新規レイヤー", "New Layer"),
            ja_en("新規塗りつぶしレイヤー", "New Fill Layer"),
            ja_en("新規調整レイヤー", "New Adjustment Layer"),
            ja_en("新規パスレイヤー", "New Path Layer"),
            None,
            ja_en("フィルターを追加", "Add Filter"),
            ja_en("ジェネレーターを追加", "Add Generator"),
            ja_en("アンカーを置く", "Add Anchor"),
            None,
            ja_en("新規グループ", "New Group"),
            ja_en("レイヤーをグループ化", "Group Layers"),
            None,
            ja_en("複製", "Duplicate"),
        ];
        assert_eq!(&labels[..head.len()], &head, "{lang:?}: {labels:?}");
        // 新規レイヤーのキー
        assert!(matches!(&v[0], Entry::Item { shortcut: Some(k), .. } if k == "Ctrl+Shift+N"));
        // 参照レイヤーは先頭でなく、属性（クリッピング・マスク・ロック）の組の中
        let at = |name: &str| {
            labels
                .iter()
                .position(|l| l.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("{name}: {labels:?}"))
        };
        let reference = at(lang.pick("参照レイヤー", "Reference Layer"));
        assert!(reference > at(lang.pick("複製", "Duplicate")));
        assert_eq!(reference + 1, at(lang.pick("クリッピング", "Clipping")));
        assert!(reference < at(lang.pick("レイヤーマスクを追加", "Add Layer Mask")));
        assert!(
            at(lang.pick("レイヤーマスクを追加", "Add Layer Mask"))
                < at(lang.pick("透明部分をロック", "Lock Transparent Pixels"))
        );
        // 新規グループは、新規レイヤーの組ではなくグループ化の組
        assert_eq!(
            at(lang.pick("新規グループ", "New Group")) + 1,
            at(lang.pick("レイヤーをグループ化", "Group Layers"))
        );
        // 理由・種類を「: 」でつないだ平らな項目（旧「新規調整レイヤー: …」）は無い
        for l in labels.iter().flatten() {
            assert!(!l.contains(": ") && !l.contains(" — "), "{l}");
        }
        // 入れ子の中は、フィルター・ジェネレーター・調整（全部）・塗りつぶし 4 種
        let filters = submenu(&v, lang.pick("フィルターを追加", "Add Filter"));
        assert_eq!(filters.len(), yolu_app::fx::FilterKind::ALL.len());
        let generators = submenu(&v, lang.pick("ジェネレーターを追加", "Add Generator"));
        assert_eq!(generators.len(), yolu_app::fx::names::GENERATOR_KINDS.len());
        let adjustments = submenu(&v, lang.pick("新規調整レイヤー", "New Adjustment Layer"));
        let expected: Vec<Option<String>> = AdjustmentKind::ALL
            .iter()
            .map(|k| Some(k.name(lang).to_owned()))
            .collect();
        assert_eq!(names(adjustments), expected, "{lang:?}: 調整の種類は全部");
        let fills = submenu(&v, lang.pick("新規塗りつぶしレイヤー", "New Fill Layer"));
        assert_eq!(
            names(fills),
            [
                ja_en("単色", "Solid Color"),
                ja_en("グラデーションデカール", "Gradient Decal"),
                ja_en("画像", "Image"),
                ja_en("デカール", "Decal"),
            ]
        );
        if lang == Lang::En {
            assert!(
                labels.iter().flatten().all(|l| !has_japanese(l)),
                "{labels:?}"
            );
        }
    }
}

#[test]
fn the_fill_submenus_list_the_shelf_images_and_the_file_import() {
    for lang in Lang::ALL {
        let mut s = AppState::new(64, 64);
        s.lang = lang;
        let import = lang.pick("ファイルから取り込む…", "Import from File…");
        let fills = submenu(
            &shell::menu_entries(&s, 2),
            lang.pick("新規塗りつぶしレイヤー", "New Fill Layer"),
        )
        .to_vec();
        // 棚が空なら「ファイルから取り込む…」だけ
        for (kind, mode) in [
            ("画像", ProjectionMode::Uv),
            ("デカール", ProjectionMode::Decal),
        ] {
            let kind = match (lang, kind) {
                (Lang::En, "画像") => "Image",
                (Lang::En, _) => "Decal",
                _ => kind,
            };
            let inner = submenu(&fills, kind);
            assert_eq!(names(inner), [Some(import.to_owned())], "{lang:?} {kind}");
            assert!(
                matches!(&inner[0], Entry::Item { action: Action::LayerMenu(Op::FillImageDialog(m)), .. } if *m == mode)
            );
        }
        // 棚に画像があれば、その一覧（寸法つき）→ 区切り → 取り込み
        let (_, image) = shelf_image(&mut s, "四色");
        let fills = submenu(
            &shell::menu_entries(&s, 2),
            lang.pick("新規塗りつぶしレイヤー", "New Fill Layer"),
        )
        .to_vec();
        let inner = submenu(&fills, lang.pick("画像", "Image")).to_vec();
        assert_eq!(
            names(&inner),
            [
                Some("四色  (2 × 2)".to_owned()),
                None,
                Some(import.to_owned())
            ]
        );
        assert!(
            matches!(&inner[0], Entry::Item { action: Action::LayerMenu(Op::FillImage { image: i, mode: ProjectionMode::Uv }), .. } if *i == image)
        );
        let inner = submenu(&fills, lang.pick("デカール", "Decal")).to_vec();
        assert!(matches!(
            &inner[0],
            Entry::Item {
                action: Action::LayerMenu(Op::FillImage {
                    mode: ProjectionMode::Decal,
                    ..
                }),
                ..
            }
        ));
    }
}

// ───────── 塗りつぶしの作成 ─────────

fn layer_count(s: &AppState) -> usize {
    s.doc.layers().len()
}

#[test]
fn a_menu_image_fill_adds_one_fill_layer_with_the_image_and_one_undo_takes_it_back() {
    let mut s = AppState::new(64, 64);
    let (_, image) = shelf_image(&mut s, "四色");
    let below = s.selected_layer.unwrap();
    let steps = s.doc.undo_count();
    s.apply(Action::LayerMenu(Op::FillImage {
        image,
        mode: ProjectionMode::Uv,
    }));
    let id = s.selected_layer.expect("足したレイヤーを選ぶ");
    assert_ne!(id, below);
    assert_eq!(layer_count(&s), 2);
    let layer = s.doc.layer(id).unwrap();
    assert_eq!(layer.kind(), LayerKind::Fill);
    assert_eq!(layer.name(), "四色");
    assert_eq!(layer.fill_image(Channel::Color), Some(image));
    assert_eq!(layer.projection().mode, ProjectionMode::Uv);
    assert_eq!(s.doc.undo_count(), steps + 1, "{}", s.message);
    assert!(
        s.message.starts_with("画像の塗りつぶしを追加しました"),
        "{}",
        s.message
    );
    // 選んでいたレイヤーの上に重なる
    let order: Vec<_> = s.doc.layers().iter().map(|l| l.id()).collect();
    assert!(order.iter().position(|l| *l == id) > order.iter().position(|l| *l == below));
    // 1 回の取り消しでレイヤーごと戻り、やり直しで戻る
    s.apply(Action::Undo);
    assert_eq!(layer_count(&s), 1);
    assert!(s.doc.layer(id).is_none());
    s.apply(Action::Redo);
    assert_eq!(
        s.doc.layer(id).unwrap().fill_image(Channel::Color),
        Some(image)
    );
}

#[test]
fn a_menu_decal_is_a_fill_layer_with_the_decal_projection_fitted_to_the_model() {
    // モデルが無くても作れる（置き場は初めのまま）
    let mut s = AppState::new(64, 64);
    let (_, image) = shelf_image(&mut s, "四色");
    s.apply(Action::LayerMenu(Op::FillImage {
        image,
        mode: ProjectionMode::Decal,
    }));
    let id = s.selected_layer.unwrap();
    let p = *s.doc.layer(id).unwrap().projection();
    assert_eq!(p.mode, ProjectionMode::Decal);
    assert_eq!(p.wrap, Wrap::None, "デカールは 1 回だけ（繰り返さない）");
    assert_eq!(
        s.doc.layer(id).unwrap().fill_image(Channel::Color),
        Some(image)
    );
    assert!(
        s.message.starts_with("デカールを置きました"),
        "{}",
        s.message
    );

    // 試しの立方体があれば、今の 3D ビューから見て正面に合わせる
    let mut s = AppState::new(64, 64);
    s.apply(Action::LoadDemoModel);
    let (_, image) = shelf_image(&mut s, "四色");
    let steps = s.doc.undo_count();
    s.apply(Action::LayerMenu(Op::FillImage {
        image,
        mode: ProjectionMode::Decal,
    }));
    let id = s.selected_layer.unwrap();
    let p = *s.doc.layer(id).unwrap().projection();
    assert_eq!(p.mode, ProjectionMode::Decal);
    assert_ne!(
        p.placement,
        Placement::default(),
        "モデルの外形に合わせた置き場"
    );
    assert!(!s.fillfx.handles_hidden, "置き場のハンドルを出す");
    assert_eq!(
        s.doc.undo_count(),
        steps + 1,
        "レイヤー・画像・投影が 1 回の Undo"
    );
    s.apply(Action::Undo);
    assert!(s.doc.layer(id).is_none());
}

#[test]
fn a_menu_gradient_fill_is_one_undo_and_opens_the_shape_for_editing() {
    let mut s = AppState::new(64, 64);
    let steps = s.doc.undo_count();
    s.apply(Action::LayerMenu(Op::FillGradient(Shape::Box)));
    let id = s.selected_layer.unwrap();
    let layer = s.doc.layer(id).unwrap();
    assert_eq!(layer.kind(), LayerKind::Fill);
    assert!(layer.fill_gradient(Channel::Color).is_some());
    assert_eq!(
        s.fillfx.edit_gradient,
        Some((id, Channel::Color)),
        "3D ビューで形を編集できる"
    );
    assert_eq!(s.doc.undo_count(), steps + 1, "{}", s.message);
    s.apply(Action::Undo);
    assert!(s.doc.layer(id).is_none());
    assert_eq!(layer_count(&s), 1);
}

#[test]
fn the_gradient_entry_is_a_nested_choice_of_shapes_named_like_the_fill_panel() {
    use yolu_app::panels::fill_props::shape_name;
    for lang in Lang::ALL {
        let mut s = AppState::new(64, 64);
        s.lang = lang;
        let fills = submenu(
            &shell::menu_entries(&s, 2),
            lang.pick("新規塗りつぶしレイヤー", "New Fill Layer"),
        )
        .to_vec();
        let gradient = submenu(
            &fills,
            lang.pick("グラデーションデカール", "Gradient Decal"),
        );
        // 形の名前と順は、塗りつぶしの欄の「形」の選びと同じ
        let shapes = [Shape::Box, Shape::Sphere, Shape::Plane];
        assert_eq!(
            names(gradient),
            shapes.map(|shape| Some(shape_name(lang, shape).to_owned()))
        );
        for (entry, shape) in gradient.iter().zip(shapes) {
            assert!(
                matches!(entry, Entry::Item { action: Action::LayerMenu(Op::FillGradient(got)), enabled: true, tooltip: Some(_), .. } if *got == shape),
                "{lang:?} {shape:?}: {entry:?}"
            );
        }
        // 日本語の画面に英語、英語の画面に日本語を出さない。名前・ツールチップは短く、使い方の文を置かない
        for entry in fills.iter().chain(gradient) {
            if let Entry::Item { label, tooltip, .. } | Entry::Submenu { label, tooltip, .. } =
                entry
            {
                for text in std::iter::once(label).chain(tooltip) {
                    assert_plain(&format!("{lang:?} {text}"), text);
                }
                assert_eq!(has_japanese(label), lang == Lang::Ja, "{label}");
                if let Some(tip) = tooltip {
                    assert_eq!(has_japanese(tip), lang == Lang::Ja, "{tip}");
                }
            }
        }
        // 描いている間は選べない
        let mut s = AppState::new(64, 64);
        s.lang = lang;
        let layer = s.selected_layer.unwrap();
        let brush = s.stroke_settings(false);
        let stroke = s.doc.begin_stroke(layer, &brush).unwrap();
        let fills = submenu(
            &shell::menu_entries(&s, 2),
            lang.pick("新規塗りつぶしレイヤー", "New Fill Layer"),
        )
        .to_vec();
        let gradient = submenu(
            &fills,
            lang.pick("グラデーションデカール", "Gradient Decal"),
        );
        assert!(leaves(gradient)
            .iter()
            .all(|e| matches!(e, Entry::Item { enabled: false, .. })));
        s.doc.cancel_stroke(stroke);
    }
}

#[test]
fn the_fill_panels_shape_choice_lists_the_same_names_in_the_same_order_as_the_menu() {
    for lang in Lang::ALL {
        let mut s = AppState::new(64, 64);
        s.lang = lang;
        s.apply(Action::LayerMenu(Op::FillGradient(Shape::Sphere)));
        let id = s.selected_layer.unwrap();
        let panel = yolu_app::panels::fill_props::entries(
            &s,
            yolu_app::m2_menu::Popup::GradientShape(id, Channel::Color),
        );
        let fills = submenu(
            &shell::menu_entries(&s, 2),
            lang.pick("新規塗りつぶしレイヤー", "New Fill Layer"),
        )
        .to_vec();
        let menu = submenu(
            &fills,
            lang.pick("グラデーションデカール", "Gradient Decal"),
        );
        assert_eq!(names(&panel), names(menu), "{lang:?}");
        assert_eq!(names(&panel).len(), 3);
    }
}

#[test]
fn each_shape_makes_one_fill_layer_of_that_shape_fitted_to_the_model_with_one_undo() {
    for shape in [Shape::Box, Shape::Sphere, Shape::Plane] {
        let mut s = AppState::new(64, 64);
        s.apply(Action::LoadDemoModel);
        let b = s.model_bounds().expect("試しの立方体の外形");
        let full = b.extents * 2.0;
        let center = [
            f64::from(b.center.x),
            f64::from(b.center.y),
            f64::from(b.center.z),
        ];
        let layers = layer_count(&s);
        let steps = s.doc.undo_count();
        s.apply(Action::LayerMenu(Op::FillGradient(shape)));
        let id = s.selected_layer.unwrap();
        let g = s
            .doc
            .layer(id)
            .unwrap()
            .fill_gradient(Channel::Color)
            .unwrap()
            .clone();
        assert_eq!(g.volume.shape, shape, "{shape:?}: {}", s.message);
        assert_eq!(g.volume.center, center, "{shape:?}: 外形の中央");
        assert!(g.ramp.is_some() && g.validate().is_ok());
        let near = |a: f64, b: f32| (a - f64::from(b)).abs() < 1e-5;
        match shape {
            Shape::Box => assert!(near(g.volume.size[1], full.y * 0.5), "高さは外形の半分"),
            Shape::Sphere => assert!(
                g.volume
                    .size
                    .iter()
                    .all(|d| near(*d, full.x.max(full.y).max(full.z) * 0.75)),
                "直径は一番長い辺の 4 分の 3"
            ),
            Shape::Plane => assert!(
                near(g.volume.size[1], full.y),
                "幅は外形の高さ（下が 0・上が 1）"
            ),
        }
        assert_eq!(
            s.fillfx.edit_gradient,
            Some((id, Channel::Color)),
            "{shape:?}"
        );
        assert_eq!(layer_count(&s), layers + 1);
        assert_eq!(s.doc.undo_count(), steps + 1, "{shape:?}: 1 回の Undo");
        assert!(
            s.message.contains("グラデーションデカール"),
            "{}",
            s.message
        );
        assert_plain(&format!("{shape:?} 知らせ"), &s.message);
        s.apply(Action::Undo);
        assert!(s.doc.layer(id).is_none(), "{shape:?}");
        assert_eq!(layer_count(&s), layers);
        // 英語の画面の知らせ
        s.lang = Lang::En;
        s.apply(Action::LayerMenu(Op::FillGradient(shape)));
        assert!(
            !has_japanese(&s.message),
            "英語の画面に日本語: {}",
            s.message
        );
        assert!(
            s.message.starts_with("Gradient decal added"),
            "{}",
            s.message
        );
        assert_plain(&format!("{shape:?} message"), &s.message);
    }
}

#[test]
fn a_shape_gradient_from_the_menu_is_saved_with_its_shape_and_placement() {
    // アプリで開く道は位置のマップが無いと読むだけになるので、保存した正本を core の文書へ戻して見る（デカールの試験と同じ）
    for shape in [Shape::Box, Shape::Sphere, Shape::Plane] {
        let mut s = AppState::new(64, 64);
        s.apply(Action::LoadDemoModel);
        s.apply(Action::LayerMenu(Op::FillGradient(shape)));
        let id = s.selected_layer.unwrap();
        let settings = s
            .doc
            .layer(id)
            .unwrap()
            .fill_gradient(Channel::Color)
            .cloned()
            .unwrap();
        let dir = temp_dir(&format!("gradient-shape-{shape:?}"));
        let path = dir.join("g.ylp");
        s.apply(Action::SaveProjectAs(path.clone()));
        assert!(s.message.starts_with("保存しました"), "{}", s.message);
        let project = yolu_io::Project::read(&std::fs::read(&path).unwrap()).expect("読める");
        let restored = project.sets()[0].document.to_core().expect("core の文書");
        let g = restored
            .layers()
            .iter()
            .find(|l| l.id() == id)
            .and_then(|l| l.fill_gradient(Channel::Color))
            .unwrap_or_else(|| panic!("{shape:?}: 保存したレイヤーに無い"));
        assert_eq!(g.volume.shape, shape);
        assert_eq!(g, &settings, "{shape:?}: 置き場も同じ");
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn choosing_a_file_asks_for_it_and_changes_nothing_until_a_file_is_chosen() {
    let mut s = AppState::new(64, 64);
    let revision = s.doc.revision();
    let steps = s.doc.undo_count();
    s.apply(Action::LayerMenu(Op::FillImageDialog(
        ProjectionMode::Decal,
    )));
    assert_eq!(
        s.dialog_request,
        Some(DialogRequest::NewFillImage(ProjectionMode::Decal)),
        "ファイルのウィンドウを頼む"
    );
    // 選ばずに閉じる（ウィンドウの結果が来ない）と、何も作らず、履歴も増えない
    s.dialog_request = None;
    assert_eq!(layer_count(&s), 1);
    assert_eq!(s.doc.undo_count(), steps);
    assert_eq!(s.doc.revision(), revision);
    assert!(s.shelf.resources().iter().all(|r| r.kind != "image"));
}

#[test]
fn a_chosen_file_goes_to_the_shelf_and_makes_the_layer_and_a_bad_file_makes_nothing() {
    let dir = temp_dir("file");
    let ok = dir.join("四色.png");
    image::RgbaImage::from_raw(2, 2, quad_image())
        .unwrap()
        .save(&ok)
        .unwrap();
    let mut s = AppState::new(64, 64);
    let steps = s.doc.undo_count();
    s.apply(Action::LayerMenu(Op::FillImageFile {
        path: ok.clone(),
        mode: ProjectionMode::Uv,
    }));
    let id = s.selected_layer.unwrap();
    let layer = s.doc.layer(id).unwrap();
    assert_eq!(layer.kind(), LayerKind::Fill, "{}", s.message);
    assert_eq!(layer.name(), "四色");
    assert!(layer.fill_image(Channel::Color).is_some());
    assert!(
        s.shelf.resources().iter().any(|r| r.kind == "image"),
        "棚に入る"
    );
    assert_eq!(
        s.doc.undo_count(),
        steps + 1,
        "文書の履歴はレイヤーを作った 1 回だけ"
    );
    // 同じファイルをもう一度選ぶと、棚には足さずにその画像でレイヤーを作る
    let before = s.shelf.resources().len();
    s.apply(Action::LayerMenu(Op::FillImageFile {
        path: ok,
        mode: ProjectionMode::Decal,
    }));
    assert_eq!(s.shelf.resources().len(), before);
    assert_eq!(layer_count(&s), 3);

    // 読めないファイル・無いファイル: 何も作らず、棚の選びも変えない
    let (rid, _) = shelf_image(&mut s, "もう 1 枚");
    s.shelf.selected = Some(rid.clone());
    let junk = dir.join("junk.png");
    std::fs::write(&junk, b"not a png").unwrap();
    let steps = s.doc.undo_count();
    let layers = layer_count(&s);
    for path in [junk, dir.join("missing.png")] {
        s.apply(Action::LayerMenu(Op::FillImageFile {
            path,
            mode: ProjectionMode::Uv,
        }));
        assert_eq!(layer_count(&s), layers, "{}", s.message);
        assert_eq!(s.doc.undo_count(), steps);
        assert_eq!(s.shelf.selected, Some(rid.clone()), "選びを元へ戻す");
        assert!(!s.message.is_empty());
    }
}

#[test]
fn an_image_that_cannot_be_used_makes_no_layer_and_says_why() {
    // 棚に無い画像
    let mut s = AppState::new(64, 64);
    let steps = s.doc.undo_count();
    s.apply(Action::LayerMenu(Op::FillImage {
        image: ImageId(0x1234_5678),
        mode: ProjectionMode::Uv,
    }));
    assert_eq!(layer_count(&s), 1);
    assert_eq!(s.doc.undo_count(), steps);
    assert_eq!(s.message, "アセットに画像がありません");
    s.lang = Lang::En;
    s.apply(Action::LayerMenu(Op::FillImage {
        image: ImageId(0x1234_5678),
        mode: ProjectionMode::Uv,
    }));
    assert_eq!(s.message, "No such image in the project's assets");

    // 復号の予算を超える画像: 断られた画像は誰も持たない
    let mut s = AppState::new(64, 64);
    let (_, image) = shelf_image(&mut s, "四色");
    s.fx.inputs.image_limit = Some(10);
    s.apply(Action::LayerMenu(Op::FillImage {
        image,
        mode: ProjectionMode::Uv,
    }));
    assert_eq!(layer_count(&s), 1);
    assert_eq!(s.doc.undo_count(), 0);
    assert!(s.message.contains("予算"), "{}", s.message);
    s.sync_effects();
    assert_eq!(s.fx.inputs.decoded_image_count(), 0);
}

/// 断られた操作が何も残していない: レイヤーの数・Undo の段・復号している画像（断られた画像を予算に残さない）。
fn assert_nothing_made(s: &mut AppState, layers: usize, steps: usize, what: &str) {
    assert_eq!(layer_count(s), layers, "{what}: {}", s.message);
    assert_eq!(s.doc.undo_count(), steps, "{what}");
    assert!(!s.message.is_empty(), "{what}: 理由を出す");
    s.sync_effects();
    assert_eq!(s.fx.inputs.decoded_image_count(), 0, "{what}: 画像を手放す");
}

/// 四色の PNG をファイルへ書く（ファイルから取り込む道の試験用）。
fn quad_png(name: &str) -> PathBuf {
    let path = temp_dir(name).join("四色.png");
    image::RgbaImage::from_raw(2, 2, quad_image())
        .unwrap()
        .save(&path)
        .unwrap();
    path
}

#[test]
fn nothing_is_made_while_drawing() {
    let mut s = AppState::new(64, 64);
    let (_, image) = shelf_image(&mut s, "四色");
    let file = quad_png("drawing");
    let layer = s.selected_layer.unwrap();
    let brush = s.stroke_settings(false);
    let stroke = s.doc.begin_stroke(layer, &brush).unwrap();
    let shelf = s.shelf.resources().len();
    for op in [
        Op::FillImage {
            image,
            mode: ProjectionMode::Uv,
        },
        Op::FillImage {
            image,
            mode: ProjectionMode::Decal,
        },
        Op::FillGradient(Shape::Box),
        // ファイルの取り込みも、描いている間は棚へ入れず、レイヤーも作らない
        Op::FillImageFile {
            path: file.clone(),
            mode: ProjectionMode::Decal,
        },
    ] {
        s.message.clear();
        s.apply(Action::LayerMenu(op.clone()));
        assert_eq!(s.message, "描いている間はできません。", "{op:?}");
        assert_eq!(layer_count(&s), 1, "{op:?}");
        assert_eq!(s.shelf.resources().len(), shelf, "{op:?}: 棚も変えない");
    }
    s.lang = Lang::En;
    s.apply(Action::LayerMenu(Op::FillImageFile {
        path: file,
        mode: ProjectionMode::Uv,
    }));
    assert_eq!(s.message, "Not while drawing.");
    s.doc.cancel_stroke(stroke);
}

#[test]
fn nothing_is_made_inside_a_locked_group_and_the_image_is_let_go() {
    use yolu_core::LayerLocks;
    let file = quad_png("locked");
    for lock in [
        LayerLocks::PIXELS,
        LayerLocks::TRANSPARENCY,
        LayerLocks::ALL,
    ] {
        let mut s = AppState::new(64, 64);
        let (_, image) = shelf_image(&mut s, "四色");
        let child = s.selected_layer.unwrap();
        let group = s.doc.group_layers(&[child], "G").unwrap();
        s.doc.set_layer_locks(group, lock).unwrap();
        s.selected_layer = Some(child);
        let (layers, steps) = (layer_count(&s), s.doc.undo_count());
        for op in [
            Op::FillImage {
                image,
                mode: ProjectionMode::Uv,
            },
            Op::FillImage {
                image,
                mode: ProjectionMode::Decal,
            },
            Op::FillGradient(Shape::Box),
            Op::FillImageFile {
                path: file.clone(),
                mode: ProjectionMode::Uv,
            },
        ] {
            s.message.clear();
            s.apply(Action::LayerMenu(op.clone()));
            let what = format!("{lock:?} {op:?}");
            assert!(
                s.message.contains("ロック"),
                "{what}: 親のロックを理由に出す: {}",
                s.message
            );
            assert!(s.fillfx.edit_gradient.is_none(), "{what}");
            // 棚へ取り込むファイルのほかは、何も変えていないので変更の印も付けない
            assert!(
                s.modified == matches!(op, Op::FillImageFile { .. }),
                "{what}: 変更の印 {}",
                s.modified
            );
            // 取り込んだファイルの画像は棚に残るが、どのレイヤーも指さず、復号したままにしない
            assert_nothing_made(&mut s, layers, steps, &what);
        }
    }
}

#[test]
fn a_gradient_fill_on_the_normal_channel_makes_no_layer_and_says_why() {
    let mut s = AppState::new(64, 64);
    s.m2.paint_channel = Channel::Normal;
    let (layers, steps) = (layer_count(&s), s.doc.undo_count());
    s.apply(Action::LayerMenu(Op::FillGradient(Shape::Box)));
    assert_eq!(layer_count(&s), layers, "{}", s.message);
    assert_eq!(
        s.doc.undo_count(),
        steps,
        "レイヤーは 1 回の Undo の中で戻る"
    );
    assert!(!s.modified, "変更の印を付けない");
    assert!(s.message.contains("法線"), "{}", s.message);
    assert!(s.fillfx.edit_gradient.is_none(), "形の編集を始めない");
    s.lang = Lang::En;
    s.apply(Action::LayerMenu(Op::FillGradient(Shape::Box)));
    assert!(
        !has_japanese(&s.message),
        "英語の画面に日本語を出さない: {}",
        s.message
    );
    // 法線の画像の塗りつぶしは作れる（画像は法線として差す）。ここは断られないことだけ確かめる
    let (_, image) = shelf_image(&mut s, "四色");
    s.apply(Action::LayerMenu(Op::FillImage {
        image,
        mode: ProjectionMode::Uv,
    }));
    assert_eq!(layer_count(&s), layers + 1, "{}", s.message);
}

#[test]
fn a_file_over_the_image_budget_makes_no_layer_and_the_decal_is_the_same() {
    let file = quad_png("budget");
    for mode in [ProjectionMode::Uv, ProjectionMode::Decal] {
        let mut s = AppState::new(64, 64);
        s.fx.inputs.image_limit = Some(10);
        let (layers, steps) = (layer_count(&s), s.doc.undo_count());
        s.apply(Action::LayerMenu(Op::FillImageFile {
            path: file.clone(),
            mode,
        }));
        assert!(s.message.contains("予算"), "{mode:?}: {}", s.message);
        assert_nothing_made(&mut s, layers, steps, &format!("{mode:?}"));
        // 取り込んだ画像は棚に残る（消えるのはレイヤーを作らなかったことだけ）。棚の画像から作り直しても、同じ断りで何も作らない
        let image = s
            .shelf
            .selected
            .as_deref()
            .and_then(inputs::image_id)
            .expect("取り込んだ画像を選んでいる");
        s.message.clear();
        s.apply(Action::LayerMenu(Op::FillImage { image, mode }));
        assert!(s.message.contains("予算"), "{mode:?}: {}", s.message);
        assert_nothing_made(&mut s, layers, steps, &format!("{mode:?} 棚から"));
    }
}

#[test]
fn the_menu_the_image_fields_projection_switch_and_place_decal_make_the_same_projection() {
    use yolu_app::fillfx::FillOp;
    use yolu_core::fill_image::Projection;
    let mut s = AppState::new(64, 64);
    s.apply(Action::LoadDemoModel);
    s.view3d.camera.yaw = -40.0;
    s.view3d.camera.pitch = 15.0;
    let (_, image) = shelf_image(&mut s, "四色");
    let projection = |s: &AppState| *s.doc.layer(s.selected_layer.unwrap()).unwrap().projection();
    for mode in [
        ProjectionMode::Triplanar,
        ProjectionMode::Planar,
        ProjectionMode::Spherical,
        ProjectionMode::Cylindrical,
        ProjectionMode::Decal,
    ] {
        // メニューが作るレイヤーの投影
        s.apply(Action::LayerMenu(Op::FillImage { image, mode }));
        let from_menu = projection(&s);
        assert_eq!(from_menu.mode, mode);
        assert_ne!(from_menu.placement, Placement::default(), "{mode:?}");
        // メニューで UV の画像のレイヤーを作り、画像の欄で投影の種類を替えたレイヤー（置き場は外形・今のビューに合わせる）
        s.apply(Action::LayerMenu(Op::FillImage {
            image,
            mode: ProjectionMode::Uv,
        }));
        let layer = s.selected_layer.unwrap();
        s.apply(Action::Fill(FillOp::ProjectionMode { layer, mode }));
        assert_eq!(
            projection(&s),
            from_menu,
            "{mode:?}: 欄で替えたレイヤーと同じ"
        );
    }
    // 3D ビューへ落として置くデカール: 置き場は当たった点で決まるので、置き場のほかは同じ（種類・繰り返さない・減衰）
    s.apply(Action::LayerMenu(Op::FillImage {
        image,
        mode: ProjectionMode::Decal,
    }));
    let from_menu = projection(&s);
    s.apply(Action::Fill(FillOp::PlaceDecal {
        image,
        at: pos2(400.0, 300.0),
        rect: Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0)),
    }));
    let placed = projection(&s);
    assert_eq!(placed.mode, ProjectionMode::Decal);
    let without_placement = |p: Projection| Projection {
        placement: Placement::default(),
        ..p
    };
    assert_eq!(without_placement(placed), without_placement(from_menu));
}

#[test]
fn image_and_decal_fills_survive_saving_and_opening_and_stay_editable() {
    let dir = temp_dir("save");
    // 画像の塗りつぶし（UV）: アプリで開き直しても同じレイヤー・同じ画像で、続けて編集できる
    let path = dir.join("image.ylp");
    let mut s = AppState::new(64, 64);
    let (rid, image) = shelf_image(&mut s, "四色");
    s.apply(Action::LayerMenu(Op::FillImage {
        image,
        mode: ProjectionMode::Uv,
    }));
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let mut t = AppState::new(8, 8);
    t.apply(Action::OpenProject(path));
    assert!(t.message.starts_with("開きました"), "{}", t.message);
    t.sync_effects();
    assert_eq!(t.read_only_reason(), None);
    assert!(t.shelf.get(&rid).is_some());
    let fills: Vec<_> = t
        .doc
        .layers()
        .iter()
        .filter(|l| l.kind() == LayerKind::Fill)
        .map(|l| {
            (
                l.name().to_owned(),
                l.fill_image(Channel::Color),
                l.projection().mode,
            )
        })
        .collect();
    assert_eq!(
        fills,
        [("四色".to_owned(), Some(image), ProjectionMode::Uv)]
    );
    // 開いたあとも、メニューから足せて、1 回の Undo
    let steps = t.doc.undo_count();
    t.apply(Action::LayerMenu(Op::FillImage {
        image,
        mode: ProjectionMode::Uv,
    }));
    assert_eq!(t.doc.undo_count(), steps + 1, "{}", t.message);

    // デカールとグラデーション: 保存した正本のレイヤーが、同じ画像・投影・グラデーションを持つ
    // （アプリで開く道は、デカールの位置のマップが無いので読むだけにする。ここでは正本を core の文書へ戻して見る）
    let path = dir.join("decal.ylp");
    let mut s = AppState::new(64, 64);
    s.apply(Action::LoadDemoModel);
    let (rid, image) = shelf_image(&mut s, "四色");
    s.apply(Action::LayerMenu(Op::FillImage {
        image,
        mode: ProjectionMode::Decal,
    }));
    let decal = s.selected_layer.unwrap();
    let decal_projection = *s.doc.layer(decal).unwrap().projection();
    s.apply(Action::LayerMenu(Op::FillGradient(Shape::Box)));
    let gradient = s.selected_layer.unwrap();
    let settings = s
        .doc
        .layer(gradient)
        .unwrap()
        .fill_gradient(Channel::Color)
        .cloned()
        .unwrap();
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let project = yolu_io::Project::read(&std::fs::read(&path).unwrap()).expect("読める");
    let restored = project.sets()[0]
        .document
        .to_core()
        .expect("core の文書にできる");
    let find = |id| {
        restored
            .layers()
            .iter()
            .find(|l| l.id() == id)
            .unwrap()
            .clone()
    };
    let (d, g) = (find(decal), find(gradient));
    assert_eq!(d.fill_image(Channel::Color), Some(image));
    assert_eq!(*d.projection(), decal_projection);
    assert_eq!(d.projection().mode, ProjectionMode::Decal);
    assert_eq!(d.name(), "四色");
    assert_eq!(g.fill_gradient(Channel::Color), Some(&settings));
    assert!(
        project.resources().iter().any(|r| r.id == rid),
        "棚の画像も保存される"
    );
}

// ───────── 画面の操作 ─────────

/// 開いているポップアップの、入れ子の段数。
fn depth(h: &Harness<'_, YoluApp>) -> usize {
    h.state()
        .state
        .popup
        .as_ref()
        .map_or(0, |p| p.state.open_depth())
}

fn hover(h: &mut Harness<'_, YoluApp>, at: egui::Pos2) {
    h.event(Event::PointerMoved(at));
    h.step();
    h.step();
}

#[test]
fn the_menu_bar_opens_the_fill_submenu_on_hover_and_a_click_in_it_makes_the_layer() {
    let mut h = app(1280.0, 800.0, 64);
    let at = menu_title(&h, "レイヤー").center();
    click(&mut h, at);
    assert_eq!(
        h.state().state.popup.as_ref().map(|p| p.kind),
        Some(PopupKind::MenuBar(2))
    );
    // ポインタが前の位置を持つように 1 度動かしてから、入れ子の行へ乗せる
    hover(&mut h, pos2(640.0, 400.0));
    let fill = popup_item(&h, "新規塗りつぶしレイヤー");
    hover(&mut h, fill.center());
    assert_eq!(depth(&h), 1, "乗せると右に開く");
    let sub = h.state().state.popup.as_ref().unwrap().state.sub_rects()[0];
    assert!(sub.left() > fill.right(), "{sub:?} {fill:?}");
    // 入れ子の「単色」を押す
    let solid = popup_item(&h, "単色");
    hover(&mut h, pos2(fill.right() - 2.0, fill.center().y));
    hover(&mut h, solid.center());
    click(&mut h, solid.center());
    assert!(h.state().state.popup.is_none(), "選んだら閉じる");
    let id = h.state().state.selected_layer.unwrap();
    assert_eq!(
        h.state().state.doc.layer(id).unwrap().kind(),
        LayerKind::Fill
    );
    assert_eq!(h.state().state.doc.layers().len(), 2);
    h.state_mut().state.apply(Action::Undo);
    assert_eq!(
        h.state().state.doc.layers().len(),
        1,
        "1 回の取り消しで戻る"
    );
}

/// 下から a・b・c の 3 つのレイヤーがある画面（選んでいるのは一番上の c）。
fn three_layers() -> (Harness<'static, YoluApp>, [yolu_app::engine::LayerId; 3]) {
    let mut h = app(1280.0, 800.0, 64);
    let a = h.state().state.selected_layer.unwrap();
    h.state_mut().state.apply(Action::NewLayer);
    let b = h.state().state.selected_layer.unwrap();
    h.state_mut().state.apply(Action::NewLayer);
    let c = h.state().state.selected_layer.unwrap();
    h.run();
    (h, [a, b, c])
}

/// レイヤーの一覧の行の名前の所（行の真ん中）。
fn row_name(h: &Harness<'_, YoluApp>, layer: yolu_app::engine::LayerId) -> egui::Pos2 {
    let name = h.state().state.doc.layer(layer).unwrap().name().to_owned();
    common::rect_of(h, &name, |r| r.width() > 100.0 && r.left() > 1000.0).center()
}

fn order(h: &Harness<'_, YoluApp>) -> Vec<yolu_app::engine::LayerId> {
    h.state()
        .state
        .doc
        .layers()
        .iter()
        .map(|l| l.id())
        .collect()
}

fn right_click(h: &mut Harness<'_, YoluApp>, at: egui::Pos2) {
    h.event(Event::PointerMoved(at));
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
    }
    h.run();
}

#[test]
fn a_new_fill_layer_from_the_menu_bar_goes_right_above_the_row_chosen_in_the_list() {
    let (mut h, [a, b, c]) = three_layers();
    // 一番下の行を押して選んでから、メニューバーの「レイヤー」→ 新規塗りつぶしレイヤー → 単色
    let at = row_name(&h, a);
    click(&mut h, at);
    assert_eq!(h.state().state.selected_layer, Some(a));
    let at = menu_title(&h, "レイヤー").center();
    click(&mut h, at);
    hover(&mut h, pos2(640.0, 400.0));
    let fill = popup_item(&h, "新規塗りつぶしレイヤー");
    hover(&mut h, fill.center());
    let solid = popup_item(&h, "単色");
    hover(&mut h, pos2(fill.right() - 2.0, fill.center().y));
    hover(&mut h, solid.center());
    click(&mut h, solid.center());
    let made = h.state().state.selected_layer.unwrap();
    assert_eq!(
        h.state().state.doc.layer(made).unwrap().kind(),
        LayerKind::Fill
    );
    assert_eq!(order(&h), vec![a, made, b, c], "一番下の行のすぐ上");
    h.state_mut().state.apply(Action::Undo);
    assert_eq!(order(&h), vec![a, b, c]);
}

#[test]
fn a_new_fill_layer_from_the_right_click_goes_right_above_the_right_clicked_row() {
    let (mut h, [a, b, c]) = three_layers();
    assert_eq!(h.state().state.selected_layer, Some(c));
    // 選んでいない真ん中の行を右クリック: その行を選んでレイヤーのメニュー → 新規塗りつぶしレイヤー → 単色
    let at = row_name(&h, b);
    right_click(&mut h, at);
    assert_eq!(
        h.state().state.popup.as_ref().map(|p| p.kind),
        Some(PopupKind::LayerContext(b))
    );
    // 一覧の下の帯にも同じ名前のボタンがあるので、メニューの行（幅のある物）を選ぶ
    let fill = common::rect_of(&h, "新規塗りつぶしレイヤー", |r| {
        r.width() > 100.0
    });
    hover(&mut h, fill.center());
    assert_eq!(depth(&h), 1, "乗せると入れ子が開く");
    let solid = popup_item(&h, "単色");
    // 入れ子は、右に入らないので左に開く
    let sub = h.state().state.popup.as_ref().unwrap().state.sub_rects()[0];
    hover(&mut h, pos2(fill.left() + 2.0, fill.center().y));
    hover(&mut h, solid.center());
    assert!(sub.contains(solid.center()), "{sub:?} {solid:?}");
    click(&mut h, solid.center());
    let made = h.state().state.selected_layer.unwrap();
    assert_eq!(order(&h), vec![a, b, made, c], "右クリックした行のすぐ上");
}

#[test]
fn the_menu_bar_opens_the_gradient_shapes_two_levels_down_and_a_click_makes_that_shape() {
    for (lang, shape) in [(Lang::Ja, Shape::Sphere), (Lang::En, Shape::Plane)] {
        let mut h = app(1280.0, 800.0, 64);
        h.state_mut().state.lang = lang;
        h.run();
        let at = menu_title(&h, lang.pick("レイヤー", "Layer")).center();
        click(&mut h, at);
        hover(&mut h, pos2(640.0, 400.0));
        let fill = popup_item(&h, lang.pick("新規塗りつぶしレイヤー", "New Fill Layer"));
        hover(&mut h, fill.center());
        let gradient = popup_item(&h, lang.pick("グラデーションデカール", "Gradient Decal"));
        hover(&mut h, pos2(fill.right() - 2.0, fill.center().y));
        hover(&mut h, gradient.center());
        assert_eq!(depth(&h), 2, "{lang:?}: 形の一覧がもう 1 段右に開く");
        let item = popup_item(&h, yolu_app::panels::fill_props::shape_name(lang, shape));
        assert!(item.left() > gradient.right(), "{item:?} {gradient:?}");
        hover(&mut h, pos2(gradient.right() - 2.0, gradient.center().y));
        hover(&mut h, item.center());
        click(&mut h, item.center());
        assert!(h.state().state.popup.is_none(), "選んだら閉じる");
        let id = h.state().state.selected_layer.unwrap();
        let g = h
            .state()
            .state
            .doc
            .layer(id)
            .and_then(|l| l.fill_gradient(Channel::Color))
            .cloned()
            .unwrap_or_else(|| panic!("{lang:?}: {}", h.state().state.message));
        assert_eq!(g.volume.shape, shape);
        assert_eq!(h.state().state.doc.layers().len(), 2);
        h.state_mut().state.apply(Action::Undo);
        assert_eq!(
            h.state().state.doc.layers().len(),
            1,
            "1 回の取り消しで戻る"
        );
    }
}

#[test]
fn escape_closes_the_fill_submenu_first_and_then_the_menu() {
    let mut h = app(1280.0, 800.0, 64);
    let at = menu_title(&h, "レイヤー").center();
    click(&mut h, at);
    hover(&mut h, pos2(640.0, 400.0));
    let fill = popup_item(&h, "新規塗りつぶしレイヤー");
    hover(&mut h, fill.center());
    assert_eq!(depth(&h), 1);
    common::key(&h, egui::Key::Escape, egui::Modifiers::NONE);
    h.run();
    assert_eq!(depth(&h), 0, "Esc は入れ子から閉じる");
    assert!(h.state().state.popup.is_some());
    common::key(&h, egui::Key::Escape, egui::Modifiers::NONE);
    h.run();
    assert!(h.state().state.popup.is_none());
    assert_eq!(h.state().state.doc.layers().len(), 1, "何も作らない");
}

#[test]
fn the_layers_toolbar_fill_button_opens_the_kinds_and_the_effect_button_the_filters() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 800.0, 64);
        h.state_mut().state.lang = lang;
        h.run();
        // 塗りつぶし: 押しただけでは作らず、種類のポップアップ
        let fill = lang.pick("新規塗りつぶしレイヤー", "New Fill Layer");
        h.get_by_label(fill).click();
        h.run();
        assert_eq!(
            h.state().state.popup.as_ref().map(|p| p.kind),
            Some(PopupKind::M2(yolu_app::m2_menu::Popup::NewFill))
        );
        assert_eq!(h.state().state.doc.layers().len(), 1);
        // 押せば画像の入れ子の中に「ファイルから取り込む…」がある
        let kind = lang.pick("画像", "Image");
        let row = popup_item(&h, kind);
        hover(&mut h, pos2(640.0, 400.0));
        hover(&mut h, row.center());
        assert_eq!(depth(&h), 1);
        assert!(h
            .query_by_label(lang.pick("ファイルから取り込む…", "Import from File…"))
            .is_some());
        common::key(&h, egui::Key::Escape, egui::Modifiers::NONE);
        common::key(&h, egui::Key::Escape, egui::Modifiers::NONE);
        h.run();
        assert!(h.state().state.popup.is_none());
        // 調整: 今のポップアップ（種類の全部）
        h.get_by_label(lang.pick("新規調整レイヤー", "New Adjustment Layer"))
            .click();
        h.run();
        assert_eq!(
            h.state().state.popup.as_ref().map(|p| p.kind),
            Some(PopupKind::M2(yolu_app::m2_menu::Popup::NewAdjustment))
        );
        for kind in AdjustmentKind::ALL {
            assert!(
                h.query_by_label(kind.name(lang)).is_some(),
                "{lang:?} {kind:?}"
            );
        }
        common::key(&h, egui::Key::Escape, egui::Modifiers::NONE);
        h.run();
        // 効果: 「フィルターを追加」はフィルターだけ、「ジェネレーターを追加」はジェネレーターだけ（見出し・入れ子なし）
        let at = toolbar_button(&h, lang.pick("フィルターを追加", "Add Filter")).center();
        click(&mut h, at);
        assert_eq!(
            h.state().state.popup.as_ref().map(|p| p.kind),
            Some(PopupKind::M2(yolu_app::m2_menu::Popup::AddFilter(
                FilterTarget::Content
            )))
        );
        assert!(h
            .query_by_label(lang.pick("ぼかし（ガウス）", "Gaussian Blur"))
            .is_some());
        assert!(h
            .query_by_label(lang.pick("エッジの摩耗", "Edge Wear"))
            .is_none());
        assert!(h.query_by_label("Generator").is_none());
        common::key(&h, egui::Key::Escape, egui::Modifiers::NONE);
        h.run();
        let at = toolbar_button(&h, lang.pick("ジェネレーターを追加", "Add Generator")).center();
        click(&mut h, at);
        assert_eq!(
            h.state().state.popup.as_ref().map(|p| p.kind),
            Some(PopupKind::M2(yolu_app::m2_menu::Popup::AddGenerator(
                FilterTarget::Content
            )))
        );
        assert!(h
            .query_by_label(lang.pick("エッジの摩耗", "Edge Wear"))
            .is_some());
        assert!(h
            .query_by_label(lang.pick("ぼかし（ガウス）", "Gaussian Blur"))
            .is_none());
    }
}

/// レイヤーのパネルの下の帯のボタン（小さい四角。プロパティの欄の同じ名前のボタンと取り違えない）。
fn toolbar_button(h: &Harness<'_, YoluApp>, label: &str) -> Rect {
    common::rect_of(h, label, |r| r.top() > 200.0 && r.width() < 40.0)
}

#[test]
fn the_effect_buttons_need_a_selected_layer() {
    let mut h = app(1280.0, 800.0, 64);
    h.state_mut().state.selected_layer = None;
    h.run();
    for label in ["フィルターを追加", "ジェネレーターを追加"] {
        let at = toolbar_button(&h, label).center();
        click(&mut h, at);
        assert!(
            h.state().state.popup.is_none(),
            "レイヤーが無ければ効果は足せない"
        );
    }
}

#[test]
fn the_layers_toolbar_buttons_all_fit_in_the_panel_at_the_minimum_window_in_both_languages() {
    for lang in Lang::ALL {
        let mut h = app(960.0, 640.0, 64);
        h.state_mut().state.lang = lang;
        h.run();
        let labels = [
            lang.pick("新規レイヤー", "New Layer"),
            lang.pick("新規塗りつぶしレイヤー", "New Fill Layer"),
            lang.pick("新規調整レイヤー", "New Adjustment Layer"),
            lang.pick("フィルターを追加", "Add Filter"),
            lang.pick("ジェネレーターを追加", "Add Generator"),
            lang.pick("レイヤーをグループ化", "Group Layers"),
            lang.pick("レイヤーマスクを追加", "Add Layer Mask"),
            // 押せないときは名前に理由が続く
            lang.pick("下のレイヤーでクリッピング", "Clip to the Layer Below"),
            lang.pick("レイヤーを上へ", "Move Layer Up"),
            lang.pick("レイヤーを下へ", "Move Layer Down"),
            lang.pick("レイヤーを削除", "Delete Layer"),
        ];
        let rects: Vec<Rect> = labels
            .iter()
            .map(|l| {
                h.query_all_by_label_contains(l)
                    .filter(|n| n.rect().top() > 200.0)
                    .map(|n| n.rect())
                    .next()
                    .unwrap_or_else(|| panic!("{lang:?} {l}"))
            })
            .collect();
        for (i, a) in rects.iter().enumerate() {
            assert!(
                a.right() <= 960.0 && a.left() >= 0.0,
                "{lang:?} {}: {a:?}",
                labels[i]
            );
            for (j, b) in rects.iter().enumerate().skip(i + 1) {
                assert!(
                    !a.intersects(*b),
                    "{lang:?} {} と {} が重なる: {a:?} {b:?}",
                    labels[i],
                    labels[j]
                );
            }
        }
        // 全部が同じ 1 行（帯）の中
        let top = rects.iter().map(|r| r.top()).fold(f32::MAX, f32::min);
        let bottom = rects.iter().map(|r| r.bottom()).fold(f32::MIN, f32::max);
        assert!(bottom - top < 32.0, "{lang:?}: 帯は 1 行 {top}..{bottom}");
    }
}

// ───────── メインとサブの色（ツールの帯の一番下） ─────────

fn tool_label(lang: Lang, tool: Tool) -> String {
    // キーの無いツール（ゆがみ）は名前だけ
    match (lang, tool.key()) {
        (_, "") => tool.name_in(lang).to_string(),
        (Lang::Ja, key) => format!("{}（{key}）", tool.name_in(lang)),
        (Lang::En, key) => format!("{} ({key})", tool.name_in(lang)),
    }
}

fn swatch_labels(lang: Lang) -> [String; 4] {
    // 色の四角のラベルは、ツールチップの文字そのもの（16 進が続く）
    [
        lang.pick(
            "メインの色（描画色。ブラシで塗る色）",
            "Foreground color (the color the brush paints)",
        )
        .to_owned(),
        lang.pick(
            "サブの色（背景色）。押すとメインの色と入れ替えます",
            "Background color. Click to swap with the foreground color.",
        )
        .to_owned(),
        lang.pick(
            "メインとサブの色を入れ替え（X）",
            "Swap foreground and background colors (X)",
        )
        .to_owned(),
        lang.pick("初期設定の色（D）", "Default colors (D)")
            .to_owned(),
    ]
}

fn swatch_rects(h: &Harness<'_, YoluApp>, lang: Lang) -> [Rect; 4] {
    let labels = swatch_labels(lang);
    let find = |prefix: &str| {
        let found: Vec<Rect> = h
            .get_all_by_label_contains(prefix)
            .map(|n| n.rect())
            .collect();
        assert_eq!(found.len(), 1, "{prefix}: ツールの帯に 1 つだけ: {found:?}");
        found[0]
    };
    [
        find(&labels[0]),
        find(&labels[1]),
        find(&labels[2]),
        find(&labels[3]),
    ]
}

#[test]
fn the_two_colors_sit_at_the_bottom_of_the_tool_strip_and_never_overlap_the_tools() {
    use yolu_app::ui::theme::TOOL_STRIP_WIDTH;
    // 1000 はツールが帯を埋めない高さ（ツールが増えて、800 では帯がほぼ埋まる）
    for (width, height) in [(1280.0, 1000.0), (1280.0, 800.0), (960.0, 640.0)] {
        for lang in Lang::ALL {
            let mut h = app(width, height, 64);
            h.state_mut().state.lang = lang;
            h.run();
            let rects = swatch_rects(&h, lang);
            // ツールの帯の下の端にある（帯は左端の幅 44、状態の帯の上まで）
            let strip_bottom = height - 22.0;
            for r in rects {
                assert!(
                    r.left() >= 0.0 && r.right() <= TOOL_STRIP_WIDTH,
                    "{lang:?}: 帯の幅に収まる {r:?}"
                );
                assert!(r.bottom() <= strip_bottom, "{lang:?}: {r:?}");
            }
            let all = rects.iter().skip(1).fold(rects[0], |a, r| a.union(*r));
            assert!(
                all.bottom() > strip_bottom - 16.0 && all.top() > strip_bottom - 60.0,
                "帯の下の端に付く: {all:?}"
            );
            // 最後のツールのすぐ下ではなく、ウィンドウが高ければ間が空く
            let last_tool = Tool::ALL[Tool::ALL.len() - 1];
            let last = h.get_by_label(&tool_label(lang, last_tool)).rect();
            let top_of_colors = rects.iter().map(|r| r.top()).fold(f32::MAX, f32::min);
            assert!(
                last.bottom() <= top_of_colors,
                "{lang:?}: 最後のツールと色が重ならない {last:?} {rects:?}"
            );
            if height > 900.0 {
                assert!(
                    top_of_colors - last.bottom() > 20.0,
                    "高いウィンドウでは色は帯の下の端に付く（ツールの下に寄らない）: 高さ {height} 最後のツール {last:?} 色の上 {top_of_colors}"
                );
            }
            // どのツールのボタンとも重ならない
            for tool in Tool::ALL {
                let label = tool_label(lang, tool);
                let r = h.get_by_label(&label).rect();
                for c in rects {
                    assert!(!r.intersects(c), "{lang:?} {label}: {r:?} と色 {c:?}");
                }
            }
            // 2 枚の色の部品どうしも、メインとサブの重なり（Photoshop の配置）以外は重ならない
            let overlap = |a: Rect, b: Rect| {
                let i = a.intersect(b);
                i.width() > 0.5 && i.height() > 0.5
            };
            assert!(!overlap(rects[2], rects[0]) && !overlap(rects[2], rects[1]));
            assert!(!overlap(rects[3], rects[0]) && !overlap(rects[3], rects[1]));
            assert!(!overlap(rects[2], rects[3]));
            // メインとサブは重なる（描画色が手前）
            assert!(overlap(rects[0], rects[1]));
        }
    }
}

#[test]
fn the_color_squares_swap_and_reset_and_the_color_panel_no_longer_holds_them() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 800.0, 64);
        h.state_mut().state.lang = lang;
        h.state_mut().state.color.set_main([0.2, 0.4, 0.8, 1.0]);
        h.state_mut().state.color.sub = [1.0, 1.0, 0.0, 1.0];
        h.run();
        let rects = swatch_rects(&h, lang);
        let (main, sub) = (h.state().state.color.main, h.state().state.color.sub);
        // 背景色の四角を押すと入れ替わる
        click(&mut h, rects[1].center());
        assert_eq!(
            (h.state().state.color.main, h.state().state.color.sub),
            (sub, main),
            "{lang:?}"
        );
        // 入れ替えのボタン
        click(&mut h, rects[2].center());
        assert_eq!(
            (h.state().state.color.main, h.state().state.color.sub),
            (main, sub),
            "{lang:?}"
        );
        // 初期設定（黒・白）
        click(&mut h, rects[3].center());
        assert_eq!(h.state().state.color.main, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(h.state().state.color.sub, [1.0, 1.0, 1.0, 1.0]);
        // 色のパネルの中には 2 枚の色の部品が無い（ツールの帯に 1 つずつだけ。上の swatch_rects が 1 つだけを確かめている）
        for r in swatch_rects(&h, lang) {
            assert!(r.right() <= yolu_app::ui::theme::TOOL_STRIP_WIDTH);
        }
    }
}

#[test]
fn the_keys_x_and_d_still_swap_and_reset_the_colors() {
    let mut h = app(1280.0, 800.0, 64);
    h.state_mut().state.color.set_main([0.2, 0.4, 0.8, 1.0]);
    h.run();
    let main = h.state().state.color.main;
    common::key(&h, egui::Key::X, egui::Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.color.sub, main);
    common::key(&h, egui::Key::D, egui::Modifiers::NONE);
    h.run();
    assert_eq!(h.state().state.color.main, [0.0, 0.0, 0.0, 1.0]);
}

/// 色のパネルだけを描くウィンドウ（`color::show`）。
fn color_panel(width: f32, height: f32, wheel: bool, lang: Lang) -> Harness<'static, AppState> {
    let mut state = AppState::new(64, 64);
    state.lang = lang;
    state.color.wheel = wheel;
    let mut textures = yolu_app::panels::color::ColorTextures::default();
    let mut ready = false;
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(width, height))
        .renderer(common::shared_gpu::renderer())
        .build_ui_state(
            move |ui, state| {
                if !ready {
                    YoluApp::setup(ui.ctx());
                    ready = true;
                    ui.ctx().request_repaint();
                    return;
                }
                yolu_app::panels::color::show(ui, state, &mut textures)
            },
            state,
        );
    h.run();
    h
}

#[test]
fn the_color_panel_gives_the_freed_height_to_the_color_surface_and_holds_no_swatches() {
    for lang in Lang::ALL {
        for wheel in [false, true] {
            // 同じ幅で高さだけ増やすと、色の面（四角・円）が高くなる（2 枚の色の分の高さを取らない）
            let surface = |height: f32| {
                let h = color_panel(300.0, height, wheel, lang);
                let label = if wheel {
                    lang.pick("色相の円", "Hue wheel")
                } else {
                    lang.pick("彩度と明度", "Saturation and value")
                };
                h.get_by_label(label).rect().height()
            };
            let (low, high) = (surface(240.0), surface(300.0));
            assert!(
                high >= low + 55.0,
                "{lang:?} wheel={wheel}: 高さ 240 → 300 で面が {low} → {high}（60 近く増える）"
            );
            // 2 枚の色の部品はパネルの中に無い
            let h = color_panel(300.0, 300.0, wheel, lang);
            for label in swatch_labels(lang) {
                assert!(
                    h.query_all_by_label_contains(&label).next().is_none(),
                    "{lang:?}: {label}"
                );
            }
            // 16 進の欄・アルファ・使った色は、パネルの下まで収まる
            let hex = h
                .get_all_by_value("#000000")
                .next()
                .expect("16 進の欄")
                .rect();
            assert!(hex.bottom() <= 300.0, "{hex:?}");
        }
    }
}

#[test]
fn the_color_panel_wraps_hex_and_alpha_to_two_lines_when_narrow() {
    for lang in Lang::ALL {
        // 幅が足りなければ 16 進とアルファは 2 行（どちらも欄の幅）、あれば 1 行に並ぶ
        let narrow = color_panel(150.0, 360.0, false, lang);
        let hex = narrow.get_all_by_value("#000000").next().unwrap().rect();
        let alpha = narrow.get_by_label("A").rect();
        assert!(
            alpha.top() >= hex.bottom(),
            "{lang:?}: 狭い欄は 2 行 {hex:?} {alpha:?}"
        );
        let wide = color_panel(300.0, 360.0, false, lang);
        let hex = wide.get_all_by_value("#000000").next().unwrap().rect();
        let alpha = wide.get_by_label("A").rect();
        assert!(
            (alpha.center().y - hex.center().y).abs() < 2.0,
            "{lang:?}: 広い欄は 1 行 {hex:?} {alpha:?}"
        );
    }
}

#[test]
fn the_tool_strip_bottom_snapshot_at_the_minimum_window() {
    let mut h = app(960.0, 640.0, 64);
    h.run();
    let image = h.render().expect("描画");
    // 左端のツールの帯（幅 44）の全体
    let cropped = image::imageops::crop_imm(&image, 0, 24 + 36, 48, 640 - 24 - 36 - 22).to_image();
    egui_kittest::image_snapshot(&cropped, "menus_tool_strip_min");
}

/// 開いているメニュー全体（親と開いている入れ子）を切り抜いて撮る。
fn snapshot_open_menu(h: &mut Harness<'_, YoluApp>, name: &str) {
    let rect = h
        .state()
        .state
        .popup
        .as_ref()
        .expect("開いている")
        .state
        .rect
        .expand(2.0);
    let image = h.render().expect("描画");
    let cropped = image::imageops::crop_imm(
        &image,
        rect.left().max(0.0).floor() as u32,
        rect.top().max(0.0).floor() as u32,
        rect.width().ceil() as u32,
        rect.height().ceil() as u32,
    )
    .to_image();
    egui_kittest::image_snapshot(&cropped, name);
}

#[test]
fn snapshot_the_layer_menu_with_the_image_submenu_open_in_both_languages() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 800.0, 64);
        h.state_mut().state.lang = lang;
        let (_, _) = shelf_image(&mut h.state_mut().state, "四色");
        h.run();
        let at = menu_title(&h, lang.pick("レイヤー", "Layer")).center();
        click(&mut h, at);
        hover(&mut h, pos2(640.0, 400.0));
        let fill = popup_item(&h, lang.pick("新規塗りつぶしレイヤー", "New Fill Layer"));
        hover(&mut h, fill.center());
        let image = popup_item(&h, lang.pick("画像", "Image"));
        hover(&mut h, pos2(fill.right() - 2.0, fill.center().y));
        hover(&mut h, image.center());
        assert_eq!(depth(&h), 2, "{lang:?}");
        snapshot_open_menu(
            &mut h,
            &format!("menus_layer_fill_image_{}", lang.pick("ja", "en")),
        );
    }
}

#[test]
fn snapshot_the_layer_menu_with_the_gradient_shapes_open_in_both_languages() {
    for lang in Lang::ALL {
        let mut h = app(1280.0, 800.0, 64);
        h.state_mut().state.lang = lang;
        h.run();
        let at = menu_title(&h, lang.pick("レイヤー", "Layer")).center();
        click(&mut h, at);
        hover(&mut h, pos2(640.0, 400.0));
        let fill = popup_item(&h, lang.pick("新規塗りつぶしレイヤー", "New Fill Layer"));
        hover(&mut h, fill.center());
        let gradient = popup_item(&h, lang.pick("グラデーションデカール", "Gradient Decal"));
        hover(&mut h, pos2(fill.right() - 2.0, fill.center().y));
        hover(&mut h, gradient.center());
        assert_eq!(depth(&h), 2, "{lang:?}");
        snapshot_open_menu(
            &mut h,
            &format!("menus_layer_fill_gradient_{}", lang.pick("ja", "en")),
        );
    }
}

#[test]
fn snapshot_the_filter_menu_and_the_layer_menu_generators_in_both_languages() {
    let mut results = egui_kittest::SnapshotResults::new();
    for lang in Lang::ALL {
        // メニューバーの「フィルター」: フィルターとアンカーだけ
        let mut h = app(1280.0, 800.0, 64);
        h.state_mut().state.lang = lang;
        h.run();
        let at = menu_title(&h, lang.pick("フィルター", "Filter")).center();
        click(&mut h, at);
        hover(&mut h, pos2(640.0, 400.0));
        assert_eq!(depth(&h), 0, "{lang:?}");
        snapshot_open_menu(&mut h, &format!("menus_filter_{}", lang.pick("ja", "en")));
        // メニューバーの「レイヤー」の「ジェネレーターを追加 ▸」
        let mut h = app(1280.0, 800.0, 64);
        h.state_mut().state.lang = lang;
        h.run();
        let at = menu_title(&h, lang.pick("レイヤー", "Layer")).center();
        click(&mut h, at);
        hover(&mut h, pos2(640.0, 400.0));
        let generators = popup_item(&h, lang.pick("ジェネレーターを追加", "Add Generator"));
        hover(&mut h, generators.center());
        assert_eq!(depth(&h), 1, "{lang:?}");
        h.snapshot(format!("menus_layer_generators_{}", lang.pick("ja", "en")));
        results.extend_harness(&mut h);
    }
}

#[test]
fn snapshot_the_filter_menu_on_a_normal_map_shows_the_reason_as_a_tooltip() {
    let mut h = app(1280.0, 800.0, 64);
    h.state_mut().state.m2.paint_channel = Channel::Normal;
    h.run();
    let at = menu_title(&h, "フィルター").center();
    click(&mut h, at);
    hover(&mut h, pos2(640.0, 400.0));
    let sharpen = popup_item(&h, "シャープ（アンシャープマスク）");
    h.event(Event::PointerMoved(sharpen.center()));
    for _ in 0..60 {
        h.step();
    }
    // ラベルは名前だけ。理由はツールチップ（アクセシビリティの木に出る）
    let entries = yolu_app::fx::menu::add_filter_entries(&h.state().state, FilterTarget::Content);
    let tip = leaves(&entries)
        .into_iter()
        .find_map(|e| match e {
            Entry::Item {
                label,
                enabled: false,
                tooltip: Some(t),
                ..
            } if label == "シャープ（アンシャープマスク）" => Some(t.clone()),
            _ => None,
        })
        .expect("法線ではシャープは押せず、理由を持つ");
    assert!(
        h.query_by_label(&tip).is_some(),
        "押せない項目に乗せると理由が出る: {tip}"
    );
    snapshot_open_menu(&mut h, "menus_filter_refusal_tooltip");
}

#[test]
fn snapshot_the_layers_toolbar_at_the_minimum_window() {
    let mut h = app(960.0, 640.0, 64);
    h.run();
    let new_layer = h
        .query_all_by_label("新規レイヤー")
        .map(|n| n.rect())
        .find(|r| r.top() > 200.0)
        .expect("帯の新規レイヤー");
    let delete = h
        .query_all_by_label("レイヤーを削除")
        .map(|n| n.rect())
        .find(|r| r.top() > 200.0)
        .expect("帯の削除");
    let bar = new_layer.union(delete).expand2(vec2(8.0, 4.0));
    let image = h.render().expect("描画");
    let cropped = image::imageops::crop_imm(
        &image,
        bar.left().max(0.0).floor() as u32,
        bar.top().max(0.0).floor() as u32,
        bar.width().ceil() as u32,
        bar.height().ceil() as u32,
    )
    .to_image();
    egui_kittest::image_snapshot(&cropped, "menus_layers_toolbar_min");
}
