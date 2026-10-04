//! 塗りつぶしの層の画像と投影・デカール・ワールドスペースのグラデーションと、グラデーションの道具（Shift+G）の試験。`headless_` で始まる試験は
//! 画面を描かず、Wine でも回る。どれも「操作 → 文書が変わる → 取り消し 1 回で戻る」（ロックの層は断って何も変えない・保存して開き直すと同じ）。
//! マップ（位置・法線）は、試しの立方体を CPU で焼いた結果を文書の効果の入力へ足して使う（入力を渡す側は別の担当。ここは渡された後の振る舞い）。
mod common;

use std::path::PathBuf;

use egui::{pos2, vec2, Rect};
use yolu_app::bake::{BakeAction, BakeBackend};
use yolu_app::engine::{Channel, Document, LayerId, Rgba8, TileCoord};
use yolu_app::fillfx::{gizmo, inputs, FillOp};
use yolu_app::gradient::{End, GradientOp};
use yolu_app::lang::Lang;
use yolu_app::m2::Edit;
use yolu_app::matpaint::MatAction;
use yolu_app::shelf::ShelfOp;
use yolu_app::state::{Action, AppState, Tool};
use yolu_app::view3d::shape_gizmo::{Handle, Mode};
use yolu_core::fill_image::{Placement, Projection, ProjectionMode, Wrap};
use yolu_core::generator::{Blend, Kind, Shape};
use yolu_core::glam::Vec3;
use yolu_core::material::GradientShape;
use yolu_core::mesh_maps::MeshMapKind;
use yolu_core::{ImageId, LayerLocks, MapInput};

const RED: Rgba8 = Rgba8::new(255, 0, 0, 255);
const GREEN: Rgba8 = Rgba8::new(0, 255, 0, 255);
const BLUE: Rgba8 = Rgba8::new(0, 0, 255, 255);
const WHITE: Rgba8 = Rgba8::new(255, 255, 255, 255);

/// 2 × 2 の画像（下の行が先: 下は赤・緑、上は青・白）。
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

fn new_fill(s: &mut AppState) -> LayerId {
    s.apply(Action::M2(Edit::NewFill));
    s.selected_layer.expect("足した層を選ぶ")
}

fn fill(s: &mut AppState, op: FillOp) {
    s.apply(Action::Fill(op));
}

/// 試しの立方体を読み、位置と法線のマップを CPU で焼いて、文書の効果の入力へ足した状態（64 × 64）。
fn cube() -> AppState {
    let mut s = AppState::new(64, 64);
    s.bake.backend = BakeBackend::Cpu;
    s.apply(Action::LoadDemoModel);
    s.bake.settings.maps = vec![MeshMapKind::WorldNormal, MeshMapKind::Position];
    s.bake.settings.padding = 4;
    s.apply(Action::Bake(BakeAction::Start));
    s.wait_bake();
    give_maps(&mut s);
    s
}

fn give_maps(s: &mut AppState) {
    use yolu_core::generator::MapState;
    let mut inputs = s.doc.effect_inputs().clone();
    for kind in [MeshMapKind::Position, MeshMapKind::WorldNormal] {
        let map = s
            .sets
            .current()
            .mesh_maps
            .get(kind)
            .expect("焼いたマップ")
            .clone();
        inputs = inputs
            .with_map(MapInput::from_baked(&map, MapState::Current).expect("マップ"))
            .expect("入力");
    }
    s.doc.set_effect_inputs(inputs).expect("入力を置く");
}

/// 文書の合成の色（Color）を 4 画素おきに。
fn composite(doc: &Document) -> Vec<[u8; 4]> {
    let mut out = Vec::new();
    for y in (0..doc.height()).step_by(4) {
        for x in (0..doc.width()).step_by(4) {
            let p = doc.composite_pixel(Channel::Color, x, y).unwrap();
            out.push([p.r, p.g, p.b, p.a]);
        }
    }
    out
}

fn px(doc: &Document, x: u32, y: u32) -> Rgba8 {
    doc.composite_pixel(Channel::Color, x, y).unwrap()
}

fn layer_projection(s: &AppState, id: LayerId) -> Projection {
    *s.doc.layer(id).unwrap().projection()
}

fn view_rect() -> Rect {
    Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yolu-fillfx-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ───────── 画像 ─────────

#[test]
fn headless_choosing_an_image_reads_it_into_the_channel_and_one_undo_takes_it_back() {
    let mut s = AppState::new(64, 64);
    let (_, image) = shelf_image(&mut s, "四色");
    let layer = new_fill(&mut s);
    let before = composite(&s.doc);
    assert!(px(&s.doc, 5, 5).a == 255, "塗りつぶしの値で全面を覆う");
    let steps = s.doc.undo_count();
    fill(
        &mut s,
        FillOp::Image {
            layer,
            channel: Channel::Color,
            image: Some(image),
        },
    );
    assert_eq!(
        s.doc.layer(layer).unwrap().fill_image(Channel::Color),
        Some(image)
    );
    assert_eq!(s.doc.undo_count(), steps + 1, "1 回の Undo");
    assert!(s.message.contains("四色"), "{}", s.message);
    assert!(s.modified);
    // UV の正方形に 4 色が敷かれる
    let quadrants = [
        px(&s.doc, 5, 5),
        px(&s.doc, 58, 5),
        px(&s.doc, 5, 58),
        px(&s.doc, 58, 58),
    ];
    let mut distinct = quadrants.to_vec();
    distinct.sort_by_key(|c| (c.r, c.g, c.b));
    distinct.dedup();
    assert_eq!(distinct.len(), 4, "4 色が別々の所に出る: {quadrants:?}");
    // 2 × 2 の画像は補間されて敷かれるので、元の色そのものではなく、赤・緑・青に寄った色と、明るい色が出る
    let red = quadrants
        .iter()
        .filter(|c| c.r > c.g + 30 && c.r > c.b + 30)
        .count();
    let green = quadrants
        .iter()
        .filter(|c| c.g > c.r + 30 && c.g > c.b + 30)
        .count();
    let blue = quadrants
        .iter()
        .filter(|c| c.b > c.r + 30 && c.b > c.g + 30)
        .count();
    let light = quadrants
        .iter()
        .filter(|c| c.r >= 130 && c.g >= 130 && c.b >= 130)
        .count();
    assert_eq!((red, green, blue, light), (1, 1, 1, 1), "{quadrants:?}");
    // 取り消し・やり直し
    assert!(s.doc.undo().unwrap());
    assert_eq!(s.doc.layer(layer).unwrap().fill_image(Channel::Color), None);
    assert_eq!(composite(&s.doc), before);
    assert!(s.doc.redo().unwrap());
    assert_eq!(
        s.doc.layer(layer).unwrap().fill_image(Channel::Color),
        Some(image)
    );
    // 画像を外す: 値に戻る（値は残る）。1 回の Undo
    let steps = s.doc.undo_count();
    fill(
        &mut s,
        FillOp::Image {
            layer,
            channel: Channel::Color,
            image: None,
        },
    );
    assert_eq!(s.doc.undo_count(), steps + 1);
    assert_eq!(s.doc.layer(layer).unwrap().fill_image(Channel::Color), None);
    assert!(s
        .doc
        .layer(layer)
        .unwrap()
        .fill_value(Channel::Color)
        .is_some());
    assert_eq!(composite(&s.doc), before);
}

#[test]
fn headless_an_image_missing_from_the_shelf_is_refused_and_a_missing_one_in_a_file_shows_the_value_with_a_reason(
) {
    let mut s = AppState::new(64, 64);
    let layer = new_fill(&mut s);
    // 棚に無い画像は断って何も変えない
    let ghost = ImageId(0x1234_5678_9abc_def0_1234_5678_9abc_def0);
    let steps = s.doc.undo_count();
    fill(
        &mut s,
        FillOp::Image {
            layer,
            channel: Channel::Color,
            image: Some(ghost),
        },
    );
    assert_eq!(s.doc.undo_count(), steps);
    assert_eq!(s.doc.layer(layer).unwrap().fill_image(Channel::Color), None);
    assert!(!s.message.is_empty());
    // 英語でも理由を言う
    s.lang = Lang::En;
    fill(
        &mut s,
        FillOp::Image {
            layer,
            channel: Channel::Color,
            image: Some(ghost),
        },
    );
    assert!(s.message.is_ascii(), "{}", s.message);
    // 画像を持つ層が棚に画像の無い文書へ入ったら（読み込みの形）、値を見せ、理由を言う
    s.lang = Lang::Ja;
    let (rid, image) = shelf_image(&mut s, "四色");
    fill(
        &mut s,
        FillOp::Image {
            layer,
            channel: Channel::Color,
            image: Some(image),
        },
    );
    assert!(
        s.fill_image_problem(layer, Channel::Color).is_none(),
        "読めている間は理由が無い"
    );
    // 入力から画像を落とす（棚は変えない）と、次の同期で戻る
    s.doc
        .set_effect_inputs(yolu_core::EffectInputs::new())
        .unwrap();
    assert!(
        s.fill_image_problem(layer, Channel::Color).is_some(),
        "入力に画像が無い"
    );
    s.sync_effects();
    assert!(
        s.fill_image_problem(layer, Channel::Color).is_none(),
        "棚から読み直す"
    );
    let _ = rid;
}

#[test]
fn headless_the_image_and_projection_edits_are_refused_by_locks_and_by_read_only_sets() {
    let mut s = cube();
    let (_, image) = shelf_image(&mut s, "四色");
    let layer = new_fill(&mut s);
    fill(
        &mut s,
        FillOp::Image {
            layer,
            channel: Channel::Color,
            image: Some(image),
        },
    );
    s.doc.set_layer_locks(layer, LayerLocks::ALL).unwrap();
    let steps = s.doc.undo_count();
    let revision = s.doc.revision();
    let p = layer_projection(&s, layer);
    for op in [
        FillOp::Image {
            layer,
            channel: Channel::Color,
            image: None,
        },
        FillOp::Projection {
            layer,
            projection: Box::new(Projection {
                mode: ProjectionMode::Planar,
                ..p
            }),
            coalesce: false,
        },
        FillOp::ProjectionMode {
            layer,
            mode: ProjectionMode::Triplanar,
        },
        FillOp::FitPlacement { layer },
        FillOp::AddGradient {
            layer,
            channel: Channel::Roughness,
        },
    ] {
        s.message.clear();
        fill(&mut s, op.clone());
        assert!(!s.message.is_empty(), "{op:?} は理由を言う");
        assert_eq!(s.doc.undo_count(), steps, "{op:?} は何も変えない");
        assert_eq!(s.doc.revision(), revision, "{op:?}");
    }
    assert_eq!(layer_projection(&s, layer), p);
    // ロックを外せば通る
    s.doc.set_layer_locks(layer, LayerLocks::NONE).unwrap();
    let steps = s.doc.undo_count();
    fill(
        &mut s,
        FillOp::ProjectionMode {
            layer,
            mode: ProjectionMode::Triplanar,
        },
    );
    assert_eq!(layer_projection(&s, layer).mode, ProjectionMode::Triplanar);
    assert!(s.doc.undo_count() > steps);
    // 読むだけのテクスチャセットは文書を変える操作を断る（Action の入口）
    s.sets.get_mut(0).unwrap().read_only = Some("理由".into());
    let steps = s.doc.undo_count();
    s.apply(Action::Fill(FillOp::ProjectionMode {
        layer,
        mode: ProjectionMode::Planar,
    }));
    assert_eq!(s.doc.undo_count(), steps);
    assert!(s.message.contains("読むだけ"), "{}", s.message);
    // 画面だけの操作（ハンドルを隠す）は読むだけでも通る
    s.apply(Action::Fill(FillOp::Handles(true)));
    assert!(s.fillfx.handles_hidden);
}

// ───────── 投影 ─────────

#[test]
fn headless_choosing_a_projection_fits_the_model_and_shows_the_handles_and_each_change_is_one_undo()
{
    let mut s = cube();
    let layer = new_fill(&mut s);
    let uv = layer_projection(&s, layer);
    assert_eq!(uv.mode, ProjectionMode::Uv);
    // UV → トライプラナー: 置き場が初めのままならモデルの外形（立方体は 1、× 1.02、トライプラナーは立方体）に合わせる
    s.fillfx.handles_hidden = true;
    let steps = s.doc.undo_count();
    fill(
        &mut s,
        FillOp::ProjectionMode {
            layer,
            mode: ProjectionMode::Triplanar,
        },
    );
    let p = layer_projection(&s, layer);
    assert_eq!(p.mode, ProjectionMode::Triplanar);
    assert_eq!(s.doc.undo_count(), steps + 1);
    assert!(
        (p.placement.size[0] - 1.02).abs() < 1e-5,
        "{:?}",
        p.placement
    );
    assert_eq!(p.placement.size[0], p.placement.size[2]);
    assert!(
        p.placement.center.iter().all(|c| c.abs() < 1e-6),
        "立方体の中心は原点"
    );
    assert!(
        !s.fillfx.handles_hidden,
        "型の上の投影を選んだら置き場のハンドルを出す"
    );
    // 平面へ: 置き場は動かした後なので、そのまま
    let mut moved = p;
    moved.placement.center = [0.25, 0.0, 0.0];
    fill(
        &mut s,
        FillOp::Projection {
            layer,
            projection: Box::new(moved),
            coalesce: false,
        },
    );
    fill(
        &mut s,
        FillOp::ProjectionMode {
            layer,
            mode: ProjectionMode::Planar,
        },
    );
    assert_eq!(
        layer_projection(&s, layer).placement.center,
        [0.25, 0.0, 0.0]
    );
    // デカール: 画像は 1 回（繰り返さず、外は透明）
    fill(
        &mut s,
        FillOp::ProjectionMode {
            layer,
            mode: ProjectionMode::Decal,
        },
    );
    let d = layer_projection(&s, layer);
    assert_eq!((d.mode, d.wrap), (ProjectionMode::Decal, Wrap::None));
    // デカールを出ると、減衰は既定へ戻る（core はデカールだけが減衰を持てる）
    let mut cut = d;
    cut.depth_hardness = 0.3;
    cut.backface_angle = 45.0;
    fill(
        &mut s,
        FillOp::Projection {
            layer,
            projection: Box::new(cut),
            coalesce: false,
        },
    );
    fill(
        &mut s,
        FillOp::ProjectionMode {
            layer,
            mode: ProjectionMode::Spherical,
        },
    );
    let sph = layer_projection(&s, layer);
    assert_eq!(sph.mode, ProjectionMode::Spherical);
    assert_eq!((sph.depth_hardness, sph.backface_angle), (0.8, 90.0));
    // 同じ種類を選んでも何も足さない
    let steps = s.doc.undo_count();
    fill(
        &mut s,
        FillOp::ProjectionMode {
            layer,
            mode: ProjectionMode::Spherical,
        },
    );
    assert_eq!(s.doc.undo_count(), steps);
    // モデルに合わせる: 動かした置き場が外形へ戻る。1 回の Undo
    fill(&mut s, FillOp::FitPlacement { layer });
    let fit = layer_projection(&s, layer);
    assert!((fit.placement.size[0] - 1.02).abs() < 1e-5);
    assert!(s.doc.undo().unwrap());
    assert_eq!(layer_projection(&s, layer), sph);
    // モデルが無ければ合わせられない（理由）
    let mut bare = AppState::new(64, 64);
    let l = new_fill(&mut bare);
    fill(&mut bare, FillOp::FitPlacement { layer: l });
    assert!(bare.message.contains("モデル"), "{}", bare.message);
}

#[test]
fn headless_tiling_offset_and_rotation_drags_make_one_undo_step() {
    let mut s = AppState::new(64, 64);
    let layer = new_fill(&mut s);
    let start = layer_projection(&s, layer);
    let steps = s.doc.undo_count();
    for k in 1..=5 {
        let mut p = start;
        p.tiles = [1.0 + k as f64, 1.0];
        p.rotation = 10.0 * k as f64;
        fill(
            &mut s,
            FillOp::Projection {
                layer,
                projection: Box::new(p),
                coalesce: true,
            },
        );
    }
    s.m2_end_drag();
    assert_eq!(s.doc.undo_count(), steps + 1, "ドラッグは 1 回にまとまる");
    assert_eq!(layer_projection(&s, layer).tiles, [6.0, 1.0]);
    assert!(s.doc.undo().unwrap());
    assert_eq!(layer_projection(&s, layer), start);
    // 範囲外の値は core の検査で断る（理由は画面の言語）
    let mut bad = start;
    bad.tiles = [0.0, 1.0];
    fill(
        &mut s,
        FillOp::Projection {
            layer,
            projection: Box::new(bad),
            coalesce: false,
        },
    );
    assert!(
        s.message.contains("0.001") || s.message.contains("繰り返し"),
        "{}",
        s.message
    );
    s.lang = Lang::En;
    fill(
        &mut s,
        FillOp::Projection {
            layer,
            projection: Box::new(bad),
            coalesce: false,
        },
    );
    assert!(
        s.message.starts_with("Invalid projection") && s.message.is_ascii(),
        "{}",
        s.message
    );
    assert_eq!(layer_projection(&s, layer), start);
}

#[test]
fn headless_projected_values_are_the_cores_values() {
    // 同じ入力・同じ設定を、操作の道（app）と core を直に呼ぶ道で作り、合成の画素が全部同じ（app は値を変えない）
    let mut app_side = cube();
    let mut core_side = cube();
    let (_, image_a) = shelf_image(&mut app_side, "四色");
    let (_, image_b) = shelf_image(&mut core_side, "四色");
    for mode in [
        ProjectionMode::Planar,
        ProjectionMode::Triplanar,
        ProjectionMode::Spherical,
        ProjectionMode::Cylindrical,
    ] {
        let a = new_fill(&mut app_side);
        let b = new_fill(&mut core_side);
        fill(
            &mut app_side,
            FillOp::Image {
                layer: a,
                channel: Channel::Color,
                image: Some(image_a),
            },
        );
        fill(&mut app_side, FillOp::ProjectionMode { layer: a, mode });
        let placement = layer_projection(&app_side, a).placement;
        core_side.use_shelf_image(&inputs::resource_id(image_b)).unwrap();
        core_side
            .doc
            .set_fill_image(b, Channel::Color, Some(image_b))
            .unwrap();
        core_side
            .doc
            .set_fill_projection(
                b,
                Projection {
                    mode,
                    placement,
                    ..Projection::default()
                },
                false,
            )
            .unwrap();
        let ours = composite(&app_side.doc);
        assert_eq!(ours, composite(&core_side.doc), "{mode:?}");
        // マップが効いている（UV の敷き方と違う絵が、モデルの面に出ている）
        let uv_side = {
            let mut t = cube();
            let (_, im) = shelf_image(&mut t, "四色");
            let l = new_fill(&mut t);
            fill(
                &mut t,
                FillOp::Image {
                    layer: l,
                    channel: Channel::Color,
                    image: Some(im),
                },
            );
            composite(&t.doc)
        };
        assert_ne!(ours, uv_side, "{mode:?} は UV の敷き方と違う");
        // 次の種類のために層を消す
        for s in [&mut app_side, &mut core_side] {
            let id = s.selected_layer.unwrap();
            s.doc.remove_layer(id).unwrap();
            s.selected_layer = s.doc.layers().last().map(|l| l.id());
        }
    }
}

#[test]
fn headless_a_decal_outside_its_box_is_transparent_and_the_reason_is_told_without_maps() {
    let mut s = AppState::new(64, 64); // マップ無し
    let (_, image) = shelf_image(&mut s, "四色");
    let layer = new_fill(&mut s);
    fill(
        &mut s,
        FillOp::Image {
            layer,
            channel: Channel::Color,
            image: Some(image),
        },
    );
    fill(
        &mut s,
        FillOp::ProjectionMode {
            layer,
            mode: ProjectionMode::Decal,
        },
    );
    let why = s.decal_problem(layer).expect("マップが無い");
    assert!(why.contains("Position"), "{why}");
    s.lang = Lang::En;
    assert_eq!(s.decal_problem(layer).unwrap(), "No Position map");
    let why = s
        .fill_image_problem(layer, Channel::Color)
        .expect("画像も投影できない");
    assert_eq!(why, "No Position map");
}

// ───────── デカール ─────────

#[test]
fn headless_dropping_an_image_on_the_model_places_a_decal_layer_in_one_undo() {
    let mut s = cube();
    let (_, image) = shelf_image(&mut s, "四色");
    let base = new_fill(&mut s);
    let layers = s.doc.layers().len();
    let steps = s.doc.undo_count();
    let center = pos2(400.0, 300.0);
    fill(
        &mut s,
        FillOp::PlaceDecal {
            image,
            at: center,
            rect: view_rect(),
        },
    );
    assert_eq!(s.doc.layers().len(), layers + 1);
    assert_eq!(
        s.doc.undo_count(),
        steps + 1,
        "層・画像・投影が 1 回の Undo"
    );
    let id = s.selected_layer.unwrap();
    assert_ne!(id, base);
    let layer = s.doc.layer(id).unwrap();
    assert_eq!(layer.name(), "四色");
    assert_eq!(layer.fill_image(Channel::Color), Some(image));
    let p = *layer.projection();
    assert_eq!((p.mode, p.wrap), (ProjectionMode::Decal, Wrap::None));
    // 当たった面（画面の中心の面）の上に、画像の縦横比（正方形）で置く
    assert!(p.placement.size[0] > 0.01 && (p.placement.size[0] - p.placement.size[1]).abs() < 1e-6);
    assert!((p.placement.size[2] - p.placement.size[0] * 0.5).abs() < 1e-9);
    let dist = Vec3::new(
        p.placement.center[0] as f32,
        p.placement.center[1] as f32,
        p.placement.center[2] as f32,
    );
    assert!(
        dist.abs().max_element() <= 0.5001,
        "立方体の表面の上: {dist:?}"
    );
    assert!(s.fillfx.edit_gradient.is_none() && !s.fillfx.handles_hidden);
    assert!(s.message.contains("四色"), "{}", s.message);
    // 出る: 面の上に画像の色が出て、箱の外は透明のまま
    let shown = composite(&s.doc)
        .iter()
        .filter(|c| c[3] > 0 && *c != &[255, 255, 255, 255])
        .count();
    assert!(shown > 0, "デカールの色が出る");
    // 1 回の Undo で層ごと戻る
    assert!(s.doc.undo().unwrap());
    assert_eq!(s.doc.layers().len(), layers);
    assert!(s.doc.layer(id).is_none());
    s.selected_layer = Some(base);
    // モデルの外は断る（何も変えない）
    let steps = s.doc.undo_count();
    fill(
        &mut s,
        FillOp::PlaceDecal {
            image,
            at: pos2(2.0, 2.0),
            rect: view_rect(),
        },
    );
    assert_eq!(s.doc.undo_count(), steps);
    assert!(
        s.message.contains("モデルの上ではありません"),
        "{}",
        s.message
    );
    s.lang = Lang::En;
    fill(
        &mut s,
        FillOp::PlaceDecal {
            image,
            at: pos2(2.0, 2.0),
            rect: view_rect(),
        },
    );
    assert_eq!(s.message, "Not on the model");
    // 棚に無い画像・モデルの無い状態も理由を言う
    s.lang = Lang::Ja;
    fill(
        &mut s,
        FillOp::PlaceDecal {
            image: ImageId(7),
            at: center,
            rect: view_rect(),
        },
    );
    assert!(s.message.contains("棚"), "{}", s.message);
    let mut bare = AppState::new(64, 64);
    let (_, im) = shelf_image(&mut bare, "四色");
    fill(
        &mut bare,
        FillOp::PlaceDecal {
            image: im,
            at: center,
            rect: view_rect(),
        },
    );
    assert!(bare.message.contains("モデル"), "{}", bare.message);
    // ロックされた層の上へも置ける（新しい層を足すだけ）。読むだけのセットは断る
    s.sets.get_mut(0).unwrap().read_only = Some("理由".into());
    let layers = s.doc.layers().len();
    s.apply(Action::Fill(FillOp::PlaceDecal {
        image,
        at: center,
        rect: view_rect(),
    }));
    assert_eq!(s.doc.layers().len(), layers);
}

#[test]
fn headless_a_decal_without_baked_maps_is_placed_with_the_reason() {
    let mut s = AppState::new(64, 64);
    s.apply(Action::LoadDemoModel);
    let (_, image) = shelf_image(&mut s, "四色");
    fill(
        &mut s,
        FillOp::PlaceDecal {
            image,
            at: pos2(400.0, 300.0),
            rect: view_rect(),
        },
    );
    let id = s.selected_layer.unwrap();
    assert_eq!(layer_projection(&s, id).mode, ProjectionMode::Decal);
    assert!(
        s.message.contains("まだ出ません") && s.message.contains("Position"),
        "{}",
        s.message
    );
}

// ───────── 形のギズモ ─────────

fn planar_cube() -> (AppState, LayerId) {
    let mut s = cube();
    let (_, image) = shelf_image(&mut s, "四色");
    let layer = new_fill(&mut s);
    fill(
        &mut s,
        FillOp::Image {
            layer,
            channel: Channel::Color,
            image: Some(image),
        },
    );
    fill(
        &mut s,
        FillOp::ProjectionMode {
            layer,
            mode: ProjectionMode::Planar,
        },
    );
    s.view3d.camera.yaw = -40.0;
    s.view3d.camera.pitch = 15.0;
    (s, layer)
}

#[test]
fn headless_dragging_a_gizmo_handle_edits_the_projection_in_one_undo_and_escape_puts_it_back() {
    let (mut s, layer) = planar_cube();
    let rect = view_rect();
    assert_eq!(gizmo::target(&s), Some(gizmo::Target::Projection(layer)));
    let start = layer_projection(&s, layer);
    let from = gizmo::handle_point(&s, rect, Handle::MoveX).expect("X の矢印");
    assert_eq!(gizmo::handle_at(&s, rect, from), Handle::MoveX);
    let steps = s.doc.undo_count();
    assert!(gizmo::press(&mut s, rect, from, gizmo::Source::Mouse));
    // 続けて動かす（途中の位置を経由しても、始まりから計算するので同じ）
    gizmo::drag_to(&mut s, rect, from + vec2(20.0, 0.0), false, false);
    gizmo::drag_to(&mut s, rect, from + vec2(40.0, 0.0), false, false);
    gizmo::drag_to(&mut s, rect, from + vec2(60.0, 0.0), false, false);
    let moved = layer_projection(&s, layer);
    assert_ne!(moved.placement.center, start.placement.center);
    assert!(
        moved.placement.center[1].abs() < 1e-6 && moved.placement.center[2].abs() < 1e-6,
        "X の軸だけ動く"
    );
    gizmo::release(&mut s, true);
    assert_eq!(s.doc.undo_count(), steps + 1, "1 回の Undo");
    assert!(s.modified);
    assert!(s.doc.undo().unwrap());
    assert_eq!(layer_projection(&s, layer), start);
    assert!(s.doc.redo().unwrap());
    assert_eq!(layer_projection(&s, layer), moved);
    assert!(s.doc.undo().unwrap());
    // Esc（フォーカスの喪失も同じ）: ドラッグの前へ戻し、履歴にも残さない
    let steps = s.doc.undo_count();
    assert!(gizmo::press(&mut s, rect, from, gizmo::Source::Mouse));
    gizmo::drag_to(&mut s, rect, from + vec2(50.0, 0.0), false, false);
    assert_ne!(layer_projection(&s, layer), start);
    gizmo::release(&mut s, false);
    assert_eq!(layer_projection(&s, layer), start);
    assert_eq!(s.doc.undo_count(), steps, "履歴に残さない");
    assert!(s.message.contains("形の操作をやめました"), "{}", s.message);
    assert!(!gizmo::dragging(&s));
    // ハンドルの無い所の押下は受け取らない（今のツールへ）
    assert!(!gizmo::press(
        &mut s,
        rect,
        pos2(3.0, 3.0),
        gizmo::Source::Mouse
    ));
    // 回す輪・大きさのつまみ
    s.apply(Action::Fill(FillOp::GizmoMode(Mode::Rotate)));
    let ring = gizmo::handle_point(&s, rect, Handle::RotateY).expect("Y の輪");
    assert!(gizmo::press(&mut s, rect, ring, gizmo::Source::Mouse));
    gizmo::drag_to(&mut s, rect, ring + vec2(30.0, 10.0), false, false);
    gizmo::release(&mut s, true);
    assert_ne!(
        layer_projection(&s, layer).placement.rotation,
        start.placement.rotation
    );
    assert!(s.doc.undo().unwrap());
    let knob = gizmo::handle_point(&s, rect, Handle::SizeXPos).expect("X の面のつまみ");
    assert!(gizmo::press(&mut s, rect, knob, gizmo::Source::Mouse));
    gizmo::drag_to(&mut s, rect, knob + vec2(25.0, 0.0), false, false);
    gizmo::release(&mut s, true);
    let sized = layer_projection(&s, layer);
    assert_ne!(sized.placement.size[0], start.placement.size[0]);
    assert_eq!(sized.placement.size[1], start.placement.size[1]);
}

#[test]
fn headless_the_gizmo_hides_with_q_and_the_gradient_being_edited_comes_first() {
    let (mut s, layer) = planar_cube();
    let rect = view_rect();
    assert!(gizmo::target(&s).is_some());
    // Q（隠す・出す）
    s.apply(Action::Fill(FillOp::ToggleHandles));
    assert!(s.fillfx.handles_hidden && gizmo::target(&s).is_none());
    let any = pos2(400.0, 300.0);
    assert!(!gizmo::press(&mut s, rect, any, gizmo::Source::Mouse));
    s.apply(Action::Fill(FillOp::ToggleHandles));
    assert_eq!(gizmo::target(&s), Some(gizmo::Target::Projection(layer)));
    // マスクを編集している間は出さない
    s.apply(Action::M2(Edit::AddMask(layer)));
    assert!(s.m2.edit_mask && gizmo::target(&s).is_none());
    s.m2.edit_mask = false;
    // グラデーションを足して 3D で編集すると、そちらが先
    fill(
        &mut s,
        FillOp::AddGradient {
            layer,
            channel: Channel::Roughness,
        },
    );
    assert_eq!(
        gizmo::target(&s),
        Some(gizmo::Target::Gradient(layer, Channel::Roughness))
    );
    // やめると投影の置き場へ戻る
    s.apply(Action::Fill(FillOp::EditGradient(None)));
    assert_eq!(gizmo::target(&s), Some(gizmo::Target::Projection(layer)));
    // UV の投影には置き場が無い
    fill(
        &mut s,
        FillOp::ProjectionMode {
            layer,
            mode: ProjectionMode::Uv,
        },
    );
    assert!(gizmo::target(&s).is_none());
    // 塗りつぶしでない層・モデルの無い状態でも出ない
    s.apply(Action::NewLayer);
    assert!(gizmo::target(&s).is_none());
}

#[test]
fn headless_a_locked_layer_refuses_the_gizmo_and_a_hidden_handle_cancels_a_running_drag() {
    let (mut s, layer) = planar_cube();
    let rect = view_rect();
    let from = gizmo::handle_point(&s, rect, Handle::MoveX).unwrap();
    // ロックされた層: ハンドルを押しても何も変わらず、理由を言う（描き始めない = 押下は受け取る）
    s.doc.set_layer_locks(layer, LayerLocks::ALL).unwrap();
    let start = layer_projection(&s, layer);
    let steps = s.doc.undo_count();
    assert!(gizmo::press(&mut s, rect, from, gizmo::Source::Mouse));
    gizmo::drag_to(&mut s, rect, from + vec2(40.0, 0.0), false, false);
    assert_eq!(layer_projection(&s, layer), start);
    assert_eq!(s.doc.undo_count(), steps);
    assert!(!s.message.is_empty());
    assert!(!gizmo::dragging(&s), "断られたドラッグは取り残さない");
    s.doc.set_layer_locks(layer, LayerLocks::NONE).unwrap();
    // 読むだけのセット: 押下は受け取り、理由を言う
    s.sets.get_mut(0).unwrap().read_only = Some("理由".into());
    assert!(gizmo::press(&mut s, rect, from, gizmo::Source::Mouse));
    assert!(!gizmo::dragging(&s));
    s.sets.get_mut(0).unwrap().read_only = None;
    // ドラッグの途中でハンドルを隠すと、前へ戻す
    assert!(gizmo::press(&mut s, rect, from, gizmo::Source::Mouse));
    gizmo::drag_to(&mut s, rect, from + vec2(40.0, 0.0), false, false);
    assert_ne!(layer_projection(&s, layer), start);
    s.apply(Action::Fill(FillOp::Handles(true)));
    assert_eq!(layer_projection(&s, layer), start);
    assert!(!gizmo::dragging(&s));
    // 描いている最中は始めない
    s.apply(Action::Fill(FillOp::Handles(false)));
    let brush = s.stroke_settings(false);
    let other = s.doc.layers()[0].id();
    let stroke = s.doc.begin_stroke(other, &brush).unwrap();
    assert!(!gizmo::press(&mut s, rect, from, gizmo::Source::Mouse));
    s.doc.end_stroke(stroke).unwrap();
}

// ───────── ワールドスペースのグラデーション ─────────

#[test]
fn headless_a_world_space_gradient_is_added_fitted_to_the_model_and_every_edit_is_one_undo() {
    let mut s = cube();
    let layer = new_fill(&mut s);
    let plain = composite(&s.doc);
    let steps = s.doc.undo_count();
    fill(
        &mut s,
        FillOp::AddGradient {
            layer,
            channel: Channel::Color,
        },
    );
    assert_eq!(s.doc.undo_count(), steps + 1);
    let g = s
        .doc
        .layer(layer)
        .unwrap()
        .fill_gradient(Channel::Color)
        .unwrap()
        .clone();
    assert_eq!(g.kind, Kind::ShapeGradient);
    assert_eq!(g.blend, Blend::Replace);
    assert!(g.ramp.is_some());
    assert!(
        (g.volume.size[1] - 0.5).abs() < 1e-6,
        "高さは外形の半分の箱で始める: {:?}",
        g.volume
    );
    assert!((g.volume.size[0] - 1.05).abs() < 1e-5);
    assert_eq!(s.fillfx.edit_gradient, Some((layer, Channel::Color)));
    // 値が出る: 立方体の面に色の階調が敷かれる（全面が同じ色ではない）
    let c = composite(&s.doc);
    assert_ne!(c, plain, "値が出る（塗りつぶしの値だけの絵と違う）");
    assert!(
        c.iter().any(|p| p[3] > 0 && p != &[255, 255, 255, 255]),
        "階調の色が出る"
    );
    // 値のドラッグ（置き場・減衰）は 1 回にまとまる
    let steps = s.doc.undo_count();
    for k in 1..=4 {
        let mut next = g.clone();
        next.volume.center = [0.0, 0.05 * f64::from(k), 0.0];
        next.volume.falloff = 0.1 * f64::from(k);
        fill(
            &mut s,
            FillOp::Gradient {
                layer,
                channel: Channel::Color,
                gradient: Some(Box::new(next)),
                coalesce: true,
            },
        );
    }
    s.m2_end_drag();
    assert_eq!(s.doc.undo_count(), steps + 1);
    assert!(s.doc.undo().unwrap());
    assert_eq!(
        s.doc
            .layer(layer)
            .unwrap()
            .fill_gradient(Channel::Color)
            .unwrap()
            .volume,
        g.volume
    );
    // 形を替える（球・平面）
    for shape in [Shape::Sphere, Shape::Plane, Shape::Box] {
        let mut next = g.clone();
        next.volume.shape = shape;
        let steps = s.doc.undo_count();
        fill(
            &mut s,
            FillOp::Gradient {
                layer,
                channel: Channel::Color,
                gradient: Some(Box::new(next)),
                coalesce: false,
            },
        );
        assert_eq!(s.doc.undo_count(), steps + 1, "{shape:?}");
        assert_eq!(
            s.doc
                .layer(layer)
                .unwrap()
                .fill_gradient(Channel::Color)
                .unwrap()
                .volume
                .shape,
            shape
        );
    }
    // 反転
    let mut inv = g.clone();
    inv.invert = true;
    let before = composite(&s.doc);
    fill(
        &mut s,
        FillOp::Gradient {
            layer,
            channel: Channel::Color,
            gradient: Some(Box::new(inv)),
            coalesce: false,
        },
    );
    assert_ne!(composite(&s.doc), before);
    // 外す: 値に戻り、3D での編集もやめる。法線には足せない
    fill(
        &mut s,
        FillOp::Gradient {
            layer,
            channel: Channel::Color,
            gradient: None,
            coalesce: false,
        },
    );
    assert!(s
        .doc
        .layer(layer)
        .unwrap()
        .fill_gradient(Channel::Color)
        .is_none());
    assert!(s.fillfx.edit_gradient.is_none());
    s.message.clear();
    fill(
        &mut s,
        FillOp::AddGradient {
            layer,
            channel: Channel::Normal,
        },
    );
    assert!(!s.message.is_empty());
    assert!(s
        .doc
        .layer(layer)
        .unwrap()
        .fill_gradient(Channel::Normal)
        .is_none());
}

#[test]
fn headless_the_gizmo_edits_a_gradients_shape_in_one_undo() {
    let mut s = cube();
    s.view3d.camera.yaw = -40.0;
    s.view3d.camera.pitch = 15.0;
    let rect = view_rect();
    let layer = new_fill(&mut s);
    fill(
        &mut s,
        FillOp::AddGradient {
            layer,
            channel: Channel::Roughness,
        },
    );
    let start = s
        .doc
        .layer(layer)
        .unwrap()
        .fill_gradient(Channel::Roughness)
        .unwrap()
        .volume;
    let from = gizmo::handle_point(&s, rect, Handle::SizeYPos).expect("上の面のつまみ");
    let steps = s.doc.undo_count();
    assert!(gizmo::press(&mut s, rect, from, gizmo::Source::Mouse));
    gizmo::drag_to(&mut s, rect, from + vec2(0.0, -30.0), false, false);
    gizmo::drag_to(&mut s, rect, from + vec2(0.0, -45.0), false, false);
    gizmo::release(&mut s, true);
    assert_eq!(s.doc.undo_count(), steps + 1);
    let after = s
        .doc
        .layer(layer)
        .unwrap()
        .fill_gradient(Channel::Roughness)
        .unwrap()
        .volume;
    assert_ne!(after.size[1], start.size[1]);
    assert_eq!(after.shape, start.shape);
    assert_eq!(after.falloff, start.falloff, "ギズモは置き場だけを変える");
    assert!(s.doc.undo().unwrap());
    assert_eq!(
        s.doc
            .layer(layer)
            .unwrap()
            .fill_gradient(Channel::Roughness)
            .unwrap()
            .volume,
        start
    );
}

#[test]
fn headless_ramp_edits_presets_stops_and_curve_are_one_undo_each_and_keep_the_rest() {
    use yolu_app::ui::ramp::ops;
    let mut s = cube();
    let layer = new_fill(&mut s);
    fill(
        &mut s,
        FillOp::AddGradient {
            layer,
            channel: Channel::Color,
        },
    );
    let g0 = s
        .doc
        .layer(layer)
        .unwrap()
        .fill_gradient(Channel::Color)
        .unwrap()
        .clone();
    let ramp0 = g0.ramp.clone().unwrap();
    let apply = |s: &mut AppState, ramp: yolu_core::generator::Ramp| {
        let mut g = s
            .doc
            .layer(layer)
            .unwrap()
            .fill_gradient(Channel::Color)
            .unwrap()
            .clone();
        g.ramp = Some(ramp);
        let steps = s.doc.undo_count();
        fill(
            s,
            FillOp::Gradient {
                layer,
                channel: Channel::Color,
                gradient: Some(Box::new(g)),
                coalesce: false,
            },
        );
        assert_eq!(s.doc.undo_count(), steps + 1);
    };
    // 分岐点を足す・動かす・消す
    let (added, k) = ops::add_color(&ramp0, 0.5, false).unwrap();
    apply(&mut s, added.clone());
    let moved = ops::move_stop(&added, false, k, 0.7).unwrap();
    apply(&mut s, moved.clone());
    let recolored = {
        let mut list = moved.colors().to_vec();
        list[k].color = Rgba8::new(10, 200, 30, 255);
        ops::with_colors(&moved, list).unwrap()
    };
    apply(&mut s, recolored.clone());
    let (op_added, _) = ops::add_opacity(&recolored, 0.3).unwrap();
    apply(&mut s, op_added.clone());
    let removed = ops::remove(&op_added, false, k).unwrap();
    apply(&mut s, removed.clone());
    // カーブのプリセットは階調を変えず、保存された設定にも残る
    let curved = ops::curve_preset(&removed, 3).unwrap();
    apply(&mut s, curved.clone());
    let now = s
        .doc
        .layer(layer)
        .unwrap()
        .fill_gradient(Channel::Color)
        .unwrap();
    assert_eq!(now.ramp.as_ref().unwrap().colors(), removed.colors());
    assert_eq!(now.ramp.as_ref().unwrap().curve().len(), 4);
    assert_eq!(now.volume, g0.volume, "階調の編集は置き場に触らない");
    // 1 つずつ Undo で戻る
    for expect in [&removed, &op_added, &recolored, &moved, &added, &ramp0] {
        assert!(s.doc.undo().unwrap());
        let r = s
            .doc
            .layer(layer)
            .unwrap()
            .fill_gradient(Channel::Color)
            .unwrap()
            .ramp
            .clone()
            .unwrap();
        assert_eq!(&r, expect);
    }
    // 階調のプリセット（サブの色も使う）
    s.color.set_main([1.0, 0.0, 0.0, 1.0]);
    s.color.sub = [0.0, 0.0, 1.0, 1.0];
    let entries = yolu_app::m2_menu::entries(
        &s,
        yolu_app::m2_menu::Popup::RampPresets(layer, Channel::Color),
    );
    assert_eq!(entries.len(), 5);
    let fg_bg = entries
        .into_iter()
        .filter_map(|e| match e {
            yolu_app::ui::menu::Entry::Item { action, .. } => Some(action),
            _ => None,
        })
        .nth(2)
        .unwrap();
    s.apply(fg_bg);
    let ramp = s
        .doc
        .layer(layer)
        .unwrap()
        .fill_gradient(Channel::Color)
        .unwrap()
        .ramp
        .clone()
        .unwrap();
    assert_eq!(ramp.colors()[0].color, Rgba8::new(255, 0, 0, 255));
    assert_eq!(ramp.colors()[1].color, Rgba8::new(0, 0, 255, 255));
    // データのチャンネルでは値（輝度）の階調で、色は灰色になる
    fill(
        &mut s,
        FillOp::AddGradient {
            layer,
            channel: Channel::Roughness,
        },
    );
    assert!(s
        .doc
        .layer(layer)
        .unwrap()
        .fill_gradient(Channel::Roughness)
        .is_some());
}

// ───────── 保存と復元 ─────────

#[test]
fn headless_images_projections_gradients_and_decals_survive_saving_and_opening() {
    let dir = temp_dir("save");
    let path = dir.join("fx.ylp");
    let mut s = cube();
    let (rid, image) = shelf_image(&mut s, "四色");
    let a = new_fill(&mut s);
    fill(
        &mut s,
        FillOp::Image {
            layer: a,
            channel: Channel::Color,
            image: Some(image),
        },
    );
    fill(
        &mut s,
        FillOp::ProjectionMode {
            layer: a,
            mode: ProjectionMode::Triplanar,
        },
    );
    let mut p = layer_projection(&s, a);
    p.tiles = [2.0, 3.0];
    p.offset = [0.25, -0.5];
    p.rotation = 30.0;
    p.blend_width = 0.6;
    fill(
        &mut s,
        FillOp::Projection {
            layer: a,
            projection: Box::new(p),
            coalesce: false,
        },
    );
    let b = new_fill(&mut s);
    fill(
        &mut s,
        FillOp::AddGradient {
            layer: b,
            channel: Channel::Roughness,
        },
    );
    let mut g = s
        .doc
        .layer(b)
        .unwrap()
        .fill_gradient(Channel::Roughness)
        .unwrap()
        .clone();
    g.volume.shape = Shape::Sphere;
    g.volume.center = [0.1, 0.2, -0.3];
    g.volume.rotation = [10.0, 20.0, 30.0];
    g.invert = true;
    fill(
        &mut s,
        FillOp::Gradient {
            layer: b,
            channel: Channel::Roughness,
            gradient: Some(Box::new(g.clone())),
            coalesce: false,
        },
    );
    fill(
        &mut s,
        FillOp::PlaceDecal {
            image,
            at: pos2(400.0, 300.0),
            rect: view_rect(),
        },
    );
    let c = s.selected_layer.unwrap();
    let decal = layer_projection(&s, c);
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    // 保存した正本を core の文書へ戻して、同じ並びの層が同じ設定を持つ（画像の参照・投影・グラデーション・デカール）。
    // 開く道（アプリ）は、画像・マップを文書へ渡す前の文書を「効かない効果がある」として読むだけにするので、ここでは正本を直に戻す
    let bytes = std::fs::read(&path).unwrap();
    let project = yolu_io::Project::read(&bytes).expect("読める");
    let restored = project.sets()[0]
        .document
        .to_core()
        .expect("core の文書にできる");
    let order = |id: LayerId| s.doc.layers().iter().position(|l| l.id() == id).unwrap();
    let (ra, rb, rc) = (
        restored.layers()[order(a)].clone(),
        restored.layers()[order(b)].clone(),
        restored.layers()[order(c)].clone(),
    );
    assert_eq!(ra.fill_image(Channel::Color), Some(image));
    assert_eq!(*ra.projection(), p);
    assert_eq!(ra.fill_gradient(Channel::Color), None);
    assert_eq!(rb.fill_gradient(Channel::Roughness), Some(&g));
    assert_eq!(rc.fill_image(Channel::Color), Some(image));
    assert_eq!(*rc.projection(), decal);
    assert_eq!(rc.name(), "四色");
    assert!(
        project
            .resources()
            .iter()
            .any(|r| r.id == rid && r.kind == "image"),
        "画像は .ylp の棚に入っている"
    );
    // 画像の画素も棚から戻る（同じ中身の鍵）
    let images = project.image_inputs().expect("画像を読める");
    assert!(images
        .iter()
        .any(|(id, i)| *id == image && i.hash == s.doc.effect_inputs().image(image).unwrap().hash));
    let _ = std::fs::remove_dir_all(dir);
}

// ───────── 棚への取り込み ─────────

#[test]
fn headless_importing_a_png_adds_one_shelf_image_and_refuses_what_it_cannot_read() {
    let dir = temp_dir("import");
    let path = dir.join("tile.png");
    image::RgbaImage::from_raw(2, 2, quad_image())
        .unwrap()
        .save(&path)
        .unwrap();
    let mut s = AppState::new(32, 32);
    let before = s.shelf.resources().len();
    fill(&mut s, FillOp::ImportImage(path.clone()));
    assert_eq!(s.shelf.resources().len(), before + 1);
    let r = s
        .shelf
        .resources()
        .iter()
        .find(|r| r.name == "tile")
        .expect("名前はファイル名")
        .clone();
    assert_eq!(r.kind, "image");
    assert_eq!(
        (r.metadata["width"].as_u64(), r.metadata["height"].as_u64()),
        (Some(2), Some(2))
    );
    assert!(
        s.shelf.changed && s.modified && s.message.contains("tile"),
        "{}",
        s.message
    );
    // 同じ中身は足さない
    fill(&mut s, FillOp::ImportImage(path.clone()));
    assert_eq!(s.shelf.resources().len(), before + 1);
    // 読み込んだ画像を層へ差せる
    let id = inputs::image_id(&r.id).unwrap();
    let layer = new_fill(&mut s);
    fill(
        &mut s,
        FillOp::Image {
            layer,
            channel: Channel::Color,
            image: Some(id),
        },
    );
    assert_eq!(
        s.doc.layer(layer).unwrap().fill_image(Channel::Color),
        Some(id)
    );
    // PNG でないもの・無いファイルは理由を言って棚を変えない
    let junk = dir.join("junk.png");
    std::fs::write(&junk, b"not a png").unwrap();
    fill(&mut s, FillOp::ImportImage(junk));
    assert!(s.message.contains("PNG"), "{}", s.message);
    fill(&mut s, FillOp::ImportImage(dir.join("missing.png")));
    assert!(s.message.contains("missing"), "{}", s.message);
    assert_eq!(s.shelf.resources().len(), before + 1);
    s.lang = Lang::En;
    fill(&mut s, FillOp::ImportImage(dir.join("junk.png")));
    assert!(s.message.contains("Not a readable PNG"), "{}", s.message);
    // 窓を開く頼み
    fill(&mut s, FillOp::ImportImageDialog);
    assert_eq!(
        s.dialog_request,
        Some(yolu_app::state::DialogRequest::FillImage)
    );
    let _ = std::fs::remove_dir_all(dir);
}

// ───────── 棚への取り込みの断り ─────────

/// 左下のタイルの画布を単色にした 1 層の文書（棚へ「層を保存」できる中身）。
fn painted(size: u32) -> (AppState, LayerId) {
    let mut s = AppState::new(size, size);
    let id = s.selected_layer.unwrap();
    let ts = s.doc.tile_size() as usize;
    let mut bytes = vec![0u8; ts * ts * 4];
    for y in 0..size as usize {
        for x in 0..size as usize {
            bytes[(y * ts + x) * 4..][..4].copy_from_slice(&[200, 40, 30, 255]);
        }
    }
    s.doc
        .import_tile(id, Channel::Color, TileCoord::new(0, 0), &bytes)
        .unwrap();
    (s, id)
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend(data);
    out.extend(&body);
    out.extend(crc32(&body).to_be_bytes());
}

/// 寸法だけが本物で、画素を持たない（IDAT は空の流れ）PNG。寸法を先に見て断るなら、画素の無さには気づかない。
fn png_header_only(width: u32, height: u32) -> Vec<u8> {
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend(width.to_be_bytes());
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    png_chunk(&mut out, b"IHDR", &ihdr);
    png_chunk(
        &mut out,
        b"IDAT",
        &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
    );
    png_chunk(&mut out, b"IEND", &[]);
    out
}

#[test]
fn headless_importing_or_changing_the_reading_while_a_shelf_save_runs_is_refused_and_changes_nothing(
) {
    let dir = temp_dir("saving");
    let path = dir.join("tile.png");
    image::RgbaImage::from_raw(2, 2, quad_image())
        .unwrap()
        .save(&path)
        .unwrap();
    let (mut s, base) = painted(16);
    let (rid, image) = shelf_image(&mut s, "四色");
    let space = |s: &AppState| inputs::space_of(s.shelf.get(&rid).unwrap()).0;
    assert_eq!(space(&s), "srgb");
    s.shelf.async_bytes = 0; // 小さな素材でも別のスレッドで保存する
    s.shelf.hold_saves(true);
    s.apply(Action::Shelf(ShelfOp::SaveMaterial(base)));
    assert!(s.shelf.saving_name().is_some(), "{}", s.message);
    let resources = s.shelf.resources().len();
    let revision = s.doc.revision();
    s.modified = false;
    // 取り込み・読み方・取り込みの窓は、保存が終わるまで断る（保存の結果は足した後の棚で丸ごと差し替えるので、
    // その間に変えると変えた分が黙って消える）
    fill(&mut s, FillOp::ImportImage(path.clone()));
    assert!(s.message.contains("保存中です"), "{}", s.message);
    fill(
        &mut s,
        FillOp::ImageColorSpace {
            image,
            space: yolu_core::ImageColorSpace::Linear,
        },
    );
    assert!(s.message.contains("保存中です"), "{}", s.message);
    s.message.clear();
    fill(&mut s, FillOp::ImportImageDialog);
    assert!(s.message.contains("保存中です"), "{}", s.message);
    assert!(s.dialog_request.is_none(), "窓も開かない");
    assert_eq!(s.shelf.resources().len(), resources, "棚は変わらない");
    assert_eq!(space(&s), "srgb", "読み方も変わらない");
    assert!(!s.modified, "文書は変わらない扱いのまま");
    assert_eq!(s.doc.revision(), revision);
    s.lang = Lang::En;
    fill(&mut s, FillOp::ImportImage(path.clone()));
    assert!(s.message.contains("Already saving"), "{}", s.message);
    s.lang = Lang::Ja;
    // 保存が終われば、足した分は残り（取り込みは消えていない）、取り込み・読み方も受けられる
    s.shelf.hold_saves(false);
    s.shelf_wait();
    assert_eq!(s.shelf.resources().len(), resources + 1, "{}", s.message);
    fill(&mut s, FillOp::ImportImage(path.clone()));
    assert_eq!(s.shelf.resources().len(), resources + 2, "{}", s.message);
    fill(
        &mut s,
        FillOp::ImageColorSpace {
            image,
            space: yolu_core::ImageColorSpace::Linear,
        },
    );
    assert_eq!(space(&s), "linear");
    assert!(s.modified && s.shelf.changed);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_import_looks_at_the_size_before_the_pixels_and_refuses_with_the_real_reason() {
    let dir = temp_dir("size");
    let mut s = AppState::new(32, 32);
    // 本物の PNG の 1 辺 8192 は取り込め、8193 は断る
    let ok = dir.join("wide.png");
    image::RgbaImage::new(8192, 1).save(&ok).unwrap();
    fill(&mut s, FillOp::ImportImage(ok));
    assert_eq!(s.shelf.resources().len(), 1, "{}", s.message);
    let tall = dir.join("tall.png");
    image::RgbaImage::new(1, 8192).save(&tall).unwrap();
    fill(&mut s, FillOp::ImportImage(tall));
    assert_eq!(s.shelf.resources().len(), 2, "{}", s.message);
    s.modified = false;
    let before = s.shelf.resources().len();
    let over = dir.join("over.png");
    image::RgbaImage::new(8193, 1).save(&over).unwrap();
    fill(&mut s, FillOp::ImportImage(over));
    assert!(s.message.contains("1〜8192"), "{}", s.message);
    // 9000 × 9000 で画素を持たない PNG: 画素を読んでいたら「PNG として読めません」になる。寸法を先に見るので、本当の理由で断る
    let huge = dir.join("huge.png");
    std::fs::write(&huge, png_header_only(9000, 9000)).unwrap();
    fill(&mut s, FillOp::ImportImage(huge.clone()));
    assert!(s.message.contains("1〜8192"), "{}", s.message);
    assert!(!s.message.contains("PNG として"), "{}", s.message);
    s.lang = Lang::En;
    fill(&mut s, FillOp::ImportImage(huge));
    assert!(s.message.contains("1 to 8192"), "{}", s.message);
    assert_eq!(s.shelf.resources().len(), before, "棚は変わらない");
    assert!(!s.modified);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_importing_what_the_shelf_already_has_says_so_and_changes_nothing() {
    let dir = temp_dir("again");
    let path = dir.join("tile.png");
    image::RgbaImage::from_raw(2, 2, quad_image())
        .unwrap()
        .save(&path)
        .unwrap();
    let mut s = AppState::new(32, 32);
    fill(&mut s, FillOp::ImportImage(path.clone()));
    assert!(s.message.contains("取り込みました"), "{}", s.message);
    let id = s.shelf.selected.clone().unwrap();
    s.modified = false;
    s.shelf.selected = None;
    fill(&mut s, FillOp::ImportImage(path.clone()));
    assert!(s.message.contains("すでに棚にあります"), "{}", s.message);
    assert!(!s.modified, "同じ中身なら、棚も文書も変えた扱いにしない");
    assert_eq!(s.shelf.resources().len(), 1);
    assert_eq!(
        s.shelf.selected.as_deref(),
        Some(id.as_str()),
        "その画像を選ぶ"
    );
    s.lang = Lang::En;
    fill(&mut s, FillOp::ImportImage(path));
    assert_eq!(s.message, "Already on the shelf: tile");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_import_into_a_full_or_unreadable_shelf_is_refused_with_the_reason_and_changes_nothing()
{
    let dir = temp_dir("full");
    let path = dir.join("tile.png");
    image::RgbaImage::from_raw(2, 2, quad_image())
        .unwrap()
        .save(&path)
        .unwrap();
    let mut s = AppState::new(32, 32);
    for i in 0..yolu_io::shelf::MAX_RESOURCES {
        let rgba = [(i % 256) as u8, (i / 256) as u8, 9, 255];
        s.shelf
            .add_image(Lang::Ja, &format!("c{i}"), &rgba, 1, 1)
            .expect("棚へ入る");
    }
    assert_eq!(s.shelf.resources().len(), yolu_io::shelf::MAX_RESOURCES);
    s.modified = false;
    fill(&mut s, FillOp::ImportImage(path.clone()));
    assert!(s.message.contains("棚がいっぱい"), "{}", s.message);
    assert_eq!(s.shelf.resources().len(), yolu_io::shelf::MAX_RESOURCES);
    assert!(!s.modified, "{}", s.message);
    s.lang = Lang::En;
    fill(&mut s, FillOp::ImportImage(path.clone()));
    assert!(s.message.contains("Shelf is full"), "{}", s.message);
    // 読めなかった棚には足さない
    let mut t = AppState::new(32, 32);
    t.shelf = yolu_app::shelf::ShelfState::unreadable("棚が壊れています");
    fill(&mut t, FillOp::ImportImage(path));
    assert!(t.message.contains("棚が壊れています"), "{}", t.message);
    assert!(t.shelf.resources().is_empty() && !t.modified);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_an_image_over_the_core_budget_or_a_layer_that_is_not_a_fill_is_refused_and_leaves_the_layer(
) {
    let mut s = AppState::new(32, 32);
    let (_, image) = shelf_image(&mut s, "四色");
    let layer = new_fill(&mut s);
    let paint = s
        .doc
        .layers()
        .iter()
        .find(|l| l.kind() != yolu_app::engine::LayerKind::Fill)
        .unwrap()
        .id();
    // core の画像の予算（ミップマップを持つ量）に収まらない画像は、層へ差さない
    let steps = s.doc.undo_count();
    s.doc.set_fill_image_cache_budget_bytes(3); // 2 × 2 のミップマップは 4 バイト要る
    fill(
        &mut s,
        FillOp::Image {
            layer,
            channel: Channel::Color,
            image: Some(image),
        },
    );
    assert!(
        s.message.contains("作業のメモリを超えます"),
        "{}",
        s.message
    );
    assert_eq!(s.doc.layer(layer).unwrap().fill_image(Channel::Color), None);
    assert_eq!(s.doc.undo_count(), steps, "Undo の段を足さない");
    assert_eq!(s.fx.inputs.decoded_image_count(), 0, "差さなかった画像は復号したまま残さない");
    s.lang = Lang::En;
    fill(
        &mut s,
        FillOp::Image {
            layer,
            channel: Channel::Color,
            image: Some(image),
        },
    );
    assert!(
        s.message.contains("Over the working memory"),
        "{}",
        s.message
    );
    // 予算が戻れば同じ操作が通る（断りの理由が予算だったこと）
    s.doc.set_fill_image_cache_budget_bytes(256 * 1024 * 1024);
    fill(
        &mut s,
        FillOp::Image {
            layer,
            channel: Channel::Color,
            image: Some(image),
        },
    );
    assert_eq!(
        s.doc.layer(layer).unwrap().fill_image(Channel::Color),
        Some(image),
        "{}",
        s.message
    );
    // 塗りつぶしでない層には差せない
    let steps = s.doc.undo_count();
    s.lang = Lang::Ja;
    fill(
        &mut s,
        FillOp::Image {
            layer: paint,
            channel: Channel::Color,
            image: Some(image),
        },
    );
    assert!(
        s.message.contains("塗りつぶしのレイヤーではありません"),
        "{}",
        s.message
    );
    assert_eq!(s.doc.undo_count(), steps);
    assert_eq!(s.doc.layer(paint).unwrap().fill_image(Channel::Color), None);
    s.lang = Lang::En;
    fill(
        &mut s,
        FillOp::Image {
            layer: paint,
            channel: Channel::Color,
            image: None,
        },
    );
    assert_eq!(s.message, "Not a fill layer");
}

#[test]
fn headless_an_image_that_is_refused_is_not_left_decoded_in_the_shared_budget() {
    let mut s = cube();
    let (_, image_a) = shelf_image(&mut s, "四色");
    let rid_b = s
        .shelf
        .add_image(Lang::Ja, "別の画像", &[[7u8, 8, 9, 255]; 4].concat(), 2, 2)
        .unwrap();
    let image_b = inputs::image_id(&rid_b).unwrap();
    let layer = new_fill(&mut s);
    fill(&mut s, FillOp::Image { layer, channel: Channel::Color, image: Some(image_a) });
    assert_eq!(s.doc.layer(layer).unwrap().fill_image(Channel::Color), Some(image_a));
    let (count, bytes) = (s.fx.inputs.decoded_image_count(), s.fx.inputs.decoded_image_bytes());
    assert_eq!(count, 1);
    // ロック中の層への差し替え: 断られ、差そうとした画像は文書にも予算にも残らない（フレームを回しても増えない）
    s.doc.set_layer_locks(layer, LayerLocks::ALL).unwrap();
    let steps = s.doc.undo_count();
    s.message.clear();
    fill(&mut s, FillOp::Image { layer, channel: Channel::Color, image: Some(image_b) });
    assert!(s.message.contains("ロック"), "{}", s.message);
    assert_eq!(s.doc.undo_count(), steps);
    assert_eq!(s.doc.layer(layer).unwrap().fill_image(Channel::Color), Some(image_a));
    for _ in 0..3 {
        s.sync_effects();
        assert_eq!(s.fx.inputs.decoded_image_count(), count);
        assert_eq!(s.fx.inputs.decoded_image_bytes(), bytes);
        assert!(s.doc.effect_inputs().image(image_b).is_none(), "文書の入力にも残さない");
    }
    // 断られた画像が別の層の指している画像でもあるなら、その層のためにそのまま持つ
    let other = new_fill(&mut s);
    fill(&mut s, FillOp::Image { layer: other, channel: Channel::Color, image: Some(image_b) });
    assert_eq!(s.fx.inputs.decoded_image_count(), 2);
    fill(&mut s, FillOp::Image { layer, channel: Channel::Color, image: Some(image_b) });
    assert_eq!(s.fx.inputs.decoded_image_count(), 2, "別の層が指している画像は手放さない");
    assert!(s.doc.effect_inputs().image(image_b).is_some());
    // ロックを外せば同じ操作が通る（断りの理由がロックだったこと）
    s.doc.set_layer_locks(layer, LayerLocks::NONE).unwrap();
    fill(&mut s, FillOp::Image { layer, channel: Channel::Color, image: Some(image_b) });
    assert_eq!(s.doc.layer(layer).unwrap().fill_image(Channel::Color), Some(image_b), "{}", s.message);
    s.sync_effects();
    assert_eq!(s.fx.inputs.decoded_image_count(), 1, "指されなくなった画像は手放す");
    // デカールを置けなかったとき（core の予算で断られる）も、その画像を残さない
    let (_, image_c) = shelf_image(&mut s, "デカール");
    s.doc.set_fill_image_cache_budget_bytes(3);
    let layers = s.doc.layers().len();
    fill(&mut s, FillOp::PlaceDecal { image: image_c, at: pos2(400.0, 300.0), rect: view_rect() });
    assert_eq!(s.doc.layers().len(), layers, "{}", s.message);
    assert!(!s.fx.inputs.has_decoded_image(image_c), "置けなかったデカールの画像は復号したまま残さない");
    s.doc.set_fill_image_cache_budget_bytes(256 * 1024 * 1024);
    // 使っていない棚の画像の読み方を替えても、復号して予算に残さない
    let before = s.fx.inputs.decoded_image_count();
    fill(&mut s, FillOp::ImageColorSpace { image: image_c, space: yolu_core::ImageColorSpace::Linear });
    assert_eq!(s.fx.inputs.decoded_image_count(), before);
    assert!(!s.fx.inputs.has_decoded_image(image_c));
}

#[test]
fn headless_syncing_images_each_frame_keeps_the_shared_budget_and_inputs() {
    let mut s = AppState::new(32, 32);
    s.fx.inputs.image_limit = Some(40);
    for k in 0..3u8 {
        let rgba: Vec<u8> = [[k * 40, 10, 20, 255]; 4].concat();
        let rid = s.shelf.add_image(Lang::Ja, &format!("c{k}"), &rgba, 2, 2).unwrap();
        let image = inputs::image_id(&rid).unwrap();
        let layer = new_fill(&mut s);
        fill(&mut s, FillOp::Image { layer, channel: Channel::Color, image: Some(image) });
        assert_eq!(s.doc.layer(layer).unwrap().fill_image(Channel::Color), (k < 2).then_some(image));
    }
    assert_eq!(s.fx.inputs.decoded_image_count(), 2);
    assert_eq!(s.fx.inputs.decoded_image_bytes(), 32);
    let revision = s.doc.revision();
    let passed = s.fx.inputs.passed;
    for _ in 0..30 { s.sync_effects(); }
    assert_eq!(s.fx.inputs.passed, passed, "同じ入力を文書へ渡し直さない");
    assert_eq!(s.doc.revision(), revision);
    assert_eq!(s.fx.inputs.decoded_image_count(), 2);
    assert_eq!(s.fx.inputs.decoded_image_bytes(), 32);
}

#[test]
fn headless_a_project_with_an_image_layer_opens_and_is_editable_once_the_shelf_image_is_passed() {
    // 画像を使う層は、開いたあと棚の画像を効果の入力へ渡すと（毎フレームの sync_effects）編集できる。画像は棚から戻る
    let dir = temp_dir("reopen-image");
    let path = dir.join("image.ylp");
    let again = dir.join("image-again.ylp");
    let mut s = AppState::new(64, 64);
    let (rid, image) = shelf_image(&mut s, "四色");
    let layer = new_fill(&mut s);
    fill(
        &mut s,
        FillOp::Image {
            layer,
            channel: Channel::Color,
            image: Some(image),
        },
    );
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let mut t = AppState::new(8, 8);
    t.apply(Action::OpenProject(path.clone()));
    assert!(t.message.starts_with("開きました"), "{}", t.message);
    t.sync_effects();
    assert_eq!(t.read_only_reason(), None);
    let layer = t
        .doc
        .layers()
        .iter()
        .find(|l| l.kind() == yolu_app::engine::LayerKind::Fill)
        .expect("塗りつぶしの層が戻る")
        .id();
    assert_eq!(t.doc.layer(layer).unwrap().fill_image(Channel::Color), Some(image));
    assert!(t.doc.effect_inputs().image(image).is_some(), "棚の画像が入力に戻る");
    assert!(t.shelf.get(&rid).is_some());
    // 編集できる: 画像を外すのは 1 回の Undo
    let steps = t.doc.undo_count();
    fill(
        &mut t,
        FillOp::Image {
            layer,
            channel: Channel::Color,
            image: None,
        },
    );
    assert_eq!(t.doc.undo_count(), steps + 1);
    assert_eq!(t.doc.layer(layer).unwrap().fill_image(Channel::Color), None);
    t.apply(Action::Undo);
    assert_eq!(t.doc.layer(layer).unwrap().fill_image(Channel::Color), Some(image));
    // 保存し直して開き直しても、同じく編集できる
    t.apply(Action::SaveProjectAs(again.clone()));
    assert!(t.message.starts_with("保存しました"), "{}", t.message);
    let mut u = AppState::new(8, 8);
    u.apply(Action::OpenProject(again));
    u.sync_effects();
    assert_eq!(u.read_only_reason(), None);
    assert!(u.shelf.get(&rid).is_some());
    let _ = std::fs::remove_dir_all(dir);
}

// ───────── グラデーションの道具 ─────────

fn paint_layer_pixels(s: &AppState, id: LayerId, channel: Channel, x: u32, y: u32) -> Rgba8 {
    s.doc
        .layer(id)
        .unwrap()
        .surface(channel)
        .map_or(Rgba8::TRANSPARENT, |f| f.pixel(x, y).unwrap())
}

fn apply_gradient(s: &mut AppState, a: (f64, f64), b: (f64, f64)) {
    s.apply(Action::Gradient(GradientOp::Apply { start: a, end: b }));
}

#[test]
fn headless_the_gradient_tool_paints_from_the_paint_color_to_transparent_in_one_undo() {
    let mut s = AppState::new(64, 64);
    let layer = s.selected_layer.unwrap();
    s.apply(Action::SelectTool(Tool::Gradient));
    s.color.set_main([1.0, 0.0, 0.0, 1.0]);
    let steps = s.doc.undo_count();
    apply_gradient(&mut s, (4.0, 32.0), (60.0, 32.0));
    assert_eq!(s.doc.undo_count(), steps + 1);
    assert!(
        s.modified && s.message.contains("グラデーション"),
        "{}",
        s.message
    );
    let left = paint_layer_pixels(&s, layer, Channel::Color, 4, 32);
    let mid = paint_layer_pixels(&s, layer, Channel::Color, 32, 32);
    let right = paint_layer_pixels(&s, layer, Channel::Color, 60, 32);
    assert!(
        left.r == 255 && left.g == 0 && left.a >= 250,
        "始点は描画色: {left:?}"
    );
    assert!(
        mid.a > 80 && mid.a < 180 && mid.r == 255,
        "途中は半分ほど（乗算済みで補間。色はそのまま）: {mid:?}"
    );
    assert_eq!(right.a, 0, "終点は透明");
    // 取り消し・やり直し
    assert!(s.doc.undo().unwrap());
    assert_eq!(paint_layer_pixels(&s, layer, Channel::Color, 4, 32).a, 0);
    assert!(s.doc.redo().unwrap());
    assert!(paint_layer_pixels(&s, layer, Channel::Color, 4, 32).a >= 250);
    // クリック（動かしていない）は何も塗らない
    let steps = s.doc.undo_count();
    apply_gradient(&mut s, (10.0, 10.0), (10.5, 10.2));
    assert_eq!(s.doc.undo_count(), steps);
}

#[test]
fn headless_the_gradient_tool_radial_end_color_opacity_and_erase() {
    let mut s = AppState::new(64, 64);
    let layer = s.selected_layer.unwrap();
    s.color.set_main([1.0, 0.0, 0.0, 1.0]);
    s.color.sub = [0.0, 0.0, 1.0, 1.0];
    // 放射: 始点の中心から終点の半径まで。サブの色へ
    s.apply(Action::Gradient(GradientOp::Shape(GradientShape::Radial)));
    s.apply(Action::Gradient(GradientOp::End(End::Sub)));
    apply_gradient(&mut s, (32.0, 32.0), (60.0, 32.0));
    let centre = paint_layer_pixels(&s, layer, Channel::Color, 32, 32);
    assert!(
        centre.r > 240 && centre.b < 15 && centre.a == 255,
        "中心は始点の色: {centre:?}"
    );
    let corner = paint_layer_pixels(&s, layer, Channel::Color, 63, 63);
    assert_eq!(corner, BLUE, "半径の外は終点の色（サブ）");
    let ring = paint_layer_pixels(&s, layer, Channel::Color, 46, 32);
    assert!(
        ring.r > 80 && ring.b > 80 && ring.a == 255,
        "途中は赤と青の間: {ring:?}"
    );
    // 不透明度
    s.apply(Action::Undo);
    s.brush.opacity = 0.5;
    apply_gradient(&mut s, (32.0, 32.0), (60.0, 32.0));
    let c = paint_layer_pixels(&s, layer, Channel::Color, 32, 32);
    assert!(c.a >= 126 && c.a <= 129, "不透明度 50%: {c:?}");
    // 消す: 透明へ向かって消していく（始点の近くが消える）
    s.brush.opacity = 1.0;
    s.apply(Action::Gradient(GradientOp::End(End::Transparent)));
    s.apply(Action::Gradient(GradientOp::Erase(true)));
    s.apply(Action::Gradient(GradientOp::Shape(GradientShape::Linear)));
    apply_gradient(&mut s, (0.0, 32.0), (63.0, 32.0));
    let left = paint_layer_pixels(&s, layer, Channel::Color, 1, 32);
    let right = paint_layer_pixels(&s, layer, Channel::Color, 62, 32);
    assert!(left.a < right.a, "{left:?} {right:?}");
}

#[test]
fn headless_the_gradient_tool_fills_the_mask_with_white_or_black_and_the_material_set_in_one_undo()
{
    let mut s = AppState::new(64, 64);
    let layer = s.selected_layer.unwrap();
    // マスク
    s.apply(Action::M2(Edit::AddMask(layer)));
    assert!(s.m2.edit_mask);
    let steps = s.doc.undo_count();
    apply_gradient(&mut s, (0.0, 32.0), (63.0, 32.0));
    assert_eq!(s.doc.undo_count(), steps + 1);
    let mask = |s: &AppState, x: u32| {
        s.doc
            .layer(layer)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .pixel(x, 32)
            .unwrap()
    };
    let (m0, m1) = (mask(&s, 1), mask(&s, 62));
    assert!(m0.a > m1.a, "始点の側が強く隠す: {m0:?} {m1:?}");
    assert!(s.doc.undo().unwrap());
    // 隠してから、白（見せる）で見せていく
    apply_gradient(&mut s, (0.0, 32.0), (63.0, 32.0));
    let hidden = mask(&s, 1).a;
    s.apply(Action::Gradient(GradientOp::Erase(true)));
    let steps = s.doc.undo_count();
    apply_gradient(&mut s, (63.0, 32.0), (0.0, 32.0));
    assert_eq!(s.doc.undo_count(), steps + 1);
    assert!(
        mask(&s, 62).a < m1.a.max(hidden),
        "反対の側が見えるようになる"
    );
    s.apply(Action::Undo);
    s.apply(Action::Undo);
    s.apply(Action::M2Ui(yolu_app::m2::UiOp::EditMask(false)));
    // マテリアル: 組の全チャンネルを 1 回の Undo で
    s.apply(Action::Gradient(GradientOp::Erase(false)));
    s.color.set_main([0.0, 1.0, 0.0, 1.0]);
    s.apply(Action::Mat(MatAction::Enabled(true)));
    s.apply(Action::Mat(MatAction::Channel(Channel::Color, true)));
    s.apply(Action::Mat(MatAction::Channel(Channel::Roughness, true)));
    s.mat.roughness = 0.25;
    let steps = s.doc.undo_count();
    apply_gradient(&mut s, (4.0, 32.0), (60.0, 32.0));
    assert_eq!(s.doc.undo_count(), steps + 1, "組の全部が 1 回の Undo");
    let green = paint_layer_pixels(&s, layer, Channel::Color, 4, 32);
    assert!(
        green.g == 255 && green.r == 0 && green.a >= 250,
        "{green:?}"
    );
    let rough = paint_layer_pixels(&s, layer, Channel::Roughness, 4, 32);
    assert!(rough.r.abs_diff(64) <= 1 && rough.a >= 250, "{rough:?}");
    assert_eq!(
        paint_layer_pixels(&s, layer, Channel::Roughness, 60, 32).a,
        0
    );
    assert!(s.doc.undo().unwrap());
    assert_eq!(paint_layer_pixels(&s, layer, Channel::Color, 4, 32).a, 0);
    // 2 つのマテリアルの間: 終点は写したマテリアル
    s.color.set_main([0.0, 0.0, 1.0, 1.0]);
    s.mat.roughness = 1.0;
    s.apply(Action::Gradient(GradientOp::CaptureEnd));
    assert!(s.gradient.between);
    s.color.set_main([1.0, 0.0, 0.0, 1.0]);
    s.mat.roughness = 0.0;
    apply_gradient(&mut s, (0.0, 32.0), (63.0, 32.0));
    let start = paint_layer_pixels(&s, layer, Channel::Color, 0, 32);
    let end = paint_layer_pixels(&s, layer, Channel::Color, 63, 32);
    assert!(start.r > 240 && start.b < 20, "{start:?}");
    assert!(end.b > 240 && end.r < 20, "{end:?}");
    assert!(paint_layer_pixels(&s, layer, Channel::Roughness, 63, 32).r > 240);
}

#[test]
fn headless_the_gradient_tool_is_refused_by_locks_fill_layers_and_read_only_sets_and_changes_nothing(
) {
    let mut s = AppState::new(64, 64);
    let layer = s.selected_layer.unwrap();
    s.doc.set_layer_locks(layer, LayerLocks::ALL).unwrap();
    let steps = s.doc.undo_count();
    let revision = s.doc.revision();
    apply_gradient(&mut s, (4.0, 32.0), (60.0, 32.0));
    assert_eq!((s.doc.undo_count(), s.doc.revision()), (steps, revision));
    assert!(s.message.contains("ロック"), "{}", s.message);
    s.lang = Lang::En;
    apply_gradient(&mut s, (4.0, 32.0), (60.0, 32.0));
    assert!(s.message.contains("locked"), "{}", s.message);
    s.lang = Lang::Ja;
    s.doc.set_layer_locks(layer, LayerLocks::NONE).unwrap();
    // 塗りつぶしの層・グループには塗れない
    let fill_layer = new_fill(&mut s);
    let revision = s.doc.revision();
    apply_gradient(&mut s, (4.0, 32.0), (60.0, 32.0));
    assert_eq!(s.doc.revision(), revision);
    assert!(
        s.message.contains("ペイントレイヤーかマスク"),
        "{}",
        s.message
    );
    let _ = fill_layer;
    // 読むだけのセット
    s.selected_layer = Some(layer);
    s.sets.get_mut(0).unwrap().read_only = Some("理由".into());
    let revision = s.doc.revision();
    s.apply(Action::Gradient(GradientOp::Apply {
        start: (4.0, 32.0),
        end: (60.0, 32.0),
    }));
    assert_eq!(s.doc.revision(), revision);
    assert!(s.message.contains("読むだけ"), "{}", s.message);
    // 描いている間
    s.sets.get_mut(0).unwrap().read_only = None;
    let brush = s.stroke_settings(false);
    let stroke = s.doc.begin_stroke(layer, &brush).unwrap();
    s.message.clear();
    apply_gradient(&mut s, (4.0, 32.0), (60.0, 32.0));
    assert!(s.message.contains("描いている間"), "{}", s.message);
    s.doc.end_stroke(stroke).unwrap();
    // 3D ビューでは使わない（2D のキャンバスだけ）
    s.apply(Action::SelectTool(Tool::Gradient));
    assert!(!Tool::Gradient.paints());
}

#[test]
fn headless_the_gradient_drag_is_dropped_by_escape_a_tool_change_and_focus_loss() {
    let mut s = AppState::new(64, 64);
    s.apply(Action::SelectTool(Tool::Gradient));
    let view_state = s.view;
    let image = Rect::from_min_size(pos2(0.0, 0.0), vec2(64.0, 64.0));
    let view = view_state.view(image, 64, 64);
    yolu_app::gradient::canvas::press(
        &mut s,
        &view,
        pos2(10.0, 10.0),
        yolu_app::state::StrokeSource::Mouse,
    );
    assert!(s.gradient.drag.is_some());
    yolu_app::gradient::canvas::moved(
        &mut s,
        &view,
        pos2(50.0, 30.0),
        yolu_app::state::StrokeSource::Mouse,
    );
    assert_ne!(
        s.gradient.drag.unwrap().start,
        s.gradient.drag.unwrap().current
    );
    assert!(yolu_app::gradient::canvas::cancel(&mut s));
    assert!(s.gradient.drag.is_none());
    assert!(s.message.contains("やめ"), "{}", s.message);
    // 道具を替えると捨てる
    yolu_app::gradient::canvas::press(
        &mut s,
        &view,
        pos2(10.0, 10.0),
        yolu_app::state::StrokeSource::Mouse,
    );
    s.apply(Action::SelectTool(Tool::Brush));
    assert!(s.gradient.drag.is_none());
    // 何も塗っていない
    assert_eq!(s.doc.undo_count(), 0);
    // ペン: 触れる・動く・離す
    s.apply(Action::SelectTool(Tool::Gradient));
    let steps = s.doc.undo_count();
    yolu_app::gradient::canvas::pen_sample(&mut s, &view, pos2(5.0, 5.0), 3, true);
    yolu_app::gradient::canvas::pen_sample(&mut s, &view, pos2(60.0, 5.0), 3, true);
    yolu_app::gradient::canvas::pen_sample(&mut s, &view, pos2(60.0, 5.0), 3, false);
    assert_eq!(s.doc.undo_count(), steps + 1);
    assert!(s.gradient.pen_down.is_none());
}

#[test]
fn headless_every_new_name_and_notice_is_short_in_both_languages_with_no_how_to_or_dev_numbers() {
    let dir = temp_dir("texts");
    let png = dir.join("tile.png");
    image::RgbaImage::from_raw(2, 2, quad_image())
        .unwrap()
        .save(&png)
        .unwrap();
    let wide = dir.join("wide.png");
    std::fs::write(&wide, png_header_only(9000, 9000)).unwrap();
    let junk = dir.join("junk.png");
    std::fs::write(&junk, b"not a png").unwrap();
    for lang in Lang::ALL {
        let mut texts: Vec<(String, String)> = Vec::new();
        // 欄の行の名前
        for mode in [
            ProjectionMode::Uv,
            ProjectionMode::Triplanar,
            ProjectionMode::Planar,
            ProjectionMode::Spherical,
            ProjectionMode::Cylindrical,
            ProjectionMode::Decal,
        ] {
            texts.push((
                "投影の種類".into(),
                yolu_app::panels::fill_props::projection_name(lang, mode).into(),
            ));
        }
        for wrap in [Wrap::Repeat, Wrap::Clamp, Wrap::None] {
            texts.push((
                "外側".into(),
                yolu_app::panels::fill_props::wrap_name(lang, wrap).into(),
            ));
        }
        for shape in [Shape::Box, Shape::Sphere, Shape::Plane] {
            texts.push((
                "形".into(),
                yolu_app::panels::fill_props::shape_name(lang, shape).into(),
            ));
        }
        texts.push(("道具".into(), Tool::Gradient.name_in(lang).into()));
        for end in [End::Transparent, End::Sub] {
            texts.push((
                "グラデーションの終点".into(),
                yolu_app::gradient::props::end_name(lang, end).into(),
            ));
        }
        // 状態の帯と断りの文: 操作を当てて、その知らせを集める
        let mut s = cube();
        s.lang = lang;
        let mut note = |s: &AppState, what: &str| {
            assert!(!s.message.is_empty(), "{what}: 知らせが無い");
            texts.push((what.to_owned(), s.message.clone()));
        };
        fill(&mut s, FillOp::ImportImage(png.clone()));
        note(&s, "取り込み");
        fill(&mut s, FillOp::ImportImage(png.clone()));
        note(&s, "取り込み済み");
        fill(&mut s, FillOp::ImportImage(junk.clone()));
        note(&s, "PNG でない");
        fill(&mut s, FillOp::ImportImage(dir.join("missing.png")));
        note(&s, "無いファイル");
        fill(&mut s, FillOp::ImportImage(wide.clone()));
        note(&s, "大きすぎる");
        let rid = s.shelf.resources().last().unwrap().id.clone();
        let image = inputs::image_id(&rid).unwrap();
        let base = s.selected_layer.unwrap();
        fill(
            &mut s,
            FillOp::Image {
                layer: base,
                channel: Channel::Color,
                image: Some(image),
            },
        );
        note(&s, "塗りつぶしでない層");
        let layer = new_fill(&mut s);
        fill(
            &mut s,
            FillOp::Image {
                layer,
                channel: Channel::Color,
                image: Some(image),
            },
        );
        note(&s, "画像を差す");
        s.apply(Action::Shelf(ShelfOp::Remove(rid.clone())));
        note(&s, "使われている画像を消す");
        fill(
            &mut s,
            FillOp::Image {
                layer,
                channel: Channel::Color,
                image: None,
            },
        );
        note(&s, "画像を外す");
        fill(
            &mut s,
            FillOp::PlaceDecal {
                image,
                at: pos2(400.0, 300.0),
                rect: view_rect(),
            },
        );
        note(&s, "デカールを置く");
        fill(
            &mut s,
            FillOp::PlaceDecal {
                image,
                at: pos2(2.0, 2.0),
                rect: view_rect(),
            },
        );
        note(&s, "モデルの外へ置く");
        let mut bare = AppState::new(32, 32);
        bare.lang = lang;
        let bare_layer = new_fill(&mut bare);
        fill(&mut bare, FillOp::FitPlacement { layer: bare_layer });
        note(&bare, "モデルが無い");
        let (mut painted, id) = painted(16);
        painted.lang = lang;
        painted.shelf.async_bytes = 0;
        painted.shelf.hold_saves(true);
        painted.apply(Action::Shelf(ShelfOp::SaveMaterial(id)));
        fill(&mut painted, FillOp::ImportImage(png.clone()));
        note(&painted, "保存中の取り込み");
        painted.shelf.hold_saves(false);
        painted.shelf_wait();
        assert!(texts.len() >= 25, "{lang:?}: {}", texts.len());
        for (what, text) in &texts {
            common::assert_plain(&format!("{lang:?} {what}"), text);
            if lang == Lang::En {
                assert!(
                    !common::has_japanese(text),
                    "英語の画面に日本語（{what}）: {text}"
                );
            }
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}
#[test]
fn headless_a_projection_without_an_image_is_restored_when_the_project_is_opened() {
    let dir = temp_dir("reopen");
    let path = dir.join("projection.ylp");
    let mut s = AppState::new(64, 64);
    let layer = new_fill(&mut s);
    fill(
        &mut s,
        FillOp::ProjectionMode {
            layer,
            mode: ProjectionMode::Triplanar,
        },
    );
    let mut p = layer_projection(&s, layer);
    p.tiles = [2.0, 0.5];
    p.rotation = -45.0;
    p.blend_width = 0.1;
    p.placement = Placement {
        center: [0.5, -0.25, 1.5],
        rotation: [10.0, -20.0, 30.0],
        size: [2.0, 3.0, 4.0],
    };
    fill(
        &mut s,
        FillOp::Projection {
            layer,
            projection: Box::new(p),
            coalesce: false,
        },
    );
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(s.message.starts_with("保存しました"), "{}", s.message);
    let mut t = AppState::new(8, 8);
    t.apply(Action::OpenProject(path.clone()));
    assert!(t.message.starts_with("開きました"), "{}", t.message);
    assert!(
        t.read_only_reason().is_none(),
        "効かない効果が無いので編集できる"
    );
    let id = t
        .doc
        .layers()
        .iter()
        .find(|l| l.kind() == yolu_app::engine::LayerKind::Fill)
        .unwrap()
        .id();
    assert_eq!(layer_projection(&t, id), p);
    // 開いた文書でも同じ操作で直せて、1 回の Undo で戻る
    let steps = t.doc.undo_count();
    fill(
        &mut t,
        FillOp::ProjectionMode {
            layer: id,
            mode: ProjectionMode::Planar,
        },
    );
    assert_eq!(t.doc.undo_count(), steps + 1);
    assert!(t.doc.undo().unwrap());
    assert_eq!(layer_projection(&t, id), p);
    let _ = std::fs::remove_dir_all(dir);
}

/// 2 つのマテリアル（Skin・Hair。大きさは 64）の 3D のモデル。テクスチャセットが 2 つできる。
fn two_set_model() -> yolu_protocol::Model {
    use yolu_protocol::{
        channel, ChannelRoute, MaterialInfo, MaterialKey, MeshData, Model, Submesh, TextureProperty,
    };
    let material = |name: &str| MaterialInfo {
        key: MaterialKey::Material {
            name: name.into(),
            asset: None,
        },
        shader: "Standard".into(),
        textures: vec![TextureProperty {
            name: "_MainTex".into(),
            width: 64,
            height: 64,
        }],
        routes: vec![ChannelRoute {
            channel: channel::COLOR,
            property: "_MainTex".into(),
        }],
    };
    Model {
        generation: 1,
        name: "model".into(),
        materials: vec![material("Skin"), material("Hair")],
        meshes: vec![MeshData {
            key: "0".into(),
            name: "Body".into(),
            skinned: false,
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            submeshes: (0..2)
                .map(|m| Submesh {
                    material: m,
                    indices: vec![0, 1, 2],
                })
                .collect(),
        }],
    }
}

#[test]
fn headless_a_shelf_image_a_layer_reads_is_not_removed_and_the_refusal_names_the_layers() {
    let mut s = AppState::new(32, 32);
    let (rid, image) = shelf_image(&mut s, "四色");
    assert!(yolu_app::fillfx::image_users(&s, &rid).is_empty());
    let a = new_fill(&mut s);
    let b = new_fill(&mut s);
    for (layer, channel) in [(a, Channel::Color), (b, Channel::Roughness)] {
        fill(
            &mut s,
            FillOp::Image {
                layer,
                channel,
                image: Some(image),
            },
        );
    }
    let users = yolu_app::fillfx::image_users(&s, &rid);
    let names: Vec<String> = [a, b]
        .iter()
        .map(|id| s.doc.layer(*id).unwrap().name().to_owned())
        .collect();
    assert_eq!(
        users, names,
        "読んでいる層の名前（セットが 1 つなら、そのまま）"
    );
    assert!(yolu_app::fillfx::image_users(&s, "not-a-guid").is_empty());
    let shown = composite(&s.doc);
    // 消す確かめの窓を出す前に断る。層は画像を見せ続け、棚も文書も変わらない
    s.modified = false;
    let steps = s.doc.undo_count();
    s.apply(Action::Shelf(ShelfOp::AskRemove(rid.clone())));
    assert!(s.dialog_request.is_none() && s.shelf.pending_remove.is_none());
    assert!(
        s.message.contains("使われています") && s.message.contains(&names[0]),
        "{}",
        s.message
    );
    s.apply(Action::Shelf(ShelfOp::Remove(rid.clone())));
    assert!(s.message.contains("使われています"), "{}", s.message);
    assert!(s.shelf.get(&rid).is_some(), "棚から消えない");
    assert!(!s.modified, "何も変えない");
    assert_eq!(s.doc.undo_count(), steps);
    assert_eq!(
        s.doc.layer(a).unwrap().fill_image(Channel::Color),
        Some(image)
    );
    s.sync_effects();
    assert_eq!(composite(&s.doc), shown, "層は画像を見せ続ける");
    s.lang = Lang::En;
    s.apply(Action::Shelf(ShelfOp::Remove(rid.clone())));
    assert!(s.message.contains("Used by a layer"), "{}", s.message);
    s.lang = Lang::Ja;
    // 1 つの層が外しても、もう 1 つが読んでいる間は消せない
    fill(
        &mut s,
        FillOp::Image {
            layer: a,
            channel: Channel::Color,
            image: None,
        },
    );
    s.apply(Action::Shelf(ShelfOp::Remove(rid.clone())));
    assert!(s.shelf.get(&rid).is_some());
    assert!(s.message.contains(&names[1]), "{}", s.message);
    // 読む層がなくなれば、確かめの窓を出し、消せる
    fill(
        &mut s,
        FillOp::Image {
            layer: b,
            channel: Channel::Roughness,
            image: None,
        },
    );
    assert!(yolu_app::fillfx::image_users(&s, &rid).is_empty());
    s.apply(Action::Shelf(ShelfOp::AskRemove(rid.clone())));
    assert_eq!(s.shelf.pending_remove.as_deref(), Some(rid.as_str()));
    assert_eq!(
        s.dialog_request,
        Some(yolu_app::state::DialogRequest::ShelfRemove)
    );
    s.apply(Action::Shelf(ShelfOp::Remove(rid.clone())));
    assert!(s.shelf.get(&rid).is_none(), "{}", s.message);
    assert!(s.message.contains("棚から消しました"), "{}", s.message);
}

#[test]
fn headless_with_two_texture_sets_the_users_carry_the_set_name_and_the_other_set_keeps_the_image() {
    let mut s = AppState::new(64, 64);
    s.receive_link_model(&two_set_model(), 0).1.expect("モデル");
    assert_eq!(s.sets.len(), 2);
    let (rid, image) = shelf_image(&mut s, "四色");
    let names: Vec<String> = s.sets.iter().map(|x| x.name.clone()).collect();
    assert_eq!(names, ["Skin", "Hair"]);
    let uids: Vec<u32> = s.sets.iter().map(|x| x.uid).collect();
    // どちらのセットにも、その画像を読む塗りつぶしの層を 1 つずつ
    let mut layer_names = Vec::new();
    for uid in &uids {
        s.apply(Action::SelectSet(*uid));
        let layer = new_fill(&mut s);
        fill(
            &mut s,
            FillOp::Image {
                layer,
                channel: Channel::Color,
                image: Some(image),
            },
        );
        assert_eq!(
            s.doc.layer(layer).unwrap().fill_image(Channel::Color),
            Some(image),
            "{}",
            s.message
        );
        layer_names.push((layer, s.doc.layer(layer).unwrap().name().to_owned()));
    }
    let users = yolu_app::fillfx::image_users(&s, &rid);
    assert_eq!(
        users,
        [
            format!("Skin: {}", layer_names[0].1),
            format!("Hair: {}", layer_names[1].1)
        ],
        "セットが 2 つ以上なら、セットの名前を前に付ける"
    );
    // 今のセット（Hair）の層が外しても、別のセット（Skin）の層が読んでいる間は消せず、断りにそのセットの名前が出る
    fill(
        &mut s,
        FillOp::Image {
            layer: layer_names[1].0,
            channel: Channel::Color,
            image: None,
        },
    );
    s.apply(Action::Shelf(ShelfOp::Remove(rid.clone())));
    assert!(s.shelf.get(&rid).is_some());
    assert!(
        s.message.contains(&format!("Skin: {}", layer_names[0].1)),
        "{}",
        s.message
    );
    // 別のセットの文書も、入力に画像を持ち続けている（棚から消えていない）
    s.apply(Action::SelectSet(uids[0]));
    assert!(s.doc.effect_inputs().image(image).is_some());
    fill(
        &mut s,
        FillOp::Image {
            layer: layer_names[0].0,
            channel: Channel::Color,
            image: None,
        },
    );
    s.apply(Action::Shelf(ShelfOp::Remove(rid.clone())));
    assert!(s.shelf.get(&rid).is_none(), "{}", s.message);
}
#[test]
fn headless_switching_the_texture_set_during_a_gizmo_drag_drops_the_drag_without_touching_the_other_document(
) {
    let (mut s, layer) = planar_cube();
    let rect = view_rect();
    let from = gizmo::handle_point(&s, rect, Handle::MoveX).unwrap();
    let start = layer_projection(&s, layer);
    assert!(gizmo::press(&mut s, rect, from, gizmo::Source::Mouse));
    gizmo::drag_to(&mut s, rect, from + vec2(30.0, 0.0), false, false);
    let moved = layer_projection(&s, layer);
    assert_ne!(moved, start);
    // テクスチャセットを替えた状態（別の文書が今の文書になる）の代わりに、ドラッグの文書の印だけ替えて、別の文書のように扱わせる
    let mut d = s.fillfx.drag.clone().unwrap();
    d.doc_id ^= 1;
    s.fillfx.drag = Some(d);
    let before = layer_projection(&s, layer);
    gizmo::drag_to(&mut s, rect, from + vec2(60.0, 0.0), false, false);
    assert!(s.fillfx.drag.is_none(), "別の文書のドラッグは捨てる");
    assert_eq!(layer_projection(&s, layer), before, "今の文書は動かさない");
}

#[test]
fn headless_an_images_reading_follows_its_color_space_and_is_not_an_undo_step() {
    let mut s = AppState::new(32, 32);
    // 0 と 255 だけの画像は sRGB への変換で変わらないので、中間の値の画像で確かめる
    let mid: Vec<u8> = [
        [128u8, 128, 128, 255],
        [64, 64, 64, 255],
        [200, 100, 50, 255],
        [30, 60, 90, 255],
    ]
    .iter()
    .flatten()
    .copied()
    .collect();
    let rid = s.shelf.add_image(Lang::Ja, "中間", &mid, 2, 2).unwrap();
    let image = inputs::image_id(&rid).unwrap();
    let layer = new_fill(&mut s);
    fill(
        &mut s,
        FillOp::Image {
            layer,
            channel: Channel::Color,
            image: Some(image),
        },
    );
    let srgb = composite(&s.doc);
    assert_eq!(s.shelf.get(&rid).unwrap().metadata["colorSpace"], "srgb");
    let steps = s.doc.undo_count();
    s.modified = false;
    fill(
        &mut s,
        FillOp::ImageColorSpace {
            image,
            space: yolu_core::ImageColorSpace::Linear,
        },
    );
    assert_eq!(s.shelf.get(&rid).unwrap().metadata["colorSpace"], "linear");
    assert_eq!(s.doc.undo_count(), steps, "取り消しに入らない");
    assert!(s.shelf.changed && s.modified);
    // リニアの画像を色のチャンネルへ読むと sRGB に直す: 絵が変わる
    let linear = composite(&s.doc);
    assert_ne!(linear, srgb);
    // データのチャンネルは、どの色空間でも値のまま
    fill(
        &mut s,
        FillOp::Image {
            layer,
            channel: Channel::Roughness,
            image: Some(image),
        },
    );
    let data_linear = s.doc.composite_pixel(Channel::Roughness, 3, 3).unwrap();
    fill(
        &mut s,
        FillOp::ImageColorSpace {
            image,
            space: yolu_core::ImageColorSpace::Srgb,
        },
    );
    assert_eq!(
        s.doc.composite_pixel(Channel::Roughness, 3, 3).unwrap(),
        data_linear
    );
    // 戻すと元の絵
    assert_eq!(composite(&s.doc), srgb);
    // 棚に無い画像・同じ値・描いている間
    s.message.clear();
    fill(
        &mut s,
        FillOp::ImageColorSpace {
            image: ImageId(5),
            space: yolu_core::ImageColorSpace::Linear,
        },
    );
    assert!(!s.message.is_empty());
    s.message.clear();
    fill(
        &mut s,
        FillOp::ImageColorSpace {
            image,
            space: yolu_core::ImageColorSpace::Srgb,
        },
    );
    assert!(s.message.is_empty(), "同じ値は何も言わない");
    // 保存して .ylp の棚へ入った色空間を読み直す
    let dir = temp_dir("space");
    let path = dir.join("space.ylp");
    fill(
        &mut s,
        FillOp::ImageColorSpace {
            image,
            space: yolu_core::ImageColorSpace::Linear,
        },
    );
    s.apply(Action::SaveProjectAs(path.clone()));
    let project = yolu_io::Project::read(&std::fs::read(&path).unwrap()).unwrap();
    let inputs = project.image_inputs().unwrap();
    assert_eq!(
        inputs
            .iter()
            .find(|(id, _)| *id == image)
            .unwrap()
            .1
            .color_space,
        yolu_core::ImageColorSpace::Linear
    );
    let _ = std::fs::remove_dir_all(dir);
    // 名前は両方の言語で
    for lang in Lang::ALL {
        for space in [
            yolu_core::ImageColorSpace::Srgb,
            yolu_core::ImageColorSpace::Linear,
            yolu_core::ImageColorSpace::Unspecified,
        ] {
            let name = yolu_app::panels::fill_props::space_name(lang, space);
            assert_eq!(lang == Lang::En, name.is_ascii(), "{name}");
        }
    }
}

#[test]
fn headless_the_gizmo_edits_a_shape_gradient_generator_in_the_filter_stack_in_one_undo() {
    use yolu_core::generator::Settings;
    use yolu_core::{EffectSettings, FilterSpec, FilterTarget};
    let mut s = cube();
    s.view3d.camera.yaw = -40.0;
    s.view3d.camera.pitch = 15.0;
    let rect = view_rect();
    let layer = s.selected_layer.unwrap(); // 描く層（塗りつぶしでない層にも、スタックの Generator のギズモは出る）
    let filter = s
        .doc
        .add_filter(
            layer,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(Settings::new(
                Kind::ShapeGradient,
            )))
            .channels(&[Channel::Color]),
        )
        .unwrap();
    assert!(gizmo::target(&s).is_none(), "編集にするまでは出ない");
    s.apply(Action::Fill(FillOp::EditFilter(Some((layer, filter)))));
    assert_eq!(
        gizmo::target(&s),
        Some(gizmo::Target::Filter(layer, filter))
    );
    let volume = |s: &AppState| {
        s.doc
            .find_filter(filter)
            .unwrap()
            .1
            .settings()
            .generator_settings()
            .unwrap()
            .volume
    };
    let start = volume(&s);
    let from = gizmo::handle_point(&s, rect, Handle::SizeXPos).expect("X の面のつまみ");
    let steps = s.doc.undo_count();
    assert!(gizmo::press(&mut s, rect, from, gizmo::Source::Mouse));
    gizmo::drag_to(&mut s, rect, from + vec2(20.0, 0.0), false, false);
    gizmo::drag_to(&mut s, rect, from + vec2(40.0, 0.0), false, false);
    gizmo::release(&mut s, true);
    assert_eq!(s.doc.undo_count(), steps + 1, "1 回の Undo");
    assert_ne!(volume(&s).size[0], start.size[0]);
    assert!(s.doc.undo().unwrap());
    assert_eq!(volume(&s), start);
    // Esc は戻して履歴に残さない
    let steps = s.doc.undo_count();
    assert!(gizmo::press(&mut s, rect, from, gizmo::Source::Mouse));
    gizmo::drag_to(&mut s, rect, from + vec2(40.0, 0.0), false, false);
    gizmo::release(&mut s, false);
    assert_eq!(volume(&s), start);
    assert_eq!(s.doc.undo_count(), steps);
    // マスクを編集している間も出る（マスクの Generator を編集するため）。別の層を選ぶと出ない。やめるとなくなる
    s.apply(Action::M2(Edit::AddMask(layer)));
    assert!(s.m2.edit_mask && gizmo::target(&s).is_some());
    s.m2.edit_mask = false;
    s.apply(Action::NewLayer);
    assert!(gizmo::target(&s).is_none());
    s.selected_layer = Some(layer);
    s.apply(Action::Fill(FillOp::EditFilter(None)));
    assert!(gizmo::target(&s).is_none());
    // Generator が形のグラデーションでなければ出さない
    let other = s
        .doc
        .add_filter(
            layer,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(Settings::new(Kind::Dirt)))
                .channels(&[Channel::Color]),
        )
        .unwrap();
    s.apply(Action::Fill(FillOp::EditFilter(Some((layer, other)))));
    assert!(gizmo::target(&s).is_none());
    // グラデーションの編集に切り替えるとフィルターの編集は外れる（ギズモは 1 つ）
    s.apply(Action::Fill(FillOp::EditFilter(Some((layer, filter)))));
    let fill_layer = new_fill(&mut s);
    s.apply(Action::Fill(FillOp::AddGradient {
        layer: fill_layer,
        channel: Channel::Roughness,
    }));
    assert!(s.fillfx.edit_gradient.is_some());
    s.apply(Action::Fill(FillOp::EditFilter(Some((layer, filter)))));
    assert!(s.fillfx.edit_gradient.is_none());
}
