//! 3D ビューの仕上げ（アンチエイリアス・ブルーム）、光の既定の向き、描き先のメモリ（egui_kittest。描画は wgpu のソフトの描画）。
//!
//! - アンチエイリアス: サンプル数ごとに、斜めの縁の画素が中間の値になる（1 は 0・255 のどちらかだけ）。8 bit の描き先・HDR の描き先（トーンマッピング・
//!   ブルーム）・光なしの表示・lilToon の半透明（別のパスで重ねる道）の全部で。内側と背景は数で変わらない。機材が対応しない数は選んでも下がる。
//! - ブルーム: 明るい所の周りだけが明るくなり、しきい値より暗い所は変わらず、切のときは前と同じ絵（バイト）。
//! - 設定: 仕上げは設定のファイルへ書いて、次の起動で戻る。古いファイルはそのまま読める。
//! - 描き先のメモリ: 見積もりの式と、上限を超えるときにサンプル数が下がること。
//!
//! ウィンドウは `view3d_fx` の共有の装置（製品と同じ `wgpu_configuration` の装置の設定。2×・8× を調べる形式の機能つき）で作る。ほかの試験の
//! 共用の接続（`common::shared_gpu`）とは別の装置だが、ウィンドウは `gpu_thread::builder` の貸し出しで 1 つずつ作るので、装置どうしは重ならない。
use crate::common;

use std::sync::{Arc, OnceLock};

use common::*;
use eframe::egui_wgpu::{self, WgpuSetup};
use egui::{pos2, vec2, Rect};
use egui_kittest::Harness;
use yolu_app::pen::PenInput;
use yolu_app::state::{Action, AppState};
use yolu_app::view3d::brdf::Curve;
use yolu_app::view3d::display::{clamp_samples, Display, Op, Shading, SAMPLE_CHOICES};
use yolu_app::view3d::model::ViewModel;
use yolu_app::view3d::render::{bloom_levels, plan_samples, target_bytes};
use yolu_app::YoluApp;
use yolu_core::geometry::{ModelMesh, OrbitCamera, Submesh};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::look::{LookKind, LookValue, MaterialLook, TextureSource};
use yolu_core::Channel;

// ───────── ウィンドウと場面 ─────────

/// 製品と同じ装置の設定（`wgpu_configuration`）で作った、試験どうしで共有する描画の接続。アダプターは kittest の選び（ソフトの描画を優先）。
fn renderer() -> egui_kittest::wgpu::WgpuTestRenderer {
    static CONNECTION: OnceLock<egui_wgpu::RenderState> = OnceLock::new();
    let connection = CONNECTION.get_or_init(|| {
        let WgpuSetup::CreateNew(mut setup) = egui_kittest::wgpu::default_wgpu_setup() else {
            unreachable!("kittest の既定は新しく作る形")
        };
        let WgpuSetup::CreateNew(product) =
            yolu_app::view3d::render::wgpu_configuration(false).wgpu_setup
        else {
            unreachable!("製品の設定は新しく作る形")
        };
        setup.device_descriptor = product.device_descriptor;
        egui_kittest::wgpu::create_render_state(WgpuSetup::CreateNew(setup), render_options())
    });
    let mut state = connection.clone();
    state.renderer = Arc::new(egui::mutex::RwLock::new(egui_wgpu::Renderer::new(
        &state.device,
        state.target_format,
        render_options(),
    )));
    egui_kittest::wgpu::WgpuTestRenderer::from_render_state(state)
}

fn build(
    width: f32,
    height: f32,
    make: impl FnOnce(&egui::Context) -> YoluApp + 'static,
) -> Harness<'static, YoluApp> {
    let mut h = gpu_thread::builder()
        .with_size(vec2(width, height))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .renderer(renderer())
        .build_eframe(move |cc| {
            let mut app = make(&cc.egui_ctx);
            // 中央は 1 つの組（3D ビューだけが広く出る）
            app.dock = tabbed_center_dock(width);
            with_render_state_cpu_canvas(app, cc.wgpu_render_state.as_ref())
        });
    h.state_mut().state.bake.backend = yolu_app::bake::BakeBackend::Cpu;
    h.run();
    h
}

/// 3D のタブを出したウィンドウ（標準の見た目。文書は doc × doc）。
fn view(width: f32, height: f32, doc: u32) -> Harness<'static, YoluApp> {
    let mut h = build(width, height, move |ctx| {
        YoluApp::for_context(ctx, AppState::new(doc, doc), PenInput::detached())
    });
    h.state_mut()
        .state
        .doc
        .restore_look(MaterialLook::default())
        .unwrap();
    click_tab(&mut h, yolu_app::Tab::View3d);
    move_to(&h, pos2(1.0, 1.0));
    h.run();
    h
}

/// 板。法線は `normal`（−Z ならカメラ（−Z の側）に向く、+Z なら +Z の側から見える）。一辺 size。
fn quad(size: f32, normal: Vec3) -> ModelMesh {
    let h = size * 0.5;
    let front = normal.z < 0.0;
    ModelMesh {
        name: "板".into(),
        positions: vec![
            Vec3::new(-h, -h, 0.0),
            Vec3::new(h, -h, 0.0),
            Vec3::new(-h, h, 0.0),
            Vec3::new(h, h, 0.0),
        ],
        normals: vec![normal; 4],
        uvs: vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 1.0),
        ],
        submeshes: vec![Submesh {
            material: 0,
            // 表は時計回り（Unity の表）。+Z を向く板は巡りを逆にする
            indices: if front {
                vec![0, 2, 1, 2, 3, 1]
            } else {
                vec![0, 1, 2, 2, 1, 3]
            },
        }],
    }
}

fn set_model(h: &mut Harness<'_, YoluApp>, meshes: Vec<ModelMesh>) {
    let state = &mut h.state_mut().state;
    let revision = state.view3d.next_revision();
    state.view3d.material = 0;
    let model = ViewModel::new("試験", meshes, vec![Some("試験".to_string())], revision).unwrap();
    model.tangents();
    state.view3d.set_model(model);
    h.run();
}

fn look_at(h: &mut Harness<'_, YoluApp>, distance: f32, yaw: f32, pitch: f32) {
    h.state_mut().state.view3d.camera = OrbitCamera {
        target: Vec3::ZERO,
        yaw,
        pitch,
        distance,
        model_radius: 1.0,
        ..Default::default()
    };
    h.run();
}

fn op(h: &mut Harness<'_, YoluApp>, op: Op) {
    h.state_mut().apply(Action::View3d(op));
    h.run();
}

/// 文書の全面を覆う塗りつぶしレイヤーを足す。
fn fill(h: &mut Harness<'_, YoluApp>, values: &[(Channel, [u8; 4])]) {
    let values: Vec<(Channel, yolu_core::Rgba8)> = values
        .iter()
        .map(|(c, v)| (*c, yolu_core::Rgba8::new(v[0], v[1], v[2], v[3])))
        .collect();
    h.state_mut()
        .state
        .doc
        .add_fill_layer("値", &values, None)
        .unwrap();
    h.run();
}

fn view_rect(h: &Harness<'_, YoluApp>) -> Rect {
    h.state().view3d_rect().expect("3D のタブを描いた")
}

fn center(h: &Harness<'_, YoluApp>) -> egui::Pos2 {
    view_rect(h).center()
}

fn px(image: &image::RgbaImage, p: egui::Pos2) -> [u8; 3] {
    let c = image.get_pixel(p.x.round() as u32, p.y.round() as u32).0;
    [c[0], c[1], c[2]]
}

fn close(a: [u8; 3], b: [u8; 3], tol: u8) -> bool {
    (0..3).all(|k| a[k].abs_diff(b[k]) <= tol)
}

fn assert_close(a: [u8; 3], b: [u8; 3], tol: u8, what: &str) {
    assert!(
        close(a, b, tol),
        "{what}: {a:?} と期待 {b:?}（許す差 {tol}）"
    );
}

/// 数える範囲（右上のアイコンの帯の下、表示域の内側）。
fn region(h: &Harness<'_, YoluApp>) -> Rect {
    let rect = view_rect(h);
    Rect::from_min_max(rect.min + vec2(20.0, 50.0), rect.max - vec2(20.0, 20.0))
}

/// 板に覆われない背景の点（表示域の左下の隅の近く）。
fn background_point(h: &Harness<'_, YoluApp>) -> egui::Pos2 {
    let rect = view_rect(h);
    pos2(rect.left() + 30.0, rect.bottom() - 30.0)
}

/// 範囲の中の、どの色にも tol 以内で当たらない画素の数（縁の中間の値）。軸の印 skip の中は数えない。
fn count_other(
    image: &image::RgbaImage,
    area: Rect,
    skip: Option<Rect>,
    colors: &[[u8; 3]],
    tol: u8,
) -> usize {
    let mut count = 0;
    for y in area.top().ceil() as u32..area.bottom().floor() as u32 {
        for x in area.left().ceil() as u32..area.right().floor() as u32 {
            if skip.is_some_and(|r| r.contains(pos2(x as f32, y as f32))) {
                continue;
            }
            let c = image.get_pixel(x, y).0;
            let c = [c[0], c[1], c[2]];
            if !colors.iter().any(|k| close(c, *k, tol)) {
                count += 1;
            }
        }
    }
    count
}

/// 板に真正面から光を当てる（光も目も −Z）。環境なし。
fn head_on(h: &mut Harness<'_, YoluApp>) {
    op(h, Op::LightYaw(180.0));
    op(h, Op::LightPitch(0.0));
    op(h, Op::Env(yolu_app::view3d::display::EnvKind::None));
}

// ───────── アンチエイリアス ─────────

#[test]
fn the_supported_counts_are_listed_ascending_and_always_include_one_and_four() {
    let h = view(900.0, 640.0, 32);
    let supported = h.state().view3d_supported_samples().expect("描き手がある");
    // 何で確かめたかの記録（--nocapture で見える）
    eprintln!(
        "view3d_fx: {:?} のサンプル数 {supported:?}",
        h.state().view3d_adapter()
    );
    assert!(supported.windows(2).all(|w| w[0] < w[1]), "{supported:?}");
    assert!(
        supported.iter().all(|n| SAMPLE_CHOICES.contains(n)),
        "{supported:?}"
    );
    // WebGPU が保証する 1 と 4 は、どの機材・どの装置の設定でも使える
    assert!(
        supported.contains(&1) && supported.contains(&4),
        "{supported:?}"
    );
    // 画面の選びは、同じ数だけ押せる
    assert_eq!(
        h.state().state.view3d.display.supported_sample_counts(),
        supported
    );
}

/// 場面ごとの設定（面の色は一様になるものだけ。縁の中間の値を数えるため）。
fn aa_scenes() -> Vec<(&'static str, Vec<Op>)> {
    vec![
        (
            "光なし（チャンネル）",
            vec![Op::Shading(Shading::Channel(Channel::Color))],
        ),
        (
            "8 bit の描き先（中立）",
            vec![Op::Shading(Shading::Neutral)],
        ),
        (
            "HDR の描き先（トーンマッピング）",
            vec![
                Op::Shading(Shading::Neutral),
                Op::Tone(Curve::Aces),
                Op::Exposure(0.5),
            ],
        ),
        (
            "HDR の描き先（ブルーム）",
            vec![
                Op::Shading(Shading::Neutral),
                Op::Bloom(true),
                Op::BloomThreshold(4.0),
            ],
        ),
    ]
}

#[test]
fn every_sample_count_makes_intermediate_values_on_slanted_edges_and_leaves_the_inside_alone() {
    let mut h = view(900.0, 640.0, 32);
    set_model(&mut h, vec![quad(1.0, Vec3::NEG_Z)]);
    look_at(&mut h, 2.6, 30.0, 20.0);
    head_on(&mut h);
    fill(&mut h, &[(Channel::Color, [200, 120, 60, 255])]);
    let supported = h.state().view3d_supported_samples().unwrap();
    let area = region(&h);
    let corner = background_point(&h);
    for (name, ops) in aa_scenes() {
        for o in &ops {
            op(&mut h, *o);
        }
        let mut reference: Option<([u8; 3], [u8; 3])> = None;
        for n in SAMPLE_CHOICES {
            op(&mut h, Op::Antialias(n));
            let image = h.render().expect("描ける");
            let samples = h.state().view3d_stats().unwrap().samples;
            let want = clamp_samples(n, &supported);
            assert_eq!(samples, want, "{name}・{n}×を選ぶ");
            let (face, background) = (px(&image, center(&h)), px(&image, corner));
            assert_ne!(face, background, "{name}: 板が見える");
            let others = count_other(&image, area, view3d_axes_rect(&h), &[face, background], 2);
            if samples == 1 {
                assert_eq!(others, 0, "{name}・1×: 縁の画素は面か背景のどちらか");
            } else {
                assert!(
                    others > 100,
                    "{name}・{samples}×: 縁に中間の値が出る（{others}）"
                );
            }
            // 内側と背景は、数で変わらない（縁だけが変わる）
            match reference {
                None => reference = Some((face, background)),
                Some((f, b)) => {
                    assert_close(face, f, 1, &format!("{name}・{samples}×の面"));
                    assert_close(background, b, 1, &format!("{name}・{samples}×の背景"));
                }
            }
        }
        // 次の場面のために戻す
        op(&mut h, Op::Bloom(false));
        op(&mut h, Op::Tone(Curve::None));
        op(&mut h, Op::Exposure(0.0));
    }
}

#[test]
fn a_lil_toon_transparent_face_blends_edges_at_every_sample_count_too() {
    let mut h = view(900.0, 640.0, 32);
    set_model(&mut h, vec![quad(1.0, Vec3::NEG_Z)]);
    look_at(&mut h, 2.6, 30.0, 20.0);
    head_on(&mut h);
    fill(&mut h, &[(Channel::Color, [200, 120, 60, 200])]);
    let mut look = MaterialLook {
        kind: LookKind::LilToon,
        ..MaterialLook::default()
    };
    look.textures
        .insert("_MainTex".into(), TextureSource::Channel(Channel::Color));
    look.shader = "Hidden/lilToonTransparent".into();
    look.properties
        .insert("_Cutoff".into(), LookValue::Float(0.001));
    h.state_mut().state.doc.set_look(look, false).unwrap();
    h.run();
    let supported = h.state().view3d_supported_samples().unwrap();
    let area = region(&h);
    let corner = background_point(&h);
    let mut reference = None;
    for n in SAMPLE_CHOICES {
        op(&mut h, Op::Antialias(n));
        let image = h.render().expect("描ける");
        let samples = h.state().view3d_stats().unwrap().samples;
        assert_eq!(samples, clamp_samples(n, &supported));
        let (face, background) = (px(&image, center(&h)), px(&image, corner));
        assert_ne!(face, background, "半透明の板が背景と違う");
        let others = count_other(&image, area, view3d_axes_rect(&h), &[face, background], 3);
        if samples == 1 {
            assert_eq!(others, 0, "1×");
        } else {
            assert!(
                others > 100,
                "{samples}×: 半透明の縁にも中間の値が出る（{others}）"
            );
        }
        match reference {
            None => reference = Some(face),
            Some(f) => assert_close(face, f, 2, &format!("{samples}×の面は数で変わらない")),
        }
    }
}

#[test]
fn the_chosen_count_is_kept_but_lowered_for_the_device_and_for_the_memory_ceiling() {
    let mut h = view(900.0, 640.0, 32);
    set_model(&mut h, vec![quad(1.0, Vec3::NEG_Z)]);
    look_at(&mut h, 2.6, 30.0, 20.0);
    head_on(&mut h);
    fill(&mut h, &[(Channel::Color, [200, 120, 60, 255])]);
    op(&mut h, Op::Shading(Shading::Neutral));
    let supported = h.state().view3d_supported_samples().unwrap();
    // 選びは保たれ、描く数だけが下がる
    op(&mut h, Op::Antialias(8));
    assert_eq!(h.state().state.view3d.display.post.antialias, 8);
    let want = clamp_samples(8, &supported);
    assert_eq!(h.state().view3d_stats().unwrap().samples, want);
    assert_eq!(
        h.state().state.view3d.display.shown_samples(),
        want,
        "画面に出す選びは、使える最大へ下げたもの"
    );
    // 描き先の上限が小さければ、多サンプルをやめる（描けなくはしない）
    h.state_mut().view3d_set_target_budget(Some(1));
    h.run();
    assert_eq!(
        h.state().view3d_stats().unwrap().samples,
        1,
        "上限を超えるときは 1×"
    );
    let image = h.render().unwrap();
    let (face, background) = (px(&image, center(&h)), px(&image, background_point(&h)));
    assert_eq!(
        count_other(
            &image,
            region(&h),
            view3d_axes_rect(&h),
            &[face, background],
            2
        ),
        0
    );
    // 1 つ下の数の見積もりがぎりぎり入る上限では、1 つ下の数になる
    if let Some(&lower) = supported.iter().rev().find(|n| **n > 1 && **n < want) {
        let rect = view_rect(&h);
        let size = [rect.width().round() as u32, rect.height().round() as u32];
        h.state_mut()
            .view3d_set_target_budget(Some(target_bytes(size, lower, false, false)));
        h.run();
        assert_eq!(
            h.state().view3d_stats().unwrap().samples,
            lower,
            "{lower}× までなら入る上限"
        );
    }
    // 上限を戻すと、また対応する最大の数
    h.state_mut().view3d_set_target_budget(None);
    h.run();
    assert_eq!(h.state().view3d_stats().unwrap().samples, want);
    // ウィンドウが違う大きさでも、いつでも対応する数のどれか
    for n in SAMPLE_CHOICES {
        op(&mut h, Op::Antialias(n));
        assert!(supported.contains(&h.state().view3d_stats().unwrap().samples));
    }
}

#[test]
fn the_target_ceiling_is_the_size_of_the_picture_share_and_sits_outside_the_settings_total() {
    use yolu_app::gpu_memory::{self, GpuMemory};
    use yolu_app::prefs::{Pref, PrefsAction};
    let mut h = view(900.0, 640.0, 32);
    for choice in [
        GpuMemory::Low,
        GpuMemory::High,
        GpuMemory::Mib(2048),
        GpuMemory::Standard,
    ] {
        h.state_mut()
            .state
            .apply(Action::Prefs(PrefsAction::Set(Pref::GpuMemory(choice))));
        h.run();
        let adapter = h.state().state.prefs.gpu.clone();
        let budgets = gpu_memory::budgets(choice, &adapter);
        // 描き先の上限は、3D の絵の取り分と同じ量（別の勘定。取り分からは引かない）
        assert_eq!(
            h.state().view3d_paint_budget(),
            Some(budgets.paint),
            "{choice:?}"
        );
        assert_eq!(
            h.state().view3d_target_budget(),
            Some(budgets.paint),
            "{choice:?}"
        );
        // 合計に配った 3 つの外に足すので、載り切ると合計を超えうる
        let total = gpu_memory::total_bytes(choice, &adapter);
        assert!(
            budgets.paint + budgets.canvas + budgets.shelf_preview + budgets.paint > total,
            "{choice:?}"
        );
    }
    // 試験が決めた上限は取り分に従わず、None で取り分と同じ量へ戻る
    let paint = h.state().view3d_paint_budget();
    h.state_mut().view3d_set_target_budget(Some(1));
    assert_eq!(h.state().view3d_target_budget(), Some(1));
    h.state_mut().view3d_set_target_budget(None);
    assert_eq!(h.state().view3d_target_budget(), paint);
}

#[test]
fn the_sample_planner_steps_down_through_what_the_device_has() {
    let size = [1920, 1080];
    let all = [1, 2, 4, 8];
    let big = u64::MAX;
    assert_eq!(plan_samples(&all, 8, size, false, false, big), 8);
    assert_eq!(plan_samples(&all, 4, size, false, false, big), 4);
    assert_eq!(
        plan_samples(&all, 3, size, false, false, big),
        2,
        "選べない数は、以下で対応する最大"
    );
    assert_eq!(plan_samples(&all, 1, size, false, false, big), 1);
    // 機材が持たない数は選べない
    assert_eq!(plan_samples(&[1, 4], 8, size, false, false, big), 4);
    assert_eq!(plan_samples(&[1, 4], 2, size, false, false, big), 1);
    assert_eq!(plan_samples(&[1], 8, size, false, false, big), 1);
    assert_eq!(plan_samples(&[], 8, size, false, false, big), 1);
    // 上限: 8× の見積もりより 1 バイト少なければ 4×、4× より少なければ 2×、…、0 でも 1×（使えなくはしない）
    let bytes = |n, hdr, bloom| target_bytes(size, n, hdr, bloom);
    assert_eq!(
        plan_samples(&all, 8, size, false, false, bytes(8, false, false)),
        8
    );
    assert_eq!(
        plan_samples(&all, 8, size, false, false, bytes(8, false, false) - 1),
        4
    );
    assert_eq!(
        plan_samples(&all, 8, size, false, false, bytes(4, false, false) - 1),
        2
    );
    assert_eq!(
        plan_samples(&all, 8, size, false, false, bytes(2, false, false) - 1),
        1
    );
    assert_eq!(plan_samples(&all, 8, size, false, false, 0), 1);
    // 対応する数だけを飛び石に下がる
    assert_eq!(
        plan_samples(&[1, 4], 8, size, false, false, bytes(4, false, false) - 1),
        1
    );
    // HDR・ブルームは見積もりを増やすので、同じ上限で数が下がる
    let budget = bytes(4, false, false);
    assert_eq!(plan_samples(&all, 4, size, false, false, budget), 4);
    assert_eq!(plan_samples(&all, 4, size, true, false, budget), 2);
    assert!(
        bytes(4, true, true) > bytes(4, true, false)
            && bytes(4, true, false) > bytes(4, false, false)
    );
}

// ───────── 描き先のメモリ ─────────

#[test]
fn the_target_estimate_adds_up_depth_color_resolve_hdr_and_bloom() {
    let size = [1000, 500];
    let px = 500_000u64;
    // 解決後の 8 bit（4 B）+ 深度（サンプルごと 4 B）[+ 多サンプルの色（8 bit は 4 B・HDR は 8 B）× サンプル] [+ 解決後の HDR（8 B）] [+ ブルームの段]
    assert_eq!(target_bytes(size, 1, false, false), px * (4 + 4));
    assert_eq!(target_bytes(size, 2, false, false), px * (4 + 8 + 8));
    assert_eq!(target_bytes(size, 4, false, false), px * (4 + 16 + 16));
    assert_eq!(target_bytes(size, 8, false, false), px * (4 + 32 + 32));
    assert_eq!(target_bytes(size, 1, true, false), px * (4 + 4 + 8));
    assert_eq!(target_bytes(size, 4, true, false), px * (4 + 16 + 32 + 8));
    // ブルーム: 半分の大きさから 6 段（500×250・250×125・125×62・62×31・31×15・15×7）、1 画素 8 B
    assert_eq!(
        bloom_levels(size),
        vec![
            [500, 250],
            [250, 125],
            [125, 62],
            [62, 31],
            [31, 15],
            [15, 7]
        ]
    );
    let bloom = (125_000 + 31_250 + 7_750 + 1_922 + 465 + 105) * 8;
    assert_eq!(
        target_bytes(size, 4, true, true),
        px * (4 + 16 + 32 + 8) + bloom
    );
    assert_eq!(target_bytes(size, 1, true, true), px * (4 + 4 + 8) + bloom);
    // 小さい絵は段を減らし、1 画素未満にしない
    assert_eq!(bloom_levels([8, 8]), vec![[4, 4]]);
    assert_eq!(bloom_levels([1, 1]), vec![[1, 1]]);
    for s in [[1, 1], [2, 2], [7, 3], [64, 48], [4096, 2160]] {
        let levels = bloom_levels(s);
        assert!(
            !levels.is_empty()
                && levels.len() <= 6
                && levels[0] == [(s[0] / 2).max(1), (s[1] / 2).max(1)],
            "{s:?}"
        );
        assert!(
            levels
                .windows(2)
                .all(|w| w[1][0] <= w[0][0] && w[1][1] <= w[0][1] && w[1][0] >= 1 && w[1][1] >= 1),
            "{s:?}"
        );
    }
}

#[test]
fn the_stats_report_the_estimate_of_the_targets_in_use() {
    let mut h = view(900.0, 640.0, 32);
    set_model(&mut h, vec![quad(1.0, Vec3::NEG_Z)]);
    look_at(&mut h, 2.6, 30.0, 20.0);
    head_on(&mut h);
    fill(&mut h, &[(Channel::Color, [200, 120, 60, 255])]);
    let rect = view_rect(&h);
    let size = [rect.width().round() as u32, rect.height().round() as u32];
    let mut seen = Vec::new();
    for (hdr_ops, hdr, bloom) in [
        (vec![], false, false),
        (vec![Op::Exposure(1.0)], true, false),
        (vec![Op::Bloom(true)], true, true),
    ] {
        op(&mut h, Op::Exposure(0.0));
        op(&mut h, Op::Bloom(false));
        for o in hdr_ops {
            op(&mut h, o);
        }
        for n in [1, 4] {
            op(&mut h, Op::Antialias(n));
            let stats = h.state().view3d_stats().unwrap();
            assert_eq!(
                stats.target_bytes,
                target_bytes(size, stats.samples, hdr, bloom),
                "{n}× hdr {hdr} bloom {bloom}"
            );
            seen.push((stats.samples, hdr, bloom, stats.target_bytes));
        }
    }
    // 多サンプル・HDR・ブルームの順に増える
    let find = |s: u32, hdr, bloom| {
        seen.iter()
            .find(|e| e.0 == s && e.1 == hdr && e.2 == bloom)
            .map(|e| e.3)
            .unwrap()
    };
    assert!(find(4, false, false) > find(1, false, false));
    assert!(find(1, true, false) > find(1, false, false));
    assert!(find(1, true, true) > find(1, true, false));
}

/// 値の表を出す（`cargo test -p yolu-app --test gui_view3d -- --ignored --nocapture view3d_fx::memory_table`）。報告の表の元。
#[test]
#[ignore = "報告の表を出すだけ"]
fn memory_table() {
    println!("| 絵の大きさ | 仕上げ | 1× | 2× | 4× | 8× |");
    for (name, size) in [
        ("1920×1080", [1920u32, 1080]),
        ("2560×1440", [2560, 1440]),
        ("3840×2160", [3840, 2160]),
    ] {
        for (label, hdr, bloom) in [
            ("8 bit", false, false),
            ("HDR（トーンマッピング）", true, false),
            ("HDR＋ブルーム", true, true),
        ] {
            let cells: Vec<String> = SAMPLE_CHOICES
                .iter()
                .map(|n| {
                    format!(
                        "{:.0} MiB",
                        target_bytes(size, *n, hdr, bloom) as f64 / 1048576.0
                    )
                })
                .collect();
            println!("| {name} | {label} | {} |", cells.join(" | "));
        }
    }
}

// ───────── ブルーム ─────────

/// 真正面から白い板を、暗い背景に（中立の表示: 面は光の強さのまま 1.0。ガンマ・リニアとも 1）。
fn bright_scene(h: &mut Harness<'_, YoluApp>, color: [u8; 3]) {
    set_model(h, vec![quad(0.7, Vec3::NEG_Z)]);
    look_at(h, 2.6, 0.0, 0.0);
    head_on(h);
    fill(h, &[(Channel::Color, [color[0], color[1], color[2], 255])]);
    op(h, Op::Shading(Shading::Neutral));
    op(h, Op::Antialias(1));
}

/// 板のすぐ外側（左へ `gap` 画素）と、表示域の遠い隅。
fn halo_points(h: &Harness<'_, YoluApp>, gap: f32) -> (egui::Pos2, egui::Pos2) {
    // 板の端の画面上の位置
    let rect = view_rect(h);
    let view = h
        .state()
        .state
        .view3d
        .camera
        .view(rect.width(), rect.height());
    let edge = view.to_screen(Vec3::new(-0.35, 0.0, 0.0)).unwrap();
    let side = if edge.x < rect.width() * 0.5 {
        -1.0
    } else {
        1.0
    };
    let near = pos2(rect.left() + edge.x + side * gap, rect.top() + edge.y);
    (near, pos2(rect.left() + 100.0, rect.bottom() - 100.0))
}

#[test]
fn bloom_brightens_around_bright_areas_and_leaves_the_far_background_alone() {
    // （3D の表示域の幅が前の既定の並び（900 点のウィンドウ）と同じになるよう、右の列の幅と左のツールの帯の分だけウィンドウを広げる）
    let mut h = view(1024.0, 640.0, 32);
    bright_scene(&mut h, [255, 255, 255]);
    let before = h.render().unwrap();
    let (near, far) = halo_points(&h, 6.0);
    let off = (px(&before, near), px(&before, far));
    op(&mut h, Op::Bloom(true));
    op(&mut h, Op::BloomStrength(1.0));
    let after = h.render().unwrap();
    let on = (px(&after, near), px(&after, far));
    assert!(h.state().view3d_stats().unwrap().bloom_renders >= 1);
    // 板のすぐ外は明るくなり、離れた隅はほぼ変わらない
    let lum = |c: [u8; 3]| c[0] as i32 + c[1] as i32 + c[2] as i32;
    assert!(
        lum(on.0) > lum(off.0) + 30,
        "板の外が明るくなる: {:?} → {:?}",
        off.0,
        on.0
    );
    assert!(
        lum(on.1) <= lum(off.1) + 6,
        "遠い隅は変わらない: {:?} → {:?}",
        off.1,
        on.1
    );
    // 板の上（芯）はしきい値を超えて飽和したまま、減らない
    assert!(lum(px(&after, center(&h))) >= lum(px(&before, center(&h))));
    // 離れるほど弱い
    let (mid, _) = halo_points(&h, 24.0);
    let (outer, _) = halo_points(&h, 80.0);
    assert!(
        lum(px(&after, near)) > lum(px(&after, mid))
            && lum(px(&after, mid)) >= lum(px(&after, outer))
    );
    // 強さに比例して強くなる
    op(&mut h, Op::BloomStrength(2.0));
    let strong = h.render().unwrap();
    assert!(
        lum(px(&strong, near)) > lum(px(&after, near)) + 10,
        "強さを上げると強い"
    );
    op(&mut h, Op::BloomStrength(0.3));
    let weak = h.render().unwrap();
    assert!(
        lum(px(&weak, near)) < lum(px(&after, near)),
        "強さを下げると弱い"
    );
    assert!(lum(px(&weak, near)) > lum(off.0));
}

#[test]
fn bloom_leaves_what_is_below_the_threshold_alone() {
    // （3D の表示域の幅が前の既定の並び（900 点のウィンドウ）と同じになるよう、右の列の幅と左のツールの帯の分だけウィンドウを広げる）
    let mut h = view(1024.0, 640.0, 32);
    // 灰色の板（リニアで約 0.1）: しきい値 0.8 より暗い
    bright_scene(&mut h, [90, 90, 90]);
    let off = h.render().unwrap();
    op(&mut h, Op::Bloom(true));
    op(&mut h, Op::BloomStrength(2.0));
    let on = h.render().unwrap();
    assert!(h.state().view3d_stats().unwrap().bloom_renders >= 1);
    let area = view_rect(&h);
    let mut worst = 0;
    for y in area.top().ceil() as u32..area.bottom().floor() as u32 {
        for x in area.left().ceil() as u32..area.right().floor() as u32 {
            let (a, b) = (off.get_pixel(x, y).0, on.get_pixel(x, y).0);
            // 3D の絵の上に重なる画面の部品（右上のアイコン）も同じに描かれるので、差が出るのは絵の画素だけ
            worst = worst.max((0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap());
        }
    }
    assert!(
        worst <= 1,
        "しきい値より暗い絵は 1 段階以上は変わらない: {worst}"
    );
    // しきい値を上げても、明るい板がにじまなくなる（しきい値が効いている）
    bright_scene(&mut h, [255, 255, 255]);
    let (near, _) = halo_points(&h, 6.0);
    op(&mut h, Op::Bloom(false));
    let bright_off = px(&h.render().unwrap(), near);
    op(&mut h, Op::Bloom(true));
    op(&mut h, Op::BloomStrength(1.0));
    op(&mut h, Op::BloomThreshold(0.5));
    let low = px(&h.render().unwrap(), near);
    op(&mut h, Op::BloomThreshold(3.5));
    let high = px(&h.render().unwrap(), near);
    assert!(
        close(high, bright_off, 1),
        "しきい値 3.5 では 1.0 の板はにじまない: {bright_off:?} → {high:?}"
    );
    assert!(
        low[0] as i32 > high[0] as i32 + 30,
        "しきい値が低いほどにじむ: {low:?} {high:?}"
    );
}

#[test]
fn bloom_off_draws_the_same_picture_byte_for_byte() {
    let mut h = view(900.0, 640.0, 32);
    bright_scene(&mut h, [255, 255, 255]);
    op(&mut h, Op::Antialias(4));
    let first = h.render().unwrap();
    let renders = h.state().view3d_stats().unwrap().bloom_renders;
    // 入にして切に戻すと、前と同じ絵（バイト）。切のあいだはブルームの描きを数えない
    op(&mut h, Op::Bloom(true));
    let lit = h.render().unwrap();
    assert_ne!(first.as_raw(), lit.as_raw(), "入なら絵が変わる");
    op(&mut h, Op::Bloom(false));
    let back = h.render().unwrap();
    assert_eq!(
        first.as_raw(),
        back.as_raw(),
        "切に戻すと前と同じ絵（バイト）"
    );
    assert_eq!(h.state().view3d_stats().unwrap().bloom_renders, renders + 1);
    // 入でも強さ 0 は何も足さない（切と同じ道・同じ絵）
    op(&mut h, Op::BloomStrength(0.0));
    op(&mut h, Op::Bloom(true));
    let zero = h.render().unwrap();
    assert_eq!(
        first.as_raw(),
        zero.as_raw(),
        "強さ 0 は切と同じ絵（バイト）"
    );
    assert_eq!(h.state().view3d_stats().unwrap().bloom_renders, renders + 1);
    // トーンマッピングの道でも、ブルームが切（強さ 0）なら前と同じ絵（バイト）
    op(&mut h, Op::Exposure(0.5));
    op(&mut h, Op::Tone(Curve::Neutral));
    let tone = h.render().unwrap();
    op(&mut h, Op::BloomStrength(1.0));
    op(&mut h, Op::Bloom(false));
    let tone_off = h.render().unwrap();
    assert_eq!(
        tone.as_raw(),
        tone_off.as_raw(),
        "トーンマッピングの道: ブルームを切ると同じ絵（バイト）"
    );
}

#[test]
fn bloom_and_exposure_work_together_and_light_free_views_get_no_bloom() {
    // （3D の表示域の幅が前の既定の並び（900 点のウィンドウ）と同じになるよう、右の列の幅と左のツールの帯の分だけウィンドウを広げる）
    let mut h = view(1024.0, 640.0, 32);
    // 灰色の板: 露出 0 ではしきい値の下、+2 EV では上（リニア 0.26 × 4 > 0.8）
    bright_scene(&mut h, [140, 140, 140]);
    op(&mut h, Op::Bloom(true));
    op(&mut h, Op::BloomStrength(1.0));
    let (near, _) = halo_points(&h, 6.0);
    let dim = px(&h.render().unwrap(), near);
    op(&mut h, Op::Exposure(2.0));
    let lit = px(&h.render().unwrap(), near);
    op(&mut h, Op::Bloom(false));
    let lit_off = px(&h.render().unwrap(), near);
    assert!(
        lit[0] as i32 > lit_off[0] as i32 + 10,
        "露出を上げると、板の外へにじむ: {lit_off:?} → {lit:?}"
    );
    assert!(
        dim[0] <= lit_off[0] || dim[0] <= 45,
        "露出 0 ではほぼにじまない: {dim:?}"
    );
    // 光なしの表示（チャンネルだけ）は、ブルームを入にしても足さない
    op(&mut h, Op::Bloom(true));
    op(&mut h, Op::Exposure(0.0));
    op(&mut h, Op::Shading(Shading::Channel(Channel::Color)));
    let renders = h.state().view3d_stats().unwrap().bloom_renders;
    let channel_on = h.render().unwrap();
    op(&mut h, Op::Bloom(false));
    let channel_off = h.render().unwrap();
    assert_eq!(
        channel_on.as_raw(),
        channel_off.as_raw(),
        "チャンネルだけの表示にブルームは足さない"
    );
    assert_eq!(h.state().view3d_stats().unwrap().bloom_renders, renders);
}

#[test]
fn the_emission_channel_glows_in_the_material_view() {
    // （3D の表示域の幅が前の既定の並び（900 点のウィンドウ）と同じになるよう、右の列の幅と左のツールの帯の分だけウィンドウを広げる）
    let mut h = view(1024.0, 640.0, 32);
    set_model(&mut h, vec![quad(0.7, Vec3::NEG_Z)]);
    look_at(&mut h, 2.6, 0.0, 0.0);
    head_on(&mut h);
    op(&mut h, Op::Antialias(1));
    op(&mut h, Op::LightIntensity(0.0));
    fill(
        &mut h,
        &[
            (Channel::Color, [20, 20, 20, 255]),
            (Channel::Emission, [255, 160, 60, 255]),
        ],
    );
    let (near, far) = halo_points(&h, 6.0);
    let off = h.render().unwrap();
    // 発光は面そのものに出る（光を切っても板は明るい）
    assert!(
        px(&off, center(&h))[0] > 200,
        "発光の板: {:?}",
        px(&off, center(&h))
    );
    op(&mut h, Op::Bloom(true));
    op(&mut h, Op::BloomStrength(1.0));
    let on = h.render().unwrap();
    let lum = |c: [u8; 3]| c[0] as i32 + c[1] as i32 + c[2] as i32;
    assert!(
        lum(px(&on, near)) > lum(px(&off, near)) + 30,
        "発光の周りが光る: {:?} → {:?}",
        px(&off, near),
        px(&on, near)
    );
    assert!(lum(px(&on, far)) <= lum(px(&off, far)) + 6);
    // にじみは発光の色（橙）を帯びる
    let glow = px(&on, near);
    assert!(glow[0] > glow[2], "橙の発光は橙のにじみ: {glow:?}");
}

// ───────── 光の既定の向き ─────────

#[test]
fn the_default_light_lights_the_front_of_a_model_facing_plus_z_and_not_its_back() {
    let d = Display::default();
    let to_light = d.light_direction();
    let white = [255, 255, 255, 255];
    // +Z を向く板（モデルの前）を +Z の側から見る: 光の来る向きとの内積の分だけ明るい（中立の表示 0.35 + 0.65 × saturate(n·L)）
    let mut h = view(900.0, 640.0, 32);
    set_model(&mut h, vec![quad(1.4, Vec3::Z)]);
    look_at(&mut h, 2.6, 180.0, 0.0);
    fill(&mut h, &[(Channel::Color, white)]);
    op(&mut h, Op::Shading(Shading::Neutral));
    op(&mut h, Op::Env(yolu_app::view3d::display::EnvKind::None));
    op(&mut h, Op::Antialias(1));
    assert_eq!(
        h.state().state.view3d.display.light_direction(),
        to_light,
        "新しいウィンドウの光は既定のまま"
    );
    let front = px(&h.render().unwrap(), center(&h));
    let want = (255.0 * (0.35 + 0.65 * to_light.dot(Vec3::Z).clamp(0.0, 1.0))).round() as u8;
    assert_close(front, [want; 3], 2, "前から見た面");
    // −Z を向く板（モデルの背中）を −Z の側から見る: 光が当たらない（影の側の明るさ 0.35）
    let mut b = view(900.0, 640.0, 32);
    set_model(&mut b, vec![quad(1.4, Vec3::NEG_Z)]);
    look_at(&mut b, 2.6, 0.0, 0.0);
    fill(&mut b, &[(Channel::Color, white)]);
    op(&mut b, Op::Shading(Shading::Neutral));
    op(&mut b, Op::Env(yolu_app::view3d::display::EnvKind::None));
    op(&mut b, Op::Antialias(1));
    let back = px(&b.render().unwrap(), center(&b));
    assert_close(back, [(255.0f32 * 0.35).round() as u8; 3], 2, "背中側の面");
    assert!(
        front[0] > back[0] + 80,
        "前は背中より明るい: {front:?} {back:?}"
    );
}

// ───────── 設定の保存と読み直し ─────────

fn settings_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/view3d-fx-tests")
        .join(std::process::id().to_string())
        .join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn app_with_settings(path: &std::path::Path) -> Harness<'static, YoluApp> {
    let path = path.to_path_buf();
    build(1280.0, 800.0, move |ctx| {
        YoluApp::for_context_with_settings(ctx, Some(path), PenInput::detached())
    })
}

#[test]
fn the_finish_is_written_to_the_settings_file_and_comes_back_at_the_next_start() {
    let dir = settings_dir("finish");
    let path = dir.join("YoluPainter").join("settings.conf");
    let mut h = app_with_settings(&path);
    // 初めは既定（4×・ブルーム切）で、ファイルを作らない
    assert_eq!(h.state().state.view3d.display.post, Display::default().post);
    assert!(!path.exists());
    op(&mut h, Op::Antialias(8));
    op(&mut h, Op::Bloom(true));
    op(&mut h, Op::BloomStrength(1.25));
    op(&mut h, Op::BloomThreshold(1.5));
    h.run();
    let written = std::fs::read_to_string(&path).unwrap();
    for line in [
        "view3d_antialias=8",
        "view3d_bloom=on",
        "view3d_bloom_strength=1.25",
        "view3d_bloom_threshold=1.5",
    ] {
        assert!(written.lines().any(|l| l == line), "{line}\n{written}");
    }
    // スライダーをドラッグしているあいだは書かない。離したときの値を書く
    h.state_mut().state.view3d.display.post_dragging = true;
    op(&mut h, Op::BloomStrength(0.5));
    op(&mut h, Op::BloomStrength(0.75));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        written,
        "ドラッグ中は書かない"
    );
    h.state_mut().state.view3d.display.post_dragging = false;
    h.run();
    let released = std::fs::read_to_string(&path).unwrap();
    assert!(
        released.lines().any(|l| l == "view3d_bloom_strength=0.75"),
        "{released}"
    );
    drop(h);
    // 次の起動は、書いた値で始まる。既定に戻すと行が消える（既定は書かない）
    let mut h = app_with_settings(&path);
    let post = h.state().state.view3d.display.post;
    assert_eq!(
        (
            post.antialias,
            post.bloom,
            post.bloom_strength,
            post.bloom_threshold
        ),
        (8, true, 0.75, 1.5)
    );
    op(&mut h, Op::Antialias(4));
    op(&mut h, Op::Bloom(false));
    op(&mut h, Op::BloomStrength(0.6));
    op(&mut h, Op::BloomThreshold(0.8));
    h.run();
    let back = std::fs::read_to_string(&path).unwrap();
    assert!(!back.contains("view3d_"), "既定は書かない: {back}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_settings_file_from_before_the_finish_keeps_every_other_value_and_starts_at_four_samples() {
    let dir = settings_dir("old");
    let path = dir.join("YoluPainter").join("settings.conf");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let old = "language=en\nexport_padding=8\nview3d_orbit=model\nuv_wireframe=off\n";
    std::fs::write(&path, old).unwrap();
    let h = app_with_settings(&path);
    let app = &h.state().state;
    assert_eq!(app.lang, yolu_app::lang::Lang::En);
    assert_eq!(app.prefs.settings.export_padding, 8);
    assert!(!app.prefs.settings.uv_wireframe);
    assert_eq!(
        app.view3d.display.post,
        Display::default().post,
        "仕上げは既定"
    );
    assert_eq!(app.view3d.display.post.antialias, 4);
    assert_eq!(app.message, "", "壊れた設定として知らせない");
    // 読んだだけではファイルに触らない
    assert_eq!(std::fs::read_to_string(&path).unwrap(), old);
    std::fs::remove_dir_all(dir).unwrap();
}

// ───────── 当たり（画面の点からの選び）は描いた絵を読まない ─────────

#[test]
fn a_3d_stroke_lands_on_the_same_texels_at_every_sample_count() {
    let stroke = |samples: u32| {
        let mut h = view(900.0, 640.0, 64);
        set_model(&mut h, vec![quad(1.4, Vec3::NEG_Z)]);
        look_at(&mut h, 2.6, 20.0, 10.0);
        op(&mut h, Op::Antialias(samples));
        h.run();
        let c = center(&h);
        // 斜めの縁の近くまで（縁をまたぐ点を含む）
        let path: Vec<egui::Pos2> = (0..=8)
            .map(|i| c + vec2(-90.0 + 22.0 * i as f32, -60.0 + 16.0 * i as f32))
            .collect();
        drag(&mut h, &path);
        let doc = &h.state().state.doc;
        (
            h.state().view3d_stats().unwrap().samples,
            doc.composite(doc.bounds()).unwrap(),
        )
    };
    let (one, a) = stroke(1);
    let (many, b) = stroke(8);
    assert_eq!(one, 1);
    assert!(a.iter().any(|v| *v != 0), "描けた");
    assert_eq!(a, b, "サンプル数が違っても同じ画素を塗る（{many}×）");
}

// ───────── 設定のパネル（画質の面） ─────────

/// ブルームが切のあいだ、強さ・しきい値は押せず、理由（ブルームが切です）が出る。入れると理由は消える。値の欄のツールチップは、押せるときは出ない。
#[test]
fn the_bloom_values_say_the_bloom_is_off() {
    use egui_kittest::kittest::{NodeT, Queryable};
    let mut h = view(1100.0, 760.0, 32);
    h.state_mut().state.view3d.load_demo();
    h.run();
    h.get_by_label("光・環境・トーンマッピング").click();
    h.run();
    h.get_by_label("画質").click();
    h.run();
    let reason = "ブルームが切です";
    for label in ["強さ", "しきい値"] {
        let node = h.get_by_label(label);
        assert!(node.accesskit_node().is_disabled(), "{label}");
        let at = node.rect().center();
        hover_and_wait(&mut h, at);
        assert!(h.query_by_label(reason).is_some(), "{label}: 理由");
        move_to(&h, pos2(1.0, 1.0));
        h.run();
    }
    h.get_by_label("ブルーム").click();
    h.run();
    for label in ["強さ", "しきい値"] {
        let node = h.get_by_label(label);
        assert!(!node.accesskit_node().is_disabled(), "{label}");
        let at = node.rect().center();
        hover_and_wait(&mut h, at);
        assert!(h.query_by_label(reason).is_none(), "{label}: 理由は消える");
        move_to(&h, pos2(1.0, 1.0));
        h.run();
    }
    h.state_mut().state.lang = yolu_app::lang::Lang::En;
    h.run();
    let reason = "Bloom is off";
    h.get_by_label("Bloom").click();
    h.run();
    for label in ["Strength", "Threshold"] {
        let node = h.get_by_label(label);
        assert!(node.accesskit_node().is_disabled(), "{label}");
        let at = node.rect().center();
        hover_and_wait(&mut h, at);
        assert!(h.query_by_label(reason).is_some(), "{label}: reason");
        move_to(&h, pos2(1.0, 1.0));
        h.run();
    }
}

#[test]
fn the_quality_tab_picks_the_count_and_edits_the_bloom_in_both_languages() {
    use egui_kittest::kittest::Queryable;
    let mut h = view(1100.0, 760.0, 32);
    h.state_mut().state.view3d.load_demo();
    h.run();
    h.get_by_label("光・環境・トーンマッピング").click();
    h.run();
    assert!(h.state().state.view3d.display.settings_open);
    h.get_by_label("画質").click();
    h.run();
    assert_eq!(
        h.state().state.view3d.display.tab,
        yolu_app::view3d::display::SettingsTab::Quality
    );
    for label in ["切", "4×", "ブルーム", "強さ", "しきい値"] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    // 4× を選ぶ・切にする
    h.get_by_label("切").click();
    h.run();
    assert_eq!(h.state().state.view3d.display.post.antialias, 1);
    h.get_by_label("4×").click();
    h.run();
    assert_eq!(h.state().state.view3d.display.post.antialias, 4);
    // 機材が対応しない数は押せない（選びは変わらない）
    let supported = h.state().view3d_supported_samples().unwrap();
    for n in [2u32, 8] {
        if !supported.contains(&n) {
            h.get_by_label(&format!("{n}×")).click();
            h.run();
            assert_eq!(
                h.state().state.view3d.display.post.antialias,
                4,
                "{n}× は対応しない"
            );
        }
    }
    // ブルームの入・切（値の欄は切のあいだ押せない）
    assert!(!h.state().state.view3d.display.post.bloom);
    h.get_by_label("ブルーム").click();
    h.run();
    assert!(h.state().state.view3d.display.post.bloom);
    h.get_by_label("ブルーム").click();
    h.run();
    assert!(!h.state().state.view3d.display.post.bloom);
    // 既定に戻すはブルームを戻し、アンチエイリアスは戻さない（画質の選び）
    op(&mut h, Op::Antialias(2));
    op(&mut h, Op::Bloom(true));
    op(&mut h, Op::BloomThreshold(2.0));
    op(&mut h, Op::ResetLighting);
    let post = h.state().state.view3d.display.post;
    assert!(
        !post.bloom && post.bloom_threshold == yolu_app::view3d::display::DEFAULT_BLOOM_THRESHOLD
    );
    assert_eq!(post.antialias, 2);
    // 画像は機材によらず同じ並びで（全部の数を押せる並び・4× を選んだ状態）
    h.state_mut()
        .state
        .view3d
        .display
        .set_supported_samples(&[1, 2, 4, 8]);
    op(&mut h, Op::Antialias(4));
    // 英語
    h.state_mut().state.lang = yolu_app::lang::Lang::En;
    h.run();
    for label in ["Quality", "Off", "4×", "Bloom", "Strength", "Threshold"] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    h.snapshot("view3d_quality_panel_en");
    h.state_mut().state.lang = yolu_app::lang::Lang::Ja;
    h.run();
    h.snapshot("view3d_quality_panel");
}
