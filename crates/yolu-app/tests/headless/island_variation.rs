//! アイランドごとのばらつき（Generator の種類 67）の画面の操作: 「ジェネレーターを追加」のメニューに日英の名前で出て押せる・追加は 1 回の
//! 取り消し・欄（シード・最小・最大）の変更が 1 回ずつの取り消し・モデルが無ければ理由を言って入力のまま、モデルを読むと島ごとに違う値・
//! 保存して開き直すと同じ設定。
use yolu_app::fx::FxOp;
use yolu_app::lang::Lang;
use yolu_app::state::{Action, AppState};
use yolu_app::view3d::model::ViewModel;
use yolu_core::generator::{self, Inactive, Kind};
use yolu_core::geometry::{ModelMesh, Submesh};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::{EffectSettings, FilterTarget, InactiveReason, LayerId, Rect, Rgba8};

const SIZE: u32 = 64;

/// 3D で離れた 2 枚の板。UV は左下（0.1..0.4）と右下（0.6..0.9）の別の島。
fn two_plates() -> ViewModel {
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    for (k, u0) in [0.1f32, 0.6].into_iter().enumerate() {
        let z = k as f32 * 5.0;
        let base = positions.len() as u32;
        for (x, y) in [(0., 0.), (1., 0.), (0., 1.), (1., 1.)] {
            positions.push(Vec3::new(x, y, z));
            uvs.push(Vec2::new(u0 + 0.3 * x, 0.1 + 0.3 * y));
        }
        indices.extend([0, 2, 1, 2, 3, 1].map(|i| base + i));
    }
    let mesh = ModelMesh {
        name: "板".into(),
        positions,
        normals: Vec::new(),
        uvs,
        submeshes: vec![Submesh {
            material: 0,
            indices,
        }],
    };
    ViewModel::new("板", vec![mesh], vec![Some("材".into())], 1).unwrap()
}

/// 全面を塗った層を選んだ状態。
fn painted() -> (AppState, LayerId) {
    let mut s = AppState::new(SIZE, SIZE);
    let layer = s.selected_layer.unwrap();
    for y in 0..SIZE {
        for x in 0..SIZE {
            s.doc
                .set_pixel(layer, x, y, Rgba8::new(90, 140, 200, 255))
                .unwrap();
        }
    }
    s.doc.clear_history().unwrap();
    (s, layer)
}

fn stage(s: &AppState, layer: LayerId) -> (yolu_core::FilterId, generator::Settings) {
    let stages = s.doc.filters_of(layer, FilterTarget::Content).unwrap();
    let e = stages.last().unwrap();
    (e.id(), e.settings().generator_settings().unwrap().clone())
}

fn at(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    pixels[i..i + 4].try_into().unwrap()
}

#[test]
fn the_generator_menu_lists_it_in_both_languages() {
    use yolu_app::ui::menu::Entry;
    let (mut s, _) = painted();
    for lang in Lang::ALL {
        s.lang = lang;
        let entries = yolu_app::fx::menu::add_generator_entries(&s, FilterTarget::Content);
        let name = yolu_app::fx::names::generator_name(lang, Kind::UvIslandVariation);
        assert_eq!(
            name,
            lang.pick("アイランドごとのばらつき", "UV Island Variation")
        );
        let found = entries.iter().any(|e| {
            matches!(e, Entry::Item {
                label,
                enabled: true,
                action: Action::Fx(FxOp::AddGenerator { kind: Kind::UvIslandVariation, .. }),
                ..
            } if label == name)
        });
        assert!(found, "{lang:?}");
    }
    // 焼いたマップ・モデルを読む種類の後ろ（マスクの組み立ての次）
    let kinds = yolu_app::fx::names::GENERATOR_KINDS;
    let at = |k| kinds.iter().position(|x| *x == k).unwrap();
    assert_eq!(at(Kind::UvIslandVariation), at(Kind::MaskBuilder) + 1);
}

#[test]
fn adding_and_changing_the_fields_are_one_undo_step_each() {
    let (mut s, layer) = painted();
    let before = s.doc.undo_count();
    s.apply(Action::Fx(FxOp::AddGenerator {
        target: FilterTarget::Content,
        kind: Kind::UvIslandVariation,
    }));
    assert_eq!(s.doc.undo_count(), before + 1, "{}", s.message);
    let (id, g) = stage(&s, layer);
    assert_eq!(g, generator::Settings::new(Kind::UvIslandVariation));
    // シード・最小・最大を変える（それぞれ 1 回の取り消し）
    for (k, change) in [
        |g: &mut generator::Settings| g.island.seed = -77,
        |g: &mut generator::Settings| g.island.min = 0.25,
        |g: &mut generator::Settings| g.island.max = 0.5,
    ]
    .into_iter()
    .enumerate()
    {
        let mut next = stage(&s, layer).1;
        change(&mut next);
        let steps = s.doc.undo_count();
        s.apply(Action::Fx(FxOp::SetSettings {
            layer,
            id,
            settings: EffectSettings::generator(next.clone()),
            coalesce: false,
        }));
        assert_eq!(s.doc.undo_count(), steps + 1, "{k}: {}", s.message);
        assert_eq!(stage(&s, layer).1, next);
    }
    // 最小が最大を超える設定は断り、何も変えない
    let mut bad = stage(&s, layer).1;
    bad.island.min = 0.9;
    let steps = s.doc.undo_count();
    s.apply(Action::Fx(FxOp::SetSettings {
        layer,
        id,
        settings: EffectSettings::generator(bad),
        coalesce: false,
    }));
    assert_eq!(s.doc.undo_count(), steps);
    assert_eq!(stage(&s, layer).1.island.min, 0.25);
    // 取り消すと前の値
    s.apply(Action::Undo);
    assert_eq!(stage(&s, layer).1.island.max, 1.0);
}

#[test]
fn without_a_model_it_says_why_and_with_one_each_island_gets_its_own_value() {
    let (mut s, layer) = painted();
    s.apply(Action::Fx(FxOp::AddGenerator {
        target: FilterTarget::Content,
        kind: Kind::UvIslandVariation,
    }));
    let (id, mut g) = stage(&s, layer);
    g.blend = generator::Blend::Replace;
    s.apply(Action::Fx(FxOp::SetSettings {
        layer,
        id,
        settings: EffectSettings::generator(g),
        coalesce: false,
    }));
    s.sync_effect_inputs_with(true);
    let why = s.doc.generator_inactive(layer, id).unwrap().unwrap();
    assert_eq!(why, InactiveReason::Generator(Inactive::NoModel));
    assert_eq!(Lang::En.inactive_reason(&why), "No model");
    assert_eq!(Lang::Ja.inactive_reason(&why), "モデルがありません");
    // モデルを読むと効く: 2 つの島は 1 つずつの灰色で、違う値。島の外は入力のまま
    s.view3d.set_model(two_plates());
    s.sync_effect_inputs_with(true);
    assert_eq!(s.doc.generator_inactive(layer, id).unwrap(), None);
    let pixels = s
        .doc
        .composite_channel(yolu_core::Channel::Color, Rect::new(0, 0, SIZE, SIZE))
        .unwrap();
    let (a, b) = (at(&pixels, 16, 16), at(&pixels, 48, 16));
    assert!(a[0] == a[1] && a[1] == a[2], "{a:?}");
    assert!(b[0] == b[1] && b[1] == b[2], "{b:?}");
    assert_ne!(a, b);
    assert_eq!(at(&pixels, 17, 20), a);
    assert_eq!(at(&pixels, 32, 48), [90, 140, 200, 255]);
}

/// 島ごとのばらつきの段を効かせた 2 枚の板の文書（Replace で塗る）を、モデルを読んだ状態で .ylp に保存する。
fn saved_with_the_model(path: &std::path::Path) -> (AppState, LayerId, yolu_core::FilterId) {
    let (mut s, layer) = painted();
    s.view3d.set_model(two_plates());
    s.apply(Action::Fx(FxOp::AddGenerator {
        target: FilterTarget::Content,
        kind: Kind::UvIslandVariation,
    }));
    let (id, mut g) = stage(&s, layer);
    g.blend = generator::Blend::Replace;
    s.apply(Action::Fx(FxOp::SetSettings {
        layer,
        id,
        settings: EffectSettings::generator(g),
        coalesce: false,
    }));
    s.sync_effect_inputs_with(true);
    assert_eq!(s.doc.generator_inactive(layer, id).unwrap(), None);
    s.apply(Action::SaveProjectAs(path.to_path_buf()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    (s, layer, id)
}

fn composite_pixels(s: &AppState) -> Vec<u8> {
    s.doc
        .composite_channel(yolu_core::Channel::Color, Rect::new(0, 0, SIZE, SIZE))
        .unwrap()
}

#[test]
fn opening_without_the_model_keeps_the_saved_composite_read_only_until_the_model_arrives() {
    let dir = std::env::temp_dir().join(format!("yolu-island-lock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("islands.ylp");
    let (saved, _, _) = saved_with_the_model(&path);
    let shown = composite_pixels(&saved);
    assert_ne!(at(&shown, 16, 16), at(&shown, 48, 16), "島ごとに違う値");

    // モデルの無い状態で開く: 島ごとの値は評価できないので、読むだけにして、保存した合成を見せ、足りない入力（モデル）を言う
    let mut again = AppState::new(SIZE, SIZE);
    again.apply(Action::OpenProject(path.clone()));
    let reason = again
        .read_only_reason()
        .expect("モデルが無いと読むだけ")
        .to_owned();
    assert!(reason.contains("効果の入力がそろっていない"), "{reason}");
    assert!(reason.contains("モデルがありません"), "{reason}");
    assert!(again.sets.current().waiting_inputs);
    assert_eq!(
        composite_pixels(&again),
        shown,
        "保存した合成のまま（島の値を落とした絵にしない）"
    );
    // 文書を変える操作は断る（保存で合成が島の値を落とした絵に書き直されない）
    let steps = again.doc.undo_count();
    again.apply(Action::Fx(FxOp::AddGenerator {
        target: FilterTarget::Content,
        kind: Kind::UvIslandVariation,
    }));
    assert_eq!(again.doc.undo_count(), steps);
    assert!(again.message.contains("読むだけ"), "{}", again.message);

    // モデルを読むと入力がそろい、同じ設定のまま編集できて、同じ絵になる
    again.view3d.set_model(two_plates());
    again.sync_effect_inputs_with(true);
    assert!(
        again.read_only_reason().is_none(),
        "{:?} {}",
        again.read_only_reason(),
        again.message
    );
    assert!(!again.sets.current().waiting_inputs);
    let layer = again.doc.layers()[0].id();
    let (id, _) = stage(&again, layer);
    assert_eq!(again.doc.generator_inactive(layer, id).unwrap(), None);
    assert_eq!(composite_pixels(&again), shown);
    let steps = again.doc.undo_count();
    again.apply(Action::Fx(FxOp::AddGenerator {
        target: FilterTarget::Content,
        kind: Kind::UvIslandVariation,
    }));
    assert_eq!(again.doc.undo_count(), steps + 1, "{}", again.message);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_settings_come_back_after_save_and_reopen() {
    let dir = std::env::temp_dir().join(format!("yolu-island-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("islands.ylp");
    let (mut s, layer) = painted();
    s.apply(Action::Fx(FxOp::AddGenerator {
        target: FilterTarget::Content,
        kind: Kind::UvIslandVariation,
    }));
    let (id, mut g) = stage(&s, layer);
    g.island.seed = i32::MAX;
    g.island.min = 0.3;
    g.island.max = 0.3;
    g.softness = 0.25;
    s.apply(Action::Fx(FxOp::SetSettings {
        layer,
        id,
        settings: EffectSettings::generator(g.clone()),
        coalesce: false,
    }));
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let mut again = AppState::new(SIZE, SIZE);
    again.apply(Action::OpenProject(path));
    // モデルが無いあいだは読むだけ（正本の設定は保存したまま）。モデルを読むと編集できて、設定が同じ
    assert!(again.sets.current().waiting_inputs, "{}", again.message);
    again.view3d.set_model(two_plates());
    again.sync_effect_inputs_with(true);
    assert!(again.read_only_reason().is_none(), "{}", again.message);
    let reopened = again.doc.layers()[0].id();
    assert_eq!(stage(&again, reopened).1, g);
    let _ = std::fs::remove_dir_all(dir);
}
