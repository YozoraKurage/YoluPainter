//! オートアクション（操作の記録と再生）: 写せる操作の表の全部について、記録 → 別の文書（同じ形）で再生 → 画面で同じ操作をした文書と
//! 同じになる。写せない操作は記録しない（印を出す）。再生は取り消し 1 回で全部戻り、途中で断ったらそこまで戻して何番目かを知らせる。
use std::collections::HashMap;

use serde_json::{json, Value};
use yolu_app::automation::store::ActionStore;
use yolu_app::automation::AutomationOp;
use yolu_app::fx::names::FilterKind;
use yolu_app::fx::FxOp;
use yolu_app::lang::Lang;
use yolu_app::m2::{AdjustmentKind, Edit};
use yolu_app::notice::Kind;
use yolu_app::ops_host::AppHost;
use yolu_app::state::{Action, AppState};
use yolu_core::effects::{EffectSettings, FilterTarget};
use yolu_core::generator::Kind as GenKind;
use yolu_core::{AdjustmentSettings, BlendMode, Channel, Document, LayerId, LayerLocks, Rgba8};
use yolu_ops::doc_ops::{layer_info, set_info, SetFacts};
use yolu_ops::{parse_command, Command};

use crate::common::tmp::test_dir;

// ───────── 台 ─────────

/// 同じ形の文書（ID は毎回違う）: 下から「レイヤー 1」「レイヤー 2」・「グループ 1」（中に「レイヤー 4」）・塗りつぶし・レベル補正。
/// 選んでいるのは「レイヤー 2」。
fn fixture() -> AppState {
    let mut s = AppState::new_in(32, 32, Lang::Ja);
    s.apply(Action::NewLayer);
    let second = s.selected_layer.unwrap();
    s.apply(Action::M2(Edit::NewGroup));
    let group = s.selected_layer.unwrap();
    s.apply(Action::NewLayer);
    let inner = s.selected_layer.unwrap();
    s.apply(Action::M2(Edit::Move {
        id: inner,
        parent: Some(group),
        position: 0,
    }));
    s.selected_layer = Some(group);
    s.apply(Action::M2(Edit::NewFill));
    s.apply(Action::M2(Edit::NewAdjustment(AdjustmentKind::Levels)));
    s.selected_layer = Some(second);
    s.doc.clear_history().unwrap();
    s.modified = false;
    s
}

fn layer(s: &AppState, name: &str) -> LayerId {
    let found: Vec<LayerId> = s
        .doc
        .layers()
        .iter()
        .filter(|l| l.name() == name)
        .map(|l| l.id())
        .collect();
    assert_eq!(found.len(), 1, "{name}");
    found[0]
}

/// 文書の比べ: 全部のレイヤーの全部（属性・チャンネル・マスク・効果・調整）と並び。ID は文書ごとに違うので、並びの番号に替える。
fn fingerprint(doc: &Document) -> Value {
    let facts = SetFacts {
        id: "set",
        name: "set",
        unsaved: false,
    };
    let mut v = json!({
        "set": set_info(facts, doc),
        "layers": doc.layers().iter().map(|l| layer_info(doc, l)).collect::<Vec<_>>(),
    });
    let mut ids: HashMap<String, String> = HashMap::new();
    let mut effects = 0;
    for (i, l) in doc.layers().iter().enumerate() {
        ids.insert(l.id().to_string(), format!("L{i}"));
        for f in l
            .filters()
            .iter()
            .chain(l.mask().into_iter().flat_map(|m| m.filters().iter()))
        {
            ids.insert(f.id().to_string(), format!("E{effects}"));
            effects += 1;
        }
    }
    replace(&mut v, &ids);
    v
}

fn replace(v: &mut Value, ids: &HashMap<String, String>) {
    match v {
        Value::String(s) => {
            if let Some(new) = ids.get(s.as_str()) {
                *s = new.clone();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|x| replace(x, ids)),
        Value::Object(map) => map.values_mut().for_each(|x| replace(x, ids)),
        _ => {}
    }
}

fn start(s: &mut AppState) {
    s.apply(Action::Automation(AutomationOp::StartRecording));
}

fn recorded(s: &AppState) -> Vec<Command> {
    s.automation.recorder.as_ref().expect("記録中").commands()
}

fn skipped(s: &AppState) -> bool {
    s.automation.skipped()
}

fn play(s: &mut AppState, commands: &[Command]) -> Result<(), yolu_ops::OpError> {
    let mut host = AppHost::new(s);
    yolu_ops::action::run(&mut host, commands).map(|_| ())
}

/// 1 つの写し方の試験: 記録した命令を、同じ形の別の文書で再生すると、画面で同じ操作をした文書と同じ。再生は取り消し 1 回で戻る。
fn check(name: &str, setup: impl Fn(&mut AppState), run: impl Fn(&mut AppState)) -> Vec<Command> {
    let mut rec = fixture();
    setup(&mut rec);
    rec.doc.end_coalescing();
    start(&mut rec);
    run(&mut rec);
    rec.doc.end_coalescing();
    yolu_app::automation::record::sync(&mut rec);
    let commands = recorded(&rec);
    assert!(!commands.is_empty(), "{name}: 記録されていない");
    assert!(
        !skipped(&rec),
        "{name}: 記録しなかった印が出た {commands:?}"
    );
    let mut other = fixture();
    setup(&mut other);
    let before = fingerprint(&other.doc);
    assert_ne!(
        fingerprint(&rec.doc),
        before,
        "{name}: 操作が文書を変えていない"
    );
    let undo_before = other.doc.undo_count();
    play(&mut other, &commands).unwrap_or_else(|e| {
        panic!(
            "{name}: 再生が断られた {} {:?} {commands:?}",
            e.message.ja, e.data
        )
    });
    assert_eq!(
        fingerprint(&other.doc),
        fingerprint(&rec.doc),
        "{name}: 再生した文書が、画面で同じ操作をした文書と違う {commands:?}"
    );
    assert_eq!(
        other.doc.undo_count(),
        undo_before + 1,
        "{name}: 取り消しの 1 段"
    );
    other.apply(Action::Undo);
    assert_eq!(
        fingerprint(&other.doc),
        before,
        "{name}: 取り消し 1 回で全部戻る"
    );
    commands
}

fn check_skipped(name: &str, setup: impl Fn(&mut AppState), run: impl Fn(&mut AppState)) {
    let mut rec = fixture();
    setup(&mut rec);
    start(&mut rec);
    let revision = rec.doc.revision();
    run(&mut rec);
    assert_ne!(rec.doc.revision(), revision, "{name}: 文書を変えていない");
    yolu_app::automation::record::sync(&mut rec);
    assert!(
        recorded(&rec).is_empty(),
        "{name}: 記録した {:?}",
        recorded(&rec)
    );
    assert!(skipped(&rec), "{name}: 記録しなかった印が無い");
}

fn none(_: &mut AppState) {}

/// 値の欄だけで足せる種類か（効果の目録の `addable`）。
fn addable(kind: &str) -> bool {
    yolu_core::effects::catalog::kinds()
        .iter()
        .any(|k| k.id == kind && k.addable)
}

// ───────── 写せる操作 ─────────

#[test]
fn every_mappable_layer_operation_replays_to_the_same_document() {
    check("新しいレイヤー", none, |s| s.apply(Action::NewLayer));
    check("レイヤーを削除", none, |s| {
        s.apply(Action::DeleteLayer)
    });
    check("上へ", none, |s| s.apply(Action::LayerUp));
    check("下へ", none, |s| s.apply(Action::LayerDown));
    check("表示の切り替え", none, |s| {
        let id = s.selected_layer.unwrap();
        s.apply(Action::ToggleVisible(id))
    });
    check("合成モード", none, |s| {
        let id = s.selected_layer.unwrap();
        s.apply(Action::SetBlend(id, BlendMode::Multiply))
    });
    check("グループ", none, |s| {
        s.apply(Action::M2(Edit::NewGroup))
    });
    check("塗りつぶしレイヤー", none, |s| {
        s.color.set_main([0.2, 0.4, 0.8, 1.0]);
        s.apply(Action::M2(Edit::NewFill))
    });
    let mut adjustments = 0;
    for kind in AdjustmentKind::ALL {
        if addable(kind.settings().kind_id()) {
            check(&format!("調整レイヤー {kind:?}"), none, |s| {
                s.apply(Action::M2(Edit::NewAdjustment(kind)))
            });
            adjustments += 1;
        } else {
            check_skipped(
                &format!("値だけで表せない調整 {kind:?}"),
                none,
                |s| s.apply(Action::M2(Edit::NewAdjustment(kind))),
            );
        }
    }
    assert!(adjustments >= 5, "{adjustments}");
    check("グループの中へ", none, |s| {
        let id = s.selected_layer.unwrap();
        let group = layer(s, "グループ 1");
        s.apply(Action::M2(Edit::Move {
            id,
            parent: Some(group),
            position: 1,
        }))
    });
    check("グループの外へ", none, |s| {
        let id = layer(s, "レイヤー 4");
        s.apply(Action::M2(Edit::Move {
            id,
            parent: None,
            position: 0,
        }))
    });
    check("クリッピング", none, |s| {
        let id = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::Clipping(id, true)))
    });
    check("合成モード（M2）", none, |s| {
        let id = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::BlendMode {
            id,
            channel: None,
            mode: BlendMode::Screen,
        }))
    });
    check("チャンネルの有効", none, |s| {
        let id = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::ChannelEnabled {
            id,
            channel: Channel::Roughness,
            enabled: true,
        }))
    });
    check("表示（選んだレイヤー）", none, |s| {
        s.apply(Action::M2(Edit::ToggleSelectedVisible))
    });
    check("ロック", none, |s| {
        let id = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::Lock {
            ids: vec![id],
            flag: LayerLocks::PIXELS,
            on: true,
        }))
    });
    check(
        "ロックを全部外す",
        |s| {
            let id = s.selected_layer.unwrap();
            for flag in [LayerLocks::TRANSPARENCY, LayerLocks::POSITION] {
                s.apply(Action::M2(Edit::Lock {
                    ids: vec![id],
                    flag,
                    on: true,
                }));
            }
        },
        |s| {
            let id = s.selected_layer.unwrap();
            s.apply(Action::M2(Edit::Lock {
                ids: vec![id],
                flag: LayerLocks::from_bits(15).unwrap(),
                on: false,
            }))
        },
    );
    // 名前の変更（レイヤーの欄は文書へ直に当てる）
    check("名前の変更", none, |s| {
        let id = s.selected_layer.unwrap();
        let pending = yolu_app::automation::record::before_rename(s, id, "Renamed");
        s.doc.set_layer_name(id, "Renamed").unwrap();
        yolu_app::automation::record::after(s, pending);
    });
}

#[test]
fn values_dragged_with_a_slider_become_one_command() {
    let commands = check("不透明度のドラッグ", none, |s| {
        let id = s.selected_layer.unwrap();
        for value in [0.8, 0.6, 0.35] {
            s.apply(Action::M2(Edit::Opacity {
                id,
                channel: None,
                value,
            }));
        }
        s.m2_end_drag();
    });
    assert_eq!(commands.len(), 1, "{commands:?}");
    let Command::LayerSet(set) = &commands[0] else {
        panic!("{commands:?}")
    };
    assert_eq!((set.layer.as_str(), set.opacity), ("$selected", Some(0.35)));
    // 2 回のドラッグは 2 つ
    let commands = check("2 回のドラッグ", none, |s| {
        let id = s.selected_layer.unwrap();
        for value in [0.8, 0.5] {
            s.apply(Action::M2(Edit::Opacity {
                id,
                channel: None,
                value,
            }));
            s.m2_end_drag();
        }
    });
    assert_eq!(commands.len(), 2);
    // Esc で止めたドラッグは記録から外す
    let mut s = fixture();
    start(&mut s);
    s.apply(Action::NewLayer);
    let id = s.selected_layer.unwrap();
    for value in [0.7, 0.4] {
        s.apply(Action::M2(Edit::Opacity {
            id,
            channel: None,
            value,
        }));
    }
    s.m2_cancel_drag();
    yolu_app::automation::record::sync(&mut s);
    assert_eq!(recorded(&s).len(), 1, "止めたドラッグは残さない");
    assert!(!skipped(&s));
}

#[test]
fn every_mappable_mask_fill_and_adjustment_operation_replays_to_the_same_document() {
    let add_mask = |s: &mut AppState| {
        let id = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::AddMask(id)));
        s.selected_layer = Some(id);
    };
    check("マスクを追加", none, add_mask);
    check("マスクを削除", add_mask, |s| {
        let id = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::RemoveMask(id)))
    });
    check("マスクの有効", add_mask, |s| {
        let id = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::MaskEnabled(id, false)))
    });
    check("マスクの反転", add_mask, |s| {
        let id = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::MaskInverted(id, true)))
    });
    check("マスクの濃度", add_mask, |s| {
        let id = s.selected_layer.unwrap();
        for v in [0.9, 0.5] {
            s.apply(Action::M2(Edit::MaskDensity(id, v)));
        }
    });
    check("塗りつぶしの値", none, |s| {
        let id = layer(s, "塗りつぶし 1");
        s.apply(Action::M2(Edit::FillValue {
            id,
            channel: Channel::Roughness,
            value: Some(Rgba8::new(90, 90, 90, 255)),
        }))
    });
    check("塗りつぶしの値を外す", none, |s| {
        let id = layer(s, "塗りつぶし 1");
        s.apply(Action::M2(Edit::FillValue {
            id,
            channel: Channel::Color,
            value: None,
        }))
    });
    check("調整の値", none, |s| {
        let id = layer(s, AdjustmentKind::Levels.name(Lang::Ja));
        let settings = AdjustmentSettings::levels(0.1, 0.9, 1.4, 0.0, 1.0).unwrap();
        s.apply(Action::M2(Edit::Adjust { id, settings }))
    });
    check("調整の種類を替える", none, |s| {
        let id = layer(s, AdjustmentKind::Levels.name(Lang::Ja));
        s.apply(Action::M2(Edit::Adjust {
            id,
            settings: AdjustmentSettings::invert(),
        }))
    });
}

/// 選んでいるレイヤーの効果（最後の物）。
fn last_effect(s: &AppState, target: FilterTarget) -> (LayerId, yolu_core::effects::FilterId) {
    let id = s.selected_layer.unwrap();
    let l = s.doc.layer(id).unwrap();
    let list = match target {
        FilterTarget::Content => l.filters(),
        FilterTarget::Mask => l.mask().unwrap().filters(),
    };
    (id, list.last().unwrap().id())
}

#[test]
fn every_mappable_effect_operation_replays_to_the_same_document() {
    check("フィルターを追加", none, |s| {
        s.apply(Action::Fx(FxOp::AddFilter {
            target: FilterTarget::Content,
            kind: FilterKind::Blur,
        }))
    });
    let add_mask = |s: &mut AppState| {
        let id = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::AddMask(id)));
        s.selected_layer = Some(id);
    };
    let mut filters = 0;
    for kind in FilterKind::ALL {
        if addable(kind.settings().kind_id()) {
            // スカラーとマスクだけのフィルター（輪郭の検出・太らせるなど）は色のチャンネルでは断られるので、マスクへ足す
            let on_color = {
                let mut probe = fixture();
                let before = probe.doc.revision();
                probe.apply(Action::Fx(FxOp::AddFilter {
                    target: FilterTarget::Content,
                    kind,
                }));
                probe.doc.revision() != before
            };
            if on_color {
                check(&format!("フィルター {kind:?}"), none, |s| {
                    s.apply(Action::Fx(FxOp::AddFilter {
                        target: FilterTarget::Content,
                        kind,
                    }))
                });
            } else {
                check(
                    &format!("マスクのフィルター {kind:?}"),
                    add_mask,
                    |s| {
                        s.apply(Action::Fx(FxOp::AddFilter {
                            target: FilterTarget::Mask,
                            kind,
                        }))
                    },
                );
            }
            filters += 1;
        } else {
            check_skipped(
                &format!("値だけで表せないフィルター {kind:?}"),
                none,
                |s| {
                    s.apply(Action::Fx(FxOp::AddFilter {
                        target: FilterTarget::Content,
                        kind,
                    }))
                },
            );
        }
    }
    assert!(filters >= 8, "{filters}");
    check("マスクの効果", add_mask, |s| {
        s.apply(Action::Fx(FxOp::AddFilter {
            target: FilterTarget::Mask,
            kind: FilterKind::Invert,
        }))
    });
    let mut generators = 0;
    for kind in [
        GenKind::EdgeWear,
        GenKind::Dirt,
        GenKind::PositionGradient,
        GenKind::Thickness,
        GenKind::Direction,
        GenKind::Noise,
        GenKind::Grunge,
    ] {
        let settings = EffectSettings::generator(yolu_core::generator::Settings::new(kind));
        assert!(addable(settings.kind_id()), "{kind:?}");
        check(&format!("Generator {kind:?}"), none, |s| {
            s.apply(Action::Fx(FxOp::AddGenerator {
                target: FilterTarget::Content,
                kind,
            }))
        });
        generators += 1;
    }
    assert_eq!(generators, 7);
    // 記録の中で作った効果の変更
    let commands = check(
        "効果の値・強さ・有効・チャンネル・位置・削除",
        none,
        |s| {
            for kind in [FilterKind::Blur, FilterKind::Invert] {
                s.apply(Action::Fx(FxOp::AddFilter {
                    target: FilterTarget::Content,
                    kind,
                }));
            }
            let (layer, id) = last_effect(s, FilterTarget::Content);
            let first = s.doc.layer(layer).unwrap().filters()[0].id();
            for strength in [0.9, 0.5] {
                s.apply(Action::Fx(FxOp::SetStrength {
                    layer,
                    id,
                    strength,
                    coalesce: true,
                }));
            }
            s.m2_end_drag();
            s.apply(Action::Fx(FxOp::SetEnabled {
                layer,
                id,
                enabled: false,
            }));
            s.apply(Action::Fx(FxOp::SetChannels {
                layer,
                id,
                channels: vec![Channel::Color, Channel::Roughness],
            }));
            s.apply(Action::Fx(FxOp::Move {
                layer,
                id,
                index: 0,
            }));
            let blur = s.doc.find_filter(first).unwrap().1.settings().clone();
            let settings = blur
                .with_catalog_values(
                    &[(
                        "radius".to_owned(),
                        yolu_core::effects::catalog::ParamValue::Number(6.0),
                    )]
                    .into_iter()
                    .collect(),
                )
                .unwrap();
            s.apply(Action::Fx(FxOp::SetSettings {
                layer,
                id: first,
                settings,
                coalesce: true,
            }));
            s.m2_end_drag();
            s.apply(Action::Fx(FxOp::Remove { layer, id }));
        },
    );
    assert!(commands
        .iter()
        .any(|c| matches!(c, Command::EffectDelete(_))));
    let texts: Vec<String> = commands
        .iter()
        .map(|c| serde_json::to_string(c).unwrap())
        .collect();
    assert!(
        texts.iter().any(|t| t.contains("\"$created:2\"")),
        "{texts:?}"
    );
}

// ───────── 写せない操作 ─────────

#[test]
fn operations_without_a_command_are_not_recorded_and_raise_the_mark() {
    check_skipped("複製", none, |s| {
        s.apply(Action::M2(Edit::DuplicateSelected))
    });
    check_skipped("チャンネルの不透明度", none, |s| {
        let id = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::Opacity {
            id,
            channel: Some(Channel::Color),
            value: 0.5,
        }))
    });
    check_skipped("Anchor の Generator", none, |s| {
        s.apply(Action::Fx(FxOp::AddGenerator {
            target: FilterTarget::Content,
            kind: GenKind::Anchor,
        }))
    });
    check_skipped("描く（文書へ直に）", none, |s| {
        let id = s.selected_layer.unwrap();
        s.doc
            .set_pixel(id, 1, 1, Rgba8::new(255, 0, 0, 255))
            .unwrap();
    });
    check_skipped("継ぎ目をまたぐ読みの入り切り", none, |s| {
        let on = !s.doc.filter_seams();
        s.apply(Action::Fx(FxOp::SetFilterSeams(on)))
    });
    // 名前が 1 つに決まらないレイヤー（選んでいた・作ったレイヤーでもない）
    check_skipped(
        "同じ名前のレイヤー",
        |s| {
            let a = layer(s, "レイヤー 1");
            s.doc.set_layer_name(a, "Same").unwrap();
            let b = layer(s, "レイヤー 4");
            s.doc.set_layer_name(b, "Same").unwrap();
        },
        |s| {
            let id = s
                .doc
                .layers()
                .iter()
                .find(|l| l.name() == "Same")
                .unwrap()
                .id();
            s.apply(Action::ToggleVisible(id))
        },
    );
    // 記録の前からある効果（指す名前が無い）
    check_skipped(
        "前からある効果",
        |s| {
            s.apply(Action::Fx(FxOp::AddFilter {
                target: FilterTarget::Content,
                kind: FilterKind::Blur,
            }))
        },
        |s| {
            let (layer, id) = last_effect(s, FilterTarget::Content);
            s.apply(Action::Fx(FxOp::SetEnabled {
                layer,
                id,
                enabled: false,
            }))
        },
    );
    // 画面だけの操作は記録も印もしない
    let mut s = fixture();
    start(&mut s);
    s.apply(Action::ZoomIn);
    s.apply(Action::SwapColors);
    yolu_app::automation::record::sync(&mut s);
    assert!(recorded(&s).is_empty() && !skipped(&s));
}

#[test]
fn undo_and_redo_while_recording_take_the_recorded_steps_off_and_back() {
    let mut s = fixture();
    start(&mut s);
    s.apply(Action::NewLayer);
    s.apply(Action::M2(Edit::NewGroup));
    assert_eq!(recorded(&s).len(), 2);
    s.apply(Action::Undo);
    assert_eq!(recorded(&s).len(), 1);
    s.apply(Action::Undo);
    assert!(recorded(&s).is_empty());
    s.apply(Action::Redo);
    s.apply(Action::Redo);
    assert_eq!(recorded(&s).len(), 2);
    // 新しい操作の後はやり直しで戻さない
    s.apply(Action::Undo);
    s.apply(Action::NewLayer);
    let commands = recorded(&s);
    assert_eq!(commands.len(), 2);
    assert!(!skipped(&s));
    // 記録の外の段（描いた線）を取り消しても、記録した段は残る
    let id = s.selected_layer.unwrap();
    s.doc
        .set_pixel(id, 2, 2, Rgba8::new(0, 255, 0, 255))
        .unwrap();
    s.apply(Action::Undo);
    assert_eq!(recorded(&s).len(), 2);
    assert!(skipped(&s), "描いた線は記録しなかった");
}

#[test]
fn recording_only_follows_the_document_it_started_on() {
    let mut s = fixture();
    start(&mut s);
    // ほかの文書（セットを替えた・開き直した）での操作は記録しない
    let original = std::mem::replace(&mut s.doc, Document::new(16, 16).unwrap());
    s.doc.add_layer("x").unwrap();
    s.selected_layer = Some(s.doc.layers()[0].id());
    s.apply(Action::NewLayer);
    assert!(recorded(&s).is_empty());
    assert!(skipped(&s));
    s.doc = original;
    s.ensure_selection();
    s.apply(Action::NewLayer);
    assert_eq!(recorded(&s).len(), 1);
}

// ───────── 再生（アプリ） ─────────

/// 設定のフォルダの actions/ を持つアプリ。
fn with_store(s: &mut AppState) -> std::path::PathBuf {
    let dir = test_dir("actions");
    let problems = yolu_app::automation::attach(s, dir.clone());
    assert!(problems.is_none());
    dir
}

fn action_of(values: &[Value]) -> Vec<Command> {
    values.iter().map(|v| parse_command(v).unwrap()).collect()
}

#[test]
fn stopping_saves_a_file_the_cli_can_run_and_playing_is_one_undo_step() {
    let mut s = fixture();
    let dir = with_store(&mut s);
    // 何も記録しなければ保存しない
    start(&mut s);
    s.apply(Action::Automation(AutomationOp::StopRecording));
    assert!(s.automation.store.is_empty());
    assert_eq!(s.last_notice.as_ref().unwrap().kind, Kind::Info);
    start(&mut s);
    s.apply(Action::NewLayer);
    s.apply(Action::Fx(FxOp::AddFilter {
        target: FilterTarget::Content,
        kind: FilterKind::Blur,
    }));
    s.apply(Action::Automation(AutomationOp::StopRecording));
    assert!(!s.automation.is_recording());
    assert_eq!(s.automation.store.len(), 1);
    let name = s.automation.store.get(0).unwrap().name.clone();
    assert_eq!(name, "アクション 1");
    let text = std::fs::read_to_string(dir.join("アクション 1.json")).unwrap();
    let file = yolu_ops::action::parse_action(&text).unwrap();
    assert_eq!(file.commands.len(), 2);
    // 別の文書で再生 → 取り消し 1 回で全部戻る
    let mut other = fixture();
    other.automation = std::mem::take(&mut s.automation);
    let before = fingerprint(&other.doc);
    other.apply(Action::Automation(AutomationOp::Play(0)));
    assert_eq!(
        other.last_notice.as_ref().unwrap().kind,
        Kind::Info,
        "{}",
        other.message
    );
    assert_eq!(other.doc.layers().len(), fixture().doc.layers().len() + 1);
    assert!(other.modified);
    other.apply(Action::Undo);
    assert_eq!(fingerprint(&other.doc), before);
    // 名前の変更・並べ替え・削除
    other.apply(Action::Automation(AutomationOp::Rename(
        0,
        "Blur group".into(),
    )));
    assert!(dir.join("Blur group.json").is_file() && !dir.join("アクション 1.json").exists());
    let mut again = ActionStore::default();
    again.attach(dir.clone());
    assert_eq!(again.get(0).unwrap().name, "Blur group");
    other.apply(Action::Automation(AutomationOp::Delete(0)));
    assert!(other.automation.store.is_empty() && !dir.join("Blur group.json").exists());
}

#[test]
fn a_failed_order_write_still_saves_the_recording_once_and_only_warns() {
    let mut s = fixture();
    let dir = with_store(&mut s);
    // 並びのファイルの場所をフォルダにして、書けなくする
    std::fs::create_dir_all(dir.join("order.json")).unwrap();
    start(&mut s);
    s.apply(Action::NewLayer);
    s.apply(Action::Automation(AutomationOp::StopRecording));
    assert!(!s.automation.is_recording(), "保存できたので記録は終わる");
    assert_eq!(s.automation.store.len(), 1);
    assert!(dir.join("アクション 1.json").is_file());
    assert_eq!(
        s.last_notice.as_ref().unwrap().kind,
        Kind::Warning,
        "{}",
        s.message
    );
    assert!(
        s.message.contains("保存しました") && s.message.contains("並び"),
        "{}",
        s.message
    );
    // もう 1 度止める操作をしても、同じ記録がもう 1 つ保存されない
    s.apply(Action::Automation(AutomationOp::StopRecording));
    assert_eq!(s.automation.store.len(), 1);
    let saved = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".json") && e.path().is_file())
        .count();
    assert_eq!(saved, 1);
    // 名前の変えも、済んでいるので失敗にはしない（注意として知らせる）
    s.apply(Action::Automation(AutomationOp::Rename(
        0,
        "Renamed".into(),
    )));
    assert_eq!(
        s.last_notice.as_ref().unwrap().kind,
        Kind::Warning,
        "{}",
        s.message
    );
    assert!(dir.join("Renamed.json").is_file() && !dir.join("アクション 1.json").exists());
}

#[test]
fn a_refused_command_rolls_the_whole_play_back_and_says_which_one() {
    let mut s = fixture();
    with_store(&mut s);
    s.automation
        .store
        .add(
            "Broken",
            action_of(&[
                json!({"command": "layer.add", "args": {"kind": "paint", "name": "Made"}}),
                json!({"command": "layer.set", "args": {"layer": "No such layer", "visible": false}}),
            ]),
        )
        .unwrap();
    let before = fingerprint(&s.doc);
    let undo = s.doc.undo_count();
    s.apply(Action::Automation(AutomationOp::Play(0)));
    let notice = s.last_notice.clone().unwrap();
    assert_eq!(notice.kind, Kind::Error);
    assert!(
        notice.text.contains("2 番目の layer.set"),
        "{}",
        notice.text
    );
    assert!(notice.text.contains("No such layer"), "{}", notice.text);
    assert_eq!(fingerprint(&s.doc), before);
    assert_eq!(s.doc.undo_count(), undo);
    assert!(
        s.notice_log.entries().any(|e| e.notice.text == notice.text),
        "ログに残る"
    );
    // 英語
    s.set_language(Lang::En);
    s.apply(Action::Automation(AutomationOp::Play(0)));
    let text = &s.last_notice.as_ref().unwrap().text;
    assert!(
        text.starts_with("Cannot play action \"Broken\" (command 2 layer.set"),
        "{text}"
    );
}

#[test]
fn playing_is_refused_while_drawing_recording_or_on_a_read_only_set() {
    let mut s = fixture();
    with_store(&mut s);
    s.automation
        .store
        .add(
            "Add",
            action_of(&[json!({"command": "layer.add", "args": {"kind": "paint"}})]),
        )
        .unwrap();
    let before = fingerprint(&s.doc);
    // 描いている最中
    let id = s.selected_layer.unwrap();
    let brush = s.stroke_settings(false);
    let mut stroke = s.doc.begin_stroke(id, &brush).unwrap();
    stroke
        .add_point(&mut s.doc, 4.0, 4.0, 1.0, yolu_app::engine::DVec2::ZERO)
        .unwrap();
    s.apply(Action::Automation(AutomationOp::Play(0)));
    assert_eq!(s.last_notice.as_ref().unwrap().kind, Kind::Refusal);
    s.doc.end_stroke(stroke).unwrap();
    s.doc.undo().unwrap();
    assert_eq!(fingerprint(&s.doc), before);
    // 記録中
    start(&mut s);
    s.apply(Action::Automation(AutomationOp::Play(0)));
    assert_eq!(s.last_notice.as_ref().unwrap().kind, Kind::Refusal);
    s.automation.recorder = None;
    assert_eq!(fingerprint(&s.doc), before);
    // 読むだけのセット
    s.sets.get_mut(0).unwrap().read_only = Some("読めない中身".into());
    s.apply(Action::Automation(AutomationOp::Play(0)));
    assert_eq!(s.last_notice.as_ref().unwrap().kind, Kind::Refusal);
    assert_eq!(fingerprint(&s.doc), before);
}

#[test]
fn selected_in_the_app_is_the_selected_layer_and_created_is_per_run() {
    let mut s = fixture();
    let second = layer(&s, "レイヤー 2");
    let commands = action_of(&[
        json!({"command": "mask.add", "args": {"layer": "$selected"}}),
        json!({"command": "layer.add", "args": {"kind": "paint", "name": "Top", "above": "$selected"}}),
        json!({"command": "layer.set", "args": {"layer": "$created:1", "opacity": 0.5}}),
    ]);
    play(&mut s, &commands).unwrap();
    assert!(s.doc.layer(second).unwrap().mask().is_some());
    let top = layer(&s, "Top");
    assert_eq!(s.doc.layer(top).unwrap().opacity(), 0.5);
    let index = |id: LayerId| s.doc.layer_index(id).unwrap();
    assert_eq!(index(top), index(second) + 1, "選んでいたレイヤーのすぐ上");
}
